//! 环境快照的**只读事实采集**：OS/架构、终端、shell、本地时间/时区与网络状态。
//!
//! 本模块只负责“能判定的事实”，不负责排版：渲染、转义与长度上限在
//! [`crate::chat::prompt`] 的纯函数里完成（便于无平台依赖地测试）。
//!
//! 采集边界（对应 design §2.2）：
//! - **绝不**读取任意环境变量的值，只判定少数与平台行为直接相关的存在性
//!   （`TERM` / `COLORTERM` / `TMUX` / `STY`）并只保留白名单字符集；
//! - 不读取用户目录内容、不枚举进程、不拼接完整命令行；
//! - 网络探测有严格超时与 TTL，绝不在事件循环里做无界阻塞。

use std::net::{TcpStream, ToSocketAddrs};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

/// 网络探测的目标与超时。
///
/// 只做一次 TCP 建连（不发送任何应用层数据），因此不会泄露内容；DNS 解析与
/// 建连各自有上限，最坏情况仍是有界的。
const NETWORK_PROBE_HOST: &str = "1.1.1.1:443";
/// 单次网络探测的建连超时。
const NETWORK_PROBE_TIMEOUT: Duration = Duration::from_millis(80);
/// 网络探测结果的缓存有效期：环境快照每个 turn 只读一次，不需要实时。
const NETWORK_CACHE_TTL: Duration = Duration::from_secs(30);

/// 终端字段值的最大字符数（避免异常超长 TERM 撑爆提示词）。
const MAX_TERM_CHARS: usize = 40;

/// 平台只读事实：环境快照渲染的**唯一输入**。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EnvironmentFacts {
    /// `macOS` / `Linux` / `Windows` / 未知平台。
    pub os: String,
    /// 架构（`aarch64` / `x86_64` / …）。
    pub arch: String,
    /// 默认 shell 的可执行文件路径或名字。
    pub shell: String,
    /// 终端类型（`TERM` 的白名单化值）；未知为 None。
    pub terminal: Option<String>,
    /// 是否运行在 tmux 内。
    pub tmux: bool,
    /// 是否运行在 GNU screen 内。
    pub screen: bool,
    /// 本地日期（`YYYY-MM-DD`）。
    pub date: String,
    /// 本地时间（`HH:MM`）。
    pub time: String,
    /// 时区偏移（`UTC+08:00`）或 IANA 名；未知为 None。
    pub timezone: Option<String>,
    /// 网络是否可用；无法判定时为 None（不猜）。
    pub network: Option<bool>,
}

impl EnvironmentFacts {
    /// 采集当前进程的平台事实（只读、无副作用、有界）。
    pub fn capture() -> Self {
        let (date, time) = local_datetime().unwrap_or_else(utc_datetime);
        Self {
            os: os_name().to_string(),
            arch: std::env::consts::ARCH.to_string(),
            shell: super::default_shell(),
            terminal: terminal_kind(),
            tmux: std::env::var_os("TMUX").is_some(),
            screen: std::env::var_os("STY").is_some(),
            date,
            time,
            timezone: timezone_name(),
            network: network_available(),
        }
    }
}

/// OS 展示名（未知平台给出稳定值，不猜）。
pub fn os_name() -> &'static str {
    #[cfg(target_os = "macos")]
    {
        "macOS"
    }
    #[cfg(target_os = "linux")]
    {
        "Linux"
    }
    #[cfg(windows)]
    {
        "Windows"
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux", windows)))]
    {
        "an unknown OS"
    }
}

/// 终端类型：只取 `TERM`，过滤到安全字符集并截断；window/tmux 由独立字段表达。
fn terminal_kind() -> Option<String> {
    terminal_kind_with(std::env::var_os("TERM").map(|raw| raw.to_string_lossy().into_owned()))
}

/// [`terminal_kind`] 的纯函数形式：输入原始值，输出白名单化后的值。
fn terminal_kind_with(raw: Option<String>) -> Option<String> {
    let text = raw?;
    let filtered: String = text
        .chars()
        .filter(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.' | '/'))
        .take(MAX_TERM_CHARS)
        .collect();
    (!filtered.is_empty()).then_some(filtered)
}

/// 本地日期与时间（`YYYY-MM-DD` / `HH:MM`）；平台不支持时返回 None。
pub fn local_datetime() -> Option<(String, String)> {
    #[cfg(unix)]
    {
        unix::local_datetime()
    }
    #[cfg(not(unix))]
    {
        None
    }
}

/// 时区偏移（`UTC+08:00`）；Unix 用 `localtime_r` 的当前偏移，其它平台为 None。
pub fn timezone_name() -> Option<String> {
    #[cfg(unix)]
    {
        unix::timezone_name()
    }
    #[cfg(not(unix))]
    {
        None
    }
}

/// 给定 UTC 毫秒时间戳的本地 `HH:MM`（渲染层不得自取时间：状态层算好再传）。
///
/// 平台不支持本地时间时退回 UTC；`ms` 越界时同样退回 UTC。
pub fn local_time_label(ms: i64) -> String {
    #[cfg(unix)]
    {
        if let Some(label) = unix::local_time_label(ms) {
            return label;
        }
    }
    let seconds_of_day = ms.div_euclid(1_000).rem_euclid(86_400);
    format!(
        "{:02}:{:02}",
        seconds_of_day / 3600,
        (seconds_of_day % 3600) / 60
    )
}

/// UTC 兜底日期与时间（平台不支持本地时间时使用，绝不编造时区）。
fn utc_datetime() -> (String, String) {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs() as i64)
        .unwrap_or(0);
    let days = secs.div_euclid(86_400);
    let (year, month, day) = civil_from_days(days);
    let seconds_of_day = secs.rem_euclid(86_400);
    (
        format!("{year:04}-{month:02}-{day:02}"),
        format!(
            "{:02}:{:02}",
            seconds_of_day / 3600,
            (seconds_of_day % 3600) / 60
        ),
    )
}

/// 自 1970-01-01 起的天数 → 公历年月日（Howard Hinnant 的 civil_from_days）。
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let day_of_era = z.rem_euclid(146_097);
    let year_of_era =
        (day_of_era - day_of_era / 1460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let year = year_of_era + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let mp = (5 * day_of_year + 2) / 153;
    let day = u32::try_from(day_of_year - (153 * mp + 2) / 5 + 1).unwrap_or(1);
    let month = u32::try_from(if mp < 10 { mp + 3 } else { mp - 9 }).unwrap_or(1);
    (if month <= 2 { year + 1 } else { year }, month, day)
}

/// 网络是否可用：有界 TCP 探测 + TTL 缓存；无法判定返回 None。
/// 网络探测缓存条目：测量时刻 + 结论。
type NetworkCache = Option<(Instant, Option<bool>)>;

pub fn network_available() -> Option<bool> {
    static CACHE: OnceLock<Mutex<NetworkCache>> = OnceLock::new();
    let cache = CACHE.get_or_init(|| Mutex::new(None));
    if let Ok(guard) = cache.lock()
        && let Some((at, value)) = *guard
        && at.elapsed() < NETWORK_CACHE_TTL
    {
        return value;
    }
    let measured = probe_network();
    if let Ok(mut guard) = cache.lock() {
        *guard = Some((Instant::now(), measured));
    }
    measured
}

/// 一次有界 TCP 探测；解析失败或超时返回 Some(false)（确定性失败）。
///
/// 只区分“连得上 / 连不上”：这里不区分 DNS 故障与无网络，也不读取任何载荷。
fn probe_network() -> Option<bool> {
    let Ok(addresses) = NETWORK_PROBE_HOST.to_socket_addrs() else {
        return Some(false);
    };
    for address in addresses.take(2) {
        if TcpStream::connect_timeout(&address, NETWORK_PROBE_TIMEOUT).is_ok() {
            return Some(true);
        }
    }
    Some(false)
}

/// Unix 本地时间与时区（`localtime_r`）。
#[cfg(unix)]
mod unix {
    use std::time::{SystemTime, UNIX_EPOCH};

    /// 本地日期与时间；`localtime_r` 失败时返回 None（由调用方退回 UTC）。
    pub(super) fn local_datetime() -> Option<(String, String)> {
        let tm = local_tm()?;
        Some((
            format!(
                "{:04}-{:02}-{:02}",
                tm.tm_year + 1900,
                tm.tm_mon + 1,
                tm.tm_mday
            ),
            format!("{:02}:{:02}", tm.tm_hour, tm.tm_min),
        ))
    }

    /// 时区偏移；`tm_gmtoff` 以秒为单位（东为正）。
    pub(super) fn timezone_name() -> Option<String> {
        let tm = local_tm()?;
        let offset = tm.tm_gmtoff;
        let sign = if offset < 0 { '-' } else { '+' };
        let absolute = offset.unsigned_abs();
        Some(format!(
            "UTC{sign}{:02}:{:02}",
            absolute / 3600,
            (absolute % 3600) / 60
        ))
    }

    /// 给定时间戳的本地 `HH:MM`。
    pub(super) fn local_time_label(ms: i64) -> Option<String> {
        let tm = tm_at(ms.div_euclid(1_000))?;
        Some(format!("{:02}:{:02}", tm.tm_hour, tm.tm_min))
    }

    /// 当前时刻的 `struct tm` 副本。
    fn local_tm() -> Option<libc::tm> {
        let secs = SystemTime::now().duration_since(UNIX_EPOCH).ok()?.as_secs() as i64;
        tm_at(secs)
    }

    /// 指定 Unix 秒的本地 `struct tm` 副本。
    fn tm_at(secs: i64) -> Option<libc::tm> {
        let secs = libc::time_t::try_from(secs).ok()?;
        // SAFETY: `tm` 是零初始化的目标结构体；`localtime_r` 只写它并返回同一指针。
        let mut tm: libc::tm = unsafe { std::mem::zeroed() };
        // SAFETY: `secs` 指向本函数栈上的有效整数，`tm` 是可写目标。
        let result = unsafe { libc::localtime_r(&secs, &mut tm) };
        (!result.is_null()).then_some(tm)
    }
}

#[cfg(test)]
mod tests;
