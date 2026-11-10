//! 用量统计看板 JSON API：`/api/usage/*` 路由、查询参数解析与聚合序列化。
//!
//! 复用 `SettingsService` 本地回环 HTTP 服务与 capability token 鉴权，
//! 底层查询直接委托给 [`crate::db::Database`] 的 `usage_*` 方法。

use std::collections::HashMap;

use serde::Serialize;
use serde_json::json;

use super::ServiceState;
use super::handlers::{Request, Response};
use crate::db::{Database, UsageGranularity, UsageQuery};

/// 解析后的 `/api/usage/*` 查询参数。
#[derive(Debug, Clone, PartialEq, Eq)]
struct ParsedUsageParams {
    query: UsageQuery,
    granularity: UsageGranularity,
    page: usize,
    page_size: usize,
}

/// `/api/usage/*` 路由分发（调用前已由 `handlers::route` 完成 capability token 校验）。
pub(super) fn route(request: &Request, state: &ServiceState) -> Response {
    let known_path = matches!(
        request.path.as_str(),
        "/api/usage/overview"
            | "/api/usage/logs"
            | "/api/usage/summary"
            | "/api/usage/daily"
            | "/api/usage/models"
            | "/api/usage/providers"
            | "/api/usage/filters"
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

    // 将历史回填或存量记录中的内部供应商 ID（如 `provider-alq374`）同步为可读 `provider.name`，
    // 并按最新配置与内置目录补齐 `total_cost_usd IS NULL` 的历史成本。
    if let Ok(settings) =
        crate::config::ai::connect::load_runtime_ai_or_empty(&super::ai_path(state))
    {
        let _ = db.sync_usage_provider_names(&settings.provider_name_map());
        let _ = db.backfill_usage_costs(&settings);
    }

    match request.path.as_str() {
        "/api/usage/overview" => overview_json(&db, &params),
        "/api/usage/logs" => logs_json(&db, &params),
        "/api/usage/summary" => match db.usage_summary(&params.query) {
            Ok(summary) => ok_json(&summary),
            Err(error) => Response::error(500, &error.to_string()),
        },
        "/api/usage/daily" => match db.usage_series(&params.query, params.granularity) {
            Ok(series) => ok_json(&series),
            Err(error) => Response::error(500, &error.to_string()),
        },
        "/api/usage/models" => match db.usage_model_stats(&params.query) {
            Ok(models) => ok_json(&models),
            Err(error) => Response::error(500, &error.to_string()),
        },
        "/api/usage/providers" => match db.usage_provider_stats(&params.query) {
            Ok(providers) => ok_json(&providers),
            Err(error) => Response::error(500, &error.to_string()),
        },
        "/api/usage/filters" => match db.usage_filter_options(&params.query) {
            Ok(filters) => ok_json(&filters),
            Err(error) => Response::error(500, &error.to_string()),
        },
        _ => Response::error(404, "not found"),
    }
}

/// `GET /api/usage/overview`：一次请求返回汇总卡片、趋势序列、模型/供应商分布与级联筛选项。
fn overview_json(db: &Database, params: &ParsedUsageParams) -> Response {
    let summary = match db.usage_summary(&params.query) {
        Ok(summary) => summary,
        Err(error) => return Response::error(500, &error.to_string()),
    };
    let daily = match db.usage_series(&params.query, params.granularity) {
        Ok(daily) => daily,
        Err(error) => return Response::error(500, &error.to_string()),
    };
    let models = match db.usage_model_stats(&params.query) {
        Ok(models) => models,
        Err(error) => return Response::error(500, &error.to_string()),
    };
    let providers = match db.usage_provider_stats(&params.query) {
        Ok(providers) => providers,
        Err(error) => return Response::error(500, &error.to_string()),
    };
    let filters = match db.usage_filter_options(&params.query) {
        Ok(filters) => filters,
        Err(error) => return Response::error(500, &error.to_string()),
    };

    let body = json!({
        "summary": summary,
        "daily": daily,
        "models": models,
        "providers": providers,
        "filters": filters,
    });
    ok_json(&body)
}

/// `GET /api/usage/logs`：返回分页请求明细列表。
fn logs_json(db: &Database, params: &ParsedUsageParams) -> Response {
    match db.usage_logs(&params.query, params.page, params.page_size) {
        Ok(page) => ok_json(&page),
        Err(error) => Response::error(500, &error.to_string()),
    }
}

fn ok_json<T: Serialize>(value: &T) -> Response {
    match serde_json::to_string(value) {
        Ok(text) => Response::json(200, text),
        Err(error) => Response::error(500, &error.to_string()),
    }
}

/// 从请求 query 表解析筛选与分页参数。
fn parse_params(query: &HashMap<String, String>) -> Result<ParsedUsageParams, String> {
    let start_time_ms = parse_optional_i64(query, "startTime")?;
    let end_time_ms = parse_optional_i64(query, "endTime")?;
    let provider = parse_optional_string(query, "provider");
    let model = parse_optional_string(query, "model");
    let workspace_id = parse_optional_string(query, "workspaceId");
    let session_id = parse_optional_string(query, "sessionId");
    let granularity = query
        .get("granularity")
        .map(|raw| UsageGranularity::parse(raw.trim()))
        .unwrap_or(UsageGranularity::Day);
    let page = parse_optional_usize(query, "page")?.unwrap_or(1);
    let page_size = parse_optional_usize(query, "pageSize")?.unwrap_or(50);

    Ok(ParsedUsageParams {
        query: UsageQuery {
            start_time_ms,
            end_time_ms,
            provider,
            model,
            workspace_id,
            session_id,
        },
        granularity,
        page,
        page_size,
    })
}

fn parse_optional_string(query: &HashMap<String, String>, key: &str) -> Option<String> {
    query
        .get(key)
        .map(|raw| raw.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn parse_optional_i64(query: &HashMap<String, String>, key: &str) -> Result<Option<i64>, String> {
    let Some(raw) = query.get(key) else {
        return Ok(None);
    };
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Ok(None);
    }
    trimmed
        .parse::<i64>()
        .map(Some)
        .map_err(|_| format!("invalid query parameter '{key}'"))
}

fn parse_optional_usize(
    query: &HashMap<String, String>,
    key: &str,
) -> Result<Option<usize>, String> {
    let Some(raw) = query.get(key) else {
        return Ok(None);
    };
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Ok(None);
    }
    let parsed = trimmed
        .parse::<usize>()
        .map_err(|_| format!("invalid query parameter '{key}'"))?;
    if parsed == 0 {
        return Err(format!("query parameter '{key}' must be >= 1"));
    }
    Ok(Some(parsed))
}

#[cfg(test)]
mod tests;
