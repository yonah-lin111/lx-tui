//! 工具步骤回放：顺序、失败结果、不可回放状态、非法参数与跨 provider 规范化。

use super::*;
use crate::db::chat::{ChatMessage, ChatPart, ToolCallRecord};

/// 一轮含 text + tool call/result + text：items 顺序 = text, call, result, text。
#[test]
fn text_call_result_text_preserves_timeline_order() {
    let history = turn_with_tool("我先读文件。", "call_a", "读到了。");
    let assembly = assemble_unbounded(&history, "继续");
    assert_eq!(
        item_views(&assembly.request.items),
        vec![
            ItemView::User("读文件".into()),
            ItemView::Assistant("我先读文件。".into()),
            ItemView::Call("call_a".into(), "read".into()),
            ItemView::Result("call_a".into(), false),
            ItemView::Assistant("读到了。".into()),
            ItemView::User("继续".into()),
        ],
        "正文与工具按 position 合并成唯一顺序"
    );
    assert_no_orphan_results(&assembly.request.items);
    // 旧纯文本视图不受工具影响：只拼正文。
    let turns: Vec<(&str, &str)> = assembly
        .request
        .turns
        .iter()
        .map(|turn| (turn.role.as_str(), turn.text.as_str()))
        .collect();
    assert_eq!(
        turns,
        vec![
            ("user", "读文件"),
            ("assistant", "我先读文件。读到了。"),
            ("user", "继续"),
        ]
    );
}

/// 工具调用与结果紧邻成对，且 input 是解析后的 JSON 对象。
#[test]
fn tool_call_input_is_parsed_json_object() {
    let history = turn_with_tool("", "call_a", "");
    let assembly = assemble_unbounded(&history, "q");
    let call = assembly
        .request
        .items
        .iter()
        .find_map(|item| match item {
            ModelItem::AssistantToolCall { call_id, input, .. } if call_id == "call_a" => {
                Some(input)
            }
            _ => None,
        })
        .expect("call present");
    assert_eq!(call["path"], serde_json::json!("src/a.rs"));
    // 结果内容原样回放（不做二次编码）。
    let content = assembly
        .request
        .items
        .iter()
        .find_map(|item| match item {
            ModelItem::ToolResult { content, .. } => Some(content.as_str()),
            _ => None,
        })
        .expect("result present");
    assert_eq!(content, r#"{"tool":"read","content":"1: fn main() {}"}"#);
}

/// 失败工具（status=failed + error_text）→ `is_error = true` 成对回放。
#[test]
fn failed_tool_replays_as_error_result() {
    let mut history = vec![
        message("u", 0, 0, "user", "complete", "读缺失文件"),
        message("a", 1, 0, "assistant", "complete", "试着读"),
    ];
    history[1].tools = vec![tool(
        1,
        "call_fail",
        "read",
        "failed",
        r#"{"path":"missing.txt"}"#,
        Some(r#"{"tool":"error","code":"not_found"}"#),
        Some("file not found: missing.txt"),
    )];
    let assembly = assemble_unbounded(&history, "怎么办");
    assert_eq!(
        item_views(&assembly.request.items),
        vec![
            ItemView::User("读缺失文件".into()),
            ItemView::Assistant("试着读".into()),
            ItemView::Call("call_fail".into(), "read".into()),
            ItemView::Result("call_fail".into(), true),
            ItemView::User("怎么办".into()),
        ]
    );
    assert_no_orphan_results(&assembly.request.items);
    // 给模型看的是精简错误原因（不含路径全文之外的敏感内容）。
    let content = assembly
        .request
        .items
        .iter()
        .find_map(|item| match item {
            ModelItem::ToolResult { content, .. } => Some(content.as_str()),
            _ => None,
        })
        .expect("error result present");
    assert!(
        content.contains("file not found"),
        "失败原因回放: {content}"
    );
}

/// 失败但只有 result_json（无 error_text）时仍以 `is_error = true` 回放结果体。
#[test]
fn failed_tool_without_error_text_falls_back_to_result_json() {
    let mut history = vec![
        message("u", 0, 0, "user", "complete", "问"),
        message("a", 1, 0, "assistant", "complete", "答"),
    ];
    history[1].tools = vec![tool(
        1,
        "call_fail",
        "edit",
        "failed",
        r#"{"patch":"*** Begin Patch"}"#,
        Some(r#"{"tool":"error","code":"context_mismatch"}"#),
        None,
    )];
    let assembly = assemble_unbounded(&history, "再试");
    let result = assembly
        .request
        .items
        .iter()
        .find_map(|item| match item {
            ModelItem::ToolResult {
                content, is_error, ..
            } => Some((content.clone(), *is_error)),
            _ => None,
        })
        .expect("result present");
    assert!(result.1, "失败结果必须标记 is_error");
    assert!(result.0.contains("context_mismatch"));
}

/// P1-3：失败补丁的「部分成功」必须跨轮可见——回放内容含 `partially applied`
/// 与已提交文件名，与同轮 `format_for_model` 的口径一致。
#[test]
fn failed_patch_replay_keeps_committed_changes() {
    let mut history = vec![
        message("u", 0, 0, "user", "complete", "改三个文件"),
        message("a", 1, 0, "assistant", "complete", "开始改"),
    ];
    history[1].tools = vec![tool(
        1,
        "call_patch",
        "edit",
        "failed",
        r#"{"patch":"*** Begin Patch"}"#,
        Some(
            r#"{"tool":"error","code":"patch_context_mismatch","message":"hunk lines do not match","changes":[{"path":"/w/a.rs","added":2,"removed":1,"kind":"updated"},{"path":"/w/b.rs","added":0,"removed":0,"kind":"deleted"}]}"#,
        ),
        Some("hunk lines do not match at line 7"),
    )];
    let assembly = assemble_unbounded(&history, "继续");
    let content = assembly
        .request
        .items
        .iter()
        .find_map(|item| match item {
            ModelItem::ToolResult {
                content, is_error, ..
            } => Some((content.clone(), *is_error)),
            _ => None,
        })
        .expect("失败结果必须回放");
    assert!(content.1, "失败结果必须标记 is_error");
    assert!(
        content.0.contains("hunk lines do not match"),
        "错误原因先出现：{}",
        content.0
    );
    assert!(
        content.0.contains("partially applied: 2 file(s)"),
        "跨轮必须知道部分成功：{}",
        content.0
    );
    assert!(
        content.0.contains("not a rollback"),
        "必须明确不是回滚：{}",
        content.0
    );
    for path in ["/w/a.rs", "/w/b.rs"] {
        assert!(
            content.0.contains(path),
            "已改文件必须逐个列出：{}",
            content.0
        );
    }
    assert!(
        content.0.contains("updated /w/a.rs (+2 -1)"),
        "变更行形状与同轮一致：{}",
        content.0
    );
    assert!(
        content.0.contains("deleted /w/b.rs (+0 -0)"),
        "删除也要列出：{}",
        content.0
    );
    assert_no_orphan_results(&assembly.request.items);
}

/// P1-3：失败调用没有已提交变更时仍只回放错误原因（不编造 partial 摘要）。
#[test]
fn failed_call_without_changes_replays_error_only() {
    for result_json in [
        // 没有 `changes` 字段。
        Some(r#"{"tool":"error","code":"not_found","message":"missing"}"#),
        // `changes` 为空数组。
        Some(r#"{"tool":"error","code":"not_found","message":"missing","changes":[]}"#),
        // 结果体非法：退回错误原因。
        Some("{not json"),
        None,
    ] {
        let mut history = vec![
            message("u", 0, 0, "user", "complete", "读"),
            message("a", 1, 0, "assistant", "complete", "读一下"),
        ];
        history[1].tools = vec![tool(
            1,
            "call_read",
            "read",
            "failed",
            r#"{"path":"missing.txt"}"#,
            result_json,
            Some("file not found: missing.txt"),
        )];
        let assembly = assemble_unbounded(&history, "继续");
        let content = assembly
            .request
            .items
            .iter()
            .find_map(|item| match item {
                ModelItem::ToolResult { content, .. } => Some(content.as_str()),
                _ => None,
            })
            .expect("失败结果必须回放");
        assert_eq!(
            content, "file not found: missing.txt",
            "无变更时回放内容就是错误原因（{result_json:?}）"
        );
        assert!(!content.contains("partially applied"));
    }
}

/// 回放内容仍受 64 KiB 上限约束：partial 摘要追加后同样按上限截断。
#[test]
fn failed_patch_replay_is_still_truncated_to_the_replay_budget() {
    let big = "x".repeat(MAX_TOOL_RESULT_REPLAY_BYTES);
    let result_json = Some(format!(
        r#"{{"tool":"error","code":"patch_context_mismatch","changes":[{{"path":"{big}","added":1,"removed":1,"kind":"updated"}}]}}"#
    ));
    let mut history = vec![
        message("u", 0, 0, "user", "complete", "改"),
        message("a", 1, 0, "assistant", "complete", "改一下"),
    ];
    history[1].tools = vec![tool(
        1,
        "call_patch",
        "edit",
        "failed",
        r#"{"patch":"*** Begin Patch"}"#,
        result_json.as_deref(),
        Some("hunk lines do not match"),
    )];
    let assembly = assemble_unbounded(&history, "继续");
    let content = assembly
        .request
        .items
        .iter()
        .find_map(|item| match item {
            ModelItem::ToolResult { content, .. } => Some(content.as_str()),
            _ => None,
        })
        .expect("失败结果必须回放");
    assert!(
        content.len() <= MAX_TOOL_RESULT_REPLAY_BYTES,
        "回放内容必须 ≤ 64 KiB，实际 {}",
        content.len()
    );
    assert!(
        content.starts_with("hunk lines do not match"),
        "截断保留在尾部，错误原因与 partial 摘要仍在前面"
    );
    assert!(content.ends_with(TRUNCATED_MARKER), "截断必须显式标记");
}

/// pending / running / cancelled / interrupted 工具 → 不出现在 items（无伪造）。
#[test]
fn non_terminal_and_unexecuted_tools_are_not_replayed() {
    for status in ["pending", "running", "cancelled", "interrupted"] {
        let mut history = vec![
            message("u", 0, 0, "user", "complete", "写文件"),
            message("a", 1, 0, "assistant", "complete", "写好了"),
        ];
        history[1].tools = vec![tool(
            1,
            "call_x",
            "write",
            status,
            r#"{"path":"a.txt","content":"x"}"#,
            // 即使结果体存在（如 interrupted 补齐的 error_text），也不回放。
            None,
            Some("interrupted"),
        )];
        let assembly = assemble_unbounded(&history, "继续");
        let views = item_views(&assembly.request.items);
        assert!(
            !views
                .iter()
                .any(|item| matches!(item, ItemView::Call(..) | ItemView::Result(..))),
            "{status} 工具不得进入请求: {views:?}"
        );
        assert_eq!(
            views,
            vec![
                ItemView::User("写文件".into()),
                ItemView::Assistant("写好了".into()),
                ItemView::User("继续".into()),
            ]
        );
    }
}

/// completed 但既无结果体也无错误原因 → 无从判定结果，整对跳过（不伪造空结果）。
#[test]
fn completed_tool_without_any_result_body_is_skipped() {
    let mut history = vec![
        message("u", 0, 0, "user", "complete", "问"),
        message("a", 1, 0, "assistant", "complete", "答"),
    ];
    history[1].tools = vec![tool(
        1,
        "call_x",
        "read",
        "completed",
        r#"{"path":"a.rs"}"#,
        None,
        None,
    )];
    let assembly = assemble_unbounded(&history, "继续");
    assert!(
        !item_views(&assembly.request.items)
            .iter()
            .any(|item| matches!(item, ItemView::Call(..) | ItemView::Result(..)))
    );
}

/// 非法 `arguments_json` → 跳过该对，其余工具与正文正常。
#[test]
fn invalid_arguments_skip_only_that_pair() {
    let mut history = vec![
        message("u", 0, 0, "user", "complete", "两个工具"),
        message("a", 1, 0, "assistant", "complete", "先读再搜"),
    ];
    history[1].tools = vec![
        tool(
            1,
            "call_bad",
            "read",
            "completed",
            "{not json",
            Some(r#"{"tool":"read","content":"x"}"#),
            None,
        ),
        // 合法 JSON 但不是对象（数组）同样不可回放。
        tool(
            2,
            "call_array",
            "read",
            "completed",
            "[1,2,3]",
            Some(r#"{"tool":"read","content":"y"}"#),
            None,
        ),
        tool(
            3,
            "call_ok",
            "search",
            "completed",
            r#"{"pattern":"fn main"}"#,
            Some(r#"{"tool":"search","matches":[]}"#),
            None,
        ),
    ];
    let assembly = assemble_unbounded(&history, "继续");
    assert_eq!(
        item_views(&assembly.request.items),
        vec![
            ItemView::User("两个工具".into()),
            ItemView::Assistant("先读再搜".into()),
            ItemView::Call("call_ok".into(), "search".into()),
            ItemView::Result("call_ok".into(), false),
            ItemView::User("继续".into()),
        ],
        "只跳过非法参数的那一对，其余正常"
    );
    assert_no_orphan_results(&assembly.request.items);
}

/// 跨 provider 的历史只回放规范化字段（call_id / name / input / result），
/// 不带 reasoning 原文与供应商 metadata。
#[test]
fn cross_provider_history_replays_normalized_fields_only() {
    let mut history = vec![
        message("u", 0, 0, "user", "complete", "读文件"),
        message("a", 1, 0, "assistant", "complete", "我读一下"),
    ];
    // 旧 provider 的思考块带签名；目标 provider 不同。
    history[1].parts = vec![
        {
            let mut thought = part(0, "reasoning", "旧 provider 的思考原文");
            thought.provider_metadata_json = Some(r#"{"signature":"sig-abc"}"#.into());
            thought.completed_at_ms = Some(5);
            thought
        },
        {
            let mut text = part(1, "text", "我读一下");
            text.provider_metadata_json = Some(r#"{"signature":"sig-text"}"#.into());
            text
        },
    ];
    history[1].model_provider = Some("anthropic".into());
    history[1].tools = vec![tool(
        2,
        "toolu_1",
        "read",
        "completed",
        r#"{"path":"a.rs"}"#,
        Some(r#"{"tool":"read","content":"1: x"}"#),
        None,
    )];
    // 本轮目标 provider 是另一家。
    let assembly = assemble(&AssemblyInput {
        messages: &history,
        draft: "继续",
        model: &ModelSnapshot {
            provider: "google".into(),
            model: "gemini".into(),
            variant: None,
        },
        context_limit: None,
        output_limit: None,
        variant_params: None,
    });
    assert_eq!(
        item_views(&assembly.request.items),
        vec![
            ItemView::User("读文件".into()),
            ItemView::Assistant("我读一下".into()),
            ItemView::Call("toolu_1".into(), "read".into()),
            ItemView::Result("toolu_1".into(), false),
            ItemView::User("继续".into()),
        ]
    );
    // 序列化后不得出现旧 provider 的 metadata / 思考原文。
    let encoded = format!("{:?}", assembly.request.items);
    assert!(!encoded.contains("signature"), "不回传思考签名: {encoded}");
    assert!(
        !encoded.contains("旧 provider 的思考原文"),
        "不回放思考原文"
    );
}

/// 空 assistant 正文 + 工具：items 以调用起始（不插入空正文项），旧 turns 保留空串。
#[test]
fn tool_only_assistant_emits_call_without_empty_text_item() {
    let mut history = vec![
        message("u", 0, 0, "user", "complete", "读"),
        message("a", 1, 0, "assistant", "complete", ""),
    ];
    history[1].tools = vec![tool(
        1,
        "call_a",
        "read",
        "completed",
        r#"{"path":"a.rs"}"#,
        Some(r#"{"tool":"read","content":"x"}"#),
        None,
    )];
    let assembly = assemble_unbounded(&history, "继续");
    assert_eq!(
        item_views(&assembly.request.items),
        vec![
            ItemView::User("读".into()),
            ItemView::Call("call_a".into(), "read".into()),
            ItemView::Result("call_a".into(), false),
            ItemView::User("继续".into()),
        ]
    );
    // 旧纯文本视图仍是「空 assistant 正文」一条（与旧行为等价）。
    assert_eq!(assembly.request.turns[1].text, "");
}

/// 多个工具按 position 顺序成对回放；text 在工具之后继续输出。
#[test]
fn multiple_tools_replay_in_position_order() {
    let mut history = vec![
        message("u", 0, 0, "user", "complete", "写再读"),
        message("a", 1, 0, "assistant", "complete", "开始"),
    ];
    history[1].parts = vec![part(0, "text", "开始"), part(3, "text", "写完了")];
    history[1].tools = vec![
        tool(
            1,
            "call_w",
            "write",
            "completed",
            r#"{"path":"a.txt","content":"x"}"#,
            Some(r#"{"tool":"write","path":"a.txt"}"#),
            None,
        ),
        tool(
            2,
            "call_r",
            "read",
            "completed",
            r#"{"path":"a.txt"}"#,
            Some(r#"{"tool":"read","content":"1: x"}"#),
            None,
        ),
    ];
    let assembly = assemble_unbounded(&history, "继续");
    assert_eq!(
        item_views(&assembly.request.items),
        vec![
            ItemView::User("写再读".into()),
            ItemView::Assistant("开始".into()),
            ItemView::Call("call_w".into(), "write".into()),
            ItemView::Result("call_w".into(), false),
            ItemView::Call("call_r".into(), "read".into()),
            ItemView::Result("call_r".into(), false),
            ItemView::Assistant("写完了".into()),
            ItemView::User("继续".into()),
        ]
    );
    assert_no_orphan_results(&assembly.request.items);
}

/// parts 与 tools 的 position 交错给出（DB 查询顺序乱）也按 position 合并。
#[test]
fn timeline_merges_by_position_not_by_input_order() {
    let mut assistant = message("a", 1, 0, "assistant", "complete", "");
    assistant.parts = vec![part(2, "text", "后文"), part(0, "text", "前文")];
    assistant.tools = vec![tool(
        1,
        "call_a",
        "read",
        "completed",
        r#"{"path":"a.rs"}"#,
        Some(r#"{"tool":"read","content":"x"}"#),
        None,
    )];
    let history = vec![message("u", 0, 0, "user", "complete", "问"), assistant];
    let assembly = assemble_unbounded(&history, "继续");
    assert_eq!(
        item_views(&assembly.request.items),
        vec![
            ItemView::User("问".into()),
            ItemView::Assistant("前文".into()),
            ItemView::Call("call_a".into(), "read".into()),
            ItemView::Result("call_a".into(), false),
            ItemView::Assistant("后文".into()),
            ItemView::User("继续".into()),
        ]
    );
}

/// 失败轮次里的工具不回放（即使它的结果体齐备，也不伪造历史）。
#[test]
fn tools_of_failed_turn_are_not_replayed() {
    let mut history = vec![
        message("u", 0, 0, "user", "complete", "写"),
        message("a", 1, 0, "assistant", "failed", "写到一半"),
    ];
    history[1].tools = vec![tool(
        1,
        "call_w",
        "write",
        "completed",
        r#"{"path":"a.txt","content":"x"}"#,
        Some(r#"{"tool":"write","path":"a.txt"}"#),
        None,
    )];
    let assembly = assemble_unbounded(&history, "重试");
    assert_eq!(
        item_views(&assembly.request.items),
        vec![ItemView::User("重试".into())],
        "失败轮次（含其工具）整体排除"
    );
}

/// 辅助：把 `parts`/`tools` 直接拼成 assistant 消息（本文件内部用）。
#[allow(dead_code)]
fn assistant_with(parts: Vec<ChatPart>, tools: Vec<ToolCallRecord>) -> ChatMessage {
    let mut assistant = message("a", 1, 0, "assistant", "complete", "");
    assistant.parts = parts;
    assistant.tools = tools;
    assistant
}
