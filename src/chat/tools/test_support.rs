//! 测试共用工具：临时目录 fixture、文件读写与结果断言辅助。
//!
//! 项目没有 `tempfile` 依赖，这里用 `std::env::temp_dir()` + 进程/线程唯一名构造
//! 独立目录，并由 [`Fixture`] 的 `Drop` 负责清理（断言 panic 时也会清理）。

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use super::types::{FileChange, ToolContext, ToolExecutionResult};

/// fixture 目录序号。
static SEQUENCE: AtomicU64 = AtomicU64::new(0);

/// 一次测试的独立临时目录；drop 时递归删除。
pub(crate) struct Fixture {
    root: PathBuf,
}

impl Fixture {
    /// 新建唯一临时目录。
    pub(crate) fn new(label: &str) -> Self {
        let sequence = SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let root = std::env::temp_dir().join(format!(
            "lx-tools-{}-{label}-{sequence}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).expect("create fixture root");
        Self { root }
    }

    /// 目录根（绝对路径）。
    pub(crate) fn path(&self) -> &Path {
        &self.root
    }

    /// 构造指向该目录的上下文。
    pub(crate) fn ctx(&self) -> ToolContext {
        ToolContext::new(self.root.clone())
    }

    /// 写文件（自动创建父目录）。
    pub(crate) fn write(&self, relative: &str, content: &str) -> PathBuf {
        let path = self.root.join(relative);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).expect("create parent");
        }
        std::fs::write(&path, content).expect("write fixture file");
        path
    }

    /// 写原始字节（用于 BOM / CRLF / 二进制 fixture）。
    pub(crate) fn write_bytes(&self, relative: &str, bytes: &[u8]) -> PathBuf {
        let path = self.root.join(relative);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).expect("create parent");
        }
        std::fs::write(&path, bytes).expect("write fixture bytes");
        path
    }

    /// 读文本（断言存在时使用）。
    pub(crate) fn read(&self, relative: &str) -> String {
        std::fs::read_to_string(self.root.join(relative)).expect("read fixture file")
    }

    /// 读原始字节。
    pub(crate) fn read_bytes(&self, relative: &str) -> Vec<u8> {
        std::fs::read(self.root.join(relative)).expect("read fixture bytes")
    }

    /// 文件是否存在。
    pub(crate) fn exists(&self, relative: &str) -> bool {
        self.root.join(relative).exists()
    }

    /// 绝对路径。
    pub(crate) fn join(&self, relative: &str) -> PathBuf {
        self.root.join(relative)
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

/// 断言是成功结果并取出变更列表。
pub(crate) fn expect_changes(result: &ToolExecutionResult) -> &[FileChange] {
    match result {
        ToolExecutionResult::Edit { changes, .. } => changes,
        other => panic!("expected edit success, got {other:?}"),
    }
}

/// 断言是失败结果并返回错误码。
pub(crate) fn expect_failure(result: &ToolExecutionResult) -> super::ToolErrorCode {
    match result {
        ToolExecutionResult::Failed { code, .. } => *code,
        other => panic!("expected failure, got {other:?}"),
    }
}

/// 断言是失败结果并返回（错误码, 已提交变更数）。
pub(crate) fn expect_partial_failure(
    result: &ToolExecutionResult,
) -> (super::ToolErrorCode, usize) {
    match result {
        ToolExecutionResult::Failed { code, changes, .. } => (*code, changes.len()),
        other => panic!("expected failure, got {other:?}"),
    }
}

/// 取出 `read` 的正文与元数据。
pub(crate) fn expect_read(
    result: &ToolExecutionResult,
) -> (&str, usize, usize, Option<usize>, bool, usize) {
    match result {
        ToolExecutionResult::Read {
            content,
            start_line,
            end_line,
            next_offset,
            truncated,
            total_lines,
            ..
        } => (
            content.as_str(),
            *start_line,
            *end_line,
            *next_offset,
            *truncated,
            *total_lines,
        ),
        other => panic!("expected read result, got {other:?}"),
    }
}

/// 取出 `search` 的命中与截断标志。
pub(crate) fn expect_search(result: &ToolExecutionResult) -> (&[super::SearchMatch], bool, usize) {
    match result {
        ToolExecutionResult::Search {
            matches,
            truncated,
            files_scanned,
        } => (matches.as_slice(), *truncated, *files_scanned),
        other => panic!("expected search result, got {other:?}"),
    }
}

/// 取出 `write` 的返回。
pub(crate) fn expect_write(result: &ToolExecutionResult) -> (bool, usize, usize) {
    match result {
        ToolExecutionResult::Write {
            created,
            added,
            removed,
            ..
        } => (*created, *added, *removed),
        other => panic!("expected write result, got {other:?}"),
    }
}
