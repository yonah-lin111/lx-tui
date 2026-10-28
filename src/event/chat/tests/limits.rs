//! 纯文本旧路径回归、顺序与插桩证据。

use super::*;

// ===== 7. 纯文本旧路径回归 =====

#[tokio::test]
async fn plain_text_turn_matches_legacy_single_sample_behavior() {
    let fixture = Fixture::new("plain");
    let (mut state, key, persistence, session) = streaming_state(&fixture);
    let transport = FakeTransport::new(
        vec![final_response("hello", Some((7, 2)))],
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
    assert_eq!(driven.summary.outcome, TurnOutcome::Complete);
    assert_eq!(transport.calls(), 1, "一次采样");
    assert!(driven.trace.is_empty(), "无工具 → 没有 pending 请求");
    let (reason, usage) = driven.finished.expect("terminal event");
    assert_eq!(reason, StopReason::Complete);
    assert_eq!(usage.input, Some(7));
    // FinishChatTurn 只入队一次（消息终态与累计 usage 只提交一次）。
    let finishes = state
        .effects
        .iter()
        .filter(|effect| matches!(effect, Effect::FinishChatTurn { .. }))
        .count();
    assert_eq!(finishes, 1, "终态只提交一次");
    let message = assistant_message(&state, &key);
    assert_eq!(message.status, "complete");
    assert_eq!(message.text(), "hello");
    assert!(tool_rows(&fixture, &session).is_empty());
}

// ===== 9. 顺序不变量 =====

#[tokio::test]
async fn multiple_calls_never_execute_concurrently() {
    let fixture = Fixture::new("exclusive");
    let (mut state, key, persistence, _session) = streaming_state(&fixture);
    let first = r#"{"path":"a.txt","content":"1"}"#.to_string();
    let second = r#"{"path":"b.txt","content":"2"}"#.to_string();
    let transport = FakeTransport::new(
        vec![
            tool_calls(
                &[("c1", "write", &first, 0), ("c2", "write", &second, 1)],
                None,
            ),
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
    assert_eq!(driven.summary.outcome, TurnOutcome::Complete);
    // begin/finish 严格交替 ⇒ 第二个 begin 必在第一个 finish 之后（不可能并发）。
    assert_eq!(
        driven.trace,
        vec![
            Trace::Begin("c1".into()),
            Trace::Finish("c1".into(), ToolTerminalStatus::Completed),
            Trace::Begin("c2".into()),
            Trace::Finish("c2".into(), ToolTerminalStatus::Completed),
        ]
    );
    assert_eq!(fixture.read("a.txt"), "1");
    assert_eq!(fixture.read("b.txt"), "2");
}

// ===== 附加：pending 先于文件写入的直接插桩证据 =====

#[tokio::test]
async fn pending_row_committed_before_the_file_is_written() {
    let fixture = Fixture::new("pending-first");
    let (mut state, key, persistence, session) = streaming_state(&fixture);
    let write_args = r#"{"path":"gate.txt","content":"payload"}"#.to_string();
    let transport = FakeTransport::new(
        vec![
            tool_calls(&[("c1", "write", &write_args, 0)], None),
            final_response("done", None),
        ],
        StopReason::Complete,
    );
    // 执行文件工具的那一刻检查：DB 已有该 call 的 running 行，且目标文件还不存在。
    let observed = Arc::new(Mutex::new(Vec::<(bool, bool)>::new()));
    let executor = GateExecutor {
        db_path: fixture.db_path(),
        workspace: fixture.workspace().to_path_buf(),
        session: session.to_string(),
        observed: observed.clone(),
    };
    let mut streams = ChatStreams::default();
    let driven = drive_with_hook(
        &fixture,
        &mut state,
        &key,
        &transport,
        &persistence,
        &mut streams,
        no_cancel(),
        executor,
        Hooks::default(),
    )
    .await;
    assert_eq!(driven.summary.outcome, TurnOutcome::Complete);
    let observed = observed.lock().expect("lock").clone();
    assert_eq!(observed.len(), 1, "写入前只执行一次");
    let (recorded, file_absent) = observed[0];
    assert!(recorded, "执行文件工具时 DB 已有该调用的 running 行");
    assert!(file_absent, "执行文件工具时目标文件还不存在");
    assert_eq!(fixture.read("gate.txt"), "payload");
    assert_eq!(tool_rows(&fixture, &session)[0].status, "completed");
}

/// 在真正执行前核对 DB 行与文件状态。
struct GateExecutor {
    db_path: PathBuf,
    workspace: PathBuf,
    session: String,
    observed: Arc<Mutex<Vec<(bool, bool)>>>,
}

impl ToolExecutor for GateExecutor {
    async fn execute(&self, cwd: &Path, call: &ReadyToolCall) -> ToolExecutionResult {
        let database = Database::open(&self.db_path).expect("open for gate");
        let rows = database
            .load_tool_calls_for_session(&self.workspace, &self.session)
            .expect("load for gate");
        let recorded = rows
            .iter()
            .any(|row| row.call_id == call.call_id && row.status == "running");
        let file_absent = !self.workspace.join("gate.txt").exists();
        self.observed
            .lock()
            .expect("lock")
            .push((recorded, file_absent));
        let context = ToolContext::new(cwd.to_path_buf());
        crate::chat::tools::execute(&context, &call.name, &call.arguments_json)
    }
}

// ===== 附加：跨多次采样的 call/result 成对回放 =====

#[tokio::test]
async fn request_items_keep_call_result_pairing_across_steps() {
    let fixture = Fixture::new("items-pairing");
    let (mut state, key, persistence, _session) = streaming_state(&fixture);
    let write_args = r#"{"path":"p.txt","content":"x"}"#.to_string();
    let read_args = r#"{"path":"p.txt"}"#.to_string();
    let transport = FakeTransport::new(
        vec![
            tool_calls(&[("c1", "write", &write_args, 0)], None),
            tool_calls(&[("c2", "read", &read_args, 0)], None),
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
    assert_eq!(driven.summary.outcome, TurnOutcome::Complete);
    let requests = transport.requests();
    assert_eq!(requests.len(), 3, "两次工具采样 + 一次最终采样");
    let items = &requests[2].items;
    let calls: Vec<&str> = items
        .iter()
        .filter_map(|item| match item {
            ModelItem::AssistantToolCall { call_id, .. } => Some(call_id.as_str()),
            _ => None,
        })
        .collect();
    let results: Vec<&str> = items
        .iter()
        .filter_map(|item| match item {
            ModelItem::ToolResult { call_id, .. } => Some(call_id.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(calls, vec!["c1", "c2"]);
    assert_eq!(results, vec!["c1", "c2"]);
    // 每条结果都在其调用之后（成对，无 orphan result）。
    for (index, item) in items.iter().enumerate() {
        if let ModelItem::ToolResult { call_id, .. } = item {
            let call_at = items.iter().position(|candidate| {
                matches!(candidate, ModelItem::AssistantToolCall { call_id: at, .. } if at == call_id)
            });
            assert!(
                call_at.is_some_and(|at| at < index),
                "结果必须在其调用之后: {call_id}"
            );
        }
    }
}

/// 保证 `Duration` 导入被使用（测试驱动本身不设超时，交给 cargo test 外层）。
#[allow(dead_code)]
fn _timeout() -> Duration {
    Duration::from_secs(5)
}
