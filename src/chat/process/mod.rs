//! 会话进程管理：`exec_command` / `write_stdin` 的纯数据契约。
//!
//! 边界：
//!
//! - **不依赖** `app` / `ui` / `event`：进程事件经构造时注入的回调送出纯数据；
//! - 进程归属于**聊天运行实例**（workspace + chat session + runtime epoch），不是某一次
//!   模型请求；跨轮复用同一实例，旧实例的迟到事件不复活进程；
//! - `process_session_id` 是模型见到的 `session_id`，既不是聊天会话 ID，也不是 OS PID；
//! - 真实退出码只能由 `wait` 得到：读端 EOF 不代表进程成功退出。

mod manager;
pub mod output;

pub use manager::{
    DEFAULT_STDIN_YIELD_MS, DEFAULT_YIELD_MS, MAX_OUTPUT_TOKENS, MAX_STDIN_BYTES, MAX_TIMEOUT_MS,
    MAX_YIELD_MS, ProcessError, ProcessLimits, ProcessManager, ProcessSnapshot, ProcessUiEvent,
};
pub use output::{MAX_MODEL_OUTPUT_BYTES, MAX_PROCESS_OUTPUT_BYTES, OutputBuffer, OutputSource};

use std::path::PathBuf;

/// 进程归属键：跨轮事件的稳定归属，不受单轮 `RequestKey` 的代次变化影响。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProcessOwner {
    /// 冻结的工作区根。
    pub workspace: PathBuf,
    /// 聊天会话 external id。
    pub chat_session_id: String,
    /// 运行实例代次（每次为该会话建立进程管理器时分配）。
    pub runtime_epoch: String,
}

/// 进程状态（`Starting` 只存在于 spawn 与登记之间）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProcessState {
    /// 已 spawn、尚未进入等待。
    Starting,
    /// 仍在运行。
    Running,
    /// 已退出（`code` 未知时保持 None，绝不伪造 0）。
    Exited { code: Option<i32> },
    /// 被显式停止。
    Stopped,
    /// 显式超时后清理。
    TimedOut,
    /// 异常中断（lx 崩溃恢复或清理未确认）。
    Interrupted,
    /// 启动/运行失败（`code` 是稳定错误码，不是退出码）。
    Failed { code: String },
}

impl ProcessState {
    /// 稳定状态值（结果 JSON、DB 与 UI 共用）。
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Starting => "starting",
            Self::Running => "running",
            Self::Exited { .. } => "exited",
            Self::Stopped => "stopped",
            Self::TimedOut => "timed_out",
            Self::Interrupted => "interrupted",
            Self::Failed { .. } => "failed",
        }
    }

    /// 是否终态（不再接受输出变化）。
    pub fn is_terminal(&self) -> bool {
        !matches!(self, Self::Starting | Self::Running)
    }

    /// 退出码（只有 `Exited` 有）。
    pub fn exit_code(&self) -> Option<i32> {
        match self {
            Self::Exited { code } => *code,
            _ => None,
        }
    }
}

/// 进程解析：从 DB 稳定值还原（未知值返回 None，调用方按 `interrupted` 保守处理）。
pub fn parse_process_state(value: &str) -> Option<ProcessState> {
    match value {
        "starting" => Some(ProcessState::Starting),
        "running" => Some(ProcessState::Running),
        "exited" => Some(ProcessState::Exited { code: None }),
        "stopped" => Some(ProcessState::Stopped),
        "timed_out" => Some(ProcessState::TimedOut),
        "interrupted" => Some(ProcessState::Interrupted),
        "failed" => Some(ProcessState::Failed {
            code: String::new(),
        }),
        _ => None,
    }
}

/// `exec_command` 的已校验参数。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExecArgs {
    /// 原样交给 shell 的命令（非空白）。
    pub cmd: String,
    /// 显式工作目录（相对路径仍以冻结工作区为基准）。
    pub workdir: Option<String>,
    /// 显式 shell；None 用平台默认。
    pub shell: Option<String>,
    /// 登录 shell（默认 false）。
    pub login: bool,
    /// 是否使用独立 PTY（默认 false）。
    pub tty: bool,
    /// 本次等待窗口（毫秒）。
    pub yield_time_ms: u64,
    /// 命令总寿命上限（毫秒）；None 表示不限制。
    pub timeout_ms: Option<u64>,
    /// 返回模型的文本预算（近似 token）。
    pub max_output_tokens: usize,
}

/// `write_stdin` 的已校验参数。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StdinArgs {
    /// 目标进程会话 ID。
    pub session_id: u64,
    /// 要写入的字节（默认空串 = 只读取后续输出）；不自动补换行。
    pub chars: String,
    /// lx 扩展：按进程 ID 显式停止。
    pub terminate: bool,
    /// 本次等待窗口（毫秒）。
    pub yield_time_ms: u64,
    /// 返回模型的文本预算（近似 token）。
    pub max_output_tokens: usize,
}

/// 一次进程交互的结果（`exec_command` 与 `write_stdin` 共用形状）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProcessResult {
    /// 来源工具名：`exec_command` / `write_stdin`。
    pub kind: ProcessResultKind,
    /// 模型见到的进程会话 ID。
    pub session_id: u64,
    /// 命令原文（用于卡片与模型回执）。
    pub command: String,
    /// 实际工作目录。
    pub cwd: String,
    /// 是否使用 PTY。
    pub tty: bool,
    /// 当前状态。
    pub state: ProcessState,
    /// 退出码（未知保持 None）。
    pub exit_code: Option<i32>,
    /// 本次返回的输出文本（已剥离控制序列）。
    pub output: String,
    /// 本次返回区间的起点（原始字节偏移）。
    pub output_start: u64,
    /// 本次返回区间的终点（下次读取游标）。
    pub output_end: u64,
    /// 请求起点之前被淘汰的字节数。
    pub dropped_bytes: u64,
    /// 是否还有未返回的输出。
    pub truncated: bool,
    /// 本次交互的输入预览（`write_stdin` 专用，已限长转义）。
    pub input: Option<String>,
    /// 本次调用是否真的向 stdin 写了字节（空写只是读取）。
    pub wrote_input: bool,
    /// 本次调用是否为显式停止。
    pub terminate: bool,
    /// 本次结果的分块标识（6 位十六进制；对齐 codex 的 `chunk_id`）。
    ///
    /// 同一次调用内稳定，模型可用它把「这次读到的输出」与后续轮次区分开。
    pub chunk_id: String,
    /// 本次调用在等待窗口内实际花掉的墙钟时间（毫秒）。
    ///
    /// 与 `yield_time_ms` 不同：后者是**允许等待的上限**，这是**真正等了多久**，
    /// 模型据此判断「命令是不是刚好卡在窗口边界」。JSON 里按 codex 的口径
    /// 输出成秒（`wall_time_seconds`），内部存整数毫秒以保持 `Eq`。
    pub wall_time_ms: u64,
}

/// 进程结果的来源工具。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProcessResultKind {
    ExecCommand,
    WriteStdin,
}

impl ProcessResultKind {
    /// 稳定工具名。
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ExecCommand => "exec_command",
            Self::WriteStdin => "write_stdin",
        }
    }

    /// 结果 JSON 中的动作描述（UI 卡片标题/动词使用）。
    pub fn action(self) -> &'static str {
        match self {
            Self::ExecCommand => "run",
            Self::WriteStdin => "read",
        }
    }
}

impl ProcessResult {
    /// 是否应作为工具失败回给模型（非零退出、超时、启动失败、中断）。
    pub fn is_failure(&self) -> bool {
        match &self.state {
            ProcessState::TimedOut | ProcessState::Interrupted | ProcessState::Failed { .. } => {
                true
            }
            ProcessState::Exited { code } => code.is_some_and(|code| code != 0),
            ProcessState::Starting | ProcessState::Running | ProcessState::Stopped => false,
        }
    }

    /// 失败原因（给模型与 UI 的单行说明；不含输出正文）。
    pub fn failure_message(&self) -> Option<String> {
        match &self.state {
            ProcessState::Exited { code: Some(code) } if *code != 0 => {
                Some(format!("command exited with code {code}"))
            }
            ProcessState::TimedOut => Some("command timed out".to_string()),
            ProcessState::Interrupted => Some("command interrupted".to_string()),
            ProcessState::Failed { code } => Some(format!("command failed: {code}")),
            _ => None,
        }
    }

    /// 转成结果 JSON（DB `result_json`、UI 视图解析与回放共用同一形状）。
    pub fn to_json(&self) -> serde_json::Value {
        serde_json::json!({
            "tool": self.kind.as_str(),
            "session_id": self.session_id,
            "command": self.command,
            "cwd": self.cwd,
            "tty": self.tty,
            "process_state": self.state.as_str(),
            "exit_code": self.exit_code,
            "output": self.output,
            "output_start": self.output_start,
            "output_end": self.output_end,
            "dropped_bytes": self.dropped_bytes,
            "truncated": self.truncated,
            "input": self.input,
            "wrote_input": self.wrote_input,
            "terminate": self.terminate,
            "chunk_id": self.chunk_id,
            "wall_time_seconds": self.wall_time_ms as f64 / 1_000.0,
        })
    }
}

/// 生成本次结果的分块标识（6 位十六进制，对齐 codex 的 `chunk_id`）。
///
/// 用 `std` 的简易随机源：只要求同一次调用内不重复、跨调用不撞即可，
/// 不是安全标识，不引入 `rand` 依赖。
pub fn generate_chunk_id() -> String {
    use std::hash::{BuildHasher, Hasher};
    let mut id = String::with_capacity(6);
    let mut hasher = std::collections::hash_map::RandomState::new().build_hasher();
    hasher.write_u64(
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|elapsed| elapsed.as_nanos() as u64)
            .unwrap_or(0),
    );
    let value = hasher.finish();
    for shift in 0..6 {
        let nibble = (value >> (shift * 4)) & 0xf;
        id.push(char::from_digit(nibble as u32, 16).unwrap_or('0'));
    }
    id
}

/// 输入预览：控制字符转义、按字符限长（卡片里不重复展示完整 stdin）。
pub fn preview_input(chars: &str) -> String {
    let mut out = String::with_capacity(chars.len());
    for (count, ch) in chars.chars().enumerate() {
        if count >= MAX_INPUT_PREVIEW_CHARS {
            out.push('…');
            break;
        }
        match ch {
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            ch if ch.is_control() => out.push_str(&format!("\\u{{{:x}}}", ch as u32)),
            ch => out.push(ch),
        }
    }
    out
}

/// 输入预览字符上限。
pub const MAX_INPUT_PREVIEW_CHARS: usize = 120;

#[cfg(test)]
mod tests;
