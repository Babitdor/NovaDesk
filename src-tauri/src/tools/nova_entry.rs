//! The managed `nova-desktop` entry in a workspace's `.nova/mcp.json`.
//!
//! Nova's pipe sessions read MCP servers from their own config files, and the
//! Nova harness spawns one long-lived `nova --mode pipe` child per session —
//! so the per-turn argv and environment shapes the other harnesses use do not
//! exist here. What does work: writing a `nova-desktop` stdio entry into the
//! workspace's own `.nova/mcp.json` before the child boots, pointing at the
//! shim, with `NOVA_TOOLS_FILE` naming the launch's credentials file.
//!
//! Why this is safe now when the module documentation long said it was not:
//! the token never goes into the config. The credentials file is per app
//! launch (`%TEMP%\nova-desktop-tools-<pid>.json`), rewritten whenever the
//! endpoint changes, and deleted when the app exits — the config only ever
//! carries its path, and a stale path degrades to the shim's own
//! "no endpoint" line, never to a wrong token.
//!
//! Scope guards:
//!
//! - Only `<workspace>/.nova/mcp.json` is ever touched. The user's global
//!   `~/.nova/mcp.json` is shared with the CLI they run in a terminal, and a
//!   crash would leave a stale entry there forever.
//! - The merge replaces only the `nova-desktop` key and keeps every other
//!   server and every unknown top-level key; a write that would produce bytes
//!   identical to what is on disk is skipped, because sessions restart often
//!   and a rewrite per session is churn.
//! - Two Nova Desktop launches at once both write the entry for their own
//!   workspace: last launch wins, which matches the endpoint being per launch.

use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

use serde_json::{json, Value};

use crate::error::AppError;
use crate::tools::Bridge;

/// The server name this module owns in a workspace's `mcp.json`.
pub const SERVER_NAME: &str = "nova-desktop";

/// The launch's credentials file, created on first use and deleted on exit.
struct Launch {
    path: PathBuf,
    body: String,
}

fn launch() -> &'static Mutex<std::collections::HashMap<String, Launch>> {
    static LAUNCH: OnceLock<Mutex<std::collections::HashMap<String, Launch>>> = OnceLock::new();
    LAUNCH.get_or_init(|| Mutex::new(std::collections::HashMap::new()))
}

/// The credentials file for this app launch, written on first use.
///
/// One file per launch, shared by every session of that launch: the child
/// outlives the turn, so a per-turn file would be deleted under a live
/// session. The path embeds the app's process id — two simultaneous launches
/// get two files, and the config entry's `env` must name the one belonging to
/// the launch that wrote it (last launch wins; see the module header).
pub fn launch_credentials(bridge: &Bridge) -> Result<PathBuf, AppError> {
    let body = json!({ "url": bridge.url, "token": bridge.token, "harness": "nova" }).to_string();
    let mut launch = launch().lock().expect("launch credentials mutex poisoned");
    if let Some(existing) = launch.get(&bridge.token) {
        if existing.body == body && existing.path.is_file() {
            return Ok(existing.path.clone());
        }
    }
    let path = std::env::temp_dir().join(format!(
        "nova-desktop-tools-{}-{}.json",
        std::process::id(),
        uuid::Uuid::new_v4()
    ));
    std::fs::write(&path, &body).map_err(|error| {
        AppError::Io(format!(
            "could not write the Nova Desktop tools credentials to {}: {error}",
            path.display()
        ))
    })?;
    launch.insert(
        bridge.token.clone(),
        Launch {
            path: path.clone(),
            body,
        },
    );
    Ok(path)
}

/// Deletes the launch's credentials file. Called when the endpoint stops: a
/// stale path makes the shim print its own "no endpoint" line instead of
/// trusting a token that no longer exists.
pub fn remove_launch_credentials() {
    let mut launch = launch().lock().expect("launch credentials mutex poisoned");
    for (_, existing) in launch.drain() {
        let _ = std::fs::remove_file(&existing.path);
    }
}

/// `<workspace>/.nova/mcp.json` — the only file this module writes.
fn workspace_mcp_path(root: &Path) -> PathBuf {
    root.join(".nova").join("mcp.json")
}

/// Reads the workspace config, or an empty document when it does not exist.
///
/// A file that exists but does not parse is an error, not a document to
/// overwrite: the merge must never destroy servers it could not read.
fn read_workspace_document(root: &Path) -> Result<Option<(PathBuf, Value, String)>, AppError> {
    let path = workspace_mcp_path(root);
    let Ok(raw) = std::fs::read_to_string(&path) else {
        return Ok(None);
    };
    let document: Value = serde_json::from_str(&raw).map_err(|error| {
        AppError::Io(format!(
            "{} is not valid JSON, so the nova-desktop entry could not be merged: {error}",
            path.display()
        ))
    })?;
    Ok(Some((path, document, raw)))
}

/// The managed entry for `credentials` and `shim`, replacing any previous
/// version of itself wholesale: the `env` block must not accumulate keys from
/// an older shape of this code.
fn managed_entry(credentials: &Path, shim: &Path) -> Value {
    json!({
        "transport": "stdio",
        "command": shim.display().to_string(),
        "args": [],
        "env": { crate::agent::cli::TOOLS_FILE_ENV: credentials.display().to_string() },
        "disabled": false,
    })
}

/// Writes the managed entry into `<workspace>/.nova/mcp.json`, merging with
/// whatever is already there. Returns whether bytes were written: a document
/// that would come out byte-identical skips the write, because sessions
/// restart often and a rewrite per session is churn.
pub fn ensure_workspace_entry(
    root: &Path,
    credentials: &Path,
    shim: &Path,
) -> Result<bool, AppError> {
    let Some((path, mut document, _raw)) = read_workspace_document(root)? else {
        write_document(
            &workspace_mcp_path(root),
            &json!({
                "mcpServers": { SERVER_NAME: managed_entry(credentials, shim) },
            }),
        )?;
        return Ok(true);
    };
    if !document.is_object() {
        return Err(AppError::Io(format!(
            "{} is not a JSON object, so the nova-desktop entry could not be merged",
            path.display()
        )));
    }
    let servers = document
        .as_object_mut()
        .expect("checked above")
        .entry("mcpServers")
        .or_insert_with(|| json!({}));
    if !servers.is_object() {
        return Err(AppError::Io(format!(
            "{} has a non-object mcpServers, so the nova-desktop entry could not be merged",
            path.display()
        )));
    }
    servers[SERVER_NAME] = managed_entry(credentials, shim);
    write_document(&path, &document)
}

/// Removes the managed entry, best-effort: the caller logs instead of
/// propagating, because a leftover entry degrades to the shim's own "no
/// endpoint" line. An `mcpServers` object left empty is removed with it.
pub fn remove_workspace_entry(root: &Path) {
    let Ok(Some((path, mut document, _raw))) = read_workspace_document(root) else {
        return;
    };
    let Some(servers) = document
        .get_mut("mcpServers")
        .and_then(Value::as_object_mut)
    else {
        return;
    };
    if servers.remove(SERVER_NAME).is_none() {
        return;
    }
    if servers.is_empty() {
        document
            .as_object_mut()
            .expect("checked when reading")
            .remove("mcpServers");
    }
    let _ = write_document(&path, &document);
}

/// Serializes, skips an identical file, and writes atomically: a temp file in
/// the destination's directory plus a rename, so a crash mid-write leaves
/// either the old file or the new one, never a half-written config.
fn write_document(path: &Path, document: &Value) -> Result<bool, AppError> {
    let serialized =
        serde_json::to_string_pretty(document).map_err(|error| AppError::Io(error.to_string()))?;
    if let Ok(existing) = std::fs::read_to_string(path) {
        if existing == serialized {
            return Ok(false);
        }
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|error| {
            AppError::Io(format!("could not create {}: {error}", parent.display()))
        })?;
    }
    let temp = path.with_extension(format!("json.tmp-{}", std::process::id()));
    std::fs::write(&temp, &serialized)
        .map_err(|error| AppError::Io(format!("could not write {}: {error}", temp.display())))?;
    std::fs::rename(&temp, path).map_err(|error| {
        let _ = std::fs::remove_file(&temp);
        AppError::Io(format!(
            "could not move {} into place: {error}",
            temp.display()
        ))
    })?;
    Ok(true)
}

/// Session compatibility includes the credentials path fixed at process startup.
pub fn workspace_credentials(root: &Path) -> Option<PathBuf> {
    let document: Value =
        serde_json::from_slice(&std::fs::read(workspace_mcp_path(root)).ok()?).ok()?;
    document["mcpServers"][SERVER_NAME]["env"][crate::agent::cli::TOOLS_FILE_ENV]
        .as_str()
        .or_else(|| {
            document["servers"][SERVER_NAME]["env"][crate::agent::cli::TOOLS_FILE_ENV].as_str()
        })
        .map(PathBuf::from)
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Workspace(PathBuf);

    impl Workspace {
        fn new() -> Self {
            let dir = std::env::temp_dir().join(format!(
                "nova-entry-{}-{}",
                std::process::id(),
                uuid::Uuid::new_v4().simple()
            ));
            std::fs::create_dir_all(dir.join(".nova")).expect("create the workspace");
            Self(dir)
        }

        fn path(&self) -> &Path {
            &self.0
        }

        fn raw(&self) -> String {
            std::fs::read_to_string(self.path().join(".nova").join("mcp.json"))
                .expect("the config exists")
        }
    }

    impl Drop for Workspace {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    const CREDENTIALS: &str = r"C:\temp\nova-desktop-tools-1234.json";
    const SHIM: &str = r"C:\app\nova-tools.exe";

    #[test]
    fn a_fresh_workspace_gets_the_entry_with_the_shim_and_credentials_path() {
        let workspace = Workspace::new();
        let written =
            ensure_workspace_entry(workspace.path(), Path::new(CREDENTIALS), Path::new(SHIM))
                .expect("the entry writes");
        assert!(written);
        let document: Value = serde_json::from_str(&workspace.raw()).expect("valid JSON");
        let entry = &document["mcpServers"][SERVER_NAME];
        assert_eq!(entry["transport"], "stdio");
        assert_eq!(entry["command"], SHIM);
        assert_eq!(entry["args"], json!([]));
        assert_eq!(entry["env"][crate::agent::cli::TOOLS_FILE_ENV], CREDENTIALS);
        assert_eq!(entry["disabled"], false);
    }

    #[test]
    fn other_servers_and_unknown_keys_survive_the_merge() {
        let workspace = Workspace::new();
        std::fs::write(
            workspace.path().join(".nova").join("mcp.json"),
            r#"{
  "mcpServers": {
    "serena": { "transport": "stdio", "command": "uvx", "args": ["serena"] }
  },
  "someFutureKey": { "anything": true }
}"#,
        )
        .expect("seed the config");
        ensure_workspace_entry(workspace.path(), Path::new(CREDENTIALS), Path::new(SHIM))
            .expect("the entry writes");
        let document: Value = serde_json::from_str(&workspace.raw()).expect("valid JSON");
        assert_eq!(
            document["mcpServers"]["serena"]["command"], "uvx",
            "a pre-existing server must survive"
        );
        assert_eq!(
            document["someFutureKey"]["anything"], true,
            "an unknown top-level key must survive"
        );
        assert_eq!(document["mcpServers"][SERVER_NAME]["command"], SHIM);
    }

    #[test]
    fn a_second_identical_write_changes_nothing() {
        let workspace = Workspace::new();
        ensure_workspace_entry(workspace.path(), Path::new(CREDENTIALS), Path::new(SHIM))
            .expect("the first write");
        let before = workspace.raw();
        let written =
            ensure_workspace_entry(workspace.path(), Path::new(CREDENTIALS), Path::new(SHIM))
                .expect("the second write");
        assert!(!written, "an identical document must skip the write");
        assert_eq!(workspace.raw(), before, "the bytes must not change");
    }

    #[test]
    fn the_managed_entry_is_replaced_wholesale_not_merged() {
        let workspace = Workspace::new();
        ensure_workspace_entry(workspace.path(), Path::new(CREDENTIALS), Path::new(SHIM))
            .expect("the first write");
        // Simulate an older shape of this code leaving a stray env key behind.
        let document: Value = serde_json::from_str(&workspace.raw()).expect("valid JSON");
        let mut stale = document.clone();
        stale["mcpServers"][SERVER_NAME]["env"]["NOVA_TOOLS_URL"] = json!("http://127.0.0.1:1/mcp");
        std::fs::write(
            workspace.path().join(".nova").join("mcp.json"),
            serde_json::to_string_pretty(&stale).expect("serialize"),
        )
        .expect("write the stale shape");
        ensure_workspace_entry(workspace.path(), Path::new(CREDENTIALS), Path::new(SHIM))
            .expect("the rewrite");
        let document: Value = serde_json::from_str(&workspace.raw()).expect("valid JSON");
        let env = document["mcpServers"][SERVER_NAME]["env"]
            .as_object()
            .expect("env is an object");
        assert_eq!(
            env.len(),
            1,
            "the env block must not accumulate keys: {env:?}"
        );
        assert_eq!(env[crate::agent::cli::TOOLS_FILE_ENV], CREDENTIALS);
    }

    #[test]
    fn the_entry_is_gone_after_the_last_session_ends_and_the_rest_stays() {
        let workspace = Workspace::new();
        std::fs::write(
            workspace.path().join(".nova").join("mcp.json"),
            r#"{ "mcpServers": { "serena": { "transport": "stdio", "command": "uvx" } } }"#,
        )
        .expect("seed the config");
        ensure_workspace_entry(workspace.path(), Path::new(CREDENTIALS), Path::new(SHIM))
            .expect("the entry writes");
        remove_workspace_entry(workspace.path());
        let document: Value = serde_json::from_str(&workspace.raw()).expect("valid JSON");
        assert!(
            document["mcpServers"].get(SERVER_NAME).is_none(),
            "the managed entry must be gone"
        );
        assert_eq!(
            document["mcpServers"]["serena"]["command"], "uvx",
            "the other server must survive the removal"
        );
    }

    #[test]
    fn a_corrupt_config_is_reported_never_overwritten() {
        let workspace = Workspace::new();
        std::fs::write(
            workspace.path().join(".nova").join("mcp.json"),
            "{ not json",
        )
        .expect("write a corrupt config");
        let error =
            ensure_workspace_entry(workspace.path(), Path::new(CREDENTIALS), Path::new(SHIM))
                .expect_err("a corrupt file must not be silently replaced");
        assert!(error.to_string().contains("not valid JSON"), "{error}");
        assert_eq!(
            std::fs::read_to_string(workspace.path().join(".nova").join("mcp.json"))
                .expect("the file is still there"),
            "{ not json",
            "the corrupt file must be left exactly as it was"
        );
    }

    #[test]
    fn credentials_are_stable_per_session_and_isolated_between_sessions() {
        let bridge = Bridge {
            shim: PathBuf::from(SHIM),
            url: "http://127.0.0.1:9/mcp".into(),
            token: "token-a".into(),
        };
        let first = launch_credentials(&bridge).expect("the credentials write");
        let again = launch_credentials(&bridge).expect("the second call");
        assert_eq!(first, again, "one launch, one credentials file");
        let rotated = Bridge {
            token: "token-b".into(),
            ..bridge
        };
        let changed = launch_credentials(&rotated).expect("the rotated write");
        assert_ne!(changed, first, "sessions have separate credentials files");
        assert!(std::fs::read_to_string(&first).unwrap().contains("token-a"));
        let body = std::fs::read_to_string(&changed).expect("the body is readable");
        assert!(body.contains("token-b"), "the body was rewritten: {body}");
        remove_launch_credentials();
        assert!(
            !changed.is_file(),
            "the credentials file is deleted when the endpoint stops"
        );
    }
}
