//! BTW 视图：惰性构造、按 QA 独立草稿与焦点轮转。

use super::*;

#[test]
fn default_state_does_not_construct_a_composer() {
    // `Lx::new()` 必须终止：`LxBtw::default` 不创建内层 `Lx`。
    let lx = Lx::new();
    assert!(lx.btw.is_empty());
    assert!(lx.btw.active_composer().is_none());
}

#[test]
fn each_qa_keeps_its_own_draft() {
    let mut btw = LxBtw::default();
    btw.ensure_thread("qa-1")
        .composer
        .editor_mut()
        .replace_input("first draft");
    btw.ensure_thread("qa-2")
        .composer
        .editor_mut()
        .replace_input("second draft");

    assert_eq!(btw.active.as_deref(), Some("qa-2"));
    assert_eq!(
        btw.active_composer().map(|editor| editor.input.as_str()),
        Some("second draft")
    );
    assert!(btw.select_qa("qa-1"));
    assert_eq!(
        btw.active_composer().map(|editor| editor.input.as_str()),
        Some("first draft")
    );
    assert_eq!(btw.len(), 2);
}

#[test]
fn clearing_removes_threads_and_pending_flags() {
    let mut btw = LxBtw::default();
    btw.ensure_thread("qa-1");
    btw.pending_prompt = true;
    btw.clear();
    assert!(btw.is_empty());
    assert!(btw.active.is_none());
    assert!(!btw.pending_prompt);
}

#[test]
fn focus_cycles_in_both_directions() {
    assert_eq!(BtwFocus::Composer.next(), BtwFocus::Conversation);
    assert_eq!(BtwFocus::Conversation.next(), BtwFocus::Composer);
    assert_eq!(BtwFocus::Composer.previous(), BtwFocus::Conversation);
}

#[test]
fn composer_gets_scan_root_and_finished_subagent_candidates() {
    use crate::app::state::SubagentRun;
    use crate::chat::agent::AgentRole;
    use crate::db::chat::SubagentRecord;

    let mut lx = Lx::new();
    lx.btw.ensure_thread("qa-1");
    lx.agents
        .runs
        .push(SubagentRun::from_record(&SubagentRecord {
            agent_id: "agent-1".into(),
            nickname: Some("Turing".into()),
            agent_path: "/root/turing".into(),
            child_session_id: "child-1".into(),
            parent_session_id: "parent-1".into(),
            spawn_message_id: None,
            role: AgentRole::Explorer,
            model_provider: "p".into(),
            model_id: "m".into(),
            model_variant: None,
            // 已结束实例也必须出现在 BTW 的 `@` 候选里。
            status: "completed".into(),
            title: "scan".into(),
            created_at_ms: 1,
            completed_at_ms: Some(2),
            report_json: None,
            report_delivered: false,
        }));

    assert!(lx.sync_btw_composer_context(Some("/ws".into()), None));
    let composer = lx.btw.active_composer().expect("composer");
    assert_eq!(composer.mention().root(), Some(std::path::Path::new("/ws")));
    assert!(
        composer
            .mention()
            .subagent_runs()
            .iter()
            .any(|run| run.agent_id == "agent-1"),
        "结束的 subagent 也应可作为 BTW 目标"
    );

    // 模拟一次编辑触发的面板重算：外部注入的实例不能被空 `agents` 清除。
    let root = composer.mention().root().map(|path| path.to_path_buf());
    assert_eq!(root, Some("/ws".into()));
}
