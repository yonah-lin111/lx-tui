//! 快捷连接测试：独立 `connect.json` 读写、来源解析、物化与运行时合并。

use std::collections::BTreeMap;
use std::path::PathBuf;

use super::connect::*;
use super::presets::{all_presets, default_models_for_preset, find_preset};
use super::{
    AiSettings, ModelConfig, ModelLimit, ModelSelection, ProviderConfig, ProviderOptions,
    ProviderTransport, load_ai, save_ai,
};

const FAKE_KEY: &str = "test-key-only-not-real";

fn sample_custom_provider(id: &str) -> ProviderConfig {
    let model = ModelConfig {
        id: "custom-chat".into(),
        name: "Custom Chat".into(),
        transport: None,
        limit: Some(ModelLimit {
            context: 128_000,
            output: 8_192,
        }),
        modalities: Some(serde_json::json!({"input": ["text"], "output": ["text"]})),
        variants: BTreeMap::new(),
        variant: None,
        pricing: None,
        extra: serde_json::Map::new(),
    };
    ProviderConfig {
        id: id.into(),
        kind: ProviderTransport::OpenaiCompatible,
        name: "Custom".into(),
        options: ProviderOptions {
            api_key: Some(FAKE_KEY.into()),
            base_url: "https://custom.invalid/v1".into(),
        },
        models: BTreeMap::from([(model.id.clone(), model)]),
        extra: serde_json::Map::new(),
    }
}

fn temp_dir(name: &str) -> PathBuf {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_nanos());
    let path = std::env::temp_dir().join(format!(
        "lx-tui-connect-test-{name}-{}",
        nanos.map(|value| format!("{value:x}")).unwrap_or_default()
    ));
    std::fs::create_dir_all(&path).expect("temp dir");
    path
}

fn openai_record() -> ConnectedPresetRecord {
    let mut record = ConnectedPresetRecord::new("openai");
    record.api_key = Some(FAKE_KEY.to_string());
    record
}

fn openai_model_id() -> String {
    default_models_for_preset("openai")
        .keys()
        .next()
        .cloned()
        .expect("openai model")
}

#[test]
fn connect_path_is_sibling_of_ai_path() {
    let ai_path = PathBuf::from("/tmp/lx/config/ai.json");
    assert_eq!(
        connect_path_for_ai_path(&ai_path),
        PathBuf::from("/tmp/lx/config/connect.json")
    );
}

#[test]
fn connect_config_path_under_data_root() {
    if let Ok(path) = connect_config_path() {
        let text = path.to_string_lossy();
        assert!(
            text.ends_with("config/connect.json"),
            "路径为 config/connect.json：{text}"
        );
    }
}

#[test]
fn connect_round_trip_is_atomic_and_private() {
    let dir = temp_dir("atomic");
    let path = dir.join("connect.json");
    let mut settings = ConnectSettings::default();
    settings.providers.insert("openai".into(), openai_record());
    save_connect(&path, &settings).expect("save");
    assert_eq!(load_connect(&path).expect("load"), Some(settings));
    let leftovers: Vec<String> = std::fs::read_dir(&dir)
        .expect("read dir")
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .filter(|name| name.ends_with(".tmp"))
        .collect();
    assert!(leftovers.is_empty(), "临时文件已清理: {leftovers:?}");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(&path)
            .expect("metadata")
            .permissions()
            .mode();
        assert_eq!(mode & 0o077, 0, "connect.json 权限收紧到所有者");
    }
}

#[test]
fn connect_rejects_mismatched_record_id() {
    let dir = temp_dir("mismatch");
    let path = dir.join("connect.json");
    let mut settings = ConnectSettings::default();
    let mut record = openai_record();
    record.id = "other".into();
    settings.providers.insert("openai".into(), record);
    assert!(save_connect(&path, &settings).is_err());
    assert!(!path.exists());
}

#[test]
fn save_connect_does_not_touch_ai_file() {
    let dir = temp_dir("isolation");
    let ai_path = dir.join("ai.json");
    let connect_path = dir.join("connect.json");
    let ai_body = r#"{"ai":{"enabled_providers":[],"providers":{}}}"#;
    std::fs::write(&ai_path, ai_body).expect("write ai");
    let mut settings = ConnectSettings::default();
    settings.providers.insert("openai".into(), openai_record());
    save_connect(&connect_path, &settings).expect("save connect");
    assert_eq!(std::fs::read_to_string(&ai_path).expect("read ai"), ai_body);
}

#[test]
fn connection_source_priority_is_oauth_then_key_then_env() {
    let preset = find_preset("openai").expect("openai");
    let mut record = openai_record();
    record.oauth = Some(OAuthCredential {
        method_id: "chatgpt-browser".into(),
        access_token: "oauth-token".into(),
        refresh_token: None,
        expires_at_ms: None,
        account_label: Some("team".into()),
        metadata: BTreeMap::new(),
    });
    let source = resolve_preset_connection_source_with_env(preset, Some(&record), |_| {
        Some("env-key".to_string())
    })
    .expect("source");
    assert_eq!(
        source,
        ConnectionSource::OAuth {
            label: Some("team".into())
        }
    );
    assert_eq!(source.summary_label(), "OAuth · team");

    let mut record = openai_record();
    record.oauth = Some(OAuthCredential {
        method_id: "chatgpt-browser".into(),
        access_token: "   ".into(),
        refresh_token: None,
        expires_at_ms: None,
        account_label: None,
        metadata: BTreeMap::new(),
    });
    assert_eq!(
        resolve_preset_connection_source_with_env(preset, Some(&record), |_| None),
        Some(ConnectionSource::ApiKey)
    );

    let source = resolve_preset_connection_source_with_env(preset, None, |name| {
        (name == "OPENAI_API_KEY").then(|| "env-key".to_string())
    })
    .expect("env");
    assert_eq!(
        source,
        ConnectionSource::Env {
            var_name: "OPENAI_API_KEY".into()
        }
    );
    assert_eq!(source.summary_label(), "$OPENAI_API_KEY");
}

#[test]
fn disabled_record_suppresses_env_fallback() {
    let preset = find_preset("openai").expect("openai");
    let mut record = ConnectedPresetRecord::new("openai");
    record.enabled = false;
    assert!(
        resolve_preset_connection_source_with_env(preset, Some(&record), |_| {
            Some("env-key".to_string())
        })
        .is_none(),
        "显式禁用后不得回退环境变量"
    );
}

#[test]
fn materialize_sets_api_key_and_default_models() {
    let preset = find_preset("openai").expect("openai");
    let record = openai_record();
    let (provider, enabled) = materialize_preset_provider_with_env(preset, Some(&record), |_| None)
        .expect("materialized");
    assert!(enabled);
    assert_eq!(provider.id, "openai");
    assert_eq!(provider.name, preset.name);
    assert_eq!(provider.options.api_key.as_deref(), Some(FAKE_KEY));
    assert_eq!(provider.options.base_url, "https://api.openai.com/v1");
    assert!(!provider.models.is_empty());
    assert_eq!(provider.models, default_models_for_preset("openai"));
}

#[test]
fn materialize_expands_form_answers_and_env_key() {
    let preset = find_preset("cloudflare-ai-gateway").expect("cloudflare");
    let mut record = ConnectedPresetRecord::new("cloudflare-ai-gateway");
    record
        .form_answers
        .insert("accountId".into(), "account".into());
    record
        .form_answers
        .insert("gatewayId".into(), "gateway".into());
    let (provider, enabled) = materialize_preset_provider_with_env(preset, Some(&record), |name| {
        (name == "CLOUDFLARE_API_TOKEN").then(|| "env-token".to_string())
    })
    .expect("materialized");
    assert!(enabled);
    assert_eq!(provider.options.api_key.as_deref(), Some("env-token"));
    assert_eq!(
        provider.options.base_url,
        "https://gateway.ai.cloudflare.com/v1/account/gateway/openai"
    );
}

#[test]
fn materialize_env_fills_template_fields_without_record() {
    let preset = find_preset("cloudflare-ai-gateway").expect("cloudflare");
    let (provider, enabled) =
        materialize_preset_provider_with_env(preset, None, |name| match name {
            "CLOUDFLARE_ACCOUNT_ID" => Some("account".to_string()),
            "CLOUDFLARE_GATEWAY_ID" => Some("gateway".to_string()),
            "CLOUDFLARE_API_TOKEN" => Some("env-token".to_string()),
            _ => None,
        })
        .expect("env materialized");
    assert!(enabled);
    assert_eq!(provider.options.api_key.as_deref(), Some("env-token"));
    assert_eq!(
        provider.options.base_url,
        "https://gateway.ai.cloudflare.com/v1/account/gateway/openai"
    );
}

#[test]
fn materialize_uses_oauth_metadata_endpoint() {
    let preset = find_preset("github-copilot").expect("copilot");
    let mut record = ConnectedPresetRecord::new("github-copilot");
    record.oauth = Some(OAuthCredential {
        method_id: "device".into(),
        access_token: "gh-token".into(),
        refresh_token: None,
        expires_at_ms: None,
        account_label: None,
        metadata: BTreeMap::from([(
            "apiEndpoint".to_string(),
            "https://api.enterprise.example".to_string(),
        )]),
    });
    let (provider, _) = materialize_preset_provider_with_env(preset, Some(&record), |_| None)
        .expect("materialized");
    assert_eq!(provider.options.base_url, "https://api.enterprise.example");
    assert_eq!(provider.options.api_key.as_deref(), Some("gh-token"));
}

#[test]
fn merge_injects_connected_preset_and_sanitizes_defaults() {
    let model_id = openai_model_id();
    let mut connect = ConnectSettings::default();
    connect.providers.insert("openai".into(), openai_record());
    let mut custom = AiSettings::empty();
    custom.default_model = Some(ModelSelection {
        provider: "openai".into(),
        model: model_id.clone(),
        variant: None,
    });
    custom.title_summary = Some(ModelSelection {
        provider: "openai".into(),
        model: model_id.clone(),
        variant: None,
    });
    let merged = merge_runtime_ai_settings_with_env(&custom, &connect, |_| None);
    assert!(merged.providers.contains_key("openai"));
    assert!(merged.enabled_providers.iter().any(|id| id == "openai"));
    assert_eq!(
        merged
            .default_model
            .as_ref()
            .map(|selection| selection.provider.as_str()),
        Some("openai")
    );
    assert_eq!(
        merged
            .title_summary
            .as_ref()
            .map(|selection| selection.model.as_str()),
        Some(model_id.as_str())
    );
}

#[test]
fn merge_drops_dangling_default_without_connection() {
    let model_id = openai_model_id();
    let mut custom = AiSettings::empty();
    custom.default_model = Some(ModelSelection {
        provider: "openai".into(),
        model: model_id,
        variant: None,
    });
    let merged = merge_runtime_ai_settings_with_env(&custom, &ConnectSettings::default(), |_| None);
    assert!(!merged.providers.contains_key("openai"));
    assert!(merged.default_model.is_none());
}

#[test]
fn merge_keeps_custom_provider_untouched() {
    let mut custom = AiSettings::empty();
    let provider = sample_custom_provider("custom");
    custom.providers.insert("custom".into(), provider.clone());
    custom.enabled_providers.push("custom".into());
    let mut connect = ConnectSettings::default();
    connect.providers.insert("openai".into(), openai_record());
    let merged = merge_runtime_ai_settings_with_env(&custom, &connect, |_| None);
    assert_eq!(merged.providers["custom"], provider);
    assert!(merged.providers.contains_key("openai"));
}

#[test]
fn load_and_save_ai_accept_quick_connect_default_model() {
    let dir = temp_dir("quick-default");
    let ai_path = dir.join("ai.json");
    let connect_path = dir.join("connect.json");
    let model_id = openai_model_id();
    let mut connect = ConnectSettings::default();
    connect.providers.insert("openai".into(), openai_record());
    save_connect(&connect_path, &connect).expect("save connect");
    let ai_json = format!(
        r#"{{"ai":{{"enabled_providers":[],"providers":{{}},"defaultModel":{{"provider":"openai","model":"{model_id}"}}}}}}"#
    );
    std::fs::write(&ai_path, &ai_json).expect("write ai");
    let connect_before = std::fs::read_to_string(&connect_path).expect("read connect");

    // 无 connect.json 时严格校验拒绝；load_ai 结合 connect.json 后接受。
    assert!(super::parse_ai(&ai_json).is_err());
    let settings = load_ai(&ai_path).expect("load").expect("exists");
    assert_eq!(
        settings
            .default_model
            .as_ref()
            .map(|selection| selection.provider.as_str()),
        Some("openai")
    );
    save_ai(&ai_path, &settings).expect("save ai");
    assert_eq!(
        std::fs::read_to_string(&connect_path).expect("re-read connect"),
        connect_before,
        "保存 ai.json 不得改写 connect.json"
    );

    let runtime = load_runtime_ai(&ai_path).expect("runtime").expect("exists");
    assert!(runtime.providers.contains_key("openai"));
    assert_eq!(
        runtime
            .providers
            .get("openai")
            .and_then(|provider| provider.options.api_key.as_deref()),
        Some(FAKE_KEY)
    );
}

#[test]
fn runtime_load_drops_default_when_connection_is_disabled() {
    let dir = temp_dir("runtime-disabled");
    let ai_path = dir.join("ai.json");
    let connect_path = dir.join("connect.json");
    let model_id = openai_model_id();
    let mut connect = ConnectSettings::default();
    let mut record = openai_record();
    record.enabled = false;
    connect.providers.insert("openai".into(), record);
    save_connect(&connect_path, &connect).expect("save connect");
    std::fs::write(
        &ai_path,
        format!(
            r#"{{"ai":{{"enabled_providers":[],"providers":{{}},"defaultModel":{{"provider":"openai","model":"{model_id}"}}}}}}"#
        ),
    )
    .expect("write ai");
    let runtime = load_runtime_ai(&ai_path).expect("runtime").expect("exists");
    assert!(runtime.providers.contains_key("openai"));
    assert!(!runtime.enabled_providers.iter().any(|id| id == "openai"));
    assert!(
        runtime.default_model.is_none(),
        "禁用连接的悬空默认模型应清理"
    );
}

#[test]
fn load_runtime_reads_connect_without_ai_file() {
    let dir = temp_dir("runtime-no-ai");
    let ai_path = dir.join("ai.json");
    let connect_path = dir.join("connect.json");
    let mut connect = ConnectSettings::default();
    connect.providers.insert("openai".into(), openai_record());
    save_connect(&connect_path, &connect).expect("save connect");
    let runtime = load_runtime_ai(&ai_path).expect("runtime").expect("exists");
    assert!(runtime.providers.contains_key("openai"));
    assert!(runtime.default_model.is_none());
    assert!(load_runtime_ai_or_empty(&ai_path).is_ok());
}

#[test]
fn missing_connect_file_returns_none() {
    let dir = temp_dir("missing");
    assert!(
        load_connect(&dir.join("connect.json"))
            .expect("query")
            .is_none()
    );
    assert_eq!(
        load_connect_or_default(&dir.join("connect.json")).expect("default"),
        ConnectSettings::default()
    );
}

#[test]
fn env_snapshot_only_contains_known_non_empty_variables() {
    let known: std::collections::BTreeSet<&str> = all_presets()
        .iter()
        .flat_map(|preset| preset.env_vars.iter().copied())
        .collect();
    for (name, value) in collect_preset_env_snapshot() {
        assert!(known.contains(name.as_str()), "unexpected env var {name}");
        assert!(!value.is_empty());
        assert_eq!(value, value.trim());
    }
}

#[test]
fn disconnect_with_env_keeps_disabled_record_and_clears_defaults() {
    let dir = temp_dir("disconnect-env");
    let ai_path = dir.join("ai.json");
    let connect_path = dir.join("connect.json");
    let model_id = openai_model_id();
    let mut connect = ConnectSettings::default();
    let mut record = openai_record();
    record.form_answers.insert("custom".into(), "keep".into());
    connect.providers.insert("openai".into(), record);
    save_connect(&connect_path, &connect).expect("save connect");
    std::fs::write(
        &ai_path,
        format!(
            r#"{{"ai":{{"enabled_providers":[],"providers":{{}},"defaultModel":{{"provider":"openai","model":"{model_id}"}},"titleSummary":{{"provider":"openai","model":"{model_id}"}}}}}}"#
        ),
    )
    .expect("write ai");

    let runtime = disconnect_preset_on_disk_with_env(&ai_path, "openai", |name| {
        (name == "OPENAI_API_KEY").then(|| "env-key".to_string())
    })
    .expect("disconnect");

    let saved = load_connect(&connect_path).expect("load").expect("exists");
    let record = &saved.providers["openai"];
    assert!(!record.enabled, "环境变量仍可用时必须禁用而不是删除");
    assert!(record.api_key.is_none());
    assert!(record.oauth.is_none());
    assert_eq!(
        record.form_answers.get("custom").map(String::as_str),
        Some("keep")
    );
    assert!(!runtime.providers.contains_key("openai"));

    let text = std::fs::read_to_string(&ai_path).expect("read ai");
    let parsed: serde_json::Value = serde_json::from_str(&text).expect("json");
    assert!(parsed["ai"].get("defaultModel").is_none());
    assert!(parsed["ai"].get("titleSummary").is_none());
}

#[test]
fn disconnect_without_env_removes_record() {
    let dir = temp_dir("disconnect-remove");
    let ai_path = dir.join("ai.json");
    let connect_path = dir.join("connect.json");
    let model_id = openai_model_id();
    let mut connect = ConnectSettings::default();
    connect.providers.insert("openai".into(), openai_record());
    save_connect(&connect_path, &connect).expect("save connect");
    std::fs::write(
        &ai_path,
        format!(
            r#"{{"ai":{{"enabled_providers":[],"providers":{{}},"defaultModel":{{"provider":"openai","model":"{model_id}"}}}}}}"#
        ),
    )
    .expect("write ai");

    let runtime =
        disconnect_preset_on_disk_with_env(&ai_path, "openai", |_| None).expect("disconnect");

    let saved = load_connect(&connect_path).expect("load").expect("exists");
    assert!(!saved.providers.contains_key("openai"));
    assert!(!runtime.providers.contains_key("openai"));
}

#[test]
fn disconnect_propagates_corrupted_connect_file() {
    let dir = temp_dir("disconnect-corrupt");
    let ai_path = dir.join("ai.json");
    std::fs::write(dir.join("connect.json"), "{not valid json").expect("write connect");
    assert!(disconnect_preset_on_disk_with_env(&ai_path, "openai", |_| None).is_err());
}
