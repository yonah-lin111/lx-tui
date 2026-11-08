use crate::ui::markdown_render::{
    CodeLineKind, DEFAULT_CODE_LANG, MdLine, extract_code_block, render_wrapped,
    render_wrapped_code_blocks, render_wrapped_literal_rules,
};
use crate::ui::{style, text};

#[test]
fn fenced_code_block_renders_four_sided_borders_header_and_copy_button() {
    let src = "```rust\nfn main() {\n    println!(\"hi\");\n}\n```";
    let lines = render_wrapped_code_blocks(src, 30);
    assert_eq!(
        lines.len(),
        6,
        "1 行顶边 + 1 行头部 + 3 行代码正文 + 1 行底边"
    );

    // 第 0 行：顶边框铺满 30 列 `▂`，样式 template_block_bar（品牌柔粉 Indexed 218）。
    assert_eq!(lines[0].code, Some(CodeLineKind::TopBorder));
    assert_eq!(lines[0].text(), text::PROMPT_BLOCK_TOP_BORDER.repeat(30));
    assert_eq!(lines[0].width(), 30);
    assert_eq!(lines[0].spans[0].1, style::template_block_bar());

    // 第 1 行：首列 `▎` + `rust` + 空白填充 + 右侧 `[❐]` + 末列 `🮇`。
    assert_eq!(
        lines[1].code,
        Some(CodeLineKind::Header {
            block_index: 0,
            button_cols: (26, 29),
        })
    );
    assert_eq!(lines[1].width(), 30);
    assert!(lines[1].text().starts_with("▎rust"));
    assert!(lines[1].text().ends_with("[❐]🮇"));
    assert_eq!(
        lines[1].spans[0],
        ("▎".to_string(), style::template_block_bar())
    );
    assert_eq!(lines[1].spans[1], ("rust".to_string(), style::muted()));
    assert!(lines[1].spans.contains(&(
        text::TEMPLATE_BUTTON_COPY.to_string(),
        style::template_button()
    )));
    assert_eq!(
        lines[1].spans.last().cloned(),
        Some((
            text::CODE_BLOCK_RIGHT_BAR.to_string(),
            style::template_block_bar()
        ))
    );

    // 第 2..=4 行：每行首列为 `▎`、末列为 `🮇`（template_block_bar），总宽恒为 30。
    for (idx, expected_prefix) in ["▎fn main() {", "▎    println!(\"hi\");", "▎}"]
        .into_iter()
        .enumerate()
    {
        let line = &lines[2 + idx];
        assert_eq!(line.code, Some(CodeLineKind::Body));
        assert_eq!(line.width(), 30);
        assert!(line.text().starts_with(expected_prefix));
        assert!(line.text().ends_with(text::CODE_BLOCK_RIGHT_BAR));
        assert_eq!(
            line.spans[0],
            ("▎".to_string(), style::template_block_bar())
        );
        assert_eq!(
            line.spans.last().cloned(),
            Some((
                text::CODE_BLOCK_RIGHT_BAR.to_string(),
                style::template_block_bar()
            ))
        );
    }

    // 第 5 行：底边框铺满 30 列 `🮂`，样式 template_block_bar。
    assert_eq!(lines[5].code, Some(CodeLineKind::BottomBorder));
    assert_eq!(lines[5].text(), text::CODE_BLOCK_BOTTOM_BORDER.repeat(30));
    assert_eq!(lines[5].width(), 30);
    assert_eq!(lines[5].spans[0].1, style::template_block_bar());
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
    for (expected_idx, header) in headers.iter().enumerate() {
        assert!(
            header.text().starts_with(&format!("▎{DEFAULT_CODE_LANG} ")),
            "默认语言应显示为 text：{:?}",
            header.text()
        );
        assert!(header.text().ends_with("[❐]🮇"));
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
    // width = 10：col 0 为 `▎`，col 6..9 为 `[❐]`，col 9 为 `🮇`，col 5 为间隔空格，语言预算为 4 列 -> `typ…`。
    let lines = render_wrapped_code_blocks(src, 10);
    assert_eq!(lines[1].text(), "▎typ… [❐]🮇");
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
fn soft_wrapped_code_lines_and_blank_lines_have_left_and_right_bars_on_every_row() {
    let src = "```sh\necho alpha beta gamma delta\n\necho done\n```";
    let lines = render_wrapped_code_blocks(src, 14);
    let body_lines: Vec<&MdLine> = lines
        .iter()
        .filter(|line| line.code == Some(CodeLineKind::Body))
        .collect();
    assert!(body_lines.len() >= 4, "长行应软换行拆成多条视觉行");
    for line in &body_lines {
        assert!(
            line.text().starts_with('▎') && line.text().ends_with('🮇'),
            "每个视觉行（含续行与空行）首尾必须有 `▎` 与 `🮇`：{:?}",
            line.text()
        );
        assert_eq!(line.width(), 14, "每个代码行应铺满带左右边框的宽度");
    }
    assert!(
        body_lines
            .iter()
            .any(|line| line.text() == format!("▎{}🮇", " ".repeat(12))),
        "空代码行也应保留左右 `▎` 与 `🮇` 边框"
    );
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
