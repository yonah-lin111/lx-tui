//! 补充块折叠：默认折叠、换行落点、写入保护、导航与状态往返。

use super::*;
use crate::layout::PaneId;

const TASK: &str = "&&& x --start\n+++ supplement --start\n{title: Review}\nbody\n+++ supplement --end\n&&& x --end";

fn prompt(text: &str) -> Prompt {
    let mut prompt = Prompt::new(PaneId::alloc());
    prompt.resize(40, 10);
    prompt.set_text(text);
    prompt
}

fn offset_of(text: &str, needle: &str) -> usize {
    text.find(needle).expect("needle present")
}

fn block_start_end(text: &str) -> (usize, usize) {
    let start = offset_of(text, "+++ supplement --start");
    (start, start + "+++ supplement --start".len())
}

#[test]
fn closed_supplement_is_folded_by_default() {
    let prompt = prompt(TASK);
    let rows = prompt.visual_rows();
    let lines: Vec<usize> = rows.iter().map(|row| row.line).collect();
    assert_eq!(lines, vec![0, 1, 2, 5], "标题行不折叠");
    let folded = prompt.folded_blocks();
    assert_eq!(folded.len(), 1);
    assert_eq!(folded[0].start_line, 1);
    assert_eq!(folded[0].end_line, 4);
    assert_eq!(folded[0].hidden_from_line, 3);
}

#[test]
fn toggle_expands_and_collapses_without_dirty_or_history() {
    let mut prompt = prompt(TASK);
    assert!(prompt.toggle_supple_fold(1));
    assert_eq!(prompt.visual_rows().len(), 6);
    assert!(prompt.is_saved());
    assert!(!prompt.can_undo());
    assert!(prompt.toggle_supple_fold(1));
    assert_eq!(prompt.visual_rows().len(), 4);
    assert!(!prompt.toggle_supple_fold(0));
}

#[test]
fn enter_on_folded_line_breaks_after_block() {
    let mut prompt = prompt(TASK);
    let text = prompt.text().to_string();
    let (_, end) = block_start_end(&text);
    prompt.set_cursor_offset_for_test(end);
    prompt.insert_char('\n');
    assert!(
        prompt
            .text()
            .contains("+++ supplement --end\n\n&&& x --end")
    );
    let cursor = prompt.cursor();
    assert!(prompt.text()[cursor..].starts_with("\n&&& x --end"));
    assert_eq!(prompt.folded_blocks().len(), 1);
}

#[test]
fn enter_on_folded_last_line_appends_after_block() {
    let text = "&&& x --start\n+++ supplement --start\nbody\n+++ supplement --end";
    let mut prompt = prompt(text);
    let (_, end) = block_start_end(text);
    prompt.set_cursor_offset_for_test(end);
    prompt.insert_char('\n');
    assert!(prompt.text().ends_with("+++ supplement --end\n"));
    assert_eq!(prompt.cursor(), prompt.text().len());
}

#[test]
fn enter_mid_header_goes_after_block() {
    let mut prompt = prompt(TASK);
    let text = prompt.text().to_string();
    let (start, _) = block_start_end(&text);
    prompt.set_cursor_offset_for_test(start + 5);
    prompt.insert_char('\n');
    assert!(
        prompt
            .text()
            .contains("+++ supplement --end\n\n&&& x --end")
    );
    assert!(
        prompt
            .text()
            .contains("+++ supplement --start\n{title: Review}"),
        "不拆结构标记"
    );
}

#[test]
fn enter_at_header_start_inserts_before_block() {
    let mut prompt = prompt(TASK);
    let text = prompt.text().to_string();
    let (start, _) = block_start_end(&text);
    prompt.set_cursor_offset_for_test(start);
    prompt.insert_char('\n');
    assert!(
        prompt
            .text()
            .contains("&&& x --start\n\n+++ supplement --start")
    );
}

#[test]
fn typing_at_folded_boundary_is_rejected() {
    let mut prompt = prompt(TASK);
    let text = prompt.text().to_string();
    let end = offset_of(&text, "{title: Review}") + "{title: Review}".len();
    prompt.set_cursor_offset_for_test(end);
    prompt.insert_char('x');
    assert_eq!(prompt.text(), text);
    assert!(prompt.take_fold_blocked());
    assert!(!prompt.take_fold_blocked());
}

#[test]
fn navigation_skips_hidden_lines() {
    let mut prompt = prompt(TASK);
    let text = prompt.text().to_string();
    let (start, _) = block_start_end(&text);
    let title = offset_of(&text, "{title: Review}");
    let title_end = title + "{title: Review}".len();
    let after = offset_of(&text, "&&& x --end");
    prompt.set_cursor_offset_for_test(start);
    prompt.move_down();
    assert_eq!(prompt.cursor(), title, "折叠后下移落到可见标题行");
    prompt.set_cursor_offset_for_test(title_end);
    prompt.move_right();
    assert_eq!(prompt.cursor(), after, "标题行之后是整块之后");
    prompt.move_left();
    assert_eq!(prompt.cursor(), title_end);
}

#[test]
fn delete_across_folded_block_is_rejected() {
    let mut prompt = prompt(TASK);
    let text = prompt.text().to_string();
    let at = offset_of(&text, "x --start") + 2;
    let after = offset_of(&text, "&&& x --end") + 3;
    assert!(!prompt.replace_range(at, after, ""));
    assert_eq!(prompt.text(), text);
    assert!(prompt.take_fold_blocked());
}

#[test]
fn expansion_survives_edits_before_block() {
    let mut prompt = prompt(TASK);
    assert!(prompt.toggle_supple_fold(1));
    prompt.set_cursor_offset_for_test(0);
    prompt.insert_str("pre\n");
    assert_eq!(prompt.visual_rows().len(), 7);
    assert_eq!(prompt.folded_blocks().len(), 0);
}

#[test]
fn undo_restores_fold_state_with_text() {
    let mut prompt = prompt(TASK);
    assert!(prompt.toggle_supple_fold(1));
    let body = offset_of(prompt.text(), "body");
    prompt.set_cursor_offset_for_test(body);
    prompt.insert_str("more ");
    prompt.undo();
    assert_eq!(prompt.text(), TASK);
    assert_eq!(prompt.visual_rows().len(), 6);
}

#[test]
fn history_roundtrip_keeps_expansion() {
    let mut prompt = prompt(TASK);
    assert!(prompt.toggle_supple_fold(1));
    let history = prompt.history();
    let mut restored = Prompt::new(PaneId::alloc());
    restored.resize(40, 10);
    restored.restore_history(history);
    assert_eq!(restored.visual_rows().len(), 6);
    assert_eq!(restored.folded_blocks().len(), 0);
}

#[test]
fn selection_text_skips_folded_content() {
    let prompt = prompt(TASK);
    let copied = prompt.selection_text((0, 0), (3, 5));
    assert_eq!(
        copied.as_deref(),
        Some("&&& x --start\n+++ supplement --start\n{title: Review}\n&&& ")
    );
}

#[test]
fn unclosed_supplement_stays_visible() {
    let text = "&&& x --start\n+++ supplement --start\nbody\n&&& x --end";
    let prompt = prompt(text);
    assert!(prompt.folded_blocks().is_empty());
    assert_eq!(prompt.visual_rows().len(), 4);
}

#[test]
fn collapsing_moves_cursor_out_of_hidden_body() {
    let mut prompt = prompt(TASK);
    assert!(prompt.toggle_supple_fold(1));
    let body = offset_of(prompt.text(), "body");
    prompt.set_cursor_offset_for_test(body);
    assert!(prompt.toggle_supple_fold(1));
    let title_end = offset_of(prompt.text(), "{title: Review}") + "{title: Review}".len();
    assert_eq!(prompt.cursor(), title_end);
}
