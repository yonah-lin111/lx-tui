//! 报告回执 → 主输入框折叠芯片：来源头、同源替换、失败语义与会话隔离。

use super::*;
use crate::app::paste::FoldedKind;
use crate::app::update::{BtwReportReady, SubagentReportReady};
use crate::chat::agent::AgentRole;

fn report_state() -> (AppState, PaneId) {
    let mut state = ready_state("");
    let pane = focus_pane(&state);
    state.pane_mut_anywhere(pane).unwrap().lx.chat.session_id = Some("sess-1".into());
    (state, pane)
}

fn chips(state: &AppState, pane: PaneId) -> Vec<crate::app::paste::FoldedToken> {
    crate::app::paste::folded_tokens(&state.pane_anywhere(pane).unwrap().lx.input)
}

#[test]
fn subagent_report_ready_fills_source_header_and_changed_files() {
    let (mut state, pane) = report_state();
    assert!(crate::app::update::on_subagent_report_ready(
        &mut state,
        SubagentReportReady {
            pane,
            parent_session: "sess-1".into(),
            agent_id: "agent-1".into(),
            nickname: Some("Curie".into()),
            title: "Fix login".into(),
            role: AgentRole::Executor,
            body: "All done.".into(),
            changed_paths: vec!["src/a.rs".into(), "src/b.rs".into()],
        }
    ));
    let chips = chips(&state, pane);
    assert_eq!(chips.len(), 1);
    assert_eq!(chips[0].kind, FoldedKind::SubagentReport);
    assert_eq!(chips[0].source.as_deref(), Some("agent-1"));
    assert!(
        chips[0]
            .body
            .starts_with("[Subagent report] Curie · executor · Fix login")
    );
    assert!(chips[0].body.contains("Changed files: src/a.rs, src/b.rs"));
}

#[test]
fn same_source_replaces_while_different_sources_coexist() {
    let (mut state, pane) = report_state();
    let subagent = |body: &str| SubagentReportReady {
        pane,
        parent_session: "sess-1".into(),
        agent_id: "agent-1".into(),
        nickname: Some("Curie".into()),
        title: "task".into(),
        role: AgentRole::Executor,
        body: body.into(),
        changed_paths: Vec::new(),
    };
    crate::app::update::on_subagent_report_ready(&mut state, subagent("old"));
    crate::app::update::on_btw_report_ready(
        &mut state,
        BtwReportReady {
            pane,
            parent_session: "sess-1".into(),
            thread_id: "thread-1".into(),
            title: "Side".into(),
            sections: vec![("BTW".into(), "btw answer".into())],
            failed: Vec::new(),
        },
    );
    assert_eq!(chips(&state, pane).len(), 2, "不同来源共存");

    crate::app::update::on_subagent_report_ready(&mut state, subagent("new"));
    let chips = chips(&state, pane);
    assert_eq!(chips.len(), 2, "同源原位替换");
    let report = chips
        .iter()
        .find(|chip| chip.kind == FoldedKind::SubagentReport)
        .expect("subagent chip");
    assert!(report.body.contains("new"));
    assert!(!report.body.contains("old"));
}

#[test]
fn btw_report_merges_sections_and_failed_placeholders() {
    let (mut state, pane) = report_state();
    assert!(crate::app::update::on_btw_report_ready(
        &mut state,
        BtwReportReady {
            pane,
            parent_session: "sess-1".into(),
            thread_id: "thread-1".into(),
            title: "Side thread".into(),
            sections: vec![("Curie".into(), "curie body".into())],
            failed: vec!["Turing".into()],
        }
    ));
    let chip = &chips(&state, pane)[0];
    assert_eq!(chip.kind, FoldedKind::BtwReport);
    assert!(chip.body.starts_with("[BTW report] Side thread"));
    assert!(chip.body.contains("[Curie] curie body"));
    assert!(chip.body.contains("[Turing] report unavailable (failed)"));
}

#[test]
fn all_failed_btw_report_only_toasts() {
    let (mut state, pane) = report_state();
    assert!(!crate::app::update::on_btw_report_ready(
        &mut state,
        BtwReportReady {
            pane,
            parent_session: "sess-1".into(),
            thread_id: "thread-1".into(),
            title: "Side thread".into(),
            sections: Vec::new(),
            failed: vec!["Curie".into()],
        }
    ));
    assert!(chips(&state, pane).is_empty());
    let toast = state
        .pane_anywhere(pane)
        .unwrap()
        .lx
        .toast
        .as_ref()
        .expect("toast");
    assert_eq!(
        toast.message,
        crate::ui::text::detail::LX_BTW_REPORT_UNAVAILABLE
    );
}

#[test]
fn report_for_another_session_is_dropped() {
    let (mut state, pane) = report_state();
    assert!(!crate::app::update::on_subagent_report_ready(
        &mut state,
        SubagentReportReady {
            pane,
            parent_session: "other-session".into(),
            agent_id: "agent-1".into(),
            nickname: None,
            title: String::new(),
            role: AgentRole::Executor,
            body: "body".into(),
            changed_paths: Vec::new(),
        }
    ));
    assert!(chips(&state, pane).is_empty());
}
