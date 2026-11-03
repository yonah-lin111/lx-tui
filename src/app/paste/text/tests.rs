use super::*;
use crate::app::paste::PasteAnchor;
use crate::app::paste::folded::MAX_FOLDED_BYTES;

#[test]
fn any_non_whitespace_paste_triggers_candidate() {
    assert!(text_paste_candidate("").is_none());
    assert!(text_paste_candidate("   \n\t  ").is_none());

    // Short 1-line and 2-line pastes are also candidates (panel opens on every paste).
    let short = text_paste_candidate("hello").expect("short paste should trigger");
    assert_eq!(short.content, "hello");
    assert_eq!(short.full_text, "hello");
    assert_eq!(short.line_count, 1);
    assert!(!short.too_large);

    // 3 lines (after trim): candidate.
    let three_lines = "\n  first\r\nsecond\rthird  \n";
    let cand = text_paste_candidate(three_lines).expect("3 lines should trigger");
    assert_eq!(cand.content, "first\nsecond\nthird");
    assert_eq!(cand.full_text, "\n  first\nsecond\nthird  \n");
    assert_eq!(cand.line_count, 3);
    assert!(!cand.too_large);
}

#[test]
fn oversize_candidate_disables_fold_actions_and_keeps_raw_active() {
    let huge = "a".repeat(MAX_FOLDED_BYTES + 1);
    let cand = text_paste_candidate(&huge).expect("huge text should still open panel");
    assert!(cand.too_large);

    let mut panel = TextPastePanel::new(cand.content, cand.full_text, PasteAnchor::at_cursor(0));
    assert!(panel.too_large);
    assert!(!panel.collapse_enabled());
    assert!(panel.enabled(TextPasteAction::Raw));
    assert!(!panel.enabled(TextPasteAction::Text));
    assert!(!panel.enabled(TextPasteAction::Code));
    assert!(!panel.enabled(TextPasteAction::Log));
    assert_eq!(panel.disabled_mask(), [false, true, true, true]);
    assert_eq!(panel.active(), 0);
    assert_eq!(panel.active_action(), Some(TextPasteAction::Raw));

    // Moving or hovering onto disabled fold items is rejected.
    assert!(!panel.move_active(-1));
    assert!(!panel.move_active(1));
    assert!(!panel.set_active(1));
    assert!(!panel.set_active(2));
    assert!(!panel.set_active(3));
    assert_eq!(panel.active(), 0);
}

#[test]
fn normal_panel_defaults_to_raw_and_cycles_across_four_actions() {
    let content = "1\n2\n3".to_string();
    let mut panel = TextPastePanel::new(content.clone(), content, PasteAnchor::at_cursor(4));
    assert!(!panel.too_large);
    assert_eq!(panel.active(), 0);
    assert_eq!(panel.active_action(), Some(TextPasteAction::Raw));
    assert_eq!(panel.disabled_mask(), [false, false, false, false]);

    assert!(panel.move_active(1));
    assert_eq!(panel.active(), 1);
    assert_eq!(panel.active_action(), Some(TextPasteAction::Text));

    assert!(panel.move_active(1));
    assert_eq!(panel.active(), 2);
    assert_eq!(panel.active_action(), Some(TextPasteAction::Code));

    assert!(panel.move_active(1));
    assert_eq!(panel.active(), 3);
    assert_eq!(panel.active_action(), Some(TextPasteAction::Log));

    assert!(panel.move_active(1));
    assert_eq!(panel.active(), 0);

    let mut state = TextPasteState::default();
    assert!(state.panel().is_none());
    state.open(panel);
    assert!(state.panel().is_some());
    assert!(state.cancel());
    assert!(!state.cancel());
}
