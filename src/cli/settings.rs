//! `lx-tui settings`：通用布局设置读写与浏览器设置页服务。
use std::ffi::OsString;
use std::path::Path;

use serde_json::json;

use crate::config::{self, Config};
use crate::settings::commands::CommandScope;
use crate::settings::service::{CommandView, ProjectView, SettingsEvent, SettingsService};

use crate::cli::args::{
    GlobalOpts, extract_global_opts, parse_u64_flag, reject_unknown_flag, take_opt,
};
use crate::cli::context::WorkspaceContext;
use crate::cli::output::Output;
use crate::cli::{CliAction, EXIT_OK, HelpTopic};

/// `lx-tui settings` 顶层参数。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SettingsCli {
    pub global: GlobalOpts,
    pub command: SettingsCommand,
}

/// `settings` 子命令。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SettingsCommand {
    Show,
    Set(SetArgs),
    Reset,
    Open { no_open: bool },
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SetArgs {
    pub sidebar_width: Option<u16>,
    pub sidebar_min_width: Option<u16>,
    pub sidebar_max_width: Option<u16>,
    pub narrow_width: Option<u16>,
    pub min_width: Option<u16>,
    pub min_height: Option<u16>,
    pub min_pane_width: Option<u16>,
    pub min_pane_height: Option<u16>,
}

/// 解析 `lx-tui settings` 参数。
pub fn parse(raw_args: &[OsString]) -> Result<CliAction, String> {
    let (global, rest, help) = extract_global_opts(raw_args)?;
    if help {
        return Ok(CliAction::Help(HelpTopic::Settings));
    }
    let Some((sub, tail)) = rest.split_first() else {
        return Err(
            "missing 'settings' subcommand; run 'lx-tui settings --help' for usage".to_string(),
        );
    };
    let command = match sub.as_str() {
        "show" => {
            no_arguments(tail, "lx-tui settings show")?;
            SettingsCommand::Show
        }
        "set" => SettingsCommand::Set(parse_set(tail)?),
        "reset" => {
            no_arguments(tail, "lx-tui settings reset")?;
            SettingsCommand::Reset
        }
        "open" => SettingsCommand::Open {
            no_open: parse_open(tail)?,
        },
        other => {
            return Err(format!(
                "unknown 'settings' subcommand '{other}'; run 'lx-tui settings --help' for usage"
            ));
        }
    };
    Ok(CliAction::Settings(SettingsCli { global, command }))
}

/// 执行 `lx-tui settings`。
pub fn run(
    cli: SettingsCli,
    linked_worktree: Option<&Path>,
) -> Result<i32, Box<dyn std::error::Error>> {
    let ctx = WorkspaceContext::resolve(cli.global.cwd.as_deref(), linked_worktree)?;
    let out = Output::new(cli.global.json, cli.global.color);
    match cli.command {
        SettingsCommand::Show => run_show(&ctx, &out)?,
        SettingsCommand::Set(args) => run_set(&out, &args)?,
        SettingsCommand::Reset => run_reset(&out)?,
        SettingsCommand::Open { no_open } => run_open(&ctx, &out, no_open)?,
    }
    Ok(EXIT_OK)
}

fn no_arguments(args: &[String], help_cmd: &str) -> Result<(), String> {
    if let Some(arg) = args.first() {
        reject_unknown_flag(arg, help_cmd)?;
        return Err(format!(
            "unexpected argument '{arg}'; run '{help_cmd} --help' for usage"
        ));
    }
    Ok(())
}

fn parse_open(args: &[String]) -> Result<bool, String> {
    let mut no_open = false;
    for arg in args {
        if arg == "--no-open" {
            no_open = true;
            continue;
        }
        reject_unknown_flag(arg, "lx-tui settings open")?;
        return Err(format!(
            "unexpected argument '{arg}'; run 'lx-tui settings open --help' for usage"
        ));
    }
    Ok(no_open)
}

const SET_FLAGS: [&str; 8] = [
    "--sidebar-width",
    "--sidebar-min-width",
    "--sidebar-max-width",
    "--narrow-width",
    "--min-width",
    "--min-height",
    "--min-pane-width",
    "--min-pane-height",
];

fn parse_u16(value: &str, flag: &str) -> Result<u16, String> {
    let parsed = parse_u64_flag(value, flag)?;
    u16::try_from(parsed).map_err(|_| format!("'{flag}' must be between 0 and 65535"))
}

fn parse_set(args: &[String]) -> Result<SetArgs, String> {
    let mut parsed = SetArgs::default();
    let mut pending: Vec<(&str, u16)> = Vec::new();
    let mut i = 0;
    while i < args.len() {
        let mut matched = false;
        for flag in SET_FLAGS {
            if let Some(value) = take_opt(args, &mut i, "", flag)? {
                pending.push((flag, parse_u16(&value, flag)?));
                i += 1;
                matched = true;
                break;
            }
        }
        if matched {
            continue;
        }
        reject_unknown_flag(&args[i], "lx-tui settings set")?;
        return Err(format!(
            "unexpected argument '{}'; run 'lx-tui settings set --help' for usage",
            args[i]
        ));
    }
    for (flag, value) in pending {
        match flag {
            "--sidebar-width" => parsed.sidebar_width = Some(value),
            "--sidebar-min-width" => parsed.sidebar_min_width = Some(value),
            "--sidebar-max-width" => parsed.sidebar_max_width = Some(value),
            "--narrow-width" => parsed.narrow_width = Some(value),
            "--min-width" => parsed.min_width = Some(value),
            "--min-height" => parsed.min_height = Some(value),
            "--min-pane-width" => parsed.min_pane_width = Some(value),
            "--min-pane-height" => parsed.min_pane_height = Some(value),
            _ => {}
        }
    }
    Ok(parsed)
}

fn config_json(config: &Config) -> serde_json::Value {
    json!({
        "layout": {
            "sidebarWidth": config.sidebar_width,
            "sidebarMinWidth": config.sidebar_min_width,
            "sidebarMaxWidth": config.sidebar_max_width,
            "narrowWidth": config.narrow_width,
            "minWidth": config.min_width,
            "minHeight": config.min_height,
            "minPaneWidth": config.min_pane_width,
            "minPaneHeight": config.min_pane_height,
        }
    })
}

fn run_show(ctx: &WorkspaceContext, out: &Output) -> Result<(), String> {
    let config = config::load_settings().map_err(|error| error.to_string())?;
    let data_root = config::user_data_root().map_err(|error| error.to_string())?;
    let settings_path = config::settings_path().map_err(|error| error.to_string())?;
    let db_path = ctx.db_path()?;
    let ai_path = config::ai::ai_config_path().map_err(|error| error.to_string())?;
    let mcp_path = config::mcp::user_mcp_path().map_err(|error| error.to_string())?;
    let skills_path = config::skills::skills_config_path().map_err(|error| error.to_string())?;
    let subagents_path =
        config::subagents::subagents_config_path().map_err(|error| error.to_string())?;
    let value = json!({
        "layout": config_json(&config)["layout"],
        "paths": {
            "dataRoot": data_root,
            "settings": settings_path,
            "database": db_path,
            "ai": ai_path,
            "mcp": mcp_path,
            "skills": skills_path,
            "subagents": subagents_path,
        }
    });
    out.emit(&value, |_, w| {
        writeln!(w, "{:<22}{}", "Sidebar width:", config.sidebar_width)?;
        writeln!(
            w,
            "{:<22}{}",
            "Sidebar min width:", config.sidebar_min_width
        )?;
        writeln!(
            w,
            "{:<22}{}",
            "Sidebar max width:", config.sidebar_max_width
        )?;
        writeln!(w, "{:<22}{}", "Narrow width:", config.narrow_width)?;
        writeln!(w, "{:<22}{}", "Min width:", config.min_width)?;
        writeln!(w, "{:<22}{}", "Min height:", config.min_height)?;
        writeln!(w, "{:<22}{}", "Min pane width:", config.min_pane_width)?;
        writeln!(w, "{:<22}{}", "Min pane height:", config.min_pane_height)?;
        writeln!(w)?;
        writeln!(w, "{:<22}{}", "Data root:", data_root.display())?;
        writeln!(w, "{:<22}{}", "Settings file:", settings_path.display())?;
        writeln!(w, "{:<22}{}", "Database:", db_path.display())?;
        writeln!(w, "{:<22}{}", "AI config:", ai_path.display())?;
        writeln!(w, "{:<22}{}", "MCP config:", mcp_path.display())?;
        writeln!(w, "{:<22}{}", "Skills config:", skills_path.display())?;
        writeln!(w, "{:<22}{}", "Subagents config:", subagents_path.display())
    })
}

fn run_set(out: &Output, args: &SetArgs) -> Result<(), String> {
    let mut config = config::load_settings().map_err(|error| error.to_string())?;
    if let Some(value) = args.sidebar_width {
        config.sidebar_width = value;
    }
    if let Some(value) = args.sidebar_min_width {
        config.sidebar_min_width = value;
    }
    if let Some(value) = args.sidebar_max_width {
        config.sidebar_max_width = value;
    }
    if let Some(value) = args.narrow_width {
        config.narrow_width = value;
    }
    if let Some(value) = args.min_width {
        config.min_width = value;
    }
    if let Some(value) = args.min_height {
        config.min_height = value;
    }
    if let Some(value) = args.min_pane_width {
        config.min_pane_width = value;
    }
    if let Some(value) = args.min_pane_height {
        config.min_pane_height = value;
    }
    config.validate().map_err(|error| error.to_string())?;
    config::save_settings(&config).map_err(|error| error.to_string())?;
    let value = config_json(&config);
    out.emit(&value, |_, w| {
        writeln!(w, "Settings saved.")?;
        writeln!(w, "{:<22}{}", "Sidebar width:", config.sidebar_width)?;
        writeln!(w, "{:<22}{}", "Narrow width:", config.narrow_width)?;
        writeln!(w, "{:<22}{}", "Min width:", config.min_width)?;
        writeln!(w, "{:<22}{}", "Min height:", config.min_height)
    })
}

fn run_reset(out: &Output) -> Result<(), String> {
    let config = Config::default();
    config::save_settings(&config).map_err(|error| error.to_string())?;
    let value = config_json(&config);
    out.emit(&value, |_, w| {
        writeln!(w, "Settings reset to defaults.")?;
        writeln!(w, "{:<22}{}", "Sidebar width:", config.sidebar_width)?;
        writeln!(w, "{:<22}{}", "Narrow width:", config.narrow_width)
    })
}

fn run_open(ctx: &WorkspaceContext, out: &Output, no_open: bool) -> Result<(), String> {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .map_err(|error| format!("failed to initialize async runtime: {error}"))?;
    runtime.block_on(async {
        let (sender, mut receiver) = tokio::sync::mpsc::unbounded_channel::<SettingsEvent>();
        let service = SettingsService::start(sender)
            .await
            .map_err(|error| format!("failed to start settings server: {error}"))?;
        if let Ok(db) = ctx.open_db() {
            let trusted = db.is_trusted(&ctx.workspace).unwrap_or(false);
            let name = ctx
                .workspace
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_else(|| "workspace".to_string());
            let scope = crate::platform::path_text::scope_dir_name(&ctx.workspace)
                .unwrap_or_else(|| "workspace".to_string());
            service.set_projects(vec![ProjectView {
                scope,
                root: ctx.workspace.clone(),
                name,
                trusted,
            }]);
        }
        if let Ok(user_root) = config::user_data_root() {
            let (records, _) = crate::settings::commands::list(CommandScope::User, &user_root);
            let views: Vec<CommandView> = records
                .iter()
                .map(|record| CommandView::of(record, CommandScope::User))
                .collect();
            service.replace_user_commands(views);
        }
        let url = service.url();
        let value = json!({
            "url": url,
            "port": service.port(),
            "workspace": ctx.workspace,
        });
        out.emit(&value, |_, w| {
            writeln!(w, "Settings server running at: {url}")?;
            writeln!(w, "Press Ctrl+C to stop.")
        })?;
        if !no_open {
            let _ = crate::platform::open_browser(&url);
        }
        loop {
            tokio::select! {
                _ = tokio::signal::ctrl_c() => break,
                event = receiver.recv() => {
                    if event.is_none() {
                        break;
                    }
                }
            }
        }
        service.shutdown();
        crate::mcp::manager::manager().shutdown().await;
        Ok::<(), String>(())
    })
}
