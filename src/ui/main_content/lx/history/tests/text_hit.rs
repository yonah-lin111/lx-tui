//! 折叠命中收窄：只有点击物理行的文字列才触发，行尾空白不触发。

use super::*;
use crate::app::state::{
    CardSection, CardTarget, ChatMessageView, CommandView, CompactNoticeView, ProcessStateView,
    SubagentReportView, ToolCallView, ToolDetailView, ToolStatus,
};
use crate::ui::main_content::lx::history::{CardCols, CardKind};

fn text_span(cols: &CardCols, offset: usize) -> Option<(u16, u16)> {
    match cols {
        CardCols::Text(rows) => rows.get(offset).copied().flatten(),
        CardCols::Full | CardCols::Chip { .. } => None,
    }
}

fn history_width(area: Rect, lx: &Lx, cache: &mut PaneCache) -> u16 {
    let view = crate::ui::main_content::lx::history_view(area, lx, cache).expect("history view");
    super::content_rect(view.body).width
}

/// 按内容行 + 内容列点击（列相对内容区左缘）。
fn hit_at(
    area: Rect,
    lx: &Lx,
    cache: &mut PaneCache,
    content_row: usize,
    rel_col: u16,
) -> Option<CardTarget> {
    let view = crate::ui::main_content::lx::history_view(area, lx, cache).expect("history view");
    let row = view
        .body
        .y
        .saturating_add(super::rows_to_u16(content_row.saturating_sub(view.scroll)));
    assert!(row < view.body.bottom(), "content row must be visible");
    crate::ui::main_content::lx::card_target_at(
        area,
        lx,
        view.body.x.saturating_add(rel_col),
        row,
        cache,
    )
}

fn finished_command(command: &str, output: &str) -> ToolCallView {
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
            duration_ms: Some(100),
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

fn lx_with_command(call: ToolCallView) -> Lx {
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

fn lx_with_report(body: &str) -> Lx {
    let mut message = message("assistant", "complete", Vec::new());
    message.reports.push(SubagentReportView {
        agent_id: "sub-1".into(),
        nickname: None,
        title: "report".into(),
        role: crate::chat::agent::AgentRole::Explorer,
        status: "completed".into(),
        files_changed: 1,
        changed_paths: Vec::new(),
        duration_ms: Some(1_000),
        body: body.into(),
        delivered_at_ms: 1,
    });
    lx_with(vec![message])
}

#[test]
fn user_hint_text_toggles_but_row_tail_does_not() {
    let lx = lx_with(vec![message(
        "user",
        "complete",
        vec![text_part("1\n2\n3\n4\n5\n6")],
    )]);
    let area = Rect::new(0, 0, 80, 20);
    let mut cache = cache();
    let width = history_width(area, &lx, &mut cache);
    let snapshot = cache.snapshot(width, &lx);
    let card = snapshot
        .cards
        .iter()
        .find(|card| matches!(card.kind, CardKind::UserMessage { .. }))
        .expect("folded user hint");
    let (start, end) = text_span(&card.card_cols, 0).expect("hint has text columns");
    let tail = width.saturating_sub(1);
    assert!(
        end < tail,
        "hint must not span the full row: {end} < {tail}"
    );

    assert_eq!(
        hit_at(area, &lx, &mut cache, card.card_start, start),
        Some(CardTarget::UserMessage {
            message_id: "user-1".into(),
        })
    );
    assert_eq!(
        hit_at(area, &lx, &mut cache, card.card_start, tail),
        None,
        "row tail must not toggle the user message"
    );
    assert_eq!(
        hit_at(area, &lx, &mut cache, card.card_start, 0),
        None,
        "left indent must not toggle the user message"
    );
}

#[test]
fn thinking_header_and_body_hit_text_only() {
    let mut lx = lx_with(vec![message(
        "assistant",
        "complete",
        vec![reasoning_part("alpha\nbeta\ngamma")],
    )]);
    lx.chat.toggle_reasoning(0, 0);
    let area = Rect::new(0, 0, 80, 24);
    let mut cache = cache();
    let width = history_width(area, &lx, &mut cache);
    let snapshot = cache.snapshot(width, &lx);
    let card = snapshot
        .cards
        .iter()
        .find(|card| matches!(card.kind, CardKind::Thinking { .. }))
        .expect("thinking card");
    assert!(card.card_rows > 1, "expanded thinking has body rows");
    let tail = width.saturating_sub(1);
    let (header_start, _) = text_span(&card.card_cols, 0).expect("header columns");

    assert_eq!(
        hit_at(area, &lx, &mut cache, card.card_start, header_start),
        Some(CardTarget::Thinking {
            message_index: 0,
            step_index: 0,
        })
    );
    assert_eq!(
        hit_at(area, &lx, &mut cache, card.card_start, tail),
        None,
        "header tail must not toggle thinking"
    );

    let (body_start, _) = text_span(&card.card_cols, 1).expect("body columns");
    assert_eq!(
        hit_at(area, &lx, &mut cache, card.card_start + 1, body_start),
        Some(CardTarget::Thinking {
            message_index: 0,
            step_index: 0,
        })
    );
    assert_eq!(
        hit_at(area, &lx, &mut cache, card.card_start + 1, tail),
        None,
        "body tail must not toggle thinking"
    );
}

#[test]
fn command_params_and_output_hit_text_only() {
    // 参数区要超过折叠预览才会登记点击面。
    let command = format!("$ ls -la {}", "/a/very/long/path".repeat(12));
    let output = format!(
        "│\n{}",
        (0..10)
            .map(|index| format!("output line {index}"))
            .collect::<Vec<_>>()
            .join("\n")
    );
    let lx = lx_with_command(finished_command(&command, &output));
    let area = Rect::new(0, 0, 80, 40);
    let mut cache = cache();
    let width = history_width(area, &lx, &mut cache);
    let snapshot = cache.snapshot(width, &lx);
    let tail = width.saturating_sub(1);

    for section in [CardSection::Body, CardSection::Output] {
        let card = snapshot
            .cards
            .iter()
            .find(
                |card| matches!(&card.kind, CardKind::Tool { section: hit, .. } if *hit == section),
            )
            .expect("command card section");
        let text_rows: Vec<(usize, (u16, u16))> = (0..card.card_rows)
            .filter_map(|offset| text_span(&card.card_cols, offset).map(|cols| (offset, cols)))
            .collect();
        let (start, end) = *text_rows.first().expect("section has text rows");
        assert_eq!(
            hit_at(area, &lx, &mut cache, card.card_start + start, end.0),
            Some(CardTarget::Tool {
                message_id: "assistant-1".into(),
                call_id: "call-1".into(),
                section,
            })
        );
        let (tail_row, _) = text_rows
            .iter()
            .find(|(_, (_, end))| *end < tail)
            .expect("a text row shorter than the pane");
        assert_eq!(
            hit_at(area, &lx, &mut cache, card.card_start + tail_row, tail),
            None,
            "row tail must not toggle {section:?}"
        );
    }

    let output_card = snapshot
        .cards
        .iter()
        .find(|card| {
            matches!(
                &card.kind,
                CardKind::Tool {
                    section: CardSection::Output,
                    ..
                }
            )
        })
        .expect("output card");
    let blank = (0..output_card.card_rows)
        .find(|offset| text_span(&output_card.card_cols, *offset).is_none())
        .expect("fixture must contain a tree-only output row");
    assert_eq!(
        hit_at(area, &lx, &mut cache, output_card.card_start + blank, 0),
        None,
        "tree-only or blank output row must not be clickable"
    );
}

#[test]
fn report_hint_and_body_hit_text_only() {
    let body = (0..60)
        .map(|index| format!("line {index}"))
        .collect::<Vec<_>>()
        .join("\n");
    let mut lx = lx_with_report(&body);
    assert!(lx.chat.toggle_detail("assistant-1", "report:sub-1"));
    let area = Rect::new(0, 0, 60, 80);
    let mut cache = cache();
    let width = history_width(area, &lx, &mut cache);
    let snapshot = cache.snapshot(width, &lx);
    let tail = width.saturating_sub(1);

    let hint = snapshot
        .cards
        .iter()
        .find(|card| matches!(&card.kind, CardKind::Report { more: true, .. }))
        .expect("report more hint");
    let (start, _) = text_span(&hint.card_cols, 0).expect("hint columns");
    assert_eq!(
        hit_at(area, &lx, &mut cache, hint.card_start, start),
        Some(CardTarget::Report {
            message_id: "assistant-1".into(),
            agent_id: "sub-1".into(),
            more: true,
        })
    );
    assert_eq!(
        hit_at(area, &lx, &mut cache, hint.card_start, tail),
        None,
        "hint tail must not append another batch"
    );

    let whole = snapshot
        .cards
        .iter()
        .find(|card| matches!(&card.kind, CardKind::Report { more: false, .. }))
        .expect("report whole card");
    let (start, _) = text_span(&whole.card_cols, 0).expect("header columns");
    assert_eq!(
        hit_at(area, &lx, &mut cache, whole.card_start, start),
        Some(CardTarget::Report {
            message_id: "assistant-1".into(),
            agent_id: "sub-1".into(),
            more: false,
        })
    );
    assert_eq!(
        hit_at(area, &lx, &mut cache, whole.card_start, tail),
        None,
        "report header tail must not fold"
    );
}

#[test]
fn compact_body_hits_text_only() {
    let mut lx = Lx::new();
    lx.compact.replace(vec![CompactNoticeView {
        id: "compact-1".into(),
        sequence: 1,
        model_label: "gpt-5".into(),
        created_at_ms: 1_700_000_640_000,
        start_sequence: 0,
        end_sequence: 0,
        summary: (0..8)
            .map(|index| format!("summary line {index}"))
            .collect::<Vec<_>>()
            .join("\n"),
        active: true,
        auto: false,
        report: false,
    }]);
    let area = Rect::new(0, 0, 60, 20);
    let mut cache = cache();
    let width = history_width(area, &lx, &mut cache);
    let snapshot = cache.snapshot(width, &lx);
    let card = snapshot
        .cards
        .iter()
        .find(|card| matches!(card.kind, CardKind::Compact { .. }))
        .expect("foldable compact body");
    let (start, _) = text_span(&card.card_cols, 0).expect("summary columns");
    let tail = width.saturating_sub(1);
    assert_eq!(
        hit_at(area, &lx, &mut cache, card.card_start, start),
        Some(CardTarget::Compact {
            checkpoint_id: "compact-1".into(),
        })
    );
    assert_eq!(
        hit_at(area, &lx, &mut cache, card.card_start, tail),
        None,
        "compact tail must not fold"
    );
}

#[test]
fn running_command_card_keeps_whole_row_hit() {
    let mut call = finished_command("$ sleep 100", "");
    if let Some(ToolDetailView::Command(view)) = call.detail.as_deref_mut() {
        view.state = ProcessStateView::Running;
        view.duration_ms = None;
    }
    let lx = lx_with_command(call);
    let area = Rect::new(0, 0, 80, 20);
    let mut cache = cache();
    let width = history_width(area, &lx, &mut cache);
    let snapshot = cache.snapshot(width, &lx);
    let card = snapshot
        .cards
        .iter()
        .find(|card| matches!(card.kind, CardKind::Tool { .. }))
        .expect("running command card");
    assert_eq!(card.card_cols, CardCols::Full);
    assert_eq!(
        hit_at(
            area,
            &lx,
            &mut cache,
            card.card_start,
            width.saturating_sub(1)
        ),
        Some(CardTarget::Tool {
            message_id: "assistant-1".into(),
            call_id: "call-1".into(),
            section: CardSection::Body,
        }),
        "running command card opens the task panel from anywhere"
    );
}

#[test]
fn user_paste_chip_keeps_existing_chip_hit() {
    let token = crate::app::paste::encode_pasted_kind(crate::app::paste::FoldedKind::Text, "one");
    let msg = message(
        "user",
        "complete",
        vec![text_part(&format!("请看：{token}"))],
    );
    let lx = lx_with(vec![msg]);
    let area = Rect::new(0, 0, 60, 10);
    let mut cache = cache();
    let width = history_width(area, &lx, &mut cache);
    let snapshot = cache.snapshot(width, &lx);
    let card = snapshot
        .cards
        .iter()
        .find(|card| matches!(card.kind, CardKind::UserPaste { .. }))
        .expect("paste chip");
    let CardCols::Chip { first, last } = card.card_cols else {
        panic!("paste chip must keep chip columns");
    };
    assert_eq!(
        hit_at(area, &lx, &mut cache, card.card_start, first),
        Some(CardTarget::UserPaste {
            message_id: "user-1".into(),
            paste_id: match card.kind {
                CardKind::UserPaste { paste_id, .. } => paste_id,
                _ => unreachable!(),
            },
            occurrence: 0,
        })
    );
    assert_eq!(
        hit_at(
            area,
            &lx,
            &mut cache,
            card.card_start,
            last.saturating_add(1)
        ),
        None,
        "chip tail is not part of the chip"
    );
}

#[test]
fn agents_history_shares_the_same_text_hit() {
    let record = crate::db::chat::SubagentRecord {
        agent_id: "a1".into(),
        nickname: None,
        agent_path: "/root/agent".into(),
        child_session_id: "s1".into(),
        parent_session_id: "p".into(),
        spawn_message_id: None,
        role: crate::chat::agent::AgentRole::Explorer,
        model_provider: String::new(),
        model_id: String::new(),
        model_variant: None,
        status: "running".into(),
        title: "agent detail".into(),
        created_at_ms: 0,
        completed_at_ms: None,
        report_json: None,
        report_delivered: false,
    };
    let mut run = crate::app::state::SubagentRun::from_record(&record);
    run.loaded = true;
    run.chat.messages.push(ChatMessageView {
        sequence: 0,
        id: "m-1".into(),
        role: "assistant".into(),
        status: "complete".into(),
        turn_index: 0,
        kind: "chat".into(),
        shell: None,
        steps: vec![
            AssistantStepView::Part {
                position: 0,
                part: reasoning_part("thinking about it"),
            },
            AssistantStepView::Part {
                position: 1,
                part: text_part("answer"),
            },
        ],
        model_label: None,
        error_code: None,
        input_tokens: None,
        output_tokens: None,
        cache_read_tokens: None,
        duration_ms: None,
        reports: Vec::new(),
        attachments: Vec::new(),
    });
    let mut lx = Lx::new();
    lx.task = Some(crate::app::state::TaskPage {
        tab: crate::app::state::TaskTab::Agents,
        ..Default::default()
    });
    lx.agents.runs.push(run);

    let area = Rect::new(0, 0, 80, 24);
    let output =
        crate::ui::main_content::lx::task::task_output_rect(area, &lx).expect("Agents output rect");
    let mut cache = PaneCache::default();
    let mut header_hit = None;
    for row in output.y..output.bottom() {
        if let Some(target) = crate::ui::main_content::lx::history_hit::task_agents_card_target_at(
            area,
            &lx,
            output.x + 4,
            row,
            &mut cache,
        ) {
            header_hit = Some((row, target));
            break;
        }
    }
    let (row, target) = header_hit.expect("Agents thinking header hit");
    assert!(matches!(target, CardTarget::Thinking { .. }));
    assert_eq!(
        crate::ui::main_content::lx::history_hit::task_agents_card_target_at(
            area,
            &lx,
            output.right().saturating_sub(1),
            row,
            &mut cache,
        ),
        None,
        "Agents row tail must not toggle folding"
    );
}

#[test]
fn btw_history_shares_the_same_text_hit() {
    let mut lx = Lx::new();
    lx.task = Some(crate::app::state::TaskPage {
        tab: crate::app::state::TaskTab::Btw,
        ..Default::default()
    });
    lx.btw.ensure_thread("qa-1");
    let mut message = ChatMessageView {
        sequence: 0,
        id: "m-1".into(),
        role: "assistant".into(),
        status: "complete".into(),
        turn_index: 0,
        kind: "chat".into(),
        shell: None,
        steps: vec![AssistantStepView::Part {
            position: 0,
            part: reasoning_part("thinking about it"),
        }],
        model_label: None,
        error_code: None,
        input_tokens: None,
        output_tokens: None,
        cache_read_tokens: None,
        duration_ms: None,
        reports: Vec::new(),
        attachments: Vec::new(),
    };
    message.steps.push(AssistantStepView::Part {
        position: 1,
        part: text_part("answer"),
    });
    let thread = lx.btw.active_thread_mut().expect("thread");
    thread.loaded = true;
    thread.chat.messages.push(message);
    thread.rows_dirty = true;

    let area = Rect::new(0, 0, 80, 24);
    let output =
        crate::ui::main_content::lx::task::task_output_rect(area, &lx).expect("BTW output rect");
    let mut cache = PaneCache::default();
    let _ = crate::ui::main_content::lx::task::btw_rows(area, &lx, &mut cache);
    let mut header_hit = None;
    for row in output.y..output.bottom() {
        if let Some(target) = crate::ui::main_content::lx::history_hit::task_btw_card_target_at(
            area,
            &lx,
            output.x + 4,
            row,
            &mut cache,
        ) {
            header_hit = Some((row, target));
            break;
        }
    }
    let (row, target) = header_hit.expect("BTW thinking header hit");
    assert!(matches!(target, CardTarget::Thinking { .. }));
    assert_eq!(
        crate::ui::main_content::lx::history_hit::task_btw_card_target_at(
            area,
            &lx,
            output.right().saturating_sub(1),
            row,
            &mut cache,
        ),
        None,
        "BTW row tail must not toggle folding"
    );
}

fn grouped_commands() -> Lx {
    let output = (0..12)
        .map(|index| format!("grouped output line {index:02} ends-here"))
        .collect::<Vec<_>>()
        .join("\n");
    let mut first = finished_command("$ first", &output);
    first.call_id = "call-1".into();
    let mut second = finished_command("$ second", &output);
    second.call_id = "call-2".into();
    let mut lx = lx_with_command(first);
    lx.chat.messages[0].steps.push(AssistantStepView::Tool {
        position: 2,
        call: second,
    });
    lx
}

fn row_text_extent(buf: &Buffer, x0: u16, y: u16, x1: u16) -> Option<(u16, u16)> {
    let mut first = None;
    let mut last = 0u16;
    for x in x0..x1 {
        let cell = buf.cell((x, y))?;
        let symbol = cell.symbol();
        if symbol == " " || symbol == "\0" {
            continue;
        }
        first.get_or_insert(x);
        last = x + 1;
    }
    first.map(|first| (first, last))
}

#[test]
fn merged_group_rows_match_rendered_text_columns() {
    let lx = grouped_commands();
    let area = Rect::new(0, 0, 80, 60);
    let mut cache = cache();
    let width = history_width(area, &lx, &mut cache);
    let view = crate::ui::main_content::lx::history_view(area, &lx, &mut cache).expect("view");
    let snapshot = cache.snapshot(width, &lx);
    let cards: Vec<_> = snapshot
        .cards
        .iter()
        .filter(|card| matches!(card.kind, CardKind::Tool { .. }))
        .collect();
    assert_eq!(cards.len(), 4, "two calls x params/output regions");

    let mut buf = Buffer::empty(view.area);
    crate::ui::main_content::lx::render_history_for_test(view.area, &mut buf, &lx, view.scroll);
    for card in cards {
        for offset in 0..card.card_rows {
            let Some((first, last)) = text_span(&card.card_cols, offset) else {
                continue;
            };
            let y = view
                .body
                .y
                .saturating_add(super::rows_to_u16(card.card_start + offset - view.scroll));
            let actual = row_text_extent(&buf, view.body.x, y, view.body.right())
                .unwrap_or_else(|| panic!("rendered row must have text: row {y}"));
            let actual = (actual.0 - view.body.x, actual.1 - view.body.x);
            assert_eq!(
                (first, last),
                actual,
                "card columns must match rendered row {y}"
            );
        }
    }
}

#[test]
fn web_search_title_targets_the_only_expandable_block() {
    let mut call = finished_command("ignored", "");
    call.name = "web_search".into();
    call.detail = Some(Box::new(ToolDetailView::WebSearch(
        crate::app::state::WebSearchDetailView {
            queries: vec!["single query".into()],
            search_type: "fast".into(),
            num_results: 2,
            provider: Some("exa".into()),
            result: (0..10).map(|index| format!("result {index}")).collect(),
            result_more: 0,
            duration_ms: Some(120),
            truncated: false,
        },
    )));
    let lx = lx_with_command(call);
    let area = Rect::new(0, 0, 80, 30);
    let mut cache = cache();
    let width = history_width(area, &lx, &mut cache);
    let snapshot = cache.snapshot(width, &lx);
    let title = snapshot
        .cards
        .iter()
        .find(|card| {
            card.card_rows == 1
                && matches!(
                    card.kind,
                    CardKind::Tool {
                        section: CardSection::Output,
                        ..
                    }
                )
        })
        .expect("title row must bind to the expandable result block");
    let (start, _) = text_span(&title.card_cols, 0).expect("title text columns");
    assert_eq!(
        hit_at(area, &lx, &mut cache, title.card_start, start),
        Some(CardTarget::Tool {
            message_id: "assistant-1".into(),
            call_id: "call-1".into(),
            section: CardSection::Output,
        })
    );
}

#[test]
fn expanded_all_blank_output_still_can_collapse() {
    let lx = lx_with_command(finished_command("$ blank", "│\n│\n│\n│\n│\n│"));
    let mut lx = lx;
    assert!(
        lx.chat
            .toggle_section("assistant-1", "call-1", CardSection::Output)
    );
    let area = Rect::new(0, 0, 80, 20);
    let mut cache = cache();
    let width = history_width(area, &lx, &mut cache);
    let snapshot = cache.snapshot(width, &lx);
    let card = snapshot
        .cards
        .iter()
        .find(|card| {
            matches!(
                card.kind,
                CardKind::Tool {
                    section: CardSection::Output,
                    ..
                }
            )
        })
        .expect("expanded output region");
    assert!(card.card_cols.text_is_blank(), "blank output has no text");
    assert_eq!(
        hit_at(area, &lx, &mut cache, card.card_start, width - 1),
        Some(CardTarget::Tool {
            message_id: "assistant-1".into(),
            call_id: "call-1".into(),
            section: CardSection::Output,
        }),
        "a blank expanded block must still be collapsible by mouse"
    );
}

#[test]
fn folded_thinking_card_is_header_only() {
    let lx = lx_with(vec![message(
        "assistant",
        "complete",
        vec![reasoning_part("alpha\nbeta")],
    )]);
    let area = Rect::new(0, 0, 80, 20);
    let mut cache = cache();
    let width = history_width(area, &lx, &mut cache);
    let snapshot = cache.snapshot(width, &lx);
    let card = snapshot
        .cards
        .iter()
        .find(|card| matches!(card.kind, CardKind::Thinking { .. }))
        .expect("folded thinking card");
    assert_eq!(card.card_rows, 1);
    assert!(matches!(
        &card.card_cols,
        CardCols::Text(cols) if cols.len() == 1 && cols[0].is_some()
    ));
    assert_eq!(
        hit_at(area, &lx, &mut cache, card.card_start, 0),
        Some(CardTarget::Thinking {
            message_index: 0,
            step_index: 0,
        })
    );
}

fn shell_message(command: &str, output: &str) -> ChatMessageView {
    let mut message = message("user", "complete", Vec::new());
    message.kind = "user_shell".into();
    message.shell = Some(crate::app::state::UserShellView {
        command: command.into(),
        cwd: "/workspace/lx".into(),
        shell: "/bin/zsh".into(),
        state: ProcessStateView::Exited { code: Some(0) },
        exit_code: None,
        duration_ms: Some(100),
        output: output.into(),
        output_start: 0,
        output_end: u64::try_from(output.len()).unwrap_or(0),
        dropped_bytes: 0,
        truncated: false,
        runtime_epoch: None,
        process_id: None,
        revision: 1,
    });
    message
}

#[test]
fn shell_params_and_output_hit_text_only() {
    let command = format!("echo {}", "x".repeat(200));
    let output = (0..10)
        .map(|index| format!("shell line {index}"))
        .collect::<Vec<_>>()
        .join("\n");
    let lx = lx_with(vec![shell_message(&command, &output)]);
    let area = Rect::new(0, 0, 80, 40);
    let mut cache = cache();
    let width = history_width(area, &lx, &mut cache);
    let snapshot = cache.snapshot(width, &lx);
    let tail = width.saturating_sub(1);
    for section in [CardSection::Body, CardSection::Output] {
        let card = snapshot
            .cards
            .iter()
            .find(|card| {
                matches!(&card.kind, CardKind::Shell { section: hit, .. } if *hit == section)
            })
            .expect("shell card section");
        let text_rows: Vec<(usize, (u16, u16))> = (0..card.card_rows)
            .filter_map(|offset| text_span(&card.card_cols, offset).map(|cols| (offset, cols)))
            .collect();
        let (offset, (start, _)) = *text_rows.first().expect("section has text rows");
        let (tail_offset, _) = text_rows
            .iter()
            .find(|(_, (_, end))| *end < tail)
            .expect("a text row shorter than the pane");
        assert_eq!(
            hit_at(area, &lx, &mut cache, card.card_start + offset, start),
            Some(CardTarget::Shell {
                message_id: "user-1".into(),
                section,
            })
        );
        assert_eq!(
            hit_at(area, &lx, &mut cache, card.card_start + tail_offset, tail),
            None,
            "row tail must not toggle {section:?}"
        );
    }
}
