//! `lx-tui command`：自定义斜杠命令（`agentMD` / `agentBlock`）的读写管理。
use std::collections::HashSet;
use std::ffi::OsString;
use std::path::{Path, PathBuf};

use serde_json::json;

use crate::settings::commands::{
    self, BlockType, CommandKind, CommandRecord, CommandScope, MdScope,
};

use crate::cli::args::{
    ContentInput, GlobalOpts, ListScopeArg, ScopeArg, extract_global_opts, reject_unknown_flag,
    take_opt,
};
use crate::cli::context::WorkspaceContext;
use crate::cli::output::Output;
use crate::cli::{CliAction, EXIT_OK, HelpTopic};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandCli {
    pub global: GlobalOpts,
    pub command: CommandSubcommand,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CommandSubcommand {
    List {
        scope: ListScopeArg,
        kind: Option<CommandKind>,
    },
    Show {
        name: String,
        scope: Option<ScopeArg>,
        kind: Option<CommandKind>,
    },
    Create(CreateArgs),
    Edit(EditArgs),
    Rename {
        old_name: String,
        new_name: String,
        scope: Option<ScopeArg>,
        kind: Option<CommandKind>,
    },
    Delete {
        name: String,
        scope: Option<ScopeArg>,
        kind: Option<CommandKind>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CreateArgs {
    pub name: String,
    pub kind: CommandKind,
    pub description: String,
    pub md_scope: Option<MdScope>,
    pub block: Option<BlockType>,
    pub arg_hint: Option<String>,
    pub title: Option<String>,
    pub content: ContentInput,
    pub scope: ScopeArg,
    pub overwrite: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EditArgs {
    pub name: String,
    pub kind: Option<CommandKind>,
    pub description: Option<String>,
    pub md_scope: Option<MdScope>,
    pub block: Option<BlockType>,
    pub arg_hint: Option<String>,
    pub title: Option<String>,
    pub content: ContentInput,
    pub scope: Option<ScopeArg>,
}

pub fn parse(raw_args: &[OsString]) -> Result<CliAction, String> {
    let (global, rest, help) = extract_global_opts(raw_args)?;
    if help {
        return Ok(CliAction::Help(HelpTopic::Command));
    }
    let Some((sub, tail)) = rest.split_first() else {
        return Err("missing 'command' subcommand; run 'lx-tui command --help' for usage".into());
    };
    let command = match sub.as_str() {
        "list" => parse_list(tail)?,
        "show" => parse_show(tail)?,
        "create" => parse_create(tail)?,
        "edit" => parse_edit(tail)?,
        "rename" => parse_rename(tail)?,
        "delete" => parse_delete(tail)?,
        other => {
            return Err(format!(
                "unknown 'command' subcommand '{other}'; run 'lx-tui command --help' for usage"
            ));
        }
    };
    Ok(CliAction::Command(CommandCli { global, command }))
}

#[derive(Default)]
struct Raw {
    opts: std::collections::BTreeMap<String, String>,
    positionals: Vec<String>,
}

impl Raw {
    fn get(&self, flag: &str) -> Option<String> {
        self.opts.get(flag).cloned()
    }
    fn optional_positional(&self, help: &str) -> Result<Option<String>, String> {
        match self.positionals.as_slice() {
            [] => Ok(None),
            [one] => Ok(Some(one.clone())),
            [_, extra, ..] => Err(format!(
                "unexpected argument '{extra}'; run '{help} --help' for usage"
            )),
        }
    }
    fn required(&self, label: &str, help: &str) -> Result<String, String> {
        self.optional_positional(help)?
            .ok_or_else(|| format!("missing {label}; run '{help} --help' for usage"))
    }
}

fn collect(args: &[String], values: &[&str], bools: &[&str], help: &str) -> Result<Raw, String> {
    let mut raw = Raw::default();
    let mut i = 0;
    while i < args.len() {
        let arg = args[i].as_str();
        let mut matched = false;
        for flag in values {
            if let Some(value) = take_opt(args, &mut i, "", flag)? {
                if raw.opts.insert((*flag).to_string(), value).is_some() {
                    return Err(format!(
                        "option '{flag}' cannot be specified more than once"
                    ));
                }
                i += 1;
                matched = true;
                break;
            }
        }
        if !matched && bools.contains(&arg) {
            raw.opts.insert(arg.to_string(), String::new());
            i += 1;
            matched = true;
        }
        if matched {
            continue;
        }
        reject_unknown_flag(arg, help)?;
        raw.positionals.push(arg.to_string());
        i += 1;
    }
    Ok(raw)
}

fn parse_kind(value: &str, flag: &str) -> Result<CommandKind, String> {
    CommandKind::parse(value.trim()).ok_or_else(|| {
        format!("invalid '{flag}' value '{value}'; expected 'agentMD' or 'agentBlock'")
    })
}

fn parse_md_scope(value: &str) -> Result<MdScope, String> {
    MdScope::parse(value.trim()).ok_or_else(|| {
        format!("invalid '--md-scope' value '{value}'; expected 'global' or 'template'")
    })
}

fn parse_block(value: &str) -> Result<BlockType, String> {
    BlockType::parse(value.trim()).ok_or_else(|| {
        format!("invalid '--block' value '{value}'; expected 'template' or 'supple'")
    })
}

fn kind_of(raw: &Raw) -> Result<Option<CommandKind>, String> {
    raw.get("--kind")
        .map(|value| parse_kind(&value, "--kind"))
        .transpose()
}

fn scope_of(raw: &Raw) -> Result<Option<ScopeArg>, String> {
    raw.get("--scope")
        .map(|value| ScopeArg::parse(&value))
        .transpose()
}

fn parse_list(args: &[String]) -> Result<CommandSubcommand, String> {
    let raw = collect(args, &["--scope", "--kind"], &[], "lx-tui command list")?;
    Ok(CommandSubcommand::List {
        scope: match raw.get("--scope") {
            Some(v) => ListScopeArg::parse(&v)?,
            None => ListScopeArg::All,
        },
        kind: kind_of(&raw)?,
    })
}

fn parse_show(args: &[String]) -> Result<CommandSubcommand, String> {
    let help = "lx-tui command show";
    let raw = collect(args, &["--scope", "--kind"], &[], help)?;
    Ok(CommandSubcommand::Show {
        name: raw.required("<NAME>", help)?,
        scope: scope_of(&raw)?,
        kind: kind_of(&raw)?,
    })
}

fn parse_create(args: &[String]) -> Result<CommandSubcommand, String> {
    let help = "lx-tui command create";
    let mut content = ContentInput::default();
    let mut sc = Scanner::new(args);
    let mut name = None;
    let mut kind = None;
    let mut description = None;
    let mut md_scope = None;
    let mut block = None;
    let mut arg_hint = None;
    let mut title = None;
    let mut scope = ScopeArg::User;
    let mut overwrite = false;
    while !sc.done() {
        if sc.content(&mut content)? {
            continue;
        }
        if let Some(value) = sc.value("--kind")? {
            kind = Some(parse_kind(&value, "--kind")?);
            continue;
        }
        if let Some(value) = sc.value("--description")? {
            description = Some(value);
            continue;
        }
        if let Some(value) = sc.value("--md-scope")? {
            md_scope = Some(parse_md_scope(&value)?);
            continue;
        }
        if let Some(value) = sc.value("--block")? {
            block = Some(parse_block(&value)?);
            continue;
        }
        if let Some(value) = sc.value("--arg-hint")? {
            arg_hint = Some(value);
            continue;
        }
        if let Some(value) = sc.value("--title")? {
            title = Some(value);
            continue;
        }
        if let Some(value) = sc.value("--scope")? {
            scope = ScopeArg::parse(&value)?;
            continue;
        }
        if sc.peek() == "--overwrite" {
            overwrite = true;
            sc.bump();
            continue;
        }
        sc.positional(&mut name, help)?;
    }
    Ok(CommandSubcommand::Create(CreateArgs {
        name: name.ok_or_else(|| format!("missing <NAME>; run '{help} --help' for usage"))?,
        kind: kind.ok_or_else(|| {
            format!("missing '--kind <agentMD|agentBlock>'; run '{help} --help' for usage")
        })?,
        description: description.ok_or_else(|| {
            format!("missing '--description <DESC>'; run '{help} --help' for usage")
        })?,
        md_scope,
        block,
        arg_hint,
        title,
        content,
        scope,
        overwrite,
    }))
}

fn parse_edit(args: &[String]) -> Result<CommandSubcommand, String> {
    let help = "lx-tui command edit";
    let mut content = ContentInput::default();
    let mut sc = Scanner::new(args);
    let mut name = None;
    let mut kind = None;
    let mut description = None;
    let mut md_scope = None;
    let mut block = None;
    let mut arg_hint = None;
    let mut title = None;
    let mut scope = None;
    while !sc.done() {
        if sc.content(&mut content)? {
            continue;
        }
        if let Some(value) = sc.value("--kind")? {
            kind = Some(parse_kind(&value, "--kind")?);
            continue;
        }
        if let Some(value) = sc.value("--description")? {
            description = Some(value);
            continue;
        }
        if let Some(value) = sc.value("--md-scope")? {
            md_scope = Some(parse_md_scope(&value)?);
            continue;
        }
        if let Some(value) = sc.value("--block")? {
            block = Some(parse_block(&value)?);
            continue;
        }
        if let Some(value) = sc.value("--arg-hint")? {
            arg_hint = Some(value);
            continue;
        }
        if let Some(value) = sc.value("--title")? {
            title = Some(value);
            continue;
        }
        if let Some(value) = sc.value("--scope")? {
            scope = Some(ScopeArg::parse(&value)?);
            continue;
        }
        sc.positional(&mut name, help)?;
    }
    Ok(CommandSubcommand::Edit(EditArgs {
        name: name.ok_or_else(|| format!("missing <NAME>; run '{help} --help' for usage"))?,
        kind,
        description,
        md_scope,
        block,
        arg_hint,
        title,
        content,
        scope,
    }))
}

fn parse_rename(args: &[String]) -> Result<CommandSubcommand, String> {
    let help = "lx-tui command rename";
    let raw = collect(args, &["--scope", "--kind"], &[], help)?;
    if raw.positionals.len() != 2 {
        return Err(format!(
            "missing <OLD_NAME> <NEW_NAME>; run '{help} --help' for usage"
        ));
    }
    Ok(CommandSubcommand::Rename {
        old_name: raw.positionals[0].clone(),
        new_name: raw.positionals[1].clone(),
        scope: scope_of(&raw)?,
        kind: kind_of(&raw)?,
    })
}

fn parse_delete(args: &[String]) -> Result<CommandSubcommand, String> {
    let help = "lx-tui command delete";
    let raw = collect(args, &["--scope", "--kind"], &[], help)?;
    Ok(CommandSubcommand::Delete {
        name: raw.required("<NAME>", help)?,
        scope: scope_of(&raw)?,
        kind: kind_of(&raw)?,
    })
}

struct Scanner<'a> {
    args: &'a [String],
    i: usize,
}

impl<'a> Scanner<'a> {
    fn new(args: &'a [String]) -> Self {
        Self { args, i: 0 }
    }
    fn done(&self) -> bool {
        self.i >= self.args.len()
    }
    fn peek(&self) -> &'a str {
        self.args
            .get(self.i)
            .map(String::as_str)
            .unwrap_or_default()
    }
    fn bump(&mut self) {
        self.i += 1;
    }
    fn value(&mut self, flag: &str) -> Result<Option<String>, String> {
        if self.done() {
            return Ok(None);
        }
        match take_opt(self.args, &mut self.i, "", flag)? {
            Some(value) => {
                self.i += 1;
                Ok(Some(value))
            }
            None => Ok(None),
        }
    }
    fn content(&mut self, input: &mut ContentInput) -> Result<bool, String> {
        if self.done() {
            return Ok(false);
        }
        let matched = input.try_take(self.args, &mut self.i)?;
        if matched {
            self.i += 1;
        }
        Ok(matched)
    }
    fn positional(&mut self, slot: &mut Option<String>, help: &str) -> Result<(), String> {
        let arg = self.peek();
        reject_unknown_flag(arg, help)?;
        if slot.replace(arg.to_string()).is_some() {
            return Err(format!("unexpected argument '{arg}'"));
        }
        self.bump();
        Ok(())
    }
}

mod exec;
pub use exec::run;
