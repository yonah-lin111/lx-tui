//! 侧栏工作区历史列表状态：模式切换、Git 树形分组与折叠、搜索过滤、选中与滚动。

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::time::Instant;

use crate::app::overlay::TextInput;
use crate::app::state::WorkspaceGit;

/// 左侧栏上半区（Workspaces）的展示模式。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum WorkspaceMode {
    /// 默认视图：当前打开的活跃工作区列表。
    #[default]
    Active,
    /// 历史视图：持久化的工作区历史列表（含搜索框与 Git 树形结构）。
    History,
}

/// 单条工作区历史记录。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkspaceHistoryEntry {
    pub name: String,
    pub path: PathBuf,
    /// 目标目录当前在磁盘上是否存在（`!exists` 时以异色显示并允许级联删除）。
    pub exists: bool,
    /// 用户是否手动重命名过该工作区。
    pub name_is_manual: bool,
    /// 所属 git checkout 的元数据（主 checkout 或 linked worktree）；非 Git 目录为 None。
    pub git: Option<WorkspaceGit>,
}

impl WorkspaceHistoryEntry {
    /// 历史列表条目展示名：作为分组子项且未手动改名时展示分支短名，否则展示工作区名。
    pub fn display_label(&self, grouped_child: bool) -> String {
        if grouped_child
            && !self.name_is_manual
            && let Some(branch) = self.git.as_ref().and_then(WorkspaceGit::short_branch)
        {
            return branch.to_string();
        }
        self.name.clone()
    }
}

/// 工作区历史列表的一行可见视图项（树形结构 + 搜索过滤结果）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WorkspaceHistoryRow {
    /// 在 `WorkspaceHistoryPanel.entries` 中的原始下标。
    pub index: usize,
    /// 是否为 Git 仓库分组父项（显示 `▾` / `▸` 折叠箭头）。
    pub parent: bool,
    /// 父项所在仓库分组当前是否已折叠。
    pub collapsed: bool,
    /// 是否为 Git 仓库分组子项（显示 `├─` / `└─` 树形连接符）。
    pub child: bool,
    /// 是否为当前可见子项中的最后一项（决定连接符为 `└─` 还是 `├─`）。
    pub last_child: bool,
    /// 同组子项显示标签重复时的去重序号（2、3…）；无重复为 None。
    pub child_index: Option<u32>,
}

/// 工作区历史面板状态。
#[derive(Debug)]
pub struct WorkspaceHistoryPanel {
    pub mode: WorkspaceMode,
    pub entries: Vec<WorkspaceHistoryEntry>,
    /// 已折叠的 Git 仓库根路径集合。
    pub collapsed_repos: Vec<PathBuf>,
    pub search: TextInput,
    /// 当前选中可见行下标（相对于 `visible_rows`）。
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
            collapsed_repos: Vec::new(),
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

    /// 指定仓库根在历史列表中是否处于折叠态。
    pub fn is_repo_collapsed(&self, repo_root: &Path) -> bool {
        self.collapsed_repos
            .iter()
            .any(|collapsed| collapsed.as_path() == repo_root)
    }

    /// 切换指定仓库根的折叠态。
    pub fn toggle_repo_collapsed(&mut self, repo_root: &Path) {
        if let Some(pos) = self
            .collapsed_repos
            .iter()
            .position(|collapsed| collapsed.as_path() == repo_root)
        {
            self.collapsed_repos.remove(pos);
        } else {
            self.collapsed_repos.push(repo_root.to_path_buf());
        }
    }

    /// 计算当前可见的树形行列表（含折叠裁剪与搜索词过滤）。
    pub fn visible_rows(&self) -> Vec<WorkspaceHistoryRow> {
        let mut parent_by_repo: BTreeMap<&Path, usize> = BTreeMap::new();
        let mut children_by_repo: BTreeMap<&Path, Vec<usize>> = BTreeMap::new();

        for (index, entry) in self.entries.iter().enumerate() {
            let Some(git) = entry.git.as_ref() else {
                continue;
            };
            let root = git.repo_root.as_path();
            if git.is_linked {
                children_by_repo.entry(root).or_default().push(index);
            } else {
                parent_by_repo.entry(root).or_insert(index);
            }
        }
        // 仅有主工作区且含至少 1 个 linked worktree 时才构成可折叠树分组。
        parent_by_repo.retain(|root, _| {
            children_by_repo
                .get(root)
                .is_some_and(|children| !children.is_empty())
        });

        let mut child_indices: HashMap<usize, Option<u32>> = HashMap::new();
        let mut child_counts: HashMap<(&Path, &str), u32> = HashMap::new();
        for (&root, children) in &children_by_repo {
            if !parent_by_repo.contains_key(root) {
                continue;
            }
            for &child_idx in children {
                let entry = &self.entries[child_idx];
                let dup = if entry.name_is_manual {
                    None
                } else if let Some(branch) = entry.git.as_ref().and_then(WorkspaceGit::short_branch)
                {
                    let count = child_counts.entry((root, branch)).or_insert(0);
                    *count += 1;
                    (*count > 1).then_some(*count)
                } else {
                    None
                };
                child_indices.insert(child_idx, dup);
            }
        }

        let query = self.search.text().trim().to_lowercase();
        let matches_query = |entry: &WorkspaceHistoryEntry, is_child: bool| -> bool {
            if query.is_empty() {
                return true;
            }
            entry.name.to_lowercase().contains(&query)
                || entry
                    .display_label(is_child)
                    .to_lowercase()
                    .contains(&query)
                || entry.path.to_string_lossy().to_lowercase().contains(&query)
        };

        let mut rows = Vec::with_capacity(self.entries.len());
        for (index, entry) in self.entries.iter().enumerate() {
            if let Some(git) = entry.git.as_ref() {
                let root = git.repo_root.as_path();
                if git.is_linked && parent_by_repo.contains_key(root) {
                    // 成组子项统一挂载在对应父项下方输出，不在顶层重复输出。
                    continue;
                }
                if parent_by_repo.get(root) == Some(&index) {
                    let children = children_by_repo
                        .get(root)
                        .map(Vec::as_slice)
                        .unwrap_or_default();
                    let collapsed = self.is_repo_collapsed(root);
                    if query.is_empty() {
                        rows.push(WorkspaceHistoryRow {
                            index,
                            parent: true,
                            collapsed,
                            child: false,
                            last_child: false,
                            child_index: None,
                        });
                        if !collapsed {
                            for (pos, &child_idx) in children.iter().enumerate() {
                                rows.push(WorkspaceHistoryRow {
                                    index: child_idx,
                                    parent: false,
                                    collapsed: false,
                                    child: true,
                                    last_child: pos + 1 == children.len(),
                                    child_index: child_indices.get(&child_idx).copied().flatten(),
                                });
                            }
                        }
                    } else {
                        let parent_matches = matches_query(entry, false);
                        let matching_children: Vec<usize> = children
                            .iter()
                            .copied()
                            .filter(|&child_idx| matches_query(&self.entries[child_idx], true))
                            .collect();
                        if !parent_matches && matching_children.is_empty() {
                            continue;
                        }
                        let shown_children: &[usize] = if !matching_children.is_empty() {
                            &matching_children
                        } else if !collapsed {
                            children
                        } else {
                            &[]
                        };
                        rows.push(WorkspaceHistoryRow {
                            index,
                            parent: true,
                            collapsed: shown_children.is_empty() && collapsed,
                            child: false,
                            last_child: false,
                            child_index: None,
                        });
                        for (pos, &child_idx) in shown_children.iter().enumerate() {
                            rows.push(WorkspaceHistoryRow {
                                index: child_idx,
                                parent: false,
                                collapsed: false,
                                child: true,
                                last_child: pos + 1 == shown_children.len(),
                                child_index: child_indices.get(&child_idx).copied().flatten(),
                            });
                        }
                    }
                    continue;
                }
            }

            if matches_query(entry, false) {
                rows.push(WorkspaceHistoryRow {
                    index,
                    parent: false,
                    collapsed: false,
                    child: false,
                    last_child: false,
                    child_index: None,
                });
            }
        }
        rows
    }

    /// 过滤后的可见行在 `entries` 中的原始下标列表。
    pub fn filtered_indices(&self) -> Vec<usize> {
        self.visible_rows()
            .into_iter()
            .map(|row| row.index)
            .collect()
    }

    /// 按过滤后的可见行下标取 `(entry_index, &WorkspaceHistoryEntry)`。
    pub fn entry_at_visible_row(
        &self,
        visible_row: usize,
    ) -> Option<(usize, &WorkspaceHistoryEntry)> {
        let entry_index = self.visible_rows().get(visible_row)?.index;
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
        let total = self.visible_rows().len();
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
        let total = self.visible_rows().len();
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
