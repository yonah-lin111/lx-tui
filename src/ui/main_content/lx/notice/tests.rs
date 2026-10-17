//! 通知组件测试：格式、级别配色、细节换行与行数口径。

use ratatui::layout::Rect;

use super::*;
use crate::ui::main_content::lx::grid::Grid;
use crate::ui::main_content::lx::history::wrap;

fn error(detail: Option<&str>) -> Notice {
    Notice::error("Failed", detail.map(str::to_string))
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
fn error_renders_red_bold_title_and_dim_connector() {
    let notice = error(Some("http 503"));
    let area = Rect::new(0, 0, 40, 4);
    let mut grid = Grid::new(area);
    let mut row = 0;
    draw(&mut grid, &notice, area, 0, &mut row);

    assert_eq!(row, 2, "标题 + 细节两行");
    assert_eq!(row_text(&grid, area, 0), "! Failed");
    assert_eq!(row_text(&grid, area, 1), "  └─ http 503");
    // 标题整行红 + 粗（标记也同色）。
    for x in 0..8 {
        assert_eq!(grid.cell(x, 0).style, style::agent_fail(), "x={x}");
    }
    // 细节行：连接符压暗、正文正常色。
    assert_eq!(grid.cell(2, 1).style, style::agent_connector());
    assert_eq!(grid.cell(5, 1).style, style::text());
}

#[test]
fn severity_maps_to_its_palette() {
    assert_eq!(title_style(NoticeKind::Error), style::agent_fail());
    assert_eq!(title_style(NoticeKind::Warning), style::agent_warn());
    assert_eq!(
        title_style(NoticeKind::Info),
        style::chat_provider(),
        "提示用窗格底部供应商同款次要信息色"
    );
}

#[test]
fn info_notice_without_detail_is_single_row() {
    let notice = Notice::info("Interrupted");
    let area = Rect::new(0, 0, 40, 4);
    let mut grid = Grid::new(area);
    let mut row = 0;
    draw(&mut grid, &notice, area, 0, &mut row);
    assert_eq!(row, 1);
    assert_eq!(row_text(&grid, area, 0), "! Interrupted");
    assert_eq!(grid.cell(0, 0).style, style::chat_provider());
}

#[test]
fn detail_wraps_aligned_and_rows_match_draw() {
    let notice = error(Some("http 503 · upstream overloaded, retry later"));
    for width in [40u16, 24, 14, 6] {
        let area = Rect::new(0, 0, width, 40);
        let mut grid = Grid::new(area);
        let mut row = 0;
        draw(&mut grid, &notice, area, 0, &mut row);
        assert_eq!(rows(&notice, width), row, "width={width}：行数口径一致");
        if width > 5 {
            // 首条细节行带连接符；续行按前缀宽度对齐、不重复画。
            let title_rows = u16::try_from(wrap("! Failed", width).len()).expect("rows");
            let first = row_text(&grid, area, title_rows);
            assert!(first.starts_with("  └─ "), "width={width}: {first}");
        }
    }
}

#[test]
fn too_narrow_for_prefix_falls_back_to_full_width_detail() {
    // 前缀 5 列，宽度 4 放不下 → 细节行整宽、无连接符。
    let width = 4;
    let notice = error(Some("http 503"));
    let area = Rect::new(0, 0, width, 8);
    let mut grid = Grid::new(area);
    let mut row = 0;
    draw(&mut grid, &notice, area, 0, &mut row);
    assert_eq!(rows(&notice, width), row);
    let title_rows = wrap("! Failed", width).len();
    let detail: String = (title_rows..row)
        .map(|line| row_text(&grid, area, u16::try_from(line).expect("row")))
        .collect();
    assert!(!detail.contains("└─"), "不放连接符：{detail}");
    assert!(detail.contains("http"), "细节整宽：{detail}");
}

#[test]
fn scrolled_out_rows_do_not_paint() {
    let notice = error(Some("http 503"));
    let area = Rect::new(0, 0, 40, 2);
    let mut grid = Grid::new(area);
    // 视口从第 1 行开始：标题被滚出，细节落在首行。
    let mut row = 0;
    draw(&mut grid, &notice, area, 1, &mut row);
    assert_eq!(row, 2);
    assert_eq!(row_text(&grid, area, 0), "  └─ http 503");
}
