//! 上下文装配测试：完整轮次选择、窗口裁剪、partial 排除与草稿保留。

use super::*;
use crate::db::chat::ChatMessage;

fn message(
    external_id: &str,
    sequence: i64,
    turn_index: i64,
    role: &str,
    status: &str,
    text: &str,
) -> ChatMessage {
    ChatMessage {
        external_id: external_id.into(),
        sequence,
        turn_index,
        role: role.into(),
        status: status.into(),
        model_provider: (role == "assistant").then(|| "example".to_string()),
        model_id: (role == "assistant").then(|| "m".to_string()),
        variant: None,
        stop_reason: None,
        error_code: None,
        input_tokens: None,
        output_tokens: None,
        cache_read_tokens: None,
        cache_write_tokens: None,
        created_at_ms: 0,
        completed_at_ms: None,
        parts: vec![crate::db::chat::ChatPart {
            position: 0,
            kind: "text".into(),
            content: text.into(),
            provider_metadata_json: None,
            started_at_ms: None,
            ended_at_ms: None,
            completed_at_ms: None,
        }],
        tools: Vec::new(),
    }
}

fn model() -> crate::db::chat::ModelSnapshot {
    crate::db::chat::ModelSnapshot {
        provider: "example".into(),
        model: "m".into(),
        variant: None,
    }
}

/// 两轮完整历史。
fn two_complete_turns() -> Vec<ChatMessage> {
    vec![
        message("u0", 0, 0, "user", "complete", "第一问"),
        message("a0", 1, 0, "assistant", "complete", "第一答"),
        message("u1", 2, 1, "user", "complete", "第二问"),
        message("a1", 3, 1, "assistant", "complete", "第二答"),
    ]
}

#[test]
fn only_complete_turns_enter_context_in_order() {
    let history = two_complete_turns();
    let assembly = assemble(&AssemblyInput {
        messages: &history,
        draft: "第三问",
        model: &model(),
        context_limit: Some(100_000),
        output_limit: Some(4_096),
        variant_params: None,
    });
    // 原顺序回传，末尾并入当前草稿。
    let roles: Vec<&str> = assembly
        .request
        .turns
        .iter()
        .map(|turn| turn.role.as_str())
        .collect();
    assert_eq!(
        roles,
        vec!["user", "assistant", "user", "assistant", "user"]
    );
    assert_eq!(
        assembly.request.turns.last().map(|turn| turn.text.as_str()),
        Some("第三问")
    );
    assert_eq!(assembly.omitted_turns, 0);
    assert!(!assembly.draft_exceeds);
}

#[test]
fn interrupted_and_failed_turns_are_excluded() {
    let mut history = two_complete_turns();
    // 第二轮失败：partial 仅供展示、不进上下文。
    history[3].status = "failed".into();
    let assembly = assemble(&AssemblyInput {
        messages: &history,
        draft: "重试",
        model: &model(),
        context_limit: Some(100_000),
        output_limit: Some(4_096),
        variant_params: None,
    });
    // 只有第一轮完整轮次进入。
    let roles: Vec<&str> = assembly
        .request
        .turns
        .iter()
        .map(|turn| turn.role.as_str())
        .collect();
    assert_eq!(roles, vec!["user", "assistant", "user"]);
    let texts: Vec<&str> = assembly
        .request
        .turns
        .iter()
        .map(|turn| turn.text.as_str())
        .collect();
    assert_eq!(texts, vec!["第一问", "第一答", "重试"]);
}

#[test]
fn context_limit_drops_oldest_turn() {
    let history = two_complete_turns();
    // 预算仅容一轮多一点：更老的完整轮被省略。
    let assembly = assemble(&AssemblyInput {
        messages: &history,
        draft: "长问题".repeat(64).as_str(),
        model: &model(),
        context_limit: Some(200),
        output_limit: Some(50),
        variant_params: None,
    });
    assert!(
        assembly.omitted_turns >= 1,
        "省略最老轮：{}",
        assembly.omitted_turns
    );
    // 至少保留当前草稿。
    assert_eq!(
        assembly.request.turns.last().map(|turn| turn.text.as_str()),
        Some(
            assembly
                .request
                .turns
                .last()
                .expect("nonempty")
                .text
                .as_str()
        )
    );
    assert!(!assembly.request.turns.is_empty());
}

#[test]
fn no_limit_sends_full_history() {
    let history = two_complete_turns();
    let assembly = assemble(&AssemblyInput {
        messages: &history,
        draft: "q",
        model: &model(),
        context_limit: None,
        output_limit: None,
        variant_params: None,
    });
    assert_eq!(assembly.omitted_turns, 0);
    assert_eq!(assembly.request.turns.len(), 5);
}

#[test]
fn oversized_draft_is_flagged_not_truncated() {
    let history = two_complete_turns();
    let draft = "超长".repeat(10_000);
    let assembly = assemble(&AssemblyInput {
        messages: &history,
        draft: &draft,
        model: &model(),
        context_limit: Some(100),
        output_limit: Some(50),
        variant_params: None,
    });
    assert!(assembly.draft_exceeds, "单条消息超限只报错");
    // 用户输入不被截断。
    assert!(assembly.request.turns.iter().any(|turn| turn.text == draft));
}

#[test]
fn shrink_once_removes_oldest_turn() {
    let history = two_complete_turns();
    let assembly = assemble(&AssemblyInput {
        messages: &history,
        draft: "第三问",
        model: &model(),
        context_limit: Some(100_000),
        output_limit: Some(4_096),
        variant_params: None,
    });
    let shrunk = shrink_once(&assembly).expect("shrink possible");
    // 去掉最老一轮（2 条），保留后一轮 + 草稿。
    let roles: Vec<&str> = shrunk.iter().map(|turn| turn.role.as_str()).collect();
    assert_eq!(roles, vec!["user", "assistant", "user"]);
    assert_eq!(shrunk.last().map(|turn| turn.text.as_str()), Some("第三问"));
    // 只剩当前消息时无法收缩。
    let single = Assembly {
        request: ChatRequest {
            turns: vec![crate::chat::events::ChatTurn {
                role: "user".into(),
                text: "only".into(),
                reasoning_metadata: None,
            }],
            model: model(),
            variant_params: None,
        },
        omitted_turns: 0,
        draft_exceeds: false,
    };
    assert!(shrink_once(&single).is_none());
}

#[test]
fn reasoning_metadata_not_carried_across_providers() {
    let mut history = two_complete_turns();
    // 第一轮 assistant 带供应商元数据（如 signature）。
    history[1].parts[0].provider_metadata_json = Some(r#"{"signature":"sig"}"#.into());
    // 第一轮 assistant 也有 reasoning part。
    history[1].parts.push(crate::db::chat::ChatPart {
        position: 1,
        kind: "reasoning".into(),
        content: "内部思考".into(),
        provider_metadata_json: Some(r#"{"signature":"sig"}"#.into()),
        started_at_ms: None,
        ended_at_ms: None,
        completed_at_ms: None,
    });
    let assembly = assemble(&AssemblyInput {
        messages: &history,
        draft: "第三问",
        model: &model(),
        context_limit: None,
        output_limit: None,
        variant_params: None,
    });
    // 回传的 assistant 只含正文 text，不含 reasoning part；metadata 为 None。
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
}

#[test]
fn empty_history_only_has_current_draft() {
    let assembly = assemble(&AssemblyInput {
        messages: &[],
        draft: "第一条",
        model: &model(),
        context_limit: None,
        output_limit: None,
        variant_params: None,
    });
    assert_eq!(assembly.request.turns.len(), 1);
    assert_eq!(assembly.request.turns[0].role, "user");
    assert_eq!(assembly.request.turns[0].text, "第一条");
}
