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
fn assistant_image_without_source_path_has_no_underline_or_attachment_region() {
    let area = Rect::new(0, 0, 80, 20);
    let mut msg = message("assistant", "complete", vec![text_part("Generated image")]);
    msg.model_label = Some("Gemini · 3.8 Flash".into());
    msg.attachments = vec![MessageAttachmentMeta {
        id: "att-1".into(),
        position: 0,
        kind: MessageAttachmentKind::Image,
        label: "gemini-r1-1.webp".into(),
        marker_start: None,
        marker_end: None,
        source_path: None,
        mime: None,
        width: None,
        height: None,
    }];
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
    let mut buf = Buffer::empty(area);
    crate::ui::main_content::lx::render_history_for_test(area, &mut buf, &lx, 0);
    let text = rows_text(&buf, area).join("\n");
    assert!(
        text.contains("└─ Image (preview): gemini-r1-1.webp  [Download original]"),
        "{text}"
    );
    // 没有落盘预览图：文件名不带下划线。
    let row_y = (0..area.height)
        .find(|&y| rows_text(&buf, area)[usize::from(y)].contains("gemini-r1-1.webp"))
        .expect("row");
    let label_x = rows_text(&buf, area)[usize::from(row_y)]
        .find("gemini-r1-1.webp")
        .expect("col") as u16;
    let cell = buf.cell((label_x, row_y)).expect("cell");
    assert!(
        !cell.modifier.contains(ratatui::style::Modifier::UNDERLINED),
        "missing preview must not underline: {:?}",
        cell.modifier
    );

    let view = layout(area, &lx, &mut cache());
    assert!(
        !view
            .cards
            .iter()
            .any(|card| matches!(card.kind, CardKind::Attachment { .. })),
        "no preview path -> no attachment region"
    );
    let buttons: Vec<(&str, bool)> = view
        .cards
        .iter()
        .filter_map(|card| match &card.kind {
            CardKind::DownloadOriginalImage { attachment_id } => {
                Some((attachment_id.as_str(), card.clickable))
            }
            _ => None,
        })
        .collect();
    assert_eq!(
        buttons,
        vec![("att-1", true)],
        "download original button still works"
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

#[test]
fn markdown_links_and_ai_web_images_require_ctrl_or_cmd_click_to_open() {
    use crate::app::state::CardTarget;
    use crossterm::event::KeyModifiers;

    let area = Rect::new(0, 0, 80, 20);
    let mut msg = message(
        "assistant",
        "complete",
        vec![text_part(
            "请查看 [Daily World Briefing](https://english.news.cn/20261110/c.html)。",
        )],
    );
    msg.attachments = vec![
        MessageAttachmentMeta {
            id: "att-idle".into(),
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
            id: "att-done".into(),
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
        "att-idle".into(),
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
        "att-done".into(),
        crate::app::state::GeneratedImageOriginal {
            site: crate::app::state::AiWebSite::Gemini,
            session_id: "ai-web-gm-123".into(),
            original_url: "https://lh3.googleusercontent.com/gg/def=s0-d-I?alr=yes".into(),
            fallback_url: "https://lh3.googleusercontent.com/gg/def=s1024-rw".into(),
            file_stem: "gemini-123-r1-2-orig".into(),
            state: crate::app::state::OriginalImageDownloadState::Downloaded {
                path: "/tmp/gemini-123-r1-2-orig.png".into(),
            },
        },
    );

    let mut pane_cache = cache();
    let mut buf = Buffer::empty(area);
    crate::ui::main_content::lx::render(area, &mut buf, &lx, true, None, &mut pane_cache);
    let rows = rows_text(&buf, area);

    // 1. Markdown 超链接：普通左键不触发 OpenUrl，Ctrl/Cmd + 点击触发 OpenUrl。
    let link_y = (0..area.height)
        .find(|&y| rows[usize::from(y)].contains("Daily World Briefing"))
        .expect("link row");
    let link_x = rows[usize::from(link_y)]
        .chars()
        .collect::<Vec<_>>()
        .windows("Daily".chars().count())
        .position(|w| w == ['D', 'a', 'i', 'l', 'y'])
        .expect("link col") as u16;
    let plain_link_hit = crate::ui::main_content::lx::press_content_hit(
        area,
        &lx,
        link_x,
        link_y,
        KeyModifiers::NONE,
        &mut pane_cache,
    )
    .expect("plain hit");
    assert_eq!(plain_link_hit.2, None, "普通点击超链接不得触发外部打开");

    for modifier in [KeyModifiers::CONTROL, KeyModifiers::SUPER] {
        let mod_link_hit = crate::ui::main_content::lx::press_content_hit(
            area,
            &lx,
            link_x,
            link_y,
            modifier,
            &mut pane_cache,
        )
        .expect("mod hit");
        assert_eq!(
            mod_link_hit.2,
            Some(CardTarget::OpenUrl {
                url: "https://english.news.cn/20261110/c.html".into(),
            }),
            "{modifier:?} + 点击超链接应返回 OpenUrl"
        );
    }

    // 2. AI Web 预览图文件名：普通左键不触发 Attachment，Ctrl/Cmd + 点击触发 Attachment。
    let img1_y = (0..area.height)
        .find(|&y| rows[usize::from(y)].contains("gemini-r1-1.webp"))
        .expect("img1 row");
    let img1_x = rows[usize::from(img1_y)]
        .find("gemini-r1-1.webp")
        .expect("img1 col") as u16;
    let plain_img_hit = crate::ui::main_content::lx::press_content_hit(
        area,
        &lx,
        img1_x,
        img1_y,
        KeyModifiers::NONE,
        &mut pane_cache,
    )
    .expect("plain img hit");
    assert_eq!(
        plain_img_hit.2, None,
        "普通点击 AI Web 预览图文件名不得弹出文件管理器"
    );

    let ctrl_img_hit = crate::ui::main_content::lx::press_content_hit(
        area,
        &lx,
        img1_x,
        img1_y,
        KeyModifiers::CONTROL,
        &mut pane_cache,
    )
    .expect("ctrl img hit");
    assert_eq!(
        ctrl_img_hit.2,
        Some(CardTarget::Attachment {
            path: "/tmp/gemini-r1-1.webp".into(),
        })
    );

    // 3. 未下载态 `[Download original]` 按钮：普通左键即可触发下载；
    // 已下载态 `[✓ Downloaded]`：必须 Ctrl/Cmd + 点击才触发定位原图。
    let dl_idle_x = rows[usize::from(img1_y)]
        .find("[Download original]")
        .expect("idle btn col") as u16;
    let plain_dl_idle = crate::ui::main_content::lx::press_content_hit(
        area,
        &lx,
        dl_idle_x,
        img1_y,
        KeyModifiers::NONE,
        &mut pane_cache,
    )
    .expect("plain dl idle");
    assert_eq!(
        plain_dl_idle.2,
        Some(CardTarget::DownloadOriginalImage {
            attachment_id: "att-idle".into(),
        }),
        "未下载原图按钮保持普通左键可点"
    );

    let img2_y = (0..area.height)
        .find(|&y| rows[usize::from(y)].contains("gemini-r1-2.webp"))
        .expect("img2 row");
    let dl_done_x = rows[usize::from(img2_y)]
        .find("Downloaded]")
        .expect("done btn col") as u16;
    let plain_dl_done = crate::ui::main_content::lx::press_content_hit(
        area,
        &lx,
        dl_done_x,
        img2_y,
        KeyModifiers::NONE,
        &mut pane_cache,
    )
    .expect("plain dl done");
    assert_eq!(
        plain_dl_done.2, None,
        "已下载原图按钮在普通左键下不弹出文件管理器"
    );

    let cmd_dl_done = crate::ui::main_content::lx::press_content_hit(
        area,
        &lx,
        dl_done_x,
        img2_y,
        KeyModifiers::SUPER,
        &mut pane_cache,
    )
    .expect("cmd dl done");
    assert_eq!(
        cmd_dl_done.2,
        Some(CardTarget::DownloadOriginalImage {
            attachment_id: "att-done".into(),
        }),
        "Cmd/Ctrl + 点击已下载原图按钮触发定位原图"
    );
}
