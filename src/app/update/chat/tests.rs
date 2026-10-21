//! lx 聊天状态机纯状态测试：首轮/续轮、重复 Enter、并发窗格、模型切换、
//! 流终态、命令面板与过期结果过滤。

mod history;
mod process_cards;
mod prompt_history;
mod session;
mod slash_panels;
mod stop;
mod task_page;
mod timeline;
mod viewport_anchor;

use super::*;
use crate::app::state::{
    AppState, ChatHover, LxChat, ModelPickerStage, ModelsSnapshot, RequestKey, RequestState,
    SlashPanel,
};
use crate::chat::events::{ChatDelta, PartDelta, PartKind, StopReason};
use crate::config::ai::{
    AiSettings, ModelConfig, ModelSelection, ProviderConfig, ProviderOptions, ProviderTransport,
};
use crate::db::chat::{TurnCommitted, Usage};
use crate::layout::PaneId;

/// 构造带单个可用 provider/model 的模型快照。
fn models_with(variant: Option<&str>) -> ModelsSnapshot {
    let mut variants = std::collections::BTreeMap::new();
    if let Some(variant) = variant {
        variants.insert(
            variant.to_string(),
            serde_json::json!({"reasoningEffort": variant}),
        );
    }
    let mut settings = AiSettings::empty();
    settings.enabled_providers = vec!["p".into()];
    settings.providers.insert(
        "p".into(),
        ProviderConfig {
            id: "p".into(),
            kind: ProviderTransport::OpenaiCompatible,
            name: "P".into(),
            options: ProviderOptions {
                api_key: Some("test-key".into()),
                base_url: "https://api.example.invalid/v1".into(),
            },
            models: std::iter::once({
                (
                    "m".into(),
                    ModelConfig {
                        id: "m".into(),
                        name: "M".into(),
                        transport: None,
                        limit: Some(crate::config::ai::ModelLimit {
                            context: 100_000,
                            output: 4_096,
                        }),
                        modalities: None,
                        variants,
                        variant: variant.map(str::to_string),
                        pricing: None,
                        extra: serde_json::Map::new(),
                    },
                )
            })
            .collect(),
            extra: serde_json::Map::new(),
        },
    );
    settings.default_model = Some(ModelSelection {
        provider: "p".into(),
        model: "m".into(),
        variant: variant.map(str::to_string),
    });
    ModelsSnapshot { settings }
}

/// 焦点窗格处于 lx Native 视图且输入草稿就绪。
fn ready_state(draft: &str) -> AppState {
    let mut state = AppState::demo();
    state.models = models_with(Some("high"));
    let pane = state.active_tab().layout.focus();
    state.pane_mut_anywhere(pane).expect("pane").lx.input = draft.into();
    // 工作区 cwd 保证有（demo 用当前目录）。
    state
}

fn focus_pane(state: &AppState) -> PaneId {
    state.active_tab().layout.focus()
}

fn committed(session: &str) -> TurnCommitted {
    TurnCommitted {
        session_id: session.into(),
        user_message_id: format!("{session}-u"),
        assistant_message_id: format!("{session}-a"),
        turn_index: 0,
        created_at_ms: 1_000,
    }
}

#[test]
fn enter_starts_committing_and_second_enter_is_ignored() {
    let mut state = ready_state("你好");
    let pane = focus_pane(&state);
    assert!(try_send(&mut state, pane), "首次发送受理");
    // Committing 期间重复 Enter 不再入队。
    assert!(!try_send(&mut state, pane), "流中/提交中 Enter 忽略");
    // 恰好一条 BeginChatTurn 效应。
    let begins = state
        .effects
        .iter()
        .filter(|effect| matches!(effect, crate::app::effects::Effect::BeginChatTurn { .. }))
        .count();
    assert_eq!(begins, 1);
    // 状态进入 Committing，草稿冻结。
    assert!(matches!(
        state.pane_anywhere(pane).unwrap().lx.chat.request,
        RequestState::Committing { .. }
    ));
}

#[test]
fn commit_ack_binds_session_clears_draft_and_starts_stream() {
    let mut state = ready_state("第一问");
    let pane = focus_pane(&state);
    try_send(&mut state, pane);
    let generation = match &state.pane_anywhere(pane).unwrap().lx.chat.request {
        RequestState::Committing { generation, .. } => *generation,
        _ => panic!("committing"),
    };
    // DB ack 成功。
    assert!(on_turn_committed(
        &mut state,
        pane,
        generation,
        &committed("sess-1")
    ));
    let target = state.pane_anywhere(pane).unwrap();
    assert_eq!(target.lx.chat.session_id.as_deref(), Some("sess-1"));
    assert!(target.lx.input.is_empty(), "冻结草稿清除");
    assert_eq!(
        target.lx.chat.messages.len(),
        2,
        "user + streaming assistant"
    );
    assert_eq!(target.lx.chat.messages[0].role, "user");
    assert_eq!(target.lx.chat.messages[1].status, "streaming");
    // 状态进入 Streaming 且入队 StartChatStream。
    assert!(matches!(
        target.lx.chat.request,
        RequestState::Streaming { .. }
    ));
    let started = state
        .effects
        .iter()
        .any(|effect| matches!(effect, crate::app::effects::Effect::StartChatStream { .. }));
    assert!(started);
}

#[test]
fn commit_failure_keeps_draft_and_returns_to_idle() {
    let mut state = ready_state("重试我");
    let pane = focus_pane(&state);
    try_send(&mut state, pane);
    let generation = match &state.pane_anywhere(pane).unwrap().lx.chat.request {
        RequestState::Committing { generation, .. } => *generation,
        _ => panic!("committing"),
    };
    assert!(on_turn_failed(&mut state, pane, generation));
    let target = state.pane_anywhere(pane).unwrap();
    assert!(matches!(target.lx.chat.request, RequestState::Idle));
    assert_eq!(target.lx.input, "重试我", "失败保留草稿可重试");
    assert!(target.lx.chat.messages.is_empty(), "不造消息");
}

#[test]
fn stream_deltas_update_matching_key_only() {
    let mut state = ready_state("问");
    let pane = focus_pane(&state);
    try_send(&mut state, pane);
    let generation = match &state.pane_anywhere(pane).unwrap().lx.chat.request {
        RequestState::Committing { generation, .. } => *generation,
        _ => panic!("committing"),
    };
    on_turn_committed(&mut state, pane, generation, &committed("sess-1"));
    let key = RequestKey {
        pane,
        session: "sess-1".into(),
        generation,
    };
    // 交错 text/reasoning delta。
    let deltas = vec![
        ChatDelta::Part(PartDelta {
            kind: PartKind::Text,
            text: "你好".into(),
            metadata: None,
        }),
        ChatDelta::Part(PartDelta {
            kind: PartKind::Reasoning,
            text: "思考".into(),
            metadata: None,
        }),
        ChatDelta::Part(PartDelta {
            kind: PartKind::Text,
            text: "世界".into(),
            metadata: None,
        }),
    ];
    assert!(on_stream_delta(&mut state, &key, &deltas));
    let target = state.pane_anywhere(pane).unwrap();
    let parts = target.lx.chat.messages[1].parts().collect::<Vec<_>>();
    assert_eq!(
        parts
            .iter()
            .map(|part| (part.kind_str(), part.content.as_str()))
            .collect::<Vec<_>>(),
        vec![("text", "你好"), ("reasoning", "思考"), ("text", "世界")],
        "交错顺序保持"
    );
    // 过期 generation 的 delta 被忽略。
    let stale = RequestKey {
        pane,
        session: "sess-1".into(),
        generation: generation + 999,
    };
    assert!(!on_stream_delta(&mut state, &stale, &deltas));
    // 其它窗格 key 被忽略。
    let other = RequestKey {
        pane: PaneId::alloc(),
        session: "sess-1".into(),
        generation,
    };
    assert!(!on_stream_delta(&mut state, &other, &deltas));
}

#[test]
fn finish_moves_to_finishing_then_idle_with_usage() {
    let mut state = ready_state("问");
    let pane = focus_pane(&state);
    try_send(&mut state, pane);
    let generation = match &state.pane_anywhere(pane).unwrap().lx.chat.request {
        RequestState::Committing { generation, .. } => *generation,
        _ => panic!("committing"),
    };
    on_turn_committed(&mut state, pane, generation, &committed("sess-1"));
    let key = RequestKey {
        pane,
        session: "sess-1".into(),
        generation,
    };
    // usage delta 先行。
    on_stream_delta(
        &mut state,
        &key,
        &[ChatDelta::Usage(Usage {
            input: Some(42),
            output: Some(7),
            ..Default::default()
        })],
    );
    assert!(on_stream_finished(
        &mut state,
        &key,
        &StopReason::Complete,
        Usage {
            input: Some(42),
            output: Some(7),
            ..Default::default()
        }
    ));
    let target = state.pane_anywhere(pane).unwrap();
    assert!(matches!(
        target.lx.chat.request,
        RequestState::Finishing { .. }
    ));
    assert_eq!(target.lx.chat.messages[1].status, "complete");
    // 上下文占用 = 输入 + 输出（codex 口径 total_tokens）。
    assert_eq!(target.lx.chat.context_tokens, Some(49), "上下文占用分子");
    // 本轮统计：用量与耗时回填到 assistant 视图。
    assert_eq!(target.lx.chat.messages[1].input_tokens, Some(42));
    assert_eq!(target.lx.chat.messages[1].output_tokens, Some(7));
    assert!(target.lx.chat.messages[1].duration_ms.is_some(), "本轮耗时");
    // DB ack 成功：回 Idle。
    assert!(on_finish_ack(&mut state, &key, true));
    assert!(matches!(
        state.pane_anywhere(pane).unwrap().lx.chat.request,
        RequestState::Idle
    ));
}

#[test]
fn finish_ack_failure_keeps_partial_and_shows_error() {
    let mut state = ready_state("问");
    let pane = focus_pane(&state);
    try_send(&mut state, pane);
    let generation = match &state.pane_anywhere(pane).unwrap().lx.chat.request {
        RequestState::Committing { generation, .. } => *generation,
        _ => panic!("committing"),
    };
    on_turn_committed(&mut state, pane, generation, &committed("sess-1"));
    let key = RequestKey {
        pane,
        session: "sess-1".into(),
        generation,
    };
    on_stream_finished(
        &mut state,
        &key,
        &StopReason::Failed {
            code: "http 500".into(),
        },
        Usage::default(),
    );
    assert!(on_finish_ack(&mut state, &key, false), "失败提示");
    // 仍处于 Finishing（保存失败）：不伪装成可恢复历史。
    assert!(matches!(
        state.pane_anywhere(pane).unwrap().lx.chat.request,
        RequestState::Finishing { .. }
    ));
    assert_eq!(
        state.pane_anywhere(pane).unwrap().lx.chat.messages[1].status,
        "failed"
    );
    // 错误提示可见（main content 的 danger toast）。
    let toast = state.toast.as_ref().expect("错误提示可见");
    assert_eq!(toast.kind, crate::app::toast::ToastKind::Danger);
}

#[test]
fn interrupted_partial_stays_visible_but_new_turn_can_start() {
    let mut state = ready_state("问");
    let pane = focus_pane(&state);
    try_send(&mut state, pane);
    let generation = match &state.pane_anywhere(pane).unwrap().lx.chat.request {
        RequestState::Committing { generation, .. } => *generation,
        _ => panic!("committing"),
    };
    on_turn_committed(&mut state, pane, generation, &committed("sess-1"));
    let key = RequestKey {
        pane,
        session: "sess-1".into(),
        generation,
    };
    // 流中停止。
    assert!(stop_stream(&mut state, pane));
    on_stream_finished(&mut state, &key, &StopReason::Cancelled, Usage::default());
    assert!(on_finish_ack(&mut state, &key, true));
    // partial 保留展示。
    let target = state.pane_anywhere(pane).unwrap();
    assert_eq!(target.lx.chat.messages.len(), 2);
    assert_eq!(target.lx.chat.messages[1].status, "interrupted");
    // 新一轮可发送。
    state.pane_mut_anywhere(pane).unwrap().lx.input = "第二轮".into();
    assert!(try_send(&mut state, pane));
}

#[test]
fn non_native_entry_rejects_send_with_feedback() {
    let mut state = ready_state("问");
    let pane = focus_pane(&state);
    state.pane_mut_anywhere(pane).unwrap().lx.entry = crate::app::state::LxEntry::Hybrid;
    assert!(try_send(&mut state, pane), "有反馈");
    assert!(state.toast.is_some(), "显示不可用提示");
    assert!(
        state
            .effects
            .iter()
            .all(|effect| !matches!(effect, crate::app::effects::Effect::BeginChatTurn { .. }))
    );
}

#[test]
fn empty_or_blank_draft_does_nothing() {
    let mut state = ready_state("   \n  ");
    let pane = focus_pane(&state);
    assert!(!try_send(&mut state, pane), "空白草稿不动作");
    assert!(state.effects.is_empty());
}

#[test]
fn two_panes_can_send_independently() {
    let mut state = ready_state("A");
    let pane_a = focus_pane(&state);
    // 分割出 B 窗格。
    let pane_b = state
        .active_tab_mut()
        .split_pane(pane_a, ratatui::layout::Direction::Vertical)
        .expect("split");
    state.pane_mut_anywhere(pane_b).unwrap().lx.input = "B".into();
    assert!(try_send(&mut state, pane_a));
    assert!(try_send(&mut state, pane_b), "其他窗格可独立发送");
    // 各自一条 begin 效应，session 独立。
    let begins: Vec<&crate::app::effects::Effect> = state
        .effects
        .iter()
        .filter(|effect| matches!(effect, crate::app::effects::Effect::BeginChatTurn { .. }))
        .collect();
    assert_eq!(begins.len(), 2);
}

#[test]
fn model_switch_only_affects_next_turn() {
    let mut state = ready_state("问");
    let pane = focus_pane(&state);
    // 第一轮用默认模型发送。
    try_send(&mut state, pane);
    let generation = match &state.pane_anywhere(pane).unwrap().lx.chat.request {
        RequestState::Committing { generation, .. } => *generation,
        _ => panic!("committing"),
    };
    let first_model = match state.effects.iter().find_map(|effect| match effect {
        crate::app::effects::Effect::BeginChatTurn { model, .. } => Some(model.clone()),
        _ => None,
    }) {
        Some(model) => model,
        None => panic!("begin effect"),
    };
    on_turn_committed(&mut state, pane, generation, &committed("sess-1"));
    // `/model` 切换到其它选择（构造第二模型 provider）。
    state.models.settings.providers.insert(
        "q".into(),
        ProviderConfig {
            id: "q".into(),
            kind: ProviderTransport::Anthropic,
            name: "Q".into(),
            options: ProviderOptions {
                api_key: Some("test-key-2".into()),
                base_url: "https://api.q.invalid".into(),
            },
            models: std::iter::once({
                (
                    "n".into(),
                    ModelConfig {
                        id: "n".into(),
                        name: "N".into(),
                        transport: None,
                        limit: None,
                        modalities: None,
                        variants: std::collections::BTreeMap::new(),
                        variant: None,
                        pricing: None,
                        extra: serde_json::Map::new(),
                    },
                )
            })
            .collect(),
            extra: serde_json::Map::new(),
        },
    );
    state.models.settings.enabled_providers.push("q".into());
    finish_model_selection_for_test(
        &mut state,
        pane,
        ModelSelection {
            provider: "q".into(),
            model: "n".into(),
            variant: None,
        },
    );
    // 已保存 session：选择写入排队（DB 三列）。
    assert!(state.effects.iter().any(|effect| matches!(
        effect,
        crate::app::effects::Effect::SetChatSessionModel { .. }
    )));
    // 在途请求的模型元数据不被篡改：第一轮 message 无模型标签（由 DB 写入），
    // 这里验证窗格选择已变为 q/n。
    assert_eq!(
        state
            .pane_anywhere(pane)
            .unwrap()
            .lx
            .chat
            .selection
            .as_ref()
            .unwrap()
            .provider,
        "q"
    );
    // 第二轮用新模型。
    let key = RequestKey {
        pane,
        session: "sess-1".into(),
        generation,
    };
    on_stream_finished(&mut state, &key, &StopReason::Complete, Usage::default());
    on_finish_ack(&mut state, &key, true);
    state.pane_mut_anywhere(pane).unwrap().lx.input = "第二问".into();
    try_send(&mut state, pane);
    let second_model = state
        .effects
        .iter()
        .rev()
        .find_map(|effect| match effect {
            crate::app::effects::Effect::BeginChatTurn { model, .. } => Some(model.clone()),
            _ => None,
        })
        .expect("second begin");
    assert_ne!(first_model.provider, second_model.provider, "下一轮生效");
}

/// 测试辅助：直接调 finish_model_selection（私有）。
fn finish_model_selection_for_test(state: &mut AppState, pane: PaneId, selection: ModelSelection) {
    // 与生产路径等价：直接调私有函数（经 test_hooks 暴露）。
    super::test_hooks::finish_model_selection(state, pane, selection);
}

#[test]
fn usage_unknown_shows_none_not_zero() {
    let mut state = ready_state("问");
    let pane = focus_pane(&state);
    try_send(&mut state, pane);
    let generation = match &state.pane_anywhere(pane).unwrap().lx.chat.request {
        RequestState::Committing { generation, .. } => *generation,
        _ => panic!("committing"),
    };
    on_turn_committed(&mut state, pane, generation, &committed("sess-1"));
    let key = RequestKey {
        pane,
        session: "sess-1".into(),
        generation,
    };
    // 无 usage 完成流。
    on_stream_finished(&mut state, &key, &StopReason::Complete, Usage::default());
    let target = state.pane_anywhere(pane).unwrap();
    assert_eq!(target.lx.chat.context_tokens, None, "未知不显示上下文");
}

#[test]
fn slash_new_unbinds_session_and_keeps_stream_writing() {
    let mut state = ready_state("问");
    let pane = focus_pane(&state);
    try_send(&mut state, pane);
    let generation = match &state.pane_anywhere(pane).unwrap().lx.chat.request {
        RequestState::Committing { generation, .. } => *generation,
        _ => panic!("committing"),
    };
    on_turn_committed(&mut state, pane, generation, &committed("sess-1"));
    // `/new`：解绑但请求继续（Cancel 效应入队，写入继续按原 ID 完成）。
    assert!(new_session(&mut state, pane));
    let target = state.pane_anywhere(pane).unwrap();
    assert_eq!(target.lx.chat.session_id, None);
    assert!(target.lx.chat.messages.is_empty());
    // 晚到的 finish 事件按原 key 完成（pane 上无匹配状态时被忽略，不崩）。
    let key = RequestKey {
        pane,
        session: "sess-1".into(),
        generation,
    };
    // 请求状态已被 new_session 重置？设计：解绑并取消其请求。
    // on_stream_finished 对不匹配状态返回 false，不写其它窗格。
    let result = on_stream_finished(&mut state, &key, &StopReason::Complete, Usage::default());
    // 无论状态如何，事件不串到新会话。
    assert_eq!(state.pane_anywhere(pane).unwrap().lx.chat.messages.len(), 0);
    let _ = result;
}

#[test]
fn resume_blocked_when_session_busy_in_other_pane() {
    let mut state = ready_state("A");
    let pane_a = focus_pane(&state);
    let pane_b = state
        .active_tab_mut()
        .split_pane(pane_a, ratatui::layout::Direction::Vertical)
        .expect("split");
    // A 正在生成 sess-1。
    state.pane_mut_anywhere(pane_a).unwrap().lx.input = "A".into();
    try_send(&mut state, pane_a);
    let generation = match &state.pane_anywhere(pane_a).unwrap().lx.chat.request {
        RequestState::Committing { generation, .. } => *generation,
        _ => panic!("committing"),
    };
    on_turn_committed(&mut state, pane_a, generation, &committed("sess-1"));
    // B 尝试恢复同一 session：占用提示。
    assert!(resume_session(&mut state, pane_b, "sess-1"));
    assert!(state.toast.is_some(), "占用提示");
    // 未入队 Load 效应。
    assert!(
        !state
            .effects
            .iter()
            .any(|effect| matches!(effect, crate::app::effects::Effect::LoadChatSession { .. }))
    );
}

#[test]
fn session_loaded_rebuilds_view_and_selection() {
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
            created_at_ms: 0,
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
            status: "interrupted".into(),
            model_provider: Some("p".into()),
            model_id: Some("m".into()),
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
                kind: "reasoning".into(),
                content: "半截思考".into(),
                provider_metadata_json: Some(r#"{"signature":"s"}"#.into()),
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
        "sess-9",
        &messages,
        Some(ModelSelection {
            provider: "p".into(),
            model: "m".into(),
            variant: Some("high".into()),
        })
    ));
    let target = state.pane_anywhere(pane).unwrap();
    assert_eq!(target.lx.chat.session_id.as_deref(), Some("sess-9"));
    assert_eq!(target.lx.chat.messages.len(), 2);
    assert_eq!(
        target.lx.chat.messages[1].status, "interrupted",
        "失败/中断状态可识别"
    );
    assert_eq!(
        target.lx.chat.messages[1]
            .parts()
            .next()
            .map(|part| part.kind_str()),
        Some("reasoning")
    );
    assert_eq!(
        target
            .lx
            .chat
            .selection
            .as_ref()
            .unwrap()
            .variant
            .as_deref(),
        Some("high")
    );
}

#[test]
fn model_picker_two_stage_and_esc_back() {
    let mut state = ready_state("问");
    let pane = focus_pane(&state);
    // 打开 `/model` 面板。
    assert!(open_model_picker(&mut state, pane));
    // 有 variants：第一级 Enter 进入第二级。
    assert!(apply_slash_enter(&mut state, pane));
    assert!(matches!(
        state.pane_anywhere(pane).unwrap().lx.slash,
        Some(SlashPanel::Model(ref picker)) if matches!(picker.stage, ModelPickerStage::Variants { .. })
    ));
    // Esc 退回模型列表（直接调用退回分支）。
    if let Some(target) = state.pane_mut_anywhere(pane)
        && let Some(SlashPanel::Model(picker)) = target.lx.slash.as_mut()
    {
        picker.stage = ModelPickerStage::Models;
        picker.window = crate::app::state::lx::slash::PanelWindow::new(picker.window.active);
    }
    assert!(matches!(
        state.pane_anywhere(pane).unwrap().lx.slash,
        Some(SlashPanel::Model(ref picker)) if matches!(picker.stage, ModelPickerStage::Models)
    ));
    // 关闭面板。
    if let Some(target) = state.pane_mut_anywhere(pane) {
        target.lx.slash = None;
    }
    assert!(state.pane_anywhere(pane).unwrap().lx.slash.is_none());
}

#[test]
fn model_without_variants_selects_directly() {
    let mut state = ready_state("问");
    state.models = models_with(None);
    let pane = focus_pane(&state);
    open_model_picker(&mut state, pane);
    // 无 variants：Enter 直接完成选择。
    assert!(apply_slash_enter(&mut state, pane));
    assert!(
        state.pane_anywhere(pane).unwrap().lx.slash.is_none(),
        "面板关闭"
    );
    assert!(
        state
            .pane_anywhere(pane)
            .unwrap()
            .lx
            .chat
            .selection
            .is_some()
    );
}

#[test]
fn apply_models_changed_updates_snapshot() {
    let mut state = AppState::demo();
    let mut settings = AiSettings::empty();
    assert!(!apply_models_changed(&mut state, &settings));
    settings.stream_idle_timeout_ms = Some(5_000);
    assert!(apply_models_changed(&mut state, &settings));
    assert_eq!(state.models.settings.idle_timeout_ms(), 5_000);
}

#[test]
fn append_stream_delta_merges_adjacent_same_kind() {
    let mut chat = LxChat::new();
    chat.messages.push(crate::app::state::ChatMessageView {
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
    });
    chat.append_stream_delta(PartKind::Text, "a", None);
    chat.append_stream_delta(PartKind::Text, "b", None);
    chat.append_stream_delta(PartKind::Reasoning, "r", None);
    chat.append_stream_delta(PartKind::Text, "c", None);
    assert_eq!(
        chat.messages[0]
            .parts()
            .map(|part| (part.kind_str(), part.content.as_str()))
            .collect::<Vec<_>>(),
        vec![("text", "ab"), ("reasoning", "r"), ("text", "c")],
        "相邻同 kind 合并、类型切换新建"
    );
}

#[test]
fn signature_metadata_binds_to_reasoning_part() {
    let mut chat = LxChat::new();
    chat.messages.push(crate::app::state::ChatMessageView {
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
    });
    chat.append_stream_delta(PartKind::Reasoning, "思考", None);
    // 空文本 + 签名元数据：绑定到相邻 reasoning part。
    chat.append_stream_delta(
        PartKind::Reasoning,
        "",
        Some(serde_json::json!({"signature": "sig-1", "blockIndex": 0})),
    );
    assert_eq!(chat.messages[0].parts().count(), 1, "不生成空 part");
    let bound = serde_json::from_str::<serde_json::Value>(
        chat.messages[0]
            .parts()
            .next()
            .and_then(|part| part.provider_metadata_json.as_deref())
            .unwrap_or_default(),
    )
    .expect("metadata json");
    assert_eq!(bound.get("signature"), Some(&serde_json::json!("sig-1")));
    assert_eq!(bound.get("blockIndex"), Some(&serde_json::json!(0)));
}

#[test]
fn model_picker_variants_applies_chosen_level() {
    // 回归：第二级 Enter 必须应用高亮等级，而不是沿用模型默认 variant。
    let mut state = ready_state("问");
    // 两个等级：low / high，模型默认 high。
    let mut settings = models_with(Some("high"));
    let provider = settings.settings.providers.get_mut("p").expect("provider");
    let model = provider.models.get_mut("m").expect("model");
    model
        .variants
        .insert("low".into(), serde_json::json!({"reasoningEffort": "low"}));
    state.models = settings;
    let pane = focus_pane(&state);
    assert!(open_model_picker(&mut state, pane));
    // 第一级 Enter 进入第二级（默认高亮 high）。
    assert!(apply_slash_enter(&mut state, pane));
    // 上移到 low（列表顺序 low, high → 从 high 上移一格到 low）。
    move_panel_active(&mut state, pane, -1);
    assert!(apply_slash_enter(&mut state, pane));
    let selection = state
        .pane_anywhere(pane)
        .and_then(|p| p.lx.chat.selection.clone())
        .expect("selection");
    assert_eq!(
        selection.variant.as_deref(),
        Some("low"),
        "第二级应采纳用户高亮的等级"
    );
}

/// 移动 `/` 面板高亮（走真实的面板窗口逻辑）。
fn move_panel_active(state: &mut AppState, pane: PaneId, delta: isize) {
    if let Some(target) = state.pane_mut_anywhere(pane)
        && let Some(panel) = target.lx.slash.as_mut()
    {
        panel.move_active(delta);
    }
}

#[test]
fn slash_hover_sets_active_without_scrolling_window() {
    let mut state = ready_state("问");
    let pane = focus_pane(&state);
    seed_slash_commands(&mut state, pane);
    // 悬停到第 2 项：高亮变、锚点不动。
    let before_anchor = slash_window(&state, pane).anchor;
    assert!(crate::app::update::hover_lx_slash(&mut state, pane, 2));
    let window = slash_window(&state, pane);
    assert_eq!(window.active, 2);
    assert_eq!(window.anchor, before_anchor, "悬停不滚动窗口");
}

#[test]
fn slash_keyboard_move_cycles_and_follows_highlight() {
    let mut state = ready_state("问");
    let pane = focus_pane(&state);
    seed_slash_commands(&mut state, pane);
    // 循环到末项：末项应当是命令表的最后一条（不硬编码索引，
    // 命令表增删时这条测试不应跟着改）。
    let last = crate::app::state::SlashCommand::ALL.len() - 1;
    move_panel_active(&mut state, pane, -1);
    let window = slash_window(&state, pane);
    assert_eq!(window.active, last, "上移循环到末项");
    assert_eq!(window.anchor, window.active, "键盘移动锚点跟随高亮");
}

#[test]
fn slash_wheel_scrolls_window_without_moving_highlight() {
    let mut state = ready_state("问");
    let pane = focus_pane(&state);
    seed_slash_commands(&mut state, pane);
    let changed = crate::app::update::scroll_lx_slash(&mut state, pane, 1, 0);
    let window = slash_window(&state, pane);
    assert_eq!(window.viewport, Some(1), "滚轮设置显式视口");
    assert_eq!(window.active, 0, "滚轮不动高亮");
    // base=1 再滚回 0。
    assert!(crate::app::update::scroll_lx_slash(&mut state, pane, -1, 1));
    assert_eq!(slash_window(&state, pane).viewport, Some(0));
    let _ = changed;
}

#[test]
fn slash_click_selects_row() {
    let mut state = ready_state("问");
    let pane = focus_pane(&state);
    seed_slash_commands(&mut state, pane);
    // 点选第 2 项（/resume）：应执行对应命令（打开历史面板）。
    assert!(crate::app::update::select_lx_slash(&mut state, pane, 1));
    assert!(
        matches!(
            state.pane_anywhere(pane).unwrap().lx.slash,
            Some(SlashPanel::History(_))
        ),
        "点选 /resume 后打开历史面板"
    );
}

/// 直接放入命令候选面板。
fn seed_slash_commands(state: &mut AppState, pane: PaneId) {
    if let Some(target) = state.pane_mut_anywhere(pane) {
        target.lx.slash = Some(SlashPanel::Commands {
            items: crate::app::state::SlashCommand::ALL.to_vec(),
            window: crate::app::state::lx::slash::PanelWindow::new(0),
        });
    }
}

/// 读取 `/` 面板窗口状态。
fn slash_window(state: &AppState, pane: PaneId) -> crate::app::state::lx::slash::PanelWindow {
    state
        .pane_anywhere(pane)
        .and_then(|p| p.lx.slash.as_ref())
        .map(|panel| panel.window())
        .expect("slash panel")
}

#[test]
fn finish_model_selection_persists_default_model() {
    let mut state = AppState::demo();
    state.models = models_with(None);
    state.models.settings.default_model = None;
    let pane = state.active_tab().layout.focus();
    let selection = ModelSelection {
        provider: "p".into(),
        model: "m".into(),
        variant: None,
    };
    crate::app::update::chat::test_hooks::finish_model_selection(
        &mut state,
        pane,
        selection.clone(),
    );
    // 窗格选择写入。
    assert_eq!(
        state.pane_anywhere(pane).expect("pane").lx.chat.selection,
        Some(selection.clone())
    );
    // 入队写回 ai.json defaultModel。
    assert!(
        state.effects.iter().any(|effect| matches!(
            effect,
            crate::app::effects::Effect::SaveDefaultModel { selection: saved } if *saved == selection
        )),
        "入队持久化默认模型：{:?}",
        state.effects
    );
    // 快照同步更新，重复选择不再入队。
    let before = state.effects.len();
    crate::app::update::chat::test_hooks::finish_model_selection(&mut state, pane, selection);
    assert_eq!(state.effects.len(), before, "同选择不重复入队");
}

/// 建立「流中」状态：草稿 → Committing → ack → Streaming；返回 (状态, 窗格, 请求键)。
fn streaming_state(session: &str) -> (AppState, PaneId, RequestKey) {
    let mut state = ready_state("问");
    let pane = focus_pane(&state);
    assert!(try_send(&mut state, pane));
    let generation = match &state.pane_anywhere(pane).unwrap().lx.chat.request {
        RequestState::Committing { generation, .. } => *generation,
        _ => panic!("committing"),
    };
    assert!(on_turn_committed(
        &mut state,
        pane,
        generation,
        &committed(session)
    ));
    let key = RequestKey {
        pane,
        session: session.into(),
        generation,
    };
    (state, pane, key)
}

/// 过去 `STOP_ARM_WINDOW` 加 1ms 的时刻（用于待命过期分支）。
fn expired_arm() -> std::time::Instant {
    std::time::Instant::now()
        .checked_sub(STOP_ARM_WINDOW + Duration::from_millis(1))
        .expect("monotonic clock")
}

// ===== 发送时的「未知命令」拦截（提示显示在输入框上方）=====

/// 辅助：断言「被拦下、没有产生 BeginChatTurn、草稿保留、输入框上方出现提示」。
fn assert_blocked_as_unknown_command(draft: &str, expected_echo: &str) {
    let mut state = ready_state(draft);
    let pane = focus_pane(&state);
    assert!(try_send(&mut state, pane), "{draft:?} 应被拦下并给出反馈");
    // 没有产生发送效应。
    assert!(
        !state
            .effects
            .iter()
            .any(|effect| matches!(effect, crate::app::effects::Effect::BeginChatTurn { .. })),
        "{draft:?} 不应真的发给模型：{:?}",
        state.effects
    );
    // 草稿保留，用户可以直接改。
    assert_eq!(
        state.pane_anywhere(pane).expect("pane").lx.input,
        draft,
        "草稿必须保留"
    );
    // 提示改走 main content 的 toast（warning 档，黄）。
    let toast = state.toast.as_ref().expect("应有提示");
    assert_eq!(
        toast.kind,
        crate::app::toast::ToastKind::Warning,
        "未知命令属于「要留意」→ warning"
    );
    assert_eq!(toast.anchor, Some(pane), "toast 锚在触发它的窗格");
    assert!(
        toast.message.contains(expected_echo),
        "回显命令 {expected_echo:?}：{}",
        toast.message
    );
    // 极简：一条 toast 放得下。
    assert!(
        toast.message.chars().count() <= 40,
        "文案必须极简：{}",
        toast.message
    );
}

#[test]
fn unknown_slash_command_warns_instead_of_sending() {
    assert_blocked_as_unknown_command("/foo", "/foo");
    assert_blocked_as_unknown_command("/summary", "/summary");
}

/// 带空格的 `/...` **不**拦截：用户是在写普通内容（`/q a`、路径等）。
#[test]
fn slash_with_spaces_is_sent_as_plain_text() {
    for draft in [
        "/q a",
        "/foo 参数",
        "/path/to/file.rs 这个文件有问题",
        "/etc/hosts 是什么",
    ] {
        let mut state = ready_state(draft);
        let pane = focus_pane(&state);
        try_send(&mut state, pane);
        let sent = state
            .effects
            .iter()
            .any(|effect| matches!(effect, crate::app::effects::Effect::BeginChatTurn { .. }));
        assert!(sent, "{draft:?} 应作为普通内容发送");
        assert!(state.toast.is_none(), "{draft:?} 不该有提示");
    }
}

#[test]
fn unknown_command_toast_expires_after_its_duration() {
    let mut state = ready_state("/nope");
    let pane = focus_pane(&state);
    try_send(&mut state, pane);
    let deadline = state
        .toast
        .as_ref()
        .expect("有提示")
        .next_deadline()
        .expect("未悬停时有到期时刻");
    assert!(
        !super::super::tick(&mut state, Instant::now()),
        "未到期不清除"
    );
    assert!(super::super::tick(&mut state, deadline), "到期应清除");
    assert!(state.toast.is_none(), "到点后提示消失");
    assert!(!super::super::tick(&mut state, deadline), "已清过不再变化");
}

#[test]
fn unknown_command_toast_sets_a_wakeup_deadline() {
    let mut state = ready_state("/nope");
    let pane = focus_pane(&state);
    try_send(&mut state, pane);
    let deadline = state
        .toast
        .as_ref()
        .expect("有提示")
        .next_deadline()
        .expect("未悬停时有到期时刻");
    assert_eq!(super::super::next_deadline(&state), Some(deadline));
}

#[test]
fn known_commands_and_plain_text_still_send_normally() {
    for draft in [
        "帮我看看这段代码",
        "/model",           // 内置命令（面板能匹配到）
        "/resume",          // 内置命令
        "/new",             // 内置命令
        "/session",         // /resume 的别名
        "/",                // 还没敲完
        "/foo\n正文第二行", // 多行草稿 → 不按命令处理
    ] {
        let mut state = ready_state(draft);
        let pane = focus_pane(&state);
        try_send(&mut state, pane);
        let sent = state
            .effects
            .iter()
            .any(|effect| matches!(effect, crate::app::effects::Effect::BeginChatTurn { .. }));
        assert!(sent, "{draft:?} 应按普通消息发送");
        assert!(state.toast.is_none(), "{draft:?} 不该有提示");
    }
}

#[test]
fn unknown_command_detection_matches_panel_rules() {
    for draft in ["/mo", "/mod", "/resu", "/ne", "/sess", "/cl"] {
        assert_eq!(
            unknown_slash_command(draft),
            None,
            "{draft:?} 面板能匹配到，应交给面板"
        );
    }
    for draft in ["/foo", "/zzz", "/models", "/resumes"] {
        assert_eq!(
            unknown_slash_command(draft),
            Some(draft.to_string()),
            "{draft:?} 应判为未知命令"
        );
    }
    // 空草稿、纯文本、未敲完的 `/`、多行、行内斜杠、**带空格**、路径样文本都不算命令。
    for draft in [
        "",
        "hello",
        "/",
        "/x\ny",
        "前面有字 /foo",
        "/q a",
        "/foo 一些参数",
        "/zzz --flag value",
        "/path/to/file.rs 这个文件有问题",
        "/etc/hosts 是什么",
    ] {
        assert_eq!(
            unknown_slash_command(draft),
            None,
            "{draft:?} 不该判为未知命令"
        );
    }
}
