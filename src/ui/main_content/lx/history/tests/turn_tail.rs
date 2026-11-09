use ratatui::buffer::Buffer;
use ratatui::layout::Rect;

use super::*;
use crate::app::state::CardSection;
use crate::app::state::lx::{MessageRecapView, RecapPending};
use crate::db::chat::{MessageAttachmentKind, MessageAttachmentMeta};

fn rows_text(buf: &Buffer, area: Rect) -> Vec<String> {
    (0..area.height)
        .map(|y| {
            (0..area.width)
                .filter_map(|x| buf.cell((x, y)).map(|cell| cell.symbol()))
                .collect::<String>()
        })
        .collect()
}

#[test]
fn assistant_image_attachments_render_like_user_refs_and_register_clicks() {
    let area = Rect::new(0, 0, 80, 20);
    let mut msg = message("assistant", "complete", vec![text_part("Generated image")]);
    msg.model_label = Some("Gemini · 3.8 Flash".into());
    msg.duration_ms = Some(4000);
    msg.attachments = vec![
        MessageAttachmentMeta {
            id: "att-1".into(),
            position: 0,
            kind: MessageAttachmentKind::Image,
            label: "gemini-r1-1.webp".into(),
            marker_start: None,
            marker_end: None,
            source_path: Some("/tmp/gemini-r1-1.webp".into()),
            mime: None,
            width: None,
            height: None,
        },
        MessageAttachmentMeta {
            id: "att-2".into(),
            position: 1,
            kind: MessageAttachmentKind::Image,
            label: "gemini-r1-2.webp".into(),
            marker_start: None,
            marker_end: None,
            source_path: Some("/tmp/gemini-r1-2.webp".into()),
            mime: None,
            width: None,
            height: None,
        },
    ];
    let lx = lx_with(vec![msg]);
    let mut buf = Buffer::empty(area);
    crate::ui::main_content::lx::render_history_for_test(area, &mut buf, &lx, 0);
    let text = rows_text(&buf, area).join("\n");
    assert!(text.contains("├─ Image: gemini-r1-1.webp"), "{text}");
    assert!(text.contains("└─ Image: gemini-r1-2.webp"), "{text}");

    let view = layout(area, &lx, &mut cache());
    let paths: Vec<&str> = view
        .cards
        .iter()
        .filter_map(|card| match &card.kind {
            CardKind::Attachment { path } => Some(path.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(
        paths,
        vec!["/tmp/gemini-r1-1.webp", "/tmp/gemini-r1-2.webp"]
    );
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
    assert!(
        text.contains(text::agent::STEP_MARK_DIAMOND_HOLLOW),
        "生成中必须是空心菱形：{text}"
    );
}
