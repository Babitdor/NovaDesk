use std::collections::VecDeque;
use std::path::Path;
use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;

use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin};
use tokio::sync::{mpsc, Mutex};
use tokio_util::sync::CancellationToken;

use super::{AgentBackend, BoxFut, EventSink, Personality, TurnRequest};
use crate::error::AppError;
use crate::events::{AgentEvent, FinishReason, InterruptReply};

/// Nova clips its own fields, but a single frame can still be megabytes; refuse
/// anything past the protocol's documented ceiling rather than buffering it.
const MAX_LINE: usize = 8 * 1024 * 1024;
const MAX_PROMPT_FRAME: usize = 1024 * 1024;

/// A bare Nova bootstrap loads LangChain, the tool graph and its MCP servers.
/// Waiting longer is better than reporting a slow machine as a failure.
const BOOT_TIMEOUT: Duration = Duration::from_secs(120);

/// Bootstrap failures print a wall of text; the last lines explain them.
const STDERR_TAIL: usize = 40;

/// Talks to `nova --mode pipe`: a long-lived child that owns one session and
/// speaks newline-delimited JSON on stdio.
///
/// The child is spawned once and reused, so the conversation keeps its context
/// across turns. Frame catalogue: `novacode_cli/headless/pipe.py`.
#[derive(Default)]
pub struct NovaAgent {
    /// One session, one turn at a time: the lock is held for the whole turn, so
    /// a second prompt queues behind the first instead of interleaving frames.
    session: Mutex<VecDeque<(String, Session)>>,
    /// Held outside `session` so an approval reply can be written while a turn
    /// owns the session lock.
    input: Mutex<Option<Arc<Mutex<ChildStdin>>>>,
}

struct Session {
    /// `kill_on_drop` is armed: dropping the session ends the agent process.
    child: Child,
    stdin: Arc<Mutex<ChildStdin>>,
    lines: mpsc::Receiver<String>,
    cwd: String,
    model: String,
    id: Option<String>,
    root: std::path::PathBuf,
    model_override: Option<String>,
}

enum Frame {
    Continue,
    Finished(FinishReason),
}

#[derive(Default)]
struct TurnFrames {
    slash: bool,
    preview: bool,
    model_activity: bool,
    direct_report: bool,
    error: bool,
}

impl Session {
    async fn spawn(
        root: &Path,
        model: Option<String>,
        trust_workspace: bool,
        personality: Option<&Personality>,
    ) -> Result<Self, AppError> {
        let mut command = match personality {
            Some(profile) => super::cli::nova_personality(profile)?,
            None => super::cli::command("nova")?,
        };
        command.args(["--mode", "pipe"]).env("NO_COLOR", "1");
        if trust_workspace {
            command.arg("--trust-workspace");
        }
        if let Some(model) = &model {
            command.args(["--model", model]);
        }
        let mut child = command
            // The session's working directory is fixed at startup: the protocol
            // refuses a per-prompt `cwd`.
            .current_dir(root)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .spawn()
            .map_err(|err| AppError::Agent(format!("Could not start Nova: {err}")))?;

        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| AppError::Agent("nova stdin was not piped".into()))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| AppError::Agent("nova stdout was not piped".into()))?;
        let stderr = child
            .stderr
            .take()
            .ok_or_else(|| AppError::Agent("nova stderr was not piped".into()))?;

        let (sender, lines) = mpsc::channel::<String>(8);
        tokio::spawn(async move {
            let mut reader = BufReader::new(stdout);
            let mut line = String::new();
            loop {
                line.clear();
                match (&mut reader)
                    .take(MAX_LINE as u64 + 1)
                    .read_line(&mut line)
                    .await
                {
                    // EOF, a decode error, or the child died: nothing more to send.
                    Ok(0) | Err(_) => break,
                    Ok(_) => {
                        if line.len() > MAX_LINE
                            || sender.send(line.trim_end().to_string()).await.is_err()
                        {
                            break;
                        }
                    }
                }
            }
        });

        let tail = Arc::new(Mutex::new(VecDeque::<String>::new()));
        let tail_writer = Arc::clone(&tail);
        tokio::spawn(async move {
            let mut reader = BufReader::new(stderr);
            let mut line = String::new();
            loop {
                line.clear();
                match (&mut reader).take(4096).read_line(&mut line).await {
                    Ok(0) | Err(_) => break,
                    Ok(_) => {
                        let mut tail = tail_writer.lock().await;
                        if tail.len() == STDERR_TAIL {
                            tail.pop_front();
                        }
                        tail.push_back(line.trim_end().to_string());
                    }
                }
            }
        });

        let stdin = Arc::new(Mutex::new(stdin));
        let mut session = Self {
            child,
            stdin: Arc::clone(&stdin),
            lines,
            cwd: root.display().to_string(),
            model: String::new(),
            id: None,
            root: root.to_path_buf(),
            model_override: model,
        };

        // Nothing else is accepted before `ready`, so wait for it explicitly
        // rather than racing a timeout and calling a slow boot a failure.
        let ready = tokio::time::timeout(BOOT_TIMEOUT, async {
            while let Some(line) = session.lines.recv().await {
                let frame: Value = serde_json::from_str(&line).unwrap_or(Value::Null);
                match frame["type"].as_str() {
                    Some("ready") => return Ok(Some(frame)),
                    Some("error") => {
                        let message = frame["message"].as_str().unwrap_or("Nova startup failed");
                        if frame["code"] == "process_error"
                            && message.starts_with("PermissionError: Workspace is not approved.")
                        {
                            return Err(AppError::WorkspaceNotApproved);
                        }
                        let mut error = message.to_owned();
                        if matches!(
                            frame["code"].as_str(),
                            Some("process_error" | "missing_dependencies")
                        ) {
                            error.push_str(&tail_suffix(&tail).await);
                        }
                        return Err(AppError::Agent(error));
                    }
                    _ => {}
                }
            }
            Ok(None)
        })
        .await;

        match ready {
            Ok(Ok(Some(frame))) => {
                session.id = frame["session_id"].as_str().map(str::to_owned);
                session.model = frame["model"].as_str().unwrap_or("unknown").to_string();
                if let Some(cwd) = frame["cwd"].as_str() {
                    session.cwd = cwd.to_string();
                }
                Ok(session)
            }
            Ok(Ok(None)) => Err(AppError::Agent(format!(
                "nova exited before it was ready{}",
                tail_suffix(&tail).await
            ))),
            Ok(Err(error)) => Err(error),
            Err(_) => Err(AppError::Agent(format!(
                "nova did not become ready within {}s{}",
                BOOT_TIMEOUT.as_secs(),
                tail_suffix(&tail).await
            ))),
        }
    }
}

async fn tail_suffix(tail: &Arc<Mutex<VecDeque<String>>>) -> String {
    let tail = tail.lock().await;
    if tail.is_empty() {
        return String::new();
    }
    let joined = tail
        .iter()
        .rev()
        .take(10)
        .rev()
        .cloned()
        .collect::<Vec<_>>()
        .join("\n");
    format!("\n{}", clip(&joined, 3500))
}

async fn write_frame(stdin: &Arc<Mutex<ChildStdin>>, frame: Value) -> Result<(), AppError> {
    let mut out = stdin.lock().await;
    let mut line = frame.to_string();
    line.push('\n');
    out.write_all(line.as_bytes())
        .await
        .map_err(|err| AppError::Agent(format!("could not write to nova: {err}")))?;
    out.flush()
        .await
        .map_err(|err| AppError::Agent(format!("could not flush nova: {err}")))
}

/// Folds one line of Nova output into UI events.
fn handle_frame(line: &str, pid: &str, sink: &EventSink, state: &mut TurnFrames) -> Frame {
    // Noise is never fatal: the protocol explicitly allows unparseable lines.
    let Ok(frame) = serde_json::from_str::<Value>(line) else {
        return Frame::Continue;
    };
    let kind = frame["type"].as_str().unwrap_or("");

    let owner = frame["request_id"]
        .as_str()
        .or_else(|| frame["id"].as_str());
    if owner.is_some_and(|owner| owner != pid) {
        return Frame::Continue;
    }

    // Rejections (`_reject`) and turn failures share this frame; neither ends
    // the turn on its own.
    if kind == "error" {
        state.error = true;
        let code = frame["code"].as_str().unwrap_or("error");
        let message = frame["message"].as_str().unwrap_or("unknown error");
        let _ = sink.send(AgentEvent::Notice {
            level: "error".to_string(),
            message: format!("{code}: {message}"),
        });
        return if code == "invalid_request" && owner.is_some() {
            Frame::Finished(FinishReason::Failed)
        } else {
            Frame::Continue
        };
    }

    match kind {
        "text_delta" => {
            if let Some(text) = frame["content"].as_str().filter(|text| !text.is_empty()) {
                state.preview = true;
                state.model_activity = true;
                let _ = sink.send(AgentEvent::MessagePreview {
                    text: text.to_owned(),
                });
            }
        }
        "text" => {
            let subagent = frame["is_subagent"] == true;
            let replace_preview = !subagent && state.preview;
            if let Some(text) = frame["content"]
                .as_str()
                .filter(|text| !text.is_empty() || replace_preview)
            {
                let plain = state.slash && !subagent && !state.model_activity;
                state.direct_report |= plain;
                let _ = sink.send(AgentEvent::MessageFinal {
                    text: text.to_owned(),
                    author: frame["agent"].as_str().map(str::to_owned),
                    replace_preview,
                    plain,
                });
            }
            if !subagent {
                state.preview = false;
            }
        }
        "text_discard" => {
            if state.preview {
                let _ = sink.send(AgentEvent::TextDiscard);
                state.preview = false;
            }
        }
        "tool_call" => {
            state.model_activity = true;
            let _ = sink.send(AgentEvent::ToolCall {
                call_id: frame["tool_call_id"].as_str().unwrap_or("").to_string(),
                name: frame["tool"].as_str().unwrap_or("tool").to_string(),
                input: clip(
                    &serde_json::to_string_pretty(&frame["args"]).unwrap_or_default(),
                    8_000,
                ),
            });
        }
        "tool_result" => {
            let _ = sink.send(AgentEvent::ToolResult {
                call_id: frame["tool_call_id"].as_str().unwrap_or("").to_string(),
                ok: !frame["is_error"].as_bool().unwrap_or(false),
                output: clip(frame["output"].as_str().unwrap_or(""), 32_000),
            });
        }
        "approval_required" | "input_required" => {
            state.model_activity = true;
            let _ = sink.send(AgentEvent::ApprovalRequired {
                interrupt_id: frame["interrupt_id"].as_str().unwrap_or("").to_string(),
                kind: frame["kind"].as_str().unwrap_or("tool").to_string(),
                payload: frame["payload"].clone(),
            });
        }
        "usage" => {
            let _ = sink.send(AgentEvent::Usage {
                input: frame["usage"]["input_tokens"].as_u64().unwrap_or(0),
                output: frame["usage"]["output_tokens"].as_u64().unwrap_or(0),
            });
        }
        "done" => {
            return Frame::Finished(match frame["status"].as_str() {
                Some("cancelled") => FinishReason::Cancelled,
                _ if frame["ok"] == true => FinishReason::Completed,
                // Nova's worker returns directly for command reports, without the model's Done event.
                Some("error") if state.direct_report && !state.model_activity && !state.error => {
                    FinishReason::Completed
                }
                _ => FinishReason::Failed,
            });
        }
        _ => {}
    }

    Frame::Continue
}

fn clip(text: &str, limit: usize) -> String {
    if text.chars().count() <= limit {
        return text.to_string();
    }
    text.chars().take(limit).collect::<String>() + "…"
}

impl AgentBackend for NovaAgent {
    fn start<'a>(
        &'a self,
        req: TurnRequest,
        sink: EventSink,
        cancel: CancellationToken,
    ) -> BoxFut<'a, Result<FinishReason, AppError>> {
        Box::pin(async move {
            let slash = req.prompt.trim_start().starts_with('/');
            let pid = uuid::Uuid::new_v4().to_string();
            let prompt = json!({ "type": "prompt", "id": pid, "content": req.prompt });
            if prompt.to_string().len() + 1 > MAX_PROMPT_FRAME {
                return Err(AppError::Agent(
                    "The encoded Nova prompt exceeds 1 MiB".into(),
                ));
            }
            let mut guard = self.session.lock().await;
            let root = req.workspace.as_ref().ok_or(AppError::NoWorkspace)?.root();
            let previous = guard
                .iter()
                .position(|(id, _)| id == &req.chat_id)
                .and_then(|index| guard.remove(index))
                .map(|(_, session)| session)
                .filter(|session| {
                    req.personality.is_none()
                        && session.root == root
                        && session.model_override == req.model
                });
            let previous = if let Some(mut session) = previous {
                matches!(session.child.try_wait(), Ok(None)).then_some(session)
            } else {
                None
            };
            let mut session = match previous {
                Some(session) => session,
                _ => {
                    let session = tokio::select! {
                        _ = cancel.cancelled() => return Ok(FinishReason::Cancelled),
                        session = Session::spawn(root, req.model, req.trust_workspace, req.personality.as_ref()) => session?,
                    };
                    let _ = sink.send(AgentEvent::Notice {
                        level: "info".to_string(),
                        message: if req.session.is_some() {
                            "Nova opened a fresh model session. The previous transcript is still here.".into()
                        } else { format!("Nova ready · {} · {}", session.model, session.cwd) },
                    });
                    if let Some(id) = &session.id {
                        let _ = sink.send(AgentEvent::Session { id: id.clone() });
                    }
                    session
                }
            };
            *self.input.lock().await = Some(session.stdin.clone());

            let outcome = match write_frame(&session.stdin, prompt).await {
                Ok(()) => drive(&mut session, &pid, &sink, &cancel, slash).await,
                Err(err) => Err(err),
            };

            // A session whose child died is not worth keeping; the next turn
            // spawns a fresh one instead of failing forever.
            *self.input.lock().await = None;
            if req.personality.is_none()
                && outcome.is_ok()
                && matches!(session.child.try_wait(), Ok(None))
            {
                if guard.len() >= 8 {
                    guard.pop_front();
                }
                guard.push_back((req.chat_id, session));
            }
            outcome
        })
    }

    fn respond<'a>(
        &'a self,
        interrupt_id: String,
        reply: InterruptReply,
    ) -> BoxFut<'a, Result<(), AppError>> {
        Box::pin(async move {
            let stdin = self
                .input
                .lock()
                .await
                .clone()
                .ok_or_else(|| AppError::Agent("no live Nova session to approve".into()))?;
            let frame = match reply {
                InterruptReply::Tool { approve } => json!({
                    "type": "approval",
                    "interrupt_id": interrupt_id,
                    "decision": if approve { "approve" } else { "reject" },
                }),
                InterruptReply::Plan { approve, feedback } => json!({
                    "type": "interrupt_reply", "interrupt_id": interrupt_id,
                    "result": {
                        "approved": approve,
                        "action": if approve { "proceed_manual" } else { "refine" },
                        "feedback": feedback,
                    },
                }),
                InterruptReply::Question {
                    answer,
                    selected_index,
                } => json!({
                    "type": "interrupt_reply", "interrupt_id": interrupt_id,
                    "result": { "answer": answer, "selected_index": selected_index },
                }),
            };
            write_frame(&stdin, frame).await
        })
    }
}

async fn drive(
    session: &mut Session,
    pid: &str,
    sink: &EventSink,
    cancel: &CancellationToken,
    slash: bool,
) -> Result<FinishReason, AppError> {
    let mut state = TurnFrames {
        slash,
        ..TurnFrames::default()
    };
    loop {
        tokio::select! {
            _ = cancel.cancelled() => {
                let _ = write_frame(&session.stdin, json!({ "type": "cancel" })).await;
                let finished = tokio::time::timeout(Duration::from_secs(4), async {
                    while let Some(line) = session.lines.recv().await {
                        if serde_json::from_str::<Value>(&line).is_ok_and(|frame| frame["type"] == "done" && frame["request_id"] == pid) {
                            return true;
                        }
                    }
                    false
                }).await;
                if !matches!(finished, Ok(true)) {
                    let _ = session.child.kill().await;
                }
                return Ok(FinishReason::Cancelled);
            }
            line = session.lines.recv() => {
                let Some(line) = line else {
                    return Err(AppError::Agent("nova exited during the turn".into()));
                };
                match handle_frame(&line, pid, sink, &mut state) {
                    Frame::Continue => {}
                    Frame::Finished(reason) => return Ok(reason),
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::events::testutil::collect_events;

    /// Feeds one wire line and reports the events it produced plus whether it
    /// ended the turn. The samples are real frames from `nova --mode pipe`.
    fn feed(line: &str, pid: &str) -> (Vec<AgentEvent>, bool) {
        let (channel, seen) = collect_events();
        let frame = handle_frame(line, pid, &channel, &mut TurnFrames::default());
        let events = seen.lock().expect("event sink mutex poisoned").clone();
        (events, matches!(frame, Frame::Finished(_)))
    }

    #[test]
    fn a_delta_extends_the_message() {
        let (events, finished) = feed(
            r#"{"schema_version":1,"type":"text_delta","request_id":"p1","content":"hi"}"#,
            "p1",
        );
        assert!(!finished);
        assert_eq!(
            events,
            vec![AgentEvent::MessagePreview {
                text: "hi".to_string()
            }]
        );
    }

    #[test]
    fn final_text_replaces_the_preview_and_subagent_text_does_not() {
        let (sink, seen) = collect_events();
        let mut state = TurnFrames::default();
        for line in [
            r#"{"type":"text_delta","request_id":"p1","content":"preview"}"#,
            r#"{"type":"text","request_id":"p1","content":"child","agent":"Researcher","is_subagent":true}"#,
            r#"{"type":"text","request_id":"p1","content":"final","agent":"Nova"}"#,
        ] {
            handle_frame(line, "p1", &sink, &mut state);
        }
        let events = seen.lock().unwrap();
        assert!(matches!(
            &events[1],
            AgentEvent::MessageFinal {
                replace_preview: false,
                ..
            }
        ));
        assert!(
            matches!(&events[2], AgentEvent::MessageFinal { replace_preview: true, text, .. } if text == "final")
        );
    }

    #[test]
    fn only_a_direct_slash_report_recovers_legacy_completion() {
        let (sink, _) = collect_events();
        for saw_error in [false, true] {
            let mut state = TurnFrames {
                slash: true,
                ..TurnFrames::default()
            };
            handle_frame(
                r#"{"type":"text","request_id":"p1","content":"command report"}"#,
                "p1",
                &sink,
                &mut state,
            );
            if saw_error {
                handle_frame(
                    r#"{"type":"error","request_id":"p1","code":"execution_error","message":"failed"}"#,
                    "p1",
                    &sink,
                    &mut state,
                );
            }
            let reason = handle_frame(
                r#"{"type":"done","request_id":"p1","ok":false,"status":"error"}"#,
                "p1",
                &sink,
                &mut state,
            );
            assert!(matches!(
                (reason, saw_error),
                (Frame::Finished(FinishReason::Completed), false)
                    | (Frame::Finished(FinishReason::Failed), true)
            ));
        }
    }

    #[test]
    fn done_ends_the_turn() {
        let (_, finished) = feed(r#"{"type":"done","id":"p1","ok":true}"#, "p1");
        assert!(finished);
    }

    #[test]
    fn a_done_for_an_earlier_turn_is_ignored() {
        // The frame a cancelled turn still emits must not end the next one.
        let (events, finished) = feed(r#"{"type":"done","id":"old","ok":true}"#, "new");
        assert!(!finished, "a stale done ended the live turn");
        assert!(events.is_empty());
    }

    #[test]
    fn a_stale_delta_is_ignored() {
        let (events, _) = feed(
            r#"{"type":"text_delta","request_id":"old","content":"stale"}"#,
            "new",
        );
        assert!(events.is_empty());
    }

    #[test]
    fn unparseable_lines_are_never_fatal() {
        // The protocol explicitly allows noise; it must not end the turn.
        let (events, finished) = feed("not json at all", "p1");
        assert!(!finished);
        assert!(events.is_empty());
    }

    #[test]
    fn approval_frames_carry_the_id_the_reply_needs() {
        let (events, finished) = feed(
            r#"{"type":"approval_required","request_id":"p1","interrupt_id":"inst:7","kind":"tool","payload":{"action_requests":[{"name":"write_file"}]}}"#,
            "p1",
        );
        assert!(!finished);
        match &events[0] {
            AgentEvent::ApprovalRequired {
                interrupt_id,
                kind,
                payload,
            } => {
                assert_eq!(interrupt_id, "inst:7");
                assert_eq!(kind, "tool");
                assert_eq!(payload["action_requests"][0]["name"], "write_file");
            }
            other => panic!("expected an approval, got {other:?}"),
        }
    }

    #[test]
    fn errors_are_notices_not_turn_enders() {
        let (events, finished) = feed(
            r#"{"type":"error","code":"stale_interrupt","message":"already answered"}"#,
            "p1",
        );
        assert!(!finished);
        match &events[0] {
            AgentEvent::Notice { level, message } => {
                assert_eq!(level, "error");
                assert!(message.contains("stale_interrupt"));
            }
            other => panic!("expected a notice, got {other:?}"),
        }
    }

    /// The only test that talks to a real `nova`: it proves the spawn, the
    /// `ready` handshake, frame routing and turn end against the actual CLI
    /// rather than a recorded sample. Ignored by default — it needs `nova` on
    /// PATH and spends a model call. Run with:
    /// `cargo test --lib live_turn -- --ignored`
    #[tokio::test]
    #[ignore = "spawns the real nova CLI and spends a model call"]
    async fn a_live_turn_streams_then_finishes() {
        use crate::workspace::WorkspaceGuard;

        let req = TurnRequest {
            chat_id: uuid::Uuid::new_v4().to_string(),
            model: None,
            effort: None,
            personality: None,
            session: None,
            trust_workspace: true,
            prompt: "Reply with exactly the word: pong".to_string(),
            workspace: Some(WorkspaceGuard::new(std::env::temp_dir()).expect("temp dir")),
        };
        let (sink, seen) = collect_events();
        let reason = NovaAgent::default()
            .start(req, sink, CancellationToken::new())
            .await
            .expect("a live turn runs to completion");

        let events = seen.lock().expect("event sink mutex poisoned").clone();
        let text: String = events
            .iter()
            .filter_map(|event| match event {
                AgentEvent::MessageFinal { text, .. } => Some(text.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(reason, FinishReason::Completed, "events: {events:?}");
        assert!(text.to_lowercase().contains("pong"), "streamed {text:?}");
    }
}
