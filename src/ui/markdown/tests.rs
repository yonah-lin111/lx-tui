//! 单元测试；仅测试构建编译。

use super::*;

/// 全文 token 摘要：语义分类与对应原文，按逻辑行组织。
fn summarize(text: &str) -> Vec<Vec<(TokenKind, &str)>> {
    let lines: Vec<&str> = text.split('\n').collect();
    scan(text)
        .into_iter()
        .enumerate()
        .map(|(index, tokens)| {
            let line = lines.get(index).copied().unwrap_or_default();
            tokens
                .into_iter()
                .map(|token| (token.kind, &line[token.range]))
                .collect()
        })
        .collect()
}

/// 单行 token 摘要。
fn kinds(line: &str) -> Vec<(TokenKind, &str)> {
    summarize(line).into_iter().next().unwrap_or_default()
}

#[test]
fn headings_marker_and_text() {
    assert_eq!(
        kinds("## title"),
        vec![(TokenKind::Marker, "## "), (TokenKind::Heading, "title")]
    );
    assert_eq!(kinds("#hash"), vec![]);
    assert_eq!(kinds("####### x"), vec![]);
    assert_eq!(kinds("#"), vec![(TokenKind::Marker, "#")]);
}

#[test]
fn emphasis_family() {
    assert_eq!(
        kinds("**b**"),
        vec![
            (TokenKind::Marker, "**"),
            (TokenKind::Strong, "b"),
            (TokenKind::Marker, "**"),
        ]
    );
    assert_eq!(
        kinds("_i_"),
        vec![
            (TokenKind::Marker, "_"),
            (TokenKind::Emphasis, "i"),
            (TokenKind::Marker, "_"),
        ]
    );
    assert_eq!(
        kinds("~~s~~"),
        vec![
            (TokenKind::Marker, "~~"),
            (TokenKind::Strikethrough, "s"),
            (TokenKind::Marker, "~~"),
        ]
    );
    assert_eq!(kinds("**b"), vec![]);
    assert_eq!(kinds("*"), vec![]);
}

#[test]
fn nested_emphasis_is_not_parsed() {
    assert_eq!(
        kinds("**a *b* c**"),
        vec![
            (TokenKind::Marker, "**"),
            (TokenKind::Strong, "a *b* c"),
            (TokenKind::Marker, "**"),
        ]
    );
}

#[test]
fn intraword_underscores_follow_naive_scan() {
    assert_eq!(
        kinds("text_with_underscores"),
        vec![
            (TokenKind::Marker, "_"),
            (TokenKind::Emphasis, "with"),
            (TokenKind::Marker, "_"),
        ]
    );
}

#[test]
fn code_spans_suppress_inner_markup() {
    assert_eq!(
        kinds("`a*b*`"),
        vec![
            (TokenKind::Marker, "`"),
            (TokenKind::InlineCode, "a*b*"),
            (TokenKind::Marker, "`"),
        ]
    );
    assert_eq!(kinds("`abc"), vec![]);
}

#[test]
fn fenced_blocks_span_lines() {
    let lines = summarize("```rust\nlet x = 1;\n```\n**b**");
    assert_eq!(lines[0], vec![(TokenKind::Marker, "```rust")]);
    assert_eq!(lines[1], vec![(TokenKind::CodeBlock, "let x = 1;")]);
    assert_eq!(lines[2], vec![(TokenKind::Marker, "```")]);
    assert_eq!(lines[3].len(), 3);
}

#[test]
fn quotes_lists_and_rules() {
    assert_eq!(
        kinds("> hi"),
        vec![(TokenKind::Marker, "> "), (TokenKind::Quote, "hi")]
    );
    assert_eq!(
        kinds(">>a"),
        vec![(TokenKind::Marker, ">>"), (TokenKind::Quote, "a")]
    );
    // 列表标记有独立语义（符号高亮用），不再并入通用 `Marker`。
    assert_eq!(kinds("- item"), vec![(TokenKind::ListBullet, "- ")]);
    assert_eq!(kinds("1. item"), vec![(TokenKind::ListOrdered, "1. ")]);
    assert_eq!(kinds("  - nested"), vec![(TokenKind::ListBullet, "  - ")]);
    assert_eq!(kinds("*item"), vec![]);
    assert_eq!(kinds("---"), vec![(TokenKind::Marker, "---")]);
    assert_eq!(kinds("* * *"), vec![(TokenKind::Marker, "* * *")]);
}

#[test]
fn links_and_urls() {
    assert_eq!(
        kinds("[a](b)"),
        vec![
            (TokenKind::Marker, "["),
            (TokenKind::LinkText, "a"),
            (TokenKind::Marker, "]("),
            (TokenKind::Url, "b"),
            (TokenKind::Marker, ")"),
        ]
    );
    assert_eq!(
        kinds("[a]()"),
        vec![
            (TokenKind::Marker, "["),
            (TokenKind::LinkText, "a"),
            (TokenKind::Marker, "]("),
            (TokenKind::Marker, ")"),
        ]
    );
    assert_eq!(kinds("[a] b"), vec![]);
    assert_eq!(
        kinds("see https://x.y/z now"),
        vec![(TokenKind::Url, "https://x.y/z")]
    );
}

#[test]
fn empty_text_yields_single_empty_line() {
    assert_eq!(scan(""), vec![Vec::new()]);
    let lines = summarize("a\nb");
    assert_eq!(lines.len(), 2);
    assert!(lines[0].is_empty() && lines[1].is_empty());
}

// ---- 模板块与 @ 文件提及 ----

#[test]
fn template_start_line_tokens_split_by_semantics() {
    assert_eq!(
        kinds("&&& addTemplate --start {title: }"),
        vec![
            (TokenKind::TemplateMarker, "&&&"),
            (
                TokenKind::TemplateCommand(Some(SlashCommandId::Add)),
                "addTemplate"
            ),
            (TokenKind::TemplateMarker, "--start"),
            (TokenKind::TemplateTitle, "{title: }"),
        ]
    );
    assert_eq!(
        kinds("  &&& customTemplate --start"),
        vec![
            (TokenKind::TemplateMarker, "&&&"),
            (TokenKind::TemplateCommand(None), "customTemplate"),
            (TokenKind::TemplateMarker, "--start"),
        ]
    );
}

#[test]
fn template_title_on_own_line_is_styled_inside_block() {
    let lines =
        summarize("&&& addTemplate --start\n{title: }\n# Add\n&&& addTemplate --end\n{title: }");
    assert_eq!(
        lines[1],
        vec![(TokenKind::TemplateTitle, "{title: }")],
        "块内独立标题行高亮"
    );
    assert!(lines[4].is_empty(), "块外同名行不高亮: {:?}", lines[4]);
}

#[test]
fn template_end_line_tokens_include_status_and_metadata() {
    assert_eq!(
        kinds("&&& bugTemplate --end done {id:abc}"),
        vec![
            (TokenKind::TemplateMarker, "&&&"),
            (
                TokenKind::TemplateCommand(Some(SlashCommandId::Bug)),
                "bugTemplate"
            ),
            (TokenKind::TemplateMarker, "--end"),
            (TokenKind::TemplateMarker, "done"),
            (TokenKind::TemplateMarker, "{id:abc}"),
        ]
    );
    assert_eq!(
        kinds("&&& --end in_progress"),
        vec![
            (TokenKind::TemplateMarker, "&&&"),
            (TokenKind::TemplateMarker, "--end"),
            (TokenKind::TemplateMarker, "in_progress"),
        ]
    );
    assert_eq!(kinds("&&&"), vec![(TokenKind::TemplateMarker, "&&&")]);
}

#[test]
fn template_body_lines_keep_markdown_highlight() {
    let summary =
        summarize("&&& addTemplate --start {title: }\n# Add Requirement\n&&& addTemplate --end");
    assert_eq!(
        summary[1],
        vec![
            (TokenKind::Marker, "# "),
            (TokenKind::Heading, "Add Requirement"),
        ]
    );
    assert_eq!(
        summary[2].first(),
        Some(&(TokenKind::TemplateMarker, "&&&"))
    );
}

#[test]
fn fence_suppresses_template_tokens() {
    assert_eq!(
        kinds("```\n&&& addTemplate --start"),
        vec![(TokenKind::Marker, "```")]
    );
    let summary = summarize("```\n&&& addTemplate --start");
    assert_eq!(
        summary[1],
        vec![(TokenKind::CodeBlock, "&&& addTemplate --start")]
    );
}

#[test]
fn file_mention_tokens_scan_paths() {
    assert_eq!(
        kinds("see @src/app.rs now"),
        vec![(TokenKind::FileMention, "@src/app.rs")]
    );
    assert_eq!(
        kinds("@README.md"),
        vec![(TokenKind::FileMention, "@README.md")]
    );
    assert_eq!(
        kinds("[@src/ui/]"),
        vec![(TokenKind::FileMention, "@src/ui/")],
        "`[` 后是提及边界，`]` 终止路径"
    );
    assert_eq!(kinds("mail a@b.com"), vec![], "非边界 @ 不视为文件提及");
    assert_eq!(kinds("@"), vec![], "空提及不成词");
    assert_eq!(
        kinds("@file, next"),
        vec![(TokenKind::FileMention, "@file")],
        "边界标点终止路径"
    );
    assert_eq!(
        kinds("`@not/mention`"),
        vec![
            (TokenKind::Marker, "`"),
            (TokenKind::InlineCode, "@not/mention"),
            (TokenKind::Marker, "`"),
        ]
    );
}

#[test]
fn refer_tokens_carry_kind_specific_highlights() {
    assert_eq!(
        kinds("see @[refer-file](/tmp/a.md) and @[refer-folder](/tmp/dir)"),
        vec![
            (TokenKind::ReferFile, "@[refer-file](/tmp/a.md)"),
            (TokenKind::ReferFolder, "@[refer-folder](/tmp/dir)"),
        ]
    );
    assert_eq!(
        kinds("@[refer-image](/tmp/a\\(1\\).png)"),
        vec![(TokenKind::ReferImage, "@[refer-image](/tmp/a\\(1\\).png)")],
        "括号转义的路径整段成一个引用 token"
    );
    assert_eq!(
        kinds("@[refer-file](/a_b*c.md) tail"),
        vec![(TokenKind::ReferFile, "@[refer-file](/a_b*c.md)")],
        "引用整段成一个 token，路径里的 * 不参与强调解析"
    );
    assert_eq!(
        kinds("@[refer-file](/a.md) @src/app.rs"),
        vec![
            (TokenKind::ReferFile, "@[refer-file](/a.md)"),
            (TokenKind::FileMention, "@src/app.rs"),
        ],
        "引用与 @ 提及各自独立分类"
    );
    assert!(
        !kinds("@[refer-unknown](/a.md)")
            .iter()
            .any(|(kind, _)| matches!(
                kind,
                TokenKind::ReferFile | TokenKind::ReferFolder | TokenKind::ReferImage
            )),
        "类型非法的标记不按引用着色"
    );
    assert_eq!(kinds("@[refer-file]"), vec![], "缺路径不成标记");
}

#[test]
fn short_ref_tokens_are_one_highlight() {
    assert_eq!(
        kinds("see @ref/agent/lx-agent/ and @ref/src/main.rs"),
        vec![
            (TokenKind::ShortRef, "@ref/agent/lx-agent/"),
            (TokenKind::ShortRef, "@ref/src/main.rs"),
        ]
    );
    // 整段一个 token：路径内的 `*`、`_` 不参与强调解析。
    assert_eq!(
        kinds("@ref/a_b*c.md tail"),
        vec![(TokenKind::ShortRef, "@ref/a_b*c.md")]
    );
    // 句末 `.` 属于句子标点，不进 token。
    assert_eq!(
        kinds("see @ref/a/b.md."),
        vec![(TokenKind::ShortRef, "@ref/a/b.md")]
    );
    // 与既有 `@` 提及、`@[refer-*]` 引用各自独立分类。
    assert_eq!(
        kinds("@ref/a/b @src/app.rs @[refer-file](/x/y.md)"),
        vec![
            (TokenKind::ShortRef, "@ref/a/b"),
            (TokenKind::FileMention, "@src/app.rs"),
            (TokenKind::ReferFile, "@[refer-file](/x/y.md)"),
        ],
        "短引用不复用提及或引用分类"
    );
    assert_eq!(
        kinds("@ref/"),
        vec![(TokenKind::FileMention, "@ref/")],
        "只输入前缀时退回普通 @ 提及，不当作短引用"
    );
    assert_eq!(
        kinds("`@ref/a/b`"),
        vec![
            (TokenKind::Marker, "`"),
            (TokenKind::InlineCode, "@ref/a/b"),
            (TokenKind::Marker, "`"),
        ]
    );
}
