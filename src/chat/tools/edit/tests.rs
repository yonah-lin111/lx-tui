//! `edit` 工具测试：Add/Update/Delete/Move、上下文唯一匹配、部分成功、
//! 编码保留与原子替换。

use std::path::Path;

use super::super::test_support::*;
use super::super::types::FileChangeKind;
use super::super::{ToolErrorCode, ToolExecutionResult, execute};

/// 构造 `edit` 参数 JSON（补丁作为字符串字段）。
fn edit_args(patch: &str) -> String {
    serde_json::json!({ "patch": patch }).to_string()
}

/// 包一层 Begin/End。
fn wrapped(body: &str) -> String {
    format!("*** Begin Patch\n{body}\n*** End Patch\n")
}

/// Add File：新建文件，返回 Added 变更与行数。
#[test]
fn add_file_creates_file() {
    let fixture = Fixture::new("edit-add");
    let patch = wrapped("*** Add File: sub/new.txt\n+one\n+two");
    let result = execute(&fixture.ctx(), "edit", &edit_args(&patch));
    let changes = expect_changes(&result);
    assert_eq!(changes.len(), 1);
    assert_eq!(changes[0].kind, FileChangeKind::Added);
    assert_eq!(changes[0].added, 2);
    assert_eq!(fixture.read("sub/new.txt"), "one\ntwo\n");
}

/// Add File 目标已存在时失败且原文件不变。
#[test]
fn add_file_rejects_existing_file() {
    let fixture = Fixture::new("edit-add-exists");
    fixture.write("a.txt", "keep\n");
    let before = fixture.read_bytes("a.txt");
    let patch = wrapped("*** Add File: a.txt\n+replaced");
    let result = execute(&fixture.ctx(), "edit", &edit_args(&patch));
    assert_eq!(expect_failure(&result), ToolErrorCode::PatchInvalid);
    assert_eq!(fixture.read_bytes("a.txt"), before);
}

/// Update File：精确替换，返回 Updated 与增删计数。
#[test]
fn update_file_replaces_lines() {
    let fixture = Fixture::new("edit-update");
    fixture.write("a.txt", "alpha\nconst TIMEOUT: u64 = 30;\nomega\n");
    let patch =
        wrapped("*** Update File: a.txt\n@@\n-const TIMEOUT: u64 = 30;\n+const TIMEOUT: u64 = 45;");
    let result = execute(&fixture.ctx(), "edit", &edit_args(&patch));
    let changes = expect_changes(&result);
    assert_eq!(changes.len(), 1);
    assert_eq!(changes[0].kind, FileChangeKind::Updated);
    assert_eq!((changes[0].added, changes[0].removed), (1, 1));
    assert_eq!(
        fixture.read("a.txt"),
        "alpha\nconst TIMEOUT: u64 = 45;\nomega\n"
    );
}

/// Update 带 `@@` 定位行：定位行也必须在文件中唯一。
#[test]
fn update_with_context_anchor() {
    let fixture = Fixture::new("edit-context");
    fixture.write("a.py", "def f():\n    pass\n\ndef g():\n    pass\n");
    let patch = wrapped("*** Update File: a.py\n@@ def g():\n-    pass\n+    return 123");
    let result = execute(&fixture.ctx(), "edit", &edit_args(&patch));
    expect_changes(&result);
    assert_eq!(
        fixture.read("a.py"),
        "def f():\n    pass\n\ndef g():\n    return 123\n"
    );
}

/// 上下文失配：失败且原文件**字节不变**。
#[test]
fn context_mismatch_leaves_file_untouched() {
    let fixture = Fixture::new("edit-mismatch");
    fixture.write("a.txt", "alpha\nbeta\n");
    let before = fixture.read_bytes("a.txt");
    let patch = wrapped("*** Update File: a.txt\n@@\n-gamma\n+delta");
    let result = execute(&fixture.ctx(), "edit", &edit_args(&patch));
    assert_eq!(expect_failure(&result), ToolErrorCode::PatchContextMismatch);
    assert_eq!(fixture.read_bytes("a.txt"), before);
}

/// 上下文不唯一（多处匹配）同样失败，绝不猜测替换。
#[test]
fn ambiguous_context_is_rejected() {
    let fixture = Fixture::new("edit-ambiguous");
    fixture.write("a.txt", "same\nsame\n");
    let before = fixture.read_bytes("a.txt");
    let patch = wrapped("*** Update File: a.txt\n@@\n-same\n+other");
    let result = execute(&fixture.ctx(), "edit", &edit_args(&patch));
    assert_eq!(expect_failure(&result), ToolErrorCode::PatchContextMismatch);
    assert_eq!(fixture.read_bytes("a.txt"), before);

    // `@@` 定位行重复也失败。
    let patch = wrapped("*** Update File: a.txt\n@@ same\n-same\n+other");
    let result = execute(&fixture.ctx(), "edit", &edit_args(&patch));
    assert_eq!(expect_failure(&result), ToolErrorCode::PatchContextMismatch);
    assert_eq!(fixture.read_bytes("a.txt"), before);
}

/// 同一块补充更多上下文行即可唯一匹配。
#[test]
fn extra_context_lines_disambiguate() {
    let fixture = Fixture::new("edit-disambiguate");
    fixture.write("a.txt", "first\nsame\nsecond\nsame\n");
    let patch = wrapped("*** Update File: a.txt\n@@\n first\n-same\n+other");
    let result = execute(&fixture.ctx(), "edit", &edit_args(&patch));
    expect_changes(&result);
    assert_eq!(fixture.read("a.txt"), "first\nother\nsecond\nsame\n");
}

/// Delete File：删除文件并返回 Deleted。
#[test]
fn delete_file_removes_it() {
    let fixture = Fixture::new("edit-delete");
    fixture.write("obsolete.txt", "gone\n");
    let patch = wrapped("*** Delete File: obsolete.txt");
    let result = execute(&fixture.ctx(), "edit", &edit_args(&patch));
    let changes = expect_changes(&result);
    assert_eq!(changes[0].kind, FileChangeKind::Deleted);
    assert!(!fixture.exists("obsolete.txt"));
}

/// Delete File 目标不存在时报 NotFound。
#[test]
fn delete_missing_file_is_not_found() {
    let fixture = Fixture::new("edit-delete-missing");
    let patch = wrapped("*** Delete File: nope.txt");
    let result = execute(&fixture.ctx(), "edit", &edit_args(&patch));
    assert_eq!(expect_failure(&result), ToolErrorCode::NotFound);
}

/// Move to：改名并创建父目录，内容按补丁更新。
#[test]
fn move_to_renames_and_updates() {
    let fixture = Fixture::new("edit-move");
    fixture.write("old.txt", "before\nkeep\n");
    let patch =
        wrapped("*** Update File: old.txt\n*** Move to: nested/new.txt\n@@\n-before\n+after");
    let result = execute(&fixture.ctx(), "edit", &edit_args(&patch));
    let changes = expect_changes(&result);
    assert_eq!(changes.len(), 1);
    match &changes[0].kind {
        FileChangeKind::Moved { to } => {
            assert!(to.ends_with(Path::new("nested").join("new.txt")));
        }
        other => panic!("expected moved, got {other:?}"),
    }
    assert!(!fixture.exists("old.txt"));
    assert_eq!(fixture.read("nested/new.txt"), "after\nkeep\n");
}

/// `*** Move to:` 出现在 `@@` 之后 → PatchInvalid，且文件不变。
#[test]
fn move_after_context_is_invalid() {
    let fixture = Fixture::new("edit-move-order");
    fixture.write("a.txt", "before\n");
    let before = fixture.read_bytes("a.txt");
    let patch = wrapped("*** Update File: a.txt\n@@\n-before\n+after\n*** Move to: b.txt");
    let result = execute(&fixture.ctx(), "edit", &edit_args(&patch));
    assert_eq!(expect_failure(&result), ToolErrorCode::PatchInvalid);
    assert_eq!(fixture.read_bytes("a.txt"), before);
    assert!(!fixture.exists("b.txt"));
}

/// 多文件补丁：前面的 hunk 已提交，后面的失败时如实报告部分成功。
#[test]
fn multi_file_patch_reports_partial_success() {
    let fixture = Fixture::new("edit-partial");
    fixture.write("first.txt", "one\n");
    fixture.write("second.txt", "keep\n");
    let second_before = fixture.read_bytes("second.txt");
    let patch = wrapped(
        "*** Update File: first.txt\n@@\n-one\n+ONE\n\
         *** Update File: second.txt\n@@\n-not-there\n+nope",
    );
    let result = execute(&fixture.ctx(), "edit", &edit_args(&patch));
    let (code, committed) = expect_partial_failure(&result);
    assert_eq!(code, ToolErrorCode::PatchContextMismatch);
    assert_eq!(committed, 1, "第一个文件应已提交");
    assert_eq!(fixture.read("first.txt"), "ONE\n");
    assert_eq!(fixture.read_bytes("second.txt"), second_before);
    let text = super::super::format_for_model(&result);
    assert!(text.contains("partially applied"), "text: {text}");
}

/// 格式校验先于执行：第二个 hunk 格式非法时，第一个 hunk 不会被写入。
#[test]
fn format_validation_happens_before_any_write() {
    let fixture = Fixture::new("edit-prevalidate");
    fixture.write("first.txt", "one\n");
    let before = fixture.read_bytes("first.txt");
    let patch = wrapped(
        "*** Update File: first.txt\n@@\n-one\n+ONE\n*** Add File: second.txt\nbad line without plus",
    );
    let result = execute(&fixture.ctx(), "edit", &edit_args(&patch));
    assert_eq!(expect_failure(&result), ToolErrorCode::PatchInvalid);
    assert_eq!(fixture.read_bytes("first.txt"), before);
    assert!(!fixture.exists("second.txt"));
}

/// 路径校验先于执行：第二个 hunk 目标不存在时，第一个 hunk 不会被写入。
#[test]
fn path_validation_happens_before_any_write() {
    let fixture = Fixture::new("edit-prevalidate-path");
    fixture.write("first.txt", "one\n");
    let before = fixture.read_bytes("first.txt");
    let patch = wrapped(
        "*** Update File: first.txt\n@@\n-one\n+ONE\n*** Update File: missing.txt\n@@\n-x\n+y",
    );
    let result = execute(&fixture.ctx(), "edit", &edit_args(&patch));
    assert_eq!(expect_failure(&result), ToolErrorCode::NotFound);
    assert_eq!(fixture.read_bytes("first.txt"), before);
}

/// 无实际变更（hunk 应用后内容相同）→ NoChanges。
#[test]
fn no_content_change_is_reported() {
    let fixture = Fixture::new("edit-nochange");
    fixture.write("a.txt", "same\n");
    let before = fixture.read_bytes("a.txt");
    let patch = wrapped("*** Update File: a.txt\n@@\n-same\n+same");
    let result = execute(&fixture.ctx(), "edit", &edit_args(&patch));
    assert_eq!(expect_failure(&result), ToolErrorCode::NoChanges);
    assert_eq!(fixture.read_bytes("a.txt"), before);
}

/// CRLF 与 BOM 在 Update 后保留。
#[test]
fn crlf_and_bom_are_preserved() {
    let fixture = Fixture::new("edit-crlf");
    fixture.write_bytes("a.txt", b"\xEF\xBB\xBFone\r\ntwo\r\n");
    let patch = wrapped("*** Update File: a.txt\n@@\n-two\n+THREE");
    let result = execute(&fixture.ctx(), "edit", &edit_args(&patch));
    expect_changes(&result);
    let bytes = fixture.read_bytes("a.txt");
    assert!(bytes.starts_with(b"\xEF\xBB\xBF"), "BOM 应保留");
    assert_eq!(bytes, b"\xEF\xBB\xBFone\r\nTHREE\r\n".to_vec());
}

/// 原子替换后不留下临时文件，内容正确。
#[test]
fn atomic_write_leaves_no_temp_files() {
    let fixture = Fixture::new("edit-atomic");
    fixture.write("a.txt", "one\n");
    let patch = wrapped("*** Update File: a.txt\n@@\n-one\n+two");
    let result = execute(&fixture.ctx(), "edit", &edit_args(&patch));
    expect_changes(&result);
    assert_eq!(fixture.read("a.txt"), "two\n");
    let mut names: Vec<String> = std::fs::read_dir(fixture.path())
        .expect("read dir")
        .map(|entry| {
            entry
                .expect("entry")
                .file_name()
                .to_string_lossy()
                .into_owned()
        })
        .collect();
    names.sort();
    assert_eq!(names, vec!["a.txt"]);
}

/// 空补丁 / 缺标记 / 未知标记都报 PatchInvalid。
#[test]
fn invalid_patch_shapes_are_rejected() {
    let fixture = Fixture::new("edit-invalid");
    fixture.write("a.txt", "one\n");
    let cases = [
        String::new(),
        "*** Update File: a.txt\n@@\n-one\n+two".to_string(),
        "*** Begin Patch\n*** Update File: a.txt\n@@\n-one\n+two".to_string(),
        wrapped("*** Unknown Marker: a.txt"),
        wrapped("*** Update File: a.txt"),
        wrapped("*** Add File: a.txt"),
    ];
    for patch in cases {
        let result = execute(&fixture.ctx(), "edit", &edit_args(&patch));
        assert_eq!(
            expect_failure(&result),
            ToolErrorCode::PatchInvalid,
            "patch should be invalid: {patch}"
        );
        assert_eq!(fixture.read("a.txt"), "one\n");
    }
}

/// 同一补丁重复目标被拒绝（先全量校验再执行）。
#[test]
fn duplicate_targets_are_rejected() {
    let fixture = Fixture::new("edit-duplicate");
    fixture.write("a.txt", "one\n");
    let before = fixture.read_bytes("a.txt");
    let patch =
        wrapped("*** Update File: a.txt\n@@\n-one\n+TWO\n*** Update File: a.txt\n@@\n-TWO\n+THREE");
    let result = execute(&fixture.ctx(), "edit", &edit_args(&patch));
    assert_eq!(expect_failure(&result), ToolErrorCode::PatchInvalid);
    assert_eq!(fixture.read_bytes("a.txt"), before);
}

/// 一次补丁完成 Add + Update + Delete + Move（四类动作组合）。
#[test]
fn combined_patch_handles_all_four_actions() {
    let fixture = Fixture::new("edit-combined");
    fixture.write("keep.txt", "line\n");
    fixture.write("gone.txt", "bye\n");
    fixture.write("moved.txt", "old\n");
    let patch = wrapped(
        "*** Update File: keep.txt\n@@\n-line\n+LINE\n\
         *** Move to: moved.txt\n\
         *** Update File: moved.txt\n@@\n-old\n+new\n\
         *** Delete File: gone.txt",
    );
    // moved.txt 同时出现在 Move to 与 Update File：重复目标必须被拒。
    let result = execute(&fixture.ctx(), "edit", &edit_args(&patch));
    assert_eq!(expect_failure(&result), ToolErrorCode::PatchInvalid);

    let patch = wrapped(
        "*** Add File: added.txt\n+a\n\
         *** Update File: keep.txt\n@@\n-line\n+LINE\n\
         *** Delete File: gone.txt\n\
         *** Update File: moved.txt\n*** Move to: dest/moved.txt\n@@\n-old\n+new",
    );
    let result = execute(&fixture.ctx(), "edit", &edit_args(&patch));
    match &result {
        ToolExecutionResult::Edit { changes, partial } => {
            assert_eq!(changes.len(), 4);
            assert!(!partial, "全部 hunk 都有实际变更");
        }
        other => panic!("expected edit success, got {other:?}"),
    }
    assert_eq!(fixture.read("added.txt"), "a\n");
    assert_eq!(fixture.read("keep.txt"), "LINE\n");
    assert!(!fixture.exists("gone.txt"));
    assert!(!fixture.exists("moved.txt"));
    assert_eq!(fixture.read("dest/moved.txt"), "new\n");
}

/// 多块补丁按顺序应用，后一块在前一块之后搜索。
#[test]
fn sequential_blocks_apply_in_order() {
    let fixture = Fixture::new("edit-order");
    fixture.write("a.txt", "a1\na2\nb1\nb2\n");
    let patch = wrapped("*** Update File: a.txt\n@@\n-a1\n+A1\n@@\n-b2\n+B2");
    let result = execute(&fixture.ctx(), "edit", &edit_args(&patch));
    expect_changes(&result);
    assert_eq!(fixture.read("a.txt"), "A1\na2\nb1\nB2\n");
}

/// 新增空行（用 `+` 后跟空内容）写入正确。
#[test]
fn block_can_insert_blank_line() {
    let fixture = Fixture::new("edit-blank");
    fixture.write("a.txt", "one\ntwo\n");
    let patch = wrapped("*** Update File: a.txt\n@@\n one\n+\n two");
    let result = execute(&fixture.ctx(), "edit", &edit_args(&patch));
    expect_changes(&result);
    assert_eq!(fixture.read("a.txt"), "one\n\ntwo\n");
}

/// 一个 hunk 有实际变更、另一个 hunk 应用后内容不变：成功且 `partial=true`。
#[test]
fn no_op_hunk_marks_partial() {
    let fixture = Fixture::new("edit-partial-noop");
    fixture.write("changed.txt", "one\n");
    fixture.write("same.txt", "keep\n");
    let patch = wrapped(
        "*** Update File: changed.txt\n@@\n-one\n+TWO\n\
         *** Update File: same.txt\n@@\n-keep\n+keep",
    );
    let result = execute(&fixture.ctx(), "edit", &edit_args(&patch));
    match &result {
        ToolExecutionResult::Edit { changes, partial } => {
            assert_eq!(changes.len(), 1, "no-op hunk 不应产生 change");
            assert!(*partial, "有 hunk 未产生实际变更必须置 partial");
        }
        other => panic!("expected edit success, got {other:?}"),
    }
    assert_eq!(fixture.read("changed.txt"), "TWO\n");
    assert_eq!(fixture.read("same.txt"), "keep\n");
}

/// 只读文件（Unix 0444）：结果必须与磁盘真实状态一致。
///
/// 原子替换依赖目录可写，因此在多数 Unix 环境中覆盖成功；若平台拒绝写入则
/// 必须返回 `PermissionDenied` 且原字节不变。两种情况都不允许“失败但文件已变”。
#[cfg(unix)]
#[test]
fn read_only_file_write_result_matches_disk() {
    use std::os::unix::fs::PermissionsExt;

    let fixture = Fixture::new("edit-readonly");
    let path = fixture.write("ro.txt", "one\n");
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o444)).expect("chmod");
    let before = fixture.read_bytes("ro.txt");
    let patch = wrapped("*** Update File: ro.txt\n@@\n-one\n+two");
    let result = execute(&fixture.ctx(), "edit", &edit_args(&patch));
    let mut restore = std::fs::metadata(&path).expect("metadata").permissions();
    restore.set_mode(0o644);
    let _ = std::fs::set_permissions(&path, restore);

    match result {
        ToolExecutionResult::Failed { code, .. } => {
            assert_eq!(code, ToolErrorCode::PermissionDenied);
            assert_eq!(fixture.read_bytes("ro.txt"), before);
        }
        ToolExecutionResult::Edit { changes, .. } => {
            assert_eq!(changes.len(), 1);
            assert_eq!(fixture.read("ro.txt"), "two\n");
        }
        other => panic!("unexpected result {other:?}"),
    }
}

/// Delete 变更统计被删除的文本行数（二进制文件按 0 计）。
#[test]
fn delete_reports_removed_line_count() {
    let fixture = Fixture::new("edit-delete-count");
    fixture.write("gone.txt", "one\ntwo\nthree\n");
    let patch = wrapped("*** Delete File: gone.txt");
    let result = execute(&fixture.ctx(), "edit", &edit_args(&patch));
    let changes = expect_changes(&result);
    assert_eq!(changes[0].removed, 3);
    assert_eq!(changes[0].added, 0);
    assert!(!fixture.exists("gone.txt"));
}
