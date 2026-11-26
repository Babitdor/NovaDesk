//! The message queue, and steering.
//!
//! A turn is a one-shot child process whose stdin closes after the prompt, so
//! "send this when you finish" has to be Nova's own bookkeeping: the queue lives
//! here, keyed by task, and the turn that ends drains its own task's queue.
//!
//! Steering is the same idea but refused, on purpose. Every harness Nova drives
//! is started in a non-interactive mode (claude --print, codex exec, pi --print,
//! opencode run), and none of those accept a second message mid-turn. A tool
//! that accepted a "steer" and silently queued it would be lying about when the
//! message lands, so it refuses and names the harness, and the caller can queue
//! the message instead.

use serde_json::{json, Value};

use crate::error::AppError;
use crate::tools::ToolsState;

/// Long enough for a paragraph of instruction, short enough that the queue file
/// cannot become a transcript of its own.
const MAX_QUEUED_BYTES: usize = 32 * 1024;

pub fn push(tools: &ToolsState, chat_id: &str, text: &str) -> Result<Value, AppError> {
    if chat_id.trim().is_empty() || chat_id.len() > 128 {
        return Err(AppError::Tools("a task id is required".into()));
    }
    let text = text.trim();
    if text.is_empty() || text.len() > MAX_QUEUED_BYTES {
        return Err(AppError::Tools(format!(
            "a message of 1 to {MAX_QUEUED_BYTES} bytes is required"
        )));
    }
    let queued = tools.queue_push(chat_id, text);
    Ok(json!({ "queued": queued, "pending": tools.queue_len(chat_id) }))
}

pub fn list(tools: &ToolsState, chat_id: &str) -> Value {
    json!({ "taskId": chat_id, "queue": tools.queue_list(chat_id) })
}

pub fn remove(tools: &ToolsState, id: &str) -> Value {
    json!({ "removed": tools.queue_remove(id) })
}

/// True when the harness can accept a message while a turn is running. Every
/// harness Nova starts is non-interactive today, so this is the whole table —
/// kept as a function so adding one is a one-line change with a place to say
/// why.
fn mid_turn_input(harness: &str) -> bool {
    let _ = harness;
    false
}

pub fn steer(
    state: &crate::AppState,
    tools: &ToolsState,
    chat_id: &str,
    text: &str,
) -> Result<Value, AppError> {
    let Some((turn_id, harness)) = state.live_turn(chat_id) else {
        return Err(AppError::Tools(format!(
            "task {chat_id} is not running, so there is nothing to steer; queue the message with queue_message"
        )));
    };
    if mid_turn_input(&harness) {
        // Unreachable until a harness is added to the table above; the queued
        // path is the honest fallback either way.
        return push(tools, chat_id, text).map(|mut value| {
            value["steered"] = json!(turn_id);
            value
        });
    }
    Err(AppError::Tools(format!(
        "{harness} cannot take a message mid-turn: Nova starts it in a non-interactive mode, so the turn ends before another message can land. Queue it with queue_message."
    )))
}

pub fn interrupt(state: &crate::AppState, chat_id: &str) -> Result<Value, AppError> {
    let Some((turn_id, harness)) = state.live_turn(chat_id) else {
        return Err(AppError::Tools(format!(
            "task {chat_id} is not running, so there is nothing to interrupt"
        )));
    };
    state.cancel_turn(&turn_id);
    Ok(json!({ "interrupted": turn_id, "harness": harness }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn queued_messages_are_listed_in_order_and_removable() {
        let tools = ToolsState::default();
        push(&tools, "task-1", "first").unwrap();
        push(&tools, "task-1", "second").unwrap();
        let listed = list(&tools, "task-1");
        let queue = listed["queue"].as_array().unwrap();
        assert_eq!(queue.len(), 2);
        assert_eq!(queue[0]["text"], "first");

        let id = queue[0]["id"].as_str().unwrap().to_string();
        assert_eq!(remove(&tools, &id)["removed"], true);
        assert_eq!(list(&tools, "task-1")["queue"].as_array().unwrap().len(), 1);
    }

    #[test]
    fn an_empty_or_oversized_message_is_refused() {
        let tools = ToolsState::default();
        assert!(push(&tools, "task-1", "   ").is_err());
        assert!(push(&tools, "task-1", &"x".repeat(MAX_QUEUED_BYTES + 1)).is_err());
        assert!(push(&tools, "", "hello").is_err());
    }

    #[test]
    fn draining_takes_the_oldest_and_leaves_the_rest() {
        let tools = ToolsState::default();
        push(&tools, "task-1", "first").unwrap();
        push(&tools, "task-1", "second").unwrap();
        let drained = tools.queue_pop("task-1").unwrap();
        assert_eq!(drained.text, "first");
        assert_eq!(tools.queue_len("task-1"), 1);
        assert!(tools.queue_pop("task-2").is_none());
    }

    /// The refusal has to name the harness, or the caller cannot decide whether
    /// to queue instead.
    #[test]
    fn no_harness_takes_input_mid_turn_today() {
        assert!(!mid_turn_input("claude"));
        assert!(!mid_turn_input("codex"));
        assert!(!mid_turn_input("nova"));
    }
}
