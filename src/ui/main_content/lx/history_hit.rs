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
use crate::ui::PaneCache;

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
pub(crate) fn history_view(
    area: Rect,
    lx: &Lx,
    cache: &mut PaneCache,
) -> Option<history::HistoryView> {
    let history_rect = history_rect(area, lx)?;
    Some(history::view(history_rect, lx, cache))
}

/// 把指定消息滚到视口居中所需的顶部行与可滚动上限；内容不足一屏时返回 None。
///
/// 消息块高于视口时按起始行对齐（至少能看到开头）；其余情况上下留白尽量相等，
/// 接近列表末尾时自然钳到底部。
pub(crate) fn history_center_target(
    area: Rect,
    lx: &Lx,
    message_id: &str,
    cache: &mut PaneCache,
) -> Option<(usize, usize)> {
    if !chat_geometry_is_live(lx) {
        return None;
    }
    let rect = history_rect(area, lx)?;
    // 只要行数与消息区间：直接读快照，不构造每张卡片的 `CardHit`。
    let snapshot = cache.snapshot(history::content_rect(rect).width, lx);
    let index = lx
        .chat
        .messages
        .iter()
        .position(|message| message.id == message_id)?;
    let rows = snapshot.messages.get(index)?;
    let height = usize::from(rect.height.max(1));
    let max_top = snapshot.total_rows.saturating_sub(height);
    if max_top == 0 {
        return None;
    }
    let block = rows.end.saturating_sub(rows.start).min(height);
    let top = rows
        .start
        .saturating_sub(height.saturating_sub(block) / 2)
        .min(max_top);
    Some((top, max_top))
}

/// 聊天历史滚动条几何（贴右缘一列；无消息或内容未溢出时为 None）。
pub fn history_scrollbar(
    area: Rect,
    lx: &Lx,
    cache: &mut PaneCache,
) -> Option<scrollbar::ScrollbarLayout> {
    if !chat_geometry_is_live(lx) {
        return None;
    }
    let view = history_view(area, lx, cache)?;
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
pub fn history_rows(area: Rect, lx: &Lx, cache: &mut PaneCache) -> Option<(usize, usize)> {
    if !chat_geometry_is_live(lx) {
        return None;
    }
    let view = history_view(area, lx, cache)?;
    Some((view.total_rows, usize::from(view.body.height)))
}

/// 置顶用户消息行的"跳转目标"：该消息起始行（钳到 body 的滚动上限）；无置顶行返回 None。
pub fn sticky_prompt_jump(area: Rect, lx: &Lx, cache: &mut PaneCache) -> Option<usize> {
    if !chat_geometry_is_live(lx) {
        return None;
    }
    let view = history_view(area, lx, cache)?;
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

pub fn chat_hit(
    area: Rect,
    lx: &Lx,
    column: u16,
    row: u16,
    cache: &mut PaneCache,
) -> Option<LxChatHit> {
    if !chat_geometry_is_live(lx) {
        return None;
    }
    // 输入框上方聊天栏右端的「回到底部」控件。
    if let Some(rect) = chat_bar::jump_bottom_rect(area, lx)
        && rect.contains((column, row).into())
    {
        return Some(LxChatHit::JumpBottom);
    }
    let view = history_view(area, lx, cache)?;
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
/// 点击展开与思考块的悬停高亮都从这一个入口取几何：命中区与渲染共用同一份
/// 几何快照（[`PaneCache`]），滚动一变两边一起变。
pub fn card_at(
    area: Rect,
    lx: &Lx,
    column: u16,
    row: u16,
    cache: &mut PaneCache,
) -> Option<history::CardHit> {
    if !chat_geometry_is_live(lx) {
        return None;
    }
    let view = history_view(area, lx, cache)?;
    if !view.body.contains((column, row).into()) {
        return None;
    }
    let width = history::content_rect(view.body).width;
    let snapshot = cache.snapshot(width, lx);
    let content_row = view
        .scroll
        .saturating_add(usize::from(row.saturating_sub(view.body.y)));
    let region = snapshot.cards.iter().find(|region| {
        content_row >= region.card_start
            && content_row < region.card_start.saturating_add(region.card_rows)
    })?;
    // 与 `layout(view.body, ...)` 同一套矩形口径：内容行 0 落在 body 顶。
    //
    // 裁剪必须在 **usize** 空间完成：`card_start` 是内容行绝对下标，直接交给
    // `rows_to_u16` 会在 65535 行处饱和，长历史（10 万行）里所有卡片都"落在视口外"，
    // 点击被静默丢弃。先裁出可见区间，再把**已裁剪的相对偏移**转 u16。
    let clip = |start: usize, rows: usize| -> Option<Rect> {
        let first = start.max(view.scroll);
        let last = start
            .saturating_add(rows)
            .min(view.scroll.saturating_add(usize::from(view.body.height)));
        (last > first).then(|| {
            Rect::new(
                view.body.x,
                view.body
                    .y
                    .saturating_add(history::rows_to_u16(first - view.scroll)),
                width,
                history::rows_to_u16(last - first),
            )
        })
    };
    let card = clip(region.card_start, region.card_rows)?;
    let hit = history::CardHit {
        kind: region.kind.clone(),
        card,
        clickable: region.clickable,
    };
    card.contains((column, row).into()).then_some(hit)
}

/// 指针下的**可折叠内容**：按下时记录目标，松开未拖动即切换折叠。
///
/// 命令卡片的参数区与输出区各自是一块点击面（点一块不连带另一块）；文件工具与
/// 思考块仍是整块点击（折叠态没有正文行，只认正文区就永远点不开）。
/// 展开前后内容不变的区域不登记点击面：点了没反应比不登记更糟。
pub fn card_target_at(
    area: Rect,
    lx: &Lx,
    column: u16,
    row: u16,
    cache: &mut PaneCache,
) -> Option<CardTarget> {
    let hit = card_at(area, lx, column, row, cache)?;
    hit.clickable.then(|| target_of(&hit.kind))
}

/// 指针下的**悬停高亮**目标；只有思考块有悬停态。
///
/// 工具卡片与本地命令卡片的折叠内容不再随鼠标提亮，因此这里只认
/// [`history::CardKind::Thinking`]；点击展开仍走 [`card_target_at`]，与 hover 无关。
pub fn fold_target_at(
    area: Rect,
    lx: &Lx,
    column: u16,
    row: u16,
    cache: &mut PaneCache,
) -> Option<CardTarget> {
    let hit = card_at(area, lx, column, row, cache)?;
    matches!(hit.kind, history::CardKind::Thinking { .. }).then(|| target_of(&hit.kind))
}

/// 命中块 → 可折叠目标。
fn target_of(kind: &history::CardKind) -> CardTarget {
    match kind {
        history::CardKind::Tool {
            message_id,
            call_id,
            section,
            ..
        } => CardTarget::Tool {
            message_id: message_id.clone(),
            call_id: call_id.clone(),
            section: *section,
        },
        history::CardKind::Thinking {
            message_index,
            step_index,
            ..
        } => CardTarget::Thinking {
            message_index: *message_index,
            step_index: *step_index,
        },
        history::CardKind::Shell {
            message_id,
            section,
            ..
        } => CardTarget::Shell {
            message_id: message_id.clone(),
            section: *section,
        },
        history::CardKind::Compact { checkpoint_id, .. } => CardTarget::Compact {
            checkpoint_id: checkpoint_id.clone(),
        },
        history::CardKind::Report {
            message_id,
            agent_id,
            ..
        } => CardTarget::Report {
            message_id: message_id.clone(),
            agent_id: agent_id.clone(),
        },
    }
}
