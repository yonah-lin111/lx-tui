//! 子 agent Token 预算记账与 `<rollout_budget>` 阈值提醒（对齐 codex-main RolloutBudget）。

use crate::db::chat::Usage;

/// 默认提醒阈值比例：剩余 25% 与剩余 10% 时各注入一次提醒。
const REMINDER_RATIOS: [f64; 2] = [0.25, 0.10];

/// 单个子 agent 单轮 Token 预算记账器。
#[derive(Debug, Clone)]
pub struct SubagentTokenBudget {
    limit_tokens: i64,
    used_tokens: i64,
    /// 已触发过的提醒档位索引（0 = 未触发，1 = 已触发 25%，2 = 已触发 10%）。
    delivered_stage: usize,
}

impl SubagentTokenBudget {
    /// 构造预算器；非正上限视为未配置并返回 `None`。
    pub fn new(limit_tokens: i64) -> Option<Self> {
        (limit_tokens > 0).then_some(Self {
            limit_tokens,
            used_tokens: 0,
            delivered_stage: 0,
        })
    }

    /// 预算上限（token）。
    pub const fn limit_tokens(&self) -> i64 {
        self.limit_tokens
    }

    /// 已累计消耗（token）。
    pub const fn used_tokens(&self) -> i64 {
        self.used_tokens
    }

    /// 剩余可用（token，不返回负数）。
    pub const fn remaining_tokens(&self) -> i64 {
        let remaining = self.limit_tokens - self.used_tokens;
        if remaining > 0 { remaining } else { 0 }
    }

    /// 累加单步采样消耗的 token（input + output），返回是否已耗尽全部预算。
    pub fn record_step_usage(&mut self, step: Usage) -> bool {
        let step_total = step
            .input
            .unwrap_or(0)
            .max(0)
            .saturating_add(step.output.unwrap_or(0).max(0));
        self.used_tokens = self.used_tokens.saturating_add(step_total);
        self.used_tokens >= self.limit_tokens
    }

    /// 检查是否跨过新的剩余阈值（25% / 10%）；若是则返回待注入的英文提醒文本。
    pub fn take_pending_reminder(&mut self) -> Option<String> {
        let remaining = self.remaining_tokens();
        if remaining == 0 {
            return None;
        }
        let mut crossed_stage = 0usize;
        for (index, ratio) in REMINDER_RATIOS.iter().enumerate() {
            let threshold = ((self.limit_tokens as f64) * ratio).floor() as i64;
            if remaining <= threshold {
                crossed_stage = index + 1;
            }
        }
        if crossed_stage > self.delivered_stage {
            self.delivered_stage = crossed_stage;
            Some(format!(
                "<rollout_budget>\nYou have {} tokens left in your token budget (limit: {}). Wrap up active investigation and produce your final summary soon.\n</rollout_budget>",
                remaining, self.limit_tokens
            ))
        } else {
            None
        }
    }
}
