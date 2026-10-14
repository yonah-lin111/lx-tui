//! 单元测试：弹窗几何、分区、行命中与预览渲染。

use super::*;

use ratatui::Terminal;
use ratatui::backend::TestBackend;

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
fn shell_leaves_two_columns_and_rows_on_every_side() {
    let screen = Rect::new(0, 0, 100, 30);
    let shell = ui_gallery_shell(screen).expect("gallery fits");
    assert_eq!(shell.area.x, 2);
    assert_eq!(shell.area.y, 2);
    assert_eq!(shell.area.width, 96);
    assert_eq!(shell.area.height, 26);
    assert_eq!(shell.area.right(), 98);
    assert_eq!(shell.area.bottom(), 28);
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
    assert_eq!(layout.left.x, layout.shell.inner.x);
    assert_eq!(
        layout.left.right() + 1,
        layout.right.x,
        "中间保留 1 列分隔线"
    );
    assert_eq!(layout.right.right(), layout.shell.inner.right());
}

#[test]
fn narrow_screen_compresses_left_and_keeps_right_nonempty() {
    let screen = Rect::new(0, 0, 20, 12);
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
    // 末行（快捷键提示）不命中。
    let hint_y = layout.left.bottom().saturating_sub(1);
    assert_eq!(
        ui_gallery_row_at(&layout, rows, layout.left.x, hint_y),
        None
    );
}

#[test]
fn row_hit_rejects_index_beyond_rows() {
    let layout = ui_gallery_layout(Rect::new(0, 0, 100, 30)).expect("gallery fits");
    // 只有 2 行可见时，索引 5 不命中。
    assert_eq!(
        ui_gallery_row_at(&layout, 2, layout.left.x, layout.left.y + 5),
        None
    );
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
    // 首行高亮标记与默认选中（global 文件夹）对应。
    assert!(all.contains(text::AGENT_SELECTED_MARKER));
}

#[test]
fn render_preview_switches_with_selection() {
    let mut state = gallery_state();
    // 高亮第一个组件（toast）：右侧应出现其说明文字前缀。
    if let Some(Overlay::UiGallery(gallery)) = state.overlay.as_mut() {
        gallery.set_selected(1);
    }
    let lines = render_lines(&state, 100, 30);
    let all: String = lines.join("\n");
    assert!(
        all.contains("Non-modal feedback"),
        "toast description should render"
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
    // global 行仍在，但其下的 toast 组件行不再出现。
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
