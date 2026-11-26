//! code_search: ripgrep when the machine has it, a bounded walk when it does not.
//!
//! Nova ships no search engine of its own and does not want a regex crate
//! linked in for one tool, so this is a thin, honest wrapper:
//!
//! - with rg on PATH, the pattern is a real regex with ripgrep's own speed;
//! - without it, a literal, case-insensitive substring over text files, capped
//!   by file count and file size, with the engine named in the answer so a
//!   caller knows which of the two it got.
//!
//! Both paths refuse a pattern that begins with a dash: it would be read as a
//! flag by rg, and a tool that accepts flags is a tool that can be asked to
//! write files. Everything happens inside the open workspace.

use std::path::{Path, PathBuf};

use serde_json::{json, Value};

use crate::error::AppError;
use crate::workspace::WorkspaceGuard;

const MAX_MATCHES: usize = 100;
/// A walk that has not found anything in this many files is not going to.
const MAX_FILES: usize = 2000;
const MAX_FILE_BYTES: u64 = 512 * 1024;

const SKIP_DIRS: &[&str] = &[
    ".git",
    "node_modules",
    "target",
    "dist",
    "build",
    ".nova",
    ".temp",
    ".playwright-mcp",
    "vendor",
    "__pycache__",
    ".venv",
];

fn pattern(arguments: &Value) -> Result<String, AppError> {
    let pattern = arguments
        .get("pattern")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .trim()
        .to_string();
    if pattern.is_empty() || pattern.len() > 500 || pattern.contains('\0') {
        return Err(AppError::Tools(
            "a search pattern of 1 to 500 characters is required".into(),
        ));
    }
    if pattern.starts_with('-') {
        return Err(AppError::Tools(
            "a pattern may not begin with a dash: that would be read as a flag".into(),
        ));
    }
    Ok(pattern)
}

fn directory(guard: &WorkspaceGuard, arguments: &Value) -> Result<PathBuf, AppError> {
    let cwd = arguments
        .get("cwd")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|cwd| !cwd.is_empty());
    match cwd {
        Some(cwd) => guard.resolve(cwd),
        None => Ok(guard.root().to_path_buf()),
    }
}

pub async fn search(workspace: Option<&Path>, arguments: &Value) -> Result<Value, AppError> {
    let workspace = workspace.ok_or(AppError::NoWorkspace)?;
    let guard = WorkspaceGuard::new(workspace)?;
    let pattern = pattern(arguments)?;
    let cwd = directory(&guard, arguments)?;

    match ripgrep(&cwd, &pattern).await {
        Some(mut found) => {
            let truncated = found.len() > MAX_MATCHES;
            found.truncate(MAX_MATCHES);
            let matches = found
                .into_iter()
                .map(|(path, line, text)| {
                    // rg reports paths relative to the directory it searched;
                    // the walk reports them as it found them. Both are made
                    // workspace-relative here, so one tool cannot return two
                    // shapes depending on whether ripgrep happens to be
                    // installed.
                    let path = if path.is_absolute() {
                        path
                    } else {
                        cwd.join(path)
                    };
                    json!({
                        "path": guard.display_relative(&path),
                        "line": line,
                        "text": text,
                    })
                })
                .collect::<Vec<_>>();
            Ok(json!({ "engine": "rg", "matches": matches, "truncated": truncated }))
        }
        None => {
            let (found, scanned) = walk(&cwd, &pattern.to_lowercase());
            let truncated = found.len() > MAX_MATCHES;
            let matches = found
                .into_iter()
                .take(MAX_MATCHES)
                .map(|(path, line, text)| {
                    json!({
                        "path": guard.display_relative(&path),
                        "line": line,
                        "text": text,
                    })
                })
                .collect::<Vec<_>>();
            Ok(json!({
                "engine": "walk",
                "matches": matches,
                "truncated": truncated,
                "filesScanned": scanned,
            }))
        }
    }
}

/// None means "rg is not available here", which is a normal answer on a machine
/// without ripgrep installed.
///
/// rg is asked for --json rather than the default "path:line:text" because a
/// Windows path carries its own colon: a human-readable line has to guess where
/// the path ends, and a guess is a bug waiting for a folder named "a:b".
async fn ripgrep(cwd: &Path, pattern: &str) -> Option<Vec<(PathBuf, u64, String)>> {
    let mut command = tokio::process::Command::new("rg");
    command
        .args([
            "--json",
            "--max-count",
            "20",
            "--max-filesize",
            "2M",
            "-e",
            pattern,
            "--",
        ])
        .current_dir(cwd)
        .kill_on_drop(true);
    #[cfg(windows)]
    command.creation_flags(0x08000000); // CREATE_NO_WINDOW
    let output = command.output().await.ok()?;
    // 1 is "no matches", which is not a failure; anything above that (a bad
    // regex, a permission problem) is handed to the walk, which cannot fail on
    // syntax.
    if !output.status.success() && output.status.code() != Some(1) {
        return None;
    }
    Some(events(&String::from_utf8_lossy(&output.stdout)))
}

/// The match events out of rg's --json stream. Anything else (begin, end,
/// summary) is rg describing itself, not a hit.
fn events(stdout: &str) -> Vec<(PathBuf, u64, String)> {
    let mut found = Vec::new();
    for line in stdout.lines() {
        let Ok(event) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        if event.get("type").and_then(Value::as_str) != Some("match") {
            continue;
        }
        let data = &event["data"];
        let Some(path) = data
            .get("path")
            .and_then(|path| path.get("text"))
            .and_then(Value::as_str)
        else {
            continue;
        };
        found.push((
            PathBuf::from(path),
            data.get("line_number")
                .and_then(Value::as_u64)
                .unwrap_or_default(),
            data.get("lines")
                .and_then(|lines| lines.get("text"))
                .and_then(Value::as_str)
                .unwrap_or_default()
                .trim_end()
                .to_string(),
        ));
        if found.len() > MAX_MATCHES {
            break;
        }
    }
    found
}

/// The fallback: a depth-first walk over text files, stopping at the caps.
fn walk(root: &Path, needle: &str) -> (Vec<(PathBuf, u64, String)>, usize) {
    let mut found = Vec::new();
    let mut scanned = 0;
    let mut stack = vec![root.to_path_buf()];
    while let Some(directory) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&directory) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                let name = entry.file_name().to_string_lossy().to_ascii_lowercase();
                if !SKIP_DIRS.contains(&name.as_str()) {
                    stack.push(path);
                }
                continue;
            }
            if scanned >= MAX_FILES || found.len() > MAX_MATCHES {
                return (found, scanned);
            }
            if entry.metadata().map(|meta| meta.len()).unwrap_or(u64::MAX) > MAX_FILE_BYTES {
                continue;
            }
            let Ok(bytes) = std::fs::read(&path) else {
                continue;
            };
            if bytes.iter().take(1024).any(|byte| *byte == 0) {
                continue;
            }
            scanned += 1;
            let text = String::from_utf8_lossy(&bytes);
            for (index, line) in text.lines().enumerate() {
                if line.to_lowercase().contains(needle) {
                    found.push((path.clone(), index as u64 + 1, line.trim_end().to_string()));
                    if found.len() > MAX_MATCHES {
                        break;
                    }
                }
            }
        }
    }
    (found, scanned)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testtmp::TempDir;

    /// A Windows path in a "path:line:text" line is genuinely ambiguous, which
    /// is why rg is asked for JSON; this pins the shape that JSON has.
    #[test]
    fn keeps_a_windows_path_whole_from_the_json_stream() {
        let stdout = [
            r#"{"type":"begin","data":{"path":{"text":"C:\\work\\src"}}}"#,
            r#"{"type":"match","data":{"path":{"text":"C:\\work\\src\\main.rs"},"lines":{"text":"    let x = 1;\n"},"line_number":42,"absolute_offset":0,"submatches":[]}}"#,
            r#"{"type":"end","data":{"path":{"text":"C:\\work\\src\\main.rs"}}}"#,
        ]
        .join("\n");
        let found = events(&stdout);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].0, PathBuf::from(r"C:\work\src\main.rs"));
        assert_eq!(found[0].1, 42);
        assert_eq!(found[0].2, "    let x = 1;");
    }

    #[test]
    fn a_line_that_is_not_a_match_event_is_ignored() {
        assert!(events("no matches here").is_empty());
    }

    /// The fallback is tested through walk() rather than search(): search()
    /// prefers ripgrep when the machine has it, and a test that silently takes
    /// the other branch proves nothing about this one.
    #[test]
    fn the_walk_finds_a_literal_and_skips_vendored_directories() {
        let root = TempDir::new("search-walk");
        std::fs::create_dir_all(root.path().join("src")).unwrap();
        std::fs::create_dir_all(root.path().join("node_modules/pkg")).unwrap();
        std::fs::write(
            root.path().join("src/main.rs"),
            "fn main() {}\nlet needle = 1;\n",
        )
        .unwrap();
        std::fs::write(root.path().join("node_modules/pkg/index.js"), "needle\n").unwrap();

        let (found, scanned) = walk(root.path(), "needle");
        assert_eq!(found.len(), 1, "{found:?}");
        assert_eq!(found[0].0, root.path().join("src/main.rs"));
        assert_eq!(found[0].1, 2);
        assert!(scanned >= 1);
    }

    /// Whichever engine this machine has, the answer has the same shape and the
    /// path is workspace-relative — not cwd-relative for ripgrep and absolute
    /// for the walk.
    #[tokio::test]
    async fn search_answers_with_whichever_engine_it_has() {
        let root = TempDir::new("search-engine");
        std::fs::create_dir_all(root.path().join("src")).unwrap();
        std::fs::write(root.path().join("src/main.rs"), "let needle = 1;\n").unwrap();

        let found = search(
            Some(root.path()),
            &json!({ "pattern": "needle", "cwd": "src" }),
        )
        .await
        .unwrap();
        assert!(
            matches!(found["engine"].as_str(), Some("rg") | Some("walk")),
            "{found}"
        );
        let matches = found["matches"].as_array().unwrap();
        assert_eq!(matches.len(), 1, "{found}");
        assert_eq!(matches[0]["path"], "src/main.rs");
        assert_eq!(matches[0]["line"], 1);
    }

    #[tokio::test]
    async fn a_pattern_that_looks_like_a_flag_is_refused() {
        let root = TempDir::new("search-flag");
        let error = search(Some(root.path()), &json!({ "pattern": "--exec=calc" }))
            .await
            .unwrap_err();
        assert!(matches!(error, AppError::Tools(_)), "got {error:?}");
    }

    #[tokio::test]
    async fn no_workspace_is_an_error_not_an_empty_result() {
        assert!(search(None, &json!({ "pattern": "x" })).await.is_err());
    }
}
