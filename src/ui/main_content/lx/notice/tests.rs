//! 通知组件测试：固定标题枚举、原因行、级别配色、换行与行数口径。

use ratatui::layout::Rect;

use super::*;
use crate::ui::main_content::lx::grid::Grid;
use crate::ui::main_content::lx::history::wrap;

fn failed(reason: &str) -> Notice {
    Notice::new(NoticeTitle::Failed, reason)
}

/// 网格整行文本（宽字符占位列跳过）。
fn row_text(grid: &Grid, area: Rect, row: u16) -> String {
    (area.x..area.right())
        .map(|x| grid.cell(x, row).ch)
        .filter(|ch| *ch != '\0')
        .collect::<String>()
        .trim_end()
        .to_string()
}

#[test]
fn titles_are_fixed_enum_values_with_red_or_yellow_only() {
    assert_eq!(NoticeTitle::Failed.label(), "Failed");
    assert_eq!(NoticeTitle::Interrupted.label(), "Interrupted");
    assert_eq!(
        NoticeTitle::Failed.style(),
        style::agent_fail(),
        "失败红 + 粗"
    );
    assert_eq!(
        NoticeTitle::Interrupted.style(),
        style::agent_warn(),
        "中断黄 + 粗（不再有灰色档）"
    );
}

#[test]
fn notice_is_title_plus_exactly_one_reason_line() {
    let notice = failed("http 503");
    let area = Rect::new(0, 0, 40, 4);
    let mut grid = Grid::new(area);
    let mut row = 0;
    draw(&mut grid, &notice, area, 0, &mut row);

    assert_eq!(row, 2, "标题 + 原因各一行");
    assert_eq!(row_text(&grid, area, 0), "■ Failed");
    assert_eq!(row_text(&grid, area, 1), "  └─ http 503");
    // 标题整行红 + 粗（标记也同色）。
    for x in 0..8 {
        assert_eq!(grid.cell(x, 0).style, style::agent_fail(), "x={x}");
    }
    // 原因行：连接符供应商灰 + 粗、正文正常色。
    assert_eq!(grid.cell(2, 1).style, style::chat_notice_connector());
    assert!(
        style::chat_notice_connector()
            .add_modifier
            .contains(ratatui::style::Modifier::BOLD),
        "连接符加粗"
    );
    assert_eq!(
        style::chat_notice_connector().fg,
        style::chat_provider().fg,
        "连接符用供应商同款前景（默认灰）"
    );
    assert_eq!(grid.cell(5, 1).style, style::text());
}

#[test]
fn reason_wraps_aligned_and_rows_match_draw() {
    let notice = failed("http 503 · upstream overloaded, retry later");
    for width in [40u16, 24, 14, 6] {
        let area = Rect::new(0, 0, width, 40);
        let mut grid = Grid::new(area);
        let mut row = 0;
        draw(&mut grid, &notice, area, 0, &mut row);
        assert_eq!(rows(&notice, width), row, "width={width}：行数口径一致");
        if width > 5 {
            // 首条原因行带连接符；续行按前缀宽度对齐、不重复画。
            let title_rows = u16::try_from(wrap("■ Failed", width).len()).expect("rows");
            let first = row_text(&grid, area, title_rows);
            assert!(first.starts_with("  └─ "), "width={width}: {first}");
        }
    }
}

#[test]
fn too_narrow_for_prefix_falls_back_to_full_width_reason() {
    // 前缀 5 列，宽度 4 放不下 → 原因行整宽、无连接符。
    let width = 4;
    let notice = failed("http 503");
    let area = Rect::new(0, 0, width, 8);
    let mut grid = Grid::new(area);
    let mut row = 0;
    draw(&mut grid, &notice, area, 0, &mut row);
    assert_eq!(rows(&notice, width), row);
    let title_rows = wrap("■ Failed", width).len();
    let reason: String = (title_rows..row)
        .map(|line| row_text(&grid, area, u16::try_from(line).expect("row")))
        .collect();
    assert!(!reason.contains("└─"), "不放连接符：{reason}");
    assert!(reason.contains("http"), "原因整宽：{reason}");
}

#[test]
fn scrolled_out_rows_do_not_paint() {
    let notice = failed("http 503");
    let area = Rect::new(0, 0, 40, 2);
    let mut grid = Grid::new(area);
    // 视口从第 1 行开始：标题被滚出，原因落在首行。
    let mut row = 0;
    draw(&mut grid, &notice, area, 1, &mut row);
    assert_eq!(row, 2);
    assert_eq!(row_text(&grid, area, 0), "  └─ http 503");
}

/// 通知的行首标记必须是**步骤方块**，且颜色按级别红/黄。
#[test]
fn notice_marker_is_step_square_in_red_or_yellow() {
    for (title, expected) in [
        (NoticeTitle::Failed, style::agent_fail()),
        (NoticeTitle::Interrupted, style::agent_warn()),
    ] {
        let notice = Notice::new(title, "reason");
        let area = Rect::new(0, 0, 40, 4);
        let mut grid = Grid::new(area);
        let mut row = 0;
        draw(&mut grid, &notice, area, 0, &mut row);

        let cell = grid.cell(0, 0);
        assert_eq!(
            cell.ch,
            crate::ui::text::agent::STEP_MARK_FILLED,
            "{:?} 行首应为实心方块，不再是 `!`",
            title
        );
        assert_eq!(cell.style, expected, "{:?} 方块取该级别状态色", title);
        // 方块后紧跟 1 列空格（与 STEP_GUTTER = 2 同宽），文案从第 2 列起。
        assert_eq!(grid.cell(1, 0).ch, ' ');
        assert_eq!(
            grid.cell(2, 0).ch,
            title.label().chars().next().expect("非空")
        );
        assert_eq!(grid.cell(2, 0).style, expected, "文案同色，整行一个色相");
    }
}

/// 通知的方块与工具行 / 思考头的方块**形状与宽度完全一致**（同一个 `StepMark`），
/// 这样历史里所有步骤标记在视觉上是同一套符号。
#[test]
fn notice_marker_matches_step_mark_shape() {
    let area = Rect::new(0, 0, 40, 4);
    let mut grid = Grid::new(area);
    let mut row = 0;
    let notice = Notice::new(NoticeTitle::Failed, "boom");
    draw(&mut grid, &notice, area, 0, &mut row);

    // 失败步骤在工具行里的拼法：`StepMark::Failed.segment(0)`。
    let expected = crate::ui::main_content::lx::history::StepMark::Failed.segment(0);
    let painted: String = expected
        .chars()
        .enumerate()
        .map(|(x, _)| grid.cell(u16::try_from(x).expect("col"), 0).ch)
        .collect();
    assert_eq!(painted, expected, "通知方块与步骤方块拼法一致");
}
