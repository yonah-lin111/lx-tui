//! `lx-tui skill`：Skills 发现、启停配置与 lx 自有技能读写。
use std::collections::HashSet;
use std::ffi::OsString;
use std::path::{Path, PathBuf};

use serde_json::json;

use crate::config::skills::{
    load_skills, project_skills_root, save_skills, skills_config_path, user_agents_skills_root,
    user_skills_root,
};
use crate::skills::discover::ProjectSkillRoots;
use crate::skills::{SkillCatalog, SkillEntry, SkillScope, SkillSource};

use crate::cli::args::{
    ContentInput, GlobalOpts, ListScopeArg, ScopeArg, extract_global_opts, reject_unknown_flag,
    take_opt,
};
use crate::cli::context::WorkspaceContext;
use crate::cli::output::Output;
use crate::cli::{CliAction, EXIT_OK, HelpTopic};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkillCli {
    pub global: GlobalOpts,
    pub command: SkillCommand,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SkillCommand {
    List {
        scope: ListScopeArg,
    },
    Show {
        name: String,
        scope: Option<ScopeArg>,
    },
    Enable {
        target: String,
    },
    Disable {
        target: String,
    },
    Create {
        name: String,
        description: String,
        content: ContentInput,
        scope: ScopeArg,
    },
    Edit {
        name: String,
        description: Option<String>,
        content: ContentInput,
        scope: ScopeArg,
    },
    Delete {
        name: String,
        scope: ScopeArg,
    },
}

pub fn parse(raw_args: &[OsString]) -> Result<CliAction, String> {
    let (global, rest, help) = extract_global_opts(raw_args)?;
    if help {
        return Ok(CliAction::Help(HelpTopic::Skill));
    }
    let Some((sub, tail)) = rest.split_first() else {
        return Err("missing 'skill' subcommand; run 'lx-tui skill --help' for usage".into());
    };
    let command = match sub.as_str() {
        "list" => {
            let raw = collect(tail, &["--scope"], &[], "lx-tui skill list")?;
            SkillCommand::List {
                scope: match raw.get("--scope") {
                    Some(v) => ListScopeArg::parse(&v)?,
                    None => ListScopeArg::All,
                },
            }
        }
        "show" => {
            let (name, scope) = named(tail, "lx-tui skill show")?;
            SkillCommand::Show { name, scope }
        }
        "enable" => SkillCommand::Enable {
            target: single(tail, "lx-tui skill enable")?,
        },
        "disable" => SkillCommand::Disable {
            target: single(tail, "lx-tui skill disable")?,
        },
        "create" => parse_create(tail)?,
        "edit" => parse_edit(tail)?,
        "delete" => {
            let (name, scope) = named(tail, "lx-tui skill delete")?;
            SkillCommand::Delete {
                name,
                scope: scope.unwrap_or(ScopeArg::User),
            }
        }
        other => {
            return Err(format!(
                "unknown 'skill' subcommand '{other}'; run 'lx-tui skill --help' for usage"
            ));
        }
    };
    Ok(CliAction::Skill(SkillCli { global, command }))
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

fn single(args: &[String], help: &str) -> Result<String, String> {
    collect(args, &[], &[], help)?.required("<NAME_OR_PATH>", help)
}

fn named(args: &[String], help: &str) -> Result<(String, Option<ScopeArg>), String> {
    let raw = collect(args, &["--scope"], &[], help)?;
    let scope = raw
        .get("--scope")
        .map(|value| ScopeArg::parse(&value))
        .transpose()?;
    Ok((raw.required("<NAME>", help)?, scope))
}

fn parse_create(args: &[String]) -> Result<SkillCommand, String> {
    let help = "lx-tui skill create";
    let mut content = ContentInput::default();
    let mut sc = Scanner::new(args);
    let mut name = None;
    let mut description = None;
    let mut scope = ScopeArg::User;
    while !sc.done() {
        if sc.content(&mut content)? {
            continue;
        }
        if let Some(value) = sc.value("--description")? {
            description = Some(value);
            continue;
        }
        if let Some(value) = sc.value("--scope")? {
            scope = ScopeArg::parse(&value)?;
            continue;
        }
        sc.positional(&mut name, help)?;
    }
    Ok(SkillCommand::Create {
        name: name.ok_or_else(|| format!("missing <NAME>; run '{help} --help' for usage"))?,
        description: description.ok_or_else(|| {
            format!("missing '--description <DESC>'; run '{help} --help' for usage")
        })?,
        content,
        scope,
    })
}

fn parse_edit(args: &[String]) -> Result<SkillCommand, String> {
    let help = "lx-tui skill edit";
    let mut content = ContentInput::default();
    let mut sc = Scanner::new(args);
    let mut name = None;
    let mut description = None;
    let mut scope = ScopeArg::User;
    while !sc.done() {
        if sc.content(&mut content)? {
            continue;
        }
        if let Some(value) = sc.value("--description")? {
            description = Some(value);
            continue;
        }
        if let Some(value) = sc.value("--scope")? {
            scope = ScopeArg::parse(&value)?;
            continue;
        }
        sc.positional(&mut name, help)?;
    }
    Ok(SkillCommand::Edit {
        name: name.ok_or_else(|| format!("missing <NAME>; run '{help} --help' for usage"))?,
        description,
        content,
        scope,
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
        let arg = self
            .args
            .get(self.i)
            .map(String::as_str)
            .unwrap_or_default();
        reject_unknown_flag(arg, help)?;
        if slot.replace(arg.to_string()).is_some() {
            return Err(format!("unexpected argument '{arg}'"));
        }
        self.i += 1;
        Ok(())
    }
}

mod exec;
pub use exec::run;
