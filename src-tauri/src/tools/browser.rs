//! Driving the browser pane: read a snapshot, click a ref, type, screenshot.
//!
//! The child webview is external content, so nothing here trusts the page:
//!
//! - refs name DOM nodes in the current document's latest snapshot. A new
//!   snapshot, navigation or replacement node invalidates old refs;
//! - every script is a fixed template with one JSON-quoted argument, so no
//!   model text is ever concatenated into JavaScript.
//!
//! The screenshot goes through the WebView2 controller's DevTools protocol
//! (Page.captureScreenshot) rather than a screen grab: it captures the pane
//! even when another window covers it, and it needs no new dependency beyond
//! the COM bindings wry already links.

use std::time::Duration;

use serde_json::{json, Value};
use tauri::{AppHandle, Manager, Webview, Wry};

use crate::error::AppError;

/// The child webview created by commands::browser.
const BROWSER_LABEL: &str = "browser";

/// A page answering slower than this is not going to help a tool call.
const PAGE_TIMEOUT: Duration = Duration::from_secs(15);

fn child(app: &AppHandle) -> Result<Webview<Wry>, AppError> {
    app.get_webview(BROWSER_LABEL)
        .ok_or_else(|| AppError::Tools("no browser is open: call open_browser first".into()))
}

/// Runs one script and returns whatever the page produced as JSON.
async fn evaluate(app: &AppHandle, script: String) -> Result<Value, AppError> {
    let view = child(app)?;
    let (tx, rx) = tokio::sync::oneshot::channel::<String>();
    let sender = std::sync::Arc::new(std::sync::Mutex::new(Some(tx)));
    view.eval_with_callback(script, move |result| {
        if let Some(tx) = sender.lock().expect("callback mutex poisoned").take() {
            let _ = tx.send(result);
        }
    })
    .map_err(|err| AppError::Tools(format!("could not run the script: {err}")))?;
    let raw = tokio::time::timeout(PAGE_TIMEOUT, rx)
        .await
        .map_err(|_| AppError::Tools("the page did not answer in time".into()))?
        .map_err(|_| AppError::Tools("the browser closed before answering".into()))?;
    serde_json::from_str(&raw).map_err(|_| {
        let tail: String = raw.chars().take(200).collect();
        AppError::Tools(format!(
            "the page answered with something that is not JSON: {tail}"
        ))
    })
}

/// The snapshot script. One expression, because that is what an evaluated
/// script has to be; the element cap arrives as a placeholder so the script
/// text itself stays a constant.
const SNAPSHOT: &str = r#"(() => {
  const limit = __LIMIT__;
  const prefix = __PREFIX__;
  const body = document.body ? document.body.innerText : '';
  const text = body.replace(/\s+/g, ' ').trim();
  const nodes = document.querySelectorAll('a,button,input,textarea,select,[role="button"],[role="link"],[contenteditable]');
  const elements = [];
  const refs = new Map();
  for (const node of nodes) {
    if (elements.length >= limit) break;
    if (!node.getClientRects().length || getComputedStyle(node).visibility !== 'visible') continue;
    const field = node.matches('input,textarea,select') || node.isContentEditable;
    const raw = node.getAttribute('aria-label') || node.getAttribute('placeholder') ||
      (field ? node.getAttribute('name') : node.innerText) || '';
    const ref = prefix + elements.length;
    refs.set(ref, node);
    elements.push({
      ref,
      role: node.getAttribute('role') || node.tagName.toLowerCase(),
      name: String(raw).replace(/\s+/g, ' ').trim().slice(0, 80),
    });
  }
  document.__novaDesktopRefs = refs;
  return { url: location.href, title: document.title, text: text.slice(0, 4000), elements: elements };
})()"#;

pub async fn read(app: &AppHandle, max_elements: u64) -> Result<Value, AppError> {
    let limit = max_elements.clamp(1, 120);
    let prefix = json!(format!("e{}-", uuid::Uuid::new_v4().simple()));
    let script = SNAPSHOT
        .replace("__LIMIT__", &limit.to_string())
        .replace("__PREFIX__", &prefix.to_string());
    let mut snapshot = evaluate(app, script).await?;
    snapshot
        .as_object_mut()
        .ok_or_else(|| AppError::Tools("the page did not return a snapshot".into()))?
        .insert(
            "refsNote".into(),
            json!("read the page again after navigation or if a ref is stale"),
        );
    Ok(snapshot)
}

const CLICK: &str = r#"(() => {
  const el = document.__novaDesktopRefs?.get(__ARGS__);
  if (!el?.isConnected) return 'That ref is stale. Read the page again.';
  if (typeof el.click !== 'function' || el.matches(':disabled')) return 'The element cannot be clicked.';
  el.scrollIntoView({ block: 'center' });
  el.click();
  return true;
})()"#;

const TYPE: &str = r#"(() => {
  const [ref, text] = __ARGS__;
  const el = ref === null ? document.activeElement : document.__novaDesktopRefs?.get(ref);
  if (!el?.isConnected) return 'That ref is stale. Read the page again.';
  const plainInput = el instanceof HTMLInputElement && /^(text|search|tel|url|email|password|number)$/.test(el.type);
  const input = plainInput ? HTMLInputElement : el instanceof HTMLTextAreaElement ? HTMLTextAreaElement : null;
  if ((!input && !el.isContentEditable) || el.readOnly || el.matches(':disabled')) {
    return 'The element cannot accept text.';
    }
  el.focus();
  if (input) {
    const setValue = Object.getOwnPropertyDescriptor(input.prototype, 'value').set;
    setValue.call(el, text);
  } else {
    el.textContent = text;
  }
  el.dispatchEvent(new Event('input', { bubbles: true }));
  el.dispatchEvent(new Event('change', { bubbles: true }));
  return true;
})()"#;

async fn interact(app: &AppHandle, script: &str, arguments: Value) -> Result<(), AppError> {
    let outcome = evaluate(app, script.replace("__ARGS__", &arguments.to_string())).await?;
    match outcome {
        Value::Bool(true) => Ok(()),
        Value::String(message) => Err(AppError::Tools(message)),
        _ => Err(AppError::Tools(
            "the page returned an invalid action result".into(),
        )),
    }
}

pub async fn click(app: &AppHandle, reference: &str) -> Result<Value, AppError> {
    interact(app, CLICK, json!(reference)).await?;
    Ok(json!({ "clicked": reference }))
}

pub async fn type_text(
    app: &AppHandle,
    reference: Option<&str>,
    text: &str,
) -> Result<Value, AppError> {
    let length = text.chars().count();
    if length > 20_000 {
        return Err(AppError::Tools(
            "the text is longer than 20000 characters".into(),
        ));
    }
    interact(app, TYPE, json!([reference, text])).await?;
    Ok(json!({ "typed": length }))
}

pub async fn action(app: &AppHandle, action: &str) -> Result<Value, AppError> {
    let script = match action {
        "back" => "history.back()",
        "forward" => "history.forward()",
        "reload" => "location.reload()",
        other => return Err(AppError::Tools(format!("unknown browser action: {other}"))),
    };
    child(app)?
        .eval(script)
        .map_err(|err| AppError::Tools(err.to_string()))?;
    Ok(json!({ "action": action }))
}

/// A PNG of the pane, saved under the app data folder.
pub async fn capture(app: &AppHandle) -> Result<Value, AppError> {
    let png = screenshot(app).await?;
    let directory = app
        .path()
        .app_data_dir()
        .map_err(|err| AppError::Tools(err.to_string()))?
        .join("tool-captures");
    std::fs::create_dir_all(&directory).map_err(|err| AppError::from_io(err, &directory))?;
    let path = directory.join(format!("browser-{}.png", uuid::Uuid::new_v4()));
    std::fs::write(&path, &png).map_err(|err| AppError::from_io(err, &path))?;
    Ok(json!({
        "path": path.display().to_string(),
        "bytes": png.len(),
        "mechanism": "WebView2 DevTools Page.captureScreenshot",
    }))
}

#[cfg(windows)]
async fn screenshot(app: &AppHandle) -> Result<Vec<u8>, AppError> {
    use base64::Engine as _;
    use webview2_com::CallDevToolsProtocolMethodCompletedHandler;
    use windows_core::HSTRING;

    let view = child(app)?;
    let (tx, rx) = tokio::sync::oneshot::channel::<Result<String, String>>();
    let sender = std::sync::Arc::new(std::sync::Mutex::new(Some(tx)));
    view.with_webview(move |platform| {
        let controller = platform.controller();
        let core = match unsafe { controller.CoreWebView2() } {
            Ok(core) => core,
            Err(err) => {
                if let Some(tx) = sender.lock().expect("capture mutex poisoned").take() {
                    let _ = tx.send(Err(format!("no CoreWebView2: {err}")));
                }
                return;
            }
        };
        let reply = sender.clone();
        // webview2-com hands the callback the converted values, not the raw COM
        // ones: an HRESULT arrives as a Result and the PCWSTR as a String.
        let handler = CallDevToolsProtocolMethodCompletedHandler::create(Box::new(
            move |status: windows_core::Result<()>, body: String| {
                if let Some(tx) = reply.lock().expect("capture mutex poisoned").take() {
                    let _ = tx.send(match status {
                        Ok(()) => Ok(body),
                        Err(err) => Err(format!("Page.captureScreenshot failed: {err}")),
                    });
                }
                Ok(())
            },
        ));
        let method = HSTRING::from("Page.captureScreenshot");
        let arguments = HSTRING::from(r#"{"format":"png"}"#);
        if let Err(err) = unsafe { core.CallDevToolsProtocolMethod(&method, &arguments, &handler) }
        {
            if let Some(tx) = sender.lock().expect("capture mutex poisoned").take() {
                let _ = tx.send(Err(err.to_string()));
            }
        }
    })
    .map_err(|err| AppError::Tools(format!("could not reach the browser pane: {err}")))?;

    let reply = tokio::time::timeout(PAGE_TIMEOUT, rx)
        .await
        .map_err(|_| AppError::Tools("the screenshot did not arrive in time".into()))?
        .map_err(|_| AppError::Tools("the browser closed before the screenshot".into()))?
        .map_err(AppError::Tools)?;
    let parsed: Value = serde_json::from_str(&reply)
        .map_err(|_| AppError::Tools("the screenshot reply is not JSON".into()))?;
    let data = parsed
        .get("data")
        .and_then(Value::as_str)
        .ok_or_else(|| AppError::Tools("the screenshot reply carried no image".into()))?;
    base64::engine::general_purpose::STANDARD
        .decode(data)
        .map_err(|err| AppError::Tools(format!("the screenshot is not base64: {err}")))
}

/// Capture is Windows-only because the WebView2 controller is: the app already
/// requires it for the terminal dock and the browser pane.
#[cfg(not(windows))]
async fn screenshot(_app: &AppHandle) -> Result<Vec<u8>, AppError> {
    Err(AppError::Tools(
        "capturing the browser pane needs WebView2, so it is Windows-only".into(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn browser_scripts_are_single_expressions_with_one_of_each_placeholder() {
        for script in [SNAPSHOT, CLICK, TYPE] {
            assert!(script.starts_with("(() =>"));
            assert!(script.ends_with(")()"));
        }
        assert_eq!(SNAPSHOT.matches("__LIMIT__").count(), 1);
        assert_eq!(SNAPSHOT.matches("__PREFIX__").count(), 1);
        assert_eq!(CLICK.matches("__ARGS__").count(), 1);
        assert_eq!(TYPE.matches("__ARGS__").count(), 1);
    }
}
