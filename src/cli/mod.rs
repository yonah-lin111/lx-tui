//! 全功能无头 CLI 入口：顶层路由、通用参数扫描、工作区上下文与双模式输出。

pub mod args;
pub mod command;
pub mod context;
pub mod help;
pub mod mcp;
pub mod model;
pub mod output;
pub mod prompt;
pub mod session;
pub mod settings;
pub mod skill;
pub mod subagent;
pub mod todo;
pub mod workspace;

#[cfg(test)]
mod tests;

use std::ffi::OsString;
use std::path::Path;

use crate::chat::agent::exec::{self, ExecCli};
pub use crate::chat::agent::exec::{EXIT_ERR, EXIT_INTERRUPTED, EXIT_OK};
pub use help::{HelpTopic, help_text};

/// 顶层 CLI 动作枚举：进程启动时一次性解析。
///
/// 子命令结构体携带完整解析结果，枚举值体积差异大；进程只解析一次，
/// 用堆分配换解析路径的零拷贝不划算，因此保留内联并显式豁免该 lint。
#[derive(Debug, Clone, PartialEq, Eq)]
#[allow(clippy::large_enum_variant)]
pub enum CliAction {
    /// 无参数：进入全屏 TUI。
    Tui,
    /// `-h` / `--help` 或 `lx-tui help [topic...]`。
    Help(HelpTopic),
    /// `-V` / `--version` / `version`。
    Version,
    /// `lx-tui exec ...`：无头运行 Agent。
    Exec(ExecCli),
    /// `lx-tui session ...`：聊天会话管理。
    Session(session::SessionCli),
    /// `lx-tui prompt ...`：提示词库管理。
    Prompt(prompt::PromptCli),
    /// `lx-tui todo ...`：全局日程待办管理。
    Todo(todo::TodoCli),
    /// `lx-tui workspace ...`：工作区历史、信任与 Git worktree。
    Workspace(workspace::WorkspaceCli),
    /// `lx-tui model ...`：AI 模型与供应商配置。
    Model(model::ModelCli),
    /// `lx-tui mcp ...`：MCP 服务管理与探测/授权。
    Mcp(mcp::McpCli),
    /// `lx-tui skill ...`：Skills 发现、启停与读写。
    Skill(skill::SkillCli),
    /// `lx-tui command ...`：自定义斜杠命令管理。
    Command(command::CommandCli),
    /// `lx-tui subagent ...`：子 Agent 角色与并发配置。
    Subagent(subagent::SubagentCli),
    /// `lx-tui settings ...`：通用设置与浏览器设置服务。
    Settings(settings::SettingsCli),
}

/// 解析进程命令行参数（首项为二进制名 `argv[0]`）。
pub fn parse_cli<I, T>(args: I) -> Result<CliAction, String>
where
    I: IntoIterator<Item = T>,
    T: Into<OsString>,
{
    let args: Vec<OsString> = args.into_iter().map(Into::into).collect();
    let Some(first) = args.get(1) else {
        return Ok(CliAction::Tui);
    };
    let sub = args::utf8(first)?;
    let tail = &args[2..];
    match sub {
        "-h" | "--help" => Ok(CliAction::Help(HelpTopic::Root)),
        "-V" | "--version" | "version" => Ok(CliAction::Version),
        "help" => help::parse_help(tail),
        "exec" => match exec::parse_cli(args) {
            Ok(exec::CliAction::Exec(cli)) => Ok(CliAction::Exec(cli)),
            Ok(exec::CliAction::Help(topic)) => Ok(CliAction::Help(HelpTopic::from_exec(topic))),
            Ok(exec::CliAction::Version) => Ok(CliAction::Version),
            Ok(exec::CliAction::Tui) => Ok(CliAction::Tui),
            Err(err) => Err(err),
        },
        "session" => session::parse(tail),
        "prompt" => prompt::parse(tail),
        "todo" => todo::parse(tail),
        "workspace" => workspace::parse(tail),
        "model" => model::parse(tail),
        "mcp" => mcp::parse(tail),
        "skill" => skill::parse(tail),
        "command" => command::parse(tail),
        "subagent" => subagent::parse(tail),
        "settings" => settings::parse(tail),
        other => Err(format!(
            "unexpected argument '{other}'; run 'lx-tui --help' for usage"
        )),
    }
}

/// 执行除 `CliAction::Tui` 外的全部 CLI 动作。
pub fn run_cli(
    action: CliAction,
    linked_worktree: Option<&Path>,
) -> Result<i32, Box<dyn std::error::Error>> {
    match action {
        CliAction::Tui => Ok(EXIT_OK),
        CliAction::Help(topic) => {
            print!("{}", help_text(topic));
            Ok(EXIT_OK)
        }
        CliAction::Version => {
            println!("lx-tui {}", env!("CARGO_PKG_VERSION"));
            Ok(EXIT_OK)
        }
        CliAction::Exec(cli) => exec::run_main(cli, linked_worktree),
        CliAction::Session(cli) => session::run(cli, linked_worktree),
        CliAction::Prompt(cli) => prompt::run(cli, linked_worktree),
        CliAction::Todo(cli) => todo::run(cli, linked_worktree),
        CliAction::Workspace(cli) => workspace::run(cli, linked_worktree),
        CliAction::Model(cli) => model::run(cli),
        CliAction::Mcp(cli) => mcp::run(cli, linked_worktree),
        CliAction::Skill(cli) => skill::run(cli, linked_worktree),
        CliAction::Command(cli) => command::run(cli, linked_worktree),
        CliAction::Subagent(cli) => subagent::run(cli, linked_worktree),
        CliAction::Settings(cli) => settings::run(cli, linked_worktree),
    }
}
