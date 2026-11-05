//! `lx-tui prompt` 无头 CLI：提示词条目、文件夹、分页与引用目录管理。
use std::ffi::OsString;
use std::path::{Path, PathBuf};

use serde_json::{Value, json};

use crate::cli::args::{ContentInput, GlobalOpts, extract_global_opts, parse_i64_flag, take_opt};
use crate::cli::context::WorkspaceContext;
use crate::cli::output::Output;
use crate::cli::{CliAction, EXIT_OK, HelpTopic};
use crate::db::{Database, FolderRecord, PromptPageRecord, PromptRecord};

/// `lx-tui prompt` 的解析结果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PromptCli {
    pub global: GlobalOpts,
    pub command: PromptCommand,
}

/// 提示词领域子命令：`action` 为完整命令路径，参数由各处理器按需校验。
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct PromptCommand {
    action: String,
    words: Vec<String>,
    folder: Option<i64>,
    parent: Option<i64>,
    page: Option<String>,
    page_name: Option<String>,
    root: bool,
    content: ContentInput,
}

type Handler = fn(&Context, &PromptCommand) -> Result<(), String>;

/// 每个子命令对应的处理器；位置参数由各处理器按需校验。
const COMMANDS: &[(&str, usize, Handler)] = &[
    ("prompt list", 0, list),
    ("prompt show", 1, show),
    ("prompt create", 1, create),
    ("prompt edit", 1, edit),
    ("prompt rename", 2, rename_prompt),
    ("prompt move", 1, move_prompt),
    ("prompt delete", 1, delete_prompt),
    ("prompt folder list", 0, folder_list),
    ("prompt folder create", 1, folder_create),
    ("prompt folder rename", 2, folder_rename),
    ("prompt folder move", 1, folder_move),
    ("prompt folder delete", 1, folder_delete),
    ("prompt page list", 1, page_list),
    ("prompt page create", 2, page_create),
    ("prompt page edit", 1, page_edit),
    ("prompt page rename", 2, page_rename),
    ("prompt page active", 2, page_active),
    ("prompt page delete", 1, page_delete),
    ("prompt ref list", 0, ref_list),
    ("prompt ref add", 1, ref_add),
    ("prompt ref remove", 1, ref_remove),
];

/// 解析 `lx-tui prompt ...` 的剩余参数（不含 `prompt` 本身）。
pub fn parse(raw_args: &[OsString]) -> Result<CliAction, String> {
    let (global, rest, help_requested) = extract_global_opts(raw_args)?;
    if help_requested {
        return Ok(CliAction::Help(HelpTopic::Prompt));
    }
    let head = rest.first().ok_or_else(|| missing_subcommand("prompt"))?;
    if head == "help" {
        return Ok(CliAction::Help(HelpTopic::Prompt));
    }
    let (action, tail) = match head.as_str() {
        "folder" | "page" | "ref" => {
            let sub = rest
                .get(1)
                .ok_or_else(|| missing_subcommand(&format!("prompt {head}")))?;
            (format!("prompt {head} {sub}"), &rest[2..])
        }
        other => (format!("prompt {other}"), &rest[1..]),
    };
    let arity = COMMANDS
        .iter()
        .find(|(name, _, _)| *name == action)
        .map(|(_, arity, _)| *arity)
        .ok_or_else(|| unexpected(action.trim_start_matches("prompt "), "prompt"))?;
    Ok(CliAction::Prompt(PromptCli {
        global,
        command: PromptCommand::parse(&action, tail, arity)?,
    }))
}

impl PromptCommand {
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
            } else if let Some(value) = take_opt(args, &mut index, "", "--folder")? {
                command.folder = Some(parse_id(&value, "--folder")?);
            } else if let Some(value) = take_opt(args, &mut index, "", "--parent")? {
                command.parent = Some(parse_id(&value, "--parent")?);
            } else if let Some(value) = take_opt(args, &mut index, "", "--page")? {
                command.page = Some(value);
            } else if let Some(value) = take_opt(args, &mut index, "", "--page-name")? {
                command.page_name = Some(non_empty(&value, "PAGE_NAME")?);
            } else if command.content.try_take(args, &mut index)? {
            } else if arg == "--root" {
                command.root = true;
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

    fn word(&self, index: usize, label: &str) -> Result<String, String> {
        let value = self.words.get(index).ok_or_else(|| {
            format!(
                "missing <{label}>; run 'lx-tui {} --help' for usage",
                self.action
            )
        })?;
        non_empty(value, label)
    }

    fn id(&self, index: usize, label: &str) -> Result<i64, String> {
        parse_id(&self.word(index, label)?, label)
    }

    fn folder_destination(&self) -> Result<Option<i64>, String> {
        Self::destination(self.folder, self.root, "--folder")
    }

    fn parent_destination(&self) -> Result<Option<i64>, String> {
        Self::destination(self.parent, self.root, "--parent")
    }

    fn destination(target: Option<i64>, root: bool, flag: &str) -> Result<Option<i64>, String> {
        match (target, root) {
            (Some(id), false) => Ok(Some(id)),
            (None, true) => Ok(None),
            (Some(_), true) => Err(format!("cannot specify both '{flag}' and '--root'")),
            (None, false) => Err(format!("specify '{flag} <ID>' or '--root'")),
        }
    }
}

fn parse_id(raw: &str, label: &str) -> Result<i64, String> {
    let value = parse_i64_flag(raw, label)?;
    if value <= 0 {
        return Err(format!(
            "invalid {label} '{raw}'; expected a positive integer"
        ));
    }
    Ok(value)
}

fn non_empty(raw: &str, label: &str) -> Result<String, String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        Err(format!("<{label}> must not be empty"))
    } else {
        Ok(trimmed.to_string())
    }
}

fn missing_subcommand(action: &str) -> String {
    format!("{action} requires a subcommand; run 'lx-tui {action} --help' for usage")
}

fn unexpected(arg: &str, action: &str) -> String {
    format!("unexpected argument '{arg}'; run 'lx-tui {action} --help' for usage")
}

fn folder_name(folders: &[FolderRecord], id: Option<i64>) -> Option<String> {
    let id = id?;
    folders
        .iter()
        .find(|folder| folder.id == id)
        .map(|folder| folder.name.clone())
}

fn find_prompt(prompts: &[PromptRecord], id: i64) -> Result<&PromptRecord, String> {
    let prompt = prompts.iter().find(|prompt| prompt.id == id);
    prompt.ok_or_else(|| format!("prompt {id} not found"))
}

fn active_page(prompt: &PromptRecord) -> Result<&PromptPageRecord, String> {
    prompt
        .active_page_id
        .and_then(|id| prompt.pages.iter().find(|page| page.id == id))
        .or_else(|| prompt.pages.first())
        .ok_or_else(|| format!("prompt {} has no pages", prompt.id))
}

fn page_by_id(prompt: &PromptRecord, id: i64) -> Result<&PromptPageRecord, String> {
    prompt
        .pages
        .iter()
        .find(|page| page.id == id)
        .ok_or_else(|| format!("page {id} not found"))
}

fn resolve_page<'a>(prompt: &'a PromptRecord, raw: &str) -> Result<&'a PromptPageRecord, String> {
    let number = raw.trim().parse::<i64>().ok();
    let page = number.and_then(|value| {
        prompt
            .pages
            .iter()
            .find(|page| page.id == value)
            .or_else(|| {
                usize::try_from(value)
                    .ok()
                    .and_then(|index| index.checked_sub(1))
                    .and_then(|index| prompt.pages.get(index))
            })
    });
    let count = prompt.pages.len();
    page.ok_or_else(|| {
        format!(
            "page '{raw}' not found in prompt {} ({count} pages)",
            prompt.id
        )
    })
}

/// 执行已解析的 prompt 命令。
mod exec;
pub use exec::run;
use exec::*;
