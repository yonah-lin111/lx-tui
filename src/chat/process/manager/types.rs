//! 进程管理的纯数据类型与共享常量。

use std::time::Duration;

use super::super::ProcessState;

/// 资源限额。
#[derive(Debug, Clone, Copy)]
pub struct ProcessLimits {
    /// 单会话同时运行的进程上限。
    pub max_running: usize,
    /// 进程内全局运行进程上限（多窗格会话累加）。
    pub global_max_running: usize,
    /// 终态记录保留条数上限。
    pub max_finished: usize,
}

impl Default for ProcessLimits {
    fn default() -> Self {
        Self {
            max_running: 8,
            global_max_running: 32,
            max_finished: 32,
        }
    }
}

/// UI 侧进程事件（纯数据；由事件层映射成 `AppEvent`）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProcessUiEvent {
    /// 有新输出可读（限流后每进程最多一个在途通知）。
    Output {
        session_id: u64,
        origin_call_id: String,
    },
    /// 进程进入终态。
    Exited {
        session_id: u64,
        origin_call_id: String,
        state: ProcessState,
        exit_code: Option<i32>,
    },
}

/// 进程操作失败原因（稳定错误码，可回给模型）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProcessError {
    /// 稳定错误码（`process_not_found` / `process_expired` / `resource_limit` / …）。
    pub code: &'static str,
    /// 单行说明（不含命令输出）。
    pub message: String,
}

impl ProcessError {
    pub(super) fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
}

/// 一次进程的 UI 快照（只读，不推进模型游标）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProcessSnapshot {
    pub session_id: u64,
    pub command: String,
    pub cwd: String,
    pub tty: bool,
    pub state: ProcessState,
    pub exit_code: Option<i32>,
    /// 保留输出的尾部文本（已剥离控制序列）。
    pub tail: String,
    /// `tail` 在**原始字节流**中的区间 `[output_start, output_end)`。
    ///
    /// 落库时写这两列。它们不是 `tail.len()`——`tail` 是剥掉控制序列、lossy 转码
    /// 之后的文本，长度与原始偏移无关。留在库里的是真实偏移，恢复侧才能据此判断
    /// 「模型读到哪」。
    pub output_start: u64,
    pub output_end: u64,
    pub dropped_bytes: u64,
    pub started_at_ms: i64,
    pub ended_at_ms: Option<i64>,
    /// 发起该进程的 `exec_command` 工具调用 ID（卡片归属）。
    pub origin_call_id: String,
    /// shell 已退出，但它拉起的后台作业仍在进程组里存活。
    ///
    /// 真实场景：`nohup bash task.sh &` 之后 shell 立刻退出。此时 lx 的直接子进程
    /// 已是 `exited`，可用户的服务还在跑——只看 `state` 会把它从后台任务面板里
    /// 过滤掉，面板显示「no running processes」而任务其实活着。
    pub background_alive: bool,
}

/// 默认等待窗口（毫秒）。
///
/// 对齐 codex：`exec_command` 默认等 10s（`write_stdin` 另有默认，见
/// [`DEFAULT_STDIN_YIELD_MS`]）。窗口只决定**本次工具调用等多久**，到期不杀进程。
pub const DEFAULT_YIELD_MS: u64 = 10_000;
/// `write_stdin` 的默认等待窗口（毫秒）：交互写要快速返回，对齐 codex 的 250ms。
///
/// 与 `exec_command` 分开：交互是「打字 → 看反应」，等 10s 完全没有意义；
/// 而首轮命令往往要几秒才吐第一行输出，等 250ms 只会立刻返回空。
pub const DEFAULT_STDIN_YIELD_MS: u64 = 250;
/// 等待窗口上限（毫秒）。
pub const MAX_YIELD_MS: u64 = 30_000;
/// 命令总寿命上限（毫秒）：24 小时。
pub const MAX_TIMEOUT_MS: u64 = 24 * 60 * 60 * 1000;
/// 返回模型的文本近似 token 上限。
pub const MAX_OUTPUT_TOKENS: usize = 16_000;
/// 每次 stdin 输入字节上限。
pub const MAX_STDIN_BYTES: usize = 64 * 1024;
/// 温和停止后等待强制终止的时间。
pub const STOP_GRACE: Duration = Duration::from_millis(1000);
/// 终态记录保留时长。
pub const FINISHED_TTL: Duration = Duration::from_secs(600);
