mod agent;
mod commands;
mod error;
mod events;
mod http;
mod proc;
mod recovery;
pub mod speak;
#[cfg(test)]
mod testtmp;
pub mod transcribe;
pub mod voice;
mod workspace;

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use tauri::Manager;
use tokio::sync::RwLock;
use tokio_util::sync::CancellationToken;

use crate::agent::cli::{CliAgent, AGENTS};
use crate::agent::local::LocalAgent;
use crate::agent::nova::NovaAgent;
use crate::agent::{AgentBackend, AgentRegistry, DEFAULT_BACKEND};
use crate::error::AppError;
use crate::workspace::WorkspaceGuard;

/// Everything the commands share.
///
/// `Clone` is cheap — every field sits behind an Arc — so a spawned task can hold
/// its own handle and `State<'_, AppState>` stays a borrow.
/// A live turn: the token that stops it and the backend driving it.
type LiveTurns = Arc<Mutex<HashMap<String, (CancellationToken, Arc<dyn AgentBackend>)>>>;

#[derive(Clone)]
pub struct AppState {
    guard: Arc<RwLock<Option<WorkspaceGuard>>>,
    /// Live turns by id. Cancelling the token makes the backend stop and its child
    /// processes die.
    turns: LiveTurns,
    /// Live one-off commands from the output panel, same contract as `turns`.
    procs: Arc<Mutex<HashMap<String, CancellationToken>>>,
    terminals: Arc<Mutex<HashMap<String, commands::terminal::Terminal>>>,
    agents: AgentRegistry,
    /// The resident speech model, if one is loaded. One per process: loading is
    /// hundreds of megabytes and the engine drops idle weights itself.
    voice: Arc<voice::engine::VoiceEngine>,
    /// Live model downloads, so Cancel can reach a transfer already streaming.
    voice_downloads: Arc<Mutex<HashMap<String, CancellationToken>>>,
    /// Reads replies aloud. No resident model — Piper is a process per utterance
    /// — so this holds the lock that keeps two replies from overlapping, and
    /// nothing else.
    speak: Arc<speak::engine::SpeakEngine>,
    /// Live voice downloads, keyed by voice id.
    speak_downloads: Arc<Mutex<HashMap<String, CancellationToken>>>,
}

impl AppState {
    pub fn new() -> Self {
        let mut agents = AgentRegistry::new();
        // Nova is the real backend; `local` stays registered as the offline stub
        // that the tests and a no-install demo use.
        agents.register(
            DEFAULT_BACKEND,
            Arc::new(NovaAgent::default()) as Arc<dyn AgentBackend>,
        );
        agents.register("local", Arc::new(LocalAgent) as Arc<dyn AgentBackend>);
        for &(id, _) in AGENTS.iter().filter(|(id, _)| *id != DEFAULT_BACKEND) {
            agents.register(id, Arc::new(CliAgent(id)));
        }

        Self {
            guard: Arc::new(RwLock::new(None)),
            turns: Arc::new(Mutex::new(HashMap::new())),
            procs: Arc::new(Mutex::new(HashMap::new())),
            terminals: Arc::new(Mutex::new(HashMap::new())),
            agents,
            voice: Arc::new(voice::engine::VoiceEngine::default()),
            voice_downloads: Arc::new(Mutex::new(HashMap::new())),
            speak: Arc::new(speak::engine::SpeakEngine::default()),
            speak_downloads: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    pub async fn require_guard(&self) -> Result<WorkspaceGuard, AppError> {
        self.guard.read().await.clone().ok_or(AppError::NoWorkspace)
    }

    /// Stops every turn and child process. Called when the window closes and when
    /// the app exits, so closing Nova never orphans a running build.
    pub fn cancel_all(&self) {
        for (token, _) in self.turns.lock().expect("turns mutex poisoned").values() {
            token.cancel();
        }
        for token in self.procs.lock().expect("procs mutex poisoned").values() {
            token.cancel();
        }
        for terminal in self
            .terminals
            .lock()
            .expect("terminals mutex poisoned")
            .values()
        {
            terminal.cancel.cancel();
        }
        for token in self
            .voice_downloads
            .lock()
            .expect("voice downloads mutex poisoned")
            .values()
        {
            token.cancel();
        }
        for token in self
            .speak_downloads
            .lock()
            .expect("speech downloads mutex poisoned")
            .values()
        {
            token.cancel();
        }
    }
}

/// When the process started, for the About page's uptime row. A static rather
/// than a field because uptime belongs to the process, not to one state clone.
static STARTED: std::sync::OnceLock<std::time::Instant> = std::sync::OnceLock::new();

/// Read-only counters for the About page, so its snapshot cannot reach the maps
/// themselves.
impl AppState {
    /// Turns holding a live cancel token, i.e. agents working right now.
    pub fn live_turns(&self) -> usize {
        self.turns.lock().expect("turns mutex poisoned").len()
    }

    /// Terminals the dock still owns, open or backgrounded.
    pub fn open_terminals(&self) -> usize {
        self.terminals
            .lock()
            .expect("terminals mutex poisoned")
            .len()
    }

    pub fn uptime(&self) -> std::time::Duration {
        STARTED.get_or_init(std::time::Instant::now).elapsed()
    }
}

impl Default for AppState {
    fn default() -> Self {
        Self::new()
    }
}

pub fn run() {
    let main_commands: fn(tauri::ipc::Invoke<tauri::Wry>) -> bool = tauri::generate_handler![
        commands::workspace::pick_workspace,
        commands::workspace::set_workspace,
        commands::workspace::list_dir,
        commands::workspace::read_file,
        commands::agent::start_turn,
        commands::agent::cancel_turn,
        commands::agent::respond_approval,
        commands::agent::agent_status,
        commands::agent::prewarm_nova,
        agent::models::agent_models,
        commands::process::run_command,
        commands::process::kill_process,
        commands::terminal::start_terminal,
        commands::terminal::terminal_input,
        commands::terminal::terminal_resize,
        commands::terminal::close_terminal,
        commands::browser::browser_open,
        commands::browser::browser_local_servers,
        commands::browser::browser_bounds,
        commands::browser::browser_visibility,
        commands::browser::browser_reload,
        commands::browser::browser_history,
        commands::browser::browser_close,
        commands::settings::import_theme,
        commands::settings::install_theme,
        commands::settings::nova_mcp_list,
        commands::settings::add_mcp_server,
        commands::settings::open_provider_setup,
        commands::settings::open_nova_terminal,
        commands::settings::markdownify,
        commands::settings::open_link,
        commands::app::app_report,
        commands::app::check_app_update,
        commands::github::github_account,
        commands::github::github_repos,
        commands::github::github_activity,
        commands::github::github_set_token,
        commands::github::github_clone,
        commands::github::clone_parent_pick,
        commands::voice::voice_catalog,
        commands::voice::voice_installed,
        commands::voice::voice_download,
        commands::voice::voice_cancel,
        commands::voice::voice_delete,
        commands::voice::voice_transcribe,
        commands::transcribe::transcribe_providers,
        commands::transcribe::voice_transcribe_service,
        commands::voice::voice_engine_ready,
        commands::speak::speak_catalog,
        commands::speak::speak_installed,
        commands::speak::speak_runtime_ready,
        commands::speak::speak_download,
        commands::speak::speak_cancel,
        commands::speak::speak_delete,
        commands::speak::speak_synthesize,
        commands::launcher::set_quick_launcher,
        commands::launcher::open_launcher,
        commands::launcher::set_widget,
        commands::recovery::set_crash_policy,
        commands::recovery::crash_reports,
        commands::recovery::clear_crash_reports,
        commands::recovery::report_crash,
    ];
    let app = tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        // Registered empty: the chord itself comes from the saved preference, and
        // a plugin with no shortcuts costs nothing until one is set.
        .plugin(tauri_plugin_global_shortcut::Builder::new().build())
        .manage(AppState::new())
        .setup(|app| {
            // Installed before any window exists: a panic during boot is exactly
            // the crash the policy is for.
            recovery::install_panic_hook(app.handle().clone());
            Ok(())
        })
        .invoke_handler(move |invoke| {
            if invoke.message.webview().label() != "main" {
                invoke
                    .resolver
                    .reject("Desktop commands are available only in the main view");
                return true;
            }
            main_commands(invoke)
        })
        .on_window_event(|window, event| {
            // The main window owns the work: only its closing cancels transfers
            // and cuts child processes loose. Dismissing the widget is not a
            // reason to stop a build.
            if window.label() != "main" {
                return;
            }
            if matches!(event, tauri::WindowEvent::Destroyed) {
                window.app_handle().state::<AppState>().cancel_all();
                commands::browser::close_child(window.app_handle());
            }
            // Nova's companion windows are part of Nova: closing it closes them.
            // Otherwise the widget outlives the app it reports on, because the
            // process stays alive as long as one webview is still standing.
            if matches!(
                event,
                tauri::WindowEvent::CloseRequested { .. } | tauri::WindowEvent::Destroyed
            ) {
                for label in [commands::launcher::WIDGET, commands::launcher::LAUNCHER] {
                    if let Some(aux) = window.app_handle().get_webview_window(label) {
                        let _ = aux.close();
                    }
                }
            }
        })
        .build(tauri::generate_context!())
        .expect("failed to build Nova Desktop");

    app.run(|app_handle, event| {
        if let tauri::RunEvent::Exit = event {
            app_handle.state::<AppState>().cancel_all();
        }
    });
}
