//! lx agent 端到端测试：假 transport + 临时 DB + 临时工作目录。
//!
//! 每个测试都构造「一次真实点击 Enter + DB ack」后的状态（`try_send` →
//! `on_turn_committed`），再用**脚本化假 transport** 驱动同一个 agent loop；事件层用
//! 真实 `Persistence` 处理 BeginTool/FinishTool 握手。断言的核心是**可观测顺序**：
//!
//! 1. DB pending 行提交早于文件写入（执行时 DB 已有该 call 的行）；
//! 2. 工具结果真实落库、position 单调、read 看到刚写入的内容；
//! 3. DB ack 失败（pending / running / 结果）时无文件副作用或不再重写、本轮失败；
//! 4. Esc 取消与旧 generation 不执行文件 IO；
//! 5. 限额给出稳定错误码；纯文本旧路径仍是一次采样即完成；
//! 6. 崩溃恢复把 pending/running 转 interrupted 且 `/resume` 不重放。

use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use ratatui::layout::Rect;
use tokio::sync::{mpsc, watch};

use super::*;
use crate::app::state::{AppState, RequestState, ToolStatus};
use crate::chat::agent_loop::{
    FileTools, HostError, MAX_MODEL_STEPS, MAX_TOOL_CALLS, ReadyToolCall, STEP_LIMIT_CODE,
    TOOL_CALL_LIMIT_CODE, ToolExecutor, TurnOutcome, TurnSummary,
};
use crate::chat::client::StreamOutcome;
use crate::chat::events::{ModelItem, ModelStepStop, PartDelta, PartKind};
use crate::chat::tools::{ToolContext, ToolExecutionResult};
use crate::config::ai::{
    AiSettings, ModelConfig, ModelSelection, ProviderConfig, ProviderOptions, ProviderTransport,
};
use crate::db::Database;
use crate::db::chat::{ToolCallResult, ToolTerminalStatus, Usage};
use crate::db::service::Persistence;

// ===== fixture =====

/// 独立临时目录序号。
static SEQUENCE: AtomicU64 = AtomicU64::new(0);

/// 一次测试的独立临时目录（DB 与工作区都在里面）；drop 时递归删除。
struct Fixture {
    root: PathBuf,
    workspace: PathBuf,
}

impl Fixture {
    fn new(label: &str) -> Self {
        let sequence = SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let root = std::env::temp_dir().join(format!(
            "lx-agent-e2e-{}-{label}-{sequence}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&root);
        let workspace = root.join("ws");
        std::fs::create_dir_all(&workspace).expect("create workspace");
        Self { root, workspace }
    }

    fn db_path(&self) -> PathBuf {
        self.root.join("lx.db")
    }

    fn workspace(&self) -> &Path {
        &self.workspace
    }

    fn read(&self, name: &str) -> String {
        std::fs::read_to_string(self.workspace.join(name)).expect("read fixture file")
    }

    fn exists(&self, name: &str) -> bool {
        self.workspace.join(name).exists()
    }

    /// 打开（并信任当前工作区的）临时数据库。
    fn database(&self) -> Database {
        let database = Database::open(&self.db_path()).expect("database opens");
        database
            .trust_workspace(&self.workspace, "workspace")
            .expect("trust workspace");
        database
    }

    /// 重新打开同一库（模拟另一个连接/重启）。
    fn reopen(&self) -> Database {
        Database::open(&self.db_path()).expect("reopen database")
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

/// 构造带单个可用 provider/model 的模型快照（与 app 层测试同口径）。
fn models_with() -> crate::app::state::ModelsSnapshot {
    let mut settings = AiSettings::empty();
    settings.enabled_providers = vec!["p".into()];
    settings.providers.insert(
        "p".into(),
        ProviderConfig {
            id: "p".into(),
            kind: ProviderTransport::OpenaiCompatible,
            name: "P".into(),
            options: ProviderOptions {
                api_key: Some("test-key".into()),
                base_url: "http://127.0.0.1:1/v1".into(),
            },
            models: std::iter::once((
                "m".into(),
                ModelConfig {
                    id: "m".into(),
                    name: "M".into(),
                    transport: None,
                    limit: Some(crate::config::ai::ModelLimit {
                        context: 100_000,
                        output: 4_096,
                    }),
                    modalities: None,
                    variants: std::collections::BTreeMap::new(),
                    variant: None,
                    pricing: None,
                    extra: serde_json::Map::new(),
                },
            ))
            .collect(),
            extra: serde_json::Map::new(),
        },
    );
    settings.default_model = Some(ModelSelection {
        provider: "p".into(),
        model: "m".into(),
        variant: None,
    });
    crate::app::state::ModelsSnapshot { settings }
}

/// 走完「按下 Enter + DB ack」：状态进入 Streaming，并登记事件层要求的冻结工作区。
///
/// 返回（状态，request key，持久化服务，DB 生成的会话 id）。
fn streaming_state(fixture: &Fixture) -> (AppState, RequestKey, Persistence, String) {
    let database = fixture.database();
    let mut state = AppState::demo();
    state.active_workspace_mut().cwd = Some(fixture.workspace().to_path_buf());
    state.models = models_with();
    let pane = state.active_tab().layout.focus();
    {
        let target = state.pane_mut_anywhere(pane).expect("pane");
        target.lx.entry = crate::app::state::LxEntry::Native;
        target.lx.input = "write then read".into();
    }
    assert!(update::try_send(&mut state, pane), "发送受理");
    let generation = match &state.pane_anywhere(pane).expect("pane").lx.chat.request {
        RequestState::Committing { generation, .. } => *generation,
        other => panic!("expected Committing, got {other:?}"),
    };
    // 用真实 DB 完成 begin_turn（user + streaming assistant）。
    let committed = database
        .begin_turn(
            fixture.workspace(),
            None,
            "write then read",
            &crate::db::chat::ModelSnapshot {
                provider: "p".into(),
                model: "m".into(),
                variant: None,
            },
        )
        .expect("begin_turn");
    assert!(update::on_turn_committed(
        &mut state, pane, generation, &committed
    ));
    let key = RequestKey {
        pane,
        session: committed.session_id.clone(),
        generation,
    };
    // 丢弃 StartChatStream 效应：本测试用假 transport 自己驱动 loop。
    state.effects.clear();
    let session = committed.session_id.clone();
    (state, key, Persistence::with_database(database), session)
}

// ===== 假 transport =====

/// 脚本化 transport：每次采样消费一段 delta；记录收到的请求与采样次数。
#[derive(Clone)]
struct FakeTransport {
    steps: Arc<Mutex<VecDeque<Vec<ChatDelta>>>>,
    stop: StopReason,
    requests: Arc<Mutex<Vec<ChatRequest>>>,
    /// 增量即时回投通道（由 [`TransportHarness::attach`] 注入）。
    sink: Arc<Mutex<Option<mpsc::UnboundedSender<AppEvent>>>>,
    key: Arc<Mutex<Option<RequestKey>>>,
    calls: Arc<AtomicU64>,
}

impl FakeTransport {
    fn new(steps: Vec<Vec<ChatDelta>>, stop: StopReason) -> Self {
        Self {
            steps: Arc::new(Mutex::new(steps.into())),
            stop,
            requests: Arc::new(Mutex::new(Vec::new())),
            sink: Arc::new(Mutex::new(None)),
            key: Arc::new(Mutex::new(None)),
            calls: Arc::new(AtomicU64::new(0)),
        }
    }

    fn attach(&self, sender: mpsc::UnboundedSender<AppEvent>, key: RequestKey) {
        *self.sink.lock().expect("lock") = Some(sender);
        *self.key.lock().expect("lock") = Some(key);
    }

    fn calls(&self) -> u64 {
        self.calls.load(Ordering::SeqCst)
    }

    fn requests(&self) -> Vec<ChatRequest> {
        self.requests.lock().expect("lock").clone()
    }

    /// 可注入 `drive_with` 的 stream 前端：模拟真实的「增量即时回投」。
    fn stream_one(
        &self,
    ) -> impl Fn(ChatRequest, watch::Receiver<bool>) -> std::future::Ready<StreamOutcome> {
        let script = self.clone();
        move |request, _cancel| {
            script.calls.fetch_add(1, Ordering::SeqCst);
            script.requests.lock().expect("lock").push(request);
            let mut deltas = script
                .steps
                .lock()
                .expect("lock")
                .pop_front()
                .unwrap_or_default();
            let stop = script.stop.clone();
            deltas.push(ChatDelta::Finished {
                reason: stop.clone(),
            });
            let incremental: Vec<ChatDelta> = deltas
                .iter()
                .filter(|delta| delta.is_incremental())
                .cloned()
                .collect();
            if !incremental.is_empty()
                && let (Some(sender), Some(key)) = (
                    script.sink.lock().expect("lock").clone(),
                    script.key.lock().expect("lock").clone(),
                )
            {
                let _ = sender.send(AppEvent::ChatNet(ChatNetEvent::Delta {
                    key,
                    deltas: incremental,
                }));
            }
            std::future::ready(StreamOutcome { deltas, stop })
        }
    }
}

// ===== 驱动 =====

/// 事件层握手轨迹（可观测证据：Begin 早于 Finish，且严格交替）。
#[derive(Debug, Clone, PartialEq, Eq)]
enum Trace {
    Begin(String),
    Finish(String, ToolTerminalStatus),
}

/// 驱动一轮的结果。
struct Driven {
    summary: TurnSummary,
    /// 收到的整轮终态（恰好一条）。
    finished: Option<(StopReason, Usage)>,
    /// begin/finish 顺序。
    trace: Vec<Trace>,
    /// 实际执行过文件工具的 call ID 顺序。
    executed: Vec<String>,
    /// 结束时仍留在状态里的未回执 pending 数（应为 0）。
    unflushed: usize,
}

/// 一次 pending → running 之间的插桩点（注入 DB 故障用）。
type Interleave = Arc<dyn Fn(&Fixture, &RequestKey) + Send + Sync>;

/// 事件层握手的插桩钩子：分别在 BeginTool 回执前后、FinishTool 之前调用。
#[derive(Default, Clone)]
struct Hooks {
    /// BeginTool 已处理（pending 已提交、状态已 pending）但 ticket 尚未回执时。
    before_running: Option<Interleave>,
    /// FinishTool 即将执行落库前（行仍为 running）。
    before_finish: Option<Interleave>,
}

/// 驱动一轮：假 transport + 真实 `Persistence`，手工泵事件层握手。
///
/// 事件循环顺序与生产一致：`BeginTool` → DB pending → 状态 pending → 安排帧
/// →（本函数立即模拟该帧绘制）flush pending → DB running → 回执 ticket。
// 测试驱动需要同时传入状态、事件层、执行器与插桩点；拆成结构体只会增加噪声。
#[allow(clippy::too_many_arguments)]
async fn drive_with_hook<E>(
    fixture: &Fixture,
    state: &mut AppState,
    key: &RequestKey,
    transport: &FakeTransport,
    persistence: &Persistence,
    streams: &mut ChatStreams,
    cancel: watch::Receiver<bool>,
    executor: E,
    hooks: Hooks,
) -> Driven
where
    E: ToolExecutor + 'static,
{
    drive_request(
        fixture,
        state,
        key,
        transport,
        persistence,
        streams,
        cancel,
        executor,
        hooks,
        ChatRequest {
            turns: Vec::new(),
            items: Vec::new(),
            model: crate::db::chat::ModelSnapshot {
                provider: "p".into(),
                model: "m".into(),
                variant: None,
            },
            variant_params: None,
        },
    )
    .await
}

/// 用**显式装配好的请求**驱动一轮（`/resume` 后续轮、历史回放测试用）。
#[allow(clippy::too_many_arguments)]
async fn drive_request<E>(
    fixture: &Fixture,
    state: &mut AppState,
    key: &RequestKey,
    transport: &FakeTransport,
    persistence: &Persistence,
    streams: &mut ChatStreams,
    cancel: watch::Receiver<bool>,
    executor: E,
    hooks: Hooks,
    request: ChatRequest,
) -> Driven
where
    E: ToolExecutor + 'static,
{
    let (sender, mut receiver) = mpsc::unbounded_channel::<AppEvent>();
    transport.attach(sender.clone(), key.clone());
    // 事件层冻结工作区（生产由 `start_stream` 写入）。
    streams
        .workspaces
        .insert(key.clone(), fixture.workspace().to_path_buf());
    // 窗格可见：BeginTool 的回执推迟到「该帧绘制后」，从而可插桩 pending→running。
    streams.set_visible_panes(&[(key.pane, Rect::new(0, 0, 80, 24))]);
    let trace = Arc::new(Mutex::new(Vec::<Trace>::new()));
    let executed = Arc::new(Mutex::new(Vec::<String>::new()));
    let executor = TracingExecutor {
        inner: executor,
        trace: trace.clone(),
        executed: executed.clone(),
    };
    let prepared = prepare_turn(request, fixture.workspace().to_path_buf());
    let mut driver = Box::pin(host::drive_with(
        prepared.context,
        host::EventHost {
            key: key.clone(),
            sender: sender.clone(),
        },
        executor,
        transport.stream_one(),
        cancel,
    ));
    let mut finished: Option<(StopReason, Usage)> = None;
    let summary = loop {
        tokio::select! {
            summary = &mut driver => break summary,
            Some(event) = receiver.recv() => {
                pump(fixture, state, streams, persistence, key, event, &trace, &hooks, &mut finished);
            }
        }
    };
    // 先排空在途增量（真实事件循环里 delta 早于终态入队），再投整轮终态。
    while let Ok(event) = receiver.try_recv() {
        pump(
            fixture,
            state,
            streams,
            persistence,
            key,
            event,
            &trace,
            &hooks,
            &mut finished,
        );
    }
    // 模拟生产 `report_turn`：把整轮终态投回事件循环（`on_stream_finished`）。
    let terminal = AppEvent::ChatNet(ChatNetEvent::Finished {
        key: key.clone(),
        reason: summary.stop_reason(),
        usage: summary.usage,
    });
    pump(
        fixture,
        state,
        streams,
        persistence,
        key,
        terminal,
        &trace,
        &hooks,
        &mut finished,
    );
    Driven {
        summary,
        finished,
        trace: trace.lock().expect("lock").clone(),
        executed: executed.lock().expect("lock").clone(),
        unflushed: streams.pending_begins.len(),
    }
}

/// 简化入口：默认 `FileTools` 执行器、无插桩。
async fn drive(
    fixture: &Fixture,
    state: &mut AppState,
    key: &RequestKey,
    transport: &FakeTransport,
    persistence: &Persistence,
    streams: &mut ChatStreams,
    cancel: watch::Receiver<bool>,
) -> Driven {
    drive_with_hook(
        fixture,
        state,
        key,
        transport,
        persistence,
        streams,
        cancel,
        FileTools,
        Hooks::default(),
    )
    .await
}

/// 处理一条应用事件（与生产事件循环同序）。
#[allow(clippy::too_many_arguments)]
fn pump(
    fixture: &Fixture,
    state: &mut AppState,
    streams: &mut ChatStreams,
    persistence: &Persistence,
    key: &RequestKey,
    event: AppEvent,
    trace: &Arc<Mutex<Vec<Trace>>>,
    hooks: &Hooks,
    finished: &mut Option<(StopReason, Usage)>,
) {
    match event {
        AppEvent::AgentHost(event) => {
            let is_begin = matches!(event, AgentHostEvent::BeginTool { .. });
            match &event {
                AgentHostEvent::BeginTool { call, .. } => trace
                    .lock()
                    .expect("lock")
                    .push(Trace::Begin(call.call_id.clone())),
                AgentHostEvent::FinishTool {
                    ticket, outcome, ..
                } => {
                    let status = match outcome {
                        ToolOutcome::Cancelled => ToolTerminalStatus::Cancelled,
                        ToolOutcome::Executed(execution) if execution.is_failure() => {
                            ToolTerminalStatus::Failed
                        }
                        ToolOutcome::Executed(_) => ToolTerminalStatus::Completed,
                    };
                    trace
                        .lock()
                        .expect("lock")
                        .push(Trace::Finish(ticket.call_id.clone(), status));
                }
            }
            if !is_begin && let Some(hook) = hooks.before_finish.as_ref() {
                hook(fixture, key);
            }
            handle_agent_host_event(state, streams, persistence, event);
            if is_begin {
                // pending → running 之间的插桩点（注入 DB 故障）。
                if let Some(hook) = hooks.before_running.as_ref() {
                    hook(fixture, key);
                }
                // 模拟「该帧已绘制」：推进 pending → running 并回执 ticket。
                flush_pending_tool_begins(state, streams, persistence);
            }
        }
        AppEvent::ChatNet(ChatNetEvent::Delta { key, deltas }) => {
            handle_net_event(state, streams, ChatNetEvent::Delta { key, deltas });
        }
        AppEvent::ChatNet(ChatNetEvent::Finished { key, reason, usage }) => {
            *finished = Some((reason.clone(), usage));
            handle_net_event(
                state,
                streams,
                ChatNetEvent::Finished { key, reason, usage },
            );
        }
        _ => {}
    }
}

/// 记录 begin/finish 与执行顺序的执行器包装。
struct TracingExecutor<E> {
    inner: E,
    trace: Arc<Mutex<Vec<Trace>>>,
    executed: Arc<Mutex<Vec<String>>>,
}

impl<E: ToolExecutor> ToolExecutor for TracingExecutor<E> {
    async fn execute(&self, cwd: &Path, call: &ReadyToolCall) -> ToolExecutionResult {
        self.executed
            .lock()
            .expect("lock")
            .push(call.call_id.clone());
        let _ = &self.trace;
        self.inner.execute(cwd, call).await
    }
}

// ===== 响应构造 =====

/// 一次要求工具的响应（Started + Ended + 可选 usage + ResponseFinished）。
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

/// 无工具的最终响应（含正文与可选用量）。
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

/// 未取消通道（sender 泄漏保持 false）。
fn no_cancel() -> watch::Receiver<bool> {
    let (sender, receiver) = watch::channel(false);
    Box::leak(Box::new(sender));
    receiver
}

// ===== 读取辅助 =====

/// 从（重新打开的）DB 读回该会话的工具行（按 message sequence + position 排序）。
fn tool_rows(fixture: &Fixture, session: &str) -> Vec<crate::db::chat::ToolCallRecord> {
    fixture
        .reopen()
        .load_tool_calls_for_session(fixture.workspace(), session)
        .expect("load tool calls")
}

/// 该会话最后一条 assistant 消息的 parts。
fn assistant_parts(fixture: &Fixture, session: &str) -> Vec<crate::db::chat::ChatPart> {
    fixture
        .reopen()
        .load_chat_session(fixture.workspace(), session)
        .expect("load session")
        .expect("session exists")
        .into_iter()
        .rfind(|message| message.role == "assistant")
        .map(|message| message.parts)
        .unwrap_or_default()
}

fn pane_of<'a>(state: &'a AppState, key: &RequestKey) -> &'a crate::app::state::Pane {
    state.pane_anywhere(key.pane).expect("pane exists")
}

fn assistant_message<'a>(
    state: &'a AppState,
    key: &RequestKey,
) -> &'a crate::app::state::ChatMessageView {
    pane_of(state, key)
        .lx
        .chat
        .messages
        .last()
        .expect("assistant message")
}

// ===== 摘要字段映射 =====

/// `chat::tools::ToolSummary` → app 层摘要的映射必须逐字段透传（单向依赖的唯一接缝）。
///
/// 漏字段是 P1-1 那类「重进会话丢详情」的入口：新增摘要字段时本测试会失败，
/// 与「实时 == 重载」的表驱动测试一起把两条路径锁死。
#[test]
fn view_summary_maps_every_field() {
    let chat = crate::chat::tools::ToolSummary {
        path: Some("src/a.rs".into()),
        range: Some((12, 40)),
        added: Some(3),
        removed: Some(1),
        matches: Some(2),
        pattern: Some("fn render".into()),
        read_lines: Some(29),
        total_lines: Some(1000),
        truncated: true,
        partial: true,
    };
    let mapped = view_summary(&chat);
    assert_eq!(mapped.path.as_deref(), Some("src/a.rs"));
    assert_eq!(mapped.range, Some((12, 40)));
    assert_eq!(mapped.added, Some(3));
    assert_eq!(mapped.removed, Some(1));
    assert_eq!(mapped.matches, Some(2));
    assert_eq!(mapped.pattern.as_deref(), Some("fn render"));
    assert_eq!(mapped.read_lines, Some(29), "实际读到的行数必须透传");
    assert_eq!(mapped.total_lines, Some(1000));
    assert!(mapped.truncated);
    assert!(mapped.partial, "partial 必须透传");
    assert!(!mapped.is_empty(), "全字段样本不得被判成空摘要");
}

mod ack;
mod flow;
mod limits;
mod resume;
