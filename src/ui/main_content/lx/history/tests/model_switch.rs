//! 模型切换提示条目（`◆ Model switched`）：树体详情、折叠/展开、连续 `xN` 合并、压暗 Markdown 样式与按 sequence 交错。

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

/// 读取网格整段文本（逐行，保留行首缩进）。
fn rows_text(grid: &Grid, area: Rect) -> Vec<String> {
    (area.y..area.bottom())
        .map(|y| {
            (area.x..area.right())
                .map(|x| grid.cell(x, y).ch)
                .collect::<String>()
        })
        .collect()
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
    let area = Rect::new(0, 0, 130, 14);
    let folded_layout = layout(area, &lx, &mut cache());
    let mut grid = Grid::new(area);
    let drawn = render(&mut grid, area, &lx, 0, &mut cache());
    assert_eq!(drawn, folded_layout.total_rows);
    // 标题 + 两行树体详情 + 末尾呼吸行 = 4 行（宽屏单行）
    assert_eq!(folded_layout.total_rows, 4);

    let folded_text: String = rows_text(&grid, area).join("\n");
    assert!(folded_text.contains("Model switched"), "{folded_text}");
    assert!(folded_text.contains('◆'), "包含菱形标记：{folded_text}");
    assert!(
        folded_text.contains("model: DeepSeek V4.1 Flash (max)"),
        "{folded_text}"
    );
    assert!(
        folded_text.contains("  ├─ model: DeepSeek V4.1 Flash (max)"),
        "目标模型行挂在 `├─` 下：{folded_text}"
    );
    assert!(
        folded_text.contains("  └─ instructions: core/model, core/agents_md"),
        "指令源行由 `└─` 收口：{folded_text}"
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

    let expanded = rows_text(&grid2, area);
    let expanded_text = expanded.join("\n");
    assert!(
        expanded_text.contains("switched Claude Sonnet 4 (high) -> DeepSeek V4.1 Flash (max)"),
        "{expanded_text}"
    );
    assert!(expanded_text.contains("Your Model"), "{expanded_text}");

    // 展开首行带思考块同款 `  └─ ` 树形连接符，正文对齐第 5 列
    let transition_row = expanded
        .iter()
        .position(|line| line.contains("switched Claude Sonnet 4 (high)"))
        .expect("transition row");
    assert!(
        expanded[transition_row].starts_with(text::agent::TREE_LEAF),
        "展开首行必须有 `  └─ ` 前缀：{:?}",
        expanded[transition_row]
    );
    assert_eq!(
        grid2.cell(area.x, area.y + transition_row as u16).style,
        style::chat_notice_connector(),
        "连接符使用 chat_notice_connector 样式"
    );
    let prompt_row = expanded
        .iter()
        .position(|line| line.contains("Your Model"))
        .expect("prompt row");
    assert!(
        expanded[prompt_row].starts_with("     Your Model"),
        "提示词正文必须缩进 5 列：{:?}",
        expanded[prompt_row]
    );

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

/// 展开态：首行画 `  └─ `（`style::chat_notice_connector()`），其余行缩进 5 列并保持压暗。
#[test]
fn expanded_renders_tree_leaf_prefix_and_indented_muted_markdown() {
    let mut lx = Lx::new();
    let mut notice = switch_view(
        2,
        "Claude Sonnet 4 (high)",
        "9router",
        "deepseek-v4.1-flash",
        Some("max"),
        "DeepSeek V4.1 Flash",
    );
    notice.injected_prompt =
        "# Your Model\n- Provider ID: 9router-with-a-long-provider-identifier-to-wrap".into();
    lx.model_switch.replace(vec![notice]);
    assert!(lx.model_switch.toggle("msw-2"));

    let area = Rect::new(0, 0, 40, 20);
    let mut grid = Grid::new(area);
    let drawn = render(&mut grid, area, &lx, 0, &mut cache());
    assert_eq!(drawn, layout(area, &lx, &mut cache()).total_rows);

    let lines = rows_text(&grid, area);
    let body_start = lines
        .iter()
        .position(|line| line.starts_with(text::agent::TREE_LEAF))
        .expect("tree leaf row");
    assert!(
        lines[body_start].starts_with("  └─ switched Claude Sonnet 4 (high)"),
        "首行连接符 + 跃迁对比：{:?}",
        lines[body_start]
    );
    assert_eq!(
        grid.cell(2, area.y + body_start as u16).style,
        style::chat_notice_connector(),
        "`└─` 使用连接符样式"
    );
    assert_eq!(
        lines
            .iter()
            .filter(|line| line.contains(text::agent::TREE_LEAF))
            .count(),
        1,
        "连接符只在展开首行出现一次：{lines:?}"
    );

    // 展开正文（含软折行续行）统一从第 5 列起，且保持 `style::muted()` 压暗
    let body_rows: Vec<(usize, &String)> = lines
        .iter()
        .enumerate()
        .skip(body_start + 1)
        .filter(|(_, line)| !line.trim().is_empty())
        .collect();
    assert!(!body_rows.is_empty(), "展开态必须有正文行：{lines:?}");
    for (row, line) in body_rows {
        assert!(
            line.starts_with("     "),
            "展开正文续行必须缩进 5 列：{line:?}"
        );
        assert!(
            grid.cell(5, area.y + row as u16)
                .style
                .add_modifier
                .contains(ratatui::style::Modifier::DIM),
            "展开正文从第 5 列起必须保持压暗：{line:?}"
        );
    }
}

/// 非末节点折行时续行保留 `│` 树干，末节点用等宽空白收口。
#[test]
fn wrapped_system_detail_keeps_tree_bar() {
    let mut lx = Lx::new();
    let mut notice = switch_view(
        2,
        "Claude Sonnet 4 (high)",
        "9router",
        "deepseek-v4.1-flash",
        Some("max"),
        "DeepSeek V4.1 Flash with a very long display name",
    );
    notice.updated_sources = vec!["core/model".into()];
    lx.model_switch.replace(vec![notice]);

    let area = Rect::new(0, 0, 40, 14);
    let mut grid = Grid::new(area);
    let drawn = render(&mut grid, area, &lx, 0, &mut cache());
    assert_eq!(drawn, layout(area, &lx, &mut cache()).total_rows);

    let lines = rows_text(&grid, area);
    let branch = lines
        .iter()
        .position(|line| line.starts_with(text::agent::TREE_BRANCH))
        .expect("branch row");
    let leaf = lines
        .iter()
        .position(|line| line.starts_with(text::agent::TREE_LEAF))
        .expect("leaf row");
    assert!(leaf > branch + 1, "首个节点必须折行：{lines:?}");
    for line in &lines[branch + 1..leaf] {
        assert!(
            line.starts_with(text::agent::TREE_BAR),
            "非末节点续行必须保留 `│` 树干：{line:?}"
        );
    }
}

/// 连续切换合并为 `xN`：追加切换不重置展开态，描述行汇总全部指令源，
/// 展开首行保留完整跃迁链（`A -> B -> A`，不退化）。
#[test]
fn consecutive_switches_merge_into_xn_and_keep_expanded_and_chain() {
    let mut lx = Lx::new();
    let first = switch_view(
        2,
        "Claude Sonnet 4 (high)",
        "openai",
        "gpt-5",
        Some("medium"),
        "GPT-5",
    );
    lx.model_switch.replace(vec![first.clone()]);
    assert!(lx.model_switch.toggle("msw-2"), "先展开首条切换");

    // 第二次切换回原模型（A -> B -> A），并追加到同一合并区间
    let mut second = switch_view(
        3,
        "GPT-5 (medium)",
        "anthropic",
        "claude-sonnet-4",
        Some("high"),
        "Claude Sonnet 4",
    );
    second.updated_sources = vec!["core/reasoning_effort".into()];
    lx.model_switch.replace(vec![first, second]);
    assert!(
        lx.model_switch.is_range_expanded(&lx.model_switch.switches),
        "追加连续切换后卡片必须保持展开"
    );

    let area = Rect::new(0, 0, 140, 16);
    let snapshot_layout = layout(area, &lx, &mut cache());
    assert_eq!(
        snapshot_layout.cards.first().map(|card| card.kind.clone()),
        Some(CardKind::ModelSwitch {
            switch_id: "msw-2".into(),
            expanded: true,
        }),
        "合并卡片锚定区间首条 id 并保持展开"
    );
    let mut grid = Grid::new(area);
    let drawn = render(&mut grid, area, &lx, 0, &mut cache());
    assert_eq!(drawn, snapshot_layout.total_rows);

    let text = rows_text(&grid, area).join("\n");
    assert!(
        text.contains("Model switched x2"),
        "连续切换应合并为 x2：{text}"
    );
    assert!(
        text.contains("instructions: core/model, core/agents_md, core/reasoning_effort"),
        "描述行必须汇总区间内去重保序的全部指令源：{text}"
    );
    assert!(
        text.contains(
            "switched Claude Sonnet 4 (high) -> GPT-5 (medium) -> Claude Sonnet 4 (high)"
        ),
        "展开首行必须保留完整跃迁链：{text}"
    );
    assert!(
        !text.contains("switched Claude Sonnet 4 (high) -> Claude Sonnet 4 (high)"),
        "回环切换不得退化为 A -> A：{text}"
    );

    // 再点一次（卡片锚定首条 id）收起整个区间
    assert!(lx.model_switch.toggle("msw-2"));
    assert!(
        !lx.model_switch.is_range_expanded(&lx.model_switch.switches),
        "点击合并卡片必须收起整个区间"
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

    let area = Rect::new(0, 0, 120, 16);
    let mut grid = Grid::new(area);
    let drawn = render(&mut grid, area, &lx, 0, &mut cache());
    assert_eq!(drawn, layout(area, &lx, &mut cache()).total_rows);

    let text = rows_text(&grid, area).join("\n");
    assert!(
        text.contains("Model switched x2"),
        "连续切换应合并为 x2：{text}"
    );
    assert!(
        text.contains(
            "switched Claude Sonnet 4 (high) -> GPT-5 (medium) -> DeepSeek V4.1 Flash (max)"
        ),
        "合并后展开首行应显示完整跃迁链：{text}"
    );
}
