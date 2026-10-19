//! 会话加载/模型选择状态测试：上下文用量恢复与默认思考等级。

use super::*;

/// 回归：打开历史会话时上下文占用不再为空——按 codex 口径从最近一条带用量的
/// assistant 轮次恢复（输入 + 输出），并回填本轮统计（耗时/模型名）。
#[test]
fn session_loaded_restores_context_usage_and_stats() {
    let mut state = ready_state("x");
    let pane = focus_pane(&state);
    let messages = vec![
        crate::db::chat::ChatMessage {
            external_id: "u1".into(),
            sequence: 0,
            turn_index: 0,
            role: "user".into(),
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
            created_at_ms: 1_000,
            completed_at_ms: None,
            parts: vec![crate::db::chat::ChatPart {
                position: 0,
                kind: "text".into(),
                content: "问".into(),
                provider_metadata_json: None,
                started_at_ms: None,
                ended_at_ms: None,
                completed_at_ms: None,
            }],
            tools: Vec::new(),
        },
        crate::db::chat::ChatMessage {
            external_id: "a1".into(),
            sequence: 1,
            turn_index: 0,
            role: "assistant".into(),
            status: "complete".into(),
            model_provider: Some("p".into()),
            model_id: Some("m".into()),
            variant: None,
            stop_reason: Some("complete".into()),
            error_code: None,
            input_tokens: Some(2_000),
            output_tokens: Some(500),
            cache_read_tokens: Some(1_500),
            cache_write_tokens: None,
            created_at_ms: 1_000,
            completed_at_ms: Some(3_500),
            parts: vec![crate::db::chat::ChatPart {
                position: 0,
                kind: "text".into(),
                content: "答".into(),
                provider_metadata_json: None,
                started_at_ms: None,
                ended_at_ms: None,
                completed_at_ms: None,
            }],
            tools: Vec::new(),
        },
    ];
    assert!(on_session_loaded(
        &mut state,
        pane,
        "sess-ctx",
        &messages,
        Some(ModelSelection {
            provider: "p".into(),
            model: "m".into(),
            variant: Some("high".into()),
        })
    ));
    let target = state.pane_anywhere(pane).unwrap();
    let chat = &target.lx.chat;
    assert_eq!(chat.context_tokens, Some(2_500), "上下文占用 = 输入 + 输出");
    assert_eq!(chat.status.input_tokens, Some(2_500), "状态行分子同步恢复");
    assert_eq!(
        chat.status.context_limit,
        Some(100_000),
        "上下文上限同步恢复"
    );
    assert_eq!(
        chat.messages[1].model_label.as_deref(),
        Some("M"),
        "模型展示名"
    );
    assert_eq!(chat.messages[1].input_tokens, Some(2_000));
    assert_eq!(chat.messages[1].output_tokens, Some(500));
    assert_eq!(chat.messages[1].cache_read_tokens, Some(1_500));
    assert_eq!(chat.messages[1].duration_ms, Some(2_500), "本轮耗时");
    assert!(
        chat.thinking_toggled.is_empty(),
        "历史加载清空显式 thinking 切换"
    );
}

/// 模型配置了默认思考等级时，未显式选择的窗格选择也带上该等级（variant 参数）。
#[test]
fn effective_selection_applies_model_default_variant() {
    let models = models_with(Some("high"));
    let chat = LxChat::new();
    let selection = chat
        .effective_selection(&models)
        .expect("default selection");
    assert_eq!(
        selection.variant.as_deref(),
        Some("high"),
        "回落模型默认等级"
    );
    // 端到端：规范化后的选择能取到思考参数（→ 请求体 `reasoning_effort`）。
    assert_eq!(
        models.variant_params_of(&selection),
        Some(serde_json::json!({"reasoningEffort": "high"})),
        "默认等级参数可用"
    );
    // 显式失效的 variant 也回落默认。
    let mut chat = LxChat::new();
    chat.selection = Some(ModelSelection {
        provider: "p".into(),
        model: "m".into(),
        variant: Some("gone".into()),
    });
    let selection = chat.effective_selection(&models).expect("pane selection");
    assert_eq!(selection.variant.as_deref(), Some("high"));
}
