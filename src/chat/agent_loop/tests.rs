//! Agent loop 单元测试：脚本化假 transport + 可控 host / executor。
//!
//! 这里证明的是**编排不变量**：ordinal 串行顺序、DB ack 门槛、取消语义、限额与 usage
//! 累加。端到端（真 DB + 真文件 + 事件层握手）由 `src/event/chat/tests.rs` 覆盖。

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use tokio::sync::watch;

use super::*;
use crate::chat::client::StreamOutcome;
use crate::chat::events::{PartDelta, PartKind};
use crate::chat::tools::ToolErrorCode;

/// 一次 host 往返的记录（可观测的执行顺序）。
#[derive(Debug, Clone, PartialEq, Eq)]
enum HostTrace {
    Begin(String),
    /// call ID + 是否为「未执行就取消」结算。
    Finish(String, bool),
}

/// 可控 host：记录每次请求，并按脚本注入失败。
#[derive(Clone, Default)]
struct RecordingHost {
    trace: Arc<Mutex<Vec<HostTrace>>>,
    /// 依次消费的 begin 注入结果；用尽后一律成功。
    begin_failures: Arc<Mutex<Vec<HostError>>>,
    /// 依次消费的 finish 注入结果；用尽后一律成功。
    finish_failures: Arc<Mutex<Vec<HostError>>>,
    /// 已发放的 ticket（验证 begin 的 ticket 被原样交给 finish）。
    issued: Arc<Mutex<Vec<ToolCallTicket>>>,
}

impl RecordingHost {
    fn new() -> Self {
        Self::default()
    }

    fn with_begin_failure(error: HostError) -> Self {
        let host = Self::new();
        host.begin_failures.lock().expect("lock").push(error);
        host
    }

    fn with_finish_failure(error: HostError) -> Self {
        let host = Self::new();
        host.finish_failures.lock().expect("lock").push(error);
        host
    }

    fn trace(&self) -> Vec<HostTrace> {
        self.trace.lock().expect("lock").clone()
    }
}

impl AgentHost for RecordingHost {
    fn send(&mut self, request: AgentHostRequest) {
        match request {
            AgentHostRequest::UserShellUpdates { reply } => {
                let _ = reply.send(Vec::new());
            }
            AgentHostRequest::BeginTool { call, reply } => {
                self.trace
                    .lock()
                    .expect("lock")
                    .push(HostTrace::Begin(call.call_id.clone()));
                let injected = {
                    let mut failures = self.begin_failures.lock().expect("lock");
                    if failures.is_empty() {
                        None
                    } else {
                        Some(failures.remove(0))
                    }
                };
                if let Some(error) = injected {
                    let _ = reply.send(Err(error));
                    return;
                }
                let ticket = ToolCallTicket {
                    session_id: "s".into(),
                    message_id: "m".into(),
                    call_id: call.call_id.clone(),
                    position: self.issued.lock().expect("lock").len() as i64,
                    started_at_ms: 0,
                };
                self.issued.lock().expect("lock").push(ticket.clone());
                // 真实事件层在回执前完成 DB pending + running + 状态推进；此处同步模拟。
                let _ = reply.send(Ok(ticket));
            }
            AgentHostRequest::FinishTool {
                ticket,
                outcome,
                reply,
            } => {
                // ticket 必须来自 begin（同一 call ID）。
                let issued = self.issued.lock().expect("lock").clone();
                let known = issued.iter().any(|known| known.call_id == ticket.call_id);
                self.trace.lock().expect("lock").push(HostTrace::Finish(
                    ticket.call_id.clone(),
                    outcome == ToolOutcome::Cancelled,
                ));
                if !known {
                    let _ = reply.send(Err(HostError::Failed {
                        code: "missing ticket".into(),
                    }));
                    return;
                }
                let injected = {
                    let mut failures = self.finish_failures.lock().expect("lock");
                    if failures.is_empty() {
                        None
                    } else {
                        Some(failures.remove(0))
                    }
                };
                let _ = reply.send(match injected {
                    Some(error) => Err(error),
                    None => Ok(()),
                });
            }
        }
    }
}

/// 记录执行顺序并真正执行文件工具（保证“前一结果对后一调用可见”可断言）。
struct RecordingExecutor {
    executed: Arc<Mutex<Vec<String>>>,
    /// 可选：执行后触发取消（模拟 Esc 落在两个工具之间）。
    cancel: Option<watch::Sender<bool>>,
}

impl RecordingExecutor {
    fn new() -> Self {
        Self {
            executed: Arc::new(Mutex::new(Vec::new())),
            cancel: None,
        }
    }

    fn cancelling(sender: watch::Sender<bool>) -> Self {
        Self {
            executed: Arc::new(Mutex::new(Vec::new())),
            cancel: Some(sender),
        }
    }
}

impl ToolExecutor for RecordingExecutor {
    async fn execute(&self, cwd: &Path, call: &ReadyToolCall) -> ToolExecutionResult {
        self.executed
            .lock()
            .expect("lock")
            .push(call.call_id.clone());
        let context = ToolContext::new(cwd.to_path_buf());
        let result = tools::execute(&context, &call.name, &call.arguments_json);
        if let Some(cancel) = &self.cancel {
            let _ = cancel.send(true);
        }
        result
    }
}

/// 脚本化假 transport：每次调用消费一段预置 delta；记录收到的请求。
struct Script {
    steps: Mutex<Vec<Vec<ChatDelta>>>,
    stop: StopReason,
    requests: Arc<Mutex<Vec<ChatRequest>>>,
}

impl Script {
    fn new(steps: Vec<Vec<ChatDelta>>, stop: StopReason) -> Arc<Self> {
        Arc::new(Self {
            steps: Mutex::new(steps),
            stop,
            requests: Arc::new(Mutex::new(Vec::new())),
        })
    }

    /// 返回可注入 `run_agent_turn` 的 stream 闭包（脚本用尽后复用最后一段）。
    fn stream_one(
        self: &Arc<Self>,
    ) -> impl Fn(ChatRequest, watch::Receiver<bool>) -> std::future::Ready<StreamOutcome> {
        let script = self.clone();
        move |request, _cancel| {
            script.requests.lock().expect("lock").push(request);
            let mut steps = script.steps.lock().expect("lock");
            let mut deltas = if steps.len() > 1 {
                steps.remove(0)
            } else {
                steps.first().cloned().unwrap_or_default()
            };
            let stop = script.stop.clone();
            deltas.push(ChatDelta::Finished {
                reason: stop.clone(),
            });
            std::future::ready(StreamOutcome { deltas, stop })
        }
    }

    fn request_count(&self) -> usize {
        self.requests.lock().expect("lock").len()
    }

    fn items_of(&self, index: usize) -> Vec<ModelItem> {
        self.requests
            .lock()
            .expect("lock")
            .get(index)
            .map(|request| request.items.clone())
            .unwrap_or_default()
    }
}

/// 一次工具调用的 delta 序列（Started + Ended + 可选 usage + ResponseFinished）。
fn tool_calls(calls: &[(&str, &str, &str, usize)], usage: Option<(i64, i64)>) -> Vec<ChatDelta> {
    let mut deltas = Vec::new();
    for (call_id, name, arguments, ordinal) in calls {
        deltas.push(ChatDelta::ToolInputStarted {
            call_id: (*call_id).into(),
            name: (*name).into(),
            ordinal: *ordinal,
        });
        deltas.push(ChatDelta::ToolInputEnded {
            call_id: (*call_id).into(),
            arguments_json: (*arguments).into(),
        });
    }
    if let Some((input, output)) = usage {
        deltas.push(ChatDelta::Usage(Usage {
            input: Some(input),
            output: Some(output),
            cache_read: None,
            cache_write: None,
        }));
    }
    deltas.push(ChatDelta::ResponseFinished {
        stop: ModelStepStop::ToolCalls,
    });
    deltas
}

/// 无工具的最终响应。
fn final_response(text: &str, usage: Option<(i64, i64)>) -> Vec<ChatDelta> {
    let mut deltas = vec![ChatDelta::Part(PartDelta {
        kind: PartKind::Text,
        text: text.into(),
        metadata: None,
    })];
    if let Some((input, output)) = usage {
        deltas.push(ChatDelta::Usage(Usage {
            input: Some(input),
            output: Some(output),
            cache_read: None,
            cache_write: None,
        }));
    }
    deltas.push(ChatDelta::ResponseFinished {
        stop: ModelStepStop::Final,
    });
    deltas
}

/// 独立临时工作目录（drop 时递归删除）。
struct Fixture {
    root: PathBuf,
}

impl Fixture {
    fn new(label: &str) -> Self {
        use std::sync::atomic::{AtomicU64, Ordering};
        static SEQUENCE: AtomicU64 = AtomicU64::new(0);
        let sequence = SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let root = std::env::temp_dir().join(format!(
            "lx-agent-loop-{}-{label}-{sequence}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).expect("create fixture root");
        Self { root }
    }

    fn path(&self) -> &Path {
        &self.root
    }

    fn read(&self, name: &str) -> String {
        std::fs::read_to_string(self.root.join(name)).expect("read fixture file")
    }

    fn exists(&self, name: &str) -> bool {
        self.root.join(name).exists()
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

/// 空上下文（编排断言不依赖 turns/model）。
fn context(cwd: &Path) -> ModelContext {
    ModelContext::new(
        ChatRequest {
            turns: Vec::new(),
            items: Vec::new(),
            model: crate::db::chat::ModelSnapshot {
                provider: "example".into(),
                model: "example".into(),
                variant: None,
            },
            variant_params: None,
        },
        cwd.to_path_buf(),
    )
}

/// 未取消的通道：loop 只读 `borrow()`，sender 泄漏到进程结束以保持 false。
fn no_cancel() -> watch::Receiver<bool> {
    let (sender, receiver) = watch::channel(false);
    Box::leak(Box::new(sender));
    receiver
}

/// write(ordinal 1) + read(ordinal 0) 的响应里，执行顺序必须是 ordinal 顺序。
#[tokio::test]
async fn executes_tool_calls_in_ordinal_order_not_arrival_order() {
    let fixture = Fixture::new("ordinal");
    let write_args = r#"{"path":"note.txt","content":"hello\n"}"#.to_string();
    let read_args = r#"{"path":"note.txt"}"#.to_string();
    let script = Script::new(
        vec![
            // 故意让 read（ordinal 1）先到达，write（ordinal 0）后到达。
            tool_calls(
                &[
                    ("call_read", "read", &read_args, 1),
                    ("call_write", "write", &write_args, 0),
                ],
                Some((10, 2)),
            ),
            final_response("done", Some((4, 3))),
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
        &["call_write".to_string(), "call_read".to_string()],
        "执行顺序取供应商 ordinal，不是到达顺序"
    );
    // 每次都夹在 begin/finish 之间（DB ack 门槛），且严格串行。
    assert_eq!(
        host.trace(),
        vec![
            HostTrace::Begin("call_write".into()),
            HostTrace::Finish("call_write".into(), false),
            HostTrace::Begin("call_read".into()),
            HostTrace::Finish("call_read".into(), false),
        ]
    );
    assert_eq!(fixture.read("note.txt"), "hello\n");
}

/// 结果成对进入下一次采样：write 的调用 + read 的调用 + 两条结果按 ordinal 顺序。
#[tokio::test]
async fn tool_calls_and_results_are_paired_in_next_request() {
    let fixture = Fixture::new("feed-forward");
    let write_args = r#"{"path":"a.txt","content":"x\n"}"#.to_string();
    let read_args = r#"{"path":"a.txt"}"#.to_string();
    let script = Script::new(
        vec![
            tool_calls(
                &[
                    ("c1", "write", &write_args, 0),
                    ("c2", "read", &read_args, 1),
                ],
                None,
            ),
            final_response("done", None),
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
    // 第二个请求里 read 的结果是 write 刚写下的内容（前一调用对后一调用可见）。
    let items = script.items_of(1);
    assert!(matches!(
        items.first(),
        Some(ModelItem::AssistantToolCall { call_id, .. }) if call_id == "c1"
    ));
    assert!(matches!(
        items.get(1),
        Some(ModelItem::AssistantToolCall { call_id, .. }) if call_id == "c2"
    ));
    let read_result = items.iter().find_map(|item| match item {
        ModelItem::ToolResult {
            call_id, content, ..
        } if call_id == "c2" => Some(content.clone()),
        _ => None,
    });
    let read_result = read_result.expect("read result present");
    assert!(
        read_result.contains("x"),
        "read 必须看到刚写入的内容: {read_result}"
    );
    assert!(
        items.iter().any(|item| matches!(
            item,
            ModelItem::ToolResult { call_id, is_error: false, .. } if call_id == "c1"
        )),
        "两个结果都回传模型"
    );
}

/// pending 入库失败：不执行文件 IO，整轮失败。
#[tokio::test]
async fn begin_failure_skips_file_io_and_fails_turn() {
    let fixture = Fixture::new("begin-fail");
    let write_args = r#"{"path":"never.txt","content":"x"}"#.to_string();
    let script = Script::new(
        vec![tool_calls(&[("c1", "write", &write_args, 0)], None)],
        StopReason::Complete,
    );
    let host = RecordingHost::with_begin_failure(HostError::Failed {
        code: "pending failed".into(),
    });
    let executor = RecordingExecutor::new();
    let executed = executor.executed.clone();
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
            code: "pending failed".into()
        }
    );
    assert!(
        executed.lock().expect("lock").is_empty(),
        "pending 失败后不得执行文件 IO"
    );
    assert!(!fixture.exists("never.txt"), "无文件副作用");
}

/// 结果落库失败：不重试写入，整轮失败（文件可能已变化）。
#[tokio::test]
async fn finish_failure_does_not_retry_the_write() {
    let fixture = Fixture::new("finish-fail");
    let write_args = r#"{"path":"once.txt","content":"v1"}"#.to_string();
    let script = Script::new(
        vec![tool_calls(&[("c1", "write", &write_args, 0)], None)],
        StopReason::Complete,
    );
    let host = RecordingHost::with_finish_failure(HostError::Failed {
        code: "result persistence failed".into(),
    });
    let executor = RecordingExecutor::new();
    let executed = executor.executed.clone();
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
            code: "result persistence failed".into()
        }
    );
    assert_eq!(executed.lock().expect("lock").len(), 1, "只执行一次");
    assert_eq!(fixture.read("once.txt"), "v1", "文件已写入且不重复写");
}

/// 参数不是 JSON 对象：不进入 pending、不执行、整轮失败。
#[tokio::test]
async fn invalid_arguments_never_reach_the_host() {
    let fixture = Fixture::new("invalid-args");
    let deltas = vec![
        ChatDelta::ToolInputStarted {
            call_id: "c1".into(),
            name: "write".into(),
            ordinal: 0,
        },
        ChatDelta::ToolInputEnded {
            call_id: "c1".into(),
            arguments_json: "[1,2,3]".into(),
        },
        ChatDelta::ResponseFinished {
            stop: ModelStepStop::ToolCalls,
        },
        ChatDelta::Finished {
            reason: StopReason::Complete,
        },
    ];
    let script = Script::new(vec![deltas], StopReason::Complete);
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
            code: TOOL_ARGUMENTS_INVALID_CODE.into()
        }
    );
    assert!(host.trace().is_empty(), "非法参数不进入 pending");
    assert!(executed.lock().expect("lock").is_empty());
}

/// 工具输入未闭合：整轮失败，不猜执行。
#[tokio::test]
async fn unclosed_tool_input_fails_without_execution() {
    let fixture = Fixture::new("unclosed");
    let deltas = vec![
        ChatDelta::ToolInputStarted {
            call_id: "c1".into(),
            name: "read".into(),
            ordinal: 0,
        },
        ChatDelta::ToolInputDelta {
            call_id: "c1".into(),
            chunk: "{\"path\":".into(),
        },
        ChatDelta::ResponseFinished {
            stop: ModelStepStop::ToolCalls,
        },
        ChatDelta::Finished {
            reason: StopReason::Complete,
        },
    ];
    let script = Script::new(vec![deltas], StopReason::Complete);
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
    assert_eq!(
        summary.outcome,
        TurnOutcome::Failed {
            code: TOOL_INPUT_TRUNCATED_CODE.into()
        }
    );
    assert!(host.trace().is_empty());
}

mod context;
mod limits;
