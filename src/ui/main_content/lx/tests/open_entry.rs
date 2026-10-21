//! 工具卡片命中回归测试：进程卡片 `open` 与文件差异卡片末尾动作行两种动作。
//!
//! 用户要求：进程卡片只有 `open` 可点，点它打开后台任务面板并把高亮定位到该卡片
//! 对应的后台进程；进程到终态后整行不画、也不再登记命中区。文件差异卡片反过来——
//! 展开/收起的唯一入口是**树体末尾动作行**，标题与正文都不是命中动作：正文保留
//! 聊天选区，点一下（未拖拽）才折叠。两者不能用 `session_id` 是否为空互相猜测。

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

/// 卡片三行在屏幕上的 y 坐标（标题 / 命令 / `[open]`）。
fn card_rows(area: Rect, lx: &Lx) -> (u16, u16, u16) {
    let view = history_view(area, lx).expect("历史区");
    let hit = crate::ui::main_content::lx::history::layout(view.body, lx)
        .tools
        .into_iter()
        .next()
        .expect("[open] 命中区");
    let open_y = hit
        .rect
        .y
        .saturating_sub(view.body.y)
        .saturating_sub(u16::try_from(view.scroll).unwrap_or(0))
        .saturating_add(view.body.y);
    let title_y = open_y.saturating_sub(2);
    (title_y, open_y.saturating_sub(1), open_y)
}

#[test]
fn clicking_open_row_opens_the_task_panel_for_that_process() {
    let area = Rect::new(0, 0, 60, 20);
    let lx = lx_with_command("call-1", 42);
    let (_, _, open_y) = card_rows(area, &lx);
    assert_eq!(
        chat_hit(area, &lx, area.x + 6, open_y),
        Some(LxChatHit::ToolOpen(Some(42))),
        "点 [open] 行触发打开面板并携带进程会话 id"
    );
}

#[test]
fn title_and_command_rows_are_not_clickable() {
    // 决策 7：只有 `[open]` 可点，标题与命令是纯展示。
    let area = Rect::new(0, 0, 60, 20);
    let lx = lx_with_command("call-1", 42);
    let (title_y, command_y, _) = card_rows(area, &lx);
    assert_eq!(
        chat_hit(area, &lx, area.x + 6, title_y),
        None,
        "标题行不再是命中区"
    );
    assert_eq!(
        chat_hit(area, &lx, area.x + 6, command_y),
        None,
        "命令行不是命中区"
    );
}

#[test]
fn file_tool_cards_have_no_open_hit() {
    // 文件工具没有后台进程，不该出现 `[open]` 命中区。
    let mut lx = Lx::new();
    lx.chat.messages = vec![ChatMessageView {
        id: "msg-read".into(),
        role: "assistant".into(),
        status: "complete".into(),
        turn_index: 0,
        steps: vec![AssistantStepView::Tool {
            position: 0,
            call: ToolCallView {
                call_id: "call-read".into(),
                name: "read".into(),
                position: 0,
                arguments_json: r#"{"path":"src/main.rs"}"#.into(),
                status: ToolStatus::Completed,
                summary: None,
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
    }];
    let area = Rect::new(0, 0, 60, 20);
    let view = history_view(area, &lx).expect("历史区");
    assert!(
        crate::ui::main_content::lx::history::layout(view.body, &lx)
            .tools
            .is_empty(),
        "文件工具卡片没有 [open] 命中区"
    );
}

// ===== bug 回归：后台任务面板打开时不得切换 agent =====

#[test]
fn task_panel_blocks_entry_switch_hits() {
    // 真实 bug：面板打开后点"原本三个 agent 入口"的位置仍能切换 agent——
    // 因为 `layout` 照旧算出入口矩形，命中穿透到下面那层。
    let area = Rect::new(0, 0, 100, 20);
    let mut lx = Lx::new();
    let closed = layout(area, &lx);
    assert!(!closed.entries.is_empty(), "前提：关闭态确实有入口标签");

    // 记录关闭态每个入口的位置，打开面板后逐点验证都不再命中。
    let positions: Vec<(u16, u16)> = closed
        .entries
        .iter()
        .map(|(_, rect)| (rect.x + 1, rect.y))
        .collect();
    let mut lx_open = Lx::new();
    lx_open.task = Some(crate::app::state::TaskPage::default());
    assert!(
        layout(area, &lx_open).entries.is_empty(),
        "面板打开时不再登记入口矩形"
    );
    for (column, row) in positions.clone() {
        assert_eq!(
            entry_hit(area, &lx_open, column, row),
            None,
            "面板打开后点 ({column},{row}) 不得切换 agent"
        );
    }
    // 关闭面板后入口命中恢复。
    lx.task = None;
    let (column, row) = positions.first().copied().expect("至少一个入口");
    assert!(
        entry_hit(area, &lx, column, row).is_some(),
        "关闭面板后入口恢复正常"
    );
}

// ===== 文件差异卡片：只有标题行可点，动作为展开/收起 =====

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
fn file_diff_footer_toggles_and_title_is_not_a_hit() {
    // 用户要求「不是点击工具名称展开，是点击底部 … more changes 展开」。
    let area = Rect::new(0, 0, 60, 20);
    let mut lx = Lx::new();
    lx.chat.messages = vec![file_message("call-edit")];
    lx.chat.follow_bottom = true;
    let view = history_view(area, &lx).expect("历史区");
    let hit = crate::ui::main_content::lx::history::layout(view.body, &lx)
        .tools
        .into_iter()
        .next()
        .expect("文件卡片动作行命中");
    assert_eq!(
        hit.kind,
        crate::ui::main_content::lx::history::ToolHitKind::Toggle
    );
    let footer_y = hit
        .rect
        .y
        .saturating_sub(view.body.y)
        .saturating_sub(u16::try_from(view.scroll).unwrap_or(0))
        .saturating_add(view.body.y);
    assert_eq!(
        chat_hit(area, &lx, area.x + 6, footer_y),
        Some(LxChatHit::ToolToggle {
            message_id: "msg-call-edit".into(),
            call_id: "call-edit".into(),
        }),
        "底部动作行命中展开切换"
    );
    // 标题行与正文行都不是命中动作：正文要留给聊天选区。
    for row in [footer_y - 1, footer_y - 2, footer_y - 3] {
        assert_eq!(
            chat_hit(area, &lx, area.x + 6, row),
            None,
            "行 {row} 不该是点击动作"
        );
    }
}

#[test]
fn expanded_card_body_is_the_click_to_collapse_target() {
    // 展开后：动作行仍是 Toggle 入口；正文整块进入 `CardHit.body`，点它（未拖拽）折叠。
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
    // 正文至少一行（标题 + 文件头 + 3 行预览 + 动作行 = 6 行，正文 4 行）。
    assert!(card.body.height >= 2, "展开后正文非空：{card:?}");
    let body_y = card
        .body
        .y
        .saturating_sub(view.body.y)
        .saturating_sub(u16::try_from(view.scroll).unwrap_or(0))
        .saturating_add(view.body.y);
    assert_eq!(
        card_body_at(area, &lx, area.x + 6, body_y),
        Some(crate::app::state::ChatHover::Tool {
            message_id: "msg-call-edit".into(),
            call_id: "call-edit".into(),
        }),
        "正文按下点会被记录为可折叠目标"
    );
    // 标题行不是正文：点它不会折叠。
    assert_eq!(card_body_at(area, &lx, area.x + 6, body_y - 1), None);
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
            .tools
            .is_empty(),
        "没有差异正文时不登记命中区"
    );
}
