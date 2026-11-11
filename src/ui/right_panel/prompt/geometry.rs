//! prompt 右栏渲染几何与工具栏按钮布局：纯函数，不涉及渲染与状态。
//!
//! 表头/正文分割与共享尺寸来自 `app::prompt::geometry`；面板顶边框、表头行
//! 等右栏通用矩形来自 `ui::right_panel::geometry`，此处按 prompt 语义重导出。

use ratatui::layout::Rect;

use crate::app::prompt::PromptMode;
use crate::tiling::pane_inner_rect;

pub use crate::app::prompt::geometry::{
    PROMPT_BLOCK_GUTTER_LEFT, PROMPT_DIVIDER_HEIGHT, PROMPT_HEADER_HEIGHT, PROMPT_TOOLBAR_HEIGHT,
    PROMPT_TREE_HEADER_HEIGHT, PromptBodySplit, body_text_rect, prompt_body_rect,
    prompt_body_split,
};
pub use crate::ui::right_panel::geometry::{
    border_rect as prompt_border_rect, divider_rect as prompt_divider_rect,
    header_rect as prompt_header_rect, toolbar_rect as prompt_toolbar_rect,
};

/// 模式按钮（`[≡]` / `[/]`）宽度（列）。
const PROMPT_TOOLBAR_BUTTON_WIDTH: u16 = 3;
/// 撤销/重做按钮（`[←]` / `[→]`）宽度（列）。
const PROMPT_UNDO_REDO_WIDTH: u16 = 3;
/// 工具栏按钮之间的空档（列）。
const PROMPT_TOOLBAR_BUTTON_GAP: u16 = 1;
/// `[rec]` 最近页面抽屉按钮宽度（列）。
const PROMPT_RECENT_WIDTH: u16 = 5;
/// 树视图 `[tmp]` 按钮宽度（列）。
const PROMPT_TEMP_PROMPT_WIDTH: u16 = 5;
/// 显示 `[tmp]`（右对齐）所需的最小内容区宽度（列）：不足时隐藏，避免与左侧按钮重叠。
const PROMPT_TEMP_PROMPT_MIN_WIDTH: u16 = PROMPT_TOOLBAR_BUTTON_WIDTH
    + PROMPT_TOOLBAR_BUTTON_GAP
    + PROMPT_RECENT_WIDTH
    + PROMPT_TOOLBAR_BUTTON_GAP
    + PROMPT_TEMP_PROMPT_WIDTH
    + 1;
/// 引用文件夹按钮（`[refDir]`）宽度（列）；仅编辑视图显示、位于页控件左侧。
pub const PROMPT_FOLDERS_WIDTH: u16 = 8;
/// 左侧按钮组（模式 + `[rec]` + 撤销/重做）所需的最小内容区宽度（列）。
const PROMPT_LEFT_GROUP_WITH_RECENT_WIDTH: u16 = PROMPT_TOOLBAR_BUTTON_WIDTH
    + PROMPT_TOOLBAR_BUTTON_GAP
    + PROMPT_RECENT_WIDTH
    + PROMPT_TOOLBAR_BUTTON_GAP
    + PROMPT_UNDO_REDO_WIDTH
    + PROMPT_TOOLBAR_BUTTON_GAP
    + PROMPT_UNDO_REDO_WIDTH
    + PROMPT_TOOLBAR_BUTTON_GAP
    + 1;
/// 窄屏收起 `[rec]` 后左侧按钮组（模式 + 撤销/重做）所需的最小内容区宽度（列）。
const PROMPT_LEFT_GROUP_MIN_WIDTH: u16 = PROMPT_TOOLBAR_BUTTON_WIDTH
    + PROMPT_TOOLBAR_BUTTON_GAP
    + PROMPT_UNDO_REDO_WIDTH
    + PROMPT_TOOLBAR_BUTTON_GAP
    + PROMPT_UNDO_REDO_WIDTH
    + PROMPT_TOOLBAR_BUTTON_GAP
    + 1;

/// 工具栏页控件几何：左箭头、计数、右箭头（`arrows == false` 时左右箭头宽度为 0）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PromptPageControls {
    pub prev: Rect,
    pub counter: Rect,
    pub next: Rect,
}

/// 工具栏页控件命中种类。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PageControl {
    Prev,
    Next,
    Counter,
}

/// 工具栏页控件几何（编辑视图 `‹ n/m ›`、页视图 `n/m`）；左侧至少留出模式按钮。
pub fn toolbar_page_controls(bar: Rect, counter: &str, arrows: bool) -> Option<PromptPageControls> {
    if bar.width == 0 {
        return None;
    }
    let counter_width = u16::try_from(unicode_width::UnicodeWidthStr::width(counter)).ok()?;
    let min_left = bar.x.saturating_add(4);
    if arrows {
        // 最右一列留给保存点 `●`，其左一列为空档，再左为 `›`。
        let next = Rect::new(bar.right().checked_sub(3)?, bar.y, 1, 1);
        let counter_x = next.x.checked_sub(1)?.checked_sub(counter_width)?;
        let counter_rect = Rect::new(counter_x, bar.y, counter_width, 1);
        let prev = Rect::new(counter_rect.x.checked_sub(2)?, bar.y, 1, 1);
        (prev.x >= min_left).then_some(PromptPageControls {
            prev,
            counter: counter_rect,
            next,
        })
    } else {
        // 无保存点：计数右侧留 1 列空档到边框。
        let right = bar.right().checked_sub(1)?;
        let counter_x = right.checked_sub(counter_width)?;
        let counter_rect = Rect::new(counter_x, bar.y, counter_width, 1);
        (counter_x >= min_left).then_some(PromptPageControls {
            prev: Rect::new(counter_x, bar.y, 0, 1),
            counter: counter_rect,
            next: Rect::new(right, bar.y, 0, 1),
        })
    }
}

/// prompt 工具栏按钮。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PromptToolbarButton {
    /// 视图模式切换：编辑视图显示 `[≡]`、树视图显示 `[/]`。
    Mode,
    /// 树视图 `[tmp]`：新建临时提示词编辑视图（仅树视图显示、不入库）。
    TempPrompt,
    /// `[rec]`：开关底部最近修改页面抽屉（三种视图常显）。
    Recent,
    /// 编辑视图 `[refDir]`：打开引用文件夹菜单（仅编辑视图显示，位于页控件左侧）。
    Folders,
    Undo,
    Redo,
    Save,
}

/// prompt 文本区矩形：扣除顶部工具栏表头，最右一列固定预留给滚动条槽（内容区宽度 >= 2 时）。
pub fn prompt_text_rect(panel: Rect) -> Rect {
    let inner = prompt_body_rect(pane_inner_rect(panel));
    if inner.width >= 2 {
        Rect {
            width: inner.width - 1,
            ..inner
        }
    } else {
        inner
    }
}

/// prompt 可点击内容区（文本区，不含滚动条槽）：面板锚点与命中用。
pub fn prompt_content_rect(panel: Rect) -> Rect {
    prompt_text_rect(panel)
}

/// prompt 滚动条槽矩形：与文本区起始行及高度一致，取内容区最右一列；内容区过窄时为 None。
pub fn prompt_scrollbar_rect(panel: Rect) -> Option<Rect> {
    let inner = prompt_body_rect(pane_inner_rect(panel));
    (inner.width >= 2 && inner.height > 0).then_some(Rect::new(
        inner.right() - 1,
        inner.y,
        1,
        inner.height,
    ))
}

/// 树视图滚动条槽：搜索框与分割线之下、贴右缘 1 列（搜索框固定不滚动）。
pub fn prompt_tree_scrollbar_rect(panel: Rect) -> Option<Rect> {
    let inner = prompt_body_rect(pane_inner_rect(panel));
    (inner.width >= 2 && inner.height > PROMPT_TREE_HEADER_HEIGHT).then_some(Rect::new(
        inner.right() - 1,
        inner.y + PROMPT_TREE_HEADER_HEIGHT,
        1,
        inner.height - PROMPT_TREE_HEADER_HEIGHT,
    ))
}

/// 工具栏按钮矩形；按钮在内容区放不下时为 None（`[refDir]` / `[tmp]` / `[rec]`
/// 按阈值独立隐藏；各视图模式的按钮集不同）。
///
/// `counter` 为当前页计数（如 `2/4`）：`[refDir]` 紧贴页控件左侧，缺省时按无页控件处理。
pub fn prompt_toolbar_button_rect(
    panel: Rect,
    button: PromptToolbarButton,
    mode: PromptMode,
    counter: Option<&str>,
) -> Option<Rect> {
    let bar = prompt_toolbar_rect(panel)?;
    if bar.width == 0 {
        return None;
    }
    let inner = pane_inner_rect(panel);
    // 右侧保留控件（页控件 `‹ n/m ›` 或保存点 `●`）的左边界：左组按钮的 `right()` 不得越过该列。
    let right_limit = match (mode, counter) {
        (PromptMode::Editor, Some(counter)) => toolbar_page_controls(bar, counter, true)
            .map(|controls| controls.prev.x.saturating_sub(1))
            .unwrap_or_else(|| bar.right().saturating_sub(2)),
        (PromptMode::Editor, None) => bar.right().saturating_sub(2),
        (PromptMode::Pages, Some(counter)) => toolbar_page_controls(bar, counter, false)
            .map(|controls| controls.counter.x.saturating_sub(1))
            .unwrap_or(bar.right()),
        (PromptMode::Pages, None) => bar.right(),
        (PromptMode::Tree, _) => bar.right(),
    };
    let full_editor_left = PROMPT_TOOLBAR_BUTTON_WIDTH
        + PROMPT_TOOLBAR_BUTTON_GAP
        + PROMPT_RECENT_WIDTH
        + PROMPT_TOOLBAR_BUTTON_GAP
        + PROMPT_UNDO_REDO_WIDTH
        + PROMPT_TOOLBAR_BUTTON_GAP
        + PROMPT_UNDO_REDO_WIDTH;
    let compact_editor_left = PROMPT_TOOLBAR_BUTTON_WIDTH
        + PROMPT_TOOLBAR_BUTTON_GAP
        + PROMPT_UNDO_REDO_WIDTH
        + PROMPT_TOOLBAR_BUTTON_GAP
        + PROMPT_UNDO_REDO_WIDTH;
    let tree_recent_left =
        PROMPT_TOOLBAR_BUTTON_WIDTH + PROMPT_TOOLBAR_BUTTON_GAP + PROMPT_RECENT_WIDTH;
    let show_recent = match mode {
        PromptMode::Editor => {
            inner.width >= PROMPT_LEFT_GROUP_WITH_RECENT_WIDTH
                && bar.x.saturating_add(full_editor_left) <= right_limit
        }
        PromptMode::Pages => {
            inner.width >= tree_recent_left && bar.x.saturating_add(tree_recent_left) <= right_limit
        }
        PromptMode::Tree => inner.width >= tree_recent_left,
    };
    let show_undo_redo = mode == PromptMode::Editor
        && inner.width >= PROMPT_LEFT_GROUP_MIN_WIDTH
        && bar.x.saturating_add(compact_editor_left) <= right_limit;
    let undo_x = bar.x
        + PROMPT_TOOLBAR_BUTTON_WIDTH
        + PROMPT_TOOLBAR_BUTTON_GAP
        + if show_recent {
            PROMPT_RECENT_WIDTH + PROMPT_TOOLBAR_BUTTON_GAP
        } else {
            0
        };
    let redo_x = undo_x + PROMPT_UNDO_REDO_WIDTH + PROMPT_TOOLBAR_BUTTON_GAP;
    let rect = match button {
        PromptToolbarButton::Mode => Rect::new(bar.x, bar.y, PROMPT_TOOLBAR_BUTTON_WIDTH, 1),
        PromptToolbarButton::Recent => {
            if !show_recent {
                return None;
            }
            Rect::new(
                bar.x + PROMPT_TOOLBAR_BUTTON_WIDTH + PROMPT_TOOLBAR_BUTTON_GAP,
                bar.y,
                PROMPT_RECENT_WIDTH,
                1,
            )
        }
        PromptToolbarButton::TempPrompt => {
            if mode != PromptMode::Tree || inner.width < PROMPT_TEMP_PROMPT_MIN_WIDTH {
                return None;
            }
            Rect::new(
                bar.right().saturating_sub(PROMPT_TEMP_PROMPT_WIDTH + 1),
                bar.y,
                PROMPT_TEMP_PROMPT_WIDTH,
                1,
            )
        }
        PromptToolbarButton::Folders => {
            if mode != PromptMode::Editor {
                return None;
            }
            let x = match counter {
                Some(counter) => {
                    let controls = toolbar_page_controls(bar, counter, true)?;
                    controls
                        .prev
                        .x
                        .checked_sub(1)?
                        .checked_sub(PROMPT_FOLDERS_WIDTH)?
                }
                // 未绑定分页时 `[refDir]` 仍可用：回到保存点左侧的旧位置。
                None => bar
                    .right()
                    .checked_sub(1 + PROMPT_TOOLBAR_BUTTON_GAP + PROMPT_FOLDERS_WIDTH)?,
            };
            let left_end = if show_undo_redo {
                redo_x + PROMPT_UNDO_REDO_WIDTH
            } else if show_recent {
                bar.x + tree_recent_left
            } else {
                bar.x + PROMPT_TOOLBAR_BUTTON_WIDTH
            };
            if x <= left_end {
                return None;
            }
            Rect::new(x, bar.y, PROMPT_FOLDERS_WIDTH, 1)
        }
        PromptToolbarButton::Undo => {
            if !show_undo_redo {
                return None;
            }
            Rect::new(undo_x, bar.y, PROMPT_UNDO_REDO_WIDTH, 1)
        }
        PromptToolbarButton::Redo => {
            if !show_undo_redo {
                return None;
            }
            Rect::new(redo_x, bar.y, PROMPT_UNDO_REDO_WIDTH, 1)
        }
        PromptToolbarButton::Save => {
            if mode != PromptMode::Editor {
                return None;
            }
            Rect::new(bar.right().saturating_sub(1), bar.y, 1, 1)
        }
    };
    (rect.right() <= bar.right()).then_some(rect)
}

/// 命中 prompt 工具栏按钮；表头隐藏、非工具栏行或未命中时为 None。
pub fn prompt_toolbar_button_at(
    panel: Rect,
    column: u16,
    row: u16,
    mode: PromptMode,
    counter: Option<&str>,
) -> Option<PromptToolbarButton> {
    let bar = prompt_toolbar_rect(panel)?;
    if row != bar.y {
        return None;
    }
    [
        PromptToolbarButton::Mode,
        PromptToolbarButton::Recent,
        PromptToolbarButton::Folders,
        PromptToolbarButton::TempPrompt,
        PromptToolbarButton::Undo,
        PromptToolbarButton::Redo,
        PromptToolbarButton::Save,
    ]
    .into_iter()
    .find(|button| {
        prompt_toolbar_button_rect(panel, *button, mode, counter)
            .is_some_and(|rect| rect.contains((column, row).into()))
    })
}

/// prompt 文本区尺寸：预留滚动条槽，最小 1x1。
pub fn prompt_inner_size(rect: Rect) -> (u16, u16) {
    let text = prompt_text_rect(rect);
    (text.width.max(1), text.height.max(1))
}

#[cfg(test)]
mod tests;
