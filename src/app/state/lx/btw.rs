//! 旁路问答（`/btw`）的视图状态：按主 QA 隔离的线程、独立草稿与回答者集合。
//!
//! 边界：
//! - 本模块只保存 **可见缓存**（消息投影 + 编辑器草稿），不发起请求、不写库；
//! - `LxBtw::default()` 不创建任何内层编辑器，避免 `Lx -> LxBtw -> composer -> Lx`
//!   在构造期无限递归；编辑器线程只在打开/首次发问时惰性创建；
//! - 每个 QA 线程各自持有 composer，草稿、光标、撤销与 `@` 面板不跨 QA 共享。

use std::collections::BTreeMap;

use crate::app::state::Lx;

/// BTW 面板内的焦点区域。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum BtwFocus {
    /// 底部输入框。
    #[default]
    Composer,
    /// 对话阅读区。
    Conversation,
}

impl BtwFocus {
    /// `Tab` 正序轮转。
    pub const fn next(self) -> Self {
        match self {
            Self::Composer => Self::Conversation,
            Self::Conversation => Self::Composer,
        }
    }

    /// `Shift+Tab` 逆序轮转。
    pub const fn previous(self) -> Self {
        match self {
            Self::Composer => Self::Conversation,
            Self::Conversation => Self::Composer,
        }
    }
}

/// 当前线程的回答者展示项。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BtwTargetView {
    /// 稳定目标键：`default` 或 `agent_id`。
    pub key: String,
    /// 展示标签（昵称 + 角色）。
    pub label: String,
    /// 稳定实例 ID（默认助手为 None）。
    pub agent_id: Option<String>,
}

impl BtwTargetView {
    /// 默认旁路助手。
    pub fn default_assistant() -> Self {
        Self {
            key: "default".to_string(),
            label: crate::chat::prompt::AGENT_NAME.to_string(),
            agent_id: None,
        }
    }
}

/// 独立编辑缓冲：包一层空 `Lx`，只暴露文本编辑与面板能力。
///
/// **不实现 `DerefMut`**：禁止外部误写 `chat`/`task`/`rewind` 等字段；
/// 换行、撤销、提及与粘贴仍走同一套 `Lx` 方法。
#[derive(Debug)]
pub struct BtwComposer(Box<Lx>);

impl BtwComposer {
    fn new() -> Self {
        Self(Box::new(Lx::composer()))
    }

    pub fn editor(&self) -> &Lx {
        &self.0
    }

    pub fn editor_mut(&mut self) -> &mut Lx {
        &mut self.0
    }

    pub fn text(&self) -> &str {
        &self.0.input
    }

    pub fn is_empty(&self) -> bool {
        self.0.input.trim().is_empty()
    }

    pub fn clear(&mut self) {
        self.0.clear();
    }

    /// 当前文本修订号（`Lx` 每次编辑递增）；用于确认 ACK 时草稿未被改写。
    pub fn revision(&self) -> u64 {
        self.0.revision()
    }
}

/// 一个主 QA 对应的 BTW 线程视图。
#[derive(Debug)]
pub struct BtwThreadState {
    pub qa_id: String,
    /// 持久化线程 ID（首次成功提交后才有）。
    pub thread_id: Option<String>,
    /// 独立 child session（事件回投的定位键）。
    pub child_session_id: Option<String>,
    pub title: String,
    pub chat: crate::app::state::LxChat,
    pub compact: crate::app::state::LxCompact,
    pub composer: BtwComposer,
    pub targets: Vec<BtwTargetView>,
    /// 对话区滚动偏移与可见行（事件循环在绘制前写入）。
    pub scroll: usize,
    pub visible: usize,
    pub total_rows: usize,
    pub rows_dirty: bool,
    pub follow: bool,
    /// 是否已从数据库恢复过历史。
    pub loaded: bool,
    /// 提交序号：草稿 ACK 只确认同一次提交。
    pub submit_seq: u64,
    /// 正在等待本地 ACK 的提交。
    pub pending_submit: Option<(String, u64)>,
    /// 已提交但尚未开始运行的回答者数（含排队等全局槽位的）。
    ///
    /// `RecipientStarted` 到达时减一，此后该回答者由流式 assistant 消息覆盖；
    /// 这样 ACK 后到第一个回答者开始、以及回答者之间排队时 loading 都不会闪断。
    pub pending_recipients: usize,
}

impl BtwThreadState {
    fn new(qa_id: &str) -> Self {
        Self {
            qa_id: qa_id.to_string(),
            thread_id: None,
            child_session_id: None,
            title: String::new(),
            chat: crate::app::state::LxChat::new(),
            compact: crate::app::state::LxCompact::default(),
            composer: BtwComposer::new(),
            targets: Vec::new(),
            scroll: 0,
            visible: 0,
            total_rows: 0,
            rows_dirty: true,
            follow: true,
            loaded: false,
            submit_seq: 0,
            pending_submit: None,
            pending_recipients: 0,
        }
    }

    /// 输入框上沿的目标标签：`To: lx, Turing [explorer]`。
    pub fn target_label(&self) -> String {
        let labels: Vec<String> = self
            .effective_targets()
            .iter()
            .map(|target| target.label.clone())
            .collect();
        format!(
            "{}{}",
            crate::ui::text::LX_BTW_TARGETS_PREFIX,
            labels.join(", ")
        )
    }

    /// 该线程是否仍在生成/等待本地 ACK（决定底部 loading 行与动画帧）。
    pub fn is_loading(&self) -> bool {
        self.pending_submit.is_some()
            || self.pending_recipients > 0
            || self.chat.status.streaming
            || self.chat.request.is_busy()
            || self
                .chat
                .messages
                .iter()
                .any(|message| message.role == "assistant" && message.status == "streaming")
    }

    /// 当前有效回答者集合；为空时回退默认助手。
    pub fn effective_targets(&self) -> Vec<BtwTargetView> {
        if self.targets.is_empty() {
            vec![BtwTargetView::default_assistant()]
        } else {
            self.targets.clone()
        }
    }
}

/// QA 列表中的一行（由事件层从数据库回投）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BtwQaSummary {
    pub qa_id: String,
    pub title: String,
    pub has_thread: bool,
    /// 最近批次状态：`idle` / `queued` / `running` / `completed` / ...
    pub status: String,
}

/// 整个 pane 的 BTW 视图缓存。
#[derive(Debug, Default)]
pub struct LxBtw {
    threads: BTreeMap<String, BtwThreadState>,
    /// 当前主会话的正式 QA 列表（按时间正序）。
    pub qa: Vec<BtwQaSummary>,
    /// 当前显示的 QA 锚点 ID。
    pub active: Option<String>,
    pub focus: BtwFocus,
    /// `/btw prompt` 暂存正文：QA 列表到达、线程建立后立即提交一次
    /// （只认命令暂存，避免普通草稿被误发）。
    pub pending_prompt: Option<String>,
    /// `/btwList` 选择器：当前高亮的 QA 下标（`None` 表示未打开）。
    pub picking: Option<usize>,
    /// `/btwList` 面板已打开但 QA 列表尚未应用（列表到达前兼作 loading 占位；
    /// 空列表时保留空态面板）。
    pub pending_pick: bool,
    /// `/btwList` 的 QA 列表请求在途（面板显示 loading 占位）。
    pub picker_loading: bool,
}

impl LxBtw {
    /// 是否存在任何已打开的线程。
    pub fn is_empty(&self) -> bool {
        self.threads.is_empty()
    }

    /// 已缓存的线程数。
    pub fn len(&self) -> usize {
        self.threads.len()
    }

    /// 当前显示的线程。
    pub fn active_thread(&self) -> Option<&BtwThreadState> {
        self.active
            .as_deref()
            .and_then(|qa_id| self.threads.get(qa_id))
    }

    pub fn active_thread_mut(&mut self) -> Option<&mut BtwThreadState> {
        let qa_id = self.active.clone()?;
        self.threads.get_mut(&qa_id)
    }

    /// 当前显示的 composer（供输入浮层读取编辑器状态）。
    pub fn active_composer(&self) -> Option<&Lx> {
        self.active_thread().map(|thread| thread.composer.editor())
    }

    pub fn active_composer_mut(&mut self) -> Option<&mut Lx> {
        self.active_thread_mut()
            .map(|thread| thread.composer.editor_mut())
    }

    /// 取或建某 QA 的线程视图（只创建空编辑器，不读库）。
    pub fn ensure_thread(&mut self, qa_id: &str) -> &mut BtwThreadState {
        let state = self
            .threads
            .entry(qa_id.to_string())
            .or_insert_with(|| BtwThreadState::new(qa_id));
        if self.active.as_deref() != Some(qa_id) {
            self.active = Some(qa_id.to_string());
        }
        state
    }

    /// 打开 `/btwList` 选择器：默认停在当前 QA，没有则停在最后一条。
    pub fn open_picker(&mut self) -> bool {
        if self.qa.is_empty() {
            return false;
        }
        let index = self
            .active
            .as_deref()
            .and_then(|qa_id| self.qa.iter().position(|summary| summary.qa_id == qa_id))
            .unwrap_or(self.qa.len() - 1);
        self.picking = Some(index.min(self.qa.len() - 1));
        self.pending_pick = false;
        true
    }

    /// 选择器是否打开（已有列表，或列表仍在加载/为空时保留占位面板）。
    pub fn picker_open(&self) -> bool {
        self.picking.is_some() || self.pending_pick
    }

    /// 关闭选择器。
    pub fn close_picker(&mut self) -> bool {
        let was_open = self.picking.take().is_some() || self.pending_pick;
        self.pending_pick = false;
        self.picker_loading = false;
        was_open
    }

    /// 选择器上下移动（自动环绕）。
    pub fn move_picker(&mut self, delta: isize) -> bool {
        let Some(selected) = self.picking else {
            return false;
        };
        if self.qa.is_empty() {
            self.picking = None;
            return true;
        }
        let len = self.qa.len();
        let next = if delta < 0 {
            selected.checked_sub(1).unwrap_or(len - 1)
        } else {
            (selected + 1) % len
        };
        self.picking = Some(next);
        true
    }

    /// 当前高亮的 QA（供选择器渲染与确认）。
    pub fn picked_qa(&self) -> Option<&BtwQaSummary> {
        self.picking.and_then(|index| self.qa.get(index))
    }

    /// 该 QA 是否已有旁路对话：DB 标记，或本进程已加载到消息/线程。
    pub fn has_side_thread(&self, summary: &BtwQaSummary) -> bool {
        summary.has_thread
            || self.threads.get(&summary.qa_id).is_some_and(|thread| {
                thread.thread_id.is_some() || !thread.chat.messages.is_empty()
            })
    }

    /// 显式切换当前 QA。
    pub fn select_qa(&mut self, qa_id: &str) -> bool {
        if self.active.as_deref() == Some(qa_id) {
            return false;
        }
        self.ensure_thread(qa_id);
        true
    }

    /// 清空全部 BTW 视图（切主会话/`/new`/删除会话时调用）。
    pub fn clear(&mut self) {
        self.threads.clear();
        self.qa.clear();
        self.active = None;
        self.focus = BtwFocus::Composer;
        self.pending_prompt = None;
        self.picking = None;
        self.pending_pick = false;
        self.picker_loading = false;
    }

    /// 按 child session 找到线程（事件回投用）。
    pub fn thread_by_child_session(&self, child_session: &str) -> Option<&BtwThreadState> {
        self.threads
            .values()
            .find(|thread| thread.child_session_id.as_deref() == Some(child_session))
    }

    pub fn thread_by_child_session_mut(
        &mut self,
        child_session: &str,
    ) -> Option<&mut BtwThreadState> {
        self.threads
            .values_mut()
            .find(|thread| thread.child_session_id.as_deref() == Some(child_session))
    }

    /// 按线程 ID 找到对话视图。
    pub fn thread_by_id_mut(&mut self, thread_id: &str) -> Option<&mut BtwThreadState> {
        self.threads
            .values_mut()
            .find(|thread| thread.thread_id.as_deref() == Some(thread_id))
    }

    /// 全部线程（QA 列表按主 QA 顺序由外部提供，这里只读视图）。
    pub fn threads(&self) -> impl Iterator<Item = &BtwThreadState> {
        self.threads.values()
    }

    /// 是否有线程仍在生成/等待（BTW 页签的 loading 行与动画帧）。
    pub fn is_loading(&self) -> bool {
        self.threads.values().any(|thread| thread.is_loading())
    }

    /// 推进所有在跑线程的 loading 帧；返回是否有线程在跑。
    pub fn advance_loading_frames(&mut self) -> bool {
        let mut running = false;
        for thread in self.threads.values_mut() {
            // `!loaded` 也是等待状态：线程历史还在读库，底部 loading 同样要动。
            if thread.is_loading() || !thread.loaded {
                thread.chat.loading_frame = thread.chat.loading_frame.wrapping_add(1);
                running = true;
            }
        }
        running
    }

    /// 窗格尺寸变化后让所有线程重新量行。
    pub fn mark_rows_dirty(&mut self) {
        for thread in self.threads.values_mut() {
            thread.rows_dirty = true;
        }
    }
}

#[cfg(test)]
mod tests;
