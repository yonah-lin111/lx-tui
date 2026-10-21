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
fn code_block_shows_body_with_syntax_colors() {
    // 代码块只画正文：没有 `language:` 头、没有复制按钮、没有横线（用户要求移除容器装饰），
    // 但语法高亮照旧。
    let lines = render(sample());
    let joined = lines_text(&lines);
    assert!(!joined.contains("language:"), "不再显示语言头：{joined}");
    assert!(!joined.contains("[❐]"), "不再有复制按钮：{joined}");
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
    assert!(!joined.contains("python"), "不再显示语言标记：{joined}");
    assert!(joined.contains("return 1"));
    assert!(!joined.contains("```"), "围栏不显示：{joined}");
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

/// 列表项换行：标记后不留多余空格（`*   text` 的三个空格是排版对齐），续行悬挂缩进
/// 对齐正文起始列，且不出现 `•` 独占一行的孤儿标记。
#[test]
fn wrapped_list_items_hang_indent_under_the_text() {
    let src =
        "*   代码生成：支持 Python、JavaScript、C++、Java、Go、SQL、HTML/CSS 等多种主流编程语言。";
    let lines = render_wrapped(src, 40);
    assert!(lines.len() > 1, "确实换行");
    assert!(
        lines[0].text().starts_with("• 代码生成"),
        "标记后只留一个空格：{:?}",
        lines[0].text()
    );
    assert!(
        !lines.iter().any(|line| line.text().trim() == "•"),
        "不出现孤儿标记"
    );
    assert!(
        lines[1..].iter().all(|line| line.indent == 2),
        "无序列表续行悬挂 2 列：{:?}",
        lines.iter().map(|line| line.indent).collect::<Vec<_>>()
    );

    // 有序列表按标记宽度悬挂（`12. ` = 4 列）。
    let lines = render_wrapped(
        "12. 这是一个很长很长的有序列表项目，需要在窄宽度下换行显示。",
        20,
    );
    assert!(lines.len() > 1);
    assert!(
        lines[1..].iter().all(|line| line.indent == 4),
        "有序列表按标记宽度悬挂：{:?}",
        lines.iter().map(|line| line.indent).collect::<Vec<_>>()
    );
}

/// 三级及更深的列表不会被吞成「缩进代码块」：标记保留、每级 2 列。
#[test]
fn deeply_nested_lists_keep_markers_and_two_column_levels() {
    let src = "- 一级\n  - 二级\n    - 三级\n      - 四级";
    let lines = render_wrapped(src, 40);
    assert_eq!(
        lines.iter().map(|line| line.indent).collect::<Vec<_>>(),
        vec![0, 2, 4, 6],
        "每级 2 列"
    );
    assert!(
        lines.iter().all(|line| line.text().starts_with("• ")),
        "全部按列表项渲染（`-` 不再漏进正文）：{lines:?}"
    );
}

/// 有序列表按 `1. ` 的 3 空格对齐时，层级仍换算成均匀的 2 列。
#[test]
fn mixed_indent_steps_map_to_uniform_levels() {
    let src = "1. 一级\n   1. 二级\n      1. 三级";
    let lines = render_wrapped(src, 40);
    assert_eq!(
        lines.iter().map(|line| line.indent).collect::<Vec<_>>(),
        vec![0, 2, 4]
    );
    assert!(lines[2].text().starts_with("1. 三级"));
}

/// 真正的缩进代码块仍按代码渲染（4 空格 + 非列表内容）。
#[test]
fn indented_code_blocks_still_render_as_code() {
    let lines = render("    let x = 1;\n    let y = 2;");
    assert!(lines.len() >= 2, "两行都在代码块里：{lines:?}");
    let text: String = lines
        .iter()
        .map(MdLine::text)
        .collect::<Vec<_>>()
        .join("\n");
    assert!(text.contains("let x = 1;"), "{text}");
    assert!(text.contains("let y = 2;"), "{text}");
}

/// 列表结束后缩进档位复位：段落之后的同级列表仍从第 0 列开始。
#[test]
fn list_indent_stack_resets_after_other_blocks() {
    let src = "- 一级\n  - 二级\n段落\n- 另一个列表";
    let lines = render_wrapped(src, 40);
    assert_eq!(lines[1].indent, 2, "嵌套仍是二级");
    assert_eq!(lines[3].indent, 0, "段落之后复位到顶层：{lines:?}");
}

/// 引用换行：续行对齐引用正文（`> ` 之后），不丢字符。
#[test]
fn wrapped_quote_hangs_under_the_text() {
    let lines = render_wrapped("> 这是一段很长很长的引用文字，需要在窄宽度下换行显示。", 20);
    assert!(lines.len() > 1, "确实换行");
    assert!(
        lines[1..].iter().all(|line| line.indent == 2),
        "引用续行对齐正文：{:?}",
        lines.iter().map(|line| line.indent).collect::<Vec<_>>()
    );
    let flat: String = lines.iter().map(MdLine::text).collect();
    assert!(flat.contains("换行显示"), "不丢字符：{flat}");
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

/// 表格单元格内的行内标记被折叠（不出现 `**`），且加粗样式落在正文分段上。
#[test]
fn table_cells_collapse_inline_markers() {
    let src = "| 类别 | 说明 |\n| --- | --- |\n| **文本对话** | `代码` 与 [链接](https://x.io) |";
    let lines = render(src);
    let joined: String = lines
        .iter()
        .map(MdLine::text)
        .collect::<Vec<_>>()
        .join("\n");
    assert!(!joined.contains("**"), "单元格不应泄漏 ** ：{joined}");
    assert!(!joined.contains('`'), "单元格不应泄漏反引号：{joined}");
    assert!(joined.contains("文本对话"));
    assert!(joined.contains("链接"));
    assert!(joined.contains("(https://x.io)"), "链接显示 URL：{joined}");
    // 加粗样式确实落在「文本对话」分段上。
    let bold_cell = lines
        .iter()
        .find(|line| line.text().contains("文本对话"))
        .expect("cell line");
    let bold = style::md_read_strong();
    assert!(
        bold_cell
            .spans
            .iter()
            .any(|(value, style)| value.contains("文本对话") && *style == bold),
        "文本对话应为加粗样式：{:?}",
        bold_cell.spans
    );
}

/// 单元格内 `<br>` 变体换行成多物理行，且行数与列内容并排正确。
#[test]
fn table_cells_split_on_br() {
    let src = "| A | B |\n| --- | --- |\n| 一<br>二<br/>三 | **X**<br />Y |";
    let lines = render(src);
    let joined: String = lines
        .iter()
        .map(MdLine::text)
        .collect::<Vec<_>>()
        .join("\n");
    assert!(!joined.contains("<br"), "不应泄漏 `<br>` 标记：{joined}");
    assert!(joined.contains('一') && joined.contains('二') && joined.contains('三'));
    assert!(joined.contains('X') && joined.contains('Y'));
    // 行数：A 列 3 行 → 该逻辑行占 3 条物理行。
    let rows: Vec<&MdLine> = lines
        .iter()
        .filter(|line| line.text().starts_with('│'))
        .collect();
    // 表头 1 + 数据 3 = 4 条内容行（加分割线共 4 条 `│` 起始行中的 3 条为数据）。
    assert!(rows.len() >= 4, "应有表头 + 3 条数据物理行：{joined}");
}

/// `<br>` 单元格的列宽按最宽行测量（不被标记撑大）。
#[test]
fn br_cell_width_uses_visible_text() {
    let lines = render("| A |\n| --- |\n| 一<br>二二二 |");
    let joined: String = lines
        .iter()
        .map(MdLine::text)
        .collect::<Vec<_>>()
        .join("\n");
    // 「二二二」宽 6 列；`<br>` 不应计入宽度。
    assert!(!joined.contains('<'), "无标记残留：{joined}");
    assert!(joined.contains("二二二"));
}

/// 表格：表头与每条数据行之间都有水平分割线（`├`），末行与下边框合并。
#[test]
fn table_rows_have_horizontal_separators() {
    let src = "| A | B |\n| --- | --- |\n| 1 | 2 |\n| 3 | 4 |\n| 5 | 6 |";
    let lines = render(src);
    let text: Vec<String> = lines.iter().map(MdLine::text).collect();
    // 期望：┌ / 表头 / ├ / 1 / ├ / 3 / ├ / 5 / └
    let seps = text.iter().filter(|line| line.starts_with("├")).count();
    assert_eq!(
        seps, 3,
        "表头后 + 每条数据行后（末行与 └ 合并）共 3 条分割线：{text:#?}"
    );
    assert!(text.iter().any(|line| line.starts_with('┌')));
    assert!(text.iter().any(|line| line.starts_with('└')));
    // 数据行数 = 3；分割线不紧邻（每行数据之间）。
    let data_rows = text.iter().filter(|line| line.starts_with('│')).count();
    assert_eq!(data_rows, 4, "表头 1 + 数据 3 = 4 条内容行");
    // 相邻两条 `├` 之间必有一条数据行。
    for window in text.windows(2) {
        assert!(
            !(window[0].starts_with('├') && window[1].starts_with('├')),
            "不应出现相邻分割线：{text:#?}"
        );
    }
}

/// 单条数据行时：表头后一条分割线，末行与下边框合并（不重复分割）。
#[test]
fn single_body_row_keeps_one_separator() {
    let text: Vec<String> = render("| A |\n| --- |\n| 1 |")
        .iter()
        .map(MdLine::text)
        .collect();
    assert_eq!(text.iter().filter(|line| line.starts_with("├")).count(), 1);
    assert_eq!(
        text.last().map(|line| line.starts_with('└')),
        Some(true),
        "末行即下边框：{text:#?}"
    );
}

/// 空首列的续行不画分割线：同组多行视觉合并为一块（对齐 markdown「同组多行、首列留空」写法）。
#[test]
fn continuation_rows_merge_into_one_block() {
    let src = "| 类别 | 项 |\n| --- | --- |\n| A | 1 |\n|  | 2 |\n|  | 3 |\n| B | 4 |";
    let text: Vec<String> = render(src).iter().map(MdLine::text).collect();
    // 分割线：表头后 1 条 + A 组后 1 条 = 2 条（B 组只有一项，末行与下边框合并）。
    let seps = text.iter().filter(|line| line.starts_with("├")).count();
    assert_eq!(seps, 2, "续行不产生分割线：{text:#?}");
    // 承接行之间没有分割线：A 组三行紧邻。
    let a_first = text
        .iter()
        .position(|line| line.contains("A"))
        .expect("A 行");
    let b_last = text
        .iter()
        .position(|line| line.contains('3'))
        .expect("3 行");
    for line in &text[a_first..=b_last] {
        assert!(
            !line.starts_with("├"),
            "同组续行之间不应有分割线：{text:#?}"
        );
    }
    // 内容仍完整。
    assert!(text.iter().any(|line| line.contains('1')));
    assert!(text.iter().any(|line| line.contains('3')));
    assert!(text.iter().any(|line| line.contains('B')));
}

/// 每行首列都非空时：每条数据行之间都有分割线（保持上一版行为）。
#[test]
fn non_continuation_rows_keep_separators() {
    let src = "| A | B |\n| --- | --- |\n| 1 | x |\n| 2 | y |\n| 3 | z |";
    let text: Vec<String> = render(src).iter().map(MdLine::text).collect();
    let seps = text.iter().filter(|line| line.starts_with("├")).count();
    assert_eq!(seps, 3, "表头后 + 每行间共 3 条：{text:#?}");
}

#[test]
fn inline_code_uses_foreground_only_no_background() {
    // `xxx` 恢复为纯前景色（不铺背景）。
    let spans = inline_spans("before `code` after");
    let code = spans
        .iter()
        .find(|(value, _)| value == "code")
        .expect("行内代码 span");
    assert_eq!(code.1.bg, None, "行内代码不铺背景");
    assert!(code.1.fg.is_some(), "行内代码仍有前景色");
}

#[test]
fn list_markers_are_colored_while_body_stays_plain() {
    // 无序 / 有序列表：符号上结构强调色，正文保持原色（只强调标记）。
    for source in ["- first", "1. first"] {
        let lines = render(source);
        let spans = &lines[0].spans;
        let marker = &spans[0];
        assert!(
            marker.1.fg.is_some(),
            "{source:?} 的列表符号应上色: {:?}",
            marker
        );
        let body = spans
            .iter()
            .find(|(value, _)| value.contains("first"))
            .expect("正文 span");
        assert_eq!(body.1.fg, None, "{source:?} 的正文保持原色");
    }
    // 无序与有序用同一结构强调色，列表结构观感一致。
    let bullet = render("- x")[0].spans[0].1.fg;
    let ordered = render("1. x")[0].spans[0].1.fg;
    assert_eq!(bullet, ordered, "两种列表同一强调色");
}

#[test]
fn nested_list_markers_stay_colored() {
    // 缩进后的嵌套项同样保留符号高亮（缩进列不着色）。
    let lines = render("- top\n  - nested");
    let nested = lines
        .iter()
        .find(|line| line.text().contains("nested"))
        .expect("nested line");
    let marker = nested
        .spans
        .iter()
        .find(|(value, _)| value.contains('•'))
        .expect("nested bullet");
    assert!(marker.1.fg.is_some(), "嵌套列表符号同样上色");
}

#[test]
fn code_block_renders_body_only() {
    // 围栏消失、正文原样、无任何容器装饰。
    let lines = render("```js\nconst a = 1;\n```");
    assert_eq!(lines.len(), 1, "只有一行正文：{lines:?}");
    assert_eq!(lines[0].text(), "const a = 1;");
    assert!(lines.iter().all(|line| !line.text().contains('─')));
    assert!(lines.iter().all(|line| !line.text().contains("language:")));
    assert!(lines.iter().all(|line| !line.text().contains("[❐]")));
}

/// 无语言代码块不做假高亮：关键字不被染色。
#[test]
fn text_language_label_does_not_enable_syntax_highlighting() {
    let lines = render("```\nif (x) return 1;\n```");
    let body = lines.first().expect("code body");
    let highlighted = body.spans.iter().filter(|(_, span_style)| {
        *span_style == style::code_keyword()
            || *span_style == style::code_string()
            || *span_style == style::code_number()
            || *span_style == style::code_comment()
            || *span_style == style::code_type()
    });
    assert_eq!(
        highlighted.count(),
        0,
        "无语言块不做语法高亮（否则 `if`/`return` 会被当关键字染色）: {:?}",
        body.spans
    );
    assert_eq!(body.text(), "if (x) return 1;", "正文原样保留");
}

/// 缩进代码块（4 空格、无语言）同样只剩正文。
#[test]
fn indented_code_block_renders_body_only() {
    let lines = render("    let x = 1;");
    assert_eq!(lines.len(), 1);
    assert_eq!(lines[0].text(), "let x = 1;");
}
