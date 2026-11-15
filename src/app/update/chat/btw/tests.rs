use super::*;
use crate::app::actions::LxInput;
use crate::app::state::lx::btw::BtwQaSummary;
use crate::app::state::{AppState, ModelsSnapshot, PaneView, TaskPage, TaskTab};
use crate::config::ai::{
    AiSettings, ModelConfig, ModelSelection, ProviderConfig, ProviderOptions, ProviderTransport,
};

fn btw_pane() -> (AppState, PaneId) {
    let mut state = AppState::demo();
    let pane = state.active_tab().layout.focus();
    let target = state.pane_mut_anywhere(pane).expect("pane");
    target.view = PaneView::Lx;
    target.lx.entry = crate::app::state::LxEntry::Native;
    target.lx.chat.session_id = Some("sess-test".into());
    target.lx.task = Some(TaskPage {
        tab: TaskTab::Btw,
        ..Default::default()
    });
    target.lx.btw.ensure_thread("qa-1");
    target.lx.btw.editing = true;
    target.lx.btw.qa_loaded = true;
    target
        .lx
        .sync_btw_composer_context(Some(std::env::temp_dir()), None);
    (state, pane)
}

/// 最小可用模型快照：提交路径需要 provider、model、api key 同时有效。
fn models_with_default() -> ModelsSnapshot {
    let mut settings = AiSettings::empty();
    settings.enabled_providers = vec!["p".into()];
    settings.providers.insert(
        "p".into(),
        ProviderConfig {
            id: "p".into(),
            kind: ProviderTransport::OpenaiCompatible,
            name: "P".into(),
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
                    limit: Some(crate::config::ai::ModelLimit {
                        context: 100_000,
                        output: 4_096,
                    }),
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
    settings.default_model = Some(ModelSelection {
        provider: "p".into(),
        model: "m".into(),
        variant: None,
    });
    ModelsSnapshot { settings }
}

fn qa(id: &str, has_thread: bool) -> BtwQaSummary {
    BtwQaSummary {
        qa_id: id.to_string(),
        title: format!("question {id}"),
        has_thread,
        status: "idle".to_string(),
    }
}

#[test]
fn btw_list_shift_enter_enters_edit_and_escape_returns() {
    let (mut state, pane) = btw_pane();
    {
        let target = state.pane_mut_anywhere(pane).expect("pane");
        target.lx.btw.editing = false;
        target.lx.btw.qa = vec![qa("qa-1", true), qa("qa-2", false)];
        target.lx.btw.active = Some("qa-2".into());
        target.lx.btw.sync_selection_after_filter();
    }
    assert!(apply_input(&mut state, pane, LxInput::EnterFolder));
    assert!(state.pane_anywhere(pane).expect("pane").lx.btw.editing);
    assert!(apply_input(&mut state, pane, LxInput::Escape));
    assert!(!state.pane_anywhere(pane).expect("pane").lx.btw.editing);
}

#[test]
fn btw_list_search_moves_highlight_and_shift_enter_enters() {
    let (mut state, pane) = btw_pane();
    {
        let target = state.pane_mut_anywhere(pane).expect("pane");
        target.lx.btw.editing = false;
        target.lx.btw.qa = vec![qa("qa-1", true), qa("qa-2", false)];
        target.lx.btw.active = Some("qa-1".into());
        target.lx.btw.sync_selection_after_filter();
    }
    for ch in "qa-2".chars() {
        assert!(apply_input(&mut state, pane, LxInput::Char(ch)));
    }
    let btw = &state.pane_anywhere(pane).expect("pane").lx.btw;
    assert_eq!(btw.filtered_qa_indices(), vec![1]);
    assert_eq!(btw.active.as_deref(), Some("qa-2"));
    assert_eq!(btw.selected_index(), Some(0), "搜索后高亮第一项");
    assert!(!btw.editing, "普通 Enter 不进入编辑态");
    assert!(apply_input(&mut state, pane, LxInput::Enter));
    assert!(!state.pane_anywhere(pane).expect("pane").lx.btw.editing);
    assert!(
        apply_input(&mut state, pane, LxInput::EnterFolder),
        "Shift+Enter 必须进入过滤后的第一项"
    );
    assert!(state.pane_anywhere(pane).expect("pane").lx.btw.editing);
    assert!(apply_input(&mut state, pane, LxInput::Escape));
    assert!(!state.pane_anywhere(pane).expect("pane").lx.btw.editing);
    assert_eq!(
        state
            .pane_anywhere(pane)
            .expect("pane")
            .lx
            .btw
            .search
            .text(),
        "qa-2",
        "Esc 只退回列表，搜索词保留"
    );
    assert!(apply_input(&mut state, pane, LxInput::Escape));
    assert!(
        state
            .pane_anywhere(pane)
            .expect("pane")
            .lx
            .btw
            .search
            .text()
            .is_empty()
    );
}

#[test]
fn btw_qa_list_sets_latest_and_marks_side_threads() {
    let (mut state, pane) = btw_pane();
    {
        let target = state.pane_mut_anywhere(pane).expect("pane");
        target.lx.btw.editing = false;
        target.lx.btw.active = None;
        target.lx.btw.qa.clear();
    }
    assert!(on_qa_list_loaded(
        &mut state,
        pane,
        "sess-test",
        vec![qa("qa-1", true), qa("qa-2", false)],
    ));
    let btw = &state.pane_anywhere(pane).expect("pane").lx.btw;
    assert_eq!(btw.active.as_deref(), Some("qa-2"));
    assert_eq!(btw.qa_window.active, 1);
    assert!(btw.has_side_thread(&btw.qa[0]));
    assert!(!btw.has_side_thread(&btw.qa[1]));
}

#[test]
fn btw_empty_list_stays_in_list_mode() {
    let (mut state, pane) = btw_pane();
    {
        let target = state.pane_mut_anywhere(pane).expect("pane");
        target.lx.btw.editing = false;
        target.lx.btw.qa.clear();
        target.lx.btw.active = None;
        target.lx.btw.qa_loaded = false;
    }
    assert!(on_qa_list_loaded(&mut state, pane, "sess-test", Vec::new()));
    let btw = &state.pane_anywhere(pane).expect("pane").lx.btw;
    assert!(btw.qa_loaded);
    assert!(btw.qa.is_empty());
    assert!(!btw.editing);
}

#[test]
fn btw_loading_frames_advance_while_running() {
    let (mut state, pane) = btw_pane();
    let before = {
        let thread = state
            .pane_mut_anywhere(pane)
            .expect("pane")
            .lx
            .btw
            .active_thread_mut()
            .expect("thread");
        thread.pending_submit = Some(("prompt".into(), 1));
        assert!(thread.is_loading());
        thread.chat.loading_frame
    };
    assert!(crate::app::update::chat::advance_lx_loading(&mut state));
    let frame = state
        .pane_anywhere(pane)
        .expect("pane")
        .lx
        .btw
        .active_thread()
        .expect("thread")
        .chat
        .loading_frame;
    assert_eq!(frame, before + 1);
}

#[test]
fn backspace_removes_the_at_trigger_while_the_mention_panel_is_open() {
    let (mut state, pane) = btw_pane();
    state
        .pane_mut_anywhere(pane)
        .expect("pane")
        .lx
        .btw
        .active_composer_mut()
        .expect("composer")
        .insert_char('@');
    assert!(
        state
            .pane_anywhere(pane)
            .expect("pane")
            .lx
            .btw
            .active_composer()
            .expect("composer")
            .mention_visible(),
        "输入 `@` 后面板应可见"
    );

    assert!(apply_input(&mut state, pane, LxInput::Backspace));
    let composer = state
        .pane_anywhere(pane)
        .expect("pane")
        .lx
        .btw
        .active_composer()
        .expect("composer");
    assert_eq!(composer.input, "", "退格必须能删掉触发用 `@`");
    assert!(!composer.mention_visible(), "删除后提及面板应关闭");
}

#[test]
fn btw_wheel_scroll_and_scrollbar_clamp_to_the_thread() {
    let (mut state, pane) = btw_pane();
    {
        let thread = state
            .pane_mut_anywhere(pane)
            .expect("pane")
            .lx
            .btw
            .active_thread_mut()
            .expect("thread");
        thread.visible = 5;
        thread.total_rows = 20;
        thread.follow = true;
        thread.scroll = 15;
    }

    assert!(scroll_btw_detail(&mut state, pane, -3));
    let thread = state
        .pane_anywhere(pane)
        .expect("pane")
        .lx
        .btw
        .active_thread()
        .expect("thread");
    assert_eq!(thread.scroll, 12);
    assert!(!thread.follow, "上翻后必须离开贴尾状态");

    assert!(scroll_btw_detail(&mut state, pane, 3));
    let thread = state
        .pane_anywhere(pane)
        .expect("pane")
        .lx
        .btw
        .active_thread()
        .expect("thread");
    assert_eq!(thread.scroll, 15);
    assert!(thread.follow);
    assert!(!scroll_btw_detail(&mut state, pane, 3), "已在底部无变化");

    assert!(set_task_btw_scroll(&mut state, pane, 7, 25, 10));
    let thread = state
        .pane_anywhere(pane)
        .expect("pane")
        .lx
        .btw
        .active_thread()
        .expect("thread");
    assert_eq!(thread.scroll, 7);
    assert!(!thread.follow);

    assert!(set_task_btw_scroll(&mut state, pane, usize::MAX, 25, 10));
    let thread = state
        .pane_anywhere(pane)
        .expect("pane")
        .lx
        .btw
        .active_thread()
        .expect("thread");
    assert_eq!(thread.scroll, 15, "滚动条拖到底应钳到最大偏移");
    assert!(thread.follow);
    assert!(!set_task_btw_scroll(&mut state, pane, usize::MAX, 25, 10));

    // 对话区聚焦时 PgUp/PgDn 与 Agents 详情同口径翻 10 行。
    {
        let target = state.pane_mut_anywhere(pane).expect("pane");
        let thread = target.lx.btw.active_thread_mut().expect("thread");
        thread.scroll = 10;
        thread.follow = false;
    }
    assert!(apply_input(&mut state, pane, LxInput::PageDown));
    assert_eq!(
        state
            .pane_anywhere(pane)
            .expect("pane")
            .lx
            .btw
            .active_thread()
            .expect("thread")
            .scroll,
        15
    );
    assert!(apply_input(&mut state, pane, LxInput::PageUp));
    assert_eq!(
        state
            .pane_anywhere(pane)
            .expect("pane")
            .lx
            .btw
            .active_thread()
            .expect("thread")
            .scroll,
        5
    );
}

#[test]
fn btw_command_stashes_prompt_until_the_first_thread_is_created() {
    let (mut state, pane) = btw_pane();
    state.models = models_with_default();
    {
        let target = state.pane_mut_anywhere(pane).expect("pane");
        target.lx.task = None;
        target.lx.btw = Default::default();
        target.lx.replace_input("/btw regression prompt");
    }
    assert!(try_submit_btw_command(&mut state, pane));
    assert_eq!(
        state
            .pane_anywhere(pane)
            .expect("pane")
            .lx
            .btw
            .pending_prompt
            .as_deref(),
        Some("regression prompt")
    );
    assert!(
        state
            .pane_anywhere(pane)
            .expect("pane")
            .lx
            .btw
            .active_thread()
            .is_none(),
        "首次打开时线程尚未建立，正文只能先暂存"
    );

    assert!(on_qa_list_loaded(
        &mut state,
        pane,
        "sess-test",
        vec![qa("qa-1", false)]
    ));
    let thread = state
        .pane_anywhere(pane)
        .expect("pane")
        .lx
        .btw
        .active_thread()
        .expect("thread");
    assert_eq!(thread.composer.text(), "regression prompt");
    assert!(
        thread.pending_submit.is_some(),
        "QA 列表到达、线程建立后必须立即提交 /btw prompt"
    );
}

#[test]
fn btw_follow_up_without_mention_keeps_previous_agent_target() {
    let (mut state, pane) = btw_pane();
    state.models = models_with_default();
    let run = crate::app::state::SubagentRun::from_record(&crate::db::chat::SubagentRecord {
        agent_id: "sub-1".into(),
        nickname: Some("Turing".into()),
        agent_path: "/root/turing".into(),
        child_session_id: "child-1".into(),
        parent_session_id: "sess-test".into(),
        spawn_message_id: None,
        role: crate::chat::agent::AgentRole::Explorer,
        model_provider: String::new(),
        model_id: String::new(),
        model_variant: None,
        status: "completed".into(),
        title: "read token".into(),
        created_at_ms: 0,
        completed_at_ms: Some(100),
        report_json: None,
        report_delivered: false,
    });
    {
        let target = state.pane_mut_anywhere(pane).expect("pane");
        target.lx.agents.runs.push(run);
        let thread = target.lx.btw.active_thread_mut().expect("thread");
        thread.targets = vec![crate::app::state::lx::BtwTargetView {
            agent_id: Some("sub-1".into()),
        }];
        thread.composer.editor_mut().replace_input("follow up");
    }

    assert!(submit_current(&mut state, pane));
    let submit = state
        .effects
        .iter()
        .find_map(|effect| match effect {
            crate::app::effects::Effect::Btw(crate::app::effects::BtwEffect::Submit(sub)) => {
                Some(sub.as_ref())
            }
            _ => None,
        })
        .expect("BTW submit effect");
    assert_eq!(submit.targets.len(), 1);
    assert_eq!(submit.targets[0].agent_id, "sub-1");
}

#[test]
fn btw_follow_up_missing_previous_target_falls_back_to_default() {
    let (mut state, pane) = btw_pane();
    state.models = models_with_default();
    {
        let target = state.pane_mut_anywhere(pane).expect("pane");
        target.lx.agents.runs.clear();
        let thread = target.lx.btw.active_thread_mut().expect("thread");
        thread.targets = vec![crate::app::state::lx::BtwTargetView {
            agent_id: Some("missing-agent".into()),
        }];
        thread.composer.editor_mut().replace_input("continue");
    }

    assert!(submit_current(&mut state, pane));
    let submit = state
        .effects
        .iter()
        .find_map(|effect| match effect {
            crate::app::effects::Effect::Btw(crate::app::effects::BtwEffect::Submit(sub)) => {
                Some(sub.as_ref())
            }
            _ => None,
        })
        .expect("BTW submit effect");
    assert!(
        submit.targets.is_empty(),
        "missing previous target must fall back to default lx"
    );
    assert!(
        state
            .pane_anywhere(pane)
            .expect("pane")
            .lx
            .btw
            .active_thread()
            .expect("thread")
            .targets
            .is_empty(),
        "stale target badge must be cleared"
    );
}

#[test]
fn btw_command_with_prompt_switches_to_the_latest_qa() {
    let (mut state, pane) = btw_pane();
    state.models = models_with_default();
    {
        let target = state.pane_mut_anywhere(pane).expect("pane");
        target.lx.btw.qa = vec![qa("qa-1", true), qa("qa-2", false)];
        target.lx.btw.active = Some("qa-1".into());
        target.lx.replace_input("/btw newest question");
    }

    assert!(try_submit_btw_command(&mut state, pane));
    assert!(
        state
            .pane_anywhere(pane)
            .expect("pane")
            .lx
            .btw
            .active
            .is_none(),
        "命令式 /btw prompt 必须先清掉旧 active，列表回投后选择最新 QA"
    );

    assert!(on_qa_list_loaded(
        &mut state,
        pane,
        "sess-test",
        vec![qa("qa-1", true), qa("qa-2", false)],
    ));
    let target = state.pane_anywhere(pane).expect("pane");
    assert_eq!(target.lx.btw.active.as_deref(), Some("qa-2"));
    let submit = state
        .effects
        .iter()
        .find_map(|effect| match effect {
            crate::app::effects::Effect::Btw(crate::app::effects::BtwEffect::Submit(sub)) => {
                Some(sub.as_ref())
            }
            _ => None,
        })
        .expect("BTW submit effect");
    assert_eq!(submit.qa_id, "qa-2");
    assert_eq!(submit.text, "newest question");
}

#[test]
fn stale_thread_loaded_does_not_overwrite_another_session_or_qa() {
    let (mut state, pane) = btw_pane();
    assert!(!on_thread_loaded(
        &mut state,
        pane,
        "other-session",
        "qa-1",
        None,
        None,
        String::new(),
        Vec::new(),
        Vec::new(),
        Vec::new(),
        true,
    ));
    assert!(
        !state
            .pane_anywhere(pane)
            .expect("pane")
            .lx
            .btw
            .active_thread()
            .expect("thread")
            .loaded
    );

    assert!(!on_thread_loaded(
        &mut state,
        pane,
        "sess-test",
        "qa-2",
        None,
        None,
        String::new(),
        Vec::new(),
        Vec::new(),
        Vec::new(),
        true,
    ));
    assert_eq!(
        state
            .pane_anywhere(pane)
            .expect("pane")
            .lx
            .btw
            .active
            .as_deref(),
        Some("qa-1")
    );
    assert!(
        !state
            .pane_anywhere(pane)
            .expect("pane")
            .lx
            .btw
            .active_thread()
            .expect("thread")
            .loaded
    );

    assert!(on_thread_loaded(
        &mut state,
        pane,
        "sess-test",
        "qa-1",
        None,
        None,
        String::new(),
        Vec::new(),
        Vec::new(),
        Vec::new(),
        true,
    ));
    assert!(
        state
            .pane_anywhere(pane)
            .expect("pane")
            .lx
            .btw
            .active_thread()
            .expect("thread")
            .loaded
    );
}

#[test]
fn empty_qa_restores_the_stashed_btw_command_draft() {
    let (mut state, pane) = btw_pane();
    state.models = models_with_default();
    {
        let target = state.pane_mut_anywhere(pane).expect("pane");
        target.lx.btw.qa.clear();
        target.lx.btw.pending_prompt = Some("keep me".into());
        target.lx.clear();
    }

    assert!(on_qa_list_loaded(&mut state, pane, "sess-test", Vec::new()));
    assert_eq!(
        state.pane_anywhere(pane).expect("pane").lx.input,
        "/btw keep me"
    );
}

#[test]
fn btw_loading_participates_in_the_global_spinner_gate() {
    let (mut state, pane) = btw_pane();
    state
        .pane_mut_anywhere(pane)
        .expect("pane")
        .lx
        .btw
        .active_thread_mut()
        .expect("thread")
        .pending_submit = Some(("submit".into(), 1));
    assert!(state.has_streaming_lx());
}

#[test]
fn btwlist_is_not_a_slash_command() {
    assert!(
        crate::app::state::lx::slash::SlashCommand::ALL
            .iter()
            .all(|command| !command.text().eq_ignore_ascii_case("/btwList")),
        "/btwList must be removed from the slash enum"
    );
    assert!(
        crate::app::state::lx::slash::parse_btw_command("/btwList").is_none(),
        "输入路由不得把 /btwList 识别成 /btw"
    );
    assert_eq!(
        crate::app::state::lx::slash::parse_btw_command("/btw List"),
        Some(("List", false)),
        "只有空白分隔才是 /btw prompt"
    );
}

#[test]
fn bare_btw_reopen_keeps_the_current_qa() {
    let (mut state, pane) = btw_pane();
    {
        let target = state.pane_mut_anywhere(pane).expect("pane");
        target.lx.btw.qa = vec![qa("qa-1", true), qa("qa-2", true)];
        target.lx.btw.active = Some("qa-2".into());
        target.lx.btw.sync_selection_after_filter();
        target.lx.replace_input("/btw");
    }
    assert!(try_submit_btw_command(&mut state, pane));
    assert_eq!(
        state
            .pane_anywhere(pane)
            .expect("pane")
            .lx
            .btw
            .active
            .as_deref(),
        Some("qa-2"),
        "裸 /btw 重开应保留当前 QA，而不是抢到最新"
    );
}

#[test]
fn accepted_ack_updates_original_qa_after_the_user_switches() {
    let (mut state, pane) = btw_pane();
    {
        let target = state.pane_mut_anywhere(pane).expect("pane");
        target.lx.btw.qa = vec![qa("qa-1", true), qa("qa-2", false)];
        target.lx.btw.active = Some("qa-1".into());
        target.lx.btw.sync_selection_after_filter();
        target
            .lx
            .btw
            .thread_mut("qa-1")
            .expect("thread")
            .pending_submit = Some(("submit-1".into(), 0));
    }
    assert!(super::switch_qa(&mut state, pane, "qa-2"));
    assert!(on_accepted(
        &mut state,
        pane,
        AcceptedBtw {
            qa_id: "qa-1".into(),
            submit_id: "submit-1".into(),
            thread_id: "thread-1".into(),
            child_session_id: "child-1".into(),
            user_message_id: "msg-1".into(),
            text: "side question".into(),
        }
    ));
    let btw = &state.pane_anywhere(pane).expect("pane").lx.btw;
    assert_eq!(btw.active.as_deref(), Some("qa-2"), "ACK 不得抢回 active");
    let thread = btw
        .threads()
        .find(|thread| thread.qa_id == "qa-1")
        .expect("thread");
    assert!(thread.pending_submit.is_none());
    assert_eq!(thread.chat.messages.last().expect("message").role, "user");
}

#[test]
fn open_btw_tab_clears_stale_history_and_input_selection() {
    use crate::app::selection::{Selection, SelectionUnit};

    let (mut state, pane) = btw_pane();
    {
        let target = state.pane_mut_anywhere(pane).expect("pane");
        target.lx.insert_str("main draft");
        target.lx.select_all();
    }
    state.lx_selection = Some(Selection::begin_range(
        pane,
        (0, 0),
        (0, 1),
        SelectionUnit::Character,
    ));
    assert!(open_btw_tab(&mut state, pane));
    assert!(state.lx_selection.is_none(), "历史区残留选区必须清掉");
    let lx = &state.pane_anywhere(pane).expect("pane").lx;
    assert!(
        lx.text_selection_range().is_none(),
        "主输入框残留选区也必须清掉"
    );
}

#[test]
fn thread_loaded_restores_compact_checkpoints() {
    let (mut state, pane) = btw_pane();
    let checkpoint = crate::db::chat::CompactCheckpoint {
        id: "compact-1".into(),
        sequence: 7,
        operation_id: "op-1".into(),
        base_checkpoint_id: None,
        start_sequence: 0,
        end_sequence: 4,
        summary_text: "summary".into(),
        model_label: "M".into(),
        state: "active".into(),
        created_at_ms: 1,
    };
    assert!(on_thread_loaded(
        &mut state,
        pane,
        "sess-test",
        "qa-1",
        Some("thread-1".into()),
        Some("child-1".into()),
        "title".into(),
        Vec::new(),
        Vec::new(),
        vec![checkpoint],
        true,
    ));
    let thread = state
        .pane_anywhere(pane)
        .expect("pane")
        .lx
        .btw
        .active_thread()
        .expect("thread");
    assert_eq!(thread.compact.checkpoints.len(), 1);
    assert_eq!(thread.compact.checkpoints[0].id, "compact-1");
}

#[test]
fn btw_new_emits_delete_thread_and_rejects_while_loading() {
    let (mut state, pane) = btw_pane();
    state
        .pane_mut_anywhere(pane)
        .expect("pane")
        .lx
        .btw
        .active_thread_mut()
        .expect("thread")
        .thread_id = Some("thread-1".into());
    state.effects.clear();
    assert!(request_reset_thread(&mut state, pane));
    assert!(
        state.effects.iter().any(|effect| matches!(
            effect,
            Effect::Btw(BtwEffect::DeleteThread { qa_id, thread_id, .. })
                if qa_id == "qa-1" && thread_id == "thread-1"
        )),
        "发出删除线程效应：{:?}",
        state.effects
    );

    // 回答仍在进行：拒绝且不清线程。
    {
        let thread = state
            .pane_mut_anywhere(pane)
            .expect("pane")
            .lx
            .btw
            .active_thread_mut()
            .expect("thread");
        thread.pending_recipients = 1;
    }
    state.effects.clear();
    assert!(request_reset_thread(&mut state, pane));
    assert!(
        !state
            .effects
            .iter()
            .any(|effect| matches!(effect, Effect::Btw(BtwEffect::DeleteThread { .. }))),
        "忙碌时不得删除：{:?}",
        state.effects
    );
}

#[test]
fn thread_deleted_resets_view_and_qalist_mark() {
    let (mut state, pane) = btw_pane();
    {
        let thread = state
            .pane_mut_anywhere(pane)
            .expect("pane")
            .lx
            .btw
            .active_thread_mut()
            .expect("thread");
        thread.thread_id = Some("thread-1".into());
        thread.child_session_id = Some("child-1".into());
        thread.title = "old".into();
    }
    state.effects.clear();
    assert!(on_thread_deleted(&mut state, pane, "sess-test", "qa-1"));
    let thread = state
        .pane_anywhere(pane)
        .expect("pane")
        .lx
        .btw
        .active_thread()
        .expect("thread");
    assert!(thread.thread_id.is_none());
    assert!(thread.child_session_id.is_none());
    assert!(thread.chat.messages.is_empty());
    assert!(
        state
            .effects
            .iter()
            .any(|effect| matches!(effect, Effect::Btw(BtwEffect::LoadQaList { .. }))),
        "删除后刷新 QA 列表：{:?}",
        state.effects
    );
    assert!(
        state.pane_anywhere(pane).expect("pane").lx.toast.is_some(),
        "lx 页内 toast"
    );
}

/// BTW 滚动条同样以几何为准，不受滞后一帧的线程状态影响。
#[test]
fn set_task_btw_scroll_uses_scrollbar_geometry() {
    let (mut state, pane) = btw_pane();
    {
        let thread = state
            .pane_mut_anywhere(pane)
            .expect("pane")
            .lx
            .btw
            .active_thread_mut()
            .expect("thread");
        thread.visible = 10;
        thread.total_rows = 100;
    }
    assert!(set_task_btw_scroll(&mut state, pane, 71, 80, 10));
    let thread = state
        .pane_anywhere(pane)
        .expect("pane")
        .lx
        .btw
        .active_thread()
        .expect("thread");
    assert_eq!(thread.scroll, 70);
    assert!(thread.follow);
}
