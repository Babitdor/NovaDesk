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
        // documented: "models -- Lists every available model."
        "opencode" => &["models"],
        // CHANGELOG 1.x: "--list-models shows every selectable model id".
        "commandcode" => &["--list-models"],
        // Hermes keeps its catalog in config at providers.list-models.models,
        // and `config get` resolves dotted keys (docs/user-guide/configuration).
        "hermes" => &["config", "get", "providers.list-models.models", "--json"],
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
    // A missing key is a normal answer from `config get` (exit 1), not a failure:
    // an unconfigured harness simply has no models to offer yet.
    let output = match list(&agent, args, &cwd).await {
        Ok(output) => output,
        Err(_) if agent == "hermes" => String::new(),
        Err(error) => return Err(error),
    };
    if agent == "hermes" {
        let catalog = serde_json::from_str::<Value>(&output)
            .ok()
            .map(|value| hermes_models(&value))
            .unwrap_or_default();
        if !catalog.is_empty() {
            let mut models = catalog;
            models.sort_unstable_by(|a, b| a.id.cmp(&b.id));
            models.dedup_by(|a, b| a.id == b.id);
            return Ok(models);
        }
        // No curated list: fall back to whatever model is resolved right now, so a
        // working install still shows its current selection.
        let current = match list(&agent, &["config", "get", "model", "--json"], &cwd).await {
            Ok(output) => output,
            Err(_) => return Ok(Vec::new()),
        };
        let value = serde_json::from_str::<Value>(&current)
            .unwrap_or(Value::String(current.trim().to_owned()));
        return Ok(hermes_models(&value).into_iter().take(1).collect());
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

/// Hermes answers with its catalog as an object keyed by model id (each value
/// carries label/context detail), but a resolved single model comes back as a
/// string or as a `{ default: ... }` mapping — accept all three shapes.
fn hermes_models(value: &Value) -> Vec<Model> {
    let mut models = Vec::new();
    match value {
        Value::Object(map) => {
            // `{ default: "provider/model" }` is the resolved-current shape, not a
            // catalog: treat a lone `default` as one model.
            let looks_resolved = map.len() == 1 && map.contains_key("default");
            if looks_resolved {
                if let Some(id) = map["default"].as_str().filter(|id| valid_id(id)) {
                    models.push(Model::new(id));
                }
                return models;
            }
            for (id, detail) in map {
                if !valid_id(id) {
                    continue;
                }
                let mut model = Model::new(id);
                if let Some(label) = detail.get("label").and_then(Value::as_str) {
                    if !label.is_empty() {
                        model.label = label.to_owned();
                    }
                }
                model.context = detail
                    .get("context_length")
                    .and_then(Value::as_u64)
                    .map(|n| n.to_string());
                models.push(model);
            }
        }
        Value::Array(rows) => {
            for row in rows {
                let id = row
                    .get("id")
                    .or_else(|| row.get("model"))
                    .and_then(Value::as_str);
                if let Some(id) = id.filter(|id| valid_id(id)) {
                    models.push(Model::new(id));
                }
            }
        }
        Value::String(id) => {
            let id = id.trim();
            if valid_id(id) {
                models.push(Model::new(id));
            }
        }
        _ => {}
    }
    models
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
