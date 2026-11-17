//! Starlark 规则文件解析：只暴露 `prefix_rule(...)`。
//!
//! 移植自 OpenAI Codex `execpolicy/src/parser.rs`（Apache-2.0），去掉
//! `network_rule` / `host_executable` 与 CLI 专用逻辑。

use std::cell::RefCell;
use std::cell::RefMut;
use std::sync::Arc;

use starlark::any::ProvidesStaticType;
use starlark::environment::GlobalsBuilder;
use starlark::environment::Module;
use starlark::eval::Evaluator;
use starlark::starlark_module;
use starlark::syntax::AstModule;
use starlark::syntax::Dialect;
use starlark::values::Value;
use starlark::values::list::ListRef;
use starlark::values::list::UnpackList;
use starlark::values::none::NoneType;
use starlark::values::tuple::TupleRef;

use super::decision::Decision;
use super::error::Error;
use super::error::Result;
use super::rule::PatternToken;
use super::rule::PrefixPattern;
use super::rule::PrefixRule;
use super::rule::validate_match_examples;
use super::rule::validate_not_match_examples;

/// 解析入口。
pub struct PolicyParser {
    builder: RefCell<PolicyBuilder>,
}

impl Default for PolicyParser {
    fn default() -> Self {
        Self::new()
    }
}

impl PolicyParser {
    /// 新建空解析器。
    pub fn new() -> Self {
        Self {
            builder: RefCell::new(PolicyBuilder::new()),
        }
    }

    /// 解析一个规则文件；失败时不会把半成品交给调用方。
    pub fn parse(&mut self, identifier: &str, contents: &str) -> Result<()> {
        let mut dialect = Dialect::Extended.clone();
        dialect.enable_f_strings = true;
        let ast = AstModule::parse(identifier, contents.to_string(), &dialect)?;
        let globals = GlobalsBuilder::standard().with(policy_builtins).build();
        Module::with_temp_heap(|module| {
            let mut eval = Evaluator::new(&module);
            eval.extra = Some(&self.builder);
            eval.eval_module(ast, &globals).map(|_| ())?;
            Ok::<(), Error>(())
        })?;
        Ok(())
    }

    /// 取出解析出的规则。
    pub fn build(self) -> Vec<Arc<PrefixRule>> {
        self.builder.into_inner().rules
    }
}

#[derive(Debug, ProvidesStaticType)]
struct PolicyBuilder {
    rules: Vec<Arc<PrefixRule>>,
}

impl PolicyBuilder {
    fn new() -> Self {
        Self { rules: Vec::new() }
    }
}

/// 解析 `pattern`；空模式拒绝。
fn parse_pattern(pattern: UnpackList<Value<'_>>) -> Result<Vec<PatternToken>> {
    let tokens: Vec<PatternToken> = pattern
        .items
        .into_iter()
        .map(parse_pattern_token)
        .collect::<Result<_>>()?;
    if tokens.is_empty() {
        Err(Error::InvalidPattern("pattern cannot be empty".into()))
    } else {
        Ok(tokens)
    }
}

fn parse_pattern_token(value: Value<'_>) -> Result<PatternToken> {
    if let Some(text) = value.unpack_str() {
        return Ok(PatternToken::Single(text.to_string()));
    }
    let Some(items) = string_items(value) else {
        return Err(Error::InvalidPattern(format!(
            "pattern element must be a string or list of strings (got {})",
            value.get_type()
        )));
    };
    let tokens: Vec<String> = items
        .iter()
        .copied()
        .map(|item| {
            item.unpack_str()
                .ok_or_else(|| {
                    Error::InvalidPattern(format!(
                        "pattern alternative must be a string (got {})",
                        item.get_type()
                    ))
                })
                .map(str::to_string)
        })
        .collect::<Result<_>>()?;
    match tokens.as_slice() {
        [] => Err(Error::InvalidPattern(
            "pattern alternatives cannot be empty".into(),
        )),
        [single] => Ok(PatternToken::Single(single.clone())),
        _ => Ok(PatternToken::Alts(tokens)),
    }
}

fn parse_examples(examples: UnpackList<Value<'_>>) -> Result<Vec<Vec<String>>> {
    examples.items.into_iter().map(parse_example).collect()
}

/// list 或 tuple 的元素（Codex 文档用 tuple 写备选，实际两者都接受）。
fn string_items<'v>(value: Value<'v>) -> Option<&'v [Value<'v>]> {
    if let Some(list) = ListRef::from_value(value) {
        return Some(list.content());
    }
    TupleRef::from_value(value).map(TupleRef::content)
}

fn parse_example(value: Value<'_>) -> Result<Vec<String>> {
    if let Some(raw) = value.unpack_str() {
        let tokens = shlex::split(raw)
            .ok_or_else(|| Error::InvalidExample("example has invalid shell syntax".into()))?;
        if tokens.is_empty() {
            return Err(Error::InvalidExample("example cannot be empty".into()));
        }
        return Ok(tokens);
    }
    let Some(items) = string_items(value) else {
        return Err(Error::InvalidExample(format!(
            "example must be a string or list of strings (got {})",
            value.get_type()
        )));
    };
    let tokens: Vec<String> = items
        .iter()
        .copied()
        .map(|item| {
            item.unpack_str()
                .ok_or_else(|| {
                    Error::InvalidExample(format!(
                        "example tokens must be strings (got {})",
                        item.get_type()
                    ))
                })
                .map(str::to_string)
        })
        .collect::<Result<_>>()?;
    if tokens.is_empty() {
        return Err(Error::InvalidExample("example cannot be empty".into()));
    }
    Ok(tokens)
}

fn policy_builder<'v, 'a>(eval: &Evaluator<'v, 'a, '_>) -> Result<RefMut<'a, PolicyBuilder>> {
    let extra = eval
        .extra
        .ok_or_else(|| Error::InvalidRule("parser context is missing".into()))?;
    let builder = extra
        .downcast_ref::<RefCell<PolicyBuilder>>()
        .ok_or_else(|| Error::InvalidRule("parser context has the wrong type".into()))?;
    Ok(builder.borrow_mut())
}

#[starlark_module]
fn policy_builtins(builder: &mut GlobalsBuilder) {
    fn prefix_rule<'v>(
        pattern: UnpackList<Value<'v>>,
        decision: Option<&'v str>,
        r#match: Option<UnpackList<Value<'v>>>,
        not_match: Option<UnpackList<Value<'v>>>,
        justification: Option<&'v str>,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> starlark::Result<NoneType> {
        let decision = match decision {
            Some(raw) => Decision::parse(raw)?,
            None => Decision::Allow,
        };
        let justification = match justification {
            Some(raw) if raw.trim().is_empty() => {
                return Err(Error::InvalidRule("justification cannot be empty".into()).into());
            }
            Some(raw) => Some(raw.to_string()),
            None => None,
        };
        let pattern_tokens = parse_pattern(pattern)?;
        let matches: Vec<Vec<String>> =
            r#match.map(parse_examples).transpose()?.unwrap_or_default();
        let not_matches: Vec<Vec<String>> = not_match
            .map(parse_examples)
            .transpose()?
            .unwrap_or_default();

        let (first, rest) = pattern_tokens
            .split_first()
            .ok_or_else(|| Error::InvalidPattern("pattern cannot be empty".into()))?;
        let rest: Arc<[PatternToken]> = rest.to_vec().into();
        let rules: Vec<Arc<PrefixRule>> = first
            .alternatives()
            .iter()
            .map(|head| {
                Arc::new(PrefixRule {
                    pattern: PrefixPattern {
                        first: Arc::from(head.as_str()),
                        rest: rest.clone(),
                    },
                    decision,
                    justification: justification.clone(),
                })
            })
            .collect();

        validate_not_match_examples(&rules, &not_matches)?;
        validate_match_examples(&rules, &matches)?;

        policy_builder(eval)?.rules.extend(rules);
        Ok(NoneType)
    }
}
