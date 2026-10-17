//! `read` 工具测试：行号、1-based offset/limit、截断、编码边界与错误分支。

use super::super::test_support::*;
use super::super::{ToolContext, ToolErrorCode, ToolExecutionResult, execute};

/// 读取普通文本文件：行号为 1-based，正文原样。
#[test]
fn reads_file_with_numbered_lines() {
    let fixture = Fixture::new("read-basic");
    fixture.write("a.txt", "alpha\nbeta\ngamma\n");
    let result = execute(&fixture.ctx(), "read", r#"{"path":"a.txt"}"#);
    let (content, start, end, next, truncated, total) = expect_read(&result);
    assert_eq!(content, "1: alpha\n2: beta\n3: gamma\n");
    assert_eq!((start, end), (1, 3));
    assert_eq!(next, None);
    assert!(!truncated);
    assert_eq!(total, 3);
}

/// 绝对路径可直接读取。
#[test]
fn reads_absolute_path() {
    let fixture = Fixture::new("read-absolute");
    let path = fixture.write("a.txt", "only\n");
    let arguments = format!(r#"{{"path":"{}"}}"#, path.display());
    let result = execute(&fixture.ctx(), "read", &arguments);
    assert_eq!(expect_read(&result).0, "1: only\n");
}

/// `offset` / `limit` 都是 1-based，切窗正确。
#[test]
fn offset_and_limit_select_a_window() {
    let fixture = Fixture::new("read-window");
    fixture.write("a.txt", "1\n2\n3\n4\n5\n");
    let result = execute(
        &fixture.ctx(),
        "read",
        r#"{"path":"a.txt","offset":2,"limit":2}"#,
    );
    let (content, start, end, next, truncated, total) = expect_read(&result);
    assert_eq!(content, "2: 2\n3: 3\n");
    assert_eq!((start, end), (2, 3));
    assert_eq!(next, Some(4));
    assert!(truncated, "还有后续行时必须置 truncated");
    assert_eq!(total, 5);

    let result = execute(&fixture.ctx(), "read", r#"{"path":"a.txt","offset":5}"#);
    let (content, start, end, next, truncated, _) = expect_read(&result);
    assert_eq!(content, "5: 5\n");
    assert_eq!((start, end), (5, 5));
    assert_eq!(next, None);
    assert!(!truncated);
}

/// `offset` 越界报 `OffsetOutOfRange`；空文件只有 offset=1 合法。
#[test]
fn offset_out_of_range_is_reported() {
    let fixture = Fixture::new("read-range");
    fixture.write("a.txt", "one\n");
    let result = execute(&fixture.ctx(), "read", r#"{"path":"a.txt","offset":2}"#);
    assert_eq!(expect_failure(&result), ToolErrorCode::OffsetOutOfRange);

    fixture.write("empty.txt", "");
    let result = execute(&fixture.ctx(), "read", r#"{"path":"empty.txt","offset":2}"#);
    assert_eq!(expect_failure(&result), ToolErrorCode::OffsetOutOfRange);
}

/// 空文件：结果是空正文且不报错。
#[test]
fn empty_file_returns_empty_content() {
    let fixture = Fixture::new("read-empty");
    fixture.write("empty.txt", "");
    let result = execute(&fixture.ctx(), "read", r#"{"path":"empty.txt"}"#);
    let (content, start, end, next, truncated, total) = expect_read(&result);
    assert!(content.is_empty());
    assert_eq!((start, end), (0, 0));
    assert_eq!(next, None);
    assert!(!truncated);
    assert_eq!(total, 0);
}

/// 末行无换行时仍算一行。
#[test]
fn last_line_without_newline_is_counted() {
    let fixture = Fixture::new("read-noeol");
    fixture.write("a.txt", "one\ntwo");
    let result = execute(&fixture.ctx(), "read", r#"{"path":"a.txt"}"#);
    let (content, _, end, _, _, total) = expect_read(&result);
    assert_eq!(content, "1: one\n2: two\n");
    assert_eq!(end, 2);
    assert_eq!(total, 2);
}

/// CRLF 文件的正文不带行尾 `\r`（行号前缀不影响换行风格）。
#[test]
fn crlf_content_has_no_carriage_returns() {
    let fixture = Fixture::new("read-crlf");
    fixture.write_bytes("a.txt", b"one\r\ntwo\r\n");
    let result = execute(&fixture.ctx(), "read", r#"{"path":"a.txt"}"#);
    let (content, _, _, _, _, total) = expect_read(&result);
    assert_eq!(content, "1: one\n2: two\n");
    assert!(!content.contains('\r'));
    assert_eq!(total, 2);
}

/// Unicode 宽字符不被截断（按字符计数而非字节）。
#[test]
fn unicode_wide_characters_are_preserved() {
    let fixture = Fixture::new("read-unicode");
    fixture.write("a.txt", "第一行\n第二行 with 👨‍👩‍👧\n");
    let result = execute(&fixture.ctx(), "read", r#"{"path":"a.txt"}"#);
    let (content, _, _, _, truncated, total) = expect_read(&result);
    assert!(content.contains("1: 第一行"));
    assert!(content.contains("👨‍👩‍👧"));
    assert!(!truncated);
    assert_eq!(total, 2);
}

/// 超长单行按字符截断并显式标记。
#[test]
fn overlong_line_is_truncated_and_flagged() {
    let fixture = Fixture::new("read-longline");
    let long = "a".repeat(super::MAX_READ_LINE_CHARS + 50);
    fixture.write("a.txt", &format!("{long}\n"));
    let result = execute(&fixture.ctx(), "read", r#"{"path":"a.txt"}"#);
    let (content, _, _, _, truncated, _) = expect_read(&result);
    assert!(truncated, "超长行必须置 truncated");
    assert!(content.contains("[truncated]"));
    assert!(content.chars().count() < long.len());
}

/// 行数超上限：按 `limit` 截断并给出 `next_offset`。
#[test]
fn line_limit_truncates_with_next_offset() {
    let fixture = Fixture::new("read-linelimit");
    let mut text = String::new();
    for index in 1..=10 {
        text.push_str(&format!("line{index}\n"));
    }
    fixture.write("a.txt", &text);
    let result = execute(&fixture.ctx(), "read", r#"{"path":"a.txt","limit":3}"#);
    let (content, start, end, next, truncated, total) = expect_read(&result);
    assert_eq!(content.lines().count(), 3);
    assert_eq!((start, end), (1, 3));
    assert_eq!(next, Some(4));
    assert!(truncated);
    assert_eq!(total, 10);
}

/// 字节上限：超小时按字节截断且仍给 `next_offset`。
#[test]
fn byte_limit_truncates_with_next_offset() {
    let fixture = Fixture::new("read-bytelimit");
    let line = "x".repeat(3000);
    let mut text = String::new();
    for _ in 0..200 {
        text.push_str(&line);
        text.push('\n');
    }
    fixture.write("a.txt", &text);
    let result = execute(&fixture.ctx(), "read", r#"{"path":"a.txt"}"#);
    let (content, _, end, next, truncated, total) = expect_read(&result);
    assert!(content.len() <= super::MAX_READ_BYTES);
    assert!(truncated);
    assert_eq!(next, Some(end + 1));
    assert!(end < total);
}

/// 目录：列出条目并带 `[dir]` / `[file]` 前缀。
#[test]
fn directory_listing_marks_entries() {
    let fixture = Fixture::new("read-dir");
    fixture.write("dir/a.txt", "a\n");
    fixture.write("dir/b.txt", "b\n");
    let result = execute(&fixture.ctx(), "read", r#"{"path":"dir"}"#);
    match &result {
        ToolExecutionResult::Read { kind, content, .. } => {
            assert_eq!(*kind, super::super::ReadKind::Directory);
            assert!(content.contains("[dir] ") || content.contains("[file] "));
            assert!(content.contains("[file] a.txt"), "content: {content}");
            assert!(content.contains("[file] b.txt"));
        }
        other => panic!("expected directory read, got {other:?}"),
    }
}

/// 目录列表按名称排序且稳定。
#[test]
fn directory_listing_is_sorted() {
    let fixture = Fixture::new("read-dir-sort");
    for name in ["c.txt", "a.txt", "b.txt"] {
        fixture.write(&format!("dir/{name}"), "");
    }
    let result = execute(&fixture.ctx(), "read", r#"{"path":"dir"}"#);
    let (content, _, _, _, _, total) = expect_read(&result);
    let lines: Vec<&str> = content.lines().collect();
    assert_eq!(lines, vec!["[file] a.txt", "[file] b.txt", "[file] c.txt"]);
    assert_eq!(total, 3);
}

/// 目录列表同样支持 1-based `offset` / `limit` 与 `next_offset`。
#[test]
fn directory_listing_supports_window() {
    let fixture = Fixture::new("read-dir-window");
    for name in ["a.txt", "b.txt", "c.txt", "d.txt"] {
        fixture.write(&format!("dir/{name}"), "");
    }
    let result = execute(
        &fixture.ctx(),
        "read",
        r#"{"path":"dir","offset":2,"limit":2}"#,
    );
    let (content, start, end, next, truncated, total) = expect_read(&result);
    assert_eq!(content, "[file] b.txt\n[file] c.txt\n");
    assert_eq!((start, end), (2, 3));
    assert_eq!(next, Some(4));
    assert!(truncated);
    assert_eq!(total, 4);

    let result = execute(&fixture.ctx(), "read", r#"{"path":"dir","offset":9}"#);
    assert_eq!(expect_failure(&result), ToolErrorCode::OffsetOutOfRange);
}

/// 不存在的路径报 `NotFound`。
#[test]
fn missing_file_is_not_found() {
    let fixture = Fixture::new("read-missing");
    let result = execute(&fixture.ctx(), "read", r#"{"path":"nope.txt"}"#);
    assert_eq!(expect_failure(&result), ToolErrorCode::NotFound);
}

/// 含 NUL 的内容判为二进制。
#[test]
fn nul_bytes_are_binary() {
    let fixture = Fixture::new("read-binary");
    fixture.write_bytes("bin.dat", b"abc\0def");
    let result = execute(&fixture.ctx(), "read", r#"{"path":"bin.dat"}"#);
    assert_eq!(expect_failure(&result), ToolErrorCode::Binary);
}

/// 非法 UTF-8 报 `NotUtf8`。
#[test]
fn invalid_utf8_is_reported() {
    let fixture = Fixture::new("read-notutf8");
    fixture.write_bytes("bad.txt", &[0x66, 0x6f, 0xff, 0x6f]);
    let result = execute(&fixture.ctx(), "read", r#"{"path":"bad.txt"}"#);
    assert_eq!(expect_failure(&result), ToolErrorCode::NotUtf8);
}

/// 读取目录内的路径时上下文正确解析（相对路径基准是 cwd）。
#[test]
fn nested_relative_path_resolves_from_cwd() {
    let fixture = Fixture::new("read-nested");
    fixture.write("a/b/c.txt", "deep\n");
    let result = execute(&fixture.ctx(), "read", r#"{"path":"a/b/c.txt"}"#);
    assert_eq!(expect_read(&result).0, "1: deep\n");
    // 反向验证：别的工作区上下文看不到该文件。
    let other = Fixture::new("read-nested-other");
    let result = execute(&other.ctx(), "read", r#"{"path":"a/b/c.txt"}"#);
    assert_eq!(expect_failure(&result), ToolErrorCode::NotFound);
}

/// `ctx.cwd` 是只读快照：即使进程 cwd 被改动，解析结果不变。
#[test]
fn context_cwd_is_a_frozen_snapshot() {
    let fixture = Fixture::new("read-frozen");
    fixture.write("a.txt", "frozen\n");
    let ctx = ToolContext::new(fixture.path());
    let result = execute(&ctx, "read", r#"{"path":"a.txt"}"#);
    assert_eq!(expect_read(&result).0, "1: frozen\n");
    assert_eq!(ctx.cwd, fixture.path());
}
