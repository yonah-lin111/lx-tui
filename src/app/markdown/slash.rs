//! 斜杠命令：触发识别、内置/自定义候选过滤、插入内容与面板状态。

use super::template::{parse_template_end_line, parse_template_start_line};
use super::{line_end, line_start, open_fence};

/// 斜杠模板命令标识；面板展示短名，模板块使用长名。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SlashCommandId {
    Add,
    Bug,
    Common,
    Refactor,
    Style,
}

impl SlashCommandId {
    /// 全部内置命令（面板候选顺序）。
    pub const ALL: [SlashCommandId; 5] = [
        SlashCommandId::Add,
        SlashCommandId::Bug,
        SlashCommandId::Common,
        SlashCommandId::Refactor,
        SlashCommandId::Style,
    ];

    /// 面板短名（不含 `/`）。
    pub fn short_name(self) -> &'static str {
        match self {
            SlashCommandId::Add => "add",
            SlashCommandId::Bug => "bug",
            SlashCommandId::Common => "common",
            SlashCommandId::Refactor => "refactor",
            SlashCommandId::Style => "style",
        }
    }

    /// 长别名，同时是模板块命令名（不含 `/`）。
    pub fn long_name(self) -> &'static str {
        match self {
            SlashCommandId::Add => "addTemplate",
            SlashCommandId::Bug => "bugTemplate",
            SlashCommandId::Common => "commonTemplate",
            SlashCommandId::Refactor => "refactorTemplate",
            SlashCommandId::Style => "styleTemplate",
        }
    }

    /// 由命令名解析（兼容短名与长名）；未知命令返回 None。
    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|id| id.short_name() == name || id.long_name() == name)
    }
}

/// 斜杠命令触发：`line_start..line_end` 为整行替换范围，`query` 为 `/` 与光标之间的输入。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SlashTrigger {
    pub line_start: usize,
    pub line_end: usize,
    pub cursor: usize,
    pub query: String,
}

/// 解析光标处的斜杠命令触发：整行仅有前导空格、`/` 与命令字符，且光标在行尾。
///
/// 未闭合围栏内不触发；模板块内允许触发（`/gitWorkTree` 需要在块内绑定），
/// 可见候选按作用域过滤（内置模板与模板块命令仅在块外，模板作用域 Markdown
/// 仅在块内）。
pub fn slash_trigger(text: &str, cursor: usize) -> Option<SlashTrigger> {
    if cursor > text.len() || !text.is_char_boundary(cursor) {
        return None;
    }
    let start = line_start(text, cursor);
    let end = line_end(text, cursor);
    if cursor != end {
        return None;
    }
    let line = &text[start..end];
    let indent = line.len() - line.trim_start_matches(' ').len();
    let query = line[indent..].strip_prefix('/')?;
    if !query
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || ch == '_' || ch == '-')
    {
        return None;
    }
    if open_fence(&text[..start]).is_some() {
        return None;
    }
    Some(SlashTrigger {
        line_start: start,
        line_end: end,
        cursor,
        query: query.to_string(),
    })
}

/// 按查询过滤候选：内置模板 + 自定义命令 + `/gitWorkTree`。
///
/// 可见范围：内置模板只在模板块外出现（块内已有模板）；globally 作用域的自定义
/// Markdown 两处都出现，template 作用域的只在块内出现；`/gitWorkTree` 仅在块外
/// 且工作区有 git 元数据时出现。空查询返回全部，排序稳定按打分降序，同分内置优先。
pub fn slash_filter(custom: &[SlashItem], context: SlashContext, query: &str) -> Vec<SlashItem> {
    let query = query.to_lowercase();
    let mut candidates: Vec<SlashItem> = Vec::new();
    if !context.inside_block {
        candidates.extend(SlashCommandId::ALL.into_iter().map(SlashItem::builtin));
    } else if context.worktree_available {
        // `/gitWorkTree` 只在模板块内出现：绑定该块的文件提及根。
        candidates.push(SlashItem::worktree());
    }
    candidates.extend(
        custom
            .iter()
            .filter(|item| match &item.insertion {
                // 完整模板块：仅在块外插入。
                SlashInsertion::AgentBlock { .. } => !context.inside_block,
                // template 作用域的 Markdown：仅在块内。
                SlashInsertion::Raw(_) => match item.scope {
                    SlashScope::Global => true,
                    SlashScope::Template => context.inside_block,
                },
                _ => false,
            })
            .cloned(),
    );
    let mut scored: Vec<(u32, usize, SlashItem)> = candidates
        .into_iter()
        .enumerate()
        .filter_map(|(index, item)| {
            let score = slash_score(&item, &query);
            (score > 0).then_some((score, index, item))
        })
        .collect();
    scored.sort_by(|left, right| right.0.cmp(&left.0).then_with(|| left.1.cmp(&right.1)));
    scored.into_iter().map(|(_, _, item)| item).collect()
}

/// 短名优先的模糊打分；分层与数值对齐 `mention_score`（文件名优先）：
/// 精确 > 短名前缀 > 长名前缀 > 短名包含 > 短名子序列 > 长名包含 > 长名子序列。
fn slash_score(item: &SlashItem, query: &str) -> u32 {
    if query.is_empty() {
        return 1;
    }
    let short = item.name.to_lowercase();
    let long = item.preview.to_lowercase();
    if short == query {
        return 5000;
    }
    if !long.is_empty() && long == query {
        return 4000;
    }
    if short.starts_with(query) {
        return 3000;
    }
    if !long.is_empty() && long.starts_with(query) {
        return 2000;
    }
    if short.contains(query) {
        return 1800;
    }
    if let Some(span) = super::subsequence_span(&short, query) {
        return 1200.max(1500_u32.saturating_sub(span * 10));
    }
    if !long.is_empty() && long.contains(query) {
        return 800;
    }
    if !long.is_empty()
        && let Some(span) = super::subsequence_span(&long, query)
    {
        return 100.max(500_u32.saturating_sub(span));
    }
    0
}

/// 自定义模板块正文：`&&& <name> --start` + `{title: …}` 标题占位行 + 正文 + `&&& <name> --end`。
///
/// 正文若自带完整块结构会先规范化，避免嵌套起止行；光标偏移落在标题占位符的
/// `}` 之前；标题缺省时值为空。
pub fn agent_block_content(command: &str, title: Option<&str>, body: &str) -> (String, usize) {
    // 标题值不得含花括号，否则 {title: …} 行不可解析。
    let title = title.unwrap_or_default().trim().replace(['{', '}'], "");
    let title = title.as_str();
    let body = normalize_block_body(body);
    // 标题行与正文之间固定一条分割线（与 `{wt:}` 前的分割线一致）。
    let content = format!(
        "&&& {command} --start\n{{title: {title}}}\n{}\n{body}\n&&& {command} --end",
        super::template::TEMPLATE_SEPARATOR
    );
    let cursor = title_cursor(&content);
    (content, cursor)
}

/// 候选命令的插入结果：整行替换文本与光标偏移；选择器条目与占位行返回 None。
pub fn slash_insertion(item: &SlashItem) -> Option<(String, usize)> {
    match &item.insertion {
        SlashInsertion::BuiltinTemplate(id) => {
            let (content, cursor) = slash_template_content(*id);
            Some((content.to_string(), cursor))
        }
        SlashInsertion::Raw(body) => Some((body.clone(), body.len())),
        SlashInsertion::AgentBlock {
            command,
            title,
            body,
        } => Some(agent_block_content(command, title.as_deref(), body)),
        SlashInsertion::WorktreeChooser
        | SlashInsertion::WorktreeBinding(_)
        | SlashInsertion::Placeholder => None,
    }
}

/// 规范化粘贴/输入的自定义模板块文本。
///
/// 完整的 `&&& x --start … &&& x --end` 保持单一配对；仅起点行时补结束行；
/// 完全无块结构时整体作为块正文。返回正文（不含起止行），由插入侧重新包壳。
pub fn normalize_block_body(body: &str) -> String {
    let lines: Vec<&str> = body.trim().split('\n').collect();
    let start = lines.first().copied().unwrap_or_default();
    if parse_template_start_line(start).is_none() {
        return body.trim().to_string();
    }
    let end = lines.last().copied().unwrap_or_default();
    let (from, to) = if lines.len() >= 2 && parse_template_end_line(end).is_some() {
        (1, lines.len() - 1)
    } else {
        (1, lines.len())
    };
    // 正文自带的标题行与标题下分割线剥离，由插入侧重新包壳，避免出现两行标题。
    let from = if lines
        .get(from)
        .is_some_and(|line| super::template::is_title_line(line))
    {
        from + 1
    } else {
        from
    };
    let from = if lines
        .get(from)
        .is_some_and(|line| super::template::is_template_separator(line))
    {
        from + 1
    } else {
        from
    };
    // 尾部的绑定行与其分割线同样剥离（重新包壳后绑定由 `/gitWorkTree` 重写）。
    let mut to = to;
    let last = |index: usize| {
        index
            .checked_sub(1)
            .and_then(|index| lines.get(index).copied())
    };
    if last(to).is_some_and(|line| super::template::parse_worktree_line(line).is_some())
        && to > from
    {
        to -= 1;
    }
    if last(to).is_some_and(super::template::is_template_separator) && to > from {
        to -= 1;
    }
    lines[from.min(to)..to].join("\n").trim_end().to_string()
}

/// 标准模板文本与光标偏移（偏移相对模板起点，落在 `{title: }` 的 `}` 之前）。
///
/// 标题占位符独立成行（`--start` 的下一行行首），避免与起止行标记和操作按钮挤在一行。
pub fn slash_template_content(id: SlashCommandId) -> (&'static str, usize) {
    let content = match id {
        SlashCommandId::Add => ADD_TEMPLATE,
        SlashCommandId::Bug => BUG_TEMPLATE,
        SlashCommandId::Common => COMMON_TEMPLATE,
        SlashCommandId::Refactor => REFACTOR_TEMPLATE,
        SlashCommandId::Style => STYLE_TEMPLATE,
    };
    (content, title_cursor(content))
}

const ADD_TEMPLATE: &str = "&&& addTemplate --start\n{title: }\n---\n# Add Requirement\n\n- Reference: \n- Location: \n- Description: \n- Requirements: \n  - \n- Notes: \n  - \n&&& addTemplate --end";

const BUG_TEMPLATE: &str = "&&& bugTemplate --start\n{title: }\n---\n# Fix Bug\n\n- Reference: \n- Location: \n- Description: \n- Reproduction: \n- Requirements: \n  - \n- Expectations: \n- Notes: \n  - \n&&& bugTemplate --end";

const COMMON_TEMPLATE: &str = "&&& commonTemplate --start\n{title: }\n---\n# Execute Task\n\n- Reference: \n- Location: \n- Requirements: \n  - \n- Expectations: \n- Notes: \n  - \n&&& commonTemplate --end";

const REFACTOR_TEMPLATE: &str = "&&& refactorTemplate --start\n{title: }\n---\n# Refactor Feature\n\n- Reference: \n- Location: \n- Goal: \n- Requirements: \n  - \n- Notes: \n  - \n&&& refactorTemplate --end";

const STYLE_TEMPLATE: &str = "&&& styleTemplate --start\n{title: }\n---\n# Design Style\n\n- Reference: \n- Location: \n- Requirements: \n  - \n- Expectations: \n- Notes: \n  - \n&&& styleTemplate --end";

/// 标题占位符内的光标偏移：`{title: }` 的 `}` 前；无占位符回退文末。
fn title_cursor(content: &str) -> usize {
    content
        .find("{title:")
        .and_then(|start| content[start..].find('}').map(|offset| start + offset))
        .unwrap_or(content.len())
}

/// 命令来源：决定面板右侧 `[Built-in]` / `[User]` / `[Project]` / `[wt]` 标签；
/// Git 来源即 worktree 相关行（`/gitWorkTree` 与其选择器条目），用 `wt` 更达意。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommandSource {
    BuiltIn,
    User,
    Project,
    Git,
}

impl CommandSource {
    /// 标签文案（不含方括号，由 UI 层包裹）。
    pub fn label(self) -> &'static str {
        match self {
            CommandSource::BuiltIn => "Built-in",
            CommandSource::User => "User",
            CommandSource::Project => "Project",
            CommandSource::Git => "wt",
        }
    }
}

/// 候选命令的插入动作。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SlashInsertion {
    /// 内置命令：整行替换为内置模板块，光标落在标题占位符内。
    BuiltinTemplate(SlashCommandId),
    /// 自定义 Markdown：整行替换为正文，光标落在正文末尾。
    Raw(String),
    /// 自定义模板块：整行替换为 `&&& <name> --start/--end` 块，光标落在标题内。
    AgentBlock {
        command: String,
        title: Option<String>,
        body: String,
    },
    /// `/gitWorkTree`：不直接插入，打开二级 worktree 选择器。
    WorktreeChooser,
    /// 二级选择器条目：把对应分支写回块结束行上方的 `{wt:}` 行。
    WorktreeBinding(String),
    /// 选择器占位行：不可确认。
    Placeholder,
}

/// 自定义命令的 Markdown 插入范围。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SlashScope {
    /// 任意位置可用。
    Global,
    /// 仅模板块内可用（模板作用域）。
    Template,
}

/// 一条斜杠候选命令。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SlashItem {
    /// 命令名（不含 `/`）；占位行为空串。
    pub name: String,
    pub source: CommandSource,
    pub insertion: SlashInsertion,
    /// 面板右侧预览列；空串表示无预览。
    pub preview: String,
    /// 自定义命令的可见范围；内置与选择器条目恒为 `Global`。
    pub scope: SlashScope,
    /// 是否可确认；选择器占位行（加载中/空列表）为 false。
    pub selectable: bool,
}

impl SlashItem {
    /// 内置命令候选：短名 + 长别名预览。
    pub fn builtin(id: SlashCommandId) -> Self {
        Self {
            name: id.short_name().to_string(),
            source: CommandSource::BuiltIn,
            insertion: SlashInsertion::BuiltinTemplate(id),
            preview: id.long_name().to_string(),
            scope: SlashScope::Template,
            selectable: true,
        }
    }

    /// 自定义 Markdown 命令。
    pub fn custom_markdown(
        name: &str,
        source: CommandSource,
        scope: SlashScope,
        body: &str,
        description: &str,
    ) -> Self {
        Self {
            name: name.to_string(),
            source,
            insertion: SlashInsertion::Raw(body.to_string()),
            preview: description.to_string(),
            scope,
            selectable: true,
        }
    }

    /// 自定义模板块命令。
    pub fn custom_block(
        name: &str,
        source: CommandSource,
        title: Option<&str>,
        body: &str,
        description: &str,
    ) -> Self {
        Self {
            name: name.to_string(),
            source,
            insertion: SlashInsertion::AgentBlock {
                command: name.to_string(),
                title: title.map(str::to_string),
                body: body.to_string(),
            },
            preview: description.to_string(),
            scope: SlashScope::Template,
            selectable: true,
        }
    }

    /// `/gitWorkTree` 候选：打开二级 worktree 选择器。
    pub fn worktree() -> Self {
        Self {
            name: "gitWorkTree".to_string(),
            source: CommandSource::Git,
            insertion: SlashInsertion::WorktreeChooser,
            preview: String::new(),
            scope: SlashScope::Global,
            selectable: true,
        }
    }

    /// 选择器条目：确认后写回对应分支的 `{wt:}` 行。
    pub fn worktree_entry(candidate: &super::WorktreeCandidate) -> Self {
        Self {
            name: candidate.branch.clone(),
            source: CommandSource::Git,
            insertion: SlashInsertion::WorktreeBinding(candidate.branch.clone()),
            preview: candidate.path.display().to_string(),
            scope: SlashScope::Global,
            selectable: true,
        }
    }

    /// 选择器占位行：不可确认，仅提示加载/空状态。
    pub fn placeholder(label: &str) -> Self {
        Self {
            name: String::new(),
            source: CommandSource::Git,
            insertion: SlashInsertion::Placeholder,
            preview: label.to_string(),
            scope: SlashScope::Global,
            selectable: false,
        }
    }

    /// 面板标签：`/` 前缀 + 命令名；选择器条目无 `/` 前缀。
    pub fn label(&self) -> String {
        match self.source {
            CommandSource::Git if !self.name.is_empty() => self.name.clone(),
            _ if self.name.is_empty() => self.preview.clone(),
            _ => format!("/{}", self.name),
        }
    }
}

/// 面板候选的可见范围上下文。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SlashContext {
    /// 光标是否位于已闭合或未闭合的模板块内。
    pub inside_block: bool,
    /// 显示中工作区是否具备 git 仓库元数据。
    pub worktree_available: bool,
}

/// 斜杠面板的显示模式：候选命令或二级 worktree 选择器。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SlashPanelKind {
    Commands,
    /// worktree 选择器：绑定块的结束行索引；列表由 `Prompt` 的 worktree 状态提供。
    WorktreeChooser {
        block_end_line: usize,
    },
}

/// 斜杠命令面板状态：触发、候选、高亮索引、窗口锚点与显式视口，模型同 `BlockPanel`。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SlashPanel {
    trigger: SlashTrigger,
    kind: SlashPanelKind,
    items: Vec<SlashItem>,
    active: usize,
    anchor: usize,
    viewport: Option<usize>,
}

impl SlashPanel {
    /// 组装命令候选面板；`items` 不可为空，`active` 调用方保证在界内。
    pub fn new(trigger: SlashTrigger, items: Vec<SlashItem>, active: usize) -> Self {
        Self::with_kind(trigger, SlashPanelKind::Commands, items, active)
    }

    /// 组装指定模式的面板。
    pub fn with_kind(
        trigger: SlashTrigger,
        kind: SlashPanelKind,
        items: Vec<SlashItem>,
        active: usize,
    ) -> Self {
        Self {
            trigger,
            kind,
            items,
            active,
            anchor: active,
            viewport: None,
        }
    }

    /// 面板模式。
    pub fn kind(&self) -> &SlashPanelKind {
        &self.kind
    }

    /// 触发区间。
    pub fn trigger(&self) -> &SlashTrigger {
        &self.trigger
    }

    /// 候选命令。
    pub fn items(&self) -> &[SlashItem] {
        &self.items
    }

    /// 条目总数。
    pub fn entry_count(&self) -> usize {
        self.items.len()
    }

    /// 是否可调整高亮（存在可选项）。
    pub fn has_selectable(&self) -> bool {
        self.items.iter().any(|item| item.selectable)
    }

    /// 按偏移移动到下一个可选项；占位行跳过；无可选项时不动。
    pub fn move_active_selectable(&mut self, delta: isize) {
        if !self.has_selectable() {
            return;
        }
        let len = self.items.len() as isize;
        let mut index = self.active as isize;
        for _ in 0..delta.unsigned_abs() {
            let step = if delta < 0 { -1 } else { 1 };
            for _ in 0..len {
                index = (index + step).rem_euclid(len);
                if self.items[index as usize].selectable {
                    break;
                }
            }
        }
        self.active = index as usize;
        self.anchor = self.active;
        self.viewport = None;
    }
    /// 高亮索引。
    pub fn active(&self) -> usize {
        self.active
    }

    /// 窗口锚点条目（窗口底部）。
    pub fn anchor(&self) -> usize {
        self.anchor
    }

    /// 滚轮滚动后的显式视口起点；跟随高亮时为 None。
    pub fn viewport(&self) -> Option<usize> {
        self.viewport
    }

    /// 直接设置高亮索引（鼠标悬停）；越界钳到末项，窗口锚点不动。
    pub fn set_active(&mut self, index: usize) {
        self.active = index.min(self.items.len().saturating_sub(1));
    }

    /// 按偏移循环移动高亮项；窗口锚点跟随高亮，显式视口清除。
    pub fn move_active(&mut self, delta: isize) {
        let len = self.items.len() as isize;
        self.active = (self.active as isize + delta).rem_euclid(len) as usize;
        self.anchor = self.active;
        self.viewport = None;
    }

    /// 滚轮滚动可见窗口：显式视口从 `base` 起偏移并钳制（不循环），高亮与锚点不动。
    pub fn scroll_viewport(&mut self, delta: isize, base: usize) -> bool {
        let max = self.items.len().saturating_sub(1) as isize;
        let next = (base.min(max as usize) as isize + delta).clamp(0, max) as usize;
        let changed = next != base;
        self.viewport = Some(next);
        changed
    }
}
