//! 详情 `/rewind`：子会话删除 + 文件恢复回投、compact 检查点恢复、迟到回执过滤与 BTW 线程重置。

use super::subagent::{fixture_with_parent, spawn_explorer};
use super::*;
use crate::app::effects::RewindRequest;
use crate::app::state::{
    DetailRewindExecution, DetailTarget, RequestState, RewindAction, SlashPanel, TaskTab,
};
use crate::db::chat::ModelSnapshot;

fn model() -> ModelSnapshot {
    ModelSnapshot {
        provider: "p".into(),
        model: "m".into(),
        variant: None,
    }
}

/// 把子 agent 的最后一轮通过正常事件链跑完（enqueue → claim → delta → finish）。
fn run_child_turn(
    state: &mut AppState,
    streams: &mut ChatStreams,
    persistence: &Persistence,
    key: &RequestKey,
    child_session: &str,
    agent_id: &str,
    text: &str,
    notify_parent: bool,
) {
    state.effects.push(Effect::SendSubagentUserInput {
        pane: key.pane,
        session_id: key.session.clone(),
        agent_id: agent_id.to_string(),
        text: text.to_string(),
        notify_parent,
    });
    assert!(crate::event::chat::apply_subagent_effects(
        state,
        streams,
        persistence
    ));
    let child_key = RequestKey {
        pane: key.pane,
        session: child_session.to_string(),
        generation: key.generation,
    };
    {
        let pane = state.pane_mut_anywhere(key.pane).expect("pane");
        pane.lx.chat.request = RequestState::Idle;
        pane.lx.chat.status.streaming = false;
    }
    let _ = crate::event::chat::handle_subagent_net_event(
        state,
        streams,
        ChatNetEvent::Delta {
            key: child_key.clone(),
            deltas: vec![ChatDelta::Part(crate::chat::events::PartDelta {
                kind: PartKind::Text,
                text: format!("reply to {text}"),
                metadata: None,
            })],
        },
    );
    crate::event::chat::finish_subagent(
        state,
        streams,
        persistence,
        child_key,
        StopReason::Complete,
        Usage::default(),
    );
    let pane = state.pane_mut_anywhere(key.pane).expect("pane");
    pane.lx.chat.request = RequestState::Idle;
    pane.lx.chat.status.streaming = false;
}

/// 进入 Agents 编辑态并执行一次 `/rewind`（预检 → 执行 → 回投）。
fn rewind_agent(
    state: &mut AppState,
    persistence: &Persistence,
    pane: PaneId,
    workspace: &std::path::Path,
    agent_id: &str,
) -> bool {
    assert!(crate::event::chat::detail_rewind::prepare(
        state,
        persistence,
        pane,
        DetailTarget::Agent(agent_id.to_string()),
        String::new(),
    ));
    let plan = state
        .pane_anywhere(pane)
        .and_then(|target| target.lx.task.as_ref())
        .and_then(|page| page.detail_rewind_pending.clone())
        .expect("undo plan");
    let operation_id = "detail-rewind-test".to_string();
    if let Some(page) = state
        .pane_mut_anywhere(pane)
        .and_then(|target| target.lx.task.as_mut())
    {
        page.detail_rewind_executing = Some(DetailRewindExecution {
            operation_id: operation_id.clone(),
            target: plan.target.clone(),
        });
    }
    // 快照不可用时只删对话（与面板禁用含文件动作一致）。
    let action = if plan.snapshot.files_available() {
        plan.action
    } else {
        RewindAction::DeleteOnly
    };
    let request = RewindRequest {
        pane,
        session_id: plan.child_session.clone(),
        target_user_message_id: plan.user_message_id.clone(),
        expected_revision: plan.expected_revision,
        action,
        workspace: plan.workspace.clone(),
        input_at_open: plan.input_at_open.clone(),
        user_text: plan.user_text.clone(),
        operation_id: operation_id.clone(),
    };
    let database = persistence.database().expect("db");
    let outcome = crate::event::chat::rewind::execute_rewind(database, workspace, &request);
    crate::event::chat::detail_rewind::handle_finished(
        state,
        persistence,
        pane,
        workspace,
        &operation_id,
        outcome,
    )
}

#[tokio::test]
async fn detail_rewind_deletes_the_last_follow_up_and_reloads_the_run() {
    let (fixture, mut state, key, persistence, mut streams, _parent) = fixture_with_parent();
    let agent_id = spawn_explorer(&mut state, &mut streams, &persistence, &key, "first task").await;
    let child_session = streams
        .subagents
        .values()
        .find(|handle| handle.agent_id == agent_id)
        .expect("handle")
        .child_session_id
        .clone();
    crate::event::chat::finish_subagent(
        &mut state,
        &mut streams,
        &persistence,
        RequestKey {
            pane: key.pane,
            session: child_session.clone(),
            generation: key.generation,
        },
        StopReason::Complete,
        Usage::default(),
    );
    run_child_turn(
        &mut state,
        &mut streams,
        &persistence,
        &key,
        &child_session,
        &agent_id,
        "private follow-up",
        false,
    );
    {
        let target = state.pane_mut_anywhere(key.pane).expect("pane");
        target.lx.agents.ensure_composer(&agent_id);
        let page = target.lx.task.get_or_insert_with(Default::default);
        page.tab = TaskTab::Agents;
        page.agent_edit = Some(agent_id.clone());
    }
    assert!(rewind_agent(
        &mut state,
        &persistence,
        key.pane,
        fixture.workspace(),
        &agent_id
    ));
    let run = state
        .pane_anywhere(key.pane)
        .expect("pane")
        .lx
        .agents
        .by_agent(&agent_id)
        .expect("run");
    assert!(
        run.chat
            .messages
            .iter()
            .all(|message| !message.text().contains("private follow-up")),
        "最新私聊轮已撤回：{:?}",
        run.chat
            .messages
            .iter()
            .map(|message| message.text())
            .collect::<Vec<_>>()
    );
    assert!(
        run.chat
            .messages
            .iter()
            .any(|message| message.text().contains("first task")),
        "spawn 任务轮保留"
    );
}

#[tokio::test]
async fn detail_rewind_ignores_late_operation_id() {
    let (fixture, mut state, key, persistence, mut streams, _parent) = fixture_with_parent();
    let agent_id = spawn_explorer(&mut state, &mut streams, &persistence, &key, "first task").await;
    let child_session = streams
        .subagents
        .values()
        .find(|handle| handle.agent_id == agent_id)
        .expect("handle")
        .child_session_id
        .clone();
    crate::event::chat::finish_subagent(
        &mut state,
        &mut streams,
        &persistence,
        RequestKey {
            pane: key.pane,
            session: child_session.clone(),
            generation: key.generation,
        },
        StopReason::Complete,
        Usage::default(),
    );
    run_child_turn(
        &mut state,
        &mut streams,
        &persistence,
        &key,
        &child_session,
        &agent_id,
        "private follow-up",
        false,
    );
    {
        let target = state.pane_mut_anywhere(key.pane).expect("pane");
        target.lx.agents.ensure_composer(&agent_id);
        let page = target.lx.task.get_or_insert_with(Default::default);
        page.tab = TaskTab::Agents;
        page.agent_edit = Some(agent_id.clone());
    }
    assert!(crate::event::chat::detail_rewind::prepare(
        &mut state,
        &persistence,
        key.pane,
        DetailTarget::Agent(agent_id.clone()),
        String::new(),
    ));
    if let Some(page) = state
        .pane_mut_anywhere(key.pane)
        .and_then(|target| target.lx.task.as_mut())
    {
        page.detail_rewind_executing = Some(DetailRewindExecution {
            operation_id: "current-op".into(),
            target: DetailTarget::Agent(agent_id),
        });
    }
    let handled = crate::event::chat::detail_rewind::handle_finished(
        &mut state,
        &persistence,
        key.pane,
        fixture.workspace(),
        "stale-op",
        crate::event::chat::RewindOutcome::Failed {
            message: "late".into(),
        },
    );
    assert!(!handled, "迟到回执必须被忽略");
    assert!(
        state
            .pane_anywhere(key.pane)
            .and_then(|target| target.lx.task.as_ref())
            .and_then(|page| page.detail_rewind_executing.as_ref())
            .is_some_and(|exec| exec.operation_id == "current-op"),
        "当前执行不被旧回执清掉"
    );
}

/// 建一个 BTW 线程：父会话已有 QA 锚点，child session 里有两轮问答。
pub(super) fn btw_fixture(
    fixture: &Fixture,
    state: &mut AppState,
    key: &RequestKey,
) -> crate::db::chat::BtwThreadRecord {
    let database = fixture.database();
    let messages = database
        .load_chat_session(fixture.workspace(), &key.session)
        .expect("load parent")
        .expect("parent session");
    let qa_id = messages
        .iter()
        .find(|message| message.role == "user")
        .expect("parent user")
        .external_id
        .clone();
    let thread = database
        .ensure_btw_thread(fixture.workspace(), &key.session, &qa_id, "side thread")
        .expect("thread");
    for question in ["q1", "q2"] {
        let turn = database
            .begin_turn(
                fixture.workspace(),
                Some(&thread.child_session_id),
                question,
                &model(),
            )
            .expect("btw turn");
        database
            .finish(
                fixture.workspace(),
                &turn.assistant_message_id,
                &[],
                &crate::db::chat::TurnOutcome::Complete {
                    stop_reason: Some("complete".into()),
                },
                Usage::default(),
            )
            .expect("finish btw turn");
    }
    let target = state.pane_mut_anywhere(key.pane).expect("pane");
    target.lx.task = Some(crate::app::state::TaskPage {
        tab: TaskTab::Btw,
        ..Default::default()
    });
    let thread_state = target.lx.btw.ensure_thread("qa-1");
    thread_state.thread_id = Some(thread.id.clone());
    thread_state.child_session_id = Some(thread.child_session_id.clone());
    thread_state.loaded = true;
    target.lx.btw.editing = true;
    thread
}

fn rewind_btw(
    state: &mut AppState,
    persistence: &Persistence,
    pane: PaneId,
    workspace: &std::path::Path,
    thread_id: &str,
) -> bool {
    assert!(crate::event::chat::detail_rewind::prepare(
        state,
        persistence,
        pane,
        DetailTarget::Btw(thread_id.to_string()),
        String::new(),
    ));
    let plan = state
        .pane_anywhere(pane)
        .and_then(|target| target.lx.task.as_ref())
        .and_then(|page| page.detail_rewind_pending.clone())
        .expect("btw undo plan");
    let operation_id = "btw-undo-test".to_string();
    if let Some(page) = state
        .pane_mut_anywhere(pane)
        .and_then(|target| target.lx.task.as_mut())
    {
        page.detail_rewind_executing = Some(DetailRewindExecution {
            operation_id: operation_id.clone(),
            target: plan.target.clone(),
        });
    }
    // 快照不可用时只删对话（与面板禁用含文件动作一致）。
    let action = if plan.snapshot.files_available() {
        plan.action
    } else {
        RewindAction::DeleteOnly
    };
    let request = RewindRequest {
        pane,
        session_id: plan.child_session.clone(),
        target_user_message_id: plan.user_message_id.clone(),
        expected_revision: plan.expected_revision,
        action,
        workspace: plan.workspace.clone(),
        input_at_open: plan.input_at_open.clone(),
        user_text: plan.user_text.clone(),
        operation_id: operation_id.clone(),
    };
    let database = persistence.database().expect("db");
    let outcome = crate::event::chat::rewind::execute_rewind(database, workspace, &request);
    crate::event::chat::detail_rewind::handle_finished(
        state,
        persistence,
        pane,
        workspace,
        &operation_id,
        outcome,
    )
}

#[tokio::test]
async fn btw_undo_deletes_the_last_turn_and_keeps_the_thread() {
    let (fixture, mut state, key, persistence, _streams, _parent) = fixture_with_parent();
    let thread = btw_fixture(&fixture, &mut state, &key);
    assert!(rewind_btw(
        &mut state,
        &persistence,
        key.pane,
        fixture.workspace(),
        &thread.id
    ));
    let thread_state = state
        .pane_anywhere(key.pane)
        .expect("pane")
        .lx
        .btw
        .thread_by_thread_id(&thread.id)
        .expect("thread kept");
    assert_eq!(thread_state.thread_id.as_deref(), Some(thread.id.as_str()));
    assert_eq!(
        thread_state.chat.messages.len(),
        2,
        "只删除最后一轮问答：{:?}",
        thread_state
            .chat
            .messages
            .iter()
            .map(|message| message.text())
            .collect::<Vec<_>>()
    );
}

#[tokio::test]
async fn btw_undo_of_the_only_turn_resets_the_thread() {
    let (fixture, mut state, key, persistence, _streams, _parent) = fixture_with_parent();
    let database = fixture.database();
    let messages = database
        .load_chat_session(fixture.workspace(), &key.session)
        .expect("load parent")
        .expect("parent session");
    let qa_id = messages
        .iter()
        .find(|message| message.role == "user")
        .expect("parent user")
        .external_id
        .clone();
    let thread = database
        .ensure_btw_thread(fixture.workspace(), &key.session, &qa_id, "side thread")
        .expect("thread");
    let turn = database
        .begin_turn(
            fixture.workspace(),
            Some(&thread.child_session_id),
            "only question",
            &model(),
        )
        .expect("turn");
    database
        .finish(
            fixture.workspace(),
            &turn.assistant_message_id,
            &[],
            &crate::db::chat::TurnOutcome::Complete {
                stop_reason: Some("complete".into()),
            },
            Usage::default(),
        )
        .expect("finish");
    {
        let target = state.pane_mut_anywhere(key.pane).expect("pane");
        target.lx.task = Some(crate::app::state::TaskPage {
            tab: TaskTab::Btw,
            ..Default::default()
        });
        let thread_state = target.lx.btw.ensure_thread("qa-1");
        thread_state.thread_id = Some(thread.id.clone());
        thread_state.child_session_id = Some(thread.child_session_id.clone());
        thread_state.loaded = true;
        target.lx.btw.editing = true;
    }
    assert!(rewind_btw(
        &mut state,
        &persistence,
        key.pane,
        fixture.workspace(),
        &thread.id
    ));
    let thread_state = state
        .pane_anywhere(key.pane)
        .expect("pane")
        .lx
        .btw
        .active_thread()
        .expect("thread view kept");
    assert!(thread_state.thread_id.is_none(), "删空后线程重置");
    assert!(thread_state.chat.messages.is_empty());
}

/// 文件快照不可用（缺 scope）：直接拒绝，不打开确认面板。
#[tokio::test]
async fn detail_rewind_disables_file_actions_when_snapshot_is_unavailable() {
    let (fixture, mut state, key, persistence, mut streams, _parent) = fixture_with_parent();
    let agent_id = spawn_explorer(&mut state, &mut streams, &persistence, &key, "first task").await;
    let child_session = streams
        .subagents
        .values()
        .find(|handle| handle.agent_id == agent_id)
        .expect("handle")
        .child_session_id
        .clone();
    crate::event::chat::finish_subagent(
        &mut state,
        &mut streams,
        &persistence,
        RequestKey {
            pane: key.pane,
            session: child_session.clone(),
            generation: key.generation,
        },
        StopReason::Complete,
        Usage::default(),
    );
    run_child_turn(
        &mut state,
        &mut streams,
        &persistence,
        &key,
        &child_session,
        &agent_id,
        "private follow-up",
        false,
    );
    {
        let target = state.pane_mut_anywhere(key.pane).expect("pane");
        target.lx.agents.ensure_composer(&agent_id);
        let page = target.lx.task.get_or_insert_with(Default::default);
        page.tab = TaskTab::Agents;
        page.agent_edit = Some(agent_id.clone());
    }
    let turn = fixture
        .database()
        .latest_undoable_private_turn(fixture.workspace(), &child_session)
        .expect("query")
        .expect("undoable turn");
    let conn = rusqlite::Connection::open(fixture.db_path()).expect("raw db");
    conn.execute(
        "DELETE FROM agent_turn_file_scope WHERE user_message_id = ?1",
        [&turn.user_message_id],
    )
    .expect("drop scope");
    drop(conn);
    state.effects.clear();
    assert!(crate::event::chat::detail_rewind::prepare(
        &mut state,
        &persistence,
        key.pane,
        DetailTarget::Agent(agent_id.clone()),
        String::new(),
    ));
    let plan = state
        .pane_anywhere(key.pane)
        .and_then(|target| target.lx.task.as_ref())
        .and_then(|page| page.detail_rewind_pending.clone())
        .expect("快照不可用也要打开确认面板");
    assert_eq!(plan.snapshot, crate::app::state::SnapshotState::Unavailable);
    let panel = state
        .pane_anywhere(key.pane)
        .and_then(|target| target.lx.active_editor().slash.clone())
        .and_then(|panel| match panel {
            SlashPanel::Rewind(panel) => Some(panel),
            _ => None,
        })
        .expect("rewind panel");
    assert!(!panel.enabled(RewindAction::RevertAndDelete));
    assert!(!panel.enabled(RewindAction::RevertOnly));
    assert!(panel.enabled(RewindAction::DeleteOnly));
}

/// 运行中的子 agent 与加载中的 BTW 线程都拒绝 `/rewind` 预检。
#[tokio::test]
async fn detail_rewind_rejects_busy_agent_and_btw_thread() {
    let (fixture, mut state, key, persistence, mut streams, _parent) = fixture_with_parent();
    let agent_id = spawn_explorer(&mut state, &mut streams, &persistence, &key, "first task").await;
    {
        let target = state.pane_mut_anywhere(key.pane).expect("pane");
        target.lx.agents.ensure_composer(&agent_id);
        let page = target.lx.task.get_or_insert_with(Default::default);
        page.tab = TaskTab::Agents;
        page.agent_edit = Some(agent_id.clone());
    }
    state.effects.clear();
    assert!(crate::event::chat::detail_rewind::prepare(
        &mut state,
        &persistence,
        key.pane,
        DetailTarget::Agent(agent_id.clone()),
        String::new(),
    ));
    assert!(
        !state.effects.iter().any(|effect| matches!(
            effect,
            Effect::PrepareDetailRewind { .. } | Effect::ExecuteDetailRewind(_)
        )),
        "运行中不得预检"
    );
    // 压缩中同样拒绝：检查点提交会改 revision。
    state
        .pane_mut_anywhere(key.pane)
        .expect("pane")
        .lx
        .agents
        .by_agent_mut(&agent_id)
        .expect("run")
        .compact
        .start("compact-op".into(), 0);
    state.effects.clear();
    assert!(crate::event::chat::detail_rewind::prepare(
        &mut state,
        &persistence,
        key.pane,
        DetailTarget::Agent(agent_id),
        String::new(),
    ));
    assert!(
        !state.effects.iter().any(|effect| matches!(
            effect,
            Effect::PrepareDetailRewind { .. } | Effect::ExecuteDetailRewind(_)
        )),
        "压缩中不得预检"
    );
    state
        .pane_mut_anywhere(key.pane)
        .expect("pane")
        .lx
        .agents
        .runs
        .iter_mut()
        .for_each(|run| run.compact.reset());

    // BTW：先把 spawned run 停掉，避免影响后面的状态。
    let child_session = streams
        .subagents
        .values()
        .next()
        .expect("handle")
        .child_session_id
        .clone();
    crate::event::chat::finish_subagent(
        &mut state,
        &mut streams,
        &persistence,
        RequestKey {
            pane: key.pane,
            session: child_session,
            generation: key.generation,
        },
        StopReason::Complete,
        Usage::default(),
    );
    let thread = btw_fixture(&fixture, &mut state, &key);
    state
        .pane_anywhere(key.pane)
        .expect("pane")
        .lx
        .btw
        .thread_by_thread_id(&thread.id)
        .expect("thread");
    state
        .pane_mut_anywhere(key.pane)
        .expect("pane")
        .lx
        .btw
        .active_thread_mut()
        .expect("thread")
        .pending_recipients = 1;
    state.effects.clear();
    assert!(crate::event::chat::detail_rewind::prepare(
        &mut state,
        &persistence,
        key.pane,
        DetailTarget::Btw(thread.id.clone()),
        String::new(),
    ));
    assert!(
        !state.effects.iter().any(|effect| matches!(
            effect,
            Effect::PrepareDetailRewind { .. } | Effect::ExecuteDetailRewind(_)
        )),
        "BTW 生成中不得预检"
    );
    // 线程压缩中同样拒绝。
    {
        let thread_state = state
            .pane_mut_anywhere(key.pane)
            .expect("pane")
            .lx
            .btw
            .active_thread_mut()
            .expect("thread");
        thread_state.pending_recipients = 0;
        thread_state.compact.start("compact-op".into(), 0);
    }
    state.effects.clear();
    assert!(crate::event::chat::detail_rewind::prepare(
        &mut state,
        &persistence,
        key.pane,
        DetailTarget::Btw(thread.id),
        String::new(),
    ));
    assert!(
        !state.effects.iter().any(|effect| matches!(
            effect,
            Effect::PrepareDetailRewind { .. } | Effect::ExecuteDetailRewind(_)
        )),
        "BTW 压缩中不得预检"
    );
}

/// compact 检查点比最新轮次更晚：`/rewind` 恢复上下文，不删消息也不碰文件。
#[tokio::test]
async fn detail_rewind_restores_the_latest_compact_checkpoint() {
    use crate::db::chat::CompactCommit;

    let (fixture, mut state, key, persistence, mut streams, _parent) = fixture_with_parent();
    let agent_id = spawn_explorer(&mut state, &mut streams, &persistence, &key, "first task").await;
    let child_session = streams
        .subagents
        .values()
        .find(|handle| handle.agent_id == agent_id)
        .expect("handle")
        .child_session_id
        .clone();
    crate::event::chat::finish_subagent(
        &mut state,
        &mut streams,
        &persistence,
        RequestKey {
            pane: key.pane,
            session: child_session.clone(),
            generation: key.generation,
        },
        StopReason::Complete,
        Usage::default(),
    );
    {
        let target = state.pane_mut_anywhere(key.pane).expect("pane");
        target.lx.agents.ensure_composer(&agent_id);
        let page = target.lx.task.get_or_insert_with(Default::default);
        page.tab = TaskTab::Agents;
        page.agent_edit = Some(agent_id.clone());
    }
    let database = fixture.database();
    let revision = database.session_revision(&child_session).expect("revision");
    let latest = database
        .latest_user_turn(fixture.workspace(), &child_session)
        .expect("latest")
        .expect("turn");
    let committed = database
        .commit_compact(
            fixture.workspace(),
            &CompactCommit {
                session_id: child_session.clone(),
                expected_revision: revision,
                base_checkpoint_id: None,
                start_sequence: 0,
                end_sequence: latest.sequence,
                summary_text: "summary".into(),
                model_label: "m".into(),
                operation_id: "compact-rewind-test".into(),
            },
        )
        .expect("commit compact");
    assert!(crate::event::chat::detail_rewind::prepare(
        &mut state,
        &persistence,
        key.pane,
        DetailTarget::Agent(agent_id.clone()),
        String::new(),
    ));
    let plan = state
        .pane_anywhere(key.pane)
        .and_then(|target| target.lx.task.as_ref())
        .and_then(|page| page.detail_rewind_pending.clone())
        .expect("compact rewind plan");
    assert_eq!(plan.kind, crate::app::state::RewindTargetKind::Compact);
    assert_eq!(plan.action, RewindAction::RestoreCompact);
    assert_eq!(plan.user_message_id, committed.checkpoint.id);
    let operation_id = "detail-compact-rewind-test".to_string();
    if let Some(page) = state
        .pane_mut_anywhere(key.pane)
        .and_then(|target| target.lx.task.as_mut())
    {
        page.detail_rewind_executing = Some(DetailRewindExecution {
            operation_id: operation_id.clone(),
            target: plan.target.clone(),
        });
    }
    let request = crate::app::effects::DetailRewindRequest {
        pane: key.pane,
        target: plan.target.clone(),
        operation_id: operation_id.clone(),
        request: RewindRequest {
            pane: key.pane,
            session_id: plan.child_session.clone(),
            target_user_message_id: plan.user_message_id.clone(),
            expected_revision: plan.expected_revision,
            action: plan.action,
            workspace: plan.workspace.clone(),
            input_at_open: plan.input_at_open.clone(),
            user_text: plan.user_text.clone(),
            operation_id: operation_id.clone(),
        },
    };
    let (sender, mut receiver) = mpsc::unbounded_channel::<AppEvent>();
    let execute_persistence = Persistence::with_database_at(fixture.database(), fixture.db_path());
    crate::event::chat::detail_rewind::execute(&sender, &execute_persistence, request);
    let AppEvent::RewindFinished { outcome, .. } =
        tokio::time::timeout(std::time::Duration::from_secs(10), receiver.recv())
            .await
            .expect("compact 撤销必须回投")
            .expect("rewind event")
    else {
        panic!("expected RewindFinished");
    };
    assert!(
        matches!(
            &outcome,
            crate::event::chat::RewindOutcome::Succeeded {
                action: RewindAction::RestoreCompact,
                ..
            }
        ),
        "compact 检查点应恢复成功：{outcome:?}"
    );
    assert!(crate::event::chat::detail_rewind::handle_finished(
        &mut state,
        &persistence,
        key.pane,
        fixture.workspace(),
        &operation_id,
        outcome,
    ));
    assert!(
        fixture
            .database()
            .compact_checkpoints(fixture.workspace(), &child_session)
            .expect("checkpoints")
            .is_empty(),
        "检查点已撤销"
    );
    let run = state
        .pane_anywhere(key.pane)
        .expect("pane")
        .lx
        .agents
        .by_agent(&agent_id)
        .expect("run");
    assert!(run.compact.checkpoints.is_empty(), "详情卡片同步移除");
    assert!(
        run.chat
            .messages
            .iter()
            .any(|message| message.text().contains("first task")),
        "消息保持原样：{:?}",
        run.chat
            .messages
            .iter()
            .map(|message| message.text())
            .collect::<Vec<_>>()
    );
}
