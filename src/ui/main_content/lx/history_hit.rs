//! lx 聊天历史的命中与视口入口：置顶行、可折叠卡片内容，以及滚动条/行数与
//! 置顶跳转的几何口径。
//!
//! 从 `lx.rs` 拆出（原文件超出 1000 行上限）；行为与导出保持不变。命中只读状态，
//! **不修改任何东西**，也不做 IO。

use ratatui::layout::Rect;

use crate::app::state::{CardTarget, Lx};
use crate::ui::widgets::scrollbar;

use super::chat_bar;
use super::history;
use super::history_rect;

/// lx 页聊天命中结果。
///
/// 代码块复制按钮携带正文（自有 `String`），因此不是 `Copy`。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LxChatHit {
    /// 点击置顶用户消息行（跳到该 prompt）。
    PromptHeader,
    /// 点击「回到底部」按钮。
    JumpBottom,
}

/// 聊天历史视图（滚动偏移 / body / 置顶行结论）；无历史区时为 None。
///
/// 渲染、滚轮钳制、滚动条与命中**统一**走它，保证"可见行数"只有一个口径。
pub(crate) fn history_view(area: Rect, lx: &Lx) -> Option<history::HistoryView> {
    let history_rect = history_rect(area, lx)?;
    Some(history::view(history_rect, lx))
}

/// 聊天历史滚动条几何（贴右缘一列；无消息或内容未溢出时为 None）。
pub fn history_scrollbar(area: Rect, lx: &Lx) -> Option<scrollbar::ScrollbarLayout> {
    if !chat_geometry_is_live(lx) {
        return None;
    }
    let view = history_view(area, lx)?;
    history::scrollbar(
        view.body,
        view.total_rows,
        usize::from(view.body.height),
        view.scroll,
    )
}

/// 聊天历史的（渲染行数, 视口行数）；无消息时为 None。滚轮滚动按此钳制。
///
/// 视口行数取 **body**（置顶行让位后），与渲染同口径——否则滚不到最后一行。
pub fn history_rows(area: Rect, lx: &Lx) -> Option<(usize, usize)> {
    let view = history_view(area, lx)?;
    Some((view.total_rows, usize::from(view.body.height)))
}

/// 置顶用户消息行的"跳转目标"：该消息起始行（钳到 body 的滚动上限）；无置顶行返回 None。
pub fn sticky_prompt_jump(area: Rect, lx: &Lx) -> Option<usize> {
    if !chat_geometry_is_live(lx) {
        return None;
    }
    let view = history_view(area, lx)?;
    let message = view.sticky?;
    let start = view.messages.get(message)?.start;
    // 跳过去后置顶行会让位，按整块 body 钳制。
    let max_top = view
        .total_rows
        .saturating_sub(usize::from(view.area.height));
    Some(start.min(max_top))
}

/// 聊天模式命中测试：回到底部按钮 > 置顶行 > thinking 头部。
/// 聊天区的几何是否「正在被画出来」——**所有基于历史几何的命中入口都必须先问它**。
///
/// 后台任务面板打开时，输出容器与历史区是**同一块矩形**，但历史根本没画（`build_chat`
/// 只在没有面板时画）。命中若照旧按历史几何作答，输出区就成了一个巨大的点击穿透面：
/// 点置顶行会跳一块看不见的历史、点代码块 `[❐]` 会复制看不见的代码、点 thinking
/// 头部会展开看不见的块。把判定收在这里，避免每个入口各写一遍、反复漏。
pub(super) fn chat_geometry_is_live(lx: &Lx) -> bool {
    lx.task.is_none() && super::history::has_history(lx)
}

pub fn chat_hit(area: Rect, lx: &Lx, column: u16, row: u16) -> Option<LxChatHit> {
    if !chat_geometry_is_live(lx) {
        return None;
    }
    // 输入框上方聊天栏右端的「回到底部」控件。
    if let Some(rect) = chat_bar::jump_bottom_rect(area, lx)
        && rect.contains((column, row).into())
    {
        return Some(LxChatHit::JumpBottom);
    }
    let view = history_view(area, lx)?;
    // 置顶行（表现层，整区第 0 行整宽）：点击跳回该 prompt。
    if let Some(sticky) = history::sticky_rect(&view)
        && sticky.rect.contains((column, row).into())
    {
        return Some(LxChatHit::PromptHeader);
    }
    // 所有命中都限定在 body 内：内容行换算出的屏幕行可能落在 body 之下（正文比视口
    // 长时），不设边界就会把聊天栏/输入框上的点击误判成「点了看不见的头部」。
    if !view.body.contains((column, row).into()) {
        return None;
    }
    None
}

/// 鼠标下的可交互块（工具卡片 / 思考块）的整块屏幕矩形。
///
/// 悬停高亮与「点展开正文折叠」都从这一个入口取几何：命中区与渲染共用
/// [`history::layout`]，滚动一变两边一起变。
pub fn card_at(area: Rect, lx: &Lx, column: u16, row: u16) -> Option<history::CardHit> {
    if !chat_geometry_is_live(lx) {
        return None;
    }
    let view = history_view(area, lx)?;
    if !view.body.contains((column, row).into()) {
        return None;
    }
    history::layout(view.body, lx)
        .cards
        .into_iter()
        .find_map(|hit| {
            let card = screen_rect(&view, hit.card)?;
            card.contains((column, row).into()).then_some(hit)
        })
}

/// 指针下的**可折叠内容**：按下时记录目标，松开未拖动即切换折叠。
///
/// **整块都是点击面**：折叠态点标题/内容展开，展开态点内容收起。早先只认「正文区」
/// 是真实 bug——折叠态没有正文行，于是折叠的思考和长参数永远点不开。
pub fn card_target_at(area: Rect, lx: &Lx, column: u16, row: u16) -> Option<CardTarget> {
    let hit = card_at(area, lx, column, row)?;
    Some(target_of(&hit.kind))
}

/// 指针是否落在**折叠内容**上（hover 提亮判据）。
///
/// 与 [`card_target_at`] 分开：点击覆盖整块（折叠态没有正文行），而 hover 只该点亮
/// 真正被折叠的内容——展开后没有折叠内容、工具参数不在折叠内容里，都不提亮。
pub fn fold_target_at(area: Rect, lx: &Lx, column: u16, row: u16) -> Option<CardTarget> {
    let hit = card_at(area, lx, column, row)?;
    let view = history_view(area, lx)?;
    let fold = screen_rect(&view, hit.fold)?;
    fold.contains((column, row).into())
        .then(|| target_of(&hit.kind))
}

/// 命中块 → 可折叠目标。
fn target_of(kind: &history::CardKind) -> CardTarget {
    match kind {
        history::CardKind::Tool {
            message_id,
            call_id,
            ..
        } => CardTarget::Tool {
            message_id: message_id.clone(),
            call_id: call_id.clone(),
        },
        history::CardKind::Thinking {
            message_index,
            step_index,
            ..
        } => CardTarget::Thinking {
            message_index: *message_index,
            step_index: *step_index,
        },
    }
}

/// 内容行矩形 → 屏幕矩形（按滚动窗口裁剪）；完全在视口上方时为 None。
fn screen_rect(view: &history::HistoryView, rect: Rect) -> Option<Rect> {
    if rect.width == 0 || rect.height == 0 {
        return None;
    }
    let offset = usize::from(rect.y.saturating_sub(view.body.y));
    let skip = view.scroll.saturating_sub(offset);
    if skip >= usize::from(rect.height) {
        return None;
    }
    let y = view
        .body
        .y
        .saturating_add(u16::try_from(offset.saturating_sub(view.scroll)).unwrap_or(u16::MAX));
    let height = rect
        .height
        .saturating_sub(u16::try_from(skip).unwrap_or(u16::MAX));
    let bottom = y.saturating_add(height).min(view.body.bottom());
    (bottom > y).then(|| Rect::new(view.body.x, y, view.body.width, bottom - y))
}
