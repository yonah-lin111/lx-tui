//! 结果呈现测试：模型文本、UI 摘要与落库 JSON 的截断/部分成功语义。

use std::path::PathBuf;

use super::super::test_support::Fixture;
use super::super::types::{FileChange, FileChangeKind, ReadKind, SearchMatch};
use super::super::{ToolErrorCode, ToolExecutionResult, execute};
use super::*;

/// 读取结果的文本：范围、总行数与 `next_offset` 都在文本里。
#[test]
fn read_text_states_range_and_next_offset() {
    let result = ToolExecutionResult::Read {
        path: PathBuf::from("/tmp/a.txt"),
        kind: ReadKind::File,
        content: "1: a\n2: b\n".into(),
        start_line: 1,
        end_line: 2,
        next_offset: Some(3),
        truncated: true,
        total_lines: 9,
    };
    let text = format_for_model(&result);
    assert!(text.contains("lines 1-2 of 9 total lines"), "text: {text}");
    assert!(text.contains("truncated: true"));
    assert!(text.contains("next_offset: 3"));
    assert!(text.contains("1: a"));
}

/// 空文件与目录列表的文本形状。
#[test]
fn read_text_distinguishes_empty_file_and_directory() {
    let empty = ToolExecutionResult::Read {
        path: PathBuf::from("/tmp/e.txt"),
        kind: ReadKind::File,
        content: String::new(),
        start_line: 0,
        end_line: 0,
        next_offset: None,
        truncated: false,
        total_lines: 0,
    };
    assert!(format_for_model(&empty).contains("empty file"));

    let dir = ToolExecutionResult::Read {
        path: PathBuf::from("/tmp/dir"),
        kind: ReadKind::Directory,
        content: "[file] a.txt\n".into(),
        start_line: 1,
        end_line: 1,
        next_offset: None,
        truncated: false,
        total_lines: 1,
    };
    let text = format_for_model(&dir);
    assert!(text.contains("Listed 1 entries"));
    assert!(text.contains("[file] a.txt"));
}

/// 搜索文本含命中、计数与截断标志；无命中时明确说明。
#[test]
fn search_text_reports_matches_and_truncation() {
    let result = ToolExecutionResult::Search {
        matches: vec![SearchMatch {
            path: PathBuf::from("/tmp/a.txt"),
            line: 3,
            text: "needle".into(),
            line_truncated: false,
        }],
        truncated: true,
        files_scanned: 12,
    };
    let text = format_for_model(&result);
    assert!(text.contains("Found 1 match"));
    assert!(text.contains("/tmp/a.txt:3: needle"));
    assert!(text.contains("truncated: true"));

    let empty = ToolExecutionResult::Search {
        matches: Vec::new(),
        truncated: false,
        files_scanned: 3,
    };
    assert!(format_for_model(&empty).contains("no matches"));
}

/// `edit` 文本逐文件给出动作与增删；`partial` 显式呈现。
#[test]
fn edit_text_lists_each_change() {
    let result = ToolExecutionResult::Edit {
        changes: vec![
            FileChange {
                path: PathBuf::from("/tmp/a.txt"),
                added: 2,
                removed: 1,
                kind: FileChangeKind::Updated,
            },
            FileChange {
                path: PathBuf::from("/tmp/b.txt"),
                added: 0,
                removed: 0,
                kind: FileChangeKind::Moved {
                    to: PathBuf::from("/tmp/c.txt"),
                },
            },
        ],
        partial: true,
    };
    let text = format_for_model(&result);
    assert!(text.contains("updated /tmp/a.txt (+2 -1)"));
    assert!(text.contains("moved /tmp/b.txt -> /tmp/c.txt"));
    assert!(text.contains("partial: true"));
}

/// 失败文本含错误码与已提交变更，并明确不是回滚。
#[test]
fn failure_text_reports_partial_application() {
    let result = ToolExecutionResult::Failed {
        code: ToolErrorCode::PatchContextMismatch,
        message: "context mismatch".into(),
        changes: vec![FileChange {
            path: PathBuf::from("/tmp/a.txt"),
            added: 1,
            removed: 1,
            kind: FileChangeKind::Updated,
        }],
    };
    let text = format_for_model(&result);
    assert!(text.contains("patch_context_mismatch"));
    assert!(text.contains("partially applied"));
    assert!(text.contains("not a rollback"));
    assert!(text.contains("updated /tmp/a.txt"));

    let plain = ToolExecutionResult::Failed {
        code: ToolErrorCode::NotFound,
        message: "missing".into(),
        changes: Vec::new(),
    };
    assert!(format_for_model(&plain).contains("no files were changed"));
}

/// 摘要：read 只有范围、search 只有命中数、write 只有计数，均不含内容。
#[test]
fn summaries_hold_only_metadata() {
    let fixture = Fixture::new("output-summary");
    fixture.write("a.txt", "secret\nline\n");
    let ctx = fixture.ctx();

    let read = execute(&ctx, "read", r#"{"path":"a.txt"}"#);
    let summary = to_summary(&read);
    assert_eq!(summary.range, Some((1, 2)));
    assert_eq!(summary.added, None);
    assert!(!format!("{summary:?}").contains("secret"));

    let write = execute(&ctx, "write", r#"{"path":"b.txt","content":"x\ny\n"}"#);
    let summary = to_summary(&write);
    assert_eq!(summary.added, Some(2));
    assert_eq!(summary.removed, Some(0));
    assert!(summary.path.is_some());

    let search = execute(&ctx, "search", r#"{"pattern":"secret"}"#);
    let summary = to_summary(&search);
    assert_eq!(summary.matches, Some(1));
    assert!(!format!("{summary:?}").contains("secret"));
}

/// 多文件 edit 的摘要不挑单一路径，只给累计计数。
#[test]
fn multi_file_summary_has_no_single_path() {
    let result = ToolExecutionResult::Edit {
        changes: vec![
            FileChange {
                path: PathBuf::from("/tmp/a.txt"),
                added: 1,
                removed: 0,
                kind: FileChangeKind::Updated,
            },
            FileChange {
                path: PathBuf::from("/tmp/b.txt"),
                added: 2,
                removed: 2,
                kind: FileChangeKind::Updated,
            },
        ],
        partial: false,
    };
    let summary = to_summary(&result);
    assert_eq!(summary.path, None);
    assert_eq!(summary.added, Some(3));
    assert_eq!(summary.removed, Some(2));
}

/// 落库 JSON：read / search / edit 三个变体结构完整。
#[test]
fn result_json_covers_every_variant() {
    let read = ToolExecutionResult::Read {
        path: PathBuf::from("/tmp/a.txt"),
        kind: ReadKind::File,
        content: "1: a\n".into(),
        start_line: 1,
        end_line: 1,
        next_offset: None,
        truncated: false,
        total_lines: 1,
    };
    let value: serde_json::Value =
        serde_json::from_str(&to_result_json(&read).expect("json")).expect("parse");
    assert_eq!(value["tool"], "read");
    assert_eq!(value["kind"], "file");
    assert_eq!(value["total_lines"], 1);

    let search = ToolExecutionResult::Search {
        matches: vec![SearchMatch {
            path: PathBuf::from("/tmp/a.txt"),
            line: 1,
            text: "a".into(),
            line_truncated: false,
        }],
        truncated: false,
        files_scanned: 1,
    };
    let value: serde_json::Value =
        serde_json::from_str(&to_result_json(&search).expect("json")).expect("parse");
    assert_eq!(value["tool"], "search");
    assert_eq!(value["matches"][0]["line"], 1);

    let edit = ToolExecutionResult::Edit {
        changes: vec![FileChange {
            path: PathBuf::from("/tmp/a.txt"),
            added: 1,
            removed: 1,
            kind: FileChangeKind::Moved {
                to: PathBuf::from("/tmp/b.txt"),
            },
        }],
        partial: false,
    };
    let value: serde_json::Value =
        serde_json::from_str(&to_result_json(&edit).expect("json")).expect("parse");
    assert_eq!(value["tool"], "edit");
    assert_eq!(value["changes"][0]["kind"], "moved");
    assert_eq!(value["changes"][0]["to"], "/tmp/b.txt");

    let failed = ToolExecutionResult::Failed {
        code: ToolErrorCode::NoChanges,
        message: "no changes".into(),
        changes: Vec::new(),
    };
    let value: serde_json::Value =
        serde_json::from_str(&to_result_json(&failed).expect("json")).expect("parse");
    assert_eq!(value["tool"], "error");
    assert_eq!(value["code"], "no_changes");
    assert_eq!(value["changes"], serde_json::json!([]));
}

/// 超大 read 结果的 JSON 被截断并显式标记 `truncated: true`。
#[test]
fn oversized_result_json_is_truncated_with_flag() {
    let content = "x".repeat(super::MAX_TOOL_RESULT_BYTES + 1024);
    let result = ToolExecutionResult::Read {
        path: PathBuf::from("/tmp/big.txt"),
        kind: ReadKind::File,
        content,
        start_line: 1,
        end_line: 1,
        next_offset: Some(2),
        truncated: true,
        total_lines: 10,
    };
    let json = to_result_json(&result).expect("json");
    assert!(json.len() <= super::MAX_TOOL_RESULT_BYTES);
    let value: serde_json::Value = serde_json::from_str(&json).expect("parse");
    assert_eq!(value["truncated"], true);
    assert_eq!(value["next_offset"], 2);
}

/// 搜索命中过多时 JSON 只保留部分条目但仍带截断标志。
#[test]
fn many_matches_shrink_but_keep_flag() {
    // 命中条目的文本足够大，使整份 JSON 超过落库上限。
    let matches: Vec<SearchMatch> = (0..6000)
        .map(|index| SearchMatch {
            path: PathBuf::from(format!("/tmp/file{index}.txt")),
            line: index + 1,
            text: "y".repeat(300),
            line_truncated: false,
        })
        .collect();
    let result = ToolExecutionResult::Search {
        matches,
        truncated: false,
        files_scanned: 6000,
    };
    let json = to_result_json(&result).expect("json");
    assert!(
        json.len() <= super::MAX_TOOL_RESULT_BYTES,
        "json length {} exceeds limit",
        json.len()
    );
    let value: serde_json::Value = serde_json::from_str(&json).expect("parse");
    assert_eq!(value["truncated"], true);
}
