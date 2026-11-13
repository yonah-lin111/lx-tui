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
            kind: "chat".to_string(),
            user_shell: None,
            subagent_reports: None,
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
            attachments: Vec::new(),
        },
        crate::db::chat::ChatMessage {
            external_id: "a1".into(),
            sequence: 1,
            turn_index: 0,
            kind: "chat".to_string(),
            user_shell: None,
            subagent_reports: None,
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
            attachments: Vec::new(),
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

/// `/resume` 绑定会话模型后记入最近使用，但不改 `defaultModel`。
#[test]
fn session_loaded_records_recent_model_without_touching_default() {
    let mut state = ready_state("x");
    state.models.settings.recent_models.clear();
    state.models.settings.default_model = None;
    state.effects.clear();
    let pane = focus_pane(&state);
    let selection = ModelSelection {
        provider: "p".into(),
        model: "m".into(),
        variant: Some("high".into()),
    };
    assert!(on_session_loaded(
        &mut state,
        pane,
        "sess-recent",
        &[],
        Some(selection.clone())
    ));
    assert_eq!(
        state.models.settings.recent_models.first(),
        Some(&selection),
        "恢复会话模型进入最近使用队首"
    );
    assert_eq!(
        state.workspaces[0].lx_model,
        Some(selection.clone()),
        "恢复会话模型更新本工作区记忆"
    );
    assert_eq!(
        state.models.settings.default_model, None,
        "defaultModel 不被 /resume 改写（测试前也不是同一值）"
    );
    assert!(
        state.effects.iter().any(|effect| matches!(
            effect,
            crate::app::effects::Effect::SaveModelChoice {
                default_model: None,
                recent: Some(recent),
            } if recent.first() == Some(&selection)
        )),
        "只落盘 recentModels：{:?}",
        state.effects
    );
}

/// 同一 session 被两个窗格打开时，模型选择同步到另一窗格。
#[test]
fn session_model_syncs_to_other_panes_viewing_same_session() {
    let mut state = ready_state("x");
    let first = focus_pane(&state);
    let second = state
        .active_tab_mut()
        .split_pane(first, ratatui::layout::Direction::Horizontal)
        .expect("split");
    let old = ModelSelection {
        provider: "p".into(),
        model: "m".into(),
        variant: None,
    };
    let next = ModelSelection {
        provider: "p".into(),
        model: "m2".into(),
        variant: None,
    };
    for pane in [first, second] {
        let target = state.pane_mut_anywhere(pane).expect("pane");
        target.lx.chat.session_id = Some("sess-sync".into());
        target.lx.chat.selection = Some(old.clone());
    }
    crate::app::update::sync_session_selection(&mut state, first, "sess-sync", &next);
    assert_eq!(
        state
            .pane_anywhere(second)
            .expect("second")
            .lx
            .chat
            .selection,
        Some(next),
        "同会话其它窗格必须跟随发送/恢复后的模型"
    );
}

/// `/resume` 的会话模型已失效时不得污染 recent / 工作区记忆。
#[test]
fn session_loaded_skips_invalid_model_in_recent() {
    let mut state = ready_state("x");
    state.models.settings.recent_models.clear();
    state.models.settings.default_model = None;
    state.effects.clear();
    let pane = focus_pane(&state);
    let invalid = ModelSelection {
        provider: "deleted".into(),
        model: "gone".into(),
        variant: None,
    };
    assert!(on_session_loaded(
        &mut state,
        pane,
        "sess-invalid",
        &[],
        Some(invalid)
    ));
    assert!(
        state.models.settings.recent_models.is_empty(),
        "失效会话模型不得进入 recent"
    );
    assert!(
        state
            .effects
            .iter()
            .all(|effect| !matches!(effect, crate::app::effects::Effect::SaveModelChoice { .. })),
        "失效模型不触发落盘"
    );
}
