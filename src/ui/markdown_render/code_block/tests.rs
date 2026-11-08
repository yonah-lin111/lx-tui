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
