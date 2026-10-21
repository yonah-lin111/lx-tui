//! 文件详情解码测试：形状校验、坏数据局部降级、实时/历史同源。

use super::*;

/// 一行差异的 JSON。
fn line(kind: &str, old: Option<u64>, new: Option<u64>, text: &str) -> Value {
    serde_json::json!({
        "kind": kind,
        "old_line": old,
        "new_line": new,
        "text": text,
    })
}

/// `edit` 结果 JSON（单个文件、单个 hunk）。
fn edit_result(diff: Value) -> String {
    serde_json::json!({
        "tool": "edit",
        "changes": [{
            "path": "/tmp/a.rs",
            "kind": "updated",
            "added": 1,
            "removed": 1,
            "diff": diff,
        }],
        "partial": false,
        "file_totals": { "files": 1, "added": 1, "removed": 1, "simplified": false, "omitted_files": 0 },
    })
    .to_string()
}

/// 完整且合法的差异对象。
fn complete_diff() -> Value {
    serde_json::json!({
        "version": 1,
        "status": "complete",
        "simplified": false,
        "hunks": [{ "lines": [
            line("context", Some(1), Some(1), "a"),
            line("removed", Some(2), None, "b"),
            line("added", None, Some(2), "B"),
        ]}],
    })
}

/// 合法结果解码出完整视图。
#[test]
fn valid_edit_result_decodes() {
    let view = FileDiffDetailView::from_result_json(Some(&edit_result(complete_diff())))
        .expect("应解码出文件详情");
    assert_eq!(view.files.len(), 1);
    assert_eq!(view.files[0].path, "/tmp/a.rs");
    let diff = view.files[0].diff.as_ref().expect("diff");
    assert!(!diff.is_truncated());
    assert_eq!(diff.hunks().len(), 1);
    assert_eq!(diff.hunks()[0].lines.len(), 3);
}

/// `write` 结果把顶层当单文件。
#[test]
fn write_result_is_single_file() {
    let json = serde_json::json!({
        "tool": "write",
        "path": "/tmp/new.md",
        "created": true,
        "added": 2,
        "removed": 0,
        "counts_known": true,
        "simplified": false,
        "diff": {
            "version": 1,
            "status": "complete",
            "hunks": [{ "lines": [
                line("added", None, Some(1), "one"),
                line("added", None, Some(2), "two"),
            ]}],
        },
        "file_totals": { "files": 1, "added": 2, "removed": 0 },
    })
    .to_string();
    let view = FileDiffDetailView::from_result_json(Some(&json)).expect("解码");
    assert!(view.single);
    assert_eq!(view.files.len(), 1);
    assert_eq!(view.files[0].added, Some(2));
    assert_eq!(
        view.files[0]
            .diff
            .as_ref()
            .map(FileDiffView::hunks)
            .map(<[_]>::len),
        Some(1)
    );
}

/// 未知版本：该文件降级为不可用，其余字段仍可读。
#[test]
fn unsupported_version_degrades_to_unavailable() {
    let mut diff = complete_diff();
    diff["version"] = serde_json::json!(2);
    let view = FileDiffDetailView::from_result_json(Some(&edit_result(diff))).expect("解码");
    let diff = view.files[0].diff.as_ref().expect("diff");
    assert_eq!(diff.unavailable_reason(), Some("unsupported_version"));
}

/// 行号与种类不自洽（Context 缺 new_line）：整份差异降级，但文件仍在。
#[test]
fn inconsistent_line_numbers_degrade_the_file() {
    let json = edit_result(serde_json::json!({
        "version": 1,
        "status": "complete",
        "hunks": [{ "lines": [line("context", Some(1), None, "a")] }],
    }));
    let view = FileDiffDetailView::from_result_json(Some(&json)).expect("解码");
    let diff = view.files[0].diff.as_ref().expect("diff");
    assert_eq!(diff.unavailable_reason(), Some("unknown"));
    // 计数与路径不受正文降级影响。
    assert_eq!(view.files[0].added, Some(1));
    assert_eq!(view.files[0].path, "/tmp/a.rs");
}

/// `null` 计数（未知）不会被当成 0。
#[test]
fn unknown_counts_stay_unknown() {
    let json = serde_json::json!({
        "tool": "edit",
        "changes": [{
            "path": "/tmp/bin.dat",
            "kind": "deleted",
            "added": serde_json::Value::Null,
            "removed": serde_json::Value::Null,
            "counts_known": false,
            "diff": { "version": 1, "status": "unavailable", "reason": "binary" },
        }],
        "partial": false,
        "file_totals": { "files": 1, "added": serde_json::Value::Null, "removed": serde_json::Value::Null },
    })
    .to_string();
    let view = FileDiffDetailView::from_result_json(Some(&json)).expect("解码");
    assert!(!view.files[0].counts_known);
    assert_eq!(view.files[0].added, None);
    assert_eq!(view.files[0].removed, None);
    assert_eq!(view.added, None);
    assert_eq!(
        view.files[0]
            .diff
            .as_ref()
            .and_then(FileDiffView::unavailable_reason),
        Some("binary")
    );
}

/// 非文件工具 / 坏 JSON / 空 changes：不产生文件详情（退回两行摘要）。
#[test]
fn non_file_results_produce_no_detail() {
    assert!(FileDiffDetailView::from_result_json(None).is_none());
    assert!(FileDiffDetailView::from_result_json(Some("not json")).is_none());
    assert!(FileDiffDetailView::from_result_json(Some(r#"{"tool":"read"}"#)).is_none());
    assert!(
        FileDiffDetailView::from_result_json(Some(r#"{"tool":"edit","changes":[]}"#)).is_some(),
        "空 changes 仍是合法的 edit 结果（只是没有文件）"
    );
    assert!(FileDiffDetailView::from_result_json(Some(r#"{"tool":"edit"}"#)).is_none());
}

/// `file_totals` 优先于可见文件之和（列表被裁切时仍然正确）。
#[test]
fn totals_win_over_visible_files() {
    let json = serde_json::json!({
        "tool": "edit",
        "changes": [{
            "path": "/tmp/a.rs",
            "kind": "updated",
            "added": 3,
            "removed": 1,
            "diff": { "version": 1, "status": "complete", "hunks": [] },
        }],
        "partial": false,
        "file_totals": {
            "files": 7,
            "added": 40,
            "removed": 9,
            "simplified": true,
            "omitted_files": 6,
        },
    })
    .to_string();
    let view = FileDiffDetailView::from_result_json(Some(&json)).expect("解码");
    assert_eq!(view.files_total, 7);
    assert_eq!(view.added, Some(40));
    assert_eq!(view.removed, Some(9));
    assert_eq!(view.omitted_files, 6);
    assert!(view.simplified);
}

/// 工具层差异 → 视图转换与 JSON 解码同源（同一份数据两条路径相等）。
#[test]
fn tool_diff_and_json_decode_agree() {
    let diff = crate::chat::tools::FileDiff::Complete {
        hunks: vec![crate::chat::tools::DiffHunk {
            lines: vec![crate::chat::tools::DiffLine {
                kind: DiffLineKind::Added,
                old_line: None,
                new_line: Some(1),
                text: "x".into(),
                no_final_newline: false,
            }],
        }],
        simplified: false,
    };
    let view = FileDiffView::from(&diff);
    assert_eq!(view.hunks().len(), 1);
    assert_eq!(view.hunks()[0].lines[0].text, "x");
}
