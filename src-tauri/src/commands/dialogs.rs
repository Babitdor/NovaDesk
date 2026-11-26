//! Native file dialogs, in one place.
//!
//! The plugin's path type is normalised to a plain PathBuf here, so callers deal
//! with Option<PathBuf> and a dismissed dialog is simply None.

use std::path::PathBuf;

use tauri::AppHandle;
use tauri_plugin_dialog::DialogExt;

/// A folder picker. The callback receives the chosen path, or None when the
/// dialog was dismissed.
pub fn pick_folder(app: &AppHandle, done: impl FnOnce(Option<PathBuf>) + Send + 'static) {
    app.dialog().file().pick_folder(move |picked| {
        done(picked.and_then(|path| path.into_path().ok()));
    });
}

/// A file picker filtered to one extension, used by the theme importer.
pub fn pick_file(
    app: &AppHandle,
    label: &str,
    extension: &str,
    done: impl FnOnce(Option<PathBuf>) + Send + 'static,
) {
    app.dialog()
        .file()
        .add_filter(label, &[extension])
        .pick_file(move |picked| {
            done(picked.and_then(|path| path.into_path().ok()));
        });
}
