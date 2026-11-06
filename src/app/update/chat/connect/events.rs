//! `/connect` 认证流程推进与异步回执收敛。
//!
//! 表单/API Key/OAuth 阶段推进产生 `Effect`；连接、断开、OAuth 与模型刷新回执在
//! 这里更新运行时快照与模态状态。模态是否仍在屏决定 Toast 归属与是否接管窗格。

use std::collections::BTreeMap;
use std::time::Instant;

use crate::app::effects::Effect;
use crate::app::overlay::{Overlay, TextInput};
use crate::app::state::{
    AppState, ConnectPanel, ConnectPromptStep, ConnectStage, ModelPicker, PanelWindow, SlashPanel,
};
use crate::app::toast::{Toast, ToastKind};
use crate::config::ai::oauth::{OAuthPromptMode, OAuthStarted};
use crate::config::ai::presets::{PresetAuthMethod, PresetFormField, find_preset};
use crate::config::ai::{AiSettings, connect::ConnectSettings};
use crate::layout::PaneId;
use crate::ui::text;

use super::current_panel;

/// Ctrl+R / `Refresh models`：进入校验态并请求后台刷新。
pub(super) fn start_models_refresh(state: &mut AppState, provider_id: Option<String>) -> bool {
    let Some(panel) = current_panel(state) else {
        return false;
    };
    let pane = panel.pane;
    let mut panel = panel;
    panel.validating = true;
    panel.status_message = Some(text::LX_CONNECT_REFRESHING.to_string());
    state.overlay = Some(Overlay::Connect(Box::new(panel)));
    state.effects.push(Effect::RefreshModelsCatalog {
        pane: Some(pane),
        provider_id,
    });
    true
}

pub(super) fn open_browser(state: &mut AppState, pane: PaneId, url: &str) -> bool {
    if url.is_empty() {
        return true;
    }
    state.effects.push(Effect::OpenBrowser {
        url: url.to_string(),
    });
    show_connect_toast(
        state,
        pane,
        ToastKind::Default,
        text::LX_CONNECT_OAUTH_OPENED.to_string(),
    );
    true
}

pub(super) fn copy_oauth_text(state: &mut AppState, pane: PaneId, text_value: &str) -> bool {
    if text_value.is_empty() {
        return true;
    }
    state.effects.push(Effect::CopyToClipboard {
        text: text_value.to_string(),
        anchor: Some(pane),
    });
    true
}

/// 模态在屏时写入全局槽，否则落入打开弹窗的源容器窗格。
fn show_connect_toast(state: &mut AppState, pane: PaneId, kind: ToastKind, message: String) {
    crate::app::update::show_toast_at(
        state,
        Some(pane),
        Toast::new(kind, message, Some(pane), Instant::now()),
    );
}

/// 开始认证：多方法先选方法，单方法直接进入该方法。
pub(super) fn begin_auth(
    state: &mut AppState,
    provider_id: &str,
    provider_name: &str,
    methods: &'static [PresetAuthMethod],
    answers: BTreeMap<String, String>,
) -> bool {
    if methods.is_empty() {
        let Some(panel) = current_panel(state) else {
            return false;
        };
        show_connect_toast(
            state,
            panel.pane,
            ToastKind::Warning,
            text::LX_CONNECT_NO_METHODS.to_string(),
        );
        return true;
    }
    if methods.len() > 1 {
        let Some(panel) = current_panel(state) else {
            return false;
        };
        return replace_panel(
            state,
            panel,
            ConnectStage::Methods {
                provider_id: provider_id.to_string(),
                provider_name: provider_name.to_string(),
                methods,
                window: PanelWindow::default(),
            },
        );
    }
    begin_method(state, provider_id, provider_name, methods, 0, answers)
}

/// 进入某个认证方法的首个表单字段/Key 输入/OAuth 启动。
pub(super) fn begin_method(
    state: &mut AppState,
    provider_id: &str,
    provider_name: &str,
    methods: &'static [PresetAuthMethod],
    method_index: usize,
    answers: BTreeMap<String, String>,
) -> bool {
    let Some(method) = methods.get(method_index) else {
        return true;
    };
    if let Some(field_index) = first_active_field(method, 0, &answers) {
        let field = method.form()[field_index].clone();
        return set_form_field(
            state,
            provider_id,
            provider_name,
            method_index,
            field_index,
            field,
            answers,
        );
    }
    match method {
        PresetAuthMethod::ApiKey { .. } => {
            set_api_key_prompt(state, provider_id, provider_name, method_index, answers)
        }
        PresetAuthMethod::OAuth { id, .. } => {
            start_oauth(state, provider_id, provider_name, id, answers)
        }
    }
}

/// 表单字段填完后：继续下一个字段，或进入 Key 输入 / 启动 OAuth。
pub(super) fn advance_after_field(
    state: &mut AppState,
    provider_id: &str,
    provider_name: &str,
    method_index: usize,
    answers: BTreeMap<String, String>,
    field_index: usize,
) -> bool {
    let Some(spec) = find_preset(provider_id) else {
        return true;
    };
    let Some(method) = spec.methods.get(method_index) else {
        return true;
    };
    if let Some(next_index) = first_active_field(method, field_index + 1, &answers) {
        let field = method.form()[next_index].clone();
        return set_form_field(
            state,
            provider_id,
            provider_name,
            method_index,
            next_index,
            field,
            answers,
        );
    }
    match method {
        PresetAuthMethod::ApiKey { .. } => {
            set_api_key_prompt(state, provider_id, provider_name, method_index, answers)
        }
        PresetAuthMethod::OAuth { id, .. } => {
            start_oauth(state, provider_id, provider_name, id, answers)
        }
    }
}

/// 首个满足 `when_eq` 条件的表单字段。
fn first_active_field(
    method: &PresetAuthMethod,
    start: usize,
    answers: &BTreeMap<String, String>,
) -> Option<usize> {
    method
        .form()
        .iter()
        .enumerate()
        .skip(start)
        .find(|(_, field)| field.is_active(answers))
        .map(|(index, _)| index)
}

fn set_form_field(
    state: &mut AppState,
    provider_id: &str,
    provider_name: &str,
    method_index: usize,
    field_index: usize,
    field: PresetFormField,
    answers: BTreeMap<String, String>,
) -> bool {
    let Some(panel) = current_panel(state) else {
        return false;
    };
    replace_panel(
        state,
        panel,
        ConnectStage::InputPrompt {
            provider_id: provider_id.to_string(),
            provider_name: provider_name.to_string(),
            method_index,
            step: ConnectPromptStep::FormField {
                field_index,
                field,
                option_window: PanelWindow::default(),
            },
            answers,
            error: None,
        },
    )
}

fn set_api_key_prompt(
    state: &mut AppState,
    provider_id: &str,
    provider_name: &str,
    method_index: usize,
    answers: BTreeMap<String, String>,
) -> bool {
    let Some(panel) = current_panel(state) else {
        return false;
    };
    replace_panel(
        state,
        panel,
        ConnectStage::InputPrompt {
            provider_id: provider_id.to_string(),
            provider_name: provider_name.to_string(),
            method_index,
            step: ConnectPromptStep::ApiKey,
            answers,
            error: None,
        },
    )
}

pub(super) fn set_prompt_error(state: &mut AppState, panel: &ConnectPanel, error: &str) -> bool {
    let mut panel = panel.clone();
    panel.set_error(error);
    state.overlay = Some(Overlay::Connect(Box::new(panel)));
    true
}

/// 启动 OAuth：入队效应并进入等待阶段（链接由回投事件补齐）。
fn start_oauth(
    state: &mut AppState,
    provider_id: &str,
    provider_name: &str,
    method_id: &str,
    answers: BTreeMap<String, String>,
) -> bool {
    let Some(panel) = current_panel(state) else {
        return false;
    };
    state.effects.push(Effect::StartPresetOAuth {
        pane: panel.pane,
        provider_id: provider_id.to_string(),
        method_id: method_id.to_string(),
        form_answers: answers.clone(),
    });
    replace_panel(
        state,
        panel,
        ConnectStage::OAuthWaiting {
            attempt_id: String::new(),
            provider_id: provider_id.to_string(),
            provider_name: provider_name.to_string(),
            method_id: method_id.to_string(),
            form_answers: answers,
            url: String::new(),
            user_code: None,
            instructions: text::LX_CONNECT_OAUTH_STARTING.to_string(),
            mode: OAuthPromptMode::Auto,
            error: None,
            window: PanelWindow::default(),
        },
    )
}

/// 切换阶段：清空临时输入、错误与窗口状态后写回模态。
pub(super) fn replace_panel(
    state: &mut AppState,
    mut panel: ConnectPanel,
    stage: ConnectStage,
) -> bool {
    panel.stage = stage;
    panel.input = TextInput::new("");
    panel.validating = false;
    panel.status_message = None;
    panel.hover = None;
    panel.window = PanelWindow::default();
    panel.clamp_window();
    state.overlay = Some(Overlay::Connect(Box::new(panel)));
    true
}

/// 连接保存成功：更新运行时快照。
///
/// 仅当 `/connect` 仍打开且属于同一窗格时，才关闭模态、清空草稿并切到该供应商的
/// `/model` 面板；用户已按 `Esc` 关闭或打开了别的模态时只更新状态并弹 Toast，绝不
/// 抢占当前浮层或清掉无关草稿。
pub fn on_preset_connected(
    state: &mut AppState,
    pane: PaneId,
    provider_id: &str,
    connect: ConnectSettings,
    settings: AiSettings,
) -> bool {
    let name = find_preset(provider_id)
        .map(|spec| spec.name.to_string())
        .unwrap_or_else(|| provider_id.to_string());
    let current = state
        .pane_anywhere(pane)
        .and_then(|target| target.lx.chat.effective_selection(&state.models));
    let modal_open = matches!(
        state.overlay.as_ref(),
        Some(Overlay::Connect(panel)) if panel.pane == pane
    );
    state.last_connect_provider = Some(provider_id.to_string());
    state.models.settings = settings;
    state.connect = connect;
    // 连接后静默同步模型目录（`pane: None` 不弹刷新成功/鉴权失败 Toast，错误 Key 仅在发消息时报错）。
    state.effects.push(Effect::RefreshModelsCatalog {
        pane: None,
        provider_id: Some(provider_id.to_string()),
    });
    if modal_open {
        state.overlay = None;
    }
    super::super::refresh_all_lx_status(state);
    show_connect_toast(
        state,
        pane,
        ToastKind::Default,
        text::lx_connect_connected(&name),
    );
    if !modal_open {
        return true;
    }
    let picker = ModelPicker::for_provider(&state.models.settings, current.as_ref(), provider_id);
    let Some(target) = state.pane_mut_anywhere(pane) else {
        return true;
    };
    target.lx.clear();
    target.lx.slash = Some(SlashPanel::Model(Box::new(picker)));
    true
}

/// 断开连接完成：刷新快照与模态连接状态。
///
/// 只在模态仍处于供应商列表（或同一供应商的管理菜单）时退回列表并高亮刚断开的供应商；
/// 用户已经进入另一个供应商的输入/OAuth 阶段时仅更新底层条目，不打断当前流程。
pub fn on_preset_disconnected(
    state: &mut AppState,
    pane: PaneId,
    provider_id: &str,
    connect: ConnectSettings,
    settings: AiSettings,
) -> bool {
    let name = find_preset(provider_id)
        .map(|spec| spec.name.to_string())
        .unwrap_or_else(|| provider_id.to_string());
    state.last_connect_provider = Some(provider_id.to_string());
    state.models.settings = settings;
    state.connect = connect;
    super::super::refresh_all_lx_status(state);
    if let Some(Overlay::Connect(panel)) = state.overlay.as_mut()
        && panel.pane == pane
    {
        panel.refresh_entries(&state.connect, &state.connect_env);
        let back_to_list = match &panel.stage {
            ConnectStage::Providers => true,
            ConnectStage::Manage {
                provider_id: active,
                ..
            } => active == provider_id,
            _ => false,
        };
        if back_to_list {
            panel.reset_to_providers();
            panel.select_provider(provider_id);
        }
    }
    show_connect_toast(
        state,
        pane,
        ToastKind::Default,
        text::lx_connect_disconnected(&name),
    );
    true
}

/// 连接保存/校验失败：在输入阶段原地报错并保留输入；其余阶段（如断开失败）按模态在屏态分派 Toast。
pub fn on_connect_failed(state: &mut AppState, pane: PaneId, error: &str) -> bool {
    if let Some(Overlay::Connect(panel)) = state.overlay.as_mut()
        && panel.pane == pane
    {
        panel.validating = false;
        panel.status_message = None;
        if panel.set_error(error.to_string()) {
            return true;
        }
    }
    show_connect_toast(
        state,
        pane,
        ToastKind::Danger,
        text::lx_connect_failed(error),
    );
    true
}

/// OAuth 启动回调：补齐授权链接、设备码与模式。
pub fn on_preset_oauth_prompt(
    state: &mut AppState,
    pane: PaneId,
    provider_id: &str,
    method_id: &str,
    started: OAuthStarted,
) -> bool {
    let mut applied = false;
    if let Some(Overlay::Connect(panel)) = state.overlay.as_mut()
        && panel.pane == pane
        && let ConnectStage::OAuthWaiting {
            attempt_id,
            provider_id: active_provider,
            method_id: active_method,
            url,
            user_code,
            instructions,
            mode,
            error,
            window,
            ..
        } = &mut panel.stage
        && active_provider == provider_id
        && (active_method.is_empty() || active_method == method_id)
        && (attempt_id.is_empty() || attempt_id == &started.attempt_id)
    {
        *attempt_id = started.attempt_id.clone();
        *url = started.url;
        *user_code = started.user_code;
        *instructions = started.instructions;
        *mode = started.mode;
        *error = None;
        *window = PanelWindow::default();
        applied = true;
    }
    if !applied && !started.attempt_id.is_empty() {
        state.effects.push(Effect::CancelPresetOAuth {
            attempt_id: started.attempt_id,
        });
    }
    applied
}

/// OAuth 授权失败：模态内保留错误与危险 Toast。
pub fn on_preset_oauth_failed(
    state: &mut AppState,
    pane: PaneId,
    provider_id: &str,
    attempt_id: &str,
    error: &str,
) -> bool {
    let mut applied = false;
    if let Some(Overlay::Connect(panel)) = state.overlay.as_mut()
        && panel.pane == pane
        && let ConnectStage::OAuthWaiting {
            attempt_id: active_attempt,
            provider_id: active_provider,
            error: slot,
            ..
        } = &mut panel.stage
        && active_provider == provider_id
        && (active_attempt.is_empty() || active_attempt == attempt_id)
    {
        *slot = Some(error.to_string());
        applied = true;
    }
    if applied && !error.to_ascii_lowercase().contains("cancelled") {
        show_connect_toast(
            state,
            pane,
            ToastKind::Danger,
            text::lx_connect_oauth_failed(error),
        );
    }
    applied
}

/// 模型刷新完成：更新运行时快照、清掉校验态并按需分派 Toast。
pub fn on_models_refreshed(
    state: &mut AppState,
    pane: Option<PaneId>,
    count: usize,
    settings: AiSettings,
    connect: ConnectSettings,
) -> bool {
    state.models.settings = settings;
    state.connect = connect;
    if let Some(Overlay::Connect(panel)) = state.overlay.as_mut() {
        panel.validating = false;
        panel.status_message = None;
        panel.refresh_entries(&state.connect, &state.connect_env);
    }
    super::super::refresh_all_lx_status(state);
    if let Some(pane) = pane {
        show_connect_toast(
            state,
            pane,
            ToastKind::Default,
            text::lx_connect_refreshed_models(count),
        );
    }
    true
}

/// 模型刷新失败：清掉校验态；用户主动触发时提示，启动静默同步只记日志。
pub fn on_models_refresh_failed(state: &mut AppState, pane: Option<PaneId>, error: &str) -> bool {
    if let Some(Overlay::Connect(panel)) = state.overlay.as_mut() {
        panel.validating = false;
        panel.status_message = None;
    }
    if let Some(pane) = pane {
        show_connect_toast(
            state,
            pane,
            ToastKind::Warning,
            text::lx_connect_refresh_failed(error),
        );
    }
    true
}
