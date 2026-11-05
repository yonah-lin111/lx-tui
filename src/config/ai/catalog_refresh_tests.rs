//! 云端模型目录解析测试：models.dev v2/v1、deprecated 过滤与推理等级。

use super::*;

const V2_CATALOG: &str = r#"{
  "openai": {
    "id": "openai",
    "name": "OpenAI",
    "env": ["OPENAI_API_KEY"],
    "models": {
      "gpt-5": {
        "id": "gpt-5",
        "type": "chat",
        "name": "GPT-5",
        "modalities": {"input": ["text", "image"], "output": ["text"]},
        "capabilities": {"reasoning": true},
        "limit": {"context": 400000, "output": 128000}
      },
      "text-embedding-3": {
        "id": "text-embedding-3",
        "type": "embedding",
        "name": "Embedding 3"
      },
      "retired": {
        "id": "retired",
        "type": "chat",
        "name": "Retired",
        "status": "deprecated"
      }
    }
  },
  "anthropic": {
    "id": "anthropic",
    "name": "Anthropic",
    "models": {
      "claude-opus-4": {
        "id": "claude-opus-4",
        "type": "chat",
        "name": "Claude Opus 4",
        "capabilities": {"reasoning": {"supported": true, "effort": ["low", "high"]}},
        "limit": {"context": 200000}
      }
    }
  }
}"#;

const V1_CATALOG: &str = r#"{
  "openai": {
    "id": "openai",
    "name": "OpenAI",
    "npm": "@ai-sdk/openai",
    "models": {
      "o3-mini": {
        "id": "o3-mini",
        "name": "o3-mini",
        "reasoning": true,
        "limit": {"context": 200000, "output": 100000}
      },
      "gpt-4o": {
        "id": "gpt-4o",
        "name": "GPT-4o",
        "reasoning": false
      }
    }
  }
}"#;

#[test]
fn parses_v2_catalog_with_limits_modalities_and_variants() {
    let catalog = parse_models_dev_catalog(V2_CATALOG).expect("v2 parses");
    let openai = &catalog["openai"];
    assert_eq!(openai.len(), 1, "deprecated and non-chat models filtered");
    let gpt5 = &openai["gpt-5"];
    assert_eq!(gpt5.name, "GPT-5");
    assert_eq!(gpt5.limit.map(|limit| limit.context), Some(400_000));
    assert_eq!(gpt5.limit.map(|limit| limit.output), Some(128_000));
    assert_eq!(
        gpt5.modalities
            .as_ref()
            .and_then(|value| value.get("input")),
        Some(&serde_json::json!(["text", "image"]))
    );
    assert_eq!(
        gpt5.variants.keys().map(String::as_str).collect::<Vec<_>>(),
        vec!["high", "low", "medium"]
    );

    let opus = &catalog["anthropic"]["claude-opus-4"];
    assert!(opus.variants.contains_key("high"));
    assert_eq!(opus.limit.map(|limit| limit.output), Some(0));
}

#[test]
fn parses_v1_catalog_with_reasoning_flag() {
    let catalog = parse_models_dev_catalog(V1_CATALOG).expect("v1 parses");
    let openai = &catalog["openai"];
    assert!(openai["o3-mini"].variants.contains_key("medium"));
    assert!(openai["gpt-4o"].variants.is_empty());
    assert_eq!(
        openai["o3-mini"].limit.map(|limit| limit.context),
        Some(200_000)
    );
}

#[test]
fn rejects_malformed_catalog_json() {
    assert!(parse_models_dev_catalog("{not json").is_err());
    assert!(parse_models_dev_catalog("[]").is_err());
}

#[test]
fn cache_path_lives_under_config() {
    if let Ok(path) = catalog_cache_path() {
        assert!(
            path.to_string_lossy()
                .ends_with("config/models_catalog_cache.json"),
            "{}",
            path.display()
        );
    }
}
