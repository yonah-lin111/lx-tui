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

/// 尊重 `.gitignore` 忽略规则（在真正的 git 仓库内；`require_git(true)` 与 git 语义一致）。
#[test]
fn gitignore_rules_are_respected() {
    let fixture = Fixture::new("search-ignore");
    fixture.write(".git/config", "[core]\n\trepositoryformatversion = 0\n");
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
    let deadline = super::deadline(&caps);
    let (files, truncated) = super::collect_files(fixture.path(), None, &caps, deadline);
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
    let deadline = super::deadline(&caps);
    let result = super::scan_files(&matcher, &files, &caps, deadline).expect("scan");
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
    let deadline = super::deadline(&caps);
    let result = super::scan_files(&matcher, &files, &caps, deadline).expect("scan");
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

/// `.git/` 与 `.jj/` 是 always-ignore：无论是否搜隐藏文件都排除。
///
/// 凭据样式的内容（remote URL 里的密码）绝不能进模型上下文或 `result_json`。
#[test]
fn search_skips_git_and_jj_directories() {
    let fixture = Fixture::new("search-vcs");
    fixture.write(
        ".git/config",
        "[remote \"origin\"]\n\turl = https://user:SECRET@example.com/repo.git\n",
    );
    fixture.write(".jj/repo", "SECRET\n");
    fixture.write("visible.txt", "SECRET here\n");

    let result = execute(
        &fixture.ctx(),
        "search",
        r#"{"pattern":"SECRET","literal":true}"#,
    );
    let (matches, _, files_scanned) = expect_search(&result);
    assert_eq!(matches.len(), 1, "matches: {matches:?}");
    assert!(matches[0].path.ends_with("visible.txt"));
    assert!(
        matches
            .iter()
            .all(|hit| !hit.path.to_string_lossy().contains("/.git/")),
        "VCS 目录不得命中: {matches:?}"
    );
    assert!(
        matches
            .iter()
            .all(|hit| !hit.path.to_string_lossy().contains("/.jj/")),
        "VCS 目录不得命中: {matches:?}"
    );
    // `files_scanned` 也只统计真实可达文件（不含 .git/config、.jj/repo）。
    assert_eq!(files_scanned, 1, "files_scanned 不应包含 VCS 目录下的文件");
}

/// 显式把 `path` 指到 `.git/` 内部时尊重显式意图（depth 0 放行）。
#[test]
fn explicit_vcs_path_is_honored() {
    let fixture = Fixture::new("search-vcs-explicit");
    fixture.write(".git/config", "SECRET\n");
    let result = execute(
        &fixture.ctx(),
        "search",
        r#"{"pattern":"SECRET","literal":true,"path":".git"}"#,
    );
    let (matches, _, _) = expect_search(&result);
    assert_eq!(matches.len(), 1, "显式指向 .git 时应被搜索: {matches:?}");
}

/// 父级 `.gitignore` 在子目录搜索时仍然生效：同一棵树结果幂等。
#[test]
fn search_respects_parent_gitignore_from_subdirectory() {
    let fixture = Fixture::new("search-parent-ignore");
    // 让 fixture 成为真正的 git 仓库（require_git(true) 的前提）。
    fixture.write(".git/config", "[core]\n\trepositoryformatversion = 0\n");
    fixture.write(".gitignore", "*.log\n");
    fixture.write("sub/keep.txt", "needle\n");
    fixture.write("sub/drop.log", "needle\n");
    fixture.write("top.txt", "needle\n");

    let from_root = execute(
        &fixture.ctx(),
        "search",
        r#"{"pattern":"needle","literal":true}"#,
    );
    let (root_matches, _, _) = expect_search(&from_root);
    let from_sub = execute(
        &fixture.ctx(),
        "search",
        r#"{"pattern":"needle","literal":true,"path":"sub"}"#,
    );
    let (sub_matches, _, _) = expect_search(&from_sub);

    assert_eq!(
        root_matches.len(),
        2,
        "根搜索应命中 top.txt 与 sub/keep.txt: {root_matches:?}"
    );
    assert_eq!(
        sub_matches.len(),
        1,
        "从 sub/ 搜时父级 .gitignore 必须仍然生效: {sub_matches:?}"
    );
    // 同一棵子树上的命中集合必须一致（幂等）。
    assert!(
        sub_matches
            .iter()
            .all(|hit| root_matches.iter().any(|other| other.path == hit.path)),
        "sub 的命中必须是根命中的子集: root={root_matches:?} sub={sub_matches:?}"
    );
    assert_eq!(
        sub_matches.len(),
        root_matches
            .iter()
            .filter(|hit| hit.path.starts_with(fixture.join("sub")))
            .count(),
        "从 sub/ 搜与从根搜同一子树的命中数必须相同"
    );
}

/// `.gitignore` 只在真正的 git 仓库内生效（`require_git(true)` 的直接断言）。
#[test]
fn search_respects_gitignore_only_inside_git_repo() {
    let plain = Fixture::new("search-nogit");
    plain.write(".gitignore", "*.log\n");
    plain.write("drop.log", "needle\n");
    plain.write("keep.txt", "needle\n");
    let result = execute(
        &plain.ctx(),
        "search",
        r#"{"pattern":"needle","literal":true}"#,
    );
    let (matches, _, _) = expect_search(&result);
    assert_eq!(
        matches.len(),
        2,
        "非 git 目录里 .gitignore 不应生效: {matches:?}"
    );

    // 同一 fixture 加上 `.git/` 后，同一份 `.gitignore` 立刻生效。
    plain.write(".git/config", "[core]\n");
    let result = execute(
        &plain.ctx(),
        "search",
        r#"{"pattern":"needle","literal":true}"#,
    );
    let (matches, _, _) = expect_search(&result);
    assert_eq!(
        matches.len(),
        1,
        "git 仓库内 .gitignore 必须生效: {matches:?}"
    );
    assert!(matches[0].path.ends_with("keep.txt"));
}

/// 深度上限触发时置 `truncated`，且浅层命中仍然返回。
#[test]
fn search_depth_limit_sets_truncated() {
    let fixture = Fixture::new("search-depth");
    fixture.write("shallow.txt", "needle\n");
    // 造一条比默认上限更深的链：第 MAX_DEPTH 层目录仍有子目录。
    let mut deep = String::new();
    for _ in 0..(super::MAX_DEPTH + 4) {
        deep.push_str("d/");
    }
    fixture.write(&format!("{deep}deep.txt"), "needle\n");

    // 用更小的注入上限验证「落在上限上的目录」判定，避免造 32 层以上的慢 fixture。
    let caps = super::Caps {
        depth: 3,
        ..super::Caps::default()
    };
    let deadline = super::deadline(&caps);
    let (files, truncated) = super::collect_files(fixture.path(), None, &caps, deadline);
    assert!(truncated, "深度上限必须置 truncated");
    assert!(
        files.iter().any(|file| file.ends_with("shallow.txt")),
        "浅层文件仍应被收集: {files:?}"
    );

    // 默认 `MAX_DEPTH` 下过深的文件确实够不到——同一次调用必须显式标记截断。
    let result = execute(
        &fixture.ctx(),
        "search",
        r#"{"pattern":"needle","literal":true}"#,
    );
    let (matches, truncated, _) = expect_search(&result);
    assert_eq!(matches.len(), 1, "只有浅层命中: {matches:?}");
    assert!(matches[0].path.ends_with("shallow.txt"));
    assert!(truncated, "深度上限静默截断才算 bug（review P2-2）");
}

/// 落在深度上限上的**空**目录不算截断：没有内容可丢时不该谎报 `truncated`。
#[test]
fn empty_directory_at_depth_limit_is_not_truncation() {
    let fixture = Fixture::new("search-depth-empty");
    fixture.write("shallow.txt", "needle\n");
    // depth 3 上放一个空目录（没有第 4 层可下钻）。
    fixture.write("a/b/c/keep.txt", "unrelated\n");
    std::fs::create_dir_all(fixture.join("a/b/c/empty")).expect("create empty dir");

    let caps = super::Caps {
        depth: 4,
        ..super::Caps::default()
    };
    let deadline = super::deadline(&caps);
    let (_, truncated) = super::collect_files(fixture.path(), None, &caps, deadline);
    assert!(
        !truncated,
        "深度上限上的空目录没有内容可丢，不应报 truncated"
    );

    // 同一深度上限下，若该目录里有子项则必须报截断。
    fixture.write("a/b/c/empty/buried.txt", "needle\n");
    let deadline = super::deadline(&caps);
    let (files, truncated) = super::collect_files(fixture.path(), None, &caps, deadline);
    assert!(truncated, "有子项的目录被剪掉必须报 truncated");
    assert!(files.iter().all(|file| !file.ends_with("buried.txt")));
}

/// 遍历与扫描共用同一个 deadline：一次调用只创建一个预算。
#[test]
fn search_deadline_is_shared_between_walk_and_scan() {
    let fixture = Fixture::new("search-deadline");
    fixture.write("a.txt", "needle\n");
    fixture.write("sub/b.txt", "needle\n");

    super::reset_deadline_count();
    let result = execute(
        &fixture.ctx(),
        "search",
        r#"{"pattern":"needle","literal":true}"#,
    );
    let (matches, _, _) = expect_search(&result);
    assert_eq!(matches.len(), 2);
    assert_eq!(
        super::deadline_count(),
        1,
        "遍历与扫描必须共用同一个 deadline（各算一次就会变成 2× SEARCH_TIMEOUT）"
    );

    // 单文件搜索路径同样只创建一个预算。
    super::reset_deadline_count();
    let _ = execute(
        &fixture.ctx(),
        "search",
        r#"{"pattern":"needle","literal":true,"path":"a.txt"}"#,
    );
    assert_eq!(super::deadline_count(), 1, "单文件搜索也只用一个预算");

    // 生产默认预算就是一个 SEARCH_TIMEOUT。
    assert_eq!(
        super::Caps::default().timeout,
        super::super::SEARCH_TIMEOUT,
        "默认 Caps 的 timeout 必须等于 SEARCH_TIMEOUT"
    );
}

/// 隐藏文件（`.env` 等）仍然会被搜索：锁定「hidden(false) 语义不变」这个决定。
#[test]
fn search_hidden_files_are_searched() {
    let fixture = Fixture::new("search-hidden");
    fixture.write(".env", "SECRET=hidden-value\n");
    fixture.write("visible.txt", "nothing here\n");
    let result = execute(
        &fixture.ctx(),
        "search",
        r#"{"pattern":"SECRET","literal":true}"#,
    );
    let (matches, _, _) = expect_search(&result);
    assert_eq!(matches.len(), 1, "隐藏文件必须仍被搜索: {matches:?}");
    assert!(matches[0].path.ends_with(".env"));
}
