//! 读后版本保护的测试：token 稳定性、登记表隔离与四种拒绝路径。

use std::sync::Arc;

use super::*;
use crate::chat::tools::test_support::Fixture;
use crate::chat::tools::{ToolContext, ToolErrorCode, execute};

/// 同一镜像永远得到同一 token；任一字段变化都改变 token。
#[test]
fn version_token_is_stable_and_sensitive() {
    let image = RawImage::regular(b"one\n".to_vec(), Some(0o644));
    assert_eq!(version_token_of(&image), version_token_of(&image));

    let changed = RawImage::regular(b"two\n".to_vec(), Some(0o644));
    assert_ne!(version_token_of(&image), version_token_of(&changed));

    let mode_changed = RawImage::regular(b"one\n".to_vec(), Some(0o600));
    assert_ne!(version_token_of(&image), version_token_of(&mode_changed));

    let link = RawImage::symlink(std::path::PathBuf::from("/tmp/x"));
    assert_ne!(version_token_of(&image), version_token_of(&link));
    assert!(version_token_of(&image).starts_with(crate::platform::VERSION_PREFIX));
}

/// 由持久化字段重算必须与镜像 token 完全一致（跨进程持久化的前提）。
#[test]
fn version_token_from_parts_matches_image() {
    let image = RawImage::regular(b"abc".to_vec(), Some(0o755));
    let token = version_token_of(&image);
    assert_eq!(
        crate::platform::version_token_from_parts("regular", Some(b"abc"), Some(0o755), None),
        Some(token)
    );
    assert_eq!(
        crate::platform::version_token_from_parts("missing", None, None, None),
        Some(version_token_of(&RawImage::missing()))
    );
    let link = RawImage::symlink(std::path::PathBuf::from("/tmp/target"));
    let encoded = crate::platform::encode_path(std::path::Path::new("/tmp/target"));
    assert_eq!(
        crate::platform::version_token_from_parts("symlink", None, None, Some(&encoded)),
        Some(version_token_of(&link))
    );
    // 损坏的 kind 不参与 CAS。
    assert_eq!(
        crate::platform::version_token_from_parts("bogus", Some(b"x"), None, None),
        None
    );
}

/// 登记表按会话隔离：同一路径在不同会话里各自记录。
#[test]
fn registries_are_per_session() {
    drop_registry("sess-a");
    drop_registry("sess-b");
    let a = registry("sess-a");
    let b = registry("sess-b");
    let image = RawImage::regular(b"x".to_vec(), None);
    let token = a.record("key", "a.txt", &image, &image);
    assert_eq!(
        a.views_for("key").map(|(followed, _)| followed),
        Some(token)
    );
    assert_eq!(b.views_for("key"), None);
    drop_registry("sess-a");
    drop_registry("sess-b");
}

/// 读取后写：`read` 返回 token，`write` 带上它才成功。
#[test]
fn read_then_write_with_version_succeeds() {
    let fixture = Fixture::new("rv-ok");
    fixture.write("a.txt", "one\n");
    let ctx = fixture.ctx().with_reads(Arc::new(ReadRegistry::default()));
    let read = execute(&ctx, "read", r#"{"path":"a.txt"}"#);
    let token = read_token(&read);
    let arguments = serde_json::json!({
        "path": "a.txt",
        "content": "two\n",
        "read_version": token,
    })
    .to_string();
    let result = execute(&ctx, "write", &arguments);
    assert!(!result.is_failure(), "带上读版本必须允许覆盖：{result:?}");
    assert_eq!(fixture.read("a.txt"), "two\n");
}

/// 没读过就覆盖：拒绝且文件不变。
#[test]
fn write_without_read_is_rejected() {
    let fixture = Fixture::new("rv-noread");
    fixture.write("a.txt", "one\n");
    let ctx = fixture.ctx().with_reads(Arc::new(ReadRegistry::default()));
    let before = fixture.read_bytes("a.txt");
    let result = execute(&ctx, "write", r#"{"path":"a.txt","content":"two\n"}"#);
    assert_eq!(expect_code(&result), ToolErrorCode::StaleRead);
    assert_eq!(fixture.read_bytes("a.txt"), before, "拒绝必须零写入");
}

/// 读了之后被外部修改：写入时拒绝且保留外部内容。
#[test]
fn external_change_after_read_is_rejected() {
    let fixture = Fixture::new("rv-external");
    fixture.write("a.txt", "one\n");
    let ctx = fixture.ctx().with_reads(Arc::new(ReadRegistry::default()));
    let read = execute(&ctx, "read", r#"{"path":"a.txt"}"#);
    let token = read_token(&read);
    // 外部进程改了文件。
    fixture.write("a.txt", "external\n");
    let arguments = serde_json::json!({
        "path": "a.txt",
        "content": "mine\n",
        "read_version": token,
    })
    .to_string();
    let result = execute(&ctx, "write", &arguments);
    assert_eq!(expect_code(&result), ToolErrorCode::StaleRead);
    assert_eq!(fixture.read("a.txt"), "external\n", "拒绝必须零写入");
}

/// 伪造/过期的 token 同样被拒绝（票据不能自证）。
#[test]
fn forged_version_is_rejected() {
    let fixture = Fixture::new("rv-forged");
    fixture.write("a.txt", "one\n");
    let ctx = fixture.ctx().with_reads(Arc::new(ReadRegistry::default()));
    let _ = execute(&ctx, "read", r#"{"path":"a.txt"}"#);
    let arguments = serde_json::json!({
        "path": "a.txt",
        "content": "two\n",
        "read_version": "lxv1-00000000000000000000000000000000",
    })
    .to_string();
    let result = execute(&ctx, "write", &arguments);
    assert_eq!(expect_code(&result), ToolErrorCode::StaleRead);
    assert_eq!(fixture.read("a.txt"), "one\n");
}

/// 新文件：目标不存在时允许创建；提交前目标已出现则拒绝。
#[test]
fn create_requires_target_to_be_missing() {
    let fixture = Fixture::new("rv-create");
    let ctx = fixture.ctx().with_reads(Arc::new(ReadRegistry::default()));
    let result = execute(&ctx, "write", r#"{"path":"new.txt","content":"x\n"}"#);
    assert!(!result.is_failure(), "新文件无需读版本：{result:?}");
    assert_eq!(fixture.read("new.txt"), "x\n");

    // 目标是已存在的文件：即便内容“像是新建”，也必须先读。
    fixture.write("exists.txt", "old\n");
    let result = execute(&ctx, "write", r#"{"path":"exists.txt","content":"new\n"}"#);
    assert_eq!(expect_code(&result), ToolErrorCode::StaleRead);
    assert_eq!(fixture.read("exists.txt"), "old\n");
}

/// `edit` 也会要求 `read_versions`：缺 token 时整批零写入。
#[test]
fn edit_requires_read_versions() {
    let fixture = Fixture::new("rv-edit");
    fixture.write("a.txt", "alpha\nbeta\n");
    let ctx = fixture.ctx().with_reads(Arc::new(ReadRegistry::default()));
    let patch = "*** Begin Patch\n*** Update File: a.txt\n@@\n-beta\n+gamma\n*** End Patch\n";
    let result = execute(
        &ctx,
        "edit",
        &serde_json::json!({ "patch": patch }).to_string(),
    );
    assert_eq!(expect_code(&result), ToolErrorCode::StaleRead);
    assert_eq!(fixture.read("a.txt"), "alpha\nbeta\n", "拒绝必须零写入");

    // 读一次后带 token 即可成功。
    let read = execute(&ctx, "read", r#"{"path":"a.txt"}"#);
    let token = read_token(&read);
    let arguments = serde_json::json!({
        "patch": patch,
        "read_versions": { "a.txt": token },
    })
    .to_string();
    let result = execute(&ctx, "edit", &arguments);
    assert!(!result.is_failure(), "带读版本必须允许补丁：{result:?}");
    assert_eq!(fixture.read("a.txt"), "alpha\ngamma\n");
}

/// 路径别名（`./a.txt` 与 `a.txt`）共享同一份读取版本。
#[test]
fn path_aliases_share_one_version() {
    let fixture = Fixture::new("rv-alias");
    fixture.write("a.txt", "one\n");
    let ctx = fixture.ctx().with_reads(Arc::new(ReadRegistry::default()));
    let read = execute(&ctx, "read", r#"{"path":"./a.txt"}"#);
    let token = read_token(&read);
    let arguments = serde_json::json!({
        "path": "a.txt",
        "content": "two\n",
        "read_version": token,
    })
    .to_string();
    let result = execute(&ctx, "write", &arguments);
    assert!(
        !result.is_failure(),
        "同一文件的两个写法必须共享版本：{result:?}"
    );
    assert_eq!(fixture.read("a.txt"), "two\n");
}

/// 符号链接：`read` 同时登记“跟随后的内容”与“目录项本身”，
/// 因此 `delete`（目录项语义）不会被误判为读取后外部修改。
#[cfg(unix)]
#[test]
fn symlink_read_records_both_views_for_delete() {
    let fixture = Fixture::new("rv-symlink-delete");
    let real = fixture.write("real.txt", "one\n");
    let link = fixture.join("link.txt");
    std::os::unix::fs::symlink(&real, &link).expect("symlink");
    let ctx = fixture.ctx().with_reads(Arc::new(ReadRegistry::default()));
    let read = execute(&ctx, "read", r#"{"path":"link.txt"}"#);
    let token = read_token(&read);
    // 用返回的 token 删除链接本身：目录项视图必须被接受。
    let patch = "*** Begin Patch\n*** Delete File: link.txt\n*** End Patch\n";
    let arguments = serde_json::json!({
        "patch": patch,
        "read_versions": { "link.txt": token },
    })
    .to_string();
    let result = execute(&ctx, "edit", &arguments);
    assert!(!result.is_failure(), "删除符号链接必须被允许：{result:?}");
    assert!(!fixture.exists("link.txt"), "链接本身必须被删除");
    assert_eq!(fixture.read("real.txt"), "one\n", "目标内容不得被删除");
}

/// 取出 `read` 结果的版本 token（测试辅助）。
fn read_token(result: &crate::chat::tools::ToolExecutionResult) -> String {
    match result {
        crate::chat::tools::ToolExecutionResult::Read {
            version: Some(version),
            ..
        } => version.clone(),
        other => panic!("expected read result with version, got {other:?}"),
    }
}

/// 取出失败错误码（测试辅助）。
fn expect_code(result: &crate::chat::tools::ToolExecutionResult) -> ToolErrorCode {
    match result {
        crate::chat::tools::ToolExecutionResult::Failed { code, .. } => *code,
        other => panic!("expected failure, got {other:?}"),
    }
}

/// 只有 `ToolContext` 的 cwd 语义参与相等比较（登记表是句柄）。
#[test]
fn context_equality_ignores_registry_handle() {
    let a = ToolContext::new("/tmp/x");
    let b = a.clone().with_reads(Arc::new(ReadRegistry::default()));
    assert_eq!(a, b);
    assert_ne!(a, ToolContext::new("/tmp/y"));
}
