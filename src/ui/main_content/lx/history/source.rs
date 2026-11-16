//! 历史渲染的只读数据源。
//!
//! 主窗格从 `&Lx` 构造，子 agent 详情从它自己的 `LxChat` + `LxCompact` 构造。
//! 渲染层只读：不克隆 `LxChat`、不访问 DB、不做 IO。

use crate::app::state::RewindNoticeView;
use crate::app::state::lx::LxRecap;
use crate::app::state::{
    ChatMessageView, CompactNoticeView, Lx, LxChat, LxCompact, LxModelSwitch,
    ModelSwitchNoticeView, SubagentRun,
};

/// 离屏流式合并判定用的视口锚点。
#[derive(Debug, Clone, Copy)]
pub(crate) struct HistoryViewport {
    /// 当前视口顶部行（绝对行号）。
    pub scroll: usize,
    /// 是否贴底跟随；跟随时禁用合并，保证最新内容立即出现。
    pub follow_bottom: bool,
}

/// 历史渲染的会话面板类型：用户消息底色按面板区分。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum HistorySurface {
    /// 主 lx 会话。
    Main,
    /// Agents 详情的子 agent 会话。
    Subagent,
    /// BTW 旁路线程。
    Btw,
}

/// 历史渲染的只读数据源。
#[derive(Debug, Clone, Copy)]
pub(crate) struct HistorySource<'a> {
    /// 消息时间线。
    pub messages: &'a [ChatMessageView],
    /// 独立撤销提示。
    pub notices: &'a [RewindNoticeView],
    /// compact 检查点。
    pub checkpoints: &'a [CompactNoticeView],
    /// 模型切换提示。
    pub switches: &'a [ModelSwitchNoticeView],
    /// 折叠状态 / loading 相位。
    pub chat: &'a LxChat,
    /// 呼吸动画是否推进：`ask` 等待用户作答时冻结（方块保持静态实心）。
    pub animating: bool,
    /// compact 状态。
    pub compact: &'a LxCompact,
    /// 模型切换状态。
    pub model_switch: &'a LxModelSwitch,
    /// recap 状态；子 agent 详情没有独立 recap，使用 None。
    pub recap: Option<&'a LxRecap>,
    /// 撤销面板当前选中的用户消息（渲染高亮用；无则 None）。
    pub selected_user_message: Option<&'a str>,
    /// 本会话仍有子 agent 在跑：统计行是任务收尾信息，必须等它们全部结束。
    pub subagents_running: bool,
    /// 是否允许「滚离底部时屏幕外流式合并」。
    ///
    /// 主聊天从 `LxChat` 取视口；子 agent 详情通过 [`HistorySource::with_viewport`]
    /// 传入 `SubagentRun` 的滚动状态。命中/取词等只读路径不参与合并。
    pub coalesce_offscreen: bool,
    /// 离屏合并的视口锚点；`None` = 不参与合并。
    pub viewport: Option<HistoryViewport>,
    /// 面板类型：决定用户消息底色（主会话 / 子 agent / BTW）。
    pub surface: HistorySurface,
    /// 稳定缓存身份：同一数据源跨帧不变；切会话/切 run 时改变。
    ///
    /// 主历史用 session id（无 id 的草稿退回 `LxChat` 地址）；子 agent 详情用
    /// `agent_id + child_session_id`。不能用 `LxChat` 地址做详情身份：run 列表
    /// 排序会重建 `Vec`，所有 run 的内存地址都会改变。
    owner: usize,
}

impl<'a> From<&'a Lx> for HistorySource<'a> {
    fn from(lx: &'a Lx) -> Self {
        Self {
            messages: &lx.chat.messages,
            notices: &lx.chat.notices,
            checkpoints: &lx.compact.checkpoints,
            switches: &lx.model_switch.switches,
            chat: &lx.chat,
            // ask 等待用户作答期间不推进呼吸相位：步骤方块保持静态实心，
            // 不再一闪一闪（用户思考的时间不属于 Agent 执行过程）。
            animating: (lx.chat.is_animating() || lx.compact.is_running()) && lx.ask.is_none(),
            compact: &lx.compact,
            model_switch: &lx.model_switch,
            recap: Some(&lx.recap),
            selected_user_message: lx
                .rewind
                .selected
                .as_ref()
                .map(|target| target.user_message_id.as_str()),
            subagents_running: lx.agents.runs.iter().any(|run| run.is_running()),
            coalesce_offscreen: true,
            viewport: Some(HistoryViewport {
                scroll: lx.chat.history_scroll,
                follow_bottom: lx.chat.follow_bottom,
            }),
            surface: HistorySurface::Main,
            owner: lx
                .chat
                .session_id
                .as_deref()
                .map(|id| stable_owner("session", id, ""))
                // 未绑定会话的草稿：同一窗格内地址稳定；切到已绑定会话时 owner 改变。
                .unwrap_or(&lx.chat as *const LxChat as usize),
        }
    }
}

impl<'a> HistorySource<'a> {
    /// 数据源身份：主历史与某个子 agent 详情的 `LxChat` 地址不同。
    ///
    /// `PaneCache` 用它阻止同宽度下的跨页快照复用。
    pub(crate) fn owner(&self) -> usize {
        self.owner
    }

    /// 子 agent 详情的数据源：owner 用稳定 ID，run 列表排序不失效。
    pub(crate) fn from_run(run: &'a SubagentRun) -> Self {
        let mut source = Self::from_chat(&run.chat, &run.compact);
        source.owner = stable_owner("subagent", &run.agent_id, &run.child_session_id);
        source.surface = HistorySurface::Subagent;
        source
    }

    /// 只有一份 `LxChat` 的数据源（`Agents` 详情区）。
    pub(crate) fn from_chat(chat: &'a LxChat, compact: &'a LxCompact) -> HistorySource<'a> {
        static EMPTY_MODEL_SWITCH: LxModelSwitch = LxModelSwitch::EMPTY;
        HistorySource {
            messages: &chat.messages,
            notices: &chat.notices,
            checkpoints: &compact.checkpoints,
            switches: &[],
            chat,
            // 与主历史同一判据：BTW 的在途回答只体现在消息状态上，没有
            // `status.streaming`；漏掉它会让详情步骤标记整段不呼吸。
            animating: chat.is_animating() || compact.is_running(),
            compact,
            model_switch: &EMPTY_MODEL_SWITCH,
            recap: None,
            selected_user_message: None,
            subagents_running: false,
            // 详情区默认不参与离屏合并；滚动路径用 `with_viewport` 显式传入。
            coalesce_offscreen: false,
            viewport: None,
            surface: HistorySurface::Main,
            // 无 run 身份的调用方（测试/只读几何）退回地址；生产详情走 `from_run`。
            owner: chat as *const LxChat as usize,
        }
    }

    /// 指定面板类型（BTW 详情用）。
    pub(crate) fn with_surface(mut self, surface: HistorySurface) -> Self {
        self.surface = surface;
        self
    }

    /// 给数据源绑定真实的滚动视口（Agents 详情用 `SubagentRun` 的滚动状态）。
    pub(crate) fn with_viewport(mut self, viewport: HistoryViewport) -> Self {
        self.viewport = Some(viewport);
        self.coalesce_offscreen = true;
        self
    }

    /// 是否有可绘制的历史。
    pub(crate) fn has_history(&self) -> bool {
        self.messages
            .iter()
            .enumerate()
            .any(|(index, _)| !is_legacy_report_row(self.messages, index))
            || !self.notices.is_empty()
            || !self.compact.is_empty()
            || self.compact.is_running()
            || !self.model_switch.is_empty()
            || self.recap.is_some_and(|recap| !recap.is_empty())
    }
}

/// 旧版 wait 消费后留下的报告 user 消息：同轮没有 assistant，只是历史残留。
///
/// 新版 wait 报告只进工具结果，不再产生消息；后台报告批次则一定有同轮 assistant。
/// UI 隐藏这类消息，但不删除数据库行。
pub(crate) fn is_legacy_report_row(messages: &[ChatMessageView], index: usize) -> bool {
    let Some(message) = messages.get(index) else {
        return false;
    };
    if message.role != "user" || message.reports.is_empty() {
        return false;
    }
    !messages
        .iter()
        .any(|other| other.turn_index == message.turn_index && other.role == "assistant")
}

/// 稳定 owner 指纹：同一逻辑数据源跨帧一致，不依赖对象地址。
fn stable_owner(kind: &str, first: &str, second: &str) -> usize {
    const OFFSET_BASIS: u64 = 0xcbf2_9ce4_8422_2325;
    const PRIME: u64 = 0x0000_0100_0000_01b3;
    let mut hash = OFFSET_BASIS;
    for bytes in [kind.as_bytes(), first.as_bytes(), second.as_bytes()] {
        for &byte in bytes {
            hash ^= u64::from(byte);
            hash = hash.wrapping_mul(PRIME);
        }
        // 分隔字段，避免 ("ab","c") 与 ("a","bc") 碰撞。
        hash ^= 0xff;
        hash = hash.wrapping_mul(PRIME);
    }
    hash as usize
}
