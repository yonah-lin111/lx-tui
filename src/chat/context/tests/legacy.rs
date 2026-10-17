//! 旧纯文本会话等价性与完整轮次筛选（与工具化之前的行为逐字节一致）。

use super::*;

/// 旧纯文本会话（无 tools）装配结果与旧行为逐字节等价：turns 与 items 都断言。
#[test]
fn plain_text_history_matches_legacy_output_byte_for_byte() {
    let history = two_complete_turns();
    let assembly = assemble_unbounded(&history, "第三问");
    // turns：user/assistant 交替 + 末尾当前 user 消息，text 原样、metadata 恒 None。
    let turns: Vec<(&str, &str)> = assembly
        .request
        .turns
        .iter()
        .map(|turn| (turn.role.as_str(), turn.text.as_str()))
        .collect();
    assert_eq!(
        turns,
        vec![
            ("user", "第一问"),
            ("assistant", "第一答"),
            ("user", "第二问"),
            ("assistant", "第二答"),
            ("user", "第三问"),
        ]
    );
    assert!(
        assembly
            .request
            .turns
            .iter()
            .all(|turn| turn.reasoning_metadata.is_none()),
        "旧路径不回传思考元数据"
    );
    // items：与 turns 逐项对应（无工具时结构视图退化为纯文本序列）。
    assert_eq!(
        item_views(&assembly.request.items),
        vec![
            ItemView::User("第一问".into()),
            ItemView::Assistant("第一答".into()),
            ItemView::User("第二问".into()),
            ItemView::Assistant("第二答".into()),
            ItemView::User("第三问".into()),
        ]
    );
    assert_eq!(assembly.omitted_turns, 0);
    assert!(!assembly.draft_exceeds);
}

/// 只纳入 user/assistant 均 complete 的轮次；失败/中断轮次整体排除。
#[test]
fn interrupted_and_failed_turns_are_excluded() {
    let mut history = two_complete_turns();
    history[3].status = "failed".into();
    let assembly = assemble_unbounded(&history, "重试");
    let texts: Vec<&str> = assembly
        .request
        .turns
        .iter()
        .map(|turn| turn.text.as_str())
        .collect();
    assert_eq!(texts, vec!["第一问", "第一答", "重试"]);
    assert_eq!(
        item_views(&assembly.request.items),
        vec![
            ItemView::User("第一问".into()),
            ItemView::Assistant("第一答".into()),
            ItemView::User("重试".into()),
        ],
        "失败轮次不进入结构化视图"
    );
}

/// assistant 中断（正文已部分落库）同样整轮排除，不把 partial 当历史。
#[test]
fn interrupted_assistant_turn_is_excluded_from_both_views() {
    let mut history = two_complete_turns();
    history[3].status = "interrupted".into();
    let assembly = assemble_unbounded(&history, "继续");
    assert_eq!(assembly.request.turns.len(), 3, "user/assistant + 草稿");
    assert_eq!(assembly.request.items.len(), 3);
    assert!(
        !assembly
            .request
            .turns
            .iter()
            .any(|turn| turn.text == "第二答")
    );
}

/// 无上限时不裁剪（按完整历史发送）。
#[test]
fn no_limit_sends_full_history() {
    let history = two_complete_turns();
    let assembly = assemble_unbounded(&history, "q");
    assert_eq!(assembly.omitted_turns, 0);
    assert_eq!(assembly.request.turns.len(), 5);
    assert_eq!(assembly.request.items.len(), 5);
}

/// 单条当前消息超限只标记报错，绝不截断用户输入。
#[test]
fn oversized_draft_is_flagged_not_truncated() {
    let history = two_complete_turns();
    let draft = "超长".repeat(10_000);
    let assembly = assemble_with_limit(&history, &draft, 100, 50);
    assert!(assembly.draft_exceeds, "单条消息超限只报错");
    assert!(assembly.request.turns.iter().any(|turn| turn.text == draft));
    assert!(
        assembly
            .request
            .items
            .iter()
            .any(|item| matches!(item, ModelItem::UserText(text) if text == &draft))
    );
}

/// 空历史只有当前草稿（两条视图一致）。
#[test]
fn empty_history_only_has_current_draft() {
    let assembly = assemble_unbounded(&[], "第一条");
    assert_eq!(assembly.request.turns.len(), 1);
    assert_eq!(assembly.request.turns[0].role, "user");
    assert_eq!(assembly.request.turns[0].text, "第一条");
    assert_eq!(
        item_views(&assembly.request.items),
        vec![ItemView::User("第一条".into())]
    );
}

/// 同一轮内的多个 text part 依序拼接成一条 assistant 正文（旧口径：单段拼接）。
#[test]
fn consecutive_text_parts_merge_into_one_assistant_text() {
    let mut assistant = message("a0", 1, 0, "assistant", "complete", "");
    assistant.parts = vec![part(0, "text", "前半"), part(1, "text", "后半")];
    let history = vec![message("u0", 0, 0, "user", "complete", "第一问"), assistant];
    let assembly = assemble_unbounded(&history, "下一问");
    assert_eq!(assembly.request.turns[1].text, "前半后半");
    assert_eq!(
        item_views(&assembly.request.items),
        vec![
            ItemView::User("第一问".into()),
            ItemView::Assistant("前半后半".into()),
            ItemView::User("下一问".into()),
        ],
        "相邻正文合并为一条 AssistantText"
    );
}

/// reasoning part 不回放：跨 provider 不注入旧思考原文与签名。
#[test]
fn reasoning_parts_and_metadata_are_not_replayed() {
    let mut history = two_complete_turns();
    history[1].parts[0].provider_metadata_json = Some(r#"{"signature":"sig"}"#.into());
    history[1].parts.insert(0, {
        let mut thought = part(0, "reasoning", "内部思考");
        thought.provider_metadata_json = Some(r#"{"signature":"sig"}"#.into());
        thought.completed_at_ms = Some(1);
        thought
    });
    // 正文 part 的 position 需要让位给新插入的 reasoning（保持唯一编号空间）。
    history[1].parts[1].position = 1;
    let assembly = assemble_unbounded(&history, "第三问");
    for turn in &assembly.request.turns {
        assert_eq!(
            turn.reasoning_metadata, None,
            "专有 thinking 元数据不跨 provider 传输"
        );
        assert_ne!(turn.text, "内部思考");
    }
    let assistant_texts: Vec<&str> = assembly
        .request
        .turns
        .iter()
        .filter(|turn| turn.role == "assistant")
        .map(|turn| turn.text.as_str())
        .collect();
    assert_eq!(assistant_texts, vec!["第一答", "第二答"]);
    assert!(
        !assembly.request.items.iter().any(|item| matches!(
            item,
            ModelItem::AssistantText(text) if text == "内部思考"
        )),
        "reasoning 不进入结构化历史"
    );
}

/// 缺 assistant 的轮次（用户消息后直接下一轮）不参与回放，也不与前轮错配。
#[test]
fn turn_without_assistant_is_skipped() {
    let history = vec![
        message("u0", 0, 0, "user", "complete", "孤立提问"),
        message("u1", 1, 1, "user", "complete", "第二问"),
        message("a1", 2, 1, "assistant", "complete", "第二答"),
    ];
    let assembly = assemble_unbounded(&history, "第三问");
    assert_eq!(
        item_views(&assembly.request.items),
        vec![
            ItemView::User("第二问".into()),
            ItemView::Assistant("第二答".into()),
            ItemView::User("第三问".into()),
        ]
    );
}
