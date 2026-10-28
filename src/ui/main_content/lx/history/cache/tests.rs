//! 几何快照缓存的失效口径测试。

use std::sync::Arc;

use ratatui::layout::Rect;

use super::*;
use crate::app::state::{
    AssistantStepView, CardTarget, ChatMessageView, ChatPartView, Lx, LxChat, ToolCallView,
    ToolStatus, ToolSummary,
};
use crate::chat::events::PartKind;
use crate::ui::main_content::lx::history::paint::BlockPaint;
use crate::ui::main_content::lx::history::snapshot::HistorySnapshot;
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
        reports: Vec::new(),
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

/// 汇报块展开必须进列表指纹，否则快照不重建、绘制按新状态溢出到下一块。
#[test]
fn report_expansion_rebuilds_only_the_clicked_block() {
    let report = |agent: &str, title: &str| crate::chat::agent::SubagentReportView {
        agent_id: agent.into(),
        title: title.into(),
        role: crate::chat::agent::AgentRole::Explorer,
        status: "completed".into(),
        files_changed: 0,
        duration_ms: Some(1_000),
        body: "line one\nline two\nline three\nline four".into(),
        delivered_at_ms: 1,
    };
    let mut row = message("report-row", "user", "complete", Vec::new());
    row.reports = vec![report("r1", "first"), report("r2", "second")];
    let answer = message("answer", "assistant", "complete", Vec::new());
    let mut lx = lx_with(vec![row, answer]);
    let mut cache = PaneCache::default();
    let before = cache.snapshot(40, &lx).total_rows;
    lx.chat.toggle_detail("report-row", "report:r1");
    let snapshot = cache.snapshot(40, &lx);
    assert!(snapshot.total_rows > before, "展开后必须重建快照并增加行数");
    assert!(cache.builds() >= 2, "列表指纹必须因展开状态变化");
    assert!(lx.chat.detail_is_expanded("report-row", "report:r1"));
    assert!(
        !lx.chat.detail_is_expanded("report-row", "report:r2"),
        "只展开被点中的汇报"
    );
}

/// 回归：点 `… N more lines` 追加批次后必须重新量行。
///
/// 列表指纹只带展开状态、漏了 `report_expand_steps`：追加一批后快照仍返回旧几何，
/// 绘制却按新批次数多写 24 行，后面的块被覆盖，表现为正文重叠。
#[test]
fn report_more_expansion_rebuilds_snapshot_geometry() {
    let body = (0..60)
        .map(|index| format!("line {index}"))
        .collect::<Vec<_>>()
        .join("\n");
    let mut row = message("report-row", "user", "complete", Vec::new());
    row.reports.push(crate::chat::agent::SubagentReportView {
        agent_id: "r1".into(),
        title: "report".into(),
        role: crate::chat::agent::AgentRole::Explorer,
        status: "completed".into(),
        files_changed: 0,
        duration_ms: Some(1_000),
        body,
        delivered_at_ms: 1,
    });
    let answer = message("answer", "assistant", "complete", vec![text_part("tail")]);
    let mut lx = lx_with(vec![row, answer]);
    assert!(lx.chat.toggle_detail("report-row", "report:r1"));

    let mut cache = PaneCache::default();
    let before = cache.snapshot(60, &lx);
    let before_total = before.total_rows;
    let before_answer = before.messages[1].start;

    lx.chat.expand_report_more("report-row", "report:r1");
    let after = cache.snapshot(60, &lx);

    assert_eq!(
        after.total_rows,
        before_total + 24,
        "追加一批后总行数必须增加，不能复用旧快照"
    );
    assert_eq!(
        after.messages[1].start,
        before_answer + 24,
        "后续消息必须整体下移，不能被汇报正文覆盖"
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

/// 快照里按时间线顺序取每块的绘制数据。
fn paints(snapshot: &HistorySnapshot) -> Vec<Arc<BlockPaint>> {
    snapshot
        .entries
        .iter()
        .flat_map(|entry| entry.blocks.iter())
        .map(|block| Arc::clone(&block.paint))
        .collect()
}

#[test]
fn identical_frames_reuse_the_same_paints() {
    let lx = lx_with(vec![message(
        "u1",
        "user",
        "complete",
        vec![text_part("问题")],
    )]);
    let mut cache = PaneCache::default();
    let first = paints(&cache.snapshot(40, &lx));
    let second = paints(&cache.snapshot(40, &lx));
    assert_eq!(first.len(), 1);
    assert!(
        Arc::ptr_eq(&first[0], &second[0]),
        "同一状态两帧必须复用同一份绘制数据"
    );
}

#[test]
fn only_the_changed_message_is_repainted() {
    let mut lx = lx_with(vec![
        message("u1", "user", "complete", vec![text_part("第一个问题")]),
        message("a1", "assistant", "complete", vec![text_part("第一段回答")]),
        message("u2", "user", "complete", vec![text_part("第二个问题")]),
    ]);
    let mut cache = PaneCache::default();
    let before = paints(&cache.snapshot(40, &lx));
    assert_eq!(before.len(), 3);

    if let AssistantStepView::Part { part, .. } = &mut lx.chat.messages[1].steps[0] {
        part.content.push_str("，追加一句");
    }
    let after = paints(&cache.snapshot(40, &lx));
    assert_eq!(after.len(), 3, "块数不因正文追加而变");
    assert!(Arc::ptr_eq(&before[0], &after[0]), "未变的消息复用旧块");
    assert!(
        !Arc::ptr_eq(&before[1], &after[1]),
        "追加正文的消息必须重排"
    );
    assert!(Arc::ptr_eq(&before[2], &after[2]), "未变的消息复用旧块");
}

#[test]
fn cached_bytes_stay_under_the_limit() {
    let messages: Vec<ChatMessageView> = (0..50)
        .map(|index| {
            message(
                &format!("m{index}"),
                "assistant",
                "complete",
                vec![text_part("一行正文\n两行正文\n三行正文")],
            )
        })
        .collect();
    let lx = lx_with(messages);
    let mut cache = PaneCache::default();
    let _ = cache.snapshot(40, &lx);
    assert!(cache.bytes() > 0, "正文块必须进二级缓存");
    assert!(cache.bytes() <= MAX_CACHED_BYTES);
}

#[test]
fn eviction_keeps_rows_identical_after_rebuild() {
    let messages: Vec<ChatMessageView> = (0..30)
        .map(|index| {
            let role = if index % 2 == 0 { "user" } else { "assistant" };
            message(
                &format!("m{index}"),
                role,
                "complete",
                vec![text_part("一行\n两行\n三行")],
            )
        })
        .collect();
    let mut lx = lx_with(messages);
    let mut cache = PaneCache::default();
    let first_total = cache.snapshot(40, &lx).total_rows;
    let first_blocks = cache.block_count();
    assert!(first_blocks > 0);

    // 推进帧号后，上一帧构建的块都成了「冷块」，强制淘汰全部。
    cache.advance_frame();
    cache.set_limit(1);
    assert_eq!(cache.block_count(), 0, "冷块全部淘汰");
    assert_eq!(cache.bytes(), 0, "字节计数同步归零");

    // 序号只进指纹、不进画面：用它触发重建，行数必须与淘汰前一致。
    lx.chat.messages[29].sequence += 1;
    let rebuilt = cache.snapshot(40, &lx);
    assert_eq!(rebuilt.total_rows, first_total, "淘汰重建后行数不变");
    assert_eq!(cache.block_count(), first_blocks, "块按需全部重建");
}

/// 稳定态（没有新内容、不重建快照）也必须把超限块收回去。
///
/// 回归：`evict` 只在重建快照 / 插入块时被调；冷启动把整段历史插进来时所有块都是
/// 「本帧使用」，当帧谁也不可淘汰，此后静置就再没有回收点，`bytes` 永远停在上限之上。
#[test]
fn steady_state_reclaim_enforces_the_limit() {
    let messages: Vec<ChatMessageView> = (0..40)
        .map(|index| {
            message(
                &format!("m{index}"),
                "user",
                "complete",
                vec![text_part("一行正文\n两行正文\n三行正文")],
            )
        })
        .collect();
    let lx = lx_with(messages);
    let mut cache = PaneCache::default();
    // 上限在建快照**之前**设好：`set_limit` 自带的 evict 面对空缓存是空操作，
    // 下面超限只可能来自「构建后没有回收点」，不会被 set_limit 掩盖。
    let limit = 200;
    cache.set_limit(limit);
    let _ = cache.snapshot(40, &lx);
    let bytes = cache.bytes();
    assert!(
        bytes > limit,
        "冷启动的块本帧不可淘汰，应当超限：{bytes} <= {limit}"
    );

    // 下一帧这些块都成了冷块：稳定态必须自己回收。
    cache.advance_frame();
    cache.reclaim();
    assert!(
        cache.bytes() <= limit,
        "稳定态没有回收点，上限失效：{} > {limit}",
        cache.bytes()
    );
}

/// `render` 必须在 `mark_used` 之后回收：本帧画过的块保留，冷块被清走。
#[test]
fn render_reclaims_after_marking_used_blocks() {
    let messages: Vec<ChatMessageView> = (0..30)
        .map(|index| {
            message(
                &format!("m{index}"),
                "user",
                "complete",
                vec![text_part("一行正文\n两行正文\n三行正文")],
            )
        })
        .collect();
    let lx = lx_with(messages);
    let area = Rect::new(0, 0, 40, 10);
    let mut grid = Grid::new(area);
    let mut cache = PaneCache::default();
    let limit = 1_000;
    cache.set_limit(limit);
    let _ = render(&mut grid, area, &lx, 0, &mut cache);
    assert!(
        cache.bytes() > limit,
        "冷启动那一帧所有块都是本帧插入，不允许被回收：{}",
        cache.bytes()
    );

    // 换帧后只画视口内：冷块回收，可见块（本帧标记过）必须留下。
    cache.advance_frame();
    let _ = render(&mut grid, area, &lx, 0, &mut cache);
    assert!(
        cache.bytes() <= limit,
        "render 没有在绘制之后回收：{} > {limit}",
        cache.bytes()
    );
    assert!(cache.block_count() > 0, "本帧画过的块不许被回收");
}

#[test]
fn visible_blocks_survive_eviction() {
    let messages: Vec<ChatMessageView> = (0..20)
        .map(|index| {
            message(
                &format!("m{index}"),
                "user",
                "complete",
                vec![text_part(&format!("第 {index} 条消息"))],
            )
        })
        .collect();
    let lx = lx_with(messages);
    let area = Rect::new(0, 0, 40, 10);
    let mut cache = PaneCache::default();
    let mut grid = Grid::new(area);
    let total = render(&mut grid, area, &lx, 0, &mut cache);
    let all_blocks = cache.block_count();
    assert!(all_blocks > 1, "历史必须有多个块");

    // 换一帧后只画最后 10 行：只有视口内的块被标记为本帧使用。
    cache.advance_frame();
    let _ = render(&mut grid, area, &lx, total.saturating_sub(10), &mut cache);
    cache.set_limit(1);

    let kept = cache.block_count();
    assert!(kept > 0, "本帧画过的块不许淘汰");
    assert!(kept < all_blocks, "视口外的冷块必须被淘汰");
}

/// 字节预算是**所有窗格共享**的 32MB，不是每窗格 32MB。
#[test]
fn budget_is_shared_across_panes() {
    let mut render_cache = RenderCache::default();
    let a = PaneId::alloc();
    render_cache.begin_frame();
    let single = render_cache.pane(a).limit;
    assert_eq!(single, MAX_CACHED_BYTES, "单窗格独占整个预算");

    let b = PaneId::alloc();
    render_cache.begin_frame();
    let _ = render_cache.pane(b);
    assert_eq!(
        render_cache.pane(a).limit + render_cache.pane(b).limit,
        MAX_CACHED_BYTES,
        "两个窗格必须平摊同一份预算"
    );
    assert_eq!(render_cache.pane(a).limit, MAX_CACHED_BYTES / 2);

    render_cache.retain_panes(&[a]);
    assert_eq!(
        render_cache.pane(a).limit,
        MAX_CACHED_BYTES,
        "窗格关闭后份额收回"
    );
}

/// 工具卡片的点击面几何随块缓存：内容不变时**不重算**，内容变了才重建。
#[test]
fn tool_card_geometry_is_cached_with_the_block() {
    let command = crate::app::state::ToolCallView {
        call_id: "call-1".into(),
        name: "exec_command".into(),
        position: 0,
        arguments_json: String::new(),
        status: ToolStatus::Completed,
        summary: Some(Box::new(ToolSummary::default())),
        error: None,
        result_json: None,
        detail: Some(Box::new(crate::app::state::ToolDetailView::Command(
            crate::app::state::CommandView {
                process_session_id: None,
                command: "cargo test --lib history".into(),
                cwd: "/repo".into(),
                tty: false,
                state: crate::app::state::ProcessStateView::Exited { code: Some(0) },
                output_preview: Vec::new(),
                truncated: false,
                dropped_bytes: 0,
                output_tail: (0..12)
                    .map(|line| format!("test result line {line}: ok"))
                    .collect::<Vec<_>>()
                    .join("\n"),
                started_at_ms: None,
                ended_at_ms: None,
            },
        ))),
    };
    let mut card = message(
        "assistant-1",
        "assistant",
        "complete",
        vec![text_part("跑一下")],
    );
    card.steps.push(AssistantStepView::Tool {
        position: 1,
        call: command,
    });
    let lx = lx_with(vec![card]);
    let mut cache = PaneCache::default();
    let first = cache.snapshot(40, &lx);
    let spans: Vec<_> = first
        .entries
        .iter()
        .flat_map(|entry| entry.blocks.iter())
        .flat_map(|block| block.paint.card_spans().to_vec())
        .collect();
    assert!(!spans.is_empty(), "命令卡片必须登记点击面几何");
    let builds = cache.builds();

    // 同一份状态再取一次：块命中，点击面几何也不重算。
    let second = cache.snapshot(40, &lx);
    assert_eq!(cache.builds(), builds, "状态未变，不允许重建");
    assert_eq!(first.cards, second.cards, "点击面必须完全一致");
}
