//! `/` 命令面板状态：model（两级选择）、resume（历史列表）与保留命令。
//!
//! 与 `@` 提及、粘贴参照、markdown 块面板互斥；只在行首命令位置触发。

use crate::config::ai::{AiSettings, ModelSelection};

/// 面板候选窗口状态：键盘/悬停/滚轮共用的高亮、锚点与显式视口。
///
/// 与 `@` 提及面板同一套语义：键盘移动高亮并让窗口锚点跟随；悬停只改高亮
/// 不动锚点；滚轮只动显式视口（高亮可滚出视口）。`ui` 用 `anchor`/`viewport`
/// 传给 `CommandPanelView`，达到「与 @ 面板一致」的滚动观感。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PanelWindow {
    /// 高亮索引（0 基）。
    pub active: usize,
    /// 窗口底部锚点条目；跟随高亮时为高亮项。
    pub anchor: usize,
    /// 滚轮滚动后的显式视口起点；跟随高亮时为 None。
    pub viewport: Option<usize>,
}

impl PanelWindow {
    /// 以高亮项为锚点初始化。
    pub fn new(active: usize) -> Self {
        Self {
            active,
            anchor: active,
            viewport: None,
        }
    }

    /// 高亮项数变化后钳制三个索引，避免越界。
    pub fn clamp(&mut self, len: usize) {
        let last = len.saturating_sub(1);
        self.active = self.active.min(last);
        self.anchor = self.anchor.min(last);
        self.viewport = self.viewport.map(|start| start.min(last));
    }

    /// 设置高亮（鼠标悬停）：越界钳到末项，锚点与视口不动。
    pub fn set_active(&mut self, index: usize) {
        self.active = index;
    }

    /// 键盘移动高亮（循环）：锚点跟随高亮并清除显式视口。
    pub fn move_active(&mut self, delta: isize, len: usize) {
        if len == 0 {
            return;
        }
        self.active = (self.active as isize + delta).rem_euclid(len as isize) as usize;
        self.anchor = self.active;
        self.viewport = None;
    }

    /// 滚轮滚动窗口：显式视口从 `base` 起偏移并钳制（不循环），高亮与锚点不动。
    pub fn scroll(&mut self, delta: isize, base: usize, len: usize) {
        let max = len.saturating_sub(1) as isize;
        let next = (base.min(max as usize) as isize + delta).clamp(0, max) as usize;
        self.viewport = Some(next);
    }
}

impl Default for PanelWindow {
    fn default() -> Self {
        Self::new(0)
    }
}

/// 保留命令。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SlashCommand {
    /// `/model`：先选模型，再选其 variants。
    Model,
    /// `/resume`：当前工作区历史列表。
    Resume,
    /// `/new`：清当前窗格会话关联并回空白草稿。
    New,
}

impl SlashCommand {
    /// 全部命令（固定顺序）。
    pub const ALL: [SlashCommand; 3] =
        [SlashCommand::Model, SlashCommand::Resume, SlashCommand::New];

    /// 命令名（不含 `/`）。
    pub fn name(self) -> &'static str {
        match self {
            Self::Model => "model",
            Self::Resume => "resume",
            Self::New => "new",
        }
    }

    /// 额外别名（不含 `/`）；用于命令面板的模糊匹配（`/session` 命中 `/resume`）。
    pub fn aliases(self) -> &'static [&'static str] {
        match self {
            Self::Model => &[],
            Self::Resume => &["session"],
            Self::New => &["clear"],
        }
    }

    /// 命令文本（含 `/`）。
    pub fn text(self) -> &'static str {
        match self {
            Self::Model => "/model",
            Self::Resume => "/resume",
            Self::New => "/new",
        }
    }

    /// 一句话描述。
    pub fn description(self) -> &'static str {
        match self {
            Self::Model => "Choose the model and reasoning level for the next turn",
            Self::Resume => crate::ui::text::LX_RESUME_COMMAND_DESC,
            Self::New => "Start a fresh blank session",
        }
    }

    /// 面板右对齐来源标签（内置命令）。
    pub fn tag(self) -> &'static str {
        crate::ui::text::SLASH_TAG_BUILTIN
    }
}

/// `/model` 第一级候选：选择键 + 展示名（供应商名 / 模型名）。
///
/// 选择与发送用 `selection` 的稳定 id；面板展示用 `provider_name` / `model_name`
/// （配置里的可读名），避免把 id 直接暴露给用户。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelCandidate {
    pub selection: ModelSelection,
    pub provider_name: String,
    pub model_name: String,
}

/// `/model` 面板的两级视图。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ModelPickerStage {
    /// 第一级：模型列表（模型名 + 供应商名展示行）。
    Models,
    /// 第二级：所选模型的 variants；`variants` 为空时直接跳过（不进入本层）。
    Variants {
        model: ModelSelection,
        /// 该模型的 variants 键（渲染候选）。
        variants: Vec<String>,
    },
}

/// `/model` 面板。
#[derive(Debug, Clone, PartialEq)]
pub struct ModelPicker {
    pub stage: ModelPickerStage,
    /// 第一级候选：启用的 provider × 其模型（含展示名）。
    pub candidates: Vec<ModelCandidate>,
    /// 窗口状态（高亮/锚点/显式视口）。
    pub window: PanelWindow,
}

impl ModelPicker {
    /// 从配置构造第一级候选（只含已启用且有模型与 Key 的 provider）。
    pub fn candidates(settings: &AiSettings) -> Vec<ModelCandidate> {
        let mut out = Vec::new();
        for provider_id in &settings.enabled_providers {
            let Some(provider) = settings.providers.get(provider_id) else {
                continue;
            };
            let has_key = provider
                .options
                .api_key
                .as_deref()
                .is_some_and(|key| !key.is_empty());
            if !has_key {
                continue;
            }
            for (model_key, model) in &provider.models {
                out.push(ModelCandidate {
                    selection: ModelSelection {
                        provider: provider_id.clone(),
                        model: model_key.clone(),
                        variant: model.variant.clone(),
                    },
                    provider_name: provider.name.clone(),
                    model_name: model.name.clone(),
                });
            }
        }
        out
    }
}

/// `/resume` 历史列表条目。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HistoryEntry {
    pub session_id: String,
    pub title: String,
    /// `active` / `closed`。
    pub status: String,
    pub updated_at_ms: i64,
}

/// `/resume` 历史面板。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct HistoryPanel {
    pub entries: Vec<HistoryEntry>,
    /// 窗口状态（高亮/锚点/显式视口）。
    pub window: PanelWindow,
}

/// `/` 命令面板（互斥浮层；同一时刻至多一个视图）。
#[derive(Debug, Clone, PartialEq)]
pub enum SlashPanel {
    /// 命令候选列表（输入 `/` 后过滤）。
    Commands {
        items: Vec<SlashCommand>,
        window: PanelWindow,
    },
    /// `/model` 两级选择。
    Model(Box<ModelPicker>),
    /// `/resume` 历史列表。
    History(Box<HistoryPanel>),
}

impl SlashPanel {
    /// 命令候选的过滤：模糊匹配命令名与别名（对齐 `@` 提及的子串/子序列排序）。
    ///
    /// `query` 为 `/` 之后输入（可含前导 `/`）。空查询返回全部（保持固定顺序）。
    /// 别名（如 `/session`→`resume`、`/clear`→`new`）按精确/前缀/子串命中同一命令
    /// （不做子序列匹配，避免 `/r` 之类误命中）。
    pub fn commands_matching(query: &str) -> Vec<SlashCommand> {
        let query = query.trim_start_matches('/').to_lowercase();
        let mut scored: Vec<(u32, usize, SlashCommand)> = SlashCommand::ALL
            .into_iter()
            .enumerate()
            .filter_map(|(index, command)| {
                let name_score = command_score(command.name(), &query);
                let alias_score = command
                    .aliases()
                    .iter()
                    .filter_map(|alias| alias_prefix_score(alias, &query))
                    // 别名略降权，同名命中优先。
                    .map(|score| score.saturating_sub(1))
                    .max();
                name_score
                    .into_iter()
                    .chain(alias_score)
                    .max()
                    .map(|score| (score, index, command))
            })
            .collect();
        scored.sort_by(|left, right| right.0.cmp(&left.0).then_with(|| left.1.cmp(&right.1)));
        scored.into_iter().map(|(_, _, command)| command).collect()
    }

    /// 当前视图的候选数量。
    pub fn item_count(&self) -> usize {
        match self {
            Self::Commands { items, .. } => items.len(),
            Self::Model(picker) => match &picker.stage {
                ModelPickerStage::Models => picker.candidates.len(),
                ModelPickerStage::Variants { variants, .. } => variants.len(),
            },
            Self::History(panel) => panel.entries.len(),
        }
    }

    /// 当前视图的窗口状态。
    pub fn window(&self) -> PanelWindow {
        match self {
            Self::Commands { window, .. } => *window,
            Self::Model(picker) => picker.window,
            Self::History(panel) => panel.window,
        }
    }

    /// 写回窗口状态（索引变化后）。
    pub fn set_window(&mut self, window: PanelWindow) {
        match self {
            Self::Commands {
                window: current, ..
            } => *current = window,
            Self::Model(picker) => picker.window = window,
            Self::History(panel) => panel.window = window,
        }
    }

    /// 键盘移动高亮；返回是否变化。
    pub fn move_active(&mut self, delta: isize) -> bool {
        let len = self.item_count();
        if len == 0 {
            return false;
        }
        let mut window = self.window();
        window.move_active(delta, len);
        self.set_window(window);
        true
    }

    /// 鼠标悬停设置高亮；越界、未变化或面板无候选时返回 false。
    pub fn set_active(&mut self, index: usize) -> bool {
        let len = self.item_count();
        if index >= len {
            return false;
        }
        let mut window = self.window();
        if window.active == index {
            return false;
        }
        window.set_active(index);
        self.set_window(window);
        true
    }

    /// 滚轮滚动窗口；`base` 为当前窗口起点，高亮不动。返回是否移动。
    pub fn scroll_viewport(&mut self, delta: isize, base: usize) -> bool {
        let len = self.item_count();
        if len == 0 {
            return false;
        }
        let mut window = self.window();
        window.scroll(delta, base.min(len - 1), len);
        let moved = window.viewport != self.window().viewport;
        self.set_window(window);
        moved
    }
}

/// 命令名模糊匹配打分（对齐 `@` 提及的层次：精确 > 前缀 > 子串 > 子序列）。
///
/// 返回 None 表示不匹配；分数越高越靠前。空查询给基础分（保持固定顺序）。
fn command_score(name: &str, query: &str) -> Option<u32> {
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
fn alias_prefix_score(alias: &str, query: &str) -> Option<u32> {
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
    // 行首必须是 `/`（无前导文本）。
    if !line_prefix.starts_with('/') {
        return None;
    }
    Some(line_prefix.to_string())
}
