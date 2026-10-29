//! 子 agent 多轮运行时测试：排队与插队、显式 resume、失败暂停队列、缺图阻断与取消、
//! 跨重启重放、自动压缩检查点持久化、多轮独立报告投递与 UI 状态投影。

use base64::Engine;
use image::{ImageBuffer, ImageFormat, Rgba};
use std::io::Cursor;
use tokio::sync::mpsc;

use super::*;
use crate::app::state::{AppState, RequestKey};
use crate::chat::agent::{
    AgentRole, IMAGE_MISSING_CODE, NOT_ACTIVE_CODE, SubagentInputItem, SubagentOp, SubagentOutcome,
    WaitMode,
};
use crate::chat::compact::AutoCompactNotice;
use crate::chat::events::{ModelItem, StopReason};
use crate::db::chat::Usage;
use crate::db::service::Persistence;
use crate::event::AppEvent;
use crate::event::chat::tests::{Fixture, streaming_state};

fn make_test_png(w: u32, h: u32, seed: u8) -> Vec<u8> {
    let img: ImageBuffer<Rgba<u8>, Vec<u8>> = ImageBuffer::from_fn(w, h, |x, y| {
        Rgba([(x as u8).wrapping_add(seed), y as u8, seed, 255])
    });
    let mut bytes = Vec::new();
    img.write_to(&mut Cursor::new(&mut bytes), ImageFormat::Png)
        .expect("write png");
    bytes
}

fn setup_parent() -> (
    Fixture,
    AppState,
    RequestKey,
    Persistence,
    ChatStreams,
    String,
) {
    let fixture = Fixture::new("subagent-codex");
    let (state, key, persistence, _session) = streaming_state(&fixture);
    let mut frozen = crate::app::state::FrozenModel::resolve(
        &state.models,
        &crate::config::ai::ModelSelection {
            provider: "p".into(),
            model: "m".into(),
            variant: None,
        },
    )
    .expect("frozen");
    frozen.supports_image_input = true;
    let mut streams = ChatStreams::default();
    streams.frozen.insert(key.session.clone(), frozen);
    streams
        .db_workspaces
        .insert(key.clone(), fixture.workspace().to_path_buf());
    let (sender, receiver) = mpsc::unbounded_channel::<AppEvent>();
    streams.set_event_sender(sender);
    std::mem::forget(receiver);
    let (process_sender, process_receiver) =
        mpsc::unbounded_channel::<crate::chat::process::ProcessUiEvent>();
    streams.set_process_sender(process_sender);
    std::mem::forget(process_receiver);
    let parent = key.session.clone();
    (fixture, state, key, persistence, streams, parent)
}

async fn run_op(
    state: &mut AppState,
    streams: &mut ChatStreams,
    persistence: &Persistence,
    key: &RequestKey,
    call_id: Option<&str>,
    op: SubagentOp,
) -> Result<SubagentOutcome, HostError> {
    let (reply, receiver) = tokio::sync::oneshot::channel();
    handle_subagent_op_with_call(state, streams, persistence, key.clone(), call_id, op, reply);
    receiver.await.expect("reply")
}

#[tokio::test]
async fn queued_inputs_drain_in_order_and_require_resume_after_terminal_completion() {
    let (_fixture, mut state, key, persistence, mut streams, _parent) = setup_parent();
    let spawned = run_op(
        &mut state,
        &mut streams,
        &persistence,
        &key,
        Some("call-spawn"),
        SubagentOp::Spawn {
            role: AgentRole::Explorer,
            title: "Multi turn".into(),
            task: "turn 1".into(),
            items: Vec::new(),
        },
    )
    .await
    .expect("spawn");
    let SubagentOutcome::Spawned { agent_id, .. } = spawned else {
        panic!("expected spawned");
    };
    let child_session = streams
        .subagents
        .values()
        .find(|h| h.agent_id == agent_id)
        .map(|h| h.child_session_id.clone())
        .expect("child session");

    // 运行中发送普通输入：持久排队，不中断当前轮
    let sent = run_op(
        &mut state,
        &mut streams,
        &persistence,
        &key,
        Some("call-send-1"),
        SubagentOp::SendInput {
            target: agent_id.clone(),
            items: vec![SubagentInputItem::Text {
                text: "turn 2".into(),
            }],
            interrupt: false,
        },
    )
    .await
    .expect("send 1");
    let SubagentOutcome::InputAccepted {
        submission_id: sub_1,
        status,
        queued_count,
        ..
    } = sent
    else {
        panic!("expected input accepted");
    };
    assert_eq!(status, "queued");
    assert_eq!(queued_count, 1);

    // 同一 call_id 重试不会重复入队
    let dup = run_op(
        &mut state,
        &mut streams,
        &persistence,
        &key,
        Some("call-send-1"),
        SubagentOp::SendInput {
            target: agent_id.clone(),
            items: vec![SubagentInputItem::Text {
                text: "turn 2".into(),
            }],
            interrupt: false,
        },
    )
    .await
    .expect("send dup");
    if let SubagentOutcome::InputAccepted {
        submission_id,
        queued_count,
        ..
    } = dup
    {
        assert_eq!(submission_id, sub_1);
        assert_eq!(queued_count, 1);
    }

    // 第 1 轮完成：队列非空，自动接续启动第 2 轮
    finish_subagent(
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
    assert!(
        streams.subagents.contains_key(&child_session),
        "有排队输入时完成第 1 轮后应立即启动第 2 轮"
    );

    // 第 2 轮完成且队列已空：释放恢复标记，进入终态
    finish_subagent(
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
    assert!(!streams.subagents.contains_key(&child_session));
    assert!(!streams.resumed_subagents.contains(&agent_id));

    // 终态直接 send_input 必须拒绝，要求先 resume_agent
    let rejected = run_op(
        &mut state,
        &mut streams,
        &persistence,
        &key,
        Some("call-send-after-done"),
        SubagentOp::SendInput {
            target: agent_id.clone(),
            items: vec![SubagentInputItem::Text {
                text: "turn 3".into(),
            }],
            interrupt: false,
        },
    )
    .await
    .expect_err("must reject send_input when terminal");
    assert_eq!(
        rejected,
        HostError::Failed {
            code: NOT_ACTIVE_CODE.to_string()
        }
    );

    // 显式 resume_agent 后进入 idle（无队列），再次 resume 幂等
    let resumed = run_op(
        &mut state,
        &mut streams,
        &persistence,
        &key,
        Some("call-resume"),
        SubagentOp::Resume {
            id: agent_id.clone(),
        },
    )
    .await
    .expect("resume");
    assert_eq!(
        resumed,
        SubagentOutcome::Resumed {
            agent_id: agent_id.clone(),
            status: "idle".into(),
            queued_count: 0,
        }
    );

    // idle 状态下 send_input 立即启动第 3 轮
    let sent_3 = run_op(
        &mut state,
        &mut streams,
        &persistence,
        &key,
        Some("call-send-3"),
        SubagentOp::SendInput {
            target: agent_id.clone(),
            items: vec![SubagentInputItem::Text {
                text: "turn 3".into(),
            }],
            interrupt: false,
        },
    )
    .await
    .expect("send 3");
    if let SubagentOutcome::InputAccepted {
        status,
        queued_count,
        ..
    } = sent_3
    {
        assert_eq!(status, "running");
        assert_eq!(queued_count, 0);
    }
    assert!(streams.subagents.contains_key(&child_session));

    // 两轮完成已各自生成独立报告（共 2 条）
    let pending = persistence
        .database()
        .expect("db")
        .pending_reports(_fixture.workspace(), &key.session)
        .expect("pending reports");
    assert_eq!(pending.len(), 2);
}

#[tokio::test]
async fn interrupt_cancels_current_turn_without_report_and_runs_ahead_of_older_queue() {
    let (fixture, mut state, key, persistence, mut streams, _parent) = setup_parent();
    let SubagentOutcome::Spawned { agent_id, .. } = run_op(
        &mut state,
        &mut streams,
        &persistence,
        &key,
        Some("c-spawn"),
        SubagentOp::Spawn {
            role: AgentRole::Executor,
            title: "Interrupt test".into(),
            task: "turn 1".into(),
            items: Vec::new(),
        },
    )
    .await
    .expect("spawn") else {
        panic!("expected spawned");
    };
    let child_session = streams
        .subagents
        .values()
        .find(|h| h.agent_id == agent_id)
        .map(|h| h.child_session_id.clone())
        .expect("child");

    // 先排一条普通输入
    run_op(
        &mut state,
        &mut streams,
        &persistence,
        &key,
        Some("c-normal"),
        SubagentOp::SendInput {
            target: agent_id.clone(),
            items: vec![SubagentInputItem::Text {
                text: "normal queued".into(),
            }],
            interrupt: false,
        },
    )
    .await
    .expect("normal");

    // 再发一条 interrupt=true 输入
    let cancel_rx = streams
        .subagents
        .get(&child_session)
        .expect("handle")
        .cancel
        .subscribe();
    let interrupted = run_op(
        &mut state,
        &mut streams,
        &persistence,
        &key,
        Some("c-interrupt"),
        SubagentOp::SendInput {
            target: agent_id.clone(),
            items: vec![SubagentInputItem::Text {
                text: "urgent interrupt".into(),
            }],
            interrupt: true,
        },
    )
    .await
    .expect("interrupt");
    if let SubagentOutcome::InputAccepted {
        queued_count,
        interrupt,
        ..
    } = interrupted
    {
        assert!(interrupt);
        assert_eq!(queued_count, 2);
    }
    assert!(*cancel_rx.borrow(), "当前运行轮收到取消信号");

    // 旧轮以 Cancelled 封口：不生成报告，且立即启动 urgent interrupt 输入，保留旧普通队列（剩余 1 条）
    finish_subagent(
        &mut state,
        &mut streams,
        &persistence,
        RequestKey {
            pane: key.pane,
            session: child_session.clone(),
            generation: key.generation,
        },
        StopReason::Cancelled,
        Usage::default(),
    );
    let handle = streams.subagents.get(&child_session).expect("restarted");
    assert_eq!(handle.task, "urgent interrupt");
    let reports = persistence
        .database()
        .expect("db")
        .pending_reports(fixture.workspace(), &key.session)
        .expect("reports");
    assert!(reports.is_empty(), "被中断的子轮不生成完成报告");
}

#[tokio::test]
async fn failed_turn_pauses_queue_until_explicit_resume() {
    let (fixture, mut state, key, persistence, mut streams, _parent) = setup_parent();
    let SubagentOutcome::Spawned { agent_id, .. } = run_op(
        &mut state,
        &mut streams,
        &persistence,
        &key,
        Some("c-spawn"),
        SubagentOp::Spawn {
            role: AgentRole::Executor,
            title: "Fail test".into(),
            task: "turn 1".into(),
            items: Vec::new(),
        },
    )
    .await
    .expect("spawn") else {
        panic!("expected spawned");
    };
    let child_session = streams
        .subagents
        .values()
        .find(|h| h.agent_id == agent_id)
        .map(|h| h.child_session_id.clone())
        .expect("child");

    run_op(
        &mut state,
        &mut streams,
        &persistence,
        &key,
        Some("c-q1"),
        SubagentOp::SendInput {
            target: agent_id.clone(),
            items: vec![SubagentInputItem::Text {
                text: "queued after fail".into(),
            }],
            interrupt: false,
        },
    )
    .await
    .expect("queue");

    // 第 1 轮失败：生成失败报告，但暂停后续队列且清除恢复标记
    finish_subagent(
        &mut state,
        &mut streams,
        &persistence,
        RequestKey {
            pane: key.pane,
            session: child_session.clone(),
            generation: key.generation,
        },
        StopReason::Failed {
            code: "http 500".into(),
        },
        Usage::default(),
    );
    assert!(
        !streams.subagents.contains_key(&child_session),
        "子轮失败后必须暂停后续队列，不得自动启动下一条"
    );
    assert!(!streams.resumed_subagents.contains(&agent_id));
    let reports = persistence
        .database()
        .expect("db")
        .pending_reports(fixture.workspace(), &key.session)
        .expect("reports");
    assert_eq!(reports.len(), 1);
    assert_eq!(reports[0].status, "failed");

    // list_agents 显示 failed 且 queued_count = 1
    let listed = run_op(
        &mut state,
        &mut streams,
        &persistence,
        &key,
        Some("c-list"),
        SubagentOp::List,
    )
    .await
    .expect("list");
    if let SubagentOutcome::Listed { agents } = listed {
        assert_eq!(agents.len(), 1);
        assert_eq!(agents[0].status, "failed");
        assert_eq!(agents[0].queued_count, 1);
    }

    // 显式 resume_agent 后恢复并消费保留的队列项
    let resumed = run_op(
        &mut state,
        &mut streams,
        &persistence,
        &key,
        Some("c-resume"),
        SubagentOp::Resume {
            id: agent_id.clone(),
        },
    )
    .await
    .expect("resume");
    assert_eq!(
        resumed,
        SubagentOutcome::Resumed {
            agent_id,
            status: "running".into(),
            queued_count: 0,
        }
    );
    assert!(streams.subagents.contains_key(&child_session));
}

#[tokio::test]
async fn image_items_archive_preserve_interleaved_order_and_block_when_archive_missing() {
    let (fixture, mut state, key, persistence, mut streams, _parent) = setup_parent();
    // 写入一张本地图片到工作区
    let img_path = fixture.workspace().join("local_bug.png");
    std::fs::write(&img_path, make_test_png(20, 12, 7)).expect("write local png");
    let inline_png = make_test_png(16, 16, 42);
    let inline_b64 = base64::engine::general_purpose::STANDARD.encode(&inline_png);
    let data_url = format!("data:image/png;base64,{inline_b64}");

    let SubagentOutcome::Spawned { agent_id, .. } = run_op(
        &mut state,
        &mut streams,
        &persistence,
        &key,
        Some("c-spawn-img"),
        SubagentOp::Spawn {
            role: AgentRole::Explorer,
            title: "Visual inspect".into(),
            task: "First".into(),
            items: vec![
                SubagentInputItem::Text {
                    text: "First text".into(),
                },
                SubagentInputItem::LocalImage {
                    path: "local_bug.png".into(),
                },
                SubagentInputItem::Text {
                    text: "Second text".into(),
                },
            ],
        },
    )
    .await
    .expect("spawn with local image") else {
        panic!("expected spawned");
    };

    // 立刻删除工作区原图：后续回放必须走子会话归档副本，不依赖原路径
    std::fs::remove_file(&img_path).expect("remove source image");

    let child_session = streams
        .subagents
        .values()
        .find(|h| h.agent_id == agent_id)
        .map(|h| h.child_session_id.clone())
        .expect("child session");

    // 排入带内联 data URL 的第 2 轮输入
    let sent = run_op(
        &mut state,
        &mut streams,
        &persistence,
        &key,
        Some("c-send-inline"),
        SubagentOp::SendInput {
            target: agent_id.clone(),
            items: vec![
                SubagentInputItem::Text {
                    text: "Follow up before image".into(),
                },
                SubagentInputItem::Image {
                    image_url: data_url.clone(),
                },
                SubagentInputItem::Text {
                    text: "Follow up after image".into(),
                },
            ],
            interrupt: false,
        },
    )
    .await
    .expect("send inline image");
    let SubagentOutcome::InputAccepted { submission_id, .. } = sent else {
        panic!("expected input accepted");
    };

    // 验证工具结果与 DB 中不含 base64 原文
    let db = persistence.database().expect("db");
    let next_queued = db
        .next_queued_input(fixture.workspace(), &key.session, &agent_id)
        .expect("next")
        .expect("queued");
    assert_eq!(next_queued.items.len(), 3);
    let debug_items = format!("{:?}", next_queued.items);
    assert!(
        !debug_items.contains(&inline_b64),
        "DB 输入项只存归档路径与摘要，不存 base64 正文"
    );

    // 验证跨轮上下文装配保持 [text, image, text] 严格顺序（即使原图已被删）
    finish_subagent(
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
    assert!(
        streams.subagents.contains_key(&child_session),
        "原路径删除后归档图片仍应正常校验并启动第 2 轮"
    );

    // 测试归档损坏/丢失时阻断队列并投影为 blocked，且可用 cancel_agent_input 取消
    let sent_bad = run_op(
        &mut state,
        &mut streams,
        &persistence,
        &key,
        Some("c-send-missing"),
        SubagentOp::SendInput {
            target: agent_id.clone(),
            items: vec![
                SubagentInputItem::Text {
                    text: "Will lose archive".into(),
                },
                SubagentInputItem::Image {
                    image_url: data_url,
                },
            ],
            interrupt: false,
        },
    )
    .await
    .expect("send 3");
    let SubagentOutcome::InputAccepted {
        submission_id: bad_sub_id,
        ..
    } = sent_bad
    else {
        panic!("expected input accepted");
    };

    // 删除该排队项对应的归档文件（模拟归档被外部删掉）
    let queued_record = db
        .next_queued_input(fixture.workspace(), &key.session, &agent_id)
        .expect("next")
        .expect("queued");
    for item in &queued_record.items {
        if let crate::db::chat::PersistedInputItem::Image { archive_path, .. } = item {
            let _ = std::fs::remove_file(archive_path);
        }
    }

    // 第 2 轮完成：尝试消费 bad_sub_id 时检测到归档缺失，保持 queued + error_code=image_missing
    finish_subagent(
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
    assert!(
        !streams.subagents.contains_key(&child_session),
        "缺图任务必须暂停，不得降级为纯文本启动"
    );
    let head = db
        .next_queued_input(fixture.workspace(), &key.session, &agent_id)
        .expect("next")
        .expect("still queued");
    assert_eq!(head.submission_id, bad_sub_id);
    assert_eq!(head.error_code.as_deref(), Some(IMAGE_MISSING_CODE));

    // list_agents 将其投影为 blocked
    let listed = run_op(
        &mut state,
        &mut streams,
        &persistence,
        &key,
        Some("c-list-blocked"),
        SubagentOp::List,
    )
    .await
    .expect("list");
    if let SubagentOutcome::Listed { agents } = listed {
        assert_eq!(agents[0].status, "blocked");
        assert_eq!(agents[0].queued_count, 1);
    }

    // 主 agent 使用 cancel_agent_input 取消该 blocked 项
    let cancelled = run_op(
        &mut state,
        &mut streams,
        &persistence,
        &key,
        Some("c-cancel-blocked"),
        SubagentOp::CancelInput {
            submission_id: bad_sub_id,
        },
    )
    .await
    .expect("cancel blocked");
    if let SubagentOutcome::InputCancelled {
        status,
        queued_count,
        ..
    } = cancelled
    {
        assert_eq!(status, "cancelled");
        assert_eq!(queued_count, 0);
    }
    let _ = submission_id;
    crate::platform::remove_session_images(&child_session);
}

#[tokio::test]
async fn auto_compact_persists_checkpoint_and_replays_in_subsequent_turn() {
    let (fixture, mut state, key, persistence, mut streams, _parent) = setup_parent();
    let SubagentOutcome::Spawned { agent_id, .. } = run_op(
        &mut state,
        &mut streams,
        &persistence,
        &key,
        Some("c-spawn-compact"),
        SubagentOp::Spawn {
            role: AgentRole::Explorer,
            title: "Compact child".into(),
            task: "turn 1 task".into(),
            items: Vec::new(),
        },
    )
    .await
    .expect("spawn") else {
        panic!("expected spawned");
    };
    let child_session = streams
        .subagents
        .values()
        .find(|h| h.agent_id == agent_id)
        .map(|h| h.child_session_id.clone())
        .expect("child");

    let child_key = RequestKey {
        pane: key.pane,
        session: child_session.clone(),
        generation: key.generation,
    };
    let notice = AutoCompactNotice {
        model_label: "m".into(),
        summary: "compacted child summary".into(),
        estimated_tokens: 1000,
        context_limit: 2000,
    };
    let (reply, rx) = tokio::sync::oneshot::channel();
    handle_subagent_auto_compact(
        &mut state,
        &streams,
        &persistence,
        &child_key,
        &notice,
        Some(reply),
    );
    assert!(rx.await.expect("reply").is_ok());

    let db = persistence.database().expect("db");
    let cp = db
        .active_checkpoint(fixture.workspace(), &child_session)
        .expect("query cp")
        .expect("checkpoint persisted");
    assert_eq!(cp.summary_text, "compacted child summary");

    // 结束第 1 轮（已被 checkpoint 覆盖），再跑完第 2 轮（checkpoint 之后的历史），最后装配第 3 轮
    finish_subagent(
        &mut state,
        &mut streams,
        &persistence,
        child_key,
        StopReason::Complete,
        Usage::default(),
    );
    run_op(
        &mut state,
        &mut streams,
        &persistence,
        &key,
        Some("c-resume-after-cp"),
        SubagentOp::Resume {
            id: agent_id.clone(),
        },
    )
    .await
    .expect("resume");
    run_op(
        &mut state,
        &mut streams,
        &persistence,
        &key,
        Some("c-send-turn-2"),
        SubagentOp::SendInput {
            target: agent_id.clone(),
            items: vec![SubagentInputItem::Text {
                text: "post-checkpoint turn 2".into(),
            }],
            interrupt: false,
        },
    )
    .await
    .expect("send turn 2");
    finish_subagent(
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

    let model = crate::db::chat::ModelSnapshot {
        provider: "p".into(),
        model: "m".into(),
        variant: None,
    };
    let ctx = super::context::assemble_child_context(
        db,
        fixture.workspace(),
        &key.session,
        &agent_id,
        &child_session,
        "msg-next",
        AgentRole::Explorer,
        "<environment_context/>",
        None,
        &crate::mcp::McpToolSnapshot::default(),
        fixture.workspace(),
        &model,
        None,
        Some(200_000),
        Some(4096),
        true,
        &[crate::db::chat::PersistedInputItem::Text {
            text: "turn 3 question".into(),
        }],
    )
    .expect("assemble child context");
    assert_eq!(ctx.compact_summary(), Some("compacted child summary"));
    assert_eq!(
        ctx.items(),
        &[
            ModelItem::UserText("post-checkpoint turn 2".into()),
            ModelItem::AssistantText(String::new()),
            ModelItem::UserText("turn 3 question".into()),
        ]
    );
}

#[tokio::test]
async fn wait_agent_does_not_mistake_previous_completed_turn_for_current_running_turn() {
    let (_fixture, mut state, key, persistence, mut streams, _parent) = setup_parent();
    let SubagentOutcome::Spawned { agent_id, .. } = run_op(
        &mut state,
        &mut streams,
        &persistence,
        &key,
        Some("c-spawn"),
        SubagentOp::Spawn {
            role: AgentRole::Explorer,
            title: "Two turns".into(),
            task: "turn 1".into(),
            items: Vec::new(),
        },
    )
    .await
    .expect("spawn") else {
        panic!("expected spawned");
    };
    let child_session = streams
        .subagents
        .values()
        .find(|h| h.agent_id == agent_id)
        .map(|h| h.child_session_id.clone())
        .expect("child");

    // 第 1 轮完成并领取第 1 轮报告
    finish_subagent(
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
    let waited_1 = settle_wait(
        &mut streams,
        &persistence,
        &key,
        std::slice::from_ref(&agent_id),
        false,
        false,
        WaitMode::All,
    )
    .expect("settle 1");
    if let SubagentOutcome::Waited { agents, .. } = waited_1 {
        assert_eq!(agents[0].status, "completed");
        assert!(agents[0].report.is_some());
    }

    // resume 并发送第 2 轮输入（使该 agent 再次处于 running）
    run_op(
        &mut state,
        &mut streams,
        &persistence,
        &key,
        Some("c-resume"),
        SubagentOp::Resume {
            id: agent_id.clone(),
        },
    )
    .await
    .expect("resume");
    run_op(
        &mut state,
        &mut streams,
        &persistence,
        &key,
        Some("c-send-2"),
        SubagentOp::SendInput {
            target: agent_id.clone(),
            items: vec![SubagentInputItem::Text {
                text: "turn 2".into(),
            }],
            interrupt: false,
        },
    )
    .await
    .expect("send 2");

    // 第 2 轮仍在运行时，wait_for=any 不得因为历史曾完成而立即把该 agent 当成已完成返回
    let (reply, mut receiver) = tokio::sync::oneshot::channel();
    handle_subagent_op_with_call(
        &mut state,
        &mut streams,
        &persistence,
        key.clone(),
        Some("c-wait-2"),
        SubagentOp::Wait {
            targets: vec![agent_id.clone()],
            mode: WaitMode::Any,
            timeout_ms: 5_000,
        },
        reply,
    );
    assert!(
        receiver.try_recv().is_err(),
        "第 2 轮仍在运行时，wait_agent 必须等待当前活动轮而非立即返回上一轮完成"
    );
}

#[tokio::test]
async fn p0_wait_any_does_not_wake_early_between_queued_turns_and_p1_cancel_unblocks_next_item() {
    let (_fixture, mut state, key, persistence, mut streams, _parent) = setup_parent();
    let SubagentOutcome::Spawned { agent_id, .. } = run_op(
        &mut state,
        &mut streams,
        &persistence,
        &key,
        Some("c-spawn-p0"),
        SubagentOp::Spawn {
            role: AgentRole::Explorer,
            title: "Queue transition".into(),
            task: "turn 1".into(),
            items: Vec::new(),
        },
    )
    .await
    .expect("spawn") else {
        panic!("expected spawned");
    };
    let child_session = streams
        .subagents
        .values()
        .find(|h| h.agent_id == agent_id)
        .map(|h| h.child_session_id.clone())
        .expect("child");

    // 排入第 2 轮输入
    run_op(
        &mut state,
        &mut streams,
        &persistence,
        &key,
        Some("c-send-q2"),
        SubagentOp::SendInput {
            target: agent_id.clone(),
            items: vec![SubagentInputItem::Text {
                text: "turn 2".into(),
            }],
            interrupt: false,
        },
    )
    .await
    .expect("queue turn 2");

    // 获取第 1 轮句柄上的 done receiver（模拟 wait_for=any 正在订阅）
    let done_rx = streams
        .subagents
        .get(&child_session)
        .expect("handle 1")
        .done
        .subscribe();

    // 第 1 轮完成并立即接续启动第 2 轮（continued == true）：done 通道必须复用且不触发完成信号
    finish_subagent(
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
    assert!(
        !*done_rx.borrow(),
        "第 1 轮接续启动第 2 轮时，done 不得提前置为 true"
    );
    assert!(
        done_rx.has_changed().is_ok(),
        "第 1 轮切换第 2 轮时，done 通道必须保持存活不关闭"
    );

    // 第 2 轮完成且队列已空（!continued）：此时 done 才置为 true
    finish_subagent(
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
    assert!(*done_rx.borrow(), "队列全部消费完毕后 done 置为 true");
}

#[tokio::test]
async fn p1_send_input_to_idle_agent_at_concurrency_limit_does_not_leave_orphan_row() {
    let (fixture, mut state, key, persistence, mut streams, _parent) = setup_parent();
    // 先创建 1 个子 agent 并让其完成、再 resume 到 idle
    let SubagentOutcome::Spawned {
        agent_id: idle_agent,
        ..
    } = run_op(
        &mut state,
        &mut streams,
        &persistence,
        &key,
        Some("c-idle-spawn"),
        SubagentOp::Spawn {
            role: AgentRole::Explorer,
            title: "Idle target".into(),
            task: "turn 1".into(),
            items: Vec::new(),
        },
    )
    .await
    .expect("spawn idle")
    else {
        panic!("expected spawned");
    };
    let idle_child = streams
        .subagents
        .values()
        .find(|h| h.agent_id == idle_agent)
        .map(|h| h.child_session_id.clone())
        .expect("child");
    finish_subagent(
        &mut state,
        &mut streams,
        &persistence,
        RequestKey {
            pane: key.pane,
            session: idle_child,
            generation: key.generation,
        },
        StopReason::Complete,
        Usage::default(),
    );
    run_op(
        &mut state,
        &mut streams,
        &persistence,
        &key,
        Some("c-idle-resume"),
        SubagentOp::Resume {
            id: idle_agent.clone(),
        },
    )
    .await
    .expect("resume idle");

    // 再启动 3 个并发运行中的子 agent 占满 MAX_CONCURRENT_SUBAGENTS
    for idx in 0..crate::chat::agent::MAX_CONCURRENT_SUBAGENTS {
        run_op(
            &mut state,
            &mut streams,
            &persistence,
            &key,
            Some(&format!("c-busy-{idx}")),
            SubagentOp::Spawn {
                role: AgentRole::Explorer,
                title: format!("Busy {idx}"),
                task: "work".into(),
                items: Vec::new(),
            },
        )
        .await
        .expect("spawn busy");
    }

    // 此时向 idle_agent 发送 send_input：必须返回 LIMIT_REACHED_CODE，且不留孤儿 queued 记录
    let err = run_op(
        &mut state,
        &mut streams,
        &persistence,
        &key,
        Some("c-send-over-limit"),
        SubagentOp::SendInput {
            target: idle_agent.clone(),
            items: vec![SubagentInputItem::Text {
                text: "should not persist".into(),
            }],
            interrupt: false,
        },
    )
    .await
    .expect_err("must reject when concurrency full");
    assert_eq!(
        err,
        HostError::Failed {
            code: crate::chat::agent::LIMIT_REACHED_CODE.to_string()
        }
    );
    let next = persistence
        .database()
        .expect("db")
        .next_queued_input(fixture.workspace(), &key.session, &idle_agent)
        .expect("query next");
    assert!(
        next.is_none(),
        "并发受阻报错时不得在数据库遗留孤儿 queued 项"
    );
}
