//! 模型流式请求用量采集器：从 SSE 增量流提取单步 Token、解析单价并落库 `usage_log`。
//!
//! 挂载在唯一网络出口 `chat::client::run_stream_with_log` 末尾，覆盖：
//! - 主对话轮次（`chat` / `exec`）
//! - 子代理轮次（`subagent`）
//! - 上下文压缩（手动 `/compact`、CLI `session compact` 与子代理自动压缩 `maybe_auto_compact`）
//! - 会话标题生成（`title`）
//! - 轮次摘要生成（`recap`）
//!
//! 生产路径通过后台线程写库，任何配置或 SQLite 异常仅记录 `tracing::warn!`，绝不阻断主流。

use crate::chat::client::StreamLog;
use crate::chat::events::{ChatDelta, ChatRequest, RequestPurpose, StopReason};
use crate::config::ai::ModelPricing;
use crate::db::chat::Usage;
use crate::db::usage::{UsageLogInsert, UsagePurpose, UsageRates, UsageStatus, UsageTokens};

#[cfg(test)]
std::thread_local! {
    static TEST_DB_PATH: std::cell::RefCell<Option<std::path::PathBuf>> =
        const { std::cell::RefCell::new(None) };
}

/// 测试辅助：在当前线程作用域内将 `record_stream_finish` 的落库目标重定向到指定 SQLite 文件。
#[cfg(test)]
pub fn with_test_db_path<F, R>(path: impl Into<std::path::PathBuf>, f: F) -> R
where
    F: FnOnce() -> R,
{
    struct Guard(Option<std::path::PathBuf>);
    impl Drop for Guard {
        fn drop(&mut self) {
            let previous = self.0.take();
            TEST_DB_PATH.with(|slot| *slot.borrow_mut() = previous);
        }
    }
    let previous = TEST_DB_PATH.with(|slot| slot.borrow_mut().replace(path.into()));
    let _guard = Guard(previous);
    f()
}

/// 从单次流式响应的增量序列合并提取 Token 用量（与 `agent_loop::step_usage` 同口径）。
///
/// 同一次 HTTP 响应可能先报 input、再报 output（如 Anthropic / Google）；
/// 后到达的非 `None` 字段覆盖先前值，未出现的字段保持 `None`。
pub fn extract_step_usage(deltas: &[ChatDelta]) -> Usage {
    let mut step = Usage::default();
    for delta in deltas {
        let ChatDelta::Usage(part) = delta else {
            continue;
        };
        if part.input.is_some() {
            step.input = part.input;
        }
        if part.output.is_some() {
            step.output = part.output;
        }
        if part.cache_read.is_some() {
            step.cache_read = part.cache_read;
        }
        if part.cache_write.is_some() {
            step.cache_write = part.cache_write;
        }
    }
    step
}

/// 根据网络流上下文 [`StreamLog`] 与请求快照 [`ChatRequest`] 推导统计用途分类。
///
/// 判定优先级：
/// 1. `log.kind == "title"` 或 `"title_block"` → `Title`
///    （`build_title_request` 会复用 `RequestPurpose::CompactSummary` 以禁用工具，故必须优先按 `log.kind` 判定）；
/// 2. `log.kind == "recap"` → `Recap`
///    （`build_recap_request` 同样复用 `RequestPurpose::CompactSummary`）；
/// 3. `log.kind == "compact"` 或 `RequestPurpose::CompactSummary` → `Compaction`
///    （确保子代理自动压缩在 `log.kind == "subagent"` 下仍归类为压缩而非普通子代理对话）；
/// 4. `log.kind == "subagent"` 或 `RequestPurpose::Subagent(_)` → `Subagent`；
/// 5. 其余（`chat` / `exec` 等）→ `Chat`。
pub fn resolve_purpose(log: &StreamLog, request: &ChatRequest) -> UsagePurpose {
    if log.kind == "title" || log.kind == "title_block" {
        UsagePurpose::Title
    } else if log.kind == "recap" {
        UsagePurpose::Recap
    } else if log.kind == "compact" || matches!(request.purpose, RequestPurpose::CompactSummary) {
        UsagePurpose::Compaction
    } else if log.kind == "subagent" || matches!(request.purpose, RequestPurpose::Subagent(_)) {
        UsagePurpose::Subagent
    } else {
        UsagePurpose::Chat
    }
}

/// 将流式结束原因 [`StopReason`] 映射为统计终态与可选错误码。
pub fn resolve_status(stop: &StopReason) -> (UsageStatus, Option<String>) {
    match stop {
        StopReason::Complete => (UsageStatus::Success, None),
        StopReason::Cancelled => (UsageStatus::Aborted, None),
        StopReason::Failed { code } => (UsageStatus::Error, Some(code.clone())),
    }
}

/// 构造待写入 `usage_log` 的单条记录载荷（纯函数，不触碰磁盘或数据库）。
pub fn build_usage_log_insert(
    log: &StreamLog,
    request: &ChatRequest,
    deltas: &[ChatDelta],
    stop: &StopReason,
    duration_ms: i64,
    pricing: Option<ModelPricing>,
) -> UsageLogInsert {
    let usage = extract_step_usage(deltas);
    let tokens = UsageTokens {
        input: usage.input.unwrap_or(0).max(0),
        output: usage.output.unwrap_or(0).max(0),
        cache_read: usage.cache_read.unwrap_or(0).max(0),
        cache_write: usage.cache_write.unwrap_or(0).max(0),
    };
    let rates = UsageRates::compute(
        tokens.input,
        tokens.output,
        tokens.cache_read,
        tokens.cache_write,
        pricing,
    );
    let (status, error_message) = resolve_status(stop);
    let session_id = log
        .session_id
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string);
    let provider = {
        let trimmed = request.model.provider.trim();
        if trimmed.is_empty() {
            "unknown".to_string()
        } else {
            trimmed.to_string()
        }
    };
    let model = {
        let trimmed = request.model.model.trim();
        if trimmed.is_empty() {
            "unknown".to_string()
        } else {
            trimmed.to_string()
        }
    };

    UsageLogInsert {
        external_id: crate::db::new_external_id("ulog"),
        session_id,
        workspace_id: None,
        purpose: resolve_purpose(log, request),
        provider,
        model,
        tokens,
        rates,
        duration_ms: Some(duration_ms.max(0)),
        status,
        error_message,
        created_at_ms: crate::db::now_ms(),
    }
}

/// 直接向指定 [`crate::db::Database`] 句柄写入单次流式请求的统计记录。
pub fn record_stream_to_db(
    db: &crate::db::Database,
    log: &StreamLog,
    request: &ChatRequest,
    deltas: &[ChatDelta],
    stop: &StopReason,
    duration_ms: i64,
    pricing: Option<ModelPricing>,
) -> crate::db::Result<()> {
    let entry = build_usage_log_insert(log, request, deltas, stop, duration_ms, pricing);
    db.record_usage_log(&entry)
}

/// 从运行时配置（`ai.json` + `connect.json`）与内置目录解析供应商可读名称与模型单价。
#[cfg(not(test))]
fn resolve_runtime_provider_and_pricing(
    provider_id: &str,
    model_id: &str,
) -> (String, Option<ModelPricing>) {
    let runtime_settings = crate::config::ai::ai_config_path().ok().and_then(|path| {
        crate::config::ai::connect::load_runtime_ai(&path)
            .ok()
            .flatten()
    });
    let provider_name = runtime_settings
        .as_ref()
        .map(|settings| settings.resolve_provider_name(provider_id))
        .unwrap_or_else(|| {
            let trimmed = provider_id.trim();
            if trimmed.is_empty() {
                "unknown".to_string()
            } else {
                trimmed.to_string()
            }
        });
    let pricing = runtime_settings
        .as_ref()
        .and_then(|settings| settings.resolve_model_pricing(provider_id, model_id))
        .or_else(|| crate::config::ai::presets::catalog_model_pricing(provider_id, model_id));
    (provider_name, pricing)
}

/// 在 `run_stream_with_log` 结束时采集并持久化单次模型请求统计。
///
/// - `log.kind == "unknown"` 的无上下文流（如单元测试直接调用的 `run_stream`）默认跳过；
/// - 测试编译（`#[cfg(test)]`）下仅当显式设置了 `with_test_db_path` 或 `LX_TUI_DB_PATH` 时落库，
///   严禁污染开发者的 `~/.lx-t-dev/db/lx.db`；
/// - 生产编译（`#[cfg(not(test))]`）下在后台线程写入默认数据库，失败只记 `tracing::warn!`。
pub fn record_stream_finish(
    log: &StreamLog,
    request: &ChatRequest,
    deltas: &[ChatDelta],
    stop: &StopReason,
    duration_ms: i64,
) {
    #[cfg(test)]
    {
        let explicit_test_db = TEST_DB_PATH.with(|slot| slot.borrow().clone());
        if log.kind == "unknown" && explicit_test_db.is_none() {
            return;
        }
        let target_path = explicit_test_db.or_else(|| {
            std::env::var_os("LX_TUI_DB_PATH")
                .filter(|path| !path.is_empty())
                .map(std::path::PathBuf::from)
        });
        let Some(db_path) = target_path else {
            return;
        };
        let pricing = crate::config::ai::presets::catalog_model_pricing(
            &request.model.provider,
            &request.model.model,
        );
        let entry = build_usage_log_insert(log, request, deltas, stop, duration_ms, pricing);
        if let Err(error) =
            crate::db::Database::open(&db_path).and_then(|db| db.record_usage_log(&entry))
        {
            tracing::warn!(
                target: "lx::usage",
                %error,
                "failed to record usage log in test database"
            );
        }
    }

    #[cfg(not(test))]
    {
        if log.kind == "unknown" {
            return;
        }
        let (provider_name, pricing) =
            resolve_runtime_provider_and_pricing(&request.model.provider, &request.model.model);
        let mut entry = build_usage_log_insert(log, request, deltas, stop, duration_ms, pricing);
        entry.provider = provider_name;
        std::thread::spawn(move || {
            let linked_worktree = crate::db::is_dev_environment()
                .then(|| crate::git::linked_worktree_root(std::path::Path::new(".")))
                .flatten();
            if let Err(error) = crate::db::Database::open_default(linked_worktree.as_deref())
                .and_then(|db| db.record_usage_log(&entry))
            {
                tracing::warn!(
                    target: "lx::usage",
                    %error,
                    "failed to record usage log"
                );
            }
        });
    }
}

#[cfg(test)]
mod tests;
