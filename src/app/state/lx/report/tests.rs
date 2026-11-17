use super::*;
use crate::app::state::DetailTarget;

fn target() -> DetailTarget {
    DetailTarget::Agent("agent-1".into())
}

#[test]
fn defaults_to_resend_when_previous_report_exists() {
    let panel = ReportPanel::new(target(), "note".into(), Some("body".into()));
    assert_eq!(panel.len(), 3);
    assert_eq!(panel.active_action(), ReportAction::Resend);
    assert!(panel.enabled(ReportAction::Resend));
}

#[test]
fn resend_disabled_without_previous_report() {
    let panel = ReportPanel::new(target(), String::new(), None);
    assert!(!panel.enabled(ReportAction::Resend));
    assert_eq!(panel.active_action(), ReportAction::Regenerate);
    assert!(panel.enabled(ReportAction::Regenerate));

    let blank = ReportPanel::new(target(), String::new(), Some("  \n ".into()));
    assert!(!blank.enabled(ReportAction::Resend));
    assert_eq!(blank.active_action(), ReportAction::Regenerate);
}

#[test]
fn moves_skip_disabled_resend_and_keep_note() {
    let mut panel = ReportPanel::new(target(), "keep me".into(), None);
    assert!(panel.move_active(1));
    assert_eq!(panel.active_action(), ReportAction::Cancel);
    assert!(panel.move_active(1));
    assert_eq!(panel.active_action(), ReportAction::Regenerate);
    assert_eq!(panel.note, "keep me");
}
