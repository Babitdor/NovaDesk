//! Driving the browser pane: read a snapshot, click a ref, type, screenshot.
//!
//! The child webview is external content, so nothing here trusts the page:
//!
//! - read_browser returns refs ("e0", "e1"), never a selector, so a model
//!   cannot hand back CSS that reaches something the snapshot never offered;
//! - browser_click and browser_type resolve a ref through the snapshot this
//!   process stored, which also means a stale ref after a navigation fails
//!   instead of clicking whatever now sits in that spot;
//! - every script is a fixed template with one JSON-quoted argument, so no
//!   model text is ever concatenated into JavaScript.
//!
//! The screenshot goes through the WebView2 controller's DevTools protocol
//! (Page.captureScreenshot) rather than a screen grab: it captures the pane
//! even when another window covers it, and it needs no new dependency beyond
//! the COM bindings wry already links.

use std::collections::HashMap;
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
  const body = document.body ? document.body.innerText : '';
  const text = body.replace(/\s+/g, ' ').trim();
  const path = (node) => {
    const parts = [];
    let current = node;
    while (current && current.nodeType === 1 && current !== document.documentElement) {
      let index = 1;
      let sibling = current;
      while ((sibling = sibling.previousElementSibling)) {
        if (sibling.tagName === current.tagName) index += 1;
      }
      parts.unshift(current.tagName.toLowerCase() + ':nth-of-type(' + index + ')');
      if (parts.length >= 6) break;
      current = current.parentElement;
    }
    return parts.join(' > ');
  };
  const nodes = document.querySelectorAll('a,button,input,textarea,select,[role="button"],[role="link"],[contenteditable="true"]');
  const elements = [];
  let ref = 0;
  for (const node of Array.from(nodes)) {
    if (elements.length >= limit) break;
    const style = window.getComputedStyle(node);
    if (!style || style.display === 'none' || style.visibility === 'hidden') continue;
    const raw = node.getAttribute('aria-label') || node.getAttribute('placeholder') ||
      node.innerText || node.value || node.getAttribute('name') || '';
    elements.push({
      ref: 'e' + ref,
      role: node.getAttribute('role') || node.tagName.toLowerCase(),
      name: String(raw).replace(/\s+/g, ' ').trim().slice(0, 80),
      selector: path(node)
    });
    ref += 1;
  }
  return { url: location.href, title: document.title, text: text.slice(0, 4000), elements: elements };
})()"#;

pub async fn read(
    app: &AppHandle,
    tools: &crate::tools::ToolsState,
    max_elements: u64,
) -> Result<Value, AppError> {
    let limit = max_elements.clamp(1, 120);
    let script = SNAPSHOT.replace("__LIMIT__", &limit.to_string());
    let snapshot = evaluate(app, script).await?;

    // The selectors stay here: the model gets refs, and a ref is only ever
    // resolved against the snapshot that produced it.
    let mut selectors = HashMap::new();
    let mut visible = Vec::new();
    if let Some(elements) = snapshot.get("elements").and_then(Value::as_array) {
        for element in elements {
            let Some(reference) = element.get("ref").and_then(Value::as_str) else {
                continue;
            };
            let Some(selector) = element.get("selector").and_then(Value::as_str) else {
                continue;
            };
            selectors.insert(reference.to_string(), selector.to_string());
            visible.push(json!({
                "ref": reference,
                "role": element.get("role").cloned().unwrap_or(Value::Null),
                "name": element.get("name").cloned().unwrap_or(Value::Null),
            }));
        }
    }
    tools.replace_browser_refs(selectors);
    Ok(json!({
        "url": snapshot.get("url").cloned().unwrap_or(Value::Null),
        "title": snapshot.get("title").cloned().unwrap_or(Value::Null),
        "text": snapshot.get("text").cloned().unwrap_or(Value::Null),
        "elements": visible,
        "refsNote": "refs are valid until the next read_browser",
    }))
}

/// A selector from the snapshot, or the error that says why there is none.
fn selector(tools: &crate::tools::ToolsState, reference: &str) -> Result<String, AppError> {
    tools.browser_selector(reference).ok_or_else(|| {
        AppError::Tools(format!(
            "no element {reference} in the last read_browser snapshot; read the page again"
        ))
    })
}

pub async fn click(
    app: &AppHandle,
    tools: &crate::tools::ToolsState,
    reference: &str,
) -> Result<Value, AppError> {
    let selector = serde_json::to_string(&selector(tools, reference)?)
        .map_err(|err| AppError::Tools(err.to_string()))?;
    let script = [
        "(() => { const el = document.querySelector(",
        &selector,
        "); if (!el) { return { ok: false }; } el.scrollIntoView({ block: 'center' }); el.click(); return { ok: true }; })()",
    ]
    .concat();
    let outcome = evaluate(app, script).await?;
    if outcome.get("ok").and_then(Value::as_bool) != Some(true) {
        return Err(AppError::Tools(format!(
            "the element behind {reference} is gone from the page; read it again"
        )));
    }
    Ok(json!({ "clicked": reference }))
}

pub async fn type_text(
    app: &AppHandle,
    tools: &crate::tools::ToolsState,
    reference: Option<&str>,
    text: &str,
) -> Result<Value, AppError> {
    let length = text.chars().count();
    if length > 20_000 {
        return Err(AppError::Tools(
            "the text is longer than 20000 characters".into(),
        ));
    }
    let text = serde_json::to_string(text).map_err(|err| AppError::Tools(err.to_string()))?;
    let target = match reference {
        Some(reference) => format!(
            "document.querySelector({})",
            serde_json::to_string(&selector(tools, reference)?).unwrap_or_else(|_| "null".into())
        ),
        None => "document.activeElement".to_string(),
    };
    let script = [
        "(() => { const el = ",
        &target,
        "; if (!el) { return { ok: false }; } el.focus(); const proto = Object.getPrototypeOf(el); const setter = Object.getOwnPropertyDescriptor(proto, 'value'); if (setter && setter.set) { setter.set.call(el, ",
        &text,
        "); } else { el.value = ",
        &text,
        "; } el.dispatchEvent(new Event('input', { bubbles: true })); el.dispatchEvent(new Event('change', { bubbles: true })); return { ok: true }; })()",
    ]
    .concat();
    let outcome = evaluate(app, script).await?;
    if outcome.get("ok").and_then(Value::as_bool) != Some(true) {
        return Err(AppError::Tools(
            "there is no field to type into; read the page and pass a ref".into(),
        ));
    }
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

    /// The snapshot must stay a single expression, or an evaluated script that
    /// returns nothing produces "null" and a tool that looks broken.
    #[test]
    fn the_snapshot_is_one_expression_with_one_placeholder() {
        assert!(SNAPSHOT.trim_start().starts_with("(() =>"));
        assert!(SNAPSHOT.trim_end().ends_with(")()"));
        assert_eq!(SNAPSHOT.matches("__LIMIT__").count(), 1);
        assert!(!SNAPSHOT.contains("__LIMIT__\""));
    }

    #[test]
    fn selectors_are_quoted_as_json_before_they_reach_the_page() {
        let selector = "a:nth-of-type(1) > b:nth-of-type(2)";
        let quoted = serde_json::to_string(selector).unwrap();
        assert_eq!(quoted, "\"a:nth-of-type(1) > b:nth-of-type(2)\"");
    }
}
