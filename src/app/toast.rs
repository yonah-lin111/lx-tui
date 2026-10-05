//! 全局 toast 模型：单条替换、到期自动消失、hover 期间暂停消失动作。

use std::time::{Duration, Instant};

use crate::layout::PaneId;

/// 默认展示时长。
pub const TOAST_DURATION: Duration = Duration::from_millis(2000);

/// toast 语义变体；决定边框颜色。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToastKind {
    Info,
    Error,
}

/// 单条 toast；新消息整体替换旧消息并重置倒计时。
#[derive(Debug)]
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

    /// 附加标题；渲染时占一行。
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
mod tests {
    use super::*;

    fn toast(now: Instant) -> Toast {
        Toast::new(ToastKind::Info, "Copied to clipboard", None, now)
    }

    #[test]
    fn expires_after_duration() {
        let now = Instant::now();
        let toast = toast(now);
        assert!(!toast.is_expired(now + Duration::from_millis(1999)));
        assert!(toast.is_expired(now + TOAST_DURATION));
        assert_eq!(toast.next_deadline(), Some(now + TOAST_DURATION));
    }

    #[test]
    fn hover_suspends_expiry_without_extending_deadline() {
        let now = Instant::now();
        let mut toast = toast(now);
        toast.set_hovered(true);
        assert!(toast.hovered());
        assert!(!toast.is_expired(now + Duration::from_secs(60)));
        assert_eq!(toast.next_deadline(), None);
        toast.set_hovered(false);
        assert!(!toast.hovered());
        assert!(toast.is_expired(now + Duration::from_secs(60)));
        assert_eq!(toast.next_deadline(), Some(now + TOAST_DURATION));
    }

    #[test]
    fn with_title_sets_optional_title() {
        let now = Instant::now();
        let toast = toast(now).with_title("Clipboard");
        assert_eq!(toast.title.as_deref(), Some("Clipboard"));
    }
}
