//! lx 输入框 `@` 提及面板：贴输入框上沿、与输入框等宽的候选浮层。
//!
//! 渲染、点击、悬停与滚轮共用同一份纯几何（`layout`）；只读状态、只写 Buffer。

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;

use crate::app::state::Lx;
use crate::app::state::lx::LxMentionView;
use crate::ui::mention::{self, MENTION_DETAIL_MARGIN};
use crate::ui::text;
use crate::ui::widgets::command_panel::{
    self, CommandItem, CommandPanelView, PanelLayout, PanelPlacement,
};

/// 面板最多显示的内容行数；对齐 codex `MAX_POPUP_ROWS`（8），保证浮层观感统一。
pub(crate) const MAX_POPUP_ROWS: u16 = 8;
/// 面板边框行数（上下各一行）。
const PANEL_BORDER_ROWS: u16 = 2;

/// 提及面板条目文本：候选文件/目录（双行）或状态行（单行）。
enum PanelRow {
    File(mention::MentionRow),
    Status(&'static str),
}

impl PanelRow {
    /// 组装命令面板条目。
    fn item(&self) -> CommandItem<'_> {
        match self {
            Self::File(row) => row.item(),
            Self::Status(label) => CommandItem::Inline {
                label,
                preview: "",
                tag: "",
            },
        }
    }
}

/// 提及面板渲染数据：条目、高亮、上方可用行预算与标题右侧目录名。
struct PanelData {
    rows: Vec<PanelRow>,
    active: usize,
    anchor: Option<usize>,
    viewport: Option<usize>,
    /// 当前进入的目录末段名；未进入文件夹时 None（绘制在顶边右侧）。
    scope: Option<String>,
    /// 是否为交互候选（可点击采纳）；状态行时为 false。
    interactive: bool,
}

impl PanelData {
    /// 组装条目视图。
    fn items(&self) -> Vec<CommandItem<'_>> {
        self.rows.iter().map(PanelRow::item).collect()
    }
}

/// 提及面板纯几何：面板矩形、内容区、首个可见条目与可见条目数、滚动条命中区。
///
/// 以输入框为唯一定位依据：`x` 与 `width` 等于输入框，底边紧贴输入框顶边，向上占用
/// 可用行；空间不足（放不下边框 + 一行）或面板不存在时返回 None。
pub(crate) fn layout(area: Rect, lx: &Lx) -> Option<PanelLayout> {
    let input = super::input_rect(area, lx)?;
    let data = panel_data(lx, input.width)?;
    let items = data.items();
    command_panel::layout(panel_area(area, input), &view(&items, &data, input, area))
}

/// 提及条目命中：返回可见条目索引；面板未打开、状态行或未命中返回 None。
pub(crate) fn item_at(area: Rect, lx: &Lx, column: u16, row: u16) -> Option<usize> {
    let input = super::input_rect(area, lx)?;
    let data = panel_data(lx, input.width)?;
    if !data.interactive {
        return None;
    }
    let items = data.items();
    let layout = command_panel::layout(panel_area(area, input), &view(&items, &data, input, area))?;
    command_panel::item_at(&layout, &items, column, row)
}

/// 绘制提及面板；面板未打开或空间不足时不绘制。
pub(crate) fn render(area: Rect, buf: &mut Buffer, lx: &Lx) {
    let Some(input) = super::input_rect(area, lx) else {
        return;
    };
    let Some(data) = panel_data(lx, input.width) else {
        return;
    };
    let items = data.items();
    command_panel::render(
        panel_area(area, input),
        buf,
        &view(&items, &data, input, area),
    );
}

/// 面板可用区域：左边缘对齐输入框，纵向仍以窗格内容区为界。
fn panel_area(area: Rect, input: Rect) -> Rect {
    Rect {
        x: input.x,
        width: input.width,
        ..area
    }
}

/// 提及面板渲染数据；面板不显示时返回 None。
fn panel_data(lx: &Lx, width: u16) -> Option<PanelData> {
    let status = |label: &'static str| PanelData {
        rows: vec![PanelRow::Status(label)],
        active: 0,
        anchor: None,
        viewport: None,
        scope: None,
        interactive: false,
    };
    match lx.mention().view(&lx.input, lx.cursor) {
        LxMentionView::Hidden => None,
        LxMentionView::Loading => Some(status(text::LX_MENTION_LOADING)),
        LxMentionView::Empty => Some(status(text::LX_MENTION_EMPTY)),
        LxMentionView::Items => {
            let panel = lx.mention().panel()?;
            let detail_width = usize::from(width).saturating_sub(MENTION_DETAIL_MARGIN);
            Some(PanelData {
                rows: panel
                    .items()
                    .iter()
                    .map(|entry| PanelRow::File(mention::file_row(entry, detail_width)))
                    .collect(),
                active: panel.active(),
                anchor: Some(panel.anchor()),
                viewport: panel.viewport(),
                scope: panel.scope_name().map(str::to_string),
                interactive: true,
            })
        }
    }
}

/// 面板视图：强制与输入框等宽、紧贴输入框上方、视口优先于窗口锚点。
///
/// 高度对齐 codex 的两步口径：内容行先夹到 `MAX_POPUP_ROWS`（再加边框），
/// 再与输入框上方可用行数取小；空间不足时不挤压输入框。
fn view<'a>(
    items: &'a [CommandItem<'a>],
    data: &'a PanelData,
    input: Rect,
    area: Rect,
) -> CommandPanelView<'a> {
    let above = input.y.saturating_sub(area.y);
    let cap = above.min(MAX_POPUP_ROWS.saturating_add(PANEL_BORDER_ROWS));
    CommandPanelView {
        items,
        active: data.active,
        window_anchor: data.anchor,
        window_start: data.viewport,
        anchor_row: above,
        max_height: Some(cap),
        title: Some(text::LX_MENTION_PANEL_TITLE),
        right_title: data.scope.as_deref(),
        footer: data.interactive.then_some(text::MENTION_PANEL_FOOTER),
        width: Some(input.width),
        placement: PanelPlacement::Above,
    }
}
