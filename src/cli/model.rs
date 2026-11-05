//! `lx-tui model`：AI 供应商、模型定义、默认选择与远端模型发现。
use crate::cli::args::{
    GlobalOpts, extract_global_opts, parse_bool_flag, parse_u64_flag, reject_unknown_flag, take_opt,
};
use crate::cli::output::{Output, mask_secret};
use crate::cli::{CliAction, EXIT_OK, HelpTopic};
use crate::config::ai::{
    AiSettings, ModelConfig, ModelLimit, ModelSelection, ModelTransport, ProviderConfig,
    ProviderOptions, ProviderTransport, TitleLanguage,
};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::ffi::OsString;
use std::path::PathBuf;
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelCli {
    pub global: GlobalOpts,
    pub command: ModelCommand,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ModelCommand {
    List {
        all: bool,
    },
    Show {
        target: Option<String>,
    },
    Default {
        selection: Option<String>,
        clear: bool,
        title_summary: bool,
    },
    ProviderSet(ProviderSetArgs),
    ProviderRm {
        provider: String,
    },
    ModelSet(ModelSetArgs),
    ModelRm {
        target: String,
    },
    Fetch(FetchArgs),
    Config(ConfigArgs),
}
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ProviderSetArgs {
    pub provider: String,
    pub kind: Option<ProviderTransport>,
    pub name: Option<String>,
    pub base_url: Option<String>,
    pub api_key: Option<String>,
    pub clear_api_key: bool,
    pub enable: bool,
    pub disable: bool,
}
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ModelSetArgs {
    pub target: String,
    pub name: Option<String>,
    pub transport: Option<Option<ModelTransport>>,
    pub context_limit: Option<u64>,
    pub output_limit: Option<u64>,
    pub variant: Option<String>,
    pub image_input: Option<bool>,
}
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct FetchArgs {
    pub provider: String,
    pub base_url: Option<String>,
    pub api_key: Option<String>,
    pub save: bool,
}
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ConfigArgs {
    pub title_language: Option<TitleLanguage>,
    pub timeout_ms: Option<u64>,
    pub exa_key: Option<String>,
    pub tavily_key: Option<String>,
}
pub fn parse(raw_args: &[OsString]) -> Result<CliAction, String> {
    let (global, rest, help) = extract_global_opts(raw_args)?;
    if help {
        return Ok(CliAction::Help(HelpTopic::Model));
    }
    let Some((sub, tail)) = rest.split_first() else {
        return Err("missing 'model' subcommand; run 'lx-tui model --help' for usage".into());
    };
    let command = match sub.as_str() {
        "list" => {
            let raw = collect(tail, &[], &["--all"], "lx-tui model list")?;
            if let Some(arg) = raw.positionals.first() {
                return Err(format!(
                    "unexpected argument '{arg}'; run 'lx-tui model list --help' for usage"
                ));
            }
            ModelCommand::List {
                all: raw.has("--all"),
            }
        }
        "show" => {
            let raw = collect(tail, &[], &[], "lx-tui model show")?;
            ModelCommand::Show {
                target: raw.optional_positional("lx-tui model show")?,
            }
        }
        "default" => parse_default(tail)?,
        "provider-set" => ModelCommand::ProviderSet(parse_provider_set(tail)?),
        "provider-rm" => ModelCommand::ProviderRm {
            provider: single(tail, "lx-tui model provider-rm", "<PROVIDER_ID>")?,
        },
        "model-set" => ModelCommand::ModelSet(parse_model_set(tail)?),
        "model-rm" => ModelCommand::ModelRm {
            target: single(tail, "lx-tui model model-rm", "<PROVIDER_ID>/<MODEL_ID>")?,
        },
        "fetch" => ModelCommand::Fetch(parse_fetch(tail)?),
        "config" => ModelCommand::Config(parse_config(tail)?),
        other => {
            return Err(format!(
                "unknown 'model' subcommand '{other}'; run 'lx-tui model --help' for usage"
            ));
        }
    };
    Ok(CliAction::Model(ModelCli { global, command }))
}

#[derive(Default)]
struct Raw {
    opts: BTreeMap<String, String>,
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
fn parse_default(args: &[String]) -> Result<ModelCommand, String> {
    let help = "lx-tui model default";
    let raw = collect(args, &[], &["--clear", "--title-summary"], help)?;
    let selection = raw.optional_positional(help)?;
    let clear = raw.has("--clear");
    if clear && selection.is_some() {
        return Err("cannot combine '--clear' with a model selection".into());
    }
    Ok(ModelCommand::Default {
        selection,
        clear,
        title_summary: raw.has("--title-summary"),
    })
}
fn parse_provider_set(args: &[String]) -> Result<ProviderSetArgs, String> {
    let help = "lx-tui model provider-set";
    let raw = collect(
        args,
        &["--type", "--name", "--base-url", "--api-key"],
        &["--clear-api-key", "--enable", "--disable"],
        help,
    )?;
    let mut parsed = ProviderSetArgs {
        provider: raw.required("<PROVIDER_ID>", help)?,
        ..Default::default()
    };
    if let Some(value) = raw.get("--type") {
        parsed.kind = Some(
            ProviderTransport::parse(value.trim())
                .ok_or_else(|| format!("invalid '--type' value '{value}'"))?,
        );
    }
    parsed.name = raw.get("--name");
    parsed.base_url = raw.get("--base-url");
    parsed.api_key = raw.get("--api-key");
    parsed.clear_api_key = raw.has("--clear-api-key");
    parsed.enable = raw.has("--enable");
    parsed.disable = raw.has("--disable");
    if parsed.api_key.is_some() && parsed.clear_api_key {
        return Err("cannot combine '--api-key' with '--clear-api-key'".into());
    }
    if parsed.enable && parsed.disable {
        return Err("cannot combine '--enable' with '--disable'".into());
    }
    Ok(parsed)
}
fn parse_model_set(args: &[String]) -> Result<ModelSetArgs, String> {
    let help = "lx-tui model model-set";
    let raw = collect(
        args,
        &[
            "--name",
            "--transport",
            "--context-limit",
            "--output-limit",
            "--variant",
            "--image-input",
        ],
        &[],
        help,
    )?;
    let mut parsed = ModelSetArgs {
        target: raw.required("<PROVIDER_ID>/<MODEL_ID>", help)?,
        ..Default::default()
    };
    if let Some(value) = raw.get("--transport") {
        let trimmed = value.trim().to_ascii_lowercase();
        let transport = match trimmed.as_str() {
            "auto" => None,
            "chat" => Some(ModelTransport::OpenaiCompatible),
            "responses" => Some(ModelTransport::OpenaiResponses),
            "messages" => Some(ModelTransport::Anthropic),
            "gemini" => Some(ModelTransport::Google),
            other => Some(
                ModelTransport::parse(other)
                    .ok_or_else(|| format!("invalid '--transport' value '{value}'"))?,
            ),
        };
        parsed.transport = Some(transport);
    }
    parsed.name = raw.get("--name");
    parsed.context_limit = raw
        .get("--context-limit")
        .map(|value| parse_u64_flag(&value, "--context-limit"))
        .transpose()?;
    parsed.output_limit = raw
        .get("--output-limit")
        .map(|value| parse_u64_flag(&value, "--output-limit"))
        .transpose()?;
    parsed.variant = raw.get("--variant");
    parsed.image_input = raw
        .get("--image-input")
        .map(|value| parse_bool_flag(&value, "--image-input"))
        .transpose()?;
    Ok(parsed)
}
fn parse_fetch(args: &[String]) -> Result<FetchArgs, String> {
    let help = "lx-tui model fetch";
    let raw = collect(args, &["--base-url", "--api-key"], &["--save"], help)?;
    Ok(FetchArgs {
        provider: raw.required("<PROVIDER_ID>", help)?,
        base_url: raw.get("--base-url"),
        api_key: raw.get("--api-key"),
        save: raw.has("--save"),
    })
}
fn parse_config(args: &[String]) -> Result<ConfigArgs, String> {
    let help = "lx-tui model config";
    let raw = collect(
        args,
        &[
            "--title-language",
            "--timeout-ms",
            "--exa-key",
            "--tavily-key",
        ],
        &[],
        help,
    )?;
    if let Some(arg) = raw.positionals.first() {
        return Err(format!(
            "unexpected argument '{arg}'; run '{help} --help' for usage"
        ));
    }
    let mut parsed = ConfigArgs::default();
    if let Some(value) = raw.get("--title-language") {
        parsed.title_language = Some(
            TitleLanguage::parse(&value)
                .ok_or_else(|| format!("invalid '--title-language' value '{value}'"))?,
        );
    }
    parsed.timeout_ms = raw
        .get("--timeout-ms")
        .map(|value| parse_u64_flag(&value, "--timeout-ms"))
        .transpose()?;
    parsed.exa_key = raw.get("--exa-key");
    parsed.tavily_key = raw.get("--tavily-key");
    Ok(parsed)
}

mod exec;
pub use exec::run;
