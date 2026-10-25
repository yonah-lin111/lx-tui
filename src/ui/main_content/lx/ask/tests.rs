//! `ask` 面板几何与绘制：无边框、70% 高度上限与滚动、双行选项、tab 常驻、命中一致。

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

fn option(label: &str, description: &str, preview: Option<&str>) -> AskOption {
    AskOption {
        label: label.into(),
        description: description.into(),
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
            option(
                "two columns",
                "left list, right preview",
                Some("┌──┬──┐\n│a │b │\n└──┴──┘"),
            ),
            option("stacked", "top down", None),
        ],
    )]
}

fn surface(width: u16, height: u16) -> Rect {
    Rect::new(0, 0, width, height)
}

fn computed(lx: &Lx, area: Rect) -> AskLayout {
    let rect = rect(area, lx).expect("ask rect");
    AskLayout::compute(rect, area.height, lx.ask.as_ref().expect("ask"))
}

fn row_text(grid: &Grid, area: Rect, y: u16) -> String {
    (area.x..area.right())
        .map(|x| {
            let ch = grid.cell(x, y).ch;
            if ch == '\0' { ' ' } else { ch }
        })
        .collect()
}

#[test]
fn without_ask_the_input_box_keeps_its_rect() {
    let lx = Lx::new();
    let lay = super::super::layout(surface(80, 40), &lx);
    assert!(lay.ask.is_none());
    assert!(lay.input.is_some(), "无 ask 时输入框必须原样保留");
}

#[test]
fn ask_replaces_the_input_box() {
    let lx = lx_with(sample());
    let lay = super::super::layout(surface(80, 40), &lx);
    assert!(lay.input.is_none(), "ask 激活时输入框必须让位");
    let rect = lay.ask.expect("ask rect");
    assert_eq!(rect.width, 78);
    assert_eq!(rect.bottom(), 40);
}

#[test]
fn height_never_exceeds_seventy_percent() {
    // 70% 是上限；唯一例外是「最小可用高度」（分隔线 + 一行可选行 + 底栏），
    // 极矮窗格里宁可多占一行，也不留一个可见却选不了的死面板。
    for (width, height) in [(80, 40), (60, 24), (50, 20), (30, 14), (80, 8)] {
        let lx = lx_with(sample());
        let rect = rect(surface(width, height), &lx).expect("ask rect");
        let cap = (height * 70 / 100).max(1).max(3.min(height));
        assert!(
            rect.height <= cap.min(height),
            "{width}x{height}: height {} 超过上限 {cap}",
            rect.height
        );
        assert!(rect.height >= 1);
    }
}

#[test]
fn tiny_pane_keeps_at_least_one_selectable_row() {
    // 8 行窗格：70% = 5 行，扣掉分隔线 + 底栏只剩 3 行可用；题干让位后
    // 列表必须有落位的可选行，否则按键被接管却什么都选不了。
    let lx = lx_with(sample());
    let area = surface(80, 8);
    let lay = computed(&lx, area);
    assert!(
        !lay.rows.is_empty(),
        "极矮窗格必须至少落位一行可选行（面板 {} 行）",
        lay.options.height
    );
    let row = lay.rows.first().expect("row");
    assert_eq!(
        hit(area, &lx, row.rect.x + 1, row.rect.y),
        row.hit,
        "落位的行必须可命中"
    );
}

#[test]
fn tabs_are_always_visible_in_short_panes() {
    let lx = lx_with(vec![
        question("layout", false, vec![option("a", "d", None)]),
        question("theme", false, vec![option("b", "d", None)]),
    ]);
    // 14 行窗格 → 面板最多 9 行；tab 行仍然必须存在（含 Confirm）。
    let lay = computed(&lx, surface(80, 14));
    assert_eq!(lay.tabs.len(), 3, "两题 + Confirm 的 tab 必须常驻");
    assert_eq!(lay.tabs.last().map(|(index, _)| *index), Some(2));
}

#[test]
fn options_use_two_lines_with_description_below() {
    let lx = lx_with(sample());
    let area = surface(80, 30);
    let lay = computed(&lx, area);
    let kinds: Vec<&RowKind> = lay.rows.iter().map(|row| &row.kind).collect();
    assert!(matches!(kinds.first(), Some(RowKind::OptionLabel(0))));
    assert!(matches!(kinds.get(1), Some(RowKind::OptionDesc(0))));
    // 描述行与标签行同属选项 0：点描述也算选中该选项。
    let desc = &lay.rows[1];
    assert_eq!(desc.hit, Some(AskHit::Option(0)));
    assert_eq!(
        hit(area, &lx, desc.rect.x + 2, desc.rect.y),
        Some(AskHit::Option(0))
    );
    // 描述缩进到标签列（左侧括号只在第一行）。
    let mut grid = Grid::new(area);
    let _ = draw(
        &mut grid,
        rect(area, &lx).expect("ask rect"),
        area.height,
        &lx,
        true,
    );
    let label_row = row_text(&grid, area, lay.rows[0].rect.y);
    let desc_row = row_text(&grid, area, lay.rows[1].rect.y);
    assert!(
        label_row.contains("( ) two columns"),
        "标签行：{label_row:?}"
    );
    assert!(
        !desc_row.contains('('),
        "描述行左侧不得出现括号：{desc_row:?}"
    );
    assert!(label_row.find("two columns").is_some());
    let rect = rect(area, &lx).expect("ask rect");
    assert_eq!(
        desc_row.find("left list"),
        Some(usize::from(rect.x + ROW_PREFIX)),
        "描述必须缩进到标签列"
    );
}

#[test]
fn no_border_only_a_top_rule() {
    let lx = lx_with(sample());
    let area = surface(80, 30);
    let rect = rect(area, &lx).expect("ask rect");
    let mut grid = Grid::new(area);
    let _ = draw(&mut grid, rect, area.height, &lx, true);
    let rule = row_text(&grid, area, rect.y);
    let rule: String = rule
        .chars()
        .skip(usize::from(rect.x))
        .take(usize::from(rect.width))
        .collect();
    assert!(
        rule.chars().all(|ch| ch == '─'),
        "顶部必须是一条通栏分隔线：{rule:?}"
    );
    // 左右两侧与底部不得有边框字符。
    for y in rect.y + 1..rect.bottom() {
        assert_ne!(grid.cell(rect.x, y).ch, '│', "y={y} 左侧不应有竖边框");
        assert_ne!(
            grid.cell(rect.right() - 1, y).ch,
            '│',
            "y={y} 右侧不应有竖边框"
        );
    }
}

#[test]
fn wide_pane_splits_preview_and_keeps_it_verbatim() {
    let lx = lx_with(sample());
    let area = surface(80, 30);
    let lay = computed(&lx, area);
    let preview = lay.preview.expect("wide pane keeps preview");
    assert!(preview.x > lay.options.x, "宽窗格左右分栏");
    let mut grid = Grid::new(area);
    let _ = draw(
        &mut grid,
        rect(area, &lx).expect("ask rect"),
        area.height,
        &lx,
        true,
    );
    let first = row_text(&grid, area, preview.y);
    assert!(first.contains("┌──┬──┐"), "预览必须原样：{first:?}");
}

#[test]
fn narrow_pane_stacks_preview_below_options() {
    let lx = lx_with(sample());
    let lay = computed(&lx, surface(50, 30));
    let preview = lay.preview.expect("stacked preview");
    assert!(preview.y > lay.options.y);
    assert_eq!(preview.x, lay.options.x);
}

#[test]
fn scrolling_follows_the_highlight_and_keeps_every_row_hittable() {
    let mut q = question(
        "q",
        false,
        vec![
            option("a", "da", None),
            option("b", "db", None),
            option("c", "dc", None),
            option("d", "dd", None),
        ],
    );
    q.question = "很长的题干 ".repeat(40);
    let mut lx = lx_with(vec![q]);
    let area = surface(80, 16);
    {
        let ask = lx.ask.as_mut().expect("ask");
        ask.set_highlight(3);
    }
    let lay = computed(&lx, area);
    assert!(
        lay.content_rows > usize::from(lay.options.height),
        "内容必须超出可视区（否则测不到滚动）"
    );
    assert!(lay.scroll > 0, "高亮末项必须把列表向下滚动");
    for row in &lay.rows {
        if let Some(expected) = row.hit {
            assert_eq!(
                hit(area, &lx, row.rect.x + 1, row.rect.y),
                Some(expected),
                "滚动后命中必须与绘制一致"
            );
        }
    }
}

#[test]
fn submit_row_stays_visible_when_the_other_row_is_highlighted() {
    // 单题 4 个带描述的选项 + 题干 → 内容超出一屏；高亮停在「其他」时，
    // 紧跟其后的 Submit 必须仍落在可视区（否则纯鼠标用户永远点不到提交）。
    let options = (0..4)
        .map(|index| option(&format!("o{index}"), "desc", None))
        .collect();
    let mut lx = lx_with(vec![question("q", false, options)]);
    {
        let ask = lx.ask.as_mut().expect("ask");
        let rows = ask.row_count();
        ask.set_highlight(rows - 1);
        assert_eq!(ask.highlight[0], 4, "高亮「其他」");
    }
    let area = surface(80, 16);
    let lay = computed(&lx, area);
    assert!(
        lay.content_rows > usize::from(lay.options.height),
        "必须溢出"
    );
    assert!(
        lay.rows.iter().any(|row| row.hit == Some(AskHit::Submit)),
        "Submit 行必须留在可视区"
    );
}

#[test]
fn confirm_tab_renders_review_rows_and_commit() {
    let mut lx = lx_with(vec![
        question("layout", false, vec![option("a", "da", None)]),
        question("theme", false, vec![option("b", "db", None)]),
    ]);
    {
        let ask = lx.ask.as_mut().expect("ask");
        ask.goto_tab(2);
        assert!(ask.is_confirm());
    }
    let area = surface(80, 30);
    let lay = computed(&lx, area);
    assert!(lay.preview.is_none(), "Confirm 步没有预览");
    assert!(
        lay.rows
            .iter()
            .any(|row| matches!(row.kind, RowKind::Review(1)))
    );
    assert!(lay.rows.iter().any(|row| row.hit == Some(AskHit::Submit)));
    let (index, tab) = *lay.tabs.last().expect("confirm tab");
    assert_eq!(index, 2);
    assert_eq!(hit(area, &lx, tab.x + 1, tab.y), Some(AskHit::Tab(2)));
}

#[test]
fn editing_returns_cursor_inside_the_editor_area() {
    let mut lx = lx_with(sample());
    {
        let ask = lx.ask.as_mut().expect("ask");
        ask.begin_edit();
        ask.editor_mut().expect("editor").insert_str("custom");
    }
    let area = surface(80, 30);
    let (row, column) = {
        let mut grid = Grid::new(area);
        draw(
            &mut grid,
            rect(area, &lx).expect("ask rect"),
            area.height,
            &lx,
            true,
        )
        .expect("编辑态光标")
    };
    let lay = computed(&lx, area);
    let editor = lay.editor.expect("editor rect");
    assert!(
        editor.contains((column, row).into()),
        "光标 {row},{column} 必须落在编辑区内"
    );
    // 未聚焦不画光标。
    let mut grid = Grid::new(area);
    assert!(
        draw(
            &mut grid,
            rect(area, &lx).expect("ask rect"),
            area.height,
            &lx,
            false
        )
        .is_none()
    );
}

#[test]
fn wheel_zone_splits_preview_and_options() {
    let lx = lx_with(sample());
    let area = surface(80, 30);
    let lay = computed(&lx, area);
    let preview = lay.preview.expect("preview");
    assert_eq!(
        wheel_zone(area, &lx, preview.x + 1, preview.y),
        Some(AskWheelZone::Preview)
    );
    assert_eq!(
        wheel_zone(area, &lx, lay.options.x + 1, lay.options.y),
        Some(AskWheelZone::Options)
    );
    assert_eq!(wheel_zone(area, &lx, 0, 0), None);
}
