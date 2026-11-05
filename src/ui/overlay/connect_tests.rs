//! `/connect` 居中模态渲染与几何测试。

use super::*;
use crate::app::overlay::{Overlay, TextInput};
use crate::app::state::{
    AppState, ConnectManageAction, ConnectPanel, ConnectPromptStep, ConnectStage, PanelWindow,
};
use crate::config::ai::connect::{ConnectSettings, ConnectionSource};
use crate::ui::text;

use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::layout::Rect;

const SCREEN: Rect = Rect {
    x: 0,
    y: 0,
    width: 80,
    height: 24,
};

fn panel(stage: ConnectStage) -> ConnectPanel {
    let pane = crate::layout::PaneId::alloc();
    let mut panel = ConnectPanel::new_with_env(pane, &ConnectSettings::default(), |_| None);
    panel.stage = stage;
    panel
}

fn draw(panel: ConnectPanel) -> (Vec<String>, Option<(u16, u16)>) {
    let mut state = AppState::demo();
    state.overlay = Some(Overlay::Connect(Box::new(panel)));
    let mut terminal = Terminal::new(TestBackend::new(80, 24)).expect("test backend");
    let mut cursor = None;
    terminal
        .draw(|frame| {
            cursor = render(frame, frame.area(), &state);
        })
        .expect("draw");
    let buffer = terminal.backend().buffer().clone();
    let lines = (0..buffer.area.height)
        .map(|y| {
            (0..buffer.area.width)
                .map(|x| buffer[(x, y)].symbol())
                .collect()
        })
        .collect();
    (lines, cursor)
}

#[test]
fn providers_stage_renders_title_rows_and_footer() {
    let panel = panel(ConnectStage::Providers);
    let (lines, _) = draw(panel);
    let body = lines.join("\n");
    assert!(body.contains(text::LX_CONNECT_TITLE_PROVIDERS), "{body}");
    assert!(body.contains("302.AI"), "{body}");
    assert!(body.contains(text::LX_CONNECT_PROVIDERS_FOOTER), "{body}");
    assert!(body.contains("0 connected"), "{body}");
}

#[test]
fn layout_is_centered_and_exposes_input_status_and_list() {
    let panel = panel(ConnectStage::Providers);
    let layout = connect_layout(SCREEN, &panel).expect("layout");
    assert_eq!(layout.area.width, CONNECT_WIDTH);
    assert_eq!(layout.area.height, CONNECT_HEIGHT);
    assert_eq!(layout.area.x, 4);
    assert_eq!(layout.area.y, 2);
    assert_eq!(layout.input_rect.y + 1, layout.status_rect.y);
    assert_eq!(layout.status_rect.y + 1, layout.list_rect.y);
    let first = layout.rows[0];
    assert_eq!(
        connect_row_at(SCREEN, &panel, first.x, first.y),
        Some(layout.start)
    );
}

#[test]
fn manage_stage_renders_refresh_models_action() {
    let panel = panel(ConnectStage::Manage {
        provider_id: "anthropic".into(),
        provider_name: "Anthropic".into(),
        source: ConnectionSource::ApiKey,
        actions: vec![
            ConnectManageAction::SelectModel,
            ConnectManageAction::Reconnect,
            ConnectManageAction::RefreshModels,
            ConnectManageAction::Disconnect,
        ],
        window: PanelWindow::default(),
    });
    let body = draw(panel).0.join("\n");
    assert!(body.contains("Manage · Anthropic"), "{body}");
    assert!(
        body.contains(text::LX_CONNECT_ACTION_SELECT_MODEL),
        "{body}"
    );
    assert!(
        body.contains(text::LX_CONNECT_ACTION_REFRESH_MODELS),
        "{body}"
    );
}

#[test]
fn api_key_prompt_renders_status_and_error_in_red() {
    let mut panel = panel(ConnectStage::InputPrompt {
        provider_id: "anthropic".into(),
        provider_name: "Anthropic".into(),
        method_index: 0,
        step: ConnectPromptStep::ApiKey,
        answers: std::collections::BTreeMap::new(),
        error: Some("Invalid API key".into()),
    });
    panel.input = TextInput::new("sk-test");
    let layout = connect_layout(SCREEN, &panel).expect("layout");
    let (lines, cursor) = draw(panel);
    let body = lines.join("\n");
    assert!(body.contains("Verify API key · Anthropic"), "{body}");
    assert!(body.contains("sk-test"), "{body}");
    assert!(body.contains("Invalid API key"), "{body}");
    assert!(cursor.is_some(), "非校验态返回硬件光标");
    assert_eq!(layout.input_rect.y, layout.area.y + 3);
}

#[test]
fn validating_hides_cursor_and_shows_status_message() {
    let mut panel = panel(ConnectStage::InputPrompt {
        provider_id: "anthropic".into(),
        provider_name: "Anthropic".into(),
        method_index: 0,
        step: ConnectPromptStep::ApiKey,
        answers: std::collections::BTreeMap::new(),
        error: None,
    });
    panel.validating = true;
    panel.status_message = Some(text::LX_CONNECT_VERIFYING.into());
    let (lines, cursor) = draw(panel);
    assert!(lines.join("\n").contains(text::LX_CONNECT_VERIFYING));
    assert!(cursor.is_none(), "校验中不占用硬件光标");
}

#[test]
fn oauth_waiting_renders_open_copy_and_waiting_rows() {
    let panel = panel(ConnectStage::OAuthWaiting {
        attempt_id: "a".into(),
        provider_id: "github-copilot".into(),
        provider_name: "GitHub Copilot".into(),
        method_id: "device".into(),
        form_answers: std::collections::BTreeMap::new(),
        url: "https://github.com/login/device".into(),
        user_code: Some("ABCD-1234".into()),
        instructions: "Enter code: ABCD-1234".into(),
        mode: crate::config::ai::oauth::OAuthPromptMode::Auto,
        error: None,
        window: PanelWindow::default(),
    });
    let body = draw(panel).0.join("\n");
    assert!(body.contains("Authorize · GitHub Copilot"), "{body}");
    assert!(body.contains(text::LX_CONNECT_OPEN_BROWSER), "{body}");
    assert!(body.contains(text::LX_CONNECT_COPY_CODE), "{body}");
    assert!(body.contains("ABCD-1234"), "{body}");
    assert!(body.contains("Enter code: ABCD-1234"), "{body}");
}

#[test]
fn long_provider_list_exposes_scrollbar() {
    let panel = panel(ConnectStage::Providers);
    let layout = connect_layout(SCREEN, &panel).expect("layout");
    assert!(layout.scrollbar.is_some(), "预设供应商超过列表高度");
    assert!(layout.rows.len() <= layout.visible);
}
