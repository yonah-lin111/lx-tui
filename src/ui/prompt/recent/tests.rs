//! 最近页面抽屉渲染测试：分割线、双行条目、时间/状态与悬停高亮。

use super::*;
use crate::app::prompt::PromptPageId;

fn row(current: bool) -> RecentPageRow {
    RecentPageRow {
        item_id: 1,
        page_id: PromptPageId::Saved(1),
        prompt_name: "Alpha".into(),
        page_title: "Auth Flow".into(),
        status: Some(TemplateStatus::InProgress),
        updated_at: 100,
        time_label: "[5m]".into(),
        current,
    }
}

fn draw(rows: &[RecentPageRow], hover: Option<usize>) -> (ratatui::buffer::Buffer, Rect) {
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
fn renders_two_line_entries_with_time_and_status_dot() {
    let (buf, _) = draw(&[row(true)], None);
    assert!(line(&buf, 2, 40).contains("recent"), "分割线标签");
    let first = line(&buf, 3, 40);
    let second = line(&buf, 4, 40);
    assert!(first.contains("Alpha"), "{first}");
    assert!(first.contains("[5m]"), "{first}");
    assert!(first.trim_start().starts_with('❯'), "当前页标记：{first}");
    assert!(second.contains("Auth Flow"), "{second}");
    assert!(second.ends_with('●'), "状态圆点在右缘：{second}");
}

#[test]
fn hover_fills_both_rows() {
    let (buf, area) = draw(&[row(false), row(false)], Some(1));
    let highlight = style::selection();
    let bg = highlight.bg.expect("selection background");
    for y in [area.y + 2, area.y + 3] {
        assert_eq!(buf[(area.x, y)].bg, bg, "悬停两行同时铺底色");
        assert_eq!(buf[(area.right() - 1, y)].bg, bg, "铺满整行");
    }
    assert_ne!(buf[(area.x, area.y)].bg, bg, "未悬停条目不铺底色");
}

#[test]
fn empty_state_shows_placeholder() {
    let (buf, area) = draw(&[], None);
    assert!(line(&buf, area.y, 40).contains(text::PROMPT_RECENT_EMPTY));
}

#[test]
fn scroll_window_skips_entries() {
    let (buf, area) = draw(&[row(false), row(false)], None);
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
        },
    );
    let first = line(&buf2, area.y, 40);
    assert!(first.contains('❯'), "滚动后显示第二条：{first}");
    assert!(!line(&buf, area.y, 40).contains('❯'));
}
