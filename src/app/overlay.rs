//! 浮层模型：右键菜单、重命名输入、关闭确认与 worktree 对话框；同一时刻最多存在一个浮层。

use std::path::PathBuf;

use crate::layout::PaneId;

/// 浮层种类：按键路由与渲染按种类分派。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OverlayKind {
    Menu,
    Rename,
    ConfirmClose,
    ConfirmSwitchCwd,
    ConfirmSyncWorkspaceCwd,
    WorktreeOpen,
}

/// 浮层：同一时刻最多一个，由 `Option` 保证。
#[derive(Debug)]
pub enum Overlay {
    Menu(Menu),
    Rename(Rename),
    ConfirmClose(ConfirmClose),
    ConfirmSwitchCwd(ConfirmSwitchCwd),
    ConfirmSyncWorkspaceCwd(ConfirmSyncWorkspaceCwd),
    WorktreeOpen(WorktreeOpen),
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
            Self::WorktreeOpen(_) => OverlayKind::WorktreeOpen,
        }
    }
}

/// 右键菜单：锚点、目标、命令列表与高亮项索引。
#[derive(Debug)]
pub struct Menu {
    pub anchor: (u16, u16),
    pub target: OverlayTarget,
    pub commands: Vec<MenuCommand>,
    pub selected: usize,
}

/// 菜单与关闭确认作用目标：工作区、工作区内的标签或标签内的窗格。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OverlayTarget {
    Workspace(usize),
    Tab {
        workspace: usize,
        tab: usize,
    },
    Pane {
        workspace: usize,
        tab: usize,
        pane: PaneId,
    },
}

/// 菜单命令；文案由 `ui/text.rs` 按命令映射，app 层不持有用户可见字符串。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MenuCommand {
    NewTab,
    NewTerminal,
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
}

/// 重命名作用目标：工作区或标签；窗格不可重命名，因此不在此枚举内。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RenameTarget {
    Workspace(usize),
    Tab { workspace: usize, tab: usize },
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

/// 单行文本输入：字符缓冲与字符索引光标（范围 `0..=字符数`）。
#[derive(Debug)]
pub struct TextInput {
    text: String,
    cursor: usize,
}

impl TextInput {
    /// 构造输入并预填文本，光标落在末尾。
    pub fn new(text: impl Into<String>) -> Self {
        let text = text.into();
        let cursor = text.chars().count();
        Self { text, cursor }
    }

    /// 当前文本。
    pub fn text(&self) -> &str {
        &self.text
    }

    /// 光标字符索引。
    pub fn cursor(&self) -> usize {
        self.cursor
    }

    /// 在光标处插入字符。
    pub fn insert_char(&mut self, ch: char) {
        let at = self.byte_offset(self.cursor);
        self.text.insert(at, ch);
        self.cursor += 1;
    }

    /// 清空文本并把光标移到开头。
    pub fn clear(&mut self) {
        self.text.clear();
        self.cursor = 0;
    }

    /// 删除光标前一个字符。
    pub fn backspace(&mut self) {
        if self.cursor == 0 {
            return;
        }
        let start = self.byte_offset(self.cursor - 1);
        let end = self.byte_offset(self.cursor);
        self.text.replace_range(start..end, "");
        self.cursor -= 1;
    }

    /// 删除光标处字符。
    pub fn delete(&mut self) {
        if self.cursor >= self.text.chars().count() {
            return;
        }
        let start = self.byte_offset(self.cursor);
        let end = self.byte_offset(self.cursor + 1);
        self.text.replace_range(start..end, "");
    }

    /// 光标左移。
    pub fn move_left(&mut self) {
        self.cursor = self.cursor.saturating_sub(1);
    }

    /// 光标右移。
    pub fn move_right(&mut self) {
        if self.cursor < self.text.chars().count() {
            self.cursor += 1;
        }
    }

    /// 光标移到开头。
    pub fn move_home(&mut self) {
        self.cursor = 0;
    }

    /// 光标移到末尾。
    pub fn move_end(&mut self) {
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
