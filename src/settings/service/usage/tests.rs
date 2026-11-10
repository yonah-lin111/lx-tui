//! `/usage` 页面与 `/api/usage/*` 接口的 HTTP 集成测试。

use std::path::PathBuf;

use serde_json::Value;
use tokio::sync::mpsc;

use super::super::tests_helpers::{body, get, request, status, write};
use crate::config::ai::ModelPricing;
use crate::db::{Database, UsageLogInsert, UsagePurpose, UsageRates, UsageStatus, UsageTokens};
use crate::settings::service::{SettingsEvent, SettingsService};

fn temp_dir(name: &str) -> PathBuf {
    let unique = format!(
        "lx-tui-usage-http-tests-{}-{}-{}",
        name,
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|duration| duration.as_nanos())
            .unwrap_or_default()
    );
    let path = std::env::temp_dir().join(unique);
    std::fs::create_dir_all(&path).expect("temp dir");
    path
}

async fn start_usage_service(
    name: &str,
) -> (
    SettingsService,
    mpsc::UnboundedReceiver<SettingsEvent>,
    PathBuf,
) {
    let dir = temp_dir(name);
    let config_path = dir.join("settings.json");
    let user_root = dir.join("root");
    std::fs::create_dir_all(&user_root).expect("user root");
    let (sender, receiver) = mpsc::unbounded_channel();
    let service = SettingsService::start_in(sender, config_path, user_root)
        .await
        .expect("service starts");
    (service, receiver, dir)
}

fn db_path_for(service: &SettingsService) -> PathBuf {
    service.user_root_for_tests().join("db").join("lx.db")
}

fn seed_sample_usage(db: &Database) {
    let ws1_path = std::path::Path::new("/tmp/lx-usage-ws-alpha");
    let ws2_path = std::path::Path::new("/tmp/lx-usage-ws-beta");
    db.trust_workspace(ws1_path, "alpha-repo").expect("ws1");
    db.trust_workspace(ws2_path, "beta-repo").expect("ws2");

    let ses1 = db
        .begin_turn(
            ws1_path,
            None,
            "Refactor usage service",
            &crate::db::chat::ModelSnapshot {
                provider: "anthropic".into(),
                model: "claude-sonnet-4-5".into(),
                variant: None,
            },
        )
        .expect("ses1");
    let ses2 = db
        .begin_turn(
            ws2_path,
            None,
            "Fix router tests",
            &crate::db::chat::ModelSnapshot {
                provider: "openai".into(),
                model: "gpt-5".into(),
                variant: None,
            },
        )
        .expect("ses2");

    let pricing_claude = Some(ModelPricing {
        input: Some(3.0),
        output: Some(15.0),
        cache_read: Some(0.3),
        cache_write: Some(3.75),
    });
    let pricing_gpt = Some(ModelPricing {
        input: Some(2.0),
        output: Some(8.0),
        cache_read: Some(0.5),
        cache_write: None,
    });

    db.record_usage_log(&UsageLogInsert {
        external_id: "ulog-http-1".into(),
        session_id: Some(ses1.session_id.clone()),
        workspace_id: None, // 自动从 agent_session 反查补齐 ws1
        purpose: UsagePurpose::Chat,
        provider: "anthropic".into(),
        model: "claude-sonnet-4-5".into(),
        tokens: UsageTokens {
            input: 10_000,
            output: 2_000,
            cache_read: 6_000,
            cache_write: 1_000,
        },
        rates: UsageRates::compute(10_000, 2_000, 6_000, 1_000, pricing_claude),
        duration_ms: Some(1_200),
        status: UsageStatus::Success,
        error_message: None,
        created_at_ms: 1_762_780_000_000,
    })
    .expect("log 1");

    db.record_usage_log(&UsageLogInsert {
        external_id: "ulog-http-2".into(),
        session_id: Some(ses1.session_id.clone()),
        workspace_id: None,
        purpose: UsagePurpose::Compaction,
        provider: "anthropic".into(),
        model: "claude-sonnet-4-5".into(),
        tokens: UsageTokens {
            input: 5_000,
            output: 500,
            cache_read: 0,
            cache_write: 0,
        },
        rates: UsageRates::compute(5_000, 500, 0, 0, pricing_claude),
        duration_ms: Some(800),
        status: UsageStatus::Aborted,
        error_message: None,
        created_at_ms: 1_762_780_060_000,
    })
    .expect("log 2");

    db.record_usage_log(&UsageLogInsert {
        external_id: "ulog-http-3".into(),
        session_id: Some(ses2.session_id.clone()),
        workspace_id: None,
        purpose: UsagePurpose::Subagent,
        provider: "openai".into(),
        model: "gpt-5".into(),
        tokens: UsageTokens {
            input: 4_000,
            output: 1_000,
            cache_read: 1_000,
            cache_write: 0,
        },
        rates: UsageRates::compute(4_000, 1_000, 1_000, 0, pricing_gpt),
        duration_ms: Some(2_500),
        status: UsageStatus::Error,
        error_message: Some("upstream 502".into()),
        created_at_ms: 1_762_783_600_000,
    })
    .expect("log 3");
}

#[tokio::test]
async fn usage_routes_require_token_and_reject_invalid_methods_or_params() {
    let (service, _events, _dir) = start_usage_service("auth-and-validation").await;
    let port = service.port();
    let token = service.token().to_string();

    for path in ["/usage", "/api/usage/overview", "/api/usage/logs"] {
        let denied = request(port, &get(path)).await;
        assert_eq!(status(&denied), 403, "missing token for {path}");
    }

    let wrong_method = request(
        port,
        &write(
            "POST",
            &format!("/api/usage/overview?token={token}"),
            "{}",
            None,
        ),
    )
    .await;
    assert_eq!(status(&wrong_method), 405);

    let unknown = request(port, &get(&format!("/api/usage/nonexistent?token={token}"))).await;
    assert_eq!(status(&unknown), 404);

    for bad_query in [
        "startTime=not-a-number",
        "endTime=abc",
        "page=0",
        "page=-1",
        "pageSize=0",
        "pageSize=xyz",
    ] {
        let resp = request(
            port,
            &get(&format!("/api/usage/logs?token={token}&{bad_query}")),
        )
        .await;
        assert_eq!(status(&resp), 422, "bad query {bad_query}: {}", body(&resp));
    }

    service.shutdown();
}

#[tokio::test]
async fn usage_overview_and_logs_filter_and_paginate_accurately() {
    let (service, _events, _dir) = start_usage_service("overview-and-logs").await;
    let port = service.port();
    let token = service.token().to_string();

    let db = Database::open(&db_path_for(&service)).expect("open db");
    seed_sample_usage(&db);
    drop(db);

    // 1. 全量 overview（按小时粒度）
    let overview_resp = request(
        port,
        &get(&format!(
            "/api/usage/overview?token={token}&granularity=hour"
        )),
    )
    .await;
    assert_eq!(
        status(&overview_resp),
        200,
        "overview: {}",
        body(&overview_resp)
    );
    let overview: Value = serde_json::from_str(body(&overview_resp)).expect("overview json");

    assert_eq!(overview["summary"]["requestCount"], 3);
    assert_eq!(overview["summary"]["successCount"], 1);
    assert_eq!(overview["summary"]["errorCount"], 1);
    assert_eq!(overview["summary"]["abortedCount"], 1);
    assert_eq!(overview["summary"]["inputTokens"], 19_000);
    assert_eq!(overview["summary"]["outputTokens"], 3_500);
    assert_eq!(overview["summary"]["cacheReadTokens"], 7_000);
    assert_eq!(overview["summary"]["cacheWriteTokens"], 1_000);
    assert_eq!(overview["summary"]["totalTokens"], 22_500);
    assert_eq!(overview["summary"]["pricedRequestCount"], 3);
    assert!(overview["summary"]["totalCostUsd"].as_f64().unwrap() > 0.0);

    let daily = overview["daily"].as_array().expect("daily array");
    assert_eq!(daily.len(), 2, "two distinct hour buckets");

    let models = overview["models"].as_array().expect("models array");
    assert_eq!(models.len(), 2);
    assert_eq!(models[0]["model"], "claude-sonnet-4-5");
    assert_eq!(models[0]["requestCount"], 2);

    let providers = overview["providers"].as_array().expect("providers array");
    assert_eq!(providers.len(), 2);
    assert_eq!(providers[0]["provider"], "anthropic");
    assert_eq!(providers[0]["requestCount"], 2);

    let filters = &overview["filters"];
    assert_eq!(
        filters["providers"],
        serde_json::json!(["anthropic", "openai"])
    );
    assert_eq!(
        filters["models"],
        serde_json::json!(["claude-sonnet-4-5", "gpt-5"])
    );
    assert_eq!(
        filters["workspaces"].as_array().expect("workspaces").len(),
        2
    );
    assert_eq!(filters["sessions"].as_array().expect("sessions").len(), 2);

    // 2. 按 provider 过滤 overview
    let filtered_resp = request(
        port,
        &get(&format!(
            "/api/usage/overview?token={token}&provider=openai&granularity=day"
        )),
    )
    .await;
    assert_eq!(status(&filtered_resp), 200);
    let filtered: Value = serde_json::from_str(body(&filtered_resp)).expect("filtered json");
    assert_eq!(filtered["summary"]["requestCount"], 1);
    assert_eq!(filtered["summary"]["errorCount"], 1);
    assert_eq!(filtered["models"].as_array().expect("models").len(), 1);
    assert_eq!(filtered["models"][0]["model"], "gpt-5");
    // provider 筛选项排除自身维度，仍返回全部 2 个 provider
    assert_eq!(
        filtered["filters"]["providers"],
        serde_json::json!(["anthropic", "openai"])
    );
    // model 筛选项受 provider=openai 约束，只返回 gpt-5
    assert_eq!(filtered["filters"]["models"], serde_json::json!(["gpt-5"]));

    // 3. 分页查询 /api/usage/logs（pageSize=2）
    let page1_resp = request(
        port,
        &get(&format!("/api/usage/logs?token={token}&page=1&pageSize=2")),
    )
    .await;
    assert_eq!(status(&page1_resp), 200);
    let page1: Value = serde_json::from_str(body(&page1_resp)).expect("page1 json");
    assert_eq!(page1["total"], 3);
    assert_eq!(page1["page"], 1);
    assert_eq!(page1["pageSize"], 2);
    let rows1 = page1["rows"].as_array().expect("rows1");
    assert_eq!(rows1.len(), 2);
    assert_eq!(rows1[0]["externalId"], "ulog-http-3");
    assert_eq!(rows1[0]["purpose"], "subagent");
    assert_eq!(rows1[0]["status"], "error");
    assert_eq!(rows1[0]["errorMessage"], "upstream 502");
    assert_eq!(rows1[1]["externalId"], "ulog-http-2");
    assert_eq!(rows1[1]["purpose"], "compaction");

    let page2_resp = request(
        port,
        &get(&format!("/api/usage/logs?token={token}&page=2&pageSize=2")),
    )
    .await;
    assert_eq!(status(&page2_resp), 200);
    let page2: Value = serde_json::from_str(body(&page2_resp)).expect("page2 json");
    let rows2 = page2["rows"].as_array().expect("rows2");
    assert_eq!(rows2.len(), 1);
    assert_eq!(rows2[0]["externalId"], "ulog-http-1");
    assert_eq!(rows2[0]["tokens"]["input"], 10_000);
    assert_eq!(rows2[0]["tokens"]["cacheRead"], 6_000);

    // 4. 独立子端点可正常返回
    for sub in ["summary", "daily", "models", "providers", "filters"] {
        let sub_resp = request(port, &get(&format!("/api/usage/{sub}?token={token}"))).await;
        assert_eq!(status(&sub_resp), 200, "sub endpoint {sub}");
    }

    service.shutdown();
}

#[tokio::test]
async fn custom_provider_internal_id_is_synced_to_display_name() {
    let (service, _rx, _dir) = start_usage_service("provider-name-sync").await;
    let user_root = service.user_root_for_tests();
    let ai_config_dir = user_root.join("config");
    std::fs::create_dir_all(&ai_config_dir).expect("config dir");
    std::fs::write(
        ai_config_dir.join("ai.json"),
        r#"{
            "ai": {
                "enabled_providers": ["provider-alq374"],
                "providers": {
                    "provider-alq374": {
                        "id": "provider-alq374",
                        "type": "openai-compatible",
                        "name": "9router-lx-t",
                        "options": { "baseURL": "https://api.example.com/v1" },
                        "models": {
                            "deepseek-v3+plus": {
                                "id": "deepseek-v3+plus",
                                "name": "DeepSeek V3+",
                                "pricing": { "input": 2.0, "output": 8.0 }
                            }
                        }
                    }
                }
            }
        }"#,
    )
    .expect("write ai.json");

    let db = Database::open(&db_path_for(&service)).expect("open db");
    db.record_usage_log(&UsageLogInsert {
        external_id: "ulog-custom-prov-1".into(),
        session_id: None,
        workspace_id: None,
        purpose: UsagePurpose::Chat,
        provider: "provider-alq374".into(),
        model: "deepseek-v3+plus".into(),
        tokens: UsageTokens {
            input: 1_000,
            output: 200,
            cache_read: 0,
            cache_write: 0,
        },
        rates: UsageRates::default(),
        duration_ms: Some(500),
        status: UsageStatus::Success,
        error_message: None,
        created_at_ms: 1_762_780_000_000,
    })
    .expect("record log");
    drop(db);

    let port = service.port();
    let token = service
        .usage_url()
        .split("token=")
        .nth(1)
        .unwrap_or("")
        .to_string();

    let overview_resp = request(port, &get(&format!("/api/usage/overview?token={token}"))).await;
    assert_eq!(status(&overview_resp), 200);
    let overview: Value = serde_json::from_str(body(&overview_resp)).expect("overview json");
    assert_eq!(
        overview["filters"]["providers"],
        serde_json::json!(["9router-lx-t"])
    );
    assert_eq!(overview["providers"][0]["provider"], "9router-lx-t");
    assert_eq!(overview["models"][0]["model"], "deepseek-v3+plus");
    // 验证存量 NULL 成本被自动回填：(1000 * 2.0 + 200 * 8.0) / 1_000_000 = 0.0036
    let backfilled_cost = overview["summary"]["totalCostUsd"]
        .as_f64()
        .expect("backfilled cost");
    assert!((backfilled_cost - 0.0036).abs() < 1e-9);

    // 验证含 `+`（编码为 `%2B`）的 model 查询参数不会被二次解码为空格
    let logs_resp = request(
        port,
        &get(&format!(
            "/api/usage/logs?token={token}&model=deepseek-v3%2Bplus"
        )),
    )
    .await;
    assert_eq!(status(&logs_resp), 200);
    let logs: Value = serde_json::from_str(body(&logs_resp)).expect("logs json");
    assert_eq!(logs["total"], 1);
    assert_eq!(logs["rows"][0]["provider"], "9router-lx-t");
    assert_eq!(logs["rows"][0]["model"], "deepseek-v3+plus");

    service.shutdown();
}
