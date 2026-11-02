//! `ask` 面板几何与绘制：无边框、70% 高度上限与滚动、描述完整折行、预览置底、
//! 原地 Other 输入与弹窗胶囊提交按钮。

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
    AskLayout::compute(rect, lx.ask.as_ref().expect("ask"))
}

fn row_text(grid: &Grid, area: Rect, y: u16) -> String {
    (area.x..area.right())
        .map(|x| {
            let ch = grid.cell(x, y).ch;
            if ch == '\0' { ' ' } else { ch }
        })
        .collect()
}

fn paint(lx: &Lx, area: Rect) -> Grid {
    let mut grid = Grid::new(area);
    let _ = draw(
        &mut grid,
        rect(area, lx).expect("ask rect"),
        area.height,
        lx,
        true,
    );
    grid
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
    // 14 行窗格 → 面板最多 9 行；tab 行仍然必须存在（两题 + Submit 汇总）。
    let lay = computed(&lx, surface(80, 14));
    assert_eq!(lay.tabs.len(), 3, "两题 + Submit 汇总");
    assert_eq!(lay.tabs.last().map(|(index, _)| *index), Some(2));
    assert_eq!(
        tab_label(lx.ask.as_ref().expect("ask"), 2),
        a::ASK_TAB_SUBMIT
    );
}

#[test]
fn preview_lines_are_rows_of_the_single_scrollable_list() {
    let lx = lx_with(sample());
    let area = surface(80, 30);
    let panel = rect(area, &lx).expect("ask rect");
    let lay = computed(&lx, area);
    assert_eq!(lay.options.width, panel.width, "取消左右分栏，选项占满全宽");
    assert_eq!(
        lay.options.bottom(),
        lay.footer.y,
        "统一列表吃掉分隔线与底栏之间的全部高度"
    );
    let gap = lay
        .rows
        .iter()
        .position(|row| matches!(row.kind, RowKind::PreviewGap))
        .expect("预览前有一个空行");
    let line = lay
        .rows
        .iter()
        .find(|row| matches!(row.kind, RowKind::PreviewLine(0)))
        .expect("预览第 1 行必须落位");
    assert!(line.rect.y > lay.rows[gap].rect.y, "预览在选项之后");
    assert!(
        lay.rows.iter().any(|row| row.hit.is_none()),
        "预览行不是可点项"
    );
    let grid = paint(&lx, area);
    assert!(row_text(&grid, area, line.rect.y).contains("┌──┬──┐"));
}

#[test]
fn questions_without_any_preview_reserve_no_space_at_all() {
    let lx = lx_with(vec![question(
        "q",
        false,
        vec![option("a", "da", None), option("b", "db", None)],
    )]);
    let area = surface(80, 30);
    let lay = computed(&lx, area);
    assert!(
        !lay.rows.iter().any(|row| matches!(
            row.kind,
            RowKind::PreviewGap | RowKind::PreviewLine(_) | RowKind::PreviewPlaceholder
        )),
        "无 preview 时不得预占任何高度"
    );
    assert_eq!(lay.options.bottom(), lay.footer.y, "列表区吃掉剩余高度");
    let grid = paint(&lx, area);
    let text: String = (lay.options.y..lay.options.bottom())
        .map(|y| row_text(&grid, area, y))
        .collect();
    assert!(
        !text.contains(a::ASK_NO_PREVIEW),
        "无预览时绝不渲染 (no preview)"
    );
}

#[test]
fn preview_shows_placeholder_when_highlight_leaves_the_previewed_option() {
    let mut first = question(
        "q",
        false,
        vec![option("A", "da", None), option("B", "db", None)],
    );
    first.options[0].preview = Some("left".into());
    let mut lx = lx_with(vec![first]);
    {
        let ask = lx.ask.as_mut().expect("ask");
        ask.begin_edit();
    }
    let area = surface(80, 30);
    let lay = computed(&lx, area);
    let placeholder = lay
        .rows
        .iter()
        .find(|row| matches!(row.kind, RowKind::PreviewPlaceholder))
        .expect("高亮 Other 时预览占位仍然落位");
    let grid = paint(&lx, area);
    assert!(
        row_text(&grid, area, placeholder.rect.y).contains(a::ASK_NO_PREVIEW),
        "高亮 Other 时预览区显示占位，面板高度不抖动"
    );
}

#[test]
fn long_descriptions_wrap_at_full_width_and_every_line_hits_the_option() {
    let long = "描述文字非常长，需要按全宽自动折行完整显示，绝不截断任何一段内容。";
    let lx = lx_with(vec![question("q", false, vec![option("A", long, None)])]);
    let area = surface(44, 30);
    let lay = computed(&lx, area);
    let lines: Vec<&VisualRow> = lay
        .rows
        .iter()
        .filter(|row| matches!(row.kind, RowKind::OptionDesc { index: 0, .. }))
        .collect();
    assert!(
        lines.len() > 1,
        "长描述必须折成多行，实际 {} 行",
        lines.len()
    );
    let grid = paint(&lx, area);
    let mut rendered = String::new();
    for (index, line) in lines.iter().enumerate() {
        assert_eq!(
            hit(area, &lx, line.rect.x + ROW_PREFIX, line.rect.y),
            Some(AskHit::Option(0)),
            "第 {index} 行描述也必须命中该选项"
        );
        let text = row_text(&grid, area, line.rect.y);
        let text: String = text.chars().skip(usize::from(ROW_PREFIX)).collect();
        rendered.push_str(text.trim_end());
    }
    assert!(
        rendered
            .chars()
            .filter(|ch| !ch.is_whitespace())
            .collect::<String>()
            .contains("绝不截断"),
        "折行后的全部内容都必须画出来：{rendered:?}"
    );
}

#[test]
fn descriptions_are_indented_six_columns() {
    let lx = lx_with(sample());
    let area = surface(80, 30);
    let lay = computed(&lx, area);
    let desc = lay
        .rows
        .iter()
        .find(|row| matches!(row.kind, RowKind::OptionDesc { index: 0, line: 0 }))
        .expect("描述行");
    let grid = paint(&lx, area);
    let row = row_text(&grid, area, desc.rect.y);
    let rect = rect(area, &lx).expect("ask rect");
    assert_eq!(
        row.find("left list"),
        Some(usize::from(rect.x + ROW_PREFIX)),
        "描述必须缩进 6 列"
    );
    assert!(
        !row.chars()
            .take(usize::from(ROW_PREFIX))
            .any(|ch| ch == '('),
        "描述行左侧不得出现选项括号"
    );
}

#[test]
fn no_border_only_a_top_rule() {
    let lx = lx_with(vec![question(
        "q",
        false,
        vec![option("a", "da", None), option("b", "db", None)],
    )]);
    let area = surface(80, 30);
    let rect = rect(area, &lx).expect("ask rect");
    let grid = paint(&lx, area);
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
fn single_and_multi_select_have_no_submit_button_row() {
    let single = lx_with(vec![question(
        "q",
        false,
        vec![option("A", "da", None), option("B", "db", None)],
    )]);
    let lay_single = computed(&single, surface(80, 30));
    assert_eq!(lay_single.tabs.len(), 0, "单题单选无顶部 tab");
    assert_eq!(
        lay_single.rows.len(),
        5,
        "2 个带描述选项 (4行) + Other (1行)，无提交按钮行"
    );

    let multi = lx_with(vec![question("q", true, vec![option("A", "da", None)])]);
    let lay_multi = computed(&multi, surface(80, 30));
    assert_eq!(lay_multi.tabs.len(), 2, "单题多选顶部显示题目与 Submit tab");
    assert_eq!(
        lay_multi.rows.len(),
        3,
        "1 个带描述选项 (2行) + Other (1行)，列表底部无提交按钮行"
    );
}

#[test]
fn multi_question_flow_uses_submit_tab_review_without_submit_button() {
    let mut lx = lx_with(vec![
        question("layout", false, vec![option("a", "d", None)]),
        question("theme", false, vec![option("b", "d", None)]),
    ]);
    let first = computed(&lx, surface(80, 30));
    assert_eq!(
        first.rows.len(),
        3,
        "题目 tab 只有选项与 Other，无提交按钮行"
    );
    {
        let ask = lx.ask.as_mut().expect("ask");
        assert!(ask.goto_tab(2));
        assert!(ask.is_confirm());
    }
    let review = computed(&lx, surface(80, 30));
    assert!(
        review
            .rows
            .iter()
            .all(|row| matches!(row.kind, RowKind::Review { .. })),
        "Submit 汇总 tab 仅渲染问答回顾行，不渲染提交胶囊按钮"
    );
    assert_eq!(review.tabs.len(), 3, "两题 + Submit 汇总");
    let grid = paint(&lx, surface(80, 30));
    let line = review
        .rows
        .iter()
        .find(|row| matches!(row.kind, RowKind::Review { index: 0, line: 0 }))
        .expect("回顾行");
    let text = row_text(&grid, surface(80, 30), line.rect.y);
    assert!(
        text.contains("layout:") && text.contains(a::ASK_NO_ANSWER),
        "未作答的题在汇总里明确标出：{text:?}"
    );
}

#[test]
fn editing_expands_other_in_place_and_returns_the_cursor() {
    let mut lx = lx_with(sample());
    let width = computed(&lx, surface(80, 30)).options.width;
    {
        let ask = lx.ask.as_mut().expect("ask");
        ask.resize_editor(width.saturating_sub(ROW_PREFIX), 3);
        ask.begin_edit();
        let editor = ask.editor_mut().expect("editor");
        let long = "第一行文字很长需要折行".repeat(6);
        for ch in long.chars() {
            editor.insert_char(ch);
        }
    }
    let area = surface(80, 30);
    let lay = computed(&lx, area);
    let others: Vec<&VisualRow> = lay
        .rows
        .iter()
        .filter(|row| matches!(row.kind, RowKind::Other { .. }))
        .collect();
    assert!(others.len() > 1, "编辑态必须原地展开成多行输入框");
    assert!(others.len() <= AskState::MAX_OTHER_EDIT_ROWS);
    let (cursor_row, cursor_column) = {
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
    assert!(
        others
            .iter()
            .any(|row| row.rect.contains((cursor_column, cursor_row).into())),
        "光标 {cursor_row},{cursor_column} 必须落在原地输入框内"
    );
    assert!(
        lay.options.bottom() <= lay.footer.y,
        "没有底部独立编辑框：选项列表直接接底栏"
    );
}

#[test]
fn other_click_maps_back_to_the_active_editor_cell() {
    let mut lx = lx_with(sample());
    let area = surface(80, 30);
    let width = computed(&lx, area).options.width;
    {
        let ask = lx.ask.as_mut().expect("ask");
        ask.resize_editor(width.saturating_sub(ROW_PREFIX), 3);
        ask.begin_edit();
        ask.editor_mut().expect("editor").insert_str("click me");
    }
    let lay = computed(&lx, area);
    let other = lay
        .rows
        .iter()
        .find(|row| matches!(row.kind, RowKind::Other { line: 0 }))
        .expect("Other 行");
    let cell = ask_editor_cell_at(area, &lx, other.rect.x + ROW_PREFIX + 2, other.rect.y)
        .expect("落在输入框正文");
    assert_eq!(cell, (0, 2));
    assert!(
        ask_editor_cell_at(area, &lx, other.rect.x + 1, other.rect.y).is_none(),
        "前缀列不属于输入框正文"
    );
}

#[test]
fn other_without_custom_text_stays_a_single_row() {
    let lx = lx_with(sample());
    let area = surface(80, 30);
    let lay = computed(&lx, area);
    let others = lay
        .rows
        .iter()
        .filter(|row| matches!(row.kind, RowKind::Other { .. }))
        .count();
    assert_eq!(others, 1);
    let grid = paint(&lx, area);
    let other = lay
        .rows
        .iter()
        .find(|row| matches!(row.kind, RowKind::Other { .. }))
        .expect("Other 行");
    let row = row_text(&grid, area, other.rect.y);
    assert!(
        row.contains(a::ASK_OTHER_LABEL),
        "空态显示 Other...：{row:?}"
    );
}

#[test]
fn wheel_zone_targets_the_single_unified_list() {
    let lx = lx_with(sample());
    let area = surface(80, 30);
    let lay = computed(&lx, area);
    let preview = lay
        .rows
        .iter()
        .find(|row| matches!(row.kind, RowKind::PreviewLine(0)))
        .expect("preview");
    assert_eq!(
        wheel_zone(area, &lx, preview.rect.x + 1, preview.rect.y),
        Some(AskWheelZone::Options)
    );
    assert_eq!(
        wheel_zone(area, &lx, lay.options.x + 1, lay.options.y),
        Some(AskWheelZone::Options)
    );
    assert_eq!(wheel_zone(area, &lx, 0, 0), None);
}

#[test]
fn explicit_viewport_wins_over_the_highlight_and_keeps_the_highlight_stable() {
    // 滚轮建立的显式视口必须覆盖「跟随高亮」的推导：窗口滚动时高亮原地不动。
    let options = (0..6)
        .map(|index| option(&format!("o{index}"), "desc", None))
        .collect();
    let mut lx = lx_with(vec![question("q", false, options)]);
    let area = surface(80, 20);
    let before = computed(&lx, area);
    assert!(
        before.content_rows > usize::from(before.options.height),
        "内容必须溢出（否则测不到滚动）"
    );
    assert_eq!(before.scroll, 0, "初始窗口跟随首个高亮");

    {
        let ask = lx.ask.as_mut().expect("ask");
        assert!(ask.scroll_viewport(
            1,
            before.scroll,
            before.content_rows,
            usize::from(before.options.height)
        ));
    }
    let after = computed(&lx, area);
    assert_eq!(after.scroll, 1, "窗口按显式视口滚动");
    let ask = lx.ask.as_ref().expect("ask");
    assert_eq!(ask.highlight[0], 0, "滚轮不得移动高亮");
    assert_eq!(ask.viewport(), Some(1));
    // 落位后的每一行仍可命中（滚动不破坏几何一致性）。
    for row in &after.rows {
        if let Some(expected) = row.hit {
            assert_eq!(hit(area, &lx, row.rect.x + 1, row.rect.y), Some(expected));
        }
    }
}

#[test]
fn list_window_matches_the_rendered_geometry() {
    // 事件层滚轮用的基准必须与渲染同源，否则滚一格会跳两格或原地不动。
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
    let (scroll, total, visible) = super::list_window(area, &lx).expect("list window");
    assert_eq!(scroll, lay.scroll);
    assert_eq!(total, lay.content_rows);
    assert_eq!(visible, usize::from(lay.options.height));
}

#[test]
fn overflow_draws_one_scrollbar_and_reserves_its_column() {
    let options = (0..8)
        .map(|index| option(&format!("o{index}"), "desc", None))
        .collect();
    let mut lx = lx_with(vec![question("q", true, options)]);
    lx.ask.as_mut().expect("ask").set_highlight(7);
    let area = surface(60, 20);
    let lay = computed(&lx, area);
    assert!(
        lay.content_rows > usize::from(lay.options.height),
        "必须溢出"
    );
    assert!(
        lay.rows
            .iter()
            .all(|row| row.rect.width == lay.options.width - 1),
        "溢出时右侧让出一列给滚动条"
    );
    let grid = paint(&lx, area);
    let thumb = (lay.options.y..lay.options.bottom())
        .filter(|y| grid.cell(lay.options.right() - 1, *y).ch == '▐')
        .count();
    assert!(thumb > 0, "列表右侧必须画出唯一的滚动条");
}

#[test]
fn preview_scrolls_together_with_the_options_through_the_same_viewport() {
    let mut options: Vec<AskOption> = (0..6)
        .map(|index| option(&format!("o{index}"), "desc", None))
        .collect();
    options[0].preview = Some("P1\nP2\nP3".into());
    let mut lx = lx_with(vec![question("q", false, options)]);
    let area = surface(60, 20);
    let before = computed(&lx, area);
    assert!(
        before.content_rows > usize::from(before.options.height),
        "内容必须溢出"
    );
    assert!(
        !before
            .rows
            .iter()
            .any(|row| matches!(row.kind, RowKind::PreviewLine(2))),
        "折叠时预览末行应被滚出视口"
    );
    {
        let ask = lx.ask.as_mut().expect("ask");
        assert!(ask.scroll_viewport(
            isize::MAX,
            before.scroll,
            before.content_rows,
            usize::from(before.options.height)
        ));
    }
    let after = computed(&lx, area);
    let line = after
        .rows
        .iter()
        .find(|row| matches!(row.kind, RowKind::PreviewLine(2)))
        .expect("滚到底后预览末行进入同一个视口");
    let grid = paint(&lx, area);
    assert!(row_text(&grid, area, line.rect.y).contains("P3"));
    assert_eq!(
        lx.ask.as_ref().expect("ask").highlight[0],
        0,
        "滚轮/翻页绝不动高亮"
    );
}
