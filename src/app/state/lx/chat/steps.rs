//! 聊天步骤视图：一条 assistant 消息的**唯一有序时间线**（正文 / 思考 / 工具调用）。
//!
//! 每轮只有一条 assistant 消息；`steps` 的 `position` 由 app 层在内存中按
//! 「已有最大 position + 1」分配，DB 的 `agent_message_part` 与 `agent_tool_call`
//! 两表共用同一消息内编号。UI、鼠标命中、滚动几何与上下文回放都以本时间线为
//! 唯一顺序来源，不得再按时间戳二次排序。

use crate::chat::events::{MAX_SUMMARY_CHARS, PartKind};

/// 一个展示用内容块（消息内的有序 part）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChatPartView {
    /// `text` / `reasoning`。
    pub kind: PartKind,
    pub content: String,
    /// 供应商元数据 JSON（如 Anthropic signature）。
    pub provider_metadata_json: Option<String>,
    /// 该 part 首个增量到达时刻（毫秒）；本轮流式写入，历史加载时从 DB 恢复。
    pub started_at_ms: Option<i64>,
    /// 该 part 最后一个增量到达时刻（毫秒）；None 表示未知（思考耗时不显示）。
    /// 注意它不是「已完成」：思考完成看 [`ChatPartView::completed_at_ms`]。
    pub ended_at_ms: Option<i64>,
    /// 该 part 独立完成时刻（毫秒）；仅 reasoning 使用，None 表示尚未完成或非思考块。
    pub completed_at_ms: Option<i64>,
}

impl ChatPartView {
    /// 存储用稳定 kind 字符串。
    pub fn kind_str(&self) -> &'static str {
        self.kind.as_str()
    }

    /// 是否是一个**仍开放**的思考块（独立完成字段为准，不看消息状态）。
    pub fn reasoning_open(&self) -> bool {
        self.kind == PartKind::Reasoning && self.completed_at_ms.is_none()
    }
}

/// 工具调用状态（UI 语义）。
///
/// 状态机 `input_streaming -> pending -> running -> completed | failed | cancelled`，
/// 另有异常恢复写入的 `interrupted`。`InputStreaming` 只存在于内存（参数未完整接收，
/// 尚无副作用）；`Pending` 必须已入库（DB 由 agent loop 阶段保证）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolStatus {
    /// 正在接收参数（不可执行）。
    InputStreaming,
    /// 参数完整且已（或即将）入库，等待执行。
    Pending,
    /// 执行中。
    Running,
    /// 成功完成。
    Completed,
    /// 执行失败（工具级错误）。
    Failed,
    /// 用户取消（尚未开始执行）。
    Cancelled,
    /// 异常中断（崩溃/连接断开后恢复；结果未知）。
    Interrupted,
}

impl ToolStatus {
    /// DB 状态字符串 → 视图状态；未知值返回 None（调用方按 `interrupted` 保守处理）。
    pub fn from_stored(value: &str) -> Option<Self> {
        match value {
            "pending" => Some(Self::Pending),
            "running" => Some(Self::Running),
            "completed" => Some(Self::Completed),
            "failed" => Some(Self::Failed),
            "cancelled" => Some(Self::Cancelled),
            "interrupted" => Some(Self::Interrupted),
            _ => None,
        }
    }

    /// 是否终态（不再变化）。
    pub fn is_terminal(self) -> bool {
        matches!(
            self,
            Self::Completed | Self::Failed | Self::Cancelled | Self::Interrupted
        )
    }

    /// 是否仍未进入终态。
    pub fn is_open(self) -> bool {
        !self.is_terminal()
    }

    /// 存储用稳定值（与 DB `agent_tool_call.status` 同一口径）。
    ///
    /// `input_streaming` 只存在于内存（参数未完整、尚未入库），DB 不会出现该值；
    /// 上下文回放只认 `completed` / `failed`，其余状态即使出现也不会伪造结果。
    pub fn as_str(self) -> &'static str {
        match self {
            Self::InputStreaming => "input_streaming",
            Self::Pending => "pending",
            Self::Running => "running",
            Self::Completed => "completed",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
            Self::Interrupted => "interrupted",
        }
    }
}

/// 工具步骤的 UI 摘要：只装路径/行范围/计数，**不装**完整文件内容。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ToolSummary {
    /// 目标路径（相对/绝对按模型给出的形式，超长按 [`MAX_SUMMARY_CHARS`] 截断）。
    pub path: Option<String>,
    /// 1-based 起止行（`read` 类工具的行范围）。
    pub range: Option<(usize, usize)>,
    /// 新增行数。
    pub added: Option<usize>,
    /// 删除行数。
    pub removed: Option<usize>,
    /// 命中条目数（`search`）。
    pub matches: Option<usize>,
    /// 内容是否被截断（必须显式告知用户）。
    pub truncated: bool,
}

impl ToolSummary {
    /// 从参数 JSON 提取最小摘要（路径 + `read` 行范围）。
    ///
    /// 只读参数，不执行任何 IO；执行结果（增删计数/命中数/截断）由工具执行层在
    /// 结算时补齐。参数不是 JSON 对象或没有可用字段时返回空摘要。
    pub fn from_arguments(arguments_json: &str) -> Self {
        let Ok(value) = serde_json::from_str::<serde_json::Value>(arguments_json) else {
            return Self::default();
        };
        let path = value
            .get("path")
            .and_then(|path| path.as_str())
            .filter(|path| !path.is_empty())
            .map(truncate_summary);
        let offset = value
            .get("offset")
            .and_then(|offset| offset.as_u64())
            .map(|offset| usize::try_from(offset).unwrap_or(usize::MAX));
        let limit = value
            .get("limit")
            .and_then(|limit| limit.as_u64())
            .map(|limit| usize::try_from(limit).unwrap_or(usize::MAX));
        let range = match (offset, limit) {
            (Some(offset), Some(limit)) if limit > 0 => {
                let start = offset.max(1);
                Some((start, start.saturating_add(limit).saturating_sub(1)))
            }
            (Some(offset), None) => {
                let start = offset.max(1);
                Some((start, start))
            }
            _ => None,
        };
        Self {
            path,
            range,
            ..Self::default()
        }
    }
}

/// 摘要单字段截断（按字符数，避免切断 UTF-8）。
fn truncate_summary(value: &str) -> String {
    if value.chars().count() <= MAX_SUMMARY_CHARS {
        return value.to_string();
    }
    value.chars().take(MAX_SUMMARY_CHARS).collect()
}

/// 一次工具调用的展示数据。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolCallView {
    /// 供应商 call ID（无原生 ID 时由传输层合成的稳定内部 ID）；同一会话内唯一。
    pub call_id: String,
    /// 工具名（`read` / `search` / `edit` / `write`）。
    pub name: String,
    /// 该步骤在 assistant 时间线内的位置（与 part 共用编号空间）。
    pub position: i64,
    /// 已接收的参数 JSON（`InputStreaming` 期间可能是半截内容）。
    pub arguments_json: String,
    pub status: ToolStatus,
    /// UI 摘要（路径/行范围/增删计数等），不装完整文件内容。
    pub summary: Option<ToolSummary>,
    /// 失败/中断原因（稳定错误码）；仅 `Failed` / `Interrupted` 等有值。
    pub error: Option<String>,
    /// 落库的结果体原文（规范化 JSON）；仅历史加载与结算后填充。
    ///
    /// 上下文回放需要它把「本轮已发生的调用/结果」与历史同源重建；UI 只按需读取摘要，
    /// 绝不把本字段铺进历史正文。
    pub result_json: Option<String>,
}

impl ToolCallView {
    /// 新建一个正在接收参数的工具步骤。
    pub fn new(call_id: impl Into<String>, name: impl Into<String>, position: i64) -> Self {
        Self {
            call_id: call_id.into(),
            name: name.into(),
            position,
            arguments_json: String::new(),
            status: ToolStatus::InputStreaming,
            summary: None,
            error: None,
            result_json: None,
        }
    }

    /// 参数接收完成：写入完整 JSON 并转 `Pending`；JSON 非法返回 false 并记为失败。
    pub fn accept_arguments(&mut self, arguments_json: String) -> bool {
        self.arguments_json = arguments_json;
        if serde_json::from_str::<serde_json::Value>(&self.arguments_json).is_err() {
            return false;
        }
        self.summary = Some(ToolSummary::from_arguments(&self.arguments_json));
        self.status = ToolStatus::Pending;
        true
    }

    /// 记为失败（参数超限/非法、执行失败等）。
    pub fn fail(&mut self, error: impl Into<String>) {
        self.status = ToolStatus::Failed;
        self.error = Some(error.into());
    }

    /// 结算为终态（不伪造结果：只写状态与原因）。
    pub fn settle(&mut self, status: ToolStatus, error: Option<String>) {
        self.status = status;
        self.error = error;
    }
}

/// assistant 时间线中的一个步骤：内容块或工具调用（按 `position` 唯一有序）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AssistantStepView {
    Part { position: i64, part: ChatPartView },
    Tool { position: i64, call: ToolCallView },
}

impl AssistantStepView {
    /// 时间线位置（跨 part/tool 共用的消息内编号）。
    pub fn position(&self) -> i64 {
        match self {
            Self::Part { position, .. } | Self::Tool { position, .. } => *position,
        }
    }

    /// 内容块视图（工具步骤为 None）。
    pub fn part(&self) -> Option<&ChatPartView> {
        match self {
            Self::Part { part, .. } => Some(part),
            Self::Tool { .. } => None,
        }
    }

    /// 可变内容块视图（工具步骤为 None）。
    pub fn part_mut(&mut self) -> Option<&mut ChatPartView> {
        match self {
            Self::Part { part, .. } => Some(part),
            Self::Tool { .. } => None,
        }
    }

    /// 工具调用视图（内容块步骤为 None）。
    pub fn tool(&self) -> Option<&ToolCallView> {
        match self {
            Self::Tool { call, .. } => Some(call),
            Self::Part { .. } => None,
        }
    }

    /// 可变工具调用视图（内容块步骤为 None）。
    pub fn tool_mut(&mut self) -> Option<&mut ToolCallView> {
        match self {
            Self::Tool { call, .. } => Some(call),
            Self::Part { .. } => None,
        }
    }
}

/// 一条展示用消息。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChatMessageView {
    pub id: String,
    /// `user` / `assistant`。
    pub role: String,
    /// `complete` / `streaming` / `interrupted` / `failed`。
    pub status: String,
    pub turn_index: i64,
    /// 有序步骤时间线（用户消息只有单个 text part；assistant 可含思考与工具）。
    pub steps: Vec<AssistantStepView>,
    /// 该轮实际模型展示名（assistant 行）。
    pub model_label: Option<String>,
    /// 失败原因码（如 `http 503`、`timeout`）；仅 `failed` 时有值，用于历史行提示。
    pub error_code: Option<String>,
    /// 该轮供应商报的输入 / 输出 / 缓存读取 token（未知为 None，不写 0 冒充）。
    pub input_tokens: Option<i64>,
    pub output_tokens: Option<i64>,
    pub cache_read_tokens: Option<i64>,
    /// 该轮耗时（毫秒；未完成或时间戳缺失为 None）。
    pub duration_ms: Option<i64>,
}

impl ChatMessageView {
    /// 全部内容块（按时间线顺序；不含工具步骤）。
    pub fn parts(&self) -> impl Iterator<Item = &ChatPartView> {
        self.steps.iter().filter_map(AssistantStepView::part)
    }

    /// 正文内容块。
    pub fn text_parts(&self) -> impl Iterator<Item = &ChatPartView> {
        self.parts().filter(|part| part.kind == PartKind::Text)
    }

    /// 思考内容块。
    pub fn reasoning_parts(&self) -> impl Iterator<Item = &ChatPartView> {
        self.parts().filter(|part| part.kind == PartKind::Reasoning)
    }

    /// 全部工具步骤。
    pub fn tools(&self) -> impl Iterator<Item = &ToolCallView> {
        self.steps.iter().filter_map(AssistantStepView::tool)
    }

    /// 指定索引的步骤（索引即时间线下标，也是 UI 命中用的 `step_index`）。
    pub fn step_at(&self, index: usize) -> Option<&AssistantStepView> {
        self.steps.get(index)
    }

    /// 拼接全部正文（不含思考与工具）。
    pub fn text(&self) -> String {
        self.text_parts()
            .map(|part| part.content.as_str())
            .collect()
    }

    /// 消息内已分配的最大时间线位置；空消息为 None。
    pub fn max_position(&self) -> Option<i64> {
        self.steps.iter().map(AssistantStepView::position).max()
    }

    /// 下一个可分配的时间线位置（已有最大 + 1）。
    pub fn next_position(&self) -> i64 {
        self.max_position()
            .map_or(0, |position| position.saturating_add(1))
    }

    /// 按 call ID 查找工具步骤。
    pub fn tool(&self, call_id: &str) -> Option<&ToolCallView> {
        self.tools().find(|call| call.call_id == call_id)
    }

    /// 按 call ID 查找可变工具步骤。
    pub fn tool_mut(&mut self, call_id: &str) -> Option<&mut ToolCallView> {
        self.steps
            .iter_mut()
            .filter_map(AssistantStepView::tool_mut)
            .find(|call| call.call_id == call_id)
    }

    /// 追加一个内容块步骤，返回分配的 position。
    pub fn push_part(&mut self, part: ChatPartView) -> i64 {
        let position = self.next_position();
        self.steps.push(AssistantStepView::Part { position, part });
        position
    }

    /// 追加一个工具步骤，返回分配的 position（`call.position` 以分配值为准）。
    pub fn push_tool(&mut self, mut call: ToolCallView) -> i64 {
        let position = self.next_position();
        call.position = position;
        self.steps.push(AssistantStepView::Tool { position, call });
        position
    }

    /// 最后一个步骤若是**未完成**的思考块则返回其可变引用（当前唯一「开放」定义）。
    pub fn open_reasoning_mut(&mut self) -> Option<&mut ChatPartView> {
        match self.steps.last_mut() {
            Some(AssistantStepView::Part { part, .. }) if part.reasoning_open() => Some(part),
            _ => None,
        }
    }

    /// 最后一个步骤若是工具步骤则返回其可变引用。
    pub fn last_tool_mut(&mut self) -> Option<&mut ToolCallView> {
        match self.steps.last_mut() {
            Some(AssistantStepView::Tool { call, .. }) => Some(call),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests;
