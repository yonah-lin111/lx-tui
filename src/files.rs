//! 工作区文件扫描：文件提及面板的候选来源；文件系统 IO 集中在此，由事件循环在后台调用。

use std::path::Path;

use ignore::WalkBuilder;

use crate::app::markdown::MentionEntry;

/// 不参与提及的重目录；隐藏目录由 `.` 前缀规则过滤。
const IGNORED_DIRECTORIES: [&str; 6] = [
    "node_modules",
    "dist",
    "build",
    "coverage",
    ".next",
    ".nuxt",
];

/// 扫描缓存的安全上限；超大根目录下停扫，避免缓存占用过多内存。
const MAX_SCAN_ENTRIES: usize = 20_000;

/// 扫描工作区根，返回相对路径（`/` 分隔）的文件与目录候选。
///
/// 跳过隐藏目录与常见构建目录，尊重根目录下的 `.gitignore`（不要求根是 Git 仓库）；
/// 符号链接与不可读条目跳过；结果为遍历顺序，排序与截断由调用方完成。
pub fn scan(root: &Path) -> Vec<MentionEntry> {
    scan_with_limit(root, MAX_SCAN_ENTRIES)
}

/// 带条目上限的扫描；上限仅用于防御超大目录，正常仓库不会命中。
fn scan_with_limit(root: &Path, limit: usize) -> Vec<MentionEntry> {
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
            break;
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
        entries.push(MentionEntry {
            path: path.replace(std::path::MAIN_SEPARATOR, "/"),
            is_directory: file_type.is_dir(),
        });
    }
    entries
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
