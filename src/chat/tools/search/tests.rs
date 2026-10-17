//! `search` 工具测试：默认 cwd、正则/字面量、大小写、include、忽略规则、
//! 排序稳定性、上限截断与错误分支。

use super::super::test_support::*;
use super::super::{ToolErrorCode, execute};

/// 默认从工作区搜索，命中带 1-based 行号。
#[test]
fn default_search_covers_workspace_root() {
    let fixture = Fixture::new("search-default");
    fixture.write("a.txt", "alpha\nneedle here\n");
    fixture.write("sub/b.txt", "another needle\n");
    let result = execute(
        &fixture.ctx(),
        "search",
        r#"{"pattern":"needle","literal":true}"#,
    );
    let (matches, truncated, files) = expect_search(&result);
    assert_eq!(matches.len(), 2);
    assert!(!truncated);
    assert!(files >= 2);
    let lines: Vec<(String, usize)> = matches
        .iter()
        .map(|hit| (hit.path.display().to_string(), hit.line))
        .collect();
    assert_eq!(lines[0].1, 2);
    assert_eq!(lines[1].1, 1);
    // 排序：路径字典序在前，行号递增。
    let mut sorted = lines.clone();
    sorted.sort();
    assert_eq!(lines, sorted);
}

/// 正则匹配（非字面量）。
#[test]
fn regex_pattern_is_supported() {
    let fixture = Fixture::new("search-regex");
    fixture.write("a.txt", "let x = 42;\nlet y = 7;\n");
    let result = execute(
        &fixture.ctx(),
        "search",
        r#"{"pattern":"let [a-z] = [0-9]+"}"#,
    );
    let (matches, _, _) = expect_search(&result);
    assert_eq!(matches.len(), 2);
}

/// 字面量模式把正则元字符当普通字符。
#[test]
fn literal_pattern_treats_metacharacters_as_text() {
    let fixture = Fixture::new("search-literal");
    fixture.write("a.txt", "a.b\naxb\n");
    let result = execute(
        &fixture.ctx(),
        "search",
        r#"{"pattern":"a.b","literal":true}"#,
    );
    let (matches, _, _) = expect_search(&result);
    assert_eq!(matches.len(), 1);
    assert_eq!(matches[0].line, 1);

    let result = execute(&fixture.ctx(), "search", r#"{"pattern":"a.b"}"#);
    let (matches, _, _) = expect_search(&result);
    assert_eq!(matches.len(), 2, "正则 a.b 应同时匹配 a.b 与 axb");
}

/// `case_sensitive` 开关生效；默认不敏感。
#[test]
fn case_sensitivity_is_configurable() {
    let fixture = Fixture::new("search-case");
    fixture.write("a.txt", "Needle\nneedle\n");
    let insensitive = execute(
        &fixture.ctx(),
        "search",
        r#"{"pattern":"NEEDLE","literal":true}"#,
    );
    assert_eq!(expect_search(&insensitive).0.len(), 2);

    let sensitive = execute(
        &fixture.ctx(),
        "search",
        r#"{"pattern":"NEEDLE","literal":true,"case_sensitive":true}"#,
    );
    assert_eq!(expect_search(&sensitive).0.len(), 0);
}

/// `include` glob 只保留匹配的文件。
#[test]
fn include_glob_filters_candidate_files() {
    let fixture = Fixture::new("search-include");
    fixture.write("a.rs", "needle\n");
    fixture.write("a.txt", "needle\n");
    fixture.write("nested/b.rs", "needle\n");
    let result = execute(
        &fixture.ctx(),
        "search",
        r#"{"pattern":"needle","literal":true,"include":"*.rs"}"#,
    );
    let (matches, _, _) = expect_search(&result);
    assert_eq!(matches.len(), 2, "matches: {matches:?}");
    assert!(
        matches
            .iter()
            .all(|hit| hit.path.extension().is_some_and(|ext| ext == "rs"))
    );
}

/// 尊重 `.gitignore` 忽略规则。
#[test]
fn gitignore_rules_are_respected() {
    let fixture = Fixture::new("search-ignore");
    fixture.write(".gitignore", "ignored/\n*.log\n");
    fixture.write("ignored/a.txt", "needle\n");
    fixture.write("server.log", "needle\n");
    fixture.write("kept.txt", "needle\n");
    let result = execute(
        &fixture.ctx(),
        "search",
        r#"{"pattern":"needle","literal":true}"#,
    );
    let (matches, _, _) = expect_search(&result);
    assert_eq!(matches.len(), 1, "matches: {matches:?}");
    assert!(matches[0].path.ends_with("kept.txt"));
}

/// 显式绝对路径可以跨工作区搜索。
#[test]
fn explicit_absolute_path_searches_outside_workspace() {
    let outside = Fixture::new("search-outside");
    outside.write("x.txt", "needle\n");
    let workspace = Fixture::new("search-outside-ws");
    let arguments = format!(
        r#"{{"pattern":"needle","literal":true,"path":"{}"}}"#,
        outside.path().display()
    );
    let result = execute(&workspace.ctx(), "search", &arguments);
    let (matches, _, _) = expect_search(&result);
    assert_eq!(matches.len(), 1);
    assert!(matches[0].path.starts_with(outside.path()));
}

/// 单文件搜索（`path` 指向文件）。
#[test]
fn searching_a_single_file_works() {
    let fixture = Fixture::new("search-file");
    fixture.write("a.txt", "needle\nneedle\n");
    fixture.write("b.txt", "needle\n");
    let result = execute(
        &fixture.ctx(),
        "search",
        r#"{"pattern":"needle","literal":true,"path":"a.txt"}"#,
    );
    let (matches, _, files) = expect_search(&result);
    assert_eq!(matches.len(), 2);
    assert_eq!(files, 1);
}

/// `limit` 截断：命中数达到上限即停止并置 `truncated`。
#[test]
fn limit_truncates_results() {
    let fixture = Fixture::new("search-limit");
    let mut text = String::new();
    for _ in 0..50 {
        text.push_str("needle\n");
    }
    fixture.write("a.txt", &text);
    let result = execute(
        &fixture.ctx(),
        "search",
        r#"{"pattern":"needle","literal":true,"limit":5}"#,
    );
    let (matches, truncated, _) = expect_search(&result);
    assert_eq!(matches.len(), 5);
    assert!(truncated, "达到 limit 必须置 truncated");
}

/// 非法正则报 `PatternInvalid`。
#[test]
fn invalid_regex_is_reported() {
    let fixture = Fixture::new("search-badregex");
    fixture.write("a.txt", "x\n");
    let result = execute(&fixture.ctx(), "search", r#"{"pattern":"[unclosed"}"#);
    assert_eq!(expect_failure(&result), ToolErrorCode::PatternInvalid);
}

/// 非法 include glob 报 `PatternInvalid`。
#[test]
fn invalid_include_glob_is_reported() {
    let fixture = Fixture::new("search-badglob");
    fixture.write("a.txt", "x\n");
    let result = execute(
        &fixture.ctx(),
        "search",
        r#"{"pattern":"x","include":"[unclosed"}"#,
    );
    assert_eq!(expect_failure(&result), ToolErrorCode::PatternInvalid);
}

/// 不存在的搜索根报 `NotFound`。
#[test]
fn missing_search_root_is_not_found() {
    let fixture = Fixture::new("search-missing");
    let result = execute(&fixture.ctx(), "search", r#"{"pattern":"x","path":"nope"}"#);
    assert_eq!(expect_failure(&result), ToolErrorCode::NotFound);
}

/// 命中行超长按字符截断并显式标记。
#[test]
fn long_matching_line_is_truncated() {
    let fixture = Fixture::new("search-longline");
    let long = format!("needle{}", "z".repeat(super::MAX_SEARCH_LINE_CHARS + 20));
    fixture.write("a.txt", &format!("{long}\n"));
    let result = execute(
        &fixture.ctx(),
        "search",
        r#"{"pattern":"needle","literal":true}"#,
    );
    let (matches, _, _) = expect_search(&result);
    assert_eq!(matches.len(), 1);
    assert!(matches[0].line_truncated);
    assert!(matches[0].text.contains("[truncated]"));
}

/// 二进制文件（含 NUL）与非 UTF-8 文件被跳过且不报错。
#[test]
fn binary_and_non_utf8_files_are_skipped() {
    let fixture = Fixture::new("search-binary");
    fixture.write_bytes("bin.dat", b"needle\0");
    fixture.write_bytes("bad.txt", &[0xff, 0xfe, 0x66]);
    fixture.write("ok.txt", "needle\n");
    let result = execute(
        &fixture.ctx(),
        "search",
        r#"{"pattern":"needle","literal":true}"#,
    );
    let (matches, _, _) = expect_search(&result);
    assert_eq!(matches.len(), 1);
    assert!(matches[0].path.ends_with("ok.txt"));
}

/// CRLF 文件命中时正文不带 `\r`。
#[test]
fn crlf_lines_are_stripped_in_matches() {
    let fixture = Fixture::new("search-crlf");
    fixture.write_bytes("a.txt", b"one\r\nneedle\r\n");
    let result = execute(
        &fixture.ctx(),
        "search",
        r#"{"pattern":"needle","literal":true}"#,
    );
    let (matches, _, _) = expect_search(&result);
    assert_eq!(matches.len(), 1);
    assert_eq!(matches[0].text, "needle");
}

/// 候选文件数达到内部上限时标记截断（用更小的上限注入验证真实分支）。
#[test]
fn file_cap_marks_truncation() {
    let fixture = Fixture::new("search-filecap");
    for index in 0..8 {
        fixture.write(&format!("f{index}.txt"), "needle\n");
    }
    let caps = super::Caps {
        results: 100,
        files: 3,
        ..super::Caps::default()
    };
    let (files, truncated) = super::collect_files(fixture.path(), None, &caps);
    assert_eq!(files.len(), 3);
    assert!(truncated, "达到文件上限必须置 truncated");
}

/// 时限已过时立即停止并标记截断。
#[test]
fn timeout_marks_truncation() {
    let fixture = Fixture::new("search-timeout");
    for index in 0..4 {
        fixture.write(&format!("f{index}.txt"), "needle\n");
    }
    let caps = super::Caps {
        results: 100,
        timeout: std::time::Duration::ZERO,
        ..super::Caps::default()
    };
    let matcher = super::build_matcher("needle", true, false).expect("matcher");
    let files: Vec<std::path::PathBuf> = (0..4)
        .map(|index| fixture.join(&format!("f{index}.txt")))
        .collect();
    let result = super::scan_files(&matcher, &files, &caps).expect("scan");
    let (matches, truncated, _) = expect_search(&result);
    assert!(matches.is_empty(), "超时后不应继续扫描");
    assert!(truncated, "超时必须置 truncated");
}

/// 单文件字节上限：超过上限的文件被跳过并标记截断。
#[test]
fn oversized_file_is_skipped_and_flagged() {
    let fixture = Fixture::new("search-filemax");
    fixture.write("big.txt", "needle\n");
    let caps = super::Caps {
        results: 100,
        file_bytes: 1,
        ..super::Caps::default()
    };
    let matcher = super::build_matcher("needle", true, false).expect("matcher");
    let files = [fixture.join("big.txt")];
    let result = super::scan_files(&matcher, &files, &caps).expect("scan");
    let (matches, truncated, _) = expect_search(&result);
    assert!(matches.is_empty());
    assert!(truncated);
}

/// `include` glob 相对搜索根匹配：`*.rs` 命中任意深度，路径限定 glob 只看子树。
#[test]
fn include_glob_is_relative_to_search_root() {
    let fixture = Fixture::new("search-include-root");
    fixture.write("src/a.rs", "needle\n");
    fixture.write("src/deep/b.rs", "needle\n");
    fixture.write("other/c.rs", "needle\n");

    let result = execute(
        &fixture.ctx(),
        "search",
        r#"{"pattern":"needle","literal":true,"include":"src/**/*.rs"}"#,
    );
    let (matches, _, _) = expect_search(&result);
    assert_eq!(matches.len(), 2, "matches: {matches:?}");
    assert!(
        matches
            .iter()
            .all(|hit| hit.path.to_string_lossy().contains("/src/")),
        "matches: {matches:?}"
    );
}
