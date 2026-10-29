//! MCP 工具命名：模型可见的扁平名与反查映射。

use std::collections::HashSet;

use crate::platform::path_text::bytes_hash;

/// 模型可见前缀。
pub const MCP_PREFIX: &str = "mcp__";
/// server 与 tool 之间的分隔符。
pub const MCP_DELIMITER: &str = "__";
/// 各家 API 的最小公共上限（OpenAI `^[a-zA-Z0-9_-]{1,64}$`）。
pub const MAX_TOOL_NAME_LEN: usize = 64;

/// 冻结快照里的一条 MCP 工具。
#[derive(Debug, Clone, PartialEq)]
pub struct McpToolEntry {
    pub qualified: String,
    pub server: String,
    pub raw_name: String,
    pub description: String,
    pub parameters: serde_json::Value,
    pub read_only_hint: bool,
}

/// 反查：qualified name → (server, raw tool)。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QualifiedTool {
    pub server: String,
    pub raw_name: String,
}

/// 清洗：非 `[A-Za-z0-9_-]` 一律替换为 `_`；空结果为 `_`。
pub fn sanitize(raw: &str) -> String {
    let cleaned: String = raw
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-') {
                ch
            } else {
                '_'
            }
        })
        .collect();
    if cleaned.is_empty() {
        "_".to_string()
    } else {
        cleaned
    }
}

/// 生成唯一 qualified 名；超长时截断 tool 段并追加 8 位哈希。
pub fn qualified_name(server: &str, raw_tool: &str, taken: &HashSet<String>) -> String {
    let prefix = format!("{MCP_PREFIX}{server}{MCP_DELIMITER}");
    let tool = sanitize(raw_tool);
    let base = format!("{prefix}{tool}");
    if base.len() <= MAX_TOOL_NAME_LEN && !taken.contains(&base) {
        return base;
    }
    // 超长或同名工具（server 返回重复名）走哈希后缀，保证唯一。
    let mut counter = 0u64;
    loop {
        let seed = format!("{server}\0{raw_tool}\0{counter}");
        let suffix = format!("_{:08x}", bytes_hash(seed.as_bytes()) as u32);
        let keep = MAX_TOOL_NAME_LEN.saturating_sub(suffix.len());
        let mut name = format!("{prefix}{tool}");
        if name.len() > keep {
            name.truncate(keep);
        }
        name.push_str(&suffix);
        if !taken.contains(&name) {
            return name;
        }
        counter = counter.saturating_add(1);
    }
}

/// 从工具名反查 server 与原始名（不依赖快照）。
pub fn split_qualified(name: &str) -> Option<(&str, &str)> {
    let rest = name.strip_prefix(MCP_PREFIX)?;
    let (server, tool) = rest.split_once(MCP_DELIMITER)?;
    if server.is_empty() || tool.is_empty() {
        return None;
    }
    Some((server, tool))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitizes_and_prefixes() {
        assert_eq!(
            qualified_name("github", "create_issue", &HashSet::new()),
            "mcp__github__create_issue"
        );
        assert_eq!(
            qualified_name("my-kb", "search.docs v2", &HashSet::new()),
            "mcp__my-kb__search_docs_v2"
        );
    }

    #[test]
    fn long_names_fit_64_chars_and_stay_unique() {
        let taken = HashSet::new();
        let name = qualified_name(
            "company-internal-knowledge-base",
            "search_documents_by_semantic_similarity",
            &taken,
        );
        assert_eq!(name.len(), MAX_TOOL_NAME_LEN);
        assert!(name.starts_with("mcp__company-internal-knowledge-base__"));
        let mut taken = HashSet::new();
        taken.insert(name.clone());
        let other = qualified_name(
            "company-internal-knowledge-base",
            "search_documents_by_semantic_similarity",
            &taken,
        );
        assert_ne!(other, name);
        assert_eq!(other.len(), MAX_TOOL_NAME_LEN);
    }

    #[test]
    fn splits_qualified_names() {
        assert_eq!(
            split_qualified("mcp__github__create_issue"),
            Some(("github", "create_issue"))
        );
        assert_eq!(split_qualified("read"), None);
        assert_eq!(split_qualified("mcp____x"), None);
    }
}
