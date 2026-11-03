//! 最近页面抽屉渲染测试：分割线、双行条目、时间/状态与悬停高亮。

use super::*;
use crate::app::prompt::PromptPageId;
use ratatui::style::{Color, Modifier};

fn row_with(status: Option<TemplateStatus>, current: bool) -> RecentPageRow {
    RecentPageRow {
        item_id: 1,
        page_id: PromptPageId::Saved(1),
        prompt_name: "Alpha".into(),
        page_title: "Auth Flow".into(),
        status,
        updated_at: 100,
        time_label: "[5m]".into(),
        current,
    }
}

fn row(current: bool) -> RecentPageRow {
    row_with(Some(TemplateStatus::InProgress), current)
}

fn draw(
    rows: &[RecentPageRow],
    hover: Option<usize>,
    active: Option<usize>,
) -> (ratatui::buffer::Buffer, Rect) {
    let screen = Rect::new(0, 0, 40, 8);
    let mut buf = ratatui::buffer::Buffer::empty(screen);
    let panel = Rect::new(0, 0, 40, 8);
    let divider = Rect::new(0, 2, 40, 1);
    let area = Rect::new(1, 3, 38, 4);
    render(
        &mut buf,
        &RecentRender {
            panel,
            divider,
            area,
            rows,
            scroll: 0,
            visible_entries: 2,
            focused: true,
            hover,
            active,
        },
    );
    (buf, area)
}

fn line(buf: &ratatui::buffer::Buffer, y: u16, width: u16) -> String {
    (0..width)
        .map(|x| buf[(x, y)].symbol().to_string())
        .collect::<String>()
        .trim_end()
        .to_string()
}

#[test]
fn renders_page_title_then_prompt_name_without_dot() {
    let (buf, _) = draw(&[row(true)], None, None);
    assert!(line(&buf, 2, 40).contains("recent"), "分割线标签");
    let first = line(&buf, 3, 40);
    let second = line(&buf, 4, 40);
    assert!(first.contains("Auth Flow"), "{first}");
    assert!(first.contains("[5m]"), "{first}");
    assert!(first.trim_start().starts_with('❯'), "当前页标记：{first}");
    assert!(second.contains("Alpha"), "{second}");
    for text in [&first, &second] {
        assert!(
            !text.contains('●') && !text.contains('○'),
            "不再绘制状态圆点：{text}"
        );
    }
}

#[test]
fn page_title_uses_status_colors() {
    let cases = [
        (TemplateStatus::Todo, Color::Reset, false),
        (TemplateStatus::InProgress, Color::Yellow, false),
        (TemplateStatus::Done, Color::Green, false),
    ];
    for (status, color, dim) in cases {
        let (buf, area) = draw(&[row_with(Some(status), false)], None, None);
        let cell = &buf[(area.x + 2, area.y)];
        assert_eq!(cell.fg, color, "{status:?} 页标题颜色");
        assert_eq!(cell.modifier.contains(Modifier::DIM), dim, "{status:?} DIM");
    }

    let (buf, area) = draw(&[row_with(None, false)], None, None);
    let cell = &buf[(area.x + 2, area.y)];
    assert!(cell.modifier.contains(Modifier::DIM), "无状态页标题压暗");
}

#[test]
fn current_page_title_is_bold() {
    let (buf, area) = draw(&[row(true)], None, None);
    let cell = &buf[(area.x + 2, area.y)];
    assert!(cell.modifier.contains(Modifier::BOLD), "当前页标题加粗");
}

#[test]
fn status_color_wins_over_hover_foreground() {
    let (buf, area) = draw(
        &[row_with(Some(TemplateStatus::Done), false)],
        Some(0),
        None,
    );
    let cell = &buf[(area.x + 2, area.y)];
    assert_eq!(cell.fg, Color::Green, "状态色优先于选中前景");
    assert_eq!(
        cell.bg,
        style::selection().bg.expect("selection background"),
        "选中底色保留"
    );
}

#[test]
fn hover_fills_both_rows() {
    let (buf, area) = draw(&[row(false), row(false)], Some(1), None);
    let highlight = style::selection();
    let bg = highlight.bg.expect("selection background");
    for y in [area.y + 2, area.y + 3] {
        assert_eq!(buf[(area.x, y)].bg, bg, "悬停两行同时铺底色");
        assert_eq!(buf[(area.right() - 1, y)].bg, bg, "铺满整行");
    }
    assert_ne!(buf[(area.x, area.y)].bg, bg, "未悬停条目不铺底色");
}

#[test]
fn active_entry_fills_both_rows_without_mouse() {
    let (buf, area) = draw(&[row(false), row(false)], None, Some(1));
    let highlight = style::selection();
    let bg = highlight.bg.expect("selection background");
    for y in [area.y + 2, area.y + 3] {
        assert_eq!(buf[(area.x, y)].bg, bg, "键盘选中两行同时铺底色");
        assert_eq!(buf[(area.right() - 1, y)].bg, bg, "铺满整行");
    }
    assert_ne!(buf[(area.x, area.y)].bg, bg, "未选中条目不铺底色");
}

#[test]
fn divider_shows_close_button() {
    let (buf, _) = draw(&[row(false)], None, None);
    assert!(
        line(&buf, 2, 40).contains(text::PROMPT_RECENT_CLOSE),
        "抽屉右上角显示关闭按钮"
    );
}

#[test]
fn hover_takes_priority_over_active_entry() {
    let (buf, area) = draw(&[row(false), row(false)], Some(0), Some(1));
    let highlight = style::selection();
    let bg = highlight.bg.expect("selection background");
    assert_eq!(buf[(area.x, area.y)].bg, bg, "悬停项优先高亮");
    assert_ne!(buf[(area.x, area.y + 2)].bg, bg, "键盘选中项暂不铺底色");
}

#[test]
fn empty_state_shows_placeholder() {
    let (buf, area) = draw(&[], None, None);
    assert!(line(&buf, area.y, 40).contains(text::PROMPT_RECENT_EMPTY));
}

#[test]
fn scroll_window_skips_entries() {
    let (buf, area) = draw(&[row(false), row(false)], None, None);
    // 第二个条目在同一窗口下渲染在 5、6 行；用 scroll=1 重新绘制只看第二条。
    let screen = Rect::new(0, 0, 40, 8);
    let mut buf2 = ratatui::buffer::Buffer::empty(screen);
    render(
        &mut buf2,
        &RecentRender {
            panel: Rect::new(0, 0, 40, 8),
            divider: Rect::new(0, 2, 40, 1),
            area,
            rows: &[row(false), row(true)],
            scroll: 1,
            visible_entries: 1,
            focused: true,
            hover: None,
            active: None,
        },
    );
    let first = line(&buf2, area.y, 40);
    assert!(first.contains('❯'), "滚动后显示第二条：{first}");
    assert!(!line(&buf, area.y, 40).contains('❯'));
}
