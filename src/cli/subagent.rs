//! `lx-tui subagent`：子 Agent 并发、深度、委派模式与角色配置。
use crate::cli::args::{
    ContentInput, GlobalOpts, ListScopeArg, ScopeArg, extract_global_opts, parse_bool_flag,
    parse_u64_flag, reject_unknown_flag, take_opt,
};
use crate::cli::context::WorkspaceContext;
use crate::cli::output::Output;
use crate::cli::{CliAction, EXIT_OK, HelpTopic};
use crate::config::ai::ModelSelection;
use crate::config::subagents::{
    BaseRole, CustomRoleConfig, DelegationMode, MAX_ROLE_DESCRIPTION_CHARS, MAX_ROLE_PROMPT_CHARS,
    ResolvedRoleSpec, RoleCapabilities, RoleOverrideConfig, SubagentsConfigFile, SubagentsSection,
    is_valid_role_name, load_subagents, merge_project_roles_for_paths, save_subagents,
    subagents_config_path,
};
use serde_json::json;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SubagentCli {
    pub global: GlobalOpts,
    pub command: SubagentCommand,
}
#[derive(Debug, Clone, PartialEq, Eq)]
// 变体体积差异来自携带完整角色参数的 `RoleSetArgs`；命令只在启动时分发一次，
// 装箱会让所有匹配点多一层解引用，收益不抵改造成本。
#[allow(clippy::large_enum_variant)]
pub enum SubagentCommand {
    Show {
        role: Option<String>,
        scope: ListScopeArg,
    },
    Set(SetArgs),
    RoleSet(RoleSetArgs),
    RoleReset {
        name: String,
    },
    RoleRm {
        name: String,
        scope: ScopeArg,
    },
}
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SetArgs {
    pub max_concurrent: Option<usize>,
    pub max_depth: Option<usize>,
    pub default_max_tokens: Option<u64>,
    pub clear_max_tokens: bool,
    pub delegation_mode: Option<DelegationMode>,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ModelChange {
    Keep,
    Clear,
    Set(ModelSelection),
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RoleSetArgs {
    pub name: String,
    pub base: Option<BaseRole>,
    pub description: Option<String>,
    pub prompt: ContentInput,
    pub model: ModelChange,
    pub lock_model: Option<bool>,
    pub allow_web_search: Option<bool>,
    pub allow_mcp: Option<bool>,
    pub allow_skills: Option<bool>,
    pub nicknames: Option<Vec<String>>,
    pub scope: ScopeArg,
}
pub fn parse(raw_args: &[OsString]) -> Result<CliAction, String> {
    let (global, rest, help) = extract_global_opts(raw_args)?;
    if help {
        return Ok(CliAction::Help(HelpTopic::Subagent));
    }
    let Some((sub, tail)) = rest.split_first() else {
        return Err("missing 'subagent' subcommand; run 'lx-tui subagent --help' for usage".into());
    };
    let command = match sub.as_str() {
        "show" => {
            let raw = collect(tail, &["--scope"], &[], "lx-tui subagent show")?;
            let scope = match raw.get("--scope") {
                Some(v) => ListScopeArg::parse(&v)?,
                None => ListScopeArg::All,
            };
            SubagentCommand::Show {
                role: raw.optional_positional("lx-tui subagent show")?,
                scope,
            }
        }
        "set" => SubagentCommand::Set(parse_set(tail)?),
        "role-set" => SubagentCommand::RoleSet(parse_role_set(tail)?),
        "role-reset" => SubagentCommand::RoleReset {
            name: single(
                tail,
                "lx-tui subagent role-reset",
                "<explorer|executor|monitor>",
            )?,
        },
        "role-rm" => {
            let raw = collect(tail, &["--scope"], &[], "lx-tui subagent role-rm")?;
            let scope = raw
                .get("--scope")
                .map(|value| ScopeArg::parse(&value))
                .transpose()?
                .unwrap_or(ScopeArg::User);
            SubagentCommand::RoleRm {
                name: raw.required("<CUSTOM_ROLE_NAME>", "lx-tui subagent role-rm")?,
                scope,
            }
        }
        other => {
            return Err(format!(
                "unknown 'subagent' subcommand '{other}'; run 'lx-tui subagent --help' for usage"
            ));
        }
    };
    Ok(CliAction::Subagent(SubagentCli { global, command }))
}

#[derive(Default)]
struct Raw {
    opts: std::collections::BTreeMap<String, String>,
    positionals: Vec<String>,
}
impl Raw {
    fn has(&self, flag: &str) -> bool {
        self.opts.contains_key(flag)
    }
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
fn single(args: &[String], help: &str, label: &str) -> Result<String, String> {
    collect(args, &[], &[], help)?.required(label, help)
}
fn parse_set(args: &[String]) -> Result<SetArgs, String> {
    let help = "lx-tui subagent set";
    let raw = collect(
        args,
        &[
            "--max-concurrent",
            "--max-depth",
            "--default-max-tokens",
            "--delegation-mode",
        ],
        &["--clear-max-tokens"],
        help,
    )?;
    let mut parsed = SetArgs {
        clear_max_tokens: raw.has("--clear-max-tokens"),
        ..Default::default()
    };
    parsed.max_concurrent = raw
        .get("--max-concurrent")
        .map(|v| {
            v.trim()
                .parse::<usize>()
                .map_err(|_| format!("invalid integer '{v}' for '--max-concurrent'"))
        })
        .transpose()?;
    parsed.max_depth = raw
        .get("--max-depth")
        .map(|v| {
            v.trim()
                .parse::<usize>()
                .map_err(|_| format!("invalid integer '{v}' for '--max-depth'"))
        })
        .transpose()?;
    parsed.default_max_tokens = raw
        .get("--default-max-tokens")
        .map(|v| parse_u64_flag(&v, "--default-max-tokens"))
        .transpose()?;
    if let Some(value) = raw.get("--delegation-mode") {
        parsed.delegation_mode = Some(
            DelegationMode::parse(&value)
                .ok_or_else(|| format!("invalid '--delegation-mode' value '{value}'"))?,
        );
    }
    if parsed.default_max_tokens.is_some() && parsed.clear_max_tokens {
        return Err("cannot combine '--default-max-tokens' with '--clear-max-tokens'".into());
    }
    Ok(parsed)
}
fn parse_model_spec(raw: &str) -> Result<ModelSelection, String> {
    let (path, variant) = match raw.split_once('#') {
        Some((path, variant)) => (path, Some(variant.trim().to_string())),
        None => (raw, None),
    };
    let (provider, model) = path.split_once('/').ok_or_else(|| {
        format!("invalid '--model' value '{raw}'; expected '<PROVIDER>/<MODEL>[#<VARIANT>]'")
    })?;
    let (provider, model) = (provider.trim(), model.trim());
    if provider.is_empty() || model.is_empty() {
        return Err(format!(
            "invalid '--model' value '{raw}'; expected '<PROVIDER>/<MODEL>[#<VARIANT>]'"
        ));
    }
    Ok(ModelSelection {
        provider: provider.into(),
        model: model.into(),
        variant,
    })
}
fn parse_role_set(args: &[String]) -> Result<RoleSetArgs, String> {
    let help = "lx-tui subagent role-set";
    let mut prompt = ContentInput::default();
    let mut sc = Scanner::new(args);
    let mut name = None;
    let mut base = None;
    let mut description = None;
    let mut model = ModelChange::Keep;
    let mut lock_model = None;
    let mut allow_web_search = None;
    let mut allow_mcp = None;
    let mut allow_skills = None;
    let mut nicknames = None;
    let mut scope = ScopeArg::User;
    while !sc.done() {
        if let Some(value) = sc.value("--base")? {
            base = Some(BaseRole::parse(&value).ok_or_else(|| format!("invalid '--base' value '{value}'; expected 'explorer', 'executor', or 'monitor'"))?);
            continue;
        }
        if let Some(value) = sc.value("--description")? {
            description = Some(value);
            continue;
        }
        if let Some(value) = sc.value("--prompt")? {
            if prompt.inline.is_some() {
                return Err("cannot specify both '--prompt' and '--content'".into());
            }
            prompt.inline = Some(value);
            continue;
        }
        if sc.content(&mut prompt)? {
            continue;
        }
        if let Some(value) = sc.value("--model")? {
            if matches!(model, ModelChange::Clear) {
                return Err("cannot combine '--model' with '--clear-model'".into());
            }
            model = ModelChange::Set(parse_model_spec(&value)?);
            continue;
        }
        if let Some(value) = sc.value("--lock-model")? {
            lock_model = Some(parse_bool_flag(&value, "--lock-model")?);
            continue;
        }
        if let Some(value) = sc.value("--allow-web-search")? {
            allow_web_search = Some(parse_bool_flag(&value, "--allow-web-search")?);
            continue;
        }
        if let Some(value) = sc.value("--allow-mcp")? {
            allow_mcp = Some(parse_bool_flag(&value, "--allow-mcp")?);
            continue;
        }
        if let Some(value) = sc.value("--allow-skills")? {
            allow_skills = Some(parse_bool_flag(&value, "--allow-skills")?);
            continue;
        }
        if let Some(value) = sc.value("--nicknames")? {
            nicknames = Some(
                value
                    .split(',')
                    .map(str::trim)
                    .filter(|item| !item.is_empty())
                    .map(str::to_string)
                    .collect::<Vec<String>>(),
            );
            continue;
        }
        if let Some(value) = sc.value("--scope")? {
            scope = ScopeArg::parse(&value)?;
            continue;
        }
        if sc.peek() == "--clear-model" {
            if matches!(model, ModelChange::Set(_)) {
                return Err("cannot combine '--model' with '--clear-model'".into());
            }
            model = ModelChange::Clear;
            sc.bump();
            continue;
        }
        sc.positional(&mut name, help)?;
    }
    Ok(RoleSetArgs {
        name: name.ok_or_else(|| format!("missing <NAME>; run '{help} --help' for usage"))?,
        base,
        description,
        prompt,
        model,
        lock_model,
        allow_web_search,
        allow_mcp,
        allow_skills,
        nicknames,
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
