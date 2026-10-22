//! DB ack 失败、崩溃恢复、Esc 取消与旧 generation 过滤（端到端）。

use super::*;

// ===== 3. DB ack 失败 =====

/// pending 失败（工作区未登记）：不执行文件 IO、本轮失败。
#[tokio::test]
async fn pending_failure_has_no_file_side_effect_and_fails_turn() {
    let fixture = Fixture::new("pending-fail");
    let (mut state, key, _persistence, session) = streaming_state(&fixture);
    // 换成一个「工作区未登记」的库：begin_tool_call 必然失败（等价于 DB ack 失败）。
    let untrusted = Database::open(&fixture.root.join("untrusted.db")).expect("open side db");
    let persistence = Persistence::with_database(untrusted);
    let write_args = r#"{"path":"never.txt","content":"x"}"#.to_string();
    let transport = FakeTransport::new(
        vec![tool_calls(&[("c1", "write", &write_args, 0)], None)],
        StopReason::Complete,
    );
    let mut streams = ChatStreams::default();
    streams
        .db_workspaces
        .insert(key.clone(), fixture.workspace().to_path_buf());
    // 必须让该窗格「可见」：可见窗格的 BeginTool 才会推迟到该帧绘制后回执
    // （正是这个推迟窗口让 pending 条目可能被遗弃）。
    streams.set_visible_panes(&[(key.pane, Rect::new(0, 0, 80, 24))]);
    // 在途轮次（生产由 `start_stream` 写入）；`reap_stale_streams` 遍历的就是它。
    let (cancel_sender, _cancel) = watch::channel(false);
    streams.cancels.insert(key.clone(), cancel_sender);
    let driven = drive(
        &fixture,
        &mut state,
        &key,
        &transport,
        &persistence,
        &mut streams,
        no_cancel(),
    )
    .await;
    match &driven.summary.outcome {
        TurnOutcome::Failed { code } => {
            assert!(
                code.starts_with(BEGIN_TOOL_FAILED_CODE),
                "pending 失败错误码带稳定前缀: {code}"
            );
        }
        other => panic!("pending 失败必须整轮失败，实际 {other:?}"),
    }
    assert!(!fixture.exists("never.txt"), "无文件副作用");
    assert!(driven.executed.is_empty(), "不得执行文件工具");
    assert_eq!(
        driven.trace,
        vec![Trace::Begin("c1".into())],
        "只有 begin 尝试，没有执行/结算"
    );
    let (reason, _) = driven.finished.expect("terminal event");
    assert!(matches!(reason, StopReason::Failed { .. }));
    assert!(tool_rows(&fixture, &session).is_empty(), "失败时不留工具行");
}

/// running 失败（pending 已提交后行被删除）：不执行文件 IO、本轮失败。
#[tokio::test]
async fn running_failure_has_no_file_side_effect() {
    let fixture = Fixture::new("running-fail");
    let (mut state, key, persistence, _session) = streaming_state(&fixture);
    let write_args = r#"{"path":"never2.txt","content":"x"}"#.to_string();
    let transport = FakeTransport::new(
        vec![
            tool_calls(&[("c1", "write", &write_args, 0)], None),
            final_response("done", None),
        ],
        StopReason::Complete,
    );
    let mut streams = ChatStreams::default();
    // 插桩：pending 已提交、ticket 尚未回执时删掉该行（模拟 running 入库失败）。
    let hook: Interleave = Arc::new(|fixture, _key| {
        let database = fixture.reopen();
        database
            .connection()
            .execute("DELETE FROM agent_tool_call WHERE call_id = 'c1'", [])
            .expect("delete pending row");
    });
    let driven = drive_with_hook(
        &fixture,
        &mut state,
        &key,
        &transport,
        &persistence,
        &mut streams,
        no_cancel(),
        FileTools,
        Hooks {
            before_running: Some(hook),
            before_finish: None,
        },
    )
    .await;
    match &driven.summary.outcome {
        TurnOutcome::Failed { code } => assert!(
            code.starts_with(RUNNING_TOOL_FAILED_CODE),
            "running 失败错误码带稳定前缀: {code}"
        ),
        other => panic!("running 失败必须整轮失败，实际 {other:?}"),
    }
    assert!(!fixture.exists("never2.txt"), "无文件副作用");
    assert!(
        driven.executed.is_empty(),
        "running 未成功前不得执行文件工具"
    );
}

/// 结果落库失败（行已被别的结果占住）：不重试写入、本轮失败。
#[tokio::test]
async fn result_persistence_failure_does_not_retry_the_write() {
    let fixture = Fixture::new("finish-fail");
    let (mut state, key, persistence, session) = streaming_state(&fixture);
    let write_args = r#"{"path":"once.txt","content":"v1"}"#.to_string();
    let transport = FakeTransport::new(
        vec![
            tool_calls(&[("c1", "write", &write_args, 0)], None),
            final_response("done", None),
        ],
        StopReason::Complete,
    );
    let mut streams = ChatStreams::default();
    // 插桩：pending 已提交、尚未执行时把该行先结成一个**不同结果**的终态，
    // 使随后的 finish_tool_call 返回 ToolTerminalConflict（结果落库失败）。
    let hook: Interleave = Arc::new(|fixture, key| {
        let database = fixture.reopen();
        let mut rows = database
            .load_tool_calls_for_session(fixture.workspace(), &key.session)
            .expect("load");
        let row = rows.remove(0);
        let ticket = crate::db::chat::ToolCallTicket {
            session_id: key.session.clone(),
            message_id: row.message_id,
            call_id: row.call_id,
            position: row.position,
            started_at_ms: row.started_at_ms,
        };
        database
            .mark_tool_running(fixture.workspace(), &ticket)
            .expect("mark running");
        database
            .finish_tool_call(
                fixture.workspace(),
                &ticket,
                &ToolCallResult {
                    status: ToolTerminalStatus::Completed,
                    result_json: Some("{\"tool\":\"write\",\"foreign\":true}".into()),
                    error_text: None,
                },
            )
            .expect("foreign settle");
    });
    let driven = drive_with_hook(
        &fixture,
        &mut state,
        &key,
        &transport,
        &persistence,
        &mut streams,
        no_cancel(),
        FileTools,
        Hooks {
            before_running: None,
            before_finish: Some(hook),
        },
    )
    .await;
    match &driven.summary.outcome {
        TurnOutcome::Failed { code } => assert!(
            code.starts_with(FINISH_TOOL_FAILED_CODE),
            "结果落库失败错误码带稳定前缀: {code}"
        ),
        other => panic!("结果落库失败必须整轮失败，实际 {other:?}"),
    }
    // 只执行一次：落库失败后绝不重试写入。
    assert_eq!(driven.executed.len(), 1, "不得重试写入");
    assert_eq!(fixture.read("once.txt"), "v1", "文件已写入且不重复写");
    // UI 明示“结果持久化失败，文件状态需核查”——走 main content 的 danger toast。
    let toast = state.toast.as_ref().expect("应有提示");
    assert_eq!(
        toast.message,
        crate::ui::text::LX_CHAT_TOOL_RESULT_SAVE_FAILED
    );
    assert_eq!(toast.kind, crate::app::toast::ToastKind::Danger);
    // DB 里保留的是那次“外来”结果，不会被覆盖。
    let rows = tool_rows(&fixture, &session);
    assert_eq!(rows.len(), 1);
    assert!(
        rows[0]
            .result_json
            .as_deref()
            .is_some_and(|json| json.contains("foreign")),
        "不覆盖已落库终态"
    );
}

// ===== 4. 崩溃恢复 =====

#[tokio::test]
async fn crash_recovery_marks_interrupted_and_resume_never_reexecutes() {
    let fixture = Fixture::new("recovery");
    let (state, key, _persistence, session) = streaming_state(&fixture);
    let assistant_id = match &pane_of(&state, &key).lx.chat.request {
        RequestState::Streaming { assistant_id, .. } => assistant_id.clone(),
        other => panic!("expected Streaming, got {other:?}"),
    };
    // 模拟异常退出：留下一行 pending 与一行 running。
    {
        let database = fixture.reopen();
        let pending = database
            .begin_tool_call(
                fixture.workspace(),
                &session,
                &assistant_id,
                "c_pending",
                "write",
                r#"{"path":"a.txt","content":"x"}"#,
            )
            .expect("pending");
        let running = database
            .begin_tool_call(
                fixture.workspace(),
                &session,
                &assistant_id,
                "c_running",
                "write",
                r#"{"path":"b.txt","content":"y"}"#,
            )
            .expect("running");
        database
            .mark_tool_running(fixture.workspace(), &running)
            .expect("mark running");
        assert!(pending.position < running.position, "position 单调");
    }
    // 重新打开 + 启动恢复（与 `Persistence::start` 的恢复路径一致）。
    let reopened = fixture.reopen();
    assert_eq!(
        reopened
            .recover_interrupted_tool_calls()
            .expect("recover tool calls"),
        2,
        "pending 与 running 都转 interrupted"
    );
    assert_eq!(
        reopened
            .recover_interrupted_tool_calls()
            .expect("idempotent"),
        0,
        "恢复动作幂等"
    );
    let rows = reopened
        .load_tool_calls_for_session(fixture.workspace(), &session)
        .expect("load");
    assert_eq!(rows.len(), 2);
    for row in &rows {
        assert_eq!(row.status, "interrupted");
        assert_eq!(
            row.error_text.as_deref(),
            Some(crate::db::chat::INTERRUPTED_ERROR_CODE),
            "中断码明确"
        );
        assert!(row.result_json.is_none(), "绝不伪造成功结果");
    }
    // `/resume` 加载可见（工具行按 position 有序、状态 interrupted）。
    let messages = reopened
        .load_chat_session(fixture.workspace(), &session)
        .expect("load session")
        .expect("session");
    let assistant = messages
        .iter()
        .find(|message| message.role == "assistant")
        .expect("assistant");
    assert_eq!(assistant.tools.len(), 2);
    assert!(
        assistant
            .tools
            .iter()
            .all(|tool| tool.status == "interrupted")
    );
    assert!(assistant.tools[0].position < assistant.tools[1].position);
    // 恢复不重放任何文件操作。
    assert!(!fixture.exists("a.txt"), "恢复不重放写文件");
    assert!(!fixture.exists("b.txt"), "恢复不重放写文件");
}

// ===== 5. Esc 取消 =====

#[tokio::test]
async fn escape_cancel_skips_unstarted_calls_and_keeps_real_result() {
    let fixture = Fixture::new("escape");
    let (mut state, key, persistence, session) = streaming_state(&fixture);
    let first = r#"{"path":"first.txt","content":"a"}"#.to_string();
    let second = r#"{"path":"second.txt","content":"b"}"#.to_string();
    let transport = FakeTransport::new(
        vec![tool_calls(
            &[("c1", "write", &first, 0), ("c2", "write", &second, 1)],
            None,
        )],
        StopReason::Complete,
    );
    let (cancel_sender, cancel) = watch::channel(false);
    // 第一个工具执行完即触发取消（等价于 Esc 落在两个工具之间）。
    let executor = CancelAfterExecute {
        cancel: cancel_sender,
    };
    let mut streams = ChatStreams::default();
    let driven = drive_with_hook(
        &fixture,
        &mut state,
        &key,
        &transport,
        &persistence,
        &mut streams,
        cancel,
        executor,
        Hooks::default(),
    )
    .await;
    assert_eq!(
        driven.summary.outcome,
        TurnOutcome::Interrupted,
        "取消 → 中断"
    );
    let (reason, _) = driven.finished.expect("terminal event");
    assert_eq!(reason, StopReason::Cancelled);
    // 已开始的第一个工具记录真实结果；第二个从未执行。
    assert!(fixture.exists("first.txt"), "已开始的写入记录真实结果");
    assert!(!fixture.exists("second.txt"), "未开始的调用不得执行");
    assert_eq!(driven.executed, vec!["c1".to_string()], "只执行了第一个");
    let rows = tool_rows(&fixture, &session);
    let statuses: Vec<(&str, &str)> = rows
        .iter()
        .map(|row| (row.call_id.as_str(), row.status.as_str()))
        .collect();
    assert!(
        statuses.contains(&("c1", "completed")),
        "已开始的调用记录真实成功结果: {statuses:?}"
    );
    assert!(
        statuses.contains(&("c2", "cancelled")),
        "未开始的调用标 cancelled: {statuses:?}"
    );
    // DB 恢复时不会再把这个 cancelled 行当 running/interrupted。
    let reopened = fixture.reopen();
    assert_eq!(
        reopened.recover_interrupted_tool_calls().expect("recover"),
        0,
        "已结算的调用不受恢复影响"
    );
    // 取消后不再采样（只有一次模型流）。
    assert_eq!(transport.calls(), 1, "取消后不得继续采样");
}

/// 执行完即发出取消的执行器。
struct CancelAfterExecute {
    cancel: watch::Sender<bool>,
}

impl ToolExecutor for CancelAfterExecute {
    async fn execute(&self, cwd: &Path, call: &ReadyToolCall) -> ToolExecutionResult {
        let context = ToolContext::new(cwd.to_path_buf());
        let result = crate::chat::tools::execute(&context, &call.name, &call.arguments_json);
        let _ = self.cancel.send(true);
        result
    }
}

// ===== 6. 旧 generation 事件 =====

#[tokio::test]
async fn stale_generation_events_do_not_pollute_the_new_turn() {
    let fixture = Fixture::new("stale");
    let (mut state, key, persistence, session) = streaming_state(&fixture);
    let stale = RequestKey {
        pane: key.pane,
        session: key.session.clone(),
        generation: key.generation.saturating_sub(1),
    };
    let mut streams = ChatStreams::default();
    streams
        .db_workspaces
        .insert(stale.clone(), fixture.workspace().to_path_buf());
    // 旧 generation 的 delta 被丢弃。
    let deltas = vec![ChatDelta::Part(PartDelta {
        kind: PartKind::Text,
        text: "stale".into(),
        metadata: None,
    })];
    assert!(
        !handle_net_event(
            &mut state,
            &mut streams,
            ChatNetEvent::Delta {
                key: stale.clone(),
                deltas,
            }
        ),
        "旧 generation 的 delta 被丢弃"
    );
    assert!(
        assistant_message(&state, &key).text().is_empty(),
        "旧 delta 不进入新轮时间线"
    );
    // 旧 generation 的 BeginTool：直接拒绝（等价取消），不写库、不执行文件。
    let (reply, receiver) = tokio::sync::oneshot::channel();
    handle_agent_host_event(
        &mut state,
        &mut streams,
        &persistence,
        AgentHostEvent::BeginTool {
            key: stale.clone(),
            call: ReadyToolCall {
                call_id: "stale_call".into(),
                name: "write".into(),
                arguments_json: r#"{"path":"stale.txt","content":"x"}"#.into(),
                ordinal: 0,
            },
            reply,
        },
    );
    assert_eq!(
        receiver.await.expect("ack delivered"),
        Err(HostError::Cancelled),
        "旧 key 的 BeginTool 直接拒绝（等价取消）"
    );
    assert!(!fixture.exists("stale.txt"), "旧 key 不得有文件副作用");
    assert!(
        tool_rows(&fixture, &session).is_empty(),
        "旧 key 不写工具行"
    );
}

// ===== 附加：§11 矩阵的其余失败/一致性路径 =====

/// Esc 落在「pending 已提交、ticket 尚未回执」之间：窗口关闭后不再执行文件 IO，
/// 该行按取消结算（不是 running/interrupted 的中间态）。
#[tokio::test]
async fn cancel_while_waiting_for_ticket_never_executes_the_tool() {
    let fixture = Fixture::new("cancel-in-ack");
    let (mut state, key, persistence, session) = streaming_state(&fixture);
    let write_args = r#"{"path":"never.txt","content":"x"}"#.to_string();
    let transport = FakeTransport::new(
        vec![
            tool_calls(&[("c1", "write", &write_args, 0)], None),
            final_response("done", None),
        ],
        StopReason::Complete,
    );
    let (cancel_sender, cancel) = watch::channel(false);
    let mut streams = ChatStreams::default();
    // 插桩：pending 已提交、ticket 尚未回执时即取消，并丢弃待回执项（模拟窗格关闭）。
    let sender_for_hook = cancel_sender.clone();
    let hook: Interleave = Arc::new(move |_fixture, _key| {
        let _ = sender_for_hook.send(true);
    });
    let driven = drive_with_hook(
        &fixture,
        &mut state,
        &key,
        &transport,
        &persistence,
        &mut streams,
        cancel,
        FileTools,
        Hooks {
            before_running: Some(hook),
            before_finish: None,
        },
    )
    .await;
    assert_eq!(
        driven.summary.outcome,
        TurnOutcome::Interrupted,
        "取消后整轮中断"
    );
    assert!(!fixture.exists("never.txt"), "取消后绝不执行文件 IO");
    assert!(driven.executed.is_empty());
    // 已提交的 pending 行被结算为 cancelled，不会残留 running/pending 等恢复。
    let rows = tool_rows(&fixture, &session);
    if let Some(row) = rows.first() {
        assert_eq!(row.status, "cancelled", "已入库 pending 结算为 cancelled");
    }
    assert_eq!(
        fixture
            .reopen()
            .recover_interrupted_tool_calls()
            .expect("recover"),
        0,
        "没有遗留 pending/running 行"
    );
}

/// 工具级失败（如文件不存在）不终止本轮：失败结果回传模型后继续采样。
#[tokio::test]
async fn tool_level_failure_keeps_the_turn_alive_and_records_failed() {
    let fixture = Fixture::new("tool-fail");
    let (mut state, key, persistence, session) = streaming_state(&fixture);
    let read_args = r#"{"path":"missing.txt"}"#.to_string();
    let transport = FakeTransport::new(
        vec![
            tool_calls(&[("c1", "read", &read_args, 0)], None),
            final_response("cannot read", None),
        ],
        StopReason::Complete,
    );
    let mut streams = ChatStreams::default();
    let driven = drive(
        &fixture,
        &mut state,
        &key,
        &transport,
        &persistence,
        &mut streams,
        no_cancel(),
    )
    .await;
    assert_eq!(
        driven.summary.outcome,
        TurnOutcome::Complete,
        "工具失败不终止本轮"
    );
    // DB 记录 failed + 错误文本；UI 状态同步为 Failed。
    let rows = tool_rows(&fixture, &session);
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].status, "failed");
    assert!(rows[0].error_text.is_some(), "失败原因已记录");
    assert!(rows[0].result_json.is_some(), "失败也有结构化结果体");
    let message = assistant_message(&state, &key);
    assert_eq!(
        message.tools().next().expect("tool").status,
        ToolStatus::Failed
    );
    // 失败结果以 is_error 回传模型，模型随后给出最终回答。
    let requests = transport.requests();
    assert!(requests[1].items.iter().any(|item| matches!(
        item,
        ModelItem::ToolResult {
            call_id,
            is_error: true,
            ..
        } if call_id == "c1"
    )));
    assert_eq!(transport.calls(), 2, "工具失败后继续采样一次");
}

/// 同会话同 call ID 二次出现：DB 冲突 → 本轮失败、不产生第二次文件副作用。
#[tokio::test]
async fn duplicate_call_id_is_rejected_without_second_side_effect() {
    let fixture = Fixture::new("dup-call");
    let (mut state, key, persistence, session) = streaming_state(&fixture);
    // 先在库里留下同 call ID 的工具行（模拟上一轮已用过该 ID）。
    let assistant_id = match &pane_of(&state, &key).lx.chat.request {
        RequestState::Streaming { assistant_id, .. } => assistant_id.clone(),
        other => panic!("expected Streaming, got {other:?}"),
    };
    {
        let database = fixture.reopen();
        database
            .begin_tool_call(
                fixture.workspace(),
                &session,
                &assistant_id,
                "dup",
                "write",
                r#"{"path":"first.txt","content":"old"}"#,
            )
            .expect("seed row");
    }
    let write_args = r#"{"path":"second.txt","content":"new"}"#.to_string();
    let transport = FakeTransport::new(
        vec![
            tool_calls(&[("dup", "write", &write_args, 0)], None),
            final_response("done", None),
        ],
        StopReason::Complete,
    );
    let mut streams = ChatStreams::default();
    let driven = drive(
        &fixture,
        &mut state,
        &key,
        &transport,
        &persistence,
        &mut streams,
        no_cancel(),
    )
    .await;
    match &driven.summary.outcome {
        TurnOutcome::Failed { code } => assert!(
            code.starts_with(BEGIN_TOOL_FAILED_CODE),
            "同 call ID 冲突 → pending 失败: {code}"
        ),
        other => panic!("重复 call ID 必须失败，实际 {other:?}"),
    }
    assert!(driven.executed.is_empty(), "冲突时不得执行文件工具");
    assert!(!fixture.exists("second.txt"), "不得产生第二次文件副作用");
    // 库里仍是原来那一行（未被覆盖）。
    let rows = tool_rows(&fixture, &session);
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].call_id, "dup");
}

// ===== review 回归：被遗弃的 pending 必须落成 cancelled =====

#[tokio::test]
async fn abandoning_a_pending_begin_records_cancelled_not_pending() {
    // 真实缺陷：`begin_tool` 已把 `agent_tool_call` 写成 `pending` 并提交，
    // 若此时窗格被关 / 会话被切 / `/new`，`drop_pending` 直接把 `PendingBegin`
    // 丢掉 → `reply` 被 drop → loop 拿 `Err(RecvError)` 直接返回，`finish_tool`
    // 永远不会再被调用。库里那一行**永久停在 `pending`**：重开会话时卡片一直
    // 显示 Pending，直到整个进程重启的恢复逻辑才纠正。
    let fixture = Fixture::new("abandon-pending");
    let (mut state, key, persistence, session) = streaming_state(&fixture);
    let mut streams = ChatStreams::default();
    streams
        .db_workspaces
        .insert(key.clone(), fixture.workspace().to_path_buf());
    // 必须让该窗格「可见」：可见窗格的 BeginTool 才会推迟到该帧绘制后回执
    // （正是这个推迟窗口让 pending 条目可能被遗弃）。
    streams.set_visible_panes(&[(key.pane, Rect::new(0, 0, 80, 24))]);
    // 在途轮次（生产由 `start_stream` 写入）；`reap_stale_streams` 遍历的就是它。
    let (cancel_sender, _cancel) = watch::channel(false);
    streams.cancels.insert(key.clone(), cancel_sender);

    // 提交一条 pending（DB 已写 pending 行），并让它停在「等该帧绘制」的状态。
    let (reply, mut reply_rx) = oneshot::channel();
    let committed = crate::event::chat::tool_calls::begin_tool(
        &mut state,
        &mut streams,
        &persistence,
        key.clone(),
        ReadyToolCall {
            call_id: "c-abandoned".into(),
            name: "read".into(),
            arguments_json: r#"{"path":"a.txt"}"#.into(),
            ordinal: 0,
        },
        reply,
    );
    assert!(committed, "BeginTool 已受理");
    assert_eq!(
        tool_rows(&fixture, &session)[0].status,
        "pending",
        "前提：DB 里已经是 pending"
    );

    // 模拟会话切换：窗格解绑该会话并退回 Idle → 该轮不再归属于任何在途请求。
    {
        let pane = state.pane_mut_anywhere(key.pane).expect("pane");
        pane.lx.chat.session_id = None;
        pane.lx.chat.request = crate::app::state::RequestState::Idle;
    }
    assert!(
        crate::event::chat::active_assistant_id(&state, &key).is_none(),
        "前提：该轮已不再归属于任何在途请求"
    );
    let (sender, _receiver) = mpsc::unbounded_channel::<AppEvent>();
    crate::event::chat::pump_chat_streams(&mut state, &mut streams, &sender, &persistence);

    // 关键断言：DB 行必须被落成 cancelled，而不是留在 pending。
    assert_eq!(
        tool_rows(&fixture, &session)[0].status,
        "cancelled",
        "被遗弃的 pending 必须落成 cancelled（否则卡片永久显示 Pending）"
    );
    // 并且 loop 侧拿到的回执是「取消」，不会去执行文件 IO。
    assert!(
        matches!(reply_rx.try_recv(), Ok(Err(HostError::Cancelled))),
        "回执必须是 Cancelled"
    );
}
