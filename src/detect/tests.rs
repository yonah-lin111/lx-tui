use super::*;

fn lines(rows: &[&str]) -> Vec<String> {
    rows.iter().map(|row| (*row).to_string()).collect()
}

#[test]
fn identify_known_agents_with_aliases() {
    assert_eq!(identify_agent("claude"), Some(AgentKind::Claude));
    assert_eq!(identify_agent("claude-code"), Some(AgentKind::Claude));
    assert_eq!(identify_agent("codex"), Some(AgentKind::Codex));
    assert_eq!(identify_agent("gemini"), Some(AgentKind::Gemini));
    assert_eq!(identify_agent("gemini-cli"), Some(AgentKind::Gemini));
    assert_eq!(identify_agent("agy"), Some(AgentKind::Antigravity));
    assert_eq!(
        identify_agent("antigravity-cli"),
        Some(AgentKind::Antigravity)
    );
    assert_eq!(identify_agent("opencode"), Some(AgentKind::OpenCode));
    assert_eq!(identify_agent("opencode2"), Some(AgentKind::OpenCode));
    assert_eq!(identify_agent("open-code"), Some(AgentKind::OpenCode));
    assert_eq!(identify_agent("cursor"), Some(AgentKind::Cursor));
    assert_eq!(identify_agent("cursor-agent"), Some(AgentKind::Cursor));
    assert_eq!(identify_agent("pi"), Some(AgentKind::Pi));
    assert_eq!(identify_agent("kimi"), Some(AgentKind::Kimi));
    assert_eq!(identify_agent("kimi-code"), Some(AgentKind::Kimi));
}

#[test]
fn identify_accepts_paths_extensions_and_case() {
    assert_eq!(
        identify_agent("/usr/local/bin/Claude"),
        Some(AgentKind::Claude)
    );
    assert_eq!(
        identify_agent("C:\\Users\\herdr\\codex.cmd"),
        Some(AgentKind::Codex)
    );
    assert_eq!(
        identify_agent("/home/user/bin/gemini.js"),
        Some(AgentKind::Gemini)
    );
    assert_eq!(identify_agent("cursor-agent.exe"), Some(AgentKind::Cursor));
    assert_eq!(identify_agent("/opt/bin/pi"), Some(AgentKind::Pi));
}

#[test]
fn identify_rejects_shells_and_lookalikes() {
    assert_eq!(identify_agent("zsh"), None);
    assert_eq!(identify_agent("bash"), None);
    assert_eq!(identify_agent("node"), None);
    assert_eq!(identify_agent("vim"), None);
    assert_eq!(identify_agent("my-codex-helper"), None);
    assert_eq!(identify_agent("claude-code-thinking"), None);
    assert_eq!(identify_agent(""), None);
}

#[test]
fn snapshot_requires_known_process() {
    assert_eq!(
        pane_agent_snapshot("zsh", &lines(&["$ "]), None),
        None,
        "plain shell must not produce a snapshot"
    );

    let snapshot = pane_agent_snapshot("claude", &lines(&["hello"]), Some("✳ task"))
        .expect("claude is a known agent");
    assert_eq!(snapshot.kind, AgentKind::Claude);
    assert_eq!(snapshot.state, AgentState::Idle);
    assert_eq!(snapshot.title.as_deref(), Some("✳ task"));
}

#[test]
fn claude_states() {
    assert_eq!(
        arbitrate_state(AgentKind::Claude, &lines(&["❯ ", "? for shortcuts"])),
        AgentState::Idle
    );
    assert_eq!(
        arbitrate_state(
            AgentKind::Claude,
            &lines(&["✳ Simmering… (1m · esc to interrupt)"])
        ),
        AgentState::Working
    );
    assert_eq!(
        arbitrate_state(AgentKind::Claude, &lines(&["⠋ Running"])),
        AgentState::Working
    );
    assert_eq!(
        arbitrate_state(
            AgentKind::Claude,
            &lines(&["Do you want to proceed?", "❯ 1. Yes", "esc to cancel"])
        ),
        AgentState::Blocked
    );
}

#[test]
fn codex_states() {
    assert_eq!(
        arbitrate_state(AgentKind::Codex, &lines(&["› "])),
        AgentState::Idle
    );
    assert_eq!(
        arbitrate_state(
            AgentKind::Codex,
            &lines(&["• Working (10s • esc to interrupt)"])
        ),
        AgentState::Working
    );
    assert_eq!(
        arbitrate_state(
            AgentKind::Codex,
            &lines(&["press enter to confirm or esc to cancel"])
        ),
        AgentState::Blocked
    );
    assert_eq!(
        arbitrate_state(AgentKind::Codex, &lines(&["Action Required"])),
        AgentState::Blocked
    );
}

#[test]
fn gemini_states() {
    assert_eq!(
        arbitrate_state(AgentKind::Gemini, &lines(&["Type your message"])),
        AgentState::Idle
    );
    assert_eq!(
        arbitrate_state(AgentKind::Gemini, &lines(&["esc to cancel"])),
        AgentState::Working
    );
    assert_eq!(
        arbitrate_state(AgentKind::Gemini, &lines(&["│ Apply this change"])),
        AgentState::Blocked
    );
    assert_eq!(
        arbitrate_state(AgentKind::Gemini, &lines(&["│ Allow execution"])),
        AgentState::Blocked
    );
}

#[test]
fn antigravity_states() {
    assert_eq!(
        arbitrate_state(AgentKind::Antigravity, &lines(&["agy ready"])),
        AgentState::Idle
    );
    assert_eq!(
        arbitrate_state(AgentKind::Antigravity, &lines(&["⠋ Generating code"])),
        AgentState::Working
    );
    assert_eq!(
        arbitrate_state(
            AgentKind::Antigravity,
            &lines(&["requesting permission for: run command"])
        ),
        AgentState::Blocked
    );
}

#[test]
fn opencode_states() {
    assert_eq!(
        arbitrate_state(AgentKind::OpenCode, &lines(&["Ask anything"])),
        AgentState::Idle
    );
    assert_eq!(
        arbitrate_state(AgentKind::OpenCode, &lines(&["esc to interrupt"])),
        AgentState::Working
    );
    assert_eq!(
        arbitrate_state(AgentKind::OpenCode, &lines(&["■■■■■■ 50%"])),
        AgentState::Working
    );
    assert_eq!(
        arbitrate_state(AgentKind::OpenCode, &lines(&["△ Permission required"])),
        AgentState::Blocked
    );
}

#[test]
fn cursor_states() {
    assert_eq!(
        arbitrate_state(AgentKind::Cursor, &lines(&["cursor ready"])),
        AgentState::Idle
    );
    assert_eq!(
        arbitrate_state(AgentKind::Cursor, &lines(&["ctrl+c to stop"])),
        AgentState::Working
    );
    assert_eq!(
        arbitrate_state(AgentKind::Cursor, &lines(&["⬢ Generating code"])),
        AgentState::Working
    );
    assert_eq!(
        arbitrate_state(
            AgentKind::Cursor,
            &lines(&["write to this file?", "proceed (y)"])
        ),
        AgentState::Blocked
    );
}

#[test]
fn pi_states() {
    assert_eq!(
        arbitrate_state(AgentKind::Pi, &lines(&["pi> "])),
        AgentState::Idle
    );
    assert_eq!(
        arbitrate_state(AgentKind::Pi, &lines(&["Working..."])),
        AgentState::Working
    );
}

#[test]
fn kimi_states() {
    assert_eq!(
        arbitrate_state(AgentKind::Kimi, &lines(&["Kimi ready"])),
        AgentState::Idle
    );
    assert_eq!(
        arbitrate_state(AgentKind::Kimi, &lines(&["⠋ thinking..."])),
        AgentState::Working
    );
    assert_eq!(
        arbitrate_state(AgentKind::Kimi, &lines(&["🌑"])),
        AgentState::Working
    );
    assert_eq!(
        arbitrate_state(AgentKind::Kimi, &lines(&["↑↓ select", "esc cancel"])),
        AgentState::Blocked
    );
    assert_eq!(
        arbitrate_state(AgentKind::Kimi, &lines(&["↵ confirm"])),
        AgentState::Blocked
    );
}

#[test]
fn blocked_wins_over_working_when_both_visible() {
    assert_eq!(
        arbitrate_state(
            AgentKind::Codex,
            &lines(&[
                "esc to interrupt",
                "press enter to confirm or esc to cancel"
            ])
        ),
        AgentState::Blocked
    );
}
