//! Agent loop 的轮次上下文（[`ModelContext`]）与压缩摘要包裹。

use std::path::{Path, PathBuf};

use crate::chat::events::{ChatRequest, ModelItem};

use super::ReadyToolCall;

/// 本轮上下文：冻结的请求骨架 + 逐步累积的结构化历史项 + 冻结的工作区 cwd。
///
/// 多次采样共用同一上下文（当前轮的 call/result 不回到 DB 重新装配），
/// 只把本次响应产生的内容/调用与工具结果追加进来。
#[derive(Debug, Clone)]
pub struct ModelContext {
    /// 初始请求骨架（turns / model / variant_params 冻结）。
    pub(super) request: ChatRequest,
    /// 结构化历史（初始为装配结果，逐步追加）。
    pub(super) items: Vec<ModelItem>,
    /// 发送时冻结的工作区根；文件工具的相对路径基准。
    pub(super) cwd: PathBuf,
    /// 本轮开始时的**本地运行上下文**摘要（存活进程 + 当前计划）。
    ///
    /// 它不是工具结果，也不伪装成新的调用：只在第一次采样时并入用户消息末尾，
    /// 避免历史裁剪后模型忘记 `session_id` 或把已完成的计划当成当前计划。
    pub(super) runtime_summary: Option<String>,
    /// 模型上下文上限（token）；None 时用 [`super::FALLBACK_CONTEXT_LIMIT_TOKENS`]。
    pub(super) context_limit: Option<u64>,
    /// 运行时自动压缩或持久化检查点恢复的摘要；非空时注入系统提示，替代被丢掉的旧 items。
    pub(super) compact_summary: Option<String>,
    /// 上一次自动压缩失败时的 items 数量（避免同一批 items 压缩失败后死循环重试）。
    pub(super) last_failed_compact_items: Option<usize>,
    /// 当前会话所在的子 agent 深度（根会话为 0，子为 1，孙为 2）。
    pub(super) subagent_depth: usize,
    /// 允许派发子 agent 的最大深度（到达后不再声明控制工具）。
    pub(super) max_subagent_depth: usize,
    /// 当前轮的可选 Token 预算；None 表示不设预算。
    pub(super) token_budget: Option<crate::chat::agent::SubagentTokenBudget>,
}

impl ModelContext {
    /// 以装配好的请求与冻结 cwd 构造上下文。
    pub fn new(request: ChatRequest, cwd: impl Into<PathBuf>) -> Self {
        let items = request.items.clone();
        Self {
            request,
            items,
            cwd: cwd.into(),
            runtime_summary: None,
            context_limit: None,
            compact_summary: None,
            last_failed_compact_items: None,
            // 默认按“第一层子 agent、不可再派发”处理：未显式配置深度的调用方
            // （旧测试与旧路径）不会意外获得递归控制工具。
            subagent_depth: 1,
            max_subagent_depth: 1,
            token_budget: None,
        }
    }

    /// 设置当前会话深度与允许派发的最大深度。
    pub fn with_subagent_depth(mut self, depth: usize, max_depth: usize) -> Self {
        self.subagent_depth = depth;
        self.max_subagent_depth = max_depth;
        self
    }

    /// 当前会话所在的子 agent 深度。
    pub const fn subagent_depth(&self) -> usize {
        self.subagent_depth
    }

    /// 允许派发子 agent 的最大深度。
    pub const fn max_subagent_depth(&self) -> usize {
        self.max_subagent_depth
    }

    /// 设置当前轮的可选 Token 预算上限（`None` / 非正数表示不限）。
    pub fn with_token_budget(mut self, limit_tokens: Option<u64>) -> Self {
        self.token_budget = limit_tokens
            .and_then(|tokens| i64::try_from(tokens).ok())
            .and_then(crate::chat::agent::SubagentTokenBudget::new);
        self
    }

    /// 当前会话是否可以继续派发/管控子 agent。
    ///
    /// 只有 `BaseRole::Executor` 在未达 `max_subagent_depth` 时可递归；`Explorer`
    /// 在任何深度都保持只读（对齐“只读角色永远不派发”）。
    pub fn can_delegate(&self) -> bool {
        match self.request.purpose {
            crate::chat::events::RequestPurpose::Chat
            | crate::chat::events::RequestPurpose::ExecTurn => true,
            crate::chat::events::RequestPurpose::Subagent(role) => {
                role.base() == crate::chat::agent::BaseRole::Executor
                    && self.subagent_depth < self.max_subagent_depth
            }
            crate::chat::events::RequestPurpose::CompactSummary => false,
        }
    }

    /// 附加本地运行上下文摘要（存活进程与当前计划的有界快照）。
    pub fn with_runtime_summary(mut self, summary: Option<String>) -> Self {
        self.runtime_summary = summary;
        self
    }

    /// 设置模型上下文上限；未设置时使用回退值。
    pub fn with_context_limit(mut self, limit: Option<u64>) -> Self {
        self.context_limit = limit.filter(|value| *value > 0);
        self
    }

    /// 注入已持久化的自动压缩摘要检查点（子 agent 多轮恢复时使用）。
    pub fn with_compact_summary(mut self, summary: Option<String>) -> Self {
        self.compact_summary = summary.filter(|text| !text.trim().is_empty());
        self
    }

    /// 当前生效的压缩摘要（测试与诊断用）。
    pub fn compact_summary(&self) -> Option<&str> {
        self.compact_summary.as_deref()
    }

    /// 冻结的工作区根。
    pub fn cwd(&self) -> &Path {
        &self.cwd
    }

    /// 当前结构化历史（诊断与测试用）。
    pub fn items(&self) -> &[ModelItem] {
        &self.items
    }

    /// 下一次采样请求：turns/model 不变，items 为本轮累计结果。
    ///
    /// 运行上下文摘要只在**第一次**采样时并入（后续采样已包含在 items 里）。
    pub(super) fn next_request(&self) -> ChatRequest {
        // 自动压缩的摘要放在系统段：与手动 compact 的注入位置一致，不伪造
        // user/assistant 消息；原始任务仍作为 items 的第一条保留。
        let system_prompt = self.request.system_prompt.as_ref().map(|base| {
            self.compact_summary.as_ref().map_or_else(
                || base.clone(),
                |summary| format!("{base}\n\n{}", compact_summary_prompt(summary)),
            )
        });
        ChatRequest {
            turns: self.request.turns.clone(),
            items: self.items.clone(),
            model: self.request.model.clone(),
            variant_params: self.request.variant_params.clone(),
            // 用途随轮次冻结：子 agent 的请求必须保持 `Subagent(role)`，
            // 否则第二次采样会重新声明主工具面（模型就能越权 spawn）。
            purpose: self.request.purpose,
            // 系统提示在 turn 创建时冻结（稳定提示 + 环境快照 + compact 摘要）。
            // 每次采样都必须原样复用：写成 None 会让 transport 现场重采集，
            // 丢摘要且同一轮看到不同环境。
            system_prompt,
            // 工具快照随轮次冻结：同一 turn 的后续采样不会看到新启动的 MCP 工具。
            tools: self.request.tools.clone(),
        }
    }

    /// 本轮是否为子 agent（禁止递归控制工具）。
    pub fn is_subagent(&self) -> bool {
        matches!(
            self.request.purpose,
            crate::chat::events::RequestPurpose::Subagent(_)
        )
    }

    /// 运行上下文摘要（诊断与测试用）。
    pub fn runtime_summary(&self) -> Option<&str> {
        self.runtime_summary.as_deref()
    }

    /// 把运行上下文摘要并入最后一次用户消息（只并入一次）。
    pub(super) fn absorb_runtime_summary(&mut self) {
        let Some(summary) = self.runtime_summary.take() else {
            return;
        };
        match self.items.last_mut() {
            Some(ModelItem::UserText(text)) => {
                if !text.is_empty() {
                    text.push_str("\n\n");
                }
                text.push_str(&summary);
            }
            Some(ModelItem::UserContent { text, images }) if images.is_empty() => {
                if !text.is_empty() {
                    text.push_str("\n\n");
                }
                text.push_str(&summary);
            }
            Some(ModelItem::UserContent { .. }) => {
                self.items.push(ModelItem::UserContent {
                    text: summary,
                    images: Vec::new(),
                });
            }
            _ => {
                // 末尾不是用户消息（理论上不会发生）：不伪造位置，直接放弃摘要。
            }
        }
    }

    /// 追加一次模型响应：可见正文（若有）按出现顺序合并为一条 assistant 文本，
    /// 随后按 ordinal 顺序追加工具调用（结构必须保留，不能拼进文本）。
    ///
    /// 参数不是 JSON 对象时跳过该调用（`ready_calls` 已保证对象，这里只做防御）。
    pub(super) fn push_response(&mut self, text: String, calls: &[ReadyToolCall]) {
        if !text.is_empty() {
            self.items.push(ModelItem::AssistantText(text));
        }
        for call in calls {
            let Ok(input) = serde_json::from_str::<serde_json::Value>(&call.arguments_json) else {
                tracing::debug!(
                    target: "lx::agent",
                    call_id = %call.call_id,
                    name = %call.name,
                    "skip tool call in current turn: arguments are not valid JSON"
                );
                continue;
            };
            self.items.push(ModelItem::AssistantToolCall {
                call_id: call.call_id.clone(),
                name: call.name.clone(),
                input,
            });
        }
    }

    /// 追加一条本地命令结果（user 角色事实，排在已闭合工具对之后）。
    ///
    /// 同时进入 `turns` 纯文本视图与 `items` 结构化视图：不支持 item 的 transport
    /// 也能读到命令事实。
    pub(super) fn push_user_context(&mut self, text: String) {
        self.items.push(ModelItem::UserText(text.clone()));
        self.request.turns.push(crate::chat::events::ChatTurn {
            role: "user".into(),
            text,
            reasoning_metadata: None,
        });
    }

    /// 追加一条工具结果（与前面的调用成对；失败结果带 `is_error`）。
    ///
    /// 回放侧单条结果上限与历史装配同一口径（[`crate::chat::context::MAX_TOOL_RESULT_REPLAY_BYTES`]）：
    /// 超限截断并显式标记，不静默省略；落库结果体本身不变。
    pub(super) fn push_tool_result(
        &mut self,
        call_id: &str,
        content: String,
        is_error: bool,
        images: Vec<crate::chat::events::ModelImage>,
    ) {
        self.items.push(ModelItem::ToolResult {
            call_id: call_id.to_string(),
            content: crate::chat::context::truncate_result_for_replay(&content),
            is_error,
            images,
        });
    }
}

/// 自动压缩摘要的系统段包裹（与手动 compact 的 `<compact_summary>` 同形）。
pub(super) fn compact_summary_prompt(summary: &str) -> String {
    format!(
        "<compact_summary>\nThis is a summary of earlier work in this subagent run that was \
         compacted automatically. Treat it as established context, not as a new request.\n\n\
         {summary}\n</compact_summary>"
    )
}
