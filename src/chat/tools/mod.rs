//! 文件工具执行层：`read` / `search` / `edit` / `write`。
//!
//! 边界：本模块只做**文件 IO 与结构化结果**，不依赖 `app`/`ui`/`event`，不持有 TUI 状态；
//! 执行函数是同步阻塞的，调用方负责放进 `spawn_blocking`。
//!
//! 契约：
//! - 相对路径一律以 [`ToolContext::cwd`]（发送时冻结的工作区）为基准，绝不读进程 cwd；
//! - 参数按白名单字段解析，不接受未定义字段，也绝不把参数拼成 shell 命令；
//! - 结果只有一种模型可见文本来源 [`format_for_model`]，UI 摘要走 [`to_summary`]，
//!   DB 落库走 [`to_result_json`]；三者在同一处生成，避免多口径漂移；
//! - 写路径使用同目录临时文件 + rename 原子替换，并按规范化路径加短时锁避免并发写互相踩踏；
//! - 不触碰 Git 索引/分支/暂存区/提交。

mod args;
mod edit;
mod lock;
mod output;
mod patch;
mod path;
mod read;
mod schema;
mod search;
mod text;
mod types;
mod write;

#[cfg(test)]
mod test_support;
#[cfg(test)]
mod tests;

pub use output::{format_for_model, to_result_json, to_summary};
pub use schema::available_tools;
pub use types::{
    FileChange, FileChangeKind, ReadKind, SearchMatch, ToolContext, ToolErrorCode,
    ToolExecutionResult, ToolSpec, ToolSummary,
};

use types::{ToolFailure, ToolResult};

/// `read` 单次返回内容字节上限（256 KiB）：超出按行截断并给出 `next_offset`。
pub const MAX_READ_BYTES: usize = 256 * 1024;

/// `read` 单次返回行数上限（也是 `limit` 的默认值）。
pub const MAX_READ_LINES: usize = 2000;

/// `read` 单行输出字符上限：超长行截断并显式标记（避免单行撑爆上下文）。
pub const MAX_READ_LINE_CHARS: usize = 4000;

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

/// 执行一次工具调用。
///
/// 参数 JSON 先按字段白名单校验（不信任模型遵守 schema）；未知工具名返回
/// `Failed{UnknownTool}`；任何失败都不会产生文件副作用（校验先于 IO）。
/// 执行本身同步阻塞，调用方应在后台线程调用。
pub fn execute(ctx: &ToolContext, name: &str, arguments_json: &str) -> ToolExecutionResult {
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
        "edit" => edit::run(ctx, arguments_json),
        "write" => wrap(write::run(ctx, arguments_json)),
        other => {
            // 只回显截断后的名字：模型可能塞入超长内容，错误消息不得被撑爆。
            let shown: String = other.chars().take(64).collect();
            ToolFailure::new(
                ToolErrorCode::UnknownTool,
                format!("unknown tool '{shown}'; available tools: read, search, edit, write"),
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
