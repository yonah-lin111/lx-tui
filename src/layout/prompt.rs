//! prompt 右栏区域几何与工具栏按钮布局：纯函数，不涉及渲染与状态。

use ratatui::layout::Rect;

use super::pane_inner_rect;

/// prompt 工具栏高度：1（按钮行；分割线在下一行，不穿过按钮）。
pub const PROMPT_TOOLBAR_HEIGHT: u16 = 1;
/// prompt 工具栏下方分割线高度：1。
pub const PROMPT_DIVIDER_HEIGHT: u16 = 1;
/// prompt 顶部保留行数（工具栏 + 分割线）。
pub const PROMPT_HEADER_HEIGHT: u16 = PROMPT_TOOLBAR_HEIGHT + PROMPT_DIVIDER_HEIGHT;
/// 树视图顶部固定行数（搜索框 + 分割线）。
pub const PROMPT_TREE_HEADER_HEIGHT: u16 = 2;
/// prompt 模板块边框占位：左侧（`╭─`/`│ `/`╰─`）与右侧竖线（`╮`/`│`/`╯`）列数。
pub const PROMPT_TEMPLATE_GUTTER_LEFT: u16 = 2;
pub const PROMPT_TEMPLATE_GUTTER_RIGHT: u16 = 1;

/// 模式按钮（`[≡]` / `[/]`）宽度（列）。
const PROMPT_TOOLBAR_BUTTON_WIDTH: u16 = 3;
/// 撤销/重做按钮（`[←]` / `[→]`）宽度（列）。
const PROMPT_UNDO_REDO_WIDTH: u16 = 3;
/// 工具栏按钮之间的空档（列）。
const PROMPT_TOOLBAR_BUTTON_GAP: u16 = 1;
/// 树视图 `[tmp]` 按钮宽度（列）。
const PROMPT_TEMP_PROMPT_WIDTH: u16 = 5;
/// 显示 `[tmp]`（右对齐）所需的最小内容区宽度（列）：不足时隐藏，避免与左侧模式按钮重叠。
const PROMPT_TEMP_PROMPT_MIN_WIDTH: u16 =
    PROMPT_TOOLBAR_BUTTON_WIDTH + PROMPT_TOOLBAR_BUTTON_GAP + PROMPT_TEMP_PROMPT_WIDTH + 1;
/// `[all]` 标签宽度（列）；位于左组 `[≡] [←] [→]` 右侧。
const PROMPT_SELECT_ALL_WIDTH: u16 = 5;
/// 显示 `[all]` 所需的最小内容区宽度（列）：左组总宽 17 + 1 列间隔 + 1 列余量。
pub(crate) const PROMPT_SELECT_ALL_MIN_WIDTH: u16 = PROMPT_TOOLBAR_BUTTON_WIDTH
    + PROMPT_TOOLBAR_BUTTON_GAP
    + PROMPT_UNDO_REDO_WIDTH
    + PROMPT_TOOLBAR_BUTTON_GAP
    + PROMPT_UNDO_REDO_WIDTH
    + PROMPT_TOOLBAR_BUTTON_GAP
    + PROMPT_SELECT_ALL_WIDTH
    + PROMPT_TOOLBAR_BUTTON_GAP
    + 1;
/// 块聚焦按钮宽度（列）：`[⊞]` / `[⊡]`，与 `[all]` 同层右组。
const PROMPT_BLOCK_FOCUS_WIDTH: u16 = 3;
/// 显示块聚焦按钮所需的最小内容区宽度（列）：左组 17 + 1 + `[⊡]` 3 + 1 + `●` 1。
pub(crate) const PROMPT_BLOCK_FOCUS_MIN_WIDTH_EDITOR: u16 =
    PROMPT_SELECT_ALL_MIN_WIDTH + PROMPT_TOOLBAR_BUTTON_GAP + PROMPT_BLOCK_FOCUS_WIDTH;
/// 钉住按钮宽度（列）：单字图标 `[○]` / `[◉]` 恒为 3，两态同宽同址。
pub(crate) const PROMPT_PIN_WIDTH: u16 = 3;
/// 显示钉住按钮所需的最小内容区宽度（列）：编辑视图在块聚焦按钮左侧、树视图在 `[tmp]` 左侧。
pub(crate) const PROMPT_PIN_MIN_WIDTH_EDITOR: u16 =
    PROMPT_BLOCK_FOCUS_MIN_WIDTH_EDITOR + PROMPT_TOOLBAR_BUTTON_GAP + PROMPT_PIN_WIDTH;
pub(crate) const PROMPT_PIN_MIN_WIDTH_TREE: u16 =
    PROMPT_TEMP_PROMPT_MIN_WIDTH + PROMPT_TOOLBAR_BUTTON_GAP + PROMPT_PIN_WIDTH;
/// 引用文件夹按钮（`[dir]`）宽度（列）；仅编辑视图显示、位于钉住按钮左侧。
pub const PROMPT_FOLDERS_WIDTH: u16 = 5;
/// 显示引用文件夹按钮所需的最小内容区宽度（列）：在钉住按钮左侧再让出宽度与间距。
pub(crate) const PROMPT_FOLDERS_MIN_WIDTH_EDITOR: u16 =
    PROMPT_PIN_MIN_WIDTH_EDITOR + PROMPT_TOOLBAR_BUTTON_GAP + PROMPT_FOLDERS_WIDTH;

/// prompt 工具栏按钮。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PromptToolbarButton {
    /// 视图模式切换：编辑视图显示 `[≡]`、树视图显示 `[/]`。
    Mode,
    /// 钉住切换：未钉 `[○]`、已钉 `[◉]`（编辑视图在块聚焦按钮左侧、树视图在 `[tmp]` 左侧）。
    Pin,
    /// 树视图 `[tmp]`：新建临时提示词编辑视图（仅树视图显示、不入库）。
    TempPrompt,
    /// 编辑视图 `[dir]`：打开引用文件夹菜单（仅编辑视图显示，位于钉住按钮左侧）。
    Folders,
    /// 编辑视图 `[⊞]` / `[⊡]`：切换「单模板块聚焦」模式（仅编辑视图显示）。
    BlockFocus,
    Undo,
    Redo,
    SelectAll,
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

/// prompt 内容区扣除顶部表头与附件条后的文本/滚动条区域；
/// 高度不足时表头/附件条隐藏、退回全部内容区。
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

/// prompt 顶部表头矩形（工具栏 + 分割线两行）；内容区过矮时不显示。
pub fn prompt_header_rect(panel: Rect) -> Option<Rect> {
    let inner = pane_inner_rect(panel);
    (inner.height > PROMPT_HEADER_HEIGHT).then_some(Rect::new(
        inner.x,
        inner.y,
        inner.width,
        PROMPT_HEADER_HEIGHT,
    ))
}

/// prompt 面板顶边框行矩形（内嵌标题与页控件）；面板过窄时为 None。
///
/// 与边框同宽同列：标题与页控件直接覆盖边框线的 `─` 段（左边角 1 + 空档 1 起）。
pub fn prompt_border_rect(panel: Rect) -> Option<Rect> {
    (panel.width > 4 && panel.height > 0).then_some(Rect::new(panel.x, panel.y, panel.width, 1))
}

/// prompt 工具栏行矩形（表头首行）；表头隐藏时为 None。
pub fn prompt_toolbar_rect(panel: Rect) -> Option<Rect> {
    prompt_header_rect(panel).map(|header| Rect {
        height: PROMPT_TOOLBAR_HEIGHT,
        ..header
    })
}

/// prompt 工具栏下方分割线行矩形；表头隐藏时为 None。
pub fn prompt_divider_rect(panel: Rect) -> Option<Rect> {
    prompt_header_rect(panel).map(|header| Rect {
        y: header.y + PROMPT_TOOLBAR_HEIGHT,
        height: PROMPT_DIVIDER_HEIGHT,
        ..header
    })
}

/// 工具栏按钮矩形；按钮在内容区放不下时为 None（`[all]` / `[⊡]` / `[○]` / `[dir]`
/// 各自按阈值独立隐藏；树视图与编辑视图各自的按钮集不同，`tree` 为是否树视图）。
pub fn prompt_toolbar_button_rect(
    panel: Rect,
    button: PromptToolbarButton,
    tree: bool,
) -> Option<Rect> {
    let bar = prompt_toolbar_rect(panel)?;
    let inner = pane_inner_rect(panel);
    let undo_x = bar.x + PROMPT_TOOLBAR_BUTTON_WIDTH + PROMPT_TOOLBAR_BUTTON_GAP;
    let redo_x = undo_x + PROMPT_UNDO_REDO_WIDTH + PROMPT_TOOLBAR_BUTTON_GAP;
    let select_all_x = redo_x + PROMPT_UNDO_REDO_WIDTH + PROMPT_TOOLBAR_BUTTON_GAP;
    let rect = match button {
        PromptToolbarButton::Mode => Rect::new(bar.x, bar.y, PROMPT_TOOLBAR_BUTTON_WIDTH, 1),
        PromptToolbarButton::Pin => {
            let (anchor_button, min_width) = if tree {
                (PromptToolbarButton::TempPrompt, PROMPT_PIN_MIN_WIDTH_TREE)
            } else {
                (PromptToolbarButton::BlockFocus, PROMPT_PIN_MIN_WIDTH_EDITOR)
            };
            if inner.width < min_width {
                return None;
            }
            let anchor = prompt_toolbar_button_rect(panel, anchor_button, tree)?;
            Rect::new(
                anchor
                    .x
                    .saturating_sub(PROMPT_PIN_WIDTH + PROMPT_TOOLBAR_BUTTON_GAP),
                bar.y,
                PROMPT_PIN_WIDTH,
                1,
            )
        }
        PromptToolbarButton::TempPrompt => {
            if !tree || inner.width < PROMPT_TEMP_PROMPT_MIN_WIDTH {
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
            if tree || inner.width < PROMPT_FOLDERS_MIN_WIDTH_EDITOR {
                return None;
            }
            let anchor = prompt_toolbar_button_rect(panel, PromptToolbarButton::Pin, tree)?;
            Rect::new(
                anchor
                    .x
                    .saturating_sub(PROMPT_FOLDERS_WIDTH + PROMPT_TOOLBAR_BUTTON_GAP),
                bar.y,
                PROMPT_FOLDERS_WIDTH,
                1,
            )
        }
        PromptToolbarButton::BlockFocus => {
            if tree || inner.width < PROMPT_BLOCK_FOCUS_MIN_WIDTH_EDITOR {
                return None;
            }
            Rect::new(
                bar.right()
                    .saturating_sub(1 + PROMPT_TOOLBAR_BUTTON_GAP + PROMPT_BLOCK_FOCUS_WIDTH),
                bar.y,
                PROMPT_BLOCK_FOCUS_WIDTH,
                1,
            )
        }
        PromptToolbarButton::Undo => {
            if tree {
                return None;
            }
            Rect::new(undo_x, bar.y, PROMPT_UNDO_REDO_WIDTH, 1)
        }
        PromptToolbarButton::Redo => {
            if tree {
                return None;
            }
            Rect::new(redo_x, bar.y, PROMPT_UNDO_REDO_WIDTH, 1)
        }
        PromptToolbarButton::SelectAll => {
            if tree || inner.width < PROMPT_SELECT_ALL_MIN_WIDTH {
                return None;
            }
            // 左组 `[≡] [←] [→] [all]`：紧贴 `[→]` 右侧。
            Rect::new(select_all_x, bar.y, PROMPT_SELECT_ALL_WIDTH, 1)
        }
        PromptToolbarButton::Save => {
            if tree {
                return None;
            }
            Rect::new(bar.right().saturating_sub(1), bar.y, 1, 1)
        }
    };
    (rect.right() <= bar.right()).then_some(rect)
}

/// 命中 prompt 工具栏按钮；表头隐藏、非工具栏行或未命中时为 None；`tree` 为是否树视图。
pub fn prompt_toolbar_button_at(
    panel: Rect,
    column: u16,
    row: u16,
    tree: bool,
) -> Option<PromptToolbarButton> {
    let bar = prompt_toolbar_rect(panel)?;
    if row != bar.y {
        return None;
    }
    [
        PromptToolbarButton::Mode,
        PromptToolbarButton::Folders,
        PromptToolbarButton::Pin,
        PromptToolbarButton::BlockFocus,
        PromptToolbarButton::TempPrompt,
        PromptToolbarButton::Undo,
        PromptToolbarButton::Redo,
        PromptToolbarButton::SelectAll,
        PromptToolbarButton::Save,
    ]
    .into_iter()
    .find(|button| {
        prompt_toolbar_button_rect(panel, *button, tree)
            .is_some_and(|rect| rect.contains((column, row).into()))
    })
}

/// prompt 文本区尺寸：预留滚动条槽，最小 1x1。
pub fn prompt_inner_size(rect: Rect) -> (u16, u16) {
    let text = prompt_text_rect(rect);
    (text.width.max(1), text.height.max(1))
}
