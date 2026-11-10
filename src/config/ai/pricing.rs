//! 模型 Token 单价（`ModelPricing`）与供应商可读名称解析。

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use super::{AiSettings, ModelConfig, ProviderConfig, presets};

/// 模型 Token 单价（单位：USD / 1,000,000 tokens）。
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelPricing {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none", alias = "cache_read")]
    pub cache_read: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none", alias = "cache_write")]
    pub cache_write: Option<f64>,
}

impl ModelPricing {
    /// 是否至少配置了一项有效单价。
    pub fn has_any(&self) -> bool {
        self.input.is_some()
            || self.output.is_some()
            || self.cache_read.is_some()
            || self.cache_write.is_some()
    }

    /// 是否至少有一项大于 0 的正数单价（用于过滤免费/聚合渠道的 0 单价占位）。
    pub fn has_positive(&self) -> bool {
        self.input.unwrap_or(0.0) > 0.0
            || self.output.unwrap_or(0.0) > 0.0
            || self.cache_read.unwrap_or(0.0) > 0.0
            || self.cache_write.unwrap_or(0.0) > 0.0
    }

    /// 从 JSON Value（支持 camelCase 与 snake_case）解析单价，过滤负数与非有限值。
    pub fn from_json_value(value: &serde_json::Value) -> Option<Self> {
        let obj = value.as_object()?;
        let non_neg = |key1: &str, key2: &str| -> Option<f64> {
            obj.get(key1)
                .or_else(|| obj.get(key2))
                .and_then(serde_json::Value::as_f64)
                .filter(|v| v.is_finite() && *v >= 0.0)
        };
        let pricing = Self {
            input: non_neg("input", "input"),
            output: non_neg("output", "output"),
            cache_read: non_neg("cacheRead", "cache_read"),
            cache_write: non_neg("cacheWrite", "cache_write"),
        };
        pricing.has_any().then_some(pricing)
    }

    /// 转为标准 camelCase JSON Value。
    pub fn to_json_value(self) -> Option<serde_json::Value> {
        if !self.has_any() {
            return None;
        }
        serde_json::to_value(self).ok()
    }
}

impl ModelConfig {
    /// 解析模型配置的 Token 单价（USD / 1,000,000 tokens）。
    pub fn parsed_pricing(&self) -> Option<ModelPricing> {
        self.pricing
            .as_ref()
            .and_then(ModelPricing::from_json_value)
    }
}

/// 判断某供应商条目是否为未修改名称的内置预设（避免将 `anthropic` 重写为 `Anthropic` 造成新老数据分裂）。
fn is_unmodified_preset(key: &str, provider: &ProviderConfig) -> bool {
    presets::find_preset(key)
        .or_else(|| presets::find_preset(&provider.id))
        .is_some_and(|spec| {
            key == spec.id && provider.id == spec.id && provider.name.trim() == spec.name
        })
}

impl AiSettings {
    /// 按优先级查询指定供应商与模型的 Token 单价（USD / 1,000,000 tokens）：
    /// 1. 当前配置中 `provider_id`（键、`id` 或 `name`）下的 `model_id`；
    /// 2. 当前配置中任意供应商下的同名 `model_id`；
    /// 3. 预设目录（含云端同步缓存）中 `provider_id` 或任意供应商下的同名 `model_id`。
    pub fn resolve_model_pricing(&self, provider_id: &str, model_id: &str) -> Option<ModelPricing> {
        let provider_id = provider_id.trim();
        let model_id = model_id.trim();
        if model_id.is_empty() {
            return None;
        }
        let find_in = |models: &BTreeMap<String, ModelConfig>| -> Option<ModelPricing> {
            models
                .get(model_id)
                .and_then(ModelConfig::parsed_pricing)
                .or_else(|| {
                    models
                        .values()
                        .find(|m| m.id == model_id)
                        .and_then(ModelConfig::parsed_pricing)
                })
        };
        if let Some(provider) = self.providers.get(provider_id).or_else(|| {
            self.providers
                .values()
                .find(|p| p.id == provider_id || p.name.trim() == provider_id)
        }) && let Some(pricing) = find_in(&provider.models)
        {
            return Some(pricing);
        }
        if let Some(pricing) = self
            .providers
            .values()
            .filter_map(|p| find_in(&p.models))
            .find(ModelPricing::has_positive)
        {
            return Some(pricing);
        }
        if let Some(pricing) = self.providers.values().find_map(|p| find_in(&p.models)) {
            return Some(pricing);
        }
        presets::catalog_model_pricing(provider_id, model_id)
    }

    /// 将自定义供应商内部标识（如 `provider-alq374`）解析为可读展示名（`provider.name`）。
    ///
    /// 优先匹配 `self.providers` 的键或 `provider.id`；内置未改名预设保持稳定 ID（如 `anthropic`）。
    pub fn resolve_provider_name(&self, provider_id: &str) -> String {
        let trimmed = provider_id.trim();
        if trimmed.is_empty() {
            return "unknown".to_string();
        }
        if let Some(provider) = self.providers.get(trimmed)
            && !is_unmodified_preset(trimmed, provider)
        {
            let name = provider.name.trim();
            if !name.is_empty() {
                return name.to_string();
            }
        }
        for (key, provider) in &self.providers {
            if provider.id == trimmed && !is_unmodified_preset(key, provider) {
                let name = provider.name.trim();
                if !name.is_empty() {
                    return name.to_string();
                }
            }
        }
        trimmed.to_string()
    }

    /// 返回自定义供应商内部 ID 到可读展示名（`provider.name`）的映射表（仅含名称非空且与 ID 不同的项）。
    pub fn provider_name_map(&self) -> BTreeMap<String, String> {
        let mut map = BTreeMap::new();
        for (key, provider) in &self.providers {
            if is_unmodified_preset(key, provider) {
                continue;
            }
            let name = provider.name.trim();
            if !name.is_empty() {
                if key != name {
                    map.insert(key.clone(), name.to_string());
                }
                if provider.id != name {
                    map.insert(provider.id.clone(), name.to_string());
                }
            }
        }
        map
    }
}
