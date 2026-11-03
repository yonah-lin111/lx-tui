//! lx 输入框关联几何、滚动条与弹出面板（路径粘贴面板、md 格式块面板与浮层命中聚合）。

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Style;
use unicode_width::UnicodeWidthStr;

use crate::app::markdown::BlockPanel;
use crate::app::paste::PasteAction;
use crate::app::state::Lx;
use crate::ui::widgets::command_panel::{self, CommandItem, CommandPanelView};
use crate::ui::widgets::scrollbar;
use crate::ui::{style, text};

use super::grid::Grid;
use super::{
    INPUT_PREFIX, INPUT_SCROLLBAR, MARGIN, layout, mention_layout, refer_layout, slash,
    slash_layout,
};

/// 绘制 `/` 命令面板（几何/条目见 `slash` 子模块）。
pub(super) fn render_slash_panel(area: Rect, buf: &mut Buffer, lx: &Lx) {
    if lx.slash.is_none() {
        return;
    }
    slash::render(area, buf, lx);
}

/// 绘制输入框滚动条：软换行后的视觉行溢出可视高度时，覆盖在最右预留列。
pub(super) fn render_input_scrollbar(area: Rect, buf: &mut Buffer, lx: &Lx) {
    if lx.input.is_empty() {
        return;
    }
    if let Some(bar) = input_scrollbar(area, lx) {
        scrollbar::render_buffer(buf, &bar);
    }
}

/// lx 输入框文本区几何（绝对屏幕坐标）；供鼠标命中与拖拽选择换算。
pub(crate) struct InputGeom {
    text_top: u16,
    content_x: u16,
    content_w: u16,
    visible: usize,
    scroll: usize,
}

impl InputGeom {
    /// 输入框文本内容区左上角单元格（绝对屏幕坐标）；仅测试使用。
    #[cfg(test)]
    pub(crate) fn text_origin(&self) -> (u16, u16) {
        (self.content_x, self.text_top)
    }

    /// 坐标是否落在输入框文本内容区内。
    pub(crate) fn contains(&self, column: u16, row: u16) -> bool {
        self.content_w > 0
            && row >= self.text_top
            && row
                < self
                    .text_top
                    .saturating_add(u16::try_from(self.visible).unwrap_or(u16::MAX))
            && column >= self.content_x
            && column < self.content_x.saturating_add(self.content_w)
    }

    /// 单元格 →（视觉行索引，内容列）；行、列各自钳到可视区。
    pub(crate) fn cell(&self, column: u16, row: u16) -> (usize, u16) {
        let bottom = self
            .text_top
            .saturating_add(u16::try_from(self.visible).unwrap_or(u16::MAX))
            .saturating_sub(1);
        let local = row.clamp(self.text_top, bottom);
        let visual_row = self.scroll + usize::from(local - self.text_top);
        let right = self
            .content_x
            .saturating_add(self.content_w)
            .saturating_sub(1);
        let col = column.clamp(self.content_x, right) - self.content_x;
        (visual_row, col)
    }
}

/// lx 输入框几何（含边框）；（绝对屏幕坐标）；供提及面板贴附计算。
pub(crate) fn input_rect(area: Rect, lx: &Lx) -> Option<Rect> {
    layout(area, lx).input
}

/// lx 输入框文本区几何；`area` 为窗格内容区。
pub(crate) fn input_geom(area: Rect, lx: &Lx) -> Option<InputGeom> {
    let lay = layout(area, lx);
    let input = lay.input?;
    if lay.input_visible == 0 {
        return None;
    }
    Some(InputGeom {
        text_top: input.y + 1,
        content_x: input.x + INPUT_PREFIX,
        content_w: input
            .width
            .saturating_sub(INPUT_PREFIX)
            .saturating_sub(INPUT_SCROLLBAR),
        visible: lay.input_visible,
        scroll: lay.input_scroll,
    })
}

/// lx 输入框滚动条几何；内容未溢出时为 None。
pub(crate) fn input_scrollbar(area: Rect, lx: &Lx) -> Option<scrollbar::ScrollbarLayout> {
    let lay = layout(area, lx);
    let input = lay.input?;
    if lay.input_visible == 0 {
        return None;
    }
    let track = Rect::new(
        input.x,
        input.y + 1,
        input.width,
        u16::try_from(lay.input_visible).unwrap_or(u16::MAX),
    );
    scrollbar::layout(
        track,
        lx.visual_rows().len(),
        lay.input_visible,
        lay.input_scroll,
    )
}

/// `lx` 粘贴面板渲染数据。
struct PastePanelData {
    area: Rect,
    rows: Vec<(String, String)>,
    active: usize,
    disabled: Option<[bool; 4]>,
}

/// 构造文本粘贴面板的条目列表与可用区域；面板未打开或空间不足返回 None。
fn text_paste_panel_rows(area: Rect, lx: &Lx) -> Option<PastePanelData> {
    let panel = lx.text_paste_panel()?;
    let input = layout(area, lx).input?;
    let margin: u16 = if area.width >= 24 { MARGIN } else { 0 };
    let content_x = area.x + margin;
    let content_w = area.width.saturating_sub(margin.saturating_mul(2));
    let region_h = input.y.saturating_sub(area.y);
    if region_h < 3 {
        return None;
    }
    let count = text::text_paste_lines(panel.line_count);
    let fold_hint = if panel.collapse_enabled() {
        count.clone()
    } else {
        text::TEXT_PASTE_TOO_LARGE.to_string()
    };
    let rows: Vec<(String, String)> = crate::app::paste::TextPasteAction::ALL
        .iter()
        .map(|action| match action {
            crate::app::paste::TextPasteAction::Raw => {
                (text::TEXT_PASTE_ACTION_RAW.to_string(), count.clone())
            }
            crate::app::paste::TextPasteAction::Text => {
                (text::TEXT_PASTE_ACTION_TEXT.to_string(), fold_hint.clone())
            }
            crate::app::paste::TextPasteAction::Code => {
                (text::TEXT_PASTE_ACTION_CODE.to_string(), fold_hint.clone())
            }
            crate::app::paste::TextPasteAction::Log => {
                (text::TEXT_PASTE_ACTION_LOG.to_string(), fold_hint.clone())
            }
        })
        .collect();
    let panel_area = Rect::new(content_x, area.y, content_w, region_h);
    Some(PastePanelData {
        area: panel_area,
        rows,
        active: panel.active(),
        disabled: Some(panel.disabled_mask()),
    })
}

/// 构造粘贴面板的条目列表与可用区域；面板未打开或空间不足返回 None。
fn paste_panel_rows(area: Rect, lx: &Lx) -> Option<PastePanelData> {
    if let Some(data) = text_paste_panel_rows(area, lx) {
        return Some(data);
    }
    let panel = lx.paste_panel()?;
    let input = layout(area, lx).input?;
    let margin: u16 = if area.width >= 24 { MARGIN } else { 0 };
    let content_x = area.x + margin;
    let content_w = area.width.saturating_sub(margin.saturating_mul(2));
    let region_h = input.y.saturating_sub(area.y);
    if region_h < 3 {
        return None;
    }
    let count = text::paste_item_count(panel.candidates.len());
    let rows: Vec<(String, String)> = PasteAction::ALL
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
    let panel_area = Rect::new(content_x, area.y, content_w, region_h);
    Some(PastePanelData {
        area: panel_area,
        rows,
        active: panel.active(),
        disabled: None,
    })
}

/// 组装粘贴面板视图。
fn paste_view<'a>(
    items: &'a [CommandItem<'a>],
    active: usize,
    panel_area: Rect,
    disabled: Option<&'a [bool]>,
) -> CommandPanelView<'a> {
    CommandPanelView {
        items,
        active,
        current: None,
        window_anchor: Some(active),
        window_start: None,
        anchor_row: panel_area.height,
        max_height: None,
        title: Some(text::PASTE_PANEL_TITLE),
        right_title: None,
        footer: Some(text::LX_PASTE_PANEL_FOOTER),
        width: Some(panel_area.width),
        placement: command_panel::PanelPlacement::Above,
        borderless: true,
        disabled,
        ..Default::default()
    }
}

/// `lx` 路径粘贴面板布局；面板未打开或空间不足返回 None。
pub(crate) fn paste_layout(area: Rect, lx: &Lx) -> Option<command_panel::PanelLayout> {
    let data = paste_panel_rows(area, lx)?;
    let items: Vec<CommandItem<'_>> = data
        .rows
        .iter()
        .map(|(label, preview)| CommandItem::Inline {
            label,
            preview,
            tag: "",
        })
        .collect();
    command_panel::layout(
        data.area,
        &paste_view(
            &items,
            data.active,
            data.area,
            data.disabled.as_ref().map(|d| d.as_slice()),
        ),
    )
}

/// `lx` 路径粘贴面板条目命中：返回被点中的操作索引；未打开或未命中返回 None。
pub(crate) fn paste_item_at(area: Rect, lx: &Lx, column: u16, row: u16) -> Option<usize> {
    let data = paste_panel_rows(area, lx)?;
    let items: Vec<CommandItem<'_>> = data
        .rows
        .iter()
        .map(|(label, preview)| CommandItem::Inline {
            label,
            preview,
            tag: "",
        })
        .collect();
    let layout = command_panel::layout(
        data.area,
        &paste_view(
            &items,
            data.active,
            data.area,
            data.disabled.as_ref().map(|d| d.as_slice()),
        ),
    )?;
    command_panel::item_at(&layout, &items, column, row)
}

/// 坐标是否落在任意已打开的 `lx` 输入框命令面板（粘贴 / `@` 提及 / 快捷引用 / `/` 斜杠）矩形内。
///
/// 浮层打开期间，落在面板顶栏横线、空状态占位行或滚动条列上的点击必须由浮层拦截吞掉，
/// 严禁穿透到底层欢迎页入口、历史卡片或文本选区。
pub(crate) fn command_panel_contains(area: Rect, lx: &Lx, column: u16, row: u16) -> bool {
    let pos = (column, row).into();
    paste_layout(area, lx).is_some_and(|l| l.rect.contains(pos))
        || mention_layout(area, lx).is_some_and(|l| l.rect.contains(pos))
        || refer_layout(area, lx).is_some_and(|l| l.rect.contains(pos))
        || slash_layout(area, lx).is_some_and(|l| l.rect.contains(pos))
        || layout(area, lx).panel.is_some_and(|r| r.contains(pos))
}

/// 绘制路径粘贴参照面板：锚定输入框上沿、复用命令面板渲染（只读状态）。
pub(super) fn render_paste_panel(area: Rect, buf: &mut Buffer, lx: &Lx) {
    let Some(data) = paste_panel_rows(area, lx) else {
        return;
    };
    let items: Vec<CommandItem<'_>> = data
        .rows
        .iter()
        .map(|(label, preview)| CommandItem::Inline {
            label,
            preview,
            tag: "",
        })
        .collect();
    command_panel::render(
        data.area,
        buf,
        &paste_view(
            &items,
            data.active,
            data.area,
            data.disabled.as_ref().map(|d| d.as_slice()),
        ),
    );
}

/// md 面板宽度：最宽条目（标签 + 预览）与底栏提示取大 + 边框与内边距，夹取到内容区宽度。
pub(super) fn panel_width(panel: &BlockPanel, content_w: u16) -> u16 {
    let inner = panel
        .items()
        .iter()
        .map(|id| {
            let (label, preview) = text::block_command_text(*id);
            label.width() + 2 + preview.width()
        })
        .max()
        .unwrap_or(0)
        .max(text::COMMAND_PANEL_FOOTER.width())
        .max(text::LX_BLOCK_PANEL_TITLE.width());
    u16::try_from(inner + 4).unwrap_or(u16::MAX).min(content_w)
}

/// md 格式面板：强调色直角边框 + 内框粗体标题 + 候选行（选中项整行高亮）+ 底栏快捷键行（对齐 herdr 面板）。
pub(super) fn draw_panel(grid: &mut Grid, rect: Rect, panel: &BlockPanel) {
    draw_box(grid, rect, style::accent());
    let inner_w = rect.width.saturating_sub(4);
    if inner_w == 0 {
        return;
    }
    let text_x = rect.x + 2;
    let text_right = text_x.saturating_add(inner_w);
    grid.put_clipped(
        text_x,
        rect.y + 1,
        text::LX_BLOCK_PANEL_TITLE,
        style::strong(),
        inner_w,
    );
    for (index, id) in panel.items().iter().enumerate() {
        let y = rect.y + 3 + index as u16;
        let (label, preview) = text::block_command_text(*id);
        let active = index == panel.active();
        if active {
            for x in rect.x + 1..rect.right().saturating_sub(1) {
                grid.set(x, y, ' ', style::overlay_selection());
            }
        }
        let base = if active {
            style::overlay_selection()
        } else {
            style::text()
        };
        grid.put_clipped(text_x, y, &label, base, inner_w);
        let preview_x = text_x + u16::try_from(label.width()).unwrap_or(0) + 2;
        if preview_x < text_right {
            grid.put_clipped(
                preview_x,
                y,
                &preview,
                base.patch(style::muted()),
                text_right - preview_x,
            );
        }
    }
    grid.put_clipped(
        text_x,
        rect.bottom() - 2,
        text::COMMAND_PANEL_FOOTER,
        style::muted(),
        inner_w,
    );
}

/// 直角细线边框与内部清底（md 面板用）。
fn draw_box(grid: &mut Grid, rect: Rect, border: Style) {
    if rect.width < 2 || rect.height < 2 {
        return;
    }
    let right = rect.right() - 1;
    let bottom = rect.bottom() - 1;
    for y in rect.y + 1..bottom {
        for x in rect.x + 1..right {
            grid.set(x, y, ' ', Style::default());
        }
    }
    grid.set(rect.x, rect.y, '┌', border);
    grid.set(right, rect.y, '┐', border);
    grid.set(rect.x, bottom, '└', border);
    grid.set(right, bottom, '┘', border);
    for x in rect.x + 1..right {
        grid.set(x, rect.y, '─', border);
        grid.set(x, bottom, '─', border);
    }
    for y in rect.y + 1..bottom {
        grid.set(rect.x, y, '│', border);
        grid.set(right, y, '│', border);
    }
}
