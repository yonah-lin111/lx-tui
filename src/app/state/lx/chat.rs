//! lx 聊天会话状态：每窗格的会话引用、请求状态机与消息展示数据。
//!
//! 仅存纯数据（DB connection/HTTP handle 不进 `AppState`）；
//! 请求状态机按 `RequestState` 约束合法状态转换。

use crate::chat::events::PartKind;
use crate::config::ai::{AiSettings, ModelSelection};

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

/// 一个展示用内容块（消息内的有序 part）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChatPartView {
    /// `text` / `reasoning`。
    pub kind: PartKind,
    pub content: String,
    /// 供应商元数据 JSON（如 Anthropic signature）。
    pub provider_metadata_json: Option<String>,
}

impl ChatPartView {
    pub fn kind_str(&self) -> &'static str {
        self.kind.as_str()
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
    pub parts: Vec<ChatPartView>,
    /// 该轮实际模型（assistant 行）。
    pub model_label: Option<String>,
    /// 失败原因码（如 `http 503`、`timeout`）；仅 `failed` 时有值，用于历史行提示。
    pub error_code: Option<String>,
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
    /// 当前模型上下文上限（未知为 None → `--`）。
    pub context_limit: Option<u64>,
    /// 最近一次同模型成功请求的真实输入 token（未知为 None → `--`）。
    pub input_tokens: Option<u64>,
    /// 上下文占用百分比（真实输入 token / 上限，夹到 0..=100）；两者任一未知为 None。
    pub context_percent: Option<u8>,
    /// 是否正在生成（显示停止控件）。
    pub streaming: bool,
    /// 上下文裁剪省略提示（一次性）。
    pub omitted_turns: Option<usize>,
}

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
    /// 该窗格最近一次**同模型**成功请求的真实输入 token；None 显示 `--`。
    pub last_input_tokens: Option<u64>,
    /// 该输入 token 对应模型（切换模型时分子清空）。
    pub last_tokens_model: Option<String>,
    /// 上下文裁剪省略提示（一次性显示；读取后清除）。
    pub omitted_turns_notice: Option<usize>,
    /// 窗格内请求代号计数器（单调递增）。
    pub generation_counter: u64,
    /// 状态行展示数据。
    pub status: ChatStatus,
    /// 已展开的 thinking 块（消息索引, part 索引）。
    pub expanded_thinking: Vec<(usize, usize)>,
    /// 历史视图滚动偏移（跟随底部时忽略）。
    pub history_scroll: usize,
    /// loading 转轮帧序号（按时间推进，仅流中递增）。
    pub loading_frame: usize,
    /// 鼠标是否悬停在「回到底部」按钮上（决定其高亮）。
    pub jump_bottom_hover: bool,
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
            last_input_tokens: None,
            last_tokens_model: None,
            omitted_turns_notice: None,
            generation_counter: 0,
            status: ChatStatus::default(),
            expanded_thinking: Vec::new(),
            history_scroll: 0,
            loading_frame: 0,
            jump_bottom_hover: false,
        }
    }

    /// thinking 块是否展开。
    pub fn reasoning_expanded(&self, message_index: usize, part_index: usize) -> bool {
        self.expanded_thinking
            .contains(&(message_index, part_index))
    }

    /// 切换 thinking 块展开状态。
    pub fn toggle_reasoning(&mut self, message_index: usize, part_index: usize) {
        let key = (message_index, part_index);
        if let Some(position) = self.expanded_thinking.iter().position(|item| *item == key) {
            self.expanded_thinking.remove(position);
        } else {
            self.expanded_thinking.push(key);
        }
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
        let context_percent = context_percent(self.last_input_tokens, context_limit);
        self.status = ChatStatus {
            model_name,
            provider_name,
            context_limit,
            input_tokens: self.last_input_tokens,
            context_percent,
            streaming: self.request.is_busy(),
            omitted_turns: self.omitted_turns_notice.take(),
        };
    }

    /// 清空输入 token 分子（切模型/新建会话）。
    pub fn input_tokens_reset(&mut self) {
        self.last_input_tokens = None;
        self.last_tokens_model = None;
    }

    /// 当前选择；无窗格级选择时回落配置默认。
    pub fn effective_selection(&self, models: &ModelsSnapshot) -> Option<ModelSelection> {
        // 窗格选择若已失效（provider/model 被删）回落默认。
        if let Some(selection) = &self.selection
            && let Some(provider) = models.settings.providers.get(&selection.provider)
            && provider.models.contains_key(&selection.model)
        {
            let mut resolved = selection.clone();
            // variant 失效回落模型默认 variant。
            if let Some(variant) = &resolved.variant
                && !provider
                    .models
                    .get(&selection.model)
                    .is_some_and(|model| model.variants.contains_key(variant))
            {
                resolved.variant = provider
                    .models
                    .get(&selection.model)
                    .and_then(|model| model.variant.clone());
            }
            return Some(resolved);
        }
        models.default_selection()
    }

    /// 追加/合并一个流式 delta part 到最后一条 assistant 消息。
    ///
    /// 相邻同 kind 且元数据兼容的 delta 合并；类型切换或带元数据的空 delta
    /// 新建 part，保持 text/reasoning 交错顺序。
    pub fn append_stream_delta(
        &mut self,
        kind: PartKind,
        text: &str,
        metadata: Option<serde_json::Value>,
    ) {
        let Some(message) = self
            .messages
            .last_mut()
            .filter(|message| message.role == "assistant" && message.status == "streaming")
        else {
            return;
        };
        // 签名元数据（text 为空）绑定到同 kind 的相邻 part。
        if let Some(meta) = metadata.as_ref()
            && text.is_empty()
            && let Some(last) = message.parts.last_mut()
            && last.kind == kind
            && last.provider_metadata_json.is_none()
        {
            last.provider_metadata_json = Some(serde_json::to_string(meta).unwrap_or_default());
            return;
        }
        if let Some(last) = message.parts.last_mut()
            && last.kind == kind
            && metadata.is_none()
            && last.provider_metadata_json.is_none()
        {
            last.content.push_str(text);
            return;
        }
        message.parts.push(ChatPartView {
            kind,
            content: text.to_string(),
            provider_metadata_json: metadata
                .map(|meta| serde_json::to_string(&meta).unwrap_or_default()),
        });
    }
}

/// 上下文占用百分比：真实输入 token / 上限，夹到 `0..=100`；两者任一未知为 None。
pub fn context_percent(input_tokens: Option<u64>, context_limit: Option<u64>) -> Option<u8> {
    let input = input_tokens?;
    let limit = context_limit.filter(|limit| *limit > 0)?;
    let percent = input.saturating_mul(100) / limit;
    Some(percent.min(100) as u8)
}
