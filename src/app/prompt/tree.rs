//! 提示词树纯内存模型：文件夹最多 2 层（根层 + 子层），条目挂在根或任一文件夹下。
//!
//! 树保持扁平存储（与 DB 记录一一对应），可见行由 `rows` 按折叠状态构建；
//! 持久化以 DB 为准，重载时保留同名 id 的折叠状态。

use crate::app::overlay::TextInput;
use crate::db::{FolderRecord, PromptRecord};

/// 提示词条目状态；与 DB 稳定值 `todo` / `in_progress` / `completed` 一一对应。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PromptItemStatus {
    Todo,
    InProgress,
    Completed,
}

impl PromptItemStatus {
    /// DB 稳定值。
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Todo => "todo",
            Self::InProgress => "in_progress",
            Self::Completed => "completed",
        }
    }

    /// 从 DB 稳定值解析；未知值回退 `todo`。
    pub fn parse(value: &str) -> Self {
        match value {
            "in_progress" => Self::InProgress,
            "completed" => Self::Completed,
            _ => Self::Todo,
        }
    }

    /// 状态循环：todo → in_progress → completed → todo。
    pub fn next(self) -> Self {
        match self {
            Self::Todo => Self::InProgress,
            Self::InProgress => Self::Completed,
            Self::Completed => Self::Todo,
        }
    }
}

/// 文件夹：`parent_id` 指向根层文件夹（`Some` 时为第 2 层）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PromptFolder {
    pub id: i64,
    pub parent_id: Option<i64>,
    pub name: String,
    pub collapsed: bool,
}

/// 提示词条目。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PromptItem {
    pub id: i64,
    pub folder_id: Option<i64>,
    pub name: String,
    pub content: String,
    pub status: PromptItemStatus,
}

/// 可见行种类。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PromptRowKind {
    Folder,
    Item,
}

/// 一行可见节点：种类、在对应列表中的索引与缩进深度（0 或 1）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PromptRow {
    pub kind: PromptRowKind,
    pub index: usize,
    pub depth: usize,
}

/// 提示词树：文件夹与条目的扁平存储。
#[derive(Debug, Default, Clone)]
pub struct PromptTree {
    folders: Vec<PromptFolder>,
    items: Vec<PromptItem>,
}

impl PromptTree {
    /// 空树。
    pub fn new() -> Self {
        Self::default()
    }

    /// 用 DB 记录整体替换；同 id 文件夹保留折叠状态，列表按 id 排序保证稳定。
    pub fn replace(&mut self, folders: Vec<PromptFolder>, mut items: Vec<PromptItem>) {
        let collapsed: Vec<(i64, bool)> = self
            .folders
            .iter()
            .map(|folder| (folder.id, folder.collapsed))
            .collect();
        let mut folders = folders;
        for folder in &mut folders {
            folder.collapsed = collapsed
                .iter()
                .find(|(id, _)| *id == folder.id)
                .is_some_and(|(_, collapsed)| *collapsed);
        }
        folders.sort_by_key(|folder| folder.id);
        items.sort_by_key(|item| item.id);
        self.folders = folders;
        self.items = items;
    }

    /// 全部文件夹。
    pub fn folders(&self) -> &[PromptFolder] {
        &self.folders
    }

    /// 全部条目。
    pub fn items(&self) -> &[PromptItem] {
        &self.items
    }

    /// 按 id 取文件夹。
    pub fn folder(&self, id: i64) -> Option<&PromptFolder> {
        self.folders.iter().find(|folder| folder.id == id)
    }

    /// 按 id 取文件夹（可变）。
    pub fn folder_mut(&mut self, id: i64) -> Option<&mut PromptFolder> {
        self.folders.iter_mut().find(|folder| folder.id == id)
    }

    /// 按 id 取条目。
    pub fn item(&self, id: i64) -> Option<&PromptItem> {
        self.items.iter().find(|item| item.id == id)
    }

    /// 按 id 取条目（可变）。
    pub fn item_mut(&mut self, id: i64) -> Option<&mut PromptItem> {
        self.items.iter_mut().find(|item| item.id == id)
    }

    /// 文件夹深度：根层 0、子层 1；不存在为 None。
    pub fn folder_depth(&self, id: i64) -> Option<usize> {
        let folder = self.folder(id)?;
        Some(usize::from(folder.parent_id.is_some()))
    }

    /// 是否可在该文件夹下新建子文件夹（仅根层文件夹允许）。
    pub fn can_create_subfolder(&self, id: i64) -> bool {
        self.folder_depth(id) == Some(0)
    }

    /// 切换文件夹折叠状态；不存在时 no-op。
    pub fn toggle_folder(&mut self, id: i64) {
        if let Some(folder) = self.folder_mut(id) {
            folder.collapsed = !folder.collapsed;
        }
    }

    /// 删除文件夹：级联移除子文件夹与其下条目。
    pub fn delete_folder(&mut self, id: i64) {
        let mut removed = vec![id];
        let mut frontier = vec![id];
        while let Some(parent) = frontier.pop() {
            for folder in &self.folders {
                if folder.parent_id == Some(parent) && !removed.contains(&folder.id) {
                    removed.push(folder.id);
                    frontier.push(folder.id);
                }
            }
        }
        self.folders.retain(|folder| !removed.contains(&folder.id));
        self.items
            .retain(|item| !item.folder_id.is_some_and(|id| removed.contains(&id)));
    }

    /// 删除条目；不存在时 no-op。
    pub fn delete_item(&mut self, id: i64) {
        self.items.retain(|item| item.id != id);
    }

    /// 可见行：根层文件夹（含展开内容的子树）在前，根层条目在后；
    /// 文件夹内子文件夹在前、条目在后，折叠的文件夹隐藏后代。
    pub fn rows(&self) -> Vec<PromptRow> {
        let mut rows = Vec::new();
        for (index, folder) in self.folders.iter().enumerate() {
            if folder.parent_id.is_some() {
                continue;
            }
            rows.push(PromptRow {
                kind: PromptRowKind::Folder,
                index,
                depth: 0,
            });
            if folder.collapsed {
                continue;
            }
            self.push_children(&mut rows, folder.id);
        }
        for (index, item) in self.items.iter().enumerate() {
            if item.folder_id.is_none() {
                rows.push(PromptRow {
                    kind: PromptRowKind::Item,
                    index,
                    depth: 0,
                });
            }
        }
        rows
    }

    /// 展开的根层文件夹：先子文件夹（含其条目），再直属条目。
    fn push_children(&self, rows: &mut Vec<PromptRow>, parent: i64) {
        for (index, folder) in self.folders.iter().enumerate() {
            if folder.parent_id != Some(parent) {
                continue;
            }
            rows.push(PromptRow {
                kind: PromptRowKind::Folder,
                index,
                depth: 1,
            });
            if folder.collapsed {
                continue;
            }
            for (item_index, item) in self.items.iter().enumerate() {
                if item.folder_id == Some(folder.id) {
                    rows.push(PromptRow {
                        kind: PromptRowKind::Item,
                        index: item_index,
                        depth: 1,
                    });
                }
            }
        }
        for (index, item) in self.items.iter().enumerate() {
            if item.folder_id == Some(parent) {
                rows.push(PromptRow {
                    kind: PromptRowKind::Item,
                    index,
                    depth: 1,
                });
            }
        }
    }

    /// 行数据：按可见行取文件夹或条目。
    pub fn folder_of_row(&self, row: PromptRow) -> Option<&PromptFolder> {
        (row.kind == PromptRowKind::Folder)
            .then(|| self.folders.get(row.index))
            .flatten()
    }

    /// 行数据：按可见行取条目。
    pub fn item_of_row(&self, row: PromptRow) -> Option<&PromptItem> {
        (row.kind == PromptRowKind::Item)
            .then(|| self.items.get(row.index))
            .flatten()
    }
}

impl From<FolderRecord> for PromptFolder {
    fn from(record: FolderRecord) -> Self {
        Self {
            id: record.id,
            parent_id: record.parent_id,
            name: record.name,
            collapsed: false,
        }
    }
}

impl From<PromptRecord> for PromptItem {
    fn from(record: PromptRecord) -> Self {
        Self {
            id: record.id,
            folder_id: record.folder_id,
            name: record.name,
            content: record.content,
            status: PromptItemStatus::parse(&record.status),
        }
    }
}

/// 提示词面板视图模式。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PromptMode {
    /// 树状管理器：文件夹与条目列表。
    Tree,
    /// 编辑视图：编辑选中条目正文。
    Editor,
}

/// 编辑视图绑定的目标：正式条目或临时草稿。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PromptEditing {
    Item(i64),
    /// 临时草稿：内容只存内存（`PromptPanel::draft`），不入库。
    Draft,
}

/// 工作区提示词面板状态：树、视图模式、选中行、编辑目标、草稿与滚动偏移。
#[derive(Debug)]
pub struct PromptPanel {
    pub tree: PromptTree,
    /// 是否已从 DB 载入（含无持久化模式下的空树）。
    pub loaded: bool,
    pub mode: PromptMode,
    /// 选中可见行索引；列表置顶草稿行恒为 0。
    pub selected: usize,
    /// 编辑视图绑定的目标；仅编辑视图下有效。
    pub editing: Option<PromptEditing>,
    /// 树视图首个可见行索引。
    pub scroll: usize,
    /// 临时草稿正文；保存后清空，不入库。
    pub draft: String,
    /// 树视图搜索过滤词（单行输入语义：光标可移动编辑）；空串为不过滤。
    pub search: TextInput,
}

impl PromptPanel {
    /// 空面板：默认编辑视图，保持既有右栏行为。
    pub fn new() -> Self {
        Self {
            tree: PromptTree::new(),
            loaded: false,
            mode: PromptMode::Editor,
            selected: 0,
            editing: None,
            scroll: 0,
            draft: String::new(),
            search: TextInput::new(""),
        }
    }

    /// 可见行：过滤后的树行。
    pub fn rows(&self) -> Vec<PromptRow> {
        self.filtered_tree_rows()
    }

    /// 过滤后的树行；无过滤词时与 `tree.rows()` 一致。
    ///
    /// 过滤忽略折叠状态：命中文件夹展示其整棵子树，命中条目展示祖先链；
    /// 临时草稿行不参与过滤（由 `rows` 恒置顶）。
    fn filtered_tree_rows(&self) -> Vec<PromptRow> {
        let query = self.search.text().trim().to_lowercase();
        if query.is_empty() {
            return self.tree.rows();
        }
        let folders = self.tree.folders();
        let items = self.tree.items();
        let mut rows = Vec::new();
        for (index, folder) in folders.iter().enumerate() {
            if folder.parent_id.is_some() {
                continue;
            }
            let full = name_matches(&folder.name, &query);
            let sub_visible = |sub: &PromptFolder| {
                full || name_matches(&sub.name, &query)
                    || items.iter().any(|item| {
                        item.folder_id == Some(sub.id) && name_matches(&item.name, &query)
                    })
            };
            let any_child = folders
                .iter()
                .any(|sub| sub.parent_id == Some(folder.id) && sub_visible(sub))
                || items.iter().any(|item| {
                    item.folder_id == Some(folder.id) && name_matches(&item.name, &query)
                });
            if !(full || any_child) {
                continue;
            }
            rows.push(PromptRow {
                kind: PromptRowKind::Folder,
                index,
                depth: 0,
            });
            for (sub_index, sub) in folders.iter().enumerate() {
                if sub.parent_id != Some(folder.id) {
                    continue;
                }
                let sub_full = full || name_matches(&sub.name, &query);
                let child_items: Vec<usize> = items
                    .iter()
                    .enumerate()
                    .filter(|(_, item)| {
                        item.folder_id == Some(sub.id)
                            && (sub_full || name_matches(&item.name, &query))
                    })
                    .map(|(item_index, _)| item_index)
                    .collect();
                if !sub_full && child_items.is_empty() {
                    continue;
                }
                rows.push(PromptRow {
                    kind: PromptRowKind::Folder,
                    index: sub_index,
                    depth: 1,
                });
                for item_index in child_items {
                    rows.push(PromptRow {
                        kind: PromptRowKind::Item,
                        index: item_index,
                        depth: 1,
                    });
                }
            }
            for (item_index, item) in items.iter().enumerate() {
                if item.folder_id == Some(folder.id) && (full || name_matches(&item.name, &query)) {
                    rows.push(PromptRow {
                        kind: PromptRowKind::Item,
                        index: item_index,
                        depth: 1,
                    });
                }
            }
        }
        for (item_index, item) in items.iter().enumerate() {
            if item.folder_id.is_none() && name_matches(&item.name, &query) {
                rows.push(PromptRow {
                    kind: PromptRowKind::Item,
                    index: item_index,
                    depth: 0,
                });
            }
        }
        rows
    }

    /// 选中行数据。
    pub fn selected_row(&self) -> Option<PromptRow> {
        self.rows().get(self.selected).copied()
    }

    /// 选中条目 id；草稿行、文件夹行或空树为 None。
    pub fn selected_prompt_id(&self) -> Option<i64> {
        self.selected_row()
            .and_then(|row| self.tree.item_of_row(row))
            .map(|item| item.id)
    }

    /// 选中行钳制（树变化后调用）；滚动同步跟随。
    pub fn clamp_selection(&mut self, visible: usize) {
        let rows = self.rows().len();
        self.selected = self.selected.min(rows.saturating_sub(1));
        self.scroll = self.scroll.min(rows.saturating_sub(visible.max(1)));
        if self.selected < self.scroll {
            self.scroll = self.selected;
        } else if self.selected >= self.scroll + visible.max(1) {
            self.scroll = self.selected + 1 - visible.max(1);
        }
    }
}

/// 名称是否命中过滤词（大小写不敏感子串）。
fn name_matches(name: &str, query: &str) -> bool {
    name.to_lowercase().contains(query)
}

impl Default for PromptPanel {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests;
