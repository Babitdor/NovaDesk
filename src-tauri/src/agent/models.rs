use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use serde::Serialize;
use serde_json::Value;
use tauri::{AppHandle, Manager};
use tokio::io::AsyncReadExt;

use crate::error::AppError;

const CATALOG_LIMIT: u64 = 8 * 1024 * 1024;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Model {
    id: String,
    label: String,
    context: Option<String>,
    efforts: Option<Vec<String>>,
}

impl Model {
    fn new(id: &str) -> Self {
        Self {
            id: id.into(),
            label: id.into(),
            context: None,
            efforts: None,
        }
    }
}

pub fn accepts_effort(agent: &str, value: &str) -> bool {
    match agent {
        "pi" => matches!(
            value,
            "off" | "minimal" | "low" | "medium" | "high" | "xhigh" | "max"
        ),
        "claude" => matches!(value, "low" | "medium" | "high" | "xhigh" | "max"),
        "codex" => matches!(
            value,
            "none" | "minimal" | "low" | "medium" | "high" | "xhigh" | "max" | "ultra"
        ),
        "commandcode" => matches!(value, "low" | "medium" | "high"),
        _ => false,
    }
}

#[tauri::command]
pub async fn agent_models(agent: String, app: AppHandle) -> Result<Vec<Model>, AppError> {
    if agent == "codex" {
        let path = std::env::var_os("CODEX_HOME")
            .map(PathBuf::from)
            .filter(|path| path.is_absolute())
            .or_else(|| app.path().home_dir().ok().map(|home| home.join(".codex")));
        let Some(path) = path else {
            return Ok(Vec::new());
        };
        let file = match tokio::fs::File::open(path.join("models_cache.json")).await {
            Ok(file) => file,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(error) => return Err(AppError::Io(error.to_string())),
        };
        let mut data = Vec::new();
        file.take(CATALOG_LIMIT + 1)
            .read_to_end(&mut data)
            .await
            .map_err(|error| AppError::Io(error.to_string()))?;
        if data.len() as u64 > CATALOG_LIMIT {
            return Err(AppError::Agent("Codex model catalog is too large".into()));
        }
        let catalog = serde_json::from_slice(&data)
            .map_err(|_| AppError::Agent("Could not read the Codex model catalog".into()))?;
        return Ok(codex_models(&catalog));
    }

    let args: &[&str] = match agent.as_str() {
        "pi" => &["--list-models", "--no-extensions", "--no-mcp"],
        "opencode" => &["models"],
        "commandcode" => &["--list-models"],
        "hermes" => &["config", "get", "model", "--json"],
        _ => {
            return Err(AppError::Agent(
                "This agent has no CLI model catalog".into(),
            ))
        }
    };
    let cwd = app
        .path()
        .app_config_dir()
        .unwrap_or_else(|_| std::env::temp_dir());
    let cwd = if tokio::fs::create_dir_all(&cwd).await.is_ok() {
        cwd
    } else {
        std::env::temp_dir()
    };
    let output = list(&agent, args, &cwd).await?;
    if agent == "hermes" {
        let model = serde_json::from_str::<Value>(&output)
            .map_err(|_| AppError::Agent("Could not read the Hermes model configuration".into()))?;
        return Ok(model
            .as_str()
            .or_else(|| model["default"].as_str())
            .filter(|id| valid_id(id))
            .map(Model::new)
            .into_iter()
            .collect());
    }
    let mut models = terminal_models(&agent, &output);
    models.sort_unstable_by(|a, b| a.id.cmp(&b.id));
    models.dedup_by(|a, b| a.id == b.id);
    Ok(models)
}

async fn list(agent: &str, args: &[&str], cwd: &Path) -> Result<String, AppError> {
    let mut child = super::cli::command(agent)?
        .args(args)
        .current_dir(cwd)
        .env("NO_COLOR", "1")
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .stdout(Stdio::piped())
        .spawn()
        .map_err(|error| AppError::Spawn(error.to_string()))?;
    let mut stdout = child
        .stdout
        .take()
        .expect("piped model catalog")
        .take(CATALOG_LIMIT + 1);
    let mut data = Vec::new();
    let status = tokio::time::timeout(Duration::from_secs(8), async {
        stdout.read_to_end(&mut data).await?;
        if data.len() as u64 > CATALOG_LIMIT {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "Model catalog is too large",
            ));
        }
        child.wait().await
    })
    .await
    .map_err(|_| AppError::Agent("The CLI model catalog timed out".into()))?
    .map_err(|error| AppError::Io(error.to_string()))?;
    if !status.success() {
        return Err(AppError::Agent("The CLI could not list its models".into()));
    }
    Ok(String::from_utf8_lossy(&data).into_owned())
}

fn codex_models(catalog: &Value) -> Vec<Model> {
    catalog["models"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|row| {
            if row["visibility"]
                .as_str()
                .is_some_and(|value| value != "list")
            {
                return None;
            }
            let id = row["slug"].as_str().filter(|id| valid_id(id))?;
            let mut model = Model::new(id);
            if let Some(name) = row["display_name"].as_str() {
                model.label = name.into();
            }
            model.context = row["context_window"].as_u64().map(|n| n.to_string());
            model.efforts = Some(
                row["supported_reasoning_levels"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(|entry| entry["effort"].as_str())
                    .filter(|level| accepts_effort("codex", level))
                    .map(str::to_owned)
                    .collect(),
            );
            Some(model)
        })
        .collect()
}

fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 200
        && !id.starts_with('-')
        && id.chars().all(|ch| !ch.is_whitespace() && !ch.is_control())
}

fn terminal_models(agent: &str, output: &str) -> Vec<Model> {
    output
        .lines()
        .filter_map(|line| {
            let mut fields = line.split_whitespace();
            let first = fields.next()?;
            if agent == "pi" {
                let id = fields.next()?;
                let context = fields.next()?;
                if !context.starts_with(|ch: char| ch.is_ascii_digit())
                    || !valid_id(first)
                    || !valid_id(id)
                {
                    return None;
                }
                let mut model = Model::new(&format!("{first}/{id}"));
                model.context = Some(context.into());
                if fields.nth(1) == Some("no") {
                    model.efforts = Some(Vec::new());
                }
                return Some(model);
            }
            (first.contains('/') && valid_id(first)).then(|| Model::new(first))
        })
        .collect()
}

#[cfg(test)]
#[test]
fn model_outputs_keep_harness_ids_and_supported_effort() {
    let pi = terminal_models(
        "pi",
        "provider model context max-out thinking images\nopenai gpt-test 128K 16K yes yes",
    );
    assert_eq!(pi[0].id, "openai/gpt-test");
    assert_eq!(pi[0].context.as_deref(), Some("128K"));
    let command = terminal_models("commandcode", "Open Source\nsome/model  (default)\n");
    assert_eq!(command[0].id, "some/model");
    let codex = codex_models(&serde_json::json!({"models":[
        {"slug":"visible","supported_reasoning_levels":[{"effort":"high"},{"effort":"unknown"}]},
        {"slug":"hidden","visibility":"hide"}
    ]}));
    assert_eq!(codex.len(), 1);
    assert_eq!(
        codex[0].efforts.as_deref(),
        Some(["high".to_owned()].as_slice())
    );
    assert!(!accepts_effort("nova", "high"));
}
