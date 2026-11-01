//! `lx-tui exec` 命令行参数解析（零外部依赖、无 IO 副作用）。
//!
//! 解析器只做纯字符串/路径到强类型结构的转换：`argv[0]` 由调用方保留，
//! `stdin` 与终端探测通过参数注入，因此全部行为都能在单元测试里覆盖。
//! 用户可见文案（帮助、错误）一律英文。

use std::ffi::{OsStr, OsString};
use std::path::PathBuf;

use crate::config::ai::{AiSettings, ProviderConfig};

/// 顶层命令行动作。
#[derive(Debug, Clone, PartialEq, Eq)]
// `CliAction` 只在进程启动时分发一次；保持 `Exec(ExecCli)` 的直白结构与设计一致。
#[allow(clippy::large_enum_variant)]
pub enum CliAction {
    /// 无参数：进入全屏 TUI（默认既有行为）。
    Tui,
    /// `lx-tui exec ...`：终端无头执行 agent。
    Exec(ExecCli),
    /// `-h` / `--help` 或 `lx-tui help [exec [resume]]`。
    Help(HelpTopic),
    /// `-V` / `--version`。
    Version,
}

/// 帮助主题。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HelpTopic {
    Root,
    Exec,
    ExecResume,
}

/// `--color` 取值：控制 stderr 人类可读输出的 ANSI 颜色。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ColorMode {
    #[default]
    Auto,
    Always,
    Never,
}

impl ColorMode {
    /// 解析稳定字符串值；大小写不敏感。
    pub fn parse(raw: &str) -> Option<Self> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "auto" => Some(Self::Auto),
            "always" => Some(Self::Always),
            "never" => Some(Self::Never),
            _ => None,
        }
    }

    /// 是否输出颜色；`Auto` 仅在目标流是终端时启用。
    pub fn should_color(self, is_terminal: bool) -> bool {
        match self {
            Self::Auto => is_terminal,
            Self::Always => true,
            Self::Never => false,
        }
    }

    /// 把 `NO_COLOR` 状态一并纳入判断：`Auto` 下用户显式禁用则不再着色。
    pub fn should_color_with_env(self, is_terminal: bool, no_color_set: bool) -> bool {
        match self {
            Self::Auto => is_terminal && !no_color_set,
            Self::Always => true,
            Self::Never => false,
        }
    }
}

/// 读取 `NO_COLOR`：存在且非空即视为禁用颜色（见 <https://no-color.org>）。
pub fn no_color_requested() -> bool {
    std::env::var_os("NO_COLOR").is_some_and(|value| !value.is_empty())
}

/// `lx-tui exec` 命令行参数。
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ExecCli {
    /// 子命令：`None` 表示新建一轮会话，`Some(Resume)` 表示恢复既有会话。
    pub command: Option<ExecCommand>,
    /// 位置参数提示词；为 `None` 或 `Some("-")` 时从 stdin 读取。
    pub prompt: Option<String>,
    /// `-i, --image <FILE>`：附加到首条用户消息的本地图片（可多次或逗号分隔）。
    pub images: Vec<PathBuf>,
    /// `-m, --model <MODEL>`：`<provider>/<model>[#<variant>]` 或纯 `<model_id>`。
    pub model: Option<String>,
    /// `-C, --cd <DIR>`：执行工作目录（默认当前进程 cwd）。
    pub cwd: Option<PathBuf>,
    /// `--json`：向 stdout 输出 JSONL 事件流。
    pub json: bool,
    /// `-o, --output-last-message <FILE>`：将最终回复写入文件。
    pub last_message_file: Option<PathBuf>,
    /// `--ephemeral`：使用内存数据库，不落盘。
    pub ephemeral: bool,
    /// `--color <auto|always|never>`。
    pub color: ColorMode,
}

/// `exec` 子命令。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExecCommand {
    /// `lx-tui exec resume [SESSION_ID | --last] [OPTIONS] [PROMPT]`
    Resume(ResumeArgs),
}

/// `resume` 参数。
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ResumeArgs {
    /// 指定恢复的会话 id。
    pub session_id: Option<String>,
    /// `--last`：恢复当前工作区最近更新的一条会话。
    pub last: bool,
    /// 续跑的新提示词。
    pub prompt: Option<String>,
    /// 续跑附加图片。
    pub images: Vec<PathBuf>,
}

/// 无头 exec 轮次使用的冻结模型目标（不依赖 `crate::app`）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExecModelTarget {
    pub provider: String,
    pub model: String,
    pub variant: Option<String>,
    pub display_name: String,
    pub transport: String,
    pub base_url: String,
    pub api_key: String,
    pub context_limit: Option<u64>,
    pub output_limit: Option<u64>,
    pub variant_params: Option<serde_json::Value>,
    pub idle_timeout_ms: u64,
    pub supports_image_input: bool,
}

impl ExecModelTarget {
    /// 写入数据库/历史行所需的模型快照。
    pub fn snapshot(&self) -> crate::db::chat::ModelSnapshot {
        crate::db::chat::ModelSnapshot {
            provider: self.provider.clone(),
            model: self.model.clone(),
            variant: self.variant.clone(),
        }
    }
}

/// 解析命令行；`args` 的首项是二进制名（与 `std::env::args_os()` 一致）。
///
/// 只有 `argv[0]` 或完全没有参数时返回 [`CliAction::Tui`]；未知选项、
/// 缺少取值与互斥组合返回可直接展示给用户的英文错误。
pub fn parse_cli<I, T>(args: I) -> Result<CliAction, String>
where
    I: IntoIterator<Item = T>,
    T: Into<OsString>,
{
    let args: Vec<OsString> = args.into_iter().map(Into::into).collect();
    let Some(command) = args.get(1) else {
        return Ok(CliAction::Tui);
    };
    match utf8(command)? {
        "-h" | "--help" => Ok(CliAction::Help(HelpTopic::Root)),
        "-V" | "--version" | "version" => Ok(CliAction::Version),
        "help" => parse_help(&args[2..]),
        "exec" => parse_exec(&args[2..]),
        other => Err(format!(
            "unexpected argument '{other}'; run 'lx-tui --help' for usage"
        )),
    }
}

fn parse_help(topics: &[OsString]) -> Result<CliAction, String> {
    let topic = topics.first().map(utf8).transpose()?;
    let next = topics.get(1).map(utf8).transpose()?;
    if topics.len() > 2 {
        return Err("too many arguments for 'help'".to_string());
    }
    match (topic, next) {
        (None, _) => Ok(CliAction::Help(HelpTopic::Root)),
        (Some("exec"), None) => Ok(CliAction::Help(HelpTopic::Exec)),
        (Some("exec"), Some("resume")) => Ok(CliAction::Help(HelpTopic::ExecResume)),
        (Some("resume"), None) => Ok(CliAction::Help(HelpTopic::ExecResume)),
        (Some(other), _) => Err(format!("unknown help topic '{other}'")),
    }
}

fn parse_exec(args: &[OsString]) -> Result<CliAction, String> {
    let mut parser = ExecParser::default();
    let mut index = 0;
    while index < args.len() {
        let arg = utf8(&args[index])?;
        if arg == "--" {
            let rest = args[index + 1..]
                .iter()
                .map(|raw| utf8(raw).map(str::to_string))
                .collect::<Result<Vec<_>, _>>()?;
            if !rest.is_empty() {
                parser.push_literal_block(rest)?;
            }
            break;
        }
        match parser.handle_option(args, &mut index, arg)? {
            OptionOutcome::Help(topic) => return Ok(CliAction::Help(topic)),
            OptionOutcome::Handled => {
                index += 1;
                continue;
            }
            OptionOutcome::Unrecognized => {}
        }
        if arg == "resume" && parser.can_enter_resume() {
            parser.enter_resume()?;
        } else if arg == "--last" {
            parser.enable_last()?;
        } else if arg.starts_with('-') && arg != "-" {
            return Err(format!(
                "unknown option '{arg}'; run 'lx-tui exec --help' for usage"
            ));
        } else {
            parser.push_positional(arg.to_string())?;
        }
        index += 1;
    }
    parser.finish()
}

/// 单个参数的处理结果：已经消费、命中帮助或不属于选项。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum OptionOutcome {
    Handled,
    Help(HelpTopic),
    Unrecognized,
}

/// `exec` 参数累积器：统一维护全局选项与 `resume` 子命令状态。
#[derive(Debug, Default)]
struct ExecParser {
    exec: ExecCli,
    resume: Option<ResumeArgs>,
}

impl ExecParser {
    /// 处理带取值或无取值选项；`Help` 表示需要立即结束解析。
    fn handle_option(
        &mut self,
        args: &[OsString],
        index: &mut usize,
        arg: &str,
    ) -> Result<OptionOutcome, String> {
        if arg == "-h" || arg == "--help" {
            let topic = if self.resume.is_some() {
                HelpTopic::ExecResume
            } else {
                HelpTopic::Exec
            };
            return Ok(OptionOutcome::Help(topic));
        }
        if arg == "--json" {
            self.exec.json = true;
        } else if arg == "--ephemeral" {
            self.exec.ephemeral = true;
        } else if arg == "-C" || arg == "--cd" {
            let raw = take_value(args, index, arg)?;
            self.set_cwd(raw, arg)?;
        } else if arg == "-m" || arg == "--model" {
            let raw = take_value(args, index, arg)?;
            self.exec.model = Some(required_text(raw, arg)?);
        } else if arg == "-o" || arg == "--output-last-message" {
            let raw = take_value(args, index, arg)?;
            self.set_last_message_file(raw, arg)?;
        } else if arg == "-i" || arg == "--image" {
            let raw = take_value(args, index, arg)?;
            self.push_images(raw, arg)?;
        } else if arg == "--color" {
            let raw = take_value(args, index, arg)?;
            self.set_color(raw, arg)?;
        } else if let Some(value) = inline_value(arg, "--cd") {
            self.set_cwd(OsStr::new(value), "--cd")?;
        } else if let Some(value) = inline_value(arg, "--model") {
            self.exec.model = Some(required_text(OsStr::new(value), "--model")?);
        } else if let Some(value) = inline_value(arg, "--output-last-message") {
            self.set_last_message_file(OsStr::new(value), "--output-last-message")?;
        } else if let Some(value) = inline_value(arg, "--image") {
            self.push_images(OsStr::new(value), "--image")?;
        } else if let Some(value) = inline_value(arg, "--color") {
            self.set_color(OsStr::new(value), "--color")?;
        } else {
            return Ok(OptionOutcome::Unrecognized);
        }
        Ok(OptionOutcome::Handled)
    }

    fn can_enter_resume(&self) -> bool {
        self.resume.is_none() && self.exec.prompt.is_none()
    }

    /// 进入 `resume` 前把已收集的图片整体移交，避免两处各存一份。
    fn enter_resume(&mut self) -> Result<(), String> {
        if self.resume.is_some() {
            return Err("'resume' was specified more than once".to_string());
        }
        self.resume = Some(ResumeArgs {
            images: std::mem::take(&mut self.exec.images),
            ..ResumeArgs::default()
        });
        Ok(())
    }

    fn enable_last(&mut self) -> Result<(), String> {
        let Some(resume) = self.resume.as_mut() else {
            return Err("'--last' can only be used with 'resume'".to_string());
        };
        if resume.session_id.is_some() {
            return Err("cannot specify both '--last' and a session ID".to_string());
        }
        resume.last = true;
        Ok(())
    }

    fn set_cwd(&mut self, raw: &OsStr, flag: &str) -> Result<(), String> {
        self.exec.cwd = Some(required_path(raw, flag)?);
        Ok(())
    }

    fn set_last_message_file(&mut self, raw: &OsStr, flag: &str) -> Result<(), String> {
        self.exec.last_message_file = Some(required_path(raw, flag)?);
        Ok(())
    }

    fn set_color(&mut self, raw: &OsStr, flag: &str) -> Result<(), String> {
        let value = required_text(raw, flag)?;
        self.exec.color = ColorMode::parse(&value).ok_or_else(|| {
            format!("invalid value '{value}' for '{flag}': expected 'auto', 'always' or 'never'")
        })?;
        Ok(())
    }

    fn push_images(&mut self, raw: &OsStr, flag: &str) -> Result<(), String> {
        let paths = split_images(raw, flag)?;
        let images = match self.resume.as_mut() {
            Some(resume) => &mut resume.images,
            None => &mut self.exec.images,
        };
        images.extend(paths);
        Ok(())
    }

    fn push_positional(&mut self, value: String) -> Result<(), String> {
        if let Some(resume) = self.resume.as_mut() {
            if resume.last {
                // `resume --last` 之后的位置参数只可能是提示词；把 `sess-` 开头的
                // 会话 ID 当提示词是笔误，直接拒绝而不是发给模型。
                if resume.prompt.is_some() || value.starts_with("sess-") {
                    return Err("cannot specify both '--last' and a session ID".to_string());
                }
                resume.prompt = Some(value);
            } else if resume.session_id.is_none() {
                resume.session_id = Some(value);
            } else if resume.prompt.is_none() {
                resume.prompt = Some(value);
            } else {
                return Err(unexpected_argument(&value));
            }
        } else if self.exec.prompt.is_none() {
            self.exec.prompt = Some(value);
        } else {
            return Err(unexpected_argument(&value));
        }
        Ok(())
    }

    /// `--` 之后的参数一律按字面量处理；多段文本合并成一个提示词。
    fn push_literal_block(&mut self, mut tokens: Vec<String>) -> Result<(), String> {
        match self.resume.as_mut() {
            Some(resume) => {
                if resume.last {
                    if resume.prompt.is_some() {
                        return Err(unexpected_argument(&tokens[0]));
                    }
                    resume.prompt = Some(tokens.join(" "));
                } else if resume.session_id.is_none() {
                    resume.session_id = Some(tokens.remove(0));
                    if !tokens.is_empty() {
                        resume.prompt = Some(tokens.join(" "));
                    }
                } else if resume.prompt.is_none() {
                    resume.prompt = Some(tokens.join(" "));
                } else {
                    return Err(unexpected_argument(&tokens[0]));
                }
            }
            None => {
                if self.exec.prompt.is_some() {
                    return Err(unexpected_argument(&tokens[0]));
                }
                self.exec.prompt = Some(tokens.join(" "));
            }
        }
        Ok(())
    }

    fn finish(mut self) -> Result<CliAction, String> {
        if let Some(resume) = self.resume {
            if self.exec.ephemeral {
                return Err("--ephemeral cannot be combined with 'resume'".to_string());
            }
            if !resume.last && resume.session_id.is_none() {
                return Err(
                    "Specify a session ID or pass '--last' to resume the most recent session."
                        .to_string(),
                );
            }
            self.exec.command = Some(ExecCommand::Resume(resume));
        }
        Ok(CliAction::Exec(self.exec))
    }
}

fn take_value<'a>(
    args: &'a [OsString],
    index: &mut usize,
    flag: &str,
) -> Result<&'a OsString, String> {
    *index += 1;
    args.get(*index)
        .ok_or_else(|| format!("missing value for '{flag}'"))
}

fn inline_value<'a>(arg: &'a str, name: &str) -> Option<&'a str> {
    arg.strip_prefix(name)?.strip_prefix('=')
}

fn utf8(value: &OsString) -> Result<&str, String> {
    value
        .to_str()
        .ok_or_else(|| format!("argument is not valid UTF-8: {}", value.to_string_lossy()))
}

fn required_text(raw: &OsStr, flag: &str) -> Result<String, String> {
    let text = raw
        .to_str()
        .ok_or_else(|| format!("value for '{flag}' is not valid UTF-8"))?;
    if text.trim().is_empty() {
        return Err(format!("empty value for '{flag}'"));
    }
    Ok(text.to_string())
}

fn required_path(raw: &OsStr, flag: &str) -> Result<PathBuf, String> {
    if raw.is_empty() {
        return Err(format!("empty value for '{flag}'"));
    }
    Ok(PathBuf::from(raw))
}

/// `-i` 支持逗号分隔；含非 UTF-8 字节时整段作为单个路径。
fn split_images(raw: &OsStr, flag: &str) -> Result<Vec<PathBuf>, String> {
    let Some(text) = raw.to_str().filter(|text| text.contains(',')) else {
        return required_path(raw, flag).map(|path| vec![path]);
    };
    text.split(',')
        .map(|part| {
            let trimmed = part.trim();
            if trimmed.is_empty() {
                Err(format!("empty image path in '{text}'"))
            } else {
                Ok(PathBuf::from(trimmed))
            }
        })
        .collect()
}

fn unexpected_argument(value: &str) -> String {
    format!("unexpected argument '{value}'; quote the prompt or move it after '--'")
}

/// 英文帮助文本；`Root` / `Exec` / `ExecResume` 各自独立。
pub fn help_text(topic: HelpTopic) -> &'static str {
    match topic {
        HelpTopic::Root => ROOT_HELP,
        HelpTopic::Exec => EXEC_HELP,
        HelpTopic::ExecResume => EXEC_RESUME_HELP,
    }
}

const ROOT_HELP: &str = "\
lx-tui - terminal AI coding agent

Usage:
  lx-tui                                   Start the interactive TUI
  lx-tui exec [OPTIONS] [PROMPT]           Run one headless agent turn
  lx-tui exec resume [ARGS] [OPTIONS]      Continue a saved session
  lx-tui help [exec [resume]]              Print help for a command
  lx-tui --version                         Print the version

Prompt input:
  PROMPT may be passed as an argument, piped through stdin, or \"-\" to force
  reading stdin. When PROMPT is given and stdin is piped, stdin is appended
  as <stdin> context.

Run 'lx-tui exec --help' for the full list of exec options.
";

const EXEC_HELP: &str = "\
lx-tui exec - run one agent turn without the TUI

Usage:
  lx-tui exec [OPTIONS] [PROMPT]
  lx-tui exec resume [SESSION_ID | --last] [OPTIONS] [PROMPT]

Options:
  -C, --cd <DIR>                    Working directory (default: current directory)
  -m, --model <MODEL>               <provider>/<model>[#<variant>] or <model>[#<variant>]
  -i, --image <FILE>                Attach an image (repeatable, comma-separated allowed)
  -o, --output-last-message <FILE>  Write the final assistant message to FILE
      --json                        Emit one JSON event per line on stdout
      --ephemeral                   Keep the session in memory only (not persisted)
      --color <auto|always|never>   ANSI color for human-readable output (default: auto)
  -h, --help                        Print this help

Prompt input:
  Without PROMPT the prompt is read from stdin. A PROMPT of \"-\" also forces
  stdin. When PROMPT is given and stdin is piped, stdin is appended as
  <stdin> context.

Use '--' to pass a prompt that starts with '-'.
Exit codes: 0 success, 1 error, 130 interrupted.
";

const EXEC_RESUME_HELP: &str = "\
lx-tui exec resume - continue a previous session

Usage:
  lx-tui exec resume [SESSION_ID | --last] [OPTIONS] [PROMPT]

Arguments:
  SESSION_ID  Session to resume (the first positional argument)
  PROMPT      Prompt for the new turn; read from stdin when omitted

Options:
      --last                        Resume the most recently updated session in this workspace
  -C, --cd <DIR>                    Working directory (default: current directory)
  -m, --model <MODEL>               <provider>/<model>[#<variant>] or <model>[#<variant>]
  -i, --image <FILE>                Attach an image (repeatable, comma-separated allowed)
  -o, --output-last-message <FILE>  Write the final assistant message to FILE
      --json                        Emit one JSON event per line on stdout
      --color <auto|always|never>   ANSI color for human-readable output (default: auto)
  -h, --help                        Print this help

Either SESSION_ID or --last is required; --ephemeral is not supported here.
";

/// 把管道 stdin 作为 `<stdin>` 上下文追加到位置提示词末尾。
pub fn prompt_with_stdin_context(prompt: &str, stdin_text: &str) -> String {
    if stdin_text.trim().is_empty() {
        return prompt.to_string();
    }
    let mut combined = format!("{prompt}\n\n<stdin>\n{stdin_text}");
    if !stdin_text.ends_with('\n') {
        combined.push('\n');
    }
    combined.push_str("</stdin>");
    combined
}

/// 组合位置提示词与 stdin，得到本轮最终用户消息文本。
///
/// 错误条件：空提示词、TTY 下缺少提示词、强制读取时 stdin 为空或读取失败。
pub fn resolve_prompt_input(
    prompt_arg: Option<&str>,
    stdin_is_terminal: bool,
    read_stdin: impl FnOnce() -> std::io::Result<String>,
) -> Result<String, String> {
    let read = || read_stdin().map_err(|error| format!("failed to read stdin: {error}"));
    match prompt_arg {
        Some("-") => {
            let stdin_text = read()?;
            if stdin_text.trim().is_empty() {
                return Err("No prompt provided on stdin.".to_string());
            }
            Ok(stdin_text)
        }
        Some(prompt) => {
            if prompt.trim().is_empty() {
                return Err("Prompt must not be empty.".to_string());
            }
            if stdin_is_terminal {
                Ok(prompt.to_string())
            } else {
                Ok(prompt_with_stdin_context(prompt, &read()?))
            }
        }
        None => {
            if stdin_is_terminal {
                return Err(
                    "No prompt provided. Either specify one as an argument or pipe the prompt into stdin."
                        .to_string(),
                );
            }
            let stdin_text = read()?;
            if stdin_text.trim().is_empty() {
                return Err("No prompt provided on stdin.".to_string());
            }
            Ok(stdin_text)
        }
    }
}

const NO_MODEL_AVAILABLE: &str =
    "No configured AI model available. Configure a provider and API key in settings or ai.json.";

/// 解析本轮使用的模型：`-m` > 恢复会话的模型 > 默认模型 > 首个可用模型。
///
/// `#variant` 显式指定时必须存在于模型 variants；未指定时回落到模型默认
/// variant（失效则忽略）。所有错误均为可直接展示的英文文案。
pub fn resolve_model_target(
    settings: &AiSettings,
    raw_model: Option<&str>,
    session_model: Option<(&str, &str, Option<&str>)>,
) -> Result<ExecModelTarget, String> {
    if let Some(raw) = raw_model {
        let (reference, explicit_variant) = split_model_reference(raw)?;
        let (provider, model) = resolve_raw_selection(settings, reference)?;
        return build_target(settings, &provider, &model, explicit_variant, true);
    }
    if let Some((provider, model, variant)) = session_model
        && usable_selection(settings, provider, model)
    {
        return build_target(settings, provider, model, variant, false);
    }
    if let Some(selection) = settings.default_model.as_ref()
        && usable_selection(settings, &selection.provider, &selection.model)
    {
        return build_target(
            settings,
            &selection.provider,
            &selection.model,
            selection.variant.as_deref(),
            false,
        );
    }
    if let Some((provider, model)) = first_available_selection(settings) {
        return build_target(settings, &provider, &model, None, false);
    }
    Err(NO_MODEL_AVAILABLE.to_string())
}

/// 拆分 `<model>[#<variant>]`；空模型或空 variant 直接报错。
fn split_model_reference(raw: &str) -> Result<(&str, Option<&str>), String> {
    let raw = raw.trim();
    if raw.is_empty() {
        return Err("model must not be empty".to_string());
    }
    match raw.rsplit_once('#') {
        Some((model, variant)) => {
            if model.is_empty() {
                return Err(format!(
                    "invalid model '{raw}': missing model id before '#'"
                ));
            }
            if variant.is_empty() {
                return Err(format!("invalid model '{raw}': missing variant after '#'"));
            }
            Ok((model, Some(variant)))
        }
        None => Ok((raw, None)),
    }
}

/// 解析 `provider/model` 或短 `model` 到确定的 provider 与 model id。
fn resolve_raw_selection(
    settings: &AiSettings,
    reference: &str,
) -> Result<(String, String), String> {
    if let Some((provider_id, model_id)) = reference.split_once('/') {
        if provider_id.is_empty() || model_id.is_empty() {
            return Err(format!(
                "invalid model '{reference}': expected <provider>/<model>"
            ));
        }
        let provider = settings
            .providers
            .get(provider_id)
            .ok_or_else(|| format!("provider '{provider_id}' is not defined"))?;
        if !is_enabled(settings, provider_id) {
            return Err(format!("provider '{provider_id}' is not enabled"));
        }
        if api_key_of(provider).is_none() {
            return Err(format!(
                "provider '{provider_id}' has no API key configured"
            ));
        }
        if !provider.models.contains_key(model_id) {
            return Err(format!(
                "model '{model_id}' is not defined on provider '{provider_id}'"
            ));
        }
        return Ok((provider_id.to_string(), model_id.to_string()));
    }
    if let Some(default) = settings.default_model.as_ref()
        && default.model == reference
        && usable_selection(settings, &default.provider, reference)
    {
        return Ok((default.provider.clone(), reference.to_string()));
    }
    for provider_id in &settings.enabled_providers {
        if let Some(provider) = usable_provider(settings, provider_id)
            && provider.models.contains_key(reference)
        {
            return Ok((provider_id.clone(), reference.to_string()));
        }
    }
    Err(format!(
        "model '{reference}' was not found on any enabled provider"
    ))
}

fn is_enabled(settings: &AiSettings, provider_id: &str) -> bool {
    settings
        .enabled_providers
        .iter()
        .any(|id| id == provider_id)
}

/// 已启用且配置了非空 API Key 的供应商。
fn usable_provider<'a>(settings: &'a AiSettings, provider_id: &str) -> Option<&'a ProviderConfig> {
    if !is_enabled(settings, provider_id) {
        return None;
    }
    let provider = settings.providers.get(provider_id)?;
    api_key_of(provider)?;
    Some(provider)
}

fn usable_selection(settings: &AiSettings, provider_id: &str, model_id: &str) -> bool {
    usable_provider(settings, provider_id).is_some_and(|p| p.models.contains_key(model_id))
}

/// 首个可用供应商的首个模型（BTreeMap 顺序，稳定可复现）。
fn first_available_selection(settings: &AiSettings) -> Option<(String, String)> {
    settings
        .enabled_providers
        .iter()
        .filter_map(|id| usable_provider(settings, id))
        .find_map(|provider| {
            provider
                .models
                .keys()
                .next()
                .map(|model| (provider.id.clone(), model.clone()))
        })
}

fn api_key_of(provider: &ProviderConfig) -> Option<String> {
    provider
        .options
        .api_key
        .as_deref()
        .map(str::trim)
        .filter(|key| !key.is_empty())
        .map(str::to_string)
}

/// 冻结目标；`strict_variant` 为真时未知显式 variant 直接报错。
fn build_target(
    settings: &AiSettings,
    provider_id: &str,
    model_id: &str,
    requested_variant: Option<&str>,
    strict_variant: bool,
) -> Result<ExecModelTarget, String> {
    let provider = usable_provider(settings, provider_id).ok_or_else(|| {
        format!("provider '{provider_id}' is not enabled or has no API key configured")
    })?;
    let model = provider
        .models
        .get(model_id)
        .ok_or_else(|| format!("model '{model_id}' is not defined on provider '{provider_id}'"))?;
    let variant = match requested_variant {
        Some(variant) if model.variants.contains_key(variant) => Some(variant.to_string()),
        Some(variant) if strict_variant => {
            return Err(format!(
                "unknown variant '{variant}' for model '{model_id}'"
            ));
        }
        _ => model
            .variant
            .clone()
            .filter(|variant| model.variants.contains_key(variant)),
    };
    let api_key = api_key_of(provider)
        .ok_or_else(|| format!("provider '{provider_id}' has no API key configured"))?;
    Ok(ExecModelTarget {
        provider: provider.id.clone(),
        model: model.id.clone(),
        variant: variant.clone(),
        display_name: if model.name.trim().is_empty() {
            model.id.clone()
        } else {
            model.name.clone()
        },
        transport: model
            .transport
            .map(|transport| transport.as_str())
            .unwrap_or_else(|| provider.kind.as_str())
            .to_string(),
        base_url: provider.options.base_url.clone(),
        api_key,
        context_limit: model.limit.map(|limit| limit.context),
        output_limit: model.limit.map(|limit| limit.output),
        variant_params: variant
            .as_ref()
            .and_then(|variant| model.variants.get(variant).cloned()),
        idle_timeout_ms: settings.idle_timeout_ms(),
        supports_image_input: model.supports_image_input(),
    })
}
