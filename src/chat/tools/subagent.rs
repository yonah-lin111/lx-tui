//! 七个 subagent 控制工具的参数 schema（`spawn_agent` / `send_input` / `resume_agent` /
//! `wait_agent` / `close_agent` / `list_agents` / `cancel_agent_input`）。
//!
//! 描述与 schema 的稳定值全部来自 [`crate::chat::agent`]：参数校验与给模型看的
//! 文档共用一份常量，不在两处各写一遍。

use serde_json::{Value, json};

use super::ToolSpec;
use crate::chat::agent::{
    DEFAULT_WAIT_TIMEOUT_MS, MAX_INPUT_ITEMS, MAX_TASK_CHARS, MAX_TITLE_CHARS, MAX_WAIT_TARGETS,
    MAX_WAIT_TIMEOUT_MS,
};

/// `spawn_agent` 规格。
pub(crate) const fn spawn_agent_spec() -> ToolSpec {
    ToolSpec::new(
        "spawn_agent",
        "Delegate one well-defined task to a subagent that starts with a clean context. \
         `explorer` (read-only `read`/`search`) investigates a specific question about the \
         codebase; `executor` (file and process tools, no `ask`, no plan tool) makes one \
         bounded change or fix. Always give it a short human-readable `title` (for cards \
         and the Agents list) plus either `task` (plain text) or `items` (ordered text and \
         image items, mutually exclusive with `task`): goal + boundary (files/modules owned \
         by this agent) + deliverable. The call returns an `agent_id` immediately; the \
         subagent runs in the background and you can keep working. At most 3 subagents may \
         run at once per conversation.",
        spawn_parameters,
    )
}

/// `send_input` 规格。
pub(crate) const fn send_input_spec() -> ToolSpec {
    ToolSpec::new(
        "send_input",
        "Send a follow-up message or ordered input items to an active or resumed subagent. \
         Provide `target` (the `agent_id`) and either `message` or `items` (mutually exclusive). \
         When the subagent is running and `interrupt` is false (default), the input is durably \
         queued and runs after the current turn completes. When `interrupt` is true, the current \
         turn is cancelled, its partial output is preserved, and the new input runs first ahead \
         of older queued items. A subagent that has already finished, failed, or been closed \
         must be resumed with `resume_agent` before calling `send_input`.",
        send_input_parameters,
    )
}

/// `resume_agent` 规格。
pub(crate) const fn resume_agent_spec() -> ToolSpec {
    ToolSpec::new(
        "resume_agent",
        "Resume a completed, failed, cancelled, or restarted subagent by `id` (`agent_id`) so it \
         can continue working across turns. Restores its history, compact checkpoints, and role \
         within this conversation. If the agent has queued inputs, the next queued input starts \
         immediately; otherwise the agent enters `idle` state ready for `send_input`. Calling \
         `resume_agent` on an already active agent is idempotent.",
        resume_parameters,
    )
}

/// `wait_agent` 规格。
pub(crate) const fn wait_agent_spec() -> ToolSpec {
    ToolSpec::new(
        "wait_agent",
        "Wait for subagents. Omit `targets` or pass an empty array to wait for every \
         subagent in this conversation; pass ids for one or more specific agents. \
         `wait_for=all` (default) keeps waiting until all targets finish; the timeout is \
         only an internal recheck interval, so the call does not end the turn early. \
         `wait_for=any` returns as soon as one target finishes or the timeout expires. \
         Returns each agent's status and, the first time a completed or failed turn is settled, \
         its report. A timeout is not an error.",
        wait_parameters,
    )
}

/// `close_agent` 规格。
pub(crate) const fn close_agent_spec() -> ToolSpec {
    ToolSpec::new(
        "close_agent",
        "Stop a running or idle subagent. It produces no report for an interrupted turn; use \
         this to stop work that is no longer needed. The agent's session, history, and queued \
         inputs remain stored and can be resumed later with `resume_agent`.",
        close_parameters,
    )
}

/// `list_agents` 规格。
pub(crate) const fn list_agents_spec() -> ToolSpec {
    ToolSpec::new(
        "list_agents",
        "List all subagents belonging to the current conversation with their `agent_id`, `role`, \
         `title`, `status`, and `queued_count`. Use this to inspect active or resumable agents \
         before calling `resume_agent`, `send_input`, or `wait_agent`.",
        list_parameters,
    )
}

/// `cancel_agent_input` 规格。
pub(crate) const fn cancel_agent_input_spec() -> ToolSpec {
    ToolSpec::new(
        "cancel_agent_input",
        "Cancel a queued subagent input by `submission_id` before it starts running. Only \
         `queued` (including image-blocked) inputs transition to `cancelled`; repeated calls \
         are idempotent, and inputs that are already `running` or finished return their current \
         status without rolling back.",
        cancel_input_parameters,
    )
}

/// 结构化 `items` 数组 schema（`spawn_agent` 与 `send_input` 共用）。
fn input_items_schema() -> Value {
    json!({
        "type": "array",
        "minItems": 1,
        "maxItems": MAX_INPUT_ITEMS,
        "description": "Ordered input items (text, local_image, or data: image). Mutually exclusive with `task`/`message`.",
        "items": {
            "type": "object",
            "additionalProperties": false,
            "required": ["type"],
            "properties": {
                "type": {
                    "type": "string",
                    "enum": ["text", "local_image", "image"],
                    "description": "Item kind: `text` requires `text`; `local_image` requires `path`; `image` requires a `data:image/...;base64,...` `image_url`.",
                },
                "text": {
                    "type": "string",
                    "minLength": 1,
                    "maxLength": MAX_TASK_CHARS,
                    "description": "Text content when `type` is `text`.",
                },
                "path": {
                    "type": "string",
                    "minLength": 1,
                    "description": "Local image path (relative to the frozen workspace root or absolute) when `type` is `local_image`.",
                },
                "image_url": {
                    "type": "string",
                    "minLength": 1,
                    "description": "Inline `data:image/<format>;base64,...` URL when `type` is `image`. Remote http(s) URLs are rejected.",
                },
            },
        },
    })
}

/// `spawn_agent` 参数 schema。
fn spawn_parameters() -> Value {
    json!({
        "type": "object",
        "additionalProperties": false,
        "required": ["role", "title"],
        "properties": {
            "role": {
                "type": "string",
                "enum": ["explorer", "executor"],
                "description": "Subagent role: read-only investigation or bounded implementation.",
            },
            "title": {
                "type": "string",
                "minLength": 1,
                "maxLength": MAX_TITLE_CHARS,
                "description": "Short human-readable title for cards and the Agents list; not the task prompt.",
            },
            "task": {
                "type": "string",
                "minLength": 1,
                "maxLength": MAX_TASK_CHARS,
                "description": "Goal, boundary (files/modules owned by this agent), and deliverable. Mutually exclusive with `items`.",
            },
            "items": input_items_schema(),
        },
    })
}

/// `send_input` 参数 schema。
fn send_input_parameters() -> Value {
    json!({
        "type": "object",
        "additionalProperties": false,
        "required": ["target"],
        "properties": {
            "target": {
                "type": "string",
                "minLength": 1,
                "description": "Target `agent_id` belonging to this conversation.",
            },
            "message": {
                "type": "string",
                "minLength": 1,
                "maxLength": MAX_TASK_CHARS,
                "description": "Plain-text follow-up message. Mutually exclusive with `items`.",
            },
            "items": input_items_schema(),
            "interrupt": {
                "type": "boolean",
                "default": false,
                "description": "When true, cancel the currently running turn, save its partial output, and run this input next before older queued items.",
            },
        },
    })
}

/// `resume_agent` 参数 schema。
fn resume_parameters() -> Value {
    json!({
        "type": "object",
        "additionalProperties": false,
        "required": ["id"],
        "properties": {
            "id": {
                "type": "string",
                "minLength": 1,
                "description": "Agent id (`agent_id`) to resume in this conversation.",
            },
        },
    })
}

/// `wait_agent` 参数 schema。
fn wait_parameters() -> Value {
    json!({
        "type": "object",
        "additionalProperties": false,
        "required": [],
        "properties": {
            "targets": {
                "type": "array",
                "maxItems": MAX_WAIT_TARGETS,
                "items": { "type": "string" },
                "description": "Agent ids returned by spawn_agent, or omit/empty to wait for every subagent in this conversation.",
            },
            "wait_for": {
                "type": "string",
                "enum": ["all", "any"],
                "default": "all",
                "description": "Wait for all targets to finish, or return as soon as any one finishes.",
            },
            "timeout_ms": {
                "type": "integer",
                "minimum": 1,
                "maximum": MAX_WAIT_TIMEOUT_MS,
                "default": DEFAULT_WAIT_TIMEOUT_MS,
                "description": "Maximum time to wait in milliseconds. Prefer minutes; subagent work often takes several minutes.",
            },
        },
    })
}

/// `close_agent` 参数 schema。
fn close_parameters() -> Value {
    json!({
        "type": "object",
        "additionalProperties": false,
        "required": ["target"],
        "properties": {
            "target": {
                "type": "string",
                "minLength": 1,
                "description": "Agent id to stop.",
            },
        },
    })
}

/// `list_agents` 参数 schema。
fn list_parameters() -> Value {
    json!({
        "type": "object",
        "additionalProperties": false,
        "required": [],
        "properties": {},
    })
}

/// `cancel_agent_input` 参数 schema。
fn cancel_input_parameters() -> Value {
    json!({
        "type": "object",
        "additionalProperties": false,
        "required": ["submission_id"],
        "properties": {
            "submission_id": {
                "type": "string",
                "minLength": 1,
                "description": "Submission id returned by `send_input`.",
            },
        },
    })
}
