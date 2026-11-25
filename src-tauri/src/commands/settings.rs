use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use serde::Serialize;
use serde_json::Value;
use tauri::{AppHandle, Manager, State, Url};
use tauri_plugin_dialog::DialogExt;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::process::Command;

use crate::{agent::cli, error::AppError, AppState};

const THEME_LIMIT: u64 = 20 * 1024 * 1024;

fn failure(error: impl ToString) -> AppError {
    AppError::Agent(error.to_string())
}

pub fn system_command(name: &str) -> Command {
    #[cfg(windows)]
    let program = std::env::var_os("SystemRoot")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(r"C:\Windows"))
        .join("System32")
        .join(format!("{name}.exe"));
    #[cfg(not(windows))]
    let program = Path::new("/usr/bin").join(name);
    let mut command = Command::new(program);
    #[cfg(windows)]
    command.creation_flags(0x08000000);
    command
}

pub async fn capture(mut command: Command, max: u64) -> Result<Vec<u8>, AppError> {
    let mut child = command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .spawn()
        .map_err(failure)?;
    let mut data = Vec::new();
    tokio::time::timeout(Duration::from_secs(30), async {
        child
            .stdout
            .take()
            .expect("piped stdout")
            .take(max + 1)
            .read_to_end(&mut data)
            .await
            .map_err(failure)?;
        if data.len() as u64 > max {
            return Err(failure("The output exceeded its size limit"));
        }
        if !child.wait().await.map_err(failure)?.success() {
            return Err(failure("The command did not complete successfully"));
        }
        Ok(data)
    })
    .await
    .map_err(|_| failure("The operation timed out"))?
}

pub async fn download(url: &str, limit: u64) -> Result<Vec<u8>, AppError> {
    let mut command = system_command("curl");
    command.args([
        "-q",
        "--silent",
        "--fail",
        "--user-agent",
        "NovaDesktop",
        "--location",
        "--max-redirs",
        "3",
        "--proto",
        "=https",
        "--proto-redir",
        "=https",
        "--max-time",
        "25",
        "--max-filesize",
        &limit.to_string(),
        "--",
        url,
    ]);
    capture(command, limit).await
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ColorTheme {
    id: String,
    title: String,
    mode: &'static str,
    colors: Value,
}

#[tauri::command]
pub async fn import_theme(app: AppHandle) -> Result<Vec<ColorTheme>, AppError> {
    let (tx, rx) = tokio::sync::oneshot::channel();
    app.dialog()
        .file()
        .add_filter("VS Code theme extension", &["vsix"])
        .pick_file(move |path| {
            let _ = tx.send(path);
        });
    let Some(path) = rx.await.map_err(failure)? else {
        return Ok(Vec::new());
    };
    read_vsix(&path.into_path().map_err(failure)?).await
}

#[tauri::command]
pub async fn install_theme(id: String, app: AppHandle) -> Result<Vec<ColorTheme>, AppError> {
    let valid = |field: &str| {
        !field.is_empty()
            && field
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_')
    };
    let (publisher, name) = id
        .split_once('.')
        .filter(|(publisher, name)| id.len() <= 160 && valid(publisher) && valid(name))
        .ok_or_else(|| failure("Enter a publisher.extension ID"))?;
    let metadata = download(
        &format!("https://open-vsx.org/api/{publisher}/{name}/latest"),
        1024 * 1024,
    )
    .await
    .map_err(|_| failure("Could not find this extension on Open VSX"))?;
    let metadata: Value = serde_json::from_slice(&metadata).map_err(failure)?;
    let url: Url = metadata["files"]["download"]
        .as_str()
        .unwrap_or_default()
        .parse()
        .map_err(failure)?;
    if url.scheme() != "https"
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
    {
        return Err(failure("Open VSX returned an invalid download URL"));
    }
    let archive = download(url.as_str(), THEME_LIMIT).await?;
    let directory = app.path().app_cache_dir().map_err(failure)?;
    std::fs::create_dir_all(&directory).map_err(failure)?;
    let path = directory.join(format!("{}.vsix", uuid::Uuid::new_v4()));
    let result = match std::fs::write(&path, archive) {
        Ok(()) => read_vsix(&path).await,
        Err(error) => Err(failure(error)),
    };
    let _ = std::fs::remove_file(path);
    result
}

async fn read_vsix(path: &Path) -> Result<Vec<ColorTheme>, AppError> {
    if std::fs::metadata(path).map_err(failure)?.len() > THEME_LIMIT {
        return Err(failure("Theme archives are limited to 20 MiB"));
    }
    let manifest = archive_json(path, "extension/package.json").await?;
    let themes = manifest["contributes"]["themes"]
        .as_array()
        .ok_or_else(|| failure("The extension contains no VS Code color themes"))?;
    let mut result = Vec::new();
    for theme in themes.iter().take(24) {
        let (Some(title), Some(file)) = (theme["label"].as_str(), theme["path"].as_str()) else {
            continue;
        };
        let Some(member) = member_path("extension", file) else {
            continue;
        };
        let colors = theme_colors(path, member).await?;
        if colors.as_object().is_some_and(|colors| !colors.is_empty()) {
            result.push(ColorTheme {
                id: format!(
                    "{}.{}:{title}",
                    manifest["publisher"].as_str().unwrap_or("local"),
                    manifest["name"].as_str().unwrap_or("theme")
                ),
                title: title.chars().take(120).collect(),
                mode: if matches!(theme["uiTheme"].as_str(), Some("vs" | "hc-light")) {
                    "light"
                } else {
                    "dark"
                },
                colors,
            });
        }
    }
    if result.is_empty() {
        return Err(failure(
            "No usable workbench colors were found in this theme",
        ));
    }
    Ok(result)
}

fn member_path(parent: &str, relative: &str) -> Option<String> {
    if relative.starts_with('/') || relative.contains(['\\', ':', '*', '?', '[', ']', '\0']) {
        return None;
    }
    let mut parts: Vec<&str> = parent.split('/').collect();
    for part in relative.split('/') {
        match part {
            "" | "." => {}
            ".." if parts.len() > 1 => {
                parts.pop();
            }
            ".." => return None,
            part => parts.push(part),
        }
    }
    (parts.len() > 1).then(|| parts.join("/"))
}

async fn theme_colors(path: &Path, mut member: String) -> Result<Value, AppError> {
    let mut colors = serde_json::Map::new();
    for _ in 0..8 {
        let document = archive_json(path, &member).await?;
        if let Some(theme_colors) = document["colors"].as_object() {
            for (key, color) in theme_colors {
                if let Some(hex) = color.as_str().filter(|color| valid_color(color)) {
                    colors
                        .entry(key)
                        .or_insert_with(|| Value::String(hex.into()));
                }
            }
        }
        let Some(include) = document["include"].as_str() else {
            return Ok(Value::Object(colors));
        };
        let parent = member
            .rsplit_once('/')
            .map_or("extension", |(parent, _)| parent);
        member = member_path(parent, include)
            .ok_or_else(|| failure("Theme has an invalid include path"))?;
    }
    Err(failure("Theme includes are nested too deeply"))
}

fn valid_color(color: &str) -> bool {
    matches!(color.len(), 4 | 5 | 7 | 9)
        && color.starts_with('#')
        && color[1..].bytes().all(|byte| byte.is_ascii_hexdigit())
}

async fn archive_json(path: &Path, member: &str) -> Result<Value, AppError> {
    #[cfg(windows)]
    let mut command = {
        let mut command = system_command("tar");
        command.arg("-xOf").arg(path).arg("--");
        command
    };
    #[cfg(not(windows))]
    let mut command = {
        let mut command = system_command("unzip");
        command.arg("-p").arg(path);
        command
    };
    command.arg(member);
    let data = capture(command, 2 * 1024 * 1024)
        .await
        .map_err(|_| failure(format!("Could not read {member} from the VSIX")))?;
    let text = String::from_utf8(data).map_err(failure)?;
    serde_json::from_str(&jsonc(text.trim_start_matches('\u{feff}'))).map_err(failure)
}

// VS Code theme data allows JavaScript-style comments and trailing commas.
fn jsonc(text: &str) -> String {
    let mut chars = text.chars().peekable();
    let mut clean = String::with_capacity(text.len());
    let mut in_string = false;
    let mut comma = None;
    while let Some(ch) = chars.next() {
        if in_string {
            clean.push(ch);
            if ch == '\\' {
                if let Some(escaped) = chars.next() {
                    clean.push(escaped);
                }
            } else if ch == '"' {
                in_string = false;
            }
            continue;
        }
        if ch == '/' && matches!(chars.peek(), Some('/' | '*')) {
            if chars.next() == Some('/') {
                for next in chars.by_ref() {
                    if next == '\n' {
                        break;
                    }
                }
            } else {
                while let Some(next) = chars.next() {
                    if next == '*' && chars.peek() == Some(&'/') {
                        chars.next();
                        break;
                    }
                }
            }
            clean.push(' ');
            continue;
        }
        if ch.is_whitespace() {
            clean.push(ch);
            continue;
        }
        if let Some(index) = comma.take() {
            if matches!(ch, ']' | '}') {
                clean.remove(index);
            }
        }
        if ch == ',' {
            comma = Some(clean.len());
        }
        if ch == '"' {
            in_string = true;
        }
        clean.push(ch);
    }
    clean
}

#[tauri::command]
pub async fn nova_mcp_list(state: State<'_, AppState>) -> Result<String, AppError> {
    let guard = state.require_guard().await?;
    let mut command = cli::command("nova")?;
    command
        .current_dir(guard.root())
        .env("NO_COLOR", "1")
        .args(["mcp", "list"]);
    let output = capture(command, 128 * 1024)
        .await
        .map_err(|_| failure("Could not list Nova MCP servers in this workspace"))?;
    Ok(String::from_utf8_lossy(&output).trim().to_owned())
}

#[tauri::command]
pub async fn add_mcp_server(
    name: String,
    endpoint: String,
    state: State<'_, AppState>,
) -> Result<(), AppError> {
    if name.len() > 64
        || !name
            .bytes()
            .next()
            .is_some_and(|byte| byte.is_ascii_alphanumeric())
        || !name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
    {
        return Err(failure(
            "Use letters, numbers, hyphens, or underscores for the server name",
        ));
    }
    let url: Url = endpoint
        .parse()
        .map_err(|_| failure("Enter the server’s HTTP or HTTPS URL"))?;
    if !matches!(url.scheme(), "http" | "https")
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err(failure(
            "Enter an HTTP or HTTPS endpoint without credentials, query parameters, or fragments",
        ));
    }
    let guard = state.require_guard().await?;
    let mut command = cli::command("nova")?;
    command
        .current_dir(guard.root())
        .env("NO_COLOR", "1")
        .args([
            "mcp",
            "add",
            &name,
            "--transport",
            "http",
            "--url",
            url.as_str(),
        ]);
    capture(command, 128 * 1024).await.map_err(|_| {
        failure("Nova could not add this server. Check the name, endpoint, and Nova setup.")
    })?;
    Ok(())
}

#[tauri::command]
pub async fn open_provider_setup(
    id: String,
    state: State<'_, AppState>,
    app: AppHandle,
) -> Result<(), AppError> {
    let args: &[&str] = match id.as_str() {
        "nova" => &["init", "--scope", "global"],
        "pi" | "claude" | "opencode" => &["auth", "login"],
        "codex" | "commandcode" => &["login"],
        "hermes" => &["setup"],
        _ => return Err(failure("Unknown agent")),
    };
    let root = state
        .guard
        .read()
        .await
        .as_ref()
        .map(|guard| guard.root().to_path_buf());
    let cwd = root.unwrap_or(app.path().home_dir().map_err(failure)?);
    open_agent_console(&id, args, &cwd)
}

#[tauri::command]
pub async fn open_nova_terminal(state: State<'_, AppState>) -> Result<(), AppError> {
    let workspace = state.require_guard().await?;
    open_agent_console("nova", &[], workspace.root())
}

fn open_agent_console(id: &str, args: &[&str], cwd: &Path) -> Result<(), AppError> {
    #[cfg(windows)]
    {
        let mut command = cli::command(id)?;
        command.current_dir(cwd).args(args).kill_on_drop(false);
        command.creation_flags(0x00000010); // CREATE_NEW_CONSOLE
        let mut child = command.spawn().map_err(failure)?;
        tauri::async_runtime::spawn(async move {
            let _ = child.wait().await;
        });
        Ok(())
    }
    #[cfg(not(windows))]
    {
        let _ = cwd;
        Err(failure(format!(
            "Run {} {} in your terminal",
            id,
            args.join(" ")
        )))
    }
}

#[tauri::command]
pub async fn markdownify(html: String) -> Result<String, AppError> {
    if html.len() > 512 * 1024 {
        return Err(failure("HTML input is limited to 512 KiB"));
    }
    let mut command = cli::nova_python()?;
    let mut child = command.args(["-I", "-c",
        "import sys; from markdownify import markdownify; sys.stdout.buffer.write(markdownify(sys.stdin.buffer.read().decode('utf-8')).encode('utf-8'))"])
        .stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::null()).spawn().map_err(failure)?;
    let mut stdin = child.stdin.take().expect("piped stdin");
    let mut stdout = child
        .stdout
        .take()
        .expect("piped stdout")
        .take(1024 * 1024 + 1);
    let mut output = Vec::new();
    tokio::time::timeout(Duration::from_secs(20), async {
        let send = async {
            stdin.write_all(html.as_bytes()).await?;
            drop(stdin);
            Ok(())
        };
        tokio::try_join!(send, stdout.read_to_end(&mut output)).map_err(failure)?;
        if output.len() > 1024 * 1024 {
            return Err(failure("Markdown output exceeded 1 MiB"));
        }
        if !child.wait().await.map_err(failure)?.success() {
            return Err(failure(
                "Could not load markdownify from Nova's Python environment",
            ));
        }
        String::from_utf8(output).map_err(failure)
    })
    .await
    .map_err(|_| failure("HTML conversion timed out"))?
}

#[tauri::command]
pub fn open_link(url: String) -> Result<(), AppError> {
    let url: Url = url.parse().map_err(failure)?;
    if !matches!(url.scheme(), "http" | "https") {
        return Err(failure("Only HTTP and HTTPS links can be opened"));
    }
    #[cfg(windows)]
    let mut command = Command::new(
        std::env::var_os("SystemRoot")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(r"C:\Windows"))
            .join("explorer.exe"),
    );
    #[cfg(target_os = "macos")]
    let mut command = system_command("open");
    #[cfg(all(not(windows), not(target_os = "macos")))]
    let mut command = system_command("xdg-open");
    command.arg(url.as_str()).spawn().map_err(failure)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_jsonc_and_keeps_url_literals() {
        let parsed: Value = serde_json::from_str(&jsonc(
            r#"{"link": "https://a.example", /* x */ "a": [1,],}"#,
        ))
        .unwrap();
        assert_eq!(parsed["link"], "https://a.example");
        assert_eq!(parsed["a"][0], 1);
    }

    #[test]
    fn include_paths_stay_inside_the_extension() {
        assert_eq!(
            member_path("extension/themes", "../base.json").as_deref(),
            Some("extension/base.json")
        );
        assert!(member_path("extension/themes", "../../base.json").is_none());
        assert!(member_path("extension", "themes/*.json").is_none());
    }

    #[tokio::test]
    async fn reads_vsix_workbench_colors_with_a_jsonc_include() {
        let archive = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/color-theme.vsix");
        let themes = read_vsix(&archive).await.unwrap();
        assert_eq!(themes[0].title, "Fixture theme");
        assert_eq!(themes[0].colors["editor.background"], "#111521");
        assert_eq!(themes[0].colors["editor.foreground"], "#f5e9d6");
    }
}
