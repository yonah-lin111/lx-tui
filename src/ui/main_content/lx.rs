//! lx 欢迎页：顶部品牌区（像素图标 + 文案）、标签页式入口与底部多行 markdown 输入框。
//!
//! 整页以单元格网格构建：同一份网格既用于绘制，也用于整页文本复制，
//! 因此渲染与取词不会漂移。渲染只读状态、只写 Buffer。

use ratatui::buffer::{Buffer, CellDiffOption};
use ratatui::layout::Rect;
use ratatui::style::Style;
use unicode_width::UnicodeWidthStr;

use crate::app::markdown::BlockPanel;
use crate::app::paste::PasteAction;
use crate::app::selection::Selection;
use crate::app::state::{Lx, LxEntry};
use crate::ui::widgets::command_panel::{self, CommandItem, CommandPanelView};
use crate::ui::widgets::scrollbar;
use crate::ui::{style, text};

mod ask;
mod input;
mod mention;
mod selection;

pub(crate) use ask::rect as ask_rect;
pub(crate) use mention::{item_at as mention_item_at, layout as mention_layout};
pub(crate) use selection::content_geom;
pub use selection::copy_text;

pub(crate) mod brand;
pub(crate) mod chat_bar;
pub(crate) mod grid;
pub(crate) mod history;
mod history_hit;
pub(crate) mod notice;
pub(crate) mod slash;
pub(crate) mod status;
pub(crate) mod task;
pub(crate) mod toast;

pub(crate) use history::{PaneCache, RenderCache};
use history_hit::chat_geometry_is_live;
pub(crate) use history_hit::{
    LxChatHit, card_target_at, chat_hit, fold_target_at, history_center_target, history_rows,
    history_scrollbar, history_view, sticky_prompt_jump, task_agents_card_target_at,
    task_agents_scrollbar,
};
pub(crate) use slash::{item_at as slash_item_at, layout as slash_layout};

use grid::{CONTINUATION, Grid};

#[cfg(test)]
pub(crate) use brand::{ICON_LX, LOGO_GAP, icon_pixel};
pub(crate) use brand::{LOGO_ROWS, LOGO_WIDTH, draw_art};

/// 输入框水平外边距、最小宽度与最小高度（1 内容行 + 上下分割线）；
/// 边距、高度比例、前缀与滚动条槽与状态层折行宽度共用同一组 `layout` 常量。
const MARGIN: u16 = crate::layout::LX_INPUT_MARGIN;
const MIN_INPUT_WIDTH: u16 = 8;
const INPUT_MIN_BOX: u16 = crate::layout::LX_INPUT_MIN_BOX;
/// 输入框最大高度占内容区高度的百分比。
const INPUT_MAX_PERCENT: u16 = crate::layout::LX_INPUT_HEIGHT_PERCENT;
/// 输入框 `> ` 前缀列数与右侧滚动条预留列数。
const INPUT_PREFIX: u16 = crate::layout::LX_INPUT_PREFIX;
const INPUT_SCROLLBAR: u16 = crate::layout::LX_INPUT_SCROLLBAR;
/// 入口区：单行高度、横排标签间距（含分隔符）、竖排行距与区块间距。
const ENTRY_HEIGHT: u16 = 1;
const ENTRY_GAP: u16 = 3;
const ENTRY_VGAP: u16 = 1;
const SECTION_GAP: u16 = 1;
/// 输入框上下分割线字符。
const RULE: char = '─';

/// 渲染 lx 页；返回聚焦时输入光标的相对坐标（行、列，相对内容区左上角）。
///
/// `selection` 为当前 lx 选区（内容行、列，相对内容区左上角），用于反显高亮。
pub fn render(
    area: Rect,
    buf: &mut Buffer,
    lx: &Lx,
    focused: bool,
    selection: Option<&Selection>,
    cache: &mut PaneCache,
) -> Option<(u16, u16)> {
    if area.width == 0 || area.height == 0 {
        return None;
    }
    let (grid, cursor, sticky) = build(area, lx, focused, cache);
    for dy in 0..area.height {
        for dx in 0..area.width {
            let glyph = grid.cell(area.x + dx, area.y + dy);
            if let Some(cell) = buf.cell_mut((area.x + dx, area.y + dy)) {
                // 宽字符的后续列标记为 diff 跳过，禁止渲染出半个字符。
                if glyph.ch == CONTINUATION {
                    cell.set_diff_option(CellDiffOption::Skip);
                    continue;
                }
                cell.set_char(glyph.ch);
                cell.set_style(glyph.style);
                cell.set_diff_option(CellDiffOption::None);
            }
        }
    }
    if let Some(sel) = selection
        && let Some(geom) = selection::content_geom(area, lx, cache)
    {
        selection::paint_selection(buf, geom, sel);
    }
    render_input_scrollbar(area, buf, lx);
    // 置顶用户消息行：表现层，blit 之后直写 Buffer（不进网格 → 不参与复制与选区）。
    if let Some(row) = sticky
        && let Some(message) = lx.chat.messages.get(row.message)
    {
        history::render_sticky_prompt(buf, row, message);
    }
    // 窗格底边框：面板打开时整行归面板的键位提示（不显示模型名 / 用量 / 进程计数）；
    // 否则照旧画状态（模型 · 上下文占用 · 后台任务计数）。
    match lx.task.as_ref() {
        Some(page) => task::draw_footer(buf, area, page),
        None => status::render(buf, area, lx),
    }
    // 提及面板浮于基础内容之上、贴输入框上沿；与粘贴面板互斥。
    mention::render(area, buf, lx);
    render_paste_panel(area, buf, lx);
    render_slash_panel(area, buf, lx);
    // 本页 toast 最后画：盖住历史、输入框与上述所有浮层，但**只在窗格内容区内**。
    // 全局 toast 覆盖层不再承载 lx 反馈（见 `app::state::Lx::toast`）。
    if let Some(toast) = lx.toast.as_ref() {
        toast::draw(buf, area, toast);
    }
    cursor
}

/// 本页 toast 盒矩形（绝对屏幕坐标）；无 toast 或空间不足返回 None。
///
/// 绘制（[`render`] 内的 `toast::draw`）与命中（点击关闭 / hover 冻结计时）共用它，
/// 保证所见即所点。
pub(crate) fn toast_rect(area: Rect, lx: &Lx) -> Option<Rect> {
    toast::rect(area, lx.toast.as_ref()?)
}

/// 绘制 `/` 命令面板（几何/条目见 `slash` 子模块）。
fn render_slash_panel(area: Rect, buf: &mut Buffer, lx: &Lx) {
    if lx.slash.is_none() {
        return;
    }
    slash::render(area, buf, lx);
}

/// 绘制输入框滚动条：软换行后的视觉行溢出可视高度时，覆盖在最右预留列。
fn render_input_scrollbar(area: Rect, buf: &mut Buffer, lx: &Lx) {
    if lx.input.is_empty() {
        return;
    }
    if let Some(bar) = input_scrollbar(area, lx) {
        scrollbar::render_buffer(buf, &bar);
    }
}

/// 把历史滚动条渲染到网格（贴右缘一列；轨道留空、只画 thumb）。
///
/// 与公共 `scrollbar` 组件同口径：轨道不画符号，只保留占位列与命中几何。
fn render_history_scrollbar(grid: &mut Grid, bar: &scrollbar::ScrollbarLayout) {
    for y in bar.thumb.y..bar.thumb.bottom() {
        grid.set(bar.thumb.x, y, '▐', style::text());
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

/// `ask` 面板命中：选项行 / Other / Submit / 题目标签；面板未激活为 None。
pub(crate) fn ask_hit(
    area: Rect,
    lx: &Lx,
    column: u16,
    row: u16,
) -> Option<crate::app::state::AskHit> {
    ask::hit(area, lx, column, row)
}

/// `ask` 面板滚轮分区；指针在面板外为 None（事件层据此决定是否吞掉）。
pub(crate) fn ask_wheel_zone(
    area: Rect,
    lx: &Lx,
    column: u16,
    row: u16,
) -> Option<crate::app::state::AskWheelZone> {
    ask::wheel_zone(area, lx, column, row)
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

/// 绘制路径粘贴参照面板：锚定输入框上沿、复用命令面板渲染（只读状态）。
fn render_paste_panel(area: Rect, buf: &mut Buffer, lx: &Lx) {
    let Some(panel) = lx.paste_panel() else {
        return;
    };
    let Some(input) = layout(area, lx).input else {
        return;
    };
    let margin: u16 = if area.width >= 24 { MARGIN } else { 0 };
    let content_x = area.x + margin;
    let content_w = area.width.saturating_sub(margin.saturating_mul(2));
    let region_h = input.y.saturating_sub(area.y);
    if region_h < 3 {
        return;
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
    let items: Vec<CommandItem<'_>> = rows
        .iter()
        .map(|(label, preview)| CommandItem::Inline {
            label,
            preview,
            tag: "",
        })
        .collect();
    let view = CommandPanelView {
        items: &items,
        active: panel.active(),
        current: None,
        window_anchor: Some(panel.active()),
        window_start: None,
        anchor_row: region_h,
        max_height: None,
        title: Some(text::PASTE_PANEL_TITLE),
        right_title: None,
        footer: None,
        ..Default::default()
    };
    let panel_area = Rect::new(content_x, area.y, content_w, region_h);
    command_panel::render(panel_area, buf, &view);
}

/// 命中测试：坐标落在某个入口标签内则返回该入口。
///
/// 入口区在后台任务面板打开时由 [`layout`] 置空（面板接管了那块区域），
/// 因此这里自然拿不到命中——点击不会再穿透到 agent 切换。
pub(crate) fn entry_hit(area: Rect, lx: &Lx, column: u16, row: u16) -> Option<LxEntry> {
    layout(area, lx)
        .entries
        .into_iter()
        .find(|(_, rect)| rect.contains((column, row).into()))
        .map(|(entry, _)| entry)
}

/// 一段文案的落位；水平居中于 `width`。
#[derive(Clone, Copy)]
struct TextRect {
    x: u16,
    y: u16,
    width: u16,
}

/// lx 页布局：所有组件矩形（绝对屏幕坐标）。
struct Layout {
    input: Option<Rect>,
    /// `ask` 面板外框（激活时替换输入框；`input` 同时为 None）。
    ask: Option<Rect>,
    /// 输入框可见内容行的滚动起点（逻辑行索引）。
    input_scroll: usize,
    /// 输入框可见内容行数。
    input_visible: usize,
    /// 输入框上方聊天栏（loading 转轮 + 右端回到底部按钮）；无则为 None。
    chat_bar: Option<Rect>,
    /// 后台任务**输出容器**矩形：占用原历史区（消息列表的位置）；未打开时为 None。
    task_output: Option<Rect>,
    /// 后台任务 **item 列表**矩形：仍在输入框原来的位置（替换输入框）；未打开时为 None。
    task_list: Option<Rect>,
    panel: Option<Rect>,
    entries: Vec<(LxEntry, Rect)>,
    horizontal: bool,
    icon: Option<(u16, u16)>,
    description: Option<TextRect>,
    /// 空间不足时退化为居中字标。
    fallback: bool,
}

impl Layout {
    fn empty(fallback: bool) -> Self {
        Self {
            input: None,
            ask: None,
            input_scroll: 0,
            input_visible: 0,
            chat_bar: None,
            task_output: None,
            task_list: None,
            panel: None,
            entries: Vec::new(),
            horizontal: true,
            icon: None,
            description: None,
            fallback,
        }
    }
}

/// 后台任务面板两段式矩形的公开快照（`(item 列表, 输出容器)`）。
///
/// 命中测试在 `event/` 里做，而 `Layout` 是 `ui/` 私有类型；这里只把命中真正需要的
/// 两个矩形交出去，渲染与命中共用 `layout` 的同一份几何。
pub struct TaskRects {
    /// item 列表：原输入框位置。
    pub list: Rect,
    /// 输出容器：原消息列表位置。
    pub output: Rect,
}

/// 面板打开时的两段矩形；未打开返回 None。
pub fn task_rects(area: Rect, lx: &Lx) -> Option<TaskRects> {
    let lay = layout(area, lx);
    Some(TaskRects {
        list: lay.task_list?,
        output: lay.task_output?,
    })
}

/// 计算整页布局。
fn layout(area: Rect, lx: &Lx) -> Layout {
    if area.width == 0 || area.height == 0 {
        return Layout::empty(true);
    }
    let margin: u16 = if area.width >= 24 { MARGIN } else { 0 };
    let content_x = area.x + margin;
    let content_w = area.width.saturating_sub(margin.saturating_mul(2));

    if area.height < INPUT_MIN_BOX + 1 || content_w < MIN_INPUT_WIDTH {
        return Layout::empty(true);
    }

    // 输入框：高度随软换行后的视觉行数增长，最多占内容区高度的一定比例（避免挤掉上方内容）。
    // 输入框下沿贴窗格底部；状态（模型 · 供应商 · 上下文占用）绘制在其底边框上。
    //
    // 后台任务面板打开时输入框**仍在原位**（不参与面板布局）：面板接管的是上方
    // 历史区，这样退出面板后消息列表与草稿都还是离开时的样子。
    // 面板打开时这一块改由 item 列表占用：高度固定（顶边框 + tab 行 + LIST_MAX），
    // 不随草稿行数变化——否则开关面板会让上方输出容器跳动。
    // `ask` 面板占用输入框的位置并替换它（design §8.1）；后台任务页优先级更高。
    let ask_active = lx.ask.is_some();
    let (box_h, visible, scroll) = if lx.task.is_some() {
        (task::list_height(), 0, 0)
    } else if ask_active {
        (ask::height(area.height, content_w, lx), 0, 0)
    } else {
        let rows = lx.visual_rows();
        let total = rows.len().max(1);
        let max_box = (area.height.saturating_mul(INPUT_MAX_PERCENT) / 100).max(INPUT_MIN_BOX);
        let cap = usize::from(max_box.saturating_sub(2)).max(1);
        let visible = total.min(cap);
        let box_h = u16::try_from(visible).unwrap_or(u16::MAX).saturating_add(2);
        let max_scroll = total.saturating_sub(visible);
        (box_h, visible, lx.scroll().min(max_scroll))
    };
    let input = Rect::new(
        content_x,
        area.bottom().saturating_sub(box_h),
        content_w,
        box_h,
    );

    let mut out = Layout::empty(false);
    if ask_active {
        out.ask = Some(input);
    } else {
        out.input = Some(input);
        out.input_scroll = scroll;
        out.input_visible = visible;
    }

    // 输入框上方一行的聊天栏：流中显示 loading 转轮，或上滚未跟随底部时显示回到底部按钮。
    if chat_bar::visible(lx) {
        out.chat_bar = chat_bar::rect(area, input);
    }

    // 后台任务面板：**两段式**布局（用户明确要求）
    //
    //   ┌──────────────────────────────┐
    //   │ 输出容器（原消息列表位置）        │ ← out.task_output
    //   ├──────────────────────────────┤
    //   │ … 消息列表在它下方不可见 …       │
    //   └──────────────────────────────┘
    //   ┌──────────────────────────────┐
    //   │ task item 列表（原输入框位置）    │ ← out.task_list
    //   └──────────────────────────────┘
    //
    // 输出容器吃掉**历史区**（`history_rect` 同一份几何），item 列表继续占输入框
    // 矩形并沿用原本的固定高度；两者之间是面板自己的分隔行。
    if lx.task.is_some() {
        let bottom = out.chat_bar.map_or(input.y, |bar| bar.y);
        let height = bottom.saturating_sub(area.y);
        if height > 0 {
            out.task_output = Some(Rect::new(input.x, area.y, input.width, height));
        }
        out.task_list = Some(input);
    }

    // md 格式面板：贴输入框上方（聊天栏之上）；放不下则省略。
    let panel_base = out.chat_bar.map_or(input.y, |bar| bar.y);
    if let Some(panel) = lx.panel.as_ref() {
        let rows = panel.items().len();
        let panel_h = u16::try_from(rows).unwrap_or(u16::MAX).saturating_add(2);
        let panel_w = panel_width(panel, content_w);
        if panel_base >= area.y + panel_h + SECTION_GAP {
            out.panel = Some(Rect::new(content_x, panel_base - panel_h, panel_w, panel_h));
        }
    }

    let region_top = area.y;

    // 入口区：标签页式单行，横排放不下则竖排。
    let labels: Vec<(LxEntry, &str)> = LxEntry::ALL
        .iter()
        .map(|entry| (*entry, text::lx_entry_label(*entry)))
        .collect();
    let label_widths: Vec<u16> = labels
        .iter()
        .map(|(_, label)| u16::try_from(label.width()).unwrap_or(u16::MAX))
        .collect();
    let max_label = label_widths.iter().copied().max().unwrap_or(0);
    let horizontal_total = label_widths
        .iter()
        .fold(0u16, |sum, width| sum.saturating_add(*width))
        .saturating_add(ENTRY_GAP.saturating_mul(2));
    let horizontal = horizontal_total <= content_w;
    let entries_h = if horizontal {
        ENTRY_HEIGHT
    } else {
        ENTRY_HEIGHT
            .saturating_mul(3)
            .saturating_add(ENTRY_VGAP.saturating_mul(2))
    };
    let entries_fit = horizontal || max_label <= content_w;

    let tip_h = ENTRY_HEIGHT;
    // 入口区与 Tip 的上限：输入框上沿（有聊天栏时为其上沿）。
    let top_limit = out.chat_bar.map_or(input.y, |bar| bar.y);

    // 入口按钮行对齐窗格垂直中心（与主区/prompt 交换按钮同一行），且不与输入框重叠。
    let center = area.y + area.height / 2;
    let entries_top = center
        .min(top_limit.saturating_sub(entries_h + brand::UNDERLINE_ROWS))
        .max(region_top);
    // 非首页（已有消息或撤销提示）与后台任务面板打开时都**不登记入口区**：
    // 那三个入口标签根本没画出来，留下命中矩形会让点击历史内容穿透去切换 agent
    // （用户报的真实 bug）。
    let entries_ok = lx.task.is_none()
        && !history::has_history(lx)
        && entries_fit
        && entries_top
            .saturating_add(entries_h)
            .saturating_add(brand::UNDERLINE_ROWS)
            <= top_limit;

    // Tip 行贴按钮上方。
    let tip_top = if entries_ok {
        entries_top
            .checked_sub(SECTION_GAP + tip_h)
            .filter(|y| *y >= region_top)
    } else {
        None
    };

    // 品牌区：像素图标水平居中、底边距 Tip 2 行；放不下则整数倍缩小，仍放不下则隐藏。
    out.icon = tip_top.and_then(|tip_y| brand::place_icon(tip_y, region_top, content_x, content_w));

    if let Some(y) = tip_top {
        out.description = Some(TextRect {
            x: content_x,
            y,
            width: content_w,
        });
    }
    if entries_ok {
        out.horizontal = horizontal;
        out.entries = place_entries(
            &labels,
            &label_widths,
            horizontal,
            content_x,
            content_w,
            entries_top,
        );
    }
    out
}

/// md 面板宽度：最宽条目（标签 + 预览）+ 边框与内边距，夹取到内容区宽度。
fn panel_width(panel: &BlockPanel, content_w: u16) -> u16 {
    let inner = panel
        .items()
        .iter()
        .map(|id| {
            let (label, preview) = text::block_command_text(*id);
            label.width() + 2 + preview.width()
        })
        .max()
        .unwrap_or(0);
    u16::try_from(inner + 4).unwrap_or(u16::MAX).min(content_w)
}

/// 放置入口标签：横排整体居中、竖排逐行居中。
fn place_entries(
    labels: &[(LxEntry, &str)],
    label_widths: &[u16],
    horizontal: bool,
    x0: u16,
    width: u16,
    top: u16,
) -> Vec<(LxEntry, Rect)> {
    let mut out = Vec::with_capacity(labels.len());
    if horizontal {
        let total = label_widths
            .iter()
            .fold(0u16, |sum, entry| sum.saturating_add(*entry))
            .saturating_add(ENTRY_GAP.saturating_mul(2));
        let mut x = x0 + width.saturating_sub(total) / 2;
        for ((entry, _), label_w) in labels.iter().zip(label_widths) {
            out.push((*entry, Rect::new(x, top, *label_w, ENTRY_HEIGHT)));
            x = x.saturating_add(*label_w).saturating_add(ENTRY_GAP);
        }
    } else {
        let mut y = top;
        for ((entry, _), label_w) in labels.iter().zip(label_widths) {
            let x = x0 + width.saturating_sub(*label_w) / 2;
            out.push((*entry, Rect::new(x, y, *label_w, ENTRY_HEIGHT)));
            y = y.saturating_add(ENTRY_HEIGHT).saturating_add(ENTRY_VGAP);
        }
    }
    out
}

/// 构建整页网格；返回网格、聚焦时的输入光标相对坐标与置顶行绘制目标。
fn build(
    area: Rect,
    lx: &Lx,
    focused: bool,
    cache: &mut PaneCache,
) -> (Grid, Option<(u16, u16)>, Option<history::StickyRow>) {
    let lay = layout(area, lx);
    if history::has_history(lx) {
        return build_chat(area, lx, focused, &lay, cache);
    }
    let mut grid = Grid::new(area);
    if lay.fallback {
        let y = area.y + area.height.saturating_sub(1) / 2;
        grid.put_centered(area.x, area.width, y, text::LX_TITLE, style::strong());
        return (grid, None, None);
    }

    if let Some((ix, iy)) = lay.icon {
        brand::draw_icon(&mut grid, ix, iy, lx.entry);
    }
    if let Some(rect) = lay.description {
        brand::draw_tip(
            &mut grid,
            Rect::new(rect.x, rect.y, rect.width, 1),
            lx.entry,
        );
    }

    brand::draw_entries(&mut grid, lx.entry, &lay.entries, lay.horizontal);

    chat_bar::draw(&mut grid, lay.chat_bar, lx);
    let mut cursor = None;
    // 后台任务面板打开时输入框已被替换：与 `build_chat` 同一条路径，
    // 否则会画出上下两条`─`和一个空占位，而面板本身不画——按键却全被
    // `apply_task_page_input` 吞掉，只剩一个死画面。
    if let Some(page) = lx.task.as_ref() {
        if let Some(rect) = lay.task_output {
            if page.tab == crate::app::state::TaskTab::Agents {
                task::draw_agents_detail(&mut grid, rect, lx, cache);
            } else {
                task::draw_output_container(&mut grid, rect, page);
            }
        }
        if let Some(rect) = lay.task_list {
            task::draw_list(&mut grid, rect, page, &lx.agents);
        }
    } else if let Some(rect) = lay.ask {
        cursor = ask::draw(&mut grid, rect, area.height, lx, focused)
            .map(|(row, col)| (row.saturating_sub(area.y), col.saturating_sub(area.x)));
    } else if let Some(input) = lay.input {
        cursor = input::draw(
            &mut grid,
            input,
            lx,
            focused,
            lay.input_scroll,
            lay.input_visible,
        )
        .map(|(row, col)| (row.saturating_sub(area.y), col.saturating_sub(area.x)));
    }
    if let (Some(rect), Some(panel)) = (lay.panel, lx.panel.as_ref()) {
        draw_panel(&mut grid, rect, panel);
    }
    (grid, cursor, None)
}

/// 聊天模式构建：历史区 + 输入框（状态在**窗格底边框**上，由 `render` 覆盖绘制）。
fn build_chat(
    area: Rect,
    lx: &Lx,
    focused: bool,
    lay: &Layout,
    cache: &mut PaneCache,
) -> (Grid, Option<(u16, u16)>, Option<history::StickyRow>) {
    let mut grid = Grid::new(area);
    // 输入框与 ask 面板占据同一矩形：历史区上沿与宽度都跟它走。
    let bottom_box = lay.input.or(lay.ask);
    let history_bottom = if bottom_box.is_some() {
        lay.chat_bar.map_or_else(
            || bottom_box.map(|input| input.y).unwrap_or(area.bottom()),
            |bar| bar.y,
        )
    } else {
        area.bottom()
    };
    // 历史区与输入框同宽对齐（列 = 输入框列），高度到输入框（或聊天栏）上沿。
    let history_rect = Rect::new(
        bottom_box.map(|input| input.x).unwrap_or(area.x),
        area.y,
        bottom_box.map(|input| input.width).unwrap_or(area.width),
        history_bottom.saturating_sub(area.y),
    );
    // 历史视图：滚动偏移 / body（置顶行让位）/ 置顶行结论，统一由纯函数给出。
    //
    // 后台任务面板打开时**不画历史**（面板占用同一块矩形），但历史几何仍照常
    // 计算：滚动偏移与 `follow_bottom` 全程不动，关闭面板后消息列表原样回来。
    let sticky = if lay.task_output.is_some() {
        None
    } else {
        let history_view = history::view(history_rect, lx, cache);
        history::render(&mut grid, history_view.body, lx, history_view.scroll, cache);
        // 历史滚动条贴右缘一列（内容溢出时；轨道只覆盖 body）。
        if let Some(bar) = history::scrollbar(
            history_view.body,
            history_view.total_rows,
            usize::from(history_view.body.height),
            history_view.scroll,
        ) {
            render_history_scrollbar(&mut grid, &bar);
        }
        history::sticky_rect(&history_view)
    };

    chat_bar::draw(&mut grid, lay.chat_bar, lx);
    let mut cursor = None;
    // 后台任务面板：输出容器占原消息列表位置，item 列表仍在原输入框位置。
    if let Some(page) = lx.task.as_ref() {
        if let Some(rect) = lay.task_output {
            if page.tab == crate::app::state::TaskTab::Agents {
                task::draw_agents_detail(&mut grid, rect, lx, cache);
            } else {
                task::draw_output_container(&mut grid, rect, page);
            }
        }
        if let Some(rect) = lay.task_list {
            task::draw_list(&mut grid, rect, page, &lx.agents);
        }
    } else if let Some(rect) = lay.ask {
        cursor = ask::draw(&mut grid, rect, area.height, lx, focused)
            .map(|(row, col)| (row.saturating_sub(area.y), col.saturating_sub(area.x)));
    } else if let Some(input) = lay.input
        && lay.input_visible > 0
    {
        // 输入框容器（面板未打开时）。
        cursor = input::draw(
            &mut grid,
            input,
            lx,
            focused,
            lay.input_scroll,
            lay.input_visible,
        )
        .map(|(row, col)| (row.saturating_sub(area.y), col.saturating_sub(area.x)));
    }
    // 面板：块命令 / 粘贴 / 提及 / `/` 命令。
    if let (Some(rect), Some(panel)) = (lay.panel, lx.panel.as_ref()) {
        draw_panel(&mut grid, rect, panel);
    }
    (grid, cursor, sticky)
}

/// 聊天历史区矩形（与输入框同宽对齐；无消息返回 None）；滚轮/命中/滚动条用。
pub fn history_rect(area: Rect, lx: &Lx) -> Option<Rect> {
    if !history::has_history(lx) {
        return None;
    }
    let lay = layout(area, lx);
    let bottom_box = lay.input.or(lay.ask);
    // 历史区上沿到输入框（有聊天栏时为其上沿）。
    let bottom = lay.chat_bar.map_or_else(
        || bottom_box.map(|input| input.y).unwrap_or(area.bottom()),
        |bar| bar.y,
    );
    let height = bottom.saturating_sub(area.y);
    if height == 0 {
        return None;
    }
    // 与输入框同宽对齐：x/width 取输入框列（扣掉两端外边距），右侧含滚动条预留列。
    let (x, width) = match bottom_box {
        Some(input) => (input.x, input.width),
        None => (area.x, area.width),
    };
    Some(Rect::new(x, area.y, width, height))
}

/// 聊天历史滚轮命中区（与输入框同宽；上沿到输入框上沿，含聊天栏一行）。
///
/// 与 [`history_rect`] 的区别：滚轮区含输入框上方聊天栏（loading/回到底部按钮）一行，
/// 保证在聊天栏上滚轮同样滚动历史；渲染/滚动条仍用不含聊天栏的 [`history_rect`]。
pub fn history_wheel_region(area: Rect, lx: &Lx) -> Option<Rect> {
    if !chat_geometry_is_live(lx) {
        return None;
    }
    let lay = layout(area, lx);
    let bottom_box = lay.input.or(lay.ask);
    let (x, width) = match bottom_box {
        Some(input) => (input.x, input.width),
        None => (area.x, area.width),
    };
    let bottom = bottom_box.map(|input| input.y).unwrap_or(area.bottom());
    let height = bottom.saturating_sub(area.y);
    (height > 0).then(|| Rect::new(x, area.y, width, height))
}

/// 后台任务输出容器的可见行数（键位翻页钳制用）；面板未打开返回 None。
///
/// 与渲染共用 [`layout`] → `task::output_rows` 同一份几何，因此"翻到哪停"与
/// "画到哪停"永远是同一个数。
pub fn task_output_rows(area: Rect, lx: &Lx) -> Option<usize> {
    let rect = layout(area, lx).task_output?;
    Some(task::output_rows(rect))
}

/// Agents 详情区矩形（内边距扣除后的可用区域）；不在该页返回 None。
pub fn task_output_inner(area: Rect, lx: &Lx) -> Option<Rect> {
    task::task_output_rect(area, lx)
}

/// Agents 详情区的 `(可见行数, 总行数)`；不在 Agents 页或无高亮项返回 None。
pub fn task_agents_rows(area: Rect, lx: &Lx) -> Option<(usize, usize)> {
    task::agents_rows(area, lx)
}

/// `ask` 预览区的可见行数（滚动上限钳制用）；无面板或无预览区返回 None。
///
/// 与渲染共用 [`ask::layout`] 的同一份几何，因此"能滚多远"与"画到哪停"是同一个数。
pub fn ask_preview_rows(area: Rect, lx: &Lx) -> Option<usize> {
    ask::preview_rows(area, lx)
}

/// `ask` 选项列表的 `(窗口起点, 总行数, 可视行数)`；滚轮基准与绘制同源。
pub fn ask_list_window(area: Rect, lx: &Lx) -> Option<(usize, usize, usize)> {
    ask::list_window(area, lx)
}

/// md 格式面板：圆角边框 + 标题 + 候选行（标签与预览，高亮项 accent）。
fn draw_panel(grid: &mut Grid, rect: Rect, panel: &BlockPanel) {
    draw_box(grid, rect, style::muted());
    grid.put(
        rect.x + 2,
        rect.y,
        text::LX_BLOCK_PANEL_TITLE,
        style::muted(),
    );
    for (index, id) in panel.items().iter().enumerate() {
        let y = rect.y + 1 + index as u16;
        let (label, preview) = text::block_command_text(*id);
        let active = index == panel.active();
        let label_style = if active {
            style::accent()
        } else {
            style::muted()
        };
        grid.put(rect.x + 2, y, &label, label_style);
        let preview_x = rect.x + 2 + u16::try_from(label.width()).unwrap_or(0) + 2;
        grid.put(preview_x, y, &preview, style::muted());
    }
}

/// 圆角细线边框（md 面板用）。
fn draw_box(grid: &mut Grid, rect: Rect, border: Style) {
    if rect.width < 2 || rect.height < 2 {
        return;
    }
    let right = rect.right() - 1;
    let bottom = rect.bottom() - 1;
    grid.set(rect.x, rect.y, '╭', border);
    grid.set(right, rect.y, '╮', border);
    grid.set(rect.x, bottom, '╰', border);
    grid.set(right, bottom, '╯', border);
    for x in rect.x + 1..right {
        grid.set(x, rect.y, '─', border);
        grid.set(x, bottom, '─', border);
    }
    for y in rect.y + 1..bottom {
        grid.set(rect.x, y, '│', border);
        grid.set(right, y, '│', border);
    }
}

/// 测试辅助：仅构建聊天历史网格并 blit（不含输入框与面板）。
#[cfg(test)]
pub(crate) fn render_history_for_test(area: Rect, buf: &mut Buffer, lx: &Lx, scroll: usize) {
    let mut cache = PaneCache::default();
    let mut grid = Grid::new(area);
    history::render(&mut grid, area, lx, scroll, &mut cache);
    for dy in 0..area.height {
        for dx in 0..area.width {
            let glyph = grid.cell(area.x + dx, area.y + dy);
            if let Some(cell) = buf.cell_mut((area.x + dx, area.y + dy)) {
                if glyph.ch == CONTINUATION {
                    cell.set_diff_option(CellDiffOption::Skip);
                    continue;
                }
                cell.set_char(glyph.ch);
                cell.set_style(glyph.style);
                cell.set_diff_option(CellDiffOption::None);
            }
        }
    }
}

/// 测试辅助：把网格 blit 到缓冲区。
#[cfg(test)]
pub(super) fn blit_grid(area: Rect, buf: &mut Buffer, grid: &Grid) {
    for dy in 0..area.height {
        for dx in 0..area.width {
            let glyph = grid.cell(area.x + dx, area.y + dy);
            if let Some(cell) = buf.cell_mut((area.x + dx, area.y + dy)) {
                if glyph.ch == CONTINUATION {
                    cell.set_diff_option(CellDiffOption::Skip);
                    continue;
                }
                cell.set_char(glyph.ch);
                cell.set_style(glyph.style);
                cell.set_diff_option(CellDiffOption::None);
            }
        }
    }
}

#[cfg(test)]
mod tests;
