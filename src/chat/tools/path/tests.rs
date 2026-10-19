//! 路径解析测试：相对路径基准、绝对路径直通与不可用 cwd。

use std::path::PathBuf;

use super::super::test_support::Fixture;
use super::super::types::{ToolContext, ToolErrorCode};
use super::*;

#[test]
fn absolute_path_is_returned_as_is() {
    let fixture = Fixture::new("path-absolute");
    let absolute = fixture.join("a.txt");
    let ctx = fixture.ctx();
    let resolved = resolve(&ctx, &absolute.to_string_lossy()).expect("resolved");
    assert_eq!(resolved, absolute);
    assert!(resolved.is_absolute());
}

#[test]
fn relative_path_joins_context_cwd() {
    let fixture = Fixture::new("path-relative");
    let ctx = fixture.ctx();
    let resolved = resolve(&ctx, "sub/dir/a.txt").expect("resolved");
    assert_eq!(resolved, fixture.path().join("sub/dir/a.txt"));
    // 解析结果完全由 ctx.cwd 决定，不读取进程 cwd（进程 cwd 是仓库工作区）。
    let process_cwd = std::env::current_dir().expect("process cwd");
    assert!(
        !resolved.starts_with(&process_cwd) || resolved.starts_with(fixture.path()),
        "resolved path must be anchored at ctx.cwd, got {}",
        resolved.display()
    );
}

#[test]
fn empty_and_unusable_cwd_are_unresolvable() {
    let empty = ToolContext::new("");
    let failure = resolve(&empty, "a.txt").expect_err("empty cwd");
    assert_eq!(failure.code, ToolErrorCode::PathUnresolvable);

    let relative = ToolContext::new("relative/dir");
    let failure = resolve(&relative, "a.txt").expect_err("relative cwd");
    assert_eq!(failure.code, ToolErrorCode::PathUnresolvable);

    let missing = ToolContext::new("/definitely/not/here/lx-tools-path");
    let failure = resolve(&missing, "a.txt").expect_err("missing cwd");
    assert_eq!(failure.code, ToolErrorCode::PathUnresolvable);
}

#[test]
fn blank_path_is_invalid() {
    let fixture = Fixture::new("path-blank");
    let ctx = fixture.ctx();
    let failure = resolve(&ctx, "   ").expect_err("blank path");
    assert_eq!(failure.code, ToolErrorCode::InvalidArguments);
}

#[test]
fn symlinked_cwd_is_usable() {
    #[cfg(unix)]
    {
        let fixture = Fixture::new("path-symlink");
        fixture.write("real/a.txt", "x\n");
        let link = fixture.join("link");
        std::os::unix::fs::symlink(fixture.join("real"), &link).expect("symlink");
        let ctx = ToolContext::new(link.clone());
        let resolved = resolve(&ctx, "a.txt").expect("resolved");
        assert_eq!(resolved, link.join("a.txt"));
    }
}

#[test]
fn io_error_mapping_keeps_permission_and_not_found() {
    let path = PathBuf::from("/tmp/x");
    let not_found = map_io_error(
        &std::io::Error::new(std::io::ErrorKind::NotFound, "gone"),
        "read",
        &path,
    );
    assert_eq!(not_found.code, ToolErrorCode::NotFound);
    let denied = map_io_error(
        &std::io::Error::new(std::io::ErrorKind::PermissionDenied, "denied"),
        "read",
        &path,
    );
    assert_eq!(denied.code, ToolErrorCode::PermissionDenied);
    let other = map_io_error(&std::io::Error::other("boom"), "read", &path);
    assert_eq!(other.code, ToolErrorCode::Io);
    assert!(other.message.contains("read"));
    assert!(other.message.contains("/tmp/x"));
}
