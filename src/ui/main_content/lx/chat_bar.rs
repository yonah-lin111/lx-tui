//! 输入框上方聊天栏：流中 loading 卡片 + 右端「回到底部」控件 + 临时警告。
//!
//! 纯渲染 + 纯几何：布局、绘制与命中共享同一矩形口径；只读 `Lx`、只写 lx 页网格。
//! 该行贴在输入框上沿、与输入框同宽；无流中、无警告且已跟随底部时整行不占位。
//! 「回到底部」控件为箭头 + 右侧说明文字（`↓ jump to bottom`），整体右对齐。
//!
//! **loading 的形态**（对齐用户要求的「正方形卡片 Working...」）：
//! - 方块 `■` 与聊天历史的步骤方块同一个字形；颜色按 lx 页 `Native Agent` 标签的
//!   4 档渐变（218 粉 → 182 淡紫 → 153 浅蓝 → 117 天蓝）**循环**，每
//!   `LOADING_FRAME_INTERVAL`（100ms）换一档；
//! - `Working...` 平时是次要色，有一个 3 字符宽的高亮窗口**从左到右扫描**，
//!   窗口内字符立刻变成「卡片当前那一档色」——同一时刻卡片与亮点同色，形成
//!   「颜色从卡片流向文字」的观感。
//!
//! **临时警告**（未知命令等）复用这一行：黄色、`Warning: ` 开头、2 秒后自动消失；
//! 它出现时**优先占用该行**，「回到底部」控件让位不显示（用户此刻视线在输入框，
//! 不需要跳转按钮，且避免窄屏下两者互相挤压截断警告文案）。

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

/// 是否显示聊天栏：有临时警告、流中（loading），或上滚未跟随底部（回到底部控件）。
pub(super) fn visible(lx: &Lx) -> bool {
    if lx.chat.messages.is_empty() && lx.chat.warning.is_none() {
        return false;
    }
    lx.chat.warning.is_some() || lx.chat.status.streaming || !lx.chat.follow_bottom
}

/// 由输入框矩形推导聊天栏矩形（贴其上沿）；上方空间不足时 None。
pub(super) fn rect(area: Rect, input: Rect) -> Option<Rect> {
    (input.y >= area.y.saturating_add(HEIGHT))
        .then(|| Rect::new(input.x, input.y - HEIGHT, input.width, HEIGHT))
}

/// 回到底部控件矩形（未跟随底部、**无临时警告**且空间足够时）；供命中与测试。
pub(crate) fn jump_bottom_rect(area: Rect, lx: &Lx) -> Option<Rect> {
    if lx.chat.follow_bottom || lx.chat.messages.is_empty() || lx.chat.warning.is_some() {
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

/// 绘制聊天栏：左侧 loading 卡片 / 临时警告，右端「回到底部」控件。
///
/// 优先级：**临时警告 > loading > 回到底部**。
/// - 有警告时整行只画警告（黄色 `Warning: …`），jump 让位；
/// - 流中画 `■ Working...`（卡片按 Native 渐变循环，文案自左向右扫描高亮）；
/// - 未跟随底部时右端画 `↓ 说明`（悬停高亮）。
pub(super) fn draw(grid: &mut Grid, bar: Option<Rect>, lx: &Lx) {
    let Some(bar) = bar else {
        return;
    };
    if bar.width == 0 {
        return;
    }
    // 临时警告优先独占该行。
    if let Some(warning) = lx.chat.warning.as_ref() {
        let x = bar.x.saturating_add(1);
        let limit = bar.right().saturating_sub(1);
        if x < limit {
            grid.put_clipped(
                x,
                bar.y,
                &format!("{}{}", text::chat::WARNING_PREFIX, warning.message),
                style::chat_warning(),
                limit.saturating_sub(x),
            );
        }
        return;
    }
    let mut right = bar.right();
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
                button.y,
                text::LX_JUMP_BOTTOM_HINT,
                jump_style,
                button.right() - hint_x,
            );
        }
        right = button.x.saturating_sub(1);
    }
    if !lx.chat.status.streaming {
        return;
    }
    draw_loading(
        grid,
        bar.y,
        bar.x,
        right,
        lx.chat.loading_frame,
        lx.chat.stop_armed_at.is_some(),
    );
}

/// 绘制 `■ Working...`：方块取 Native 渐变循环色，文案做从左到右的扫描高亮。
///
/// 卡片与扫描窗口同色同频：`loading_frame` 同时决定「卡片当前是哪一档色」和
/// 「高亮窗口走到第几个字符」，因此视觉上是颜色从卡片一路流向文字。
fn draw_loading(
    grid: &mut Grid,
    y: u16,
    bar_x: u16,
    right: u16,
    loading_frame: usize,
    stop_armed: bool,
) {
    // 右侧留 1 列空档避免贴边。
    let limit = right.saturating_sub(1);
    let mut x = bar_x.saturating_add(1);
    if x >= limit {
        return;
    }
    // 卡片：当前档位的循环色。
    let cycle = text::agent::native_label_cycle();
    let phase = loading_frame % cycle.len();
    grid.put_char(
        x,
        y,
        text::chat::LOADING_MARK,
        style::chat_loading_phase(phase),
    );
    x = x.saturating_add(2);

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
    draw_scanned_label(grid, x, y, limit, label, loading_frame);
}

/// 画扫描高亮文案：窗口外的字符用次要色（dim），窗口内用卡片当前档位色（加粗）。
fn draw_scanned_label(
    grid: &mut Grid,
    x: u16,
    y: u16,
    limit: u16,
    label: &str,
    loading_frame: usize,
) {
    let width = usize::from(limit.saturating_sub(x));
    let chars: Vec<char> = label.chars().take(width).collect();
    if chars.is_empty() {
        return;
    }
    let cycle = text::agent::native_label_cycle();
    let phase = loading_frame % cycle.len();
    // 扫描位置：每帧前进 1 个字符，走出末尾后从左侧重新进入。
    let span = chars.len() + SCAN_WINDOW;
    let head = loading_frame % span;
    let mut cell_x = x;
    for (index, ch) in chars.iter().enumerate() {
        // 窗口内：`head - SCAN_WINDOW < index <= head`（head 为窗口右端）。
        let in_window = index + SCAN_WINDOW >= head && index < head;
        let cell_style = if in_window {
            style::chat_loading_phase(phase)
        } else {
            style::chat_loading_label()
        };
        grid.put_char(cell_x, y, *ch, cell_style);
        cell_x = cell_x.saturating_add(ch.width().unwrap_or(1).max(1) as u16);
        if cell_x >= limit {
            break;
        }
    }
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
