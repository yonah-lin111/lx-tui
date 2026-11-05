# 左侧栏工作区历史列表（Workspace History List）— 详细设计 (`design.md`)

- **特性目录**：`docs/feature/workspace-history-list/`
- **关联任务清单**：`docs/feature/workspace-history-list/task.md`
- **当前状态**：**已在 `.worktrees/workspace-history-list` (`feature/workspace-history-list`) 完成实现与测试，待用户确认合并**
- **目标范围**：
  - `src/db/`（`mod.rs`、`workspace.rs`、`service.rs`）
  - `src/app/`（`state.rs`、`state/workspace.rs`、`state/workspace_history.rs`、`effects.rs`、`overlay.rs`、`tooltip.rs`、`actions.rs`、`update/mod.rs`、`update/workspace.rs`、`update/workspace_history.rs`、`update/menu.rs`、`update/dialog.rs`、`update/worktree.rs`）
  - `src/input/mod.rs`
  - `src/event/`（`input.rs`、`overlay.rs`、`mouse/press.rs`、`mouse/drag.rs`、`mouse/wheel.rs`、`mouse/hover.rs`、`mouse/hit.rs`）
  - `src/ui/`（`mod.rs`、`sidebar.rs`、`sidebar/history.rs`、`overlay/menu.rs`、`style.rs`、`text.rs`、`tooltip.rs`）
- **架构约束**：
  - 严格保持 `app/`（纯状态与更新，禁止 IO）、`ui/`（纯渲染只读状态，禁止修改状态或执行 IO）、`db/`（唯一 rusqlite 持久层）边界清晰；
  - 所有 TUI 用户可见文案与 Toast 一律使用英文并集中定义在 `src/ui/text.rs`；
  - 生产代码单文件行数上限 1000 行（针对已接近上限的 `src/db/mod.rs` [950 行] 与 `src/event/mouse/press.rs` [992 行]，按职责拆分子模块），禁止 `unwrap()` / `expect()`。

---

## 0. 现状核实与事实基线（CodeGraph + Codebase Memory 实测）

| 维度 | 现状实测（带文件与行号） | 对本特性的设计影响 |
| --- | --- | --- |
| **侧栏顶部按钮几何** | `src/ui/sidebar.rs:247-276`：`add_workspace_button` 位于侧栏顶边框右端 `x = sidebar.right() - 4, y = sidebar.y, width = 3`（文案 `ADD_WORKSPACE_LABEL = "[+]"`，样式 `style::accent()`）。 | 历史列表按钮放在 `[+]` 左侧並留 1 列边框间隔：`x = add_button.x - 1 - 3 = sidebar.right() - 8, y = sidebar.y, width = 3`。空间不足或侧栏折叠（`width <= COLLAPSED_STRIP`）时隐藏。 |
| **侧栏上下分区结构** | `src/ui/layout.rs:90-137`：`sidebar_sections(sidebar, agents_collapsed)` 将侧栏内部划分为上半区 `workspaces`、中间 `divider`（`Agents` 表头）、下半区 `agents`。 | 切换历史列表模式时，**仅原位替换上半区 `sections.workspaces`**，下半区 `Agents` 分割线与列表完全保持不变。 |
| **右侧 Prompt 列表图标与切换交互** | `src/ui/text.rs:102-103`：`PROMPT_MODE_LIST_LABEL = "[≡]"`（默认视图显示）、`PROMPT_MODE_EDIT_LABEL = "[/]"`（列表视图显示）。`src/ui/prompt/tree.rs:19-122`：列表视图顶部第 1 行为 `> Search...` 搜索框，第 2 行为 `├────┤` 分割线，第 3 行起为条目列表（行首 `❯ ` 选中标记 + `• ` 图标 + 名称）。 | 直接复用 `text::PROMPT_MODE_LIST_LABEL` (`[≡]`) 与 `text::PROMPT_MODE_EDIT_LABEL` (`[/]`)；上半区 `workspaces` 切换到历史列表时，同样采用「第 1 行搜索框 + 第 2 行分割线 + 第 3 行起扁平条目列表」布局。 |
| **双击打开判定机制** | `src/app/update/prompt/tree_view.rs:129-135`：`PROMPT_DOUBLE_CLICK_WINDOW = Duration::from_millis(400)`，记录按下时刻 `(row, pressed_at)`，在鼠标松开（`MouseEventKind::Up`）且未发生拖拽时判定同一可见行 400ms 内二次点击触发 `open`。 | 历史列表条目采用完全一致的 400ms 同行双击判定（以及键盘 `Enter` 激活），触发打开或切换到目标工作区，并自动切回默认活跃工作区视图。 |
| **工作区持久化与外键级联** | `src/db/migration/schema.rs:5-14`：`workspace` 表含 `id`, `external_id UNIQUE`, `name`, `path UNIQUE`, `is_trusted`, `created_at`, `updated_at`。所有关联子表：`workspace_folder` (`:24`)、`workspace_prompt` (`:41`)、`workspace_referenced_folder` (`:53`)、`agent_session` (`:408`) 及其全部 12 张聊天子表均声明了 `ON DELETE CASCADE`，且运行时 `PRAGMA foreign_keys = ON`（`src/db/mod.rs:270, 280`）。 | 1. 无需递增 `SCHEMA_VERSION`（当前 v22 表结构已完备）；<br>2. 修正 `ensure_workspace` 为 `ON CONFLICT(path) DO NOTHING`，防止后台保存提示词/会话时用目录末段名覆盖用户手动修改过的 `workspace.name`；<br>3. 对路径不存在的失效条目执行 `DELETE FROM workspace WHERE path = ?1` 即可由 SQLite 外键自动级联清理全部关联提示词、文件夹、引用目录和聊天会话。 |
| **侧栏焦点与键盘路由** | `src/input/mod.rs:157-160` 与 `src/event/input.rs:255-257`：当前 `FocusedContainer::Sidebar` 在路由层无条件返回 `None` 吞掉所有按键与粘贴。 | 侧栏处于 `WorkspaceMode::History` 时，放行搜索输入、上下移动（`Up`/`Down`）、回车打开（`Enter`）、退出列表（`Esc`）、复制/清空搜索（`Ctrl/Cmd+C`）、剪切（`Ctrl/Cmd+X`）及粘贴；处于 `WorkspaceMode::Active` 时维持原样吞掉按键。 |
| **单文件行数水位** | `src/db/mod.rs` 已达 950 行；`src/event/mouse/press.rs` 已达 992 行；`src/ui/sidebar.rs` 为 615 行。 | 遵循 1000 行红线：将工作区 DB 读写抽入 `src/db/workspace.rs`；将侧栏历史列表 UI 抽入 `src/ui/sidebar/history.rs`；将侧栏鼠标按下分支抽入 `src/event/mouse/hit.rs`，确保所有生产文件远低于 1000 行。 |

---

## 1. 背景与目标 / 非目标

### 1.1 目标
1. **新增工作区历史列表切换按钮**：在左侧栏 `Workspaces` 顶边框右上角（`[+]` 按钮左侧，间隔 1 列）增加列表按钮，默认显示 `[≡]`，点击打开工作区历史列表并将图标切换为 `[/]`。
2. **工作区历史自动记录与持久化**：凡是显式创建或打开过的工作区（启动初始工作区、`[+]` 新建、从 Worktree 打开、从历史列表打开）均按规范化路径 `path` 去重记录到 SQLite `workspace` 表，按最近打开时间（`updated_at DESC, id DESC`）排序；终端内执行 `cd` 切换子目录不产生新历史记录。
3. **对齐 Prompt List 的交互体验**：
   - 在左侧栏上半区 `Workspaces` 原位渲染「顶部搜索框 + 分割线 + 扁平历史列表 + 滚动条」，下半区 `Agents` 不变；
   - 支持实时搜索过滤（匹配名称或路径，不区分大小写）、键盘上下选择、回车/双击打开、鼠标悬停高亮；
   - 支持新建工作区（Create）、重命名工作区并双向同步（Update/Rename）、双击或右键打开（Open）。
4. **失效路径高亮与级联删除**：
   - 当历史工作区的目录路径在磁盘上已不存在（`!path.is_dir()`）时，该条目使用错误/危险语义色（`style::error()`，红色）渲染；
   - 仅针对此类路径不存在的失效条目，右键菜单额外提供 `Delete`（带行内二次确认 `Confirm delete`），确认后从数据库删除该工作区并直接级联删除其关联的所有提示词、文件夹、引用目录及聊天会话数据。

### 1.2 非目标
- 历史列表不引入文件夹层级或 `tmp:` / `saved:` 分区，也不支持拖拽排序（固定按最近打开时间倒序）。
- 路径仍然存在的正常历史条目不提供删除按钮，防止用户误删仍有磁盘项目的历史记录及关联资产。

---

## 2. 已确认的边界决策汇总（Grill Me 结论表）

| 编号 | 决策维度 | 已确认方案 |
| --- | --- | --- |
| **Q1** | **数据来源与生命周期** | 凡是显式创建或打开过的工作区自动持久化到 SQLite `workspace` 表（按最近打开时间倒序）。在当前活跃列表中执行 `Close Workspace` 仅从内存 `AppState.workspaces` 移除并结束终端进程，保留历史记录。 |
| **Q2 / Q3** | **常规历史条目的 CRUD 边界** | 常规历史条目（磁盘路径存在）支持：**查看/搜索（Read）**、**新建（Create，在 `ws list` 历史视图下新建后仅写入历史列表并自动选中该项，保持停留在 `ws list` 而不直接打开工作区）**、**重命名（Update）**、**双击/回车/右键打开（Open）**，不提供删除功能。 |
| **Q4** | **按钮位置、图标与视图切换** | 顶边框右端顺序为 `[+] [≡]`（列表按钮 `[≡]`/`[/]` 居右端，新建按钮 `[+]` 紧邻其左侧 1 列）。默认显示 `[≡]`，点击后上半区 `Workspaces` 原地切换为历史列表，图标变为 `[/]`（下半区 `Agents` 不变）。点击 `[/]`、按 `Esc`、或双击/回车成功打开历史工作区后，自动切回活跃工作区列表。 |
| **Q5** | **列表结构、Git 树形与搜索过滤** | 顶部为 `> search workspaces…` 搜索框 + `├────┤` 分割线；移除 `⑂` 图标，Git 仓库自动以主工作区（`repo_root`）为父节点（`▾/▸ <主工作区名>`，支持点击箭头折叠/展开），下挂 `git worktree list` 的全部工作区及数据库中的历史 worktree（`├─/└─ <分支短名或自定义名>`），独立工作区直接显示 `<name>`。搜索框实时不区分大小写匹配工作区 `name`、分支短名与 `path`，无结果时显示 `no matching workspaces`（空库显示 `no workspaces yet`）。 |
| **Q6** | **命名污染修复、路径去重、重命名同步与失效路径级联删除** | 1. **主工作区命名修复与自愈**：修复 `startup_trust` 在 linked worktree 启动时将子工作区名误写入主工作区路径的 Bug，并在加载历史列表时对已污染的主工作区名称自动自愈回主工作区目录名（如 `lx-tui`）。<br>2. **按路径去重**：以规范化路径 `path` 唯一去重，重复打开仅刷新 `updated_at`；终端内 `cd` 不写入历史列表。<br>3. **重命名双向同步**：在活跃列表或历史列表重命名工作区均写入 `workspace.name`，并同步更新另一侧显示名；重新打开时保留自定义名称。<br>4. **双击打开防重与父项自动补齐**：若目标路径已在 `AppState.workspaces` 中打开则直接切换过去；若打开的是 linked worktree，自动调用 `ensure_main_workspace` 补开主工作区父项并退出历史视图。<br>5. **失效路径特殊颜色与级联删除**：若历史条目的 `path` 在磁盘上不存在（`!path.is_dir()`），条目文字使用 `style::error()`（红色）展示；双击打开时弹 Danger Toast `Directory does not exist` 并留在历史列表；右键菜单对该类失效条目开放 `Delete`（行内二次确认 `Confirm delete`），确认后直接删除 `workspace` 行并级联清理全部关联表数据。 |

---

## 3. 核心架构与数据流

```text
┌────────────────────────────────────────────────────────────────────────────────────┐
│ 1. 顶部按钮与模式切换链路                                                          │
│   ui::sidebar::workspace_mode_button(view)  ──► 顶边框 [≡] / [/] (位于 [+] 左侧 1列)│
│   event::mouse::press::handle_mouse_down_left()                                    │
│     └─ update::toggle_workspace_mode(state)                                        │
│          ├─ Active -> History:                                                     │
│          │    mode = WorkspaceMode::History, focus_sidebar(state),                 │
│          │    刷新各条目 path.is_dir() 状态 + Effect::LoadWorkspaceHistory          │
│          └─ History -> Active:                                                     │
│               mode = WorkspaceMode::Active, 清空搜索选区与悬停                      │
├────────────────────────────────────────────────────────────────────────────────────┤
│ 2. 历史列表渲染与键盘/鼠标交互链路 (WorkspaceMode::History)                        │
│   ui::sidebar::history::render(frame, sections.workspaces, state)                  │
│     ├─ Row 0: `> Search...` (TextInput + 硬件光标，当 sidebar_focused 时显示)      │
│     ├─ Row 1: `├────────────┤` (衔接侧栏左右边框)                                  │
│     └─ Row 2..N: `❯ • <name>` (exists=true 用 style::text(), false 用 style::error)│
│                                                                                    │
│   键盘路由 (FocusedContainer::Sidebar + WorkspaceMode::History):                   │
│     input::route() ──► Routed::Sidebar(SidebarKey)                                 │
│       ├─ Up / Down   ──► update::move_workspace_history_selection(state, ±1, rows) │
│       ├─ Enter       ──► update::activate_workspace_history_selected(state)        │
│       ├─ Esc         ──► update::exit_workspace_history(state)                     │
│       ├─ Ctrl/Cmd+C  ──► 有选区复制选区，无选区清空搜索词                          │
│       ├─ Ctrl/Cmd+X  ──► 剪切搜索框选区                                            │
│       └─ 字符/编辑键 ──► update::apply_workspace_history_search(state, key, rows)  │
│                                                                                    │
│   鼠标点击/双击 (MouseDown -> 记录 row + pressed_at -> MouseUp):                   │
│     400ms 内同行第二次松开 ──► update::open_workspace_history_row(state, row)      │
│       ├─ !path.is_dir() ──► 刷新 exists=false，弹 Danger Toast "Directory not found"│
│       ├─ 已在 state.workspaces 打开 ──► switch_workspace(idx) + 切回 Active 视图   │
│       └─ 未打开 ──► 按保存的 (path, name) 打开工作区 + Effect::RecordWorkspace      │
├────────────────────────────────────────────────────────────────────────────────────┤
│ 3. 历史条目右键菜单与 CRUD 链路                                                    │
│   event::overlay::open_right_click_menu()                                          │
│     ├─ 命中历史条目: OverlayTarget::WorkspaceHistory(index)                        │
│     │    ├─ path 存在:   [OpenWorkspaceHistory, RenameWorkspace]                   │
│     │    └─ path 不存在: [OpenWorkspaceHistory, RenameWorkspace, DeleteWorkspace]  │
│     └─ 命中历史空白: OverlayTarget::WorkspaceList -> [NewWorkspace]                │
│                                                                                    │
│   activate_menu():                                                                 │
│     ├─ OpenWorkspaceHistory ──► open_workspace_history_entry(state, index)         │
│     ├─ RenameWorkspace      ──► Overlay::Rename(RenameTarget::WorkspaceHistory(i)) │
│     │    └─ commit_rename() ──► 更新 history + 同步已打开同路径 Workspace.name      │
│     │                           + Effect::RenameWorkspace { path, name }           │
│     └─ DeleteWorkspace      ──► requires_confirm() 行内二次确认 "Confirm delete"   │
│          └─ 确认后 ──► 从 history 移除 + Effect::DeleteWorkspaceHistory { path }   │
│                        └─ SQLite `DELETE FROM workspace WHERE path = ?1` (CASCADE) │
└────────────────────────────────────────────────────────────────────────────────────┘
```

---

## 4. 详细技术设计与关键代码

### 4.1 状态模型设计（`src/app/state/workspace_history.rs` & `src/app/state.rs`）

#### 关键点
1. 新建 `src/app/state/workspace_history.rs`，定义：
   - `WorkspaceMode`：枚举 `Active`（默认，展示当前打开的工作区树）与 `History`（展示历史工作区列表）。
   - `WorkspaceHistoryEntry`：单条工作区历史记录，包含 `name: String`、`path: PathBuf`、`exists: bool`（磁盘目录是否存在）。
   - `WorkspaceHistoryPanel`：管理历史列表视图的状态（`mode`、`entries`、`search: TextInput`、`selected: usize`、`scroll: usize`、`hover: Option<usize>`、`scroll_drag: Option<u16>`、`row_click: Option<(usize, Instant)>`、`loaded: bool`）。
2. 提供过滤与行定位方法：
   - `filtered_indices(&self) -> Vec<usize>`：当 `search.text().trim()` 为空时返回 `0..entries.len()`；非空时按不区分大小写匹配 `entry.name` 或 `entry.path.to_string_lossy()`。
   - `refresh_existence(&mut self)`：遍历 `entries` 更新 `entry.exists = entry.path.is_dir()`。

#### 关键代码（`src/app/state/workspace_history.rs`）

```rust
//! 侧栏工作区历史列表状态：模式切换、搜索过滤、选中与滚动。

use std::path::PathBuf;
use std::time::Instant;

use crate::app::text_input::TextInput;

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

    /// 刷新每条记录的路径存在性（在进入历史视图或加载列表时调用）。
    pub fn refresh_existence(&mut self) {
        for entry in &mut self.entries {
            entry.exists = entry.path.is_dir();
        }
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
```

---

### 4.2 数据库层与持久化效应（`src/db/workspace.rs`、`src/app/effects.rs`、`src/db/service.rs`）

#### 关键点
1. **拆分 `src/db/workspace.rs`**：
   - 将 `src/db/mod.rs` 中原有的 `is_trusted`、`trust_workspace`、`ensure_workspace`、`ensure_workspace_row`、`workspace_external_id` 迁入 `src/db/workspace.rs`，使 `src/db/mod.rs` 行数从 950 行降至约 890 行。
   - **修正 `ensure_workspace` 覆盖自定义名称的问题**：
     - `ensure_workspace(path)`（作为保存 prompt / folder / chat session 的前置检查）改为 `ON CONFLICT(path) DO NOTHING`，绝不覆盖用户已保存的 `workspace.name`，也不篡改工作区打开时间。
   - **新增 `record_workspace_open(path, default_name)`**：
     - 显式创建/打开工作区时调用。若 `path` 不存在则插入 `default_name`；若 `path` 已存在则只刷新 `updated_at = datetime('now')`、**保留数据库中已有的 `workspace.name`**，并返回数据库中最终生效的 `name: String`（以便自定义过名称的工作区在重新打开时自动恢复其自定义名称）。
   - **新增 `rename_workspace(path, name)`**：
     - 显式重命名工作区时调用，`ON CONFLICT(path) DO UPDATE SET name = excluded.name`。
   - **新增 `list_workspaces()`**：
     - 查询 `SELECT name, path FROM workspace WHERE path IS NOT NULL AND path != '' ORDER BY updated_at DESC, id DESC`，返回 `Vec<WorkspaceRecord>`。
   - **新增 `delete_workspace(path)`**：
     - 先在事务内清理该工作区下的子 Agent 会话（或直接利用 `agent_session.workspace_id REFERENCES workspace(external_id) ON DELETE CASCADE`），执行 `DELETE FROM workspace WHERE path = ?1`，由 SQLite 外键级联删除 `workspace_folder`、`workspace_prompt`、`workspace_prompt_page`、`workspace_referenced_folder`、`agent_session` 及全部聊天子表。
2. **新增 `Effect` 变体（`src/app/effects.rs`）**：
   - `RecordWorkspace { path: PathBuf, name: String }`：记录工作区打开并刷新历史列表；
   - `RenameWorkspace { path: PathBuf, name: String }`：持久化工作区重命名并刷新历史列表；
   - `DeleteWorkspaceHistory { path: PathBuf }`：级联删除失效路径工作区并刷新历史列表；
   - `LoadWorkspaceHistory`：重新从数据库加载工作区历史列表。

#### 关键代码（`src/db/workspace.rs`）

```rust
//! 工作区表（`workspace`）读写：信任状态、历史记录、重命名与级联删除。

use std::path::{Path, PathBuf};

use rusqlite::{OptionalExtension, params};

use super::{Database, DbError, Result, new_external_id, path_string};

/// 数据库中的工作区历史记录。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkspaceRecord {
    pub name: String,
    pub path: PathBuf,
}

impl Database {
    /// 路径是否已信任（`is_trusted = 1`）。
    pub fn is_trusted(&self, path: &Path) -> Result<bool> {
        let trusted: i64 = self
            .conn
            .query_row(
                "SELECT is_trusted FROM workspace WHERE path = ?1",
                params![path_string(path)],
                |row| row.get(0),
            )
            .optional()?
            .unwrap_or(0);
        Ok(trusted == 1)
    }

    /// 记录工作区信任：按 path upsert `is_trusted = 1`，已存在时保留原自定义名称。
    pub fn trust_workspace(&self, path: &Path, name: &str) -> Result<()> {
        self.conn.execute(
            r#"INSERT INTO workspace (external_id, name, "type", path, is_trusted, created_at, updated_at)
               VALUES (?1, ?2, 'filesystem', ?3, 1, datetime('now'), datetime('now'))
               ON CONFLICT(path) DO UPDATE SET
                   is_trusted = 1,
                   updated_at = datetime('now')"#,
            params![new_external_id("ws"), name, path_string(path)],
        )?;
        Ok(())
    }

    /// 确保工作区行存在（提示词/会话读写前置）；已存在时不覆盖名称、不修改信任位与打开时间。
    pub fn ensure_workspace(&self, path: &Path) -> Result<()> {
        let name = path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| path_string(path));
        self.conn.execute(
            r#"INSERT INTO workspace (external_id, name, "type", path, is_trusted, created_at, updated_at)
               VALUES (?1, ?2, 'filesystem', ?3, 0, datetime('now'), datetime('now'))
               ON CONFLICT(path) DO NOTHING"#,
            params![new_external_id("ws"), name, path_string(path)],
        )?;
        Ok(())
    }

    /// 记录工作区显式创建/打开：新路径写入 `default_name`，已有路径保留数据库中的名称并刷新 `updated_at`；
    /// 返回数据库中保存的名称（供打开时恢复自定义名称）。
    pub fn record_workspace_open(&self, path: &Path, default_name: &str) -> Result<String> {
        let path_str = path_string(path);
        self.conn.execute(
            r#"INSERT INTO workspace (external_id, name, "type", path, is_trusted, created_at, updated_at)
               VALUES (?1, ?2, 'filesystem', ?3, 1, datetime('now'), datetime('now'))
               ON CONFLICT(path) DO UPDATE SET
                   updated_at = datetime('now')"#,
            params![new_external_id("ws"), default_name, path_str],
        )?;
        let saved_name: String = self.conn.query_row(
            "SELECT name FROM workspace WHERE path = ?1",
            params![path_str],
            |row| row.get(0),
        )?;
        Ok(saved_name)
    }

    /// 显式重命名工作区：按 path upsert 新名称。
    pub fn rename_workspace(&self, path: &Path, name: &str) -> Result<()> {
        self.conn.execute(
            r#"INSERT INTO workspace (external_id, name, "type", path, is_trusted, created_at, updated_at)
               VALUES (?1, ?2, 'filesystem', ?3, 1, datetime('now'), datetime('now'))
               ON CONFLICT(path) DO UPDATE SET
                   name = excluded.name"#,
            params![new_external_id("ws"), name, path_string(path)],
        )?;
        Ok(())
    }

    /// 按最近打开时间倒序返回全部工作区历史记录。
    pub fn list_workspaces(&self) -> Result<Vec<WorkspaceRecord>> {
        let mut statement = self.conn.prepare(
            r#"SELECT name, path
               FROM workspace
               WHERE path IS NOT NULL AND path != ''
               ORDER BY updated_at DESC, id DESC"#,
        )?;
        let rows = statement.query_map([], |row| {
            let name: String = row.get(0)?;
            let raw_path: String = row.get(1)?;
            Ok(WorkspaceRecord {
                name,
                path: PathBuf::from(raw_path),
            })
        })?;
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .map_err(DbError::from)
    }

    /// 删除指定路径的工作区行；外键 `ON DELETE CASCADE` 自动级联删除该工作区下的
    /// 文件夹、提示词、分页、引用目录与全部聊天会话及子表数据。
    pub fn delete_workspace(&self, path: &Path) -> Result<()> {
        self.conn.execute(
            "DELETE FROM workspace WHERE path = ?1",
            params![path_string(path)],
        )?;
        Ok(())
    }

    /// 工作区外部标识；未登记时 None。
    pub(crate) fn workspace_external_id(&self, path: &Path) -> Result<Option<String>> {
        Ok(self
            .conn
            .query_row(
                "SELECT external_id FROM workspace WHERE path = ?1",
                params![path_string(path)],
                |row| row.get(0),
            )
            .optional()?)
    }
}
```

---

### 4.3 侧栏右上角 `[≡]` / `[/]` 按钮与历史列表渲染（`src/ui/sidebar.rs` & `src/ui/sidebar/history.rs`）

#### 关键点
1. **顶部按钮几何与绘制（`src/ui/sidebar.rs`）**：
   - 新增 `pub fn workspace_mode_button(view: &layout::ViewLayout) -> Option<Rect>`：
     - 先取 `add = add_button_area(view.sidebar)?`（占 `sidebar.right() - 4 .. sidebar.right() - 1`）；
     - 列表按钮宽 `width = text::PROMPT_MODE_LIST_LABEL.chars().count() as u16`（3 列），间隔 1 列：`x = add.x.checked_sub(width + 1)?`；
     - 要求左侧不压顶边框标题 `" Workspaces "`（`let min_x = sidebar.x + 1 + text::SIDEBAR_TITLE.chars().count() as u16 + 2; x >= min_x`），满足时返回 `Rect::new(x, sidebar.y, width, 1)`。
   - 按钮文案根据 `state.workspace_history.mode` 切换：
     - `WorkspaceMode::Active` → `text::PROMPT_MODE_LIST_LABEL` (`"[≡]"`)
     - `WorkspaceMode::History` → `text::PROMPT_MODE_EDIT_LABEL` (`"[/]"`)
   - 样式与 `[+]` 一致使用 `style::accent()`。
   - 在 `TooltipTarget` 新增 `WorkspaceMode`，悬停提示文案在 `Active` 下为 `"open workspace history"`，在 `History` 下为 `"open active workspaces"`。
2. **上半区视图切换与硬件光标返回（`src/ui/sidebar.rs` & `src/ui/mod.rs`）**：
   - `sidebar::render(frame, area, state) -> Option<(u16, u16)>` 返回可选的硬件光标坐标：
     - 当 `state.workspace_history.mode == WorkspaceMode::History` 时，上半区 `sections.workspaces` 调用 `history::render(frame, sections.workspaces, state)`，并在 `state.sidebar_focused()` 为真时返回搜索框的光标位置 `(cursor_x, cursor_y)`；
     - 在 `src/ui/mod.rs` 中将 `sidebar_cursor` 合并入 `base_cursor`（当 `state.overlay.is_none()` 时生效），保证焦点在侧栏历史列表时搜索框显示真实的终端光标。
3. **历史列表组件渲染与命中（`src/ui/sidebar/history.rs`）**：
   - 布局结构完全对齐 `src/ui/prompt/tree.rs`：
     - 第 0 行（`area.y`）：搜索框 `> `（`text::INPUT_PREFIX_ICON`，`style::accent()`）+ `widgets::input::view`（空态显示 `text::PROMPT_TREE_SEARCH_PLACEHOLDER` `"Search..."`，非空显示输入内容与选区反显）；
     - 第 1 行（`area.y + 1`）：贯穿分割线 `├──────┤`（使用 `text::DIVIDER_LEFT_JOIN`、`text::DIVIDER_MID`、`text::DIVIDER_RIGHT_JOIN`，样式 `style::border(state.sidebar_focused())`）；
     - 第 2 行起（`rows_area`）：高度 `area.height.saturating_sub(2)`，右侧预留 1 列滚动条（当过滤后行数 `> rows_area.height` 时渲染 `widgets::scrollbar`）；
     - 每行渲染：`marker`（选中行 `"❯ "`，未选中 `"  "`）+ `icon`（`"• "`）+ `name`（按可用宽度 `text::ellipsize` 截断）：
       - **当 `entry.exists == true` 时**：文字使用 `style::text().patch(highlight)`，图标使用 `style::muted().patch(highlight)`；
       - **当 `entry.exists == false`（路径不存在）时**：文字与图标均使用 **`highlight.patch(style::error())`**（红色前景，悬停时叠加 `style::selection()` 背景但保留红色前景，直观标识失效路径！）。
     - 空状态：当 `filtered.is_empty()` 时，若搜索词为空显示 `text::WORKSPACE_HISTORY_EMPTY` (`"no workspaces yet"`)，否则显示 `text::WORKSPACE_HISTORY_NO_MATCHES` (`"no matching workspaces"`)，样式 `style::muted()`。

#### 关键代码（`src/ui/sidebar/history.rs`）

```rust
//! 侧栏工作区历史列表渲染与鼠标命中：顶部搜索框 + 分割线 + 扁平历史条目 + 滚动条。

use ratatui::Frame;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use unicode_width::UnicodeWidthChar;

use crate::app::state::AppState;
use crate::layout::PROMPT_TREE_HEADER_HEIGHT;
use crate::ui::{layout, style, text, widgets};

/// 渲染工作区历史列表；返回搜索框硬件光标坐标（仅当侧栏聚焦时由调用方启用）。
pub fn render(frame: &mut Frame<'_>, area: Rect, state: &AppState) -> Option<(u16, u16)> {
    if area.width == 0 || area.height == 0 {
        return None;
    }
    let buf = frame.buffer_mut();
    let cursor = paint_search_row(buf, area, state);
    if area.height <= 1 {
        return cursor;
    }
    paint_divider(buf, area, area.y + 1, state.sidebar_focused());
    if area.height <= PROMPT_TREE_HEADER_HEIGHT {
        return cursor;
    }
    let list_area = Rect {
        y: area.y + PROMPT_TREE_HEADER_HEIGHT,
        height: area.height - PROMPT_TREE_HEADER_HEIGHT,
        ..area
    };
    let scrollbar = scrollbar_for(list_area, state);
    let rows_area = match &scrollbar {
        Some(_) => Rect {
            width: list_area.width.saturating_sub(1),
            ..list_area
        },
        None => list_area,
    };
    paint_rows(frame.buffer_mut(), rows_area, state);
    if let Some(scrollbar) = &scrollbar {
        widgets::scrollbar::render(frame, scrollbar);
    }
    cursor.filter(|_| state.sidebar_focused())
}

fn paint_rows(buf: &mut Buffer, rows_area: Rect, state: &AppState) {
    if rows_area.width == 0 || rows_area.height == 0 {
        return;
    }
    let panel = &state.workspace_history;
    let filtered = panel.filtered_indices();
    if filtered.is_empty() {
        let empty = if panel.search.text().trim().is_empty() {
            text::WORKSPACE_HISTORY_EMPTY
        } else {
            text::WORKSPACE_HISTORY_NO_MATCHES
        };
        paint_text(
            buf,
            rows_area.x,
            rows_area.y,
            empty,
            style::muted(),
            rows_area.right(),
        );
        return;
    }
    let scroll = panel.scroll.min(filtered.len().saturating_sub(1));
    let selected = panel.selected.min(filtered.len().saturating_sub(1));
    for (vis_row, &entry_idx) in filtered
        .iter()
        .enumerate()
        .skip(scroll)
        .take(usize::from(rows_area.height))
    {
        let Some(entry) = panel.entries.get(entry_idx) else {
            continue;
        };
        let y = rows_area.y + (vis_row - scroll) as u16;
        let hovered = panel.hover == Some(vis_row);
        let highlight = if hovered {
            fill_row(buf, rows_area, y);
            style::selection()
        } else {
            ratatui::style::Style::default()
        };
        // 路径不存在的条目使用红色（style::error()），hover 时只借用背景色、保留红色前景。
        let item_style = if entry.exists {
            style::text().patch(highlight)
        } else {
            highlight.patch(style::error())
        };
        let icon_style = if entry.exists {
            style::muted().patch(highlight)
        } else {
            highlight.patch(style::error())
        };
        let marker = if vis_row == selected {
            format!("{} ", text::AGENT_SELECTED_MARKER)
        } else {
            "  ".to_string()
        };
        let icon = format!("{} ", text::PROMPT_TREE_ITEM_ICON);
        let mut x = paint_text(buf, rows_area.x, y, &marker, item_style, rows_area.right());
        x = paint_text(buf, x, y, &icon, icon_style, rows_area.right());
        let max_width = usize::from(rows_area.right().saturating_sub(x));
        let label = text::ellipsize(&entry.name, max_width);
        paint_text(buf, x, y, &label, item_style, rows_area.right());
    }
}
```

---

### 4.4 状态更新、双击打开、CRUD 与失效条目级联删除（`src/app/update/workspace_history.rs`）

#### 关键点
1. **模式切换（`toggle_workspace_mode` / `show_workspace_history` / `exit_workspace_history`）**：
   - 进入 `WorkspaceMode::History` 时：
     - 清理活跃列表拖拽态（`workspace_drag = None`、`workspace_scroll_drag = None`）；
     - 切换 `state.workspace_history.mode = WorkspaceMode::History`；
     - 立即调用 `state.workspace_history.refresh_existence()` 刷新磁盘路径存在性，并推入 `Effect::LoadWorkspaceHistory` 从数据库同步最新列表；
     - 自动将选中项对齐到当前活跃工作区对应的历史行（若存在），并调用 `focus_sidebar(state)` 让搜索框立即可输入。
   - 退出到 `WorkspaceMode::Active` 时：
     - 切换 `state.workspace_history.mode = WorkspaceMode::Active`；
     - 清理 `hover`、`row_click`、`scroll_drag`。
2. **双击判定与打开工作区（`workspace_history_row_double_click` & `open_workspace_history_entry`）**：
   - 双击窗口常量复用 400ms（`WORKSPACE_HISTORY_DOUBLE_CLICK_WINDOW = Duration::from_millis(400)`）。
   - 打开指定 `entry_index` 时：
     1. 实时检查 `entry.path.is_dir()`：
        - 若为 `false`（目录不存在）：将 `entry.exists` 置为 `false`，弹出全局 Danger Toast `text::TOAST_DIRECTORY_NOT_FOUND`（锚定侧栏或无锚点），**保持在历史列表视图不退出**。
     2. 若目录存在：
        - 检查 `state.workspaces` 是否已有同路径工作区（`ws.cwd.as_deref() == Some(entry.path.as_path())`）：
          - **已打开**：调用 `switch_workspace(state, existing_index)` 切换到该工作区，推入 `Effect::RecordWorkspace { path, name }` 刷新其 `updated_at`，并调用 `exit_workspace_history(state)` 切回默认活跃列表；
          - **未打开**：以保存的 `(entry.path, entry.name)` 创建工作区（若 `entry.name` 与默认目录末段名不同，则置 `name_is_manual = true` 保留自定义名），激活该工作区，推入 `Effect::RecordWorkspace`，并调用 `exit_workspace_history(state)` 切回默认活跃列表。
3. **重命名双向同步（`RenameTarget::Workspace` & `RenameTarget::WorkspaceHistory`）**：
   - 在活跃列表重命名工作区（`RenameTarget::Workspace(index)`）：
     - 修改 `workspace.name = name; workspace.name_is_manual = true;`；
     - 若该工作区有 `cwd`，同步修改 `state.workspace_history.entries` 中同路径条目的 `name`，并推入 `Effect::RenameWorkspace { path, name }` 落库。
   - 在历史列表重命名工作区（`RenameTarget::WorkspaceHistory(entry_index)`）：
     - 修改 `state.workspace_history.entries[entry_index].name = name`；
     - 若 `state.workspaces` 中存在同路径的已打开工作区，同步更新其 `workspace.name` 并置 `name_is_manual = true`；
     - 推入 `Effect::RenameWorkspace { path, name }` 落库。
4. **失效路径条目的级联删除（`MenuCommand::DeleteWorkspaceHistory`）**：
   - 右键菜单构造（`open_workspace_history_menu(state, visible_row, anchor)`）：
     - 先将 `state.workspace_history.selected` 移到 `visible_row`；
     - 取对应 `entry`（同时实时刷新一次 `entry.exists = entry.path.is_dir()`）：
       - 若 `entry.exists == true`：`commands = vec![MenuCommand::OpenWorkspaceHistory, MenuCommand::RenameWorkspace]`；
       - 若 `entry.exists == false`：`commands = vec![MenuCommand::OpenWorkspaceHistory, MenuCommand::RenameWorkspace, MenuCommand::DeleteWorkspaceHistory]`。
   - `MenuCommand::DeleteWorkspaceHistory` 加入 `MenuCommand::requires_confirm()`，首次点击在菜单行内变为红色 `Confirm delete`；
   - 二次点击确认后调用 `delete_workspace_history_entry(state, entry_index)`：
     - 校验 `!entry.path.is_dir()`，从 `state.workspace_history.entries` 移除该项并 `clamp` 选中与滚动；
     - 推入 `Effect::DeleteWorkspaceHistory { path }`，由 `Persistence` 调用 `db.delete_workspace(&path)` 级联删除数据库记录，并同步移除 `state.workspaces` 中引用该已删除会话的残留状态（若存在）。

#### 关键代码（`src/app/update/workspace_history.rs` 核心函数）

```rust
/// 打开指定的历史工作区条目（双击 / Enter / 右键 Open）。
pub fn open_workspace_history_entry(state: &mut AppState, entry_index: usize) -> bool {
    let Some(entry) = state.workspace_history.entries.get(entry_index).cloned() else {
        return false;
    };
    if !entry.path.is_dir() {
        if let Some(slot) = state.workspace_history.entries.get_mut(entry_index) {
            slot.exists = false;
        }
        show_toast(state, text::TOAST_DIRECTORY_NOT_FOUND, ToastKind::Danger);
        return true;
    }
    if let Some(existing) = state
        .workspaces
        .iter()
        .position(|ws| ws.cwd.as_deref() == Some(entry.path.as_path()))
    {
        switch_workspace(state, existing);
        state.effects.push(Effect::RecordWorkspace {
            path: entry.path,
            name: entry.name,
        });
        exit_workspace_history(state);
        return true;
    }
    open_workspace_from_history(state, entry.path, entry.name);
    exit_workspace_history(state);
    true
}

/// 删除路径已失效的历史工作区条目，并登记数据库级联删除效应。
pub fn delete_workspace_history_entry(
    state: &mut AppState,
    entry_index: usize,
    visible_rows: usize,
) -> bool {
    let Some(entry) = state.workspace_history.entries.get(entry_index).cloned() else {
        return false;
    };
    if entry.path.is_dir() {
        // 防御式校验：路径重新存在时拒绝删除并刷新状态。
        if let Some(slot) = state.workspace_history.entries.get_mut(entry_index) {
            slot.exists = true;
        }
        return true;
    }
    state.workspace_history.entries.remove(entry_index);
    state.workspace_history.clamp(visible_rows);
    state.effects.push(Effect::DeleteWorkspaceHistory {
        path: entry.path,
    });
    true
}
```

---

### 4.5 键盘与鼠标事件路由（`src/input/mod.rs`、`src/event/input.rs`、`src/event/mouse/`）

#### 关键点
1. **键盘路由（`src/input/mod.rs` & `src/event/input.rs`）**：
   - 在 `src/input/mod.rs` 新增 `Routed::Sidebar(SidebarKey)`（复用单行输入与列表导航键集：`Up`、`Down`、`Enter`、`Esc`、`Char`、`Backspace`、`Delete`、`Left`、`Right`、`Home`、`End`、`LineStart`、`LineEnd`、`SelectAll`、`DeleteToLineStart`、`DeleteToLineEnd`、`WordLeft`、`WordRight`、`DeleteWordBackward`、`DeleteWordForward`、`Undo`、`Redo`、`Clear`、`Cut`）。
   - 当 `focus == FocusedContainer::Sidebar` 时，`route` 返回 `sidebar_key(key).map(Routed::Sidebar)`；
   - 在 `src/event/input.rs` 中：
     - 处理 `Routed::Sidebar(key)`：若 `state.workspace_history.mode == WorkspaceMode::History`，执行搜索编辑、上下选择、回车打开、Esc 退出或 Ctrl+C 复制/清空；若为 `WorkspaceMode::Active`，直接忽略（保持活跃视图无键盘副作用）。
     - 处理粘贴 `handle_paste_event`：若 `state.sidebar_focused()` 且处于 `WorkspaceMode::History`，将文本粘贴进 `state.workspace_history.search` 并重置选中到首项；否则忽略。
2. **鼠标按下/拖动/松开/滚轮/悬停（`src/event/mouse/`）**：
   - **`press.rs` / `hit.rs`**：
     - 顶边框 `ui::workspace_mode_button(view)` 命中：调用 `update::toggle_workspace_mode(state)`；
     - 当处于 `WorkspaceMode::History` 时：
       - 命中历史滚动条 → 拖拽或跳转历史列表滚动偏移；
       - 命中搜索框行 → `focus_sidebar(state)` + 定位搜索框光标并开启拖拽选区；
       - 命中历史条目行 → `focus_sidebar(state)` + 选中该行并记录按下状态（供松开时判定单击/双击）；
     - 右键点击（`src/event/overlay.rs`）：
       - `WorkspaceMode::History` 下命中历史条目 → 打开 `OverlayTarget::WorkspaceHistory(entry_index)` 菜单；
       - 命中历史列表空白区 → 打开 `OverlayTarget::WorkspaceList`（`New Workspace`）菜单。
   - **`wheel.rs`**：
     - 指针在 `workspace_section_at` 内滚动时，若为 `WorkspaceMode::History`，滚动 `state.workspace_history.scroll` 并重算 `hover`。
   - **`hover.rs`**：
     - 在 `WorkspaceMode::History` 下计算 `ui::workspace_history_item_at` 更新 `state.workspace_history.hover`。

---

## 5. 涉及文件与行数预算清单

| 文件路径 | 操作 | 现状行数 | 预计改动后行数 | 说明（严格控制在 1000 行以内） |
| --- | --- | --- | --- | --- |
| `src/db/workspace.rs` | **新建** | 0 | ~160 | 承接 `workspace` 表的信任、历史记录、重命名、列表查询与级联删除 |
| `src/db/mod.rs` | 修改 | 950 | ~890 | 迁出工作区方法至 `src/db/workspace.rs`，远离 1000 行上限 |
| `src/db/service.rs` | 修改 | 690 | ~750 | 处理 `RecordWorkspace` / `RenameWorkspace` / `DeleteWorkspaceHistory` / `LoadWorkspaceHistory` 及启动同步 |
| `src/app/state/workspace_history.rs` | **新建** | 0 | ~150 | `WorkspaceMode`、`WorkspaceHistoryEntry`、`WorkspaceHistoryPanel` |
| `src/app/state.rs` | 修改 | 710 | ~725 | `AppState` 挂载 `workspace_history: WorkspaceHistoryPanel` |
| `src/app/update/workspace_history.rs` | **新建** | 0 | ~260 | 历史列表模式切换、搜索、上下导航、双击打开、重命名同步、级联删除 |
| `src/app/update/workspace.rs` | 修改 | 420 | ~450 | `create_workspace_with_cwd` 接入 `RecordWorkspace` 效应 |
| `src/app/update/worktree.rs` | 修改 | 310 | ~325 | 打开 worktree 创建工作区时接入 `RecordWorkspace` 效应 |
| `src/app/update/menu.rs` | 修改 | 480 | ~530 | 新增 `open_workspace_history_menu` 及菜单命令激活分支 |
| `src/app/update/dialog.rs` | 修改 | 320 | ~340 | `commit_rename` 支持 `RenameTarget::WorkspaceHistory` 及活跃工作区改名落库 |
| `src/app/overlay.rs` | 修改 | 410 | ~425 | 新增 `OverlayTarget::WorkspaceHistory`、`MenuCommand::OpenWorkspaceHistory` / `DeleteWorkspaceHistory`、`RenameTarget::WorkspaceHistory` |
| `src/app/effects.rs` | 修改 | 250 | ~275 | 新增工作区历史相关 `Effect` 变体 |
| `src/app/tooltip.rs` | 修改 | 140 | ~145 | 新增 `TooltipTarget::WorkspaceMode` |
| `src/ui/sidebar/history.rs` | **新建** | 0 | ~260 | 历史列表渲染（搜索框、分割线、红字失效项、滚动条）与命中函数 |
| `src/ui/sidebar.rs` | 修改 | 615 | ~660 | 顶边框 `[≡]`/`[/]` 按钮几何与绘制、上半区 `Active`/`History` 分派 |
| `src/ui/text.rs` | 修改 | 1160 | ~1180 | 新增历史列表空态、菜单项与 Tooltip 英文常量 |
| `src/ui/tooltip.rs` | 修改 | 260 | ~270 | 注册 `TooltipTarget::WorkspaceMode` |
| `src/input/mod.rs` | 修改 | 545 | ~610 | 新增 `Routed::Sidebar(SidebarKey)` 键盘映射 |
| `src/event/input.rs` | 修改 | 290 | ~340 | 处理侧栏历史模式下的按键与粘贴 |
| `src/event/mouse/hit.rs` | 修改 | 230 | ~320 | 承接侧栏历史列表滚动条与左键按下辅助逻辑，为 `press.rs` 减负 |
| `src/event/mouse/press.rs` | 修改 | 992 | ~950 | 抽取侧栏按下逻辑至 `hit.rs`，确保低于 1000 行上限 |
| `src/event/mouse/drag.rs` | 修改 | 570 | ~610 | 处理历史列表滚动条拖拽、搜索框选区拖拽与行点击松开（单击/双击） |
| `src/event/mouse/wheel.rs` | 修改 | 600 | ~620 | 历史列表滚轮滚动 |
| `src/event/mouse/hover.rs` | 修改 | 380 | ~400 | 历史列表悬停高亮与搜索框选区拖拽优先 |
| `src/event/overlay.rs` | 修改 | 530 | ~555 | 历史列表右键菜单入口 |

---

## 6. 测试矩阵与风险回滚

### 6.1 单元测试与集成测试矩阵
1. **DB 层测试（`src/db/tests.rs`）**：
   - `record_workspace_open` 按路径去重并更新 `updated_at` 排序；
   - `rename_workspace` 修改名称后，再次调用 `ensure_workspace` 或 `record_workspace_open` 不会覆盖已修改的自定义名称；
   - `delete_workspace` 级联删除该工作区下的 `workspace_folder`、`workspace_prompt`、`workspace_prompt_page`、`workspace_referenced_folder` 和 `agent_session`。
2. **状态与交互层测试（`src/app/update/tests.rs` & `src/event/tests/`）**：
   - 点击 `[≡]` 切换到 `WorkspaceMode::History`，按钮变为 `[/]`，再次点击或按 `Esc` 切回 `WorkspaceMode::Active`；
   - 历史列表搜索框输入关键字实时过滤（匹配名称和路径），`Up`/`Down` 移动选中，`Ctrl+C` 清空搜索词；
   - 400ms 内双击（或按 `Enter`）已存在路径的历史项：若已打开则直接切换过去，若未打开则新建并切换，且自动返回 `Active` 视图；
   - 路径不存在（`!exists`）的历史项：UI 渲染为 `style::error()`（红色），双击弹出 Danger Toast `"Directory not found"` 并留在历史列表；
   - 右键菜单差异：有效路径条目仅有 `[Open, Rename Workspace]`；失效路径条目包含 `[Open, Rename Workspace, Delete]`，且 `Delete` 首次点击进入 `Confirm delete` 行内二次确认，二次点击触发级联删除。
3. **UI 几何测试（`src/ui/sidebar/tests.rs`）**：
   - `workspace_mode_button` 严格位于 `add_workspace_button` 左侧 1 列（`x = add.x - 4`），侧栏折叠时隐藏。

### 6.2 风险与回滚
- **无 Schema 迁移风险**：完全复用现有 v22 `workspace` 表及既有 `ON DELETE CASCADE` 外键约束，不改变 `SCHEMA_VERSION`，新旧二进制完全兼容。
- **回滚策略**：所有改动集中在独立子模块与明确的状态分支内，若需回滚可直接按模块撤销而不影响既有数据库文件。
