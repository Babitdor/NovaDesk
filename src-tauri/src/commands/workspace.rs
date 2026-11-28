use std::path::Path;
use std::time::UNIX_EPOCH;

use tauri::{AppHandle, Manager, State};

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
    crate::commands::dialogs::pick_folder(&app, move |picked| {
        let _ = tx.send(picked);
    });

    // The helper normalises the plugin's path type away, so a dismissed dialog is
    // simply None and the app never has to name FilePath.
    let Some(path) = rx
        .await
        .map_err(|_| AppError::Agent("folder picker closed unexpectedly".to_string()))?
    else {
        return Ok(None);
    };
    set_root(WorkspaceGuard::new(path)?, &state, &app)
        .await
        .map(Some)
}

#[tauri::command]
pub async fn set_workspace(
    path: String,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<WorkspaceInfo, AppError> {
    set_root(WorkspaceGuard::new(path)?, &state, &app).await
}

async fn set_root(
    guard: WorkspaceGuard,
    state: &State<'_, AppState>,
    app: &AppHandle,
) -> Result<WorkspaceInfo, AppError> {
    let info = info(&guard);
    // The project list is what the list_projects tool answers from, so it is
    // written where a tool can find it; a failure here must not stop a folder
    // from opening, because losing the history is the smaller loss.
    if let Ok(data) = app.path().app_data_dir() {
        let _ = crate::tools::projects::register(&data, guard.root());
    }
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
    String::from_utf8(bytes).map_err(|_| {
        AppError::Io("This file is not UTF-8 text. Binary files cannot be edited.".into())
    })
}

/// Saves only the exact UTF-8 file the editor read, within the current workspace.
#[tauri::command]
pub async fn save_file(
    path: String,
    root: String,
    expected: String,
    content: String,
    state: State<'_, AppState>,
) -> Result<(), AppError> {
    let guard = state.require_guard().await?;
    if guard.root().to_string_lossy() != root {
        return Err(AppError::Io(
            "Workspace changed; reopen the file before saving.".into(),
        ));
    }
    save_text(&guard, &path, &expected, &content)
}

fn save_text(
    guard: &WorkspaceGuard,
    path: &str,
    expected: &str,
    content: &str,
) -> Result<(), AppError> {
    use std::io::Write;
    // Serialize saves from this app; external edits are checked again before replacement.
    static SAVES: std::sync::Mutex<()> = std::sync::Mutex::new(());
    let _lock = SAVES
        .lock()
        .map_err(|error| AppError::Io(error.to_string()))?;
    if content.len() as u64 > MAX_READ_BYTES || expected.len() as u64 > MAX_READ_BYTES {
        return Err(AppError::TooLarge(path.into()));
    }
    let file = guard.resolve(path)?;
    let meta = std::fs::metadata(&file).map_err(|error| AppError::from_io(error, &file))?;
    if !meta.is_file() {
        return Err(AppError::NotAFile(path.into()));
    }
    if meta.len() > MAX_READ_BYTES {
        return Err(AppError::TooLarge(path.into()));
    }
    if meta.permissions().readonly() {
        return Err(AppError::Io("File is read-only.".into()));
    }
    let check = || -> Result<(), AppError> {
        let bytes = std::fs::read(&file).map_err(|error| AppError::from_io(error, &file))?;
        if bytes != expected.as_bytes() || bytes.contains(&0) {
            return Err(AppError::FileConflict(path.into()));
        }
        Ok(())
    };
    check()?;
    let mut temporary = tempfile::NamedTempFile::new_in(
        file.parent()
            .ok_or_else(|| AppError::PathEscape(path.into()))?,
    )
    .map_err(|error| AppError::from_io(error, &file))?;
    temporary
        .write_all(content.as_bytes())
        .map_err(|error| AppError::from_io(error, &file))?;
    temporary
        .as_file()
        .set_permissions(meta.permissions())
        .map_err(|error| AppError::from_io(error, &file))?;
    temporary
        .as_file()
        .sync_all()
        .map_err(|error| AppError::from_io(error, &file))?;
    check()?;
    temporary
        .persist(&file)
        .map_err(|error| AppError::from_io(error.error, &file))?;
    Ok(())
}

/// Images leave as data URLs: the app's CSP already allows `img-src data:`, so a
/// picture needs no loopback request, no new crate and no capability. The cap
/// is where a file stops being a picture and starts being an IPC stall —
/// base64 turns 12 MiB into ~16 MB of string.
const MAX_IMAGE_BYTES: u64 = 12 * 1024 * 1024;

/// What the page can paint as an image, by extension. SVG is included because a
/// data URL is an image context: a script inside the file never runs.
/// Shared with the preview server, which serves the same kinds over HTTP, so
/// the extension table exists once.
pub(crate) fn mime_for(path: &Path) -> Option<&'static str> {
    let ext = path.extension()?.to_str()?.to_ascii_lowercase();
    Some(match ext.as_str() {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "avif" => "image/avif",
        "bmp" => "image/bmp",
        "ico" => "image/x-icon",
        "svg" => "image/svg+xml",
        _ => return None,
    })
}

/// Hand-rolled for the same reason the HTTP server is: base64 is a table and
/// three lines of arithmetic, and this repository does not add a dependency for
/// arithmetic.
fn base64(bytes: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let group = [
            chunk[0],
            chunk.get(1).copied().unwrap_or(0),
            chunk.get(2).copied().unwrap_or(0),
        ];
        let word = (u32::from(group[0]) << 16) | (u32::from(group[1]) << 8) | u32::from(group[2]);
        out.push(TABLE[(word >> 18) as usize & 63] as char);
        out.push(TABLE[(word >> 12) as usize & 63] as char);
        out.push(if chunk.len() > 1 {
            TABLE[(word >> 6) as usize & 63] as char
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            TABLE[word as usize & 63] as char
        } else {
            '='
        });
    }
    out
}

/// `read_file` without the UTF-8 assumption, for the preview's image kinds.
#[tauri::command]
pub async fn read_file_data_url(
    path: String,
    state: State<'_, AppState>,
) -> Result<String, AppError> {
    let guard = state.require_guard().await?;
    let file = guard.resolve(&path)?;

    let meta = std::fs::metadata(&file).map_err(|e| AppError::from_io(e, &file))?;
    if !meta.is_file() {
        return Err(AppError::NotAFile(path));
    }
    let Some(mime) = mime_for(&file) else {
        return Err(AppError::NotAFile(format!("{path}: not an image")));
    };
    if meta.len() > MAX_IMAGE_BYTES {
        return Err(AppError::TooLarge(format!(
            "{path} is {} bytes (limit {MAX_IMAGE_BYTES})",
            meta.len()
        )));
    }

    let bytes = std::fs::read(&file).map_err(|e| AppError::from_io(e, &file))?;
    Ok(format!("data:{mime};base64,{}", base64(&bytes)))
}

/// Length and mtime only — the preview panel polls this once a second to notice
/// an edit, and shipping the file itself across the IPC channel every second
/// would cost more than the whole panel does now.
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FileStamp {
    pub len: u64,
    /// Milliseconds since the epoch; 0 when the platform will not say.
    pub modified: u64,
}

#[tauri::command]
pub async fn file_stamp(path: String, state: State<'_, AppState>) -> Result<FileStamp, AppError> {
    let guard = state.require_guard().await?;
    let file = guard.resolve(&path)?;

    let meta = std::fs::metadata(&file).map_err(|e| AppError::from_io(e, &file))?;
    let modified = meta
        .modified()
        .ok()
        .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
        .map(|age| age.as_millis() as u64)
        .unwrap_or(0);
    Ok(FileStamp {
        len: meta.len(),
        modified,
    })
}

#[cfg(test)]
mod tests {
    use super::{base64, save_text};
    use crate::{error::AppError, testtmp::TempDir, workspace::WorkspaceGuard};

    #[test]
    fn editor_save_preserves_bytes_and_refuses_stale_or_binary_content() {
        let tmp = TempDir::new("editor-save");
        let path = tmp.path().join("file.ts");
        let original = "\u{feff}const café = 1;\r\n";
        std::fs::write(&path, original).unwrap();
        let guard = WorkspaceGuard::new(tmp.path()).unwrap();
        save_text(&guard, "file.ts", original, "\u{feff}const café = 2;\r\n").unwrap();
        let saved = std::fs::read(&path).unwrap();
        assert_eq!(saved, "\u{feff}const café = 2;\r\n".as_bytes());
        assert!(matches!(
            save_text(&guard, "file.ts", original, "stale"),
            Err(AppError::FileConflict(_))
        ));
        assert_eq!(std::fs::read(&path).unwrap(), saved);
        std::fs::write(&path, [0xff, 0, 0x41]).unwrap();
        assert!(save_text(&guard, "file.ts", "�\0A", "oops").is_err());
        assert_eq!(std::fs::read(&path).unwrap(), [0xff, 0, 0x41]);
        assert!(save_text(&guard, "file.ts", "", &"x".repeat(256 * 1024 + 1)).is_err());
    }

    #[test]
    fn editor_save_rejects_workspace_escape() {
        let tmp = TempDir::new("editor-escape");
        std::fs::create_dir(tmp.path().join("inner")).unwrap();
        std::fs::write(tmp.path().join("secret"), "keep").unwrap();
        let guard = WorkspaceGuard::new(tmp.path().join("inner")).unwrap();
        assert!(matches!(
            save_text(&guard, "../secret", "keep", "oops"),
            Err(AppError::PathEscape(_))
        ));
        assert_eq!(std::fs::read(tmp.path().join("secret")).unwrap(), b"keep");
    }

    #[test]
    fn base64_matches_the_rfc_4648_vectors() {
        // The three published vectors, which cover every padding width.
        assert_eq!(base64(b""), "");
        assert_eq!(base64(b"f"), "Zg==");
        assert_eq!(base64(b"fo"), "Zm8=");
        assert_eq!(base64(b"foo"), "Zm9v");
        assert_eq!(base64(b"foob"), "Zm9vYg==");
        assert_eq!(base64(b"fooba"), "Zm9vYmE=");
        assert_eq!(base64(b"foobar"), "Zm9vYmFy");
    }

    #[test]
    fn base64_encodes_binary_bytes_it_cannot_print() {
        // Every byte value once: the alphabet and the shifts, not the ASCII range.
        let all: Vec<u8> = (0..=255_u8).collect();
        assert_eq!(
            base64(&all),
            "AAECAwQFBgcICQoLDA0ODxAREhMUFRYXGBkaGxwdHh8gISIjJCUmJygpKissLS4vMDEyMzQ1Njc4\
             OTo7PD0+P0BBQkNERUZHSElKS0xNTk9QUVJTVFVWV1hZWltcXV5fYGFiY2RlZmdoaWprbG1ub3Bx\
             cnN0dXZ3eHl6e3x9fn+AgYKDhIWGh4iJiouMjY6PkJGSk5SVlpeYmZqbnJ2en6ChoqOkpaanqKmq\
             q6ytrq+wsbKztLW2t7i5uru8vb6/wMHCw8TFxsfIycrLzM3Oz9DR0tPU1dbX2Nna29zd3t/g4eLj\
             5OXm5+jp6uvs7e7v8PHy8/T19vf4+fr7/P3+/w=="
        );
    }
}
