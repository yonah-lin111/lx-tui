//! 斜杠模板命令：触发识别、候选过滤、模板内容与面板状态。

use super::template::inside_template_block;
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
/// 未闭合围栏内或已有模板块内不触发。
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
    if open_fence(&text[..start]).is_some() || inside_template_block(text, start) {
        return None;
    }
    Some(SlashTrigger {
        line_start: start,
        line_end: end,
        cursor,
        query: query.to_string(),
    })
}

/// 按查询过滤命令：短名/长名双向匹配；空查询返回全部，排序稳定按打分降序。
pub fn slash_filter(query: &str) -> Vec<SlashCommandId> {
    let query = query.to_lowercase();
    let mut scored: Vec<(u32, SlashCommandId)> = SlashCommandId::ALL
        .into_iter()
        .filter_map(|id| {
            let score = slash_score(id, &query);
            (score > 0).then_some((score, id))
        })
        .collect();
    scored.sort_by_key(|(score, _)| std::cmp::Reverse(*score));
    scored.into_iter().map(|(_, id)| id).collect()
}

/// 短名/长名匹配打分：精确 > 短名前缀 > 长名前缀 > 包含。
fn slash_score(id: SlashCommandId, query: &str) -> u32 {
    if query.is_empty() {
        return 1;
    }
    let short = id.short_name();
    let long = id.long_name().to_lowercase();
    if short == query {
        5000
    } else if long == query {
        4500
    } else if short.starts_with(query) {
        3000
    } else if long.starts_with(query) {
        2500
    } else if short.contains(query) {
        1500
    } else if long.contains(query) {
        1000
    } else {
        0
    }
}

/// 标准模板文本与光标偏移（偏移相对模板起点，落在 `「title: 」` 的 `」` 之前）。
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

const ADD_TEMPLATE: &str = "&&& addTemplate --start\n「title: 」\n# Add Requirement\n\n- Reference: \n- Location: \n- Description: \n- Requirements: \n  - \n- Notes: \n  - \n&&& addTemplate --end";

const BUG_TEMPLATE: &str = "&&& bugTemplate --start\n「title: 」\n# Fix Bug\n\n- Reference: \n- Location: \n- Description: \n- Reproduction: \n- Requirements: \n  - \n- Expectations: \n- Notes: \n  - \n&&& bugTemplate --end";

const COMMON_TEMPLATE: &str = "&&& commonTemplate --start\n「title: 」\n# Execute Task\n\n- Reference: \n- Location: \n- Requirements: \n  - \n- Expectations: \n- Notes: \n  - \n&&& commonTemplate --end";

const REFACTOR_TEMPLATE: &str = "&&& refactorTemplate --start\n「title: 」\n# Refactor Feature\n\n- Reference: \n- Location: \n- Goal: \n- Requirements: \n  - \n- Notes: \n  - \n&&& refactorTemplate --end";

const STYLE_TEMPLATE: &str = "&&& styleTemplate --start\n「title: 」\n# Design Style\n\n- Reference: \n- Location: \n- Requirements: \n  - \n- Expectations: \n- Notes: \n  - \n&&& styleTemplate --end";

/// 标题占位符内的光标偏移：`「title: 」` 的 `」` 前；无占位符回退文末。
fn title_cursor(content: &str) -> usize {
    content
        .find('「')
        .and_then(|start| content[start..].find('」').map(|offset| start + offset))
        .unwrap_or(content.len())
}

/// 斜杠命令面板状态：触发、候选命令、高亮索引、窗口锚点与显式视口，模型同 `BlockPanel`。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SlashPanel {
    trigger: SlashTrigger,
    items: Vec<SlashCommandId>,
    active: usize,
    anchor: usize,
    viewport: Option<usize>,
}

impl SlashPanel {
    /// 组装面板状态；`items` 不可为空，`active` 调用方保证在界内。
    pub fn new(trigger: SlashTrigger, items: Vec<SlashCommandId>, active: usize) -> Self {
        Self {
            trigger,
            items,
            active,
            anchor: active,
            viewport: None,
        }
    }

    /// 触发区间。
    pub fn trigger(&self) -> &SlashTrigger {
        &self.trigger
    }

    /// 候选命令。
    pub fn items(&self) -> &[SlashCommandId] {
        &self.items
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
