//! lx 欢迎页品牌区：像素字标、Tip 提示与标签页式入口。
//!
//! 只读状态、只写 lx 页网格或 Buffer；图标像素画与信任页品牌区共用。

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Style;
use unicode_width::UnicodeWidthStr;

use crate::app::state::LxEntry;
use crate::app::state::lx::{StartupCount, StartupCounts};
use crate::ui::{style, text};

use super::grid::Grid;

/// 图标像素画尺寸（列 × 像素行 = 单元格行 × 2）。
pub(crate) const LOGO_WIDTH: u16 = 20;
pub(crate) const LOGO_ROWS: u16 = 7;
/// 图标底边与 Tip 行之间的行距。
pub(crate) const LOGO_GAP: u16 = 2;
/// 选中入口下方的下划线行（字符与占用行数）。
const ENTRY_UNDERLINE: char = '─';
pub(super) const UNDERLINE_ROWS: u16 = 1;

/// 基准 LX 像素字标（20 列 × 14 像素行）；'.' 透明，其余字符取 `style::logo_pixel` 调色板。
///
/// 三个入口共用同一形状，仅按入口做配色微调，保证 `LX` 形象一致。
pub(crate) const ICON_LX: &[&str] = &[
    ".kkk...kkkkk....kkkk",
    "kppbk..kccck...kpppk",
    "kppbk..kbbbk...kppnk",
    "kppbk..kbbbk..kppnnk",
    "kppbk..kkbbbbkppnnk.",
    "kppbk...kbbbkppnnk..",
    "kppbk....kbkppnnk...",
    "kppbk....kbkppnk....",
    "kppbk....kppnnknpk..",
    "kppbk...kppnnknnpk..",
    "kppbk..kppnnknnnnpk.",
    "kppbkppkpppnkk.knnnk",
    "kbbbbbkpnnkk...knnnk",
    "kkkkkkkkkkk.....kkkk",
];

/// 按入口对基准字标做配色微调：形状不变、只换调色板槽位。
pub(crate) fn icon_pixel(entry: LxEntry, ch: char) -> char {
    match entry {
        LxEntry::Native => ch,
        // 粉/深粉 与 天蓝/冰蓝 互换。
        LxEntry::Hybrid => match ch {
            'p' => 'n',
            'n' => 'p',
            'b' => 'c',
            'c' => 'b',
            other => other,
        },
        // 粉系 与 蓝系 互换。
        LxEntry::Automation => match ch {
            'p' => 'c',
            'c' => 'p',
            'b' => 'n',
            'n' => 'b',
            other => other,
        },
    }
}

/// 放置图标：水平居中、底边距 Tip `LOGO_GAP` 行；放不下（宽度不足或越过内容区顶部）返回 None。
pub(super) fn place_icon(
    tip_y: u16,
    region_top: u16,
    content_x: u16,
    content_w: u16,
) -> Option<(u16, u16)> {
    if content_w < LOGO_WIDTH {
        return None;
    }
    let top = tip_y
        .checked_sub(LOGO_GAP + LOGO_ROWS)
        .filter(|top| *top >= region_top)?;
    let x = content_x + content_w.saturating_sub(LOGO_WIDTH) / 2;
    Some((x, top))
}

/// 绘制像素字标（按入口调色）。
pub(super) fn draw_icon(grid: &mut Grid, x: u16, y: u16, entry: LxEntry) {
    for (row, pair) in ICON_LX.chunks(2).enumerate() {
        let [top, bottom] = pair else {
            break;
        };
        let cell_y = y + row as u16;
        for column in 0..LOGO_WIDTH {
            let top = icon_pixel(entry, top.chars().nth(usize::from(column)).unwrap_or('.'));
            let bottom = icon_pixel(
                entry,
                bottom.chars().nth(usize::from(column)).unwrap_or('.'),
            );
            if let Some((ch, pixel_style)) = pixel_glyph(top, bottom) {
                grid.set(x + column, cell_y, ch, pixel_style);
            }
        }
    }
}

/// 直接把基准 LX 图标像素画进缓冲区（信任页品牌区复用；不走整页网格）。
pub(crate) fn draw_art(buf: &mut Buffer, x: u16, y: u16) {
    for (row, pair) in ICON_LX.chunks(2).enumerate() {
        let [top, bottom] = pair else {
            break;
        };
        let cell_y = y + row as u16;
        for column in 0..LOGO_WIDTH {
            let top = top.chars().nth(usize::from(column)).unwrap_or('.');
            let bottom = bottom.chars().nth(usize::from(column)).unwrap_or('.');
            if let Some((ch, pixel_style)) = pixel_glyph(top, bottom)
                && let Some(cell) = buf.cell_mut((x + column, cell_y))
            {
                cell.set_char(ch);
                cell.set_style(pixel_style);
            }
        }
    }
}

/// 两个纵向像素画进一个单元格：同色实块，异色上半块（前景上/背景下）。
fn pixel_glyph(top: char, bottom: char) -> Option<(char, Style)> {
    match (style::logo_pixel(top), style::logo_pixel(bottom)) {
        (None, None) => None,
        (Some(color), None) => Some(('▀', Style::default().fg(color))),
        (None, Some(color)) => Some(('▄', Style::default().fg(color))),
        (Some(color), Some(_)) if top == bottom => Some(('█', Style::default().fg(color))),
        (Some(top_color), Some(bottom_color)) => {
            Some(('▀', Style::default().fg(top_color).bg(bottom_color)))
        }
    }
}

/// 首页资源计数：`1 AGENTS.md · 4 mcp · 3 skills`；失败追加 `1 error`。
pub(super) fn draw_inventory(grid: &mut Grid, rect: Rect, counts: StartupCounts) {
    if rect.width == 0 {
        return;
    }
    let sep = (format!(" {} ", text::LX_ENTRY_SEPARATOR), style::muted());
    let agents_md = (
        format!("{} {}", counts.agents_md, text::LX_HOME_AGENTS_MD),
        style::chat_agents_md(),
    );
    let mcp = inventory_parts(counts.mcp, text::LX_HOME_MCP, style::chat_tool_verb());
    let skills = inventory_parts(
        counts.skills,
        text::LX_HOME_SKILLS,
        style::chat_tool_param(),
    );
    let mut segments: Vec<(String, Style)> = vec![agents_md, sep.clone()];
    segments.extend(mcp);
    segments.push(sep);
    segments.extend(skills);
    let width: usize = segments.iter().map(|(value, _)| value.width()).sum();
    let width = u16::try_from(width).unwrap_or(u16::MAX);
    // 超宽时从左边起画，后面的段由 Grid 自然裁掉。
    let mut x = if width > rect.width {
        rect.x
    } else {
        rect.x + rect.width.saturating_sub(width) / 2
    };
    let mut remaining = usize::from(rect.width);
    for (value, value_style) in segments {
        if remaining == 0 {
            break;
        }
        let value = text::truncate_to_width(&value, remaining);
        if value.is_empty() {
            break;
        }
        grid.put(x, rect.y, &value, value_style);
        let used = value.width().min(remaining);
        x = x.saturating_add(u16::try_from(used).unwrap_or(u16::MAX));
        remaining = remaining.saturating_sub(used);
    }
}

fn inventory_parts(count: StartupCount, label: &str, color: Style) -> Vec<(String, Style)> {
    let mut parts = vec![(format!("{} {label}", count.ready), color)];
    if count.errors > 0 {
        let label = if count.errors == 1 {
            text::LX_HOME_ERROR
        } else {
            text::LX_HOME_ERRORS
        };
        parts.push((" ".to_string(), style::muted()));
        parts.push((format!("{} {label}", count.errors), style::agent_fail()));
    }
    parts
}

/// 绘制 Tip 提示行：`Tip:` 前缀强调色、其后句子次要信息；整体居中、超宽截断。
pub(super) fn draw_tip(grid: &mut Grid, rect: Rect, entry: LxEntry) {
    let full = format!(
        "{} {}",
        text::LX_TIP_PREFIX,
        text::lx_entry_description(entry)
    );
    let full = text::truncate_to_width(&full, usize::from(rect.width));
    let full_w = u16::try_from(full.width()).unwrap_or(u16::MAX);
    let start = rect.x + rect.width.saturating_sub(full_w) / 2;
    let Some(rest) = full.strip_prefix(text::LX_TIP_PREFIX) else {
        grid.put(start, rect.y, &full, style::muted());
        return;
    };
    grid.put(start, rect.y, text::LX_TIP_PREFIX, style::accent());
    let prefix_w = u16::try_from(text::LX_TIP_PREFIX.width()).unwrap_or(0);
    grid.put(start + prefix_w, rect.y, rest, style::muted());
}

/// 标签页式入口：标签按该入口 logo 粉→蓝渐变着色、选中加粗；横排用 `·` 分隔。
pub(super) fn draw_entries(
    grid: &mut Grid,
    selected: LxEntry,
    entries: &[(LxEntry, Rect)],
    horizontal: bool,
) {
    for (index, (entry, rect)) in entries.iter().enumerate() {
        let label = text::lx_entry_label(*entry);
        let length = label.chars().count();
        for (position, ch) in label.chars().enumerate() {
            let cell_style =
                style::lx_entry_label_style(*entry, position, length, *entry == selected);
            grid.set(
                rect.x.saturating_add(position as u16),
                rect.y,
                ch,
                cell_style,
            );
        }
        if *entry == selected {
            for position in 0..length {
                let cell_style = style::lx_entry_label_style(*entry, position, length, false);
                grid.set(
                    rect.x.saturating_add(position as u16),
                    rect.y + UNDERLINE_ROWS,
                    ENTRY_UNDERLINE,
                    cell_style,
                );
            }
        }
        if horizontal && let Some((_, next)) = entries.get(index + 1) {
            let gap = next.x.saturating_sub(rect.right());
            grid.put(
                rect.right() + gap / 2,
                rect.y,
                text::LX_ENTRY_SEPARATOR,
                style::muted(),
            );
        }
    }
}
