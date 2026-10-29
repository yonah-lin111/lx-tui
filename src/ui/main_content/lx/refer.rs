//! lx 输入框字母快捷输入面板：贴输入框上沿、与输入框等宽的候选浮层。
//!
//! 只读状态、只写 Buffer。

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;

use crate::app::state::Lx;
use crate::ui::text;
use crate::ui::widgets::command_panel::PanelLayout;
use crate::ui::widgets::command_panel::{self, CommandItem, CommandPanelView, PanelPlacement};

use super::mention::{MAX_POPUP_ROWS, PANEL_BORDER_ROWS};

/// 快捷面板单条渲染数据。
struct ReferRow {
    label: String,
    preview: String,
    tag: String,
}

impl ReferRow {
    fn item(&self) -> CommandItem<'_> {
        CommandItem::Inline {
            label: &self.label,
            preview: &self.preview,
            tag: &self.tag,
        }
    }
}

/// 快捷面板渲染数据。
struct ReferData {
    rows: Vec<ReferRow>,
    active: usize,
    viewport: Option<usize>,
}

impl ReferData {
    fn items(&self) -> Vec<CommandItem<'_>> {
        self.rows.iter().map(ReferRow::item).collect()
    }
}

/// 快捷面板纯几何。
pub(crate) fn layout(area: Rect, lx: &Lx) -> Option<PanelLayout> {
    let input = super::input_rect(area, lx)?;
    let data = panel_data(lx)?;
    let items = data.items();
    command_panel::layout(panel_area(area, input), &view(&items, &data, input, area))
}

/// 快捷面板条目命中：返回候选索引；面板未打开或未命中返回 None。
pub(crate) fn item_at(area: Rect, lx: &Lx, column: u16, row: u16) -> Option<usize> {
    let input = super::input_rect(area, lx)?;
    let data = panel_data(lx)?;
    let items = data.items();
    let layout = command_panel::layout(panel_area(area, input), &view(&items, &data, input, area))?;
    command_panel::item_at(&layout, &items, column, row)
}

/// 绘制快捷输入面板；面板未打开或空间不足时不绘制。
pub(crate) fn render(area: Rect, buf: &mut Buffer, lx: &Lx) {
    let Some(input) = super::input_rect(area, lx) else {
        return;
    };
    let Some(data) = panel_data(lx) else {
        return;
    };
    let items = data.items();
    command_panel::render(
        panel_area(area, input),
        buf,
        &view(&items, &data, input, area),
    );
}

/// 面板可用区域：左边缘对齐输入框，纵向以窗格内容区为界。
fn panel_area(area: Rect, input: Rect) -> Rect {
    Rect {
        x: input.x,
        width: input.width,
        ..area
    }
}

/// 提取快捷面板数据。
fn panel_data(lx: &Lx) -> Option<ReferData> {
    let panel = lx.refer_panel()?;
    Some(ReferData {
        rows: panel
            .items()
            .iter()
            .map(|candidate| ReferRow {
                label: candidate.name.clone(),
                preview: candidate.text.clone(),
                tag: candidate.tag.label().to_string(),
            })
            .collect(),
        active: panel.active(),
        viewport: panel.viewport(),
    })
}

/// 面板视图：强制与输入框等宽、紧贴输入框上方。
fn view<'a>(
    items: &'a [CommandItem<'a>],
    data: &'a ReferData,
    input: Rect,
    area: Rect,
) -> CommandPanelView<'a> {
    let above = input.y.saturating_sub(area.y);
    let cap = above.min(MAX_POPUP_ROWS.saturating_add(PANEL_BORDER_ROWS));
    CommandPanelView {
        items,
        active: data.active,
        current: None,
        window_anchor: Some(data.active),
        window_start: data.viewport,
        anchor_row: above,
        max_height: Some(cap),
        title: Some(text::REFER_PANEL_TITLE),
        right_title: None,
        footer: None,
        width: Some(input.width),
        placement: PanelPlacement::Above,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lx_with_refer() -> Lx {
        let mut lx = Lx::new();
        lx.resize(80, 24);
        lx.chat.messages.push(crate::app::state::ChatMessageView {
            sequence: 0,
            id: "1".into(),
            role: "user".into(),
            status: "complete".into(),
            turn_index: 0,
            kind: "chat".into(),
            shell: None,
            steps: vec![crate::app::state::AssistantStepView::Part {
                position: 0,
                part: crate::app::state::ChatPartView {
                    kind: crate::chat::events::PartKind::Text,
                    content: "@src/main.rs".into(),
                    provider_metadata_json: None,
                    started_at_ms: None,
                    ended_at_ms: None,
                    completed_at_ms: None,
                },
            }],
            model_label: None,
            error_code: None,
            input_tokens: None,
            output_tokens: None,
            cache_read_tokens: None,
            duration_ms: None,
            reports: Vec::new(),
            attachments: Vec::new(),
        });
        lx.insert_char('m');
        lx.insert_char('a');
        lx
    }

    #[test]
    fn refer_panel_layout_and_item_at() {
        let area = Rect::new(0, 0, 80, 24);
        let lx = lx_with_refer();
        assert!(lx.refer_panel().is_some());

        let layout = layout(area, &lx).expect("layout exists");
        assert!(layout.rect.width > 0);
        assert!(layout.rect.height > 0);

        let hit = item_at(area, &lx, layout.rect.x + 2, layout.rect.y + 1);
        assert_eq!(hit, Some(0));

        let mut buf = Buffer::empty(area);
        render(area, &mut buf, &lx);
    }
}
