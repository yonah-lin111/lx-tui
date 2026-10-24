//! `ask` 面板：几何、绘制与命中（激活时**替换**输入框，design §8.1–§8.3）。
//!
//! 纯渲染：只读 `Lx`，只写 lx 页网格。几何是唯一来源：绘制、鼠标命中与滚轮分区
//! 全部走 [`AskLayout::compute`]，事件层禁止重算（design §7.3）。
//!
//! 版式：
//!
//! ```text
//! ╭ ASK 1/2 · 布局 ───────────────────────────────╮
//! │ 1 布局   2 主题                                │  ← 多题时的标签行
//! │ 侧栏用哪种布局？                                │
//! │ ❯ (•) 双栏  left list right preview  │ ┌──┬──┐ │
//! │   ( ) 单栏                           │ │a │b │ │
//! │   ( ) 上下堆叠                        │ └──┴──┘ │
//! │   ( ) Other...                       │         │
//! │   [ Submit ]                         │         │
//! │ up/down move · space select · ...     esc cancel│
//! ╰────────────────────────────────────────────────╯
//! ```

use ratatui::layout::Rect;
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use crate::app::state::{AskHit, AskState, AskWheelZone, Lx};
use crate::ui::style;
use crate::ui::text::agent as a;

use super::grid::Grid;

/// 面板高度上限占内容区高度的百分比（与输入框同一档取舍）。
const HEIGHT_PERCENT: u16 = crate::layout::LX_ASK_HEIGHT_PERCENT;
/// 面板最小高度（含上下边框）。
const MIN_HEIGHT: u16 = crate::layout::LX_ASK_MIN_BOX;
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
/// 边框与左右内边距列数（面板内容相对外框的缩进）。
const INNER_INSET: u16 = 2;

/// 一次布局的全部矩形（绝对屏幕坐标）。
pub(super) struct AskLayout {
    title: Rect,
    /// 多题时的可点击标签：`(题号, 矩形)`。
    tabs: Vec<(usize, Rect)>,
    question: Rect,
    /// 选项列区域。
    options: Rect,
    /// 选项区可点击行：选项下标 / `Other` / `Submit`。
    option_rows: Vec<(AskHit, Rect)>,
    preview: Option<Rect>,
    /// 编辑框（含上下分割线）；仅编辑态。
    editor: Option<Rect>,
    footer: Rect,
    /// 编辑框可视内容行数（不含上下分割线）。
    editor_visible: usize,
    /// 窗格太矮，预览被隐藏。
    preview_hidden: bool,
}

impl AskLayout {
    /// 按面板外框与状态计算内部几何。
    fn compute(rect: Rect, ask: &AskState) -> Self {
        let inner = Rect::new(
            rect.x.saturating_add(INNER_INSET),
            rect.y.saturating_add(1),
            rect.width.saturating_sub(INNER_INSET.saturating_mul(2)),
            rect.height.saturating_sub(2),
        );
        let mut lay = Self {
            title: inner,
            tabs: Vec::new(),
            question: Rect::new(inner.x, inner.y, inner.width, 0),
            options: Rect::new(inner.x, inner.y, inner.width, 0),
            option_rows: Vec::new(),
            preview: None,
            editor: None,
            footer: Rect::new(inner.x, inner.bottom().saturating_sub(1), inner.width, 1),
            editor_visible: 0,
            preview_hidden: false,
        };
        if inner.width == 0 || inner.height < 3 {
            return lay;
        }
        let mut y = inner.y;
        lay.title = Rect::new(inner.x, y, inner.width, 1);
        y = y.saturating_add(1);

        // 多题标签行：题数 ≥2 且高度够时才画（单题标题已说明一切）。
        if ask.questions.len() > 1 && inner.height >= 8 {
            lay.tabs = place_tabs(ask, inner.x, y, inner.width);
            y = y.saturating_add(1);
        }

        let footer_y = inner.bottom().saturating_sub(1);
        // 编辑框贴底栏上方；高度按内容行（1–5）+ 上下分割线。
        if ask.editing {
            let room = footer_y.saturating_sub(y);
            let content_rows = EDITOR_MAX_ROWS.min(room.saturating_sub(3)).max(1);
            let height = content_rows.saturating_add(2);
            let editor_y = footer_y.saturating_sub(height);
            lay.editor = Some(Rect::new(inner.x, editor_y, inner.width, height));
            lay.editor_visible = usize::from(content_rows);
        }
        let body_bottom = lay.editor.map_or(footer_y, |editor| editor.y);

        let need = question_rows(ask, inner.width);
        let available = body_bottom.saturating_sub(y);
        let question_h = need
            .min(available.saturating_sub(2))
            .max(u16::from(available > 0));
        lay.question = Rect::new(inner.x, y, inner.width, question_h);
        y = y.saturating_add(question_h);

        let body_h = body_bottom.saturating_sub(y);
        if body_h == 0 {
            return lay;
        }
        let row_count = row_count(ask);
        // 窗格太矮时只画题干 + 选项 + 底栏，预览整块让位（design §8.1）。
        let small = rect.height < SMALL_PANE_HEIGHT;
        if !small && inner.width >= SPLIT_MIN_WIDTH && body_h >= 3 {
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
        } else if !small && inner.height >= SMALL_PANE_HEIGHT && body_h >= 6 {
            // 窄窗格：上下排；选项优先，预览拿剩余（至少 2 行才有意义）。
            let options_h = u16::try_from(row_count)
                .unwrap_or(u16::MAX)
                .min(body_h.saturating_sub(3))
                .max(1);
            lay.options = Rect::new(inner.x, y, inner.width, options_h);
            let preview_y = y.saturating_add(options_h).saturating_add(PREVIEW_GAP);
            let preview_h = body_bottom.saturating_sub(preview_y);
            if preview_h >= 2 {
                lay.preview = Some(Rect::new(inner.x, preview_y, inner.width, preview_h));
            }
        } else {
            lay.options = Rect::new(inner.x, y, inner.width, body_h);
        }
        lay.preview_hidden = small || lay.preview.is_none();
        lay.option_rows = place_option_rows(ask, lay.options, row_count);
        lay
    }
}

/// 面板高度：`need` 夹在「最小 10 行」与「70% 内容区」之间（design §8.1）。
pub(super) fn height(area_h: u16, width: u16, lx: &Lx) -> u16 {
    let Some(ask) = lx.ask.as_ref() else {
        return 0;
    };
    let max_h = area_h.saturating_mul(HEIGHT_PERCENT) / 100;
    let max_h = max_h.max(MIN_HEIGHT).min(area_h);
    let min_h = MIN_HEIGHT.min(area_h);
    let inner_w = width.saturating_sub(INNER_INSET.saturating_mul(2));
    let rows = u16::try_from(row_count(ask)).unwrap_or(u16::MAX);
    let tabs = u16::from(ask.questions.len() > 1);
    let preview = if ask.preview().is_some() { 8 } else { 1 };
    let body = if inner_w >= SPLIT_MIN_WIDTH {
        rows.saturating_add(1).max(preview)
    } else {
        rows.saturating_add(1).saturating_add(preview)
    };
    let editor = if ask.editing {
        EDITOR_MAX_ROWS.saturating_add(2)
    } else {
        0
    };
    let need = 2u16
        .saturating_add(1)
        .saturating_add(tabs)
        .saturating_add(question_rows(ask, inner_w))
        .saturating_add(body)
        .saturating_add(editor)
        .saturating_add(1);
    need.clamp(min_h, max_h.max(min_h))
}

/// 面板外框（输入框原位置）；未激活为 None。
pub(crate) fn rect(area: Rect, lx: &Lx) -> Option<Rect> {
    super::layout(area, lx).ask
}

/// 命中：选项行 / Other / Submit / 题目标签。
pub(super) fn hit(area: Rect, lx: &Lx, column: u16, row: u16) -> Option<AskHit> {
    let ask = lx.ask.as_ref()?;
    let rect = rect(area, lx)?;
    let lay = AskLayout::compute(rect, ask);
    let point = (column, row).into();
    lay.option_rows
        .iter()
        .find_map(|(hit, target)| target.contains(point).then_some(*hit))
        .or_else(|| {
            lay.tabs
                .iter()
                .find_map(|(index, target)| target.contains(point).then_some(AskHit::Tab(*index)))
        })
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

/// 绘制面板；返回聚焦且编辑态时的光标绝对坐标。
pub(super) fn draw(grid: &mut Grid, rect: Rect, lx: &Lx, focused: bool) -> Option<(u16, u16)> {
    let ask = lx.ask.as_ref()?;
    let lay = AskLayout::compute(rect, ask);
    super::draw_box(grid, rect, style::border(focused));
    draw_title(grid, &lay, ask);
    draw_tabs(grid, &lay, ask);
    draw_question(grid, &lay, ask);
    draw_options(grid, &lay, ask);
    draw_preview(grid, &lay, ask);
    let cursor = draw_editor(grid, &lay, ask, focused);
    draw_footer(grid, &lay, lx, ask);
    cursor
}

/// 标题行：`ASK 1/2 · 布局`。
fn draw_title(grid: &mut Grid, lay: &AskLayout, ask: &AskState) {
    let area = lay.title;
    if area.width == 0 || area.height == 0 {
        return;
    }
    let mut x = area.x;
    x = put(grid, x, area.y, a::ASK_TITLE, style::accent(), area);
    let position = a::ask_position(ask.current, ask.questions.len());
    x = put(grid, x, area.y, " ", style::muted(), area);
    x = put(grid, x, area.y, &position, style::muted(), area);
    if let Some(question) = ask.question() {
        x = put(grid, x, area.y, a::ASK_TITLE_SEP, style::muted(), area);
        put(grid, x, area.y, &question.header, style::strong(), area);
    }
}

/// 多题标签：` 1 布局  2 主题 `，当前题反显。
fn draw_tabs(grid: &mut Grid, lay: &AskLayout, ask: &AskState) {
    for (index, rect) in &lay.tabs {
        let Some(question) = ask.questions.get(*index) else {
            continue;
        };
        let label = format!(" {} {} ", index + 1, question.header);
        let tab_style = if *index == ask.current {
            style::selected_item()
        } else {
            style::muted()
        };
        grid.put_clipped(rect.x, rect.y, &label, tab_style, rect.width);
    }
}

/// 题干：最多 3 行，超出截断。
fn draw_question(grid: &mut Grid, lay: &AskLayout, ask: &AskState) {
    let Some(question) = ask.question() else {
        return;
    };
    if lay.question.width == 0 || lay.question.height == 0 {
        return;
    }
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

/// 选项列：标记 + 标签（+ 高亮项的说明）、Other 行、Submit 行。
fn draw_options(grid: &mut Grid, lay: &AskLayout, ask: &AskState) {
    let Some(question) = ask.question() else {
        return;
    };
    let multi = question.multi_select;
    let selected = ask
        .selected
        .get(ask.current)
        .map(Vec::as_slice)
        .unwrap_or(&[]);
    let highlight = ask.highlight.get(ask.current).copied().unwrap_or(0);
    let other = question.options.len();
    for (hit, rect) in &lay.option_rows {
        let y = rect.y;
        let pointed = match hit {
            AskHit::Option(index) => *index == highlight,
            AskHit::Other => highlight == other,
            _ => false,
        };
        if pointed {
            grid.put_clipped(rect.x, y, a::ASK_CURSOR, style::accent(), 2);
        }
        match hit {
            AskHit::Option(index) => {
                let Some(option) = question.options.get(*index) else {
                    continue;
                };
                let marker = match (multi, selected.contains(index)) {
                    (true, true) => a::ASK_MULTI_ON,
                    (true, false) => a::ASK_MULTI_OFF,
                    (false, true) => a::ASK_SINGLE_ON,
                    (false, false) => a::ASK_SINGLE_OFF,
                };
                let marker_style = if selected.contains(index) || pointed {
                    style::accent()
                } else {
                    style::muted()
                };
                put(grid, rect.x + 2, y, marker, marker_style, *rect);
                let label_x = rect.x.saturating_add(ROW_PREFIX);
                let label_style = if pointed {
                    style::strong()
                } else {
                    style::text()
                };
                let x = put(grid, label_x, y, &option.label, label_style, *rect);
                if pointed && !option.description.is_empty() {
                    put(
                        grid,
                        x.saturating_add(2),
                        y,
                        &option.description,
                        style::agent_step_detail(),
                        *rect,
                    );
                }
            }
            AskHit::Other => {
                let text = ask
                    .custom
                    .get(ask.current)
                    .map(|editor| editor.text())
                    .unwrap_or_default();
                let text = text.trim();
                let custom_on = ask.custom_on.get(ask.current).copied().unwrap_or(false);
                let marker = match (multi, custom_on && !text.is_empty()) {
                    (true, true) => a::ASK_MULTI_ON,
                    (true, false) => a::ASK_MULTI_OFF,
                    (false, true) => a::ASK_SINGLE_ON,
                    (false, false) => a::ASK_SINGLE_OFF,
                };
                if text.is_empty() {
                    put(grid, rect.x + 2, y, marker, style::muted(), *rect);
                    put(
                        grid,
                        rect.x + ROW_PREFIX,
                        y,
                        a::ASK_OTHER,
                        if pointed {
                            style::strong()
                        } else {
                            style::text()
                        },
                        *rect,
                    );
                } else {
                    let marker_style = if custom_on {
                        style::accent()
                    } else {
                        style::muted()
                    };
                    put(grid, rect.x + 2, y, marker, marker_style, *rect);
                    let label = format!("{}{text}", a::ASK_OTHER_PREFIX);
                    put(
                        grid,
                        rect.x + ROW_PREFIX,
                        y,
                        &label,
                        if pointed {
                            style::strong()
                        } else {
                            style::text()
                        },
                        *rect,
                    );
                }
            }
            AskHit::Submit => {
                put(grid, rect.x + 2, y, a::ASK_SUBMIT, style::accent(), *rect);
            }
            AskHit::Tab(_) => {}
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
    // 溢出时最右列让给滚动条 thumb。
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
    if overflow {
        let bar = crate::ui::widgets::scrollbar::layout(preview, total, visible, offset);
        if let Some(bar) = bar {
            for y in bar.thumb.y..bar.thumb.bottom() {
                grid.set(preview.right().saturating_sub(1), y, '▐', style::text());
            }
        }
    }
}

/// 编辑框：上下分割线 + 软换行文本 + 光标。
fn draw_editor(
    grid: &mut Grid,
    lay: &AskLayout,
    ask: &AskState,
    focused: bool,
) -> Option<(u16, u16)> {
    let editor_rect = lay.editor?;
    if editor_rect.height < 2 || editor_rect.width < 2 {
        return None;
    }
    for x in editor_rect.x..editor_rect.right() {
        grid.set(x, editor_rect.y, '─', style::muted());
        grid.set(
            x,
            editor_rect.bottom().saturating_sub(1),
            '─',
            style::muted(),
        );
    }
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
    let content_x = editor_rect.x.saturating_add(1);
    // 与缓冲区自身的软换行宽度同源（窗格 resize 后由 `resize_panes` 精确同步）。
    let content_w = editor
        .width()
        .max(1)
        .min(editor_rect.width.saturating_sub(2).max(1));
    for (index, line) in rows.iter().skip(start).take(visible).enumerate() {
        let Some(segment) = text.get(line.start..line.end) else {
            break;
        };
        grid.put_clipped(
            content_x,
            editor_rect.y.saturating_add(1).saturating_add(index as u16),
            segment,
            style::text(),
            content_w,
        );
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

/// 底栏：左内联提示 / 键位，右预览降级或 Esc 状态。
fn draw_footer(grid: &mut Grid, lay: &AskLayout, lx: &Lx, ask: &AskState) {
    let area = lay.footer;
    if area.width == 0 || area.height == 0 {
        return;
    }
    let right = if lay.preview_hidden {
        a::ASK_PREVIEW_HIDDEN.to_string()
    } else if stop_armed(lx) {
        a::ASK_FOOTER_CANCEL_ARMED.to_string()
    } else {
        a::ASK_FOOTER_CANCEL.to_string()
    };
    let right_w = u16::try_from(right.width()).unwrap_or(u16::MAX);
    let left = ask.hint.as_deref().unwrap_or(a::ASK_FOOTER_KEYS);
    let left_style = if ask.hint.is_some() {
        style::agent_warn()
    } else {
        style::muted()
    };
    if right_w.saturating_add(2) <= area.width {
        let left_w = area.width.saturating_sub(right_w).saturating_sub(1);
        grid.put_clipped(area.x, area.y, left, left_style, left_w);
        grid.put_clipped(
            area.right().saturating_sub(right_w),
            area.y,
            &right,
            style::muted(),
            right_w,
        );
    } else {
        grid.put_clipped(area.x, area.y, left, left_style, area.width);
    }
}

/// Esc 双击停止的待命窗口（与 `chat::escape_stop` 同一份判断）。
fn stop_armed(lx: &Lx) -> bool {
    lx.chat.stop_armed_at.is_some_and(|at| {
        std::time::Instant::now().saturating_duration_since(at)
            <= crate::app::update::STOP_ARM_WINDOW
    })
}

/// 题目标签落位；标签总宽超出时只画放得下的（命中同步）。
fn place_tabs(ask: &AskState, x: u16, y: u16, width: u16) -> Vec<(usize, Rect)> {
    let mut out = Vec::with_capacity(ask.questions.len());
    let mut cx = x;
    for (index, question) in ask.questions.iter().enumerate() {
        let label = format!(" {} {} ", index + 1, question.header);
        let w = u16::try_from(label.width()).unwrap_or(u16::MAX);
        if w > width.saturating_sub(cx.saturating_sub(x)) {
            break;
        }
        out.push((index, Rect::new(cx, y, w, 1)));
        cx = cx.saturating_add(w).saturating_add(1);
    }
    out
}

/// 选项区可点击行（窗口随高亮滚动，避免高亮跑出可视区）。
fn place_option_rows(ask: &AskState, options: Rect, row_count: usize) -> Vec<(AskHit, Rect)> {
    if options.width == 0 || options.height == 0 || row_count == 0 {
        return Vec::new();
    }
    let visible = usize::from(options.height);
    let highlight = ask.highlight.get(ask.current).copied().unwrap_or(0);
    let start = if highlight >= visible {
        highlight + 1 - visible
    } else {
        0
    };
    let mut rows = Vec::with_capacity(visible);
    for index in start..row_count.min(start.saturating_add(visible)) {
        let hit = if index < ask.question().map_or(0, |q| q.options.len()) {
            AskHit::Option(index)
        } else if index + 1 == row_count {
            AskHit::Submit
        } else {
            AskHit::Other
        };
        rows.push((
            hit,
            Rect::new(
                options.x,
                options
                    .y
                    .saturating_add(u16::try_from(index - start).unwrap_or(0)),
                options.width,
                1,
            ),
        ));
    }
    rows
}

/// 选项行数：选项 + 固定「其他」+ 提交行。
fn row_count(ask: &AskState) -> usize {
    ask.row_count().saturating_add(1)
}

/// 题干占用行数（1–3）。
fn question_rows(ask: &AskState, width: u16) -> u16 {
    let Some(question) = ask.question() else {
        return 1;
    };
    let rows = wrap(&question.question, usize::from(width.max(1))).len();
    u16::try_from(rows)
        .unwrap_or(MAX_QUESTION_ROWS)
        .clamp(1, MAX_QUESTION_ROWS)
}

/// 写入一段文本并返回下一列；整体不越过 `bounds`。
fn put(
    grid: &mut Grid,
    x: u16,
    y: u16,
    content: &str,
    style: ratatui::style::Style,
    bounds: Rect,
) -> u16 {
    let right = bounds.right();
    if x >= right {
        return x;
    }
    let width = right.saturating_sub(x);
    grid.put_clipped(x, y, content, style, width);
    x.saturating_add(u16::try_from(content.width()).unwrap_or(u16::MAX))
        .min(right)
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
