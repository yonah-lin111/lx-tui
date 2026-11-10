//! 进程日志：按天滚动、级别热更新、关联字段与 payload 开关。
//!
//! 日志目录由 `platform::log::log_dir` 推导；每行由 `RoleFormat` 添加
//! `role` 与 `pid` 前缀。设置页保存后经 `apply` 热更新过滤器。

mod payload;
mod redact;

pub use payload::{PayloadCapture, PayloadMeta, PayloadRequest};
pub use redact::{redacted_headers, sanitize_text, url_origin, url_origin_and_path};

use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use tracing::Subscriber;
use tracing_subscriber::EnvFilter;
use tracing_subscriber::fmt::format::{Format, Full, Writer};
use tracing_subscriber::fmt::{FmtContext, FormatEvent, FormatFields};
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::registry::{LookupSpan, Registry};
use tracing_subscriber::reload;
use tracing_subscriber::util::SubscriberInitExt;

use crate::config::Config;

/// 日志保留天数（含今天）。
const RETENTION_DAYS: u32 = 7;
/// 清理线程检查间隔。
const CLEANUP_INTERVAL: Duration = Duration::from_secs(6 * 60 * 60);
/// 设置页可选级别。
pub const LEVELS: [&str; 6] = ["trace", "debug", "info", "warn", "error", "off"];

static RELOAD: OnceLock<reload::Handle<EnvFilter, Registry>> = OnceLock::new();
static ACTIVE_DIR: OnceLock<PathBuf> = OnceLock::new();
static PAYLOAD: AtomicBool = AtomicBool::new(false);
static ENV_FILTER_LOCKED: AtomicBool = AtomicBool::new(false);
static REQUEST_SEQ: AtomicU64 = AtomicU64::new(0);

/// 日志后台写线程守卫；进程生命周期内必须持有，Drop 时 flush。
pub struct LogGuard {
    _worker: tracing_appender::non_blocking::WorkerGuard,
}

/// 初始化日志。失败时降级到临时目录/空 writer，不阻断应用启动。
pub fn init(config: &Config, role: &'static str) -> LogGuard {
    let dir = crate::platform::log::log_dir()
        .unwrap_or_else(|_| std::env::temp_dir().join("lx-tui-logs"));
    // 有意忽略：目录创建/权限收紧失败时继续尝试写日志，最坏降级到空 writer。
    let _ = std::fs::create_dir_all(&dir);
    let _ = crate::platform::restrict_dir_permissions(&dir);
    let _ = ACTIVE_DIR.set(dir.clone());
    cleanup_old_logs(&dir, utc_today(), RETENTION_DAYS);
    spawn_cleanup_thread(dir.clone());

    let appender = tracing_appender::rolling::Builder::new()
        .rotation(tracing_appender::rolling::Rotation::DAILY)
        .filename_prefix("lx-tui.log")
        .build(&dir);
    let (writer, worker) = match appender {
        Ok(appender) => {
            let today_file = dir.join(format!("lx-tui.log.{}", format_date(utc_today())));
            let _ = crate::platform::restrict_file_permissions(&today_file);
            tracing_appender::non_blocking(appender)
        }
        // 有意忽略：文件 appender 初始化失败时降级为丢弃日志，保证应用可用。
        Err(_) => tracing_appender::non_blocking(std::io::sink()),
    };

    let (filter, env_locked, env_invalid) = initial_filter(config);
    let (filter_layer, handle) = reload::Layer::new(filter);
    let format = Format::default().with_ansi(false).with_target(true);
    let layer = tracing_subscriber::fmt::layer()
        .event_format(RoleFormat {
            role,
            inner: format,
        })
        .with_writer(writer);

    // 有意忽略：重复初始化（嵌入/测试）时保留已有 subscriber。
    let initialized = tracing_subscriber::registry()
        .with(filter_layer)
        .with(layer)
        .try_init()
        .is_ok();
    if initialized {
        let _ = RELOAD.set(handle);
    }
    ENV_FILTER_LOCKED.store(env_locked, Ordering::Relaxed);
    PAYLOAD.store(resolve_payload(config), Ordering::Relaxed);
    if env_invalid {
        tracing::warn!(
            target: "lx::logging",
            "LX_TUI_LOG is invalid; using settings log level"
        );
    }
    LogGuard { _worker: worker }
}

/// 设置页保存后应用新配置：payload 立即生效；级别在无环境变量覆盖时热更新。
pub fn apply(config: &Config) {
    PAYLOAD.store(resolve_payload(config), Ordering::Relaxed);
    if ENV_FILTER_LOCKED.load(Ordering::Relaxed) {
        return;
    }
    if let Some(handle) = RELOAD.get() {
        let level = normalize_level(&config.log_level);
        if let Ok(filter) = EnvFilter::try_new(filter_directives(level)) {
            // 有意忽略：reload 失败时保持旧过滤器，下一次保存再试。
            let _ = handle.reload(filter);
        }
    }
}

/// 当前请求是否记录原始 payload。
pub fn payload_enabled() -> bool {
    PAYLOAD.load(Ordering::Relaxed)
}

/// 日志级别是否被环境变量 `LX_TUI_LOG` 锁定。
pub fn env_filter_locked() -> bool {
    ENV_FILTER_LOCKED.load(Ordering::Relaxed)
}

/// payload 开关是否被环境变量 `LX_TUI_LOG_PAYLOAD` 锁定。
pub fn payload_env_locked() -> bool {
    std::env::var("LX_TUI_LOG_PAYLOAD")
        .ok()
        .and_then(|value| parse_env_bool(&value))
        .is_some()
}

/// 生成一次网络请求的短关联 ID。
pub fn new_request_id() -> String {
    let seq = REQUEST_SEQ.fetch_add(1, Ordering::Relaxed);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_nanos() as u64)
        .unwrap_or_default();
    let mixed = nanos ^ seq.wrapping_mul(0x9e37_79b9_7f4a_7c15);
    format!("req-{:08x}", mixed as u32)
}

/// 本次进程实际使用的日志目录。
pub fn active_log_dir() -> Option<PathBuf> {
    ACTIVE_DIR.get().cloned()
}

/// 今天正在写入的诊断日志路径。
pub fn active_log_file() -> Option<PathBuf> {
    let dir = ACTIVE_DIR.get()?;
    Some(dir.join(format!("lx-tui.log.{}", format_date(utc_today()))))
}

/// 设置页等级只作用于 `lx`/`lx_tui`，依赖库固定 warn。
pub(crate) fn filter_directives(level: &str) -> String {
    let level = normalize_level(level);
    format!("lx={level},lx_tui={level},warn")
}

/// 非法或缺失级别回退 `info`。
pub(crate) fn normalize_level(level: &str) -> &'static str {
    match level.trim().to_ascii_lowercase().as_str() {
        "trace" => "trace",
        "debug" => "debug",
        "warn" => "warn",
        "error" => "error",
        "off" => "off",
        _ => "info",
    }
}

/// payload 开关：环境变量优先于设置页。
pub(crate) fn resolve_payload(config: &Config) -> bool {
    std::env::var("LX_TUI_LOG_PAYLOAD")
        .ok()
        .and_then(|value| parse_env_bool(&value))
        .unwrap_or(config.log_payload)
}

pub(crate) fn parse_env_bool(value: &str) -> Option<bool> {
    match value.trim().to_ascii_lowercase().as_str() {
        "1" | "true" | "on" | "yes" => Some(true),
        "0" | "false" | "off" | "no" => Some(false),
        _ => None,
    }
}

fn initial_filter(config: &Config) -> (EnvFilter, bool, bool) {
    let env_value = std::env::var("LX_TUI_LOG").ok();
    resolve_filter_value(env_value.as_deref(), &config.log_level)
}

pub(crate) fn resolve_filter_value(
    env_value: Option<&str>,
    config_level: &str,
) -> (EnvFilter, bool, bool) {
    let Some(raw) = env_value.map(str::trim).filter(|value| !value.is_empty()) else {
        return (build_filter(config_level), false, false);
    };
    match EnvFilter::try_new(raw) {
        Ok(filter) if !filter.to_string().is_empty() => (filter, true, false),
        Ok(_) | Err(_) => (build_filter(config_level), false, true),
    }
}

fn build_filter(level: &str) -> EnvFilter {
    EnvFilter::new(filter_directives(level))
}

/// 每行前缀 `role` 与 `pid`，所有入口共用同一份按天日志。
struct RoleFormat {
    role: &'static str,
    inner: Format<Full>,
}

impl<S, N> FormatEvent<S, N> for RoleFormat
where
    S: Subscriber + for<'a> LookupSpan<'a>,
    N: for<'a> FormatFields<'a> + 'static,
{
    fn format_event(
        &self,
        ctx: &FmtContext<'_, S, N>,
        mut writer: Writer<'_>,
        event: &tracing::Event<'_>,
    ) -> std::fmt::Result {
        write!(writer, "role={} pid={} ", self.role, std::process::id())?;
        self.inner.format_event(ctx, writer, event)
    }
}

fn spawn_cleanup_thread(dir: PathBuf) {
    // 有意忽略：清理线程创建失败不影响日志写入。
    let _ = std::thread::Builder::new()
        .name("lx-log-cleanup".into())
        .spawn(move || {
            loop {
                std::thread::sleep(CLEANUP_INTERVAL);
                cleanup_old_logs(&dir, utc_today(), RETENTION_DAYS);
            }
        });
}

/// 删除早于 `keep_days` 的按天日志与 payload 文件；无法解析的文件不动。
pub(crate) fn cleanup_old_logs(dir: &Path, today: (i32, u32, u32), keep_days: u32) {
    let keep = i64::from(keep_days.max(1));
    let Some(cutoff) = days_from_civil(today).checked_sub(keep - 1) else {
        return;
    };
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        let Some(date) = parse_log_date(&name) else {
            continue;
        };
        if days_from_civil(date) < cutoff {
            // 有意忽略：并发删除/权限失败不阻断清理。
            let _ = std::fs::remove_file(entry.path());
        }
    }
}

/// 解析 `lx-tui.log.YYYY-MM-DD` 或 `payload-YYYY-MM-DD.jsonl` 的日期部分。
pub(crate) fn parse_log_date(name: &str) -> Option<(i32, u32, u32)> {
    let date = if let Some(rest) = name.strip_prefix("lx-tui.log.") {
        rest
    } else if let Some(rest) = name.strip_prefix("payload-") {
        rest.strip_suffix(".jsonl")?
    } else {
        return None;
    };
    parse_date(date)
}

fn parse_date(text: &str) -> Option<(i32, u32, u32)> {
    let (year, rest) = text.split_once('-')?;
    let (month, day) = rest.split_once('-')?;
    let year = year.parse().ok()?;
    let month = month.parse().ok()?;
    let day = day.parse().ok()?;
    if !(1..=12).contains(&month) || !(1..=31).contains(&day) {
        return None;
    }
    Some((year, month, day))
}

pub(crate) fn format_date(date: (i32, u32, u32)) -> String {
    format!("{:04}-{:02}-{:02}", date.0, date.1, date.2)
}

pub(crate) fn utc_today() -> (i32, u32, u32) {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs() as i64)
        .unwrap_or_default();
    utc_date_from_unix_days(secs.div_euclid(86_400))
}

pub(crate) fn utc_timestamp_now() -> String {
    let elapsed = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    let secs = elapsed.as_secs() as i64;
    let days = secs.div_euclid(86_400);
    let second_of_day = secs.rem_euclid(86_400);
    let (year, month, day) = utc_date_from_unix_days(days);
    let hour = second_of_day / 3_600;
    let minute = (second_of_day % 3_600) / 60;
    let second = second_of_day % 60;
    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}.{:03}Z",
        year,
        month,
        day,
        hour,
        minute,
        second,
        elapsed.subsec_millis()
    )
}

/// Unix 天数 → UTC 年月日（Howard Hinnant civil_from_days）。
pub(crate) fn utc_date_from_unix_days(days: i64) -> (i32, u32, u32) {
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let year = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = if month <= 2 { year + 1 } else { year };
    (year as i32, month as u32, day as u32)
}

/// UTC 年月日 → Unix 天数（days_from_civil）。
pub(crate) fn days_from_civil(date: (i32, u32, u32)) -> i64 {
    let (year, month, day) = date;
    let year = i64::from(year) - if month <= 2 { 1 } else { 0 };
    let era = if year >= 0 { year } else { year - 399 } / 400;
    let yoe = year - era * 400;
    let month = i64::from(month);
    let doy = (153 * (if month > 2 { month - 3 } else { month + 9 }) + 2) / 5 + i64::from(day) - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

#[cfg(test)]
mod tests;
