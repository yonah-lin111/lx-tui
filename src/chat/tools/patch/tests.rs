//! 补丁解析测试：四类 hunk、标记顺序、非法形状与重复目标。

use super::*;

#[test]
fn parses_add_update_delete_and_move() {
    let patch = "*** Begin Patch\n\
                 *** Add File: a.txt\n\
                 +line1\n\
                 +line2\n\
                 *** Update File: b.txt\n\
                 *** Move to: c.txt\n\
                 @@\n\
                 -old\n\
                 +new\n\
                 *** Delete File: d.txt\n\
                 *** End Patch";
    let hunks = parse(patch).expect("parse");
    assert_eq!(hunks.len(), 3);
    match &hunks[0] {
        Hunk::Add { path, lines } => {
            assert_eq!(path.to_string_lossy(), "a.txt");
            assert_eq!(lines, &vec!["line1".to_string(), "line2".to_string()]);
        }
        other => panic!("expected add, got {other:?}"),
    }
    match &hunks[1] {
        Hunk::Update {
            path,
            move_to,
            blocks,
        } => {
            assert_eq!(path.to_string_lossy(), "b.txt");
            assert_eq!(
                move_to
                    .as_ref()
                    .map(|path| path.to_string_lossy().into_owned()),
                Some("c.txt".to_string())
            );
            assert_eq!(blocks.len(), 1);
            assert!(blocks[0].context.is_none());
            assert_eq!(blocks[0].lines.len(), 2);
        }
        other => panic!("expected update, got {other:?}"),
    }
    assert!(matches!(&hunks[2], Hunk::Delete { .. }));
}

#[test]
fn parses_context_anchor_and_body_line_kinds() {
    // 注意：行首空格是补丁的上下文前缀，必须保留，因此不用行尾 `\` 续行。
    let patch = "*** Begin Patch\n*** Update File: a.txt\n@@ fn main() {\n let x = 1;\n-let y = 2;\n+let y = 3;\n*** End Patch";
    let hunks = parse(patch).expect("parse");
    let Hunk::Update { blocks, .. } = &hunks[0] else {
        panic!("expected update");
    };
    assert_eq!(blocks[0].context.as_deref(), Some("fn main() {"));
    let kinds: Vec<BodyLineKind> = blocks[0].lines.iter().map(|line| line.kind).collect();
    assert_eq!(
        kinds,
        vec![
            BodyLineKind::Context,
            BodyLineKind::Remove,
            BodyLineKind::Add
        ]
    );
    assert_eq!(blocks[0].lines[0].text, "let x = 1;");
}

#[test]
fn update_without_context_marker_is_allowed() {
    let patch = "*** Begin Patch\n*** Update File: a.txt\n import x\n+import y\n*** End Patch";
    let hunks = parse(patch).expect("parse");
    let Hunk::Update { blocks, .. } = &hunks[0] else {
        panic!("expected update");
    };
    assert_eq!(blocks.len(), 1);
    assert!(blocks[0].context.is_none());
    assert_eq!(blocks[0].lines.len(), 2);
}

#[test]
fn rejects_missing_markers() {
    assert!(parse("").is_err());
    assert!(parse("*** Update File: a.txt\n@@\n-x\n+y").is_err());
    assert!(parse("*** Begin Patch\n*** Update File: a.txt\n@@\n-x\n+y").is_err());
    assert!(parse("*** Begin Patch\n*** End Patch").is_err());
}

#[test]
fn rejects_move_after_context() {
    let patch = "*** Begin Patch\n\
                 *** Update File: a.txt\n\
                 @@\n\
                 -x\n\
                 +y\n\
                 *** Move to: b.txt\n\
                 *** End Patch";
    let failure = parse(patch).expect_err("move after context");
    assert_eq!(failure.code, ToolErrorCode::PatchInvalid);
    assert!(failure.message.contains("Move to"));
}

#[test]
fn rejects_duplicate_move_marker() {
    let patch = "*** Begin Patch\n\
                 *** Update File: a.txt\n\
                 *** Move to: b.txt\n\
                 *** Move to: c.txt\n\
                 @@\n\
                 -x\n\
                 +y\n\
                 *** End Patch";
    assert!(parse(patch).is_err());
}

#[test]
fn rejects_bad_body_prefixes() {
    let add = "*** Begin Patch\n*** Add File: a.txt\nno plus\n*** End Patch";
    assert!(parse(add).is_err());

    let update = "*** Begin Patch\n*** Update File: a.txt\n@@\n*x\n*** End Patch";
    assert!(parse(update).is_err());
}

#[test]
fn rejects_empty_update_and_empty_add() {
    assert!(parse("*** Begin Patch\n*** Update File: a.txt\n*** End Patch").is_err());
    assert!(parse("*** Begin Patch\n*** Add File: a.txt\n*** End Patch").is_err());
}

#[test]
fn rejects_duplicate_targets() {
    let patch = "*** Begin Patch\n\
                 *** Update File: a.txt\n\
                 @@\n\
                 -x\n\
                 +y\n\
                 *** Delete File: a.txt\n\
                 *** End Patch";
    let failure = parse(patch).expect_err("duplicate target");
    assert_eq!(failure.code, ToolErrorCode::PatchInvalid);
    assert!(failure.message.contains("more than once"));
}

#[test]
fn rejects_unknown_marker_lines() {
    let patch = "*** Begin Patch\n*** Unknown: a.txt\n*** End Patch";
    let failure = parse(patch).expect_err("unknown marker");
    assert_eq!(failure.code, ToolErrorCode::PatchInvalid);
}

#[test]
fn rejects_empty_paths() {
    let patch = "*** Begin Patch\n*** Delete File:   \n*** End Patch";
    assert!(parse(patch).is_err());
}
