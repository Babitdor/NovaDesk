//! GitHub commands: who is signed in, which repositories a profile has, how
//! busy a repository is, and cloning one onto this computer.
//!
//! Every network call lives here rather than in the webview: the token never
//! crosses the IPC boundary and never reaches an argument vector. curl gets the
//! Authorization header from a 0600 temp file (removed immediately afterwards),
//! and git is only ever asked for a public https URL.

use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::Serialize;
use serde_json::Value;
use tauri::ipc::Channel;
use tauri::{AppHandle, Manager, State};
use tokio_util::sync::CancellationToken;

use crate::agent::cli;
use crate::error::AppError;
use crate::events::AgentEvent;
// The Authorization header file lives in http.rs now: the speech services write
// the same one, and a credential in argv is readable by every process on the
// machine and ends up in a crash report.
use crate::http::write_header_file;
use crate::proc::ProcessRunner;
use crate::workspace::WorkspaceGuard;
use crate::AppState;

use super::settings::{capture, system_command};

/// The REST root. Every URL this module builds starts here and nowhere else.
const API: &str = "https://api.github.com";
/// Response cap. A 100-repository page is far smaller; anything bigger is not a
/// page the UI can render anyway.
const LIMIT: u64 = 2 * 1024 * 1024;
/// A clone of a large repository on a slow line is minutes, not seconds, but a
/// wedged child must not hold the pane forever.
const CLONE_TIMEOUT: Duration = Duration::from_secs(600);

fn failure(error: impl ToString) -> AppError {
    AppError::Agent(error.to_string())
}

// --- DTOs ---------------------------------------------------------------
// camelCase so the webview reads them as they are written in TypeScript; every
// field is a plain value, never a nested API object.

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GithubAccount {
    signed_in: bool,
    login: Option<String>,
    /// Which credential answered: an environment variable, gh, or the token the
    /// app stored. Present with and without a successful sign-in, so the pane
    /// can say "the saved token was rejected" instead of only "not signed in".
    source: Option<String>,
    avatar_url: Option<String>,
}

impl GithubAccount {
    fn signed_out(source: Option<&str>) -> Self {
        Self {
            signed_in: false,
            login: None,
            source: source.map(str::to_owned),
            avatar_url: None,
        }
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Repo {
    owner: String,
    name: String,
    full_name: String,
    private: bool,
    visibility: String,
    default_branch: String,
    language: Option<String>,
    updated_at: String,
    description: Option<String>,
    clone_url: String,
    web_url: String,
    stars: u64,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CommitRow {
    hash: String,
    subject: String,
    author: String,
    when: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Activity {
    pulls: u64,
    issues: u64,
    commits: Vec<CommitRow>,
}

// --- credential ---------------------------------------------------------

/// A token is only ever used as an HTTP header value, so anything that could
/// break out of that line is rejected before it is read or written.
fn valid_token(raw: &str) -> bool {
    (8..=255).contains(&raw.len())
        && raw
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
}

fn token_path(app: &AppHandle) -> Option<PathBuf> {
    app.path()
        .app_config_dir()
        .ok()
        .map(|dir| dir.join("github.json"))
}

/// A missing or corrupt file means "no token", never an error: the app still
/// browses public profiles without one.
fn read_token_file(path: &Path) -> Option<String> {
    let raw = std::fs::read(path).ok()?;
    let value: Value = serde_json::from_slice(&raw).ok()?;
    let token = value["token"].as_str()?.trim();
    valid_token(token).then(|| token.to_owned())
}

fn write_token_file(path: &Path, token: &str) -> Result<(), AppError> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| AppError::from_io(e, dir))?;
    }
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let body = serde_json::json!({ "token": token }).to_string();
    let mut file = options.open(path).map_err(|e| AppError::from_io(e, path))?;
    use std::io::Write;
    file.write_all(body.as_bytes())
        .map_err(|e| AppError::from_io(e, path))
}

/// The credential in force, and where it came from: the environment first so a
/// shell can override the app, then gh, then what the user saved here.
async fn token(app: &AppHandle) -> Option<(String, &'static str)> {
    for name in ["GITHUB_TOKEN", "GH_TOKEN"] {
        let Some(value) = std::env::var_os(name) else {
            continue;
        };
        let text = value.to_string_lossy().trim().to_owned();
        if valid_token(&text) {
            return Some((text, name));
        }
    }

    // gh is installed next to the other CLIs, which is PATH, not System32.
    if let Ok(mut command) = cli::command("gh") {
        command.args(["auth", "token"]);
        if let Ok(bytes) = capture(command, 4096).await {
            let text = String::from_utf8_lossy(&bytes).trim().to_owned();
            if valid_token(&text) {
                return Some((text, "gh auth token"));
            }
        }
    }

    token_path(app)
        .as_deref()
        .and_then(read_token_file)
        .map(|token| (token, "saved token"))
}

// --- HTTP ---------------------------------------------------------------

/// One authenticated GET, decoded as JSON.
///
/// curl runs without --fail so an HTTP error keeps its status code, which is
/// read from --write-out and turned into a sentence the pane can show. A 2xx
/// body that is not JSON is a failure, not an empty list.
async fn api_json(url: &str, token: Option<&str>) -> Result<Value, AppError> {
    let mut command = system_command("curl");
    command.args([
        "-q",
        "--silent",
        "--location",
        "--proto",
        "=https",
        "--proto-redir",
        "=https",
        "--max-redirs",
        "3",
        "--max-time",
        "25",
        "--max-filesize",
        &LIMIT.to_string(),
        "--user-agent",
        "NovaDesktop",
        "-H",
        "Accept: application/vnd.github+json",
        "-H",
        "X-GitHub-Api-Version: 2022-11-28",
        "--write-out",
        "\n%{http_code}",
    ]);

    let header = match token {
        Some(token) => Some(write_header_file(token)?),
        None => None,
    };
    if let Some(header) = &header {
        command
            .arg("-H")
            .arg(format!("@{}", header.path().display()));
    }
    command.arg("--").arg(url);

    let raw = capture(command, LIMIT + 16)
        .await
        .map_err(|_| failure("Could not reach GitHub. Check the connection and try again."))?;

    let text = String::from_utf8_lossy(&raw);
    let (body, status) = match text.rsplit_once('\n') {
        Some((body, code)) => (body, code.trim().parse::<u16>().unwrap_or(0)),
        None => (text.as_ref(), 0),
    };

    match status {
        200..=299 => serde_json::from_str(body)
            .map_err(|_| failure("GitHub returned a response Nova could not read.")),
        401 => Err(failure(
            "GitHub said 401. The saved token is no longer valid.",
        )),
        403 | 429 => Err(failure(
            "GitHub rate limit reached. Try again in a few minutes.",
        )),
        404 => Err(failure("GitHub could not find that repository or user.")),
        other => Err(failure(format!("GitHub said {other}."))),
    }
}

// --- validation ---------------------------------------------------------

/// An owner or repository name as GitHub writes it, and as a folder name: no
/// separators, no dots-only names, so the clone target stays one component.
fn repo_slug(raw: &str) -> Option<&str> {
    let trimmed = raw.trim();
    trimmed
        .bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
        .then_some(trimmed)
        .filter(|slug| (1..=100).contains(&slug.len()))
        .filter(|slug| *slug != "." && *slug != "..")
}

fn valid_login(raw: &str) -> bool {
    (1..=39).contains(&raw.len())
        && raw
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
}

fn text(value: &Value, key: &str) -> Option<String> {
    value[key].as_str().map(str::to_owned)
}

fn repo_from(value: &Value) -> Option<Repo> {
    let private = value["private"].as_bool().unwrap_or(false);
    let name = text(value, "name")?;
    Some(Repo {
        owner: text(&value["owner"], "login")?,
        full_name: text(value, "full_name").unwrap_or_else(|| name.clone()),
        name,
        private,
        visibility: text(value, "visibility")
            .unwrap_or_else(|| if private { "private" } else { "public" }.to_owned()),
        default_branch: text(value, "default_branch").unwrap_or_else(|| "main".to_owned()),
        language: text(value, "language"),
        updated_at: text(value, "updated_at").unwrap_or_default(),
        description: text(value, "description"),
        clone_url: text(value, "clone_url").unwrap_or_default(),
        web_url: text(value, "html_url").unwrap_or_default(),
        stars: value["stargazers_count"].as_u64().unwrap_or(0),
    })
}

fn commit_from(value: &Value) -> Option<CommitRow> {
    let commit = &value["commit"];
    let message = text(commit, "message")?;
    Some(CommitRow {
        hash: text(value, "sha")?,
        // A commit message is a body; the list shows its first line.
        subject: message.lines().next().unwrap_or_default().trim().to_owned(),
        author: text(&commit["author"], "name").unwrap_or_else(|| "unknown".to_owned()),
        when: text(&commit["author"], "date").unwrap_or_default(),
    })
}

fn total_count(value: &Value) -> u64 {
    value["total_count"].as_u64().unwrap_or(0)
}

// --- commands -----------------------------------------------------------

#[tauri::command]
pub async fn github_account(app: AppHandle) -> GithubAccount {
    let Some((token, source)) = token(&app).await else {
        return GithubAccount::signed_out(None);
    };
    match api_json(&format!("{API}/user"), Some(&token)).await {
        Ok(user) => GithubAccount {
            signed_in: true,
            login: text(&user, "login"),
            source: Some(source.to_owned()),
            avatar_url: text(&user, "avatar_url"),
        },
        // A rejected credential is reported as "not signed in" plus where it
        // came from, so the pane can name the stale one instead of hiding it.
        Err(_) => GithubAccount::signed_out(Some(source)),
    }
}

#[tauri::command]
pub async fn github_repos(login: Option<String>, app: AppHandle) -> Result<Vec<Repo>, AppError> {
    let credential = token(&app).await;
    let login = login
        .map(|value| value.trim().trim_start_matches('@').to_owned())
        .filter(|value| !value.is_empty());

    let url = match (&login, &credential) {
        (Some(login), _) => {
            if !valid_login(login) {
                return Err(failure("Enter a GitHub login, such as Babitor."));
            }
            format!("{API}/users/{login}/repos?per_page=100&sort=updated")
        }
        (None, Some(_)) => {
            format!("{API}/user/repos?per_page=100&sort=updated&affiliation=owner,collaborator")
        }
        (None, None) => {
            return Err(failure(
                "Enter a GitHub login to browse public repositories.",
            ))
        }
    };

    let body = api_json(&url, credential.as_ref().map(|(token, _)| token.as_str())).await?;
    Ok(body
        .as_array()
        .map(|items| items.iter().filter_map(repo_from).collect())
        .unwrap_or_default())
}

#[tauri::command]
pub async fn github_activity(
    owner: String,
    name: String,
    app: AppHandle,
) -> Result<Activity, AppError> {
    let owner = repo_slug(&owner).ok_or_else(|| failure("That is not a GitHub owner name."))?;
    let name = repo_slug(&name).ok_or_else(|| failure("That is not a GitHub repository name."))?;
    let credential = token(&app).await;
    let token = credential.as_ref().map(|(token, _)| token.as_str());

    // Three calls, in order, because the pane shows all three at once and the
    // search endpoint answers with a count rather than the threads themselves.
    let query = |kind: &str| {
        format!("{API}/search/issues?q=repo:{owner}/{name}+is:{kind}+is:open&per_page=1")
    };
    let pulls = api_json(&query("pr"), token).await?;
    let issues = api_json(&query("issue"), token).await?;
    let commits = api_json(
        &format!("{API}/repos/{owner}/{name}/commits?per_page=5"),
        token,
    )
    .await?;

    Ok(Activity {
        pulls: total_count(&pulls),
        issues: total_count(&issues),
        commits: commits
            .as_array()
            .map(|items| items.iter().filter_map(commit_from).collect())
            .unwrap_or_default(),
    })
}

#[tauri::command]
pub async fn github_set_token(
    token: Option<String>,
    app: AppHandle,
) -> Result<GithubAccount, AppError> {
    let path = token_path(&app).ok_or_else(|| failure("Nova could not find its config folder."))?;

    let Some(raw) = token else {
        match std::fs::remove_file(&path) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(AppError::from_io(error, &path)),
        }
        return Ok(GithubAccount::signed_out(None));
    };

    let token = raw.trim().to_owned();
    if !valid_token(&token) {
        return Err(failure("That does not look like a GitHub token."));
    }
    // Verified before it is stored, so a typo leaves the old state alone.
    let user = api_json(&format!("{API}/user"), Some(&token)).await?;
    write_token_file(&path, &token)?;
    Ok(GithubAccount {
        signed_in: true,
        login: text(&user, "login"),
        source: Some("saved token".to_owned()),
        avatar_url: text(&user, "avatar_url"),
    })
}

/// Native folder picker, used both for the pane's Change button and for the
/// first clone (when no parent has been chosen yet).
async fn pick_parent(app: &AppHandle) -> Result<Option<PathBuf>, AppError> {
    let (tx, rx) = tokio::sync::oneshot::channel();
    crate::commands::dialogs::pick_folder(app, move |picked| {
        let _ = tx.send(picked);
    });
    let Some(path) = rx
        .await
        .map_err(|_| failure("folder picker closed unexpectedly"))?
    else {
        return Ok(None);
    };
    Ok(Some(WorkspaceGuard::new(path)?.root().to_path_buf()))
}

#[tauri::command]
pub async fn clone_parent_pick(app: AppHandle) -> Result<Option<String>, AppError> {
    Ok(pick_parent(&app)
        .await?
        .map(|path| path.display().to_string()))
}

/// Clones owner/name into parent and returns the new folder.
///
/// The parent folder is the guard, and the target is parent/name built from the
/// validated slugs — never from the API's clone_url — so the destination cannot
/// leave the folder the user chose. An existing non-empty target is refused
/// before git runs, and git streams its progress to the pane through the same
/// process runner the rest of the app uses.
#[tauri::command]
pub async fn github_clone(
    command_id: String,
    owner: String,
    name: String,
    parent: Option<String>,
    on_event: Channel<AgentEvent>,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<String, AppError> {
    let owner = repo_slug(&owner)
        .ok_or_else(|| failure("That is not a GitHub owner name."))?
        .to_owned();
    let name = repo_slug(&name)
        .ok_or_else(|| failure("That is not a GitHub repository name."))?
        .to_owned();

    let parent = match parent.map(|path| path.trim().to_owned()) {
        Some(path) if !path.is_empty() => PathBuf::from(path),
        _ => pick_parent(&app)
            .await?
            .ok_or_else(|| failure("Choose a folder to clone into."))?,
    };

    let guard = WorkspaceGuard::new(&parent)?;
    let target = guard.root().join(&name);
    if target.exists() {
        let empty =
            target.is_dir() && std::fs::read_dir(&target).is_ok_and(|mut dir| dir.next().is_none());
        if !empty {
            return Err(failure(format!(
                "{} already exists and is not empty.",
                target.display()
            )));
        }
    }

    let args = vec![
        "clone".to_owned(),
        "--progress".to_owned(),
        format!("https://github.com/{owner}/{name}.git"),
    ];

    let cancel = CancellationToken::new();
    state
        .procs
        .lock()
        .expect("procs mutex poisoned")
        .insert(command_id.clone(), cancel.clone());

    let run = ProcessRunner::run(&guard, "git", &args, &on_event, &cancel);
    let result = tokio::time::timeout(CLONE_TIMEOUT, run).await;

    state
        .procs
        .lock()
        .expect("procs mutex poisoned")
        .remove(&command_id);

    match result {
        Err(_) => Err(failure(
            "git did not finish cloning within 10 minutes. Check the connection and try again.",
        )),
        Ok(Err(AppError::Cancelled)) => Err(AppError::Cancelled),
        Ok(Err(error)) => Err(error),
        Ok(Ok(outcome)) if outcome.succeeded() => Ok(target.display().to_string()),
        // git already explained itself on the pane's progress log; this is the
        // sentence under it, including the one thing git cannot know.
        Ok(Ok(_)) => Err(failure(format!(
            "git could not clone {owner}/{name}. Private repositories need credentials git already trusts (gh auth login)."
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testtmp::TempDir;

    #[test]
    fn repository_slugs_are_one_path_component() {
        assert_eq!(repo_slug("NovaDesk"), Some("NovaDesk"));
        assert_eq!(repo_slug("nova.desk_2"), Some("nova.desk_2"));
        assert_eq!(repo_slug(".."), None);
        assert_eq!(repo_slug("."), None);
        assert_eq!(repo_slug("a/b"), None);
        assert_eq!(repo_slug(""), None);
        assert_eq!(repo_slug("Lead Space"), None);
        assert_eq!(repo_slug(&"a".repeat(101)), None);
    }

    #[test]
    fn logins_and_tokens_are_validated_before_use() {
        assert!(valid_login("Babitor"));
        assert!(valid_login("a-b"));
        assert!(!valid_login(".."));
        assert!(!valid_login(""));
        assert!(!valid_login("a/b"));
        // A newline would end the header line it is written into.
        assert!(!valid_token("ghp_abc def"));
        assert!(!valid_token("ghp_abcdef\r\nHost: evil"));
        assert!(!valid_token("short"));
        assert!(valid_token("ghp_0123456789abcdef"));
    }

    #[test]
    fn reads_a_stored_token_and_ignores_a_corrupt_file() {
        let tmp = TempDir::new("github-token");
        let path = tmp.path().join("github.json");
        assert_eq!(read_token_file(&path), None);

        std::fs::write(&path, "{ not json").unwrap();
        assert_eq!(read_token_file(&path), None);

        std::fs::write(&path, r#"{"token": "ghp_0123456789"}"#).unwrap();
        assert_eq!(read_token_file(&path).as_deref(), Some("ghp_0123456789"));

        write_token_file(&path, "ghp_fedcba9876").unwrap();
        assert_eq!(read_token_file(&path).as_deref(), Some("ghp_fedcba9876"));
    }

    #[test]
    fn maps_every_repository_field() {
        let raw = r#"{
            "name": "NovaDesk",
            "full_name": "Babitor/NovaDesk",
            "private": false,
            "visibility": "public",
            "default_branch": "main",
            "language": "TypeScript",
            "updated_at": "2026-11-20T10:00:00Z",
            "description": "A Tauri shell",
            "clone_url": "https://github.com/Babitor/NovaDesk.git",
            "html_url": "https://github.com/Babitor/NovaDesk",
            "stargazers_count": 3,
            "owner": { "login": "Babitor" }
        }"#;
        let value: Value = serde_json::from_str(raw).unwrap();
        let repo = repo_from(&value).expect("a repository with a name and an owner");
        assert_eq!(repo.owner, "Babitor");
        assert_eq!(repo.name, "NovaDesk");
        assert_eq!(repo.full_name, "Babitor/NovaDesk");
        assert!(!repo.private);
        assert_eq!(repo.visibility, "public");
        assert_eq!(repo.default_branch, "main");
        assert_eq!(repo.language.as_deref(), Some("TypeScript"));
        assert_eq!(repo.updated_at, "2026-11-20T10:00:00Z");
        assert_eq!(repo.description.as_deref(), Some("A Tauri shell"));
        assert_eq!(repo.clone_url, "https://github.com/Babitor/NovaDesk.git");
        assert_eq!(repo.web_url, "https://github.com/Babitor/NovaDesk");
        assert_eq!(repo.stars, 3);
    }

    #[test]
    fn falls_back_to_private_and_branch_names_the_api_omits() {
        let raw = r#"{"name": "secret", "private": true, "owner": {"login": "me"}}"#;
        let value: Value = serde_json::from_str(raw).unwrap();
        let repo = repo_from(&value).expect("a repository with a name and an owner");
        assert_eq!(repo.visibility, "private");
        assert_eq!(repo.default_branch, "main");
        assert_eq!(repo.full_name, "secret");
        assert_eq!(repo.language, None);
        assert_eq!(repo.stars, 0);
        // Without an owner there is no full name to show, so it is skipped
        // rather than rendered as an empty row.
        assert!(repo_from(&serde_json::json!({"name": "orphan"})).is_none());
    }

    #[test]
    fn maps_activity_counts_and_the_first_line_of_a_commit() {
        let pulls: Value = serde_json::from_str(r#"{"total_count": 3}"#).unwrap();
        let issues: Value = serde_json::from_str(r#"{"total_count": 11}"#).unwrap();
        let commits: Value = serde_json::from_str(
            r#"[{
                "sha": "3f2a1b0",
                "commit": {
                    "message": "GitHub pane: browse and clone\n\nA body line.",
                    "author": { "name": "Babitor", "date": "2026-11-20T09:00:00Z" }
                }
            }]"#,
        )
        .unwrap();

        let activity = Activity {
            pulls: total_count(&pulls),
            issues: total_count(&issues),
            commits: commits
                .as_array()
                .unwrap()
                .iter()
                .filter_map(commit_from)
                .collect(),
        };

        assert_eq!(activity.pulls, 3);
        assert_eq!(activity.issues, 11);
        assert_eq!(activity.commits[0].hash, "3f2a1b0");
        assert_eq!(activity.commits[0].subject, "GitHub pane: browse and clone");
        assert_eq!(activity.commits[0].author, "Babitor");
        assert_eq!(activity.commits[0].when, "2026-11-20T09:00:00Z");
    }
}
