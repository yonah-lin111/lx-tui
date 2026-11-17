//! 规则集合与最严格决策匹配。
//!
//! 移植自 OpenAI Codex `execpolicy/src/policy.rs`（Apache-2.0），只保留前缀规则。

use std::collections::HashMap;
use std::sync::Arc;

use super::decision::Decision;
use super::rule::PrefixRule;
use super::rule::RuleMatch;

/// 一次求值结果。
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Evaluation {
    /// 最严格裁决。
    pub decision: Decision,
    /// 命中前缀。
    pub matched_prefix: Vec<String>,
    /// 理由。
    pub justification: Option<String>,
}

/// 按首 token 索引的规则集合。
#[derive(Clone, Debug, Default)]
pub struct Policy {
    rules_by_program: HashMap<String, Vec<Arc<PrefixRule>>>,
}

impl Policy {
    /// 空策略（无规则文件）。
    pub fn empty() -> Self {
        Self::default()
    }

    /// 由平坦规则列表构建索引。
    pub fn from_rules(rules: Vec<Arc<PrefixRule>>) -> Self {
        let mut policy = Self::empty();
        for rule in rules {
            policy.add_rule(rule);
        }
        policy
    }

    /// 追加一条规则。
    pub fn add_rule(&mut self, rule: Arc<PrefixRule>) {
        self.rules_by_program
            .entry(rule.program().to_string())
            .or_default()
            .push(rule);
    }

    /// 追加另一个策略的全部规则。
    pub fn merge(&mut self, other: Policy) {
        for (_, rules) in other.rules_by_program {
            for rule in rules {
                self.add_rule(rule);
            }
        }
    }

    /// 规则总数。
    pub fn len(&self) -> usize {
        self.rules_by_program.values().map(Vec::len).sum()
    }

    /// 是否没有任何规则。
    pub fn is_empty(&self) -> bool {
        self.rules_by_program.is_empty()
    }

    /// 对一条命令取最严格裁决；无匹配返回 None。
    ///
    /// 同裁决时取更长前缀（更具体）的规则作为展示来源。
    pub fn evaluate(&self, command: &[String]) -> Option<Evaluation> {
        let first = command.first()?;
        let rules = self.rules_by_program.get(first)?;
        let mut best: Option<RuleMatch> = None;
        for rule in rules {
            let Some(candidate) = rule.matches(command) else {
                continue;
            };
            let better = match &best {
                None => true,
                Some(current) => {
                    candidate.decision > current.decision
                        || (candidate.decision == current.decision
                            && candidate.matched_prefix.len() > current.matched_prefix.len())
                }
            };
            if better {
                best = Some(candidate);
            }
        }
        best.map(|matched| Evaluation {
            decision: matched.decision,
            matched_prefix: matched.matched_prefix,
            justification: matched.justification,
        })
    }
}
