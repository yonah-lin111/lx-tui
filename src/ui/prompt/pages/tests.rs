//! 页列表渲染与头部单元测试。

use super::*;
use crate::app::prompt::{PromptEditing, PromptItem, PromptMode, PromptPage, PromptPageId};
use crate::ui::prompt::{PageHeaderData, TitleRowData, paint_title_row, toolbar_page_controls};
use ratatui::style::{Color, Modifier};

fn demo_panel() -> PromptPanel {
    let mut panel = PromptPanel::new();
    let mut item = PromptItem::single(
        1,
        None,
        "MyItem",
        "&&& addTemplate --start\n{title: Todo Page}\n&&& addTemplate --end",
    );
    item.pages.push(PromptPage::saved(
        2,
        "Page 2",
        "&&& bugTemplate --start\n{title: Running Page}\n&&& bugTemplate --end in_progress",
    ));
    item.pages.push(PromptPage::saved(
        3,
        "Page 3",
        "&&& refactorTemplate --start\n{title: Done Page}\n&&& refactorTemplate --end done",
    ));
    item.pages
        .push(PromptPage::saved(4, "Plain Page", "no template block"));
    item.active_page = PromptPageId::Saved(2);
    panel.tree.replace(vec![], vec![item]);
    panel.editing = Some(PromptEditing::Item(1));
    panel.mode = PromptMode::Pages;
    panel.pages_view.selected = 1;
    panel
}

fn row_string(buf: &Buffer, area: Rect, y: u16) -> String {
    (area.x..area.right())
        .map(|x| buf[(x, y)].symbol())
        .collect()
}

#[test]
fn renders_status_dots_current_bold_and_selected_marker() {
    let panel = demo_panel();
    let area = Rect::new(0, 0, 36, 10);
    let mut buf = Buffer::empty(area);
    render(area, &mut buf, &panel, true, None);

    // y=2: Todo Page (todo -> ○ dim)
    let r0 = row_string(&buf, area, 2);
    assert!(r0.starts_with("  Todo Page"), "{r0:?}");
    assert_eq!(buf[(area.right() - 1, 2)].symbol(), "○");
    assert!(buf[(area.right() - 1, 2)].modifier.contains(Modifier::DIM));
    assert!(!buf[(2, 2)].modifier.contains(Modifier::BOLD));

    // y=3: Running Page (current + selected -> ❯ + bold + yellow ●)
    let r1 = row_string(&buf, area, 3);
    assert!(r1.starts_with("❯ Running Page"), "{r1:?}");
    assert!(buf[(2, 3)].modifier.contains(Modifier::BOLD), "当前页加粗");
    assert_eq!(buf[(area.right() - 1, 3)].symbol(), "●");
    assert_eq!(buf[(area.right() - 1, 3)].fg, Color::Yellow);

    // y=4: Done Page (done -> green ●)
    let r2 = row_string(&buf, area, 4);
    assert!(r2.starts_with("  Done Page"), "{r2:?}");
    assert_eq!(buf[(area.right() - 1, 4)].symbol(), "●");
    assert_eq!(buf[(area.right() - 1, 4)].fg, Color::Green);

    // y=5: Plain Page (无 &&& -> 不画圆点)
    let r3 = row_string(&buf, area, 5);
    assert!(r3.starts_with("  Plain Page"), "{r3:?}");
    assert_eq!(buf[(area.right() - 1, 5)].symbol(), " ");
}

#[test]
fn long_title_ellipsizes_while_preserving_status_dot() {
    let mut panel = demo_panel();
    panel.tree.item_mut(1).unwrap().pages[0].content =
        "&&& addTemplate --start\n{title: Very Long Title That Exceeds Width}\n&&& addTemplate --end in_progress"
            .into();
    let area = Rect::new(0, 0, 18, 6);
    let mut buf = Buffer::empty(area);
    render(area, &mut buf, &panel, true, None);

    let r0 = row_string(&buf, area, 2);
    assert!(r0.contains('…'), "长标题应以省略号截断: {r0:?}");
    assert_eq!(
        buf[(area.right() - 1, 2)].symbol(),
        "●",
        "截断后最右列状态圆点仍保留: {r0:?}"
    );
}

#[test]
fn title_row_in_pages_mode_shows_pages_label_without_controls() {
    let panel = Rect::new(0, 0, 40, 8);
    let mut buf = Buffer::empty(panel);
    let block = ratatui::widgets::Block::bordered();
    ratatui::widgets::Widget::render(block, panel, &mut buf);
    let header = crate::layout::prompt_header_rect(panel).unwrap();
    super::super::render_divider(panel, header, &mut buf, false);
    let row = crate::ui::prompt_frame::title_row(panel).unwrap();
    let data = TitleRowData {
        title: Some("MyItem"),
        draft: false,
        page: Some(PageHeaderData {
            current: 3,
            total: 4,
            name: text::PROMPT_PAGES_LABEL,
            arrows: false,
        }),
    };
    paint_title_row(&mut buf, row, &data, None);
    let rendered: String = (panel.x..panel.right())
        .map(|x| buf[(x, row.y)].symbol())
        .collect();
    assert!(rendered.contains("MyItem"), "{rendered:?}");
    assert!(rendered.contains("· pages"), "{rendered:?}");
    assert!(!rendered.contains("3/4"), "页码已移到工具栏: {rendered:?}");
    assert!(
        !rendered.contains(text::PROMPT_PAGE_PREV) && !rendered.contains(text::PROMPT_PAGE_NEXT),
        "Pages 视图标题行不画 ‹ / ›: {rendered:?}"
    );

    // 工具栏仍提供无箭头 `n/m` 计数。
    let bar = crate::layout::prompt_toolbar_rect(panel).expect("toolbar");
    let controls = toolbar_page_controls(bar, "3/4", false).expect("counter visible");
    assert_eq!(controls.counter.width, 3);
    assert_eq!(controls.prev.width, 0);
    assert_eq!(controls.next.width, 0);
}

#[test]
fn narrow_width_renders_without_panic_or_half_wide_chars() {
    let mut panel = demo_panel();
    panel.tree.item_mut(1).unwrap().pages[0].content =
        "&&& addTemplate --start\n{title: 中文宽字符标题测试}\n&&& addTemplate --end".into();
    let area = Rect::new(0, 0, 12, 8);
    let mut buf = Buffer::empty(area);
    render(area, &mut buf, &panel, true, Some(0));
    assert_eq!(page_row_at(area, &panel, 2, 2), Some(0));
    assert_eq!(page_row_at(area, &panel, 2, 0), None);
}
