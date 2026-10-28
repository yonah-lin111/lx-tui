//! 工具执行层：`read` / `search` / `edit` / `write` 与进程/计划工具的参数校验入口。
//!
//! 边界：本模块只做**文件 IO、参数校验与结构化结果**，不依赖 `app`/`ui`/`event`，
//! 不持有 TUI 状态；文件工具是同步阻塞的，调用方负责放进 `spawn_blocking`。
//!
//! 契约：
//! - 相对路径一律以 [`ToolContext::cwd`]（发送时冻结的工作区）为基准，绝不读进程 cwd；
//! - 参数按白名单字段解析，不接受未定义字段，也绝不把参数拼成 shell 命令；
//! - 结果只有一种模型可见文本来源 [`format_for_model`]，UI 摘要走 [`to_summary`]，
//!   DB 落库走 [`to_result_json`]；三者在同一处生成，避免多口径漂移；
//! - 输入与输出都有硬上限：`read` 的单文件输入 16 MiB、目录条目 50 000 条，`search` 的
//!   遍历深度/文件数/命中数/时长亦然；任一触顶都显式标记 `truncated`，不静默省略；
//! - 写路径使用同目录临时文件 + rename 原子替换，并按规范化路径加短时锁避免并发写互相踩踏；
//! - `exec_command` / `write_stdin` 需要长期存活的进程句柄，**不在**本模块执行：
//!   这里只有它们的参数规范化（[`parse_exec_args`] / [`parse_stdin_args`]）；
//! - 不触碰 Git 索引/分支/暂存区/提交。

mod args;
pub mod ask;
mod diff;
mod edit;
mod lock;
mod mutation;
mod output;
mod patch;
mod path;
mod plan_args;
mod process_args;
mod read;
mod read_version;
mod schema;
mod search;
mod subagent;
mod text;
mod types;
mod write;

#[cfg(test)]
mod test_support;
#[cfg(test)]
mod tests;

pub use ask::{
    ASK_TOOL, AnsweredQuestion, AskAnswers, AskOption, AskQuestion, answered_result,
    parse_arguments as parse_ask_arguments,
};
pub use diff::{
    DiffCounts, DiffHunk, DiffLine, DiffLineKind, DiffUnavailableReason, FileDiff, MAX_DIFF_LINES,
    diff_lines,
};
pub use mutation::{
    MutationError, MutationOperation, MutationRecorder, MutationRecorderFactory, MutationTicket,
    ObservedMutation, ObservedPath, PathMutation, PreparedMutation,
};
pub use output::{format_for_model, to_result_json, to_summary};
pub use plan_args::build_plan_result;
pub use process_args::{parse_exec_args, parse_stdin_args, process_failure};
pub use read_version::{
    ReadRegistry, ReadVersion, canonical_key as read_canonical_key, drop_registry as drop_reads,
    registry as read_registry, require_missing as require_new_target,
    require_read_version as require_version,
};
pub use schema::{available_tools, tools_for};
pub use types::{
    FileChange, FileChangeKind, ReadKind, SUBAGENT_TOOL_NAMES, SearchMatch, TOOL_NAMES,
    ToolContext, ToolErrorCode, ToolExecutionResult, ToolFailure, ToolResultPublic, ToolSpec,
    ToolSummary,
};

use types::ToolResult;

/// `update_plan` 的限额常量（与 schema 同源）。
pub use crate::chat::plan::{MAX_EXPLANATION_CHARS, MAX_PLAN_STEPS, MAX_STEP_CHARS};
/// 进程工具的参数与限额常量（与 schema 同源）。
pub use crate::chat::process::{
    DEFAULT_STDIN_YIELD_MS, DEFAULT_YIELD_MS, MAX_OUTPUT_TOKENS, MAX_STDIN_BYTES, MAX_TIMEOUT_MS,
    MAX_YIELD_MS,
};

/// `read` 单次返回内容字节上限（256 KiB）：超出按行截断并给出 `next_offset`。
pub const MAX_READ_BYTES: usize = 256 * 1024;

/// `read` 单次返回行数上限（也是 `limit` 的默认值）。
pub const MAX_READ_LINES: usize = 2000;

/// `read` 单行输出字符上限：超长行截断并显式标记（避免单行撑爆上下文）。
pub const MAX_READ_LINE_CHARS: usize = 4000;

/// `read` **输入**单文件字节上限（16 MiB）：超过直接返回 `TooLarge`，不做部分读取。
///
/// 这是**输入**上限（愿意读进内存多少字节），与 [`MAX_READ_BYTES`] 的**输出**上限是两件
/// 不同的事。codex 的对应值是 512 MiB（`exec-server/src/local_file_system_read.rs`），
/// 我们取更保守的 16 MiB：超过此体积的文本文件几乎只能靠 `search` 定位后分段读，
/// 全量读入既无意义也会挤占内存。
pub const MAX_READ_FILE_INPUT_BYTES: u64 = 16 * 1024 * 1024;

/// `read` 单次列目录的条目上限（对齐 codex 的 50 000）：
/// 到达上限即停止收集并置 `truncated`，`total_lines` 退化为「已列出条目数」。
pub const MAX_READ_DIR_ENTRIES: usize = 50_000;

/// `search` 单次返回命中条数上限（也是 `limit` 参数的默认值）。
pub const MAX_SEARCH_RESULTS: usize = 200;

/// `search` 单次遍历文件数上限：到达后返回已收集结果并置 `truncated`。
pub const MAX_SEARCH_FILES: usize = 20_000;

/// `search` 遍历总时长上限：超时返回已收集结果并置 `truncated`。
pub const SEARCH_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);

/// `search` 单个文件读取上限：更大的文件跳过，避免误读巨型二进制/日志。
pub const MAX_SEARCH_FILE_BYTES: u64 = 8 * 1024 * 1024;

/// `search` 命中行输出字符上限（单行截断并显式标记）。
pub const MAX_SEARCH_LINE_CHARS: usize = 400;

/// 单次工具结果 JSON 落库上限（复用 DB 侧 1 MiB 约定）：超限截断并置 `truncated`。
pub const MAX_TOOL_RESULT_BYTES: usize = crate::db::chat::MAX_TOOL_RESULT_BYTES;

/// 执行一次**文件**工具调用。
///
/// 参数 JSON 先按字段白名单校验（不信任模型遵守 schema）；未知工具名返回
/// `Failed{UnknownTool}`；任何失败都不会产生文件副作用（校验先于 IO）。
/// 执行本身同步阻塞，调用方应在后台线程调用。
///
/// 进程工具（`exec_command` / `write_stdin`）与 `update_plan` 不经过本函数：
/// 它们需要跨轮存活的会话句柄，由生产执行器直接分派（见
/// [`crate::chat::agent_loop::AgentTools`]）；此处的同名分支只做参数校验失败，
/// 保证“走错入口”也不会产生副作用。
pub fn execute(ctx: &ToolContext, name: &str, arguments_json: &str) -> ToolExecutionResult {
    execute_with(ctx, name, arguments_json, None)
}

/// 带 mutation recorder 的版本：生产执行器必须注入 recorder（写工具据此快照）。
pub fn execute_with(
    ctx: &ToolContext,
    name: &str,
    arguments_json: &str,
    recorder: Option<&dyn MutationRecorder>,
) -> ToolExecutionResult {
    if arguments_json.len() > crate::chat::events::MAX_TOOL_ARGUMENTS_BYTES {
        return ToolFailure::new(
            ToolErrorCode::TooLarge,
            format!(
                "arguments exceed the {} byte limit",
                crate::chat::events::MAX_TOOL_ARGUMENTS_BYTES
            ),
        )
        .into_result(Vec::new());
    }
    match name {
        "read" => wrap(read::run(ctx, arguments_json)),
        "search" => wrap(search::run(ctx, arguments_json)),
        // `edit` 自带已提交变更累积，失败时需把部分成功一并返回。
        "edit" => edit::run_with(ctx, arguments_json, recorder),
        "write" => write::run_with(ctx, arguments_json, recorder),
        // 需要长期进程句柄/会话状态：本入口不执行，返回明确的错误而不是静默成功。
        "exec_command" | "write_stdin" | "update_plan" | ASK_TOOL => ToolFailure::new(
            ToolErrorCode::UnknownTool,
            format!("tool '{name}' must be executed by the agent loop, not the file executor"),
        )
        .into_result(Vec::new()),
        other => {
            // 只回显截断后的名字：模型可能塞入超长内容，错误消息不得被撑爆。
            let shown: String = other.chars().take(64).collect();
            ToolFailure::new(
                ToolErrorCode::UnknownTool,
                format!(
                    "unknown tool '{shown}'; available tools: {}",
                    TOOL_NAMES.join(", ")
                ),
            )
            .into_result(Vec::new())
        }
    }
}

/// 把内部 `Result` 收敛成对外结果（无已提交变更）。
fn wrap(result: ToolResult<ToolExecutionResult>) -> ToolExecutionResult {
    match result {
        Ok(value) => value,
        Err(failure) => failure.into_result(Vec::new()),
    }
}
