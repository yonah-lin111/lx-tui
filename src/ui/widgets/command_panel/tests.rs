//! 单元测试；仅测试构建编译。

use super::*;

fn items<'a>(pairs: &[(&'a str, &'a str)]) -> Vec<CommandItem<'a>> {
    pairs
        .iter()
        .map(|&(label, preview)| CommandItem { label, preview })
        .collect()
}

/// 行文本：内容区一行按符号拼接。
fn row_text(buf: &Buffer, area: Rect, y: u16) -> String {
    (area.x..area.right())
        .map(|x| buf[(x, y)].symbol())
        .collect()
}

#[test]
fn renders_below_anchor_without_covering_it() {
    let area = Rect::new(0, 0, 24, 10);
    let mut buf = Buffer::empty(area);
    let items = items(&[
        ("Heading 1", "#"),
        ("Heading 2", "##"),
        ("Heading 3", "###"),
    ]);
    let rect = render(
        area,
        &mut buf,
        &CommandPanelView {
            items: &items,
            active: 0,
            anchor_row: 2,
        },
    )
    .expect("panel renders");

    assert_eq!(rect, Rect::new(0, 3, 17, 5));
    assert_eq!(buf[(0, 3)].symbol(), "╭");
    assert_eq!(buf[(0, 2)].symbol(), " ");
    assert!(buf[(2, 4)].modifier.contains(Modifier::REVERSED));
    assert!(row_text(&buf, area, 4).contains("Heading 1"));
    assert!(row_text(&buf, area, 4).contains("#"));
}

#[test]
fn flips_above_when_below_is_tight() {
    let area = Rect::new(0, 0, 24, 6);
    let mut buf = Buffer::empty(area);
    let items = items(&[
        ("A", "a"),
        ("B", "b"),
        ("C", "c"),
        ("D", "d"),
        ("E", "e"),
        ("F", "f"),
    ]);
    let rect = render(
        area,
        &mut buf,
        &CommandPanelView {
            items: &items,
            active: 0,
            anchor_row: 4,
        },
    )
    .expect("panel renders");

    assert_eq!(rect, Rect::new(0, 0, 7, 4));
    assert_eq!(rect.bottom(), 4);
    assert!(buf[(2, 1)].modifier.contains(Modifier::REVERSED));
}

#[test]
fn scrolls_window_to_keep_active_visible() {
    let area = Rect::new(0, 0, 24, 8);
    let mut buf = Buffer::empty(area);
    let items = items(&[
        ("Heading 1", "#"),
        ("Heading 2", "##"),
        ("Heading 3", "###"),
        ("Heading 4", "####"),
        ("Heading 5", "#####"),
        ("Heading 6", "######"),
    ]);
    let rect = render(
        area,
        &mut buf,
        &CommandPanelView {
            items: &items,
            active: 5,
            anchor_row: 0,
        },
    )
    .expect("panel renders");

    assert_eq!(rect, Rect::new(0, 1, 20, 7));
    assert!(row_text(&buf, area, 2).contains("Heading 2"));
    assert!(!row_text(&buf, area, 2).contains("Heading 1"));
    assert!(row_text(&buf, area, 6).contains("Heading 6"));
    assert!(buf[(2, 6)].modifier.contains(Modifier::REVERSED));
    assert!(!buf[(2, 2)].modifier.contains(Modifier::REVERSED));
}

#[test]
fn skips_render_without_space() {
    let area = Rect::new(0, 0, 24, 3);
    let mut buf = Buffer::empty(area);
    let items = items(&[("Heading 1", "#"), ("Heading 2", "##")]);
    assert_eq!(
        render(
            area,
            &mut buf,
            &CommandPanelView {
                items: &items,
                active: 0,
                anchor_row: 1,
            },
        ),
        None
    );
}

#[test]
fn clamps_width_and_truncates_items() {
    let area = Rect::new(0, 0, 10, 6);
    let mut buf = Buffer::empty(area);
    let items = items(&[("A very long heading label", "######")]);
    let rect = render(
        area,
        &mut buf,
        &CommandPanelView {
            items: &items,
            active: 0,
            anchor_row: 0,
        },
    )
    .expect("panel renders");

    assert_eq!(rect, Rect::new(0, 1, 10, 3));
    assert_eq!(buf[(0, 1)].symbol(), "╭");
    assert_eq!(buf[(9, 1)].symbol(), "╮");
    assert_eq!(buf[(0, 3)].symbol(), "╰");
    assert_eq!(buf[(9, 3)].symbol(), "╯");
}

#[test]
fn skips_without_items_or_area() {
    let area = Rect::new(0, 0, 24, 8);
    let mut buf = Buffer::empty(area);
    assert_eq!(
        render(
            area,
            &mut buf,
            &CommandPanelView {
                items: &[],
                active: 0,
                anchor_row: 0,
            },
        ),
        None
    );
    let narrow = Rect::new(0, 0, 2, 8);
    let mut buf = Buffer::empty(narrow);
    let items = items(&[("A", "a")]);
    assert_eq!(
        render(
            narrow,
            &mut buf,
            &CommandPanelView {
                items: &items,
                active: 0,
                anchor_row: 0,
            },
        ),
        None
    );
}
