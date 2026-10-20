//! lx 页的后台任务页面：在**窗格内**整页显示本会话的后台进程与（预留的）子代理。
//!
//! 与浮层（`app::overlay`）不同，这里是窗格自己的第二个页面：进入它不覆盖屏幕，
//! 也不销毁 lx 页状态——`pane.lx` 的会话、消息、草稿、滚动位置全部原样保留，
//! 退出页面即可回到离开时的样子。因此「打开后台任务」不需要保存/恢复任何东西。
//!
//! 页面结构对齐 opencode 的 composer 多 tab：顶部一条 tab（`Processes` / `Agents`），
//! 主体是当前 tab 的列表，底部一行键位提示。`Agents` 目前是预留位（后续显示 subagent），
//! 列表为空并给出占位提示。

use crate::ui::text;

use super::slash::PanelWindow;

/// 后台任务页的 tab（固定顺序，←/→ 轮转）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TaskTab {
    /// 后台进程列表。
    #[default]
    Processes,
    /// 子代理列表；本阶段为预留位（尚无数据来源）。
    Agents,
}

impl TaskTab {
    /// 全部 tab（固定顺序）。
    pub const ALL: [TaskTab; 2] = [TaskTab::Processes, TaskTab::Agents];

    /// tab 标签。
    pub fn label(self) -> &'static str {
        match self {
            Self::Processes => text::LX_TASK_TAB_PROCESSES,
            Self::Agents => text::LX_TASK_TAB_AGENTS,
        }
    }
}

/// 后台进程在页面列表里的一行。
///
/// 只装**展示 + 定位**所需的数据：`session_id` 是模型见过的进程会话 ID（停止操作的
/// 唯一键），其余字段供列表行显示。进程句柄不进 `AppState`（`app/` 不持有进程）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProcessEntry {
    /// 模型见到的进程会话 ID（停止操作的唯一键）。
    pub session_id: u64,
    /// 命令原文（单行化后的展示文本）。
    pub command: String,
    /// 该进程的原始工作目录。
    pub cwd: String,
    /// 是否使用 PTY（交互式）。
    pub tty: bool,
    /// 启动时刻（UTC 毫秒）：用于计算已运行时长。
    pub started_at_ms: i64,
}

impl ProcessEntry {
    /// 已运行时长（毫秒）；`now_ms` 早于启动时刻时按 0 计。
    pub fn elapsed_ms(&self, now_ms: i64) -> i64 {
        now_ms.saturating_sub(self.started_at_ms).max(0)
    }

    /// 运行时长标签：`1m 30s` / `12s` / `2h 05m`（裸时长，前缀由调用方决定）。
    pub fn age_label(&self, now_ms: i64) -> String {
        let seconds = self.elapsed_ms(now_ms) / 1_000;
        if seconds < 60 {
            format!("{seconds}s")
        } else if seconds < 3_600 {
            format!("{}m {:02}s", seconds / 60, seconds % 60)
        } else {
            format!("{}h {:02}m", seconds / 3_600, (seconds % 3_600) / 60)
        }
    }
}

/// lx 窗格的第二个页面：后台任务管理。
///
/// 数据由事件层按需回投（`app/` 不持有进程句柄）；页面自身只保存展示状态。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TaskPage {
    /// 当前 tab。
    pub tab: TaskTab,
    /// 本会话当前运行中的后台进程。
    pub processes: Vec<ProcessEntry>,
    /// 计算运行时长的"现在"（UTC 毫秒），由事件循环每轮刷新。
    pub now_ms: i64,
    /// 进程列表的窗口状态（高亮/锚点/显式视口）。
    pub window: PanelWindow,
    /// `[open]` 带进来的目标进程会话 ID：**一次性定位请求**。
    ///
    /// 落地由 [`Self::resolve_focus`] 消费（高亮落到该任务并清空），因此不会
    /// 干扰后续的 ↑/↓ 操作；目标进程已经退出时回落列表首项。
    pub focus: Option<u64>,
    /// 高亮任务的输出尾部快照（事件层回投；`app/` 不持有进程句柄）。
    pub output: Vec<String>,
    /// 输出区顶部行偏移（`PageUp`/`PageDown` 与滚轮用）。
    pub output_scroll: usize,
    /// 输出区可见行数（事件循环在绘制前写入）。
    ///
    /// 翻页与滚动条几何需要知道「一屏几行」，而 `app/` 拿不到窗格几何——与
    /// `LxChat::viewport_top` 同一套做法：渲染只读、事件循环写。
    /// 面板打开时历史区根本没画，因此**不**维护历史行数（那会白烧每帧一次
    /// markdown 排版，且没有任何读者）。
    pub output_visible: usize,
}

impl TaskPage {
    /// 当前 tab 的条目数（`Agents` 阶段恒为 0：尚无数据来源）。
    pub fn len(&self) -> usize {
        match self.tab {
            TaskTab::Processes => self.processes.len(),
            TaskTab::Agents => 0,
        }
    }

    /// 当前 tab 是否没有条目。
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// 高亮当前的进程条目（`Processes` tab 且索引有效时）。
    pub fn selected(&self) -> Option<&ProcessEntry> {
        matches!(self.tab, TaskTab::Processes)
            .then(|| self.processes.get(self.window.active))
            .flatten()
    }

    /// 切换 tab（←/→ 轮转），并钳制高亮。
    pub fn cycle_tab(&mut self, delta: isize) -> bool {
        let count = TaskTab::ALL.len() as isize;
        let current = TaskTab::ALL
            .iter()
            .position(|tab| *tab == self.tab)
            .unwrap_or(0) as isize;
        let next = (current + delta).rem_euclid(count) as usize;
        let tab = TaskTab::ALL[next];
        if tab == self.tab {
            return false;
        }
        self.tab = tab;
        // 窗口状态属于**进程列表**，与当前 tab 的条目数无关：`Agents` 恒为 0，
        // 若按 `self.len()` 钳制会把 `active/anchor/viewport` 全部抹成 0，
        // 切回 Processes 时用户原来选的那项已经丢了（`^d` 会停错进程）。
        self.window.clamp(self.processes.len());
        true
    }

    /// 直接切到指定 tab（鼠标点击）；返回是否变化。
    pub fn select_tab(&mut self, tab: TaskTab) -> bool {
        if self.tab == tab {
            return false;
        }
        self.tab = tab;
        self.window.clamp(self.processes.len());
        true
    }

    /// 移动高亮（**不循环**，到两端即停）；返回高亮是否真的移动了。
    ///
    /// 不循环是「↑ 在列表顶部回到输入框」的前提：面板要能分辨"上移成功"与
    /// "已经在顶部、无处可去"，后者才该退出面板。循环会把顶部再绕回底部，
    /// 用户永远退不出去。
    pub fn move_active(&mut self, delta: isize) -> bool {
        let len = self.len();
        if len == 0 {
            return false;
        }
        let last = len - 1;
        let next = (self.window.active as isize + delta).clamp(0, last as isize) as usize;
        if next == self.window.active {
            return false;
        }
        self.window.active = next;
        self.window.anchor = next;
        self.window.viewport = None;
        // 换任务就把输出贴到尾部：用户点开/切到某条任务，关心的是它**最新**的输出。
        self.pin_output_to_tail();
        true
    }

    /// 消费一次性的 `[open]` 定位请求：高亮落到指定会话的进程。
    ///
    /// **列表还没到时要保留请求**：`[open]` 打开面板那一刻 `processes` 必然是空的
    /// （真正的列表由 `ListSessionProcesses` 在下一轮回投）。早先这里先 `take()`
    /// 再判空，请求就被吃掉了，高亮永远落在首项。现在只在列表非空时消费。
    ///
    /// 目标进程已消失（卡片还留在历史里，但进程早退出了）时回落列表首项——
    /// 面板照常打开，而不是给一个空高亮。请求无论成败都清空（一次性）。
    pub fn resolve_focus(&mut self) {
        if self.processes.is_empty() {
            return;
        }
        let Some(session_id) = self.focus.take() else {
            return;
        };
        let index = self
            .processes
            .iter()
            .position(|process| process.session_id == session_id)
            .unwrap_or(0);
        self.window.active = index;
        self.window.anchor = index;
        self.window.viewport = None;
        self.window.clamp(self.processes.len());
        self.pin_output_to_tail();
    }

    /// 输出视口贴到尾部（最新输出）；总行数不足一屏时归零。
    pub fn pin_output_to_tail(&mut self) {
        self.output_scroll = self.output_max_scroll();
    }

    /// 输出区可滚动的最大偏移（总行数 − 可见行数）。
    ///
    /// `pub(crate)`：滚动条轨道点击/拖拽也要用它，**不能**在别处再写一遍公式，
    /// 否则「键盘滚到哪停」与「拖拽滚到哪停」会分叉。
    pub(crate) fn output_max_scroll(&self) -> usize {
        self.output.len().saturating_sub(self.output_visible)
    }

    /// 可见行数变化后调整滚动位置并写入新行数。
    ///
    /// 「是否原本贴尾」用**旧**可见行数判断（此时 `output_visible` 还是上一帧的值）：
    /// 直接在写入新行数之后再 `pin_output_to_tail()` 会在用户改窗口大小时把他从
    /// 日志中间甩到最新一屏——与 [`Self::set_output`] 里的 `was_at_tail` 同一规则。
    pub fn resize_output_viewport(&mut self, visible: usize) {
        let was_at_tail = self.output_scroll >= self.output_max_scroll();
        self.output_visible = visible;
        let max = self.output_max_scroll();
        self.output_scroll = if was_at_tail {
            max
        } else {
            self.output_scroll.min(max)
        };
    }

    /// 列表滚轮：移动显式视口起点（`visible` 为当前可见行数）；返回是否移动。
    pub fn scroll_window(&mut self, delta: isize, visible: usize) -> bool {
        let len = self.processes.len();
        if len == 0 || visible == 0 {
            return false;
        }
        let max = len.saturating_sub(visible);
        let current = self.window.viewport.unwrap_or_else(|| {
            // 没有显式视口时以高亮为准（与渲染的 `window_start` 同口径）。
            let active = self.window.active.min(len - 1);
            active.saturating_sub(visible - 1).min(max)
        });
        let next = (current as isize)
            .saturating_add(delta)
            .clamp(0, max as isize) as usize;
        if next == current && self.window.viewport.is_some() {
            return false;
        }
        self.window.viewport = Some(next);
        true
    }

    /// 翻动输出区（负数向上）；返回视口是否移动。
    pub fn scroll_output(&mut self, delta: isize) -> bool {
        let max = self.output_max_scroll();
        let current = self.output_scroll.min(max);
        let next = (current as isize)
            .saturating_add(delta)
            .clamp(0, max as isize) as usize;
        let moved = next != self.output_scroll;
        self.output_scroll = next;
        moved
    }

    /// 写入高亮任务的输出快照。
    ///
    /// **不改变滚动位置**：面板每轮都会重新拉一次输出，如果每次都贴尾，用户往上翻
    /// 看历史日志时就会被立刻拽回底部（用户报的 bug）。「贴尾」只发生在真正切换
    /// 目标的时刻——打开面板定位（[`Self::resolve_focus`]）与切换高亮
    /// （[`Self::move_active`]）。
    ///
    /// 内容变短时把偏移钳到新的上限，避免视口停在一个越界位置。
    pub fn set_output(&mut self, output: Vec<String>) -> bool {
        if self.output == output {
            return false;
        }
        let was_at_tail = self.output_scroll >= self.output_max_scroll();
        self.output = output;
        let max = self.output_max_scroll();
        self.output_scroll = if was_at_tail {
            // 原本就贴着底部（跟随最新）→ 继续跟随，新日志照常推进。
            max
        } else {
            // 用户手动上翻过 → 保持位置，只做越界钳制。
            self.output_scroll.min(max)
        };
        true
    }

    /// 以新列表替换进程并钳制高亮；返回**列表内容**是否变化。
    ///
    /// `now_ms` 只用于渲染时长，不参与「是否变化」判定：它由 `tick_task_pages`
    /// 单独按秒推进。早先把它算进变化判定，导致面板打开期间每次回投都判「有变化」
    /// → 每帧重绘。
    pub fn set_processes(&mut self, processes: Vec<ProcessEntry>, now_ms: i64) -> bool {
        let changed = self.processes != processes;
        self.processes = processes;
        self.now_ms = now_ms;
        // 列表长度变了才需要钳制（且按进程列表长度，不看当前 tab）。
        if changed {
            self.window.clamp(self.processes.len());
        }
        changed
    }

    /// 移除已退出的进程（保留高亮所及范围内最近的位置）；返回是否有变化。
    pub fn retain_process(&mut self, process_session_id: u64) -> bool {
        let before = self.processes.len();
        self.processes
            .retain(|process| process.session_id != process_session_id);
        if self.processes.len() == before {
            return false;
        }
        self.window.clamp(self.processes.len());
        true
    }
}

#[cfg(test)]
mod tests;
