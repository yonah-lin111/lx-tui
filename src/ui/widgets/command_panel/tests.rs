//! 单元测试；仅测试构建编译。

use super::*;

/// 单行条目。
fn inline<'a>(pairs: &[(&'a str, &'a str)]) -> Vec<CommandItem<'a>> {
    pairs
        .iter()
        .map(|&(label, preview)| CommandItem::Inline { label, preview })
        .collect()
}

/// 双行条目。
fn stacked<'a>(pairs: &[(&'a str, &'a str)]) -> Vec<CommandItem<'a>> {
    pairs
        .iter()
        .map(|&(label, detail)| CommandItem::Stacked { label, detail })
        .collect()
}

/// 视图。
fn view<'a>(
    items: &'a [CommandItem<'a>],
    active: usize,
    anchor_row: u16,
    max_height: Option<u16>,
) -> CommandPanelView<'a> {
    CommandPanelView {
        items,
        active,
        anchor_row,
        max_height,
    }
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
    let items = inline(&[
        ("Heading 1", "#"),
        ("Heading 2", "##"),
        ("Heading 3", "###"),
    ]);
    let rect = render(area, &mut buf, &view(&items, 0, 2, None)).expect("panel renders");

    assert_eq!(rect, Rect::new(0, 3, 17, 5));
    assert_eq!(buf[(0, 3)].symbol(), "╭");
    assert_eq!(buf[(0, 2)].symbol(), " ");
    assert!(buf[(2, 4)].modifier.contains(Modifier::REVERSED));
    let row = row_text(&buf, area, 4);
    let label_at = row.find("Heading 1").expect("label first");
    let format_at = row.find('#').expect("preview after");
    assert!(label_at < format_at);
}

#[test]
fn aligns_preview_column_across_items() {
    let area = Rect::new(0, 0, 24, 8);
    let mut buf = Buffer::empty(area);
    let items = inline(&[("Heading 1", "#"), ("Hi", "##")]);
    let rect = render(area, &mut buf, &view(&items, 0, 0, None)).expect("panel renders");

    assert_eq!(rect, Rect::new(0, 1, 16, 4));
    let first = row_text(&buf, area, 2);
    let second = row_text(&buf, area, 3);
    assert_eq!(first.find('#'), second.find('#'));
    assert!(first.contains("Heading 1"));
    assert!(second.contains("Hi"));
}

#[test]
fn flips_above_when_below_is_tight() {
    let area = Rect::new(0, 0, 24, 6);
    let mut buf = Buffer::empty(area);
    let items = inline(&[
        ("A", "a"),
        ("B", "b"),
        ("C", "c"),
        ("D", "d"),
        ("E", "e"),
        ("F", "f"),
    ]);
    let rect = render(area, &mut buf, &view(&items, 0, 4, None)).expect("panel renders");

    assert_eq!(rect, Rect::new(0, 0, 8, 4));
    assert_eq!(rect.bottom(), 4);
    assert!(buf[(2, 1)].modifier.contains(Modifier::REVERSED));
}

#[test]
fn scrolls_window_to_keep_active_visible() {
    let area = Rect::new(0, 0, 24, 8);
    let mut buf = Buffer::empty(area);
    let items = inline(&[
        ("Heading 1", "#"),
        ("Heading 2", "##"),
        ("Heading 3", "###"),
        ("Heading 4", "####"),
        ("Heading 5", "#####"),
        ("Heading 6", "######"),
    ]);
    let rect = render(area, &mut buf, &view(&items, 5, 0, None)).expect("panel renders");

    assert_eq!(rect, Rect::new(0, 1, 21, 7));
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
    let items = inline(&[("Heading 1", "#"), ("Heading 2", "##")]);
    assert_eq!(render(area, &mut buf, &view(&items, 0, 1, None)), None);
}

#[test]
fn clamps_width_and_truncates_items() {
    let area = Rect::new(0, 0, 10, 6);
    let mut buf = Buffer::empty(area);
    let items = inline(&[("A very long heading label", "######")]);
    let rect = render(area, &mut buf, &view(&items, 0, 0, None)).expect("panel renders");

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
    assert_eq!(render(area, &mut buf, &view(&[], 0, 0, None)), None);
    let narrow = Rect::new(0, 0, 2, 8);
    let mut buf = Buffer::empty(narrow);
    let items = inline(&[("A", "a")]);
    assert_eq!(render(narrow, &mut buf, &view(&items, 0, 0, None)), None);
}

#[test]
fn renders_stacked_items_with_detail_row() {
    let area = Rect::new(0, 0, 24, 10);
    let mut buf = Buffer::empty(area);
    let items = stacked(&[("app.rs", "src"), ("main.rs", "src/bin")]);
    let rect = render(area, &mut buf, &view(&items, 0, 0, None)).expect("panel renders");

    assert_eq!(rect, Rect::new(0, 1, 11, 6));
    assert!(row_text(&buf, area, 2).contains("app.rs"));
    assert!(row_text(&buf, area, 3).contains("src"));
    assert!(buf[(3, 3)].modifier.contains(Modifier::DIM));
    assert!(row_text(&buf, area, 4).contains("main.rs"));
    assert!(row_text(&buf, area, 5).contains("src/bin"));
    assert!(buf[(2, 2)].modifier.contains(Modifier::REVERSED));
    assert!(buf[(2, 3)].modifier.contains(Modifier::REVERSED));
    assert!(!buf[(2, 4)].modifier.contains(Modifier::REVERSED));
}

#[test]
fn stacked_item_without_detail_takes_one_row() {
    let area = Rect::new(0, 0, 24, 8);
    let mut buf = Buffer::empty(area);
    let items = stacked(&[("app.rs", "")]);
    let rect = render(area, &mut buf, &view(&items, 0, 0, None)).expect("panel renders");

    assert_eq!(rect, Rect::new(0, 1, 10, 3));
    assert!(row_text(&buf, area, 2).contains("app.rs"));
}

#[test]
fn caps_panel_height_and_follows_active() {
    let area = Rect::new(0, 0, 24, 12);
    let mut buf = Buffer::empty(area);
    let labels: Vec<(String, String)> = (0..10)
        .map(|index| (format!("file{index}.rs"), "src".to_string()))
        .collect();
    let items: Vec<CommandItem<'_>> = labels
        .iter()
        .map(|(label, detail)| CommandItem::Stacked { label, detail })
        .collect();
    let rect = render(area, &mut buf, &view(&items, 5, 0, Some(8))).expect("panel renders");

    assert_eq!(rect.height, 8);
    assert!(
        row_text(&buf, area, 2).contains("file3.rs"),
        "row2={:?}",
        row_text(&buf, area, 2)
    );
    assert!(
        row_text(&buf, area, 6).contains("file5.rs"),
        "row6={:?}",
        row_text(&buf, area, 6)
    );
    assert!(buf[(2, 6)].modifier.contains(Modifier::REVERSED));
    assert!(!buf[(2, 2)].modifier.contains(Modifier::REVERSED));
}

#[test]
fn item_at_maps_rows_to_items() {
    let area = Rect::new(0, 0, 24, 12);
    let items = stacked(&[("a.rs", "src"), ("b.rs", "src"), ("c.rs", "src")]);
    let layout = layout(area, &view(&items, 0, 0, Some(6))).expect("panel layout");

    assert_eq!(layout.start, 0);
    assert_eq!(layout.visible, 2);
    assert!(layout.scrollbar.is_some());
    assert_eq!(layout.content.width, layout.inner.width - 1);
    assert_eq!(item_at(&layout, &items, 2, layout.content.y), Some(0));
    assert_eq!(item_at(&layout, &items, 2, layout.content.y + 1), Some(0));
    assert_eq!(item_at(&layout, &items, 2, layout.content.y + 2), Some(1));
    assert_eq!(item_at(&layout, &items, 2, layout.rect.y), None);
    assert_eq!(
        item_at(&layout, &items, layout.rect.x, layout.content.y),
        None
    );
    assert_eq!(
        item_at(&layout, &items, layout.inner.right() - 1, layout.content.y),
        None
    );
    assert_eq!(
        item_at(&layout, &items, 2, layout.content.bottom() + 1),
        None
    );
}

#[test]
fn item_at_respects_window_offset() {
    let area = Rect::new(0, 0, 24, 12);
    let items = stacked(&[
        ("f0", "s"),
        ("f1", "s"),
        ("f2", "s"),
        ("f3", "s"),
        ("f4", "s"),
        ("f5", "s"),
    ]);
    let layout = layout(area, &view(&items, 5, 0, Some(8))).expect("panel layout");

    assert_eq!(layout.start, 3);
    assert_eq!(item_at(&layout, &items, 2, layout.content.y), Some(3));
    assert_eq!(item_at(&layout, &items, 2, layout.content.y + 4), Some(5));
}

#[test]
fn renders_scrollbar_when_overflowing() {
    let area = Rect::new(0, 0, 24, 12);
    let mut buf = Buffer::empty(area);
    let items = stacked(&[
        ("f0", "s"),
        ("f1", "s"),
        ("f2", "s"),
        ("f3", "s"),
        ("f4", "s"),
        ("f5", "s"),
    ]);
    let layout = layout(area, &view(&items, 0, 0, Some(8))).expect("panel layout");
    render(area, &mut buf, &view(&items, 0, 0, Some(8))).expect("panel renders");

    let bar = layout.scrollbar.expect("scrollbar visible");
    assert_eq!(bar.track.x, layout.inner.right() - 1);
    assert_eq!(buf[(bar.thumb.x, bar.thumb.y)].symbol(), "▐");
    assert!(
        buf[(bar.thumb.x, bar.thumb.y)]
            .modifier
            .contains(Modifier::BOLD)
    );
    assert_eq!(buf[(bar.track.x, bar.track.bottom() - 1)].symbol(), "▕");
    assert!(
        buf[(bar.track.x, bar.track.bottom() - 1)]
            .modifier
            .contains(Modifier::DIM)
    );
}
