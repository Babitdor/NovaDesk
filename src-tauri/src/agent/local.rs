use std::time::Duration;

use tokio_util::sync::CancellationToken;

use super::{AgentBackend, BoxFut, EventSink, TurnRequest};
use crate::error::AppError;
use crate::events::{AgentEvent, FinishReason};
use crate::proc::ProcessRunner;
use crate::workspace::WorkspaceGuard;

/// Deterministic placeholder backend: no API key, no network.
///
/// It exercises every pipe the UI needs — streaming deltas, a tool call, process
/// output, cancellation — so the shell is verifiable end to end on day one.
#[derive(Default)]
pub struct LocalAgent;

/// Delay between deltas: slow enough to watch, fast enough for tests.
const DELTA_DELAY: Duration = Duration::from_millis(12);

const TOOL_COMMAND: &str = "/run ";
const PROMPT_ECHO_CHARS: usize = 120;

impl AgentBackend for LocalAgent {
    fn start<'a>(
        &'a self,
        req: TurnRequest,
        sink: EventSink,
        cancel: CancellationToken,
    ) -> BoxFut<'a, Result<FinishReason, AppError>> {
        Box::pin(async move {
            for token in split_tokens(&reply_text(&req.prompt)) {
                if cancel.is_cancelled() {
                    return Ok(FinishReason::Cancelled);
                }
                tokio::select! {
                    _ = cancel.cancelled() => return Ok(FinishReason::Cancelled),
                    _ = tokio::time::sleep(DELTA_DELAY) => {}
                }
                let _ = sink.send(AgentEvent::MessageDelta {
                    text: token.to_string(),
                });
            }

            if let Some(command) = req.prompt.trim().strip_prefix(TOOL_COMMAND) {
                return run_tool(command, req.workspace.as_ref(), &sink, &cancel).await;
            }

            Ok(FinishReason::Completed)
        })
    }
}

/// Splits on whitespace but keeps it, so concatenating every delta reproduces the
/// reply byte for byte (including newlines).
fn split_tokens(text: &str) -> impl Iterator<Item = &str> {
    text.split_inclusive(char::is_whitespace)
}

fn reply_text(prompt: &str) -> String {
    let short: String = prompt.trim().chars().take(PROMPT_ECHO_CHARS).collect();
    format!(
        "Nova echo: {short}\n\nThis is the local streaming stub — no model, no network. \
         Send `{TOOL_COMMAND}<program> [args...]` to run a command inside the workspace."
    )
}

async fn run_tool(
    command: &str,
    workspace: Option<&WorkspaceGuard>,
    sink: &EventSink,
    cancel: &CancellationToken,
) -> Result<FinishReason, AppError> {
    let mut parts = command.split_whitespace();
    let program = parts
        .next()
        .ok_or_else(|| AppError::Agent(format!("usage: {TOOL_COMMAND}<program> [args...]")))?;
    let args: Vec<String> = parts.map(str::to_string).collect();
    let guard = workspace.ok_or(AppError::NoWorkspace)?;
    let call_id = uuid::Uuid::new_v4().to_string();

    let _ = sink.send(AgentEvent::ToolCall {
        call_id: call_id.clone(),
        name: "run".to_string(),
        input: command.to_string(),
    });

    match ProcessRunner::run(guard, program, &args, sink, cancel).await {
        Ok(outcome) => {
            let output = match (outcome.exit_code, outcome.truncated) {
                (Some(code), false) => format!("exit {code}"),
                (Some(code), true) => format!("exit {code} (output truncated)"),
                (None, true) => "terminated (output truncated)".to_string(),
                (None, false) => "terminated".to_string(),
            };
            let _ = sink.send(AgentEvent::ToolResult {
                call_id,
                ok: outcome.succeeded(),
                output,
            });
            Ok(FinishReason::Completed)
        }
        Err(AppError::Cancelled) => Ok(FinishReason::Cancelled),
        // A tool that fails is still a turn that completed; the error travels in
        // the tool result, which is where the UI shows it.
        Err(err) => {
            let _ = sink.send(AgentEvent::ToolResult {
                call_id,
                ok: false,
                output: err.to_string(),
            });
            Ok(FinishReason::Completed)
        }
    }
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use super::*;
    use crate::agent::drive_turn;
    use crate::events::testutil::collect_events;

    fn request(prompt: &str) -> TurnRequest {
        TurnRequest {
            chat_id: uuid::Uuid::new_v4().to_string(),
            model: None,
            effort: None,
            personality: None,
            session: None,
            trust_workspace: false,
            prompt: prompt.to_string(),
            workspace: None,
            requests_file: None,
            tools: None,
        }
    }

    fn deltas(events: &[AgentEvent]) -> String {
        events
            .iter()
            .filter_map(|e| match e {
                AgentEvent::MessageDelta { text } => Some(text.as_str()),
                _ => None,
            })
            .collect()
    }

    #[tokio::test]
    async fn deltas_are_ordered_and_reassemble_the_reply() {
        let (sink, seen) = collect_events();
        let reason = drive_turn(
            &LocalAgent,
            request("hello there"),
            sink,
            CancellationToken::new(),
        )
        .await;

        let events = seen.lock().unwrap().clone();
        assert_eq!(events.first(), Some(&AgentEvent::TurnStarted));
        assert_eq!(reason, FinishReason::Completed);
        assert_eq!(deltas(&events), reply_text("hello there"));

        let delta_count = events
            .iter()
            .filter(|e| matches!(e, AgentEvent::MessageDelta { .. }))
            .count();
        assert!(
            delta_count > 1,
            "expected streaming, got {delta_count} delta(s)"
        );
    }

    #[tokio::test]
    async fn cancel_mid_stream_finishes_cancelled_and_promptly() {
        let (sink, seen) = collect_events();
        let cancel = CancellationToken::new();
        let token = cancel.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(30)).await;
            token.cancel();
        });

        let started = Instant::now();
        let reason = drive_turn(
            &LocalAgent,
            request("a prompt long enough to stream"),
            sink,
            cancel,
        )
        .await;
        let elapsed = started.elapsed();

        let events = seen.lock().unwrap().clone();
        assert_eq!(reason, FinishReason::Cancelled);
        assert!(elapsed < Duration::from_secs(1), "cancel took {elapsed:?}");

        let delta_count = events
            .iter()
            .filter(|e| matches!(e, AgentEvent::MessageDelta { .. }))
            .count();
        assert!(delta_count > 0, "nothing streamed before cancel");
    }
}
