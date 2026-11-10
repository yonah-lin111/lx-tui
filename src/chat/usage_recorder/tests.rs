//! `usage_recorder` 单元测试：验证增量 Usage 合并、5 类用途推导、3 类终态映射、单价成本计算与流式自动落库。

use std::io::{Read, Write};
use std::net::TcpListener;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use super::*;
use crate::chat::agent::AgentRole;
use crate::chat::client::{run_stream_with_log, uncancellable};
use crate::chat::events::{ChatTurn, PartDelta, PartKind};
use crate::db::Database;
use crate::db::chat::ModelSnapshot;
use crate::db::usage::UsageQuery;

fn sample_request(purpose: RequestPurpose) -> ChatRequest {
    ChatRequest {
        turns: vec![ChatTurn {
            role: "user".to_string(),
            text: "hello".to_string(),
            reasoning_metadata: None,
        }],
        items: Vec::new(),
        model: ModelSnapshot {
            provider: "anthropic".to_string(),
            model: "claude-sonnet-4-5".to_string(),
            variant: None,
        },
        variant_params: None,
        purpose,
        system_prompt: None,
        tools: Arc::new(Vec::new()),
    }
}

fn approx_eq(actual: Option<f64>, expected: f64) {
    let val = actual.expect("expected Some(f64)");
    assert!(
        (val - expected).abs() < 1e-9,
        "expected {expected}, got {val}"
    );
}

#[test]
fn extract_step_usage_merges_split_frames_and_overwrites_non_none_fields() {
    assert_eq!(extract_step_usage(&[]), Usage::default());

    let deltas = vec![
        ChatDelta::Part(PartDelta {
            kind: PartKind::Text,
            text: "hello".to_string(),
            metadata: None,
        }),
        ChatDelta::Usage(Usage {
            input: Some(1_200),
            output: None,
            cache_read: Some(800),
            cache_write: Some(100),
        }),
        ChatDelta::Usage(Usage {
            input: None,
            output: Some(350),
            cache_read: None,
            cache_write: None,
        }),
        ChatDelta::Usage(Usage {
            input: None,
            output: Some(420),
            cache_read: Some(850),
            cache_write: None,
        }),
    ];

    assert_eq!(
        extract_step_usage(&deltas),
        Usage {
            input: Some(1_200),
            output: Some(420),
            cache_read: Some(850),
            cache_write: Some(100),
        }
    );
}

#[test]
fn resolve_purpose_covers_all_five_categories_and_prioritizes_compaction() {
    let chat_req = sample_request(RequestPurpose::Chat);
    let exec_req = sample_request(RequestPurpose::ExecTurn);
    let subagent_req = sample_request(RequestPurpose::Subagent(AgentRole::Explorer));
    let compact_req = sample_request(RequestPurpose::CompactSummary);

    // 1. Chat：主会话与 CLI exec 均归入 Chat。
    assert_eq!(
        resolve_purpose(
            &StreamLog {
                kind: "chat",
                ..StreamLog::default()
            },
            &chat_req
        ),
        UsagePurpose::Chat
    );
    assert_eq!(
        resolve_purpose(
            &StreamLog {
                kind: "exec",
                ..StreamLog::default()
            },
            &exec_req
        ),
        UsagePurpose::Chat
    );

    // 2. Subagent：通过 log.kind == "subagent" 或 RequestPurpose::Subagent 识别。
    assert_eq!(
        resolve_purpose(
            &StreamLog {
                kind: "subagent",
                ..StreamLog::default()
            },
            &chat_req
        ),
        UsagePurpose::Subagent
    );
    assert_eq!(
        resolve_purpose(
            &StreamLog {
                kind: "chat",
                ..StreamLog::default()
            },
            &subagent_req
        ),
        UsagePurpose::Subagent
    );

    // 3. Compaction：手动压缩（log.kind == "compact"）与子代理自动压缩（log.kind == "subagent" + RequestPurpose::CompactSummary）。
    assert_eq!(
        resolve_purpose(
            &StreamLog {
                kind: "compact",
                ..StreamLog::default()
            },
            &chat_req
        ),
        UsagePurpose::Compaction
    );
    assert_eq!(
        resolve_purpose(
            &StreamLog {
                kind: "subagent",
                ..StreamLog::default()
            },
            &compact_req
        ),
        UsagePurpose::Compaction
    );

    // 4. Title：会话标题生成。
    assert_eq!(
        resolve_purpose(
            &StreamLog {
                kind: "title",
                ..StreamLog::default()
            },
            &chat_req
        ),
        UsagePurpose::Title
    );

    // 5. Recap：轮次摘要生成。
    assert_eq!(
        resolve_purpose(
            &StreamLog {
                kind: "recap",
                ..StreamLog::default()
            },
            &chat_req
        ),
        UsagePurpose::Recap
    );
}

#[test]
fn resolve_status_maps_complete_cancelled_and_failed() {
    assert_eq!(
        resolve_status(&StopReason::Complete),
        (UsageStatus::Success, None)
    );
    assert_eq!(
        resolve_status(&StopReason::Cancelled),
        (UsageStatus::Aborted, None)
    );
    assert_eq!(
        resolve_status(&StopReason::Failed {
            code: "http 429 · rate limit".to_string()
        }),
        (
            UsageStatus::Error,
            Some("http 429 · rate limit".to_string())
        )
    );
}

#[test]
fn record_stream_to_db_resolves_workspace_and_computes_pricing_or_null_costs() {
    let db = Database::open_in_memory().expect("in-memory db");
    let workspace = PathBuf::from("/tmp/lx-usage-recorder-ws");
    db.trust_workspace(&workspace, "Recorder Workspace")
        .expect("trust workspace");
    let model = ModelSnapshot {
        provider: "anthropic".to_string(),
        model: "claude-sonnet-4-5".to_string(),
        variant: None,
    };
    let turn = db
        .begin_turn(&workspace, None, "build dashboard", &model)
        .expect("begin turn");

    let log = StreamLog {
        kind: "chat",
        session_id: Some(turn.session_id.clone()),
        turn_index: Some(turn.turn_index),
        ..StreamLog::default()
    };
    let request = sample_request(RequestPurpose::Chat);
    let deltas = vec![
        ChatDelta::Usage(Usage {
            input: Some(10_000),
            output: None,
            cache_read: Some(6_000),
            cache_write: Some(1_000),
        }),
        ChatDelta::Usage(Usage {
            input: None,
            output: Some(2_000),
            cache_read: None,
            cache_write: None,
        }),
    ];
    let pricing = ModelPricing {
        input: Some(3.0),
        output: Some(15.0),
        cache_read: Some(0.3),
        cache_write: Some(3.75),
    };

    record_stream_to_db(
        &db,
        &log,
        &request,
        &deltas,
        &StopReason::Complete,
        1_850,
        Some(pricing),
    )
    .expect("record priced stream");

    // 第二条请求：无单价 + 失败状态。
    let failed_log = StreamLog {
        kind: "title",
        session_id: Some(turn.session_id.clone()),
        ..StreamLog::default()
    };
    record_stream_to_db(
        &db,
        &failed_log,
        &request,
        &[],
        &StopReason::Failed {
            code: "http 503".to_string(),
        },
        420,
        None,
    )
    .expect("record unpriced failed stream");

    let page = db
        .usage_logs(&UsageQuery::default(), 1, 10)
        .expect("query usage logs");
    assert_eq!(page.total, 2);

    let priced_row = page
        .rows
        .iter()
        .find(|row| row.purpose == UsagePurpose::Chat)
        .expect("chat row");
    assert_eq!(
        priced_row.session_id.as_deref(),
        Some(turn.session_id.as_str())
    );
    assert!(
        priced_row.workspace_id.is_some(),
        "workspace_id 应自动从 agent_session 反查补齐"
    );
    assert_eq!(
        priced_row.tokens,
        UsageTokens {
            input: 10_000,
            output: 2_000,
            cache_read: 6_000,
            cache_write: 1_000,
        }
    );
    // fresh_input = 10_000 - 6_000 - 1_000 = 3_000 -> $0.009
    approx_eq(priced_row.rates.input_cost_usd, 0.009);
    // output = 2_000 -> $0.030
    approx_eq(priced_row.rates.output_cost_usd, 0.030);
    // cache_read = 6_000 -> $0.0018
    approx_eq(priced_row.rates.cache_read_cost_usd, 0.0018);
    // cache_write = 1_000 -> $0.00375
    approx_eq(priced_row.rates.cache_write_cost_usd, 0.00375);
    approx_eq(priced_row.rates.total_cost_usd, 0.04455);
    assert_eq!(priced_row.duration_ms, Some(1_850));
    assert_eq!(priced_row.status, UsageStatus::Success);
    assert_eq!(priced_row.error_message, None);

    let unpriced_row = page
        .rows
        .iter()
        .find(|row| row.purpose == UsagePurpose::Title)
        .expect("title row");
    assert_eq!(unpriced_row.rates, UsageRates::default());
    assert_eq!(unpriced_row.status, UsageStatus::Error);
    assert_eq!(unpriced_row.error_message.as_deref(), Some("http 503"));
}

#[test]
fn run_stream_with_log_automatically_persists_usage_log_when_test_db_is_set() {
    static SEQ: AtomicU64 = AtomicU64::new(0);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let temp_dir = std::env::temp_dir().join(format!(
        "lx-usage-stream-test-{nanos:x}-{}",
        SEQ.fetch_add(1, Ordering::Relaxed)
    ));
    let db_path = temp_dir.join("lx.db");

    // 启动本地一次性 SSE 服务模拟 Anthropic 流式返回。
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind mock sse server");
    let addr = listener.local_addr().expect("local addr");
    let server = std::thread::spawn(move || {
        if let Ok((mut stream, _)) = listener.accept() {
            let mut buf = [0u8; 4096];
            let _ = stream.read(&mut buf);
            let sse_body = concat!(
                "event: message_start\n",
                "data: {\"type\":\"message_start\",\"message\":{\"usage\":{\"input_tokens\":50,\"cache_read_input_tokens\":7,\"cache_creation_input_tokens\":3}}}\n\n",
                "event: message_delta\n",
                "data: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"end_turn\"},\"usage\":{\"output_tokens\":9}}\n\n",
                "event: message_stop\n",
                "data: {\"type\":\"message_stop\"}\n\n"
            );
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                sse_body.len(),
                sse_body
            );
            let _ = stream.write_all(response.as_bytes());
            let _ = stream.flush();
        }
    });

    let base_url = format!("http://{addr}/v1");
    let log = StreamLog {
        kind: "chat",
        session_id: Some("ses-live-stream".to_string()),
        ..StreamLog::default()
    };
    let request = sample_request(RequestPurpose::Chat);

    with_test_db_path(&db_path, || {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("tokio runtime");
        rt.block_on(async {
            let (_tx, cancel) = uncancellable();
            let outcome = run_stream_with_log(
                "anthropic",
                &base_url,
                Some("sk-test"),
                Duration::from_secs(5),
                request,
                &log,
                cancel,
                |_| {},
            )
            .await;
            assert_eq!(outcome.stop, StopReason::Complete);
        });
    });

    let _ = server.join();

    let db = Database::open(&db_path).expect("open test db");
    let page = db
        .usage_logs(&UsageQuery::default(), 1, 10)
        .expect("query usage logs");
    assert_eq!(page.total, 1);
    let row = &page.rows[0];
    assert_eq!(row.session_id.as_deref(), Some("ses-live-stream"));
    assert_eq!(row.purpose, UsagePurpose::Chat);
    assert_eq!(row.provider, "anthropic");
    assert_eq!(row.model, "claude-sonnet-4-5");
    assert_eq!(
        row.tokens,
        UsageTokens {
            input: 60,
            output: 9,
            cache_read: 7,
            cache_write: 3,
        }
    );
    assert_eq!(row.status, UsageStatus::Success);
    assert!(row.duration_ms.is_some_and(|d| d >= 0));

    let _ = std::fs::remove_dir_all(&temp_dir);
}
