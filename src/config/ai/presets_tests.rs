//! 预设目录测试：A-Z 排序、协议映射、模型合法性与 Base URL 展开。

use std::collections::BTreeMap;

use super::presets::*;
use super::{ModelTransport, ProviderTransport};

#[test]
fn presets_are_sorted_by_name_case_insensitively() {
    let presets = all_presets();
    assert!(presets.len() > 100, "预设目录不应为空");
    for pair in presets.windows(2) {
        let left = pair[0].name.to_ascii_lowercase();
        let right = pair[1].name.to_ascii_lowercase();
        assert!(
            left <= right,
            "预设未按名称 A-Z 排序: {} ({}) > {} ({})",
            pair[0].name,
            pair[0].id,
            pair[1].name,
            pair[1].id
        );
    }
}

#[test]
fn preset_ids_are_unique() {
    let mut ids = std::collections::BTreeSet::new();
    for preset in all_presets() {
        assert!(ids.insert(preset.id), "重复的预设 ID: {}", preset.id);
    }
}

#[test]
fn preset_base_urls_are_http_or_templates() {
    for preset in all_presets() {
        if preset.default_base_url.contains("${") {
            assert!(
                preset.default_base_url.starts_with("https://"),
                "{} has a malformed template base URL",
                preset.id
            );
            continue;
        }
        super::validate_base_url(preset.default_base_url)
            .unwrap_or_else(|error| panic!("{} has an invalid base URL: {error}", preset.id));
    }
}

#[test]
fn core_presets_exist_with_expected_transports() {
    let cases = [
        ("openai", ProviderTransport::Openai),
        ("anthropic", ProviderTransport::Anthropic),
        ("google", ProviderTransport::Google),
        ("deepseek", ProviderTransport::OpenaiCompatible),
        ("opencode", ProviderTransport::Openai),
        ("opencode-go", ProviderTransport::OpenaiCompatible),
        ("github-copilot", ProviderTransport::Openai),
        ("cloudflare-ai-gateway", ProviderTransport::Openai),
        ("azure", ProviderTransport::OpenaiCompatible),
    ];
    for (id, transport) in cases {
        let preset = find_preset(id).unwrap_or_else(|| panic!("missing preset {id}"));
        assert_eq!(preset.transport, transport, "preset {id}");
    }
    assert!(
        find_preset("amazon-bedrock").is_none(),
        "不支持的 AWS 认证预设应剔除"
    );
    assert!(find_preset("google-vertex").is_none(), "ADC 预设应剔除");
}

#[test]
fn catalog_models_are_valid_for_runtime_validation() {
    for preset in all_presets() {
        let models = default_models_for_preset(preset.id);
        assert!(!models.is_empty(), "preset {} has no models", preset.id);
        for (key, model) in &models {
            assert_eq!(&model.id, key, "model key mismatch on {}", preset.id);
            if let Some(variant) = &model.variant {
                assert!(
                    model.variants.contains_key(variant),
                    "{}/{} default variant {variant} missing",
                    preset.id,
                    model.id
                );
            }
            for value in model.variants.values() {
                assert!(value.is_object(), "variant value must be an object");
            }
            if preset.transport == ProviderTransport::Openai {
                assert_ne!(
                    model.transport,
                    Some(ModelTransport::OpenaiCompatible),
                    "{}/{} uses forbidden openai-compatible transport",
                    preset.id,
                    model.id
                );
            }
            assert!(
                model
                    .limit
                    .is_none_or(|limit| limit.context > 0 || limit.output > 0),
                "{}/{} has an empty limit",
                preset.id,
                model.id
            );
            assert!(
                model
                    .modalities
                    .as_ref()
                    .and_then(|value| value.get("output"))
                    .is_some_and(|value| value.is_array()),
                "{}/{} missing output modalities",
                preset.id,
                model.id
            );
        }
    }
}

#[test]
fn responses_models_declare_responses_transport() {
    let openai = find_preset("openai").expect("openai");
    let models = default_models_for_preset(openai.id);
    assert!(
        models
            .values()
            .all(|model| model.transport == Some(ModelTransport::OpenaiResponses)),
        "OpenAI responses 模型必须显式声明 openai-responses"
    );
    let cloudflare = find_preset("cloudflare-ai-gateway").expect("cloudflare");
    assert!(
        default_models_for_preset(cloudflare.id)
            .values()
            .all(|model| model.transport == Some(ModelTransport::OpenaiResponses)),
        "Cloudflare AI Gateway responses 模型必须显式声明 openai-responses"
    );
}

#[test]
fn preset_auth_methods_match_opencode() {
    let opencode = find_preset("opencode").expect("opencode");
    assert_eq!(opencode.name, "OpenCode Zen");
    let oauth = opencode
        .oauth_methods()
        .next()
        .and_then(PresetAuthMethod::oauth_flow)
        .expect("opencode oauth");
    assert!(matches!(
        oauth,
        OAuthFlowKind::DeviceCode {
            device_code_url: "https://opencode.ai/auth/device/code",
            token_url: "https://opencode.ai/auth/device/token",
            client_id: "opencode-cli",
            scope: "model.request",
        }
    ));
    assert!(opencode.api_key_method().is_some());

    let openai = find_preset("openai").expect("openai");
    let flows: Vec<OAuthFlowKind> = openai
        .oauth_methods()
        .filter_map(PresetAuthMethod::oauth_flow)
        .collect();
    assert!(flows.iter().any(|flow| matches!(
        flow,
        OAuthFlowKind::BrowserPkce {
            fixed_port: Some(1455),
            client_id: "app_EMoamEEZ73f0CkXaXp7hrann",
            ..
        }
    )));
    assert!(flows.iter().any(|flow| matches!(
        flow,
        OAuthFlowKind::DeviceCode {
            device_code_url: "https://auth.openai.com/api/accounts/deviceauth/usercode",
            token_url: "https://auth.openai.com/api/accounts/deviceauth/token",
            ..
        }
    )));

    let copilot = find_preset("github-copilot").expect("copilot");
    let method = copilot.methods.first().expect("copilot method");
    assert_eq!(method.id(), "device");
    assert_eq!(method.form().len(), 2);
    assert_eq!(method.form()[0].key, "deploymentType");
    assert_eq!(
        method.form()[1].when_eq,
        Some(("deploymentType", "enterprise"))
    );
    assert!(
        method.form()[0]
            .options
            .iter()
            .any(|option| option.value == "enterprise")
    );

    for id in ["xai", "poe", "gitlab", "digitalocean"] {
        assert!(
            find_preset(id).is_some_and(PresetProviderSpec::supports_oauth),
            "{id}"
        );
    }
    assert_eq!(
        find_preset("gitlab")
            .expect("gitlab")
            .method("pkce")
            .expect("pkce")
            .form()[0]
            .key,
        "instanceUrl"
    );
    assert_eq!(
        find_preset("cloudflare-ai-gateway")
            .expect("cf")
            .api_key_method()
            .expect("cf key")
            .form()
            .len(),
        2
    );
    assert_eq!(
        find_preset("azure")
            .expect("azure")
            .api_key_method()
            .expect("azure key")
            .form()
            .iter()
            .map(|field| field.key)
            .collect::<Vec<_>>(),
        vec!["resourceName", "baseUrl"]
    );
}

#[test]
fn default_models_for_unknown_preset_is_empty() {
    assert!(default_models_for_preset("not-a-preset").is_empty());
}

#[test]
fn resolve_base_url_expands_form_templates() {
    let azure = find_preset("azure").expect("azure");
    let answers = BTreeMap::from([("resourceName".to_string(), "my-models".to_string())]);
    assert_eq!(
        resolve_preset_base_url(azure, &answers, None),
        "https://my-models.openai.azure.com/openai/v1"
    );

    let cloudflare = find_preset("cloudflare-ai-gateway").expect("cf");
    let answers = BTreeMap::from([
        ("accountId".to_string(), "abc".to_string()),
        ("gatewayId".to_string(), "gw".to_string()),
    ]);
    assert_eq!(
        resolve_preset_base_url(cloudflare, &answers, None),
        "https://gateway.ai.cloudflare.com/v1/abc/gw/openai"
    );

    let gitlab = find_preset("gitlab").expect("gitlab");
    let answers = BTreeMap::from([(
        "instanceUrl".to_string(),
        "https://git.example.com/".to_string(),
    )]);
    assert_eq!(
        resolve_preset_base_url(gitlab, &answers, None),
        "https://git.example.com/api/v4"
    );

    let answers = BTreeMap::from([("instanceUrl".to_string(), "git.example.com".to_string())]);
    assert_eq!(
        resolve_preset_base_url(gitlab, &answers, None),
        "https://git.example.com/api/v4"
    );

    let copilot = find_preset("github-copilot").expect("copilot");
    let answers = BTreeMap::from([
        ("deploymentType".to_string(), "enterprise".to_string()),
        (
            "enterpriseUrl".to_string(),
            "https://company.ghe.com/path".to_string(),
        ),
    ]);
    assert_eq!(
        resolve_preset_base_url(copilot, &answers, None),
        "https://api.company.ghe.com"
    );
}

#[test]
fn resolve_base_url_prefers_explicit_override() {
    let openai = find_preset("openai").expect("openai");
    let answers = BTreeMap::from([(
        "baseUrl".to_string(),
        "https://proxy.invalid/v1".to_string(),
    )]);
    assert_eq!(
        resolve_preset_base_url(openai, &answers, Some("https://override.invalid/v1/")),
        "https://override.invalid/v1"
    );
    assert_eq!(
        resolve_preset_base_url(openai, &answers, None),
        "https://proxy.invalid/v1"
    );
}
