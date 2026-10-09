//! lx 欢迎页：顶部品牌区（像素小狐狸 + 字标）、白色占位面板与底部输入框。
//! 只写 Buffer；颜色全部取自 `ui::style` 的吉祥物/占位调色板。

use ratatui::buffer::{Buffer, Cell};
use ratatui::layout::{Alignment, Rect};
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Paragraph, Widget};

use crate::ui::{style, text};

/// 狐狸像素图尺寸（列 × 像素行 = 单元格行 × 2）。
const FOX_WIDTH: u16 = 18;
const FOX_ROWS: u16 = 7;

/// 底部输入框高度（含边框）与最小宽度。
const INPUT_HEIGHT: u16 = 3;
const MIN_INPUT_WIDTH: u16 = 8;
/// 区块之间保留的空行。
const SECTION_GAP: u16 = 1;
/// 占位面板至少需要的高度（上下边框 + 一行内容）。
const PANEL_MIN_HEIGHT: u16 = 3;

/// 全尺寸狐狸像素图（18 列 × 14 像素行，静态）。
const FOX: &[&str] = &[
    "..k........k......",
    ".kpk......kpk.....",
    ".knpk....kpnk.....",
    ".kppkkkkkkppk.....",
    ".kppppppppppk.....",
    ".kpwwppppwwpkkk...",
    ".kpkkwnnwkkpkwwk..",
    ".knppwwwwppnknwk..",
    "..kppppppppkknnk..",
    "..kppppppppkknnk..",
    "..kpwwwwwwpkknnk..",
    "..kppwwwwppkknnk..",
    "..kkk....kkkkkkk..",
    "..kkk....kkk......",
];

/// 渲染 lx 页。
///
/// 垂直结构（自底向上）：输入框贴底 → 切换提示 → 白色占位面板 → 顶部品牌区；
/// 空间不足时依次省略面板与品牌区，最小退化为居中字标。
pub fn render(area: Rect, buf: &mut Buffer) {
    if area.width == 0 || area.height == 0 {
        return;
    }
    let Some(input) = input_box(area) else {
        let y = area.y + area.height.saturating_sub(1) / 2;
        draw_centered(area, buf, y, text::LX_TITLE, style::strong());
        return;
    };
    draw_input(buf, input);

    let hint = text::lx_hint();
    let hint_fits = input.width >= hint.chars().count() as u16;
    let hint_y = input.y.checked_sub(SECTION_GAP).filter(|_| hint_fits);
    let content_bottom = hint_y.unwrap_or(input.y);

    // 顶部品牌区：狐狸在左；高度或宽度放不下整只狐狸时省略。
    let mut next_y = area.y;
    if content_bottom >= next_y + FOX_ROWS + SECTION_GAP && input.width >= FOX_WIDTH {
        draw_art(buf, input.x, next_y);
        next_y += FOX_ROWS + SECTION_GAP;
    }

    // 白色占位面板：填满品牌区与底部区之间的剩余空间。
    let panel_bottom = content_bottom.saturating_sub(SECTION_GAP);
    if panel_bottom >= next_y + PANEL_MIN_HEIGHT {
        let panel = Rect::new(input.x, next_y, input.width, panel_bottom - next_y);
        draw_panel(panel, buf);
    }

    if let Some(y) = hint_y {
        draw_left(buf, input.x, y, &hint, style::muted());
    }
}

/// 底部输入框矩形：贴内容区底部，宽屏左右各留 2 列边距；空间不足返回 None。
fn input_box(area: Rect) -> Option<Rect> {
    let margin: u16 = if area.width >= 24 { 2 } else { 0 };
    let width = area.width.saturating_sub(margin.saturating_mul(2));
    (area.height >= INPUT_HEIGHT && width >= MIN_INPUT_WIDTH).then(|| {
        Rect::new(
            area.x + margin,
            area.bottom() - INPUT_HEIGHT,
            width,
            INPUT_HEIGHT,
        )
    })
}

/// 输入框：muted 圆角边框 + `>` 前缀与占位文案；纯视觉占位，不可输入。
///
/// 占位文案按内容区宽度截断，窄窗格里不画出半截字符。
fn draw_input(buf: &mut Buffer, rect: Rect) {
    let block = Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(style::muted());
    let inner = block.inner(rect);
    block.render(rect, buf);
    if inner.width < 2 || inner.height == 0 {
        return;
    }
    let placeholder = text::ellipsize(
        text::LX_INPUT_PLACEHOLDER,
        usize::from(inner.width.saturating_sub(2)),
    );
    Paragraph::new(Line::from(vec![
        Span::styled(format!("{} ", text::LX_INPUT_PROMPT), style::accent()),
        Span::styled(placeholder, style::muted()),
    ]))
    .render(Rect::new(inner.x, inner.y, inner.width, 1), buf);
}

/// 白色占位面板：未来内容区的边界，内部居中占位文案（放不下时只留边框）。
fn draw_panel(rect: Rect, buf: &mut Buffer) {
    let block = Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(style::lx_placeholder());
    let inner = block.inner(rect);
    block.render(rect, buf);
    if inner.width == 0 || inner.height == 0 {
        return;
    }
    if usize::from(inner.width) < text::LX_CONTENT_PLACEHOLDER.chars().count() {
        return;
    }
    let y = inner.y + inner.height / 2;
    draw_centered(
        inner,
        buf,
        y,
        text::LX_CONTENT_PLACEHOLDER,
        style::lx_placeholder(),
    );
}

/// 把静态像素图画进以 `(x, y)` 为左上角的单元格区域。
fn draw_art(buf: &mut Buffer, x: u16, y: u16) {
    for (row, pair) in FOX.chunks(2).enumerate() {
        let [top, bottom] = pair else {
            break;
        };
        let cell_y = y + row as u16;
        for column in 0..FOX_WIDTH {
            let Some(cell) = buf.cell_mut((x + column, cell_y)) else {
                continue;
            };
            let top = top.chars().nth(usize::from(column)).unwrap_or('.');
            let bottom = bottom.chars().nth(usize::from(column)).unwrap_or('.');
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

/// 在 `y` 行以 `x` 起始左对齐绘制一段带样式的文本。
fn draw_left(buf: &mut Buffer, x: u16, y: u16, content: &str, style: Style) {
    Paragraph::new(Span::styled(content.to_string(), style)).render(
        Rect::new(
            x,
            y,
            u16::try_from(content.chars().count()).unwrap_or(u16::MAX),
            1,
        ),
        buf,
    );
}

/// 在 `y` 行按 `area` 宽度水平居中绘制一段带样式的文本。
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
