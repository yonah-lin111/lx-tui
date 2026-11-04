//! 子 agent 模型与思考等级三级优先级解析：
//! 1. `spawn_agent` 显式参数（`model` / `reasoning_effort`）
//! 2. 角色默认配置（`subagents.json` 中的 `model`）
//! 3. 父会话当前冻结模型（`parent_frozen`）

use crate::app::state::{FrozenModel, ModelsSnapshot};
use crate::config::ai::ModelSelection;

/// 解析后的子 agent 模型快照及是否固定（显式覆写或角色绑定时为 `true`）。
pub(crate) struct ResolvedSubagentModel {
    pub frozen: FrozenModel,
    pub pinned: bool,
    pub model_override_label: Option<String>,
    pub effort_override_label: Option<String>,
}

/// 按三级优先级解析子 agent 的 `FrozenModel`；模型不存在、未启用、缺 Key 或不支持指定 `reasoning_effort` 时返回明确英文错误。
pub(crate) fn resolve_subagent_model(
    models: &ModelsSnapshot,
    parent_frozen: &FrozenModel,
    role_default: Option<&ModelSelection>,
    model_override: Option<&str>,
    effort_override: Option<&str>,
) -> Result<ResolvedSubagentModel, String> {
    let explicit_model = model_override.map(str::trim).filter(|s| !s.is_empty());
    let explicit_effort = effort_override.map(str::trim).filter(|s| !s.is_empty());

    // 若既无显式参数也无角色默认模型，直接继承父会话已冻结模型（零额外校验，兼容测试直注 FrozenModel）。
    if explicit_model.is_none() && explicit_effort.is_none() && role_default.is_none() {
        return Ok(ResolvedSubagentModel {
            frozen: parent_frozen.clone(),
            pinned: false,
            model_override_label: None,
            effort_override_label: None,
        });
    }

    let mut selection = if let Some(raw_model) = explicit_model {
        parse_model_override_str(models, &parent_frozen.provider, raw_model)?
    } else if let Some(role_sel) = role_default {
        validate_role_model_selection(models, role_sel)?
    } else {
        ModelSelection {
            provider: parent_frozen.provider.clone(),
            model: parent_frozen.model.clone(),
            variant: parent_frozen.variant.clone(),
        }
    };

    if let Some(effort) = explicit_effort {
        // 若仍在父模型上仅覆盖 reasoning_effort，且 models.settings 中有该模型配置，校验 variants；
        // 若测试环境未填充 models.settings 但目标是当前父模型，则仅在模型存在时严格校验 variants。
        if let Some(provider_cfg) = models.settings.providers.get(&selection.provider) {
            let model_cfg = provider_cfg.models.get(&selection.model).ok_or_else(|| {
                format!(
                    "model '{}/{}' is not found",
                    selection.provider, selection.model
                )
            })?;
            if !model_cfg.variants.contains_key(effort) {
                let mut supported: Vec<&str> =
                    model_cfg.variants.keys().map(String::as_str).collect();
                supported.sort_unstable();
                return Err(format!(
                    "reasoning_effort '{}' is not supported by model '{}/{}' (available: {})",
                    effort,
                    selection.provider,
                    selection.model,
                    if supported.is_empty() {
                        "none".to_string()
                    } else {
                        supported.join(", ")
                    }
                ));
            }
        } else {
            return Err(format!(
                "provider '{}' is not configured for reasoning_effort override '{}'",
                selection.provider, effort
            ));
        }
        selection.variant = Some(effort.to_string());
    } else if explicit_model.is_some() {
        selection = models.normalize_selection(selection);
    }

    let frozen = FrozenModel::resolve(models, &selection).ok_or_else(|| {
        format!(
            "model '{}/{}' is unavailable (provider disabled or API key missing)",
            selection.provider, selection.model
        )
    })?;

    let model_differs =
        frozen.provider != parent_frozen.provider || frozen.model != parent_frozen.model;
    let effort_differs = frozen.variant != parent_frozen.variant;
    let model_override_label = if explicit_model.is_some() || model_differs {
        Some(format!("{}/{}", frozen.provider, frozen.model))
    } else {
        None
    };
    let effort_override_label =
        if explicit_effort.is_some() || (effort_differs && role_default.is_some()) {
            frozen.variant.clone()
        } else {
            None
        };

    Ok(ResolvedSubagentModel {
        frozen,
        pinned: true,
        model_override_label,
        effort_override_label,
    })
}

/// 解析 `"provider/model"` 或同 provider 下 `"model"` 简写。
fn parse_model_override_str(
    models: &ModelsSnapshot,
    parent_provider: &str,
    raw: &str,
) -> Result<ModelSelection, String> {
    if let Some((prov, rest)) = raw.split_once('/') {
        let prov = prov.trim();
        let rest = rest.trim();
        if !prov.is_empty()
            && !rest.is_empty()
            && let Some(p_cfg) = models.settings.providers.get(prov)
            && p_cfg.models.contains_key(rest)
        {
            if !models.settings.enabled_providers.iter().any(|p| p == prov) {
                return Err(format!("provider '{prov}' is disabled"));
            }
            return Ok(models.normalize_selection(ModelSelection {
                provider: prov.to_string(),
                model: rest.to_string(),
                variant: None,
            }));
        }
    }

    // 1. 优先在父会话同一 provider 下精确匹配 model id（兼容含 `/` 的模型 ID 如 `gemini/gemini-4-argon`）。
    if let Some(p_cfg) = models.settings.providers.get(parent_provider)
        && p_cfg.models.contains_key(raw)
    {
        if !models
            .settings
            .enabled_providers
            .iter()
            .any(|p| p == parent_provider)
        {
            return Err(format!("provider '{parent_provider}' is disabled"));
        }
        return Ok(models.normalize_selection(ModelSelection {
            provider: parent_provider.to_string(),
            model: raw.to_string(),
            variant: None,
        }));
    }

    // 2. 在所有已启用 providers 中查找唯一匹配的 model id。
    let mut matches = Vec::new();
    for prov in &models.settings.enabled_providers {
        if let Some(p_cfg) = models.settings.providers.get(prov)
            && p_cfg.models.contains_key(raw)
        {
            matches.push(prov.clone());
        }
    }
    match matches.len() {
        1 => Ok(models.normalize_selection(ModelSelection {
            provider: matches.remove(0),
            model: raw.to_string(),
            variant: None,
        })),
        0 => Err(format!(
            "model '{raw}' is not found or its provider is disabled"
        )),
        _ => Err(format!(
            "model '{raw}' is ambiguous across enabled providers ({}); specify 'provider/model'",
            matches.join(", ")
        )),
    }
}

fn validate_role_model_selection(
    models: &ModelsSnapshot,
    sel: &ModelSelection,
) -> Result<ModelSelection, String> {
    let prov = sel.provider.trim();
    let model = sel.model.trim();
    if !models.settings.enabled_providers.iter().any(|p| p == prov) {
        return Err(format!("role default provider '{prov}' is not enabled"));
    }
    let p_cfg = models
        .settings
        .providers
        .get(prov)
        .ok_or_else(|| format!("role default provider '{prov}' is not found"))?;
    let m_cfg = p_cfg
        .models
        .get(model)
        .ok_or_else(|| format!("role default model '{prov}/{model}' is not found"))?;
    if let Some(variant) = sel
        .variant
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        && !m_cfg.variants.contains_key(variant)
    {
        return Err(format!(
            "role default reasoning_effort '{variant}' is not supported by model '{prov}/{model}'"
        ));
    }
    Ok(models.normalize_selection(ModelSelection {
        provider: prov.to_string(),
        model: model.to_string(),
        variant: sel.variant.clone(),
    }))
}
