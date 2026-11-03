use super::*;
use crate::app::paste::{PasteAnchor, TextPasteAction, encode_pasted_text, folded_tokens};

#[test]
fn confirm_collapse_and_full_insert_as_single_undo_steps_and_cancel_leaves_buffer_untouched() {
    let mut p = prompt(40, 10);
    p.insert_str("prefix");
    let rev_before = p.revision();

    // Cancel leaves buffer and undo stack untouched.
    assert!(p.begin_text_paste(
        "line 1\nline 2\nline 3".into(),
        "line 1\nline 2\nline 3".into(),
        PasteAnchor::at_cursor(p.cursor()),
    ));
    assert!(p.text_paste_panel().is_some());
    assert_eq!(p.text(), "prefix");
    assert_eq!(p.revision(), rev_before);
    assert!(p.text_paste_cancel());
    assert!(p.text_paste_panel().is_none());
    assert_eq!(p.text(), "prefix");

    // Confirm Collapse inserts token with surrounding spaces and is 1 undo step.
    assert!(p.begin_text_paste(
        "line 1\nline 2\nline 3".into(),
        "line 1\nline 2\nline 3".into(),
        PasteAnchor::at_cursor(p.cursor()),
    ));
    assert!(p.text_paste_confirm(TextPasteAction::Collapse));
    assert!(p.text_paste_panel().is_none());
    let expected_token = encode_pasted_text("line 1\nline 2\nline 3");
    assert_eq!(p.text(), format!("prefix {expected_token} "));

    p.undo();
    assert_eq!(p.text(), "prefix");
    p.redo();
    assert_eq!(p.text(), format!("prefix {expected_token} "));

    // Confirm Full replaces selection with full_text.
    let anchor = PasteAnchor {
        selection: Some((0, p.text().len())),
        cursor: p.cursor(),
    };
    assert!(p.begin_text_paste("a\nb\nc".into(), "  a\nb\nc  ".into(), anchor,));
    assert!(p.text_paste_confirm(TextPasteAction::Full));
    assert_eq!(p.text(), "  a\nb\nc  ");
}

#[test]
fn fence_and_protected_lines_reject_text_paste_panel() {
    let mut p = prompt(40, 10);
    p.insert_str("```rust\n");
    assert!(!p.begin_text_paste(
        "1\n2\n3".into(),
        "1\n2\n3".into(),
        PasteAnchor::at_cursor(p.cursor()),
    ));
    assert!(p.text_paste_panel().is_none());

    let mut p2 = prompt(40, 10);
    p2.set_text("+++ supplement --start\n{title: demo}\nbody\n+++ supplement --end");
    // Cursor on `{title: demo}` protected line (offset 25).
    assert!(!p2.begin_text_paste(
        "1\n2\n3".into(),
        "1\n2\n3".into(),
        PasteAnchor::at_cursor(25),
    ));
}

#[test]
fn cursor_movement_and_backspace_delete_treat_folded_token_as_atomic() {
    let mut p = prompt(40, 10);
    let tok = encode_pasted_text("alpha\nbeta\ngamma");
    p.set_text(&format!("A{tok}B"));
    let tok_start = 1;
    let tok_end = 1 + tok.len();

    // Start at end ("A<tok>B|"), move left step by step.
    assert_eq!(p.cursor(), tok_end + 1);
    p.move_left();
    assert_eq!(p.cursor(), tok_end);
    p.move_left();
    assert_eq!(p.cursor(), tok_start, "Left jumps across the entire token");
    p.move_left();
    assert_eq!(p.cursor(), 0);

    // Move right step by step.
    p.move_right();
    assert_eq!(p.cursor(), tok_start);
    p.move_right();
    assert_eq!(p.cursor(), tok_end, "Right jumps across the entire token");

    // Backspace at tok_end deletes the entire token in one step.
    p.backspace();
    assert_eq!(p.text(), "AB");
    assert_eq!(p.cursor(), 1);
    p.undo();
    assert_eq!(p.text(), format!("A{tok}B"));

    // Delete at tok_start deletes the entire token in one step.
    p.set_cursor_offset(tok_start);
    p.delete();
    assert_eq!(p.text(), "AB");
}

#[test]
fn expand_and_collapse_do_not_bump_revision_or_dirty_and_reset_on_undo_or_set_text() {
    let mut p = prompt(40, 10);
    let tok = encode_pasted_text("first\nsecond\nthird");
    p.set_text(&tok);
    let parsed = &folded_tokens(p.text())[0];
    let rev = p.revision();
    assert!(p.is_saved());
    assert_eq!(p.visual_rows().len(), 1);

    // Expand: visual rows grow to 3, but revision and is_saved stay unchanged.
    assert!(p.toggle_paste(parsed.id, 0));
    assert_eq!(p.visual_rows().len(), 3);
    assert_eq!(p.revision(), rev);
    assert!(p.is_saved());

    // Single click without drag inside expanded preview collapses it back.
    assert!(p.begin_preview_selection(1, 3, 1));
    assert!(p.finish_preview_selection());
    assert!(!p.has_expanded_pastes());
    assert_eq!(p.visual_rows().len(), 1);

    // Drag inside expanded preview selects decoded slice without collapsing.
    assert!(p.toggle_paste(parsed.id, 0));
    assert!(p.begin_preview_selection(1, 0, 1)); // row 1 ("second"), col 0..2
    assert!(p.drag_preview_selection(1, 2));
    assert!(p.finish_preview_selection());
    assert!(p.has_expanded_pastes());
    let sel = p.preview_selection().expect("preview selection");
    assert_eq!(p.projection().copy_display_range(sel.0..sel.1), "sec");

    // Reloading via set_text resets expanded state.
    p.set_text(&tok);
    assert!(!p.has_expanded_pastes());
    assert!(p.preview_selection().is_none());
}

#[test]
fn token_body_with_template_or_fence_markers_does_not_corrupt_block_scan() {
    let mut p = prompt(60, 12);
    let tok = encode_pasted_text(
        "+++ supplement --end\n+++ supplement --start\n{title: injected}\n```rust\nlet x = 1;",
    );
    p.set_text(&format!(
        "+++ supplement --start\n{{title: real}}\n{tok}\n+++ supplement --end"
    ));

    let spans =
        crate::app::markdown::block_scan(&crate::app::paste::mask_pasted_text(p.text())).spans;
    assert_eq!(
        spans.len(),
        1,
        "Token body markers must not split or open extra blocks"
    );
    assert!(spans[0].closed);
}
