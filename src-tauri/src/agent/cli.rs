use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use serde::Serialize;
use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, BufReader};
use tokio::process::Command;
use tokio_util::sync::CancellationToken;

use super::{AgentBackend, BoxFut, EventSink, TurnRequest};
use crate::error::AppError;
use crate::events::{AgentEvent, FinishReason};

pub const AGENTS: &[(&str, &str)] = &[
    ("nova", "Nova"),
    ("pi", "Pi Agent"),
    ("claude", "Claude Code"),
    ("codex", "Codex"),
    ("opencode", "OpenCode"),
    ("commandcode", "CommandCode"),
    ("hermes", "Hermes"),
];

const FRAME_LIMIT: u64 = 8 * 1024 * 1024;

struct Executable {
    program: PathBuf,
    entry: EntryPoint,
}

enum EntryPoint {
    Binary,
    Node(PathBuf),
    NovaSource(PathBuf),
}

impl Executable {
    fn command(&self) -> Command {
        let mut command = Command::new(&self.program);
        match &self.entry {
            EntryPoint::Binary => {}
            EntryPoint::Node(script) => {
                command.arg(script);
            }
            EntryPoint::NovaSource(checkout) => {
                command.args(["-I", "-B", "-c",
                    "import sys; sys.path.insert(0, sys.argv.pop(1)); sys.argv[0] = 'nova'; from novacode_cli.entrypoint import cli_main; cli_main()"])
                    .arg(checkout);
            }
        }
        #[cfg(windows)]
        command.creation_flags(0x08000000); // CREATE_NO_WINDOW
        command.kill_on_drop(true);
        command
    }

    fn display(&self) -> String {
        match &self.entry {
            EntryPoint::Binary => self.program.display().to_string(),
            EntryPoint::Node(script) => script.display().to_string(),
            EntryPoint::NovaSource(checkout) => {
                format!("{} via {}", checkout.display(), self.program.display())
            }
        }
    }
}

fn executable(dir: &Path, name: &str) -> Option<PathBuf> {
    let file = dir.join(if cfg!(windows) {
        format!("{name}.exe")
    } else {
        name.into()
    });
    file.is_file().then_some(file)
}

fn nova_checkout() -> PathBuf {
    std::env::var_os("NOVA_SOURCE")
        .map(PathBuf::from)
        .unwrap_or_else(|| Path::new(env!("CARGO_MANIFEST_DIR")).join("../../NovaCode"))
}

fn resolve(id: &str) -> Option<Executable> {
    let dirs: Vec<_> = std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default())
        .filter(|dir| dir.is_absolute())
        .collect();
    let direct = |program| Executable {
        program,
        entry: EntryPoint::Binary,
    };
    if id == "nova" {
        if let Some(override_path) = std::env::var_os("NOVA_BIN") {
            let path = PathBuf::from(override_path);
            return if path.is_absolute() && path.is_file() {
                Some(direct(path))
            } else {
                let name = path.file_stem()?.to_str()?;
                (path.components().count() == 1)
                    .then(|| {
                        dirs.iter()
                            .find_map(|dir| executable(dir, name))
                            .map(direct)
                    })
                    .flatten()
            };
        }
        let checkout = nova_checkout();
        if checkout.is_absolute() && checkout.join("novacode_cli/entrypoint.py").is_file() {
            let venv = checkout.join(".venv");
            let scripts = if cfg!(windows) { "Scripts" } else { "bin" };
            let bin = venv.join(scripts);
            let base_exists = !cfg!(windows)
                || std::fs::read_to_string(venv.join("pyvenv.cfg")).is_ok_and(|config| {
                    config.lines().any(|line| {
                        line.split_once('=').is_some_and(|(key, value)| {
                            key.trim() == "home"
                                && Path::new(value.trim()).join("python.exe").is_file()
                        })
                    })
                });
            if base_exists {
                if let Some(path) = executable(&bin, "nova") {
                    return Some(direct(path));
                }
            }
            let python = std::env::var_os("NOVA_PYTHON")
                .map(PathBuf::from)
                .filter(|path| path.is_absolute() && path.is_file());
            if let Some(program) = python {
                return Some(Executable {
                    program,
                    entry: EntryPoint::NovaSource(checkout),
                });
            }
        }
    }
    let npm = match id {
        "pi" => &[
            "@earendil-works/pi-coding-agent",
            "@mariozechner/pi-coding-agent",
        ][..],
        "codex" => &["@openai/codex"],
        "opencode" => &["@opencode/cli", "opencode-ai"],
        "commandcode" => &["command-code"],
        "claude" => &["@anthropic-ai/claude-code"],
        _ => &[],
    };
    for dir in &dirs {
        if let Some(path) = executable(dir, id) {
            return Some(direct(path));
        }
        if cfg!(windows) {
            for package in npm {
                let root = dir.join("node_modules").join(package);
                let Ok(raw) = std::fs::read(root.join("package.json")) else {
                    continue;
                };
                let Ok(manifest) = serde_json::from_slice::<Value>(&raw) else {
                    continue;
                };
                let bin = &manifest["bin"];
                let Some(relative) = bin[id].as_str().or_else(|| bin.as_str()) else {
                    continue;
                };
                let Ok(entry) = root.join(relative).canonicalize() else {
                    continue;
                };
                if !entry.is_file()
                    || !root
                        .canonicalize()
                        .is_ok_and(|path| entry.starts_with(path))
                {
                    continue;
                }
                if entry
                    .extension()
                    .is_some_and(|ext| ext.eq_ignore_ascii_case("exe"))
                {
                    return Some(direct(entry));
                }
                if let Some(program) = dirs.iter().find_map(|path| executable(path, "node")) {
                    return Some(Executable {
                        program,
                        entry: EntryPoint::Node(entry),
                    });
                }
            }
        }
    }
    None
}

pub fn command(id: &str) -> Result<Command, AppError> {
    resolve(id).map(|path| path.command()).ok_or_else(|| {
        AppError::Agent(format!(
            "Could not find {id}. Install its CLI on PATH{}.",
            if id == "nova" { " or set NOVA_BIN" } else { "" }
        ))
    })
}

pub fn nova_python() -> Result<Command, AppError> {
    let python = python_name();
    let path = std::env::var_os("NOVA_PYTHON")
        .map(PathBuf::from)
        .filter(|path| path.is_absolute() && path.is_file())
        .or_else(|| {
            resolve("nova").and_then(|entry| match entry.entry {
                EntryPoint::NovaSource(_) => Some(entry.program),
                _ => uv_script_python(&entry.program).or_else(|| {
                    entry
                        .program
                        .canonicalize()
                        .ok()
                        .and_then(|path| path.parent().and_then(|dir| executable(dir, python)))
                }),
            })
        })
        .or_else(|| resolve(python).map(|entry| entry.program))
        .ok_or_else(|| AppError::Agent("Could not find Nova's Python runtime".into()))?;
    Ok(Executable {
        program: path,
        entry: EntryPoint::Binary,
    }
    .command())
}

fn uv_script_python(path: &Path) -> Option<PathBuf> {
    use std::io::Read;
    let mut data = Vec::new();
    std::fs::File::open(path)
        .ok()?
        .take(1024 * 1024)
        .read_to_end(&mut data)
        .ok()?;
    let trailer = data.len().checked_sub(8)?;
    if &data[trailer + 4..] != b"UVSC" {
        return None;
    }
    let length = u32::from_le_bytes(data[trailer..trailer + 4].try_into().ok()?) as usize;
    let interpreter =
        PathBuf::from(std::str::from_utf8(data.get(trailer.checked_sub(length)?..trailer)?).ok()?);
    (interpreter.is_absolute()
        && interpreter
            .file_name()?
            .to_str()?
            .eq_ignore_ascii_case("python.exe"))
    .then_some(interpreter)
}

pub fn nova_personality(profile: &super::Personality) -> Result<Command, AppError> {
    let checkout = nova_checkout();
    let source = (checkout.is_absolute() && checkout.join("novacode_cli/entrypoint.py").is_file())
        .then_some(checkout);
    let mut command = nova_python()?;
    command
        .args(["-I", "-B", "-c", include_str!("personality.py")])
        .arg(source.as_deref().unwrap_or_else(|| Path::new("")))
        .env("NOVA_DESKTOP_PERSONALITY", &profile.role)
        .env("NOVA_DESKTOP_SYSTEM_PROMPT", &profile.system_prompt);
    Ok(command)
}

fn python_name() -> &'static str {
    if cfg!(windows) {
        "python"
    } else {
        "python3"
    }
}

pub fn terminal_python() -> Result<Command, AppError> {
    if !cfg!(windows) {
        return Err(AppError::Agent(
            "The interactive dock uses Windows ConPTY".into(),
        ));
    }
    let directories = std::env::var_os("PATH").unwrap_or_default();
    for dir in std::env::split_paths(&directories).filter(|dir| dir.is_absolute()) {
        let site = dir.join("Lib/site-packages");
        if site.join("winpty").is_dir() && site.join("pyte").is_dir() {
            if let Some(program) = executable(&dir, "python") {
                return Ok(Executable {
                    program,
                    entry: EntryPoint::Binary,
                }
                .command());
            }
        }
    }
    nova_python()
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentStatus {
    id: &'static str,
    name: &'static str,
    installed: bool,
    version: Option<String>,
    latest: Option<String>,
    update_available: bool,
    executable: Option<String>,
}

pub async fn statuses(versions: bool, safe_cwd: &Path) -> Vec<AgentStatus> {
    let mut jobs = tokio::task::JoinSet::new();
    for (index, &(id, name)) in AGENTS.iter().enumerate() {
        let cwd = safe_cwd.to_path_buf();
        jobs.spawn(async move {
            let path = resolve(id);
            let installed_version = async {
                if let Some(path) = &path {
                    tokio::time::timeout(
                        Duration::from_secs(7),
                        path.command().current_dir(cwd).arg("--version").output(),
                    )
                    .await
                    .ok()
                    .and_then(Result::ok)
                    .filter(|output| output.status.success())
                    .and_then(|output| {
                        String::from_utf8_lossy(&output.stdout)
                            .lines()
                            .rfind(|line| !line.trim().is_empty())
                            .map(|line| clip(line.trim(), 150))
                    })
                } else {
                    None
                }
            };
            let (version, latest) = if versions {
                tokio::join!(installed_version, latest_version(id))
            } else {
                (None, None)
            };
            let version_number = |version: &str| {
                version
                    .split_whitespace()
                    .find_map(|part| semver::Version::parse(part.trim_start_matches('v')).ok())
            };
            let update_available = version
                .as_deref()
                .and_then(version_number)
                .zip(latest.as_deref().and_then(version_number))
                .is_some_and(|(version, latest)| latest > version);
            (
                index,
                AgentStatus {
                    id,
                    name,
                    installed: path.is_some(),
                    version,
                    latest,
                    update_available,
                    executable: path.as_ref().map(Executable::display),
                },
            )
        });
    }
    let mut statuses = Vec::new();
    while let Some(Ok(status)) = jobs.join_next().await {
        statuses.push(status);
    }
    statuses.sort_by_key(|(index, _)| *index);
    statuses.into_iter().map(|(_, status)| status).collect()
}

async fn latest_version(id: &str) -> Option<String> {
    let (url, field) = match id {
        "nova" => ("https://pypi.org/pypi/novacode-cli/json", "/info/version"),
        "pi" => (
            "https://registry.npmjs.org/@earendil-works%2fpi-coding-agent/latest",
            "/version",
        ),
        "claude" => (
            "https://registry.npmjs.org/@anthropic-ai%2fclaude-code/latest",
            "/version",
        ),
        "codex" => (
            "https://registry.npmjs.org/@openai%2fcodex/latest",
            "/version",
        ),
        "opencode" => (
            "https://registry.npmjs.org/@opencode%2fcli/latest",
            "/version",
        ),
        "commandcode" => ("https://registry.npmjs.org/command-code/latest", "/version"),
        "hermes" => (
            "https://api.github.com/repos/NousResearch/hermes-agent/releases/latest",
            "/tag_name",
        ),
        _ => return None,
    };
    let data = tokio::time::timeout(
        Duration::from_secs(8),
        crate::commands::settings::download(url, 4 * 1024 * 1024),
    )
    .await
    .ok()?
    .ok()?;
    let release: Value = serde_json::from_slice(&data).ok()?;
    release.pointer(field)?.as_str().map(str::to_owned)
}

pub struct CliAgent(pub &'static str);

struct CommandCodePromptMod(PathBuf);

impl CommandCodePromptMod {
    fn write() -> Result<Self, AppError> {
        use std::io::Write;
        let path =
            std::env::temp_dir().join(format!("nova-personality-{}.ts", uuid::Uuid::new_v4()));
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
            .map_err(|error| AppError::Io(error.to_string()))?;
        let prompt_mod = Self(path);
        let result = file.write_all(include_bytes!("commandcode-personality.ts"));
        drop(file);
        result.map_err(|error| AppError::Io(error.to_string()))?;
        Ok(prompt_mod)
    }
}

impl Drop for CommandCodePromptMod {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

impl AgentBackend for CliAgent {
    fn start<'a>(
        &'a self,
        req: TurnRequest,
        sink: EventSink,
        cancel: CancellationToken,
    ) -> BoxFut<'a, Result<FinishReason, AppError>> {
        Box::pin(async move {
            let root = req.workspace.as_ref().ok_or(AppError::NoWorkspace)?.root();
            let (args, stdin_prompt) = arguments(self.0, &req);
            let mut process = command(self.0)?;
            let mut prompt_mod = None;
            if let Some(profile) = &req.personality {
                match self.0 {
                    "commandcode" => {
                        let module = CommandCodePromptMod::write()?;
                        process
                            .arg("--mod")
                            .arg(&module.0)
                            .env("NOVA_DESKTOP_SYSTEM_PROMPT", &profile.system_prompt);
                        prompt_mod = Some(module);
                    }
                    "hermes" => {
                        process.env("HERMES_EPHEMERAL_SYSTEM_PROMPT", &profile.system_prompt);
                    }
                    "opencode" => {
                        let raw = std::env::var("OPENCODE_CONFIG_CONTENT")
                            .unwrap_or_else(|_| "{}".into());
                        let mut config: Value = serde_json::from_str(&raw).map_err(|_| {
                            AppError::Agent(
                                "OpenCode inline configuration must be JSON for personalities"
                                    .into(),
                            )
                        })?;
                        if !config.is_object()
                            || config.get("agent").is_some_and(|value| !value.is_object())
                        {
                            return Err(AppError::Agent(
                                "Invalid OpenCode inline configuration".into(),
                            ));
                        }
                        config["agent"]["nova-desktop-personality"] = json!({
                            "mode": "primary", "prompt": &profile.system_prompt
                        });
                        process.env("OPENCODE_CONFIG_CONTENT", config.to_string());
                    }
                    _ => {}
                }
            }
            let mut child = process
                .args(args)
                .current_dir(root)
                .env("NO_COLOR", "1")
                .stdin(if stdin_prompt {
                    Stdio::piped()
                } else {
                    Stdio::null()
                })
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()
                .map_err(|err| AppError::Agent(format!("Could not start {}: {err}", self.0)))?;

            let mut stdout = BufReader::new(child.stdout.take().expect("piped stdout"));
            let stderr = child.stderr.take().expect("piped stderr");
            let diagnostics = tokio::spawn(async move {
                let mut stderr = BufReader::new(stderr);
                let mut tail = Vec::new();
                let mut buffer = [0_u8; 4096];
                while let Ok(count @ 1..) = stderr.read(&mut buffer).await {
                    tail.extend_from_slice(&buffer[..count]);
                    if tail.len() > 8192 {
                        tail.drain(..tail.len() - 8192);
                    }
                }
                String::from_utf8_lossy(&tail).trim().to_owned()
            });
            let mut frames = Frames::default();
            let result = tokio::select! {
                biased;
                _ = cancel.cancelled() => None,
                result = async {
                    if let Some(mut stdin) = child.stdin.take() {
                        use tokio::io::AsyncWriteExt;
                        stdin.write_all(req.prompt.as_bytes()).await?;
                    }
                    let mut line = Vec::new();
                    loop {
                        line.clear();
                        if (&mut stdout).take(FRAME_LIMIT + 1).read_until(b'\n', &mut line).await? == 0 {
                            break;
                        }
                        if line.len() as u64 > FRAME_LIMIT {
                            return Err(std::io::Error::new(std::io::ErrorKind::InvalidData, "agent frame exceeds 8 MiB"));
                        }
                        let raw = String::from_utf8_lossy(&line);
                        if let Ok(frame) = serde_json::from_str::<Value>(&raw) {
                            handle_frame(self.0, &frame, &sink, &mut frames);
                        }
                    }
                    child.wait().await
                } => Some(result),
            };
            let Some(result) = result else {
                let _ = child.kill().await;
                diagnostics.abort();
                return Ok(FinishReason::Cancelled);
            };
            drop(prompt_mod);
            if result.is_err() {
                diagnostics.abort();
            }
            let status = result.map_err(|err| AppError::Agent(err.to_string()))?;
            let diagnostics = tokio::time::timeout(Duration::from_secs(1), diagnostics)
                .await
                .ok()
                .and_then(Result::ok)
                .unwrap_or_default();
            if frames.cancelled {
                return Ok(FinishReason::Cancelled);
            }
            if !status.success() || frames.failure.is_some() || !frames.sent {
                let detail = frames
                    .failure
                    .filter(|s| !s.is_empty())
                    .or_else(|| (!diagnostics.is_empty()).then_some(diagnostics))
                    .unwrap_or_else(|| {
                        if status.success() {
                            "No reply was returned.".into()
                        } else {
                            status.to_string()
                        }
                    });
                return Err(AppError::Agent(format!(
                    "{}: {}",
                    self.0,
                    clip(&detail, 3000)
                )));
            }
            Ok(FinishReason::Completed)
        })
    }
}

fn arguments(id: &str, req: &TurnRequest) -> (Vec<String>, bool) {
    let session = req.session.as_deref().filter(|id| valid_session(id));
    let mut args: Vec<String> = match id {
        "pi" => vec!["--mode", "json", "--print", "--session-id", &req.chat_id],
        "claude" => vec![
            "--print",
            "--output-format",
            "stream-json",
            "--verbose",
            "--permission-mode",
            "default",
        ],
        "codex" => vec![
            "--ask-for-approval",
            "never",
            "exec",
            "--json",
            "--skip-git-repo-check",
            "-c",
            "sandbox_mode=\"workspace-write\"",
        ],
        "opencode" => vec!["run", "--format", "json"],
        "commandcode" => vec![
            "--print",
            "--output-format",
            "json",
            "--permission-mode",
            "default",
            "--no-auto-update",
        ],
        "hermes" => vec![
            "chat",
            "--format",
            "stream-json",
            "-c",
            &req.chat_id,
            "--create-if-missing",
        ],
        _ => unreachable!(),
    }
    .into_iter()
    .map(str::to_owned)
    .collect();
    if let Some(model) = req.model.as_deref() {
        args.extend(["--model".to_owned(), model.to_owned()]);
    }
    if let Some(profile) = &req.personality {
        match id {
            "pi" | "claude" => args.extend([
                "--append-system-prompt".into(),
                profile.system_prompt.clone(),
            ]),
            "codex" => args.extend([
                "-c".into(),
                format!("developer_instructions={}", json!(profile.system_prompt)),
            ]),
            "opencode" => args.extend(["--agent".into(), "nova-desktop-personality".into()]),
            _ => {}
        }
    }
    if let Some(effort) = req.effort.as_deref() {
        match id {
            "pi" => args.extend(["--thinking".into(), effort.into()]),
            "claude" | "commandcode" => args.extend(["--effort".into(), effort.into()]),
            "codex" => args.extend(["-c".into(), format!("model_reasoning_effort=\"{effort}\"")]),
            _ => {}
        }
    }
    if let Some(session) = session {
        match id {
            "claude" => args.extend(["--resume".into(), session.into()]),
            "codex" => args.extend(["resume".into(), session.into()]),
            "opencode" | "commandcode" => args.extend(["--session".into(), session.into()]),
            _ => {}
        }
    }
    let use_stdin = matches!(id, "claude" | "codex");
    match id {
        "claude" => {}
        "codex" => args.push("-".into()),
        "hermes" => args.push(format!("--query={}", req.prompt)),
        _ => {
            args.push("--".into());
            // Pi treats a leading @ as an attachment argument even after --.
            args.push(if id == "pi" && req.prompt.starts_with('@') {
                format!("\n{}", req.prompt)
            } else {
                req.prompt.clone()
            });
        }
    }
    (args, use_stdin)
}

fn valid_session(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 128
        && id
            .bytes()
            .next()
            .is_some_and(|byte| byte.is_ascii_alphanumeric())
        && id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b':'))
}

fn clip(s: &str, limit: usize) -> String {
    s.chars().take(limit).collect()
}

#[derive(Default)]
struct Frames {
    sent: bool,
    cancelled: bool,
    failure: Option<String>,
    session: Option<String>,
    tools: HashMap<String, VecDeque<String>>,
}

fn handle_frame(id: &str, frame: &Value, sink: &EventSink, state: &mut Frames) {
    let kind = frame["type"].as_str().unwrap_or_default();
    let sent = &mut state.sent;
    let failure = &mut state.failure;
    let first = !*sent;
    let mut text = |value: &Value| {
        if let Some(value) = value.as_str().filter(|value| !value.is_empty()) {
            *sent = true;
            let _ = sink.send(AgentEvent::MessageDelta { text: value.into() });
        }
    };
    let call = |key: &Value, name: &Value, input: &Value| {
        if let Some(key) = key.as_str() {
            let _ = sink.send(AgentEvent::ToolCall {
                call_id: key.into(),
                name: name.as_str().unwrap_or("Tool").into(),
                input: clip(&input.to_string(), 8_000),
            });
        }
    };
    let result = |key: &Value, ok: bool, output: &Value| {
        if let Some(key) = key.as_str() {
            let output = output
                .as_str()
                .map(str::to_owned)
                .unwrap_or_else(|| output.to_string());
            let _ = sink.send(AgentEvent::ToolResult {
                call_id: key.into(),
                ok,
                output: clip(&output, 32_000),
            });
        }
    };
    let session = match id {
        "codex" => &frame["thread_id"],
        "opencode" => &frame["sessionID"],
        "commandcode" => &frame["sessionId"],
        _ => &frame["session_id"],
    };
    if let Some(session) = session
        .as_str()
        .filter(|session| valid_session(session) && Some(*session) != state.session.as_deref())
    {
        state.session = Some(session.into());
        let _ = sink.send(AgentEvent::Session { id: session.into() });
    }
    let usage = match (id, kind) {
        ("codex", "turn.completed") | ("claude", "result") => Some((
            &frame["usage"]["input_tokens"],
            &frame["usage"]["output_tokens"],
        )),
        ("hermes", "result") => Some((&frame["tokens"]["input"], &frame["tokens"]["output"])),
        ("commandcode", "result") => Some((
            &frame["usage"]["inputTokens"],
            &frame["usage"]["outputTokens"],
        )),
        _ => None,
    };
    if let Some((Some(input), Some(output))) =
        usage.map(|(input, output)| (input.as_u64(), output.as_u64()))
    {
        let _ = sink.send(AgentEvent::Usage { input, output });
    }
    match (id, kind) {
        ("commandcode", "event") => {
            let event = &frame["event"];
            match event["type"].as_str() {
                Some("text_delta") => text(&event["delta"]),
                Some("tool_running") => call(
                    &event["toolCallId"],
                    &event["toolName"],
                    &event["description"],
                ),
                Some("tool_completed") => result(
                    &event["toolCallId"],
                    event["isError"] != true,
                    &event["result"],
                ),
                Some("tool_errored" | "tool_denied" | "tool_hook_blocked") => result(
                    &event["toolCallId"],
                    false,
                    event.get("error").unwrap_or(&event["reason"]),
                ),
                _ => {}
            }
        }
        ("commandcode", "result") => {
            if first {
                text(&frame["finalText"]);
            }
            if frame["subtype"] != "success" {
                *failure = Some(
                    frame["error"]
                        .as_str()
                        .or_else(|| frame["subtype"].as_str())
                        .unwrap_or("CommandCode turn failed")
                        .into(),
                );
            }
        }
        ("hermes", "text") => text(&frame["text"]),
        ("hermes", "tool_use") => {
            if let Some(name) = frame["name"].as_str() {
                let key = uuid::Uuid::new_v4().to_string();
                state
                    .tools
                    .entry(name.into())
                    .or_default()
                    .push_back(key.clone());
                call(&json!(key), &frame["name"], &frame["input"]);
            }
        }
        ("hermes", "tool_result") => {
            if let Some(key) = frame["name"]
                .as_str()
                .and_then(|name| state.tools.get_mut(name))
                .and_then(VecDeque::pop_front)
            {
                result(&json!(key), frame["is_error"] != true, &frame["output"]);
            }
        }
        ("hermes", "result") => {
            if first {
                text(&frame["text"]);
            }
            match frame["exit_code"].as_i64() {
                Some(130) => state.cancelled = true,
                Some(0) | None => {}
                _ => {
                    *failure = Some(
                        frame["error"]
                            .as_str()
                            .unwrap_or("Hermes turn failed")
                            .into(),
                    )
                }
            }
        }
        ("pi", "message_update") if frame["assistantMessageEvent"]["type"] == "text_delta" => {
            text(&frame["assistantMessageEvent"]["delta"])
        }
        ("pi", "tool_execution_start") => {
            call(&frame["toolCallId"], &frame["toolName"], &frame["args"])
        }
        ("pi", "tool_execution_end") => result(
            &frame["toolCallId"],
            frame["isError"] != true,
            &frame["result"],
        ),
        ("pi", "message_end") if frame["message"]["stopReason"] == "error" => {
            *failure = frame["message"]["errorMessage"].as_str().map(str::to_owned)
        }
        ("claude", "assistant") => {
            if let Some(blocks) = frame["message"]["content"].as_array() {
                for block in blocks {
                    match block["type"].as_str() {
                        Some("text") => text(&block["text"]),
                        Some("tool_use") => call(&block["id"], &block["name"], &block["input"]),
                        _ => {}
                    }
                }
            }
        }
        ("claude", "user") => {
            if let Some(blocks) = frame["message"]["content"].as_array() {
                for block in blocks.iter().filter(|block| block["type"] == "tool_result") {
                    result(
                        &block["tool_use_id"],
                        block["is_error"] != true,
                        &block["content"],
                    );
                }
            }
        }
        ("codex", "item.started" | "item.completed") => {
            let item = &frame["item"];
            let completed = kind == "item.completed";
            let label = &item["type"];
            match label.as_str() {
                Some("agent_message") if completed => text(&item["text"]),
                Some("command_execution" | "file_change" | "mcp_tool_call" | "web_search") => {
                    if !completed || label == "file_change" {
                        call(
                            &item["id"],
                            label,
                            &item["command"]
                                .as_str()
                                .map_or_else(|| item.clone(), |v| json!(v)),
                        );
                    }
                    if completed {
                        result(
                            &item["id"],
                            item["status"] != "failed"
                                && item["exit_code"].as_i64().unwrap_or(0) == 0,
                            item.get("aggregated_output")
                                .or_else(|| item.get("result"))
                                .unwrap_or(item),
                        );
                    }
                }
                _ => {}
            }
        }
        ("opencode", "text") => text(&frame["part"]["text"]),
        ("opencode", "tool_use") => {
            let part = &frame["part"];
            let state = &part["state"];
            call(&part["callID"], &part["tool"], &state["input"]);
            result(
                &part["callID"],
                state["status"] == "completed",
                state.get("output").unwrap_or(&state["error"]),
            );
        }
        (_, "error" | "turn.failed") | (_, "result")
            if kind != "result" || frame["is_error"] == true =>
        {
            *failure = Some(
                frame["message"]
                    .as_str()
                    .or_else(|| frame["error"]["message"].as_str())
                    .or_else(|| frame["result"].as_str())
                    .map(str::to_owned)
                    .unwrap_or_else(|| frame.to_string()),
            );
        }
        (_, "text_delta" | "assistant.delta") => text(
            frame
                .get("text")
                .or_else(|| frame.get("content"))
                .unwrap_or(&frame["delta"]),
        ),
        (_, "result" | "final" | "response") if first => {
            let value = frame
                .get("result")
                .or_else(|| frame.get("response"))
                .unwrap_or(&frame["text"]);
            text(value);
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::events::testutil::collect_events;

    #[test]
    fn resolves_the_python_used_by_a_uv_script_launcher() {
        let root = crate::testtmp::TempDir::new("uv-launcher");
        let launcher = root.path().join("nova.exe");
        let python = root.path().join("python.exe");
        let path = python.to_str().unwrap().as_bytes();
        let mut data = b"launcher stub".to_vec();
        data.extend(path);
        data.extend((path.len() as u32).to_le_bytes());
        data.extend(b"UVSC");
        std::fs::write(&launcher, &data).unwrap();
        assert_eq!(uv_script_python(&launcher), Some(python));
        let trailer = data.len() - 8;
        data[trailer..trailer + 4].copy_from_slice(&u32::MAX.to_le_bytes());
        std::fs::write(&launcher, &data).unwrap();
        assert_eq!(uv_script_python(&launcher), None);
    }

    #[test]
    fn model_and_effort_reach_the_harness() {
        let request = TurnRequest {
            prompt: "Hello".into(),
            workspace: None,
            chat_id: "task".into(),
            model: Some("provider/model".into()),
            effort: Some("high".into()),
            session: None,
            trust_workspace: false,
            personality: None,
        };
        let (pi, _) = arguments("pi", &request);
        assert!(pi
            .windows(2)
            .any(|pair| pair == ["--model", "provider/model"]));
        assert!(pi.windows(2).any(|pair| pair == ["--thinking", "high"]));
        let (codex, _) = arguments("codex", &request);
        assert!(codex
            .windows(2)
            .any(|pair| pair == ["-c", "model_reasoning_effort=\"high\""]));
    }

    #[test]
    fn personality_instructions_are_separate_from_the_user_prompt() {
        let mut request = TurnRequest {
            prompt: "Find the caller".into(),
            workspace: None,
            chat_id: "task".into(),
            model: None,
            effort: None,
            session: None,
            trust_workspace: true,
            personality: Some(super::super::Personality {
                role: "serena".into(),
                system_prompt: "Stay in scope.".into(),
            }),
        };
        let profile = request.personality.as_ref().unwrap();
        assert!(AGENTS
            .iter()
            .all(|&(backend, _)| profile.validate(backend).is_ok()));
        assert!(profile.validate("local").is_err());
        let (pi, _) = arguments("pi", &request);
        assert!(pi
            .windows(2)
            .any(|pair| pair == ["--append-system-prompt", "Stay in scope."]));
        assert_eq!(pi.last().unwrap(), "Find the caller");
        request.personality.as_mut().unwrap().role = "nora".into();
        let (codex, _) = arguments("codex", &request);
        assert!(codex
            .windows(2)
            .any(|pair| pair == ["-c", "developer_instructions=\"Stay in scope.\""]));
    }

    #[test]
    fn commandcode_personality_mod_is_removed_after_use() {
        let module = CommandCodePromptMod::write().unwrap();
        let path = module.0.clone();
        assert!(std::fs::read_to_string(&path)
            .unwrap()
            .contains("appendSystemPrompt"));
        drop(module);
        assert!(!path.exists());
    }

    #[test]
    fn commandcode_streams_tools_and_preserves_exact_session() {
        let (sink, events) = collect_events();
        let mut state = Frames::default();
        for event in [
            json!({"type":"event","event":{"type":"tool_running","toolCallId":"t1","toolName":"read_file","description":"README"}}),
            json!({"type":"event","event":{"type":"tool_completed","toolCallId":"t1","result":"README contents"}}),
            json!({"type":"event","event":{"type":"text_delta","delta":"Answer"}}),
            json!({"type":"result","subtype":"success","sessionId":"specific-session","finalText":"Answer"}),
        ] {
            handle_frame("commandcode", &event, &sink, &mut state);
        }
        assert!(state.failure.is_none());
        let events = events.lock().unwrap();
        assert!(
            matches!(&events[1], AgentEvent::ToolResult { call_id, ok: true, .. } if call_id == "t1")
        );
        assert_eq!(
            events[2],
            AgentEvent::MessageDelta {
                text: "Answer".into()
            }
        );
        assert_eq!(
            events[3],
            AgentEvent::Session {
                id: "specific-session".into()
            }
        );
        assert_eq!(events.len(), 4);
    }

    #[test]
    fn hermes_correlates_parallel_calls_and_records_failed_exit() {
        let (sink, events) = collect_events();
        let mut state = Frames::default();
        for event in [
            json!({"type":"tool_use","name":"read_file"}),
            json!({"type":"tool_use","name":"read_file"}),
            json!({"type":"tool_result","name":"read_file","output":"missing","is_error":true}),
            json!({"type":"result","session_id":"s1","exit_code":1,"error":"provider unavailable"}),
        ] {
            handle_frame("hermes", &event, &sink, &mut state);
        }
        let events = events.lock().unwrap();
        assert!(matches!((&events[0], &events[2]),
            (AgentEvent::ToolCall { call_id: call, .. }, AgentEvent::ToolResult { call_id: result, ok: false, .. }) if call == result));
        assert_eq!(state.failure.as_deref(), Some("provider unavailable"));
    }

    #[test]
    fn supported_cli_frames_keep_the_assistant_reply() {
        for (agent, event) in [
            (
                "pi",
                json!({"type":"message_update","assistantMessageEvent":{"type":"text_delta","delta":"hello"}}),
            ),
            (
                "claude",
                json!({"type":"assistant","message":{"content":[{"type":"text","text":"hello"}]}}),
            ),
            (
                "codex",
                json!({"type":"item.completed","item":{"type":"agent_message","text":"hello"}}),
            ),
            ("opencode", json!({"type":"text","part":{"text":"hello"}})),
        ] {
            let (sink, events) = collect_events();
            let mut state = Frames::default();
            handle_frame(agent, &event, &sink, &mut state);
            assert!(state.sent, "{agent}");
            assert_eq!(
                *events.lock().unwrap(),
                [AgentEvent::MessageDelta {
                    text: "hello".into()
                }]
            );
        }
    }
}
