//! 每轮冻结 skill 目录：读取配置、按 mtime 失效缓存、渲染系统提示段。
//!
//! 目录扫描的上限由 [`crate::skills::MAX_DIRS_PER_ROOT`] 约束；缓存命中时每轮只做
//! 目录与 `SKILL.md` 的 `stat` 签名（不重读正文）。签名变化时后台重扫，当前轮沿用
//! 上一份目录，避免编辑 skill 时卡住事件循环。

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

use crate::config::skills as skills_config;
use crate::db::service::Persistence;
use crate::skills::{
    ProjectSkillRoots, SkillEntry, catalog_signature, discover, render_catalog_with_omitted,
};

/// 缓存键：发现根、禁用集合与子 agent 标记。
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct CacheKey {
    user_root: PathBuf,
    user_agents: PathBuf,
    project: Option<(PathBuf, PathBuf)>,
    disabled: Vec<PathBuf>,
    enabled: Vec<PathBuf>,
    subagent: bool,
}

struct CachedCatalog {
    signature: u64,
    prompt: Option<String>,
    omitted: usize,
    errors: usize,
    entries: Vec<SkillEntry>,
}

/// 后台预扫描任务：`run` 结束即写回缓存并返回首页计数。
pub(crate) struct PreloadJob {
    job: ScanJob,
    root: Option<PathBuf>,
}

impl PreloadJob {
    /// 执行同步扫描；返回 `(工作区根, skill 数, 解析错误数, 条目)`。
    pub(crate) fn run(self) -> (Option<PathBuf>, usize, usize, Vec<SkillEntry>) {
        let cached = scan(&self.job);
        let total = cached
            .entries
            .iter()
            .filter(|entry| entry.enabled && entry.shadowed_by.is_none())
            .count();
        let errors = cached.errors;
        let entries = cached.entries.clone();
        crate::skills::set_cached_entries(entries.clone());
        if let Ok(mut map) = cache().lock() {
            map.insert(self.job.key.clone(), cached);
        }
        (self.root, total, errors, entries)
    }
}

/// 为首页预扫描 skill；`workspace_root` 未信任时只扫用户级目录。
pub(crate) fn prepare_preload(
    persistence: &Persistence,
    workspace_root: Option<&Path>,
    subagent: bool,
) -> Option<PreloadJob> {
    build_job(persistence, workspace_root, subagent).map(|job| PreloadJob {
        job,
        root: workspace_root.map(Path::to_path_buf),
    })
}

struct ScanJob {
    key: CacheKey,
    user_root: PathBuf,
    user_agents: PathBuf,
    project: Option<ProjectSkillRoots>,
    disabled: std::collections::HashSet<PathBuf>,
    enabled: std::collections::HashSet<PathBuf>,
    signature: u64,
    subagent: bool,
}

fn cache() -> &'static Mutex<HashMap<CacheKey, CachedCatalog>> {
    static CACHE: OnceLock<Mutex<HashMap<CacheKey, CachedCatalog>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Skills 设置变化后清空缓存（下一次 turn 重新扫描）。
pub(crate) fn invalidate_skill_cache() {
    if let Ok(mut cache) = cache().lock() {
        cache.clear();
    }
}

/// 渲染本轮的 `## Skills` 段；返回 `(提示段, 预算外省略的 skill 数)`。
pub(crate) fn catalog_prompt(
    persistence: &Persistence,
    workspace_root: Option<&Path>,
    subagent: bool,
) -> (Option<String>, usize) {
    let Some(job) = build_job(persistence, workspace_root, subagent) else {
        return (None, 0);
    };
    let key = job.key.clone();
    let guard = cache().lock().ok();
    if let Some(cached) = guard.as_ref().and_then(|cached| cached.get(&key))
        && cached.signature == job.signature
    {
        crate::skills::set_cached_entries(cached.entries.clone());
        // 目录未变化：省略提示已经在那次刷新时弹过，不每轮重复。
        return (cached.prompt.clone(), 0);
    }
    let has_stale = guard
        .as_ref()
        .is_some_and(|cached| cached.contains_key(&key));
    drop(guard);
    // 已有旧快照：后台重扫，本轮继续使用旧目录（只影响编辑后的第一个 turn）。
    if has_stale {
        spawn_refresh(job);
        if let Ok(cached) = cache().lock()
            && let Some(cached) = cached.get(&key)
        {
            crate::skills::set_cached_entries(cached.entries.clone());
            return (cached.prompt.clone(), cached.omitted);
        }
        return (None, 0);
    }
    // 首次扫描：结果必须在本轮可用，直接同步执行（目录数量有硬上限）。
    let cached = scan(&job);
    let result = (cached.prompt.clone(), cached.omitted);
    crate::skills::set_cached_entries(cached.entries.clone());
    if let Ok(mut cached_map) = cache().lock() {
        cached_map.insert(key, cached);
    }
    result
}

fn build_job(
    persistence: &Persistence,
    workspace_root: Option<&Path>,
    subagent: bool,
) -> Option<ScanJob> {
    let (disabled, enabled): (Vec<PathBuf>, Vec<PathBuf>) = skills_config::skills_config_path()
        .ok()
        .and_then(|path| skills_config::load_skills(&path).ok().flatten())
        .map(|file| {
            let mut disabled: Vec<PathBuf> = file.disabled_set().into_iter().collect();
            disabled.sort();
            let mut enabled: Vec<PathBuf> = file.enabled_set().into_iter().collect();
            enabled.sort();
            (disabled, enabled)
        })
        .unwrap_or_default();
    let user_root = skills_config::user_skills_root().ok()?;
    let user_agents = skills_config::user_agents_skills_root().ok()?;
    let project = workspace_root
        .filter(|root| persistence.is_trusted(root))
        .and_then(|root| {
            let lx = skills_config::project_skills_root(root).ok()?;
            Some(ProjectSkillRoots {
                lx: Some(lx),
                agents: Some(skills_config::project_agents_skills_root(root)),
            })
        });
    let key = CacheKey {
        user_root: user_root.clone(),
        user_agents: user_agents.clone(),
        project: project.as_ref().and_then(|roots| {
            roots
                .lx
                .as_ref()
                .zip(roots.agents.as_ref())
                .map(|(lx, agents)| (lx.clone(), agents.clone()))
        }),
        disabled: disabled.clone(),
        enabled: enabled.clone(),
        subagent,
    };
    let signature = catalog_signature(&user_root, &user_agents, project.as_ref());
    Some(ScanJob {
        key,
        user_root,
        user_agents,
        project,
        disabled: disabled.into_iter().collect(),
        enabled: enabled.into_iter().collect(),
        signature,
        subagent,
    })
}

fn scan(job: &ScanJob) -> CachedCatalog {
    let catalog = discover(
        &job.user_root,
        &job.user_agents,
        job.project.as_ref(),
        &job.disabled,
        &job.enabled,
    );
    let (prompt, omitted) = render_catalog_with_omitted(&catalog.entries, job.subagent);
    CachedCatalog {
        signature: job.signature,
        prompt,
        omitted,
        errors: catalog.errors.len(),
        entries: catalog.entries,
    }
}

fn spawn_refresh(job: ScanJob) {
    let key = job.key.clone();
    let handle = tokio::runtime::Handle::try_current();
    let Ok(handle) = handle else {
        return;
    };
    handle.spawn_blocking(move || {
        let cached = scan(&job);
        if let Ok(mut cache) = cache().lock() {
            cache.insert(key, cached);
        }
    });
}

/// 从用户草稿里提取 `@[refer-skill]` 正文块（当轮注入，不走 DB）。
pub(crate) fn reference_blocks(text: &str) -> Vec<String> {
    let mut blocks = Vec::new();
    let masked = crate::app::paste::mask_pasted_text(text);
    for reference in crate::app::markdown::parse_references(&masked) {
        if reference.kind != crate::app::markdown::ReferKind::Skill {
            continue;
        }
        let path = std::path::Path::new(&reference.path);
        let name = path
            .parent()
            .and_then(std::path::Path::file_name)
            .and_then(|name| name.to_str())
            .unwrap_or("skill");
        match crate::skills::load_skill_body(path) {
            Ok(body) => {
                let (block, truncated) = crate::skills::render_injection(
                    name,
                    path,
                    &body,
                    crate::skills::MAX_SKILL_INJECTION_BYTES,
                );
                if truncated {
                    tracing::warn!(
                        target: "lx::skills",
                        skill = name,
                        "skill injection truncated"
                    );
                }
                blocks.push(block);
            }
            Err(error) => {
                tracing::warn!(target: "lx::skills", skill = name, %error, "skill load failed");
                blocks.push(format!("[skill {name} could not be loaded: {error}]"));
            }
        }
    }
    blocks
}
