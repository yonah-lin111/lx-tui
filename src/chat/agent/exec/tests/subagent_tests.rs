//! `exec` 子 agent 协同测试：spawn / wait、并发上限、打断续跑与自动续等。
//!
//! 夹具与主轮测试共用 `fixtures`。

use tokio::sync::watch;

use super::super::{HumanOutputProcessor, run_exec_turn};
use super::fixtures::{
    ScriptedFactory, Step, TestDir, assemble, delayed, session_messages, text_step,
    tool_result_texts, tool_step,
};
use crate::chat::agent::{SubagentReportView, format_report_envelope};
use crate::chat::agent_loop::TurnOutcome;
use crate::chat::events::{ChatRequest, ModelItem, RequestPurpose};
use crate::db::Database;

// ===== 1. spawn + wait =====

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

// ===== 2. 并发上限 =====

/// 整树并发上限（默认 4）：第 5 个并发 spawn 必须以 `LIMIT_REACHED_CODE` 拒绝。
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn spawning_a_fifth_concurrent_subagent_is_rejected_with_limit_reached() {
    let dir = TestDir::new("subagent-limit");
    let db = Database::open_in_memory().expect("in-memory db");
    let spawn_1 = r#"{"role":"explorer","title":"a1","task":"task 1"}"#;
    let spawn_2 = r#"{"role":"explorer","title":"a2","task":"task 2"}"#;
    let spawn_3 = r#"{"role":"explorer","title":"a3","task":"task 3"}"#;
    let spawn_4 = r#"{"role":"explorer","title":"a4","task":"task 4"}"#;
    let spawn_5 = r#"{"role":"explorer","title":"a5","task":"task 5"}"#;
    let config = assemble(&db, &dir, "spawn five explorers", None);
    let factory = ScriptedFactory::new(
        vec![
            tool_step(&[
                ("call-s1", "spawn_agent", spawn_1),
                ("call-s2", "spawn_agent", spawn_2),
                ("call-s3", "spawn_agent", spawn_3),
                ("call-s4", "spawn_agent", spawn_4),
                ("call-s5", "spawn_agent", spawn_5),
            ]),
            text_step("observed limit"),
        ],
        // 4 个子 agent 全部挂起直到主轮结束被清理，确保第 5 个 spawn 时前 4 个仍在运行。
        vec![Step::Hang, Step::Hang, Step::Hang, Step::Hang],
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
    assert_eq!(results.len(), 5, "results: {results:?}");
    assert!(
        results[4].contains(crate::chat::agent::LIMIT_REACHED_CODE),
        "5th spawn result: {}",
        results[4]
    );
}

// ===== 3. `send_input(interrupt = true)` 续跑 =====

/// P1-3：`send_input(interrupt = true)` 打断在途子 agent 后，队列输入必须自动续跑。
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn interrupting_a_subagent_with_send_input_runs_the_queued_turn_to_completion() {
    let dir = TestDir::new("subagent-interrupt");
    let db = Database::open_in_memory().expect("in-memory db");
    let spawn_args = r#"{"role":"explorer","title":"interruptible","task":"wait for a follow-up"}"#;
    let wait_args = r#"{"timeout_ms":10000}"#;
    let config = assemble(&db, &dir, "spawn then interrupt", None);
    let factory = ScriptedFactory::new(
        vec![
            tool_step(&[("call-spawn", "spawn_agent", spawn_args)]),
            Step::SendInputInterrupt {
                message: "answer the urgent follow-up".to_string(),
            },
            tool_step(&[("call-wait", "wait_agent", wait_args)]),
            text_step("parent done"),
        ],
        vec![Step::Hang, text_step("urgent answer from child")],
    );
    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    let mut processor = HumanOutputProcessor::new(&mut stdout, &mut stderr, false, false, None);
    let (_cancel_tx, cancel_rx) = watch::channel(false);
    let summary = run_exec_turn(&db, config, &mut processor, factory.clone(), cancel_rx).await;

    assert_eq!(summary.outcome, TurnOutcome::Complete);
    assert_eq!(stdout, b"parent done\n");
    let main = factory.main_requests();
    assert_eq!(main.len(), 4, "parent turns: {}", main.len());
    let accepted = tool_result_texts(&main[2]);
    assert!(
        accepted.iter().any(|text| text.contains("interrupt=true")),
        "send_input results: {accepted:?}"
    );
    let waited = tool_result_texts(&main[3]);
    assert!(
        waited
            .iter()
            .any(|text| text.contains("urgent answer from child")),
        "wait_agent results: {waited:?}"
    );
}

// ===== 4. `wait_agent(all)` 跨超时窗口自动续等 =====

/// 空 `targets`（等全部）把 timeout 当作内部重检周期：子 agent 跨过第一个
/// 1000ms 超时窗口仍在运行，等待必须自动续等并拿到完整报告，而不是报超时。
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn wait_agent_all_auto_continues_across_short_timeout_until_child_finishes() {
    let dir = TestDir::new("subagent-slow-wait");
    let db = Database::open_in_memory().expect("in-memory db");
    let spawn_args = r#"{"role":"explorer","title":"slow scanner","task":"report after a pause"}"#;
    let wait_args = r#"{"timeout_ms":1000}"#;
    let config = assemble(&db, &dir, "wait for the slow explorer", None);
    let session_id = config.session_id.clone();
    let factory = ScriptedFactory::new(
        vec![
            tool_step(&[("call-spawn", "spawn_agent", spawn_args)]),
            tool_step(&[("call-wait", "wait_agent", wait_args)]),
            text_step("parent finished"),
        ],
        // 子轮 1.5s：跨过 wait_agent 的 1000ms 超时窗口后才结束。
        vec![delayed(1500, text_step("slow report: the answer is 42"))],
    );

    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    let mut processor = HumanOutputProcessor::new(&mut stdout, &mut stderr, false, false, None);
    let (_cancel_tx, cancel_rx) = watch::channel(false);
    let summary = run_exec_turn(&db, config, &mut processor, factory.clone(), cancel_rx).await;

    assert_eq!(summary.outcome, TurnOutcome::Complete);
    assert_eq!(stdout, b"parent finished\n");
    let child_requests = factory
        .requests()
        .iter()
        .filter(|request| matches!(request.purpose, RequestPurpose::Subagent(_)))
        .count();
    assert_eq!(child_requests, 1, "child turns: {child_requests}");

    let main = factory.main_requests();
    assert_eq!(main.len(), 3, "parent turns: {}", main.len());
    let waited = tool_result_texts(&main[2]);
    assert!(
        waited.iter().any(|text| text.contains("timed_out: false")),
        "wait_agent results: {waited:?}"
    );
    assert!(
        waited.iter().any(|text| text.contains("status=completed")),
        "wait_agent results: {waited:?}"
    );
    assert!(
        waited
            .iter()
            .any(|text| text.contains("slow report: the answer is 42")),
        "wait_agent results: {waited:?}"
    );

    let messages = session_messages(&db, &dir.path, &session_id);
    let names: Vec<&str> = messages
        .iter()
        .flat_map(|message| message.tools.iter())
        .map(|tool| tool.name.as_str())
        .collect();
    assert_eq!(names, ["spawn_agent", "wait_agent"]);
}

// ===== 5. `task_name` 路径、`list_agents(path_prefix)` 与变更文件 =====

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn spawn_with_task_name_and_list_agents_prefix_and_changed_paths() {
    let dir = TestDir::new("subagent-task-name");
    let db = Database::open_in_memory().expect("in-memory db");
    let spawn_args = r#"{"role":"executor","title":"patch the file","task":"write patched.txt","task_name":"patcher"}"#;
    let list_args = r#"{"path_prefix":"patcher"}"#;
    let wait_args = r#"{"targets":["/root/patcher"],"timeout_ms":10000}"#;
    let write_args = r#"{"path":"patched.txt","content":"patched by child\n"}"#;
    let config = assemble(&db, &dir, "delegate the file patch", None);
    let session_id = config.session_id.clone();
    let factory = ScriptedFactory::new(
        vec![
            tool_step(&[("call-spawn", "spawn_agent", spawn_args)]),
            tool_step(&[("call-list", "list_agents", list_args)]),
            tool_step(&[("call-wait", "wait_agent", wait_args)]),
            text_step("parent finished"),
        ],
        vec![
            tool_step(&[("child-write", "write", write_args)]),
            text_step("patched ok"),
        ],
    );

    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    let mut processor = HumanOutputProcessor::new(&mut stdout, &mut stderr, false, false, None);
    let (_cancel_tx, cancel_rx) = watch::channel(false);
    let summary = run_exec_turn(&db, config, &mut processor, factory.clone(), cancel_rx).await;

    assert_eq!(summary.outcome, TurnOutcome::Complete);
    assert_eq!(stdout, b"parent finished\n");
    assert_eq!(
        std::fs::read_to_string(dir.join("patched.txt")).expect("child write"),
        "patched by child\n"
    );

    let main = factory.main_requests();
    assert_eq!(main.len(), 4, "parent turns: {}", main.len());
    let listed = tool_result_texts(&main[2]);
    let listing = listed.last().expect("list_agents result");
    assert!(
        listing.contains("/root/patcher"),
        "list_agents results: {listed:?}"
    );
    let waited = tool_result_texts(&main[3]);
    let waited = waited.last().expect("wait_agent result");
    assert!(
        waited.contains("status=completed") && waited.contains("patched ok"),
        "wait_agent results: {waited:?}"
    );

    // 变更路径必须同时进入报告载荷，并由父 agent 的等待结果回显。
    let report = db
        .resolve_owned_subagent(&dir.path, &session_id, "/root/patcher")
        .expect("resolve patcher")
        .expect("patcher record")
        .report_json
        .as_deref()
        .and_then(SubagentReportView::from_json)
        .expect("patcher report");
    assert_eq!(report.changed_paths, ["patched.txt"]);
    assert!(
        format_report_envelope(&report).contains("Changed files: patched.txt"),
        "report envelope: {}",
        format_report_envelope(&report)
    );
    // 父 agent 的等待结果必须直接带出变更文件，而不是只留在 DB/信封里。
    assert!(
        waited.contains("Changed files: patched.txt"),
        "wait_agent must include changed files: {waited:?}"
    );
}

// ===== 6. `send_input(trigger_turn = false)` 仅入队 =====

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn send_input_with_trigger_turn_false_queues_without_waking_idle_subagent() {
    let dir = TestDir::new("subagent-trigger-turn");
    let db = Database::open_in_memory().expect("in-memory db");
    let spawn_args =
        r#"{"role":"explorer","title":"reader","task":"read the notes","task_name":"reader"}"#;
    let wait_args = r#"{"timeout_ms":10000}"#;
    let resume_args = r#"{"id":"/root/reader"}"#;
    let send_args =
        r#"{"target":"/root/reader","message":"queued follow-up","trigger_turn":false}"#;
    let config = assemble(&db, &dir, "queue a follow-up for the reader", None);
    let factory = ScriptedFactory::new(
        vec![
            tool_step(&[("call-spawn", "spawn_agent", spawn_args)]),
            tool_step(&[("call-wait", "wait_agent", wait_args)]),
            tool_step(&[("call-resume", "resume_agent", resume_args)]),
            tool_step(&[("call-send", "send_input", send_args)]),
            tool_step(&[("call-list", "list_agents", "{}")]),
            text_step("parent finished"),
        ],
        vec![text_step("reader done")],
    );

    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    let mut processor = HumanOutputProcessor::new(&mut stdout, &mut stderr, false, false, None);
    let (_cancel_tx, cancel_rx) = watch::channel(false);
    let summary = run_exec_turn(&db, config, &mut processor, factory.clone(), cancel_rx).await;

    assert_eq!(summary.outcome, TurnOutcome::Complete);
    let main = factory.main_requests();
    assert_eq!(main.len(), 6, "parent turns: {}", main.len());
    let sent = tool_result_texts(&main[4]);
    let sent = sent.last().expect("send_input result");
    assert!(
        sent.contains("trigger_turn=false") && sent.contains("queued_count=1"),
        "send_input results: {sent:?}"
    );
    let listed = tool_result_texts(&main[5]);
    let listing = listed.last().expect("list_agents result");
    assert!(
        listing.contains("/root/reader")
            && listing.contains("status=idle")
            && listing.contains("queued_count=1"),
        "list_agents results: {listed:?}"
    );
    let child_requests = factory
        .requests()
        .iter()
        .filter(|request| matches!(request.purpose, RequestPurpose::Subagent(_)))
        .count();
    assert_eq!(
        child_requests, 1,
        "trigger_turn=false must not wake the idle subagent"
    );
}

// ===== 7. `interrupt_agent` 半程汇报与 `close_agent` 抑制汇报 =====

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn interrupt_agent_by_path_produces_partial_report_while_close_suppresses_report() {
    let dir = TestDir::new("subagent-interrupt-close");
    let db = Database::open_in_memory().expect("in-memory db");
    let spawn_hanger = r#"{"role":"executor","title":"hanging worker","task":"wait forever","task_name":"hanger"}"#;
    let spawn_closer =
        r#"{"role":"executor","title":"closed worker","task":"wait forever","task_name":"closer"}"#;
    let interrupt_args = r#"{"target":"/root/hanger"}"#;
    let close_args = r#"{"target":"/root/closer"}"#;
    let wait_args = r#"{"timeout_ms":10000}"#;
    let config = assemble(&db, &dir, "stop the workers", None);
    let session_id = config.session_id.clone();
    let factory = ScriptedFactory::new(
        vec![
            tool_step(&[("call-spawn-hanger", "spawn_agent", spawn_hanger)]),
            tool_step(&[("call-interrupt", "interrupt_agent", interrupt_args)]),
            tool_step(&[("call-wait-hanger", "wait_agent", wait_args)]),
            tool_step(&[("call-spawn-closer", "spawn_agent", spawn_closer)]),
            tool_step(&[("call-close", "close_agent", close_args)]),
            tool_step(&[("call-wait-closer", "wait_agent", wait_args)]),
            text_step("parent finished"),
        ],
        vec![Step::Hang, Step::Hang],
    );

    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    let mut processor = HumanOutputProcessor::new(&mut stdout, &mut stderr, false, false, None);
    let (_cancel_tx, cancel_rx) = watch::channel(false);
    let summary = run_exec_turn(&db, config, &mut processor, factory.clone(), cancel_rx).await;

    assert_eq!(summary.outcome, TurnOutcome::Complete);
    let main = factory.main_requests();
    assert_eq!(main.len(), 7, "parent turns: {}", main.len());
    let interrupted = tool_result_texts(&main[3]);
    let interrupted = interrupted.last().expect("interrupt wait_agent result");
    assert!(
        interrupted.contains("/root/hanger")
            && interrupted.contains("status=idle")
            && interrupted.contains("[Partial report (interrupted)]"),
        "interrupt wait_agent results: {interrupted:?}"
    );
    let closed = tool_result_texts(&main[6]);
    let closed = closed.last().expect("close wait_agent result");
    assert!(
        closed.contains("/root/closer") && closed.contains("status=interrupted"),
        "close wait_agent results: {closed:?}"
    );
    assert!(
        !closed.contains("Partial report"),
        "close_agent must not produce a report: {closed:?}"
    );

    let interrupted_report = db
        .resolve_owned_subagent(&dir.path, &session_id, "/root/hanger")
        .expect("resolve hanger")
        .expect("hanger record")
        .report_json
        .as_deref()
        .and_then(SubagentReportView::from_json)
        .expect("partial report");
    assert!(
        interrupted_report
            .body
            .contains("[Partial report (interrupted)]"),
        "partial body: {}",
        interrupted_report.body
    );
    let closer = db
        .resolve_owned_subagent(&dir.path, &session_id, "/root/closer")
        .expect("resolve closer")
        .expect("closer record");
    assert!(
        closer.report_json.is_none(),
        "close must suppress the report: {:?}",
        closer.report_json
    );
}

// ===== 8. 嵌套 executor：wait all 不等自己、孙 agent 队列轮可续跑 =====

/// 空 `targets` 的 `wait_agent` 只能等调用者的严格后代：depth-1 executor 不得把自己算进
/// 等待集合（否则等自己的 done 信号永久死锁），且 depth-2 孙 agent 的排队输入必须在其
/// 首轮结束后按真实父会话续跑。
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn nested_executor_wait_all_and_grandchild_queued_input_succeed() {
    let dir = TestDir::new("subagent-nested");
    let db = Database::open_in_memory().expect("in-memory db");
    let spawn_coord = r#"{"role":"executor","title":"coordinate","task":"coordinate the leaf","task_name":"coord"}"#;
    let spawn_leaf = r#"{"role":"explorer","title":"leaf scan","task":"scan for the answer","task_name":"leaf"}"#;
    let send_leaf = r#"{"target":"leaf","message":"follow-up for leaf","trigger_turn":true}"#;
    let wait_all = r#"{"timeout_ms":10000}"#;
    let config = assemble(&db, &dir, "delegate to a coordinator", None);
    let factory = ScriptedFactory::new(
        vec![
            tool_step(&[("call-spawn-coord", "spawn_agent", spawn_coord)]),
            tool_step(&[("call-wait-coord", "wait_agent", wait_all)]),
            text_step("root finished"),
        ],
        vec![
            // /root/coord：派发孙 agent → 相对路径排队跟进输入 → 等全部后代。
            tool_step(&[
                ("call-spawn-leaf", "spawn_agent", spawn_leaf),
                ("call-send-leaf", "send_input", send_leaf),
                ("call-wait-leaf", "wait_agent", wait_all),
            ]),
            // 首轮故意跨过 send_input：确保排队输入落在运行中的孙 agent 上，而不是竞态下的 idle 拒绝。
            delayed(1500, text_step("leaf initial report")),
            text_step("leaf follow-up report"),
            text_step("coord report: leaf finished"),
        ],
    );

    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    let mut processor = HumanOutputProcessor::new(&mut stdout, &mut stderr, false, false, None);
    let (_cancel_tx, cancel_rx) = watch::channel(false);
    let summary = tokio::time::timeout(
        std::time::Duration::from_secs(30),
        run_exec_turn(&db, config, &mut processor, factory.clone(), cancel_rx),
    )
    .await
    .expect("nested wait_agent must not deadlock");

    assert_eq!(summary.outcome, TurnOutcome::Complete);
    assert_eq!(stdout, b"root finished\n");
    let requests = factory.requests();
    let subagent_requests: Vec<&ChatRequest> = requests
        .iter()
        .filter(|request| matches!(request.purpose, RequestPurpose::Subagent(_)))
        .collect();
    assert_eq!(
        subagent_requests.len(),
        4,
        "coord turn + leaf initial + leaf queued + coord report"
    );
    assert!(
        subagent_requests.iter().any(|request| request
            .items
            .iter()
            .any(|item| matches!(item, ModelItem::UserText(text) if text.contains("follow-up for leaf")))),
        "grandchild queued input must run its own turn"
    );
    let main = factory.main_requests();
    assert_eq!(main.len(), 3, "parent turns: {}", main.len());
    let waited = tool_result_texts(&main[2]);
    let waited = waited.last().expect("root wait_agent result");
    assert!(
        waited.contains("/root/coord") && waited.contains("coord report: leaf finished"),
        "root wait_agent results: {waited:?}"
    );
}
