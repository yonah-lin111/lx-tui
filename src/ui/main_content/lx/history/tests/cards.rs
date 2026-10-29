//! 卡片命中与指纹：命令/文件卡片的点击面、超长历史命中、差异指纹。

use super::*;

/// 构造只有一个汇报块的历史（正文 `body` 折行后足够长时才有更多提示）。
fn lx_with_report(body: &str) -> Lx {
    let mut message = message("assistant", "complete", Vec::new());
    message.reports.push(crate::app::state::SubagentReportView {
        agent_id: "sub-1".into(),
        title: "report".into(),
        role: crate::chat::agent::AgentRole::Explorer,
        status: "completed".into(),
        files_changed: 1,
        duration_ms: Some(1_000),
        body: body.into(),
        delivered_at_ms: 1,
    });
    lx_with(vec![message])
}

/// 构造一个已结束的命令卡片。
fn finished_command(command: &str, output: &str) -> crate::app::state::ToolCallView {
    use crate::app::state::{
        CommandView, ProcessStateView, ToolCallView, ToolDetailView, ToolStatus,
    };
    ToolCallView {
        call_id: "call-1".into(),
        name: "exec_command".into(),
        position: 0,
        arguments_json: String::new(),
        status: ToolStatus::Completed,
        summary: None,
        error: None,
        result_json: None,
        detail: Some(Box::new(ToolDetailView::Command(CommandView {
            process_session_id: Some(1),
            command: command.into(),
            cwd: "/workspace/lx".into(),
            tty: false,
            duration_ms: None,
            state: ProcessStateView::Exited { code: Some(0) },
            output_preview: Vec::new(),
            truncated: false,
            dropped_bytes: 0,
            output_tail: output.into(),
            started_at_ms: Some(0),
            ended_at_ms: Some(100),
        }))),
    }
}

/// 带一个命令卡片的历史（消息下标 0 是 assistant，卡片是它的第二步）。
fn lx_with_command(call: crate::app::state::ToolCallView) -> Lx {
    let mut lx = lx_with(vec![message(
        "assistant",
        "complete",
        vec![text_part("跑一下")],
    )]);
    lx.chat.messages[0]
        .steps
        .push(AssistantStepView::Tool { position: 1, call });
    lx
}

/// 点击落在某块首行时必须命中该块；用来验证「折叠提示行不能吞掉点击」。
fn target_at_block_start(
    area: Rect,
    lx: &Lx,
    cache: &mut PaneCache,
    section: crate::app::state::CardSection,
) -> Option<crate::app::state::CardTarget> {
    use crate::ui::main_content::lx::history::CardKind;
    let width = super::content_rect(area).width;
    let snapshot = cache.snapshot(width, lx);
    let region = snapshot.cards.iter().find(
        |card| matches!(&card.kind, CardKind::Tool { section: hit, .. } if *hit == section),
    )?;
    let view = crate::ui::main_content::lx::history_view(area, lx, cache)?;
    let row = view.body.y.saturating_add(super::rows_to_u16(
        region.card_start.saturating_sub(view.scroll),
    ));
    crate::ui::main_content::lx::card_target_at(area, lx, view.body.x + 2, row, cache)
}

/// 回归：折叠态末尾那行 `… N more lines` 是提示，不是内容。
///
/// 展开只比折叠预览多一行时，折叠总行数（2 行预览 + 1 行提示）与展开总行数相等；
/// 旧判据用「展开总行数 > 折叠总行数」，于是卡片明明写着「… 1 more line」，
/// 点它却没有反应。
#[test]
fn folded_hint_row_does_not_swallow_the_click() {
    use crate::app::state::{CardSection, CardTarget};
    // 展开后恰好 3 行（折叠 2 行 + 提示行 = 3 行）。
    let command = "$ echo \"=== 当前用户/系统 ===\"; whoami; uname -a; echo \"=== 桌面路径 ===\"; ls -d ~/Desktop 2>/dev/null && echo \"桌面存在\" || echo \"无 ~/Desktop\"; echo \"=== 工作目录 ===\"; pwd";
    let output = (0..12)
        .map(|line| format!("output line {line}"))
        .collect::<Vec<_>>()
        .join("\n");
    let lx = lx_with_command(finished_command(command, &output));
    let area = Rect::new(0, 0, 80, 40);
    let mut cache = PaneCache::default();
    assert_eq!(
        target_at_block_start(area, &lx, &mut cache, CardSection::Body),
        Some(CardTarget::Tool {
            message_id: "assistant-1".into(),
            call_id: "call-1".into(),
            section: CardSection::Body,
        }),
        "参数区的提示行必须可点"
    );
}

/// 回归：输出只比折叠预览多一行时，点击输出区也必须能展开。
#[test]
fn short_output_above_preview_still_expands() {
    use crate::app::state::{CardSection, CardTarget};
    // 折叠预览 = 首 3 + 尾 2 + 提示行 = 6 行；展开 = 6 行——旧判据同样判成不可展开。
    let output = (0..6)
        .map(|line| format!("line {line}"))
        .collect::<Vec<_>>()
        .join("\n");
    let lx = lx_with_command(finished_command("$ ls", &output));
    let area = Rect::new(0, 0, 80, 40);
    let mut cache = PaneCache::default();
    assert_eq!(
        target_at_block_start(area, &lx, &mut cache, CardSection::Output),
        Some(CardTarget::Tool {
            message_id: "assistant-1".into(),
            call_id: "call-1".into(),
            section: CardSection::Output,
        }),
        "输出区必须可点"
    );
}

/// 一条文件工具调用（`write`，带一份小差异）。
fn file_call(call_id: &str, path: &str) -> crate::app::state::ToolCallView {
    use crate::app::state::{
        DiffHunkView, DiffLineView, FileChangeView, FileDiffDetailView, FileDiffView, ToolCallView,
        ToolDetailView, ToolStatus, ToolSummary,
    };
    use crate::chat::tools::{DiffLineKind, FileChangeKind};
    ToolCallView {
        call_id: call_id.into(),
        name: "write".into(),
        position: 0,
        arguments_json: String::new(),
        status: ToolStatus::Completed,
        summary: Some(Box::new(ToolSummary {
            path: Some(path.into()),
            added: Some(1),
            removed: Some(0),
            counts_known: true,
            ..ToolSummary::default()
        })),
        error: None,
        result_json: None,
        detail: Some(Box::new(ToolDetailView::File(FileDiffDetailView {
            files: vec![FileChangeView {
                path: path.into(),
                to: None,
                kind: FileChangeKind::Added,
                added: Some(1),
                removed: Some(0),
                counts_known: true,
                simplified: false,
                diff: Some(FileDiffView::Complete {
                    hunks: vec![DiffHunkView {
                        lines: vec![DiffLineView {
                            kind: DiffLineKind::Added,
                            old_line: None,
                            new_line: Some(1),
                            text: "hello".into(),
                            no_final_newline: false,
                        }],
                    }],
                    simplified: false,
                }),
            }],
            files_total: 1,
            added: Some(1),
            removed: Some(0),
            simplified: false,
            omitted_files: 0,
            single: false,
        }))),
    }
}

/// 消息内**每两块之间都必须空一行**，合并文件组也不例外。
///
/// 回归：快照重写时漏了合并组的块前空行，正文下面直接顶着 `▣ Write 2 files`。
#[test]
fn every_block_is_separated_by_a_blank_row() {
    // 正文 + 两步连续 write（合并成一组）+ 一段思考 + 一句正文。
    let mut lx = lx_with(vec![message(
        "assistant",
        "complete",
        vec![
            text_part("测试 1: write — 在桌面创建文件和目录"),
            reasoning_part("先写两个文件"),
            text_part("收尾"),
        ],
    )]);
    for (step, call) in [
        file_call("call-1", "/Users/yonah/Desktop/tool-test/hello.txt"),
        file_call("call-2", "/Users/yonah/Desktop/tool-test/BubbleSort.java"),
    ]
    .into_iter()
    .enumerate()
    {
        lx.chat.messages[0].steps.insert(
            1 + step,
            AssistantStepView::Tool {
                position: 1 + step as i64,
                call,
            },
        );
    }

    let mut cache = PaneCache::default();
    let width = super::content_rect(Rect::new(0, 0, 80, 40)).width;
    let snapshot = cache.snapshot(width, &lx);
    let entry = snapshot.entries.first().expect("有一条消息");
    assert!(
        entry.blocks.len() >= 4,
        "至少 4 块（正文/合并组/thinking/正文），实际 {}",
        entry.blocks.len()
    );
    for pair in entry.blocks.windows(2) {
        assert_eq!(
            pair[1].start,
            pair[0].start + pair[0].rows + 1,
            "块之间必须空一行：{:?} 结束于 {}，下一块从 {} 开始",
            pair[0].source,
            pair[0].start + pair[0].rows,
            pair[1].start
        );
    }
}

/// 圆点给文字（正文 / 思考 / 通知），方块只给工具调用，上三角只给子 agent 汇报；
/// 终态（含取消）一律实心。
#[test]
fn step_marks_use_dots_squares_and_report_triangles() {
    use crate::ui::text;
    for kind in [super::StepKind::Body, super::StepKind::Thinking] {
        assert_eq!(
            super::StepMark::Done.glyph(kind, 0, false),
            text::agent::STEP_MARK_DOT_FILLED,
            "{kind:?} 完成态必须是圆点"
        );
        // 取消不再是空心：空心会被读成「还在跑」，终态一律实心。
        assert_eq!(
            super::StepMark::Cancelled.glyph(kind, 0, false),
            text::agent::STEP_MARK_DOT_FILLED,
            "{kind:?} 取消态也是实心圆点"
        );
    }
    assert_eq!(
        super::StepMark::Done.glyph(super::StepKind::Tool, 0, false),
        text::agent::STEP_MARK_FILLED,
        "工具调用完成态必须是方块"
    );
    assert_eq!(
        super::StepMark::Cancelled.glyph(super::StepKind::Tool, 0, false),
        text::agent::STEP_MARK_FILLED,
        "工具调用取消态也是实心方块"
    );
    assert_eq!(
        super::StepMark::Done.glyph(super::StepKind::Report, 0, false),
        text::agent::STEP_MARK_TRIANGLE_FILLED,
        "子 agent 汇报完成态必须是上三角"
    );
    assert_eq!(
        super::StepMark::Cancelled.glyph(super::StepKind::Report, 0, false),
        text::agent::STEP_MARK_TRIANGLE_FILLED,
        "子 agent 汇报取消态也是实心上三角"
    );
}

/// 回归：spawn/wait 卡片里的 agent 链接必须压过整卡折叠面。
///
/// 快照按注册顺序取第一个命中的区域；整卡折叠面先登记时会永久吞掉链接行，
/// 子 agent 结束后最想打开详情的那一下反而只会把卡片收起。
#[test]
fn agent_link_wins_over_the_whole_card_fold_region() {
    use crate::app::state::{
        CardTarget, SubagentDetailView, SubagentReportView, ToolDetailView, ToolStatus,
    };
    use crate::chat::agent::WaitMode;
    use crate::ui::main_content::lx::history::CardKind;

    let report = SubagentReportView {
        agent_id: "sub-1".into(),
        title: "report".into(),
        role: crate::chat::agent::AgentRole::Explorer,
        status: "completed".into(),
        files_changed: 0,
        duration_ms: Some(1_000),
        body: "line one\nline two\nline three\nline four".into(),
        delivered_at_ms: 1,
    };
    let mut call = finished_command("ignored", "");
    call.name = "spawn_agent".into();
    call.status = ToolStatus::Completed;
    call.summary = None;
    call.detail = Some(Box::new(ToolDetailView::Subagent(SubagentDetailView {
        role: Some(crate::chat::agent::AgentRole::Explorer),
        title: Some("report".into()),
        task: Some("investigate".into()),
        agent_id: Some("sub-1".into()),
        agent_status: Some("completed".into()),
        targets: Vec::new(),
        all: false,
        mode: WaitMode::All,
        timeout_ms: None,
        timed_out: false,
        agents: Vec::new(),
        report: Some(report),
        closed_status: None,
    })));
    let lx = lx_with_command(call);
    let area = Rect::new(0, 0, 80, 40);
    let mut cache = PaneCache::default();
    let view = crate::ui::main_content::lx::history_view(area, &lx, &mut cache).expect("view");
    let width = super::content_rect(view.body).width;
    let (start, agent_id) = cache
        .snapshot(width, &lx)
        .cards
        .iter()
        .find_map(|card| match &card.kind {
            CardKind::AgentLink { agent_id } => Some((card.card_start, agent_id.clone())),
            _ => None,
        })
        .expect("spawn card has an agent link");
    let row = view
        .body
        .y
        .saturating_add(super::rows_to_u16(start.saturating_sub(view.scroll)));
    assert!(row < view.body.bottom(), "链接行必须在视口内");
    assert_eq!(
        crate::ui::main_content::lx::card_target_at(
            area,
            &lx,
            view.body.x.saturating_add(2),
            row,
            &mut cache,
        ),
        Some(CardTarget::OpenAgent { agent_id }),
        "点击链接行必须打开 Agents，而不是命中整卡折叠面"
    );
}

#[test]
fn report_more_hint_is_a_separate_click_target() {
    use crate::app::state::CardTarget;
    use crate::ui::main_content::lx::history::CardKind;

    let body = (0..60)
        .map(|index| format!("line {index}"))
        .collect::<Vec<_>>()
        .join("\n");
    let mut lx = lx_with_report(&body);
    assert!(lx.chat.toggle_detail("assistant-1", "report:sub-1"));
    let area = Rect::new(0, 0, 60, 80);
    let mut cache = PaneCache::default();
    let width = super::content_rect(area).width;
    let snapshot = cache.snapshot(width, &lx);
    let more = snapshot
        .cards
        .iter()
        .find(|card| matches!(&card.kind, CardKind::Report { more: true, .. }))
        .expect("expanded report has a more hint");
    let view = crate::ui::main_content::lx::history_view(area, &lx, &mut cache).expect("view");
    let row = view.body.y.saturating_add(super::rows_to_u16(
        more.card_start.saturating_sub(view.scroll),
    ));
    assert_eq!(
        crate::ui::main_content::lx::card_target_at(area, &lx, view.body.x + 2, row, &mut cache),
        Some(CardTarget::Report {
            message_id: "assistant-1".into(),
            agent_id: "sub-1".into(),
            more: true,
        })
    );

    // 提示行上方的正文仍命中整块（点击收起），不会误触「再展开一批」。
    let whole = snapshot
        .cards
        .iter()
        .find(|card| matches!(&card.kind, CardKind::Report { more: false, .. }))
        .expect("whole report click target");
    let whole_row = view.body.y.saturating_add(super::rows_to_u16(
        whole.card_start.saturating_sub(view.scroll),
    ));
    assert_eq!(
        crate::ui::main_content::lx::card_target_at(
            area,
            &lx,
            view.body.x + 2,
            whole_row,
            &mut cache,
        ),
        Some(CardTarget::Report {
            message_id: "assistant-1".into(),
            agent_id: "sub-1".into(),
            more: false,
        })
    );
}

#[test]
fn search_result_lines_are_part_of_copyable_history_text() {
    use crate::app::state::{
        SearchDetailView, SearchMatchView, ToolDetailView, ToolStatus, ToolSummary,
    };

    let mut call = finished_command("ignored", "");
    call.name = "search".into();
    call.status = ToolStatus::Completed;
    call.summary = Some(Box::new(ToolSummary {
        path: Some("src".into()),
        pattern: Some("render".into()),
        matches: Some(1),
        ..ToolSummary::default()
    }));
    call.detail = Some(Box::new(ToolDetailView::Search(SearchDetailView {
        matches: vec![SearchMatchView {
            path: "src/foo.rs".into(),
            line: 12,
            text: "pub fn render() {}".into(),
            line_truncated: false,
        }],
        truncated: false,
        files_scanned: 1,
    })));
    let lx = lx_with_command(call);
    let area = Rect::new(0, 0, 100, 20);
    let copied = crate::ui::main_content::lx::copy_text(
        area,
        &lx,
        ((0, 0), (19, 99)),
        &mut PaneCache::default(),
    );
    assert!(
        copied.contains("src/foo.rs:12: pub fn render() {}"),
        "{copied}"
    );
}

/// 回归：历史超过 65535 行后，卡片点击不能被 u16 饱和吞掉。
///
/// `card_at` 曾把**绝对内容行**交给 `rows_to_u16`：在 65535 处饱和后，换算出的屏幕
/// 矩形恒在视口上方，命中的裁剪回判直接返回 `None`——10 万行的会话滚到 70000 行，
/// 点任何 thinking 头部 / 工具卡片都没有反应。
#[test]
fn card_hit_survives_history_longer_than_u16() {
    use crate::app::state::CardTarget;
    // 70_000 行正文足够把后面的卡片推到 u16 饱和点之后。
    let body = (0..70_000)
        .map(|line| format!("第 {line} 行"))
        .collect::<Vec<_>>()
        .join("\n");
    let mut lx = lx_with(vec![
        message("user", "complete", vec![text_part(&body)]),
        message(
            "assistant",
            "complete",
            vec![reasoning_part("长历史里的思考块"), text_part("回答")],
        ),
    ]);
    lx.chat
        .expanded_user_messages
        .insert(lx.chat.messages[0].id.clone());
    let area = Rect::new(0, 0, 40, 20);
    let mut cache = PaneCache::default();
    // 先按跟随底部取一次视图，确定内容宽度；快照随后可复用。
    let view = crate::ui::main_content::lx::history_view(area, &lx, &mut cache).expect("历史区");
    let width = super::content_rect(view.body).width;
    let start = cache
        .snapshot(width, &lx)
        .cards
        .iter()
        .find(|card| matches!(card.kind, CardKind::Thinking { .. }))
        .map(|card| card.card_start)
        .expect("思考块必须有命中区");
    assert!(
        start > usize::from(u16::MAX),
        "卡片必须落在 u16 饱和点之后：{start}"
    );

    // 滚到卡片上方 5 行，再点它的首行。
    lx.chat.follow_bottom = false;
    lx.chat.history_scroll = start.saturating_sub(5);
    let view = crate::ui::main_content::lx::history_view(area, &lx, &mut cache).expect("历史区");
    let row = view
        .body
        .y
        .saturating_add(super::rows_to_u16(start.saturating_sub(view.scroll)));
    assert!(
        row < view.body.bottom(),
        "卡片首行必须在视口内：row={row} body={:?}",
        view.body
    );
    assert_eq!(
        crate::ui::main_content::lx::card_target_at(
            area,
            &lx,
            view.body.x.saturating_add(2),
            row,
            &mut cache,
        ),
        Some(CardTarget::Thinking {
            message_index: 1,
            step_index: 0,
        }),
        "超 65535 行的历史卡片必须可点"
    );
}

/// 构造一条带 `line_count` 行新增正文的 `write` 调用（每行 `text_chars` 个字符）。
fn write_call_with_lines(
    call_id: &str,
    line_count: usize,
    text_chars: usize,
) -> crate::app::state::ToolCallView {
    use crate::app::state::{
        DiffHunkView, DiffLineView, FileChangeView, FileDiffDetailView, FileDiffView, ToolCallView,
        ToolDetailView, ToolStatus, ToolSummary,
    };
    use crate::chat::tools::{DiffLineKind, FileChangeKind};
    ToolCallView {
        call_id: call_id.into(),
        name: "write".into(),
        position: 0,
        arguments_json: String::new(),
        status: ToolStatus::Completed,
        summary: Some(Box::new(ToolSummary {
            path: Some("src/lib.rs".into()),
            added: Some(line_count),
            removed: Some(0),
            counts_known: true,
            ..ToolSummary::default()
        })),
        error: None,
        result_json: None,
        detail: Some(Box::new(ToolDetailView::File(FileDiffDetailView {
            files: vec![FileChangeView {
                path: "src/lib.rs".into(),
                to: None,
                kind: FileChangeKind::Added,
                added: Some(line_count),
                removed: Some(0),
                counts_known: true,
                simplified: false,
                diff: Some(FileDiffView::Complete {
                    hunks: vec![DiffHunkView {
                        lines: (0..line_count)
                            .map(|index| DiffLineView {
                                kind: DiffLineKind::Added,
                                old_line: None,
                                new_line: Some(index + 1),
                                text: "x".repeat(text_chars),
                                no_final_newline: false,
                            })
                            .collect(),
                    }],
                    simplified: false,
                }),
            }],
            files_total: 1,
            added: Some(line_count),
            removed: Some(0),
            simplified: false,
            omitted_files: 0,
            single: true,
        }))),
    }
}

/// 回归：差异指纹必须覆盖每行文本宽度，否则折行后的行数会漂。
///
/// 旧指纹只写 hunk 数与每 hunk 行数。同一调用、同样 1 hunk 20 行，第二次把每行拉长
/// 到会折行时指纹不变 → 命中旧块 → 卡片仍按旧行数排版，后续块整体错位。
#[test]
fn file_diff_fingerprint_covers_text_width() {
    use crate::app::state::{FileDiffView, ToolDetailView};
    let mut lx = lx_with(vec![message(
        "assistant",
        "complete",
        vec![text_part("写文件")],
    )]);
    lx.chat.messages[0].steps.push(AssistantStepView::Tool {
        position: 1,
        call: write_call_with_lines("call-1", 20, 4),
    });
    // 展开后差异正文才参与排版（折叠态预览恒为最多 3 个物理行）。
    lx.chat.toggle_detail("assistant-1", "call-1");
    let mut cache = PaneCache::default();
    let width = 38;
    let short = cache.snapshot(width, &lx).total_rows;

    let Some(AssistantStepView::Tool { call, .. }) = lx.chat.messages[0].steps.get_mut(1) else {
        panic!("第二步应是工具调用");
    };
    let Some(ToolDetailView::File(view)) = call.detail.as_deref_mut() else {
        panic!("文件工具必须有差异详情");
    };
    let Some(FileDiffView::Complete { hunks, .. }) = view.files[0].diff.as_mut() else {
        panic!("差异必须完整");
    };
    for line in &mut hunks[0].lines {
        line.text = "x".repeat(200);
    }

    let long = cache.snapshot(width, &lx);
    assert_eq!(cache.builds(), 2, "文本宽度变化必须让指纹失效");
    assert!(
        long.total_rows > short,
        "折行后行数必须变多：{short} -> {}",
        long.total_rows
    );
}

/// 后台任务面板打开时，历史区被输出容器接管：几何类入口一律不回答。
///
/// 这条守卫不在每个入口各写一遍（曾经漏过）：`history_rows` / `history_center_target`
/// 若照旧按看不见的历史作答，滚轮与居中就会把用户滚到不存在的位置。
#[test]
fn geometry_entries_stay_silent_while_task_panel_is_open() {
    use crate::ui::main_content::lx::{history_rows, history_scrollbar, sticky_prompt_jump};
    let body = (0..40)
        .map(|line| format!("第 {line} 行"))
        .collect::<Vec<_>>()
        .join("\n");
    let mut lx = lx_with(vec![
        message("user", "complete", vec![text_part("问题")]),
        message("assistant", "complete", vec![text_part(&body)]),
    ]);
    let area = Rect::new(0, 0, 40, 12);
    let mut cache_state = cache();
    assert!(
        history_rows(area, &lx, &mut cache_state).is_some(),
        "面板未开时历史几何必须可用"
    );
    assert!(
        history_center_target(area, &lx, "assistant-1", &mut cache_state).is_some(),
        "面板未开时居中目标必须可算"
    );

    let mut page = crate::app::state::TaskPage::default();
    page.set_processes(Vec::new(), 0);
    lx.task = Some(page);
    assert_eq!(
        history_rows(area, &lx, &mut cache_state),
        None,
        "面板打开后历史没被绘制，不能回答可滚动行数"
    );
    assert_eq!(
        history_center_target(area, &lx, "assistant-1", &mut cache_state),
        None,
        "面板打开后不允许把视口滚到看不见的历史"
    );
    assert_eq!(sticky_prompt_jump(area, &lx, &mut cache_state), None);
    assert_eq!(history_scrollbar(area, &lx, &mut cache_state), None);
}
