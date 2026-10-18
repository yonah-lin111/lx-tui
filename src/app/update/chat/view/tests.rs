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

// ===== 重进会话：工具详情必须从结果体补全（回归：写文件类操作信息消失）=====

/// 重载后的工具视图：参数派生摘要 + 结果体补全。
fn reloaded(name: &str, arguments: &str, result: &str) -> ToolCallView {
    let mut record = tool(1, "call-1", name, "completed");
    record.arguments_json = arguments.into();
    record.result_json = Some(result.into());
    let view = view_message_of(
        &message(Vec::new(), vec![record]),
        &ModelsSnapshot::default(),
    );
    let AssistantStepView::Tool { call, .. } = &view.steps[0] else {
        panic!("应为工具步骤");
    };
    call.clone()
}

/// 回归：`edit` 的增删计数只在结果体里，重载后必须还在
/// （旧 bug：重进 session 后 `Edit` 只剩方块与空 `└─`）。
#[test]
fn reload_keeps_edit_added_and_removed_counts() {
    let call = reloaded(
        "edit",
        r#"{"patch":"*** Begin Patch\n*** Update File: a.rs\n@@\n-x\n+y\n*** End Patch"}"#,
        r#"{"tool":"edit","changes":[{"path":"a.rs","added":12,"removed":4,"kind":"updated"}],"partial":false}"#,
    );
    let summary = call.summary.expect("结算摘要");
    assert_eq!(summary.added, Some(12), "新增计数必须保留");
    assert_eq!(summary.removed, Some(4), "删除计数必须保留");
}

/// 回归：`write` 的增删计数同样来自结果体。
#[test]
fn reload_keeps_write_added_and_removed_counts() {
    let call = reloaded(
        "write",
        r#"{"path":"docs/notes.md","content":"a\nb"}"#,
        r#"{"tool":"write","path":"docs/notes.md","created":true,"added":18,"removed":0}"#,
    );
    let summary = call.summary.expect("结算摘要");
    assert_eq!(summary.added, Some(18));
    assert_eq!(summary.removed, Some(0), "0 删也要保留（表示整文件覆盖）");
}

/// 回归：`search` 的命中数来自结果体、pattern 来自参数，两者都要在。
#[test]
fn reload_keeps_search_pattern_and_match_count() {
    let call = reloaded(
        "search",
        r#"{"pattern":"测试","path":"tool_test.txt"}"#,
        r#"{"tool":"search","matches":[{"path":"tool_test.txt","line":1,"text":"x","line_truncated":false},{"path":"tool_test.txt","line":3,"text":"y","line_truncated":false},{"path":"tool_test.txt","line":4,"text":"z","line_truncated":false}],"truncated":false,"files_scanned":1}"#,
    );
    let summary = call.summary.expect("结算摘要");
    assert_eq!(summary.pattern.as_deref(), Some("测试"), "pattern 来自参数");
    assert_eq!(summary.matches, Some(3), "命中数来自结果体");
    assert_eq!(summary.path.as_deref(), Some("tool_test.txt"));
}

/// 回归：`read` 的实际行数来自结果体（参数里只有 offset/limit）。
#[test]
fn reload_keeps_read_total_lines() {
    let call = reloaded(
        "read",
        r#"{"path":"src/a.rs","offset":12,"limit":29}"#,
        r#"{"tool":"read","path":"src/a.rs","kind":"file","content":"x","start_line":12,"end_line":40,"next_offset":null,"truncated":false,"total_lines":31}"#,
    );
    let summary = call.summary.expect("结算摘要");
    assert_eq!(summary.range, Some((12, 40)), "区间来自参数");
    assert_eq!(summary.total_lines, Some(31), "实际行数来自结果体");
}

/// 结果体缺失/非法时退回纯参数摘要，不 panic 也不编造。
#[test]
fn reload_without_result_json_falls_back_to_arguments_only() {
    let record = {
        let mut record = tool(1, "call-1", "edit", "interrupted");
        record.arguments_json =
            r#"{"patch":"*** Begin Patch\n*** Update File: a.rs\n*** End Patch"}"#.into();
        record.result_json = None;
        record
    };
    let view = view_message_of(
        &message(Vec::new(), vec![record]),
        &ModelsSnapshot::default(),
    );
    let AssistantStepView::Tool { call, .. } = &view.steps[0] else {
        panic!("应为工具步骤");
    };
    let summary = call.summary.as_ref().expect("参数摘要");
    assert_eq!(summary.added, None, "没有结果体就没有增删计数");
    assert_eq!(summary.removed, None);

    // 非法结果体同样不 panic：仍然只按参数派生（patch 里有目标文件，所以有摘要）。
    let call = reloaded(
        "edit",
        r#"{"patch":"*** Begin Patch\n*** Update File: a.rs\n*** End Patch"}"#,
        "not json",
    );
    let summary = call.summary.expect("参数派生摘要");
    assert_eq!(
        summary.path.as_deref(),
        Some("a.rs"),
        "非法结果体时仍能从 patch 取到目标文件"
    );
    assert_eq!(summary.added, None, "非法结果体不编造计数");

    // 参数也没有可用字段时，摘要为空（不凭空造一个空摘要）。
    let call = reloaded("edit", "{}", "not json");
    assert!(call.summary.is_none(), "无任何可用字段时不造摘要");
}
