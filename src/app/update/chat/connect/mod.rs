//! `/connect` 居中模态：供应商选择、认证推进与交互分派。
//!
//! 异步回执（连接/断开/OAuth/模型刷新）的状态收敛在 [`events`]。

use std::collections::BTreeMap;

use crate::app::actions::OverlayKey;
use crate::app::effects::Effect;
use crate::app::overlay::Overlay;
use crate::app::state::{
    AppState, ConnectManageAction, ConnectPanel, ConnectPromptStep, ConnectStage, ModelPicker,
    PanelWindow, SlashPanel,
};
use crate::config::ai::oauth::OAuthPromptMode;
use crate::config::ai::presets::find_preset;
use crate::layout::PaneId;
use crate::ui::text;

mod events;

use events::{
    advance_after_field, begin_auth, begin_method, copy_oauth_text, open_browser, replace_panel,
    set_prompt_error, start_models_refresh,
};
pub use events::{
    on_connect_failed, on_models_refresh_failed, on_models_refreshed, on_preset_connected,
    on_preset_disconnected, on_preset_oauth_failed, on_preset_oauth_prompt,
};

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
    if key == OverlayKey::Esc && crate::app::update::escape_clear_selection(state) {
        return true;
    }
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
    let provider_id = {
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
        matches!(panel.stage, ConnectStage::Providers)
            .then(|| panel.active_entry().map(|entry| entry.id.clone()))
            .flatten()
    };
    if let Some(provider_id) = provider_id {
        state.last_connect_provider = Some(provider_id);
    }
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
    let provider_id = {
        let Some(Overlay::Connect(panel)) = state.overlay.as_mut() else {
            return false;
        };
        if !panel.move_active(delta) {
            return false;
        }
        matches!(panel.stage, ConnectStage::Providers)
            .then(|| panel.active_entry().map(|entry| entry.id.clone()))
            .flatten()
    };
    if let Some(provider_id) = provider_id {
        state.last_connect_provider = Some(provider_id);
    }
    true
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
            // 断开在后台落盘：先锁住模态，避免连按 Enter 重复触发。
            let mut disconnecting = panel.clone();
            disconnecting.validating = true;
            disconnecting.status_message = Some("Disconnecting…".to_string());
            let pane = disconnecting.pane;
            state.overlay = Some(Overlay::Connect(Box::new(disconnecting)));
            state.effects.push(Effect::DisconnectPreset {
                pane,
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

/// `/connect` 模态 Esc：逐级退回。
///
/// OAuth 等待与表单/Key 输入阶段先退回认证方式选择（多方法供应商），再退回供应商列表；
/// 单方法供应商、管理菜单与方式选择阶段直接退回列表；列表阶段关闭模态。
pub fn apply_connect_esc(state: &mut AppState) -> bool {
    let Some(panel) = current_panel(state) else {
        return false;
    };
    if let Some(attempt_id) = panel.oauth_attempt_id() {
        state.effects.push(Effect::CancelPresetOAuth {
            attempt_id: attempt_id.to_string(),
        });
    }
    let stage = panel.stage.clone();
    match stage {
        ConnectStage::Providers => {
            state.overlay = None;
            true
        }
        ConnectStage::Manage { .. } | ConnectStage::Methods { .. } => {
            reset_panel_to_providers(state, panel)
        }
        ConnectStage::OAuthWaiting {
            provider_id,
            provider_name,
            method_id,
            ..
        } => {
            let method_index = find_preset(&provider_id)
                .and_then(|spec| {
                    spec.methods
                        .iter()
                        .position(|method| method.id() == method_id)
                })
                .unwrap_or(0);
            back_from_method(state, panel, &provider_id, &provider_name, method_index)
        }
        ConnectStage::InputPrompt {
            provider_id,
            provider_name,
            method_index,
            ..
        } => back_from_method(state, panel, &provider_id, &provider_name, method_index),
    }
}

/// 输入/OAuth 阶段退回：多方法回到认证方式选择并高亮当前方法，否则回到供应商列表。
fn back_from_method(
    state: &mut AppState,
    panel: ConnectPanel,
    provider_id: &str,
    provider_name: &str,
    method_index: usize,
) -> bool {
    let Some(spec) = find_preset(provider_id) else {
        return reset_panel_to_providers(state, panel);
    };
    if spec.methods.len() <= 1 {
        return reset_panel_to_providers(state, panel);
    }
    let active = method_index.min(spec.methods.len().saturating_sub(1));
    replace_panel(
        state,
        panel,
        ConnectStage::Methods {
            provider_id: provider_id.to_string(),
            provider_name: provider_name.to_string(),
            methods: spec.methods,
            window: PanelWindow::new(active),
        },
    )
}

/// 退回供应商列表并保持之前选中的供应商高亮。
fn reset_panel_to_providers(state: &mut AppState, mut panel: ConnectPanel) -> bool {
    panel.reset_to_providers();
    state.overlay = Some(Overlay::Connect(Box::new(panel)));
    true
}
