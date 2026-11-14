//! 子 agent 环境快照与面板内联追问调度。
//!
//! `<subagents>` 快照只展示可复用子树（排除 `cancelled`），按活跃度排序并限制
//! 8 条 / 1024 B；面板追问在目标已结束时先自动 `resume` 再投递 `send_input`。

use std::path::Path;
use std::time::Instant;

use super::runner::{load_subagent_tree, tree_queue_states};
use super::*;
use crate::chat::agent::{AgentRole, AgentStatus, BaseRole, SubagentInputItem, ToolSurface};
use crate::db::Database;

/// `@[refer-subagent]` 提示块内联字段（`task` / `description`）的单行化截断上限。
const MAX_REFERENCE_FIELD_CHARS: usize = 600;
/// `latest_report` 的单行化截断上限（报告通常长于普通字段）。
const MAX_REFERENCE_REPORT_CHARS: usize = 2_400;
/// `system_prompt` 的截断上限（保留换行）。
const MAX_REFERENCE_PROMPT_CHARS: usize = 4_000;
/// 单轮最多注入的 `<subagent_reference>` 块数（限住当轮上下文体量）。
const MAX_SUBAGENT_REFERENCE_BLOCKS: usize = 8;

/// 环境快照最多列出的子 agent 条数。
pub const MAX_SNAPSHOT_SUBAGENTS: usize = 8;
/// 环境快照（含 `<subagents>` 标签）的最大字节数。
pub const MAX_SNAPSHOT_SUBAGENT_BYTES: usize = 1024;

/// 快照条目：DB 记录与内存句柄归一化后的展示数据。
struct SnapshotEntry {
    agent_path: String,
    agent_id: String,
    nickname: String,
    role: String,
    status: String,
    title: String,
    created_at_ms: i64,
}

/// 生成当前根会话树下已知子 agent 的结构化环境摘要（含 running / idle / interrupted /
/// completed，排除 cancelled）。无 DB 时回退到内存句柄（单测路径）。
pub(crate) fn known_subagent_lines(
    streams: &ChatStreams,
    database: Option<&Database>,
    workspace: Option<&Path>,
    parent_session: &str,
) -> Vec<String> {
    let mut entries = Vec::new();
    let mut known_ids = std::collections::HashSet::new();
    if let (Some(database), Some(workspace)) = (database, workspace) {
        let tree = runner::load_subagent_tree(database, workspace, parent_session);
        let queues = runner::tree_queue_states(database, workspace, &tree);
        for record in &tree.records {
            let status = projected_agent_status(streams, record, queues.get(&record.agent_id));
            if AgentStatus::parse(&status) == Some(AgentStatus::Cancelled) {
                continue;
            }
            known_ids.insert(record.agent_id.clone());
            entries.push(SnapshotEntry {
                agent_path: record.agent_path.clone(),
                agent_id: record.agent_id.clone(),
                nickname: display_nickname(record.nickname.as_deref()),
                role: record.role.as_str().to_string(),
                status,
                title: record.title.clone(),
                created_at_ms: record.created_at_ms,
            });
        }
    }
    for handle in streams.subagents.values() {
        if handle.parent.session != parent_session && handle.root_session != parent_session {
            continue;
        }
        if known_ids.contains(&handle.agent_id) || handle.status == STATUS_CANCELLED {
            continue;
        }
        entries.push(SnapshotEntry {
            agent_path: handle.agent_path.clone(),
            agent_id: handle.agent_id.clone(),
            nickname: display_nickname(handle.nickname.as_deref()),
            role: handle.role.as_str().to_string(),
            status: handle.status.to_string(),
            title: if handle.title.trim().is_empty() {
                handle.task.clone()
            } else {
                handle.title.clone()
            },
            created_at_ms: handle.started_at_ms,
        });
    }
    if entries.is_empty() {
        return Vec::new();
    }
    entries.sort_by(|left, right| {
        status_priority(&left.status)
            .cmp(&status_priority(&right.status))
            .then(right.created_at_ms.cmp(&left.created_at_ms))
    });

    let header = "<subagents>";
    let footer = "</subagents>";
    let mut bytes = header.len() + footer.len() + 2;
    let mut lines = vec![header.to_string()];
    for entry in entries.into_iter().take(MAX_SNAPSHOT_SUBAGENTS) {
        let title = first_line(&entry.title);
        let line = if title.is_empty() {
            format!(
                "- path={} id={} nickname={} role={} status={}",
                entry.agent_path, entry.agent_id, entry.nickname, entry.role, entry.status
            )
        } else {
            format!(
                "- path={} id={} nickname={} role={} status={} · {}",
                entry.agent_path, entry.agent_id, entry.nickname, entry.role, entry.status, title
            )
        };
        let extra = line.len() + 1;
        if bytes.saturating_add(extra) > MAX_SNAPSHOT_SUBAGENT_BYTES {
            continue;
        }
        bytes = bytes.saturating_add(extra);
        lines.push(line);
    }
    if lines.len() == 1 {
        return Vec::new();
    }
    lines.push(footer.to_string());
    lines
}

/// 兼容旧入口：仅基于内存句柄的环境快照。
#[allow(dead_code)]
pub(crate) fn running_subagent_lines(streams: &ChatStreams, parent_session: &str) -> Vec<String> {
    known_subagent_lines(streams, None, None, parent_session)
}

/// 最新一条非 shell 用户消息是否来自合成 subagent 汇报轮：此时草稿是注入的报告
/// 文本而非用户输入，不应再解析其中的 `@` 引用块。
pub(crate) fn is_synthetic_report_turn(messages: &[crate::app::state::ChatMessageView]) -> bool {
    messages
        .iter()
        .rev()
        .find(|message| message.role == "user" && !message.is_user_shell())
        .is_some_and(|message| !message.reports.is_empty())
}

/// 从用户当轮草稿中提取 `@[refer-subagent](...)` 引用，生成英文自身规格与调度提示块
/// （仅当轮注入，不落库）。重复点名同一目标只生成一个块。
pub(crate) fn subagent_reference_blocks(
    draft_text: &str,
    effective_subagents: &crate::config::subagents::SubagentsSection,
    runs: &[crate::app::state::SubagentRun],
) -> Vec<String> {
    let mut blocks = Vec::new();
    let mut seen = std::collections::HashSet::new();
    let mut seen_agent_ids = std::collections::HashSet::new();
    let masked = crate::app::paste::mask_pasted_text(draft_text);
    for reference in crate::app::markdown::parse_references(&masked) {
        if blocks.len() >= MAX_SUBAGENT_REFERENCE_BLOCKS {
            break;
        }
        if reference.kind != crate::app::markdown::ReferKind::Subagent {
            continue;
        }
        let raw_target = reference.path.trim();
        if raw_target.is_empty() || !seen.insert(raw_target.to_ascii_lowercase()) {
            continue;
        }
        // 裸角色名优先按角色解析（即使恰好有实例同名）；`<nickname_or_id>:<role>` 才查实例。
        let (instance_key, role_name) = match raw_target.rsplit_once(':') {
            Some((head, role)) => (head.trim(), Some(role.trim())),
            None => (raw_target, None),
        };
        let preferred_role = match role_name {
            Some(_) => None,
            None => effective_subagents.resolve_role(raw_target),
        };
        if preferred_role.is_none()
            && let Some(run) = find_mentioned_run(runs, instance_key)
        {
            if seen_agent_ids.insert(run.agent_id.to_ascii_lowercase()) {
                blocks.push(instance_reference_block(raw_target, run));
            }
            continue;
        }
        let Some(spec) = preferred_role
            .or_else(|| effective_subagents.resolve_role(role_name.unwrap_or(raw_target)))
        else {
            continue;
        };
        blocks.push(role_reference_block(
            raw_target,
            &spec,
            effective_subagents.max_depth,
        ));
    }
    blocks
}

/// 优先活动实例；否则从最近到最早取任一已结束实例（completed / failed / interrupted / cancelled）。
fn find_mentioned_run<'a>(
    runs: &'a [crate::app::state::SubagentRun],
    instance_key: &str,
) -> Option<&'a crate::app::state::SubagentRun> {
    runs.iter()
        .find(|run| {
            crate::app::state::AgentFilter::Active.matches(run)
                && instance_matches(run, instance_key)
        })
        .or_else(|| {
            runs.iter().rev().find(|run| {
                !crate::app::state::AgentFilter::Active.matches(run)
                    && instance_matches(run, instance_key)
            })
        })
}

fn instance_matches(run: &crate::app::state::SubagentRun, instance_key: &str) -> bool {
    run.agent_id.eq_ignore_ascii_case(instance_key)
        || run
            .nickname
            .as_deref()
            .map(str::trim)
            .filter(|nickname| !nickname.is_empty())
            .is_some_and(|nickname| nickname.eq_ignore_ascii_case(instance_key))
}

/// 存活或已结束实例的自身规格块；两者只差 `kind` 与追问/续作指引。
fn instance_reference_block(raw_target: &str, run: &crate::app::state::SubagentRun) -> String {
    let active = crate::app::state::AgentFilter::Active.matches(run);
    let nickname = run
        .nickname
        .as_deref()
        .map(str::trim)
        .filter(|nickname| !nickname.is_empty())
        .unwrap_or(run.agent_id.as_str());
    let kind = if active {
        "active_instance"
    } else {
        "finished_instance"
    };
    let mut block = format!(
        "<subagent_reference>\n\
         kind: {kind}\n\
         reference: @[refer-subagent]({})\n\
         agent_id: {}\n\
         nickname: {}\n\
         role: {}\n\
         status: {}\n\
         agent_path: {}\n\
         title: {}",
        reference_scalar(raw_target),
        reference_scalar(&run.agent_id),
        reference_scalar(nickname),
        run.role.as_str(),
        reference_scalar(&run.status),
        reference_scalar(&run.agent_path),
        reference_scalar(&run.title),
    );
    if let Some(task) = reference_field(&run.task) {
        block.push_str(&format!("\ntask: {task}"));
    }
    if let Some(report) = run
        .report
        .as_ref()
        .and_then(|report| reference_report(&report.body))
    {
        block.push_str(&format!("\nlatest_report: {report}"));
    }
    let target = reference_scalar(&run.agent_id);
    if active {
        block.push_str(&format!(
            "\nguidance: The user explicitly referenced this active subagent in the current session. \
             If the user is asking about this subagent (its role, task, status, or progress), \
             answer directly from the details above (or call `list_agents` to inspect the session tree) \
             without searching workspace files. If the user is assigning follow-up work or waiting \
             for completion, prefer interacting with it directly using `send_input` (target=\"{target}\") \
             or `wait_agent` instead of spawning a duplicate subagent.\n\
             </subagent_reference>"
        ));
    } else {
        block.push_str(&format!(
            "\nguidance: The user explicitly referenced this finished subagent instance in the current session. \
             If the user is asking about this subagent (its role, task, status, findings, or report), \
             answer directly from the details and latest_report above (or call `list_agents` to inspect \
             the session tree) without searching workspace files. If the user is assigning follow-up work \
             to this subagent, prefer resuming or messaging it with `resume_agent` / `send_input` \
             (target=\"{target}\") instead of spawning a duplicate subagent.\n\
             </subagent_reference>"
        ));
    }
    block
}

/// 内置或自定义角色的完整规格块（无实例可匹配时的回退）。
fn role_reference_block(
    raw_target: &str,
    spec: &crate::config::subagents::ResolvedRoleSpec,
    max_depth: usize,
) -> String {
    let scope = if spec.is_builtin {
        "built-in"
    } else if spec.is_project_scoped {
        "custom (project)"
    } else {
        "custom (user)"
    };
    let base_agent_role = match spec.base_role {
        BaseRole::Explorer => AgentRole::Explorer,
        BaseRole::Executor => AgentRole::Executor,
        BaseRole::Monitor => AgentRole::Monitor,
    };
    let mut tools = ToolSurface::from_role(base_agent_role)
        .names()
        .iter()
        .filter(|name| spec.capabilities.allow_web_search || **name != "web_search")
        .copied()
        .collect::<Vec<_>>()
        .join(", ");
    if spec.base_role == BaseRole::Executor && max_depth > 1 {
        tools.push_str(
            " (plus spawn_agent, send_input, interrupt_agent, resume_agent, wait_agent, \
             close_agent, list_agents, cancel_agent_input when spawned below max_depth)",
        );
    }
    let capabilities = format!(
        "allow_web_search={}, allow_mcp={}, allow_skills={}",
        spec.capabilities.allow_web_search,
        spec.capabilities.allow_mcp,
        spec.capabilities.allow_skills,
    );
    let model = match spec.default_model.as_ref() {
        Some(selection) => {
            let variant = selection
                .variant
                .as_deref()
                .map(|variant| format!(" ({variant})"))
                .unwrap_or_default();
            let mode = if spec.lock_model {
                "locked"
            } else {
                "default (overridable)"
            };
            format!(
                "{}/{}{variant} ({mode})",
                selection.provider, selection.model
            )
        }
        None => "inherits parent model".to_string(),
    };
    let system_prompt = spec
        .system_prompt_override
        .as_deref()
        .filter(|prompt| !prompt.trim().is_empty())
        .unwrap_or_else(|| base_agent_role.system_prompt());
    format!(
        "<subagent_reference>\n\
         kind: role\n\
         reference: @[refer-subagent]({})\n\
         role: {}\n\
         base_role: {}\n\
         scope: {scope}\n\
         description: {}\n\
         tools: {tools}\n\
         capabilities: {capabilities}\n\
         model: {}\n\
         system_prompt:\n\
         {}\n\
         guidance: The user explicitly referenced the `{}` subagent role. \
         Its complete specification, tool surface, capabilities, and role system prompt are \
         provided above (and general lx subagent documentation is available in the built-in \
         `lx-guide` skill). If the user is asking what this subagent role is, how it works, \
         or what tools/prompt it uses, answer directly from the specification above without \
         searching workspace files. If the user is asking to perform a task with this role, \
         delegate the relevant task by calling `spawn_agent` with `role: \"{}\"`.\n\
         </subagent_reference>",
        reference_scalar(raw_target),
        reference_scalar(&spec.name),
        spec.base_role.as_str(),
        reference_field(&spec.description).unwrap_or_default(),
        reference_scalar(&model),
        reference_prompt(system_prompt),
        reference_scalar(&spec.name),
        reference_scalar(&spec.name),
    )
}

/// 面板内联追问：目标已结束时自动 `resume`，随后投递并立即触发新一轮。
pub(super) fn send_user_input_from_panel(
    state: &mut AppState,
    streams: &mut ChatStreams,
    persistence: &Persistence,
    pane: PaneId,
    session_id: &str,
    agent_id: &str,
    text: &str,
    notify_parent: bool,
) -> bool {
    let text = text.trim();
    if text.is_empty() {
        return false;
    }
    let generation = state
        .pane_anywhere(pane)
        .map_or(0, |target| target.lx.chat.generation_counter);
    let key = RequestKey {
        pane,
        session: session_id.to_string(),
        generation,
    };
    if !streams.db_workspaces.contains_key(&key)
        && let Some(workspace) = super::workspace_for_pane(state, pane)
    {
        streams.db_workspaces.insert(key.clone(), workspace);
    }
    if !streams.frozen.contains_key(session_id)
        && let Some(frozen) = runner::resolve_frozen_for_pane(state, pane)
    {
        streams.frozen.insert(session_id.to_string(), frozen);
    }

    let record = persistence.database().and_then(|database| {
        streams.db_workspaces.get(&key).and_then(|workspace| {
            database
                .resolve_owned_subagent(workspace, session_id, agent_id)
                .ok()
                .flatten()
        })
    });
    let closed_in_memory = streams
        .subagents
        .values()
        .any(|handle| handle.agent_id == agent_id && handle.closed_without_report);
    let closed_in_db = record.as_ref().is_some_and(|rec| {
        let queue = persistence.database().and_then(|db| {
            streams.db_workspaces.get(&key).and_then(|ws| {
                db.subagent_queue_states(ws, &rec.parent_session_id)
                    .ok()
                    .and_then(|mut map| map.remove(&rec.agent_id))
            })
        });
        super::projected_agent_status(streams, rec, queue.as_ref()) == STATUS_CANCELLED
    });
    if closed_in_memory || closed_in_db {
        show_panel_toast(
            state,
            pane,
            ToastKind::Warning,
            crate::ui::text::LX_TASK_AGENT_REPLY_CLOSED.to_string(),
        );
        return true;
    }

    let is_active = streams.subagents.values().any(|handle| {
        handle.agent_id == agent_id
            && handle.status == STATUS_RUNNING
            && !handle.closed_without_report
    }) || streams.resumed_subagents.contains(agent_id);
    if !is_active && let Err(error) = runner::resume(state, streams, persistence, &key, agent_id) {
        show_panel_toast(state, pane, ToastKind::Danger, error);
        return true;
    }
    let nickname = record
        .as_ref()
        .and_then(|record| record.nickname.as_deref())
        .map(str::trim)
        .filter(|nickname| !nickname.is_empty())
        .map(str::to_string);
    let child_session = record
        .as_ref()
        .map(|record| record.child_session_id.clone());

    let outcome = runner::send_input(
        state,
        streams,
        persistence,
        &key,
        None,
        agent_id,
        &[SubagentInputItem::Text {
            text: text.to_string(),
        }],
        false,
        true,
        notify_parent,
    );
    match outcome {
        Ok(_) => {
            let message = nickname
                .map(|nickname| format!("Sent input to {nickname}"))
                .unwrap_or_else(|| "Sent input to subagent".to_string());
            show_panel_toast(state, pane, ToastKind::Default, message);
            let unloaded = state
                .pane_anywhere(pane)
                .and_then(|target| target.lx.agents.by_agent(agent_id))
                .is_some_and(|run| !run.loaded);
            if unloaded && let Some(child_session) = child_session {
                state.effects.push(Effect::LoadSubagentSession {
                    pane,
                    agent_id: agent_id.to_string(),
                    child_session,
                });
            }
            true
        }
        Err(error) => {
            show_panel_toast(state, pane, ToastKind::Danger, error);
            true
        }
    }
}

fn show_panel_toast(state: &mut AppState, pane: PaneId, kind: ToastKind, message: String) {
    update::show_lx_toast(
        state,
        pane,
        Toast::new(kind, message, Some(pane), Instant::now()),
    );
}

fn status_priority(status: &str) -> u8 {
    match AgentStatus::parse(status) {
        Some(AgentStatus::Running) => 0,
        Some(AgentStatus::Blocked) => 1,
        Some(AgentStatus::Idle) => 2,
        Some(AgentStatus::Interrupted) => 3,
        Some(AgentStatus::Failed) => 4,
        Some(AgentStatus::Completed) => 5,
        Some(AgentStatus::Cancelled) => u8::MAX,
        None => 6,
    }
}

fn display_nickname(nickname: Option<&str>) -> String {
    nickname
        .map(str::trim)
        .filter(|nickname| !nickname.is_empty())
        .unwrap_or("-")
        .to_string()
}

fn first_line(text: &str) -> String {
    text.lines()
        .next()
        .unwrap_or_default()
        .chars()
        .take(120)
        .collect()
}

/// 转义大小写不敏感的 `</subagent_reference` 闭合标签，避免字段提前闭合注入块。
fn escape_reference_close_tags(text: &str) -> String {
    const NEEDLE: &str = "</subagent_reference";
    let lowered = text.to_ascii_lowercase();
    let mut escaped = String::with_capacity(text.len());
    let mut cursor = 0;
    while let Some(offset) = lowered[cursor..].find(NEEDLE) {
        let start = cursor + offset;
        escaped.push_str(&text[cursor..start]);
        escaped.push_str("<\\/subagent_reference");
        cursor = start + NEEDLE.len();
    }
    escaped.push_str(&text[cursor..]);
    escaped
}

/// 去除控制字符（`keep_newlines` 时保留换行），转义闭合标签后按字符数截断。
fn truncate_reference_text(text: &str, limit: usize, suffix: &str, keep_newlines: bool) -> String {
    let cleaned: String = text
        .chars()
        .filter(|ch| !ch.is_control() || (keep_newlines && *ch == '\n'))
        .collect();
    let cleaned = escape_reference_close_tags(&cleaned);
    if cleaned.chars().count() <= limit {
        return cleaned;
    }
    let mut truncated: String = cleaned.chars().take(limit).collect();
    truncated.push_str(suffix);
    truncated
}

/// 单行化并截断提示块的内联字段；空字段返回 `None`（不输出空行）。
fn reference_field(text: &str) -> Option<String> {
    let single_line = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if single_line.is_empty() {
        return None;
    }
    Some(truncate_reference_text(
        &single_line,
        MAX_REFERENCE_FIELD_CHARS,
        " ...[truncated]",
        false,
    ))
}

/// 单行标量字段（`reference` / `nickname` / `title` / `agent_path` 等）；空值输出空串。
fn reference_scalar(text: &str) -> String {
    reference_field(text).unwrap_or_default()
}

/// `latest_report`：单行化并放宽到报告上限。
fn reference_report(text: &str) -> Option<String> {
    let single_line = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if single_line.is_empty() {
        return None;
    }
    Some(truncate_reference_text(
        &single_line,
        MAX_REFERENCE_REPORT_CHARS,
        " ...[truncated]",
        false,
    ))
}

/// `system_prompt`：保留换行、去除其余控制字符，并按上限截断。
fn reference_prompt(text: &str) -> String {
    truncate_reference_text(
        text.trim(),
        MAX_REFERENCE_PROMPT_CHARS,
        "\n...[truncated]",
        true,
    )
}

/// 同步窗格 `Agents` 列表中的状态投影与 `queued_count`（并刷新底栏 `N blocked` 计数与筛选高亮）。
pub(super) fn sync_agent_ui_queue(
    state: &mut AppState,
    streams: &ChatStreams,
    persistence: &Persistence,
    pane: PaneId,
    workspace: &Path,
    parent_session: &str,
) -> bool {
    let Some(database) = persistence.database() else {
        return false;
    };
    let tree = load_subagent_tree(database, workspace, parent_session);
    if tree.records.is_empty() {
        return false;
    }
    let queue_states = tree_queue_states(database, workspace, &tree);
    let Some(target) = state.pane_mut_anywhere(pane) else {
        return false;
    };
    let selected_agent_id = target
        .lx
        .task
        .as_ref()
        .and_then(|page| page.detail_agent_index(&target.lx.agents))
        .and_then(|index| target.lx.agents.runs.get(index))
        .map(|run| run.agent_id.clone());
    let mut changed = false;
    for record in &tree.records {
        let q = queue_states.get(&record.agent_id);
        let projected = super::projected_agent_status(streams, record, q);
        let queued_count = q.map_or(0, |s| s.queued_count);
        if let Some(run) = target.lx.agents.by_agent_mut(&record.agent_id)
            && (run.status != projected || run.queued_count != queued_count)
        {
            run.status = projected;
            run.queued_count = queued_count;
            changed = true;
        }
    }
    let blocked = target.lx.agents.blocked_count();
    if target.lx.chat.status.blocked_subagents != blocked {
        target.lx.chat.status.blocked_subagents = blocked;
        changed = true;
    }
    crate::app::update::restore_selected_task_agent(&mut target.lx, selected_agent_id.as_deref());
    target.lx.sync_mention_subagent_runs();
    changed
}
