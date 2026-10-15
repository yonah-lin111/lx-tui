//! 单元测试：全屏覆盖几何、分区、行命中与真实外观渲染。

use super::*;

use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::style::Color;

use crate::app::overlay::Overlay;
use crate::app::state::AppState;
use crate::app::ui_gallery::{UiGallery, UiGalleryComponentId, UiGalleryFolderId};

/// 渲染整屏并返回文本行。
fn render_lines(state: &AppState, width: u16, height: u16) -> Vec<String> {
    let mut terminal =
        Terminal::new(TestBackend::new(width, height)).expect("test backend is infallible");
    if let Err(error) = terminal.draw(|frame| {
        crate::ui::render(frame, state, &crate::config::Config::default());
    }) {
        panic!("draw failed: {error}");
    }
    let buffer = terminal.backend().buffer().clone();
    (0..buffer.area.height)
        .map(|y| {
            (0..buffer.area.width)
                .map(|x| buffer[(x, y)].symbol())
                .collect()
        })
        .collect()
}

fn gallery_state() -> AppState {
    let mut state = AppState::demo();
    state.overlay = Some(Overlay::UiGallery(UiGallery::new()));
    state
}

#[test]
fn shell_covers_the_whole_screen_with_a_border() {
    let screen = Rect::new(0, 0, 100, 30);
    let shell = ui_gallery_shell(screen).expect("gallery fits");
    assert_eq!(shell.area, screen, "全屏覆盖无四周留白");
    // 最外层边框内区即内容区（标题/提示内嵌边框线上，不占内容行）。
    assert_eq!(shell.content, Rect::new(1, 1, 98, 28));
}

#[test]
fn too_small_screen_has_no_shell() {
    assert!(ui_gallery_shell(Rect::new(0, 0, 5, 5)).is_none());
}

#[test]
fn layout_splits_left_fixed_width_and_right_remainder() {
    let screen = Rect::new(0, 0, 100, 30);
    let layout = ui_gallery_layout(screen).expect("gallery fits");
    assert_eq!(layout.left.width, 24);
    assert_eq!(layout.left.x, layout.shell.content.x);
    assert_eq!(
        layout.left.right() + 1,
        layout.right.x,
        "中间保留 1 列分隔线"
    );
    assert_eq!(layout.right.right(), layout.shell.content.right());
}

#[test]
fn narrow_screen_compresses_left_and_keeps_right_nonempty() {
    // 内容区宽度 24 < 左侧固定 24 + 分隔 1，左侧被压缩。
    let screen = Rect::new(0, 0, 24, 12);
    let layout = ui_gallery_layout(screen).expect("gallery fits");
    assert!(layout.left.width < 24);
    assert!(layout.right.width >= 1);
}

#[test]
fn row_hit_returns_visible_index_and_rejects_outside() {
    let layout = ui_gallery_layout(Rect::new(0, 0, 100, 30)).expect("gallery fits");
    let rows = 10;
    assert_eq!(
        ui_gallery_row_at(&layout, rows, layout.left.x, layout.left.y),
        Some(0)
    );
    assert_eq!(
        ui_gallery_row_at(&layout, rows, layout.left.x, layout.left.y + 3),
        Some(3)
    );
    // 右侧预览区不命中原树行。
    assert_eq!(
        ui_gallery_row_at(&layout, rows, layout.right.x, layout.right.y),
        None
    );
}

#[test]
fn row_hit_rejects_index_beyond_rows() {
    let layout = ui_gallery_layout(Rect::new(0, 0, 100, 30)).expect("gallery fits");
    assert_eq!(
        ui_gallery_row_at(&layout, 2, layout.left.x, layout.left.y + 5),
        None
    );
}

#[test]
fn render_is_fullscreen_without_panel_background() {
    let state = gallery_state();
    let mut terminal = Terminal::new(TestBackend::new(100, 30)).expect("test backend");
    terminal
        .draw(|frame| {
            crate::ui::render(frame, &state, &crate::config::Config::default());
        })
        .expect("draw succeeds");
    let buffer = terminal.backend().buffer().clone();
    // 全屏覆盖不铺自包含面板底色（overlay_panel 为 Indexed 236）。
    assert_ne!(buffer[(2, 2)].bg, Color::Indexed(236));
    assert_ne!(buffer[(50, 15)].bg, Color::Indexed(236));
}

#[test]
fn render_shows_title_tree_and_hint() {
    let state = gallery_state();
    let lines = render_lines(&state, 100, 30);
    let all: String = lines.join("\n");
    assert!(all.contains(text::UI_GALLERY_TITLE));
    assert!(all.contains(UiGalleryFolderId::Global.label()));
    assert!(all.contains(UiGalleryFolderId::Prompt.label()));
    assert!(all.contains(UiGalleryComponentId::Toast.label()));
    assert!(all.contains(text::UI_GALLERY_HINT));
    assert!(all.contains(text::AGENT_SELECTED_MARKER));
}

#[test]
fn render_preview_shows_real_toast_without_wrapper_title() {
    let mut state = gallery_state();
    // 高亮第一个组件（toast）。
    if let Some(Overlay::UiGallery(gallery)) = state.overlay.as_mut() {
        gallery.set_selected(1);
    }
    let width = 100u16;
    let lines = render_lines(&state, width, 30);
    let layout = ui_gallery_layout(Rect::new(0, 0, width, 30)).expect("gallery fits");
    // 只取右侧预览区的文本。
    let preview: String = lines
        .iter()
        .map(|line| {
            line.chars()
                .skip(usize::from(layout.right.x))
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("\n");
    // 真实 toast 文案与说明文字出现。
    assert!(preview.contains(text::UI_GALLERY_TOAST_SAMPLE));
    assert!(preview.contains("Non-modal feedback"));
    // 不再有拉满宽度的样例包裹标题（组件名作为边框标题）。
    assert!(
        !preview.contains(&format!(" {} ", UiGalleryComponentId::Toast.label())),
        "toast 不应被统一的样例大框包裹: {preview}"
    );
}

#[test]
fn collapsed_folder_hides_component_rows_in_render() {
    let mut state = gallery_state();
    if let Some(Overlay::UiGallery(gallery)) = state.overlay.as_mut() {
        gallery.toggle_folder(UiGalleryFolderId::Global);
    }
    let lines = render_lines(&state, 100, 30);
    let header_rows: Vec<&String> = lines
        .iter()
        .filter(|line| line.contains(UiGalleryFolderId::Global.label()))
        .collect();
    assert!(!header_rows.is_empty());
    let tree_text: String = lines
        .iter()
        .map(|line| line.chars().take(24).collect::<String>())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(!tree_text.contains(UiGalleryComponentId::Toast.label()));
}

#[test]
fn render_does_not_panic_on_each_component() {
    for index in 0..10 {
        let mut state = gallery_state();
        if let Some(Overlay::UiGallery(gallery)) = state.overlay.as_mut() {
            gallery.set_selected(index);
        }
        let _ = render_lines(&state, 100, 30);
    }
}

#[test]
fn preview_component_is_horizontally_centered() {
    let mut state = gallery_state();
    if let Some(Overlay::UiGallery(gallery)) = state.overlay.as_mut() {
        gallery.set_selected(1); // toast
    }
    let width = 100u16;
    let lines = render_lines(&state, width, 30);
    let layout = ui_gallery_layout(Rect::new(0, 0, width, 30)).expect("fits");
    let (component_width, _) = appearance_size(UiGalleryComponentId::Toast, layout.right);
    let expected_x = layout.right.x + layout.right.width.saturating_sub(component_width) / 2;
    // 组件不是贴左渲染，且顶部圆角出现在水平居中位置。
    assert!(expected_x > layout.right.x, "组件应水平居中而非贴左");
    let row: Vec<char> = lines[layout.right.y as usize].chars().collect();
    assert_eq!(
        row[usize::from(expected_x)],
        '╭',
        "组件左上角应位于居中列 {expected_x}"
    );
}
