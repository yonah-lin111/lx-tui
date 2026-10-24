//! 几何快照缓存的失效口径测试。

use ratatui::layout::Rect;

use super::*;
use crate::app::state::{
    AssistantStepView, CardTarget, ChatMessageView, ChatPartView, Lx, LxChat, ToolCallView,
    ToolStatus, ToolSummary,
};
use crate::chat::events::PartKind;
use crate::ui::main_content::lx::history::{Grid, layout, render};

fn text_part(content: &str) -> ChatPartView {
    ChatPartView {
        kind: PartKind::Text,
        content: content.into(),
        provider_metadata_json: None,
        started_at_ms: None,
        ended_at_ms: None,
        completed_at_ms: None,
    }
}

fn message(id: &str, role: &str, status: &str, parts: Vec<ChatPartView>) -> ChatMessageView {
    ChatMessageView {
        sequence: 0,
        id: id.into(),
        role: role.into(),
        status: status.into(),
        turn_index: 0,
        kind: "chat".into(),
        shell: None,
        steps: parts
            .into_iter()
            .enumerate()
            .map(|(index, part)| AssistantStepView::Part {
                position: index as i64,
                part,
            })
            .collect(),
        model_label: None,
        error_code: None,
        input_tokens: None,
        output_tokens: None,
        cache_read_tokens: None,
        duration_ms: None,
    }
}

fn lx_with(messages: Vec<ChatMessageView>) -> Lx {
    let mut lx = Lx::new();
    let mut chat = LxChat::new();
    chat.messages = messages;
    lx.chat = chat;
    lx
}

#[test]
fn same_state_reuses_snapshot() {
    let lx = lx_with(vec![
        message("u1", "user", "complete", vec![text_part("问题")]),
        message("a1", "assistant", "complete", vec![text_part("回答")]),
    ]);
    let mut cache = PaneCache::default();
    let first = cache.snapshot(40, &lx);
    let second = cache.snapshot(40, &lx);
    assert_eq!(cache.builds(), 1, "同一份状态只重建一次");
    assert!(
        std::sync::Arc::ptr_eq(&first, &second),
        "命中时返回同一个 Arc"
    );
}

#[test]
fn appending_text_rebuilds_and_grows_rows() {
    let mut lx = lx_with(vec![message(
        "a1",
        "assistant",
        "streaming",
        vec![text_part("第一行")],
    )]);
    let mut cache = PaneCache::default();
    let before = cache.snapshot(40, &lx);
    let rows_before = before.total_rows;
    let AssistantStepView::Part { part, .. } = &mut lx.chat.messages[0].steps[0] else {
        panic!("assistant 首个步骤应是正文块");
    };
    part.content.push_str("\n第二行\n第三行");
    let after = cache.snapshot(40, &lx);
    assert_eq!(cache.builds(), 2, "正文变化必须重建");
    assert!(after.total_rows > rows_before, "行数随内容增长");
}

#[test]
fn width_change_rebuilds() {
    let lx = lx_with(vec![message(
        "u1",
        "user",
        "complete",
        vec![text_part("一段足够长的用户消息，用于跨宽度折行")],
    )]);
    let mut cache = PaneCache::default();
    let _ = cache.snapshot(60, &lx);
    let _ = cache.snapshot(20, &lx);
    assert_eq!(cache.builds(), 2, "宽度进入指纹");
}

#[test]
fn fold_toggle_rebuilds() {
    let mut lx = lx_with(vec![message(
        "a1",
        "assistant",
        "complete",
        vec![ChatPartView {
            kind: PartKind::Reasoning,
            content: "长思考\n第二行\n第三行".into(),
            provider_metadata_json: None,
            started_at_ms: None,
            ended_at_ms: None,
            completed_at_ms: None,
        }],
    )]);
    let mut cache = PaneCache::default();
    let collapsed = cache.snapshot(40, &lx).total_rows;
    lx.chat.toggle_reasoning(0, 0);
    let expanded = cache.snapshot(40, &lx);
    assert_eq!(cache.builds(), 2, "折叠状态进入指纹");
    assert!(expanded.total_rows > collapsed, "展开后行数增加");
}

#[test]
fn frame_only_state_does_not_rebuild() {
    let mut lx = lx_with(vec![message(
        "a1",
        "assistant",
        "streaming",
        vec![text_part("回答")],
    )]);
    let mut cache = PaneCache::default();
    let _ = cache.snapshot(40, &lx);
    lx.chat.hover = Some(CardTarget::Thinking {
        message_index: 0,
        step_index: 0,
    });
    lx.chat.follow_bottom = false;
    lx.chat.loading_frame = 42;
    lx.chat.history_scroll = 7;
    let _ = cache.snapshot(40, &lx);
    assert_eq!(cache.builds(), 1, "hover/跟随/加载帧/滚动不进指纹");
}

#[test]
fn tool_output_change_rebuilds() {
    let mut lx = lx_with(vec![message("a1", "assistant", "complete", Vec::new())]);
    let mut call = ToolCallView::new("call-1", "exec_command", 0);
    call.status = ToolStatus::Completed;
    call.summary = Some(Box::new(ToolSummary::default()));
    lx.chat.messages[0]
        .steps
        .push(AssistantStepView::Tool { position: 0, call });
    let mut cache = PaneCache::default();
    let _ = cache.snapshot(40, &lx);
    if let AssistantStepView::Tool { call, .. } = &mut lx.chat.messages[0].steps[0] {
        call.status = ToolStatus::Running;
    }
    let _ = cache.snapshot(40, &lx);
    assert_eq!(cache.builds(), 2, "工具状态进入指纹");
}

#[test]
fn viewport_render_still_matches_layout_total_rows() {
    // 200 行视口 + 100 条消息：视口外的条目被跳过，但返回值必须仍是总行数。
    let messages: Vec<ChatMessageView> = (0..100)
        .map(|index| {
            let role = if index % 2 == 0 { "user" } else { "assistant" };
            message(
                &format!("m{index}"),
                role,
                "complete",
                vec![text_part(&format!("第 {index} 条消息\n第二行"))],
            )
        })
        .collect();
    let lx = lx_with(messages);
    let area = Rect::new(0, 0, 40, 200);
    let mut cache = PaneCache::default();
    let total = layout(area, &lx, &mut cache).total_rows;
    let mut grid = Grid::new(area);
    let drawn = render(&mut grid, area, &lx, total.saturating_sub(50), &mut cache);
    assert_eq!(drawn, total, "视口化渲染仍推进到总行数");
}

#[test]
fn one_frame_geometry_and_draw_share_the_snapshot() {
    // 一帧里布局、命中、绘制都走同一份快照：同宽度下只重建一次。
    let lx = lx_with(vec![
        message("u1", "user", "complete", vec![text_part("问题")]),
        message("a1", "assistant", "complete", vec![text_part("回答")]),
    ]);
    let area = Rect::new(0, 0, 40, 20);
    let mut cache = PaneCache::default();
    let _ = layout(area, &lx, &mut cache);
    let mut grid = Grid::new(area);
    let _ = render(&mut grid, area, &lx, 0, &mut cache);
    let _ = layout(area, &lx, &mut cache);
    assert_eq!(cache.builds(), 1, "同宽度同状态不重复建快照");
}
