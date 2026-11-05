//! `lx-tui workspace`：工作区历史列表、信任状态、排序与 Git worktree。

use std::collections::HashSet;
use std::ffi::OsString;
use std::path::{Path, PathBuf};

use serde_json::json;

use crate::cli::args::{GlobalOpts, extract_global_opts, required_non_empty};
use crate::cli::context::WorkspaceContext;
use crate::cli::output::Output;
use crate::cli::{CliAction, EXIT_OK, HelpTopic};
use crate::db::Database;
use crate::db::workspace::WorkspaceRecord;
use crate::platform::FileGuard;

/// `lx-tui workspace` 的完整解析结果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkspaceCli {
    pub global: GlobalOpts,
    pub command: WorkspaceCommand,
}

/// `workspace` 子命令。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WorkspaceCommand {
    List {
        raw: bool,
        search: Option<String>,
    },
    Status,
    Add {
        dir: Option<PathBuf>,
        name: Option<String>,
        trust: bool,
    },
    Rename {
        new_name: String,
    },
    Delete {
        dir: Option<PathBuf>,
        include_worktrees: bool,
    },
    Reorder {
        paths: Vec<PathBuf>,
    },
    Trust {
        name: Option<String>,
    },
    Worktrees,
}

const HELP: &str = "lx-tui workspace";

struct Scan<'a> {
    args: &'a [String],
    index: usize,
    positionals: Vec<String>,
    literal: bool,
}
impl<'a> Scan<'a> {
    fn new(args: &'a [String]) -> Self {
        Self {
            args,
            index: 0,
            positionals: Vec::new(),
            literal: false,
        }
    }
    fn token(&mut self) -> Option<String> {
        let token = self.args.get(self.index)?.clone();
        self.index += 1;
        Some(token)
    }
    fn value(&mut self, current: &str, long: &str) -> Result<Option<String>, String> {
        if self.literal {
            return Ok(None);
        }
        if current == long {
            let value = self
                .args
                .get(self.index)
                .ok_or_else(|| format!("option '{long}' requires a value"))?;
            self.index += 1;
            return Ok(Some(value.clone()));
        }
        if let Some(value) = current
            .strip_prefix(long)
            .and_then(|rest| rest.strip_prefix('='))
        {
            return Ok(Some(value.to_string()));
        }
        Ok(None)
    }
    fn positional(&mut self, arg: &str) -> Result<(), String> {
        if self.literal {
            self.positionals.push(arg.to_string());
            return Ok(());
        }
        if arg == "--" {
            self.literal = true;
            return Ok(());
        }
        if arg.starts_with('-') && arg != "-" {
            return Err(format!(
                "unknown option '{arg}'; run '{HELP} --help' for usage"
            ));
        }
        self.positionals.push(arg.to_string());
        Ok(())
    }
}

/// 解析 `lx-tui workspace ...`（不含顶层命令名）。
pub fn parse(raw_args: &[OsString]) -> Result<CliAction, String> {
    let (global, rest, help_requested) = extract_global_opts(raw_args)?;
    if help_requested {
        return Ok(CliAction::Help(HelpTopic::Workspace));
    }
    let Some(sub) = rest.first() else {
        return Err("missing workspace subcommand; run 'lx-tui workspace --help' for usage".into());
    };
    let args = &rest[1..];
    let command = match sub.as_str() {
        "list" => parse_list(args)?,
        "status" => {
            ensure_empty(args, "workspace status")?;
            WorkspaceCommand::Status
        }
        "add" => parse_add(args)?,
        "rename" => parse_rename(args)?,
        "delete" => parse_delete(args)?,
        "reorder" => parse_reorder(args)?,
        "trust" => parse_trust(args)?,
        "worktrees" => {
            ensure_empty(args, "workspace worktrees")?;
            WorkspaceCommand::Worktrees
        }
        other => {
            return Err(format!(
                "unknown workspace subcommand '{other}'; run '{HELP} --help' for usage"
            ));
        }
    };
    Ok(CliAction::Workspace(WorkspaceCli { global, command }))
}

fn ensure_empty(args: &[String], command: &str) -> Result<(), String> {
    let Some(extra) = args.first() else {
        return Ok(());
    };
    if extra.starts_with('-') && extra != "-" {
        return Err(format!(
            "unknown option '{extra}'; run '{HELP} --help' for usage"
        ));
    }
    Err(format!("unexpected argument '{extra}' for '{command}'"))
}
fn parse_list(args: &[String]) -> Result<WorkspaceCommand, String> {
    let mut scan = Scan::new(args);
    let (mut raw, mut search) = (false, None);
    while let Some(arg) = scan.token() {
        if arg == "--raw" {
            raw = true;
        } else if let Some(v) = scan.value(&arg, "--search")? {
            search = Some(required_non_empty(&v, "--search")?);
        } else {
            return Err(format!("unexpected argument '{arg}' for 'workspace list'"));
        }
    }
    Ok(WorkspaceCommand::List { raw, search })
}
fn parse_add(args: &[String]) -> Result<WorkspaceCommand, String> {
    let mut scan = Scan::new(args);
    let (mut name, mut trust) = (None, false);
    while let Some(arg) = scan.token() {
        if arg == "--trust" {
            trust = true;
        } else if let Some(v) = scan.value(&arg, "--name")? {
            name = Some(required_non_empty(&v, "--name")?);
        } else {
            scan.positional(&arg)?;
        }
    }
    if scan.positionals.len() > 1 {
        return Err("too many arguments for 'workspace add'".into());
    }
    Ok(WorkspaceCommand::Add {
        dir: scan.positionals.first().map(|p| PathBuf::from(p.as_str())),
        name,
        trust,
    })
}
fn parse_rename(args: &[String]) -> Result<WorkspaceCommand, String> {
    let mut scan = Scan::new(args);
    while let Some(arg) = scan.token() {
        scan.positional(&arg)?;
    }
    let [name] = scan.positionals.as_slice() else {
        return Err("usage: lx-tui workspace rename <NEW_NAME>".into());
    };
    Ok(WorkspaceCommand::Rename {
        new_name: required_non_empty(name, "<NEW_NAME>")?,
    })
}
fn parse_delete(args: &[String]) -> Result<WorkspaceCommand, String> {
    let mut scan = Scan::new(args);
    let mut include_worktrees = false;
    while let Some(arg) = scan.token() {
        if arg == "--include-worktrees" {
            include_worktrees = true;
        } else {
            scan.positional(&arg)?;
        }
    }
    if scan.positionals.len() > 1 {
        return Err("too many arguments for 'workspace delete'".into());
    }
    Ok(WorkspaceCommand::Delete {
        dir: scan.positionals.first().map(|p| PathBuf::from(p.as_str())),
        include_worktrees,
    })
}
fn parse_reorder(args: &[String]) -> Result<WorkspaceCommand, String> {
    let mut scan = Scan::new(args);
    while let Some(arg) = scan.token() {
        scan.positional(&arg)?;
    }
    if scan.positionals.is_empty() {
        return Err("usage: lx-tui workspace reorder <PATH1> <PATH2> ...".into());
    }
    Ok(WorkspaceCommand::Reorder {
        paths: scan
            .positionals
            .iter()
            .map(|p| PathBuf::from(p.as_str()))
            .collect(),
    })
}
fn parse_trust(args: &[String]) -> Result<WorkspaceCommand, String> {
    let mut scan = Scan::new(args);
    let mut name = None;
    while let Some(arg) = scan.token() {
        if let Some(v) = scan.value(&arg, "--name")? {
            name = Some(required_non_empty(&v, "--name")?);
        } else {
            return Err(format!("unexpected argument '{arg}' for 'workspace trust'"));
        }
    }
    Ok(WorkspaceCommand::Trust { name })
}

mod exec;
pub use exec::run;
