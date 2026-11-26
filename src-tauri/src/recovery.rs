//! Crash policy, crash reports and the restart budget.
//!
//! The policy lives in a JSON file rather than in AppState because the panic
//! hook has to read it *after* the process is already unhealthy: a mutex that a
//! panicking thread poisoned is not the place to keep the answer to "should this
//! restart?".
//!
//! What this covers: a panic in Nova's own Rust code and errors the page reports.
//! What it cannot cover, and says so in the UI: an OS kill, power loss or a
//! native fault below the Rust boundary.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager, Wry};

const POLICY: &str = "recovery.json";
const RESTARTS: &str = "restarts.json";
const CRASHES: &str = "crashes";

/// Three restarts in fifteen minutes, then stop: a crash loop must not turn into
/// a fork bomb on the user's desktop.
const RESTART_LIMIT: usize = 3;
const RESTART_WINDOW_MS: u64 = 15 * 60 * 1000;
const REPORT_LIMIT: usize = 20;
const MESSAGE_LIMIT: usize = 4000;
const BACKTRACE_LIMIT: usize = 20_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RecoveryPolicy {
    pub restart_after_crash: bool,
    pub crash_reports: bool,
}

impl Default for RecoveryPolicy {
    fn default() -> Self {
        // Reporting on, restarting off: a report costs nothing, and a restart the
        // user did not ask for is a surprise.
        Self {
            restart_after_crash: false,
            crash_reports: true,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct CrashReport {
    pub path: String,
    pub at: u64,
    pub source: String,
    pub message: String,
    pub location: String,
    pub backtrace: String,
}

pub fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_millis() as u64)
        .unwrap_or(0)
}

fn data_dir(app: &AppHandle<Wry>) -> Option<PathBuf> {
    app.path().app_data_dir().ok()
}

pub fn policy_path(app: &AppHandle<Wry>) -> Option<PathBuf> {
    data_dir(app).map(|dir| dir.join(POLICY))
}

/// Absent or unreadable means the defaults: a missing file is a first run, and a
/// corrupt one is not a reason to refuse to start.
pub fn read_policy_at(path: &Path) -> RecoveryPolicy {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_default()
}

pub fn write_policy_at(path: &Path, policy: RecoveryPolicy) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let bytes = serde_json::to_vec_pretty(&policy).map_err(std::io::Error::other)?;
    write_atomic(path, &bytes)
}

/// Write-then-rename: a crash in the middle of a save must not leave a truncated
/// file that turns every future launch into a defaults run.
fn write_atomic(path: &Path, contents: &[u8]) -> std::io::Result<()> {
    let temporary = path.with_extension("tmp");
    {
        let mut file = std::fs::File::create(&temporary)?;
        file.write_all(contents)?;
        file.sync_all()?;
    }
    std::fs::rename(&temporary, path)
}

fn crashes_dir(app: &AppHandle<Wry>) -> Option<PathBuf> {
    data_dir(app).map(|dir| dir.join(CRASHES))
}

/// Records one report. The file name carries the timestamp and a suffix, because
/// two errors can land inside the same millisecond and one must not overwrite the
/// other.
pub fn write_report_at(dir: &Path, report: &CrashReport) -> std::io::Result<PathBuf> {
    std::fs::create_dir_all(dir)?;
    let bytes = serde_json::to_vec_pretty(report).map_err(std::io::Error::other)?;
    let mut path = dir.join(format!("crash-{}.json", report.at));
    for suffix in 1..100 {
        if !path.exists() {
            break;
        }
        path = dir.join(format!("crash-{}-{}.json", report.at, suffix));
    }
    write_atomic(&path, &bytes)?;
    Ok(path)
}

pub fn clip(text: &str, limit: usize) -> String {
    if text.chars().count() <= limit {
        return text.to_string();
    }
    text.chars().take(limit).collect::<String>() + "…"
}

/// Writes a report for something the page caught. A missing crash folder or a
/// policy with reporting off both mean "nothing written", which is not an error.
pub fn record(
    app: &AppHandle<Wry>,
    source: &str,
    message: &str,
    location: &str,
    backtrace: &str,
) -> Result<Option<CrashReport>, std::io::Error> {
    let policy = policy_path(app)
        .map(|path| read_policy_at(&path))
        .unwrap_or_default();
    if !policy.crash_reports {
        return Ok(None);
    }
    let Some(dir) = crashes_dir(app) else {
        return Ok(None);
    };
    let report = CrashReport {
        path: String::new(),
        at: now_ms(),
        source: clip(source.trim(), 32),
        message: clip(message.trim(), MESSAGE_LIMIT),
        location: clip(location.trim(), 400),
        backtrace: clip(backtrace, BACKTRACE_LIMIT),
    };
    let path = write_report_at(&dir, &report)?;
    Ok(Some(CrashReport {
        path: path.display().to_string(),
        ..report
    }))
}

/// Newest first. A file that no longer parses is skipped rather than failing the
/// whole list: one bad report must not hide the other nineteen.
pub fn reports_at(dir: &Path) -> Vec<CrashReport> {
    let mut reports: Vec<CrashReport> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter(|entry| entry.path().extension().is_some_and(|ext| ext == "json"))
        .filter_map(|entry| {
            let path = entry.path();
            let text = std::fs::read_to_string(&path).ok()?;
            let mut report: CrashReport = serde_json::from_str(&text).ok()?;
            report.path = path.display().to_string();
            Some(report)
        })
        .collect();
    reports.sort_by_key(|report| std::cmp::Reverse(report.at));
    reports.truncate(REPORT_LIMIT);
    reports
}

pub fn clear_reports_at(dir: &Path) -> std::io::Result<()> {
    if !dir.exists() {
        return Ok(());
    }
    for entry in std::fs::read_dir(dir)?.flatten() {
        let path = entry.path();
        if path.extension().is_some_and(|ext| ext == "json") {
            std::fs::remove_file(path)?;
        }
    }
    Ok(())
}

/// The restart timestamps still inside the window, after dropping the old ones.
fn recent_restarts_at(path: &Path) -> Vec<u64> {
    let now = now_ms();
    let mut stamps: Vec<u64> = std::fs::read_to_string(path)
        .ok()
        .and_then(|text| serde_json::from_str::<Vec<u64>>(&text).ok())
        .unwrap_or_default();
    stamps.retain(|stamp| now.saturating_sub(*stamp) <= RESTART_WINDOW_MS);
    stamps
}

pub fn may_restart_at(path: &Path) -> bool {
    recent_restarts_at(path).len() < RESTART_LIMIT
}

pub fn note_restart_at(path: &Path) {
    let mut stamps = recent_restarts_at(path);
    stamps.push(now_ms());
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if let Ok(bytes) = serde_json::to_vec(&stamps) {
        let _ = write_atomic(path, &bytes);
    }
}

/// Installs the panic hook. The previous hook still runs — a user's own hook, or
/// the runtime's — so this only adds a report and a possible restart.
///
/// Everything here is synchronous and takes no shared lock a panicking thread
/// could be holding: file I/O, the policy file and a process spawn.
pub fn install_panic_hook(app: AppHandle<Wry>) {
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        previous(info);
        let message = if let Some(text) = info.payload().downcast_ref::<&str>() {
            (*text).to_string()
        } else if let Some(text) = info.payload().downcast_ref::<String>() {
            text.clone()
        } else {
            "panic with a payload that is not a string".to_string()
        };
        let location = info
            .location()
            .map(|place| format!("{}:{}:{}", place.file(), place.line(), place.column()))
            .unwrap_or_default();
        let backtrace = std::backtrace::Backtrace::force_capture().to_string();
        let _ = record(&app, "rust", &message, &location, &backtrace);

        let policy = policy_path(&app)
            .map(|path| read_policy_at(&path))
            .unwrap_or_default();
        let Some(budget) = data_dir(&app).map(|dir| dir.join(RESTARTS)) else {
            return;
        };
        if !policy.restart_after_crash || !may_restart_at(&budget) {
            return;
        }
        note_restart_at(&budget);
        let Ok(executable) = std::env::current_exe() else {
            return;
        };
        std::thread::spawn(move || {
            // A short delay lets the report reach the disk before the new process
            // starts reading the same folder.
            std::thread::sleep(std::time::Duration::from_millis(400));
            let _ = std::process::Command::new(executable)
                .args(std::env::args_os().skip(1))
                .spawn();
        });
    }));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testtmp::TempDir;

    #[test]
    fn a_missing_policy_is_the_default() {
        let dir = TempDir::new("recovery-policy");
        let path = dir.path().join(POLICY);
        assert_eq!(read_policy_at(&path), RecoveryPolicy::default());
        assert!(!read_policy_at(&path).restart_after_crash);
    }

    #[test]
    fn a_policy_round_trips_through_disk() {
        let dir = TempDir::new("recovery-round-trip");
        let path = dir.path().join(POLICY);
        write_policy_at(
            &path,
            RecoveryPolicy {
                restart_after_crash: true,
                crash_reports: false,
            },
        )
        .expect("policy writes");
        assert_eq!(
            read_policy_at(&path),
            RecoveryPolicy {
                restart_after_crash: true,
                crash_reports: false
            }
        );
    }

    #[test]
    fn reports_are_listed_newest_first_and_clearable() {
        let dir = TempDir::new("recovery-reports");
        let folder = dir.path().join(CRASHES);
        let older = CrashReport {
            path: String::new(),
            at: 1_000,
            source: "rust".into(),
            message: "older".into(),
            location: String::new(),
            backtrace: String::new(),
        };
        let newer = CrashReport {
            at: 2_000,
            message: "newer".into(),
            ..older.clone()
        };
        write_report_at(&folder, &older).expect("older report");
        write_report_at(&folder, &newer).expect("newer report");
        // Two reports in the same millisecond still get separate files.
        write_report_at(&folder, &older).expect("second older report");
        let listed = reports_at(&folder);
        assert_eq!(listed.len(), 3);
        assert_eq!(listed[0].message, "newer");
        assert!(listed[1].path != listed[2].path);
        assert!(listed[0].path.ends_with(".json"));
        clear_reports_at(&folder).expect("clears");
        assert!(reports_at(&folder).is_empty());
    }

    #[test]
    fn the_restart_budget_stops_after_three() {
        let dir = TempDir::new("recovery-budget");
        let path = dir.path().join(RESTARTS);
        assert!(may_restart_at(&path));
        for _ in 0..RESTART_LIMIT {
            note_restart_at(&path);
        }
        assert!(!may_restart_at(&path));
    }

    #[test]
    fn long_text_is_clipped_on_a_character_boundary() {
        let clipped = clip(&"é".repeat(10), 3);
        assert_eq!(clipped.chars().count(), 4);
        assert!(clipped.ends_with('…'));
    }
}
