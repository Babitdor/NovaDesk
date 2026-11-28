use std::path::Path;
use std::time::UNIX_EPOCH;

use tauri::{AppHandle, Manager, State};

use crate::error::AppError;
use crate::workspace::WorkspaceGuard;
use crate::AppState;

/// Bounded so a stray multi-megabyte file cannot stall the IPC channel. Two
/// MiB is where a minified bundle or a long log still opens; past it the file is
/// a mistake to paste into a text editor, and the pane says so by name.
const MAX_READ_BYTES: u64 = 2 * 1024 * 1024;

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

/// The composer's paperclip: paths, not contents. Every harness reads a file it
/// is handed, so this only has to name it — in the workspace's own terms when
/// there is one, in the host's when there is not.
#[tauri::command]
pub async fn pick_attachments(
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<Vec<String>, AppError> {
    let guard = state.guard.read().await.clone();
    let (tx, rx) = tokio::sync::oneshot::channel();
    crate::commands::dialogs::pick_files(&app, move |picked| {
        let _ = tx.send(picked);
    });

    let picked = rx
        .await
        .map_err(|_| AppError::Agent("file picker closed unexpectedly".to_string()))?;
    // A dismissed dialog is an empty list, not an error: nothing to attach is
    // the most ordinary outcome there is.
    Ok(picked
        .unwrap_or_default()
        .iter()
        .map(|path| crate::workspace::attachment_label(guard.as_ref(), path))
        .collect())
}

#[tauri::command]
pub async fn pick_attachment_folder(
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<Vec<String>, AppError> {
    let guard = state.guard.read().await.clone();
    let (tx, rx) = tokio::sync::oneshot::channel();
    crate::commands::dialogs::pick_folder(&app, move |picked| {
        let _ = tx.send(picked);
    });
    let picked = rx
        .await
        .map_err(|_| AppError::Agent("folder picker closed unexpectedly".into()))?;
    Ok(picked
        .into_iter()
        .map(|path| crate::workspace::attachment_label(guard.as_ref(), &path))
        .collect())
}

#[tauri::command]
pub async fn save_sketch(
    root: String,
    data_url: String,
    state: State<'_, AppState>,
) -> Result<String, AppError> {
    let guard = state.require_guard().await?;
    if guard.root().to_string_lossy() != root {
        return Err(AppError::Io(
            "Workspace changed; return to the original folder before attaching this sketch.".into(),
        ));
    }
    save_sketch_png(&guard, &data_url)
}

fn save_sketch_png(guard: &WorkspaceGuard, data_url: &str) -> Result<String, AppError> {
    use base64::Engine;
    use std::io::Write;
    let invalid =
        || AppError::Io("Sketch must be a PNG up to 2 MiB and 4096 × 4096 pixels.".into());
    if data_url.len() > 3 * 1024 * 1024 {
        return Err(invalid());
    }
    let encoded = data_url
        .strip_prefix("data:image/png;base64,")
        .ok_or_else(invalid)?;
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(encoded)
        .map_err(|_| invalid())?;
    if bytes.len() < 57
        || bytes.len() > 2 * 1024 * 1024
        || !bytes.starts_with(b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR")
        || !bytes.ends_with(b"\0\0\0\0IEND\xaeB`\x82")
    {
        return Err(invalid());
    }
    let width = u32::from_be_bytes(bytes[16..20].try_into().map_err(|_| invalid())?);
    let height = u32::from_be_bytes(bytes[20..24].try_into().map_err(|_| invalid())?);
    if !(1..=4096).contains(&width) || !(1..=4096).contains(&height) {
        return Err(invalid());
    }
    // Resolve after each mkdir too: an existing .nova symlink must not redirect a write.
    for relative in [".nova", ".nova/attachments"] {
        let target = guard.resolve_for_write(relative)?;
        if let Err(error) = std::fs::create_dir(&target) {
            if error.kind() != std::io::ErrorKind::AlreadyExists {
                return Err(AppError::from_io(error, &target));
            }
        }
        let folder = guard.resolve(relative)?;
        if !folder.is_dir() {
            return Err(AppError::NotADirectory(relative.into()));
        }
    }
    let relative = format!(".nova/attachments/sketch-{}.png", uuid::Uuid::new_v4());
    let destination = guard.resolve_for_write(&relative)?;
    let mut temporary = tempfile::NamedTempFile::new_in(guard.resolve(".nova/attachments")?)
        .map_err(|error| AppError::from_io(error, &destination))?;
    temporary
        .write_all(&bytes)
        .map_err(|error| AppError::from_io(error, &destination))?;
    temporary
        .as_file()
        .sync_all()
        .map_err(|error| AppError::from_io(error, &destination))?;
    temporary
        .persist_noclobber(&destination)
        .map_err(|error| AppError::from_io(error.error, &destination))?;
    Ok(relative)
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

/// The walk behind the pane's Go-to-file box. Bounded three ways — depth, entries
/// visited, results returned — because it runs on the command thread and a repo
/// with a stray `node_modules` is the normal case, not the exception.
const SEARCH_MAX_RESULTS: usize = 100;
const SEARCH_MAX_ENTRIES: usize = 20_000;
const SEARCH_MAX_DEPTH: usize = 12;

/// Folders no one opens from a picker and everyone pays for in a walk: build
/// output, dependency trees, and this app's own scratch directories.
const SEARCH_SKIP: [&str; 12] = [
    ".git",
    ".nuxt",
    ".playwright-mcp",
    ".temp",
    ".venv",
    ".next",
    "build",
    "dist",
    "node_modules",
    "target",
    "venv",
    "__pycache__",
];

/// Files whose name contains `query`, case-insensitively, for Ctrl+P. An empty
/// query answers nothing rather than the whole tree: the box only means something
/// once it has narrowed it.
#[tauri::command]
pub async fn search_files(
    query: String,
    state: State<'_, AppState>,
) -> Result<Vec<DirEntry>, AppError> {
    let guard = state.require_guard().await?;
    let needle = query.trim().to_lowercase();
    if needle.is_empty() {
        return Ok(Vec::new());
    }
    Ok(search(&guard, &needle))
}

/// A stack walk, not recursion: depth is a guard, and a symlink loop is a real
/// possibility in a workspace. Paths come from the walk itself, never from the
/// caller, so this cannot address anything outside the root.
fn search(guard: &WorkspaceGuard, needle: &str) -> Vec<DirEntry> {
    let mut found: Vec<DirEntry> = Vec::new();
    let mut stack = vec![(guard.root().to_path_buf(), 0_usize)];
    let mut visited = 0_usize;
    while let Some((dir, depth)) = stack.pop() {
        if depth > SEARCH_MAX_DEPTH {
            continue;
        }
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            visited += 1;
            if visited > SEARCH_MAX_ENTRIES || found.len() >= SEARCH_MAX_RESULTS {
                found.sort_by(|a, b| a.path.cmp(&b.path));
                return found;
            }
            let name = entry.file_name().to_string_lossy().into_owned();
            let is_dir = entry.file_type().map(|t| t.is_dir()).unwrap_or(false);
            if is_dir {
                if !SEARCH_SKIP
                    .iter()
                    .any(|skip| skip.eq_ignore_ascii_case(&name))
                {
                    stack.push((entry.path(), depth + 1));
                }
            } else if name.to_lowercase().contains(needle) {
                found.push(DirEntry {
                    path: guard.display_relative(&entry.path()),
                    name,
                    is_dir,
                });
            }
        }
    }
    // read_dir order is the filesystem's, so sort for a stable picker order.
    found.sort_by(|a, b| a.path.cmp(&b.path));
    found
}

#[cfg(test)]
mod tests {
    use super::{base64, save_sketch_png, save_text, search, MAX_READ_BYTES};
    use crate::{error::AppError, testtmp::TempDir, workspace::WorkspaceGuard};

    #[test]
    fn sketch_saves_unique_pngs_and_refuses_invalid_content() {
        let tmp = crate::testtmp::TempDir::new("sketch");
        let guard = WorkspaceGuard::new(tmp.path()).unwrap();
        let png = "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+aK9sAAAAASUVORK5CYII=";
        let first = save_sketch_png(&guard, png).unwrap();
        let second = save_sketch_png(&guard, png).unwrap();
        assert_ne!(first, second);
        assert!(guard.resolve(&first).unwrap().is_file());
        for invalid in [
            "data:text/html;base64,PHNjcmlwdD4=",
            "data:image/png;base64,!!!",
            "data:image/png;base64,aGVsbG8=",
        ] {
            assert!(save_sketch_png(&guard, invalid).is_err());
        }
        use base64::Engine;
        let mut bytes = std::fs::read(guard.resolve(&first).unwrap()).unwrap();
        bytes[16..20].copy_from_slice(&5000u32.to_be_bytes());
        assert!(save_sketch_png(
            &guard,
            &format!(
                "data:image/png;base64,{}",
                base64::engine::general_purpose::STANDARD.encode(bytes)
            )
        )
        .is_err());
    }

    #[test]
    fn sketch_does_not_write_through_an_attachment_directory_file() {
        let tmp = crate::testtmp::TempDir::new("sketch-parent");
        std::fs::write(tmp.path().join(".nova"), "keep").unwrap();
        let guard = WorkspaceGuard::new(tmp.path()).unwrap();
        let png = "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+aK9sAAAAASUVORK5CYII=";
        assert!(save_sketch_png(&guard, png).is_err());
        assert_eq!(
            std::fs::read_to_string(tmp.path().join(".nova")).unwrap(),
            "keep"
        );
    }

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
        assert!(save_text(
            &guard,
            "file.ts",
            "",
            &"x".repeat(MAX_READ_BYTES as usize + 1)
        )
        .is_err());
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

    #[test]
    fn go_to_file_walks_the_workspace_but_not_build_or_dependency_folders() {
        let tmp = TempDir::new("search-files");
        for dir in ["src/deep", "node_modules/pkg", ".git/objects"] {
            std::fs::create_dir_all(tmp.path().join(dir)).unwrap();
        }
        for file in [
            "src/app.ts",
            "src/deep/Helper.TSX",
            "README.md",
            "node_modules/pkg/hidden.ts",
            ".git/objects/blob.ts",
        ] {
            std::fs::write(tmp.path().join(file), "x").unwrap();
        }
        let guard = WorkspaceGuard::new(tmp.path()).unwrap();
        let found: Vec<String> = search(&guard, "ts").into_iter().map(|e| e.path).collect();
        assert_eq!(found, ["src/app.ts", "src/deep/Helper.TSX"]);
        assert_eq!(search(&guard, "readme")[0].path, "README.md");
    }
}
