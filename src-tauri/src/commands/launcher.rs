//! The quick launcher and the running-agents widget.
//!
//! Both are ordinary webview windows created at runtime, and both are outside the
//! command surface on purpose: the invoke guard in lib.rs only admits the main
//! window, so these two can read events and move themselves and nothing else.

use std::sync::Mutex;

use tauri::{AppHandle, Manager, WebviewUrl, WebviewWindow, WebviewWindowBuilder, Wry};
use tauri_plugin_global_shortcut::{GlobalShortcutExt, ShortcutState};

use crate::error::AppError;

/// One shell mutation at a time.
///
/// The pane and the preference effect can both ask for the same shortcut, and the
/// plugin's own error for the loser of that race is "HotKey already registered" —
/// which reads to a user as "another app owns your chord". Serialising the
/// unregister/register pair makes the second call a replacement instead.
static SHELL: Mutex<()> = Mutex::new(());

fn lock_shell() -> std::sync::MutexGuard<'static, ()> {
    SHELL
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

pub const LAUNCHER: &str = "launcher";
pub const WIDGET: &str = "widget";

/// One line of prompt and a Start button.
const LAUNCHER_WIDTH: f64 = 560.0;
const LAUNCHER_HEIGHT: f64 = 96.0;
const WIDGET_WIDTH: f64 = 320.0;
const WIDGET_HEIGHT: f64 = 220.0;

fn create_launcher(app: &AppHandle<Wry>) -> Result<WebviewWindow<Wry>, AppError> {
    WebviewWindowBuilder::new(app, LAUNCHER, WebviewUrl::App("index.html".into()))
        .title("Nova quick launcher")
        .inner_size(LAUNCHER_WIDTH, LAUNCHER_HEIGHT)
        .resizable(false)
        .decorations(false)
        .always_on_top(true)
        .skip_taskbar(true)
        .visible(false)
        .center()
        .build()
        .map_err(|err| AppError::Agent(format!("could not create the launcher window: {err}")))
}

/// Shows the launcher, creating it the first time. Focus is explicit: the window
/// was raised over another application, so the chord's whole promise is that the
/// cursor is already in the prompt.
fn show_launcher(app: &AppHandle<Wry>) -> Result<(), AppError> {
    let _shell = lock_shell();
    show_launcher_inner(app)
}

/// The unlocked half.
///
/// The shortcut handler runs on the main thread and must never wait for the shell
/// lock: the command holding it is itself blocked on a main-thread round trip.
fn show_launcher_inner(app: &AppHandle<Wry>) -> Result<(), AppError> {
    let window = match app.get_webview_window(LAUNCHER) {
        Some(window) => window,
        None => create_launcher(app)?,
    };
    window
        .show()
        .map_err(|err| AppError::Agent(format!("could not show the launcher: {err}")))?;
    let _ = window.set_focus();
    Ok(())
}

fn modifier_present(chord: &str) -> bool {
    let lower = chord.to_ascii_lowercase();
    [
        "ctrl",
        "control",
        "commandorcontrol",
        "cmdorcontrol",
        "super",
        "alt",
        "option",
        "shift",
    ]
    .iter()
    .any(|name| lower.contains(name))
}

/// Registers, replaces or releases the global chord.
///
/// An empty accelerator with the launcher on is refused rather than registered:
/// a launcher with no chord could only be reached from the button, which is not
/// what the toggle says.
#[tauri::command]
pub async fn set_quick_launcher(
    enabled: bool,
    accelerator: String,
    app: AppHandle<Wry>,
) -> Result<(), AppError> {
    let _shell = lock_shell();
    let shortcuts = app.global_shortcut();
    shortcuts
        .unregister_all()
        .map_err(|err| AppError::Agent(format!("could not release the old shortcut: {err}")))?;
    if !enabled {
        // The chord is what the toggle owns. A launcher left on screen after the
        // shortcut is released looks like the toggle did nothing.
        if let Some(window) = app.get_webview_window(LAUNCHER) {
            let _ = window.hide();
        }
        return Ok(());
    }
    let chord = accelerator.trim().to_string();
    if chord.is_empty() {
        return Err(AppError::Agent(
            "Record a shortcut before turning the launcher on".into(),
        ));
    }
    if !modifier_present(&chord) {
        return Err(AppError::Agent(
            "A global shortcut needs Ctrl, Alt, Shift or Super, or it would capture that key everywhere".into(),
        ));
    }
    shortcuts
        .on_shortcut(chord.as_str(), |app, _shortcut, event| {
            if event.state == ShortcutState::Pressed {
                // A failed show is not worth a notice: the user sees nothing happen
                // and can press the chord again.
                let _ = show_launcher_inner(app);
            }
        })
        .map_err(|err| {
            AppError::Agent(format!(
                "The system refused that shortcut ({err}). Another application may already own it."
            ))
        })?;
    Ok(())
}

#[tauri::command]
pub async fn open_launcher(app: AppHandle<Wry>) -> Result<(), AppError> {
    show_launcher(&app)
}

fn create_widget(app: &AppHandle<Wry>) -> Result<WebviewWindow<Wry>, AppError> {
    let mut builder = WebviewWindowBuilder::new(app, WIDGET, WebviewUrl::App("index.html".into()))
        .title("Nova agents")
        .inner_size(WIDGET_WIDTH, WIDGET_HEIGHT)
        .resizable(false)
        .decorations(false)
        .skip_taskbar(true)
        .visible(false);
    // The bottom-right corner is where a companion window is expected, and it is
    // clear of the composer; without a monitor report the builder centres it.
    match app.primary_monitor().ok().flatten() {
        Some(monitor) => {
            let scale = monitor.scale_factor();
            let size = monitor.size().to_logical::<f64>(scale);
            let position = monitor.position().to_logical::<f64>(scale);
            builder = builder.position(
                position.x + size.width - WIDGET_WIDTH - 24.0,
                position.y + size.height - WIDGET_HEIGHT - 72.0,
            );
        }
        None => {
            builder = builder.center();
        }
    }
    builder
        .build()
        .map_err(|err| AppError::Agent(format!("could not create the widget window: {err}")))
}

/// Creates, pins and shows the widget — or closes it when the toggle goes off.
/// Pinning writes through to the window, so unchecking it takes effect without a
/// restart.
#[tauri::command]
pub async fn set_widget(enabled: bool, pinned: bool, app: AppHandle<Wry>) -> Result<(), AppError> {
    let _shell = lock_shell();
    if !enabled {
        if let Some(window) = app.get_webview_window(WIDGET) {
            let _ = window.close();
        }
        return Ok(());
    }
    let window = match app.get_webview_window(WIDGET) {
        Some(window) => window,
        None => create_widget(&app)?,
    };
    window
        .set_always_on_top(pinned)
        .map_err(|err| AppError::Agent(format!("could not pin the widget: {err}")))?;
    window
        .show()
        .map_err(|err| AppError::Agent(format!("could not show the widget: {err}")))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_chord_without_a_modifier_is_refused() {
        assert!(!modifier_present("K"));
        assert!(!modifier_present("F5"));
        assert!(modifier_present("CommandOrControl+Shift+Space"));
        assert!(modifier_present("ctrl+k"));
    }
}
