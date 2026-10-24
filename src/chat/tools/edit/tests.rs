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

/// 纯插入块带 `@@ <context>`：插在该定位行之后。
#[test]
fn pure_insert_with_context_goes_after_context() {
    let fixture = Fixture::new("edit-insert-context");
    fixture.write("a.txt", "alpha\nmarker\nomega\n");
    let patch = wrapped("*** Update File: a.txt\n@@ marker\n+inserted");
    let result = execute(&fixture.ctx(), "edit", &edit_args(&patch));
    expect_changes(&result);
    assert_eq!(fixture.read("a.txt"), "alpha\nmarker\ninserted\nomega\n");
}

/// 纯插入块无 `@@ <context>`：追加到**文件末尾**（与 codex 一致，不是文件开头）。
#[test]
fn pure_insert_without_context_appends_to_end() {
    let fixture = Fixture::new("edit-insert-append");
    fixture.write("a.txt", "line1\nline2\nline3\n");
    let patch = wrapped("*** Update File: a.txt\n@@\n+tail");
    let result = execute(&fixture.ctx(), "edit", &edit_args(&patch));
    let changes = expect_changes(&result);
    assert_eq!((changes[0].added, changes[0].removed), (1, 0));
    assert_eq!(fixture.read("a.txt"), "line1\nline2\nline3\ntail\n");
    assert_ne!(fixture.read("a.txt"), "tail\nline1\nline2\nline3\n");

    // 追加到末尾的块不移动搜索游标：同文件的后续块仍能在原范围内定位。
    fixture.write("b.txt", "one\ntwo\n");
    let patch = wrapped("*** Update File: b.txt\n@@\n+appended\n@@\n-one\n+ONE");
    let result = execute(&fixture.ctx(), "edit", &edit_args(&patch));
    expect_changes(&result);
    assert_eq!(fixture.read("b.txt"), "ONE\ntwo\nappended\n");
}

/// `*** End of File`：块锚定文件末尾，修改最后一段成功。
#[test]
fn patch_accepts_end_of_file_marker() {
    let fixture = Fixture::new("edit-eof");
    fixture.write("a.txt", "head\nmiddle\ntail\n");
    let patch = wrapped("*** Update File: a.txt\n@@\n-tail\n+TAIL\n*** End of File");
    let result = execute(&fixture.ctx(), "edit", &edit_args(&patch));
    let changes = expect_changes(&result);
    assert_eq!((changes[0].added, changes[0].removed), (1, 1));
    assert_eq!(fixture.read("a.txt"), "head\nmiddle\nTAIL\n");
}

/// `*** End of File` 锚定失败（文件末尾不是该块）→ PatchContextMismatch 且文件不变。
#[test]
fn patch_end_of_file_mismatch_is_reported() {
    let fixture = Fixture::new("edit-eof-mismatch");
    fixture.write("a.txt", "head\nmiddle\ntail\n");
    let before = fixture.read_bytes("a.txt");
    let patch = wrapped("*** Update File: a.txt\n@@\n-middle\n+MIDDLE\n*** End of File");
    let result = execute(&fixture.ctx(), "edit", &edit_args(&patch));
    assert_eq!(expect_failure(&result), ToolErrorCode::PatchContextMismatch);
    assert_eq!(fixture.read_bytes("a.txt"), before);
    let text = super::super::format_for_model(&result);
    assert!(text.contains("*** End of File"), "text: {text}");
}

/// `*** End of File` 之后还有 `@@` 块 → PatchInvalid（只能标记最后一个块）。
#[test]
fn patch_rejects_end_of_file_on_non_last_block() {
    let fixture = Fixture::new("edit-eof-non-last");
    fixture.write("a.txt", "one\ntwo\n");
    let before = fixture.read_bytes("a.txt");
    let patch = wrapped("*** Update File: a.txt\n@@\n-one\n+ONE\n*** End of File\n@@\n-two\n+TWO");
    let result = execute(&fixture.ctx(), "edit", &edit_args(&patch));
    assert_eq!(expect_failure(&result), ToolErrorCode::PatchInvalid);
    assert_eq!(fixture.read_bytes("a.txt"), before);
}

/// 标记行前后允许空白，正文行的前导空格保持原义（不被 trim）。
#[test]
fn patch_allows_whitespace_around_markers() {
    let fixture = Fixture::new("edit-marker-space");
    fixture.write("a.py", "def f():\n    pass\n    return None\n");
    // 行首空白必须真实存在，故用 concat! 而非 `\` 续行。
    let patch = concat!(
        "  *** Begin Patch  \n",
        "\t*** Update File: a.py\t\n",
        "@@ def f():\n",
        "     pass\n",
        "-    return None\n",
        "+    return 1\n",
        "\t *** End Patch  ",
    );
    let result = execute(&fixture.ctx(), "edit", &edit_args(patch));
    let changes = expect_changes(&result);
    assert_eq!((changes[0].added, changes[0].removed), (1, 1));
    // 上下文行 `     pass` 剥掉一个前缀空格后是 `    pass`，与文件一致。
    assert_eq!(fixture.read("a.py"), "def f():\n    pass\n    return 1\n");
}

/// 文件行尾有空格、补丁里没有：rstrip 回退后仍能应用。
#[test]
fn patch_matches_after_trailing_whitespace_difference() {
    let fixture = Fixture::new("edit-rstrip");
    fixture.write("a.txt", "alpha  \nbeta\n");
    let patch = wrapped("*** Update File: a.txt\n@@\n-alpha\n+ALPHA");
    let result = execute(&fixture.ctx(), "edit", &edit_args(&patch));
    expect_changes(&result);
    assert_eq!(fixture.read("a.txt"), "ALPHA\nbeta\n");
}

/// 上下文多处匹配：失败消息列出候选行号，绝不猜测替换位置。
#[test]
fn patch_context_ambiguity_lists_candidate_lines() {
    let fixture = Fixture::new("edit-candidates");
    fixture.write("a.txt", "line1\nsame\nline2\nsame\nline3\nsame\n");
    let before = fixture.read_bytes("a.txt");
    let patch = wrapped("*** Update File: a.txt\n@@\n-same\n+other");
    let result = execute(&fixture.ctx(), "edit", &edit_args(&patch));
    let ToolExecutionResult::Failed { code, message, .. } = &result else {
        panic!("expected failure, got {result:?}");
    };
    assert_eq!(*code, ToolErrorCode::PatchContextMismatch);
    assert!(
        message.contains("(lines 2, 4, 6)"),
        "message must list candidate lines: {message}"
    );
    assert_eq!(fixture.read_bytes("a.txt"), before);
}

/// `*** Move to:` 目标已存在 → PatchInvalid，且目标内容与源文件都未被修改。
#[test]
fn move_to_existing_target_is_rejected() {
    let fixture = Fixture::new("edit-move-exists");
    fixture.write("from.txt", "new content\n");
    fixture.write("to.txt", "PRECIOUS USER DATA\n");
    let from_before = fixture.read_bytes("from.txt");
    let to_before = fixture.read_bytes("to.txt");
    let patch =
        wrapped("*** Update File: from.txt\n*** Move to: to.txt\n@@\n-new content\n+changed");
    let result = execute(&fixture.ctx(), "edit", &edit_args(&patch));
    let ToolExecutionResult::Failed { code, message, .. } = &result else {
        panic!("expected failure, got {result:?}");
    };
    assert_eq!(*code, ToolErrorCode::PatchInvalid);
    assert!(
        message.contains("destination already exists"),
        "message: {message}"
    );
    // 消息只含路径，不回显被覆盖的内容。
    assert!(!message.contains("PRECIOUS"), "message: {message}");
    assert_eq!(fixture.read_bytes("to.txt"), to_before);
    assert_eq!(fixture.read_bytes("from.txt"), from_before);
}

/// 多 hunk 的增删计数按补丁行统计：20 行文件改 2 行 → `+2 -2`。
#[test]
fn multi_hunk_edit_reports_patch_line_counts() {
    let fixture = Fixture::new("edit-patch-counts");
    let mut lines = Vec::new();
    for index in 1..=20 {
        lines.push(format!("line{index}"));
    }
    let before = format!("{}\n", lines.join("\n"));
    fixture.write("a.txt", &before);
    let patch = wrapped("*** Update File: a.txt\n@@\n-line2\n+LINE2\n@@\n-line19\n+LINE19");
    let result = execute(&fixture.ctx(), "edit", &edit_args(&patch));
    let changes = expect_changes(&result);
    assert_eq!((changes[0].added, changes[0].removed), (2, 2));
}

/// 候选行号最多展示 5 个，其余用 `…` 表示。
#[test]
fn candidate_line_list_is_capped_with_ellipsis() {
    assert_eq!(
        super::format_candidate_lines(&[0, 11, 47, 90]),
        "lines 1, 12, 48, 91"
    );
    assert_eq!(
        super::format_candidate_lines(&[0, 1, 2, 3, 4, 5, 6]),
        "lines 1, 2, 3, 4, 5, …"
    );
}

// ===== 文件差异：纯移动、移动失败与内容不变 =====

/// 纯移动（内容不变）：必须真的 rename，不能被 `before == after` 提前跳过。
#[test]
fn pure_move_renames_without_content_change() {
    let fixture = Fixture::new("edit-pure-move");
    fixture.write("from.txt", "same\nbody\n");
    let patch = wrapped("*** Update File: from.txt\n*** Move to: dest/from.txt");
    let result = execute(&fixture.ctx(), "edit", &edit_args(&patch));
    let changes = expect_changes(&result);
    assert_eq!(changes.len(), 1);
    assert!(matches!(&changes[0].kind, FileChangeKind::Moved { .. }));
    assert_eq!((changes[0].added, changes[0].removed), (0, 0));
    assert!(!fixture.exists("from.txt"), "纯移动必须真的移走源文件");
    assert_eq!(fixture.read("dest/from.txt"), "same\nbody\n");
    let diff = changes[0].diff.as_ref().expect("纯移动也带 diff");
    assert!(diff.counts_known());
    assert!(diff.hunks().is_empty(), "内容没变就不应有差异正文");
}

/// 修改并移动：差异正文与 `Moved` 动作一致，磁盘结果同步。
#[test]
fn move_with_edit_reports_content_diff() {
    let fixture = Fixture::new("edit-move-diff");
    fixture.write("old.txt", "before\nkeep\n");
    let patch = wrapped("*** Update File: old.txt\n*** Move to: new.txt\n@@\n-before\n+after");
    let result = execute(&fixture.ctx(), "edit", &edit_args(&patch));
    let changes = expect_changes(&result);
    assert!(matches!(&changes[0].kind, FileChangeKind::Moved { .. }));
    let diff = changes[0].diff.as_ref().expect("移动同时改内容");
    let kinds: Vec<super::super::DiffLineKind> = diff
        .hunks()
        .iter()
        .flat_map(|hunk| hunk.lines.iter())
        .map(|line| line.kind)
        .collect();
    assert!(kinds.contains(&super::super::DiffLineKind::Removed));
    assert!(kinds.contains(&super::super::DiffLineKind::Added));
    assert_eq!(fixture.read("new.txt"), "after\nkeep\n");
}

/// 移动阶段失败（目标目录不可写）：源文件已写入，失败结果必须带上这份 Updated 与 diff。
#[cfg(unix)]
#[test]
fn move_failure_reports_already_written_source() {
    use std::os::unix::fs::PermissionsExt;
    let fixture = Fixture::new("edit-move-failure");
    fixture.write("src.txt", "old\n");
    // 目标目录去掉写位：源文件所在目录仍可写，写源成功后 rename 必然失败。
    let dest = fixture.path().join("destdir");
    std::fs::create_dir_all(&dest).expect("dest dir");
    let mut permissions = std::fs::metadata(&dest).expect("metadata").permissions();
    permissions.set_mode(0o500);
    std::fs::set_permissions(&dest, permissions).expect("chmod");
    let patch = wrapped("*** Update File: src.txt\n*** Move to: destdir/dest.txt\n@@\n-old\n+new");
    let result = execute(&fixture.ctx(), "edit", &edit_args(&patch));
    let mut restore = std::fs::metadata(&dest).expect("metadata").permissions();
    restore.set_mode(0o700);
    let _ = std::fs::set_permissions(&dest, restore);
    let ToolExecutionResult::Failed { changes, .. } = &result else {
        panic!("expected failure, got {result:?}");
    };
    assert_eq!(changes.len(), 1, "已写盘的源文件必须出现在失败变更里");
    assert_eq!(changes[0].kind, FileChangeKind::Updated);
    assert!(changes[0].diff.is_some(), "已写盘的源文件必须带差异");
    assert_eq!(fixture.read("src.txt"), "new\n");
    assert!(!fixture.exists("destdir/dest.txt"));
}

/// 多文件补丁中途失败：失败前已提交的文件仍带 diff，未执行文件不出现。
#[test]
fn partial_failure_keeps_committed_diffs() {
    let fixture = Fixture::new("edit-partial-diff");
    fixture.write("first.txt", "one\n");
    fixture.write("second.txt", "keep\n");
    let patch = wrapped(
        "*** Update File: first.txt\n@@\n-one\n+ONE\n\
         *** Update File: second.txt\n@@\n-not-there\n+x",
    );
    let result = execute(&fixture.ctx(), "edit", &edit_args(&patch));
    let ToolExecutionResult::Failed { changes, .. } = &result else {
        panic!("expected failure, got {result:?}");
    };
    assert_eq!(changes.len(), 1);
    assert_eq!(changes[0].kind, FileChangeKind::Updated);
    assert!(changes[0].diff.is_some());
}

/// 删除不可解码文件：仍能删除，差异标记为不可用且计数未知。
#[test]
fn delete_binary_file_keeps_deletion_and_marks_diff_unavailable() {
    let fixture = Fixture::new("edit-delete-binary");
    fixture.write_bytes("blob.bin", &[0u8, 1, 2, 3]);
    let patch = wrapped("*** Delete File: blob.bin");
    let result = execute(&fixture.ctx(), "edit", &edit_args(&patch));
    let changes = expect_changes(&result);
    assert!(!fixture.exists("blob.bin"));
    let diff = changes[0].diff.as_ref().expect("删除也带 diff");
    assert!(!diff.counts_known());
    assert!(diff.hunks().is_empty());
    // 汇总摘要必须显示未知而不是 0。
    let summary = super::super::to_summary(&result);
    assert!(!summary.counts_known);
    assert!(summary.added.is_none() && summary.removed.is_none());
}

/// 记录 Prepare 阶段提交的 mutation 形状。
#[derive(Default)]
struct RecordingRecorder {
    prepared: std::sync::Mutex<Option<crate::chat::tools::mutation::PreparedMutation>>,
}

impl crate::chat::tools::mutation::MutationRecorder for RecordingRecorder {
    fn prepare(
        &self,
        mutation: &crate::chat::tools::mutation::PreparedMutation,
    ) -> Result<
        crate::chat::tools::mutation::MutationTicket,
        crate::chat::tools::mutation::MutationError,
    > {
        *self.prepared.lock().expect("lock") = Some(mutation.clone());
        Ok(crate::chat::tools::mutation::MutationTicket {
            ids: (0..mutation.paths.len())
                .map(|index| i64::try_from(index).unwrap_or(0))
                .collect(),
        })
    }

    fn commit(
        &self,
        _ticket: crate::chat::tools::mutation::MutationTicket,
        _observed: &crate::chat::tools::mutation::ObservedMutation,
    ) -> Result<(), crate::chat::tools::mutation::MutationError> {
        Ok(())
    }
}

/// Update + Move（无链接间接）必须只产生一条同路径 mutation：内容写入与 rename
/// 是同一路径的连续状态，拆成两条会被撤销的线性检查判成第三方交错写。
#[test]
fn update_with_move_records_one_mutation_per_path() {
    let fixture = Fixture::new("edit-move-linear");
    fixture.write("a.txt", "one\ntwo\n");
    let patch = wrapped("*** Update File: a.txt\n*** Move to: b.txt\n@@\n-one\n+uno");
    let recorder = RecordingRecorder::default();
    let result = super::run_with(&fixture.ctx(), &edit_args(&patch), Some(&recorder));
    let changes = expect_changes(&result);
    assert!(matches!(&changes[0].kind, FileChangeKind::Moved { .. }));
    assert!(!fixture.exists("a.txt"));
    assert_eq!(fixture.read("b.txt"), "uno\ntwo\n");

    let prepared = recorder
        .prepared
        .lock()
        .expect("lock")
        .clone()
        .expect("prepared");
    let mut targets: Vec<_> = prepared
        .paths
        .iter()
        .map(|path| path.target.clone())
        .collect();
    targets.sort();
    let total = targets.len();
    targets.dedup();
    assert_eq!(
        targets.len(),
        total,
        "同一路径不能有两条 mutation：{targets:?}"
    );
    let source = prepared
        .paths
        .iter()
        .find(|path| path.operation == crate::chat::tools::mutation::MutationOperation::MoveSource)
        .expect("move source");
    assert_eq!(
        source.before.bytes,
        b"one\ntwo\n".to_vec(),
        "before 必须是写前内容"
    );
}

/// 移动符号链接：`MoveSource` 按目录项重算 token（不追随目标），
/// 否则持久层的 before 重算会误判为读取后外部修改。
#[cfg(unix)]
#[test]
fn move_symlink_uses_entry_version_for_move_source() {
    use crate::chat::tools::mutation::{
        MutationError, MutationRecorder, MutationTicket, ObservedMutation, PreparedMutation,
    };
    use std::sync::{Arc, Mutex};

    /// 只记录 prepare 请求、永远成功的内存 recorder。
    struct Recording {
        prepared: Arc<Mutex<Vec<PreparedMutation>>>,
    }
    impl MutationRecorder for Recording {
        fn prepare(&self, mutation: &PreparedMutation) -> Result<MutationTicket, MutationError> {
            self.prepared.lock().expect("lock").push(mutation.clone());
            Ok(MutationTicket {
                ids: vec![1; mutation.paths.len()],
            })
        }
        fn commit(
            &self,
            _ticket: MutationTicket,
            _observed: &ObservedMutation,
        ) -> Result<(), MutationError> {
            Ok(())
        }
    }

    let fixture = Fixture::new("edit-move-symlink");
    let real = fixture.write("real.txt", "one\n");
    let link = fixture.join("link.txt");
    std::os::unix::fs::symlink(&real, &link).expect("symlink");
    let prepared = Arc::new(Mutex::new(Vec::new()));
    let recorder = Recording {
        prepared: prepared.clone(),
    };
    let patch = wrapped("*** Update File: link.txt\n*** Move to: moved.txt\n@@\n-one\n+two");
    let result =
        super::super::execute_with(&fixture.ctx(), "edit", &edit_args(&patch), Some(&recorder));
    assert!(!result.is_failure(), "移动符号链接必须成功：{result:?}");
    let recorded = prepared.lock().expect("lock");
    let mutation = recorded.first().expect("prepare recorded");
    let source = mutation
        .paths
        .iter()
        .find(|path| path.operation.as_str() == "move_src")
        .expect("move_src recorded");
    let expected = source.expected_version.as_deref().expect("token");
    // token 必须与 MoveSource 的 before（目录项）一致，而不是跟随目标的版本。
    assert_eq!(
        expected,
        crate::platform::version_token_of(&source.before),
        "move_src 的期望版本必须按目录项重算"
    );
    let resolved = crate::platform::read_raw(&real, true).expect("read target");
    assert_ne!(
        expected,
        crate::platform::version_token_of(&resolved),
        "目录项版本不得等于跟随后的内容版本（符号链接场景）"
    );
}
