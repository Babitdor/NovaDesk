//! The MCP endpoint, exercised over a real socket.
//!
//! Nothing here is mocked at the protocol level: an ephemeral loopback listener,
//! the same handler table the app serves, and one JSON-RPC message per request.
//! The caller is a stub only because a Tauri AppHandle needs a window; every
//! native handler that needs nothing but paths (list_projects, the moodboards,
//! the search) runs for real.
//!
//! It runs in the normal suite rather than being ignored like the speak tests:
//! the socket is loopback and ephemeral, so there is nothing for CI to be
//! surprised by, and the security checks below are worth failing a build over.

use std::sync::Arc;

use novacode_lib::tools::{
    self,
    dispatch::CallContext,
    server::{serve, Caller, Config},
    ToolsState, TOOLS,
};
use serde_json::{json, Value};

/// Registers a temp project list and returns a caller bound to it.
fn caller(workspace: std::path::PathBuf, data: std::path::PathBuf) -> Caller {
    Arc::new(move |name: &str, arguments: Value| {
        let name = name.to_string();
        let workspace = workspace.clone();
        let data = data.clone();
        Box::pin(async move {
            let context = CallContext {
                app: None,
                state: None,
                workspace: Some(workspace),
                data_dir: Some(data),
            };
            let tools = ToolsState::default();
            tools::dispatch::call(&context, &tools, &name, arguments).await
        })
    })
}

/// One HTTP/1.1 POST, returning the status and the body.
async fn post(authority: &str, token: Option<&str>, body: &Value) -> (u16, Value) {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let mut stream = tokio::net::TcpStream::connect(authority)
        .await
        .expect("connect");
    let body = body.to_string();
    let auth = token
        .map(|token| format!("Authorization: Bearer {token}\r\n"))
        .unwrap_or_default();
    let request = format!(
        "POST /mcp HTTP/1.1\r\nHost: {authority}\r\n{auth}Content-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    stream.write_all(request.as_bytes()).await.expect("write");
    let mut response = Vec::new();
    stream.read_to_end(&mut response).await.expect("read");
    let text = String::from_utf8_lossy(&response).into_owned();
    let (head, payload) = text.split_once("\r\n\r\n").expect("a header block");
    let status: u16 = head
        .split(' ')
        .nth(1)
        .and_then(|code| code.parse().ok())
        .expect("a status code");
    let payload = if payload.trim().is_empty() {
        Value::Null
    } else {
        serde_json::from_str(payload).expect("a JSON body")
    };
    (status, payload)
}

#[tokio::test(flavor = "multi_thread")]
async fn the_endpoint_speaks_mcp_over_a_socket() {
    // A temp workspace and app data folder, both real: list_projects answers
    // from the file set_workspace writes.
    let root = std::env::temp_dir().join(format!("nova-tools-{}", std::process::id()));
    let workspace = root.join("workspace");
    let data = root.join("data");
    std::fs::create_dir_all(&workspace).expect("workspace");
    std::fs::create_dir_all(&data).expect("data");
    tools::projects::register(&data, &workspace).expect("register");

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind");
    let authority = listener.local_addr().expect("address").to_string();
    let state = ToolsState::default();
    let token = state.token().to_string();
    tokio::spawn(serve(
        listener,
        Arc::new(Config {
            state: state.clone(),
            authority: authority.clone(),
            caller: caller(workspace.clone(), data.clone()),
        }),
    ));

    // initialize
    let (status, body) = post(
        &authority,
        Some(&token),
        &json!({
            "jsonrpc": "2.0", "id": 1, "method": "initialize",
            "params": { "protocolVersion": "2025-06-18", "capabilities": {}, "clientInfo": {"name": "test"} }
        }),
    )
    .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["result"]["serverInfo"]["name"], "nova-desktop");
    assert!(body["result"]["protocolVersion"].is_string());

    // tools/list: exactly the registry, each with an object schema.
    let (status, body) = post(
        &authority,
        Some(&token),
        &json!({ "jsonrpc": "2.0", "id": 2, "method": "tools/list" }),
    )
    .await;
    assert_eq!(status, 200);
    let listed = body["result"]["tools"].as_array().expect("a tool list");
    assert_eq!(listed.len(), TOOLS.len());
    assert!(listed
        .iter()
        .all(|tool| tool["inputSchema"]["type"] == "object"));

    // tools/call for a native handler answers from the temp project list.
    let (status, body) = post(
        &authority,
        Some(&token),
        &json!({
            "jsonrpc": "2.0", "id": 3, "method": "tools/call",
            "params": { "name": "list_projects", "arguments": {} }
        }),
    )
    .await;
    assert_eq!(status, 200);
    assert_eq!(body["result"]["isError"], false, "{body}");
    let answer: Value =
        serde_json::from_str(body["result"]["content"][0]["text"].as_str().expect("text"))
            .expect("the tool answer is JSON");
    assert_eq!(
        answer["projects"][0]["path"],
        workspace.display().to_string()
    );

    // An unknown tool is a JSON-RPC error, not a panic and not a 500.
    let (status, body) = post(
        &authority,
        Some(&token),
        &json!({
            "jsonrpc": "2.0", "id": 4, "method": "tools/call",
            "params": { "name": "make_sandwiches", "arguments": {} }
        }),
    )
    .await;
    assert_eq!(status, 200);
    assert_eq!(body["error"]["code"], -32602);
    assert!(body["result"].is_null());

    // No token is 401, and so is the wrong one.
    for token in [None, Some("not-the-token")] {
        let (status, body) = post(
            &authority,
            token,
            &json!({ "jsonrpc": "2.0", "id": 5, "method": "tools/list" }),
        )
        .await;
        assert_eq!(status, 401, "{body}");
    }

    // A non-loopback Host is refused even with the right token (DNS rebinding).
    {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let mut stream = tokio::net::TcpStream::connect(&authority)
            .await
            .expect("connect");
        let body = json!({ "jsonrpc": "2.0", "id": 6, "method": "tools/list" }).to_string();
        let request = format!(
            "POST /mcp HTTP/1.1\r\nHost: evil.example:80\r\nAuthorization: Bearer {token}\r\nContent-Length: {}\r\n\r\n{body}",
            body.len()
        );
        stream.write_all(request.as_bytes()).await.expect("write");
        let mut response = Vec::new();
        stream.read_to_end(&mut response).await.expect("read");
        let text = String::from_utf8_lossy(&response);
        assert!(text.starts_with("HTTP/1.1 403"), "{text}");
    }

    // After the window goes away the endpoint refuses rather than half-answering.
    state.shutdown();
    let (status, body) = post(
        &authority,
        Some(&token),
        &json!({ "jsonrpc": "2.0", "id": 7, "method": "tools/list" }),
    )
    .await;
    assert_eq!(status, 503, "{body}");

    let _ = std::fs::remove_dir_all(&root);
}
