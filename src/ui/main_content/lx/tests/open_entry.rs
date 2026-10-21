//! 工具卡片命中回归测试：进程卡片 `[open]` 与文件差异卡片标题两种动作。
//!
//! 用户要求（决策 4/7/9）：进程卡片只有 `[open]` 可点；点它打开后台任务面板，并把
//! 高亮定位到该卡片对应的后台进程；标题行与命令行是纯展示。文件差异卡片反过来——
//! 只有标题行可点（展开/收起），正文保留聊天选区；两者不能用 `session_id` 是否为空
//! 互相猜测。

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
                    lines: vec![
                        DiffLineView {
                            kind: DiffLineKind::Removed,
                            old_line: Some(1),
                            new_line: None,
                            text: "old".into(),
                            no_final_newline: false,
                        },
                        DiffLineView {
                            kind: DiffLineKind::Added,
                            old_line: None,
                            new_line: Some(1),
                            text: "new".into(),
                            no_final_newline: false,
                        },
                    ],
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
fn file_diff_title_toggles_and_body_is_not_a_hit() {
    let area = Rect::new(0, 0, 60, 20);
    let mut lx = Lx::new();
    lx.chat.messages = vec![file_message("call-edit")];
    lx.chat.follow_bottom = true;
    let view = history_view(area, &lx).expect("历史区");
    let hit = crate::ui::main_content::lx::history::layout(view.body, &lx)
        .tools
        .into_iter()
        .next()
        .expect("文件卡片标题命中");
    assert_eq!(
        hit.kind,
        crate::ui::main_content::lx::history::ToolHitKind::Toggle
    );
    let title_y = hit
        .rect
        .y
        .saturating_sub(view.body.y)
        .saturating_sub(u16::try_from(view.scroll).unwrap_or(0))
        .saturating_add(view.body.y);
    assert_eq!(
        chat_hit(area, &lx, area.x + 6, title_y),
        Some(LxChatHit::ToolToggle {
            message_id: "msg-call-edit".into(),
            call_id: "call-edit".into(),
        }),
        "标题行命中展开切换，不是任务面板"
    );
    // 正文行保留聊天选区，不是命中区。
    assert_eq!(chat_hit(area, &lx, area.x + 6, title_y + 1), None);
    assert_eq!(chat_hit(area, &lx, area.x + 6, title_y + 2), None);
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
