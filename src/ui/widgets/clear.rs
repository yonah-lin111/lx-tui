//! 浮层清底：清空区域，并处理被浮层边缘切成两半的底层宽字符。
//!
//! 底层（窗格 / prompt / lx 页）绘制宽字符时，会把其右半格标记为 diff 跳过
//! （`CellDiffOption::Skip`）。若浮层边缘恰好切在宽字符中间，ratatui 的 diff 会因
//! 「宽字符另一半未变」而整格跳过相邻单元格，导致两类残留：
//!
//! - 左缘：宽字符左半格露在浮层外、右半格落在浮层边框列上；右半格永不刷新，
//!   边框缺格。
//! - 右缘：宽字符左半格落在浮层边框列上、右半格露在浮层外；右半格永不刷新，
//!   边框右侧留半个残字。
//!
//! 这里在清空区域后，把浮层外侧那一半宽字符擦成空白（保留其原有底色与修饰符），
//! 使其不再「未变」，diff 随之放行被压住的边框格。只影响紧贴浮层两侧边缘、
//! 被切成半格的宽字符，其余内容不动。

use ratatui::Frame;
use ratatui::buffer::{Buffer, CellDiffOption};
use ratatui::layout::Rect;
use unicode_width::UnicodeWidthStr;

/// 清空浮层区域；画在 `Frame` 上的浮层统一走这里。
pub fn render(frame: &mut Frame<'_>, area: Rect) {
    render_buffer(frame.buffer_mut(), area);
}

/// 缓冲区版本：命令面板等直接以 `Buffer` 绘制的路径复用。
pub fn render_buffer(buf: &mut Buffer, area: Rect) {
    let area = area.intersection(buf.area);
    if area.is_empty() {
        return;
    }
    // 先探测跨右缘的宽字符（其左半格在区域内、右半格露在外侧），
    // 记录需额外擦除的行；随后 Clear 会覆盖其左半格。
    let right_orphans: Vec<u16> = if area.right() < buf.area.right() {
        (area.top()..area.bottom())
            .filter(|y| {
                buf.cell((area.right() - 1, *y))
                    .is_some_and(|cell| cell.symbol().width() >= 2)
            })
            .collect()
    } else {
        Vec::new()
    };

    for y in area.top()..area.bottom() {
        for x in area.left()..area.right() {
            if let Some(cell) = buf.cell_mut((x, y)) {
                cell.reset();
            }
        }
    }

    // 右缘：把露在浮层外的占位半格置空并清除 diff 跳过标记，
    // 使其与上一帧不同而被重新下发，抹掉终端上的残字。
    for y in right_orphans {
        if let Some(cell) = buf.cell_mut((area.right(), y)) {
            cell.set_char(' ');
            cell.set_diff_option(CellDiffOption::None);
        }
    }

    erase_left_edge_wide_chars(buf, area);
}

/// 把紧贴 `area` 左缘外侧、跨入区域的宽字符左半格擦成空白。
///
/// 保留该格的底色与修饰符（只抹掉字符），使被压住的半格与区域底色连贯。
fn erase_left_edge_wide_chars(buf: &mut Buffer, area: Rect) {
    let Some(x) = area.left().checked_sub(1) else {
        return;
    };
    if x < buf.area.x {
        return;
    }
    for y in area.top()..area.bottom() {
        let Some(cell) = buf.cell_mut((x, y)) else {
            continue;
        };
        if cell.symbol().width() < 2 {
            continue;
        }
        cell.set_char(' ');
        cell.set_diff_option(CellDiffOption::None);
    }
}

#[cfg(test)]
mod tests;
