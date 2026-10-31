//! lx 页 toast 的几何与绘制测试。
//!
//! 重点是那条用户报的问题的反面：**永远不越出窗格内容区**。全局 toast 在容器放不下
//! 时会降级到屏幕顶部居中、横跨整个窗口；lx 的提示必须始终留在自己的窗格内。

use super::*;
use crate::app::toast::{TOAST_DURATION, ToastKind};
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use std::time::Instant;

/// 一个 lx 页内容区（窗格内框）：宽度可调，用来模拟窄窗格。
fn area(width: u16, height: u16) -> Rect {
    Rect::new(4, 2, width, height)
}

/// 建一条 toast（不依赖 `AppState`，纯几何测试）。
fn toast(kind: ToastKind, message: &str) -> Toast {
    Toast::new(kind, message, None, Instant::now())
}

/// 长文案在窗格内折行：盒宽被夹到内容区宽度，绝不越界。
#[test]
fn long_message_wraps_inside_the_pane() {
    let area = area(30, 12);
    let message =
        "Compact failed; conversation was left unchanged. (summary request failed: cancelled)";
    let toast = toast(ToastKind::Danger, message);
    let rect = rect(area, &toast).expect("窄窗格也要放得下");
    assert!(
        rect.width <= area.width,
        "toast 宽度 {} 不得超过窗格内容区 {}",
        rect.width,
        area.width
    );
    assert!(rect.left() >= area.left(), "不得越出左边界");
    assert!(rect.right() <= area.right(), "不得越出右边界");
    assert!(
        rect.height > 3,
        "长文案必须折成多行而不是被切掉（高度 {}）",
        rect.height
    );
    assert!(rect.height <= MAX_BODY_ROWS + 2, "正文行数有上限");
}

/// 极窄窗格：宽度不足时整体不画（而不是溢出到隔壁窗格）。
#[test]
fn too_narrow_pane_draws_nothing() {
    let toast = toast(ToastKind::Warning, "watch out");
    assert!(rect(area(MIN_WIDTH - 1, 12), &toast).is_none());
    assert!(rect(area(2, 12), &toast).is_none());
}

/// 极矮窗格：放不下边框 + 一行正文时不画。
#[test]
fn too_short_pane_draws_nothing() {
    let toast = toast(ToastKind::Warning, "watch out");
    assert!(rect(area(40, 2), &toast).is_none());
}

/// 短文案保持紧凑：盒宽按内容收缩，不会撑满整个窗格。
#[test]
fn short_message_stays_compact() {
    let toast = toast(ToastKind::Default, "ok");
    let rect = rect(area(60, 12), &toast).expect("fits");
    assert_eq!(rect.width, MIN_WIDTH, "短文案按最小宽度显示");
    assert_eq!(rect.height, 3, "单行正文 + 上下边框");
}

/// 水平居中、贴在内容区顶边下方一行（与全局 toast 的容器内位置一致）。
#[test]
fn box_is_centered_below_the_top_edge() {
    let area = area(40, 12);
    let toast = toast(ToastKind::Default, "copied");
    let rect = rect(area, &toast).expect("fits");
    assert_eq!(rect.y, area.y + 1);
    assert_eq!(rect.x, area.x + (area.width - rect.width) / 2);
}

/// 绘制：移除背景色（保持默认透明底色 Color::Reset）、边框与 `[✕]` 取专属语义色。
#[test]
fn draw_clears_background_and_paints_dedicated_border_and_close_button() {
    let area = area(40, 10);
    let toast = toast(
        ToastKind::Danger,
        "Compact failed; conversation was left unchanged.",
    );
    let mut terminal = Terminal::new(TestBackend::new(50, 16)).expect("backend");
    let box_area = rect(area, &toast).expect("fits");
    terminal
        .draw(|frame| {
            draw(frame.buffer_mut(), area, &toast);
        })
        .expect("draw");
    let buffer = terminal.backend().buffer().clone();
    for y in box_area.top()..box_area.bottom() {
        for x in box_area.left()..box_area.right() {
            assert_eq!(
                buffer[(x, y)].bg,
                ratatui::style::Color::Reset,
                "toast 单元格 ({x},{y}) 不得铺设背景色"
            );
        }
    }
    // 边框与首行圆点均为语义红（danger）。
    assert_eq!(
        Some(buffer[(box_area.x, box_area.y)].fg),
        crate::ui::style::toast_border(ToastKind::Danger).fg
    );
    assert_eq!(
        buffer[(box_area.x + 2, box_area.y + 1)].symbol(),
        text::AGENT_STATUS_DOT
    );
    assert_eq!(
        Some(buffer[(box_area.x + 2, box_area.y + 1)].fg),
        crate::ui::style::toast_border(ToastKind::Danger).fg
    );
}

/// 三档语义色与全局 toast 完全一致（复用同一套样式函数）。
#[test]
fn kinds_share_the_global_palette() {
    for kind in [ToastKind::Default, ToastKind::Warning, ToastKind::Danger] {
        assert_eq!(
            crate::ui::toast::border_style(kind),
            crate::ui::toast::border_style(kind),
            "{kind:?} 边框色必须与全局 toast 一致"
        );
        assert_eq!(
            crate::ui::toast::title_style(kind),
            crate::ui::toast::title_style(kind),
            "{kind:?} 标题色必须与全局 toast 一致"
        );
    }
}

/// 有标题 toast 为 4 行紧凑盒：首行 `● <title>`、次行缩进正文。
#[test]
fn titled_toast_renders_dot_title_and_indented_body() {
    let area = area(40, 10);
    let toast = toast(ToastKind::Warning, "watch out").with_title("Clipboard");
    let box_area = rect(area, &toast).expect("fits");
    assert_eq!(box_area.height, 4);
    let mut terminal = Terminal::new(TestBackend::new(50, 16)).expect("backend");
    terminal
        .draw(|frame| {
            draw(frame.buffer_mut(), area, &toast);
        })
        .expect("draw");
    let buffer = terminal.backend().buffer().clone();
    let title_row: String = (box_area.x + 1..box_area.right() - 1)
        .map(|x| buffer[(x, box_area.y + 1)].symbol())
        .collect();
    let body_row: String = (box_area.x + 1..box_area.right() - 1)
        .map(|x| buffer[(x, box_area.y + 2)].symbol())
        .collect();
    assert!(title_row.contains("● Clipboard"), "{title_row:?}");
    assert!(body_row.contains("  watch out"), "{body_row:?}");
}

/// 盒宽按“正文 + 左右边框 + 左右内边距”计算，不把正文挤成提前折行。
#[test]
fn box_width_reserves_borders_and_padding() {
    let area = area(40, 12);
    let toast = toast(ToastKind::Default, "0123456789");
    let rect = rect(area, &toast).expect("fits");
    assert_eq!(rect.width, 10 + FRAME, "正文 10 列 + 边框/内边距 4 列");
}

/// 超出行数上限时最后一行必须带省略号（不能静默切掉）。
#[test]
fn overflowing_body_is_ellipsized() {
    let area = area(20, 10);
    let toast = toast(ToastKind::Danger, &"x".repeat(200));
    let box_area = rect(area, &toast).expect("fits");
    assert_eq!(box_area.height, MAX_BODY_ROWS + 2, "正文行数有上限");
    let mut terminal = Terminal::new(TestBackend::new(30, 16)).expect("backend");
    terminal
        .draw(|frame| {
            draw(frame.buffer_mut(), area, &toast);
        })
        .expect("draw");
    let buffer = terminal.backend().buffer().clone();
    let last_y = box_area.bottom() - 2;
    let row: String = (box_area.x + 1..box_area.right() - 1)
        .map(|x| buffer[(x, last_y)].symbol())
        .collect();
    assert!(
        row.trim_end().ends_with('…'),
        "最后一行必须显式省略：{row:?}"
    );
}

/// 极矮内容区：盒高被夹在内容区内，底边不得压到窗格边框。
#[test]
fn short_pane_keeps_the_box_inside() {
    let area = area(40, MIN_HEIGHT);
    let toast = toast(ToastKind::Warning, "watch out");
    let rect = rect(area, &toast).expect("恰好放得下");
    assert_eq!(rect.bottom(), area.bottom());
    assert!(rect.top() >= area.top());
}

/// 到期语义与全局 toast 共用 `Toast`：hover 期间不过期。
#[test]
fn hovering_freezes_expiry() {
    let mut toast = toast(ToastKind::Default, "hi");
    let deadline = toast.next_deadline().expect("未悬停时有到期时刻");
    assert!(
        !toast.is_expired(deadline - TOAST_DURATION / 2),
        "未到期时不过期"
    );
    toast.set_hovered(true);
    assert!(toast.next_deadline().is_none(), "悬停时不安排唤醒");
    assert!(
        !toast.is_expired(deadline + TOAST_DURATION),
        "悬停期间不过期"
    );
    toast.set_hovered(false);
    assert!(toast.is_expired(deadline), "取消悬停后到点即过期");
}

/// 回归：包含中文宽字符的 toast（如 `/rename 询问用户`）按显示宽度推进列，
/// 终端 diff 不得因宽字符挤在同一列而吞掉后续字符。
#[test]
fn wide_cjk_characters_advance_by_display_width_without_clipping() {
    let area = area(60, 10);
    let message = text::chat::session_renamed("询问用户");
    assert_eq!(message, "Renamed to \"询问用户\"");
    let toast = toast(ToastKind::Warning, &message);
    let box_area = rect(area, &toast).expect("fits");
    let prev = Buffer::empty(Rect::new(0, 0, 70, 14));
    let mut next = prev.clone();
    draw(&mut next, area, &toast);

    // 验证终端 diff 下发的完整字符序列包含全部中文字符「询问用户」，不再跳过任何宽字符。
    let emitted: String = prev
        .diff_iter(&next)
        .filter(|(_, y, _)| *y == box_area.y + 1)
        .map(|(_, _, cell)| cell.symbol())
        .collect();
    assert!(
        emitted.contains("\"询问用户\""),
        "toast 必须完整下发全部宽字符，实际: {emitted:?}"
    );
}
