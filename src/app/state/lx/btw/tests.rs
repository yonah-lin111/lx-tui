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
    btw.pending_prompt = Some("stashed".into());
    btw.clear();
    assert!(btw.is_empty());
    assert!(btw.active.is_none());
    assert!(btw.pending_prompt.is_none());
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

#[test]
fn typing_at_in_the_composer_opens_the_mention_panel() {
    let mut lx = Lx::new();
    lx.btw.ensure_thread("qa-1");
    lx.task = Some(crate::app::state::TaskPage {
        tab: crate::app::state::TaskTab::Btw,
        ..Default::default()
    });
    lx.btw.editing = true;
    let root = std::env::temp_dir();
    lx.sync_btw_composer_context(Some(root.clone()), None);

    let opened = lx.with_active_editor(|composer| {
        composer.insert_char('@');
        composer.mention_open() || composer.mention_visible()
    });
    assert!(opened, "composer 输入 `@` 必须打开提及面板");
    let request = lx.with_active_editor(|composer| composer.take_mention_scan_request());
    assert_eq!(request.map(|(_, scan_root)| scan_root), Some(root));
}

#[test]
fn search_filters_titles_and_moves_selection_to_the_first_match() {
    let mut lx = Lx::new();
    lx.btw.qa = vec![
        BtwQaSummary {
            qa_id: "qa-1".into(),
            title: "first".into(),
            has_thread: true,
            status: "idle".into(),
        },
        BtwQaSummary {
            qa_id: "qa-2".into(),
            title: "second".into(),
            has_thread: false,
            status: "running".into(),
        },
    ];
    lx.btw.active = Some("qa-1".into());
    lx.btw.sync_selection_after_filter();
    assert_eq!(lx.btw.qa_window.active, 0);
    assert!(lx.btw.has_side_thread(&lx.btw.qa[0]));
    assert!(!lx.btw.has_side_thread(&lx.btw.qa[1]));

    lx.btw.search.insert_str("sec");
    lx.btw.sync_selection_after_filter();
    assert_eq!(lx.btw.filtered_qa_indices(), vec![1]);
    assert_eq!(
        lx.btw.selected_index(),
        Some(0),
        "过滤后高亮第一项，不能与 active 分叉"
    );
    assert_eq!(
        lx.btw.active.as_deref(),
        Some("qa-2"),
        "详情跟随过滤后的第一项"
    );

    lx.btw.search.clear();
    lx.btw.sync_selection_after_filter();
    assert_eq!(lx.btw.selected_index(), Some(1));
    assert!(!lx.btw.move_selection(1), "列表末尾不环绕");
    assert!(lx.btw.move_selection(-1));
    assert_eq!(lx.btw.active.as_deref(), Some("qa-1"));
}

#[test]
fn enter_and_leave_edit_preserve_the_draft() {
    let mut lx = Lx::new();
    lx.btw.qa = vec![
        BtwQaSummary {
            qa_id: "qa-1".into(),
            title: "first".into(),
            has_thread: true,
            status: "idle".into(),
        },
        BtwQaSummary {
            qa_id: "qa-2".into(),
            title: "second".into(),
            has_thread: true,
            status: "idle".into(),
        },
    ];
    lx.btw.ensure_thread("qa-1");
    lx.btw.ensure_thread("qa-2");
    lx.btw.active = Some("qa-1".into());
    lx.btw.sync_selection_after_filter();
    lx.btw
        .active_thread_mut()
        .expect("thread")
        .composer
        .editor_mut()
        .replace_input("draft");
    assert!(lx.btw.enter_edit());
    assert!(lx.btw.editing);
    assert!(lx.btw.leave_edit());
    assert!(!lx.btw.editing);
    assert_eq!(
        lx.btw.active_composer().map(|editor| editor.input.as_str()),
        Some("draft")
    );
}
