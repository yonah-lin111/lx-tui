//! `rtk` 集成：优先调用用户安装的 rtk，缺失时使用内置输出过滤器。
//!
//! rtk 是命令代理：`rtk cargo test` / `rtk git status` / `rtk rg …` 在保留命令语义的
//! 前提下压缩输出，降低进入模型上下文的 token。本模块只做两件事：
//!
//! 1. [`Rtk::prepare`]：判断命令首段是否属于 rtk 支持的安全子命令，能包装就包装；
//! 2. [`Rtk::filter_output`]：没有外部 rtk（或命令无法包装）时，按命令类别执行内置
//!    过滤，并把省略量显式写进结果，绝不静默丢数据。
//!
//! 环境变量：
//! - `LX_RTK_MODE`：`auto`（默认，优先外部）、`builtin`（强制内置）、`off`（关闭）；
//! - `LX_RTK_BIN`：显式指定 rtk 可执行文件（优先于 PATH）。
//!
//! 测试构建默认 `off`：单元/集成测试不得因开发机是否安装 rtk 而改变命令输出。

mod builtin;

#[cfg(test)]
mod tests;

use std::path::{Path, PathBuf};

pub use builtin::{MAX_GENERIC_CHARS, filter_output};

/// 可执行文件名（Windows 带 `.exe`）。
const BINARY_NAME: &str = "rtk";

/// 运行模式。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// 不包装、不过滤（测试默认）。
    Off,
    /// 忽略外部 rtk，只用内置过滤。
    Builtin,
    /// 有外部 rtk 就包装，否则内置过滤。
    Auto,
}

impl Mode {
    /// 从 `LX_RTK_MODE` 解析；未设置时测试构建为 Off，生产为 Auto。
    fn from_env() -> Self {
        if let Some(raw) = std::env::var_os("LX_RTK_MODE") {
            return match raw.to_string_lossy().trim().to_ascii_lowercase().as_str() {
                "off" | "none" | "0" | "false" => Self::Off,
                "builtin" | "internal" => Self::Builtin,
                _ => Self::Auto,
            };
        }
        if cfg!(test) { Self::Off } else { Self::Auto }
    }
}

/// rtk 后端配置（检测一次，随执行器/事件层复用）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rtk {
    mode: Mode,
    bin: Option<PathBuf>,
}

/// 一次命令的准备结果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreparedCommand {
    /// 实际交给 shell 的脚本（外部 rtk 可用时是 `rtk <原命令>`）。
    pub command: String,
    /// 原命令（卡片展示、上下文事实与警告判定都用它）。
    pub original: String,
    /// 已由外部 rtk 过滤，内置过滤不得重复执行。
    pub external: bool,
}

impl Default for Rtk {
    fn default() -> Self {
        Self::detect()
    }
}

impl Rtk {
    /// 按环境检测。`auto` 下查找 `LX_RTK_BIN` 或 PATH 中的 rtk。
    pub fn detect() -> Self {
        let mode = Mode::from_env();
        let bin = match mode {
            Mode::Auto => resolve_bin(),
            Mode::Off | Mode::Builtin => None,
        };
        Self { mode, bin }
    }

    /// 显式构造（测试与配置注入）。
    pub fn with(mode: Mode, bin: Option<PathBuf>) -> Self {
        Self { mode, bin }
    }

    /// 当前是否启用过滤（外部或内置）。
    pub fn is_enabled(&self) -> bool {
        self.mode != Mode::Off
    }

    /// 外部 rtk 路径（仅 `auto` 且找到时）。
    pub fn external_bin(&self) -> Option<&Path> {
        self.bin.as_deref()
    }

    /// 准备命令：能安全包装就包装，否则保留原命令交给内置过滤。
    pub fn prepare(&self, command: &str) -> PreparedCommand {
        let original = command.to_string();
        if self.mode == Mode::Off {
            return PreparedCommand {
                command: original.clone(),
                original,
                external: false,
            };
        }
        if self.mode == Mode::Auto
            && let Some(bin) = self.bin.as_deref()
            && wrap_supported(command)
        {
            return PreparedCommand {
                command: format!("{} {command}", shell_quote(bin)),
                original,
                external: true,
            };
        }
        PreparedCommand {
            command: original.clone(),
            original,
            external: false,
        }
    }

    /// 对**未被外部 rtk 包装**的输出执行内置过滤；`off` 时原样返回。
    pub fn filter_output(&self, command: &str, output: &str) -> String {
        if self.mode == Mode::Off {
            return output.to_string();
        }
        builtin::filter_output(command, output)
    }

    /// 根据准备结果决定是否需要内置过滤。
    pub fn should_filter_builtin(&self, prepared: &PreparedCommand) -> bool {
        self.mode != Mode::Off && !prepared.external
    }
}

/// 从 `LX_RTK_BIN` 或 PATH 解析 rtk 可执行文件。
fn resolve_bin() -> Option<PathBuf> {
    if let Some(path) = std::env::var_os("LX_RTK_BIN").filter(|value| !value.is_empty()) {
        return Some(PathBuf::from(path));
    }
    let path = std::env::var_os("PATH")?;
    let file = format!("{BINARY_NAME}{}", std::env::consts::EXE_SUFFIX);
    std::env::split_paths(&path)
        .map(|dir| dir.join(&file))
        .find(|candidate| candidate.is_file())
}

/// 命令是否可以安全地前缀 `rtk`。
///
/// 只包装**首段是受支持工具**的简单命令：带前导赋值、`cd &&`、换行、已带 rtk 或
/// 含管道/重定向的脚本一律不包装。`rtk` 只压缩 stdout，若用户把 stdout 写进文件或
/// 交给下游（`curl … > resp.json`、`git diff | patch`），压缩结果会替换真实数据。
fn wrap_supported(command: &str) -> bool {
    let trimmed = command.trim_start();
    if trimmed.is_empty() || trimmed.contains('\n') || has_shell_operators(trimmed) {
        return false;
    }
    let mut words = trimmed.split_whitespace();
    let Some(first) = words.next() else {
        return false;
    };
    // 前导 shell 语法 / 变量赋值 / 已包装：都不动。
    if first.contains([';', '|', '&', '<', '>', '(', ')', '`', '$'])
        || is_assignment(first)
        || first == "rtk"
    {
        return false;
    }
    let second = words.next();
    match first {
        "cargo" => matches!(
            second,
            Some("build" | "test" | "clippy" | "check" | "install" | "nextest")
        ),
        "git" => matches!(
            second,
            Some(
                "diff"
                    | "log"
                    | "status"
                    | "show"
                    | "add"
                    | "commit"
                    | "push"
                    | "pull"
                    | "branch"
                    | "fetch"
                    | "stash"
                    | "worktree"
            )
        ),
        // 只列**短时、非交互**工具；长驻/交互命令（dev、serve、logs -f 等）不包装，
        // 避免把流式输出改成结束才返回。
        // JS 包管理器只包装测试/已知工具：`npm run dev`、任意 `npx` 都可能长驻。
        "npm" | "pnpm" => matches!(second, Some("test")),
        "npx" => matches!(second, Some("tsc" | "eslint" | "prisma")),
        "ls" | "tree" | "grep" | "rg" | "find" | "wc" | "diff" | "gh" | "glab" | "aws" | "psql"
        | "dotnet" | "jest" | "vitest" | "prisma" | "tsc" | "prettier" | "playwright" | "curl"
        | "wget" => true,
        _ => false,
    }
}

/// 命令是否包含未加引号的管道/重定向（引号内的字面字符不算）。
fn has_shell_operators(command: &str) -> bool {
    let mut quote: Option<char> = None;
    let mut escaped = false;
    for ch in command.chars() {
        if escaped {
            escaped = false;
            continue;
        }
        match quote {
            Some(current) => {
                if ch == '\\' && current == '"' {
                    escaped = true;
                } else if ch == current {
                    quote = None;
                }
            }
            None => match ch {
                '\'' | '"' => quote = Some(ch),
                '\\' => escaped = true,
                '|' | '<' | '>' => return true,
                _ => {}
            },
        }
    }
    false
}

/// `FOO=bar` 形式的前导赋值。
fn is_assignment(word: &str) -> bool {
    let Some((name, _)) = word.split_once('=') else {
        return false;
    };
    !name.is_empty()
        && name
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == '_')
        && !name.starts_with(|ch: char| ch.is_ascii_digit())
}

/// shell 路径转义（POSIX 单引号 / Windows 双引号），差异收在 platform 层。
fn shell_quote(path: &Path) -> String {
    crate::platform::shell_quote_path(path)
}
