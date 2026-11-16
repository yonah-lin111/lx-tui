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
    let output = (0..10)
        .map(|index| format!("output line {index}"))
        .collect::<Vec<_>>()
        .join("\n")
        + "\n";
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
    if let Some(blank) = (0..output_card.card_rows)
        .find(|offset| text_span(&output_card.card_cols, *offset).is_none())
    {
        assert_eq!(
            hit_at(area, &lx, &mut cache, output_card.card_start + blank, 0),
            None,
            "tree-only or blank output row must not be clickable"
        );
    }
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
