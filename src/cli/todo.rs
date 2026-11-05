//! `lx-tui todo` 无头 CLI：全局日程待办的增删改查、状态切换、重排、顺延与统计。
use std::ffi::OsString;
use std::path::Path;

use serde_json::{Value, json};

use crate::cli::args::{GlobalOpts, extract_global_opts, parse_usize_flag, take_opt};
use crate::cli::context::open_global_db;
use crate::cli::output::Output;
use crate::cli::{CliAction, EXIT_OK, HelpTopic};
use crate::db::Database;
use crate::db::todo::{TodoPriority, TodoRecord, TodoStatus, parse_date, shift_date, today_date};

/// `lx-tui todo` 的解析结果。
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct TodoCli {
    pub global: GlobalOpts,
    pub command: TodoCommand,
}

/// 待办领域子命令：`action` 为完整命令路径，参数由各处理器按需校验。
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct TodoCommand {
    action: String,
    words: Vec<String>,
    date: Option<String>,
    all: bool,
    past: bool,
    status: Option<TodoStatus>,
    priority: Option<TodoPriority>,
    text: Option<String>,
    to: Option<String>,
    days: Option<usize>,
    from: Option<String>,
    ids: Vec<String>,
}

type Handler = fn(&Context, &TodoCommand) -> Result<(), String>;

/// 每个子命令对应的处理器。
const COMMANDS: &[(&str, usize, Handler)] = &[
    ("todo list", 0, list),
    ("todo add", usize::MAX, add),
    ("todo update", 1, update),
    ("todo done", 1, done),
    ("todo start", 1, start),
    ("todo reopen", 1, reopen),
    ("todo delete", 1, delete),
    ("todo reorder", usize::MAX, reorder),
    ("todo rollover", 0, rollover),
    ("todo stats", 0, stats),
];

/// 解析 `lx-tui todo ...` 的剩余参数（不含 `todo` 本身）。
pub fn parse(raw_args: &[OsString]) -> Result<CliAction, String> {
    let (global, rest, help_requested) = extract_global_opts(raw_args)?;
    if help_requested {
        return Ok(CliAction::Help(HelpTopic::Todo));
    }
    let head = rest.first().ok_or_else(missing_subcommand)?;
    if head == "help" {
        return Ok(CliAction::Help(HelpTopic::Todo));
    }
    let action = format!("todo {head}");
    let arity = COMMANDS
        .iter()
        .find(|(name, _, _)| *name == action)
        .map(|(_, arity, _)| *arity)
        .ok_or_else(|| unexpected(head, "todo"))?;
    Ok(CliAction::Todo(TodoCli {
        global,
        command: TodoCommand::parse(&action, &rest[1..], arity)?,
    }))
}

impl TodoCommand {
    fn parse(action: &str, args: &[String], arity: usize) -> Result<Self, String> {
        let mut command = Self {
            action: action.to_string(),
            ..Self::default()
        };
        let mut index = 0;
        while index < args.len() {
            let arg = args[index].as_str();
            if let Some(value) = take_opt(args, &mut index, "", "--date")? {
                command.date = Some(value);
            } else if arg == "--all" {
                command.all = true;
            } else if arg == "--past-incomplete" {
                command.past = true;
            } else if let Some(value) = take_opt(args, &mut index, "", "--status")? {
                command.status = Some(parse_status(&value)?);
            } else if let Some(value) = take_opt(args, &mut index, "-p", "--priority")? {
                command.priority = Some(parse_priority(&value)?);
            } else if let Some(value) = take_opt(args, &mut index, "", "--text")? {
                command.text = Some(value);
            } else if let Some(value) = take_opt(args, &mut index, "", "--to")? {
                command.to = Some(value);
            } else if let Some(value) = take_opt(args, &mut index, "", "--id")? {
                command.ids.push(value);
            } else if let Some(value) = take_opt(args, &mut index, "", "--days")? {
                command.days = Some(parse_usize_flag(&value, "--days")?);
            } else if let Some(value) = take_opt(args, &mut index, "", "--from")? {
                command.from = Some(value);
            } else if arg.starts_with('-') && arg != "-" {
                return Err(unexpected(arg, action));
            } else {
                command.words.push(arg.to_string());
            }
            index += 1;
        }
        if command.words.len() > arity {
            return Err(unexpected(&command.words[arity], action));
        }
        Ok(command)
    }

    fn text(&self) -> Option<String> {
        let joined = self.words.join(" ");
        (!joined.trim().is_empty()).then(|| joined.trim().to_string())
    }
}

fn parse_status(raw: &str) -> Result<TodoStatus, String> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "todo" => Ok(TodoStatus::Todo),
        "in_progress" => Ok(TodoStatus::InProgress),
        "done" => Ok(TodoStatus::Done),
        other => Err(format!(
            "invalid '--status' value '{other}'; expected 'todo', 'in_progress', or 'done'"
        )),
    }
}

fn parse_priority(raw: &str) -> Result<TodoPriority, String> {
    match raw.trim().to_ascii_uppercase().as_str() {
        "P0" => Ok(TodoPriority::P0),
        "P1" => Ok(TodoPriority::P1),
        "P2" => Ok(TodoPriority::P2),
        "P3" => Ok(TodoPriority::P3),
        other => Err(format!(
            "invalid '--priority' value '{other}'; expected 'P0', 'P1', 'P2', or 'P3'"
        )),
    }
}

fn date_value(raw: &str, flag: &str) -> Result<String, String> {
    let date = raw.trim();
    let shape = date.len() == 10
        && date.as_bytes().get(4) == Some(&b'-')
        && date.as_bytes().get(7) == Some(&b'-');
    if shape && parse_date(date).is_some() {
        Ok(date.to_string())
    } else {
        Err(format!(
            "invalid date '{raw}' for '{flag}'; expected YYYY-MM-DD"
        ))
    }
}

fn missing_subcommand() -> String {
    "todo requires a subcommand; run 'lx-tui todo --help' for usage".to_string()
}

fn unexpected(arg: &str, action: &str) -> String {
    format!("unexpected argument '{arg}'; run 'lx-tui {action} --help' for usage")
}

/// 执行已解析的 todo 命令。
pub fn run(
    cli: TodoCli,
    linked_worktree: Option<&Path>,
) -> Result<i32, Box<dyn std::error::Error>> {
    execute(cli, linked_worktree).map_err(Into::into)
}

fn execute(cli: TodoCli, linked_worktree: Option<&Path>) -> Result<i32, String> {
    let db = open_global_db(cli.global.cwd.as_deref(), linked_worktree)?;
    let ctx = Context {
        db,
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

struct Context {
    db: Database,
    out: Output,
}

fn list(ctx: &Context, command: &TodoCommand) -> Result<(), String> {
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

fn add(ctx: &Context, command: &TodoCommand) -> Result<(), String> {
    let text = command
        .text()
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
    action(
        &ctx.out,
        "add",
        &record.id,
        format!("Added todo {} on {date}.", record.id),
    )
}

fn update(ctx: &Context, command: &TodoCommand) -> Result<(), String> {
    let query = one_id(command)?;
    let todos = db(ctx.db.list_todos(), "failed to list todos")?;
    let mut record = resolve(&todos, &query)?.clone();
    if command.text.is_none()
        && command.status.is_none()
        && command.priority.is_none()
        && command.date.is_none()
    {
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

fn done(ctx: &Context, command: &TodoCommand) -> Result<(), String> {
    set_status(ctx, command, TodoStatus::Done)
}

fn start(ctx: &Context, command: &TodoCommand) -> Result<(), String> {
    set_status(ctx, command, TodoStatus::InProgress)
}

fn reopen(ctx: &Context, command: &TodoCommand) -> Result<(), String> {
    set_status(ctx, command, TodoStatus::Todo)
}

fn set_status(ctx: &Context, command: &TodoCommand, status: TodoStatus) -> Result<(), String> {
    let query = one_id(command)?;
    let todos = db(ctx.db.list_todos(), "failed to list todos")?;
    let mut record = resolve(&todos, &query)?.clone();
    record.status = status;
    record.completed_date = (status == TodoStatus::Done).then(today_date);
    db(ctx.db.save_todo(&record), "failed to update todo")?;
    action(
        &ctx.out,
        "status",
        &record.id,
        format!("Todo {} is now {}.", record.id, status.as_str()),
    )
}

fn delete(ctx: &Context, command: &TodoCommand) -> Result<(), String> {
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

fn reorder(ctx: &Context, command: &TodoCommand) -> Result<(), String> {
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
        writeln!(
            writer,
            "Reordered {} todos on {}.",
            payload["ids"].as_array().map_or(0, Vec::len),
            payload["date"].as_str().unwrap_or("")
        )
    })
}

fn rollover(ctx: &Context, command: &TodoCommand) -> Result<(), String> {
    let target = match command.to.as_deref() {
        Some(raw) => date_value(raw, "--to")?,
        None => today_date(),
    };
    let mut ids = Vec::with_capacity(command.ids.len());
    if command.ids.is_empty() {
        let past = db(
            ctx.db.list_past_incomplete_todos(&target),
            "failed to list past todos",
        )?;
        ids.extend(past.into_iter().map(|todo| todo.id));
    } else {
        let todos = db(ctx.db.list_todos(), "failed to list todos")?;
        for query in &command.ids {
            ids.push(resolve(&todos, query)?.id.clone());
        }
    }
    db(
        ctx.db.rollover_todos_to_date(&ids, &target),
        "failed to roll over todos",
    )?;
    let payload = json!({ "action": "rollover", "to": target, "count": ids.len(), "ids": ids });
    ctx.out.emit(&payload, |_, writer| {
        writeln!(
            writer,
            "Rolled over {} todos to {}.",
            payload["count"],
            payload["to"].as_str().unwrap_or("")
        )
    })
}

fn stats(ctx: &Context, command: &TodoCommand) -> Result<(), String> {
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
        writeln!(
            writer,
            "Status: todo {}, in_progress {}, done {}",
            status_counts[0], status_counts[1], status_counts[2]
        )?;
        writeln!(
            writer,
            "Priority: P0 {}, P1 {}, P2 {}, P3 {}",
            priority_counts[0], priority_counts[1], priority_counts[2], priority_counts[3]
        )?;
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

fn range(command: &TodoCommand) -> Result<(String, String), String> {
    if command.days.is_some() && (command.from.is_some() || command.to.is_some()) {
        return Err("cannot combine '--days' with '--from' or '--to'".to_string());
    }
    if let Some(days) = command.days {
        if days == 0 {
            return Err("'--days' must be greater than 0".to_string());
        }
        let to = today_date();
        let back = i32::try_from(days - 1).map_err(|_| "'--days' is too large".to_string())?;
        return Ok((shift_date(&to, -back), to));
    }
    match (command.from.as_deref(), command.to.as_deref()) {
        (Some(from), Some(to)) => {
            let from = date_value(from, "--from")?;
            let to = date_value(to, "--to")?;
            if from.as_str() > to.as_str() {
                return Err("'--from' must not be after '--to'".to_string());
            }
            Ok((from, to))
        }
        (None, None) => {
            let to = today_date();
            Ok((shift_date(&to, -6), to))
        }
        _ => Err("both '--from' and '--to' must be provided together".to_string()),
    }
}

fn one_id(command: &TodoCommand) -> Result<String, String> {
    match command.words.as_slice() {
        [id] if !id.trim().is_empty() => Ok(id.trim().to_string()),
        [] => Err("missing <TODO_ID>; run 'lx-tui todo --help' for usage".to_string()),
        _ => Err("expected exactly one <TODO_ID>".to_string()),
    }
}

fn resolve<'a>(todos: &'a [TodoRecord], query: &str) -> Result<&'a TodoRecord, String> {
    if let Some(exact) = todos.iter().find(|todo| todo.id == query) {
        return Ok(exact);
    }
    let matches: Vec<&TodoRecord> = todos
        .iter()
        .filter(|todo| todo.id.starts_with(query))
        .collect();
    match matches.as_slice() {
        [single] => Ok(*single),
        [] => Err(format!("todo '{query}' not found")),
        many => Err(format!(
            "todo prefix '{query}' is ambiguous (matches: {})",
            many.iter()
                .map(|todo| todo.id.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        )),
    }
}

fn todo_json(todo: &TodoRecord) -> Value {
    json!({
        "id": todo.id, "date": todo.entry_date, "text": todo.text,
        "status": todo.status.as_str(), "priority": todo.priority.as_str(),
        "completed_date": todo.completed_date, "position": todo.position,
    })
}

fn action(out: &Output, name: &str, id: &str, message: String) -> Result<(), String> {
    let payload = json!({ "action": name, "id": id, "message": message.clone() });
    out.emit(&payload, |_, writer| writeln!(writer, "{message}"))
}

fn db<T>(result: crate::db::Result<T>, context: &str) -> Result<T, String> {
    result.map_err(|error| format!("{context}: {}", error.code()))
}
