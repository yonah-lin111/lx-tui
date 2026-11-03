use super::*;
use crate::app::paste::PasteAnchor;
use crate::app::paste::folded::MAX_FOLDED_BYTES;

#[test]
fn candidate_thresholds_respect_line_count_and_unicode_char_count() {
    assert!(text_paste_candidate("").is_none());
    assert!(text_paste_candidate("   \n\t  ").is_none());

    // 2 lines and <= 150 chars: not a candidate.
    assert!(text_paste_candidate("line 1\nline 2").is_none());
    let chars_150: String = "汉".repeat(150);
    assert!(text_paste_candidate(&chars_150).is_none());

    // 3 lines (after trim): candidate.
    let three_lines = "\n  first\r\nsecond\rthird  \n";
    let cand = text_paste_candidate(three_lines).expect("3 lines should trigger");
    assert_eq!(cand.content, "first\nsecond\nthird");
    assert_eq!(cand.full_text, "\n  first\nsecond\nthird  \n");
    assert_eq!(cand.line_count, 3);
    assert!(!cand.too_large);

    // 1 line with 151 Unicode chars: candidate.
    let chars_151: String = "汉".repeat(151);
    let cand_chars = text_paste_candidate(&chars_151).expect("151 chars should trigger");
    assert_eq!(cand_chars.line_count, 1);
    assert!(!cand_chars.too_large);
}

#[test]
fn oversize_candidate_disables_collapse_and_defaults_to_full_text() {
    let huge = "a".repeat(MAX_FOLDED_BYTES + 1);
    let cand = text_paste_candidate(&huge).expect("huge text should still open panel");
    assert!(cand.too_large);

    let mut panel = TextPastePanel::new(cand.content, cand.full_text, PasteAnchor::at_cursor(0));
    assert!(panel.too_large);
    assert!(!panel.collapse_enabled());
    assert!(!panel.enabled(TextPasteAction::Collapse));
    assert!(panel.enabled(TextPasteAction::Full));
    assert_eq!(panel.disabled_mask(), [true, false]);
    assert_eq!(panel.active(), 1);
    assert_eq!(panel.active_action(), Some(TextPasteAction::Full));

    // Moving or hovering onto disabled Collapse item is rejected.
    assert!(!panel.move_active(-1));
    assert!(!panel.move_active(1));
    assert!(!panel.set_active(0));
    assert_eq!(panel.active(), 1);
}

#[test]
fn normal_panel_defaults_to_collapse_and_cycles_between_actions() {
    let content = "1\n2\n3".to_string();
    let mut panel = TextPastePanel::new(content.clone(), content, PasteAnchor::at_cursor(4));
    assert!(!panel.too_large);
    assert_eq!(panel.active(), 0);
    assert_eq!(panel.active_action(), Some(TextPasteAction::Collapse));
    assert_eq!(panel.disabled_mask(), [false, false]);

    assert!(panel.move_active(1));
    assert_eq!(panel.active(), 1);
    assert_eq!(panel.active_action(), Some(TextPasteAction::Full));

    assert!(panel.move_active(1));
    assert_eq!(panel.active(), 0);

    let mut state = TextPasteState::default();
    assert!(state.panel().is_none());
    state.open(panel);
    assert!(state.panel().is_some());
    assert!(state.cancel());
    assert!(!state.cancel());
}
