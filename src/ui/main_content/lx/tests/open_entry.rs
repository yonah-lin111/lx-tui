//! 工具卡片命中回归测试：折叠/展开的唯一点击面是**卡片内容**。
//!
//! 用户要求移除动作按钮与 hover 高亮：点折叠内容展开、点展开内容收起，标题行不是
//! 点击面（避免误触），拖拽选中不折叠。进程卡片不再有 `open` 行，后台任务面板改走
//! `^o` / 空输入框按 `↓`。

use ratatui::layout::Rect;

use super::*;
use crate::app::state::{
    AssistantStepView, ChatMessageView, CommandView, Lx, ProcessStateView, ToolCallView, ToolStatus,
};

/// 构造一条 assistant 消息，内含一张进程卡片。
fn command_message(call_id: &str, session_id: u64) -> ChatMessageView {
    let call = ToolCallView {
        call_id: call_id.into(),
        name: "exec_command".into(),
        position: 0,
        arguments_json: r#"{"cmd":"serve"}"#.into(),
        status: ToolStatus::Completed,
        summary: None,
        error: None,
        result_json: None,
        detail: Some(Box::new(crate::app::state::ToolDetailView::Command(
            CommandView {
                process_session_id: Some(session_id),
                command: "while true; do echo heartbeat; done".into(),
                cwd: "/workspace/lx".into(),
                tty: false,
                state: ProcessStateView::Running,
                output_preview: Vec::new(),
                truncated: false,
                dropped_bytes: 0,
                output_tail: "heartbeat\n".into(),
                started_at_ms: Some(0),
                ended_at_ms: None,
            },
        ))),
    };
    ChatMessageView {
        sequence: 0,
        id: format!("msg-{call_id}"),
        role: "assistant".into(),
        status: "complete".into(),
        turn_index: 0,
        kind: "chat".into(),
        shell: None,
        steps: vec![AssistantStepView::Tool { position: 0, call }],
        model_label: None,
        error_code: None,
        input_tokens: None,
        output_tokens: None,
        cache_read_tokens: None,
        duration_ms: None,
        reports: Vec::new(),
        attachments: Vec::new(),
    }
}

fn lx_with_command(call_id: &str, session_id: u64) -> Lx {
    let mut lx = Lx::new();
    lx.chat.messages = vec![command_message(call_id, session_id)];
    lx.chat.follow_bottom = true;
    lx
}

/// 构造一条含文件差异卡片的 assistant 消息。
fn file_message(call_id: &str) -> ChatMessageView {
    use crate::app::state::{
        DiffHunkView, DiffLineView, FileChangeView, FileDiffDetailView, FileDiffView,
    };
    use crate::chat::tools::{DiffLineKind, FileChangeKind};

    let diff = FileDiffDetailView {
        files: vec![FileChangeView {
            path: "/w/a.rs".into(),
            to: None,
            kind: FileChangeKind::Updated,
            added: Some(1),
            removed: Some(1),
            counts_known: true,
            simplified: false,
            diff: Some(FileDiffView::Complete {
                hunks: vec![DiffHunkView {
                    // 5 条变更行 > 3 行预览额度：折叠态必然有省略，因此一定有
                    // 可点的「剩余行数」动作行。
                    lines: (1..=5)
                        .flat_map(|number| {
                            [
                                DiffLineView {
                                    kind: DiffLineKind::Removed,
                                    old_line: Some(number),
                                    new_line: None,
                                    text: format!("old {number}"),
                                    no_final_newline: false,
                                },
                                DiffLineView {
                                    kind: DiffLineKind::Added,
                                    old_line: None,
                                    new_line: Some(number),
                                    text: format!("new {number}"),
                                    no_final_newline: false,
                                },
                            ]
                        })
                        .collect(),
                }],
                simplified: false,
            }),
        }],
        files_total: 1,
        added: Some(1),
        removed: Some(1),
        simplified: false,
        omitted_files: 0,
        single: false,
    };
    let call = ToolCallView {
        call_id: call_id.into(),
        name: "edit".into(),
        position: 0,
        arguments_json: r#"{"patch":"*** Begin Patch"}"#.into(),
        status: ToolStatus::Completed,
        summary: None,
        error: None,
        result_json: None,
        detail: Some(Box::new(crate::app::state::ToolDetailView::File(diff))),
    };
    ChatMessageView {
        sequence: 0,
        id: format!("msg-{call_id}"),
        role: "assistant".into(),
        status: "complete".into(),
        turn_index: 0,
        kind: "chat".into(),
        shell: None,
        steps: vec![AssistantStepView::Tool { position: 0, call }],
        model_label: None,
        error_code: None,
        input_tokens: None,
        output_tokens: None,
        cache_read_tokens: None,
        duration_ms: None,
        reports: Vec::new(),
        attachments: Vec::new(),
    }
}

#[test]
fn card_content_is_the_toggle_target() {
    // 没有动作按钮、没有剩余行数：整块内容就是点击面。
    let area = Rect::new(0, 0, 60, 20);
    let mut lx = Lx::new();
    lx.chat.messages = vec![file_message("call-edit")];
    lx.chat.follow_bottom = true;
    let view = history_view(area, &lx, &mut PaneCache::default()).expect("历史区");
    let cards =
        crate::ui::main_content::lx::history::layout(view.body, &lx, &mut PaneCache::default())
            .cards;
    assert_eq!(cards.len(), 1, "一张卡片");
    let card = &cards[0];
    // 整块可点：折叠态点预览行就能展开。
    let top_y = card
        .card
        .y
        .saturating_sub(view.body.y)
        .saturating_sub(u16::try_from(view.scroll).unwrap_or(0))
        .saturating_add(view.body.y);
    for row in [top_y, top_y + 1, top_y + 2] {
        assert_eq!(
            card_target_at(area, &lx, area.x + 6, row, &mut PaneCache::default()),
            Some(crate::app::state::CardTarget::Tool {
                message_id: "msg-call-edit".into(),
                call_id: "call-edit".into(),
                section: crate::app::state::CardSection::Body,
            }),
            "行 {row} 属于卡片，点它切换折叠"
        );
    }
}

#[test]
fn expanded_card_body_is_the_click_to_collapse_target() {
    // 展开后：整块内容仍是 `CardHit.body`，点它（未拖拽）收起。
    let area = Rect::new(0, 0, 60, 20);
    let mut lx = Lx::new();
    lx.chat.messages = vec![file_message("call-edit")];
    lx.chat.follow_bottom = true;
    lx.chat.toggle_detail("msg-call-edit", "call-edit");
    let view = history_view(area, &lx, &mut PaneCache::default()).expect("历史区");
    let cards =
        crate::ui::main_content::lx::history::layout(view.body, &lx, &mut PaneCache::default())
            .cards;
    let card = cards.first().expect("文件卡片整块命中");
    assert!(
        matches!(
            &card.kind,
            crate::ui::main_content::lx::history::CardKind::Tool { expanded: true, .. }
        ),
        "布局记录的展开状态必须与状态层一致"
    );
    // 展开态整块仍是点击面：点内容即收起。
    let top_y = card
        .card
        .y
        .saturating_sub(view.body.y)
        .saturating_sub(u16::try_from(view.scroll).unwrap_or(0))
        .saturating_add(view.body.y);
    assert_eq!(
        card_target_at(area, &lx, area.x + 6, top_y + 2, &mut PaneCache::default()),
        Some(crate::app::state::CardTarget::Tool {
            message_id: "msg-call-edit".into(),
            call_id: "call-edit".into(),
            section: crate::app::state::CardSection::Body,
        }),
        "展开态点内容收起"
    );
}

#[test]
fn file_tool_without_diff_has_no_toggle_hit() {
    // 旧记录（无 diff）保持两行摘要，不制造无效展开入口。
    let mut lx = Lx::new();
    lx.chat.messages = vec![file_message("call-edit")];
    if let AssistantStepView::Tool { call, .. } = &mut lx.chat.messages[0].steps[0] {
        call.detail = Some(Box::new(crate::app::state::ToolDetailView::File(
            crate::app::state::FileDiffDetailView::default(),
        )));
    }
    let area = Rect::new(0, 0, 60, 20);
    let view = history_view(area, &lx, &mut PaneCache::default()).expect("历史区");
    assert!(
        crate::ui::main_content::lx::history::layout(view.body, &lx, &mut PaneCache::default(),)
            .cards
            .is_empty(),
        "无差异正文、展开也不多行时不登记点击面"
    );
}

#[test]
fn running_process_card_is_clickable_without_an_open_row() {
    // 进程仍在跑：整卡可点（打开后台任务面板），但卡片里没有 `open` 入口行，
    // 只有末行的引导文字。
    let area = Rect::new(0, 0, 60, 20);
    let lx = lx_with_command("call-1", 42);
    let view = history_view(area, &lx, &mut PaneCache::default()).expect("历史区");
    let cards =
        crate::ui::main_content::lx::history::layout(view.body, &lx, &mut PaneCache::default())
            .cards;
    // 参数区与引导行各自可点，行为都是打开后台任务面板。
    assert_eq!(cards.len(), 2, "运行中的卡片必须可点");
    let mut buf = Buffer::empty(area);
    crate::ui::main_content::lx::render_history_for_test(area, &mut buf, &lx, 0);
    let text = super::buffer_text(&buf);
    assert!(
        !text.lines().any(|line| line.trim_end().ends_with(" open")),
        "没有 open 行：{text}"
    );
    assert!(
        text.contains(text::agent::TASK_OPEN_GUIDE),
        "改为整卡引导：{text}"
    );
}

#[test]
fn long_process_command_wraps_and_is_clickable() {
    // 长命令折行后确实有更多内容：内容区可点，点它切换折叠。
    let area = Rect::new(0, 0, 60, 20);
    let mut lx = lx_with_command("call-1", 42);
    if let AssistantStepView::Tool { call, .. } = &mut lx.chat.messages[0].steps[0]
        && let Some(crate::app::state::ToolDetailView::Command(view)) = call.detail.as_deref_mut()
    {
        // 足够长：折叠 2 行装不下，展开才铺得开。
        view.command = r#"$ { echo "==== 项目概览 ====" && echo "Rust 版本: $(rustc --version)" && echo "文件数: $(find . -type f | wc -l)" && echo "目录结构" && ls -R && echo "依赖" && cat Cargo.toml && echo "完成" }"#
            .into();
    }
    let view = history_view(area, &lx, &mut PaneCache::default()).expect("历史区");
    let cards =
        crate::ui::main_content::lx::history::layout(view.body, &lx, &mut PaneCache::default())
            .cards;
    let card = cards.first().expect("折行命令可展开，应有卡片");
    let top_y = card
        .card
        .y
        .saturating_sub(view.body.y)
        .saturating_sub(u16::try_from(view.scroll).unwrap_or(0))
        .saturating_add(view.body.y);
    assert_eq!(
        card_target_at(area, &lx, area.x + 6, top_y + 1, &mut PaneCache::default()),
        Some(crate::app::state::CardTarget::Tool {
            message_id: "msg-call-1".into(),
            call_id: "call-1".into(),
            section: crate::app::state::CardSection::Body,
        }),
        "点折行命令切换折叠"
    );
}

#[test]
fn folded_thinking_and_long_param_are_clickable() {
    // 回归：折叠态点内容要能展开——点击面必须覆盖**整个卡片**，而不只是「正文区」。
    let area = Rect::new(0, 0, 60, 20);
    // 1) 折叠的思考块：只有头部一行，点它必须能展开。
    let mut lx = Lx::new();
    lx.chat.messages = vec![ChatMessageView {
        sequence: 0,
        id: "think".into(),
        role: "assistant".into(),
        status: "complete".into(),
        turn_index: 0,
        kind: "chat".into(),
        shell: None,
        steps: vec![AssistantStepView::Part {
            position: 0,
            part: crate::app::state::ChatPartView {
                kind: crate::chat::events::PartKind::Reasoning,
                content: "想一想".into(),
                provider_metadata_json: None,
                started_at_ms: None,
                ended_at_ms: None,
                completed_at_ms: None,
            },
        }],
        model_label: None,
        error_code: None,
        input_tokens: None,
        output_tokens: None,
        cache_read_tokens: None,
        duration_ms: None,
        reports: Vec::new(),
        attachments: Vec::new(),
    }];
    lx.chat.follow_bottom = true;
    let view = history_view(area, &lx, &mut PaneCache::default()).expect("历史区");
    let card =
        crate::ui::main_content::lx::history::layout(view.body, &lx, &mut PaneCache::default())
            .cards
            .into_iter()
            .next()
            .expect("思考块卡片");
    let header_y = card
        .card
        .y
        .saturating_sub(view.body.y)
        .saturating_sub(u16::try_from(view.scroll).unwrap_or(0))
        .saturating_add(view.body.y);
    assert_eq!(
        card_target_at(area, &lx, area.x + 6, header_y, &mut PaneCache::default()),
        Some(crate::app::state::CardTarget::Thinking {
            message_index: 0,
            step_index: 0,
        }),
        "折叠态点思考头必须可展开"
    );

    // 2) 折叠的超长参数：点详情行必须能展开。
    let mut lx = Lx::new();
    let long = "src/some/really/deeply/nested/path/that/keeps/going/and/going/mod.rs".repeat(3);
    lx.chat.messages = vec![ChatMessageView {
        sequence: 0,
        id: "m-param".into(),
        role: "assistant".into(),
        status: "complete".into(),
        turn_index: 0,
        kind: "chat".into(),
        shell: None,
        steps: vec![AssistantStepView::Tool {
            position: 0,
            call: ToolCallView {
                call_id: "call-param".into(),
                name: "read".into(),
                position: 0,
                arguments_json: String::new(),
                status: ToolStatus::Completed,
                summary: Some(Box::new(crate::app::state::ToolSummary {
                    path: Some(long),
                    range: Some((1, 9)),
                    ..Default::default()
                })),
                error: None,
                result_json: None,
                detail: None,
            },
        }],
        model_label: None,
        error_code: None,
        input_tokens: None,
        output_tokens: None,
        cache_read_tokens: None,
        duration_ms: None,
        reports: Vec::new(),
        attachments: Vec::new(),
    }];
    lx.chat.follow_bottom = true;
    let view = history_view(area, &lx, &mut PaneCache::default()).expect("历史区");
    let cards =
        crate::ui::main_content::lx::history::layout(view.body, &lx, &mut PaneCache::default())
            .cards;
    let card = cards.first().expect("长参数可展开，必须有卡片");
    assert!(card.card.height > 1, "长参数卡片含详情行：{card:?}");
    let detail_y = card
        .card
        .y
        .saturating_sub(view.body.y)
        .saturating_sub(u16::try_from(view.scroll).unwrap_or(0))
        .saturating_add(view.body.y)
        .saturating_add(1);
    assert_eq!(
        card_target_at(area, &lx, area.x + 6, detail_y, &mut PaneCache::default()),
        Some(crate::app::state::CardTarget::Tool {
            message_id: "m-param".into(),
            call_id: "call-param".into(),
            section: crate::app::state::CardSection::Body,
        }),
        "折叠态点参数必须可展开"
    );
}
