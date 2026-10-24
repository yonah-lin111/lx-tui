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
    /// `/rewind`（别名 `/undo`）：撤销当前会话最新一轮。
    Rewind,
    /// `/compact`：把增量历史压缩成可撤销的检查点。
    Compact,
}

impl SlashCommand {
    /// 全部命令（固定顺序）。
    pub const ALL: [SlashCommand; 5] = [
        SlashCommand::Model,
        SlashCommand::Resume,
        SlashCommand::New,
        SlashCommand::Rewind,
        SlashCommand::Compact,
    ];

    /// 命令名（不含 `/`）。
    pub fn name(self) -> &'static str {
        match self {
            Self::Model => "model",
            Self::Resume => "resume",
            Self::New => "new",
            Self::Rewind => "rewind",
            Self::Compact => "compact",
        }
    }

    /// 额外别名（不含 `/`）；用于命令面板的模糊匹配（`/session` 命中 `/resume`）。
    pub fn aliases(self) -> &'static [&'static str] {
        match self {
            Self::Model => &[],
            Self::Resume => &["session"],
            Self::New => &["clear"],
            Self::Rewind => &["undo"],
            Self::Compact => &["summarize"],
        }
    }

    /// 命令文本（含 `/`）。
    pub fn text(self) -> &'static str {
        match self {
            Self::Model => "/model",
            Self::Resume => "/resume",
            Self::New => "/new",
            Self::Rewind => "/rewind",
            Self::Compact => "/compact",
        }
    }

    /// 一句话描述。
    pub fn description(self) -> &'static str {
        match self {
            Self::Model => "Choose the model and reasoning level for the next turn",
            Self::Resume => crate::ui::text::LX_RESUME_COMMAND_DESC,
            Self::New => "Start a fresh blank session",
            Self::Rewind => "Undo the latest turn (files and conversation)",
            Self::Compact => crate::ui::text::LX_COMPACT_COMMAND_DESC,
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
    /// 当前生效的模型在 `candidates` 中的索引（行首 `❯` 与底色标记）；
    /// 未绑定或不在候选列表时为 None。
    pub current: Option<usize>,
    /// 当前生效的 variant 在第二级候选中的索引（进入 `Variants` 时按阶段计算）。
    pub current_variant: Option<usize>,
    /// 窗口状态（高亮/锚点/显式视口）。
    pub window: PanelWindow,
    /// 过滤词（输入框全文快照）；空表示不过滤。
    pub filter: String,
    /// 过滤后的候选索引视图（按分数排序）；空过滤时为恒等序列。
    ///
    /// 只按当前 `stage` 解释：`Models` 指向 `candidates`，`Variants` 指向其本级的
    /// `variants`。原始列表全量保留，因此退格删字符能恢复。
    pub filtered: Vec<usize>,
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

/// 超过该时长改用绝对日期（对齐 opencode 的"近期相对、久远绝对"）。
const RELATIVE_TIME_LIMIT_MS: i64 = 7 * 24 * 60 * 60 * 1000;

impl HistoryEntry {
    /// 右侧时间列：7 天内相对时间（`just now` / `5m ago` / `3h ago` / `2d ago`），
    /// 更早显示 `YYYY-MM-DD`。`now_ms` 由渲染层传入（`ui/` 不自己取系统时间）。
    ///
    /// 未来时间戳（时钟回拨）按 `just now` 处理，不显示负数。
    pub fn time_label(&self, now_ms: i64) -> String {
        let elapsed = now_ms.saturating_sub(self.updated_at_ms);
        if elapsed < 0 {
            return "just now".to_string();
        }
        if elapsed < 60_000 {
            return "just now".to_string();
        }
        if elapsed < 60 * 60 * 1000 {
            return format!("{}m ago", elapsed / 60_000);
        }
        if elapsed < 24 * 60 * 60 * 1000 {
            return format!("{}h ago", elapsed / (60 * 60 * 1000));
        }
        if elapsed < RELATIVE_TIME_LIMIT_MS {
            return format!("{}d ago", elapsed / (24 * 60 * 60 * 1000));
        }
        format_date(self.updated_at_ms)
    }
}

/// 由 UTC 毫秒时间戳格式化为 `YYYY-MM-DD`（不引入时区依赖）。
fn format_date(ms: i64) -> String {
    let days = ms.div_euclid(86_400_000);
    let (year, month, day) = civil_from_days(days);
    format!("{year:04}-{month:02}-{day:02}")
}

/// 自 1970-01-01 起的天数 → 公历年月日（Howard Hinnant 的 civil_from_days）。
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let day_of_era = z.rem_euclid(146_097);
    let year_of_era =
        (day_of_era - day_of_era / 1460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let year = year_of_era + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let mp = (5 * day_of_year + 2) / 153;
    let day = u32::try_from(day_of_year - (153 * mp + 2) / 5 + 1).unwrap_or(1);
    let month = u32::try_from(if mp < 10 { mp + 3 } else { mp - 9 }).unwrap_or(1);
    (if month <= 2 { year + 1 } else { year }, month, day)
}

/// `/resume` 历史面板。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct HistoryPanel {
    pub entries: Vec<HistoryEntry>,
    /// 当前窗格已绑定的会话在 `entries` 中的索引（行首 `❯` 与底色标记）。
    pub current: Option<usize>,
    /// 计算相对时间的"现在"（UTC 毫秒），由事件循环每轮 `tick` 刷新。
    ///
    /// 渲染层只读它做算术，不自己取系统时间：面板停留期间 `just now` 会自然走成
    /// `1m ago`，而 `ui/` 保持纯函数可测。
    pub now_ms: i64,
    /// 窗口状态（高亮/锚点/显式视口）。
    pub window: PanelWindow,
    /// 过滤词（输入框全文快照）；空表示不过滤。
    pub filter: String,
    /// 过滤后的条目索引视图（按分数排序）；空过滤时为恒等序列。
    ///
    /// `entries` 全量保留，因此退格删字符能恢复原列表。
    pub filtered: Vec<usize>,
}

impl HistoryPanel {
    /// 按 `query` 重算 `filtered`：匹配**标题或会话 id**，按分数降序、同分保持原序。
    ///
    /// 空查询（或全空白）恢复恒等视图（全部条目、原顺序）。时间列不参与匹配——
    /// 它在变，参与只会让人困惑。
    pub fn refilter(&mut self, query: &str) {
        self.filter = query.to_string();
        if query.trim().is_empty() {
            self.filtered = (0..self.entries.len()).collect();
            self.clamp_window();
            return;
        }
        let query = query.trim().to_lowercase();
        let mut scored: Vec<(u32, usize)> = self
            .entries
            .iter()
            .enumerate()
            .filter_map(|(index, entry)| {
                best_score(&[&entry.title, &entry.session_id], &query).map(|score| (score, index))
            })
            .collect();
        scored.sort_by(|left, right| right.0.cmp(&left.0).then_with(|| left.1.cmp(&right.1)));
        self.filtered = scored.into_iter().map(|(_, index)| index).collect();
        self.clamp_window();
    }

    /// 过滤后可见的条目（按视图顺序）；供渲染与命中共用。
    pub fn visible(&self) -> Vec<&HistoryEntry> {
        self.filtered
            .iter()
            .filter_map(|index| self.entries.get(*index))
            .collect()
    }

    /// 高亮项在 `entries` 中的真实索引（供采纳/复制/编辑读取数据）。
    pub fn active_entry(&self) -> Option<&HistoryEntry> {
        self.filtered
            .get(self.window.active)
            .and_then(|index| self.entries.get(*index))
    }

    /// 高亮项对应的 `entries` 索引（供 `current` 标记换算）。
    pub fn active_entry_index(&self) -> Option<usize> {
        self.filtered.get(self.window.active).copied()
    }

    /// 可见项数变化后把窗口三索引钳进新范围。
    pub fn clamp_window(&mut self) {
        self.window.clamp(self.filtered.len());
    }

    /// 过滤为空（有过滤词但无命中）——与「真的没有会话」不同，渲染要区分提示。
    pub fn is_filter_empty(&self) -> bool {
        !self.filter.trim().is_empty() && self.filtered.is_empty()
    }
}

impl ModelPicker {
    /// 按 `query` 重算 `filtered`：只在**第一级**（模型列表）生效，匹配
    /// **模型名或 provider 名**。
    ///
    /// 第二级（思考等级 variants）**不支持过滤**：候选通常只有 `low`/`high`/`max`
    /// 两三档，搜索没有意义；且第一级带来的过滤词（如 `gpt`）对等级名必然无命中，
    /// 只会制造 `no matches`。因此 `Variants` 阶段恒为全量视图，`filter` 也保持空。
    pub fn refilter(&mut self, query: &str) {
        if matches!(self.stage, ModelPickerStage::Variants { .. }) {
            self.filter.clear();
            self.filtered = (0..self.stage_len()).collect();
            self.clamp_window();
            return;
        }
        self.filter = query.to_string();
        if query.trim().is_empty() {
            self.filtered = (0..self.stage_len()).collect();
            self.clamp_window();
            return;
        }
        let query = query.trim().to_lowercase();
        let mut scored: Vec<(u32, usize)> = self
            .candidates
            .iter()
            .enumerate()
            .filter_map(|(index, candidate)| {
                best_score(&[&candidate.model_name, &candidate.provider_name], &query)
                    .map(|score| (score, index))
            })
            .collect();
        scored.sort_by(|left, right| right.0.cmp(&left.0).then_with(|| left.1.cmp(&right.1)));
        self.filtered = scored.into_iter().map(|(_, index)| index).collect();
        self.clamp_window();
    }

    /// 当前阶段原始候选数（`Models` 为 candidates，`Variants` 为其 variants）。
    pub fn stage_len(&self) -> usize {
        match &self.stage {
            ModelPickerStage::Models => self.candidates.len(),
            ModelPickerStage::Variants { variants, .. } => variants.len(),
        }
    }

    /// 高亮项在当前阶段的原始索引（供采纳读取数据）。
    pub fn active_stage_index(&self) -> Option<usize> {
        self.filtered.get(self.window.active).copied()
    }

    /// 高亮项（第一级）候选。
    pub fn active_candidate(&self) -> Option<&ModelCandidate> {
        self.active_stage_index()
            .and_then(|index| self.candidates.get(index))
    }

    /// 可见项数变化后把窗口三索引钳进新范围。
    pub fn clamp_window(&mut self) {
        self.window.clamp(self.filtered.len());
    }

    /// 过滤为空（有过滤词但无命中）。
    pub fn is_filter_empty(&self) -> bool {
        !self.filter.trim().is_empty() && self.filtered.is_empty()
    }
}

/// 多个候选字段取最高分（任一命中即保留）。
fn best_score(targets: &[&str], query: &str) -> Option<u32> {
    targets
        .iter()
        .filter_map(|target| command_score(&target.to_lowercase(), query))
        .max()
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
    /// `/rewind` 四项动作面板。
    Rewind(Box<crate::app::state::lx::rewind::RewindPanel>),
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

    /// 当前视图的候选数量（过滤后可见项数；渲染与导航共用同一口径）。
    pub fn item_count(&self) -> usize {
        match self {
            Self::Commands { items, .. } => items.len(),
            Self::Model(picker) => picker.filtered.len(),
            Self::History(panel) => panel.filtered.len(),
            // 动作集按目标类型不同（普通轮次四项 / compact 两项），以面板为准。
            Self::Rewind(panel) => panel.len(),
        }
    }

    /// 当前视图的窗口状态。
    pub fn window(&self) -> PanelWindow {
        match self {
            Self::Commands { window, .. } => *window,
            Self::Model(picker) => picker.window,
            Self::History(panel) => panel.window,
            Self::Rewind(panel) => panel.window,
        }
    }

    /// 当前视图"已生效项"的索引（行首 `❯` 标记与底色）；命令候选面板没有该概念。
    ///
    /// 返回的是**视图位置**（过滤后），由真实索引换算而来：过滤把已生效项过滤掉时
    /// 自然返回 None（不画标记），不会把标记错贴到别的行上。
    pub fn current_index(&self) -> Option<usize> {
        match self {
            Self::Commands { .. } => None,
            Self::Model(picker) => {
                let real = match &picker.stage {
                    ModelPickerStage::Models => picker.current,
                    ModelPickerStage::Variants { .. } => picker.current_variant,
                }?;
                picker.filtered.iter().position(|index| *index == real)
            }
            Self::History(panel) => {
                let real = panel.current?;
                panel.filtered.iter().position(|index| *index == real)
            }
            Self::Rewind(_) => None,
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
            Self::Rewind(panel) => panel.window = window,
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

#[cfg(test)]
mod tests;
