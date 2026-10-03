//! 终端网格渲染：把仿真终端视口画进 ratatui Buffer，只读不写。

use alacritty_terminal::term::TermMode;
use alacritty_terminal::term::cell::{Cell, Flags};
use alacritty_terminal::vte::ansi::{Color as VteColor, NamedColor};
use ratatui::buffer::{Buffer, CellDiffOption};
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};

use crate::terminal::Terminal;

/// 渲染终端内容；`focused` 为真时叠加光标反显。
pub fn render(area: Rect, buf: &mut Buffer, terminal: &Terminal, focused: bool) {
    if area.width == 0 || area.height == 0 {
        return;
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
            target.set_style(cell_style(cell));
            target.set_diff_option(CellDiffOption::None);
        }
    }

    if focused
        && terminal.mode().contains(TermMode::SHOW_CURSOR)
        && let Some((row, col)) = terminal.cursor_viewport()
        && (row as u16) < area.height
        && (col as u16) < area.width
        && let Some(target) = buf.cell_mut((area.x + col as u16, area.y + row as u16))
    {
        target.modifier |= Modifier::REVERSED;
    }
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
mod tests {
    use super::*;

    #[test]
    fn renders_text_and_skips_wide_spacers() {
        let mut terminal = Terminal::new(10, 2);
        terminal.feed("hi 你好".as_bytes());
        let mut buf = Buffer::empty(Rect::new(0, 0, 10, 2));
        render(Rect::new(0, 0, 10, 2), &mut buf, &terminal, false);
        assert_eq!(buf[(0, 0)].symbol(), "h");
        assert_eq!(buf[(1, 0)].symbol(), "i");
        assert_eq!(buf[(3, 0)].symbol(), "你");
        assert_eq!(buf[(4, 0)].diff_option, CellDiffOption::Skip);
        assert_eq!(buf[(5, 0)].symbol(), "好");
        assert_eq!(buf[(6, 0)].diff_option, CellDiffOption::Skip);
    }

    #[test]
    fn focused_terminal_paints_reversed_cursor() {
        let mut terminal = Terminal::new(10, 2);
        terminal.feed(b"ab");
        let mut buf = Buffer::empty(Rect::new(0, 0, 10, 2));
        render(Rect::new(0, 0, 10, 2), &mut buf, &terminal, true);
        assert!(buf[(2, 0)].modifier.contains(Modifier::REVERSED));
    }
}
