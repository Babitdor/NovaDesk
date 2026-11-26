//! Model downloads and the on-disk layout under the app data folder.
//!
//! Transfers go through the system curl, the same helper the theme installer
//! uses: it already speaks TLS and Range, so a 671 MB download resumes instead
//! of restarting when the page is closed. Progress is measured from the file on
//! disk rather than parsed from curl's stderr, which keeps the reported number
//! and the bytes actually kept in sync.

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tauri::ipc::Channel;
use tauri::{AppHandle, Manager};
use tokio_util::sync::CancellationToken;

use crate::commands::settings::system_command;
use crate::error::AppError;
use crate::voice::catalog::{self, Engine, Model, Source};

/// Progress frames for one download, in bytes.
#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Progress {
    pub received: u64,
    pub total: u64,
}

/// Written last, so a half-finished download never looks installed.
pub const MANIFEST: &str = ".nova-model.json";

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Manifest {
    pub id: String,
    pub engine: Engine,
    pub files: Vec<String>,
    pub bytes: u64,
}

const POLL: Duration = Duration::from_millis(120);

/// <app data>/voice - everything Voice owns lives under here.
pub fn root(app: &AppHandle) -> Result<PathBuf, AppError> {
    let dir = app
        .path()
        .app_data_dir()
        .map_err(|err| AppError::Voice(format!("could not resolve the model folder: {err}")))?;
    Ok(dir.join("voice"))
}

pub fn model_dir(app: &AppHandle, id: &str) -> Result<PathBuf, AppError> {
    Ok(root(app)?.join(id))
}

/// Installed means the manifest is there and every file the engine needs is
/// there with it; a partially extracted archive fails this.
pub fn is_installed(dir: &Path, model: &Model) -> bool {
    dir.join(MANIFEST).is_file() && model.required.iter().all(|name| dir.join(name).is_file())
}

pub fn installed(app: &AppHandle) -> Result<Vec<String>, AppError> {
    let root = root(app)?;
    Ok(catalog::MODELS
        .iter()
        .filter(|model| is_installed(&root.join(model.id), model))
        .map(|model| model.id.to_string())
        .collect())
}

/// Fetch, verify and install one model. Returns once the model is usable.
pub async fn install(
    app: &AppHandle,
    id: &str,
    cancel: &CancellationToken,
    on: &Channel<Progress>,
) -> Result<(), AppError> {
    let model = catalog::find(id)
        .ok_or_else(|| AppError::Voice(format!("Nova does not know a model called {id}")))?;
    if !model.supported {
        return Err(AppError::Voice(
            model
                .unsupported_reason
                .unwrap_or("this model cannot run on this build")
                .to_string(),
        ));
    }
    let dir = model_dir(app, model.id)?;
    if is_installed(&dir, model) {
        return Ok(());
    }
    tokio::fs::create_dir_all(&dir)
        .await
        .map_err(|err| AppError::from_io(err, &dir))?;

    let mut reporter = Reporter::new(model.bytes);
    emit(on, reporter.frame(0));
    match model.source {
        Some(Source::Files(files)) => {
            for (name, url, bytes) in files {
                let path = dir.join(name);
                fetch(url, &path, *bytes, true, cancel, &mut reporter, on).await?;
            }
        }
        Some(Source::Archive(url, bytes)) => {
            let root = root(app)?;
            let archive = root.join(format!("{id}.tar.gz"));
            let staging = root.join(format!("{id}.unpack"));
            fetch(url, &archive, bytes, true, cancel, &mut reporter, on).await?;
            if staging.exists() {
                tokio::fs::remove_dir_all(&staging)
                    .await
                    .map_err(|err| AppError::from_io(err, &staging))?;
            }
            let (archive_path, staging_path, required) =
                (archive.clone(), staging.clone(), model.required.to_vec());
            tauri::async_runtime::spawn_blocking(move || {
                unpack(&archive_path, &staging_path, &required)
            })
            .await
            .map_err(|err| {
                AppError::Voice(format!("the archive unpacking task failed: {err}"))
            })??;
            for name in model.required {
                let from = staging.join(name);
                tokio::fs::rename(&from, dir.join(name))
                    .await
                    .map_err(|err| AppError::from_io(err, &from))?;
            }
            let _ = tokio::fs::remove_dir_all(&staging).await;
            let _ = tokio::fs::remove_file(&archive).await;
        }
        None => return Err(AppError::Voice(format!("{id} has nothing to download"))),
    }

    let manifest = Manifest {
        id: model.id.to_string(),
        engine: model.engine,
        files: model.required.iter().map(|name| name.to_string()).collect(),
        bytes: model.bytes,
    };
    let json = serde_json::to_vec_pretty(&manifest)
        .map_err(|err| AppError::Voice(format!("could not describe the model: {err}")))?;
    let path = dir.join(MANIFEST);
    tokio::fs::write(&path, json)
        .await
        .map_err(|err| AppError::from_io(err, &path))?;
    emit(on, reporter.finish());
    Ok(())
}

pub async fn remove(app: &AppHandle, id: &str) -> Result<(), AppError> {
    if catalog::find(id).is_none() {
        return Err(AppError::Voice(format!(
            "Nova does not know a model called {id}"
        )));
    }
    let dir = model_dir(app, id)?;
    match tokio::fs::remove_dir_all(&dir).await {
        Ok(()) => Ok(()),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(err) => Err(AppError::from_io(err, &dir)),
    }
}

/// Running total for one install. Kept free of the channel so the accounting
/// can be tested on its own; the caller decides what to do with each frame.
struct Reporter {
    total: u64,
    done: u64,
    sent: Option<u64>,
}

impl Reporter {
    fn new(total: u64) -> Self {
        Self {
            total,
            done: 0,
            sent: None,
        }
    }

    /// Returns the frame to emit, or None when the number has not moved.
    /// "file" is how much of the current file is on disk.
    fn frame(&mut self, file: u64) -> Option<Progress> {
        let received = (self.done + file).min(self.total);
        if self.sent == Some(received) {
            return None;
        }
        self.sent = Some(received);
        Some(Progress {
            received,
            total: self.total,
        })
    }

    fn complete_file(&mut self, bytes: u64) -> Option<Progress> {
        self.done = (self.done + bytes).min(self.total);
        self.frame(0)
    }

    fn finish(&mut self) -> Option<Progress> {
        self.done = self.total;
        self.frame(0)
    }
}

fn emit(on: &Channel<Progress>, frame: Option<Progress>) {
    if let Some(frame) = frame {
        let _ = on.send(frame);
    }
}

async fn file_len(path: &Path) -> u64 {
    tokio::fs::metadata(path)
        .await
        .map(|meta| meta.len())
        .unwrap_or(0)
}

/// Stream one URL to one path, resuming whatever is already there.
async fn fetch(
    url: &str,
    path: &Path,
    expected: u64,
    resume: bool,
    cancel: &CancellationToken,
    reporter: &mut Reporter,
    on: &Channel<Progress>,
) -> Result<(), AppError> {
    let already = file_len(path).await;
    if already == expected && expected > 0 {
        emit(on, reporter.complete_file(expected));
        return Ok(());
    }
    if already > expected {
        let _ = tokio::fs::remove_file(path).await;
    }
    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .map_err(|err| AppError::from_io(err, parent))?;
    }

    let resumed = resume && file_len(path).await > 0;
    let mut command = system_command("curl");
    command.args([
        "-q",
        "--silent",
        "--show-error",
        "--fail",
        "--location",
        "--max-redirs",
        "3",
        "--proto",
        "=https",
        "--proto-redir",
        "=https",
        "--user-agent",
        "NovaDesktop",
        "--create-dirs",
        "--output",
    ]);
    command.arg(path);
    if resumed {
        command.args(["--continue-at", "-"]);
    }
    command.arg(url);
    command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .kill_on_drop(true);
    let mut child = command
        .spawn()
        .map_err(|err| AppError::Spawn(format!("could not start curl: {err}")))?;

    let status = loop {
        if cancel.is_cancelled() {
            let _ = child.start_kill();
            let _ = child.wait().await;
            return Err(AppError::Cancelled);
        }
        emit(on, reporter.frame(file_len(path).await));
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) => tokio::time::sleep(POLL).await,
            Err(err) => {
                return Err(AppError::Spawn(format!(
                    "the download could not be watched: {err}"
                )))
            }
        }
    };

    if !status.success() {
        // 33 is curl's "this server cannot resume": drop the part file and let
        // the caller start clean instead of failing a download the user asked for.
        if status.code() == Some(33) && resumed {
            let _ = tokio::fs::remove_file(path).await;
            return Box::pin(fetch(url, path, expected, false, cancel, reporter, on)).await;
        }
        return Err(describe_curl_failure(status.code()));
    }

    let size = file_len(path).await;
    if expected > 0 && size != expected {
        let _ = tokio::fs::remove_file(path).await;
        return Err(AppError::Voice(format!(
            "the download stopped at {size} of {expected} bytes; nothing was installed"
        )));
    }
    emit(on, reporter.complete_file(size));
    Ok(())
}

fn describe_curl_failure(code: Option<i32>) -> AppError {
    match code {
        Some(22) => AppError::Voice(
            "the model host refused the download; the file may have moved".to_string(),
        ),
        Some(6) | Some(7) => {
            AppError::Voice("could not reach the model host; check the connection".to_string())
        }
        Some(28) => AppError::Voice("the download timed out".to_string()),
        Some(code) => AppError::Voice(format!("the download failed (curl exit {code})")),
        None => AppError::Voice("the download was terminated".to_string()),
    }
}

/// Extract an archive and pull the files the engine needs up to the top of the
/// staging folder, wherever the archive happened to put them.
fn unpack(archive: &Path, staging: &Path, required: &[&'static str]) -> Result<(), AppError> {
    let raw = staging.join("raw");
    std::fs::create_dir_all(&raw).map_err(|err| AppError::from_io(err, &raw))?;
    let file = std::fs::File::open(archive).map_err(|err| AppError::from_io(err, archive))?;
    let mut tar = tar::Archive::new(flate2::read::GzDecoder::new(std::io::BufReader::new(file)));
    tar.unpack(&raw)
        .map_err(|err| AppError::Voice(format!("the archive could not be unpacked: {err}")))?;

    let found = find_dir_holding(&raw, required).ok_or_else(|| {
        AppError::Voice(format!(
            "the archive did not contain {}",
            required.join(", ")
        ))
    })?;
    for name in required {
        let from = found.join(name);
        let to = staging.join(name);
        std::fs::rename(&from, &to).map_err(|err| AppError::from_io(err, &from))?;
    }
    let _ = std::fs::remove_dir_all(&raw);
    Ok(())
}

/// Shallowest directory holding every required file, breadth first so a
/// top-level layout wins over a nested copy of the same names.
fn find_dir_holding(root: &Path, required: &[&'static str]) -> Option<PathBuf> {
    let mut queue = vec![root.to_path_buf()];
    let mut seen = 0;
    while let Some(dir) = queue.first().cloned() {
        queue.remove(0);
        seen += 1;
        if seen > 512 {
            return None;
        }
        if required.iter().all(|name| dir.join(name).is_file()) {
            return Some(dir);
        }
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                queue.push(path);
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testtmp::TempDir;

    #[test]
    fn finds_nested_model_directory() {
        let dir = TempDir::new("unpack-nested");
        let nested = dir.path().join("cohere-int4").join("onnx");
        std::fs::create_dir_all(&nested).expect("nested dir");
        for name in [
            "cohere-encoder.int4.onnx",
            "cohere-decoder.int4.onnx",
            "tokens.txt",
        ] {
            std::fs::write(nested.join(name), b"x").expect("write");
        }
        let found = find_dir_holding(
            dir.path(),
            &[
                "cohere-encoder.int4.onnx",
                "cohere-decoder.int4.onnx",
                "tokens.txt",
            ],
        )
        .expect("a directory holding every file");
        assert_eq!(found, nested);
    }

    #[test]
    fn reports_a_missing_file_instead_of_guessing() {
        let dir = TempDir::new("unpack-missing");
        std::fs::write(dir.path().join("tokens.txt"), b"x").expect("write");
        assert!(
            find_dir_holding(dir.path(), &["cohere-encoder.int4.onnx", "tokens.txt"]).is_none()
        );
    }

    #[test]
    fn progress_never_exceeds_the_total() {
        let mut reporter = Reporter::new(100);
        assert_eq!(reporter.frame(0).map(|frame| frame.received), Some(0));
        assert!(
            reporter.frame(0).is_none(),
            "an unchanged total is not re-sent"
        );
        reporter
            .complete_file(60)
            .expect("a frame for the finished file");
        let last = reporter.finish().expect("a final frame");
        assert_eq!((last.received, last.total), (100, 100));
    }
}
