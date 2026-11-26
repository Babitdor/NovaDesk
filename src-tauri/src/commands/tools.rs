//! Commands for the Tools pane and the tool bridge.
//!
//! Both are deliberately thin: the catalogue is a projection of the registry, a
//! reply is handed to the one-shot that is waiting for it, and the switch is the
//! same flag the endpoint reads. Nothing here decides what a tool is.

use serde_json::Value;
use tauri::State;

use crate::error::AppError;
use crate::tools;
use crate::AppState;

/// The catalogue the pane renders: every tool, every harness, and whether this
/// launch is serving them at all.
#[tauri::command]
pub fn tools_catalog(state: State<'_, AppState>) -> Value {
    tools::view(state.tools.enabled(), state.tools.endpoint())
}

/// Answers a tool the webview owes a reply to. The id is one-shot: a duplicate
/// reply finds nothing waiting and is rejected instead of answering a different
/// call.
#[tauri::command]
pub fn tool_reply(id: String, result: Value, state: State<'_, AppState>) -> Result<bool, AppError> {
    let sender = state.tools.take_reply(&id).ok_or_else(|| {
        AppError::Tools("that tool call was already answered or has timed out".into())
    })?;
    sender
        .send(result)
        .map_err(|_| AppError::Tools("the tool call has already timed out".into()))?;
    Ok(true)
}

/// The master switch. Turning it off refuses every call at the endpoint rather
/// than trusting the harness to have been told.
#[tauri::command]
pub fn set_tools_enabled(enabled: bool, state: State<'_, AppState>) {
    state.tools.set_enabled(enabled);
}

/// The queued messages for a task. The same three operations the tools expose,
/// offered to the page as commands so a future queue view needs no protocol
/// round trip.
#[tauri::command]
pub fn list_queue(task_id: String, state: State<'_, AppState>) -> Value {
    crate::tools::queue::list(&state.tools, &task_id)
}

#[tauri::command]
pub fn queue_message(
    task_id: String,
    text: String,
    state: State<'_, AppState>,
) -> Result<Value, AppError> {
    crate::tools::queue::push(&state.tools, &task_id, &text)
}

#[tauri::command]
pub fn remove_queued(id: String, state: State<'_, AppState>) -> Value {
    crate::tools::queue::remove(&state.tools, &id)
}

/// Skills the workspace mounts. Read-only, and the same list the list_skills
/// tool returns, so the pane cannot show a skill a harness cannot see.
#[tauri::command]
pub async fn tools_skills(state: State<'_, AppState>) -> Result<Value, AppError> {
    let workspace = state
        .require_guard()
        .await
        .ok()
        .map(|guard| guard.root().to_path_buf());
    Ok(tools::skills::list(workspace.as_deref()))
}
