//! 沙箱可写根与保护路径解析（跨平台纯逻辑）。
//!
//! 可写根顺序固定：工作区 cwd → `$TMPDIR` → `/tmp` → 配置的额外根；
//! 规范化后按首次出现去重。保护路径是每个可写根下的 `.git` / `.agents` / `.aws`。

use std::path::{Component, Path, PathBuf};

/// 解析 `workspace-write` 的可写根。
///
/// 不存在的额外根会被跳过并记录警告（配置允许先保存后创建）。
pub fn writable_roots(cwd: &Path, extra: &[PathBuf]) -> Vec<PathBuf> {
    let mut roots = Vec::new();
    push_unique(&mut roots, canonical_or_absolute(cwd));
    if let Some(tmp) = std::env::var_os("TMPDIR").filter(|value| !value.is_empty()) {
        push_unique(&mut roots, canonical_or_absolute(Path::new(&tmp)));
    }
    #[cfg(unix)]
    push_unique(&mut roots, canonical_or_absolute(Path::new("/tmp")));
    for root in extra {
        let absolute = canonical_or_absolute(root);
        if !absolute.exists() {
            tracing::warn!(
                path = %absolute.display(),
                "sandbox writable root does not exist; skipping"
            );
            continue;
        }
        push_unique(&mut roots, absolute);
    }
    roots
}

/// 每个可写根的保护子路径（只读）：`.git` 恒保护，`.agents` / `.aws` 存在时保护。
///
/// `.git` 为 worktree 指针文件时，同时保护文件本身与 `gitdir:` 指向的真实目录。
pub fn protected_paths(roots: &[PathBuf]) -> Vec<PathBuf> {
    let mut protected = Vec::new();
    for root in roots {
        let git = root.join(".git");
        push_unique(&mut protected, git.clone());
        if git.is_file()
            && let Some(target) = gitdir_target(&git)
        {
            push_unique(&mut protected, canonical_or_absolute(&target));
        }
        for name in [".agents", ".aws"] {
            let candidate = root.join(name);
            if candidate.exists() {
                push_unique(&mut protected, canonical_or_absolute(&candidate));
            }
        }
    }
    protected
}

/// `path` 是否位于 `root` 内（按路径组件边界，`/a/bc` 不属于 `/a/b`）。
pub fn path_within(path: &Path, root: &Path) -> bool {
    path == root || path.starts_with(root)
}

/// `path` 是否命中任一保护路径（含其子路径）。
pub fn is_protected(path: &Path, protected: &[PathBuf]) -> bool {
    protected.iter().any(|entry| path_within(path, entry))
}

/// 解析写目标：存在的路径直接 canonicalize；不存在时取最近存在祖先拼接剩余组件。
///
/// 剩余组件含 `..` 时返回 None（禁止逃逸），调用方按拒绝处理。
pub fn resolve_write_target(path: &Path) -> Option<PathBuf> {
    if path
        .components()
        .any(|part| matches!(part, Component::ParentDir))
    {
        return None;
    }
    if path.exists() {
        return std::fs::canonicalize(path).ok();
    }
    let mut remaining = Vec::new();
    let mut cursor = path;
    loop {
        let parent = cursor.parent()?;
        let name = cursor.file_name()?;
        remaining.push(name.to_os_string());
        if parent.exists() {
            let mut resolved = std::fs::canonicalize(parent).ok()?;
            for component in remaining.iter().rev() {
                let as_path = Path::new(component);
                if as_path
                    .components()
                    .any(|part| matches!(part, Component::ParentDir | Component::RootDir))
                {
                    return None;
                }
                resolved.push(component);
            }
            return Some(resolved);
        }
        cursor = parent;
    }
}

/// 读取 `.git` 指针文件的 `gitdir:` 目标（相对路径以工作区根为基准）。
pub fn gitdir_target(git_file: &Path) -> Option<PathBuf> {
    let text = std::fs::read_to_string(git_file).ok()?;
    let target = text.lines().find_map(|line| line.strip_prefix("gitdir:"))?;
    let target = PathBuf::from(target.trim());
    if target.is_absolute() {
        return Some(target);
    }
    git_file.parent().map(|parent| parent.join(target))
}

fn canonical_or_absolute(path: &Path) -> PathBuf {
    match std::fs::canonicalize(path) {
        Ok(path) => path,
        Err(_) => {
            if path.is_absolute() {
                path.to_path_buf()
            } else {
                std::env::current_dir()
                    .map(|cwd| cwd.join(path))
                    .unwrap_or_else(|_| path.to_path_buf())
            }
        }
    }
}

fn push_unique(list: &mut Vec<PathBuf>, value: PathBuf) {
    if !list.contains(&value) {
        list.push(value);
    }
}

#[cfg(test)]
mod tests;
