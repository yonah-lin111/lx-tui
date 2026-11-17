//! 停止面板的纯状态：可停止工作快照、五项动作与显式窗口。
//!
//! 只保存选择与可用性；真正停止由 `app/update/chat/stop.rs` 转成 `Effect` 交给事件层。

use super::slash::PanelWindow;

/// 停止面板的动作（顺序固定，默认第一项）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StopAction {
    /// 主轮 + 全部 subagent + 全部 shell/后台进程。
    All,
    /// 只停主轮。
    Main,
    /// 只停当前会话树全部 subagent。
    Subagents,
    /// 只停用户 shell 与受管后台进程。
    Shells,
    Cancel,
}

impl StopAction {
    /// 面板顺序：`Stop all` 默认高亮。
    pub const ALL: [StopAction; 5] = [
        StopAction::All,
        StopAction::Main,
        StopAction::Subagents,
        StopAction::Shells,
        StopAction::Cancel,
    ];
}

/// 打开/执行停止面板时重新计算的可用性快照。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct StopAvailability {
    /// 主轮在途（Streaming / Committing）。
    pub main: bool,
    /// 可停止 subagent 数（运行中或有排队输入）。
    pub subagents: usize,
    /// 是否有用户 `!` shell 在跑（含尚未拿到进程 ID 的 pending）。
    pub shells: usize,
    /// 受管后台进程数（`exec_command` 等的运行实例）。
    pub processes: usize,
}

impl StopAvailability {
    /// 从当前窗格状态采集。
    pub fn from_lx(lx: &super::Lx) -> Self {
        Self {
            // `Finishing` 只是等 DB ack，取消流是 no-op；不能显示成可停止。
            main: matches!(
                lx.chat.request,
                super::chat::RequestState::Streaming { .. }
                    | super::chat::RequestState::Committing { .. }
            ),
            subagents: lx.agents.stoppable_count(),
            shells: usize::from(lx.chat.has_running_shell()),
            processes: lx.background_tasks,
        }
    }

    /// 是否存在任何可停止工作。
    pub fn any(&self) -> bool {
        self.main || self.subagents > 0 || self.shells > 0 || self.processes > 0
    }

    /// 动作当前是否可选。
    pub fn enabled(&self, action: StopAction) -> bool {
        match action {
            StopAction::All => self.any(),
            StopAction::Main => self.main,
            StopAction::Subagents => self.subagents > 0,
            StopAction::Shells => self.shells > 0 || self.processes > 0,
            StopAction::Cancel => true,
        }
    }
}

/// 停止动作面板。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StopPanel {
    pub window: PanelWindow,
    /// 固定五项（禁用项保留在列表中并置灰）。
    pub actions: Vec<StopAction>,
    pub availability: StopAvailability,
}

impl StopPanel {
    /// 以首个可用项为默认高亮（正常为 `Stop all`）。
    pub fn new(availability: StopAvailability) -> Self {
        let active = StopAction::ALL
            .iter()
            .position(|action| availability.enabled(*action))
            .unwrap_or(0);
        Self {
            window: PanelWindow::new(active),
            actions: StopAction::ALL.to_vec(),
            availability,
        }
    }

    /// 动作数量（渲染与导航共用）。
    pub fn len(&self) -> usize {
        self.actions.len()
    }

    /// 动作集是否为空（正常构造下恒为 false）。
    pub fn is_empty(&self) -> bool {
        self.actions.is_empty()
    }

    /// 高亮动作。
    pub fn active_action(&self) -> StopAction {
        self.actions
            .get(self.window.active)
            .copied()
            .unwrap_or(StopAction::Cancel)
    }

    /// 动作是否可选。
    pub fn enabled(&self, action: StopAction) -> bool {
        self.availability.enabled(action)
    }

    /// 键盘移动高亮：循环并跳过禁用项。
    pub fn move_active(&mut self, delta: isize) -> bool {
        let len = self.actions.len() as isize;
        if len == 0 {
            return false;
        }
        let mut index = self.window.active as isize;
        for _ in 0..len {
            index = (index + delta.signum()).rem_euclid(len);
            if self.enabled(self.actions[index as usize]) {
                let next = index as usize;
                if next == self.window.active {
                    return false;
                }
                self.window = PanelWindow::new(next);
                return true;
            }
        }
        false
    }

    /// 鼠标悬停：只接受可用项。
    pub fn set_active(&mut self, index: usize) -> bool {
        let Some(action) = self.actions.get(index).copied() else {
            return false;
        };
        if !self.enabled(action) || self.window.active == index {
            return false;
        }
        self.window.active = index;
        true
    }
}

#[cfg(test)]
mod tests;
