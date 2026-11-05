# 左侧栏工作区历史列表（Workspace History List）— 详细设计 (`design.md`)

- **特性目录**：`docs/feature/workspace-history-list/`
- **关联任务清单**：`docs/feature/workspace-history-list/task.md`
- **当前状态**：**已在 `.worktrees/workspace-history-list` (`feature/workspace-history-list`) 完成实现、review 修复与测试，待用户确认合并**
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
| **工作区持久化与外键级联** | `src/db/migration/schema.rs`：`workspace` 表含 `id`, `external_id UNIQUE`, `name`, `path UNIQUE`, `is_trusted`, `position`（v23）, `created_at`, `updated_at`；v24 另有 `workspace_hidden(path PRIMARY KEY, hidden_at)`。所有关联子表：`workspace_folder`、`workspace_prompt`、`workspace_referenced_folder`、`agent_session` 及其全部 12 张聊天子表均声明了 `ON DELETE CASCADE`，且运行时 `PRAGMA foreign_keys = ON`（`src/db/mod.rs`）。 | 1. `SCHEMA_VERSION` 递增到 v24：v23 增加全局 `position`，v24 增加隐藏路径表；<br>2. 修正 `ensure_workspace` 为 `ON CONFLICT(path) DO NOTHING`，防止后台保存提示词/会话时用目录末段名覆盖用户手动修改过的 `workspace.name`；<br>3. 删除按 canonical 等价清理 `workspace` 行，由 SQLite 外键自动级联清理全部关联提示词、文件夹、引用目录和聊天会话。 |
| **侧栏焦点与键盘路由** | `src/input/mod.rs:157-160` 与 `src/event/input.rs:255-257`：当前 `FocusedContainer::Sidebar` 在路由层无条件返回 `None` 吞掉所有按键与粘贴。 | 侧栏处于 `WorkspaceMode::History` 时，放行搜索输入、上下移动（`Up`/`Down`）、回车打开（`Enter`）、退出列表（`Esc`）、复制/清空搜索（`Ctrl/Cmd+C`）、剪切（`Ctrl/Cmd+X`）及粘贴；处于 `WorkspaceMode::Active` 时维持原样吞掉按键。 |
| **单文件行数水位** | `src/db/mod.rs` 已达 950 行；`src/event/mouse/press.rs` 已达 992 行；`src/ui/sidebar.rs` 为 615 行。 | 遵循 1000 行红线：将工作区 DB 读写抽入 `src/db/workspace.rs`；将侧栏历史列表 UI 抽入 `src/ui/sidebar/history.rs`；将侧栏鼠标按下分支抽入 `src/event/mouse/hit.rs`，确保所有生产文件远低于 1000 行。 |

---

## 1. 背景与目标 / 非目标

### 1.1 目标
1. **新增工作区历史列表切换按钮**：在左侧栏 `Workspaces` 顶边框右上角（`[+]` 按钮左侧，间隔 1 列）增加列表按钮，默认显示 `[≡]`，点击打开工作区历史列表并将图标切换为 `[/]`。
2. **工作区历史自动记录与持久化**：凡是显式创建或打开过的工作区（启动初始工作区、`[+]` 新建、从 Worktree 打开、从历史列表打开、显式 Sync workspace to terminal cwd）均按规范化路径 `path` 去重记录到 SQLite `workspace` 表，并与活跃列表共用全局顺序列 `position`（schema v23）；拖拽排序写回，重启后保持一致；终端内执行 `cd` 切换子目录不产生新历史记录。
3. **对齐 Prompt List 的交互体验**：
   - 在左侧栏上半区 `Workspaces` 原位渲染「顶部搜索框 + 分割线 + 扁平历史列表 + 滚动条」，下半区 `Agents` 不变；
   - 支持实时搜索过滤（匹配名称或路径，不区分大小写）、键盘上下选择、回车/双击打开、鼠标悬停高亮；
   - 支持新建工作区（Create）、重命名工作区并双向同步（Update/Rename）、双击或右键打开（Open）、拖拽排序（与活跃列表同为 Git 整组块移动）。
4. **失效路径高亮与级联删除**：
   - 当历史工作区的目录路径在磁盘上已不存在（`!path.is_dir()`）时，该条目使用错误/危险语义色（`style::error()`，红色）渲染；
   - 所有历史条目均提供 `Delete`（3 击菜单确认 + 最终弹窗确认），确认后中断该工作区运行中的 Agent / 子 Agent / 本地命令，并从数据库级联删除其关联的所有提示词、文件夹、引用目录及聊天会话数据。

### 1.2 非目标
- 历史列表不引入文件夹层级或 `tmp:` / `saved:` 分区。
- 搜索过滤期间不响应拖拽排序（避免对过滤子集重排导致隐藏条目的相对顺序不可预期）。

---

## 2. 已确认的边界决策汇总（Grill Me 结论表）

| 编号 | 决策维度 | 已确认方案 |
| --- | --- | --- |
| **Q1** | **数据来源与生命周期** | 凡是显式创建或打开过的工作区自动持久化到 SQLite `workspace` 表，与活跃列表共用全局顺序列 `position`（拖拽排序写回，重启后保持）。在当前活跃列表中执行 `Close Workspace` 仅从内存 `AppState.workspaces` 移除并结束终端进程，保留历史记录。 |
| **Q2 / Q3** | **历史条目的完整 CRUD 与单弹窗确认** | 历史列表中的所有工作区均支持：**查看/搜索（Read）**、**新建（Create，在首个 `NewWorkspace` 弹窗内直接二次确认 `[Create]` → `[Confirm]`，不再弹第二个 `TrustWorkspace` 弹窗；在 `ws list` 下新建后仅入库并选中，保持停留在 `ws list`）**、**重命名（Update）**、**双击/回车/右键打开（Open）**、以及**全量级联删除（Delete，需右键菜单 3 击确认 + 最终弹窗确认，若有运行中 Agent 直接中断并级联删除数据库记录）**。 |
| **Q4** | **按钮位置、图标与视图切换** | 顶边框右端顺序为 `[+] [≡]`（列表按钮 `[≡]`/`[/]` 居右端，新建按钮 `[+]` 紧邻其左侧 1 列）。默认显示 `[≡]`，点击后上半区 `Workspaces` 原地切换为历史列表，图标变为 `[/]`（下半区 `Agents` 不变）。点击 `[/]`、按 `Esc`、或双击/回车成功打开历史工作区后，自动切回活跃工作区列表。 |
| **Q5** | **列表结构、Git 树形与搜索过滤** | 顶部为 `> search workspaces…` 搜索框 + `├────┤` 分割线；移除 `⑂` 图标，Git 仓库自动以主工作区（`repo_root`）为父节点（`▾/▸ <主工作区名>`，支持点击箭头折叠/展开），下挂 `git worktree list` 的全部工作区及数据库中的历史 worktree（`├─/└─ <分支短名或自定义名>`），独立工作区直接显示 `<name>`。搜索框实时不区分大小写匹配工作区 `name`、分支短名与 `path`，无结果时显示 `no matching workspaces`（空库显示 `no workspaces yet`）。 |
| **Q6** | **命名污染修复、路径去重、重命名同步与失效路径级联删除** | 1. **主工作区命名修复与自愈**：修复 `startup_trust` 在 linked worktree 启动时将子工作区名误写入主工作区路径的 Bug，并在加载历史列表时对已污染的主工作区名称自动自愈回主工作区目录名（如 `lx-tui`）。<br>2. **按路径去重**：以规范化路径 `path` 唯一去重，重复打开仅刷新 `updated_at`，不改变 `position` 顺序；终端内 `cd` 不写入历史列表。<br>3. **重命名双向同步**：在活跃列表或历史列表重命名工作区均写入 `workspace.name`，并同步更新另一侧显示名；重新打开时保留自定义名称。<br>4. **双击打开防重与父项自动补齐**：若目标路径已在 `AppState.workspaces` 中打开则直接切换过去；若打开的是 linked worktree，自动调用 `ensure_main_workspace` 补开主工作区父项并退出历史视图。<br>5. **失效路径特殊颜色与全量级联删除**：若历史条目的 `path` 在磁盘上不存在（`!path.is_dir()`），条目文字使用 `style::error()`（红色）展示；双击打开时弹 Danger Toast `Directory does not exist` 并留在历史列表；全部条目右键菜单开放 `Delete`（3 击菜单确认 + 最终弹窗确认），确认后中断运行中的 Agent / 子 Agent / 本地命令并级联清理全部关联表数据。删除仓库父项时会一并删除同仓库 linked worktree，最终弹窗明确显示数量；仍存在于磁盘的 linked worktree 写入 `workspace_hidden`，重启后不自动复活。 |
| **Q7** | **全局排序与拖拽** | `workspace.position` 是活跃列表与历史列表唯一的顺序源（schema v23）：`list_workspaces` 按 `position ASC, id ASC` 返回，历史列表再按 Git 树形展开；活跃列表按历史顺序稳定重排（未打开项保持槽位）。两个列表均支持以 Git 仓库整组块为单位的拖拽排序：拖父项或任一 linked 子项都会移动整组；非 Git 项单独移动。历史列表搜索过滤期间不响应拖拽。拖拽结束后通过 `Effect::SaveWorkspaceOrder` 将完整顺序写回 SQLite。 |
| **Q8** | **信任与隐藏** | 历史条目携带 `trusted` 快照；未信任条目打开前先走 `CheckTrust { trigger: OpenWorkspaceHistory }`，授予后按路径重新定位并续作。`workspace_hidden` 持久化已删除但仍存在的 linked worktree；`RecordWorkspace` 显式登记时解除隐藏。 |

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
│     └─ Row 2..N: `❯ ▾/├─ <name>` (exists=true 用 style::text(), false 用 style::error)│
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
│   鼠标点击/双击 (MouseDown 记录 entry_path + pressed_at；滚动/拖拽/重载使其失效):  │
│     400ms 内同一条目第二次按下 ──► update::open_workspace_history_row(state, row)  │
│       ├─ !path.is_dir() ──► 刷新 exists=false，弹 Danger Toast "Directory not found"│
│       ├─ 已在 state.workspaces 打开 ──► switch_workspace + 补开主 checkout + 聚焦主区│
│       ├─ entry.trusted == false ──► Effect::CheckTrust(OpenWorkspaceHistory)        │
│       │                              └─ 授予后 resume_trust 继续创建并打开           │
│       └─ 已信任且未打开 ──► 创建/激活 + Effect::RecordWorkspace + 切回 Active       │
├────────────────────────────────────────────────────────────────────────────────────┤
│ 3. 历史条目右键菜单与 CRUD 链路                                                    │
│   event::overlay::open_right_click_menu()                                          │
│     ├─ 命中历史条目: OverlayTarget::WorkspaceHistory(index)                        │
│     │    └─ 全部条目: [OpenWorkspaceHistory, RenameWorkspace, DeleteWorkspace]     │
│     └─ 命中历史空白: OverlayTarget::WorkspaceList -> [NewWorkspace]                │
│                                                                                    │
│   activate_menu():                                                                 │
│     ├─ OpenWorkspaceHistory ──► open_workspace_history_entry(state, index)         │
│     ├─ RenameWorkspace      ──► Overlay::Rename(RenameTarget::WorkspaceHistory(i)) │
│     │    └─ commit_rename() ──► 更新 history + 同步已打开同路径 Workspace.name      │
│     │                           + Effect::RenameWorkspace { path, name }           │
│     └─ DeleteWorkspace      ──► 3 击菜单确认（Delete → 1/2 → 2/2）                 │
│          └─ 第 3 击 ──► Overlay::ConfirmClose（父项显示 N 个 linked worktree）     │
│               └─ 确认后 ──► 中断 Agent/子 Agent/Shell + 关闭已打开工作区            │
│                              + Effect::DeleteWorkspaceHistory { path }             │
│                              └─ SQLite DELETE ... CASCADE；仍存在的 linked worktree │
│                                 写入 workspace_hidden，重启后不再自动复活            │
└────────────────────────────────────────────────────────────────────────────────────┘
```

---

## 4. 详细技术设计与关键代码

### 4.1 状态模型设计（`src/app/state/workspace_history.rs` & `src/app/state.rs`）

#### 关键点
1. `WorkspaceHistoryEntry` 字段：`name`、`path`、`exists`、`name_is_manual`、`position`、`trusted`、`git`。
2. `WorkspaceHistoryPanel` 字段：`mode`、`entries`、`search`、`selected`、`scroll`、`hover`、`scroll_drag`、`search_drag`、`drag`、`dragging`、`drag_moved`、`row_click: Option<(PathBuf, Instant)>`、`reveal_selected`、`loaded`、`collapsed_repos`、`hidden_worktrees`、`new_workspace_confirming`。
3. 核心方法：`filtered_indices`、`visible_rows`、`tree_order`、`drag_block`、`delete_worktree_count`、`reorder_entries`、`normalize`、`clamp`（返回是否有变化）、`scroll_selected_into_view`、`refresh_existence`、`entry_at_visible_row`、`toggle_repo_collapsed`。
4. 树形不变量：父项在子项前；拖拽块 = 父项 + 全部 linked worktree；`entries` 归一化后 raw 顺序等于完整树序，活跃列表据此稳定重排。
5. 交互约束：搜索过滤期间禁止拖拽与折叠切换；双击以条目路径而非可见行号识别，滚动/拖拽/重载会作废点击序列。

> 实现以 `src/app/state/workspace_history.rs` 为准，本文不再复制整段代码。

---

### 4.2 数据库层与持久化效应（`src/db/workspace.rs`、`src/app/effects.rs`、`src/db/service.rs`）

#### 关键点
1. **Schema 版本**：v23 为 `workspace` 增加全局 `position`（活跃/历史共用，拖拽写回 `0..n`）；v24 增加 `workspace_hidden` 表，持久化“已从历史列表删除但仍存在于磁盘”的 worktree 路径。
2. **路径身份**：所有读写入口通过 `workspace_row_path` 先精确匹配、再按 canonical 等价匹配已有行，兼容旧库/终端 cwd 留下的尾斜杠、`..` 与符号链接文本；删除会清理同一目录的全部等价行。
3. **历史列表组装**：`list_workspace_history_entries` 按 `position` 排序，Git 仓库以主 checkout 为父项、live/历史 linked worktree 为子项；只有路径确实等于主 checkout 或某个 worktree 根时才归入 Git 树，仓库子目录保持顶层；`workspace_hidden` 中的路径不输出。
4. **信任状态**：每条历史记录带 `trusted` 快照（路径本身或仓库根已信任）；历史列表打开未信任条目时先发 `Effect::CheckTrust { trigger: OpenWorkspaceHistory }`，不再绕过信任门。
5. **效应**：`RecordWorkspace`（登记/刷新并解除隐藏）、`RenameWorkspace`、`DeleteWorkspaceHistory`（中断会话后 CASCADE 删除，仍存在的 linked worktree 写入隐藏表）、`SaveWorkspaceOrder`、`LoadWorkspaceHistory`。写失败对重命名/删除弹出 Danger Toast。
6. **删除父项**：会一并删除同仓库的 linked worktree（关闭已打开窗口、中断 Agent/子 Agent/Shell）；最终确认文案会明确显示将一并删除的 worktree 数量。

---

### 4.3 侧栏右上角 `[≡]` / `[/]` 按钮与历史列表渲染（`src/ui/sidebar.rs` & `src/ui/sidebar/history.rs`）

#### 关键点
1. **顶部按钮**：`[+] [≡]`（`[+]` 在左、`[≡]`/`[/]` 在右，间隔 1 列）；折叠或空间不足时隐藏。
2. **列表布局**：搜索框 + `├────┤` 分割线 + 树形条目列表 + 滚动条；Git 组父项显示 `▾/▸ <主工作区名>`，子项显示 `├─/└─ <分支短名或自定义名>`，不再使用 `•` / `⑂` 图标。
3. **行样式**：选中行 `❯ ` 标记；失效路径红色、hover 保留红色前景；拖拽块整块反显；名称按显示列截断（宽字符不越界），并保留去重序号/初始工作区 `*`。
4. **命中几何**：折叠箭头只命中父项行第 2 列（行首 2 列为选中标记）；搜索过滤期间箭头不响应；列表滚动偏移统一按 `total - viewport` 夹取；条目少于视口时拖拽落点吸附最后一条。
5. **滚动/揭示**：渲染与命中都使用夹取后的偏移；进入历史、新建条目、重载后由事件循环消费 `reveal_selected` 把选中项滚入视口。

> 实现以 `src/ui/sidebar/history.rs` 与 `src/ui/sidebar.rs` 为准。

---

### 4.4 状态更新、双击打开、CRUD 与级联删除（`src/app/update/workspace_history.rs`）

#### 关键点
1. **模式切换**：进入历史列表时清理两套拖拽状态、刷新路径存在性、对齐当前活跃工作区并请求 DB 快照；退出时清空 hover/点击/拖拽状态。
2. **排序同步**：`sync_active_order_to_history` 只重排已匹配历史条目的活跃工作区，未匹配项保持原槽位；历史拖拽后反向同步活跃列表。`entries` 归一化后 raw 顺序与渲染树序一致。
3. **拖拽**：以 Git 仓库整组块为单位；`dragging` 控制整块反显，`drag_moved` 只在顺序真正变化时置位，松开时仅后者触发 `SaveWorkspaceOrder`，避免抖动写库。搜索过滤期间禁止拖拽。
4. **打开**：失效路径 Toast 并留在列表；已打开则切换并补开主 checkout、交回主区焦点；未打开且已信任则创建并记录；未信任先走 `CheckTrust`，授予后按路径重新定位条目续作。
5. **新建确认**：`NewWorkspace` 首次 Create 进入弹窗内 Confirm；任何文本编辑、粘贴、剪切都会复位确认态，避免确认一个已变化的路径。
6. **删除**：全部条目支持 3 击菜单 + 最终弹窗；父项确认文案显示同仓库 linked worktree 数量；删除时中断运行中 Agent/子 Agent/Shell、关闭已打开工作区、CASCADE 清理 DB；最后一个打开工作区保留一条“已信任 + 隐藏”行，避免后续保存以未信任状态复活。

---

### 4.5 键盘与鼠标事件路由（`src/input/mod.rs`、`src/event/input.rs`、`src/event/mouse/`）

#### 关键点
1. **键盘路由**：`Routed::Sidebar(SidebarKey)` 仅在侧栏聚焦、历史模式且**侧栏未折叠**时生效；`Up`/`Down` 移动选中、`Enter` 打开、`Esc` 退出，搜索编辑/粘贴/Ctrl+C/Ctrl+X 复用单行输入键集。Active 模式仍吞掉侧栏按键。
2. **鼠标按下**：`press.rs` 入口先清理上一轮可能丢失 Up 的历史手势状态（`search_drag`、`scroll_drag`、`drag`、`dragging`、`drag_moved`）；`hit.rs` 按滚动条 → 搜索框 → 折叠箭头 → 条目行的优先级分派。
3. **双击**：以条目路径 + 400ms 窗口判定，第二次按下触发；滚动、真正拖拽、重载会作废点击序列，避免滚动后同一屏幕行误开另一条目。
4. **折叠箭头**：只命中父项行第 2 列（`list.x + 2`），选中标记列不触发；搜索过滤期间不响应折叠。
5. **拖拽**：Git 整组块移动；`dragging` 控制反显，`drag_moved` 控制松开时是否 `SaveWorkspaceOrder`；搜索过滤期间禁用。
6. **滚动/悬停**：滚轮与滚动条偏移统一按 `total - viewport` 夹取；拖拽/搜索拖选期间抑制 hover 与 tooltip；侧栏折叠时列表/Agents/滚动条命中全部返回 None。
7. **焦点**：打开已打开的历史工作区或新建工作区后调用 `focus_panes`，避免焦点停留在侧栏导致键盘输入被吞。

---

## 5. 涉及文件与行数（实现后实测）

| 文件路径 | 操作 | 行数 | 说明（生产代码 ≤ 1000 行） |
| --- | --- | --- | --- |
| `src/db/workspace.rs` | 新建/扩展 | 974（含内联 tests） | 工作区信任、历史组装、position、隐藏、canonical 路径、级联删除 |
| `src/db/mod.rs` | 修改 | 914 | 迁出工作区方法，`SCHEMA_VERSION = 24` |
| `src/db/service.rs` | 修改 | 958 | 工作区历史效应、会话中断、隐藏/解除隐藏 |
| `src/app/state/workspace_history.rs` | 新建 | 520 | 模式、条目、树序、拖拽块、过滤/滚动/选中 |
| `src/app/update/workspace_history.rs` | 新建 | 986 | 模式切换、搜索、打开/信任、排序同步、拖拽、删除 |
| `src/app/update/workspace_history/tests.rs` | 新建 | 816 | 历史列表状态机测试 |
| `src/ui/sidebar/history.rs` | 新建 | 773（含 tests） | 历史列表渲染、命中、拖拽落点 |
| `src/ui/sidebar.rs` | 修改 | 688 | `[+] [≡]` 按钮、Active/History 分派、树形渲染与命中 |
| `src/event/mouse/press.rs` | 修改 | 963 | 手势状态清理与侧栏按下分派 |
| `src/event/mouse/hit.rs` | 修改 | 743 | 侧栏/历史命中与滚动条 |
| `src/event/mouse/drag.rs` | 修改 | 695 | 历史拖拽、搜索拖选、释放 |
| `src/ui/text.rs` | 修改 | — | 历史列表/确认弹窗/Toast 英文常量 |

---

## 6. 测试矩阵与风险回滚

### 6.1 单元测试与集成测试矩阵
1. **DB 层测试（`src/db/workspace.rs` tests、`src/db/tests.rs`、`src/db/service/tests.rs`）**：
   - `position` 追加/重复打开不改序/拖拽写回/删除后空洞；
   - v22→v23 position 回填、v23→v24 `workspace_hidden` 建表；
   - canonical 等价路径的 trust/rename/delete 去重；
   - Git 仓库子目录保持顶层，不被误判为 linked worktree；
   - `workspace_hidden` 隐藏/解除隐藏与历史列表过滤；
   - `delete_workspace` 级联删除 folders/prompts/referenced/sessions；
   - 删除最后打开的工作区后保留“已信任 + 隐藏”行。
2. **状态与交互层测试（`src/app/update/workspace_history/tests.rs`、`src/app/update/tests.rs`、`src/event/tests/`）**：
   - 模式切换、搜索过滤、上下选择、双击/Enter 打开、失效路径 Toast；
   - 未信任历史条目只发 `CheckTrust`，不直接创建；
   - 打开已打开工作区后焦点回到 Panes；
   - 历史/活跃拖拽整组移动、未匹配活跃项保持槽位、抖动不写库；
   - 3 击菜单 + 最终弹窗删除，父项确认文案包含 worktree 数量；
   - `NewWorkspace` 输入/粘贴/剪切后确认态复位；
   - 侧栏折叠时列表命中/键盘输入/滚动不生效。
3. **UI/几何测试（`src/ui/tests.rs`、`src/ui/sidebar/history.rs` tests、`src/ui/overlay/tests.rs`）**：
   - `[+] [≡]` 几何与折叠隐藏；
   - 历史树连接符、失效路径红色、拖拽整块反显、搜索框光标；
   - 折叠箭头只命中 `list.x + 2`；
   - 滚动偏移按视口夹取、空白区拖拽吸附最后一条、搜索态禁用折叠；
   - 删除父项确认文案包含 linked worktree 数量。

### 6.2 风险与回滚
- **Schema 迁移**：v23 增加 `position` 并回填；v24 增加 `workspace_hidden`。均为增量 DDL，不改写已有表；升级前建议备份开发库（`~/.lx-t-dev/db/`）。
- **一次性排序变化**：旧库按 `updated_at DESC` 回填 `position`，升级后首次进入历史/触发 reload 时活跃列表会按该顺序重排；之后由拖拽维护。
- **路径等价**：旧库中同一目录的多行文本在 trust/rename/delete/save-order 时按 canonical 等价合并处理；仅删除/重命名会清理等价行。
- **回滚策略**：代码集中在工作区历史相关模块；数据库新增列/表可保留，旧二进制读取时忽略。若需完全回滚，先停 lx 进程并恢复 `lx.db.v*` 备份。
