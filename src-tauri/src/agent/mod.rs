pub mod cli;
pub mod local;
pub mod models;
pub mod nova;
pub mod requests;

use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use tauri::ipc::Channel;
use tokio_util::sync::CancellationToken;

use crate::error::AppError;
use crate::events::{AgentEvent, FinishReason};
use crate::workspace::WorkspaceGuard;

pub type BoxFut<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// The stream a backend writes to. One channel per turn.
pub type EventSink = Channel<AgentEvent>;

/// Name used when no specific backend is requested.
pub const DEFAULT_BACKEND: &str = "nova";

/// Everything a backend needs for one turn, built by the command layer so a
/// backend never resolves paths, tokens or capabilities itself.
pub struct TurnRequest {
    pub prompt: String,
    pub workspace: Option<WorkspaceGuard>,
    pub chat_id: String,
    pub model: Option<String>,
    pub effort: Option<String>,
    pub session: Option<String>,
    pub trust_workspace: bool,
    pub personality: Option<Personality>,
    /// The request file this turn may write, passed to the harness as
    /// `NOVA_DESKTOP_REQUESTS`. Keyed by task (not by turn) so a reused session
    /// keeps a valid path for as long as it lives.
    pub requests_file: Option<std::path::PathBuf>,
}

#[derive(Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Personality {
    pub role: String,
    pub system_prompt: String,
}

impl Personality {
    pub fn validate(&self, harness: &str) -> Result<(), AppError> {
        let supported = matches!(self.role.as_str(), "nova" | "serena" | "ralph" | "nora")
            && cli::AGENTS.iter().any(|&(id, _)| id == harness);
        if !supported
            || self.system_prompt.trim().is_empty()
            || self.system_prompt.len() > 8192
            || self.system_prompt.contains('\0')
        {
            return Err(AppError::Agent(
                "Invalid personality instructions or unsupported harness".into(),
            ));
        }
        Ok(())
    }
}

/// The swap point of the whole app.
///
/// A sidecar process or an HTTP/SSE backend is a new file implementing this
/// trait plus one `register` call in `AppState::new`; no command and no React
/// component changes.
pub trait AgentBackend: Send + Sync {
    /// Drives one turn. Must return promptly once `cancel` fires and must not
    /// outlive it. Must not emit `TurnFinished`: `drive_turn` does that once.
    fn start<'a>(
        &'a self,
        req: TurnRequest,
        sink: EventSink,
        cancel: CancellationToken,
    ) -> BoxFut<'a, Result<FinishReason, AppError>>;

    /// Answers an `ApprovalRequired` event. Backends that never ask for a
    /// decision keep this default and reject the call instead of pretending.
    fn respond<'a>(
        &'a self,
        _interrupt_id: String,
        _reply: crate::events::InterruptReply,
    ) -> BoxFut<'a, Result<(), AppError>> {
        Box::pin(async {
            Err(AppError::Agent(
                "this backend does not accept approvals".to_string(),
            ))
        })
    }

    /// Spawns a warm session for the workspace so the first turn skips the
    /// boot. Only called for trusted workspaces; backends without a persistent
    /// process keep this no-op.
    fn prewarm<'a>(&'a self, _root: &'a std::path::Path) -> BoxFut<'a, Result<(), AppError>> {
        Box::pin(async { Ok(()) })
    }
}

/// Runs a turn. The command sends its closing event after releasing the active slot.
pub async fn drive_turn(
    agent: &dyn AgentBackend,
    req: TurnRequest,
    sink: EventSink,
    cancel: CancellationToken,
) -> FinishReason {
    let _ = sink.send(AgentEvent::TurnStarted);
    // A leftover request from an earlier turn must not fire now, so the file is
    // cleared before the turn and read only after it: a harness cannot start work
    // while it is still mid-turn, and a restart cannot replay an old request.
    let requests_file = req.requests_file.clone();
    if let Some(path) = &requests_file {
        requests::clear(path);
    }
    let reason = match agent.start(req, sink.clone(), cancel).await {
        Ok(reason) => reason,
        Err(err) => {
            let _ = sink.send(AgentEvent::TurnError {
                message: err.to_string(),
                kind: err.kind().to_owned(),
            });
            FinishReason::Failed
        }
    };
    if let Some(path) = &requests_file {
        for request in requests::read(path) {
            let event = match request {
                requests::Request::Wakeup {
                    prompt,
                    agent,
                    after_seconds,
                } => AgentEvent::WakeupRequested {
                    prompt,
                    agent,
                    after_seconds,
                },
                requests::Request::ThreadMessage { thread, text } => {
                    AgentEvent::ThreadMessage { thread, text }
                }
            };
            let _ = sink.send(event);
        }
        requests::clear(path);
    }
    reason
}

/// Name → backend, so adding a backend never touches the command signatures.
#[derive(Clone, Default)]
pub struct AgentRegistry {
    backends: HashMap<String, Arc<dyn AgentBackend>>,
}

impl AgentRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register(&mut self, name: impl Into<String>, backend: Arc<dyn AgentBackend>) {
        self.backends.insert(name.into(), backend);
    }

    pub fn get(&self, id: &str) -> Result<Arc<dyn AgentBackend>, AppError> {
        self.backends
            .get(id)
            .cloned()
            .ok_or_else(|| AppError::Agent(format!("Unknown agent: {id}")))
    }
}
