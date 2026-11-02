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
/// prompt 块左侧色条占位（`▎` + 0 列间隔）；右侧不再占位。
pub const PROMPT_BLOCK_GUTTER_LEFT: u16 = 1;

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
/// 引用文件夹按钮（`[refDir]`）宽度（列）；仅编辑视图显示、位于保存点左侧。
pub const PROMPT_FOLDERS_WIDTH: u16 = 8;
/// 编辑视图左组 `[≡] [←] [→]` 所需的最小内容区宽度（列）：左组总宽 11 + 1 列间隔 + 1 列保存点 = 13。
const PROMPT_LEFT_GROUP_MIN_WIDTH: u16 = PROMPT_TOOLBAR_BUTTON_WIDTH
    + PROMPT_TOOLBAR_BUTTON_GAP
    + PROMPT_UNDO_REDO_WIDTH
    + PROMPT_TOOLBAR_BUTTON_GAP
    + PROMPT_UNDO_REDO_WIDTH
    + PROMPT_TOOLBAR_BUTTON_GAP
    + 1;
/// 显示引用文件夹按钮 `[refDir]`（8 列）所需的最小内容区宽度（列）：在保存点左侧让出宽度与间距（13 + 1 + 8 = 22）。
pub(crate) const PROMPT_FOLDERS_MIN_WIDTH_EDITOR: u16 =
    PROMPT_LEFT_GROUP_MIN_WIDTH + PROMPT_TOOLBAR_BUTTON_GAP + PROMPT_FOLDERS_WIDTH;

/// 提示词面板视图模式。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PromptMode {
    /// 树状管理器：文件夹与条目列表。
    Tree,
    /// 编辑视图：编辑选中条目正文。
    Editor,
    /// 页列表：当前编辑目标的页目录（编辑绑定保留，编辑缓冲原样驻留不渲染）。
    Pages,
}

/// prompt 工具栏按钮。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PromptToolbarButton {
    /// 视图模式切换：编辑视图显示 `[≡]`、树视图显示 `[/]`。
    Mode,
    /// 树视图 `[tmp]`：新建临时提示词编辑视图（仅树视图显示、不入库）。
    TempPrompt,
    /// 编辑视图 `[refDir]`：打开引用文件夹菜单（仅编辑视图显示，位于保存点左侧）。
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

/// 工具栏按钮矩形；按钮在内容区放不下时为 None（`[refDir]` / `[tmp]`
/// 各自按阈值独立隐藏；各视图模式的按钮集不同）。
pub fn prompt_toolbar_button_rect(
    panel: Rect,
    button: PromptToolbarButton,
    mode: PromptMode,
) -> Option<Rect> {
    let bar = prompt_toolbar_rect(panel)?;
    let inner = pane_inner_rect(panel);
    let undo_x = bar.x + PROMPT_TOOLBAR_BUTTON_WIDTH + PROMPT_TOOLBAR_BUTTON_GAP;
    let redo_x = undo_x + PROMPT_UNDO_REDO_WIDTH + PROMPT_TOOLBAR_BUTTON_GAP;
    let rect = match button {
        PromptToolbarButton::Mode => Rect::new(bar.x, bar.y, PROMPT_TOOLBAR_BUTTON_WIDTH, 1),
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
            if mode != PromptMode::Editor || inner.width < PROMPT_FOLDERS_MIN_WIDTH_EDITOR {
                return None;
            }
            Rect::new(
                bar.right()
                    .saturating_sub(1 + PROMPT_TOOLBAR_BUTTON_GAP + PROMPT_FOLDERS_WIDTH),
                bar.y,
                PROMPT_FOLDERS_WIDTH,
                1,
            )
        }
        PromptToolbarButton::Undo => {
            if mode != PromptMode::Editor {
                return None;
            }
            Rect::new(undo_x, bar.y, PROMPT_UNDO_REDO_WIDTH, 1)
        }
        PromptToolbarButton::Redo => {
            if mode != PromptMode::Editor {
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
) -> Option<PromptToolbarButton> {
    let bar = prompt_toolbar_rect(panel)?;
    if row != bar.y {
        return None;
    }
    [
        PromptToolbarButton::Mode,
        PromptToolbarButton::Folders,
        PromptToolbarButton::TempPrompt,
        PromptToolbarButton::Undo,
        PromptToolbarButton::Redo,
        PromptToolbarButton::Save,
    ]
    .into_iter()
    .find(|button| {
        prompt_toolbar_button_rect(panel, *button, mode)
            .is_some_and(|rect| rect.contains((column, row).into()))
    })
}

/// prompt 文本区尺寸：预留滚动条槽，最小 1x1。
pub fn prompt_inner_size(rect: Rect) -> (u16, u16) {
    let text = prompt_text_rect(rect);
    (text.width.max(1), text.height.max(1))
}
