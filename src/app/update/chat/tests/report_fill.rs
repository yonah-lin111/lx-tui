//! 报告回执 → 主输入框折叠芯片：来源头、同源替换、失败语义与会话隔离。

use super::*;
use crate::app::paste::FoldedKind;
use crate::app::update::GeneratedReportReady;

fn report_state() -> (AppState, PaneId) {
    let mut state = ready_state("");
    let pane = focus_pane(&state);
    state.pane_mut_anywhere(pane).unwrap().lx.chat.session_id = Some("sess-1".into());
    (state, pane)
}

fn chips(state: &AppState, pane: PaneId) -> Vec<crate::app::paste::FoldedToken> {
    crate::app::paste::folded_tokens(&state.pane_anywhere(pane).unwrap().lx.input)
}

fn subagent(body: &str) -> GeneratedReportReady {
    GeneratedReportReady {
        pane: PaneId::alloc(),
        parent_session: "sess-1".into(),
        kind: FoldedKind::SubagentReport,
        source: "agent-1".into(),
        body: crate::app::update::format_report_body(
            "[Subagent report] Curie · executor · Fix login",
            body,
            &["src/a.rs".into(), "src/b.rs".into()],
            2,
        ),
    }
}

#[test]
fn generated_report_fills_source_header_and_changed_files() {
    let (mut state, pane) = report_state();
    let mut ready = subagent("All done.");
    ready.pane = pane;
    assert!(crate::app::update::on_generated_report_ready(
        &mut state, ready
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
    let mut old = subagent("old");
    old.pane = pane;
    crate::app::update::on_generated_report_ready(&mut state, old);
    crate::app::update::on_generated_report_ready(
        &mut state,
        GeneratedReportReady {
            pane,
            parent_session: "sess-1".into(),
            kind: FoldedKind::BtwReport,
            source: "thread-1".into(),
            body: "[BTW report] Side\nbtw answer".into(),
        },
    );
    assert_eq!(chips(&state, pane).len(), 2, "不同来源共存");

    let mut new = subagent("new");
    new.pane = pane;
    crate::app::update::on_generated_report_ready(&mut state, new);
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
fn btw_report_uses_thread_header() {
    let (mut state, pane) = report_state();
    assert!(crate::app::update::on_generated_report_ready(
        &mut state,
        GeneratedReportReady {
            pane,
            parent_session: "sess-1".into(),
            kind: FoldedKind::BtwReport,
            source: "thread-1".into(),
            body: "[BTW report] Side thread\ncurie body".into(),
        }
    ));
    let chip = &chips(&state, pane)[0];
    assert_eq!(chip.kind, FoldedKind::BtwReport);
    assert!(chip.body.starts_with("[BTW report] Side thread"));
    assert!(chip.body.contains("curie body"));
}

#[test]
fn empty_report_only_toasts() {
    let (mut state, pane) = report_state();
    let mut ready = subagent("");
    ready.pane = pane;
    ready.body = String::new();
    assert!(!crate::app::update::on_generated_report_ready(
        &mut state, ready
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
        crate::ui::text::detail::LX_REPORT_UNAVAILABLE
    );
}

#[test]
fn report_for_another_session_is_dropped() {
    let (mut state, pane) = report_state();
    let mut ready = subagent("body");
    ready.pane = pane;
    ready.parent_session = "other-session".into();
    assert!(!crate::app::update::on_generated_report_ready(
        &mut state, ready
    ));
    assert!(chips(&state, pane).is_empty());
}
