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
        id: format!("msg-{call_id}"),
        role: "assistant".into(),
        status: "complete".into(),
        turn_index: 0,
        steps: vec![AssistantStepView::Tool { position: 0, call }],
        model_label: None,
        error_code: None,
        input_tokens: None,
        output_tokens: None,
        cache_read_tokens: None,
        duration_ms: None,
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
        id: format!("msg-{call_id}"),
        role: "assistant".into(),
        status: "complete".into(),
        turn_index: 0,
        steps: vec![AssistantStepView::Tool { position: 0, call }],
        model_label: None,
        error_code: None,
        input_tokens: None,
        output_tokens: None,
        cache_read_tokens: None,
        duration_ms: None,
    }
}

#[test]
fn card_content_is_the_toggle_target() {
    // 没有动作按钮、没有剩余行数：整块内容就是点击面。
    let area = Rect::new(0, 0, 60, 20);
    let mut lx = Lx::new();
    lx.chat.messages = vec![file_message("call-edit")];
    lx.chat.follow_bottom = true;
    let view = history_view(area, &lx).expect("历史区");
    let cards = crate::ui::main_content::lx::history::layout(view.body, &lx).cards;
    assert_eq!(cards.len(), 1, "一张卡片");
    let card = &cards[0];
    assert!(card.body.height > 0, "内容区非空：{card:?}");
    let body_y = card
        .body
        .y
        .saturating_sub(view.body.y)
        .saturating_sub(u16::try_from(view.scroll).unwrap_or(0))
        .saturating_add(view.body.y);
    assert_eq!(
        card_target_at(area, &lx, area.x + 6, body_y),
        Some(crate::app::state::CardTarget::Tool {
            message_id: "msg-call-edit".into(),
            call_id: "call-edit".into(),
        }),
        "点内容记录为折叠目标"
    );
    // 标题行不是内容：点它不切换（避免误触）。
    assert_eq!(card_target_at(area, &lx, area.x + 6, body_y - 1), None);
}

#[test]
fn expanded_card_body_is_the_click_to_collapse_target() {
    // 展开后：整块内容仍是 `CardHit.body`，点它（未拖拽）收起。
    let area = Rect::new(0, 0, 60, 20);
    let mut lx = Lx::new();
    lx.chat.messages = vec![file_message("call-edit")];
    lx.chat.follow_bottom = true;
    lx.chat.toggle_detail("msg-call-edit", "call-edit");
    let view = history_view(area, &lx).expect("历史区");
    let cards = crate::ui::main_content::lx::history::layout(view.body, &lx).cards;
    let card = cards.first().expect("文件卡片整块命中");
    assert!(
        matches!(
            &card.kind,
            crate::ui::main_content::lx::history::CardKind::Tool { expanded: true, .. }
        ),
        "布局记录的展开状态必须与状态层一致"
    );
    // 正文至少一行（标题 + 文件头 + 预览）。
    assert!(card.body.height >= 2, "展开后正文非空：{card:?}");
    let body_y = card
        .body
        .y
        .saturating_sub(view.body.y)
        .saturating_sub(u16::try_from(view.scroll).unwrap_or(0))
        .saturating_add(view.body.y);
    assert_eq!(
        card_target_at(area, &lx, area.x + 6, body_y),
        Some(crate::app::state::CardTarget::Tool {
            message_id: "msg-call-edit".into(),
            call_id: "call-edit".into(),
        }),
        "正文按下点会被记录为可折叠目标"
    );
    // 标题行不是正文：点它不会折叠。
    assert_eq!(card_target_at(area, &lx, area.x + 6, body_y - 1), None);
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
    let view = history_view(area, &lx).expect("历史区");
    assert!(
        crate::ui::main_content::lx::history::layout(view.body, &lx)
            .cards
            .iter()
            .all(|card| card.body.height == 0),
        "无差异正文时内容区不可展开"
    );
}

#[test]
fn process_card_has_no_open_row_and_single_line_command_is_not_clickable() {
    // 用户要求移除 `open` 按钮：进程卡片不再有入口行。
    // 命令一行放得下时**没有更多内容可展开**，因此内容区零高——点了也不会变。
    let area = Rect::new(0, 0, 60, 20);
    let lx = lx_with_command("call-1", 42);
    let view = history_view(area, &lx).expect("历史区");
    let cards = crate::ui::main_content::lx::history::layout(view.body, &lx).cards;
    let card = cards.first().expect("进程卡片整块命中");
    assert_eq!(card.body.height, 0, "单行命令没有可展开内容：{card:?}");
    let mut buf = Buffer::empty(area);
    crate::ui::main_content::lx::render_history_for_test(area, &mut buf, &lx, 0);
    let text = super::buffer_text(&buf);
    assert!(
        !text.contains(text::agent::TASK_OPEN_LABEL),
        "没有 open 行：{text}"
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
    let view = history_view(area, &lx).expect("历史区");
    let cards = crate::ui::main_content::lx::history::layout(view.body, &lx).cards;
    let card = cards.first().expect("进程卡片整块命中");
    assert!(card.body.height > 0, "折行命令可展开：{card:?}");
    let body_y = card
        .body
        .y
        .saturating_sub(view.body.y)
        .saturating_sub(u16::try_from(view.scroll).unwrap_or(0))
        .saturating_add(view.body.y);
    assert_eq!(
        card_target_at(area, &lx, area.x + 6, body_y),
        Some(crate::app::state::CardTarget::Tool {
            message_id: "msg-call-1".into(),
            call_id: "call-1".into(),
        }),
        "点折行命令切换折叠"
    );
}
