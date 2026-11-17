use super::*;
use crate::app::paste::{
    FoldedKind, TextPasteAction, encode_pasted_text, encode_report, folded_tokens,
};

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
fn report_chip_inserts_at_top_with_blank_line_and_keeps_draft() {
    let mut lx = Lx::new();
    lx.insert_str("先看登录流程");
    let cursor_before = lx.cursor;
    assert!(lx.insert_report_chip(
        FoldedKind::SubagentReport,
        "agent-1",
        "[Subagent report] Curie\nbody"
    ));
    let token = encode_report(
        FoldedKind::SubagentReport,
        "agent-1",
        "[Subagent report] Curie\nbody",
    );
    assert_eq!(lx.input, format!("{token}\n\n先看登录流程"));
    assert_eq!(lx.cursor, cursor_before + token.len() + 2);
    let parsed = folded_tokens(&lx.input);
    assert_eq!(parsed.len(), 1);
    assert_eq!(parsed[0].kind, FoldedKind::SubagentReport);
    assert_eq!(parsed[0].source.as_deref(), Some("agent-1"));
    assert_eq!(
        lx.expanded_input(),
        "[Subagent report] Curie\nbody\n\n先看登录流程"
    );

    // 一次撤销即恢复原草稿。
    lx.undo();
    assert_eq!(lx.input, "先看登录流程");
}

/// Shell 模式下报告正文不能进入输入框：Enter 会把整段报告当命令执行。
#[test]
fn report_chip_is_rejected_in_shell_mode() {
    let mut lx = Lx::new();
    lx.insert_char('!');
    assert!(lx.shell_mode());
    assert!(!lx.insert_report_chip(FoldedKind::SubagentReport, "agent-1", "body"));
    assert!(lx.input.is_empty(), "只留下 shell 前缀，不写报告");
}

/// 切会话：报告芯片属于原会话，必须从草稿中剥除且不留在撤销栈。
#[test]
fn strip_report_chips_removes_all_report_tokens_and_keeps_draft() {
    let mut lx = Lx::new();
    lx.insert_str("keep me");
    assert!(lx.insert_report_chip(FoldedKind::SubagentReport, "agent-1", "one"));
    assert!(lx.insert_report_chip(FoldedKind::BtwReport, "thread-1", "two"));
    assert!(lx.strip_report_chips());
    assert_eq!(lx.input, "keep me");
    assert!(folded_tokens(&lx.input).is_empty());
    assert!(lx.undo.is_empty(), "旧会话报告不得通过撤销回来");
}

#[test]
fn report_chip_replaces_same_source_in_place_and_keeps_other_sources() {
    let mut lx = Lx::new();
    lx.insert_str("draft");
    assert!(lx.insert_report_chip(FoldedKind::SubagentReport, "agent-1", "old"));
    assert!(lx.insert_report_chip(FoldedKind::BtwReport, "thread-1", "btw body"));
    let before = folded_tokens(&lx.input);
    assert_eq!(before.len(), 2);
    assert_eq!(before[0].kind, FoldedKind::BtwReport, "新报告在顶部");

    assert!(lx.insert_report_chip(FoldedKind::SubagentReport, "agent-1", "new body"));
    let after = folded_tokens(&lx.input);
    assert_eq!(after.len(), 2, "同源只保留一个芯片");
    let report = after
        .iter()
        .find(|tok| tok.kind == FoldedKind::SubagentReport)
        .expect("report chip");
    assert_eq!(report.body, "new body");
    assert_eq!(report.source.as_deref(), Some("agent-1"));
    assert!(lx.input.ends_with("\n\ndraft"));
}

#[test]
fn report_chip_expands_for_model_and_can_be_deleted_atomically() {
    let mut lx = Lx::new();
    assert!(lx.insert_report_chip(FoldedKind::BtwReport, "thread-1", "[BTW report] T\nanswer"));
    assert_eq!(lx.expanded_input().trim_end(), "[BTW report] T\nanswer");
    let token = folded_tokens(&lx.input).remove(0);
    lx.cursor = token.range.end;
    lx.backspace();
    assert!(lx.input.trim().is_empty(), "芯片整块删除，仅留分隔空行");
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
