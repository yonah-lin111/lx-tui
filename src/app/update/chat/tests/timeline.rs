//! G2：统一纯事件模型 → 有序步骤时间线（app 层状态转换）。
//!
//! 覆盖：思考独立完成态、工具状态流转、`Finished` 兜底、旧 key 过滤、
//! `on_stream_finished` 的真实 position 快照与 `on_session_loaded` 的跨表合并。

use super::*;

/// 思考块 → 正文：思考在正文开始时即完成，assistant 消息仍在流中。
#[test]
fn reasoning_completes_when_text_starts_while_message_streams() {
    let (mut state, pane, key) = streaming_state("sess-g2");
    on_stream_delta(
        &mut state,
        &key,
        &[ChatDelta::Part(PartDelta {
            kind: PartKind::Reasoning,
            text: "想法".into(),
            metadata: None,
        })],
    );
    assert!(
        state
            .pane_anywhere(pane)
            .unwrap()
            .lx
            .chat
            .reasoning_streaming(1, 0),
        "思考生成中"
    );
    on_stream_delta(
        &mut state,
        &key,
        &[ChatDelta::Part(PartDelta {
            kind: PartKind::Text,
            text: "正文".into(),
            metadata: None,
        })],
    );
    let target = state.pane_anywhere(pane).unwrap();
    let message = &target.lx.chat.messages[1];
    assert_eq!(message.status, "streaming", "消息仍在流中");
    assert!(!target.lx.chat.reasoning_streaming(1, 0), "思考已完成");
    let completed = message
        .reasoning_parts()
        .next()
        .and_then(|part| part.completed_at_ms);
    assert!(completed.is_some(), "独立完成字段已写");
}

/// 思考 → 工具输入：思考完成，工具步骤以 InputStreaming 出现在同一时间线。
#[test]
fn reasoning_completes_before_tool_input_started() {
    let (mut state, pane, key) = streaming_state("sess-g2");
    on_stream_delta(
        &mut state,
        &key,
        &[ChatDelta::Part(PartDelta {
            kind: PartKind::Reasoning,
            text: "计划".into(),
            metadata: None,
        })],
    );
    assert!(on_stream_delta(
        &mut state,
        &key,
        &[ChatDelta::ToolInputStarted {
            call_id: "call-1".into(),
            name: "read".into(),
            ordinal: 0,
        }]
    ));
    let target = state.pane_anywhere(pane).unwrap();
    let message = &target.lx.chat.messages[1];
    assert_eq!(message.steps.len(), 2, "思考 + 工具两步");
    assert_eq!(message.steps[0].position(), 0);
    assert_eq!(message.steps[1].position(), 1);
    assert!(
        message
            .reasoning_parts()
            .next()
            .unwrap()
            .completed_at_ms
            .is_some(),
        "工具输入开始前思考已关闭"
    );
    let call = message.tool("call-1").expect("工具步骤");
    assert_eq!(call.status, crate::app::state::ToolStatus::InputStreaming);
    assert!(call.arguments_json.is_empty());
}

/// 工具参数增量与完整参数：只更新内存状态，不产生任何 IO 效应。
#[test]
fn tool_input_deltas_accumulate_and_end_moves_to_pending() {
    let (mut state, pane, key) = streaming_state("sess-g2");
    let effects_before = state.effects.len();
    on_stream_delta(
        &mut state,
        &key,
        &[
            ChatDelta::ToolInputStarted {
                call_id: "call-1".into(),
                name: "read".into(),
                ordinal: 0,
            },
            ChatDelta::ToolInputDelta {
                call_id: "call-1".into(),
                chunk: r#"{"path":"#.into(),
            },
            ChatDelta::ToolInputDelta {
                call_id: "call-1".into(),
                chunk: r#""a.rs"}"#.into(),
            },
        ],
    );
    {
        let target = state.pane_anywhere(pane).unwrap();
        let call = target.lx.chat.messages[1].tool("call-1").expect("工具");
        assert_eq!(call.arguments_json, r#"{"path":"a.rs"}"#);
        assert_eq!(call.status, crate::app::state::ToolStatus::InputStreaming);
    }
    on_stream_delta(
        &mut state,
        &key,
        &[ChatDelta::ToolInputEnded {
            call_id: "call-1".into(),
            arguments_json: r#"{"path":"a.rs"}"#.into(),
        }],
    );
    let target = state.pane_anywhere(pane).unwrap();
    let call = target.lx.chat.messages[1].tool("call-1").expect("工具");
    assert_eq!(call.status, crate::app::state::ToolStatus::Pending);
    assert_eq!(
        call.summary
            .as_ref()
            .and_then(|summary| summary.path.as_deref()),
        Some("a.rs")
    );
    assert_eq!(
        state.effects.len(),
        effects_before,
        "参数接收阶段不产生任何效应（不执行 IO、不调度工具）"
    );
}

/// 参数超上限：步骤标失败，事件不 panic、不写别的步骤。
#[test]
fn tool_input_over_limit_marks_failed() {
    let (mut state, pane, key) = streaming_state("sess-g2");
    let big = "x".repeat(crate::chat::events::MAX_TOOL_ARGUMENTS_BYTES + 1);
    on_stream_delta(
        &mut state,
        &key,
        &[
            ChatDelta::ToolInputStarted {
                call_id: "call-1".into(),
                name: "write".into(),
                ordinal: 0,
            },
            ChatDelta::ToolInputDelta {
                call_id: "call-1".into(),
                chunk: big,
            },
        ],
    );
    let call = state.pane_anywhere(pane).unwrap().lx.chat.messages[1]
        .tool("call-1")
        .cloned()
        .expect("工具");
    assert_eq!(call.status, crate::app::state::ToolStatus::Failed);
    assert_eq!(
        call.error.as_deref(),
        Some(crate::chat::events::TOOL_ARGUMENTS_TOO_LARGE_CODE)
    );
}

/// `ResponseFinished`（工具阶段）不结束用户轮次：请求状态仍是 Streaming。
#[test]
fn response_finished_keeps_request_streaming() {
    let (mut state, pane, key) = streaming_state("sess-g2");
    on_stream_delta(
        &mut state,
        &key,
        &[ChatDelta::ResponseFinished {
            stop: crate::chat::events::ModelStepStop::ToolCalls,
        }],
    );
    let target = state.pane_anywhere(pane).unwrap();
    assert!(matches!(
        target.lx.chat.request,
        RequestState::Streaming { .. }
    ));
    assert_eq!(target.lx.chat.messages[1].status, "streaming");
}

/// `Finished` 兜底：思考关闭、未终态工具标中断，且消息进入终态。
#[test]
fn finished_settles_reasoning_and_open_tools() {
    let (mut state, pane, key) = streaming_state("sess-g2");
    on_stream_delta(
        &mut state,
        &key,
        &[
            ChatDelta::Part(PartDelta {
                kind: PartKind::Reasoning,
                text: "想".into(),
                metadata: None,
            }),
            ChatDelta::ToolInputStarted {
                call_id: "call-1".into(),
                name: "read".into(),
                ordinal: 0,
            },
            ChatDelta::ToolInputEnded {
                call_id: "call-1".into(),
                arguments_json: r#"{"path":"a.rs"}"#.into(),
            },
        ],
    );
    assert!(on_stream_finished(
        &mut state,
        &key,
        &StopReason::Failed {
            code: "timeout".into()
        },
        Usage::default()
    ));
    let target = state.pane_anywhere(pane).unwrap();
    let message = &target.lx.chat.messages[1];
    assert_eq!(message.status, "failed");
    assert!(
        message
            .reasoning_parts()
            .next()
            .unwrap()
            .completed_at_ms
            .is_some()
    );
    let call = message.tool("call-1").expect("工具");
    assert_eq!(call.status, crate::app::state::ToolStatus::Interrupted);
    assert_eq!(
        call.error.as_deref(),
        Some(crate::db::chat::INTERRUPTED_ERROR_CODE)
    );
    assert!(call.arguments_json.contains("a.rs"), "不伪造结果体");
}

/// 旧 generation 的工具事件不得污染新轮（状态机 key 比对拦下）。
#[test]
fn stale_generation_tool_events_do_not_pollute_new_turn() {
    let (mut state, pane, key) = streaming_state("sess-old");
    let stale = RequestKey {
        pane,
        session: "sess-old".into(),
        generation: key.generation.wrapping_add(7),
    };
    assert!(!on_stream_delta(
        &mut state,
        &stale,
        &[
            ChatDelta::ToolInputStarted {
                call_id: "stale-call".into(),
                name: "write".into(),
                ordinal: 0,
            },
            ChatDelta::ToolInputEnded {
                call_id: "stale-call".into(),
                arguments_json: r#"{"path":"x"}"#.into(),
            },
        ],
    ));
    let message = &state.pane_anywhere(pane).unwrap().lx.chat.messages[1];
    assert!(message.tools().next().is_none(), "旧 key 不建工具步骤");
    assert!(message.steps.is_empty());
}

/// 轮次结束后（非 Streaming 状态）的迟到事件同样被拒。
#[test]
fn events_after_finish_are_ignored() {
    let (mut state, pane, key) = streaming_state("sess-g2");
    on_stream_finished(&mut state, &key, &StopReason::Complete, Usage::default());
    assert!(!on_stream_delta(
        &mut state,
        &key,
        &[ChatDelta::ToolInputStarted {
            call_id: "late".into(),
            name: "read".into(),
            ordinal: 0,
        }]
    ));
    let message = &state.pane_anywhere(pane).unwrap().lx.chat.messages[1];
    assert!(message.tools().next().is_none());
}

/// `on_stream_finished` 的快照必须带真实 position（不按 enumerate 压平）。
#[test]
fn finish_snapshot_keeps_real_positions() {
    let (mut state, pane, key) = streaming_state("sess-g2");
    on_stream_delta(
        &mut state,
        &key,
        &[
            ChatDelta::Part(PartDelta {
                kind: PartKind::Text,
                text: "前".into(),
                metadata: None,
            }),
            ChatDelta::ToolInputStarted {
                call_id: "call-1".into(),
                name: "read".into(),
                ordinal: 0,
            },
            ChatDelta::ToolInputEnded {
                call_id: "call-1".into(),
                arguments_json: r#"{"path":"a.rs"}"#.into(),
            },
            ChatDelta::Part(PartDelta {
                kind: PartKind::Text,
                text: "后".into(),
                metadata: None,
            }),
        ],
    );
    let target = state.pane_anywhere(pane).unwrap();
    let positions: Vec<i64> = target.lx.chat.messages[1]
        .steps
        .iter()
        .map(|step| step.position())
        .collect();
    assert_eq!(positions, vec![0, 1, 2], "步骤位置连续唯一");
    on_stream_finished(&mut state, &key, &StopReason::Complete, Usage::default());
    let parts = state
        .effects
        .iter()
        .rev()
        .find_map(|effect| match effect {
            crate::app::effects::Effect::FinishChatTurn { parts, .. } => Some(parts.clone()),
            _ => None,
        })
        .expect("finish effect");
    // 工具步骤不是内容块：只有两个 part，且 position 保留原值（0 与 2，不压平成 0/1）。
    assert_eq!(
        parts.iter().map(|part| part.position).collect::<Vec<_>>(),
        vec![0, 2]
    );
}

/// `on_session_loaded` 按 position 合并 parts 与 tools，且 interrupted 可见。
#[test]
fn session_loaded_merges_parts_and_tools_by_position() {
    let mut state = ready_state("x");
    let pane = focus_pane(&state);
    let messages = vec![
        crate::db::chat::ChatMessage {
            external_id: "u1".into(),
            sequence: 0,
            turn_index: 0,
            role: "user".into(),
            status: "complete".into(),
            model_provider: None,
            model_id: None,
            variant: None,
            stop_reason: None,
            error_code: None,
            input_tokens: None,
            output_tokens: None,
            cache_read_tokens: None,
            cache_write_tokens: None,
            created_at_ms: 0,
            completed_at_ms: None,
            parts: vec![crate::db::chat::ChatPart {
                position: 0,
                kind: "text".into(),
                content: "问".into(),
                provider_metadata_json: None,
                started_at_ms: None,
                ended_at_ms: None,
                completed_at_ms: None,
            }],
            tools: Vec::new(),
        },
        crate::db::chat::ChatMessage {
            external_id: "a1".into(),
            sequence: 1,
            turn_index: 0,
            role: "assistant".into(),
            status: "interrupted".into(),
            model_provider: None,
            model_id: None,
            variant: None,
            stop_reason: None,
            error_code: None,
            input_tokens: None,
            output_tokens: None,
            cache_read_tokens: None,
            cache_write_tokens: None,
            created_at_ms: 0,
            completed_at_ms: None,
            parts: vec![
                crate::db::chat::ChatPart {
                    position: 0,
                    kind: "reasoning".into(),
                    content: "想".into(),
                    provider_metadata_json: None,
                    started_at_ms: Some(1),
                    ended_at_ms: Some(2),
                    completed_at_ms: Some(2),
                },
                crate::db::chat::ChatPart {
                    position: 2,
                    kind: "text".into(),
                    content: "答".into(),
                    provider_metadata_json: None,
                    started_at_ms: None,
                    ended_at_ms: None,
                    completed_at_ms: None,
                },
            ],
            tools: vec![crate::db::chat::ToolCallRecord {
                call_id: "call-1".into(),
                message_id: "a1".into(),
                name: "read".into(),
                arguments_json: r#"{"path":"src/a.rs"}"#.into(),
                position: 1,
                status: "interrupted".into(),
                result_json: None,
                error_text: Some(crate::db::chat::INTERRUPTED_ERROR_CODE.into()),
                started_at_ms: 10,
                completed_at_ms: Some(20),
                duration_ms: Some(10),
            }],
        },
    ];
    assert!(on_session_loaded(
        &mut state,
        pane,
        "sess-mixed",
        &messages,
        None
    ));
    let message = &state.pane_anywhere(pane).unwrap().lx.chat.messages[1];
    assert_eq!(
        message
            .steps
            .iter()
            .map(|step| step.position())
            .collect::<Vec<_>>(),
        vec![0, 1, 2],
        "parts 与 tools 按 position 合并成唯一时间线"
    );
    let call = message.tool("call-1").expect("工具步骤可见");
    assert_eq!(call.status, crate::app::state::ToolStatus::Interrupted);
    assert_eq!(
        call.error.as_deref(),
        Some(crate::db::chat::INTERRUPTED_ERROR_CODE)
    );
    assert_eq!(
        call.summary
            .as_ref()
            .and_then(|summary| summary.path.as_deref()),
        Some("src/a.rs")
    );
    assert_eq!(message.text(), "答");
}
