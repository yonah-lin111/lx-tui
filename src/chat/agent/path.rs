//! 层级化 Agent 路径（`/root`、`/root/turing`、`/root/turing/curie`）与零迁移存储编解码。

use crate::config::subagents::BaseRole;

/// 根会话固定路径。
pub const ROOT_AGENT_PATH: &str = "/root";
/// 单个路径段最大长度。
pub const MAX_PATH_SEGMENT_LEN: usize = 32;
/// 超过最大深度时的稳定错误文本（对齐 codex-main）。
pub const DEPTH_LIMIT_REACHED_CODE: &str = "Agent depth limit reached. Solve the task yourself.";

/// 校验单个 `task_name` / 路径段：`1..=32` 字符，首字符 `[a-z0-9]`，其余 `[a-z0-9_]`，
/// 且不得为 `"root"`。
pub fn is_valid_path_segment(segment: &str) -> bool {
    let bytes = segment.as_bytes();
    if bytes.is_empty() || bytes.len() > MAX_PATH_SEGMENT_LEN || segment == "root" {
        return false;
    }
    if !(bytes[0].is_ascii_lowercase() || bytes[0].is_ascii_digit()) {
        return false;
    }
    bytes
        .iter()
        .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || *b == b'_')
}

/// 将昵称（如 `"Turing"`、`"Turing #2"`、`"vonNeumann"`）规范化为合法路径段
/// （`"turing"`、`"turing_2"`、`"vonneumann"`）；无法得到合法段时回退 `"agent"`。
pub fn slugify_segment(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len().min(MAX_PATH_SEGMENT_LEN));
    let mut prev_underscore = false;
    for ch in raw.trim().chars() {
        if ch.is_ascii_alphanumeric() {
            out.push(ch.to_ascii_lowercase());
            prev_underscore = false;
        } else if !out.is_empty() && !prev_underscore {
            out.push('_');
            prev_underscore = true;
        }
        if out.len() >= MAX_PATH_SEGMENT_LEN {
            break;
        }
    }
    while out.ends_with('_') {
        out.pop();
    }
    if out.is_empty() || out == "root" {
        "agent".to_string()
    } else {
        out
    }
}

/// 计算路径所处深度：`/root` -> 0，`/root/turing` -> 1，`/root/turing/curie` -> 2。
pub fn path_depth(agent_path: &str) -> usize {
    let trimmed = agent_path.trim().trim_end_matches('/');
    if trimmed.is_empty() || trimmed == ROOT_AGENT_PATH {
        return 0;
    }
    trimmed
        .strip_prefix("/root/")
        .map_or(1, |rest| rest.split('/').filter(|s| !s.is_empty()).count())
}

/// 在父路径下拼接子路径段，并确保在 `existing_paths` 中唯一（冲突时自动追加 `_2`, `_3`）。
pub fn allocate_child_path(
    parent_path: &str,
    preferred_segment: &str,
    existing_paths: &[String],
) -> String {
    let parent = {
        let p = parent_path.trim().trim_end_matches('/');
        if p.is_empty() || !p.starts_with("/root") {
            ROOT_AGENT_PATH
        } else {
            p
        }
    };
    let base_seg = if is_valid_path_segment(preferred_segment) {
        preferred_segment.to_string()
    } else {
        slugify_segment(preferred_segment)
    };
    let candidate = format!("{parent}/{base_seg}");
    if !existing_paths
        .iter()
        .any(|path| path.eq_ignore_ascii_case(&candidate))
    {
        return candidate;
    }
    let mut suffix = 2usize;
    loop {
        let next = format!("{parent}/{base_seg}_{suffix}");
        if !existing_paths
            .iter()
            .any(|path| path.eq_ignore_ascii_case(&next))
        {
            return next;
        }
        suffix = suffix.saturating_add(1);
    }
}

/// 编码 `BaseRole` 与 `agent_path` 到数据库 `agent_path` 列：`"executor:/root/turing"`。
pub fn encode_stored_role_and_path(base: BaseRole, canonical_path: &str) -> String {
    format!("{}:{}", base.as_str(), canonical_path.trim())
}

/// 从数据库 `agent_path` 列解码 `(Option<BaseRole>, Option<String>)`：
/// - 新格式 `"executor:/root/turing"` -> `(Some(Executor), Some("/root/turing"))`
/// - 旧格式 `"explorer"` / `"executor"` -> `(Some(Explorer/Executor), None)`
/// - 纯路径 `"/root/turing"` -> `(None, Some("/root/turing"))`
pub fn decode_stored_role_and_path(stored: Option<&str>) -> (Option<BaseRole>, Option<String>) {
    let Some(raw) = stored.map(str::trim).filter(|s| !s.is_empty()) else {
        return (None, None);
    };
    if let Some((left, right)) = raw.split_once(':') {
        let base = BaseRole::parse(left);
        let path = right.trim();
        let path_opt = (!path.is_empty()).then(|| path.to_string());
        return (base, path_opt);
    }
    if let Some(base) = BaseRole::parse(raw) {
        return (Some(base), None);
    }
    if raw.starts_with('/') {
        return (None, Some(raw.to_string()));
    }
    (None, None)
}

/// 三合一寻址：`agent_id` / `nickname` / `agent_path`（绝对或相对 `caller_path`）。
pub fn matches_agent_target(
    agent_id: &str,
    nickname: Option<&str>,
    agent_path: Option<&str>,
    caller_path: &str,
    target: &str,
) -> bool {
    let trimmed = target.trim();
    if trimmed.is_empty() {
        return false;
    }
    if agent_id == trimmed {
        return true;
    }
    if nickname.is_some_and(|name| name.eq_ignore_ascii_case(trimmed)) {
        return true;
    }
    let Some(path) = agent_path.map(str::trim).filter(|path| !path.is_empty()) else {
        return false;
    };
    if path.eq_ignore_ascii_case(trimmed) {
        return true;
    }
    if trimmed.starts_with('/') {
        return false;
    }
    let rel_caller = format!("{}/{trimmed}", caller_path.trim_end_matches('/'));
    path.eq_ignore_ascii_case(&rel_caller) || path.eq_ignore_ascii_case(&format!("/root/{trimmed}"))
}

/// 路径前缀匹配：`prefix` 不以 `/` 开头时按根树相对路径处理（`"/root/<prefix>"`）。
pub fn path_matches_prefix(agent_path: &str, prefix: &str) -> bool {
    let trimmed = prefix.trim();
    let normalized = if trimmed.starts_with('/') {
        trimmed.to_string()
    } else {
        format!("/root/{trimmed}")
    };
    let normalized = normalized.trim_end_matches('/');
    if normalized.is_empty() {
        return true;
    }
    let path = agent_path.trim();
    path.eq_ignore_ascii_case(normalized)
        || path
            .to_ascii_lowercase()
            .starts_with(&format!("{}/", normalized.to_ascii_lowercase()))
}
