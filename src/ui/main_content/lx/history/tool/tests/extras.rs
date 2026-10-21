//! 各工具特有的次要信息：命中数、区间 + 实际行数、增删计数、截断提示与分隔符。

use super::*;

// ===== 2. 各工具特有的次要信息 =====

#[test]
fn search_shows_match_count_and_never_the_match_body() {
    let mut summary = search_summary();
    summary.truncated = true;
    let call = tool_call("search", ToolStatus::Completed, Some(summary), None);
    let painted = detail_text(&call, 100);
    assert!(painted.contains("3 matches"), "命中数：{painted}");
    assert!(
        painted.contains(text::agent::TOOL_TRUNCATED),
        "截断显式告知：{painted}"
    );
    // 命中正文（结果体）绝不进历史行。
    for needle in ["src/ui/style.rs:12:", "pub fn text()", "fn render"] {
        assert!(!painted.contains(needle), "不应铺开命中正文：{painted}");
    }

    // 单复数：1 用 `match`。
    let single = tool_call(
        "search",
        ToolStatus::Completed,
        Some(ToolSummary {
            path: Some("src".into()),
            matches: Some(1),
            ..ToolSummary::default()
        }),
        None,
    );
    assert!(detail_text(&single, 80).contains("1 match"));
}

#[test]
fn edit_and_write_show_added_and_removed_counts_in_diff_colors() {
    let write = tool_call("write", ToolStatus::Completed, Some(write_summary()), None);
    let painted = detail_text(&write, 80);
    assert!(
        painted.contains(text::agent::added_label(18).as_str()),
        "{painted}"
    );
    // `removed: Some(0)` 仍显示 `-0`（表示整文件覆盖无删除）。
    assert!(
        painted.contains(text::agent::removed_label(0).as_str()),
        "{painted}"
    );
}

#[test]
fn unknown_tool_shows_name_and_summary_only() {
    let call = tool_call(
        "web_fetch",
        ToolStatus::Completed,
        Some(ToolSummary {
            path: Some("https://example.com".into()),
            ..ToolSummary::default()
        }),
        None,
    );
    let (painted, _) = render_row(&call, 80);
    assert!(
        painted.starts_with(text::agent::STEP_MARK_FILLED),
        "未知工具同样以方块起头：{painted}"
    );
    let (header_line, detail_line) = painted.split_once('\n').expect("两行");
    assert!(
        header_line.contains("web_fetch"),
        "未知工具在第 1 行显示工具名：{header_line:?}"
    );
    assert!(
        detail_line.contains("https://example.com"),
        "参数摘要在第 2 行：{detail_line:?}"
    );
}

#[test]
fn optional_error_and_summary_leave_no_empty_segments() {
    let bare = tool_call("read", ToolStatus::Completed, None, None);
    let painted = render_row(&bare, 60).0;
    assert!(!painted.contains("()"), "无摘要不显示空括号：{painted:?}");
    // 第 1 行不留尾随空格以外的痕迹；第 2 行只有前缀。
    let (header_line, detail_line) = painted.split_once('\n').expect("两行");
    assert_eq!(
        header_line.trim_end(),
        format!(
            "{} {}",
            text::agent::STEP_MARK_FILLED,
            text::agent::VERB_TOOL_READ
        ),
        "第 1 行只有方块 + 动词：{header_line:?}"
    );
    assert_eq!(
        detail_line.trim_end(),
        format!("  {}", text::agent::MARKER_TREE_LEAF),
        "第 2 行只有前缀：{detail_line:?}"
    );
}

// ===== 5. 搜索模式 / 实际行数 / 计数组装 =====

/// 搜索行必须显示 pattern（加引号）且在路径之前：`"测试" in path · 3 matches`。
#[test]
fn search_shows_the_pattern_before_the_path() {
    let call = tool_call(
        "search",
        ToolStatus::Completed,
        Some(ToolSummary {
            path: Some("tool_test.txt".into()),
            pattern: Some("测试".into()),
            matches: Some(3),
            ..ToolSummary::default()
        }),
        None,
    );
    let detail = detail_text(&call, 80);
    let expected = format!(
        "  {} {} {}tool_test.txt{}{}",
        text::agent::MARKER_TREE_LEAF,
        text::agent::pattern_label("测试"),
        text::agent::IN_SCOPE_SEP,
        text::LX_CHAT_TOOL_STATUS_SEP,
        text::agent::matches_label(3)
    );
    assert_eq!(detail, expected, "搜索行 = 「模式 in 路径 · 命中数」");

    // 模式与路径都用默认字体色（参数不再单独上色）。
    let segments = detail_segments(&call, 80);
    let scope = segments
        .iter()
        .find(|(text, _)| text.contains("测试"))
        .cloned()
        .expect("模式段");
    assert_eq!(scope.1, style::chat_tool_param());
    assert!(
        !scope
            .1
            .add_modifier
            .contains(ratatui::style::Modifier::UNDERLINED),
        "参数不再加下划线"
    );
}

/// 只有模式没有路径时仍显示模式（`"fn render" · 5 matches`）。
#[test]
fn search_without_path_still_shows_the_pattern() {
    let call = tool_call(
        "search",
        ToolStatus::Completed,
        Some(ToolSummary {
            path: None,
            pattern: Some("fn render".into()),
            matches: Some(5),
            ..ToolSummary::default()
        }),
        None,
    );
    let detail = detail_text(&call, 80);
    assert!(detail.contains("fn render"), "模式必须显示：{detail:?}");
    assert!(detail.contains("5 matches"), "{detail:?}");
    assert!(
        !detail.contains(text::agent::IN_SCOPE_SEP.trim()),
        "无路径时不该有 in：{detail:?}"
    );
}

/// `read` 把请求区间与实际行数合成一段：`12-40 (31 lines)`。
#[test]
fn read_shows_range_and_actual_line_count() {
    let call = tool_call(
        "read",
        ToolStatus::Completed,
        Some(ToolSummary {
            path: Some("src/ui/style.rs".into()),
            range: Some((12, 40)),
            read_lines: Some(31),
            ..ToolSummary::default()
        }),
        None,
    );
    let detail = detail_text(&call, 80);
    let expected = format!(
        "  {} src/ui/style.rs{}{} {}",
        text::agent::MARKER_TREE_LEAF,
        text::LX_CHAT_TOOL_STATUS_SEP,
        text::agent::range_label(12, 40),
        text::agent::lines_label(31),
    );
    assert_eq!(detail, expected, "read 行 = 「路径 · 区间 (行数)」");
    // 区间与行数在**同一段**（同色 dim 元信息），不再各占一段。
    let segments = detail_segments(&call, 80);
    let range = segments
        .iter()
        .find(|(text, _)| text.contains("31 lines"))
        .cloned()
        .expect("行数段");
    assert_eq!(range.1, style::chat_tool_range());
    assert!(
        range.0.contains("12-40"),
        "区间与行数必须合体：{:?}",
        range.0
    );
}

/// `(N lines)` 用 `read_lines`（本次实际读到），**绝不用 `total_lines`**（文件总行数）。
#[test]
fn read_uses_read_lines_not_total_lines() {
    let call = tool_call(
        "read",
        ToolStatus::Completed,
        Some(ToolSummary {
            path: Some("src/big.rs".into()),
            range: Some((12, 40)),
            read_lines: Some(29),
            total_lines: Some(1000),
            ..ToolSummary::default()
        }),
        None,
    );
    let detail = detail_text(&call, 80);
    assert!(
        detail.contains(&text::agent::lines_label(29)),
        "应显示实际读到的 29 行：{detail:?}"
    );
    assert!(
        !detail.contains(&text::agent::lines_label(1000)),
        "不得把文件总行数当成本次读到的行数：{detail:?}"
    );
}

/// 没有实际行数时只显示区间（老数据/中断调用不编造，也不拿 total_lines 兜底）。
#[test]
fn read_without_read_lines_shows_the_range_only() {
    let call = tool_call(
        "read",
        ToolStatus::Completed,
        Some(ToolSummary {
            total_lines: Some(1000),
            ..read_summary()
        }),
        None,
    );
    let detail = detail_text(&call, 80);
    assert!(detail.contains("12-40"));
    assert!(!detail.contains("lines)"), "不编造行数：{detail:?}");
    assert!(
        !detail.contains(&text::agent::lines_label(1000)),
        "total_lines 不得兜底行数：{detail:?}"
    );
}

/// 增删计数合成一段 `+12 -4`（中间不插 `·`），且各自保留绿/红。
#[test]
fn edit_counts_are_one_group_without_a_separator() {
    let call = tool_call("edit", ToolStatus::Completed, Some(edit_summary()), None);
    let detail = detail_text(&call, 80);
    assert!(
        detail.ends_with("+12 -4"),
        "应是 `+12 -4` 一个整体：{detail:?}"
    );
    assert!(
        !detail.contains(&format!("+12{}", text::LX_CHAT_TOOL_STATUS_SEP)),
        "两个计数之间不该有 `·`：{detail:?}"
    );
    // 颜色仍分开：`+12` 绿、`-4` 红。
    let segments = detail_segments(&call, 80);
    let added = segments
        .iter()
        .find(|(text, _)| text.contains("+12"))
        .cloned()
        .expect("+12");
    assert_eq!(added.1, style::agent_added());
    let removed = segments
        .iter()
        .find(|(text, _)| text.contains("-4"))
        .cloned()
        .expect("-4");
    assert_eq!(removed.1, style::agent_removed());
}

/// `write` 的 `+18 -0` 同样合体（0 删除仍显示）。
#[test]
fn write_counts_are_one_group_too() {
    let call = tool_call("write", ToolStatus::Completed, Some(write_summary()), None);
    let detail = detail_text(&call, 80);
    assert!(detail.ends_with("+18 -0"), "{detail:?}");
}

/// `partial == true` 时详情行尾追加 dim 的 `· partial`（与增删计数同一行）。
#[test]
fn edit_partial_shows_dim_tail() {
    let call = tool_call(
        "edit",
        ToolStatus::Completed,
        Some(ToolSummary {
            partial: true,
            ..edit_summary()
        }),
        None,
    );
    let detail = detail_text(&call, 80);
    let expected_tail = format!("{}partial", text::LX_CHAT_TOOL_STATUS_SEP);
    assert!(
        detail.ends_with(&expected_tail),
        "partial 必须在详情行尾：{detail:?}"
    );
    assert!(
        detail.contains(&format!("+12 -4{expected_tail}")),
        "partial 与计数之间要有 `· ` 分隔：{detail:?}"
    );
    // 该段是 dim 元信息（不是警告黄、也不是计数色）。
    let segments = detail_segments(&call, 80);
    let partial = segments
        .iter()
        .find(|(text, _)| text.contains(text::agent::TOOL_PARTIAL))
        .cloned()
        .expect("partial 段");
    assert_eq!(partial.1, style::agent_step_detail(), "partial 用 dim 样式");
}

/// 没有 partial 时不追加尾巴（不得凭空出现 `· partial`）。
#[test]
fn edit_without_partial_has_no_tail() {
    let call = tool_call("edit", ToolStatus::Completed, Some(edit_summary()), None);
    let detail = detail_text(&call, 80);
    assert!(
        !detail.contains(text::agent::TOOL_PARTIAL),
        "未部分成功时不显示 partial：{detail:?}"
    );
    // 只有路径 + partial（没有计数）时同样不能产生孤立分隔符。
    let only_partial = tool_call(
        "edit",
        ToolStatus::Completed,
        Some(ToolSummary {
            path: Some("src/app/state.rs".into()),
            partial: true,
            ..ToolSummary::default()
        }),
        None,
    );
    let detail = detail_text(&only_partial, 80);
    let body = detail
        .strip_prefix(&format!("  {} ", text::agent::MARKER_TREE_LEAF))
        .expect("前缀");
    let expected = format!("src/app/state.rs{}partial", text::LX_CHAT_TOOL_STATUS_SEP);
    assert_eq!(body, expected, "详情 = 路径 + `· partial`，无孤立分隔符");
}

/// 多文件 `edit`（`path == None`）不得渲染出孤立的 `· `：应为 `└─ +12 -4`。
#[test]
fn multi_file_edit_has_no_orphan_separator() {
    let call = tool_call(
        "edit",
        ToolStatus::Completed,
        Some(ToolSummary {
            path: None,
            added: Some(12),
            removed: Some(4),
            ..ToolSummary::default()
        }),
        None,
    );
    let detail = detail_text(&call, 80);
    let prefix = format!("  {} ", text::agent::MARKER_TREE_LEAF);
    assert_eq!(
        detail,
        format!("{prefix}+12 -4"),
        "多文件 edit 详情 = 「└─ +12 -4」，无前导 `· `：{detail:?}"
    );
    let body = detail.strip_prefix(&prefix).expect("前缀");
    assert!(
        !body.starts_with(text::LX_CHAT_TOOL_STATUS_SEP.trim()),
        "详情首段不得以 `·` 开头：{body:?}"
    );
    assert!(!body.contains("  "), "详情里不得出现两个连续空格：{body:?}");
}

/// 只有 `truncated` 时同样不能出现孤立分隔符：`└─ …[truncated]`。
#[test]
fn truncated_only_has_no_orphan_separator() {
    let call = tool_call(
        "read",
        ToolStatus::Completed,
        Some(ToolSummary {
            truncated: true,
            ..ToolSummary::default()
        }),
        None,
    );
    let detail = detail_text(&call, 80);
    let prefix = format!("  {} ", text::agent::MARKER_TREE_LEAF);
    assert_eq!(
        detail,
        format!("{prefix}{}", text::agent::TOOL_TRUNCATED),
        "只有截断提示时无前导 `· `：{detail:?}"
    );
}
