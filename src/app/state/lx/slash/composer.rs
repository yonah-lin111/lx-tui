//! 详情输入框（Agents / BTW composer）的上下文命令：候选过滤、整段解析与参数提示。
//!
//! 与主输入框的 [`super::SlashCommand`] 刻意分离：主输入的 `/undo` 是 `/rewind` 的别名，
//! 而详情里的 `/undo` 走独立的撤销管线；`/report`、`/new` 也不属于主命令集。

use super::{SlashEntry, alias_prefix_score, command_score};

/// 详情输入框里的命令；只在 composer 上下文出现。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DetailCommand {
    Undo,
    Compact,
    Report,
    New,
}

/// 两类详情 composer；决定候选集合与执行语义。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DetailContext {
    Agent,
    Btw,
}

impl DetailCommand {
    /// Agents 详情命令（固定顺序）。
    pub const AGENT: [Self; 3] = [Self::Undo, Self::Compact, Self::Report];
    /// BTW 详情命令（固定顺序）。
    pub const BTW: [Self; 3] = [Self::Undo, Self::Compact, Self::New];

    /// 命令名（不含 `/`）。
    pub const fn name(self) -> &'static str {
        match self {
            Self::Undo => "undo",
            Self::Compact => "compact",
            Self::Report => "report",
            Self::New => "new",
        }
    }

    /// 命令文本（含 `/`）。
    pub const fn text(self) -> &'static str {
        match self {
            Self::Undo => "/undo",
            Self::Compact => "/compact",
            Self::Report => "/report",
            Self::New => "/new",
        }
    }

    /// 额外别名（不含 `/`）。
    pub fn aliases(self) -> &'static [&'static str] {
        match self {
            Self::Undo => &["rewind"],
            Self::New => &["clear", "reset"],
            _ => &[],
        }
    }

    /// 一句话描述；保持上下文中立，同一句在两个 composer 都成立。
    pub const fn description(self) -> &'static str {
        match self {
            Self::Undo => "Undo the last turn",
            Self::Compact => "Compact this conversation's context",
            Self::Report => "Report progress to the parent agent",
            Self::New => "Start a fresh side thread",
        }
    }

    /// 命令采纳后回显到输入框的带空格前缀（仅带参命令返回 `Some`）。
    pub const fn echo_prefix(self) -> Option<&'static str> {
        match self {
            Self::Report => Some("/report "),
            _ => None,
        }
    }

    /// 等待参数时的灰色提示。
    pub const fn argument_hint(self) -> Option<&'static str> {
        match self {
            Self::Report => Some("[note]"),
            _ => None,
        }
    }
}

/// 按上下文生成候选（命令名用模糊匹配，别名只用精确/前缀，规则与主命令一致）。
pub fn entries(context: DetailContext, query: &str) -> Vec<SlashEntry> {
    let query = query.trim_start_matches('/').to_lowercase();
    let all = match context {
        DetailContext::Agent => DetailCommand::AGENT.as_slice(),
        DetailContext::Btw => DetailCommand::BTW.as_slice(),
    };
    let mut scored: Vec<(u32, usize, DetailCommand)> = all
        .iter()
        .enumerate()
        .filter_map(|(index, command)| {
            let name = command_score(command.name(), &query);
            let alias = command
                .aliases()
                .iter()
                .filter_map(|alias| alias_prefix_score(alias, &query))
                // 别名略降权，同名命中优先。
                .map(|score| score.saturating_sub(1))
                .max();
            name.into_iter()
                .chain(alias)
                .max()
                .map(|score| (score, index, *command))
        })
        .collect();
    scored.sort_by(|left, right| right.0.cmp(&left.0).then_with(|| left.1.cmp(&right.1)));
    scored
        .into_iter()
        .map(|(_, _, command)| SlashEntry::Detail(command))
        .collect()
}

/// 详情命令的整段解析结果；`Report` 携带已 trim 的备注。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParsedDetailCommand<'a> {
    Undo,
    Compact,
    Report(&'a str),
    New,
}

/// 整段草稿必须是一条详情命令：首个非空白字符是 `/`，命令词后只允许参数。
///
/// `/report` 的备注允许换行；其余命令不允许任何参数。命令不能夹在长草稿中悄悄执行。
pub fn parse(input: &str, context: DetailContext) -> Option<ParsedDetailCommand<'_>> {
    let trimmed = input.trim_start_matches([' ', '\t']);
    let line = trimmed.strip_prefix('/')?;
    let (word, rest) = line
        .split_once(char::is_whitespace)
        .map_or((line, ""), |(word, rest)| (word, rest));
    let command = match (context, word) {
        (_, "undo") | (_, "rewind") => DetailCommand::Undo,
        (_, "compact") => DetailCommand::Compact,
        (DetailContext::Agent, "report") => DetailCommand::Report,
        (DetailContext::Btw, "new")
        | (DetailContext::Btw, "clear")
        | (DetailContext::Btw, "reset") => DetailCommand::New,
        _ => return None,
    };
    // 非带参命令必须结束在命令词之后：`/undo x` 不是命令，交给未知命令提示。
    if command != DetailCommand::Report && (!rest.trim().is_empty() || rest.contains('\n')) {
        return None;
    }
    Some(match command {
        DetailCommand::Undo => ParsedDetailCommand::Undo,
        DetailCommand::Compact => ParsedDetailCommand::Compact,
        DetailCommand::Report => ParsedDetailCommand::Report(rest.trim()),
        DetailCommand::New => ParsedDetailCommand::New,
    })
}

/// 处于「`/report ` + 尾随空格、尚无备注」状态时返回灰色参数提示。
pub fn argument_hint(input: &str) -> Option<&'static str> {
    let trimmed = input.trim_start_matches([' ', '\t']);
    let line = trimmed.strip_prefix('/')?;
    let rest = line.strip_prefix(DetailCommand::Report.name())?;
    (rest.starts_with(' ') && rest.trim().is_empty())
        .then(|| DetailCommand::Report.argument_hint())
        .flatten()
}

#[cfg(test)]
mod tests;
