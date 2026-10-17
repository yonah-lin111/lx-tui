//! 消息底部通知：`! <标题>` + `  └─ <原因>`（对齐 ui 页 `agent` 分组的 error 步骤）。
//!
//! 标题是**固定枚举**（[`NoticeTitle`]）：文案与配色都由枚举决定，调用点只给一句原因，
//! 不允许随手拼自由标题。标题整行按级别着色：失败红 + 粗、警告黄 + 粗、提示用窗格底部
//! 供应商同款次要信息色；原因行连接符用供应商灰 + 粗、正文正常色。
//!
//! **不含统计**——本轮统计行由 `history::stats_line` 负责，显示在通知下方（空一行）。

use ratatui::layout::Rect;
use ratatui::style::Style;
use unicode_width::UnicodeWidthStr;

use crate::ui::{style, text};

use super::Grid;
use super::history::{visible_row, wrap};

/// 标题行的标记（与 ui 页 agent error 一致）。
const MARKER: &str = text::agent::ICON_ERROR;
/// 原因行的连接符（与 ui 页 agent error 一致）。
const LEAF: &str = text::agent::MARKER_TREE_LEAF;
/// 原因行缩进（两空格，与 ui 页样例一致）。
const DETAIL_INDENT: &str = "  ";

/// 通知标题：固定枚举，文案与配色都从这里派生。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum NoticeTitle {
    /// 失败（红 + 粗）：请求 / agent 错误、模型问题。
    Failed,
    /// 中断（次要信息色）：本轮未完成（用户停止或异常退出）。
    Interrupted,
    /// 警告（黄 + 粗）：非致命问题，留给模型重试等（当前无构造点）。
    #[allow(dead_code)]
    Warning,
}

impl NoticeTitle {
    /// 标题文案（渲染为 `! <label>`）。
    fn label(self) -> &'static str {
        match self {
            Self::Failed => text::LX_CHAT_NOTICE_FAILED,
            Self::Interrupted => text::LX_CHAT_NOTICE_INTERRUPTED,
            Self::Warning => text::LX_CHAT_NOTICE_WARNING,
        }
    }

    /// 标题整行样式。
    fn style(self) -> Style {
        match self {
            Self::Failed => style::agent_fail(),
            Self::Interrupted => style::chat_provider(),
            Self::Warning => style::agent_warn(),
        }
    }
}

/// 一条消息级通知：固定标题 + **一句话原因**；由 `history` 从消息终态派生。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Notice {
    pub title: NoticeTitle,
    pub reason: String,
}

impl Notice {
    pub(super) fn new(title: NoticeTitle, reason: impl Into<String>) -> Self {
        Self {
            title,
            reason: reason.into(),
        }
    }
}

/// 原因行前缀（缩进 + 连接符 + 空格）；列宽固定，续行按它对齐。
fn reason_prefix() -> String {
    format!("{DETAIL_INDENT}{LEAF} ")
}

/// 通知占用的渲染行数（与 [`draw`] 同口径）。
pub(super) fn rows(notice: &Notice, width: u16) -> usize {
    let title_rows = wrap(&format!("{MARKER} {}", notice.title.label()), width).len();
    let prefix_width = u16::try_from(reason_prefix().width()).unwrap_or(u16::MAX);
    let inner = width.saturating_sub(prefix_width);
    let reason_rows = if inner == 0 {
        wrap(&notice.reason, width).len()
    } else {
        wrap(&notice.reason, inner).len()
    };
    title_rows + reason_rows
}

/// 绘制通知并推进内容行号；宽度不够放前缀时原因行退回整宽（不画连接符）。
pub(super) fn draw(
    grid: &mut Grid,
    notice: &Notice,
    content: Rect,
    scroll: usize,
    row_index: &mut usize,
) {
    let title_style = notice.title.style();
    for line in wrap(&format!("{MARKER} {}", notice.title.label()), content.width) {
        if let Some(y) = visible_row(content, scroll, *row_index) {
            grid.put_clipped(content.x, y, &line, title_style, content.width);
        }
        *row_index += 1;
    }
    let prefix = reason_prefix();
    let prefix_width = u16::try_from(prefix.width()).unwrap_or(u16::MAX);
    let inner = content.width.saturating_sub(prefix_width);
    let lines = if inner == 0 {
        wrap(&notice.reason, content.width)
    } else {
        wrap(&notice.reason, inner)
    };
    for (index, line) in lines.into_iter().enumerate() {
        if let Some(y) = visible_row(content, scroll, *row_index) {
            if index == 0 && inner > 0 {
                grid.put_clipped(
                    content.x,
                    y,
                    &prefix,
                    style::chat_notice_connector(),
                    prefix_width,
                );
            }
            let x = content
                .x
                .saturating_add(if inner == 0 { 0 } else { prefix_width });
            grid.put_clipped(
                x,
                y,
                &line,
                style::text(),
                content.right().saturating_sub(x),
            );
        }
        *row_index += 1;
    }
}

#[cfg(test)]
mod tests;
