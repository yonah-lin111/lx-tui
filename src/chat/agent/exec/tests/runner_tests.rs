//! `exec` 运行器端到端测试：脚本化模型流驱动真实数据库、真实工具与子 agent。
//!
//! 不触网：模型流由 [`ScriptedFactory`] 按请求用途（主轮 / 子 agent 轮）回放脚本。
//! 其余全部走生产路径——`assemble_exec_turn` 装配、`run_exec_turn` 事件循环、
//! `AgentTools` 真实文件 IO 与 SQLite 落库。

use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use tokio::sync::watch;

use super::super::cli::{ExecModelTarget, ResumeArgs};
use super::super::runner::{BoxStreamFuture, ExecStreamFactory};
use super::super::{
    ExecTurnConfig, HumanOutputProcessor, JsonlOutputProcessor, RealStreamFactory,
    assemble_exec_turn_with_snapshot, resolve_resume_session, run_exec_turn,
};
use crate::chat::agent::EXEC_ASK_UNSUPPORTED_CODE;
use crate::chat::agent_loop::{ModelContext, TurnOutcome};
use crate::chat::client::StreamOutcome;
use crate::chat::events::{
    ChatDelta, ChatRequest, ChatTurn, ModelItem, ModelStepStop, PartDelta, PartKind,
    RequestPurpose, StopReason,
};
use crate::db::Database;

// ===== 夹具 =====

/// 每个测试独立的临时目录（仓库未引入 `tempfile` 依赖）。
struct TestDir {
    path: PathBuf,
}

impl TestDir {
    fn new(label: &str) -> Self {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|value| value.as_nanos())
            .unwrap_or_default();
        let path = std::env::temp_dir().join(format!(
            "lx-exec-runner-{}-{label}-{nanos}",
            std::process::id()
        ));
        std::fs::create_dir_all(&path).expect("create temp dir");
        Self { path }
    }

    fn join(&self, name: &str) -> PathBuf {
        self.path.join(name)
    }
}

impl Drop for TestDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

fn model_target() -> ExecModelTarget {
    ExecModelTarget {
        provider: "test-provider".to_string(),
        model: "test-model".to_string(),
        variant: None,
        display_name: "Test Model".to_string(),
        transport: "openai".to_string(),
        base_url: "http://127.0.0.1:1".to_string(),
        api_key: "test-key".to_string(),
        context_limit: Some(200_000),
        output_limit: Some(8_192),
        variant_params: None,
        idle_timeout_ms: 1_000,
        supports_image_input: false,
    }
}

/// 一次脚本化模型响应。
enum Step {
    Response {
        deltas: Vec<ChatDelta>,
        stop: StopReason,
    },
    /// 只等取消（`Ctrl+C` 中断测试）。
    Hang,
}

fn text_step(text: &str) -> Step {
    Step::Response {
        deltas: vec![
            ChatDelta::Part(PartDelta {
                kind: PartKind::Text,
                text: text.to_string(),
                metadata: None,
            }),
            ChatDelta::ResponseFinished {
                stop: ModelStepStop::Final,
            },
            ChatDelta::Finished {
                reason: StopReason::Complete,
            },
        ],
        stop: StopReason::Complete,
    }
}

/// 一次要求执行工具的响应；`calls` 为 `(call_id, 工具名, 参数 JSON)`。
fn tool_step(calls: &[(&str, &str, &str)]) -> Step {
    let mut deltas = Vec::new();
    for (ordinal, (call_id, name, arguments)) in calls.iter().enumerate() {
        deltas.push(ChatDelta::ToolInputStarted {
            call_id: (*call_id).to_string(),
            name: (*name).to_string(),
            ordinal,
        });
        deltas.push(ChatDelta::ToolInputDelta {
            call_id: (*call_id).to_string(),
            chunk: (*arguments).to_string(),
        });
        deltas.push(ChatDelta::ToolInputEnded {
            call_id: (*call_id).to_string(),
            arguments_json: (*arguments).to_string(),
        });
    }
    deltas.push(ChatDelta::ResponseFinished {
        stop: ModelStepStop::ToolCalls,
    });
    deltas.push(ChatDelta::Finished {
        reason: StopReason::Complete,
    });
    Step::Response {
        deltas,
        stop: StopReason::Complete,
    }
}

/// 脚本化模型流：主轮与子 agent 轮各自一条队列，顺带记录全部请求快照。
#[derive(Default)]
struct ScriptedFactory {
    main: Mutex<VecDeque<Step>>,
    subagent: Mutex<VecDeque<Step>>,
    requests: Mutex<Vec<ChatRequest>>,
}

impl ScriptedFactory {
    fn new(main: Vec<Step>, subagent: Vec<Step>) -> Arc<Self> {
        Arc::new(Self {
            main: Mutex::new(main.into()),
            subagent: Mutex::new(subagent.into()),
            requests: Mutex::new(Vec::new()),
        })
    }

    fn requests(&self) -> Vec<ChatRequest> {
        self.requests
            .lock()
            .map(|requests| requests.clone())
            .unwrap_or_default()
    }

    fn main_requests(&self) -> Vec<ChatRequest> {
        self.requests()
            .into_iter()
            .filter(|request| !matches!(request.purpose, RequestPurpose::Subagent(_)))
            .collect()
    }

    fn take(&self, purpose: RequestPurpose) -> Step {
        let queue = match purpose {
            RequestPurpose::Subagent(_) => &self.subagent,
            _ => &self.main,
        };
        queue
            .lock()
            .ok()
            .and_then(|mut queue| queue.pop_front())
            .unwrap_or(Step::Response {
                deltas: Vec::new(),
                stop: StopReason::Failed {
                    code: "script exhausted".to_string(),
                },
            })
    }
}

impl ExecStreamFactory for ScriptedFactory {
    fn stream(
        &self,
        _model: &ExecModelTarget,
        request: ChatRequest,
        cancel: watch::Receiver<bool>,
        mut on_delta: Box<dyn FnMut(&[ChatDelta]) + Send>,
    ) -> BoxStreamFuture {
        if let Ok(mut requests) = self.requests.lock() {
            requests.push(request.clone());
        }
        let step = self.take(request.purpose);
        Box::pin(async move {
            match step {
                Step::Response { deltas, stop } => {
                    on_delta(&deltas);
                    StreamOutcome { deltas, stop }
                }
                Step::Hang => hang_until_cancelled(cancel).await,
            }
        })
    }
}

async fn hang_until_cancelled(mut cancel: watch::Receiver<bool>) -> StreamOutcome {
    while !*cancel.borrow() {
        if cancel.changed().await.is_err() {
            // 发送端已释放：不会再有取消，按失败收尾而不是无限等待。
            return StreamOutcome {
                deltas: Vec::new(),
                stop: StopReason::Failed {
                    code: "script hang released".to_string(),
                },
            };
        }
    }
    StreamOutcome {
        deltas: Vec::new(),
        stop: StopReason::Cancelled,
    }
}

/// 走生产装配路径（MCP 快照注入空实现，测试不启动任何 server）。
fn assemble(
    db: &Database,
    dir: &TestDir,
    prompt: &str,
    existing_session: Option<&str>,
) -> ExecTurnConfig {
    assemble_exec_turn_with_snapshot(
        db,
        &dir.path,
        &dir.path,
        existing_session,
        prompt,
        &[],
        &model_target(),
        Arc::new(crate::mcp::McpToolSnapshot::default()),
    )
    .expect("assemble exec turn")
}

/// 手工装配一轮（工具面可覆盖）：用于验证 exec 宿主自己的 `ask` 兜底。
fn manual_turn(db: &Database, dir: &TestDir, prompt: &str) -> ExecTurnConfig {
    let model = model_target();
    let committed = db
        .begin_turn(&dir.path, None, prompt, &model.snapshot())
        .expect("begin turn");
    let request = ChatRequest {
        turns: vec![ChatTurn {
            role: "user".into(),
            text: prompt.to_string(),
            reasoning_metadata: None,
        }],
        items: vec![ModelItem::UserText(prompt.to_string())],
        model: model.snapshot(),
        variant_params: None,
        purpose: RequestPurpose::ExecTurn,
        system_prompt: Some("test system prompt".to_string()),
        tools: Arc::new(crate::chat::tools::builtin_tools(
            crate::chat::agent::ToolSurface::Main,
        )),
    };
    ExecTurnConfig {
        workspace: dir.path.clone(),
        cwd: dir.path.clone(),
        session_id: committed.session_id,
        user_message_id: committed.user_message_id,
        assistant_id: committed.assistant_message_id,
        prompt_display: prompt.to_string(),
        model,
        context: ModelContext::new(request, dir.path.clone()),
        exa_api_key: String::new(),
        tavily_api_key: String::new(),
        mcp_snapshot: Arc::new(crate::mcp::McpToolSnapshot::default()),
    }
}

fn session_messages(
    db: &Database,
    workspace: &Path,
    session_id: &str,
) -> Vec<crate::db::chat::ChatMessage> {
    db.load_chat_session(workspace, session_id)
        .expect("load session")
        .expect("session exists")
}

fn tool_result_texts(request: &ChatRequest) -> Vec<String> {
    request
        .items
        .iter()
        .filter_map(|item| match item {
            ModelItem::ToolResult { content, .. } => Some(content.clone()),
            _ => None,
        })
        .collect()
}

// ===== 1. 单步纯文本 =====

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn single_text_step_completes_and_writes_the_final_message() {
    let dir = TestDir::new("single-step");
    let db = Database::open_in_memory().expect("in-memory db");
    let config = assemble(&db, &dir, "say hello", None);
    let session_id = config.session_id.clone();
    let assistant_id = config.assistant_id.clone();

    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    let last_message = dir.join("nested/final.txt");
    let mut processor = HumanOutputProcessor::new(
        &mut stdout,
        &mut stderr,
        false,
        false,
        Some(last_message.clone()),
    );
    let (_cancel_tx, cancel_rx) = watch::channel(false);
    let summary = run_exec_turn(
        &db,
        config,
        &mut processor,
        ScriptedFactory::new(vec![text_step("hello from exec")], Vec::new()),
        cancel_rx,
    )
    .await;

    assert_eq!(summary.outcome, TurnOutcome::Complete);
    assert_eq!(stdout, b"hello from exec\n");
    assert_eq!(
        std::fs::read_to_string(&last_message).expect("last message file"),
        "hello from exec"
    );

    let messages = session_messages(&db, &dir.path, &session_id);
    let assistant = messages
        .iter()
        .find(|message| message.external_id == assistant_id)
        .expect("assistant message");
    assert_eq!(assistant.status, "complete");
    assert!(
        assistant
            .parts
            .iter()
            .any(|part| part.kind == "text" && part.content == "hello from exec"),
        "assistant parts: {:?}",
        assistant.parts
    );

    let progress = String::from_utf8(stderr).expect("utf8");
    assert!(progress.contains("lx-tui exec v"), "{progress}");
    assert!(progress.contains("user\nsay hello"), "{progress}");
    assert!(progress.contains("tokens used:"), "{progress}");
}

// ===== 2. 多步工具调用 =====

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn tool_steps_touch_the_disk_and_are_persisted_as_completed() {
    let dir = TestDir::new("tool-chain");
    let db = Database::open_in_memory().expect("in-memory db");
    let target = dir.join("notes.txt");
    let write_args = format!(
        r#"{{"path":"{}","content":"written by exec\n"}}"#,
        target.display()
    );
    let read_args = format!(r#"{{"path":"{}"}}"#, target.display());
    let exec_args = r#"{"cmd":"printf exec-ok","login":false,"yield_time_ms":2000}"#;
    let step = tool_step(&[
        ("call-write", "write", write_args.as_str()),
        ("call-read", "read", read_args.as_str()),
        ("call-exec", "exec_command", exec_args),
    ]);
    let config = assemble(&db, &dir, "prepare the notes file", None);
    let session_id = config.session_id.clone();
    let factory = ScriptedFactory::new(vec![step, text_step("all done")], Vec::new());

    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    let mut processor = HumanOutputProcessor::new(&mut stdout, &mut stderr, false, false, None);
    let (_cancel_tx, cancel_rx) = watch::channel(false);
    let summary = run_exec_turn(&db, config, &mut processor, factory.clone(), cancel_rx).await;

    assert_eq!(summary.outcome, TurnOutcome::Complete);
    assert_eq!(stdout, b"all done\n");
    assert_eq!(
        std::fs::read_to_string(&target).expect("written file"),
        "written by exec\n"
    );

    let messages = session_messages(&db, &dir.path, &session_id);
    let tools: Vec<_> = messages
        .iter()
        .flat_map(|message| message.tools.iter())
        .collect();
    assert_eq!(tools.len(), 3, "tools: {tools:?}");
    assert_eq!(tools[0].name, "write");
    assert_eq!(tools[1].name, "read");
    assert_eq!(tools[2].name, "exec_command");
    for tool in &tools {
        assert_eq!(tool.status, "completed", "tool {}: {tool:?}", tool.name);
    }

    // 第二步拿到真实工具结果：读取正文与命令输出，而不是占位文本。
    let requests = factory.main_requests();
    assert_eq!(requests.len(), 2);
    let results = tool_result_texts(&requests[1]);
    assert!(
        results.iter().any(|text| text.contains("written by exec")),
        "tool results: {results:?}"
    );
    assert!(
        results.iter().any(|text| text.contains("exec-ok")),
        "tool results: {results:?}"
    );
}

// ===== 3. `ask` 无头拒绝 =====

const ASK_ARGS: &str =
    r#"{"questions":[{"id":"scope","header":"Scope","question":"Which scope?"}]}"#;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn ask_is_rejected_without_hanging_and_the_turn_completes() {
    let dir = TestDir::new("ask-rejected");
    let db = Database::open_in_memory().expect("in-memory db");
    let config = assemble(&db, &dir, "ask me something", None);
    let session_id = config.session_id.clone();
    let factory = ScriptedFactory::new(
        vec![
            tool_step(&[("call-ask", "ask", ASK_ARGS)]),
            text_step("done"),
        ],
        Vec::new(),
    );

    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    let mut processor = HumanOutputProcessor::new(&mut stdout, &mut stderr, false, false, None);
    let (_cancel_tx, cancel_rx) = watch::channel(false);
    let summary = run_exec_turn(&db, config, &mut processor, factory.clone(), cancel_rx).await;

    // ExecMain 不声明 `ask`：调用被工具面拦截（工具级失败），模型在第二步正常收尾。
    assert_eq!(summary.outcome, TurnOutcome::Complete);
    assert_eq!(stdout, b"done\n");
    let messages = session_messages(&db, &dir.path, &session_id);
    let ask = messages
        .iter()
        .flat_map(|message| message.tools.iter())
        .find(|tool| tool.name == "ask")
        .expect("ask tool call recorded");
    assert_eq!(ask.status, "failed");

    // 第二步请求必须携带拒绝结果，而不是挂起等待输入。
    let requests = factory.main_requests();
    assert_eq!(requests.len(), 2, "requests: {}", requests.len());
    let results = tool_result_texts(&requests[1]);
    assert!(
        results.iter().any(|text| text.contains("unavailable")),
        "tool results: {results:?}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn exec_host_rejects_ask_with_the_stable_exec_code() {
    let dir = TestDir::new("ask-host-fallback");
    let db = Database::open_in_memory().expect("in-memory db");
    let config = manual_turn(&db, &dir, "ask me directly");
    let factory = ScriptedFactory::new(
        vec![tool_step(&[("call-ask", "ask", ASK_ARGS)])],
        Vec::new(),
    );

    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    let mut processor = HumanOutputProcessor::new(&mut stdout, &mut stderr, false, false, None);
    let (_cancel_tx, cancel_rx) = watch::channel(false);
    let summary = run_exec_turn(&db, config, &mut processor, factory, cancel_rx).await;

    assert_eq!(
        summary.outcome,
        TurnOutcome::Failed {
            code: EXEC_ASK_UNSUPPORTED_CODE.to_string()
        }
    );
}

// ===== 4. 子 agent 协同 =====

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn explorer_subagent_reads_a_file_and_reports_back() {
    let dir = TestDir::new("subagent");
    let notes = dir.join("notes.txt");
    std::fs::write(&notes, "the answer is 42\n").expect("write notes");

    let db = Database::open_in_memory().expect("in-memory db");
    let spawn_args = r#"{"role":"explorer","title":"scan notes","task":"read notes.txt and report what it says"}"#;
    let wait_args = r#"{"timeout_ms":10000}"#;
    let read_args = format!(r#"{{"path":"{}"}}"#, notes.display());
    let config = assemble(&db, &dir, "delegate the reading to an explorer", None);
    let session_id = config.session_id.clone();
    let factory = ScriptedFactory::new(
        vec![
            tool_step(&[("call-spawn", "spawn_agent", spawn_args)]),
            tool_step(&[("call-wait", "wait_agent", wait_args)]),
            text_step("parent finished"),
        ],
        vec![
            tool_step(&[("child-read", "read", read_args.as_str())]),
            text_step("report from scanner: notes say the answer is 42"),
        ],
    );

    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    let mut processor = HumanOutputProcessor::new(&mut stdout, &mut stderr, false, false, None);
    let (_cancel_tx, cancel_rx) = watch::channel(false);
    let summary = run_exec_turn(&db, config, &mut processor, factory.clone(), cancel_rx).await;

    assert_eq!(summary.outcome, TurnOutcome::Complete);
    assert_eq!(stdout, b"parent finished\n");

    let requests = factory.requests();
    let child_requests = requests
        .iter()
        .filter(|request| matches!(request.purpose, RequestPurpose::Subagent(_)))
        .count();
    assert_eq!(child_requests, 2, "child turns: {child_requests}");
    let main = factory.main_requests();
    assert_eq!(main.len(), 3, "parent turns: {}", main.len());
    let waited = tool_result_texts(&main[2]);
    assert!(
        waited
            .iter()
            .any(|text| text.contains("report from scanner: notes say the answer is 42")),
        "wait_agent results: {waited:?}"
    );

    // 父会话完整：spawn / wait 两条工具记录都已结算。
    let messages = session_messages(&db, &dir.path, &session_id);
    let names: Vec<&str> = messages
        .iter()
        .flat_map(|message| message.tools.iter())
        .map(|tool| tool.name.as_str())
        .collect();
    assert_eq!(names, ["spawn_agent", "wait_agent"]);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn spawning_a_fourth_concurrent_subagent_is_rejected_with_limit_reached() {
    let dir = TestDir::new("subagent-limit");
    let db = Database::open_in_memory().expect("in-memory db");
    let spawn_1 = r#"{"role":"explorer","title":"a1","task":"task 1"}"#;
    let spawn_2 = r#"{"role":"explorer","title":"a2","task":"task 2"}"#;
    let spawn_3 = r#"{"role":"explorer","title":"a3","task":"task 3"}"#;
    let spawn_4 = r#"{"role":"explorer","title":"a4","task":"task 4"}"#;
    let config = assemble(&db, &dir, "spawn four explorers", None);
    let factory = ScriptedFactory::new(
        vec![
            tool_step(&[
                ("call-s1", "spawn_agent", spawn_1),
                ("call-s2", "spawn_agent", spawn_2),
                ("call-s3", "spawn_agent", spawn_3),
                ("call-s4", "spawn_agent", spawn_4),
            ]),
            text_step("observed limit"),
        ],
        // 3 个子 agent 全部挂起直到主轮结束被清理，确保第 4 个 spawn 时前 3 个仍在运行。
        vec![Step::Hang, Step::Hang, Step::Hang],
    );

    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    let mut processor = HumanOutputProcessor::new(&mut stdout, &mut stderr, false, false, None);
    let (_cancel_tx, cancel_rx) = watch::channel(false);
    let summary = run_exec_turn(&db, config, &mut processor, factory.clone(), cancel_rx).await;

    assert_eq!(summary.outcome, TurnOutcome::Complete);
    let main = factory.main_requests();
    assert_eq!(main.len(), 2);
    let results = tool_result_texts(&main[1]);
    assert_eq!(results.len(), 4, "results: {results:?}");
    assert!(
        results[3].contains(crate::chat::agent::LIMIT_REACHED_CODE),
        "4th spawn result: {}",
        results[3]
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn ephemeral_in_memory_turn_does_not_touch_the_disk_database() {
    let dir = TestDir::new("ephemeral");
    let disk_db_path = dir.join("disk.db");
    let disk_db = Database::open(&disk_db_path).expect("open disk db");
    assert!(
        disk_db
            .list_chat_sessions(&dir.path, 0, 10)
            .expect("list disk sessions")
            .is_empty()
    );

    let ephemeral_db = Database::open_in_memory().expect("open in-memory db");
    let config = assemble(&ephemeral_db, &dir, "ephemeral question", None);
    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    let mut processor = HumanOutputProcessor::new(&mut stdout, &mut stderr, false, false, None);
    let (_cancel_tx, cancel_rx) = watch::channel(false);
    let summary = run_exec_turn(
        &ephemeral_db,
        config,
        &mut processor,
        ScriptedFactory::new(vec![text_step("ephemeral answer")], Vec::new()),
        cancel_rx,
    )
    .await;
    assert_eq!(summary.outcome, TurnOutcome::Complete);
    assert_eq!(stdout, b"ephemeral answer\n");

    // 磁盘数据库零新增会话。
    assert!(
        disk_db
            .list_chat_sessions(&dir.path, 0, 10)
            .expect("list disk sessions after ephemeral")
            .is_empty()
    );
}

// ===== 5. resume --last =====

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn resume_last_replays_history_and_increments_turn_index() {
    let dir = TestDir::new("resume-last");
    let db = Database::open_in_memory().expect("in-memory db");

    let first = assemble(&db, &dir, "first question", None);
    let session_id = first.session_id.clone();
    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    let mut processor = HumanOutputProcessor::new(&mut stdout, &mut stderr, false, false, None);
    let (_cancel_tx, cancel_rx) = watch::channel(false);
    let summary = run_exec_turn(
        &db,
        first,
        &mut processor,
        ScriptedFactory::new(vec![text_step("first answer")], Vec::new()),
        cancel_rx,
    )
    .await;
    assert_eq!(summary.outcome, TurnOutcome::Complete);

    let args = ResumeArgs {
        last: true,
        ..ResumeArgs::default()
    };
    let resumed = resolve_resume_session(&db, &dir.path, &args).expect("resume last");
    assert_eq!(resumed, session_id);

    let second = assemble(&db, &dir, "second question", Some(&resumed));
    let factory = ScriptedFactory::new(vec![text_step("second answer")], Vec::new());
    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    let mut processor = HumanOutputProcessor::new(&mut stdout, &mut stderr, false, false, None);
    let (_cancel_tx, cancel_rx) = watch::channel(false);
    let summary = run_exec_turn(&db, second, &mut processor, factory.clone(), cancel_rx).await;
    assert_eq!(summary.outcome, TurnOutcome::Complete);

    let requests = factory.main_requests();
    assert_eq!(requests.len(), 1);
    let items = &requests[0].items;
    assert!(
        items
            .iter()
            .any(|item| matches!(item, ModelItem::UserText(text) if text == "first question")),
        "items: {items:?}"
    );
    assert!(
        items
            .iter()
            .any(|item| matches!(item, ModelItem::AssistantText(text) if text == "first answer")),
        "items: {items:?}"
    );
    assert!(
        items
            .iter()
            .any(|item| matches!(item, ModelItem::UserText(text) if text == "second question")),
        "items: {items:?}"
    );

    let messages = session_messages(&db, &dir.path, &session_id);
    let last_user = messages
        .iter()
        .rev()
        .find(|message| message.role == "user")
        .expect("second user message");
    assert_eq!(last_user.turn_index, 1);
    assert_eq!(last_user.status, "complete");
}

#[test]
fn assemble_exec_turn_creates_the_turn_and_freezes_the_model() {
    let dir = TestDir::new("assemble");
    let db = Database::open_in_memory().expect("in-memory db");
    let config = super::super::assemble_exec_turn(
        &db,
        &dir.path,
        &dir.path,
        None,
        "hello there",
        &[],
        &model_target(),
    )
    .expect("assemble exec turn");

    assert!(
        config.session_id.starts_with("sess"),
        "{}",
        config.session_id
    );
    assert_eq!(config.workspace, dir.path);
    assert_eq!(config.cwd, dir.path);
    assert_eq!(config.model.provider, "test-provider");
    assert_eq!(config.prompt_display, "hello there");

    let messages = session_messages(&db, &dir.path, &config.session_id);
    assert_eq!(messages.len(), 2, "messages: {messages:?}");
    assert_eq!(messages[0].role, "user");
    assert_eq!(messages[0].status, "complete");
    assert_eq!(messages[1].role, "assistant");
    assert_eq!(messages[1].status, "streaming");
}

#[test]
fn image_attachments_require_an_image_capable_model() {
    let dir = TestDir::new("assemble-image");
    let db = Database::open_in_memory().expect("in-memory db");
    let error = match super::super::assemble_exec_turn(
        &db,
        &dir.path,
        &dir.path,
        None,
        "look at this",
        &[dir.join("shot.png")],
        &model_target(),
    ) {
        Ok(_) => panic!("text-only model rejects images"),
        Err(error) => error,
    };
    assert_eq!(error, "Current model does not support image input.");
}

#[test]
fn resume_addressing_errors_are_explicit() {
    let dir = TestDir::new("resume-errors");
    let db = Database::open_in_memory().expect("in-memory db");

    let args = ResumeArgs {
        session_id: Some("sess-missing".to_string()),
        ..ResumeArgs::default()
    };
    let error = resolve_resume_session(&db, &dir.path, &args).expect_err("missing session");
    assert_eq!(
        error,
        format!(
            "Session 'sess-missing' not found in workspace {}.",
            dir.path.display()
        )
    );

    let args = ResumeArgs {
        last: true,
        ..ResumeArgs::default()
    };
    let error = resolve_resume_session(&db, &dir.path, &args).expect_err("no sessions");
    assert_eq!(
        error,
        format!(
            "No existing sessions found in workspace {}.",
            dir.path.display()
        )
    );

    let error =
        resolve_resume_session(&db, &dir.path, &ResumeArgs::default()).expect_err("no addressing");
    assert!(error.contains("--last"), "{error}");
}

// ===== 6. Ctrl+C 取消 =====

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cancel_interrupts_an_in_flight_turn_and_persists_it() {
    let dir = TestDir::new("cancel");
    let db = Database::open_in_memory().expect("in-memory db");
    let config = assemble(&db, &dir, "start a long turn", None);
    let session_id = config.session_id.clone();
    let assistant_id = config.assistant_id.clone();
    let factory = ScriptedFactory::new(vec![Step::Hang], Vec::new());

    let mut output = Vec::new();
    let mut processor = JsonlOutputProcessor::new(&mut output, None);
    let (cancel_tx, cancel_rx) = watch::channel(false);
    let canceller = tokio::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        let _ = cancel_tx.send(true);
    });
    let summary = tokio::time::timeout(
        std::time::Duration::from_secs(10),
        run_exec_turn(&db, config, &mut processor, factory, cancel_rx),
    )
    .await
    .expect("turn must finish after cancellation");
    canceller.await.expect("canceller task");

    assert_eq!(summary.outcome, TurnOutcome::Interrupted);
    let messages = session_messages(&db, &dir.path, &session_id);
    let assistant = messages
        .iter()
        .find(|message| message.external_id == assistant_id)
        .expect("assistant message");
    assert_eq!(assistant.status, "interrupted");
    assert!(assistant.completed_at_ms.is_some());

    let events = String::from_utf8(output).expect("utf8");
    assert!(events.contains("\"type\":\"turn.failed\""), "{events}");
    assert!(events.contains("turn interrupted"), "{events}");
}

// 真实网络流只在生产路径用到：此处仅锁定它仍可被构造成 trait object。
#[allow(dead_code)]
fn real_factory_is_object_safe() -> Arc<dyn ExecStreamFactory> {
    Arc::new(RealStreamFactory)
}
