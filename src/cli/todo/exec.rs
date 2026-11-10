//! `lx-tui todo` 子命令执行与输出格式化。

use std::path::Path;

use serde_json::json;

use crate::cli::EXIT_OK;
use crate::cli::context::open_global_db;
use crate::cli::output::Output;
use crate::db::Database;
use crate::db::todo::{TodoPriority, TodoRecord, TodoStatus, new_todo_group_id, today_date};

use super::*;

pub fn run(
    cli: TodoCli,
    linked_worktree: Option<&Path>,
) -> Result<i32, Box<dyn std::error::Error>> {
    execute(cli, linked_worktree).map_err(Into::into)
}

fn execute(cli: TodoCli, linked_worktree: Option<&Path>) -> Result<i32, String> {
    let ctx = Context {
        db: open_global_db(cli.global.cwd.as_deref(), linked_worktree)?,
        out: Output::new(cli.global.json, cli.global.color),
    };
    let handler = COMMANDS
        .iter()
        .find(|(name, _, _)| *name == cli.command.action)
        .map(|(_, _, handler)| *handler)
        .ok_or_else(|| format!("unknown todo command '{}'", cli.command.action))?;
    handler(&ctx, &cli.command)?;
    Ok(EXIT_OK)
}

pub(super) struct Context {
    db: Database,
    out: Output,
}

pub(super) fn list(ctx: &Context, command: &TodoCommand) -> Result<(), String> {
    let today = today_date();
    let selected =
        usize::from(command.date.is_some()) + usize::from(command.all) + usize::from(command.past);
    if selected > 1 {
        return Err("cannot combine '--date', '--all', and '--past-incomplete'".to_string());
    }
    let (scope, date, mut todos) = if command.all {
        (
            "all",
            None,
            db(ctx.db.list_todos(), "failed to list todos")?,
        )
    } else if command.past {
        (
            "past_incomplete",
            None,
            db(
                ctx.db.list_past_incomplete_todos(&today),
                "failed to list past todos",
            )?,
        )
    } else {
        let date = match command.date.as_deref() {
            Some(raw) => date_value(raw, "--date")?,
            None => today,
        };
        let records = db(
            ctx.db.list_todos_by_date(&date),
            "failed to list todos by date",
        )?;
        ("day", Some(date), records)
    };
    if let Some(status) = command.status {
        todos.retain(|todo| todo.status == status);
    }
    if let Some(priority) = command.priority {
        todos.retain(|todo| todo.priority == priority);
    }
    let payload = json!({
        "scope": scope,
        "date": date,
        "count": todos.len(),
        "todos": todos.iter().map(todo_json).collect::<Vec<_>>(),
    });
    ctx.out.emit(&payload, |out, writer| {
        if todos.is_empty() {
            return writeln!(writer, "No todos found.");
        }
        writeln!(writer, "{}", out.bold("ID  DATE  PRIORITY  STATUS  TEXT"))?;
        for todo in &todos {
            writeln!(
                writer,
                "{}  {}  {}  {}  {}",
                todo.id,
                todo.entry_date,
                todo.priority.as_str(),
                todo.status.as_str(),
                todo.text
            )?;
        }
        Ok(())
    })
}

pub(super) fn add(ctx: &Context, command: &TodoCommand) -> Result<(), String> {
    let text = command
        .text()
        .or_else(|| command.text.as_deref().map(str::trim).map(str::to_string))
        .filter(|text| !text.is_empty())
        .ok_or_else(|| "missing <TEXT>; run 'lx-tui todo add --help' for usage".to_string())?;
    let date = match command.date.as_deref() {
        Some(raw) => date_value(raw, "--date")?,
        None => today_date(),
    };
    let status = command.status.unwrap_or(TodoStatus::Todo);
    let priority = command.priority.unwrap_or(TodoPriority::P1);
    let position = db(
        ctx.db.list_todos_by_date(&date),
        "failed to list todos by date",
    )?
    .len();
    let record = TodoRecord::new_on(date.clone(), text, status, priority, position as i64);
    db(ctx.db.save_todo(&record), "failed to create todo")?;
    let message = format!("Added todo {} on {date}.", record.id);
    action(&ctx.out, "add", &record.id, message)
}

pub(super) fn update(ctx: &Context, command: &TodoCommand) -> Result<(), String> {
    let query = one_id(command)?;
    let todos = db(ctx.db.list_todos(), "failed to list todos")?;
    let mut record = resolve(&todos, &query)?.clone();
    let changed = command.text.is_some()
        || command.status.is_some()
        || command.priority.is_some()
        || command.date.is_some();
    if !changed {
        return Err(
            "no changes specified; pass '--text', '--status', '--priority', or '--date'"
                .to_string(),
        );
    }
    if let Some(text) = &command.text {
        let trimmed = text.trim();
        if trimmed.is_empty() {
            return Err("'--text' must not be empty".to_string());
        }
        record.text = trimmed.to_string();
    }
    if let Some(status) = command.status {
        if status == TodoStatus::Done {
            if record.status != TodoStatus::Done || record.completed_date.is_none() {
                record.completed_date = Some(today_date());
            }
        } else {
            record.completed_date = None;
        }
        record.status = status;
    }
    if let Some(priority) = command.priority {
        record.priority = priority;
    }
    if let Some(raw) = &command.date {
        let date = date_value(raw, "--date")?;
        if date != record.entry_date {
            record.position = db(
                ctx.db.list_todos_by_date(&date),
                "failed to list todos by date",
            )?
            .len() as i64;
            record.entry_date = date;
            record.group_id = None;
        }
    }
    db(ctx.db.save_todo(&record), "failed to update todo")?;
    if let (Some(status), Some(gid)) = (command.status, record.group_id.as_ref()) {
        sync_group_status(
            ctx,
            &record.entry_date,
            gid,
            status,
            record.completed_date.as_deref(),
        )?;
    }
    action(
        &ctx.out,
        "update",
        &record.id,
        format!("Updated todo {}.", record.id),
    )
}

pub(super) fn done(ctx: &Context, command: &TodoCommand) -> Result<(), String> {
    set_status(ctx, command, TodoStatus::Done)
}

pub(super) fn start(ctx: &Context, command: &TodoCommand) -> Result<(), String> {
    set_status(ctx, command, TodoStatus::InProgress)
}

pub(super) fn reopen(ctx: &Context, command: &TodoCommand) -> Result<(), String> {
    set_status(ctx, command, TodoStatus::Todo)
}

fn set_status(ctx: &Context, command: &TodoCommand, status: TodoStatus) -> Result<(), String> {
    let query = one_id(command)?;
    let todos = db(ctx.db.list_todos(), "failed to list todos")?;
    let mut record = resolve(&todos, &query)?.clone();
    if status == TodoStatus::Done {
        if record.status != TodoStatus::Done || record.completed_date.is_none() {
            record.completed_date = Some(today_date());
        }
    } else {
        record.completed_date = None;
    }
    record.status = status;
    if let Some(ref gid) = record.group_id {
        sync_group_status(
            ctx,
            &record.entry_date,
            gid,
            status,
            record.completed_date.as_deref(),
        )?;
    } else {
        db(ctx.db.save_todo(&record), "failed to update todo")?;
    }
    let message = format!("Todo {} is now {}.", record.id, status.as_str());
    action(&ctx.out, "status", &record.id, message)
}

fn sync_group_status(
    ctx: &Context,
    entry_date: &str,
    group_id: &str,
    status: TodoStatus,
    completed_date: Option<&str>,
) -> Result<(), String> {
    let mut day_todos = db(
        ctx.db.list_todos_by_date(entry_date),
        "failed to list todos by date",
    )?;
    let target_completed = if status == TodoStatus::Done {
        Some(
            completed_date
                .map(str::to_string)
                .unwrap_or_else(today_date),
        )
    } else {
        None
    };
    for item in &mut day_todos {
        if item.group_id.as_deref() == Some(group_id) {
            item.status = status;
            item.completed_date = target_completed.clone();
        }
    }
    db(
        ctx.db.sync_date_todos(entry_date, &day_todos),
        "failed to update todo",
    )
}

pub(super) fn group(ctx: &Context, command: &TodoCommand) -> Result<(), String> {
    let queries: Vec<&str> = command
        .ids
        .iter()
        .chain(command.words.iter())
        .map(String::as_str)
        .filter(|s| !s.trim().is_empty())
        .collect();
    if queries.len() < 2 {
        return Err("todo group requires at least two todo IDs".to_string());
    }
    let todos = db(ctx.db.list_todos(), "failed to list todos")?;
    let mut resolved_ids: Vec<String> = Vec::with_capacity(queries.len());
    let mut entry_date: Option<String> = None;
    for query in queries {
        let item = resolve(&todos, query.trim())?;
        if let Some(ref date) = entry_date {
            if &item.entry_date != date {
                return Err("cannot group todos from different dates".to_string());
            }
        } else {
            entry_date = Some(item.entry_date.clone());
        }
        if !resolved_ids.contains(&item.id) {
            resolved_ids.push(item.id.clone());
        }
    }
    if resolved_ids.len() < 2 {
        return Err("todo group requires at least two distinct todo IDs".to_string());
    }
    let entry_date = entry_date.unwrap_or_else(today_date);
    let mut day_todos = db(
        ctx.db.list_todos_by_date(&entry_date),
        "failed to list todos by date",
    )?;
    let first = day_todos
        .iter()
        .find(|item| item.id == resolved_ids[0])
        .ok_or_else(|| format!("todo '{}' not found", resolved_ids[0]))?
        .clone();
    let group_id = first.group_id.clone().unwrap_or_else(new_todo_group_id);
    let base_status = first.status;
    let base_completed = (base_status == TodoStatus::Done)
        .then(|| first.completed_date.clone().unwrap_or_else(today_date));

    let mut target_member_ids: Vec<String> = Vec::new();
    if let Some(ref existing_gid) = first.group_id {
        for item in &day_todos {
            if item.group_id.as_deref() == Some(existing_gid.as_str()) {
                target_member_ids.push(item.id.clone());
            }
        }
    } else {
        target_member_ids.push(first.id.clone());
    }
    for id in &resolved_ids[1..] {
        if !target_member_ids.contains(id) {
            target_member_ids.push(id.clone());
        }
        if let Some(other_gid) = day_todos
            .iter()
            .find(|item| &item.id == id)
            .and_then(|item| item.group_id.clone())
        {
            for item in &day_todos {
                if item.group_id.as_deref() == Some(other_gid.as_str())
                    && !target_member_ids.contains(&item.id)
                {
                    target_member_ids.push(item.id.clone());
                }
            }
        }
    }
    if target_member_ids.len() < 2 {
        return Err("todo group requires at least two distinct todo IDs".to_string());
    }

    let anchor_id = &target_member_ids[0];
    let anchor_index = day_todos
        .iter()
        .position(|item| &item.id == anchor_id)
        .unwrap_or(0);
    let before_count = day_todos[..anchor_index]
        .iter()
        .filter(|item| !target_member_ids.contains(&item.id))
        .count();

    let mut grouped_members: Vec<TodoRecord> = Vec::with_capacity(target_member_ids.len());
    for member_id in &target_member_ids {
        if let Some(pos) = day_todos.iter().position(|item| &item.id == member_id) {
            let mut item = day_todos.remove(pos);
            item.group_id = Some(group_id.clone());
            item.status = base_status;
            item.completed_date = base_completed.clone();
            grouped_members.push(item);
        }
    }
    let insert_at = before_count.min(day_todos.len());
    for (offset, member) in grouped_members.into_iter().enumerate() {
        day_todos.insert(insert_at + offset, member);
    }
    for index in 0..day_todos.len() {
        if let Some(gid) = day_todos[index].group_id.clone() {
            let count = day_todos
                .iter()
                .filter(|other| other.group_id.as_deref() == Some(gid.as_str()))
                .count();
            if count < 2 {
                day_todos[index].group_id = None;
            }
        }
    }

    db(
        ctx.db.sync_date_todos(&entry_date, &day_todos),
        "failed to group todos",
    )?;
    let payload = json!({
        "action": "group",
        "date": entry_date,
        "group_id": group_id,
        "ids": target_member_ids,
    });
    ctx.out.emit(&payload, |_, writer| {
        writeln!(
            writer,
            "Grouped {} todos into {group_id}.",
            target_member_ids.len()
        )
    })
}

pub(super) fn ungroup(ctx: &Context, command: &TodoCommand) -> Result<(), String> {
    let queries: Vec<String> = command
        .words
        .iter()
        .chain(command.ids.iter())
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .collect();
    let query = match queries.as_slice() {
        [id] => id.clone(),
        [] => return Err("missing <TODO_ID>; run 'lx-tui todo --help' for usage".to_string()),
        _ => return Err("expected a single <TODO_ID>".to_string()),
    };
    let todos = db(ctx.db.list_todos(), "failed to list todos")?;
    let target = resolve(&todos, &query)?.clone();
    let Some(group_id) = target.group_id.clone() else {
        return Err(format!("todo '{}' is not in a group", target.id));
    };
    let mut day_todos = db(
        ctx.db.list_todos_by_date(&target.entry_date),
        "failed to list todos by date",
    )?;
    if command.all {
        for item in &mut day_todos {
            if item.group_id.as_deref() == Some(group_id.as_str()) {
                item.group_id = None;
            }
        }
    } else if let Some(idx) = day_todos.iter().position(|item| item.id == target.id) {
        let group_end = day_todos
            .iter()
            .rposition(|item| item.group_id.as_deref() == Some(group_id.as_str()))
            .unwrap_or(idx);
        let mut detached = day_todos.remove(idx);
        detached.group_id = None;
        let insert_at = group_end.min(day_todos.len());
        day_todos.insert(insert_at, detached);
        let remaining = day_todos
            .iter()
            .filter(|item| item.group_id.as_deref() == Some(group_id.as_str()))
            .count();
        if remaining < 2 {
            for item in &mut day_todos {
                if item.group_id.as_deref() == Some(group_id.as_str()) {
                    item.group_id = None;
                }
            }
        }
    }
    db(
        ctx.db.sync_date_todos(&target.entry_date, &day_todos),
        "failed to ungroup todo",
    )?;
    let payload = json!({
        "action": "ungroup",
        "id": target.id,
        "group_id": group_id,
        "all": command.all,
    });
    ctx.out.emit(&payload, |_, writer| {
        if command.all {
            writeln!(writer, "Ungrouped all todos in {group_id}.")
        } else {
            writeln!(writer, "Split todo {} from {group_id}.", target.id)
        }
    })
}

pub(super) fn delete(ctx: &Context, command: &TodoCommand) -> Result<(), String> {
    let query = one_id(command)?;
    let todos = db(ctx.db.list_todos(), "failed to list todos")?;
    let record = resolve(&todos, &query)?;
    db(ctx.db.delete_todo(&record.id), "failed to delete todo")?;
    action(
        &ctx.out,
        "delete",
        &record.id,
        format!("Deleted todo {}.", record.id),
    )
}

pub(super) fn reorder(ctx: &Context, command: &TodoCommand) -> Result<(), String> {
    let date = command
        .date
        .as_deref()
        .ok_or_else(|| "missing '--date <YYYY-MM-DD>'".to_string())?;
    let date = date_value(date, "--date")?;
    if command.words.is_empty() {
        return Err("missing todo IDs; pass at least one <TODO_ID>".to_string());
    }
    let todos = db(
        ctx.db.list_todos_by_date(&date),
        "failed to list todos by date",
    )?;
    let mut ids = Vec::with_capacity(command.words.len());
    for query in &command.words {
        ids.push(resolve(&todos, query)?.id.clone());
    }
    db(ctx.db.reorder_todos(&date, &ids), "failed to reorder todos")?;
    let payload = json!({ "action": "reorder", "date": date, "ids": ids });
    ctx.out.emit(&payload, |_, writer| {
        writeln!(writer, "Reordered {} todos on {date}.", ids.len())
    })
}

pub(super) fn rollover(ctx: &Context, command: &TodoCommand) -> Result<(), String> {
    let target = match command.to.as_deref() {
        Some(raw) => date_value(raw, "--to")?,
        None => today_date(),
    };
    let ids = if command.ids.is_empty() {
        db(
            ctx.db.list_past_incomplete_todos(&target),
            "failed to list past todos",
        )?
        .into_iter()
        .map(|todo| todo.id)
        .collect::<Vec<_>>()
    } else {
        let todos = db(ctx.db.list_todos(), "failed to list todos")?;
        let mut ids = Vec::with_capacity(command.ids.len());
        for query in &command.ids {
            let item = resolve(&todos, query)?;
            if item.status == TodoStatus::Done {
                let id = &item.id;
                return Err(format!(
                    "cannot roll over completed todo {id}; reopen it first"
                ));
            }
            if let Some(ref gid) = item.group_id {
                for member in &todos {
                    if member.entry_date == item.entry_date
                        && member.group_id.as_deref() == Some(gid.as_str())
                        && member.status != TodoStatus::Done
                        && !ids.contains(&member.id)
                    {
                        ids.push(member.id.clone());
                    }
                }
            } else if !ids.contains(&item.id) {
                ids.push(item.id.clone());
            }
        }
        ids
    };
    db(
        ctx.db.rollover_todos_to_date(&ids, &target),
        "failed to roll over todos",
    )?;
    let payload = json!({ "action": "rollover", "to": target, "count": ids.len(), "ids": ids });
    ctx.out.emit(&payload, |_, writer| {
        writeln!(writer, "Rolled over {} todos to {target}.", ids.len())
    })
}

pub(super) fn stats(ctx: &Context, command: &TodoCommand) -> Result<(), String> {
    let (from, to) = range(command)?;
    let days = db(
        ctx.db.query_range_stats(&from, &to),
        "failed to query stats",
    )?;
    let all = db(ctx.db.list_todos(), "failed to list todos")?;
    let mut status_counts = [0usize; 3];
    let mut priority_counts = [0usize; 4];
    for todo in all.iter().filter(|todo| {
        todo.entry_date.as_str() >= from.as_str() && todo.entry_date.as_str() <= to.as_str()
    }) {
        status_counts[match todo.status {
            TodoStatus::Todo => 0,
            TodoStatus::InProgress => 1,
            TodoStatus::Done => 2,
        }] += 1;
        priority_counts[match todo.priority {
            TodoPriority::P0 => 0,
            TodoPriority::P1 => 1,
            TodoPriority::P2 => 2,
            TodoPriority::P3 => 3,
        }] += 1;
    }
    let planned: usize = days.iter().map(|day| day.planned_count).sum();
    let completed: usize = days.iter().map(|day| day.completed_count).sum();
    let percent = if planned == 0 {
        0.0
    } else {
        completed as f64 * 100.0 / planned as f64
    };
    let payload = json!({
        "from": from,
        "to": to,
        "planned": planned,
        "completed": completed,
        "completion_percent": percent,
        "by_status": { "todo": status_counts[0], "in_progress": status_counts[1], "done": status_counts[2] },
        "by_priority": { "p0": priority_counts[0], "p1": priority_counts[1], "p2": priority_counts[2], "p3": priority_counts[3] },
        "days": days.iter().map(|day| json!({
            "date": day.date, "planned": day.planned_count, "completed": day.completed_count,
        })).collect::<Vec<_>>(),
    });
    ctx.out.emit(&payload, |_, writer| {
        writeln!(writer, "Todo statistics: {from} .. {to}")?;
        writeln!(
            writer,
            "Planned: {planned}  Completed: {completed}  Completion: {percent:.1}%"
        )?;
        let [todo, progress, done] = status_counts;
        writeln!(
            writer,
            "Status: todo {todo}, in_progress {progress}, done {done}"
        )?;
        let [p0, p1, p2, p3] = priority_counts;
        writeln!(writer, "Priority: P0 {p0}, P1 {p1}, P2 {p2}, P3 {p3}")?;
        writeln!(writer, "DATE  COMPLETED/PLANNED")?;
        for day in &days {
            writeln!(
                writer,
                "{}  {}/{}",
                day.date, day.completed_count, day.planned_count
            )?;
        }
        Ok(())
    })
}
