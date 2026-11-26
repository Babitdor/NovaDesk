//! Moodboards: a folder of images and captions inside the workspace.
//!
//! Read-only for now. The pane that renders a board is a UI action (adding
//! images needs a picker and a drop target), so a tool can look at what exists
//! but cannot create one. Saying that plainly beats half a feature: an empty
//! list means "no board here yet", not "the tool failed".
//!
//! Layout, one directory per board:
//!
//!   <workspace>/.nova/moodboards/<id>/board.json   { name, captions }
//!   <workspace>/.nova/moodboards/<id>/*.png        the images
//!
//! Everything in board.json is authored by the user, so it is data, not
//! instructions: the tool returns it verbatim and says so in its description.

use std::path::{Path, PathBuf};

use serde_json::{json, Value};

use crate::error::AppError;

/// A board larger than this is not a reference image, and base64 makes it
/// bigger still.
const MAX_IMAGE_BYTES: u64 = 4 * 1024 * 1024;

const IMAGE_EXTENSIONS: &[&str] = &["png", "jpg", "jpeg", "webp", "gif", "avif"];

fn boards(workspace: &Path) -> PathBuf {
    workspace.join(".nova").join("moodboards")
}

/// One path component, and nothing that could climb out of the board folder.
fn component(value: &str, what: &str) -> Result<String, AppError> {
    let trimmed = value.trim();
    if trimmed.is_empty()
        || trimmed.len() > 128
        || trimmed == "."
        || trimmed == ".."
        || trimmed.contains('/')
        || trimmed.contains('\\')
        || trimmed.contains(':')
        || trimmed.chars().any(char::is_control)
    {
        return Err(AppError::Tools(format!("{what} is not a board name")));
    }
    Ok(trimmed.to_string())
}

fn is_image(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .map(|extension| extension.to_ascii_lowercase())
        .is_some_and(|extension| IMAGE_EXTENSIONS.contains(&extension.as_str()))
}

fn mime(path: &Path) -> &'static str {
    match path
        .extension()
        .and_then(|extension| extension.to_str())
        .map(|extension| extension.to_ascii_lowercase())
        .as_deref()
    {
        Some("png") => "image/png",
        Some("jpg" | "jpeg") => "image/jpeg",
        Some("webp") => "image/webp",
        Some("gif") => "image/gif",
        Some("avif") => "image/avif",
        _ => "application/octet-stream",
    }
}

fn image_names(board: &Path) -> Vec<(String, u64)> {
    let mut images = Vec::new();
    let Ok(entries) = std::fs::read_dir(board) else {
        return images;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_file() || !is_image(&path) {
            continue;
        }
        let size = entry.metadata().map(|meta| meta.len()).unwrap_or_default();
        images.push((entry.file_name().to_string_lossy().into_owned(), size));
    }
    images.sort_by(|left, right| left.0.cmp(&right.0));
    images
}

fn board_json(board: &Path) -> Value {
    std::fs::read(board.join("board.json"))
        .ok()
        .and_then(|raw| serde_json::from_slice::<Value>(&raw).ok())
        .filter(Value::is_object)
        .unwrap_or_else(|| json!({}))
}

pub fn list(workspace: Option<&Path>) -> Value {
    let Some(workspace) = workspace else {
        return json!({
            "moodboards": [],
            "note": "no workspace is open; moodboards live inside a project",
        });
    };
    let mut found: Vec<Value> = Vec::new();
    if let Ok(entries) = std::fs::read_dir(boards(workspace)) {
        for entry in entries.flatten() {
            if !entry.path().is_dir() {
                continue;
            }
            let id = entry.file_name().to_string_lossy().into_owned();
            let board = entry.path();
            let meta = board_json(&board);
            let images = image_names(&board);
            found.push(json!({
                "id": id,
                "name": meta.get("name").and_then(Value::as_str).unwrap_or(&id),
                "images": images.len(),
                "hasBoardJson": board.join("board.json").is_file(),
            }));
        }
    }
    found.sort_by(|left, right| left["name"].as_str().cmp(&right["name"].as_str()));
    json!({ "moodboards": found, "root": ".nova/moodboards" })
}

pub fn read(workspace: Option<&Path>, id: &str) -> Result<Value, AppError> {
    let workspace = workspace.ok_or(AppError::NoWorkspace)?;
    let id = component(id, "the board id")?;
    let board = boards(workspace).join(&id);
    if !board.is_dir() {
        return Err(AppError::NotFound(format!("moodboard {id}")));
    }
    Ok(json!({
        "id": id,
        "board": board_json(&board),
        "images": image_names(&board)
            .into_iter()
            .map(|(name, bytes)| json!({ "file": name, "bytes": bytes }))
            .collect::<Vec<_>>(),
    }))
}

pub fn image(workspace: Option<&Path>, id: &str, image: &str) -> Result<Value, AppError> {
    let workspace = workspace.ok_or(AppError::NoWorkspace)?;
    let id = component(id, "the board id")?;
    let name = component(image, "the image name")?;
    let path = boards(workspace).join(&id).join(&name);
    let meta = std::fs::metadata(&path).map_err(|err| AppError::from_io(err, &path))?;
    if !meta.is_file() {
        return Err(AppError::NotAFile(name));
    }
    if meta.len() > MAX_IMAGE_BYTES {
        return Err(AppError::TooLarge(format!(
            "{name} is {} bytes (limit {MAX_IMAGE_BYTES})",
            meta.len()
        )));
    }
    let bytes = std::fs::read(&path).map_err(|err| AppError::from_io(err, &path))?;
    use base64::Engine as _;
    Ok(json!({
        "file": name,
        "mime": mime(&path),
        "bytes": bytes.len(),
        "base64": base64::engine::general_purpose::STANDARD.encode(&bytes),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testtmp::TempDir;

    fn board(root: &TempDir, id: &str) -> PathBuf {
        let board = boards(root.path()).join(id);
        std::fs::create_dir_all(&board).unwrap();
        board
    }

    #[test]
    fn lists_boards_with_their_name_and_image_count() {
        let root = TempDir::new("boards-list");
        let first = board(&root, "alpha");
        std::fs::write(first.join("board.json"), r#"{"name":"Mood A"}"#).unwrap();
        std::fs::write(first.join("one.png"), [0_u8, 1, 2]).unwrap();
        std::fs::write(first.join("notes.txt"), "ignored").unwrap();
        board(&root, "beta");

        let listed = list(Some(root.path()));
        let items = listed["moodboards"].as_array().unwrap();
        assert_eq!(items.len(), 2);
        assert_eq!(items[0]["id"], "alpha");
        assert_eq!(items[0]["name"], "Mood A");
        assert_eq!(items[0]["images"], 1);
        assert_eq!(items[1]["name"], "beta");

        let read = read(Some(root.path()), "alpha").unwrap();
        assert_eq!(read["board"]["name"], "Mood A");
        assert_eq!(read["images"][0]["file"], "one.png");
    }

    #[test]
    fn refuses_a_name_that_could_climb_out_of_the_boards_folder() {
        let root = TempDir::new("boards-escape");
        for id in ["..", "../secrets", "a/b", ""] {
            assert!(read(Some(root.path()), id).is_err(), "{id}");
        }
        let board = board(&root, "alpha");
        std::fs::write(board.join("one.png"), [0_u8]).unwrap();
        assert!(image(Some(root.path()), "alpha", "../board.json").is_err());
        assert!(image(Some(root.path()), "alpha", "one.png").is_ok());
    }

    #[test]
    fn no_workspace_is_an_empty_list_not_an_error() {
        assert_eq!(list(None)["moodboards"].as_array().unwrap().len(), 0);
        assert!(read(None, "alpha").is_err());
    }
}
