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
    assert_eq!(lines[0].text(), text::PROMPT_BLOCK_TOP_BORDER.repeat(30));
    assert_eq!(lines[0].width(), 30);
    assert_eq!(lines[0].spans[0].1, style::md_read_rule());

    // 第 1 行：首列 `▎` + `rust` + 空白填充 + 右对齐 `[❐]`。
    assert_eq!(
        lines[1].code,
        Some(CodeLineKind::Header {
            block_index: 0,
            button_cols: (27, 30),
        })
    );
    assert_eq!(lines[1].width(), 30);
    assert!(lines[1].text().starts_with("▎rust"));
    assert!(lines[1].text().ends_with(text::TEMPLATE_BUTTON_COPY));
    assert_eq!(lines[1].spans[0], ("▎".to_string(), style::md_read_rule()));
    assert_eq!(lines[1].spans[1], ("rust".to_string(), style::muted()));
    assert_eq!(
        lines[1].spans.last().cloned(),
        Some((
            text::TEMPLATE_BUTTON_COPY.to_string(),
            style::template_button()
        ))
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
    for (expected_idx, header) in headers.iter().enumerate() {
        assert!(
            header.text().starts_with(&format!("▎{DEFAULT_CODE_LANG} ")),
            "默认语言应显示为 text：{:?}",
            header.text()
        );
        assert!(header.text().ends_with(text::TEMPLATE_BUTTON_COPY));
        assert_eq!(
            header.code,
            Some(CodeLineKind::Header {
                block_index: expected_idx,
                button_cols: (21, 24),
            })
        );
    }
}

#[test]
fn narrow_width_truncates_language_with_ellipsis_and_keeps_copy_button() {
    let src = "```typescript\nconst x = 1;\n```";
    // width = 10：col 0 为 `▎`，col 7..10 为 `[❐]`，col 6 为间隔空格，语言预算为 5 列 -> `type…`。
    let lines = render_wrapped_code_blocks(src, 10);
    assert_eq!(lines[1].text(), "▎type… [❐]");
    assert_eq!(lines[1].width(), 10);
    assert_eq!(
        lines[1].code,
        Some(CodeLineKind::Header {
            block_index: 0,
            button_cols: (7, 10),
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
        assert!(line.width() <= 14, "不得超过给定折行宽度：{}", line.width());
    }
    assert!(
        body_lines.iter().any(|line| line.text() == "▎"),
        "空代码行也应保留首列 `▎`"
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
