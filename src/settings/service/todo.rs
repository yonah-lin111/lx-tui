//! Todo 看板 JSON API：`/api/todo/*` 路由、查询参数解析与聚合序列化。
//!
//! 只读接口：`overview` 返回汇总卡、按日聚合与聚合组统计，`items` 返回分页明细。
//! 底层查询委托给 [`crate::db::Database`] 的 `query_todo_*` 方法。

use std::collections::HashMap;

use serde_json::{Value, json};

use super::ServiceState;
use super::handlers::{Request, Response};
use crate::db::Database;
use crate::db::todo::{
    TodoDailyRow, TodoFilter, TodoGroupRow, TodoItemRow, TodoPriority, TodoStatus, TodoSummary,
    civil_from_days, days_from_civil, format_date, parse_date, today_date,
};

/// 解析后的 `/api/todo/*` 查询参数。
#[derive(Debug, Clone, PartialEq, Eq)]
struct ParsedTodoParams {
    filter: TodoFilter,
    page: usize,
    page_size: usize,
}

/// `/api/todo/*` 路由分发（调用前已由 `handlers::route` 完成 capability token 校验）。
pub(super) fn route(request: &Request, state: &ServiceState) -> Response {
    let known_path = matches!(
        request.path.as_str(),
        "/api/todo/overview" | "/api/todo/items"
    );
    if !known_path {
        return Response::error(404, "not found");
    }
    if request.method != "GET" {
        return Response::error(405, "method not allowed");
    }

    let params = match parse_params(&request.query) {
        Ok(params) => params,
        Err(message) => return Response::error(422, &message),
    };

    let db = match Database::open(&state.db_path()) {
        Ok(db) => db,
        Err(error) => return Response::error(500, &error.to_string()),
    };

    match request.path.as_str() {
        "/api/todo/overview" => overview_json(&db, &params.filter),
        _ => items_json(&db, &params),
    }
}

/// `GET /api/todo/overview`：汇总卡、按日聚合与聚合组统计一次返回。
fn overview_json(db: &Database, filter: &TodoFilter) -> Response {
    let today = today_date();
    let summary = match db.query_todo_summary(filter, &today) {
        Ok(summary) => summary,
        Err(error) => return Response::error(500, &error.to_string()),
    };
    let daily = match db.query_todo_daily(filter) {
        Ok(daily) => daily,
        Err(error) => return Response::error(500, &error.to_string()),
    };
    let groups = match db.query_todo_groups(filter) {
        Ok(groups) => groups,
        Err(error) => return Response::error(500, &error.to_string()),
    };

    let body = json!({
        "summary": summary_json(&summary),
        "daily": daily.iter().map(daily_json).collect::<Vec<_>>(),
        "groups": groups.iter().map(group_json).collect::<Vec<_>>(),
    });
    json_response(&body)
}

/// `GET /api/todo/items`：分页返回 Todo 明细。
fn items_json(db: &Database, params: &ParsedTodoParams) -> Response {
    let page = match db.query_todos_page(&params.filter, params.page, params.page_size) {
        Ok(page) => page,
        Err(error) => return Response::error(500, &error.to_string()),
    };
    let body = json!({
        "rows": page.rows.iter().map(item_json).collect::<Vec<_>>(),
        "total": page.total,
        "page": page.page,
        "pageSize": page.page_size,
    });
    json_response(&body)
}

fn summary_json(summary: &TodoSummary) -> Value {
    json!({
        "total": summary.total,
        "open": summary.open,
        "inProgress": summary.in_progress,
        "done": summary.done,
        "overdue": summary.overdue,
    })
}

fn daily_json(row: &TodoDailyRow) -> Value {
    json!({
        "date": row.date,
        "planned": row.planned,
        "done": row.done,
        "remaining": row.remaining,
    })
}

fn group_json(row: &TodoGroupRow) -> Value {
    json!({
        "groupId": row.group_id,
        "date": row.date,
        "label": row.label,
        "items": row.items,
        "open": row.open,
        "inProgress": row.in_progress,
        "done": row.done,
        "topPriority": row.top_priority.as_str(),
    })
}

fn item_json(row: &TodoItemRow) -> Value {
    let record = &row.record;
    json!({
        "id": record.id,
        "date": record.entry_date,
        "text": record.text,
        "status": record.status.as_str(),
        "priority": record.priority.as_str(),
        "groupId": record.group_id,
        "groupLabel": row.group_label,
        "completedDate": record.completed_date,
        "position": record.position,
    })
}

fn json_response(value: &Value) -> Response {
    match serde_json::to_string(value) {
        Ok(text) => Response::json(200, text),
        Err(error) => Response::error(500, &error.to_string()),
    }
}

/// 从请求 query 表解析筛选与分页参数。
fn parse_params(query: &HashMap<String, String>) -> Result<ParsedTodoParams, String> {
    let mut start_date = parse_date_param(query, "startDate")?;
    let mut end_date = parse_date_param(query, "endDate")?;
    if let (Some(start), Some(end)) = (&start_date, &end_date)
        && start > end
    {
        std::mem::swap(&mut start_date, &mut end_date);
    }

    let filter = TodoFilter {
        start_date,
        end_date,
        status: parse_status(query)?,
        priority: parse_priority(query)?,
        search: parse_search(query),
    };

    Ok(ParsedTodoParams {
        filter,
        page: parse_page(query)?,
        page_size: parse_page_size(query)?,
    })
}

/// 日期参数：严格校验真实日历日（拒绝 `2026-02-31`）。
fn parse_date_param(query: &HashMap<String, String>, key: &str) -> Result<Option<String>, String> {
    let Some(raw) = query.get(key) else {
        return Ok(None);
    };
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Ok(None);
    }
    // 只接受定长 `YYYY-MM-DD`；`2026-1-1` 这类宽松写法直接拒绝。
    if trimmed.len() != 10 || trimmed.as_bytes()[4] != b'-' || trimmed.as_bytes()[7] != b'-' {
        return Err(format!("invalid query parameter '{key}'"));
    }
    let Some((year, month, day)) = parse_date(trimmed) else {
        return Err(format!("invalid query parameter '{key}'"));
    };
    let days = days_from_civil(year, month, day);
    if civil_from_days(days) != (year, month, day) {
        return Err(format!("invalid query parameter '{key}'"));
    }
    Ok(Some(format_date(year, month, day)))
}

fn parse_status(query: &HashMap<String, String>) -> Result<Option<TodoStatus>, String> {
    let Some(raw) = query.get("status") else {
        return Ok(None);
    };
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Ok(None);
    }
    match trimmed {
        "todo" => Ok(Some(TodoStatus::Todo)),
        "in_progress" => Ok(Some(TodoStatus::InProgress)),
        "done" => Ok(Some(TodoStatus::Done)),
        _ => Err("invalid query parameter 'status'".to_string()),
    }
}

fn parse_priority(query: &HashMap<String, String>) -> Result<Option<TodoPriority>, String> {
    let Some(raw) = query.get("priority") else {
        return Ok(None);
    };
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Ok(None);
    }
    match trimmed {
        "P0" => Ok(Some(TodoPriority::P0)),
        "P1" => Ok(Some(TodoPriority::P1)),
        "P2" => Ok(Some(TodoPriority::P2)),
        "P3" => Ok(Some(TodoPriority::P3)),
        _ => Err("invalid query parameter 'priority'".to_string()),
    }
}

/// 搜索词：trim 后最多 200 字符；空串视为不过滤。
fn parse_search(query: &HashMap<String, String>) -> Option<String> {
    query
        .get("q")
        .map(|raw| raw.trim().chars().take(200).collect::<String>())
        .filter(|value| !value.is_empty())
}

fn parse_page(query: &HashMap<String, String>) -> Result<usize, String> {
    let Some(raw) = query.get("page") else {
        return Ok(1);
    };
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Ok(1);
    }
    let parsed = trimmed
        .parse::<usize>()
        .map_err(|_| "invalid query parameter 'page'".to_string())?;
    if parsed == 0 {
        return Err("query parameter 'page' must be >= 1".to_string());
    }
    Ok(parsed)
}

/// 页大小：缺省 50，越界钳制到 `1..=200`。
fn parse_page_size(query: &HashMap<String, String>) -> Result<usize, String> {
    let Some(raw) = query.get("pageSize") else {
        return Ok(50);
    };
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Ok(50);
    }
    let parsed = trimmed
        .parse::<usize>()
        .map_err(|_| "invalid query parameter 'pageSize'".to_string())?;
    Ok(parsed.clamp(1, 200))
}

#[cfg(test)]
mod tests;
