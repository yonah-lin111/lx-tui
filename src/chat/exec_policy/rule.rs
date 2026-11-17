//! `prefix_rule` 规则模型与加载期示例自检。
//!
//! 移植自 OpenAI Codex `execpolicy/src/rule.rs`（Apache-2.0），去掉 canonical
//! matching、network rule 与 host executable。

use std::sync::Arc;

use super::decision::Decision;
use super::error::Error;
use super::error::Result;

/// 单个命令 token 的模式：固定字符串或若干备选。
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PatternToken {
    /// 固定 token。
    Single(String),
    /// 任一备选命中即可。
    Alts(Vec<String>),
}

impl PatternToken {
    /// 全部允许的取值。
    pub fn alternatives(&self) -> &[String] {
        match self {
            Self::Single(value) => std::slice::from_ref(value),
            Self::Alts(values) => values,
        }
    }

    fn render(&self) -> String {
        match self {
            Self::Single(value) => value.clone(),
            Self::Alts(values) => format!("[{}]", values.join("|")),
        }
    }
}

/// 以首个 token 建索引的前缀模式。
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PrefixPattern {
    /// 首 token（固定）。
    pub first: Arc<str>,
    /// 其余 token。
    pub rest: Arc<[PatternToken]>,
}

impl PrefixPattern {
    /// 命令是否命中该前缀；命中返回匹配到的前缀 token。
    pub fn matches_prefix(&self, command: &[String]) -> Option<Vec<String>> {
        let length = self.rest.len() + 1;
        if command.len() < length || command[0] != self.first.as_ref() {
            return None;
        }
        for (pattern, token) in self.rest.iter().zip(&command[1..length]) {
            if !pattern
                .alternatives()
                .iter()
                .any(|alternative| alternative == token)
            {
                return None;
            }
        }
        Some(command[..length].to_vec())
    }

    /// 用于落盘/展示的完整模式 token。
    pub fn render(&self) -> Vec<String> {
        std::iter::once(self.first.to_string())
            .chain(self.rest.iter().map(PatternToken::render))
            .collect()
    }
}

/// 一条前缀规则。
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PrefixRule {
    /// 匹配模式。
    pub pattern: PrefixPattern,
    /// 裁决。
    pub decision: Decision,
    /// 用户可读理由。
    pub justification: Option<String>,
}

impl PrefixRule {
    /// 规则索引键。
    pub fn program(&self) -> &str {
        self.pattern.first.as_ref()
    }

    /// 命令命中时返回匹配详情。
    pub fn matches(&self, command: &[String]) -> Option<RuleMatch> {
        self.pattern
            .matches_prefix(command)
            .map(|matched_prefix| RuleMatch {
                matched_prefix,
                decision: self.decision,
                justification: self.justification.clone(),
            })
    }
}

/// 一次命中的结果。
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RuleMatch {
    /// 实际匹配的前缀。
    pub matched_prefix: Vec<String>,
    /// 裁决。
    pub decision: Decision,
    /// 理由。
    pub justification: Option<String>,
}

/// 校验 `match` 示例至少命中一条规则。
pub fn validate_match_examples(rules: &[Arc<PrefixRule>], examples: &[Vec<String>]) -> Result<()> {
    let mut unmatched = Vec::new();
    for example in examples {
        if rules.iter().any(|rule| rule.matches(example).is_some()) {
            continue;
        }
        unmatched.push(render_example(example));
    }
    if unmatched.is_empty() {
        Ok(())
    } else {
        Err(Error::ExampleDidNotMatch {
            rules: rules.iter().map(|rule| format!("{rule:?}")).collect(),
            examples: unmatched,
        })
    }
}

/// 校验 `not_match` 示例不命中任何规则。
pub fn validate_not_match_examples(
    rules: &[Arc<PrefixRule>],
    examples: &[Vec<String>],
) -> Result<()> {
    for example in examples {
        if let Some(rule) = rules.iter().find(|rule| rule.matches(example).is_some()) {
            return Err(Error::ExampleDidMatch {
                rule: format!("{rule:?}"),
                example: render_example(example),
            });
        }
    }
    Ok(())
}

fn render_example(example: &[String]) -> String {
    shlex::try_join(example.iter().map(String::as_str)).unwrap_or_else(|_| example.join(" "))
}
