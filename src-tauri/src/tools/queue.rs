//! The message queue, and steering.
//!
//! Queued messages start a new turn. Steering goes through the active backend.

use serde_json::{json, Value};

use crate::error::AppError;
use crate::tools::ToolsState;

/// Long enough for a paragraph of instruction, short enough that the queue file
/// cannot become a transcript of its own.
const MAX_QUEUED_BYTES: usize = 32 * 1024;

pub fn push(tools: &ToolsState, chat_id: &str, text: &str) -> Result<Value, AppError> {
    if chat_id.trim().is_empty() || chat_id.len() > 128 {
        return Err(AppError::Tools("a task id is required".into()));
    }
    let text = text.trim();
    if text.is_empty() || text.len() > MAX_QUEUED_BYTES {
        return Err(AppError::Tools(format!(
            "a message of 1 to {MAX_QUEUED_BYTES} bytes is required"
        )));
    }
    let queued = tools.queue_push(chat_id, text);
    Ok(json!({ "queued": queued, "pending": tools.queue_len(chat_id) }))
}

pub fn list(tools: &ToolsState, chat_id: &str) -> Value {
    json!({ "taskId": chat_id, "queue": tools.queue_list(chat_id) })
}

pub fn remove(tools: &ToolsState, id: &str) -> Value {
    json!({ "removed": tools.queue_remove(id) })
}

pub async fn steer(state: &crate::AppState, chat_id: &str, text: &str) -> Result<Value, AppError> {
    if text.trim().is_empty() || text.len() > MAX_QUEUED_BYTES || text.contains('\0') {
        return Err(AppError::Tools(format!(
            "enter steering text of 1 to {MAX_QUEUED_BYTES} UTF-8 bytes"
        )));
    }
    let Some((turn_id, harness)) = state.live_turn(chat_id) else {
        return Err(AppError::Tools(format!(
            "task {chat_id} is not running, so there is nothing to steer; queue the message with queue_message"
        )));
    };
    state.agents.get(&harness)?.steer(&turn_id, text).await?;
    Ok(json!({ "accepted": true, "turnId": turn_id, "harness": harness }))
}

pub fn interrupt(state: &crate::AppState, chat_id: &str) -> Result<Value, AppError> {
    let Some((turn_id, harness)) = state.live_turn(chat_id) else {
        return Err(AppError::Tools(format!(
            "task {chat_id} is not running, so there is nothing to interrupt"
        )));
    };
    if !state.cancel_turn(&turn_id) {
        return Err(AppError::Tools(format!(
            "task {chat_id} has already finished"
        )));
    }
    Ok(json!({ "interrupted": turn_id, "harness": harness }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn queued_messages_are_listed_in_order_and_removable() {
        let tools = ToolsState::default();
        push(&tools, "task-1", "first").unwrap();
        push(&tools, "task-1", "second").unwrap();
        let listed = list(&tools, "task-1");
        let queue = listed["queue"].as_array().unwrap();
        assert_eq!(queue.len(), 2);
        assert_eq!(queue[0]["text"], "first");

        let id = queue[0]["id"].as_str().unwrap().to_string();
        assert_eq!(remove(&tools, &id)["removed"], true);
        assert_eq!(list(&tools, "task-1")["queue"].as_array().unwrap().len(), 1);
    }

    #[test]
    fn an_empty_or_oversized_message_is_refused() {
        let tools = ToolsState::default();
        assert!(push(&tools, "task-1", "   ").is_err());
        assert!(push(&tools, "task-1", &"x".repeat(MAX_QUEUED_BYTES + 1)).is_err());
        assert!(push(&tools, "", "hello").is_err());
    }

    #[test]
    fn draining_takes_the_oldest_and_leaves_the_rest() {
        let tools = ToolsState::default();
        push(&tools, "task-1", "first").unwrap();
        push(&tools, "task-1", "second").unwrap();
        let drained = tools.queue_pop("task-1").unwrap();
        assert_eq!(drained.text, "first");
        assert_eq!(tools.queue_len("task-1"), 1);
        assert!(tools.queue_pop("task-2").is_none());
        tools.queue_restore("task-1", drained.clone());
        assert_eq!(tools.queue_pop("task-1").unwrap().id, drained.id);
        assert_eq!(tools.queue_pop("task-1").unwrap().text, "second");
    }

    #[tokio::test]
    async fn steering_an_idle_task_suggests_queuing() {
        let error = steer(&crate::AppState::default(), "task-1", "hello")
            .await
            .unwrap_err();
        assert!(error.to_string().contains("queue_message"));
    }
}
