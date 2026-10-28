//! 浮层模型：右键菜单、重命名输入、关闭确认与 worktree 对话框；同一时刻最多存在一个浮层。

use std::path::PathBuf;

use crate::layout::PaneId;

use super::markdown::TemplateCounts;
use super::prompt::{PromptEditing, PromptPageId};
use super::trust::TrustWorkspace;

/// 浮层种类：按键路由与渲染按种类分派。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OverlayKind {
    Menu,
    Rename,
    ConfirmClose,
    ConfirmSwitchCwd,
    ConfirmSyncWorkspaceCwd,
    NewWorkspace,
    WorktreeOpen,
    TrustWorkspace,
    NewNode,
    SaveDraft,
    PageMenu,
    /// 引用文件夹目录弹窗（编辑视图工具栏文件夹按钮）。
    ReferencedFolderMenu,
    /// 引用文件夹路径输入弹窗。
    ReferencedFolderInput,
    /// 引用文件夹内容搜索弹窗。
    ReferencedFolderSearch,
    /// 引用文件夹删除确认浮层。
    ReferencedFolderDelete,
    /// `/resume` 会话编辑模态：改名与删除（`Shift+Enter` 打开）。
    ResumeSessionEdit,
}

/// 浮层：同一时刻最多一个，由 `Option` 保证。
#[derive(Debug)]
pub enum Overlay {
    Menu(Menu),
    Rename(Rename),
    ConfirmClose(ConfirmClose),
    ConfirmSwitchCwd(ConfirmSwitchCwd),
    ConfirmSyncWorkspaceCwd(ConfirmSyncWorkspaceCwd),
    NewWorkspace(NewWorkspace),
    WorktreeOpen(WorktreeOpen),
    TrustWorkspace(TrustWorkspace),
    NewNode(NewNode),
    SaveDraft(SaveDraft),
    PageMenu(PageMenu),
    ReferencedFolderMenu(ReferencedFolderMenu),
    ReferencedFolderInput(ReferencedFolderInput),
    ReferencedFolderSearch(ReferencedFolderSearch),
    ReferencedFolderDelete(ReferencedFolderDelete),
    /// `/resume` 会话编辑模态：改名与删除（`Shift+Enter` 打开）。
    ResumeSessionEdit(ResumeSessionEdit),
}

impl Overlay {
    /// 浮层种类。
    pub fn kind(&self) -> OverlayKind {
        match self {
            Self::Menu(_) => OverlayKind::Menu,
            Self::Rename(_) => OverlayKind::Rename,
            Self::ConfirmClose(_) => OverlayKind::ConfirmClose,
            Self::ConfirmSwitchCwd(_) => OverlayKind::ConfirmSwitchCwd,
            Self::ConfirmSyncWorkspaceCwd(_) => OverlayKind::ConfirmSyncWorkspaceCwd,
            Self::NewWorkspace(_) => OverlayKind::NewWorkspace,
            Self::WorktreeOpen(_) => OverlayKind::WorktreeOpen,
            Self::TrustWorkspace(_) => OverlayKind::TrustWorkspace,
            Self::NewNode(_) => OverlayKind::NewNode,
            Self::SaveDraft(_) => OverlayKind::SaveDraft,
            Self::PageMenu(_) => OverlayKind::PageMenu,
            Self::ReferencedFolderMenu(_) => OverlayKind::ReferencedFolderMenu,
            Self::ReferencedFolderInput(_) => OverlayKind::ReferencedFolderInput,
            Self::ReferencedFolderSearch(_) => OverlayKind::ReferencedFolderSearch,
            Self::ReferencedFolderDelete(_) => OverlayKind::ReferencedFolderDelete,
            Self::ResumeSessionEdit(_) => OverlayKind::ResumeSessionEdit,
        }
    }
}

/// `/resume` 会话编辑模态：目标会话、标题输入与删除确认态。
///
/// 归属键（工作区 cwd + 发起窗格）随打开时固定：键变化即作废，避免把 A 工作区的
/// 会话改到 B 工作区的库里。改名与删除都是异步落库，成功回执后才更新列表。
#[derive(Debug)]
pub struct ResumeSessionEdit {
    /// 发起窗格（回执按它路由回 `/resume` 面板）。
    pub pane: PaneId,
    /// 该会话所属工作区 cwd（打开时快照）。
    pub workspace: PathBuf,
    pub session_id: String,
    /// 标题输入（预填当前标题）。
    pub input: TextInput,
    /// 已进入删除确认态（再按 `^D`/`Enter` 才真删）。
    pub confirming_delete: bool,
    /// 模态内提示（如 `Session busy`）；显示在按钮行上方。
    pub hint: Option<String>,
}

/// 引用文件夹目录弹窗：锚点、行快照与高亮项。
///
/// 行快照在打开时按项目快照生成；弹窗为模态，行集合在打开期间不再变化。
/// 归属的 prompt 键与项目身份随菜单固定，避免切换工作区后误用旧结果。
#[derive(Debug)]
pub struct ReferencedFolderMenu {
    pub anchor: (u16, u16),
    /// 弹窗归属的 prompt 键（显示中工作区）。
    pub key: Option<PathBuf>,
    /// 空状态、`[add]` 或某条记录。
    pub rows: Vec<ReferencedFolderMenuRow>,
    pub selected: usize,
    /// 指针悬停行：仅铺悬停底色，不移动 `❯` 选中标记；`None` 表示指针不在行上。
    pub hover: Option<usize>,
    /// 滚轮/滚动条滚动后的显式窗口起点；键盘移动后清空，窗口重新跟随高亮。
    pub viewport: Option<usize>,
}

/// 引用文件夹目录弹窗行：空状态、数据库不可用、新建动作或一条记录。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReferencedFolderMenuRow {
    /// 无记录（可选新增）。
    Empty,
    /// 数据库不可用：无法加载/保存（不伪装成空表）。
    Unavailable,
    Add,
    Folder {
        id: i64,
        path: PathBuf,
    },
}

/// 引用文件夹路径输入弹窗：单行输入与校验状态。
#[derive(Debug)]
pub struct ReferencedFolderInput {
    /// 来源菜单的锚点；成功后重开菜单沿用同一锚点。
    pub anchor: (u16, u16),
    pub input: TextInput,
    /// 归属的 prompt 键（打开时的显示中工作区）；键变化时弹窗作废。
    pub key: Option<PathBuf>,
    /// 校验中（阻止重复提交）。
    pub validating: bool,
    /// 校验产生的错误提示；None 表示无错误。
    pub error: Option<&'static str>,
    /// 在途校验的请求代号。
    pub request: Option<u64>,
}

/// 引用文件夹内容搜索弹窗：搜索根、过滤输入、候选与请求代号。
#[derive(Debug)]
pub struct ReferencedFolderSearch {
    /// 来源菜单锚点；`Esc` 关闭时据此重开菜单。
    pub anchor: (u16, u16),
    /// 所属项目记录的 id（删除后弹窗须失效）。
    pub folder: i64,
    /// 归属的 prompt 键。
    pub key: Option<PathBuf>,
    /// 当前搜索范围根（进入子目录后收窄）。
    pub root: PathBuf,
    /// 初始根（`Ctrl+Z` 回退到此即停）。
    pub initial_root: PathBuf,
    /// 搜索范围栈（`Ctrl+Z` 逐级回退）。
    pub stack: Vec<PathBuf>,
    pub query: TextInput,
    /// 扫描到的全部候选（未筛选）。
    pub entries: Vec<crate::app::referenced::ReferencedEntry>,
    /// 待展示的候选（已按输入筛选）。
    pub items: Vec<crate::app::referenced::ReferencedEntry>,
    /// 扫描本身命中条目上限（结果被截断）。
    pub scan_truncated: bool,
    /// 展示候选命中展示上限（结果被截断）。
    pub truncated: bool,
    pub selected: usize,
    /// 滚轮滚动后的显式窗口起点；键盘移动/筛选后清空，窗口重新跟随高亮。
    pub viewport: Option<usize>,
    /// 后台扫描尚未返回。
    pub loading: bool,
    /// 扫描根不可访问。
    pub failed: bool,
    /// 在途扫描请求代号；结果代号不匹配即丢弃。
    pub request: Option<u64>,
}

/// 引用文件夹删除确认浮层：归属键与待删记录。
#[derive(Debug)]
pub struct ReferencedFolderDelete {
    pub key: Option<PathBuf>,
    pub id: i64,
    pub path: PathBuf,
}

/// 页码菜单：锚点、作用目标、行快照与高亮项。
///
/// 行快照在打开时结算当前页后生成；当前页行首 `❯` 标记（Prompt 列表选中语义），
/// 菜单打开期间为模态，行集合不会再变化。
#[derive(Debug)]
pub struct PageMenu {
    pub anchor: (u16, u16),
    pub workspace: usize,
    /// 作用目标：正式条目或临时提示词。
    pub target: PromptEditing,
    pub rows: Vec<PageMenuRow>,
    pub selected: usize,
}

/// 页码菜单行：新建动作、页行或删除动作。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PageMenuRow {
    NewPage,
    Page {
        page: PromptPageId,
        current: bool,
        /// 正文去空白后为空（渲染为占位名/空白页语义）。
        empty: bool,
        counts: TemplateCounts,
    },
    DeletePage {
        enabled: bool,
    },
}

/// 新建节点命名浮层：文件夹或提示词条目；名称输入与来源选择器。
#[derive(Debug)]
pub struct NewNode {
    pub kind: NewNodeKind,
    /// 名称（预填对应默认名，可编辑）。
    pub name: TextInput,
    /// 来自草稿保存选择器（仅文件夹）：取消或确认后恢复该选择器
    /// （确认后由服务刷新条目并选中新文件夹）。
    pub resume: Option<Box<SaveDraft>>,
}

/// 新建节点种类：新建文件夹或新建提示词条目。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NewNodeKind {
    /// 新建文件夹：`parent_id` 为父文件夹（`None` 为根层）。
    Folder { parent_id: Option<i64> },
    /// 新建提示词：`folder_id` 为所属文件夹（`None` 为根层）。
    Prompt { folder_id: Option<i64> },
}

/// 草稿保存选择器：工作区、提示词名称输入、候选目标与高亮项。
#[derive(Debug)]
pub struct SaveDraft {
    pub workspace: usize,
    /// 保存的提示词名称（预填默认名，可编辑）。
    pub name: TextInput,
    pub entries: Vec<SaveDraftEntry>,
    pub selected: usize,
}

/// 草稿保存目标项：根层、现有文件夹（含层级缩进）或追加动作行。
#[derive(Debug)]
pub enum SaveDraftEntry {
    Root,
    Folder {
        id: i64,
        depth: usize,
        name: String,
    },
    /// 动作行：点击/回车打开新建文件夹命名浮层。
    AddFolder,
}

/// 右键菜单：锚点、目标、命令列表与高亮项索引。
#[derive(Debug)]
pub struct Menu {
    pub anchor: (u16, u16),
    pub target: OverlayTarget,
    pub commands: Vec<MenuCommand>,
    pub selected: usize,
}

/// 菜单与关闭确认作用目标：工作区、工作区内的标签或标签内的窗格，以及提示词树节点。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OverlayTarget {
    Workspace(usize),
    /// 工作区列表空白区域。
    WorkspaceList,
    Tab {
        workspace: usize,
        tab: usize,
    },
    Pane {
        workspace: usize,
        tab: usize,
        pane: PaneId,
    },
    /// 提示词树根层文件夹。
    PromptFolder {
        workspace: usize,
        folder_id: i64,
    },
    /// 提示词条目。
    PromptItem {
        workspace: usize,
        item_id: i64,
    },
    /// 临时提示词（仅内存，不入库）。
    TempPrompt {
        workspace: usize,
        temp_id: u64,
    },
    /// 提示词树空白区域。
    PromptTree {
        workspace: usize,
    },
    /// 提示词页（删除确认作用目标）：owner 为条目或临时提示词。
    PromptPage {
        workspace: usize,
        owner: PromptEditing,
        page: PromptPageId,
    },
}

/// 菜单命令；文案由 `ui/text.rs` 按命令映射，app 层不持有用户可见字符串。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MenuCommand {
    NewTab,
    NewTerminal,
    NewWorkspace,
    RenameWorkspace,
    OpenWorktree,
    OpenPrompt,
    CloseWorkspace,
    RenameTab,
    CloseTab,
    SplitRight,
    SplitDown,
    SwitchToTerminal,
    SwitchToLx,
    SwitchToWorkspaceCwd,
    SyncWorkspaceToTerminalCwd,
    ClosePane,
    NewPrompt,
    NewFolder,
    NewSubFolder,
    RenameFolder,
    DeleteFolder,
    RenamePrompt,
    DeletePrompt,
    /// 删除临时提示词（仅内存、立即生效、无确认浮层）。
    DeleteTemp,
    EditPrompt,
    CopyPromptContent,
    CopyPath,
}

/// 重命名作用目标：工作区、标签或提示词树节点；窗格不可重命名，因此不在此枚举内。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RenameTarget {
    Workspace(usize),
    Tab { workspace: usize, tab: usize },
    PromptFolder { workspace: usize, folder_id: i64 },
    PromptItem { workspace: usize, item_id: i64 },
}

/// 重命名浮层：目标与单行输入。
#[derive(Debug)]
pub struct Rename {
    pub target: RenameTarget,
    pub input: TextInput,
}

/// 关闭确认浮层：目标。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ConfirmClose {
    pub target: OverlayTarget,
}

/// 切换工作区路径确认浮层：目标窗格与要写入的目标路径。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfirmSwitchCwd {
    pub workspace: usize,
    pub tab: usize,
    pub pane: PaneId,
    pub path: PathBuf,
}

/// 切换当前工作区路径到终端路径确认浮层：目标工作区、目标窗格与要设置的终端路径。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfirmSyncWorkspaceCwd {
    pub workspace: usize,
    pub pane: PaneId,
    pub path: PathBuf,
}

/// 新建工作区浮层：单行路径输入。
#[derive(Debug)]
pub struct NewWorkspace {
    pub input: TextInput,
}

/// 打开已有 worktree 的浮层：源工作区、仓库根、条目与搜索输入。
#[derive(Debug)]
pub struct WorktreeOpen {
    /// 发起菜单的工作区索引。
    pub source: usize,
    /// 仓库主 checkout 路径；查询结果按此匹配。
    pub repo_root: PathBuf,
    pub entries: Vec<WorktreeOpenEntry>,
    pub selected: usize,
    pub query: TextInput,
    /// 后台查询尚未返回。
    pub loading: bool,
    /// 后台查询失败（非仓库、git 不可用）。
    pub failed: bool,
}

/// 对话框中的一条 worktree。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorktreeOpenEntry {
    pub path: PathBuf,
    pub branch: Option<String>,
    pub is_bare: bool,
    pub is_linked: bool,
    /// 已作为工作区打开时的索引。
    pub already_open: Option<usize>,
}

/// 条目的状态标记；文案由 ui 映射。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorktreeStatus {
    Open,
    Detached,
    Root,
    Branch,
}

impl WorktreeOpenEntry {
    /// 展示名：分支短名优先，无分支回退目录名。
    pub fn display_name(&self) -> String {
        if let Some(branch) = self.branch.as_deref() {
            return branch.to_string();
        }
        self.path
            .file_name()
            .and_then(|name| name.to_str())
            .filter(|name| !name.is_empty())
            .map(str::to_string)
            .unwrap_or_else(|| self.path.display().to_string())
    }

    /// 状态标记：已打开优先，其次分支/detached/主 checkout。
    pub fn status(&self) -> WorktreeStatus {
        if self.already_open.is_some() {
            WorktreeStatus::Open
        } else if self.branch.is_some() {
            WorktreeStatus::Branch
        } else if self.is_linked {
            WorktreeStatus::Detached
        } else {
            WorktreeStatus::Root
        }
    }

    /// 搜索匹配：名字、目录名、全路径与状态文本的 lowercase contains。
    pub fn matches_query(&self, query: &str) -> bool {
        let directory = self
            .path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or_default();
        let haystack = format!(
            "{} {} {} {:?}",
            self.display_name(),
            directory,
            self.path.display(),
            self.status()
        )
        .to_lowercase();
        query
            .to_lowercase()
            .split_whitespace()
            .all(|needle| haystack.contains(needle))
    }
}

impl WorktreeOpen {
    /// 过滤后的条目索引；空查询返回全部。
    pub fn filtered_indices(&self) -> Vec<usize> {
        let query = self.query.text().trim();
        self.entries
            .iter()
            .enumerate()
            .filter_map(|(index, entry)| {
                (query.is_empty() || entry.matches_query(query)).then_some(index)
            })
            .collect()
    }

    /// 当前选中项索引；选中项被过滤掉时回退到首个可见项。
    pub fn selected_entry_index(&self) -> Option<usize> {
        let indices = self.filtered_indices();
        if indices.contains(&self.selected) {
            return Some(self.selected);
        }
        indices.first().copied()
    }

    /// 把选中索引钳到过滤结果内。
    pub fn normalize_selection(&mut self) {
        if let Some(selected) = self.selected_entry_index() {
            self.selected = selected;
        }
    }

    /// 选择上一个可见项；已在首个或列表为空时不动。
    pub fn select_previous(&mut self) {
        let indices = self.filtered_indices();
        let Some(current) = self.selected_entry_index() else {
            return;
        };
        let position = indices
            .iter()
            .position(|index| *index == current)
            .unwrap_or(0);
        self.selected = indices[position.saturating_sub(1)];
    }

    /// 选择下一个可见项；已在末尾或列表为空时不动。
    pub fn select_next(&mut self) {
        let indices = self.filtered_indices();
        let Some(current) = self.selected_entry_index() else {
            return;
        };
        let position = indices
            .iter()
            .position(|index| *index == current)
            .unwrap_or(0);
        self.selected = indices[(position + 1).min(indices.len().saturating_sub(1))];
    }
}

/// 单行文本输入：字符缓冲与字符索引光标（范围 `0..=字符数`），及可选选区锚点。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TextInput {
    text: String,
    cursor: usize,
    selection_anchor: Option<usize>,
}

impl TextInput {
    /// 构造输入并预填文本，光标落在末尾。
    pub fn new(text: impl Into<String>) -> Self {
        let text = text.into();
        let cursor = text.chars().count();
        Self {
            text,
            cursor,
            selection_anchor: None,
        }
    }

    /// 当前文本。
    pub fn text(&self) -> &str {
        &self.text
    }

    /// 光标字符索引。
    pub fn cursor(&self) -> usize {
        self.cursor
    }

    /// 当前选区字符范围 `[start, end)`；无选区或空选区返回 None。
    pub fn selection(&self) -> Option<(usize, usize)> {
        let anchor = self.selection_anchor?;
        (anchor != self.cursor).then_some((anchor.min(self.cursor), anchor.max(self.cursor)))
    }

    /// 是否存在有效选区。
    pub fn has_selection(&self) -> bool {
        self.selection().is_some()
    }

    /// 获取当前选中的文本；无有效选区返回 None。
    pub fn selected_text(&self) -> Option<String> {
        let (start, end) = self.selection()?;
        let start_byte = self.byte_offset(start);
        let end_byte = self.byte_offset(end);
        Some(self.text[start_byte..end_byte].to_string())
    }

    /// 清除选区。
    pub fn clear_selection(&mut self) {
        self.selection_anchor = None;
    }

    /// 直接设置选区字符区间。
    pub fn set_selection(&mut self, start: usize, end: usize) {
        let len = self.text.chars().count();
        let start = start.min(len);
        let end = end.min(len);
        self.selection_anchor = Some(start);
        self.cursor = end;
    }

    /// 在指定字符索引处开始选区。
    pub fn begin_selection(&mut self, char_index: usize) {
        let char_index = char_index.min(self.text.chars().count());
        self.cursor = char_index;
        self.selection_anchor = Some(char_index);
    }

    /// 按点击次数在指定字符处开始选择（1=落光标、2=选词、3=全选）。
    pub fn begin_selection_with_clicks(&mut self, char_index: usize, clicks: u8) {
        match clicks {
            2 => self.select_word_at(char_index),
            clicks if clicks >= 3 => self.select_all(),
            _ => self.begin_selection(char_index),
        }
    }

    /// 拖拽延伸选区到指定字符索引。
    pub fn drag_selection(&mut self, char_index: usize) {
        let char_index = char_index.min(self.text.chars().count());
        if self.selection_anchor.is_none() {
            self.selection_anchor = Some(self.cursor);
        }
        self.cursor = char_index;
    }

    /// 全选当前文本。
    pub fn select_all(&mut self) {
        self.selection_anchor = Some(0);
        self.cursor = self.text.chars().count();
    }

    /// 选中指定字符所在单词。
    pub fn select_word_at(&mut self, char_index: usize) {
        let total = self.text.chars().count();
        if total == 0 {
            self.clear_selection();
            self.cursor = 0;
            return;
        }
        let char_index = char_index.min(total.saturating_sub(1));
        let chars: Vec<char> = self.text.chars().collect();
        let is_word_char = |c: char| c.is_alphanumeric() || c == '_' || c == '-';
        let target_word = is_word_char(chars[char_index]);
        let mut start = char_index;
        while start > 0
            && is_word_char(chars[start - 1]) == target_word
            && !chars[start - 1].is_whitespace()
        {
            start -= 1;
        }
        let mut end = char_index + 1;
        while end < total && is_word_char(chars[end]) == target_word && !chars[end].is_whitespace()
        {
            end += 1;
        }
        self.selection_anchor = Some(start);
        self.cursor = end;
    }

    /// 删除选中的文本区间，并把光标重置到选区起点。有选区删除返回 true。
    pub fn delete_selected(&mut self) -> bool {
        let Some((start, end)) = self.selection() else {
            self.selection_anchor = None;
            return false;
        };
        let start_byte = self.byte_offset(start);
        let end_byte = self.byte_offset(end);
        self.text.replace_range(start_byte..end_byte, "");
        self.cursor = start;
        self.selection_anchor = None;
        true
    }

    /// 在光标处插入字符（有选区时先替换选区）。
    pub fn insert_char(&mut self, ch: char) {
        self.delete_selected();
        let at = self.byte_offset(self.cursor);
        self.text.insert(at, ch);
        self.cursor += 1;
    }

    /// 在光标处插入一段文本（单行语义：换行与制表符替换为空格；有选区时先替换）；
    /// 返回是否实际插入。
    pub fn insert_str(&mut self, value: &str) -> bool {
        let cleaned: String = value
            .chars()
            .map(|ch| {
                if matches!(ch, '\n' | '\r' | '\t') {
                    ' '
                } else {
                    ch
                }
            })
            .filter(|ch| !ch.is_control())
            .collect();
        if cleaned.is_empty() {
            return false;
        }
        self.delete_selected();
        let at = self.byte_offset(self.cursor);
        self.text.insert_str(at, &cleaned);
        self.cursor += cleaned.chars().count();
        true
    }

    /// 清空文本并把光标移到开头，清除选区。
    pub fn clear(&mut self) {
        self.text.clear();
        self.cursor = 0;
        self.selection_anchor = None;
    }

    /// 删除光标前一个字符（有选区时删除整段选区）。
    pub fn backspace(&mut self) {
        if self.delete_selected() {
            return;
        }
        if self.cursor == 0 {
            return;
        }
        let start = self.byte_offset(self.cursor - 1);
        let end = self.byte_offset(self.cursor);
        self.text.replace_range(start..end, "");
        self.cursor -= 1;
    }

    /// 删除光标处字符（有选区时删除整段选区）。
    pub fn delete(&mut self) {
        if self.delete_selected() {
            return;
        }
        if self.cursor >= self.text.chars().count() {
            return;
        }
        let start = self.byte_offset(self.cursor);
        let end = self.byte_offset(self.cursor + 1);
        self.text.replace_range(start..end, "");
    }

    /// 删除光标到行首的内容（Ctrl+U；有选区时删除选区）。
    pub fn delete_to_line_start(&mut self) {
        if self.delete_selected() {
            return;
        }
        if self.cursor == 0 {
            return;
        }
        let end = self.byte_offset(self.cursor);
        self.text.replace_range(0..end, "");
        self.cursor = 0;
    }

    /// 删除光标到行尾的内容（Ctrl+K；有选区时删除选区）。
    pub fn delete_to_line_end(&mut self) {
        if self.delete_selected() {
            return;
        }
        let total = self.text.chars().count();
        if self.cursor >= total {
            return;
        }
        let start = self.byte_offset(self.cursor);
        self.text.replace_range(start.., "");
    }

    /// 光标左移（有选区时折叠到选区开头）。
    pub fn move_left(&mut self) {
        if let Some((start, _)) = self.selection() {
            self.cursor = start;
            self.selection_anchor = None;
        } else {
            self.cursor = self.cursor.saturating_sub(1);
        }
    }

    /// 光标右移（有选区时折叠到选区结尾）。
    pub fn move_right(&mut self) {
        if let Some((_, end)) = self.selection() {
            self.cursor = end;
            self.selection_anchor = None;
        } else if self.cursor < self.text.chars().count() {
            self.cursor += 1;
        }
    }

    /// 光标移到开头（清除选区）。
    pub fn move_home(&mut self) {
        self.clear_selection();
        self.cursor = 0;
    }

    /// 光标移到末尾（清除选区）。
    pub fn move_end(&mut self) {
        self.clear_selection();
        self.cursor = self.text.chars().count();
    }

    /// 字符索引到字节偏移；越界返回文本长度。
    fn byte_offset(&self, char_index: usize) -> usize {
        self.text
            .char_indices()
            .nth(char_index)
            .map(|(byte, _)| byte)
            .unwrap_or(self.text.len())
    }
}

#[cfg(test)]
mod tests;
