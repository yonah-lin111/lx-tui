//! lx 输入框粘贴单元测试：净化、撤销步、面板互斥与参照面板状态机。

use std::path::PathBuf;

use super::*;
use crate::app::paste::PasteCandidateKind;

/// 一个存在的目录候选（引用可无损表达）。
fn folder(path: &str) -> PasteCandidate {
    PasteCandidate::external(PathBuf::from(path), PasteCandidateKind::Folder)
}

#[test]
fn insert_str_sanitizes_line_endings_tabs_and_controls() {
    let mut lx = Lx::new();
    lx.insert_str("a\r\nb\tc\x07d");
    assert_eq!(lx.input, "a\nb  cd");
    assert_eq!(lx.cursor, lx.input.len(), "光标落在插入文本末尾");
}

#[test]
fn insert_str_is_a_single_undo_step() {
    let mut lx = Lx::new();
    lx.insert_str("hello\nworld");
    lx.undo();
    assert_eq!(lx.input, "", "整段粘贴一步撤销");
    lx.redo();
    assert_eq!(lx.input, "hello\nworld");
}

#[test]
fn insert_str_inserts_at_cursor() {
    let mut lx = Lx::new();
    lx.insert_str("ab");
    lx.move_left();
    lx.insert_str("X");
    assert_eq!(lx.input, "aXb");
    assert_eq!(lx.cursor, 2);
}

#[test]
fn insert_str_empty_after_sanitize_keeps_no_undo_step() {
    let mut lx = Lx::new();
    lx.insert_str("\x07\x07");
    assert_eq!(lx.input, "");
    lx.undo();
    assert_eq!(lx.input, "", "没有产生撤销步");
}

#[test]
fn insert_str_suppresses_markdown_block_panel() {
    let mut lx = Lx::new();
    lx.insert_str("##");
    assert!(lx.panel.is_none(), "粘贴不弹 md 块面板");
    // 触发标记确实存在，只是被粘贴压制；下一次编辑才刷新。
    lx.refresh_panel();
    assert!(lx.panel.is_some(), "行尾 ## 本是块触发标记");
}

#[test]
fn paste_open_inserts_padded_path_placeholder_and_opens_panel() {
    let mut lx = Lx::new();
    lx.insert_char('x');
    assert!(lx.paste_open(vec![folder("/tmp/dir")]));
    assert_eq!(lx.input, "x /tmp/dir ", "占位路径带前后空格");
    assert_eq!(lx.cursor, lx.input.len());
    let panel = lx.paste_panel().expect("panel");
    assert!(panel.reference_enabled());
    assert_eq!(panel.active_action(), Some(PasteAction::Reference));
    assert!(lx.panel.is_none(), "粘贴面板与 md 块面板互斥");
}

#[test]
fn paste_open_empty_candidates_is_noop() {
    let mut lx = Lx::new();
    assert!(!lx.paste_open(Vec::new()));
    assert_eq!(lx.input, "");
    assert!(lx.paste_panel().is_none());
}

#[test]
fn paste_move_cycles_to_path_action() {
    let mut lx = Lx::new();
    lx.paste_open(vec![folder("/tmp/dir")]);
    assert!(lx.paste_move(1));
    assert_eq!(
        lx.paste_panel().and_then(PastePanel::active_action),
        Some(PasteAction::Path)
    );
}

#[test]
fn paste_confirm_reference_rewrites_in_same_undo_step() {
    let mut lx = Lx::new();
    lx.paste_open(vec![folder("/tmp/dir")]);
    assert!(lx.paste_confirm(PasteAction::Reference));
    assert_eq!(lx.input, "@[refer-folder](/tmp/dir) ");
    assert!(lx.paste_panel().is_none());
    lx.undo();
    assert_eq!(lx.input, "", "占位插入与引用改写同属一步撤销");
}

#[test]
fn paste_confirm_path_keeps_placeholder_text() {
    let mut lx = Lx::new();
    lx.paste_open(vec![folder("/tmp/dir")]);
    assert!(lx.paste_confirm(PasteAction::Path));
    assert_eq!(lx.input, "/tmp/dir ");
    assert!(lx.paste_panel().is_none());
}

#[test]
fn paste_cancel_rolls_back_without_residual_undo_step() {
    let mut lx = Lx::new();
    lx.insert_char('a');
    lx.paste_open(vec![folder("/tmp/dir")]);
    assert!(lx.paste_cancel());
    assert_eq!(lx.input, "a", "取消回滚占位插入");
    assert!(lx.paste_panel().is_none());
    lx.undo();
    assert_eq!(lx.input, "", "回滚不残留粘贴撤销步");
}

#[test]
fn paste_cancel_without_panel_is_false() {
    let mut lx = Lx::new();
    assert!(!lx.paste_cancel());
}

#[test]
fn visual_rows_wrap_at_content_width() {
    let mut lx = Lx::new();
    lx.resize(4, 100);
    lx.insert_str("abcdef");
    let rows = lx.visual_rows();
    assert_eq!(rows.len(), 2);
    assert_eq!(&lx.input[rows[0].start..rows[0].end], "abcd");
    assert_eq!(&lx.input[rows[1].start..rows[1].end], "ef");
}

#[test]
fn default_width_does_not_wrap() {
    let mut lx = Lx::new();
    lx.insert_str("a very long single line that should stay on one row");
    assert_eq!(lx.visual_rows().len(), 1);
}

#[test]
fn move_up_down_cross_wrapped_rows() {
    let mut lx = Lx::new();
    lx.resize(2, 100);
    lx.insert_str("abcd");
    assert_eq!(lx.cursor_visual(), (1, 2));
    lx.move_up();
    assert_eq!(lx.cursor, 2, "上移到第一折行行尾");
    lx.move_down();
    assert_eq!(lx.cursor, 4, "下移回末折行行尾");
}

#[test]
fn home_end_use_visual_rows() {
    let mut lx = Lx::new();
    lx.resize(2, 100);
    lx.insert_str("abcd");
    lx.home();
    assert_eq!(lx.cursor, 2, "当前视觉行行首为第二折行起点");
    lx.end();
    assert_eq!(lx.cursor, 4, "当前视觉行行尾");
}

#[test]
fn cursor_visual_keeps_display_column_across_wrap() {
    let mut lx = Lx::new();
    lx.resize(3, 100);
    lx.insert_str("ab你谁");
    // 宽字符按 2 列计：视觉行为 "ab"(0..2)、"你"(2..5)、"谁"(5..8)。
    let rows = lx.visual_rows();
    assert_eq!(&lx.input[rows[1].start..rows[1].end], "你");
    lx.cursor = rows[1].start;
    assert_eq!(lx.cursor_visual(), (1, 0));
}

#[test]
fn text_selection_range_and_offset_mapping() {
    let mut lx = Lx::new();
    lx.resize(10, 5);
    lx.insert_str("hello");
    assert_eq!(lx.offset_at_cell(0, 2), 2);
    lx.begin_text_selection(0, 1);
    assert_eq!(lx.cursor, 1);
    lx.drag_text_selection(0, 4);
    assert_eq!(lx.text_selection_range(), Some((1, 4)));
    assert_eq!(lx.selected_text().as_deref(), Some("ell"));
    assert!(lx.has_text_selection());
    lx.clear_text_selection();
    assert!(!lx.has_text_selection());
}

#[test]
fn scroll_by_clamps_to_content() {
    let mut lx = Lx::new();
    lx.resize(2, 2);
    lx.insert_str("abcdef");
    // 三视觉行、可视两行：光标在末尾使视口贴底。
    assert_eq!(lx.visual_rows().len(), 3);
    assert_eq!(lx.scroll(), 1);
    assert!(!lx.scroll_by(1), "已在底部，向下不移动");
    assert!(lx.scroll_by(-1));
    assert_eq!(lx.scroll(), 0);
    assert!(!lx.scroll_by(-1), "已在顶部，向上不移动");
}

#[test]
fn resize_clamps_scroll_back_into_view() {
    let mut lx = Lx::new();
    lx.resize(2, 1);
    lx.insert_str("abcd");
    assert_eq!(lx.scroll(), 1, "单行可视为滚动到第二折行");
    lx.resize(2, 10);
    assert_eq!(lx.scroll(), 0, "视口变高后回钳到顶部");
}

// ===== 历史 prompt 切换 =====

/// 三条历史（旧 → 新），末尾为最近使用。
fn entries() -> Vec<String> {
    vec!["one".to_string(), "two".to_string(), "three".to_string()]
}

#[test]
fn history_up_from_empty_input_takes_newest_entry() {
    let mut lx = Lx::new();
    assert!(lx.pending_history_move(&entries(), -1).is_some());
    lx.apply_history(lx.pending_history_move(&entries(), -1).expect("接管"));
    assert_eq!(lx.input, "three");
    assert_eq!(lx.cursor, 0, "`↑` 恢复后光标在文首，才能继续向上翻");
    assert!(lx.history_browsing());
}

#[test]
fn history_up_walks_back_and_clamps_at_oldest() {
    let mut lx = Lx::new();
    for _ in 0..3 {
        let next = lx.pending_history_move(&entries(), -1).expect("接管");
        lx.apply_history(next);
    }
    assert_eq!(lx.input, "one");
    // 已到最旧一条：继续按 `↑` 不接管，回落光标移动。
    assert_eq!(lx.pending_history_move(&entries(), -1), None);
}

#[test]
fn history_down_walks_forward_and_restores_draft() {
    let mut lx = Lx::new();
    lx.insert_str("my draft");
    lx.cursor = 0;
    // 首次 `↑` 暂存草稿并显示最新一条（光标落文首）。
    let next = lx.pending_history_move(&entries(), -1).expect("接管");
    lx.apply_history(next);
    assert_eq!(lx.input, "three");
    assert_eq!(lx.cursor, 0);
    // `↑` 继续向前翻到上一条。
    let next = lx.pending_history_move(&entries(), -1).expect("接管");
    lx.apply_history(next);
    assert_eq!(lx.input, "two");
    // 光标不在文末时 `↓` 不归历史：先移动到文末（单行钳到行尾）。
    assert_eq!(lx.pending_history_move(&entries(), 1), None);
    lx.move_down();
    assert_eq!(lx.cursor, lx.input.len(), "单行文本按 `↓` 钳到行尾");
    // 光标在文末后 `↓` 接管：前进到更新的一条。
    let next = lx.pending_history_move(&entries(), 1).expect("接管");
    lx.apply_history(next);
    assert_eq!(lx.input, "three");
    // 再 `↓` 越过最新一条：恢复进入浏览前的草稿。
    lx.cursor = lx.input.len();
    let next = lx.pending_history_move(&entries(), 1).expect("接管");
    lx.apply_history(next);
    assert_eq!(lx.input, "my draft");
    assert_eq!(lx.cursor, lx.input.len(), "恢复草稿后光标在末尾");
    assert!(!lx.history_browsing());
}

#[test]
fn history_down_without_browsing_is_not_taken() {
    let lx = Lx::new();
    assert_eq!(lx.pending_history_move(&entries(), 1), None);
}

#[test]
fn history_requires_cursor_at_boundary() {
    let mut lx = Lx::new();
    lx.insert_str("multi\nline");
    // 光标在中间：`↑`/`↓` 都不归历史。
    lx.cursor = 2;
    assert_eq!(lx.pending_history_move(&entries(), -1), None);
    assert_eq!(lx.pending_history_move(&entries(), 1), None);
    // 光标在文首：只有 `↑` 接管。
    lx.cursor = 0;
    assert!(lx.pending_history_move(&entries(), -1).is_some());
    assert_eq!(lx.pending_history_move(&entries(), 1), None);
    // 光标在文末：只有 `↓` 接管（此时不在浏览态，`↓` 仍不接管）。
    lx.cursor = lx.input.len();
    assert_eq!(lx.pending_history_move(&entries(), -1), None);
}

#[test]
fn history_empty_is_never_taken() {
    let lx = Lx::new();
    assert_eq!(lx.pending_history_move(&[], -1), None);
    assert_eq!(lx.pending_history_move(&[], 1), None);
}

#[test]
fn end_history_browse_keeps_text() {
    let mut lx = Lx::new();
    let next = lx.pending_history_move(&entries(), -1).expect("接管");
    lx.apply_history(next);
    assert_eq!(lx.input, "three");
    lx.end_history_browse();
    assert!(!lx.history_browsing());
    assert_eq!(lx.input, "three", "退出浏览不改文本");
}

#[test]
fn history_restore_is_undoable() {
    let mut lx = Lx::new();
    lx.insert_str("my draft");
    lx.cursor = 0;
    let next = lx.pending_history_move(&entries(), -1).expect("接管");
    lx.apply_history(next);
    assert_eq!(lx.input, "three");
    lx.undo();
    assert_eq!(lx.input, "my draft", "恢复历史压了一次撤销快照");
}

#[test]
fn history_same_text_does_not_push_undo_step() {
    let mut lx = Lx::new();
    // 输入框内容正好等于历史末尾条目。
    lx.insert_str("three");
    lx.cursor = 0;
    let next = lx.pending_history_move(&entries(), -1).expect("接管");
    lx.apply_history(next);
    assert_eq!(lx.input, "three");
    assert_eq!(lx.cursor, 0);
    // 撤销应回到空输入（历史恢复没有额外产生撤销步）。
    lx.undo();
    assert_eq!(lx.input, "");
}

#[test]
fn add_draft_image_and_caps_at_four() {
    let mut lx = Lx::new();
    for i in 0..4 {
        let processed = crate::platform::ProcessedImage {
            source_name: format!("img{i}.png"),
            source_path: None,
            mime: "image/png".into(),
            width: 100,
            height: 100,
            bytes: vec![1, 2, 3],
            original_byte_count: 3,
        };
        assert!(lx.add_draft_image(processed));
    }
    assert_eq!(lx.draft_images.len(), 4);

    // 第五张被拒绝
    let fifth = crate::platform::ProcessedImage {
        source_name: "fifth.png".into(),
        source_path: None,
        mime: "image/png".into(),
        width: 100,
        height: 100,
        bytes: vec![5],
        original_byte_count: 1,
    };
    assert!(!lx.add_draft_image(fifth));
    assert_eq!(lx.draft_images.len(), 4);
}

#[test]
fn undo_redo_restores_draft_images_with_text() {
    let mut lx = Lx::new();
    lx.insert_str("before image");
    let processed = crate::platform::ProcessedImage {
        source_name: "img.png".into(),
        source_path: None,
        mime: "image/png".into(),
        width: 50,
        height: 50,
        bytes: vec![10],
        original_byte_count: 1,
    };
    lx.add_draft_image(processed);
    assert_eq!(lx.draft_images.len(), 1);

    lx.insert_str(" after image");
    assert_eq!(lx.input, "before image after image");

    // 撤销打字
    lx.undo();
    assert_eq!(lx.input, "before image");
    assert_eq!(lx.draft_images.len(), 1);

    // 撤销附图
    lx.undo();
    assert_eq!(lx.draft_images.len(), 0);

    // 重做附图
    lx.redo();
    assert_eq!(lx.draft_images.len(), 1);

    // 重做打字
    lx.redo();
    assert_eq!(lx.input, "before image after image");
}

#[test]
fn remove_draft_image_is_undoable() {
    let mut lx = Lx::new();
    let processed = crate::platform::ProcessedImage {
        source_name: "img.png".into(),
        source_path: None,
        mime: "image/png".into(),
        width: 50,
        height: 50,
        bytes: vec![10],
        original_byte_count: 1,
    };
    lx.add_draft_image(processed);
    assert_eq!(lx.draft_images.len(), 1);

    assert!(lx.remove_draft_image(0));
    assert_eq!(lx.draft_images.len(), 0);

    lx.undo();
    assert_eq!(lx.draft_images.len(), 1);
}

#[test]
fn clear_cleans_draft_images_and_text() {
    let mut lx = Lx::new();
    lx.insert_str("text");
    let processed = crate::platform::ProcessedImage {
        source_name: "img.png".into(),
        source_path: None,
        mime: "image/png".into(),
        width: 50,
        height: 50,
        bytes: vec![10],
        original_byte_count: 1,
    };
    lx.add_draft_image(processed);

    lx.clear();
    assert_eq!(lx.input, "");
    assert_eq!(lx.draft_images.len(), 0);

    // 撤销清空能完整恢复文本与图片
    lx.undo();
    assert_eq!(lx.input, "text");
    assert_eq!(lx.draft_images.len(), 1);
}

#[test]
fn leading_fullwidth_bang_enters_shell_and_strips_prefix() {
    let mut lx = Lx::new();
    lx.insert_str("  ！pwd");
    assert_eq!(lx.input_mode(), LxInputMode::Shell);
    assert_eq!(lx.input, "  pwd");
    assert_eq!(lx.cursor, lx.input.len());
    assert_eq!(lx.serialized_input(), "!  pwd");
}

#[test]
fn bang_later_in_text_is_not_shell() {
    let mut lx = Lx::new();
    lx.insert_str("说明\n！pwd");
    assert_eq!(lx.input_mode(), LxInputMode::Chat);
    assert_eq!(lx.input, "说明\n！pwd");
    lx.insert_str("¥100");
    assert_eq!(lx.input_mode(), LxInputMode::Chat);
    assert!(lx.input.contains('¥'));
    assert!(!lx.input.contains('￥'));
}

#[test]
fn shell_body_keeps_fullwidth_bang_and_currency_symbols() {
    let mut lx = Lx::new();
    lx.insert_str("!echo 好！ ¥HOME");
    assert_eq!(lx.input_mode(), LxInputMode::Shell);
    assert_eq!(lx.input, "echo 好！ ¥HOME");
}

#[test]
fn backspace_at_shell_body_start_exits_mode_and_undo_restores_it() {
    let mut lx = Lx::new();
    lx.insert_str("!pwd");
    lx.cursor = 0;
    lx.backspace();
    assert_eq!(lx.input_mode(), LxInputMode::Chat);
    assert_eq!(lx.input, "pwd");
    lx.undo();
    assert_eq!(lx.input_mode(), LxInputMode::Shell);
    assert_eq!(lx.input, "pwd");
}

#[test]
fn clear_exits_empty_shell_mode_and_revision_never_regresses() {
    let mut lx = Lx::new();
    lx.insert_str("!pwd");
    let revision = lx.revision();
    lx.clear();
    assert_eq!(lx.input_mode(), LxInputMode::Chat);
    assert!(lx.revision() > revision);
    let after_clear = lx.revision();
    lx.undo();
    assert!(lx.revision() > after_clear, "undo 也推进修订号");
}

#[test]
fn shell_mode_never_opens_editing_panels() {
    let mut lx = Lx::new();
    lx.insert_str("!echo @src /model #");
    assert!(lx.panel.is_none());
    assert!(lx.slash.is_none());
    assert!(!lx.mention_visible());
    assert!(!lx.mention_open());
}

#[test]
fn history_recall_restores_shell_mode_and_draft() {
    let mut lx = Lx::new();
    lx.insert_str("my draft");
    let next = HistoryMove::Entry {
        index: 0,
        text: "!pwd".into(),
        cursor_at_start: true,
    };
    lx.apply_history(next);
    assert_eq!(lx.input_mode(), LxInputMode::Shell);
    assert_eq!(lx.input, "pwd");
    // 越过最新一条：恢复进入浏览前的普通草稿。
    lx.apply_history(HistoryMove::Draft);
    assert_eq!(lx.input_mode(), LxInputMode::Chat);
    assert_eq!(lx.input, "my draft");
}

#[test]
fn replace_input_recomputes_shell_mode() {
    let mut lx = Lx::new();
    lx.insert_str("!");
    assert!(lx.shell_mode());
    lx.replace_input("普通提问");
    assert_eq!(lx.input, "普通提问");
    assert!(!lx.shell_mode(), "聊天原文不得留在 Shell 模式");

    lx.replace_input("!pwd");
    assert_eq!(lx.input, "pwd");
    assert!(lx.shell_mode(), "带 ! 前缀的文本恢复 Shell 模式");
}

#[test]
fn exit_shell_is_undoable() {
    let mut lx = Lx::new();
    lx.insert_str("!");
    lx.insert_str("pwd");
    assert!(lx.exit_shell());
    assert!(!lx.shell_mode());
    lx.undo();
    assert!(lx.shell_mode(), "undo 恢复退出前的 Shell 模式");
    assert_eq!(lx.input, "pwd");
}
