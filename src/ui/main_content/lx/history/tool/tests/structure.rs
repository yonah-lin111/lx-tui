//! 两行结构与状态编码：方块/动词/错误文案/兜底状态词在任何宽度下都可辨。

use super::*;

// ===== 1. 两行结构与状态编码 =====

/// 第 1 行只有「方块 + 动词」：**没有任何状态文字**（`done` 已删除）。
#[test]
fn header_is_only_square_and_verb_without_status_text() {
    let cases: [(&str, ToolSummary, &str); 4] = [
        ("read", read_summary(), text::agent::VERB_TOOL_READ),
        ("search", search_summary(), text::agent::VERB_TOOL_SEARCH),
        ("edit", edit_summary(), text::agent::VERB_TOOL_EDIT),
        ("write", write_summary(), text::agent::VERB_TOOL_WRITE),
    ];
    for (name, summary, verb) in cases {
        for status in [
            ToolStatus::InputStreaming,
            ToolStatus::Pending,
            ToolStatus::Running,
            ToolStatus::Completed,
            ToolStatus::Failed,
            ToolStatus::Cancelled,
            ToolStatus::Interrupted,
        ] {
            let call = tool_call(name, status, Some(summary.clone()), Some("boom"));
            let painted = header_text(&call, 80);
            assert!(
                painted.starts_with(text::agent::STEP_MARK_FILLED)
                    || painted.starts_with(text::agent::STEP_MARK_HOLLOW),
                "{name}/{status:?} 行首应为步骤方块：{painted:?}"
            );
            assert!(
                painted.contains(verb),
                "{name}/{status:?} 第 1 行应含动词 {verb}：{painted:?}"
            );
            // 关键：第 1 行不再出现任何状态文字（尤其 `done`）。
            for status_text in [
                text::LX_CHAT_TOOL_INPUT_STREAMING,
                text::LX_CHAT_TOOL_PENDING,
                text::LX_CHAT_TOOL_RUNNING,
                text::LX_CHAT_TOOL_COMPLETED,
                text::LX_CHAT_TOOL_FAILED,
                text::LX_CHAT_TOOL_CANCELLED,
                text::LX_CHAT_TOOL_INTERRUPTED,
            ] {
                assert!(
                    !painted.contains(status_text),
                    "{name}/{status:?} 第 1 行不该有状态文字 {status_text:?}：{painted:?}"
                );
            }
            // 第 1 行也不带参数：路径只在第 2 行。
            if let Some(summary) = call.summary.as_ref().and_then(|s| s.path.as_deref()) {
                assert!(
                    !painted.contains(summary),
                    "{name}/{status:?} 第 1 行不该有路径：{painted:?}"
                );
            }
            // 也没有 `·` 分隔符（状态/参数都在第 2 行）。
            assert!(
                !painted.contains(text::LX_CHAT_TOOL_STATUS_SEP.trim()),
                "{name}/{status:?} 第 1 行不该有分隔符：{painted:?}"
            );
        }
    }
}

/// 完成态、失败态都不显示 `done` / `failed` 这类状态词；只有第 2 行承载细节。
#[test]
fn completed_and_failed_never_show_status_words_on_any_line() {
    let completed = tool_call("read", ToolStatus::Completed, Some(read_summary()), None);
    let painted = full_text(&completed, 80);
    assert!(
        !painted.contains(text::LX_CHAT_TOOL_COMPLETED),
        "完成态不再显示 done：{painted:?}"
    );
    // 完成态第 2 行是参数。
    assert!(painted.contains("src/ui/style.rs"), "{painted:?}");

    let failed = tool_call(
        "read",
        ToolStatus::Failed,
        Some(read_summary()),
        Some("read /etc/shadow failed: permission denied"),
    );
    // 第 1 行不该有状态词；第 2 行是错误文案本身（可能天然含 "failed" 字样，
    // 那是错误文本的一部分，不是状态字段）。
    let header_line = header_text(&failed, 100);
    assert!(
        !header_line.contains(text::LX_CHAT_TOOL_FAILED),
        "失败态第 1 行不再显示 failed：{header_line:?}"
    );
    assert!(
        header_line.trim_end()
            == format!(
                "{} {}",
                text::agent::STEP_MARK_FILLED,
                text::agent::VERB_TOOL_READ
            ),
        "失败态第 1 行只有方块 + 动词：{header_line:?}"
    );
}

/// 第 2 行恒以 `  └─ ` 起头（缩进 2 列 + 连接符 + 空格），与通知组件同口径。
#[test]
fn detail_line_starts_with_tree_leaf_prefix() {
    let call = tool_call("read", ToolStatus::Completed, Some(read_summary()), None);
    let prefix = format!("  {} ", text::agent::MARKER_TREE_LEAF);
    let painted = detail_text(&call, 80);
    assert!(painted.starts_with(&prefix), "第 2 行前缀：{painted:?}");
    // 连接符本身用通知同款灰色 + 加粗。
    let segments = detail_segments(&call, 80);
    assert_eq!(
        segments[0].0, prefix,
        "首段必须是完整前缀（缩进+连接符+空格）"
    );
    assert_eq!(
        segments[0].1,
        style::chat_notice_connector(),
        "连接符用通知同款灰色+粗"
    );
}

/// 第 2 行的参数：路径金黄下划线、行范围 dim、增删绿/红。
#[test]
fn detail_line_carries_params_with_their_own_colors() {
    let call = tool_call("read", ToolStatus::Completed, Some(read_summary()), None);
    let segments = detail_segments(&call, 80);
    let path = segments
        .iter()
        .find(|(text, _)| text == "src/ui/style.rs")
        .cloned()
        .expect("路径段");
    assert_eq!(path.1, style::chat_tool_param(), "路径用默认字体色");
    let range = segments
        .iter()
        .find(|(text, _)| text.contains("12-40"))
        .cloned()
        .expect("行范围段");
    assert_eq!(range.1, style::chat_tool_range(), "行范围 dim");

    // edit 的增删计数。
    let edit = tool_call("edit", ToolStatus::Completed, Some(edit_summary()), None);
    let segments = detail_segments(&edit, 80);
    let added = segments
        .iter()
        .find(|(text, _)| text.ends_with(text::agent::added_label(12).as_str()))
        .cloned()
        .expect("+12 段");
    assert_eq!(added.1, style::agent_added(), "新增绿");
    let removed = segments
        .iter()
        .find(|(text, _)| text.ends_with(text::agent::removed_label(4).as_str()))
        .cloned()
        .expect("-4 段");
    assert_eq!(removed.1, style::agent_removed(), "删除红");
}

/// 失败：**整行第 1 行红**（方块 + 动词），第 2 行**只有错误**，不显示任何参数。
#[test]
fn failed_header_is_red_and_detail_shows_only_the_error() {
    for (name, summary) in [
        ("read", read_summary()),
        ("search", search_summary()),
        ("edit", edit_summary()),
        ("write", write_summary()),
    ] {
        let summary_path = summary.path.clone().expect("有路径");
        let call = tool_call(
            name,
            ToolStatus::Failed,
            Some(summary),
            Some("read /etc/shadow failed: permission denied\nsecond line"),
        );
        let (painted, grid) = render_row(&call, 100);
        let (header_line, detail_line) = painted.split_once('\n').expect("两行");
        // 第 1 行整行红 + 粗。
        let fail = style::agent_fail();
        for x in 0..u16::try_from(header_line.width()).unwrap_or(0) {
            let glyph = grid.cell(x, 0);
            if glyph.ch == ' ' {
                continue;
            }
            assert_eq!(
                glyph.style.fg, fail.fg,
                "{name} 第 1 行应整行红：{header_line:?}"
            );
        }
        // 第 2 行只放错误：不含路径、不含计数。
        assert!(
            detail_line.contains("permission denied"),
            "{name} 第 2 行显示错误：{detail_line:?}"
        );
        assert!(
            detail_line.contains("permission denied second line"),
            "{name} 错误合并成单行（换行转空格）：{detail_line:?}"
        );
        assert!(
            !detail_line.contains(&summary_path),
            "{name} 失败时第 2 行不该再显示路径参数：{detail_line:?}"
        );
        for extra in ["12-40", "+12", "-4", "3 matches", "+18"] {
            assert!(
                !detail_line.contains(extra),
                "{name} 失败时第 2 行不该显示计数 {extra:?}：{detail_line:?}"
            );
        }
        // 错误文案用正常前景（不是红），红只在第 1 行。
        let error_segment = detail_segments(&call, 100)
            .into_iter()
            .find(|(text, _)| text.contains("permission denied"))
            .expect("错误段");
        assert_eq!(error_segment.1, style::text(), "{name} 错误文案用正常前景");
    }
}

/// 中断：第 1 行黄，第 2 行只放原因；无 error 时第 2 行为空（行数仍算 2）。
#[test]
fn interrupted_header_is_yellow_and_detail_shows_only_the_reason() {
    let call = tool_call(
        "edit",
        ToolStatus::Interrupted,
        Some(edit_summary()),
        Some("interrupted before completion"),
    );
    let (painted, grid) = render_row(&call, 100);
    let (header_line, detail_line) = painted.split_once('\n').expect("两行");
    assert_eq!(
        grid.cell(0, 0).style.fg,
        style::agent_warn().fg,
        "中断方块黄色"
    );
    assert!(
        header_line.contains(text::agent::VERB_TOOL_EDIT),
        "第 1 行动词：{header_line:?}"
    );
    assert!(
        detail_line.contains("interrupted before completion"),
        "第 2 行原因：{detail_line:?}"
    );
    assert!(
        !detail_line.contains("src/app/state.rs"),
        "中断时不显示路径参数：{detail_line:?}"
    );

    // 无 error：第 2 行只有前缀（不留状态词）。
    let bare = tool_call("edit", ToolStatus::Interrupted, Some(edit_summary()), None);
    let detail = detail_text(&bare, 80);
    assert_eq!(
        detail,
        format!("  {} ", text::agent::MARKER_TREE_LEAF),
        "无原因时第 2 行只剩前缀：{detail:?}"
    );
}

/// 取消：空心方块 + 第 2 行 `cancelled` 兜底（状态词从第 1 行搬到第 2 行）。
#[test]
fn cancelled_uses_hollow_square_and_puts_the_word_in_the_detail_line() {
    let call = tool_call("read", ToolStatus::Cancelled, Some(read_summary()), None);
    let (painted, grid) = render_row(&call, 80);
    let (header_line, detail_line) = painted.split_once('\n').expect("两行");
    assert!(
        header_line.starts_with(text::agent::STEP_MARK_HOLLOW),
        "取消态行首用空心方块：{header_line:?}"
    );
    assert_eq!(
        grid.cell(0, 0).style.fg,
        style::agent_step_detail().fg,
        "取消方块用次要信息色"
    );
    assert!(
        detail_line.contains(text::LX_CHAT_TOOL_CANCELLED),
        "第 2 行显示 cancelled：{detail_line:?}"
    );
    let segment = detail_segments(&call, 80)
        .into_iter()
        .find(|(text, _)| text.contains(text::LX_CHAT_TOOL_CANCELLED))
        .expect("兜底段");
    assert_eq!(segment.1, style::agent_step_detail(), "兜底词 dim");
}

/// 没有参数可展示的进行中状态：第 2 行用 dim 状态词兜底，永不为空。
#[test]
fn open_states_without_params_fall_back_to_a_status_word() {
    for (status, expected) in [
        (
            ToolStatus::InputStreaming,
            text::LX_CHAT_TOOL_INPUT_STREAMING,
        ),
        (ToolStatus::Pending, text::LX_CHAT_TOOL_PENDING),
        (ToolStatus::Running, text::LX_CHAT_TOOL_RUNNING),
    ] {
        let call = tool_call("read", status, None, None);
        let detail = detail_text(&call, 80);
        assert!(
            detail.contains(expected),
            "{status:?} 第 2 行应兜底 {expected:?}：{detail:?}"
        );
        assert!(
            detail.starts_with(text::agent::MARKER_TREE_LEAF.trim_end().trim_start())
                || detail.contains(text::agent::MARKER_TREE_LEAF),
            "{status:?} 兜底同样带连接符：{detail:?}"
        );
    }
    // 完成态没有兜底：`done` 已删除，第二行留空。
    let done = tool_call("read", ToolStatus::Completed, None, None);
    assert_eq!(
        detail_text(&done, 80),
        format!("  {} ", text::agent::MARKER_TREE_LEAF),
        "完成态无摘要时第 2 行留空（不显示 done）"
    );
}

/// 方块呼吸：运行中两帧交替，终态静态。
#[test]
fn step_square_breathes_while_open_and_stays_static_when_settled() {
    let open = tool_call("read", ToolStatus::Running, Some(read_summary()), None);
    for frame in 0..text::agent::STEP_BREATH_TICKS {
        let (painted, _) = render_row_at(&open, 40, frame);
        assert!(
            painted.starts_with(text::agent::STEP_MARK_FILLED),
            "帧 {frame} 应为实心：{painted}"
        );
    }
    for frame in text::agent::STEP_BREATH_TICKS..text::agent::STEP_BREATH_TICKS * 2 {
        let (painted, _) = render_row_at(&open, 40, frame);
        assert!(
            painted.starts_with(text::agent::STEP_MARK_HOLLOW),
            "帧 {frame} 应为空心：{painted}"
        );
    }
    // 终态与帧无关：完成 / 失败 / 中断恒实心（取消恒空心，见下个测试）。
    for status in [
        ToolStatus::Completed,
        ToolStatus::Failed,
        ToolStatus::Interrupted,
    ] {
        let call = tool_call("read", status, Some(read_summary()), Some("boom"));
        for frame in [0usize, text::agent::STEP_BREATH_TICKS, 13] {
            let (painted, _) = render_row_at(&call, 40, frame);
            assert!(
                painted.starts_with(text::agent::STEP_MARK_FILLED),
                "{status:?} 帧 {frame} 应停为静态实心：{painted}"
            );
        }
    }
}

/// 完成态：静态翡翠绿方块 + 动词，不出现 braille 转轮。
#[test]
fn completed_tool_has_square_and_verb() {
    let cases: [(&str, ToolSummary, &str); 4] = [
        ("read", read_summary(), text::agent::VERB_TOOL_READ),
        ("search", search_summary(), text::agent::VERB_TOOL_SEARCH),
        ("edit", edit_summary(), text::agent::VERB_TOOL_EDIT),
        ("write", write_summary(), text::agent::VERB_TOOL_WRITE),
    ];
    for (name, summary, verb) in cases {
        let call = tool_call(name, ToolStatus::Completed, Some(summary), None);
        let (painted, grid) = render_row(&call, 80);
        assert!(
            !painted
                .chars()
                .any(|ch| ('\u{2800}'..='\u{28ff}').contains(&ch)),
            "{name} 完成态不得出现 braille 转轮：{painted}"
        );
        assert!(
            painted.starts_with(text::agent::STEP_MARK_FILLED),
            "{name} 行首为静态方块：{painted}"
        );
        let (header_line, _) = painted.split_once('\n').expect("两行");
        // 标题行 = `■ <动词>`：只有方块与固定动词。
        let expected = format!("{} {verb}", text::agent::STEP_MARK_FILLED);
        assert!(
            header_line.trim_end() == expected,
            "{name} 标题行为 `{expected}`：{header_line:?}"
        );
        // 不再有步骤图标：方块后直接是动词（类型图标只在 `[UI]` 查看器样例里）。
        for icon in [
            text::agent::ICON_READ,
            text::agent::ICON_SEARCH,
            text::agent::ICON_EDIT,
            text::agent::ICON_WRITE,
        ] {
            assert!(
                !header_line.contains(icon),
                "{name} 不再画步骤图标 {icon:?}：{header_line:?}"
            );
        }
        // 完成方块：翡翠绿、不加粗、**不压暗**。
        let lead = grid.cell(0, 0).style;
        assert_eq!(
            lead.fg,
            style::chat_step_mark_done().fg,
            "{name} 完成方块翡翠绿"
        );
        assert!(
            !lead.add_modifier.contains(ratatui::style::Modifier::BOLD),
            "{name} 完成方块不加粗（与运行中区分）"
        );
        assert!(
            !lead.add_modifier.contains(ratatui::style::Modifier::DIM),
            "{name} 完成方块不压暗"
        );
        assert_ne!(lead.fg, style::accent().fg, "{name} 不与边框青同色");
    }
}
