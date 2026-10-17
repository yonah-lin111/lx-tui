//! 整轮预算裁剪、`shrink_once`、token 估算口径与单条结果回放上限。

use super::*;

/// 构造一轮「正文 + 大工具结果」的历史（工具结果体大小可调）。
fn turn_with_result(
    turn_index: i64,
    sequence_base: i64,
    user: &str,
    result_body: &str,
) -> Vec<ChatMessage> {
    let mut user_message = message(
        &format!("u{turn_index}"),
        sequence_base,
        turn_index,
        "user",
        "complete",
        user,
    );
    user_message.parts = vec![part(0, "text", user)];
    let mut assistant = message(
        &format!("a{turn_index}"),
        sequence_base + 1,
        turn_index,
        "assistant",
        "complete",
        "",
    );
    assistant.parts = vec![part(0, "text", "已读取")];
    assistant.tools = vec![tool(
        1,
        &format!("call_{turn_index}"),
        "read",
        "completed",
        r#"{"path":"big.txt"}"#,
        Some(result_body),
        None,
    )];
    vec![user_message, assistant]
}

/// 多轮裁剪：预算只够最近一轮时，最老一轮的 call/result 全部消失（无 orphan）。
#[test]
fn trimming_drops_whole_oldest_turn_including_tools() {
    // 每轮正文与结果都足够大，使预算只能容下最近一轮（约 100 token/轮）。
    let first_question = "第一轮提问".repeat(50);
    let second_question = "第二轮提问".repeat(50);
    let old_result = format!(r#"{{"tool":"read","content":"{}"}}"#, "o".repeat(400));
    let new_result = format!(r#"{{"tool":"read","content":"{}"}}"#, "n".repeat(400));
    let mut history = turn_with_result(0, 0, &first_question, &old_result);
    history.extend(turn_with_result(1, 2, &second_question, &new_result));
    // 预算 ≈ 300 token：只容得下最近一轮 + 草稿。
    let assembly = assemble_with_limit(&history, "第三问", 556, 0);
    assert_eq!(assembly.omitted_turns, 1, "只省略最老一轮");
    let views = item_views(&assembly.request.items);
    assert_eq!(
        views,
        vec![
            ItemView::User(second_question.clone()),
            ItemView::Assistant("已读取".into()),
            ItemView::Call("call_1".into(), "read".into()),
            ItemView::Result("call_1".into(), false),
            ItemView::User("第三问".into()),
        ],
        "最老一轮的 call/result 一并消失"
    );
    assert_no_orphan_results(&assembly.request.items);
    assert!(
        !assembly
            .request
            .items
            .iter()
            .any(|item| matches!(item, ModelItem::AssistantToolCall { call_id, .. } if call_id == "call_0")),
        "被丢弃轮次的调用不得残留"
    );
}

/// `shrink_once` 后同样无 orphan：两个视图的轮次边界一致。
#[test]
fn shrink_once_removes_oldest_turn_in_both_views() {
    let mut history = turn_with_result(0, 0, "第一问", r#"{"tool":"read","content":"old"}"#);
    history.extend(turn_with_result(
        1,
        2,
        "第二问",
        r#"{"tool":"read","content":"new"}"#,
    ));
    let assembly = assemble_unbounded(&history, "第三问");
    assert_eq!(assembly.omitted_turns, 0, "不限预算时全部纳入");
    let shrunk = shrink_once(&assembly).expect("shrink possible");
    // turns：去掉了最老一轮的两条。
    let turns: Vec<(&str, &str)> = shrunk
        .turns
        .iter()
        .map(|turn| (turn.role.as_str(), turn.text.as_str()))
        .collect();
    assert_eq!(
        turns,
        vec![
            ("user", "第二问"),
            ("assistant", "已读取"),
            ("user", "第三问")
        ]
    );
    // items：最老一轮的 UserText/AssistantText/call/result 一起消失。
    assert_eq!(
        item_views(&shrunk.items),
        vec![
            ItemView::User("第二问".into()),
            ItemView::Assistant("已读取".into()),
            ItemView::Call("call_1".into(), "read".into()),
            ItemView::Result("call_1".into(), false),
            ItemView::User("第三问".into()),
        ]
    );
    assert_no_orphan_results(&shrunk.items);
    // 再收缩一次：只剩当前消息，无法继续（调用方报错）。
    let single = Assembly {
        request: shrunk,
        omitted_turns: 0,
        draft_exceeds: false,
    };
    let once = shrink_once(&single).expect("还剩一轮完整轮次可收缩");
    assert_eq!(
        item_views(&once.items),
        vec![ItemView::User("第三问".into())],
        "第二次收缩后只剩当前消息"
    );
    let last = Assembly {
        request: once,
        omitted_turns: 0,
        draft_exceeds: false,
    };
    assert!(shrink_once(&last).is_none(), "只剩当前消息时不可收缩");
}

/// 纯文本历史收缩：与旧行为一致（去掉最老一轮的两条 turn）。
#[test]
fn shrink_once_plain_text_history_matches_legacy() {
    let history = two_complete_turns();
    let assembly = assemble_unbounded(&history, "第三问");
    let shrunk = shrink_once(&assembly).expect("shrink possible");
    let roles: Vec<&str> = shrunk.turns.iter().map(|turn| turn.role.as_str()).collect();
    assert_eq!(roles, vec!["user", "assistant", "user"]);
    assert_eq!(
        shrunk.turns.last().map(|turn| turn.text.as_str()),
        Some("第三问")
    );
    assert_eq!(
        item_views(&shrunk.items),
        vec![
            ItemView::User("第二问".into()),
            ItemView::Assistant("第二答".into()),
            ItemView::User("第三问".into()),
        ]
    );
}

/// 只剩当前用户消息时 `shrink_once` 返回 None（结构退化也拒绝收缩）。
#[test]
fn shrink_once_refuses_without_complete_turn() {
    let assembly = Assembly {
        request: ChatRequest {
            turns: vec![crate::chat::events::ChatTurn {
                role: "user".into(),
                text: "only".into(),
                reasoning_metadata: None,
            }],
            items: vec![ModelItem::UserText("only".into())],
            model: model(),
            variant_params: None,
        },
        omitted_turns: 0,
        draft_exceeds: false,
    };
    assert!(shrink_once(&assembly).is_none());
}

/// token 估算包含工具参数与结果：工具内容把轮次推过预算 → 更早裁剪。
#[test]
fn token_estimate_includes_tool_arguments_and_results() {
    let mut plain = turn_with_result(0, 0, "问", "");
    // 去掉工具（纯正文轮次）作为基线。
    plain[1].tools.clear();
    let mut with_tool = plain.clone();
    // 同样的正文，但多加一条 8 KiB 的工具参数 + 结果。
    let big_args = format!(r#"{{"path":"{}"}}"#, "a".repeat(8 * 1024));
    let big_result = "x".repeat(8 * 1024);
    with_tool[1].parts = vec![part(0, "text", "已读取")];
    with_tool[1].tools = vec![tool(
        1,
        "call_big",
        "read",
        "completed",
        &big_args,
        Some(&big_result),
        None,
    )];
    // 预算刚好容得下纯正文轮次，但容不下带工具内容的同一轮。
    let plain_assembly = assemble_with_limit(&plain, "继续", 400, 0);
    let tool_assembly = assemble_with_limit(&with_tool, "继续", 400, 0);
    assert_eq!(plain_assembly.omitted_turns, 0, "纯正文轮次在预算内");
    assert_eq!(
        tool_assembly.omitted_turns, 1,
        "工具参数与结果计入估算后，同一轮被整轮裁掉"
    );
    assert_eq!(
        item_views(&tool_assembly.request.items),
        vec![ItemView::User("继续".into())]
    );
}

/// 预算按整轮计：即便只差一点也整轮丢弃，不会出现「留下调用、丢掉结果」。
#[test]
fn budget_never_splits_a_turn() {
    let mut history = turn_with_result(0, 0, "第一问", r#"{"tool":"read","content":"old"}"#);
    history.extend(turn_with_result(
        1,
        2,
        "第二问",
        r#"{"tool":"read","content":"new"}"#,
    ));
    // 极小预算：两轮都放不下，只剩当前草稿。
    let assembly = assemble_with_limit(&history, "第三问", 120, 0);
    assert_eq!(assembly.omitted_turns, 2);
    assert_eq!(
        item_views(&assembly.request.items),
        vec![ItemView::User("第三问".into())]
    );
    assert_no_orphan_results(&assembly.request.items);
}

/// 单条结果超 64 KiB → 截断并带显式 `[truncated]` 标记；估算按截断后的内容。
#[test]
fn oversized_single_result_is_truncated_with_marker() {
    let body = "y".repeat(MAX_TOOL_RESULT_REPLAY_BYTES + 4_096);
    let history = turn_with_result(0, 0, "读大文件", &body);
    let assembly = assemble_unbounded(&history, "继续");
    let content = assembly
        .request
        .items
        .iter()
        .find_map(|item| match item {
            ModelItem::ToolResult { content, .. } => Some(content.clone()),
            _ => None,
        })
        .expect("result present");
    assert!(
        content.len() <= MAX_TOOL_RESULT_REPLAY_BYTES,
        "回放结果不得超过上限: {}",
        content.len()
    );
    assert!(
        content.ends_with("[truncated]"),
        "截断必须显式标记: {:?}",
        &content[content.len().saturating_sub(32)..]
    );
    assert!(content.starts_with("yyy"), "保留前缀内容");
}

/// 恰好等于上限时不截断（边界不引入噪声标记）。
#[test]
fn result_at_the_limit_is_not_truncated() {
    let body = "z".repeat(MAX_TOOL_RESULT_REPLAY_BYTES);
    let truncated = truncate_result_for_replay(&body);
    assert_eq!(truncated.len(), body.len());
    assert!(!truncated.contains("[truncated]"));
}

/// 截断按 UTF-8 字符边界回退（不产生非法 UTF-8）。
#[test]
fn truncation_respects_utf8_boundaries() {
    // 三字节字符铺满上限，必然落在字符中间。
    let body = "中".repeat(MAX_TOOL_RESULT_REPLAY_BYTES);
    let truncated = truncate_result_for_replay(&body);
    assert!(truncated.len() <= MAX_TOOL_RESULT_REPLAY_BYTES);
    assert!(truncated.ends_with("[truncated]"));
    // 前缀仍必须是合法 UTF-8（能正常按字符迭代）。
    let prefix = truncated.trim_end_matches("\n…[truncated]");
    assert!(prefix.chars().all(|ch| ch == '中'));
}

/// 截断标记不会覆盖工具结果里已有的 `truncated` 语义（JSON 原样保留）。
#[test]
fn truncation_keeps_explicit_truncated_flag_from_tool_layer() {
    let mut body = String::from(r#"{"tool":"read","truncated":true,"content":""#);
    body.push_str(&"q".repeat(MAX_TOOL_RESULT_REPLAY_BYTES));
    body.push_str(r#""}"#);
    let truncated = truncate_result_for_replay(&body);
    assert!(truncated.len() <= MAX_TOOL_RESULT_REPLAY_BYTES);
    assert!(truncated.starts_with(r#"{"tool":"read","truncated":true"#));
    assert!(truncated.ends_with("[truncated]"));
}
