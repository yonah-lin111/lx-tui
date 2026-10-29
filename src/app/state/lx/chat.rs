//! lx 聊天会话状态：每窗格的会话引用、请求状态机与消息展示数据。
//!
//! 仅存纯数据（DB connection/HTTP handle 不进 `AppState`）；
//! 请求状态机按 `RequestState` 约束合法状态转换。
//! 消息展示用**单一有序步骤时间线**（[`AssistantStepView`]），正文/思考/工具调用共用
//! 同一 `position` 编号空间；流事件的状态转换见 [`stream`]。

use crate::config::ai::{AiSettings, ModelSelection};

pub mod file_diff;
pub mod shell;
mod steps;
mod stream;
pub mod tool_view;

pub use file_diff::{DiffHunkView, DiffLineView, FileChangeView, FileDiffDetailView, FileDiffView};
pub use shell::{PendingShell, QueuedSubmission, UserShellView};
pub use steps::{
    AssistantStepView, ChatMessageView, ChatPartView, ToolCallView, ToolStatus, ToolSummary,
    relativize_project_path,
};
pub use tool_view::{
    AskAnsweredView, AskDetailView, CommandView, InteractionAction, PlanSnapshotView, PlanStepView,
    ProcessInteractionView, ProcessStateView, SearchDetailView, SearchMatchView,
    SubagentDetailView, ToolDetailView,
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
    /// 思考等级（variant 原始 key，如 `high`）；模型无 variants 时为 None（整段不显示）。
    pub variant: Option<String>,
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

/// 反馈提示的语义级别；决定 toast 的边框与标题色。
///
/// 提示本身已改为 toast（见 `app::update::chat::show_lx_hint`），这里只剩级别——
/// 颜色映射在 `ui::toast` 的 `ToastKind` 上，状态层不再保存提示正文与到期时刻
/// （toast 自己管这两个）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InputHintLevel {
    /// 失败：红色（danger）。
    Error,
    /// 需要留意但不算失败：黄色（warning）。
    Warning,
}

/// 可点击折叠/展开的聊天历史块。
///
/// 既回答「按下这里点的是哪一块」（松开时切换折叠），也回答「鼠标现在悬在哪一块」
/// （折叠内容提亮 / 思考块恢复品红）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CardTarget {
    /// 工具卡片（按稳定 ID 定位，与展开状态同一口径）。
    Tool {
        message_id: String,
        call_id: String,
        section: CardSection,
    },
    /// 思考块（按时间线步骤下标定位，与 [`LxChat::reasoning_expanded`] 同口径）。
    Thinking {
        message_index: usize,
        step_index: usize,
    },
    /// 本地命令卡片（按稳定消息 ID 定位；折叠状态存 detail_expanded 的 `shell` 槽）。
    Shell {
        message_id: String,
        section: CardSection,
    },
    /// compact 检查点条目（按检查点 ID 定位；默认折叠）。
    Compact { checkpoint_id: String },
    /// 子 agent 汇报块（按消息 ID + agent ID 定位；槽位 `report:<agent_id>`）。
    Report {
        message_id: String,
        agent_id: String,
        /// 点击的是 `… N more lines`：追加固定一批正文；false 时折叠整块。
        more: bool,
    },
    /// 子 agent 卡片里的 agent 链接：打开 Agents 面板并定位。
    OpenAgent { agent_id: String },
}

/// 卡片内可独立折叠的区域。
///
/// 命令卡片把「命令参数」与「命令输出」拆成两块：各自的点击面与折叠状态互不影响，
/// 点其中一块不会连带另一块。其余卡片只有一个整体（[`Self::Body`]）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CardSection {
    /// 卡片主体（文件差异 / 普通工具详情 / 命令参数）。
    Body,
    /// 命令卡片的输出区。
    Output,
}

/// 本地命令卡片在 [`LxChat::detail_expanded`] 里的槽位（它没有 call_id）。
pub const SHELL_SLOT: &str = "shell";

/// 单窗格的聊天状态：会话引用、历史视图与请求状态机。
#[derive(Debug)]
pub struct LxChat {
    /// 绑定的会话 external id；None 为尚未发送的空白草稿（不写库）。
    pub session_id: Option<String>,
    /// 当前轮选择（`/model` 改这里；下一轮生效）。
    pub selection: Option<ModelSelection>,
    /// 展示用消息列表（含流中 partial；DB 恢复时重建）。
    pub messages: Vec<ChatMessageView>,
    /// 独立持久化的撤销提示（不进入 messages / 模型上下文）。
    pub notices: Vec<super::rewind::RewindNoticeView>,
    /// 会话 revision（撤销预检与迟到结果防护）。
    pub revision: u64,
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
    /// 上一帧实际绘制的**顶部内容行号**（事件循环每次绘制前写入，渲染层只读）。
    ///
    /// 跟随时 `history_scroll` 是过期值（顶部行由 `total - height` 现算），而「切换
    /// 展开状态」必须知道**用户当前正看着哪一行**才能原地展开。把它作为跨帧记忆存
    /// 在这里，与 [`Self::suppressed_prompt_header`] 同一套做法（渲染不改状态）。
    pub viewport_top: Option<usize>,
    /// loading 转轮帧序号（按时间推进，仅流中递增）。
    pub loading_frame: usize,
    /// 鼠标悬停的可折叠块；None 表示不在任何卡片上。
    ///
    /// 渲染层只读：只有思考块用它把压暗的头部恢复品红（工具卡片与本地命令卡片的
    /// 折叠内容不随悬停变色）。
    pub hover: Option<CardTarget>,
    /// 本轮已耗时（毫秒），由事件循环在流中每帧刷新。
    ///
    /// 渲染层不取时钟（纯渲染），因此时钟只在 `advance_lx_loading` 里读一次写在这里；
    /// loading 行据此显示 `(2.7s · ↑1.6k ↓320 tok)`。
    pub turn_elapsed_ms: Option<i64>,
    /// 鼠标是否悬停在「回到底部」按钮上（决定其高亮）。
    pub jump_bottom_hover: bool,
    /// Esc 停止待命时刻：流中首次按 Esc 只待命并显示提示，窗口内再按一次才真正停止
    /// （对齐 opencode 的双击停止）；超时或流结束清除。
    pub stop_armed_at: Option<std::time::Instant>,
    /// 主轮是否被用户显式停止（Esc 双击）。
    ///
    /// 停止后常驻后台的子 agent 不撑输入框上方 loading；它们仍在底边框计数。
    /// 下一次用户发送或自动报告轮开始时清为 false。
    pub main_stopped: bool,
    /// 置顶行抑制记忆（跨帧状态，唯一必须落库到 app 侧的部分）；见 [`StickySuppression`]。
    /// 由事件循环每轮 `resolve_lx_history` 写入，渲染层只读（渲染不改状态）；
    /// "要钉哪条消息"本身是纯函数结果，渲染/命中各自现算，不存第二份真相。
    pub suppressed_prompt_header: Option<StickySuppression>,
    /// 卡片展开状态：(message_id, slot, 区域, 是否展开)。
    ///
    /// 用稳定 ID 而不是数组下标定位：历史增删、重载与切会话都不会把展开状态
    /// 误挂到别的卡片上；重启或切会话后默认折叠（本字段随窗格内存存在）。
    /// `slot` 是工具调用的 call_id（本地命令卡片固定为 [`SHELL_SLOT`]）；
    /// 命令卡片的参数与输出各占一条记录，互不影响。
    pub detail_expanded: Vec<(String, String, CardSection, bool)>,
    /// 子 agent 汇报块已追加的展开批次：每批固定行数，点 `… N more lines` 递增。
    /// 键与汇报块展开状态同口径 `(message_id, "report:<agent_id>")`。
    pub report_expand_steps: Vec<(String, String, usize)>,
    /// 键盘选中的可展开工具卡片：(message_id, call_id)；None 表示未选中。
    pub selected_tool: Option<(String, String)>,
    /// 键盘选中的 compact 检查点（没有工具卡片时 Alt+↑/↓ 的切换目标）。
    pub selected_compact: Option<String>,
    /// 当前计划快照（最近一次**成功提交**的 update_plan）；失败/中断不覆盖。
    pub plan: Option<PlanSnapshotView>,
    /// 已提交、尚未收到 spawn 回执的本地命令（按提交顺序）。
    pub pending_shells: Vec<PendingShell>,
    /// 忙时排队、空闲后按 FIFO 分派的提交（只保存在内存，重启不执行）。
    pub queue: Vec<QueuedSubmission>,
    /// 当前所属项目根目录（用于剥离工具卡片里的当前项目绝对路径前缀）。
    pub project_root: Option<std::path::PathBuf>,
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
            notices: Vec::new(),
            revision: 0,
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
            viewport_top: None,
            loading_frame: 0,
            hover: None,
            turn_elapsed_ms: None,
            jump_bottom_hover: false,
            stop_armed_at: None,
            main_stopped: false,
            suppressed_prompt_header: None,
            detail_expanded: Vec::new(),
            report_expand_steps: Vec::new(),
            selected_tool: None,
            selected_compact: None,
            plan: None,
            pending_shells: Vec::new(),
            queue: Vec::new(),
            project_root: None,
        }
    }

    /// 同步当前项目根目录，并将消息中的工具展示路径收短为相对路径。
    pub fn set_project_root(&mut self, root: Option<std::path::PathBuf>) -> bool {
        let switched_root = self.project_root.is_some() && self.project_root != root;
        let mut changed = self.project_root != root;
        self.project_root = root;
        if let Some(root) = self.project_root.as_deref() {
            for message in &mut self.messages {
                changed |= if switched_root {
                    message.rebuild_paths(root)
                } else {
                    message.relativize_paths(root)
                };
            }
        }
        changed
    }

    /// 复位**会话级**展示状态（`/new`、会话删除、切换到别的会话共用）。
    ///
    /// 三条路径的口径必须一致：`plan` 不只是展示——`start_stream` 会把它拼进本轮的
    /// system 上下文（`current plan: …`），漏清会让新会话继承上一会话的待办；
    /// `selected_tool` 残留会让 `^o` 指向已删除的消息而静默失去定位能力。
    pub fn reset_session_view(&mut self) {
        self.session_id = None;
        self.messages.clear();
        self.notices.clear();
        self.revision = 0;
        self.follow_bottom = true;
        self.input_tokens_reset();
        self.thinking_toggled.clear();
        self.turn_started_at_ms = None;
        self.turn_elapsed_ms = None;
        self.main_stopped = false;
        self.history_scroll = 0;
        self.viewport_top = None;
        self.detail_expanded.clear();
        self.report_expand_steps.clear();
        self.hover = None;
        self.selected_tool = None;
        self.selected_compact = None;
        self.plan = None;
        // 会话解绑：未执行的命令与排队项不得漂移到新会话执行。
        self.pending_shells.clear();
        self.queue.clear();
    }

    /// 是否仍有本地命令在途（已提交待 spawn 或消息仍显示运行中）。
    pub fn has_running_shell(&self) -> bool {
        !self.pending_shells.is_empty()
            || self.messages.iter().any(|message| {
                message
                    .shell
                    .as_ref()
                    .is_some_and(UserShellView::is_running)
            })
    }

    /// 仍在运行的本地命令消息 ID（停止目标）。
    pub fn running_shell_ids(&self) -> Vec<String> {
        self.messages
            .iter()
            .filter(|message| {
                message
                    .shell
                    .as_ref()
                    .is_some_and(UserShellView::is_running)
            })
            .map(|message| message.id.clone())
            .collect()
    }

    /// 按消息 ID 定位 shell 记录（可变）。
    pub fn shell_message_mut(&mut self, message_id: &str) -> Option<&mut UserShellView> {
        self.messages
            .iter_mut()
            .find(|message| message.id == message_id)
            .and_then(|message| message.shell.as_mut())
    }

    /// 按消息 ID 定位消息并标记状态（迟到的终态不得被覆盖由调用方负责比较）。
    pub fn shell_status_mut(&mut self, message_id: &str) -> Option<&mut String> {
        self.messages
            .iter_mut()
            .find(|message| message.id == message_id)
            .map(|message| &mut message.status)
    }

    /// 卡片某区域是否展开（按 `(message_id, slot, section)` 定位，默认折叠）。
    pub fn section_is_expanded(&self, message_id: &str, slot: &str, section: CardSection) -> bool {
        self.detail_expanded
            .iter()
            .find(|(message, key, part, _)| {
                message == message_id && key == slot && *part == section
            })
            .is_some_and(|(_, _, _, expanded)| *expanded)
    }

    /// 卡片主体（整卡 / 命令参数）是否展开。
    pub fn detail_is_expanded(&self, message_id: &str, slot: &str) -> bool {
        self.section_is_expanded(message_id, slot, CardSection::Body)
    }

    /// 切换卡片某区域的展开状态，返回新状态。
    pub fn toggle_section(&mut self, message_id: &str, slot: &str, section: CardSection) -> bool {
        let next = !self.section_is_expanded(message_id, slot, section);
        if let Some(existing) = self
            .detail_expanded
            .iter_mut()
            .find(|(message, key, part, _)| {
                message == message_id && key == slot && *part == section
            })
        {
            existing.3 = next;
            return next;
        }
        self.detail_expanded
            .push((message_id.to_string(), slot.to_string(), section, next));
        next
    }

    /// 切换卡片主体的展开状态，返回新状态。
    pub fn toggle_detail(&mut self, message_id: &str, slot: &str) -> bool {
        self.toggle_section(message_id, slot, CardSection::Body)
    }

    /// 汇报块已追加的展开批次数（0 表示首次展开的固定窗口）。
    pub fn report_expand_steps(&self, message_id: &str, slot: &str) -> usize {
        self.report_expand_steps
            .iter()
            .find(|(message, key, _)| message == message_id && key == slot)
            .map_or(0, |(_, _, steps)| *steps)
    }

    /// 点 `… N more lines`：汇报块再追加固定一批正文，返回新的批次数。
    pub fn expand_report_more(&mut self, message_id: &str, slot: &str) -> usize {
        if let Some(existing) = self
            .report_expand_steps
            .iter_mut()
            .find(|(message, key, _)| message == message_id && key == slot)
        {
            existing.2 = existing.2.saturating_add(1);
            return existing.2;
        }
        self.report_expand_steps
            .push((message_id.to_string(), slot.to_string(), 1));
        1
    }

    /// 收起汇报块时清空追加批次；下次展开从第一批重新开始。
    pub fn clear_report_expand(&mut self, message_id: &str, slot: &str) {
        self.report_expand_steps
            .retain(|(message, key, _)| message != message_id || key != slot);
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

    /// 流中已产生的**输出 token 估算值**（rough）：ASCII 约 4 字符 1 token，
    /// 非 ASCII（CJK 等）约 1 字符 1 token；只统计正文与思考正文。
    ///
    /// 用途仅限输入框上方 loading 行的 `↓1.2k tok` 提示——真实 usage 只在请求
    /// 结束时回来，那时 loading 行已经消失（真实值由消息底部统计行给出）。
    /// 因此这里是估算而不是统计：宁可给出「还在往外吐多少字」的活信号，也不让
    /// 用户在漫长的流里盯着一个没有数字的 `Working...`。
    pub fn stream_output_tokens(&self) -> Option<u64> {
        let message = self.streaming_message()?;
        let mut ascii = 0u64;
        let mut wide = 0u64;
        for step in &message.steps {
            let Some(part) = step.part() else {
                continue;
            };
            for ch in part.content.chars() {
                if ch.is_ascii() {
                    ascii += 1;
                } else {
                    wide += 1;
                }
            }
        }
        let tokens = ascii / 4 + wide;
        (tokens > 0).then_some(tokens)
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

    /// 当前仍在流转的 assistant 消息的只读引用。
    ///
    /// **按本轮 `assistant_id` 定位**，不能取 `messages.last()`：本地命令记录可能追加在
    /// 流式 assistant 之后，最后一条已不是模型消息。
    pub fn streaming_message(&self) -> Option<&ChatMessageView> {
        match &self.request {
            RequestState::Streaming { assistant_id, .. } => self
                .messages
                .iter()
                .find(|message| message.id == *assistant_id && message.role == "assistant"),
            _ => self
                .messages
                .last()
                .filter(|message| message.role == "assistant" && message.status == "streaming"),
        }
    }

    /// 当前仍在流转的 assistant 消息的可变引用（按 `assistant_id` 定位）。
    pub(crate) fn streaming_message_mut(&mut self) -> Option<&mut ChatMessageView> {
        let assistant_id = match &self.request {
            RequestState::Streaming { assistant_id, .. } => Some(assistant_id.clone()),
            _ => None,
        };
        match assistant_id {
            Some(assistant_id) => self
                .messages
                .iter_mut()
                .find(|message| message.id == assistant_id && message.role == "assistant"),
            None => self
                .messages
                .last_mut()
                .filter(|message| message.role == "assistant" && message.status == "streaming"),
        }
    }
}
