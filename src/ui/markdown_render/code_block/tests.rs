use unicode_width::UnicodeWidthChar;

use crate::ui::markdown_render::{
    CodeLineKind, DEFAULT_CODE_LANG, MdLine, extract_code_block, render_wrapped,
    render_wrapped_code_blocks, render_wrapped_literal_rules,
};
use crate::ui::{style, text};

#[test]
fn fenced_code_block_renders_top_border_header_and_left_bar() {
    let src = "```rust\nfn main() {\n    println!(\"hi\");\n}\n```";
    let lines = render_wrapped_code_blocks(src, 30);
    assert_eq!(lines.len(), 5, "1 行顶边 + 1 行头部 + 3 行代码正文");

    // 第 0 行：顶边框铺满 30 列 `▂`，样式 md_read_rule。
    assert_eq!(lines[0].code, Some(CodeLineKind::TopBorder));
    assert_eq!(lines[0].text(), text::CODE_BLOCK_TOP_BORDER.repeat(30));
    assert_eq!(lines[0].width(), 30);
    assert_eq!(lines[0].spans[0].1, style::md_read_rule());

    // 第 1 行：`▎ rust ... [❐] `，语言左侧 1 列空白、按钮右侧 1 列空白。
    assert_eq!(
        lines[1].code,
        Some(CodeLineKind::Header {
            block_index: 0,
            button_cols: (26, 29),
        })
    );
    assert_eq!(lines[1].width(), 30);
    assert!(lines[1].text().starts_with("▎ rust "));
    assert!(lines[1].text().ends_with("[❐] "));
    assert_eq!(lines[1].spans[0], ("▎".to_string(), style::md_read_rule()));
    assert_eq!(lines[1].spans[1], (" ".to_string(), style::text()));
    assert_eq!(lines[1].spans[2], ("rust".to_string(), style::muted()));
    assert_eq!(
        lines[1].spans[4],
        (
            text::TEMPLATE_BUTTON_COPY.to_string(),
            style::template_button()
        )
    );

    // 第 2..=4 行：每行首列为 `▎`（md_read_rule），紧接高亮后的代码。
    for (idx, expected) in ["▎fn main() {", "▎    println!(\"hi\");", "▎}"]
        .into_iter()
        .enumerate()
    {
        let line = &lines[2 + idx];
        assert_eq!(line.code, Some(CodeLineKind::Body));
        assert_eq!(line.text(), expected);
        assert_eq!(line.spans[0], ("▎".to_string(), style::md_read_rule()));
    }
}

#[test]
fn empty_lang_and_indented_code_block_fallback_to_text_label() {
    let src = "```\nplain line\n```\n\n    indented line";
    let lines = render_wrapped_code_blocks(src, 24);
    let headers: Vec<&MdLine> = lines
        .iter()
        .filter(|line| matches!(line.code, Some(CodeLineKind::Header { .. })))
        .collect();
    assert_eq!(headers.len(), 2, "围栏无语言块 + 缩进块各 1 个头部");
    let borders = lines
        .iter()
        .filter(|line| line.code == Some(CodeLineKind::TopBorder))
        .count();
    assert_eq!(borders, 2, "两个代码块各 1 行 `▂` 顶边");
    for (expected_idx, header) in headers.iter().enumerate() {
        assert!(
            header
                .text()
                .starts_with(&format!("▎ {DEFAULT_CODE_LANG} ")),
            "默认语言应显示为 text：{:?}",
            header.text()
        );
        assert!(header.text().ends_with("[❐] "));
        assert_eq!(header.width(), 24);
        assert_eq!(
            header.code,
            Some(CodeLineKind::Header {
                block_index: expected_idx,
                button_cols: (20, 23),
            })
        );
    }
}

#[test]
fn narrow_width_truncates_language_with_ellipsis_and_keeps_copy_button() {
    let src = "```typescript\nconst x = 1;\n```";
    // width = 10：第 0 行顶边（10 列），第 1 行 `▎ ty… [❐] `（按钮右侧 1 列空白）。
    let lines = render_wrapped_code_blocks(src, 10);
    assert_eq!(lines[0].code, Some(CodeLineKind::TopBorder));
    assert_eq!(lines[0].width(), 10);
    assert_eq!(lines[1].text(), "▎ ty… [❐] ");
    assert_eq!(lines[1].width(), 10);
    assert_eq!(
        lines[1].code,
        Some(CodeLineKind::Header {
            block_index: 0,
            button_cols: (6, 9),
        })
    );
}

#[test]
fn soft_wrapped_code_lines_and_blank_lines_prepend_left_bar_on_every_row() {
    let src = "```sh\necho alpha beta gamma delta\n\necho done\n```";
    let lines = render_wrapped_code_blocks(src, 14);
    let body_lines: Vec<&MdLine> = lines
        .iter()
        .filter(|line| line.code == Some(CodeLineKind::Body))
        .collect();
    assert!(body_lines.len() >= 4, "长行应软换行拆成多条视觉行");
    for line in &body_lines {
        assert!(
            line.text().starts_with('▎'),
            "每个视觉行（含续行与空行）首列必须有 `▎`：{:?}",
            line.text()
        );
        assert!(
            line.width() <= 14,
            "左侧 `▎` + 代码不超过 14 列：{}",
            line.width()
        );
    }
    assert!(
        body_lines.iter().any(|line| line.text() == "▎"),
        "空代码行也应保留首列 `▎`"
    );
}

#[test]
fn code_blocks_have_one_blank_line_only_below_surrounding_text() {
    let src = "before\n\n```rust\nlet x = 1;\n```\n\nafter";
    let lines = render_wrapped_code_blocks(src, 30);
    assert_eq!(lines.len(), 6, "正文 / 顶边 / 头部 / 代码 / 空行 / 正文");
    assert_eq!(lines[0].text(), "before");
    assert_eq!(lines[1].code, Some(CodeLineKind::TopBorder), "顶部不补空行");
    assert_eq!(
        lines[1].text(),
        text::CODE_BLOCK_TOP_BORDER.repeat(30),
        "顶边紧接前一段正文"
    );
    assert_eq!(
        lines[2].code,
        Some(CodeLineKind::Header {
            block_index: 0,
            button_cols: (26, 29),
        })
    );
    assert_eq!(lines[3].text(), "▎let x = 1;");
    assert_eq!(lines[4].width(), 0, "代码块后应有一行空行");
    assert_eq!(lines[5].text(), "after");
}

#[test]
fn consecutive_code_blocks_share_a_single_blank_line_and_edges_stay_tight() {
    // 消息以代码块开头：不在顶边前补空行（首行要承载步骤标记）。
    let lines = render_wrapped_code_blocks("```rust\nlet a = 1;\n```", 20);
    assert_eq!(lines.len(), 3, "起始代码块：顶边 / 头部 / 代码");
    assert_eq!(lines[0].code, Some(CodeLineKind::TopBorder));

    // 两个相邻代码块之间只保留一行空白，不叠加两行。
    let lines =
        render_wrapped_code_blocks("```rust\nlet a = 1;\n```\n\n```rust\nlet b = 2;\n```", 20);
    assert_eq!(lines.len(), 7, "顶边/头部/代码/空行/顶边/头部/代码");
    assert_eq!(lines[3].width(), 0, "相邻代码块之间恰好一行空行");
    assert_eq!(lines[6].text(), "▎let b = 2;", "第二个代码块正文紧接头部");

    // 缩进代码块同样只在下方留一行空行。
    let lines = render_wrapped_code_blocks("before\n\n    indented\n\nafter", 20);
    let texts: Vec<String> = lines.iter().map(MdLine::text).collect();
    assert_eq!(texts.len(), 6);
    assert_eq!(texts[0], "before");
    assert_eq!(
        texts[1],
        text::CODE_BLOCK_TOP_BORDER.repeat(20),
        "缩进代码块顶边紧接前一段正文"
    );
    assert!(
        texts[2].starts_with("▎ text ") && texts[2].ends_with("[❐] "),
        "缩进代码块头部：{:?}",
        texts[2]
    );
    assert_eq!(texts[3], "▎indented");
    assert_eq!(texts[4], "", "缩进代码块后应有一行空行");
    assert_eq!(texts[5], "after");
}

#[test]
fn ultra_narrow_header_and_body_stay_within_width() {
    for width in 1..7u16 {
        let lines = render_wrapped_code_blocks("```rust\nx\n```", width);
        for line in &lines {
            assert!(
                line.width() <= usize::from(width),
                "width={width} 时行宽 {} 越界：{:?}",
                line.width(),
                line.text()
            );
        }
        assert!(
            lines
                .iter()
                .any(|line| line.code == Some(CodeLineKind::TopBorder)),
            "width={width} 时仍应保留顶边"
        );
    }
}

#[test]
fn list_continuation_paragraph_is_not_treated_as_indented_code() {
    for src in [
        "1. step\n\n    explanation paragraph\n2. next",
        "- item\n\n    explanation paragraph\n- next",
    ] {
        let lines = render_wrapped_code_blocks(src, 40);
        assert!(
            lines.iter().all(|line| line.code.is_none()),
            "列表续段不得生成代码块 chrome：{src:?} -> {lines:?}"
        );
        assert!(
            lines
                .iter()
                .any(|line| line.text().contains("explanation paragraph")),
            "续段正文必须保留：{src:?} -> {lines:?}"
        );
    }

    let src = "1. step\n\n    explanation\n2. next\n\n```rust\ncode\n```";
    assert_eq!(
        extract_code_block(src, 0).as_deref(),
        Some("code"),
        "列表续段不得占用代码块序号"
    );
}

#[test]
fn code_block_inside_list_keeps_sibling_list_indent() {
    let src = "- a\n  - b\n    ```sh\n    cmd\n    ```\n  - c";
    let lines = render_wrapped_code_blocks(src, 40);
    let sibling = lines
        .iter()
        .find(|line| line.text() == "• c")
        .expect("sibling list item");
    assert_eq!(
        sibling.indent, 2,
        "列表内代码块之后的兄弟项仍保持二级缩进：{lines:?}"
    );
    assert_eq!(
        extract_code_block(src, 0).as_deref(),
        Some("    cmd"),
        "列表内围栏代码块提取保持原始行"
    );
}

#[test]
fn header_sanitizes_control_chars_in_language_label() {
    let src = "```ru\tst\nlet x = 1;\n```";
    let lines = render_wrapped_code_blocks(src, 20);
    let header = &lines[1];
    assert!(
        !header.text().chars().any(char::is_control),
        "语言标签不得把控制字符带进渲染行：{:?}",
        header.text()
    );
    assert!(
        header.text().starts_with("▎ ru st "),
        "制表符应替换为空格后再参与排版：{:?}",
        header.text()
    );
    assert_eq!(header.width(), 20);
    assert_eq!(
        header.code,
        Some(CodeLineKind::Header {
            block_index: 0,
            button_cols: (16, 19),
        })
    );
}

#[test]
fn header_drops_zero_width_language_chars() {
    let src = "```e\u{0301}\nlet x = 1;\n```";
    let lines = render_wrapped_code_blocks(src, 20);
    let header = &lines[1];
    assert!(
        header.text().chars().all(|ch| ch.width().unwrap_or(0) > 0),
        "语言标签不得把零宽/组合字符带进渲染行：{:?}",
        header.text()
    );
    assert!(
        header.text().starts_with("▎ e "),
        "组合字符应被丢弃后再参与排版：{:?}",
        header.text()
    );
    assert_eq!(
        header.code,
        Some(CodeLineKind::Header {
            block_index: 0,
            button_cols: (16, 19),
        })
    );
}

#[test]
fn ultra_narrow_wide_code_char_stays_within_width() {
    for width in 1..=3u16 {
        let lines = render_wrapped_code_blocks("```\n日\n```", width);
        for line in &lines {
            assert!(
                line.width() <= usize::from(width),
                "width={width} 时宽字符行宽 {} 越界：{:?}",
                line.width(),
                line.text()
            );
        }
    }
}

#[test]
fn width_one_code_line_does_not_explode_into_per_char_rows() {
    let lines = render_wrapped_code_blocks("```\nabcdefghij\n```", 1);
    let bodies: Vec<&MdLine> = lines
        .iter()
        .filter(|line| line.code == Some(CodeLineKind::Body))
        .collect();
    assert_eq!(bodies.len(), 1, "宽度 1 时一条逻辑代码行只留一条 `▎`");
    assert_eq!(bodies[0].text(), "▎");
}

#[test]
fn code_body_drops_zero_width_chars() {
    let lines = render_wrapped_code_blocks("```\n👨\u{200d}👩\u{200d}👧 x\n```", 10);
    let body = lines
        .iter()
        .find(|line| line.code == Some(CodeLineKind::Body))
        .expect("body");
    assert!(
        body.text().chars().all(|ch| ch.width().unwrap_or(0) > 0),
        "正文不得保留零宽/ZWJ 字符：{:?}",
        body.text()
    );
    assert!(
        body.text().contains('x'),
        "正文内容不得被裁掉：{:?}",
        body.text()
    );
    assert!(body.width() <= 10);
}

#[test]
fn four_backtick_fence_is_not_closed_by_inner_three_backticks() {
    let src = "````markdown\n```rust\nlet a = 1;\n```\n````";
    let lines = render_wrapped_code_blocks(src, 40);
    let bodies: Vec<&MdLine> = lines
        .iter()
        .filter(|line| line.code == Some(CodeLineKind::Body))
        .collect();
    assert_eq!(bodies.len(), 3, "内层三反引号是代码正文：{lines:?}");
    assert_eq!(bodies[0].text(), "▎```rust");
    assert_eq!(bodies[1].text(), "▎let a = 1;");
    assert_eq!(bodies[2].text(), "▎```");
    assert_eq!(
        extract_code_block(src, 0).as_deref(),
        Some("```rust\nlet a = 1;\n```"),
        "四反引号围栏复制应保留内层三反引号"
    );
}

#[test]
fn extreme_widths_do_not_panic_or_overflow() {
    for width in [0u16, 1, 2, u16::MAX] {
        let limit = usize::from(width.max(1));
        let lines = render_wrapped_code_blocks("```rust\n日\n```", width);
        for line in &lines {
            assert!(
                line.width() <= limit,
                "width={width} 时行宽 {} 越界：{:?}",
                line.width(),
                line.text()
            );
        }
    }
}

#[test]
fn unlimited_width_code_block_keeps_body() {
    let lines = render_wrapped_code_blocks("```rust\nlet x = 1;\n```", u16::MAX);
    assert!(lines.iter().any(|line| line.text() == "▎let x = 1;"));
}

#[test]
fn unclosed_fence_trailing_newline_is_not_an_extra_code_line() {
    let src = "```rust\nlet x = 1;\n";
    let lines = render_wrapped_code_blocks(src, 30);
    assert_eq!(lines.len(), 3, "顶边 / 头部 / 1 行正文");
    assert_eq!(lines[2].text(), "▎let x = 1;");
    assert_eq!(
        extract_code_block(src, 0).as_deref(),
        Some("let x = 1;"),
        "复制不得带末尾换行"
    );

    let src = "```rust\n";
    let lines = render_wrapped_code_blocks(src, 30);
    assert_eq!(lines.len(), 2, "空围栏只有顶边与头部");
    assert_eq!(extract_code_block(src, 0).as_deref(), Some(""));

    let src = "```\n\n";
    let lines = render_wrapped_code_blocks(src, 30);
    assert_eq!(lines.len(), 3, "真正的空代码行必须保留");
    assert_eq!(lines[2].text(), "▎");
    assert_eq!(extract_code_block(src, 0).as_deref(), Some(""));
}

#[test]
fn code_lines_expand_tabs_and_replace_control_chars() {
    let src = "```rust\n\tlet x = 1;\r\n\u{1}bad\n```";
    let lines = render_wrapped_code_blocks(src, 40);
    let bodies: Vec<&MdLine> = lines
        .iter()
        .filter(|line| line.code == Some(CodeLineKind::Body))
        .collect();
    assert_eq!(bodies.len(), 2);
    assert_eq!(
        bodies[0].text(),
        "▎    let x = 1;",
        "制表符按 4 列展开，`\\r` 替换为空格（行尾空白不显示）"
    );
    assert_eq!(bodies[1].text(), "▎ bad", "其他控制字符替换为空格");
    for line in &bodies {
        assert!(line.text().chars().all(|ch| !ch.is_control()));
    }
}

#[test]
fn plain_render_wrapped_keeps_code_blocks_without_chrome_for_thinking_and_tools() {
    let src = "```rust\nlet x = 1;\n```";
    for lines in [
        render_wrapped(src, 30),
        render_wrapped_literal_rules(src, 30),
    ] {
        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0].code, None);
        assert_eq!(lines[0].text(), "let x = 1;");
    }
}

#[test]
fn extract_code_block_strips_crlf_line_endings() {
    let src = "```rust\r\nlet x = 1;\r\n\r\nlet y = 2;\r\n```";
    assert_eq!(
        extract_code_block(src, 0).as_deref(),
        Some("let x = 1;\n\nlet y = 2;"),
        "围栏代码块复制必须剥离 CRLF 的 `\\r`"
    );

    let src = "    indented();\r\n";
    assert_eq!(
        extract_code_block(src, 0).as_deref(),
        Some("indented();"),
        "缩进代码块复制必须剥离 CRLF 的 `\\r`"
    );
}

#[test]
fn extract_code_block_index_matches_render_after_list_quote_and_table() {
    let src = "Intro\n\n- item\n\n> quoted\n\n| a | b |\n| - | - |\n| c | d |\n\n```rust\nfirst\n```\n\n    indented\n";
    let headers: Vec<usize> = render_wrapped_code_blocks(src, 40)
        .iter()
        .filter_map(|line| match line.code {
            Some(CodeLineKind::Header { block_index, .. }) => Some(block_index),
            _ => None,
        })
        .collect();
    assert_eq!(headers, vec![0, 1], "列表/引用/表格之后的代码块序号");
    assert_eq!(extract_code_block(src, 0).as_deref(), Some("first"));
    assert_eq!(extract_code_block(src, 1).as_deref(), Some("indented"));
}

#[test]
fn extract_code_block_returns_raw_unwrapped_code_for_fenced_indented_and_unclosed_blocks() {
    let src = [
        "Intro",
        "```rust",
        "fn first() {",
        "    let a = 1;",
        "}",
        "```",
        "",
        "    indented_one();",
        "",
        "    indented_two();",
        "",
        "```",
        "```",
        "",
        "```python",
        "def streaming():",
        "    return 42",
    ]
    .join("\n");

    assert_eq!(
        extract_code_block(&src, 0).as_deref(),
        Some("fn first() {\n    let a = 1;\n}")
    );
    assert_eq!(
        extract_code_block(&src, 1).as_deref(),
        Some("indented_one();\n\nindented_two();")
    );
    assert_eq!(
        extract_code_block(&src, 2).as_deref(),
        Some(""),
        "空围栏代码块返回空串"
    );
    assert_eq!(
        extract_code_block(&src, 3).as_deref(),
        Some("def streaming():\n    return 42"),
        "未闭合流式代码块返回已生成代码行"
    );
    assert_eq!(extract_code_block(&src, 4), None);
}

#[test]
fn top_level_blocks_have_single_blank_line_between_paragraphs_headings_quotes_rules_and_tables() {
    // 1. 段落单换行紧邻，双换行/多空行合并为 1 行空行，首尾空行不产生多余空行。
    let para_src = "\n\n第一段第一行\n第一段第二行\n\n\n\n第二段正文\n\n";
    let para_texts: Vec<String> = render_wrapped_code_blocks(para_src, 40)
        .iter()
        .map(MdLine::text)
        .collect();
    assert_eq!(
        para_texts,
        vec![
            "第一段第一行".to_string(),
            "第一段第二行".to_string(),
            String::new(),
            "第二段正文".to_string(),
        ]
    );

    // 2. 标题、引用、分割线、表格即便源码未写空行，与前后顶层块之间也恒隔 1 行空行；
    // 连续 `>` 行保持紧邻，隔空行的两个 `>` 块之间空 1 行。
    let struct_src = [
        "开场段落",
        "## 小标题",
        "标题下紧跟段落",
        "> 引用第一行",
        "> 引用第二行",
        "",
        "> 独立第二段引用",
        "---",
        "| 列一 | 列二 |",
        "| --- | --- |",
        "| 值一 | 值二 |",
        "表后收尾段落",
    ]
    .join("\n");
    let struct_lines = render_wrapped_code_blocks(&struct_src, 40);
    let texts: Vec<String> = struct_lines.iter().map(MdLine::text).collect();

    let heading_idx = texts
        .iter()
        .position(|t| t == "小标题")
        .expect("heading line");
    assert_eq!(texts[heading_idx - 1], "", "标题上方空 1 行");
    assert_eq!(texts[heading_idx + 1], "", "标题下方空 1 行");

    let q1_idx = texts
        .iter()
        .position(|t| t == "> 引用第一行")
        .expect("quote 1");
    assert_eq!(texts[q1_idx - 1], "", "引用块上方空 1 行");
    assert_eq!(
        texts[q1_idx + 1],
        "> 引用第二行",
        "连续引用行之间不得插空行"
    );
    assert_eq!(texts[q1_idx + 2], "", "两个独立引用块之间空 1 行");
    assert_eq!(texts[q1_idx + 3], "> 独立第二段引用");
    assert_eq!(texts[q1_idx + 4], "", "引用块与分割线之间空 1 行");
    assert!(texts[q1_idx + 5].starts_with('─'), "分割线行");
    assert_eq!(texts[q1_idx + 6], "", "分割线与表格之间空 1 行");
    assert!(texts[q1_idx + 7].starts_with('┌'), "表格顶边框");

    let end_para_idx = texts
        .iter()
        .position(|t| t == "表后收尾段落")
        .expect("trailing paragraph");
    assert_eq!(texts[end_para_idx - 1], "", "表格下方与段落之间空 1 行");
}

#[test]
fn list_spacing_distinguishes_tight_loose_nested_exit_and_kind_switch() {
    // 1. 紧凑列表（含窄屏软折行成多视觉行的项）：项间 0 空行。
    let tight_src =
        "- 第一条比较长的列表项在窄宽度下会自动软折行\n- 第二条紧凑列表项\n- 第三条紧凑列表项";
    let tight_lines = render_wrapped_code_blocks(tight_src, 18);
    assert!(
        tight_lines.len() >= 4,
        "第一条列表项应软折行成多行：{tight_lines:?}"
    );
    assert!(
        tight_lines.iter().all(|line| !line.text().is_empty()),
        "紧凑列表项（即使软折行）之间不得插入空行：{tight_lines:?}"
    );

    // 2. 松散列表（源码含 `\n\n`）：项间保留 1 行空行。
    let loose_src = "- 松散项一\n\n- 松散项二\n\n- 松散项三";
    let loose_texts: Vec<String> = render_wrapped_code_blocks(loose_src, 40)
        .iter()
        .map(MdLine::text)
        .collect();
    assert_eq!(
        loose_texts,
        vec![
            "• 松散项一".to_string(),
            String::new(),
            "• 松散项二".to_string(),
            String::new(),
            "• 松散项三".to_string(),
        ]
    );

    // 3. 父列表项 → 嵌套子列表保持 0 空行；子列表结束退回外层列表项自动补 1 行空行。
    let nested_src = "1. 第一大点\n   - 子项 A\n   - 子项 B\n2. 第二大点";
    let nested_texts: Vec<String> = render_wrapped_code_blocks(nested_src, 40)
        .iter()
        .map(MdLine::text)
        .collect();
    assert_eq!(
        nested_texts,
        vec![
            "1. 第一大点".to_string(),
            "• 子项 A".to_string(),
            "• 子项 B".to_string(),
            String::new(),
            "2. 第二大点".to_string(),
        ]
    );

    // 4. 同级不同类型列表切换（无序 ↔ 有序 ↔ 任务）即使源码无空行也自动补 1 行空行。
    let switch_src = "- 无序项\n1. 有序项\n- [x] 任务项";
    let switch_texts: Vec<String> = render_wrapped_code_blocks(switch_src, 40)
        .iter()
        .map(MdLine::text)
        .collect();
    assert_eq!(
        switch_texts,
        vec![
            "• 无序项".to_string(),
            String::new(),
            "1. 有序项".to_string(),
            String::new(),
            "[x] 任务项".to_string(),
        ]
    );
}

#[test]
fn assistant_report_layout_separates_sections_lists_and_paragraphs_while_non_assistant_stays_compact()
 {
    let src = [
        "已通过 `codegraph` 与 `codebase-memory-mcp` 核查完毕。",
        "",
        "## 核查结论",
        "",
        "**1. AGENTS.md 的两条描述更准确，允许保留**",
        "",
        "两点保留：",
        "- **`app/` 不是完全无 IO**：保留“主状态更新保持纯内存”的表述。",
        "- **`src/db/mod.rs` 确实承载连接打开**：保留。",
        "",
        "**2. 规范红线与单文件体量核查**",
        "",
        "全仓目前共 **19 个生产文件 > 1000 行**。",
        "",
        "测试文件同样已与规范脱节：单文件超 1500 行的测试共 **10 个**。",
    ]
    .join("\n");

    let assistant_texts: Vec<String> = render_wrapped_code_blocks(&src, 80)
        .iter()
        .map(MdLine::text)
        .collect();
    assert_eq!(
        assistant_texts,
        vec![
            "已通过 codegraph 与 codebase-memory-mcp 核查完毕。".to_string(),
            String::new(),
            "核查结论".to_string(),
            String::new(),
            "1. AGENTS.md 的两条描述更准确，允许保留".to_string(),
            String::new(),
            "两点保留：".to_string(),
            String::new(),
            "• app/ 不是完全无 IO：保留“主状态更新保持纯内存”的表述。".to_string(),
            "• src/db/mod.rs 确实承载连接打开：保留。".to_string(),
            String::new(),
            "2. 规范红线与单文件体量核查".to_string(),
            String::new(),
            "全仓目前共 19 个生产文件 > 1000 行。".to_string(),
            String::new(),
            "测试文件同样已与规范脱节：单文件超 1500 行的测试共 10 个。".to_string(),
        ]
    );

    // 思考块与工具/汇报卡片（code_chrome == false）保持紧凑无空行。
    for compact_lines in [
        render_wrapped(&src, 80),
        render_wrapped_literal_rules(&src, 80),
    ] {
        assert!(
            compact_lines.iter().all(|line| !line.text().is_empty()),
            "非助手正文不得插入块间空行：{compact_lines:?}"
        );
    }

    // P1-3：列表项含松散续段（`\n\n` 引导）后进入下一条同级列表项，续段上下均空 1 行。
    let loose_cont = "1. step\n\n    explanation\n2. next";
    let loose_texts: Vec<String> = render_wrapped_code_blocks(loose_cont, 80)
        .iter()
        .map(MdLine::text)
        .collect();
    assert_eq!(
        loose_texts,
        vec![
            "1. step".to_string(),
            String::new(),
            "explanation".to_string(),
            String::new(),
            "2. next".to_string(),
        ]
    );
}
