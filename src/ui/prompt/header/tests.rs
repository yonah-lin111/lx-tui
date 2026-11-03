//! 顶部表头与标题行单元测试：标题行页名状态着色、工具栏页控件命中与窄屏避让。

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier};

use super::*;
use crate::layout::{PromptToolbarButton, prompt_toolbar_button_rect, prompt_toolbar_rect};

/// 标题行渲染工具：先画直角边框与分割线，再把标题覆盖到分割线行，
/// 返回标题行整行字符串（含两端 `├` / `┤`）。
fn title_row_render(panel: Rect, data: &TitleRowData<'_>) -> String {
    title_row_render_with_status(panel, data, None)
}

/// 同上，附加本页模板块状态。
fn title_row_render_with_status(
    panel: Rect,
    data: &TitleRowData<'_>,
    status: Option<TemplateStatus>,
) -> String {
    let mut buf = Buffer::empty(panel);
    let block = ratatui::widgets::Block::bordered();
    ratatui::widgets::Widget::render(block, panel, &mut buf);
    if let Some(header) = crate::layout::prompt_header_rect(panel) {
        render_divider(panel, header, &mut buf, false);
    }
    let row = crate::ui::prompt_frame::title_row(panel).expect("title row");
    paint_title_row(&mut buf, row, data, status);
    (panel.x..panel.right())
        .map(|x| buf[(x, row.y)].symbol())
        .collect()
}

/// 页数据简写。
fn page_data(current: usize, total: usize, name: &str) -> PageHeaderData<'_> {
    PageHeaderData {
        current,
        total,
        name,
        arrows: true,
    }
}

#[test]
fn title_row_shows_item_name_and_page_title_only() {
    let panel = Rect::new(0, 0, 56, 8);
    let row = title_row_render(
        panel,
        &TitleRowData {
            title: Some("named prompt"),
            draft: false,
            page: Some(page_data(2, 4, "Intro")),
        },
    );
    assert!(row.contains(" named prompt "), "{row}");
    assert!(row.contains("· Intro"), "页名跟随条目名: {row}");
    assert!(!row.contains("2/4"), "页码已移到工具栏: {row}");
    assert!(
        !row.contains('‹') && !row.contains('›'),
        "标题行无箭头: {row}"
    );
    // 落在分割线行上：两端以 `├` / `┤` 衔接边框。
    assert!(row.starts_with(text::DIVIDER_LEFT_JOIN), "{row}");
    assert!(row.ends_with(text::DIVIDER_RIGHT_JOIN), "{row}");
}

#[test]
fn toolbar_page_control_hit_areas() {
    let panel = Rect::new(0, 0, 56, 8);
    let bar = prompt_toolbar_rect(panel).expect("toolbar");
    let data = TitleRowData {
        title: Some("named prompt"),
        draft: false,
        page: Some(page_data(2, 4, "Intro")),
    };
    let controls = toolbar_page_controls(bar, "2/4", true).expect("controls visible");
    for (rect, expected) in [
        (controls.prev, PageControl::Prev),
        (controls.counter, PageControl::Counter),
        (controls.next, PageControl::Next),
    ] {
        assert_eq!(page_control_at(bar, &data, rect.x, bar.y), Some(expected));
    }
    // 标题行页名为纯展示；工具栏左侧空白也不产生控件命中。
    let title = crate::ui::prompt_frame::title_row(panel).expect("title row");
    assert_eq!(
        page_control_at(bar, &data, bar.x + 3, bar.y),
        None,
        "左侧按钮区不响应页控件"
    );
    assert_eq!(
        page_control_at(bar, &data, controls.prev.x, title.y),
        None,
        "非工具栏行不响应"
    );
}

#[test]
fn toolbar_page_controls_degrade_when_narrow() {
    let narrow = prompt_toolbar_rect(Rect::new(0, 0, 12, 8)).expect("toolbar");
    assert!(
        toolbar_page_controls(narrow, "1/2", true).is_none(),
        "放不下整组时隐藏"
    );
    let wide = prompt_toolbar_rect(Rect::new(0, 0, 56, 8)).expect("toolbar");
    assert!(toolbar_page_controls(wide, "1/2", true).is_some());
    assert!(
        toolbar_page_controls(wide, "999/1000", true).is_some(),
        "页码变宽仍完整容纳"
    );
}

#[test]
fn toolbar_buttons_never_overlap_page_controls_across_widths() {
    for width in 6..=48 {
        let panel = Rect::new(0, 0, width, 8);
        let Some(bar) = prompt_toolbar_rect(panel) else {
            continue;
        };
        for (mode, arrows) in [(PromptMode::Editor, true), (PromptMode::Pages, false)] {
            for counter in ["1/2", "99/100"] {
                let Some(controls) = toolbar_page_controls(bar, counter, arrows) else {
                    continue;
                };
                let page_left = if arrows {
                    controls.prev.x
                } else {
                    controls.counter.x
                };
                for button in [
                    PromptToolbarButton::Mode,
                    PromptToolbarButton::Recent,
                    PromptToolbarButton::Undo,
                    PromptToolbarButton::Redo,
                    PromptToolbarButton::Folders,
                ] {
                    if let Some(rect) =
                        prompt_toolbar_button_rect(panel, button, mode, Some(counter))
                    {
                        assert!(
                            rect.right() < page_left,
                            "width={width} mode={mode:?} counter={counter} button={button:?} rect={rect:?} 与页控件左缘 {page_left} 冲突"
                        );
                    }
                }
            }
        }
    }
}

#[test]
fn title_row_degrades_page_name_before_item_name() {
    // 宽面板：条目名完整，超长页名被截断。
    let wide = Rect::new(0, 0, 56, 8);
    let long_name = "y".repeat(80);
    let row = title_row_render(
        wide,
        &TitleRowData {
            title: Some("item"),
            draft: false,
            page: Some(page_data(1, 3, &long_name)),
        },
    );
    assert!(row.contains(" item "), "条目名优先完整: {row}");
    assert!(!row.contains(&long_name), "页名被截断: {row}");
    assert!(row.contains('…'), "页名以省略号收尾: {row}");

    // 极窄：只剩截断的条目名，两端衔接符不被压到。
    let narrow = Rect::new(0, 0, 10, 8);
    let row = title_row_render(
        narrow,
        &TitleRowData {
            title: Some("a-very-long-item-name"),
            draft: false,
            page: Some(page_data(1, 3, "Page 1")),
        },
    );
    assert!(row.starts_with(text::DIVIDER_LEFT_JOIN), "{row}");
    assert!(row.ends_with(text::DIVIDER_RIGHT_JOIN), "{row}");
    assert!(!row.contains("1/3"), "标题行不承载页码: {row}");
    assert!(row.contains('…'), "条目名截断: {row}");
}

#[test]
fn title_row_draft_uses_muted_style() {
    let panel = Rect::new(0, 0, 56, 8);
    let border = crate::ui::prompt_frame::title_row(panel).expect("title row");

    // 临时提示词：标题用未聚焦灰（显式前景 + dim），不被聚焦边框染色。
    let mut buf = Buffer::empty(panel);
    paint_title_row(
        &mut buf,
        border,
        &TitleRowData {
            title: Some(text::PROMPT_DRAFT_TITLE),
            draft: true,
            page: Some(page_data(1, 1, "Page 1")),
        },
        None,
    );
    let cell = &buf[(border.x + 2, border.y)];
    assert_eq!(cell.fg, Color::Reset, "草稿标题显式重置前景");
    assert!(cell.modifier.contains(Modifier::DIM));
}

#[test]
fn title_row_colors_page_title_by_status_and_omits_status_button() {
    let panel = Rect::new(0, 0, 56, 8);
    let border = crate::ui::prompt_frame::title_row(panel).expect("title row");
    let data = TitleRowData {
        title: Some("item"),
        draft: false,
        page: Some(page_data(2, 4, "Page 2")),
    };

    let mut buf = Buffer::empty(panel);
    paint_title_row(&mut buf, border, &data, Some(TemplateStatus::InProgress));
    let row: String = (panel.x..panel.right())
        .map(|x| buf[(x, border.y)].symbol())
        .collect();
    assert!(
        !row.contains("[run]") && !row.contains("[todo]") && !row.contains("[done]"),
        "标题行不再绘制 [todo]/[run]/[done] 状态按钮: {row}"
    );
    let hit = status_rect(border, &data).expect("page title status rect");
    assert_eq!(hit.width, 6, "命中区精确包裹 `Page 2`");
    assert_eq!(
        buf[(hit.x, border.y)].fg,
        Color::Yellow,
        "in_progress 页名黄色"
    );
    assert!(
        !buf[(hit.x, border.y)].modifier.contains(Modifier::DIM),
        "状态着色页名不继承分割线 dim"
    );

    // 三态各自颜色：todo 默认亮白、done 绿（对齐 recent 页状态圆点）。
    for (status, color) in [
        (TemplateStatus::Todo, Color::Reset),
        (TemplateStatus::Done, Color::Green),
    ] {
        let mut buf = Buffer::empty(panel);
        paint_title_row(&mut buf, border, &data, Some(status));
        assert_eq!(buf[(hit.x, border.y)].fg, color, "{status:?} 页名颜色");
        assert!(!buf[(hit.x, border.y)].modifier.contains(Modifier::DIM));
    }

    // 无模板块：页名为暗灰 muted_on_border。
    let mut buf = Buffer::empty(panel);
    paint_title_row(&mut buf, border, &data, None);
    assert_eq!(buf[(hit.x, border.y)].fg, Color::Reset);
    assert!(buf[(hit.x, border.y)].modifier.contains(Modifier::DIM));

    // 极窄挤掉页名时：status_rect 为 None。
    let narrow = Rect::new(0, 0, 10, 8);
    let narrow_border = crate::ui::prompt_frame::title_row(narrow).expect("title row");
    assert!(
        status_rect(narrow_border, &data).is_none(),
        "页名被挤掉时无状态命中区"
    );
}

#[test]
fn tree_view_title_row_is_plain() {
    let panel = Rect::new(0, 0, 40, 8);
    let border = crate::ui::prompt_frame::title_row(panel).expect("title row");
    let data = TitleRowData {
        title: None,
        draft: false,
        page: None,
    };
    let row = title_row_render(panel, &data);
    assert!(row.starts_with(text::DIVIDER_LEFT_JOIN), "{row}");
    assert!(row.ends_with(text::DIVIDER_RIGHT_JOIN), "{row}");
    assert!(
        row.chars()
            .skip(1)
            .take(usize::from(panel.width) - 2)
            .all(|ch| ch == text::DIVIDER_MID.chars().next().unwrap_or('─')),
        "树视图标题行为纯 `─`: {row}"
    );
    assert!(
        !row.contains('‹') && !row.contains('›'),
        "列表页无页控件: {row}"
    );
    assert_eq!(
        page_control_at(border, &data, border.x + 3, border.y),
        None,
        "树视图标题行无任何页控件命中"
    );
}
