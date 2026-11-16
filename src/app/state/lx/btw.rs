//! 旁路问答（`/btw`）的视图状态：按主 QA 隔离的线程、独立草稿与回答者集合。
//!
//! 边界：
//! - 本模块只保存 **可见缓存**（消息投影 + 编辑器草稿），不发起请求、不写库；
//! - `LxBtw::default()` 不创建任何内层编辑器，避免 `Lx -> LxBtw -> composer -> Lx`
//!   在构造期无限递归；编辑器线程只在打开/首次发问时惰性创建；
//! - 每个 QA 线程各自持有 composer，草稿、光标、撤销与 `@` 面板不跨 QA 共享。

use std::collections::BTreeMap;

use crate::app::overlay::TextInput;
use crate::app::state::Lx;
use crate::app::state::lx::composer::DetailComposer;
use crate::app::state::lx::slash::PanelWindow;

/// 当前线程的回答者。
///
/// composer 已不再展示目标段，这里只保留回查 subagent 实例所需的稳定 ID。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BtwTargetView {
    /// 稳定实例 ID（默认助手为 None）。
    pub agent_id: Option<String>,
}

impl BtwTargetView {
    /// 默认旁路助手。
    pub fn default_assistant() -> Self {
        Self { agent_id: None }
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
    pub composer: DetailComposer,
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
            composer: DetailComposer::new_btw(),
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

    /// 线程被删除（`/rewind` 删空或 `/new`）：回到空白线程视图，取消所有在途状态。
    pub fn reset_after_delete(&mut self) {
        self.thread_id = None;
        self.child_session_id = None;
        self.title.clear();
        self.chat = crate::app::state::LxChat::new();
        self.compact.reset();
        self.targets.clear();
        self.scroll = 0;
        self.total_rows = 0;
        self.rows_dirty = true;
        self.follow = true;
        self.loaded = true;
        self.pending_submit = None;
        self.pending_recipients = 0;
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
    /// 是否处于编辑态；false 为列表态。
    pub editing: bool,
    /// 列表搜索框；列表态常驻焦点。
    pub search: TextInput,
    /// Q 列表窗口（高亮/锚点/显式视口）。
    pub qa_window: PanelWindow,
    /// QA 列表是否已从数据库回投过。
    pub qa_loaded: bool,
    /// `/btw prompt` 暂存正文：QA 列表到达、线程建立后立即提交一次。
    pub pending_prompt: Option<String>,
}

impl LxBtw {
    /// 是否存在任何已打开的线程。
    pub fn is_empty(&self) -> bool {
        self.threads.is_empty()
    }

    /// 是否有 BTW 线程正在压缩。
    pub fn any_compacting(&self) -> bool {
        self.threads
            .values()
            .any(|thread| thread.compact.is_running())
    }

    /// 刷新所有 BTW 线程的压缩耗时；同时推进压缩线程的呼吸帧（运行中的检查点行）。
    pub fn tick_compacting(&mut self, now_ms: i64) {
        for thread in self.threads.values_mut() {
            if thread.compact.is_running() {
                thread.compact.elapsed_ms = thread
                    .compact
                    .started_at_ms
                    .map(|started| now_ms.saturating_sub(started).max(0));
                thread.chat.loading_frame = thread.chat.loading_frame.wrapping_add(1);
            }
        }
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

    /// 按 QA 取线程，不修改 active（异步失败回投用）。
    pub fn thread_mut(&mut self, qa_id: &str) -> Option<&mut BtwThreadState> {
        self.threads.get_mut(qa_id)
    }

    /// 按持久化线程 ID 查线程（详情 `/rewind` 的回投目标）。
    pub fn thread_by_thread_id(&self, thread_id: &str) -> Option<&BtwThreadState> {
        self.threads
            .values()
            .find(|thread| thread.thread_id.as_deref() == Some(thread_id))
    }

    /// 按持久化线程 ID 查可变线程（详情 `/compact` 的状态回投）。
    pub fn thread_by_thread_id_mut(&mut self, thread_id: &str) -> Option<&mut BtwThreadState> {
        self.threads
            .values_mut()
            .find(|thread| thread.thread_id.as_deref() == Some(thread_id))
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

    /// 搜索过滤后的 QA 真实下标（保持列表时间正序）。
    pub fn filtered_qa_indices(&self) -> Vec<usize> {
        let query = self.search.text().trim().to_lowercase();
        self.qa
            .iter()
            .enumerate()
            .filter(|(_, summary)| {
                query.is_empty()
                    || (!summary.title.is_empty() && summary.title.to_lowercase().contains(&query))
            })
            .map(|(index, _)| index)
            .collect()
    }

    /// `active` 在当前过滤结果中的位置；被过滤掉时为 None。
    pub fn selected_index(&self) -> Option<usize> {
        let active = self.active.as_deref()?;
        self.filtered_qa_indices()
            .iter()
            .position(|index| self.qa.get(*index).is_some_and(|qa| qa.qa_id == active))
    }

    /// 搜索变化后把窗口高亮同步到稳定 `active`。
    ///
    /// `active` 被当前过滤结果排除但仍有匹配项时，高亮与详情一起落到第一项：
    /// 否则列表画出的高亮与 `active` 分叉，`Shift+Enter` 会静默失败。
    /// 返回是否切换了 `active`（调用方需要补一次线程加载）。
    pub fn sync_selection_after_filter(&mut self) -> bool {
        let filtered = self.filtered_qa_indices();
        if filtered.is_empty() {
            self.qa_window.active = 0;
            self.qa_window.anchor = 0;
            self.qa_window.viewport = None;
            self.qa_window.clamp(0);
            return false;
        }
        let active_index = self.active.as_deref().and_then(|active| {
            filtered
                .iter()
                .position(|index| self.qa.get(*index).is_some_and(|qa| qa.qa_id == active))
        });
        let switched = active_index.is_none();
        let active_index = match active_index {
            Some(index) => index,
            None => {
                if let Some(qa_id) = filtered
                    .first()
                    .and_then(|index| self.qa.get(*index))
                    .map(|qa| qa.qa_id.clone())
                {
                    self.select_qa(&qa_id);
                }
                0
            }
        };
        self.qa_window.active = active_index;
        self.qa_window.anchor = active_index;
        self.qa_window.viewport = None;
        self.qa_window.clamp(filtered.len());
        switched
    }

    /// 列表上下移动：高亮变化时把稳定 `active` 指向新 QA；到两端即停。
    pub fn move_selection(&mut self, delta: isize) -> bool {
        let filtered = self.filtered_qa_indices();
        if filtered.is_empty() {
            return false;
        }
        let last = filtered.len() - 1;
        let Some(current) = self.selected_index() else {
            let next = if delta >= 0 { 0 } else { last };
            let Some(qa_id) = filtered
                .get(next)
                .and_then(|index| self.qa.get(*index))
                .map(|qa| qa.qa_id.clone())
            else {
                return false;
            };
            self.qa_window.active = next;
            self.qa_window.anchor = next;
            self.qa_window.viewport = None;
            self.select_qa(&qa_id);
            return true;
        };
        let next = (current as isize + delta).clamp(0, last as isize) as usize;
        if next == current {
            return false;
        }
        let Some(index) = filtered.get(next).copied() else {
            return false;
        };
        let Some(qa_id) = self.qa.get(index).map(|qa| qa.qa_id.clone()) else {
            return false;
        };
        self.qa_window.active = next;
        self.qa_window.anchor = next;
        self.qa_window.viewport = None;
        self.select_qa(&qa_id);
        true
    }

    /// 进入当前 QA 的编辑态；没有可编辑 QA 时返回 false。
    pub fn enter_edit(&mut self) -> bool {
        if self.selected_index().is_none() {
            return false;
        }
        let Some(qa_id) = self.active.clone() else {
            return false;
        };
        self.ensure_thread(&qa_id);
        self.editing = true;
        true
    }

    /// 退回列表态并保留草稿。
    pub fn leave_edit(&mut self) -> bool {
        if !self.editing {
            return false;
        }
        self.editing = false;
        true
    }

    /// 清空搜索并恢复稳定选中项的高亮；返回是否清空。
    pub fn clear_search(&mut self) -> bool {
        if self.search.text().is_empty() {
            return false;
        }
        self.search.clear();
        self.sync_selection_after_filter();
        true
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
        self.editing = false;
        self.search.clear();
        self.qa_window = PanelWindow::new(0);
        self.qa_loaded = false;
        self.pending_prompt = None;
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
