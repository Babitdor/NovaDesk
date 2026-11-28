//! The loopback file server behind the Files-pane HTML preview.
//!
//! A framed page needs a real origin. `srcdoc` would leave it without a base
//! URL — its own stylesheet, scripts and relative images would all 404 — and the
//! embedded webview refuses `file://` URLs outright. So the workspace is served
//! over loopback HTTP, hand-rolled on tokio's `TcpListener` for the same reason
//! `tools::server` is: this app does not take a dependency to answer GET.
//!
//! What this server can do is read every file in the workspace, so six rules
//! run before a byte leaves the disk:
//!
//! 1. it binds `127.0.0.1:0`, never a public interface;
//! 2. every path carries a per-launch random secret, so a page that discovers
//!    the port still cannot name a file (a phone-home guard, not an auth
//!    system: the only reader this is protecting is the app itself);
//! 3. GET and HEAD only — nothing here writes, and no request body is read;
//! 4. the path is percent-decoded and then resolved through
//!    `WorkspaceGuard::resolve`, which canonicalizes before the containment
//!    check, so `..`, absolute paths and symlinks cannot leave the root;
//! 5. a directory serves its own `index.html` or nothing at all, because a
//!    listing hands the shape of the workspace to whatever framed the server;
//! 6. `Cache-Control: no-store`, so the panel's refresh actually re-reads.
//!
//! One server per workspace root, started on the first preview and replaced when
//! the folder changes: the guard it holds is the folder.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio_util::sync::CancellationToken;

use crate::error::AppError;
use crate::workspace::WorkspaceGuard;
use crate::AppState;

/// A preview larger than this is a mistake, not a page: the webview would spend
/// longer parsing it than the reader spends looking at it.
const MAX_FILE: usize = 32 * 1024 * 1024;
const MAX_HEAD: usize = 8 * 1024;
const READ_TIMEOUT: Duration = Duration::from_secs(10);

/// The running server, or the handle the first preview leaves behind for the
/// next one.
pub struct PreviewServer {
    root: PathBuf,
    /// `http://127.0.0.1:<port>`, without a trailing slash.
    base: String,
    secret: String,
    /// Cancelled when another root replaces this server.
    stop: CancellationToken,
}

/// Shared through `AppState`. A plain `std` mutex, not tokio's: nothing awaits
/// while it is held — the bind happens outside it, on purpose.
pub type PreviewSlot = Arc<Mutex<Option<PreviewServer>>>;

#[tauri::command]
pub async fn preview_open(
    path: String,
    state: tauri::State<'_, AppState>,
) -> Result<String, AppError> {
    let guard = state.require_guard().await?;
    let file = guard.resolve(&path)?;
    if !file.is_file() {
        return Err(AppError::NotAFile(path));
    }

    let (base, secret) = PreviewServer::ensure(&state.preview, &guard).await?;
    Ok(format!(
        "{base}/{secret}/{}",
        encode_path(&guard.display_relative(&file))
    ))
}

impl PreviewServer {
    /// The server for this root, started on first use.
    ///
    /// The slot is read, released, and written again around the bind: holding a
    /// `MutexGuard` across `.await` is a compile error here, and two previews
    /// racing each other only costs one extra listener either way.
    async fn ensure(
        slot: &PreviewSlot,
        guard: &WorkspaceGuard,
    ) -> Result<(String, String), AppError> {
        if let Some(found) = current(slot, guard.root()) {
            return Ok(found);
        }

        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .map_err(|err| AppError::Io(format!("could not open the preview server: {err}")))?;
        let port = listener
            .local_addr()
            .map_err(|err| AppError::Io(err.to_string()))?
            .port();
        let secret = uuid::Uuid::new_v4().simple().to_string();
        let stop = CancellationToken::new();

        tokio::spawn(serve(listener, guard.clone(), secret.clone(), stop.clone()));

        let server = PreviewServer {
            root: guard.root().to_path_buf(),
            base: format!("http://127.0.0.1:{port}"),
            secret,
            stop,
        };
        let url = (server.base.clone(), server.secret.clone());
        let replaced = slot
            .lock()
            .expect("preview slot mutex poisoned")
            .replace(server);
        if let Some(previous) = replaced {
            // The old listener is still answering on its own port; stopping it
            // is what keeps a stale secret from reading the new folder, and it
            // lets a closed workspace release its port instead of leaking it.
            previous.stop.cancel();
        }
        Ok(url)
    }
}

fn current(slot: &PreviewSlot, root: &Path) -> Option<(String, String)> {
    let slot = slot.lock().ok()?;
    let server = slot.as_ref()?;
    (server.root == root).then(|| (server.base.clone(), server.secret.clone()))
}

/// Serves until another root replaces this server, or the process ends.
async fn serve(
    listener: TcpListener,
    guard: WorkspaceGuard,
    secret: String,
    stop: CancellationToken,
) {
    loop {
        let accepted = tokio::select! {
            _ = stop.cancelled() => break,
            accepted = listener.accept() => accepted,
        };
        let (stream, _) = match accepted {
            Ok(accepted) => accepted,
            Err(_) => break,
        };
        let guard = guard.clone();
        let secret = secret.clone();
        tokio::spawn(async move {
            let _ = handle(stream, &guard, &secret).await;
        });
    }
}

async fn handle(
    mut stream: TcpStream,
    guard: &WorkspaceGuard,
    secret: &str,
) -> std::io::Result<()> {
    let text = "text/plain; charset=utf-8";
    let head = match tokio::time::timeout(READ_TIMEOUT, read_head(&mut stream)).await {
        Ok(Ok(head)) => head,
        _ => {
            return respond(
                &mut stream,
                400,
                text,
                b"the request head is malformed\n",
                false,
            )
            .await
        }
    };

    let mut line = head.split("\r\n").next().unwrap_or_default().split(' ');
    let method = line.next().unwrap_or_default();
    let target = line.next().unwrap_or_default();
    // HEAD is what the webview probes with; the body is computed either way and
    // left off the wire.
    if method != "GET" && method != "HEAD" {
        return respond(
            &mut stream,
            405,
            text,
            b"only GET and HEAD are served\n",
            false,
        )
        .await;
    }

    let Some(found) = relative(target, secret).and_then(|rel| locate(guard, &rel)) else {
        return respond(&mut stream, 404, text, b"not found\n", method == "HEAD").await;
    };
    let bytes = match std::fs::read(&found) {
        Ok(bytes) => bytes,
        Err(_) => return respond(&mut stream, 404, text, b"not found\n", method == "HEAD").await,
    };
    if bytes.len() > MAX_FILE {
        return respond(
            &mut stream,
            413,
            text,
            b"the file is too large to serve\n",
            method == "HEAD",
        )
        .await;
    }
    // Chromium scrubs audio and video, and lays out a PDF, with ranged requests:
    // without this a media preview would download the whole file and then refuse
    // to seek.
    if let Some((first, last)) = requested_range(&head, bytes.len()) {
        return respond_range(
            &mut stream,
            content_type(&found),
            &bytes,
            first,
            last,
            method == "HEAD",
        )
        .await;
    }
    respond(
        &mut stream,
        200,
        content_type(&found),
        &bytes,
        method == "HEAD",
    )
    .await
}

/// The workspace-relative path a request asks for, or `None` when it must not be
/// served at all. Pure, so the traversal rules have tests that never open a
/// socket.
fn relative(target: &str, secret: &str) -> Option<String> {
    // A query string is a cache-buster here, not part of a name; a fragment never
    // reaches a server but a hand-written request may carry one.
    let path = target.split(['?', '#']).next()?;
    let decoded = percent_decode(path)?;
    let prefix = format!("/{secret}/");
    let rest = decoded.strip_prefix(prefix.as_str())?;
    // A backslash is a separator on Windows and a colon is a drive letter, so a
    // name holding either could address a path the segment check never saw.
    if rest.is_empty() || rest.contains('\\') || rest.contains(':') {
        return None;
    }
    let mut clean: Vec<&str> = Vec::new();
    for segment in rest.split('/') {
        match segment {
            "" | "." => {}
            ".." => return None,
            _ => clean.push(segment),
        }
    }
    (!clean.is_empty()).then(|| clean.join("/"))
}

/// Resolves a checked relative path. A directory answers with its own
/// `index.html` and nothing else; a missing file answers `None`, which the
/// handler turns into the same 404 as a bad secret.
fn locate(guard: &WorkspaceGuard, relative: &str) -> Option<PathBuf> {
    let found = guard.resolve(relative).ok()?;
    if found.is_dir() {
        return guard.resolve(&format!("{relative}/index.html")).ok();
    }
    Some(found)
}

/// Byte-level, because the decode has to happen before the segment check —
/// `%2e%2e` is `..` and a string-level decoder would let it through.
fn percent_decode(text: &str) -> Option<String> {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' {
            let pair = std::str::from_utf8(bytes.get(index + 1..index + 3)?).ok()?;
            out.push(u8::from_str_radix(pair, 16).ok()?);
            index += 3;
        } else {
            out.push(bytes[index]);
            index += 1;
        }
    }
    // A path that is not UTF-8 cannot be a workspace path: the UI only ever sees
    // forward-slashed strings.
    String::from_utf8(out).ok()
}

/// Percent-encodes what a URL cannot carry verbatim, so a file called
/// `my file #1.html` still arrives as one name.
fn encode_path(relative: &str) -> String {
    let mut out = String::with_capacity(relative.len());
    for byte in relative.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~' | b'/') {
            out.push(byte as char);
        } else {
            out.push_str(&format!("%{byte:02X}"));
        }
    }
    out
}

fn content_type(path: &Path) -> &'static str {
    if let Some(image) = crate::commands::workspace::mime_for(path) {
        return image;
    }
    match path
        .extension()
        .and_then(|ext| ext.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase()
        .as_str()
    {
        "html" | "htm" => "text/html; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        "js" | "mjs" => "text/javascript; charset=utf-8",
        "json" => "application/json",
        "svg" => "image/svg+xml",
        "md" | "markdown" | "txt" | "log" => "text/plain; charset=utf-8",
        "woff2" => "font/woff2",
        "woff" => "font/woff",
        "ttf" => "font/ttf",
        "wasm" => "application/wasm",
        "pdf" => "application/pdf",
        "mp4" => "video/mp4",
        "webm" => "video/webm",
        "mp3" => "audio/mpeg",
        "wav" => "audio/wav",
        // Anything else is an unknown binary; a wrong type would make the
        // webview parse garbage as HTML, which is the one mistake worth avoiding.
        _ => "application/octet-stream",
    }
}

async fn read_head(stream: &mut TcpStream) -> Result<String, String> {
    let mut buffer = Vec::with_capacity(1024);
    loop {
        if let Some(index) = find(&buffer, b"\r\n\r\n") {
            buffer.truncate(index + 4);
            return Ok(String::from_utf8_lossy(&buffer).into_owned());
        }
        if buffer.len() > MAX_HEAD {
            return Err("the request head is too large".into());
        }
        let mut chunk = [0_u8; 1024];
        let count = stream
            .read(&mut chunk)
            .await
            .map_err(|_| "the connection failed".to_string())?;
        if count == 0 {
            return Err("the connection closed before the request".into());
        }
        buffer.extend_from_slice(&chunk[..count]);
    }
}

fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

/// The single range a request asks for, as inclusive `(first, last)` bytes.
///
/// Only the `bytes=` form matters, and only one of them: this server answers whole
/// files, and a client asking for several ranges at once is asking a question no
/// preview needs answered. Anything unparseable answers `None`, which serves the
/// whole file — a client that cannot use a 206 still gets its bytes.
fn requested_range(head: &str, len: usize) -> Option<(usize, usize)> {
    if len == 0 {
        return None;
    }
    let value = head.lines().find_map(|line| {
        let (name, rest) = line.split_once(':')?;
        name.eq_ignore_ascii_case("range").then(|| rest.trim())
    })?;
    let spec = value.strip_prefix("bytes=")?;
    if spec.contains(',') {
        return None;
    }
    let (first, last) = spec.split_once('-')?;
    let (start, end) = match (first.trim(), last.trim()) {
        // `-500`: the last 500 bytes, which is how a player asks for a tail.
        ("", suffix) => (len.saturating_sub(suffix.parse().ok()?), len - 1),
        (start, "") => (start.parse().ok()?, len - 1),
        (start, end) => (start.parse().ok()?, end.parse().ok()?),
    };
    (start <= end && end < len).then_some((start, end))
}

/// The header block every response shares. `extra` carries `Content-Range` for a
/// 206 and nothing at all otherwise.
fn headers(status: u16, mime: &str, len: usize, extra: &str) -> String {
    let reason = match status {
        200 => "OK",
        206 => "Partial Content",
        400 => "Bad Request",
        404 => "Not Found",
        405 => "Method Not Allowed",
        413 => "Content Too Large",
        _ => "Internal Server Error",
    };
    format!(
        "HTTP/1.1 {status} {reason}\r\nContent-Type: {mime}\r\nContent-Length: {len}\r\nAccept-Ranges: bytes\r\n{extra}Connection: close\r\nCache-Control: no-store\r\nX-Content-Type-Options: nosniff\r\n\r\n"
    )
}

async fn respond(
    stream: &mut TcpStream,
    status: u16,
    mime: &str,
    body: &[u8],
    head_only: bool,
) -> std::io::Result<()> {
    send(
        stream,
        &headers(status, mime, body.len(), ""),
        body,
        head_only,
    )
    .await
}

/// A 206 for one range of `bytes`: the same headers, plus where the slice sits.
async fn respond_range(
    stream: &mut TcpStream,
    mime: &str,
    bytes: &[u8],
    first: usize,
    last: usize,
    head_only: bool,
) -> std::io::Result<()> {
    let extra = format!("Content-Range: bytes {first}-{last}/{}\r\n", bytes.len());
    send(
        stream,
        &headers(206, mime, last - first + 1, &extra),
        &bytes[first..=last],
        head_only,
    )
    .await
}

async fn send(
    stream: &mut TcpStream,
    head: &str,
    body: &[u8],
    head_only: bool,
) -> std::io::Result<()> {
    stream.write_all(head.as_bytes()).await?;
    if !head_only {
        stream.write_all(body).await?;
    }
    stream.flush().await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testtmp::TempDir;

    const SECRET: &str = "abc123";

    #[test]
    fn a_range_request_is_one_inclusive_slice_or_nothing() {
        let head = "GET /x HTTP/1.1\r\nRange: bytes=0-9\r\n\r\n";
        assert_eq!(requested_range(head, 100), Some((0, 9)));
        assert_eq!(requested_range("Range: bytes=90-\r\n", 100), Some((90, 99)));
        assert_eq!(requested_range("Range: bytes=-5\r\n", 100), Some((95, 99)));
        // Out of bounds, several ranges at once, another unit, and no range at all
        // are all "serve the whole file" rather than a refusal.
        assert_eq!(requested_range("Range: bytes=0-100\r\n", 100), None);
        assert_eq!(requested_range("Range: bytes=0-1,5-6\r\n", 100), None);
        assert_eq!(requested_range("Range: items=0-1\r\n", 100), None);
        assert_eq!(requested_range("Accept: */*\r\n", 100), None);
        assert_eq!(requested_range("Range: bytes=0-9\r\n", 0), None);
    }

    #[test]
    fn a_partial_response_names_the_slice_it_carries() {
        let head = headers(206, "audio/mpeg", 10, "Content-Range: bytes 5-14/100\r\n");
        assert!(head.starts_with("HTTP/1.1 206 Partial Content\r\n"));
        assert!(head.contains("Content-Length: 10\r\n"));
        assert!(head.contains("Accept-Ranges: bytes\r\n"));
        assert!(head.contains("Content-Range: bytes 5-14/100\r\n"));
    }

    #[test]
    fn a_nested_request_maps_to_its_relative_path() {
        assert_eq!(
            relative("/abc123/a/b.txt", SECRET).as_deref(),
            Some("a/b.txt")
        );
        assert_eq!(
            relative("/abc123/a/b.txt?v=1", SECRET).as_deref(),
            Some("a/b.txt")
        );
        // Decoded before the segment check, so an encoded space still names a file.
        assert_eq!(
            relative("/abc123/a%20b.txt", SECRET).as_deref(),
            Some("a b.txt")
        );
    }

    #[test]
    fn escapes_a_bad_secret_and_absolute_paths_are_refused() {
        assert_eq!(relative("/abc123/../secret.txt", SECRET), None);
        assert_eq!(relative("/abc123/a/%2e%2e/secret.txt", SECRET), None);
        assert_eq!(relative("/abc123/C:/Windows/win.ini", SECRET), None);
        assert_eq!(relative("/abc123/a\\b.txt", SECRET), None);
        assert_eq!(relative("/deadbeef/a.txt", SECRET), None);
        assert_eq!(relative("/abc123", SECRET), None);
    }

    #[test]
    fn a_directory_serves_its_own_index_and_a_missing_name_serves_nothing() {
        let tmp = TempDir::new("preview-index");
        std::fs::create_dir_all(tmp.path().join("site")).unwrap();
        std::fs::write(tmp.path().join("site/index.html"), "<p>hi</p>").unwrap();
        let guard = WorkspaceGuard::new(tmp.path()).unwrap();

        let found = locate(&guard, "site").unwrap();
        assert_eq!(guard.display_relative(&found), "site/index.html");
        assert!(locate(&guard, "missing").is_none());
        assert!(locate(&guard, "..").is_none());
    }

    /// The whole risk of a hand-rolled server is a header or a read that is off
    /// by one, so one request goes over a real socket rather than through a stub.
    #[tokio::test]
    async fn answers_a_real_request_over_loopback() {
        let tmp = TempDir::new("preview-socket");
        std::fs::write(tmp.path().join("index.html"), "<p>hi</p>").unwrap();
        let guard = WorkspaceGuard::new(tmp.path()).unwrap();

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        tokio::spawn(serve(
            listener,
            guard,
            SECRET.to_string(),
            CancellationToken::new(),
        ));

        let mut client = TcpStream::connect(address).await.unwrap();
        client
            .write_all(
                format!("GET /{SECRET}/index.html HTTP/1.1\r\nHost: {address}\r\n\r\n").as_bytes(),
            )
            .await
            .unwrap();
        let mut answer = Vec::new();
        client.read_to_end(&mut answer).await.unwrap();
        let answer = String::from_utf8_lossy(&answer);

        assert!(answer.starts_with("HTTP/1.1 200 OK"), "got {answer}");
        assert!(
            answer.contains("Content-Type: text/html; charset=utf-8"),
            "got {answer}"
        );
        assert!(answer.contains("Cache-Control: no-store"), "got {answer}");
        assert!(answer.ends_with("<p>hi</p>"), "got {answer}");
    }
}
