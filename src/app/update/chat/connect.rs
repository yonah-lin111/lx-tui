//! `/connect` 面板交互：供应商选择、认证方式推进、表单/API Key 提交与 OAuth 回调。

use std::collections::BTreeMap;
use std::time::Instant;

use crate::app::actions::LxInput;
use crate::app::effects::Effect;
use crate::app::state::{
    AppState, ConnectManageAction, ConnectPanel, ConnectPromptStep, ConnectStage, ModelPicker,
    SlashPanel,
};
use crate::app::toast::{Toast, ToastKind};
use crate::config::ai::AiSettings;
use crate::config::ai::connect::ConnectSettings;
use crate::config::ai::oauth::{OAuthPromptMode, OAuthStarted};
use crate::config::ai::presets::{PresetAuthMethod, find_preset};
use crate::layout::PaneId;
use crate::ui::text;

use super::slash::clear_lx_input;

/// `/connect` 命令采纳：打开 A-Z 供应商面板。
pub fn open_connect_panel(state: &mut AppState, pane: PaneId) -> bool {
    if !state
        .pane_anywhere(pane)
        .is_some_and(|target| target.view == crate::app::state::PaneView::Lx)
    {
        return false;
    }
    let panel = ConnectPanel::new(&state.connect);
    let Some(target) = state.pane_mut_anywhere(pane) else {
        return false;
    };
    target.lx.clear();
    target.lx.slash = Some(SlashPanel::Connect(Box::new(panel)));
    true
}

/// `/connect` 面板按键的特殊动作（Enter/Esc 与 OAuth 快捷键）。
///
/// 返回 `true` 表示已消费；返回 `false` 时编辑键继续走输入框编辑分支。
pub fn handle_connect_input(state: &mut AppState, pane: PaneId, command: LxInput) -> bool {
    let Some(panel) = current_panel(state, pane) else {
        return false;
    };
    match command {
        LxInput::Enter | LxInput::Tab => {
            apply_connect_enter(state, pane);
            true
        }
        LxInput::Escape => {
            apply_connect_esc(state, pane);
            true
        }
        LxInput::Char(ch) if panel.shortcut_char(ch) => {
            let input_empty = state
                .pane_anywhere(pane)
                .is_some_and(|target| target.lx.input.trim().is_empty());
            if !input_empty {
                return false;
            }
            apply_connect_shortcut(state, pane, ch);
            true
        }
        _ => false,
    }
}

/// 当前 `/connect` 面板快照。
fn current_panel(state: &AppState, pane: PaneId) -> Option<ConnectPanel> {
    state
        .pane_anywhere(pane)
        .and_then(|target| match &target.lx.slash {
            Some(SlashPanel::Connect(panel)) => Some((**panel).clone()),
            _ => None,
        })
}

/// `/connect` 面板确认。
pub fn apply_connect_enter(state: &mut AppState, pane: PaneId) -> bool {
    let Some(panel) = current_panel(state, pane) else {
        return false;
    };
    match panel.stage {
        ConnectStage::Providers => enter_provider(state, pane, &panel),
        ConnectStage::Manage { .. } => enter_manage(state, pane, &panel),
        ConnectStage::Methods { .. } => enter_method(state, pane, &panel),
        ConnectStage::InputPrompt { .. } => enter_input_prompt(state, pane, &panel),
        ConnectStage::OAuthWaiting { .. } => enter_oauth_waiting(state, pane, &panel),
    }
}

/// 供应商列表 Enter：已连接进入管理菜单，未连接直接进入认证流程。
fn enter_provider(state: &mut AppState, pane: PaneId, panel: &ConnectPanel) -> bool {
    let Some(entry) = panel.active_entry().cloned() else {
        return true;
    };
    let Some(source) = entry.source.clone() else {
        return begin_auth(
            state,
            pane,
            &entry.id,
            &entry.name,
            entry.methods,
            BTreeMap::new(),
        );
    };
    let mut actions = vec![
        ConnectManageAction::SelectModel,
        ConnectManageAction::Reconnect,
    ];
    if has_stored_credentials(&state.connect, &entry.id) {
        actions.push(ConnectManageAction::Disconnect);
    }
    replace_panel(
        state,
        pane,
        panel.clone(),
        ConnectStage::Manage {
            provider_id: entry.id,
            provider_name: entry.name,
            source,
            actions,
            window: crate::app::state::PanelWindow::default(),
        },
    )
}

/// 管理菜单 Enter：选模型 / 重新认证 / 断开连接。
fn enter_manage(state: &mut AppState, pane: PaneId, panel: &ConnectPanel) -> bool {
    let ConnectStage::Manage {
        provider_id,
        provider_name,
        actions,
        window,
        ..
    } = &panel.stage
    else {
        return true;
    };
    let Some(action) = actions.get(window.active).copied() else {
        return true;
    };
    match action {
        ConnectManageAction::SelectModel => {
            let current = state
                .pane_anywhere(pane)
                .and_then(|target| target.lx.chat.effective_selection(&state.models));
            let picker =
                ModelPicker::for_provider(&state.models.settings, current.as_ref(), provider_id);
            let Some(target) = state.pane_mut_anywhere(pane) else {
                return true;
            };
            target.lx.clear();
            target.lx.slash = Some(SlashPanel::Model(Box::new(picker)));
            true
        }
        ConnectManageAction::Reconnect => {
            let Some(spec) = find_preset(provider_id) else {
                return true;
            };
            begin_auth(
                state,
                pane,
                provider_id,
                provider_name,
                spec.methods,
                BTreeMap::new(),
            )
        }
        ConnectManageAction::Disconnect => {
            state.effects.push(Effect::DisconnectPreset {
                pane,
                provider_id: provider_id.clone(),
            });
            true
        }
    }
}

/// 认证方式 Enter：进入所选方法的表单/Key 输入或直接启动 OAuth。
fn enter_method(state: &mut AppState, pane: PaneId, panel: &ConnectPanel) -> bool {
    let ConnectStage::Methods {
        provider_id,
        provider_name,
        methods,
        window,
    } = &panel.stage
    else {
        return true;
    };
    let method_index = window.active.min(methods.len().saturating_sub(1));
    begin_method(
        state,
        pane,
        provider_id,
        provider_name,
        methods,
        method_index,
        BTreeMap::new(),
    )
}

/// 表单/API Key Enter。
fn enter_input_prompt(state: &mut AppState, pane: PaneId, panel: &ConnectPanel) -> bool {
    let ConnectStage::InputPrompt {
        provider_id,
        provider_name,
        method_index,
        step,
        answers,
        ..
    } = &panel.stage
    else {
        return true;
    };
    match step {
        ConnectPromptStep::FormField {
            field_index,
            field,
            option_window,
        } => {
            if field.options.is_empty() {
                let value = state
                    .pane_anywhere(pane)
                    .map(|target| target.lx.input.trim().to_string())
                    .unwrap_or_default();
                if field.required && value.is_empty() {
                    return set_prompt_error(
                        state,
                        pane,
                        panel,
                        text::LX_CONNECT_FIELD_REQUIRED.to_string(),
                    );
                }
                clear_lx_input(state, pane);
                let mut answers = answers.clone();
                if value.is_empty() {
                    answers.remove(field.key);
                } else {
                    answers.insert(field.key.to_string(), value);
                }
                return advance_after_field(
                    state,
                    pane,
                    provider_id,
                    provider_name,
                    *method_index,
                    answers,
                    *field_index,
                );
            }
            let Some(option) = field.options.get(option_window.active).copied() else {
                return true;
            };
            let mut answers = answers.clone();
            answers.insert(field.key.to_string(), option.value.to_string());
            advance_after_field(
                state,
                pane,
                provider_id,
                provider_name,
                *method_index,
                answers,
                *field_index,
            )
        }
        ConnectPromptStep::ApiKey => {
            let value = state
                .pane_anywhere(pane)
                .map(|target| target.lx.input.trim().to_string())
                .unwrap_or_default();
            if value.is_empty() {
                show_lx_toast(
                    state,
                    pane,
                    ToastKind::Warning,
                    text::LX_CONNECT_API_KEY_EMPTY.to_string(),
                );
                return true;
            }
            clear_lx_input(state, pane);
            state.effects.push(Effect::SavePresetConnection {
                pane,
                provider_id: provider_id.clone(),
                api_key: Some(value),
                oauth: None,
                form_answers: answers.clone(),
            });
            true
        }
    }
}

/// OAuth 等待 Enter：打开浏览器 / 复制代码 / 提交授权码。
fn enter_oauth_waiting(state: &mut AppState, pane: PaneId, panel: &ConnectPanel) -> bool {
    let ConnectStage::OAuthWaiting {
        attempt_id,
        url,
        user_code,
        mode,
        window,
        ..
    } = &panel.stage
    else {
        return true;
    };
    if *mode == OAuthPromptMode::Code {
        let code = state
            .pane_anywhere(pane)
            .map(|target| target.lx.input.trim().to_string())
            .unwrap_or_default();
        if !code.is_empty() {
            clear_lx_input(state, pane);
            if !attempt_id.is_empty() {
                state.effects.push(Effect::SubmitPresetOAuthCode {
                    attempt_id: attempt_id.clone(),
                    code,
                });
            }
            return true;
        }
    }
    match window.active {
        0 => open_browser(state, pane, url),
        1 => copy_oauth_text(state, pane, user_code.as_deref().unwrap_or(url)),
        _ => true,
    }
}

/// OAuth 等待阶段的 `o` / `c` 快捷键（仅自动模式、输入框为空时）。
fn apply_connect_shortcut(state: &mut AppState, pane: PaneId, ch: char) -> bool {
    let Some(panel) = current_panel(state, pane) else {
        return false;
    };
    let ConnectStage::OAuthWaiting { url, user_code, .. } = &panel.stage else {
        return false;
    };
    match ch {
        'o' => open_browser(state, pane, url),
        'c' => copy_oauth_text(state, pane, user_code.as_deref().unwrap_or(url)),
        _ => false,
    }
}

/// `/connect` 面板 Esc：OAuth 取消并退回列表；其余阶段退回列表；列表关闭面板。
pub fn apply_connect_esc(state: &mut AppState, pane: PaneId) -> bool {
    let Some(panel) = current_panel(state, pane) else {
        return false;
    };
    if let ConnectStage::OAuthWaiting { attempt_id, .. } = &panel.stage
        && !attempt_id.is_empty()
    {
        state.effects.push(Effect::CancelPresetOAuth {
            attempt_id: attempt_id.clone(),
        });
    }
    clear_lx_input(state, pane);
    if matches!(panel.stage, ConnectStage::Providers) {
        if let Some(target) = state.pane_mut_anywhere(pane) {
            target.lx.slash = None;
        }
        return true;
    }
    let mut panel = panel;
    panel.stage = ConnectStage::Providers;
    panel.refilter("");
    let Some(target) = state.pane_mut_anywhere(pane) else {
        return true;
    };
    target.lx.slash = Some(SlashPanel::Connect(Box::new(panel)));
    true
}

/// 开始认证：多方法先选方法，单方法直接进入该方法。
fn begin_auth(
    state: &mut AppState,
    pane: PaneId,
    provider_id: &str,
    provider_name: &str,
    methods: &'static [PresetAuthMethod],
    answers: BTreeMap<String, String>,
) -> bool {
    if methods.is_empty() {
        show_lx_toast(
            state,
            pane,
            ToastKind::Warning,
            text::LX_CONNECT_NO_METHODS.to_string(),
        );
        return true;
    }
    if methods.len() > 1 {
        let Some(panel) = current_panel(state, pane) else {
            return false;
        };
        return replace_panel(
            state,
            pane,
            panel,
            ConnectStage::Methods {
                provider_id: provider_id.to_string(),
                provider_name: provider_name.to_string(),
                methods,
                window: crate::app::state::PanelWindow::default(),
            },
        );
    }
    begin_method(state, pane, provider_id, provider_name, methods, 0, answers)
}

/// 进入某个认证方法的首个表单字段/Key 输入/OAuth 启动。
fn begin_method(
    state: &mut AppState,
    pane: PaneId,
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
            pane,
            provider_id,
            provider_name,
            method_index,
            field_index,
            field,
            answers,
        );
    }
    match method {
        PresetAuthMethod::ApiKey { .. } => set_api_key_prompt(
            state,
            pane,
            provider_id,
            provider_name,
            method_index,
            answers,
        ),
        PresetAuthMethod::OAuth { id, .. } => {
            start_oauth(state, pane, provider_id, provider_name, id, answers)
        }
    }
}

/// 表单字段填完后：继续下一个字段，或进入 Key 输入 / 启动 OAuth。
fn advance_after_field(
    state: &mut AppState,
    pane: PaneId,
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
            pane,
            provider_id,
            provider_name,
            method_index,
            next_index,
            field,
            answers,
        );
    }
    match method {
        PresetAuthMethod::ApiKey { .. } => set_api_key_prompt(
            state,
            pane,
            provider_id,
            provider_name,
            method_index,
            answers,
        ),
        PresetAuthMethod::OAuth { id, .. } => {
            start_oauth(state, pane, provider_id, provider_name, id, answers)
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
    pane: PaneId,
    provider_id: &str,
    provider_name: &str,
    method_index: usize,
    field_index: usize,
    field: crate::config::ai::presets::PresetFormField,
    answers: BTreeMap<String, String>,
) -> bool {
    let Some(panel) = current_panel(state, pane) else {
        return false;
    };
    replace_panel(
        state,
        pane,
        panel,
        ConnectStage::InputPrompt {
            provider_id: provider_id.to_string(),
            provider_name: provider_name.to_string(),
            method_index,
            step: ConnectPromptStep::FormField {
                field_index,
                field,
                option_window: crate::app::state::PanelWindow::default(),
            },
            answers,
            error: None,
        },
    )
}

fn set_api_key_prompt(
    state: &mut AppState,
    pane: PaneId,
    provider_id: &str,
    provider_name: &str,
    method_index: usize,
    answers: BTreeMap<String, String>,
) -> bool {
    let Some(panel) = current_panel(state, pane) else {
        return false;
    };
    replace_panel(
        state,
        pane,
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

fn set_prompt_error(
    state: &mut AppState,
    pane: PaneId,
    panel: &ConnectPanel,
    error: String,
) -> bool {
    let mut panel = panel.clone();
    if let ConnectStage::InputPrompt { error: slot, .. } = &mut panel.stage {
        *slot = Some(error);
    }
    let Some(target) = state.pane_mut_anywhere(pane) else {
        return true;
    };
    target.lx.slash = Some(SlashPanel::Connect(Box::new(panel)));
    true
}

/// 启动 OAuth：入队效应并进入等待阶段（链接由回投事件补齐）。
fn start_oauth(
    state: &mut AppState,
    pane: PaneId,
    provider_id: &str,
    provider_name: &str,
    method_id: &str,
    answers: BTreeMap<String, String>,
) -> bool {
    state.effects.push(Effect::StartPresetOAuth {
        pane,
        provider_id: provider_id.to_string(),
        method_id: method_id.to_string(),
        form_answers: answers.clone(),
    });
    let Some(panel) = current_panel(state, pane) else {
        return false;
    };
    replace_panel(
        state,
        pane,
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
            window: crate::app::state::PanelWindow::default(),
        },
    )
}

fn replace_panel(
    state: &mut AppState,
    pane: PaneId,
    mut panel: ConnectPanel,
    stage: ConnectStage,
) -> bool {
    panel.stage = stage;
    panel.clamp_window();
    let Some(target) = state.pane_mut_anywhere(pane) else {
        return false;
    };
    target.lx.slash = Some(SlashPanel::Connect(Box::new(panel)));
    true
}

fn open_browser(state: &mut AppState, pane: PaneId, url: &str) -> bool {
    if url.is_empty() {
        return true;
    }
    state.effects.push(Effect::OpenBrowser {
        url: url.to_string(),
    });
    show_lx_toast(
        state,
        pane,
        ToastKind::Default,
        text::LX_CONNECT_OAUTH_OPENED.to_string(),
    );
    true
}

fn copy_oauth_text(state: &mut AppState, pane: PaneId, text_value: &str) -> bool {
    if text_value.is_empty() {
        return true;
    }
    state.effects.push(Effect::CopyToClipboard {
        text: text_value.to_string(),
        anchor: Some(pane),
    });
    true
}

/// 是否存在可断开的落盘凭证（API Key / OAuth）。
fn has_stored_credentials(connect: &ConnectSettings, provider_id: &str) -> bool {
    let Some(record) = connect.providers.get(provider_id) else {
        return false;
    };
    record
        .api_key
        .as_deref()
        .is_some_and(|key| !key.trim().is_empty())
        || record
            .oauth
            .as_ref()
            .is_some_and(|oauth| !oauth.access_token.trim().is_empty())
}

/// 连接保存成功：刷新快照、弹窗格 Toast，并切到该供应商的 `/model` 面板。
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
    state.models.settings = settings;
    state.connect = connect;
    super::refresh_all_lx_status(state);
    show_lx_toast(
        state,
        pane,
        ToastKind::Default,
        text::lx_connect_connected(&name),
    );
    let picker = ModelPicker::for_provider(&state.models.settings, current.as_ref(), provider_id);
    let Some(target) = state.pane_mut_anywhere(pane) else {
        return true;
    };
    target.lx.clear();
    target.lx.slash = Some(SlashPanel::Model(Box::new(picker)));
    true
}

/// 断开连接完成：刷新快照与面板连接状态。
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
    state.models.settings = settings;
    state.connect = connect;
    super::refresh_all_lx_status(state);
    show_lx_toast(
        state,
        pane,
        ToastKind::Default,
        text::lx_connect_disconnected(&name),
    );
    let refreshed = ConnectPanel::new(&state.connect);
    let Some(target) = state.pane_mut_anywhere(pane) else {
        return true;
    };
    if matches!(target.lx.slash, Some(SlashPanel::Connect(_))) {
        target.lx.clear();
        target.lx.slash = Some(SlashPanel::Connect(Box::new(refreshed)));
    }
    true
}

/// 连接保存/断开失败：窗格内危险 Toast。
pub fn on_connect_failed(state: &mut AppState, pane: PaneId, error: &str) -> bool {
    show_lx_toast(
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
    if let Some(target) = state.pane_mut_anywhere(pane)
        && let Some(SlashPanel::Connect(panel)) = target.lx.slash.as_mut()
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
        *window = crate::app::state::PanelWindow::default();
        applied = true;
    }
    if !applied && !started.attempt_id.is_empty() {
        state.effects.push(Effect::CancelPresetOAuth {
            attempt_id: started.attempt_id,
        });
    }
    applied
}

/// OAuth 授权失败：面板内保留错误与危险 Toast。
pub fn on_preset_oauth_failed(
    state: &mut AppState,
    pane: PaneId,
    provider_id: &str,
    attempt_id: &str,
    error: &str,
) -> bool {
    let mut applied = false;
    if let Some(target) = state.pane_mut_anywhere(pane)
        && let Some(SlashPanel::Connect(panel)) = target.lx.slash.as_mut()
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
    show_lx_toast(
        state,
        pane,
        ToastKind::Danger,
        text::lx_connect_oauth_failed(error),
    );
    applied
}

/// 统一构造窗格 Toast（保留 `Instant::now()` 单一入口）。
fn show_lx_toast(state: &mut AppState, pane: PaneId, kind: ToastKind, message: String) {
    crate::app::update::show_lx_toast(
        state,
        pane,
        Toast::new(kind, message, Some(pane), Instant::now()),
    );
}
