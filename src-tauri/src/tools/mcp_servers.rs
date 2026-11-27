//! What each harness's own CLI says its MCP servers are.
//!
//! Settings shows the servers Nova inherits rather than a table it maintained
//! itself: every harness keeps its own configuration, and a copy in Nova would
//! drift the first time someone ran `claude mcp add`. So this runs their own
//! `mcp list` and reports what came back, read-only. Nova never writes a
//! harness's MCP config here.
//!
//! The difference between "no servers" and "did not answer" is the whole point
//! of the shape below: a CLI that exits non-zero has no usable list, and one
//! that misses the deadline is reported as timed out. Both are told apart from
//! an empty list, because a reader who sees "0 servers" would believe it.

use std::path::{Path, PathBuf};
use std::time::Duration;

use serde_json::{json, Value};

use crate::agent::cli;
use crate::commands::settings::capture;

/// The harnesses whose own CLI keeps an MCP list, with the name the sheet
/// shows. The ids match the harness table in mod.rs.
const HARNESSES: &[(&str, &str)] = cli::AGENTS;

/// Every one of these CLIs health-checks its servers before answering, so this is
/// a deadline for "did it answer at all", not for a fast answer: anything slower
/// is reported as a timeout instead of a half list.
///
/// Measured on the machine this was written on: `claude mcp list` 6.9 s, `nova mcp
/// list` 11.7 s, the other two under a second. Fifteen seconds is the first round
/// number that fits both without hiding a hang; the section is loaded when it is
/// opened, so nothing waits on it before the pane is drawn.
const DEADLINE: Duration = Duration::from_secs(15);

/// One list is a line per server; nothing here needs a megabyte of it.
const MAX_BYTES: u64 = 256 * 1024;

pub(crate) async fn commandcode_listing(
    workspace: &Path,
) -> Result<String, crate::error::AppError> {
    let mut command = cli::command("commandcode")?;
    command
        .current_dir(workspace)
        .env("NO_COLOR", "1")
        .args(["--no-auto-update", "mcp", "list"]);
    let bytes = capture(command, MAX_BYTES).await?;
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

pub(crate) fn commandcode_tools_registered(listing: &str) -> bool {
    listing.lines().any(|line| {
        let mut fields = line.split_whitespace();
        fields.next() == Some("nova-desktop")
            && fields.next() == Some("stdio")
            && fields.next() == Some("local")
            && fields.any(|field| field == "enabled")
    })
}

enum Answer {
    Rows(Vec<(String, String)>),
    Unsupported,
    TimedOut,
}

async fn one(id: &'static str, workspace: Option<PathBuf>) -> Answer {
    let Ok(mut command) = cli::command(id) else {
        return Answer::Unsupported;
    };
    command.env("NO_COLOR", "1").args(["mcp", "list"]);
    if id == "pi" {
        command.arg("--json");
    }
    if let Some(dir) = workspace {
        command.current_dir(dir);
    }
    // capture() kills the child on drop, so a timeout leaves nothing running.
    match tokio::time::timeout(DEADLINE, capture(command, MAX_BYTES)).await {
        Ok(Ok(bytes)) => match parse_harness(id, &String::from_utf8_lossy(&bytes)) {
            Some(rows) => Answer::Rows(rows),
            None => Answer::Unsupported,
        },
        Ok(Err(_)) => Answer::Unsupported,
        Err(_) => Answer::TimedOut,
    }
}

/// The servers every harness reports, and the harnesses that could not answer.
pub async fn discover(workspace: Option<&Path>) -> Value {
    let workspace = workspace.map(Path::to_path_buf);
    let mut running = Vec::new();
    for &(id, name) in HARNESSES {
        running.push((id, name, tokio::spawn(one(id, workspace.clone()))));
    }
    let mut servers: Vec<Value> = Vec::new();
    let mut timed_out: Vec<&str> = Vec::new();
    let mut unsupported: Vec<&str> = Vec::new();
    for (id, name, handle) in running {
        match handle.await {
            Ok(Answer::Rows(rows)) => {
                for (server, detail) in rows {
                    // The CLI's own words, including its connected/failed
                    // marker, rather than a state Nova only half knows.
                    servers.push(json!({
                        "harnessId": id,
                        "harness": name,
                        "name": server,
                        "detail": detail,
                    }));
                }
            }
            Ok(Answer::TimedOut) => timed_out.push(name),
            // A CLI that is missing or refused the command reports the same
            // thing: there is no list to read.
            Ok(Answer::Unsupported) | Err(_) => unsupported.push(name),
        }
    }
    json!({
        "servers": servers,
        "timedOut": timed_out,
        "unsupported": unsupported,
    })
}

/// The glyphs nova puts in front of each server it lists.
const MARKERS: [char; 4] = ['✓', '✗', '✔', '✘'];

fn parse_harness(id: &str, text: &str) -> Option<Vec<(String, String)>> {
    if id == "pi" {
        let doc: Value = serde_json::from_str(text).ok()?;
        if doc["errors"]
            .as_array()
            .is_some_and(|items| !items.is_empty())
        {
            return None;
        }
        return Some(
            doc["servers"]
                .as_array()?
                .iter()
                .filter_map(|server| {
                    let name = server["name"].as_str()?;
                    let scope = server["scope"].as_str().unwrap_or("configured");
                    let state = server["state"].as_str().unwrap_or("unknown");
                    Some((name.to_owned(), format!("{scope} · {state}")))
                })
                .collect(),
        );
    }
    if id == "opencode" {
        return Some(
            text.lines()
                .filter_map(|line| {
                    let line = line
                        .trim()
                        .strip_prefix(['✓', '✗', '○', '⚠', '✔', '✘'])?
                        .trim();
                    let (name, state) = line.split_once(char::is_whitespace)?;
                    Some((
                        name.trim_end_matches(':').to_owned(),
                        state.trim().to_owned(),
                    ))
                })
                .collect(),
        );
    }
    if id == "commandcode" || id == "hermes" {
        return Some(
            text.lines()
                .filter_map(|line| {
                    let mut columns = line.split_whitespace();
                    let name = columns.next()?;
                    if name.eq_ignore_ascii_case("name")
                        || !columns.any(|part| matches!(part, "enabled" | "disabled"))
                    {
                        return None;
                    }
                    let detail = if line.split_whitespace().any(|part| part == "disabled") {
                        "Disabled"
                    } else {
                        "Enabled"
                    };
                    Some((name.to_owned(), detail.into()))
                })
                .collect(),
        );
    }
    Some(parse(text))
}

/// The servers in a `mcp list` answer, in the CLI's own order.
///
/// Two shapes, chosen by what the answer contains rather than by which CLI asked:
/// nova prints one block per server opening with a status glyph, and the others
/// print one line per server.
///
/// shortcut: this covers the four formats observed on the machine this was
/// written on — nova's marked blocks, claude's `name: command - ✔ Connected`,
/// codex's two padded tables, and the "nothing configured" sentence opencode
/// prints. A fifth format needs its own reading, not a looser guess here.
fn parse(text: &str) -> Vec<(String, String)> {
    if text
        .lines()
        .any(|line| line.trim_start().starts_with(&MARKERS[..]))
    {
        parse_blocks(text)
    } else {
        parse_lines(text)
    }
}

/// nova's list: a glyph, the name, then Transport/Command/Args/Tools lines and
/// sometimes a description, until the next glyph. The detail keeps those lines
/// as the CLI wrote them, separated so one row stays readable.
fn parse_blocks(text: &str) -> Vec<(String, String)> {
    let mut rows: Vec<(String, String)> = Vec::new();
    for line in text.lines().map(str::trim) {
        if line.is_empty() {
            continue;
        }
        if let Some(rest) = line.strip_prefix(&MARKERS[..]) {
            let mut parts = rest.trim().splitn(2, char::is_whitespace);
            rows.push((
                parts.next().unwrap_or_default().to_string(),
                parts.next().unwrap_or_default().trim().to_string(),
            ));
            continue;
        }
        let Some(row) = rows.last_mut() else {
            // The heading above the first server names no server.
            continue;
        };
        if line.starts_with("Configuration file:") {
            continue;
        }
        if !row.1.is_empty() {
            row.1.push_str(" · ");
        }
        row.1.push_str(line);
    }
    rows
}

/// The one line per server shape: the name is the first token with its trailing
/// colon removed, and the detail is everything after it.
fn parse_lines(text: &str) -> Vec<(String, String)> {
    let mut rows = Vec::new();
    for line in text.lines().map(str::trim) {
        if line.is_empty()
            // claude's progress line before the table.
            || line.starts_with("Checking ")
            || line.eq_ignore_ascii_case("No MCP servers configured")
        {
            continue;
        }
        let mut parts = line.splitn(2, char::is_whitespace);
        let name = parts.next().unwrap_or_default().trim_end_matches(':');
        // Table headers and the rule under them are not servers.
        if name.is_empty() || name.eq_ignore_ascii_case("name") || name.chars().all(|c| c == '-') {
            continue;
        }
        rows.push((
            name.to_string(),
            parts.next().unwrap_or_default().trim().to_string(),
        ));
    }
    rows
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_status_without_mistaking_headings_for_servers() {
        let pi =
            r#"{"servers":[{"name":"fixture","scope":"project","state":"disabled"}],"errors":[]}"#;
        assert_eq!(
            parse_harness("pi", pi).unwrap(),
            [("fixture".into(), "project · disabled".into())]
        );
        let open = "✓ first connected\n⚠ second needs authentication\n○ third pending\n";
        assert_eq!(parse_harness("opencode", open).unwrap().len(), 3);
        let command = "Name  Transport\nnova-desktop  stdio  local ✔ enabled\n";
        assert_eq!(
            parse_harness("commandcode", command).unwrap(),
            [("nova-desktop".into(), "Enabled".into())]
        );
        assert!(commandcode_tools_registered(command));
        assert!(!commandcode_tools_registered(
            &command.replace("local", "project")
        ));
        assert!(!commandcode_tools_registered(
            &command.replace("enabled", "disabled")
        ));
        let hermes = "MCP Servers:\nName  Transport\n──────\nfixture  npx  all  ✗ disabled\n";
        assert_eq!(
            parse_harness("hermes", hermes).unwrap(),
            [("fixture".into(), "Disabled".into())]
        );
    }

    /// `claude mcp list`, captured as it answered here: five servers, one of
    /// them refused, after a status line that names no server.
    const CLAUDE: &str = r#"Checking MCP server health…

higgsfield: https://mcp.higgsfield.ai/mcp (HTTP) - ✔ Connected
rea: C:\\nvm4w\\nodejs\\node.exe C:\\Users\\Babit-PC\\AppData\\Local\\npm-cache\\_npx\\9b9809298fe40d2c\\node_modules\\rea-agents\\scripts\\rea.mjs mcp - ✘ Failed to connect — CONNECTION_CLOSED: Connection closed
caveman: C:\Users\Babit-PC\.caveman\bin\caveman-mcp.exe  - ✔ Connected
agentmemory: npx -y @agentmemory/mcp - ✔ Connected
rust-anything: C:\Users\Babit-PC\AppData\Local\nvm\v24.21.0\node_modules\rust-anything\binaries\win32-x64\rust-anything.exe mcp - ✔ Connected
"#;

    /// `codex mcp list`, captured here: two padded tables (stdio servers, then
    /// the HTTP one), each with its own header row.
    const CODEX: &str = r#"Name           Command                                                 Args                        Env                    Cwd  Status   Auth
agentmemory    npx                                                     -y @agentmemory/mcp         AGENTMEMORY_URL=*****  -    enabled  Unsupported
caveman        C:\Users\Babit-PC\.caveman\bin\caveman-mcp.exe           -                           -                      -    enabled  Unsupported
rust-anything  C:\Users\Babit-PC\AppData\Local\nvm\v24.21.0\node_modules\rust-anything\binaries\win32-x64\rust-anything.exe  mcp  -  -  enabled  Unsupported

Name     Url                          Bearer Token Env Var  Status   Auth
netlify  https://mcp.netlify.com/mcp  -                     enabled  OAuth
"#;

    /// `nova mcp list`, captured here: a heading, then one block per server
    /// opened by a glyph, and the configuration file at the end.
    const NOVA: &str = r#"Configured MCP Servers:

  ✓ rust-anything (active)
    Transport: stdio
    Command: rust-anything
    Args: mcp
    Tools: 16

  ✓ playwright (active)
    Browser automation using Playwright for web scraping and testing
    Transport: stdio
    Command: npx
    Args: -y @playwright/mcp@latest --browser=chrome --viewport-size=1280x720
--headless --timeout-action=30000
    Tools: 25

  ✓ obscura (active)
    Headless browser navigation, page inspection, and interaction
    Transport: stdio
    Command: C:\Tools\Obscura\obscura.exe
    Args: mcp
    Tools: 38

Configuration file: C:\Users\Babit-PC\.nova\mcp.json
"#;

    fn names(rows: &[(String, String)]) -> Vec<&str> {
        rows.iter().map(|(name, _)| name.as_str()).collect()
    }

    #[test]
    fn claude_rows_are_its_servers_and_not_its_status_line() {
        let rows = parse(CLAUDE);
        assert_eq!(
            names(&rows),
            [
                "higgsfield",
                "rea",
                "caveman",
                "agentmemory",
                "rust-anything"
            ]
        );
        // The connected/failed marker stays in the CLI's own words.
        assert!(rows[0].1.starts_with("https://mcp.higgsfield.ai/mcp"));
        assert!(rows[0].1.ends_with("Connected"));
        assert!(rows[1].1.contains("Failed to connect"));
    }

    #[test]
    fn codex_rows_skip_both_header_rows() {
        let rows = parse(CODEX);
        assert_eq!(
            names(&rows),
            ["agentmemory", "caveman", "rust-anything", "netlify"]
        );
    }

    #[test]
    fn nova_blocks_become_one_row_each_with_their_fields_kept() {
        let rows = parse(NOVA);
        assert_eq!(names(&rows), ["rust-anything", "playwright", "obscura"]);
        // The fields stay as nova wrote them, and no heading or file path
        // becomes a server of its own.
        assert!(rows[0].1.contains("Command: rust-anything"));
        assert!(rows[0].1.contains("Tools: 16"));
        assert!(rows[1].1.contains("Command: npx"));
        assert!(!rows
            .iter()
            .any(|(name, _)| name == "Configured" || name == "Transport" || name == "Command"));
    }

    #[test]
    fn nothing_configured_is_no_rows_rather_than_one_odd_row() {
        assert!(parse("No MCP servers configured\n").is_empty());
        assert!(parse("").is_empty());
    }
}
