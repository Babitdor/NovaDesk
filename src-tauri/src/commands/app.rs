//! The About page's data, all of it read from the running process.
//!
//! Every number here is measured, never estimated. The memory figure is this
//! process's own working set: the WebView (a separate process) and each agent
//! CLI are deliberately excluded, so the row is labelled "native core" rather
//! than pretending to be the whole app.

use serde::Serialize;
use serde_json::Value;
use tauri::{AppHandle, Manager, State};

use crate::commands::settings::download;
use crate::error::AppError;
use crate::AppState;

/// GitHub's "latest release" endpoint. One unauthenticated GET (60/h), issued
/// only when the About page opens or the user presses Check now — never polled.
const RELEASES: &str = "https://api.github.com/repos/Babitdor/NovaDesk/releases/latest";

/// Release notes longer than this are clipped; the sheet links to GitHub instead.
const NOTES_LIMIT: usize = 1200;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppReport {
    version: String,
    identifier: String,
    os: &'static str,
    arch: &'static str,
    /// Resident set size in bytes, or `None` when the platform query fails — the
    /// pane prints an em dash rather than an error.
    rss_bytes: Option<u64>,
    uptime_secs: u64,
    /// Turns currently running, i.e. agents holding a live cancel token.
    agents: usize,
    terminals: usize,
    data_dir: Option<String>,
}

#[tauri::command]
pub fn app_report(app: AppHandle, state: State<'_, AppState>) -> AppReport {
    AppReport {
        version: app.package_info().version.to_string(),
        identifier: app.config().identifier.clone(),
        os: std::env::consts::OS,
        arch: std::env::consts::ARCH,
        rss_bytes: resident_bytes(),
        uptime_secs: state.uptime().as_secs(),
        agents: state.live_turns(),
        terminals: state.open_terminals(),
        data_dir: app
            .path()
            .app_data_dir()
            .ok()
            .map(|dir| dir.display().to_string()),
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateCheck {
    current: String,
    latest: String,
    url: Option<String>,
    published_at: Option<String>,
    notes: Option<String>,
    /// True when the published release is newer than this build.
    available: bool,
}

#[tauri::command]
pub async fn check_app_update(app: AppHandle) -> Result<UpdateCheck, AppError> {
    let body = download(RELEASES, 1024 * 1024)
        .await
        .map_err(|err| AppError::Agent(format!("Could not reach GitHub: {err}")))?;
    let release: Value = serde_json::from_slice(&body)
        .map_err(|_| AppError::Agent("GitHub returned a response Nova could not read".into()))?;
    let current = app.package_info().version.to_string();
    // Tags are conventionally `v0.3.0`; semver rejects the leading v.
    let latest = release["tag_name"]
        .as_str()
        .unwrap_or_default()
        .trim()
        .trim_start_matches('v')
        .to_string();
    if latest.is_empty() {
        return Err(AppError::Agent("GitHub returned no release tag".into()));
    }
    Ok(UpdateCheck {
        available: is_newer(&latest, &current),
        notes: release["body"]
            .as_str()
            .map(|notes| notes.chars().take(NOTES_LIMIT).collect::<String>())
            .filter(|notes| !notes.trim().is_empty()),
        url: release["html_url"].as_str().map(str::to_string),
        published_at: release["published_at"].as_str().map(str::to_string),
        latest,
        current,
    })
}

/// True when `latest` is strictly newer than `current`.
///
/// Prerelease tags (`1.2.0-rc.1`) parse and compare as semver, so a build ahead
/// of the last release stays quiet. A tag that does not parse at all falls back
/// to a plain difference: better to offer a release Nova cannot read than to
/// hide it behind "Up to date".
pub fn is_newer(latest: &str, current: &str) -> bool {
    match (
        semver::Version::parse(latest),
        semver::Version::parse(current),
    ) {
        (Ok(latest), Ok(current)) => latest > current,
        _ => latest != current,
    }
}

/// Working set of this process, in bytes.
#[cfg(windows)]
fn resident_bytes() -> Option<u64> {
    use std::mem::size_of;

    // PROCESS_MEMORY_COUNTERS, verbatim: every field is SIZE_T except the two
    // leading DWORDs, and `cb` must be the struct's exact size or the call fails.
    #[repr(C)]
    #[allow(non_snake_case)]
    struct MemoryCounters {
        cb: u32,
        PageFaultCount: u32,
        PeakWorkingSetSize: usize,
        WorkingSetSize: usize,
        QuotaPeakPagedPoolUsage: usize,
        QuotaPagedPoolUsage: usize,
        QuotaPeakNonPagedPoolUsage: usize,
        QuotaNonPagedPoolUsage: usize,
        PagefileUsage: usize,
        PeakPagefileUsage: usize,
    }

    #[link(name = "kernel32")]
    extern "system" {
        fn GetCurrentProcess() -> isize;
        fn K32GetProcessMemoryInfo(process: isize, counters: *mut MemoryCounters, cb: u32) -> i32;
    }

    let mut counters = MemoryCounters {
        cb: 0,
        PageFaultCount: 0,
        PeakWorkingSetSize: 0,
        WorkingSetSize: 0,
        QuotaPeakPagedPoolUsage: 0,
        QuotaPagedPoolUsage: 0,
        QuotaPeakNonPagedPoolUsage: 0,
        QuotaNonPagedPoolUsage: 0,
        PagefileUsage: 0,
        PeakPagefileUsage: 0,
    };
    counters.cb = size_of::<MemoryCounters>() as u32;
    // SAFETY: the pseudo-handle addresses this process, and `counters` is a
    // correctly sized, writable buffer of the layout the call expects.
    let ok = unsafe { K32GetProcessMemoryInfo(GetCurrentProcess(), &mut counters, counters.cb) };
    (ok != 0).then_some(counters.WorkingSetSize as u64)
}

#[cfg(all(unix, not(target_os = "macos")))]
fn resident_bytes() -> Option<u64> {
    // VmRSS is reported in kB, so no page-size lookup is needed.
    let status = std::fs::read_to_string("/proc/self/status").ok()?;
    let line = status.lines().find(|line| line.starts_with("VmRSS:"))?;
    let kb: u64 = line.split_whitespace().nth(1)?.parse().ok()?;
    Some(kb * 1024)
}

/// macOS would need `task_info`; reporting nothing is honest, a guess is not.
#[cfg(not(any(windows, all(unix, not(target_os = "macos")))))]
fn resident_bytes() -> Option<u64> {
    None
}

#[cfg(test)]
mod tests {
    use super::is_newer;

    #[test]
    fn newer_versions_are_recognised() {
        assert!(is_newer("0.3.0", "0.2.0"));
        assert!(is_newer("0.2.1", "0.2.0"));
        assert!(is_newer("1.0.0", "0.9.9"));
        // A release candidate of the next minor still counts as news.
        assert!(is_newer("0.3.0-rc.1", "0.2.0"));
        assert!(!is_newer("0.2.0", "0.2.0"));
        assert!(!is_newer("0.1.2", "0.2.0"));
        // An unparsable tag must not be swallowed as "up to date".
        assert!(is_newer("nightly", "0.2.0"));
    }

    /// The one native bit of this module: a wrong struct layout makes the call
    /// fail (None) rather than return a plausible number, so a sane value here
    /// is real evidence the counters struct and `cb` are right.
    #[cfg(any(windows, all(unix, not(target_os = "macos"))))]
    #[test]
    fn resident_memory_is_readable() {
        let bytes = super::resident_bytes().expect("the platform query should answer");
        assert!(
            bytes > 1024 * 1024,
            "a live process holds more than a megabyte: {bytes}"
        );
        assert!(
            bytes < 64 * 1024 * 1024 * 1024,
            "and less than 64 GiB: {bytes}"
        );
    }
}
