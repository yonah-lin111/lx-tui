use super::*;

fn parse(json: &str) -> Option<AiWebStreamEvent> {
    parse_script_line(&format!("{PROTOCOL_PREFIX}{json}"))
}

#[test]
fn parses_thinking_and_text_deltas() {
    assert_eq!(
        parse(r#"{"event":"thinking","delta":"hmm"}"#),
        Some(AiWebStreamEvent::Thinking("hmm".into()))
    );
    assert_eq!(
        parse(r#"{"event":"text","delta":"hello"}"#),
        Some(AiWebStreamEvent::Text("hello".into()))
    );
}

#[test]
fn parses_empty_delta() {
    assert_eq!(
        parse(r#"{"event":"text","delta":""}"#),
        Some(AiWebStreamEvent::Text(String::new()))
    );
    assert_eq!(
        parse(r#"{"event":"thinking","delta":""}"#),
        Some(AiWebStreamEvent::Thinking(String::new()))
    );
}

#[test]
fn parses_login_required_with_url() {
    assert_eq!(
        parse(r#"{"event":"login_required","url":"https://chat.deepseek.com/sign_in"}"#),
        Some(AiWebStreamEvent::LoginRequired {
            url: "https://chat.deepseek.com/sign_in".into()
        })
    );
}

#[test]
fn parses_done_and_error() {
    assert_eq!(parse(r#"{"event":"done"}"#), Some(AiWebStreamEvent::Done));
    assert_eq!(
        parse(r#"{"event":"error","message":"user is controlling"}"#),
        Some(AiWebStreamEvent::Error("user is controlling".into()))
    );
}

#[test]
fn ignores_invalid_json_and_unknown_events() {
    assert_eq!(parse("not json"), None);
    assert_eq!(parse(r#"{"event":"unknown"}"#), None);
    assert_eq!(parse(r#"{"delta":"missing event"}"#), None);
}

#[test]
fn ignores_non_protocol_noise() {
    assert_eq!(
        parse_script_line("[ego-browser:notice] Ego Lite update is available"),
        None
    );
    assert_eq!(parse_script_line(""), None);
    assert_eq!(parse_script_line("console output"), None);
}

#[test]
fn parses_session_events() {
    assert_eq!(
        parse(r#"{"event":"session","id":"s-1","title":"Hello"}"#),
        Some(AiWebStreamEvent::Session {
            id: "s-1".into(),
            title: Some("Hello".into()),
        })
    );
    assert_eq!(
        parse(
            r#"{"event":"sessions","items":[{"id":"s-1","title":"Chat 1","updated_at_ms":1000}]}"#
        ),
        Some(AiWebStreamEvent::Sessions(vec![AiWebSessionSummary {
            id: "s-1".into(),
            title: "Chat 1".into(),
            updated_at_ms: 1000,
        }]))
    );
    assert_eq!(
        parse(
            r#"{"event":"session_loaded","id":"s-1","title":"Chat 1","messages":[{"role":"user","thinking":"","thinking_ms":null,"text":"hi","created_at_ms":1000}]}"#
        ),
        Some(AiWebStreamEvent::SessionLoaded {
            id: "s-1".into(),
            title: Some("Chat 1".into()),
            messages: vec![AiWebHistoryMessage {
                role: "user".into(),
                thinking: String::new(),
                thinking_ms: None,
                text: "hi".into(),
                files: Vec::new(),
                created_at_ms: 1000,
            }],
        })
    );
}

#[test]
fn scripts_for_site_returns_deepseek_and_gemini_driver_bundles() {
    let (ds_helpers, ds_main) = scripts_for_site(crate::app::state::AiWebSite::Deepseek);
    assert!(ds_helpers.contains("lx-tui-ai-web-deepseek"));
    assert!(ds_main.contains("ds-toggle-button"));

    let (gm_helpers, gm_main) = scripts_for_site(crate::app::state::AiWebSite::Gemini);
    assert!(gm_helpers.contains("lx-tui-ai-web-gemini"));
    assert!(gm_helpers.contains("bard-mode-menu-button"));
    assert!(gm_main.contains("__lxGeminiStream"));
}
