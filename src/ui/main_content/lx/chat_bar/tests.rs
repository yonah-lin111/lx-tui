//! 聊天栏（输入框上方一行）的组件级测试：文案派生、统计优先布局、宽字符边界、
//! 极窄降级、Esc 待命组合与扫描动画。
//!
//! 只构造状态并在独立网格上画这一行，不渲染整页、不启动终端。

use ratatui::layout::Rect;

use super::*;
use crate::app::state::{
    AssistantStepView, ChatMessageView, ChatPartView, LxChat, PlanSnapshotView, PlanStepView,
    RequestKey, RequestState, ToolCallView, ToolStatus,
};
use crate::chat::events::PartKind;
use crate::layout::PaneId;
use crate::ui::text;

/// 测试统计文案：耗时 2.7s 取整 3s + 1280 个 ASCII 字符 ≈ 320 输出 token。
const STATS: &str = "(3s · ↓320 tok)";

/// 正文步骤。
fn text_step(content: &str) -> AssistantStepView {
    AssistantStepView::Part {
        position: 0,
        part: ChatPartView {
            kind: PartKind::Text,
            content: content.into(),
            provider_metadata_json: None,
            started_at_ms: None,
            ended_at_ms: None,
            completed_at_ms: None,
        },
    }
}

/// 思考步骤；`open` 表示仍是开放思考块。
fn reasoning_step(position: i64, open: bool) -> AssistantStepView {
    AssistantStepView::Part {
        position,
        part: ChatPartView {
            kind: PartKind::Reasoning,
            content: "先想一下".into(),
            provider_metadata_json: None,
            started_at_ms: None,
            ended_at_ms: None,
            completed_at_ms: (!open).then_some(1_000),
        },
    }
}

/// 工具步骤。
fn tool_step(position: i64, status: ToolStatus) -> AssistantStepView {
    AssistantStepView::Tool {
        position,
        call: ToolCallView {
            call_id: format!("call-{position}"),
            name: "read".into(),
            position,
            arguments_json: String::new(),
            status,
            summary: None,
            error: None,
            result_json: None,
            detail: None,
        },
    }
}

/// 一条 assistant 消息。
fn assistant(id: &str, status: &str, steps: Vec<AssistantStepView>) -> ChatMessageView {
    ChatMessageView {
        id: id.into(),
        sequence: 0,
        kind: "chat".into(),
        role: "assistant".into(),
        status: status.into(),
        turn_index: 0,
        shell: None,
        steps,
        model_label: None,
        error_code: None,
        input_tokens: None,
        output_tokens: None,
        cache_read_tokens: None,
        duration_ms: None,
        reports: Vec::new(),
    }
}

/// 一条本地命令记录（可追加在流式 assistant 之后）。
fn shell_message(id: &str) -> ChatMessageView {
    ChatMessageView {
        kind: "user_shell".into(),
        ..assistant(id, "complete", vec![text_step("$ ls")])
    }
}

/// 指向 `id` 的流中请求状态。
fn streaming_request(id: &str) -> RequestState {
    RequestState::Streaming {
        key: RequestKey {
            pane: PaneId::alloc(),
            session: "session".into(),
            generation: 1,
        },
        assistant_id: id.into(),
    }
}

/// 流中的 lx：唯一一条 assistant 消息（`id`）+ 给定步骤。
fn streaming_lx(id: &str, steps: Vec<AssistantStepView>) -> Lx {
    let mut lx = Lx::new();
    let mut chat = LxChat::new();
    chat.messages.push(assistant(id, "streaming", steps));
    chat.request = streaming_request(id);
    chat.status.streaming = true;
    lx.chat = chat;
    lx
}

/// 计划快照。
fn plan(steps: &[(&str, PlanStepView)]) -> PlanSnapshotView {
    PlanSnapshotView {
        revision: 1,
        explanation: None,
        steps: steps
            .iter()
            .map(|(step, status)| ((*step).to_string(), *status))
            .collect(),
    }
}

/// 让统计稳定为 [`STATS`]：2.7s 耗时 + 1280 个 ASCII 字符 ≈ 320 输出 token。
fn with_stats(lx: &mut Lx) {
    lx.chat.turn_elapsed_ms = Some(2_700);
    if let Some(message) = lx.chat.messages.last_mut()
        && let Some(part) = message.steps.first_mut().and_then(|step| step.part_mut())
    {
        part.content = "a".repeat(1_280);
    }
}

/// 只画聊天栏（不渲染整页，也不经过页面布局）：返回网格与栏矩形。
fn draw_bar(lx: &Lx, width: u16) -> (Grid, Rect) {
    let bar = Rect::new(0, 0, width, HEIGHT);
    let mut grid = Grid::new(bar);
    draw(&mut grid, Some(bar), lx);
    (grid, bar)
}

/// 栏内整行文本：宽字符占位列丢弃，因此 `row` 的**显示宽度**等于列数偏移，
/// 可以直接用 [`col_of`] 换算列号（也便于 `contains` 断言中文片段）。
fn row_text(grid: &Grid, bar: Rect) -> String {
    (bar.x..bar.right())
        .map(|x| grid.cell(x, bar.y).ch)
        .filter(|ch| *ch != crate::ui::main_content::lx::grid::CONTINUATION)
        .collect()
}

/// 栏内整行样式（按列，含宽字符占位列）。
fn row_styles(grid: &Grid, bar: Rect) -> Vec<ratatui::style::Style> {
    (bar.x..bar.right())
        .map(|x| grid.cell(x, bar.y).style)
        .collect()
}

/// `needle` 在整行里的**显示列**下标（行内可能有中文，不能用字节下标）。
fn col_of(row: &str, needle: &str) -> u16 {
    let byte = row.find(needle).expect("行内应包含目标文案");
    u16::try_from(row[..byte].width()).unwrap_or(u16::MAX)
}

/// 第 `col` 列上的字符；该列是宽字符的后半格时返回其前半格字符。
fn char_at_col(grid: &Grid, bar: Rect, col: u16) -> Option<char> {
    let x = bar.x.checked_add(col)?;
    if x >= bar.right() {
        return None;
    }
    match grid.cell(x, bar.y).ch {
        crate::ui::main_content::lx::grid::CONTINUATION => {
            (x > bar.x).then(|| grid.cell(x - 1, bar.y).ch)
        }
        ch => Some(ch),
    }
}

/// `needle` 前一个**显示列**上的字符（按列回退，宽字符两列都指向同一个字）。
fn char_before(row: &str, needle: &str) -> Option<char> {
    let byte = row.find(needle)?;
    row[..byte].chars().next_back()
}

mod animation;
mod compacting;
mod label;
mod layout;
mod stop_and_lifecycle;
