//! `write` 工具测试：创建/覆盖/清空、父目录创建、增删计数、权限错误与编码保留。

use super::super::test_support::*;
use super::super::{ToolErrorCode, execute};

/// 新建文件：`created=true`，added 为写入行数。
#[test]
fn creates_new_file() {
    let fixture = Fixture::new("write-create");
    let result = execute(
        &fixture.ctx(),
        "write",
        r#"{"path":"new.txt","content":"one\ntwo\n"}"#,
    );
    let (created, added, removed) = expect_write(&result);
    assert!(created);
    assert_eq!((added, removed), (2, 0));
    assert_eq!(fixture.read("new.txt"), "one\ntwo\n");
}

/// 覆盖既有文件：`created=false`，按行 diff 统计增删。
#[test]
fn overwrites_existing_file_completely() {
    let fixture = Fixture::new("write-overwrite");
    fixture.write("a.txt", "keep\ndrop\n");
    let result = execute(
        &fixture.ctx(),
        "write",
        r#"{"path":"a.txt","content":"keep\nnew\n"}"#,
    );
    let (created, added, removed) = expect_write(&result);
    assert!(!created);
    assert_eq!((added, removed), (1, 1));
    assert_eq!(fixture.read("a.txt"), "keep\nnew\n");
    assert!(
        !fixture.read("a.txt").contains("drop"),
        "必须是覆盖而非追加"
    );
}

/// 空内容合法并清空文件。
#[test]
fn empty_content_clears_file() {
    let fixture = Fixture::new("write-clear");
    fixture.write("a.txt", "something\n");
    let result = execute(&fixture.ctx(), "write", r#"{"path":"a.txt","content":""}"#);
    let (created, added, removed) = expect_write(&result);
    assert!(!created);
    assert_eq!(added, 0);
    assert_eq!(removed, 1);
    assert_eq!(fixture.read("a.txt"), "");
}

/// 父目录按需创建。
#[test]
fn creates_missing_parent_directories() {
    let fixture = Fixture::new("write-parent");
    let result = execute(
        &fixture.ctx(),
        "write",
        r#"{"path":"deep/nested/dir/a.txt","content":"x\n"}"#,
    );
    let (created, _, _) = expect_write(&result);
    assert!(created);
    assert_eq!(fixture.read("deep/nested/dir/a.txt"), "x\n");
}

/// 覆盖保留原文件的 CRLF 与 BOM。
#[test]
fn preserves_crlf_and_bom() {
    let fixture = Fixture::new("write-crlf");
    fixture.write_bytes("a.txt", b"\xEF\xBB\xBFone\r\ntwo\r\n");
    let result = execute(
        &fixture.ctx(),
        "write",
        r#"{"path":"a.txt","content":"one\ntwo\nthree\n"}"#,
    );
    let (created, _, _) = expect_write(&result);
    assert!(!created);
    let bytes = fixture.read_bytes("a.txt");
    assert!(bytes.starts_with(b"\xEF\xBB\xBF"));
    assert_eq!(bytes, b"\xEF\xBB\xBFone\r\ntwo\r\nthree\r\n".to_vec());
}

/// 缺少 `content` 字段报参数错误（不是静默清空）。
#[test]
fn missing_content_is_an_argument_error() {
    let fixture = Fixture::new("write-missing");
    fixture.write("a.txt", "keep\n");
    let result = execute(&fixture.ctx(), "write", r#"{"path":"a.txt"}"#);
    assert_eq!(expect_failure(&result), ToolErrorCode::InvalidArguments);
    assert_eq!(fixture.read("a.txt"), "keep\n");
}

/// 目标为目录时报 NotAFile 且不创建文件。
#[test]
fn writing_to_a_directory_fails() {
    let fixture = Fixture::new("write-dir");
    fixture.write("dir/a.txt", "");
    let result = execute(&fixture.ctx(), "write", r#"{"path":"dir","content":"x"}"#);
    assert_eq!(expect_failure(&result), ToolErrorCode::NotAFile);
}

/// 只读目录：在目录内创建文件失败为 `PermissionDenied`，且不留下任何文件。
#[cfg(unix)]
#[test]
fn permission_denied_is_reported() {
    use std::os::unix::fs::PermissionsExt;

    // root 等可绕过目录权限位的环境：探测可写则跳过（避免把环境差异当失败）。
    if directory_mode_is_ignored() {
        return;
    }
    let fixture = Fixture::new("write-perm");
    let dir = fixture.join("readonly");
    std::fs::create_dir_all(&dir).expect("create readonly dir");
    let mut permissions = std::fs::metadata(&dir).expect("metadata").permissions();
    permissions.set_mode(0o500);
    std::fs::set_permissions(&dir, permissions).expect("chmod");

    let result = execute(
        &fixture.ctx(),
        "write",
        r#"{"path":"readonly/a.txt","content":"x"}"#,
    );
    // 恢复权限以便清理。
    let mut restore = std::fs::metadata(&dir).expect("metadata").permissions();
    restore.set_mode(0o700);
    let _ = std::fs::set_permissions(&dir, restore);
    assert_eq!(expect_failure(&result), ToolErrorCode::PermissionDenied);
    assert!(!fixture.exists("readonly/a.txt"));
}

/// 探测目录权限位是否真的生效（root 下 0500 仍可写）。
#[cfg(unix)]
fn directory_mode_is_ignored() -> bool {
    use std::os::unix::fs::PermissionsExt;

    let probe = std::env::temp_dir().join(format!("lx-tools-perm-probe-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&probe);
    if std::fs::create_dir_all(&probe).is_err() {
        return true;
    }
    let Ok(metadata) = std::fs::metadata(&probe) else {
        return true;
    };
    let mut permissions = metadata.permissions();
    permissions.set_mode(0o500);
    let _ = std::fs::set_permissions(&probe, permissions);
    let ignored = std::fs::write(probe.join("x"), "x").is_ok();
    if let Ok(metadata) = std::fs::metadata(&probe) {
        let mut restore = metadata.permissions();
        restore.set_mode(0o700);
        let _ = std::fs::set_permissions(&probe, restore);
    }
    let _ = std::fs::remove_dir_all(&probe);
    ignored
}

/// 原子替换后目录里不留临时文件。
#[test]
fn write_is_atomic_and_leaves_no_temp_files() {
    let fixture = Fixture::new("write-atomic");
    fixture.write("a.txt", "one\n");
    let result = execute(
        &fixture.ctx(),
        "write",
        r#"{"path":"a.txt","content":"two\n"}"#,
    );
    let (created, _, _) = expect_write(&result);
    assert!(!created);
    assert_eq!(fixture.read("a.txt"), "two\n");
    let names: Vec<String> = std::fs::read_dir(fixture.path())
        .expect("read dir")
        .map(|entry| {
            entry
                .expect("entry")
                .file_name()
                .to_string_lossy()
                .into_owned()
        })
        .collect();
    assert_eq!(names, vec!["a.txt"]);
}

/// Unicode 内容完整写入（UTF-8 安全）。
#[test]
fn unicode_content_round_trips() {
    let fixture = Fixture::new("write-unicode");
    let content = "第一行\nemoji 👨‍👩‍👧\n";
    let arguments = serde_json::json!({ "path": "u.txt", "content": content }).to_string();
    let result = execute(&fixture.ctx(), "write", &arguments);
    let (created, added, _) = expect_write(&result);
    assert!(created);
    assert_eq!(added, 2);
    assert_eq!(fixture.read("u.txt"), content);
}

// ===== 差异正文 =====

/// 新建文件：全绿新增，行号从 1 开始。
#[test]
fn write_new_file_reports_diff_base() {
    let fixture = Fixture::new("write-diff-new");
    let result = execute(
        &fixture.ctx(),
        "write",
        r#"{"path":"new.txt","content":"one\ntwo\n"}"#,
    );
    let super::super::ToolExecutionResult::Write { diff, .. } = &result else {
        panic!("expected write result, got {result:?}");
    };
    let diff = diff.as_ref().expect("write 必须带 diff");
    assert!(diff.counts_known());
    let added: Vec<Option<usize>> = diff
        .hunks()
        .iter()
        .flat_map(|hunk| hunk.lines.iter())
        .filter_map(|line| {
            (line.kind == super::super::DiffLineKind::Added).then_some(line.new_line)
        })
        .collect();
    assert_eq!(added, vec![Some(1), Some(2)]);
}

/// 覆盖：差异来自真实 before/after，不只是计数。
#[test]
fn write_overwrite_reports_real_diff() {
    let fixture = Fixture::new("write-diff-overwrite");
    fixture.write("a.txt", "keep\ndrop\n");
    let result = execute(
        &fixture.ctx(),
        "write",
        r#"{"path":"a.txt","content":"keep\nnew\n"}"#,
    );
    let super::super::ToolExecutionResult::Write {
        diff,
        added,
        removed,
        ..
    } = &result
    else {
        panic!("expected write result, got {result:?}");
    };
    assert_eq!((*added, *removed), (1, 1));
    let diff = diff.as_ref().expect("覆盖必须带 diff");
    let body: Vec<&super::super::DiffLine> = diff
        .hunks()
        .iter()
        .flat_map(|hunk| hunk.lines.iter())
        .collect();
    assert!(body.iter().any(|line| line.text == "drop"));
    assert!(body.iter().any(|line| line.text == "new"));
}

/// 相同内容覆盖：没有差异正文，但仍是成功写入。
#[test]
fn write_same_content_has_no_diff_body() {
    let fixture = Fixture::new("write-diff-same");
    fixture.write("a.txt", "same\n");
    let result = execute(
        &fixture.ctx(),
        "write",
        r#"{"path":"a.txt","content":"same\n"}"#,
    );
    let super::super::ToolExecutionResult::Write {
        diff,
        added,
        removed,
        ..
    } = &result
    else {
        panic!("expected write result, got {result:?}");
    };
    assert_eq!((*added, *removed), (0, 0));
    assert!(diff.as_ref().is_some_and(|diff| diff.hunks().is_empty()));
}

/// 只改 EOF 换行：必须显示差异（不是「无变化」）。
#[test]
fn write_eof_newline_change_is_visible() {
    let fixture = Fixture::new("write-diff-eof");
    fixture.write("a.txt", "hello");
    let result = execute(
        &fixture.ctx(),
        "write",
        r#"{"path":"a.txt","content":"hello\n"}"#,
    );
    let super::super::ToolExecutionResult::Write { diff, .. } = &result else {
        panic!("expected write result, got {result:?}");
    };
    let diff = diff.as_ref().expect("必须带 diff");
    assert!(!diff.hunks().is_empty(), "EOF 换行变化必须可见");
    assert_eq!(fixture.read("a.txt"), "hello\n");
}

/// 非工作区：绝对路径可写（快照/落库归属由会话键决定，不由目标路径决定）。
#[test]
fn writes_absolute_path_outside_workspace() {
    let fixture = Fixture::new("write-outside");
    let outside = std::env::temp_dir().join(format!(
        "lx-write-outside-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock")
            .as_nanos()
    ));
    std::fs::create_dir_all(&outside).expect("outside");
    let target = outside.join("note.txt");
    let args = serde_json::json!({
        "path": target.to_string_lossy(),
        "content": "outside\n",
    })
    .to_string();
    let result = execute(&fixture.ctx(), "write", &args);
    let (created, added, _) = expect_write(&result);
    assert!(created);
    assert_eq!(added, 1);
    assert_eq!(std::fs::read(&target).expect("file"), b"outside\n");
    let _ = std::fs::remove_file(&target);
    let _ = std::fs::remove_dir_all(outside);
}
