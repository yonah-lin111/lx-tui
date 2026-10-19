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

/// `*** End of File` 被接受，并记录在它所属的块上。
#[test]
fn accepts_end_of_file_marker_on_last_block() {
    let patch = "*** Begin Patch\n\
                 *** Update File: a.txt\n\
                 @@\n\
                 -old\n\
                 +new\n\
                 *** End of File\n\
                 *** End Patch";
    let hunks = parse(patch).expect("parse");
    let Hunk::Update { blocks, .. } = &hunks[0] else {
        panic!("expected update");
    };
    assert_eq!(blocks.len(), 1);
    assert!(blocks[0].at_eof, "块必须记录 at_eof");
}

/// `*** End of File` 之后还有 `@@` 块 → PatchInvalid（该标记只能标记最后一个块）。
#[test]
fn rejects_end_of_file_followed_by_another_block() {
    let patch = "*** Begin Patch\n\
                 *** Update File: a.txt\n\
                 @@\n\
                 -old\n\
                 +new\n\
                 *** End of File\n\
                 @@\n\
                 -x\n\
                 +y\n\
                 *** End Patch";
    let failure = parse(patch).expect_err("eof then block");
    assert_eq!(failure.code, ToolErrorCode::PatchInvalid);
    assert!(
        failure.message.contains("last block"),
        "message: {}",
        failure.message
    );
}

/// 标记行前后允许空白（codex strict 也 trim 两侧），但正文行的前导空格不被 trim。
#[test]
fn allows_whitespace_around_markers_but_not_in_body() {
    // 注意：行首空白必须真实存在于字符串里，因此不用 `\` 续行（它会吃掉下一行前导空白）。
    let patch = concat!(
        "  *** Begin Patch  \n",
        "\t*** Update File: a.txt\t\n",
        "@@ fn main() {\n",
        " \tlet x = 1;\n",
        "-  let y = 2;\n",
        "+  let y = 3;\n",
        "\t *** End of File  \n",
        "\t *** End Patch  ",
    );
    let hunks = parse(patch).expect("parse");
    let Hunk::Update { blocks, .. } = &hunks[0] else {
        panic!("expected update");
    };
    assert_eq!(blocks[0].context.as_deref(), Some("fn main() {"));
    // 以 ` ` 前缀的上下文行只剥掉那一个前缀空格，行内前导空白保真。
    assert_eq!(blocks[0].lines[0].kind, BodyLineKind::Context);
    assert_eq!(blocks[0].lines[0].text, "\tlet x = 1;");
    assert_eq!(blocks[0].lines[1].kind, BodyLineKind::Remove);
    assert_eq!(blocks[0].lines[1].text, "  let y = 2;");
    assert_eq!(blocks[0].lines[2].kind, BodyLineKind::Add);
    assert_eq!(blocks[0].lines[2].text, "  let y = 3;");
    assert!(blocks[0].at_eof);
}

/// 块内只有 `+` 行的纯插入块：裸 `@@` 也解析成功且 at_eof 默认为 false。
#[test]
fn pure_insert_block_parses_without_context() {
    let patch = "*** Begin Patch\n*** Update File: a.txt\n@@\n+tail\n*** End Patch";
    let hunks = parse(patch).expect("parse");
    let Hunk::Update { blocks, .. } = &hunks[0] else {
        panic!("expected update");
    };
    assert!(blocks[0].context.is_none());
    assert!(!blocks[0].at_eof);
    assert_eq!(blocks[0].lines.len(), 1);
    assert_eq!(blocks[0].lines[0].kind, BodyLineKind::Add);
}

/// codex `apply-patch/src/parser.rs` 的官方样例形态：逐条断言我们接受或明确报 PatchInvalid。
#[test]
fn codex_patch_fixtures_are_accepted_or_rejected_explicitly() {
    // 1. 三类 hunk 混排 + `*** Move to:` + `@@ def f():` —— codex `test_parse_patch` 样例。
    let official = "*** Begin Patch\n\
                    *** Add File: path/add.py\n\
                    +abc\n\
                    +def\n\
                    *** Delete File: path/delete.py\n\
                    *** Update File: path/update.py\n\
                    *** Move to: path/update2.py\n\
                    @@ def f():\n\
                    -    pass\n\
                    +    return 123\n\
                    *** End Patch";
    let hunks = parse(official).expect("official codex fixture must parse");
    assert_eq!(hunks.len(), 3);
    assert!(matches!(hunks[0], Hunk::Add { .. }));
    assert!(matches!(hunks[1], Hunk::Delete { .. }));
    let Hunk::Update {
        move_to, blocks, ..
    } = &hunks[2]
    else {
        panic!("expected update");
    };
    assert_eq!(
        move_to
            .as_ref()
            .map(|path| path.to_string_lossy().into_owned()),
        Some("path/update2.py".to_string())
    );
    assert_eq!(blocks.len(), 1);
    assert_eq!(blocks[0].context.as_deref(), Some("def f():"));

    // 2. Update hunk 后紧跟另一个 hunk（Add File）—— codex 亦支持。
    let update_then_add = "*** Begin Patch\n\
                           *** Update File: file.py\n\
                           @@\n\
                           +line\n\
                           *** Add File: other.py\n\
                           +content\n\
                           *** End Patch";
    let hunks = parse(update_then_add).expect("update followed by add must parse");
    assert_eq!(hunks.len(), 2);

    // 3. 无 `@@` 头的首个块（行首空格就是上下文前缀）—— codex 亦支持。
    let no_header = "*** Begin Patch\n*** Update File: file2.py\n import foo\n+bar\n*** End Patch";
    let hunks = parse(no_header).expect("implicit block must parse");
    let Hunk::Update { blocks, .. } = &hunks[0] else {
        panic!("expected update");
    };
    assert!(blocks[0].context.is_none());
    assert_eq!(blocks[0].lines[0].text, "import foo");

    // 4. 我们明确不支持的 codex 语法：`*** Environment ID:` 与 heredoc 包裹。
    //    两者都必须报 PatchInvalid，绝不静默忽略。
    let with_env = "*** Begin Patch\n\
                    *** Environment ID: abc\n\
                    *** Add File: a.txt\n\
                    +x\n\
                    *** End Patch";
    let failure = parse(with_env).expect_err("environment id is unsupported");
    assert_eq!(failure.code, ToolErrorCode::PatchInvalid);

    let heredoc = "<<'EOF'\n*** Begin Patch\n*** Add File: a.txt\n+x\n*** End Patch\nEOF";
    let failure = parse(heredoc).expect_err("heredoc wrapping is unsupported");
    assert_eq!(failure.code, ToolErrorCode::PatchInvalid);

    // 5. `*** End of File` 在 codex 的 grammar 中合法：我们接受并解析为块标记。
    let with_eof = "*** Begin Patch\n\
                    *** Update File: file.txt\n\
                    @@\n\
                    +quux\n\
                    *** End of File\n\
                    \n\
                    *** End Patch";
    let hunks = parse(with_eof).expect("end of file marker must parse");
    let Hunk::Update { blocks, .. } = &hunks[0] else {
        panic!("expected update");
    };
    assert!(blocks[0].at_eof);
    let expected = vec![BodyLine {
        kind: BodyLineKind::Add,
        text: "quux".to_string(),
    }];
    assert_eq!(blocks[0].lines, expected);
}
