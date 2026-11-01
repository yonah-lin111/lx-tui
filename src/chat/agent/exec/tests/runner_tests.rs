//! `exec` 运行器端到端测试：脚本化模型流驱动真实数据库、真实工具与真实装配路径。
//!
//! 不触网：模型流由 [`ScriptedFactory`] 回放脚本；其余全部走生产路径。
//! 子 agent 协同场景见 `subagent_tests`。

use std::path::PathBuf;
use std::sync::Arc;

use tokio::sync::watch;

use super::super::cli::ResumeArgs;
use super::super::runner::ExecStreamFactory;
use super::super::{
    HumanOutputProcessor, JsonlOutputProcessor, RealStreamFactory, resolve_resume_session,
    run_exec_turn,
};
use super::fixtures::{
    ScriptedFactory, Step, TestDir, assemble, assemble_images, image_model_target, manual_turn,
    manual_turn_with, model_target, session_messages, text_step, tool_result_texts, tool_step,
    tool_step_with_text, write_test_png,
};
use crate::chat::agent::EXEC_ASK_UNSUPPORTED_CODE;
use crate::chat::agent_loop::TurnOutcome;
use crate::chat::events::{ModelItem, RequestPurpose, StopReason};
use crate::db::Database;

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

// ===== 4. 临时内存库 =====

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

// ===== 7. Review 回归（P0-2 / P0-3 / P1-1 / P1-2 / P1-5）=====

/// P0-2：主 runner 收到 `AutoCompact` 必须把摘要落成检查点，而不是只打印告警。
///
/// 自动压缩只在子 agent 上下文启用；这里让同一主 runner 执行该上下文的轮次，
/// 直接覆盖 `ExecLoopEvent::AutoCompact` 的持久化分支。
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn main_turn_auto_compact_persists_a_checkpoint() {
    let dir = TestDir::new("auto-compact");
    let db = Database::open_in_memory().expect("in-memory db");
    let config = manual_turn_with(
        &db,
        &dir,
        "investigate the directory",
        RequestPurpose::Subagent(crate::chat::agent::AgentRole::Explorer),
        Some(1),
    );
    let session_id = config.session_id.clone();
    // 上下文按 Subagent 用途路由：主轮采样走子 agent 队列，摘要请求走主队列。
    let factory = ScriptedFactory::new(
        vec![text_step("summary of the investigation")],
        vec![
            tool_step(&[("call-read", "read", r#"{"path":"."}"#)]),
            text_step("done"),
        ],
    );
    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    let mut processor = HumanOutputProcessor::new(&mut stdout, &mut stderr, false, false, None);
    let (_cancel_tx, cancel_rx) = watch::channel(false);
    let summary = run_exec_turn(&db, config, &mut processor, factory, cancel_rx).await;

    assert_eq!(summary.outcome, TurnOutcome::Complete);
    let checkpoints = db
        .compact_checkpoints(&dir.path, &session_id)
        .expect("compact checkpoints");
    assert_eq!(checkpoints.len(), 1, "checkpoints: {checkpoints:?}");
    assert_eq!(checkpoints[0].summary_text, "summary of the investigation");
    assert!(checkpoints[0].is_active());
}

/// P0-3 + P1-5：相对 `-i` 按 `-C` 解析；resume 时历史图片必须回填字节，
/// 切换到不支持图片的模型时降级为显式提示文本。
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn resume_hydrates_historical_images_and_degrades_for_text_only_models() {
    let dir = TestDir::new("resume-images");
    let db = Database::open_in_memory().expect("in-memory db");
    write_test_png(&dir.join("rel.png"));

    let image_model = image_model_target();
    let first = assemble_images(
        &db,
        &dir,
        "first image question",
        None,
        &[PathBuf::from("rel.png")],
        &image_model,
    );
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

    // 图片模型 resume：历史 UserContent 带回非空图片字节。
    let second = assemble_images(
        &db,
        &dir,
        "second question",
        Some(&session_id),
        &[],
        &image_model,
    );
    let factory = ScriptedFactory::new(vec![text_step("second answer")], Vec::new());
    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    let mut processor = HumanOutputProcessor::new(&mut stdout, &mut stderr, false, false, None);
    let (_cancel_tx, cancel_rx) = watch::channel(false);
    let summary = run_exec_turn(&db, second, &mut processor, factory.clone(), cancel_rx).await;
    assert_eq!(summary.outcome, TurnOutcome::Complete);

    let request = &factory.main_requests()[0];
    let images = request
        .items
        .iter()
        .find_map(|item| match item {
            ModelItem::UserContent { text, images } if text == "first image question" => {
                Some(images.clone())
            }
            _ => None,
        })
        .expect("historical user content with images");
    assert!(!images.is_empty(), "history images must be hydrated");
    assert!(!images[0].bytes.is_empty(), "image bytes must be non-empty");
    assert_eq!(images[0].mime, "image/png");

    // 文本模型 resume：历史图片降级为提示文本，且当前轮仍可正常发送。
    let third = assemble_images(
        &db,
        &dir,
        "third question",
        Some(&session_id),
        &[],
        &model_target(),
    );
    let factory = ScriptedFactory::new(vec![text_step("third answer")], Vec::new());
    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    let mut processor = HumanOutputProcessor::new(&mut stdout, &mut stderr, false, false, None);
    let (_cancel_tx, cancel_rx) = watch::channel(false);
    let summary = run_exec_turn(&db, third, &mut processor, factory.clone(), cancel_rx).await;
    assert_eq!(summary.outcome, TurnOutcome::Complete);

    let degraded = factory.main_requests()[0]
        .items
        .iter()
        .find_map(|item| match item {
            ModelItem::UserText(text) if text.starts_with("first image question") => {
                Some(text.clone())
            }
            _ => None,
        })
        .expect("degraded historical text");
    assert!(degraded.contains("ERROR: Cannot read"), "{degraded}");
    assert!(
        degraded.contains("does not support image input"),
        "{degraded}"
    );
    crate::platform::remove_session_images(&session_id);
}

/// P1-1：项目 skill 目录进入系统提示，`$skill-name` 正文作为当轮块注入。
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn project_skill_catalog_and_dollar_reference_are_injected() {
    let dir = TestDir::new("skills");
    let db = Database::open_in_memory().expect("in-memory db");
    let root = crate::config::skills::project_skills_root(&dir.path).expect("project skills root");
    let skill_dir = root.join("my-skill");
    std::fs::create_dir_all(&skill_dir).expect("create skill dir");
    std::fs::write(
        skill_dir.join("SKILL.md"),
        "---\nname: my-skill\ndescription: Handle the special task.\n---\nUse the special procedure.\n",
    )
    .expect("write skill");

    let config = assemble(&db, &dir, "please use $my-skill now", None);
    let factory = ScriptedFactory::new(vec![text_step("skill applied")], Vec::new());
    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    let mut processor = HumanOutputProcessor::new(&mut stdout, &mut stderr, false, false, None);
    let (_cancel_tx, cancel_rx) = watch::channel(false);
    let summary = run_exec_turn(&db, config, &mut processor, factory.clone(), cancel_rx).await;
    assert_eq!(summary.outcome, TurnOutcome::Complete);

    let request = &factory.main_requests()[0];
    let system = request.system_prompt.as_deref().unwrap_or_default();
    assert!(system.contains("## Skills"), "{system}");
    assert!(system.contains("my-skill"), "{system}");
    assert!(
        request.items.iter().any(|item| matches!(
            item,
            ModelItem::UserText(text)
                if text.contains("<skill>")
                    && text.contains("<name>my-skill</name>")
                    && text.contains("Use the special procedure.")
        )),
        "items: {:?}",
        request.items
    );
    let _ = std::fs::remove_dir_all(&root);
}

/// P1-2：工具前说明只作为中间步骤输出，失败轮次不得把它当最终回复写 stdout/-o。
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn tool_preamble_text_is_not_reused_as_the_final_message() {
    let dir = TestDir::new("tool-preamble");
    let db = Database::open_in_memory().expect("in-memory db");
    let config = assemble(&db, &dir, "inspect the directory", None);
    let factory = ScriptedFactory::new(
        vec![
            tool_step_with_text(
                "I will run a tool",
                &[("call-read", "read", r#"{"path":"."}"#)],
            ),
            Step::Response {
                deltas: Vec::new(),
                stop: StopReason::Failed {
                    code: "stream failed".to_string(),
                },
            },
        ],
        Vec::new(),
    );
    let last_message = dir.join("final.txt");
    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    let mut processor = HumanOutputProcessor::new(
        &mut stdout,
        &mut stderr,
        false,
        false,
        Some(last_message.clone()),
    );
    let (_cancel_tx, cancel_rx) = watch::channel(false);
    let summary = run_exec_turn(&db, config, &mut processor, factory, cancel_rx).await;

    assert!(matches!(summary.outcome, TurnOutcome::Failed { .. }));
    assert_eq!(stdout, b"");
    assert_eq!(
        std::fs::read_to_string(&last_message).unwrap_or_default(),
        ""
    );
    let progress = String::from_utf8(stderr).expect("utf8");
    assert!(progress.contains("I will run a tool"), "{progress}");
}

// 真实网络流只在生产路径用到：此处仅锁定它仍可被构造成 trait object。
#[allow(dead_code)]
fn real_factory_is_object_safe() -> Arc<dyn ExecStreamFactory> {
    Arc::new(RealStreamFactory)
}
