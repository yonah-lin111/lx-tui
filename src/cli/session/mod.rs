//! `lx-tui session`：会话列表、详情、重命名、删除、分叉、回滚、压缩与子 Agent 查询。

pub mod ops;

use std::collections::HashMap;
use std::ffi::OsString;
use std::io::Write;
use std::path::Path;

use serde_json::{Value, json};

use crate::chat::agent::AgentStatus;
use crate::cli::args::{GlobalOpts, extract_global_opts, parse_usize_flag, required_non_empty};
use crate::cli::context::{WorkspaceContext, acquire_session_lock, resolve_target_session};
use crate::cli::output::Output;
use crate::cli::{CliAction, EXIT_OK, HelpTopic};
use crate::db::Database;
use crate::db::chat::{
    ChatMessage, ModelSnapshot, SessionSummary, SubagentQueueState, SubagentRecord,
};

use ops::RewindModeArg;

/// `lx-tui session` 的完整解析结果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionCli {
    pub global: GlobalOpts,
    pub command: SessionCommand,
}

/// `session` 子命令。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SessionCommand {
    List {
        limit: usize,
        offset: usize,
    },
    Show {
        session_id: Option<String>,
        last: bool,
        tools: bool,
        thinking: bool,
    },
    Rename {
        session_id: Option<String>,
        last: bool,
        new_title: String,
    },
    Delete {
        session_id: Option<String>,
        last: bool,
    },
    Fork {
        session_id: Option<String>,
        last: bool,
        before: Option<String>,
    },
    Rewind {
        session_id: Option<String>,
        last: bool,
        mode: RewindModeArg,
        dry_run: bool,
    },
    Compact {
        session_id: Option<String>,
        last: bool,
        model: Option<String>,
        revert: bool,
    },
    Agents {
        session_id: Option<String>,
        last: bool,
        tree: bool,
        filter: AgentFilterArg,
    },
    Processes {
        session_id: Option<String>,
        last: bool,
    },
}

/// `session agents --filter` 的三种取值（对齐 TUI Agents 面板）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AgentFilterArg {
    #[default]
    All,
    Active,
    Done,
}
impl AgentFilterArg {
    fn parse(raw: &str) -> Result<Self, String> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "all" => Ok(Self::All),
            "active" => Ok(Self::Active),
            "done" => Ok(Self::Done),
            other => Err(format!(
                "invalid '--filter' value '{other}'; expected 'all', 'active', or 'done'"
            )),
        }
    }
    fn matches(self, status: AgentStatus) -> bool {
        match self {
            Self::All => true,
            Self::Active => matches!(
                status,
                AgentStatus::Running | AgentStatus::Idle | AgentStatus::Blocked
            ),
            Self::Done => matches!(
                status,
                AgentStatus::Completed
                    | AgentStatus::Failed
                    | AgentStatus::Interrupted
                    | AgentStatus::Cancelled
            ),
        }
    }
    fn label(self) -> &'static str {
        match self {
            Self::All => "all",
            Self::Active => "active",
            Self::Done => "done",
        }
    }
}

const HELP: &str = "lx-tui session";

/// 顺序扫描器：`--last` 由 `positional` 统一识别，值选项交给 `value`。
struct Scan<'a> {
    args: &'a [String],
    index: usize,
    positionals: Vec<String>,
    last: bool,
    literal: bool,
}
impl<'a> Scan<'a> {
    fn new(args: &'a [String]) -> Self {
        Self {
            args,
            index: 0,
            positionals: Vec::new(),
            last: false,
            literal: false,
        }
    }
    fn token(&mut self) -> Option<String> {
        let token = self.args.get(self.index)?.clone();
        self.index += 1;
        Some(token)
    }
    fn value(&mut self, current: &str, short: &str, long: &str) -> Result<Option<String>, String> {
        if self.literal {
            return Ok(None);
        }
        if (!short.is_empty() && current == short) || current == long {
            let label = if long.is_empty() { short } else { long };
            let value = self
                .args
                .get(self.index)
                .ok_or_else(|| format!("option '{label}' requires a value"))?;
            self.index += 1;
            return Ok(Some(value.clone()));
        }
        for flag in [short, long] {
            if !flag.is_empty()
                && let Some(value) = current
                    .strip_prefix(flag)
                    .and_then(|rest| rest.strip_prefix('='))
            {
                return Ok(Some(value.to_string()));
            }
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
        if arg == "--last" {
            self.last = true;
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
    fn target(&self) -> Result<Option<String>, String> {
        if self.last {
            if let Some(extra) = self.positionals.first() {
                return Err(format!(
                    "unexpected argument '{extra}' when '--last' is used"
                ));
            }
            return Ok(None);
        }
        match self.positionals.as_slice() {
            [] => Err("specify a <SESSION_ID> or pass '--last'".to_string()),
            [id] => Ok(Some(id.clone())),
            _ => Err("too many arguments for a single-session command".to_string()),
        }
    }
}

/// 解析 `lx-tui session ...`（不含顶层命令名）。
pub fn parse(raw_args: &[OsString]) -> Result<CliAction, String> {
    let (global, rest, help_requested) = extract_global_opts(raw_args)?;
    if help_requested {
        return Ok(CliAction::Help(HelpTopic::Session));
    }
    let Some(sub) = rest.first() else {
        return Err("missing session subcommand; run 'lx-tui session --help' for usage".into());
    };
    let args = &rest[1..];
    let command = match sub.as_str() {
        "list" => parse_list(args)?,
        "show" => parse_show(args)?,
        "rename" => parse_rename(args)?,
        "delete" => parse_delete(args)?,
        "fork" => parse_fork(args)?,
        "rewind" => parse_rewind(args)?,
        "compact" => parse_compact(args)?,
        "agents" => parse_agents(args)?,
        "processes" => parse_processes(args)?,
        other => {
            return Err(format!(
                "unknown session subcommand '{other}'; run '{HELP} --help' for usage"
            ));
        }
    };
    Ok(CliAction::Session(SessionCli { global, command }))
}

fn parse_target_only(args: &[String]) -> Result<(Option<String>, bool), String> {
    let mut scan = Scan::new(args);
    while let Some(arg) = scan.token() {
        scan.positional(&arg)?;
    }
    Ok((scan.target()?, scan.last))
}
fn parse_list(args: &[String]) -> Result<SessionCommand, String> {
    let mut scan = Scan::new(args);
    let (mut limit, mut offset) = (20usize, 0usize);
    while let Some(arg) = scan.token() {
        if let Some(v) = scan.value(&arg, "", "--limit")? {
            limit = parse_usize_flag(&v, "--limit")?;
        } else if let Some(v) = scan.value(&arg, "", "--offset")? {
            offset = parse_usize_flag(&v, "--offset")?;
        } else {
            return Err(format!("unexpected argument '{arg}' for 'session list'"));
        }
    }
    if limit == 0 {
        return Err("--limit must be at least 1".to_string());
    }
    Ok(SessionCommand::List { limit, offset })
}
fn parse_show(args: &[String]) -> Result<SessionCommand, String> {
    let mut scan = Scan::new(args);
    let (mut tools, mut thinking) = (false, false);
    while let Some(arg) = scan.token() {
        match arg.as_str() {
            "--tools" => tools = true,
            "--thinking" => thinking = true,
            _ => scan.positional(&arg)?,
        }
    }
    Ok(SessionCommand::Show {
        session_id: scan.target()?,
        last: scan.last,
        tools,
        thinking,
    })
}
fn parse_rename(args: &[String]) -> Result<SessionCommand, String> {
    let mut scan = Scan::new(args);
    let mut title = None;
    while let Some(arg) = scan.token() {
        if let Some(value) = scan.value(&arg, "", "--title")? {
            title = Some(required_non_empty(&value, "--title")?);
        } else {
            scan.positional(&arg)?;
        }
    }
    if let Some(title) = title {
        let session_id = match (scan.last, scan.positionals.as_slice()) {
            (true, []) => None,
            (false, [id]) => Some(id.clone()),
            _ => {
                return Err(
                    "usage: lx-tui session rename (<SESSION_ID> | --last) --title <NEW_TITLE>"
                        .into(),
                );
            }
        };
        return Ok(SessionCommand::Rename {
            session_id,
            last: scan.last,
            new_title: title,
        });
    }
    let (session_id, title) = match (scan.last, scan.positionals.as_slice()) {
        (true, [title]) if title.starts_with("sess-") => {
            return Err(
                "cannot pass both a session ID and '--last'; use '--title <TITLE>' if the new title starts with 'sess-'"
                    .into(),
            );
        }
        (true, [title]) => (None, title.clone()),
        (false, [id, title]) => (Some(id.clone()), title.clone()),
        _ => return Err("usage: lx-tui session rename (<SESSION_ID> | --last) <NEW_TITLE>".into()),
    };
    Ok(SessionCommand::Rename {
        session_id,
        last: scan.last,
        new_title: required_non_empty(&title, "<NEW_TITLE>")?,
    })
}

fn parse_delete(args: &[String]) -> Result<SessionCommand, String> {
    let (session_id, last) = parse_target_only(args)?;
    Ok(SessionCommand::Delete { session_id, last })
}

fn parse_processes(args: &[String]) -> Result<SessionCommand, String> {
    let (session_id, last) = parse_target_only(args)?;
    Ok(SessionCommand::Processes { session_id, last })
}
fn parse_fork(args: &[String]) -> Result<SessionCommand, String> {
    let mut scan = Scan::new(args);
    let mut before = None;
    while let Some(arg) = scan.token() {
        if let Some(v) = scan.value(&arg, "", "--before")? {
            before = Some(required_non_empty(&v, "--before")?);
        } else {
            scan.positional(&arg)?;
        }
    }
    Ok(SessionCommand::Fork {
        session_id: scan.target()?,
        last: scan.last,
        before,
    })
}
fn parse_rewind(args: &[String]) -> Result<SessionCommand, String> {
    let mut scan = Scan::new(args);
    let (mut mode, mut dry_run) = (RewindModeArg::default(), false);
    while let Some(arg) = scan.token() {
        if arg == "--dry-run" {
            dry_run = true;
        } else if let Some(v) = scan.value(&arg, "", "--mode")? {
            mode = RewindModeArg::parse(&v)?;
        } else {
            scan.positional(&arg)?;
        }
    }
    Ok(SessionCommand::Rewind {
        session_id: scan.target()?,
        last: scan.last,
        mode,
        dry_run,
    })
}
fn parse_compact(args: &[String]) -> Result<SessionCommand, String> {
    let mut scan = Scan::new(args);
    let (mut model, mut revert) = (None, false);
    while let Some(arg) = scan.token() {
        if arg == "--revert" {
            revert = true;
        } else if let Some(v) = scan.value(&arg, "-m", "--model")? {
            model = Some(required_non_empty(&v, "--model")?);
        } else {
            scan.positional(&arg)?;
        }
    }
    Ok(SessionCommand::Compact {
        session_id: scan.target()?,
        last: scan.last,
        model,
        revert,
    })
}
fn parse_agents(args: &[String]) -> Result<SessionCommand, String> {
    let mut scan = Scan::new(args);
    let (mut tree, mut filter) = (false, AgentFilterArg::default());
    while let Some(arg) = scan.token() {
        if arg == "--tree" {
            tree = true;
        } else if let Some(v) = scan.value(&arg, "", "--filter")? {
            filter = AgentFilterArg::parse(&v)?;
        } else {
            scan.positional(&arg)?;
        }
    }
    Ok(SessionCommand::Agents {
        session_id: scan.target()?,
        last: scan.last,
        tree,
        filter,
    })
}

/// 简单成功输出：JSON 值 + 一行绿色确认。
mod views;
pub use views::run;
