//! 限额、取消与 usage 累加的编排测试（与 `tests.rs` 共用同一套可控 host/executor）。

use super::*;

/// 步数上限：每步都要求工具，达到 MAX_MODEL_STEPS 时以稳定错误码失败。
#[tokio::test]
async fn step_limit_fails_with_stable_code() {
    let fixture = Fixture::new("step-limit");
    let read_args = r#"{"path":"."}"#.to_string();
    let steps: Vec<Vec<ChatDelta>> = (0..MAX_MODEL_STEPS)
        .map(|index| {
            tool_calls(
                &[(format!("c{index}").as_str(), "read", &read_args, 0)],
                None,
            )
        })
        .collect();
    let script = Script::new(steps, StopReason::Complete);
    let host = RecordingHost::new();
    let executor = RecordingExecutor::new();
    let summary = run_agent_turn(
        context(fixture.path()),
        host,
        executor,
        script.stream_one(),
        no_cancel(),
    )
    .await;
    assert_eq!(
        summary.outcome,
        TurnOutcome::Failed {
            code: STEP_LIMIT_CODE.into()
        }
    );
    // 最后一步只剩采样机会、没有下一步：整批不执行。
    assert_eq!(script.request_count(), MAX_MODEL_STEPS);
}

/// 工具调用上限：一批调用超出配额时整批不执行（不做半批）。
#[tokio::test]
async fn tool_call_limit_fails_before_any_execution() {
    let fixture = Fixture::new("call-limit");
    let search_args = r#"{"pattern":"x"}"#.to_string();
    let ids: Vec<String> = (0..=MAX_TOOL_CALLS)
        .map(|index| format!("c{index}"))
        .collect();
    let calls: Vec<(&str, &str, &str, usize)> = ids
        .iter()
        .enumerate()
        .map(|(ordinal, call_id)| (call_id.as_str(), "search", search_args.as_str(), ordinal))
        .collect();
    let script = Script::new(vec![tool_calls(&calls, None)], StopReason::Complete);
    let host = RecordingHost::new();
    let executor = RecordingExecutor::new();
    let executed = executor.executed.clone();
    let summary = run_agent_turn(
        context(fixture.path()),
        host.clone(),
        executor,
        script.stream_one(),
        no_cancel(),
    )
    .await;
    assert_eq!(
        summary.outcome,
        TurnOutcome::Failed {
            code: TOOL_CALL_LIMIT_CODE.into()
        }
    );
    assert!(executed.lock().expect("lock").is_empty());
    assert!(host.trace().is_empty());
}

/// usage 跨采样累加；未知保持 None（不写 0 冒充真实值）。
#[tokio::test]
async fn usage_accumulates_across_steps_and_keeps_none() {
    let fixture = Fixture::new("usage");
    let read_args = r#"{"path":"."}"#.to_string();
    let script = Script::new(
        vec![
            tool_calls(&[("c1", "read", &read_args, 0)], Some((10, 2))),
            final_response("done", Some((5, 7))),
        ],
        StopReason::Complete,
    );
    let host = RecordingHost::new();
    let executor = RecordingExecutor::new();
    let summary = run_agent_turn(
        context(fixture.path()),
        host,
        executor,
        script.stream_one(),
        no_cancel(),
    )
    .await;
    assert_eq!(summary.outcome, TurnOutcome::Complete);
    assert_eq!(summary.usage.input, Some(15));
    assert_eq!(summary.usage.output, Some(9));
    assert_eq!(summary.usage.cache_read, None, "未知保持 None");
}

/// usage 全部未知时整轮仍为 None（不写 0）。
#[tokio::test]
async fn usage_stays_none_when_provider_reports_nothing() {
    let fixture = Fixture::new("usage-none");
    let script = Script::new(vec![final_response("hi", None)], StopReason::Complete);
    let executor = RecordingExecutor::new();
    let summary = run_agent_turn(
        context(fixture.path()),
        RecordingHost::new(),
        executor,
        script.stream_one(),
        no_cancel(),
    )
    .await;
    assert_eq!(summary.usage, Usage::default());
}

/// 取消：已执行的调用记录真实结果，未开始的调用不执行，且不再采样。
#[tokio::test]
async fn cancel_between_calls_settles_started_one_and_skips_the_rest() {
    let fixture = Fixture::new("cancel");
    let write_args = r#"{"path":"first.txt","content":"a"}"#.to_string();
    let read_args = r#"{"path":"first.txt"}"#.to_string();
    let script = Script::new(
        vec![tool_calls(
            &[
                ("c1", "write", &write_args, 0),
                ("c2", "read", &read_args, 1),
            ],
            None,
        )],
        StopReason::Complete,
    );
    let (cancel_sender, cancel) = watch::channel(false);
    let executor = RecordingExecutor::cancelling(cancel_sender);
    let executed = executor.executed.clone();
    let host = RecordingHost::new();
    let summary = run_agent_turn(
        context(fixture.path()),
        host.clone(),
        executor,
        script.stream_one(),
        cancel,
    )
    .await;
    assert_eq!(summary.outcome, TurnOutcome::Interrupted);
    assert_eq!(
        executed.lock().expect("lock").as_slice(),
        &["c1".to_string()],
        "第二个调用从未执行"
    );
    // 已开始的那个必须有真实结果（不是谎报取消）；未开始的记为 cancelled 记录。
    assert_eq!(
        host.trace(),
        vec![
            HostTrace::Begin("c1".into()),
            HostTrace::Finish("c1".into(), false),
            HostTrace::Begin("c2".into()),
            HostTrace::Finish("c2".into(), true),
        ],
        "已执行的记录真实结果，未执行的结算为 cancelled"
    );
    assert_eq!(fixture.read("first.txt"), "a");
    assert_eq!(script.request_count(), 1, "取消后不再采样");
}

/// 取消发生在 begin ack 之后、执行之前：结算为 cancelled 且不执行为文件 IO。
#[tokio::test]
async fn cancel_after_begin_ack_settles_cancelled_without_execution() {
    let fixture = Fixture::new("cancel-after-ack");
    let write_args = r#"{"path":"never.txt","content":"x"}"#.to_string();
    let script = Script::new(
        vec![tool_calls(&[("c1", "write", &write_args, 0)], None)],
        StopReason::Complete,
    );
    // host 在回执 ticket 的同时发出取消：loop 必须走 cancelled 分支。
    #[derive(Clone)]
    struct CancellingHost {
        cancel: watch::Sender<bool>,
        trace: Arc<Mutex<Vec<String>>>,
    }
    impl AgentHost for CancellingHost {
        fn send(&mut self, request: AgentHostRequest) {
            match request {
                AgentHostRequest::UserShellUpdates { reply } => {
                    let _ = reply.send(Vec::new());
                }
                AgentHostRequest::BeginTool { call, reply } => {
                    self.trace
                        .lock()
                        .expect("lock")
                        .push(format!("begin:{}", call.call_id));
                    let ticket = ToolCallTicket {
                        session_id: "s".into(),
                        message_id: "m".into(),
                        call_id: call.call_id,
                        position: 0,
                        started_at_ms: 0,
                    };
                    let _ = self.cancel.send(true);
                    let _ = reply.send(Ok(ticket));
                }
                AgentHostRequest::FinishTool { ticket, reply, .. } => {
                    self.trace
                        .lock()
                        .expect("lock")
                        .push(format!("finish:{}", ticket.call_id));
                    let _ = reply.send(Ok(()));
                }
            }
        }
    }
    let (cancel_sender, cancel) = watch::channel(false);
    let trace = Arc::new(Mutex::new(Vec::new()));
    let host = CancellingHost {
        cancel: cancel_sender,
        trace: trace.clone(),
    };
    let executor = RecordingExecutor::new();
    let executed = executor.executed.clone();
    let summary = run_agent_turn(
        context(fixture.path()),
        host,
        executor,
        script.stream_one(),
        cancel,
    )
    .await;
    assert_eq!(summary.outcome, TurnOutcome::Interrupted);
    assert!(executed.lock().expect("lock").is_empty(), "不得执行文件 IO");
    assert!(!fixture.exists("never.txt"));
    assert_eq!(
        trace.lock().expect("lock").as_slice(),
        &["begin:c1".to_string(), "finish:c1".to_string()],
        "pending 已提交的调用要结算为 cancelled"
    );
}

/// 纯文本旧路径：一次采样无工具 → Complete。
#[tokio::test]
async fn plain_text_response_completes_in_one_step() {
    let fixture = Fixture::new("plain");
    let script = Script::new(
        vec![final_response("hi", Some((3, 1)))],
        StopReason::Complete,
    );
    let host = RecordingHost::new();
    let executor = RecordingExecutor::new();
    let summary = run_agent_turn(
        context(fixture.path()),
        host.clone(),
        executor,
        script.stream_one(),
        no_cancel(),
    )
    .await;
    assert_eq!(summary.outcome, TurnOutcome::Complete);
    assert_eq!(summary.usage.input, Some(3));
    assert!(host.trace().is_empty(), "无工具时没有 pending 请求");
    assert_eq!(script.request_count(), 1);
}

/// 网络失败：直接失败，不执行任何工具（即使已经回投过工具输入）。
#[tokio::test]
async fn stream_failure_fails_turn_immediately() {
    let fixture = Fixture::new("stream-fail");
    let script = Script::new(
        vec![vec![
            ChatDelta::ToolInputStarted {
                call_id: "c1".into(),
                name: "read".into(),
                ordinal: 0,
            },
            ChatDelta::ResponseFinished {
                stop: ModelStepStop::ToolCalls,
            },
        ]],
        StopReason::Failed {
            code: "http 503".into(),
        },
    );
    let host = RecordingHost::new();
    let executor = RecordingExecutor::new();
    let executed = executor.executed.clone();
    let summary = run_agent_turn(
        context(fixture.path()),
        host.clone(),
        executor,
        script.stream_one(),
        no_cancel(),
    )
    .await;
    assert_eq!(
        summary.outcome,
        TurnOutcome::Failed {
            code: "http 503".into()
        }
    );
    assert!(executed.lock().expect("lock").is_empty());
    assert!(host.trace().is_empty());
}

/// 同响应两工具不会并发：执行与 ack 严格交替。
#[tokio::test]
async fn multiple_calls_in_one_response_never_overlap() {
    let fixture = Fixture::new("serial");
    let first = r#"{"path":"a.txt","content":"1"}"#.to_string();
    let second = r#"{"path":"b.txt","content":"2"}"#.to_string();
    let script = Script::new(
        vec![
            tool_calls(
                &[("c1", "write", &first, 0), ("c2", "write", &second, 1)],
                None,
            ),
            final_response("done", None),
        ],
        StopReason::Complete,
    );
    let host = RecordingHost::new();
    let executor = RecordingExecutor::new();
    let executed = executor.executed.clone();
    let summary = run_agent_turn(
        context(fixture.path()),
        host.clone(),
        executor,
        script.stream_one(),
        no_cancel(),
    )
    .await;
    assert_eq!(summary.outcome, TurnOutcome::Complete);
    assert_eq!(
        executed.lock().expect("lock").as_slice(),
        &["c1".to_string(), "c2".to_string()]
    );
    assert_eq!(
        host.trace(),
        vec![
            HostTrace::Begin("c1".into()),
            HostTrace::Finish("c1".into(), false),
            HostTrace::Begin("c2".into()),
            HostTrace::Finish("c2".into(), false),
        ],
        "begin/finish 严格交替 = 串行执行"
    );
    assert_eq!(fixture.read("a.txt"), "1");
    assert_eq!(fixture.read("b.txt"), "2");
}

/// 工具级失败不终止本轮：失败结果以 is_error 回传后继续采样。
#[tokio::test]
async fn tool_level_failure_continues_the_turn() {
    let fixture = Fixture::new("tool-fail");
    let read_args = r#"{"path":"missing.txt"}"#.to_string();
    let script = Script::new(
        vec![
            tool_calls(&[("c1", "read", &read_args, 0)], None),
            final_response("cannot read", None),
        ],
        StopReason::Complete,
    );
    let summary = run_agent_turn(
        context(fixture.path()),
        RecordingHost::new(),
        RecordingExecutor::new(),
        script.stream_one(),
        no_cancel(),
    )
    .await;
    assert_eq!(summary.outcome, TurnOutcome::Complete);
    assert!(
        script.items_of(1).iter().any(|item| matches!(
            item,
            ModelItem::ToolResult {
                call_id,
                is_error: true,
                ..
            } if call_id == "c1"
        )),
        "工具失败以 is_error=true 回传模型"
    );
}

/// 未知工具名由执行层返回结构化失败，但仍走完整 pending → running → 结果门槛。
#[tokio::test]
async fn unknown_tool_still_goes_through_pending_and_result_gate() {
    let fixture = Fixture::new("unknown-tool");
    let args = r#"{"x":1}"#.to_string();
    let script = Script::new(
        vec![
            tool_calls(&[("c1", "shell", &args, 0)], None),
            final_response("no", None),
        ],
        StopReason::Complete,
    );
    let host = RecordingHost::new();
    let summary = run_agent_turn(
        context(fixture.path()),
        host.clone(),
        RecordingExecutor::new(),
        script.stream_one(),
        no_cancel(),
    )
    .await;
    assert_eq!(summary.outcome, TurnOutcome::Complete);
    assert_eq!(
        host.trace().len(),
        2,
        "未知工具也是「已入库的一次调用」，必须有真实失败结果"
    );
    assert_eq!(ToolErrorCode::UnknownTool.as_str(), "unknown_tool");
}

/// 生产执行器 `FileTools` 走 `spawn_blocking` 并返回结构化结果。
#[tokio::test]
async fn file_tools_executor_writes_through_spawn_blocking() {
    let fixture = Fixture::new("spawn-blocking");
    let call = ReadyToolCall {
        call_id: "c1".into(),
        name: "write".into(),
        arguments_json: r#"{"path":"out.txt","content":"ok"}"#.into(),
        ordinal: 0,
    };
    let result = FileTools.execute(fixture.path(), &call).await;
    assert!(!result.is_failure(), "{result:?}");
    assert_eq!(fixture.read("out.txt"), "ok");
}

/// 同一响应内工具数 > 1 时不会并发（用互斥计数器证明没有重叠区间）。
#[tokio::test]
async fn concurrent_overlap_is_impossible_by_construction() {
    let fixture = Fixture::new("exclusive");
    let args = r#"{"path":"c.txt","content":"v"}"#.to_string();
    let script = Script::new(
        vec![
            tool_calls(
                &[("c1", "write", &args, 0), ("c2", "write", &args, 1)],
                None,
            ),
            final_response("done", None),
        ],
        StopReason::Complete,
    );
    let fixture_path = fixture.path().to_path_buf();
    let state = Arc::new(Mutex::new((0_usize, 0_usize))); // (在途, 最大并发)
    #[derive(Clone)]
    struct OverlapExecutor {
        root: PathBuf,
        state: Arc<Mutex<(usize, usize)>>,
    }
    impl ToolExecutor for OverlapExecutor {
        async fn execute(&self, cwd: &Path, call: &ReadyToolCall) -> ToolExecutionResult {
            {
                let mut guard = self.state.lock().expect("lock");
                guard.0 += 1;
                guard.1 = guard.1.max(guard.0);
            }
            let context = ToolContext::new(cwd.to_path_buf());
            let result = tools::execute(&context, &call.name, &call.arguments_json);
            // 让出一次执行权：若实现并发，第二个调用会在此刻进入并推高最大并发。
            tokio::task::yield_now().await;
            self.state.lock().expect("lock").0 -= 1;
            let _ = &self.root;
            result
        }
    }
    let executor = OverlapExecutor {
        root: fixture_path,
        state: state.clone(),
    };
    let summary = run_agent_turn(
        context(fixture.path()),
        RecordingHost::new(),
        executor,
        script.stream_one(),
        no_cancel(),
    )
    .await;
    assert_eq!(summary.outcome, TurnOutcome::Complete);
    assert_eq!(state.lock().expect("lock").1, 1, "最大并发必须为 1");
}

/// 脚本化 stream 必须在超时内完成（防止测试悬挂）。
#[tokio::test]
async fn scripted_turn_returns_within_timeout() {
    let fixture = Fixture::new("timeout");
    let script = Script::new(vec![final_response("x", None)], StopReason::Complete);
    let summary = tokio::time::timeout(
        Duration::from_secs(5),
        run_agent_turn(
            context(fixture.path()),
            RecordingHost::new(),
            RecordingExecutor::new(),
            script.stream_one(),
            no_cancel(),
        ),
    )
    .await
    .expect("turn completes");
    assert_eq!(summary.outcome, TurnOutcome::Complete);
}
