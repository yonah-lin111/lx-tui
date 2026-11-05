//! `lx-tui session` 子命令执行与输出格式化。

use super::*;

pub fn run(
    cli: SessionCli,
    linked_worktree: Option<&Path>,
) -> Result<i32, Box<dyn std::error::Error>> {
    let SessionCli { global, command } = cli;
    let ctx = WorkspaceContext::resolve(global.cwd.as_deref(), linked_worktree)?;
    let out = Output::new(global.json, global.color);
    let db = ctx.open_db()?;
    let ws = &ctx.workspace;
    match command {
        SessionCommand::List { limit, offset } => list(&ctx, &db, limit, offset, &out)?,
        SessionCommand::Show {
            session_id,
            last,
            tools,
            thinking,
        } => {
            let id = resolve_target_session(&db, ws, session_id.as_deref(), last)?;
            show(&ctx, &db, &id, tools, thinking, &out)?;
        }
        SessionCommand::Rename {
            session_id,
            last,
            new_title,
        } => {
            let id = resolve_target_session(&db, ws, session_id.as_deref(), last)?;
            let value = json!({ "renamed": true, "session_id": id, "title": new_title });
            db.rename_chat_session(ws, &id, &new_title)
                .map_err(|e| format!("failed to rename session: {}", e.code()))?;
            emit_ok(
                &out,
                value,
                format!("session '{id}' renamed to '{new_title}'"),
            )?;
        }
        SessionCommand::Delete { session_id, last } => {
            let id = resolve_target_session(&db, ws, session_id.as_deref(), last)?;
            let descendants = db
                .descendant_session_ids(&id)
                .map_err(|e| format!("failed to list descendant sessions: {}", e.code()))?;
            // 与 TUI 的 DeleteChatSession 效应一致：先取得跨进程锁，再级联删除会话。
            let guard = acquire_session_lock(&id)?;
            db.delete_chat_session(ws, &id)
                .map_err(|e| format!("failed to delete session: {}", e.code()))?;
            drop(guard);
            for child in descendants
                .iter()
                .map(String::as_str)
                .chain(Some(id.as_str()))
            {
                crate::platform::remove_session_images(child);
            }
            emit_ok(
                &out,
                json!({ "deleted": true, "session_id": id }),
                format!("session '{id}' deleted"),
            )?;
        }
        SessionCommand::Fork {
            session_id,
            last,
            before,
        } => {
            let id = resolve_target_session(&db, ws, session_id.as_deref(), last)?;
            let forked = db
                .fork_chat_session(ws, &id, before.as_deref())
                .map_err(|e| format!("failed to fork session: {}", e.code()))?;
            let value = json!({ "forked": true, "session_id": id, "new_session_id": forked.session_id, "before": before, "restore_text": forked.restore_text });
            emit_ok(
                &out,
                value,
                format!("forked '{id}' -> '{}'", forked.session_id),
            )?;
        }
        SessionCommand::Rewind {
            session_id,
            last,
            mode,
            dry_run,
        } => {
            let id = resolve_target_session(&db, ws, session_id.as_deref(), last)?;
            ops::run_rewind(&ctx, &db, &id, mode, dry_run, &out)?;
        }
        SessionCommand::Compact {
            session_id,
            last,
            model,
            revert,
        } => {
            let id = resolve_target_session(&db, ws, session_id.as_deref(), last)?;
            ops::run_compact(&ctx, &db, &id, model.as_deref(), revert, &out)?;
        }
        SessionCommand::Agents {
            session_id,
            last,
            tree,
            filter,
        } => {
            let id = resolve_target_session(&db, ws, session_id.as_deref(), last)?;
            agents(&ctx, &db, &id, tree, filter, &out)?;
        }
        SessionCommand::Processes { session_id, last } => {
            let id = resolve_target_session(&db, ws, session_id.as_deref(), last)?;
            processes(&ctx, &db, &id, &out)?;
        }
    }
    Ok(EXIT_OK)
}

fn emit_ok(out: &Output, value: Value, message: String) -> Result<(), String> {
    out.emit(&value, |out, writer| {
        writeln!(writer, "{} {message}", out.green("OK"))
    })
}
fn list(
    ctx: &WorkspaceContext,
    db: &Database,
    limit: usize,
    offset: usize,
    out: &Output,
) -> Result<(), String> {
    let sessions = list_sessions(db, &ctx.workspace, offset, limit)?;
    let rows = sessions.iter().map(|s| json!({
        "id": s.external_id, "title": s.title, "model": session_model(s), "status": s.status,
        "updated_at_ms": s.updated_at_ms, "total_input_tokens": s.total_input_tokens, "total_output_tokens": s.total_output_tokens,
    })).collect::<Vec<_>>();
    let value =
        json!({ "workspace": ctx.workspace, "offset": offset, "limit": limit, "sessions": &rows });
    out.emit(&value, |out, writer| {
        if sessions.is_empty() {
            return writeln!(writer, "{}", out.muted("No sessions found."));
        }
        let table = sessions
            .iter()
            .map(|s| {
                vec![
                    s.external_id.clone(),
                    single_line(&s.title),
                    session_model(s),
                    s.status.clone(),
                    format!("{} / {}", s.total_input_tokens, s.total_output_tokens),
                    s.updated_at_ms.to_string(),
                ]
            })
            .collect::<Vec<_>>();
        out.print_table(
            writer,
            &["ID", "TITLE", "MODEL", "STATUS", "TOKENS", "UPDATED"],
            &table,
        )
    })
}
// 单页上限固定 200；更大 `--limit` 通过分页拉满，避免被底层静默截断。
fn list_sessions(
    db: &Database,
    workspace: &Path,
    offset: usize,
    limit: usize,
) -> Result<Vec<SessionSummary>, String> {
    let mut sessions = Vec::new();
    let mut page_offset = offset;
    loop {
        let page_size = (limit - sessions.len()).min(200);
        let page = db
            .list_chat_sessions(workspace, page_offset, page_size)
            .map_err(|e| format!("failed to list sessions: {}", e.code()))?;
        let count = page.len();
        sessions.extend(page);
        if limit <= 200 || sessions.len() >= limit || count < page_size {
            break;
        }
        page_offset = page_offset.saturating_add(count);
    }
    Ok(sessions)
}
fn session_model(session: &SessionSummary) -> String {
    model_label(
        session.model_provider.as_deref(),
        session.model_id.as_deref(),
        session.variant.as_deref(),
    )
}

fn model_label(provider: Option<&str>, model: Option<&str>, variant: Option<&str>) -> String {
    match (provider, model) {
        (Some(provider), Some(model)) => match variant.filter(|v| !v.is_empty()) {
            Some(variant) => format!("{provider}/{model}#{variant}"),
            None => format!("{provider}/{model}"),
        },
        _ => "-".to_string(),
    }
}

fn snapshot_label(snapshot: &ModelSnapshot) -> String {
    model_label(
        Some(snapshot.provider.as_str()),
        Some(snapshot.model.as_str()),
        snapshot.variant.as_deref(),
    )
}

fn single_line(text: &str) -> String {
    text.lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}
fn show(
    ctx: &WorkspaceContext,
    db: &Database,
    session_id: &str,
    tools: bool,
    thinking: bool,
    out: &Output,
) -> Result<(), String> {
    let title = db
        .chat_session_title(&ctx.workspace, session_id)
        .map_err(|e| format!("failed to read session title: {}", e.code()))?
        .unwrap_or_default();
    let model = db
        .chat_session_model(&ctx.workspace, session_id)
        .map_err(|e| format!("failed to read session model: {}", e.code()))?;
    let messages = db
        .load_chat_session(&ctx.workspace, session_id)
        .map_err(|e| format!("failed to load session: {}", e.code()))?
        .ok_or_else(|| format!("session '{session_id}' not found"))?;
    let checkpoint = db
        .active_checkpoint(&ctx.workspace, session_id)
        .map_err(|e| format!("failed to query compact checkpoint: {}", e.code()))?;
    let notices = db
        .load_notices(&ctx.workspace, session_id)
        .map_err(|e| format!("failed to load notices: {}", e.code()))?;
    let value = json!({
        "session_id": session_id, "title": title, "model": model.as_ref().map(snapshot_label),
        "messages": messages.iter().map(|m| message_json(m, tools, thinking)).collect::<Vec<_>>(),
        "checkpoint": checkpoint.as_ref().map(|c| json!({ "id": c.id, "sequence": c.sequence, "start_sequence": c.start_sequence, "end_sequence": c.end_sequence, "model": c.model_label, "summary": c.summary_text })),
        "notices": notices.iter().map(|n| json!({ "kind": n.kind, "target": n.target_user_message_id, "restored": n.restored_file_count })).collect::<Vec<_>>(),
    });
    out.emit(&value, |out, writer| {
        writeln!(
            writer,
            "{} {} ({})",
            out.bold("Session:"),
            session_id,
            if title.is_empty() {
                "-"
            } else {
                title.as_str()
            }
        )?;
        writeln!(
            writer,
            "{} {}",
            out.bold("Model:"),
            model.as_ref().map_or("-".to_string(), snapshot_label)
        )?;
        writeln!(writer, "{} {}", out.bold("Messages:"), messages.len())?;
        match &checkpoint {
            Some(c) => writeln!(
                writer,
                "{} sequence {} ({}..{}, {}) model {}",
                out.bold("Checkpoint:"),
                c.sequence,
                c.start_sequence,
                c.end_sequence,
                c.state,
                c.model_label
            )?,
            None => writeln!(writer, "{} none", out.bold("Checkpoint:"))?,
        }
        for notice in &notices {
            writeln!(
                writer,
                "{} {} ({} files)",
                out.bold("Notice:"),
                notice.kind,
                notice.restored_file_count
            )?;
        }
        for message in &messages {
            render_message(out, writer, message, tools, thinking)?;
        }
        Ok(())
    })
}
fn message_json(message: &ChatMessage, tools: bool, thinking: bool) -> Value {
    let parts = message
        .parts
        .iter()
        .filter(|p| thinking || p.kind != "reasoning")
        .map(|p| json!({ "kind": p.kind, "text": p.content }))
        .collect::<Vec<_>>();
    let calls = message.tools.iter().map(|c| if tools {
        json!({ "call_id": c.call_id, "name": c.name, "status": c.status, "arguments": c.arguments_json, "result": c.result_json, "error": c.error_text })
    } else {
        json!({ "call_id": c.call_id, "name": c.name, "status": c.status })
    }).collect::<Vec<_>>();
    json!({
        "id": message.external_id, "sequence": message.sequence, "role": message.role, "status": message.status,
        "error": message.error_code,
        "model": model_label(message.model_provider.as_deref(), message.model_id.as_deref(), message.variant.as_deref()),
        "parts": parts, "tools": calls,
        "shell": message.user_shell.as_ref().map(|s| json!({ "command": s.command, "state": s.state, "exit_code": s.exit_code })),
    })
}
fn render_message(
    out: &Output,
    writer: &mut dyn Write,
    message: &ChatMessage,
    tools: bool,
    thinking: bool,
) -> std::io::Result<()> {
    writeln!(
        writer,
        "\n{} [{}] {} {}",
        out.bold("Message"),
        message.sequence,
        message.role,
        out.muted(&message.status)
    )?;
    if let Some(shell) = &message.user_shell {
        writeln!(
            writer,
            "  shell: {} [{}]",
            single_line(&shell.command),
            shell.state
        )?;
    }
    for part in &message.parts {
        match part.kind.as_str() {
            "reasoning" if thinking => {
                writeln!(writer, "  thinking: {}", out.muted(&part.content))?;
            }
            "text" => writeln!(writer, "{}", part.content)?,
            _ => {}
        }
    }
    for call in &message.tools {
        write!(writer, "  tool: {} [{}]", call.name, call.status)?;
        if tools {
            writeln!(
                writer,
                "\n    call: {}\n    arguments: {}\n    result: {}{}",
                call.call_id,
                call.arguments_json,
                call.result_json.as_deref().unwrap_or("-"),
                call.error_text
                    .as_ref()
                    .map(|e| format!("\n    error: {e}"))
                    .unwrap_or_default()
            )?;
        } else {
            writeln!(writer)?;
        }
    }
    Ok(())
}
fn agents(
    ctx: &WorkspaceContext,
    db: &Database,
    session_id: &str,
    tree: bool,
    filter: AgentFilterArg,
    out: &Output,
) -> Result<(), String> {
    let records = if tree {
        let (root, _, _) = db
            .resolve_session_tree_root(&ctx.workspace, session_id)
            .map_err(|e| format!("failed to resolve session tree: {}", e.code()))?;
        db.list_subagent_tree(&ctx.workspace, &root)
            .map_err(|e| format!("failed to list subagents: {}", e.code()))?
    } else {
        db.list_subagents(&ctx.workspace, session_id)
            .map_err(|e| format!("failed to list subagents: {}", e.code()))?
    };
    let mut queues: HashMap<String, HashMap<String, SubagentQueueState>> = HashMap::new();
    let mut agents = Vec::new();
    for record in &records {
        let queue = queues
            .entry(record.parent_session_id.clone())
            .or_insert_with(|| {
                db.subagent_queue_states(&ctx.workspace, &record.parent_session_id)
                    .unwrap_or_default()
            })
            .get(&record.agent_id)
            .cloned();
        let status = project_status(record, queue.as_ref());
        if !filter.matches(AgentStatus::parse(&status).unwrap_or(AgentStatus::Completed)) {
            continue;
        }
        agents.push(json!({
            "agent_id": record.agent_id, "nickname": record.nickname, "role": record.role.as_str(),
            "agent_path": record.agent_path, "child_session_id": record.child_session_id, "status": status,
            "title": record.title,
            "model": model_label(Some(record.model_provider.as_str()), Some(record.model_id.as_str()), record.model_variant.as_deref()),
            "files_changed": changed_paths(db, ctx, &record.child_session_id),
        }));
    }
    let value = json!({ "session_id": session_id, "tree": tree, "filter": filter.label(), "agents": &agents });
    out.emit(&value, |out, writer| {
        if agents.is_empty() {
            return writeln!(writer, "{}", out.muted("No subagents found."));
        }
        let table = agents
            .iter()
            .map(|agent| {
                vec![
                    agent["agent_id"].as_str().unwrap_or("-").to_string(),
                    agent["nickname"].as_str().unwrap_or("-").to_string(),
                    agent["role"].as_str().unwrap_or("-").to_string(),
                    agent["agent_path"].as_str().unwrap_or("-").to_string(),
                    agent["status"].as_str().unwrap_or("-").to_string(),
                    agent["title"].as_str().unwrap_or("-").to_string(),
                    agent["files_changed"].to_string(),
                ]
            })
            .collect::<Vec<_>>();
        out.print_table(
            writer,
            &[
                "AGENT", "NICKNAME", "ROLE", "PATH", "STATUS", "TITLE", "FILES",
            ],
            &table,
        )
    })
}

/// 与 `event/chat/subagent` 相同的投影规则：CLI 无内存态，running 以持久状态为准。
fn project_status(record: &SubagentRecord, queue: Option<&SubagentQueueState>) -> String {
    let resumed = record.status != "failed"
        && queue.is_some_and(|q| q.queued_count > 0 && q.head_error_code.is_none());
    let blocked = queue.and_then(|q| q.head_error_code.as_ref()).is_some();
    let projected = crate::chat::agent::projected_agent_status(
        record.status == "running",
        resumed,
        blocked,
        &record.status,
    );
    if projected == AgentStatus::Cancelled
        && queue.and_then(|q| q.last_input_status.as_deref()) == Some("interrupted")
    {
        return "interrupted".to_string();
    }
    projected.as_str().to_string()
}

fn changed_paths(db: &Database, ctx: &WorkspaceContext, child: &str) -> usize {
    let Some(turn) = db.latest_user_turn(&ctx.workspace, child).ok().flatten() else {
        return 0;
    };
    db.count_changed_paths(&ctx.workspace, child, &turn.user_message_id)
        .unwrap_or(0)
}
fn processes(
    ctx: &WorkspaceContext,
    db: &Database,
    session_id: &str,
    out: &Output,
) -> Result<(), String> {
    let records = db
        .load_processes_for_session(&ctx.workspace, session_id)
        .map_err(|e| format!("failed to load processes: {}", e.code()))?;
    let rows = records.iter().map(|r| json!({
        "id": r.external_id, "process_session_id": r.process_session_id, "state": r.state,
        "exit_code": r.exit_code, "command": r.command, "cwd": r.cwd,
        "started_at_ms": r.started_at_ms, "ended_at_ms": r.ended_at_ms, "output_tail": r.output_tail,
    })).collect::<Vec<_>>();
    let value = json!({ "session_id": session_id, "processes": &rows });
    out.emit(&value, |out, writer| {
        if rows.is_empty() {
            return writeln!(writer, "{}", out.muted("No processes found."));
        }
        let table = rows
            .iter()
            .map(|r| {
                vec![
                    r["id"].as_str().unwrap_or("-").to_string(),
                    r["process_session_id"].to_string(),
                    r["state"].as_str().unwrap_or("-").to_string(),
                    r["exit_code"].to_string(),
                    single_line(r["command"].as_str().unwrap_or("-")),
                    r["output_tail"]
                        .as_str()
                        .map(single_line)
                        .unwrap_or_default(),
                ]
            })
            .collect::<Vec<_>>();
        out.print_table(
            writer,
            &["PROCESS", "PID", "STATE", "EXIT", "COMMAND", "TAIL"],
            &table,
        )
    })
}
