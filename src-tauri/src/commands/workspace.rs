use tauri::{AppHandle, State};
use tauri_plugin_dialog::DialogExt;

use crate::error::AppError;
use crate::workspace::WorkspaceGuard;
use crate::AppState;

/// Bounded so a stray multi-megabyte file cannot stall the IPC channel.
const MAX_READ_BYTES: u64 = 256 * 1024;

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DirEntry {
    /// Workspace-relative and forward-slashed: the UI never sees host paths.
    pub path: String,
    pub name: String,
    pub is_dir: bool,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceInfo {
    pub root: String,
    pub name: String,
}

fn info(guard: &WorkspaceGuard) -> WorkspaceInfo {
    WorkspaceInfo {
        root: guard.root().display().to_string(),
        name: guard
            .root()
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| guard.root().display().to_string()),
    }
}

/// Opens the native folder picker from Rust, which is why the frontend needs no
/// `dialog:*` capability and no dialog npm package.
#[tauri::command]
pub async fn pick_workspace(
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<Option<WorkspaceInfo>, AppError> {
    let (tx, rx) = tokio::sync::oneshot::channel();
    app.dialog().file().pick_folder(move |picked| {
        let _ = tx.send(picked);
    });

    let Some(picked) = rx
        .await
        .map_err(|_| AppError::Agent("folder picker closed unexpectedly".to_string()))?
    else {
        return Ok(None);
    };

    let path = picked
        .into_path()
        .map_err(|e| AppError::Agent(format!("unsupported folder selection: {e}")))?;
    set_root(WorkspaceGuard::new(path)?, &state).await.map(Some)
}

#[tauri::command]
pub async fn set_workspace(
    path: String,
    state: State<'_, AppState>,
) -> Result<WorkspaceInfo, AppError> {
    set_root(WorkspaceGuard::new(path)?, &state).await
}

async fn set_root(
    guard: WorkspaceGuard,
    state: &State<'_, AppState>,
) -> Result<WorkspaceInfo, AppError> {
    let info = info(&guard);
    *state.guard.write().await = Some(guard);
    Ok(info)
}

#[tauri::command]
pub async fn list_dir(path: String, state: State<'_, AppState>) -> Result<Vec<DirEntry>, AppError> {
    let guard = state.require_guard().await?;
    let dir = guard.resolve(&path)?;

    let mut entries = Vec::new();
    for entry in std::fs::read_dir(&dir).map_err(|e| AppError::from_io(e, &dir))? {
        let entry = entry.map_err(|e| AppError::Io(e.to_string()))?;
        let is_dir = entry.file_type().map(|t| t.is_dir()).unwrap_or(false);
        entries.push(DirEntry {
            path: guard.display_relative(&entry.path()),
            name: entry.file_name().to_string_lossy().into_owned(),
            is_dir,
        });
    }

    // Directories first, then case-insensitive by name — matches every file tree
    // the user has ever seen.
    entries.sort_by(|a, b| {
        b.is_dir
            .cmp(&a.is_dir)
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });
    Ok(entries)
}

#[tauri::command]
pub async fn read_file(path: String, state: State<'_, AppState>) -> Result<String, AppError> {
    let guard = state.require_guard().await?;
    let file = guard.resolve(&path)?;

    let meta = std::fs::metadata(&file).map_err(|e| AppError::from_io(e, &file))?;
    if !meta.is_file() {
        return Err(AppError::NotAFile(path));
    }
    if meta.len() > MAX_READ_BYTES {
        return Err(AppError::TooLarge(format!(
            "{path} is {} bytes (limit {MAX_READ_BYTES})",
            meta.len()
        )));
    }

    let bytes = std::fs::read(&file).map_err(|e| AppError::from_io(e, &file))?;
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}
