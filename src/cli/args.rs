//! 通用零依赖 CLI 参数扫描器、全局选项剥离与多行长文本输入解析。

use std::ffi::{OsStr, OsString};
use std::io::{IsTerminal, Read};
use std::path::PathBuf;

use crate::chat::agent::exec::ColorMode;

/// 所有新增子命令共用的全局选项。
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct GlobalOpts {
    /// `-C, --cd <DIR>`：目标工作目录。
    pub cwd: Option<PathBuf>,
    /// `--json`：输出结构化 JSON。
    pub json: bool,
    /// `--color <auto|always|never>`：控制人类可读输出颜色。
    pub color: ColorMode,
}

/// 写操作或单作用域查询的 `--scope <user|project>`。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ScopeArg {
    #[default]
    User,
    Project,
}

impl ScopeArg {
    pub fn parse(raw: &str) -> Result<Self, String> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "user" => Ok(Self::User),
            "project" => Ok(Self::Project),
            other => Err(format!(
                "invalid '--scope' value '{other}'; expected 'user' or 'project'"
            )),
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::User => "user",
            Self::Project => "project",
        }
    }
}

/// 列表/展示命令的 `--scope <all|user|project>`。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ListScopeArg {
    #[default]
    All,
    User,
    Project,
}

impl ListScopeArg {
    pub fn parse(raw: &str) -> Result<Self, String> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "all" => Ok(Self::All),
            "user" => Ok(Self::User),
            "project" => Ok(Self::Project),
            other => Err(format!(
                "invalid '--scope' value '{other}'; expected 'all', 'user', or 'project'"
            )),
        }
    }

    pub fn includes_user(self) -> bool {
        matches!(self, Self::All | Self::User)
    }

    pub fn includes_project(self) -> bool {
        matches!(self, Self::All | Self::Project)
    }
}

/// 多行长文本输入来源（用于 prompt / skill / command / subagent prompt）。
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ContentInput {
    /// `--content <TEXT>`（或 `"-" ` 表示从 stdin 读取）。
    pub inline: Option<String>,
    /// `-f, --file <PATH>`（或 `"-"` 表示从 stdin 读取）。
    pub file: Option<PathBuf>,
}

impl ContentInput {
    /// 尝试消费 `--content` 或 `-f/--file` 参数；命中返回 `Ok(true)`。
    pub fn try_take(&mut self, args: &[String], index: &mut usize) -> Result<bool, String> {
        let current = &args[*index];
        if let Some(val) = take_opt(args, index, "", "--content")? {
            if self.inline.is_some() {
                return Err("'--content' cannot be specified more than once".to_string());
            }
            self.inline = Some(val);
            return Ok(true);
        }
        if let Some(val) = take_opt(args, index, "-f", "--file")? {
            if self.file.is_some() {
                return Err("'--file' cannot be specified more than once".to_string());
            }
            let path = required_non_empty(&val, "--file")?;
            self.file = Some(PathBuf::from(path));
            return Ok(true);
        }
        let _ = current;
        Ok(false)
    }

    /// 使用进程真实 `stdin` 解析内容。
    pub fn resolve(&self, required: bool) -> Result<Option<String>, String> {
        self.resolve_with(required, std::io::stdin().is_terminal(), read_stdin_to_end)
    }

    /// 可注入终端状态与读取函数的解析器（便于单元测试）。
    pub fn resolve_with(
        &self,
        required: bool,
        stdin_is_terminal: bool,
        read_stdin: impl FnOnce() -> std::io::Result<String>,
    ) -> Result<Option<String>, String> {
        if self.inline.is_some() && self.file.is_some() {
            return Err("cannot specify both '--content' and '--file'".to_string());
        }
        if let Some(raw) = &self.inline {
            if raw == "-" {
                let text = read_stdin().map_err(|e| format!("failed to read stdin: {e}"))?;
                return Ok(Some(text));
            }
            return Ok(Some(raw.clone()));
        }
        if let Some(path) = &self.file {
            if path.as_os_str() == "-" {
                let text = read_stdin().map_err(|e| format!("failed to read stdin: {e}"))?;
                return Ok(Some(text));
            }
            let text = std::fs::read_to_string(path)
                .map_err(|e| format!("failed to read '{}': {e}", path.display()))?;
            return Ok(Some(text));
        }
        if !stdin_is_terminal {
            let text = read_stdin().map_err(|e| format!("failed to read stdin: {e}"))?;
            // 可选正文的隐式 stdin（脚本/CI 的空管道）不得覆盖已有正文。
            if !required && text.is_empty() {
                return Ok(None);
            }
            return Ok(Some(text));
        }
        if required {
            Err(
                "missing content; pass '--content <TEXT>', '-f <FILE>', or pipe via stdin"
                    .to_string(),
            )
        } else {
            Ok(None)
        }
    }
}

fn read_stdin_to_end() -> std::io::Result<String> {
    let mut buf = String::new();
    std::io::stdin().read_to_string(&mut buf)?;
    Ok(buf)
}

/// 将 `OsStr` 转为 UTF-8 `&str`。
pub fn utf8(raw: &OsStr) -> Result<&str, String> {
    raw.to_str()
        .ok_or_else(|| format!("argument '{}' is not valid UTF-8", raw.to_string_lossy()))
}

/// 已知需要取值的子命令选项：全局扫描必须连同紧随的值原样保留，
/// 否则像 `--content --json` 这样的值会被误当成全局标志吞掉。
const VALUE_OPTIONS: &[&str] = &[
    "--content",
    "-f",
    "--file",
    "--description",
    "--prompt",
    "--title",
    "--name",
    "--text",
    "--note",
    "--command",
    "--arg",
    "--env",
    "--env-var",
    "--header",
    "--env-http-header",
    "--url",
    "--base-url",
    "--api-key",
    "--bearer-token",
    "--bearer-token-env",
    "--bearer-env",
    "--client-id",
    "--client-secret",
    "--scope-item",
    "--cwd",
    "--folder",
    "--parent",
    "--page",
    "--page-name",
    "--date",
    "-d",
    "-p",
    "--priority",
    "--status",
    "-m",
    "--model",
    "--mode",
    "--before",
    "--limit",
    "--offset",
    "--search",
    "-s",
    "--scope",
    "--kind",
    "--type",
    "--md-scope",
    "--block",
    "--block-type",
    "--arg-hint",
    "--argument-hint",
    "--when-to-use",
    "--base",
    "--delegation-mode",
    "--auth",
    "--transport",
    "--variant",
    "--context-limit",
    "--output-limit",
    "--image-input",
    "--filter",
    "--to",
    "--id",
    "--days",
    "--from",
    "--exa-key",
    "--tavily-key",
    "--title-language",
    "--timeout-ms",
    "--max-concurrent",
    "--max-depth",
    "--default-max-tokens",
    "--startup-timeout",
    "--tool-timeout",
    "--sidebar-width",
    "--sidebar-min-width",
    "--sidebar-max-width",
    "--narrow-width",
    "--min-width",
    "--min-height",
    "--min-pane-width",
    "--min-pane-height",
    "--lock-model",
    "--allow-web-search",
    "--allow-mcp",
    "--allow-skills",
    "--nicknames",
    "--disable-mcp",
    "--site",
    "--session",
    "-t",
    "--edit-index",
    "--keep-file",
];

/// 扫描并剥离全局选项（`-C/--cd`、`--json`、`--color`、`-h/--help`），
/// 保留其余参数按原顺序交由子命令解析。
pub fn extract_global_opts(
    raw_args: &[OsString],
) -> Result<(GlobalOpts, Vec<String>, bool), String> {
    let mut global = GlobalOpts::default();
    let mut rest = Vec::with_capacity(raw_args.len());
    let mut help_requested = false;
    let mut i = 0;
    while i < raw_args.len() {
        let arg = utf8(&raw_args[i])?;
        if arg == "--" {
            rest.push("--".to_string());
            for item in &raw_args[i + 1..] {
                rest.push(utf8(item)?.to_string());
            }
            break;
        }
        if matches!(arg, "-h" | "--help") {
            help_requested = true;
            i += 1;
            continue;
        }
        if arg == "--json" {
            global.json = true;
            i += 1;
            continue;
        }
        if let Some(val) = take_os_opt_value(raw_args, &mut i, arg, "-C", "--cd")? {
            global.cwd = Some(PathBuf::from(required_non_empty(&val, "--cd")?));
            i += 1;
            continue;
        }
        if let Some(val) = take_os_opt_value(raw_args, &mut i, arg, "", "--color")? {
            global.color = ColorMode::parse(&val).ok_or_else(|| {
                format!("invalid '--color' value '{val}'; expected 'auto', 'always', or 'never'")
            })?;
            i += 1;
            continue;
        }
        let value_flag = arg.split_once('=').map_or(arg, |(name, _)| name);
        if VALUE_OPTIONS.contains(&value_flag) {
            rest.push(arg.to_string());
            if arg.contains('=') || i + 1 >= raw_args.len() {
                i += 1;
            } else {
                rest.push(utf8(&raw_args[i + 1])?.to_string());
                i += 2;
            }
            continue;
        }
        rest.push(arg.to_string());
        i += 1;
    }
    Ok((global, rest, help_requested))
}

fn take_os_opt_value(
    args: &[OsString],
    index: &mut usize,
    current: &str,
    short: &str,
    long: &str,
) -> Result<Option<String>, String> {
    if (!short.is_empty() && current == short) || (!long.is_empty() && current == long) {
        let label = if !long.is_empty() { long } else { short };
        let next = args
            .get(*index + 1)
            .ok_or_else(|| format!("option '{label}' requires a value"))?;
        *index += 1;
        return Ok(Some(utf8(next)?.to_string()));
    }
    if !long.is_empty()
        && let Some(rest) = current.strip_prefix(long)
        && let Some(val) = rest.strip_prefix('=')
    {
        return Ok(Some(val.to_string()));
    }
    if !short.is_empty()
        && let Some(rest) = current.strip_prefix(short)
        && let Some(val) = rest.strip_prefix('=')
    {
        return Ok(Some(val.to_string()));
    }
    Ok(None)
}

/// 在 `Vec<String>` 参数切片上提取带值选项（支持 `-k val`、`--key val`、`--key=val`）。
pub fn take_opt(
    args: &[String],
    index: &mut usize,
    short: &str,
    long: &str,
) -> Result<Option<String>, String> {
    let current = args[*index].as_str();
    if (!short.is_empty() && current == short) || (!long.is_empty() && current == long) {
        let label = if !long.is_empty() { long } else { short };
        let next = args
            .get(*index + 1)
            .ok_or_else(|| format!("option '{label}' requires a value"))?;
        *index += 1;
        return Ok(Some(next.clone()));
    }
    if !long.is_empty()
        && let Some(rest) = current.strip_prefix(long)
        && let Some(val) = rest.strip_prefix('=')
    {
        return Ok(Some(val.to_string()));
    }
    if !short.is_empty()
        && let Some(rest) = current.strip_prefix(short)
        && let Some(val) = rest.strip_prefix('=')
    {
        return Ok(Some(val.to_string()));
    }
    Ok(None)
}

/// 校验选项值非空（去首尾空白后不为空）。
pub fn required_non_empty(value: &str, flag: &str) -> Result<String, String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err(format!("option '{flag}' requires a non-empty value"));
    }
    Ok(trimmed.to_string())
}

pub fn parse_usize_flag(value: &str, flag: &str) -> Result<usize, String> {
    value
        .trim()
        .parse::<usize>()
        .map_err(|_| format!("invalid integer '{value}' for '{flag}'"))
}

pub fn parse_u64_flag(value: &str, flag: &str) -> Result<u64, String> {
    value
        .trim()
        .parse::<u64>()
        .map_err(|_| format!("invalid integer '{value}' for '{flag}'"))
}

pub fn parse_i64_flag(value: &str, flag: &str) -> Result<i64, String> {
    value
        .trim()
        .parse::<i64>()
        .map_err(|_| format!("invalid integer '{value}' for '{flag}'"))
}

pub fn parse_bool_flag(value: &str, flag: &str) -> Result<bool, String> {
    match value.trim().to_ascii_lowercase().as_str() {
        "true" | "1" | "yes" | "on" => Ok(true),
        "false" | "0" | "no" | "off" => Ok(false),
        _ => Err(format!(
            "invalid boolean '{value}' for '{flag}'; expected 'true' or 'false'"
        )),
    }
}

/// 若参数以 `-` 开头（且不是单独的 `"-"` 或 `--` 终止符），拒绝为未知选项。
pub fn reject_unknown_flag(arg: &str, help_cmd: &str) -> Result<(), String> {
    if arg.starts_with('-') && arg != "-" && arg != "--" {
        return Err(format!(
            "unknown option '{arg}'; run '{help_cmd} --help' for usage"
        ));
    }
    Ok(())
}
