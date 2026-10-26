//! 三个 subagent 控制工具的参数 schema（`spawn_agent` / `wait_agent` / `close_agent`）。
//!
//! 描述与 schema 的稳定值全部来自 [`crate::chat::agent`]：参数校验与给模型看的
//! 文档共用一份常量，不在两处各写一遍。

use serde_json::{Value, json};

use super::ToolSpec;
use crate::chat::agent::{
    DEFAULT_WAIT_TIMEOUT_MS, MAX_TASK_CHARS, MAX_WAIT_TARGETS, MAX_WAIT_TIMEOUT_MS,
};

/// `spawn_agent` 规格。
pub(crate) const fn spawn_agent_spec() -> ToolSpec {
    ToolSpec::new(
        "spawn_agent",
        "Delegate one well-defined task to a subagent that starts with a clean context. \
         `explorer` (read-only `read`/`search`) investigates a specific question about the \
         codebase; `executor` (file and process tools, no `ask`, no plan tool) makes one \
         bounded change or fix. Write the task as goal + boundary (files/modules owned by \
         this agent) + deliverable. The call returns an `agent_id` immediately; the subagent \
         runs in the background and you can keep working. At most 3 subagents may run at \
         once per conversation.",
        spawn_parameters,
    )
}

/// `wait_agent` 规格。
pub(crate) const fn wait_agent_spec() -> ToolSpec {
    ToolSpec::new(
        "wait_agent",
        "Wait until every listed subagent finishes or the timeout expires, whichever comes \
         first. Returns each agent's status and, the first time it is settled, its final \
         report. A timeout is not an error: `timed_out: true` means some agents are still \
         running and you can wait again later.",
        wait_parameters,
    )
}

/// `close_agent` 规格。
pub(crate) const fn close_agent_spec() -> ToolSpec {
    ToolSpec::new(
        "close_agent",
        "Stop a running subagent. It produces no report; use this to abandon work that is \
         no longer needed. The agent's session and partial trace remain available.",
        close_parameters,
    )
}

/// `spawn_agent` 参数 schema。
fn spawn_parameters() -> Value {
    json!({
        "type": "object",
        "additionalProperties": false,
        "required": ["role", "task"],
        "properties": {
            "role": {
                "type": "string",
                "enum": ["explorer", "executor"],
                "description": "Subagent role: read-only investigation or bounded implementation.",
            },
            "task": {
                "type": "string",
                "minLength": 1,
                "maxLength": MAX_TASK_CHARS,
                "description": "Goal, boundary (files/modules owned by this agent), and deliverable.",
            },
        },
    })
}

/// `wait_agent` 参数 schema。
fn wait_parameters() -> Value {
    json!({
        "type": "object",
        "additionalProperties": false,
        "required": ["targets"],
        "properties": {
            "targets": {
                "type": "array",
                "minItems": 1,
                "maxItems": MAX_WAIT_TARGETS,
                "items": { "type": "string" },
                "description": "Agent ids returned by spawn_agent.",
            },
            "timeout_ms": {
                "type": "integer",
                "minimum": 1,
                "maximum": MAX_WAIT_TIMEOUT_MS,
                "default": DEFAULT_WAIT_TIMEOUT_MS,
                "description": "Maximum time to wait in milliseconds.",
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
