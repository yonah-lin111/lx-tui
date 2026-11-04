//! `exec` 子 agent 协同测试：spawn / wait、并发上限、打断续跑与自动续等。
//!
//! 夹具与主轮测试共用 `fixtures`。

use tokio::sync::watch;

use super::super::{HumanOutputProcessor, run_exec_turn};
use super::fixtures::{
    ScriptedFactory, Step, TestDir, assemble, delayed, session_messages, text_step,
    tool_result_texts, tool_step,
};
use crate::chat::agent_loop::TurnOutcome;
use crate::chat::events::RequestPurpose;
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
