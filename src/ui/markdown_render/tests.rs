//! 单元测试：markdown 渲染器覆盖全部格式、标记折叠与任意来源。

use super::*;

use crate::ui::text;

/// 把一组渲染行拼成纯文本（用于断言可见内容）。
fn lines_text(lines: &[MdLine]) -> String {
    lines
        .iter()
        .map(MdLine::text)
        .collect::<Vec<_>>()
        .join("\n")
}

/// 样例文档（与 UI 查看器共用）。
fn sample() -> &'static str {
    text::agent::MD_SAMPLE
}

#[test]
fn sample_document_covers_every_block_and_inline_format() {
    let sample = sample();
    // 六级标题。
    for hashes in 1..=6 {
        assert!(
            sample.contains(&format!("\n{} ", "#".repeat(hashes)))
                || sample.starts_with(&format!("{} ", "#".repeat(hashes))),
            "缺少 {} 级标题",
            hashes
        );
    }
    // 段落 / 行内。
    for needle in ["**bold**", "*italic*", "***both***", "~~strike~~", "`code`"] {
        assert!(sample.contains(needle), "缺少行内 {needle}");
    }
    // 列表 / 任务 / 引用 / 代码 / 表格 / 水平线 / 图片 / 链接。
    for needle in [
        "- bullet one",
        "  - nested bullet",
        "1. ordered one",
        "- [x] done task",
        "- [ ] open task",
        "> quoted line one",
        "```rust",
        "    indented code",
        "| Left | Center | Right |",
        "---",
        "[OpenAI](https://openai.com)",
        "<https://example.com>",
        "![logo]",
    ] {
        assert!(sample.contains(needle), "缺少 {needle}");
    }
}

#[test]
fn sample_renders_full_document_without_leaking_markers() {
    let lines = render(sample());
    let joined = lines_text(&lines);
    // 六级标题正文都在。
    for title in [
        "Heading 1",
        "Heading 2",
        "Heading 3",
        "Heading 4",
        "Heading 5",
        "Heading 6",
    ] {
        assert!(joined.contains(title), "缺标题 {title}");
    }
    // 行内标记被吃掉（不出现 `**`、`~~`、`***`），但正文在。
    assert!(!joined.contains("**"), "泄漏 ** : {joined}");
    assert!(!joined.contains("~~"), "泄漏 ~~ : {joined}");
    assert!(!joined.contains("***"), "泄漏 *** : {joined}");
    assert!(joined.contains("bold") && joined.contains("strike"));
    // 围栏被吃掉。
    assert!(!joined.contains("```"), "泄漏围栏");
    // 代码正文在（含关键字）。
    assert!(joined.contains("fn main()"));
    assert!(joined.contains("hello"));
    // 表格内容在。
    assert!(joined.contains("Left") && joined.contains("Center") && joined.contains("Right"));
    // 链接文字与 URL 都在。
    assert!(joined.contains("OpenAI"));
    assert!(joined.contains("https://openai.com"));
    // 图片 alt 在。
    assert!(joined.contains("logo"));
}

#[test]
fn headings_hide_hashes() {
    let joined = lines_text(&render(sample()));
    assert!(!joined.contains("# Heading 1"), "应隐藏 `#`");
    assert!(joined.contains("Heading 1"));
}

#[test]
fn table_is_bordered() {
    let joined = lines_text(&render(sample()));
    assert!(joined.contains('┌'), "表格应有方框");
    assert!(joined.contains('┘'));
}

#[test]
fn code_block_shows_language_and_syntax_colors() {
    let lines = render(sample());
    let joined = lines_text(&lines);
    assert!(joined.contains(text::agent::MD_LANG), "应显示代码语言标记");
    // 代码块内的关键字/字符串做了分色（至少出现关键字样式）。
    let code_spans: usize = lines
        .iter()
        .flat_map(|line| line.spans.iter())
        .filter(|(_, span_style)| {
            *span_style == style::code_keyword()
                || *span_style == style::code_string()
                || *span_style == style::code_number()
                || *span_style == style::code_comment()
                || *span_style == style::code_type()
        })
        .count();
    assert!(
        code_spans > 0,
        "代码块应做语法高亮（关键字/字符串/数字/注释/类型分色）"
    );
    // 正文文本仍完整保留（拼接即源码）。
    assert!(joined.contains("fn main()"), "代码正文应保留");
}

#[test]
fn blockquote_uses_angle_prefix_not_left_bar() {
    let lines = render(sample());
    let quote = lines
        .iter()
        .find(|line| line.text().contains("quoted line one"))
        .expect("quote line");
    let text = quote.text();
    assert!(text.starts_with("> "), "引用应以 `> ` 开头：{text}");
    assert!(!text.contains('│'), "引用不应使用左边框：{text}");
}

#[test]
fn inline_renders_link_with_url_and_collapses_markers() {
    let spans = inline_spans("see [docs](https://x.io) and **bold** end");
    let joined: String = spans.iter().map(|(value, _)| value.as_str()).collect();
    assert!(joined.contains("docs"));
    assert!(joined.contains("(https://x.io)"));
    assert!(joined.contains("bold"));
    assert!(!joined.contains('*'));
    assert!(!joined.contains('['));
}

#[test]
fn heading_levels_parse_and_strip_marks() {
    assert_eq!(heading("### Heading 3"), Some((3, "Heading 3")));
    assert_eq!(heading("#x"), None, "无空格不算标题");
    assert_eq!(heading("####### too many"), None, "7 级不算标题");
}

/// 任意来源都能渲染：普通段落、空输入、裸文本不 panic。
#[test]
fn renders_arbitrary_source() {
    assert!(render("").is_empty());
    assert!(render("   \n\n  ").is_empty());
    let lines = render("hello **world**");
    assert_eq!(lines_text(&lines), "hello world");
    // 行宽 = 缩进 + 各分段显示宽度。
    let nested = render("- parent\n  - child");
    assert!(nested.iter().any(|line| line.indent == 2), "嵌套缩进生效");
}

/// 代码块正文独立于样例：`render` 只吃传入源码。
#[test]
fn code_block_renders_arbitrary_fence() {
    let lines = render("```python\ndef f():\n    return 1\n```");
    let joined = lines_text(&lines);
    assert!(joined.contains("python"), "语言标记：{joined}");
    assert!(joined.contains("return 1"));
    assert!(!joined.contains("```"));
}

/// 窄宽度下表格：方框网格不超宽、每行等宽、含边界字符、内容不丢。
#[test]
fn wrapped_table_never_exceeds_width_and_stays_aligned() {
    let src = "| 序号 | 名称 | 类别 | 特点 |\n| --- | --- | --- | --- |\n| 1 | DeepSeek | AI助手 | 免费、支持长上下文 |\n| 2 | Python | 编程语言 | 简洁、生态丰富 |\n| 3 | 番茄 | 蔬菜/水果 | 富含维生素C |";
    for width in [24u16, 30, 34, 40, 50, 58, 70] {
        let lines = render_wrapped(src, width);
        assert!(!lines.is_empty(), "宽度 {width} 应有内容");
        let rows: Vec<usize> = lines.iter().map(MdLine::width).collect();
        for (index, row) in rows.iter().enumerate() {
            assert!(
                *row <= usize::from(width),
                "宽度 {width} 第 {index} 行超宽：{row}（{:?}）",
                lines[index].text()
            );
        }
        // 所有表格物理行应等宽（方框列对齐）。
        for (index, row) in rows.iter().enumerate() {
            assert_eq!(
                *row,
                rows[0],
                "宽度 {width} 第 {index} 行宽度与首行不一致：{:?}",
                lines[index].text()
            );
        }
        let joined: String = lines
            .iter()
            .map(MdLine::text)
            .collect::<Vec<_>>()
            .join("\n");
        for ch in ['┌', '┬', '┐', '├', '┼', '┤', '└', '┴', '┘'] {
            assert!(joined.contains(ch), "宽度 {width} 缺少边界字符 {ch}");
        }
        // 内容不丢：表格内所有字符（去空白与方框字符）构成的多重集等于源单元格。
        let flat: String = lines.iter().map(MdLine::text).collect::<Vec<_>>().join("");
        let mut present: std::collections::BTreeMap<char, usize> =
            std::collections::BTreeMap::new();
        for ch in flat
            .chars()
            .filter(|ch| !ch.is_whitespace() && !"─│┌┬┐├┼┤└┴┘".contains(*ch))
        {
            *present.entry(ch).or_default() += 1;
        }
        let mut expected: std::collections::BTreeMap<char, usize> =
            std::collections::BTreeMap::new();
        for cell in [
            "序号",
            "名称",
            "类别",
            "特点",
            "1",
            "DeepSeek",
            "AI助手",
            "免费、支持长上下文",
            "2",
            "Python",
            "编程语言",
            "简洁、生态丰富",
            "3",
            "番茄",
            "蔬菜/水果",
            "富含维生素C",
        ] {
            for ch in cell.chars() {
                *expected.entry(ch).or_default() += 1;
            }
        }
        assert_eq!(present, expected, "宽度 {width} 表格内容多重集不一致");
    }
}

/// 窄宽度下段落按词边界软换行；每行不超过宽度；超长不可断内容按列硬切。
#[test]
fn wrapped_paragraph_breaks_at_word_boundaries() {
    let src = "This is a long English sentence that should wrap at word boundaries.";
    let lines = render_wrapped(src, 20);
    for line in &lines {
        assert!(line.width() <= 20, "超宽：{:?}", line.text());
    }
    let joined = lines_text(&lines);
    // 逐词归一后应还原原句（不拆词、不丢词）。
    let normalize = |value: &str| value.split_whitespace().collect::<Vec<_>>().join(" ");
    assert_eq!(normalize(&joined), normalize(src), "换行不丢词/不拆词");
    // 超长不可断（无空白）内容按宽度切分且不超宽。
    let long = "x".repeat(37);
    let lines = render_wrapped(&long, 10);
    assert_eq!(lines.len(), 4, "37 列按 10 列切成 4 行");
    assert!(lines.iter().all(|line| line.width() <= 10));
}

/// 缩进的多行内容续行保持缩进，且不丢字符。
#[test]
fn wrapped_indented_lines_keep_indent() {
    let src = "- 一级项目内容较长需要换行显示\n  - 二级项目内容也较长需要换行显示";
    let lines = render_wrapped(src, 16);
    assert!(lines.iter().all(|line| line.width() <= 16));
    let flat: String = lines.iter().map(MdLine::text).collect();
    assert!(flat.contains("二级项目内容"));
    assert!(lines.iter().any(|line| line.indent == 2), "二级缩进保留");
}
