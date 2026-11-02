//! prompt 编辑器渲染：文本、markdown 高亮与选区，只读状态。

mod header;
pub mod pages;
mod panels;
pub mod tree;

pub use header::*;

use self::panels::render_panels;
pub use self::panels::{
    command_panel_contains, mention_item_at, mention_layout, mention_panel_rect, mention_tab_at,
    panel_item_at, panel_layout, panel_rect, paste_item_at, paste_layout, paste_panel_rect,
    refer_item_at, refer_layout, refer_panel_rect, slash_item_at, slash_layout, slash_panel_rect,
};

use ratatui::buffer::{Buffer, CellDiffOption};
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use crate::app::markdown::{BlockKind, BlockLineInfo, BlockRole, block_line_infos};
use crate::app::prompt::{Prompt, PromptMode, VisualRow};
use crate::app::selection::Selection;
use crate::layout::{self, PromptToolbarButton};
use crate::ui::markdown::{self, Token, TokenKind};
use crate::ui::style;
use crate::ui::text;

/// 分割线整行 `─`（位于按钮行下一行，不穿过功能按钮），
/// 两端以 `├` / `┤` 衔接外层边框；样式跟随边框焦点态。
pub(crate) fn render_divider(panel: Rect, header: Rect, buf: &mut Buffer, focused: bool) {
    render_rule(
        panel,
        header.y + layout::PROMPT_TOOLBAR_HEIGHT,
        buf,
        focused,
    );
}

/// 在指定行画一条与容器外框衔接的横向分割线（`├ ─ ┤`）；样式跟随边框焦点态。
pub(crate) fn render_rule(panel: Rect, y: u16, buf: &mut Buffer, focused: bool) {
    let inner = layout::pane_inner_rect(panel);
    for x in inner.x..inner.right() {
        if let Some(cell) = buf.cell_mut((x, y)) {
            cell.reset();
            cell.set_symbol(text::DIVIDER_MID);
            cell.set_style(style::border(focused));
        }
    }
    for (x, symbol) in [
        (panel.x, text::DIVIDER_LEFT_JOIN),
        (panel.right().saturating_sub(1), text::DIVIDER_RIGHT_JOIN),
    ] {
        if let Some(cell) = buf.cell_mut((x, y)) {
            cell.reset();
            cell.set_symbol(symbol);
            cell.set_style(style::border(focused));
        }
    }
}

/// 工具栏：模式切换按钮常显；编辑视图左组为 `[≡] [←] [→]`，
/// 右组自右向左为 `● [refDir]`；树视图为 `[/]` + `[tmp]`；页列表视图为 `[/]`。
fn render_toolbar(
    panel: Rect,
    buf: &mut Buffer,
    prompt: &Prompt,
    mode: PromptMode,
    draft: bool,
    _pinned: bool,
) {
    let mode_label = match mode {
        PromptMode::Editor => text::PROMPT_MODE_LIST_LABEL,
        PromptMode::Tree | PromptMode::Pages => text::PROMPT_MODE_EDIT_LABEL,
    };
    let mut buttons = vec![(PromptToolbarButton::Mode, mode_label, style::accent())];
    match mode {
        PromptMode::Tree => {
            buttons.push((
                PromptToolbarButton::TempPrompt,
                text::PROMPT_TEMP_PROMPT_LABEL,
                style::accent(),
            ));
        }
        PromptMode::Pages => {}
        PromptMode::Editor => {
            buttons.extend([
                (
                    PromptToolbarButton::Undo,
                    text::PROMPT_UNDO_LABEL,
                    if prompt.can_undo() {
                        style::accent()
                    } else {
                        style::muted()
                    },
                ),
                (
                    PromptToolbarButton::Redo,
                    text::PROMPT_REDO_LABEL,
                    if prompt.can_redo() {
                        style::accent()
                    } else {
                        style::muted()
                    },
                ),
                (
                    PromptToolbarButton::Folders,
                    text::PROMPT_FOLDERS_LABEL,
                    style::accent(),
                ),
                (
                    PromptToolbarButton::Save,
                    if draft {
                        text::PROMPT_DRAFT_DOT
                    } else {
                        text::PROMPT_SAVE_DOT
                    },
                    if draft {
                        style::status_dot_draft()
                    } else {
                        style::status_dot(prompt.is_saved())
                    },
                ),
            ]);
        }
    }
    for (button, label, button_style) in buttons {
        let Some(rect) = layout::prompt_toolbar_button_rect(panel, button, mode) else {
            continue;
        };
        // 整格重置：按钮槽位宽度固定，两态切换时不留旧字形残影。
        for x in rect.x..rect.right() {
            let Some(cell) = buf.cell_mut((x, rect.y)) else {
                break;
            };
            cell.reset();
            cell.set_char(' ');
            cell.set_style(button_style);
        }
        // 标签左对齐；所有按钮两态同宽，无需再按槽宽补空格。
        for (offset, symbol) in label.chars().enumerate() {
            let x = rect.x + offset as u16;
            if x >= rect.right() {
                break;
            }
            if let Some(cell) = buf.cell_mut((x, rect.y)) {
                cell.set_char(symbol);
            }
        }
    }
}

/// 绘制 prompt 内容区；光标由调用方以终端原生硬件光标呈现。
///
/// 块行按类型渲染：`&&&` 状态色、`+++` 固定品红；起始行顶边右端
/// 常显该类型的操作按钮，文本超宽以省略号截断。
pub fn render(area: Rect, buf: &mut Buffer, prompt: &Prompt, selection: Option<&Selection>) {
    if area.width == 0 || area.height == 0 {
        return;
    }
    let rows = prompt.visual_rows();
    let tokens = markdown::scan(prompt.text());
    let infos = block_line_infos(prompt.text());
    let lines: Vec<&str> = prompt.text().split('\n').collect();
    let mut line_starts = Vec::new();
    let mut offset = 0;
    for line in &lines {
        line_starts.push(offset);
        offset += line.len() + 1;
    }
    let scroll = prompt.scroll().min(rows.len().saturating_sub(1));
    let mut separator_rows = Vec::new();
    for (index, row) in rows
        .iter()
        .enumerate()
        .skip(scroll)
        .take(usize::from(area.height))
    {
        let y = area.y + (index - scroll) as u16;
        let info = infos.get(row.line).copied().flatten();
        if row.top_border {
            if let Some(info) = info {
                paint_block_top_border(buf, area, y, info);
            }
            if let Some(selection) = selection {
                paint_selection(buf, area, y, index as u16, selection);
            }
            continue;
        }
        let line_tokens = tokens.get(row.line).map(Vec::as_slice).unwrap_or_default();
        let base = row.start - line_starts.get(row.line).copied().unwrap_or(row.start);
        let first = first_text_row(&rows, index);
        let last = index + 1 >= rows.len() || rows[index + 1].line != row.line;
        let start_row = info.is_some_and(|info| info.role == BlockRole::Start && first);
        let text_start = area.x + block_gutter(info.is_some());
        let buttons_start = if start_row {
            let labels = block_button_labels(info);
            block_buttons_start(area, block_buttons_total(&labels))
        } else {
            area.right()
        };
        let text_end = block_text_end(area, start_row, buttons_start);
        let geometry = RowGeometry {
            start_x: text_start,
            end_x: text_end,
            ellipsize: start_row,
        };
        paint_row(buf, y, prompt.text(), row, line_tokens, base, geometry);
        if let Some(selection) = selection {
            paint_selection(buf, area, y, index as u16, selection);
        }
        if let Some(info) = info {
            paint_block_bar(buf, area, y, info);
            if first && last && info.separator {
                separator_rows.push((y, info));
            }
        }
    }
    render_block_buttons(buf, area, prompt);
    render_template_buttons(buf, area, prompt);
    for (y, info) in separator_rows {
        paint_block_separator(buf, area, y, info);
    }
    if prompt.text().is_empty() {
        paint_placeholder(buf, area);
    }
    render_panels(buf, area, prompt);
}

/// 空内容输入说明：内容区首行左侧暗色占位，超出宽度的部分裁剪。
fn paint_placeholder(buf: &mut Buffer, area: Rect) {
    let mut x = area.x;
    for ch in text::PROMPT_PLACEHOLDER.chars() {
        let width = ch.width().unwrap_or(0);
        if x.saturating_add(width as u16) > area.right() {
            break;
        }
        if let Some(cell) = buf.cell_mut((x, area.y)) {
            cell.reset();
            cell.set_char(ch);
            cell.set_style(style::muted());
        }
        x = x.saturating_add(width as u16);
    }
}

/// 模板块操作按钮种类（状态切换到标题行的状态标签，块内只保留动作按钮）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TemplateBlockButton {
    Copy,
    Clean,
    Delete,
}

/// 模板块操作按钮命中：起始逻辑行索引、按钮与绘制矩形。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TemplateButtonHit {
    pub line: usize,
    pub button: TemplateBlockButton,
    pub rect: Rect,
}

/// 补充块操作按钮种类。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BlockButton {
    Copy,
    Clean,
    Delete,
}

/// 补充块操作按钮命中。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BlockButtonHit {
    pub line: usize,
    pub kind: BlockKind,
    pub button: BlockButton,
    pub rect: Rect,
}

/// 按钮组顺序：复制、清理、删除。
const TEMPLATE_BUTTONS: [TemplateBlockButton; 3] = [
    TemplateBlockButton::Copy,
    TemplateBlockButton::Clean,
    TemplateBlockButton::Delete,
];

/// 补充块按钮组顺序：复制、清理、删除。
const SUPPLE_BUTTONS: [BlockButton; 3] =
    [BlockButton::Copy, BlockButton::Clean, BlockButton::Delete];

/// 是否为所属逻辑行的首个文本视觉行（跳过块顶边装饰行）。
fn first_text_row(rows: &[VisualRow], index: usize) -> bool {
    let row = rows[index];
    !row.top_border
        && (index == 0 || rows[index - 1].line != row.line || rows[index - 1].top_border)
}

/// 块行左侧色条槽宽度：块行为 `PROMPT_BLOCK_GUTTER_LEFT`，其余为 0。
fn block_gutter(block: bool) -> u16 {
    if block {
        layout::PROMPT_BLOCK_GUTTER_LEFT
    } else {
        0
    }
}

/// 按钮组文案（按块类型）。
fn block_button_labels(info: Option<BlockLineInfo>) -> Vec<&'static str> {
    let Some(info) = info else {
        return Vec::new();
    };
    match info.kind {
        BlockKind::Template => template_button_labels().to_vec(),
        BlockKind::Supple => SUPPLE_BUTTONS
            .iter()
            .map(|button| block_button_label(*button))
            .collect(),
    }
}

/// 按钮组总宽（含 1 列间距）。
fn block_buttons_total(labels: &[&str]) -> u16 {
    let width: usize = labels.iter().map(|label| label.width()).sum();
    (width + labels.len().saturating_sub(1)) as u16
}

/// 按钮组起点：右对齐贴文本区右缘；极窄时钳到左侧色条之后（按钮常显、优先于文本）。
fn block_buttons_start(area: Rect, total: u16) -> u16 {
    area.right()
        .saturating_sub(total)
        .max(area.x + layout::PROMPT_BLOCK_GUTTER_LEFT)
}

/// 视觉行文本区右边界（不含）：普通行与非起始块行均到文本区右缘；
/// 起始行再为按钮组让出一列间距（空间不足时退到左侧色条之后，文本仅剩省略号）。
fn block_text_end(area: Rect, start_row: bool, buttons_start: u16) -> u16 {
    if start_row {
        area.right()
            .min(buttons_start.saturating_sub(1))
            .max(area.x + layout::PROMPT_BLOCK_GUTTER_LEFT)
    } else {
        area.right()
    }
}

/// 计算可见模板块起始行的操作按钮布局。
pub fn template_buttons(prompt: &Prompt, area: Rect) -> Vec<TemplateButtonHit> {
    if area.width == 0 || area.height == 0 {
        return Vec::new();
    }
    let rows = prompt.visual_rows();
    let infos = crate::app::markdown::template_line_infos(prompt.text());
    let scroll = prompt.scroll();
    let mut hits = Vec::new();
    for (index, row) in rows.iter().enumerate() {
        if !first_text_row(&rows, index) {
            continue;
        }
        let Some(visible) = index.checked_sub(scroll) else {
            continue;
        };
        if visible >= usize::from(area.height) {
            continue;
        }
        let Some(info) = infos.get(row.line).copied().flatten() else {
            continue;
        };
        if info.role != crate::app::markdown::TemplateLineRole::Start {
            continue;
        }
        let labels = template_button_labels();
        let start = block_buttons_start(area, block_buttons_total(&labels));
        let y = area.y + visible as u16;
        let mut x = start;
        for (button, label) in TEMPLATE_BUTTONS.iter().zip(labels) {
            let width = label.width() as u16;
            hits.push(TemplateButtonHit {
                line: row.line,
                button: *button,
                rect: Rect::new(x, y, width, 1),
            });
            x = x.saturating_add(width + 1);
        }
    }
    hits
}

/// 命中模板块操作按钮；未命中返回 None。
pub fn template_button_at(
    prompt: &Prompt,
    area: Rect,
    column: u16,
    row: u16,
) -> Option<TemplateButtonHit> {
    template_buttons(prompt, area)
        .into_iter()
        .find(|hit| hit.rect.contains((column, row).into()))
}

/// 计算可见补充块起始行的操作按钮布局。
pub fn block_buttons(prompt: &Prompt, area: Rect) -> Vec<BlockButtonHit> {
    if area.width == 0 || area.height == 0 {
        return Vec::new();
    }
    let rows = prompt.visual_rows();
    let infos = block_line_infos(prompt.text());
    let scroll = prompt.scroll();
    let mut hits = Vec::new();
    for (index, row) in rows.iter().enumerate() {
        if !first_text_row(&rows, index) {
            continue;
        }
        let Some(visible) = index.checked_sub(scroll) else {
            continue;
        };
        if visible >= usize::from(area.height) {
            continue;
        }
        let Some(info) = infos.get(row.line).copied().flatten() else {
            continue;
        };
        if info.role != BlockRole::Start || info.kind != BlockKind::Supple {
            continue;
        }
        let labels = block_button_labels(Some(info));
        let start = block_buttons_start(area, block_buttons_total(&labels));
        let y = area.y + visible as u16;
        let buttons: &[BlockButton] = &SUPPLE_BUTTONS;
        let mut x = start;
        for (button, label) in buttons.iter().zip(labels) {
            let width = label.width() as u16;
            hits.push(BlockButtonHit {
                line: row.line,
                kind: info.kind,
                button: *button,
                rect: Rect::new(x, y, width, 1),
            });
            x = x.saturating_add(width + 1);
        }
    }
    hits
}

/// 命中补充块操作按钮；未命中返回 None。
pub fn block_button_at(
    prompt: &Prompt,
    area: Rect,
    column: u16,
    row: u16,
) -> Option<BlockButtonHit> {
    block_buttons(prompt, area)
        .into_iter()
        .find(|hit| hit.rect.contains((column, row).into()))
}

/// 模板块动作按钮文案（状态标签在标题行渲染）。
fn template_button_label(button: TemplateBlockButton) -> &'static str {
    match button {
        TemplateBlockButton::Copy => text::TEMPLATE_BUTTON_COPY,
        TemplateBlockButton::Clean => text::TEMPLATE_BUTTON_CLEAN,
        TemplateBlockButton::Delete => text::TEMPLATE_BUTTON_DEL,
    }
}

/// 补充块按钮文案。
fn block_button_label(button: BlockButton) -> &'static str {
    match button {
        BlockButton::Copy => text::TEMPLATE_BUTTON_COPY,
        BlockButton::Clean => text::SUPPLE_BUTTON_CLEAN,
        BlockButton::Delete => text::SUPPLE_BUTTON_DEL,
    }
}

/// 按钮组文案（复制/清理/删除）。
fn template_button_labels() -> [&'static str; 3] {
    TEMPLATE_BUTTONS.map(template_button_label)
}

/// 块颜色：`&&&` 固定 lx 粉（`style::template_block_bar`）、`+++` 固定品红（`style::supple_border`）。
fn block_color(info: BlockLineInfo) -> Style {
    match info.kind {
        BlockKind::Template => style::template_block_bar(),
        BlockKind::Supple => style::supple_border(),
    }
}

/// 块行左侧色条：每个视觉行（含软换行续行）首列画 `▎`，色随块类型/状态。
fn paint_block_bar(buf: &mut Buffer, area: Rect, y: u16, info: BlockLineInfo) {
    if let Some(cell) = buf.cell_mut((area.x, y)) {
        cell.reset();
        cell.set_symbol(text::PROMPT_BLOCK_BAR);
        cell.set_style(block_color(info));
    }
}

/// 绘制块顶部边框：整行画满与左侧色条等厚的下 1/4 块 `▂`，颜色跟随所在块。
fn paint_block_top_border(buf: &mut Buffer, area: Rect, y: u16, info: BlockLineInfo) {
    let color = block_color(info);
    for x in area.x..area.right() {
        if let Some(cell) = buf.cell_mut((x, y)) {
            cell.reset();
            cell.set_symbol(text::PROMPT_BLOCK_TOP_BORDER);
            cell.set_style(color);
        }
    }
}

/// 绘制块结构分割线：首列画色条 `▎`，其余列画满 `─`，颜色跟随所在块。
fn paint_block_separator(buf: &mut Buffer, area: Rect, y: u16, info: BlockLineInfo) {
    let color = block_color(info);
    paint_block_bar(buf, area, y, info);
    for x in area.x.saturating_add(1)..area.right() {
        if let Some(cell) = buf.cell_mut((x, y)) {
            cell.reset();
            cell.set_char(text::PROMPT_BLOCK_RULE);
            cell.set_style(color);
        }
    }
}

/// 绘制模板块操作按钮。
fn render_template_buttons(buf: &mut Buffer, area: Rect, prompt: &Prompt) {
    for hit in template_buttons(prompt, area) {
        let label = template_button_label(hit.button);
        let button_style = style::template_button();
        for (offset, symbol) in label.chars().enumerate() {
            if let Some(cell) = buf.cell_mut((hit.rect.x + offset as u16, hit.rect.y)) {
                cell.reset();
                cell.set_char(symbol);
                cell.set_style(button_style);
            }
        }
    }
}

/// 绘制补充块操作按钮。
fn render_block_buttons(buf: &mut Buffer, area: Rect, prompt: &Prompt) {
    for hit in block_buttons(prompt, area) {
        let label = block_button_label(hit.button);
        let button_style = style::block_button();
        for (offset, symbol) in label.chars().enumerate() {
            if let Some(cell) = buf.cell_mut((hit.rect.x + offset as u16, hit.rect.y)) {
                cell.reset();
                cell.set_char(symbol);
                cell.set_style(button_style);
            }
        }
    }
}

/// 行绘制几何：文本区起止 x（不含右端）与超宽是否以 `…` 截断。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct RowGeometry {
    start_x: u16,
    end_x: u16,
    ellipsize: bool,
}

/// 绘制一个视觉行：按 token 着色，宽字符占位单元格标记为跳过；
/// 返回最后一个绘制单元格之后的 x（供边框补线）；`ellipsize` 时超宽以 `…` 收尾。
///
/// `base` 为视觉行起点在逻辑行内的字节偏移；token 区间按逻辑行计。
fn paint_row(
    buf: &mut Buffer,
    y: u16,
    text: &str,
    row: &VisualRow,
    tokens: &[Token],
    base: usize,
    geometry: RowGeometry,
) -> u16 {
    let RowGeometry {
        start_x,
        end_x,
        ellipsize,
    } = geometry;
    let mut x = start_x;
    let mut token_index = 0;
    let mut clipped = false;
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
        if x.saturating_add(width as u16) > end_x {
            clipped = true;
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
    if clipped && ellipsize && end_x > start_x {
        if let Some(cell) = buf.cell_mut((end_x - 1, y)) {
            cell.reset();
            cell.set_char('…');
            cell.set_style(style::muted());
        }
        x = end_x;
    }
    x
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

/// token 语义到样式的映射（prompt 与 lx 输入框共用）。
pub(crate) fn token_style(kind: TokenKind) -> Style {
    match kind {
        TokenKind::Marker => style::markdown_marker(),
        TokenKind::HeadingMarker => style::markdown_heading_marker(),
        TokenKind::Heading => style::markdown_heading(),
        TokenKind::Strong => style::markdown_strong(),
        TokenKind::Emphasis => style::markdown_emphasis(),
        TokenKind::Strikethrough => style::markdown_strikethrough(),
        TokenKind::InlineCode => style::markdown_inline_code(),
        TokenKind::CodeBlock => style::markdown_code_block(),
        TokenKind::Quote => style::markdown_quote(),
        TokenKind::LinkText => style::markdown_link_text(),
        TokenKind::Url => style::markdown_url(),
        TokenKind::ListBullet => style::markdown_bullet(),
        TokenKind::ListOrdered => style::markdown_ordered(),
        TokenKind::TaskOpen => style::markdown_task_open(),
        TokenKind::TaskDone => style::markdown_task_done(),
        TokenKind::TemplateMarker => style::template_marker(),
        TokenKind::TemplateCommand(id) => style::template_command(id),
        TokenKind::TemplateTitle => style::template_title(),
        TokenKind::SuppleMarker => style::supple_marker(),
        TokenKind::SuppleCommand => style::supple_command(),
        TokenKind::FileMention => style::markdown_file_mention(),
        TokenKind::ReferFile => style::markdown_refer_file(),
        TokenKind::ReferFolder => style::markdown_refer_folder(),
        TokenKind::ReferImage => style::markdown_refer_image(),
        TokenKind::ShortRef => style::markdown_short_ref(),
        TokenKind::RefContent => style::markdown_ref_content(),
    }
}

#[cfg(test)]
#[path = "prompt/blocks/tests.rs"]
mod block_tests;
#[cfg(test)]
mod tests;
