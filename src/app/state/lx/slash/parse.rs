//! `/` 命令模糊打分与单行命令解析（从 `slash.rs` 拆分）。

use super::{SlashCommand, composer};

/// 命令名模糊匹配打分（对齐 `@` 提及的层次：精确 > 前缀 > 子串 > 子序列）。
///
/// 返回 None 表示不匹配；分数越高越靠前。空查询给基础分（保持固定顺序）。
///
/// 二级面板（`/resume`、`/model`）的过滤复用同一套打分，只把 `name` 换成条目文本。
pub(crate) fn command_score(name: &str, query: &str) -> Option<u32> {
    if query.is_empty() {
        return Some(100);
    }
    if name == query {
        return Some(1000);
    }
    if name.starts_with(query) {
        return Some(800);
    }
    if name.contains(query) {
        return Some(600);
    }
    // 子序列：query 的字符按序出现在 name 中。
    let mut chars = name.chars();
    let mut matched = 0usize;
    for needle in query.chars() {
        if chars.by_ref().any(|ch| ch == needle) {
            matched += 1;
        } else {
            return None;
        }
    }
    (matched == query.chars().count()).then_some(400)
}

/// 别名匹配打分：仅精确/前缀（不做子串/子序列，避免 `/r` 之类误命中别名）。
///
/// 别名是「用另一个名字访问同一命令」，按从头输入的名字匹配即可。
pub(crate) fn alias_prefix_score(alias: &str, query: &str) -> Option<u32> {
    if query.is_empty() {
        return Some(100);
    }
    if alias == query {
        return Some(1000);
    }
    alias.starts_with(query).then_some(800)
}

/// 行首命令位置判定：当前段落首个非空字符是 `/` 且光标不在围栏代码块内。
///
/// 输入 `/model` 并确认后消费整条命令，不写入草稿或 DB。
pub fn slash_trigger(input: &str, cursor: usize) -> Option<String> {
    let before = &input[..cursor];
    // 围栏代码块内不触发。
    if crate::app::prompt::list::in_fence(input, cursor) {
        return None;
    }
    // 命令面板只在行首命令位置：当前光标所在行的前缀（含换行前内容）。
    let line_start = before.rfind('\n').map_or(0, |offset| offset + 1);
    let line_prefix = &input[line_start..cursor];
    // 行首必须是 `/`（无前导文本），且命令词内部不含空白（输入空格即进入命令参数区，关闭候选面板）。
    if !line_prefix.starts_with('/') || line_prefix.contains(char::is_whitespace) {
        return None;
    }
    Some(line_prefix.to_string())
}

/// 当输入框全文处于某带参命令的「已回显命令 + 尾随空格、尚无有效参数」状态时，
/// 返回应当紧跟渲染的灰色参数说明文案（如 `"[name]"`）。
///
/// 兼容主名（`/rename `）与别名（`/title `）。
pub fn active_argument_hint(input: &str) -> Option<&'static str> {
    if input.contains('\n') || input.contains('\r') {
        return None;
    }
    let trimmed = input.trim_start_matches([' ', '\t']);
    let line = trimmed.strip_prefix('/')?;
    for command in [SlashCommand::Rename, SlashCommand::Btw] {
        let matched = std::iter::once(command.name())
            .chain(command.aliases().iter().copied())
            .any(|keyword| {
                line.strip_prefix(keyword)
                    .is_some_and(|rest| rest.starts_with(' ') && rest.trim().is_empty())
            });
        if matched {
            return command.argument_hint();
        }
    }
    // 详情 composer 的 `/report `：主输入框不会命中 `/report` 命令，提示只是灰色文本。
    composer::argument_hint(input)
}

/// 解析 `/btw [prompt]`：返回 `(prompt, 是否为裸命令)`。
///
/// 与 `/rename` 不同，问题允许换行：命令词必须是首个非空白词，其余全文都是问题。
pub fn parse_btw_command(input: &str) -> Option<(&str, bool)> {
    let trimmed = input.trim_start_matches([' ', '\t']);
    let line = trimmed.strip_prefix('/')?;
    let rest = line.strip_prefix(SlashCommand::Btw.name())?;
    if !rest.is_empty() && !rest.starts_with(char::is_whitespace) {
        return None;
    }
    let prompt = rest.trim();
    Some((prompt, prompt.is_empty()))
}

/// 解析单行 `/rename`（或别名 `/title`）命令输入；
/// 若匹配，返回命令后的参数切片（已去除首尾空白，可能为空串 `""`）。
pub fn parse_rename_command(input: &str) -> Option<&str> {
    let trimmed = input.trim_start_matches([' ', '\t']);
    if trimmed.contains('\n') || trimmed.contains('\r') {
        return None;
    }
    let line = trimmed.strip_prefix('/')?;
    for keyword in std::iter::once(SlashCommand::Rename.name())
        .chain(SlashCommand::Rename.aliases().iter().copied())
    {
        if let Some(rest) = line.strip_prefix(keyword)
            && (rest.is_empty() || rest.starts_with(char::is_whitespace))
        {
            return Some(rest.trim());
        }
    }
    None
}
