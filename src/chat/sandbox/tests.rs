//! `chat::sandbox` 单元测试：计划构建、覆盖、会话规则与文件工具视图。

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
    assert_eq!(plan.policy.access_for(&cwd), FileSystemAccessMode::Read);
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
    assert_eq!(
        plan.policy.access_for(&cwd.join(".git/config")),
        FileSystemAccessMode::Read
    );
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
        network: Some(true),
        file_system: Some(FileSystemRequest {
            write: vec![extra.display().to_string()],
            ..FileSystemRequest::default()
        }),
    });
    assert!(extended.writable_roots.contains(&extra));
    assert!(extended.network_access);
    assert!(extended.process_sandbox());
    assert_eq!(
        extended.policy.access_for(&extra.join("x")),
        FileSystemAccessMode::Write
    );
    // 原计划不被修改（turn 起点冻结语义）。
    assert!(!plan.writable_roots.contains(&extra));
    assert!(!plan.network_access);
    let _ = std::fs::remove_dir_all(cwd);
}

#[test]
fn additional_deny_and_read_apply_to_the_command_policy() {
    let cwd = temp_dir("additional-deny");
    std::fs::create_dir_all(cwd.join("sealed")).expect("sealed");
    let plan = SandboxPlan::for_turn(&SandboxSettings::default(), &cwd);
    let extended = plan.with_override(&SandboxOverride::Additional {
        network: None,
        file_system: Some(FileSystemRequest {
            read: vec![cwd.join("sealed").display().to_string()],
            deny: vec![format!("{}/**/.env", cwd.display())],
            ..FileSystemRequest::default()
        }),
    });
    assert!(extended.protected.contains(&cwd.join("sealed")));
    assert_eq!(extended.deny_globs.len(), 1);
    assert_eq!(
        extended.policy.access_for(&cwd.join("sealed/x")),
        FileSystemAccessMode::Read
    );
    assert_eq!(
        extended.policy.access_for(&cwd.join("a/.env")),
        FileSystemAccessMode::Deny
    );
    let spec = extended.spec(&cwd);
    assert!(spec.read_only_paths.contains(&cwd.join("sealed")));
    assert_eq!(spec.deny_globs.len(), 1);
    assert_eq!(spec.glob_scan_max_depth, 16);
    let _ = std::fs::remove_dir_all(cwd);
}

#[test]
fn policy_view_matches_plan() {
    let cwd = temp_dir("view");
    let plan = SandboxPlan::for_turn(&SandboxSettings::default(), &cwd);
    let view = plan.policy_view();
    assert_eq!(view.mode, plan.mode);
    assert_eq!(view.policy, plan.policy);
    let _ = std::fs::remove_dir_all(cwd);
}

#[test]
fn approved_exact_and_prefix_rules_bypass_the_command() {
    let cwd = temp_dir("rules");
    let plan = SandboxPlan::for_turn(&SandboxSettings::default(), &cwd).with_exact_rules(
        vec![vec!["touch".into(), "~/x".into()]],
        vec![vec!["cargo".into(), "publish".into()]],
    );
    assert!(plan.command_is_approved("touch '~/x'"));
    assert!(plan.command_is_approved("cargo publish --dry-run"));
    assert!(!plan.command_is_approved("cargo build"));
    assert!(!plan.command_is_approved("touch ~/y"));
    let _ = std::fs::remove_dir_all(cwd);
}
