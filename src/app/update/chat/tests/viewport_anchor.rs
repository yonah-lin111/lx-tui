//! 展开交互的视口锚定：跟随底部时切换 thinking / 工具卡片，视口不得跳回顶部。
//!
//! 回归的 bug：`follow_bottom == true` 时 `history_scroll` 是过期值（通常 0），
//! 旧实现直接把它当顶部行用，于是"点击展开"后界面跳到历史最开头——屏幕上再也
//! 看不到刚点的那块，用户读作"点了展不开"。

use ratatui::layout::Rect;

use super::*;
use crate::app::state::{
    AppState, ChatPartView, CommandView, PaneView, ProcessStateView, ToolCallView, ToolDetailView,
};

fn reasoning_part(content: &str) -> ChatPartView {
    ChatPartView {
        kind: crate::chat::events::PartKind::Reasoning,
        content: content.into(),
        provider_metadata_json: None,
        started_at_ms: None,
        ended_at_ms: None,
        completed_at_ms: None,
    }
}

fn text_part(content: &str) -> ChatPartView {
    ChatPartView {
        kind: crate::chat::events::PartKind::Text,
        content: content.into(),
        provider_metadata_json: None,
        started_at_ms: None,
        ended_at_ms: None,
        completed_at_ms: None,
    }
}

fn message(role: &str, parts: Vec<ChatPartView>, turn: i64) -> ChatMessageView {
    ChatMessageView {
        sequence: 0,
        id: format!("{role}-{turn}"),
        role: role.into(),
        status: "complete".into(),
        turn_index: turn,
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

/// 一条足够长的历史：每轮"问题 + 思考 + 回答"，滚到底时视口里必有多块思考。
fn long_history() -> Vec<ChatMessageView> {
    let mut messages = Vec::new();
    for turn in 0..6 {
        messages.push(message("user", vec![text_part("问题")], turn * 2));
        messages.push(message(
            "assistant",
            vec![
                reasoning_part("思考内容\n第二行\n第三行"),
                text_part("回答正文"),
            ],
            turn * 2 + 1,
        ));
    }
    messages
}

/// 把一个 lx 窗格装成"跟随底部 + 已绘制过一帧"的状态。
fn state_with_history(messages: Vec<ChatMessageView>) -> (AppState, PaneId) {
    let mut state = AppState::demo();
    let pane = state.active_tab().layout.focus();
    let target = state.pane_mut_anywhere(pane).expect("pane");
    target.view = PaneView::Lx;
    target.lx.chat.messages = messages;
    target.lx.chat.follow_bottom = true;
    (state, pane)
}

/// 模拟事件循环绘制前的那一步：把当前视口顶部行写进记忆。
///
/// 事件循环给的是**窗格外框**（内部再取 `pane_inner_rect`）；测试里 `area` 已是
/// 历史区本身，故按同一份几何直接读一次 `history_view` 写入，避免两套矩形口径。
fn remember_viewport(state: &mut AppState, pane: PaneId, area: Rect) {
    let scroll = crate::ui::main_content::lx::history_view(
        area,
        &state.pane_anywhere(pane).expect("pane").lx,
        crate::ui::RenderCache::default().pane(pane),
    )
    .expect("view")
    .scroll;
    let target = state.pane_mut_anywhere(pane).expect("pane");
    target.lx.chat.viewport_top = Some(scroll);
}

#[test]
fn pin_viewport_keeps_the_row_the_user_is_looking_at() {
    let (mut state, pane) = state_with_history(long_history());
    let area = Rect::new(0, 0, 40, 8);
    // 绘制前记忆：跟随时顶部行 = total - height，不是 0。
    remember_viewport(&mut state, pane, area);
    let followed = state
        .pane_anywhere(pane)
        .expect("pane")
        .lx
        .chat
        .viewport_top
        .expect("跟随时也记录了实际绘制的顶部行");
    assert!(followed > 0, "长历史滚到底时顶部行不为 0：{followed}");

    assert!(pin_lx_viewport(&mut state, pane), "跟随态需要锚定");
    let chat = &state.pane_anywhere(pane).expect("pane").lx.chat;
    assert_eq!(chat.history_scroll, followed, "锚定到用户正看着的那一行");
    assert!(!chat.follow_bottom, "锚定即进入阅读态");
    // 已在阅读态：不重复锚定（否则会把用户后续滚动的位置覆盖掉）。
    assert!(!pin_lx_viewport(&mut state, pane));
}

#[test]
fn toggling_thinking_while_following_keeps_the_viewport() {
    let (mut state, pane) = state_with_history(long_history());
    let area = Rect::new(0, 0, 40, 8);
    remember_viewport(&mut state, pane, area);
    let before = crate::ui::main_content::lx::history_view(
        area,
        &state.pane_anywhere(pane).expect("pane").lx,
        crate::ui::RenderCache::default().pane(pane),
    )
    .expect("view");

    // 点最后一块思考的头部（它就在视口里）。
    let target = state.pane_anywhere(pane).expect("pane");
    let thinking = crate::ui::main_content::lx::history::layout(
        before.body,
        &target.lx,
        crate::ui::RenderCache::default().pane(pane),
    )
    .thinking;
    let (message_index, step_index, _) = *thinking.last().expect("thinking 头部");
    assert!(toggle_thinking_at(
        &mut state,
        pane,
        message_index,
        step_index
    ));

    let after = crate::ui::main_content::lx::history_view(
        area,
        &state.pane_anywhere(pane).expect("pane").lx,
        crate::ui::RenderCache::default().pane(pane),
    )
    .expect("view");
    assert_eq!(after.scroll, before.scroll, "展开后顶部行不变");
    assert!(
        state
            .pane_anywhere(pane)
            .expect("pane")
            .lx
            .chat
            .reasoning_expanded(message_index, step_index),
        "思考块确实展开了"
    );
}

#[test]
fn toggling_tool_detail_while_following_keeps_the_viewport() {
    let mut state = AppState::demo();
    let pane = state.active_tab().layout.focus();
    let area = Rect::new(0, 0, 40, 8);
    {
        let target = state.pane_mut_anywhere(pane).expect("pane");
        target.view = PaneView::Lx;
        let mut call = ToolCallView::new("call-1", "exec_command", 0);
        call.status = ToolStatus::Running;
        call.detail = Some(Box::new(ToolDetailView::Command(CommandView {
            process_session_id: Some(1),
            command: "cargo test".into(),
            cwd: "/workspace".into(),
            tty: false,
            state: ProcessStateView::Running,
            output_preview: Vec::new(),
            truncated: false,
            dropped_bytes: 0,
            output_tail: "line1\nline2\n".into(),
            started_at_ms: Some(0),
            ended_at_ms: None,
        })));
        let mut card = message("assistant", Vec::new(), 0);
        card.steps
            .push(AssistantStepView::Tool { position: 0, call });
        target.lx.chat.messages = vec![
            message("user", vec![text_part("问题")], 0),
            card,
            message("user", vec![text_part("再问")], 2),
            message("assistant", vec![text_part("回答")], 3),
        ];
        target.lx.chat.follow_bottom = true;
    }
    remember_viewport(&mut state, pane, area);
    let before = crate::ui::main_content::lx::history_view(
        area,
        &state.pane_anywhere(pane).expect("pane").lx,
        crate::ui::RenderCache::default().pane(pane),
    )
    .expect("view");
    assert!(toggle_tool_detail(
        &mut state,
        pane,
        "assistant-0",
        "call-1",
        crate::app::state::CardSection::Body
    ));
    let after = crate::ui::main_content::lx::history_view(
        area,
        &state.pane_anywhere(pane).expect("pane").lx,
        crate::ui::RenderCache::default().pane(pane),
    )
    .expect("view");
    assert_eq!(after.scroll, before.scroll, "展开卡片后顶部行不变");
    assert!(
        state
            .pane_anywhere(pane)
            .expect("pane")
            .lx
            .chat
            .detail_is_expanded("assistant-0", "call-1"),
        "卡片确实展开了"
    );
}

#[test]
fn unknown_targets_do_not_anchor_the_viewport() {
    let (mut state, pane) = state_with_history(long_history());
    let area = Rect::new(0, 0, 40, 8);
    remember_viewport(&mut state, pane, area);
    // 不存在的消息/步骤：不切换，也不该顺手把跟随态改成阅读态。
    assert!(!toggle_thinking_at(&mut state, pane, 99, 0));
    assert!(!toggle_thinking_at(&mut state, pane, 0, 99));
    // 存在但不是 thinking 的步骤（用户消息的正文）。
    assert!(!toggle_thinking_at(&mut state, pane, 0, 0));
    assert!(!toggle_tool_detail(
        &mut state,
        pane,
        "missing",
        "missing",
        crate::app::state::CardSection::Body
    ));
    let chat = &state.pane_anywhere(pane).expect("pane").lx.chat;
    assert!(chat.follow_bottom, "失败的切换不应改动视口状态");
    assert_eq!(chat.history_scroll, 0);
}

/// 文件差异卡片：展开后卡片行数真的变了，视口顶部行仍必须停在同一行。
///
/// 这里刻意让展开后的正文远多于 3 行预览——如果锚定用的是旧的"两行卡片"高度，
/// 展开一次就会把视口顶行推走。
#[test]
fn toggling_file_diff_card_while_following_keeps_the_viewport() {
    use crate::app::state::{
        DiffHunkView, DiffLineView, FileChangeView, FileDiffDetailView, FileDiffView,
    };
    use crate::chat::tools::{DiffLineKind, FileChangeKind};

    let lines: Vec<DiffLineView> = (0..12)
        .flat_map(|index| {
            let old = u64::try_from(index * 2 + 1).unwrap_or(u64::MAX);
            [
                DiffLineView {
                    kind: DiffLineKind::Removed,
                    old_line: Some(usize::try_from(old).unwrap_or(usize::MAX)),
                    new_line: None,
                    text: format!("old line {index}"),
                    no_final_newline: false,
                },
                DiffLineView {
                    kind: DiffLineKind::Added,
                    old_line: None,
                    new_line: Some(usize::try_from(old).unwrap_or(usize::MAX)),
                    text: format!("new line {index}"),
                    no_final_newline: false,
                },
            ]
        })
        .collect();
    let detail = FileDiffDetailView {
        files: vec![FileChangeView {
            path: "/w/wide.rs".into(),
            to: None,
            kind: FileChangeKind::Updated,
            added: Some(12),
            removed: Some(12),
            counts_known: true,
            simplified: false,
            diff: Some(FileDiffView::Complete {
                hunks: vec![DiffHunkView { lines }],
                simplified: false,
            }),
        }],
        files_total: 1,
        added: Some(12),
        removed: Some(12),
        simplified: false,
        omitted_files: 0,
        single: false,
    };
    let mut state = AppState::demo();
    let pane = state.active_tab().layout.focus();
    let area = Rect::new(0, 0, 60, 10);
    {
        let target = state.pane_mut_anywhere(pane).expect("pane");
        target.view = PaneView::Lx;
        let mut call = ToolCallView::new("call-1", "edit", 0);
        call.status = ToolStatus::Completed;
        call.detail = Some(Box::new(ToolDetailView::File(detail)));
        let mut card = message("assistant", Vec::new(), 1);
        card.steps
            .push(AssistantStepView::Tool { position: 0, call });
        target.lx.chat.messages = vec![
            message("user", vec![text_part("问题")], 0),
            card,
            message("user", vec![text_part("再问")], 2),
            message("assistant", vec![text_part("回答")], 3),
        ];
        target.lx.chat.follow_bottom = true;
    }
    remember_viewport(&mut state, pane, area);
    let (before_scroll, before_total) = {
        let view = crate::ui::main_content::lx::history_view(
            area,
            &state.pane_anywhere(pane).expect("pane").lx,
            crate::ui::RenderCache::default().pane(pane),
        )
        .expect("view");
        (view.scroll, view.total_rows)
    };
    assert!(toggle_tool_detail(
        &mut state,
        pane,
        "assistant-1",
        "call-1",
        crate::app::state::CardSection::Body
    ));
    let view = crate::ui::main_content::lx::history_view(
        area,
        &state.pane_anywhere(pane).expect("pane").lx,
        crate::ui::RenderCache::default().pane(pane),
    )
    .expect("view");
    assert!(
        view.total_rows > before_total,
        "展开后总行数必须增加（{} → {}）",
        before_total,
        view.total_rows
    );
    assert_eq!(view.scroll, before_scroll, "展开文件卡片后顶部行不变");
    assert!(
        state
            .pane_anywhere(pane)
            .expect("pane")
            .lx
            .chat
            .detail_is_expanded("assistant-1", "call-1")
    );
}
