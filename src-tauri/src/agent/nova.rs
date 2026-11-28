use std::collections::{HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;

use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin};
use tokio::sync::{mpsc, oneshot, Mutex};
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

/// Chat sessions plus the four crew roles stay warm; older entries are evicted.
const SESSION_CACHE: usize = 12;

/// How long a `/clear` handshake may take before a reused session is judged
/// wedged and a fresh process is spawned instead.
const CLEAR_TIMEOUT: Duration = Duration::from_secs(10);

/// Talks to `nova --mode pipe`: a long-lived child that owns one session and
/// speaks newline-delimited JSON on stdio.
///
/// The child is spawned once and reused, so the conversation keeps its context
/// across turns. Frame catalogue: `novacode_cli/headless/pipe.py`.
#[derive(Default)]
pub struct NovaAgent {
    ready: std::sync::atomic::AtomicBool,
    steering: std::sync::atomic::AtomicBool,
    /// One session, one turn at a time: the lock is held for the whole turn, so
    /// a second prompt queues behind the first instead of interleaving frames.
    session: Mutex<VecDeque<(String, Session)>>,
    /// Held outside `session` so an approval reply can be written while a turn
    /// owns the session lock.
    input: Mutex<Option<ActiveInput>>,
    /// Workspace roots with a live managed `nova-desktop` entry in their
    /// `.nova/mcp.json`. The entry is written before the first session for the
    /// workspace and removed when its last one ends.
    injected: std::sync::Mutex<HashSet<PathBuf>>,
}

struct ActiveInput {
    turn_id: String,
    stdin: Arc<Mutex<ChildStdin>>,
    steer: Option<mpsc::Sender<SteerRequest>>,
}

struct SteerRequest {
    text: String,
    reply: oneshot::Sender<Result<(), AppError>>,
}

struct Session {
    /// `kill_on_drop` is armed: dropping the session ends the agent process.
    child: Child,
    _job: super::process::ChildJob,
    stdin: Arc<Mutex<ChildStdin>>,
    lines: mpsc::Receiver<String>,
    cwd: String,
    model: String,
    id: Option<String>,
    root: std::path::PathBuf,
    model_override: Option<String>,
    system_prompt: String,
    supports_steer: bool,
    tools_file: Option<PathBuf>,
    startup_ms: u64,
    requests_file: Option<PathBuf>,
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
        requests_file: Option<&Path>,
        tools: Option<&crate::tools::Bridge>,
    ) -> Result<Self, AppError> {
        // Debug builds time the boot: "slow to reply" is a number, and this is
        // the half of it the app can act on.
        let booted_at = std::time::Instant::now();
        let mut command = if std::env::var_os("NOVA_BIN").is_none() || personality.is_some() {
            super::cli::nova_desktop(personality, tools.is_some())?
        } else {
            super::cli::command("nova")?
        };
        command.args(["--mode", "pipe"]).env("NO_COLOR", "1");
        if let Some(bridge) = tools {
            let credentials = crate::tools::nova_entry::launch_credentials(bridge)?;
            command
                .env("NOVA_DESKTOP_MCP_FILE", credentials)
                .env("NOVA_TOOLS_SHIM", &bridge.shim);
        }
        // The documented drop point for agent-originated requests. Passing it is
        // the whole contract: a harness that ignores the variable is unaffected.
        if let Some(path) = requests_file {
            command.env(super::requests::ENV, path);
        }
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

        let job = super::process::ChildJob::attach(&child)?;
        let startup_ms = booted_at.elapsed().as_millis() as u64;
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
            _job: job,
            stdin: Arc::clone(&stdin),
            lines,
            cwd: root.display().to_string(),
            model: String::new(),
            id: None,
            root: root.to_path_buf(),
            model_override: model,
            system_prompt: super::desktop_instructions(personality, tools.is_some(), false),
            supports_steer: false,
            tools_file: crate::tools::nova_entry::workspace_credentials(root),
            startup_ms,
            requests_file: requests_file.map(Path::to_path_buf),
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
                if cfg!(debug_assertions) {
                    eprintln!(
                        "nova boot: {} ms in {}",
                        booted_at.elapsed().as_millis(),
                        root.display()
                    );
                }
                session.id = frame["session_id"].as_str().map(str::to_owned);
                session.model = frame["model"].as_str().unwrap_or("unknown").to_string();
                session.supports_steer = frame["capabilities"]
                    .as_array()
                    .is_some_and(|capabilities| capabilities.iter().any(|value| value == "steer"));
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

/// Clears a warm session's conversation in place (`/clear` keeps the agent, the
/// tools and the process; it resets the thread, todos and steering). Returns
/// false when the command was rejected, timed out or the child died, so the
/// caller spawns a fresh session rather than continuing a polluted context.
async fn reset_conversation(session: &mut Session) -> bool {
    let frame = json!({
        "type": "command",
        "id": uuid::Uuid::new_v4().to_string(),
        "content": "/clear",
    });
    if write_frame(&session.stdin, frame).await.is_err() {
        return false;
    }
    let wait = async {
        while let Some(line) = session.lines.recv().await {
            let Ok(value) = serde_json::from_str::<Value>(&line) else {
                continue;
            };
            if value["type"].as_str() == Some("command_result") {
                return value["ok"].as_bool().unwrap_or(false);
            }
        }
        false
    };
    tokio::time::timeout(CLEAR_TIMEOUT, wait)
        .await
        .unwrap_or(false)
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
        "reasoning_delta" | "thinking_delta" => {
            if let Some(text) = frame["content"].as_str().filter(|text| !text.is_empty()) {
                state.model_activity = true;
                let _ = sink.send(AgentEvent::ReasoningDelta {
                    text: clip(text, 64000),
                });
            }
        }
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
        "subagent_task" => {
            let _ = sink.send(AgentEvent::SubagentTask {
                task_id: frame["task_id"].as_str().unwrap_or("").to_string(),
                status: frame["status"].as_str().unwrap_or("running").to_string(),
                label: frame["label"].as_str().map(str::to_owned),
                subagent_type: frame["subagent_type"].as_str().map(str::to_owned),
                duration_ms: frame["duration_ms"].as_u64(),
            });
        }
        "subagent_preview" => {
            if let Some(text) = frame["text"].as_str().filter(|text| !text.is_empty()) {
                let _ = sink.send(AgentEvent::SubagentPreview {
                    task_id: frame["task_id"].as_str().unwrap_or("").to_string(),
                    text: clip(text, 8_000),
                });
            }
        }
        "steer_undelivered" => {
            let count = frame["count"].as_u64().unwrap_or(1);
            let _ = sink.send(AgentEvent::Notice {
                level: "error".into(),
                message: format!(
                    "The Nova turn ended before consuming {count} steering request(s)."
                ),
            });
        }
        "done" => {
            if frame["status"].as_str() == Some("approval_timeout") {
                let _ = sink.send(AgentEvent::Notice {
                    level: "error".to_string(),
                    message: "An approval was not answered in time — Nova stopped waiting."
                        .to_string(),
                });
            }
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

impl NovaAgent {
    /// Removes the workspace's managed entry when no cached session for that
    /// root remains: the last Nova session for the workspace has ended.
    fn retire_if_last(&self, root: &Path, guard: &VecDeque<(String, Session)>) {
        if guard.iter().any(|(_, session)| session.root == root) {
            return;
        }
        let injected = self
            .injected
            .lock()
            .expect("injected mutex poisoned")
            .remove(root);
        if injected {
            crate::tools::nova_entry::remove_workspace_entry(root);
        }
    }

    /// Writes the managed `nova-desktop` entry for a workspace before its
    /// first session boots: the child reads MCP servers from the workspace
    /// config at startup and never again.
    fn write_injection(&self, root: &Path, bridge: &crate::tools::Bridge) -> Result<(), AppError> {
        let credentials = crate::tools::nova_entry::launch_credentials(bridge)?;
        crate::tools::nova_entry::ensure_workspace_entry(root, &credentials, &bridge.shim)?;
        self.injected
            .lock()
            .expect("injected mutex poisoned")
            .insert(root.to_path_buf());
        Ok(())
    }
}

impl AgentBackend for NovaAgent {
    fn capabilities(&self) -> super::protocol::Capabilities {
        let ready = self.ready.load(std::sync::atomic::Ordering::SeqCst);
        super::protocol::Capabilities {
            streaming: ready,
            permissions: ready,
            session_reuse: ready,
            steering: ready && self.steering.load(std::sync::atomic::Ordering::SeqCst),
            ..Default::default()
        }
    }
    fn start<'a>(
        &'a self,
        req: TurnRequest,
        sink: EventSink,
        cancel: CancellationToken,
    ) -> BoxFut<'a, Result<FinishReason, AppError>> {
        Box::pin(async move {
            let turn_started = std::time::Instant::now();
            let system_prompt = req.system_prompt();
            let slash = req.prompt.trim_start().starts_with('/');
            let pid = req.turn_id;
            let prompt = json!({ "type": "prompt", "id": pid, "content": req.prompt });
            if prompt.to_string().len() + 1 > MAX_PROMPT_FRAME {
                return Err(AppError::Agent(
                    "The encoded Nova prompt exceeds 1 MiB".into(),
                ));
            }
            let root = req
                .workspace
                .as_ref()
                .ok_or(AppError::NoWorkspace)?
                .root()
                .to_path_buf();
            let mut guard = tokio::select! { _ = cancel.cancelled() => return Ok(FinishReason::Cancelled), guard = self.session.lock() => guard };

            // The managed entry has to exist before the child boots. When the
            // workspace cannot take it (unwritable, or a config this code
            // cannot parse), the turn continues without the desktop tools and
            // the reason lands in the transcript, rather than the session
            // silently promising tools it cannot reach.
            if req.tools.is_none() && self.injected.lock().expect("injected mutex").remove(&root) {
                crate::tools::nova_entry::remove_workspace_entry(&root);
            }
            if let Some(bridge) = req.tools.as_ref() {
                if let Err(error) = self.write_injection(&root, bridge) {
                    let _ = sink.send(AgentEvent::Notice {
                        level: "error".into(),
                        message: format!("Desktop tools are unavailable: {error}"),
                    });
                }
            }
            let tools_file = req
                .tools
                .as_ref()
                .map(crate::tools::nova_entry::launch_credentials)
                .transpose()?;

            // Chat turns reuse their chat's session. Personality turns reuse the
            // role's warm process — the conversation is reset below, so each
            // stage still starts with a clean context but pays no boot.
            let key = match &req.personality {
                Some(profile) => format!(
                    "personality:{}:{}:{}:{}",
                    req.chat_id,
                    profile.role,
                    root.display(),
                    req.model.as_deref().unwrap_or("")
                ),
                None => req.chat_id.clone(),
            };
            let previous = guard
                .iter()
                .position(|(id, _)| id == &key)
                .and_then(|index| guard.remove(index))
                .map(|(_, session)| session)
                .filter(|session| {
                    session.root == root
                        && session.model_override == req.model
                        && session.system_prompt == system_prompt
                        && session.tools_file == tools_file
                        && session.requests_file == req.requests_file
                });
            let previous = if let Some(mut session) = previous {
                matches!(session.child.try_wait(), Ok(None)).then_some(session)
            } else {
                None
            };
            // No cached chat session: adopt a prewarmed one for this workspace,
            // if the workspace was opened ahead of the first turn. Only plain
            // turns match — a personality or model override spawns its own.
            let mut previous = previous;
            // Whether this turn is the one that spends the workspace's warm
            // session, which is the only turn that owes the next new chat a
            // replacement. Booting one after every turn would put an eight-
            // second Nova start behind the reply the user is already typing.

            if previous.is_none() && req.personality.is_none() && req.model.is_none() {
                let prewarm_key = format!("prewarm:{}", root.display());
                if let Some(index) = guard.iter().position(|(id, _)| *id == prewarm_key) {
                    let (_, mut candidate) = guard.remove(index).expect("position was valid");
                    if candidate.root == root
                        && candidate.system_prompt == system_prompt
                        && candidate.tools_file == tools_file
                        && candidate.requests_file == req.requests_file
                        && matches!(candidate.child.try_wait(), Ok(None))
                    {
                        previous = Some(candidate);
                    }
                }
            }
            // A reused process still owns the previous stage's conversation, so
            // clear it; a wedged or rejected reset falls back to a fresh spawn.
            let mut reused = None;
            if let Some(mut session) = previous {
                if req.personality.is_none() || reset_conversation(&mut session).await {
                    reused = Some(session);
                }
            }
            let warm = reused.is_some();
            if !warm {
                self.ready.store(false, std::sync::atomic::Ordering::SeqCst);
            }
            let mut session = match reused {
                Some(session) => session,
                _ => {
                    let session = tokio::select! {
                        _ = cancel.cancelled() => return Ok(FinishReason::Cancelled),
                        session = Session::spawn(&root, req.model.clone(), req.trust_workspace,
                            req.personality.as_ref(), req.requests_file.as_deref(), req.tools.as_ref()) => session?,
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
            let _ = sink.send(AgentEvent::Timing {
                phase: "processStartup".into(),
                ms: if warm { 0 } else { session.startup_ms },
            });
            let _ = sink.send(AgentEvent::Timing {
                phase: "protocolReady".into(),
                ms: turn_started.elapsed().as_millis() as u64,
            });
            self.ready.store(true, std::sync::atomic::Ordering::SeqCst);
            self.steering
                .store(session.supports_steer, std::sync::atomic::Ordering::SeqCst);
            let (steer, inbox) = mpsc::channel(1);
            let mut input = self.input.lock().await;
            let written = write_frame(&session.stdin, prompt).await;
            if written.is_ok() {
                *input = Some(ActiveInput {
                    turn_id: pid.clone(),
                    stdin: session.stdin.clone(),
                    steer: session.supports_steer.then_some(steer),
                });
            }
            drop(input);
            let outcome = match written {
                Ok(()) => drive(&mut session, &pid, &sink, &cancel, slash, inbox).await,
                Err(err) => Err(err),
            };

            // A session whose child died is not worth keeping; the next turn
            // spawns a fresh one instead of failing forever. Either way, a
            // session that leaves the cache may have been the workspace's
            // last one — its managed entry goes with it.
            *self.input.lock().await = None;
            if outcome.is_ok() && matches!(session.child.try_wait(), Ok(None)) {
                if guard.len() >= SESSION_CACHE {
                    if let Some((_, evicted)) = guard.pop_front() {
                        self.retire_if_last(&evicted.root, &guard);
                    }
                }
                guard.push_back((key, session));
            } else {
                self.retire_if_last(&root, &guard);
            }
            // Replenishment is requested by the UI after completion, never on the reply path.
            drop(guard);
            outcome
        })
    }

    fn steer<'a>(&'a self, turn_id: &'a str, text: &'a str) -> BoxFut<'a, Result<(), AppError>> {
        Box::pin(async move {
            let (reply, response) = oneshot::channel();
            {
                let input = self.input.lock().await;
                let active = input
                    .as_ref()
                    .filter(|active| active.turn_id == turn_id)
                    .ok_or_else(|| {
                        AppError::Tools("the Nova turn is starting or has already finished".into())
                    })?;
                let sender = active.steer.as_ref().ok_or_else(|| AppError::Tools(
                    "this Nova CLI does not advertise live steering; update Nova or use queue_message".into(),
                ))?;
                sender
                    .try_send(SteerRequest {
                        text: text.to_owned(),
                        reply,
                    })
                    .map_err(|_| {
                        AppError::Tools(
                            "the turn ended or another steering request is pending".into(),
                        )
                    })?;
            }
            tokio::time::timeout(Duration::from_secs(8), response)
                .await
                .map_err(|_| {
                    AppError::Tools("Nova did not acknowledge the steering request in time".into())
                })?
                .map_err(|_| {
                    AppError::Tools("the Nova turn ended before acknowledging steering".into())
                })?
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
                .as_ref()
                .map(|active| active.stdin.clone())
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

    fn prewarm<'a>(&'a self, req: TurnRequest) -> BoxFut<'a, Result<(), AppError>> {
        Box::pin(async move {
            let system_prompt = req.system_prompt();
            let mut guard = self.session.lock().await;
            let root = req.workspace.as_ref().ok_or(AppError::NoWorkspace)?.root();
            if let Some(bridge) = &req.tools {
                self.write_injection(root, bridge)?;
            }
            let key = format!("prewarm:{}", root.display());
            let tools_file = req
                .tools
                .as_ref()
                .map(crate::tools::nova_entry::launch_credentials)
                .transpose()?;
            if guard.iter().any(|(id, session)| {
                (id == &req.chat_id || id == &key)
                    && session.root == root
                    && session.model_override == req.model
                    && session.system_prompt == system_prompt
                    && session.tools_file == tools_file
                    && session.requests_file == req.requests_file
            }) {
                return Ok(());
            }
            if let Some(at) = guard.iter().position(|(id, _)| id == &key) {
                guard.remove(at);
            }
            // Spawned silently: the session sits in the cache until a turn
            // adopts it, and boot cost is paid while the user is not waiting.
            // A prewarmed session is never asked to run a turn, so it gets no
            // request file; the key below cannot match a chat or personality.
            let session = Session::spawn(
                root,
                req.model,
                true,
                None,
                req.requests_file.as_deref(),
                req.tools.as_ref(),
            )
            .await?;
            if guard.len() >= SESSION_CACHE {
                if let Some((_, evicted)) = guard.pop_front() {
                    self.retire_if_last(&evicted.root, &guard);
                }
            }
            self.ready.store(true, std::sync::atomic::Ordering::SeqCst);
            self.steering
                .store(session.supports_steer, std::sync::atomic::Ordering::SeqCst);
            guard.push_back((key, session));
            Ok(())
        })
    }
}

async fn drive(
    session: &mut Session,
    pid: &str,
    sink: &EventSink,
    cancel: &CancellationToken,
    slash: bool,
    mut inbox: mpsc::Receiver<SteerRequest>,
) -> Result<FinishReason, AppError> {
    let mut state = TurnFrames {
        slash,
        ..TurnFrames::default()
    };
    let mut pending = None::<(String, SteerRequest)>;
    loop {
        tokio::select! {
            _ = cancel.cancelled() => {
                let _ = write_frame(&session.stdin, json!({ "type": "cancel" })).await;
                let finished = tokio::time::timeout(Duration::from_secs(4), async {
                    while let Some(line) = session.lines.recv().await {
                        if let Ok(frame) = serde_json::from_str::<Value>(&line) {
                            if frame["request_id"] != pid {
                                continue;
                            }
                            if frame["type"] == "done" {
                                return true;
                            }
                            if frame["type"] == "steer_undelivered" {
                                handle_frame(&line, pid, sink, &mut state);
                            }
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
                if let Some(frame) = pending.as_ref().and_then(|_| serde_json::from_str::<Value>(&line).ok()) {
                    if frame["type"] == "steer_result" && frame["request_id"] == pid
                        && pending.as_ref().is_some_and(|(id, _)| frame["id"] == id.as_str())
                    {
                        if let Some((_, request)) = pending.take() {
                            let result = if frame["ok"] == true {
                                Ok(())
                            } else {
                                Err(AppError::Tools(frame["error"].as_str().unwrap_or("Nova refused steering").to_owned()))
                            };
                            let _ = request.reply.send(result);
                        }
                        continue;
                    }
                }
                match handle_frame(&line, pid, sink, &mut state) {
                    Frame::Continue => {}
                    Frame::Finished(reason) => return Ok(reason),
                }
            }
            Some(request) = inbox.recv(), if pending.is_none() => {
                if request.reply.is_closed() {
                    continue;
                }
                let id = uuid::Uuid::new_v4().to_string();
                let frame = json!({"type": "steer", "id": id, "request_id": pid, "content": request.text});
                tokio::time::timeout(Duration::from_secs(5), write_frame(&session.stdin, frame))
                    .await
                    .map_err(|_| AppError::Agent("could not write to the Nova pipe in time".into()))??;
                pending = Some((id, request));
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
    fn thinking_deltas_stay_out_of_the_final_answer_and_ignore_other_turns() {
        let (sink, seen) = crate::events::testutil::collect_events();
        let mut state = TurnFrames::default();
        handle_frame(
            r#"{"type":"reasoning_delta","request_id":"other","content":"stale"}"#,
            "p1",
            &sink,
            &mut state,
        );
        handle_frame(
            r#"{"type":"reasoning_delta","request_id":"p1","content":"Checking"}"#,
            "p1",
            &sink,
            &mut state,
        );
        assert_eq!(
            *seen.lock().unwrap(),
            vec![AgentEvent::ReasoningDelta {
                text: "Checking".into()
            }]
        );
        assert!(!state.preview);
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

    #[test]
    fn subagent_rows_stream_then_upsert() {
        let (sink, seen) = collect_events();
        let mut state = TurnFrames::default();
        for line in [
            r#"{"type":"subagent_task","request_id":"p1","task_id":"ptc_1","status":"running","subagent_type":"research","label":"research: docling"}"#,
            r#"{"type":"subagent_preview","request_id":"p1","task_id":"ptc_1","kind":"text","text":"reading the spec"}"#,
            r#"{"type":"subagent_task","request_id":"p1","task_id":"ptc_1","status":"done","duration_ms":42000}"#,
            // A row for another prompt belongs to another turn.
            r#"{"type":"subagent_task","request_id":"old","task_id":"ptc_2","status":"running"}"#,
        ] {
            handle_frame(line, "p1", &sink, &mut state);
        }
        let events = seen.lock().expect("event sink mutex poisoned").clone();
        assert_eq!(
            events,
            vec![
                AgentEvent::SubagentTask {
                    task_id: "ptc_1".into(),
                    status: "running".into(),
                    label: Some("research: docling".into()),
                    subagent_type: Some("research".into()),
                    duration_ms: None,
                },
                AgentEvent::SubagentPreview {
                    task_id: "ptc_1".into(),
                    text: "reading the spec".into(),
                },
                AgentEvent::SubagentTask {
                    task_id: "ptc_1".into(),
                    status: "done".into(),
                    label: None,
                    subagent_type: None,
                    duration_ms: Some(42000),
                },
            ]
        );
    }

    #[test]
    fn an_approval_timeout_fails_the_turn_with_a_word() {
        let (events, finished) = feed(
            r#"{"type":"done","request_id":"p1","status":"approval_timeout","ok":false}"#,
            "p1",
        );
        assert!(finished);
        assert!(matches!(
            &events[0],
            AgentEvent::Notice { level, message }
                if level == "error" && message.contains("approval")
        ));
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
            read_only: false,
            turn_id: uuid::Uuid::new_v4().to_string(),
            chat_id: uuid::Uuid::new_v4().to_string(),
            model: None,
            effort: None,
            personality: None,
            session: None,
            trust_workspace: true,
            requests_file: None,
            tools: None,
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
