use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use super::{AgentBackend, EventSink};
use crate::error::AppError;
use crate::events::{AgentEvent, InterruptReply};

#[derive(Clone)]
struct Pending {
    agent: Arc<dyn AgentBackend>,
    sink: EventSink,
    kind: String,
    answering: bool,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::{BoxFut, TurnRequest};
    use crate::events::FinishReason;
    use std::sync::atomic::AtomicUsize;
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
        permissions.register("a", "same-id", "tool", mock.clone(), sink.clone());
        permissions.register("b", "same-id", "tool", mock.clone(), sink);
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
            permissions.register("turn", kind, kind, mock.clone(), sink.clone());
        }
        permissions.set_automatic(true).await;
        assert_eq!(mock.replies.load(Ordering::SeqCst), 2);
        assert!(permissions
            .pending
            .lock()
            .unwrap()
            .contains_key(&("turn".into(), "question".into())));
        permissions.set_automatic(false).await;
        permissions.register("turn", "later", "tool", mock.clone(), sink);
        assert_eq!(mock.replies.load(Ordering::SeqCst), 2);
        permissions.finish("turn");
    }
}

/// The turn is part of the key: harness request IDs are not globally unique.
#[derive(Clone, Default)]
pub struct Permissions {
    automatic: Arc<AtomicBool>,
    pending: Arc<Mutex<HashMap<(String, String), Pending>>>,
}

impl Permissions {
    pub fn register(
        &self,
        turn: &str,
        id: &str,
        kind: &str,
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
                answering: false,
            });
    }

    pub fn auto_answer(&self, turn: &str, id: &str, kind: &str) {
        if self.automatic.load(Ordering::SeqCst) && matches!(kind, "tool" | "plan") {
            let permissions = self.clone();
            let (turn, id, kind) = (turn.to_owned(), id.to_owned(), kind.to_owned());
            tauri::async_runtime::spawn(async move {
                if !permissions.automatic.load(Ordering::SeqCst) {
                    return;
                }
                let reply = if kind == "plan" {
                    InterruptReply::Plan {
                        approve: true,
                        feedback: None,
                    }
                } else {
                    InterruptReply::Tool { approve: true }
                };
                let _ = permissions.respond(&turn, &id, reply).await;
            });
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

    pub async fn set_automatic(&self, enabled: bool) {
        self.automatic.store(enabled, Ordering::SeqCst);
        if !enabled {
            return;
        }
        let requests: Vec<_> = self
            .pending
            .lock()
            .expect("permissions mutex")
            .iter()
            .filter(|(_, p)| matches!(p.kind.as_str(), "tool" | "plan") && !p.answering)
            .map(|((turn, id), p)| (turn.clone(), id.clone(), p.kind.clone()))
            .collect();
        for (turn, id, kind) in requests {
            if !self.automatic.load(Ordering::SeqCst) {
                break;
            }
            let reply = if kind == "plan" {
                InterruptReply::Plan {
                    approve: true,
                    feedback: None,
                }
            } else {
                InterruptReply::Tool { approve: true }
            };
            let _ = self.respond(&turn, &id, reply).await;
        }
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
