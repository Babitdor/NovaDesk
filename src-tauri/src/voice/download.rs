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
use crate::voice::catalog::{self, Model, Source};

/// Progress frames for one download, in bytes.
///
/// `phase` names what is arriving, because one install can be two transfers: a
/// voice from the speech catalogue first fetches Piper itself (about 22 MB), then
/// the voice (about 63 MB). The numbers restart with each phase, and the page
/// needs to say which one it is watching.
#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Progress {
    pub received: u64,
    pub total: u64,
    pub phase: &'static str,
}

/// Written last, so a half-finished download never looks installed.
pub const MANIFEST: &str = ".nova-model.json";

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Manifest {
    pub id: String,
    /// Which engine the download was for, as a plain string. Informational only:
    /// what decides whether something is installed and runnable is the files on
    /// disk plus this build's own support, so an older manifest still reads.
    pub engine: String,
    pub files: Vec<String>,
    pub bytes: u64,
}

/// One install, described without saying what is being installed.
///
/// A dictation model, a Piper voice and the Piper binary itself all reduce to
/// the same three things: files with exact sizes, the names that prove the
/// install is complete, and the folder they belong in.
pub struct Spec<'a> {
    pub id: &'a str,
    /// (file name on disk, absolute URL, bytes)
    pub files: &'a [(&'a str, &'a str, u64)],
    pub bytes: u64,
    pub required: &'a [&'a str],
    /// Recorded in the manifest, never consulted.
    pub engine: &'a str,
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
///
/// A thin adapter: it decides which model, where it lives and whether it is
/// already there, then hands the transfer to `install_spec` — the same code the
/// speech voices use, which is why there is only one copy of the size check.
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
    match model.source {
        Some(Source::Files(files)) => {
            let spec = Spec {
                id: model.id,
                files,
                bytes: model.bytes,
                required: model.required,
                engine: model.engine.label(),
            };
            install_spec(&spec, &dir, cancel, on, phase::MODEL).await
        }
        Some(Source::Archive(url, bytes)) => {
            install_archive(app, model, url, bytes, &dir, cancel, on).await
        }
        None => Err(AppError::Voice(format!("{id} has nothing to download"))),
    }
}

/// Phase labels for a progress frame the page can name.
pub mod phase {
    pub const MODEL: &str = "model";
    pub const VOICE: &str = "voice";
    pub const RUNTIME: &str = "runtime";
}

/// The one shape every file download takes: create the folder, fetch each file
/// and check its size, then write the manifest last so a half-finished install
/// never looks done.
pub async fn install_spec(
    spec: &Spec<'_>,
    dir: &Path,
    cancel: &CancellationToken,
    on: &Channel<Progress>,
    phase: &'static str,
) -> Result<(), AppError> {
    tokio::fs::create_dir_all(dir)
        .await
        .map_err(|err| AppError::from_io(err, dir))?;
    let mut reporter = Reporter::new(spec.bytes, phase);
    emit(on, reporter.frame(0));
    for (name, url, bytes) in spec.files {
        let path = dir.join(name);
        fetch(url, &path, *bytes, true, cancel, &mut reporter, on).await?;
    }
    write_manifest(dir, spec).await?;
    emit(on, reporter.finish());
    Ok(())
}

async fn write_manifest(dir: &Path, spec: &Spec<'_>) -> Result<(), AppError> {
    let manifest = Manifest {
        id: spec.id.to_string(),
        engine: spec.engine.to_string(),
        files: spec.required.iter().map(|name| name.to_string()).collect(),
        bytes: spec.bytes,
    };
    let json = serde_json::to_vec_pretty(&manifest)
        .map_err(|err| AppError::Voice(format!("could not describe the model: {err}")))?;
    let path = dir.join(MANIFEST);
    tokio::fs::write(&path, json)
        .await
        .map_err(|err| AppError::from_io(err, &path))
}

/// The dictation models that arrive as one tar.gz.
async fn install_archive(
    app: &AppHandle,
    model: &Model,
    url: &str,
    bytes: u64,
    dir: &Path,
    cancel: &CancellationToken,
    on: &Channel<Progress>,
) -> Result<(), AppError> {
    let root = root(app)?;
    let archive = root.join(format!("{}.tar.gz", model.id));
    let staging = root.join(format!("{}.unpack", model.id));
    let mut reporter = Reporter::new(model.bytes, phase::MODEL);
    emit(on, reporter.frame(0));
    fetch(url, &archive, bytes, true, cancel, &mut reporter, on).await?;
    if staging.exists() {
        tokio::fs::remove_dir_all(&staging)
            .await
            .map_err(|err| AppError::from_io(err, &staging))?;
    }
    let (archive_path, staging_path, required) =
        (archive.clone(), staging.clone(), model.required.to_vec());
    tauri::async_runtime::spawn_blocking(move || unpack(&archive_path, &staging_path, &required))
        .await
        .map_err(|err| AppError::Voice(format!("the archive unpacking task failed: {err}")))??;
    tokio::fs::create_dir_all(dir)
        .await
        .map_err(|err| AppError::from_io(err, dir))?;
    for name in model.required {
        let from = staging.join(name);
        tokio::fs::rename(&from, dir.join(name))
            .await
            .map_err(|err| AppError::from_io(err, &from))?;
    }
    let _ = tokio::fs::remove_dir_all(&staging).await;
    let _ = tokio::fs::remove_file(&archive).await;
    let spec = Spec {
        id: model.id,
        files: &[],
        bytes: model.bytes,
        required: model.required,
        engine: model.engine.label(),
    };
    write_manifest(dir, &spec).await?;
    emit(on, reporter.finish());
    Ok(())
}

/// Whether every file and folder an archive is expected to contain is there.
///
/// The Piper runtime is a folder of executables and a directory of phoneme data,
/// so a half-unpacked archive has to fail this outright: a missing espeak-ng-data
/// is a Piper that starts and then cannot read anything aloud.
pub fn archive_ready(dir: &Path, files: &[&str], dirs: &[&str]) -> bool {
    dir.is_dir()
        && files.iter().all(|name| dir.join(name).is_file())
        && dirs.iter().all(|name| dir.join(name).is_dir())
}

/// Fetch a zip and unpack it into `dir`, reporting both transfers as phases of
/// one progress channel.
///
/// A zip rather than a tarball because there is no tar on a Windows host: the
/// GNU tar that ships with git-bash is not on PATH for a GUI app, and `tar.exe`
/// in System32 reads tar and not zip.
pub async fn install_zip(
    url: &str,
    bytes: u64,
    dir: &Path,
    cancel: &CancellationToken,
    on: &Channel<Progress>,
    phase: &'static str,
) -> Result<(), AppError> {
    let parent = dir.parent().ok_or_else(|| {
        AppError::Voice(format!("{} has no folder to download into", dir.display()))
    })?;
    tokio::fs::create_dir_all(parent)
        .await
        .map_err(|err| AppError::from_io(err, parent))?;
    let name = dir
        .file_name()
        .map(|name| name.to_string_lossy().to_string())
        .unwrap_or_else(|| "archive".to_string());
    let archive = parent.join(format!("{name}.zip"));
    let mut reporter = Reporter::new(bytes, phase);
    emit(on, reporter.frame(0));
    fetch(url, &archive, bytes, true, cancel, &mut reporter, on).await?;

    if dir.exists() {
        tokio::fs::remove_dir_all(dir)
            .await
            .map_err(|err| AppError::from_io(err, dir))?;
    }
    let (from, to) = (archive.clone(), dir.to_path_buf());
    tauri::async_runtime::spawn_blocking(move || unpack_zip(&from, &to))
        .await
        .map_err(|err| AppError::Voice(format!("the archive unpacking task failed: {err}")))??;
    let _ = tokio::fs::remove_file(&archive).await;
    emit(on, reporter.finish());
    Ok(())
}

/// Unpack a zip, refusing any entry that would land outside `dir`.
///
/// An archive that wraps everything in one folder loses that folder. The Piper
/// release ships as `piper/piper.exe` plus `piper/espeak-ng-data/`, and an
/// install that kept the wrapper would put the executable one level deeper than
/// the engine, the catalogue and archive_ready all look for it.
fn unpack_zip(archive: &Path, dir: &Path) -> Result<(), AppError> {
    std::fs::create_dir_all(dir).map_err(|err| AppError::from_io(err, dir))?;
    let file = std::fs::File::open(archive).map_err(|err| AppError::from_io(err, archive))?;
    let mut zip = zip::ZipArchive::new(std::io::BufReader::new(file))
        .map_err(|err| AppError::Voice(format!("the archive could not be read: {err}")))?;
    let wrapper = wrapper_folder(&mut zip);
    for index in 0..zip.len() {
        let mut entry = zip
            .by_index(index)
            .map_err(|err| AppError::Voice(format!("the archive could not be read: {err}")))?;
        // enclosed_name is the ../ guard: anything that would land outside the
        // target folder is not extracted at all. The archive's own size is
        // pinned by the fetch, not by the zip crate.
        let Some(name) = entry.enclosed_name() else {
            continue;
        };
        let name = match &wrapper {
            // The wrapper entry itself has nothing left once it is stripped.
            Some(top) => match name.strip_prefix(top) {
                Ok(rest) if !rest.as_os_str().is_empty() => rest.to_path_buf(),
                _ => continue,
            },
            None => name,
        };
        let target = dir.join(name);
        if entry.is_dir() {
            std::fs::create_dir_all(&target).map_err(|err| AppError::from_io(err, &target))?;
            continue;
        }
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent).map_err(|err| AppError::from_io(err, parent))?;
        }
        let mut out =
            std::fs::File::create(&target).map_err(|err| AppError::from_io(err, &target))?;
        std::io::copy(&mut entry, &mut out).map_err(|err| AppError::from_io(err, &target))?;
    }
    Ok(())
}

/// The one folder an archive wraps everything in, if it has one.
///
/// Read before anything is written, so the decision cannot depend on the order
/// entries arrived in. A single top-level file next to a folder is not a
/// wrapper, and an entry that would escape makes the whole answer "no wrapper":
/// the guarded path then extracts exactly what it did before.
fn wrapper_folder(zip: &mut zip::ZipArchive<std::io::BufReader<std::fs::File>>) -> Option<String> {
    let mut top: Option<String> = None;
    let mut nested = false;
    for index in 0..zip.len() {
        let entry = zip.by_index(index).ok()?;
        let name = entry.enclosed_name()?;
        let mut parts = name.components();
        let first = parts.next()?.as_os_str().to_string_lossy().to_string();
        if parts.next().is_some() {
            nested = true;
        }
        match &top {
            Some(known) if *known != first => return None,
            Some(_) => {}
            None => top = Some(first),
        }
    }
    if nested {
        top
    } else {
        None
    }
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
    phase: &'static str,
}

impl Reporter {
    fn new(total: u64, phase: &'static str) -> Self {
        Self {
            total,
            done: 0,
            sent: None,
            phase,
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
            phase: self.phase,
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
        let mut reporter = Reporter::new(100, phase::VOICE);
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
        assert_eq!(last.phase, phase::VOICE, "the page names the transfer");
    }

    #[test]
    fn a_spec_round_trips_through_the_manifest() {
        let dir = TempDir::new("spec-manifest");
        let spec = Spec {
            id: "en_US-amy-low",
            files: &[("en_US-amy-low.onnx", "https://example.test/a.onnx", 10)],
            bytes: 10,
            required: &["en_US-amy-low.onnx"],
            engine: "piper",
        };
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("a runtime");
        runtime
            .block_on(write_manifest(dir.path(), &spec))
            .expect("written");

        let raw = std::fs::read(dir.path().join(MANIFEST)).expect("the manifest is on disk");
        let manifest: Manifest = serde_json::from_slice(&raw).expect("it parses");
        assert_eq!(manifest.id, "en_US-amy-low");
        assert_eq!(manifest.engine, "piper");
        assert_eq!(manifest.bytes, 10);
        assert_eq!(manifest.files, vec!["en_US-amy-low.onnx".to_string()]);
    }

    #[test]
    fn an_archive_is_ready_only_when_its_files_and_folders_are() {
        let dir = TempDir::new("archive-ready");
        assert!(!archive_ready(
            dir.path(),
            &["piper.exe"],
            &["espeak-ng-data"]
        ));
        std::fs::write(dir.path().join("piper.exe"), b"x").expect("write");
        assert!(
            !archive_ready(dir.path(), &["piper.exe"], &["espeak-ng-data"]),
            "a missing phoneme folder is a Piper that cannot speak"
        );
        std::fs::create_dir_all(dir.path().join("espeak-ng-data")).expect("folder");
        assert!(archive_ready(
            dir.path(),
            &["piper.exe"],
            &["espeak-ng-data"]
        ));
    }

    #[test]
    fn a_zip_unpacks_with_its_folders_intact() {
        let dir = TempDir::new("unpack-zip");
        let archive = dir.path().join("bundle.zip");
        {
            let file = std::fs::File::create(&archive).expect("create");
            let mut zip = zip::ZipWriter::new(file);
            let options = zip::write::SimpleFileOptions::default();
            use std::io::Write;
            zip.start_file("piper.exe", options).expect("entry");
            zip.write_all(b"binary").expect("write");
            zip.add_directory("espeak-ng-data", options)
                .expect("folder");
            zip.start_file("espeak-ng-data/en_dict", options)
                .expect("entry");
            zip.write_all(b"dict").expect("write");
            zip.finish().expect("finished");
        }
        let out = dir.path().join("piper");
        unpack_zip(&archive, &out).expect("unpacked");
        assert!(archive_ready(&out, &["piper.exe"], &["espeak-ng-data"]));
        assert_eq!(
            std::fs::read(out.join("espeak-ng-data").join("en_dict")).expect("the dictionary"),
            b"dict"
        );
    }

    #[test]
    fn a_bundle_that_wraps_everything_in_one_folder_is_flattened() {
        let dir = TempDir::new("unpack-wrapped");
        let archive = dir.path().join("bundle.zip");
        {
            let file = std::fs::File::create(&archive).expect("create");
            let mut zip = zip::ZipWriter::new(file);
            let options = zip::write::SimpleFileOptions::default();
            use std::io::Write;
            zip.add_directory("piper", options).expect("folder");
            zip.start_file("piper/piper.exe", options).expect("entry");
            zip.write_all(b"binary").expect("write");
            zip.add_directory("piper/espeak-ng-data", options)
                .expect("folder");
            zip.start_file("piper/espeak-ng-data/en_dict", options)
                .expect("entry");
            zip.write_all(b"dict").expect("write");
            zip.finish().expect("finished");
        }
        let out = dir.path().join("piper");
        unpack_zip(&archive, &out).expect("unpacked");
        assert!(
            archive_ready(&out, &["piper.exe"], &["espeak-ng-data"]),
            "the wrapper folder is not part of the installed layout"
        );
        assert_eq!(
            std::fs::read(out.join("espeak-ng-data").join("en_dict")).expect("the dictionary"),
            b"dict"
        );
    }

    #[test]
    fn a_zip_with_a_file_beside_a_folder_keeps_its_shape() {
        let dir = TempDir::new("unpack-flat");
        let archive = dir.path().join("bundle.zip");
        {
            let file = std::fs::File::create(&archive).expect("create");
            let mut zip = zip::ZipWriter::new(file);
            let options = zip::write::SimpleFileOptions::default();
            use std::io::Write;
            zip.start_file("piper.exe", options).expect("entry");
            zip.write_all(b"binary").expect("write");
            zip.add_directory("espeak-ng-data", options)
                .expect("folder");
            zip.finish().expect("finished");
        }
        let out = dir.path().join("piper");
        unpack_zip(&archive, &out).expect("unpacked");
        // Two different first components: nothing is a wrapper, so nothing is
        // stripped, which is what the on-device voice archives rely on.
        assert!(archive_ready(&out, &["piper.exe"], &["espeak-ng-data"]));
    }

    #[test]
    fn a_zip_entry_that_climbs_out_is_not_written() {
        let dir = TempDir::new("unpack-escape");
        let archive = dir.path().join("evil.zip");
        {
            let file = std::fs::File::create(&archive).expect("create");
            let mut zip = zip::ZipWriter::new(file);
            let options = zip::write::SimpleFileOptions::default();
            use std::io::Write;
            zip.start_file("../escaped.txt", options).expect("entry");
            zip.write_all(b"nope").expect("write");
            zip.finish().expect("finished");
        }
        let out = dir.path().join("piper");
        unpack_zip(&archive, &out).expect("nothing to unpack");
        assert!(
            !dir.path().join("escaped.txt").exists(),
            "the entry escaped the folder"
        );
    }
}
