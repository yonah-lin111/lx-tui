//! lx 页 `/` 命令面板：条目、视图、几何与命中（`/` 命令 / `/model` 两级 / `/resume`）。
//!
//! 与 `@` 提及面板同款几何：与输入框等宽、底边紧贴输入框上沿、条目区滚动、
//! 顶边标题 + 底边快捷键提示、右端来源标签；`/model`、`/resume` 无候选时显示
//! 空态占位。全部文案为 ASCII，避免双宽字符撑破面板。

use ratatui::layout::Rect;

use crate::app::state::Lx;
use crate::ui::text;
use crate::ui::widgets::command_panel::{
    self, CommandItem, CommandPanelView, PanelLayout, PanelPlacement,
};

use super::mention;

/// `/` 命令面板条目（与 `render` 同源）；面板在但无候选时给一行空态占位。
///
/// 返回的 `CommandItem` 只借用 `panel` 与 `time_labels`（含 `&'static str` 占位），
/// 无自有字符串；相对时间由调用方预先算好并以切片传入，避免在条目里持有临时值。
pub(super) fn items<'a>(
    panel: &'a crate::app::state::SlashPanel,
    time_labels: &'a [String],
) -> Vec<CommandItem<'a>> {
    use crate::app::state::{ModelPickerStage, SlashPanel};
    match panel {
        SlashPanel::Commands { items, .. } => items
            .iter()
            .map(|command| CommandItem::Inline {
                label: command.text(),
                preview: command.description(),
                tag: command.tag(),
            })
            .collect(),
        SlashPanel::Model(picker) => match &picker.stage {
            ModelPickerStage::Models => {
                if picker.candidates.is_empty() {
                    return vec![CommandItem::Inline {
                        label: text::LX_CHAT_NO_MODEL,
                        preview: "",
                        tag: "",
                    }];
                }
                picker
                    .candidates
                    .iter()
                    .map(|candidate| CommandItem::Inline {
                        label: candidate.model_name.as_str(),
                        preview: candidate.provider_name.as_str(),
                        tag: "",
                    })
                    .collect()
            }
            ModelPickerStage::Variants { variants, .. } => variants
                .iter()
                .map(|variant| CommandItem::Inline {
                    label: variant.as_str(),
                    preview: "",
                    tag: "",
                })
                .collect(),
        },
        SlashPanel::History(history) => {
            if history.entries.is_empty() {
                return vec![CommandItem::Inline {
                    label: text::LX_HISTORY_EMPTY,
                    preview: "",
                    tag: "",
                }];
            }
            // 行右端是相对时间（对齐 opencode）；会话 id 不再上桌（只用于内部绑定）。
            history
                .entries
                .iter()
                .enumerate()
                .map(|(index, entry)| CommandItem::Inline {
                    label: entry.title.as_str(),
                    preview: "",
                    tag: time_labels.get(index).map_or("", String::as_str),
                })
                .collect()
        }
    }
}

/// 历史条目的相对时间标签（与 `items` 的条目索引一一对应）；非历史面板为空。
pub(super) fn history_time_labels(panel: &crate::app::state::SlashPanel) -> Vec<String> {
    match panel {
        crate::app::state::SlashPanel::History(history) => history
            .entries
            .iter()
            .map(|entry| entry.time_label(history.now_ms))
            .collect(),
        _ => Vec::new(),
    }
}

/// `/` 命令面板视图：与 `@` 面板同款宽度/放置/高度口径，窗口取自面板状态。
fn view<'a>(
    items: &'a [CommandItem<'a>],
    panel: &'a crate::app::state::SlashPanel,
    input: Rect,
    area: Rect,
) -> CommandPanelView<'a> {
    use crate::app::state::{ModelPickerStage, SlashPanel};
    let (title, window, footer) = match panel {
        SlashPanel::Commands { window, .. } => (
            text::LX_SLASH_PANEL_TITLE,
            *window,
            text::LX_SLASH_PANEL_FOOTER,
        ),
        SlashPanel::Model(picker) => (
            if matches!(picker.stage, ModelPickerStage::Models) {
                text::LX_MODEL_PANEL_TITLE
            } else {
                text::LX_MODEL_VARIANT_PANEL_TITLE
            },
            picker.window,
            text::LX_SLASH_PANEL_FOOTER,
        ),
        // `/resume` 额外提示 `Shift+Enter` 打开会话编辑模态（`⇧↵` 与 `@` 面板同符号）。
        SlashPanel::History(history) => (
            text::LX_HISTORY_PANEL_TITLE,
            history.window,
            text::LX_HISTORY_PANEL_FOOTER,
        ),
    };
    let above = input.y.saturating_sub(area.y);
    let cap = above.min(mention::MAX_POPUP_ROWS.saturating_add(mention::PANEL_BORDER_ROWS));
    CommandPanelView {
        items,
        active: window.active,
        // 空态占位行时 current 越界，渲染侧按索引匹配不到，自然不画标记。
        current: panel.current_index(),
        window_anchor: Some(window.anchor),
        window_start: window.viewport,
        anchor_row: above,
        max_height: Some(cap),
        title: Some(title),
        right_title: None,
        footer: Some(footer),
        width: Some(input.width),
        placement: PanelPlacement::Above,
    }
}

/// `/` 命令面板整块矩形（贴输入框上沿、与输入框等宽）。
fn area_of(input: Rect, area: Rect) -> Rect {
    Rect::new(input.x, area.y, input.width, input.y.saturating_sub(area.y))
}

/// `/` 命令面板纯几何（与 `render_slash_panel` 同源）；供滚轮与命中复用。
pub(crate) fn layout(area: Rect, lx: &Lx) -> Option<PanelLayout> {
    let panel = lx.slash.as_ref()?;
    let input = super::input_rect(area, lx)?;
    let time_labels = history_time_labels(panel);
    let items = items(panel, &time_labels);
    command_panel::layout(area_of(input, area), &view(&items, panel, input, area))
}

/// `/` 命令面板命中：返回被点/悬停的条目索引；未命中返回 None。
pub(crate) fn item_at(area: Rect, lx: &Lx, column: u16, row: u16) -> Option<usize> {
    let panel = lx.slash.as_ref()?;
    // 空态占位行不可选。
    if matches!(panel, crate::app::state::SlashPanel::Model(p) if p.candidates.is_empty())
        || matches!(panel, crate::app::state::SlashPanel::History(h) if h.entries.is_empty())
    {
        return None;
    }
    let input = super::input_rect(area, lx)?;
    let time_labels = history_time_labels(panel);
    let items = items(panel, &time_labels);
    let layout = command_panel::layout(area_of(input, area), &view(&items, panel, input, area))?;
    command_panel::item_at(&layout, &items, column, row)
}

/// 绘制 `/` 命令面板（复用 `layout` 的矩形口径）。
pub(super) fn render(area: Rect, buf: &mut ratatui::buffer::Buffer, lx: &Lx) {
    let Some(panel) = lx.slash.as_ref() else {
        return;
    };
    let Some(input) = super::input_rect(area, lx) else {
        return;
    };
    let time_labels = history_time_labels(panel);
    let items = items(panel, &time_labels);
    command_panel::render(area_of(input, area), buf, &view(&items, panel, input, area));
}
