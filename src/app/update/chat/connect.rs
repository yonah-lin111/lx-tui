//! `/connect` 居中模态：供应商选择、认证推进、表单/API Key 提交、OAuth 与模型刷新。

use std::collections::BTreeMap;
use std::time::Instant;

use crate::app::actions::OverlayKey;
use crate::app::effects::Effect;
use crate::app::overlay::{Overlay, TextInput};
use crate::app::state::{
    AppState, ConnectManageAction, ConnectPanel, ConnectPromptStep, ConnectStage, ModelPicker,
    PanelWindow, SlashPanel,
};
use crate::app::toast::{Toast, ToastKind};
use crate::config::ai::AiSettings;
use crate::config::ai::connect::ConnectSettings;
use crate::config::ai::oauth::{OAuthPromptMode, OAuthStarted};
use crate::config::ai::presets::{PresetAuthMethod, PresetFormField, find_preset};
use crate::layout::PaneId;
use crate::ui::text;

/// `/connect` 命令采纳：打开居中模态，并清空草稿与斜杠面板。
pub fn open_connect_panel(state: &mut AppState, pane: PaneId) -> bool {
    if !state
        .pane_anywhere(pane)
        .is_some_and(|target| target.view == crate::app::state::PaneView::Lx)
    {
        return false;
    }
    if let Some(target) = state.pane_mut_anywhere(pane) {
        target.lx.clear();
        target.lx.slash = None;
    }
    let mut panel = ConnectPanel::new_with_env_map(pane, &state.connect, &state.connect_env);
    if let Some(provider_id) = state.last_connect_provider.as_deref() {
        panel.select_provider(provider_id);
    }
    state.overlay = Some(Overlay::Connect(Box::new(panel)));
    true
}

/// 当前 `/connect` 模态快照。
fn current_panel(state: &AppState) -> Option<ConnectPanel> {
    match state.overlay.as_ref() {
        Some(Overlay::Connect(panel)) => Some((**panel).clone()),
        _ => None,
    }
}

/// `/connect` 模态按键分派。
pub fn apply_connect_overlay_key(state: &mut AppState, key: OverlayKey) -> bool {
    let Some(panel) = current_panel(state) else {
        return false;
    };
    let validating = panel.validating;
    match key {
        OverlayKey::Esc => apply_connect_esc(state),
        OverlayKey::Enter => {
            if validating {
                return false;
            }
            apply_connect_enter(state)
        }
        OverlayKey::Up => !validating && move_connect_active(state, -1),
        OverlayKey::Down => !validating && move_connect_active(state, 1),
        OverlayKey::Refresh => {
            if validating {
                return false;
            }
            let provider_id = match &panel.stage {
                ConnectStage::Manage { provider_id, .. } => Some(provider_id.clone()),
                _ => None,
            };
            start_models_refresh(state, provider_id)
        }
        OverlayKey::Char(ch) if panel.shortcut_char(ch) => apply_connect_shortcut(state, ch),
        _ => edit_connect_input(state, key),
    }
}

/// 编辑模态输入框；`Providers` 阶段同步重算过滤视图。
fn edit_connect_input(state: &mut AppState, key: OverlayKey) -> bool {
    let Some(Overlay::Connect(panel)) = state.overlay.as_mut() else {
        return false;
    };
    if panel.validating || !panel.has_text_input() || !panel.input.apply_edit_key(key) {
        return false;
    }
    if matches!(panel.stage, ConnectStage::Providers) {
        panel.refilter_from_input();
    } else {
        panel.clear_error();
    }
    true
}

/// 指针悬停行：只记录悬停（铺底色），不移动 `❯` 选中标记。
pub fn hover_connect(state: &mut AppState, hover: Option<usize>) -> bool {
    let Some(Overlay::Connect(panel)) = state.overlay.as_mut() else {
        return false;
    };
    let hover = hover.filter(|index| *index < panel.item_count());
    if panel.hover == hover {
        return false;
    }
    panel.hover = hover;
    true
}

/// 点击行：把 `❯` 移到该可见行（不自动执行动作）。
pub fn set_connect_selection(state: &mut AppState, index: usize) -> bool {
    let Some(Overlay::Connect(panel)) = state.overlay.as_mut() else {
        return false;
    };
    if index >= panel.item_count() {
        return false;
    }
    let mut window = panel.window();
    if window.active == index {
        return false;
    }
    window.set_active(index);
    panel.set_window(window);
    true
}

/// 滚轮滚动模态列表视口（高亮不动、越界钳制）。
pub fn scroll_connect(state: &mut AppState, delta: isize, base: usize, visible: usize) -> bool {
    let Some(Overlay::Connect(panel)) = state.overlay.as_mut() else {
        return false;
    };
    let total = panel.item_count();
    if total <= visible {
        return false;
    }
    let max_offset = total - visible;
    let mut window = panel.window();
    let current = window.viewport.unwrap_or(base).min(max_offset);
    let next = (current as isize + delta).clamp(0, max_offset as isize) as usize;
    if next == current && window.viewport.is_some() {
        return false;
    }
    window.viewport = Some(next);
    panel.set_window(window);
    true
}

/// 设置模态列表的显式视口起点（滚动条轨道/拖拽）。
pub fn set_connect_viewport(state: &mut AppState, offset: usize, visible: usize) -> bool {
    let Some(Overlay::Connect(panel)) = state.overlay.as_mut() else {
        return false;
    };
    let total = panel.item_count();
    if total <= visible {
        return false;
    }
    let next = offset.min(total - visible);
    let mut window = panel.window();
    if window.viewport == Some(next) {
        return false;
    }
    window.viewport = Some(next);
    panel.set_window(window);
    true
}

fn move_connect_active(state: &mut AppState, delta: isize) -> bool {
    let Some(Overlay::Connect(panel)) = state.overlay.as_mut() else {
        return false;
    };
    panel.move_active(delta)
}

/// `/connect` 模态确认。
pub fn apply_connect_enter(state: &mut AppState) -> bool {
    let Some(panel) = current_panel(state) else {
        return false;
    };
    match panel.stage {
        ConnectStage::Providers => enter_provider(state, &panel),
        ConnectStage::Manage { .. } => enter_manage(state, &panel),
        ConnectStage::Methods { .. } => enter_method(state, &panel),
        ConnectStage::InputPrompt { .. } => enter_input_prompt(state, &panel),
        ConnectStage::OAuthWaiting { .. } => enter_oauth_waiting(state, &panel),
    }
}

/// 供应商列表 Enter：记录所选供应商；已连接进入管理菜单，未连接直接进入认证流程。
fn enter_provider(state: &mut AppState, panel: &ConnectPanel) -> bool {
    let Some(entry) = panel.active_entry().cloned() else {
        return true;
    };
    state.last_connect_provider = Some(entry.id.clone());
    let Some(source) = entry.source.clone() else {
        return begin_auth(
            state,
            &entry.id,
            &entry.name,
            entry.methods,
            BTreeMap::new(),
        );
    };
    let actions = vec![
        ConnectManageAction::SelectModel,
        ConnectManageAction::Reconnect,
        ConnectManageAction::RefreshModels,
        // Manage 只在已连接时打开；`$ENV` 连接同样允许 Disconnect（落盘禁用）。
        ConnectManageAction::Disconnect,
    ];
    replace_panel(
        state,
        panel.clone(),
        ConnectStage::Manage {
            provider_id: entry.id,
            provider_name: entry.name,
            source,
            actions,
            window: PanelWindow::default(),
        },
    )
}

/// 管理菜单 Enter：选模型 / 重新认证 / 刷新模型 / 断开连接。
fn enter_manage(state: &mut AppState, panel: &ConnectPanel) -> bool {
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
                .pane_anywhere(panel.pane)
                .and_then(|target| target.lx.chat.effective_selection(&state.models));
            let picker =
                ModelPicker::for_provider(&state.models.settings, current.as_ref(), provider_id);
            state.overlay = None;
            let Some(target) = state.pane_mut_anywhere(panel.pane) else {
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
                provider_id,
                provider_name,
                spec.methods,
                BTreeMap::new(),
            )
        }
        ConnectManageAction::RefreshModels => {
            start_models_refresh(state, Some(provider_id.clone()))
        }
        ConnectManageAction::Disconnect => {
            state.effects.push(Effect::DisconnectPreset {
                pane: panel.pane,
                provider_id: provider_id.clone(),
            });
            true
        }
    }
}

/// 认证方式 Enter：进入所选方法的表单/Key 输入或直接启动 OAuth。
fn enter_method(state: &mut AppState, panel: &ConnectPanel) -> bool {
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
        provider_id,
        provider_name,
        methods,
        method_index,
        BTreeMap::new(),
    )
}

/// 表单/API Key Enter。
fn enter_input_prompt(state: &mut AppState, panel: &ConnectPanel) -> bool {
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
                let value = panel.input.text().trim().to_string();
                if field.required && value.is_empty() {
                    return set_prompt_error(state, panel, text::LX_CONNECT_FIELD_REQUIRED);
                }
                let mut answers = answers.clone();
                if value.is_empty() {
                    answers.remove(field.key);
                } else {
                    answers.insert(field.key.to_string(), value);
                }
                return advance_after_field(
                    state,
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
                provider_id,
                provider_name,
                *method_index,
                answers,
                *field_index,
            )
        }
        ConnectPromptStep::ApiKey => {
            let value = panel.input.text().trim().to_string();
            if value.is_empty() {
                return set_prompt_error(state, panel, text::LX_CONNECT_API_KEY_EMPTY);
            }
            let pane = panel.pane;
            let mut panel = panel.clone();
            panel.validating = true;
            panel.status_message = Some(text::LX_CONNECT_VERIFYING.to_string());
            panel.clear_error();
            state.overlay = Some(Overlay::Connect(Box::new(panel)));
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
fn enter_oauth_waiting(state: &mut AppState, panel: &ConnectPanel) -> bool {
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
        let code = panel.input.text().trim().to_string();
        if !code.is_empty() {
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
        0 => open_browser(state, panel.pane, url),
        1 => copy_oauth_text(state, panel.pane, user_code.as_deref().unwrap_or(url)),
        _ => true,
    }
}

/// OAuth 等待阶段的 `o` / `c` 快捷键（仅自动模式）。
fn apply_connect_shortcut(state: &mut AppState, ch: char) -> bool {
    let Some(panel) = current_panel(state) else {
        return false;
    };
    let ConnectStage::OAuthWaiting { url, user_code, .. } = &panel.stage else {
        return false;
    };
    match ch {
        'o' => open_browser(state, panel.pane, url),
        'c' => copy_oauth_text(state, panel.pane, user_code.as_deref().unwrap_or(url)),
        _ => false,
    }
}

/// `/connect` 模态 Esc：OAuth 取消并退回列表；其余阶段退回列表；列表关闭模态。
pub fn apply_connect_esc(state: &mut AppState) -> bool {
    let Some(panel) = current_panel(state) else {
        return false;
    };
    if let Some(attempt_id) = panel.oauth_attempt_id() {
        state.effects.push(Effect::CancelPresetOAuth {
            attempt_id: attempt_id.to_string(),
        });
    }
    if matches!(panel.stage, ConnectStage::Providers) {
        state.overlay = None;
        return true;
    }
    let mut panel = panel;
    panel.reset_to_providers();
    state.overlay = Some(Overlay::Connect(Box::new(panel)));
    true
}

/// 开始认证：多方法先选方法，单方法直接进入该方法。
fn begin_auth(
    state: &mut AppState,
    provider_id: &str,
    provider_name: &str,
    methods: &'static [PresetAuthMethod],
    answers: BTreeMap<String, String>,
) -> bool {
    if methods.is_empty() {
        if current_panel(state).is_none() {
            return false;
        };
        show_connect_toast(
            state,
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
fn begin_method(
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
fn advance_after_field(
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

fn set_prompt_error(state: &mut AppState, panel: &ConnectPanel, error: &str) -> bool {
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

fn replace_panel(state: &mut AppState, mut panel: ConnectPanel, stage: ConnectStage) -> bool {
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

/// Ctrl+R / `Refresh models`：进入校验态并请求后台刷新。
fn start_models_refresh(state: &mut AppState, provider_id: Option<String>) -> bool {
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

fn open_browser(state: &mut AppState, _pane: PaneId, url: &str) -> bool {
    if url.is_empty() {
        return true;
    }
    state.effects.push(Effect::OpenBrowser {
        url: url.to_string(),
    });
    show_connect_toast(
        state,
        ToastKind::Default,
        text::LX_CONNECT_OAUTH_OPENED.to_string(),
    );
    true
}

fn copy_oauth_text(state: &mut AppState, _pane: PaneId, text_value: &str) -> bool {
    if text_value.is_empty() {
        return true;
    }
    state.effects.push(Effect::CopyToClipboard {
        text: text_value.to_string(),
        anchor: None,
    });
    true
}

/// 连接保存成功：关闭模态、刷新快照、弹全局 Toast，并切到该供应商的 `/model` 面板。
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
    state.last_connect_provider = Some(provider_id.to_string());
    state.models.settings = settings;
    state.connect = connect;
    // 连接后静默同步模型目录（`pane: None` 不弹刷新成功/鉴权失败 Toast，错误 Key 仅在发消息时报错）。
    state.effects.push(Effect::RefreshModelsCatalog {
        pane: None,
        provider_id: Some(provider_id.to_string()),
    });
    state.overlay = None;
    super::refresh_all_lx_status(state);
    show_connect_toast(state, ToastKind::Default, text::lx_connect_connected(&name));
    let picker = ModelPicker::for_provider(&state.models.settings, current.as_ref(), provider_id);
    let Some(target) = state.pane_mut_anywhere(pane) else {
        return true;
    };
    target.lx.clear();
    target.lx.slash = Some(SlashPanel::Model(Box::new(picker)));
    true
}

/// 断开连接完成：刷新快照与模态连接状态，并保持高亮在刚断开的供应商上。
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
    let modal = state.overlay.as_ref().and_then(|overlay| match overlay {
        Overlay::Connect(panel) if panel.pane == pane => Some(panel.pane),
        _ => None,
    });
    state.last_connect_provider = Some(provider_id.to_string());
    state.models.settings = settings;
    state.connect = connect;
    super::refresh_all_lx_status(state);
    show_connect_toast(
        state,
        ToastKind::Default,
        text::lx_connect_disconnected(&name),
    );
    if let Some(pane) = modal {
        let mut refreshed =
            ConnectPanel::new_with_env_map(pane, &state.connect, &state.connect_env);
        refreshed.select_provider(provider_id);
        state.overlay = Some(Overlay::Connect(Box::new(refreshed)));
    }
    true
}

/// 连接保存/校验失败：在输入阶段原地报错并保留输入；其余阶段（如断开失败）弹全局 Toast。
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
    show_connect_toast(state, ToastKind::Danger, text::lx_connect_failed(error));
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
            ToastKind::Danger,
            text::lx_connect_oauth_failed(error),
        );
    }
    applied
}

/// 模型刷新完成：更新运行时快照、清掉校验态并按需弹全局 Toast。
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
    super::refresh_all_lx_status(state);
    if pane.is_some() {
        show_connect_toast(
            state,
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
    if pane.is_some() {
        show_connect_toast(
            state,
            ToastKind::Warning,
            text::lx_connect_refresh_failed(error),
        );
    }
    true
}

/// 统一构造 `/connect` 模态触发的全局 Toast（`anchor: None`，显示在屏幕顶部居中、晚于模态渲染）。
fn show_connect_toast(state: &mut AppState, kind: ToastKind, message: String) {
    crate::app::update::show_toast(state, Toast::new(kind, message, None, Instant::now()));
}
