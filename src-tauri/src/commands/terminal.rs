use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::json;
use tauri::{ipc::Channel, State};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::process::ChildStdin;
use tokio::sync::Mutex;
use tokio_util::sync::CancellationToken;

use crate::{agent::cli, error::AppError, AppState};

#[derive(Clone)]
pub struct Terminal {
    stdin: Arc<Mutex<ChildStdin>>,
    pub cancel: CancellationToken,
}

#[derive(Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum TerminalEvent {
    Screen {
        lines: Vec<String>,
        row: u32,
        column: u32,
    },
    Error {
        message: String,
    },
    Exit,
}

#[derive(Default, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TerminalShell {
    #[default]
    PowerShell,
    Cmd,
    GitBash,
}

impl TerminalShell {
    fn name(&self) -> &'static str {
        match self {
            Self::PowerShell => "powershell",
            Self::Cmd => "cmd",
            Self::GitBash => "gitbash",
        }
    }
}

#[tauri::command]
pub async fn start_terminal(
    id: String,
    shell: Option<TerminalShell>,
    rows: u32,
    cols: u32,
    on_event: Channel<TerminalEvent>,
    state: State<'_, AppState>,
) -> Result<(), AppError> {
    if uuid::Uuid::parse_str(&id).is_err() {
        return Err(AppError::Agent("Invalid terminal ID".into()));
    }
    let workspace = state.require_guard().await?;
    let (rows, cols) = size(rows, cols);
    let mut terminals = state.terminals.lock().expect("terminals mutex poisoned");
    if terminals.len() >= 8 || terminals.contains_key(&id) {
        return Err(AppError::Agent(
            "Close a terminal before opening another one".into(),
        ));
    }
    let rows = rows.to_string();
    let cols = cols.to_string();
    let mut command = cli::terminal_python()?;
    command.args([
        "-I",
        "-u",
        "-c",
        include_str!("terminal.py"),
        &rows,
        &cols,
        shell.unwrap_or_default().name(),
    ]);
    command
        .current_dir(workspace.root())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    // The helper drives a ConPTY whose picture comes back over stdout; its own
    // console window would only flash on screen.
    #[cfg(windows)]
    command.creation_flags(0x08000000); // CREATE_NO_WINDOW
    let mut child = command
        .spawn()
        .map_err(|err| AppError::Spawn(err.to_string()))?;
    let mut stderr = child.stderr.take().expect("piped stderr");
    let mut stdout = BufReader::new(child.stdout.take().expect("piped stdout"));
    let terminal = Terminal {
        stdin: Arc::new(Mutex::new(child.stdin.take().expect("piped stdin"))),
        cancel: CancellationToken::new(),
    };
    terminals.insert(id.clone(), terminal.clone());
    drop(terminals);
    let state = state.inner().clone();
    tauri::async_runtime::spawn(async move {
        let diagnostics = tokio::spawn(async move {
            let mut text = Vec::new();
            let _ = (&mut stderr).take(8192).read_to_end(&mut text).await;
            String::from_utf8_lossy(&text).into_owned()
        });
        let result = tokio::select! {
            _ = terminal.cancel.cancelled() => None,
            result = async {
                loop {
                    let mut line = Vec::new();
                    if (&mut stdout).take(2 * 1024 * 1024).read_until(b'\n', &mut line).await? == 0 {
                        break;
                    }
                    match serde_json::from_slice(&line) {
                        Ok(TerminalEvent::Exit) => {
                            let _ = send(&terminal, json!({"type": "close"})).await;
                            break;
                        }
                        Ok(event) => { let _ = on_event.send(event); }
                        Err(_) => return Err(std::io::Error::new(std::io::ErrorKind::InvalidData, "Invalid terminal frame")),
                    }
                }
                child.wait().await
            } => Some(result),
        };
        let failed = match result {
            Some(Ok(status)) => !status.success(),
            _ => {
                let _ = tokio::time::timeout(Duration::from_secs(1), async {
                    send(&terminal, json!({"type": "close"})).await?;
                    child
                        .wait()
                        .await
                        .map_err(|err| AppError::Io(err.to_string()))
                })
                .await;
                let _ = child.kill().await;
                result.is_some()
            }
        };
        state
            .terminals
            .lock()
            .expect("terminals mutex poisoned")
            .remove(&id);
        if failed {
            let message = tokio::time::timeout(Duration::from_millis(500), diagnostics)
                .await
                .ok()
                .and_then(Result::ok)
                .filter(|text| !text.is_empty())
                .unwrap_or_else(|| "The shell exited before it could start".into());
            let _ = on_event.send(TerminalEvent::Error { message });
        } else {
            diagnostics.abort();
        }
        let _ = on_event.send(TerminalEvent::Exit);
    });
    Ok(())
}

fn size(rows: u32, cols: u32) -> (u32, u32) {
    (rows.clamp(4, 100), cols.clamp(20, 240))
}

async fn send(terminal: &Terminal, frame: serde_json::Value) -> Result<(), AppError> {
    terminal
        .stdin
        .lock()
        .await
        .write_all((frame.to_string() + "\n").as_bytes())
        .await
        .map_err(|err| AppError::Io(err.to_string()))
}

fn find(id: &str, state: &AppState) -> Option<Terminal> {
    state
        .terminals
        .lock()
        .expect("terminals mutex poisoned")
        .get(id)
        .cloned()
}

#[tauri::command]
pub async fn terminal_input(
    id: String,
    text: String,
    state: State<'_, AppState>,
) -> Result<(), AppError> {
    if text.len() > 128 * 1024 {
        return Err(AppError::Agent(
            "Terminal paste is limited to 128 KiB".into(),
        ));
    }
    let terminal =
        find(&id, &state).ok_or_else(|| AppError::Agent("Terminal has closed".into()))?;
    send(&terminal, json!({"type": "input", "text": text})).await
}

#[tauri::command]
pub async fn terminal_resize(
    id: String,
    rows: u32,
    cols: u32,
    state: State<'_, AppState>,
) -> Result<(), AppError> {
    if let Some(terminal) = find(&id, &state) {
        let (rows, cols) = size(rows, cols);
        send(
            &terminal,
            json!({"type": "resize", "rows": rows, "cols": cols}),
        )
        .await?;
    }
    Ok(())
}

#[tauri::command]
pub fn close_terminal(id: String, state: State<'_, AppState>) {
    if let Some(terminal) = find(&id, &state) {
        terminal.cancel.cancel();
    }
}
