//! 文件工具公共类型：执行上下文、结构化结果、变更记录与稳定错误码。
//!
//! 这些类型是工具执行层对外的唯一契约：结果只描述**发生了什么**（路径、行号、
//! 增删计数、截断标志），不含任何 TUI/UI 概念；转成模型文本与 UI 摘要分别由
//! [`super::format_for_model`] 与 [`super::to_summary`] 完成。

use std::path::PathBuf;

/// 稳定工具名清单（错误消息、系统提示与测试共用一份，不各写一遍）。
pub const TOOL_NAMES: [&str; 7] = [
    "read",
    "search",
    "edit",
    "write",
    "exec_command",
    "write_stdin",
    "update_plan",
];

/// 工具执行上下文：相对路径的基准目录是**发送时冻结**的工作区 cwd。
///
/// 执行层从不读取进程 cwd，也不因后续 `cd`/切换窗格改变基准；绝对路径原样使用。
///
/// `reads` 是本会话的读版本登记表（读后版本保护）。生产入口必须注入；没有登记表时
/// 写工具一律拒绝修改已有文件（宁可要求重读，也不放行无版本写入）。
#[derive(Debug, Clone, Default)]
pub struct ToolContext {
    /// 工作区根（冻结快照）。
    pub cwd: PathBuf,
    /// 会话级读版本登记表。
    pub reads: Option<std::sync::Arc<super::read_version::ReadRegistry>>,
}

impl PartialEq for ToolContext {
    /// 只比较路径基准：登记表是共享句柄，不是值语义的一部分。
    fn eq(&self, other: &Self) -> bool {
        self.cwd == other.cwd
    }
}

impl Eq for ToolContext {}

impl ToolContext {
    /// 以工作区根构造上下文（不带读版本登记表）。
    pub fn new(cwd: impl Into<PathBuf>) -> Self {
        Self {
            cwd: cwd.into(),
            reads: None,
        }
    }

    /// 注入会话级读版本登记表（生产执行器必须调用）。
    pub fn with_reads(mut self, reads: std::sync::Arc<super::read_version::ReadRegistry>) -> Self {
        self.reads = Some(reads);
        self
    }
}

/// `read` 的结果种类：普通文件内容或目录列表（目录内容为每行一个条目）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReadKind {
    /// 文本文件内容。
    File,
    /// 目录条目列表（`[dir]` / `[file]` 前缀）。
    Directory,
}

/// 工具错误码：稳定英文标识，用于回传模型、落库与追踪，不是用户可见文案。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolErrorCode {
    /// 参数缺失、类型错误或包含未定义字段。
    InvalidArguments,
    /// 目标路径不存在。
    NotFound,
    /// 目标存在但不是普通文件（如目录、设备）。
    NotAFile,
    /// 目标存在但不是目录。
    NotADirectory,
    /// 内容不是合法 UTF-8。
    NotUtf8,
    /// 内容被判定为二进制（含 NUL 字节）。
    Binary,
    /// `offset` 超出文件行数。
    OffsetOutOfRange,
    /// 搜索模式或 include glob 非法。
    PatternInvalid,
    /// 搜索超时。
    Timeout,
    /// OS 权限拒绝（读写或创建目录）。
    PermissionDenied,
    /// 其余 IO 错误。
    Io,
    /// 补丁文本格式非法（标记缺失、顺序错误、路径重复等）。
    PatchInvalid,
    /// 补丁上下文未在当前文件中唯一匹配。
    PatchContextMismatch,
    /// 应用补丁后没有任何实际变更。
    NoChanges,
    /// 上下文 cwd 缺失或不可用，无法解析相对路径。
    PathUnresolvable,
    /// 输入超出规定上限（参数体或结果体）。
    TooLarge,
    /// 未知工具名。
    UnknownTool,
    /// 进程会话 ID 不存在（或不属于本聊天运行实例）。
    ProcessNotFound,
    /// 进程记录已过期（超过保留时间或条数被回收）。
    ProcessExpired,
    /// 运行中进程数达到上限（不自动杀已有服务）。
    ResourceLimit,
    /// 进程启动或运行失败。
    ProcessFailed,
    /// stdin 写入失败（半写 / BrokenPipe / 已退出）。
    StdinFailed,
    /// 计划校验失败（超限、非法状态、多个进行中）。
    PlanInvalid,
    /// 读后版本保护拒绝：没读过、读取后文件变化、或新文件目标已存在。
    StaleRead,
}

impl ToolErrorCode {
    /// 稳定字符串值（日志、DB 与模型回传共用）。
    pub fn as_str(self) -> &'static str {
        match self {
            Self::InvalidArguments => "invalid_arguments",
            Self::NotFound => "not_found",
            Self::NotAFile => "not_a_file",
            Self::NotADirectory => "not_a_directory",
            Self::NotUtf8 => "not_utf8",
            Self::Binary => "binary",
            Self::OffsetOutOfRange => "offset_out_of_range",
            Self::PatternInvalid => "pattern_invalid",
            Self::Timeout => "timeout",
            Self::PermissionDenied => "permission_denied",
            Self::Io => "io",
            Self::PatchInvalid => "patch_invalid",
            Self::PatchContextMismatch => "patch_context_mismatch",
            Self::NoChanges => "no_changes",
            Self::PathUnresolvable => "path_unresolvable",
            Self::TooLarge => "too_large",
            Self::UnknownTool => "unknown_tool",
            Self::ProcessNotFound => "process_not_found",
            Self::ProcessExpired => "process_expired",
            Self::ResourceLimit => "resource_limit",
            Self::ProcessFailed => "process_failed",
            Self::StdinFailed => "stdin_failed",
            Self::PlanInvalid => "plan_invalid",
            Self::StaleRead => "stale_read",
        }
    }
}

impl std::fmt::Display for ToolErrorCode {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// 一次搜索命中：文件、1-based 行号与该行文本（超长按列宽截断）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SearchMatch {
    /// 命中文件的绝对路径。
    pub path: PathBuf,
    /// 1-based 行号。
    pub line: usize,
    /// 命中行文本（行尾换行已剥离；超长已截断）。
    pub text: String,
    /// 该行是否因超长被截断（显式标志，不静默省略）。
    pub line_truncated: bool,
}

/// 文件变更种类。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FileChangeKind {
    /// 新建（覆盖既有同名文件时也记为该文件被写入）。
    Added,
    /// 内容修改。
    Updated,
    /// 删除。
    Deleted,
    /// 移动/重命名到新路径。
    Moved {
        /// 目标绝对路径。
        to: PathBuf,
    },
}

/// 一个文件的变更摘要（增删行数按补丁行/行 diff 统计）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileChange {
    /// 受影响的绝对路径（`Moved` 时为源路径）。
    pub path: PathBuf,
    /// 新增行数。
    pub added: usize,
    /// 删除行数。
    pub removed: usize,
    pub kind: FileChangeKind,
    /// 执行后实际内容差异；旧路径/测试可省略（UI 退回两行摘要）。
    pub diff: Option<super::diff::FileDiff>,
}

/// 工具执行的结构化结局。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ToolExecutionResult {
    /// 读取成功：`content` 已带 `{n}: {text}` 行号或目录条目前缀。
    Read {
        /// 目标绝对路径。
        path: PathBuf,
        /// 文件内容或目录列表。
        kind: ReadKind,
        content: String,
        /// 首个返回行的 1-based 行号（空结果或目录列表为 0）。
        start_line: usize,
        /// 末个返回行的 1-based 行号。
        end_line: usize,
        /// 还有后续内容时的下一个 1-based 起始行。
        next_offset: Option<usize>,
        /// 是否因上限/超时而截断（显式告知模型）。
        truncated: bool,
        /// 文件总行数（目录列表为条目数）。
        total_lines: usize,
        /// 读后版本 token（仅普通文件；目录为 None）。写工具要求携带它。
        version: Option<String>,
    },
    /// 搜索成功。
    Search {
        matches: Vec<SearchMatch>,
        truncated: bool,
        files_scanned: usize,
    },
    /// 补丁执行成功。
    Edit {
        changes: Vec<FileChange>,
        /// 是否有 hunk 未产生实际变更（例如内容原本相同）。
        partial: bool,
    },
    /// 写入成功。
    Write {
        path: PathBuf,
        created: bool,
        added: usize,
        removed: usize,
        /// 实际内容差异（新建/覆盖前后快照生成）。
        diff: Option<super::diff::FileDiff>,
    },
    /// 命令执行 / 进程交互成功送达（进程可能仍在运行）。
    ///
    /// 装箱：进程结果远大于文件类结果（命令、cwd、输出正文、chunk/时间戳），
    /// 不装箱会把整个 `ToolExecutionResult` 撑成进程结果的尺寸。
    Process(Box<super::super::process::ProcessResult>),
    /// 计划更新成功；携带完整规范化快照（已分配修订号）。
    Plan(super::super::plan::PlanSnapshot),
    /// 失败；`changes` 保留多文件补丁中**已经提交**的变更，绝不谎报整体回滚。
    Failed {
        code: ToolErrorCode,
        message: String,
        changes: Vec<FileChange>,
    },
}

impl ToolExecutionResult {
    /// 是否为**工具级**失败结果。
    ///
    /// **命令以非零码退出不算**：那是命令跑完并给出了结果（`grep` 无匹配 = 1、
    /// `diff` 有差异 = 1、`test` 条件不成立 = 1），不是工具坏了。把它算成失败会让
    /// 界面把成功的查询标成红叉、并让模型误以为工具不可用而反复重试。
    ///
    /// 真正的进程级失败（超时 / 中断 / 启动失败）仍算失败。
    /// 非零退出由结果里的 `exit_code` 自己表达，模型与界面都看得到。
    pub fn is_failure(&self) -> bool {
        match self {
            Self::Failed { .. } => true,
            Self::Process(result) => result.is_failure(),
            _ => false,
        }
    }

    /// 失败错误码（成功为 None）。
    pub fn error_code(&self) -> Option<ToolErrorCode> {
        match self {
            Self::Failed { code, .. } => Some(*code),
            Self::Process(result) if result.is_failure() => Some(match result.state {
                super::super::process::ProcessState::TimedOut => ToolErrorCode::Timeout,
                super::super::process::ProcessState::Interrupted => ToolErrorCode::ProcessFailed,
                super::super::process::ProcessState::Failed { .. } => ToolErrorCode::ProcessFailed,
                super::super::process::ProcessState::Exited { .. } => ToolErrorCode::ProcessFailed,
                _ => ToolErrorCode::ProcessFailed,
            }),
            _ => None,
        }
    }
}

/// 一次工具执行内部的失败原因（`code` + 已脱敏的单行说明）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolFailure {
    pub code: ToolErrorCode,
    pub message: String,
}

impl ToolFailure {
    /// 构造失败原因。
    pub fn new(code: ToolErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }

    /// 参数非法（消息只描述字段名与期望，不含参数全文）。
    pub fn invalid(message: impl Into<String>) -> Self {
        Self::new(ToolErrorCode::InvalidArguments, message)
    }

    /// 转为对外失败结果（`changes` 由调用方补上已提交部分）。
    pub fn into_result(self, changes: Vec<FileChange>) -> ToolExecutionResult {
        ToolExecutionResult::Failed {
            code: self.code,
            message: self.message,
            changes,
        }
    }
}

/// 执行层内部结果别名。
pub(crate) type ToolResult<T> = Result<T, ToolFailure>;

/// 公开的“校验失败”结果别名（进程/计划参数解析对外返回它）。
pub type ToolResultPublic<T> = Result<T, ToolFailure>;

/// 工具规格：稳定英文名、英文描述与 JSON Schema 生成函数（用函数指针保持 `'static`）。
#[derive(Debug, Clone, Copy)]
pub struct ToolSpec {
    /// 稳定工具名（`read` / `search` / `edit` / `write` / `exec_command` / `write_stdin` /
    /// `update_plan`）。
    pub name: &'static str,
    /// 英文描述（同时作为模型可见文档）。
    pub description: &'static str,
    /// 参数 JSON Schema 生成函数；每次调用返回独立值，调用方可自由修改。
    parameters: fn() -> serde_json::Value,
}

impl ToolSpec {
    /// 构造规格（仅本模块内使用，保证 `available_tools` 的静态性）。
    pub(crate) const fn new(
        name: &'static str,
        description: &'static str,
        parameters: fn() -> serde_json::Value,
    ) -> Self {
        Self {
            name,
            description,
            parameters,
        }
    }

    /// 参数 JSON Schema（`type: object`、`additionalProperties: false`）。
    pub fn parameters(&self) -> serde_json::Value {
        (self.parameters)()
    }
}

/// UI 摘要：只含路径/行范围/计数，**不含**文件内容。
///
/// 与 `app` 层 `ToolSummary` 字段一致；放在 `chat` 侧是为了保持
/// `chat -> app` 的单向依赖（`chat` 不依赖 `app`/`ui`），由事件层做一次字段映射。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolSummary {
    /// 目标路径（绝对路径，超长由 UI 按字符截断）。
    pub path: Option<String>,
    /// 1-based 起止行（`read` 的行范围）；目录读取与空文件为 None。
    pub range: Option<(usize, usize)>,
    pub added: Option<usize>,
    pub removed: Option<usize>,
    /// 命中条目数（`search` 命中数 / 目录读取的条目数）。
    pub matches: Option<usize>,
    /// `search` 的搜索模式：结果体不带它，由调用参数提取后带入摘要。
    pub pattern: Option<String>,
    /// `read` **实际读到的行数**（`end_line - start_line + 1`），UI 的 `(N lines)` 只用它。
    pub read_lines: Option<usize>,
    /// 文件总行数（目录读取时为条目数）；**不是**本次读到的行数。
    pub total_lines: Option<usize>,
    pub truncated: bool,
    /// `edit` 是否有 hunk 应用后内容与原文相同（无实际变更）；UI 在详情行尾显示
    /// `· partial`。其它工具恒为 false。
    pub partial: bool,
    /// 增删计数是否已知（旧内容不可解码时为 false；此时 added/removed 是兼容占位）。
    pub counts_known: bool,
    /// 计数是否来自超限输入的线性简化脚本（UI 显示 `Diff simplified`）。
    pub simplified: bool,
    /// 因元数据超限而未保存摘要的文件数（0 表示全部保留）。
    pub omitted_files: usize,
}

impl Default for ToolSummary {
    /// `counts_known` 默认为 true：旧结果与无文件计数的工具都按已知处理。
    fn default() -> Self {
        Self {
            path: None,
            range: None,
            added: None,
            removed: None,
            matches: None,
            pattern: None,
            read_lines: None,
            total_lines: None,
            truncated: false,
            partial: false,
            counts_known: true,
            simplified: false,
            omitted_files: 0,
        }
    }
}
