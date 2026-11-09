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
fn assistant_image_attachments_render_preview_underline_and_download_original_button() {
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
    let mut lx = lx_with(vec![msg]);
    lx.chat.image_originals.insert(
        "att-1".into(),
        crate::app::state::GeneratedImageOriginal {
            site: crate::app::state::AiWebSite::Gemini,
            session_id: "ai-web-gm-123".into(),
            original_url: "https://lh3.googleusercontent.com/gg/abc=s0-d-I?alr=yes".into(),
            fallback_url: "https://lh3.googleusercontent.com/gg/abc=s1024-rw".into(),
            file_stem: "gemini-123-r1-1-orig".into(),
            state: crate::app::state::OriginalImageDownloadState::Idle,
        },
    );
    lx.chat.image_originals.insert(
        "att-2".into(),
        crate::app::state::GeneratedImageOriginal {
            site: crate::app::state::AiWebSite::Gemini,
            session_id: "ai-web-gm-123".into(),
            original_url: "https://lh3.googleusercontent.com/gg/def=s0-d-I?alr=yes".into(),
            fallback_url: "https://lh3.googleusercontent.com/gg/def=s1024-rw".into(),
            file_stem: "gemini-123-r1-2-orig".into(),
            state: crate::app::state::OriginalImageDownloadState::Downloading,
        },
    );
    let mut buf = Buffer::empty(area);
    crate::ui::main_content::lx::render_history_for_test(area, &mut buf, &lx, 0);
    let text = rows_text(&buf, area).join("\n");
    assert!(
        text.contains("├─ Image (preview): gemini-r1-1.webp  [Download original]"),
        "{text}"
    );
    assert!(
        text.contains("└─ Image (preview): gemini-r1-2.webp  [downloading...]"),
        "{text}"
    );
    // 下载中按钮为黄色。
    let dl_row_y = (0..area.height)
        .find(|&y| rows_text(&buf, area)[usize::from(y)].contains("[downloading...]"))
        .expect("dl row");
    let dl_x = rows_text(&buf, area)[usize::from(dl_row_y)]
        .find("[downloading...]")
        .expect("dl col") as u16;
    let dl_cell = buf.cell((dl_x, dl_row_y)).expect("dl cell");
    assert_eq!(dl_cell.fg, ratatui::style::Color::Yellow);
    // 文件名本身带下划线。
    let row_y = (0..area.height)
        .find(|&y| rows_text(&buf, area)[usize::from(y)].contains("gemini-r1-1.webp"))
        .expect("row");
    let label_x = rows_text(&buf, area)[usize::from(row_y)]
        .find("gemini-r1-1.webp")
        .expect("col") as u16;
    let cell = buf.cell((label_x, row_y)).expect("cell");
    assert!(
        cell.modifier.contains(ratatui::style::Modifier::UNDERLINED),
        "filename must be underlined: {:?}",
        cell.modifier
    );

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
    let dl_buttons: Vec<(&str, bool)> = view
        .cards
        .iter()
        .filter_map(|card| match &card.kind {
            CardKind::DownloadOriginalImage { attachment_id } => {
                Some((attachment_id.as_str(), card.clickable))
            }
            _ => None,
        })
        .collect();
    assert_eq!(dl_buttons, vec![("att-1", true), ("att-2", false)]);
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
