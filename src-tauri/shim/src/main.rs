//! nova-tools: the stdio MCP shim every harness starts.
//!
//! One binary, no dependencies, no shared code with the app: it reads one
//! JSON-RPC message per line from stdin, posts it to Nova's loopback endpoint,
//! and writes the answer back as one line. Everything the harness sees comes
//! from that endpoint, so the shim cannot drift from the catalogue the app
//! serves.
//!
//! Why a shim instead of pointing each harness at the HTTP endpoint: the seven
//! CLIs disagree about HTTP MCP configuration (Claude Code takes JSON files,
//! Codex takes dotted TOML overrides, OpenCode takes an inline config document)
//! but all of them start a stdio server the same way. One proxy, one shape.
//!
//! The endpoint and the bearer token arrive in the environment, never in argv:
//! a command line is readable by every process on the machine and ends up in
//! crash reports. Nova mints the token per launch, so a stale copy is useless.
//!
//! Nothing is ever printed to stdout except protocol messages.

use std::io::{BufRead, Read, Write};
use std::net::TcpStream;
use std::time::Duration;

/// One tool call may legitimately take a while (a screenshot, a search over a
/// large tree); a hung app must not hang the harness forever.
const REPLY_TIMEOUT: Duration = Duration::from_secs(120);

/// Where the endpoint is and what to present to it.
///
/// NOVA_TOOLS_FILE wins: Nova writes the endpoint and this launch's token into a
/// file only this user can read, and hands the harness its path, so neither
/// value has to appear on a command line. NOVA_TOOLS_URL/TOKEN stay supported
/// for a hand-written configuration.
fn credentials() -> Option<(String, String, String)> {
    if let Some(path) = std::env::var_os("NOVA_TOOLS_FILE") {
        let body = std::fs::read_to_string(path).ok()?;
        let url = unquote(&serde_field(&body, "\"url\"")?)?;
        let token = serde_field(&body, "\"token\"")
            .and_then(|value| unquote(&value))
            .unwrap_or_default();
        let harness = serde_field(&body, "\"harness\"")
            .and_then(|value| unquote(&value))
            .unwrap_or_default();
        return Some((url, token, harness));
    }
    let url = std::env::var("NOVA_TOOLS_URL").ok()?;
    let token = std::env::var("NOVA_TOOLS_TOKEN").unwrap_or_default();
    Some((url, token, String::new()))
}

fn unquote(value: &str) -> Option<String> {
    let trimmed = value.trim();
    let inner = trimmed
        .strip_prefix('"')
        .and_then(|rest| rest.strip_suffix('"'))?;
    Some(inner.replace("\\\"", "\"").replace("\\\\", "\\"))
}

fn main() {
    let (url, token, harness) = match credentials() {
        Some(credentials) => credentials,
        None => {
            eprintln!("nova-tools: no endpoint; set NOVA_TOOLS_FILE (Nova sets it per turn)");
            std::process::exit(2);
        }
    };
    let target = match parse(&url) {
        Some(target) => target,
        None => {
            eprintln!("nova-tools: NOVA_TOOLS_URL is not a loopback http URL: {url}");
            std::process::exit(2);
        }
    };

    let stdin = std::io::stdin();
    let mut stdout = std::io::stdout();
    for line in stdin.lock().lines() {
        let line = match line {
            Ok(line) => line,
            Err(_) => break,
        };
        let message = line.trim();
        if message.is_empty() {
            continue;
        }
        let reply = match post(&target, &token, &harness, message) {
            Ok(body) => body,
            Err(reason) => {
                // Answering in-band keeps the harness moving: a transport that
                // silently drops a request looks like a hung agent.
                let id = id_of(message);
                format!(
                    r#"{{"jsonrpc":"2.0","id":{id},"error":{{"code":-32000,"message":"{reason}"}}}}"#
                )
            }
        };
        if reply.is_empty() {
            continue;
        }
        if writeln!(stdout, "{reply}").is_err() {
            break;
        }
        if stdout.flush().is_err() {
            break;
        }
    }
}

/// The id of a request, or null: a notification has none, and an error for a
/// notification is not allowed to carry an id that was never given.
fn id_of(message: &str) -> String {
    match serde_field(message, "\"id\"") {
        Some(value) => value,
        None => "null".to_string(),
    }
}

/// A deliberately tiny field reader, so the shim needs no JSON parser: find the
/// key, then take the following value up to a delimiter at depth zero.
fn serde_field(message: &str, key: &str) -> Option<String> {
    let at = message.find(key)? + key.len();
    let rest = message[at..].trim_start();
    let rest = rest.strip_prefix(':')?.trim_start();
    let mut depth = 0_i32;
    let mut out = String::new();
    for byte in rest.chars() {
        match byte {
            '{' | '[' => depth += 1,
            '}' | ']' if depth == 0 => break,
            '}' | ']' => depth -= 1,
            ',' if depth == 0 => break,
            _ => {}
        }
        out.push(byte);
    }
    let value = out.trim().to_string();
    (!value.is_empty()).then_some(value)
}

struct Target {
    host: String,
    port: u16,
    path: String,
}

fn parse(url: &str) -> Option<Target> {
    let rest = url.strip_prefix("http://")?;
    let (authority, path) = match rest.split_once('/') {
        Some((authority, path)) => (authority, format!("/{path}")),
        None => (rest, "/".to_string()),
    };
    let (host, port) = match authority.rsplit_once(':') {
        Some((host, port)) => (host.to_string(), port.parse::<u16>().ok()?),
        None => (authority.to_string(), 80),
    };
    // Loopback only: the shim must never be talked into sending the token
    // somewhere else, even if the environment was tampered with.
    if !matches!(host.as_str(), "127.0.0.1" | "localhost" | "[::1]" | "::1") {
        return None;
    }
    Some(Target { host, port, path })
}

fn post(target: &Target, token: &str, harness: &str, body: &str) -> Result<String, String> {
    let host = target.host.trim_start_matches('[').trim_end_matches(']');
    let mut stream = TcpStream::connect((host, target.port)).map_err(|err| {
        format!(
            "Nova's tools endpoint is not answering on {}:{} ({err})",
            target.host, target.port
        )
    })?;
    stream
        .set_read_timeout(Some(REPLY_TIMEOUT))
        .map_err(|err| err.to_string())?;
    stream
        .set_write_timeout(Some(REPLY_TIMEOUT))
        .map_err(|err| err.to_string())?;
    let harness = if harness.bytes().all(|byte| byte.is_ascii_lowercase()) {
        harness
    } else {
        ""
    };
    let request = format!(
        "POST {} HTTP/1.1\r\nHost: {}:{}\r\nAuthorization: Bearer {}\r\nX-Nova-Harness: {}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        target.path,
        target.host,
        target.port,
        token,
        harness,
        body.len(),
        body
    );
    stream
        .write_all(request.as_bytes())
        .map_err(|err| format!("could not send the request ({err})"))?;
    let mut response = Vec::new();
    stream
        .read_to_end(&mut response)
        .map_err(|err| format!("could not read the reply ({err})"))?;
    body_of(&response)
}

/// Splits a response into its status and body. A non-2xx status becomes an
/// error whose text is the endpoint's own JSON-RPC message when there is one.
fn body_of(response: &[u8]) -> Result<String, String> {
    let split = response
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .ok_or_else(|| "the reply has no header block".to_string())?;
    let head = String::from_utf8_lossy(&response[..split]);
    let status: u16 = head
        .split(' ')
        .nth(1)
        .and_then(|code| code.parse().ok())
        .unwrap_or(0);
    let body = String::from_utf8_lossy(&response[split + 4..])
        .trim()
        .to_string();
    if (200..300).contains(&status) {
        return Ok(body);
    }
    let detail = serde_field(&body, "\"message\"").unwrap_or_else(|| body.clone());
    Err(format!("Nova refused the call (HTTP {status}): {detail}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_loopback_targets_are_accepted() {
        assert!(parse("http://127.0.0.1:1234/mcp").is_some());
        assert!(parse("http://localhost:1234/mcp").is_some());
        assert!(parse("http://example.com:1234/mcp").is_none());
        assert!(parse("https://127.0.0.1:1234/mcp").is_none());
    }

    #[test]
    fn reads_the_id_of_a_request_and_null_for_a_notification() {
        assert_eq!(
            id_of(r#"{"jsonrpc":"2.0","id":7,"method":"initialize"}"#),
            "7"
        );
        assert_eq!(
            id_of(r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#),
            "null"
        );
    }

    #[test]
    fn a_non_2xx_reply_keeps_the_endpoint_message() {
        let raw = b"HTTP/1.1 401 Unauthorized\r\nContent-Length: 60\r\n\r\n{\"error\":{\"code\":-32001,\"message\":\"the token is wrong\"}}";
        let error = body_of(raw).unwrap_err();
        assert!(error.contains("401"), "{error}");
        assert!(error.contains("the token is wrong"), "{error}");
    }

    #[test]
    fn a_2xx_reply_is_the_body_alone() {
        let raw = b"HTTP/1.1 200 OK\r\nContent-Length: 11\r\n\r\n{\"result\":1}";
        assert_eq!(body_of(raw).unwrap(), r#"{"result":1}"#);
    }
}
