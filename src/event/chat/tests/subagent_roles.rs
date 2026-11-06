//! 子 agent 角色能力（模型锁定、monitor 默认档位、能力收紧）、`<subagents>` 快照、
//! `<turn_aborted>` 中断回放与面板内联追问的事件层测试。

use super::subagent::{fixture_with_parent, op, spawn_explorer};
use super::*;
use crate::app::effects::Effect;
use crate::chat::agent::{
    AgentRole, BaseRole, SUBAGENT_TURN_ABORTED_MARKER, SubagentOp, SubagentOutcome, ToolSurface,
};
use crate::config::subagents::{CustomRoleConfig, RoleCapabilities};
use crate::layout::PaneId;

// ===== T3: 模型锁定 / monitor 默认档位 / 能力收紧 =====

fn add_model(state: &mut AppState, id: &str, variants: &[&str]) {
    let provider = state
        .models
        .settings
        .providers
        .get_mut("p")
        .expect("provider p");
    let mut config = provider.models.get("m").expect("model m").clone();
    config.id = id.to_string();
    config.name = id.to_string();
    config.variants = variants
        .iter()
        .map(|variant| ((*variant).to_string(), serde_json::json!({})))
        .collect();
    provider.models.insert(id.to_string(), config);
}

fn add_variant(state: &mut AppState, model: &str, variant: &str) {
    if let Some(config) = state
        .models
        .settings
        .providers
        .get_mut("p")
        .and_then(|provider| provider.models.get_mut(model))
    {
        config
            .variants
            .insert(variant.to_string(), serde_json::json!({}));
    }
}

fn custom_role(name: &str, lock_model: bool) -> CustomRoleConfig {
    CustomRoleConfig {
        name: name.to_string(),
        base_role: BaseRole::Explorer,
        description: format!("{name} role"),
        system_prompt: format!("{name} role prompt"),
        model: Some(crate::config::ai::ModelSelection {
            provider: "p".into(),
            model: "m".into(),
            variant: None,
        }),
        lock_model,
        ..Default::default()
    }
}

fn role_id(name: &str) -> AgentRole {
    AgentRole::parse(name).expect("valid role")
}

fn spawned_id(outcome: &SubagentOutcome) -> String {
    match outcome {
        SubagentOutcome::Spawned { agent_id, .. } => agent_id.clone(),
        other => panic!("expected spawned, got {other:?}"),
    }
}

async fn spawn_with(
    state: &mut AppState,
    streams: &mut ChatStreams,
    persistence: &Persistence,
    key: &RequestKey,
    role: AgentRole,
    task: &str,
    model: Option<&str>,
    effort: Option<&str>,
) -> SubagentOutcome {
    op(
        state,
        streams,
        persistence,
        key,
        SubagentOp::Spawn {
            role,
            title: task.into(),
            task_name: None,
            task: task.into(),
            items: Vec::new(),
            fork_context: false,
            fork_turns: None,
            max_tokens: None,
            model: model.map(str::to_string),
            reasoning_effort: effort.map(str::to_string),
        },
    )
    .await
    .expect("spawn")
}

#[tokio::test]
async fn lock_model_ignores_spawn_overrides_while_unlocked_role_allows_them() {
    let (fixture, mut state, key, persistence, mut streams, _parent) = fixture_with_parent();
    add_model(&mut state, "alt", &["high"]);
    state
        .subagents_config
        .roles
        .push(custom_role("locked", true));
    state
        .subagents_config
        .roles
        .push(custom_role("open", false));

    let locked = spawn_with(
        &mut state,
        &mut streams,
        &persistence,
        &key,
        role_id("locked"),
        "locked task",
        Some("p/alt"),
        Some("high"),
    )
    .await;
    let locked_id = spawned_id(&locked);
    let record = persistence
        .database()
        .expect("db")
        .owned_subagent(fixture.workspace(), &key.session, &locked_id)
        .expect("query")
        .expect("record");
    assert_eq!(record.model_id, "m", "locked role ignores model override");
    assert_eq!(
        record.model_variant, None,
        "locked role ignores effort override"
    );

    let open = spawn_with(
        &mut state,
        &mut streams,
        &persistence,
        &key,
        role_id("open"),
        "open task",
        Some("p/alt"),
        Some("high"),
    )
    .await;
    let open_id = spawned_id(&open);
    let record = persistence
        .database()
        .expect("db")
        .owned_subagent(fixture.workspace(), &key.session, &open_id)
        .expect("query")
        .expect("record");
    assert_eq!(
        record.model_id, "alt",
        "unlocked role accepts model override"
    );
    assert_eq!(
        record.model_variant.as_deref(),
        Some("high"),
        "unlocked role accepts effort override"
    );
}

#[tokio::test]
async fn monitor_only_defaults_to_low_when_the_model_supports_it() {
    let (fixture, mut state, key, persistence, mut streams, _parent) = fixture_with_parent();
    let first = spawn_with(
        &mut state,
        &mut streams,
        &persistence,
        &key,
        AgentRole::Monitor,
        "watch build",
        None,
        None,
    )
    .await;
    let first_id = spawned_id(&first);
    let record = persistence
        .database()
        .expect("db")
        .owned_subagent(fixture.workspace(), &key.session, &first_id)
        .expect("query")
        .expect("record");
    assert_eq!(record.model_variant, None, "no low variant -> keep default");

    add_variant(&mut state, "m", "low");
    let second = spawn_with(
        &mut state,
        &mut streams,
        &persistence,
        &key,
        AgentRole::Monitor,
        "watch tests",
        None,
        None,
    )
    .await;
    let second_id = spawned_id(&second);
    let record = persistence
        .database()
        .expect("db")
        .owned_subagent(fixture.workspace(), &key.session, &second_id)
        .expect("query")
        .expect("record");
    assert_eq!(
        record.model_variant.as_deref(),
        Some("low"),
        "monitor defaults to low when supported"
    );
}

#[tokio::test]
async fn role_capabilities_strip_skills_mcp_and_web_search() {
    let (fixture, mut state, key, persistence, _streams, _parent) = fixture_with_parent();
    let capabilities = RoleCapabilities {
        allow_web_search: false,
        allow_mcp: false,
        allow_skills: false,
    };
    state.subagents_config.roles.push(CustomRoleConfig {
        name: "narrow".into(),
        base_role: BaseRole::Explorer,
        description: "narrow".into(),
        system_prompt: "narrow role prompt".into(),
        capabilities,
        ..Default::default()
    });
    let frozen = crate::app::state::FrozenModel::resolve(
        &state.models,
        &crate::config::ai::ModelSelection {
            provider: "p".into(),
            model: "m".into(),
            variant: None,
        },
    )
    .expect("frozen");
    let inputs = crate::event::chat::subagent::capture_child_prompt_inputs(
        &state,
        &persistence,
        key.pane,
        fixture.workspace(),
        fixture.workspace(),
        &frozen,
        role_id("narrow"),
    );
    assert!(inputs.skills.is_none(), "skills must be stripped");
    assert!(inputs.mcp.tools().is_empty(), "mcp snapshot must be empty");
    let names = crate::event::chat::subagent::child_tool_names(
        ToolSurface::Explorer,
        capabilities,
        &inputs.mcp,
        &state.subagents_config,
    );
    assert!(!names.iter().any(|name| name == "web_search"));
    assert!(names.iter().any(|name| name == "read"));
}

// ===== T3: <subagents> 环境快照 =====

fn snapshot_handle(
    parent: &str,
    id: &str,
    status: &'static str,
    started_at_ms: i64,
) -> crate::event::chat::subagent::SubagentHandle {
    let (cancel, _) = tokio::sync::watch::channel(false);
    let (done, _) = tokio::sync::watch::channel(false);
    crate::event::chat::subagent::SubagentHandle {
        parent: RequestKey {
            pane: PaneId::alloc(),
            session: parent.to_string(),
            generation: 0,
        },
        agent_id: id.to_string(),
        nickname: Some("Turing".into()),
        agent_path: format!("/root/{id}"),
        root_session: parent.to_string(),
        depth: 1,
        child_session_id: format!("child-{id}"),
        child_assistant_id: String::new(),
        child_user_message_id: String::new(),
        role: AgentRole::Explorer,
        title: "Auth flow lookup".into(),
        task: "Auth flow lookup".into(),
        cancel,
        done,
        status,
        started_at_ms,
        max_tokens: None,
        closed_without_report: false,
        chat: crate::app::state::LxChat::new(),
        workspace: std::path::PathBuf::new(),
    }
}

#[test]
fn known_subagent_lines_orders_statuses_filters_cancelled_and_caps_entries() {
    let mut streams = ChatStreams::default();
    let statuses = [
        ("running", "running"),
        ("blocked", "blocked"),
        ("idle", "idle"),
        ("interrupted", "interrupted"),
        ("completed-a", "completed"),
        ("completed-b", "completed"),
        ("completed-c", "completed"),
        ("completed-d", "completed"),
        ("completed-e", "completed"),
        ("completed-f", "completed"),
        ("cancelled", "cancelled"),
    ];
    for (index, (id, status)) in statuses.iter().enumerate() {
        streams.subagents.insert(
            format!("child-{id}"),
            snapshot_handle("parent", id, status, index as i64),
        );
    }
    let lines = crate::event::chat::subagent::known_subagent_lines(&streams, None, None, "parent");
    assert_eq!(lines.first().map(String::as_str), Some("<subagents>"));
    assert_eq!(lines.last().map(String::as_str), Some("</subagents>"));
    let body = &lines[1..lines.len() - 1];
    assert!(body.len() <= 8, "snapshot must be capped: {body:?}");
    assert!(lines.join("\n").len() <= 1024);
    assert!(!lines.iter().any(|line| line.contains("cancelled")));
    let position = |status: &str| {
        lines
            .iter()
            .position(|line| line.contains(&format!("status={status}")))
            .unwrap_or_else(|| panic!("missing {status}: {lines:?}"))
    };
    assert!(position("running") < position("blocked"));
    assert!(position("blocked") < position("idle"));
    assert!(position("idle") < position("interrupted"));
    assert!(position("interrupted") < position("completed"));
}

#[tokio::test]
async fn known_subagent_lines_include_persisted_records() {
    let (fixture, mut state, key, persistence, mut streams, _parent) = fixture_with_parent();
    let agent_id =
        spawn_explorer(&mut state, &mut streams, &persistence, &key, "db snapshot").await;
    let lines = crate::event::chat::subagent::known_subagent_lines(
        &streams,
        persistence.database(),
        Some(fixture.workspace()),
        &key.session,
    );
    assert!(
        lines
            .iter()
            .any(|line| line.contains(&agent_id) && line.contains("status=running")),
        "db record missing from snapshot: {lines:?}"
    );
}

// ===== T3: <turn_aborted> 与后续 send_input 信封 =====

#[tokio::test]
async fn interrupted_turn_is_replayed_with_turn_aborted_and_followup_envelope() {
    let (fixture, mut state, key, persistence, mut streams, _parent) = fixture_with_parent();
    let agent_id =
        spawn_explorer(&mut state, &mut streams, &persistence, &key, "initial task").await;
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
        StopReason::Cancelled,
        Usage::default(),
    );

    let model = crate::db::chat::ModelSnapshot {
        provider: "p".into(),
        model: "m".into(),
        variant: None,
    };
    let mcp = crate::mcp::McpToolSnapshot::default();
    let current = [crate::db::chat::PersistedInputItem::Text {
        text: "follow-up question".into(),
    }];
    let ctx = crate::event::chat::subagent::assemble_child_context(
        persistence.database().expect("db"),
        fixture.workspace(),
        &key.session,
        &agent_id,
        &child_session,
        "",
        AgentRole::Explorer,
        crate::event::chat::subagent::ChildPromptContext {
            system_prompt_override: None,
            model_identity: None,
            environment: "<environment_context/>",
            instructions: None,
            skills: None,
            mcp: &mcp,
        },
        fixture.workspace(),
        &model,
        None,
        Some(200_000),
        Some(4096),
        true,
        &current,
        &state.subagents_config,
        crate::event::chat::subagent::ChildTurnLimits {
            depth: 1,
            max_depth: 2,
            max_tokens: None,
        },
    )
    .expect("assemble child context");
    let items = ctx.items();
    assert!(
        !items
            .iter()
            .any(|item| matches!(item, ModelItem::AssistantText(s) if s.is_empty())),
        "aborted turn without text must not emit empty AssistantText block: {items:?}"
    );
    assert!(
        items
            .iter()
            .any(|item| matches!(item, ModelItem::UserText(text)
            if text == SUBAGENT_TURN_ABORTED_MARKER)),
        "interrupted turn must carry <turn_aborted>: {items:?}"
    );
    assert!(
        matches!(items.last(), Some(ModelItem::UserText(text))
            if text.starts_with("Message Type: NEW_TASK")
                && text.ends_with("follow-up question")),
        "follow-up turn must be wrapped in an envelope: {items:?}"
    );

    let messages = persistence
        .database()
        .expect("db")
        .load_chat_session(fixture.workspace(), &child_session)
        .expect("load")
        .expect("messages");
    assert!(
        !messages.iter().any(|message| message
            .parts
            .iter()
            .any(|part| part.content.contains("Message Type:"))),
        "DB message text must stay clean"
    );
}

// ===== T3: 面板追问自动恢复 =====

#[tokio::test]
async fn send_subagent_user_input_effect_resumes_completed_agent_and_starts_a_turn() {
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
    assert!(!streams.resumed_subagents.contains(&agent_id));

    state.effects.push(Effect::SendSubagentUserInput {
        pane: key.pane,
        session_id: key.session.clone(),
        agent_id: agent_id.clone(),
        text: "  ask again  ".into(),
    });
    let changed =
        crate::event::chat::apply_subagent_effects(&mut state, &mut streams, &persistence);
    assert!(changed);
    assert!(streams.resumed_subagents.contains(&agent_id));
    assert!(
        streams
            .subagents
            .values()
            .any(|handle| handle.agent_id == agent_id && handle.status == "running"),
        "panel follow-up must start a new child turn"
    );
    let toast = state
        .pane_anywhere(key.pane)
        .and_then(|pane| pane.lx.toast.as_ref())
        .expect("pane toast");
    assert!(
        toast.message.contains("Sent input to"),
        "toast: {}",
        toast.message
    );

    let messages = persistence
        .database()
        .expect("db")
        .load_chat_session(fixture.workspace(), &child_session)
        .expect("load")
        .expect("messages");
    let last_user = messages
        .iter()
        .rev()
        .find(|message| message.role == "user")
        .expect("user message");
    let text: String = last_user
        .parts
        .iter()
        .filter(|part| part.kind == "text")
        .map(|part| part.content.as_str())
        .collect();
    assert_eq!(text, "ask again", "DB keeps the clean user text");
}

#[tokio::test]
async fn trigger_turn_false_stays_idle_after_running_turn_and_uses_message_envelope_on_wake() {
    let (fixture, mut state, key, persistence, mut streams, _parent) = fixture_with_parent();
    let spawned = op(
        &mut state,
        &mut streams,
        &persistence,
        &key,
        SubagentOp::Spawn {
            role: AgentRole::Monitor,
            title: "poller".into(),
            task_name: Some("poller".into()),
            task: "initial watch".into(),
            items: vec![crate::chat::agent::SubagentInputItem::Text {
                text: "initial watch".into(),
            }],
            fork_context: false,
            fork_turns: None,
            model: None,
            reasoning_effort: None,
            max_tokens: None,
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
        .find(|handle| handle.agent_id == agent_id)
        .expect("handle")
        .child_session_id
        .clone();

    // 1. 运行期间调用 send_input(trigger_turn=false) 仅排队补充上下文。
    op(
        &mut state,
        &mut streams,
        &persistence,
        &key,
        SubagentOp::SendInput {
            target: agent_id.clone(),
            items: vec![crate::chat::agent::SubagentInputItem::Text {
                text: "context note alpha".into(),
            }],
            interrupt: false,
            trigger_turn: false,
        },
    )
    .await
    .expect("queue note");

    // 2. 首轮自然完成后不得自动消费仅入队的 `trigger_turn=false` 消息，必须保持 idle + queued=1。
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
    assert!(
        !streams.subagents.contains_key(&child_session),
        "trigger_turn=false must not auto-start a turn after Complete"
    );
    assert!(
        streams.resumed_subagents.contains(&agent_id),
        "subagent with queued note stays in idle resumed state"
    );

    // 3. 显式调用 resume_agent 唤醒后，认领该条消息并包裹为 `Message Type: MESSAGE` 信封。
    op(
        &mut state,
        &mut streams,
        &persistence,
        &key,
        SubagentOp::Resume {
            id: agent_id.clone(),
        },
    )
    .await
    .expect("resume");
    let handle = streams.subagents.get(&child_session).expect("running turn");
    let current_user_id = handle.child_user_message_id.clone();

    let frozen = streams.frozen.get(&child_session).expect("frozen").clone();
    let inputs = crate::event::chat::subagent::capture_child_prompt_inputs(
        &state,
        &persistence,
        key.pane,
        fixture.workspace(),
        fixture.workspace(),
        &frozen,
        AgentRole::Monitor,
    );
    let model = crate::db::chat::ModelSnapshot {
        provider: frozen.provider.clone(),
        model: frozen.model.clone(),
        variant: frozen.variant.clone(),
    };
    let ctx = crate::event::chat::subagent::assemble_child_context(
        persistence.database().expect("db"),
        fixture.workspace(),
        &key.session,
        &agent_id,
        &child_session,
        &current_user_id,
        AgentRole::Monitor,
        crate::event::chat::subagent::ChildPromptContext {
            system_prompt_override: inputs.system_prompt_override.as_deref(),
            model_identity: Some(&inputs.model_identity),
            environment: &inputs.environment,
            instructions: inputs.instructions.as_deref(),
            skills: inputs.skills.as_deref(),
            mcp: &inputs.mcp,
        },
        fixture.workspace(),
        &model,
        frozen.variant_params.clone(),
        frozen.context_limit,
        frozen.output_limit,
        frozen.supports_image_input,
        &[crate::db::chat::PersistedInputItem::Text {
            text: "context note alpha".into(),
        }],
        &state.subagents_config,
        crate::event::chat::subagent::ChildTurnLimits {
            depth: 1,
            max_depth: 2,
            max_tokens: None,
        },
    )
    .expect("assemble child context");
    let items = ctx.items();
    assert!(
        matches!(items.last(), Some(ModelItem::UserText(text))
            if text.starts_with("Message Type: MESSAGE\nTask name: /root/poller\nSender: /root\nPayload:\ncontext note alpha")),
        "trigger_turn=false input must use MESSAGE envelope: {items:?}"
    );
}
