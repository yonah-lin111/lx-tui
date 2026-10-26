//! 历史视图状态测试：滚动钳制、滚动条偏移、回到底部、loading 帧、
//! 置顶行抑制记忆与跳转。

use super::*;

/// 构造带 `count` 条用户消息的 lx 窗格（每行一条），用于滚动语义测试。
fn lx_with_messages(state: &mut AppState, count: usize) {
    let pane = state.active_tab().layout.focus();
    let target = state.pane_mut_anywhere(pane).expect("pane");
    for index in 0..count {
        target.lx.chat.messages.push(ChatMessageView {
            sequence: 0,
            id: format!("m{index}"),
            role: "user".into(),
            status: "complete".into(),
            turn_index: index as i64,
            kind: "chat".into(),
            shell: None,
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
            reports: Vec::new(),
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
fn toggle_lx_card_flips_the_pressed_block() {
    // 卡片内容就是点击面：点折叠内容展开、点展开内容收起，一次点击只切一次状态。
    let mut state = AppState::demo();
    let pane = state.active_tab().layout.focus();
    let target = state.pane_mut_anywhere(pane).expect("pane");
    target.view = crate::app::state::PaneView::Lx;
    target.lx.chat.messages = vec![ChatMessageView {
        sequence: 0,
        id: "m0".into(),
        role: "assistant".into(),
        status: "complete".into(),
        turn_index: 0,
        kind: "chat".into(),
        shell: None,
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
        reports: Vec::new(),
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
    let card = CardTarget::Thinking {
        message_index: 0,
        step_index: 0,
    };
    assert!(toggle_lx_card(&mut state, pane, card.clone()), "切换成功");
    assert!(
        state
            .pane_anywhere(pane)
            .expect("pane")
            .lx
            .chat
            .reasoning_expanded(0, 0),
        "第一次点击展开"
    );
    assert!(toggle_lx_card(&mut state, pane, card), "再次切换");
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

/// 压缩中也要有心跳：推进扫描帧并刷新本次压缩耗时（否则输入框上方那一行是静止的）。
#[test]
fn advance_lx_loading_drives_the_compact_timer() {
    let mut state = AppState::demo();
    lx_with_messages(&mut state, 1);
    let pane = state.active_tab().layout.focus();
    // 压缩开始于 5 秒前。
    let started = crate::db::now_ms() - 5_000;
    {
        let target = state.pane_mut_anywhere(pane).expect("pane");
        target.lx.compact.start("op".into(), started);
    }
    assert!(state.has_streaming_lx(), "压缩中必须有心跳");
    assert!(advance_lx_loading(&mut state), "压缩中推进帧");
    let target = state.pane_anywhere(pane).expect("pane");
    assert_eq!(target.lx.chat.loading_frame, 1);
    let elapsed = target.lx.compact.elapsed_ms.expect("刷新压缩耗时");
    assert!(
        (5_000..6_000).contains(&elapsed),
        "耗时按开始时刻计算：{elapsed}"
    );
    // 帧推进本身不推进聊天轮次帧（压缩没有聊天轮次）。
    assert_eq!(target.lx.chat.turn_elapsed_ms, None);

    // 压缩结束：心跳停止。
    state
        .pane_mut_anywhere(pane)
        .expect("pane")
        .lx
        .compact
        .finish();
    assert!(!state.has_streaming_lx());
    assert!(!advance_lx_loading(&mut state));
}

/// 造一条展示用消息（置顶行测试）。
fn lx_view_message(role: &str, text: &str, index: usize) -> ChatMessageView {
    ChatMessageView {
        sequence: 0,
        id: format!("{role}-{index}"),
        role: role.into(),
        status: "complete".into(),
        turn_index: 0,
        kind: "chat".into(),
        shell: None,
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
        reports: Vec::new(),
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
    let view = crate::ui::main_content::lx::history_view(
        inner,
        &target.lx,
        crate::ui::RenderCache::default().pane(pane),
    )
    .expect("view");
    let answer_start = view.messages[1].start;
    let boundary = view.messages[2].start.saturating_sub(1);
    assert!(view.total_rows > boundary + 1, "内容确实溢出");
    let target = state.pane_mut_anywhere(pane).expect("pane");
    target.lx.chat.follow_bottom = false;
    target.lx.chat.history_scroll = answer_start;
    // 合并后的 resolve 同时写视口顶部行与抑制记忆：首次调用会因 `viewport_top`
    // 由 None 变为具体行而返回 true；这里只关心抑制记忆本身。
    let mut cache = crate::ui::RenderCache::default();
    resolve_lx_history(&mut state, &rects, &mut cache);
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
        let probe = crate::ui::main_content::lx::history_view(
            probe_inner,
            &target.lx,
            crate::ui::RenderCache::default().pane(pane),
        )
        .expect("view");
        if probe.suppressed.is_some() {
            scenario = Some((probe_rect, probe.suppressed));
            break;
        }
    }
    let (probe_rect, expected) = scenario.expect("存在跟随态下应抑制的视口尺寸");
    let rects = [(pane, probe_rect)];
    let mut cache = crate::ui::RenderCache::default();
    assert!(
        resolve_lx_history(&mut state, &rects, &mut cache),
        "写记忆即变化"
    );
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
        !resolve_lx_history(&mut state, &rects, &mut cache),
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
