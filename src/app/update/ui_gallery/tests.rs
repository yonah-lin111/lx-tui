//! 单元测试：打开弹窗、键处理与鼠标选中/折叠。

use super::*;

use crate::app::actions::OverlayKey;
use crate::app::state::AppState;
use crate::app::ui_gallery::{UiGalleryComponentId, UiGalleryFolderId, UiGalleryNode};

fn open() -> AppState {
    let mut state = AppState::demo();
    open_ui_gallery(&mut state);
    state
}

fn gallery(state: &AppState) -> &UiGallery {
    match state.overlay.as_ref() {
        Some(Overlay::UiGallery(gallery)) => gallery,
        _ => panic!("gallery overlay open"),
    }
}

#[test]
fn open_sets_gallery_overlay_with_default_row() {
    let state = open();
    assert!(matches!(state.overlay, Some(Overlay::UiGallery(_))));
    assert_eq!(gallery(&state).selected(), 0);
}

#[test]
fn escape_closes_overlay() {
    let mut state = open();
    apply_ui_gallery_key(&mut state, OverlayKey::Esc);
    assert!(state.overlay.is_none());
}

#[test]
fn down_and_up_move_selection_within_bounds() {
    let mut state = open();
    apply_ui_gallery_key(&mut state, OverlayKey::Down);
    assert_eq!(gallery(&state).selected(), 1);
    apply_ui_gallery_key(&mut state, OverlayKey::Up);
    apply_ui_gallery_key(&mut state, OverlayKey::Up);
    assert_eq!(gallery(&state).selected(), 0);
}

#[test]
fn enter_toggles_folder_collapse() {
    let mut state = open();
    apply_ui_gallery_key(&mut state, OverlayKey::Enter);
    assert!(gallery(&state).is_collapsed(UiGalleryFolderId::Global));
    apply_ui_gallery_key(&mut state, OverlayKey::Right);
    assert!(!gallery(&state).is_collapsed(UiGalleryFolderId::Global));
    apply_ui_gallery_key(&mut state, OverlayKey::Left);
    assert!(gallery(&state).is_collapsed(UiGalleryFolderId::Global));
}

#[test]
fn click_selects_row_and_activates_folder() {
    let mut state = open();
    set_ui_gallery_selection(&mut state, 7);
    assert_eq!(
        gallery(&state).selected_node(),
        UiGalleryNode::Folder(UiGalleryFolderId::Prompt)
    );
    activate_ui_gallery_row(&mut state);
    assert!(gallery(&state).is_collapsed(UiGalleryFolderId::Prompt));
}

#[test]
fn click_on_component_selects_without_toggling() {
    let mut state = open();
    set_ui_gallery_selection(&mut state, 1);
    assert_eq!(
        gallery(&state).selected_node(),
        UiGalleryNode::Component(UiGalleryComponentId::Toast)
    );
    activate_ui_gallery_row(&mut state);
    assert!(!gallery(&state).is_collapsed(UiGalleryFolderId::Global));
}

#[test]
fn set_selection_ignored_for_out_of_range_index() {
    let mut state = open();
    set_ui_gallery_selection(&mut state, 99);
    assert_eq!(gallery(&state).selected(), 0);
}

#[test]
fn unhandled_keys_keep_overlay_open() {
    let mut state = open();
    apply_ui_gallery_key(&mut state, OverlayKey::Char('x'));
    assert!(matches!(state.overlay, Some(Overlay::UiGallery(_))));
}

#[test]
fn apply_key_noop_without_gallery_overlay() {
    let mut state = AppState::demo();
    apply_ui_gallery_key(&mut state, OverlayKey::Down);
    assert!(state.overlay.is_none());
}
