//! 子 agent 控制工具的参数白名单解析与校验。
//!
//! 执行层不信任模型遵守 JSON Schema：未知字段、空文本、空 `items`、`task/items` 或
//! `message/items` 同传、远程图片 URL、非图片 `data:` URL 以及超过 1 MiB 的参数体
//! 一律在解析阶段拒绝，绝不进入事件循环或写库。

use serde_json::Value;

use super::{
    AgentRole, DEFAULT_WAIT_TIMEOUT_MS, MAX_INPUT_ITEMS, MAX_TASK_CHARS, MAX_TITLE_CHARS,
    MAX_WAIT_TARGETS, MAX_WAIT_TIMEOUT_MS, SubagentInputItem, SubagentOp, WaitMode,
    joined_input_text,
};
use crate::chat::tools::{ToolErrorCode, ToolFailure};

/// 七个子 agent 控制工具之一；非控制工具返回 None，参数非法返回 Err。
pub fn parse_control(name: &str, arguments_json: &str) -> Option<Result<SubagentOp, ToolFailure>> {
    match name {
        "spawn_agent" => Some(checked_size(arguments_json).and_then(parse_spawn)),
        "send_input" => Some(checked_size(arguments_json).and_then(parse_send_input)),
        "resume_agent" => Some(checked_size(arguments_json).and_then(parse_resume)),
        "wait_agent" => Some(checked_size(arguments_json).and_then(parse_wait)),
        "close_agent" => Some(checked_size(arguments_json).and_then(parse_close)),
        "list_agents" => Some(checked_size(arguments_json).and_then(parse_list)),
        "cancel_agent_input" => Some(checked_size(arguments_json).and_then(parse_cancel_input)),
        _ => None,
    }
}

/// 检查整条工具参数 JSON 不超过全局 1 MiB 上限。
fn checked_size(arguments_json: &str) -> Result<&str, ToolFailure> {
    if arguments_json.len() > crate::chat::events::MAX_TOOL_ARGUMENTS_BYTES {
        return Err(ToolFailure::new(
            ToolErrorCode::TooLarge,
            format!(
                "arguments exceed the {} byte limit",
                crate::chat::events::MAX_TOOL_ARGUMENTS_BYTES
            ),
        ));
    }
    Ok(arguments_json)
}

/// `spawn_agent` 的参数解析：`role, title` +（`task` 或 `items` 互斥）。
fn parse_spawn(arguments_json: &str) -> Result<SubagentOp, ToolFailure> {
    let object = object_of(arguments_json)?;
    reject_unknown(&object, &["role", "title", "task", "items"])?;
    let role_text = object
        .get("role")
        .and_then(Value::as_str)
        .ok_or_else(|| ToolFailure::invalid("`role` must be \"explorer\" or \"executor\""))?;
    let role = AgentRole::parse(role_text)
        .ok_or_else(|| ToolFailure::invalid("`role` must be \"explorer\" or \"executor\""))?;
    // 旧会话的 spawn 参数没有 title：执行层容忍缺失（历史卡片照常显示 task），
    // 新调用传入 title 时必须为非空字符串且不超长。
    let title = match object.get("title") {
        Some(value) => {
            let raw = value
                .as_str()
                .ok_or_else(|| ToolFailure::invalid("`title` must be a string"))?;
            let line = raw.lines().next().unwrap_or_default().trim().to_string();
            if line.is_empty() {
                return Err(ToolFailure::invalid("`title` must not be empty"));
            }
            if line.chars().count() > MAX_TITLE_CHARS {
                return Err(ToolFailure::new(
                    ToolErrorCode::TooLarge,
                    format!("`title` must be at most {MAX_TITLE_CHARS} characters"),
                ));
            }
            line
        }
        None => String::new(),
    };
    let has_task = object.contains_key("task");
    let has_items = object.contains_key("items");
    if has_task && has_items {
        return Err(ToolFailure::invalid(
            "cannot specify both `task` and `items`",
        ));
    }
    if !has_task && !has_items {
        return Err(ToolFailure::invalid(
            "must provide either `task` or `items`",
        ));
    }
    let (task, items) = if let Some(task_val) = object.get("task") {
        let task = parse_text_field(task_val, "task")?;
        let items = vec![SubagentInputItem::Text { text: task.clone() }];
        (task, items)
    } else {
        let items_val = object
            .get("items")
            .ok_or_else(|| ToolFailure::invalid("`items` is required"))?;
        let items = parse_input_items(items_val)?;
        let task = joined_input_text(&items);
        (task, items)
    };
    Ok(SubagentOp::Spawn {
        role,
        title,
        task,
        items,
    })
}

/// `send_input` 的参数解析：`target` +（`message` 或 `items` 互斥）+ 可选 `interrupt`。
fn parse_send_input(arguments_json: &str) -> Result<SubagentOp, ToolFailure> {
    let object = object_of(arguments_json)?;
    reject_unknown(&object, &["target", "message", "items", "interrupt"])?;
    let target = parse_id_field(object.get("target"), "target")?;
    let interrupt = match object.get("interrupt") {
        None | Some(Value::Null) => false,
        Some(value) => value
            .as_bool()
            .ok_or_else(|| ToolFailure::invalid("`interrupt` must be a boolean"))?,
    };
    let has_message = object.contains_key("message");
    let has_items = object.contains_key("items");
    if has_message && has_items {
        return Err(ToolFailure::invalid(
            "cannot specify both `message` and `items`",
        ));
    }
    if !has_message && !has_items {
        return Err(ToolFailure::invalid(
            "must provide either `message` or `items`",
        ));
    }
    let items = if let Some(msg_val) = object.get("message") {
        let text = parse_text_field(msg_val, "message")?;
        vec![SubagentInputItem::Text { text }]
    } else {
        let items_val = object
            .get("items")
            .ok_or_else(|| ToolFailure::invalid("`items` is required"))?;
        parse_input_items(items_val)?
    };
    Ok(SubagentOp::SendInput {
        target,
        items,
        interrupt,
    })
}

/// `resume_agent` 的参数解析。
fn parse_resume(arguments_json: &str) -> Result<SubagentOp, ToolFailure> {
    let object = object_of(arguments_json)?;
    reject_unknown(&object, &["id"])?;
    let id = parse_id_field(object.get("id"), "id")?;
    Ok(SubagentOp::Resume { id })
}

/// `list_agents` 的参数解析：严格空对象。
fn parse_list(arguments_json: &str) -> Result<SubagentOp, ToolFailure> {
    let object = object_of(arguments_json)?;
    reject_unknown(&object, &[])?;
    Ok(SubagentOp::List)
}

/// `cancel_agent_input` 的参数解析。
fn parse_cancel_input(arguments_json: &str) -> Result<SubagentOp, ToolFailure> {
    let object = object_of(arguments_json)?;
    reject_unknown(&object, &["submission_id"])?;
    let submission_id = parse_id_field(object.get("submission_id"), "submission_id")?;
    Ok(SubagentOp::CancelInput { submission_id })
}

/// `wait_agent` 的参数解析。
fn parse_wait(arguments_json: &str) -> Result<SubagentOp, ToolFailure> {
    let object = object_of(arguments_json)?;
    reject_unknown(&object, &["targets", "wait_for", "timeout_ms"])?;
    let mut parsed = Vec::new();
    if let Some(targets) = object.get("targets").filter(|value| !value.is_null()) {
        let targets = targets
            .as_array()
            .ok_or_else(|| ToolFailure::invalid("`targets` must be an array of agent ids"))?;
        if targets.len() > MAX_WAIT_TARGETS {
            return Err(ToolFailure::invalid(format!(
                "`targets` must contain at most {MAX_WAIT_TARGETS} agent ids"
            )));
        }
        for target in targets {
            let target = target
                .as_str()
                .ok_or_else(|| ToolFailure::invalid("`targets` must contain only strings"))?;
            if target.trim().is_empty() {
                return Err(ToolFailure::invalid("`targets` must not contain empty ids"));
            }
            if !parsed.iter().any(|item: &String| item == target) {
                parsed.push(target.to_string());
            }
        }
    }
    let mode = match object.get("wait_for") {
        None | Some(Value::Null) => WaitMode::All,
        Some(value) => {
            let text = value
                .as_str()
                .ok_or_else(|| ToolFailure::invalid("`wait_for` must be \"all\" or \"any\""))?;
            match text {
                "all" => WaitMode::All,
                "any" => WaitMode::Any,
                _ => {
                    return Err(ToolFailure::invalid(
                        "`wait_for` must be \"all\" or \"any\"",
                    ));
                }
            }
        }
    };
    let timeout_ms = match object.get("timeout_ms") {
        None | Some(Value::Null) => DEFAULT_WAIT_TIMEOUT_MS,
        Some(value) => {
            let raw = value
                .as_i64()
                .ok_or_else(|| ToolFailure::invalid("`timeout_ms` must be an integer"))?;
            raw.clamp(1, MAX_WAIT_TIMEOUT_MS as i64) as u64
        }
    };
    Ok(SubagentOp::Wait {
        targets: parsed,
        mode,
        timeout_ms: timeout_ms.clamp(1, MAX_WAIT_TIMEOUT_MS),
    })
}

/// `close_agent` 的参数解析。
fn parse_close(arguments_json: &str) -> Result<SubagentOp, ToolFailure> {
    let object = object_of(arguments_json)?;
    reject_unknown(&object, &["target"])?;
    let target = parse_id_field(object.get("target"), "target")?;
    Ok(SubagentOp::Close { target })
}

/// 解析非空文本字段（`task` / `message`）。
fn parse_text_field(value: &Value, field: &str) -> Result<String, ToolFailure> {
    let text = value
        .as_str()
        .ok_or_else(|| ToolFailure::invalid(format!("`{field}` must be a string")))?
        .to_string();
    if text.trim().is_empty() {
        return Err(ToolFailure::invalid(format!("`{field}` must not be empty")));
    }
    if text.chars().count() > MAX_TASK_CHARS {
        return Err(ToolFailure::new(
            ToolErrorCode::TooLarge,
            format!("`{field}` must be at most {MAX_TASK_CHARS} characters"),
        ));
    }
    Ok(text)
}

/// 解析非空标识符字段（`target` / `id` / `submission_id`）。
fn parse_id_field(value: Option<&Value>, field: &str) -> Result<String, ToolFailure> {
    let text = value
        .and_then(Value::as_str)
        .ok_or_else(|| ToolFailure::invalid(format!("`{field}` must be a string")))?
        .trim()
        .to_string();
    if text.is_empty() {
        return Err(ToolFailure::invalid(format!("`{field}` must not be empty")));
    }
    if text.chars().count() > 128 {
        return Err(ToolFailure::new(
            ToolErrorCode::TooLarge,
            format!("`{field}` must be at most 128 characters"),
        ));
    }
    Ok(text)
}

/// 解析有序结构化 `items` 数组（仅支持 `text` / `local_image` / `image`）。
fn parse_input_items(value: &Value) -> Result<Vec<SubagentInputItem>, ToolFailure> {
    let array = value
        .as_array()
        .ok_or_else(|| ToolFailure::invalid("`items` must be a non-empty array"))?;
    if array.is_empty() {
        return Err(ToolFailure::invalid("`items` must not be empty"));
    }
    if array.len() > MAX_INPUT_ITEMS {
        return Err(ToolFailure::new(
            ToolErrorCode::TooLarge,
            format!("`items` must contain at most {MAX_INPUT_ITEMS} entries"),
        ));
    }
    let mut out = Vec::with_capacity(array.len());
    let mut total_text_chars = 0usize;
    let mut image_count = 0usize;
    for raw in array {
        let obj = raw
            .as_object()
            .ok_or_else(|| ToolFailure::invalid("each item in `items` must be a JSON object"))?;
        let kind = obj.get("type").and_then(Value::as_str).ok_or_else(|| {
            ToolFailure::invalid("each item in `items` must have a string `type`")
        })?;
        match kind {
            "text" => {
                reject_unknown(obj, &["type", "text"])?;
                let text = obj
                    .get("text")
                    .and_then(Value::as_str)
                    .ok_or_else(|| ToolFailure::invalid("`text` item requires a string `text`"))?
                    .to_string();
                if text.trim().is_empty() {
                    return Err(ToolFailure::invalid("`text` item must not be empty"));
                }
                total_text_chars = total_text_chars.saturating_add(text.chars().count());
                if total_text_chars > MAX_TASK_CHARS {
                    return Err(ToolFailure::new(
                        ToolErrorCode::TooLarge,
                        format!(
                            "total text in `items` must be at most {MAX_TASK_CHARS} characters"
                        ),
                    ));
                }
                out.push(SubagentInputItem::Text { text });
            }
            "local_image" => {
                reject_unknown(obj, &["type", "path"])?;
                let path = obj
                    .get("path")
                    .and_then(Value::as_str)
                    .ok_or_else(|| {
                        ToolFailure::invalid("`local_image` item requires a string `path`")
                    })?
                    .trim()
                    .to_string();
                if path.is_empty() {
                    return Err(ToolFailure::invalid("`local_image.path` must not be empty"));
                }
                let lower = path.to_ascii_lowercase();
                if lower.starts_with("http://") || lower.starts_with("https://") {
                    return Err(ToolFailure::invalid(
                        "remote image URLs are not supported; pass a local file path",
                    ));
                }
                if lower.starts_with("data:") {
                    return Err(ToolFailure::invalid(
                        "`local_image.path` must be a file path, not a data URL",
                    ));
                }
                image_count = image_count.saturating_add(1);
                if image_count > crate::platform::MAX_ATTACHED_IMAGES {
                    return Err(ToolFailure::new(
                        ToolErrorCode::TooLarge,
                        format!(
                            "at most {} images may be attached per input",
                            crate::platform::MAX_ATTACHED_IMAGES
                        ),
                    ));
                }
                out.push(SubagentInputItem::LocalImage { path });
            }
            "image" => {
                reject_unknown(obj, &["type", "image_url"])?;
                let image_url = obj
                    .get("image_url")
                    .and_then(Value::as_str)
                    .ok_or_else(|| {
                        ToolFailure::invalid("`image` item requires a string `image_url`")
                    })?
                    .trim()
                    .to_string();
                if image_url.is_empty() {
                    return Err(ToolFailure::invalid("`image_url` must not be empty"));
                }
                let lower = image_url.to_ascii_lowercase();
                if lower.starts_with("http://") || lower.starts_with("https://") {
                    return Err(ToolFailure::invalid(
                        "remote image URLs are not supported; use `local_image` or a `data:image/...;base64,...` URL",
                    ));
                }
                if !is_valid_image_data_url_header(&lower) {
                    return Err(ToolFailure::invalid(
                        "`image_url` must be a valid `data:image/<format>;base64,...` URL",
                    ));
                }
                image_count = image_count.saturating_add(1);
                if image_count > crate::platform::MAX_ATTACHED_IMAGES {
                    return Err(ToolFailure::new(
                        ToolErrorCode::TooLarge,
                        format!(
                            "at most {} images may be attached per input",
                            crate::platform::MAX_ATTACHED_IMAGES
                        ),
                    ));
                }
                out.push(SubagentInputItem::Image { image_url });
            }
            other => {
                let shown: String = other.chars().take(48).collect();
                return Err(ToolFailure::invalid(format!(
                    "unsupported item type: `{shown}`; supported types are `text`, `local_image`, `image`"
                )));
            }
        }
    }
    Ok(out)
}

/// 校验 `data:` URL 头部声明为 `data:image/<subtype>;base64,<payload>`。
fn is_valid_image_data_url_header(lower_url: &str) -> bool {
    let Some(rest) = lower_url.strip_prefix("data:") else {
        return false;
    };
    let Some((header, payload)) = rest.split_once(',') else {
        return false;
    };
    if payload.trim().is_empty() {
        return false;
    }
    let Some(media_type) = header.trim().strip_suffix(";base64") else {
        return false;
    };
    let media_type = media_type.split(';').next().unwrap_or_default().trim();
    media_type.starts_with("image/") && media_type.len() > "image/".len()
}

/// 参数体必须是 JSON 对象。
fn object_of(arguments_json: &str) -> Result<serde_json::Map<String, Value>, ToolFailure> {
    serde_json::from_str::<Value>(arguments_json)
        .ok()
        .and_then(|value| value.as_object().cloned())
        .ok_or_else(|| ToolFailure::invalid("arguments must be a JSON object"))
}

/// 拒绝白名单外的字段（只报字段名，不回显参数全文）。
fn reject_unknown(
    object: &serde_json::Map<String, Value>,
    allowed: &[&str],
) -> Result<(), ToolFailure> {
    for key in object.keys() {
        if !allowed.contains(&key.as_str()) {
            return Err(ToolFailure::invalid(format!("unknown field: `{key}`")));
        }
    }
    Ok(())
}
