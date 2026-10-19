//! 步骤视图与工具状态的纯数据测试（无 IO、无时间依赖）。

use super::*;

fn text(content: &str) -> ChatPartView {
    ChatPartView {
        kind: PartKind::Text,
        content: content.into(),
        provider_metadata_json: None,
        started_at_ms: None,
        ended_at_ms: None,
        completed_at_ms: None,
    }
}

fn reasoning(content: &str) -> ChatPartView {
    ChatPartView {
        kind: PartKind::Reasoning,
        content: content.into(),
        ..text("")
    }
}

fn assistant() -> ChatMessageView {
    ChatMessageView {
        id: "a".into(),
        role: "assistant".into(),
        status: "streaming".into(),
        turn_index: 0,
        steps: Vec::new(),
        model_label: None,
        error_code: None,
        input_tokens: None,
        output_tokens: None,
        cache_read_tokens: None,
        duration_ms: None,
    }
}

#[test]
fn positions_increment_per_new_step_and_stay_stable_on_merge() {
    let mut message = assistant();
    assert_eq!(message.next_position(), 0);
    assert_eq!(message.push_part(text("a")), 0);
    assert_eq!(message.push_part(reasoning("r")), 1);
    assert_eq!(message.push_tool(ToolCallView::new("c1", "read", 0)), 2);
    assert_eq!(message.push_part(text("b")), 3);
    assert_eq!(
        message
            .steps
            .iter()
            .map(AssistantStepView::position)
            .collect::<Vec<_>>(),
        vec![0, 1, 2, 3]
    );
    assert_eq!(message.max_position(), Some(3));
    assert_eq!(message.next_position(), 4);
}

#[test]
fn iterator_helpers_skip_tools_and_keep_order() {
    let mut message = assistant();
    message.push_part(reasoning("想"));
    message.push_part(text("答"));
    message.push_tool(ToolCallView::new("c1", "read", 0));
    message.push_part(text("续"));

    assert_eq!(message.parts().count(), 3);
    assert_eq!(message.text_parts().count(), 2);
    assert_eq!(message.reasoning_parts().count(), 1);
    assert_eq!(message.tools().count(), 1);
    assert_eq!(message.text(), "答续");
    assert!(message.tool("c1").is_some());
    assert!(message.tool("missing").is_none());
    assert!(
        message
            .step_at(2)
            .and_then(AssistantStepView::tool)
            .is_some()
    );
    assert!(message.step_at(9).is_none());
}

#[test]
fn open_reasoning_tracks_only_unfinished_tail() {
    let mut message = assistant();
    message.push_part(text("正文"));
    assert!(message.open_reasoning_mut().is_none(), "正文不是思考块");

    message.push_part(reasoning("想"));
    assert!(message.open_reasoning_mut().is_some());

    if let Some(part) = message.open_reasoning_mut() {
        part.completed_at_ms = Some(1);
    }
    assert!(message.open_reasoning_mut().is_none(), "已完成的不再开放");

    message.push_tool(ToolCallView::new("c1", "read", 0));
    assert!(message.open_reasoning_mut().is_none(), "尾部不是思考块");
}

#[test]
fn tool_status_terminal_classification_and_stored_mapping() {
    for status in [
        ToolStatus::Completed,
        ToolStatus::Failed,
        ToolStatus::Cancelled,
        ToolStatus::Interrupted,
    ] {
        assert!(status.is_terminal(), "{status:?}");
        assert!(!status.is_open(), "{status:?}");
    }
    for status in [
        ToolStatus::InputStreaming,
        ToolStatus::Pending,
        ToolStatus::Running,
    ] {
        assert!(status.is_open(), "{status:?}");
        assert!(!status.is_terminal(), "{status:?}");
    }
    assert_eq!(
        ToolStatus::from_stored("pending"),
        Some(ToolStatus::Pending)
    );
    assert_eq!(
        ToolStatus::from_stored("interrupted"),
        Some(ToolStatus::Interrupted)
    );
    assert_eq!(ToolStatus::from_stored("input_streaming"), None);
    assert_eq!(ToolStatus::from_stored("weird"), None);
}

#[test]
fn summary_extracts_path_and_read_range_only() {
    let summary = ToolSummary::from_arguments(r#"{"path":"src/a.rs","offset":10,"limit":5}"#);
    assert_eq!(summary.path.as_deref(), Some("src/a.rs"));
    assert_eq!(summary.range, Some((10, 14)));

    // 单边参数推不出闭区间：区间以结果体为准，参数侧只是兜底。
    let offset_only = ToolSummary::from_arguments(r#"{"path":"src/b.rs","offset":7}"#);
    assert_eq!(
        offset_only.range, None,
        "只有 offset 不推导区间（旧实现会塌成 (7,7)）"
    );

    let limit_only = ToolSummary::from_arguments(r#"{"path":"src/c.rs","limit":29}"#);
    assert_eq!(limit_only.range, None, "只有 limit 同样不推导区间");

    let zero_limit = ToolSummary::from_arguments(r#"{"path":"src/d.rs","offset":3,"limit":0}"#);
    assert_eq!(zero_limit.range, None, "limit 为 0 时没有区间");

    let no_range = ToolSummary::from_arguments(r#"{"path":"src/e.rs"}"#);
    assert_eq!(no_range.range, None);

    let invalid = ToolSummary::from_arguments("{not json");
    assert_eq!(invalid, ToolSummary::default());
}

#[test]
fn summary_path_is_truncated_to_max_chars() {
    let long = "x".repeat(MAX_SUMMARY_CHARS + 40);
    let summary = ToolSummary::from_arguments(&format!(r#"{{"path":"{long}"}}"#));
    let path = summary.path.expect("path");
    assert_eq!(path.chars().count(), MAX_SUMMARY_CHARS);
}

#[test]
fn accept_arguments_marks_pending_or_reports_invalid() {
    let mut call = ToolCallView::new("c1", "read", 0);
    assert_eq!(call.status, ToolStatus::InputStreaming);
    assert!(call.accept_arguments(r#"{"path":"a"}"#.into()));
    assert_eq!(call.status, ToolStatus::Pending);
    assert_eq!(
        call.summary.and_then(|summary| summary.path).as_deref(),
        Some("a")
    );

    let mut broken = ToolCallView::new("c2", "read", 1);
    assert!(!broken.accept_arguments("{半截".into()));
    // 调用方负责标失败（此处只报结果）。
    assert_eq!(broken.status, ToolStatus::InputStreaming);
    broken.fail("bad");
    assert_eq!(broken.status, ToolStatus::Failed);
    assert_eq!(broken.error.as_deref(), Some("bad"));
}
