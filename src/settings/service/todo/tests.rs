//! `/todo` 页面与 `/api/todo/*` 接口的 HTTP 集成测试。

use std::path::PathBuf;

use serde_json::Value;
use tokio::sync::mpsc;

use super::super::tests_helpers::{body, get, request, status, write};
use crate::db::Database;
use crate::db::todo::{
    TodoPriority, TodoRecord, TodoStatus, new_todo_group_id, shift_date, today_date,
};
use crate::settings::service::{SettingsEvent, SettingsService};

fn temp_dir(name: &str) -> PathBuf {
    let unique = format!(
        "lx-tui-todo-http-tests-{}-{}-{}",
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

async fn start_todo_service(
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

/// 种子数据：今天 5 条（含 2 条聚合组），昨天 2 条。
fn seed_todos(db: &Database) {
    let today = today_date();
    let yesterday = shift_date(&today, -1);
    let group_id = new_todo_group_id();

    let a = TodoRecord::new_on(&today, "alpha-a", TodoStatus::Todo, TodoPriority::P1, 0)
        .with_group_id(Some(group_id.clone()));
    let b = TodoRecord::new_on(&today, "alpha-b", TodoStatus::Todo, TodoPriority::P0, 1)
        .with_group_id(Some(group_id));
    let c = TodoRecord::new_on(
        &today,
        "beta-c",
        TodoStatus::InProgress,
        TodoPriority::P2,
        2,
    );
    let d = TodoRecord::new_on(&today, "gamma-d", TodoStatus::Done, TodoPriority::P1, 3);
    let e = TodoRecord::new_on(&today, "delta-e", TodoStatus::Todo, TodoPriority::P3, 4);
    let f = TodoRecord::new_on(
        &yesterday,
        "epsilon-f",
        TodoStatus::Todo,
        TodoPriority::P1,
        0,
    );
    let g = TodoRecord::new_on(&yesterday, "zeta-g", TodoStatus::Done, TodoPriority::P0, 1);

    for record in [&a, &b, &c, &d, &e] {
        db.save_todo(record).expect("save today");
    }
    for record in [&f, &g] {
        db.save_todo(record).expect("save yesterday");
    }
    // `save_todo` 会清理单成员聚合组；两条都写完后用整日同步把组关系落定。
    db.sync_date_todos(&today, &[a, b, c, d, e])
        .expect("sync today");
}

#[tokio::test]
async fn todo_overview_aggregates_summary_daily_and_groups() {
    let (service, _events, _dir) = start_todo_service("overview").await;
    let port = service.port();
    let token = service.token().to_string();

    let db = Database::open(&db_path_for(&service)).expect("open db");
    seed_todos(&db);
    drop(db);

    let resp = request(port, &get(&format!("/api/todo/overview?token={token}"))).await;
    assert_eq!(status(&resp), 200, "overview: {}", body(&resp));
    let overview: Value = serde_json::from_str(body(&resp)).expect("overview json");

    assert_eq!(overview["summary"]["total"], 7);
    assert_eq!(overview["summary"]["open"], 4);
    assert_eq!(overview["summary"]["inProgress"], 1);
    assert_eq!(overview["summary"]["done"], 2);
    assert_eq!(overview["summary"]["overdue"], 1, "昨天未完成 1 条");

    let priority = &overview["priority"];
    assert_eq!(priority["p0"], 2, "b 与 g 为 P0");
    assert_eq!(priority["p1"], 3, "a、d、f 为 P1");
    assert_eq!(priority["p2"], 1, "c 为 P2");
    assert_eq!(priority["p3"], 1, "e 为 P3");

    let daily = overview["daily"].as_array().expect("daily array");
    assert_eq!(daily.len(), 2);
    assert_eq!(daily[0]["planned"], 5, "按日期倒序，今天在前");
    assert_eq!(daily[0]["done"], 1);
    assert_eq!(daily[0]["remaining"], 4);
    assert_eq!(daily[1]["planned"], 2);
    assert_eq!(daily[1]["remaining"], 1);

    let groups = overview["groups"].as_array().expect("groups array");
    assert_eq!(groups.len(), 1, "独立条目不得出现在 Groups");
    let group = &groups[0];
    assert_eq!(group["items"], 2);
    assert_eq!(group["open"], 2);
    assert_eq!(group["inProgress"], 0);
    assert_eq!(group["done"], 0);
    assert_eq!(group["topPriority"], "P0");
    assert_eq!(group["label"], "alpha-a", "组标签取组内首条文本");
    assert!(
        group["groupId"]
            .as_str()
            .is_some_and(|id| id.starts_with("tgrp-"))
    );
}

#[tokio::test]
async fn todo_overview_overdue_ignores_date_range() {
    let (service, _events, _dir) = start_todo_service("overdue").await;
    let port = service.port();
    let token = service.token().to_string();
    let today = today_date();

    let db = Database::open(&db_path_for(&service)).expect("open db");
    seed_todos(&db);
    drop(db);

    let resp = request(
        port,
        &get(&format!(
            "/api/todo/overview?token={token}&startDate={today}&endDate={today}"
        )),
    )
    .await;
    assert_eq!(status(&resp), 200, "overview: {}", body(&resp));
    let overview: Value = serde_json::from_str(body(&resp)).expect("overview json");
    assert_eq!(overview["summary"]["total"], 5, "日期范围只影响范围统计");
    assert_eq!(
        overview["summary"]["overdue"], 1,
        "Overdue 必须忽略日期范围，仍统计历史未完成"
    );
    assert_eq!(overview["priority"]["p0"], 1, "优先级分布按日期范围收敛");
    assert_eq!(overview["priority"]["p1"], 2);
    assert_eq!(overview["priority"]["p2"], 1);
    assert_eq!(overview["priority"]["p3"], 1);

    let done_only = request(
        port,
        &get(&format!(
            "/api/todo/overview?token={token}&startDate={today}&endDate={today}&status=done"
        )),
    )
    .await;
    assert_eq!(status(&done_only), 200);
    let done_only: Value = serde_json::from_str(body(&done_only)).expect("done json");
    assert_eq!(
        done_only["summary"]["overdue"], 0,
        "status=done 与未完成互斥，Overdue 归零"
    );
    assert_eq!(
        done_only["priority"]["p0"], 0,
        "日期范围收窄到今天后，昨天的 g 不再计入"
    );
    assert_eq!(done_only["priority"]["p1"], 1, "今天仅 d 为 done/P1");
    assert_eq!(done_only["priority"]["p2"], 0);
    assert_eq!(done_only["priority"]["p3"], 0);
}

#[tokio::test]
async fn todo_items_filters_and_paginates() {
    let (service, _events, _dir) = start_todo_service("items").await;
    let port = service.port();
    let token = service.token().to_string();

    let db = Database::open(&db_path_for(&service)).expect("open db");
    seed_todos(&db);
    drop(db);

    let page2 = request(
        port,
        &get(&format!("/api/todo/items?token={token}&page=2&pageSize=5")),
    )
    .await;
    assert_eq!(status(&page2), 200, "items: {}", body(&page2));
    let page2: Value = serde_json::from_str(body(&page2)).expect("items json");
    assert_eq!(page2["total"], 7);
    assert_eq!(page2["page"], 2);
    assert_eq!(page2["pageSize"], 5);
    assert_eq!(page2["rows"].as_array().expect("rows").len(), 2);

    let alpha = request(
        port,
        &get(&format!("/api/todo/items?token={token}&q=ALPHA")),
    )
    .await;
    let alpha: Value = serde_json::from_str(body(&alpha)).expect("alpha json");
    assert_eq!(alpha["total"], 2, "ASCII 子串大小写不敏感");
    let rows = alpha["rows"].as_array().expect("rows");
    assert_eq!(rows[0]["groupLabel"], "alpha-a");
    assert_eq!(rows[0]["status"], "todo");
    assert_eq!(rows[0]["priority"], "P1");
    assert_eq!(rows[0]["date"], today_date());

    let done = request(
        port,
        &get(&format!(
            "/api/todo/items?token={token}&status=done&priority=P0"
        )),
    )
    .await;
    let done: Value = serde_json::from_str(body(&done)).expect("done json");
    assert_eq!(done["total"], 1, "status 与 priority 必须同时生效");
    assert_eq!(done["rows"][0]["text"], "zeta-g");
    assert!(done["rows"][0]["completedDate"].as_str().is_some());
}

#[tokio::test]
async fn todo_api_rejects_bad_params_and_methods() {
    let (service, _events, _dir) = start_todo_service("bad-params").await;
    let port = service.port();
    let token = service.token().to_string();

    for bad_query in [
        "startDate=2026-13-01",
        "startDate=2026-02-31",
        "startDate=2026-1-1",
        "endDate=not-a-date",
        "status=nope",
        "priority=P9",
        "page=0",
        "page=abc",
        "pageSize=abc",
    ] {
        let resp = request(
            port,
            &get(&format!("/api/todo/overview?token={token}&{bad_query}")),
        )
        .await;
        assert_eq!(status(&resp), 422, "bad query {bad_query}: {}", body(&resp));
    }

    let post = request(
        port,
        &write(
            "POST",
            &format!("/api/todo/items?token={token}"),
            "{}",
            None,
        ),
    )
    .await;
    assert_eq!(status(&post), 405);

    let unknown = request(port, &get(&format!("/api/todo/nope?token={token}"))).await;
    assert_eq!(status(&unknown), 404);

    let denied = request(port, &get("/api/todo/items")).await;
    assert_eq!(status(&denied), 403, "无 token 必须拒绝");
}

#[tokio::test]
async fn todo_page_and_assets_are_wired() {
    let (service, _events, _dir) = start_todo_service("page").await;
    let port = service.port();
    let token = service.token().to_string();

    let page = request(port, &get(&format!("/todo?token={token}"))).await;
    assert_eq!(status(&page), 200);
    let html = body(&page);
    for needle in [
        "id=\"view-todo\"",
        "data-view=\"todo\"",
        "href=\"#/todo\"",
        "/todo.css",
        "/todo_charts.js",
        "/todo.js",
        "todo-summary-cards",
        "todo-chart-trend",
        "todo-chart-status",
        "todo-status-legend",
        "todo-chart-priority",
        "todo-table-container",
    ] {
        assert!(html.contains(needle), "shell missing {needle}");
    }

    let css = request(port, &get("/todo.css")).await;
    assert_eq!(status(&css), 200);
    assert!(
        css.contains("text/css; charset=utf-8"),
        "todo.css content type"
    );

    let js = request(port, &get("/todo.js")).await;
    assert_eq!(status(&js), 200);
    assert!(
        js.contains("text/javascript; charset=utf-8"),
        "todo.js content type"
    );
    assert!(js.contains("function initTodoView"), "todo view entry");
    assert!(!js.contains("innerHTML"), "no innerHTML");
    assert!(!js.contains("function createSelect"), "reuse shared select");
    assert!(!js.contains("function showToast"), "reuse shared toast");
    assert!(
        js.contains("renderTodoCharts"),
        "todo controller wires the chart renderer"
    );
    assert!(
        js.contains("renderTodoSkeleton"),
        "todo paints a stable first-load skeleton"
    );

    let charts = request(port, &get("/todo_charts.js")).await;
    assert_eq!(status(&charts), 200);
    assert!(
        charts.contains("text/javascript; charset=utf-8"),
        "todo_charts.js content type"
    );
    assert!(
        charts.contains("function renderTodoCharts"),
        "chart renderer entry"
    );
    assert!(!charts.contains("innerHTML"), "no innerHTML in charts");
}
