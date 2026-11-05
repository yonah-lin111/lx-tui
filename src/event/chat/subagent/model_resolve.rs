//! 子 agent 模型与思考等级三级优先级解析：
//! 1. `spawn_agent` 显式参数（`model` / `reasoning_effort`）
//! 2. 角色默认配置（`subagents.json` 中的 `model`）
//! 3. 父会话当前冻结模型（`parent_frozen`）

use crate::app::state::{AppState, FrozenModel, ModelsSnapshot};
use crate::config::ai::ModelSelection;
use crate::config::subagents::BaseRole;
use crate::db::chat::SubagentRecord;
use crate::event::chat::ChatStreams;

/// 恢复子 agent 持久化的模型快照（TUI/exec 重启后子会话仍用自己的模型）。
///
/// 仅当记录中有完整 `(provider, model)` 且与父会话当前模型不同才返回解析结果；
/// 解析失败（provider 关闭/缺 Key/模型下线）由调用方回退到父会话模型。
pub(crate) fn recover_persisted_child_model(
    models: &ModelsSnapshot,
    parent_frozen: &FrozenModel,
    record: &SubagentRecord,
) -> Option<FrozenModel> {
    let provider = record.model_provider.trim();
    let model = record.model_id.trim();
    if provider.is_empty() || model.is_empty() {
        return None;
    }
    let variant = record
        .model_variant
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty());
    if provider == parent_frozen.provider
        && model == parent_frozen.model
        && variant == parent_frozen.variant.as_deref()
    {
        return None;
    }
    let selection = models.normalize_selection(ModelSelection {
        provider: provider.to_string(),
        model: model.to_string(),
        variant: variant.map(str::to_string),
    });
    FrozenModel::resolve(models, &selection)
}

/// 解析后的子 agent 模型快照及是否固定（显式覆写或角色绑定时为 `true`）。
pub(crate) struct ResolvedSubagentModel {
    pub frozen: FrozenModel,
    pub pinned: bool,
    pub model_override_label: Option<String>,
    pub effort_override_label: Option<String>,
}

/// 按三级优先级解析子 agent 的 `FrozenModel`；模型不存在、未启用、缺 Key 或不支持指定 `reasoning_effort` 时返回明确英文错误。
///
/// `lock_model == true` 且角色配置了默认模型时，调用方传入的 `model_override` /
/// `effort_override` 一律静默忽略；`monitor` 角色未显式指定思考等级时优先选 `low`。
pub(crate) fn resolve_subagent_model(
    models: &ModelsSnapshot,
    parent_frozen: &FrozenModel,
    base_role: BaseRole,
    role_default: Option<&ModelSelection>,
    lock_model: bool,
    model_override: Option<&str>,
    effort_override: Option<&str>,
) -> Result<ResolvedSubagentModel, String> {
    let locked = lock_model && role_default.is_some();
    let explicit_model = if locked {
        None
    } else {
        model_override.map(str::trim).filter(|s| !s.is_empty())
    };
    let explicit_effort = if locked {
        None
    } else {
        effort_override.map(str::trim).filter(|s| !s.is_empty())
    };

    // 若既无显式参数也无角色默认模型，直接继承父会话已冻结模型（零额外校验，兼容测试直注 FrozenModel）。
    if explicit_model.is_none() && explicit_effort.is_none() && role_default.is_none() {
        let mut selection = ModelSelection {
            provider: parent_frozen.provider.clone(),
            model: parent_frozen.model.clone(),
            variant: parent_frozen.variant.clone(),
        };
        if !apply_monitor_default_effort(
            models,
            &mut selection,
            base_role,
            role_default,
            explicit_effort,
        ) {
            return Ok(ResolvedSubagentModel {
                frozen: parent_frozen.clone(),
                pinned: false,
                model_override_label: None,
                effort_override_label: None,
            });
        }
        let frozen = FrozenModel::resolve(models, &selection).ok_or_else(|| {
            format!(
                "model '{}/{}' is unavailable (provider disabled or API key missing)",
                selection.provider, selection.model
            )
        })?;
        let effort_override_label = (frozen.variant != parent_frozen.variant)
            .then(|| frozen.variant.clone())
            .flatten();
        return Ok(ResolvedSubagentModel {
            frozen,
            pinned: true,
            model_override_label: None,
            effort_override_label,
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
    let monitor_default_low = apply_monitor_default_effort(
        models,
        &mut selection,
        base_role,
        role_default,
        explicit_effort,
    );

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
    let effort_override_label = if explicit_effort.is_some()
        || (effort_differs && (role_default.is_some() || monitor_default_low))
    {
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

/// `monitor` 未显式指定思考等级且角色未配置 `variant` 时，目标模型支持 `"low"` 则默认选中。
fn apply_monitor_default_effort(
    models: &ModelsSnapshot,
    selection: &mut ModelSelection,
    base_role: BaseRole,
    role_default: Option<&ModelSelection>,
    explicit_effort: Option<&str>,
) -> bool {
    if base_role != BaseRole::Monitor
        || explicit_effort.is_some()
        || role_default
            .and_then(|sel| sel.variant.as_deref())
            .is_some()
        || selection.variant.as_deref() == Some("low")
    {
        return false;
    }
    let supports_low = models
        .settings
        .providers
        .get(&selection.provider)
        .and_then(|provider| provider.models.get(&selection.model))
        .is_some_and(|model| model.variants.contains_key("low"));
    if !supports_low {
        return false;
    }
    selection.variant = Some("low".to_string());
    true
}

/// 解析 `"provider/model"` 或同 provider 下 `"model"` 简写。
///
/// 模型 ID 自身可能含 `/`（如 `gemini/gemini-4-argon`）：`provider/model` 拆分失败时，
/// 先把整串当作模型 ID 在父 provider / 唯一启用的 provider 下查找，再返回 provider 错误。
fn parse_model_override_str(
    models: &ModelsSnapshot,
    parent_provider: &str,
    raw: &str,
) -> Result<ModelSelection, String> {
    let mut provider_error: Option<String> = None;
    if let Some((prov, rest)) = raw.split_once('/') {
        let prov = prov.trim();
        let rest = rest.trim();
        if !prov.is_empty()
            && !rest.is_empty()
            && let Some(p_cfg) = models.settings.providers.get(prov)
        {
            if !models.settings.enabled_providers.iter().any(|p| p == prov) {
                provider_error = Some(format!("provider '{prov}' is disabled"));
            } else if p_cfg.models.contains_key(rest) {
                return Ok(models.normalize_selection(ModelSelection {
                    provider: prov.to_string(),
                    model: rest.to_string(),
                    variant: None,
                }));
            } else {
                provider_error = Some(format!(
                    "model '{rest}' is not defined on provider '{prov}'"
                ));
            }
        }
    }

    // 1. 优先在父会话同一 provider 下精确匹配 model id（兼容含 `/` 的模型 ID）。
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
        0 => Err(provider_error
            .unwrap_or_else(|| format!("model '{raw}' is not found or its provider is disabled"))),
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

/// 解析子会话本轮模型：已固定的子模型 > 数据库持久化的子 agent 模型（重新固定）> 父模型。
///
/// 只有内存里没有该子会话的冻结快照（进程重启/面板重开）时才从数据库恢复；
/// 同进程内仍沿用「父窗格最新模型优先」的继承规则，避免把未固定的子 agent 钉在 spawn 旧模型上。
pub(super) fn child_frozen_for(
    state: &AppState,
    streams: &mut ChatStreams,
    record: &SubagentRecord,
    parent_frozen: &crate::app::state::FrozenModel,
) -> crate::app::state::FrozenModel {
    if let Some(pinned) = streams
        .pinned_subagent_models
        .get(&record.child_session_id)
        .cloned()
    {
        return pinned;
    }
    let has_cached_snapshot = streams.frozen.contains_key(&record.child_session_id);
    if !has_cached_snapshot
        && let Some(recovered) = recover_persisted_child_model(&state.models, parent_frozen, record)
    {
        streams
            .frozen
            .insert(record.child_session_id.clone(), recovered.clone());
        streams
            .pinned_subagent_models
            .insert(record.child_session_id.clone(), recovered.clone());
        return recovered;
    }
    parent_frozen.clone()
}
