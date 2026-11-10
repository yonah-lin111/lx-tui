//! 模型调用用量统计流水表（`usage_log`）的数据模型、写入与多维聚合查询。
//!
//! 对齐 `lx-agent` 的 `usageLogService`：
//! - 单次模型请求完成后通过 [`Database::record_usage_log`] 落盘；
//! - 若未显式传入 `workspace_id`，自动根据 `session_id` 从 `agent_session` 反查补齐；
//! - `session_id` / `workspace_id` 为快照文本（无外键级联删除），会话或工作区删除后历史账单保留；
//! - 聚合查询（汇总卡、时间序列、模型排行、供应商排行、级联筛选项、分页明细）均通过参数化 SQL 完成。

use rusqlite::types::Value as SqlValue;
use rusqlite::{OptionalExtension, params, params_from_iter};
use serde::{Deserialize, Serialize};

use super::{Database, DbError, Result, new_external_id, now_ms};

/// 模型请求用途分类。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum UsagePurpose {
    #[default]
    Chat,
    Subagent,
    Compaction,
    Title,
    Recap,
}

impl UsagePurpose {
    /// 数据库与 JSON 稳定字符串表示。
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Chat => "chat",
            Self::Subagent => "subagent",
            Self::Compaction => "compaction",
            Self::Title => "title",
            Self::Recap => "recap",
        }
    }

    /// 从字符串解析用途分类。
    pub fn parse(raw: &str) -> Option<Self> {
        match raw.trim() {
            "chat" => Some(Self::Chat),
            "subagent" => Some(Self::Subagent),
            "compaction" => Some(Self::Compaction),
            "title" => Some(Self::Title),
            "recap" => Some(Self::Recap),
            _ => None,
        }
    }
}

/// 模型请求终态。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum UsageStatus {
    #[default]
    Success,
    Error,
    Aborted,
}

impl UsageStatus {
    /// 数据库与 JSON 稳定字符串表示。
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Success => "success",
            Self::Error => "error",
            Self::Aborted => "aborted",
        }
    }

    /// 从字符串解析请求终态。
    pub fn parse(raw: &str) -> Option<Self> {
        match raw.trim() {
            "success" => Some(Self::Success),
            "error" => Some(Self::Error),
            "aborted" => Some(Self::Aborted),
            _ => None,
        }
    }
}

/// 时间序列聚合粒度。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum UsageGranularity {
    Hour,
    #[default]
    Day,
}

impl UsageGranularity {
    /// 解析粒度参数（`"hour"` 为小时，其余默认按天）。
    pub fn parse(raw: &str) -> Self {
        if raw.trim().eq_ignore_ascii_case("hour") {
            Self::Hour
        } else {
            Self::Day
        }
    }

    /// 稳定字符串表示。
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Hour => "hour",
            Self::Day => "day",
        }
    }
}

/// 单次请求的四类 Token 计数。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UsageTokens {
    pub input: i64,
    pub output: i64,
    pub cache_read: i64,
    pub cache_write: i64,
}

/// 单次请求的美元成本分解（单位：USD）。
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UsageRates {
    pub input_cost_usd: Option<f64>,
    pub output_cost_usd: Option<f64>,
    pub cache_read_cost_usd: Option<f64>,
    pub cache_write_cost_usd: Option<f64>,
    pub total_cost_usd: Option<f64>,
}

impl UsageRates {
    /// 按模型单价（USD / 1,000,000 tokens）计算单次请求成本。
    ///
    /// `input_tokens` 为总输入 Token（包含 `cache_read_tokens` 与 `cache_write_tokens`），
    /// 因此新鲜输入部分为 `fresh_input = (input - cache_read - cache_write).max(0.0)`。
    pub fn compute(
        input_tokens: i64,
        output_tokens: i64,
        cache_read_tokens: i64,
        cache_write_tokens: i64,
        pricing: Option<crate::config::ai::ModelPricing>,
    ) -> Self {
        let Some(pricing) = pricing.filter(|p| p.has_any()) else {
            return Self::default();
        };
        let input = input_tokens.max(0) as f64;
        let output = output_tokens.max(0) as f64;
        let cache_read = cache_read_tokens.max(0) as f64;
        let cache_write = cache_write_tokens.max(0) as f64;
        let fresh_input = (input - cache_read - cache_write).max(0.0);

        let per_million = |tokens: f64, rate: Option<f64>| -> f64 {
            tokens * rate.unwrap_or(0.0).max(0.0) / 1_000_000.0
        };
        let input_cost = per_million(fresh_input, pricing.input);
        let output_cost = per_million(output, pricing.output);
        let cache_read_cost = per_million(cache_read, pricing.cache_read);
        let cache_write_cost = per_million(cache_write, pricing.cache_write);
        let total_cost = input_cost + output_cost + cache_read_cost + cache_write_cost;

        Self {
            input_cost_usd: Some(input_cost),
            output_cost_usd: Some(output_cost),
            cache_read_cost_usd: Some(cache_read_cost),
            cache_write_cost_usd: Some(cache_write_cost),
            total_cost_usd: Some(total_cost),
        }
    }
}

/// 写入 `usage_log` 的单条记录载荷。
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UsageLogInsert {
    pub external_id: String,
    pub session_id: Option<String>,
    pub workspace_id: Option<String>,
    pub purpose: UsagePurpose,
    pub provider: String,
    pub model: String,
    pub tokens: UsageTokens,
    pub rates: UsageRates,
    pub duration_ms: Option<i64>,
    pub status: UsageStatus,
    pub error_message: Option<String>,
    pub created_at_ms: i64,
}

/// 从 `usage_log` 读取的单条完整记录。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UsageLogRecord {
    pub id: i64,
    pub external_id: String,
    pub session_id: Option<String>,
    pub workspace_id: Option<String>,
    pub purpose: UsagePurpose,
    pub provider: String,
    pub model: String,
    pub tokens: UsageTokens,
    pub rates: UsageRates,
    pub duration_ms: Option<i64>,
    pub status: UsageStatus,
    pub error_message: Option<String>,
    pub created_at_ms: i64,
}

/// 统计看板多维筛选条件。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UsageQuery {
    pub start_time_ms: Option<i64>,
    pub end_time_ms: Option<i64>,
    pub provider: Option<String>,
    pub model: Option<String>,
    pub workspace_id: Option<String>,
    pub session_id: Option<String>,
}

/// 顶部汇总卡片统计指标。
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UsageSummary {
    pub request_count: i64,
    pub success_count: i64,
    pub error_count: i64,
    pub aborted_count: i64,
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub cache_read_tokens: i64,
    pub cache_write_tokens: i64,
    pub total_tokens: i64,
    pub priced_request_count: i64,
    pub total_cost_usd: Option<f64>,
    pub success_rate: Option<f64>,
    pub avg_duration_ms: Option<f64>,
}

/// 时间序列（按日或按小时）聚合数据点。
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UsageDailyPoint {
    pub date: String,
    pub request_count: i64,
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub cache_read_tokens: i64,
    pub cache_write_tokens: i64,
    pub total_cost_usd: Option<f64>,
}

/// 模型维度聚合统计行。
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UsageModelStats {
    pub model: String,
    pub request_count: i64,
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub cache_read_tokens: i64,
    pub cache_write_tokens: i64,
    pub total_tokens: i64,
    pub total_cost_usd: Option<f64>,
    pub avg_cost_per_request_usd: Option<f64>,
}

/// 供应商维度聚合统计行。
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UsageProviderStats {
    pub provider: String,
    pub request_count: i64,
    pub total_tokens: i64,
    pub total_cost_usd: Option<f64>,
    pub success_rate: Option<f64>,
    pub avg_duration_ms: Option<f64>,
}

/// 带可读名称的下拉筛选项（工作区 / 会话）。
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UsageNamedOption {
    pub id: String,
    pub name: String,
}

/// 筛选栏动态可选项集合。
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UsageFilterOptions {
    pub providers: Vec<String>,
    pub models: Vec<String>,
    pub workspaces: Vec<UsageNamedOption>,
    pub sessions: Vec<UsageNamedOption>,
}

/// 分页请求日志列表。
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UsageLogPage {
    pub rows: Vec<UsageLogRecord>,
    pub total: i64,
    pub page: usize,
    pub page_size: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FilterExclude {
    None,
    Provider,
    Model,
    Workspace,
}

fn non_empty(value: Option<&str>) -> Option<&str> {
    value.map(str::trim).filter(|s| !s.is_empty())
}

fn build_where(
    query: &UsageQuery,
    alias: Option<&str>,
    exclude: FilterExclude,
    extra_clause: Option<&str>,
) -> (String, Vec<SqlValue>) {
    let prefix = alias.map(|a| format!("{a}.")).unwrap_or_default();
    let mut clauses: Vec<String> = Vec::new();
    let mut params: Vec<SqlValue> = Vec::new();

    if let Some(start) = query.start_time_ms {
        clauses.push(format!("{prefix}created_at_ms >= ?"));
        params.push(SqlValue::Integer(start));
    }
    if let Some(end) = query.end_time_ms {
        clauses.push(format!("{prefix}created_at_ms <= ?"));
        params.push(SqlValue::Integer(end));
    }
    if exclude != FilterExclude::Provider
        && let Some(provider) = non_empty(query.provider.as_deref())
    {
        clauses.push(format!("{prefix}provider = ?"));
        params.push(SqlValue::Text(provider.to_string()));
    }
    if exclude != FilterExclude::Model
        && let Some(model) = non_empty(query.model.as_deref())
    {
        clauses.push(format!("{prefix}model = ?"));
        params.push(SqlValue::Text(model.to_string()));
    }
    if exclude != FilterExclude::Workspace
        && let Some(workspace_id) = non_empty(query.workspace_id.as_deref())
    {
        clauses.push(format!("{prefix}workspace_id = ?"));
        params.push(SqlValue::Text(workspace_id.to_string()));
    }
    if let Some(session_id) = non_empty(query.session_id.as_deref()) {
        clauses.push(format!("{prefix}session_id = ?"));
        params.push(SqlValue::Text(session_id.to_string()));
    }
    if let Some(extra) = extra_clause {
        clauses.push(extra.to_string());
    }

    if clauses.is_empty() {
        (String::new(), params)
    } else {
        (format!("WHERE {}", clauses.join(" AND ")), params)
    }
}

impl Database {
    /// 将 `usage_log` 中的供应商内部 ID（如 `provider-alq374`）同步替换为可读名称（`provider.name`）。
    pub fn sync_usage_provider_names(
        &self,
        name_map: &std::collections::BTreeMap<String, String>,
    ) -> Result<usize> {
        let mut updated = 0usize;
        for (raw_id, display_name) in name_map {
            let id = raw_id.trim();
            let name = display_name.trim();
            if id.is_empty() || name.is_empty() || id == name {
                continue;
            }
            updated += self.conn.execute(
                "UPDATE usage_log SET provider = ?1 WHERE provider = ?2",
                params![name, id],
            )?;
        }
        Ok(updated)
    }

    /// 写入单条模型调用统计日志（`external_id` 冲突时幂等忽略）。
    ///
    /// 若 `entry.workspace_id` 为空且提供了 `entry.session_id`，自动从 `agent_session` 反查补齐。
    pub fn record_usage_log(&self, entry: &UsageLogInsert) -> Result<()> {
        let external_id = if entry.external_id.trim().is_empty() {
            new_external_id("ulog")
        } else {
            entry.external_id.trim().to_string()
        };
        let session_id = non_empty(entry.session_id.as_deref()).map(str::to_string);
        let mut workspace_id = non_empty(entry.workspace_id.as_deref()).map(str::to_string);
        if workspace_id.is_none()
            && let Some(sid) = session_id.as_deref()
        {
            workspace_id = self
                .conn
                .query_row(
                    "SELECT workspace_id FROM agent_session WHERE external_id = ?1",
                    params![sid],
                    |row| row.get::<_, String>(0),
                )
                .optional()?;
        }
        let provider = non_empty(Some(entry.provider.as_str())).unwrap_or("unknown");
        let model = non_empty(Some(entry.model.as_str())).unwrap_or("unknown");
        let created_at_ms = if entry.created_at_ms > 0 {
            entry.created_at_ms
        } else {
            now_ms()
        };
        let duration_ms = entry.duration_ms.filter(|d| *d >= 0);

        self.conn.execute(
            r#"INSERT OR IGNORE INTO usage_log (
                   external_id,
                   session_id,
                   workspace_id,
                   purpose,
                   provider,
                   model,
                   input_tokens,
                   output_tokens,
                   cache_read_tokens,
                   cache_write_tokens,
                   input_cost_usd,
                   output_cost_usd,
                   cache_read_cost_usd,
                   cache_write_cost_usd,
                   total_cost_usd,
                   duration_ms,
                   status,
                   error_message,
                   created_at_ms
               ) VALUES (
                   ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10,
                   ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19
               )"#,
            params![
                external_id,
                session_id,
                workspace_id,
                entry.purpose.as_str(),
                provider,
                model,
                entry.tokens.input.max(0),
                entry.tokens.output.max(0),
                entry.tokens.cache_read.max(0),
                entry.tokens.cache_write.max(0),
                entry.rates.input_cost_usd,
                entry.rates.output_cost_usd,
                entry.rates.cache_read_cost_usd,
                entry.rates.cache_write_cost_usd,
                entry.rates.total_cost_usd,
                duration_ms,
                entry.status.as_str(),
                entry.error_message.as_deref(),
                created_at_ms,
            ],
        )?;
        Ok(())
    }

    /// 聚合顶部汇总卡片指标。
    pub fn usage_summary(&self, query: &UsageQuery) -> Result<UsageSummary> {
        let (where_clause, params) = build_where(query, None, FilterExclude::None, None);
        let sql = format!(
            r#"SELECT
                   COUNT(*) AS request_count,
                   COALESCE(SUM(CASE WHEN status = 'success' THEN 1 ELSE 0 END), 0) AS success_count,
                   COALESCE(SUM(CASE WHEN status = 'error' THEN 1 ELSE 0 END), 0) AS error_count,
                   COALESCE(SUM(CASE WHEN status = 'aborted' THEN 1 ELSE 0 END), 0) AS aborted_count,
                   COALESCE(SUM(input_tokens), 0) AS input_tokens,
                   COALESCE(SUM(output_tokens), 0) AS output_tokens,
                   COALESCE(SUM(cache_read_tokens), 0) AS cache_read_tokens,
                   COALESCE(SUM(cache_write_tokens), 0) AS cache_write_tokens,
                   COALESCE(SUM(CASE WHEN total_cost_usd IS NOT NULL THEN 1 ELSE 0 END), 0) AS priced_count,
                   SUM(total_cost_usd) AS total_cost_usd,
                   AVG(duration_ms) AS avg_duration_ms
               FROM usage_log
               {where_clause}"#
        );
        self.conn
            .query_row(&sql, params_from_iter(params.iter()), |row| {
                let request_count: i64 = row.get(0)?;
                let success_count: i64 = row.get(1)?;
                let error_count: i64 = row.get(2)?;
                let aborted_count: i64 = row.get(3)?;
                let input_tokens: i64 = row.get(4)?;
                let output_tokens: i64 = row.get(5)?;
                let cache_read_tokens: i64 = row.get(6)?;
                let cache_write_tokens: i64 = row.get(7)?;
                let priced_request_count: i64 = row.get(8)?;
                let raw_total_cost: Option<f64> = row.get(9)?;
                let avg_duration_ms: Option<f64> = row.get(10)?;

                let total_tokens = input_tokens + output_tokens;
                let total_cost_usd = if priced_request_count > 0 {
                    Some(raw_total_cost.unwrap_or(0.0))
                } else {
                    None
                };
                let success_rate = if request_count > 0 {
                    Some(success_count as f64 / request_count as f64 * 100.0)
                } else {
                    None
                };

                Ok(UsageSummary {
                    request_count,
                    success_count,
                    error_count,
                    aborted_count,
                    input_tokens,
                    output_tokens,
                    cache_read_tokens,
                    cache_write_tokens,
                    total_tokens,
                    priced_request_count,
                    total_cost_usd,
                    success_rate,
                    avg_duration_ms,
                })
            })
            .map_err(DbError::from)
    }

    /// 按时间粒度（小时或天）聚合趋势序列。
    pub fn usage_series(
        &self,
        query: &UsageQuery,
        granularity: UsageGranularity,
    ) -> Result<Vec<UsageDailyPoint>> {
        let bucket_expr = match granularity {
            UsageGranularity::Hour => {
                "strftime('%Y-%m-%d %H:00', created_at_ms / 1000, 'unixepoch', 'localtime')"
            }
            UsageGranularity::Day => "date(created_at_ms / 1000, 'unixepoch', 'localtime')",
        };
        let (where_clause, params) = build_where(query, None, FilterExclude::None, None);
        let sql = format!(
            r#"SELECT
                   {bucket_expr} AS bucket,
                   COUNT(*) AS request_count,
                   COALESCE(SUM(input_tokens), 0) AS input_tokens,
                   COALESCE(SUM(output_tokens), 0) AS output_tokens,
                   COALESCE(SUM(cache_read_tokens), 0) AS cache_read_tokens,
                   COALESCE(SUM(cache_write_tokens), 0) AS cache_write_tokens,
                   SUM(total_cost_usd) AS total_cost_usd
               FROM usage_log
               {where_clause}
               GROUP BY bucket
               ORDER BY bucket ASC"#
        );
        let mut statement = self.conn.prepare(&sql)?;
        let rows = statement.query_map(params_from_iter(params.iter()), |row| {
            Ok(UsageDailyPoint {
                date: row.get::<_, Option<String>>(0)?.unwrap_or_default(),
                request_count: row.get(1)?,
                input_tokens: row.get(2)?,
                output_tokens: row.get(3)?,
                cache_read_tokens: row.get(4)?,
                cache_write_tokens: row.get(5)?,
                total_cost_usd: row.get(6)?,
            })
        })?;
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .map_err(DbError::from)
    }

    /// 按模型聚合统计（优先按总成本降序，无成本模型按请求数与名称兜底）。
    pub fn usage_model_stats(&self, query: &UsageQuery) -> Result<Vec<UsageModelStats>> {
        let (where_clause, params) = build_where(query, None, FilterExclude::None, None);
        let sql = format!(
            r#"SELECT
                   model,
                   COUNT(*) AS request_count,
                   COALESCE(SUM(input_tokens), 0) AS input_tokens,
                   COALESCE(SUM(output_tokens), 0) AS output_tokens,
                   COALESCE(SUM(cache_read_tokens), 0) AS cache_read_tokens,
                   COALESCE(SUM(cache_write_tokens), 0) AS cache_write_tokens,
                   COALESCE(SUM(CASE WHEN total_cost_usd IS NOT NULL THEN 1 ELSE 0 END), 0) AS priced_count,
                   SUM(total_cost_usd) AS total_cost_usd
               FROM usage_log
               {where_clause}
               GROUP BY model
               ORDER BY (SUM(total_cost_usd) IS NULL) ASC,
                        SUM(total_cost_usd) DESC,
                        COUNT(*) DESC,
                        model ASC"#
        );
        let mut statement = self.conn.prepare(&sql)?;
        let rows = statement.query_map(params_from_iter(params.iter()), |row| {
            let model: String = row.get(0)?;
            let request_count: i64 = row.get(1)?;
            let input_tokens: i64 = row.get(2)?;
            let output_tokens: i64 = row.get(3)?;
            let cache_read_tokens: i64 = row.get(4)?;
            let cache_write_tokens: i64 = row.get(5)?;
            let priced_count: i64 = row.get(6)?;
            let raw_cost: Option<f64> = row.get(7)?;
            let total_cost_usd = if priced_count > 0 {
                Some(raw_cost.unwrap_or(0.0))
            } else {
                None
            };
            let avg_cost_per_request_usd =
                total_cost_usd.map(|cost| cost / priced_count.max(1) as f64);
            Ok(UsageModelStats {
                model,
                request_count,
                input_tokens,
                output_tokens,
                cache_read_tokens,
                cache_write_tokens,
                total_tokens: input_tokens + output_tokens,
                total_cost_usd,
                avg_cost_per_request_usd,
            })
        })?;
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .map_err(DbError::from)
    }

    /// 按供应商聚合统计（按请求数降序排列）。
    pub fn usage_provider_stats(&self, query: &UsageQuery) -> Result<Vec<UsageProviderStats>> {
        let (where_clause, params) = build_where(query, None, FilterExclude::None, None);
        let sql = format!(
            r#"SELECT
                   provider,
                   COUNT(*) AS request_count,
                   COALESCE(SUM(CASE WHEN status = 'success' THEN 1 ELSE 0 END), 0) AS success_count,
                   COALESCE(SUM(input_tokens + output_tokens), 0) AS total_tokens,
                   COALESCE(SUM(CASE WHEN total_cost_usd IS NOT NULL THEN 1 ELSE 0 END), 0) AS priced_count,
                   SUM(total_cost_usd) AS total_cost_usd,
                   AVG(duration_ms) AS avg_duration_ms
               FROM usage_log
               {where_clause}
               GROUP BY provider
               ORDER BY COUNT(*) DESC, provider ASC"#
        );
        let mut statement = self.conn.prepare(&sql)?;
        let rows = statement.query_map(params_from_iter(params.iter()), |row| {
            let provider: String = row.get(0)?;
            let request_count: i64 = row.get(1)?;
            let success_count: i64 = row.get(2)?;
            let total_tokens: i64 = row.get(3)?;
            let priced_count: i64 = row.get(4)?;
            let raw_cost: Option<f64> = row.get(5)?;
            let avg_duration_ms: Option<f64> = row.get(6)?;
            let total_cost_usd = if priced_count > 0 {
                Some(raw_cost.unwrap_or(0.0))
            } else {
                None
            };
            let success_rate = if request_count > 0 {
                Some(success_count as f64 / request_count as f64 * 100.0)
            } else {
                None
            };
            Ok(UsageProviderStats {
                provider,
                request_count,
                total_tokens,
                total_cost_usd,
                success_rate,
                avg_duration_ms,
            })
        })?;
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .map_err(DbError::from)
    }

    /// 查询级联筛选项（每个下拉维度排除自身过滤条件，联表解析工作区与会话可读名称）。
    pub fn usage_filter_options(&self, query: &UsageQuery) -> Result<UsageFilterOptions> {
        let (prov_where, prov_params) =
            build_where(query, None, FilterExclude::Provider, Some("provider != ''"));
        let mut prov_stmt = self.conn.prepare(&format!(
            "SELECT DISTINCT provider FROM usage_log {prov_where} ORDER BY provider ASC"
        ))?;
        let providers = prov_stmt
            .query_map(params_from_iter(prov_params.iter()), |row| {
                row.get::<_, String>(0)
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;

        let (model_where, model_params) =
            build_where(query, None, FilterExclude::Model, Some("model != ''"));
        let mut model_stmt = self.conn.prepare(&format!(
            "SELECT DISTINCT model FROM usage_log {model_where} ORDER BY model ASC"
        ))?;
        let models = model_stmt
            .query_map(params_from_iter(model_params.iter()), |row| {
                row.get::<_, String>(0)
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;

        let (ws_where, ws_params) = build_where(
            query,
            Some("ul"),
            FilterExclude::Workspace,
            Some("ul.workspace_id IS NOT NULL AND ul.workspace_id != ''"),
        );
        let mut ws_stmt = self.conn.prepare(&format!(
            r#"SELECT
                   ul.workspace_id,
                   COALESCE(NULLIF(TRIM(MAX(w.name)), ''), ul.workspace_id) AS ws_name
               FROM usage_log ul
               LEFT JOIN workspace w ON w.external_id = ul.workspace_id
               {ws_where}
               GROUP BY ul.workspace_id
               ORDER BY ws_name ASC, ul.workspace_id ASC"#
        ))?;
        let workspaces = ws_stmt
            .query_map(params_from_iter(ws_params.iter()), |row| {
                Ok(UsageNamedOption {
                    id: row.get(0)?,
                    name: row.get(1)?,
                })
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;

        let session_only_query = UsageQuery {
            start_time_ms: query.start_time_ms,
            end_time_ms: query.end_time_ms,
            provider: None,
            model: None,
            workspace_id: query.workspace_id.clone(),
            session_id: None,
        };
        let (ses_where, ses_params) = build_where(
            &session_only_query,
            Some("ul"),
            FilterExclude::None,
            Some("ul.session_id IS NOT NULL AND ul.session_id != ''"),
        );
        let mut ses_stmt = self.conn.prepare(&format!(
            r#"SELECT
                   ul.session_id,
                   MAX(s.title) AS title
               FROM usage_log ul
               LEFT JOIN agent_session s ON s.external_id = ul.session_id
               {ses_where}
               GROUP BY ul.session_id
               ORDER BY MAX(ul.created_at_ms) DESC, ul.session_id ASC"#
        ))?;
        let sessions = ses_stmt
            .query_map(params_from_iter(ses_params.iter()), |row| {
                let id: String = row.get(0)?;
                let title: Option<String> = row.get(1)?;
                let name = title
                    .as_deref()
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .map(str::to_string)
                    .unwrap_or_else(|| id.chars().take(8).collect());
                Ok(UsageNamedOption { id, name })
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;

        Ok(UsageFilterOptions {
            providers,
            models,
            workspaces,
            sessions,
        })
    }

    /// 分页查询请求明细表（按时间与自增 id 倒序）。
    pub fn usage_logs(
        &self,
        query: &UsageQuery,
        page: usize,
        page_size: usize,
    ) -> Result<UsageLogPage> {
        let page = page.max(1);
        let page_size = if page_size == 0 {
            50
        } else {
            page_size.clamp(1, 200)
        };
        let offset = (page - 1) * page_size;

        let (where_clause, mut params) = build_where(query, None, FilterExclude::None, None);
        let count_sql = format!("SELECT COUNT(*) FROM usage_log {where_clause}");
        let total: i64 =
            self.conn
                .query_row(&count_sql, params_from_iter(params.iter()), |row| {
                    row.get(0)
                })?;

        params.push(SqlValue::Integer(page_size as i64));
        params.push(SqlValue::Integer(offset as i64));
        let rows_sql = format!(
            r#"SELECT
                   id,
                   external_id,
                   session_id,
                   workspace_id,
                   purpose,
                   provider,
                   model,
                   input_tokens,
                   output_tokens,
                   cache_read_tokens,
                   cache_write_tokens,
                   input_cost_usd,
                   output_cost_usd,
                   cache_read_cost_usd,
                   cache_write_cost_usd,
                   total_cost_usd,
                   duration_ms,
                   status,
                   error_message,
                   created_at_ms
               FROM usage_log
               {where_clause}
               ORDER BY created_at_ms DESC, id DESC
               LIMIT ? OFFSET ?"#
        );
        let mut statement = self.conn.prepare(&rows_sql)?;
        let rows = statement
            .query_map(params_from_iter(params.iter()), |row| {
                let raw_purpose: String = row.get(4)?;
                let raw_status: String = row.get(17)?;
                Ok(UsageLogRecord {
                    id: row.get(0)?,
                    external_id: row.get(1)?,
                    session_id: row.get(2)?,
                    workspace_id: row.get(3)?,
                    purpose: UsagePurpose::parse(&raw_purpose).unwrap_or(UsagePurpose::Chat),
                    provider: row.get(5)?,
                    model: row.get(6)?,
                    tokens: UsageTokens {
                        input: row.get(7)?,
                        output: row.get(8)?,
                        cache_read: row.get(9)?,
                        cache_write: row.get(10)?,
                    },
                    rates: UsageRates {
                        input_cost_usd: row.get(11)?,
                        output_cost_usd: row.get(12)?,
                        cache_read_cost_usd: row.get(13)?,
                        cache_write_cost_usd: row.get(14)?,
                        total_cost_usd: row.get(15)?,
                    },
                    duration_ms: row.get(16)?,
                    status: UsageStatus::parse(&raw_status).unwrap_or(UsageStatus::Success),
                    error_message: row.get(18)?,
                    created_at_ms: row.get(19)?,
                })
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;

        Ok(UsageLogPage {
            rows,
            total,
            page,
            page_size,
        })
    }
}

#[cfg(test)]
mod tests;
