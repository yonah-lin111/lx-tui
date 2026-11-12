use super::*;
use crate::app::actions::LxInput;
use crate::app::state::lx::btw::{BtwFocus, BtwQaSummary};
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
fn btw_composer_slash_panel_lists_btwlist_and_opens_the_picker() {
    // `/btwList` 不属于主会话命令表。
    assert!(
        !crate::app::state::lx::slash::SlashCommand::ALL
            .contains(&crate::app::state::lx::slash::SlashCommand::BtwList)
    );
    let (mut state, pane) = btw_pane();
    // 在 BTW 输入框输入 `/`：打开 BTW 专属命令面板，而不是主会话的命令面板。
    assert!(apply_input(&mut state, pane, LxInput::Char('/')));
    let composer = state
        .pane_anywhere(pane)
        .expect("pane")
        .lx
        .btw
        .active_composer()
        .expect("composer");
    let Some(crate::app::state::SlashPanel::Commands { items, .. }) = &composer.slash else {
        panic!("/ 面板应打开");
    };
    assert_eq!(items.len(), 1, "BTW 面板只列 BTW 自己的命令");
    assert!(matches!(
        items[0],
        crate::app::state::SlashEntry::Command(crate::app::state::lx::slash::SlashCommand::BtwList)
    ));
    assert!(
        state.pane_anywhere(pane).expect("pane").lx.slash.is_none(),
        "主输入框的 / 面板不能被 BTW 命令污染"
    );

    // 继续输入过滤：完整命令仍保留唯一候选。
    for ch in "btwlist".chars() {
        assert!(apply_input(&mut state, pane, LxInput::Char(ch)));
    }
    assert!(matches!(
        state
            .pane_anywhere(pane)
            .expect("pane")
            .lx
            .btw
            .active_composer()
            .expect("composer")
            .slash,
        Some(crate::app::state::SlashPanel::Commands { .. })
    ));

    // Enter 采纳：草稿被消费，QA 列表未到达时先挂起选择器。
    assert!(apply_input(&mut state, pane, LxInput::Enter));
    let btw = &state.pane_anywhere(pane).expect("pane").lx.btw;
    assert!(btw.pending_pick);
    let composer = btw.active_composer().expect("composer");
    assert_eq!(composer.input, "");
    assert!(composer.slash.is_none());

    // QA 列表回投：默认停当前 QA，并保留 DB 的 side-thread 标记。
    assert!(on_qa_list_loaded(
        &mut state,
        pane,
        "sess-test",
        vec![qa("qa-1", true), qa("qa-2", false)],
    ));
    let btw = &state.pane_anywhere(pane).expect("pane").lx.btw;
    assert_eq!(btw.picking, Some(0));
    assert!(btw.has_side_thread(&btw.qa[0]));
    assert!(!btw.has_side_thread(&btw.qa[1]));

    // ↓ 选到另一条 QA，Enter 切换并触发历史加载。
    assert!(apply_input(&mut state, pane, LxInput::Down));
    assert!(apply_input(&mut state, pane, LxInput::Enter));
    let btw = &state.pane_anywhere(pane).expect("pane").lx.btw;
    assert_eq!(btw.active.as_deref(), Some("qa-2"));
    assert!(btw.picking.is_none());
    assert!(state.effects.iter().any(|effect| matches!(
        effect,
        Effect::Btw(BtwEffect::LoadThread { qa_id, .. }) if qa_id == "qa-2"
    )));
}

#[test]
fn empty_btw_composer_arrows_switch_tabs() {
    let (mut state, pane) = btw_pane();
    assert!(apply_input(&mut state, pane, LxInput::Left));
    assert_eq!(
        state
            .pane_anywhere(pane)
            .expect("pane")
            .lx
            .task
            .as_ref()
            .expect("task")
            .tab,
        TaskTab::Agents
    );
}

#[test]
fn non_empty_btw_composer_arrows_move_the_cursor() {
    let (mut state, pane) = btw_pane();
    state
        .pane_mut_anywhere(pane)
        .expect("pane")
        .lx
        .btw
        .active_composer_mut()
        .expect("composer")
        .insert_char('x');
    let _ = apply_input(&mut state, pane, LxInput::Left);
    let target = state.pane_anywhere(pane).expect("pane");
    assert_eq!(target.lx.task.as_ref().expect("task").tab, TaskTab::Btw);
    assert_eq!(target.lx.btw.active_composer().expect("composer").cursor, 0);
}

#[test]
fn empty_btw_composer_up_returns_to_main_session() {
    let (mut state, pane) = btw_pane();
    assert!(state.pane_anywhere(pane).expect("pane").lx.task.is_some());
    assert!(apply_input(&mut state, pane, LxInput::Up));
    assert!(
        state.pane_anywhere(pane).expect("pane").lx.task.is_none(),
        "空输入框 ↑ 应关闭后台面板，回到主会话"
    );
}

#[test]
fn non_empty_btw_composer_up_keeps_the_panel() {
    let (mut state, pane) = btw_pane();
    state
        .pane_mut_anywhere(pane)
        .expect("pane")
        .lx
        .btw
        .active_composer_mut()
        .expect("composer")
        .insert_char('x');
    let _ = apply_input(&mut state, pane, LxInput::Up);
    assert!(
        state.pane_anywhere(pane).expect("pane").lx.task.is_some(),
        "有草稿时 ↑ 仍属于编辑器，不应关闭面板"
    );
}

#[test]
fn btw_list_panel_stays_open_while_loading_and_for_an_empty_list() {
    let (mut state, pane) = btw_pane();
    {
        let btw = &mut state.pane_mut_anywhere(pane).expect("pane").lx.btw;
        btw.qa.clear();
        btw.picking = None;
        btw.pending_pick = false;
    }

    // 列表未到达：面板先以 loading 占位打开，并独占键盘。
    assert!(open_btw_list(&mut state, pane));
    let btw = &state.pane_anywhere(pane).expect("pane").lx.btw;
    assert!(btw.pending_pick);
    assert!(btw.picker_loading);
    assert!(btw.picker_open());
    assert!(btw.picking.is_none());

    assert!(apply_input(&mut state, pane, LxInput::Escape));
    assert!(
        !state
            .pane_anywhere(pane)
            .expect("pane")
            .lx
            .btw
            .picker_open()
    );

    // 空列表到达：保留空态面板，而不是关掉面板或退回草稿。
    assert!(open_btw_list(&mut state, pane));
    assert!(on_qa_list_loaded(&mut state, pane, "sess-test", Vec::new(),));
    let btw = &state.pane_anywhere(pane).expect("pane").lx.btw;
    assert!(btw.picker_open());
    assert!(btw.pending_pick);
    assert!(!btw.picker_loading, "空列表到达后应显示空态而不是 loading");
    assert!(btw.picking.is_none());
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

    assert!(set_task_btw_scroll(&mut state, pane, 7));
    let thread = state
        .pane_anywhere(pane)
        .expect("pane")
        .lx
        .btw
        .active_thread()
        .expect("thread");
    assert_eq!(thread.scroll, 7);
    assert!(!thread.follow);

    assert!(set_task_btw_scroll(&mut state, pane, usize::MAX));
    let thread = state
        .pane_anywhere(pane)
        .expect("pane")
        .lx
        .btw
        .active_thread()
        .expect("thread");
    assert_eq!(thread.scroll, 15, "滚动条拖到底应钳到最大偏移");
    assert!(thread.follow);
    assert!(!set_task_btw_scroll(&mut state, pane, usize::MAX));

    // 对话区聚焦时 PgUp/PgDn 与 Agents 详情同口径翻 10 行。
    {
        let target = state.pane_mut_anywhere(pane).expect("pane");
        target.lx.btw.focus = BtwFocus::Conversation;
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
            key: "sub-1".into(),
            label: "Turing [explorer]".into(),
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
            key: "missing-agent".into(),
            label: "Missing [explorer]".into(),
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
fn default_target_sentinel_renders_as_lx_not_an_agent_id() {
    let (mut state, pane) = btw_pane();
    let thread = state
        .pane_mut_anywhere(pane)
        .expect("pane")
        .lx
        .btw
        .active_thread_mut()
        .expect("thread");
    thread.targets = vec![crate::app::state::lx::BtwTargetView::default_assistant()];
    assert_eq!(thread.target_label(), "To: lx");
    assert!(thread.targets[0].agent_id.is_none());
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
