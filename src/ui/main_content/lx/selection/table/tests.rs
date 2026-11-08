use ratatui::layout::Rect;
use ratatui::style::Style;

use crate::app::selection::SelectionUnit;
use crate::ui::main_content::lx::grid::{CONTINUATION, Grid, GridTableRow};
use crate::ui::markdown_render;

use super::{
    TableSelectionMode, align_wide_char_cols, extract_table_lines, rendered_table_row_bounds,
    resolve_table_selection, table_unit_cell_range,
};

/// 将一段 Markdown 表格渲染到 `Grid`（带可选左侧缩进 `base_col`）。
fn render_table_to_grid(md: &str, width: u16, base_col: u16) -> Grid {
    use unicode_width::UnicodeWidthChar;

    let md_lines = markdown_render::render_wrapped(md, width.saturating_sub(base_col));
    let height = u16::try_from(md_lines.len()).unwrap_or(1).max(1) + 2;
    let mut grid = Grid::new(Rect::new(0, 0, width, height));
    for (idx, line) in md_lines.iter().enumerate() {
        let y = u16::try_from(idx).unwrap_or(0);
        if let Some(ref tr) = line.table {
            grid.set_table_row(base_col, y, tr.clone());
        }
        let mut cx = base_col;
        for (text, span_style) in &line.spans {
            for ch in text.chars() {
                grid.put_char(cx, y, ch, *span_style);
                cx = cx.saturating_add(u16::try_from(ch.width().unwrap_or(1).max(1)).unwrap_or(1));
            }
        }
    }
    grid
}

#[test]
fn empty_cells_preserve_tsv_column_alignment_in_grid_and_column_locked_modes() {
    let md = "| A | B | C |\n| --- | --- | --- |\n| x |   | z |";
    let grid = render_table_to_grid(md, 40, 0);
    let tr = grid.table_row_at(0).expect("table row 0").clone();
    let total_lines = tr.line_ref.table.lines.len();
    let end_row = i32::try_from(total_lines - 1).unwrap_or(0);
    let col0 = tr.line_ref.table.columns[0];
    let col2 = tr.line_ref.table.columns[2];

    let plan = resolve_table_selection(
        &tr,
        0,
        (0, end_row),
        (1, col0.content_start),
        (end_row - 1, col2.content_start),
    )
    .expect("grid plan");
    assert!(matches!(plan.mode, TableSelectionMode::Grid { .. }));

    let offsets: Vec<usize> = (0..total_lines).collect();
    let rows = extract_table_lines(&grid, 0, &plan, &offsets);
    assert_eq!(rows.join("\n"), "A\tB\tC\nx\t\tz");
}

#[test]
fn consecutive_br_tags_preserve_blank_lines_inside_cell() {
    let md = "| 键 | 值 |\n| --- | --- |\n| k | x<br><br>y |";
    let grid = render_table_to_grid(md, 40, 0);
    let tr = grid.table_row_at(0).expect("table row").clone();
    let total_lines = tr.line_ref.table.lines.len();
    let end_row = i32::try_from(total_lines - 1).unwrap_or(0);
    let row1 = tr.line_ref.table.logical_rows[1];
    let col1 = tr.line_ref.table.columns[1];

    let plan = resolve_table_selection(
        &tr,
        0,
        (0, end_row),
        (
            i32::try_from(row1.first_line).unwrap_or(0),
            col1.content_start,
        ),
        (
            i32::try_from(row1.last_line).unwrap_or(0),
            col1.content_end - 1,
        ),
    )
    .expect("single cell plan");
    let offsets: Vec<usize> = (0..total_lines).collect();
    let rows = extract_table_lines(&grid, 0, &plan, &offsets);
    assert_eq!(rows.join("\n"), "x\n\ny");
}

#[test]
fn cjk_wide_char_continuation_start_and_lead_end_snap_to_full_glyph() {
    let md = "| 标题 | 内容 |\n| --- | --- |\n| 1 | 中文测试 |";
    let grid = render_table_to_grid(md, 40, 0);
    let tr = grid.table_row_at(0).expect("table row").clone();
    let total_lines = tr.line_ref.table.lines.len();
    let end_row = i32::try_from(total_lines - 1).unwrap_or(0);
    let row1 = tr.line_ref.table.logical_rows[1];
    let col1 = tr.line_ref.table.columns[1];
    let y = u16::try_from(row1.first_line).unwrap_or(0);

    // `中` 占 `col1.content_start`（主格）与 `col1.content_start + 1`（CONTINUATION 右半格）。
    let cont_col = col1.content_start + 1;
    assert_eq!(grid.cell(cont_col, y).ch, CONTINUATION);
    let (aligned0, aligned1) = align_wide_char_cols(&grid, y, cont_col, col1.content_start + 6);
    assert_eq!(aligned0, col1.content_start);
    assert_eq!(aligned1, col1.content_start + 7);

    // 从 `中` 的右半格开始拖选到末尾，提取结果必须完整包含首字 `中`。
    let line_i32 = i32::try_from(row1.first_line).unwrap_or(0);
    let plan = resolve_table_selection(
        &tr,
        0,
        (0, end_row),
        (line_i32, cont_col),
        (line_i32, col1.content_end - 1),
    )
    .expect("cjk plan");
    let offsets: Vec<usize> = (0..total_lines).collect();
    let rows = extract_table_lines(&grid, 0, &plan, &offsets);
    assert_eq!(rows.join("\n"), "中文测试");
}

#[test]
fn double_click_on_border_or_filler_or_empty_cell_does_not_select_neighbor_char() {
    let md = "| A | B |\n| --- | --- |\n| 第一行<br>第二行 | 短 |";
    let grid = render_table_to_grid(md, 40, 0);
    let tr0 = grid.table_row_at(0).expect("top border").clone();
    let total_lines = tr0.line_ref.table.lines.len();
    let end_row = i32::try_from(total_lines - 1).unwrap_or(0);
    let col0 = tr0.line_ref.table.columns[0];
    let col1 = tr0.line_ref.table.columns[1];

    // 1. 双击顶部水平边框：返回的退化单点在 resolve_table_selection 中必须返回 None。
    let border_click = (0, col0.content_start);
    let (a, b) =
        table_unit_cell_range(&grid, &tr0, border_click, SelectionUnit::Word).expect("range");
    assert_eq!(a, b);
    assert!(resolve_table_selection(&tr0, 0, (0, end_row), a, b).is_none());

    // 2. 双击第 1 列第二行的空白填充行（filler）：同样不得钳入上一行复制字符。
    let row1 = tr0.line_ref.table.logical_rows[1];
    let filler_y = u16::try_from(row1.last_line).unwrap_or(0);
    let tr_filler = grid.table_row_at(filler_y).expect("filler row").clone();
    let filler_click = (i32::from(filler_y), col1.content_start);
    let (fa, fb) = table_unit_cell_range(&grid, &tr_filler, filler_click, SelectionUnit::Word)
        .expect("filler range");
    assert!(
        resolve_table_selection(&tr_filler, i32::from(filler_y), (0, end_row), fa, fb).is_none()
    );
}

#[test]
fn dragging_or_clicking_in_left_or_right_table_margin_is_ignored() {
    let md = "| A | B |\n| --- | --- |\n| 1 | 2 |";
    let base_col = 4u16;
    let grid = render_table_to_grid(md, 40, base_col);
    let tr = grid.table_row_at(1).expect("header row").clone();
    let end_row = i32::try_from(tr.line_ref.table.lines.len() - 1).unwrap_or(0);
    let table_right = base_col + tr.line_ref.table.total_width;

    // 左侧缩进空白区拖选与双击
    assert!(resolve_table_selection(&tr, 1, (0, end_row), (1, 0), (1, 2)).is_none());
    assert!(table_unit_cell_range(&grid, &tr, (1, 1), SelectionUnit::Word).is_none());

    // 右侧空白区拖选与双击
    assert!(
        resolve_table_selection(&tr, 1, (0, end_row), (1, table_right), (1, table_right + 4))
            .is_none()
    );
    assert!(table_unit_cell_range(&grid, &tr, (1, table_right + 1), SelectionUnit::Line).is_none());
}

#[test]
fn truncated_table_bounds_prevent_false_selection_from_rows_below_table() {
    let md = "| 阶段 | 结论 |\n| --- | --- |\n| 1 | 数据一 |\n| 2 | 数据二 |";
    let md_lines = markdown_render::render_wrapped(md, 36);
    // 模拟折叠汇报仅渲染表格前 2 行（顶边框 + 表头），第 2 行是 `… 5 more lines` 普通提示行。
    let mut grid = Grid::new(Rect::new(0, 0, 36, 4));
    for (idx, line) in md_lines.iter().take(2).enumerate() {
        let y = u16::try_from(idx).unwrap_or(0);
        if let Some(ref tr) = line.table {
            grid.set_table_row(0, y, tr.clone());
        }
        let mut cx = 0;
        for (text, s) in &line.spans {
            for ch in text.chars() {
                grid.put_char(cx, y, ch, *s);
                cx = cx.saturating_add(
                    u16::try_from(
                        unicode_width::UnicodeWidthChar::width(ch)
                            .unwrap_or(1)
                            .max(1),
                    )
                    .unwrap_or(1),
                );
            }
        }
    }
    grid.put_clipped(0, 2, "… 5 more lines", Style::default(), 36);

    let header_tr: GridTableRow = grid.table_row_at(1).expect("header table row").clone();
    let bounds = rendered_table_row_bounds(&grid, 0, 0, 4, 0, &header_tr, 1);
    assert_eq!(bounds, (0, 1), "折叠截断后表格真实有效行必须停在第 1 行");

    // 在第 2 行（提示行）横向拖选时，绝不能命中第 1 行的表头选区。
    assert!(resolve_table_selection(&header_tr, 1, bounds, (2, 2), (2, 12)).is_none());
}
