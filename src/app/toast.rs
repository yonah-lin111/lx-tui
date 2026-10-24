//! 全局 toast 模型：单条替换、到期自动消失、hover 期间暂停消失动作。

use std::time::{Duration, Instant};

use crate::layout::PaneId;

/// 默认展示时长。
pub const TOAST_DURATION: Duration = Duration::from_millis(2000);

/// toast 语义变体；决定边框与标题颜色。
///
/// 三档由用户指定：`default`（默认强调色）、`warning`（黄）、`danger`（红）。
/// 原先只有 `Info` / `Error` 两档，`Warning` 是为一类「要留意但不算失败」的反馈
/// （例如 lx 输入框上方的未知命令提示）补的——把两者混进 `danger` 会让真正的
/// 失败失去分量。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToastKind {
    Default,
    Warning,
    Danger,
}

/// 单条 toast；新消息整体替换旧消息并重置倒计时。
///
/// `Clone` 用于把同一条反馈投给**多个**持有同一会话的窗格（会话级效应没有单一锚点）；
/// `deadline` 是绝对时刻，克隆出的每一份到期时间一致。
#[derive(Debug, Clone)]
pub struct Toast {
    pub kind: ToastKind,
    pub title: Option<String>,
    pub message: String,
    /// 锚点窗格；None 或容器放不下时显示在屏幕顶部居中。
    pub anchor: Option<PaneId>,
    deadline: Instant,
    hovered: bool,
}

impl Toast {
    /// 以当前时刻开始计时。
    pub fn new(
        kind: ToastKind,
        message: impl Into<String>,
        anchor: Option<PaneId>,
        now: Instant,
    ) -> Self {
        Self {
            kind,
            title: None,
            message: message.into(),
            anchor,
            deadline: now + TOAST_DURATION,
            hovered: false,
        }
    }

    /// 附加标题；渲染在顶边框上。
    pub fn with_title(mut self, title: impl Into<String>) -> Self {
        self.title = Some(title.into());
        self
    }

    /// 是否已过期；hover 期间不过期。
    pub fn is_expired(&self, now: Instant) -> bool {
        !self.hovered && now >= self.deadline
    }

    /// 下一次到期时间；hover 期间为 None，事件循环据此决定唤醒。
    pub fn next_deadline(&self) -> Option<Instant> {
        (!self.hovered).then_some(self.deadline)
    }

    /// 是否处于 hover。
    pub fn hovered(&self) -> bool {
        self.hovered
    }

    /// 设置 hover；只冻结消失动作，不延长 deadline。
    pub fn set_hovered(&mut self, hovered: bool) {
        self.hovered = hovered;
    }
}

#[cfg(test)]
mod tests;
