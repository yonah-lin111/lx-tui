//! 单元测试：显示延时与到期语义。

use super::*;

fn tooltip(now: Instant) -> Tooltip {
    Tooltip::new(TooltipTarget::AddTab, now)
}

#[test]
fn stays_hidden_until_delay_elapses() {
    let now = Instant::now();
    let mut tooltip = tooltip(now);
    assert!(!tooltip.is_visible());
    assert!(!tooltip.reveal(now + Duration::from_millis(399)));
    assert!(!tooltip.is_visible(), "未满延时保持隐藏");
    assert!(tooltip.reveal(now + TOOLTIP_DELAY));
    assert!(tooltip.is_visible());
}

#[test]
fn reveal_reports_change_once() {
    let now = Instant::now();
    let mut tooltip = tooltip(now);
    assert!(tooltip.reveal(now + TOOLTIP_DELAY));
    assert!(
        !tooltip.reveal(now + Duration::from_secs(60)),
        "已可见不再变化"
    );
}

#[test]
fn next_deadline_clears_after_reveal() {
    let now = Instant::now();
    let mut tooltip = tooltip(now);
    assert_eq!(tooltip.next_deadline(), Some(now + TOOLTIP_DELAY));
    tooltip.reveal(now + TOOLTIP_DELAY);
    assert_eq!(tooltip.next_deadline(), None);
}

#[test]
fn target_is_retained_for_anchor_resolution() {
    let now = Instant::now();
    let pane = PaneId::alloc();
    let tooltip = Tooltip::new(TooltipTarget::PaneToggle(pane), now);
    assert_eq!(tooltip.target, TooltipTarget::PaneToggle(pane));
}
