//! The shim, run as a real child process against a socket.
//!
//! This is the packaging path, not the protocol: the binary every harness starts
//! is spawned with the environment Nova gives it, one MCP frame goes in on
//! stdin, and one JSON-RPC message must come back on stdout. A shim that
//! compiled but could not read its credentials would pass a unit test and fail
//! every harness, which is exactly what this catches.
//!
//! The credentials file is the production shape: Nova writes it per turn and
//! passes its path, so the endpoint and the token never appear on a command
//! line. NOVA_TOOLS_URL/TOKEN are covered too, because they are what a
//! hand-written configuration uses.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::path::PathBuf;
use std::process::{Command, Stdio};

/// Where cargo put the shim. It is a workspace member, so its binary lands in
/// the same target directory as the app's tests.
fn shim() -> PathBuf {
    let profile = if cfg!(debug_assertions) {
        "debug"
    } else {
        "release"
    };
    let name = if cfg!(windows) {
        "nova-tools.exe"
    } else {
        "nova-tools"
    };
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("target")
        .join(profile)
        .join(name)
}

/// Runs the shim with one initialize frame and returns the single reply line.
fn run(environment: &[(&str, &str)]) -> String {
    let binary = shim();
    assert!(
        binary.is_file(),
        "the shim is missing at {}; run npm run tools:shim first",
        binary.display()
    );

    let mut child = Command::new(&binary)
        .envs(environment.iter().copied())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("the shim starts");

    {
        let mut stdin = child.stdin.take().expect("stdin");
        writeln!(
            stdin,
            r#"{{"jsonrpc":"2.0","id":1,"method":"initialize","params":{{}}}}"#
        )
        .expect("write a frame");
    }

    let mut stdout = BufReader::new(child.stdout.take().expect("stdout"));
    let mut line = String::new();
    stdout.read_line(&mut line).expect("read the reply");
    let mut diagnostics = String::new();
    if let Some(mut stderr) = child.stderr.take() {
        let _ = stderr.read_to_string(&mut diagnostics);
    }
    let _ = child.wait();
    assert!(
        !line.trim().is_empty(),
        "no reply; stderr was {diagnostics}"
    );
    line
}

/// A one-request stand-in for the endpoint that echoes whether it saw the token.
fn serve_three(listener: TcpListener, authority: String) -> std::thread::JoinHandle<()> {
    std::thread::spawn(move || {
        for _ in 0..3 {
            let Ok((mut stream, _)) = listener.accept() else {
                return;
            };
            let mut buffer = [0_u8; 8192];
            let count = stream.read(&mut buffer).unwrap_or_default();
            let request = String::from_utf8_lossy(&buffer[..count]).to_string();
            assert!(request.starts_with("POST /mcp HTTP/1.1"), "{request}");
            assert!(request.contains(&format!("Host: {authority}")), "{request}");
            let (status, body) = if request.contains("Authorization: Bearer good-token") {
                (
                    "200 OK",
                    r#"{"jsonrpc":"2.0","id":1,"result":{"protocolVersion":"2025-06-18","serverInfo":{"name":"nova-desktop"}}}"#
                        .to_string(),
                )
            } else {
                (
                    "401 Unauthorized",
                    r#"{"jsonrpc":"2.0","id":1,"error":{"code":-32001,"message":"the token is wrong"}}"#
                        .to_string(),
                )
            };
            let response = format!(
                "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            let _ = stream.write_all(response.as_bytes());
        }
    })
}

#[test]
fn the_shim_proxies_a_frame_and_reports_a_refused_token() {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind a loopback port");
    let authority = listener.local_addr().expect("address").to_string();
    let server = serve_three(listener, authority.clone());
    let url = format!("http://{authority}/mcp");

    // The hand-written shape: endpoint and token in the environment.
    let reply = run(&[("NOVA_TOOLS_URL", &url), ("NOVA_TOOLS_TOKEN", "good-token")]);
    assert!(reply.contains("protocolVersion"), "{reply}");
    assert!(reply.contains("nova-desktop"), "{reply}");

    // The production shape: a credentials file, whose path is the only thing a
    // harness carries.
    let credentials = std::env::temp_dir().join(format!("nova-shim-{}.json", std::process::id()));
    std::fs::write(
        &credentials,
        r#"{"url":"__URL__","token":"good-token"}"#.replace("__URL__", &url),
    )
    .expect("write the credentials file");
    let reply = run(&[("NOVA_TOOLS_FILE", &credentials.display().to_string())]);
    assert!(reply.contains("protocolVersion"), "{reply}");

    // A refused token comes back in-band, so the harness sees why instead of
    // waiting for a reply that will never arrive.
    let refused = std::env::temp_dir().join(format!("nova-shim-bad-{}.json", std::process::id()));
    std::fs::write(
        &refused,
        r#"{"url":"__URL__","token":"wrong-token"}"#.replace("__URL__", &url),
    )
    .expect("write the refused credentials");
    let reply = run(&[("NOVA_TOOLS_FILE", &refused.display().to_string())]);
    assert!(reply.contains("401"), "{reply}");
    assert!(reply.contains("the token is wrong"), "{reply}");

    server.join().expect("the stand-in server finishes");
    let _ = std::fs::remove_file(&credentials);
    let _ = std::fs::remove_file(&refused);
}
