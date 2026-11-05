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
        if p.is_empty() || !(p == ROOT_AGENT_PATH || p.starts_with("/root/")) {
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
        let suffix_str = format!("_{suffix}");
        let max_base = MAX_PATH_SEGMENT_LEN.saturating_sub(suffix_str.len()).max(1);
        let trimmed_base = base_seg
            .get(..base_seg.len().min(max_base))
            .unwrap_or(base_seg.as_str())
            .trim_end_matches('_');
        let trimmed_base = if trimmed_base.is_empty() {
            "a"
        } else {
            trimmed_base
        };
        let next = format!("{parent}/{trimmed_base}{suffix_str}");
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
    if let Some((left, right)) = raw.split_once(':')
        && let Some(base) = BaseRole::parse(left)
    {
        let path = right.trim();
        let path_opt = (!path.is_empty()).then(|| path.to_string());
        return (Some(base), path_opt);
    }
    if let Some(base) = BaseRole::parse(raw) {
        return (Some(base), None);
    }
    if raw.starts_with('/') {
        return (None, Some(raw.to_string()));
    }
    (None, None)
}

/// 三合一寻址（优先级：`agent_id` → `agent_path`（绝对或相对 `caller_path`） → `nickname`）。
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
    let norm_target = trimmed
        .strip_prefix("./")
        .unwrap_or(trimmed)
        .trim_end_matches('/');
    if let Some(raw_path) = agent_path.map(str::trim).filter(|path| !path.is_empty()) {
        let path = raw_path.trim_end_matches('/');
        if !norm_target.is_empty() {
            if path.eq_ignore_ascii_case(norm_target) {
                return true;
            }
            if !norm_target.starts_with('/') {
                let caller = caller_path.trim().trim_end_matches('/');
                let caller = if caller.is_empty() {
                    ROOT_AGENT_PATH
                } else {
                    caller
                };
                let rel_caller = format!("{caller}/{norm_target}");
                let rel_root = format!("/root/{norm_target}");
                if path.eq_ignore_ascii_case(&rel_caller) || path.eq_ignore_ascii_case(&rel_root) {
                    return true;
                }
            }
        }
    }
    nickname.is_some_and(|name| name.eq_ignore_ascii_case(trimmed))
}

/// 判断 `candidate_path` 是否为 `ancestor_path` 的严格后代（不含自身）。
pub fn is_strict_descendant_path(ancestor_path: &str, candidate_path: &str) -> bool {
    let ancestor = ancestor_path.trim().trim_end_matches('/');
    let candidate = candidate_path.trim().trim_end_matches('/');
    if ancestor.is_empty() || candidate.is_empty() || ancestor.eq_ignore_ascii_case(candidate) {
        return false;
    }
    candidate
        .to_ascii_lowercase()
        .starts_with(&format!("{}/", ancestor.to_ascii_lowercase()))
}

/// 路径前缀匹配：`prefix` 不以 `/` 开头时按根树相对路径处理（`"/root/<prefix>"`）。
pub fn path_matches_prefix(agent_path: &str, prefix: &str) -> bool {
    path_matches_caller_prefix(agent_path, ROOT_AGENT_PATH, prefix)
}

/// 支持调用者相对前缀的路径前缀匹配：`prefix` 不以 `/` 开头时同时匹配 `<caller_path>/<prefix>` 与 `/root/<prefix>`。
pub fn path_matches_caller_prefix(agent_path: &str, caller_path: &str, prefix: &str) -> bool {
    let trimmed = prefix.trim().strip_prefix("./").unwrap_or(prefix.trim());
    let trimmed = trimmed.trim_end_matches('/');
    if trimmed.is_empty() {
        return true;
    }
    let path = agent_path.trim().trim_end_matches('/');
    let matches_norm = |norm: &str| {
        let norm = norm.trim_end_matches('/');
        if norm.is_empty() {
            return true;
        }
        path.eq_ignore_ascii_case(norm)
            || path
                .to_ascii_lowercase()
                .starts_with(&format!("{}/", norm.to_ascii_lowercase()))
    };
    if trimmed.starts_with('/') {
        return matches_norm(trimmed);
    }
    let caller = caller_path.trim().trim_end_matches('/');
    let caller = if caller.is_empty() {
        ROOT_AGENT_PATH
    } else {
        caller
    };
    matches_norm(&format!("{caller}/{trimmed}")) || matches_norm(&format!("/root/{trimmed}"))
}
