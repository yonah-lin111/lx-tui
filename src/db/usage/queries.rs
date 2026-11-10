//! `usage_log` 看板聚合与分页查询（`usage_summary`、`usage_series`、`usage_model_stats`、`usage_provider_stats`、`usage_filter_options`、`usage_logs`）。

use rusqlite::params_from_iter;
use rusqlite::types::Value as SqlValue;

use super::{
    FilterExclude, UsageDailyPoint, UsageFilterOptions, UsageGranularity, UsageLogPage,
    UsageLogRecord, UsageModelStats, UsageNamedOption, UsageProviderStats, UsagePurpose,
    UsageQuery, UsageRates, UsageStatus, UsageSummary, UsageTokens, build_where,
};
use crate::db::{Database, DbError, Result};

impl Database {
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
        let base_query = UsageQuery {
            session_id: None,
            ..query.clone()
        };
        let (prov_where, prov_params) = build_where(
            &base_query,
            None,
            FilterExclude::Provider,
            Some("provider != ''"),
        );
        let mut prov_stmt = self.conn.prepare(&format!(
            "SELECT DISTINCT provider FROM usage_log {prov_where} ORDER BY provider ASC"
        ))?;
        let providers = prov_stmt
            .query_map(params_from_iter(prov_params.iter()), |row| {
                row.get::<_, String>(0)
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;

        let (model_where, model_params) =
            build_where(&base_query, None, FilterExclude::Model, Some("model != ''"));
        let mut model_stmt = self.conn.prepare(&format!(
            "SELECT DISTINCT model FROM usage_log {model_where} ORDER BY model ASC"
        ))?;
        let models = model_stmt
            .query_map(params_from_iter(model_params.iter()), |row| {
                row.get::<_, String>(0)
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;

        let (ws_where, ws_params) = build_where(
            &base_query,
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
               ORDER BY (MAX(w.name) IS NULL OR TRIM(MAX(w.name)) = '') ASC, ws_name ASC, ul.workspace_id ASC"#
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
                    .unwrap_or_else(|| {
                        if let Some(rest) = id.strip_prefix("sess-")
                            && rest.len() > 6
                        {
                            format!("sess-{}", &rest[rest.len() - 6..])
                        } else {
                            id.chars().take(8).collect()
                        }
                    });
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
