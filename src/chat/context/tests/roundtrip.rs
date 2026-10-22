//! 视图 → 记录 → 装配往返：`chat_message_views_to_db`（`chat_part_views_to_db` 的
//! 唯一实现）必须保留工具步骤与 position，使 `/resume` 与当前轮装配同源。

use super::*;
use crate::app::state::{
    AssistantStepView, ChatMessageView, ChatPartView, ToolCallView, ToolStatus, ToolSummary,
};
use crate::chat::events::PartKind;

/// 一个 text 步骤。
fn text_step(position: i64, content: &str) -> AssistantStepView {
    AssistantStepView::Part {
        position,
        part: ChatPartView {
            kind: PartKind::Text,
            content: content.into(),
            provider_metadata_json: None,
            started_at_ms: None,
            ended_at_ms: None,
            completed_at_ms: None,
        },
    }
}

/// 一个工具步骤。
#[allow(clippy::too_many_arguments)]
fn tool_step(
    position: i64,
    call_id: &str,
    name: &str,
    status: ToolStatus,
    arguments_json: &str,
    result_json: Option<&str>,
    error: Option<&str>,
) -> AssistantStepView {
    AssistantStepView::Tool {
        position,
        call: ToolCallView {
            call_id: call_id.into(),
            name: name.into(),
            position,
            arguments_json: arguments_json.into(),
            status,
            summary: Some(Box::new(ToolSummary::from_arguments(arguments_json))),
            error: error.map(str::to_string),
            result_json: result_json.map(str::to_string),
            detail: None,
        },
    }
}

/// 一条视图消息。
fn view(
    id: &str,
    role: &str,
    status: &str,
    turn_index: i64,
    steps: Vec<AssistantStepView>,
) -> ChatMessageView {
    ChatMessageView {
        sequence: 0,
        id: id.into(),
        role: role.into(),
        status: status.into(),
        turn_index,
        steps,
        model_label: None,
        error_code: None,
        input_tokens: None,
        output_tokens: None,
        cache_read_tokens: None,
        duration_ms: None,
    }
}

/// 视图 → DB 记录 → `assemble`：call/result 与视图一致，position 保持。
#[test]
fn view_to_db_round_trip_keeps_tool_pairing_and_position() {
    let views = vec![
        view("u1", "user", "complete", 1, vec![text_step(0, "读文件")]),
        view(
            "a1",
            "assistant",
            "complete",
            1,
            vec![
                text_step(0, "我先读"),
                tool_step(
                    1,
                    "call_a",
                    "read",
                    ToolStatus::Completed,
                    r#"{"path":"src/a.rs"}"#,
                    Some(r#"{"tool":"read","content":"1: fn main() {}"}"#),
                    None,
                ),
                text_step(2, "读到了"),
            ],
        ),
    ];
    // 窗格视图 → 记录（`event::chat::chat_part_views_to_db` 委托本函数）。
    let records = crate::db::chat::chat_message_views_to_db(&views);
    assert_eq!(records.len(), 2);
    assert_eq!(records[1].tools.len(), 1, "工具步骤必须映射进记录");
    let record = &records[1].tools[0];
    assert_eq!(record.position, 1, "position 保持视图编号空间");
    assert_eq!(record.status, "completed");
    assert_eq!(record.name, "read");
    assert_eq!(
        record.result_json.as_deref(),
        Some(r#"{"tool":"read","content":"1: fn main() {}"}"#)
    );
    // 记录 → assemble：顺序与视图完全一致。
    let assembly = assemble_unbounded(&records, "继续");
    assert_eq!(
        item_views(&assembly.request.items),
        vec![
            ItemView::User("读文件".into()),
            ItemView::Assistant("我先读".into()),
            ItemView::Call("call_a".into(), "read".into()),
            ItemView::Result("call_a".into(), false),
            ItemView::Assistant("读到了".into()),
            ItemView::User("继续".into()),
        ]
    );
    assert_no_orphan_results(&assembly.request.items);
}

/// `/resume` 场景：视图来自 DB 加载（含 interrupted 工具）→ 记录 → 装配。
///
/// interrupted 工具在视图/历史里可见，但**不得**进入模型请求。
#[test]
fn resume_view_keeps_interrupted_tool_visible_but_not_replayed() {
    let views = vec![
        view(
            "u1",
            "user",
            "complete",
            0,
            vec![text_step(0, "写两个文件")],
        ),
        view(
            "a1",
            "assistant",
            "interrupted",
            0,
            vec![
                text_step(0, "先写第一个"),
                tool_step(
                    1,
                    "call_done",
                    "write",
                    ToolStatus::Completed,
                    r#"{"path":"a.txt","content":"1"}"#,
                    Some(r#"{"tool":"write","path":"a.txt"}"#),
                    None,
                ),
                tool_step(
                    2,
                    "call_died",
                    "write",
                    ToolStatus::Interrupted,
                    r#"{"path":"b.txt","content":"2"}"#,
                    None,
                    Some(crate::db::chat::INTERRUPTED_ERROR_CODE),
                ),
            ],
        ),
    ];
    let records = crate::db::chat::chat_message_views_to_db(&views);
    assert_eq!(records[1].tools.len(), 2, "两个工具步骤都映射进记录");
    assert_eq!(records[1].tools[1].status, "interrupted");
    let assembly = assemble_unbounded(&records, "继续");
    // assistant 为 interrupted → 整轮（含已完成工具）都不回放。
    assert_eq!(
        item_views(&assembly.request.items),
        vec![ItemView::User("继续".into())],
        "中断轮次整体排除，interrupted 与 completed 工具都不进请求"
    );
    assert_no_orphan_results(&assembly.request.items);
}

/// 工具视图缺 `result_json`（未结算）时映射为不可回放，不伪造结果。
#[test]
fn tool_view_without_result_maps_to_unreplayable_record() {
    let views = vec![
        view("u1", "user", "complete", 0, vec![text_step(0, "读")]),
        view(
            "a1",
            "assistant",
            "complete",
            0,
            vec![
                text_step(0, "读一下"),
                tool_step(
                    1,
                    "call_pending",
                    "read",
                    ToolStatus::Pending,
                    r#"{"path":"a.rs"}"#,
                    None,
                    None,
                ),
            ],
        ),
    ];
    let records = crate::db::chat::chat_message_views_to_db(&views);
    assert_eq!(records[1].tools[0].status, "pending");
    assert!(records[1].tools[0].result_json.is_none());
    let assembly = assemble_unbounded(&records, "继续");
    assert_eq!(
        item_views(&assembly.request.items),
        vec![
            ItemView::User("读".into()),
            ItemView::Assistant("读一下".into()),
            ItemView::User("继续".into()),
        ],
        "未结算工具不回放"
    );
}
