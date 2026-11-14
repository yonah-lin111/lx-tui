//! 侧栏 Agent 标题解析与内存覆盖生命周期测试。

use super::*;

use crate::app::prompt::PromptPageId;
use crate::app::state::{AiWebSite, AppState, LxEntry};
use crate::app::update::update_pane_agent;
use crate::detect::{AgentKind, AgentState, PaneAgentSnapshot};

fn lx_kind() -> SidebarAgentKind {
    SidebarAgentKind::Lx {
        entry: LxEntry::Native,
        ai_web_site: AiWebSite::Deepseek,
    }
}

fn terminal_kind(kind: AgentKind) -> SidebarAgentKind {
    SidebarAgentKind::Terminal(kind)
}

fn snapshot(kind: AgentKind, state: AgentState, title: Option<&str>) -> PaneAgentSnapshot {
    PaneAgentSnapshot {
        kind,
        state,
        title: title.map(str::to_string),
    }
}

fn custom(text: &str) -> AgentTitleOverride {
    AgentTitleOverride::custom(text)
}

fn bound(text: &str, item_id: i64, page_id: i64) -> AgentTitleOverride {
    AgentTitleOverride {
        text: text.to_string(),
        prompt: Some(AgentPromptRef {
            item_id,
            page_id: PromptPageId::Saved(page_id),
        }),
    }
}

fn active_lx_pane() -> Pane {
    let mut pane = Pane::new();
    pane.lx.chat.session_id = Some("session-1".to_string());
    pane
}

#[test]
fn pane_new_has_no_agent_title_overrides() {
    let pane = Pane::new();
    assert_eq!(pane.terminal_agent_title, None);
    assert_eq!(pane.lx.chat.agent_title_override, None);
}

#[test]
fn lx_session_title_is_not_used_without_override() {
    let mut pane = active_lx_pane();
    assert_eq!(pane.agent_display_title(lx_kind()), None);

    pane.lx.chat.session_title = Some("Fix login bug".to_string());
    assert_eq!(
        pane.agent_display_title(lx_kind()),
        None,
        "lx 会话标题不再作为侧栏标题来源"
    );
}

#[test]
fn lx_override_shows_custom_title_and_clear_removes_title() {
    let mut pane = active_lx_pane();
    pane.lx.chat.session_title = Some("Auto title".to_string());

    pane.set_agent_title_override(lx_kind(), Some(custom("  Manual title  ")));
    assert_eq!(
        pane.agent_display_title(lx_kind()),
        Some("Manual title"),
        "覆盖首尾空白被裁掉"
    );
    assert_eq!(
        pane.lx.chat.session_title.as_deref(),
        Some("Auto title"),
        "设置覆盖不得修改会话真实标题"
    );

    pane.set_agent_title_override(lx_kind(), None);
    assert_eq!(
        pane.agent_display_title(lx_kind()),
        None,
        "清除覆盖后不再回退会话标题"
    );

    pane.set_agent_title_override(lx_kind(), Some(custom("Manual title")));
    pane.set_agent_title_override(lx_kind(), Some(custom("   ")));
    assert_eq!(
        pane.agent_display_title(lx_kind()),
        None,
        "空白覆盖按清除处理"
    );
}

#[test]
fn lx_override_keeps_prompt_binding() {
    let mut pane = active_lx_pane();
    pane.set_agent_title_override(lx_kind(), Some(bound("Auth Flow", 7, 9)));

    let title = pane
        .agent_title_override(lx_kind())
        .expect("title override");
    assert_eq!(title.text, "Auth Flow");
    assert_eq!(
        title.prompt,
        Some(AgentPromptRef {
            item_id: 7,
            page_id: PromptPageId::Saved(9)
        })
    );
    assert_eq!(pane.agent_display_title(lx_kind()), Some("Auth Flow"));
}

#[test]
fn lx_override_requires_active_session() {
    let mut pane = Pane::new();
    pane.lx.chat.session_title = Some("Auto title".to_string());

    pane.set_agent_title_override(lx_kind(), Some(custom("Manual title")));

    assert_eq!(pane.lx.chat.agent_title_override, None);
    assert_eq!(pane.agent_display_title(lx_kind()), None);
}

#[test]
fn terminal_snapshot_title_is_not_used_without_override() {
    let mut pane = Pane::new();
    let kind = terminal_kind(AgentKind::Claude);

    assert_eq!(pane.agent_display_title(kind), None);
    pane.agent = Some(snapshot(
        AgentKind::Claude,
        AgentState::Idle,
        Some("Refactor parser"),
    ));
    assert_eq!(
        pane.agent_display_title(kind),
        None,
        "终端 OSC 标题不再作为侧栏标题来源"
    );
}

#[test]
fn terminal_override_only_applies_to_matching_kind() {
    let mut pane = Pane::new();
    pane.agent = Some(snapshot(
        AgentKind::Claude,
        AgentState::Idle,
        Some("Claude OSC"),
    ));

    pane.set_agent_title_override(terminal_kind(AgentKind::Claude), Some(custom("Manual")));
    assert_eq!(
        pane.agent_display_title(terminal_kind(AgentKind::Claude)),
        Some("Manual")
    );
    assert_eq!(
        pane.agent_display_title(terminal_kind(AgentKind::Codex)),
        None,
        "没有对应快照时不得借用别的 Agent 覆盖"
    );
    assert_eq!(
        pane.terminal_agent_title,
        Some((AgentKind::Claude, custom("Manual")))
    );

    // 当前窗格是 Claude 时，写入 Codex 的覆盖无效。
    pane.set_agent_title_override(terminal_kind(AgentKind::Codex), Some(custom("Wrong")));
    assert_eq!(
        pane.terminal_agent_title,
        Some((AgentKind::Claude, custom("Manual")))
    );

    // 覆盖按 AgentKind 存储：查询另一种 Agent 不借用 Claude 的覆盖。
    assert_eq!(
        pane.agent_display_title(terminal_kind(AgentKind::Codex)),
        None
    );
}

#[test]
fn update_pane_agent_clears_override_on_kind_change() {
    let mut state = AppState::demo();
    let pane_id = state.active_tab().layout.focus();
    update_pane_agent(
        &mut state,
        pane_id,
        Some(snapshot(
            AgentKind::Claude,
            AgentState::Idle,
            Some("Claude OSC"),
        )),
    );
    state
        .pane_mut_anywhere(pane_id)
        .expect("pane")
        .set_agent_title_override(terminal_kind(AgentKind::Claude), Some(custom("Manual")));

    assert!(update_pane_agent(
        &mut state,
        pane_id,
        Some(snapshot(
            AgentKind::Codex,
            AgentState::Idle,
            Some("Codex OSC"),
        )),
    ));

    assert_eq!(
        state
            .pane_anywhere(pane_id)
            .expect("pane")
            .terminal_agent_title,
        None
    );
}

#[test]
fn update_pane_agent_clears_override_on_agent_exit() {
    let mut state = AppState::demo();
    let pane_id = state.active_tab().layout.focus();
    update_pane_agent(
        &mut state,
        pane_id,
        Some(snapshot(
            AgentKind::Claude,
            AgentState::Idle,
            Some("Claude OSC"),
        )),
    );
    state
        .pane_mut_anywhere(pane_id)
        .expect("pane")
        .set_agent_title_override(terminal_kind(AgentKind::Claude), Some(custom("Manual")));

    assert!(update_pane_agent(&mut state, pane_id, None));

    assert_eq!(
        state
            .pane_anywhere(pane_id)
            .expect("pane")
            .terminal_agent_title,
        None
    );
}

#[test]
fn update_pane_agent_keeps_override_for_same_kind() {
    let mut state = AppState::demo();
    let pane_id = state.active_tab().layout.focus();
    update_pane_agent(
        &mut state,
        pane_id,
        Some(snapshot(
            AgentKind::Claude,
            AgentState::Idle,
            Some("First OSC"),
        )),
    );
    state
        .pane_mut_anywhere(pane_id)
        .expect("pane")
        .set_agent_title_override(terminal_kind(AgentKind::Claude), Some(custom("Manual")));

    assert!(update_pane_agent(
        &mut state,
        pane_id,
        Some(snapshot(
            AgentKind::Claude,
            AgentState::Working,
            Some("Second OSC"),
        )),
    ));

    let pane = state.pane_anywhere(pane_id).expect("pane");
    assert_eq!(
        pane.terminal_agent_title,
        Some((AgentKind::Claude, custom("Manual")))
    );
    assert_eq!(
        pane.agent_display_title(terminal_kind(AgentKind::Claude)),
        Some("Manual")
    );
}

#[test]
fn reset_session_view_clears_lx_override() {
    let mut pane = active_lx_pane();
    pane.lx.chat.session_title = Some("Auto title".to_string());
    pane.set_agent_title_override(lx_kind(), Some(custom("Manual title")));
    assert_eq!(
        pane.lx
            .chat
            .agent_title_override
            .as_ref()
            .map(|t| t.text.as_str()),
        Some("Manual title")
    );

    pane.lx.chat.reset_session_view();

    assert_eq!(pane.lx.chat.agent_title_override, None);
    assert_eq!(pane.agent_display_title(lx_kind()), None);
}
