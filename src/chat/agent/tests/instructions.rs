use std::path::{Path, PathBuf};

use super::*;

struct TempDir {
    path: PathBuf,
}

impl TempDir {
    fn new(label: &str) -> Self {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "lx-agents-md-test-{label}-{}-{nanos}",
            std::process::id()
        ));
        std::fs::create_dir_all(&path).expect("mkdir tempdir");
        Self { path }
    }

    fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

#[test]
fn ensure_global_agents_md_creates_default_file_without_overwriting_existing() {
    let dir = TempDir::new("ensure-create");
    let created = ensure_global_agents_md(dir.path()).expect("ensure global");
    assert_eq!(created, dir.path().join("AGENTS.md"));
    assert!(created.is_file());
    assert_eq!(std::fs::read_to_string(&created).expect("read"), "");

    // 当目录内已有 agent.md 时，不应再新建或覆盖
    let dir2 = TempDir::new("ensure-existing");
    let custom = dir2.path().join("agent.md");
    std::fs::write(&custom, "Global rule from agent.md").expect("write custom");
    let found = ensure_global_agents_md(dir2.path()).expect("ensure existing");
    assert_eq!(found, custom);
    assert!(!dir2.path().join("AGENTS.md").exists());
}

#[test]
fn load_instructions_merges_global_and_project_hierarchy_in_order() {
    let user_root = TempDir::new("merge-global");
    std::fs::write(user_root.path().join("AGENTS.md"), "Global rule 1").expect("global");

    let workspace = TempDir::new("merge-ws");
    let root = workspace.path();
    let sub = root.join("crates/core");
    std::fs::create_dir_all(&sub).expect("mkdir sub");
    std::fs::write(root.join("AGENTS.md"), "Root project rule").expect("root doc");
    std::fs::write(sub.join("agent.md"), "Subdir agent rule").expect("sub doc");

    let loaded = load_instructions(Some(user_root.path()), Some(root), Some(&sub), true);
    assert!(!loaded.is_empty());
    assert_eq!(
        loaded.global_source,
        Some(user_root.path().join("AGENTS.md"))
    );
    assert_eq!(
        loaded.project_sources,
        vec![root.join("AGENTS.md"), sub.join("agent.md")]
    );
    assert_eq!(
        loaded.body,
        format!("Global rule 1{AGENTS_MD_SEPARATOR}Root project rule\n\nSubdir agent rule")
    );
    let rendered = loaded.render_prompt_section().expect("rendered");
    assert!(rendered.starts_with(&format!(
        "# AGENTS.md instructions for {}\n\n<INSTRUCTIONS>\n",
        sub.display()
    )));
    assert!(rendered.ends_with("\n</INSTRUCTIONS>"));
}

#[test]
fn candidate_priority_prefers_override_and_skips_empty_files() {
    let workspace = TempDir::new("priority");
    let root = workspace.path();
    // AGENTS.override.md 为空时应跳过，回落到 AGENTS.md；AGENTS.md 优先于 agent.md
    std::fs::write(root.join("AGENTS.override.md"), "   \n  ").expect("empty override");
    std::fs::write(root.join("AGENTS.md"), "From AGENTS.md").expect("agents.md");
    std::fs::write(root.join("agent.md"), "From agent.md").expect("agent.md");

    let loaded = load_instructions(None, Some(root), Some(root), true);
    assert_eq!(loaded.project_sources, vec![root.join("AGENTS.md")]);
    assert_eq!(loaded.body, "From AGENTS.md");

    // 非空 AGENTS.override.md 压制 AGENTS.md
    std::fs::write(root.join("AGENTS.override.md"), "From override").expect("override");
    let loaded2 = load_instructions(None, Some(root), Some(root), true);
    assert_eq!(
        loaded2.project_sources,
        vec![root.join("AGENTS.override.md")]
    );
    assert_eq!(loaded2.body, "From override");
}

#[test]
fn untrusted_workspace_loads_only_global_instructions() {
    let user_root = TempDir::new("untrusted-global");
    std::fs::write(user_root.path().join("agent.md"), "Trusted global rule").expect("global");

    let workspace = TempDir::new("untrusted-ws");
    std::fs::write(workspace.path().join("AGENTS.md"), "Untrusted repo rule").expect("repo");

    let loaded = load_instructions(
        Some(user_root.path()),
        Some(workspace.path()),
        Some(workspace.path()),
        false,
    );
    assert_eq!(
        loaded.global_source,
        Some(user_root.path().join("agent.md"))
    );
    assert!(loaded.project_sources.is_empty());
    assert_eq!(loaded.body, "Trusted global rule");
    let section = loaded.render_prompt_section().expect("section");
    assert_eq!(
        section,
        "# AGENTS.md instructions\n\n<INSTRUCTIONS>\nTrusted global rule\n</INSTRUCTIONS>"
    );
}

#[test]
fn project_doc_budget_truncates_at_32_kib_boundary() {
    let workspace = TempDir::new("budget");
    let root = workspace.path();
    let sub = root.join("nested");
    std::fs::create_dir_all(&sub).expect("mkdir");

    let oversized = "a".repeat(MAX_PROJECT_DOC_BYTES + 1024);
    std::fs::write(root.join("AGENTS.md"), &oversized).expect("root");
    std::fs::write(sub.join("AGENTS.md"), "Should not fit").expect("sub");

    let loaded = load_instructions(None, Some(root), Some(&sub), true);
    assert_eq!(loaded.project_sources, vec![root.join("AGENTS.md")]);
    assert!(
        loaded.body.ends_with("\n…[truncated]"),
        "超预算截断必须显式标记: {:?}",
        &loaded.body[loaded.body.len().saturating_sub(40)..]
    );
    assert_eq!(
        loaded.body.trim_end_matches("\n…[truncated]").len(),
        MAX_PROJECT_DOC_BYTES,
        "截断保留 32 KiB 原始前缀"
    );
    assert!(!loaded.body.contains("Should not fit"));
}

#[cfg(unix)]
#[test]
fn symlinked_cwd_still_walks_the_project_root_ancestor_chain() {
    let workspace = TempDir::new("symlink");
    let root = workspace.path().join("real");
    let sub = root.join("nested");
    std::fs::create_dir_all(&sub).expect("mkdir sub");
    std::fs::write(root.join("AGENTS.md"), "Root rule").expect("root doc");
    std::fs::write(sub.join("AGENTS.md"), "Nested rule").expect("sub doc");
    let alias = workspace.path().join("alias");
    std::os::unix::fs::symlink(&root, &alias).expect("symlink");

    // cwd 走软链别名：字面前缀比较失败，规范化后仍必须扫描到项目根。
    let loaded = load_instructions(None, Some(&root), Some(&alias.join("nested")), true);
    let canon_root = std::fs::canonicalize(&root).expect("canon root");
    assert_eq!(
        loaded.project_sources,
        vec![
            canon_root.join("AGENTS.md"),
            canon_root.join("nested/AGENTS.md")
        ]
    );
    assert_eq!(loaded.body, "Root rule\n\nNested rule");
}

#[test]
fn linked_worktree_git_boundary_stops_upward_walk() {
    let repo = TempDir::new("worktree");
    let root = repo.path();
    std::fs::write(root.join("AGENTS.md"), "Main repo rule").expect("main rule");

    let wt = root.join(".worktrees/feature-x");
    let wt_src = wt.join("src");
    std::fs::create_dir_all(&wt_src).expect("mkdir wt");
    // 模拟 git worktree 下的 .git 文件边界
    std::fs::write(wt.join(".git"), "gitdir: ../../.git/worktrees/feature-x").expect("git file");
    std::fs::write(wt.join("AGENTS.md"), "Worktree root rule").expect("wt rule");

    let loaded = load_instructions(None, Some(root), Some(&wt_src), true);
    assert_eq!(loaded.project_sources, vec![wt.join("AGENTS.md")]);
    assert_eq!(loaded.body, "Worktree root rule");
}
