use serde::{Deserialize, Serialize};

/// Everything a turn can tell the UI, as a single tagged enum.
///
/// Events carry no `turnId`: each turn owns its own `Channel`, so the frontend
/// already knows which turn an event belongs to.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum AgentEvent {
    Timing {
        phase: String,
        ms: u64,
    },
    ApprovalResolved {
        interrupt_id: String,
    },
    TurnStarted,
    Session {
        id: String,
    },
    Usage {
        input: u64,
        output: u64,
    },
    FinalAnswerDelta {
        text: String,
    },
    MessageDelta {
        text: String,
    },
    MessagePreview {
        text: String,
    },
    MessageFinal {
        text: String,
        author: Option<String>,
        replace_preview: bool,
        plain: bool,
    },
    TextDiscard,
    ToolCall {
        call_id: String,
        name: String,
        input: String,
    },
    ToolResult {
        call_id: String,
        ok: bool,
        output: String,
    },
    /// Reserved for backends that rewrite files; the local agent never emits it.
    FileEdit {
        path: String,
        diff: String,
    },
    /// A Nova subagent dispatch (Ralph, Nora, Serena, `task` runs, Code Mode
    /// workers). The pipe upserts rows by `task_id`; a later event with the
    /// same id updates the row instead of appending a new one.
    SubagentTask {
        task_id: String,
        status: String,
        label: Option<String>,
        subagent_type: Option<String>,
        duration_ms: Option<u64>,
    },
    /// A live progress line for a running `SubagentTask` row.
    SubagentPreview {
        task_id: String,
        text: String,
    },
    /// The backend needs a decision before it can continue. Answer with
    /// `respond_approval`; the turn stays open until it is answered.
    ApprovalRequired {
        interrupt_id: String,
        kind: String,
        payload: serde_json::Value,
    },
    /// Transient status line (boot progress, recoverable protocol errors). Never
    /// part of the transcript proper.
    Notice {
        level: String,
        message: String,
    },
    ProcessOutput {
        stream: OutputStream,
        chunk: String,
    },
    TurnError {
        message: String,
        kind: String,
    },
    TurnFinished {
        reason: FinishReason,
    },
    /// The harness asked Nova to start another task, through the documented
    /// request file. The UI decides whether that is allowed.
    WakeupRequested {
        prompt: String,
        agent: Option<String>,
        after_seconds: Option<u64>,
    },
    /// The harness asked Nova to put a message in another task.
    ThreadMessage {
        thread: String,
        text: String,
    },
}

#[derive(Debug, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum InterruptReply {
    Tool {
        approve: bool,
    },
    Plan {
        approve: bool,
        feedback: Option<String>,
    },
    Question {
        answer: String,
        selected_index: Option<usize>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum OutputStream {
    Stdout,
    Stderr,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum FinishReason {
    Completed,
    Cancelled,
    Failed,
}

/// Test-only sink that records what a backend pushed down the channel, so tests
/// assert on the same event stream the UI would receive.
#[cfg(test)]
pub mod testutil {
    use std::sync::{Arc, Mutex};

    use tauri::ipc::{Channel, InvokeResponseBody};

    use super::AgentEvent;

    pub fn collect_events() -> (Channel<AgentEvent>, Arc<Mutex<Vec<AgentEvent>>>) {
        let seen = Arc::new(Mutex::new(Vec::new()));
        let sink = Arc::clone(&seen);

        // `Channel::new` hands the callback the already-encoded body, so decoding
        // it back means these tests also prove the wire format the UI parses.
        let channel = Channel::new(move |body: InvokeResponseBody| {
            let text = match body {
                InvokeResponseBody::Json(text) => text,
                InvokeResponseBody::Raw(bytes) => String::from_utf8_lossy(&bytes).into_owned(),
            };
            let event: AgentEvent =
                serde_json::from_str(&text).expect("channel body is a serialized AgentEvent");
            sink.lock().expect("event sink mutex poisoned").push(event);
            Ok(())
        });

        (channel, seen)
    }
}
