//! `/resume` 后续轮端到端：从**已落库 + 已加载**的会话历史重新装配请求，
//! 断言实际发给模型的 `ChatRequest.items` 含成对的 call/result，且 interrupted 工具不在其中。
//!
//! 路径与生产一致：`load_chat_session` → `view_message_of`（窗格视图）→
//! `assemble_turn_request`（`chat_part_views_to_db` 含 tools）→ agent loop。

use super::*;

/// 构造一条已落库的 user + assistant（complete）轮次，assistant 内有一个工具步骤。
fn seed_completed_turn(
    fixture: &Fixture,
    database: &Database,
    session: &str,
    assistant_id: &str,
    call_id: &str,
    text: &str,
) {
    database
        .checkpoint(
            fixture.workspace(),
            assistant_id,
            &[crate::db::chat::ChatPart {
                position: 0,
                kind: "text".into(),
                content: text.into(),
                provider_metadata_json: None,
                started_at_ms: None,
                ended_at_ms: None,
                completed_at_ms: None,
            }],
        )
        .expect("checkpoint assistant text");
    let ticket = database
        .begin_tool_call(
            fixture.workspace(),
            session,
            assistant_id,
            call_id,
            "read",
            r#"{"path":"seed.txt"}"#,
        )
        .expect("begin tool call");
    database
        .mark_tool_running(fixture.workspace(), &ticket)
        .expect("mark running");
    database
        .finish_tool_call(
            fixture.workspace(),
            &ticket,
            &ToolCallResult::completed(Some(
                r#"{"tool":"read","content":"1: seeded content"}"#.into(),
            )),
        )
        .expect("finish tool call");
}

/// `/resume` 后继续：发给模型的历史里成对出现 `AssistantToolCall` + `ToolResult`，
/// interrupted 工具既不可见也不进请求。
#[tokio::test]
async fn resume_then_continue_replays_completed_pairs_and_skips_interrupted() {
    let fixture = Fixture::new("resume-replay");
    // 先把会话与工具行写进库（模拟上一进程已经跑完一轮）。
    let database = fixture.database();
    let session_id = {
        let committed = database
            .begin_turn(
                fixture.workspace(),
                None,
                "读一下 seed.txt",
                &crate::db::chat::ModelSnapshot {
                    provider: "p".into(),
                    model: "m".into(),
                    variant: None,
                },
            )
            .expect("begin turn");
        seed_completed_turn(
            &fixture,
            &database,
            &committed.session_id,
            &committed.assistant_message_id,
            "call_seed",
            "我读一下 seed.txt",
        );
        // 再补一个未结算的调用（模拟崩溃遗留 → 启动恢复标 interrupted）。
        let assistant_id = committed.assistant_message_id.clone();
        database
            .begin_tool_call(
                fixture.workspace(),
                &committed.session_id,
                &assistant_id,
                "call_died",
                "write",
                r#"{"path":"dead.txt","content":"x"}"#,
            )
            .expect("begin interrupted call");
        database
            .finish(
                fixture.workspace(),
                &assistant_id,
                &database
                    .load_chat_session(fixture.workspace(), &committed.session_id)
                    .expect("load")
                    .expect("session")
                    .into_iter()
                    .find(|message| message.role == "assistant")
                    .map(|message| message.parts)
                    .unwrap_or_default(),
                &crate::db::chat::TurnOutcome::Complete {
                    stop_reason: Some("stop".into()),
                },
                Usage::default(),
            )
            .expect("finish turn");
        committed.session_id
    };
    assert_eq!(
        database
            .recover_interrupted_tool_calls(std::slice::from_ref(&session_id))
            .expect("recover tool calls"),
        1,
        "遗留调用转 interrupted"
    );
    // 加载会话 → 窗格视图（`view_message_of` 合并 steps）。
    let messages = database
        .load_chat_session(fixture.workspace(), &session_id)
        .expect("load session")
        .expect("session exists");
    let mut state = AppState::demo();
    state.active_workspace_mut().cwd = Some(fixture.workspace().to_path_buf());
    state.models = models_with();
    let pane = state.active_tab().layout.focus();
    assert!(update::on_session_loaded(
        &mut state,
        pane,
        &session_id,
        &messages,
        None
    ));
    // 历史可见：interrupted 工具在步骤时间线里（UI 需展示），且工具数 = 2。
    {
        let history = &state.pane_anywhere(pane).expect("pane").lx.chat.messages;
        let assistant = history
            .iter()
            .find(|message| message.role == "assistant")
            .expect("assistant history");
        assert_eq!(assistant.tools().count(), 2, "两个工具步骤都可见");
        assert_eq!(
            assistant.tool("call_died").map(|call| call.status),
            Some(ToolStatus::Interrupted),
            "interrupted 在历史上明确可见"
        );
    }
    // 发送「继续」：走生产 `try_send` → `on_turn_committed` → 装配 → 假 transport。
    {
        let target = state.pane_mut_anywhere(pane).expect("pane");
        target.lx.entry = crate::app::state::LxEntry::Native;
        target.lx.input = "继续".into();
    }
    assert!(update::try_send(&mut state, pane), "发送受理");
    let generation = match &state.pane_anywhere(pane).expect("pane").lx.chat.request {
        RequestState::Committing { generation, .. } => *generation,
        other => panic!("expected Committing, got {other:?}"),
    };
    let committed = database
        .begin_turn(
            fixture.workspace(),
            Some(&session_id),
            "继续",
            &crate::db::chat::ModelSnapshot {
                provider: "p".into(),
                model: "m".into(),
                variant: None,
            },
        )
        .expect("begin continuation turn");
    assert!(update::on_turn_committed(
        &mut state, pane, generation, &committed
    ));
    let key = RequestKey {
        pane,
        session: session_id.clone(),
        generation,
    };
    state.effects.clear();
    // 用生产装配函数得到本轮请求（`start_stream` 内部即是它）。
    let frozen = crate::app::state::FrozenModel::resolve(
        &state.models,
        &state
            .pane_anywhere(pane)
            .expect("pane")
            .lx
            .chat
            .effective_selection(&state.models)
            .expect("selection"),
    )
    .expect("frozen model");
    let assembled = super::assemble_turn_request(
        &state.pane_anywhere(pane).expect("pane").lx.chat.messages,
        &frozen,
    );
    assert!(
        !assembled.draft_exceeds,
        "「继续」远小于上下文预算，不应报错"
    );
    let request = assembled.request;
    // 装配结果先自检：成对、无 orphan。
    let calls: Vec<&str> = request
        .items
        .iter()
        .filter_map(|item| match item {
            ModelItem::AssistantToolCall { call_id, .. } => Some(call_id.as_str()),
            _ => None,
        })
        .collect();
    let results: Vec<&str> = request
        .items
        .iter()
        .filter_map(|item| match item {
            ModelItem::ToolResult { call_id, .. } => Some(call_id.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(calls, vec!["call_seed"], "只回放已完成且有结果的调用");
    assert_eq!(results, vec!["call_seed"], "结果与调用成对");
    assert!(
        !calls.contains(&"call_died") && !results.contains(&"call_died"),
        "interrupted 调用不得进入模型请求"
    );
    // 端到端：假 transport 捕获实际发出的请求。
    let transport = FakeTransport::new(vec![final_response("ok", None)], StopReason::Complete);
    let persistence = Persistence::with_database(database);
    let mut streams = ChatStreams::default();
    let driven = drive_request(
        &fixture,
        &mut state,
        &key,
        &transport,
        &persistence,
        &mut streams,
        no_cancel(),
        FileTools,
        Hooks::default(),
        request,
    )
    .await;
    assert_eq!(driven.summary.outcome, TurnOutcome::Complete);
    let sent = transport.requests();
    assert_eq!(sent.len(), 1, "一次采样");
    let items = &sent[0].items;
    // 请求里成对的 call/result（位置相邻且 call 在前）。
    let call_at = items
        .iter()
        .position(|item| {
            matches!(item, ModelItem::AssistantToolCall { call_id, .. } if call_id == "call_seed")
        })
        .expect("历史调用进入实际请求");
    assert!(
        matches!(
            items.get(call_at + 1),
            Some(ModelItem::ToolResult { call_id, is_error: false, content })
                if call_id == "call_seed" && content.contains("seeded content")
        ),
        "实际请求里调用后面紧跟同 call_id 的结果: {:?}",
        items.get(call_at + 1)
    );
    assert!(
        !items.iter().any(|item| matches!(
            item,
            ModelItem::AssistantToolCall { call_id, .. }
                | ModelItem::ToolResult { call_id, .. } if call_id == "call_died"
        )),
        "interrupted 工具不出现在实际请求"
    );
    // 未产生任何文件副作用（interrupted 不被重放）。
    assert!(!fixture.exists("dead.txt"), "恢复不重放文件操作");
}

/// 当前用户消息单独超出上下文预算：装配标记 `draft_exceeds`（调用方报错，不截断输入）。
#[test]
fn oversized_draft_is_flagged_by_the_production_assembler() {
    let mut state = AppState::demo();
    state.models = models_with();
    let pane = state.active_tab().layout.focus();
    let messages = vec![crate::app::state::ChatMessageView {
        sequence: 0,
        id: "u1".into(),
        role: "user".into(),
        status: "streaming".into(),
        turn_index: 0,
        kind: "chat".into(),
        shell: None,
        steps: vec![crate::app::state::AssistantStepView::Part {
            position: 0,
            part: crate::app::state::ChatPartView {
                kind: crate::chat::events::PartKind::Text,
                content: "超长".repeat(20_000),
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
    let mut frozen = crate::app::state::FrozenModel::resolve(
        &state.models,
        &state
            .pane_anywhere(pane)
            .expect("pane")
            .lx
            .chat
            .effective_selection(&state.models)
            .expect("selection"),
    )
    .expect("frozen model");
    // 上下文上限调到远小于草稿估算值。
    frozen.context_limit = Some(2_000);
    let assembled = super::assemble_turn_request(&messages, &frozen);
    assert!(
        assembled.draft_exceeds,
        "当前用户消息超预算必须被标记（不截断用户输入）"
    );
    assert!(
        assembled.request.items.iter().any(
            |item| matches!(item, ModelItem::UserText(text) if text.chars().count() == 40_000)
        ),
        "用户输入不被截断"
    );
}

/// 生产装配入口的失败路径：当前用户消息超预算 → 整轮失败且不 spawn 网络流。
///
/// 这里走真实的 `pump_chat_streams`（`StartChatStream` 效应 → `start_stream`），
/// 断言回投的是稳定错误码，而不是带着截断输入去请求供应商。
#[test]
fn start_stream_fails_the_turn_when_draft_exceeds_context() {
    let fixture = Fixture::new("draft-exceeds");
    let (mut state, key, persistence, _session) = streaming_state(&fixture);
    // 把模型上下文上限压到远小于本轮草稿（`on_turn_committed` 已写入的长正文）。
    if let Some(model) = state
        .models
        .settings
        .providers
        .get_mut("p")
        .and_then(|provider| provider.models.get_mut("m"))
    {
        model.limit = Some(crate::config::ai::ModelLimit {
            context: 1_000,
            output: 128,
        });
    }
    let assistant_id = match &state.pane_anywhere(key.pane).expect("pane").lx.chat.request {
        RequestState::Streaming { assistant_id, .. } => assistant_id.clone(),
        other => panic!("expected Streaming, got {other:?}"),
    };
    // 放大本轮 user 正文到明显超出 1_000 token 预算。
    {
        let target = state.pane_mut_anywhere(key.pane).expect("pane");
        let message = target
            .lx
            .chat
            .messages
            .iter_mut()
            .find(|message| message.role == "user")
            .expect("user message");
        if let Some(crate::app::state::AssistantStepView::Part { part, .. }) =
            message.steps.first_mut()
        {
            part.content = "超长输入".repeat(10_000);
        }
    }
    state.effects.push(Effect::StartChatStream {
        key: key.clone(),
        assistant_id,
    });
    let (sender, mut receiver) = mpsc::unbounded_channel::<AppEvent>();
    let mut streams = ChatStreams::default();
    assert!(pump_chat_streams(
        &mut state,
        &mut streams,
        &sender,
        &persistence
    ));
    let event = receiver.try_recv().expect("terminal event emitted");
    let AppEvent::ChatNet(ChatNetEvent::Finished { reason, .. }) = event else {
        panic!("expected a terminal ChatNet event, got {event:?}");
    };
    assert_eq!(
        reason,
        StopReason::Failed {
            code: DRAFT_EXCEEDS_CONTEXT_CODE.to_string()
        },
        "超预算草稿走稳定错误码，不截断用户输入"
    );
    // 回投终态：本轮 assistant 以该错误码失败（UI 可见原因）。
    assert!(handle_net_event(
        &mut state,
        &mut streams,
        ChatNetEvent::Finished {
            key: key.clone(),
            reason: reason.clone(),
            usage: Usage::default(),
        }
    ));
    let message = assistant_message(&state, &key);
    assert_eq!(message.status, "failed");
    assert_eq!(
        message.error_code.as_deref(),
        Some(DRAFT_EXCEEDS_CONTEXT_CODE)
    );
    // 用户输入保持原样（未被截断改写）。
    let user_text = state
        .pane_anywhere(key.pane)
        .expect("pane")
        .lx
        .chat
        .messages
        .iter()
        .find(|message| message.role == "user")
        .map(|message| message.text())
        .expect("user message");
    assert_eq!(
        user_text.chars().count(),
        "超长输入".repeat(10_000).chars().count()
    );
}
