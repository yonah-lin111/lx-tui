//! 单元测试：markdown 渲染器覆盖全部格式、标记折叠与不 panic。

use super::*;

use crate::app::ui_gallery::AgentStepId;
use crate::ui::overlay::ui_gallery::agent::{
    SampleLine, agent_appearance_size, render_agent_appearance,
};

/// 把一组渲染行拼成纯文本（用于断言可见内容）。
fn lines_text(lines: &[SampleLine]) -> String {
    lines
        .iter()
        .map(|line| {
            line.spans
                .iter()
                .map(|(value, _)| value.as_str())
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn sample_document_covers_every_block_and_inline_format() {
    let sample = text::agent::MD_SAMPLE;
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
    let lines = markdown_lines((12_400, 1_200, 8_100));
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
    // 代码正文在。
    assert!(joined.contains("fn main() {}"));
    // 表格内容在。
    assert!(joined.contains("Left") && joined.contains("Center") && joined.contains("Right"));
    // 链接文字与 URL 都在。
    assert!(joined.contains("OpenAI"));
    assert!(joined.contains("https://openai.com"));
    // 图片 alt 在。
    assert!(joined.contains("logo"));
    // 最后一行是 token 页脚。
    let footer = lines_text(&lines[lines.len() - 1..]);
    assert!(footer.contains(text::agent::TOKEN_IN), "{footer}");
}

#[test]
fn headings_hide_hashes() {
    let joined = lines_text(&markdown_lines((1, 1, 1)));
    assert!(!joined.contains("# Heading 1"), "应隐藏 `#`");
    assert!(joined.contains("Heading 1"));
}

#[test]
fn table_is_bordered() {
    let joined = lines_text(&markdown_lines((1, 1, 1)));
    assert!(joined.contains('┌'), "表格应有方框");
    assert!(joined.contains('┘'));
}

#[test]
fn code_block_wraps_in_a_bar_and_shows_language() {
    let joined = lines_text(&markdown_lines((1, 1, 1)));
    assert!(joined.contains(text::agent::MD_LANG), "应显示代码语言标记");
    assert!(joined.contains('│'), "代码块应带直角竖条");
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

#[test]
fn render_does_not_panic_for_markdown_step_on_all_sizes() {
    for (width, height) in [(6, 4), (20, 6), (52, 90), (120, 60)] {
        let mut buf =
            ratatui::buffer::Buffer::empty(ratatui::layout::Rect::new(0, 0, width, height));
        let area = buf.area;
        render_agent_appearance(&mut buf, area, AgentStepId::Markdown);
    }
}

#[test]
fn markdown_appearance_size_is_positive_and_clamped() {
    let available = ratatui::layout::Rect::new(0, 0, 80, 60);
    let (width, height) = agent_appearance_size(AgentStepId::Markdown, available);
    assert!(width >= 6);
    assert!(height > 0 && height <= available.height);
    let tiny = ratatui::layout::Rect::new(0, 0, 80, 3);
    let (_, height) = agent_appearance_size(AgentStepId::Markdown, tiny);
    assert_eq!(height, 3);
}
