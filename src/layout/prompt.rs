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
    let recent_count = recent_count.min(crate::app::prompt::MAX_RECENT_PAGES);
    let (visible, drawer_height) = if recent_count == 0 {
        (0, 2)
    } else {
        let capacity = usize::from(max_height.saturating_sub(1) / 2);
        if capacity < crate::app::prompt::RECENT_MIN_VISIBLE_ENTRIES {
            return suppressed_split(body);
        }
        let visible = recent_count
            .max(crate::app::prompt::RECENT_MIN_VISIBLE_ENTRIES)
            .min(capacity);
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
