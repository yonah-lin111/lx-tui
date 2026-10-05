//! 单元测试；仅测试构建编译。

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
