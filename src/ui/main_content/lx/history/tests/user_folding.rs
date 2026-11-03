//! 用户消息正文五行折叠与引用树渲染测试。

use super::*;
use crate::db::chat::{MessageAttachmentKind, MessageAttachmentMeta};

fn user_msg_with_text(id: &str, content: &str) -> ChatMessageView {
    ChatMessageView {
        sequence: 0,
        id: id.into(),
        kind: "chat".into(),
        role: "user".into(),
        status: "complete".into(),
        turn_index: 0,
        shell: None,
        steps: vec![AssistantStepView::Part {
            position: 0,
            part: ChatPartView {
                kind: PartKind::Text,
                content: content.into(),
                provider_metadata_json: None,
                started_at_ms: None,
                ended_at_ms: None,
                completed_at_ms: None,
            },
        }],
        model_label: None,
        error_code: None,
        input_tokens: None,
        output_tokens: None,
        cache_read_tokens: None,
        duration_ms: None,
        reports: Vec::new(),
        attachments: Vec::new(),
    }
}

#[test]
fn user_message_5_lines_or_less_not_folded() {
    let msg = user_msg_with_text("u1", "line 1\nline 2\nline 3\nline 4\nline 5");
    let lx = lx_with(vec![msg]);
    let mut cache = cache();
    let snap = cache.snapshot(80, &lx);

    assert_eq!(snap.entries[0].blocks[0].rows, 5);
    assert!(
        !snap
            .cards
            .iter()
            .any(|c| matches!(c.kind, CardKind::UserMessage { .. }))
    );
}

#[test]
fn user_message_6_lines_folded_to_5_plus_more_lines_hint() {
    let msg = user_msg_with_text("u1", "1\n2\n3\n4\n5\n6");
    let lx = lx_with(vec![msg]);
    let mut cache = cache();
    let snap = cache.snapshot(80, &lx);

    // 默认折叠态：5 行正文 + 1 行 more lines
    assert_eq!(snap.entries[0].blocks[0].rows, 6);
    let card = snap
        .cards
        .iter()
        .find(|c| matches!(c.kind, CardKind::UserMessage { .. }))
        .expect("more lines card exists");
    assert_eq!(card.card_start, 5);
    assert_eq!(card.card_rows, 1);
    assert_eq!(
        card.kind,
        CardKind::UserMessage {
            message_id: "u1".into(),
            expanded: false
        }
    );
}

#[test]
fn user_message_expands_to_all_lines_plus_less_lines_hint() {
    let msg = user_msg_with_text("u1", "1\n2\n3\n4\n5\n6\n7");
    let mut lx = lx_with(vec![msg]);
    lx.chat.expanded_user_messages.insert("u1".into());

    let mut cache = cache();
    let snap = cache.snapshot(80, &lx);

    // 展开态：7 行正文 + 1 行 less lines
    assert_eq!(snap.entries[0].blocks[0].rows, 8);
    let card = snap
        .cards
        .iter()
        .find(|c| matches!(c.kind, CardKind::UserMessage { .. }))
        .expect("less lines card exists");
    assert_eq!(card.card_start, 7);
    assert_eq!(card.card_rows, 1);
    assert_eq!(
        card.kind,
        CardKind::UserMessage {
            message_id: "u1".into(),
            expanded: true
        }
    );

    // 切换折叠
    lx.chat.toggle_user_message_expanded("u1");
    let snap_collapsed = cache.snapshot(80, &lx);
    assert_eq!(snap_collapsed.entries[0].blocks[0].rows, 6);
}

#[test]
fn user_message_renders_reference_tree_without_user_background() {
    let mut msg = user_msg_with_text("u1", "Check these files:");
    msg.attachments = vec![
        MessageAttachmentMeta {
            id: "att-1".into(),
            position: 0,
            kind: MessageAttachmentKind::File,
            label: "src/main.rs".into(),
            marker_start: None,
            marker_end: None,
            source_path: Some("src/main.rs".into()),
            mime: None,
            width: None,
            height: None,
        },
        MessageAttachmentMeta {
            id: "att-2".into(),
            position: 1,
            kind: MessageAttachmentKind::Folder,
            label: "src/ui/".into(),
            marker_start: None,
            marker_end: None,
            source_path: Some("src/ui".into()),
            mime: None,
            width: None,
            height: None,
        },
        MessageAttachmentMeta {
            id: "att-3".into(),
            position: 2,
            kind: MessageAttachmentKind::Image,
            label: "error.png".into(),
            marker_start: None,
            marker_end: None,
            source_path: Some("error.png".into()),
            mime: Some("image/png".into()),
            width: Some(100),
            height: Some(50),
        },
    ];

    let lx = lx_with(vec![msg]);
    let mut cache = cache();
    let snap = cache.snapshot(60, &lx);

    // 1 行正文 + 3 行引用 = 4 行
    assert_eq!(snap.entries[0].blocks[0].rows, 4);
    // 引用行不产生 CardRegion
    assert!(snap.cards.is_empty());

    let area = Rect::new(0, 0, 60, 6);
    let mut grid = Grid::new(area);
    let _ = render(&mut grid, area, &lx, 0, &mut cache);

    // 行 0 是正文：铺设 chat_user_bg
    let cell_line0 = grid.cell(0, 0);
    assert_eq!(cell_line0.style.bg, style::chat_user_bg().bg);

    // 行 1 是非末项引用：`  ├─ File: src/main.rs`
    let cell_conn1 = grid.cell(2, 1);
    assert_eq!(cell_conn1.ch, '├');
    assert_eq!(cell_conn1.style.bg, None); // 无用户背景！
    assert_eq!(grid.cell(5, 1).ch, 'F');
    assert_eq!(grid.cell(5, 1).style.fg, None); // `File: ` 用默认前景色
    let label1_cell = grid.cell(11, 1);
    assert_eq!(label1_cell.ch, 's');
    assert_eq!(label1_cell.style.fg, Some(Color::LightGreen));

    // 行 2 是文件夹引用：`  ├─ Folder: src/ui/`
    assert_eq!(grid.cell(2, 2).ch, '├');
    assert_eq!(grid.cell(5, 2).ch, 'F');
    assert_eq!(grid.cell(5, 2).style.fg, None); // `Folder: ` 用默认前景色
    let label2_cell = grid.cell(13, 2);
    assert_eq!(label2_cell.ch, 's');
    assert_eq!(label2_cell.style.fg, Some(Color::Indexed(208)));

    // 行 3 是末项图片引用：`  └─ Image: error.png`
    let cell_conn3 = grid.cell(2, 3);
    assert_eq!(cell_conn3.ch, '└');
    assert_eq!(cell_conn3.style.bg, None); // 无用户背景！
    assert_eq!(grid.cell(5, 3).ch, 'I');
    assert_eq!(grid.cell(5, 3).style.fg, None); // `Image: ` 用默认前景色
    let label3_cell = grid.cell(12, 3);
    assert_eq!(label3_cell.ch, 'e');
    assert_eq!(label3_cell.style.fg, Some(Color::LightMagenta));
}

#[test]
fn user_message_filters_ref_content_from_reference_tree_at_bottom() {
    let mut msg = user_msg_with_text("u1", "Check this: @[ref-content]/src/ui/");
    msg.attachments = vec![
        MessageAttachmentMeta {
            id: "att-1".into(),
            position: 0,
            kind: MessageAttachmentKind::File,
            label: "src/main.rs".into(),
            marker_start: None,
            marker_end: None,
            source_path: Some("src/main.rs".into()),
            mime: None,
            width: None,
            height: None,
        },
        MessageAttachmentMeta {
            id: "att-2".into(),
            position: 1,
            kind: MessageAttachmentKind::Folder,
            label: "@[ref-content]/src/ui/".into(),
            marker_start: Some(12),
            marker_end: Some(37),
            source_path: Some("src/ui".into()),
            mime: None,
            width: None,
            height: None,
        },
    ];

    let lx = lx_with(vec![msg]);
    let mut cache = cache();
    let snap = cache.snapshot(60, &lx);

    // 正文 1 行（保留 @[ref-content]/src/ui/）+ 仅 1 行常规附件（src/main.rs），过滤掉 ref-content = 总共 2 行
    assert_eq!(snap.entries[0].blocks[0].rows, 2);
}

#[test]
fn user_message_folded_paste_chip_renders_and_toggles_independently() {
    let tok =
        crate::app::paste::encode_pasted_text("alpha\nbeta\ngamma\ndelta\nepsilon\nzeta\neta");
    let paste_id = crate::app::paste::folded_tokens(&tok)[0].id;
    let msg = user_msg_with_text("u-paste", &tok);
    let mut lx = lx_with(vec![msg]);
    let mut cache = cache();

    // 1. Collapsed state: 1 row (`[Pasted 7 lines: alpha]`), registers `CardKind::UserPaste`.
    let snap = cache.snapshot(60, &lx);
    assert_eq!(snap.entries[0].blocks[0].rows, 1);
    let card = snap
        .cards
        .iter()
        .find(|c| matches!(c.kind, CardKind::UserPaste { .. }))
        .expect("UserPaste card region");
    assert_eq!(
        card.kind,
        CardKind::UserPaste {
            message_id: "u-paste".into(),
            paste_id,
            occurrence: 0,
            expanded: false,
        }
    );

    let area = Rect::new(0, 0, 60, 10);
    let mut grid = Grid::new(area);
    let _ = render(&mut grid, area, &lx, 0, &mut cache);
    // Column 2 (after `> `) starts `[Pasted 7 lines: alpha]` with Yellow BOLD on Indexed(236).
    let chip_cell = grid.cell(2, 0);
    assert_eq!(chip_cell.ch, '[');
    assert_eq!(chip_cell.style.fg, Some(Color::Yellow));
    assert_eq!(chip_cell.style.bg, Some(Color::Indexed(236)));
    assert!(
        chip_cell
            .style
            .add_modifier
            .contains(ratatui::style::Modifier::BOLD)
    );

    // 2. Toggle paste chip to expanded: shows all 7 lines (not cut off by 5-line rule).
    assert!(lx.chat.toggle_user_paste("u-paste", paste_id, 0));
    let snap_exp = cache.snapshot(60, &lx);
    assert_eq!(snap_exp.entries[0].blocks[0].rows, 7);
    let card_exp = snap_exp
        .cards
        .iter()
        .find(|c| matches!(c.kind, CardKind::UserPaste { .. }))
        .expect("expanded UserPaste card region");
    assert_eq!(card_exp.card_rows, 7);
    assert_eq!(
        card_exp.kind,
        CardKind::UserPaste {
            message_id: "u-paste".into(),
            paste_id,
            occurrence: 0,
            expanded: true,
        }
    );

    let mut grid_exp = Grid::new(area);
    let _ = render(&mut grid_exp, area, &lx, 0, &mut cache);
    // Column 2 (right after `> `) starts 'a' of "alpha" on Indexed(236) background.
    assert_eq!(grid_exp.cell(2, 0).ch, 'a');
    assert_eq!(grid_exp.cell(2, 0).style.bg, Some(Color::Indexed(236)));

    // 3. Toggle again collapses back to 1 row.
    assert!(!lx.chat.toggle_user_paste("u-paste", paste_id, 0));
    let snap_col = cache.snapshot(60, &lx);
    assert_eq!(snap_col.entries[0].blocks[0].rows, 1);
}
