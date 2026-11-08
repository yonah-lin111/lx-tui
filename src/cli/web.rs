//! `lx-tui web`：无头驱动网页端 AI 站点（DeepSeek / Gemini）的模型查询、流式对话、附件上传、`/rewind` 原地修改与云端会话管理。

mod exec;

use std::ffi::OsString;
use std::io::IsTerminal;
use std::path::PathBuf;

use crate::app::state::{
    AiWebSite, DeepseekThinkingMode, GeminiThinkingLevel, GeminiWebModel, ai_web_remote_session,
};
use crate::cli::args::{
    GlobalOpts, extract_global_opts, parse_usize_flag, reject_unknown_flag, required_non_empty,
    take_opt,
};
use crate::cli::{CliAction, HelpTopic};
use crate::ui::text;

pub use exec::run;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WebCli {
    pub global: GlobalOpts,
    pub command: WebCommand,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WebCommand {
    /// 列出双站点支持的模型、思考等级、默认值与会话 ID 前缀。
    Models { site: Option<AiWebSite> },
    /// 发送消息或对历史轮次执行 `/rewind` 原地修改（`action = "chat"`）。
    Chat {
        site: AiWebSite,
        model: String,
        thinking: String,
        prompt: String,
        session_id: Option<String>,
        remote_session_id: Option<String>,
        first_turn: bool,
        edit_user_index: Option<usize>,
        files: Vec<PathBuf>,
        keep_files: Vec<String>,
    },
    /// 重置网页端开启新对话（`action = "new"`）。
    New { site: AiWebSite },
    /// 停止网页端正在生成的回答（`action = "stop"`）。
    Stop { site: AiWebSite },
    /// 拉取网页端云端历史会话列表（`action = "list_sessions"`）。
    List {
        site: AiWebSite,
        limit: Option<usize>,
    },
    /// 加载并展示单条云端会话的完整消息（`action = "load_session"`）。
    Show {
        site: AiWebSite,
        session_id: String,
        remote_session_id: String,
        show_thinking: bool,
    },
    /// 重命名云端会话标题（`action = "rename_session"`）。
    Rename {
        site: AiWebSite,
        session_id: String,
        remote_session_id: String,
        title: String,
    },
    /// 删除云端会话（`action = "delete_session"`）。
    Delete {
        site: AiWebSite,
        session_id: String,
        remote_session_id: String,
    },
}

pub fn parse(raw_args: &[OsString]) -> Result<CliAction, String> {
    let (global, rest, help) = extract_global_opts(raw_args)?;
    if help {
        return Ok(CliAction::Help(HelpTopic::Web));
    }
    let Some((sub, tail)) = rest.split_first() else {
        return Err("missing 'web' subcommand; run 'lx-tui web --help' for usage".into());
    };
    let command = match sub.as_str() {
        "models" => parse_models(tail)?,
        "chat" => parse_chat(tail)?,
        "new" => WebCommand::New {
            site: parse_site_only(tail, "lx-tui web new")?.unwrap_or_default(),
        },
        "stop" => WebCommand::Stop {
            site: parse_site_only(tail, "lx-tui web stop")?.unwrap_or_default(),
        },
        "list" => parse_list(tail)?,
        "show" => parse_show(tail)?,
        "rename" => parse_rename(tail)?,
        "delete" => parse_delete(tail)?,
        other => {
            return Err(format!(
                "unknown 'web' subcommand '{other}'; run 'lx-tui web --help' for usage"
            ));
        }
    };
    Ok(CliAction::Web(WebCli { global, command }))
}

pub fn parse_site(raw: &str) -> Result<AiWebSite, String> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "deepseek" | "ds" => Ok(AiWebSite::Deepseek),
        "gemini" | "gm" => Ok(AiWebSite::Gemini),
        other => Err(format!(
            "invalid '--site' value '{other}'; expected 'deepseek' or 'gemini'"
        )),
    }
}

pub fn site_key(site: AiWebSite) -> &'static str {
    match site {
        AiWebSite::Deepseek => "deepseek",
        AiWebSite::Gemini => "gemini",
    }
}

pub fn resolve_model_and_thinking(
    site: AiWebSite,
    model_opt: Option<&str>,
    thinking_opt: Option<&str>,
) -> Result<(String, String), String> {
    match site {
        AiWebSite::Deepseek => {
            let model = match model_opt {
                Some(raw) => {
                    if !raw.trim().eq_ignore_ascii_case(text::AI_WEB_SITE_DEEPSEEK) {
                        return Err(format!(
                            "invalid '--model' value '{}' for DeepSeek; expected '{}'",
                            raw.trim(),
                            text::AI_WEB_SITE_DEEPSEEK
                        ));
                    }
                    text::AI_WEB_SITE_DEEPSEEK.to_string()
                }
                None => text::AI_WEB_SITE_DEEPSEEK.to_string(),
            };
            let thinking = match thinking_opt {
                Some(raw) => DeepseekThinkingMode::from_id(raw)
                    .ok_or_else(|| {
                        format!(
                            "invalid '--thinking' value '{}' for DeepSeek; expected 'deepthink' or 'off'",
                            raw.trim()
                        )
                    })?
                    .id()
                    .to_string(),
                None => DeepseekThinkingMode::default().id().to_string(),
            };
            Ok((model, thinking))
        }
        AiWebSite::Gemini => {
            let model = match model_opt {
                Some(raw) => GeminiWebModel::from_id(raw)
                    .ok_or_else(|| {
                        format!(
                            "invalid '--model' value '{}' for Gemini; expected '3.5 Flash-Lite', '3.8 Flash', '3.1 Pro', or '4 Argon'",
                            raw.trim()
                        )
                    })?
                    .id()
                    .to_string(),
                None => GeminiWebModel::default().id().to_string(),
            };
            let thinking = match thinking_opt {
                Some(raw) => GeminiThinkingLevel::from_id(raw)
                    .ok_or_else(|| {
                        format!(
                            "invalid '--thinking' value '{}' for Gemini; expected 'low', 'medium', or 'high'",
                            raw.trim()
                        )
                    })?
                    .id()
                    .to_string(),
                None => GeminiThinkingLevel::default().id().to_string(),
            };
            Ok((model, thinking))
        }
    }
}

fn resolve_session_target(
    site_opt: Option<AiWebSite>,
    raw_id: &str,
) -> Result<(AiWebSite, String, String), String> {
    let trimmed = raw_id.trim();
    if trimmed.is_empty() {
        return Err("session ID cannot be empty".to_string());
    }
    if let Some((prefixed_site, remote_id)) = ai_web_remote_session(trimmed) {
        if let Some(explicit_site) = site_opt
            && explicit_site != prefixed_site
        {
            return Err(format!(
                "session '{trimmed}' belongs to '{}' but '--site {}' was specified",
                site_key(prefixed_site),
                site_key(explicit_site)
            ));
        }
        return Ok((
            prefixed_site,
            prefixed_site.format_remote_session_id(remote_id),
            remote_id.to_string(),
        ));
    }
    let site = site_opt.unwrap_or_default();
    Ok((
        site,
        site.format_remote_session_id(trimmed),
        trimmed.to_string(),
    ))
}

fn parse_models(args: &[String]) -> Result<WebCommand, String> {
    let site = parse_site_only(args, "lx-tui web models")?;
    Ok(WebCommand::Models { site })
}

fn parse_site_only(args: &[String], help_cmd: &str) -> Result<Option<AiWebSite>, String> {
    let mut site = None;
    let mut i = 0;
    while i < args.len() {
        if let Some(val) = take_opt(args, &mut i, "", "--site")? {
            if site.is_some() {
                return Err("option '--site' cannot be specified more than once".into());
            }
            site = Some(parse_site(&val)?);
            i += 1;
            continue;
        }
        reject_unknown_flag(&args[i], help_cmd)?;
        return Err(format!(
            "unexpected argument '{}'; run '{help_cmd} --help' for usage",
            args[i]
        ));
    }
    Ok(site)
}

fn parse_chat(args: &[String]) -> Result<WebCommand, String> {
    let help_cmd = "lx-tui web chat";
    let mut site_opt: Option<AiWebSite> = None;
    let mut model_opt: Option<String> = None;
    let mut thinking_opt: Option<String> = None;
    let mut session_opt: Option<String> = None;
    let mut first_turn = false;
    let mut edit_user_index: Option<usize> = None;
    let mut files: Vec<PathBuf> = Vec::new();
    let mut keep_files: Vec<String> = Vec::new();
    let mut positionals: Vec<String> = Vec::new();
    let mut literal = false;
    let mut i = 0;

    while i < args.len() {
        let arg = args[i].as_str();
        if literal {
            positionals.push(arg.to_string());
            i += 1;
            continue;
        }
        if arg == "--" {
            literal = true;
            i += 1;
            continue;
        }
        if let Some(val) = take_opt(args, &mut i, "", "--site")? {
            if site_opt.is_some() {
                return Err("option '--site' cannot be specified more than once".into());
            }
            site_opt = Some(parse_site(&val)?);
            i += 1;
            continue;
        }
        if let Some(val) = take_opt(args, &mut i, "-m", "--model")? {
            if model_opt.is_some() {
                return Err("option '--model' cannot be specified more than once".into());
            }
            model_opt = Some(required_non_empty(&val, "--model")?);
            i += 1;
            continue;
        }
        if let Some(val) = take_opt(args, &mut i, "-t", "--thinking")? {
            if thinking_opt.is_some() {
                return Err("option '--thinking' cannot be specified more than once".into());
            }
            thinking_opt = Some(required_non_empty(&val, "--thinking")?);
            i += 1;
            continue;
        }
        if let Some(val) = take_opt(args, &mut i, "-s", "--session")? {
            if session_opt.is_some() {
                return Err("option '--session' cannot be specified more than once".into());
            }
            session_opt = Some(required_non_empty(&val, "--session")?);
            i += 1;
            continue;
        }
        if arg == "--first-turn" {
            first_turn = true;
            i += 1;
            continue;
        }
        if let Some(val) = take_opt(args, &mut i, "", "--edit-index")? {
            if edit_user_index.is_some() {
                return Err("option '--edit-index' cannot be specified more than once".into());
            }
            edit_user_index = Some(parse_usize_flag(&val, "--edit-index")?);
            i += 1;
            continue;
        }
        if let Some(val) = take_opt(args, &mut i, "-f", "--file")? {
            let p = required_non_empty(&val, "--file")?;
            files.push(PathBuf::from(p));
            i += 1;
            continue;
        }
        if let Some(val) = take_opt(args, &mut i, "", "--keep-file")? {
            let name = required_non_empty(&val, "--keep-file")?;
            keep_files.push(name);
            i += 1;
            continue;
        }
        reject_unknown_flag(arg, help_cmd)?;
        positionals.push(arg.to_string());
        i += 1;
    }

    let (site, session_id, remote_session_id) = match session_opt {
        Some(raw) => {
            let (s, full, remote) = resolve_session_target(site_opt, &raw)?;
            (s, Some(full), Some(remote))
        }
        None => (site_opt.unwrap_or_default(), None, None),
    };
    if edit_user_index.is_some() && session_id.is_none() {
        return Err(format!(
            "option '--edit-index' requires '--session <SESSION_ID>'; run '{help_cmd} --help' for usage"
        ));
    }
    if first_turn && (session_id.is_some() || edit_user_index.is_some()) {
        return Err(format!(
            "option '--first-turn' cannot be combined with '--session' or '--edit-index'; run '{help_cmd} --help' for usage"
        ));
    }
    if !keep_files.is_empty() && edit_user_index.is_none() {
        return Err(format!(
            "option '--keep-file' requires '--edit-index'; run '{help_cmd} --help' for usage"
        ));
    }
    let (model, thinking) =
        resolve_model_and_thinking(site, model_opt.as_deref(), thinking_opt.as_deref())?;

    let prompt = match positionals.as_slice() {
        [one] => one.clone(),
        [] => {
            if !std::io::stdin().is_terminal() {
                use std::io::Read;
                let mut buf = String::new();
                std::io::stdin()
                    .read_to_string(&mut buf)
                    .map_err(|e| format!("failed to read stdin: {e}"))?;
                if buf.trim().is_empty() && files.is_empty() && keep_files.is_empty() {
                    return Err(format!(
                        "missing <PROMPT>; run '{help_cmd} --help' for usage"
                    ));
                }
                buf
            } else if files.is_empty() && keep_files.is_empty() {
                return Err(format!(
                    "missing <PROMPT>; run '{help_cmd} --help' for usage"
                ));
            } else {
                String::new()
            }
        }
        [_, extra, ..] => {
            return Err(format!(
                "unexpected argument '{extra}'; run '{help_cmd} --help' for usage"
            ));
        }
    };

    Ok(WebCommand::Chat {
        site,
        model,
        thinking,
        prompt,
        session_id,
        remote_session_id,
        first_turn,
        edit_user_index,
        files,
        keep_files,
    })
}

fn parse_list(args: &[String]) -> Result<WebCommand, String> {
    let help_cmd = "lx-tui web list";
    let mut site: Option<AiWebSite> = None;
    let mut limit: Option<usize> = None;
    let mut i = 0;
    while i < args.len() {
        if let Some(val) = take_opt(args, &mut i, "", "--site")? {
            if site.is_some() {
                return Err("option '--site' cannot be specified more than once".into());
            }
            site = Some(parse_site(&val)?);
            i += 1;
            continue;
        }
        if let Some(val) = take_opt(args, &mut i, "", "--limit")? {
            if limit.is_some() {
                return Err("option '--limit' cannot be specified more than once".into());
            }
            limit = Some(parse_usize_flag(&val, "--limit")?);
            i += 1;
            continue;
        }
        reject_unknown_flag(&args[i], help_cmd)?;
        return Err(format!(
            "unexpected argument '{}'; run '{help_cmd} --help' for usage",
            args[i]
        ));
    }
    Ok(WebCommand::List {
        site: site.unwrap_or_default(),
        limit,
    })
}

fn parse_show(args: &[String]) -> Result<WebCommand, String> {
    let help_cmd = "lx-tui web show";
    let mut site_opt: Option<AiWebSite> = None;
    let mut show_thinking = false;
    let mut positionals: Vec<String> = Vec::new();
    let mut i = 0;
    while i < args.len() {
        let arg = args[i].as_str();
        if let Some(val) = take_opt(args, &mut i, "", "--site")? {
            if site_opt.is_some() {
                return Err("option '--site' cannot be specified more than once".into());
            }
            site_opt = Some(parse_site(&val)?);
            i += 1;
            continue;
        }
        if arg == "--thinking" {
            show_thinking = true;
            i += 1;
            continue;
        }
        reject_unknown_flag(arg, help_cmd)?;
        positionals.push(arg.to_string());
        i += 1;
    }
    let raw_id = match positionals.as_slice() {
        [id] => id,
        [] => {
            return Err(format!(
                "missing <SESSION_ID>; run '{help_cmd} --help' for usage"
            ));
        }
        [_, extra, ..] => {
            return Err(format!(
                "unexpected argument '{extra}'; run '{help_cmd} --help' for usage"
            ));
        }
    };
    let (site, session_id, remote_session_id) = resolve_session_target(site_opt, raw_id)?;
    Ok(WebCommand::Show {
        site,
        session_id,
        remote_session_id,
        show_thinking,
    })
}

fn parse_rename(args: &[String]) -> Result<WebCommand, String> {
    let help_cmd = "lx-tui web rename";
    let mut site_opt: Option<AiWebSite> = None;
    let mut positionals: Vec<String> = Vec::new();
    let mut literal = false;
    let mut i = 0;
    while i < args.len() {
        let arg = args[i].as_str();
        if literal {
            positionals.push(arg.to_string());
            i += 1;
            continue;
        }
        if arg == "--" {
            literal = true;
            i += 1;
            continue;
        }
        if let Some(val) = take_opt(args, &mut i, "", "--site")? {
            if site_opt.is_some() {
                return Err("option '--site' cannot be specified more than once".into());
            }
            site_opt = Some(parse_site(&val)?);
            i += 1;
            continue;
        }
        reject_unknown_flag(arg, help_cmd)?;
        positionals.push(arg.to_string());
        i += 1;
    }
    let (raw_id, raw_title) = match positionals.as_slice() {
        [id, title] => (id, title),
        [] => {
            return Err(format!(
                "missing <SESSION_ID> and <NEW_TITLE>; run '{help_cmd} --help' for usage"
            ));
        }
        [_] => {
            return Err(format!(
                "missing <NEW_TITLE>; run '{help_cmd} --help' for usage"
            ));
        }
        [_, _, extra, ..] => {
            return Err(format!(
                "unexpected argument '{extra}'; run '{help_cmd} --help' for usage"
            ));
        }
    };
    let title = required_non_empty(raw_title, "<NEW_TITLE>")?;
    let (site, session_id, remote_session_id) = resolve_session_target(site_opt, raw_id)?;
    Ok(WebCommand::Rename {
        site,
        session_id,
        remote_session_id,
        title,
    })
}

fn parse_delete(args: &[String]) -> Result<WebCommand, String> {
    let help_cmd = "lx-tui web delete";
    let mut site_opt: Option<AiWebSite> = None;
    let mut positionals: Vec<String> = Vec::new();
    let mut i = 0;
    while i < args.len() {
        let arg = args[i].as_str();
        if let Some(val) = take_opt(args, &mut i, "", "--site")? {
            if site_opt.is_some() {
                return Err("option '--site' cannot be specified more than once".into());
            }
            site_opt = Some(parse_site(&val)?);
            i += 1;
            continue;
        }
        reject_unknown_flag(arg, help_cmd)?;
        positionals.push(arg.to_string());
        i += 1;
    }
    let raw_id = match positionals.as_slice() {
        [id] => id,
        [] => {
            return Err(format!(
                "missing <SESSION_ID>; run '{help_cmd} --help' for usage"
            ));
        }
        [_, extra, ..] => {
            return Err(format!(
                "unexpected argument '{extra}'; run '{help_cmd} --help' for usage"
            ));
        }
    };
    let (site, session_id, remote_session_id) = resolve_session_target(site_opt, raw_id)?;
    Ok(WebCommand::Delete {
        site,
        session_id,
        remote_session_id,
    })
}
