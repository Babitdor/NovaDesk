//! The webview bridge: tools whose data lives in localStorage.
//!
//! Nova keeps tasks, their transcripts and the board in the page
//! (src/lib/sessions.ts). Rather than copy that store into Rust and migrate
//! every user's history, a task tool asks the window and waits for the answer:
//!
//!   Rust: emit nova:tool-call { id, name, args } to the main webview
//!   Page: invoke tool_reply { id, result: { ok, value } | { ok: false, error } }
//!
//! The request id is a one-shot: the sender is removed from the map the moment
//! it is used, so a late or duplicated reply cannot answer a second call, and a
//! tool that times out drops its channel instead of leaking it.
//!
//! This is also why those tools are marked requiresWindow: with no window there
//! is nobody to ask, and pretending otherwise would answer "no tasks exist".

use std::time::Duration;

use serde_json::{json, Value};
use tauri::{AppHandle, Emitter, EventTarget};

use crate::error::AppError;
use crate::tools::ToolsState;

pub const EVENT: &str = "nova:tool-call";

/// A model waiting on a human-paced UI should not wait forever: the window
/// answers from memory, so a slow reply means something is wrong.
const REPLY_TIMEOUT: Duration = Duration::from_secs(30);

pub async fn ask(
    app: &AppHandle,
    tools: &ToolsState,
    name: &str,
    arguments: Value,
) -> Result<Value, AppError> {
    let id = uuid::Uuid::new_v4().to_string();
    let (sender, receiver) = tools.wait_reply();
    tools.register_reply(&id, sender);
    app.emit_to(
        EventTarget::webview(crate::tools::dispatch::MAIN),
        EVENT,
        json!({ "id": id, "name": name, "args": arguments }),
    )
    .map_err(|err| {
        tools.take_reply(&id);
        AppError::Tools(format!("could not reach the window: {err}"))
    })?;

    match tokio::time::timeout(REPLY_TIMEOUT, receiver).await {
        Ok(Ok(answer)) => {
            if answer.get("ok").and_then(Value::as_bool) == Some(true) {
                Ok(answer.get("value").cloned().unwrap_or(Value::Null))
            } else {
                Err(AppError::Tools(
                    answer
                        .get("error")
                        .and_then(Value::as_str)
                        .unwrap_or("the window refused the call")
                        .to_string(),
                ))
            }
        }
        Ok(Err(_)) => Err(AppError::Tools(
            "the window closed before it answered".to_string(),
        )),
        Err(_) => {
            tools.take_reply(&id);
            Err(AppError::Tools(format!(
                "{name} was not answered within {} seconds",
                REPLY_TIMEOUT.as_secs()
            )))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The event name is a contract with the page; a typo would look like the
    /// tool hanging for thirty seconds.
    #[test]
    fn the_event_name_matches_the_page_listener() {
        assert_eq!(EVENT, "nova:tool-call");
    }

    #[test]
    fn a_reply_is_removed_once_it_is_answered() {
        let tools = ToolsState::default();
        let (sender, _receiver) = tools.wait_reply();
        tools.register_reply("abc", sender);
        assert!(tools.take_reply("abc").is_some());
        assert!(tools.take_reply("abc").is_none());
    }
}
