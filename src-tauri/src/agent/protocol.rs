//! Interactive transports. Capabilities come from a successful handshake, never
//! from assuming that every installed CLI implements the latest protocol.
use std::collections::HashMap;
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::Arc;
use std::time::{Duration, Instant};

use serde::Serialize;
use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin};
use tokio::sync::{mpsc, oneshot, Mutex};
use tokio_util::sync::CancellationToken;

use super::{cli, AgentBackend, BoxFut, EventSink, TurnRequest};
use crate::error::AppError;
use crate::events::{AgentEvent, FinishReason, InterruptReply};

#[derive(Clone, Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Capabilities {
    pub streaming: bool,
    pub permissions: bool,
    pub steering: bool,
    pub session_reuse: bool,
    pub read_only: bool,
    pub final_streaming: bool,
}

enum Input {
    Reply(
        String,
        InterruptReply,
        oneshot::Sender<Result<(), AppError>>,
    ),
    Steer(String, oneshot::Sender<Result<(), AppError>>),
}

pub struct InteractiveAgent {
    id: &'static str,
    sessions: Mutex<HashMap<String, Session>>,
    negotiated: std::sync::Mutex<Capabilities>,
    inputs: Mutex<HashMap<String, mpsc::Sender<Input>>>,
}

impl InteractiveAgent {
    pub fn new(id: &'static str) -> Self {
        Self {
            id,
            negotiated: std::sync::Mutex::new(Capabilities::default()),
            sessions: Mutex::new(HashMap::new()),
            inputs: Mutex::new(HashMap::new()),
        }
    }
}

struct Session {
    child: Child,
    _job: super::process::ChildJob,
    stdin: ChildStdin,
    frames: mpsc::Receiver<Result<Value, String>>,
    stderr: Arc<Mutex<String>>,
    identity: String,
    startup_ms: u64,
    acp: bool,
    thread: Option<String>,
    capabilities: Capabilities,
    _files: Vec<cli::TurnFile>,
    decisions: Option<PathBuf>,
}

impl Drop for Session {
    fn drop(&mut self) {
        if let Some(path) = &self.decisions {
            let _ = std::fs::remove_dir_all(path);
        }
    }
}

async fn commandcode_acp() -> bool {
    static AVAILABLE: tokio::sync::OnceCell<bool> = tokio::sync::OnceCell::const_new();
    *AVAILABLE
        .get_or_init(|| async {
            let Ok(mut command) = cli::command("commandcode") else {
                return false;
            };
            tokio::time::timeout(
                Duration::from_secs(10),
                command.args(["acp", "--help"]).output(),
            )
            .await
            .is_ok_and(|result| result.is_ok_and(|output| output.status.success()))
        })
        .await
}

fn acp_option(result: &Value, categories: &[&str], wanted: &str) -> Option<(String, String)> {
    for option in result["configOptions"].as_array()? {
        if !categories
            .iter()
            .any(|category| option["category"] == *category || option["id"] == *category)
        {
            continue;
        }
        let values: Vec<_> = option["options"]
            .as_array()?
            .iter()
            .flat_map(|value| {
                value["options"]
                    .as_array()
                    .map_or_else(|| vec![value], |values| values.iter().collect())
            })
            .collect();
        let matches: Vec<_> = values
            .into_iter()
            .filter_map(|value| value["value"].as_str())
            .filter(|value| {
                *value == wanted
                    || (categories.contains(&"model") && value.ends_with(&format!(":{wanted}")))
            })
            .collect();
        if matches.len() == 1 {
            return Some((option["id"].as_str()?.into(), matches[0].into()));
        }
    }
    None
}

fn parse_frame(bytes: &[u8]) -> Result<Value, String> {
    if bytes.len() > 8 * 1024 * 1024 {
        return Err("Harness frame exceeds 8 MiB".into());
    }
    let value: Value = serde_json::from_slice(bytes)
        .map_err(|error| format!("Malformed harness frame: {error}"))?;
    if !value.is_object() {
        return Err("Harness frames must be JSON objects".into());
    }
    Ok(value)
}

fn mcp_disable_override(name: &str) -> Result<String, AppError> {
    if name.is_empty()
        || !name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
    {
        return Err(AppError::Agent(
            "Cannot safely disable this Codex MCP server name for read-only isolation".into(),
        ));
    }
    Ok(format!("mcp_servers.{name}.enabled=false"))
}

fn mcp_names(value: &Value) -> Result<Vec<String>, AppError> {
    if let Some(servers) = value.as_object() {
        return Ok(servers.keys().cloned().collect());
    }
    if let Some(servers) = value.as_array() {
        return servers
            .iter()
            .map(|server| {
                server["name"].as_str().map(str::to_owned).ok_or_else(|| {
                    AppError::Agent("Codex returned an MCP server without a name".into())
                })
            })
            .collect();
    }
    Err(AppError::Agent(
        "Unsupported Codex MCP listing; read-only isolation cannot be guaranteed".into(),
    ))
}

fn identity(req: &TurnRequest) -> String {
    format!(
        "{}|{:?}|{:?}|{}|{}|{}|{:?}",
        req.workspace
            .as_ref()
            .map(|w| w.root().display().to_string())
            .unwrap_or_default(),
        req.model,
        req.effort,
        req.personality
            .as_ref()
            .map(|p| p.system_prompt.as_str())
            .unwrap_or(""),
        req.read_only,
        req.tools.is_some(),
        req.requests_file
    )
}

impl Session {
    async fn write(&mut self, frame: Value) -> Result<(), AppError> {
        let mut bytes = serde_json::to_vec(&frame).map_err(|e| AppError::Agent(e.to_string()))?;
        bytes.push(b'\n');
        self.stdin
            .write_all(&bytes)
            .await
            .map_err(|e| AppError::Agent(e.to_string()))?;
        self.stdin
            .flush()
            .await
            .map_err(|e| AppError::Agent(e.to_string()))?;
        Ok(())
    }

    async fn next(&mut self) -> Result<Value, AppError> {
        match self.frames.recv().await {
            Some(Ok(frame)) => Ok(frame),
            Some(Err(error)) => Err(AppError::Agent(error)),
            None => Err(AppError::Agent(format!(
                "Harness process closed the protocol. {}",
                self.stderr.lock().await
            ))),
        }
    }

    async fn request(&mut self, frame: Value) -> Result<Value, AppError> {
        let id = frame["id"].clone();
        let operation = frame["method"]
            .as_str()
            .or_else(|| frame["type"].as_str())
            .unwrap_or("request")
            .to_owned();
        let timeout = if matches!(operation.as_str(), "session/new" | "session/load") {
            Duration::from_secs(120)
        } else {
            Duration::from_secs(45)
        };
        self.write(frame).await?;
        let result = tokio::time::timeout(timeout, async {
            loop {
                let frame = self.next().await?;
                if frame["id"] != id {
                    continue;
                }
                if !frame["error"].is_null() || frame["success"] == false {
                    return Err(AppError::Agent(format!(
                        "Protocol request rejected: {}",
                        frame.get("error").unwrap_or(&frame)
                    )));
                }
                if frame["type"] == "control_response" {
                    if frame["response"]["subtype"] == "error" {
                        return Err(AppError::Agent(frame.to_string()));
                    }
                    return Ok(frame["response"]["response"].clone());
                }
                return Ok(frame
                    .get("result")
                    .or_else(|| frame.get("data"))
                    .cloned()
                    .unwrap_or(frame));
            }
        })
        .await;
        match result {
            Ok(result) => result,
            Err(_) => Err(AppError::Agent(format!(
                "Protocol {operation} timed out after {} seconds. {}",
                timeout.as_secs(),
                self.stderr.lock().await
            ))),
        }
    }

    async fn spawn(id: &str, req: &TurnRequest) -> Result<Self, AppError> {
        let spawned = Instant::now();
        let mut command = cli::command(id)?;
        let mut files = Vec::new();
        let credentials = req
            .tools
            .as_ref()
            .map(|bridge| cli::bridge_file(bridge, id))
            .transpose()?;
        let mut mcp = match (&req.tools, &credentials) {
            (Some(bridge), Some(file)) => {
                json!([{ "name": "nova-desktop", "command": bridge.shim, "args": [],
                "env": [{ "name": cli::TOOLS_FILE_ENV, "value": file.0 }] }])
            }
            _ => json!([]),
        };
        if id == "commandcode" {
            if let Some(bridge) = &req.tools {
                mcp[0]["command"] = json!(cli::commandcode_mcp_command(&bridge.shim));
            }
        }
        let mut decisions = None;
        let acp =
            matches!(id, "opencode" | "hermes") || (id == "commandcode" && commandcode_acp().await);
        match id {
            "codex" => {
                // Keep MCP configuration from the proven CLI adapter; replace only exec's transport.
                let (args, _) =
                    cli::arguments(id, req, credentials.as_ref().map(|f| f.0.as_path()));
                command.arg("app-server");
                if req.read_only {
                    let mut probe = cli::command("codex")?;
                    let listing = tokio::time::timeout(
                        Duration::from_secs(10),
                        probe.args(["mcp", "list", "--json"]).output(),
                    )
                    .await
                    .map_err(|_| {
                        AppError::Agent(
                            "Cannot verify read-only MCP isolation: Codex listing timed out".into(),
                        )
                    })?
                    .map_err(|error| AppError::Agent(error.to_string()))?;
                    if !listing.status.success() {
                        return Err(AppError::Agent(
                            "Cannot verify read-only MCP isolation in this Codex installation"
                                .into(),
                        ));
                    }
                    let servers: Value = serde_json::from_slice(&listing.stdout).map_err(|_| {
                        AppError::Agent("Codex returned an unsupported MCP listing".into())
                    })?;
                    for name in mcp_names(&servers)? {
                        command.args(["-c", &mcp_disable_override(&name)?]);
                    }
                    command.args(["-c", "features.apps=false"]);
                }
                let mut index = 0;
                while index + 1 < args.len() {
                    if args[index] == "-c"
                        && !args[index + 1].starts_with("sandbox_mode=")
                        && !args[index + 1].contains("default_tools_approval_mode")
                    {
                        command.args(&args[index..index + 2]);
                        index += 2;
                    } else {
                        index += 1;
                    }
                }
            }
            "pi" => {
                command.args(["--mode", "rpc", "--session-id", &req.chat_id]);
                if let Some(model) = &req.model {
                    command.args(["--model", model]);
                }
                if let Some(effort) = &req.effort {
                    command.args(["--thinking", effort]);
                }
                command.args(["--append-system-prompt", &req.system_prompt()]);
                let extension =
                    cli::TurnFile::write("pi-desktop", ".mjs", include_bytes!("pi-desktop.mjs"))?;
                command.arg("--extension").arg(&extension.0);
                if let (Some(bridge), Some(file)) = (&req.tools, &credentials) {
                    command
                        .env(cli::TOOLS_FILE_ENV, &file.0)
                        .env("NOVA_TOOLS_SHIM", &bridge.shim);
                }
                files.push(extension);
            }
            "claude" => {
                let (args, _) =
                    cli::arguments(id, req, credentials.as_ref().map(|f| f.0.as_path()));
                // No server-wide allowlist: ordinary permissions go through control_request.
                let mut index = 0;
                while index < args.len() {
                    if args[index] == "--allowedTools" {
                        index += 2;
                        continue;
                    }
                    command.arg(&args[index]);
                    index += 1;
                }
                command.args([
                    "--input-format",
                    "stream-json",
                    "--include-partial-messages",
                    "--permission-prompt-tool",
                    "stdio",
                ]);
            }
            "opencode" | "hermes" => {
                command.arg("acp");
            }
            "commandcode" if acp => {
                // Existing local registrations take precedence over ACP session servers.
                if let Some(file) = &credentials {
                    command.env(cli::TOOLS_FILE_ENV, &file.0);
                }
                command.arg("--no-auto-update");
                {
                    let module = cli::TurnFile::write(
                        "personality",
                        ".ts",
                        include_bytes!("commandcode-personality.ts"),
                    )?;
                    command
                        .arg("--mod")
                        .arg(&module.0)
                        .env("NOVA_DESKTOP_SYSTEM_PROMPT", req.system_prompt());
                    files.push(module);
                }
                command.arg("acp");
            }
            "commandcode" => {
                let (args, _) =
                    cli::arguments(id, req, credentials.as_ref().map(|f| f.0.as_path()));
                let module = cli::TurnFile::write(
                    "commandcode-desktop",
                    ".mjs",
                    include_bytes!("commandcode-desktop.mjs"),
                )?;
                let directory =
                    std::env::temp_dir().join(format!("nova-decisions-{}", uuid::Uuid::new_v4()));
                std::fs::create_dir(&directory).map_err(|e| AppError::Agent(e.to_string()))?;
                command
                    .arg("--mod")
                    .arg(&module.0)
                    .env("NOVA_DESKTOP_DECISIONS", &directory);
                command.env("NOVA_DESKTOP_SYSTEM_PROMPT", req.system_prompt());
                if let Some(file) = &credentials {
                    command.env(cli::TOOLS_FILE_ENV, &file.0);
                }
                command.args(args);
                files.push(module);
                decisions = Some(directory);
            }
            _ => unreachable!(),
        }
        if let Some(path) = &req.requests_file {
            command.env(super::requests::ENV, path);
        }
        command
            .env("NO_COLOR", "1")
            .current_dir(req.workspace.as_ref().ok_or(AppError::NoWorkspace)?.root())
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        let mut child = command
            .spawn()
            .map_err(|e| AppError::Agent(format!("Could not start {id}: {e}")))?;
        let job = super::process::ChildJob::attach(&child)?;
        let startup_ms = spawned.elapsed().as_millis() as u64;
        let stdin = child.stdin.take().expect("piped stdin");
        let mut stdout = BufReader::new(child.stdout.take().expect("piped stdout"));
        let mut stderr_reader = child.stderr.take().expect("piped stderr");
        let stderr = Arc::new(Mutex::new(String::new()));
        let tail = stderr.clone();
        tokio::spawn(async move {
            let mut bytes = [0; 2048];
            while let Ok(count @ 1..) = stderr_reader.read(&mut bytes).await {
                let mut tail = tail.lock().await;
                tail.push_str(&String::from_utf8_lossy(&bytes[..count]));
                if tail.len() > 8192 {
                    *tail = tail
                        .chars()
                        .rev()
                        .take(4096)
                        .collect::<String>()
                        .chars()
                        .rev()
                        .collect();
                }
            }
        });
        let (tx, frames) = mpsc::channel(32);
        tokio::spawn(async move {
            let mut bytes = Vec::new();
            loop {
                bytes.clear();
                match (&mut stdout)
                    .take(8 * 1024 * 1024 + 1)
                    .read_until(b'\n', &mut bytes)
                    .await
                {
                    Ok(0) => break,
                    Ok(_) if bytes.len() <= 8 * 1024 * 1024 => {
                        let frame = parse_frame(&bytes);
                        let failed = frame.is_err();
                        if tx.send(frame).await.is_err() || failed {
                            break;
                        }
                    }
                    _ => {
                        let _ = tx
                            .send(Err(
                                "Harness frame exceeds 8 MiB or could not be read".into()
                            ))
                            .await;
                        break;
                    }
                }
            }
        });
        if let Some(file) = credentials {
            files.push(file);
        }
        let mut session = Self {
            child,
            _job: job,
            stdin,
            frames,
            stderr,
            identity: identity(req),
            startup_ms,
            acp,
            thread: None,
            capabilities: Capabilities {
                streaming: true,
                permissions: true,
                steering: matches!(id, "pi" | "codex"),
                session_reuse: id != "commandcode" || acp,
                read_only: id == "codex",
                final_streaming: false,
            },
            _files: files,
            decisions,
        };
        match id {
            "codex" => {
                session.request(json!({"id": "init", "method": "initialize", "params": {
                    "clientInfo": {"name": "nova_desktop", "version": env!("CARGO_PKG_VERSION")},
                    "capabilities": {"experimentalApi": true}
                }})).await?;
                session.write(json!({"method": "initialized"})).await?;
                let mut params = json!({ "cwd": req.workspace.as_ref().unwrap().root(), "model": req.model,
                    "approvalPolicy": "on-request", "sandbox": if req.read_only { "read-only" } else { "workspace-write" },
                    "developerInstructions": req.system_prompt() });
                let method = if let Some(thread) = &req.session {
                    params["threadId"] = json!(thread);
                    "thread/resume"
                } else {
                    "thread/start"
                };
                let result = session
                    .request(json!({"id": "thread", "method": method, "params": params}))
                    .await?;
                session.thread = result["thread"]["id"].as_str().map(str::to_owned);
                if session.thread.is_none() {
                    return Err(AppError::Agent("Codex did not return a thread ID".into()));
                }
            }
            "pi" => {
                let state = session
                    .request(json!({"id":"init","type":"get_state"}))
                    .await?;
                session.thread = state["sessionId"].as_str().map(str::to_owned);
            }
            "claude" => {
                session.write(json!({"type": "control_request", "request_id": "init", "request": {"subtype": "initialize", "hooks": {}}})).await?;
                tokio::time::timeout(Duration::from_secs(45), async {
                    loop {
                        let frame = session.next().await?;
                        if frame["type"] == "control_response"
                            && frame["response"]["request_id"] == "init"
                        {
                            if frame["response"]["subtype"] == "error" {
                                return Err(AppError::Agent(frame.to_string()));
                            }
                            return Ok(());
                        }
                    }
                })
                .await
                .map_err(|_| {
                    AppError::Agent("Claude streaming control initialization timed out".into())
                })??;
            }
            "opencode" | "hermes" | "commandcode" if acp => {
                let ready = session.request(json!({"jsonrpc":"2.0", "id": "init", "method": "initialize", "params": {
                    "protocolVersion": 1, "clientCapabilities": {"fs": {"readTextFile": false, "writeTextFile": false}, "terminal": false},
                    "clientInfo": {"name": "nova-desktop", "version": env!("CARGO_PKG_VERSION")}
                }})).await?;
                if ready["protocolVersion"] != 1 {
                    return Err(AppError::Agent("Unsupported ACP protocol version".into()));
                }
                let load =
                    req.session.is_some() && ready["agentCapabilities"]["loadSession"] == true;
                let mut params =
                    json!({"cwd": req.workspace.as_ref().unwrap().root(), "mcpServers": mcp});
                if load {
                    params["sessionId"] = json!(req.session);
                }
                let result = session.request(json!({"jsonrpc":"2.0", "id": "session", "method": if load { "session/load" } else { "session/new" }, "params": params})).await?;
                session.thread = result["sessionId"]
                    .as_str()
                    .map(str::to_owned)
                    .or_else(|| load.then(|| req.session.clone()).flatten());
                if session.thread.is_none() {
                    return Err(AppError::Agent("ACP did not return a session ID".into()));
                }
                if let Some(model) = &req.model {
                    let supported = result["models"]["availableModels"]
                        .as_array()
                        .is_some_and(|models| models.iter().any(|item| item["modelId"] == *model));
                    if supported {
                        session.request(json!({"jsonrpc":"2.0","id":"model","method":"session/set_model","params":{"sessionId":session.thread,"modelId":model}})).await?;
                    } else if let Some((config, value)) = acp_option(&result, &["model"], model) {
                        session.request(json!({"jsonrpc":"2.0","id":"model","method":"session/set_config_option","params":{"sessionId":session.thread,"configId":config,"value":value}})).await?;
                    } else {
                        return Err(AppError::Agent("The installed ACP harness does not advertise the selected model; choose an available model or its default".into()));
                    }
                }
                if let Some(effort) = &req.effort {
                    let (config, value) = acp_option(&result, &["thought_level","reasoning_effort","effort"], effort).ok_or_else(|| AppError::Agent("The installed ACP harness does not advertise the selected reasoning effort; use its default".into()))?;
                    session.request(json!({"jsonrpc":"2.0","id":"effort","method":"session/set_config_option","params":{"sessionId":session.thread,"configId":config,"value":value}})).await?;
                }
            }
            _ => {}
        }
        Ok(session)
    }
}

/// Response recipes preserve the original wire ID and protocol-specific fields.
fn decision(id: &str, frame: &Value) -> Option<(String, String, Value)> {
    match id {
        "pi" if frame["type"] == "extension_ui_request"
            && matches!(
                frame["method"].as_str(),
                Some("confirm" | "input" | "select" | "editor")
            ) =>
        {
            let kind =
                if frame["method"] == "confirm" && frame["title"] == "Desktop tool permission" {
                    "tool"
                } else {
                    "question"
                };
            Some((
                kind.into(),
                frame["id"].as_str()?.into(),
                json!({"question":frame["title"],"options":frame["options"],"detail":frame["message"]}),
            ))
        }
        "claude"
            if frame["type"] == "control_request"
                && frame["request"]["subtype"] == "can_use_tool" =>
        {
            let request = &frame["request"];
            let kind = match request["tool_name"].as_str() {
                Some("AskUserQuestion") => "question",
                Some("ExitPlanMode") => "plan",
                _ => "tool",
            };
            Some((
                kind.into(),
                frame["request_id"].as_str()?.into(),
                if kind == "question" {
                    json!({"questions":request["input"]["questions"]})
                } else {
                    request.clone()
                },
            ))
        }
        "codex"
            if frame["method"] == "mcpServer/elicitation/request" && frame.get("id").is_some() =>
        {
            let params = &frame["params"];
            let tool = params["_meta"]["codex_approval_kind"] == "mcp_tool_call";
            let payload = if tool {
                params["_meta"].clone()
            } else {
                json!({"question":format!("{}\n{}", params["message"].as_str().unwrap_or("MCP server question"), if params["mode"] == "url" {params["url"].to_string()} else {format!("Answer with a JSON object matching this form: {}", params["requestedSchema"])} )})
            };
            Some((
                if tool { "tool" } else { "question" }.into(),
                frame["id"].to_string(),
                payload,
            ))
        }
        "codex"
            if frame.get("id").is_some()
                && frame["method"].as_str().is_some_and(|m| {
                    m.ends_with("requestApproval")
                        || m.ends_with("requestUserInput")
                        || m.ends_with("requestPermissions")
                }) =>
        {
            let kind = if frame["method"].as_str()?.ends_with("requestUserInput") {
                "question"
            } else {
                "tool"
            };
            Some((
                kind.into(),
                frame["id"].to_string(),
                if kind == "question" {
                    json!({"questions": frame["params"]["questions"]})
                } else {
                    frame["params"].clone()
                },
            ))
        }
        "opencode" | "hermes" if frame["method"] == "session/request_permission" => Some((
            "tool".into(),
            frame["id"].to_string(),
            frame["params"].clone(),
        )),
        "commandcode" if frame["type"] == "desktop_permission" => Some((
            "tool".into(),
            frame["id"].as_str()?.into(),
            frame["request"].clone(),
        )),
        _ => None,
    }
}

fn answer_for(answer: &str, key: &str) -> String {
    match serde_json::from_str::<Value>(answer) {
        Ok(Value::Object(values)) => values
            .get(key)
            .and_then(Value::as_str)
            .unwrap_or_default()
            .into(),
        _ => answer.into(),
    }
}

fn response(id: &str, frame: &Value, reply: InterruptReply) -> Result<Value, AppError> {
    let approve = matches!(
        reply,
        InterruptReply::Tool { approve: true } | InterruptReply::Plan { approve: true, .. }
    );
    let answer = match &reply {
        InterruptReply::Question { answer, .. } => answer.as_str(),
        _ => "",
    };
    Ok(match id {
        "pi" => {
            if frame["method"] == "confirm" {
                json!({"type":"extension_ui_response","id":frame["id"],"confirmed":approve})
            } else {
                json!({"type":"extension_ui_response","id":frame["id"],"value":answer,"cancelled":answer.is_empty()})
            }
        }
        "claude" => {
            let mut input = frame["request"]["input"].clone();
            let question = matches!(reply, InterruptReply::Question { .. });
            if question {
                let mut answers = serde_json::Map::new();
                if let Some(questions) = input["questions"].as_array() {
                    for question in questions {
                        if let Some(key) = question["question"].as_str() {
                            answers.insert(key.into(), json!(answer_for(answer, key)));
                        }
                    }
                }
                input["answers"] = json!(answers);
            }
            json!({"type":"control_response","response":{"subtype":"success","request_id":frame["request_id"],"response":
                if approve || question { json!({"behavior":"allow","updatedInput":input}) } else { json!({"behavior":"deny","message":match &reply { InterruptReply::Plan { feedback:Some(feedback), .. } => feedback.as_str(), _ => "Denied in Nova Desktop" }}) }
            }})
        }
        "codex" => {
            let method = frame["method"].as_str().unwrap_or_default();
            let result = if method == "mcpServer/elicitation/request" {
                if approve {
                    json!({"action":"accept","content":{}})
                } else if matches!(reply, InterruptReply::Question { .. }) && !answer.is_empty() {
                    let content: Value = serde_json::from_str(answer).map_err(|_| {
                        AppError::Agent(
                            "Answer this MCP form with a JSON object, or Skip it".into(),
                        )
                    })?;
                    if !content.is_object() {
                        return Err(AppError::Agent(
                            "The MCP form answer must be a JSON object".into(),
                        ));
                    }
                    json!({"action":"accept","content":content})
                } else {
                    json!({"action":"decline"})
                }
            } else if method.ends_with("requestUserInput") {
                let mut answers = serde_json::Map::new();
                if let Some(questions) = frame["params"]["questions"].as_array() {
                    for question in questions {
                        if let Some(key) = question["id"].as_str() {
                            answers.insert(key.into(), json!({"answers": if answer_for(answer, key).is_empty() { Vec::<String>::new() } else {vec![answer_for(answer, key)]}}));
                        }
                    }
                }
                json!({"answers":answers})
            } else if method == "item/permissions/requestApproval"
                || method.ends_with("requestPermissions")
            {
                json!({"permissions":if approve {frame["params"]["permissions"].clone()} else {json!({})},"scope":"turn"})
            } else {
                json!({"decision":if approve {"accept"} else {"decline"}})
            };
            json!({"id":frame["id"],"result":result})
        }
        "opencode" | "hermes" => {
            let selected = frame["params"]["options"].as_array().and_then(|options| {
                options
                    .iter()
                    .find(|o| o["kind"] == if approve { "allow_once" } else { "reject_once" })
            });
            let outcome = selected
                .map(|o| json!({"outcome":"selected","optionId":o["optionId"]}))
                .unwrap_or(json!({"outcome":"cancelled"}));
            json!({"jsonrpc":"2.0","id":frame["id"],"result":{"outcome":outcome}})
        }
        "commandcode" => json!({"approve":approve}),
        _ => return Err(AppError::Agent("Unknown permission protocol".into())),
    })
}

impl AgentBackend for InteractiveAgent {
    fn capabilities(&self) -> Capabilities {
        self.negotiated.lock().expect("capabilities mutex").clone()
    }
    fn start<'a>(
        &'a self,
        req: TurnRequest,
        sink: EventSink,
        cancel: CancellationToken,
    ) -> BoxFut<'a, Result<FinishReason, AppError>> {
        Box::pin(async move {
            let started = Instant::now();
            let key = req.chat_id.clone();
            let cached = self.sessions.lock().await.remove(&key);
            let mut cached = cached.filter(|s| {
                s.identity == identity(&req)
                    && req
                        .session
                        .as_ref()
                        .is_none_or(|id| s.thread.as_ref() == Some(id))
            });
            if cached
                .as_mut()
                .is_some_and(|s| !matches!(s.child.try_wait(), Ok(None)))
            {
                cached = None;
            }
            let reused = cached.is_some();
            if !reused {
                *self.negotiated.lock().expect("capabilities mutex") = Capabilities::default();
            }
            let mut session = match cached {
                Some(s) => s,
                None => tokio::select! {
                    _ = cancel.cancelled() => return Ok(FinishReason::Cancelled),
                    session = Session::spawn(self.id, &req) => match session {
                        Ok(session) => session,
                        Err(error) if !req.read_only && !matches!(self.id, "commandcode" | "hermes") => {
                            let _ = sink.send(AgentEvent::Notice { level:"error".into(), message:format!("{} interactive capability unavailable: {error}. Using CLI compatibility mode; interactive decisions and steering are unavailable until the harness is updated.", self.id) });
                            return cli::CliAgent(self.id).start(req, sink, cancel).await;
                        }
                        Err(error) => return Err(error),
                    },
                },
            };
            if self.id == "commandcode" {
                for line in session
                    .stderr
                    .lock()
                    .await
                    .lines()
                    .filter(|line| line.starts_with("[acp] mcp nova-desktop "))
                {
                    let _ = sink.send(AgentEvent::Notice {
                        level: "info".into(),
                        message: line.into(),
                    });
                }
            }
            *self.negotiated.lock().expect("capabilities mutex") = session.capabilities.clone();
            let _ = sink.send(AgentEvent::Notice {
                level: "info".into(),
                message: format!(
                    "{} {} · protocol ready {} ms · {:?}",
                    self.id,
                    if reused { "session reused" } else { "started" },
                    started.elapsed().as_millis(),
                    session.capabilities
                ),
            });
            let _ = sink.send(AgentEvent::Timing {
                phase: "processStartup".into(),
                ms: if reused { 0 } else { session.startup_ms },
            });
            let _ = sink.send(AgentEvent::Timing {
                phase: "protocolReady".into(),
                ms: started.elapsed().as_millis() as u64,
            });
            if let Some(thread) = &session.thread {
                let _ = sink.send(AgentEvent::Session { id: thread.clone() });
            }
            let (tx, mut rx) = mpsc::channel(16);
            self.inputs.lock().await.insert(req.turn_id.clone(), tx);
            let wire = if session.acp { "opencode" } else { self.id };
            let prompt = match wire {
                "codex" => {
                    json!({"id":"prompt","method":"turn/start","params":{"threadId":session.thread,"input":[{"type":"text","text":req.prompt,"text_elements":[]}],"effort":req.effort}})
                }
                "pi" => json!({"id":"prompt","type":"prompt","message":req.prompt}),
                "claude" => json!({"type":"user","message":{"role":"user","content":req.prompt}}),
                "opencode" | "hermes" => {
                    json!({"jsonrpc":"2.0","id":"prompt","method":"session/prompt","params":{"sessionId":session.thread,"prompt":[{"type":"text","text": format!("{}\n\n{}",req.system_prompt(),req.prompt)}]}})
                }
                _ => Value::Null,
            };
            let outcome = async {
                if !prompt.is_null() { session.write(prompt).await?; }
                let mut pending = HashMap::<String, Value>::new();
                let mut steering = HashMap::<String, oneshot::Sender<Result<(), AppError>>>::new();
                let mut frames = cli::Frames::default();
                let mut final_text = String::new();
                let mut turn = String::new();
                let mut first = true;
                let mut final_items = std::collections::HashSet::new();
                loop {
                    tokio::select! {
                        biased;
                        _ = cancel.cancelled() => { let _ = session.child.kill().await; return Ok(FinishReason::Cancelled); }
                        Some(input) = rx.recv() => match input {
                            Input::Reply(id, reply, done) => {
                                let result = if let Some(frame) = pending.get(&id) {
                                    match response(wire, frame, reply) {
                                        Ok(response) => if let Some(directory) = &session.decisions {
                                            let target = directory.join(format!("{id}.json"));
                                            let temporary = target.with_extension("tmp");
                                            std::fs::write(&temporary, response.to_string()).and_then(|()| std::fs::rename(temporary, target)).map_err(|e| AppError::Agent(e.to_string()))
                                        } else { session.write(response).await },
                                        Err(error) => Err(error),
                                    }
                                } else { Err(AppError::Agent("Unknown protocol decision".into())) };
                                if result.is_ok() { pending.remove(&id); }
                                let _ = done.send(result);
                            }
                            Input::Steer(text, done) => {
                                let frame = match self.id {
                                    "pi" => Some(json!({"id":uuid::Uuid::new_v4().to_string(),"type":"steer","message":text})),
                                    "codex" if !turn.is_empty() => Some(json!({"id":uuid::Uuid::new_v4().to_string(),"method":"turn/steer","params":{"threadId":session.thread,"expectedTurnId":turn,"input":[{"type":"text","text":text,"text_elements":[]}]}})),
                                    _ => None,
                                };
                                match frame {
                                    Some(frame) => {
                                        let id = frame["id"].as_str().unwrap_or_default().to_owned();
                                        match session.write(frame).await {
                                            Ok(()) => { steering.insert(id, done); }
                                            Err(error) => { let _ = done.send(Err(error)); }
                                        }
                                    }
                                    None => { let _ = done.send(Err(AppError::Agent("This harness cannot be steered during this turn".into()))); }
                                }
                            }
                        },
                        frame = session.next() => {
                            let frame = frame?;
                            if let Some(done) = frame["id"].as_str().and_then(|id| steering.remove(id)) {
                                let result = if frame["success"] == false || !frame["error"].is_null() { Err(AppError::Agent(format!("Steering was rejected: {}", frame["error"]))) } else { Ok(()) };
                                let _ = done.send(result);
                                continue;
                            }
                            if let Some((kind, id, payload)) = decision(wire, &frame) {
                                if req.read_only && kind != "question" {
                                    session.write(response(wire, &frame, InterruptReply::Tool { approve:false })?).await?;
                                    continue;
                                }
                                pending.insert(id.clone(), frame);
                                let _ = sink.send(AgentEvent::ApprovalRequired { interrupt_id:format!("{}/{}", req.turn_id, id), kind, payload });
                                continue;
                            }
                            if self.id == "claude" && frame["type"] == "control_request" {
                                session.write(json!({"type":"control_response","response":{"subtype":"error","request_id":frame["request_id"],"error":"Desktop does not support this control request"}})).await?;
                                continue;
                            }
                            // Unknown server requests are refused explicitly instead of hanging.
                            if matches!(wire, "codex" | "opencode" | "hermes") && frame.get("method").is_some() && frame.get("id").is_some() {
                                let _ = sink.send(AgentEvent::Notice { level:"error".into(), message:format!("{} requested unsupported client capability {}", self.id, frame["method"]) });
                                session.write(json!({"jsonrpc":"2.0","id":frame["id"],"error":{"code":-32601,"message":"Desktop does not support this client capability"}})).await?;
                                continue;
                            }
                            if frame["success"] == false || !frame["error"].is_null() { return Err(AppError::Agent(frame.to_string())); }
                            if frame["method"] == "turn/started" { turn = frame["params"]["turn"]["id"].as_str().unwrap_or_default().into(); }
                            if self.id == "codex" && frame["method"] == "item/completed" && frame["params"]["item"]["type"] == "agentMessage" {
                                let item = &frame["params"]["item"];
                                if item["phase"] != "commentary" {
                                    if item["phase"] == "final_answer" && !final_text.is_empty() { final_text.push_str("\n\n"); }
                                    else { final_text.clear(); }
                                    final_text.push_str(item["text"].as_str().unwrap_or_default());
                                }
                            }
                            if matches!(wire, "opencode" | "hermes") && frame["method"] == "session/update" {
                                let update = &frame["params"]["update"];
                                if update["sessionUpdate"] == "tool_call" { final_text.clear(); }
                                if update["sessionUpdate"] == "agent_message_chunk" { final_text.push_str(update["content"]["text"].as_str().unwrap_or_default()); }
                            }
                            if self.id == "pi" && frame["type"] == "message_end" && frame["message"]["role"] == "assistant" {
                                let message = &frame["message"];
                                final_text = message["content"].as_array().map(|blocks| blocks.iter()
                                    .filter(|block| block["type"] == "text").filter_map(|block| block["text"].as_str()).collect::<Vec<_>>().join("\n")).unwrap_or_default();
                                if message["content"].as_array().is_some_and(|blocks| blocks.iter().any(|block| block["type"] == "toolCall")) { final_text.clear(); }
                            }
                            if self.id == "commandcode" && frame["type"] == "result" { final_text = frame["finalText"].as_str().unwrap_or_default().into(); }
                            if self.id == "claude" && frame["type"] == "result" { final_text = frame["result"].as_str().unwrap_or_default().into(); }
                            if self.id == "codex" && frame["method"] == "item/started" && frame["params"]["item"]["phase"] == "final_answer" {
                                session.capabilities.final_streaming = true;
                                self.negotiated.lock().expect("capabilities mutex").final_streaming = true;
                                final_items.insert(frame["params"]["item"]["id"].as_str().unwrap_or_default().to_owned());
                            }
                            let final_delta = self.id == "codex" && frame["method"] == "item/agentMessage/delta"
                                && final_items.contains(frame["params"]["itemId"].as_str().unwrap_or_default());
                            let visible = if final_delta {
                                let _ = sink.send(AgentEvent::FinalAnswerDelta { text:frame["params"]["delta"].as_str().unwrap_or_default().into() });
                                true
                            } else { normalize(wire, &frame, &sink, &mut frames) };
                            if first && visible {
                                first = false;
                                let _ = sink.send(AgentEvent::Notice { level:"info".into(), message:format!("First visible text · {} ms", started.elapsed().as_millis()) });
                            }
                            let finished = match wire {
                                "codex" => frame["method"] == "turn/completed",
                                "pi" => frame["type"] == "agent_settled" || (frame["type"] == "response" && frame["command"] == "prompt" && frame["data"]["disposition"] == "handled"),
                                "claude" | "commandcode" => frame["type"] == "result",
                                _ => frame["id"] == "prompt" && frame.get("result").is_some(),
                            };
                            if finished {
                                if frame["is_error"] == true || frame["params"]["turn"]["status"] == "failed" || frames.failure.is_some() {
                                    return Err(AppError::Agent(frames.failure.clone().unwrap_or_else(|| frame.to_string())));
                                }
                                if frame["params"]["turn"]["status"] == "interrupted" { return Ok(FinishReason::Cancelled); }
                                if !final_text.is_empty() { let _ = sink.send(AgentEvent::MessageFinal { text:final_text, author:None, replace_preview:true, plain:false }); }
                                return Ok(FinishReason::Completed);
                            }
                        }
                    }
                }
            }.await;
            self.inputs.lock().await.remove(&req.turn_id);
            if matches!(outcome, Ok(FinishReason::Completed)) && session.capabilities.session_reuse
            {
                let mut sessions = self.sessions.lock().await;
                if sessions.len() >= 12 {
                    sessions.clear();
                }
                sessions.insert(key, session);
            }
            outcome
        })
    }

    fn prewarm<'a>(&'a self, req: TurnRequest) -> BoxFut<'a, Result<(), AppError>> {
        Box::pin(async move {
            // CommandCode's print mode executes immediately; it cannot be prewarmed.
            if self.id == "commandcode" && !commandcode_acp().await {
                return Ok(());
            }
            let key = req.chat_id.clone();
            if self
                .sessions
                .lock()
                .await
                .get(&key)
                .is_some_and(|session| session.identity == identity(&req))
            {
                return Ok(());
            }
            *self.negotiated.lock().expect("capabilities mutex") = Capabilities::default();
            let session = Session::spawn(self.id, &req).await?;
            *self.negotiated.lock().expect("capabilities mutex") = session.capabilities.clone();
            let mut sessions = self.sessions.lock().await;
            if !sessions.contains_key(&key) && sessions.len() < 12 {
                sessions.insert(key, session);
            }
            Ok(())
        })
    }

    fn respond<'a>(
        &'a self,
        id: String,
        reply: InterruptReply,
    ) -> BoxFut<'a, Result<(), AppError>> {
        Box::pin(async move {
            // Each protocol request is namespaced by turn before reaching the broker.
            let (turn, id) = id
                .split_once('/')
                .ok_or_else(|| AppError::Agent("Decision is missing its turn identity".into()))?;
            let input = self
                .inputs
                .lock()
                .await
                .get(turn)
                .cloned()
                .ok_or_else(|| AppError::Agent("The turn has finished".into()))?;
            let (tx, rx) = oneshot::channel();
            input
                .send(Input::Reply(id.into(), reply, tx))
                .await
                .map_err(|_| AppError::Agent("Protocol input closed".into()))?;
            rx.await
                .map_err(|_| AppError::Agent("Protocol response was interrupted".into()))?
        })
    }

    fn steer<'a>(&'a self, turn: &'a str, text: &'a str) -> BoxFut<'a, Result<(), AppError>> {
        Box::pin(async move {
            let input = self
                .inputs
                .lock()
                .await
                .get(turn)
                .cloned()
                .ok_or_else(|| AppError::Agent("The turn has finished".into()))?;
            let (tx, rx) = oneshot::channel();
            input
                .send(Input::Steer(text.into(), tx))
                .await
                .map_err(|_| AppError::Agent("Protocol input closed".into()))?;
            tokio::time::timeout(Duration::from_secs(15), rx)
                .await
                .map_err(|_| {
                    AppError::Agent("Steering was not acknowledged within 15 seconds".into())
                })?
                .map_err(|_| AppError::Agent("Protocol input was interrupted".into()))?
        })
    }
}

/// A codex tool item is one JSON object, and the fields a person reads — the
/// diff, the command, the output — hold real newlines. `Value::to_string`
/// flattens those into escapes, so the transcript gets a single line of \n; take
/// the payload out of the item instead and let the row's label say what ran.
fn codex_payload(item: &Value, keys: &[&str]) -> String {
    if let Some(changes) = item["changes"].as_array() {
        return changes
            .iter()
            .map(|change| match change["diff"].as_str() {
                Some(diff) => format!("{}\n{diff}", change["path"].as_str().unwrap_or("")),
                None => change["path"].as_str().unwrap_or("").to_string(),
            })
            .collect::<Vec<_>>()
            .join("\n\n");
    }
    for key in keys {
        let value = &item[*key];
        if value.is_null() {
            continue;
        }
        return match value.as_str() {
            Some(text) => text.to_string(),
            None => serde_json::to_string_pretty(value).unwrap_or_default(),
        };
    }
    String::new()
}

fn codex_input(item: &Value) -> String {
    // The order matters: commandExecution carries a command, mcpToolCall its
    // arguments, a search its query.
    codex_payload(item, &["command", "arguments", "query", "prompt"])
}

fn codex_output(item: &Value) -> String {
    codex_payload(item, &["aggregatedOutput", "result", "content", "error"])
}

fn normalize(id: &str, frame: &Value, sink: &EventSink, frames: &mut cli::Frames) -> bool {
    if let Some(text) = cli::reasoning(id, frame) {
        let _ = sink.send(AgentEvent::ReasoningDelta {
            text: text.chars().take(64000).collect(),
        });
        return false;
    }
    let mut text = None;
    match id {
        "codex" => match frame["method"].as_str() {
            Some("item/agentMessage/delta") => {
                text = frame["params"]["delta"].as_str();
            }
            Some("item/started") => {
                let item = &frame["params"]["item"];
                if matches!(
                    item["type"].as_str(),
                    Some(
                        "commandExecution"
                            | "fileChange"
                            | "mcpToolCall"
                            | "dynamicToolCall"
                            | "webSearch"
                            | "imageView"
                            | "imageGeneration"
                            | "collabAgentToolCall"
                    )
                ) {
                    let _ = sink.send(AgentEvent::ToolCall {
                        call_id: item["id"].as_str().unwrap_or_default().into(),
                        name: item["type"].as_str().unwrap_or("Tool").into(),
                        input: codex_input(item).chars().take(8_000).collect(),
                    });
                }
            }
            Some("item/completed") => {
                let item = &frame["params"]["item"];
                if matches!(
                    item["type"].as_str(),
                    Some(
                        "commandExecution"
                            | "fileChange"
                            | "mcpToolCall"
                            | "dynamicToolCall"
                            | "webSearch"
                            | "imageView"
                            | "imageGeneration"
                            | "collabAgentToolCall"
                    )
                ) {
                    let _ = sink.send(AgentEvent::ToolResult {
                        call_id: item["id"].as_str().unwrap_or_default().into(),
                        ok: !matches!(
                            item["status"].as_str(),
                            Some("failed" | "declined" | "cancelled")
                        ) && item["exitCode"].as_i64().unwrap_or(0) == 0
                            && item["error"].is_null()
                            && item["result"]["isError"] != true,
                        output: codex_output(item).chars().take(32_000).collect(),
                    });
                }
            }
            Some("thread/tokenUsage/updated") => {
                let usage = &frame["params"]["tokenUsage"]["last"];
                let _ = sink.send(AgentEvent::Usage {
                    input: usage["inputTokens"].as_u64().unwrap_or(0),
                    output: usage["outputTokens"].as_u64().unwrap_or(0),
                });
            }
            _ => {}
        },
        "opencode" | "hermes" => {
            let update = &frame["params"]["update"];
            match update["sessionUpdate"].as_str() {
                Some("agent_message_chunk") => text = update["content"]["text"].as_str(),
                Some("tool_call") => {
                    let _ = sink.send(AgentEvent::ToolCall {
                        call_id: update["toolCallId"].as_str().unwrap_or_default().into(),
                        name: update["title"].as_str().unwrap_or("Tool").into(),
                        input: update["rawInput"].to_string(),
                    });
                }
                Some("tool_call_update")
                    if matches!(update["status"].as_str(), Some("completed" | "failed")) =>
                {
                    let _ = sink.send(AgentEvent::ToolResult {
                        call_id: update["toolCallId"].as_str().unwrap_or_default().into(),
                        ok: update["status"] == "completed",
                        output: update["content"].to_string(),
                    });
                }
                _ => {}
            }
        }
        "claude" => {
            if frame["type"] == "result" {
                if let (Some(input), Some(output)) = (
                    frame["usage"]["input_tokens"].as_u64(),
                    frame["usage"]["output_tokens"].as_u64(),
                ) {
                    let _ = sink.send(AgentEvent::Usage { input, output });
                }
            }
            if frame["type"] == "stream_event" && frame["event"]["delta"]["type"] == "text_delta" {
                text = frame["event"]["delta"]["text"].as_str();
            }
            // Tool and usage events still use the proven one-shot decoder, but its
            // assistant text is withheld until result identifies the final answer.
            if frame["type"] != "assistant" && frame["type"] != "result" {
                cli::handle_frame(id, frame, sink, frames);
            } else if let Some(blocks) = frame["message"]["content"].as_array() {
                for block in blocks.iter().filter(|b| b["type"] == "tool_use") {
                    let _ = sink.send(AgentEvent::ToolCall {
                        call_id: block["id"].as_str().unwrap_or_default().into(),
                        name: block["name"].as_str().unwrap_or("Tool").into(),
                        input: block["input"].to_string(),
                    });
                }
            }
        }
        "pi" => {
            if frame["type"] == "extension_ui_request"
                && matches!(frame["method"].as_str(), Some("notify" | "setStatus"))
            {
                if let Some(message) = frame["message"]
                    .as_str()
                    .or_else(|| frame["statusText"].as_str())
                {
                    let _ = sink.send(AgentEvent::Notice {
                        level: "info".into(),
                        message: message.chars().take(2000).collect(),
                    });
                }
            }
            if frame["type"] == "message_end" {
                let usage = &frame["message"]["usage"];
                if let (Some(input), Some(output)) =
                    (usage["input"].as_u64(), usage["output"].as_u64())
                {
                    let _ = sink.send(AgentEvent::Usage { input, output });
                }
            }
            if frame["type"] == "message_update"
                && frame["assistantMessageEvent"]["type"] == "text_delta"
            {
                text = frame["assistantMessageEvent"]["delta"].as_str();
            } else {
                cli::handle_frame(id, frame, sink, frames);
            }
        }
        "commandcode" => {
            let target = sink.clone();
            let preview = tauri::ipc::Channel::new(move |body| {
                let event: AgentEvent = match body {
                    tauri::ipc::InvokeResponseBody::Json(text) => serde_json::from_str(&text),
                    tauri::ipc::InvokeResponseBody::Raw(bytes) => serde_json::from_slice(&bytes),
                }
                .map_err(|error| tauri::Error::Anyhow(error.into()))?;
                target.send(match event {
                    AgentEvent::MessageDelta { text } => AgentEvent::MessagePreview { text },
                    other => other,
                })
            });
            cli::handle_frame(id, frame, &preview, frames);
        }
        _ => {
            cli::handle_frame(id, frame, sink, frames);
        }
    }
    if let Some(text) = text.filter(|s| !s.is_empty()) {
        let _ = sink.send(AgentEvent::MessagePreview { text: text.into() });
        true
    } else {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codex_overrides_use_literal_safe_path_segments() {
        assert_eq!(
            mcp_disable_override("agentmemory").unwrap(),
            "mcp_servers.agentmemory.enabled=false"
        );
        assert!(mcp_disable_override("local.files").is_err());
        assert!(mcp_disable_override("\"quoted\"").is_err());
    }

    #[test]
    fn codex_tool_items_are_read_as_their_payload_not_as_escaped_json() {
        let change = json!({"changes":[
            {"path":"src/lib/patch.py","diff":"@@ -1 +1 @@\n-old\n+new"},
            {"path":"src/gone.py","diff":null}
        ]});
        assert_eq!(
            codex_input(&change),
            "src/lib/patch.py\n@@ -1 +1 @@\n-old\n+new\n\nsrc/gone.py"
        );
        let command = json!({"command":"npm run lint","aggregatedOutput":"ok\n"});
        assert_eq!(codex_input(&command), "npm run lint");
        assert_eq!(codex_output(&command), "ok\n");
        let mcp = json!({"arguments":{"path":"README"},"result":{"content":"# Fixture"}});
        assert_eq!(codex_input(&mcp), "{\n  \"path\": \"README\"\n}");
        assert_eq!(codex_output(&mcp), "{\n  \"content\": \"# Fixture\"\n}");
        // Nothing to show is empty, not a dump of the protocol item.
        assert_eq!(codex_output(&json!({"type":"webSearch"})), "");
    }

    #[test]
    fn malformed_and_oversized_frames_fail_without_buffering_further() {
        assert!(parse_frame(b"{broken\n").is_err());
        assert!(parse_frame(b"null\n").is_err());
        assert!(parse_frame(&vec![b' '; 8 * 1024 * 1024 + 1]).is_err());
        assert_eq!(parse_frame(b"{\"id\":7}\r\n").unwrap()["id"], 7);
    }

    #[test]
    fn acp_configuration_uses_advertised_values_and_rejects_ambiguous_models() {
        let result = json!({"configOptions":[{"id":"model","category":"model","options":[{"value":"provider:model"}]},{"id":"effort","category":"thought_level","options":[{"value":"high"}]}]});
        assert_eq!(
            acp_option(&result, &["model"], "model"),
            Some(("model".into(), "provider:model".into()))
        );
        assert_eq!(
            acp_option(&result, &["thought_level"], "high"),
            Some(("effort".into(), "high".into()))
        );
        assert!(acp_option(&result, &["thought_level"], "unknown").is_none());
        let ambiguous = json!({"configOptions":[{"id":"model","category":"model","options":[{"value":"first:model"},{"value":"second:model"}]}]});
        assert!(acp_option(&ambiguous, &["model"], "model").is_none());
    }

    #[test]
    fn codex_tool_elicitation_is_a_permission_but_forms_are_questions() {
        let mut frame = json!({"method":"mcpServer/elicitation/request","id":0,"params":{"mode":"form","_meta":{"codex_approval_kind":"mcp_tool_call","tool_name":"create_task"},"requestedSchema":{"type":"object","properties":{}}}});
        assert_eq!(decision("codex", &frame).unwrap().0, "tool");
        assert_eq!(
            response("codex", &frame, InterruptReply::Tool { approve: true }).unwrap()["result"],
            json!({"action":"accept","content":{}})
        );
        frame["params"]["_meta"] = json!({});
        assert_eq!(decision("codex", &frame).unwrap().0, "question");
        assert_eq!(
            response(
                "codex",
                &frame,
                InterruptReply::Question {
                    answer: "{\"name\":\"reader\"}".into(),
                    selected_index: None
                }
            )
            .unwrap()["result"]["content"]["name"],
            "reader"
        );
    }

    #[test]
    fn permission_grants_do_not_become_exec_approval_responses() {
        let frame = json!({"method":"item/permissions/requestApproval","id":3,"params":{"permissions":{"network":{"enabled":true}}}});
        assert_eq!(
            response("codex", &frame, InterruptReply::Tool { approve: false }).unwrap()["result"]
                ["permissions"],
            json!({})
        );
        assert_eq!(
            response("codex", &frame, InterruptReply::Tool { approve: true }).unwrap()["result"]
                ["scope"],
            "turn"
        );
    }

    #[test]
    fn read_only_isolation_accepts_both_installed_listing_shapes_and_rejects_unknown_data() {
        assert_eq!(
            mcp_names(&json!({"local.files":{},"remote":{}})).unwrap(),
            vec!["local.files", "remote"]
        );
        assert_eq!(
            mcp_names(&json!([{"name":"remote"}])).unwrap(),
            vec!["remote"]
        );
        assert!(mcp_names(&json!([{}])).is_err());
        assert!(mcp_names(&json!(null)).is_err());
    }

    #[test]
    fn progress_notifications_are_not_permission_requests_or_model_text() {
        let (sink, seen) = crate::events::testutil::collect_events();
        let frame = json!({"type":"extension_ui_request","id":"notice","method":"notify","message":"Working"});
        assert!(decision("pi", &frame).is_none());
        normalize("pi", &frame, &sink, &mut cli::Frames::default());
        assert!(
            matches!(&seen.lock().unwrap()[0], AgentEvent::Notice { message, .. } if message == "Working")
        );
    }
    #[test]
    fn protocol_decisions_keep_wire_ids_and_deny_by_default() {
        let frame = json!({"id":7,"method":"session/request_permission","params":{"options":[{"optionId":"yes","kind":"allow_once"},{"optionId":"no","kind":"reject_once"}]}});
        assert_eq!(decision("hermes", &frame).unwrap().1, "7");
        assert_eq!(
            response("hermes", &frame, InterruptReply::Tool { approve: false }).unwrap()["result"]
                ["outcome"]["optionId"],
            "no"
        );
        let frame = json!({"id":42,"method":"item/commandExecution/requestApproval","params":{}});
        assert_eq!(
            response("codex", &frame, InterruptReply::Tool { approve: false }).unwrap(),
            json!({"id":42,"result":{"decision":"decline"}})
        );
    }
    #[test]
    fn questions_are_not_tool_permissions() {
        let frame =
            json!({"type":"extension_ui_request","id":"q","method":"input","title":"Name?"});
        assert_eq!(decision("pi", &frame).unwrap().0, "question");
        assert!(decision(
            "pi",
            &json!({"type":"extension_ui_request","method":"notify"})
        )
        .is_none());
    }
    #[test]
    fn codex_user_messages_are_not_tool_activity() {
        let (sink, seen) = crate::events::testutil::collect_events();
        for method in ["item/started", "item/completed"] {
            normalize(
                "codex",
                &json!({"method":method,"params":{"item":{"type":"userMessage","id":"user","content":[]}}}),
                &sink,
                &mut cli::Frames::default(),
            );
        }
        assert!(seen.lock().unwrap().is_empty());
    }

    #[test]
    fn exposed_thinking_uses_its_own_channel_across_protocols() {
        for (id, frame) in [
            (
                "codex",
                json!({"method":"item/reasoning/summaryTextDelta","params":{"delta":"Checking"}}),
            ),
            (
                "pi",
                json!({"type":"message_update","assistantMessageEvent":{"type":"thinking_delta","delta":"Checking"}}),
            ),
            (
                "claude",
                json!({"type":"stream_event","event":{"delta":{"type":"thinking_delta","thinking":"Checking"}}}),
            ),
            (
                "opencode",
                json!({"method":"session/update","params":{"update":{"sessionUpdate":"agent_thought_chunk","content":{"text":"Checking"}}}}),
            ),
        ] {
            let (sink, seen) = crate::events::testutil::collect_events();
            assert!(!normalize(id, &frame, &sink, &mut cli::Frames::default()));
            assert_eq!(
                *seen.lock().unwrap(),
                vec![AgentEvent::ReasoningDelta {
                    text: "Checking".into()
                }]
            );
        }
    }

    #[test]
    fn reasoning_never_becomes_assistant_text() {
        let (sink, seen) = crate::events::testutil::collect_events();
        normalize(
            "codex",
            &json!({"method":"item/reasoning/textDelta","params":{"delta":"private"}}),
            &sink,
            &mut cli::Frames::default(),
        );
        assert!(seen.lock().unwrap().is_empty());
    }
}
