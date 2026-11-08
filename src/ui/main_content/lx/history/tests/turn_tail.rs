use ratatui::buffer::Buffer;
use ratatui::layout::Rect;

use super::*;
use crate::app::state::lx::{MessageRecapView, RecapPending};
use crate::app::state::{CardSection, ToolStatus};

fn rows_text(buf: &Buffer, area: Rect) -> Vec<String> {
    (0..area.height)
        .map(|y| {
            (0..area.width)
                .filter_map(|x| buf.cell((x, y)).map(|cell| cell.symbol()))
                .collect::<String>()
        })
        .collect()
}

fn file_summary_message() -> ChatMessageView {
    let mut message = message("assistant", "complete", vec![text_part("done")]);
    message.steps = (1..=6)
        .map(|index| {
            super::tool_geometry::file_step(
                index,
                &format!("call-{index}"),
                "edit",
                ToolStatus::Completed,
            )
        })
        .collect();
    message
}

#[test]
fn file_changes_fold_after_five_and_expand_on_click() {
    let area = Rect::new(0, 0, 80, 60);
    let lx = lx_with(vec![file_summary_message()]);
    let mut buf = Buffer::empty(area);
    crate::ui::main_content::lx::render_history_for_test(area, &mut buf, &lx, 0);
    let text = rows_text(&buf, area).join("\n");
    assert!(text.contains("… 1 more file"), "{text}");
    assert_eq!(text.matches("edit").count(), 5, "默认只显示 5 行：{text}");

    let mut expanded = lx_with(vec![file_summary_message()]);
    assert!(expanded.chat.toggle_section(
        "assistant-1",
        super::file_changes::MORE_SLOT,
        CardSection::Body,
    ));
    let mut buf = Buffer::empty(area);
    crate::ui::main_content::lx::render_history_for_test(area, &mut buf, &expanded, 0);
    let text = rows_text(&buf, area).join("\n");
    assert!(!text.contains("more file"), "{text}");
    assert_eq!(text.matches("edit").count(), 6, "展开后显示全部：{text}");
}

#[test]
fn file_change_row_expands_its_diff_in_place() {
    let area = Rect::new(0, 0, 80, 60);
    let lx = lx_with(vec![file_summary_message()]);
    let before = layout(area, &lx, &mut cache()).total_rows;
    let mut expanded = lx_with(vec![file_summary_message()]);
    assert!(
        expanded
            .chat
            .toggle_section("assistant-1", "file:src/file1.rs", CardSection::Body,)
    );
    let after = layout(area, &expanded, &mut cache()).total_rows;
    assert!(after > before, "展开文件行必须增加 diff 正文行");
    let mut buf = Buffer::empty(area);
    crate::ui::main_content::lx::render_history_for_test(area, &mut buf, &expanded, 0);
    let text = rows_text(&buf, area).join("\n");
    assert!(text.contains("new1"), "展开后显示文件 diff：{text}");
}

#[test]
fn recap_card_renders_title_next_and_folds_summary() {
    let area = Rect::new(0, 0, 80, 60);
    let mut message = message("assistant", "complete", vec![text_part("done")]);
    message.id = "assistant-recap".into();
    let mut lx = lx_with(vec![message.clone()]);
    lx.recap.entries.insert(
        message.id.clone(),
        MessageRecapView {
            id: "recap-1".into(),
            summary: "line one\nline two\nline three\nline four".into(),
            next_action: "run tests".into(),
            model_label: "Model".into(),
            end_turn: 0,
            end_sequence: 0,
            created_at_ms: 0,
        },
    );
    let mut buf = Buffer::empty(area);
    crate::ui::main_content::lx::render_history_for_test(area, &mut buf, &lx, 0);
    let text = rows_text(&buf, area).join("\n");
    assert!(text.contains(text::LX_RECAP_TITLE), "{text}");
    assert!(text.contains("Next: run tests"), "{text}");
    assert!(text.contains("line one"), "{text}");
    assert!(!text.contains("line four"), "默认只保留前 2 行：{text}");

    let mut expanded = lx;
    assert!(
        expanded
            .chat
            .toggle_section("assistant-recap", "recap", CardSection::Body,)
    );
    let mut buf = Buffer::empty(area);
    crate::ui::main_content::lx::render_history_for_test(area, &mut buf, &expanded, 0);
    let text = rows_text(&buf, area).join("\n");
    assert!(text.contains("line four"), "展开后显示全部：{text}");
}

#[test]
fn running_recap_shows_placeholder_under_pending_message() {
    let area = Rect::new(0, 0, 80, 40);
    let mut message = message("assistant", "complete", vec![text_part("done")]);
    message.id = "assistant-running".into();
    let mut lx = lx_with(vec![message]);
    lx.recap.pending = Some(RecapPending {
        generation: 1,
        manual: true,
        message_id: "assistant-running".into(),
        end_turn: 0,
        end_sequence: 0,
        started_at: std::time::Instant::now(),
        elapsed_ms: 3_000,
    });
    let mut buf = Buffer::empty(area);
    crate::ui::main_content::lx::render_history_for_test(area, &mut buf, &lx, 0);
    let text = rows_text(&buf, area).join("\n");
    assert!(text.contains(text::LX_RECAP_RUNNING), "{text}");
    assert!(text.contains("3s"), "{text}");
}
