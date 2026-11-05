//! `/connect` 状态机：供应商过滤、认证方式推进、表单/API Key 提交与管理动作。

use super::*;
use crate::app::state::{
    ConnectManageAction, ConnectPanel, ConnectPromptStep, ConnectStage, PanelWindow, SlashCommand,
};
use crate::app::update::{apply_connect_enter, apply_connect_esc, open_connect_panel};
use crate::config::ai::connect::{
    ConnectSettings, ConnectedPresetRecord, ConnectionSource, OAuthCredential,
};
use crate::config::ai::oauth::{OAuthPromptMode, OAuthStarted};
use crate::config::ai::presets::find_preset;

fn connect_state() -> (AppState, PaneId) {
    let state = ready_state("");
    let pane = focus_pane(&state);
    (state, pane)
}

fn open(state: &mut AppState, pane: PaneId) {
    assert!(open_connect_panel(state, pane));
    assert!(matches!(
        state
            .pane_anywhere(pane)
            .and_then(|target| target.lx.slash.clone()),
        Some(SlashPanel::Connect(_))
    ));
}

fn connect_panel(state: &AppState, pane: PaneId) -> ConnectPanel {
    match state
        .pane_anywhere(pane)
        .and_then(|target| target.lx.slash.clone())
    {
        Some(SlashPanel::Connect(panel)) => *panel,
        other => panic!("connect panel expected, got {other:?}"),
    }
}

fn select_provider(state: &mut AppState, pane: PaneId, id: &str) {
    let target = state.pane_mut_anywhere(pane).expect("pane");
    let Some(SlashPanel::Connect(panel)) = target.lx.slash.as_mut() else {
        panic!("connect panel");
    };
    panel.refilter(id);
    panel.window = PanelWindow::new(0);
    assert_eq!(
        panel.active_entry().map(|entry| entry.id.as_str()),
        Some(id),
        "按 id 搜索后选中目标供应商"
    );
}

fn type_input(state: &mut AppState, pane: PaneId, text: &str) {
    let target = state.pane_mut_anywhere(pane).expect("pane");
    target.lx.input = text.to_string();
    target.lx.cursor = text.len();
}

fn input(state: &AppState, pane: PaneId) -> String {
    state
        .pane_anywhere(pane)
        .map(|target| target.lx.input.clone())
        .unwrap_or_default()
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
fn connect_command_opens_provider_panel() {
    let (mut state, pane) = connect_state();
    assert!(apply_slash_command(&mut state, pane, SlashCommand::Connect));
    let panel = connect_panel(&state, pane);
    assert_eq!(panel.stage, ConnectStage::Providers);
    assert!(panel.entries.len() > 10);
    assert_eq!(panel.filtered.len(), panel.entries.len());
}

#[test]
fn typing_in_input_filters_provider_list_in_real_time() {
    let (mut state, pane) = connect_state();
    open(&mut state, pane);
    type_input(&mut state, pane, "github");
    crate::app::update::refresh_slash_panel(&mut state, pane);
    let panel = connect_panel(&state, pane);
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
    type_input(&mut state, pane, "");
    crate::app::update::refresh_slash_panel(&mut state, pane);
    assert_eq!(
        connect_panel(&state, pane).filtered.len(),
        connect_panel(&state, pane).entries.len()
    );
}

#[test]
fn slash_typed_in_connect_filter_keeps_connect_panel() {
    let (mut state, pane) = connect_state();
    open(&mut state, pane);
    type_input(&mut state, pane, "/model");
    crate::app::update::refresh_slash_panel(&mut state, pane);
    assert!(
        matches!(
            state
                .pane_anywhere(pane)
                .and_then(|target| target.lx.slash.clone()),
            Some(SlashPanel::Connect(_))
        ),
        "`/connect` 输入框里的 `/` 是过滤词，不切换成命令候选"
    );
}

#[test]
fn single_method_provider_goes_straight_to_api_key_prompt() {
    let (mut state, pane) = connect_state();
    open(&mut state, pane);
    select_provider(&mut state, pane, "anthropic");
    assert!(apply_connect_enter(&mut state, pane));
    let panel = connect_panel(&state, pane);
    assert!(matches!(
        panel.stage,
        ConnectStage::InputPrompt {
            step: ConnectPromptStep::ApiKey,
            ..
        }
    ));

    type_input(&mut state, pane, "sk-ant-test");
    assert!(apply_connect_enter(&mut state, pane));
    assert!(input(&state, pane).is_empty(), "提交后清空输入框");
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
fn empty_api_key_is_rejected_with_toast() {
    let (mut state, pane) = connect_state();
    open(&mut state, pane);
    select_provider(&mut state, pane, "anthropic");
    apply_connect_enter(&mut state, pane);
    assert!(apply_connect_enter(&mut state, pane));
    assert!(state.effects.iter().all(|effect| !matches!(
        effect,
        crate::app::effects::Effect::SavePresetConnection { .. }
    )));
    let toast = state
        .pane_anywhere(pane)
        .and_then(|target| target.lx.toast.as_ref())
        .expect("pane toast");
    assert_eq!(toast.message, text::LX_CONNECT_API_KEY_EMPTY);
}

#[test]
fn multi_method_provider_opens_methods_with_oauth_first() {
    let (mut state, pane) = connect_state();
    open(&mut state, pane);
    select_provider(&mut state, pane, "openai");
    apply_connect_enter(&mut state, pane);
    let panel = connect_panel(&state, pane);
    let ConnectStage::Methods {
        methods, window, ..
    } = panel.stage
    else {
        panic!("methods stage");
    };
    assert!(methods.len() > 1);
    assert!(methods[0].oauth_flow().is_some(), "OAuth 排在前面");
    assert!(window.active < methods.len(), "高亮落在有效方法上");

    assert!(apply_connect_esc(&mut state, pane));
    let panel = connect_panel(&state, pane);
    assert_eq!(panel.stage, ConnectStage::Providers);
    assert!(input(&state, pane).is_empty());
}

#[test]
fn form_option_selects_enterprise_and_asks_for_url() {
    let (mut state, pane) = connect_state();
    open(&mut state, pane);
    select_provider(&mut state, pane, "github-copilot");
    apply_connect_enter(&mut state, pane);
    let panel = connect_panel(&state, pane);
    let ConnectStage::InputPrompt {
        step: ConnectPromptStep::FormField { field, .. },
        ..
    } = &panel.stage
    else {
        panic!("form field");
    };
    assert_eq!(field.options[0].value, "github.com");

    let target = state.pane_mut_anywhere(pane).expect("pane");
    let Some(SlashPanel::Connect(panel)) = target.lx.slash.as_mut() else {
        panic!("connect");
    };
    panel.move_active(1);
    assert!(apply_connect_enter(&mut state, pane));
    let panel = connect_panel(&state, pane);
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
    assert!(apply_connect_enter(&mut state, pane));
    let panel = connect_panel(&state, pane);
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
    select_provider(&mut state, pane, "cloudflare-ai-gateway");
    apply_connect_enter(&mut state, pane);
    type_input(&mut state, pane, "account-123");
    assert!(apply_connect_enter(&mut state, pane));
    type_input(&mut state, pane, "gateway-456");
    assert!(apply_connect_enter(&mut state, pane));
    assert!(matches!(
        connect_panel(&state, pane).stage,
        ConnectStage::InputPrompt {
            step: ConnectPromptStep::ApiKey,
            ..
        }
    ));
    type_input(&mut state, pane, "cf-token");
    assert!(apply_connect_enter(&mut state, pane));
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
fn connected_provider_opens_manage_with_disconnect() {
    let (mut state, pane) = connect_state();
    let mut connect = ConnectSettings::default();
    let mut record = ConnectedPresetRecord::new("anthropic");
    record.api_key = Some("stored-key".into());
    connect.providers.insert("anthropic".into(), record);
    state.connect = connect;
    open(&mut state, pane);
    select_provider(&mut state, pane, "anthropic");
    assert_eq!(
        connect_panel(&state, pane)
            .active_entry()
            .and_then(|entry| entry.source.clone()),
        Some(ConnectionSource::ApiKey)
    );
    assert!(apply_connect_enter(&mut state, pane));
    let ConnectStage::Manage { actions, .. } = connect_panel(&state, pane).stage else {
        panic!("manage stage");
    };
    assert!(actions.contains(&ConnectManageAction::SelectModel));
    assert!(actions.contains(&ConnectManageAction::Reconnect));
    assert!(actions.contains(&ConnectManageAction::Disconnect));

    let target = state.pane_mut_anywhere(pane).expect("pane");
    let Some(SlashPanel::Connect(panel)) = target.lx.slash.as_mut() else {
        panic!("connect");
    };
    panel.move_active(2);
    assert!(apply_connect_enter(&mut state, pane));
    assert!(state.effects.iter().any(|effect| matches!(
        effect,
        crate::app::effects::Effect::DisconnectPreset { provider_id, .. } if provider_id == "anthropic"
    )));
}

#[test]
fn reconnect_from_manage_returns_to_method_form() {
    let (mut state, pane) = connect_state();
    let mut connect = ConnectSettings::default();
    let mut record = ConnectedPresetRecord::new("openai");
    record.oauth = Some(OAuthCredential {
        method_id: "chatgpt-browser".into(),
        access_token: "token".into(),
        refresh_token: None,
        expires_at_ms: None,
        account_label: None,
        metadata: std::collections::BTreeMap::new(),
    });
    connect.providers.insert("openai".into(), record);
    state.connect = connect;
    open(&mut state, pane);
    select_provider(&mut state, pane, "openai");
    apply_connect_enter(&mut state, pane);
    let target = state.pane_mut_anywhere(pane).expect("pane");
    let Some(SlashPanel::Connect(panel)) = target.lx.slash.as_mut() else {
        panic!("connect");
    };
    panel.move_active(1);
    assert!(apply_connect_enter(&mut state, pane));
    assert!(matches!(
        connect_panel(&state, pane).stage,
        ConnectStage::Methods { .. }
    ));
}

#[test]
fn connected_event_switches_to_model_picker_and_toasts() {
    let (mut state, pane) = connect_state();
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
    select_provider(&mut state, pane, "anthropic");
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
    let panel = connect_panel(&state, pane);
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
fn oauth_prompt_fills_url_and_shortcuts_open_browser() {
    let (mut state, pane) = connect_state();
    open(&mut state, pane);
    select_provider(&mut state, pane, "github-copilot");
    apply_connect_enter(&mut state, pane);
    // github.com 是默认选项：选企业的路径已在另一个测试覆盖；这里直接选默认项启动。
    assert!(apply_connect_enter(&mut state, pane));
    assert!(matches!(
        connect_panel(&state, pane).stage,
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
    let panel = connect_panel(&state, pane);
    let ConnectStage::OAuthWaiting { url, user_code, .. } = &panel.stage else {
        panic!("oauth waiting");
    };
    assert_eq!(url, "https://github.com/login/device");
    assert_eq!(user_code.as_deref(), Some("ABCD-1234"));

    assert!(crate::app::update::handle_connect_input(
        &mut state,
        pane,
        crate::app::actions::LxInput::Char('c')
    ));
    assert!(state.effects.iter().any(|effect| matches!(
        effect,
        crate::app::effects::Effect::CopyToClipboard { text, .. } if text == "ABCD-1234"
    )));
    assert!(crate::app::update::handle_connect_input(
        &mut state,
        pane,
        crate::app::actions::LxInput::Char('o')
    ));
    assert!(state.effects.iter().any(|effect| matches!(
        effect,
        crate::app::effects::Effect::OpenBrowser { url } if url == "https://github.com/login/device"
    )));
}

#[test]
fn oauth_code_mode_submits_typed_code() {
    let (mut state, pane) = connect_state();
    open(&mut state, pane);
    select_provider(&mut state, pane, "github-copilot");
    apply_connect_enter(&mut state, pane);
    apply_connect_enter(&mut state, pane);
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
    assert!(apply_connect_enter(&mut state, pane));
    assert!(input(&state, pane).is_empty());
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
    select_provider(&mut state, pane, "github-copilot");
    apply_connect_enter(&mut state, pane);
    apply_connect_enter(&mut state, pane);
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
    assert!(apply_connect_esc(&mut state, pane));
    assert!(state.effects.iter().any(|effect| matches!(
        effect,
        crate::app::effects::Effect::CancelPresetOAuth { attempt_id } if attempt_id == "attempt-cancel"
    )));
    assert_eq!(connect_panel(&state, pane).stage, ConnectStage::Providers);
}

#[test]
fn esc_in_providers_closes_panel() {
    let (mut state, pane) = connect_state();
    open(&mut state, pane);
    assert!(apply_connect_esc(&mut state, pane));
    assert!(state.pane_anywhere(pane).expect("pane").lx.slash.is_none());
}

#[test]
fn preset_lookup_used_by_panel_is_available() {
    let spec = find_preset("anthropic").expect("anthropic preset");
    assert_eq!(spec.name, "Anthropic");
}
