//! `/connect` 异步编排：`connect.json` 写入、OAuth 授权流与运行时模型快照刷新。

use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, Mutex, MutexGuard};

use tokio::sync::mpsc;

use crate::app::effects::Effect;
use crate::app::state::AppState;
use crate::app::update;
use crate::config::ai;
use crate::config::ai::connect::{
    self, ConnectSettings, ConnectedPresetRecord, OAuthCredential, connect_path_for_ai_path,
    load_connect_or_default, save_connect,
};
use crate::config::ai::oauth::{OAuthStarted, OAuthTaskHandle, start_oauth_flow};
use crate::config::ai::presets::{default_models_for_preset, find_preset, resolve_preset_base_url};
use crate::config::ai::verify::{merge_discovered_models_for_preset, verify_provider_api_key};
use crate::event::AppEvent;
use crate::layout::PaneId;

/// `/connect` 后台任务回投事件。
#[derive(Debug)]
pub enum ConnectEvent {
    Connected {
        pane: PaneId,
        provider_id: String,
        connect: Box<ConnectSettings>,
        settings: Box<ai::AiSettings>,
    },
    Disconnected {
        pane: PaneId,
        provider_id: String,
        connect: Box<ConnectSettings>,
        settings: Box<ai::AiSettings>,
    },
    Failed {
        pane: PaneId,
        error: String,
    },
    OAuthPrompt {
        pane: PaneId,
        provider_id: String,
        method_id: String,
        started: OAuthStarted,
    },
    OAuthFailed {
        pane: PaneId,
        provider_id: String,
        attempt_id: String,
        error: String,
    },
    /// 云端目录与已连接供应商模型刷新完成；`pane` 为 None 时是启动静默同步。
    ModelsRefreshed {
        pane: Option<PaneId>,
        count: usize,
        settings: Box<ai::AiSettings>,
        connect: ConnectSettings,
    },
    /// 模型刷新失败；`pane` 为 None 时只记录日志。
    ModelsRefreshFailed {
        pane: Option<PaneId>,
        error: String,
    },
}

/// 进行中的 OAuth 尝试：事件循环与后台任务共享的取消/授权码句柄。
#[derive(Clone, Default)]
pub struct ConnectTasks {
    oauth: Arc<Mutex<HashMap<String, OAuthTaskHandle>>>,
}

impl ConnectTasks {
    fn lock(&self) -> MutexGuard<'_, HashMap<String, OAuthTaskHandle>> {
        match self.oauth.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        }
    }
}

fn is_connect_effect(effect: &Effect) -> bool {
    matches!(
        effect,
        Effect::SavePresetConnection { .. }
            | Effect::DisconnectPreset { .. }
            | Effect::StartPresetOAuth { .. }
            | Effect::CancelPresetOAuth { .. }
            | Effect::SubmitPresetOAuthCode { .. }
            | Effect::RefreshModelsCatalog { .. }
            | Effect::OpenBrowser { .. }
    )
}

/// 取走并执行 `/connect` 效应；返回是否处理了任何效应。
pub fn apply_effects(
    state: &mut AppState,
    tasks: &ConnectTasks,
    sender: &mpsc::UnboundedSender<AppEvent>,
) -> bool {
    let effects: Vec<Effect> = state
        .effects
        .iter()
        .filter(|effect| is_connect_effect(effect))
        .cloned()
        .collect();
    if effects.is_empty() {
        return false;
    }
    state.effects.retain(|effect| !is_connect_effect(effect));
    for effect in effects {
        match effect {
            Effect::SavePresetConnection {
                pane,
                provider_id,
                api_key,
                oauth,
                form_answers,
            } => {
                let sender = sender.clone();
                tokio::spawn(async move {
                    let event =
                        match save_preset_connection(&provider_id, api_key, oauth, form_answers)
                            .await
                        {
                            Ok((connect, settings)) => ConnectEvent::Connected {
                                pane,
                                provider_id,
                                connect: Box::new(connect),
                                settings: Box::new(settings),
                            },
                            Err(error) => ConnectEvent::Failed { pane, error },
                        };
                    let _ = sender.send(AppEvent::Connect(event));
                });
            }
            Effect::DisconnectPreset { pane, provider_id } => {
                let sender = sender.clone();
                tokio::task::spawn_blocking(move || {
                    let event = match disconnect_preset(&provider_id) {
                        Ok((connect, settings)) => ConnectEvent::Disconnected {
                            pane,
                            provider_id,
                            connect: Box::new(connect),
                            settings: Box::new(settings),
                        },
                        Err(error) => ConnectEvent::Failed { pane, error },
                    };
                    let _ = sender.send(AppEvent::Connect(event));
                });
            }
            Effect::StartPresetOAuth {
                pane,
                provider_id,
                method_id,
                form_answers,
            } => {
                start_oauth_task(tasks, sender, pane, provider_id, method_id, form_answers);
            }
            Effect::CancelPresetOAuth { attempt_id } => {
                if let Some(handle) = tasks.lock().remove(&attempt_id) {
                    handle.cancel();
                }
            }
            Effect::RefreshModelsCatalog { pane, provider_id } => {
                let sender = sender.clone();
                tokio::spawn(async move {
                    let event = match refresh_models(provider_id.as_deref()).await {
                        Ok((count, settings, connect)) => ConnectEvent::ModelsRefreshed {
                            pane,
                            count,
                            settings: Box::new(settings),
                            connect,
                        },
                        Err(error) => ConnectEvent::ModelsRefreshFailed { pane, error },
                    };
                    let _ = sender.send(AppEvent::Connect(event));
                });
            }
            Effect::SubmitPresetOAuthCode { attempt_id, code } => {
                let error = tasks
                    .lock()
                    .get(&attempt_id)
                    .and_then(|handle| handle.submit_code(code).err());
                if let Some(error) = error {
                    tracing::warn!(%error, "OAuth code submission failed");
                }
            }
            Effect::OpenBrowser { url } => {
                tokio::task::spawn_blocking(move || {
                    if let Err(error) = crate::platform::open_browser(&url) {
                        tracing::warn!(%error, "opening the OAuth authorization page failed");
                    }
                });
            }
            _ => {}
        }
    }
    true
}

/// 回投事件分派给状态层。
pub fn handle_event(state: &mut AppState, event: ConnectEvent) -> bool {
    match event {
        ConnectEvent::Connected {
            pane,
            provider_id,
            connect,
            settings,
        } => update::on_preset_connected(state, pane, &provider_id, *connect, *settings),
        ConnectEvent::Disconnected {
            pane,
            provider_id,
            connect,
            settings,
        } => update::on_preset_disconnected(state, pane, &provider_id, *connect, *settings),
        ConnectEvent::Failed { pane, error } => update::on_connect_failed(state, pane, &error),
        ConnectEvent::OAuthPrompt {
            pane,
            provider_id,
            method_id,
            started,
        } => update::on_preset_oauth_prompt(state, pane, &provider_id, &method_id, started),
        ConnectEvent::OAuthFailed {
            pane,
            provider_id,
            attempt_id,
            error,
        } => update::on_preset_oauth_failed(state, pane, &provider_id, &attempt_id, &error),
        ConnectEvent::ModelsRefreshed {
            pane,
            count,
            settings,
            connect,
        } => update::on_models_refreshed(state, pane, count, *settings, connect),
        ConnectEvent::ModelsRefreshFailed { pane, error } => {
            update::on_models_refresh_failed(state, pane, &error)
        }
    }
}

fn start_oauth_task(
    tasks: &ConnectTasks,
    sender: &mpsc::UnboundedSender<AppEvent>,
    pane: PaneId,
    provider_id: String,
    method_id: String,
    form_answers: BTreeMap<String, String>,
) {
    let tasks = tasks.clone();
    let sender = sender.clone();
    tokio::spawn(async move {
        let (started, mut handle) =
            match start_oauth_flow(&provider_id, &method_id, &form_answers).await {
                Ok(started) => started,
                Err(error) => {
                    let _ = sender.send(AppEvent::Connect(ConnectEvent::OAuthFailed {
                        pane,
                        provider_id,
                        attempt_id: String::new(),
                        error,
                    }));
                    return;
                }
            };
        let attempt_id = started.attempt_id.clone();
        let Some(result) = handle.take_result() else {
            let _ = sender.send(AppEvent::Connect(ConnectEvent::OAuthFailed {
                pane,
                provider_id,
                attempt_id,
                error: "OAuth flow did not provide a result channel".to_string(),
            }));
            return;
        };
        tasks.lock().insert(attempt_id.clone(), handle);
        let _ = sender.send(AppEvent::Connect(ConnectEvent::OAuthPrompt {
            pane,
            provider_id: provider_id.clone(),
            method_id,
            started,
        }));
        let outcome = match result.await {
            Ok(outcome) => outcome,
            Err(_) => Err("OAuth flow ended without a result".to_string()),
        };
        tasks.lock().remove(&attempt_id);
        match outcome {
            Ok(credential) => {
                let event = match save_preset_connection(
                    &provider_id,
                    None,
                    Some(credential),
                    form_answers,
                )
                .await
                {
                    Ok((connect, settings)) => ConnectEvent::Connected {
                        pane,
                        provider_id,
                        connect: Box::new(connect),
                        settings: Box::new(settings),
                    },
                    Err(error) => ConnectEvent::Failed { pane, error },
                };
                let _ = sender.send(AppEvent::Connect(event));
            }
            Err(error) => {
                let _ = sender.send(AppEvent::Connect(ConnectEvent::OAuthFailed {
                    pane,
                    provider_id,
                    attempt_id,
                    error,
                }));
            }
        }
    });
}

async fn save_preset_connection(
    provider_id: &str,
    api_key: Option<String>,
    oauth: Option<OAuthCredential>,
    form_answers: BTreeMap<String, String>,
) -> Result<(ConnectSettings, ai::AiSettings), String> {
    let ai_path = ai::ai_config_path().map_err(|error| error.to_string())?;
    let connect_path = connect_path_for_ai_path(&ai_path);
    let mut settings = load_connect_or_default(&connect_path).map_err(|error| error.to_string())?;
    let mut record = settings
        .providers
        .remove(provider_id)
        .unwrap_or_else(|| ConnectedPresetRecord::new(provider_id));
    record.enabled = true;
    record.form_answers = form_answers;
    if let Some(api_key) = api_key {
        let spec = find_preset(provider_id)
            .ok_or_else(|| format!("unknown preset provider '{provider_id}'"))?;
        let base_url =
            resolve_preset_base_url(spec, &record.form_answers, record.base_url.as_deref());
        let probe_model = default_models_for_preset(provider_id)
            .keys()
            .next()
            .cloned();
        let outcome =
            verify_provider_api_key(spec.transport, &base_url, &api_key, probe_model.as_deref())
                .await?;
        if record.models.is_none() && !outcome.discovered_models.is_empty() {
            record.models = Some(merge_discovered_models_for_preset(
                provider_id,
                record.models.as_ref(),
                &outcome.discovered_models,
            ));
        }
        record.api_key = Some(api_key);
        record.oauth = None;
    }
    if let Some(oauth) = oauth {
        record.oauth = Some(oauth);
        record.api_key = None;
    }
    settings.providers.insert(provider_id.to_string(), record);
    save_connect(&connect_path, &settings).map_err(|error| error.to_string())?;
    let runtime = connect::load_runtime_ai_or_empty(&ai_path).map_err(|error| error.to_string())?;
    Ok((settings, runtime))
}

/// 后台刷新云端目录与已连接供应商模型，并重新加载运行时快照。
async fn refresh_models(
    provider_id: Option<&str>,
) -> Result<(usize, ai::AiSettings, ConnectSettings), String> {
    let ai_path = ai::ai_config_path().map_err(|error| error.to_string())?;
    let count = crate::config::ai::catalog_refresh::refresh_connected_provider_models(
        &ai_path,
        provider_id,
    )
    .await?;
    let connect_path = connect_path_for_ai_path(&ai_path);
    let connect_settings =
        load_connect_or_default(&connect_path).map_err(|error| error.to_string())?;
    let runtime = connect::load_runtime_ai_or_empty(&ai_path).map_err(|error| error.to_string())?;
    Ok((count, runtime, connect_settings))
}

fn disconnect_preset(provider_id: &str) -> Result<(ConnectSettings, ai::AiSettings), String> {
    let ai_path = ai::ai_config_path().map_err(|error| error.to_string())?;
    let connect_path = connect_path_for_ai_path(&ai_path);
    let mut settings = load_connect_or_default(&connect_path).map_err(|error| error.to_string())?;
    settings.providers.remove(provider_id);
    save_connect(&connect_path, &settings).map_err(|error| error.to_string())?;
    let runtime = connect::load_runtime_ai_or_empty(&ai_path).map_err(|error| error.to_string())?;
    Ok((settings, runtime))
}

/// 启动加载模型配置快照：`ai.json` + `connect.json` 合并后的运行时视图。
pub(crate) fn load_models_snapshot(state: &mut AppState) {
    let path = match ai::ai_config_path() {
        Ok(path) => path,
        Err(error) => {
            tracing::warn!(%error, "ai config path unavailable");
            return;
        }
    };
    let connect_path = connect_path_for_ai_path(&path);
    match connect::load_connect_or_default(&connect_path) {
        Ok(settings) => state.connect = settings,
        Err(error) => tracing::warn!(%error, "connect config load failed"),
    }
    match connect::load_runtime_ai(&path) {
        Ok(Some(settings)) => state.models.settings = settings,
        Ok(None) => {}
        Err(error) => tracing::warn!(%error, "ai config load failed"),
    }
    update::refresh_all_lx_status(state);
}
