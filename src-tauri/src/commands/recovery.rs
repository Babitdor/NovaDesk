//! Crash-policy and crash-report commands.

use tauri::{AppHandle, Manager, Wry};

use crate::error::AppError;
use crate::recovery::{self, CrashReport, RecoveryPolicy};

/// Writes the policy where the panic hook reads it, before the next crash rather
/// than after one.
#[tauri::command]
pub async fn set_crash_policy(
    restart_after_crash: bool,
    crash_reports: bool,
    app: AppHandle<Wry>,
) -> Result<(), AppError> {
    let path = recovery::policy_path(&app)
        .ok_or_else(|| AppError::Io("the app data folder is unavailable".into()))?;
    recovery::write_policy_at(
        &path,
        RecoveryPolicy {
            restart_after_crash,
            crash_reports,
        },
    )
    .map_err(|err| AppError::Io(format!("could not save the crash policy: {err}")))
}

#[tauri::command]
pub async fn crash_reports(app: AppHandle<Wry>) -> Result<Vec<CrashReport>, AppError> {
    let dir = app
        .path()
        .app_data_dir()
        .map_err(|err| AppError::Io(format!("the app data folder is unavailable: {err}")))?;
    Ok(recovery::reports_at(&dir.join("crashes")))
}

#[tauri::command]
pub async fn clear_crash_reports(app: AppHandle<Wry>) -> Result<(), AppError> {
    let dir = app
        .path()
        .app_data_dir()
        .map_err(|err| AppError::Io(format!("the app data folder is unavailable: {err}")))?;
    recovery::clear_reports_at(&dir.join("crashes"))
        .map_err(|err| AppError::Io(format!("could not clear the crash reports: {err}")))
}

/// A page error, reported from the webview. The source is a closed set because it
/// is written into the report: anything else is recorded as a page error rather
/// than trusted as a label.
#[tauri::command]
pub async fn report_crash(
    message: String,
    location: String,
    source: String,
    app: AppHandle<Wry>,
) -> Result<(), AppError> {
    if message.trim().is_empty() {
        return Ok(());
    }
    let source = if source == "rust" { "rust" } else { "webview" };
    recovery::record(&app, source, &message, &location, "")
        .map(|_| ())
        .map_err(|err| AppError::Io(format!("could not write the crash report: {err}")))
}
