//! `chat::sandbox` 单元测试：计划构建、覆盖与文件工具视图。

use std::path::PathBuf;

use super::*;

fn temp_dir(label: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "lx-chat-sandbox-{}-{label}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or_default()
    ));
    std::fs::create_dir_all(&dir).expect("create temp dir");
    std::fs::canonicalize(&dir).expect("canonical temp dir")
}

#[test]
fn read_only_has_no_writable_roots() {
    let cwd = temp_dir("read-only");
    let settings = SandboxSettings {
        mode: SandboxMode::ReadOnly,
        ..SandboxSettings::default()
    };
    let plan = SandboxPlan::for_turn(&settings, &cwd);
    assert!(plan.writable_roots.is_empty());
    assert!(plan.protected.is_empty());
    assert!(!plan.network_access);
    let _ = std::fs::remove_dir_all(cwd);
}

#[test]
fn workspace_write_includes_cwd_and_protects_git() {
    let cwd = temp_dir("workspace");
    std::fs::create_dir_all(cwd.join(".git")).expect("git dir");
    let settings = SandboxSettings::default();
    let plan = SandboxPlan::for_turn(&settings, &cwd);
    assert!(plan.writable_roots.contains(&cwd));
    assert!(plan.protected.contains(&cwd.join(".git")));
    assert_eq!(plan.mode, SandboxMode::WorkspaceWrite);
    let _ = std::fs::remove_dir_all(cwd);
}

#[test]
fn danger_mode_is_not_process_sandboxed() {
    let cwd = temp_dir("danger");
    let settings = SandboxSettings {
        mode: SandboxMode::DangerFullAccess,
        ..SandboxSettings::default()
    };
    let plan = SandboxPlan::for_turn(&settings, &cwd);
    assert!(!plan.process_sandbox());
    assert!(plan.writable_roots.is_empty());
    assert!(plan.network_access);
    let _ = std::fs::remove_dir_all(cwd);
}

#[test]
fn unsandboxed_override_clears_roots_and_network_gate() {
    let cwd = temp_dir("override");
    let plan = SandboxPlan::for_turn(&SandboxSettings::default(), &cwd);
    let free = plan.with_override(&SandboxOverride::Unsandboxed);
    assert_eq!(free.mode, SandboxMode::DangerFullAccess);
    assert!(free.writable_roots.is_empty());
    assert!(!free.process_sandbox());
    let _ = std::fs::remove_dir_all(cwd);
}

#[test]
fn additional_permissions_extend_roots_and_network() {
    let cwd = temp_dir("additional");
    let extra = cwd.join("extra");
    std::fs::create_dir_all(&extra).expect("extra root");
    let plan = SandboxPlan::for_turn(&SandboxSettings::default(), &cwd);
    let extended = plan.with_override(&SandboxOverride::Additional {
        network: true,
        write_roots: vec![extra.clone()],
    });
    assert!(extended.writable_roots.contains(&extra));
    assert!(extended.network_access);
    assert!(extended.process_sandbox());
    // 原计划不被修改（turn 起点冻结语义）。
    assert!(!plan.writable_roots.contains(&extra));
    assert!(!plan.network_access);
    let _ = std::fs::remove_dir_all(cwd);
}

#[test]
fn policy_view_matches_plan() {
    let cwd = temp_dir("view");
    let plan = SandboxPlan::for_turn(&SandboxSettings::default(), &cwd);
    let view = plan.policy_view();
    assert_eq!(view.mode, plan.mode);
    assert_eq!(view.writable_roots, plan.writable_roots);
    assert_eq!(view.protected, plan.protected);
    let _ = std::fs::remove_dir_all(cwd);
}
