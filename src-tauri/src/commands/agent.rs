use tauri::ipc::Channel;
use tauri::{AppHandle, Emitter, Manager, State};
use tokio_util::sync::CancellationToken;

use crate::agent::{cli, drive_turn, Personality, TurnRequest, DEFAULT_BACKEND};
use crate::error::AppError;
use crate::events::{AgentEvent, FinishReason, InterruptReply};
use crate::proc::ProcessRunner;
use crate::AppState;

/// Spawns Nova's pipe session for the current workspace before the first turn,
/// so the user's first message streams instead of waiting on the boot. The
/// frontend calls this only for folders it already trusts; failures are silent
/// because a cold start is a slow first turn, never an error.
#[tauri::command]
pub async fn prewarm_nova(state: State<'_, AppState>) -> Result<(), AppError> {
    let guard = state.require_guard().await?;
    let agent = state.agents.get(DEFAULT_BACKEND)?;
    let root = guard.root().to_path_buf();
    tauri::async_runtime::spawn(async move {
        let _ = agent.prewarm(&root).await;
    });
    Ok(())
}

/// Returns as soon as the turn is queued; the reply streams down `on_event`, so
/// a webview call never sits between the user's click and a cancellation.
///
/// The flat signature is the IPC contract — `invoke('start_turn', { turnId, ... })`
/// maps one argument per field — so grouping them into a struct would only rename
/// the same payload on both sides.
#[allow(clippy::too_many_arguments)]
#[tauri::command]
pub async fn start_turn(
    turn_id: String,
    chat_id: String,
    prompt: String,
    backend: String,
    model: Option<String>,
    effort: Option<String>,
    personality: Option<Personality>,
    session: Option<String>,
    check: Option<Vec<String>>,
    trust_workspace: bool,
    on_event: Channel<AgentEvent>,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<(), AppError> {
    if uuid::Uuid::parse_str(&chat_id).is_err() || uuid::Uuid::parse_str(&turn_id).is_err() {
        return Err(AppError::Agent("Invalid chat or turn ID".into()));
    }
    if prompt.trim().is_empty() || prompt.len() > 1024 * 1024 || prompt.contains('\0') {
        return Err(AppError::Agent(
            "Enter a non-empty prompt of at most 1 MiB".into(),
        ));
    }
    let model = model
        .map(|model| model.trim().to_owned())
        .filter(|model| !model.is_empty());
    if model.as_ref().is_some_and(|model| {
        model.len() > 200 || model.starts_with('-') || model.chars().any(char::is_control)
    }) {
        return Err(AppError::Agent("Invalid model ID".into()));
    }
    if effort
        .as_deref()
        .is_some_and(|level| !crate::agent::models::accepts_effort(&backend, level))
    {
        return Err(AppError::Agent(
            "Unsupported reasoning effort for this agent".into(),
        ));
    }
    let workspace = state.require_guard().await?;
    if let Some(profile) = &personality {
        profile.validate(&backend)?;
        if !trust_workspace {
            return Err(AppError::Agent(
                "Approve the workspace before running a personality".into(),
            ));
        }
    }
    let check = check.unwrap_or_default();
    if check.len() > 100
        || check
            .iter()
            .any(|arg| arg.len() > 4000 || arg.contains('\0'))
        || check
            .first()
            .is_some_and(|program| program.trim().is_empty())
    {
        return Err(AppError::Agent("Invalid project check command".into()));
    }
    let agent = state.agents.get(&backend)?;
    let cancel = CancellationToken::new();
    {
        let mut turns = state.turns.lock().expect("turns mutex poisoned");
        if !turns.is_empty() {
            return Err(AppError::Agent("Wait for the active turn to finish".into()));
        }
        turns.insert(
            turn_id.clone(),
            crate::LiveTurn {
                cancel: cancel.clone(),
                agent: agent.clone(),
                chat_id: chat_id.clone(),
                harness: backend.clone(),
            },
        );
    }
    // Which task this turn belongs to, kept for the queue drain below: the
    // request below takes ownership of the id.
    let queued_for = chat_id.clone();

    // One request file per task, or per personality stage: the path is fixed for
    // the life of a reused harness process, so it must not change per turn.
    let requests_file = app.path().app_data_dir().ok().map(|dir| {
        let key = personality
            .as_ref()
            .map(|profile| format!("personality-{}", profile.role))
            .unwrap_or_else(|| chat_id.clone());
        crate::agent::requests::path_for(&dir, &key)
    });
    let state = state.inner().clone();
    let req = TurnRequest {
        prompt,
        workspace: Some(workspace.clone()),
        chat_id,
        model,
        effort,
        session,
        trust_workspace,
        personality,
        requests_file,
        // Only when this launch is serving them: with the switch off, a turn's
        // argv must be exactly what it was before the tools existed.
        tools: state
            .tools
            .enabled()
            .then(|| crate::tools::bridge(&state.tools))
            .flatten(),
    };

    tauri::async_runtime::spawn(async move {
        let mut reason = drive_turn(agent.as_ref(), req, on_event.clone(), cancel.clone()).await;
        if reason == FinishReason::Completed {
            if let Some((program, args)) = check.split_first() {
                let call_id = format!("check-{turn_id}");
                let _ = on_event.send(AgentEvent::ToolCall {
                    call_id: call_id.clone(),
                    name: "Project check".into(),
                    input: check.join(" "),
                });
                let result =
                    ProcessRunner::run(&workspace, program, args, &on_event, &cancel).await;
                let ok = result.as_ref().is_ok_and(|outcome| outcome.succeeded());
                let output = match result {
                    Ok(outcome) => format!(
                        "Exit status: {}{}",
                        outcome
                            .exit_code
                            .map_or("unknown".into(), |code| code.to_string()),
                        if outcome.truncated {
                            " (output truncated)"
                        } else {
                            ""
                        }
                    ),
                    Err(err) => err.to_string(),
                };
                let _ = on_event.send(AgentEvent::ToolResult {
                    call_id,
                    ok,
                    output,
                });
                if !ok {
                    reason = if cancel.is_cancelled() {
                        FinishReason::Cancelled
                    } else {
                        FinishReason::Failed
                    };
                }
            }
        }
        state
            .turns
            .lock()
            .expect("turns mutex poisoned")
            .remove(&turn_id);
        let _ = on_event.send(AgentEvent::TurnFinished { reason });
        // A message queued while this turn ran is delivered now. The queue is
        // Rust's, but the next turn needs the webview's channel, so the page is
        // told which task has work waiting and it starts the turn itself — the
        // one place that knows what the composer would have sent.
        if let Some(queued) = state.tools.queue_pop(&queued_for) {
            let _ = app.emit_to(
                tauri::EventTarget::webview("main"),
                "nova:queue-ready",
                serde_json::json!({ "chatId": queued_for, "message": queued }),
            );
        }
    });

    Ok(())
}

/// Answers a backend that is blocked on a human decision. Rejections are the
/// fail-closed default: a stale or unknown id is Nova's problem to report, not
/// something the UI should swallow.
#[tauri::command]
pub async fn respond_approval(
    turn_id: String,
    interrupt_id: String,
    reply: InterruptReply,
    state: State<'_, AppState>,
) -> Result<(), AppError> {
    let agent = state
        .turns
        .lock()
        .expect("turns mutex poisoned")
        .get(&turn_id)
        .map(|turn| turn.agent.clone())
        .ok_or_else(|| AppError::Agent("This turn has already finished".into()))?;
    agent.respond(interrupt_id, reply).await
}

#[tauri::command]
pub async fn agent_status(versions: bool, app: AppHandle) -> Vec<cli::AgentStatus> {
    let cwd = app
        .path()
        .app_config_dir()
        .unwrap_or_else(|_| std::env::temp_dir());
    let cwd = if std::fs::create_dir_all(&cwd).is_ok() {
        cwd
    } else {
        std::env::temp_dir()
    };
    cli::statuses(versions, &cwd).await
}

#[tauri::command]
pub async fn cancel_turn(turn_id: String, state: State<'_, AppState>) -> Result<bool, AppError> {
    Ok(state.cancel_turn(&turn_id))
}
