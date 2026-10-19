//! 结果呈现测试：模型文本、UI 摘要与落库 JSON 的截断/部分成功语义。

use std::path::PathBuf;

use super::super::test_support::Fixture;
use super::super::types::{FileChange, FileChangeKind, ReadKind, SearchMatch};
use super::super::{ToolErrorCode, ToolExecutionResult, execute};
use super::*;
use crate::app::state::ToolSummary as AppToolSummary;

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
    assert_eq!(
        summary.read_lines,
        Some(2),
        "实际读到的行数 = end - start + 1"
    );
    assert_eq!(summary.added, None);
    assert!(!format!("{summary:?}").contains("secret"));

    // 目录读取：条目数映射成 matches，且没有行区间。
    let directory = to_summary(&execute(&ctx, "read", r#"{"path":"."}"#));
    assert_eq!(directory.range, None, "目录读取没有行区间");
    assert_eq!(directory.read_lines, None, "目录读取没有「读到的行数」");
    assert_eq!(
        directory.matches, directory.total_lines,
        "目录条目数同时进 matches 与 total_lines"
    );
    assert!(directory.matches.is_some_and(|entries| entries > 0));

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

// ===== 写端 / 读端 JSON 形状往返（review §4 P3-2 的总闸门）=====

/// chat 层摘要 → app 层摘要（与事件层 `view_summary` 同口径的测试侧映射）。
///
/// `to_summary` 返回 `chat::Tools::ToolSummary`，而往返的另一端
/// （`from_arguments` / `absorb_result_json`）是 app 层类型；这里逐字段列出，
/// **不写** `..Default::default()`：任何一层新增字段都会让本测试编译失败。
fn to_app_summary(summary: &super::super::types::ToolSummary) -> AppToolSummary {
    AppToolSummary {
        path: summary.path.clone(),
        range: summary.range,
        added: summary.added,
        removed: summary.removed,
        matches: summary.matches,
        pattern: summary.pattern.clone(),
        read_lines: summary.read_lines,
        total_lines: summary.total_lines,
        truncated: summary.truncated,
    }
}

/// 一条往返用例：真实工具参数 + 真实执行结果。
struct RoundTripCase {
    /// 用例名（失败时直接指出是哪个工具/哪种形状）。
    name: &'static str,
    /// 模型给出的真实调用参数（`edit` 是 patch 文本）。
    arguments: &'static str,
    /// 真实 `ToolExecutionResult`。
    result: ToolExecutionResult,
}

/// 表驱动的写端↔读端形状往返：`to_summary` 与
/// `from_arguments(参数).absorb_result_json(&to_result_json(结果))` 必须逐字段相等。
///
/// 这是 P1-1 / P1-4 / G14 那类 bug 的**总闸门**：`to_result_json`（写端）与
/// `absorb_result_json`（读端）互为镜像，字段名/嵌套形状一变就静默漂移，编译器不管。
/// 任何一侧改名、加减嵌套层、改 `kind` 取值，这条测试都会立刻失败。
///
/// 两个例外，都写在断言里：
/// - **`pattern`**：只来自调用参数（结果体不带它）。两侧的 `pattern` 要么同为 `None`，
///   要么由同一份参数派生（`search`），因此仍要求严格相等——它证明「读端不会把
///   `pattern` 从结果体里编出来」。
/// - **`path`**：写端 `to_summary` 给的是**执行后解析出的绝对路径**，读端
///   `from_arguments` 优先保留**模型原文里的相对路径**（展示口径：重进会话要显示模型
///   当初写的那个路径）。实时路径同样先跑 `from_arguments().absorb_summary()`，
///   所以真实的两条路径在这里也是等价的——第二层断言（`live_merged == reload_merged`）
///   连 `path` 一起严格比较，正是 P2-1 / P2-8 的口径。
#[test]
fn to_result_json_round_trips_into_summary() {
    let cases = vec![
        RoundTripCase {
            name: "read:file-with-offset-and-limit",
            arguments: r#"{"path":"src/a.rs","offset":12,"limit":29}"#,
            result: ToolExecutionResult::Read {
                path: PathBuf::from("/w/src/a.rs"),
                kind: ReadKind::File,
                content: "12: x\n".into(),
                start_line: 12,
                end_line: 40,
                next_offset: Some(41),
                truncated: true,
                total_lines: 1000,
            },
        },
        RoundTripCase {
            name: "read:file-without-range-args",
            arguments: r#"{"path":"src/b.rs"}"#,
            result: ToolExecutionResult::Read {
                path: PathBuf::from("/w/src/b.rs"),
                kind: ReadKind::File,
                content: "1: x\n".into(),
                start_line: 1,
                end_line: 40,
                next_offset: None,
                truncated: false,
                total_lines: 40,
            },
        },
        RoundTripCase {
            name: "read:directory",
            arguments: r#"{"path":"src"}"#,
            result: ToolExecutionResult::Read {
                path: PathBuf::from("/w/src"),
                kind: ReadKind::Directory,
                content: "[file] a.rs\n[file] b.rs\n".into(),
                start_line: 1,
                end_line: 2,
                next_offset: None,
                truncated: false,
                total_lines: 2,
            },
        },
        RoundTripCase {
            name: "read:empty-file",
            arguments: r#"{"path":"empty.txt"}"#,
            result: ToolExecutionResult::Read {
                path: PathBuf::from("/w/empty.txt"),
                kind: ReadKind::File,
                content: String::new(),
                start_line: 0,
                end_line: 0,
                next_offset: None,
                truncated: false,
                total_lines: 0,
            },
        },
        RoundTripCase {
            name: "search:with-matches",
            arguments: r#"{"pattern":"fn render","path":"src/ui"}"#,
            result: ToolExecutionResult::Search {
                matches: vec![
                    SearchMatch {
                        path: PathBuf::from("/w/src/ui/a.rs"),
                        line: 3,
                        text: "fn render()".into(),
                        line_truncated: false,
                    },
                    SearchMatch {
                        path: PathBuf::from("/w/src/ui/b.rs"),
                        line: 9,
                        text: "fn render_helper()".into(),
                        line_truncated: true,
                    },
                ],
                truncated: false,
                files_scanned: 42,
            },
        },
        RoundTripCase {
            name: "search:no-match",
            arguments: r#"{"pattern":"没有这个"}"#,
            result: ToolExecutionResult::Search {
                matches: Vec::new(),
                truncated: false,
                files_scanned: 7,
            },
        },
        RoundTripCase {
            name: "search:truncated",
            arguments: r#"{"pattern":"needle"}"#,
            result: ToolExecutionResult::Search {
                matches: vec![SearchMatch {
                    path: PathBuf::from("/w/a.txt"),
                    line: 1,
                    text: "needle".into(),
                    line_truncated: false,
                }],
                truncated: true,
                files_scanned: 20_000,
            },
        },
        RoundTripCase {
            name: "edit:single-file",
            arguments: r#"{"patch":"*** Begin Patch\n*** Update File: a.rs\n@@\n-x\n+y\n*** End Patch"}"#,
            result: ToolExecutionResult::Edit {
                changes: vec![FileChange {
                    path: PathBuf::from("/w/a.rs"),
                    added: 1,
                    removed: 1,
                    kind: FileChangeKind::Updated,
                }],
                partial: false,
            },
        },
        RoundTripCase {
            name: "edit:multi-file",
            arguments: r#"{"patch":"*** Begin Patch\n*** Update File: a.rs\n@@\n-x\n+y\n*** Update File: b.rs\n@@\n-p\n+q\n*** End Patch"}"#,
            result: ToolExecutionResult::Edit {
                changes: vec![
                    FileChange {
                        path: PathBuf::from("/w/a.rs"),
                        added: 3,
                        removed: 1,
                        kind: FileChangeKind::Updated,
                    },
                    FileChange {
                        path: PathBuf::from("/w/b.rs"),
                        added: 0,
                        removed: 0,
                        kind: FileChangeKind::Moved {
                            to: PathBuf::from("/w/c.rs"),
                        },
                    },
                ],
                partial: true,
            },
        },
        RoundTripCase {
            name: "edit:partial-single",
            arguments: r#"{"patch":"*** Begin Patch\n*** Add File: new.rs\n+x\n*** End Patch"}"#,
            result: ToolExecutionResult::Edit {
                changes: vec![FileChange {
                    path: PathBuf::from("/w/new.rs"),
                    added: 1,
                    removed: 0,
                    kind: FileChangeKind::Added,
                }],
                partial: true,
            },
        },
        RoundTripCase {
            name: "write:created",
            arguments: r#"{"path":"docs/new.md","content":"a\nb\n"}"#,
            result: ToolExecutionResult::Write {
                path: PathBuf::from("/w/docs/new.md"),
                created: true,
                added: 2,
                removed: 0,
            },
        },
        RoundTripCase {
            name: "write:overwritten",
            arguments: r#"{"path":"docs/old.md","content":"x\n"}"#,
            result: ToolExecutionResult::Write {
                path: PathBuf::from("/w/docs/old.md"),
                created: false,
                added: 18,
                removed: 4,
            },
        },
        RoundTripCase {
            name: "failed:with-changes",
            arguments: r#"{"patch":"*** Begin Patch\n*** Update File: a.rs\n@@\n-x\n+y\n*** End Patch"}"#,
            result: ToolExecutionResult::Failed {
                code: ToolErrorCode::PatchContextMismatch,
                message: "hunk lines do not match".into(),
                changes: vec![FileChange {
                    path: PathBuf::from("/w/a.rs"),
                    added: 2,
                    removed: 1,
                    kind: FileChangeKind::Updated,
                }],
            },
        },
        RoundTripCase {
            name: "failed:single-change-deleted",
            arguments: r#"{"patch":"*** Begin Patch\n*** Delete File: gone.rs\n*** End Patch"}"#,
            result: ToolExecutionResult::Failed {
                code: ToolErrorCode::NoChanges,
                message: "no changes".into(),
                changes: vec![FileChange {
                    path: PathBuf::from("/w/gone.rs"),
                    added: 0,
                    removed: 7,
                    kind: FileChangeKind::Deleted,
                }],
            },
        },
        RoundTripCase {
            name: "failed:without-changes",
            arguments: r#"{"patch":"*** Begin Patch\n*** Update File: a.rs\n@@\n-x\n+y\n*** End Patch"}"#,
            result: ToolExecutionResult::Failed {
                code: ToolErrorCode::PatchInvalid,
                message: "patch must end with '*** End Patch'".into(),
                changes: Vec::new(),
            },
        },
    ];

    for case in cases {
        let live = to_app_summary(&to_summary(&case.result));
        let json = to_result_json(&case.result);
        assert!(
            json.is_some(),
            "{}: 结果必须能落库（None 会让重载路径完全看不到结果）",
            case.name
        );
        let from_args = AppToolSummary::from_arguments(case.arguments);
        let reloaded = from_args.clone().absorb_result_json(json.as_deref());
        // 第一层：写端摘要与读端摘要除 `path` / `pattern` 外必须**逐字段相等**——这就是
        // 「JSON 形状互逆」的断言；任何字段被写端/读端单方面改名都会在这里失败。
        let mut live_result_only = live.clone();
        live_result_only.path = reloaded.path.clone();
        live_result_only.pattern = reloaded.pattern.clone();
        assert_eq!(
            live_result_only, reloaded,
            "{}: 写端 to_result_json 与读端 absorb_result_json 的形状必须互逆",
            case.name
        );
        // `pattern` 只来自参数：`to_summary`（纯结果侧）必须为 None，读端只能由参数
        // 提供同一个值——读端不得从结果体里编造 pattern。
        assert_eq!(
            live.pattern, None,
            "{}: 结果侧摘要不得携带 pattern（它只来自参数）",
            case.name
        );
        assert_eq!(
            reloaded.pattern, from_args.pattern,
            "{}: pattern 只能来自参数（两侧口径必须一致）",
            case.name
        );
        // 第二层：真实的两条路径（实时 `absorb_summary` / 重载 `absorb_result_json`）
        // 连 `path` 一起严格相等——P2-1 / P2-8 的口径。
        let live_merged = from_args.clone().absorb_summary(&live);
        assert_eq!(
            live_merged, reloaded,
            "{}: 实时结算摘要与重进会话摘要必须逐字段一致（含 path）",
            case.name
        );
    }
}
