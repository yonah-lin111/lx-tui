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

    // 整轮完成；终态恰好一条；usage 只取最后一次采样（上下文占用口径）。
    assert_eq!(driven.summary.outcome, TurnOutcome::Complete);
    let (reason, usage) = driven.finished.expect("exactly one terminal event");
    assert_eq!(reason, StopReason::Complete);
    assert_eq!(usage.input, Some(4), "只取最后一步，不累加 10 + 4");
    assert_eq!(usage.output, Some(3), "只取最后一步，不累加 2 + 3");
    // 账单口径仍然跨步累计：CLI/会话统计不能被单步上下文覆盖。
    assert_eq!(driven.summary.billing.input, Some(14), "10 + 4");
    assert_eq!(driven.summary.billing.output, Some(5), "2 + 3");
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

// ===== 3. 同一响应 4 个只读调用：事件层/DB 的多 running 握手 =====

/// 同一响应内 4 个 `read` 必须全部先 begin（位置稠密 0..3），再按 ordinal finish。
///
/// 这是并行批次在真实事件层 + DB 上的最小集成回归：编排层测试用的 `RecordingHost`
/// 不分配 timeline position，无法发现“多张 running 卡片”与位置分配的回归。
#[tokio::test]
async fn four_reads_in_one_response_create_dense_tool_positions() {
    let fixture = Fixture::new("four-reads");
    for name in ["a.txt", "b.txt", "c.txt", "d.txt"] {
        std::fs::write(fixture.workspace().join(name), format!("{name}\n")).expect("seed file");
    }
    let (mut state, key, persistence, session) = streaming_state(&fixture);
    let a = r#"{"path":"a.txt"}"#.to_string();
    let b = r#"{"path":"b.txt"}"#.to_string();
    let c = r#"{"path":"c.txt"}"#.to_string();
    let d = r#"{"path":"d.txt"}"#.to_string();
    let transport = FakeTransport::new(
        vec![
            tool_calls(
                &[
                    ("r0", "read", &a, 0),
                    ("r1", "read", &b, 1),
                    ("r2", "read", &c, 2),
                    ("r3", "read", &d, 3),
                ],
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
    assert_eq!(driven.unflushed, 0, "四个 begin 都必须回执");
    let mut executed = driven.executed.clone();
    executed.sort();
    assert_eq!(executed, vec!["r0", "r1", "r2", "r3"], "四个调用都执行");
    assert_eq!(
        driven.trace,
        vec![
            Trace::Begin("r0".into()),
            Trace::Begin("r1".into()),
            Trace::Begin("r2".into()),
            Trace::Begin("r3".into()),
            Trace::Finish("r0".into(), ToolTerminalStatus::Completed),
            Trace::Finish("r1".into(), ToolTerminalStatus::Completed),
            Trace::Finish("r2".into(), ToolTerminalStatus::Completed),
            Trace::Finish("r3".into(), ToolTerminalStatus::Completed),
        ],
        "批次必须先批量 begin，再按 ordinal finish"
    );
    let rows = tool_rows(&fixture, &session);
    assert_eq!(rows.len(), 4, "每个调用各一行");
    let positions: Vec<i64> = rows.iter().map(|row| row.position).collect();
    assert!(
        positions.windows(2).all(|pair| pair[1] == pair[0] + 1),
        "四个工具位置必须稠密连续: {positions:?}"
    );
    assert!(
        rows.iter()
            .all(|row| row.status == "completed" && row.result_json.is_some()),
        "四个结果都必须落库: {rows:?}"
    );
    let message = assistant_message(&state, &key);
    let tools: Vec<&crate::app::state::ToolCallView> = message.tools().collect();
    assert_eq!(tools.len(), 4);
    assert!(
        tools
            .iter()
            .all(|tool| tool.status == ToolStatus::Completed),
        "四张卡片都必须进入终态"
    );
}

// ===== 4. Gemini 多步工具链：跨采样的合成 call ID 不得撞库（G8 回归） =====

/// 把一段 Google SSE 帧喂给**真实 Google 解码器**，收集它产出的增量。
///
/// 每次调用都新建解码器（与 `client::stream_once` 一致），序号来自生产用的
/// 进程内计数器（[`crate::chat::transport::next_response_seq`]），因此这里
/// 走的就是生产 ID 生成路径，不是测试自造的 call ID。
fn google_decoder_step(frames: &[&str]) -> Vec<ChatDelta> {
    let mut decoder =
        crate::chat::transport::decoder_for("google", crate::chat::transport::next_response_seq());
    let mut deltas = Vec::new();
    for frame in frames {
        deltas.extend(decoder.decode_frame(frame).expect("google frame decodes"));
    }
    deltas.extend(decoder.finish());
    deltas
}

/// 构造只有一个 functionCall 的 Google 帧（`finishReason=STOP` = 本次响应结束）。
fn google_step(name: &str, args: &str) -> Vec<ChatDelta> {
    let args: serde_json::Value = serde_json::from_str(args).expect("args json");
    let frame = serde_json::json!({
        "candidates": [{
            "content": {"parts": [{"functionCall": {"name": name, "args": args}}], "role": "model"},
            "finishReason": "STOP",
        }],
    })
    .to_string();
    google_decoder_step(&[&frame])
}

/// **G8 端到端回归**：Gemini 下同一个工具在第 1、3 步各被调用一次
/// （read → write → read），整轮必须成功。
///
/// 修复前，两次采样都合成 `gemini-0-0-read`：第 3 步的 `begin_tool_call`
/// 撞上 `UNIQUE(session_id, call_id)` 返回 `ToolCallConflict`，整轮失败。
#[tokio::test]
async fn google_repeated_tool_across_steps_does_not_collide_in_db() {
    let fixture = Fixture::new("google-repeat-tool");
    std::fs::write(fixture.workspace().join("note.txt"), "from agent\n").expect("seed file");
    let (mut state, key, persistence, session) = streaming_state(&fixture);
    // 三步：read（响应 1）→ write（响应 2）→ read（响应 3，与第 1 步同名同形状）→ 最终回答。
    let transport = FakeTransport::new(
        vec![
            google_step("read", r#"{"path":"note.txt"}"#),
            google_step("write", r#"{"path":"note.txt","content":"from agent2\n"}"#),
            google_step("read", r#"{"path":"note.txt"}"#),
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

    assert_eq!(
        driven.summary.outcome,
        TurnOutcome::Complete,
        "多步工具链不得因 call ID 撞库而整轮失败：{:?}",
        driven.summary
    );
    let rows = tool_rows(&fixture, &session);
    assert_eq!(rows.len(), 3, "read / write / read 各一行：{rows:?}");
    assert_eq!(rows[0].name, "read");
    assert_eq!(rows[1].name, "write");
    assert_eq!(rows[2].name, "read");
    assert_ne!(
        rows[0].call_id, rows[2].call_id,
        "跨模型采样的同名调用不得复用 call ID"
    );
    assert!(
        rows[0].position < rows[1].position && rows[1].position < rows[2].position,
        "position 必须单调"
    );
    // 第 3 步读到第 2 步写入后的内容（工具链真的跑了第二遍）。
    let last = rows[2].result_json.as_deref().expect("read result");
    assert!(
        last.contains("from agent2"),
        "重读应看到 write 之后的内容: {last}"
    );
    // 合成 ID 不得进入发给供应商的请求体（Gemini 无原生 ID）。
    let requests = transport.requests();
    let last_request = requests.last().expect("final request");
    let built = crate::chat::transport::build_request(
        "google",
        "https://example.invalid",
        Some("k"),
        last_request,
    )
    .expect("build google request");
    assert!(built.body.contains("\"functionResponse\""));
    assert!(!built.body.contains("\"call_id\""));
    for row in &rows {
        assert!(
            !built.body.contains(&row.call_id),
            "合成 call ID {} 泄漏进 Google 请求体",
            row.call_id
        );
    }
}
