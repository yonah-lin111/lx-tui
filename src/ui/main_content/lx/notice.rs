//! 消息底部通知：`■ <标题>` + `  └─ <原因>`（对齐 ui 页 `agent` 分组的 error 步骤）。
//!
//! 标题是**固定枚举**（[`NoticeTitle`]）：文案与配色都由枚举决定，调用点只给一句原因，
//! 不允许随手拼自由标题。标题整行着色只有两档——失败红 + 粗、中断黄 + 粗（没有灰色档）；
//! 原因行连接符用供应商灰 + 粗、正文正常色。
//!
//! **行首标记与聊天步骤共用同一个方块**（[`StepMark`]）：失败为红色实心 `■`、中断为
//! 黄色实心 `■`，与工具行 / 思考头 / 正文块的方块形状、字宽、缩进完全一致（都是 1 列
//! 字形 + 1 列空格）。
//!
//! **不含统计**——本轮统计行由 `history::stats_line` 负责，显示在通知下方（空一行）。

use ratatui::layout::Rect;
use ratatui::style::Style;
use unicode_width::UnicodeWidthStr;

use crate::ui::{style, text};

use super::Grid;
use super::history::{StepKind, StepMark, visible_row, wrap};

/// 原因行的连接符（与 ui 页 agent error 一致）。
const LEAF: &str = text::agent::MARKER_TREE_LEAF;
/// 原因行缩进（两空格，与 ui 页样例一致；同时对齐方块的 `STEP_GUTTER` 缩进）。
const DETAIL_INDENT: &str = "  ";

/// 通知标题：固定枚举，文案与配色都从这里派生（只有红、黄两档，没有灰色档）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum NoticeTitle {
    /// 失败（红 + 粗）：请求 / agent 错误、模型问题。
    Failed,
    /// 中断（黄 + 粗）：本轮未完成（用户停止或异常退出）。
    Interrupted,
}

impl NoticeTitle {
    /// 标题文案（渲染为 `■ <label>`）。
    fn label(self) -> &'static str {
        match self {
            Self::Failed => text::LX_CHAT_NOTICE_FAILED,
            Self::Interrupted => text::LX_CHAT_NOTICE_INTERRUPTED,
        }
    }

    /// 行首方块（与聊天步骤共用同一款标记：失败红实心、中断黄实心）。
    ///
    /// 通知只在**终态**渲染（流中不出通知），因此没有"运行中"的呼吸态，
    /// 直接取 [`StepMark`] 的两个终态。这两个状态是**跨角色**的语义覆盖色，
    /// 与 [`StepKind`] 无关，所以这里传 [`StepKind::Tool`] 只是为了满足签名
    /// （`Failed` / `Interrupted` 会直接命中红/黄，不会落到角色色上）。
    fn mark(self) -> StepMark {
        match self {
            Self::Failed => StepMark::Failed,
            Self::Interrupted => StepMark::Interrupted,
        }
    }

    /// 通知方块样式：状态色（红/黄）优先，与角色无关。
    fn mark_style(self) -> Style {
        self.mark().style(StepKind::Tool, false)
    }

    /// 标题文案样式（不含行首方块：方块自带状态色）。
    fn style(self) -> Style {
        match self {
            Self::Failed => style::agent_fail(),
            Self::Interrupted => style::agent_warn(),
        }
    }

    /// 标题整行的标记段：`■ `（方块 + 1 列空格，与 [`super::history::STEP_GUTTER`] 同宽）。
    ///
    /// 通知只在终态渲染，[`StepMark::Failed`] / [`StepMark::Interrupted`] 的字形与
    /// 呼吸相位无关（恒为实心 `■`），因此这里固定传 0 帧。
    fn marker_segment(self) -> String {
        self.mark().segment(0, false)
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
    let title_rows = wrap(&title_line(notice.title), width).len();
    let prefix_width = u16::try_from(reason_prefix().width()).unwrap_or(u16::MAX);
    let inner = width.saturating_sub(prefix_width);
    let reason_rows = if inner == 0 {
        wrap(&notice.reason, width).len()
    } else {
        wrap(&notice.reason, inner).len()
    };
    title_rows + reason_rows
}

/// 标题整行文本（`■ <label>`）；宽度口径与 [`draw`] 共用。
fn title_line(title: NoticeTitle) -> String {
    format!("{}{}", title.marker_segment(), title.label())
}

/// 绘制通知并推进内容行号；宽度不够放前缀时原因行退回整宽（不画连接符）。
///
/// 行首方块用 [`StepMark`] 的状态色单独绘制（红/黄），其后的标题文案用
/// [`NoticeTitle::style`]——两者在当前配色下同色同字重，但分开画才能保证
/// 「方块永远是步骤方块的样子」，不受标题样式改动影响。
pub(super) fn draw(
    grid: &mut Grid,
    notice: &Notice,
    content: Rect,
    scroll: usize,
    row_index: &mut usize,
) {
    let title_style = notice.title.style();
    let marker = notice.title.marker_segment();
    let marker_width = u16::try_from(marker.width()).unwrap_or(u16::MAX);
    let label = notice.title.label();
    // 标题行按「方块 + 文案」整体换行，但首行要把方块单独着状态色。
    for (index, line) in wrap(&title_line(notice.title), content.width)
        .into_iter()
        .enumerate()
    {
        if let Some(y) = visible_row(content, scroll, *row_index) {
            if index == 0 {
                let mark_style = notice.title.mark_style();
                grid.put_clipped(
                    content.x,
                    y,
                    &marker,
                    mark_style,
                    content.width.min(marker_width),
                );
                let x = content.x.saturating_add(marker_width);
                grid.put_clipped(x, y, label, title_style, content.right().saturating_sub(x));
            } else {
                // 续行：整段仍用标题样式（极端窄宽度下的兜底，不影响常规显示）。
                grid.put_clipped(content.x, y, &line, title_style, content.width);
            }
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
