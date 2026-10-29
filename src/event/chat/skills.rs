//! 每轮冻结 skill 目录：读取配置、后台无关的阻塞扫描、渲染系统提示段。
//!
//! 扫描是阻塞文件 IO，但只 stat 目录与读取 frontmatter；规模上限由
//! [`crate::skills::MAX_DIRS_PER_ROOT`] 约束，足够小，直接在事件循环内完成。

use std::path::Path;

use crate::config::skills as skills_config;
use crate::db::service::Persistence;
use crate::skills::{ProjectSkillRoots, discover, render_catalog};

/// 渲染本轮的 `## Skills` 段；没有可用 skill 时返回 None。
pub(crate) fn catalog_prompt(
    persistence: &Persistence,
    workspace_root: Option<&Path>,
) -> Option<String> {
    let disabled: std::collections::HashSet<std::path::PathBuf> =
        skills_config::skills_config_path()
            .ok()
            .and_then(|path| skills_config::load_skills(&path).ok().flatten())
            .map(|file| file.disabled_set().into_iter().collect())
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
    let catalog = discover(&user_root, &user_agents, project.as_ref(), &disabled);
    crate::skills::set_cached_entries(catalog.entries.clone());
    render_catalog(&catalog.entries, false)
}

/// 从用户草稿里提取 `@[refer-skill]` 正文块（当轮注入，不走 DB）。
pub(crate) fn reference_blocks(text: &str) -> Vec<String> {
    let mut blocks = Vec::new();
    for reference in crate::app::markdown::parse_references(text) {
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
