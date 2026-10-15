//! 浮层清底助手单元测试：处理被浮层边缘切成两半的底层宽字符。

use super::*;
use ratatui::buffer::{Buffer, CellDiffOption};
use ratatui::layout::Rect;
use ratatui::style::{Color, Style};

/// 造一帧底层内容：在 `x` 处放一个宽字符，右半格标记 diff 跳过。
fn underlying(width: u16, x: u16, y: u16, ch: char, bg: Color) -> Buffer {
    let mut buf = Buffer::empty(Rect::new(0, 0, width, 3));
    let cell = buf.cell_mut((x, y)).expect("in bounds");
    cell.set_char(ch);
    cell.set_style(Style::default().bg(bg));
    cell.set_diff_option(CellDiffOption::None);
    if let Some(spacer) = buf.cell_mut((x + 1, y)) {
        spacer.set_diff_option(CellDiffOption::Skip);
    }
    buf
}

#[test]
fn clears_region_to_blank() {
    let mut buf = Buffer::empty(Rect::new(0, 0, 12, 3));
    buf.cell_mut((2, 1)).expect("in bounds").set_char('x');
    render_buffer(&mut buf, Rect::new(1, 1, 4, 2));
    assert_eq!(buf[(2, 1)].symbol(), " ");
    assert_eq!(buf[(0, 1)].symbol(), " ", "区域外不受影响");
    assert_eq!(buf[(6, 1)].symbol(), " ", "区域外不受影响");
}

#[test]
fn erases_left_half_of_straddling_wide_char_keeping_style() {
    let mut buf = underlying(12, 3, 1, '页', Color::Blue);
    // 浮层左缘在 x=4：宽字符左半格在 3、右半格在 4。
    render_buffer(&mut buf, Rect::new(4, 1, 4, 2));
    let left = &buf[(3, 1)];
    assert_eq!(left.symbol(), " ", "左半格应被擦成空白");
    assert_eq!(left.bg, Color::Blue, "擦除保留原有底色");
    assert_eq!(left.diff_option, CellDiffOption::None);
}

#[test]
fn leaves_non_wide_left_neighbor_untouched() {
    let mut buf = Buffer::empty(Rect::new(0, 0, 12, 3));
    buf.cell_mut((3, 1)).expect("in bounds").set_char('a');
    render_buffer(&mut buf, Rect::new(4, 1, 4, 2));
    assert_eq!(buf[(3, 1)].symbol(), "a", "窄字符不应被改写");
}

#[test]
fn clears_right_orphan_spacer_of_straddling_wide_char() {
    // 宽字符左半格在 4（浮层内最后一列），右半格在 5（浮层外）。
    let mut buf = underlying(12, 4, 1, '页', Color::Blue);
    render_buffer(&mut buf, Rect::new(1, 1, 4, 2));
    let orphan = &buf[(5, 1)];
    assert_eq!(orphan.symbol(), " ", "浮层外占位半格应为空白");
    assert_eq!(
        orphan.diff_option,
        CellDiffOption::None,
        "必须清除跳过标记，否则终端残留半字"
    );
}

#[test]
fn no_panic_when_region_touches_left_screen_edge() {
    let mut buf = Buffer::empty(Rect::new(0, 0, 12, 3));
    render_buffer(&mut buf, Rect::new(0, 0, 4, 3));
    assert_eq!(buf[(3, 2)].symbol(), " ");
}

#[test]
fn clamps_out_of_bounds_region() {
    let mut buf = Buffer::empty(Rect::new(0, 0, 8, 2));
    render_buffer(&mut buf, Rect::new(6, 1, 10, 10));
    assert_eq!(buf[(7, 1)].symbol(), " ");
}

/// 回归（左缘）：差分必须放行被宽字符右半格压住的浮层左缘格。
#[test]
fn diff_emits_overlay_left_edge_after_break() {
    let prev = underlying(12, 3, 1, '页', Color::Blue);

    let mut next = underlying(12, 3, 1, '页', Color::Blue);
    render_buffer(&mut next, Rect::new(4, 1, 4, 2));
    next.cell_mut((4, 1))
        .expect("in bounds")
        .set_char('│')
        .set_style(Style::default().bg(Color::Indexed(236)));

    let emitted: Vec<(u16, u16)> = prev.diff_iter(&next).map(|(x, y, _)| (x, y)).collect();
    assert!(
        emitted.contains(&(4, 1)),
        "浮层左缘格必须下发，实际 {emitted:?}"
    );
}

/// 回归（右缘）：浮层外占位半格必须在差分中下发，否则终端残留半字。
#[test]
fn diff_emits_overlay_right_orphan_after_break() {
    let prev = underlying(12, 4, 1, '页', Color::Blue);

    let mut next = underlying(12, 4, 1, '页', Color::Blue);
    render_buffer(&mut next, Rect::new(1, 1, 4, 2));
    next.cell_mut((4, 1))
        .expect("in bounds")
        .set_char('│')
        .set_style(Style::default().bg(Color::Indexed(236)));

    let emitted: Vec<(u16, u16)> = prev.diff_iter(&next).map(|(x, y, _)| (x, y)).collect();
    assert!(
        emitted.contains(&(5, 1)),
        "浮层外占位半格必须下发以清除残留，实际 {emitted:?}"
    );
}
