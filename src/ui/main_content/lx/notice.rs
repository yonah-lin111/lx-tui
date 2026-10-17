//! 消息底部通知：`! <标题>` + 可选 `  └─ <细节>`（对齐 ui 页 `agent` 分组的 error 步骤）。
//!
//! 标题整行按级别着色：错误红 + 粗、警告黄 + 粗、提示用底部供应商同款次要信息色；
//! 细节行连接符压暗、正文正常色。**不含统计**——本轮统计行由 `history::stats_line`
//! 紧随其后绘制（统计是消息的 footer，通知只回答“这轮发生了什么”）。

use ratatui::layout::Rect;
use ratatui::style::Style;
use unicode_width::UnicodeWidthStr;

use crate::ui::{style, text};

use super::Grid;
use super::history::{visible_row, wrap};

/// 标题行的标记（与 ui 页 agent error 一致）。
const MARKER: &str = text::agent::ICON_ERROR;
/// 细节行的连接符（与 ui 页 agent error 一致）。
const LEAF: &str = text::agent::MARKER_TREE_LEAF;
/// 细节行缩进（两空格，与 ui 页样例一致）。
const DETAIL_INDENT: &str = "  ";

/// 通知级别：决定标题整行颜色。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum NoticeKind {
    /// 失败（红 + 粗）。
    Error,
    /// 非致命提示（黄 + 粗）：模型问题、重试进度等。
    ///
    /// 当前没有构造点（重试在传输层静默进行），先由组件支持住该级别。
    #[allow(dead_code)]
    Warning,
    /// 普通提示（次要信息色，与窗格底部供应商同款）。
    Info,
}

/// 一条消息级通知；由 `history` 从消息终态派生，渲染在统计行上方。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Notice {
    pub kind: NoticeKind,
    pub title: String,
    pub detail: Option<String>,
}

impl Notice {
    /// 失败通知：细节为脱敏后的失败原因码（标题见 `text::LX_CHAT_NOTICE_FAILED`）。
    pub(super) fn error(title: &str, detail: Option<String>) -> Self {
        Self {
            kind: NoticeKind::Error,
            title: title.to_string(),
            detail,
        }
    }

    /// 提示通知：如用户双击 Esc 停止本轮。
    pub(super) fn info(title: &str) -> Self {
        Self {
            kind: NoticeKind::Info,
            title: title.to_string(),
            detail: None,
        }
    }
}

/// 标题整行样式。
fn title_style(kind: NoticeKind) -> Style {
    match kind {
        NoticeKind::Error => style::agent_fail(),
        NoticeKind::Warning => style::agent_warn(),
        NoticeKind::Info => style::chat_provider(),
    }
}

/// 细节行前缀（缩进 + 连接符 + 空格）；列宽固定，续行按它对齐。
fn detail_prefix() -> String {
    format!("{DETAIL_INDENT}{LEAF} ")
}

/// 通知占用的渲染行数（与 [`draw`] 同口径）。
pub(super) fn rows(notice: &Notice, width: u16) -> usize {
    let mut rows = wrap(&format!("{MARKER} {}", notice.title), width).len();
    if let Some(detail) = notice.detail.as_deref() {
        let prefix_width = u16::try_from(detail_prefix().width()).unwrap_or(u16::MAX);
        let inner = width.saturating_sub(prefix_width);
        rows += if inner == 0 {
            wrap(detail, width).len()
        } else {
            wrap(detail, inner).len()
        };
    }
    rows
}

/// 绘制通知并推进内容行号；宽度不够放前缀时细节行退回整宽（不画连接符）。
pub(super) fn draw(
    grid: &mut Grid,
    notice: &Notice,
    content: Rect,
    scroll: usize,
    row_index: &mut usize,
) {
    let title_style = title_style(notice.kind);
    for line in wrap(&format!("{MARKER} {}", notice.title), content.width) {
        if let Some(y) = visible_row(content, scroll, *row_index) {
            grid.put_clipped(content.x, y, &line, title_style, content.width);
        }
        *row_index += 1;
    }
    let Some(detail) = notice.detail.as_deref() else {
        return;
    };
    let prefix = detail_prefix();
    let prefix_width = u16::try_from(prefix.width()).unwrap_or(u16::MAX);
    let inner = content.width.saturating_sub(prefix_width);
    let lines = if inner == 0 {
        wrap(detail, content.width)
    } else {
        wrap(detail, inner)
    };
    for (index, line) in lines.into_iter().enumerate() {
        if let Some(y) = visible_row(content, scroll, *row_index) {
            if index == 0 && inner > 0 {
                grid.put_clipped(
                    content.x,
                    y,
                    &prefix,
                    style::agent_connector(),
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
