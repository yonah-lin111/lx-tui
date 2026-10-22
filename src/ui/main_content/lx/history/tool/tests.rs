//! 工具步骤行测试：两行式（`■ 动词` + `  └─ 详情`）× 多状态的 buffer 断言、
//! 宽度自适应、列宽裁剪与几何一致性。
//!
//! 只断言自动可验证的部分（文本/样式/列宽/行数）；真实配色与对齐由用户 `cargo run` 验收。
//!
//! 按职责拆成子模块（单文件行数上限 1000）：
//! - `structure`：两行结构与状态编码（方块/动词/错误文案/兜底状态词）；
//! - `extras`：各工具特有的次要信息（命中数、区间 + 实际行数、增删计数、截断与分隔符）；
//! - `width`：宽度自适应、列宽裁剪与行数几何不变量；
//! - `command_cards`：进程/计划卡片的宽窄、中文、展开与几何一致。

use ratatui::layout::Rect;
use ratatui::style::Style;
use unicode_width::UnicodeWidthStr;

use super::*;
use crate::app::state::{ToolStatus, ToolSummary};

use crate::ui::style;
use crate::ui::text;

mod background_cards;
mod command_cards;
mod extras;
mod structure;
mod width;

// ===== 构造与辅助 =====

/// 构造一个工具调用步骤。
fn tool_call(
    name: &str,
    status: ToolStatus,
    summary: Option<ToolSummary>,
    error: Option<&str>,
) -> ToolCallView {
    ToolCallView {
        call_id: format!("call-{name}"),
        name: name.into(),
        position: 0,
        arguments_json: String::new(),
        status,
        summary: summary.map(Box::new),
        error: error.map(str::to_string),
        result_json: None,
        detail: None,
    }
}

/// `read` 的摘要：路径 + 行范围。
fn read_summary() -> ToolSummary {
    ToolSummary {
        path: Some("src/ui/style.rs".into()),
        range: Some((12, 40)),
        ..ToolSummary::default()
    }
}

/// `search` 的摘要：命中数（搜索范围作为路径）。
fn search_summary() -> ToolSummary {
    ToolSummary {
        path: Some("src".into()),
        matches: Some(3),
        ..ToolSummary::default()
    }
}

/// `edit` 的摘要：路径 + 增删计数。
fn edit_summary() -> ToolSummary {
    ToolSummary {
        path: Some("src/app/state.rs".into()),
        added: Some(12),
        removed: Some(4),
        ..ToolSummary::default()
    }
}

/// `write` 的摘要：路径 + 增删计数。
fn write_summary() -> ToolSummary {
    ToolSummary {
        path: Some("docs/notes.md".into()),
        added: Some(18),
        removed: Some(0),
        ..ToolSummary::default()
    }
}

/// 渲染两行到网格，按行拼出文本（跳过宽字符占位格），用 `\n` 连接。
fn render_row(call: &ToolCallView, width: u16) -> (String, Grid) {
    render_row_at(call, width, 0)
}

/// 以指定 loading 帧渲染（方块呼吸相位断言用）。
fn render_row_at(call: &ToolCallView, width: u16, loading_frame: usize) -> (String, Grid) {
    let total = rows(call, width, false);
    let rect = Rect::new(0, 0, width, u16::try_from(total.max(1)).unwrap_or(u16::MAX));
    let mut grid = Grid::new(rect);
    let mut row_index = 0usize;
    draw(
        &mut grid,
        rect,
        0,
        &mut row_index,
        call,
        loading_frame,
        false,
    );
    assert_eq!(
        row_index,
        rows(call, width, false),
        "绘制推进行数必须与 rows 一致"
    );
    (grid_text(&grid, rect), grid)
}

/// 网格整块文本（两行以 `\n` 连接）。
fn grid_text(grid: &Grid, rect: Rect) -> String {
    let mut lines: Vec<String> = Vec::new();
    for y in rect.y..rect.bottom() {
        let mut out = String::new();
        for x in rect.x..rect.right() {
            let glyph = grid.cell(x, y);
            if glyph.ch != crate::ui::main_content::lx::grid::CONTINUATION {
                out.push(glyph.ch);
            }
        }
        lines.push(out);
    }
    lines.join("\n")
}

/// 标题行的段（第 1 行：`■ 动词`）。
fn header_segments(call: &ToolCallView, width: u16) -> Vec<(String, Style)> {
    header(call, width, 0, true)
        .into_iter()
        .map(|segment| (segment.text, segment.style))
        .collect()
}

/// 详情行（`  └─ 详情`，可能多行）拼成一段段文本。
fn detail_segments(call: &ToolCallView, width: u16) -> Vec<(String, Style)> {
    file_summary::detail(call, width, false)
        .into_iter()
        .flatten()
        .map(|segment| (segment.text, segment.style))
        .collect()
}

/// 第 1 行文本（未裁剪）。
fn header_text(call: &ToolCallView, width: u16) -> String {
    header_segments(call, width)
        .into_iter()
        .map(|(text, _)| text)
        .collect()
}

/// 详情行文本（可能多行，按 `\\n` 连接，与绘制一致）。
fn detail_text(call: &ToolCallView, width: u16) -> String {
    file_summary::detail(call, width, false)
        .into_iter()
        .map(|row| {
            row.into_iter()
                .map(|segment| segment.text)
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// 详情行的**逻辑内容**（折行的续行缩进不计）：用于「某段文字是否还在」的断言。
fn detail_flat(call: &ToolCallView, width: u16) -> String {
    let mut out = String::new();
    for row in file_summary::detail(call, width, false) {
        for (index, segment) in row.into_iter().enumerate() {
            // 续行的第一个段是等宽空白缩进，不属于内容。
            if index == 0 && segment.text.trim().is_empty() {
                continue;
            }
            out.push_str(&segment.text);
        }
    }
    out
}

/// 两行用 `\n` 连接（段拼接，未裁剪）。
fn full_text(call: &ToolCallView, width: u16) -> String {
    format!("{}\n{}", header_text(call, width), detail_text(call, width))
}
