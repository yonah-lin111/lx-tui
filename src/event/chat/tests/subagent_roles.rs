//! 子 agent 角色能力（模型锁定、monitor 默认档位、能力收紧）、`<subagents>` 快照、
//! `<turn_aborted>` 中断回放与面板内联追问的事件层测试。

use super::subagent::{fixture_with_parent, op, spawn_explorer};
use super::*;
use crate::app::effects::Effect;
use crate::chat::agent::{
    AgentRole, BaseRole, SUBAGENT_TURN_ABORTED_MARKER, SubagentOp, SubagentOutcome, ToolSurface,
};
use crate::config::subagents::{CustomRoleConfig, RoleCapabilities};
use crate::tiling::PaneId;

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
        notify_parent: true,
        mutation_user_id: std::sync::Arc::new(std::sync::RwLock::new(String::new())),
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
    let (fixture, mut state, key, mut persistence, mut streams, _parent) = fixture_with_parent();
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
        notify_parent: false,
    });
    // 事件循环顺序：持久化泵先执行；该效应必须被认作聊天效应留到下一层，不能被吞掉。
    persistence.pump(&mut state);
    assert!(
        matches!(
            state.effects.as_slice(),
            [Effect::SendSubagentUserInput { .. }]
        ),
        "面板追问被持久化泵吞掉：{:?}",
        state.effects
    );
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

// ===== T3: @[refer-subagent] 当轮提示块 =====

fn mention_run(
    agent_id: &str,
    nickname: Option<&str>,
    role: AgentRole,
    status: &str,
) -> crate::app::state::SubagentRun {
    crate::app::state::SubagentRun::from_record(&crate::db::chat::SubagentRecord {
        agent_id: agent_id.into(),
        nickname: nickname.map(str::to_string),
        agent_path: "/root/turing".into(),
        child_session_id: format!("child-{agent_id}"),
        parent_session_id: "sess-parent".into(),
        spawn_message_id: None,
        role,
        model_provider: String::new(),
        model_id: String::new(),
        model_variant: None,
        status: status.into(),
        title: "auth lookup".into(),
        created_at_ms: 1_000,
        completed_at_ms: None,
        report_json: None,
        report_delivered: false,
    })
}

fn mention_report(agent_id: &str, body: &str) -> crate::chat::agent::SubagentReportView {
    crate::chat::agent::SubagentReportView {
        agent_id: agent_id.into(),
        nickname: None,
        title: "auth lookup".into(),
        role: AgentRole::Explorer,
        status: "completed".into(),
        files_changed: 0,
        changed_paths: Vec::new(),
        duration_ms: None,
        body: body.into(),
        delivered_at_ms: 2_000,
    }
}

fn reference_blocks(
    draft: &str,
    config: &crate::config::subagents::SubagentsSection,
    runs: &[crate::app::state::SubagentRun],
) -> Vec<String> {
    crate::event::chat::subagent::snapshot::subagent_reference_blocks(draft, config, runs)
}

#[test]
fn subagent_reference_blocks_render_active_instance_and_role_guidance() {
    let config = crate::config::subagents::SubagentsSection::default();
    let runs = vec![mention_run(
        "agent-turing",
        Some("Turing"),
        AgentRole::Explorer,
        "running",
    )];
    let blocks = reference_blocks(
        "@[refer-subagent](Turing:explorer) then @[refer-subagent](monitor)",
        &config,
        &runs,
    );
    assert_eq!(blocks.len(), 2, "blocks: {blocks:?}");
    let active = &blocks[0];
    assert!(active.contains("kind: active_instance"), "{active}");
    assert!(active.contains("agent_id: agent-turing"), "{active}");
    assert!(active.contains("nickname: Turing"), "{active}");
    assert!(active.contains("status: running"), "{active}");
    assert!(active.contains("title: auth lookup"), "{active}");
    assert!(active.contains("\ntask: auth lookup\n"), "{active}");
    assert!(
        active.contains("without searching workspace files"),
        "{active}"
    );
    assert!(active.contains("`list_agents`"), "{active}");
    assert!(active.contains("`send_input`"), "{active}");
    assert!(active.contains("`wait_agent`"), "{active}");
    assert!(!active.contains("spawn_agent"), "{active}");

    let role = &blocks[1];
    assert!(role.contains("kind: role"), "{role}");
    assert!(role.contains("role: monitor"), "{role}");
    assert!(role.contains("base_role: monitor"), "{role}");
    assert!(role.contains("scope: built-in"), "{role}");
    assert!(
        role.contains("tools: read, search, exec_command, write_stdin\ncapabilities:"),
        "{role}"
    );
    assert!(
        role.contains("capabilities: allow_web_search=true, allow_mcp=true, allow_skills=true"),
        "{role}"
    );
    assert!(role.contains("model: inherits parent model"), "{role}");
    assert!(
        role.contains("system_prompt:\nYou are a monitor subagent:"),
        "{role}"
    );
    assert!(role.contains("without searching workspace files"), "{role}");
    assert!(
        role.contains("`spawn_agent` with `role: \"monitor\"`"),
        "{role}"
    );
}

#[test]
fn subagent_reference_blocks_render_builtin_role_full_spec() {
    let config = crate::config::subagents::SubagentsSection::default();
    let blocks = reference_blocks(
        "@[refer-subagent](explorer) then @[refer-subagent](monitor)",
        &config,
        &[],
    );
    assert_eq!(blocks.len(), 2, "blocks: {blocks:?}");

    let explorer = &blocks[0];
    assert!(explorer.contains("role: explorer"), "{explorer}");
    assert!(explorer.contains("base_role: explorer"), "{explorer}");
    assert!(
        explorer.contains("tools: read, search, web_search\ncapabilities:"),
        "{explorer}"
    );
    assert!(
        explorer.contains("capabilities: allow_web_search=true, allow_mcp=true, allow_skills=true"),
        "{explorer}"
    );
    assert!(
        explorer.contains("model: inherits parent model"),
        "{explorer}"
    );
    assert!(
        explorer.contains("system_prompt:\nYou are an explorer:"),
        "{explorer}"
    );
    assert!(
        explorer.contains("without searching workspace files"),
        "{explorer}"
    );
    assert!(
        explorer.contains("`spawn_agent` with `role: \"explorer\"`"),
        "{explorer}"
    );

    let monitor = &blocks[1];
    assert!(monitor.contains("role: monitor"), "{monitor}");
    assert!(monitor.contains("base_role: monitor"), "{monitor}");
    assert!(
        monitor.contains("tools: read, search, exec_command, write_stdin\ncapabilities:"),
        "{monitor}"
    );
    assert!(
        monitor.contains("model: inherits parent model"),
        "{monitor}"
    );
    assert!(
        monitor.contains("system_prompt:\nYou are a monitor subagent:"),
        "{monitor}"
    );
    assert!(
        monitor.contains("without searching workspace files"),
        "{monitor}"
    );
    assert!(
        monitor.contains("`spawn_agent` with `role: \"monitor\"`"),
        "{monitor}"
    );
}

#[test]
fn subagent_reference_blocks_render_custom_role_effective_spec() {
    let mut config = crate::config::subagents::SubagentsSection::default();
    config.roles.push(CustomRoleConfig {
        name: "auditor".into(),
        base_role: BaseRole::Explorer,
        description: "audit diffs before merge".into(),
        system_prompt: "You are the auditor: verify diffs before merge.".into(),
        model: Some(crate::config::ai::ModelSelection {
            provider: "p".into(),
            model: "m".into(),
            variant: None,
        }),
        lock_model: true,
        capabilities: RoleCapabilities {
            allow_web_search: false,
            ..Default::default()
        },
        ..Default::default()
    });
    let blocks = reference_blocks("@[refer-subagent](auditor)", &config, &[]);
    assert_eq!(blocks.len(), 1, "blocks: {blocks:?}");
    let block = &blocks[0];
    assert!(block.contains("kind: role"), "{block}");
    assert!(block.contains("role: auditor"), "{block}");
    assert!(block.contains("base_role: explorer"), "{block}");
    assert!(block.contains("scope: custom (user)"), "{block}");
    assert!(
        block.contains("description: audit diffs before merge"),
        "{block}"
    );
    assert!(
        block.contains("tools: read, search\ncapabilities:"),
        "web_search must be stripped: {block}"
    );
    assert!(
        block.contains("capabilities: allow_web_search=false, allow_mcp=true, allow_skills=true"),
        "{block}"
    );
    assert!(block.contains("model: p/m (locked)"), "{block}");
    assert!(
        block.contains("system_prompt:\nYou are the auditor: verify diffs before merge."),
        "{block}"
    );
    assert!(
        block.contains("`spawn_agent` with `role: \"auditor\"`"),
        "{block}"
    );
}

#[test]
fn subagent_reference_blocks_preserve_finished_instance_when_completed() {
    let config = crate::config::subagents::SubagentsSection::default();
    let mut run = mention_run(
        "agent-turing",
        Some("Turing"),
        AgentRole::Explorer,
        "completed",
    );
    run.report = Some(mention_report("agent-turing", "found the auth flow"));
    let blocks = reference_blocks(
        "@[refer-subagent](Turing:explorer) then @[refer-subagent](UnknownAgent:explorer)",
        &config,
        &[run],
    );
    assert_eq!(blocks.len(), 2, "blocks: {blocks:?}");

    let finished = &blocks[0];
    assert!(finished.contains("kind: finished_instance"), "{finished}");
    assert!(finished.contains("agent_id: agent-turing"), "{finished}");
    assert!(finished.contains("status: completed"), "{finished}");
    assert!(
        finished.contains("latest_report: found the auth flow"),
        "{finished}"
    );
    assert!(
        finished.contains("`resume_agent` / `send_input`"),
        "{finished}"
    );
    assert!(!finished.contains("\nkind: role"), "{finished}");

    let fallback = &blocks[1];
    assert!(fallback.contains("kind: role"), "{fallback}");
    assert!(fallback.contains("role: explorer"), "{fallback}");
    assert!(fallback.contains("scope: built-in"), "{fallback}");
    assert!(fallback.contains("`spawn_agent`"), "{fallback}");
}

#[test]
fn subagent_reference_blocks_deduplicate_repeated_references() {
    let config = crate::config::subagents::SubagentsSection::default();
    let runs = vec![mention_run(
        "agent-turing",
        Some("Turing"),
        AgentRole::Explorer,
        "idle",
    )];
    let blocks = reference_blocks(
        "@[refer-subagent](Turing:explorer) and again @[refer-subagent](Turing:explorer) \
         plus @[refer-subagent](monitor) and @[refer-subagent](monitor)",
        &config,
        &runs,
    );
    assert_eq!(blocks.len(), 2, "blocks: {blocks:?}");
    assert!(
        blocks[0].contains("kind: active_instance"),
        "{:?}",
        blocks[0]
    );
    assert!(blocks[1].contains("role: monitor"), "{:?}", blocks[1]);
}

#[test]
fn subagent_reference_blocks_list_executor_control_tools_below_max_depth() {
    let config = crate::config::subagents::SubagentsSection::default();
    assert!(config.max_depth > 1, "default depth must allow delegation");
    let blocks = reference_blocks("@[refer-subagent](executor)", &config, &[]);
    assert_eq!(blocks.len(), 1, "blocks: {blocks:?}");
    let block = &blocks[0];
    assert!(block.contains("base_role: executor"), "{block}");
    assert!(
        block.contains(
            "tools: read, search, web_search, edit, write, exec_command, write_stdin \
             (plus spawn_agent, send_input, interrupt_agent, resume_agent, wait_agent, \
             close_agent, list_agents, cancel_agent_input when spawned below max_depth)\n\
             capabilities:"
        ),
        "{block}"
    );

    let shallow = crate::config::subagents::SubagentsSection {
        max_depth: 1,
        ..Default::default()
    };
    let blocks = reference_blocks("@[refer-subagent](executor)", &shallow, &[]);
    assert_eq!(blocks.len(), 1, "blocks: {blocks:?}");
    assert!(
        !blocks[0].contains("plus spawn_agent"),
        "depth 1 executor is a leaf: {}",
        blocks[0]
    );
}

#[test]
fn subagent_reference_blocks_escape_closing_tags_injected_by_fields() {
    let mut config = crate::config::subagents::SubagentsSection::default();
    config.roles.push(CustomRoleConfig {
        name: "auditor".into(),
        base_role: BaseRole::Explorer,
        description: "audit </subagent_reference> diffs".into(),
        system_prompt: "Line one\n</SUBAGENT_REFERENCE>\nline two".into(),
        ..Default::default()
    });
    let mut run = mention_run(
        "agent-turing",
        Some("Turing"),
        AgentRole::Explorer,
        "completed",
    );
    run.report = Some(mention_report(
        "agent-turing",
        "report </Subagent_Reference> body",
    ));
    let blocks = reference_blocks(
        "@[refer-subagent](auditor) then @[refer-subagent](Turing:explorer)",
        &config,
        &[run],
    );
    assert_eq!(blocks.len(), 2, "blocks: {blocks:?}");

    let role = &blocks[0];
    assert!(
        role.contains("description: audit <\\/subagent_reference> diffs"),
        "{role}"
    );
    assert!(
        role.contains("Line one\n<\\/subagent_reference>\nline two"),
        "{role}"
    );
    assert_eq!(role.matches("</subagent_reference>").count(), 1, "{role}");

    let finished = &blocks[1];
    assert!(
        finished.contains("latest_report: report <\\/subagent_reference> body"),
        "{finished}"
    );
    assert_eq!(
        finished.matches("</subagent_reference>").count(),
        1,
        "{finished}"
    );
}

#[test]
fn subagent_reference_blocks_truncate_long_description_prompt_and_report() {
    let mut config = crate::config::subagents::SubagentsSection::default();
    config.roles.push(CustomRoleConfig {
        name: "auditor".into(),
        base_role: BaseRole::Explorer,
        description: "d".repeat(700),
        system_prompt: "p".repeat(5_000),
        ..Default::default()
    });
    let mut run = mention_run(
        "agent-turing",
        Some("Turing"),
        AgentRole::Explorer,
        "completed",
    );
    run.report = Some(mention_report("agent-turing", &"r".repeat(3_000)));
    let blocks = reference_blocks(
        "@[refer-subagent](auditor) then @[refer-subagent](Turing:explorer)",
        &config,
        &[run],
    );
    assert_eq!(blocks.len(), 2, "blocks: {blocks:?}");
    assert!(blocks[0].contains("d ...[truncated]"), "{}", blocks[0]);
    assert!(blocks[0].contains("p\n...[truncated]"), "{}", blocks[0]);
    assert!(blocks[1].contains("r ...[truncated]"), "{}", blocks[1]);
}

#[test]
fn subagent_reference_blocks_deduplicate_case_insensitively_and_by_agent_id() {
    let config = crate::config::subagents::SubagentsSection::default();
    let runs = vec![mention_run(
        "agent-turing",
        Some("Turing"),
        AgentRole::Explorer,
        "idle",
    )];
    let blocks = reference_blocks(
        "@[refer-subagent](Turing:explorer) @[refer-subagent](turing:explorer) \
         @[refer-subagent](agent-turing:monitor) @[refer-subagent](AGENT-TURING:explorer)",
        &config,
        &runs,
    );
    assert_eq!(blocks.len(), 1, "blocks: {blocks:?}");
    assert!(blocks[0].contains("kind: active_instance"), "{}", blocks[0]);
    assert!(
        blocks[0].contains("agent_id: agent-turing"),
        "{}",
        blocks[0]
    );
}

#[test]
fn subagent_reference_blocks_prefer_bare_role_name_over_same_named_instance() {
    let config = crate::config::subagents::SubagentsSection::default();
    let runs = vec![mention_run(
        "agent-explorer",
        Some("explorer"),
        AgentRole::Monitor,
        "running",
    )];
    let blocks = reference_blocks("@[refer-subagent](explorer)", &config, &runs);
    assert_eq!(blocks.len(), 1, "blocks: {blocks:?}");
    assert!(blocks[0].contains("kind: role"), "{}", blocks[0]);
    assert!(blocks[0].contains("role: explorer"), "{}", blocks[0]);
    assert!(!blocks[0].contains("agent_id:"), "{}", blocks[0]);
}

/// 面板追问完成后：新子轮的 user 消息与流式 delta 必须都落到 Agents 详情。
#[tokio::test]
async fn resumed_completed_agent_deltas_land_in_the_detail_chat() {
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
    assert!(!streams.subagents.contains_key(&child_session));

    state.effects.push(Effect::SendSubagentUserInput {
        pane: key.pane,
        session_id: key.session.clone(),
        agent_id: agent_id.clone(),
        text: "ask again".into(),
        notify_parent: false,
    });
    assert!(crate::event::chat::apply_subagent_effects(
        &mut state,
        &mut streams,
        &persistence
    ));

    let run = state
        .pane_anywhere(key.pane)
        .expect("pane")
        .lx
        .agents
        .by_agent(&agent_id)
        .expect("run");
    assert_eq!(run.status, "running", "resume 后应重新进入 running");
    assert!(
        run.chat
            .messages
            .iter()
            .any(|message| message.role == "user" && message.text().contains("ask again")),
        "详情缺少新追问的用户消息：{:?}",
        run.chat
            .messages
            .iter()
            .map(|message| (message.role.clone(), message.text()))
            .collect::<Vec<_>>()
    );

    let changed = crate::event::chat::handle_subagent_net_event(
        &mut state,
        &mut streams,
        super::ChatNetEvent::Delta {
            key: RequestKey {
                pane: key.pane,
                session: child_session,
                generation: key.generation,
            },
            deltas: vec![ChatDelta::Part(crate::chat::events::PartDelta {
                kind: PartKind::Text,
                text: "second reply".into(),
                metadata: None,
            })],
        },
    );
    assert!(changed);
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
            .any(|message| message.role == "assistant" && message.text().contains("second reply")),
        "详情缺少新子轮输出：{:?}",
        run.chat
            .messages
            .iter()
            .map(|message| (message.role.clone(), message.text()))
            .collect::<Vec<_>>()
    );
    let _ = fixture;
}

/// 私聊追问：不生成报告、不唤醒主 agent；显式 include_private 才能读到内容。
#[tokio::test]
async fn private_panel_follow_up_stays_out_of_the_parent_and_is_readable_on_request() {
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
    // 清掉首轮自动汇报轮留下的 UI/请求状态，让父窗格回到 Idle。
    let pane = state.pane_mut_anywhere(key.pane).expect("pane");
    pane.lx.chat.request = crate::app::state::RequestState::Idle;
    pane.lx.chat.status.streaming = false;
    // 把首轮的正常报告投递掉，确保下面观察到的只可能是私聊轮产生的消息。
    assert!(crate::event::chat::subagent::drain_reports(
        &mut state,
        &persistence,
        key.pane
    ));
    let pane = state.pane_mut_anywhere(key.pane).expect("pane");
    pane.lx.chat.request = crate::app::state::RequestState::Idle;
    pane.lx.chat.status.streaming = false;
    pane.lx.chat.messages.clear();
    state.effects.clear();
    let before = state
        .pane_anywhere(key.pane)
        .expect("pane")
        .lx
        .chat
        .messages
        .len();

    // 私聊追问：notify_parent=false。
    state.effects.push(Effect::SendSubagentUserInput {
        pane: key.pane,
        session_id: key.session.clone(),
        agent_id: agent_id.clone(),
        text: "secret".into(),
        notify_parent: false,
    });
    assert!(crate::event::chat::apply_subagent_effects(
        &mut state,
        &mut streams,
        &persistence
    ));
    assert!(
        streams
            .subagents
            .values()
            .any(|handle| handle.agent_id == agent_id && handle.status == "running"),
        "私聊仍应立即起子轮"
    );
    let changed = crate::event::chat::handle_subagent_net_event(
        &mut state,
        &mut streams,
        ChatNetEvent::Delta {
            key: RequestKey {
                pane: key.pane,
                session: child_session.clone(),
                generation: key.generation,
            },
            deltas: vec![ChatDelta::Part(crate::chat::events::PartDelta {
                kind: PartKind::Text,
                text: "private reply".into(),
                metadata: None,
            })],
        },
    );
    assert!(changed);
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

    let database = persistence.database().expect("db");
    assert!(
        database
            .pending_reports(fixture.workspace(), &key.session)
            .expect("pending")
            .is_empty(),
        "私聊轮不得生成待投递报告"
    );
    assert_eq!(
        state
            .pane_anywhere(key.pane)
            .expect("pane")
            .lx
            .chat
            .messages
            .len(),
        before,
        "父对话不得因为私聊轮新增报告消息"
    );
    let tail = database
        .private_turn_tail(fixture.workspace(), &key.session, &agent_id)
        .expect("query")
        .expect("private tail");
    assert!(tail.contains("secret"), "私聊 user 文本：{tail}");
    assert!(
        tail.contains("private reply"),
        "私聊 assistant 文本：{tail}"
    );
    assert!(
        database
            .private_turn_tail(fixture.workspace(), &key.session, "missing")
            .expect("query")
            .is_none(),
        "未知 agent 无私聊"
    );
    let listed = op(
        &mut state,
        &mut streams,
        &persistence,
        &key,
        SubagentOp::List {
            path_prefix: None,
            include_private: true,
        },
    )
    .await
    .expect("list");
    let SubagentOutcome::Listed { agents } = listed else {
        panic!("expected listed");
    };
    let tail = agents
        .iter()
        .find(|agent| agent.agent_id == agent_id)
        .and_then(|agent| agent.private_tail.as_deref())
        .expect("list_agents include_private tail");
    assert!(tail.contains("secret") && tail.contains("private reply"));
}

/// 运行中子 agent 的 steer：DB 切片段、handle 锚点前移、pending 被消费并回执给 loop。
#[tokio::test]
async fn running_subagent_steer_commits_a_new_segment_and_returns_input() {
    let (fixture, mut state, key, persistence, mut streams, _parent) = fixture_with_parent();
    let agent_id = spawn_explorer(&mut state, &mut streams, &persistence, &key, "first task").await;
    let child_session = streams
        .subagents
        .values()
        .find(|handle| handle.agent_id == agent_id)
        .expect("handle")
        .child_session_id
        .clone();
    let previous_assistant = streams.subagents[&child_session].child_assistant_id.clone();
    let target = state.pane_mut_anywhere(key.pane).expect("pane");
    target
        .lx
        .agents
        .by_agent_mut(&agent_id)
        .expect("run")
        .chat
        .pending_steers
        .push(crate::app::state::lx::chat::steer::PendingSteer {
            text: "steer this turn".into(),
            attachments: Vec::new(),
            model_images: Vec::new(),
            model: crate::db::chat::ModelSnapshot {
                provider: "p".into(),
                model: "m".into(),
                variant: None,
            },
        });
    let (reply, receiver) = tokio::sync::oneshot::channel();
    let changed = crate::event::chat::steer::deliver(
        &mut state,
        &mut streams,
        &persistence,
        RequestKey {
            pane: key.pane,
            session: child_session.clone(),
            generation: key.generation,
        },
        reply,
    );
    assert!(changed);
    let inputs = receiver.await.expect("reply").expect("steer inputs");
    assert_eq!(inputs.len(), 1);
    assert_eq!(inputs[0].text, "steer this turn");
    assert_ne!(
        streams.subagents[&child_session].child_assistant_id, previous_assistant,
        "handle 必须切到新 assistant 片段"
    );
    let run = state
        .pane_anywhere(key.pane)
        .expect("pane")
        .lx
        .agents
        .by_agent(&agent_id)
        .expect("run");
    assert!(run.chat.pending_steers.is_empty(), "pending 被消费");
    assert!(
        run.chat
            .messages
            .iter()
            .any(|message| message.role == "user" && message.text() == "steer this turn"),
        "UI 出现 steer user 消息：{:?}",
        run.chat
            .messages
            .iter()
            .map(|message| (message.role.clone(), message.text()))
            .collect::<Vec<_>>()
    );
    let messages = persistence
        .database()
        .expect("db")
        .load_chat_session(fixture.workspace(), &child_session)
        .expect("load")
        .expect("messages");
    assert!(
        messages.iter().any(|message| message.role == "user"
            && message
                .parts
                .iter()
                .any(|part| part.content == "steer this turn")),
        "DB 写入 steer user 消息"
    );
}
