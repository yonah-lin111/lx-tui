//! UI 组件查看弹窗的打开与键盘交互：折叠、选择移动与关闭。

use crate::app::actions::OverlayKey;
use crate::app::overlay::Overlay;
use crate::app::state::AppState;
use crate::app::ui_gallery::UiGallery;

use super::menu::close_overlay;

/// 打开 UI 组件查看弹窗（重置为默认展开、高亮首行）。
pub fn open_ui_gallery(state: &mut AppState) {
    state.overlay = Some(Overlay::UiGallery(UiGallery::new()));
}

/// 鼠标点击：把高亮移到指定可见行；越界忽略。
pub fn set_ui_gallery_selection(state: &mut AppState, index: usize) {
    if let Some(Overlay::UiGallery(gallery)) = state.overlay.as_mut() {
        gallery.set_selected(index);
    }
}

/// 鼠标点击组件行：文件夹行切换折叠、组件行无动作。
pub fn activate_ui_gallery_row(state: &mut AppState) {
    if let Some(Overlay::UiGallery(gallery)) = state.overlay.as_mut() {
        gallery.activate();
    }
}

/// UI 组件查看弹窗的按键处理：上下移动、左右折叠展开、回车切换、Esc 关闭。
pub fn apply_ui_gallery_key(state: &mut AppState, key: OverlayKey) {
    let Some(Overlay::UiGallery(gallery)) = state.overlay.as_mut() else {
        return;
    };
    match key {
        OverlayKey::Esc => close_overlay(state),
        OverlayKey::Up => {
            gallery.move_selection(-1);
        }
        OverlayKey::Down => {
            gallery.move_selection(1);
        }
        OverlayKey::Left => {
            gallery.move_left();
        }
        OverlayKey::Right => {
            gallery.move_right();
        }
        OverlayKey::Enter => {
            gallery.activate();
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests;
