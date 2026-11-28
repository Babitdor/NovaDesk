use std::collections::HashMap;
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::{Arc, Mutex};

use super::{AgentBackend, EventSink};
use crate::error::AppError;
use crate::events::{AgentEvent, InterruptReply};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
#[repr(u8)]
pub enum ApprovalMode {
    #[default]
    Standard,
    AcceptEdits,
    Plan,
    FullAccess,
}

impl ApprovalMode {
    fn approves(self, kind: &str, file_edit: bool) -> bool {
        match self {
            Self::FullAccess => matches!(kind, "tool" | "plan"),
            Self::AcceptEdits => kind == "tool" && file_edit,
            Self::Standard | Self::Plan => false,
        }
    }
}

// Only adapter-specific editing requests qualify; commands, unknown tools and
// mixed Nova action batches always require the user's decision.
pub fn file_edit(backend: &str, payload: &serde_json::Value) -> bool {
    let named_edit =
        |name: &serde_json::Value| matches!(name.as_str(), Some("write_file" | "edit_file"));
    match backend {
        "nova" => payload["action_requests"]
            .as_array()
            .is_some_and(|actions| {
                !actions.is_empty() && actions.iter().all(|action| named_edit(&action["name"]))
            }),
        "claude" => matches!(
            payload["tool_name"].as_str(),
            Some("Edit" | "Write" | "MultiEdit" | "NotebookEdit")
        ),
        "codex" => payload["desktopMethod"] == "item/fileChange/requestApproval",
        "pi" => payload["detail"]
            .as_str()
            .and_then(|detail| serde_json::from_str::<serde_json::Value>(detail).ok())
            .is_some_and(|detail| matches!(detail["tool"].as_str(), Some("edit" | "write"))),
        "opencode" | "hermes" | "commandcode" => payload["toolCall"]["kind"] == "edit",
        _ => false,
    }
}

#[derive(Clone)]
struct Pending {
    agent: Arc<dyn AgentBackend>,
    sink: EventSink,
    kind: String,
    file_edit: bool,
    answering: bool,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::{BoxFut, TurnRequest};
    use crate::events::FinishReason;
    use std::sync::atomic::{AtomicBool, AtomicUsize};
    use tokio_util::sync::CancellationToken;

    #[derive(Default)]
    struct Mock {
        replies: AtomicUsize,
        fail_once: AtomicBool,
    }
    impl AgentBackend for Mock {
        fn start<'a>(
            &'a self,
            _: TurnRequest,
            _: EventSink,
            _: CancellationToken,
        ) -> BoxFut<'a, Result<FinishReason, AppError>> {
            Box::pin(async { Ok(FinishReason::Completed) })
        }
        fn respond<'a>(&'a self, _: String, _: InterruptReply) -> BoxFut<'a, Result<(), AppError>> {
            Box::pin(async move {
                if self.fail_once.swap(false, Ordering::SeqCst) {
                    return Err(AppError::Agent("fixture write failed".into()));
                }
                self.replies.fetch_add(1, Ordering::SeqCst);
                Ok(())
            })
        }
    }

    #[tokio::test]
    async fn concurrent_requests_are_bound_to_turns_and_failed_answers_can_retry() {
        let permissions = Permissions::default();
        let mock = Arc::new(Mock::default());
        mock.fail_once.store(true, Ordering::SeqCst);
        let (sink, _) = crate::events::testutil::collect_events();
        permissions.register("a", "same-id", "tool", false, mock.clone(), sink.clone());
        permissions.register("b", "same-id", "tool", false, mock.clone(), sink);
        assert!(permissions
            .respond("wrong", "same-id", InterruptReply::Tool { approve: true })
            .await
            .is_err());
        assert!(permissions
            .respond("a", "same-id", InterruptReply::Tool { approve: true })
            .await
            .is_err());
        permissions
            .respond("a", "same-id", InterruptReply::Tool { approve: false })
            .await
            .unwrap();
        assert_eq!(permissions.pending.lock().unwrap().len(), 1);
        permissions.finish("b");
        assert!(permissions.pending.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn toggle_applies_to_pending_tools_and_plans_but_never_questions() {
        let permissions = Permissions::default();
        let mock = Arc::new(Mock::default());
        let (sink, _) = crate::events::testutil::collect_events();
        for kind in ["tool", "plan", "question"] {
            permissions.register("turn", kind, kind, false, mock.clone(), sink.clone());
        }
        permissions.set_automatic(true).await;
        assert_eq!(mock.replies.load(Ordering::SeqCst), 2);
        assert!(permissions
            .pending
            .lock()
            .unwrap()
            .contains_key(&("turn".into(), "question".into())));
        permissions.set_automatic(false).await;
        permissions.register("turn", "later", "tool", false, mock.clone(), sink);
        assert_eq!(mock.replies.load(Ordering::SeqCst), 2);
        permissions.finish("turn");
    }
    #[test]
    fn edit_classification_is_adapter_specific_and_mixed_batches_require_review() {
        use serde_json::json;
        for (backend, payload) in [
            (
                "nova",
                json!({"action_requests":[{"name":"write_file"},{"name":"edit_file"}]}),
            ),
            ("claude", json!({"tool_name":"Edit"})),
            (
                "codex",
                json!({"desktopMethod":"item/fileChange/requestApproval"}),
            ),
            ("pi", json!({"detail":"{\"tool\":\"write\"}"})),
            ("opencode", json!({"toolCall":{"kind":"edit"}})),
        ] {
            assert!(file_edit(backend, &payload), "{backend}: {payload}");
        }
        for (backend, payload) in [
            (
                "nova",
                json!({"action_requests":[{"name":"write_file"},{"name":"execute"}]}),
            ),
            ("nova", json!({"action_requests":[]})),
            ("claude", json!({"tool_name":"Bash"})),
            ("claude", json!({"tool_name":"mcp__external__Write"})),
            (
                "codex",
                json!({"desktopMethod":"item/commandExecution/requestApproval"}),
            ),
            ("codex", json!({"changes":{"a.txt":{} }})),
            ("pi", json!({"detail":"{\"tool\":\"bash\"}"})),
            (
                "hermes",
                json!({"toolCall":{"title":"edit files","kind":"execute"}}),
            ),
            ("unknown", json!({"toolCall":{"kind":"edit"}})),
        ] {
            assert!(!file_edit(backend, &payload), "{backend}: {payload}");
        }
    }

    #[tokio::test]
    async fn accept_edits_releases_only_edits_and_plan_approves_nothing() {
        let permissions = Permissions::default();
        let mock = Arc::new(Mock::default());
        let (sink, _) = crate::events::testutil::collect_events();
        for (id, kind, edit) in [
            ("edit", "tool", true),
            ("command", "tool", false),
            ("plan", "plan", false),
            ("question", "question", true),
        ] {
            permissions.register("turn", id, kind, edit, mock.clone(), sink.clone());
        }
        permissions.set_mode(ApprovalMode::AcceptEdits).await;
        assert_eq!(mock.replies.load(Ordering::SeqCst), 1);
        assert_eq!(permissions.pending.lock().unwrap().len(), 3);
        permissions.register("turn", "later", "tool", true, mock.clone(), sink);
        permissions.enter_plan();
        permissions.answer_automatic("turn", "later").await;
        assert_eq!(mock.replies.load(Ordering::SeqCst), 1);
        assert_eq!(permissions.mode(), ApprovalMode::Plan);
        assert!(serde_json::from_str::<ApprovalMode>("\"unknown\"").is_err());
    }
}

/// The turn is part of the key: harness request IDs are not globally unique.
#[derive(Clone, Default)]
pub struct Permissions {
    mode: Arc<AtomicU8>,
    pending: Arc<Mutex<HashMap<(String, String), Pending>>>,
}

impl Permissions {
    pub fn register(
        &self,
        turn: &str,
        id: &str,
        kind: &str,
        file_edit: bool,
        agent: Arc<dyn AgentBackend>,
        sink: EventSink,
    ) {
        self.pending
            .lock()
            .expect("permissions mutex")
            .entry((turn.into(), id.into()))
            .or_insert(Pending {
                agent,
                sink,
                kind: kind.into(),
                file_edit,
                answering: false,
            });
    }

    pub fn mode(&self) -> ApprovalMode {
        match self.mode.load(Ordering::SeqCst) {
            1 => ApprovalMode::AcceptEdits,
            2 => ApprovalMode::Plan,
            3 => ApprovalMode::FullAccess,
            _ => ApprovalMode::Standard,
        }
    }

    pub fn auto_answer(&self, turn: &str, id: &str) {
        let permissions = self.clone();
        let (turn, id) = (turn.to_owned(), id.to_owned());
        tauri::async_runtime::spawn(async move {
            permissions.answer_automatic(&turn, &id).await;
        });
    }

    async fn answer_automatic(&self, turn: &str, id: &str) {
        let kind = self
            .pending
            .lock()
            .expect("permissions mutex")
            .get(&(turn.to_owned(), id.to_owned()))
            .filter(|pending| {
                !pending.answering && self.mode().approves(&pending.kind, pending.file_edit)
            })
            .map(|pending| pending.kind.clone());
        if let Some(kind) = kind {
            let reply = if kind == "plan" {
                InterruptReply::Plan {
                    approve: true,
                    feedback: None,
                }
            } else {
                InterruptReply::Tool { approve: true }
            };
            let _ = self.respond(turn, id, reply).await;
        }
    }

    pub async fn respond(
        &self,
        turn: &str,
        id: &str,
        reply: InterruptReply,
    ) -> Result<(), AppError> {
        let key = (turn.to_owned(), id.to_owned());
        let pending = {
            let mut requests = self.pending.lock().expect("permissions mutex");
            let pending = requests.get_mut(&key).ok_or_else(|| {
                AppError::Agent("This decision is no longer pending for this turn".into())
            })?;
            let kind = match &reply {
                InterruptReply::Tool { .. } => "tool",
                InterruptReply::Plan { .. } => "plan",
                InterruptReply::Question { .. } => "question",
            };
            if pending.kind != kind || pending.answering {
                return Err(AppError::Agent(
                    "Decision kind mismatch or response already in progress".into(),
                ));
            }
            pending.answering = true;
            pending.clone()
        };
        match pending.agent.respond(id.to_owned(), reply).await {
            Ok(()) => {
                self.pending.lock().expect("permissions mutex").remove(&key);
                let _ = pending.sink.send(AgentEvent::ApprovalResolved {
                    interrupt_id: id.into(),
                });
                Ok(())
            }
            Err(error) => {
                if let Some(request) = self
                    .pending
                    .lock()
                    .expect("permissions mutex")
                    .get_mut(&key)
                {
                    request.answering = false;
                }
                let _ = pending.sink.send(AgentEvent::Notice {
                    level: "error".into(),
                    message: format!("Decision was not delivered; retry it: {error}"),
                });
                Err(error)
            }
        }
    }

    pub fn enter_plan(&self) {
        self.mode.store(ApprovalMode::Plan as u8, Ordering::SeqCst);
    }

    pub async fn set_mode(&self, mode: ApprovalMode) {
        self.mode.store(mode as u8, Ordering::SeqCst);
        let requests: Vec<_> = self
            .pending
            .lock()
            .expect("permissions mutex")
            .keys()
            .cloned()
            .collect();
        for (turn, id) in requests {
            self.answer_automatic(&turn, &id).await;
        }
    }

    pub async fn set_automatic(&self, enabled: bool) {
        self.set_mode(if enabled {
            ApprovalMode::FullAccess
        } else {
            ApprovalMode::Standard
        })
        .await;
    }

    pub fn finish(&self, turn: &str) {
        let mut requests = self.pending.lock().expect("permissions mutex");
        requests.retain(|(owner, id), pending| {
            if owner != turn {
                return true;
            }
            let _ = pending.sink.send(AgentEvent::ApprovalResolved {
                interrupt_id: id.clone(),
            });
            false
        });
    }
}
