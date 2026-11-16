//! 事件层 steer 交付：DB 原子换轮、assistant id 切换与 pending 消费。

use super::*;
use crate::app::state::lx::chat::PendingSteer;
use crate::app::state::{AssistantStepView, ChatMessageView, ChatPartView, FrozenModel};
use crate::chat::events::PartKind;
use crate::db::chat::ModelSnapshot;

fn snapshot() -> ModelSnapshot {
    ModelSnapshot {
        provider: "p".into(),
        model: "m".into(),
        variant: None,
    }
}

fn frozen() -> FrozenModel {
    FrozenModel {
        provider: "p".into(),
        model: "m".into(),
        variant: None,
        transport: "openai".into(),
        base_url: "https://api.example.invalid".into(),
        api_key: "key".into(),
        context_limit: Some(10_000),
        output_limit: Some(1_000),
        variant_params: None,
        idle_timeout_ms: 1_000,
        supports_image_input: false,
    }
}

fn assistant_view(committed: &crate::db::chat::TurnCommitted) -> ChatMessageView {
    ChatMessageView {
        sequence: committed.assistant_sequence,
        id: committed.assistant_message_id.clone(),
        kind: "chat".into(),
        role: "assistant".into(),
        status: "streaming".into(),
        turn_index: committed.turn_index,
        shell: None,
        steps: vec![AssistantStepView::Part {
            position: 0,
            part: ChatPartView {
                kind: PartKind::Text,
                content: "before steer".into(),
                provider_metadata_json: None,
                started_at_ms: None,
                ended_at_ms: None,
                completed_at_ms: None,
            },
        }],
        model_label: Some("M".into()),
        error_code: None,
        input_tokens: None,
        output_tokens: None,
        cache_read_tokens: None,
        duration_ms: None,
        reports: Vec::new(),
        attachments: Vec::new(),
    }
}

#[test]
fn deliver_closes_previous_segment_and_switches_assistant() {
    let fixture = Fixture::new("steer-deliver");
    let database = fixture.database();
    let persistence = Persistence::with_database_at(database, fixture.db_path());
    let committed = persistence
        .database()
        .expect("db")
        .begin_turn(fixture.workspace(), None, "initial", &snapshot())
        .expect("begin turn");
    let mut state = AppState::demo();
    state.models = models_with();
    let pane = state.active_tab().layout.focus();
    let key = RequestKey {
        pane,
        session: committed.session_id.clone(),
        generation: 1,
    };
    {
        let target = state.pane_mut_anywhere(pane).expect("pane");
        target.lx.chat.session_id = Some(committed.session_id.clone());
        target.lx.chat.request = RequestState::Streaming {
            key: key.clone(),
            assistant_id: committed.assistant_message_id.clone(),
        };
        target.lx.chat.turn_started_at_ms = Some(committed.created_at_ms);
        target.lx.chat.messages.push(assistant_view(&committed));
        target.lx.chat.pending_steers.push(PendingSteer {
            text: "steer text".into(),
            attachments: Vec::new(),
            model_images: Vec::new(),
            model: snapshot(),
        });
    }
    let mutation_user = Arc::new(std::sync::RwLock::new(committed.user_message_id.clone()));
    let mut streams = ChatStreams::default();
    streams
        .db_workspaces
        .insert(key.clone(), fixture.workspace().to_path_buf());
    streams.frozen.insert(key.session.clone(), frozen());
    streams
        .mutation_users
        .insert(key.clone(), mutation_user.clone());
    let (reply, mut receiver) = tokio::sync::oneshot::channel();
    assert!(super::super::steer::deliver(
        &mut state,
        &mut streams,
        &persistence,
        key.clone(),
        reply,
    ));
    let inputs = receiver.try_recv().expect("reply").expect("ok");
    assert_eq!(inputs.len(), 1);
    assert_eq!(inputs[0].text, "steer text");
    let target = state.pane_anywhere(pane).expect("pane");
    let RequestState::Streaming {
        assistant_id: next, ..
    } = &target.lx.chat.request
    else {
        panic!("expected streaming");
    };
    assert_ne!(next, &committed.assistant_message_id);
    assert!(target.lx.chat.pending_steers.is_empty());
    assert_eq!(
        target
            .lx
            .chat
            .messages
            .iter()
            .find(|message| message.id == committed.assistant_message_id)
            .map(|message| (message.status.as_str(), message.duration_ms)),
        Some(("complete", None)),
        "被 steer 截断的旧 assistant 不写耗时、不渲染统计行"
    );
    let steered_user_id = target
        .lx
        .chat
        .messages
        .iter()
        .rev()
        .find(|message| message.role == "user")
        .expect("new user")
        .id
        .clone();
    assert!(
        target.lx.chat.is_steered_user_message(&steered_user_id),
        "steer 插入的用户消息应记录在 steered_user_messages 集合中"
    );
    assert_eq!(*mutation_user.read().expect("lock"), steered_user_id);
    let loaded = persistence
        .database()
        .expect("db")
        .load_chat_session(fixture.workspace(), &committed.session_id)
        .expect("load")
        .expect("session");
    let statuses: Vec<_> = loaded
        .iter()
        .map(|message| (message.role.as_str(), message.status.as_str()))
        .collect();
    assert_eq!(
        statuses,
        vec![
            ("user", "complete"),
            ("assistant", "complete"),
            ("user", "complete"),
            ("assistant", "streaming"),
        ]
    );
}

#[test]
fn has_pending_is_read_only_and_requires_active_key() {
    let fixture = Fixture::new("steer-has-pending");
    let database = fixture.database();
    let committed = database
        .begin_turn(fixture.workspace(), None, "initial", &snapshot())
        .expect("begin turn");
    let mut state = AppState::demo();
    let pane = state.active_tab().layout.focus();
    let key = RequestKey {
        pane,
        session: committed.session_id.clone(),
        generation: 1,
    };
    {
        let target = state.pane_mut_anywhere(pane).expect("pane");
        target.lx.chat.session_id = Some(committed.session_id.clone());
        target.lx.chat.request = RequestState::Streaming {
            key: key.clone(),
            assistant_id: committed.assistant_message_id.clone(),
        };
        target.lx.chat.pending_steers.push(PendingSteer {
            text: "x".into(),
            attachments: Vec::new(),
            model_images: Vec::new(),
            model: snapshot(),
        });
    }
    assert!(super::super::steer::has_pending(&state, &key));
    let target = state.pane_anywhere(pane).expect("pane");
    assert_eq!(target.lx.chat.pending_steers.len(), 1, "探测不消费");
}

#[test]
fn deliver_rejects_images_when_model_lacks_image_support_and_keeps_pending() {
    let fixture = Fixture::new("steer-image-unsupported");
    let database = fixture.database();
    let persistence = Persistence::with_database_at(database, fixture.db_path());
    let committed = persistence
        .database()
        .expect("db")
        .begin_turn(fixture.workspace(), None, "initial", &snapshot())
        .expect("begin turn");
    let mut state = AppState::demo();
    state.models = models_with();
    let pane = state.active_tab().layout.focus();
    let key = RequestKey {
        pane,
        session: committed.session_id.clone(),
        generation: 1,
    };
    {
        let target = state.pane_mut_anywhere(pane).expect("pane");
        target.lx.chat.session_id = Some(committed.session_id.clone());
        target.lx.chat.request = RequestState::Streaming {
            key: key.clone(),
            assistant_id: committed.assistant_message_id.clone(),
        };
        target.lx.chat.messages.push(assistant_view(&committed));
        target.lx.chat.pending_steers.push(PendingSteer {
            text: "look at image".into(),
            attachments: Vec::new(),
            model_images: vec![crate::chat::events::ModelImage {
                mime: "image/png".into(),
                bytes: Arc::new(vec![1, 2, 3]),
            }],
            model: snapshot(),
        });
    }
    let mut streams = ChatStreams::default();
    streams
        .db_workspaces
        .insert(key.clone(), fixture.workspace().to_path_buf());
    streams.frozen.insert(key.session.clone(), frozen());
    let (reply, mut receiver) = tokio::sync::oneshot::channel();
    assert!(!super::super::steer::deliver(
        &mut state,
        &mut streams,
        &persistence,
        key,
        reply,
    ));
    let err = receiver.try_recv().expect("reply").expect_err("failed");
    assert_eq!(
        err,
        HostError::Failed {
            code: super::super::steer::STEER_IMAGE_UNSUPPORTED_CODE.to_string(),
        }
    );
    let target = state.pane_anywhere(pane).expect("pane");
    assert_eq!(
        target.lx.chat.pending_steers.len(),
        1,
        "失败时不消费 pending"
    );
}

#[test]
fn deliver_preserves_pending_when_db_begin_steer_turn_fails() {
    let fixture = Fixture::new("steer-db-fail");
    let database = fixture.database();
    let persistence = Persistence::with_database_at(database, fixture.db_path());
    let committed = persistence
        .database()
        .expect("db")
        .begin_turn(fixture.workspace(), None, "initial", &snapshot())
        .expect("begin turn");
    let mut state = AppState::demo();
    state.models = models_with();
    let pane = state.active_tab().layout.focus();
    let key = RequestKey {
        pane,
        session: committed.session_id.clone(),
        generation: 1,
    };
    {
        let target = state.pane_mut_anywhere(pane).expect("pane");
        target.lx.chat.session_id = Some(committed.session_id.clone());
        target.lx.chat.request = RequestState::Streaming {
            key: key.clone(),
            assistant_id: "missing-assistant".into(),
        };
        target.lx.chat.pending_steers.push(PendingSteer {
            text: "steer text".into(),
            attachments: Vec::new(),
            model_images: Vec::new(),
            model: snapshot(),
        });
    }
    let mut streams = ChatStreams::default();
    streams
        .db_workspaces
        .insert(key.clone(), fixture.workspace().to_path_buf());
    streams.frozen.insert(key.session.clone(), frozen());
    let (reply, mut receiver) = tokio::sync::oneshot::channel();
    assert!(!super::super::steer::deliver(
        &mut state,
        &mut streams,
        &persistence,
        key,
        reply,
    ));
    let err = receiver.try_recv().expect("reply").expect_err("failed");
    assert!(matches!(
        err,
        HostError::Failed { ref code } if code.starts_with(super::super::steer::STEER_TURN_BEGIN_FAILED_CODE)
    ));
    let target = state.pane_anywhere(pane).expect("pane");
    assert_eq!(
        target.lx.chat.pending_steers.len(),
        1,
        "DB 失败时不消费 pending"
    );
}

#[test]
fn mutation_after_steer_belongs_to_steered_user_and_rewind_reverts_only_steered_segment() {
    use crate::chat::tools::{
        MutationOperation, MutationRecorderFactory, ObservedMutation, ObservedPath, PathMutation,
        PreparedMutation,
    };
    use crate::platform::RawImage;

    let fixture = Fixture::new("steer-mutation-rewind");
    let database = fixture.database();
    let persistence = Persistence::with_database_at(database, fixture.db_path());
    let committed = persistence
        .database()
        .expect("db")
        .begin_turn(fixture.workspace(), None, "initial", &snapshot())
        .expect("begin turn");
    let mut state = AppState::demo();
    state.models = models_with();
    let pane = state.active_tab().layout.focus();
    let key = RequestKey {
        pane,
        session: committed.session_id.clone(),
        generation: 1,
    };
    {
        let target = state.pane_mut_anywhere(pane).expect("pane");
        target.lx.chat.session_id = Some(committed.session_id.clone());
        target.lx.chat.request = RequestState::Streaming {
            key: key.clone(),
            assistant_id: committed.assistant_message_id.clone(),
        };
        target.lx.chat.messages.push(assistant_view(&committed));
        target.lx.chat.pending_steers.push(PendingSteer {
            text: "steer text".into(),
            attachments: Vec::new(),
            model_images: Vec::new(),
            model: snapshot(),
        });
    }
    let mutation_user = Arc::new(std::sync::RwLock::new(committed.user_message_id.clone()));
    let (sender, mut events) = mpsc::unbounded_channel();
    let factory = super::super::file_snapshots::HostRecorderFactory::new(
        sender,
        fixture.workspace().to_path_buf(),
        committed.session_id.clone(),
        mutation_user.clone(),
    );
    let mut streams = ChatStreams::default();
    streams
        .db_workspaces
        .insert(key.clone(), fixture.workspace().to_path_buf());
    streams.frozen.insert(key.session.clone(), frozen());
    streams
        .mutation_users
        .insert(key.clone(), mutation_user.clone());

    let mut record_file_change = |call_id: &str, rel: &str, after: &[u8]| {
        let recorder = factory.for_call(call_id);
        let path = fixture.workspace().join(rel);
        let handle = std::thread::spawn({
            let path = path.clone();
            let after = after.to_vec();
            move || {
                let raw_after = RawImage::regular(after, Some(0o644));
                let ticket = recorder
                    .prepare(&PreparedMutation {
                        paths: vec![PathMutation {
                            operation: MutationOperation::Create,
                            path: path.clone(),
                            target: path,
                            before: RawImage::missing(),
                            planned_after: raw_after.clone(),
                            expected_version: None,
                        }],
                    })
                    .expect("prepare");
                recorder
                    .commit(
                        ticket,
                        &ObservedMutation {
                            paths: vec![ObservedPath {
                                actual_after: raw_after,
                                success: true,
                            }],
                        },
                    )
                    .expect("commit");
            }
        });
        for _ in 0..2 {
            let Some(crate::event::AppEvent::AgentHost(
                super::super::host::AgentHostEvent::Mutation(event),
            )) = events.blocking_recv()
            else {
                panic!("expected mutation event");
            };
            super::super::file_snapshots::handle_mutation(&persistence, event);
        }
        handle.join().expect("worker join");
    };

    // 1. steer 前的工具修改归属初始 user_message_id。
    record_file_change("call-before", "before.txt", b"new-1");

    // 2. steer 生效：共享 mutation_user 原地切到新 user_message_id。
    let (reply, mut receiver) = tokio::sync::oneshot::channel();
    assert!(super::super::steer::deliver(
        &mut state,
        &mut streams,
        &persistence,
        key,
        reply,
    ));
    let _ = receiver.try_recv().expect("reply").expect("ok");
    let steered_user_id = mutation_user.read().expect("lock").clone();
    assert_ne!(steered_user_id, committed.user_message_id);

    // 3. steer 后的工具修改自动使用同一 factory 读取到的新 user_message_id。
    record_file_change("call-after", "after.txt", b"new-2");

    let steered_assistant_id = match &state.pane_anywhere(pane).expect("pane").lx.chat.request {
        RequestState::Streaming { assistant_id, .. } => assistant_id.clone(),
        other => panic!("expected streaming, got {other:?}"),
    };
    let db = persistence.database().expect("db");
    db.finish(
        fixture.workspace(),
        &steered_assistant_id,
        &[],
        &crate::db::chat::TurnOutcome::Complete { stop_reason: None },
        crate::db::chat::Usage::default(),
    )
    .expect("finish steered turn");

    // 4. 针对 steer 轮次的 rewind 预检只包含 after.txt，不误伤 before.txt。
    let plan = db
        .restore_plan(fixture.workspace(), &committed.session_id, &steered_user_id)
        .expect("restore plan");
    let paths: Vec<_> = plan
        .paths
        .iter()
        .map(|entry| entry.path.as_path())
        .collect();
    assert_eq!(
        paths,
        vec![fixture.workspace().join("after.txt").as_path()],
        "steer 轮次 rewind 只回滚 steer 之后的文件修改"
    );
}

#[test]
fn subagent_steer_inputs_returns_empty_without_cancelling_child_turn() {
    let fixture = Fixture::new("steer-subagent-host");
    let database = fixture.database();
    let persistence = Persistence::with_database_at(database, fixture.db_path());
    let committed = persistence
        .database()
        .expect("db")
        .begin_turn(fixture.workspace(), None, "parent", &snapshot())
        .expect("begin turn");
    let mut state = AppState::demo();
    let pane = state.active_tab().layout.focus();
    let parent_key = RequestKey {
        pane,
        session: committed.session_id.clone(),
        generation: 1,
    };
    {
        let target = state.pane_mut_anywhere(pane).expect("pane");
        target.lx.chat.session_id = Some(committed.session_id.clone());
        target.lx.chat.request = RequestState::Streaming {
            key: parent_key.clone(),
            assistant_id: committed.assistant_message_id.clone(),
        };
    }
    let child_key = RequestKey {
        pane,
        session: "sess-child-1".into(),
        generation: 1,
    };
    let (cancel_tx, _cancel_rx) = tokio::sync::watch::channel(false);
    let (done_tx, _done_rx) = tokio::sync::watch::channel(false);
    let mut streams = ChatStreams::default();
    streams.cancels.insert(child_key.clone(), cancel_tx.clone());
    streams.subagents.insert(
        child_key.session.clone(),
        super::super::subagent::SubagentHandle {
            parent: parent_key,
            agent_id: "sub-1".into(),
            nickname: Some("Turing".into()),
            agent_path: "/root/turing".into(),
            root_session: committed.session_id,
            depth: 1,
            child_session_id: child_key.session.clone(),
            child_assistant_id: "msg-child-a".into(),
            child_user_message_id: "msg-child-u".into(),
            turn_user_message_ids: vec!["msg-child-u".into()],
            role: crate::chat::agent::AgentRole::Explorer,
            title: "check".into(),
            task: "explore".into(),
            cancel: cancel_tx.clone(),
            done: done_tx,
            status: super::super::subagent::STATUS_RUNNING,
            started_at_ms: 0,
            max_tokens: None,
            closed_without_report: false,
            stopped_by_user: false,
            notify_parent: true,
            report_to_input: false,
            mutation_user_id: std::sync::Arc::new(std::sync::RwLock::new(String::new())),
            chat: crate::app::state::LxChat::new(),
            workspace: fixture.workspace().to_path_buf(),
        },
    );

    // 运行中的子 agent 请求 SteerInputs：必须回 Ok(empty)，不能因父窗格 key 不匹配而误报 Cancelled。
    let (reply, mut receiver) = tokio::sync::oneshot::channel();
    assert!(!super::super::steer::deliver(
        &mut state,
        &mut streams,
        &persistence,
        child_key.clone(),
        reply,
    ));
    let batch = receiver.try_recv().expect("reply").expect("child ok");
    assert!(batch.is_empty());

    // 子 agent 已置取消标志时返回 Cancelled。
    let _ = cancel_tx.send(true);
    let (reply2, mut receiver2) = tokio::sync::oneshot::channel();
    assert!(!super::super::steer::deliver(
        &mut state,
        &mut streams,
        &persistence,
        child_key,
        reply2,
    ));
    assert_eq!(
        receiver2
            .try_recv()
            .expect("reply2")
            .expect_err("cancelled"),
        HostError::Cancelled
    );
}

#[test]
fn session_reload_rebuilds_steered_user_messages_and_hides_steered_assistant_duration() {
    let fixture = Fixture::new("steer-reload-view");
    let database = fixture.database();
    let first = database
        .begin_turn(fixture.workspace(), None, "u1", &snapshot())
        .expect("begin turn");
    let steered = database
        .begin_steer_turn(
            fixture.workspace(),
            &first.session_id,
            &first.assistant_message_id,
            &[],
            "u2 steered",
            &[],
            &snapshot(),
        )
        .expect("steer turn");
    database
        .finish(
            fixture.workspace(),
            &steered.assistant_message_id,
            &[],
            &crate::db::chat::TurnOutcome::Complete { stop_reason: None },
            crate::db::chat::Usage::default(),
        )
        .expect("finish");

    let messages = database
        .load_chat_session(fixture.workspace(), &first.session_id)
        .expect("load")
        .expect("session");
    let mut state = AppState::demo();
    state.models = models_with();
    let pane = state.active_tab().layout.focus();
    crate::app::update::on_session_loaded(&mut state, pane, &first.session_id, &messages, None);

    let target = state.pane_anywhere(pane).expect("pane");
    assert!(
        !target
            .lx
            .chat
            .is_steered_user_message(&first.user_message_id)
    );
    assert!(
        target
            .lx
            .chat
            .is_steered_user_message(&steered.user_message_id)
    );
    assert_eq!(
        target.lx.chat.messages[1].duration_ms, None,
        "被 steer 截断的旧 assistant 重载后同样不带 duration_ms"
    );
}

#[test]
fn deliver_injects_subagent_reference_blocks_and_checks_session_binding() {
    let fixture = Fixture::new("steer-subagent-ref");
    let database = fixture.database();
    let persistence = Persistence::with_database_at(database, fixture.db_path());
    let committed = persistence
        .database()
        .expect("db")
        .begin_turn(fixture.workspace(), None, "initial", &snapshot())
        .expect("begin turn");
    let mut state = AppState::demo();
    state.models = models_with();
    let pane = state.active_tab().layout.focus();
    let key = RequestKey {
        pane,
        session: committed.session_id.clone(),
        generation: 1,
    };
    {
        let target = state.pane_mut_anywhere(pane).expect("pane");
        target.lx.chat.session_id = Some(committed.session_id.clone());
        target.lx.chat.request = RequestState::Streaming {
            key: key.clone(),
            assistant_id: committed.assistant_message_id.clone(),
        };
        target.lx.chat.turn_started_at_ms = Some(committed.created_at_ms);
        target.lx.chat.messages.push(assistant_view(&committed));
        target.lx.chat.pending_steers.push(PendingSteer {
            text: "Use @[refer-subagent](explorer) here".into(),
            attachments: Vec::new(),
            model_images: Vec::new(),
            model: snapshot(),
        });
    }
    let mut streams = ChatStreams::default();
    streams
        .db_workspaces
        .insert(key.clone(), fixture.workspace().to_path_buf());
    streams.frozen.insert(key.session.clone(), frozen());

    let (reply, mut rx) = tokio::sync::oneshot::channel();
    assert!(super::super::steer::deliver(
        &mut state,
        &mut streams,
        &persistence,
        key.clone(),
        reply,
    ));
    let delivered = rx.try_recv().expect("reply").expect("deliver ok");
    assert_eq!(delivered.len(), 1);
    assert!(
        delivered[0]
            .skill_blocks
            .iter()
            .any(|block| block.contains("<subagent_reference>") && block.contains("explorer")),
        "steer 中 @[refer-subagent](explorer) 应注入 <subagent_reference> 块：{:?}",
        delivered[0].skill_blocks
    );

    // 窗格 session_id 被切走但 request 尚未清理时，deliver 必须拒绝并返回 Cancelled。
    state
        .pane_mut_anywhere(key.pane)
        .expect("pane")
        .lx
        .chat
        .session_id = Some("other-session".into());
    let (reply2, mut rx2) = tokio::sync::oneshot::channel();
    assert!(!super::super::steer::deliver(
        &mut state,
        &mut streams,
        &persistence,
        key,
        reply2,
    ));
    assert_eq!(
        rx2.try_recv().expect("reply2").unwrap_err(),
        HostError::Cancelled
    );
}
