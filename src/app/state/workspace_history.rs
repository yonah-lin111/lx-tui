//! 侧栏工作区历史列表状态：模式切换、搜索过滤、选中与滚动。

use std::path::PathBuf;
use std::time::Instant;

use crate::app::overlay::TextInput;

/// 左侧栏上半区（Workspaces）的展示模式。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum WorkspaceMode {
    /// 默认视图：当前打开的活跃工作区列表。
    #[default]
    Active,
    /// 历史视图：持久化的工作区历史列表（含搜索框）。
    History,
}

/// 单条工作区历史记录。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkspaceHistoryEntry {
    pub name: String,
    pub path: PathBuf,
    /// 目标目录当前在磁盘上是否存在（`!exists` 时以异色显示并允许级联删除）。
    pub exists: bool,
}

/// 工作区历史面板状态。
#[derive(Debug)]
pub struct WorkspaceHistoryPanel {
    pub mode: WorkspaceMode,
    pub entries: Vec<WorkspaceHistoryEntry>,
    pub search: TextInput,
    /// 当前选中可见行下标（相对于 `filtered_indices`）。
    pub selected: usize,
    /// 可见行顶部滚动偏移。
    pub scroll: usize,
    /// 鼠标悬停的可见行下标。
    pub hover: Option<usize>,
    /// 历史列表滚动条拖拽锚点。
    pub scroll_drag: Option<u16>,
    /// 是否正在拖拽选择搜索框文本。
    pub search_drag: bool,
    /// 最近一次左键按下记录 `(visible_row, pressed_at)`，用于 400ms 双击判定。
    pub row_click: Option<(usize, Instant)>,
    /// 启动后是否已完成首次数据库加载。
    pub loaded: bool,
}

impl Default for WorkspaceHistoryPanel {
    fn default() -> Self {
        Self::new()
    }
}

impl WorkspaceHistoryPanel {
    pub fn new() -> Self {
        Self {
            mode: WorkspaceMode::Active,
            entries: Vec::new(),
            search: TextInput::new(String::new()),
            selected: 0,
            scroll: 0,
            hover: None,
            scroll_drag: None,
            search_drag: false,
            row_click: None,
            loaded: false,
        }
    }

    /// 过滤后的条目在 `entries` 中的原始下标列表。
    pub fn filtered_indices(&self) -> Vec<usize> {
        let query = self.search.text().trim().to_lowercase();
        if query.is_empty() {
            return (0..self.entries.len()).collect();
        }
        self.entries
            .iter()
            .enumerate()
            .filter(|(_, entry)| {
                entry.name.to_lowercase().contains(&query)
                    || entry.path.to_string_lossy().to_lowercase().contains(&query)
            })
            .map(|(index, _)| index)
            .collect()
    }

    /// 按过滤后的可见行下标取 `(entry_index, &WorkspaceHistoryEntry)`。
    pub fn entry_at_visible_row(
        &self,
        visible_row: usize,
    ) -> Option<(usize, &WorkspaceHistoryEntry)> {
        let entry_index = *self.filtered_indices().get(visible_row)?;
        let entry = self.entries.get(entry_index)?;
        Some((entry_index, entry))
    }

    /// 刷新每条记录的路径存在性（在进入历史视图或加载列表时调用）；返回是否有变化。
    pub fn refresh_existence(&mut self) -> bool {
        let mut changed = false;
        for entry in &mut self.entries {
            let exists = entry.path.is_dir();
            if entry.exists != exists {
                entry.exists = exists;
                changed = true;
            }
        }
        changed
    }

    /// 钳制选中项与滚动偏移。
    pub fn clamp(&mut self, visible_rows: usize) {
        let total = self.filtered_indices().len();
        if total == 0 {
            self.selected = 0;
            self.scroll = 0;
            return;
        }
        self.selected = self.selected.min(total - 1);
        let max_scroll = total.saturating_sub(visible_rows.max(1));
        self.scroll = self.scroll.min(max_scroll);
    }

    /// 确保当前选中行在可见视口内。
    pub fn scroll_selected_into_view(&mut self, visible_rows: usize) {
        let visible = visible_rows.max(1);
        let total = self.filtered_indices().len();
        if total == 0 {
            self.scroll = 0;
            return;
        }
        self.selected = self.selected.min(total - 1);
        if self.selected < self.scroll {
            self.scroll = self.selected;
        } else if self.selected >= self.scroll + visible {
            self.scroll = self.selected + 1 - visible;
        }
        let max_scroll = total.saturating_sub(visible);
        self.scroll = self.scroll.min(max_scroll);
    }
}
