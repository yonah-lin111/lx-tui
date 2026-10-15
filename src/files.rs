//! 工作区文件扫描：文件提及面板与引用文件夹内容搜索的候选来源；
//! 文件系统 IO 集中在此，由事件循环在后台调用。

use std::path::{Path, PathBuf};

use ignore::WalkBuilder;

use crate::app::markdown::MentionEntry;
use crate::app::referenced::{MAX_SCAN_ENTRIES, ReferencedEntry, ReferencedScanOutcome};

/// 不参与扫描的重目录；隐藏目录由 `.` 前缀规则过滤。
const IGNORED_DIRECTORIES: [&str; 6] = [
    "node_modules",
    "dist",
    "build",
    "coverage",
    ".next",
    ".nuxt",
];

/// 目录扫描根不可访问（不存在、非目录或无权限）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScanRootError {
    Unavailable,
}

/// 一次遍历得到的原始条目：绝对路径、相对根的 `/` 分隔路径与类型。
#[derive(Debug, Clone, PartialEq, Eq)]
struct WalkEntry {
    absolute: PathBuf,
    relative: String,
    is_directory: bool,
}

/// 扫描工作区根，返回相对路径（`/` 分隔）的文件与目录候选。
///
/// 跳过隐藏目录与常见构建目录，尊重根目录下的 `.gitignore`（不要求根是 Git 仓库）；
/// 符号链接与不可读条目跳过；结果为遍历顺序，排序与截断由调用方完成。
pub fn scan(root: &Path) -> Vec<MentionEntry> {
    scan_with_limit(root, MAX_SCAN_ENTRIES)
}

/// 带条目上限的扫描；上限仅用于防御超大目录（测试与内部复用）。
fn scan_with_limit(root: &Path, limit: usize) -> Vec<MentionEntry> {
    let (entries, _) = walk(root, limit);
    entries
        .into_iter()
        .map(|entry| MentionEntry {
            path: entry.relative,
            is_directory: entry.is_directory,
        })
        .collect()
}

/// 扫描引用文件夹根，返回相对根路径、绝对路径与类型；根不可访问时返回错误。
///
/// 与 `scan` 共用同一遍历策略（忽略规则、噪声目录与符号链接行为一致）；
/// 命中条目上限时标记截断，绝不 panic。
pub fn scan_referenced(root: &Path) -> Result<ReferencedScanOutcome, ScanRootError> {
    let metadata = std::fs::metadata(root).map_err(|_| ScanRootError::Unavailable)?;
    if !metadata.is_dir() {
        return Err(ScanRootError::Unavailable);
    }
    let (entries, truncated) = walk(root, MAX_SCAN_ENTRIES);
    let entries = entries
        .into_iter()
        .map(|entry| {
            let name = entry
                .relative
                .rsplit('/')
                .next()
                .unwrap_or(&entry.relative)
                .to_string();
            ReferencedEntry {
                path: entry.absolute,
                relative: entry.relative,
                name,
                is_directory: entry.is_directory,
            }
        })
        .collect();
    Ok(ReferencedScanOutcome { entries, truncated })
}

/// 共享遍历：跳过根自身、隐藏目录、噪声目录与符号链接；`limit` 为条目上限。
///
/// 返回条目与是否因上限截断。局部读取失败按跳过处理，绝不向上报错。
fn walk(root: &Path, limit: usize) -> (Vec<WalkEntry>, bool) {
    let mut entries = Vec::new();
    let walker = WalkBuilder::new(root)
        .hidden(false)
        .ignore(false)
        .git_ignore(true)
        .git_global(false)
        .git_exclude(false)
        .parents(false)
        .require_git(false)
        .follow_links(false)
        .filter_entry(|entry| entry.depth() == 0 || !is_ignored_directory(entry))
        .build();
    for result in walker {
        if entries.len() >= limit {
            return (entries, true);
        }
        let Ok(entry) = result else {
            continue;
        };
        if entry.depth() == 0 {
            continue;
        }
        let Some(file_type) = entry.file_type() else {
            continue;
        };
        if file_type.is_symlink() {
            continue;
        }
        let Ok(relative) = entry.path().strip_prefix(root) else {
            continue;
        };
        let Some(path) = relative.to_str() else {
            continue;
        };
        entries.push(WalkEntry {
            absolute: entry.path().to_path_buf(),
            relative: path.replace(std::path::MAIN_SEPARATOR, "/"),
            is_directory: file_type.is_dir(),
        });
    }
    (entries, false)
}

/// 条目是否为硬跳过目录：隐藏目录（`.` 开头）或常见构建目录。
///
/// 隐藏文件（如 `.gitignore`）仍参与候选；非 UTF-8 名称的目录跳过。
fn is_ignored_directory(entry: &ignore::DirEntry) -> bool {
    let Some(file_type) = entry.file_type() else {
        return false;
    };
    if !file_type.is_dir() {
        return false;
    }
    entry
        .file_name()
        .to_str()
        .is_none_or(|name| name.starts_with('.') || IGNORED_DIRECTORIES.contains(&name))
}

#[cfg(test)]
mod tests;
