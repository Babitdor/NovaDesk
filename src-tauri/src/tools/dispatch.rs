//! One place that turns a tool name into an answer.
//!
//! The table in tools/mod.rs decides *who* answers; this module decides *how*,
//! for the native half. Keeping the two apart is what stops a tool from being
//! advertised and unimplemented: dispatch is exhaustive over the names it knows,
//! and anything else is the registry's missing-name error, not a panic.
//!
//! A CallContext is built per call, not per launch, so a tool cannot straddle a
//! workspace switch: the root it was given is the root it answered from.

use std::path::PathBuf;

use serde_json::{json, Value};
use tauri::{AppHandle, Emitter, Manager};

use crate::error::AppError;
use crate::tools::{
    browser, moodboards, projects, queue, search, skills, Handler, ToolsState, TOOLS,
};

/// The main window's label, used for both events and command routing.
pub const MAIN: &str = "main";

pub struct CallContext {
    pub app: Option<AppHandle>,
    /// The app's shared state, when a call comes from the running app. Tests
    /// build a context without it and exercise the handlers that need only
    /// paths.
    pub state: Option<crate::AppState>,
    pub workspace: Option<PathBuf>,
    pub data_dir: Option<PathBuf>,
}

impl CallContext {
    pub async fn in_app(app: AppHandle, state: crate::AppState) -> Self {
        let workspace = state
            .require_guard()
            .await
            .ok()
            .map(|guard| guard.root().to_path_buf());
        Self {
            data_dir: app.path().app_data_dir().ok(),
            app: Some(app),
            state: Some(state),
            workspace,
        }
    }

    fn app(&self) -> Result<&AppHandle, AppError> {
        self.app
            .as_ref()
            .ok_or_else(|| AppError::Tools("this tool needs the Nova window".into()))
    }
}

fn text(arguments: &Value, key: &str) -> Result<String, AppError> {
    arguments
        .get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .ok_or_else(|| AppError::Tools(format!("{key} is required")))
}

pub async fn call(
    context: &CallContext,
    tools: &ToolsState,
    name: &str,
    arguments: Value,
) -> Result<Value, AppError> {
    let Some(tool) = TOOLS.iter().find(|tool| tool.id == name) else {
        return Err(crate::tools::missing(name));
    };
    match tool.handler {
        Handler::Frontend => {
            let app = context.app()?;
            crate::tools::frontend::ask(app, tools, name, arguments).await
        }
        Handler::Native => native(context, tools, name, &arguments).await,
    }
}

async fn native(
    context: &CallContext,
    tools: &ToolsState,
    name: &str,
    arguments: &Value,
) -> Result<Value, AppError> {
    match name {
        "team_delegate" | "team_follow_up" | "team_message" | "team_status" => {
            let turn = arguments["_desktopTurnId"].as_str().ok_or_else(|| {
                AppError::Tools(
                    "Team tools require an authenticated active teammate session".into(),
                )
            })?;
            let scope = tools.scope_for_turn(turn).ok_or_else(|| {
                AppError::Tools("The teammate session is no longer active".into())
            })?;
            let run = scope
                .team_id
                .ok_or_else(|| AppError::Tools("No team run is active for this session".into()))?;
            require_state(context)?.teams.get(&run)?;
            if matches!(name, "team_delegate" | "team_follow_up")
                && scope.role.as_deref() != Some("nova")
            {
                return Err(AppError::Tools("Only Nova may assign team work".into()));
            }
            let mut arguments = arguments.clone();
            arguments["_runId"] = json!(run);
            arguments["_sender"] = json!(scope.role);
            arguments["_harness"] = json!(scope.harness);
            crate::tools::frontend::ask(context.app()?, tools, name, arguments).await
        }
        "list_projects" => Ok(projects::list(context.data_dir.as_deref())),
        "list_moodboards" => Ok(moodboards::list(context.workspace.as_deref())),
        "read_moodboard" => moodboards::read(context.workspace.as_deref(), &text(arguments, "id")?),
        "read_moodboard_image" => moodboards::image(
            context.workspace.as_deref(),
            &text(arguments, "id")?,
            &text(arguments, "image")?,
        ),
        "list_skills" => {
            // A switched-off skill is left out of the answer a harness reads,
            // while the pane still lists it so the switch can be found again.
            let disabled = tools.disabled_skills();
            let mut listed = skills::list(context.workspace.as_deref(), &disabled);
            if let Some(rows) = listed["skills"].as_array_mut() {
                rows.retain(|row| row["enabled"] != json!(false));
            }
            listed["count"] = json!(listed["skills"].as_array().map_or(0, Vec::len));
            Ok(listed)
        }
        "code_search" => search::search(context.workspace.as_deref(), arguments).await,
        "list_local_servers" => Ok(json!({
            "servers": crate::commands::browser::discover_local_servers().await?,
        })),
        "list_subagents" => {
            let runs = context
                .state
                .as_ref()
                .map(|state| state.live_runs())
                .unwrap_or_default();
            Ok(json!({ "running": runs, "count": runs.len() }))
        }
        "list_queue" => Ok(queue::list(tools, &text(arguments, "taskId")?)),
        "queue_message" => queue::push(
            tools,
            &text(arguments, "taskId")?,
            &text(arguments, "text")?,
        ),
        "remove_queued" => Ok(queue::remove(tools, &text(arguments, "id")?)),
        "steer_task" => {
            let state = require_state(context)?;
            queue::steer(
                state,
                &text(arguments, "taskId")?,
                &text(arguments, "text")?,
            )
            .await
        }
        "interrupt_task" => {
            let state = require_state(context)?;
            queue::interrupt(state, &text(arguments, "taskId")?)
        }
        "open_file" => {
            let path = text(arguments, "path")?;
            let workspace = context.workspace.as_ref().ok_or(AppError::NoWorkspace)?;
            let resolved = crate::workspace::WorkspaceGuard::new(workspace)?.resolve(&path)?;
            if !resolved.is_file() {
                return Err(AppError::Tools("open_file needs a file path".into()));
            }
            emit(context, "nova:open-file", json!({ "path": path }))?;
            Ok(json!({ "opened": path }))
        }
        "open_terminal" => {
            context.workspace.as_ref().ok_or(AppError::NoWorkspace)?;
            let shell = arguments
                .get("shell")
                .and_then(Value::as_str)
                .unwrap_or("powershell")
                .to_string();
            if !matches!(shell.as_str(), "powershell" | "cmd" | "gitbash") {
                return Err(AppError::Tools(format!("unknown shell: {shell}")));
            }
            emit(context, "nova:open-terminal", json!({ "shell": shell }))?;
            Ok(json!({ "shell": shell }))
        }
        "open_browser" => {
            let url = text(arguments, "url")?;
            let parsed = url
                .parse::<tauri::Url>()
                .ok()
                .filter(|url| matches!(url.scheme(), "http" | "https"))
                .ok_or_else(|| AppError::Tools("enter an http or https URL".into()))?;
            crate::tools::frontend::ask(
                context.app()?,
                tools,
                name,
                json!({ "url": parsed.as_str() }),
            )
            .await
        }
        "read_browser" => {
            let max = arguments
                .get("maxElements")
                .and_then(Value::as_u64)
                .unwrap_or(60);
            browser::read(context.app()?, max).await
        }
        "browser_action" => browser::action(context.app()?, &text(arguments, "action")?).await,
        "browser_click" => browser::click(context.app()?, &text(arguments, "ref")?).await,
        "browser_type" => {
            let reference = arguments.get("ref").and_then(Value::as_str);
            let contents = arguments["text"]
                .as_str()
                .ok_or_else(|| AppError::Tools("text must be a string".into()))?;
            browser::type_text(context.app()?, reference, contents).await
        }
        "capture_browser" => browser::capture(context.app()?).await,
        other => Err(AppError::Tools(format!(
            "{other} is in the catalogue but has no handler; that is a bug"
        ))),
    }
}

fn require_state(context: &CallContext) -> Result<&crate::AppState, AppError> {
    context
        .state
        .as_ref()
        .ok_or_else(|| AppError::Tools("this tool needs the running app".into()))
}

/// Emits one of the page's own events. A tool that has to move the UI does it
/// this way rather than calling a command: the page owns what is on screen, and
/// a second opinion about the selected task would be a second source of truth.
fn emit(context: &CallContext, event: &str, payload: Value) -> Result<(), AppError> {
    context
        .app()?
        .emit_to(tauri::EventTarget::webview(MAIN), event, payload)
        .map_err(|err| AppError::Tools(format!("could not reach the window: {err}")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testtmp::TempDir;

    fn context(workspace: &TempDir, data: &TempDir) -> CallContext {
        CallContext {
            app: None,
            state: None,
            workspace: Some(workspace.path().to_path_buf()),
            data_dir: Some(data.path().to_path_buf()),
        }
    }

    /// Every native name in the catalogue must reach a handler. This is the
    /// test that would have caught "advertised but unimplemented", which is the
    /// one failure mode the shared table is supposed to make impossible.
    #[tokio::test]
    async fn every_native_tool_has_a_handler() {
        let workspace = TempDir::new("dispatch-ws");
        let data = TempDir::new("dispatch-data");
        let context = context(&workspace, &data);
        let tools = ToolsState::default();
        for tool in TOOLS.iter().filter(|tool| tool.handler == Handler::Native) {
            let outcome = call(&context, &tools, tool.id, json!({})).await;
            match outcome {
                Ok(_) => {}
                // A missing argument or a missing window is a handler that ran;
                // "no handler" is the bug this test is for.
                Err(AppError::Tools(message)) => assert!(
                    !message.contains("has no handler"),
                    "{} has no handler",
                    tool.id
                ),
                Err(_) => {}
            }
        }
    }

    #[tokio::test]
    async fn an_unknown_name_is_an_error_not_a_panic() {
        let workspace = TempDir::new("dispatch-unknown");
        let data = TempDir::new("dispatch-unknown-data");
        let error = call(
            &context(&workspace, &data),
            &ToolsState::default(),
            "make_me_a_sandwich",
            json!({}),
        )
        .await
        .unwrap_err();
        assert!(matches!(error, AppError::Tools(_)), "got {error:?}");
    }
}
