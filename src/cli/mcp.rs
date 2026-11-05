//! `lx-tui mcp`：MCP 服务配置、启停、探测与 OAuth 授权。
use crate::cli::args::{
    GlobalOpts, ListScopeArg, ScopeArg, extract_global_opts, parse_u64_flag, reject_unknown_flag,
    take_opt,
};
use crate::cli::context::WorkspaceContext;
use crate::cli::output::{Output, mask_secret};
use crate::cli::{CliAction, EXIT_OK, HelpTopic};
use crate::config::mcp::{
    EffectiveServer, McpConfigFile, McpHttpAuth, McpScope, McpServerConfig, McpTransportConfig,
    effective_servers, load_mcp, project_mcp_path, save_mcp, user_mcp_path, validate_server_name,
};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct McpCli {
    pub global: GlobalOpts,
    pub command: McpCommand,
}
#[derive(Debug, Clone, PartialEq, Eq)]
#[allow(clippy::large_enum_variant)]
pub enum McpCommand {
    List {
        scope: ListScopeArg,
    },
    Show {
        name: String,
        scope: Option<ScopeArg>,
    },
    AddStdio(AddStdioArgs),
    AddHttp(AddHttpArgs),
    Remove {
        name: String,
        scope: ScopeArg,
    },
    Enable {
        name: String,
        scope: ScopeArg,
    },
    Disable {
        name: String,
        scope: ScopeArg,
    },
    Probe {
        name: String,
        scope: Option<ScopeArg>,
    },
    Login(LoginArgs),
    Logout {
        name: String,
        scope: Option<ScopeArg>,
    },
}
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct AddStdioArgs {
    pub name: String,
    pub command: String,
    pub args: Vec<String>,
    pub cwd: Option<PathBuf>,
    pub env: BTreeMap<String, String>,
    pub env_vars: Vec<String>,
    pub startup_timeout: Option<u64>,
    pub tool_timeout: Option<u64>,
    pub scope: ScopeArg,
}
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct AddHttpArgs {
    pub name: String,
    pub url: String,
    pub auth: McpHttpAuth,
    pub bearer_token: Option<String>,
    pub bearer_env: Option<String>,
    pub headers: BTreeMap<String, String>,
    pub scopes: Vec<String>,
    pub client_id: Option<String>,
    pub client_secret: Option<String>,
    pub startup_timeout: Option<u64>,
    pub tool_timeout: Option<u64>,
    pub scope: ScopeArg,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoginArgs {
    pub name: String,
    pub scope: Option<ScopeArg>,
    pub no_open: bool,
}
pub fn parse(raw_args: &[OsString]) -> Result<CliAction, String> {
    let (global, rest, help) = extract_global_opts(raw_args)?;
    if help {
        return Ok(CliAction::Help(HelpTopic::Mcp));
    }
    let Some((sub, tail)) = rest.split_first() else {
        return Err("missing 'mcp' subcommand; run 'lx-tui mcp --help' for usage".into());
    };
    let command = match sub.as_str() {
        "list" => {
            let raw = collect(tail, &["--scope"], &[], &[], "lx-tui mcp list")?;
            let scope = match raw.get("--scope") {
                Some(v) => ListScopeArg::parse(&v)?,
                None => ListScopeArg::All,
            };
            McpCommand::List { scope }
        }
        "show" => {
            let (name, scope) = named(tail, "lx-tui mcp show")?;
            McpCommand::Show { name, scope }
        }
        "add-stdio" => McpCommand::AddStdio(parse_add_stdio(tail)?),
        "add-http" => McpCommand::AddHttp(parse_add_http(tail)?),
        "remove" => {
            let (name, scope) = named(tail, "lx-tui mcp remove")?;
            McpCommand::Remove {
                name,
                scope: scope.unwrap_or(ScopeArg::User),
            }
        }
        "enable" => {
            let (name, scope) = named(tail, "lx-tui mcp enable")?;
            McpCommand::Enable {
                name,
                scope: scope.unwrap_or(ScopeArg::User),
            }
        }
        "disable" => {
            let (name, scope) = named(tail, "lx-tui mcp disable")?;
            McpCommand::Disable {
                name,
                scope: scope.unwrap_or(ScopeArg::User),
            }
        }
        "probe" => {
            let (name, scope) = named(tail, "lx-tui mcp probe")?;
            McpCommand::Probe { name, scope }
        }
        "login" => {
            let raw = collect(tail, &["--scope"], &[], &["--no-open"], "lx-tui mcp login")?;
            McpCommand::Login(LoginArgs {
                name: raw.required("<NAME>", "lx-tui mcp login")?,
                scope: raw.scope()?,
                no_open: raw.has("--no-open"),
            })
        }
        "logout" => {
            let (name, scope) = named(tail, "lx-tui mcp logout")?;
            McpCommand::Logout { name, scope }
        }
        other => {
            return Err(format!(
                "unknown 'mcp' subcommand '{other}'; run 'lx-tui mcp --help' for usage"
            ));
        }
    };
    Ok(CliAction::Mcp(McpCli { global, command }))
}

#[derive(Default)]
struct Raw {
    opts: BTreeMap<String, String>,
    lists: BTreeMap<String, Vec<String>>,
    positionals: Vec<String>,
}
impl Raw {
    fn has(&self, flag: &str) -> bool {
        self.opts.contains_key(flag)
    }
    fn get(&self, flag: &str) -> Option<String> {
        self.opts.get(flag).cloned()
    }
    fn list(&self, flag: &str) -> Vec<String> {
        self.lists.get(flag).cloned().unwrap_or_default()
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
    fn scope(&self) -> Result<Option<ScopeArg>, String> {
        self.get("--scope")
            .map(|value| ScopeArg::parse(&value))
            .transpose()
    }
}
fn collect(
    args: &[String],
    values: &[&str],
    lists: &[&str],
    bools: &[&str],
    help: &str,
) -> Result<Raw, String> {
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
        for flag in lists {
            if matched {
                break;
            }
            if let Some(value) = take_opt(args, &mut i, "", flag)? {
                raw.lists
                    .entry((*flag).to_string())
                    .or_default()
                    .push(value);
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
fn named(args: &[String], help: &str) -> Result<(String, Option<ScopeArg>), String> {
    let raw = collect(args, &["--scope"], &[], &[], help)?;
    Ok((raw.required("<NAME>", help)?, raw.scope()?))
}
fn parse_pair(raw: &str, flag: &str) -> Result<(String, String), String> {
    let (key, value) = raw
        .split_once('=')
        .ok_or_else(|| format!("invalid '{flag}' value '{raw}'; expected KEY=VALUE"))?;
    let key = key.trim();
    if key.is_empty() {
        return Err(format!(
            "invalid '{flag}' value '{raw}'; key must not be empty"
        ));
    }
    Ok((key.to_string(), value.trim().to_string()))
}
fn parse_auth(raw: &str) -> Result<McpHttpAuth, String> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "none" => Ok(McpHttpAuth::None),
        "bearer" => Ok(McpHttpAuth::Bearer),
        "oauth" => Ok(McpHttpAuth::Oauth),
        other => Err(format!(
            "invalid '--auth' value '{other}'; expected 'none', 'bearer', or 'oauth'"
        )),
    }
}
fn parse_add_stdio(args: &[String]) -> Result<AddStdioArgs, String> {
    let help = "lx-tui mcp add-stdio";
    let raw = collect(
        args,
        &[
            "--command",
            "--cwd",
            "--startup-timeout",
            "--tool-timeout",
            "--scope",
        ],
        &["--arg", "--env", "--env-var"],
        &[],
        help,
    )?;
    let mut parsed = AddStdioArgs {
        name: raw.required("<NAME>", help)?,
        ..Default::default()
    };
    parsed.command = raw
        .get("--command")
        .ok_or_else(|| format!("missing '--command <CMD>'; run '{help} --help' for usage"))?;
    parsed.args = raw.list("--arg");
    parsed.cwd = raw.get("--cwd").map(PathBuf::from);
    for item in raw.list("--env") {
        let (key, value) = parse_pair(&item, "--env")?;
        parsed.env.insert(key, value);
    }
    parsed.env_vars = raw.list("--env-var");
    parsed.startup_timeout = raw
        .get("--startup-timeout")
        .map(|v| parse_u64_flag(&v, "--startup-timeout"))
        .transpose()?;
    parsed.tool_timeout = raw
        .get("--tool-timeout")
        .map(|v| parse_u64_flag(&v, "--tool-timeout"))
        .transpose()?;
    parsed.scope = raw.scope()?.unwrap_or(ScopeArg::User);
    Ok(parsed)
}
fn parse_add_http(args: &[String]) -> Result<AddHttpArgs, String> {
    let help = "lx-tui mcp add-http";
    let raw = collect(
        args,
        &[
            "--url",
            "--auth",
            "--bearer-token",
            "--bearer-env",
            "--client-id",
            "--client-secret",
            "--startup-timeout",
            "--tool-timeout",
            "--scope",
        ],
        &["--header", "--scope-item"],
        &[],
        help,
    )?;
    let mut parsed = AddHttpArgs {
        name: raw.required("<NAME>", help)?,
        ..Default::default()
    };
    parsed.url = raw
        .get("--url")
        .ok_or_else(|| format!("missing '--url <URL>'; run '{help} --help' for usage"))?;
    if let Some(value) = raw.get("--auth") {
        parsed.auth = parse_auth(&value)?;
    }
    parsed.bearer_token = raw.get("--bearer-token");
    parsed.bearer_env = raw.get("--bearer-env");
    for item in raw.list("--header") {
        let (key, value) = parse_pair(&item, "--header")?;
        parsed.headers.insert(key, value);
    }
    parsed.scopes = raw.list("--scope-item");
    parsed.client_id = raw.get("--client-id");
    parsed.client_secret = raw.get("--client-secret");
    parsed.startup_timeout = raw
        .get("--startup-timeout")
        .map(|v| parse_u64_flag(&v, "--startup-timeout"))
        .transpose()?;
    parsed.tool_timeout = raw
        .get("--tool-timeout")
        .map(|v| parse_u64_flag(&v, "--tool-timeout"))
        .transpose()?;
    parsed.scope = raw.scope()?.unwrap_or(ScopeArg::User);
    Ok(parsed)
}

mod exec;
pub use exec::run;
