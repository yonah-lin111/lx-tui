//! 模型切换提示条目（`-- Model switched --`）：折叠/展开、连续 `xN` 合并、压暗 Markdown 样式与按 sequence 交错。

use super::*;

fn switch_view(
    sequence: i64,
    from_label: &str,
    to_provider: &str,
    to_model: &str,
    to_variant: Option<&str>,
    to_label: &str,
) -> crate::app::state::ModelSwitchNoticeView {
    crate::app::state::ModelSwitchNoticeView {
        id: format!("msw-{sequence}"),
        sequence,
        from_provider: Some("anthropic".into()),
        from_model: Some("claude-sonnet-4".into()),
        from_variant: Some("high".into()),
        to_provider: to_provider.into(),
        to_model: to_model.into(),
        to_variant: to_variant.map(str::to_string),
        from_label: Some(from_label.into()),
        to_label: to_label.into(),
        updated_sources: vec!["core/model".into(), "core/agents_md".into()],
        injected_prompt: "# Your Model\n- Name: `DeepSeek V4.1 Flash`\n- Provider ID: 9router"
            .into(),
        created_at_ms: 1_700_000_640_000,
    }
}

#[test]
fn model_switch_folds_to_two_header_rows_and_expands_with_muted_markdown() {
    let mut lx = Lx::new();
    lx.model_switch.replace(vec![switch_view(
        2,
        "Claude Sonnet 4 (high)",
        "9router",
        "deepseek-v4.1-flash",
        Some("max"),
        "DeepSeek V4.1 Flash",
    )]);
    let area = Rect::new(0, 0, 100, 14);
    let folded_layout = layout(area, &lx, &mut cache());
    let mut grid = Grid::new(area);
    let drawn = render(&mut grid, area, &lx, 0, &mut cache());
    assert_eq!(drawn, folded_layout.total_rows);
    // 标题 + 描述 + 末尾呼吸行 = 3 行
    assert_eq!(folded_layout.total_rows, 3);

    let folded_text: String = (area.y..area.bottom())
        .map(|y| {
            (area.x..area.right())
                .map(|x| grid.cell(x, y).ch)
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        folded_text.contains("-- Model switched --"),
        "{folded_text}"
    );
    assert!(
        folded_text.contains("model: DeepSeek V4.1 Flash (max)"),
        "{folded_text}"
    );
    assert!(
        folded_text.contains("instructions: core/model, core/agents_md"),
        "{folded_text}"
    );
    assert!(
        !folded_text.contains("switched Claude Sonnet 4"),
        "折叠态不应渲染展开详情：{folded_text}"
    );

    // 点击/切换展开后，渲染模型切换前后对比及压暗 Markdown 内容
    assert!(lx.model_switch.toggle("msw-2"));
    let expanded_layout = layout(area, &lx, &mut cache());
    let mut grid2 = Grid::new(area);
    let drawn2 = render(&mut grid2, area, &lx, 0, &mut cache());
    assert_eq!(drawn2, expanded_layout.total_rows);
    assert!(expanded_layout.total_rows > folded_layout.total_rows);

    let expanded_text: String = (area.y..area.bottom())
        .map(|y| {
            (area.x..area.right())
                .map(|x| grid2.cell(x, y).ch)
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        expanded_text.contains("switched Claude Sonnet 4 (high) -> DeepSeek V4.1 Flash (max)"),
        "{expanded_text}"
    );
    assert!(expanded_text.contains("Your Model"), "{expanded_text}");

    // 验证展开正文采用思考块同款的压暗 Markdown 样式（`inline_code` 叠加 `style::muted()`）
    let code_row = (area.y..area.bottom())
        .find(|&y| {
            let line: String = (area.x..area.right())
                .map(|x| grid2.cell(x, y).ch)
                .collect();
            line.contains("DeepSeek V4.1 Flash") && line.contains("Name:")
        })
        .expect("markdown inline code row");
    let has_muted_inline_code = (area.x..area.right()).any(|x| {
        grid2.cell(x, code_row).style == style::md_read_inline_code().patch(style::muted())
    });
    assert!(
        has_muted_inline_code,
        "展开内容中的反引号代码必须使用 md_read_inline_code().patch(style::muted())"
    );
}

#[test]
fn consecutive_model_switches_coalesce_into_x_count() {
    let mut lx = Lx::new();
    let sw1 = switch_view(
        2,
        "Claude Sonnet 4 (high)",
        "openai",
        "gpt-5",
        Some("medium"),
        "GPT-5",
    );
    let sw2 = switch_view(
        3,
        "GPT-5 (medium)",
        "9router",
        "deepseek-v4.1-flash",
        Some("max"),
        "DeepSeek V4.1 Flash",
    );
    lx.model_switch.replace(vec![sw1, sw2]);
    assert!(lx.model_switch.toggle("msw-3"));

    let area = Rect::new(0, 0, 72, 16);
    let mut grid = Grid::new(area);
    let drawn = render(&mut grid, area, &lx, 0, &mut cache());
    assert_eq!(drawn, layout(area, &lx, &mut cache()).total_rows);

    let text: String = (area.y..area.bottom())
        .map(|y| {
            (area.x..area.right())
                .map(|x| grid.cell(x, y).ch)
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        text.contains("-- Model switched x2 --"),
        "连续切换应合并为 x2：{text}"
    );
    assert!(
        text.contains("switched Claude Sonnet 4 (high) -> DeepSeek V4.1 Flash (max)"),
        "合并后展开首行应显示从初始模型到最终模型的跃迁：{text}"
    );
}
