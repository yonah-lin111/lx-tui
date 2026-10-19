//! 全局 prompt 历史：跨窗格、跨工作区、跨会话共享的已发送 prompt 列表。
//!
//! 只存输入框原文（`trim` 后），不存展开内容；去重后按最近使用排在末尾，
//! 超出 [`PROMPT_HISTORY_LIMIT`] 时丢弃最旧条目。持久化由 `db` 负责，
//! 本模块保持纯数据、无 IO。

/// 全局保留的历史条数上限。
pub const PROMPT_HISTORY_LIMIT: usize = 100;

/// 单条历史文本字节上限；超限的 prompt 不写入历史（仍正常发送）。
pub const PROMPT_HISTORY_MAX_BYTES: usize = 32 * 1024;

/// 全局 prompt 历史；末尾为最近使用的一条。
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct PromptHistory {
    /// 条目正文（输入框原文），最多 [`PROMPT_HISTORY_LIMIT`] 条，末尾最新。
    entries: Vec<String>,
}

impl PromptHistory {
    /// 空历史。
    pub fn new() -> Self {
        Self::default()
    }

    /// 全部条目（旧 → 新，末尾为最近使用）。
    pub fn entries(&self) -> &[String] {
        &self.entries
    }

    /// 是否没有任何条目。
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// 条目数量。
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// 取第 `index` 条（0 为最旧）；越界返回 None。
    pub fn get(&self, index: usize) -> Option<&str> {
        self.entries.get(index).map(String::as_str)
    }

    /// 记录一条已发送 prompt：去重后置顶，裁剪到上限。
    ///
    /// 空白内容、超 [`PROMPT_HISTORY_MAX_BYTES`] 的内容直接忽略。已存在相同文本时
    /// 移除旧位置并追加到末尾（最近使用置顶）。返回历史是否发生变化。
    pub fn record(&mut self, text: &str) -> bool {
        let trimmed = text.trim();
        if trimmed.is_empty() || trimmed.len() > PROMPT_HISTORY_MAX_BYTES {
            return false;
        }
        if self.entries.iter().any(|entry| entry == trimmed) {
            self.entries.retain(|entry| entry != trimmed);
        }
        self.entries.push(trimmed.to_string());
        if self.entries.len() > PROMPT_HISTORY_LIMIT {
            let excess = self.entries.len() - PROMPT_HISTORY_LIMIT;
            self.entries.drain(..excess);
        }
        true
    }

    /// 用启动时载入的条目整体替换（保持传入顺序，末尾为最新）。
    ///
    /// 逐条按 [`Self::record`] 的口径过滤去重；传入顺序即最终顺序。
    pub fn replace_all(&mut self, entries: Vec<String>) {
        self.entries.clear();
        for entry in entries {
            self.record(&entry);
        }
    }
}

#[cfg(test)]
mod tests;
