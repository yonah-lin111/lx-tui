//! lx 聊天历史的命中与视口入口：置顶行、thinking 头部、代码复制、工具卡片，
//! 以及滚动条/行数与置顶跳转的几何口径。
//!
//! 从 `lx.rs` 拆出（原文件超出 1000 行上限）；行为与导出保持不变。命中只读状态，
//! **不修改任何东西**，也不做 IO。

use ratatui::layout::Rect;

use crate::app::state::{ChatHover, Lx};
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
    /// 点击 thinking 折叠块头部（消息索引, part 索引）。
    Thinking(usize, usize),
    /// 点击「回到底部」按钮。
    JumpBottom,
    /// 点击代码块头部的复制按钮；`text` 为该块正文。
    CodeCopy(String),
    /// 点击进程卡片的 `open` 行：打开后台任务面板并把高亮定位到该进程。
    ///
    /// `None` 表示卡片没有可定位的进程（理论上不会出现：文件工具与计划卡片不登记
    /// 该命中区），此时只打开面板、高亮落在列表首项。
    ToolOpen(Option<u64>),
    /// 点击文件差异卡片末尾的动作行：切换整张卡片的展开/收起。
    ///
    /// 与 [`Self::ToolOpen`] 是**两种不同动作**，不能靠 `session_id` 是否为空猜测。
    ToolToggle { message_id: String, call_id: String },
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
    lx.task.is_none() && !lx.chat.messages.is_empty()
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
    // thinking 头部命中（按 body + 滚动窗口换算屏幕坐标）。
    for (message_index, part_index, header) in history::layout(view.body, lx).thinking {
        let offset = header.y.saturating_sub(view.body.y);
        if (offset as usize) < view.scroll {
            continue;
        }
        let screen_y = view
            .body
            .y
            .saturating_add(u16::try_from(offset as usize - view.scroll).unwrap_or(u16::MAX));
        let rect = Rect::new(view.body.x, screen_y, view.body.width, 1);
        if rect.contains((column, row).into()) {
            return Some(LxChatHit::Thinking(message_index, part_index));
        }
    }
    // 代码块头部的复制按钮（同样按 body + 滚动窗口换算屏幕坐标）。
    for button in code_copy_hits(area, lx) {
        if button.rect.contains((column, row).into()) {
            return Some(LxChatHit::CodeCopy(button.body));
        }
    }
    // 工具卡片的可点击行：进程 `open` 打开任务面板，文件卡片末尾动作行切换展开。
    if let Some(hit) = tool_hit_at(&view, lx, column, row) {
        return Some(match hit.kind {
            history::ToolHitKind::OpenTask { session_id } => LxChatHit::ToolOpen(session_id),
            history::ToolHitKind::Toggle => LxChatHit::ToolToggle {
                message_id: hit.message_id,
                call_id: hit.call_id,
            },
        });
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

/// 指针落在哪张卡片上（悬停判据）；与 [`card_at`] 同一份几何。
pub fn chat_hover(area: Rect, lx: &Lx, column: u16, row: u16) -> Option<ChatHover> {
    card_at(area, lx, column, row).map(|hit| hover_of(&hit.kind))
}

/// 指针是否落在**已展开卡片的正文**上：按下时记录目标，松开未拖动即折叠。
pub fn card_body_at(area: Rect, lx: &Lx, column: u16, row: u16) -> Option<ChatHover> {
    let hit = card_at(area, lx, column, row)?;
    let view = history_view(area, lx)?;
    let body = screen_rect(&view, hit.body)?;
    body.contains((column, row).into())
        .then(|| hover_of(&hit.kind))
}

/// 命中块 → 状态层悬停标识。
fn hover_of(kind: &history::CardKind) -> ChatHover {
    match kind {
        history::CardKind::Tool {
            message_id,
            call_id,
            ..
        } => ChatHover::Tool {
            message_id: message_id.clone(),
            call_id: call_id.clone(),
        },
        history::CardKind::Thinking {
            message_index,
            step_index,
            ..
        } => ChatHover::Thinking {
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

/// 包装卡片标题行命中：按 body + 滚动窗口把内容行换算成屏幕坐标。
fn tool_hit_at(
    view: &history::HistoryView,
    lx: &Lx,
    column: u16,
    row: u16,
) -> Option<history::ToolHit> {
    history::layout(view.body, lx)
        .tools
        .into_iter()
        .find(|hit| {
            let offset = hit.rect.y.saturating_sub(view.body.y);
            if (offset as usize) < view.scroll {
                return false;
            }
            let screen_y = view
                .body
                .y
                .saturating_add(u16::try_from(offset as usize - view.scroll).unwrap_or(u16::MAX));
            Rect::new(view.body.x, screen_y, view.body.width, 1).contains((column, row).into())
        })
}

/// 代码块复制按钮的**屏幕矩形**（按当前滚动偏移换算）与正文。
///
/// 渲染、命中与 tooltip 共用本入口：滚动位置一变，三者一起跟着变，不会漂移。
pub fn code_copy_hits(area: Rect, lx: &Lx) -> Vec<history::CodeCopyButton> {
    if !chat_geometry_is_live(lx) {
        return Vec::new();
    }
    let Some(view) = history_view(area, lx) else {
        return Vec::new();
    };
    history::layout(view.body, lx)
        .code_copy
        .into_iter()
        .filter_map(|mut button| {
            let offset = button.rect.y.saturating_sub(view.body.y);
            if (offset as usize) < view.scroll {
                return None;
            }
            button.rect.y = view
                .body
                .y
                .saturating_add(u16::try_from(offset as usize - view.scroll).unwrap_or(u16::MAX));
            Some(button)
        })
        .collect()
}
