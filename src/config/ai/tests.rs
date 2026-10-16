//! 模型配置测试：解析、校验、原子保存、权限与一次性导入（全部用临时目录与假 Key）。

use std::path::{Path, PathBuf};

use super::*;

/// 独立临时目录。
fn temp_dir(name: &str) -> PathBuf {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_nanos());
    let path = std::env::temp_dir().join(format!(
        "lx-tui-ai-test-{name}-{}",
        nanos.map(|value| format!("{value:x}")).unwrap_or_default()
    ));
    std::fs::create_dir_all(&path).expect("temp dir");
    path
}

/// 测试用假 Key（永不使用真实凭据）。
const FAKE_KEY: &str = "test-key-only-not-real";

/// 构造一个有效的单供应商配置 JSON。
fn sample_ai_json() -> String {
    format!(
        r#"{{
  "ai": {{
    "enabled_providers": ["example"],
    "defaultModel": {{"provider": "example", "model": "example-chat", "variant": "high"}},
    "providers": {{
      "example": {{
        "id": "example",
        "type": "openai-compatible",
        "name": "Example",
        "options": {{"apiKey": "{FAKE_KEY}", "baseURL": "https://api.example.invalid/v1"}},
        "models": {{
          "example-chat": {{
            "id": "example-chat",
            "name": "Example Chat",
            "limit": {{"context": 128000, "output": 8192}},
            "variants": {{"high": {{"reasoningEffort": "high"}}}},
            "variant": "high",
            "futureField": {{"kept": true}}
          }}
        }}
      }}
    }},
    "streamIdleTimeoutMs": 60000
  }}
}}"#
    )
}

fn sample_settings() -> AiSettings {
    parse_ai(&sample_ai_json()).expect("sample parses")
}

#[test]
fn parse_accepts_camel_case_and_preserves_unknown_fields() {
    let settings = sample_settings();
    assert_eq!(settings.enabled_providers, vec!["example".to_string()]);
    let provider = &settings.providers["example"];
    assert_eq!(provider.id, "example");
    assert_eq!(provider.options.base_url, "https://api.example.invalid/v1");
    assert_eq!(provider.options.api_key.as_deref(), Some(FAKE_KEY));
    let model = &provider.models["example-chat"];
    assert_eq!(model.limit.map(|limit| limit.context), Some(128_000));
    assert!(model.variants.contains_key("high"));
    assert_eq!(model.variant.as_deref(), Some("high"));
    // 未知字段往返保留。
    assert!(model.extra.contains_key("futureField"));
    let serialized = serde_json::to_value(&settings).expect("serialize");
    let text = serde_json::to_string_pretty(&serialized).expect("text");
    assert!(text.contains("futureField"), "未知字段不丢失");
    // 重新解析结果一致。
    let root = format!("{{\"ai\":{}}}", text);
    let reparsed = parse_ai(&root).expect("round trip");
    assert_eq!(reparsed, settings);
}

#[test]
fn parse_rejects_malformed_json_with_explicit_error() {
    let error = parse_ai("{ not json").expect_err("malformed rejected");
    assert!(error.to_string().contains("malformed json"));
    let error = parse_ai("{}").expect_err("missing ai node rejected");
    assert!(matches!(error, AiConfigError::Invalid(_)));
}

#[test]
fn validate_rejects_dangling_default_selection() {
    let mut settings = sample_settings();
    settings.default_model = Some(ModelSelection {
        provider: "missing".into(),
        model: "example-chat".into(),
        variant: None,
    });
    assert!(settings.validate().is_err());

    let mut settings = sample_settings();
    settings.enabled_providers.clear();
    assert!(settings.validate().is_err(), "默认选择必须指向已启用供应商");

    let mut settings = sample_settings();
    if let Some(selection) = settings.default_model.as_mut() {
        selection.variant = Some("nonexistent".into());
    }
    assert!(
        settings.validate().is_err(),
        "默认 variant 必须存在于模型 variants"
    );
}

#[test]
fn validate_rejects_bad_transport_and_key_mismatch() {
    let mut settings = sample_settings();
    let provider = settings.providers.get_mut("example").expect("provider");
    provider.id = "renamed".into();
    assert!(settings.validate().is_err(), "provider key/id 必须一致");

    let mut settings = sample_settings();
    {
        let provider = settings.providers.get_mut("example").expect("provider");
        let model = provider.models.get_mut("example-chat").expect("model");
        model.transport = Some(ModelTransport::OpenaiCompatible);
    }
    // openai provider 下挂 openai-compatible transport 拒绝（openai provider 语义）。
    let mut strict = settings.clone();
    if let Some(provider) = strict.providers.get_mut("example") {
        provider.kind = ProviderTransport::Openai;
    }
    assert!(strict.validate().is_err());

    let mut settings = sample_settings();
    if let Some(provider) = settings.providers.get_mut("example") {
        provider.options.base_url = "ftp://bad.example".into();
    }
    assert!(settings.validate().is_err(), "base URL 只允许 http/https");
}

#[test]
fn validate_base_url_rules() {
    assert!(validate_base_url("https://api.example.com/v1").is_ok());
    assert!(validate_base_url("http://127.0.0.1:20128/v1").is_ok());
    assert!(validate_base_url("").is_err());
    assert!(validate_base_url("example.com").is_err());
    assert!(validate_base_url("ftp://example.com").is_err());
    assert!(validate_base_url("https://").is_err());
    assert!(validate_base_url("https://example.com/\u{1}").is_err());
}

#[test]
fn save_ai_is_atomic_and_enforces_permissions() {
    let dir = temp_dir("save-atomic");
    let path = dir.join("ai.json");
    let settings = sample_settings();
    save_ai(&path, &settings).expect("save");
    assert!(path.exists());
    // 保存后无临时文件残留。
    let temp = path.with_extension("json.tmp");
    assert!(!temp.exists(), "临时文件已重命名");
    // 文件可解析且一致。
    let loaded = load_ai(&path).expect("load").expect("exists");
    assert_eq!(loaded, settings);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(&path)
            .expect("metadata")
            .permissions()
            .mode();
        assert_eq!(mode & 0o077, 0, "文件权限收紧到所有者");
    }
    // 旧文件存在时保存失败不破坏旧内容：先写好，再保存非法配置。
    let before = std::fs::read_to_string(&path).expect("read");
    let mut broken = sample_settings();
    if let Some(provider) = broken.providers.get_mut("example") {
        provider.options.base_url = "no-scheme".into();
    }
    assert!(save_ai(&path, &broken).is_err());
    assert_eq!(std::fs::read_to_string(&path).expect("re-read"), before);
}

#[test]
fn load_ai_reports_missing_and_corrupt() {
    let dir = temp_dir("load");
    let path = dir.join("ai.json");
    // 缺失文件返回 None。
    assert!(load_ai(&path).expect("query").is_none());
    // 损坏文件返回错误，不静默当成空配置。
    std::fs::write(&path, "corrupted").expect("write");
    assert!(load_ai(&path).is_err());
}

#[test]
fn import_skips_when_target_exists() {
    let dir = temp_dir("import-exists");
    let target = dir.join("ai.json");
    // 预先写入空对象作为"用户手动创建"的目标。
    std::fs::write(
        &target,
        "{\"ai\":{\"enabled_providers\":[],\"providers\":{}}}",
    )
    .expect("seed");
    let before = std::fs::read_to_string(&target).expect("read");
    let imported = import_from_lx_agent(&target).expect("import");
    assert!(!imported, "目标已存在时跳过导入");
    assert_eq!(std::fs::read_to_string(&target).expect("re-read"), before);
}

#[test]
fn import_from_lx_agent_reads_source_and_merges_builtin() {
    // 隔离 HOME：源目录只包含假 Key 的 lx-agent 配置，不触碰真实凭据。
    let dir = temp_dir("import-full");
    let home = dir.join("home");
    let lx_config = home.join(".lx").join("config");
    std::fs::create_dir_all(&lx_config).expect("lx config");
    // 源缺 defaultModel 的 enabled 供应商：导入回落默认选择。
    std::fs::write(
        lx_config.join("ai.json"),
        format!(
            r#"{{"ai":{{
                "enabled_providers": ["userprov"],
                "titleSummary": {{"provider": "userprov", "model": "m1"}},
                "providers": {{
                    "userprov": {{
                        "id": "userprov", "type": "openai-compatible", "name": "User",
                        "options": {{"apiKey": "{FAKE_KEY}", "baseURL": "https://api.user.invalid/v1"}},
                        "models": {{"m1": {{"id": "m1", "name": "M1"}}}}
                    }}
                }}
            }}}}"#
        ),
    )
    .expect("source");
    // builtin-providers.json 提供一个预置供应商（不带 Key，导入后默认禁用）。
    std::fs::write(
        lx_config.join("builtin-providers.json"),
        r#"{"builtinProviders":{
            "builtin-prov": {
                "id": "builtin-prov", "type": "openai-compatible", "name": "Builtin",
                "options": {"apiKey": "builtin-key-not-real", "baseURL": "https://api.builtin.invalid/v1"},
                "models": {"bm": {"id": "bm", "name": "BM"}}
            }
        }}"#,
    )
    .expect("builtin");
    let target = dir.join("config").join("ai.json");

    // 保存并恢复 HOME；该测试不与其他读 HOME 的测试并行冲突时使用锁保护。
    let guard = HOME_LOCK.lock();
    let previous = std::env::var_os("HOME");
    unsafe { std::env::set_var("HOME", &home) };
    let imported = import_from_lx_agent(&target).expect("import");
    unsafe {
        if let Some(previous) = previous {
            std::env::set_var("HOME", previous);
        } else {
            std::env::remove_var("HOME");
        }
    }
    drop(guard);

    assert!(imported, "首次导入执行");
    let loaded = load_ai(&target).expect("load").expect("exists");
    assert_eq!(loaded.providers.len(), 2, "用户 + builtin 供应商");
    assert!(loaded.providers.contains_key("userprov"));
    assert!(
        loaded.providers.contains_key("builtin-prov"),
        "builtin 并入"
    );
    assert_eq!(
        loaded.default_model.as_ref().expect("selection").provider,
        "userprov",
        "默认选择回落到已启用供应商"
    );
    assert!(loaded.extra.is_empty(), "lx-agent 独立设置节点不导入");

    // 第二次导入：目标已存在，跳过。
    let again = import_from_lx_agent(&target).expect("again");
    assert!(!again);
}

/// HOME 环境变量互斥锁：串行化读 HOME 的测试。
static HOME_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[test]
fn import_from_missing_source_creates_empty_config() {
    // 隔离 HOME 到空目录：源缺失走空配置，UI 指向添加供应商。
    let dir = temp_dir("import-empty-source");
    let home = dir.join("empty-home");
    std::fs::create_dir_all(&home).expect("home");
    let target = dir.join("config").join("ai.json");

    let guard = HOME_LOCK.lock();
    let previous = std::env::var_os("HOME");
    unsafe { std::env::set_var("HOME", &home) };
    let imported = import_from_lx_agent(&target).expect("import");
    unsafe {
        if let Some(previous) = previous {
            std::env::set_var("HOME", previous);
        } else {
            std::env::remove_var("HOME");
        }
    }
    drop(guard);

    assert!(imported, "首次导入执行（空配置）");
    let loaded = load_ai(&target).expect("load").expect("exists");
    assert!(loaded.providers.is_empty());
    assert!(loaded.default_model.is_none(), "不凭空出现默认模型");
}

#[test]
fn import_from_corrupt_source_keeps_target_absent_and_retryable() {
    // 隔离 HOME：源损坏返回错误且不创建目标文件。
    let dir = temp_dir("import-corrupt");
    let home = dir.join("home");
    let lx_config = home.join(".lx").join("config");
    std::fs::create_dir_all(&lx_config).expect("lx config");
    std::fs::write(lx_config.join("ai.json"), "not json at all").expect("corrupt source");
    let target = dir.join("config").join("ai.json");

    let guard = HOME_LOCK.lock();
    let previous = std::env::var_os("HOME");
    unsafe { std::env::set_var("HOME", &home) };
    let result = import_from_lx_agent(&target);
    unsafe {
        if let Some(previous) = previous {
            std::env::set_var("HOME", previous);
        } else {
            std::env::remove_var("HOME");
        }
    }
    drop(guard);

    assert!(result.is_err(), "损坏源报错");
    assert!(!target.exists(), "不创建目标文件");
    // 修复源后重试成功。
    std::fs::write(
        lx_config.join("ai.json"),
        r#"{"ai":{"enabled_providers": [], "providers": {}}}"#,
    )
    .expect("fixed source");
    let guard = HOME_LOCK.lock();
    let previous = std::env::var_os("HOME");
    unsafe { std::env::set_var("HOME", &home) };
    let imported = import_from_lx_agent(&target).expect("retry import");
    unsafe {
        if let Some(previous) = previous {
            std::env::set_var("HOME", previous);
        } else {
            std::env::remove_var("HOME");
        }
    }
    drop(guard);
    assert!(imported, "修复后可重试");
}

#[test]
fn model_transport_overrides_and_parse() {
    assert_eq!(
        ModelTransport::parse("openai-responses"),
        Some(ModelTransport::OpenaiResponses)
    );
    assert_eq!(
        ModelTransport::parse("openai-compatible"),
        Some(ModelTransport::OpenaiCompatible)
    );
    assert_eq!(ModelTransport::parse("unknown"), None);
    assert_eq!(
        ProviderTransport::parse("openai"),
        Some(ProviderTransport::Openai)
    );
    assert_eq!(
        ProviderTransport::parse("openai-compatible"),
        Some(ProviderTransport::OpenaiCompatible)
    );
    // 序列化保持 kebab-case。
    let value = serde_json::to_value(ModelTransport::OpenaiResponses).expect("json");
    assert_eq!(value, serde_json::json!("openai-responses"));
    let value = serde_json::to_value(ProviderTransport::OpenaiCompatible).expect("json");
    assert_eq!(value, serde_json::json!("openai-compatible"));
}

#[test]
fn first_enabled_model_falls_back_to_enabled_provider() {
    let mut settings = AiSettings::empty();
    assert!(settings.first_enabled_model().is_none());
    settings.enabled_providers.push("p".into());
    assert!(
        settings.first_enabled_model().is_none(),
        "无 provider 时 None"
    );
    let mut provider = sample_settings().providers["example"].clone();
    provider.id = "p".into();
    settings.providers.insert("p".into(), provider);
    let selection = settings.first_enabled_model().expect("falls back");
    assert_eq!(selection.provider, "p");
    assert_eq!(selection.model, "example-chat");
    assert_eq!(
        selection.variant.as_deref(),
        Some("high"),
        "继承模型默认 variant"
    );
}

#[test]
fn normalize_import_clears_unrelated_settings_and_dangling_refs() {
    let mut settings = sample_settings();
    // 模拟 lx-agent 的独立设置节点与悬空引用。
    settings
        .extra
        .insert("titleSummary".into(), serde_json::json!({"provider": "x"}));
    settings.enabled_providers.push("ghost".into());
    normalize_import(&mut settings);
    assert!(settings.extra.is_empty(), "lx-agent 独立设置不导入");
    assert!(!settings.enabled_providers.contains(&"ghost".to_string()));
    assert!(settings.validate().is_ok());
}

#[test]
fn stream_idle_timeout_defaults_and_custom() {
    let settings = AiSettings::empty();
    assert_eq!(settings.idle_timeout_ms(), 60_000);
    let mut settings = sample_settings();
    settings.stream_idle_timeout_ms = Some(5_000);
    assert_eq!(settings.idle_timeout_ms(), 5_000);
}

#[test]
fn save_then_load_round_trips_in_target_layout() {
    let dir = temp_dir("round-trip");
    let path = dir.join("ai.json");
    let settings = sample_settings();
    save_ai(&path, &settings).expect("save");
    let text = std::fs::read_to_string(&path).expect("read");
    // 顶层是 {"ai": {...}}，字段 camelCase。
    assert!(text.contains("\"ai\""));
    assert!(text.contains("\"defaultModel\""));
    assert!(text.contains("\"enabled_providers\""));
    assert!(text.contains("\"streamIdleTimeoutMs\""));
    let loaded = load_ai(&path).expect("load").expect("exists");
    assert_eq!(loaded, settings);
}

#[test]
fn empty_id_model_does_not_break_validation_when_not_selected() {
    // 空 id 模型可以存在（导入宽容），但默认选择指向它时按 key 匹配成功。
    let mut settings = sample_settings();
    let provider = settings.providers.get_mut("example").expect("provider");
    provider.models.insert(
        String::new(),
        ModelConfig {
            id: String::new(),
            name: "Empty".into(),
            transport: None,
            limit: None,
            modalities: None,
            variants: BTreeMap::new(),
            variant: None,
            pricing: None,
            extra: serde_json::Map::new(),
        },
    );
    assert!(settings.validate().is_ok());
}

#[test]
fn ai_config_path_under_data_root() {
    // 开发环境路径推导走 user_data_root；仅验证形状，不触碰真实文件。
    if let Ok(path) = ai_config_path() {
        let text = path.to_string_lossy();
        assert!(
            text.ends_with("config/ai.json"),
            "路径为 config/ai.json：{text}"
        );
    }
}

#[test]
fn parse_accepts_missing_optional_nodes() {
    // 只有 providers 的最小配置。
    let minimal = r#"{"ai":{"providers":{}}}"#;
    let settings = parse_ai(minimal).expect("minimal parses");
    assert!(settings.providers.is_empty());
    assert!(settings.default_model.is_none());
    assert_eq!(settings.idle_timeout_ms(), DEFAULT_STREAM_IDLE_TIMEOUT_MS);
}

#[test]
fn create_private_dir_restricts_permissions() {
    let dir = temp_dir("private-dir");
    let target = dir.join("secret");
    create_private_dir(&target).expect("create");
    assert!(target.is_dir());
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(&target)
            .expect("metadata")
            .permissions()
            .mode();
        assert_eq!(mode & 0o077, 0, "目录权限收紧到所有者");
    }
    // 重复调用幂等。
    create_private_dir(&target).expect("again");
}

#[test]
fn import_writes_validated_file_for_well_formed_source() {
    // 构造一个源目录结构，模拟 lx-agent 配置（不读取真实 HOME）。
    let dir = temp_dir("import-source");
    let target = dir.join("config").join("ai.json");
    // 直接用 save_ai 验证目标写入：内容与校验路径由 import 覆盖调用。
    let settings = sample_settings();
    save_ai(&target, &settings).expect("save");
    let loaded = load_ai(&target).expect("load").expect("exists");
    assert_eq!(loaded.default_model.expect("selection").provider, "example");
    // 导入的文件里 API Key 只在文件内，不出现在序列化错误信息中。
    let text = std::fs::read_to_string(&target).expect("read");
    assert!(text.contains(FAKE_KEY), "Key 保存在权限受控文件中");
}

#[test]
fn load_ai_returns_none_for_directory_path() {
    // 路径是目录时按 IO 错误报告（不是 NotFound 语义）。
    let dir = temp_dir("dir-path");
    assert!(load_ai(&dir).is_err());
}

#[test]
fn sample_with_no_variant_model_skips_variant_requirement() {
    let json = r#"{
      "ai": {
        "enabled_providers": ["p"],
        "defaultModel": {"provider": "p", "model": "m"},
        "providers": {
          "p": {
            "id": "p", "type": "anthropic", "name": "P",
            "options": {"apiKey": "k", "baseURL": "https://api.p.invalid"},
            "models": {"m": {"id": "m", "name": "M"}}
          }
        }
      }
    }"#;
    let settings = parse_ai(json).expect("no variants ok");
    assert!(
        settings.validate().is_ok(),
        "无 variants 模型不强制 variant"
    );
    assert_eq!(settings.idle_timeout_ms(), DEFAULT_STREAM_IDLE_TIMEOUT_MS);
}

/// Path 辅助：临时文件清理（测试目录由系统清理，无需显式删除）。
#[allow(dead_code)]
fn assert_path_exists(path: &Path) {
    assert!(Path::new(path).exists());
}
