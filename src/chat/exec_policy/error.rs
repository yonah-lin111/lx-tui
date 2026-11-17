//! execpolicy 错误类型（去掉 Codex 的 span/CLI 相关部分）。

use std::fmt;

/// 解析/校验结果。
pub type Result<T> = std::result::Result<T, Error>;

/// execpolicy 错误。
#[derive(Debug)]
pub enum Error {
    /// `decision` 不是 allow / prompt / prompt-user / forbidden。
    InvalidDecision(String),
    /// `pattern` 结构非法。
    InvalidPattern(String),
    /// `match` / `not_match` 示例非法。
    InvalidExample(String),
    /// 规则本身非法。
    InvalidRule(String),
    /// `match` 示例没有命中任何规则。
    ExampleDidNotMatch {
        /// 本组规则。
        rules: Vec<String>,
        /// 未命中的示例。
        examples: Vec<String>,
    },
    /// `not_match` 示例命中了规则。
    ExampleDidMatch {
        /// 命中的规则。
        rule: String,
        /// 反例。
        example: String,
    },
    /// Starlark 解析/求值错误（已格式化为文本）。
    Starlark(String),
}

impl fmt::Display for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidDecision(value) => write!(formatter, "invalid decision: {value}"),
            Self::InvalidPattern(message) => write!(formatter, "invalid pattern: {message}"),
            Self::InvalidExample(message) => write!(formatter, "invalid example: {message}"),
            Self::InvalidRule(message) => write!(formatter, "invalid rule: {message}"),
            Self::ExampleDidNotMatch { rules, examples } => write!(
                formatter,
                "expected every example to match at least one rule; rules: {rules:?}; unmatched \
                 examples: {examples:?}"
            ),
            Self::ExampleDidMatch { rule, example } => {
                write!(
                    formatter,
                    "expected example not to match rule `{rule}`: {example}"
                )
            }
            Self::Starlark(message) => write!(formatter, "starlark error: {message}"),
        }
    }
}

impl std::error::Error for Error {}

impl From<starlark::Error> for Error {
    fn from(error: starlark::Error) -> Self {
        Self::Starlark(error.to_string())
    }
}

impl From<Error> for starlark::Error {
    fn from(error: Error) -> Self {
        starlark::Error::new_other(error)
    }
}
