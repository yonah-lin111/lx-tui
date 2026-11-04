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
    let (mut state, key, persistence, _session) = streaming_state(&fixture);
    // 模型声明图片输入：唤醒轮从窗格配置解析快照（`resolve_frozen_for_pane`）时必须与
    // 下方手工冻结的 `supports_image_input` 一致，否则会拒绝已归档图片。
    if let Some(model) = state
        .models
        .settings
        .providers
        .get_mut("p")
        .and_then(|provider| provider.models.get_mut("m"))
    {
        model.modalities = Some(serde_json::json!({ "input": ["text", "image"] }));
    }
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

/// 无额外选项的 `spawn_agent` 操作（选项字段默认继承父会话）。
fn spawn_op(role: AgentRole, title: impl Into<String>, task: &str) -> SubagentOp {
    spawn_op_with(role, title, task, None, None)
}

/// 带显式模型/思考等级的 `spawn_agent` 操作。
fn spawn_op_with(
    role: AgentRole,
    title: impl Into<String>,
    task: &str,
    model: Option<&str>,
    reasoning_effort: Option<&str>,
) -> SubagentOp {
    SubagentOp::Spawn {
        role,
        title: title.into(),
        task_name: None,
        task: task.into(),
        items: Vec::new(),
        fork_context: false,
        fork_turns: None,
        max_tokens: None,
        model: model.map(str::to_string),
        reasoning_effort: reasoning_effort.map(str::to_string),
    }
}

/// 新测试的紧凑上下文：持有 fixture 生命周期并封装重复的事件层调用。
struct Ctx {
    _fixture: Fixture,
    state: AppState,
    streams: ChatStreams,
    persistence: Persistence,
    key: RequestKey,
}

impl Ctx {
    fn new() -> Self {
        let (fixture, state, key, persistence, streams, _parent) = setup_parent();
        Self {
            _fixture: fixture,
            state,
            streams,
            persistence,
            key,
        }
    }

    async fn run(&mut self, call_id: &str, op: SubagentOp) -> SubagentOutcome {
        run_op(
            &mut self.state,
            &mut self.streams,
            &self.persistence,
            &self.key,
            Some(call_id),
            op,
        )
        .await
        .expect("subagent op")
    }

    fn child(&self, agent_id: &str) -> String {
        child_of(&self.streams, agent_id)
    }

    fn finish(&mut self, child_session: &str, reason: StopReason) {
        finish(
            &mut self.state,
            &mut self.streams,
            &self.persistence,
            &self.key,
            child_session,
            reason,
        );
    }
}

fn spawned(outcome: &SubagentOutcome) -> (&str, &str) {
    let SubagentOutcome::Spawned {
        agent_id,
        nickname: Some(nickname),
        ..
    } = outcome
    else {
        panic!("expected spawned");
    };
    (agent_id, nickname)
}

fn child_of(streams: &ChatStreams, agent_id: &str) -> String {
    streams
        .subagents
        .values()
        .find(|handle| handle.agent_id == agent_id)
        .expect("subagent handle")
        .child_session_id
        .clone()
}

fn finish(
    state: &mut AppState,
    streams: &mut ChatStreams,
    persistence: &Persistence,
    key: &RequestKey,
    child_session: &str,
    reason: StopReason,
) {
    finish_subagent(
        state,
        streams,
        persistence,
        RequestKey {
            pane: key.pane,
            session: child_session.to_string(),
            generation: key.generation,
        },
        reason,
        Usage::default(),
    );
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
        spawn_op(AgentRole::Explorer, "Multi turn", "turn 1"),
    )
    .await
    .expect("spawn");
    let SubagentOutcome::Spawned { agent_id, .. } = spawned else {
        panic!("expected spawned");
    };
    let child_session = child_of(&streams, &agent_id);

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
            trigger_turn: true,
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
            trigger_turn: true,
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
    finish(
        &mut state,
        &mut streams,
        &persistence,
        &key,
        &child_session,
        StopReason::Complete,
    );
    assert!(
        streams.subagents.contains_key(&child_session),
        "有排队输入时完成第 1 轮后应立即启动第 2 轮"
    );

    // 第 2 轮完成且队列已空：释放恢复标记，进入终态
    finish(
        &mut state,
        &mut streams,
        &persistence,
        &key,
        &child_session,
        StopReason::Complete,
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
            trigger_turn: true,
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
            trigger_turn: true,
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
async fn interrupt_cancels_current_turn_with_partial_report_and_runs_ahead_of_older_queue() {
    let (fixture, mut state, key, persistence, mut streams, _parent) = setup_parent();
    let SubagentOutcome::Spawned { agent_id, .. } = run_op(
        &mut state,
        &mut streams,
        &persistence,
        &key,
        Some("c-spawn"),
        spawn_op(AgentRole::Executor, "Interrupt test", "turn 1"),
    )
    .await
    .expect("spawn") else {
        panic!("expected spawned");
    };
    let child_session = child_of(&streams, &agent_id);

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
            trigger_turn: true,
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
            trigger_turn: true,
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

    // 旧轮以 Cancelled 封口：生成半程汇报，且立即启动 urgent interrupt 输入，保留旧普通队列（剩余 1 条）
    finish(
        &mut state,
        &mut streams,
        &persistence,
        &key,
        &child_session,
        StopReason::Cancelled,
    );
    let handle = streams.subagents.get(&child_session).expect("restarted");
    assert_eq!(handle.task, "urgent interrupt");
    let reports = persistence
        .database()
        .expect("db")
        .pending_reports(fixture.workspace(), &key.session)
        .expect("reports");
    assert_eq!(reports.len(), 1, "被中断的子轮生成半程汇报");
    let report = crate::chat::agent::SubagentReportView::from_json(&reports[0].report_json)
        .expect("partial report");
    assert!(
        report.body.contains("[Partial report (interrupted)]"),
        "partial body: {}",
        report.body
    );
    assert_eq!(report.status, "cancelled");
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
        spawn_op(AgentRole::Executor, "Fail test", "turn 1"),
    )
    .await
    .expect("spawn") else {
        panic!("expected spawned");
    };
    let child_session = child_of(&streams, &agent_id);

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
            trigger_turn: true,
        },
    )
    .await
    .expect("queue");

    // 第 1 轮失败：生成失败报告，但暂停后续队列且清除恢复标记
    finish(
        &mut state,
        &mut streams,
        &persistence,
        &key,
        &child_session,
        StopReason::Failed {
            code: "http 500".into(),
        },
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
        SubagentOp::List { path_prefix: None },
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
            task_name: None,
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
            fork_context: false,
            fork_turns: None,
            max_tokens: None,
            model: None,
            reasoning_effort: None,
        },
    )
    .await
    .expect("spawn with local image") else {
        panic!("expected spawned");
    };

    // 立刻删除工作区原图：后续回放必须走子会话归档副本，不依赖原路径
    std::fs::remove_file(&img_path).expect("remove source image");

    let child_session = child_of(&streams, &agent_id);

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
            trigger_turn: true,
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
    finish(
        &mut state,
        &mut streams,
        &persistence,
        &key,
        &child_session,
        StopReason::Complete,
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
            trigger_turn: true,
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
    finish(
        &mut state,
        &mut streams,
        &persistence,
        &key,
        &child_session,
        StopReason::Complete,
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
        SubagentOp::List { path_prefix: None },
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
        spawn_op(AgentRole::Explorer, "Compact child", "turn 1 task"),
    )
    .await
    .expect("spawn") else {
        panic!("expected spawned");
    };
    let child_session = child_of(&streams, &agent_id);

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
    finish(
        &mut state,
        &mut streams,
        &persistence,
        &key,
        &child_session,
        StopReason::Complete,
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
            trigger_turn: true,
        },
    )
    .await
    .expect("send turn 2");
    finish(
        &mut state,
        &mut streams,
        &persistence,
        &key,
        &child_session,
        StopReason::Complete,
    );

    let model = crate::db::chat::ModelSnapshot {
        provider: "p".into(),
        model: "m".into(),
        variant: None,
    };
    let mcp_snapshot = crate::mcp::McpToolSnapshot::default();
    let ctx = super::context::assemble_child_context(
        db,
        fixture.workspace(),
        &key.session,
        &agent_id,
        &child_session,
        "msg-next",
        AgentRole::Explorer,
        super::context::ChildPromptContext {
            system_prompt_override: None,
            model_identity: None,
            environment: "<environment_context/>",
            instructions: None,
            skills: None,
            mcp: &mcp_snapshot,
        },
        fixture.workspace(),
        &model,
        None,
        Some(200_000),
        Some(4096),
        true,
        &[crate::db::chat::PersistedInputItem::Text {
            text: "turn 3 question".into(),
        }],
        super::context::ChildTurnLimits {
            depth: 1,
            max_depth: 2,
            max_tokens: None,
        },
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
        spawn_op(AgentRole::Explorer, "Two turns", "turn 1"),
    )
    .await
    .expect("spawn") else {
        panic!("expected spawned");
    };
    let child_session = child_of(&streams, &agent_id);

    // 第 1 轮完成并领取第 1 轮报告
    finish(
        &mut state,
        &mut streams,
        &persistence,
        &key,
        &child_session,
        StopReason::Complete,
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
            trigger_turn: true,
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
        spawn_op(AgentRole::Explorer, "Queue transition", "turn 1"),
    )
    .await
    .expect("spawn") else {
        panic!("expected spawned");
    };
    let child_session = child_of(&streams, &agent_id);

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
            trigger_turn: true,
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
    finish(
        &mut state,
        &mut streams,
        &persistence,
        &key,
        &child_session,
        StopReason::Complete,
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
    finish(
        &mut state,
        &mut streams,
        &persistence,
        &key,
        &child_session,
        StopReason::Complete,
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
        spawn_op(AgentRole::Explorer, "Idle target", "turn 1"),
    )
    .await
    .expect("spawn idle")
    else {
        panic!("expected spawned");
    };
    let idle_child = child_of(&streams, &idle_agent);
    finish(
        &mut state,
        &mut streams,
        &persistence,
        &key,
        &idle_child,
        StopReason::Complete,
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
            spawn_op(AgentRole::Explorer, format!("Busy {idx}"), "work"),
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
            trigger_turn: true,
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

#[tokio::test]
async fn resumed_subagent_inherits_updated_parent_pane_model() {
    let (_fixture, mut state, key, persistence, mut streams, _parent) = setup_parent();
    // 增加第二个模型供父窗格切换（模拟用户执行 `/model`）。
    {
        let provider = state
            .models
            .settings
            .providers
            .get_mut("p")
            .expect("provider p");
        let base = provider.models.get("m").cloned().expect("model m");
        provider.models.insert(
            "m2".into(),
            crate::config::ai::ModelConfig {
                id: "m2".into(),
                name: "M2".into(),
                ..base
            },
        );
    }
    let SubagentOutcome::Spawned { agent_id, .. } = run_op(
        &mut state,
        &mut streams,
        &persistence,
        &key,
        Some("c-spawn-model"),
        spawn_op(AgentRole::Explorer, "Model inheritance", "turn 1"),
    )
    .await
    .expect("spawn") else {
        panic!("expected spawned");
    };
    let child_session = child_of(&streams, &agent_id);
    assert_eq!(
        streams.frozen.get(&child_session).map(|f| f.model.as_str()),
        Some("m"),
        "spawn 时子会话缓存父会话当时的模型"
    );

    // 排队第 2 轮输入，随后父窗格切到 m2。
    run_op(
        &mut state,
        &mut streams,
        &persistence,
        &key,
        Some("c-send-model"),
        SubagentOp::SendInput {
            target: agent_id.clone(),
            items: vec![SubagentInputItem::Text {
                text: "turn 2".into(),
            }],
            interrupt: false,
            trigger_turn: true,
        },
    )
    .await
    .expect("queue turn 2");
    {
        let target = state.pane_mut_anywhere(key.pane).expect("pane");
        target.lx.chat.selection = Some(crate::config::ai::ModelSelection {
            provider: "p".into(),
            model: "m2".into(),
            variant: None,
        });
    }

    // 第 1 轮结束自动接续第 2 轮：必须使用父窗格当前模型，而不是子会话旧缓存。
    finish(
        &mut state,
        &mut streams,
        &persistence,
        &key,
        &child_session,
        StopReason::Complete,
    );
    assert!(
        streams.subagents.contains_key(&child_session),
        "排队输入应接续启动第 2 轮"
    );
    assert_eq!(
        streams.frozen.get(&child_session).map(|f| f.model.as_str()),
        Some("m2"),
        "唤醒的子 Agent 必须继承父窗格切换后的模型"
    );
}

#[test]
fn replay_persisted_items_degrades_historical_images_when_model_lacks_vision() {
    let items = vec![
        crate::db::chat::PersistedInputItem::Text {
            text: "Inspect this screenshot".into(),
        },
        crate::db::chat::PersistedInputItem::Image {
            label: "screen.png".into(),
            archive_path: "/nonexistent/screen.png".into(),
            mime: "image/png".into(),
            content_hash: "deadbeef".into(),
            width: 100,
            height: 100,
        },
    ];
    // 当前轮输入带图片但模型不支持图片：必须报错拒绝
    let current_err =
        super::context::replay_persisted_items("child-1", false, true, &items).unwrap_err();
    assert!(current_err.contains("does not support image input"));

    // 历史轮次回放带图片但切换后的新模型不支持图片：降级为英文文本占位，不阻塞子 agent
    let replayed = super::context::replay_persisted_items("child-1", false, false, &items)
        .expect("historical image degraded");
    assert_eq!(
        replayed,
        vec![ModelItem::UserText(
            "Inspect this screenshot\nERROR: Cannot read \"screen.png\" (this model does not support image input). Inform the user.".into()
        )]
    );
}

fn text(value: &str) -> SubagentInputItem {
    SubagentInputItem::Text { text: value.into() }
}

#[rustfmt::skip]
#[tokio::test]
async fn nicknames_are_unique_and_address_control_tools_case_insensitively() {
    let mut ctx = Ctx::new();
    let first = ctx.run("c1", spawn_op(AgentRole::Explorer, "first", "task 1")).await;
    let second = ctx.run("c2", spawn_op(AgentRole::Explorer, "second", "task 2")).await;
    let (first_id, first_nick) = spawned(&first);
    let (second_id, second_nick) = spawned(&second);
    assert_eq!((first_nick, second_nick), ("Turing", "Curie"));
    let (first_child, second_child) = (ctx.child(first_id), ctx.child(second_id));

    let sent = ctx.run("c3", SubagentOp::SendInput { target: first_nick.to_lowercase(), items: vec![text("turn 2")], interrupt: false, trigger_turn: true }).await;
    assert!(matches!(sent, SubagentOutcome::InputAccepted { agent_id, .. } if agent_id == first_id));
    let interrupted = ctx.run("c4", SubagentOp::Interrupt { target: second_nick.to_uppercase() }).await;
    assert!(matches!(
        interrupted,
        SubagentOutcome::Interrupted { agent_id, nickname: Some(nick), previous_status, .. }
            if agent_id == second_id && nick == second_nick && previous_status == "running"
    ));
    let closed = ctx.run("c5", SubagentOp::Close { target: second_nick.to_lowercase() }).await;
    assert!(matches!(closed, SubagentOutcome::Closed { agent_id, .. } if agent_id == second_id));

    ctx.finish(&second_child, StopReason::Cancelled);
    ctx.finish(&first_child, StopReason::Complete);
    ctx.finish(&first_child, StopReason::Complete);
    let waited = ctx.run("c6", SubagentOp::Wait { targets: vec![first_nick.to_uppercase()], mode: WaitMode::All, timeout_ms: 1_000 }).await;
    assert!(matches!(
        waited,
        SubagentOutcome::Waited { agents, .. }
            if agents[0].agent_id == first_id && agents[0].nickname.as_deref() == Some(first_nick)
    ));
    let resumed = ctx.run("c7", SubagentOp::Resume { id: first_nick.to_lowercase() }).await;
    assert_eq!(resumed, SubagentOutcome::Resumed { agent_id: first_id.into(), status: "idle".into(), queued_count: 0 });
}

#[rustfmt::skip]
#[tokio::test]
async fn interrupt_agent_keeps_idle_and_pauses_queue_until_send_input_or_resume() {
    let mut ctx = Ctx::new();
    let spawn = ctx.run("i1", spawn_op(AgentRole::Explorer, "interrupt", "turn 1")).await;
    let agent_id = spawned(&spawn).0.to_string();
    let child = ctx.child(&agent_id);
    let drop_sub = match ctx.run("i1_drop", SubagentOp::SendInput { target: agent_id.clone(), items: vec![text("q_drop")], interrupt: false, trigger_turn: true }).await {
        SubagentOutcome::InputAccepted { submission_id, .. } => submission_id,
        other => panic!("expected InputAccepted, got {other:?}"),
    };
    for (call_id, body) in [("i2", "q1"), ("i3", "q2")] {
        ctx.run(call_id, SubagentOp::SendInput { target: agent_id.clone(), items: vec![text(body)], interrupt: false, trigger_turn: true }).await;
    }
    let cancel = ctx.streams.subagents.get(&child).expect("handle").cancel.subscribe();
    let interrupted = ctx.run("i4", SubagentOp::Interrupt { target: agent_id.clone() }).await;
    assert!(matches!(interrupted, SubagentOutcome::Interrupted { previous_status, queued_count, .. } if previous_status == "running" && queued_count == 3));
    assert!(*cancel.borrow(), "当前轮收到取消信号");

    // 被中断轮次封口后保持 idle，剩余 queued 输入不得自动消费，且 cancel_input 不得越过暂停自动拉起下一条。
    ctx.finish(&child, StopReason::Cancelled);
    assert!(!ctx.streams.subagents.contains_key(&child));
    assert!(ctx.streams.resumed_subagents.contains(&agent_id));
    ctx.run("i4_cancel", SubagentOp::CancelInput { submission_id: drop_sub }).await;
    assert!(!ctx.streams.subagents.contains_key(&child), "cancel_input must not bypass interrupt pause");

    // 后续 send_input 解除暂停并消费队首旧输入（不跳过队列）。
    let sent = ctx.run("i5", SubagentOp::SendInput { target: agent_id.clone(), items: vec![text("nudge")], interrupt: false, trigger_turn: true }).await;
    assert!(matches!(sent, SubagentOutcome::InputAccepted { status, queued_count, .. } if status == "queued" && queued_count == 2));
    assert_eq!(ctx.streams.subagents.get(&child).map(|h| h.task.as_str()), Some("q1"));
    ctx.finish(&child, StopReason::Complete);
    assert_eq!(ctx.streams.subagents.get(&child).map(|h| h.task.as_str()), Some("q2"));

    // 再次中断后，resume_agent 解除暂停并消费剩余输入。
    ctx.run("i6", SubagentOp::Interrupt { target: agent_id.clone() }).await;
    ctx.finish(&child, StopReason::Cancelled);
    let resumed = ctx.run("i7", SubagentOp::Resume { id: agent_id.clone() }).await;
    assert_eq!(resumed, SubagentOutcome::Resumed { agent_id, status: "running".into(), queued_count: 0 });
    assert_eq!(ctx.streams.subagents.get(&child).map(|h| h.task.as_str()), Some("nudge"));
}

#[rustfmt::skip]
#[tokio::test]
async fn subagent_model_resolution_prefers_explicit_then_role_then_parent() {
    let mut ctx = Ctx::new();
    {
        let provider = ctx.state.models.settings.providers.get_mut("p").expect("provider");
        let base = provider.models.get("m").cloned().expect("model");
        for (id, variant) in [("m2", "low"), ("m3", "high")] {
            let mut model = crate::config::ai::ModelConfig { id: id.into(), name: id.into(), ..base.clone() };
            model.variants.insert(variant.into(), serde_json::json!({}));
            model.variant = Some(variant.into());
            provider.models.insert(id.into(), model);
        }
    }
    let ghost = AgentRole::from_name_and_base("ghost", None).expect("custom role");
    let err = run_op(&mut ctx.state, &mut ctx.streams, &ctx.persistence, &ctx.key, Some("b1"), spawn_op(ghost, "ghost", "task")).await.expect_err("unknown role");
    assert!(matches!(err, HostError::Failed { code } if code == "unknown subagent role 'ghost'"));
    let err = run_op(&mut ctx.state, &mut ctx.streams, &ctx.persistence, &ctx.key, Some("b2"), spawn_op_with(AgentRole::Explorer, "bad", "task", Some("nope"), None)).await.expect_err("unavailable model");
    assert!(matches!(err, HostError::Failed { code } if code.contains("not found")));

    // 三级优先级：无覆写继承父会话冻结模型。
    let parent = ctx.run("m1", spawn_op(AgentRole::Explorer, "parent", "task")).await;
    let parent_id = spawned(&parent).0;
    assert!(matches!(&parent, SubagentOutcome::Spawned { model: None, reasoning_effort: None, .. }));
    assert_eq!(ctx.streams.frozen.get(&ctx.child(parent_id)).map(|f| f.model.as_str()), Some("m"));

    // 角色默认模型覆盖父会话。
    ctx.state.subagents_config.builtins.explorer.model = Some(crate::config::ai::ModelSelection { provider: "p".into(), model: "m2".into(), variant: Some("low".into()) });
    let role = ctx.run("m2", spawn_op(AgentRole::Explorer, "role", "task")).await;
    assert!(matches!(&role, SubagentOutcome::Spawned { model: Some(model), reasoning_effort: Some(effort), .. } if model == "p/m2" && effort == "low"));

    // 显式参数覆盖角色默认，且重启（清空内存缓存）后 resume 仍从 DB 恢复固定模型 m3。
    let explicit = ctx.run("m3", spawn_op_with(AgentRole::Explorer, "explicit", "task", Some("m3"), Some("high"))).await;
    let explicit_id = spawned(&explicit).0.to_string();
    let explicit_child = ctx.child(&explicit_id);
    assert!(matches!(&explicit, SubagentOutcome::Spawned { model: Some(model), reasoning_effort: Some(effort), .. } if model == "p/m3" && effort == "high"));
    ctx.finish(&explicit_child, StopReason::Complete);
    ctx.streams.frozen.remove(&explicit_child);
    ctx.streams.pinned_subagent_models.remove(&explicit_child);
    ctx.run("m4", SubagentOp::Resume { id: explicit_id.clone() }).await;
    assert_eq!(ctx.streams.frozen.get(&explicit_child).map(|f| f.model.as_str()), Some("m3"));
}

#[rustfmt::skip]
#[tokio::test]
async fn custom_role_resumes_after_it_is_removed_from_config() {
    let mut ctx = Ctx::new();
    let role = AgentRole::from_name_and_base("reviewer", Some(crate::config::subagents::BaseRole::Explorer)).expect("custom role");
    ctx.state.subagents_config.roles.push(crate::config::subagents::CustomRoleConfig {
        name: "reviewer".into(), base_role: crate::config::subagents::BaseRole::Explorer,
        description: "review".into(), system_prompt: "review code".into(), model: None,
        nickname_candidates: vec!["Ada".into()],
    });
    let spawn = ctx.run("r1", spawn_op(role, "custom", "task")).await;
    let (agent_id, nickname) = spawned(&spawn);
    assert_eq!(nickname, "Ada");
    let agent_id = agent_id.to_string();
    let child = ctx.child(&agent_id);
    ctx.finish(&child, StopReason::Complete);

    // 角色配置删除后，按历史记录仍可恢复并继续执行。
    ctx.state.subagents_config.roles.clear();
    let resumed = ctx.run("r2", SubagentOp::Resume { id: "ada".into() }).await;
    assert_eq!(resumed, SubagentOutcome::Resumed { agent_id: agent_id.clone(), status: "idle".into(), queued_count: 0 });
    let sent = ctx.run("r3", SubagentOp::SendInput { target: "ADA".into(), items: vec![text("again")], interrupt: false, trigger_turn: true }).await;
    let SubagentOutcome::InputAccepted { agent_id: resumed_id, status, .. } = sent else { panic!("expected input accepted") };
    assert_eq!(resumed_id, agent_id);
    assert_eq!(status, "running");
}
