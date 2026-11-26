use tauri::webview::{NewWindowResponse, PageLoadEvent, WebviewBuilder};
use tauri::{
    AppHandle, Emitter, EventTarget, LogicalPosition, LogicalSize, Manager, Url, Webview,
    WebviewUrl, Window, Wry,
};

use crate::error::AppError;

/// Loopback listeners, for both the pane's picker and the list_local_servers
/// tool. One implementation so the tool and the UI cannot disagree about what
/// is running.
#[cfg(windows)]
pub async fn discover_local_servers() -> Result<Vec<String>, AppError> {
    let system = std::env::var_os("SystemRoot")
        .ok_or_else(|| AppError::Io("SystemRoot is missing".into()))?;
    let output =
        tokio::process::Command::new(std::path::PathBuf::from(system).join("System32/netstat.exe"))
            .args(["-a", "-n"])
            .creation_flags(0x08000000) // CREATE_NO_WINDOW
            .kill_on_drop(true)
            .output()
            .await
            .map_err(|error| AppError::Io(error.to_string()))?;
    if !output.status.success() {
        return Err(AppError::Io(
            "Windows could not list local TCP listeners".into(),
        ));
    }
    Ok(local_servers(&String::from_utf8_lossy(&output.stdout)))
}

#[cfg(not(windows))]
pub async fn discover_local_servers() -> Result<Vec<String>, AppError> {
    Err(AppError::Agent(
        "Local server discovery requires Windows. Enter a URL or port to open it.".into(),
    ))
}

#[tauri::command]
pub async fn browser_local_servers() -> Result<Vec<String>, AppError> {
    discover_local_servers().await
}

#[cfg(any(windows, test))]
fn local_servers(output: &str) -> Vec<String> {
    use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};

    let mut listeners = output
        .lines()
        .filter_map(|line| {
            let mut fields = line.split_ascii_whitespace();
            if fields.next()? != "TCP" {
                return None;
            }
            let local: SocketAddr = fields.next()?.parse().ok()?;
            let remote: SocketAddr = fields.next()?.parse().ok()?;
            // An unspecified peer with port zero identifies a listening socket across locales.
            if !remote.ip().is_unspecified() || remote.port() != 0 || fields.next().is_none() {
                return None;
            }
            let ip = match local.ip() {
                IpAddr::V4(ip) if ip.is_unspecified() => IpAddr::V4(Ipv4Addr::LOCALHOST),
                IpAddr::V6(ip) if ip.is_unspecified() => IpAddr::V6(Ipv6Addr::LOCALHOST),
                ip if ip.is_loopback() => ip,
                _ => return None,
            };
            Some(SocketAddr::new(ip, local.port()))
        })
        .collect::<Vec<_>>();
    listeners.sort_unstable_by_key(|address| (address.port(), address.ip()));
    listeners.dedup();
    listeners
        .into_iter()
        .map(|address| address.to_string())
        .collect()
}

#[cfg(test)]
#[test]
fn discovers_loopback_listeners_without_probing_ports() {
    let output = "\
TCP  0.0.0.0:43210    0.0.0.0:0        ABHÖREN
TCP  127.0.0.1:43210  0.0.0.0:0        LISTENING
TCP  [::]:43211       [::]:0          LISTENING
TCP  127.0.0.1:50001  127.0.0.1:43210 ESTABLISHED
TCP  192.168.1.2:80   0.0.0.0:0        LISTENING
UDP  0.0.0.0:5353    *:*";
    assert_eq!(local_servers(output), ["127.0.0.1:43210", "[::1]:43211"]);
}

/// The window from `tauri.conf.json`; the browser child is one of its webviews.
const MAIN_WINDOW: &str = "main";

/// One browser, so the label is a constant rather than a parameter: a second
/// `add_child` with the same label is rejected by the runtime, and reusing the
/// child is what makes "open" behave like "navigate".
const BROWSER_LABEL: &str = "browser";

fn fail(err: tauri::Error) -> AppError {
    AppError::Agent(err.to_string())
}

fn parse(url: &str) -> Result<Url, AppError> {
    url.parse::<Url>()
        .ok()
        .filter(|url| matches!(url.scheme(), "http" | "https"))
        .ok_or_else(|| AppError::Agent("Enter an HTTP or HTTPS URL".into()))
}

fn main_window(app: &AppHandle<Wry>) -> Result<Window<Wry>, AppError> {
    app.get_window(MAIN_WINDOW)
        .ok_or_else(|| AppError::Agent(format!("no window labelled {MAIN_WINDOW}")))
}

fn place(view: &Webview<Wry>, x: f64, y: f64, width: f64, height: f64) -> Result<(), AppError> {
    view.set_position(LogicalPosition::new(x, y))
        .map_err(fail)?;
    view.set_size(LogicalSize::new(width, height)).map_err(fail)
}

/// Creates the browser child on first use, then points it at `url`.
///
/// Async on purpose: `add_child` blocks until the window thread has built the
/// child, and a sync command would be running on that very thread.
#[tauri::command]
pub async fn browser_open(
    url: String,
    x: f64,
    y: f64,
    width: f64,
    height: f64,
    app: AppHandle<Wry>,
) -> Result<(), AppError> {
    let target = parse(&url)?;

    if let Some(view) = app.get_webview(BROWSER_LABEL) {
        view.navigate(target).map_err(fail)?;
        place(&view, x, y, width, height)?;
        view.show().map_err(fail)?;
        return Ok(());
    }

    let window = main_window(&app)?;
    let view = window
        .add_child(
            WebviewBuilder::new(BROWSER_LABEL, WebviewUrl::External(target))
                .on_navigation(|url| matches!(url.scheme(), "http" | "https"))
                .on_new_window(|_, _| NewWindowResponse::Deny)
                .on_page_load(|view, page| {
                    if page.event() == PageLoadEvent::Finished {
                        let _ = view.app_handle().emit_to(
                            EventTarget::webview(MAIN_WINDOW),
                            "browser-location",
                            page.url().as_str(),
                        );
                    }
                }),
            LogicalPosition::new(x, y),
            LogicalSize::new(width, height),
        )
        .map_err(|e| AppError::Agent(format!("could not create the browser view: {e}")))?;
    // A child is created visible, but showing keeps both paths identical.
    view.show().map_err(fail)
}

/// Follows the pane. Called on every resize, so it is deliberately silent when
/// there is no browser yet: a closed tab is not an error.
#[tauri::command]
pub async fn browser_bounds(
    x: f64,
    y: f64,
    width: f64,
    height: f64,
    app: AppHandle<Wry>,
) -> Result<(), AppError> {
    let Some(view) = app.get_webview(BROWSER_LABEL) else {
        return Ok(());
    };
    place(&view, x, y, width, height)
}

/// A native child paints above the page, so hiding it is the only way to show a
/// tab underneath it.
#[tauri::command]
pub async fn browser_visibility(visible: bool, app: AppHandle<Wry>) -> Result<(), AppError> {
    let Some(view) = app.get_webview(BROWSER_LABEL) else {
        return Ok(());
    };
    let result = if visible { view.show() } else { view.hide() };
    result.map_err(fail)
}

#[tauri::command]
pub async fn browser_reload(app: AppHandle<Wry>) -> Result<(), AppError> {
    let Some(view) = app.get_webview(BROWSER_LABEL) else {
        return Ok(());
    };
    view.reload().map_err(fail)
}

/// Back and forward come from the child's own history. The direction is a closed
/// set because it reaches `eval`: anything else would be script injection through
/// a command that is not meant to run script.
#[tauri::command]
pub async fn browser_history(direction: String, app: AppHandle<Wry>) -> Result<(), AppError> {
    let script = match direction.as_str() {
        "back" => "history.back()",
        "forward" => "history.forward()",
        other => {
            return Err(AppError::Agent(format!(
                "unknown history direction: {other}"
            )))
        }
    };

    let Some(view) = app.get_webview(BROWSER_LABEL) else {
        return Ok(());
    };
    view.eval(script).map_err(fail)
}

#[tauri::command]
pub async fn browser_close(app: AppHandle<Wry>) -> Result<(), AppError> {
    if let Some(view) = app.get_webview(BROWSER_LABEL) {
        view.close().map_err(fail)?;
    }
    Ok(())
}

/// Teardown hook: a closing window must not keep a child webview alive. Called
/// from `lib.rs` next to `cancel_all`.
pub fn close_child(app: &AppHandle<Wry>) {
    if let Some(view) = app.get_webview(BROWSER_LABEL) {
        let _ = view.close();
    }
}
