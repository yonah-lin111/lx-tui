//! 八个 subagent 控制工具的参数 schema（`spawn_agent` / `send_input` / `interrupt_agent` /
//! `resume_agent` / `wait_agent` / `close_agent` / `list_agents` / `cancel_agent_input`）。
//!
//! 描述与 schema 的稳定值全部来自 [`crate::chat::agent`]：参数校验与给模型看的
//! 文档共用一份常量，不在两处各写一遍。

use serde_json::{Value, json};

use super::{ToolDecl, ToolSpec};
use crate::chat::agent::{
    DEFAULT_WAIT_TIMEOUT_MS, MAX_FORK_TURNS, MAX_INPUT_ITEMS, MAX_PATH_SEGMENT_LEN,
    MAX_ROLE_NAME_LEN, MAX_TASK_CHARS, MAX_TITLE_CHARS, MAX_WAIT_TARGETS, MAX_WAIT_TIMEOUT_MS,
};
use crate::config::subagents::{MIN_SUBAGENT_MAX_TOKENS, SubagentsSection};

/// `spawn_agent` 规格。
pub(crate) const fn spawn_agent_spec() -> ToolSpec {
    ToolSpec::new(
        "spawn_agent",
        "Delegate one well-defined task to a subagent. By default (`fork_context=false`) it starts \
         with a clean context; set `fork_context=true` to clone all completed turns from the \
         parent conversation before appending this task, or pass `fork_turns` (1-64) to clone \
         only the most recent completed turns. Built-in roles are `explorer` (read-only \
         `read`/`search`/`web_search`) and `executor` (file and process tools, no `ask`, no plan \
         tool), plus any custom roles configured in settings. Optionally pass `model` \
         ('provider/model' or 'model'), `reasoning_effort`, and `max_tokens` to override the role \
         or parent model and cap the subagent's token budget. Always give it a short \
         human-readable `title` plus either `task` (plain text) or `items` (ordered text and \
         image items, mutually exclusive with `task`). Use `task_name` to control the stable \
         path segment when the subagent may itself delegate. Returns `agent_id`, `nickname`, and \
         `agent_path` immediately; the subagent runs in the background.",
        spawn_parameters,
    )
}

/// `send_input` 规格。
pub(crate) const fn send_input_spec() -> ToolSpec {
    ToolSpec::new(
        "send_input",
        "Send a follow-up message or ordered input items to an active or resumed subagent. \
         Provide `target` (`agent_id`, `nickname`, or `agent_path` in this conversation) and \
         either `message` or `items` (mutually exclusive). When the subagent is running and \
         `interrupt` is false (default), the input is durably queued and runs after the current \
         turn completes. When `interrupt` is true, the current turn is cancelled, its partial \
         output is preserved, and the new input runs first ahead of older queued items. Set \
         `trigger_turn=false` to queue input without waking an idle subagent (not allowed with \
         `interrupt=true`). A subagent that has already finished, failed, or been closed must be \
         resumed with `resume_agent` before calling `send_input`.",
        send_input_parameters,
    )
}

/// `interrupt_agent` 规格。
pub(crate) const fn interrupt_agent_spec() -> ToolSpec {
    ToolSpec::new(
        "interrupt_agent",
        "Interrupt a running subagent's current turn by `target` (`agent_id`, `nickname`, or \
         `agent_path`), \
         preserving its partial output while keeping the subagent in active `idle` state ready \
         for follow-up `send_input`. Any remaining queued inputs are paused until you send new \
         input or call `resume_agent`. Calling `interrupt_agent` on an already `idle` subagent \
         is idempotent.",
        interrupt_parameters,
    )
}

/// `resume_agent` 规格。
pub(crate) const fn resume_agent_spec() -> ToolSpec {
    ToolSpec::new(
        "resume_agent",
        "Resume a completed, failed, cancelled, or restarted subagent by `id` (`agent_id`, \
         `nickname`, or `agent_path`) so it can continue working across turns. Restores its history, compact \
         checkpoints, and role within this conversation. If the agent has queued inputs, the \
         next queued input starts immediately; otherwise the agent enters `idle` state ready \
         for `send_input`. Calling `resume_agent` on an already active agent is idempotent.",
        resume_parameters,
    )
}

/// `wait_agent` 规格。
pub(crate) const fn wait_agent_spec() -> ToolSpec {
    ToolSpec::new(
        "wait_agent",
        "Wait for subagents. Omit `targets` or pass an empty array to wait for every \
         subagent in this conversation; pass `agent_id`, `nickname`, or `agent_path` values for \
         one or more specific agents. `wait_for=all` (default) keeps waiting until all targets finish; the \
         timeout is only an internal recheck interval, so the call does not end the turn early. \
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
        "Stop a running or idle subagent by `target` (`agent_id`, `nickname`, or `agent_path`), \
         including all of its surviving descendants. It produces no report for an interrupted \
         turn; use this to stop work that is no longer needed. The agent's session, history, and \
         queued inputs remain stored and can be resumed later with `resume_agent`.",
        close_parameters,
    )
}

/// `list_agents` 规格。
pub(crate) const fn list_agents_spec() -> ToolSpec {
    ToolSpec::new(
        "list_agents",
        "List subagents belonging to the current conversation with their `agent_id`, \
         `nickname`, `agent_path`, `role`, `title`, `status`, and `queued_count`. Pass \
         `path_prefix` (absolute like `/root/turing` or relative like `turing`) to restrict the \
         result to one subtree. Use this to inspect active or resumable agents before calling \
         `resume_agent`, `send_input`, `interrupt_agent`, or `wait_agent`.",
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

/// 生成 `spawn_agent` 的角色指导说明段落（英文，对齐 Codex `spawn_tool_spec`）。
pub fn build_spawn_role_guidance(subagents: &SubagentsSection) -> String {
    let roles = subagents.all_roles();
    let mut out = format!(
        "\n\nConcurrency limit: at most {} subagents may run at once across the whole session \
         tree.\nMaximum delegation depth: {} (1 = only the main agent delegates; 2 = subagents \
         may delegate once).\nAvailable roles:\n",
        subagents.max_concurrent, subagents.max_depth
    );
    for role in roles {
        let model_note = match &role.default_model {
            Some(sel) => match &sel.variant {
                Some(v) => format!(
                    " (base: {}, default model: {}/{} ({v}))",
                    role.base_role.as_str(),
                    sel.provider,
                    sel.model
                ),
                None => format!(
                    " (base: {}, default model: {}/{})",
                    role.base_role.as_str(),
                    sel.provider,
                    sel.model
                ),
            },
            None => format!(
                " (base: {}, inherits parent model)",
                role.base_role.as_str()
            ),
        };
        out.push_str(&format!(
            "- `{}`{}: {}\n",
            role.name, model_note, role.description
        ));
    }
    out.trim_end().to_string()
}

/// 将生效的 `SubagentsSection` 注入 `spawn_agent` 工具声明（描述 + 可选角色枚举）。
pub fn enrich_spawn_tool_decl(decl: &mut ToolDecl, subagents: &SubagentsSection) {
    if decl.name != "spawn_agent" {
        return;
    }
    decl.description
        .push_str(&build_spawn_role_guidance(subagents));
    let role_names: Vec<Value> = subagents
        .all_roles()
        .into_iter()
        .map(|r| Value::String(r.name))
        .collect();
    if let Some(role_prop) = decl
        .parameters
        .get_mut("properties")
        .and_then(|p| p.get_mut("role"))
        .and_then(Value::as_object_mut)
    {
        role_prop.insert("enum".to_string(), Value::Array(role_names));
    }
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
                "minLength": 1,
                "maxLength": MAX_ROLE_NAME_LEN,
                "description": "Subagent role identifier (`explorer`, `executor`, or a configured custom role name).",
            },
            "title": {
                "type": "string",
                "minLength": 1,
                "maxLength": MAX_TITLE_CHARS,
                "description": "Short human-readable title for cards and the Agents list; not the task prompt.",
            },
            "task_name": {
                "type": "string",
                "minLength": 1,
                "maxLength": MAX_PATH_SEGMENT_LEN,
                "pattern": "^[a-z0-9][a-z0-9_]{0,31}$",
                "description": "Optional stable path segment for this subagent (`[a-z0-9_]{1,32}`, must not be `root`). Omit to derive it from the nickname.",
            },
            "task": {
                "type": "string",
                "minLength": 1,
                "maxLength": MAX_TASK_CHARS,
                "description": "Goal, boundary (files/modules owned by this agent), and deliverable. Mutually exclusive with `items`.",
            },
            "items": input_items_schema(),
            "fork_context": {
                "type": "boolean",
                "default": false,
                "description": "When true, clones all completed turns from the parent conversation into the subagent before appending this task. Defaults to false (clean context).",
            },
            "fork_turns": {
                "type": "integer",
                "minimum": 0,
                "maximum": MAX_FORK_TURNS,
                "description": "When greater than 0, clones only the most recent N completed turns and implies `fork_context=true`. 0 disables history cloning.",
            },
            "max_tokens": {
                "type": "integer",
                "minimum": MIN_SUBAGENT_MAX_TOKENS,
                "description": format!("Optional token budget for this subagent turn (at least {MIN_SUBAGENT_MAX_TOKENS}). Omit to use the configured default or no limit."),
            },
            "model": {
                "type": "string",
                "minLength": 1,
                "maxLength": 128,
                "description": "Optional model override in 'provider/model' or 'model' format. Omit to use the role's configured default model or inherit your current model.",
            },
            "reasoning_effort": {
                "type": "string",
                "minLength": 1,
                "maxLength": 64,
                "description": "Optional reasoning effort / variant override (for example 'low', 'medium', 'high', 'max'). Omit to use the role's default or inherit.",
            },
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
                "description": "Target `agent_id`, `nickname`, or `agent_path` (absolute like `/root/turing` or relative like `turing`) belonging to this conversation.",
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
                "description": "When true, cancel the currently running turn, save its partial output, and run this input next before older queued items. Cannot be combined with `trigger_turn=false`.",
            },
            "trigger_turn": {
                "type": "boolean",
                "default": true,
                "description": "When true (default), start a new turn immediately if the target is idle. When false, only enqueue the input without waking the subagent.",
            },
        },
    })
}

/// `interrupt_agent` 参数 schema。
fn interrupt_parameters() -> Value {
    json!({
        "type": "object",
        "additionalProperties": false,
        "required": ["target"],
        "properties": {
            "target": {
                "type": "string",
                "minLength": 1,
                "description": "Target subagent `agent_id`, `nickname`, or `agent_path` in this conversation.",
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
                "description": "Agent id (`agent_id`), `nickname`, or `agent_path` to resume in this conversation.",
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
                "description": "Agent ids (`agent_id`), `nickname` values, or `agent_path` values returned by spawn_agent, or omit/empty to wait for every subagent in this conversation.",
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
                "description": "Agent id (`agent_id`), `nickname`, or `agent_path` to stop (its surviving descendants are stopped too).",
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
        "properties": {
            "path_prefix": {
                "type": "string",
                "minLength": 1,
                "description": "Optional `agent_path` prefix (absolute like `/root/turing` or relative like `turing`) restricting the listing to one subtree.",
            },
        },
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
