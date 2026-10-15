//! UI 组件查看弹窗的打开与键盘交互：折叠、选择移动与关闭。

use crate::app::actions::OverlayKey;
use crate::app::overlay::Overlay;
use crate::app::state::AppState;
use crate::app::ui_gallery::UiGallery;

use super::menu::close_overlay;

/// 打开 UI 组件查看弹窗（重置为默认展开、高亮首行、右侧滚动在顶部）。
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

/// 右侧预览滚动一页的行数：键盘 `PageUp/PageDown` 的步长。
///
/// 与鼠标滚轮一样按行滚动；这里用固定页长（不依赖终端几何），由渲染层钳制到
/// 实际最大偏移，界面小窗时不越界、大窗时一页略短但不影响可用。
pub const UI_GALLERY_PAGE: isize = 5;

/// 滚动右侧预览：`delta` 为行偏移，`max` 为渲染层算出的最大偏移；越界钳制。
///
/// `max` 由调用方（渲染/事件层）按当前内容与视口高度给出；传入 `usize::MAX`
/// 表示不夹取（最终仍由渲染层钳制）。
pub fn scroll_ui_gallery(state: &mut AppState, delta: isize, max: usize) -> bool {
    let Some(Overlay::UiGallery(gallery)) = state.overlay.as_mut() else {
        return false;
    };
    gallery.scroll_by(delta, max)
}

/// 设置右侧预览滚动偏移（滚动条轨道/拖拽）；越界钳制；返回是否变化。
pub fn set_ui_gallery_scroll(state: &mut AppState, offset: usize, max: usize) -> bool {
    let Some(Overlay::UiGallery(gallery)) = state.overlay.as_mut() else {
        return false;
    };
    gallery.set_scroll(offset, max)
}

/// UI 组件查看弹窗的按键处理：上下移动、左右折叠展开、回车切换、Esc 关闭。
///
/// 右侧预览的 `PageUp/PageDown` 翻页在事件层处理（需按当前内容高度钳制最大
/// 偏移，几何只在事件层可得），此处只保留移动左树的键。
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
