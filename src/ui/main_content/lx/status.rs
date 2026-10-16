//! 聊天状态行：模型、思考等级、上下文用量与停止/历史控件。
//!
//! 状态区高度由同一布局函数计算后传给渲染与鼠标命中，字符串长度不改变
//! 编辑器或按钮矩形。窄窗格按「模型 → 等级 → 用量」次序分行。

use ratatui::layout::Rect;
use unicode_width::UnicodeWidthStr;

use crate::app::state::Lx;
use crate::app::state::lx::chat::ChatStatus;
use crate::ui::{style, text};

use super::Grid;

/// 停止/历史按钮占用的列数。
const BUTTON_WIDTH: u16 = 5;
/// 分隔符。
const SEPARATOR: &str = " · ";

/// 状态区布局：文本区与按钮矩形。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StatusLayout {
    pub area: Rect,
    pub stop: Option<Rect>,
    pub history: Option<Rect>,
    pub label: Rect,
}

/// 状态区期望行数：窄窗格（< 36 列）2 行，否则 1 行。
pub(super) fn rows(area: Rect, _status: &ChatStatus) -> u16 {
    if area.width < 36 {
        2.min(area.height)
    } else {
        1.min(area.height)
    }
}

/// 计算状态区布局；空间不足时返回零行。
pub(super) fn layout(area: Rect, status: &ChatStatus, rows: u16) -> StatusLayout {
    let rows = rows.min(area.height);
    if area.width < 8 || rows == 0 {
        return StatusLayout {
            area: Rect::new(area.x, area.y, area.width, 0),
            stop: None,
            history: None,
            label: area,
        };
    }
    let status_row = Rect::new(area.x, area.y, area.width, rows);
    let mut right = status_row.right();
    let mut history = None;
    let mut stop = None;
    if right.saturating_sub(status_row.x) > BUTTON_WIDTH + 4 {
        history = Some(Rect::new(
            right - BUTTON_WIDTH,
            status_row.y,
            BUTTON_WIDTH,
            1,
        ));
        right -= BUTTON_WIDTH;
    }
    if status.streaming && right.saturating_sub(status_row.x) > BUTTON_WIDTH + 4 {
        stop = Some(Rect::new(
            right - BUTTON_WIDTH,
            status_row.y,
            BUTTON_WIDTH,
            1,
        ));
        right -= BUTTON_WIDTH;
    }
    StatusLayout {
        area: status_row,
        stop,
        history,
        label: Rect::new(
            status_row.x,
            status_row.y,
            right.saturating_sub(status_row.x),
            rows,
        ),
    }
}

/// 渲染状态区；返回实际占用行数。
pub(super) fn render(grid: &mut Grid, area: Rect, lx: &Lx) -> u16 {
    let status = &lx.chat.status;
    let rows = rows(area, status);
    let lay = layout(area, status, rows);
    if lay.area.height == 0 {
        return 0;
    }
    let usage = usage_text(status);
    let level = status
        .variant_label
        .clone()
        .unwrap_or_else(|| text::LX_CHAT_TOKENS_UNKNOWN.to_string());
    let model = status
        .model_label
        .clone()
        .unwrap_or_else(|| text::LX_CHAT_NO_MODEL_SHORT.to_string());
    let two_rows = rows >= 2;

    if two_rows {
        grid.put_clipped(
            lay.label.x,
            lay.label.y,
            &model,
            style::muted(),
            lay.label.width,
        );
        let level_line = format!("{SEPARATOR}{level}");
        grid.put_clipped(
            lay.label.x,
            lay.label.y + 1,
            &level_line,
            style::muted(),
            lay.label.width,
        );
        // 用量右对齐、第二行。
        let width = u16::try_from(usage.width())
            .unwrap_or(0)
            .min(lay.label.width);
        let x = lay.label.x + lay.label.width.saturating_sub(width);
        grid.put_clipped(x, lay.label.y + 1, &usage, style::muted(), lay.label.width);
    } else {
        let line = format!("{model}{SEPARATOR}{level}{SEPARATOR}{usage}");
        grid.put_clipped(
            lay.label.x,
            lay.label.y,
            &line,
            style::muted(),
            lay.label.width,
        );
    }

    if let Some(history) = lay.history {
        put_centered(grid, history, text::LX_CHAT_HISTORY_BUTTON, style::accent());
    }
    if let Some(stop) = lay.stop {
        put_centered(grid, stop, text::LX_CHAT_STOP_BUTTON, style::error());
    }
    // 省略提示贴模型行右侧（有空间时）。
    if let Some(omitted) = status.omitted_turns {
        let notice = format!("{} {omitted}", text::LX_CHAT_OMITTED_PREFIX);
        let width = u16::try_from(notice.width()).unwrap_or(u16::MAX);
        if width < lay.label.width {
            let x = lay.label.x + lay.label.width.saturating_sub(width);
            grid.put_clipped(x, lay.label.y, &notice, style::muted(), lay.label.width);
        }
    }
    lay.area.height
}

/// 用量文本：`输入 / 上限`，未知用 `--`。
fn usage_text(status: &ChatStatus) -> String {
    let input = status
        .input_tokens
        .map(format_tokens)
        .unwrap_or_else(|| text::LX_CHAT_TOKENS_UNKNOWN.to_string());
    let limit = status
        .context_limit
        .map(format_tokens)
        .unwrap_or_else(|| text::LX_CHAT_TOKENS_UNKNOWN.to_string());
    format!("{input}{}{limit}", text::LX_CHAT_STATUS_SEPARATOR)
}

/// 大数缩写（128000 → 128k、1200 → 1.2k）。
pub(super) fn format_tokens(value: u64) -> String {
    if value >= 1_000_000 {
        let whole = value / 1_000_000;
        let frac = (value % 1_000_000) / 100_000;
        if frac == 0 {
            format!("{whole}m")
        } else {
            format!("{whole}.{frac}m")
        }
    } else if value >= 1_000 {
        let whole = value / 1_000;
        let frac = (value % 1_000) / 100;
        if frac == 0 {
            format!("{whole}k")
        } else {
            format!("{whole}.{frac}k")
        }
    } else {
        format!("{value}")
    }
}

/// 在矩形内水平居中写入。
fn put_centered(grid: &mut Grid, rect: Rect, content: &str, style: ratatui::style::Style) {
    let width = u16::try_from(content.width()).unwrap_or(u16::MAX);
    if width > rect.width {
        grid.put_clipped(rect.x, rect.y, content, style, rect.width);
        return;
    }
    let x = rect.x + (rect.width - width) / 2;
    grid.put_clipped(x, rect.y, content, style, width);
}

#[cfg(test)]
mod tests;
