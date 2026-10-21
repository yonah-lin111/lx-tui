//! 工作区数据模型与工作区列表分组/排序查询。

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};

use crate::app::prompt::{Prompt, PromptPanel};
use crate::layout::PaneId;

use super::AppState;
use super::pane::Tab;

/// 当前路径无最后一段且显示为空时的兜底工作区名。
const FALLBACK_WORKSPACE_NAME: &str = "workspace";

/// 工作区。
#[derive(Debug)]
pub struct Workspace {
    pub name: String,
    pub tabs: Vec<Tab>,
    pub active_tab: usize,
    /// 用户是否手动改过名；为真时 cwd 变化不再自动改名。
    pub name_is_manual: bool,
    /// 驱动命名的窗格 cwd（自动命名跟踪用）。
    pub cwd: Option<PathBuf>,
    /// 是否为启动时创建的工作区；列表项显示不可移除的 `*` 标记。
    pub is_initial: bool,
    /// 所属 git checkout 的元数据；非仓库或尚未查询到时为 None。
    pub git: Option<WorkspaceGit>,
    /// 挂起的 prompt 草稿；None 表示该工作区草稿正被显示
    /// （位于 `AppState.prompt` / `AppState.prompt_root`），全局恒有且仅有一处为 None。
    pub prompt: Option<PromptDraft>,
    /// 提示词树状管理器状态（树、视图模式、选中与滚动）。
    pub prompt_panel: PromptPanel,
}

/// 工作区挂起的 prompt 草稿：编辑器完整状态与显式上下文根绑定。
#[derive(Debug)]
pub struct PromptDraft {
    pub editor: Prompt,
    pub root: Option<PathBuf>,
}

impl PromptDraft {
    pub fn new() -> Self {
        Self {
            editor: Prompt::new(PaneId::alloc()),
            root: None,
        }
    }
}

impl Default for PromptDraft {
    fn default() -> Self {
        Self::new()
    }
}

/// 工作区所属 git checkout 的只读元数据。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkspaceGit {
    /// 仓库主 checkout 路径（同一仓库的分组键）。
    pub repo_root: PathBuf,
    /// 本工作区 cwd 所在 checkout 路径。
    pub checkout_path: PathBuf,
    /// 是否为 linked worktree（非主 checkout）。
    pub is_linked: bool,
    /// 当前分支短名；detached 或 bare 为 None。
    pub branch: Option<String>,
    /// 仓库主 checkout 的分支短名；linked worktree 的状态栏展示用。
    pub main_branch: Option<String>,
}

impl WorkspaceGit {
    /// 展示用分支名：去掉 `worktree/` 前缀；detached 或 bare 为 None。
    pub fn short_branch(&self) -> Option<&str> {
        self.branch
            .as_deref()
            .map(|branch| branch.strip_prefix("worktree/").unwrap_or(branch))
    }

    /// 状态栏展示分支：linked worktree 取主 checkout 分支（回退本 checkout 分支），
    /// 主 checkout 取自身分支；去掉 `worktree/` 前缀。
    pub fn status_branch(&self) -> Option<&str> {
        if self.is_linked
            && let Some(main) = self.main_branch.as_deref()
        {
            return Some(main.strip_prefix("worktree/").unwrap_or(main));
        }
        self.short_branch()
    }
}

/// 侧栏工作区列表的可见行；折叠的组只保留父项与当前激活子项。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WorkspaceRow {
    /// 工作区索引。
    pub index: usize,
    /// 分组子项：缩进并显示分支短名。
    pub child: bool,
    /// 分组父项：显示折叠箭头。
    pub parent: bool,
    /// 父项所在组是否已折叠。
    pub collapsed: bool,
    /// 子项标签与组内更早子项重复时的去重序号（2、3…）；无重复为 None。
    pub child_index: Option<u32>,
}

impl Workspace {
    /// 单窗格终端工作区；新建工作区使用，初始为自动命名。
    pub(crate) fn single_terminal(name: String, cwd: Option<PathBuf>) -> Self {
        Self {
            name,
            tabs: vec![Tab::single_terminal()],
            active_tab: 0,
            name_is_manual: false,
            cwd,
            is_initial: false,
            git: None,
            prompt: Some(PromptDraft::new()),
            prompt_panel: PromptPanel::new(),
        }
    }

    /// 身份窗格：第一个标签的根窗格，工作区命名跟随它的 cwd。
    pub fn root_pane(&self) -> Option<PaneId> {
        self.tabs.first().and_then(|tab| tab.root_pane())
    }

    /// 仓库级持久化键：git 工作区取仓库主 checkout（同一仓库的主 checkout 与
    /// linked worktree 共享一棵树），其余取工作区 cwd。
    ///
    /// prompt 面板、提示词与 chat 会话共用此键；agent 执行 cwd 另用
    /// [`AppState::workspace_cwd_for_pane`]，不得混用。
    pub fn repo_key(&self) -> Option<&Path> {
        self.git
            .as_ref()
            .map(|git| git.repo_root.as_path())
            .or(self.cwd.as_deref())
    }

    /// 边框身份标签的分段数据：项目名必填，分支与工作区名按可用信息可选。
    pub fn identity(&self) -> WorkspaceIdentity {
        let project = self
            .git
            .as_ref()
            .map(|git| git.repo_root.as_path())
            .or(self.cwd.as_deref())
            .and_then(Path::file_name)
            .and_then(|name| name.to_str())
            .filter(|name| !name.is_empty())
            .map(str::to_string)
            .unwrap_or_else(|| self.name.clone());
        let branch = self
            .git
            .as_ref()
            .and_then(WorkspaceGit::status_branch)
            .map(str::to_string);
        // 仅 linked worktree 有"工作区名"这一段；主 checkout 自己就是仓库。
        let worktree = self
            .git
            .as_ref()
            .filter(|git| git.is_linked)
            .map(|_| self.name.clone());
        WorkspaceIdentity {
            project,
            branch,
            worktree,
        }
    }
}

/// 边框身份标签分段：`项目 · b:分支 · wt:工作区名`，缺项为 None。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkspaceIdentity {
    pub project: String,
    pub branch: Option<String>,
    pub worktree: Option<String>,
}

/// 自动工作区名：HOME 显示 `~`，否则取路径末段；根路径等无末段时显示完整路径。
///
/// 与 herdr 的 `fallback_label_from_cwd` 对齐（HOME 特判 + file_name 回退 display）。
pub fn workspace_label(cwd: &Path, home: Option<&Path>) -> String {
    if home.is_some_and(|home| cwd == home) {
        return "~".to_string();
    }
    cwd.file_name()
        .and_then(|name| name.to_str())
        .filter(|name| !name.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| cwd.display().to_string())
}

/// 工作区自动名：当前进程工作目录的标签；读不到 cwd 时回退 `workspace`。
pub fn workspace_name() -> String {
    current_workspace_identity().1
}

/// 路径末两段（`a/b`），用于 lx 窗格顶边框右侧 `ws:` 值。
///
/// 不足两段时用现有的全部段；无可用段（根路径等）时回退完整路径显示。
pub fn path_tail_two(path: &Path) -> String {
    let mut segments: Vec<&str> = path
        .components()
        .filter_map(|component| match component {
            std::path::Component::Normal(name) => name.to_str(),
            _ => None,
        })
        .filter(|name| !name.is_empty())
        .collect();
    let tail = segments.split_off(segments.len().saturating_sub(2));
    if tail.is_empty() {
        return path.display().to_string();
    }
    tail.join("/")
}

/// 当前进程 cwd 与其自动工作区名；读不到 cwd 时 cwd 为 None。
pub fn current_workspace_identity() -> (Option<PathBuf>, String) {
    match std::env::current_dir() {
        Ok(path) => {
            let name = workspace_label(&path, home_dir().as_deref());
            (Some(path), name)
        }
        // 读不到 cwd 属于极端环境问题，工作区名兜底即可，不阻断启动。
        Err(_) => (None, FALLBACK_WORKSPACE_NAME.to_string()),
    }
}

/// HOME 环境变量；缺失或为空时 None。
pub fn home_dir() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .filter(|home| !home.is_empty())
        .map(PathBuf::from)
}

/// 标签标题：重命名后显示自定义名，否则按位置显示 `tab N`。
pub fn tab_label(index: usize, name: Option<&str>) -> String {
    match name {
        Some(name) if !name.trim().is_empty() => name.to_string(),
        _ => format!("tab {}", index.saturating_add(1)),
    }
}

/// 去重命名：`base` 已被占用时追加最小未用序号（`base 2`、`base 3`…）。
///
/// `taken` 由调用方给出占用判定（排除自身）。
pub fn unique_workspace_name(base: &str, taken: impl Fn(&str) -> bool) -> String {
    if !taken(base) {
        return base.to_string();
    }
    let mut suffix: u32 = 2;
    loop {
        let candidate = format!("{base} {suffix}");
        if !taken(&candidate) {
            return candidate;
        }
        suffix = suffix.saturating_add(1);
    }
}

impl AppState {
    /// 侧栏可见行：同仓库 ≥1 个 linked worktree 且含非 linked 主项时成组，
    /// 根之后（索引更大）的 linked 项为子项；重复主 checkout 与非组项为独立顶层行。
    /// 折叠的组只保留父项与当前激活子项。
    ///
    /// linked 子项标签与组内更早子项重复时给出显示去重序号（含被折叠隐藏的子项，序号稳定）。
    /// 列表保持工作区原始顺序，不做父项前置重排；渲染、命中与滚动共用此结果。
    pub fn workspace_rows(&self) -> Vec<WorkspaceRow> {
        let parents = self.group_parents();
        let collapsed = |key: &Path| {
            self.collapsed_groups
                .iter()
                .any(|group| group.as_path() == key)
        };
        let mut child_counts: HashMap<(&Path, &str), u32> = HashMap::new();
        let mut rows = Vec::with_capacity(self.workspaces.len());
        for (index, workspace) in self.workspaces.iter().enumerate() {
            let git = workspace.git.as_ref();
            let group = git.and_then(|git| {
                let parent = parents.get(git.repo_root.as_path())?;
                Some((git.repo_root.as_path(), *parent))
            });
            match group {
                Some((key, parent)) if parent == index => {
                    rows.push(WorkspaceRow {
                        index,
                        child: false,
                        parent: true,
                        collapsed: collapsed(key),
                        child_index: None,
                    });
                }
                Some((key, parent)) if index > parent && git.is_some_and(|git| git.is_linked) => {
                    let child_index = if workspace.name_is_manual {
                        None
                    } else {
                        match git.and_then(WorkspaceGit::short_branch) {
                            Some(label) => {
                                let count = child_counts.entry((key, label)).or_insert(0);
                                *count += 1;
                                (*count > 1).then_some(*count)
                            }
                            None => None,
                        }
                    };
                    if !collapsed(key) || self.active_workspace == index {
                        rows.push(WorkspaceRow {
                            index,
                            child: true,
                            parent: false,
                            collapsed: false,
                            child_index,
                        });
                    }
                }
                _ => rows.push(WorkspaceRow {
                    index,
                    child: false,
                    parent: false,
                    collapsed: false,
                    child_index: None,
                }),
            }
        }
        rows
    }

    /// 已成立分组的根索引：仓库根 → 创建最早的非 linked 主项（根窗格标识随创建递增，
    /// 拖拽换位不改变）；无主项或无 linked worktree 成员时不成组（重复主 checkout 平铺）。
    fn group_parents(&self) -> BTreeMap<&Path, usize> {
        let mut members: BTreeMap<&Path, Vec<usize>> = BTreeMap::new();
        for (index, workspace) in self.workspaces.iter().enumerate() {
            if let Some(git) = workspace.git.as_ref() {
                members
                    .entry(git.repo_root.as_path())
                    .or_default()
                    .push(index);
            }
        }
        members
            .into_iter()
            .filter_map(|(key, indices)| {
                if !indices
                    .iter()
                    .any(|index| self.workspace_git(*index).is_some_and(|git| git.is_linked))
                {
                    return None;
                }
                let parent = indices
                    .iter()
                    .copied()
                    .filter(|index| self.workspace_git(*index).is_some_and(|git| !git.is_linked))
                    .min_by_key(|index| self.workspace_created_key(*index))?;
                Some((key, parent))
            })
            .collect()
    }

    /// 工作区 git 元数据。
    fn workspace_git(&self, index: usize) -> Option<&WorkspaceGit> {
        self.workspaces
            .get(index)
            .and_then(|workspace| workspace.git.as_ref())
    }

    /// 创建序键：根窗格标识随创建单调递增；无窗格时排最后。
    fn workspace_created_key(&self, index: usize) -> u32 {
        self.workspaces
            .get(index)
            .and_then(Workspace::root_pane)
            .map(PaneId::raw)
            .unwrap_or(u32::MAX)
    }

    /// 拖拽块：树的根与 linked 子项整体移动；重复主 checkout 等独立项返回自身。
    ///
    /// 折叠隐藏的子项在块内；根按创建序固定，拖拽换位不会改变树的归属。
    pub fn workspace_block(&self, index: usize) -> Vec<usize> {
        let Some(git) = self.workspace_git(index) else {
            return vec![index];
        };
        let Some(parent) = self.group_parents().get(git.repo_root.as_path()).copied() else {
            return vec![index];
        };
        if index != parent && !git.is_linked {
            return vec![index];
        }
        let block: Vec<usize> = self
            .workspaces
            .iter()
            .enumerate()
            .filter(|(other, workspace)| {
                workspace.git.as_ref().is_some_and(|other_git| {
                    other_git.repo_root == git.repo_root
                        && (*other == parent || (*other > parent && other_git.is_linked))
                })
            })
            .map(|(other, _)| other)
            .collect();
        if block.contains(&index) {
            block
        } else {
            vec![index]
        }
    }
}
