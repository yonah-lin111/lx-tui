//! `ask` 面板：几何、绘制与命中（激活时**替换**输入框，design §8.1–§8.3）。
//!
//! 纯渲染：只读 `Lx`，只写 lx 页网格。几何是唯一来源：绘制、鼠标命中与滚轮分区
//! 全部走 [`AskLayout::compute`]，事件层禁止重算（design §7.3）。
//!
//! 版式对照后台任务面板：**只有顶部一条分隔线**，不画四周边框。
//!
//! ```text
//! ────────────────────────────────────────────────────────
//!  1 布局   2 主题   Confirm
//!  侧栏用哪种布局？
//! ❯ (•) 双栏                    │ ┌──┬──┐
//!       left list right preview  │ │a │b │
//!   ( ) 单栏                    │ └──┴──┘
//!       one column             │
//!   ( ) Other...
//!   [ Submit ]
//! enter select · space toggle · tab switch     esc cancel
//! ```
//!
//! 问题项**双行**：第一行是标记 + 选项，第二行是缩进的描述；标记始终在第一行左侧。
//! 内容超过「窗格高度 70%」时选项区内部滚动（右侧滚动条），顶部题目 tab 始终保留。

use ratatui::layout::Rect;
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use crate::app::state::{AskHit, AskState, AskWheelZone, Lx};
use crate::ui::style;
use crate::ui::text::agent as a;

use super::grid::Grid;

/// 面板高度上限占内容区高度的百分比（用户要求：最多 70%，不够就滚动）。
const HEIGHT_PERCENT: u16 = crate::layout::LX_ASK_HEIGHT_PERCENT;
/// 低于该高度不画预览（design §8.1）。
const SMALL_PANE_HEIGHT: u16 = 12;
/// 内容宽度达到该列才左右分栏。
const SPLIT_MIN_WIDTH: u16 = 60;
/// 分栏时选项列最小宽度。
const OPTIONS_MIN_COLS: u16 = 24;
/// 题干最多显示行数。
const MAX_QUESTION_ROWS: u16 = 3;
/// 编辑框最多内容行数。
const EDITOR_MAX_ROWS: u16 = crate::layout::LX_ASK_EDITOR_ROWS;
/// 分栏间隙列数。
const PREVIEW_GAP: u16 = 1;
/// 选项行前缀列数（高亮箭头 2 + 标记 3 + 空格 1）。
const ROW_PREFIX: u16 = 6;
/// 面板最小可用行数：分隔线 + 一行可选行 + 底栏（题目 tab 另算）。
const MIN_PANEL_ROWS: u16 = 3;

/// 选项区一行。
#[derive(Clone, Copy)]
enum RowKind {
    /// 选项第一行：标记 + 标签。
    OptionLabel(usize),
    /// 选项第二行：缩进的描述。
    OptionDesc(usize),
    /// 「其他」行（未输入 / 已输入）。
    Other,
    /// 单题提交行。
    Submit,
    /// Confirm 步：一题的作答摘要。
    Review(usize),
    /// Confirm 步的确认行。
    Confirm,
}

/// 一条已落位的可视行（绘制与命中共用）。
struct VisualRow {
    kind: RowKind,
    hit: Option<AskHit>,
    rect: Rect,
}

/// 一次布局的全部矩形（绝对屏幕坐标）。
pub(super) struct AskLayout {
    /// 顶部可选中的 tab：`(tab 下标, 矩形)`；下标 `questions.len()` 是 Confirm。
    tabs: Vec<(usize, Rect)>,
    question: Rect,
    /// 选项 / Confirm 列表区域。
    options: Rect,
    /// 列表区可视行（已按滚动窗口裁剪）。
    rows: Vec<VisualRow>,
    /// 列表内容总行数（滚动条用）。
    content_rows: usize,
    /// 列表滚动起点（行）。
    scroll: usize,
    preview: Option<Rect>,
    /// 编辑框（顶部一条分割线 + 内容行）；仅编辑态。
    editor: Option<Rect>,
    footer: Rect,
    /// 编辑框可视内容行数。
    editor_visible: usize,
}

impl AskLayout {
    /// 按面板外框与状态计算内部几何。
    fn compute(rect: Rect, pane_height: u16, ask: &AskState) -> Self {
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
            editor: None,
            footer: Rect::new(inner.x, inner.bottom().saturating_sub(1), inner.width, 1),
            editor_visible: 0,
        };
        if inner.width == 0 || inner.height < 2 {
            return lay;
        }
        let mut y = inner.y;
        // 题目 tab **始终显示**（多题时含 Confirm）；高度再小也保留这一行。
        if ask.tab_len() > 1 {
            lay.tabs = place_tabs(ask, inner.x, y, inner.width);
            y = y.saturating_add(1);
        }

        let footer_y = inner.bottom().saturating_sub(1);
        if y > footer_y {
            return lay;
        }
        // 编辑框贴底栏上方：顶部一条分割线 + 1–5 行内容。
        if ask.editing && ask.question().is_some() {
            let room = footer_y.saturating_sub(y);
            let content_rows = EDITOR_MAX_ROWS.min(room.saturating_sub(2)).max(1);
            let height = content_rows.saturating_add(1);
            let editor_y = footer_y.saturating_sub(height);
            lay.editor = Some(Rect::new(inner.x, editor_y, inner.width, height));
            lay.editor_visible = usize::from(content_rows);
        }
        let body_bottom = lay.editor.map_or(footer_y, |editor| editor.y);

        // 题干让位规则：先保证列表至少留一行可操作。极矮窗格（扣掉分隔线、tab 与
        // 底栏后只剩 1–2 行）里如果题干硬占一行，列表就是 0 行——面板可见、按键
        // 被整块接管、却什么都选不了，等于一个吃键的死面板。
        let need = question_rows(ask, inner.width);
        let available = body_bottom.saturating_sub(y);
        let question_h = if available >= 2 {
            need.min(available.saturating_sub(1))
        } else {
            0
        };
        lay.question = Rect::new(inner.x, y, inner.width, question_h);
        y = y.saturating_add(question_h);

        let body_h = body_bottom.saturating_sub(y);
        if body_h == 0 {
            return lay;
        }
        let small = pane_height < SMALL_PANE_HEIGHT;
        let show_preview = !small && !ask.is_confirm();
        if show_preview && inner.width >= SPLIT_MIN_WIDTH && body_h >= 3 {
            let options_w = (inner.width.saturating_mul(2) / 5)
                .max(OPTIONS_MIN_COLS)
                .min(
                    inner
                        .width
                        .saturating_sub(OPTIONS_MIN_COLS.saturating_add(PREVIEW_GAP)),
                )
                .max(1);
            lay.options = Rect::new(inner.x, y, options_w, body_h);
            lay.preview = Some(Rect::new(
                inner
                    .x
                    .saturating_add(options_w)
                    .saturating_add(PREVIEW_GAP),
                y,
                inner
                    .width
                    .saturating_sub(options_w)
                    .saturating_sub(PREVIEW_GAP),
                body_h,
            ));
        } else if show_preview && inner.height >= SMALL_PANE_HEIGHT && body_h >= 6 {
            let rows = u16::try_from(content_rows(ask)).unwrap_or(u16::MAX);
            let options_h = rows.min(body_h.saturating_sub(3)).max(1);
            lay.options = Rect::new(inner.x, y, inner.width, options_h);
            let preview_y = y.saturating_add(options_h).saturating_add(PREVIEW_GAP);
            let preview_h = body_bottom.saturating_sub(preview_y);
            if preview_h >= 2 {
                lay.preview = Some(Rect::new(inner.x, preview_y, inner.width, preview_h));
            }
        } else {
            lay.options = Rect::new(inner.x, y, inner.width, body_h);
        }
        place_rows(&mut lay, ask);
        lay
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
    let inner_w = width;
    let tabs = u16::from(ask.tab_len() > 1);
    let editor = if ask.editing && ask.question().is_some() {
        EDITOR_MAX_ROWS.saturating_add(1)
    } else {
        0
    };
    let rows = content_rows(ask);
    let body = if ask.is_confirm() || inner_w >= SPLIT_MIN_WIDTH {
        // 分栏时预览与选项并排，不增加高度。
        u16::try_from(rows).unwrap_or(u16::MAX)
    } else {
        u16::try_from(rows)
            .unwrap_or(u16::MAX)
            .saturating_add(if ask.preview().is_some() { 8 } else { 1 })
    };
    let need = 1u16
        .saturating_add(tabs)
        .saturating_add(question_rows(ask, inner_w))
        .saturating_add(body)
        .saturating_add(editor)
        .saturating_add(1);
    // 1 分隔线 + tabs + 1 行可选行 + 1 底栏。
    let viable = MIN_PANEL_ROWS.saturating_add(tabs);
    need.min(max_h).max(viable.min(area_h)).min(area_h).max(1)
}

/// 面板外框（输入框原位置）；未激活为 None。
pub(crate) fn rect(area: Rect, lx: &Lx) -> Option<Rect> {
    super::layout(area, lx).ask
}

/// 命中：选项行 / Other / Submit（含 Confirm）/ 题目 tab。
pub(super) fn hit(area: Rect, lx: &Lx, column: u16, row: u16) -> Option<AskHit> {
    let ask = lx.ask.as_ref()?;
    let rect = rect(area, lx)?;
    let lay = AskLayout::compute(rect, area.height, ask);
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

/// 命中手写输入框单元格 `(visual_row, content_col)`；未处于编辑态或未落在编辑区返回 None。
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
    let lay = AskLayout::compute(rect, area.height, ask);
    let editor_rect = lay.editor?;
    if column < editor_rect.x || column >= editor_rect.right() {
        return None;
    }
    let content_top = editor_rect.y.saturating_add(1);
    let content_bottom = content_top.saturating_add(lay.editor_visible as u16);
    if row < content_top || row >= content_bottom {
        return None;
    }
    let editor = ask.editor()?;
    let (cursor_row, _) = editor.cursor_visual();
    let visible = lay.editor_visible;
    let start = if visible > 0 && cursor_row >= visible {
        cursor_row + 1 - visible
    } else {
        0
    };
    let visual_row = start + usize::from(row.saturating_sub(content_top));
    let content_col = column.saturating_sub(editor_rect.x);
    Some((visual_row, content_col))
}

/// 选项列表的 `(窗口起点, 总行数, 可视行数)`；面板未激活或无列表区返回 None。
///
/// 与渲染共用 [`AskLayout::compute`] 的同一份几何，因此滚轮的基准与绘制永远一致。
pub(super) fn list_window(area: Rect, lx: &Lx) -> Option<(usize, usize, usize)> {
    let ask = lx.ask.as_ref()?;
    let rect = rect(area, lx)?;
    let lay = AskLayout::compute(rect, area.height, ask);
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
    let lay = AskLayout::compute(rect, area.height, ask);
    Some(usize::from(lay.preview?.height))
}

/// 滚轮分区：面板内返回区域，面板外 None（事件层据此吞掉）。
pub(super) fn wheel_zone(area: Rect, lx: &Lx, column: u16, row: u16) -> Option<AskWheelZone> {
    let ask = lx.ask.as_ref()?;
    let rect = rect(area, lx)?;
    if !rect.contains((column, row).into()) {
        return None;
    }
    let lay = AskLayout::compute(rect, area.height, ask);
    match lay.preview {
        Some(preview) if preview.contains((column, row).into()) => Some(AskWheelZone::Preview),
        _ => Some(AskWheelZone::Options),
    }
}

/// 绘制面板；返回聚焦且编辑态时的光标绝对坐标（行, 列）。
pub(super) fn draw(
    grid: &mut Grid,
    rect: Rect,
    pane_height: u16,
    lx: &Lx,
    focused: bool,
) -> Option<(u16, u16)> {
    let ask = lx.ask.as_ref()?;
    let lay = AskLayout::compute(rect, pane_height, ask);
    draw_rule(grid, rect);
    draw_tabs(grid, &lay, ask);
    draw_question(grid, &lay, ask);
    draw_rows(grid, &lay, ask);
    draw_scrollbar(grid, &lay);
    draw_preview(grid, &lay, ask);
    let cursor = draw_editor(grid, &lay, ask, focused);
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

/// 题目 tab：题目标题（+ Confirm）；当前步反显。
fn draw_tabs(grid: &mut Grid, lay: &AskLayout, ask: &AskState) {
    for (index, rect) in &lay.tabs {
        let active = if *index == ask.questions.len() {
            ask.is_confirm()
        } else {
            !ask.is_confirm() && *index == ask.current
        };
        let label = tab_label(ask, *index);
        let tab_style = if active {
            style::selected_item()
        } else {
            style::muted()
        };
        grid.put_clipped(rect.x, rect.y, &label, tab_style, rect.width);
    }
}

/// 题干；Confirm 步显示复核标题。
fn draw_question(grid: &mut Grid, lay: &AskLayout, ask: &AskState) {
    if lay.question.width == 0 || lay.question.height == 0 {
        return;
    }
    let text = match ask.question() {
        Some(question) if !ask.is_confirm() => question.question.clone(),
        _ => a::ASK_REVIEW.to_string(),
    };
    let lines = wrap(&text, usize::from(lay.question.width));
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

/// 列表：选项（双行）/ Other / Submit / Confirm 复核行。
fn draw_rows(grid: &mut Grid, lay: &AskLayout, ask: &AskState) {
    for line in &lay.rows {
        let y = line.rect.y;
        let rect = line.rect;
        match line.kind {
            RowKind::OptionLabel(index) | RowKind::OptionDesc(index) => {
                let Some(question) = ask.question() else {
                    continue;
                };
                let Some(option) = question.options.get(index) else {
                    continue;
                };
                let selected = ask
                    .selected
                    .get(ask.current)
                    .is_some_and(|picked| picked.contains(&index));
                let pointed = ask.highlight.get(ask.current).copied() == Some(index);
                if matches!(line.kind, RowKind::OptionLabel(_)) {
                    if pointed {
                        grid.put_clipped(rect.x, y, a::ASK_CURSOR, style::accent(), 2);
                    }
                    let marker = match (question.multi_select, selected) {
                        (true, true) => a::ASK_MULTI_ON,
                        (true, false) => a::ASK_MULTI_OFF,
                        (false, true) => a::ASK_SINGLE_ON,
                        (false, false) => a::ASK_SINGLE_OFF,
                    };
                    let marker_style = if selected || pointed {
                        style::accent()
                    } else {
                        style::muted()
                    };
                    grid.put_clipped(
                        rect.x.saturating_add(2),
                        y,
                        marker,
                        marker_style,
                        rect.width,
                    );
                    let label_style = if pointed {
                        style::strong()
                    } else {
                        style::text()
                    };
                    grid.put_clipped(
                        rect.x.saturating_add(ROW_PREFIX),
                        y,
                        &option.label,
                        label_style,
                        rect.width.saturating_sub(ROW_PREFIX),
                    );
                } else {
                    // 描述行缩进到标签列；选项未选中也显示（双行布局的核心信息）。
                    grid.put_clipped(
                        rect.x.saturating_add(ROW_PREFIX),
                        y,
                        &option.description,
                        if pointed {
                            style::text()
                        } else {
                            style::agent_step_detail()
                        },
                        rect.width.saturating_sub(ROW_PREFIX),
                    );
                }
            }
            RowKind::Other => {
                let text = ask
                    .custom
                    .get(ask.current)
                    .map(|editor| editor.text())
                    .unwrap_or_default();
                let text = text.trim();
                let custom_on = ask.custom_on.get(ask.current).copied().unwrap_or(false);
                let multi = ask.question().is_some_and(|question| question.multi_select);
                let pointed = ask
                    .question()
                    .is_some_and(|question| ask.highlight[ask.current] == question.options.len());
                if pointed {
                    grid.put_clipped(rect.x, y, a::ASK_CURSOR, style::accent(), 2);
                }
                let marker = match (multi, custom_on && !text.is_empty()) {
                    (true, true) => a::ASK_MULTI_ON,
                    (true, false) => a::ASK_MULTI_OFF,
                    (false, true) => a::ASK_SINGLE_ON,
                    (false, false) => a::ASK_SINGLE_OFF,
                };
                grid.put_clipped(
                    rect.x.saturating_add(2),
                    y,
                    marker,
                    if custom_on {
                        style::accent()
                    } else {
                        style::muted()
                    },
                    rect.width,
                );
                let label = if text.is_empty() {
                    a::ASK_OTHER.to_string()
                } else {
                    format!("{}{text}", a::ASK_OTHER_PREFIX)
                };
                grid.put_clipped(
                    rect.x.saturating_add(ROW_PREFIX),
                    y,
                    &label,
                    if pointed {
                        style::strong()
                    } else {
                        style::text()
                    },
                    rect.width.saturating_sub(ROW_PREFIX),
                );
            }
            RowKind::Submit => {
                grid.put_clipped(rect.x, y, a::ASK_SUBMIT, style::accent(), rect.width);
            }
            RowKind::Review(index) => {
                let Some(question) = ask.questions.get(index) else {
                    continue;
                };
                let answers = ask.answers_at(index);
                let summary = if answers.is_empty() {
                    a::ASK_NO_ANSWER.to_string()
                } else {
                    answers.join(", ")
                };
                let line = format!("{} · {summary}", question.header);
                let ok = !answers.is_empty();
                grid.put_clipped(
                    rect.x,
                    y,
                    &line,
                    if ok {
                        style::text()
                    } else {
                        style::agent_warn()
                    },
                    rect.width,
                );
            }
            RowKind::Confirm => {
                let ready = ask.all_answered();
                grid.put_clipped(
                    rect.x,
                    y,
                    a::ASK_CONFIRM,
                    if ready {
                        style::accent()
                    } else {
                        style::muted()
                    },
                    rect.width,
                );
            }
        }
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

/// 预览列：原样等宽渲染（保留前导空格、`\t`=4 列、宽字符占两列）。
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

/// 编辑框：顶部一条分割线 + 软换行文本 + 光标。
fn draw_editor(
    grid: &mut Grid,
    lay: &AskLayout,
    ask: &AskState,
    focused: bool,
) -> Option<(u16, u16)> {
    let editor_rect = lay.editor?;
    if editor_rect.height < 2 || editor_rect.width == 0 {
        return None;
    }
    draw_rule(grid, editor_rect);
    let editor = ask.editor()?;
    let text = editor.text();
    let rows = editor.visual_rows();
    let (cursor_row, cursor_col) = editor.cursor_visual();
    let visible = lay.editor_visible;
    let start = if visible > 0 && cursor_row >= visible {
        cursor_row + 1 - visible
    } else {
        0
    };
    let content_x = editor_rect.x;
    let content_w = editor.width().max(1).min(editor_rect.width.max(1));
    let selection = editor.text_selection_range();
    for (index, line) in rows.iter().skip(start).take(visible).enumerate() {
        let Some(segment) = text.get(line.start..line.end) else {
            break;
        };
        let y = editor_rect.y.saturating_add(1).saturating_add(index as u16);
        if let Some((sel_start, sel_end)) = selection {
            let mut cx = content_x;
            for (offset, ch) in segment.char_indices() {
                let abs_offset = line.start + offset;
                let ch_width =
                    u16::try_from(unicode_width::UnicodeWidthChar::width(ch).unwrap_or(0))
                        .unwrap_or(0);
                if ch_width == 0 {
                    continue;
                }
                if cx.saturating_add(ch_width) > content_x.saturating_add(content_w) {
                    break;
                }
                let mut cell_style = style::text();
                if abs_offset >= sel_start && abs_offset < sel_end {
                    cell_style = cell_style.add_modifier(ratatui::style::Modifier::REVERSED);
                }
                grid.set(cx, y, ch, cell_style);
                for dx in 1..ch_width {
                    grid.set(cx.saturating_add(dx), y, ' ', cell_style);
                }
                cx = cx.saturating_add(ch_width);
            }
        } else {
            grid.put_clipped(content_x, y, segment, style::text(), content_w);
        }
    }
    if !focused || visible == 0 {
        return None;
    }
    Some((
        editor_rect
            .y
            .saturating_add(1)
            .saturating_add(u16::try_from(cursor_row.saturating_sub(start)).unwrap_or(0)),
        content_x
            .saturating_add(u16::try_from(cursor_col).unwrap_or(0))
            .min(editor_rect.right().saturating_sub(1)),
    ))
}

/// 底栏：左侧键位提示（多选/单选、单题/多题各一套），右侧预览降级或 Esc 状态。
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

/// tab 文案：题目 ` N header `，最后一个是 ` Confirm `。
fn tab_label(ask: &AskState, index: usize) -> String {
    match ask.questions.get(index) {
        Some(question) => format!(" {} {} ", index + 1, question.header),
        None => format!(" {} ", a::ASK_CONFIRM_TAB),
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

/// 列表内容行数（选项各 1–2 行 + Other + Submit / 复核行）。
fn content_rows(ask: &AskState) -> usize {
    rows_model(ask).iter().map(|row| row.1).sum()
}

/// 列表模型：`(行种类, 占用行数)`；绘制、滚动与命中共用。
fn rows_model(ask: &AskState) -> Vec<(RowKind, usize)> {
    let mut out = Vec::new();
    if ask.is_confirm() {
        for index in 0..ask.questions.len() {
            out.push((RowKind::Review(index), 1));
        }
        out.push((RowKind::Confirm, 1));
        return out;
    }
    let Some(question) = ask.question() else {
        return out;
    };
    for (index, option) in question.options.iter().enumerate() {
        out.push((RowKind::OptionLabel(index), 1));
        if !option.description.trim().is_empty() {
            out.push((RowKind::OptionDesc(index), 1));
        }
    }
    out.push((RowKind::Other, 1));
    // 单题才有独立 Submit；多题的提交入口是 Confirm（避免两个提交点）。
    if ask.questions.len() == 1 {
        out.push((RowKind::Submit, 1));
    }
    out
}

/// 可见行落位：按高亮自动滚动，保证高亮项完整可见。
fn place_rows(lay: &mut AskLayout, ask: &AskState) {
    let model = rows_model(ask);
    let total: usize = model.iter().map(|row| row.1).sum();
    lay.content_rows = total;
    let visible = usize::from(lay.options.height);
    if visible == 0 || total == 0 {
        return;
    }
    // 高亮项的末行：滚动只需保证它在可视区底部（其余内容自然在上方）。
    let highlight_end = if ask.is_confirm() {
        total
    } else {
        let highlight = ask.highlight.get(ask.current).copied().unwrap_or(0);
        let mut row = 0usize;
        let mut end = 0usize;
        for (kind, height) in &model {
            row += height;
            let item = match kind {
                RowKind::OptionLabel(index) | RowKind::OptionDesc(index) => Some(*index),
                RowKind::Other => ask.question().map(|question| question.options.len()),
                _ => None,
            };
            if item == Some(highlight) {
                end = row;
            }
        }
        end
    };
    // 显式视口（滚轮）优先：滚轮只滚列表，不移动高亮，因此窗口起点不能被高亮覆盖。
    // 与 `@`/`/` 面板的 `PanelWindow.viewport` 同一语义；键盘移动高亮会清空它。
    let scroll = match ask.viewport() {
        Some(start) => start.min(total.saturating_sub(visible)),
        None => {
            let mut follow = highlight_end.saturating_sub(visible);
            // 高亮停在「其他」时它的下一行就是 `Submit`：按高亮对齐的窗口恰好把
            // `Submit` 挤到可视区外（行下标 == visible 被丢弃），纯鼠标用户因此
            // 永远点不到提交行。
            if highlight_end + 1 == total && follow + visible < total {
                follow = follow.saturating_add(1);
            }
            follow
        }
    };
    lay.scroll = scroll;
    let mut row = 0usize;
    for (kind, height) in model {
        for offset in 0..height {
            let absolute = row + offset;
            if absolute < scroll {
                continue;
            }
            let index = absolute - scroll;
            if index >= visible {
                break;
            }
            let rect = Rect::new(
                lay.options.x,
                lay.options
                    .y
                    .saturating_add(u16::try_from(index).unwrap_or(0)),
                lay.options.width,
                1,
            );
            let hit = match kind {
                RowKind::OptionLabel(i) | RowKind::OptionDesc(i) => Some(AskHit::Option(i)),
                RowKind::Other => Some(AskHit::Other),
                RowKind::Submit | RowKind::Confirm => Some(AskHit::Submit),
                RowKind::Review(_) => None,
            };
            lay.rows.push(VisualRow { kind, hit, rect });
        }
        row += height;
    }
}

/// 题干占用行数：Confirm 步固定 1 行，其余最多 3 行。
fn question_rows(ask: &AskState, width: u16) -> u16 {
    if ask.is_confirm() {
        return 1;
    }
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

/// 按显示列换行（宽字符按两列，保留原空格）。
fn wrap(text: &str, width: usize) -> Vec<String> {
    let width = width.max(1);
    let mut out = Vec::new();
    let mut current = String::new();
    let mut col = 0usize;
    for ch in text.chars() {
        let cell = ch.width().unwrap_or(1).max(1);
        if col + cell > width && col > 0 {
            out.push(std::mem::take(&mut current));
            col = 0;
        }
        current.push(ch);
        col += cell;
    }
    if !current.is_empty() || out.is_empty() {
        out.push(current);
    }
    out
}

#[cfg(test)]
mod tests;
