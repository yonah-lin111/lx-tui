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
        self.window.clamp(self.len());
        true
    }

    /// 移动高亮（循环）；无可选项时不动。
    pub fn move_active(&mut self, delta: isize) -> bool {
        let len = self.len();
        if len == 0 {
            return false;
        }
        self.window.move_active(delta, len);
        true
    }

    /// 以新列表替换进程并钳制高亮；返回是否有变化。
    pub fn set_processes(&mut self, processes: Vec<ProcessEntry>, now_ms: i64) -> bool {
        let changed = self.processes != processes || self.now_ms != now_ms;
        self.processes = processes;
        self.now_ms = now_ms;
        self.window.clamp(self.len());
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
        self.window.clamp(self.len());
        true
    }
}

#[cfg(test)]
mod tests;
