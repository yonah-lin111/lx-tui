//! 供应商用量：单步上下文占用口径。

/// 供应商返回的用量；未知字段保持 None，不写 0 冒充真实值。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Usage {
    pub input: Option<i64>,
    pub output: Option<i64>,
    pub cache_read: Option<i64>,
    pub cache_write: Option<i64>,
}

impl Usage {
    /// 单步上下文占用：一次模型请求的输入 + 输出。
    ///
    /// 两侧都未知时返回 None（不写 0 冒充真实值）；只有一侧已知时另一侧按 0 计。
    pub fn context_tokens(&self) -> Option<u64> {
        if self.input.is_none() && self.output.is_none() {
            return None;
        }
        let input = self.input.unwrap_or(0).max(0);
        let output = self.output.unwrap_or(0).max(0);
        Some(u64::try_from(input.saturating_add(output)).unwrap_or(u64::MAX))
    }
}
