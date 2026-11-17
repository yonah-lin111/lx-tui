//! `/report` 二级面板的纯状态：重发上一份报告或按备注重新生成。

use super::detail::DetailTarget;
use super::slash::PanelWindow;

/// 报告二级面板的动作。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReportAction {
    /// 直接复用上一轮报告正文，不调用模型。
    Resend,
    /// 按备注重新生成一轮报告。
    Regenerate,
    Cancel,
}

impl ReportAction {
    /// 面板顺序：默认 `Resend`。
    pub const ALL: [ReportAction; 3] = [
        ReportAction::Resend,
        ReportAction::Regenerate,
        ReportAction::Cancel,
    ];
}

/// 报告动作面板。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReportPanel {
    pub window: PanelWindow,
    pub actions: Vec<ReportAction>,
    /// 打开时的详情目标；执行前复核仍活跃。
    pub target: DetailTarget,
    /// `/report` 的备注，只作用于 `Regenerate`。
    pub note: String,
    /// 上一轮报告正文；为空或缺失时 `Resend` 禁用。
    pub last_report: Option<String>,
}

impl ReportPanel {
    /// 以首个可用项为默认高亮：有上一轮报告时 `Resend`，否则 `Regenerate`。
    pub fn new(target: DetailTarget, note: String, last_report: Option<String>) -> Self {
        let last_report = last_report.filter(|body| !body.trim().is_empty());
        let active = if last_report.is_some() { 0 } else { 1 };
        Self {
            window: PanelWindow::new(active),
            actions: ReportAction::ALL.to_vec(),
            target,
            note,
            last_report,
        }
    }

    /// 动作数量。
    pub fn len(&self) -> usize {
        self.actions.len()
    }

    /// 动作集是否为空（正常构造下恒为 false）。
    pub fn is_empty(&self) -> bool {
        self.actions.is_empty()
    }

    /// 高亮动作。
    pub fn active_action(&self) -> ReportAction {
        self.actions
            .get(self.window.active)
            .copied()
            .unwrap_or(ReportAction::Cancel)
    }

    /// 动作是否可选。
    pub fn enabled(&self, action: ReportAction) -> bool {
        match action {
            ReportAction::Resend => self.last_report.is_some(),
            ReportAction::Regenerate | ReportAction::Cancel => true,
        }
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
