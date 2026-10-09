//! 单元测试；仅测试构建编译。

use super::*;
use crate::app::state::{PaneKind, PaneView};

#[test]
fn renders_text_and_skips_wide_spacers() {
    let mut terminal = Terminal::new(10, 2);
    terminal.feed("hi 你好".as_bytes());
    let mut buf = Buffer::empty(Rect::new(0, 0, 10, 2));
    let cursor = render_terminal(Rect::new(0, 0, 10, 2), &mut buf, &terminal, false);
    assert_eq!(cursor, None);
    assert_eq!(buf[(0, 0)].symbol(), "h");
    assert_eq!(buf[(1, 0)].symbol(), "i");
    assert_eq!(buf[(3, 0)].symbol(), "你");
    assert_eq!(buf[(4, 0)].diff_option, CellDiffOption::Skip);
    assert_eq!(buf[(5, 0)].symbol(), "好");
    assert_eq!(buf[(6, 0)].diff_option, CellDiffOption::Skip);
}

#[test]
fn focused_terminal_returns_cursor_position() {
    let mut terminal = Terminal::new(10, 2);
    terminal.feed(b"ab");
    let mut buf = Buffer::empty(Rect::new(0, 0, 10, 2));
    let cursor = render_terminal(Rect::new(0, 0, 10, 2), &mut buf, &terminal, true);
    assert_eq!(cursor, Some((0, 2)));
}

#[test]
fn hidden_cursor_is_not_returned() {
    let mut terminal = Terminal::new(10, 2);
    terminal.feed(b"\x1b[?25l");
    let mut buf = Buffer::empty(Rect::new(0, 0, 10, 2));
    let cursor = render_terminal(Rect::new(0, 0, 10, 2), &mut buf, &terminal, true);
    assert_eq!(cursor, None);
}

#[test]
fn cursor_outside_area_is_not_returned() {
    let mut terminal = Terminal::new(10, 2);
    terminal.feed(b"ab");
    let mut buf = Buffer::empty(Rect::new(0, 0, 1, 1));
    let cursor = render_terminal(Rect::new(0, 0, 1, 1), &mut buf, &terminal, true);
    assert_eq!(cursor, None);
}

#[test]
fn selection_marks_cells_reversed() {
    let mut terminal = Terminal::new(10, 2);
    terminal.feed(b"hello");
    terminal.start_selection(0, 1);
    terminal.update_selection(0, 3);
    let mut buf = Buffer::empty(Rect::new(0, 0, 10, 2));
    let cursor = render_terminal(Rect::new(0, 0, 10, 2), &mut buf, &terminal, false);
    assert_eq!(cursor, None);
    assert!(buf[(1, 0)].modifier.contains(Modifier::REVERSED));
    assert!(buf[(3, 0)].modifier.contains(Modifier::REVERSED));
    assert!(!buf[(0, 0)].modifier.contains(Modifier::REVERSED));
    assert!(!buf[(4, 0)].modifier.contains(Modifier::REVERSED));
}

#[test]
fn selection_follows_content_when_scrolled() {
    let mut terminal = Terminal::new(10, 2);
    terminal.feed(b"one\r\ntwo\r\nthree");
    // 屏幕显示 two/three；选中 two 后滚回一行，选区仍钉在 two 上（换到屏幕第 2 行）。
    terminal.start_selection(0, 0);
    terminal.update_selection(0, 2);
    assert!(terminal.scroll_display(1));
    let mut buf = Buffer::empty(Rect::new(0, 0, 10, 2));
    render_terminal(Rect::new(0, 0, 10, 2), &mut buf, &terminal, false);
    assert!(!buf[(0, 0)].modifier.contains(Modifier::REVERSED), "one");
    assert!(buf[(0, 1)].modifier.contains(Modifier::REVERSED), "two");
}

#[test]
fn scrollbar_appears_only_for_local_scrollback() {
    let mut terminal = Terminal::new(10, 2);
    terminal.feed(b"only");
    assert_eq!(terminal_scrollbar(Rect::new(0, 0, 10, 2), &terminal), None);

    for i in 0..8 {
        terminal.feed(format!("line {i}\r\n").as_bytes());
    }
    let bar = terminal_scrollbar(Rect::new(0, 0, 10, 2), &terminal).expect("scrollbar");
    assert_eq!(bar.track, Rect::new(9, 0, 1, 2));
    assert_eq!(bar.thumb.bottom(), bar.track.bottom());

    // 鼠标上报模式（opencode/claude 等）：应用自己滚动，隐藏滚动条。
    terminal.feed(b"\x1b[?1000h\x1b[?1006h");
    assert_eq!(terminal_scrollbar(Rect::new(0, 0, 10, 2), &terminal), None);

    // 备用屏且应用不接管滚轮时也没有本地内容可滚。
    terminal.feed(b"\x1b[?1000l\x1b[?1006l\x1b[?1049h");
    assert_eq!(terminal_scrollbar(Rect::new(0, 0, 10, 2), &terminal), None);
}

/// 构造指定视图的终端窗格。
fn pane_with(view: PaneView, terminal: Terminal) -> Pane {
    Pane {
        kind: PaneKind::Terminal,
        view,
        terminal,
        exited: false,
        cwd_label: None,
        cwd: None,
        agent: None,
    }
}

/// 把缓冲区按行拼成文本，便于断言内容。
fn buffer_text(buf: &Buffer) -> String {
    (0..buf.area.height)
        .map(|y| {
            (0..buf.area.width)
                .map(|x| buf[(x, y)].symbol())
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn lx_view_renders_page_without_cursor() {
    let pane = pane_with(PaneView::Lx, Terminal::new(60, 12));
    let mut buf = Buffer::empty(Rect::new(0, 0, 60, 12));
    let cursor = render(Rect::new(0, 0, 60, 12), &mut buf, &pane, true, 0);
    assert_eq!(cursor, None, "lx 页不产生硬件光标");
    let text = buffer_text(&buf);
    assert!(text.contains("click [>_] to open terminal"), "提示可见");
    assert!(text.contains('▀') || text.contains('█'), "像素画可见");
}

#[test]
fn terminal_view_dispatches_to_grid() {
    let mut terminal = Terminal::new(10, 2);
    terminal.feed(b"ab");
    let pane = pane_with(PaneView::Terminal, terminal);
    let mut buf = Buffer::empty(Rect::new(0, 0, 10, 2));
    let cursor = render(Rect::new(0, 0, 10, 2), &mut buf, &pane, true, 0);
    assert_eq!(cursor, Some((0, 2)));
    assert_eq!(buf[(0, 0)].symbol(), "a");
    assert_eq!(buf[(1, 0)].symbol(), "b");
}

#[test]
fn lx_view_hides_terminal_scrollbar() {
    let mut terminal = Terminal::new(10, 2);
    for i in 0..8 {
        terminal.feed(format!("line {i}\r\n").as_bytes());
    }
    assert!(terminal_scrollbar(Rect::new(0, 0, 10, 2), &terminal).is_some());
    let pane = pane_with(PaneView::Lx, terminal);
    assert_eq!(scrollbar(Rect::new(0, 0, 10, 2), &pane), None);
}

#[test]
fn toggle_button_sits_at_top_right_and_hides_when_narrow() {
    assert_eq!(
        toggle_button(Rect::new(0, 0, 20, 5)),
        Some(Rect::new(20 - 1 - 4, 0, 4, 1))
    );
    assert_eq!(toggle_button(Rect::new(0, 0, 6, 5)), None);
    assert_eq!(toggle_button(Rect::new(0, 0, 20, 0)), None);
}

#[test]
fn toggle_button_hit_test_and_label() {
    let id = PaneId::alloc();
    let rect = Rect::new(0, 0, 20, 5);
    let button = toggle_button(rect).expect("button fits");
    assert_eq!(
        toggle_button_at(&[(id, rect)], button.x, button.y),
        Some(id)
    );
    assert_eq!(toggle_button_at(&[(id, rect)], 0, 0), None);
    assert_eq!(toggle_label(PaneView::Lx), text::LX_TOGGLE_TERMINAL);
    assert_eq!(toggle_label(PaneView::Terminal), text::LX_TOGGLE_LX);
}

#[test]
fn draw_toggle_button_paints_border_label() {
    let rect = Rect::new(0, 0, 20, 5);
    let mut buf = Buffer::empty(rect);
    draw_toggle_button(&mut buf, rect, PaneView::Lx);
    let area = toggle_button(rect).expect("button fits");
    let painted: String = (area.x..area.right())
        .map(|x| buf[(x, 0)].symbol())
        .collect();
    assert_eq!(painted, text::LX_TOGGLE_TERMINAL);
}

#[test]
fn toggle_button_hit_ignores_narrow_panes() {
    let id = PaneId::alloc();
    let rect = Rect::new(0, 0, 6, 5);
    assert_eq!(toggle_button_at(&[(id, rect)], 2, 0), None);
}

#[test]
fn control_chars_render_as_blank() {
    let mut terminal = Terminal::new(10, 2);
    terminal.feed(b"a\tb");
    let mut buf = Buffer::empty(Rect::new(0, 0, 10, 2));
    render_terminal(Rect::new(0, 0, 10, 2), &mut buf, &terminal, false);
    assert_eq!(buf[(0, 0)].symbol(), "a");
    assert_eq!(buf[(1, 0)].symbol(), " ", "制表符落格渲染为空白");
    assert_eq!(buf[(8, 0)].symbol(), "b");
    let previous = Buffer::empty(Rect::new(0, 0, 10, 2));
    previous.diff(&buf);
}
