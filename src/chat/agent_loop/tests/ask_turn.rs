//! `ask` 工具在 loop 里的三条路径：成功作答、参数非法、整轮取消。
//!
//! 这里的 host 是脚本化的，答案由测试直接回执——不触网、不碰 UI，只证明编排顺序
//! （BeginTool → AskUser → FinishTool）与取消/失败语义。

use super::*;

const ASK_ARGS: &str = r#"{"questions":[{"id":"layout_choice","header":"布局","question":"侧栏用哪种布局？","options":[{"label":"双栏","description":"左右并排"}]}]}"#;

/// 成功：提问夹在 begin/finish 之间，答案作为工具结果进入下一轮上下文。
#[tokio::test]
async fn ask_answers_become_tool_result_for_next_request() {
    let fixture = Fixture::new("ask-ok");
    let script = Script::new(
        vec![
            tool_calls(&[("c1", "ask", ASK_ARGS, 0)], None),
            final_response("done", None),
        ],
        StopReason::Complete,
    );
    let mut answers = AskAnswers::new();
    answers.insert("layout_choice".into(), vec!["双栏".into()]);
    let host = RecordingHost::answering(answers);
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
    assert_eq!(
        host.trace(),
        vec![
            HostTrace::Begin("c1".into()),
            HostTrace::Ask("c1".into(), 1),
            HostTrace::Finish("c1".into(), false),
        ],
        "AskUser 必须夹在 BeginTool 与 FinishTool 之间"
    );
    assert_eq!(script.request_count(), 2, "作答后继续采样");
    let result = script
        .items_of(1)
        .into_iter()
        .find_map(|item| match item {
            ModelItem::ToolResult {
                call_id,
                content,
                is_error,
            } if call_id == "c1" => Some((content, is_error)),
            _ => None,
        })
        .expect("ask result present");
    assert!(!result.1, "作答成功不是错误结果");
    assert!(
        result.0.contains("layout_choice") && result.0.contains("双栏"),
        "下一轮上下文必须含答案 JSON: {}",
        result.0
    );
}

/// 参数非法：不打扰用户，失败结果落库后轮次继续采样。
#[tokio::test]
async fn invalid_ask_arguments_fail_tool_but_keep_the_turn() {
    let fixture = Fixture::new("ask-invalid");
    let script = Script::new(
        vec![
            tool_calls(&[("c1", "ask", r#"{"questions":[]}"#, 0)], None),
            final_response("recovered", None),
        ],
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
    assert_eq!(
        host.trace(),
        vec![
            HostTrace::Begin("c1".into()),
            HostTrace::Finish("c1".into(), false),
        ],
        "非法参数不得弹面板"
    );
    assert_eq!(script.request_count(), 2, "失败结果回给模型后继续");
    assert!(
        script.items_of(1).iter().any(|item| matches!(
            item,
            ModelItem::ToolResult {
                call_id,
                is_error: true,
                ..
            } if call_id == "c1"
        )),
        "非法参数以错误结果进入上下文"
    );
}

/// 取消：本轮立即收尾，已开始的调用按 cancelled 结算，未开始的也落 cancelled。
#[tokio::test]
async fn cancelled_ask_interrupts_turn_and_settles_remaining_calls() {
    let fixture = Fixture::new("ask-cancel");
    let second_args = r#"{"path":"never.txt","content":"x"}"#;
    let script = Script::new(
        vec![tool_calls(
            &[("c1", "ask", ASK_ARGS, 0), ("c2", "write", second_args, 1)],
            None,
        )],
        StopReason::Complete,
    );
    let host = RecordingHost::ask_failing(HostError::Cancelled);
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
    assert_eq!(summary.outcome, TurnOutcome::Interrupted);
    assert_eq!(
        host.trace(),
        vec![
            HostTrace::Begin("c1".into()),
            HostTrace::Ask("c1".into(), 1),
            HostTrace::Finish("c1".into(), true),
            HostTrace::Begin("c2".into()),
            HostTrace::Finish("c2".into(), true),
        ],
        "取消只结算这一行，未开始的调用按 cancelled 落库"
    );
    assert_eq!(script.request_count(), 1, "取消后不再采样");
    assert!(
        executed.lock().expect("lock").is_empty(),
        "未开始的调用绝不执行"
    );
    assert!(!fixture.exists("never.txt"), "无文件副作用");
}
