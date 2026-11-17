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

use super::{Database, Result, new_external_id, now_ms};

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
            let valid_rate = rate.filter(|v| v.is_finite() && *v >= 0.0).unwrap_or(0.0);
            tokens * valid_rate / 1_000_000.0
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
    ///
    /// 先只读检查是否存在待替换的 `provider`，无命中时直接返回 `Ok(0)` 避免开启写事务；
    /// 命中时在单事务内通过单条 `CASE provider WHEN ... END` 原子更新，防止名称互换造成链式覆盖。
    pub fn sync_usage_provider_names(
        &self,
        name_map: &std::collections::BTreeMap<String, String>,
    ) -> Result<usize> {
        let valid_pairs: Vec<(&str, &str)> = name_map
            .iter()
            .map(|(k, v)| (k.trim(), v.trim()))
            .filter(|(k, v)| !k.is_empty() && !v.is_empty() && k != v)
            .collect();
        if valid_pairs.is_empty() {
            return Ok(0);
        }

        let mut existing_stmt = self
            .conn
            .prepare("SELECT DISTINCT provider FROM usage_log")?;
        let existing_providers: std::collections::HashSet<String> = existing_stmt
            .query_map([], |row| row.get::<_, String>(0))?
            .collect::<std::result::Result<_, _>>()?;

        let active_pairs: Vec<(&str, &str)> = valid_pairs
            .into_iter()
            .filter(|(k, _)| existing_providers.contains(*k))
            .collect();
        if active_pairs.is_empty() {
            return Ok(0);
        }

        let mut sql = String::from("UPDATE usage_log SET provider = CASE provider");
        let mut params: Vec<&str> = Vec::with_capacity(active_pairs.len() * 3);
        for (old_id, new_name) in &active_pairs {
            sql.push_str(" WHEN ? THEN ?");
            params.push(old_id);
            params.push(new_name);
        }
        sql.push_str(" ELSE provider END WHERE provider IN (");
        for (idx, (old_id, _)) in active_pairs.iter().enumerate() {
            if idx > 0 {
                sql.push_str(", ");
            }
            sql.push('?');
            params.push(old_id);
        }
        sql.push(')');

        let tx = self.write_transaction()?;
        let updated = tx.execute(&sql, params_from_iter(params.iter()))?;
        tx.commit()?;
        Ok(updated)
    }

    /// 为 `usage_log` 中尚未计算成本（`total_cost_usd IS NULL`）的历史记录补齐美元成本。
    ///
    /// 先只读查询缺少成本的 `(provider, model)` 组合；若无匹配的已知正数单价则直接返回 `Ok(0)`（零写锁）。
    pub fn backfill_usage_costs(&self, settings: &crate::config::ai::AiSettings) -> Result<usize> {
        let mut stmt = self.conn.prepare(
            "SELECT DISTINCT provider, model FROM usage_log WHERE total_cost_usd IS NULL",
        )?;
        let pairs: Vec<(String, String)> = stmt
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?
            .collect::<std::result::Result<_, _>>()?;
        if pairs.is_empty() {
            return Ok(0);
        }

        let priced_pairs: Vec<(String, String, crate::config::ai::ModelPricing)> = pairs
            .into_iter()
            .filter_map(|(provider, model)| {
                let pricing = settings
                    .resolve_model_pricing(&provider, &model)
                    .filter(crate::config::ai::ModelPricing::has_positive)?;
                Some((provider, model, pricing))
            })
            .collect();
        if priced_pairs.is_empty() {
            return Ok(0);
        }

        let tx = self.write_transaction()?;
        let mut updated = 0usize;
        for (provider, model, pricing) in priced_pairs {
            let in_rate = pricing.input.unwrap_or(0.0).max(0.0);
            let out_rate = pricing.output.unwrap_or(0.0).max(0.0);
            let cr_rate = pricing.cache_read.unwrap_or(0.0).max(0.0);
            let cw_rate = pricing.cache_write.unwrap_or(0.0).max(0.0);
            updated += tx.execute(
                r#"UPDATE usage_log
                   SET input_cost_usd = MAX(0, input_tokens - cache_read_tokens - cache_write_tokens) * ?1 / 1000000.0,
                       output_cost_usd = MAX(0, output_tokens) * ?2 / 1000000.0,
                       cache_read_cost_usd = MAX(0, cache_read_tokens) * ?3 / 1000000.0,
                       cache_write_cost_usd = MAX(0, cache_write_tokens) * ?4 / 1000000.0,
                       total_cost_usd = (
                           MAX(0, input_tokens - cache_read_tokens - cache_write_tokens) * ?1
                           + MAX(0, output_tokens) * ?2
                           + MAX(0, cache_read_tokens) * ?3
                           + MAX(0, cache_write_tokens) * ?4
                       ) / 1000000.0
                   WHERE total_cost_usd IS NULL AND provider = ?5 AND model = ?6"#,
                params![in_rate, out_rate, cr_rate, cw_rate, provider, model],
            )?;
        }
        tx.commit()?;
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

        let clean_cost = |val: Option<f64>| val.filter(|v| v.is_finite() && *v >= 0.0);

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
                clean_cost(entry.rates.input_cost_usd),
                clean_cost(entry.rates.output_cost_usd),
                clean_cost(entry.rates.cache_read_cost_usd),
                clean_cost(entry.rates.cache_write_cost_usd),
                clean_cost(entry.rates.total_cost_usd),
                duration_ms,
                entry.status.as_str(),
                entry.error_message.as_deref(),
                created_at_ms,
            ],
        )?;
        Ok(())
    }
}

mod queries;

#[cfg(test)]
mod tests;
