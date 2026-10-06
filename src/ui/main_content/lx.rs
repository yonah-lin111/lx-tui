//! lx 欢迎页：像素狐狸（半块渲染 + 200ms 帧动画）、字标与切换提示。
//! 只读相位、只写 Buffer；颜色全部取自 `ui::style` 的吉祥物调色板。

use ratatui::buffer::{Buffer, Cell};
use ratatui::layout::{Alignment, Rect};
use ratatui::style::Style;
use ratatui::text::Span;
use ratatui::widgets::{Paragraph, Widget};

use crate::ui::{style, text};

/// 全尺寸狐狸像素图尺寸（列 × 像素行 = 单元格行 × 2）。
const FOX_WIDTH: u16 = 18;
const FOX_ROWS: u16 = 7;
/// 紧凑狐狸头尺寸。
const HEAD_WIDTH: u16 = 10;
const HEAD_ROWS: u16 = 4;

/// 空白像素行（18 列）。
const BLANK_18: &str = "..................";
/// 空白像素行（10 列）。
const BLANK_10: &str = "..........";

/// 全尺寸狐狸底图：耳/尾为覆盖层，眼睛为睁眼。
const FOX_BASE: &[&str] = &[
    BLANK_18,
    BLANK_18,
    BLANK_18,
    "..kkkkkkkkkkkkkk..",
    ".kppppppppppppppk.",
    ".kppppppppppppppk.",
    ".kppkwwppppwwkppk.",
    ".kppkkkppppkkkppk.",
    ".kppppppnnppppppk.",
    ".kkkkkkkkkkkkkkkk.",
    "..kkkkkkkkkkk.....",
    "..kpppppppppk.....",
    "..kpbbbbbbbpk.....",
    "..kkkk....kkkk....",
];

/// 立耳（常态）。
const FOX_EARS_UP: &[&str] = &[
    "...k..........k...",
    "..kbk........kbk..",
    "..kkk........kkk..",
    BLANK_18,
    BLANK_18,
    BLANK_18,
    BLANK_18,
    BLANK_18,
    BLANK_18,
    BLANK_18,
    BLANK_18,
    BLANK_18,
    BLANK_18,
    BLANK_18,
];

/// 抖耳：右耳下沉一像素。
const FOX_EARS_TWITCH: &[&str] = &[
    "...k..............",
    "..kbk.........k...",
    "..kkk........kbk..",
    BLANK_18,
    BLANK_18,
    BLANK_18,
    BLANK_18,
    BLANK_18,
    BLANK_18,
    BLANK_18,
    BLANK_18,
    BLANK_18,
    BLANK_18,
    BLANK_18,
];

/// 摆尾：尾梢朝内。
const FOX_TAIL_IN: &[&str] = &[
    BLANK_18,
    BLANK_18,
    BLANK_18,
    BLANK_18,
    BLANK_18,
    BLANK_18,
    BLANK_18,
    BLANK_18,
    BLANK_18,
    BLANK_18,
    "..kkkkkkkkkkk..n..",
    "..kpppppppppk.nnn.",
    "..kpbbbbbbbpkknnnk",
    "..kkkk....kkkk.kk.",
];

/// 摆尾：尾梢朝外。
const FOX_TAIL_OUT: &[&str] = &[
    BLANK_18,
    BLANK_18,
    BLANK_18,
    BLANK_18,
    BLANK_18,
    BLANK_18,
    BLANK_18,
    BLANK_18,
    BLANK_18,
    BLANK_18,
    "..kkkkkkkkkkk...n.",
    "..kpppppppppk..nnn",
    "..kpbbbbbbbpk.knnn",
    "..kkkk....kkkk..kk",
];

/// 眨眼：白色眼白转为描边线，瞳孔行还原毛色。
const FOX_BLINK: &[&str] = &[
    BLANK_18,
    BLANK_18,
    BLANK_18,
    BLANK_18,
    BLANK_18,
    BLANK_18,
    ".kppkkkppppkkkppk.",
    ".kppppppppppppppk.",
    BLANK_18,
    BLANK_18,
    BLANK_18,
    BLANK_18,
    BLANK_18,
    BLANK_18,
];

/// 紧凑狐狸头底图。
const HEAD_BASE: &[&str] = &[
    "..k....k..",
    ".kkkkkkkk.",
    ".kppppppk.",
    ".kpwwkwwpk",
    ".kpkkpkkpk",
    ".kppnnppk.",
    ".kkkkkkkk.",
    BLANK_10,
];

/// 紧凑狐狸头眨眼。
const HEAD_BLINK: &[&str] = &[
    BLANK_10,
    BLANK_10,
    BLANK_10,
    ".kpkkkkkpk",
    ".kpppppppk",
    BLANK_10,
    BLANK_10,
    BLANK_10,
];

/// 分层像素图：底图 + 可选覆盖层；覆盖层非 `.` 像素替换底图。
struct Art {
    base: &'static [&'static str],
    ears: Option<[&'static [&'static str]; 2]>,
    tail: Option<[&'static [&'static str]; 2]>,
    blink: Option<&'static [&'static str]>,
}

/// 全尺寸狐狸。
const FOX: Art = Art {
    base: FOX_BASE,
    ears: Some([FOX_EARS_UP, FOX_EARS_TWITCH]),
    tail: Some([FOX_TAIL_IN, FOX_TAIL_OUT]),
    blink: Some(FOX_BLINK),
};

/// 紧凑狐狸头。
const FOX_HEAD: Art = Art {
    base: HEAD_BASE,
    ears: None,
    tail: None,
    blink: Some(HEAD_BLINK),
};

/// 页面降级形态。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Page {
    /// 全尺寸狐狸 + 字标（可选提示）。
    Fox { hint: bool },
    /// 紧凑狐狸头（可选字标）。
    Head { wordmark: bool },
    /// 仅字标。
    Wordmark,
}

/// 渲染 lx 页；`phase` 为 200ms 粒度的动画相位。
pub fn render(area: Rect, buf: &mut Buffer, phase: u64) {
    if area.width == 0 || area.height == 0 {
        return;
    }
    let hint = text::lx_hint();
    match plan(area, hint.chars().count() as u16) {
        Page::Fox { hint: show_hint } => {
            let total = FOX_ROWS + 2 + if show_hint { 2 } else { 0 };
            let y = area.y + area.height.saturating_sub(total) / 2;
            draw_art(area, buf, y, &FOX, phase, FOX_WIDTH);
            draw_centered(area, buf, y + FOX_ROWS + 1, text::LX_TITLE, style::strong());
            if show_hint {
                draw_centered(area, buf, y + FOX_ROWS + 3, &hint, style::muted());
            }
        }
        Page::Head { wordmark } => {
            let total = HEAD_ROWS + if wordmark { 2 } else { 0 };
            let y = area.y + area.height.saturating_sub(total) / 2;
            draw_art(area, buf, y, &FOX_HEAD, phase, HEAD_WIDTH);
            if wordmark {
                draw_centered(
                    area,
                    buf,
                    y + HEAD_ROWS + 1,
                    text::LX_TITLE,
                    style::strong(),
                );
            }
        }
        Page::Wordmark => {
            let y = area.y + area.height.saturating_sub(1) / 2;
            draw_centered(area, buf, y, text::LX_TITLE, style::strong());
        }
    }
}

/// 按可用空间选择页面形态；提示只在宽度放得下时显示。
fn plan(area: Rect, hint_width: u16) -> Page {
    if area.width >= FOX_WIDTH + 2 {
        if area.height >= FOX_ROWS + 4 && area.width >= hint_width + 4 {
            return Page::Fox { hint: true };
        }
        if area.height >= FOX_ROWS + 2 {
            return Page::Fox { hint: false };
        }
    }
    if area.width >= HEAD_WIDTH + 2 && area.height >= HEAD_ROWS {
        return Page::Head {
            wordmark: area.height >= HEAD_ROWS + 2,
        };
    }
    Page::Wordmark
}

/// 按相位合成当前帧（底图 + 覆盖层），纯函数。
fn compose(art: &Art, phase: u64) -> Vec<Vec<char>> {
    let mut grid: Vec<Vec<char>> = art.base.iter().map(|row| row.chars().collect()).collect();
    for layer in layers(art, phase) {
        for (y, row) in layer.iter().enumerate() {
            for (x, ch) in row.chars().enumerate() {
                if ch == '.' {
                    continue;
                }
                if let Some(cell) = grid.get_mut(y).and_then(|row| row.get_mut(x)) {
                    *cell = ch;
                }
            }
        }
    }
    grid
}

/// 当前相位的覆盖层顺序：耳 → 尾 → 眼。
fn layers(art: &Art, phase: u64) -> Vec<&'static [&'static str]> {
    let mut layers = Vec::new();
    if let Some(ears) = art.ears {
        layers.push(ears[usize::from(matches!(phase % 24, 10 | 11))]);
    }
    if let Some(tail) = art.tail {
        layers.push(tail[(phase / 2 % 2) as usize]);
    }
    if let Some(blink) = art.blink
        && matches!(phase % 12, 6 | 7)
    {
        layers.push(blink);
    }
    layers
}

/// 把当前帧画进 `y` 起始的单元格区域，水平居中。
fn draw_art(area: Rect, buf: &mut Buffer, y: u16, art: &Art, phase: u64, width: u16) {
    let x = area.x + area.width.saturating_sub(width) / 2;
    let grid = compose(art, phase);
    for (row, pair) in grid.chunks(2).enumerate() {
        let [top, bottom] = pair else {
            break;
        };
        let cell_y = y + row as u16;
        for column in 0..width {
            let Some(cell) = buf.cell_mut((x + column, cell_y)) else {
                continue;
            };
            let top = top.get(usize::from(column)).copied().unwrap_or('.');
            let bottom = bottom.get(usize::from(column)).copied().unwrap_or('.');
            draw_pixels(cell, top, bottom);
        }
    }
}

/// 两个纵向像素画进一个单元格：同色实块，异色上半块（前景上/背景下）。
fn draw_pixels(cell: &mut Cell, top: char, bottom: char) {
    match (style::mascot_pixel(top), style::mascot_pixel(bottom)) {
        (None, None) => {}
        (Some(color), None) => {
            cell.set_char('▀');
            cell.set_style(Style::default().fg(color));
        }
        (None, Some(color)) => {
            cell.set_char('▄');
            cell.set_style(Style::default().fg(color));
        }
        (Some(color), Some(_)) if top == bottom => {
            cell.set_char('█');
            cell.set_style(Style::default().fg(color));
        }
        (Some(top_color), Some(bottom_color)) => {
            cell.set_char('▀');
            cell.set_style(Style::default().fg(top_color).bg(bottom_color));
        }
    }
}

/// 在 `y` 行水平居中绘制一段带样式的文本。
fn draw_centered(area: Rect, buf: &mut Buffer, y: u16, content: &str, style: Style) {
    if y >= area.bottom() {
        return;
    }
    let line = Rect::new(area.x, y, area.width, 1);
    Paragraph::new(Span::styled(content.to_string(), style))
        .alignment(Alignment::Center)
        .render(line, buf);
}

#[cfg(test)]
mod tests;
