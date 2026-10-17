//! 主流程端到端：write → read → 最终回答、同响应两工具串行。

use super::*;

// ===== 1. write → read → 最终回答 =====

#[tokio::test]
async fn write_then_read_then_final_answer_persists_in_order() {
    let fixture = Fixture::new("write-read");
    let (mut state, key, persistence, session) = streaming_state(&fixture);
    let write_args = r#"{"path":"note.txt","content":"from agent\n"}"#.to_string();
    let read_args = r#"{"path":"note.txt"}"#.to_string();
    let transport = FakeTransport::new(
        vec![
            tool_calls(
                &[
                    ("call_w", "write", &write_args, 0),
                    ("call_r", "read", &read_args, 1),
                ],
                Some((10, 2)),
            ),
            final_response("done", Some((4, 3))),
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

    // 整轮完成；终态恰好一条；usage 为两次采样之和。
    assert_eq!(driven.summary.outcome, TurnOutcome::Complete);
    let (reason, usage) = driven.finished.expect("exactly one terminal event");
    assert_eq!(reason, StopReason::Complete);
    assert_eq!(usage.input, Some(14), "10 + 4");
    assert_eq!(usage.output, Some(5), "2 + 3");
    assert_eq!(driven.unflushed, 0, "没有遗留未回执的 pending");

    // ① 可观测顺序：两个调用都先 begin（DB pending 已提交）再 finish。
    assert_eq!(
        driven.trace,
        vec![
            Trace::Begin("call_w".into()),
            Trace::Finish("call_w".into(), ToolTerminalStatus::Completed),
            Trace::Begin("call_r".into()),
            Trace::Finish("call_r".into(), ToolTerminalStatus::Completed),
        ],
        "pending 门槛先于执行、结果落库后于执行"
    );
    assert_eq!(
        driven.executed,
        vec!["call_w".to_string(), "call_r".to_string()]
    );

    // ② read 读到刚写入的内容（前一结果对后一调用可见）。
    assert_eq!(fixture.read("note.txt"), "from agent\n");

    // ③ 两个结果都入库、position 单调、结果体齐备。
    let rows = tool_rows(&fixture, &session);
    assert_eq!(rows.len(), 2, "两次调用各一行");
    assert_eq!(rows[0].call_id, "call_w");
    assert_eq!(rows[1].call_id, "call_r");
    assert!(
        rows[0].position < rows[1].position,
        "position 必须单调: {} {}",
        rows[0].position,
        rows[1].position
    );
    assert_eq!(rows[0].status, "completed");
    assert_eq!(rows[1].status, "completed");
    assert!(rows[0].result_json.is_some(), "结果体已落库");
    let read_json = rows[1].result_json.as_deref().expect("read result");
    assert!(
        read_json.contains("from agent"),
        "read 结果记录刚写入的内容: {read_json}"
    );

    // ④ 最终消息 complete；UI 步骤时间线保留两个已完成工具与摘要。
    let message = assistant_message(&state, &key);
    assert_eq!(message.status, "complete");
    let tools: Vec<&crate::app::state::ToolCallView> = message.tools().collect();
    assert_eq!(tools.len(), 2);
    assert!(
        tools
            .iter()
            .all(|tool| tool.status == ToolStatus::Completed)
    );
    assert!(
        tools[0]
            .summary
            .as_ref()
            .is_some_and(|summary| summary.path.is_some()),
        "UI 摘要来自 chat::tools::to_summary"
    );

    // ⑤ checkpoint/finish 未撞 PositionConflict：parts 与工具位置不重叠、且 position 单调。
    let parts = assistant_parts(&fixture, &session);
    let tool_positions: Vec<i64> = rows.iter().map(|row| row.position).collect();
    assert!(
        parts
            .iter()
            .all(|part| !tool_positions.contains(&part.position)),
        "part 与工具位置不得重叠"
    );
}

// ===== 2. 同响应两工具串行且 ordinal 顺序 =====

#[tokio::test]
async fn two_calls_in_one_response_run_serially_in_ordinal_order() {
    let fixture = Fixture::new("serial");
    let (mut state, key, persistence, session) = streaming_state(&fixture);
    let write_args = r#"{"path":"order.txt","content":"v"}"#.to_string();
    let read_args = r#"{"path":"order.txt"}"#.to_string();
    // 到达顺序故意与 ordinal 相反。
    let transport = FakeTransport::new(
        vec![
            tool_calls(
                &[
                    ("c_read", "read", &read_args, 1),
                    ("c_write", "write", &write_args, 0),
                ],
                None,
            ),
            final_response("ok", None),
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
    assert_eq!(
        driven.executed,
        vec!["c_write".to_string(), "c_read".to_string()],
        "执行顺序取供应商 ordinal，不是到达顺序"
    );
    // begin/finish 严格交替 = 串行（不可能并发）。
    assert_eq!(
        driven.trace,
        vec![
            Trace::Begin("c_write".into()),
            Trace::Finish("c_write".into(), ToolTerminalStatus::Completed),
            Trace::Begin("c_read".into()),
            Trace::Finish("c_read".into(), ToolTerminalStatus::Completed),
        ]
    );
    let rows = tool_rows(&fixture, &session);
    assert_eq!(rows[0].call_id, "c_write");
    assert_eq!(rows[1].call_id, "c_read");
    assert!(rows[0].position < rows[1].position);
    // read 的结果含写入内容（前一调用的结果对后一调用可见）。
    let read_json = rows[1].result_json.as_deref().expect("read result");
    assert!(
        read_json.contains("\"content\":\"1: v"),
        "read 返回刚写入的内容: {read_json}"
    );
}
