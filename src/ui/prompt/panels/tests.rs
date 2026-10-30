//! Prompt 浮层面板渲染与 Tab 命中单元测试。

use std::path::PathBuf;

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;

use super::*;
use crate::app::prompt::{MentionTab, Prompt};
use crate::app::referenced::ReferencedFolder;
use crate::layout::PaneId;
use crate::ui::text;

fn create_editor_with_ref_folders() -> Prompt {
    let mut editor = Prompt::new(PaneId::from_raw_for_test(1));
    editor.resize(60, 20);
    editor.sync_referenced_folders(&[
        ReferencedFolder {
            id: 1,
            path: PathBuf::from("/Users/yonah/projects/herdr-master"),
        },
        ReferencedFolder {
            id: 2,
            path: PathBuf::from("/Users/yonah/projects/lx-agent"),
        },
    ]);
    editor
}

#[test]
fn renders_ref_dir_tab_and_switches_view() {
    let mut editor = create_editor_with_ref_folders();
    let area = Rect::new(0, 0, 60, 20);

    editor.insert_char('@');
    assert_eq!(editor.mention_tab(), MentionTab::Files);

    // Switch to refDir
    assert!(editor.mention_switch_tab(1));
    assert_eq!(editor.mention_tab(), MentionTab::RefDir);

    let layout = mention_layout(&editor, area).expect("mention layout");
    let mut buf = Buffer::empty(area);
    assert!(
        command_panel::render(
            area,
            &mut buf,
            &mention_view(
                &mention_panel_data(&editor, area.width)
                    .expect("data")
                    .items(),
                &mention_panel_data(&editor, area.width).expect("data"),
                area.height / 2
            )
        )
        .is_some()
    );

    // Top border contains tabs
    let top: String = (0..area.width)
        .map(|x| buf[(x, layout.rect.y)].symbol())
        .collect();
    assert!(top.contains(text::MENTION_TAB_FILES), "{top}");
    assert!(top.contains(text::MENTION_TAB_REF_DIR), "{top}");

    // Content rows contain referenced folder display names
    let content: String = buf.content.iter().map(|cell| cell.symbol()).collect();
    assert!(content.contains("herdr-master/"), "{content}");
    assert!(content.contains("lx-agent/"), "{content}");
    assert!(content.contains("└─ "), "{content}");

    // Footer shows refDir specific hint (no enter folder)
    let bottom: String = (0..area.width)
        .map(|x| buf[(x, layout.rect.bottom() - 1)].symbol())
        .collect();
    assert!(bottom.contains(text::MENTION_FOOTER_REF_DIR), "{bottom}");
    assert!(!bottom.contains("enter"), "{bottom}");
}

#[test]
fn mention_tab_at_hits_tabs() {
    let mut editor = create_editor_with_ref_folders();
    let area = Rect::new(0, 0, 60, 20);
    editor.insert_char('@');
    editor.mention_switch_tab(1); // open refDir so it is visible

    let layout = mention_layout(&editor, area).expect("mention layout");
    assert_eq!(layout.tabs.len(), 4);

    // Hit tab 0 (Files), 1 (refDir), 2 (Skills), 3 (MCP)
    for idx in 0..4 {
        let tab_x = layout.tabs[idx].x;
        assert_eq!(
            mention_tab_at(&editor, area, tab_x, layout.rect.y),
            Some(idx)
        );
    }

    // Outside tabs on the border row returns None
    assert_eq!(
        mention_tab_at(&editor, area, layout.rect.x, layout.rect.y),
        None
    );
}

#[test]
fn ref_dir_empty_state_renders_notice_and_ignores_item_click() {
    let mut editor = Prompt::new(PaneId::from_raw_for_test(1));
    editor.resize(60, 20);
    let area = Rect::new(0, 0, 60, 20);

    editor.insert_char('@');
    editor.mention_switch_tab(1); // switch to refDir with 0 folders

    let layout = mention_layout(&editor, area).expect("mention layout");
    let mut buf = Buffer::empty(area);
    let data = mention_panel_data(&editor, area.width).expect("data");
    let items = data.items();
    command_panel::render(
        area,
        &mut buf,
        &mention_view(&items, &data, area.height / 2),
    );

    assert!(matches!(
        data.rows.first(),
        Some(MentionRowText::Empty(msg)) if *msg == text::MENTION_REF_DIR_EMPTY
    ));
    let content: String = buf
        .content
        .iter()
        .map(|cell| cell.symbol())
        .collect::<String>()
        .replace(' ', "");
    assert!(
        content.contains(&text::MENTION_REF_DIR_EMPTY.replace(' ', "")),
        "{content}"
    );

    // Clicking on empty state does not hit any valid item
    assert_eq!(
        mention_item_at(&editor, area, layout.content.x, layout.content.y),
        None
    );
}
