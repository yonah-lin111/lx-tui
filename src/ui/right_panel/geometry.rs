//! 右侧容器与列表通用几何：面板顶边框、顶部表头（工具栏 + 分割线）与主体区。
//!
//! prompt 与 todo 共用这份布局；prompt 专属的按钮/文本/滚动条几何仍在
//! `ui::right_panel::prompt::geometry`，`app` 使用的纯正文分割在
//! `app::prompt::geometry`，此模块只做渲染侧的共享矩形。

use ratatui::layout::Rect;

use crate::tiling::pane_inner_rect;

pub use crate::app::prompt::geometry::{
    PROMPT_DIVIDER_HEIGHT, PROMPT_HEADER_HEIGHT, PROMPT_TOOLBAR_HEIGHT,
    prompt_body_rect as body_rect,
};

/// 面板顶边框行矩形（内嵌标题与页控件/tab）；面板过窄时为 None。
pub fn border_rect(panel: Rect) -> Option<Rect> {
    (panel.width > 4 && panel.height > 0).then_some(Rect::new(panel.x, panel.y, panel.width, 1))
}

/// 列表顶部表头矩形（工具栏 + 分割线两行）；内容区过矮时不显示。
pub fn header_rect(panel: Rect) -> Option<Rect> {
    let inner = pane_inner_rect(panel);
    (inner.height > PROMPT_HEADER_HEIGHT).then_some(Rect::new(
        inner.x,
        inner.y,
        inner.width,
        PROMPT_HEADER_HEIGHT,
    ))
}

/// 工具栏行矩形（表头首行）；表头隐藏时为 None。
pub fn toolbar_rect(panel: Rect) -> Option<Rect> {
    header_rect(panel).map(|header| Rect {
        height: PROMPT_TOOLBAR_HEIGHT,
        ..header
    })
}

/// 工具栏下方分割线行矩形；表头隐藏时为 None。
pub fn divider_rect(panel: Rect) -> Option<Rect> {
    header_rect(panel).map(|header| Rect {
        y: header.y + PROMPT_TOOLBAR_HEIGHT,
        height: PROMPT_DIVIDER_HEIGHT,
        ..header
    })
}
