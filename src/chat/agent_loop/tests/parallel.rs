//! 只读工具并行批次的编排测试：并发峰值、ordinal 结算、取消与 begin 失败清理。

use std::sync::atomic::{AtomicUsize, Ordering};

use super::*;

const READ_ARGS: &str = r#"{"path":"a.txt"}"#;
const WRITE_ARGS: &str = r#"{"path":"a.txt","content":"x"}"#;

/// 观测并发峰值，并可选择在执行后触发取消。
struct ProbeExecutor {
    running: Arc<AtomicUsize>,
    peak: Arc<AtomicUsize>,
    executed: Arc<Mutex<Vec<String>>>,
    cancel: Option<watch::Sender<bool>>,
}

impl ProbeExecutor {
    fn new() -> Self {
        Self {
            running: Arc::new(AtomicUsize::new(0)),
            peak: Arc::new(AtomicUsize::new(0)),
            executed: Arc::new(Mutex::new(Vec::new())),
            cancel: None,
        }
    }

    fn cancelling(cancel: watch::Sender<bool>) -> Self {
        let mut executor = Self::new();
        executor.cancel = Some(cancel);
        executor
    }
}

impl ToolExecutor for ProbeExecutor {
    async fn execute(&self, _cwd: &Path, call: &ReadyToolCall) -> ToolExecutionResult {
        let now = self.running.fetch_add(1, Ordering::SeqCst) + 1;
        self.peak.fetch_max(now, Ordering::SeqCst);
        self.executed
            .lock()
            .expect("lock")
            .push(call.call_id.clone());
        // 让 join_all 在同一个任务里依次 poll 到所有 future，稳定制造重叠。
        tokio::task::yield_now().await;
        tokio::task::yield_now().await;
        self.running.fetch_sub(1, Ordering::SeqCst);
        if let Some(cancel) = &self.cancel {
            let _ = cancel.send(true);
        }
        ToolExecutionResult::Search {
            matches: Vec::new(),
            truncated: false,
            files_scanned: 0,
        }
    }
}

/// 三个只读调用必须重叠执行；Begin/Finish/上下文仍严格按 ordinal。
#[tokio::test]
async fn parallel_reads_overlap_and_keep_ordinal_order() {
    let fixture = Fixture::new("parallel-order");
    let script = Script::new(
        vec![
            tool_calls(
                &[
                    ("c0", "read", READ_ARGS, 0),
                    ("c1", "read", READ_ARGS, 1),
                    ("c2", "read", READ_ARGS, 2),
                ],
                None,
            ),
            final_response("done", None),
        ],
        StopReason::Complete,
    );
    let host = RecordingHost::new();
    let executor = ProbeExecutor::new();
    let peak = executor.peak.clone();
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
    assert_eq!(peak.load(Ordering::SeqCst), 3, "三个只读调用必须重叠执行");
    assert_eq!(
        executed.lock().expect("lock").as_slice(),
        &["c0".to_string(), "c1".to_string(), "c2".to_string()],
        "执行顺序仍取 ordinal"
    );
    assert_eq!(
        host.trace(),
        vec![
            HostTrace::Begin("c0".into()),
            HostTrace::Begin("c1".into()),
            HostTrace::Begin("c2".into()),
            HostTrace::Finish("c0".into(), false),
            HostTrace::Finish("c1".into(), false),
            HostTrace::Finish("c2".into(), false),
        ],
        "Begin 成批、Finish 按 ordinal"
    );
    let results: Vec<String> = script
        .items_of(1)
        .into_iter()
        .filter_map(|item| match item {
            ModelItem::ToolResult { call_id, .. } => Some(call_id),
            _ => None,
        })
        .collect();
    assert_eq!(
        results,
        vec!["c0".to_string(), "c1".to_string(), "c2".to_string()],
        "模型上下文里的工具结果仍按 ordinal"
    );
}

/// 单批最多 4 个：5 个只读调用拆成 4 + 1 两批。
#[tokio::test]
async fn parallel_batch_is_capped_at_four() {
    let fixture = Fixture::new("parallel-cap");
    let script = Script::new(
        vec![
            tool_calls(
                &[
                    ("c0", "read", READ_ARGS, 0),
                    ("c1", "read", READ_ARGS, 1),
                    ("c2", "read", READ_ARGS, 2),
                    ("c3", "read", READ_ARGS, 3),
                    ("c4", "read", READ_ARGS, 4),
                ],
                None,
            ),
            final_response("done", None),
        ],
        StopReason::Complete,
    );
    let host = RecordingHost::new();
    let executor = ProbeExecutor::new();
    let peak = executor.peak.clone();
    let summary = run_agent_turn(
        context(fixture.path()),
        host.clone(),
        executor,
        script.stream_one(),
        no_cancel(),
    )
    .await;

    assert_eq!(summary.outcome, TurnOutcome::Complete);
    assert_eq!(peak.load(Ordering::SeqCst), 4, "峰值并发固定为 4");
    assert_eq!(
        host.trace(),
        vec![
            HostTrace::Begin("c0".into()),
            HostTrace::Begin("c1".into()),
            HostTrace::Begin("c2".into()),
            HostTrace::Begin("c3".into()),
            HostTrace::Finish("c0".into(), false),
            HostTrace::Finish("c1".into(), false),
            HostTrace::Finish("c2".into(), false),
            HostTrace::Finish("c3".into(), false),
            HostTrace::Begin("c4".into()),
            HostTrace::Finish("c4".into(), false),
        ]
    );
}

/// 写工具是屏障：read → write → read 仍然是三段串行。
#[tokio::test]
async fn mutating_call_splits_batches() {
    let fixture = Fixture::new("parallel-split");
    let script = Script::new(
        vec![
            tool_calls(
                &[
                    ("c0", "read", READ_ARGS, 0),
                    ("c1", "write", WRITE_ARGS, 1),
                    ("c2", "read", READ_ARGS, 2),
                ],
                None,
            ),
            final_response("done", None),
        ],
        StopReason::Complete,
    );
    let host = RecordingHost::new();
    let executor = ProbeExecutor::new();
    let peak = executor.peak.clone();
    let summary = run_agent_turn(
        context(fixture.path()),
        host.clone(),
        executor,
        script.stream_one(),
        no_cancel(),
    )
    .await;

    assert_eq!(summary.outcome, TurnOutcome::Complete);
    assert_eq!(peak.load(Ordering::SeqCst), 1, "写工具必须切断批次");
    assert_eq!(
        host.trace(),
        vec![
            HostTrace::Begin("c0".into()),
            HostTrace::Finish("c0".into(), false),
            HostTrace::Begin("c1".into()),
            HostTrace::Finish("c1".into(), false),
            HostTrace::Begin("c2".into()),
            HostTrace::Finish("c2".into(), false),
        ]
    );
}

/// 批次执行中 Esc：已启动的 4 个记录真实结果，第 5 个结算为 cancelled。
#[tokio::test]
async fn cancel_during_batch_settles_started_and_cancels_unstarted() {
    let fixture = Fixture::new("parallel-cancel");
    let script = Script::new(
        vec![tool_calls(
            &[
                ("c0", "read", READ_ARGS, 0),
                ("c1", "read", READ_ARGS, 1),
                ("c2", "read", READ_ARGS, 2),
                ("c3", "read", READ_ARGS, 3),
                ("c4", "read", READ_ARGS, 4),
            ],
            None,
        )],
        StopReason::Complete,
    );
    let (cancel_sender, cancel) = watch::channel(false);
    let host = RecordingHost::new();
    let executor = ProbeExecutor::cancelling(cancel_sender);
    let executed = executor.executed.clone();
    let summary = run_agent_turn(
        context(fixture.path()),
        host.clone(),
        executor,
        script.stream_one(),
        cancel,
    )
    .await;

    assert_eq!(summary.outcome, TurnOutcome::Interrupted);
    assert_eq!(script.request_count(), 1, "取消后不再采样");
    assert_eq!(
        executed.lock().expect("lock").as_slice(),
        &[
            "c0".to_string(),
            "c1".to_string(),
            "c2".to_string(),
            "c3".to_string()
        ],
        "只有第一批真正执行"
    );
    assert_eq!(
        host.trace(),
        vec![
            HostTrace::Begin("c0".into()),
            HostTrace::Begin("c1".into()),
            HostTrace::Begin("c2".into()),
            HostTrace::Begin("c3".into()),
            HostTrace::Finish("c0".into(), false),
            HostTrace::Finish("c1".into(), false),
            HostTrace::Finish("c2".into(), false),
            HostTrace::Finish("c3".into(), false),
            HostTrace::Begin("c4".into()),
            HostTrace::Finish("c4".into(), true),
        ],
        "已执行的记录真实结果，未执行的结算为 cancelled"
    );
}

/// 批次中途 begin 失败：已 begin 的票据按 cancelled 收尾，且零执行。
#[tokio::test]
async fn begin_failure_cancels_already_begun_batch() {
    let fixture = Fixture::new("parallel-begin-fail");
    let script = Script::new(
        vec![tool_calls(
            &[
                ("c0", "read", READ_ARGS, 0),
                ("c1", "read", READ_ARGS, 1),
                ("c2", "read", READ_ARGS, 2),
            ],
            None,
        )],
        StopReason::Complete,
    );
    let host = RecordingHost::with_begin_failure_after(
        2,
        HostError::Failed {
            code: "pending failed".into(),
        },
    );
    let executor = ProbeExecutor::new();
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
            code: "pending failed".into()
        }
    );
    assert_eq!(script.request_count(), 1, "begin 失败后不再采样");
    assert!(
        executed.lock().expect("lock").is_empty(),
        "begin 失败前不得执行任何调用"
    );
    assert_eq!(
        host.trace(),
        vec![
            HostTrace::Begin("c0".into()),
            HostTrace::Begin("c1".into()),
            HostTrace::Begin("c2".into()),
            HostTrace::Finish("c0".into(), true),
            HostTrace::Finish("c1".into(), true),
        ],
        "已 begin 的票据全部按 cancelled 收尾，失败的调用没有 ticket"
    );
}

/// 完成顺序反转（c1 先返回、c0 后返回）时，结算与模型上下文仍按 ordinal。
struct ReverseExecutor {
    finished: Arc<Mutex<Vec<String>>>,
    release: Arc<std::sync::atomic::AtomicBool>,
}

impl ReverseExecutor {
    fn new() -> Self {
        Self {
            finished: Arc::new(Mutex::new(Vec::new())),
            release: Arc::new(std::sync::atomic::AtomicBool::new(false)),
        }
    }
}

impl ToolExecutor for ReverseExecutor {
    async fn execute(&self, _cwd: &Path, call: &ReadyToolCall) -> ToolExecutionResult {
        if call.call_id == "c0" {
            while !self.release.load(Ordering::SeqCst) {
                tokio::task::yield_now().await;
            }
        } else {
            self.release.store(true, Ordering::SeqCst);
        }
        self.finished
            .lock()
            .expect("lock")
            .push(call.call_id.clone());
        ToolExecutionResult::Search {
            matches: Vec::new(),
            truncated: false,
            files_scanned: 0,
        }
    }
}

#[tokio::test]
async fn reversed_completion_still_commits_in_ordinal_order() {
    let fixture = Fixture::new("parallel-reverse");
    let script = Script::new(
        vec![
            tool_calls(
                &[("c0", "read", READ_ARGS, 0), ("c1", "read", READ_ARGS, 1)],
                None,
            ),
            final_response("done", None),
        ],
        StopReason::Complete,
    );
    let host = RecordingHost::new();
    let executor = ReverseExecutor::new();
    let finished = executor.finished.clone();
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
        finished.lock().expect("lock").as_slice(),
        &["c1".to_string(), "c0".to_string()],
        "执行完成顺序必须反转"
    );
    assert_eq!(
        host.trace(),
        vec![
            HostTrace::Begin("c0".into()),
            HostTrace::Begin("c1".into()),
            HostTrace::Finish("c0".into(), false),
            HostTrace::Finish("c1".into(), false),
        ],
        "Finish 仍按 ordinal，而非完成顺序"
    );
    let results: Vec<String> = script
        .items_of(1)
        .into_iter()
        .filter_map(|item| match item {
            ModelItem::ToolResult { call_id, .. } => Some(call_id),
            _ => None,
        })
        .collect();
    assert_eq!(results, vec!["c0".to_string(), "c1".to_string()]);
}

/// 可取消入口优先（`web_search` / MCP），其余工具回落到带票据的通用执行入口。
struct CancellableFirstExecutor {
    cancellable: Arc<Mutex<Vec<String>>>,
    executed: Arc<Mutex<Vec<String>>>,
}

impl CancellableFirstExecutor {
    fn new() -> Self {
        Self {
            cancellable: Arc::new(Mutex::new(Vec::new())),
            executed: Arc::new(Mutex::new(Vec::new())),
        }
    }
}

impl ToolExecutor for CancellableFirstExecutor {
    async fn execute(&self, _cwd: &Path, call: &ReadyToolCall) -> ToolExecutionResult {
        self.executed
            .lock()
            .expect("lock")
            .push(call.call_id.clone());
        ToolExecutionResult::Search {
            matches: Vec::new(),
            truncated: false,
            files_scanned: 0,
        }
    }

    async fn execute_mcp_cancellable(
        &self,
        _cwd: &Path,
        call: &ReadyToolCall,
        _cancel: watch::Receiver<bool>,
    ) -> Option<ToolExecutionResult> {
        if call.call_id != "c0" {
            return None;
        }
        self.cancellable
            .lock()
            .expect("lock")
            .push(call.call_id.clone());
        Some(ToolExecutionResult::Search {
            matches: Vec::new(),
            truncated: false,
            files_scanned: 0,
        })
    }
}

#[tokio::test]
async fn cancellable_entry_wins_and_others_fall_back() {
    let fixture = Fixture::new("parallel-cancellable");
    let script = Script::new(
        vec![
            tool_calls(
                &[
                    ("c0", "web_search", r#"{"queries":["x"]}"#, 0),
                    ("c1", "read", READ_ARGS, 1),
                ],
                None,
            ),
            final_response("done", None),
        ],
        StopReason::Complete,
    );
    let host = RecordingHost::new();
    let executor = CancellableFirstExecutor::new();
    let cancellable = executor.cancellable.clone();
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
        cancellable.lock().expect("lock").as_slice(),
        &["c0".to_string()],
        "web_search 必须走可取消入口"
    );
    assert_eq!(
        executed.lock().expect("lock").as_slice(),
        &["c1".to_string()],
        "返回 None 的调用才回落到通用入口"
    );
    assert_eq!(
        host.trace(),
        vec![
            HostTrace::Begin("c0".into()),
            HostTrace::Begin("c1".into()),
            HostTrace::Finish("c0".into(), false),
            HostTrace::Finish("c1".into(), false),
        ]
    );
}

/// 批次结算中途 finish 失败：停止提交并把剩余票据按 cancelled 收尾。
#[tokio::test]
async fn finish_failure_cancels_remaining_batch_tickets() {
    let fixture = Fixture::new("parallel-finish-fail");
    let script = Script::new(
        vec![tool_calls(
            &[
                ("c0", "read", READ_ARGS, 0),
                ("c1", "read", READ_ARGS, 1),
                ("c2", "read", READ_ARGS, 2),
            ],
            None,
        )],
        StopReason::Complete,
    );
    let host = RecordingHost::with_finish_failure(HostError::Failed {
        code: "finish failed".into(),
    });
    let summary = run_agent_turn(
        context(fixture.path()),
        host.clone(),
        ProbeExecutor::new(),
        script.stream_one(),
        no_cancel(),
    )
    .await;

    assert_eq!(
        summary.outcome,
        TurnOutcome::Failed {
            code: "finish failed".into()
        }
    );
    assert_eq!(script.request_count(), 1, "finish 失败后不再采样");
    assert_eq!(
        host.trace(),
        vec![
            HostTrace::Begin("c0".into()),
            HostTrace::Begin("c1".into()),
            HostTrace::Begin("c2".into()),
            HostTrace::Finish("c0".into(), false),
            HostTrace::Finish("c1".into(), true),
            HostTrace::Finish("c2".into(), true),
        ]
    );
}

/// 冻结工具面之外的并行安全调用：仍走 begin/finish，但绝不进 executor。
#[tokio::test]
async fn call_outside_frozen_tool_surface_is_not_executed() {
    let fixture = Fixture::new("parallel-unavailable");
    let script = Script::new(
        vec![
            tool_calls(
                &[
                    ("c0", "read", READ_ARGS, 0),
                    ("c1", "search", r#"{"query":"needle"}"#, 1),
                ],
                None,
            ),
            final_response("done", None),
        ],
        StopReason::Complete,
    );
    let host = RecordingHost::new();
    let executor = ProbeExecutor::new();
    let executed = executor.executed.clone();
    let mut context = context(fixture.path());
    context.request.tools = std::sync::Arc::new(vec![crate::chat::tools::ToolDecl {
        name: "read".into(),
        description: "read".into(),
        parameters: serde_json::json!({"type": "object"}),
    }]);
    let summary = run_agent_turn(
        context,
        host.clone(),
        executor,
        script.stream_one(),
        no_cancel(),
    )
    .await;

    assert_eq!(summary.outcome, TurnOutcome::Complete);
    assert_eq!(
        executed.lock().expect("lock").as_slice(),
        &["c0".to_string()],
        "工具面外的调用不得执行"
    );
    assert_eq!(
        host.trace(),
        vec![
            HostTrace::Begin("c0".into()),
            HostTrace::Begin("c1".into()),
            HostTrace::Finish("c0".into(), false),
            HostTrace::Finish("c1".into(), false),
        ],
        "不可用调用按真实失败结果结算，不是 cancelled"
    );
    let text = format!("{:?}", script.items_of(1));
    assert!(
        text.contains(crate::chat::agent::UNAVAILABLE_CODE),
        "{text}"
    );
}
