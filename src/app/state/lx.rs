//! lx 页状态与多行文本输入编辑模型。

use crate::app::markdown::BlockPanel;
use crate::app::paste::{
    ExpandedPastes, PastePanel, TextPastePanel, clamp_cursor, snap_pasted_range,
};
#[cfg(test)]
use crate::app::paste::{PasteAction, PasteCandidate};
use crate::app::prompt::text;
use crate::app::selection::SelectionUnit;

pub mod agent;
pub mod ask;
pub mod chat;
pub mod compact;
mod edit;
mod folded;
mod history;
pub mod mention;
pub mod model_switch;
mod panels;
pub mod refer;
pub mod rewind;
mod selection;
pub mod shell;
pub mod slash;
pub mod task;

use history::HistoryBrowse;
pub(crate) use history::HistoryMove;
pub use shell::LxInputMode;

pub use agent::{AgentsState, SubagentReportView, SubagentRun};
pub use ask::{AskEditor, AskHit, AskState, AskWheelZone};
pub use chat::SHELL_SLOT;
pub use chat::{
    AskAnsweredView, AskDetailView, AssistantStepView, CardSection, CardTarget, ChatMessageView,
    ChatPartView, CommandView, DiffHunkView, DiffLineView, FileChangeView, FileDiffDetailView,
    FileDiffView, FrozenModel, InputHintLevel, InteractionAction, LxChat, McpDetailView,
    McpImageView, ModelsSnapshot, PendingShell, PlanSnapshotView, PlanStepView,
    ProcessInteractionView, ProcessStateView, QueuedSubmission, RequestKey, RequestState,
    SearchDetailView, SearchMatchView, SubagentDetailView, ToolCallView, ToolDetailView,
    ToolStatus, ToolSummary, UserShellView, WebSearchDetailView,
};
pub use compact::{CompactNoticeView, LxCompact};
pub use mention::{LxMention, LxMentionView};
pub use model_switch::{LxModelSwitch, ModelSwitchNoticeView};
pub use rewind::{
    LxRewind, RewindAction, RewindExecution, RewindNoticeView, RewindPanel, RewindTarget,
    RewindTargetKind, SnapshotState,
};
pub use slash::{
    ForkEntry, ForkPanel, HistoryEntry, HistoryPanel, ModelCandidate, ModelPicker,
    ModelPickerStage, PanelWindow, SlashCommand, SlashEntry, SlashPanel,
};
pub use task::{ProcessEntry, TaskPage, TaskTab};

/// lx 页三个入口：原生 agent、混合 agent、自动化 agent。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LxEntry {
    Native,
    Hybrid,
    Automation,
}

impl LxEntry {
    /// 三个入口的显示顺序。
    pub const ALL: [LxEntry; 3] = [LxEntry::Native, LxEntry::Hybrid, LxEntry::Automation];
}

/// 首页资源计数的一类（MCP server / skill）。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct StartupCount {
    /// 配置中的总数；0 表示未配置。
    pub total: usize,
    /// 已就绪数量。
    pub ready: usize,
    /// 失败或需要登录的数量。
    pub errors: usize,
    /// 首次启动/重载是否仍在进行。
    pub loading: bool,
}

/// 首页展示的 AGENTS.md、MCP 与 skill 启动计数。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct StartupCounts {
    /// 当前工作区生效（非空）的全局 + 项目级 AGENTS.md 文件数量。
    pub agents_md: usize,
    pub mcp: StartupCount,
    pub skills: StartupCount,
}

/// lx 页交互状态：选中入口、多行输入缓冲与 markdown 块命令面板；每个 lx 窗格各持一份。
#[derive(Debug)]
pub struct Lx {
    /// 当前选中入口；仅鼠标点击切换，重启复位为 Native。
    pub entry: LxEntry,
    /// 多行输入内容；Shell 模式下只存命令正文（不含 `!` 前缀）。
    pub input: String,
    /// 当前编辑模式；仅首个非空白字符决定，正文中的 `！` 不触发。
    input_mode: LxInputMode,
    /// 输入内容修订号：任何正文/模式改动单调递增，undo/redo 也不回退。
    revision: u64,
    /// 光标字节偏移；恒位于字符边界。
    pub cursor: usize,
    /// 输入框内容宽度（列）；按几何同步，用于软换行与视觉行移动。未同步前视为不换行。
    width: u16,
    /// 输入框可视高度（内容行）；按几何同步，用于滚动钳制。
    height: usize,
    /// 视口首视觉行索引；滚轮与光标跟随共同维护。
    scroll: usize,
    /// 输入框文本选择原点区间与粒度 `(origin_start, origin_end, unit)`。
    text_selection: Option<(usize, usize, SelectionUnit)>,
    /// markdown 块命令面板；光标处触发时打开。
    pub panel: Option<BlockPanel>,
    /// 路径粘贴参照面板；与 `panel` 互斥，同一时刻至多一个打开。
    paste: Option<LxPaste>,
    /// 长文本粘贴面板；与 `paste` / `panel` 互斥。
    text_paste: Option<LxTextPaste>,
    /// 已展开的折叠粘贴芯片集合（纯视图状态，不落库、不入撤销栈）。
    expanded_pastes: ExpandedPastes,
    /// 展开芯片预览区内的选区（投影文本字节起止）。
    preview_selection: Option<(usize, usize)>,
    /// 正在按住/拖拽的展开芯片 `(id, occurrence, display_range)`。
    preview_drag: Option<(u64, usize, std::ops::Range<usize>)>,
    /// `@` 文件提及面板：扫描根、缓存与候选；与块面板/粘贴面板互斥。
    mention: LxMention,
    /// 字母快捷输入面板（由正文已出现的引用与会话历史生成候选）。
    pub refer: Option<crate::app::markdown::ReferPanel>,
    /// 快捷面板压制标记：Esc 关闭或确认插入后，等待下一次编辑/移动再重算。
    pub refer_suppressed: bool,
    /// `/` 命令面板（model/resume/new）；与块面板/粘贴/提及面板互斥。
    pub slash: Option<super::lx::SlashPanel>,
    /// 聊天会话状态：会话引用、历史视图与请求状态机。
    pub chat: LxChat,
    /// 本窗格派发的子 agent 展示态（面板关闭时依然保留）。
    pub agents: AgentsState,
    /// 最新轮次撤销：待命、目标选择、动作面板与执行中。
    pub rewind: LxRewind,
    /// 上下文压缩：检查点、展开状态与执行中标记（只读视图，IO 在事件层）。
    pub compact: LxCompact,
    /// 会话中途模型切换记录与展开状态。
    pub model_switch: LxModelSwitch,
    /// 本页自己的 toast 槽；`Some` 时覆盖绘制在**窗格内容区内部**。
    pub toast: Option<crate::app::toast::Toast>,
    /// 后台任务页面；`Some` 表示当前窗格正停在该页。
    pub task: Option<TaskPage>,
    /// 当前挂起的 ask 面板；`None` 表示输入框正常显示。
    pub ask: Option<AskState>,
    /// 纯文本缓冲：不识别 shell 前缀、不弹 markdown/提及/斜杠面板。
    plain: bool,
    /// 本会话正在运行的后台任务数（底部状态行指标）。
    pub background_tasks: usize,
    /// 首页资源计数：MCP server 与 skill 的加载/失败状态。
    pub startup: StartupCounts,
    /// 撤销历史（上限 `LX_HISTORY_LIMIT`）。
    undo: Vec<LxSnapshot>,
    /// 重做历史。
    redo: Vec<LxSnapshot>,
    /// 已发送 prompt 的历史浏览态（游标 + 进入浏览前的草稿）。
    history: HistoryBrowse,
}

/// 软换行后的一个视觉行：投影显示文本中的字节区间 `[start, end)`。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LxRow {
    pub start: usize,
    pub end: usize,
}

/// 在途路径粘贴：参照面板与占位插入现场。
#[derive(Debug)]
struct LxPaste {
    panel: PastePanel,
    /// 占位插入前的重做栈；`snapshot` 会清空它，取消必须原样还回。
    redo: Vec<LxSnapshot>,
}

/// 在途长文本粘贴：选择面板（确认前不改动输入缓冲区）。
#[derive(Debug)]
struct LxTextPaste {
    panel: TextPastePanel,
}

/// lx 输入缓冲的一份快照（文本 + 光标 + 模式），供撤销/重做。
#[derive(Debug, Clone)]
struct LxSnapshot {
    text: String,
    cursor: usize,
    mode: LxInputMode,
}

/// 撤销历史上限。
const LX_HISTORY_LIMIT: usize = 200;

impl Default for Lx {
    fn default() -> Self {
        Self::new()
    }
}

impl Lx {
    pub fn new() -> Self {
        Self::build(false)
    }

    /// 纯文本缓冲：`!` 只是普通字符，也不触发任何面板（`ask` 手写答案用）。
    pub fn plain() -> Self {
        Self::build(true)
    }

    fn build(plain: bool) -> Self {
        Self {
            entry: LxEntry::Native,
            input: String::new(),
            input_mode: LxInputMode::Chat,
            revision: 0,
            cursor: 0,
            width: u16::MAX,
            height: usize::MAX,
            scroll: 0,
            text_selection: None,
            panel: None,
            paste: None,
            text_paste: None,
            expanded_pastes: ExpandedPastes::default(),
            preview_selection: None,
            preview_drag: None,
            mention: LxMention::default(),
            refer: None,
            refer_suppressed: false,
            slash: None,
            chat: LxChat::new(),
            agents: AgentsState::default(),
            rewind: LxRewind::default(),
            compact: LxCompact::default(),
            model_switch: LxModelSwitch::default(),
            toast: None,
            task: None,
            ask: None,
            plain,
            background_tasks: 0,
            startup: StartupCounts::default(),
            undo: Vec::new(),
            redo: Vec::new(),
            history: HistoryBrowse::default(),
        }
    }

    /// 清空子 agent 展示态（会话切换/新建时随会话一起复位）。
    pub fn clear_agents(&mut self) {
        self.agents = AgentsState::default();
    }

    /// 清除撤销状态，并关闭 `/rewind` 动作面板。
    pub fn clear_rewind(&mut self) {
        self.rewind.clear();
        if matches!(self.slash, Some(slash::SlashPanel::Rewind(_))) {
            self.slash = None;
        }
    }

    /// 同步输入框内容宽度（列）与可视高度（内容行）；用于软换行、视觉行移动与滚动。
    pub fn resize(&mut self, width: u16, height: u16) {
        self.width = width.max(1);
        self.height = usize::from(height.max(1));
        self.scroll_cursor_into_view();
    }

    /// 输入框内容宽度（列）；ask 面板同步手写编辑缓冲的软换行宽度用。
    pub fn input_width(&self) -> u16 {
        self.width
    }

    /// 是否有挂起的 `ask` 面板（输入分支、标签页标记与 sweep 共用）。
    pub fn has_pending_ask(&self) -> bool {
        self.ask.is_some()
    }

    /// 输入框视口首视觉行索引。
    pub fn scroll(&self) -> usize {
        self.scroll
    }

    /// 当前编辑模式。
    pub fn input_mode(&self) -> LxInputMode {
        self.input_mode
    }

    /// 输入内容修订号（单调递增；供 shell 草稿 ACK 比对）。
    pub fn revision(&self) -> u64 {
        self.revision
    }

    /// 完整输入文本：普通模式为正文，shell 模式为 `!` + 正文。
    pub fn serialized_input(&self) -> String {
        shell::serialize(self.input_mode, &self.input)
    }

    /// 是否处于 shell 模式。
    pub fn shell_mode(&self) -> bool {
        self.input_mode == LxInputMode::Shell
    }

    /// 退出 shell 模式（保留正文），返回是否确实退出。
    pub fn exit_shell(&mut self) -> bool {
        if self.input_mode != LxInputMode::Shell {
            return false;
        }
        self.snapshot();
        self.input_mode = LxInputMode::Chat;
        self.refresh_panel();
        true
    }

    /// 用完整文本替换草稿（历史召回、排队恢复共用同一模式识别）。
    pub fn import_full_text(&mut self, text: &str) {
        self.text_paste = None;
        self.clear_expanded_pastes();
        let (mode, body) = shell::split_mode(text);
        if self.input == body && self.input_mode == mode {
            self.cursor = self.input.len();
            self.text_selection = None;
            self.scroll_cursor_into_view();
            return;
        }
        self.snapshot();
        self.input = body;
        self.input_mode = mode;
        self.cursor = self.input.len();
        self.text_selection = None;
        self.scroll_cursor_into_view();
        self.refresh_panel();
    }

    /// 记录一次内容或模式改动（修订号只增不减）。
    pub(super) fn touch(&mut self) {
        self.revision = self.revision.saturating_add(1);
    }

    /// 新插入的文本若以首个非空白 `!`/`！` 开头，吸收该标记并进入 shell 模式。
    pub(super) fn adopt_shell_prefix(&mut self) {
        if self.plain || self.input_mode == LxInputMode::Shell {
            return;
        }
        let Some(offset) = shell::marker_offset(&self.input) else {
            return;
        };
        let Some(marker) = self.input[offset..].chars().next() else {
            return;
        };
        self.input.remove(offset);
        if self.cursor > offset {
            self.cursor = self.cursor.saturating_sub(marker.len_utf8());
        }
        self.input_mode = LxInputMode::Shell;
        self.panel = None;
        self.paste = None;
        self.text_paste = None;
        self.slash = None;
        self.mention.suppress();
        self.text_selection = None;
        self.touch();
    }

    /// 滚轮滚动输入框视口（正数向下）；返回视口是否移动。
    pub fn scroll_by(&mut self, delta: isize) -> bool {
        let max = self.max_scroll();
        let next = (self.scroll.min(max) as isize)
            .saturating_add(delta)
            .clamp(0, max as isize) as usize;
        let moved = next != self.scroll;
        self.scroll = next;
        moved
    }

    /// 真实内容的最大滚动偏移。
    pub(super) fn max_scroll(&self) -> usize {
        self.visual_rows().len().saturating_sub(self.height.max(1))
    }

    /// 调整滚动量，保证光标所在视觉行可见。
    pub(super) fn scroll_cursor_into_view(&mut self) {
        let (row, _) = self.cursor_visual();
        let height = self.height.max(1);
        if row < self.scroll {
            self.scroll = row;
        } else if row >= self.scroll.saturating_add(height) {
            self.scroll = row + 1 - height;
        }
        self.scroll = self.scroll.min(self.max_scroll());
    }

    /// 直接设置视口滚动偏移（滚动条点击/拖拽）；越界钳到可滚动范围。
    pub fn scroll_to(&mut self, offset: usize) {
        self.scroll = offset.min(self.max_scroll());
    }

    /// 在指定视觉行/内容列开始输入框文本选择：光标落到该单元格并登记锚点。
    pub fn begin_text_selection(&mut self, row: usize, col: u16) {
        self.begin_text_selection_with_clicks(row, col, 1);
    }

    /// 按连续点击次数（1=单字、2=选词、3=选逻辑行）在指定视觉行/内容列开始输入框文本选择。
    pub fn begin_text_selection_with_clicks(&mut self, row: usize, col: u16, clicks: u8) {
        self.preview_selection = None;
        self.preview_drag = None;
        let offset = self.offset_at_cell(row, col);
        let unit = SelectionUnit::from_clicks(clicks);
        let (start, end) = unit.byte_range(&self.input, offset);
        let snapped = snap_pasted_range(&self.input, start..end);
        self.cursor = snapped.end;
        self.text_selection = Some((snapped.start, snapped.end, unit));
        self.panel = None;
        self.mention.suppress();
        self.scroll_cursor_into_view();
    }

    /// 拖拽扩展输入框选区：按当前选区粒度（单字/词/逻辑行）延伸光标、保留初始原点区间。
    pub fn drag_text_selection(&mut self, row: usize, col: u16) {
        let Some((origin_start, _origin_end, unit)) = self.text_selection else {
            return;
        };
        let proj = self.projection();
        let disp = self.display_offset_at_cell(row, col, false);
        let offset = if let Some(hit) = proj.chip_at(disp) {
            if hit.raw_range.start >= origin_start {
                hit.raw_range.end
            } else {
                hit.raw_range.start
            }
        } else {
            proj.display_to_raw(disp)
        };
        let (start, end) = unit.byte_range(&self.input, offset);
        let snapped = snap_pasted_range(&self.input, start..end);
        self.cursor = if snapped.start < origin_start {
            snapped.start
        } else {
            snapped.end
        };
        self.scroll_cursor_into_view();
    }

    /// 清除输入框文本选区。
    pub fn clear_text_selection(&mut self) {
        self.text_selection = None;
    }

    /// 是否存在输入框文本选区锚点。
    pub fn has_text_selection(&self) -> bool {
        self.text_selection.is_some()
    }

    /// 输入框选区字节范围 `[start, end)`；空选区返回 None。
    pub fn text_selection_range(&self) -> Option<(usize, usize)> {
        let (origin_start, origin_end, _) = self.text_selection?;
        let start = origin_start.min(self.cursor);
        let end = origin_end.max(self.cursor);
        let snapped = snap_pasted_range(&self.input, start..end);
        (snapped.start < snapped.end).then_some((snapped.start, snapped.end))
    }

    /// 取输入框选中文本；空选区返回 None。
    pub fn selected_text(&self) -> Option<String> {
        self.text_selection_range()
            .map(|(start, end)| self.input[start..end].to_string())
    }

    /// 视口单元格（视觉行、内容列）→ 输入缓冲原始字节偏移；越界钳到最近位置。
    pub fn offset_at_cell(&self, row: usize, col: u16) -> usize {
        let disp = self.display_offset_at_cell(row, col, false);
        let raw = self.projection().display_to_raw(disp);
        clamp_cursor(&self.input, raw)
    }

    /// 按内容宽度软换行得到的视觉行（基于折叠/展开投影文本）。
    pub fn visual_rows(&self) -> Vec<LxRow> {
        let width = usize::from(self.width.max(1));
        let proj = self.projection();
        let proj_text = proj.text();
        let mut rows = Vec::new();
        let mut line_start = 0;
        for segment in proj_text.split('\n') {
            let line_end = line_start + segment.len();
            let mut row_start = line_start;
            let mut col = 0;
            for (offset, ch) in segment.char_indices() {
                let absolute = line_start + offset;
                let cell = text::char_width(ch);
                if col > 0 && col + cell > width {
                    rows.push(LxRow {
                        start: row_start,
                        end: absolute,
                    });
                    row_start = absolute;
                    col = 0;
                }
                col += cell;
            }
            rows.push(LxRow {
                start: row_start,
                end: line_end,
            });
            line_start = line_end + 1;
        }
        rows
    }

    /// 光标在视觉行中的索引与显示列（跨折行边界归入下一视觉行）。
    pub fn cursor_visual(&self) -> (usize, usize) {
        let rows = self.visual_rows();
        self.cursor_visual_in(&rows)
    }

    pub(super) fn cursor_visual_in(&self, rows: &[LxRow]) -> (usize, usize) {
        let proj = self.projection();
        let disp_cursor = proj.raw_to_display(self.cursor);
        let disp_text = proj.text();
        for (index, row) in rows.iter().enumerate() {
            if disp_cursor < row.end {
                return (
                    index,
                    text::display_width(&disp_text[row.start..disp_cursor]),
                );
            }
            if disp_cursor == row.end {
                let continues = rows
                    .get(index + 1)
                    .is_some_and(|next| next.start == row.end);
                if !continues {
                    return (index, text::display_width(&disp_text[row.start..row.end]));
                }
            }
        }
        let index = rows.len().saturating_sub(1);
        let row = rows[index];
        (index, text::display_width(&disp_text[row.start..row.end]))
    }

    /// 是否正在浏览已发送 prompt 的历史。
    pub(crate) fn history_browsing(&self) -> bool {
        self.history.browsing()
    }

    /// 结束历史浏览（不改输入框文本）。
    pub(crate) fn end_history_browse(&mut self) {
        self.history.end();
    }

    /// 只读判定：这次方向键是否由历史接管，以及要显示哪一条。
    pub(crate) fn pending_history_move(
        &self,
        entries: &[String],
        direction: i8,
    ) -> Option<HistoryMove> {
        history::decide(&self.history, entries, direction, &self.input, self.cursor)
    }

    /// 应用一次历史切换：更新游标并按 [`Self::apply_history_text`] 写回文本。
    pub(crate) fn apply_history(&mut self, next: HistoryMove) {
        match next {
            HistoryMove::Entry {
                index,
                text,
                cursor_at_start,
            } => {
                if !self.history.browsing() {
                    self.history.begin(self.serialized_input());
                }
                self.history.set_index(index);
                self.apply_history_text(&text);
                if cursor_at_start {
                    self.cursor = 0;
                }
                self.scroll_cursor_into_view();
            }
            HistoryMove::Draft => {
                let draft = self.history.take_draft();
                self.apply_history_text(&draft);
            }
        }
    }

    /// 一次性替换输入正文（撤销回填复用）：只压一次本地编辑撤销、光标置末尾，
    /// 并关闭块面板、提及与粘贴面板。
    pub fn replace_input(&mut self, text: &str) {
        self.snapshot();
        let (mode, body) = shell::split_mode(text);
        self.input = body;
        self.input_mode = mode;
        self.cursor = self.input.len();
        self.text_selection = None;
        self.panel = None;
        self.paste = None;
        self.text_paste = None;
        self.clear_expanded_pastes();
        self.mention.suppress();
        self.end_history_browse();
        self.scroll_cursor_into_view();
        self.refresh_panel();
    }

    /// 把历史文本写回输入框：吸收首部 `!`/`！` 恢复模式，压一次撤销快照、光标置末尾。
    fn apply_history_text(&mut self, text: &str) {
        self.text_paste = None;
        self.clear_expanded_pastes();
        let (mode, body) = shell::split_mode(text);
        if self.input == body && self.input_mode == mode {
            self.cursor = self.input.len();
            self.text_selection = None;
            self.scroll_cursor_into_view();
            return;
        }
        self.snapshot();
        self.input = body;
        self.input_mode = mode;
        self.cursor = self.input.len();
        self.text_selection = None;
        self.scroll_cursor_into_view();
        self.refresh_panel();
    }
}

#[cfg(test)]
mod tests;
