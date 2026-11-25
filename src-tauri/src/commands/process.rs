use tauri::ipc::Channel;
use tauri::State;
use tokio_util::sync::CancellationToken;

use crate::error::AppError;
use crate::events::AgentEvent;
use crate::proc::ProcessRunner;
use crate::AppState;

/// One-off command execution outside a turn. Cancellable through
/// `kill_process`, which is what makes the output panel's Stop button real.
#[tauri::command]
pub async fn run_command(
    command_id: String,
    program: String,
    args: Vec<String>,
    on_event: Channel<AgentEvent>,
    state: State<'_, AppState>,
) -> Result<(), AppError> {
    let guard = state.require_guard().await?;
    let cancel = CancellationToken::new();
    state
        .procs
        .lock()
        .expect("procs mutex poisoned")
        .insert(command_id.clone(), cancel.clone());

    let state = state.inner().clone();
    tauri::async_runtime::spawn(async move {
        let result = ProcessRunner::run(&guard, &program, &args, &on_event, &cancel).await;

        // ToolResult doubles as the terminal frame so the UI can print an exit
        // code without a second event variant.
        let (ok, output) = match result {
            Ok(outcome) => (
                outcome.succeeded(),
                match outcome.exit_code {
                    Some(code) => format!("exit {code}"),
                    None => "terminated".to_string(),
                },
            ),
            Err(AppError::Cancelled) => (false, "cancelled".to_string()),
            Err(err) => (false, err.to_string()),
        };

        let _ = on_event.send(AgentEvent::ToolResult {
            call_id: command_id.clone(),
            ok,
            output,
        });
        state
            .procs
            .lock()
            .expect("procs mutex poisoned")
            .remove(&command_id);
    });

    Ok(())
}

#[tauri::command]
pub async fn kill_process(
    command_id: String,
    state: State<'_, AppState>,
) -> Result<bool, AppError> {
    let token = state
        .procs
        .lock()
        .expect("procs mutex poisoned")
        .get(&command_id)
        .cloned();

    match token {
        Some(token) => {
            token.cancel();
            Ok(true)
        }
        None => Ok(false),
    }
}
