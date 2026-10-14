//! 提示词面板工作区状态：树视图/编辑视图切换、搜索过滤、临时提示词与可见行钳制。

use std::path::PathBuf;

use crate::app::overlay::TextInput;
use crate::ui::text;

use super::{
    PromptFolder, PromptPageId, PromptRow, PromptRowKind, PromptSection, PromptTree, TempPrompt,
};

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
