pub mod cli;
pub mod local;
pub mod models;
pub mod nova;
pub mod permissions;
pub mod process;
pub mod protocol;
pub mod requests;
pub mod teams;

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
    pub turn_id: String,
    pub read_only: bool,
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
    /// Nova Desktop Tools, when this launch is hosting them. A backend that
    /// cannot be told about an MCP server ignores it; the registry says which
    /// ones those are, so the pane never claims otherwise.
    pub tools: Option<crate::tools::Bridge>,
}

impl TurnRequest {
    pub fn system_prompt(&self) -> String {
        desktop_instructions(
            self.personality.as_ref(),
            self.tools.is_some(),
            self.read_only,
        )
    }
}

pub(super) fn desktop_instructions(
    profile: Option<&Personality>,
    tools: bool,
    read_only: bool,
) -> String {
    let mut text = String::from("You are running inside Nova Desktop, a graphical desktop app. The user is chatting in its task conversation, not a standalone CLI. References to this app, its tasks, board, files, terminal, or browser mean Nova Desktop's visible interface. Answer the user in this conversation.");
    if read_only {
        text.push_str(" This is an enforced read-only investigation. Desktop tools are withheld. Return findings without editing files or attempting Desktop actions.");
    } else if tools {
        text.push_str(" Desktop integration is configured through the nova-desktop MCP server (displayed as nova_desktop in some harnesses). For requested Desktop actions, discover its tools through your harness's tool-discovery mechanism and use the advertised names and schemas. Use list_tasks/read_task for the app's task list and conversations, create_task/rename_task/set_task_stage for its visible tasks and board, open_file/open_terminal for its panes, and open_browser/read_browser/browser_click/browser_type for its embedded browser. Your built-in task checklist is not the Desktop task list. Use the Desktop tools for these actions rather than suggesting CLI slash commands, editing the app's storage, or opening a separate browser. Do not ask the user to configure or enable tools that are already exposed. If discovery or a call fails, report the actual limitation; never claim a UI change without a successful tool result. Use tools when the requested action calls for them, not for ordinary conversational answers.");
    } else {
        text.push_str(" No Desktop tool bridge is provided for this session. Do not claim access to the app's tasks, board, or browser. If asked to act on them, explain that Desktop tools are unavailable and point to Settings > Tools.");
    }
    if let Some(profile) = profile {
        text.push_str("\n\nRole instructions:\n");
        text.push_str(&profile.system_prompt);
    }
    text
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
    fn capabilities(&self) -> protocol::Capabilities {
        protocol::Capabilities::default()
    }
    /// Drives one turn. Must return promptly once `cancel` fires and must not
    /// outlive it. Must not emit `TurnFinished`: `drive_turn` does that once.
    fn start<'a>(
        &'a self,
        req: TurnRequest,
        sink: EventSink,
        cancel: CancellationToken,
    ) -> BoxFut<'a, Result<FinishReason, AppError>>;

    fn steer<'a>(&'a self, _turn_id: &'a str, _text: &'a str) -> BoxFut<'a, Result<(), AppError>> {
        Box::pin(async {
            Err(AppError::Tools(
                "this harness does not accept mid-turn input; use queue_message".into(),
            ))
        })
    }

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
    fn prewarm<'a>(&'a self, _req: TurnRequest) -> BoxFut<'a, Result<(), AppError>> {
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
        for request in if reason == FinishReason::Completed {
            requests::read(path)
        } else {
            Vec::new()
        } {
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

#[cfg(test)]
mod context_tests {
    use super::*;

    #[test]
    fn ordinary_chats_get_host_context_and_concrete_desktop_actions() {
        let prompt = desktop_instructions(None, true, false);
        assert!(prompt.starts_with("You are running inside Nova Desktop"));
        assert!(prompt.contains("create_task/rename_task"));
        assert!(prompt.contains("embedded browser"));
        assert!(prompt.contains("built-in task checklist is not the Desktop task list"));
        assert!(prompt.contains("never claim a UI change without a successful tool result"));
    }

    #[test]
    fn unavailable_and_read_only_sessions_do_not_advertise_desktop_actions() {
        for (tools, read_only) in [(false, false), (false, true), (true, true)] {
            let prompt = desktop_instructions(None, tools, read_only);
            assert!(prompt.contains("Nova Desktop"));
            assert!(!prompt.contains("create_task"));
            assert!(!prompt.contains("discover its tools"));
            assert_eq!(prompt.contains("enforced read-only"), read_only);
        }
    }

    #[test]
    fn role_instructions_extend_host_context_and_invalidate_cached_context() {
        let mut profile = Personality {
            role: "serena".into(),
            system_prompt: "Inspect the project.".into(),
        };
        let first = desktop_instructions(Some(&profile), true, false);
        assert!(first.starts_with("You are running inside Nova Desktop"));
        assert!(first.ends_with("Inspect the project."));
        profile.system_prompt = "Inspect a different scope.".into();
        assert_ne!(first, desktop_instructions(Some(&profile), true, false));
        assert_ne!(first, desktop_instructions(Some(&profile), false, false));
    }
}
