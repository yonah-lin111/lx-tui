//! 历史视图状态测试：滚动钳制、滚动条偏移、回到底部、loading 帧、
//! 置顶行抑制记忆与跳转。

use super::*;

/// 构造带 `count` 条用户消息的 lx 窗格（每行一条），用于滚动语义测试。
fn lx_with_messages(state: &mut AppState, count: usize) {
    let pane = state.active_tab().layout.focus();
    let target = state.pane_mut_anywhere(pane).expect("pane");
    for index in 0..count {
        target.lx.chat.messages.push(ChatMessageView {
            id: format!("m{index}"),
            role: "user".into(),
            status: "complete".into(),
            turn_index: index as i64,
            steps: vec![AssistantStepView::Part {
                position: 0,
                part: ChatPartView {
                    kind: PartKind::Text,
                    content: format!("消息 {index}"),
                    provider_metadata_json: None,
                    started_at_ms: None,
                    ended_at_ms: None,
                    completed_at_ms: None,
                },
            }],
            model_label: None,
            error_code: None,
            input_tokens: None,
            output_tokens: None,
            cache_read_tokens: None,
            duration_ms: None,
        });
    }
}

#[test]
fn history_scroll_clamps_and_tracks_follow_bottom() {
    let mut state = AppState::demo();
    lx_with_messages(&mut state, 3);
    let pane = state.active_tab().layout.focus();
    // 总行数 3 条消息 + 3 个消息间空行 = 6；视口 2 行 → 最大顶部行 4。
    assert!(scroll_lx_history(&mut state, pane, 999, 6, 2), "向下滚动");
    let chat = &state.pane_anywhere(pane).expect("pane").lx.chat;
    assert_eq!(chat.history_scroll, 4, "钳到最大顶部行");
    assert!(chat.follow_bottom, "滚到底恢复跟随");
    // 向上滚动离开底部。
    assert!(scroll_lx_history(&mut state, pane, -1, 6, 2));
    let chat = &state.pane_anywhere(pane).expect("pane").lx.chat;
    assert_eq!(chat.history_scroll, 3);
    assert!(!chat.follow_bottom, "离开底部不再跟随");
    // 再向上到顶。
    assert!(scroll_lx_history(&mut state, pane, -999, 6, 2));
    assert_eq!(
        state
            .pane_anywhere(pane)
            .expect("pane")
            .lx
            .chat
            .history_scroll,
        0
    );
}

#[test]
fn history_scrollbar_set_offset_marks_follow_at_bottom() {
    let mut state = AppState::demo();
    lx_with_messages(&mut state, 5);
    let pane = state.active_tab().layout.focus();
    // 顶部行 2，视口 3 → 不跟随。
    assert!(set_lx_history_scroll(&mut state, pane, 2, 10, 3));
    {
        let chat = &state.pane_anywhere(pane).expect("pane").lx.chat;
        assert_eq!(chat.history_scroll, 2);
        assert!(!chat.follow_bottom);
    }
    // 顶部行 = total - visible → 钉底跟随。
    assert!(set_lx_history_scroll(&mut state, pane, 7, 10, 3));
    {
        let chat = &state.pane_anywhere(pane).expect("pane").lx.chat;
        assert_eq!(chat.history_scroll, 7);
        assert!(chat.follow_bottom);
    }
}

#[test]
fn jump_lx_history_bottom_rearms_follow() {
    let mut state = AppState::demo();
    lx_with_messages(&mut state, 6);
    let pane = state.active_tab().layout.focus();
    set_lx_history_scroll(&mut state, pane, 0, 12, 3);
    {
        let chat = &state.pane_anywhere(pane).expect("pane").lx.chat;
        assert!(!chat.follow_bottom, "上滚后不跟随");
    }
    assert!(jump_lx_history_bottom(&mut state, pane));
    let chat = &state.pane_anywhere(pane).expect("pane").lx.chat;
    assert!(chat.follow_bottom, "回到底部恢复跟随");
    assert!(!jump_lx_history_bottom(&mut state, pane), "已在底部无变化");
}

#[test]
fn hover_lx_jump_bottom_tracks_state() {
    let mut state = AppState::demo();
    lx_with_messages(&mut state, 1);
    let pane = state.active_tab().layout.focus();
    assert!(hover_lx_jump_bottom(&mut state, pane, true));
    assert!(
        state
            .pane_anywhere(pane)
            .expect("pane")
            .lx
            .chat
            .jump_bottom_hover,
        "悬停置位"
    );
    assert!(
        !hover_lx_jump_bottom(&mut state, pane, true),
        "重复置位无变化"
    );
    assert!(hover_lx_jump_bottom(&mut state, pane, false));
    assert!(
        !state
            .pane_anywhere(pane)
            .expect("pane")
            .lx
            .chat
            .jump_bottom_hover,
        "离开清空"
    );
}

#[test]
fn hover_lx_chat_tracks_card_state_per_pane() {
    let mut state = AppState::demo();
    lx_with_messages(&mut state, 1);
    let pane = state.active_tab().layout.focus();
    let target = ChatHover::Tool {
        message_id: "m0".into(),
        call_id: "c0".into(),
    };
    assert!(hover_lx_chat(&mut state, pane, Some(target.clone())));
    let chat = &state.pane_anywhere(pane).expect("pane").lx.chat;
    assert!(chat.tool_is_hovered("m0", "c0"));
    assert!(!chat.tool_is_hovered("m0", "c1"));
    assert!(
        !hover_lx_chat(&mut state, pane, Some(target)),
        "重复置位无变化"
    );
    assert!(hover_lx_chat(&mut state, pane, None));
    assert!(
        !state
            .pane_anywhere(pane)
            .expect("pane")
            .lx
            .chat
            .tool_is_hovered("m0", "c0"),
        "离开清空"
    );
}

#[test]
fn hover_lx_chat_distinguishes_thinking_from_tool_at_same_index() {
    let mut state = AppState::demo();
    lx_with_messages(&mut state, 1);
    let pane = state.active_tab().layout.focus();
    let thinking = ChatHover::Thinking {
        message_index: 0,
        step_index: 1,
    };
    assert!(hover_lx_chat(&mut state, pane, Some(thinking)));
    let chat = &state.pane_anywhere(pane).expect("pane").lx.chat;
    assert!(chat.thinking_is_hovered(0, 1), "思考块悬停");
    assert!(!chat.thinking_is_hovered(0, 0), "别的思考块不亮");
    assert!(!chat.tool_is_hovered("m0", "c0"), "思考悬停不点亮工具卡");
}

#[test]
fn collapse_lx_card_toggles_the_pressed_block() {
    // 「点正文收起」复用工具栏展开入口：一次点击只切一次状态。
    let mut state = AppState::demo();
    let pane = state.active_tab().layout.focus();
    let target = state.pane_mut_anywhere(pane).expect("pane");
    target.view = crate::app::state::PaneView::Lx;
    target.lx.chat.messages = vec![ChatMessageView {
        id: "m0".into(),
        role: "assistant".into(),
        status: "complete".into(),
        turn_index: 0,
        steps: vec![AssistantStepView::Part {
            position: 0,
            part: ChatPartView {
                kind: PartKind::Reasoning,
                content: "想一想".into(),
                provider_metadata_json: None,
                started_at_ms: None,
                ended_at_ms: None,
                completed_at_ms: None,
            },
        }],
        model_label: None,
        error_code: None,
        input_tokens: None,
        output_tokens: None,
        cache_read_tokens: None,
        duration_ms: None,
    }];
    assert!(
        !state
            .pane_anywhere(pane)
            .expect("pane")
            .lx
            .chat
            .reasoning_expanded(0, 0),
        "默认折叠"
    );
    let card = ChatHover::Thinking {
        message_index: 0,
        step_index: 0,
    };
    assert!(collapse_lx_card(&mut state, pane, card.clone()), "切换成功");
    assert!(
        state
            .pane_anywhere(pane)
            .expect("pane")
            .lx
            .chat
            .reasoning_expanded(0, 0),
        "第一次点击展开"
    );
    assert!(collapse_lx_card(&mut state, pane, card), "再次切换");
    assert!(
        !state
            .pane_anywhere(pane)
            .expect("pane")
            .lx
            .chat
            .reasoning_expanded(0, 0),
        "第二次点击收起"
    );
}

#[test]
fn advance_lx_loading_only_when_streaming() {
    let mut state = AppState::demo();
    lx_with_messages(&mut state, 1);
    let pane = state.active_tab().layout.focus();
    // 空闲：不推进。
    assert!(!advance_lx_loading(&mut state));
    // 流中：推进帧号。
    state
        .pane_mut_anywhere(pane)
        .expect("pane")
        .lx
        .chat
        .status
        .streaming = true;
    assert!(state.has_streaming_lx());
    assert!(advance_lx_loading(&mut state));
    assert_eq!(
        state
            .pane_anywhere(pane)
            .expect("pane")
            .lx
            .chat
            .loading_frame,
        1
    );
}

/// 造一条展示用消息（置顶行测试）。
fn lx_view_message(role: &str, text: &str, index: usize) -> ChatMessageView {
    ChatMessageView {
        id: format!("{role}-{index}"),
        role: role.into(),
        status: "complete".into(),
        turn_index: 0,
        steps: vec![AssistantStepView::Part {
            position: 0,
            part: ChatPartView {
                kind: PartKind::Text,
                content: text.into(),
                provider_metadata_json: None,
                started_at_ms: None,
                ended_at_ms: None,
                completed_at_ms: None,
            },
        }],
        model_label: None,
        error_code: None,
        input_tokens: None,
        output_tokens: None,
        cache_read_tokens: None,
        duration_ms: None,
    }
}

/// 焦点窗格置为 lx，并塞入一段会溢出的历史（两轮问答，正文各若干行）。
fn state_with_lx_history() -> (AppState, PaneId, ratatui::layout::Rect) {
    let mut state = ready_state("");
    let pane = focus_pane(&state);
    let rect = ratatui::layout::Rect::new(0, 0, 24, 10);
    let target = state.pane_mut_anywhere(pane).expect("pane");
    target.view = crate::app::state::PaneView::Lx;
    target.lx.chat.messages = vec![
        lx_view_message("user", "第一轮的问题", 0),
        lx_view_message("assistant", "A1\nA2\nA3\nA4", 1),
        lx_view_message("user", "第二轮的问题", 2),
        lx_view_message("assistant", "B1\nB2\nB3\nB4", 3),
    ];
    (state, pane, rect)
}

#[test]
fn resolve_writes_sticky_suppression_memory_from_pure_view() {
    let (mut state, pane, rect) = state_with_lx_history();
    let rects = [(pane, rect)];
    let inner = crate::layout::pane_inner_rect(rect);

    // 场景 1：顶行落在第一轮回复里 → 让位后仍需要 → 不写记忆。
    let target = state.pane_anywhere(pane).expect("pane");
    let view = crate::ui::main_content::lx::history_view(inner, &target.lx).expect("view");
    let answer_start = view.messages[1].start;
    let boundary = view.messages[2].start.saturating_sub(1);
    assert!(view.total_rows > boundary + 1, "内容确实溢出");
    let target = state.pane_mut_anywhere(pane).expect("pane");
    target.lx.chat.follow_bottom = false;
    target.lx.chat.history_scroll = answer_start;
    assert!(
        !resolve_lx_sticky_prompt(&mut state, &rects),
        "无需抑制：无变化"
    );
    assert_eq!(
        state
            .pane_anywhere(pane)
            .expect("pane")
            .lx
            .chat
            .suppressed_prompt_header,
        None
    );

    // 场景 2：抑制只在"让位会改变顶行"时发生——跟随底部（顶行 = total - height）
    // 才会；阅读态的顶行是绝对行号，不随让位变化。几何随面板尺寸变化，故搜索视口高度。
    let mut scenario = None;
    for height in 4..24u16 {
        let probe_rect = ratatui::layout::Rect::new(0, 0, 24, height);
        let probe_inner = crate::layout::pane_inner_rect(probe_rect);
        {
            let target = state.pane_mut_anywhere(pane).expect("pane");
            target.lx.chat.follow_bottom = true;
        }
        let target = state.pane_anywhere(pane).expect("pane");
        let probe =
            crate::ui::main_content::lx::history_view(probe_inner, &target.lx).expect("view");
        if probe.suppressed.is_some() {
            scenario = Some((probe_rect, probe.suppressed));
            break;
        }
    }
    let (probe_rect, expected) = scenario.expect("存在跟随态下应抑制的视口尺寸");
    let rects = [(pane, probe_rect)];
    assert!(resolve_lx_sticky_prompt(&mut state, &rects), "写记忆即变化");
    assert_eq!(
        state
            .pane_anywhere(pane)
            .expect("pane")
            .lx
            .chat
            .suppressed_prompt_header,
        expected,
        "记忆与纯函数结果一致"
    );
    assert!(
        !resolve_lx_sticky_prompt(&mut state, &rects),
        "重复 resolve 无变化"
    );
}

#[test]
fn jump_to_prompt_clears_follow_and_moves_viewport() {
    let (mut state, pane, _rect) = state_with_lx_history();
    assert!(jump_lx_history_to_prompt(&mut state, pane, 5));
    let chat = &state.pane_anywhere(pane).expect("pane").lx.chat;
    assert_eq!(chat.history_scroll, 5, "跳到目标行");
    assert!(!chat.follow_bottom, "进入阅读态（置顶行随让位规则收敛）");
    assert!(
        !jump_lx_history_to_prompt(&mut state, pane, 5),
        "重复跳转无变化"
    );
}
