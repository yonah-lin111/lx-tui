//! `ask` 面板：几何、绘制与命中（激活时**替换**输入框，design §4.3）。
//!
//! 纯渲染：只读 `Lx`，只写 lx 页网格。几何是唯一来源：绘制、鼠标命中与滚轮分区
//! 全部走 [`AskLayout::compute`]，事件层禁止重算。
//!
//! 版式对照后台任务面板：**只有顶部一条分隔线**，不画四周边框；取消左右分栏，
//! 选项区恒占满全宽。
//!
//! ```text
//! ────────────────────────────────────────────────────────
//!  1 布局   2 主题
//!  侧栏用哪种布局？
//! ❯ (•) 双栏
//!       推荐项，描述按全宽自动换行，续行缩进 6 列。
//!   ( ) 单栏
//!   ( ) Other...                       ← 激活后原地变成 1~3 行输入框
//!        ↵ submit                      ← 仅单题多选 / 多题最后一题
//!   ┌──┬──┐                             ← 预览置于所有选项正下方
//!   │a │b │
//!   └──┴──┘
//! ↑↓ select · enter submit                     esc cancel
//! ```
//!
//! 末尾的提交按钮是一个真实的进度门：多选/末题必须把焦点移上去按回车才提交。
//! 内容超过「窗格高度 70%」时选项区内部滚动（右侧滚动条），顶部题目 tab 始终保留。

use ratatui::layout::Rect;
use ratatui::style::Modifier;
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use crate::app::state::{AskHit, AskState, AskWheelZone, Lx};
use crate::ui::style;
use crate::ui::text::agent as a;

use super::grid::Grid;

/// 面板高度上限占内容区高度的百分比（用户要求：最多 70%，不够就滚动）。
const HEIGHT_PERCENT: u16 = crate::layout::LX_ASK_HEIGHT_PERCENT;
/// 题干最多显示行数。
const MAX_QUESTION_ROWS: u16 = 3;
/// 预览区与选项区之间的空行。
const PREVIEW_GAP: u16 = 1;
/// 预览区最多占用行数。
const PREVIEW_MAX_ROWS: u16 = 8;
/// 选项行前缀列数（高亮箭头 2 + 标记 3 + 空格 1）。
const ROW_PREFIX: u16 = 6;
/// 面板最小可用行数：分隔线 + 一行可选行 + 底栏（题目 tab 另算）。
const MIN_PANEL_ROWS: u16 = 3;

/// 选项区一行。
#[derive(Clone, Copy)]
enum RowKind {
    /// 选项第一行：标记 + 标签。
    OptionLabel(usize),
    /// 选项描述的第 `line` 个折行（续行缩进到标签列）。
    OptionDesc { index: usize, line: usize },
    /// 「其他」行原地输入框的第 `line` 个视觉行。
    Other { line: usize },
    /// 提交按钮行（仅 `has_submit_row()` 时存在）。
    Submit,
}

/// 一条已落位的可视行（绘制与命中共用）。
struct VisualRow {
    kind: RowKind,
    hit: Option<AskHit>,
    rect: Rect,
}

/// 一次布局的全部矩形（绝对屏幕坐标）。
pub(super) struct AskLayout {
    /// 顶部可选中的题目 tab：`(题目下标, 矩形)`。
    tabs: Vec<(usize, Rect)>,
    question: Rect,
    /// 选项 / Other / 提交按钮列表区域（恒占满全宽）。
    options: Rect,
    /// 列表区可视行（已按滚动窗口裁剪）。
    rows: Vec<VisualRow>,
    /// 列表内容总行数（滚动条用）。
    content_rows: usize,
    /// 列表滚动起点（行）。
    scroll: usize,
    preview: Option<Rect>,
    footer: Rect,
}

impl AskLayout {
    /// 按面板外框与状态计算内部几何。
    fn compute(rect: Rect, ask: &AskState) -> Self {
        // 顶部一条分隔线占第一行，其余从下一行开始（与后台任务面板同款）。
        let inner = Rect::new(
            rect.x,
            rect.y.saturating_add(1),
            rect.width,
            rect.height.saturating_sub(1),
        );
        let mut lay = Self {
            tabs: Vec::new(),
            question: Rect::new(inner.x, inner.y, inner.width, 0),
            options: Rect::new(inner.x, inner.y, inner.width, 0),
            rows: Vec::new(),
            content_rows: 0,
            scroll: 0,
            preview: None,
            footer: Rect::new(inner.x, inner.bottom().saturating_sub(1), inner.width, 1),
        };
        if inner.width == 0 || inner.height < 2 {
            return lay;
        }
        let mut y = inner.y;
        // 题目 tab 只在多题时出现（没有 Confirm 汇总 Tab）；高度再小也保留这一行。
        if ask.tab_len() > 1 {
            lay.tabs = place_tabs(ask, inner.x, y, inner.width);
            y = y.saturating_add(1);
        }

        let footer_y = inner.bottom().saturating_sub(1);
        if y > footer_y {
            return lay;
        }
        // 题干让位规则：先保证列表至少留一行可操作。极矮窗格（扣掉分隔线、tab 与
        // 底栏后只剩 1–2 行）里如果题干硬占一行，列表就是 0 行——面板可见、按键
        // 被整块接管、却什么都选不了，等于一个吃键的死面板。
        let need = question_rows(ask, inner.width);
        let available = footer_y.saturating_sub(y);
        let question_h = if available >= 2 {
            need.min(available.saturating_sub(1))
        } else {
            0
        };
        lay.question = Rect::new(inner.x, y, inner.width, question_h);
        y = y.saturating_add(question_h);

        let body_h = footer_y.saturating_sub(y);
        if body_h == 0 {
            return lay;
        }
        place_body(&mut lay, ask, y, footer_y, body_h);
        place_rows(&mut lay, ask);
        lay
    }
}

/// 选项区与预览区的垂直切分：预览恒在选项（含 Other 与提交按钮）正下方，
/// 本题没有任何 preview 时完全不占高度。
fn place_body(lay: &mut AskLayout, ask: &AskState, y: u16, bottom: u16, body_h: u16) {
    let width = lay.question.width;
    let rows = u16::try_from(rows_model(ask, width).len()).unwrap_or(u16::MAX);
    // 选项 1 行 + 空行 + 预览 1 行是最小可行组合。
    let has_preview = ask.question_has_preview() && body_h >= PREVIEW_GAP + 2;
    if !has_preview {
        lay.options = Rect::new(lay.question.x, y, width, body_h);
        return;
    }
    let options_h = rows.min(body_h.saturating_sub(PREVIEW_GAP + 1)).max(1);
    lay.options = Rect::new(lay.question.x, y, width, options_h);
    let preview_y = y.saturating_add(options_h).saturating_add(PREVIEW_GAP);
    let preview_h = bottom.saturating_sub(preview_y).min(PREVIEW_MAX_ROWS);
    if preview_h > 0 {
        lay.preview = Some(Rect::new(lay.question.x, preview_y, width, preview_h));
    }
}

/// 面板高度：内容需要多少给多少，但**不超过窗格内容区的 70%**；超出部分靠列表滚动。
///
/// 唯一例外是「最小可用高度」：分隔线 + 至少一行可选行 + 底栏（多题再加 tab 行）。
/// 70% 上限的本意是不让面板挤掉历史；而在极矮窗格里死守 70% 会得到「可见、吃键、
/// 什么都选不了」的面板——那种情况下宁可多占一行。两者取 `max`，再夹到窗格高度内。
pub(super) fn height(area_h: u16, width: u16, lx: &Lx) -> u16 {
    let Some(ask) = lx.ask.as_ref() else {
        return 0;
    };
    let max_h = area_h.saturating_mul(HEIGHT_PERCENT) / 100;
    let tabs = u16::from(ask.tab_len() > 1);
    let rows = u16::try_from(rows_model(ask, width).len()).unwrap_or(u16::MAX);
    let preview = if ask.question_has_preview() {
        PREVIEW_GAP.saturating_add(PREVIEW_MAX_ROWS)
    } else {
        0
    };
    let need = 1u16
        .saturating_add(tabs)
        .saturating_add(question_rows(ask, width))
        .saturating_add(rows)
        .saturating_add(preview)
        .saturating_add(1);
    // 1 分隔线 + tabs + 1 行可选行 + 1 底栏。
    let viable = MIN_PANEL_ROWS.saturating_add(tabs);
    need.min(max_h).max(viable.min(area_h)).min(area_h).max(1)
}

/// 面板外框（输入框原位置）；未激活为 None。
pub(crate) fn rect(area: Rect, lx: &Lx) -> Option<Rect> {
    super::layout(area, lx).ask
}

/// 命中：选项行 / Other / Submit / 题目 tab。
pub(super) fn hit(area: Rect, lx: &Lx, column: u16, row: u16) -> Option<AskHit> {
    let ask = lx.ask.as_ref()?;
    let rect = rect(area, lx)?;
    let lay = AskLayout::compute(rect, ask);
    let point = (column, row).into();
    lay.rows
        .iter()
        .find_map(|line| line.hit.filter(|_| line.rect.contains(point)))
        .or_else(|| {
            lay.tabs
                .iter()
                .find_map(|(index, target)| target.contains(point).then_some(AskHit::Tab(*index)))
        })
}

/// 命中行内 `Other` 输入框单元格 `(visual_row, content_col)`；未编辑或未落在
/// 输入框正文列返回 None。
pub(crate) fn ask_editor_cell_at(
    area: Rect,
    lx: &Lx,
    column: u16,
    row: u16,
) -> Option<(usize, u16)> {
    let ask = lx.ask.as_ref()?;
    if !ask.editing {
        return None;
    }
    let rect = rect(area, lx)?;
    let lay = AskLayout::compute(rect, ask);
    let point = (column, row).into();
    let target = lay
        .rows
        .iter()
        .find(|line| matches!(line.kind, RowKind::Other { .. }) && line.rect.contains(point))?;
    let RowKind::Other { line } = target.kind else {
        return None;
    };
    let content_x = target.rect.x.saturating_add(ROW_PREFIX);
    if column < content_x {
        return None;
    }
    let (start, _) = other_window(ask);
    Some((start.saturating_add(line), column.saturating_sub(content_x)))
}

/// 选项列表的 `(窗口起点, 总行数, 可视行数)`；面板未激活或无列表区返回 None。
///
/// 与渲染共用 [`AskLayout::compute`] 的同一份几何，因此滚轮的基准与绘制永远一致。
pub(super) fn list_window(area: Rect, lx: &Lx) -> Option<(usize, usize, usize)> {
    let ask = lx.ask.as_ref()?;
    let rect = rect(area, lx)?;
    let lay = AskLayout::compute(rect, ask);
    Some((
        lay.scroll,
        lay.content_rows,
        usize::from(lay.options.height),
    ))
}

/// 预览区可见行数（滚动上限钳制用）；面板未激活或无预览区返回 None。
pub(super) fn preview_rows(area: Rect, lx: &Lx) -> Option<usize> {
    let ask = lx.ask.as_ref()?;
    let rect = rect(area, lx)?;
    let lay = AskLayout::compute(rect, ask);
    Some(usize::from(lay.preview?.height))
}

/// 滚轮分区：面板内返回区域，面板外 None（事件层据此吞掉）。
pub(super) fn wheel_zone(area: Rect, lx: &Lx, column: u16, row: u16) -> Option<AskWheelZone> {
    let ask = lx.ask.as_ref()?;
    let rect = rect(area, lx)?;
    if !rect.contains((column, row).into()) {
        return None;
    }
    let lay = AskLayout::compute(rect, ask);
    match lay.preview {
        Some(preview) if preview.contains((column, row).into()) => Some(AskWheelZone::Preview),
        _ => Some(AskWheelZone::Options),
    }
}

/// 绘制面板；返回聚焦且编辑态时的光标绝对坐标（行, 列）。
pub(super) fn draw(
    grid: &mut Grid,
    rect: Rect,
    _pane_height: u16,
    lx: &Lx,
    focused: bool,
) -> Option<(u16, u16)> {
    let ask = lx.ask.as_ref()?;
    let lay = AskLayout::compute(rect, ask);
    draw_rule(grid, rect);
    draw_tabs(grid, &lay, ask);
    draw_question(grid, &lay, ask);
    draw_rows(grid, &lay, ask);
    draw_scrollbar(grid, &lay);
    draw_preview(grid, &lay, ask);
    let cursor = cursor_cell(&lay, ask, focused);
    draw_footer(grid, &lay, lx);
    cursor
}

/// 顶部一条分隔线（与后台任务面板同款：淡青 + dim，画在内容区内）。
fn draw_rule(grid: &mut Grid, rect: Rect) {
    if rect.width == 0 {
        return;
    }
    grid.put_clipped(
        rect.x,
        rect.y,
        &"─".repeat(usize::from(rect.width)),
        style::border_title(),
        rect.width,
    );
}

/// 题目 tab：题目标题；当前题反显。
fn draw_tabs(grid: &mut Grid, lay: &AskLayout, ask: &AskState) {
    for (index, rect) in &lay.tabs {
        let active = *index == ask.current;
        let label = tab_label(ask, *index);
        grid.put_clipped(
            rect.x,
            rect.y,
            &label,
            if active {
                style::selected_item()
            } else {
                style::muted()
            },
            rect.width,
        );
    }
}

/// 题干（最多 3 行软折行）。
fn draw_question(grid: &mut Grid, lay: &AskLayout, ask: &AskState) {
    if lay.question.width == 0 || lay.question.height == 0 {
        return;
    }
    let Some(question) = ask.question() else {
        return;
    };
    let lines = wrap(&question.question, usize::from(lay.question.width));
    for (index, line) in lines
        .iter()
        .take(usize::from(lay.question.height))
        .enumerate()
    {
        grid.put_clipped(
            lay.question.x,
            lay.question.y.saturating_add(index as u16),
            line,
            style::text(),
            lay.question.width,
        );
    }
}

/// 列表：选项（标签 + 完整折行描述）/ 原地 Other / 提交按钮。
fn draw_rows(grid: &mut Grid, lay: &AskLayout, ask: &AskState) {
    for row in &lay.rows {
        match row.kind {
            RowKind::OptionLabel(index) => draw_option_label(grid, row.rect, ask, index),
            RowKind::OptionDesc { index, line } => {
                draw_option_desc(grid, row.rect, ask, index, line);
            }
            RowKind::Other { line } => draw_other(grid, row.rect, ask, line),
            RowKind::Submit => draw_submit(grid, row.rect, ask),
        }
    }
}

/// 选项标记：单选圆点 / 多选方框。
fn option_marker(multi: bool, selected: bool) -> &'static str {
    match (multi, selected) {
        (true, true) => a::ASK_MULTI_ON,
        (true, false) => a::ASK_MULTI_OFF,
        (false, true) => a::ASK_SINGLE_ON,
        (false, false) => a::ASK_SINGLE_OFF,
    }
}

fn draw_option_label(grid: &mut Grid, rect: Rect, ask: &AskState, index: usize) {
    let Some(option) = ask
        .question()
        .and_then(|question| question.options.get(index))
    else {
        return;
    };
    let selected = ask
        .selected
        .get(ask.current)
        .is_some_and(|picked| picked.contains(&index));
    let pointed = ask.highlight.get(ask.current).copied() == Some(index);
    if pointed {
        grid.put_clipped(rect.x, rect.y, a::ASK_CURSOR, style::accent(), 2);
    }
    let multi = ask.question().is_some_and(|question| question.multi_select);
    grid.put_clipped(
        rect.x.saturating_add(2),
        rect.y,
        option_marker(multi, selected),
        if selected || pointed {
            style::accent()
        } else {
            style::muted()
        },
        rect.width,
    );
    grid.put_clipped(
        rect.x.saturating_add(ROW_PREFIX),
        rect.y,
        &option.label,
        if pointed {
            style::strong()
        } else {
            style::text()
        },
        rect.width.saturating_sub(ROW_PREFIX),
    );
}

/// 选项描述：缩进 6 列后按全宽折行，逐行绘制（长描述绝不截断）。
fn draw_option_desc(grid: &mut Grid, rect: Rect, ask: &AskState, index: usize, line: usize) {
    let Some(description) = ask
        .question()
        .and_then(|question| question.options.get(index))
        .map(|option| option.description.trim())
    else {
        return;
    };
    let wrapped = wrap(description, desc_width(rect.width));
    let Some(text) = wrapped.get(line) else {
        return;
    };
    grid.put_clipped(
        rect.x.saturating_add(ROW_PREFIX),
        rect.y,
        text,
        style::muted(),
        rect.width.saturating_sub(ROW_PREFIX),
    );
}

/// 「其他」行：非编辑态一行占位/已写文本；编辑态原地展开 1~3 行软折行输入框。
fn draw_other(grid: &mut Grid, rect: Rect, ask: &AskState, line: usize) {
    let multi = ask.question().is_some_and(|question| question.multi_select);
    let text = other_text(ask);
    let custom_on = ask.custom_on.get(ask.current).copied().unwrap_or(false) && text.is_some();
    let pointed = ask.highlight.get(ask.current).copied() == Some(ask.other_row());
    if line == 0 {
        if pointed {
            grid.put_clipped(rect.x, rect.y, a::ASK_CURSOR, style::accent(), 2);
        }
        grid.put_clipped(
            rect.x.saturating_add(2),
            rect.y,
            option_marker(multi, custom_on),
            if custom_on {
                style::accent()
            } else {
                style::muted()
            },
            rect.width,
        );
    }
    let content_x = rect.x.saturating_add(ROW_PREFIX);
    let content_w = rect.width.saturating_sub(ROW_PREFIX);
    if content_w == 0 {
        return;
    }
    if ask.editing {
        let Some(editor) = ask.editor() else {
            return;
        };
        let text = editor.text();
        if text.is_empty() {
            if line == 0 {
                grid.put_clipped(
                    content_x,
                    rect.y,
                    a::ASK_OTHER_LABEL,
                    style::muted(),
                    content_w,
                );
            }
            return;
        }
        let rows = editor.visual_rows();
        let (start, _) = other_window(ask);
        let Some(row) = rows.get(start.saturating_add(line)) else {
            return;
        };
        let Some(segment) = text.get(row.start..row.end) else {
            return;
        };
        draw_segment(
            grid,
            content_x,
            rect.y,
            segment,
            row.start,
            editor.text_selection_range(),
            content_w,
        );
        return;
    }
    match text {
        Some(text) => {
            let wrapped = wrap(&text, desc_width(rect.width));
            if let Some(segment) = wrapped.get(line) {
                grid.put_clipped(content_x, rect.y, segment, style::muted(), content_w);
            }
        }
        None if line == 0 => {
            grid.put_clipped(
                content_x,
                rect.y,
                a::ASK_OTHER_LABEL,
                style::muted(),
                content_w,
            );
        }
        None => {}
    }
}

/// 提交按钮：弹窗同款实心胶囊；就绪与否决定配色，聚焦时行首带 `❯`。
fn draw_submit(grid: &mut Grid, rect: Rect, ask: &AskState) {
    let pointed = ask.submit_row() == ask.highlight.get(ask.current).copied();
    if pointed {
        grid.put_clipped(rect.x, rect.y, a::ASK_CURSOR, style::accent(), 2);
    }
    let button = if ask.all_answered() {
        style::button_primary()
    } else {
        style::button_secondary()
    };
    grid.put_clipped(
        rect.x.saturating_add(ROW_PREFIX),
        rect.y,
        a::ASK_SUBMIT_BUTTON,
        button,
        rect.width.saturating_sub(ROW_PREFIX),
    );
}

/// 行内编辑光标：`(行, 列)` 绝对坐标；未聚焦或未编辑返回 None。
fn cursor_cell(lay: &AskLayout, ask: &AskState, focused: bool) -> Option<(u16, u16)> {
    if !focused || !ask.editing {
        return None;
    }
    let target = lay
        .rows
        .iter()
        .find(|line| matches!(line.kind, RowKind::Other { line: 0 }))?;
    let editor = ask.editor()?;
    let (start, _) = other_window(ask);
    let (cursor_row, cursor_col) = editor.cursor_visual();
    let line = u16::try_from(cursor_row.saturating_sub(start)).unwrap_or(u16::MAX);
    let y = target.rect.y.saturating_add(line);
    let x = target
        .rect
        .x
        .saturating_add(ROW_PREFIX)
        .saturating_add(u16::try_from(cursor_col).unwrap_or(u16::MAX))
        .min(target.rect.right().saturating_sub(1));
    Some((y, x))
}

/// 按选区反显绘制一段文本（宽字符占两列，绝不写半个字符）。
fn draw_segment(
    grid: &mut Grid,
    x: u16,
    y: u16,
    segment: &str,
    absolute_start: usize,
    selection: Option<(usize, usize)>,
    width: u16,
) {
    let Some((sel_start, sel_end)) = selection else {
        grid.put_clipped(x, y, segment, style::text(), width);
        return;
    };
    let limit = x.saturating_add(width);
    let mut cx = x;
    for (offset, ch) in segment.char_indices() {
        let Some(cell) = UnicodeWidthChar::width(ch).filter(|width| *width > 0) else {
            continue;
        };
        let cell = u16::try_from(cell).unwrap_or(u16::MAX);
        if cx.saturating_add(cell) > limit {
            break;
        }
        let absolute = absolute_start.saturating_add(offset);
        let mut cell_style = style::text();
        if absolute >= sel_start && absolute < sel_end {
            cell_style = cell_style.add_modifier(Modifier::REVERSED);
        }
        grid.set(cx, y, ch, cell_style);
        for dx in 1..cell {
            grid.set(cx.saturating_add(dx), y, ' ', cell_style);
        }
        cx = cx.saturating_add(cell);
    }
}

/// 列表滚动条：内容超出可视区时贴列表最右一列画 thumb（只画 thumb，不画轨道）。
fn draw_scrollbar(grid: &mut Grid, lay: &AskLayout) {
    let visible = usize::from(lay.options.height);
    if visible == 0 || lay.content_rows <= visible || lay.options.width < 2 {
        return;
    }
    let track = Rect::new(
        lay.options.right().saturating_sub(1),
        lay.options.y,
        1,
        lay.options.height,
    );
    if let Some(bar) =
        crate::ui::widgets::scrollbar::layout(track, lay.content_rows, visible, lay.scroll)
    {
        for y in bar.thumb.y..bar.thumb.bottom() {
            grid.set(track.x, y, '▐', style::text());
        }
    }
}

/// 预览区：原样等宽渲染（保留前导空格、`\t`=4 列、宽字符占两列）。
///
/// 高亮移到 `Other` / 提交按钮时没有预览可看，此时渲染 `(no preview)` 占位：
/// 面板高度因此恒定，不会随高亮上下抖动。
fn draw_preview(grid: &mut Grid, lay: &AskLayout, ask: &AskState) {
    let Some(preview) = lay.preview else {
        return;
    };
    if preview.width == 0 || preview.height == 0 {
        return;
    }
    let Some(content) = ask.preview() else {
        grid.put_clipped(
            preview.x,
            preview.y,
            a::ASK_NO_PREVIEW,
            style::muted(),
            preview.width,
        );
        return;
    };
    let lines: Vec<String> = content.split('\n').map(expand_tabs).collect();
    let total = lines.len();
    let visible = usize::from(preview.height);
    let offset = ask
        .preview_scroll
        .get(ask.current)
        .copied()
        .unwrap_or(0)
        .min(total.saturating_sub(1));
    let overflow = total > visible;
    let width = preview.width.saturating_sub(u16::from(overflow));
    for (index, line) in lines.iter().skip(offset).take(visible).enumerate() {
        grid.put_clipped(
            preview.x,
            preview.y.saturating_add(index as u16),
            line,
            style::text(),
            width,
        );
    }
    if overflow
        && let Some(bar) = crate::ui::widgets::scrollbar::layout(preview, total, visible, offset)
    {
        for y in bar.thumb.y..bar.thumb.bottom() {
            grid.set(preview.right().saturating_sub(1), y, '▐', style::text());
        }
    }
}

/// 底栏：左侧键位提示，右侧 Esc 状态。
fn draw_footer(grid: &mut Grid, lay: &AskLayout, lx: &Lx) {
    let area = lay.footer;
    if area.width == 0 || area.height == 0 {
        return;
    }
    let right = if stop_armed(lx) {
        a::ASK_FOOTER_CANCEL_ARMED.to_string()
    } else {
        a::ASK_FOOTER_CANCEL.to_string()
    };
    let right_w = u16::try_from(right.width()).unwrap_or(u16::MAX);
    if right_w.saturating_add(2) <= area.width {
        grid.put_clipped(
            area.x,
            area.y,
            a::ASK_FOOTER_KEYS,
            style::muted(),
            area.width.saturating_sub(right_w).saturating_sub(1),
        );
        grid.put_clipped(
            area.right().saturating_sub(right_w),
            area.y,
            &right,
            style::muted(),
            right_w,
        );
    } else {
        grid.put_clipped(
            area.x,
            area.y,
            a::ASK_FOOTER_KEYS,
            style::muted(),
            area.width,
        );
    }
}

/// Esc 双击停止的待命窗口（与 `chat::escape_stop` 同一份判断）。
fn stop_armed(lx: &Lx) -> bool {
    lx.chat.stop_armed_at.is_some_and(|at| {
        std::time::Instant::now().saturating_duration_since(at)
            <= crate::app::update::STOP_ARM_WINDOW
    })
}

/// tab 文案：` N header `（只有真实题目，没有 Confirm）。
fn tab_label(ask: &AskState, index: usize) -> String {
    match ask.questions.get(index) {
        Some(question) => format!(" {} {} ", index + 1, question.header),
        None => String::new(),
    }
}

/// 题目 tab 落位；总宽超出时只画放得下的（命中同步）。
fn place_tabs(ask: &AskState, x: u16, y: u16, width: u16) -> Vec<(usize, Rect)> {
    let mut out = Vec::with_capacity(ask.tab_len());
    let mut cx = x;
    for index in 0..ask.tab_len() {
        let label = tab_label(ask, index);
        let w = u16::try_from(label.width()).unwrap_or(u16::MAX);
        if w > width.saturating_sub(cx.saturating_sub(x)) {
            break;
        }
        out.push((index, Rect::new(cx, y, w, 1)));
        cx = cx.saturating_add(w).saturating_add(1);
    }
    out
}

/// 列表模型：每个视觉行一条（绘制、滚动与命中共用同一份几何）。
fn rows_model(ask: &AskState, width: u16) -> Vec<RowKind> {
    let Some(question) = ask.question() else {
        return Vec::new();
    };
    let desc_w = desc_width(width);
    let mut out = Vec::new();
    for (index, option) in question.options.iter().enumerate() {
        out.push(RowKind::OptionLabel(index));
        if !option.description.trim().is_empty() {
            for line in 0..wrap(option.description.trim(), desc_w).len() {
                out.push(RowKind::OptionDesc { index, line });
            }
        }
    }
    for line in 0..other_rows(ask, desc_w) {
        out.push(RowKind::Other { line });
    }
    if ask.has_submit_row() {
        out.push(RowKind::Submit);
    }
    out
}

/// 描述 / 行内输入框正文的可用宽度（缩进 6 列后占满剩余宽度）。
fn desc_width(width: u16) -> usize {
    usize::from(width.saturating_sub(ROW_PREFIX)).max(1)
}

/// 「其他」行的视觉行数：编辑态按软折行视口，非编辑态按已写文本折行，空态 1 行。
fn other_rows(ask: &AskState, width: usize) -> usize {
    if ask.editing {
        return other_window(ask).1;
    }
    other_text(ask)
        .map(|text| {
            wrap(&text, width)
                .len()
                .clamp(1, AskState::MAX_OTHER_EDIT_ROWS)
        })
        .unwrap_or(1)
}

/// 「其他」行已写文本（去首尾空白；空串视为未写）。
fn other_text(ask: &AskState) -> Option<String> {
    let text = ask.editor()?.text();
    let trimmed = text.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_string())
}

/// 行内输入框的可见窗口：`(起始视觉行, 可见行数)`；光标始终落在窗口内。
fn other_window(ask: &AskState) -> (usize, usize) {
    let Some(editor) = ask.editor() else {
        return (0, 1);
    };
    let total = editor.visual_rows().len().max(1);
    let visible = total.clamp(1, AskState::MAX_OTHER_EDIT_ROWS);
    let (cursor_row, _) = editor.cursor_visual();
    let start = if cursor_row >= visible {
        cursor_row + 1 - visible
    } else {
        0
    };
    (start, visible)
}

/// 行种类 → 它所属的逻辑条目下标（`↑`/`↓` 的高亮单位）。
fn item_index(ask: &AskState, kind: RowKind) -> Option<usize> {
    match kind {
        RowKind::OptionLabel(index) | RowKind::OptionDesc { index, .. } => Some(index),
        RowKind::Other { .. } => Some(ask.other_row()),
        RowKind::Submit => ask.submit_row(),
    }
}

/// 可见行落位：按高亮自动滚动，保证高亮项完整可见。
fn place_rows(lay: &mut AskLayout, ask: &AskState) {
    let model = rows_model(ask, lay.options.width);
    let total = model.len();
    lay.content_rows = total;
    let visible = usize::from(lay.options.height);
    if visible == 0 || total == 0 {
        return;
    }
    // 高亮项的末行：滚动只需保证它在可视区底部（其余内容自然在上方）。
    let highlight = ask.highlight.get(ask.current).copied().unwrap_or(0);
    let mut end = 0usize;
    for (index, kind) in model.iter().enumerate() {
        if item_index(ask, *kind) == Some(highlight) {
            end = index + 1;
        }
    }
    // 显式视口（滚轮）优先：滚轮只滚列表，不移动高亮，因此窗口起点不能被高亮覆盖。
    // 与 `@`/`/` 面板的 `PanelWindow.viewport` 同一语义；键盘移动高亮会清空它。
    let scroll = match ask.viewport() {
        Some(start) => start.min(total.saturating_sub(visible)),
        None => {
            let mut follow = end.saturating_sub(visible);
            // 高亮停在「其他」时它的下一行就是 `Submit`：按高亮对齐的窗口恰好把
            // `Submit` 挤到可视区外（行下标 == visible 被丢弃），纯鼠标用户因此
            // 永远点不到提交按钮。
            if end + 1 == total && follow + visible < total {
                follow = follow.saturating_add(1);
            }
            follow
        }
    };
    lay.scroll = scroll;
    for (index, kind) in model.iter().enumerate() {
        if index < scroll {
            continue;
        }
        let line = index - scroll;
        if line >= visible {
            break;
        }
        let rect = Rect::new(
            lay.options.x,
            lay.options
                .y
                .saturating_add(u16::try_from(line).unwrap_or(u16::MAX)),
            lay.options.width,
            1,
        );
        let hit = match kind {
            RowKind::OptionLabel(i) | RowKind::OptionDesc { index: i, .. } => {
                Some(AskHit::Option(*i))
            }
            RowKind::Other { .. } => Some(AskHit::Other),
            RowKind::Submit => Some(AskHit::Submit),
        };
        lay.rows.push(VisualRow {
            kind: *kind,
            hit,
            rect,
        });
    }
}

/// 题干占用行数（最多 [`MAX_QUESTION_ROWS`] 行）。
fn question_rows(ask: &AskState, width: u16) -> u16 {
    let Some(question) = ask.question() else {
        return 1;
    };
    let rows = wrap(&question.question, usize::from(width.max(1))).len();
    u16::try_from(rows)
        .unwrap_or(MAX_QUESTION_ROWS)
        .clamp(1, MAX_QUESTION_ROWS)
}

/// `\t` 展开为 4 列（预览原样渲染，不做 Markdown）。
fn expand_tabs(line: &str) -> String {
    let mut out = String::with_capacity(line.len());
    for ch in line.chars() {
        if ch == '\t' {
            out.push_str("    ");
        } else {
            out.push(ch);
        }
    }
    out
}

/// 按显示列换行（宽字符按两列，保留原空格；`\n` 是硬换行）。
fn wrap(text: &str, width: usize) -> Vec<String> {
    let width = width.max(1);
    let mut out = Vec::new();
    for raw in text.split('\n') {
        let mut current = String::new();
        let mut col = 0usize;
        for ch in raw.chars() {
            let cell = UnicodeWidthChar::width(ch).unwrap_or(1).max(1);
            if col + cell > width && col > 0 {
                out.push(std::mem::take(&mut current));
                col = 0;
            }
            current.push(ch);
            col += cell;
        }
        out.push(current);
    }
    if out.is_empty() {
        out.push(String::new());
    }
    out
}

#[cfg(test)]
mod tests;
