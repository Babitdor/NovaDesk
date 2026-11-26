//! The local MCP endpoint the shim talks to.
//!
//! Deliberately small: one route (/mcp), JSON-RPC 2.0, three methods, and no
//! new dependency. Nova already shells out to curl for its outbound HTTP and
//! never links a TLS stack, so hosting a loopback-only server with tokio's
//! TcpListener and a hand-rolled HTTP/1.1 reader keeps the dependency graph
//! exactly as wide as it was.
//!
//! What this endpoint can do is a privilege boundary: a caller that reaches it
//! can open windows, type into pages and run searches. So four checks run
//! before a single byte of JSON is parsed:
//!
//! 1. the Host must be this loopback authority (a DNS-rebinding guard),
//! 2. no Origin, ever: the caller is a native process, not a browser,
//! 3. a bearer token minted per launch, kept in memory and handed to the shim
//!    through its environment (never argv, which any process can read),
//! 4. the master switch, which also turns off when the window goes away,
//!    because a closed window has nobody to answer the task tools.
//!
//! JSON-RPC errors travel back in a 200 response, as the MCP transports expect;
//! only failures of the four checks above use an HTTP status.

use std::sync::Arc;
use std::time::Duration;

use serde_json::{json, Value};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

use crate::agent::BoxFut;
use crate::error::AppError;
use crate::tools::ToolsState;

/// A request bigger than this is not a tool call, it is an attempt to run the
/// server out of memory.
const MAX_REQUEST: usize = 512 * 1024;
const MAX_HEAD: usize = 8 * 1024;
const READ_TIMEOUT: Duration = Duration::from_secs(15);

/// What the protocol version this server implements is called.
const PROTOCOL: &str = "2025-06-18";

/// Answers one tool call. Boxed rather than generic so the server can be tested
/// with a stub and run in the app with the real dispatcher.
pub type Caller =
    Arc<dyn Fn(&str, Value) -> BoxFut<'static, Result<Value, AppError>> + Send + Sync>;

pub struct Config {
    pub state: ToolsState,
    /// The loopback authority this listener bound to, e.g. 127.0.0.1:51234.
    pub authority: String,
    pub caller: Caller,
}

/// Serves until the listener errors. Every connection is independent, so a
/// harness that opens two shims never queues behind one.
pub async fn serve(listener: TcpListener, config: Arc<Config>) {
    loop {
        let (stream, _) = match listener.accept().await {
            Ok(accepted) => accepted,
            Err(_) => break,
        };
        let config = config.clone();
        tokio::spawn(async move {
            let _ = handle(stream, &config).await;
        });
    }
}

async fn handle(mut stream: TcpStream, config: &Config) -> std::io::Result<()> {
    let read = tokio::time::timeout(READ_TIMEOUT, read_request(&mut stream)).await;
    let (status, body) = match read {
        Ok(Ok(request)) => answer(&request, config).await,
        Ok(Err(reason)) => (400, error_body(Value::Null, -32700, &reason)),
        Err(_) => (
            408,
            error_body(Value::Null, -32603, "the request did not arrive in time"),
        ),
    };
    write_response(&mut stream, status, &body).await
}

struct Request {
    method: String,
    path: String,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
}

impl Request {
    fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(key, _)| key.eq_ignore_ascii_case(name))
            .map(|(_, value)| value.trim())
    }
}

async fn read_request(stream: &mut TcpStream) -> Result<Request, String> {
    let mut buffer = Vec::with_capacity(1024);
    let head_end = loop {
        if let Some(index) = find(&buffer, b"\r\n\r\n") {
            break index + 4;
        }
        if buffer.len() > MAX_HEAD {
            return Err("the request head is too large".into());
        }
        let mut chunk = [0_u8; 2048];
        let count = stream
            .read(&mut chunk)
            .await
            .map_err(|_| "the connection failed".to_string())?;
        if count == 0 {
            return Err("the connection closed before the request".into());
        }
        buffer.extend_from_slice(&chunk[..count]);
    };
    let head = String::from_utf8_lossy(&buffer[..head_end]).into_owned();
    let (method, path, headers) = parse_head(&head)?;
    let length = headers
        .iter()
        .find(|(key, _)| key.eq_ignore_ascii_case("content-length"))
        .and_then(|(_, value)| value.trim().parse::<usize>().ok())
        .unwrap_or(0);
    if length > MAX_REQUEST {
        return Err("the request body is too large".into());
    }
    let mut body = buffer[head_end..].to_vec();
    while body.len() < length {
        let mut chunk = vec![0_u8; length - body.len()];
        let count = stream
            .read(&mut chunk)
            .await
            .map_err(|_| "the connection failed".to_string())?;
        if count == 0 {
            return Err("the body was shorter than its Content-Length".into());
        }
        body.extend_from_slice(&chunk[..count]);
    }
    body.truncate(length);
    Ok(Request {
        method,
        path,
        headers,
        body,
    })
}

/// A request line and its headers, split out of the head block.
type Head = (String, String, Vec<(String, String)>);

fn parse_head(head: &str) -> Result<Head, String> {
    let mut lines = head.split("\r\n");
    let request_line = lines.next().ok_or("the request line is missing")?;
    let mut parts = request_line.split(' ');
    let method = parts.next().unwrap_or_default().to_string();
    let path = parts.next().unwrap_or_default().to_string();
    if method.is_empty() || path.is_empty() {
        return Err("the request line is malformed".into());
    }
    let mut headers = Vec::new();
    for line in lines {
        if line.is_empty() {
            break;
        }
        if let Some((key, value)) = line.split_once(':') {
            headers.push((key.trim().to_string(), value.trim().to_string()));
        }
    }
    Ok((method, path, headers))
}

fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

/// The four guards, then the protocol.
async fn answer(request: &Request, config: &Config) -> (u16, Value) {
    if request.method != "POST" || request.path != "/mcp" {
        return (
            404,
            error_body(Value::Null, -32601, "only POST /mcp is served"),
        );
    }
    let host = request.header("host").unwrap_or_default();
    if !host.eq_ignore_ascii_case(&config.authority)
        && !host.eq_ignore_ascii_case(&alternate(&config.authority))
    {
        return (
            403,
            error_body(Value::Null, -32600, "the Host is not this endpoint"),
        );
    }
    if request.header("origin").is_some() {
        return (
            403,
            error_body(
                Value::Null,
                -32600,
                "the shim is a native process; a browser origin may not call these tools",
            ),
        );
    }
    let presented = request
        .header("authorization")
        .and_then(|value| value.strip_prefix("Bearer "))
        .unwrap_or_default();
    if !constant_time_eq(presented.as_bytes(), config.state.token().as_bytes()) {
        return (
            401,
            error_body(Value::Null, -32001, "the bearer token is missing or wrong"),
        );
    }
    if !config.state.enabled() {
        return (
            503,
            error_body(Value::Null, -32002, "Nova Desktop Tools are switched off"),
        );
    }
    let call: Value = match serde_json::from_slice(&request.body) {
        Ok(value) => value,
        Err(_) => return (400, error_body(Value::Null, -32700, "the body is not JSON")),
    };
    match dispatch(&call, config).await {
        // A notification carries no id, so it gets no body at all.
        Value::Null => (202, Value::Null),
        answer => (200, answer),
    }
}

/// The same authority with localhost spelled out, because a shim built from
/// NOVA_TOOLS_URL and a client that reformats it must agree.
fn alternate(authority: &str) -> String {
    authority.replacen("127.0.0.1", "localhost", 1)
}

async fn dispatch(call: &Value, config: &Config) -> Value {
    let id = call.get("id").cloned().unwrap_or(Value::Null);
    if call.get("jsonrpc").and_then(Value::as_str) != Some("2.0") {
        return error_body(id, -32600, "this endpoint speaks JSON-RPC 2.0");
    }
    let method = call
        .get("method")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let params = call.get("params").cloned().unwrap_or_else(|| json!({}));
    match method {
        "initialize" => result_body(
            id,
            json!({
                "protocolVersion": PROTOCOL,
                "capabilities": { "tools": { "listChanged": false } },
                "serverInfo": { "name": "nova-desktop", "version": env!("CARGO_PKG_VERSION") },
            }),
        ),
        "tools/list" => result_body(id, json!({ "tools": listing() })),
        "ping" => result_body(id, json!({})),
        "tools/call" => {
            let name = params
                .get("name")
                .and_then(Value::as_str)
                .unwrap_or_default();
            let arguments = params
                .get("arguments")
                .cloned()
                .unwrap_or_else(|| json!({}));
            if !crate::tools::TOOLS.iter().any(|tool| tool.id == name) {
                return error_body(id, -32602, &format!("unknown tool: {name}"));
            }
            let outcome = (config.caller)(name, arguments).await;
            match outcome {
                Ok(value) => result_body(id, tool_content(value, false)),
                Err(err) => result_body(id, tool_content(json!(err.to_string()), true)),
            }
        }
        // A notification has no id, so it gets no response body at all.
        _ if call.get("id").is_none() => Value::Null,
        _ => error_body(id, -32601, &format!("unknown method: {method}")),
    }
}

/// The catalogue as an MCP client sees it.
fn listing() -> Vec<Value> {
    crate::tools::TOOLS
        .iter()
        .map(|tool| {
            json!({
                "name": tool.id,
                "description": tool.description,
                "inputSchema": serde_json::from_str::<Value>(tool.schema).unwrap_or_else(|_| json!({"type": "object"})),
            })
        })
        .collect()
}

fn tool_content(value: Value, is_error: bool) -> Value {
    json!({
        "content": [{ "type": "text", "text": value.to_string() }],
        "isError": is_error,
    })
}

fn result_body(id: Value, result: Value) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "result": result })
}

fn error_body(id: Value, code: i64, message: &str) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": message } })
}

async fn write_response(stream: &mut TcpStream, status: u16, body: &Value) -> std::io::Result<()> {
    let reason = match status {
        200 => "OK",
        202 => "Accepted",
        400 => "Bad Request",
        401 => "Unauthorized",
        403 => "Forbidden",
        404 => "Not Found",
        408 => "Request Timeout",
        503 => "Service Unavailable",
        _ => "Internal Server Error",
    };
    let body = if status == 202 {
        String::new()
    } else {
        body.to_string()
    };
    let head = format!(
        "HTTP/1.1 {status} {reason}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\nCache-Control: no-store\r\n\r\n",
        body.len()
    );
    stream.write_all(head.as_bytes()).await?;
    stream.write_all(body.as_bytes()).await?;
    stream.flush().await
}

/// The token is not a password, but comparing it in variable time would still
/// tell a patient local process how much of it is right.
fn constant_time_eq(left: &[u8], right: &[u8]) -> bool {
    if left.len() != right.len() {
        return false;
    }
    left.iter()
        .zip(right)
        .fold(0_u8, |acc, (a, b)| acc | (a ^ b))
        == 0
}

/// Starts the endpoint on an ephemeral loopback port and remembers where it is.
pub async fn start(app: tauri::AppHandle, state: crate::AppState) -> Result<String, AppError> {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .map_err(|err| AppError::Tools(format!("could not open the tools endpoint: {err}")))?;
    let address = listener
        .local_addr()
        .map_err(|err| AppError::Tools(err.to_string()))?;
    let authority = address.to_string();
    let endpoint = format!("http://{authority}/mcp");
    state.tools.set_endpoint(endpoint.clone());

    let tools = state.tools.clone();
    let caller: Caller = Arc::new(move |name: &str, arguments: Value| {
        // Owned, because the future this returns outlives the call: the
        // caller's &str is borrowed from a socket read that is already gone.
        let name = name.to_string();
        let app = app.clone();
        let state = state.clone();
        Box::pin(async move {
            let context = crate::tools::dispatch::CallContext::in_app(app, state.clone()).await;
            let tools = state.tools.clone();
            crate::tools::dispatch::call(&context, &tools, &name, arguments).await
        })
    });
    let config = Arc::new(Config {
        state: tools,
        authority,
        caller,
    });
    tokio::spawn(serve(listener, config));
    Ok(endpoint)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_a_request_line_and_its_headers() {
        let (method, path, headers) =
            parse_head("POST /mcp HTTP/1.1\r\nHost: 127.0.0.1:1\r\nContent-Length: 12\r\n\r\n")
                .unwrap();
        assert_eq!(method, "POST");
        assert_eq!(path, "/mcp");
        assert_eq!(headers.len(), 2);
        assert_eq!(headers[1].1, "12");
    }

    #[test]
    fn rejects_a_head_without_a_request_line() {
        assert!(parse_head("").is_err());
        assert!(parse_head("POST\r\n").is_err());
    }

    #[test]
    fn compares_tokens_without_early_exit() {
        assert!(constant_time_eq(b"abc", b"abc"));
        assert!(!constant_time_eq(b"abc", b"abd"));
        assert!(!constant_time_eq(b"abc", b"ab"));
    }

    /// The pane's key and the server's method table must agree, or a tool is
    /// advertised and answers "unknown method" on the first call.
    #[test]
    fn the_listing_matches_the_registry() {
        let listing = listing();
        assert_eq!(listing.len(), crate::tools::TOOLS.len());
        assert!(listing
            .iter()
            .all(|entry| entry["inputSchema"]["type"] == "object"));
    }
}
