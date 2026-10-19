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
    /// `/stop`：列出本会话的后台进程，进入二级面板逐个/全部停止。
    Stop,
    /// `/new`：清当前窗格会话关联并回空白草稿。
    New,
}

impl SlashCommand {
    /// 全部命令（固定顺序）。
    pub const ALL: [SlashCommand; 4] = [
        SlashCommand::Model,
        SlashCommand::Resume,
        SlashCommand::Stop,
        SlashCommand::New,
    ];

    /// 命令名（不含 `/`）。
    pub fn name(self) -> &'static str {
        match self {
            Self::Model => "model",
            Self::Resume => "resume",
            Self::Stop => "stop",
            Self::New => "new",
        }
    }

    /// 额外别名（不含 `/`）；用于命令面板的模糊匹配（`/session` 命中 `/resume`）。
    pub fn aliases(self) -> &'static [&'static str] {
        match self {
            Self::Model => &[],
            Self::Resume => &["session"],
            // `kill` / `ps` 都是用户会先想到的词（对齐 codex 的 `/stop` 与 `/ps`）。
            Self::Stop => &["kill", "ps"],
            Self::New => &["clear"],
        }
    }

    /// 命令文本（含 `/`）。
    pub fn text(self) -> &'static str {
        match self {
            Self::Model => "/model",
            Self::Resume => "/resume",
            Self::Stop => "/stop",
            Self::New => "/new",
        }
    }

    /// 一句话描述。
    pub fn description(self) -> &'static str {
        match self {
            Self::Model => "Choose the model and reasoning level for the next turn",
            Self::Resume => crate::ui::text::LX_RESUME_COMMAND_DESC,
            Self::Stop => crate::ui::text::LX_STOP_COMMAND_DESC,
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
    /// 当前生效的模型在 `candidates` 中的索引（行首 `❯` 与底色标记）；
    /// 未绑定或不在候选列表时为 None。
    pub current: Option<usize>,
    /// 当前生效的 variant 在第二级候选中的索引（进入 `Variants` 时按阶段计算）。
    pub current_variant: Option<usize>,
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
}

/// `/stop` 面板中的一个后台进程条目。
///
/// 只装**展示 + 定位**所需的数据：`session_id` 是模型见过的进程会话 ID（真正用于
/// 停止操作），其余字段供列表行显示。进程句柄不进 `AppState`（`app/` 不持有进程）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProcessEntry {
    /// 模型见到的进程会话 ID（停止操作的唯一键）。
    pub session_id: u64,
    /// 命令原文（单行化后的展示文本）。
    pub command: String,
    /// 该进程的原始工作目录。
    pub cwd: String,
    /// 是否使用 PTY。
    pub tty: bool,
    /// 运行中为 true（列表只收运行中的进程，留此字段备将来展示终态）。
    pub running: bool,
    /// 启动时刻（UTC 毫秒）：用于计算已运行时长。
    pub started_at_ms: i64,
}

impl ProcessEntry {
    /// 运行时长标签：`running 12s` / `running 4m 03s`。
    pub fn age_label(&self, now_ms: i64) -> String {
        if !self.running {
            return String::new();
        }
        let elapsed = now_ms.saturating_sub(self.started_at_ms).max(0);
        let seconds = elapsed / 1_000;
        let text = if seconds < 60 {
            format!("{seconds}s")
        } else if seconds < 3_600 {
            format!("{}m {:02}s", seconds / 60, seconds % 60)
        } else {
            format!("{}h {:02}m", seconds / 3_600, (seconds % 3_600) / 60)
        };
        crate::ui::text::agent::process_age_label(&text)
    }
}

/// `/stop` 二级面板对**某个进程**可执行的动作。
///
/// 与 `/model` 的 `Variants` 同构：第一级选进程，第二级选动作。动作只有停止一个，
/// 但保留两级结构是为了让「先看清是哪个进程、再确认停掉」这件事有个确认步骤——
/// 停掉正在跑的服务是不可撤销的。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StopAction {
    /// 停止这一个进程。
    StopProcess,
}

impl StopAction {
    /// 全部动作（固定顺序）。
    pub const ALL: [StopAction; 1] = [StopAction::StopProcess];

    /// 动作标签。
    pub fn label(self) -> &'static str {
        match self {
            Self::StopProcess => crate::ui::text::LX_STOP_ACTION_KILL,
        }
    }
}

/// `/stop` 面板的两级视图。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum StopPickerStage {
    /// 第一级：本会话的后台进程列表。
    #[default]
    Processes,
    /// 第二级：所选进程的动作列表。
    Actions {
        /// 目标进程在 `Processes` 里的索引（Esc 返回时恢复高亮）。
        index: usize,
        /// 该进程的会话 ID（停哪个）。
        session_id: u64,
        /// 该进程的命令（二级面板标题里回显，确认停的是哪一个）。
        command: String,
    },
}

/// `/stop` 面板：先列进程，再选动作。
///
/// 与 `/model` 完全同构的两级面板（↑↓ 移动、Enter 进入下一级、Esc 返回上一级、
/// 顶层 Esc 关闭），差别只在数据来源与「第二级采纳后执行的是停止而不是写配置」。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct StopPicker {
    pub stage: StopPickerStage,
    /// 本会话当前运行中的进程（事件层按需加载）。
    pub processes: Vec<ProcessEntry>,
    /// 计算运行时长的"现在"（UTC 毫秒），由事件循环每轮刷新。
    pub now_ms: i64,
    /// 一级列表的窗口状态。
    pub window: PanelWindow,
    /// 二级动作列表的窗口状态。
    pub action_window: PanelWindow,
}

impl StopPicker {
    /// 当前级要高亮的条目数。
    pub fn stage_len(&self) -> usize {
        match self.stage {
            StopPickerStage::Processes => self.processes.len(),
            StopPickerStage::Actions { .. } => StopAction::ALL.len(),
        }
    }

    /// 当前级的窗口（一级/二级各有一份，互不串位置）。
    pub fn stage_window(&self) -> PanelWindow {
        match self.stage {
            StopPickerStage::Processes => self.window,
            StopPickerStage::Actions { .. } => self.action_window,
        }
    }

    /// 写回当前级的窗口。
    pub fn set_stage_window(&mut self, window: PanelWindow) {
        match self.stage {
            StopPickerStage::Processes => self.window = window,
            StopPickerStage::Actions { .. } => self.action_window = window,
        }
    }

    /// 选中的进程（一级）。
    pub fn selected(&self) -> Option<&ProcessEntry> {
        self.processes.get(self.window.active)
    }

    /// 进入二级：记录目标进程并重置动作窗口。
    pub fn enter_actions(&mut self) -> bool {
        let Some(entry) = self.processes.get(self.window.active).cloned() else {
            return false;
        };
        self.stage = StopPickerStage::Actions {
            index: self.window.active,
            session_id: entry.session_id,
            command: entry.command,
        };
        self.action_window = PanelWindow::new(0);
        true
    }

    /// 返回一级（Esc）：恢复进入前的进程高亮。
    pub fn back_to_processes(&mut self) -> bool {
        let StopPickerStage::Actions { index, .. } = self.stage else {
            return false;
        };
        self.stage = StopPickerStage::Processes;
        self.window = PanelWindow::new(index);
        true
    }

    /// 二级当前选中的动作。
    pub fn selected_action(&self) -> Option<StopAction> {
        let StopPickerStage::Actions { .. } = self.stage else {
            return None;
        };
        StopAction::ALL.get(self.action_window.active).copied()
    }

    /// 二级采纳后的目标进程 ID。
    pub fn target_session_id(&self) -> Option<u64> {
        match &self.stage {
            StopPickerStage::Processes => None,
            StopPickerStage::Actions { session_id, .. } => Some(*session_id),
        }
    }
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
    /// `/stop` 两级选择（进程 → 动作）。
    Stop(Box<StopPicker>),
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
            Self::Stop(picker) => picker.stage_len(),
        }
    }

    /// 当前视图的窗口状态。
    pub fn window(&self) -> PanelWindow {
        match self {
            Self::Commands { window, .. } => *window,
            Self::Model(picker) => picker.window,
            Self::History(panel) => panel.window,
            Self::Stop(picker) => picker.stage_window(),
        }
    }

    /// 当前视图"已生效项"的索引（行首 `❯` 标记与底色）；命令候选面板没有该概念。
    pub fn current_index(&self) -> Option<usize> {
        match self {
            Self::Commands { .. } => None,
            Self::Model(picker) => match &picker.stage {
                ModelPickerStage::Models => picker.current,
                ModelPickerStage::Variants { .. } => picker.current_variant,
            },
            Self::History(panel) => panel.current,
            // `/stop` 没有"已生效项"：后台进程不是可长期选中的状态。
            Self::Stop(_) => None,
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
            Self::Stop(picker) => picker.set_stage_window(window),
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

#[cfg(test)]
mod tests;
