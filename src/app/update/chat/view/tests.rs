//! DB 记录 → 步骤时间线投影的纯测试：跨表按 position 合并、顺序唯一、中断工具可见。

use super::*;
use crate::db::chat::{ChatMessage, ChatPart, ToolCallRecord};

fn part(position: i64, kind: &str, content: &str) -> ChatPart {
    ChatPart {
        position,
        kind: kind.into(),
        content: content.into(),
        provider_metadata_json: None,
        started_at_ms: None,
        ended_at_ms: None,
        completed_at_ms: None,
    }
}

fn tool(position: i64, call_id: &str, name: &str, status: &str) -> ToolCallRecord {
    ToolCallRecord {
        call_id: call_id.into(),
        message_id: "a1".into(),
        name: name.into(),
        arguments_json: r#"{"path":"src/a.rs"}"#.into(),
        position,
        status: status.into(),
        result_json: None,
        error_text: None,
        started_at_ms: 100,
        completed_at_ms: Some(200),
        duration_ms: Some(100),
    }
}

fn message(parts: Vec<ChatPart>, tools: Vec<ToolCallRecord>) -> ChatMessage {
    ChatMessage {
        external_id: "a1".into(),
        sequence: 1,
        turn_index: 0,
        role: "assistant".into(),
        status: "complete".into(),
        model_provider: None,
        model_id: None,
        variant: None,
        stop_reason: None,
        error_code: None,
        input_tokens: None,
        output_tokens: None,
        cache_read_tokens: None,
        cache_write_tokens: None,
        created_at_ms: 0,
        completed_at_ms: None,
        parts,
        tools,
    }
}

/// 交错 position 的 parts 与 tools 必须按 position 合并成唯一步骤序列。
#[test]
fn parts_and_tools_merge_by_position() {
    let models = ModelsSnapshot::default();
    let db = message(
        vec![
            part(0, "reasoning", "想"),
            part(2, "text", "答"),
            part(4, "text", "续答"),
        ],
        vec![
            // 故意乱序给出（DB 查询按 position 有序，这里验证投影不依赖入参顺序）。
            tool(3, "c2", "search", "completed"),
            tool(1, "c1", "read", "completed"),
        ],
    );
    let view = view_message_of(&db, &models);
    let positions: Vec<i64> = view.steps.iter().map(|step| step.position()).collect();
    assert_eq!(positions, vec![0, 1, 2, 3, 4], "唯一时间线顺序");
    assert_eq!(view.steps.len(), 5);
    assert_eq!(view.reasoning_parts().count(), 1);
    assert_eq!(view.text_parts().count(), 2);
    assert_eq!(view.tools().count(), 2);
    assert_eq!(view.tool("c1").map(|call| call.position), Some(1));
    assert_eq!(
        view.tool("c1").map(|call| call.status),
        Some(ToolStatus::Completed)
    );
}

/// `interrupted` 工具在历史中可见（不隐藏、不伪装完成）。
#[test]
fn interrupted_tool_is_visible_with_reason() {
    let models = ModelsSnapshot::default();
    let mut record = tool(1, "c1", "write", "interrupted");
    record.error_text = Some(crate::db::chat::INTERRUPTED_ERROR_CODE.into());
    let view = view_message_of(&message(vec![part(0, "text", "答")], vec![record]), &models);
    let call = view.tool("c1").expect("可见");
    assert_eq!(call.status, ToolStatus::Interrupted);
    assert_eq!(
        call.error.as_deref(),
        Some(crate::db::chat::INTERRUPTED_ERROR_CODE)
    );
    assert_eq!(
        call.summary
            .as_ref()
            .and_then(|summary| summary.path.as_deref()),
        Some("src/a.rs"),
        "摘要只装路径，不装结果体"
    );
}

/// 未知状态按 interrupted 保守处理（不伪装成功）。
#[test]
fn unknown_status_falls_back_to_interrupted() {
    let models = ModelsSnapshot::default();
    let record = tool(0, "c1", "read", "some-new-status");
    let view = view_message_of(&message(Vec::new(), vec![record]), &models);
    assert_eq!(
        view.tool("c1").map(|call| call.status),
        Some(ToolStatus::Interrupted)
    );
}

/// 旧纯文本会话（无工具）投影等价于原来的 parts 顺序（回归）。
#[test]
fn plain_text_history_matches_legacy_order() {
    let models = ModelsSnapshot::default();
    let db = message(
        vec![part(0, "reasoning", "旧思考"), part(1, "text", "旧正文")],
        Vec::new(),
    );
    let view = view_message_of(&db, &models);
    assert_eq!(
        view.parts()
            .map(|part| (part.kind_str(), part.content.as_str()))
            .collect::<Vec<_>>(),
        vec![("reasoning", "旧思考"), ("text", "旧正文")]
    );
    assert!(view.tools().next().is_none());
    assert_eq!(view.text(), "旧正文");
}
