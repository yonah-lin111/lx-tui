//! prompt 编辑器渲染：文本、markdown 高亮与选区，只读状态。

pub mod tree;

use ratatui::buffer::{Buffer, CellDiffOption};
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use crate::app::markdown::{
    MentionEntry, TemplateLineInfo, TemplateLineRole, TemplateStatus, template_line_infos,
};
use crate::app::prompt::{Prompt, PromptMode, VisualRow};
use crate::app::selection::Selection;
use crate::layout::{self, PromptToolbarButton};
use crate::ui::markdown::{self, Token, TokenKind};
use crate::ui::style;
use crate::ui::text;
use crate::ui::widgets::command_panel::{self, CommandItem, CommandPanelView, PanelLayout};

/// 绘制 prompt 顶部工具栏与固定分割线；内容区过矮时不绘制；
/// 编辑视图在分割线行内嵌编辑目标 title（`├─ <title> ────┤`），
/// 绑定临时提示词时 title 右端追加 accent `[tmp]` 标识；
/// `draft` 表示当前绑定临时提示词：保存点恒灰（不参与自动保存）；
/// `pinned` 决定工具栏钉住按钮 `[◉]` / `[○]`。
pub fn render_header(
    panel: Rect,
    buf: &mut Buffer,
    prompt: &Prompt,
    focused: bool,
    mode: PromptMode,
    title: Option<&str>,
    draft: bool,
    pinned: bool,
) {
    let Some(header) = layout::prompt_header_rect(panel) else {
        return;
    };
    render_divider(panel, header, buf, focused);
    if mode == PromptMode::Editor
        && let Some(title) = title
    {
        paint_header_title(header, buf, title, draft);
    }
    render_toolbar(panel, buf, prompt, mode, draft, pinned);
}

/// 分割线行内嵌 title：起于左衔接符之后，超宽以 `…` 截断，不覆盖右衔接符；
/// 绑定临时提示词时在右端追加 accent 的 `[tmp]` 标识（与列表一致）。
fn paint_header_title(header: Rect, buf: &mut Buffer, title: &str, draft: bool) {
    let start = header.x.saturating_add(1);
    let end = header.right().saturating_sub(1);
    let available = usize::from(end.saturating_sub(start));
    if available < 4 {
        return;
    }
    let y = header.y.saturating_add(layout::PROMPT_TOOLBAR_HEIGHT);
    let badge = if draft {
        format!(" {}", text::PROMPT_TEMP_BADGE)
    } else {
        String::new()
    };
    let badge_width = badge.width();
    let badge_width = if badge_width + 4 <= available {
        let x = end.saturating_sub(badge_width as u16);
        buf.set_stringn(x, y, &badge, badge_width, style::accent());
        badge_width
    } else {
        0
    };
    let title_available = available.saturating_sub(badge_width);
    let label = text::ellipsize(&format!(" {title} "), title_available);
    buf.set_stringn(start, y, &label, title_available, style::muted());
}

/// 分割线整行 `─`（位于按钮行下一行，不穿过功能按钮），
/// 两端以 `├` / `┤` 衔接外层边框；样式跟随边框焦点态。
fn render_divider(panel: Rect, header: Rect, buf: &mut Buffer, focused: bool) {
    let y = header.y + layout::PROMPT_TOOLBAR_HEIGHT;
    for x in header.x..header.right() {
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

/// 工具栏：模式切换按钮常显；钉住按钮在右侧 `[*]`/`[tmp]` 左侧；树视图追加 `[tmp]`；
/// 编辑视图追加 undo/redo（不可用时置灰）、select all 与保存状态点（条目实心、草稿空心恒灰）。
fn render_toolbar(
    panel: Rect,
    buf: &mut Buffer,
    prompt: &Prompt,
    mode: PromptMode,
    draft: bool,
    pinned: bool,
) {
    let mode_label = match mode {
        PromptMode::Editor => text::PROMPT_MODE_LIST_LABEL,
        PromptMode::Tree => text::PROMPT_MODE_EDIT_LABEL,
    };
    let tree = mode == PromptMode::Tree;
    let mut buttons = vec![
        (PromptToolbarButton::Mode, mode_label, style::accent()),
        (
            PromptToolbarButton::Pin,
            if pinned {
                text::PROMPT_PINNED_LABEL
            } else {
                text::PROMPT_UNPINNED_LABEL
            },
            style::accent(),
        ),
    ];
    if tree {
        buttons.push((
            PromptToolbarButton::TempPrompt,
            text::PROMPT_TEMP_PROMPT_LABEL,
            style::accent(),
        ));
    } else {
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
                PromptToolbarButton::SelectAll,
                text::PROMPT_SELECT_ALL_LABEL,
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
    for (button, label, button_style) in buttons {
        let Some(rect) = layout::prompt_toolbar_button_rect(panel, button, tree) else {
            continue;
        };
        for (offset, symbol) in label.chars().enumerate() {
            let x = rect.x + offset as u16;
            if x >= rect.right() {
                break;
            }
            if let Some(cell) = buf.cell_mut((x, rect.y)) {
                cell.reset();
                cell.set_char(symbol);
                cell.set_style(button_style);
            }
        }
    }
}

/// 绘制 prompt 内容区；光标由调用方以终端原生硬件光标呈现。
///
/// 模板块行渲染为带边框的块：左边框（`╭─`/`│ `/`╰─`）、右边框（`╮`/`│`/`╯`），
/// 边框颜色随块状态；起始行顶边右端常显操作按钮，左侧内容超出时以省略号截断。
pub fn render(area: Rect, buf: &mut Buffer, prompt: &Prompt, selection: Option<&Selection>) {
    if area.width == 0 || area.height == 0 {
        return;
    }
    let rows = prompt.visual_rows();
    let tokens = markdown::scan(prompt.text());
    let infos = template_line_infos(prompt.text());
    let lines: Vec<&str> = prompt.text().split('\n').collect();
    let mut line_starts = Vec::new();
    let mut offset = 0;
    for line in &lines {
        line_starts.push(offset);
        offset += line.len() + 1;
    }
    let scroll = prompt.scroll().min(rows.len().saturating_sub(1));
    let mut block_rows = Vec::new();
    let mut separator_rows = Vec::new();
    for (index, row) in rows
        .iter()
        .enumerate()
        .skip(scroll)
        .take(usize::from(area.height))
    {
        let y = area.y + (index - scroll) as u16;
        let line_tokens = tokens.get(row.line).map(Vec::as_slice).unwrap_or_default();
        let base = row.start - line_starts.get(row.line).copied().unwrap_or(row.start);
        let info = infos.get(row.line).copied().flatten();
        let first = index == 0 || rows[index - 1].line != row.line;
        let last = index + 1 >= rows.len() || rows[index + 1].line != row.line;
        let role = template_row_role(info, first, last);
        let start_row = role == Some(TemplateLineRole::Start);
        let text_start = area.x + template_gutter(info.is_some());
        let buttons_start = if start_row {
            let labels =
                template_button_labels(info.map_or(TemplateStatus::Todo, |info| info.status));
            template_buttons_start(area, template_buttons_total(&labels))
        } else {
            area.right()
        };
        let text_end = template_text_end(area, info.is_some(), start_row, buttons_start);
        let geometry = RowGeometry {
            start_x: text_start,
            end_x: text_end,
            ellipsize: start_row,
        };
        let painted = paint_row(buf, y, prompt.text(), row, line_tokens, base, geometry);
        if let Some(selection) = selection {
            paint_selection(buf, area, y, index as u16, selection);
        }
        if let Some(info) = info {
            paint_template_border(buf, area, y, info, role, painted, buttons_start);
            block_rows.push((y, info.status));
            // 模板块分割线行：整行以边框线条绘制（内容为 `---` 的结构行）。
            if first
                && last
                && lines
                    .get(row.line)
                    .is_some_and(|line| crate::app::markdown::is_template_separator(line))
            {
                separator_rows.push((y, info.status));
            }
        }
    }
    render_template_buttons(buf, area, prompt);
    for (y, status) in block_rows {
        paint_template_background(buf, area, y, status);
    }
    for (y, status) in separator_rows {
        paint_template_separator(buf, area, y, status);
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

/// 提及条目父路径可用宽度：面板最宽占满文本区，扣除两侧边框、滚动条列与行内前导空格。
const MENTION_DETAIL_MARGIN: usize = 5;

/// 绘制浮层面板：文件提及优先，其次斜杠命令，最后块命令；状态由 app 层维护，这里只做只读映射。
fn render_panels(buf: &mut Buffer, area: Rect, prompt: &Prompt) {
    if let Some(data) = mention_panel_data(prompt, area.width) {
        let items = data.items();
        command_panel::render(area, buf, &mention_view(&items, &data, area.height / 2));
        return;
    }
    if let Some(data) = slash_panel_data(prompt) {
        let items = slash_items(&data);
        command_panel::render(area, buf, &slash_view(&items, &data));
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

/// 块命令面板条目：单行（名称 + 右侧预览，无来源标签）。
fn block_items<'a>(data: &'a BlockPanelData) -> Vec<CommandItem<'a>> {
    data.texts
        .iter()
        .map(|(label, preview)| CommandItem::Inline {
            label,
            preview,
            tag: "",
        })
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

/// 斜杠面板单条渲染数据：标签、预览与右对齐来源标签。
struct SlashRow {
    label: String,
    preview: String,
    tag: String,
}

/// 斜杠命令面板渲染数据：条目、高亮索引、窗口锚点/显式视口、锚点行与面板模式。
struct SlashPanelData {
    rows: Vec<SlashRow>,
    active: usize,
    anchor: usize,
    viewport: Option<usize>,
    anchor_row: u16,
    chooser: bool,
}

/// 斜杠命令面板渲染数据；面板未打开或光标滚出视口返回 None。
fn slash_panel_data(prompt: &Prompt) -> Option<SlashPanelData> {
    let panel = prompt.slash_panel()?;
    let (anchor_row, _) = prompt.cursor_cell()?;
    let chooser = matches!(
        panel.kind(),
        crate::app::markdown::SlashPanelKind::WorktreeChooser { .. }
    );
    Some(SlashPanelData {
        rows: panel.items().iter().map(slash_row).collect(),
        active: panel.active(),
        anchor: panel.anchor(),
        viewport: panel.viewport(),
        anchor_row,
        chooser,
    })
}

/// 单条候选的渲染行：选择器显示分支与路径，`/gitWorkTree` 用固定文案。
fn slash_row(item: &crate::app::markdown::SlashItem) -> SlashRow {
    use crate::app::markdown::{CommandSource, SlashInsertion};
    // tag 存纯文案，方括号由命令面板渲染层统一包裹。
    let tag = item.source.label().to_string();
    match item.source {
        CommandSource::Git => {
            let (label, preview) = match &item.insertion {
                SlashInsertion::WorktreeChooser => (
                    text::SLASH_WORKTREE_LABEL.to_string(),
                    text::SLASH_WORKTREE_PREVIEW.to_string(),
                ),
                SlashInsertion::Placeholder => (String::new(), item.preview.clone()),
                _ => (item.name.clone(), item.preview.clone()),
            };
            SlashRow {
                label,
                preview,
                tag,
            }
        }
        _ => SlashRow {
            label: item.label(),
            preview: item.preview.clone(),
            tag,
        },
    }
}

/// 斜杠命令面板条目：单行（标签 + 预览 + 右对齐来源标签）。
fn slash_items<'a>(data: &'a SlashPanelData) -> Vec<CommandItem<'a>> {
    data.rows
        .iter()
        .map(|row| CommandItem::Inline {
            label: row.label.as_str(),
            preview: row.preview.as_str(),
            tag: row.tag.as_str(),
        })
        .collect()
}

/// 斜杠命令面板视图：窗口锚定面板状态，显式视口优先；选择器换标题。
fn slash_view<'a>(items: &'a [CommandItem<'a>], data: &'a SlashPanelData) -> CommandPanelView<'a> {
    CommandPanelView {
        items,
        active: data.active,
        window_anchor: Some(data.anchor),
        window_start: data.viewport,
        anchor_row: data.anchor_row,
        max_height: None,
        title: Some(if data.chooser {
            text::WORKTREE_PANEL_TITLE
        } else {
            text::SLASH_PANEL_TITLE
        }),
        right_title: None,
        footer: None,
    }
}

/// 斜杠命令面板布局；面板未打开或空间不足返回 None。
pub fn slash_layout(prompt: &Prompt, area: Rect) -> Option<PanelLayout> {
    let data = slash_panel_data(prompt)?;
    let items = slash_items(&data);
    command_panel::layout(area, &slash_view(&items, &data))
}

/// 斜杠命令面板命中：返回被点中的条目索引；面板未打开或未命中返回 None。
pub fn slash_item_at(prompt: &Prompt, area: Rect, column: u16, row: u16) -> Option<usize> {
    let data = slash_panel_data(prompt)?;
    let items = slash_items(&data);
    let layout = command_panel::layout(area, &slash_view(&items, &data))?;
    command_panel::item_at(&layout, &items, column, row)
}

/// 斜杠命令面板矩形；用于滚轮命中。
pub fn slash_panel_rect(prompt: &Prompt, area: Rect) -> Option<Rect> {
    slash_layout(prompt, area).map(|layout| layout.rect)
}

/// 提及面板布局；面板未打开或空间不足返回 None。
pub fn mention_layout(prompt: &Prompt, area: Rect) -> Option<PanelLayout> {
    let data = mention_panel_data(prompt, area.width)?;
    let items = data.items();
    command_panel::layout(area, &mention_view(&items, &data, area.height / 2))
}

/// 提及面板命中：返回被点中的条目索引；面板未打开或未命中返回 None。
pub fn mention_item_at(prompt: &Prompt, area: Rect, column: u16, row: u16) -> Option<usize> {
    let data = mention_panel_data(prompt, area.width)?;
    let items = data.items();
    let layout = command_panel::layout(area, &mention_view(&items, &data, area.height / 2))?;
    command_panel::item_at(&layout, &items, column, row)
}

/// 提及面板矩形；用于滚轮命中。
pub fn mention_panel_rect(prompt: &Prompt, area: Rect) -> Option<Rect> {
    mention_layout(prompt, area).map(|layout| layout.rect)
}

/// 提及面板条目：文件候选（图标 + 文件名 + 父目录明细）或取根选择（单行）。
enum MentionRowText {
    File {
        icon: &'static str,
        label: String,
        detail: String,
    },
    Choice {
        label: &'static str,
    },
}

impl MentionRowText {
    /// 组装条目视图：文件双行展示，取根选择单行。
    fn item(&self) -> CommandItem<'_> {
        match self {
            Self::File {
                icon,
                label,
                detail,
            } => CommandItem::Stacked {
                icon: Some(icon),
                label,
                detail,
            },
            Self::Choice { label } => CommandItem::Inline {
                label,
                preview: "",
                tag: "",
            },
        }
    }
}

/// 提及面板渲染数据：条目、高亮索引、窗口锚点/显式视口、锚点行、当前目录名与标题。
struct MentionPanelData {
    rows: Vec<MentionRowText>,
    active: usize,
    anchor: usize,
    viewport: Option<usize>,
    anchor_row: u16,
    /// 当前进入的目录末段名；未进入文件夹时 None。
    scope: Option<String>,
    /// 面板标题：`Files`，选定取根后追加 `-[main]` / `-[wt]`。
    title: String,
    /// 取根选择器：两项单行、不显示目录导航页脚。
    chooser: bool,
}

impl MentionPanelData {
    /// 组装条目视图；图标列固定占位，标签与明细借用自身文本。
    fn items(&self) -> Vec<CommandItem<'_>> {
        self.rows.iter().map(MentionRowText::item).collect()
    }
}

/// 提及面板渲染数据；面板未打开或光标滚出视口返回 None。
fn mention_panel_data(prompt: &Prompt, width: u16) -> Option<MentionPanelData> {
    let (anchor_row, _) = prompt.cursor_cell()?;
    if let Some(active) = prompt.mention_chooser() {
        return Some(MentionPanelData {
            rows: crate::app::prompt::MentionRootChoice::ALL
                .iter()
                .map(|choice| MentionRowText::Choice {
                    label: choice.label(),
                })
                .collect(),
            active,
            anchor: active,
            viewport: None,
            anchor_row,
            scope: None,
            title: text::MENTION_PANEL_TITLE.to_string(),
            chooser: true,
        });
    }
    let panel = prompt.mention()?;
    let detail_width = usize::from(width).saturating_sub(MENTION_DETAIL_MARGIN);
    Some(MentionPanelData {
        rows: panel
            .items()
            .iter()
            .map(|entry| mention_row_text(entry, detail_width))
            .collect(),
        active: panel.active(),
        anchor: panel.anchor(),
        viewport: panel.viewport(),
        anchor_row,
        scope: panel.scope_name().map(str::to_string),
        title: mention_title(prompt),
        chooser: false,
    })
}

/// 提及面板标题：绑定 linked worktree 且已选定取根时追加 `-[main]` / `-[wt]`。
fn mention_title(prompt: &Prompt) -> String {
    match prompt.mention_root_label() {
        Some(label) => format!("{}-[{label}]", text::MENTION_PANEL_TITLE),
        None => text::MENTION_PANEL_TITLE.to_string(),
    }
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
        title: Some(data.title.as_str()),
        right_title: data.scope.as_deref(),
        footer: (!data.chooser).then_some(text::MENTION_PANEL_FOOTER),
    }
}

/// 提及条目展示：第一行目录/文件图标 + 文件名（目录带 `/`），第二行 `└─ ` 前缀的父目录。
///
/// 父目录超出可用宽度时按显示列中间截断（保留路径首尾），对齐 opencode 的路径省略逻辑。
fn mention_row_text(entry: &MentionEntry, detail_width: usize) -> MentionRowText {
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
        let prefix = format!("{} ", text::WORKSPACE_TREE_LAST);
        let path_width = detail_width.saturating_sub(prefix.width());
        format!("{prefix}{}", text::ellipsize_middle(directory, path_width))
    };
    let icon = if entry.is_directory {
        text::MENTION_DIR_ICON
    } else {
        text::MENTION_FILE_ICON
    };
    MentionRowText::File {
        icon,
        label,
        detail,
    }
}

/// 模板块操作按钮种类。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TemplateBlockButton {
    Status,
    Copy,
    Clean,
    Delete,
}

/// 模板块操作按钮命中：起始逻辑行索引、按钮、所属块状态与绘制矩形。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TemplateButtonHit {
    pub line: usize,
    pub button: TemplateBlockButton,
    pub status: TemplateStatus,
    pub rect: Rect,
}

/// 按钮组顺序：状态、复制、清理、删除。
const TEMPLATE_BUTTONS: [TemplateBlockButton; 4] = [
    TemplateBlockButton::Status,
    TemplateBlockButton::Copy,
    TemplateBlockButton::Clean,
    TemplateBlockButton::Delete,
];

/// 模板块行左边框槽宽度：块行为 `PROMPT_TEMPLATE_GUTTER_LEFT`，其余为 0。
fn template_gutter(block: bool) -> u16 {
    if block {
        layout::PROMPT_TEMPLATE_GUTTER_LEFT
    } else {
        0
    }
}

/// 视觉行的模板块角色：软换行续行降级为中间行（`╭`/`╰` 只在首末行）。
fn template_row_role(
    info: Option<TemplateLineInfo>,
    first: bool,
    last: bool,
) -> Option<TemplateLineRole> {
    let info = info?;
    Some(match info.role {
        TemplateLineRole::Start if !first => TemplateLineRole::Middle,
        TemplateLineRole::End if !last => TemplateLineRole::Middle,
        role => role,
    })
}

/// 按钮组文案（状态标签 + 复制/清理/删除）。
fn template_button_labels(status: TemplateStatus) -> [&'static str; 4] {
    TEMPLATE_BUTTONS.map(|button| template_button_label(button, status))
}

/// 按钮组总宽（含 1 列间距）。
fn template_buttons_total(labels: &[&str; 4]) -> u16 {
    let width: usize = labels.iter().map(|label| label.width()).sum();
    (width + labels.len().saturating_sub(1)) as u16
}

/// 按钮组起点：右对齐贴右边框内侧；极窄时钳到左边框之后（按钮常显、优先于文本）。
fn template_buttons_start(area: Rect, total: u16) -> u16 {
    let corner = area.right().saturating_sub(1);
    corner
        .saturating_sub(total)
        .max(area.x + layout::PROMPT_TEMPLATE_GUTTER_LEFT)
}

/// 视觉行文本区右边界（不含）：普通行到内容区右缘，块行到右边框前；
/// 起始行再为按钮组让出一列间距（空间不足时退到左边框之后，文本仅剩省略号）。
fn template_text_end(area: Rect, block: bool, start_row: bool, buttons_start: u16) -> u16 {
    let end = if block {
        area.right()
            .saturating_sub(layout::PROMPT_TEMPLATE_GUTTER_RIGHT)
    } else {
        area.right()
    };
    if start_row {
        end.min(buttons_start.saturating_sub(1))
            .max(area.x + layout::PROMPT_TEMPLATE_GUTTER_LEFT)
    } else {
        end
    }
}

/// 计算可见模板块起始行的操作按钮布局：按钮组常显、贴内容区右缘右对齐；
/// 文本与按钮组重叠时由渲染层省略号截断，不遮挡按钮。
pub fn template_buttons(prompt: &Prompt, area: Rect) -> Vec<TemplateButtonHit> {
    if area.width == 0 || area.height == 0 {
        return Vec::new();
    }
    let rows = prompt.visual_rows();
    let infos = template_line_infos(prompt.text());
    let scroll = prompt.scroll();
    let mut hits = Vec::new();
    for (index, row) in rows.iter().enumerate() {
        let first = index == 0 || rows[index - 1].line != row.line;
        if !first {
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
        if info.role != TemplateLineRole::Start {
            continue;
        }
        let labels = template_button_labels(info.status);
        let start = template_buttons_start(area, template_buttons_total(&labels));
        let y = area.y + visible as u16;
        let mut x = start;
        for (button, label) in TEMPLATE_BUTTONS.iter().zip(labels) {
            let width = label.width() as u16;
            hits.push(TemplateButtonHit {
                line: row.line,
                button: *button,
                status: info.status,
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

/// 按钮文案：状态按钮按模板块当前状态取标签。
fn template_button_label(button: TemplateBlockButton, status: TemplateStatus) -> &'static str {
    match button {
        TemplateBlockButton::Status => text::template_status_label(status),
        TemplateBlockButton::Copy => text::TEMPLATE_BUTTON_COPY,
        TemplateBlockButton::Clean => text::TEMPLATE_BUTTON_CLEAN,
        TemplateBlockButton::Delete => text::TEMPLATE_BUTTON_DEL,
    }
}

/// 绘制模板块边框：左槽 `╭─`/`│ `/`╰─`、右槽 `╮`/`│`/`╯`；
/// 起始行顶边与结束行底边以 `─` 补满文本之后的空间（按钮组由后续绘制覆盖）。
fn paint_template_border(
    buf: &mut Buffer,
    area: Rect,
    y: u16,
    info: TemplateLineInfo,
    role: Option<TemplateLineRole>,
    painted: u16,
    buttons_start: u16,
) {
    let border = style::template_border(info.status);
    let left = area.x;
    let right = area.right().saturating_sub(1);
    let mut put = |x: u16, symbol: char| {
        if let Some(cell) = buf.cell_mut((x, y)) {
            cell.reset();
            cell.set_char(symbol);
            cell.set_style(border);
        }
    };
    match role {
        Some(TemplateLineRole::Start) => {
            put(left, '╭');
            put(left + 1, '─');
            put(right, '╮');
            for x in painted..buttons_start.min(right) {
                put(x, '─');
            }
        }
        Some(TemplateLineRole::Middle) => {
            put(left, '│');
            put(right, '│');
        }
        Some(TemplateLineRole::End) => {
            put(left, '╰');
            put(left + 1, '─');
            put(right, '╯');
            for x in painted..right {
                put(x, '─');
            }
        }
        None => {}
    }
}

/// 绘制模板块分割线：`├─…─┤` 以块边框样式横贯整行，与块上下边同色；
/// 保留单元格底色与选区反显修饰符。
fn paint_template_separator(buf: &mut Buffer, area: Rect, y: u16, status: TemplateStatus) {
    let border = style::template_border(status);
    let right = area.right().saturating_sub(1);
    for x in area.x..=right {
        let symbol = if x == area.x {
            '├'
        } else if x == right {
            '┤'
        } else {
            '─'
        };
        if let Some(cell) = buf.cell_mut((x, y)) {
            cell.set_char(symbol);
            cell.set_style(border);
        }
    }
}

/// 模板块整行底色：按状态铺暗色底；选区反显格跳过，保持反显在终端默认底色上呈现。
fn paint_template_background(buf: &mut Buffer, area: Rect, y: u16, status: TemplateStatus) {
    let bg = style::template_block_bg(status);
    for x in area.x..area.right() {
        if let Some(cell) = buf.cell_mut((x, y)) {
            if cell.modifier.contains(Modifier::REVERSED) {
                continue;
            }
            cell.set_style(bg);
        }
    }
}

/// 绘制模板块操作按钮；状态按钮按状态着色，其余次要信息。
fn render_template_buttons(buf: &mut Buffer, area: Rect, prompt: &Prompt) {
    for hit in template_buttons(prompt, area) {
        let label = template_button_label(hit.button, hit.status);
        let button_style = match hit.button {
            TemplateBlockButton::Status => style::template_status(hit.status),
            _ => style::template_button(),
        };
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
        TokenKind::TemplateMarker => style::template_marker(),
        TokenKind::TemplateCommand(id) => style::template_command(id),
        TokenKind::TemplateTitle => style::template_title(),
        TokenKind::FileMention => style::markdown_file_mention(),
    }
}

#[cfg(test)]
mod tests;
