//! 提示词树纯内存模型：文件夹最多 2 层（根层 + 子层），条目挂在根或任一文件夹下。
//!
//! 树保持扁平存储（与 DB 记录一一对应），可见行由 `rows` 按折叠状态构建；
//! 持久化以 DB 为准，重载时保留同名 id 的折叠状态。

use std::path::PathBuf;

use crate::app::markdown::TemplateCounts;
use crate::app::overlay::TextInput;
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
    /// 全部页（按创建顺序，恒非空）。
    pub pages: Vec<PromptPage>,
    /// 上次打开的页 id；失效时回退第一页。
    pub active_page: PromptPageId,
    pub status: PromptItemStatus,
}

impl PromptItem {
    /// 单页条目：正文进入以条目 id 为页 id 的唯一 `Page 1`（测试与直插数据辅助）。
    pub fn single(
        id: i64,
        folder_id: Option<i64>,
        name: impl Into<String>,
        content: impl Into<String>,
        status: PromptItemStatus,
    ) -> Self {
        let page = PromptPage::saved(id, text::default_page_name(1), content);
        Self {
            id,
            folder_id,
            name: name.into(),
            active_page: page.id,
            pages: vec![page],
            status,
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
            status: PromptItemStatus::parse(&record.status),
        }
    }
}

impl From<PromptPageRecord> for PromptPage {
    fn from(record: PromptPageRecord) -> Self {
        Self::saved(record.id, record.name, record.content)
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

/// 编辑视图绑定的目标：正式条目或临时提示词。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PromptEditing {
    Item(i64),
    /// 临时提示词：内容只存内存（`PromptPanel::temps`），不入库。
    Temp(u64),
}

/// 页重命名输入现场：目标页与单行输入（名称预填，Esc 保留旧名）。
#[derive(Debug)]
pub struct PageRename {
    pub page: PromptPageId,
    pub input: TextInput,
}

/// 工作区提示词面板状态：树、视图模式、选中行、编辑目标、临时提示词与滚动偏移。
#[derive(Debug)]
pub struct PromptPanel {
    pub tree: PromptTree,
    /// 是否已从 DB 载入（含无持久化模式下的空树）。
    pub loaded: bool,
    /// 已载入树对应的 prompt 键（git 工作区为仓库主 checkout，其余为 cwd）；
    /// 未载入或工作区无路径时为 None；git 元数据晚到导致键变化时据此重载。
    pub loaded_key: Option<PathBuf>,
    pub mode: PromptMode,
    /// 选中可见行索引；选中恒为可交互行（分区标签跳过）。
    pub selected: usize,
    /// 编辑视图绑定的目标；仅编辑视图下有效。
    pub editing: Option<PromptEditing>,
    /// 树视图首个可见行索引。
    pub scroll: usize,
    /// 临时提示词列表（仅内存）；`[tmp]` 新建、保存到库后移除；
    /// 空白项至多一条（default temp，列表中占位名 `default`，恒保留）。
    temps: Vec<TempPrompt>,
    /// 下一个临时提示词 id（单调递增，不复用）。
    next_temp_id: u64,
    /// 下一个临时页 id（单调递增，不复用）。
    next_temp_page_id: u64,
    /// 进入树视图时暂存的编辑绑定；返回编辑且用户未改过选中时恢复。
    tree_return: Option<PromptEditing>,
    /// 本次树视图中用户是否显式改过选中（点击行 / 方向键）。
    tree_selection_touched: bool,
    /// 树视图搜索过滤词（单行输入语义：光标可移动编辑）；空串为不过滤。
    pub search: TextInput,
    /// 页标题行的重命名输入现场；None 表示未在重命名。
    page_rename: Option<PageRename>,
    /// 已更新内存但数据库尚未确认的页（条目 id、页 id）。
    pending_saves: Vec<(i64, i64)>,
}

impl PromptPanel {
    /// 空面板：默认编辑视图并绑定一个空临时提示词（进入应用/切换工作区默认打开临时提示词）。
    pub fn new() -> Self {
        Self {
            tree: PromptTree::new(),
            loaded: false,
            loaded_key: None,
            mode: PromptMode::Editor,
            selected: 0,
            editing: Some(PromptEditing::Temp(0)),
            scroll: 0,
            temps: vec![TempPrompt::single(
                0,
                0,
                text::default_page_name(1),
                String::new(),
            )],
            next_temp_id: 1,
            next_temp_page_id: 1,
            tree_return: None,
            tree_selection_touched: false,
            search: TextInput::new(""),
            page_rename: None,
            pending_saves: Vec::new(),
        }
    }

    /// 标记条目页已写入内存、等待数据库确认。
    pub fn mark_page_pending(&mut self, item: i64, page: i64) {
        if !self.pending_saves.contains(&(item, page)) {
            self.pending_saves.push((item, page));
        }
    }

    /// 清除条目页的待确认标记（保存成功或页已被删除）。
    pub fn clear_page_pending(&mut self, item: i64, page: i64) {
        self.pending_saves
            .retain(|pending| *pending != (item, page));
    }

    /// 条目页是否有未确认的保存。
    pub fn page_pending(&self, item: i64, page: i64) -> bool {
        self.pending_saves.contains(&(item, page))
    }

    /// 待确认条目页列表（重载合并与清理用）。
    pub fn pending_saves(&self) -> &[(i64, i64)] {
        &self.pending_saves
    }

    /// 仅保留仍有效的待确认标记（取消失效条目/页的重试）。
    pub fn retain_pending(&mut self, keep: &[(i64, i64)]) {
        self.pending_saves.retain(|pair| keep.contains(pair));
    }

    /// 清除全部待确认标记（树重载：无有效页时）。
    pub fn clear_pending_saves(&mut self) {
        self.pending_saves.clear();
    }

    /// 页重命名现场（只读）。
    pub fn page_rename(&self) -> Option<&PageRename> {
        self.page_rename.as_ref()
    }

    /// 页重命名现场（可变，供输入编辑）。
    pub fn page_rename_mut(&mut self) -> Option<&mut PageRename> {
        self.page_rename.as_mut()
    }

    /// 进入页重命名输入态（预填页名）。
    pub fn start_page_rename(&mut self, page: PromptPageId, name: &str) {
        self.page_rename = Some(PageRename {
            page,
            input: TextInput::new(name),
        });
    }

    /// 结束页重命名输入态，返回现场。
    pub fn take_page_rename(&mut self) -> Option<PageRename> {
        self.page_rename.take()
    }

    /// 丢弃页重命名输入态（保留旧名）。
    pub fn cancel_page_rename(&mut self) {
        self.page_rename = None;
    }

    /// 进入树视图：暂存当前编辑绑定并复位"用户改过选中"标记（须在空白绑定清除之后调用）。
    pub fn enter_tree_view(&mut self) {
        self.tree_return = self.editing;
        self.tree_selection_touched = false;
    }

    /// 记录用户显式改过选中（点击行 / 方向键）。
    pub fn mark_tree_selection(&mut self) {
        self.tree_selection_touched = true;
    }

    /// 本次树视图中用户是否显式改过选中。
    pub fn tree_selection_touched(&self) -> bool {
        self.tree_selection_touched
    }

    /// 返回编辑视图的恢复目标：暂存绑定仍有效时返回其绑定（用户未改过选中时由调用方取用）。
    pub fn tree_restore_target(&self) -> Option<PromptEditing> {
        match self.tree_return {
            Some(PromptEditing::Item(id)) if self.tree.item(id).is_some() => {
                Some(PromptEditing::Item(id))
            }
            Some(PromptEditing::Temp(id)) if self.temp(id).is_some() => {
                Some(PromptEditing::Temp(id))
            }
            _ => None,
        }
    }

    /// 新建空临时提示词并返回其 id（不改变绑定）。
    pub fn new_temp(&mut self) -> u64 {
        let id = self.next_temp_id;
        self.next_temp_id = self.next_temp_id.saturating_add(1);
        let page_id = self.alloc_temp_page_id();
        self.temps.push(TempPrompt::single(
            id,
            page_id,
            text::default_page_name(1),
            String::new(),
        ));
        id
    }

    /// 分配临时页 id（单调递增、不复用）。
    pub fn alloc_temp_page_id(&mut self) -> u64 {
        let id = self.next_temp_page_id;
        self.next_temp_page_id = self.next_temp_page_id.saturating_add(1);
        id
    }

    /// 空白 default temp id：列表中恒保留、占位名 `default` 的空 temp。
    pub fn blank_temp(&self) -> Option<u64> {
        self.temps
            .iter()
            .find(|temp| temp.is_blank())
            .map(|temp| temp.id)
    }

    /// 确保存在空白 default temp（缺失时新建）并返回其 id。
    pub fn ensure_blank_temp(&mut self) -> u64 {
        self.blank_temp().unwrap_or_else(|| self.new_temp())
    }

    /// 按 id 取临时提示词。
    pub fn temp(&self, id: u64) -> Option<&TempPrompt> {
        self.temps.iter().find(|temp| temp.id == id)
    }

    /// 全部临时提示词（内存引用扫描用）。
    pub fn temps(&self) -> &[TempPrompt] {
        &self.temps
    }

    /// 按 id 取临时提示词（可变）。
    pub fn temp_mut(&mut self, id: u64) -> Option<&mut TempPrompt> {
        self.temps.iter_mut().find(|temp| temp.id == id)
    }

    /// 删除临时提示词；不存在时 no-op；返回是否移除。
    pub fn remove_temp(&mut self, id: u64) -> bool {
        let before = self.temps.len();
        self.temps.retain(|temp| temp.id != id);
        self.temps.len() != before
    }

    /// 回收全部空白临时提示词（`[tmp]` 新建前兜底，保证空白项至多一条）。
    pub fn remove_blank_temps(&mut self) {
        self.temps.retain(|temp| !temp.is_blank());
    }

    /// 离开编辑视图的绑定回收：绑定的临时提示词为空时仅清除绑定
    /// （空白 default temp 保留在列表）；返回是否回收。
    pub fn drop_blank_temp_binding(&mut self) -> bool {
        let Some(PromptEditing::Temp(id)) = self.editing else {
            return false;
        };
        if !self.temp(id).is_some_and(TempPrompt::is_blank) {
            return false;
        }
        self.editing = None;
        true
    }

    /// 可见行：置顶 `tmp:` 区（含空白 default temp）并在有树行时补 `saved:` 标签；
    /// 无可见临时条目时与树行一致（保持无标签原状）。
    pub fn rows(&self) -> Vec<PromptRow> {
        let query = self.search.text().trim().to_lowercase();
        let temp_indices: Vec<usize> = self
            .temps
            .iter()
            .enumerate()
            .filter(|(_, temp)| name_matches(temp.display_name(), &query))
            .map(|(index, _)| index)
            .collect();
        let tree_rows = self.filtered_tree_rows();
        if temp_indices.is_empty() {
            return tree_rows;
        }
        let mut rows = Vec::with_capacity(temp_indices.len() + tree_rows.len() + 2);
        rows.push(PromptRow {
            kind: PromptRowKind::Section(PromptSection::Temp),
            index: 0,
            depth: 0,
        });
        for index in temp_indices {
            rows.push(PromptRow {
                kind: PromptRowKind::Temp,
                index,
                depth: 0,
            });
        }
        if !tree_rows.is_empty() {
            rows.push(PromptRow {
                kind: PromptRowKind::Section(PromptSection::Saved),
                index: 0,
                depth: 0,
            });
            rows.extend(tree_rows);
        }
        rows
    }

    /// 过滤后的树行；无过滤词时与 `tree.rows()` 一致。
    ///
    /// 过滤忽略折叠状态：命中文件夹展示其整棵子树，命中条目展示祖先链。
    /// （临时提示词与分区标签由 `rows` 在树行之前构建。）
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
                        depth: 2,
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

    /// 行数据：按可见行取临时提示词。
    pub fn temp_of_row(&self, row: PromptRow) -> Option<&TempPrompt> {
        (row.kind == PromptRowKind::Temp)
            .then(|| self.temps.get(row.index))
            .flatten()
    }

    /// 选中条目 id；临时行、文件夹行、标签行或空树为 None。
    pub fn selected_prompt_id(&self) -> Option<i64> {
        self.selected_row()
            .and_then(|row| self.tree.item_of_row(row))
            .map(|item| item.id)
    }

    /// 选中行钳制（树变化后调用）：吸附到最近可选中行，滚动同步跟随。
    pub fn clamp_selection(&mut self, visible: usize) {
        let rows = self.rows();
        let len = rows.len();
        self.selected = snap_selectable(&rows, self.selected.min(len.saturating_sub(1)));
        self.scroll = self.scroll.min(len.saturating_sub(visible.max(1)));
        if self.selected < self.scroll {
            self.scroll = self.selected;
        } else if self.selected >= self.scroll + visible.max(1) {
            self.scroll = self.selected + 1 - visible.max(1);
        }
    }
}

/// 吸附到最近的可选中行：优先向下取首个，缺失时向上取最近；全为标签行时归 0。
fn snap_selectable(rows: &[PromptRow], index: usize) -> usize {
    if rows.is_empty() {
        return 0;
    }
    let start = index.min(rows.len() - 1);
    if let Some(offset) = rows[start..].iter().position(|row| row.kind.selectable()) {
        return start + offset;
    }
    rows[..start]
        .iter()
        .rposition(|row| row.kind.selectable())
        .unwrap_or(0)
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
