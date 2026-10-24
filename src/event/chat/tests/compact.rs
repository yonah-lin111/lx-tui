//! `/compact` 事件层端到端测试：真实 DB + 本地 mock SSE 服务。
//!
//! 覆盖 design §4.2 与 §6 的关键不变量：
//! 1. 摘要请求真实发出（系统提示、无工具），成功后在**一个**事务里写检查点；
//! 2. 失败（网络/协议/空摘要）时零写入、revision 不变、消息不动；
//! 3. 幂等：同一 `operation_id` 重复完成回执只写一条检查点；
//! 4. 撤销只切检查点状态与 revision，不触碰消息与文件。
//!
//! 历史 bug 的回归点：摘要请求曾用 `watch::channel(false).1`（丢弃 sender），
//! `run_stream` 的取消 select 立刻返回，导致 **每一次** `/compact` 都失败并报
//! `summary request failed: cancelled`。见 [`compact_commits_a_checkpoint`]。

use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

use super::*;
use crate::app::effects::CompactRequest;
use crate::db::chat::CompactCommit;
use crate::event::chat::compact::CompactOutcome;

/// 启动一次性 mock SSE 服务：读一个请求、回固定响应、关闭。
async fn spawn_mock(status: &str, body: &'static str) -> String {
    let listener = TcpListener::bind(("127.0.0.1", 0)).await.expect("bind");
    let port = listener.local_addr().expect("addr").port();
    let status = status.to_string();
    tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.expect("accept");
        let mut buffer = Vec::new();
        let mut chunk = [0_u8; 4096];
        loop {
            let read = stream.read(&mut chunk).await.expect("read");
            if read == 0 {
                break;
            }
            buffer.extend_from_slice(&chunk[..read]);
            if let Some(header_end) = buffer.windows(4).position(|window| window == b"\r\n\r\n") {
                let header = String::from_utf8_lossy(&buffer[..header_end]).into_owned();
                let content_length = header
                    .lines()
                    .find_map(|line| {
                        let (name, value) = line.split_once(':')?;
                        if name.eq_ignore_ascii_case("content-length") {
                            value.trim().parse::<usize>().ok()
                        } else {
                            None
                        }
                    })
                    .unwrap_or(0);
                if buffer.len() >= header_end + 4 + content_length {
                    break;
                }
            }
        }
        let response = format!(
            "HTTP/1.1 {status}\r\nContent-Type: text/event-stream\r\nConnection: close\r\nContent-Length: {}\r\n\r\n{}",
            body.len(),
            body
        );
        let _ = stream.write_all(response.as_bytes()).await;
        let _ = stream.shutdown().await;
    });
    format!("http://127.0.0.1:{port}/v1")
}

/// 一段正常的 OpenAI 兼容摘要流。
fn summary_stream() -> &'static str {
    concat!(
        "data: {\"choices\":[{\"delta\":{\"content\":\"- goal: ship the harness\\n\"}}]}\n\n",
        "data: {\"choices\":[{\"delta\":{}, \"finish_reason\":\"stop\"}]}\n\n",
        "data: [DONE]\n\n",
    )
}

/// 把一次压缩请求作为效应交给事件层执行（`/compact` 的真实出箱路径）。
fn execute(
    request: &CompactRequest,
    sender: &mpsc::UnboundedSender<AppEvent>,
    persistence: &Persistence,
) -> bool {
    let mut state = AppState::demo();
    state
        .effects
        .push(crate::app::effects::Effect::ExecuteCompact(request.clone()));
    apply_compact_effects(&mut state, sender, persistence)
}

/// 建一个带完整一轮（user + complete assistant）的会话，返回其 id。
fn session_with_one_turn(fixture: &Fixture) -> String {
    let database = fixture.database();
    let committed = database
        .begin_turn(
            fixture.workspace(),
            None,
            "please add the feature",
            &crate::db::chat::ModelSnapshot {
                provider: "p".into(),
                model: "m".into(),
                variant: None,
            },
        )
        .expect("begin_turn");
    database
        .finish(
            fixture.workspace(),
            &committed.assistant_message_id,
            &[crate::db::chat::ChatPart {
                position: 0,
                kind: "text".into(),
                content: "done".into(),
                provider_metadata_json: None,
                started_at_ms: None,
                ended_at_ms: None,
                completed_at_ms: None,
            }],
            &crate::db::chat::TurnOutcome::Complete { stop_reason: None },
            Default::default(),
        )
        .expect("finish");
    committed.session_id
}

/// 压缩请求（真实模型快照 + 指向 mock 服务的 transport）。
fn compact_request(
    fixture: &Fixture,
    session_id: &str,
    revision: u64,
    base_url: String,
) -> CompactRequest {
    CompactRequest {
        pane: crate::layout::PaneId::alloc(),
        workspace: fixture.workspace().to_path_buf(),
        session_id: session_id.to_string(),
        expected_revision: revision,
        base_checkpoint_id: None,
        model: crate::db::chat::ModelSnapshot {
            provider: "p".into(),
            model: "m".into(),
            variant: None,
        },
        model_label: "m".into(),
        transport: "openai-compatible".into(),
        base_url,
        api_key: "test-key".into(),
        variant_params: None,
        idle_timeout_ms: 5_000,
        operation_id: crate::db::new_external_id("compact-op"),
    }
}

/// 成功路径：摘要请求真正发出 → 检查点落库 → revision 前进。
///
/// 这条测试直接锁死历史 bug：cancel 通道只留 receiver 时这里会是
/// `Failed { message: "summary request failed: cancelled" }`。
#[tokio::test]
async fn compact_commits_a_checkpoint() {
    let fixture = Fixture::new("compact-commit");
    let session_id = session_with_one_turn(&fixture);
    let database = fixture.database();
    let revision = database.session_revision(&session_id).expect("revision") as u64;
    let base_url = spawn_mock("200 OK", summary_stream()).await;
    let request = compact_request(&fixture, &session_id, revision, base_url);
    let operation_id = request.operation_id.clone();
    let (sender, mut receiver) = mpsc::unbounded_channel::<AppEvent>();
    // 事件层用文件 DB 路径在后台开新连接（与生产一致）。
    let persistence = Persistence::with_database_at(database, fixture.db_path());

    assert!(execute(&request, &sender, &persistence));
    let outcome = tokio::time::timeout(Duration::from_secs(10), receiver.recv())
        .await
        .expect("compact 必须在超时前回投")
        .expect("compact 事件");
    let AppEvent::CompactFinished { outcome, .. } = outcome else {
        panic!("expected CompactFinished");
    };
    assert_eq!(outcome, CompactOutcome::Committed, "摘要成功必须提交检查点");

    let reopened = fixture.reopen();
    let checkpoints = reopened
        .compact_checkpoints(fixture.workspace(), &session_id)
        .expect("checkpoints");
    assert_eq!(checkpoints.len(), 1, "只写一条检查点");
    assert_eq!(checkpoints[0].operation_id, operation_id);
    assert!(checkpoints[0].is_active());
    assert!(checkpoints[0].summary_text.contains("ship the harness"));
    assert_eq!(checkpoints[0].model_label, "m");
    assert_eq!(checkpoints[0].start_sequence, 0, "覆盖整轮");
    assert_eq!(checkpoints[0].end_sequence, 1);
    assert_eq!(
        reopened.session_revision(&session_id).expect("revision"),
        i64::try_from(revision + 1).expect("revision fits"),
        "提交检查点必须推进 revision"
    );
}

/// 协议失败（HTTP 401，认证类错误不重试）：不写检查点、revision 不变。
#[tokio::test]
async fn failed_summary_writes_nothing() {
    let fixture = Fixture::new("compact-fail");
    let session_id = session_with_one_turn(&fixture);
    let database = fixture.database();
    let revision = database.session_revision(&session_id).expect("revision") as u64;
    // 用 4xx 而不是 5xx：5xx 属于临时故障，会走 10 次指数退避重试（2s→10s）。
    let base_url = spawn_mock("401 Unauthorized", "{\"error\":\"bad key\"}").await;
    let request = compact_request(&fixture, &session_id, revision, base_url);
    let (sender, mut receiver) = mpsc::unbounded_channel::<AppEvent>();
    let persistence = Persistence::with_database_at(database, fixture.db_path());

    assert!(execute(&request, &sender, &persistence));
    let AppEvent::CompactFinished { outcome, .. } =
        tokio::time::timeout(Duration::from_secs(10), receiver.recv())
            .await
            .expect("compact 必须在超时前回投")
            .expect("compact 事件")
    else {
        panic!("expected CompactFinished");
    };
    let CompactOutcome::Failed { message } = outcome else {
        panic!("HTTP 401 必须失败，实际 {outcome:?}");
    };
    assert!(
        !message.contains("cancelled"),
        "失败原因是真实协议错误，不是假取消：{message}"
    );

    let reopened = fixture.reopen();
    assert!(
        reopened
            .compact_checkpoints(fixture.workspace(), &session_id)
            .expect("checkpoints")
            .is_empty(),
        "失败不得写检查点"
    );
    assert_eq!(
        reopened.session_revision(&session_id).expect("revision"),
        i64::try_from(revision).expect("revision fits"),
        "失败不得推进 revision"
    );
    assert_eq!(
        reopened
            .load_chat_session(fixture.workspace(), &session_id)
            .expect("load")
            .expect("session")
            .len(),
        2,
        "原始消息原样保留"
    );
}

/// 空摘要不落库：空压缩会静默丢掉整段上下文。
#[tokio::test]
async fn empty_summary_is_rejected() {
    let fixture = Fixture::new("compact-empty");
    let session_id = session_with_one_turn(&fixture);
    let database = fixture.database();
    let revision = database.session_revision(&session_id).expect("revision") as u64;
    let body = concat!(
        "data: {\"choices\":[{\"delta\":{\"content\":\"   \"}}]}\n\n",
        "data: {\"choices\":[{\"delta\":{}, \"finish_reason\":\"stop\"}]}\n\n",
        "data: [DONE]\n\n",
    );
    let base_url = spawn_mock("200 OK", body).await;
    let request = compact_request(&fixture, &session_id, revision, base_url);
    let (sender, mut receiver) = mpsc::unbounded_channel::<AppEvent>();
    let persistence = Persistence::with_database_at(database, fixture.db_path());

    assert!(execute(&request, &sender, &persistence));
    let AppEvent::CompactFinished { outcome, .. } =
        tokio::time::timeout(Duration::from_secs(10), receiver.recv())
            .await
            .expect("compact 必须在超时前回投")
            .expect("compact 事件")
    else {
        panic!("expected CompactFinished");
    };
    assert!(
        matches!(&outcome, CompactOutcome::Failed { message } if message.contains("empty")),
        "空白摘要必须失败：{outcome:?}"
    );
    assert!(
        fixture
            .reopen()
            .compact_checkpoints(fixture.workspace(), &session_id)
            .expect("checkpoints")
            .is_empty(),
        "空摘要不得写检查点"
    );
}

/// 工具调用出现在摘要流里：整体失败，不执行、不落库。
#[tokio::test]
async fn summary_with_tool_calls_is_rejected() {
    let fixture = Fixture::new("compact-tools");
    let session_id = session_with_one_turn(&fixture);
    let database = fixture.database();
    let revision = database.session_revision(&session_id).expect("revision") as u64;
    let body = concat!(
        "data: {\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":0,\"id\":\"call_a\",\"function\":{\"name\":\"write\",\"arguments\":\"{}\"}}]}}]}\n\n",
        "data: {\"choices\":[{\"delta\":{}, \"finish_reason\":\"tool_calls\"}]}\n\n",
        "data: [DONE]\n\n",
    );
    let base_url = spawn_mock("200 OK", body).await;
    let request = compact_request(&fixture, &session_id, revision, base_url);
    let (sender, mut receiver) = mpsc::unbounded_channel::<AppEvent>();
    let persistence = Persistence::with_database_at(database, fixture.db_path());

    assert!(execute(&request, &sender, &persistence));
    let AppEvent::CompactFinished { outcome, .. } =
        tokio::time::timeout(Duration::from_secs(10), receiver.recv())
            .await
            .expect("compact 必须在超时前回投")
            .expect("compact 事件")
    else {
        panic!("expected CompactFinished");
    };
    assert!(
        matches!(outcome, CompactOutcome::Failed { .. }),
        "摘要请求出现工具调用必须失败：{outcome:?}"
    );
    assert!(
        fixture
            .reopen()
            .compact_checkpoints(fixture.workspace(), &session_id)
            .expect("checkpoints")
            .is_empty(),
        "工具调用不得写检查点"
    );
}

/// 幂等：同一 `operation_id` 重复提交只写一条、revision 只推进一次。
#[test]
fn repeated_commit_with_same_operation_is_idempotent() {
    let fixture = Fixture::new("compact-idempotent");
    let session_id = session_with_one_turn(&fixture);
    let database = fixture.database();
    let revision = database.session_revision(&session_id).expect("revision");
    let commit = CompactCommit {
        session_id: session_id.clone(),
        expected_revision: revision,
        base_checkpoint_id: None,
        start_sequence: 0,
        end_sequence: 1,
        summary_text: "- goal: ship".into(),
        model_label: "m".into(),
        operation_id: "compact-op-fixed".into(),
    };
    let first = database
        .commit_compact(fixture.workspace(), &commit)
        .expect("first commit");
    assert!(first.created);
    let second = database
        .commit_compact(fixture.workspace(), &commit)
        .expect("replay");
    assert!(!second.created, "重放不得再写一条");
    assert_eq!(second.checkpoint.id, first.checkpoint.id);
    assert_eq!(
        database
            .compact_checkpoints(fixture.workspace(), &session_id)
            .expect("checkpoints")
            .len(),
        1
    );
    assert_eq!(
        database.session_revision(&session_id).expect("revision"),
        revision + 1,
        "revision 只推进一次"
    );
}

/// 迟到回执：同一会话上已经开始了新一次压缩时，旧 `operation_id` 的失败不得清执行态。
#[test]
fn late_compact_failure_for_another_operation_is_ignored() {
    let fixture = Fixture::new("compact-late-failure");
    let persistence = Persistence::with_database(fixture.database());
    let mut state = AppState::demo();
    let pane = *state.all_pane_ids().first().expect("pane");
    {
        let target = state.pane_mut_anywhere(pane).expect("pane");
        target.view = crate::app::state::PaneView::Lx;
        target.lx.chat.session_id = Some("s-late".into());
        target.lx.compact.running = Some("op-b".into());
    }
    let handled = handle_compact_finished(
        &mut state,
        &persistence,
        pane,
        "s-late",
        fixture.workspace(),
        "op-a",
        CompactOutcome::Failed {
            message: "old failure".into(),
        },
    );
    assert!(!handled, "旧 operation 的回执必须被丢弃");
    let target = state.pane_anywhere(pane).expect("pane");
    assert_eq!(
        target.lx.compact.running.as_deref(),
        Some("op-b"),
        "不得清掉正在执行的新一次压缩"
    );
}
