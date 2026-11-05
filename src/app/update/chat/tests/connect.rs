//! `/connect` 居中模态：供应商过滤、认证推进、表单/API Key 提交、OAuth 与模型刷新。

use super::*;
use crate::app::actions::OverlayKey;
use crate::app::overlay::{Overlay, TextInput};
use crate::app::state::{
    ConnectManageAction, ConnectPanel, ConnectPromptStep, ConnectStage, PanelWindow, SlashCommand,
};
use crate::app::update::{apply_connect_overlay_key, open_connect_panel};
use crate::config::ai::connect::{ConnectSettings, ConnectedPresetRecord, ConnectionSource};
use crate::config::ai::oauth::{OAuthPromptMode, OAuthStarted};
use crate::config::ai::presets::find_preset;

fn connect_state() -> (AppState, PaneId) {
    let state = ready_state("");
    let pane = focus_pane(&state);
    (state, pane)
}

fn open(state: &mut AppState, pane: PaneId) {
    assert!(open_connect_panel(state, pane));
    assert!(matches!(state.overlay, Some(Overlay::Connect(_))));
    assert!(
        state.pane_anywhere(pane).expect("pane").lx.slash.is_none(),
        "`/connect` 不再占用斜杠面板"
    );
    assert!(state.pane_anywhere(pane).expect("pane").lx.input.is_empty());
}

fn connect_panel(state: &AppState) -> ConnectPanel {
    match state.overlay.as_ref() {
        Some(Overlay::Connect(panel)) => (**panel).clone(),
        other => panic!("connect overlay expected, got {other:?}"),
    }
}

fn connect_panel_mut(state: &mut AppState) -> &mut ConnectPanel {
    match state.overlay.as_mut() {
        Some(Overlay::Connect(panel)) => panel,
        other => panic!("connect overlay expected, got {other:?}"),
    }
}

fn select_provider(state: &mut AppState, id: &str) {
    let panel = connect_panel_mut(state);
    panel.input = TextInput::new(id);
    panel.refilter_from_input();
    panel.window = PanelWindow::new(0);
    assert_eq!(
        panel.active_entry().map(|entry| entry.id.as_str()),
        Some(id),
        "按 id 搜索后选中目标供应商"
    );
}

fn type_input(state: &mut AppState, pane: PaneId, text: &str) {
    if let Some(target) = state.pane_mut_anywhere(pane) {
        target.lx.input = text.to_string();
        target.lx.cursor = text.len();
    }
    connect_panel_mut(state).input = TextInput::new(text);
}

fn input(state: &AppState) -> String {
    connect_panel(state).input.text().to_string()
}

fn settings_for(provider_id: &str, provider_name: &str) -> AiSettings {
    let mut settings = AiSettings::empty();
    settings.enabled_providers = vec![provider_id.into()];
    settings.providers.insert(
        provider_id.into(),
        ProviderConfig {
            id: provider_id.into(),
            kind: ProviderTransport::OpenaiCompatible,
            name: provider_name.into(),
            options: ProviderOptions {
                api_key: Some("test-key".into()),
                base_url: "https://api.example.invalid/v1".into(),
            },
            models: std::iter::once((
                "m".into(),
                ModelConfig {
                    id: "m".into(),
                    name: "M".into(),
                    transport: None,
                    limit: None,
                    modalities: None,
                    variants: std::collections::BTreeMap::new(),
                    variant: None,
                    pricing: None,
                    extra: serde_json::Map::new(),
                },
            ))
            .collect(),
            extra: serde_json::Map::new(),
        },
    );
    settings
}

#[test]
fn connect_command_opens_centered_overlay() {
    let (mut state, pane) = connect_state();
    assert!(apply_slash_command(&mut state, pane, SlashCommand::Connect));
    let panel = connect_panel(&state);
    assert_eq!(panel.stage, ConnectStage::Providers);
    assert_eq!(panel.pane, pane);
    assert!(panel.entries.len() > 10);
    assert_eq!(panel.filtered.len(), panel.entries.len());
}

#[test]
fn typing_in_modal_input_filters_provider_list_in_real_time() {
    let (mut state, pane) = connect_state();
    open(&mut state, pane);
    for ch in "github".chars() {
        apply_connect_overlay_key(&mut state, OverlayKey::Char(ch));
    }
    let panel = connect_panel(&state);
    assert_eq!(panel.filter, "github");
    assert!(
        panel
            .visible()
            .iter()
            .all(|entry| entry.id.contains("github")
                || entry.name.to_ascii_lowercase().contains("github")),
        "只保留命中的供应商"
    );
    assert!(panel.active_entry().is_some());
    apply_connect_overlay_key(&mut state, OverlayKey::DeleteToLineStart);
    assert_eq!(
        connect_panel(&state).filtered.len(),
        connect_panel(&state).entries.len()
    );
}

#[test]
fn single_method_provider_goes_straight_to_api_key_prompt() {
    let (mut state, pane) = connect_state();
    open(&mut state, pane);
    select_provider(&mut state, "anthropic");
    assert!(apply_connect_overlay_key(&mut state, OverlayKey::Enter));
    let panel = connect_panel(&state);
    assert!(matches!(
        panel.stage,
        ConnectStage::InputPrompt {
            step: ConnectPromptStep::ApiKey,
            ..
        }
    ));

    type_input(&mut state, pane, "sk-ant-test");
    assert!(apply_connect_overlay_key(&mut state, OverlayKey::Enter));
    let panel = connect_panel(&state);
    assert!(panel.validating, "提交后进入校验态");
    assert_eq!(
        panel.status_message.as_deref(),
        Some(text::LX_CONNECT_VERIFYING)
    );
    assert_eq!(panel.input.text(), "sk-ant-test", "失败时可原地修改");
    let saved = state
        .effects
        .iter()
        .find_map(|effect| match effect {
            crate::app::effects::Effect::SavePresetConnection {
                provider_id,
                api_key,
                form_answers,
                ..
            } => Some((provider_id.clone(), api_key.clone(), form_answers.clone())),
            _ => None,
        })
        .expect("SavePresetConnection effect");
    assert_eq!(saved.0, "anthropic");
    assert_eq!(saved.1.as_deref(), Some("sk-ant-test"));
    assert!(saved.2.is_empty());
}

#[test]
fn empty_api_key_is_rejected_in_modal() {
    let (mut state, pane) = connect_state();
    open(&mut state, pane);
    select_provider(&mut state, "anthropic");
    apply_connect_overlay_key(&mut state, OverlayKey::Enter);
    assert!(apply_connect_overlay_key(&mut state, OverlayKey::Enter));
    assert!(state.effects.iter().all(|effect| !matches!(
        effect,
        crate::app::effects::Effect::SavePresetConnection { .. }
    )));
    let panel = connect_panel(&state);
    assert_eq!(panel.current_error(), Some(text::LX_CONNECT_API_KEY_EMPTY));
    assert!(!panel.validating);
    assert!(!panel.entries.is_empty(), "模态保持打开");
    assert!(
        state
            .pane_anywhere(pane)
            .and_then(|target| target.lx.toast.as_ref())
            .is_none(),
        "错误显示在模态内，不再弹 Toast"
    );
}

#[test]
fn multi_method_provider_opens_methods_with_oauth_first() {
    let (mut state, pane) = connect_state();
    open(&mut state, pane);
    select_provider(&mut state, "openai");
    apply_connect_overlay_key(&mut state, OverlayKey::Enter);
    let panel = connect_panel(&state);
    let ConnectStage::Methods {
        methods, window, ..
    } = panel.stage
    else {
        panic!("methods stage");
    };
    assert!(methods.len() > 1);
    assert!(methods[0].oauth_flow().is_some(), "OAuth 排在前面");
    assert!(window.active < methods.len(), "高亮落在有效方法上");

    assert!(apply_connect_overlay_key(&mut state, OverlayKey::Esc));
    let panel = connect_panel(&state);
    assert_eq!(panel.stage, ConnectStage::Providers);
    assert!(panel.input.text().is_empty());
}

#[test]
fn form_option_selects_enterprise_and_asks_for_url() {
    let (mut state, pane) = connect_state();
    open(&mut state, pane);
    select_provider(&mut state, "github-copilot");
    apply_connect_overlay_key(&mut state, OverlayKey::Enter);
    let panel = connect_panel(&state);
    let ConnectStage::InputPrompt {
        step: ConnectPromptStep::FormField { field, .. },
        ..
    } = &panel.stage
    else {
        panic!("form field");
    };
    assert_eq!(field.options[0].value, "github.com");

    connect_panel_mut(&mut state).move_active(1);
    assert!(apply_connect_overlay_key(&mut state, OverlayKey::Enter));
    let panel = connect_panel(&state);
    let ConnectStage::InputPrompt {
        step: ConnectPromptStep::FormField { field, .. },
        answers,
        ..
    } = &panel.stage
    else {
        panic!("enterprise URL form field");
    };
    assert_eq!(field.key, "enterpriseUrl");
    assert_eq!(
        answers.get("deploymentType").map(String::as_str),
        Some("enterprise")
    );

    type_input(&mut state, pane, "company.ghe.com");
    assert!(apply_connect_overlay_key(&mut state, OverlayKey::Enter));
    let panel = connect_panel(&state);
    assert!(matches!(panel.stage, ConnectStage::OAuthWaiting { .. }));
    let started = state
        .effects
        .iter()
        .find_map(|effect| match effect {
            crate::app::effects::Effect::StartPresetOAuth {
                provider_id,
                form_answers,
                ..
            } => Some((provider_id.clone(), form_answers.clone())),
            _ => None,
        })
        .expect("StartPresetOAuth effect");
    assert_eq!(started.0, "github-copilot");
    assert_eq!(
        started.1.get("enterpriseUrl").map(String::as_str),
        Some("company.ghe.com")
    );
}

#[test]
fn free_text_form_fields_then_api_key_submits_answers() {
    let (mut state, pane) = connect_state();
    open(&mut state, pane);
    select_provider(&mut state, "cloudflare-ai-gateway");
    apply_connect_overlay_key(&mut state, OverlayKey::Enter);
    type_input(&mut state, pane, "account-123");
    assert!(apply_connect_overlay_key(&mut state, OverlayKey::Enter));
    type_input(&mut state, pane, "gateway-456");
    assert!(apply_connect_overlay_key(&mut state, OverlayKey::Enter));
    assert!(matches!(
        connect_panel(&state).stage,
        ConnectStage::InputPrompt {
            step: ConnectPromptStep::ApiKey,
            ..
        }
    ));
    type_input(&mut state, pane, "cf-token");
    assert!(apply_connect_overlay_key(&mut state, OverlayKey::Enter));
    let (api_key, answers) = state
        .effects
        .iter()
        .find_map(|effect| match effect {
            crate::app::effects::Effect::SavePresetConnection {
                api_key,
                form_answers,
                ..
            } => Some((api_key.clone(), form_answers.clone())),
            _ => None,
        })
        .expect("save effect");
    assert_eq!(api_key.as_deref(), Some("cf-token"));
    assert_eq!(
        answers.get("accountId").map(String::as_str),
        Some("account-123")
    );
    assert_eq!(
        answers.get("gatewayId").map(String::as_str),
        Some("gateway-456")
    );
}

#[test]
fn connected_provider_opens_manage_with_refresh_and_disconnect() {
    let (mut state, pane) = connect_state();
    let mut connect = ConnectSettings::default();
    let mut record = ConnectedPresetRecord::new("anthropic");
    record.api_key = Some("stored-key".into());
    connect.providers.insert("anthropic".into(), record);
    state.connect = connect;
    open(&mut state, pane);
    select_provider(&mut state, "anthropic");
    assert_eq!(
        connect_panel(&state)
            .active_entry()
            .and_then(|entry| entry.source.clone()),
        Some(ConnectionSource::ApiKey)
    );
    assert!(apply_connect_overlay_key(&mut state, OverlayKey::Enter));
    let ConnectStage::Manage { actions, .. } = connect_panel(&state).stage else {
        panic!("manage stage");
    };
    assert!(actions.contains(&ConnectManageAction::SelectModel));
    assert!(actions.contains(&ConnectManageAction::Reconnect));
    assert!(actions.contains(&ConnectManageAction::RefreshModels));
    assert!(actions.contains(&ConnectManageAction::Disconnect));

    connect_panel_mut(&mut state).move_active(3);
    assert!(apply_connect_overlay_key(&mut state, OverlayKey::Enter));
    assert!(state.effects.iter().any(|effect| matches!(
        effect,
        crate::app::effects::Effect::DisconnectPreset { provider_id, .. } if provider_id == "anthropic"
    )));
}

#[test]
fn manage_refresh_models_enters_validating_and_emits_effect() {
    let (mut state, pane) = connect_state();
    let mut connect = ConnectSettings::default();
    let mut record = ConnectedPresetRecord::new("anthropic");
    record.api_key = Some("stored-key".into());
    connect.providers.insert("anthropic".into(), record);
    state.connect = connect;
    open(&mut state, pane);
    select_provider(&mut state, "anthropic");
    apply_connect_overlay_key(&mut state, OverlayKey::Enter);
    connect_panel_mut(&mut state).move_active(2);
    assert!(apply_connect_overlay_key(&mut state, OverlayKey::Enter));
    let panel = connect_panel(&state);
    assert!(panel.validating);
    assert_eq!(
        panel.status_message.as_deref(),
        Some(text::LX_CONNECT_REFRESHING)
    );
    assert!(state.effects.iter().any(|effect| matches!(
        effect,
        crate::app::effects::Effect::RefreshModelsCatalog {
            provider_id: Some(provider_id),
            ..
        } if provider_id == "anthropic"
    )));
}

#[test]
fn ctrl_r_in_provider_list_refreshes_all_connections() {
    let (mut state, pane) = connect_state();
    open(&mut state, pane);
    assert!(apply_connect_overlay_key(&mut state, OverlayKey::Refresh));
    assert!(connect_panel(&state).validating);
    assert!(state.effects.iter().any(|effect| matches!(
        effect,
        crate::app::effects::Effect::RefreshModelsCatalog {
            provider_id: None,
            ..
        }
    )));
    assert!(
        !apply_connect_overlay_key(&mut state, OverlayKey::Enter),
        "刷新中阻止重复提交"
    );
}

#[test]
fn manage_select_model_closes_overlay_and_opens_model_picker() {
    let (mut state, pane) = connect_state();
    let mut connect = ConnectSettings::default();
    let mut record = ConnectedPresetRecord::new("anthropic");
    record.api_key = Some("stored-key".into());
    connect.providers.insert("anthropic".into(), record);
    state.connect = connect;
    state.models.settings = settings_for("anthropic", "Anthropic");
    open(&mut state, pane);
    select_provider(&mut state, "anthropic");
    apply_connect_overlay_key(&mut state, OverlayKey::Enter);
    assert!(apply_connect_overlay_key(&mut state, OverlayKey::Enter));
    assert!(state.overlay.is_none());
    let Some(SlashPanel::Model(picker)) = state
        .pane_anywhere(pane)
        .and_then(|target| target.lx.slash.clone())
    else {
        panic!("model panel");
    };
    assert!(
        picker
            .candidates
            .iter()
            .all(|candidate| candidate.selection.provider == "anthropic")
    );
}

#[test]
fn connected_event_closes_overlay_switches_to_model_picker_and_toasts() {
    let (mut state, pane) = connect_state();
    open(&mut state, pane);
    state.connect = ConnectSettings::default();
    let settings = settings_for("anthropic", "Anthropic");
    let connect = state.connect.clone();
    assert!(crate::app::update::on_preset_connected(
        &mut state,
        pane,
        "anthropic",
        connect,
        settings
    ));
    assert!(state.overlay.is_none(), "连接成功后关闭模态");
    let toast = state
        .pane_anywhere(pane)
        .and_then(|target| target.lx.toast.as_ref())
        .expect("pane toast");
    assert_eq!(toast.message, "Connected Anthropic");
    let Some(SlashPanel::Model(picker)) = state
        .pane_anywhere(pane)
        .and_then(|target| target.lx.slash.clone())
    else {
        panic!("model panel");
    };
    assert!(
        picker
            .candidates
            .iter()
            .all(|candidate| candidate.selection.provider == "anthropic"),
        "只展示刚连接供应商的模型"
    );
}

#[test]
fn disconnected_event_restores_provider_list() {
    let (mut state, pane) = connect_state();
    open(&mut state, pane);
    select_provider(&mut state, "anthropic");
    state
        .connect
        .providers
        .insert("anthropic".into(), ConnectedPresetRecord::new("anthropic"));
    assert!(crate::app::update::on_preset_disconnected(
        &mut state,
        pane,
        "anthropic",
        ConnectSettings::default(),
        settings_for("anthropic", "Anthropic"),
    ));
    let panel = connect_panel(&state);
    assert_eq!(panel.stage, ConnectStage::Providers);
    assert_eq!(
        panel.active_entry().and_then(|entry| entry.source.clone()),
        None,
        "断开后不再显示已连接标签"
    );
    let toast = state
        .pane_anywhere(pane)
        .and_then(|target| target.lx.toast.as_ref())
        .expect("pane toast");
    assert_eq!(toast.message, "Disconnected Anthropic");
}

#[test]
fn connect_failure_sets_inline_error_and_keeps_input() {
    let (mut state, pane) = connect_state();
    open(&mut state, pane);
    select_provider(&mut state, "anthropic");
    apply_connect_overlay_key(&mut state, OverlayKey::Enter);
    type_input(&mut state, pane, "bad-key");
    apply_connect_overlay_key(&mut state, OverlayKey::Enter);
    crate::app::update::on_connect_failed(&mut state, pane, "Invalid API key (HTTP 401)");
    let panel = connect_panel(&state);
    assert!(!panel.validating);
    assert!(panel.status_message.is_none());
    assert_eq!(panel.current_error(), Some("Invalid API key (HTTP 401)"));
    assert_eq!(input(&state), "bad-key", "保留输入以便原地修改");
    assert!(state.overlay.is_some(), "失败不关闭模态");
}

#[test]
fn models_refreshed_clears_validating_and_toasts() {
    let (mut state, pane) = connect_state();
    open(&mut state, pane);
    apply_connect_overlay_key(&mut state, OverlayKey::Refresh);
    assert!(connect_panel(&state).validating);
    assert!(crate::app::update::on_models_refreshed(
        &mut state,
        Some(pane),
        7,
        AiSettings::empty(),
        ConnectSettings::default(),
    ));
    let panel = connect_panel(&state);
    assert!(!panel.validating);
    assert!(panel.status_message.is_none());
    let toast = state
        .pane_anywhere(pane)
        .and_then(|target| target.lx.toast.as_ref())
        .expect("pane toast");
    assert_eq!(toast.message, "Refreshed 7 models");
}

#[test]
fn oauth_prompt_fills_url_and_shortcuts_open_browser() {
    let (mut state, pane) = connect_state();
    open(&mut state, pane);
    select_provider(&mut state, "github-copilot");
    apply_connect_overlay_key(&mut state, OverlayKey::Enter);
    // github.com 是默认选项：选企业的路径已在另一个测试覆盖；这里直接选默认项启动。
    assert!(apply_connect_overlay_key(&mut state, OverlayKey::Enter));
    assert!(matches!(
        connect_panel(&state).stage,
        ConnectStage::OAuthWaiting { .. }
    ));
    assert!(crate::app::update::on_preset_oauth_prompt(
        &mut state,
        pane,
        "github-copilot",
        "device",
        OAuthStarted {
            attempt_id: "attempt-1".into(),
            url: "https://github.com/login/device".into(),
            user_code: Some("ABCD-1234".into()),
            instructions: "Enter code: ABCD-1234".into(),
            mode: OAuthPromptMode::Auto,
        }
    ));
    let panel = connect_panel(&state);
    let ConnectStage::OAuthWaiting { url, user_code, .. } = &panel.stage else {
        panic!("oauth waiting");
    };
    assert_eq!(url, "https://github.com/login/device");
    assert_eq!(user_code.as_deref(), Some("ABCD-1234"));

    assert!(apply_connect_overlay_key(&mut state, OverlayKey::Char('c')));
    assert!(state.effects.iter().any(|effect| matches!(
        effect,
        crate::app::effects::Effect::CopyToClipboard { text, .. } if text == "ABCD-1234"
    )));
    assert!(apply_connect_overlay_key(&mut state, OverlayKey::Char('o')));
    assert!(state.effects.iter().any(|effect| matches!(
        effect,
        crate::app::effects::Effect::OpenBrowser { url } if url == "https://github.com/login/device"
    )));
}

#[test]
fn oauth_code_mode_submits_typed_code() {
    let (mut state, pane) = connect_state();
    open(&mut state, pane);
    select_provider(&mut state, "github-copilot");
    apply_connect_overlay_key(&mut state, OverlayKey::Enter);
    apply_connect_overlay_key(&mut state, OverlayKey::Enter);
    crate::app::update::on_preset_oauth_prompt(
        &mut state,
        pane,
        "github-copilot",
        "device",
        OAuthStarted {
            attempt_id: "attempt-code".into(),
            url: "https://github.com/login/device".into(),
            user_code: None,
            instructions: "Paste the authorization code".into(),
            mode: OAuthPromptMode::Code,
        },
    );
    type_input(&mut state, pane, "one-time-code");
    assert!(apply_connect_overlay_key(&mut state, OverlayKey::Enter));
    assert!(state.effects.iter().any(|effect| matches!(
        effect,
        crate::app::effects::Effect::SubmitPresetOAuthCode { attempt_id, code }
            if attempt_id == "attempt-code" && code == "one-time-code"
    )));
}

#[test]
fn esc_cancels_oauth_and_returns_to_providers() {
    let (mut state, pane) = connect_state();
    open(&mut state, pane);
    select_provider(&mut state, "github-copilot");
    apply_connect_overlay_key(&mut state, OverlayKey::Enter);
    apply_connect_overlay_key(&mut state, OverlayKey::Enter);
    crate::app::update::on_preset_oauth_prompt(
        &mut state,
        pane,
        "github-copilot",
        "device",
        OAuthStarted {
            attempt_id: "attempt-cancel".into(),
            url: "https://github.com/login/device".into(),
            user_code: None,
            instructions: String::new(),
            mode: OAuthPromptMode::Auto,
        },
    );
    assert!(apply_connect_overlay_key(&mut state, OverlayKey::Esc));
    assert!(state.effects.iter().any(|effect| matches!(
        effect,
        crate::app::effects::Effect::CancelPresetOAuth { attempt_id } if attempt_id == "attempt-cancel"
    )));
    assert_eq!(connect_panel(&state).stage, ConnectStage::Providers);
}

#[test]
fn esc_in_providers_closes_overlay() {
    let (mut state, pane) = connect_state();
    open(&mut state, pane);
    assert!(apply_connect_overlay_key(&mut state, OverlayKey::Esc));
    assert!(state.overlay.is_none());
}

#[test]
fn preset_lookup_used_by_panel_is_available() {
    let spec = find_preset("anthropic").expect("anthropic preset");
    assert_eq!(spec.name, "Anthropic");
}
