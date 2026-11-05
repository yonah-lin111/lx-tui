//! `lx-tui todo` 子命令执行与输出格式化。

use std::path::Path;

use serde_json::json;

use crate::cli::EXIT_OK;
use crate::cli::context::open_global_db;
use crate::cli::output::Output;
use crate::db::Database;
use crate::db::todo::{TodoPriority, TodoRecord, TodoStatus, today_date};

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
        record.status = status;
        record.completed_date = (status == TodoStatus::Done).then(today_date);
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
        }
    }
    db(ctx.db.save_todo(&record), "failed to update todo")?;
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
    db(ctx.db.save_todo(&record), "failed to update todo")?;
    let message = format!("Todo {} is now {}.", record.id, status.as_str());
    action(&ctx.out, "status", &record.id, message)
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
            ids.push(item.id.clone());
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
