//! 引用文件夹目录弹窗：几何、命中与渲染。
//!
//! 居中模态，与路径输入弹窗、内容搜索弹窗同宽同高（固定尺寸，不随记录数变化）；
//! 候选为 prompt 列表式单行：选中行 `❯` 标记、目录三角、名称与右端 muted 绝对路径，
//! 末行为 `+ add folder…` 动作项，底边为快捷键提示。

use ratatui::Frame;
use ratatui::layout::{Alignment, Rect};
use ratatui::style::Style;
use ratatui::text::Span;
use ratatui::widgets::{Block, BorderType};
use unicode_width::UnicodeWidthStr;

use crate::app::overlay::{ReferencedFolderMenu, ReferencedFolderMenuRow};
use crate::app::referenced::menu_window_start;
use crate::ui::widgets::clear;
use crate::ui::{style, text, widgets};

use super::{fill_row, paint_text};

/// 目录弹窗宽度（列）；与路径输入弹窗、内容搜索弹窗一致。
const MENU_WIDTH: u16 = 72;
/// 目录弹窗高度（行）；固定高度，不随记录数变化。
const MENU_HEIGHT: u16 = 18;
/// 行首标记列宽（`❯ ` 与空标记占位）。
const MARKER_WIDTH: u16 = 2;
/// 名称与右端地址之间的最小间隔（列）。
const NAME_GAP: u16 = 1;
/// 右端地址低于该宽度时整段省略（放不下有意义的内容）。
const ADDRESS_MIN_WIDTH: u16 = 4;

/// 目录弹窗几何：整体区域与可见行（每行一项、高 1 行）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReferencedMenuLayout {
    pub area: Rect,
    /// 可见行矩形（顺序对应窗口内行，1 行高）。
    pub rows: Vec<Rect>,
    /// 窗口内首行的行下标。
    pub start: usize,
    /// 行总数。
    pub total: usize,
    /// 列表滚动条；不溢出时为 None。
    pub scrollbar: Option<widgets::scrollbar::ScrollbarLayout>,
}

/// 目录弹窗几何；渲染与鼠标命中共用。
pub fn referenced_menu_layout(screen: Rect, menu: &ReferencedFolderMenu) -> ReferencedMenuLayout {
    let width = MENU_WIDTH.min(screen.width);
    let height = MENU_HEIGHT.min(screen.height);
    let x = screen.x + (screen.width - width) / 2;
    let y = screen.y + (screen.height - height) / 2;
    let area = Rect::new(x, y, width, height);
    let inner = Block::bordered().inner(area);
    let total = menu.rows.len();
    let visible = usize::from(inner.height);
    // 列表区固定预留右缘 1 列为滚动条槽（与 prompt 列表一致）。
    let list = Rect::new(
        inner.x,
        inner.y,
        inner.width.saturating_sub(1),
        inner.height,
    );
    // 显式视口优先（滚轮滚动，高亮允许不在窗口内）；否则窗口跟随高亮。
    let max_start = total.saturating_sub(visible);
    let selected = menu.selected.min(total.saturating_sub(1));
    let start = menu
        .viewport
        .map(|offset| offset.min(max_start))
        .unwrap_or_else(|| menu_window_start(selected, visible, total));
    let shown = visible.min(total.saturating_sub(start));
    let rows = (0..shown)
        .map(|offset| Rect::new(list.x, list.y + offset as u16, list.width, 1))
        .collect();
    // 列表溢出时在最右一列显示滚动条（轨道贴内容区右缘；行文本已为其让出该列）。
    let scrollbar = widgets::scrollbar::layout(
        Rect {
            width: inner.width,
            ..list
        },
        total,
        visible.max(1),
        start,
    );
    ReferencedMenuLayout {
        area,
        rows,
        start,
        total,
        scrollbar,
    }
}

impl ReferencedMenuLayout {
    /// 列表可见行数。
    pub fn visible(&self) -> usize {
        self.rows.len()
    }
}

/// 命中目录弹窗行；返回行下标。
pub fn referenced_menu_row_at(
    screen: Rect,
    menu: &ReferencedFolderMenu,
    column: u16,
    row: u16,
) -> Option<usize> {
    let layout = referenced_menu_layout(screen, menu);
    layout
        .rows
        .iter()
        .position(|rect| rect.contains((column, row).into()))
        .map(|offset| layout.start + offset)
}

/// 渲染目录弹窗：边框标题与 prompt 列表式行（底边提示由调用方叠加）。
pub fn render_referenced_menu(frame: &mut Frame<'_>, screen: Rect, menu: &ReferencedFolderMenu) {
    let layout = referenced_menu_layout(screen, menu);
    clear::render(frame, layout.area);
    let block = Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(style::accent())
        .style(style::overlay_panel())
        .title_alignment(Alignment::Left)
        .title(Span::styled(
            format!(" {} ", text::MENU_TITLE_REFERENCED_FOLDERS),
            style::border_title(),
        ));
    frame.render_widget(block, layout.area);
    for (offset, rect) in layout.rows.iter().enumerate() {
        let Some(row) = menu.rows.get(layout.start + offset) else {
            continue;
        };
        let index = layout.start + offset;
        // 空状态/不可用行非可选：不显示标记，也不铺悬停底色（对齐 prompt 树的分区标签行）。
        let selectable = !matches!(
            row,
            ReferencedFolderMenuRow::Empty | ReferencedFolderMenuRow::Unavailable
        );
        let hovered = selectable && menu.hover == Some(index);
        render_menu_row(
            frame,
            *rect,
            row,
            selectable && menu.selected == index,
            hovered,
        );
    }
    // 列表溢出时在最右一列绘制滚动条（轨道 muted、thumb 亮色）。
    if let Some(scrollbar) = layout.scrollbar {
        widgets::scrollbar::render_with_base(frame, &scrollbar, style::overlay_panel());
    }
}

/// 渲染目录弹窗底边快捷键提示（贴底边，覆盖边框上）。
pub fn render_referenced_menu_footer(
    frame: &mut Frame<'_>,
    screen: Rect,
    menu: &ReferencedFolderMenu,
) {
    let layout = referenced_menu_layout(screen, menu);
    let y = layout.area.bottom().saturating_sub(1);
    paint_text(
        frame,
        Rect::new(layout.area.x + 2, y, layout.area.width.saturating_sub(4), 1),
        text::REFERENCED_MENU_FOOTER,
        style::muted_on_border(),
    );
}

/// 单行：悬停时整行铺选中底色；空状态/不可用为 muted 提示，目录记录为
/// `❯` 标记 + 三角图标 + 名称（右端 muted 绝对路径），新建行为强调色动作项。
///
/// `selected` 只决定 `❯` 标记（仅点击/键盘移动），**不铺底色**；`hovered` 只叠加悬停底色、
/// 不显示标记（完全对齐 prompt 树列表）。
fn render_menu_row(
    frame: &mut Frame<'_>,
    rect: Rect,
    row: &ReferencedFolderMenuRow,
    selected: bool,
    hovered: bool,
) {
    // 背景颜色只在悬停时出现；选中行仅显示 `❯` 标记，底色保持面板本色。
    let base = if hovered {
        style::overlay_selection()
    } else {
        style::overlay_panel()
    };
    fill_row(frame, rect, base);
    match row {
        ReferencedFolderMenuRow::Empty => {
            paint_text(
                frame,
                rect,
                text::REFERENCED_MENU_EMPTY,
                base.patch(style::muted()),
            );
        }
        ReferencedFolderMenuRow::Unavailable => {
            paint_text(
                frame,
                rect,
                text::REFERENCED_DB_UNAVAILABLE,
                base.patch(style::muted()),
            );
        }
        ReferencedFolderMenuRow::Add => {
            let x = paint_marker(frame, rect, selected, base);
            paint_text_x(
                frame,
                rect,
                x,
                text::REFERENCED_MENU_ADD_ROW,
                base.patch(style::accent()),
            );
        }
        ReferencedFolderMenuRow::Folder { path, .. } => {
            let mut x = paint_marker(frame, rect, selected, base);
            x = paint_text_x(
                frame,
                rect,
                x,
                text::REFERENCED_DIR_ICON,
                base.patch(style::accent()),
            );
            x = paint_text_x(frame, rect, x, " ", base);
            let available = rect.right().saturating_sub(x);
            // 右端 muted 绝对路径（中间截断）；剩余宽度不足时整段省略，名称独占。
            let reserved = address_width(path, available);
            if reserved > 0 {
                let address =
                    text::ellipsize_middle(&path.display().to_string(), usize::from(reserved));
                paint_text(
                    frame,
                    sub_rect(rect, rect.right().saturating_sub(reserved)),
                    &address,
                    base.patch(style::muted()),
                );
            }
            let name_max = available
                .saturating_sub(reserved)
                .saturating_sub(if reserved > 0 { NAME_GAP } else { 0 });
            paint_text(
                frame,
                sub_rect(rect, x),
                &text::truncate_to_width(&display_name(path), usize::from(name_max)),
                base.patch(style::directory()),
            );
        }
    }
}

/// 行首标记列：选中行 `❯ `，否则等宽空格占位；返回其后起始列。
fn paint_marker(frame: &mut Frame<'_>, rect: Rect, selected: bool, base: Style) -> u16 {
    let marker = if selected {
        format!("{} ", text::AGENT_SELECTED_MARKER)
    } else {
        " ".repeat(usize::from(MARKER_WIDTH))
    };
    paint_text(frame, rect, &marker, base);
    rect.x.saturating_add(MARKER_WIDTH)
}

/// 从 `x` 起绘制一行文本；返回文本结束列（超出右界时钳在右界）。
fn paint_text_x(
    frame: &mut Frame<'_>,
    rect: Rect,
    x: u16,
    content: &str,
    cell_style: Style,
) -> u16 {
    if x >= rect.right() {
        return x;
    }
    paint_text(frame, sub_rect(rect, x), content, cell_style);
    x.saturating_add(u16::try_from(content.width()).unwrap_or(u16::MAX))
        .min(rect.right())
}

/// 右端地址保留列宽：取路径宽度与半行宽的较小值；低于下限或挤掉名称时返回 0（整段省略）。
fn address_width(path: &std::path::Path, available: u16) -> u16 {
    let full = u16::try_from(path.display().to_string().width()).unwrap_or(u16::MAX);
    let reserved = full.min(available / 2);
    if reserved < ADDRESS_MIN_WIDTH || reserved.saturating_add(NAME_GAP + 1) > available {
        return 0;
    }
    reserved
}

/// 目录末段名；无末段时回退完整路径。
fn display_name(path: &std::path::Path) -> String {
    path.file_name()
        .and_then(|name| name.to_str())
        .filter(|name| !name.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| path.display().to_string())
}

/// 行内子矩形：从 `x` 起、右界保持父行右界，避免文本溢出覆盖边框。
fn sub_rect(row: Rect, x: u16) -> Rect {
    Rect::new(x, row.y, row.right().saturating_sub(x), row.height)
}
