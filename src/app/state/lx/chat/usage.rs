//! 上下文占用：单步 usage 合并、重置与 `/compact` 边界重算。
//!
//! 上下文占用只认**最后一次模型响应**的 `input + output`；多步工具循环不累加。
//! 所有方法都是纯状态计算，IO 与事件分发留在 update 层。

use super::{ChatStatus, LxChat, ModelsSnapshot};
use crate::app::state::LxCompact;
use crate::db::chat::Usage;

impl LxChat {
    /// 清空上下文占用分子（切模型/新建会话）。
    pub fn input_tokens_reset(&mut self) {
        self.context_tokens = None;
        self.last_tokens_model = None;
        self.step_usage = Usage::default();
    }

    /// 重算状态行展示数据：模型/供应商展示名、上限、用量与上下文百分比来自当前选择
    /// （或默认）；输入 token 只在仍属同一模型时保留（切模型清空分子）。
    pub fn refresh_status(&mut self, models: &ModelsSnapshot) {
        let selection = self.effective_selection(models);
        let (model_name, provider_name, variant, context_limit) = match &selection {
            Some(selection) => {
                let provider = models.settings.providers.get(&selection.provider);
                let model = provider.and_then(|provider| provider.models.get(&selection.model));
                let provider_name = provider.map(|provider| provider.name.clone());
                let model_name = model
                    .map(|model| model.name.clone())
                    .or_else(|| Some(selection.model.clone()));
                let limit = model.and_then(|model| model.limit.map(|limit| limit.context));
                // 思考等级与模型名同源（都来自当前生效选择），保证两者永不漂移。
                (model_name, provider_name, selection.variant.clone(), limit)
            }
            None => (None, None, None, None),
        };
        // 切模型清空分子：当前模型（provider/model 稳定键）与上次记录不一致时重置。
        let token_key = selection
            .as_ref()
            .map(|selection| format!("{}/{}", selection.provider, selection.model));
        if let Some(previous) = &self.last_tokens_model
            && Some(previous) != token_key.as_ref()
        {
            self.input_tokens_reset();
        }
        self.last_tokens_model = token_key;
        self.status = ChatStatus {
            model_name,
            provider_name,
            variant,
            context_limit,
            input_tokens: self.context_tokens,
            streaming: self.request.is_busy(),
            omitted_turns: self.omitted_turns_notice.take(),
        };
    }

    /// 合并同一次模型响应内分帧到达的 usage，并立即刷新上下文占用。
    ///
    /// 只认最后一次模型响应的单步快照：`context_tokens = input + output`，绝不跨
    /// 工具步累加 input。分帧字段后到者覆盖先到者（Anthropic 的 input 在
    /// `message_start`、output 在 `message_delta`）。
    pub fn absorb_usage_delta(&mut self, delta: &Usage) {
        if delta.input.is_some() {
            self.step_usage.input = delta.input;
        }
        if delta.output.is_some() {
            self.step_usage.output = delta.output;
        }
        if delta.cache_read.is_some() {
            self.step_usage.cache_read = delta.cache_read;
        }
        if delta.cache_write.is_some() {
            self.step_usage.cache_write = delta.cache_write;
        }
        if let Some(context) = self.step_usage.context_tokens() {
            self.context_tokens = Some(context);
        }
    }

    /// 一次模型响应结束：清空单步聚合槽，避免上一步的 output 混进下一步。
    pub fn reset_step_usage(&mut self) {
        self.step_usage = Usage::default();
    }

    /// 按当前消息列表与生效的 `/compact` 检查点边界重算上下文占用。
    ///
    /// 对齐 OpenCode `lastAssistantWithUsage`：只取最新生效压缩边界之后的最后一条
    /// 带用量的 assistant 消息；压缩后尚无新回复时清空占用分子（隐藏旧百分比）。
    /// 模型键由 `refresh_status` 按当前选择维护，这里只负责分子。
    pub fn recompute_context_tokens(&mut self, compact: &LxCompact) {
        let boundary = compact.active_upto_sequence();
        let recent = self.messages.iter().rev().find(|message| {
            message.role == "assistant"
                && boundary.is_none_or(|upto| message.sequence > upto)
                && (message.input_tokens.is_some() || message.output_tokens.is_some())
        });
        self.context_tokens = recent.map(|message| {
            let input = message.input_tokens.unwrap_or(0).max(0);
            let output = message.output_tokens.unwrap_or(0).max(0);
            u64::try_from(input.saturating_add(output)).unwrap_or(u64::MAX)
        });
    }
}
