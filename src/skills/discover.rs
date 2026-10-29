//! skill 目录发现：四个根、优先级、隐藏目录跳过、深度/数量上限与符号链接去重。

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use super::parse::parse_dir;
use super::{MAX_DEPTH, MAX_DIRS_PER_ROOT, SkillCatalog, SkillEntry, SkillScope, SkillSource};

/// 项目级 skill 根（未信任项目不传这一项）。
#[derive(Debug, Clone, Default)]
pub struct ProjectSkillRoots {
    pub lx: Option<PathBuf>,
    pub agents: Option<PathBuf>,
}

impl ProjectSkillRoots {
    /// 项目根 → 两个项目级根；调用方负责信任门控。
    pub fn for_project(project_root: &Path, lx_root: PathBuf) -> Self {
        Self {
            lx: Some(lx_root),
            agents: Some(crate::config::skills::project_agents_skills_root(
                project_root,
            )),
        }
    }
}

/// 按优先级发现全部 skill。
///
/// 顺序：项目 lx → 项目 `.agents` → 用户 lx → 用户 `.agents`。
/// 同名只保留第一个，其余带 `shadowed_by` 保留在列表里供设置页展示。
pub fn discover(
    user_root: &Path,
    user_agents_root: &Path,
    project: Option<&ProjectSkillRoots>,
    disabled: &HashSet<PathBuf>,
) -> SkillCatalog {
    let mut roots: Vec<(PathBuf, SkillScope, SkillSource, bool)> = Vec::new();
    if let Some(project) = project {
        if let Some(root) = &project.lx {
            roots.push((root.clone(), SkillScope::Project, SkillSource::Lx, false));
        }
        if let Some(root) = &project.agents {
            roots.push((
                root.clone(),
                SkillScope::Project,
                SkillSource::Agents,
                false,
            ));
        }
    }
    roots.push((
        user_root.to_path_buf(),
        SkillScope::User,
        SkillSource::Lx,
        true,
    ));
    roots.push((
        user_agents_root.to_path_buf(),
        SkillScope::User,
        SkillSource::Agents,
        false,
    ));

    let mut catalog = SkillCatalog::default();
    let mut discovered: Vec<SkillEntry> = Vec::new();
    let mut seen_names: HashSet<String> = HashSet::new();
    let mut seen_paths: HashSet<PathBuf> = HashSet::new();
    for (root, scope, source, skip_projects) in roots {
        for entry in scan_root(&root, scope, source, skip_projects, disabled, &mut catalog) {
            if !seen_paths.insert(entry.path.clone()) {
                continue;
            }
            if seen_names.insert(entry.name.clone()) {
                discovered.push(entry);
            } else {
                let shadowed_by = discovered
                    .iter()
                    .find(|first| first.name == entry.name)
                    .map(|first| first.path.clone());
                discovered.push(SkillEntry {
                    shadowed_by,
                    ..entry
                });
            }
        }
    }
    discovered.sort_by(|left, right| {
        left.scope
            .cmp(&right.scope)
            .then(left.source.cmp(&right.source))
            .then(left.name.cmp(&right.name))
    });
    catalog.entries = discovered;
    catalog
}

fn scan_root(
    root: &Path,
    scope: SkillScope,
    source: SkillSource,
    skip_projects: bool,
    disabled: &HashSet<PathBuf>,
    catalog: &mut SkillCatalog,
) -> Vec<SkillEntry> {
    if !root.is_dir() {
        return Vec::new();
    }
    let mut found = Vec::new();
    let mut visited: HashSet<PathBuf> = HashSet::new();
    let mut stack: Vec<(PathBuf, usize)> = vec![(root.to_path_buf(), 1)];
    let mut dirs = 0usize;
    while let Some((dir, depth)) = stack.pop() {
        if depth > MAX_DEPTH || dirs > MAX_DIRS_PER_ROOT {
            continue;
        }
        dirs += 1;
        let canonical = std::fs::canonicalize(&dir).unwrap_or_else(|_| dir.clone());
        if !visited.insert(canonical) {
            continue;
        }
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        let mut children: Vec<(PathBuf, usize)> = Vec::new();
        for entry in entries.flatten() {
            let path = entry.path();
            let is_dir = entry.metadata().map(|meta| meta.is_dir()).unwrap_or(false);
            if !is_dir {
                if entry.file_name() == "SKILL.md" {
                    push_skill(&path, &dir, scope, source, disabled, catalog, &mut found);
                }
                continue;
            }
            let Some(name) = entry.file_name().to_str().map(str::to_string) else {
                continue;
            };
            if name.starts_with('.') || (skip_projects && depth == 1 && name == "projects") {
                continue;
            }
            children.push((path, depth + 1));
        }
        // 逆序压栈，保持目录名字典序遍历。
        children.sort_by(|left, right| left.0.cmp(&right.0));
        for child in children.into_iter().rev() {
            stack.push(child);
        }
    }
    if dirs > MAX_DIRS_PER_ROOT {
        tracing::warn!(
            target: "lx::skills",
            root = %root.display(),
            "skill directory limit reached"
        );
    }
    found.sort_by(|left, right| left.path.cmp(&right.path));
    found
}

/// 目录与 `SKILL.md` 的 mtime 签名（只 stat，不读正文）。
///
/// 缓存键由调用方补充禁用集合与子 agent 标记；这里只覆盖会影响发现结果的
/// 文件系统事实。目录 mtime 捕捉新增/删除/重命名，文件 mtime+长度捕捉编辑。
pub fn catalog_signature(
    user_root: &Path,
    user_agents_root: &Path,
    project: Option<&ProjectSkillRoots>,
) -> u64 {
    let mut hash = 0xcbf2_9ce4_8422_2325u64;
    let mut mix = |bytes: &[u8]| {
        for byte in bytes {
            hash ^= u64::from(*byte);
            hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
        }
    };
    let mut roots: Vec<(PathBuf, bool)> = Vec::new();
    if let Some(project) = project {
        if let Some(root) = &project.lx {
            roots.push((root.clone(), false));
        }
        if let Some(root) = &project.agents {
            roots.push((root.clone(), false));
        }
    }
    roots.push((user_root.to_path_buf(), true));
    roots.push((user_agents_root.to_path_buf(), false));
    for (root, skip_projects) in roots {
        mix(root.to_string_lossy().as_bytes());
        hash_root_signature(&root, skip_projects, &mut mix);
    }
    hash
}

fn hash_root_signature(root: &Path, skip_projects: bool, mix: &mut impl FnMut(&[u8])) {
    let mut visited: HashSet<PathBuf> = HashSet::new();
    let mut stack: Vec<(PathBuf, usize)> = vec![(root.to_path_buf(), 1)];
    let mut dirs = 0usize;
    while let Some((dir, depth)) = stack.pop() {
        if depth > MAX_DEPTH || dirs > MAX_DIRS_PER_ROOT {
            continue;
        }
        dirs += 1;
        let canonical = std::fs::canonicalize(&dir).unwrap_or_else(|_| dir.clone());
        if !visited.insert(canonical) {
            continue;
        }
        if let Ok(meta) = std::fs::metadata(&dir) {
            mix(dir.to_string_lossy().as_bytes());
            mix_modified(&meta, mix);
        }
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        let mut children: Vec<(PathBuf, usize)> = Vec::new();
        for entry in entries.flatten() {
            let path = entry.path();
            let Ok(meta) = entry.metadata() else {
                continue;
            };
            if !meta.is_dir() {
                if entry.file_name() == "SKILL.md" {
                    mix(path.to_string_lossy().as_bytes());
                    mix_modified(&meta, mix);
                    mix(&meta.len().to_le_bytes());
                }
                continue;
            }
            let Some(name) = entry.file_name().to_str().map(str::to_string) else {
                continue;
            };
            if name.starts_with('.') || (skip_projects && depth == 1 && name == "projects") {
                continue;
            }
            children.push((path, depth + 1));
        }
        children.sort_by(|left, right| left.0.cmp(&right.0));
        for child in children.into_iter().rev() {
            stack.push(child);
        }
    }
}

fn mix_modified(meta: &std::fs::Metadata, mix: &mut impl FnMut(&[u8])) {
    let modified = meta
        .modified()
        .ok()
        .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|duration| duration.as_nanos() as u64)
        .unwrap_or(0);
    mix(&modified.to_le_bytes());
}

#[cfg(test)]
mod tests;

fn push_skill(
    path: &Path,
    dir: &Path,
    scope: SkillScope,
    source: SkillSource,
    disabled: &HashSet<PathBuf>,
    catalog: &mut SkillCatalog,
    found: &mut Vec<SkillEntry>,
) {
    let dir_name = dir
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("skill");
    let Some(parsed) = parse_dir(path, dir_name, catalog) else {
        return;
    };
    let canonical = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    found.push(SkillEntry {
        name: parsed.name,
        description: parsed.description,
        path: canonical.clone(),
        scope,
        source,
        enabled: !disabled.contains(&canonical),
        shadowed_by: None,
    });
}
