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
    assert_eq!(kinds("- item"), vec![(TokenKind::Marker, "- ")]);
    assert_eq!(kinds("1. item"), vec![(TokenKind::Marker, "1. ")]);
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
