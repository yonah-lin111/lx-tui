//! `usage_log` 迁移回填、单次写入与多维聚合查询单元测试。

use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use rusqlite::params;

use super::*;
use crate::config::ai::ModelPricing;
use crate::db::{Database, SCHEMA_VERSION};

fn temp_db_path(label: &str) -> PathBuf {
    static SEQ: AtomicU64 = AtomicU64::new(0);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let seq = SEQ.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir()
        .join(format!("lx-tui-usage-test-{label}-{nanos:x}-{seq:x}"))
        .join("lx.db")
}

#[test]
fn v25_to_v26_migration_backfills_assistant_messages_idempotently() {
    let path = temp_db_path("v26-backfill");
    let ws_path = path.parent().expect("parent").join("ws-demo");
    {
        let db = Database::open(&path).expect("open db");
        db.trust_workspace(&ws_path, "Demo Workspace")
            .expect("trust");
        let ws_ext: String = db
            .conn
            .query_row(
                "SELECT external_id FROM workspace WHERE name = 'Demo Workspace'",
                [],
                |row| row.get(0),
            )
            .expect("ws_ext");

        // 模拟退回 v25：删除 usage_log 表并写入存量会话与消息。
        db.conn
            .execute_batch("DROP TABLE IF EXISTS usage_log; PRAGMA user_version = 25;")
            .expect("downgrade to v25");

        db.conn
            .execute(
                r#"INSERT INTO agent_session
                       (external_id, workspace_id, source, title, first_user_message,
                        model_provider, model_id, created_at_ms, updated_at_ms, last_message_at_ms)
                   VALUES ('ses-native', ?1, 'native', 'Main Session', 'hello',
                           'anthropic', 'claude-sonnet-4-5', 1000, 2000, 2000),
                          ('ses-sub', ?1, 'subagent', 'Sub Session', 'sub task',
                           'openai', 'gpt-5', 1500, 2500, 2500)"#,
                params![ws_ext],
            )
            .expect("seed sessions");

        db.conn
            .execute_batch(
                r#"INSERT INTO agent_message
                       (external_id, session_id, sequence, turn_index, role, kind, status,
                        model_provider, model_id, input_tokens, output_tokens,
                        cache_read_tokens, cache_write_tokens, error_code,
                        created_at_ms, completed_at_ms)
                   VALUES
                       ('msg-user-1', 'ses-native', 0, 0, 'user', 'chat', 'complete',
                        NULL, NULL, NULL, NULL, NULL, NULL, NULL, 1000, 1000),
                       ('msg-asst-ok', 'ses-native', 1, 0, 'assistant', 'chat', 'complete',
                        'anthropic', 'claude-sonnet-4-5', 1200, 300, 800, 100, NULL, 1100, 1600),
                       ('msg-asst-abort', 'ses-native', 2, 1, 'assistant', 'chat', 'interrupted',
                        NULL, NULL, 400, 50, NULL, NULL, NULL, 1700, 1950),
                       ('msg-asst-fail', 'ses-sub', 0, 0, 'assistant', 'chat', 'failed',
                        'openai', 'gpt-5', NULL, NULL, NULL, NULL, 'rate_limit', 2000, 2300),
                       ('msg-shell', 'ses-native', 3, 2, 'assistant', 'user_shell', 'complete',
                        'anthropic', 'claude-sonnet-4-5', 10, 10, 0, 0, NULL, 2400, 2410),
                       ('msg-streaming', 'ses-native', 4, 3, 'assistant', 'chat', 'streaming',
                        'anthropic', 'claude-sonnet-4-5', 50, 10, 0, 0, NULL, 2500, NULL);"#,
            )
            .expect("seed messages");
    }

    // 重新打开触发 v25 -> v26 迁移与历史回填。
    let db = Database::open(&path).expect("upgrade to v26");
    let version: i64 = db
        .conn
        .query_row("PRAGMA user_version", [], |row| row.get(0))
        .expect("version");
    assert_eq!(version, SCHEMA_VERSION);

    let page = db
        .usage_logs(&UsageQuery::default(), 1, 50)
        .expect("usage_logs");
    assert_eq!(
        page.total, 3,
        "only assistant chat messages in complete/interrupted/failed are backfilled"
    );

    let by_ext = |ext: &str| -> UsageLogRecord {
        page.rows
            .iter()
            .find(|r| r.external_id == ext)
            .cloned()
            .expect(ext)
    };

    let ok_row = by_ext("ulog-backfill-msg-asst-ok");
    assert_eq!(ok_row.purpose, UsagePurpose::Chat);
    assert_eq!(ok_row.status, UsageStatus::Success);
    assert_eq!(ok_row.provider, "anthropic");
    assert_eq!(ok_row.model, "claude-sonnet-4-5");
    assert_eq!(
        ok_row.tokens,
        UsageTokens {
            input: 1200,
            output: 300,
            cache_read: 800,
            cache_write: 100,
        }
    );
    assert_eq!(ok_row.duration_ms, Some(500));
    assert_eq!(ok_row.created_at_ms, 1600);

    // 消息级 model_provider/model_id 为空时回退会话级模型信息。
    let abort_row = by_ext("ulog-backfill-msg-asst-abort");
    assert_eq!(abort_row.purpose, UsagePurpose::Chat);
    assert_eq!(abort_row.status, UsageStatus::Aborted);
    assert_eq!(abort_row.provider, "anthropic");
    assert_eq!(abort_row.model, "claude-sonnet-4-5");
    assert_eq!(abort_row.duration_ms, Some(250));

    // subagent 会话消息映射为 UsagePurpose::Subagent，failed 映射为 UsageStatus::Error。
    let fail_row = by_ext("ulog-backfill-msg-asst-fail");
    assert_eq!(fail_row.purpose, UsagePurpose::Subagent);
    assert_eq!(fail_row.status, UsageStatus::Error);
    assert_eq!(fail_row.error_message.as_deref(), Some("rate_limit"));
    assert_eq!(fail_row.duration_ms, Some(300));

    // 再次执行 v26 SQL 验证幂等不重复插入。
    db.conn
        .execute_batch("PRAGMA user_version = 25;")
        .expect("reset user_version");
    drop(db);
    let db_again = Database::open(&path).expect("re-open v26");
    let page_again = db_again
        .usage_logs(&UsageQuery::default(), 1, 50)
        .expect("usage_logs again");
    assert_eq!(page_again.total, 3);
}

#[test]
fn usage_rates_compute_subtracts_cached_tokens_from_fresh_input() {
    let pricing = ModelPricing {
        input: Some(3.0),
        output: Some(15.0),
        cache_read: Some(0.3),
        cache_write: Some(3.75),
    };
    // input = 10_000 (其中 cache_read = 6_000, cache_write = 1_000 -> fresh_input = 3_000)
    // output = 2_000
    let rates = UsageRates::compute(10_000, 2_000, 6_000, 1_000, Some(pricing));
    let approx_eq = |a: Option<f64>, b: f64| {
        let val = a.expect("some cost");
        assert!((val - b).abs() < 1e-9, "expected {b}, got {val}");
    };
    // fresh_input: 3_000 * 3.0 / 1e6 = 0.009
    approx_eq(rates.input_cost_usd, 0.009);
    // output: 2_000 * 15.0 / 1e6 = 0.03
    approx_eq(rates.output_cost_usd, 0.03);
    // cache_read: 6_000 * 0.3 / 1e6 = 0.0018
    approx_eq(rates.cache_read_cost_usd, 0.0018);
    // cache_write: 1_000 * 3.75 / 1e6 = 0.00375
    approx_eq(rates.cache_write_cost_usd, 0.00375);
    // total: 0.009 + 0.03 + 0.0018 + 0.00375 = 0.04455
    approx_eq(rates.total_cost_usd, 0.04455);

    // cache 超过 input 时 fresh_input 夹到 0，不出现负成本。
    let clamped = UsageRates::compute(1_000, 0, 2_000, 0, Some(pricing));
    approx_eq(clamped.input_cost_usd, 0.0);

    // 无单价时全部返回 None。
    assert_eq!(
        UsageRates::compute(1_000, 500, 0, 0, None),
        UsageRates::default()
    );
    assert_eq!(
        UsageRates::compute(1_000, 500, 0, 0, Some(ModelPricing::default())),
        UsageRates::default()
    );
}

#[test]
fn record_usage_log_resolves_workspace_and_survives_session_deletion() {
    let db = Database::open_in_memory().expect("in-memory db");
    let ws_path = PathBuf::from("/tmp/lx-usage-ws-resolve");
    db.trust_workspace(&ws_path, "Alpha Repo").expect("trust");
    let ws_id: String = db
        .conn
        .query_row(
            "SELECT external_id FROM workspace WHERE name = 'Alpha Repo'",
            [],
            |row| row.get(0),
        )
        .expect("ws_id");
    db.conn
        .execute(
            r#"INSERT INTO agent_session
                   (external_id, workspace_id, source, title, first_user_message,
                    model_provider, model_id, created_at_ms, updated_at_ms, last_message_at_ms)
               VALUES ('ses-alpha', ?1, 'native', 'Alpha Chat', 'hi',
                       'anthropic', 'claude-sonnet-4-5', 1000, 1000, 1000)"#,
            params![ws_id],
        )
        .expect("insert session");

    db.record_usage_log(&UsageLogInsert {
        external_id: "ulog-1".to_string(),
        session_id: Some("ses-alpha".to_string()),
        workspace_id: None,
        purpose: UsagePurpose::Compaction,
        provider: "anthropic".to_string(),
        model: "claude-sonnet-4-5".to_string(),
        tokens: UsageTokens {
            input: 5000,
            output: 600,
            cache_read: 4000,
            cache_write: 0,
        },
        rates: UsageRates::default(),
        duration_ms: Some(1200),
        status: UsageStatus::Success,
        error_message: None,
        created_at_ms: 2000,
    })
    .expect("record usage");

    // 删除会话后，usage_log 记录及其反查出的 workspace_id 仍然保留。
    db.conn
        .execute(
            "DELETE FROM agent_session WHERE external_id = 'ses-alpha'",
            [],
        )
        .expect("delete session");

    let page = db
        .usage_logs(&UsageQuery::default(), 1, 10)
        .expect("usage_logs");
    assert_eq!(page.total, 1);
    assert_eq!(page.rows[0].workspace_id.as_deref(), Some(ws_id.as_str()));
    assert_eq!(page.rows[0].session_id.as_deref(), Some("ses-alpha"));
    assert_eq!(page.rows[0].purpose, UsagePurpose::Compaction);
}

#[test]
fn usage_aggregations_filters_and_pagination_work_end_to_end() {
    let db = Database::open_in_memory().expect("in-memory db");
    db.conn
        .execute(
            r#"INSERT INTO workspace (external_id, name, "type", path, is_trusted, position, created_at, updated_at)
               VALUES ('ws-1', 'Project One', 'filesystem', '/tmp/p1', 1, 0, 'now', 'now'),
                      ('ws-2', 'Project Two', 'filesystem', '/tmp/p2', 1, 1, 'now', 'now')"#,
            [],
        )
        .expect("workspaces");
    db.conn
        .execute(
            r#"INSERT INTO agent_session
                   (external_id, workspace_id, source, title, first_user_message,
                    model_provider, model_id, created_at_ms, updated_at_ms, last_message_at_ms)
               VALUES ('ses-1', 'ws-1', 'native', 'Build Dashboard', 'build it',
                       'anthropic', 'claude-sonnet-4-5', 1000, 1000, 1000),
                      ('ses-2', 'ws-2', 'native', '', 'fallback title',
                       'openai', 'gpt-5', 2000, 2000, 2000)"#,
            [],
        )
        .expect("sessions");

    let base_ts: i64 = 1_762_780_000_000;
    let entries = [
        UsageLogInsert {
            external_id: "u1".to_string(),
            session_id: Some("ses-1".to_string()),
            workspace_id: Some("ws-1".to_string()),
            purpose: UsagePurpose::Chat,
            provider: "anthropic".to_string(),
            model: "claude-sonnet-4-5".to_string(),
            tokens: UsageTokens {
                input: 1000,
                output: 200,
                cache_read: 600,
                cache_write: 100,
            },
            rates: UsageRates {
                input_cost_usd: Some(0.01),
                output_cost_usd: Some(0.02),
                cache_read_cost_usd: Some(0.001),
                cache_write_cost_usd: Some(0.002),
                total_cost_usd: Some(0.033),
            },
            duration_ms: Some(1000),
            status: UsageStatus::Success,
            error_message: None,
            created_at_ms: base_ts,
        },
        UsageLogInsert {
            external_id: "u2".to_string(),
            session_id: Some("ses-1".to_string()),
            workspace_id: Some("ws-1".to_string()),
            purpose: UsagePurpose::Title,
            provider: "anthropic".to_string(),
            model: "claude-haiku-4".to_string(),
            tokens: UsageTokens {
                input: 500,
                output: 50,
                cache_read: 0,
                cache_write: 0,
            },
            rates: UsageRates {
                input_cost_usd: Some(0.001),
                output_cost_usd: Some(0.001),
                cache_read_cost_usd: Some(0.0),
                cache_write_cost_usd: Some(0.0),
                total_cost_usd: Some(0.002),
            },
            duration_ms: Some(400),
            status: UsageStatus::Success,
            error_message: None,
            created_at_ms: base_ts + 1_000,
        },
        UsageLogInsert {
            external_id: "u3".to_string(),
            session_id: Some("ses-2".to_string()),
            workspace_id: Some("ws-2".to_string()),
            purpose: UsagePurpose::Subagent,
            provider: "openai".to_string(),
            model: "gpt-5".to_string(),
            tokens: UsageTokens {
                input: 2000,
                output: 400,
                cache_read: 1000,
                cache_write: 0,
            },
            rates: UsageRates::default(),
            duration_ms: Some(1600),
            status: UsageStatus::Error,
            error_message: Some("upstream timeout".to_string()),
            created_at_ms: base_ts + 3_600_000,
        },
    ];
    for entry in &entries {
        db.record_usage_log(entry).expect("insert entry");
    }

    // 1. Summary
    let summary = db.usage_summary(&UsageQuery::default()).expect("summary");
    assert_eq!(summary.request_count, 3);
    assert_eq!(summary.success_count, 2);
    assert_eq!(summary.error_count, 1);
    assert_eq!(summary.aborted_count, 0);
    assert_eq!(summary.input_tokens, 3500);
    assert_eq!(summary.output_tokens, 650);
    assert_eq!(summary.cache_read_tokens, 1600);
    assert_eq!(summary.cache_write_tokens, 100);
    assert_eq!(summary.total_tokens, 4150);
    assert_eq!(summary.priced_request_count, 2);
    assert!((summary.total_cost_usd.expect("cost") - 0.035).abs() < 1e-9);
    assert!((summary.avg_duration_ms.expect("duration") - 1000.0).abs() < 1e-6);

    // 2. Series (Hour & Day)
    let hourly = db
        .usage_series(&UsageQuery::default(), UsageGranularity::Hour)
        .expect("hourly");
    assert_eq!(hourly.len(), 2, "two distinct hours");
    assert_eq!(hourly[0].request_count, 2);
    assert_eq!(hourly[1].request_count, 1);
    assert_eq!(hourly[1].total_cost_usd, None, "u3 has no pricing");

    let daily = db
        .usage_series(&UsageQuery::default(), UsageGranularity::Day)
        .expect("daily");
    assert!(!daily.is_empty());

    // 3. Model stats (cost-first ordering, unpriced last)
    let models = db
        .usage_model_stats(&UsageQuery::default())
        .expect("model stats");
    assert_eq!(
        models.iter().map(|m| m.model.as_str()).collect::<Vec<_>>(),
        vec!["claude-sonnet-4-5", "claude-haiku-4", "gpt-5"]
    );
    assert_eq!(models[2].total_cost_usd, None);
    assert_eq!(models[2].avg_cost_per_request_usd, None);

    // 4. Provider stats
    let providers = db
        .usage_provider_stats(&UsageQuery::default())
        .expect("provider stats");
    assert_eq!(providers.len(), 2);
    assert_eq!(providers[0].provider, "anthropic");
    assert_eq!(providers[0].request_count, 2);
    assert_eq!(providers[0].success_rate, Some(100.0));
    assert_eq!(providers[1].provider, "openai");
    assert_eq!(providers[1].total_cost_usd, None);
    assert_eq!(providers[1].success_rate, Some(0.0));

    // 5. Filter options (cascading exclusion & readable names)
    let filtered_query = UsageQuery {
        provider: Some("anthropic".to_string()),
        ..UsageQuery::default()
    };
    let options = db
        .usage_filter_options(&filtered_query)
        .expect("filter options");
    assert_eq!(
        options.providers,
        vec!["anthropic".to_string(), "openai".to_string()],
        "provider filter excludes itself so all providers remain selectable"
    );
    assert_eq!(
        options.models,
        vec![
            "claude-haiku-4".to_string(),
            "claude-sonnet-4-5".to_string()
        ],
        "models are scoped to selected provider"
    );
    assert_eq!(
        options.workspaces,
        vec![UsageNamedOption {
            id: "ws-1".to_string(),
            name: "Project One".to_string(),
        }]
    );
    assert_eq!(options.sessions.len(), 2);
    assert_eq!(options.sessions[0].id, "ses-2");
    assert_eq!(
        options.sessions[0].name, "ses-2",
        "empty title falls back to id prefix"
    );
    assert_eq!(options.sessions[1].id, "ses-1");
    assert_eq!(options.sessions[1].name, "Build Dashboard");

    // 6. Pagination
    let page1 = db.usage_logs(&UsageQuery::default(), 1, 2).expect("page 1");
    assert_eq!(page1.total, 3);
    assert_eq!(page1.rows.len(), 2);
    assert_eq!(page1.rows[0].external_id, "u3");
    assert_eq!(page1.rows[1].external_id, "u2");

    let page2 = db.usage_logs(&UsageQuery::default(), 2, 2).expect("page 2");
    assert_eq!(page2.rows.len(), 1);
    assert_eq!(page2.rows[0].external_id, "u1");
}
