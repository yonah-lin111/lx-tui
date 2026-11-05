//! `lx-tui todo` 无头 CLI：全局日程待办的增删改查、状态切换、重排、顺延与统计。
use std::ffi::OsString;

use serde_json::{Value, json};

use crate::cli::args::{GlobalOpts, extract_global_opts, parse_usize_flag, take_opt};
use crate::cli::output::Output;
use crate::cli::{CliAction, HelpTopic};
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
        let mut positional_only = false;
        while index < args.len() {
            let arg = args[index].as_str();
            if positional_only {
                command.words.push(arg.to_string());
            } else if arg == "--" {
                positional_only = true;
            } else if let Some(value) = take_opt(args, &mut index, "", "--date")? {
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

fn missing_subcommand() -> String {
    "todo requires a subcommand; run 'lx-tui todo --help' for usage".to_string()
}

fn unexpected(arg: &str, action: &str) -> String {
    format!("unexpected argument '{arg}'; run 'lx-tui {action} --help' for usage")
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

/// 执行已解析的 todo 命令。
mod exec;
pub use exec::run;
use exec::*;
