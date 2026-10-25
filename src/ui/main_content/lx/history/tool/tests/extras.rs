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

// ===== 详情行折叠：默认最多 2 行，超出可展开 =====

/// 超长参数：默认只画 2 个物理行，末行以 `…` 收尾；展开后铺开全部（同一份换行口径）。
#[test]
fn long_detail_wraps_to_two_rows_and_expands() {
    let long = "a/very/long/nested/directory/structure/with/many/segments/file.rs".repeat(3);
    let call = tool_call(
        "read",
        ToolStatus::Completed,
        Some(ToolSummary {
            path: Some(long.clone()),
            range: Some((1, 9)),
            ..ToolSummary::default()
        }),
        None,
    );
    let width = 30u16;
    let folded = file_summary::detail(&call, width, false);
    assert_eq!(
        folded.len(),
        file_summary::DETAIL_PREVIEW_ROWS + 1,
        "折叠态 2 行内容 + 1 行剩余行数：{folded:#?}"
    );
    for row in &folded {
        let cols: usize = segments_cols(row);
        assert!(cols <= usize::from(width), "不越界：{cols} > {width}");
    }
    // 最后一行是剩余行数提示（在内容下方），不重复连接符。
    // 续行对齐到 `└─ ` 之后的正文列（前缀等宽空白，不重复连接符）。
    let prefix_cols = file_summary::detail_prefix().width();
    assert_eq!(folded[1][0].text.width(), prefix_cols, "续行用等宽空白缩进");
    assert_eq!(folded[1][0].text.trim(), "", "续行不重复 `└─`");
    // 折叠态画满 2 行内容后，末尾提示剩余行数（长参数折不进 2 行）。
    let folded_text: String = folded.iter().flatten().map(|s| s.text.as_str()).collect();
    assert!(
        folded_text.contains("more line"),
        "有剩余行数提示：{folded_text:?}"
    );

    let expanded = file_summary::detail(&call, width, true);
    assert!(
        expanded.len() >= folded.len(),
        "展开不少于折叠：{expanded:#?}"
    );
    assert!(
        expanded.len() <= file_summary::DETAIL_MAX_ROWS,
        "展开有上限，不会把整屏历史挤没"
    );
}

/// 超过 2 行额度的极长参数：折叠态末行以 `…` 收尾，展开后能看到更多。
#[test]
fn very_long_detail_marks_truncation_and_expands_further() {
    let long = "x".repeat(400);
    let call = tool_call(
        "write",
        ToolStatus::Completed,
        Some(ToolSummary {
            path: Some(long),
            added: Some(1),
            removed: Some(0),
            ..ToolSummary::default()
        }),
        None,
    );
    let width = 24u16;
    let folded = file_summary::detail(&call, width, false);
    assert_eq!(
        folded.len(),
        file_summary::DETAIL_PREVIEW_ROWS + 1,
        "2 行内容 + 1 行剩余行数"
    );
    let folded_text: String = folded.iter().flatten().map(|s| s.text.as_str()).collect();
    assert!(
        folded_text.contains('…'),
        "放不下的参数有省略号：{folded_text:?}"
    );
    let expanded = file_summary::detail(&call, width, true);
    assert!(expanded.len() > folded.len(), "展开铺开更多行");
    assert!(expanded.len() <= file_summary::DETAIL_MAX_ROWS);
}

/// 短参数恒为 1 行：没有可展开内容，也不注册命中区。
#[test]
fn short_detail_stays_one_row_and_is_not_clickable() {
    let call = tool_call("read", ToolStatus::Completed, Some(read_summary()), None);
    assert_eq!(file_summary::detail(&call, 80, false).len(), 1);
    assert_eq!(
        file_summary::detail(&call, 80, true).len(),
        1,
        "短参数展开也只有 1 行"
    );
    assert_eq!(rows(&call, 80, folds(false)), 2, "标题 + 详情");
    assert_eq!(rows(&call, 80, folds(true)), 2, "展开也不变");
}

/// 长参数：布局行数随展开变化，且折叠/展开差值就是多出来的行。
#[test]
fn long_detail_rows_follow_expansion() {
    let long = "src/deeply/nested/module/path/that/keeps/going/mod.rs".repeat(3);
    let call = tool_call(
        "write",
        ToolStatus::Completed,
        Some(ToolSummary {
            path: Some(long),
            added: Some(9),
            removed: Some(2),
            ..ToolSummary::default()
        }),
        None,
    );
    let folded = rows(&call, 24, folds(false));
    let expanded = rows(&call, 24, folds(true));
    assert!(
        file_summary::detail(&call, 24, true).len() > file_summary::DETAIL_PREVIEW_ROWS,
        "长参数展开超过折叠额度"
    );
    // 折叠 = 标题 + 2 行内容 + 1 行剩余行数提示。
    assert_eq!(folded, 1 + file_summary::DETAIL_PREVIEW_ROWS + 1);
    assert!(
        expanded > folded,
        "展开比折叠多行（折叠 2 行内容 + 1 行提示）：{folded} → {expanded}"
    );
}

/// 参数文字：**折叠态 dim、展开态默认前景**（展开后保持高亮，收起后恢复）。
///
/// 用户要求「展开参数后参数内容文字保持高亮的文字颜色，折叠后恢复」。
#[test]
fn parameter_stays_light_yellow_when_expanded() {
    let call = tool_call("read", ToolStatus::Completed, Some(read_summary()), None);
    let folded = file_summary::detail(&call, 80, false);
    let expanded = file_summary::detail(&call, 80, true);
    assert_eq!(folded.len(), expanded.len(), "短参数展开后行数不变");
    let param_style = |rows: &[Vec<Segment>]| {
        rows.iter()
            .flatten()
            .find(|segment| segment.text.contains("src/ui/style.rs"))
            .map(|segment| segment.style)
            .expect("参数段")
    };
    assert_eq!(param_style(&folded), style::chat_tool_param(), "折叠态浅黄");
    assert_eq!(
        param_style(&expanded),
        style::chat_tool_param(),
        "展开态仍是同一档浅黄（不做展开高亮）"
    );
}

// ===== 折叠态的剩余行数提示、悬停提亮与展开块底色 =====

/// 折叠态：内容下方单独一行写剩余行数；展开后这一行消失。
#[test]
fn folded_detail_reports_remaining_lines_below_content() {
    let long = "src/deeply/nested/module/path/that/keeps/going/mod.rs".repeat(3);
    let call = tool_call(
        "read",
        ToolStatus::Completed,
        Some(ToolSummary {
            path: Some(long),
            range: Some((1, 9)),
            ..ToolSummary::default()
        }),
        None,
    );
    let folded = file_summary::detail(&call, 24, false);
    assert_eq!(
        folded.len(),
        file_summary::DETAIL_PREVIEW_ROWS + 1,
        "2 行内容 + 1 行剩余行数：{folded:#?}"
    );
    let hint: String = folded
        .last()
        .expect("hint row")
        .iter()
        .map(|segment| segment.text.as_str())
        .collect();
    assert!(hint.contains("more line"), "剩余行数文案：{hint:?}");
    assert!(
        !hint.contains(text::agent::MARKER_TREE_LEAF),
        "提示行不重复连接符：{hint:?}"
    );
    let expanded = file_summary::detail(&call, 24, true);
    assert!(
        !expanded
            .iter()
            .flatten()
            .any(|s| s.text.contains("more line")),
        "展开后不再提示剩余行数"
    );
}

/// 折叠参数恒为浅黄；普通工具不铺展开底色（底色只属于 diff/命令这类卡片）。
#[test]
fn folded_detail_keeps_the_light_yellow() {
    let call = tool_call("read", ToolStatus::Completed, Some(read_summary()), None);
    let folded = file_summary::detail(&call, 80, false);
    let param_style = |rows: &[Vec<Segment>]| {
        rows.iter()
            .flatten()
            .find(|segment| segment.text.contains("src/ui/style.rs"))
            .map(|segment| segment.style)
            .expect("参数段")
    };
    assert_eq!(param_style(&folded), style::chat_tool_param(), "参数恒浅黄");
    // 普通工具的 `└─` 行是参数本身，展开时**不铺**底色（用户要求不覆盖参数）。
    // 底色只属于文件差异卡片与进程命令这类「展开后铺开一坨内容」的卡片。
}

/// 折叠态的「剩余行数」提示行样式恒为 dim（不随鼠标提亮）。
#[test]
fn remaining_hint_row_stays_dim() {
    let long = "src/deeply/nested/module/path/that/keeps/going/mod.rs".repeat(3);
    let call = tool_call(
        "read",
        ToolStatus::Completed,
        Some(ToolSummary {
            path: Some(long),
            range: Some((1, 9)),
            ..ToolSummary::default()
        }),
        None,
    );
    let folded = file_summary::detail(&call, 24, false);
    let hint_style = folded
        .last()
        .and_then(|row| row.last())
        .map(|segment| segment.style)
        .expect("提示行");
    assert_eq!(hint_style, style::agent_step_detail(), "提示行恒为 dim");
}
