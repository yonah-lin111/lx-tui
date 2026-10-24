//! 受控文件 mutation：原始镜像、同步 recorder 契约与锁内捕获原语。
//!
//! 工具只依赖本模块的 trait，不接触 DB / event / Git。生产执行器持有的 recorder 经
//! channel 把 Prepare/Commit 送到事件层，事件层完成持久化后回执；worker 在文件锁内
//! **同步等待**回执，因此「保存 before + planned-after」先于任何写入。
//!
//! 没有 recorder 时（单元测试/只读调用）不产生快照记录；生产入口由执行器保证注入，
//! 不存在静默绕过的分支。

use std::path::{Path, PathBuf};

use crate::platform::{RawImage, RawKind, read_raw};

use super::types::{ToolErrorCode, ToolFailure};

/// 一次路径变更的操作类型（DB 稳定字符串）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MutationOperation {
    Create,
    Update,
    Delete,
    MoveSource,
    MoveDestination,
}

impl MutationOperation {
    /// DB 稳定值。
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Create => "create",
            Self::Update => "update",
            Self::Delete => "delete",
            Self::MoveSource => "move_src",
            Self::MoveDestination => "move_dst",
        }
    }
}

/// 一个路径的 before / planned-after。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PathMutation {
    pub operation: MutationOperation,
    /// 工具/模型给出的原始路径（审计）。
    pub path: PathBuf,
    /// 实际写入对象：写入时是物理目标；删除/移动时是目录项本身。
    pub target: PathBuf,
    pub before: RawImage,
    pub planned_after: RawImage,
    /// 读后版本保护的期望 token（修改已有文件时必填；新建时为空）。
    ///
    /// 持久层会在同一事务内用 `before` 重算 token 并核对，不一致即整批零写入。
    pub expected_version: Option<String>,
}

/// 一次工具调用产生的全部路径变更。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreparedMutation {
    pub paths: Vec<PathMutation>,
}

impl PreparedMutation {
    /// 快照字节合计（限额由持久层再次校验）。
    pub fn snapshot_bytes(&self) -> u64 {
        self.paths
            .iter()
            .map(|path| path.before.size() + path.planned_after.size())
            .sum()
    }
}

/// 持久化完成后发放的票据；只有持票才能提交实际结果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MutationTicket {
    pub ids: Vec<i64>,
}

/// 单个路径观察到的实际结果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ObservedPath {
    pub actual_after: RawImage,
    pub success: bool,
}

/// 一次调用的观察结果（与 prepare 的路径顺序一致）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ObservedMutation {
    pub paths: Vec<ObservedPath>,
}

/// recorder 失败：稳定码 + 给模型的一行说明。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MutationError {
    pub code: &'static str,
    pub message: String,
}

impl MutationError {
    pub fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
}

/// 同步 recorder：工具在文件锁内调用；实现方负责持久化并等待回执。
pub trait MutationRecorder: Send + Sync {
    fn prepare(&self, mutation: &PreparedMutation) -> Result<MutationTicket, MutationError>;
    fn commit(
        &self,
        ticket: MutationTicket,
        observed: &ObservedMutation,
    ) -> Result<(), MutationError>;
}

/// 为每次工具调用构造 recorder 的工厂。
///
/// 一轮只构造一次（绑定 session / user message），每次调用按 `call_id` 绑定所有权；
/// 不在 recorder 内部用“最后一个工具”之类的猜测。
pub trait MutationRecorderFactory: Send + Sync {
    fn for_call(&self, call_id: &str) -> std::sync::Arc<dyn MutationRecorder>;
}

/// 在持有全部目标锁的前提下记录 before 并执行 `body`，随后观察实际结果。
///
/// `paths` 是调用方已经取好的锁；本函数只做快照/回执，不自己加锁。
pub fn capture(
    recorder: Option<&dyn MutationRecorder>,
    paths: Vec<PathMutation>,
    body: impl FnOnce() -> std::io::Result<()>,
) -> Result<(), ToolFailure> {
    let Some(recorder) = recorder else {
        return body().map_err(io_failure);
    };
    let mutation = PreparedMutation { paths };
    let ticket = recorder.prepare(&mutation).map_err(map_mutation_error)?;
    let result = body();
    // 无论写入是否报告失败，都以真实磁盘状态提交：部分写入同样要可恢复。
    let mut observed = Vec::with_capacity(mutation.paths.len());
    for path in &mutation.paths {
        match read_raw(&path.target, false) {
            Ok(actual) => observed.push(ObservedPath {
                actual_after: actual,
                success: result.is_ok(),
            }),
            Err(error) => {
                // 观察失败不能写假结果：提交调用本身失败，调用方据此止损。
                return Err(ToolFailure::new(
                    ToolErrorCode::Io,
                    format!(
                        "failed to observe {} after mutation: {error}",
                        path.target.display()
                    ),
                ));
            }
        }
    }
    recorder
        .commit(ticket, &ObservedMutation { paths: observed })
        .map_err(map_mutation_error)?;
    result.map_err(io_failure)
}

/// IO 错误 → 工具错误码（保留权限/不存在语义，其余归为 Io）。
fn io_failure(error: std::io::Error) -> ToolFailure {
    let code = match error.kind() {
        std::io::ErrorKind::PermissionDenied => ToolErrorCode::PermissionDenied,
        std::io::ErrorKind::NotFound => ToolErrorCode::NotFound,
        _ => ToolErrorCode::Io,
    };
    ToolFailure::new(code, format!("file mutation failed: {error}"))
}

/// recorder 错误 → 工具错误（超限用 `TooLarge`，版本拒绝用 `StaleRead`，其余按 IO 能力拒绝）。
fn map_mutation_error(error: MutationError) -> ToolFailure {
    let code = match error.code {
        "limit" => ToolErrorCode::TooLarge,
        // 持久层版本核对拒绝：与执行层同一错误码，模型读到的是“重新读取后重试”。
        "stale_read" => ToolErrorCode::StaleRead,
        _ => ToolErrorCode::Io,
    };
    ToolFailure::new(code, error.message)
}

/// 读取写入目标（跟随链接）的 before 镜像。
pub fn read_write_target(path: &Path) -> std::io::Result<RawImage> {
    read_raw(path, true)
}

/// 读取目录项（不跟随链接）的 before 镜像。
pub fn read_entry(path: &Path) -> std::io::Result<RawImage> {
    read_raw(path, false)
}

/// 拒绝原地写入多硬链接文件：撤销只能恢复一个目录项，无法证明不影响其他链接。
pub fn ensure_single_link(path: &Path) -> Result<(), ToolFailure> {
    if crate::platform::hard_link_count(path).is_some_and(|count| count > 1) {
        return Err(ToolFailure::new(
            ToolErrorCode::Io,
            format!(
                "{} has multiple hard links; refusing to snapshot it for rewind",
                path.display()
            ),
        ));
    }
    Ok(())
}

/// 取得跨进程路径锁（与恢复共用同一套 sidecar）；超时按冲突拒绝。
pub fn lock_external(paths: &[&Path]) -> Result<Option<crate::platform::PathGuards>, ToolFailure> {
    let owned: Vec<PathBuf> = paths.iter().map(|path| path.to_path_buf()).collect();
    match crate::platform::lock_paths(&owned) {
        Ok(Some(guard)) => Ok(Some(guard)),
        Ok(None) => Err(ToolFailure::new(
            ToolErrorCode::Io,
            "timed out waiting for a path lock; another process is writing the target",
        )),
        Err(error) => Err(ToolFailure::new(
            ToolErrorCode::Io,
            format!("failed to acquire path lock: {error}"),
        )),
    }
}

/// 计划写入后的镜像：普通文件字节 + 保留原权限（新文件 0644）。
pub fn planned_regular(before: &RawImage, bytes: Vec<u8>) -> RawImage {
    let mode = match before.kind {
        RawKind::Regular => before.mode,
        _ => Some(0o644),
    };
    RawImage::regular(bytes, mode)
}

#[cfg(test)]
mod tests;
