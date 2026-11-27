//! Native file dialogs, in one place.
//!
//! The plugin's path type is normalised to a plain PathBuf here, so callers deal
//! with Option<PathBuf> and a dismissed dialog is simply None.
//!
//! The real pickers are compiled out of unit-test builds: rfd's dialogs import
//! `TaskDialogIndirect`, which only resolves under a ComCtl32 v6 manifest, and
//! the lib's test binary cannot be given one from build.rs (cargo has no
//! per-target link args for it — see build.rs and the README's Checks section).
//! No test opens a native dialog, so a stub that reports "dismissed" keeps the
//! test link free of the import instead of killing the loader.

#[cfg(not(test))]
use std::path::PathBuf;

#[cfg(not(test))]
use tauri::AppHandle;

/// A folder picker. The callback receives the chosen path, or None when the
/// dialog was dismissed.
#[cfg(not(test))]
pub fn pick_folder(app: &AppHandle, done: impl FnOnce(Option<PathBuf>) + Send + 'static) {
    use tauri_plugin_dialog::DialogExt as _;

    app.dialog().file().pick_folder(move |picked| {
        done(picked.and_then(|path| path.into_path().ok()));
    });
}

/// A file picker filtered to one extension, used by the theme importer.
#[cfg(not(test))]
pub fn pick_file(
    app: &AppHandle,
    label: &str,
    extension: &str,
    done: impl FnOnce(Option<PathBuf>) + Send + 'static,
) {
    use tauri_plugin_dialog::DialogExt as _;

    app.dialog()
        .file()
        .add_filter(label, &[extension])
        .pick_file(move |picked| {
            done(picked.and_then(|path| path.into_path().ok()));
        });
}

#[cfg(test)]
pub use test_stubs::{pick_file, pick_folder};

/// Unit-test build: no native dialog is ever opened, so both pickers report a
/// dismissal immediately and tauri-plugin-dialog stays out of the test link.
#[cfg(test)]
mod test_stubs {
    use std::path::PathBuf;

    use tauri::AppHandle;

    pub fn pick_folder(_app: &AppHandle, done: impl FnOnce(Option<PathBuf>) + Send + 'static) {
        done(None)
    }

    pub fn pick_file(
        _app: &AppHandle,
        _label: &str,
        _extension: &str,
        done: impl FnOnce(Option<PathBuf>) + Send + 'static,
    ) {
        done(None)
    }
}
