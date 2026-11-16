//! `chat::tools::sandbox_gate` 测试：模式、边界、保护路径与写工具集成。

use std::path::{Path, PathBuf};

use super::*;
use crate::chat::sandbox::SandboxPlan;
use crate::chat::tools::types::ToolExecutionResult;
use crate::config::sandbox::{SandboxMode, SandboxSettings};

fn temp_dir(label: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "lx-gate-{}-{label}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or_default()
    ));
    std::fs::create_dir_all(&dir).expect("create temp dir");
    std::fs::canonicalize(&dir).expect("canonical temp dir")
}

fn ctx(dir: &Path, mode: SandboxMode) -> ToolContext {
    let settings = SandboxSettings {
        mode,
        ..SandboxSettings::default()
    };
    let plan = SandboxPlan::for_turn(&settings, dir);
    ToolContext::new(dir.to_path_buf()).with_sandbox(plan.policy_view())
}

#[test]
fn read_only_rejects_all_writes() {
    let dir = temp_dir("readonly");
    let ctx = ctx(&dir, SandboxMode::ReadOnly);
    let failure = ensure_writable(&ctx, &[&dir.join("x.txt")]).expect_err("denied");
    assert_eq!(
        failure.code,
        crate::chat::tools::ToolErrorCode::SandboxDenied
    );
    assert!(failure.message.contains("read-only"));
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn workspace_write_allows_inside_and_rejects_outside() {
    let dir = temp_dir("workspace");
    let ctx = ctx(&dir, SandboxMode::WorkspaceWrite);
    ensure_writable(&ctx, &[&dir.join("a").join("b.txt")]).expect("inside");
    // `$HOME` 通常不在 cwd/TMPDIR//tmp 之内。
    if let Some(home) = std::env::var_os("HOME") {
        let outside = PathBuf::from(home).join("lx-sandbox-gate-outside.txt");
        assert!(ensure_writable(&ctx, &[&outside]).is_err());
    }
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn protected_git_path_is_rejected_but_sibling_prefix_is_not() {
    let dir = temp_dir("protected");
    std::fs::create_dir_all(dir.join(".git")).expect("git dir");
    let ctx = ctx(&dir, SandboxMode::WorkspaceWrite);
    let failure = ensure_writable(&ctx, &[&dir.join(".git").join("config")]).expect_err("denied");
    assert!(failure.message.contains("protected"));
    ensure_writable(&ctx, &[&dir.join(".gitlab").join("x")]).expect("not protected");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn parent_escape_is_rejected() {
    let dir = temp_dir("escape");
    let ctx = ctx(&dir, SandboxMode::WorkspaceWrite);
    assert!(ensure_writable(&ctx, &[&dir.join("..").join("x")]).is_err());
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn symlink_pointing_outside_is_rejected_after_resolution() {
    let base = temp_dir("symlink");
    let root = base.join("ws");
    let Some(home) = std::env::var_os("HOME").map(PathBuf::from) else {
        return;
    };
    let outside = home.join("lx-sandbox-symlink-target");
    std::fs::create_dir_all(&root).expect("root");
    std::fs::create_dir_all(&outside).expect("outside");
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(&outside, root.join("link")).expect("symlink");
        let ctx = ctx(&root, SandboxMode::WorkspaceWrite);
        let target = root.join("link").join("x.txt");
        let resolved =
            crate::platform::sandbox::roots::resolve_write_target(&target).expect("resolve");
        assert!(ensure_writable_resolved(&ctx, &target, &resolved).is_err());
    }
    let _ = std::fs::remove_dir_all(base);
}

#[test]
fn write_tool_respects_read_only_mode() {
    let dir = temp_dir("write-tool");
    let ctx = ctx(&dir, SandboxMode::ReadOnly);
    let target = dir.join("blocked.txt");
    let arguments = format!(r#"{{"path":"{}","content":"x"}}"#, target.display());
    let result = crate::chat::tools::execute(&ctx, "write", &arguments);
    match result {
        ToolExecutionResult::Failed { code, .. } => {
            assert_eq!(code, crate::chat::tools::ToolErrorCode::SandboxDenied);
        }
        other => panic!("expected failure, got {other:?}"),
    }
    assert!(!target.exists());
    let _ = std::fs::remove_dir_all(dir);
}
