//! prompt 编辑器渲染：文本、markdown 高亮与选区，只读状态。

use ratatui::buffer::{Buffer, CellDiffOption};
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use unicode_width::UnicodeWidthChar;

use crate::app::markdown::MentionEntry;
use crate::app::prompt::{Prompt, VisualRow};
use crate::app::selection::Selection;
use crate::ui::markdown::{self, Token, TokenKind};
use crate::ui::style;
use crate::ui::text;
use crate::ui::widgets::command_panel::{self, CommandItem, CommandPanelView};

/// 绘制 prompt 内容区；光标由调用方以终端原生硬件光标呈现。
pub fn render(area: Rect, buf: &mut Buffer, prompt: &Prompt, selection: Option<&Selection>) {
    if area.width == 0 || area.height == 0 {
        return;
    }
    let rows = prompt.visual_rows();
    let tokens = markdown::scan(prompt.text());
    let mut line_starts = Vec::new();
    let mut offset = 0;
    for line in prompt.text().split('\n') {
        line_starts.push(offset);
        offset += line.len() + 1;
    }
    let scroll = prompt.scroll().min(rows.len().saturating_sub(1));
    for (index, row) in rows
        .iter()
        .enumerate()
        .skip(scroll)
        .take(usize::from(area.height))
    {
        let y = area.y + (index - scroll) as u16;
        let line_tokens = tokens.get(row.line).map(Vec::as_slice).unwrap_or_default();
        let base = row.start - line_starts.get(row.line).copied().unwrap_or(row.start);
        paint_row(buf, area, y, prompt.text(), row, line_tokens, base);
        if let Some(selection) = selection {
            paint_selection(buf, area, y, index as u16, selection);
        }
    }
    render_panels(buf, area, prompt);
}

/// 绘制浮层面板：文件提及优先，其次块命令；状态由 app 层维护，这里只做只读映射。
fn render_panels(buf: &mut Buffer, area: Rect, prompt: &Prompt) {
    let Some((anchor_row, _)) = prompt.cursor_cell() else {
        return;
    };
    if let Some(panel) = prompt.mention() {
        let texts: Vec<(String, String)> = panel.items().iter().map(mention_item_text).collect();
        let items: Vec<CommandItem<'_>> = texts
            .iter()
            .map(|(label, detail)| CommandItem::Stacked { label, detail })
            .collect();
        command_panel::render(
            area,
            buf,
            &mention_view(
                &items,
                panel.active(),
                panel.anchor(),
                anchor_row,
                area.height / 2,
            ),
        );
        return;
    }
    let Some(panel) = prompt.panel() else {
        return;
    };
    let copy: Vec<(String, String)> = panel
        .items()
        .iter()
        .map(|id| text::block_command_text(*id))
        .collect();
    let items: Vec<CommandItem<'_>> = copy
        .iter()
        .map(|(label, preview)| CommandItem::Inline { label, preview })
        .collect();
    command_panel::render(
        area,
        buf,
        &CommandPanelView {
            items: &items,
            active: panel.active(),
            window_anchor: None,
            anchor_row,
            max_height: None,
        },
    );
}

/// 提及面板命中：返回被点中的条目索引；面板未打开或未命中返回 None。
pub fn mention_item_at(prompt: &Prompt, area: Rect, column: u16, row: u16) -> Option<usize> {
    let data = mention_panel_data(prompt)?;
    let items: Vec<CommandItem<'_>> = data
        .texts
        .iter()
        .map(|(label, detail)| CommandItem::Stacked { label, detail })
        .collect();
    let layout = command_panel::layout(
        area,
        &mention_view(
            &items,
            data.active,
            data.anchor,
            data.anchor_row,
            area.height / 2,
        ),
    )?;
    command_panel::item_at(&layout, &items, column, row)
}

/// 提及面板矩形；用于滚轮命中。
pub fn mention_panel_rect(prompt: &Prompt, area: Rect) -> Option<Rect> {
    let data = mention_panel_data(prompt)?;
    let items: Vec<CommandItem<'_>> = data
        .texts
        .iter()
        .map(|(label, detail)| CommandItem::Stacked { label, detail })
        .collect();
    command_panel::layout(
        area,
        &mention_view(
            &items,
            data.active,
            data.anchor,
            data.anchor_row,
            area.height / 2,
        ),
    )
    .map(|layout| layout.rect)
}

/// 提及面板渲染数据：条目文本、高亮索引、窗口锚点与锚点行。
struct MentionPanelData {
    texts: Vec<(String, String)>,
    active: usize,
    anchor: usize,
    anchor_row: u16,
}

/// 提及面板渲染数据；面板未打开或光标滚出视口返回 None。
fn mention_panel_data(prompt: &Prompt) -> Option<MentionPanelData> {
    let panel = prompt.mention()?;
    let (anchor_row, _) = prompt.cursor_cell()?;
    Some(MentionPanelData {
        texts: panel.items().iter().map(mention_item_text).collect(),
        active: panel.active(),
        anchor: panel.anchor(),
        anchor_row,
    })
}

/// 提及面板视图：最大高度取容器（prompt 内容区）的一半；窗口锚定面板状态。
fn mention_view<'a>(
    items: &'a [CommandItem<'a>],
    active: usize,
    anchor: usize,
    anchor_row: u16,
    max_height: u16,
) -> CommandPanelView<'a> {
    CommandPanelView {
        items,
        active,
        window_anchor: Some(anchor),
        anchor_row,
        max_height: Some(max_height),
    }
}

/// 提及条目展示：第一行文件名（目录带 `/`），第二行父目录。
fn mention_item_text(entry: &MentionEntry) -> (String, String) {
    let (directory, name) = entry
        .path
        .rsplit_once('/')
        .unwrap_or(("", entry.path.as_str()));
    let label = if entry.is_directory {
        format!("{name}/")
    } else {
        name.to_string()
    };
    (label, directory.to_string())
}

/// 绘制一个视觉行：按 token 着色，宽字符占位单元格标记为跳过。
///
/// `base` 为视觉行起点在逻辑行内的字节偏移；token 区间按逻辑行计。
fn paint_row(
    buf: &mut Buffer,
    area: Rect,
    y: u16,
    text: &str,
    row: &VisualRow,
    tokens: &[Token],
    base: usize,
) {
    let mut x = area.x;
    let mut token_index = 0;
    for (offset, ch) in text[row.start..row.end].char_indices() {
        let width = ch.width().unwrap_or(0);
        if width == 0 {
            continue;
        }
        let absolute = base + offset;
        while tokens
            .get(token_index)
            .is_some_and(|token| token.range.end <= absolute)
        {
            token_index += 1;
        }
        let style = tokens
            .get(token_index)
            .filter(|token| token.range.contains(&absolute))
            .map_or_else(style::text, |token| token_style(token.kind));
        if x.saturating_add(width as u16) > area.right() {
            break;
        }
        if let Some(cell) = buf.cell_mut((x, y)) {
            cell.reset();
            cell.set_char(ch);
            cell.set_style(style);
            cell.set_diff_option(CellDiffOption::None);
        }
        for dx in 1..width as u16 {
            if let Some(cell) = buf.cell_mut((x + dx, y)) {
                cell.set_diff_option(CellDiffOption::Skip);
            }
        }
        x += width as u16;
    }
}

/// 选区按内容行坐标反显；空白单元格同样覆盖，保证拖拽范围可见。
fn paint_selection(buf: &mut Buffer, area: Rect, y: u16, row: u16, selection: &Selection) {
    for col in 0..area.width {
        if selection.contains(i32::from(row), col)
            && let Some(cell) = buf.cell_mut((area.x + col, y))
        {
            cell.modifier |= Modifier::REVERSED;
        }
    }
}

/// token 语义到样式的映射。
fn token_style(kind: TokenKind) -> Style {
    match kind {
        TokenKind::Marker => style::markdown_marker(),
        TokenKind::Heading => style::markdown_heading(),
        TokenKind::Strong => style::markdown_strong(),
        TokenKind::Emphasis => style::markdown_emphasis(),
        TokenKind::Strikethrough => style::markdown_strikethrough(),
        TokenKind::InlineCode => style::markdown_inline_code(),
        TokenKind::CodeBlock => style::markdown_code_block(),
        TokenKind::Quote => style::markdown_quote(),
        TokenKind::LinkText => style::markdown_link_text(),
        TokenKind::Url => style::markdown_url(),
    }
}

#[cfg(test)]
mod tests;
