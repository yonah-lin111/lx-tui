//! `LxChat` 上下文占用：单步分帧合并与 `/compact` 边界过滤。

use super::*;
use crate::app::state::CompactNoticeView;

fn message(sequence: i64, role: &str, input: Option<i64>, output: Option<i64>) -> ChatMessageView {
    ChatMessageView {
        id: format!("m{sequence}"),
        sequence,
        kind: "chat".into(),
        role: role.into(),
        status: "complete".into(),
        turn_index: 0,
        shell: None,
        steps: Vec::new(),
        model_label: None,
        error_code: None,
        input_tokens: input,
        output_tokens: output,
        cache_read_tokens: None,
        duration_ms: None,
        reports: Vec::new(),
        attachments: Vec::new(),
    }
}

fn checkpoint(end_sequence: i64, active: bool) -> CompactNoticeView {
    CompactNoticeView {
        id: "c1".into(),
        sequence: end_sequence,
        model_label: "m".into(),
        created_at_ms: 0,
        start_sequence: 0,
        end_sequence,
        summary: "sum".into(),
        active,
        auto: false,
    }
}

/// 分帧到达的 input / output 在同一采样步内合并，下一步从空槽重新开始。
#[test]
fn absorb_usage_delta_merges_split_frames() {
    let mut chat = LxChat::new();
    chat.absorb_usage_delta(&Usage {
        input: Some(100),
        cache_read: Some(40),
        ..Default::default()
    });
    assert_eq!(
        chat.context_tokens,
        Some(100),
        "只有 input 时 output 按 0 计"
    );
    chat.absorb_usage_delta(&Usage {
        output: Some(20),
        ..Default::default()
    });
    assert_eq!(chat.context_tokens, Some(120), "同一步 input + output");
    // 下一步只报 input：不得混入上一步 output。
    chat.reset_step_usage();
    chat.absorb_usage_delta(&Usage {
        input: Some(300),
        ..Default::default()
    });
    assert_eq!(chat.context_tokens, Some(300));
}

/// 只取生效压缩边界之后的最后一条带用量 assistant 消息。
#[test]
fn recompute_uses_last_assistant_after_boundary() {
    let mut chat = LxChat::new();
    chat.messages = vec![
        message(0, "user", None, None),
        message(1, "assistant", Some(100), Some(10)),
        message(2, "user", None, None),
        message(3, "assistant", Some(200), Some(20)),
    ];
    let mut compact = crate::app::state::LxCompact::default();
    compact.replace(vec![checkpoint(1, true)]);
    chat.recompute_context_tokens(&compact);
    assert_eq!(chat.context_tokens, Some(220), "边界后最后一条 assistant");
}

/// 压缩边界覆盖全部消息：尚无新回复时清空占用（隐藏旧百分比）。
#[test]
fn recompute_clears_when_boundary_covers_every_message() {
    let mut chat = LxChat::new();
    chat.messages = vec![
        message(0, "user", None, None),
        message(1, "assistant", Some(100), Some(10)),
    ];
    let mut compact = crate::app::state::LxCompact::default();
    compact.replace(vec![checkpoint(1, true)]);
    chat.recompute_context_tokens(&compact);
    assert_eq!(chat.context_tokens, None);
}

/// 已撤销的检查点不构成边界：撤销后恢复压缩前的最后一条回复。
#[test]
fn recompute_ignores_reverted_checkpoint() {
    let mut chat = LxChat::new();
    chat.messages = vec![
        message(0, "user", None, None),
        message(1, "assistant", Some(100), Some(10)),
    ];
    let mut compact = crate::app::state::LxCompact::default();
    compact.replace(vec![checkpoint(1, false)]);
    chat.recompute_context_tokens(&compact);
    assert_eq!(chat.context_tokens, Some(110));
}

/// 无生效检查点时按全部消息重算。
#[test]
fn recompute_without_checkpoint_uses_latest_assistant() {
    let mut chat = LxChat::new();
    chat.messages = vec![
        message(0, "user", None, None),
        message(1, "assistant", Some(100), Some(10)),
    ];
    chat.recompute_context_tokens(&crate::app::state::LxCompact::default());
    assert_eq!(chat.context_tokens, Some(110));
}

/// 取消发生在 usage 之前：保留最后已知上下文，不写成 0/None。
#[test]
fn close_stream_without_usage_keeps_known_context() {
    let mut chat = LxChat::new();
    chat.context_tokens = Some(500);
    chat.close_stream(&crate::chat::events::StopReason::Cancelled);
    assert_eq!(chat.context_tokens, Some(500));
}

/// 缓存命中率取整个对话累计值（Σcache / Σinput），不是最近一轮。
#[test]
fn refresh_status_aggregates_conversation_cache_hit() {
    let mut chat = LxChat::new();
    let mut first = message(1, "assistant", Some(1_000), Some(10));
    first.cache_read_tokens = Some(500);
    let mut second = message(3, "assistant", Some(1_000), Some(10));
    second.cache_read_tokens = Some(300);
    chat.messages = vec![
        message(0, "user", None, None),
        first,
        message(2, "user", None, None),
        second,
    ];
    chat.refresh_status(&ModelsSnapshot::default());
    assert_eq!(chat.status.cache_hit_percent, Some(40));
}

/// 缓存分子与分母按单条消息绑定：缺 input 的消息不计缓存，超出 input 的缓存被截断。
#[test]
fn refresh_status_binds_cache_to_input_and_clamps_per_message() {
    // 缺 `input_tokens` 的 assistant 只带 cache：不得计入分子（否则 1000/1000=100%）。
    let mut orphan = message(1, "assistant", None, Some(10));
    orphan.cache_read_tokens = Some(900);
    let mut measured = message(3, "assistant", Some(1_000), Some(10));
    measured.cache_read_tokens = Some(100);
    let mut chat = LxChat::new();
    chat.messages = vec![orphan, measured];
    chat.refresh_status(&ModelsSnapshot::default());
    assert_eq!(chat.status.cache_hit_percent, Some(10));

    // 单条 cache 超过 input 时按 input 截断：5000 → 1000，累计 1000/2000=50%。
    let mut inflated = message(5, "assistant", Some(1_000), Some(10));
    inflated.cache_read_tokens = Some(5_000);
    let mut plain = message(7, "assistant", Some(1_000), Some(10));
    plain.cache_read_tokens = Some(0);
    chat.messages = vec![inflated, plain];
    chat.refresh_status(&ModelsSnapshot::default());
    assert_eq!(chat.status.cache_hit_percent, Some(50));
}

/// 没有任何缓存读取时整段隐藏，不显示 0%。
#[test]
fn refresh_status_hides_cache_hit_without_cache_data() {
    let mut chat = LxChat::new();
    chat.messages = vec![message(1, "assistant", Some(1_000), Some(10))];
    chat.refresh_status(&ModelsSnapshot::default());
    assert_eq!(chat.status.cache_hit_percent, None);
}

// ===== 工作区模型记忆与全局最近使用回退 =====

fn selection(provider: &str, model: &str) -> ModelSelection {
    ModelSelection {
        provider: provider.into(),
        model: model.into(),
        variant: None,
    }
}

fn model_config(id: &str) -> crate::config::ai::ModelConfig {
    crate::config::ai::ModelConfig {
        id: id.into(),
        name: format!("{id} display"),
        transport: None,
        limit: None,
        modalities: None,
        variants: std::collections::BTreeMap::new(),
        variant: None,
        pricing: None,
        extra: serde_json::Map::new(),
    }
}

/// 构造启用且有 Key 的供应商；每个供应商的模型按 BTreeMap 顺序（传入顺序无关）。
fn settings_with(providers: &[(&str, &[&str])]) -> AiSettings {
    let mut settings = AiSettings::empty();
    for (provider_id, models) in providers {
        settings.enabled_providers.push((*provider_id).into());
        settings.providers.insert(
            (*provider_id).into(),
            crate::config::ai::ProviderConfig {
                id: (*provider_id).into(),
                kind: crate::config::ai::ProviderTransport::OpenaiCompatible,
                name: (*provider_id).into(),
                options: crate::config::ai::ProviderOptions {
                    api_key: Some("test-key".into()),
                    base_url: "https://api.example.invalid/v1".into(),
                },
                models: models
                    .iter()
                    .map(|model| ((*model).into(), model_config(model)))
                    .collect(),
                extra: serde_json::Map::new(),
            },
        );
    }
    settings
}

#[test]
fn fallback_selection_prefers_valid_default_model() {
    let mut settings = settings_with(&[("p1", &["m1"]), ("p2", &["m2"])]);
    settings.default_model = Some(selection("p2", "m2"));
    settings.recent_models = vec![selection("p1", "m1")];
    let models = ModelsSnapshot { settings };
    assert_eq!(
        models.fallback_selection(),
        Some(selection("p2", "m2")),
        "defaultModel 有效时优先"
    );
    assert!(models.title_selection().is_none(), "标题模型语义不变");
    assert!(models.recap_selection().is_none(), "recap 模型语义不变");
}

#[test]
fn fallback_selection_uses_first_valid_recent_when_default_missing_or_stale() {
    let mut settings = settings_with(&[("p1", &["m1"]), ("p2", &["m2"])]);
    settings.recent_models = vec![selection("deleted", "gone"), selection("p1", "m1")];
    let models = ModelsSnapshot { settings };
    assert_eq!(
        models.fallback_selection(),
        Some(selection("p1", "m1")),
        "跳过失效的 recent，取第一个有效项"
    );

    let mut settings = models.settings;
    settings.default_model = Some(selection("deleted", "gone"));
    let models = ModelsSnapshot { settings };
    assert_eq!(
        models.fallback_selection(),
        Some(selection("p1", "m1")),
        "defaultModel 失效时同样回退 recent"
    );
}

#[test]
fn fallback_selection_falls_back_to_first_available_model() {
    let mut settings = settings_with(&[("p1", &["m2", "m1"]), ("p2", &["m3"])]);
    settings.recent_models = vec![selection("deleted", "gone")];
    let models = ModelsSnapshot { settings };
    assert_eq!(
        models.fallback_selection(),
        Some(selection("p1", "m1")),
        "recent 全失效时按候选顺序取第一个可用模型"
    );
}

#[test]
fn fallback_selection_is_none_without_usable_model() {
    assert_eq!(ModelsSnapshot::default().fallback_selection(), None);
    let mut settings = settings_with(&[("p1", &["m1"])]);
    settings.enabled_providers.clear();
    assert_eq!(
        ModelsSnapshot { settings }.fallback_selection(),
        None,
        "未启用供应商不可用"
    );
    let mut settings = settings_with(&[("p1", &["m1"])]);
    settings.providers.get_mut("p1").unwrap().options.api_key = None;
    assert_eq!(
        ModelsSnapshot { settings }.fallback_selection(),
        None,
        "无 Key 不可用"
    );
}

#[test]
fn model_display_name_prefers_configured_name_and_falls_back_to_id() {
    let models = ModelsSnapshot {
        settings: settings_with(&[("p1", &["m1"])]),
    };
    assert_eq!(models.model_display_name("p1", "m1"), "m1 display");
    assert_eq!(
        models.model_display_name("p1", "unknown"),
        "unknown",
        "未知模型回退模型 id"
    );
    assert_eq!(
        ModelsSnapshot::default().model_display_name("p0", "m0"),
        "m0",
        "空快照回退模型 id"
    );
}

#[test]
fn remember_model_prepends_dedupes_overrides_variant_and_caps() {
    let mut models = ModelsSnapshot::default();
    assert!(models.remember_model(&selection("p", "a")));
    assert_eq!(models.settings.recent_models, vec![selection("p", "a")]);
    assert!(models.remember_model(&selection("p", "b")));
    assert_eq!(
        models.settings.recent_models,
        vec![selection("p", "b"), selection("p", "a")]
    );

    let mut variant_high = selection("p", "a");
    variant_high.variant = Some("high".into());
    assert!(models.remember_model(&variant_high), "variant 变化也算变化");
    assert_eq!(
        models.settings.recent_models,
        vec![variant_high.clone(), selection("p", "b")],
        "同 provider/model 去重前置，并覆盖最新 variant"
    );
    assert!(
        !models.remember_model(&variant_high),
        "同 provider/model/variant 不产生变化"
    );

    for index in 0..12 {
        let mut next = selection("p", &format!("m{index}"));
        next.variant = Some("low".into());
        models.remember_model(&next);
    }
    assert_eq!(
        models.settings.recent_models.len(),
        MODEL_RECENT_LIMIT,
        "保留上限"
    );
    assert_eq!(
        models.settings.recent_models.first(),
        Some(&{
            let mut newest = selection("p", "m11");
            newest.variant = Some("low".into());
            newest
        }),
        "最近使用在最前"
    );
    assert!(
        !models
            .settings
            .recent_models
            .iter()
            .any(|item| item.model == "m0" || item.model == "m1"),
        "最旧条目被截断"
    );

    let mut oldest_kept = selection("p", "m2");
    oldest_kept.variant = Some("low".into());
    assert!(models.remember_model(&oldest_kept));
    assert_eq!(
        models.settings.recent_models.first(),
        Some(&{
            let mut promoted = selection("p", "m2");
            promoted.variant = Some("low".into());
            promoted
        }),
        "已存在条目重新使用后移到队首"
    );
    assert_eq!(models.settings.recent_models.len(), MODEL_RECENT_LIMIT);
}

#[test]
fn effective_selection_falls_back_when_pane_provider_disabled_or_key_missing() {
    let mut settings = settings_with(&[("p1", &["m1"]), ("p2", &["m2"])]);
    settings.default_model = Some(selection("p2", "m2"));
    let mut models = ModelsSnapshot { settings };
    let mut chat = LxChat::new();
    chat.selection = Some(selection("p1", "m1"));

    models.settings.enabled_providers.retain(|id| id != "p1");
    assert_eq!(
        chat.effective_selection(&models),
        Some(selection("p2", "m2")),
        "窗格选择的供应商被停用时必须回落 fallback"
    );

    models.settings.enabled_providers.push("p1".into());
    models
        .settings
        .providers
        .get_mut("p1")
        .expect("provider")
        .options
        .api_key = None;
    assert_eq!(
        chat.effective_selection(&models),
        Some(selection("p2", "m2")),
        "窗格选择无 Key 时不得压住可用模型"
    );
}

#[test]
fn fallback_selection_normalizes_stale_recent_variant() {
    let mut settings = settings_with(&[("p", &["m"])]);
    let model = settings
        .providers
        .get_mut("p")
        .expect("provider")
        .models
        .get_mut("m")
        .expect("model");
    model
        .variants
        .insert("high".into(), serde_json::json!({"reasoning": "high"}));
    model.variant = Some("high".into());
    let mut stale = selection("p", "m");
    stale.variant = Some("gone".into());
    settings.recent_models = vec![stale];
    let models = ModelsSnapshot { settings };
    let mut expected = selection("p", "m");
    expected.variant = Some("high".into());
    assert_eq!(
        models.fallback_selection(),
        Some(expected),
        "recent 的 variant 失效时回落到模型默认等级，而不是丢弃整个模型"
    );
}

#[test]
fn effective_selection_prefers_valid_pane_selection() {
    let mut settings = settings_with(&[("p1", &["m1"]), ("p2", &["m2"])]);
    settings.default_model = Some(selection("p1", "m1"));
    settings.recent_models = vec![selection("p1", "m1")];
    let models = ModelsSnapshot { settings };
    let mut chat = LxChat::new();
    chat.selection = Some(selection("p2", "m2"));
    assert_eq!(
        chat.effective_selection(&models),
        Some(selection("p2", "m2")),
        "窗格选择有效时优先于 default/recent"
    );

    chat.selection = Some(selection("deleted", "gone"));
    assert_eq!(
        chat.effective_selection(&models),
        Some(selection("p1", "m1")),
        "窗格选择失效时走 fallback 链"
    );
}
