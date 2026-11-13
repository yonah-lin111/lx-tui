//! 连续同种工具调用合并与二级子树测试：组头聚合、二级缩进折行、异常语义色、
//! MCP 身份组、`wait_agent` 独立成卡、组内分隔行、快照合并与动态追加。

use ratatui::layout::Rect;

use super::*;
use crate::app::state::{
    AskAnsweredView, AskDetailView, AssistantStepView, CommandView, InteractionAction,
    McpDetailView, PlanSnapshotView, PlanStepView, ProcessInteractionView, ProcessStateView,
    SearchDetailView, SubagentDetailView, ToolCallView, ToolDetailView, ToolStatus, ToolSummary,
    WebSearchDetailView,
};
use crate::chat::agent::{AgentRole, AgentStatusView, WaitMode};

fn cache() -> PaneCache {
    PaneCache::default()
}

fn read_step(position: i64, status: ToolStatus) -> AssistantStepView {
    AssistantStepView::Tool {
        position,
        call: ToolCallView {
            call_id: format!("call-{position}"),
            name: "read".into(),
            position,
            arguments_json: String::new(),
            status,
            summary: Some(Box::new(ToolSummary {
                path: Some("src/ui/style.rs".into()),
                range: Some((12, 40)),
                read_lines: Some(29),
                ..ToolSummary::default()
            })),
            error: None,
            result_json: None,
            detail: None,
        },
    }
}

fn render_rows(area: Rect, lx: &Lx, scroll: usize) -> usize {
    let mut grid = Grid::new(area);
    render(&mut grid, area, lx, scroll, &mut cache())
}

fn rendered_text(lx: &Lx, area: Rect) -> String {
    let mut buf = Buffer::empty(area);
    crate::ui::main_content::lx::render_history_for_test(area, &mut buf, lx, 0);
    (0..area.height)
        .map(|y| row_text(&buf, area, y))
        .collect::<Vec<_>>()
        .join("\n")
}

fn card_hits(lx: &Lx, area: Rect) -> usize {
    layout(area, lx, &mut cache())
        .cards
        .iter()
        .filter(|hit| matches!(hit.kind, CardKind::Tool { .. }))
        .count()
}

/// 连续同名工具合并为一组：一个组头 + 逐调用树体，调用间只有一行分隔，失败调用连接符染红。
#[test]
fn consecutive_tool_steps_share_one_group_header() {
    let mut message = message("assistant", "complete", Vec::new());
    for (index, status) in [
        ToolStatus::Completed,
        ToolStatus::Failed,
        ToolStatus::Running,
    ]
    .into_iter()
    .enumerate()
    {
        let mut step = read_step(index as i64 + 1, status);
        if let AssistantStepView::Tool { call, .. } = &mut step
            && status == ToolStatus::Failed
        {
            call.error = Some("boom".into());
        }
        message.steps.push(step);
    }
    let lx = lx_with(vec![message]);
    let area = Rect::new(0, 0, 60, 14);
    let computed = layout(area, &lx, &mut cache());
    assert_eq!(
        computed.total_rows, 7,
        "合并组只用一个组头，非末调用各带一行分隔"
    );
    assert_eq!(render_rows(area, &lx, 0), 7);

    let mut buf = Buffer::empty(area);
    crate::ui::main_content::lx::render_history_for_test(area, &mut buf, &lx, 0);
    let rows: Vec<String> = (0..7).map(|row| row_text(&buf, area, row)).collect();
    assert!(rows[0].contains("3 calls"), "组头写调用数：{rows:?}");
    assert!(rows[0].contains("1 failed"), "组头聚合失败：{rows:?}");
    assert!(rows[0].contains("1 running"), "组头聚合运行中：{rows:?}");
    assert!(
        rows[1].starts_with(text::agent::TREE_BRANCH),
        "非末调用用 ├─：{rows:?}"
    );
    assert!(
        rows[1].contains("src/ui/style.rs:12-40 · 29 lines"),
        "组内 read 仍带结果体行数：{rows:?}"
    );
    assert!(
        rows[2].trim_end() == text::agent::TREE_BAR.trim_end(),
        "首个调用后有一行分隔：{rows:?}"
    );
    assert!(rows[3].contains("boom"), "{rows:?}");
    assert_eq!(
        Some(buf[(2, 3)].fg),
        style::agent_fail().fg,
        "失败调用连接符染红"
    );
    assert_eq!(
        buf[(5, 3)].fg,
        ratatui::style::Color::Reset,
        "错误正文保持正常前景色"
    );
    assert!(
        rows[5].starts_with(text::agent::TREE_LEAF),
        "末调用用 └─：{rows:?}"
    );
    assert!(rows[6].trim().is_empty(), "末尾呼吸行：{rows:?}");
}

/// 合并后的命令组保留每个调用的退出状态，组头只出现一次动词。
#[test]
fn command_group_keeps_each_exit_status_on_its_body() {
    let mut message = message("assistant", "complete", Vec::new());
    for (position, id, code) in [(1, 0, 0), (2, 1, 1)] {
        message.steps.push(command_step(position, id, code));
    }
    let lx = lx_with(vec![message]);
    let area = Rect::new(0, 0, 70, 12);
    let text = rendered_text(&lx, area);
    assert_eq!(
        text.matches(text::agent::VERB_EXEC_COMMAND).count(),
        1,
        "组头只出现一次 Run command：{text}"
    );
    assert!(text.contains("2 calls"), "{text}");
    assert!(text.contains("$ cargo check"), "{text}");
    assert!(text.contains("$ cargo test"), "{text}");
    assert!(text.contains("Exited 0"), "{text}");
    assert!(text.contains("exit 1"), "{text}");
    assert_eq!(
        render_rows(area, &lx, 0),
        layout(area, &lx, &mut cache()).total_rows,
        "组内每行都计入快照"
    );
}

/// 合并后的搜索组把总命中数与截断标记放组头，单调用命中数留在各自首行。
#[test]
fn search_group_aggregates_matches_and_keeps_each_status() {
    let mut message = message("assistant", "complete", Vec::new());
    for (position, matches) in [(1, 3), (2, 2)] {
        message.steps.push(search_step(position, matches));
    }
    let lx = lx_with(vec![message]);
    let area = Rect::new(0, 0, 80, 10);
    let text = rendered_text(&lx, area);
    assert_eq!(
        text.matches(text::agent::VERB_TOOL_SEARCH).count(),
        1,
        "组头只出现一次 Search：{text}"
    );
    assert!(text.contains("2 calls"), "{text}");
    assert!(text.contains("5 matches"), "组头汇总命中：{text}");
    assert!(text.contains("3 matches"), "首调用保留命中：{text}");
    assert!(text.contains("2 matches"), "次调用保留命中：{text}");
    assert_eq!(
        render_rows(area, &lx, 0),
        layout(area, &lx, &mut cache()).total_rows
    );
}

/// 组头不可点击，调用体各自可点。
#[test]
fn group_header_has_no_tool_card_region() {
    let mut message = message("assistant", "complete", Vec::new());
    message
        .steps
        .push(file_step(1, "call-a", "edit", ToolStatus::Completed));
    message
        .steps
        .push(file_step(2, "call-b", "edit", ToolStatus::Completed));
    let lx = lx_with(vec![message]);
    let area = Rect::new(0, 0, 60, 10);
    let mut cache = cache();
    let snapshot = cache.snapshot(super::content_rect(area).width, &lx);
    let header_start = snapshot
        .entries
        .first()
        .and_then(|entry| {
            entry.blocks.iter().find_map(|block| match block.source {
                BlockSource::ToolGroupHeader { .. } => Some(block.start),
                _ => None,
            })
        })
        .expect("组头块");
    assert!(
        snapshot.cards.iter().all(|card| match card.kind {
            CardKind::Tool { .. } => card.card_start != header_start,
            _ => true,
        }),
        "组头不得登记工具点击面：{:?}",
        snapshot.cards
    );
    assert_eq!(
        snapshot
            .cards
            .iter()
            .filter(|card| matches!(card.kind, CardKind::Tool { .. }))
            .count(),
        2,
        "两个调用体各自一个点击面"
    );
}

/// 合并组的分隔行只占行数，不进入任何调用体的点击面。
#[test]
fn group_separator_row_is_not_inside_any_tool_card_region() {
    let mut message = message("assistant", "complete", Vec::new());
    message
        .steps
        .push(file_step(1, "call-a", "edit", ToolStatus::Completed));
    message
        .steps
        .push(file_step(2, "call-b", "edit", ToolStatus::Completed));
    let lx = lx_with(vec![message]);
    let area = Rect::new(0, 0, 60, 12);
    let mut cache = cache();
    let snapshot = cache.snapshot(super::content_rect(area).width, &lx);
    let body_starts: Vec<usize> = snapshot
        .entries
        .iter()
        .flat_map(|entry| entry.blocks.iter())
        .filter_map(|block| match block.source {
            BlockSource::ToolBody { .. } => Some(block.start),
            _ => None,
        })
        .collect();
    assert!(body_starts.len() >= 2, "两个调用体块：{body_starts:?}");
    let separator_row = body_starts[1].saturating_sub(1);
    let first_cards: Vec<_> = snapshot
        .cards
        .iter()
        .filter(|card| matches!(&card.kind, CardKind::Tool { call_id, .. } if call_id == "call-a"))
        .collect();
    assert!(!first_cards.is_empty(), "首调用应有可点击区域");
    for card in first_cards {
        assert!(
            card.card_start + card.card_rows <= separator_row,
            "点击面不得覆盖分隔行：card={card:?} separator_row={separator_row}"
        );
    }
}

/// `write_stdin` 按显示动词分组：读输出与写入不混组，且后台运行中进程使组头聚合 `running`。
#[test]
fn write_stdin_groups_by_display_verb() {
    let mut message = message("assistant", "complete", Vec::new());
    message.steps.push(write_stdin_step(
        1,
        InteractionAction::ReadOutput,
        "cargo test",
    ));
    message.steps.push(write_stdin_step(
        2,
        InteractionAction::ReadOutput,
        "cargo test",
    ));
    message.steps.push(write_stdin_step(
        3,
        InteractionAction::SendInput,
        "cargo test",
    ));
    let lx = lx_with(vec![message]);
    let area = Rect::new(0, 0, 80, 16);
    let text = rendered_text(&lx, area);
    assert_eq!(
        text.matches(text::agent::VERB_READ_OUTPUT).count(),
        1,
        "读输出组头只出现一次：{text}"
    );
    assert!(text.contains("2 calls"), "{text}");
    assert!(
        text.contains("2 running"),
        "后台运行中进程必须反映到组头：{text}"
    );
    assert_eq!(
        text.matches(text::agent::VERB_SEND_INPUT).count(),
        1,
        "写入单独成组：{text}"
    );
}

/// `ask` 是阻塞交互，连续两次也各自一张卡。
#[test]
fn ask_calls_are_never_grouped() {
    let mut message = message("assistant", "complete", Vec::new());
    message.steps.push(ask_step(1, "first?"));
    message.steps.push(ask_step(2, "second?"));
    let lx = lx_with(vec![message]);
    let area = Rect::new(0, 0, 60, 12);
    let text = rendered_text(&lx, area);
    assert_eq!(
        text.matches(text::agent::VERB_TOOL_ASK).count(),
        2,
        "每次 ask 都有自己的标题：{text}"
    );
    assert!(!text.contains("2 calls"), "{text}");
}

/// 计划快照只显示最新一次，组头写 updates + 完成进度，步骤行不拼 `·`。
#[test]
fn plan_snapshot_group_shows_only_the_latest_update_and_header_progress() {
    let mut message = message("assistant", "complete", Vec::new());
    message.steps.push(plan_step(1, "old step"));
    message.steps.push(plan_step(2, "new step"));
    let lx = lx_with(vec![message]);
    let area = Rect::new(0, 0, 60, 10);
    let text = rendered_text(&lx, area);
    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(
        text.matches(text::agent::VERB_UPDATE_PLAN).count(),
        1,
        "组头只出现一次 Plan：{text}"
    );
    assert!(lines[0].contains("2 updates"), "{text}");
    assert!(lines[0].contains("1/1 completed"), "进度显示在组头：{text}");
    assert!(lines[1].contains("new step"), "{text}");
    assert!(
        !lines[1].contains("completed"),
        "步骤行不再拼接完成进度：{text}"
    );
    assert!(!text.contains("old step"), "旧快照不铺列表：{text}");
    assert!(
        !lines
            .iter()
            .any(|line| line.trim_end() == text::agent::TREE_BAR.trim_end()),
        "快照合并只有一个调用体，不得出现调用间分隔行：{lines:?}"
    );
    assert_eq!(
        render_rows(area, &lx, 0),
        layout(area, &lx, &mut cache()).total_rows
    );
}

/// 连续文件工具合并为一个组头；每个调用仍有独立点击面，调用间只有一行分隔。
#[test]
fn consecutive_file_tools_share_a_group_header() {
    let area = Rect::new(0, 0, 60, 24);
    let mut message = message("assistant", "complete", Vec::new());
    message
        .steps
        .push(file_step(1, "call-a", "edit", ToolStatus::Completed));
    message
        .steps
        .push(file_step(2, "call-b", "edit", ToolStatus::Completed));
    let lx = lx_with(vec![message.clone()]);
    let computed = layout(area, &lx, &mut cache());
    assert_eq!(card_hits(&lx, area), 2, "两个调用各一张点击面");
    assert_eq!(render_rows(area, &lx, 0), computed.total_rows);
    let text = rendered_text(&lx, area);
    assert_eq!(
        text.matches(text::agent::VERB_TOOL_EDIT).count(),
        1,
        "组头只出现一次 Edit：{text}"
    );
    assert!(text.contains("2 calls"), "组头写调用数：{text}");
    assert!(!text.contains("2 files"), "不再出现旧合并标题：{text}");
    let mut buf = Buffer::empty(area);
    crate::ui::main_content::lx::render_history_for_test(area, &mut buf, &lx, 0);
    let tool_cards: Vec<_> = computed
        .cards
        .iter()
        .filter(|card| matches!(card.kind, CardKind::Tool { .. }))
        .collect();
    let blanks: Vec<usize> = (usize::from(tool_cards[0].card.bottom())
        ..usize::from(tool_cards[1].card.y))
        .filter(|row| {
            row_text(&buf, area, u16::try_from(*row).unwrap_or(u16::MAX))
                .trim()
                .is_empty()
        })
        .collect();
    assert_eq!(blanks.len(), 0, "组内调用之间不空行：{blanks:?}");
    let lines: Vec<String> = text.lines().map(str::to_string).collect();
    assert!(
        lines[1].starts_with(text::agent::TREE_BRANCH) && lines[1].contains("src/file1.rs"),
        "首调用文件头用 ├─：{lines:?}"
    );
    assert!(
        lines[2].starts_with(text::agent::TREE_NESTED_BRANCH_LEAF),
        "首调用 diff 首行缩进为二级 └─：{lines:?}"
    );
    assert!(
        lines[3].starts_with(text::agent::TREE_NESTED_BRANCH_PAD),
        "首调用 diff 续行保留外层竖线：{lines:?}"
    );
    let second_header = lines
        .iter()
        .position(|line| line.contains("src/file2.rs"))
        .expect("第二个调用文件头");
    assert!(
        lines[second_header].starts_with(text::agent::TREE_LEAF),
        "末调用文件头用 └─ 收口：{lines:?}"
    );
    assert!(
        lines[second_header + 1].starts_with(text::agent::TREE_NESTED_LEAF_LEAF),
        "末调用 diff 首行缩进为二级 └─：{lines:?}"
    );
    assert!(
        lines[second_header + 2].starts_with(text::agent::TREE_NESTED_LEAF_PAD),
        "末调用 diff 续行外层留白：{lines:?}"
    );
    let untinted_cols = u16::try_from(
        text::agent::TREE_NESTED_BRANCH_LEAF
            .chars()
            .count()
            .saturating_sub(1),
    )
    .expect("7 列");
    for x in 0..untinted_cols {
        assert_eq!(
            buf[(x, 2)].bg,
            ratatui::style::Color::Reset,
            "diff 左侧树连接符列 {x} 不铺背景色"
        );
    }
    assert_eq!(
        Some(buf[(untinted_cols, 2)].bg),
        style::agent_removed_bg().bg,
        "diff 背景从 └─ 右侧空列起铺底色"
    );

    let mut expanded = lx_with(vec![message.clone()]);
    assert!(expanded.chat.toggle_detail("assistant-1", "call-a"));
    let expanded_layout = layout(area, &expanded, &mut cache());
    assert_eq!(render_rows(area, &expanded, 0), expanded_layout.total_rows);
    assert_eq!(card_hits(&expanded, area), 2, "展开后仍是两个调用体");
}

/// 正文步骤打断即分组：`edit` → 正文 → `edit` 是两张卡。
#[test]
fn a_part_step_separates_two_file_cards() {
    let mut message = message("assistant", "complete", vec![text_part("中间说一句")]);
    message.steps = vec![
        file_step(1, "call-a", "edit", ToolStatus::Completed),
        AssistantStepView::Part {
            position: 2,
            part: text_part("中间说一句"),
        },
        file_step(3, "call-b", "edit", ToolStatus::Completed),
    ];
    let lx = lx_with(vec![message]);
    let area = Rect::new(0, 0, 60, 20);
    assert_eq!(card_hits(&lx, area), 2, "正文打断合并");
    assert_eq!(
        render_rows(area, &lx, 0),
        layout(area, &lx, &mut cache()).total_rows
    );
}

/// 连续 `web_search` 合并组：查询行作为一级分支（`├─`/`└─`），结果块缩进为二级子树。
#[test]
fn web_search_group_nests_results_under_each_query() {
    fn web_search_step(position: i64, query: &str, title: &str, url: &str) -> AssistantStepView {
        AssistantStepView::Tool {
            position,
            call: ToolCallView {
                call_id: format!("call-ws-{position}"),
                name: "web_search".into(),
                position,
                arguments_json: String::new(),
                status: ToolStatus::Completed,
                summary: None,
                error: None,
                result_json: None,
                detail: Some(Box::new(ToolDetailView::WebSearch(WebSearchDetailView {
                    queries: vec![query.into()],
                    search_type: "fast".into(),
                    num_results: 2,
                    provider: Some("exa".into()),
                    result: vec![
                        format!("Title: {title}"),
                        format!("URL: {url}"),
                        "Published: N/A".into(),
                        "snippet line".into(),
                    ],
                    result_more: 10,
                    duration_ms: Some(250),
                    truncated: false,
                }))),
            },
        }
    }

    let mut message = message("assistant", "complete", Vec::new());
    message.steps.push(web_search_step(
        1,
        "ratatui rust tui",
        "Ratatui",
        "https://ratatui.rs/",
    ));
    message.steps.push(web_search_step(
        2,
        "crossterm rust terminal",
        "crossterm",
        "https://crates.io/crates/crossterm",
    ));
    let lx = lx_with(vec![message]);
    let area = Rect::new(0, 0, 80, 16);
    let text = rendered_text(&lx, area);
    let lines: Vec<&str> = text.lines().collect();
    assert!(lines[0].contains("Web search") && lines[0].contains("2 calls"));
    assert!(
        lines[1].starts_with(text::agent::TREE_BRANCH)
            && lines[1].contains("\"ratatui rust tui\"")
            && lines[1].contains("exa"),
        "第一调用查询行用 ├─：{lines:?}"
    );
    assert!(
        lines[2].starts_with(text::agent::TREE_NESTED_BRANCH_LEAF)
            && lines[2].contains("Title: Ratatui"),
        "第一调用结果首行缩进为二级 └─：{lines:?}"
    );
    assert!(
        lines[3].starts_with(text::agent::TREE_NESTED_BRANCH_PAD)
            && lines[3].contains("https://ratatui.rs/"),
        "第一调用结果续行保留外层竖线：{lines:?}"
    );
    assert!(
        lines[5].starts_with(text::agent::TREE_NESTED_BRANCH_PAD) && lines[5].contains("more line"),
        "第一调用折叠提示保留外层竖线：{lines:?}"
    );
    assert!(
        lines[6].trim_end() == text::agent::TREE_BAR.trim_end(),
        "两调用之间有一行分隔：{lines:?}"
    );
    assert!(
        lines[7].starts_with(text::agent::TREE_LEAF)
            && lines[7].contains("\"crossterm rust terminal\""),
        "第二调用查询行用 └─ 收口：{lines:?}"
    );
    assert!(
        lines[8].starts_with(text::agent::TREE_NESTED_LEAF_LEAF)
            && lines[8].contains("Title: crossterm"),
        "第二调用结果首行缩进为二级 └─：{lines:?}"
    );
    assert!(
        lines[9].starts_with(text::agent::TREE_NESTED_LEAF_PAD)
            && lines[9].contains("https://crates.io/crates/crossterm"),
        "第二调用结果续行外层留白：{lines:?}"
    );
    assert_eq!(
        render_rows(area, &lx, 0),
        layout(area, &lx, &mut cache()).total_rows
    );
}

/// P0-1 回归：合并组内二级子节点按 `width - 8` 折行，折行末尾 3 列字符不得被右边界裁切。
#[test]
fn grouped_child_nodes_wrap_at_nested_width_without_clipping() {
    let mut cmd1 = command_step(1, 0, 0);
    if let AssistantStepView::Tool { call, .. } = &mut cmd1
        && let Some(ToolDetailView::Command(view)) = call.detail.as_deref_mut()
    {
        // 在宽 20（一级 15 列、二级 12 列）下：14 字符行必须在第 12 列处折行，末尾 "MN" 折到下一行而非被裁掉。
        view.output_tail = "ABCDEFGHIJKLMN".into();
    }
    let cmd2 = command_step(2, 1, 0);
    let mut message = message("assistant", "complete", Vec::new());
    message.steps.push(cmd1);
    message.steps.push(cmd2);
    let lx = lx_with(vec![message]);
    let area = Rect::new(0, 0, 22, 12);
    let text = rendered_text(&lx, area);
    assert!(
        text.contains("ABCDEFGHIJKLM") && text.contains("\n  │     N"),
        "二级子节点折行不得丢失末尾字符：{text}"
    );
    assert_eq!(
        render_rows(area, &lx, 0),
        layout(area, &lx, &mut cache()).total_rows
    );
}

/// 相邻 MCP 调用（不同 server/tool）合并为一个 `MCP` 组：server 一级、tool 二级、调用间空行。
#[test]
fn adjacent_mcp_calls_merge_into_one_identity_group() {
    fn mcp_step(
        position: i64,
        name: &str,
        server: &str,
        tool: &str,
        duration_ms: u64,
        result: &str,
    ) -> AssistantStepView {
        AssistantStepView::Tool {
            position,
            call: ToolCallView {
                call_id: format!("call-mcp-{position}"),
                name: name.into(),
                position,
                arguments_json: "{}".into(),
                status: ToolStatus::Completed,
                summary: None,
                error: None,
                result_json: None,
                detail: Some(Box::new(ToolDetailView::Mcp(McpDetailView {
                    server: server.into(),
                    tool: tool.into(),
                    summary: None,
                    arguments: Vec::new(),
                    arguments_more: 0,
                    result: vec![result.into()],
                    result_more: 0,
                    images: Vec::new(),
                    images_omitted: 0,
                    duration_ms: Some(duration_ms),
                }))),
            },
        }
    }

    let mut message = message("assistant", "complete", Vec::new());
    message.steps.push(mcp_step(
        1,
        "mcp__codebase-memory-mcp__list_projects",
        "codebase-memory-mcp",
        "list_projects",
        4,
        "{\"count\": 1}",
    ));
    // resources 桥接工具也进同一个 MCP 组。
    message.steps.push(mcp_step(
        2,
        "read_resource",
        "codegraph",
        "codegraph_explore",
        5,
        "{\"count\": 2}",
    ));
    let lx = lx_with(vec![message]);
    let area = Rect::new(0, 0, 70, 12);
    let text = rendered_text(&lx, area);
    let lines: Vec<&str> = text.lines().collect();
    assert!(
        lines[0].contains(text::agent::VERB_MCP) && lines[0].contains("2 calls"),
        "不同 MCP 工具也进同一个 MCP 组：{lines:?}"
    );
    assert!(
        lines[1].starts_with(text::agent::TREE_BRANCH)
            && lines[1].contains("codebase-memory-mcp")
            && lines[1].contains("4ms"),
        "server 是一级分支并承载耗时：{lines:?}"
    );
    assert!(
        lines[2].starts_with(text::agent::TREE_NESTED_BRANCH_LEAF)
            && lines[2].contains("list_projects"),
        "tool 缩进为二级叶子：{lines:?}"
    );
    assert!(
        lines[3].trim_end() == text::agent::TREE_BAR.trim_end(),
        "调用体之间有一行 │ 分隔：{lines:?}"
    );
    assert!(
        lines[4].starts_with(text::agent::TREE_LEAF)
            && lines[4].contains("codegraph")
            && lines[4].contains("5ms"),
        "末调用 server 用一级 └─ 收口：{lines:?}"
    );
    assert!(
        lines[5].starts_with(text::agent::TREE_NESTED_LEAF_LEAF)
            && lines[5].contains("codegraph_explore"),
        "末调用 tool 缩进为二级叶子：{lines:?}"
    );
    assert!(
        !text.contains("{\"count\""),
        "MCP 身份卡不得显示结果正文：{text}"
    );
}

/// `wait_agent` 不合并：连续等待各自成卡，目标行在卡内保持一级平级分支。
#[test]
fn consecutive_wait_agent_calls_stay_as_separate_cards() {
    fn wait_step(position: i64, targets: &[(&str, &str)]) -> AssistantStepView {
        let agents: Vec<AgentStatusView> = targets
            .iter()
            .map(|(id, nick)| AgentStatusView {
                agent_id: (*id).into(),
                nickname: Some((*nick).into()),
                agent_path: None,
                role: AgentRole::Explorer,
                title: "task".into(),
                status: "completed".into(),
                report: None,
            })
            .collect();
        AssistantStepView::Tool {
            position,
            call: ToolCallView {
                call_id: format!("call-wait-{position}"),
                name: "wait_agent".into(),
                position,
                arguments_json: String::new(),
                status: ToolStatus::Completed,
                summary: None,
                error: None,
                result_json: None,
                detail: Some(Box::new(ToolDetailView::Subagent(SubagentDetailView {
                    role: None,
                    nickname: None,
                    title: None,
                    task: None,
                    agent_id: None,
                    agent_path: None,
                    live_activity: None,
                    agent_status: None,
                    model_label: None,
                    fork_context: false,
                    targets: targets.iter().map(|(id, _)| (*id).into()).collect(),
                    all: false,
                    mode: WaitMode::All,
                    timeout_ms: None,
                    timed_out: false,
                    agents,
                    report: None,
                    closed_status: None,
                }))),
            },
        }
    }

    let mut message = message("assistant", "complete", Vec::new());
    message
        .steps
        .push(wait_step(1, &[("a1", "Alpha"), ("a2", "Beta")]));
    message.steps.push(wait_step(2, &[("a3", "Gamma")]));
    let lx = lx_with(vec![message]);
    let area = Rect::new(0, 0, 80, 10);
    let text = rendered_text(&lx, area);
    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(
        text.matches("Wait agents").count(),
        2,
        "连续 wait_agent 必须各自成卡：{lines:?}"
    );
    assert!(
        !text.contains("2 calls"),
        "wait_agent 不得进入合并组：{lines:?}"
    );
    assert!(
        lines[1].starts_with(text::agent::TREE_BRANCH) && lines[1].contains("Alpha"),
        "首调用第 1 个目标为一级 ├─：{lines:?}"
    );
    assert!(
        lines[2].starts_with(text::agent::TREE_LEAF) && lines[2].contains("Beta"),
        "首调用最后一个目标用一级 └─ 收口：{lines:?}"
    );
    assert!(
        lines[4].contains("Wait agents") && lines[4].contains("1 target"),
        "第二个 wait_agent 是独立卡片：{lines:?}"
    );
    assert!(
        lines[5].starts_with(text::agent::TREE_LEAF) && lines[5].contains("Gamma"),
        "第二张卡的目标仍是一级平级分支：{lines:?}"
    );
}

/// 运行中追加同种调用：原末项连接符从 `└─` 变为 `├─`，总行数同步更新。
#[test]
fn appending_same_tool_updates_previous_leaf_connector_to_branch() {
    let area = Rect::new(0, 0, 60, 12);
    let mut message = message("assistant", "streaming", Vec::new());
    message.steps.push(read_step(1, ToolStatus::Completed));
    message.steps.push(read_step(2, ToolStatus::Completed));
    let mut lx = lx_with(vec![message]);
    let mut cache = cache();

    let mut grid = Grid::new(area);
    let rows_two = render(&mut grid, area, &lx, 0, &mut cache);
    assert_eq!(rows_two, 5);
    let second_before: String = (0..area.width).map(|x| grid.cell(x, 3).ch).collect();
    assert!(
        second_before.starts_with(text::agent::TREE_LEAF),
        "追加前第 2 个调用是 └─：{second_before:?}"
    );

    lx.chat.messages[0]
        .steps
        .push(read_step(3, ToolStatus::Running));
    let mut grid = Grid::new(area);
    let rows_three = render(&mut grid, area, &lx, 0, &mut cache);
    assert_eq!(rows_three, 7);
    let second_after: String = (0..area.width).map(|x| grid.cell(x, 3).ch).collect();
    let third_after: String = (0..area.width).map(|x| grid.cell(x, 5).ch).collect();
    assert!(
        second_after.starts_with(text::agent::TREE_BRANCH),
        "追加后原末项必须从 └─ 更新为 ├─：{second_after:?}"
    );
    assert!(
        third_after.starts_with(text::agent::TREE_LEAF),
        "新末项使用 └─ 收口：{third_after:?}"
    );
}

fn write_stdin_step(position: i64, action: InteractionAction, command: &str) -> AssistantStepView {
    AssistantStepView::Tool {
        position,
        call: ToolCallView {
            call_id: format!("call-stdin-{position}"),
            name: "write_stdin".into(),
            position,
            arguments_json: String::new(),
            status: ToolStatus::Completed,
            summary: None,
            error: None,
            result_json: None,
            detail: Some(Box::new(ToolDetailView::Interaction(
                ProcessInteractionView {
                    action,
                    process_session_id: 1,
                    command: command.into(),
                    input_preview: (action == InteractionAction::SendInput)
                        .then(|| "y\\n".to_string()),
                    output: "ok".into(),
                    state: ProcessStateView::Running,
                    exit_code: None,
                    duration_ms: None,
                    truncated: false,
                    dropped_bytes: 0,
                },
            ))),
        },
    }
}

fn ask_step(position: i64, question: &str) -> AssistantStepView {
    AssistantStepView::Tool {
        position,
        call: ToolCallView {
            call_id: format!("call-ask-{position}"),
            name: "ask".into(),
            position,
            arguments_json: String::new(),
            status: ToolStatus::Completed,
            summary: None,
            error: None,
            result_json: None,
            detail: Some(Box::new(ToolDetailView::Ask(AskDetailView {
                questions: vec![AskAnsweredView {
                    header: "Q".into(),
                    question: question.into(),
                    answers: vec!["yes".into()],
                }],
            }))),
        },
    }
}

fn plan_step(position: i64, step: &str) -> AssistantStepView {
    AssistantStepView::Tool {
        position,
        call: ToolCallView {
            call_id: format!("call-plan-{position}"),
            name: "update_plan".into(),
            position,
            arguments_json: String::new(),
            status: ToolStatus::Completed,
            summary: None,
            error: None,
            result_json: None,
            detail: Some(Box::new(ToolDetailView::Plan(PlanSnapshotView {
                revision: position as u64,
                explanation: None,
                steps: vec![(step.to_string(), PlanStepView::Completed)],
            }))),
        },
    }
}

fn command_step(position: i64, id: i32, exit: i32) -> AssistantStepView {
    AssistantStepView::Tool {
        position,
        call: ToolCallView {
            call_id: format!("call-cmd-{position}"),
            name: "exec_command".into(),
            position,
            arguments_json: String::new(),
            status: ToolStatus::Completed,
            summary: None,
            error: None,
            result_json: None,
            detail: Some(Box::new(ToolDetailView::Command(CommandView {
                process_session_id: Some(id.unsigned_abs().into()),
                command: if id == 0 {
                    "cargo check".into()
                } else {
                    "cargo test".into()
                },
                cwd: "/w".into(),
                tty: false,
                state: ProcessStateView::Exited { code: Some(exit) },
                duration_ms: Some(1_200),
                output_preview: Vec::new(),
                truncated: false,
                dropped_bytes: 0,
                output_tail: String::new(),
                started_at_ms: None,
                ended_at_ms: None,
            }))),
        },
    }
}

fn search_step(position: i64, matches: usize) -> AssistantStepView {
    AssistantStepView::Tool {
        position,
        call: ToolCallView {
            call_id: format!("call-search-{position}"),
            name: "search".into(),
            position,
            arguments_json: String::new(),
            status: ToolStatus::Completed,
            summary: Some(Box::new(ToolSummary {
                path: Some("src".into()),
                pattern: Some("fixme".into()),
                matches: Some(matches),
                ..ToolSummary::default()
            })),
            error: None,
            result_json: None,
            detail: Some(Box::new(ToolDetailView::Search(SearchDetailView {
                matches: Vec::new(),
                truncated: false,
                files_scanned: 1,
            }))),
        },
    }
}

fn file_step(position: i64, call_id: &str, name: &str, status: ToolStatus) -> AssistantStepView {
    use crate::app::state::{FileChangeView, FileDiffDetailView, FileDiffView};
    use crate::chat::tools::{DiffHunk, DiffLine, DiffLineKind, FileChangeKind, FileDiff};

    let diff = FileDiff::Complete {
        hunks: vec![DiffHunk {
            lines: (1..=2)
                .flat_map(|n| {
                    [
                        DiffLine {
                            kind: DiffLineKind::Removed,
                            old_line: Some(n),
                            new_line: None,
                            text: format!("old{n}"),
                            no_final_newline: false,
                        },
                        DiffLine {
                            kind: DiffLineKind::Added,
                            old_line: None,
                            new_line: Some(n),
                            text: format!("new{n}"),
                            no_final_newline: false,
                        },
                    ]
                })
                .collect(),
        }],
        simplified: false,
    };
    AssistantStepView::Tool {
        position,
        call: ToolCallView {
            call_id: call_id.into(),
            name: name.into(),
            position,
            arguments_json: String::new(),
            status,
            summary: None,
            error: None,
            result_json: None,
            detail: Some(Box::new(crate::app::state::ToolDetailView::File(
                FileDiffDetailView {
                    files: vec![FileChangeView {
                        path: format!("src/file{position}.rs"),
                        to: None,
                        kind: FileChangeKind::Updated,
                        added: Some(1),
                        removed: Some(1),
                        counts_known: true,
                        simplified: false,
                        diff: Some(FileDiffView::from(&diff)),
                    }],
                    files_total: 1,
                    added: Some(1),
                    removed: Some(1),
                    simplified: false,
                    omitted_files: 0,
                    single: false,
                },
            ))),
        },
    }
}
