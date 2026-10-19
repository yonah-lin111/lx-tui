//! prompt 浮层面板渲染：粘贴选择、字母快捷、文件提及、斜杠与块命令的只读映射。
//!
//! 状态由 app 层维护；这里只把状态映射为命令面板条目并计算布局/命中，不修改任何状态。

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;

use crate::app::markdown::MentionEntry;
use crate::app::prompt::{PasteAction, Prompt};
use crate::ui::text;
use crate::ui::widgets::command_panel::{self, CommandItem, CommandPanelView, PanelLayout};

/// 粘贴面板渲染数据：三行操作、高亮与锚点行。
struct PastePanelData {
    rows: Vec<(String, String)>,
    active: usize,
    anchor_row: u16,
}

/// 粘贴面板渲染数据；面板未打开或光标滚出视口返回 None。
fn paste_panel_data(prompt: &Prompt) -> Option<PastePanelData> {
    let panel = prompt.paste_panel()?;
    let (anchor_row, _) = prompt.cursor_cell()?;
    let count = text::paste_item_count(panel.candidates.len());
    let rows = PasteAction::ALL
        .iter()
        .map(|action| match action {
            PasteAction::Reference => (
                text::PASTE_ACTION_REFERENCE.to_string(),
                if panel.reference_enabled() {
                    count.clone()
                } else {
                    text::PASTE_REFERENCE_UNSUPPORTED.to_string()
                },
            ),
            PasteAction::Path => (text::PASTE_ACTION_PATH.to_string(), count.clone()),
        })
        .collect();
    Some(PastePanelData {
        rows,
        active: panel.active(),
        anchor_row,
    })
}

/// 粘贴面板条目：单行（操作名 + 说明，无来源标签）。
fn paste_items<'a>(data: &'a PastePanelData) -> Vec<CommandItem<'a>> {
    data.rows
        .iter()
        .map(|(label, preview)| CommandItem::Inline {
            label,
            preview,
            tag: "",
        })
        .collect()
}

/// 粘贴面板视图：锚定光标行，标题固定；不设高度上限。
fn paste_view<'a>(items: &'a [CommandItem<'a>], data: &PastePanelData) -> CommandPanelView<'a> {
    CommandPanelView {
        current: None,
        items,
        active: data.active,
        window_anchor: Some(data.active),
        window_start: None,
        anchor_row: data.anchor_row,
        max_height: None,
        title: Some(text::PASTE_PANEL_TITLE),
        right_title: None,
        footer: None,
        ..Default::default()
    }
}

/// 粘贴面板布局；面板未打开或空间不足返回 None。
pub fn paste_layout(prompt: &Prompt, area: Rect) -> Option<PanelLayout> {
    let data = paste_panel_data(prompt)?;
    let items = paste_items(&data);
    command_panel::layout(area, &paste_view(&items, &data))
}

/// 粘贴面板命中：返回被点中的操作索引；面板未打开或未命中返回 None。
pub fn paste_item_at(prompt: &Prompt, area: Rect, column: u16, row: u16) -> Option<usize> {
    let data = paste_panel_data(prompt)?;
    let items = paste_items(&data);
    let layout = command_panel::layout(area, &paste_view(&items, &data))?;
    command_panel::item_at(&layout, &items, column, row)
}

/// 粘贴面板矩形；用于滚轮命中。
pub fn paste_panel_rect(prompt: &Prompt, area: Rect) -> Option<Rect> {
    paste_layout(prompt, area).map(|layout| layout.rect)
}

/// 字母快捷面板渲染数据：候选行、高亮与锚点行。
struct ReferPanelData {
    rows: Vec<SlashRow>,
    active: usize,
    anchor_row: u16,
}

/// 字母快捷面板渲染数据；面板未打开或光标滚出视口返回 None。
fn refer_panel_data(prompt: &Prompt) -> Option<ReferPanelData> {
    let panel = prompt.refer_panel()?;
    let (anchor_row, _) = prompt.cursor_cell()?;
    Some(ReferPanelData {
        rows: panel
            .items()
            .iter()
            .map(|candidate| SlashRow {
                label: candidate.name.clone(),
                preview: candidate.text.clone(),
                tag: candidate.tag.label().to_string(),
            })
            .collect(),
        active: panel.active(),
        anchor_row,
    })
}

/// 字母快捷面板视图：锚定光标行，窗口跟随高亮；标题固定。
fn refer_view<'a>(items: &'a [CommandItem<'a>], data: &ReferPanelData) -> CommandPanelView<'a> {
    CommandPanelView {
        current: None,
        items,
        active: data.active,
        window_anchor: Some(data.active),
        window_start: None,
        anchor_row: data.anchor_row,
        max_height: None,
        title: Some(text::REFER_PANEL_TITLE),
        right_title: None,
        footer: None,
        ..Default::default()
    }
}

/// 字母快捷面板布局；面板未打开或空间不足返回 None。
pub fn refer_layout(prompt: &Prompt, area: Rect) -> Option<PanelLayout> {
    let data = refer_panel_data(prompt)?;
    let items = slash_row_items(&data.rows);
    command_panel::layout(area, &refer_view(&items, &data))
}

/// 字母快捷面板命中：返回被点中的候选索引；面板未打开或未命中返回 None。
pub fn refer_item_at(prompt: &Prompt, area: Rect, column: u16, row: u16) -> Option<usize> {
    let data = refer_panel_data(prompt)?;
    let items = slash_row_items(&data.rows);
    let layout = command_panel::layout(area, &refer_view(&items, &data))?;
    command_panel::item_at(&layout, &items, column, row)
}

/// 字母快捷面板矩形；用于滚轮命中。
pub fn refer_panel_rect(prompt: &Prompt, area: Rect) -> Option<Rect> {
    refer_layout(prompt, area).map(|layout| layout.rect)
}

/// 提及条目父路径可用宽度：面板最宽占满文本区，扣除两侧边框、滚动条列与行内前导空格。
const MENTION_DETAIL_MARGIN: usize = crate::ui::mention::MENTION_DETAIL_MARGIN;

/// 绘制浮层面板：粘贴选择优先，其次文件提及、字母快捷、斜杠命令，最后块命令；
/// 状态由 app 层维护，这里只做只读映射。
pub(super) fn render_panels(buf: &mut Buffer, area: Rect, prompt: &Prompt) {
    if let Some(data) = paste_panel_data(prompt) {
        let items = paste_items(&data);
        command_panel::render(area, buf, &paste_view(&items, &data));
        return;
    }
    if let Some(data) = mention_panel_data(prompt, area.width) {
        let items = data.items();
        command_panel::render(area, buf, &mention_view(&items, &data, area.height / 2));
        return;
    }
    if let Some(data) = refer_panel_data(prompt) {
        let items = slash_row_items(&data.rows);
        command_panel::render(area, buf, &refer_view(&items, &data));
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
        current: None,
        items,
        active: data.active,
        window_anchor: Some(data.anchor),
        window_start: data.viewport,
        anchor_row: data.anchor_row,
        max_height: None,
        title: Some(text::BLOCK_PANEL_TITLE),
        right_title: None,
        footer: None,
        ..Default::default()
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
    slash_row_items(&data.rows)
}

/// 候选行 → 面板条目（斜杠与字母快捷共用）。
fn slash_row_items(rows: &[SlashRow]) -> Vec<CommandItem<'_>> {
    rows.iter()
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
        current: None,
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
        ..Default::default()
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
    File(crate::ui::mention::MentionRow),
    Choice { label: &'static str },
}

impl MentionRowText {
    /// 组装条目视图：文件双行展示（目录按目录色），取根选择单行。
    fn item(&self) -> CommandItem<'_> {
        match self {
            Self::File(row) => row.item(),
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
        current: None,
        items,
        active: data.active,
        window_anchor: Some(data.anchor),
        window_start: data.viewport,
        anchor_row: data.anchor_row,
        max_height: Some(max_height),
        title: Some(data.title.as_str()),
        right_title: data.scope.as_deref(),
        footer: (!data.chooser).then_some(text::MENTION_PANEL_FOOTER),
        ..Default::default()
    }
}

/// 提及条目展示：第一行目录/文件图标 + 文件名（目录带 `/`），第二行 `└─ ` 前缀的父目录。
///
/// 父目录超出可用宽度时按显示列中间截断（保留路径首尾），对齐 opencode 的路径省略逻辑。
fn mention_row_text(entry: &MentionEntry, detail_width: usize) -> MentionRowText {
    MentionRowText::File(crate::ui::mention::file_row(entry, detail_width))
}
