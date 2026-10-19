//! lx 聊天会话状态：每窗格的会话引用、请求状态机与消息展示数据。
//!
//! 仅存纯数据（DB connection/HTTP handle 不进 `AppState`）；
//! 请求状态机按 `RequestState` 约束合法状态转换。
//! 消息展示用**单一有序步骤时间线**（[`AssistantStepView`]），正文/思考/工具调用共用
//! 同一 `position` 编号空间；流事件的状态转换见 [`stream`]。

use crate::config::ai::{AiSettings, ModelSelection};

mod steps;
mod stream;

pub use steps::{
    AssistantStepView, ChatMessageView, ChatPartView, ToolCallView, ToolStatus, ToolSummary,
};

/// 模型配置快照：设置页保存后由事件层更新；发送时冻结成 `FrozenModel`。
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ModelsSnapshot {
    /// 解析后的 ai 配置（含全部供应商与模型）。
    pub settings: AiSettings,
}

impl ModelsSnapshot {
    /// 是否存在可发送的有效默认模型（provider 启用、模型存在、有 Key）。
    pub fn default_selection(&self) -> Option<ModelSelection> {
        let selection = self.settings.default_model.as_ref()?;
        let provider = self.settings.providers.get(&selection.provider)?;
        if !self
            .settings
            .enabled_providers
            .contains(&selection.provider)
        {
            return None;
        }
        let model = provider.models.get(&selection.model)?;
        let has_key = provider
            .options
            .api_key
            .as_deref()
            .is_some_and(|key| !key.is_empty());
        if !has_key {
            return None;
        }
        // 选择的 variant 必须存在（或无 variants 直接用默认）。
        let variant_ok = selection
            .variant
            .as_ref()
            .is_none_or(|variant| model.variants.contains_key(variant));
        variant_ok.then(|| selection.clone())
    }

    /// 该选择对应模型的有效 transport（模型级覆盖优先）。
    pub fn transport_of(&self, selection: &ModelSelection) -> Option<String> {
        let provider = self.settings.providers.get(&selection.provider)?;
        let model = provider.models.get(&selection.model)?;
        Some(
            model
                .transport
                .map(|transport| transport.as_str())
                .unwrap_or_else(|| provider.kind.as_str())
                .to_string(),
        )
    }

    /// 该选择对应模型的 base URL 与 Key。
    pub fn endpoint_of(&self, selection: &ModelSelection) -> Option<(String, String)> {
        let provider = self.settings.providers.get(&selection.provider)?;
        let key = provider
            .options
            .api_key
            .clone()
            .filter(|key| !key.is_empty())?;
        Some((provider.options.base_url.clone(), key))
    }

    /// 该选择对应模型的上下文/输出上限。
    pub fn limits_of(&self, selection: &ModelSelection) -> (Option<u64>, Option<u64>) {
        let Some(model) = self
            .settings
            .providers
            .get(&selection.provider)
            .and_then(|provider| provider.models.get(&selection.model))
        else {
            return (None, None);
        };
        match &model.limit {
            Some(limit) => (Some(limit.context), Some(limit.output)),
            None => (None, None),
        }
    }

    /// 模型展示名：配置里的 `name`，缺失回退模型 id（历史行统计用）。
    pub fn model_display_name(&self, provider_key: &str, model_id: &str) -> String {
        self.settings
            .providers
            .get(provider_key)
            .and_then(|provider| provider.models.get(model_id))
            .map(|model| model.name.clone())
            .filter(|name| !name.is_empty())
            .unwrap_or_else(|| model_id.to_string())
    }

    /// 该选择对应模型的思考等级参数对象（variants 的 value）。
    pub fn variant_params_of(&self, selection: &ModelSelection) -> Option<serde_json::Value> {
        let variant = selection.variant.as_ref()?;
        let model = self
            .settings
            .providers
            .get(&selection.provider)?
            .models
            .get(&selection.model)?;
        model.variants.get(variant).cloned()
    }

    /// 规范化选择：显式 `variant` 失效或缺失时回落模型配置里的默认思考等级
    /// （模型 `variant` 字段），使「配了默认等级」的模型开箱即用（如 deepseek 思考）。
    pub fn normalize_selection(&self, mut selection: ModelSelection) -> ModelSelection {
        let Some(model) = self
            .settings
            .providers
            .get(&selection.provider)
            .and_then(|provider| provider.models.get(&selection.model))
        else {
            return selection;
        };
        let valid = selection
            .variant
            .as_ref()
            .is_some_and(|variant| model.variants.contains_key(variant));
        if !valid {
            selection.variant = model.variant.clone();
        }
        selection
    }

    /// 流式空闲超时。
    pub fn idle_timeout_ms(&self) -> u64 {
        self.settings.idle_timeout_ms()
    }
}

/// 一轮请求的归属键：窗格 + 会话 + 代次。
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct RequestKey {
    pub pane: crate::layout::PaneId,
    pub session: String,
    pub generation: u64,
}

/// 发送时冻结的模型快照（写入该轮 assistant message）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FrozenModel {
    pub provider: String,
    pub model: String,
    pub variant: Option<String>,
    /// 有效 transport（模型级覆盖优先）。
    pub transport: String,
    /// base URL。
    pub base_url: String,
    /// API Key（只在内存与冻结快照中，不落库）。
    pub api_key: String,
    /// 上下文/输出上限。
    pub context_limit: Option<u64>,
    pub output_limit: Option<u64>,
    /// 思考等级参数对象。
    pub variant_params: Option<serde_json::Value>,
    /// 流式空闲超时毫秒。
    pub idle_timeout_ms: u64,
}

impl FrozenModel {
    /// 从快照与选择构造发送快照；provider/模型无效或缺 Key 返回 None。
    pub fn resolve(models: &ModelsSnapshot, selection: &ModelSelection) -> Option<Self> {
        let transport = models.transport_of(selection)?;
        let (base_url, api_key) = models.endpoint_of(selection)?;
        let (context_limit, output_limit) = models.limits_of(selection);
        Some(Self {
            provider: selection.provider.clone(),
            model: selection.model.clone(),
            variant: selection.variant.clone(),
            transport,
            base_url,
            api_key,
            context_limit,
            output_limit,
            variant_params: models.variant_params_of(selection),
            idle_timeout_ms: models.idle_timeout_ms(),
        })
    }
}

/// 请求状态机：Idle → Committing → Streaming → Finishing → Idle。
#[derive(Debug, Clone, PartialEq)]
pub enum RequestState {
    Idle,
    /// 草稿已冻结、DB 写入在途。
    Committing {
        generation: u64,
        draft: String,
    },
    /// 网络流在途；`assistant_id` 为该轮 checkpoint/finish 的 DB 目标。
    Streaming {
        key: RequestKey,
        assistant_id: String,
    },
    /// 终态 DB 写入在途。
    Finishing {
        key: RequestKey,
        reason: crate::chat::events::StopReason,
    },
}

impl RequestState {
    /// 是否允许新一轮发送（同窗格同时只有一轮）。
    pub fn can_send(&self) -> bool {
        matches!(self, Self::Idle)
    }

    /// 是否正在生成（含提交/流式/收尾；Enter 不重复发送）。
    pub fn is_busy(&self) -> bool {
        !matches!(self, Self::Idle)
    }

    /// 在途请求的 session 与 generation（取消/删除竞争用）。
    pub fn active(&self) -> Option<(&str, u64)> {
        match self {
            Self::Committing { .. } => None,
            Self::Streaming { key, .. } | Self::Finishing { key, .. } => {
                Some((key.session.as_str(), key.generation))
            }
            _ => None,
        }
    }
}

/// 状态行展示数据：由 app 层按当前选择/usage 刷新，`ui/` 只读。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ChatStatus {
    /// 模型展示名（模型 `name`，缺失回退模型 id）。
    pub model_name: Option<String>,
    /// 供应商展示名。
    pub provider_name: Option<String>,
    /// 当前模型上下文上限（未知为 None → 不显示上下文占用）。
    pub context_limit: Option<u64>,
    /// 上下文占用分子：最近一次同模型请求的输入 + 输出 token（未知为 None）。
    pub input_tokens: Option<u64>,
    /// 是否正在生成（决定 loading 行与 Esc 停止待命是否生效）。
    pub streaming: bool,
    /// 上下文裁剪省略提示（一次性）。
    pub omitted_turns: Option<usize>,
}

/// 置顶用户消息行的抑制记忆（对齐 codex `suppressed_prompt_header`）。
///
/// 非跟随时，若"视口 + 顶行位置"与记忆一致，则整帧跳过置顶行重判——避免
/// "让位一行 → 重判不再需要 → 撤销让位" 的往返抖动。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StickySuppression {
    /// 未让位时的历史整区（位置或尺寸一变记忆即失效）。
    pub viewport: ratatui::layout::Rect,
    /// 顶行所属消息索引。
    pub message: usize,
    /// 该消息内的行偏移。
    pub row: usize,
}

/// 输入框上方那一行的**临时提示**（loading 之外的第二类内容）。
///
/// lx 页所有反馈都收口到输入框上方：`Working...`（loading）、`↓ jump to bottom`（导航）
/// 与本提示共用一个位置。提示到点自动消失，超宽时渲染层用 `…` 截断。
///
/// 文案要求**十分简洁**（见 `ui::text::chat` 里的常量），不要写整句说明。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InputHint {
    /// 提示正文（不含图标与颜色，渲染层只负责排版）。
    pub message: String,
    /// 语义级别：决定颜色（错误红 / 默认黄）。
    pub level: InputHintLevel,
    /// 到期时刻；`tick` 到点清除。
    pub deadline: std::time::Instant,
}

/// 输入框上方提示的两个语义级别（用户指定：错误红、其余黄）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InputHintLevel {
    /// 失败：红色。
    Error,
    /// 需要留意但不算失败：黄色（默认档）。
    Warning,
}

impl InputHint {
    /// 以当前时刻起算的 [`INPUT_HINT_TTL`]。
    pub fn new(level: InputHintLevel, message: impl Into<String>, now: std::time::Instant) -> Self {
        Self {
            message: message.into(),
            level,
            deadline: now + INPUT_HINT_TTL,
        }
    }

    /// 是否已到期。
    pub fn is_expired(&self, now: std::time::Instant) -> bool {
        now >= self.deadline
    }
}

/// 输入框上方提示的展示时长（与 toast 一致，2 秒）。
pub const INPUT_HINT_TTL: std::time::Duration = std::time::Duration::from_millis(2000);

/// 单窗格的聊天状态：会话引用、历史视图与请求状态机。
#[derive(Debug)]
pub struct LxChat {
    /// 绑定的会话 external id；None 为尚未发送的空白草稿（不写库）。
    pub session_id: Option<String>,
    /// 当前轮选择（`/model` 改这里；下一轮生效）。
    pub selection: Option<ModelSelection>,
    /// 展示用消息列表（含流中 partial；DB 恢复时重建）。
    pub messages: Vec<ChatMessageView>,
    /// 请求状态机。
    pub request: RequestState,
    /// 历史视图滚动：是否跟随底部（用户上滚后暂停）。
    pub follow_bottom: bool,
    /// 上下文占用分子：最近一次**同模型**请求的输入 + 输出 token
    /// （codex 口径 `total_tokens`——下一轮请求的输入近似为其上一轮输入加输出）；
    /// None 表示未知（状态行整段隐藏上下文占用）。
    pub context_tokens: Option<u64>,
    /// 该占用分子对应模型（切换模型时分子清空）。
    pub last_tokens_model: Option<String>,
    /// 上下文裁剪省略提示（一次性显示；读取后清除）。
    pub omitted_turns_notice: Option<usize>,
    /// 窗格内请求代号计数器（单调递增）。
    pub generation_counter: u64,
    /// 状态行展示数据。
    pub status: ChatStatus,
    /// 用户显式切换过的 thinking 块（消息索引, part 索引, 是否展开）；覆盖流中自动展开。
    pub thinking_toggled: Vec<(usize, usize, bool)>,
    /// 当前轮开始时刻（毫秒；来自 `begin_turn` 落库时间），用于在线计算本轮耗时。
    pub turn_started_at_ms: Option<i64>,
    /// 历史视图滚动偏移（跟随底部时忽略）。
    pub history_scroll: usize,
    /// loading 转轮帧序号（按时间推进，仅流中递增）。
    pub loading_frame: usize,
    /// 鼠标是否悬停在「回到底部」按钮上（决定其高亮）。
    pub jump_bottom_hover: bool,
    /// 鼠标悬停的代码块复制按钮 `(消息索引, 步骤索引, 该步骤内的块序号)`；
    /// 用于按钮高亮 + tooltip（一个正文步骤里可能有多个代码块，必须精确到块）。
    pub code_copy_hover: Option<(usize, usize, usize)>,
    /// Esc 停止待命时刻：流中首次按 Esc 只待命并显示提示，窗口内再按一次才真正停止
    /// （对齐 opencode 的双击停止）；超时或流结束清除。
    pub stop_armed_at: Option<std::time::Instant>,
    /// 输入框上方的临时提示（错误红 / 默认黄），到点自动消失。
    pub hint: Option<InputHint>,
    /// 置顶行抑制记忆（跨帧状态，唯一必须落库到 app 侧的部分）；见 [`StickySuppression`]。
    /// 由事件循环每轮 `resolve_lx_sticky_prompt` 写入，渲染层只读（渲染不改状态）；
    /// "要钉哪条消息"本身是纯函数结果，渲染/命中各自现算，不存第二份真相。
    pub suppressed_prompt_header: Option<StickySuppression>,
}

impl Default for LxChat {
    fn default() -> Self {
        Self::new()
    }
}

impl LxChat {
    pub fn new() -> Self {
        Self {
            session_id: None,
            selection: None,
            messages: Vec::new(),
            request: RequestState::Idle,
            follow_bottom: true,
            context_tokens: None,
            last_tokens_model: None,
            omitted_turns_notice: None,
            generation_counter: 0,
            status: ChatStatus::default(),
            thinking_toggled: Vec::new(),
            turn_started_at_ms: None,
            history_scroll: 0,
            loading_frame: 0,
            jump_bottom_hover: false,
            code_copy_hover: None,
            stop_armed_at: None,
            hint: None,
            suppressed_prompt_header: None,
        }
    }

    /// thinking 块是否展开：**默认折叠**（生成中也不自动展开），只有用户显式点击头部或
    /// `Ctrl+T` 才展开；状态一直保留——生成结束不会强制展开或强制收起（对齐 opencode 的
    /// hide 模式：单击展开、再点收起）。
    ///
    /// `step_index` 是**时间线步骤下标**（含工具步骤），与 UI 命中几何同源。
    pub fn reasoning_expanded(&self, message_index: usize, step_index: usize) -> bool {
        self.thinking_toggled
            .iter()
            .find(|(message, step, _)| *message == message_index && *step == step_index)
            .is_some_and(|(_, _, expanded)| *expanded)
    }

    /// 该思考块是否仍在生成：该步骤是 Reasoning、其独立完成字段为空、且消息仍在流中。
    ///
    /// **不再**要求它是最后一个 reasoning 块或所属消息是最后一条：思考块自己的完成
    /// 字段才是唯一依据（`reasoning -> text -> reasoning` 交错时同样正确）。
    pub fn reasoning_streaming(&self, message_index: usize, step_index: usize) -> bool {
        let Some(message) = self.messages.get(message_index) else {
            return false;
        };
        if message.status != "streaming" {
            return false;
        }
        matches!(
            message.step_at(step_index),
            Some(AssistantStepView::Part { part, .. }) if part.reasoning_open()
        )
    }

    /// 切换 thinking 块展开状态（记录显式选择，覆盖流中自动展开）。
    ///
    /// `step_index` 为时间线步骤下标（与 [`Self::reasoning_expanded`] 同口径）。
    pub fn toggle_reasoning(&mut self, message_index: usize, step_index: usize) {
        let next = !self.reasoning_expanded(message_index, step_index);
        if let Some(existing) = self
            .thinking_toggled
            .iter_mut()
            .find(|(message, step, _)| *message == message_index && *step == step_index)
        {
            existing.2 = next;
            return;
        }
        self.thinking_toggled
            .push((message_index, step_index, next));
    }

    /// 重算状态行展示数据：模型/供应商展示名、上限、用量与上下文百分比来自当前选择
    /// （或默认）；输入 token 只在仍属同一模型时保留（切模型清空分子）。
    pub fn refresh_status(&mut self, models: &ModelsSnapshot) {
        let selection = self.effective_selection(models);
        let (model_name, provider_name, context_limit) = match &selection {
            Some(selection) => {
                let provider = models.settings.providers.get(&selection.provider);
                let model = provider.and_then(|provider| provider.models.get(&selection.model));
                let provider_name = provider.map(|provider| provider.name.clone());
                let model_name = model
                    .map(|model| model.name.clone())
                    .or_else(|| Some(selection.model.clone()));
                let limit = model.and_then(|model| model.limit.map(|limit| limit.context));
                (model_name, provider_name, limit)
            }
            None => (None, None, None),
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
            context_limit,
            input_tokens: self.context_tokens,
            streaming: self.request.is_busy(),
            omitted_turns: self.omitted_turns_notice.take(),
        };
    }

    /// 清空上下文占用分子（切模型/新建会话）。
    pub fn input_tokens_reset(&mut self) {
        self.context_tokens = None;
        self.last_tokens_model = None;
    }

    /// 当前选择；无窗格级选择时回落配置默认。
    ///
    /// 统一规范化：显式 variant 失效或缺失时回落模型默认思考等级（`normalize_selection`），
    /// 否则「只配了模型默认等级」的模型永远不会带上思考参数。
    pub fn effective_selection(&self, models: &ModelsSnapshot) -> Option<ModelSelection> {
        // 窗格选择若已失效（provider/model 被删）回落默认。
        if let Some(selection) = &self.selection
            && models
                .settings
                .providers
                .get(&selection.provider)
                .is_some_and(|provider| provider.models.contains_key(&selection.model))
        {
            return Some(models.normalize_selection(selection.clone()));
        }
        models
            .default_selection()
            .map(|selection| models.normalize_selection(selection))
    }

    /// 当前仍在流转的 assistant 消息（最后一条且 `streaming`）的可变引用。
    pub(crate) fn streaming_message_mut(&mut self) -> Option<&mut ChatMessageView> {
        self.messages
            .last_mut()
            .filter(|message| message.role == "assistant" && message.status == "streaming")
    }
}
