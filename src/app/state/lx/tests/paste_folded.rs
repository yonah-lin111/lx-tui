use super::*;
use crate::app::paste::{TextPasteAction, encode_pasted_text, folded_tokens};

#[test]
fn lx_text_paste_panel_respects_chat_mode_fences_and_plain_mode() {
    // Plain mode (ask editor) rejects text paste panel.
    let mut plain = Lx::plain();
    assert!(!plain.begin_text_paste("1\n2\n3".into(), "1\n2\n3".into()));
    assert!(plain.text_paste_panel().is_none());

    // Shell mode rejects text paste panel.
    let mut shell = Lx::new();
    shell.insert_char('!');
    assert!(shell.shell_mode());
    assert!(!shell.begin_text_paste("1\n2\n3".into(), "1\n2\n3".into()));

    // Inside code fence rejects text paste panel.
    let mut fenced = Lx::new();
    fenced.insert_str("```rust\n");
    assert!(!fenced.begin_text_paste("1\n2\n3".into(), "1\n2\n3".into()));

    // Normal Chat mode opens panel, cancel leaves buffer untouched, confirm Collapse inserts token.
    let mut lx = Lx::new();
    lx.insert_str("check");
    assert!(lx.begin_text_paste("a\nb\nc".into(), "a\nb\nc".into()));
    assert_eq!(lx.input, "check");
    assert!(lx.text_paste_cancel());
    assert_eq!(lx.input, "check");

    assert!(lx.begin_text_paste("a\nb\nc".into(), "a\nb\nc".into()));
    assert!(lx.text_paste_confirm(TextPasteAction::Text));
    let tok = encode_pasted_text("a\nb\nc");
    assert_eq!(lx.input, format!("check {tok} "));
    assert_eq!(lx.expanded_input(), "check a\nb\nc ");

    lx.undo();
    assert_eq!(lx.input, "check");
    lx.redo();
    assert_eq!(lx.input, format!("check {tok} "));
}

#[test]
fn lx_atomic_cursor_and_deletion_and_independent_multiple_expansions() {
    let mut lx = Lx::new();
    lx.resize(40, 8);
    let tok = encode_pasted_text("one\ntwo\nthree");
    lx.insert_str(&format!("X{tok}Y{tok}Z"));
    let parsed = folded_tokens(&lx.input);
    assert_eq!(parsed.len(), 2);

    // Move left across second token and first token.
    assert_eq!(lx.cursor, lx.input.len());
    lx.move_left(); // before 'Z' (= end of tok2)
    assert_eq!(lx.cursor, parsed[1].range.end);
    lx.move_left(); // jumps over tok2 to start of tok2
    assert_eq!(lx.cursor, parsed[1].range.start);
    lx.move_left(); // before 'Y' (= end of tok1)
    assert_eq!(lx.cursor, parsed[0].range.end);
    lx.move_left(); // jumps over tok1 to start of tok1
    assert_eq!(lx.cursor, parsed[0].range.start);

    // Expand only occurrence 1 (second chip).
    assert!(lx.toggle_paste(parsed[1].id, 1));
    assert!(lx.has_expanded_pastes());
    assert_eq!(lx.visual_rows().len(), 3);

    // Clear resets expansion state.
    lx.clear();
    assert!(!lx.has_expanded_pastes());
}
