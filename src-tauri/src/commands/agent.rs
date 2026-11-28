use std::path::PathBuf;

use tauri::ipc::Channel;
use tauri::{AppHandle, Manager, State};
use tokio_util::sync::CancellationToken;

use crate::agent::permissions::{file_edit, ApprovalMode};
use crate::agent::{cli, drive_turn, Personality, TurnRequest, DEFAULT_BACKEND};
use crate::error::AppError;
use crate::events::{AgentEvent, FinishReason, InterruptReply};
use crate::proc::ProcessRunner;
use crate::AppState;

fn model_options(
    backend: &str,
    model: Option<String>,
    effort: &Option<String>,
) -> Result<Option<String>, AppError> {
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
        .is_some_and(|level| !crate::agent::models::accepts_effort(backend, level))
    {
        return Err(AppError::Agent(
            "Unsupported reasoning effort for this agent".into(),
        ));
    }
    Ok(model)
}

/// Spawns Nova's pipe session for the current workspace before the first turn,
/// so the user's first message streams instead of waiting on the boot. The
/// frontend calls this only for folders it already trusts; failures are silent
/// because a cold start is a slow first turn, never an error.
#[tauri::command]
pub async fn prewarm_nova(
    chat_id: Option<String>,
    backend: Option<String>,
    model: Option<String>,
    effort: Option<String>,
    session: Option<String>,
    state: State<'_, AppState>,
    app: AppHandle,
) -> Result<(), AppError> {
    let workspace = state.require_guard().await?;
    if state.permissions.mode() == ApprovalMode::Plan {
        return Ok(());
    }
    if !state.turns.lock().expect("turns mutex").is_empty() {
        return Ok(());
    }
    let legacy_prewarm = chat_id.is_none();
    let chat_id = chat_id.unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
    if uuid::Uuid::parse_str(&chat_id).is_err() {
        return Err(AppError::Agent("Invalid chat ID".into()));
    }
    let backend = backend.as_deref().unwrap_or(DEFAULT_BACKEND);
    let model = model_options(backend, model, &effort)?;
    let agent = state.agents.get(backend)?;
    let tools = crate::tools::bridge(&state.tools)
        .filter(|_| state.tools.enabled())
        .map(|bridge| {
            if legacy_prewarm {
                return bridge;
            }
            state.tools.scope_bridge(
                bridge,
                crate::tools::ToolScope {
                    chat_id: chat_id.clone(),
                    turn_id: String::new(),
                    harness: backend.into(),
                    team_id: None,
                    role: None,
                    root: workspace.root().to_path_buf(),
                    active: false,
                    sink: Channel::new(|_| Ok(())),
                },
            )
        });
    let requests_file = app
        .path()
        .app_data_dir()
        .ok()
        .map(|dir| crate::agent::requests::path_for(&dir, &chat_id));
    let req = TurnRequest {
        turn_id: uuid::Uuid::new_v4().to_string(),
        chat_id,
        prompt: String::new(),
        workspace: Some(workspace),
        model,
        effort,
        session,
        trust_workspace: true,
        personality: None,
        requests_file,
        tools,
        read_only: false,
    };
    tauri::async_runtime::spawn(async move {
        let _ = agent.prewarm(req).await;
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
    mut prompt: String,
    backend: String,
    model: Option<String>,
    effort: Option<String>,
    personality: Option<Personality>,
    session: Option<String>,
    check: Option<Vec<String>>,
    trust_workspace: bool,
    read_only: Option<bool>,
    team_id: Option<String>,
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
    let model = model_options(&backend, model, &effort)?;
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
    let read_only = read_only.unwrap_or(false) || state.permissions.mode() == ApprovalMode::Plan;
    if read_only && !check.is_empty() {
        return Err(AppError::Agent(
            "Read-only investigation cannot run a project check command".into(),
        ));
    }
    if read_only && backend != "codex" {
        return Err(AppError::Agent(
            "This harness does not enforce read-only investigation".into(),
        ));
    }
    let agent = state.agents.get(&backend)?;
    let cancel = CancellationToken::new();
    {
        let mut turns = state.turns.lock().expect("turns mutex poisoned");
        let team = team_id
            .as_deref()
            .map(|id| state.teams.get(id))
            .transpose()?;
        if team
            .as_ref()
            .is_some_and(|team| team.root != workspace.root())
        {
            return Err(AppError::Agent(
                "Team workspace changed; start a new run".into(),
            ));
        }
        let active: Vec<_> = turns
            .values()
            .map(|turn| (turn.read_only, turn.team_id.as_deref()))
            .collect();
        if state.permissions.mode() == ApprovalMode::Plan && !read_only {
            return Err(AppError::Agent(
                "Approval mode changed to Plan; start a new read-only Codex turn".into(),
            ));
        }
        if turns.contains_key(&turn_id)
            || !crate::agent::teams::can_schedule(
                &active,
                team_id.as_deref(),
                read_only,
                team.as_ref().map_or(1, |team| team.limit),
            )
        {
            return Err(AppError::Agent(
                "Agent limit or single-writer scheduling prevents this turn from starting".into(),
            ));
        }
        turns.insert(
            turn_id.clone(),
            crate::LiveTurn {
                cancel: cancel.clone(),
                agent: agent.clone(),
                chat_id: chat_id.clone(),
                harness: backend.clone(),
                team_id: team_id.clone(),
                read_only,
                role: personality.as_ref().map(|p| p.role.clone()),
                sink: on_event.clone(),
            },
        );
    }
    // Which task this turn belongs to, kept for the queue drain below: the
    // request below takes ownership of the id.
    let queued_for = chat_id.clone();

    // One request file per task, or per personality stage: the path is fixed for
    // the life of a reused harness process, so it must not change per turn.
    let requests_file = app.path().app_data_dir().ok().map(|dir| {
        let key = chat_id.clone();
        crate::agent::requests::path_for(&dir, &key)
    });
    // Only when this launch is serving them: with the switch off, a turn's argv
    // must be exactly what it was before the tools existed.
    let tools = state
        .tools
        .enabled()
        .then(|| crate::tools::bridge(&state.tools))
        .flatten()
        .filter(|_| !read_only)
        .map(|bridge| {
            state.tools.scope_bridge(
                bridge,
                crate::tools::ToolScope {
                    chat_id: chat_id.clone(),
                    turn_id: turn_id.clone(),
                    harness: backend.clone(),
                    team_id: team_id.clone(),
                    role: personality.as_ref().map(|p| p.role.clone()),
                    root: workspace.root().to_path_buf(),
                    active: true,
                    sink: on_event.clone(),
                },
            )
        });
    // The switch can be on while the shim is missing, which is how a dev build
    // silently hands every harness nothing. The turn still runs; it says why it
    // has no tools rather than letting the transcript look like a refusal.
    if tools.is_none() && state.tools.enabled() && !read_only {
        let _ = on_event.send(AgentEvent::Notice {
            level: "error".into(),
            message: "Nova Desktop tools are unavailable in this build: the nova-tools shim was not found."
                .into(),
        });
    }
    let state = state.inner().clone();
    let destination = on_event.clone();
    let permissions = state.permissions.clone();
    let owner = turn_id.clone();
    let responder = agent.clone();
    let approval_backend = backend.clone();
    let launched = std::time::Instant::now();
    let first_text = std::sync::atomic::AtomicBool::new(true);
    let on_event = Channel::new(move |body| {
        let event: AgentEvent = match body {
            tauri::ipc::InvokeResponseBody::Json(text) => serde_json::from_str(&text),
            tauri::ipc::InvokeResponseBody::Raw(bytes) => serde_json::from_slice(&bytes),
        }
        .map_err(|error| tauri::Error::Anyhow(error.into()))?;
        if matches!(
            &event,
            AgentEvent::MessageDelta { .. }
                | AgentEvent::MessagePreview { .. }
                | AgentEvent::FinalAnswerDelta { .. }
                | AgentEvent::MessageFinal { .. }
        ) && first_text.swap(false, std::sync::atomic::Ordering::SeqCst)
        {
            destination.send(AgentEvent::Timing {
                phase: "firstVisibleText".into(),
                ms: launched.elapsed().as_millis() as u64,
            })?;
        }
        if let AgentEvent::ApprovalRequired {
            interrupt_id,
            kind,
            payload,
        } = &event
        {
            // Render before auto-answering, so failures retain a retryable card.
            permissions.register(
                &owner,
                interrupt_id,
                kind,
                file_edit(&approval_backend, payload),
                responder.clone(),
                destination.clone(),
            );
            destination.send(event.clone())?;
            permissions.auto_answer(&owner, interrupt_id);
            return Ok(());
        }
        destination.send(event)
    });
    prompt.push_str(&format!("\n\nDesktop session: You are running inside Nova Desktop. Current task {chat_id}, turn {turn_id}, harness {backend}. "));
    if read_only {
        prompt.push_str("This is an enforced read-only investigation. Desktop tools and other MCP servers are withheld; return findings and evidence without editing the project.");
    } else if tools.is_some() {
        prompt.push_str("Desktop integration is configured through the nova-desktop MCP server. Discover its advertised tools and use them for requested changes to this app's tasks, files, terminal or embedded browser. Built-in task checklists do not change Desktop tasks. Report success only after a tool confirms the action. ");
        if team_id.is_some() {
            prompt.push_str("This is an active team run. Nova may use team_delegate, team_follow_up, team_message and team_status to assign bounded work, address teammates and inspect outcomes. Delegated assignments execute after this coordinator turn returns. Return promptly after delegation; do not poll for work that cannot start yet. Workers send findings via handoffs or team_message. Only adapters enforcing read-only may investigate in parallel; other work runs with one editing turn at a time.");
        }
    } else {
        prompt.push_str("No Desktop tool bridge is provided for this turn. Do not claim Desktop actions succeeded; direct requests for unavailable Desktop actions to Settings > Tools.");
    }
    let req = TurnRequest {
        turn_id: turn_id.clone(),
        read_only,
        prompt,
        workspace: Some(workspace.clone()),
        chat_id,
        model,
        effort,
        session,
        trust_workspace,
        personality,
        requests_file,
        tools,
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
        state.permissions.finish(&turn_id);
        state.tools.finish_scope(&turn_id);
        state
            .turns
            .lock()
            .expect("turns mutex poisoned")
            .remove(&turn_id);
        let _ = on_event.send(AgentEvent::TurnFinished { reason });
        // The next turn needs the webview's channel. Give the message back to
        // the queue if the composer cannot start it.
        if let Some(queued) = state.tools.queue_pop(&queued_for) {
            if crate::tools::frontend::ask(
                &app,
                &state.tools,
                "deliver_queued_message",
                serde_json::json!({ "chatId": queued_for, "text": queued.text }),
            )
            .await
            .is_err()
            {
                state.tools.queue_restore(&queued_for, queued);
            }
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
    state
        .permissions
        .respond(&turn_id, &interrupt_id, reply)
        .await
}

#[tauri::command]
pub async fn set_auto_approve(enabled: bool, state: State<'_, AppState>) -> Result<(), AppError> {
    state.permissions.set_automatic(enabled).await;
    Ok(())
}

#[tauri::command]
pub async fn set_approval_mode(
    mode: ApprovalMode,
    state: State<'_, AppState>,
) -> Result<(), AppError> {
    if mode == ApprovalMode::Plan {
        let turns = state.turns.lock().expect("turns mutex");
        if !turns.is_empty() {
            return Err(AppError::Agent(
                "Stop the running turn before entering Plan mode".into(),
            ));
        }
        state.permissions.enter_plan();
        return Ok(());
    }
    state.permissions.set_mode(mode).await;
    Ok(())
}

/// Harness probes and updaters run from the config directory rather than the
/// user's workspace: an updater must not see (or write) project files.
fn status_cwd(app: &AppHandle) -> PathBuf {
    let cwd = app
        .path()
        .app_config_dir()
        .unwrap_or_else(|_| std::env::temp_dir());
    if std::fs::create_dir_all(&cwd).is_ok() {
        cwd
    } else {
        std::env::temp_dir()
    }
}

#[tauri::command]
pub async fn agent_status(versions: bool, app: AppHandle) -> Vec<cli::AgentStatus> {
    cli::statuses(versions, &status_cwd(&app)).await
}

#[tauri::command]
pub async fn update_agent(id: String, app: AppHandle) -> Result<cli::AgentUpdate, AppError> {
    cli::update(&id, &status_cwd(&app)).await
}

#[tauri::command]
pub async fn cancel_turn(turn_id: String, state: State<'_, AppState>) -> Result<bool, AppError> {
    Ok(state.cancel_turn(&turn_id))
}

#[tauri::command]
pub async fn begin_team(
    id: String,
    limit: usize,
    state: State<'_, AppState>,
) -> Result<(), AppError> {
    let root = state.require_guard().await?.root().to_path_buf();
    state.teams.begin(id, root, limit)
}

#[tauri::command]
pub fn finish_team(id: String, state: State<'_, AppState>) {
    state.teams.finish(&id);
    let turns: Vec<_> = state
        .turns
        .lock()
        .expect("turns mutex")
        .iter()
        .filter(|(_, turn)| turn.team_id.as_deref() == Some(&id))
        .map(|(id, _)| id.clone())
        .collect();
    for id in turns {
        state.cancel_turn(&id);
    }
}

#[tauri::command]
pub fn agent_capabilities(state: State<'_, AppState>) -> serde_json::Value {
    let values: serde_json::Map<String, serde_json::Value> = cli::AGENTS
        .iter()
        .filter_map(|(id, _)| {
            state
                .agents
                .get(id)
                .ok()
                .map(|agent| ((*id).into(), serde_json::json!(agent.capabilities())))
        })
        .collect();
    serde_json::json!(values)
}
