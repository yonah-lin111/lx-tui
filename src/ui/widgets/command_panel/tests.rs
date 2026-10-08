//! 单元测试；仅测试构建编译。

use super::*;
use ratatui::style::{Color, Modifier};

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
        .map(|&(label, detail)| CommandItem::Stacked {
            icon: None,
            label,
            detail,
        })
        .collect()
}

/// 带图标双行条目。
fn stacked_icons<'a>(triples: &[(&'a str, &'a str, &'a str)]) -> Vec<CommandItem<'a>> {
    triples
        .iter()
        .map(|&(icon, label, detail)| CommandItem::Stacked {
            icon: Some(icon),
            label,
            detail,
        })
        .collect()
}

/// 视图；窗口锚点默认跟随高亮，无显式视口。
fn view<'a>(
    items: &'a [CommandItem<'a>],
    active: usize,
    anchor_row: u16,
    max_height: Option<u16>,
) -> CommandPanelView<'a> {
    view_window(items, active, None, None, anchor_row, max_height)
}

/// 指定窗口锚点的视图。
fn view_anchored<'a>(
    items: &'a [CommandItem<'a>],
    active: usize,
    window_anchor: Option<usize>,
    anchor_row: u16,
    max_height: Option<u16>,
) -> CommandPanelView<'a> {
    view_window(items, active, window_anchor, None, anchor_row, max_height)
}

/// 指定窗口锚点与显式视口的视图。
fn view_window<'a>(
    items: &'a [CommandItem<'a>],
    active: usize,
    window_anchor: Option<usize>,
    window_start: Option<usize>,
    anchor_row: u16,
    max_height: Option<u16>,
) -> CommandPanelView<'a> {
    CommandPanelView {
        items,
        active,
        window_anchor,
        window_start,
        anchor_row,
        max_height,
        title: None,
        right_title: None,
        footer: None,
    }
}

/// 带边框文案的视图：顶边左/右标题与底边提示。
fn view_titled<'a>(
    items: &'a [CommandItem<'a>],
    title: Option<&'a str>,
    right_title: Option<&'a str>,
    footer: Option<&'a str>,
) -> CommandPanelView<'a> {
    CommandPanelView {
        title,
        right_title,
        footer,
        ..view(items, 0, 0, None)
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
    assert_eq!(buf[(0, 3)].bg, Color::Indexed(236));
    assert_eq!(buf[(1, 5)].bg, Color::Indexed(236));
    assert_eq!(buf[(1, 5)].fg, Color::Indexed(252));
    assert_eq!(buf[(2, 4)].bg, Color::Reset);
    assert_eq!(buf[(2, 4)].fg, Color::Reset);
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
        .map(|(label, detail)| CommandItem::Stacked {
            icon: None,
            label,
            detail,
        })
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
fn window_stays_while_hovering_visible_item() {
    let area = Rect::new(0, 0, 24, 12);
    let items = stacked(&[
        ("f0", "s"),
        ("f1", "s"),
        ("f2", "s"),
        ("f3", "s"),
        ("f4", "s"),
        ("f5", "s"),
        ("f6", "s"),
        ("f7", "s"),
        ("f8", "s"),
        ("f9", "s"),
    ]);
    let anchored =
        layout(area, &view_anchored(&items, 9, Some(9), 0, Some(8))).expect("panel layout");
    assert_eq!(anchored.start, 7);
    assert_eq!(anchored.visible, 3);

    // 悬停窗口内的条目：锚点不动，窗口不滚动。
    let hovered =
        layout(area, &view_anchored(&items, 8, Some(9), 0, Some(8))).expect("panel layout");
    assert_eq!(hovered.start, 7);

    // 高亮落在锚点窗口之外（如列表变化）时回退为以高亮为底。
    let fallback =
        layout(area, &view_anchored(&items, 4, Some(9), 0, Some(8))).expect("panel layout");
    assert_eq!(fallback.start, 2);
}

#[test]
fn explicit_window_start_ignores_active_highlight() {
    let area = Rect::new(0, 0, 24, 12);
    let labels: Vec<(String, String)> = (0..10)
        .map(|index| (format!("file{index}.rs"), "src".to_string()))
        .collect();
    let items: Vec<CommandItem<'_>> = labels
        .iter()
        .map(|(label, detail)| CommandItem::Stacked {
            icon: None,
            label,
            detail,
        })
        .collect();

    // 显式视口起点优先于锚点：高亮第 0 项时窗口仍从第 2 项开始，高亮可滚出窗口。
    let mut buf = Buffer::empty(area);
    let view = view_window(&items, 0, Some(0), Some(2), 0, Some(8));
    let scrolled = layout(area, &view).expect("panel layout");
    assert_eq!(scrolled.start, 2);
    assert_eq!(scrolled.visible, 3);
    render(area, &mut buf, &view).expect("panel renders");
    assert!(row_text(&buf, area, 2).contains("file2.rs"));
    assert!(!row_text(&buf, area, 2).contains("file0.rs"));
    assert!(!buf[(3, 2)].modifier.contains(Modifier::REVERSED));

    // 视口起点越界时钳到贴底窗口，保证窗口填满预算。
    let bottom =
        layout(area, &view_window(&items, 0, Some(0), Some(99), 0, Some(8))).expect("panel layout");
    assert_eq!(bottom.start, 7);
    assert_eq!(bottom.visible, 3);
}

#[test]
fn renders_stacked_icon_muted_before_label() {
    let area = Rect::new(0, 0, 24, 8);
    let mut buf = Buffer::empty(area);
    let items = stacked_icons(&[("\u{f15c}", "app.rs", "src")]);
    let rect = render(area, &mut buf, &view(&items, 0, 0, None)).expect("panel renders");

    assert_eq!(rect.width, 12, "宽度计入图标与其后空格");
    assert_eq!(buf[(3, 2)].symbol(), "\u{f15c}");
    assert!(buf[(3, 2)].modifier.contains(Modifier::DIM));
    assert_eq!(buf[(4, 2)].symbol(), " ");
    assert_eq!(buf[(5, 2)].symbol(), "a");
    assert!(row_text(&buf, area, 2).contains("app.rs"));
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

#[test]
fn renders_titles_and_footer_on_border() {
    let area = Rect::new(0, 0, 40, 8);
    let mut buf = Buffer::empty(area);
    let items = stacked(&[("app.rs", "src")]);
    let rect = render(
        area,
        &mut buf,
        &view_titled(
            &items,
            Some("Files"),
            Some("src"),
            Some("[open ⇧↵] [back ^z]"),
        ),
    )
    .expect("panel renders");

    let top = row_text(&buf, area, rect.y);
    assert!(top.contains(" Files "), "{top}");
    assert!(top.contains(" src "), "{top}");
    assert!(
        top.find(" Files ").expect("left title") < top.find(" src ").expect("right title"),
        "左侧标题在右标题之前：{top}"
    );
    let bottom = row_text(&buf, area, rect.bottom() - 1);
    assert!(bottom.contains("[open ⇧↵] [back ^z]"), "{bottom}");
}

#[test]
fn panel_width_covers_footer_and_titles() {
    use unicode_width::UnicodeWidthStr;

    let area = Rect::new(0, 0, 60, 8);
    let footer = "[open ⇧↵] [back ^z]";
    let items = stacked(&[("a.rs", "s")]);
    let plain = layout(area, &view(&items, 0, 0, None)).expect("plain layout");
    let titled = layout(
        area,
        &view_titled(&items, Some("Files"), Some("src"), Some(footer)),
    )
    .expect("titled layout");

    assert!(titled.rect.width > plain.rect.width);
    assert!(
        usize::from(titled.rect.width) >= footer.width() + 4,
        "footer 必须完整可见: {} < {}",
        titled.rect.width,
        footer.width() + 4
    );
}

#[test]
fn clamps_titles_when_area_is_narrow() {
    let area = Rect::new(0, 0, 12, 6);
    let mut buf = Buffer::empty(area);
    let items = stacked(&[("a.rs", "s")]);
    let rect = render(
        area,
        &mut buf,
        &view_titled(
            &items,
            Some("Files"),
            Some("very-long-directory"),
            Some("[open ⇧↵] [back ^z]"),
        ),
    )
    .expect("panel renders");

    assert_eq!(rect.width, 12, "宽度钳到可用区域且不 panic");
}
