//! 流事件 → 消息步骤时间线的纯状态转换。
//!
//! 只改内存状态、不执行 IO：delta 的语义见 [`crate::chat::events::ChatDelta`]。
//! 关键不变量：
//!
//! 1. 一条 assistant 消息只有一个有序步骤列表；新步骤 `position = 已有最大 + 1`。
//! 2. 相邻同 kind 的文本增量只改原 part，`position` 不变。
//! 3. 思考块的**完成**由 `completed_at_ms` 独立表达（`ended_at_ms` 仍表示最后一段增量）；
//!    开始正文、开始工具输入或开始下一个思考块前先关闭当前开放思考块，只写一次。
//! 4. 工具参数未完整接收前不执行任何操作；`ToolInputEnded` 只把状态推进到 `Pending`。
//! 5. 终态（`Finished`）不伪造工具结果：未结算工具标 `Cancelled` / `Interrupted`。

use crate::chat::events::{
    MAX_TOOL_ARGUMENTS_BYTES, ModelStepStop, PartKind, TOOL_ARGUMENTS_INVALID_CODE,
    TOOL_ARGUMENTS_TOO_LARGE_CODE,
};

use super::LxChat;
use super::steps::{ChatMessageView, ChatPartView, ToolCallView, ToolStatus, ToolSummary};

/// 工具步骤在终态收尾时的最终去向（只决定状态与原因码，不伪造结果体）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum OpenToolOutcome {
    /// 用户取消且尚未执行：标取消。
    Cancelled,
    /// 结果未知（失败/中断/未结算）：标中断，不伪造结果。
    Interrupted,
}

impl LxChat {
    /// 追加/合并一个流式 delta part 到最后一条 assistant 消息。
    ///
    /// 相邻同 kind 且元数据兼容的 delta 合并；类型切换或带元数据的空 delta
    /// 新建 part，保持 text/reasoning 交错顺序。切换到正文或新思考块前，
    /// 先关闭当前开放的思考块（完成时间只写一次）。返回是否有状态变化。
    pub fn append_stream_delta(
        &mut self,
        kind: PartKind,
        text: &str,
        metadata: Option<serde_json::Value>,
    ) -> bool {
        let now = crate::db::now_ms();
        let Some(message) = self.streaming_message_mut() else {
            return false;
        };
        // 签名元数据（text 为空）绑定到同 kind 的相邻 part，不新建空块、也不关闭思考。
        if let Some(meta) = metadata.as_ref()
            && text.is_empty()
        {
            if let Some(last) = message.steps.last_mut()
                && let Some(part) = last.part_mut()
                && part.kind == kind
                && part.provider_metadata_json.is_none()
            {
                part.provider_metadata_json = Some(serde_json::to_string(meta).unwrap_or_default());
                return true;
            }
            // 无同 kind 相邻块：不创建空 part（与旧行为一致）。
            return false;
        }
        // 相邻同 kind 的纯文本增量：只改原 part，position 不变。
        if metadata.is_none()
            && let Some(last) = message.steps.last_mut()
            && let Some(part) = last.part_mut()
            && part.kind == kind
            && part.provider_metadata_json.is_none()
        {
            part.content.push_str(text);
            part.ended_at_ms = Some(now);
            return true;
        }
        // 新建块：先关闭仍开放的思考块（完成时间早于新块起始）。
        close_open_reasoning(message, now);
        message.push_part(ChatPartView {
            kind,
            content: text.to_string(),
            provider_metadata_json: metadata
                .map(|meta| serde_json::to_string(&meta).unwrap_or_default()),
            started_at_ms: Some(now),
            ended_at_ms: Some(now),
            completed_at_ms: None,
        });
        true
    }

    /// 关闭当前开放的思考块（完成时间只写一次）。
    ///
    /// 「当前开放」= 时间线末尾且 `completed_at_ms` 为空的 reasoning 块。
    /// **限制**：本组不按 item ID 建索引，`ReasoningEnded` 的 `item_id` 只用于文档
    /// 化契约（见 [`Self::reasoning_ended`]），实际只关闭尾部开放块。
    pub fn close_open_reasoning(&mut self) -> bool {
        let now = crate::db::now_ms();
        let Some(message) = self.streaming_message_mut() else {
            return false;
        };
        close_open_reasoning(message, now)
    }

    /// 显式思考结束事件（`ReasoningEnded`）：关闭对应 item，幂等。
    ///
    /// 无 item ID 的协议只关闭当前开放块；对未开始/已完成的 item 是 no-op（不创建空思考块）。
    pub fn reasoning_ended(&mut self, item_id: Option<&str>) -> bool {
        if let Some(item_id) = item_id {
            // 限制：内存中只保留「当前开放块」这一状态，item ID 仅作诊断记录，
            // 未匹配时不创建、不关闭其它块（跨 item 并发结束由后续阶段补齐）。
            tracing::trace!(target: "lx::chat", item_id, "reasoning ended");
        }
        self.close_open_reasoning()
    }

    /// 工具参数开始接收（`ToolInputStarted`）：先关闭开放思考，再插入
    /// `InputStreaming` 工具步骤。同 `call_id` 重复事件忽略（不覆盖已有步骤）。
    ///
    /// `ordinal` 为供应商响应内顺序；本组只用于诊断（内存顺序即到达顺序）。
    pub fn start_tool_input(&mut self, call_id: &str, name: &str, ordinal: usize) -> bool {
        let now = crate::db::now_ms();
        let Some(message) = self.streaming_message_mut() else {
            return false;
        };
        if message.tool(call_id).is_some() {
            // 同响应内重复快照（如 Gemini 累计候选）：只执行一次、不覆盖。
            return false;
        }
        close_open_reasoning(message, now);
        tracing::trace!(
            target: "lx::chat",
            call_id,
            name,
            ordinal,
            "tool input started"
        );
        let position = message.next_position();
        message.push_tool(ToolCallView::new(call_id, name, position));
        true
    }

    /// 工具参数增量（`ToolInputDelta`）：按 `call_id` 追加到对应步骤。
    ///
    /// 累计超过 [`MAX_TOOL_ARGUMENTS_BYTES`] 或 `call_id` 未知时：前者标记该步骤
    /// `Failed`（错误码 [`TOOL_ARGUMENTS_TOO_LARGE_CODE`]）并停止累计；后者忽略。
    pub fn append_tool_input(&mut self, call_id: &str, chunk: &str) -> bool {
        let Some(message) = self.streaming_message_mut() else {
            return false;
        };
        let Some(call) = message.tool_mut(call_id) else {
            return false;
        };
        if call.status != ToolStatus::InputStreaming {
            // 已结束/已失败的调用不再接受增量（迟到帧幂等忽略）。
            return false;
        }
        if call.arguments_json.len().saturating_add(chunk.len()) > MAX_TOOL_ARGUMENTS_BYTES {
            call.fail(TOOL_ARGUMENTS_TOO_LARGE_CODE);
            return true;
        }
        call.arguments_json.push_str(chunk);
        true
    }

    /// 工具参数接收完成（`ToolInputEnded`）：写入完整 JSON，状态转 `Pending`。
    ///
    /// 完整参数非法时标 `Failed`（[`TOOL_ARGUMENTS_INVALID_CODE`]）；**不执行任何 IO**。
    /// `call_id` 未知时（缺 `ToolInputStarted`）不创建步骤，返回 false。
    pub fn end_tool_input(&mut self, call_id: &str, arguments_json: &str) -> bool {
        if arguments_json.len() > MAX_TOOL_ARGUMENTS_BYTES {
            return self.tool_fail(call_id, TOOL_ARGUMENTS_TOO_LARGE_CODE);
        }
        let Some(message) = self.streaming_message_mut() else {
            return false;
        };
        let Some(call) = message.tool_mut(call_id) else {
            return false;
        };
        if call.status != ToolStatus::InputStreaming {
            // 参数已结算：重复结束事件幂等忽略，不覆盖已有状态。
            return false;
        }
        let accepted = call.accept_arguments(arguments_json.to_string());
        if !accepted {
            call.fail(TOOL_ARGUMENTS_INVALID_CODE);
        }
        true
    }

    /// 把指定工具步骤标为失败（超限/非法等）；步骤不存在时返回 false。
    pub fn tool_fail(&mut self, call_id: &str, error: &str) -> bool {
        let Some(message) = self.streaming_message_mut() else {
            return false;
        };
        let Some(call) = message.tool_mut(call_id) else {
            return false;
        };
        if call.status.is_terminal() {
            return false;
        }
        call.fail(error);
        true
    }

    /// DB pending 已提交后的状态推进：`InputStreaming -> Pending`。
    ///
    /// 参数已完整接收但与 `ToolInputEnded` 存在竞态时（例如响应收尾先转 Pending），
    /// 本方法补齐一次；已经是 `Pending` 时幂等返回 false，不伪造其他转移。
    pub fn mark_tool_pending(&mut self, call_id: &str) -> bool {
        let Some(message) = self.streaming_message_mut() else {
            return false;
        };
        let Some(call) = message.tool_mut(call_id) else {
            return false;
        };
        match call.status {
            ToolStatus::InputStreaming if !call.arguments_json.is_empty() => {
                let arguments = std::mem::take(&mut call.arguments_json);
                let accepted = call.accept_arguments(arguments);
                if !accepted {
                    call.fail(TOOL_ARGUMENTS_INVALID_CODE);
                }
                true
            }
            // 已 Pending：DB 也已提交，无需再改状态。
            _ => false,
        }
    }

    /// `Pending -> Running`（DB pending 已入库后由 agent loop 通知；本组不执行 IO）。
    ///
    /// 只有 `Pending` 可推进；重复推进或从其它状态推进返回 false（不伪造转移）。
    pub fn mark_tool_running(&mut self, call_id: &str) -> bool {
        let Some(message) = self.streaming_message_mut() else {
            return false;
        };
        let Some(call) = message.tool_mut(call_id) else {
            return false;
        };
        if call.status != ToolStatus::Pending {
            return false;
        }
        call.status = ToolStatus::Running;
        true
    }

    /// 结算为终态（`Completed` / `Failed` / `Cancelled` / `Interrupted`）。
    ///
    /// 只接受从开放状态（`InputStreaming`/`Pending`/`Running`）的转移；已终态时幂等返回
    /// false。本组只写状态与原因码，**不写入任何伪造的结果体**（结果 JSON 由 G3 结算阶段提供）。
    pub fn settle_tool(
        &mut self,
        call_id: &str,
        status: ToolStatus,
        error: Option<String>,
    ) -> bool {
        self.settle_tool_result(call_id, status, None, None, error)
    }

    /// 结果落库后的终态结算：除状态与错误码外还写入**UI 摘要**与结果体原文。
    ///
    /// 摘要由 `chat::tools::to_summary` 生成、事件层做一次字段映射；摘要绝不含文件内容。
    /// `result_json` 是刚落库的规范化结果（上下文回放需要，UI 不铺开全文）。
    /// 只接受从开放状态的转移，已终态时幂等返回 false（迟到结果不覆盖真实终态）。
    pub fn settle_tool_result(
        &mut self,
        call_id: &str,
        status: ToolStatus,
        summary: Option<ToolSummary>,
        result_json: Option<String>,
        error: Option<String>,
    ) -> bool {
        if !status.is_terminal() {
            return false;
        }
        let Some(message) = self.streaming_message_mut() else {
            return false;
        };
        let Some(call) = message.tool_mut(call_id) else {
            return false;
        };
        if call.status.is_terminal() {
            return false;
        }
        call.settle(status, error);
        if let Some(summary) = summary {
            // 结算只拿到**结果侧**摘要（`chat::tools::to_summary` 没有参数），
            // 因此与重载路径共用同一套合并：参数派生值打底、结果侧覆盖。
            // `tool_view_of` 走 `from_arguments().absorb_result_json()`，两条路同口径。
            let from_args = ToolSummary::from_arguments(&call.arguments_json);
            call.summary = Some(Box::new(from_args.absorb_summary(&summary)));
        }
        if result_json.is_some() {
            call.result_json = result_json;
        }
        true
    }

    /// 本次**模型响应**结束（`ResponseFinished`）：只收尾思考与在途工具输入，
    /// 不结束消息、不产生工具结果。
    pub fn finish_response(&mut self, stop: ModelStepStop) -> bool {
        let now = crate::db::now_ms();
        let Some(message) = self.streaming_message_mut() else {
            return false;
        };
        let mut changed = close_open_reasoning(message, now);
        changed |= settle_streaming_tools(message);
        tracing::trace!(target: "lx::chat", ?stop, "model response finished");
        changed
    }

    /// 整轮结束：关闭仍开放的思考块并结算未终态工具（不伪造结果）。
    ///
    /// - `Complete`：未结算工具按「结果未知」标 `Interrupted`；
    /// - `Cancelled`：尚未执行的工具标 `Cancelled`；
    /// - `Failed`：同 `Interrupted`（结果未知，绝不伪造成功）。
    pub fn close_stream(&mut self, reason: &crate::chat::events::StopReason) -> bool {
        use crate::chat::events::StopReason;
        let now = crate::db::now_ms();
        let Some(message) = self.streaming_message_mut() else {
            return false;
        };
        let mut changed = close_open_reasoning(message, now);
        let outcome = match reason {
            StopReason::Complete => OpenToolOutcome::Interrupted,
            StopReason::Cancelled => OpenToolOutcome::Cancelled,
            StopReason::Failed { .. } => OpenToolOutcome::Interrupted,
        };
        changed |= settle_open_tools(message, outcome);
        changed
    }
}

/// 关闭消息末尾仍开放的思考块（完成时间只写一次）。
fn close_open_reasoning(message: &mut ChatMessageView, now: i64) -> bool {
    let Some(part) = message.open_reasoning_mut() else {
        return false;
    };
    part.completed_at_ms = Some(now);
    true
}

/// 把仍处在 `InputStreaming` 的工具收尾为 `Pending`（参数完整）或 `Failed`（参数非法）。
///
/// 响应结束时参数流本应已闭合；未闭合说明协议截断，记失败而不执行。
fn settle_streaming_tools(message: &mut ChatMessageView) -> bool {
    let mut changed = false;
    for step in &mut message.steps {
        let Some(call) = step.tool_mut() else {
            continue;
        };
        if call.status != ToolStatus::InputStreaming {
            continue;
        }
        let accepted = take_arguments(call);
        if !accepted {
            call.fail(TOOL_ARGUMENTS_INVALID_CODE);
        }
        changed = true;
    }
    changed
}

/// 取走工具步骤当前的参数字符串并按完整 JSON 校验（成功则转 `Pending`）。
fn take_arguments(call: &mut ToolCallView) -> bool {
    let arguments = std::mem::take(&mut call.arguments_json);
    call.accept_arguments(arguments)
}

/// 整轮结束时把未终态工具标为取消/中断（不写结果体）。
fn settle_open_tools(message: &mut ChatMessageView, outcome: OpenToolOutcome) -> bool {
    let (status, error) = match outcome {
        OpenToolOutcome::Cancelled => (ToolStatus::Cancelled, None),
        OpenToolOutcome::Interrupted => (
            ToolStatus::Interrupted,
            Some(crate::db::chat::INTERRUPTED_ERROR_CODE.to_string()),
        ),
    };
    let mut changed = false;
    for step in &mut message.steps {
        let Some(call) = step.tool_mut() else {
            continue;
        };
        if !call.status.is_open() {
            continue;
        }
        if call.summary.is_none() && !call.arguments_json.is_empty() {
            call.summary = Some(Box::new(ToolSummary::from_arguments(&call.arguments_json)));
        }
        call.settle(status, error.clone());
        changed = true;
    }
    changed
}

/// 断言步骤时间线位置严格单调递增（测试与不变量自检用）。
#[cfg(test)]
pub(crate) fn positions_are_monotonic(steps: &[super::steps::AssistantStepView]) -> bool {
    steps
        .windows(2)
        .all(|pair| pair[0].position() < pair[1].position())
}

#[cfg(test)]
mod tests;
