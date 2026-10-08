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
use crate::ui::widgets::command_panel::{self, CommandItem, CommandPanelView, PanelLayout};

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
    if let Some(data) = mention_panel_data(prompt) {
        let items = data.items();
        command_panel::render(area, buf, &mention_view(&items, &data, area.height / 2));
        return;
    }
    let Some(data) = block_panel_data(prompt) else {
        return;
    };
    let items = block_items(&data);
    command_panel::render(area, buf, &block_view(&items, &data));
}

/// 块命令面板渲染数据：条目文本、高亮索引、窗口锚点/显式视口与锚点行。
struct BlockPanelData {
    texts: Vec<(String, String)>,
    active: usize,
    anchor: usize,
    viewport: Option<usize>,
    anchor_row: u16,
}

/// 块命令面板渲染数据；面板未打开或光标滚出视口返回 None。
fn block_panel_data(prompt: &Prompt) -> Option<BlockPanelData> {
    let panel = prompt.panel()?;
    let (anchor_row, _) = prompt.cursor_cell()?;
    Some(BlockPanelData {
        texts: panel
            .items()
            .iter()
            .map(|id| text::block_command_text(*id))
            .collect(),
        active: panel.active(),
        anchor: panel.anchor(),
        viewport: panel.viewport(),
        anchor_row,
    })
}

/// 块命令面板条目：单行（名称 + 右侧预览）。
fn block_items<'a>(data: &'a BlockPanelData) -> Vec<CommandItem<'a>> {
    data.texts
        .iter()
        .map(|(label, preview)| CommandItem::Inline { label, preview })
        .collect()
}

/// 块命令面板视图：窗口锚定面板状态，显式视口优先，不设高度上限；顶边左侧为 `Commands` 标题。
fn block_view<'a>(items: &'a [CommandItem<'a>], data: &BlockPanelData) -> CommandPanelView<'a> {
    CommandPanelView {
        items,
        active: data.active,
        window_anchor: Some(data.anchor),
        window_start: data.viewport,
        anchor_row: data.anchor_row,
        max_height: None,
        title: Some(text::BLOCK_PANEL_TITLE),
        right_title: None,
        footer: None,
    }
}

/// 块命令面板布局；面板未打开或空间不足返回 None。
pub fn panel_layout(prompt: &Prompt, area: Rect) -> Option<PanelLayout> {
    let data = block_panel_data(prompt)?;
    let items = block_items(&data);
    command_panel::layout(area, &block_view(&items, &data))
}

/// 块命令面板命中：返回被点中的条目索引；面板未打开或未命中返回 None。
pub fn panel_item_at(prompt: &Prompt, area: Rect, column: u16, row: u16) -> Option<usize> {
    let data = block_panel_data(prompt)?;
    let items = block_items(&data);
    let layout = command_panel::layout(area, &block_view(&items, &data))?;
    command_panel::item_at(&layout, &items, column, row)
}

/// 块命令面板矩形；用于滚轮命中。
pub fn panel_rect(prompt: &Prompt, area: Rect) -> Option<Rect> {
    panel_layout(prompt, area).map(|layout| layout.rect)
}

/// 提及面板布局；面板未打开或空间不足返回 None。
pub fn mention_layout(prompt: &Prompt, area: Rect) -> Option<PanelLayout> {
    let data = mention_panel_data(prompt)?;
    let items = data.items();
    command_panel::layout(area, &mention_view(&items, &data, area.height / 2))
}

/// 提及面板命中：返回被点中的条目索引；面板未打开或未命中返回 None。
pub fn mention_item_at(prompt: &Prompt, area: Rect, column: u16, row: u16) -> Option<usize> {
    let data = mention_panel_data(prompt)?;
    let items = data.items();
    let layout = command_panel::layout(area, &mention_view(&items, &data, area.height / 2))?;
    command_panel::item_at(&layout, &items, column, row)
}

/// 提及面板矩形；用于滚轮命中。
pub fn mention_panel_rect(prompt: &Prompt, area: Rect) -> Option<Rect> {
    mention_layout(prompt, area).map(|layout| layout.rect)
}

/// 提及条目文本：图标、标签与父目录明细。
struct MentionItemText {
    icon: &'static str,
    label: String,
    detail: String,
}

/// 提及面板渲染数据：条目文本、高亮索引、窗口锚点/显式视口、锚点行与当前目录名。
struct MentionPanelData {
    texts: Vec<MentionItemText>,
    active: usize,
    anchor: usize,
    viewport: Option<usize>,
    anchor_row: u16,
    /// 当前进入的目录末段名；未进入文件夹时 None。
    scope: Option<String>,
}

impl MentionPanelData {
    /// 组装条目视图；图标列固定占位，标签与明细借用自身文本。
    fn items(&self) -> Vec<CommandItem<'_>> {
        self.texts
            .iter()
            .map(|entry| CommandItem::Stacked {
                icon: Some(entry.icon),
                label: entry.label.as_str(),
                detail: entry.detail.as_str(),
            })
            .collect()
    }
}

/// 提及面板渲染数据；面板未打开或光标滚出视口返回 None。
fn mention_panel_data(prompt: &Prompt) -> Option<MentionPanelData> {
    let panel = prompt.mention()?;
    let (anchor_row, _) = prompt.cursor_cell()?;
    Some(MentionPanelData {
        texts: panel.items().iter().map(mention_item_text).collect(),
        active: panel.active(),
        anchor: panel.anchor(),
        viewport: panel.viewport(),
        anchor_row,
        scope: panel.scope_name().map(str::to_string),
    })
}

/// 提及面板视图：最大高度取容器（prompt 内容区）的一半；显式视口优先于窗口锚点。
///
/// 顶边左侧为 `Files` 标题，右侧为当前目录名（未进入文件夹时不显示），底边为目录导航快捷键。
fn mention_view<'a>(
    items: &'a [CommandItem<'a>],
    data: &'a MentionPanelData,
    max_height: u16,
) -> CommandPanelView<'a> {
    CommandPanelView {
        items,
        active: data.active,
        window_anchor: Some(data.anchor),
        window_start: data.viewport,
        anchor_row: data.anchor_row,
        max_height: Some(max_height),
        title: Some(text::MENTION_PANEL_TITLE),
        right_title: data.scope.as_deref(),
        footer: Some(text::MENTION_PANEL_FOOTER),
    }
}

/// 提及条目展示：第一行目录/文件图标 + 文件名（目录带 `/`），第二行 `└─ ` 前缀的父目录。
fn mention_item_text(entry: &MentionEntry) -> MentionItemText {
    let (directory, name) = entry
        .path
        .rsplit_once('/')
        .unwrap_or(("", entry.path.as_str()));
    let label = if entry.is_directory {
        format!("{name}/")
    } else {
        name.to_string()
    };
    let detail = if directory.is_empty() {
        String::new()
    } else {
        format!("{} {directory}", text::WORKSPACE_TREE_LAST)
    };
    let icon = if entry.is_directory {
        text::MENTION_DIR_ICON
    } else {
        text::MENTION_FILE_ICON
    };
    MentionItemText {
        icon,
        label,
        detail,
    }
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
