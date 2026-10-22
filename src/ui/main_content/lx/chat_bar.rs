//! 输入框上方那一行：loading（含 token 计数）/ 「回到底部」控件。
//!
//! 纯渲染 + 纯几何：布局、绘制与命中共享同一矩形口径；只读 `Lx`、只写 lx 页网格。
//! 该行贴在输入框上沿、与输入框同宽；没有内容时整行不占位。
//!
//! **优先级：loading > 回到底部**（`↓ jump to bottom` 在被挤出时让位——用户此刻
//! 视线在输入框）。提示类反馈已全部改走 main content 的 toast（见
//! `app::update::chat::show_lx_hint`），这一行不再承载临时提示。
//!
//! **loading**：`Working... (3s · ↓320 tok)`（只显示输出方向；耗时最小单位是秒）。`Working...` 带**从左到右的
//! 扫描高亮**：窗口外的字符用次要色（dim），3 字符宽的窗口内用当前帧色（按 Native
//! 标签的 4 档渐变 `218 → 182 → 153 → 117` 循环，每 `LOADING_FRAME_INTERVAL` 换一档），
//! 走出末尾后从左侧重新进入。括号里的统计**紧贴文案右侧**（用户要求「在 working 的
//! 组右侧」，不是贴栏右缘），静态 dim、不参与扫描；宽度不够时整段先丢。

use ratatui::layout::Rect;
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use crate::app::state::Lx;
use crate::ui::{style, text};

use super::Grid;

/// 聊天栏高度（一行）。
pub(crate) const HEIGHT: u16 = 1;
/// 右侧控件与栏右缘之间的空档（列）。
const RIGHT_PAD: u16 = 1;
/// 箭头与说明文字之间的空档（列）。
const HINT_GAP: u16 = 1;
/// 扫描高亮窗口宽度（字符）。
const SCAN_WINDOW: usize = 3;
/// loading 文案与括号统计之间的空档（列）。
const STATS_GAP: u16 = 1;

/// 是否显示聊天栏：有临时提示、流中（loading）或上滚未跟随底部（回到底部控件）。
///
/// **后台任务面板打开时不显示**：这一行属于聊天历史区（loading / 回到底部都是
/// 针对消息列表的），而面板已经把历史区换成输出容器——此时漂一个
/// `↓ jump to bottom` 出来既是错位信息，点了也只会滚动看不见的历史。
pub(super) fn visible(lx: &Lx) -> bool {
    if lx.task.is_some() {
        return false;
    }
    if !super::history::has_history(lx) {
        return false;
    }
    lx.chat.status.streaming || !lx.chat.follow_bottom
}

/// 由输入框矩形推导聊天栏矩形（贴其上沿）；上方空间不足时 None。
pub(super) fn rect(area: Rect, input: Rect) -> Option<Rect> {
    (input.y >= area.y.saturating_add(HEIGHT))
        .then(|| Rect::new(input.x, input.y - HEIGHT, input.width, HEIGHT))
}

/// 回到底部控件矩形（未跟随底部、**无临时提示**且空间足够时）；供命中与测试。
pub(crate) fn jump_bottom_rect(area: Rect, lx: &Lx) -> Option<Rect> {
    // 面板打开时聊天栏整体不画（见 [`visible`]），命中区也必须一起消失，
    // 否则会出现"看不见却能点"的穿透。
    if lx.task.is_some() || lx.chat.follow_bottom || !super::history::has_history(lx) {
        return None;
    }
    let input = super::input_rect(area, lx)?;
    jump_rect(rect(area, input)?)
}

/// 栏内「回到底部」控件矩形（整体右对齐）。
///
/// 优先「箭头 + 空档 + 说明」；宽度只够箭头时退回纯箭头；再窄返回 None。
fn jump_rect(bar: Rect) -> Option<Rect> {
    let arrow = u16::try_from(text::LX_JUMP_BOTTOM.width()).unwrap_or(u16::MAX);
    let hint = u16::try_from(text::LX_JUMP_BOTTOM_HINT.width()).unwrap_or(u16::MAX);
    let width = arrow
        .saturating_add(HINT_GAP)
        .saturating_add(hint)
        .saturating_add(RIGHT_PAD);
    if bar.width >= width {
        let inner = arrow.saturating_add(HINT_GAP).saturating_add(hint);
        return Some(Rect::new(bar.right() - RIGHT_PAD - inner, bar.y, inner, 1));
    }
    (bar.width > arrow + RIGHT_PAD)
        .then(|| Rect::new(bar.right() - RIGHT_PAD - arrow, bar.y, arrow, 1))
}

/// 绘制聊天栏：loading（含 token 计数）> 回到底部。
pub(super) fn draw(grid: &mut Grid, bar: Option<Rect>, lx: &Lx) {
    let Some(bar) = bar else {
        return;
    };
    if bar.width == 0 {
        return;
    }
    let right = bar.right();
    // 回到底部控件（仅未跟随底部时）。
    if !lx.chat.follow_bottom
        && let Some(button) = jump_rect(bar)
    {
        let jump_style = style::chat_jump_bottom(lx.chat.jump_bottom_hover);
        grid.put_clipped(
            button.x,
            button.y,
            text::LX_JUMP_BOTTOM,
            jump_style,
            button.width,
        );
        // 说明文字在箭头右侧（宽度只够箭头时跳过）。
        let arrow = u16::try_from(text::LX_JUMP_BOTTOM.width()).unwrap_or(u16::MAX);
        let hint_x = button.x.saturating_add(arrow).saturating_add(HINT_GAP);
        if hint_x < button.right() {
            grid.put_clipped(
                hint_x,
                bar.y,
                text::LX_JUMP_BOTTOM_HINT,
                jump_style,
                button.right() - hint_x,
            );
        }
    }
    if !lx.chat.status.streaming {
        return;
    }
    // 括号统计紧贴 `Working...` 右侧（用户要求「在 working 的组右侧」）：
    // 静态 dim、不参与扫描——它回答「这轮跑了多久、大概用了多少 token」，
    // 与左侧的进行中动画是两种信息，不能一起闪。宽度不够时整段先丢；
    // Esc 待命时不画——那一刻用户要读的是「再按一次停止」。
    let stop_armed = lx.chat.stop_armed_at.is_some();
    let stats = if stop_armed {
        None
    } else {
        text::chat::stream_stats_label(lx.chat.turn_elapsed_ms, lx.chat.stream_output_tokens())
    };
    draw_loading(
        grid,
        bar.y,
        bar.x,
        right,
        lx.chat.loading_frame,
        stop_armed,
        stats.as_deref(),
    );
}

/// 绘制 `Working...`（无方块、无转轮）：只画带扫描高亮的文字。
#[allow(clippy::too_many_arguments)]
fn draw_loading(
    grid: &mut Grid,
    y: u16,
    bar_x: u16,
    right: u16,
    loading_frame: usize,
    stop_armed: bool,
    stats: Option<&str>,
) {
    // 右侧留 1 列空档避免贴边。
    let limit = right.saturating_sub(1);
    let x = bar_x.saturating_add(1);
    if x >= limit {
        return;
    }
    // Esc 待命：提示「再按一次停止」（取第一条放得下的短语，对齐 opencode）；
    // 待命提示同样走扫描高亮（它也是「进行中」的提示，观感保持一致）。
    let label = if stop_armed {
        stop_phrase(limit.saturating_sub(x))
    } else {
        text::chat::LOADING_LABEL
    };
    let after_label = draw_scanned_label(grid, x, y, limit, label, loading_frame);
    // 统计段紧跟其后：放得下才画，放不下整段丢（不在末尾留半截括号）。
    let Some(stats) = stats else {
        return;
    };
    let stats_x = after_label.saturating_add(STATS_GAP);
    let stats_w = u16::try_from(stats.width()).unwrap_or(u16::MAX);
    if stats_x.saturating_add(stats_w) > limit {
        return;
    }
    grid.put_clipped(
        stats_x,
        y,
        stats,
        style::chat_loading_label(),
        limit.saturating_sub(stats_x),
    );
}

/// 画扫描高亮文案，返回**文案结束后的列**（供紧随其后的统计段定位）。
fn draw_scanned_label(
    grid: &mut Grid,
    x: u16,
    y: u16,
    limit: u16,
    label: &str,
    loading_frame: usize,
) -> u16 {
    let width = usize::from(limit.saturating_sub(x));
    let chars: Vec<char> = label.chars().take(width).collect();
    if chars.is_empty() {
        return x;
    }
    // 扫描位置：每帧前进 1 个字符，走出末尾后从左侧重新进入。
    let span = chars.len() + SCAN_WINDOW;
    let head = loading_frame % span;
    let mut cell_x = x;
    for (index, ch) in chars.iter().enumerate() {
        // 窗口内：`head - SCAN_WINDOW < index <= head`（head 为窗口右端）。
        let in_window = index + SCAN_WINDOW >= head && index < head;
        let cell_style = if in_window {
            style::chat_loading_scan(loading_frame)
        } else {
            style::chat_loading_label()
        };
        grid.put_char(cell_x, y, *ch, cell_style);
        cell_x = cell_x.saturating_add(ch.width().unwrap_or(1).max(1) as u16);
        if cell_x >= limit {
            break;
        }
    }
    cell_x
}

/// Esc 待命提示短语：从长到短取第一条放得下的；都放不下时用最短的（由调用方裁剪）。
fn stop_phrase(width: u16) -> &'static str {
    let width = usize::from(width);
    text::LX_CHAT_STOP_PHRASES
        .iter()
        .find(|phrase| phrase.width() <= width)
        .copied()
        .unwrap_or_else(|| text::LX_CHAT_STOP_PHRASES[text::LX_CHAT_STOP_PHRASES.len() - 1])
}

/// 命中「回到底部」控件的窗格；无控件或未命中返回 None。
pub fn jump_bottom_hit(
    rects: &[(crate::layout::PaneId, Rect)],
    state: &crate::app::state::AppState,
    column: u16,
    row: u16,
) -> Option<crate::layout::PaneId> {
    rects.iter().find_map(|(id, rect)| {
        let target = state
            .pane_anywhere(*id)
            .filter(|pane| pane.view == crate::app::state::PaneView::Lx)?;
        let inner = crate::layout::pane_inner_rect(*rect);
        jump_bottom_rect(inner, &target.lx)
            .filter(|area| area.contains((column, row).into()))
            .map(|_| *id)
    })
}
