//! 提示词正文区几何：app 与 ui 共用的纯布局原语，不含渲染与状态。
//!
//! `app` 只依赖此处的表头/正文分割结果与少量尺寸常量；prompt 工具栏按钮、
//! 文本区与滚动条等渲染专属几何仍留在 `ui::right_panel::prompt::geometry`。

use ratatui::layout::Rect;

use crate::tiling::pane_inner_rect;

use super::{MAX_RECENT_PAGES, RECENT_MIN_VISIBLE_ENTRIES};

/// 工具栏高度：1（按钮行；分割线在下一行，不穿过按钮）。
pub const PROMPT_TOOLBAR_HEIGHT: u16 = 1;
/// 工具栏下方分割线高度：1。
pub const PROMPT_DIVIDER_HEIGHT: u16 = 1;
/// 顶部保留行数（工具栏 + 分割线）。
pub const PROMPT_HEADER_HEIGHT: u16 = PROMPT_TOOLBAR_HEIGHT + PROMPT_DIVIDER_HEIGHT;
/// 树视图顶部固定行数（搜索框 + 分割线）。
pub const PROMPT_TREE_HEADER_HEIGHT: u16 = 2;
/// 块左侧色条占位（`▎` + 0 列间隔）；右侧不再占位。
pub const PROMPT_BLOCK_GUTTER_LEFT: u16 = 1;

/// 主视图与底部 recent 抽屉的上下分割几何。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PromptBodySplit {
    /// 上方主视图（文本/列表，不含右侧滚动条槽）。
    pub main: Rect,
    /// 上方主视图右侧滚动条槽。
    pub main_scrollbar: Option<Rect>,
    /// 抽屉顶部分割线行（整行 `├─ recent ─┤`）；抽屉收起时 None。
    pub recent_divider: Option<Rect>,
    /// 抽屉内容区（不含右侧滚动条槽）。
    pub recent_text: Option<Rect>,
    /// 抽屉右侧独立滚动条槽。
    pub recent_scrollbar: Option<Rect>,
    /// 抽屉实际可见条目数（0 表示空态行）。
    pub visible_entries: usize,
}

/// 分割主视图与底部抽屉：抽屉高度按内容区一半内的容量动态增长（1 行分割线 + 每条 2 行），
/// 非空记录至少 2 条、至多 `MAX_RECENT_PAGES` 条；空态固定占 2 行；半屏放不下最低 2 条时收起。
pub fn prompt_body_split(panel: Rect, recent_open: bool, recent_count: usize) -> PromptBodySplit {
    let inner = pane_inner_rect(panel);
    let body = prompt_body_rect(inner);
    if !recent_open || body.height == 0 {
        return suppressed_split(body);
    }
    let main = body_text_rect(body);
    let max_height = body.height / 2;
    // 与 recent_page_rows 的截断保持一致，避免布局与渲染行数不一致。
    let recent_count = recent_count.min(MAX_RECENT_PAGES);
    let (visible, drawer_height) = if recent_count == 0 {
        (0, 2)
    } else {
        let capacity = usize::from(max_height.saturating_sub(1) / 2);
        if capacity < RECENT_MIN_VISIBLE_ENTRIES {
            return suppressed_split(body);
        }
        let visible = recent_count.max(RECENT_MIN_VISIBLE_ENTRIES).min(capacity);
        (visible, 1 + 2 * visible as u16)
    };
    if drawer_height > max_height {
        return suppressed_split(body);
    }
    let divider_y = body.y + body.height - drawer_height;
    let divider = Rect::new(body.x, divider_y, body.width, 1);
    let content = Rect::new(
        body.x,
        divider_y + 1,
        body.width,
        drawer_height.saturating_sub(1),
    );
    let scrollbar =
        (body.width >= 2).then_some(Rect::new(body.right() - 1, content.y, 1, content.height));
    let recent_text = if body.width >= 2 {
        Rect {
            width: content.width - 1,
            ..content
        }
    } else {
        content
    };
    PromptBodySplit {
        main: Rect {
            height: main.height.saturating_sub(drawer_height),
            ..main
        },
        main_scrollbar: (body.width >= 2).then_some(Rect::new(
            body.right() - 1,
            main.y,
            1,
            main.height.saturating_sub(drawer_height),
        )),
        recent_divider: Some(divider),
        recent_text: Some(recent_text),
        recent_scrollbar: scrollbar,
        visible_entries: visible,
    }
}

/// 抽屉收起或空间不足时的分割结果：主视图占满内容区。
fn suppressed_split(body: Rect) -> PromptBodySplit {
    let main = body_text_rect(body);
    PromptBodySplit {
        main,
        main_scrollbar: (body.width >= 2).then_some(Rect::new(
            body.right() - 1,
            main.y,
            1,
            main.height,
        )),
        recent_divider: None,
        recent_text: None,
        recent_scrollbar: None,
        visible_entries: 0,
    }
}

/// 内容区扣除顶部表头后的文本/滚动条区域；
/// 高度不足时表头隐藏、退回全部内容区。
pub fn prompt_body_rect(inner: Rect) -> Rect {
    if inner.height > PROMPT_HEADER_HEIGHT {
        Rect {
            y: inner.y + PROMPT_HEADER_HEIGHT,
            height: inner.height - PROMPT_HEADER_HEIGHT,
            ..inner
        }
    } else {
        inner
    }
}

/// 内容区扣除右侧滚动条槽后的可绘制区域（宽 < 2 时不预留）。
pub fn body_text_rect(body: Rect) -> Rect {
    if body.width >= 2 {
        Rect {
            width: body.width - 1,
            ..body
        }
    } else {
        body
    }
}

#[cfg(test)]
mod tests;
