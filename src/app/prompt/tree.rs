//! 提示词树纯内存模型：文件夹最多 2 层（根层 + 子层），条目挂在根或任一文件夹下。
//!
//! 树保持扁平存储（与 DB 记录一一对应），可见行由 `rows` 按折叠状态构建；
//! 持久化以 DB 为准，重载时保留同名 id 的折叠状态。

pub mod panel;

pub use panel::*;

use crate::app::markdown::TemplateCounts;
use crate::db::{FolderRecord, PromptPageRecord, PromptRecord};
use crate::ui::text;

/// 页 id：正式页来自数据库、临时页由面板内存分配；两者互不混用。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PromptPageId {
    Saved(i64),
    Temp(u64),
}

impl PromptPageId {
    /// 数据库页 id；临时页为 None。
    pub fn saved(self) -> Option<i64> {
        match self {
            Self::Saved(id) => Some(id),
            Self::Temp(_) => None,
        }
    }
}

/// 提示词页：稳定 id、名称与正文。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PromptPage {
    pub id: PromptPageId,
    pub name: String,
    pub content: String,
}

impl PromptPage {
    /// 正式页（id 来自数据库）。
    pub fn saved(id: i64, name: impl Into<String>, content: impl Into<String>) -> Self {
        Self {
            id: PromptPageId::Saved(id),
            name: name.into(),
            content: content.into(),
        }
    }

    /// 临时页（id 由面板内存分配）。
    pub fn temp(id: u64, name: impl Into<String>, content: impl Into<String>) -> Self {
        Self {
            id: PromptPageId::Temp(id),
            name: name.into(),
            content: content.into(),
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
    /// 全部页（按创建顺序，恒非空）。
    pub pages: Vec<PromptPage>,
    /// 上次打开的页 id；失效时回退第一页。
    pub active_page: PromptPageId,
}

impl PromptItem {
    /// 单页条目：正文进入以条目 id 为页 id 的唯一 `Page 1`（测试与直插数据辅助）。
    pub fn single(
        id: i64,
        folder_id: Option<i64>,
        name: impl Into<String>,
        content: impl Into<String>,
    ) -> Self {
        let page = PromptPage::saved(id, text::default_page_name(1), content);
        Self {
            id,
            folder_id,
            name: name.into(),
            active_page: page.id,
            pages: vec![page],
        }
    }

    /// 当前页正文。
    pub fn active_content(&self) -> &str {
        self.active_page()
            .map(|page| page.content.as_str())
            .unwrap_or("")
    }

    /// 页在集合中的索引；不存在为 None。
    pub fn page_index(&self, id: PromptPageId) -> Option<usize> {
        self.pages.iter().position(|page| page.id == id)
    }

    /// 当前页索引；失效 id 回退第一页。
    pub fn active_index(&self) -> usize {
        self.page_index(self.active_page).unwrap_or(0)
    }

    /// 当前页。
    pub fn active_page(&self) -> Option<&PromptPage> {
        self.pages.get(self.active_index())
    }

    /// 按 id 取页。
    pub fn page(&self, id: PromptPageId) -> Option<&PromptPage> {
        self.pages.iter().find(|page| page.id == id)
    }

    /// 按 id 取页（可变）。
    pub fn page_mut(&mut self, id: PromptPageId) -> Option<&mut PromptPage> {
        self.pages.iter_mut().find(|page| page.id == id)
    }

    /// 当前页（可变）。
    pub fn active_page_mut(&mut self) -> Option<&mut PromptPage> {
        let index = self.active_index();
        self.pages.get_mut(index)
    }

    /// 全部页的模板块状态合计。
    pub fn template_counts(&self) -> TemplateCounts {
        pages_template_counts(&self.pages)
    }
}

/// 临时提示词：仅存内存、不入库；列表展示名取各页首个非空行。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TempPrompt {
    pub id: u64,
    /// 全部页（按创建顺序，恒非空）。
    pub pages: Vec<PromptPage>,
    /// 当前页 id；失效时回退第一页。
    pub active_page: PromptPageId,
}

impl TempPrompt {
    /// 单页临时提示词（页 id 取提示词 id；测试与新建默认形态辅助）。
    pub fn single_page(id: u64, content: impl Into<String>) -> Self {
        Self::single(id, id, text::default_page_name(1), content)
    }

    /// 单页临时提示词。
    pub fn single(
        id: u64,
        page_id: u64,
        name: impl Into<String>,
        content: impl Into<String>,
    ) -> Self {
        let page = PromptPage::temp(page_id, name, content);
        Self {
            id,
            active_page: page.id,
            pages: vec![page],
        }
    }

    /// 当前页正文。
    pub fn active_content(&self) -> &str {
        self.active_page()
            .map(|page| page.content.as_str())
            .unwrap_or("")
    }

    /// 列表展示名：各页按顺序的首个非空行（trim）；全部空白时为空串。
    pub fn summary(&self) -> &str {
        self.pages
            .iter()
            .flat_map(|page| page.content.lines())
            .map(str::trim)
            .find(|line| !line.is_empty())
            .unwrap_or("")
    }

    /// 全部页内容去空白后是否为空（空白 temp 即工作区恒保留的 default temp）。
    pub fn is_blank(&self) -> bool {
        self.pages.iter().all(|page| page.content.trim().is_empty())
    }

    /// 列表展示名：非空内容取首行摘要；空白（default temp）取占位名。
    pub fn display_name(&self) -> &str {
        if self.is_blank() {
            text::PROMPT_TEMP_DEFAULT_NAME
        } else {
            self.summary()
        }
    }

    /// 当前页索引；失效 id 回退第一页。
    pub fn active_index(&self) -> usize {
        self.pages
            .iter()
            .position(|page| page.id == self.active_page)
            .unwrap_or(0)
    }

    /// 当前页。
    pub fn active_page(&self) -> Option<&PromptPage> {
        self.pages.get(self.active_index())
    }

    /// 当前页（可变）。
    pub fn active_page_mut(&mut self) -> Option<&mut PromptPage> {
        let index = self.active_index();
        self.pages.get_mut(index)
    }

    /// 按 id 取页（可变）。
    pub fn page_mut(&mut self, id: PromptPageId) -> Option<&mut PromptPage> {
        self.pages.iter_mut().find(|page| page.id == id)
    }

    /// 全部页的模板块状态合计。
    pub fn template_counts(&self) -> TemplateCounts {
        pages_template_counts(&self.pages)
    }
}

/// 页集合的模板块状态合计。
pub fn pages_template_counts(pages: &[PromptPage]) -> TemplateCounts {
    pages
        .iter()
        .map(|page| crate::app::markdown::template_counts(&page.content))
        .fold(TemplateCounts::default(), TemplateCounts::merge)
}

/// 分区标签种类：临时区与已入库区。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PromptSection {
    /// 临时提示词区（`tmp:`）。
    Temp,
    /// 已入库提示词区（`saved:`）。
    Saved,
}

/// 可见行种类。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PromptRowKind {
    /// 分区标签行：不可选中、不可交互。
    Section(PromptSection),
    Folder,
    Item,
    /// 临时提示词行。
    Temp,
}

impl PromptRowKind {
    /// 行是否可选中（分区标签只作分隔）。
    pub fn selectable(self) -> bool {
        !matches!(self, Self::Section(_))
    }
}

/// 一行可见节点：种类、在对应列表中的索引与缩进深度（0 根层、1 根层文件夹内、2 子文件夹内）。
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
                        depth: 2,
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
        let mut pages: Vec<PromptPage> = record
            .pages
            .into_iter()
            .map(|page| PromptPage::saved(page.id, page.name, page.content))
            .collect();
        // 迁移后恒有页；直插旧数据兜底为第一页镜像（id 0 不会命中保存）。
        if pages.is_empty() {
            pages.push(PromptPage::saved(
                0,
                text::default_page_name(1),
                record.content,
            ));
        }
        let active_page = record
            .active_page_id
            .map(PromptPageId::Saved)
            .filter(|id| pages.iter().any(|page| page.id == *id))
            .unwrap_or(pages[0].id);
        Self {
            id: record.id,
            folder_id: record.folder_id,
            name: record.name,
            pages,
            active_page,
        }
    }
}

impl From<PromptPageRecord> for PromptPage {
    fn from(record: PromptPageRecord) -> Self {
        Self::saved(record.id, record.name, record.content)
    }
}

#[cfg(test)]
mod tests;
