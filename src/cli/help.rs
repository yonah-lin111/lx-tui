//! 全局与各领域子命令的英文帮助主题与文本常量。

use std::ffi::OsString;

use crate::chat::agent::exec;

use super::CliAction;
use super::args::utf8;

/// CLI 帮助主题枚举。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HelpTopic {
    Root,
    Exec,
    ExecResume,
    Session,
    Prompt,
    Todo,
    Workspace,
    Model,
    Mcp,
    Skill,
    Command,
    Subagent,
    Settings,
}

impl HelpTopic {
    pub fn from_exec(topic: exec::HelpTopic) -> Self {
        match topic {
            exec::HelpTopic::Root => Self::Root,
            exec::HelpTopic::Exec => Self::Exec,
            exec::HelpTopic::ExecResume => Self::ExecResume,
        }
    }
}

/// 解析 `lx-tui help [topic...]`。
pub fn parse_help(topics: &[OsString]) -> Result<CliAction, String> {
    let first = topics.first().map(|s| utf8(s)).transpose()?;
    let second = topics.get(1).map(|s| utf8(s)).transpose()?;
    if topics.len() > 2 {
        return Err("too many arguments for 'help'".to_string());
    }
    let topic = match (first, second) {
        (None, _) => HelpTopic::Root,
        (Some("exec"), None) => HelpTopic::Exec,
        (Some("exec"), Some("resume")) | (Some("resume"), None) => HelpTopic::ExecResume,
        (Some("session"), None) => HelpTopic::Session,
        (Some("prompt"), None) => HelpTopic::Prompt,
        (Some("todo"), None) => HelpTopic::Todo,
        (Some("workspace"), None) => HelpTopic::Workspace,
        (Some("model"), None) => HelpTopic::Model,
        (Some("mcp"), None) => HelpTopic::Mcp,
        (Some("skill"), None) => HelpTopic::Skill,
        (Some("command"), None) => HelpTopic::Command,
        (Some("subagent"), None) => HelpTopic::Subagent,
        (Some("settings"), None) => HelpTopic::Settings,
        (Some(other), _) => return Err(format!("unknown help topic '{other}'")),
    };
    Ok(CliAction::Help(topic))
}

/// 返回对应主题的英文帮助文本。
pub fn help_text(topic: HelpTopic) -> &'static str {
    match topic {
        HelpTopic::Root => ROOT_HELP,
        HelpTopic::Exec => exec::help_text(exec::HelpTopic::Exec),
        HelpTopic::ExecResume => exec::help_text(exec::HelpTopic::ExecResume),
        HelpTopic::Session => SESSION_HELP,
        HelpTopic::Prompt => PROMPT_HELP,
        HelpTopic::Todo => TODO_HELP,
        HelpTopic::Workspace => WORKSPACE_HELP,
        HelpTopic::Model => MODEL_HELP,
        HelpTopic::Mcp => MCP_HELP,
        HelpTopic::Skill => SKILL_HELP,
        HelpTopic::Command => COMMAND_HELP,
        HelpTopic::Subagent => SUBAGENT_HELP,
        HelpTopic::Settings => SETTINGS_HELP,
    }
}

const ROOT_HELP: &str = "\
lx-tui - Terminal agent workspace and headless CLI

Usage:
  lx-tui                        Launch the interactive TUI
  lx-tui <COMMAND> [OPTIONS]

Commands:
  exec        Run lx agent non-interactively in the terminal
  session     List, inspect, rename, delete, fork, rewind, or compact chat sessions
  prompt      Manage workspace prompts, folders, pages, and referenced directories
  todo        Manage daily todos, priorities, rollovers, and completion stats
  workspace   Manage workspace history, trust state, and Git worktrees
  model       Configure AI providers, models, defaults, and fetch remote models
  mcp         Configure, enable/disable, probe, and authorize MCP servers
  skill       Discover, enable/disable, create, edit, and delete skills
  command     Manage custom slash commands (agentMD and agentBlock)
  subagent    Configure subagent concurrency, depth, builtins, and custom roles
  settings    Inspect/update layout settings or open the browser settings UI
  help        Print help for lx-tui or a subcommand
  version     Print version information

Global Options (supported across domain subcommands):
  -C, --cd <DIR>                Run as if started in <DIR>
      --json                    Print structured JSON output to stdout
      --color <auto|always|never>  Control ANSI color in human output [default: auto]
  -h, --help                    Print help
  -V, --version                 Print version
";

const SESSION_HELP: &str = "\
Manage workspace chat sessions

Usage:
  lx-tui session <SUBCOMMAND> [OPTIONS]

Subcommands:
  list [--limit <N>] [--offset <N>]
      List sessions in the target workspace
  show (<SESSION_ID> | --last) [--tools] [--thinking]
      Show messages, active compact checkpoint, and notices for a session
  rename (<SESSION_ID> | --last) <NEW_TITLE>
      Rename a session
  delete (<SESSION_ID> | --last)
      Delete a session (fails if the session is active in another process)
  fork (<SESSION_ID> | --last) [--before <USER_MESSAGE_ID>]
      Fork a session into a new session ID
  rewind (<SESSION_ID> | --last) [--mode <MODE>] [--dry-run]
      Rewind the latest user turn or restore compact checkpoint
      Modes: revert-and-delete (default), delete-only, revert-only, restore-compact
  compact (<SESSION_ID> | --last) [-m, --model <MODEL>] [--revert]
      Compact completed turns into a summary checkpoint, or revert the active checkpoint
  agents (<SESSION_ID> | --last) [--tree] [--filter <all|active|done>]
      List subagents spawned by a session
  processes (<SESSION_ID> | --last)
      List background/command processes recorded for a session

Global Options:
  -C, --cd <DIR>                Working directory
      --json                    Output as JSON
      --color <auto|always|never>
  -h, --help                    Print help
";

const PROMPT_HELP: &str = "\
Manage workspace prompts, folders, multi-page content, and referenced directories

Usage:
  lx-tui prompt <SUBCOMMAND> [OPTIONS]

Prompt Subcommands:
  list [--folder <FOLDER_ID>]
  show <PROMPT_ID> [--page <PAGE_ID_OR_INDEX>]
  create <NAME> [--folder <FOLDER_ID>] [--page-name <NAME>] [--content <TEXT> | -f <FILE>]
  edit <PROMPT_ID> [--page <PAGE_ID>] [--content <TEXT> | -f <FILE>]
  rename <PROMPT_ID> <NEW_NAME>
  move <PROMPT_ID> (--folder <FOLDER_ID> | --root)
  delete <PROMPT_ID>

Folder Subcommands:
  folder list
  folder create <NAME> [--parent <FOLDER_ID>]
  folder rename <FOLDER_ID> <NEW_NAME>
  folder move <FOLDER_ID> (--parent <FOLDER_ID> | --root)
  folder delete <FOLDER_ID>

Page Subcommands:
  page list <PROMPT_ID>
  page create <PROMPT_ID> <PAGE_NAME> [--content <TEXT> | -f <FILE>]
  page edit <PAGE_ID> [--content <TEXT> | -f <FILE>]
  page rename <PAGE_ID> <NEW_NAME>
  page active <PROMPT_ID> <PAGE_ID>
  page delete <PAGE_ID>

Referenced Folder Subcommands:
  ref list
  ref add <DIR_PATH>
  ref remove <REF_ID>

Global Options:
  -C, --cd <DIR>                Working directory
      --json                    Output as JSON
      --color <auto|always|never>
  -h, --help                    Print help
";

const TODO_HELP: &str = "\
Manage daily todos and completion statistics

Usage:
  lx-tui todo <SUBCOMMAND> [OPTIONS]

Subcommands:
  list [--date <YYYY-MM-DD> | --all | --past-incomplete] [--status <todo|in_progress|done>] [-p, --priority <P0|P1|P2|P3>]
  add <TEXT> [--date <YYYY-MM-DD>] [-p, --priority <P0|P1|P2|P3>] [--status <todo|in_progress|done>]
  update <TODO_ID> [--text <TEXT>] [--status <todo|in_progress|done>] [-p, --priority <P0|P1|P2|P3>] [--date <YYYY-MM-DD>]
  done <TODO_ID>
  start <TODO_ID>
  reopen <TODO_ID>
  delete <TODO_ID>
  reorder --date <YYYY-MM-DD> <ID1> <ID2> ...
  rollover [--to <YYYY-MM-DD>] [--id <TODO_ID>...]
  stats [--days <N> | --from <YYYY-MM-DD> --to <YYYY-MM-DD>]

Global Options:
  -C, --cd <DIR>                Working directory
      --json                    Output as JSON
      --color <auto|always|never>
  -h, --help                    Print help
";

const WORKSPACE_HELP: &str = "\
Manage workspace history list, trust state, and Git worktrees

Usage:
  lx-tui workspace <SUBCOMMAND> [OPTIONS]

Subcommands:
  list [--raw] [--search <QUERY>]
      List workspace history entries (tree view by default, or flat records with --raw)
  status
      Show current workspace trust state, Git branch, worktree info, and DB path
  add [<DIR>] [--name <NAME>] [--trust]
      Record a workspace in the history list (and optionally grant trust)
  rename <NEW_NAME>
      Rename the target workspace in history
  delete [<DIR>] [--include-worktrees]
      Cascade-delete a workspace from history (fails if any session is active)
  reorder <PATH1> <PATH2> ...
      Save explicit global display order for workspaces
  trust [--name <NAME>]
      Mark the target workspace as trusted
  worktrees
      List Git worktrees for the current repository

Global Options:
  -C, --cd <DIR>                Target workspace directory
      --json                    Output as JSON
      --color <auto|always|never>
  -h, --help                    Print help
";

const MODEL_HELP: &str = "\
Configure AI providers, models, default selections, and remote model discovery

Usage:
  lx-tui model <SUBCOMMAND> [OPTIONS]

Subcommands:
  list [--all]
  show [<PROVIDER_ID>[/<MODEL_ID>]]
  default [<PROVIDER>/<MODEL>[#<VARIANT>] | --clear] [--title-summary]
  provider-set <PROVIDER_ID> [--type <openai|anthropic|google|openai-compatible>] [--name <NAME>]
               [--base-url <URL>] [--api-key <KEY> | --clear-api-key] [--enable | --disable]
  provider-rm <PROVIDER_ID>
  model-set <PROVIDER_ID>/<MODEL_ID> [--name <NAME>] [--transport <TRANSPORT>]
            [--context-limit <N>] [--output-limit <N>] [--variant <NAME>] [--image-input <true|false>]
  model-rm <PROVIDER_ID>/<MODEL_ID>
  fetch <PROVIDER_ID> [--base-url <URL>] [--api-key <KEY>] [--save]
  config [--title-language <auto|zh-CN|zh-TW|en|ja|ko>] [--timeout-ms <MS>]
         [--exa-key <KEY>] [--tavily-key <KEY>]

Global Options:
  -C, --cd <DIR>                Working directory
      --json                    Output as JSON
      --color <auto|always|never>
  -h, --help                    Print help
";

const MCP_HELP: &str = "\
Configure, enable/disable, probe, and authorize MCP servers

Usage:
  lx-tui mcp <SUBCOMMAND> [OPTIONS]

Subcommands:
  list [--scope <all|user|project>]
  show <NAME> [--scope <user|project>]
  add-stdio <NAME> --command <CMD> [--arg <ARG>...] [--cwd <DIR>] [--env <KEY=VAL>...]
            [--env-var <VAR>...] [--startup-timeout <SEC>] [--tool-timeout <SEC>] [--scope <user|project>]
  add-http <NAME> --url <URL> [--auth <none|bearer|oauth>] [--bearer-token <TOKEN>]
           [--bearer-env <VAR>] [--header <KEY=VAL>...] [--scope-item <SCOPE>...]
           [--client-id <ID>] [--client-secret <SECRET>] [--startup-timeout <SEC>]
           [--tool-timeout <SEC>] [--scope <user|project>]
  remove <NAME> [--scope <user|project>]
  enable <NAME> [--scope <user|project>]
  disable <NAME> [--scope <user|project>]
  probe <NAME> [--scope <user|project>]
  login <NAME> [--scope <user|project>] [--no-open]
  logout <NAME> [--scope <user|project>]

Global Options:
  -C, --cd <DIR>                Working directory for project scope
      --json                    Output as JSON
      --color <auto|always|never>
  -h, --help                    Print help
";

const SKILL_HELP: &str = "\
Discover, enable/disable, create, edit, and delete skills

Usage:
  lx-tui skill <SUBCOMMAND> [OPTIONS]

Subcommands:
  list [--scope <all|user|project>]
  show <NAME> [--scope <user|project>]
  enable <NAME_OR_PATH>
  disable <NAME_OR_PATH>
  create <NAME> --description <DESC> [--content <TEXT> | -f <FILE>] [--scope <user|project>]
  edit <NAME> [--description <DESC>] [--content <TEXT> | -f <FILE>] [--scope <user|project>]
  delete <NAME> [--scope <user|project>]

Global Options:
  -C, --cd <DIR>                Working directory for project scope
      --json                    Output as JSON
      --color <auto|always|never>
  -h, --help                    Print help
";

const COMMAND_HELP: &str = "\
Manage custom slash commands (agentMD and agentBlock)

Usage:
  lx-tui command <SUBCOMMAND> [OPTIONS]

Subcommands:
  list [--scope <all|user|project>] [--kind <agentMD|agentBlock>]
  show <NAME> [--scope <user|project>] [--kind <agentMD|agentBlock>]
  create <NAME> --kind <agentMD|agentBlock> --description <DESC>
         [--md-scope <global|template>] [--block <template|supple>]
         [--arg-hint <HINT>] [--title <TITLE>] [--content <TEXT> | -f <FILE>]
         [--scope <user|project>] [--overwrite]
  edit <NAME> [--scope <user|project>] [--kind <agentMD|agentBlock>]
       [--description <DESC>] [--md-scope <global|template>] [--block <template|supple>]
       [--arg-hint <HINT>] [--title <TITLE>] [--content <TEXT> | -f <FILE>]
  rename <OLD_NAME> <NEW_NAME> [--scope <user|project>] [--kind <agentMD|agentBlock>]
  delete <NAME> [--scope <user|project>] [--kind <agentMD|agentBlock>]

Global Options:
  -C, --cd <DIR>                Working directory for project scope
      --json                    Output as JSON
      --color <auto|always|never>
  -h, --help                    Print help
";

const SUBAGENT_HELP: &str = "\
Configure subagent concurrency, depth, delegation mode, builtins, and custom roles

Usage:
  lx-tui subagent <SUBCOMMAND> [OPTIONS]

Subcommands:
  show [<ROLE_NAME>] [--scope <all|user|project>]
  set [--max-concurrent <1..12>] [--max-depth <1..3>]
      [--default-max-tokens <N> | --clear-max-tokens]
      [--delegation-mode <auto|proactive|explicit>]
  role-set <NAME> [--base <explorer|executor|monitor>] [--description <DESC>]
           [--prompt <TEXT> | -f <FILE>] [--model <PROVIDER>/<MODEL>[#<VARIANT>] | --clear-model]
           [--lock-model <true|false>] [--allow-web-search <true|false>]
           [--allow-mcp <true|false>] [--allow-skills <true|false>]
           [--nicknames <N1,N2,...>] [--scope <user|project>]
  role-reset <explorer|executor|monitor>
  role-rm <CUSTOM_ROLE_NAME> [--scope <user|project>]

Global Options:
  -C, --cd <DIR>                Working directory for project-scoped roles
      --json                    Output as JSON
      --color <auto|always|never>
  -h, --help                    Print help
";

const SETTINGS_HELP: &str = "\
Inspect/update layout settings or open the browser settings UI

Usage:
  lx-tui settings <SUBCOMMAND> [OPTIONS]

Subcommands:
  show
      Show layout settings and configuration file paths
  set [--sidebar-width <N>] [--sidebar-min-width <N>] [--sidebar-max-width <N>]
      [--narrow-width <N>] [--min-width <N>] [--min-height <N>]
      [--min-pane-width <N>] [--min-pane-height <N>]
      Update layout settings in settings.json
  reset
      Reset settings.json to default values
  open [--no-open]
      Start the local settings web server on 127.0.0.1 until Ctrl+C

Global Options:
  -C, --cd <DIR>                Working directory for project settings context
      --json                    Output as JSON
      --color <auto|always|never>
  -h, --help                    Print help
";
