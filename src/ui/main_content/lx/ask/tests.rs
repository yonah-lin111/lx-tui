//! `ask` 面板几何与绘制：高度夹取、分栏/降级、命中与绘制一致、预览原样。

use super::*;
use crate::app::state::RequestKey;
use crate::chat::tools::ask::{AskOption, AskQuestion};

fn key() -> RequestKey {
    RequestKey {
        pane: crate::layout::PaneId::alloc(),
        session: "s".into(),
        generation: 1,
    }
}

fn option(label: &str, preview: Option<&str>) -> AskOption {
    AskOption {
        label: label.into(),
        description: format!("{label} description"),
        preview: preview.map(str::to_string),
    }
}

fn question(id: &str, multi: bool, options: Vec<AskOption>) -> AskQuestion {
    AskQuestion {
        id: id.into(),
        header: id.into(),
        question: format!("{id}?"),
        multi_select: multi,
        options,
    }
}

fn lx_with(questions: Vec<AskQuestion>) -> Lx {
    let mut lx = Lx::new();
    lx.ask = Some(AskState::new(key(), "call".into(), questions));
    lx
}

fn sample() -> Vec<AskQuestion> {
    vec![question(
        "layout",
        false,
        vec![
            option("two columns", Some("┌──┬──┐\n│a │b │\n└──┴──┘")),
            option("stacked", None),
        ],
    )]
}

fn area(width: u16, height: u16) -> Rect {
    Rect::new(0, 0, width, height)
}

#[test]
fn without_ask_the_input_box_keeps_its_rect() {
    let lx = Lx::new();
    let lay = super::super::layout(area(80, 40), &lx);
    assert!(lay.ask.is_none());
    assert!(lay.input.is_some(), "无 ask 时输入框必须原样保留");
}

#[test]
fn ask_replaces_the_input_box() {
    let lx = lx_with(sample());
    let lay = super::super::layout(area(80, 40), &lx);
    assert!(lay.input.is_none(), "ask 激活时输入框必须让位");
    let rect = lay.ask.expect("ask rect");
    // 与输入框同宽（外边距 1），下沿贴窗格底部。
    assert_eq!(rect.width, 78);
    assert_eq!(rect.bottom(), 40);
}

#[test]
fn height_never_exceeds_seventy_percent_and_stays_at_least_ten() {
    for (width, height) in [(80, 40), (60, 24), (50, 20), (30, 14)] {
        let lx = lx_with(sample());
        let rect = rect(area(width, height), &lx).expect("ask rect");
        let cap = (height * 70 / 100).max(10);
        assert!(
            rect.height <= cap,
            "{width}x{height}: height {} over cap {cap}",
            rect.height
        );
        assert!(rect.height >= 10.min(height));
    }
}

#[test]
fn tiny_pane_degrades_and_hides_preview() {
    let lx = lx_with(sample());
    let small = area(80, 11);
    let lay = super::super::layout(small, &lx);
    let rect = lay.ask.expect("ask rect");
    let computed = AskLayout::compute(rect, lx.ask.as_ref().expect("ask"));
    assert!(computed.preview.is_none(), "矮窗格不画预览");
    assert!(computed.preview_hidden);
}

#[test]
fn wide_pane_splits_options_and_preview_side_by_side() {
    let lx = lx_with(sample());
    let rect = rect(area(80, 30), &lx).expect("ask rect");
    let lay = AskLayout::compute(rect, lx.ask.as_ref().expect("ask"));
    let preview = lay.preview.expect("wide pane keeps preview");
    assert!(
        preview.x > lay.options.x,
        "宽窗格必须左右分栏（preview 在选项列右侧）"
    );
    assert!(lay.options.width >= OPTIONS_MIN_COLS);
}

#[test]
fn narrow_pane_stacks_preview_below_options() {
    let lx = lx_with(sample());
    let rect = rect(area(50, 30), &lx).expect("ask rect");
    let lay = AskLayout::compute(rect, lx.ask.as_ref().expect("ask"));
    let preview = lay.preview.expect("stacked preview");
    assert!(preview.y > lay.options.y, "窄窗格上下排");
    assert_eq!(preview.x, lay.options.x);
}

#[test]
fn hit_matches_every_clickable_row() {
    let lx = lx_with(sample());
    let rect = rect(area(80, 30), &lx).expect("ask rect");
    let lay = AskLayout::compute(rect, lx.ask.as_ref().expect("ask"));
    assert!(!lay.option_rows.is_empty());
    for (expected, target) in &lay.option_rows {
        let hit = hit(area(80, 30), &lx, target.x, target.y);
        assert_eq!(hit, Some(*expected), "行 {expected:?} 命中不一致");
    }
    // 面板内、但不在可点击行上（例如预览区）：命中为 None，交给外框吞掉。
    let preview = lay.preview.expect("preview");
    assert_eq!(hit(area(80, 30), &lx, preview.x + 1, preview.y), None);
}

#[test]
fn multi_question_tabs_are_clickable() {
    let lx = lx_with(vec![
        question("layout", false, vec![option("a", None)]),
        question("theme", false, vec![option("b", None)]),
    ]);
    let rect = rect(area(80, 30), &lx).expect("ask rect");
    let lay = AskLayout::compute(rect, lx.ask.as_ref().expect("ask"));
    assert_eq!(lay.tabs.len(), 2);
    let (_, second) = lay.tabs[1];
    assert_eq!(
        hit(area(80, 30), &lx, second.x + 1, second.y),
        Some(AskHit::Tab(1))
    );
}

#[test]
fn drawing_renders_title_options_and_preview_verbatim() {
    let lx = lx_with(sample());
    let surface = area(80, 30);
    let rect = rect(surface, &lx).expect("ask rect");
    let mut grid = Grid::new(surface);
    let _ = draw(&mut grid, rect, &lx, true);
    let row = |y: u16| -> String {
        (surface.x..surface.right())
            .map(|x| grid.cell(x, y).ch)
            .collect()
    };
    let title = row(rect.y + 1);
    assert!(title.contains("ASK"), "标题行缺少 ASK：{title:?}");
    let body: String = (rect.y + 2..rect.bottom() - 1).map(row).collect();
    assert!(body.contains("two columns"), "选项标签缺失");
    assert!(body.contains("[ Submit ]"), "提交行缺失");
    let preview = AskLayout::compute(rect, lx.ask.as_ref().expect("ask"))
        .preview
        .expect("preview");
    let preview_row = row(preview.y);
    let marker = preview_row.find('┌').expect("预览首行必须原样绘制");
    assert!(marker >= usize::from(preview.x), "预览越出预览区左缘");
    assert!(preview_row.contains("┌──┬──┐"), "预览内容被改写");
}

#[test]
fn preview_keeps_leading_spaces_and_expands_tabs() {
    assert_eq!(expand_tabs("a\tb"), "a    b");
    assert_eq!(expand_tabs("  x"), "  x");
    let lx = lx_with(vec![question(
        "sketch",
        false,
        vec![option("box", Some("    indented"))],
    )]);
    let surface = area(80, 30);
    let rect = rect(surface, &lx).expect("ask rect");
    let mut grid = Grid::new(surface);
    let _ = draw(&mut grid, rect, &lx, true);
    let lay = AskLayout::compute(rect, lx.ask.as_ref().expect("ask"));
    let preview = lay.preview.expect("preview");
    let text: String = (preview.x..preview.right())
        .map(|x| grid.cell(x, preview.y).ch)
        .collect();
    assert!(text.contains("    indented"), "前导空格必须保留：{text:?}");
}

#[test]
fn editing_returns_cursor_inside_the_editor_box() {
    let mut lx = lx_with(sample());
    {
        let ask = lx.ask.as_mut().expect("ask");
        ask.begin_edit();
        ask.editor_mut().expect("editor").insert_str("custom");
    }
    let surface = area(80, 30);
    let rect = rect(surface, &lx).expect("ask rect");
    let mut grid = Grid::new(surface);
    let (row, column) = draw(&mut grid, rect, &lx, true).expect("编辑态必须返回光标");
    let lay = AskLayout::compute(rect, lx.ask.as_ref().expect("ask"));
    let editor = lay.editor.expect("editor rect");
    assert!(
        editor.contains((column, row).into()),
        "光标 {row},{column} 必须落在编辑框内"
    );
    // 未聚焦不画光标。
    assert!(draw(&mut grid, rect, &lx, false).is_none());
}

#[test]
fn wheel_zone_splits_preview_and_options() {
    let lx = lx_with(sample());
    let surface = area(80, 30);
    let rect = rect(surface, &lx).expect("ask rect");
    let lay = AskLayout::compute(rect, lx.ask.as_ref().expect("ask"));
    let preview = lay.preview.expect("preview");
    assert_eq!(
        wheel_zone(surface, &lx, preview.x + 1, preview.y),
        Some(AskWheelZone::Preview)
    );
    assert_eq!(
        wheel_zone(surface, &lx, lay.options.x + 1, lay.options.y),
        Some(AskWheelZone::Options)
    );
    assert_eq!(wheel_zone(surface, &lx, 0, 0), None);
}
