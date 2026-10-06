//! 终端网格渲染：把仿真终端视口画进 ratatui Buffer，只读不写。

use alacritty_terminal::term::TermMode;
use alacritty_terminal::term::cell::{Cell, Flags};
use alacritty_terminal::vte::ansi::{Color as VteColor, NamedColor};
use ratatui::buffer::{Buffer, CellDiffOption};
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};

use crate::terminal::{Terminal, WheelRouting};
use crate::ui::widgets::scrollbar::{self, ScrollbarLayout};

/// 终端滚动条几何：仅本地回滚模式且有回滚内容时，覆盖在内容区最右一列。
///
/// 鼠标上报（opencode/claude 等）与备用屏应用自己滚动，不显示。
pub fn scrollbar(inner: Rect, terminal: &Terminal) -> Option<ScrollbarLayout> {
    if terminal.wheel_routing() != WheelRouting::HostScroll {
        return None;
    }
    let history = terminal.history_size();
    if history == 0 || inner.width == 0 || inner.height == 0 {
        return None;
    }
    scrollbar::layout(
        inner,
        history + usize::from(inner.height),
        usize::from(inner.height),
        history.saturating_sub(terminal.display_offset()),
    )
}

/// 渲染终端内容；仿真器选区命中的单元格反显高亮。
///
/// 返回聚焦且光标可见时仿真光标在 `area` 内的坐标；调用方据此同步硬件光标，
/// 让 IME 预输入与候选窗跟随终端光标（显示与闪烁由终端原生光标承担）。
pub fn render(
    area: Rect,
    buf: &mut Buffer,
    terminal: &Terminal,
    focused: bool,
) -> Option<(u16, u16)> {
    if area.width == 0 || area.height == 0 {
        return None;
    }

    let content = terminal.renderable_content();
    let offset = content.display_offset as i32;
    for indexed in content.display_iter {
        let row = indexed.point.line.0 + offset;
        if row < 0 || row as u16 >= area.height {
            continue;
        }
        let col = indexed.point.column.0 as u16;
        if col >= area.width {
            continue;
        }
        let x = area.x + col;
        let y = area.y + row as u16;
        let cell = indexed.cell;

        if cell
            .flags
            .intersects(Flags::WIDE_CHAR_SPACER | Flags::LEADING_WIDE_CHAR_SPACER)
        {
            if let Some(target) = buf.cell_mut((x, y)) {
                target.set_diff_option(CellDiffOption::Skip);
            }
            continue;
        }

        if let Some(target) = buf.cell_mut((x, y)) {
            if cell.flags.contains(Flags::HIDDEN) {
                target.set_char(' ');
            } else {
                target.set_char(cell.c);
            }
            let mut style = cell_style(cell);
            if content
                .selection
                .is_some_and(|selection| selection.contains(indexed.point))
            {
                style = style.add_modifier(Modifier::REVERSED);
            }
            target.set_style(style);
            target.set_diff_option(CellDiffOption::None);
        }
    }

    let cursor = (focused && terminal.mode().contains(TermMode::SHOW_CURSOR))
        .then(|| terminal.cursor_viewport())
        .flatten()
        .filter(|(row, col)| *row < usize::from(area.height) && *col < usize::from(area.width));
    cursor.map(|(row, col)| (row as u16, col as u16))
}

/// 单元格样式：颜色与修饰符映射到 ratatui。
fn cell_style(cell: &Cell) -> Style {
    let mut style = Style::default();
    if let Some(fg) = map_color(cell.fg) {
        style = style.fg(fg);
    }
    if let Some(bg) = map_color(cell.bg) {
        style = style.bg(bg);
    }
    let flags = cell.flags;
    if flags.contains(Flags::BOLD) {
        style = style.add_modifier(Modifier::BOLD);
    }
    if flags.contains(Flags::DIM) {
        style = style.add_modifier(Modifier::DIM);
    }
    if flags.contains(Flags::ITALIC) {
        style = style.add_modifier(Modifier::ITALIC);
    }
    if flags.contains(Flags::UNDERLINE) {
        style = style.add_modifier(Modifier::UNDERLINED);
    }
    if flags.contains(Flags::INVERSE) {
        style = style.add_modifier(Modifier::REVERSED);
    }
    style
}

/// 终端调色板颜色到 ratatui 颜色；默认前景/背景返回 None 交给终端。
fn map_color(color: VteColor) -> Option<Color> {
    match color {
        VteColor::Named(named) => match named {
            NamedColor::Black => Some(Color::Black),
            NamedColor::Red => Some(Color::Red),
            NamedColor::Green => Some(Color::Green),
            NamedColor::Yellow => Some(Color::Yellow),
            NamedColor::Blue => Some(Color::Blue),
            NamedColor::Magenta => Some(Color::Magenta),
            NamedColor::Cyan => Some(Color::Cyan),
            NamedColor::White => Some(Color::Gray),
            NamedColor::BrightBlack => Some(Color::DarkGray),
            NamedColor::BrightRed => Some(Color::LightRed),
            NamedColor::BrightGreen => Some(Color::LightGreen),
            NamedColor::BrightYellow => Some(Color::LightYellow),
            NamedColor::BrightBlue => Some(Color::LightBlue),
            NamedColor::BrightMagenta => Some(Color::LightMagenta),
            NamedColor::BrightCyan => Some(Color::LightCyan),
            NamedColor::BrightWhite => Some(Color::White),
            _ => None,
        },
        VteColor::Indexed(index) => Some(Color::Indexed(index)),
        VteColor::Spec(rgb) => Some(Color::Rgb(rgb.r, rgb.g, rgb.b)),
    }
}

#[cfg(test)]
mod tests;
