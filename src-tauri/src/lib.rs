mod agent;
mod commands;
mod error;
mod events;
mod proc;
#[cfg(test)]
mod testtmp;
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
    ];
    let app = tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .manage(AppState::new())
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
            // A closing window must not leave child processes or a child webview
            // behind.
            if matches!(event, tauri::WindowEvent::Destroyed) {
                window.app_handle().state::<AppState>().cancel_all();
                commands::browser::close_child(window.app_handle());
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
