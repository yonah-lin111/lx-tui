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
    /// QA 列表。
    QaList,
    /// 对话阅读区。
    Conversation,
}

impl BtwFocus {
    /// `Tab` 正序轮转。
    pub const fn next(self) -> Self {
        match self {
            Self::Composer => Self::QaList,
            Self::QaList => Self::Conversation,
            Self::Conversation => Self::Composer,
        }
    }

    /// `Shift+Tab` 逆序轮转。
    pub const fn previous(self) -> Self {
        match self {
            Self::Composer => Self::Conversation,
            Self::Conversation => Self::QaList,
            Self::QaList => Self::Composer,
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
        }
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
    /// QA 列表的窗口起点（超过可见行时滚动）。
    pub qa_window: usize,
    /// `/btw prompt` 暂存：QA 列表到达后立即提交一次（避免普通草稿被误发）。
    pub pending_prompt: bool,
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
        self.qa_window = 0;
        self.pending_prompt = false;
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

    /// 窗格尺寸变化后让所有线程重新量行。
    pub fn mark_rows_dirty(&mut self) {
        for thread in self.threads.values_mut() {
            thread.rows_dirty = true;
        }
    }
}

#[cfg(test)]
mod tests;
