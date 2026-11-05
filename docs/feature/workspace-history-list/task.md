# 左侧栏工作区历史列表（Workspace History List）— 执行任务清单 (`task.md`)

- **状态**：**已在 `.worktrees/workspace-history-list` (`feature/workspace-history-list`) 完成实现与测试，待用户确认合并**
- **设计依据**：`docs/feature/workspace-history-list/design.md`

---

## 0. 开工门禁与硬性规范

- **门禁**：必须等用户审核并明确确认本目录下的 `design.md` 与 `task.md` 后，才可修改 `src/` 与 `tests/` 代码。
- **架构红线**：
  - `app/` 只管纯内存状态与状态转换，严禁直接读写磁盘 DB 或引用 `ui/`；
  - `ui/` 只读渲染，严禁修改状态或执行 IO（路径存在性 `exists: bool` 存于 `WorkspaceHistoryEntry` 状态中，不在 `ui/` 渲染帧内调 `Path::is_dir()`）；
  - 所有 TUI 可见文案统一使用英文并集中定义在 `src/ui/text.rs`；
  - 生产代码单文件严禁超过 1000 行，严禁使用 `unwrap()` / `expect()`；
  - 未获明确授权禁止执行任何修改 Git 状态的命令。

---

## 1. 需求与边界确认清单（Grill Me 已确认）

- [x] **Q1（数据来源与生命周期）**：所有显式创建/打开过的工作区自动持久化到 SQLite `workspace` 表（按最近打开时间倒序）；活跃列表 `Close Workspace` 仅移除内存工作区，保留历史记录。
- [x] **Q2 / Q3（常规条目 CRUD 边界）**：磁盘路径存在的常规历史条目支持搜索查看（Read）、新建工作区（Create）、重命名工作区（Update/Rename）和双击/回车/右键打开（Open），不提供删除。
- [x] **Q4（按钮位置、图标与视图切换）**：列表按钮位于左侧栏 `Workspaces` 顶边框右上角、`[+]` 按钮左侧 1 列（`[≡] [+]`）；默认显示 `[≡]`，进入历史列表后显示 `[/]`；仅原位切换侧栏上半区 `Workspaces`，下半区 `Agents` 不变；点击 `[/]`、按 `Esc` 或成功打开历史工作区后自动返回活跃列表视图。
- [x] **Q5（列表结构、行样式与搜索过滤）**：无文件夹扁平列表，顶部为 `> Search...` 搜索框 + `├────┤` 分割线；每行展示 `❯ • <name>`（未选中 `  • <name>`）；实时不区分大小写过滤名称与路径。
- [x] **Q6（去重、重命名同步与失效路径高亮+级联删除）**：
  - 按规范化路径 `path` 去重，终端内 `cd` 不产生新历史记录；
  - 活跃列表与历史列表重命名双向同步并持久化到 `workspace.name`；
  - 双击已打开的工作区直接切换过去，不重复新建；
  - **路径不存在（`!path.is_dir()`）的历史条目使用 `style::error()`（红色）字体展示，双击弹 `"Directory not found"`，且右键菜单额外支持 `Delete`（行内二次确认 `Confirm delete`），确认后直接级联删除 `workspace` 及关联的所有提示词、文件夹、引用目录和聊天会话**。

---

## 2. 分阶段实施步骤拆解

### Phase 1：数据库层重构与工作区历史读写（`src/db/`）

> 目标：将工作区 DB 操作从 950 行的 `src/db/mod.rs` 拆出到 `src/db/workspace.rs`，补齐历史记录、重命名保真与级联删除接口，并接入 `Persistence` 服务。

- [x] **1.1 拆分并实现 `src/db/workspace.rs`（预估 ~160 行）与更新 `src/db/mod.rs`（降至 ~890 行）**
  - 在 `src/db/mod.rs` 声明 `mod workspace; pub use workspace::WorkspaceRecord;`，迁出 `is_trusted`、`trust_workspace`、`ensure_workspace`、`workspace_external_id`。
  - 修正 `Database::ensure_workspace(&self, path: &Path)` 为 `ON CONFLICT(path) DO NOTHING`，避免后台保存提示词/会话时把用户重命名过的 `workspace.name` 覆盖回目录名。
  - 实现核心方法：
    ```rust
    pub struct WorkspaceRecord {
        pub name: String,
        pub path: PathBuf,
    }

    impl Database {
        pub fn record_workspace_open(&self, path: &Path, default_name: &str) -> Result<String>;
        pub fn rename_workspace(&self, path: &Path, name: &str) -> Result<()>;
        pub fn list_workspaces(&self) -> Result<Vec<WorkspaceRecord>>;
        pub fn delete_workspace(&self, path: &Path) -> Result<()>;
    }
    ```
- [x] **1.2 新增持久化效应 `src/app/effects.rs` 与 `src/db/service.rs` 处理（预估 +65 行）**
  - 在 `src/app/effects.rs` 的 `Effect` 枚举中新增：
    ```rust
    /// 记录工作区显式打开并刷新历史列表（若库中已有自定义名则回填给活跃工作区）。
    RecordWorkspace { path: PathBuf, name: String },
    /// 持久化工作区重命名并刷新历史列表。
    RenameWorkspace { path: PathBuf, name: String },
    /// 删除路径已失效的工作区历史记录（级联删除关联资产）。
    DeleteWorkspaceHistory { path: PathBuf },
    /// 从数据库重新加载工作区历史列表。
    LoadWorkspaceHistory,
    ```
  - 在 `src/db/service.rs`：
    - `pump()` 末尾新增 `changed |= self.ensure_workspace_history_loaded(state);`（首次启动时自动将初始工作区登记入库并加载全部历史列表）；
    - 在 `execute()` 中处理上述 4 个 `Effect`，完成后调用 `reload_workspace_history(state)` 更新 `state.workspace_history.entries`。
- [x] **1.3 Phase 1 单元测试（`src/db/tests.rs`）**
  - 测试 `record_workspace_open` 排序与去重；
  - 测试 `rename_workspace` 后调用 `ensure_workspace` 和 `record_workspace_open` 保留自定义名；
  - 测试 `delete_workspace` 级联删除 `workspace_folder`、`workspace_prompt`、`workspace_referenced_folder` 与 `agent_session`。

---

### Phase 2：应用状态模型与更新逻辑（`src/app/`）

> 目标：建立纯内存状态 `WorkspaceHistoryPanel`，实现模式切换、搜索过滤、双击打开、重命名同步、失效条目级联删除及菜单对接。

- [x] **2.1 新建 `src/app/state/workspace_history.rs`（预估 ~150 行）并挂载到 `AppState`（`src/app/state.rs`）**
  - 定义 `WorkspaceMode`（`Active` / `History`）、`WorkspaceHistoryEntry { name, path, exists }`、`WorkspaceHistoryPanel`。
  - 实现 `filtered_indices()`、`refresh_existence()`、`clamp(visible_rows)`、`scroll_selected_into_view(visible_rows)`。
  - 在 `AppState` 中新增字段 `pub workspace_history: WorkspaceHistoryPanel`。
- [x] **2.2 扩展浮层与菜单目标（`src/app/overlay.rs` & `src/app/tooltip.rs`）**
  - `OverlayTarget` 新增 `WorkspaceHistory(usize)`（参数为 `entries` 中的真实下标 `entry_index`）。
  - `MenuCommand` 新增 `OpenWorkspaceHistory` 与 `DeleteWorkspaceHistory`；并将 `MenuCommand::DeleteWorkspaceHistory` 加入 `MenuCommand::requires_confirm()`（启用行内二次确认 `"Confirm delete"`）。
  - `RenameTarget` 新增 `WorkspaceHistory(usize)`。
  - `TooltipTarget` 新增 `WorkspaceMode`。
- [x] **2.3 新建 `src/app/update/workspace_history.rs`（预估 ~260 行）**
  - 实现视图切换与同步：
    ```rust
    pub fn workspace_history_mode(state: &AppState) -> bool;
    pub fn toggle_workspace_mode(state: &mut AppState) -> bool;
    pub fn show_workspace_history(state: &mut AppState) -> bool;
    pub fn exit_workspace_history(state: &mut AppState) -> bool;
    pub fn apply_workspace_history(state: &mut AppState, records: Vec<(String, PathBuf)>) -> bool;
    ```
  - 实现列表导航、搜索编辑与双击打开：
    ```rust
    pub fn move_workspace_history_selection(state: &mut AppState, delta: isize, visible_rows: usize) -> bool;
    pub fn select_workspace_history_row(state: &mut AppState, visible_row: usize) -> bool;
    pub fn workspace_history_row_double_click(state: &mut AppState, visible_row: usize, now: Instant) -> bool;
    pub fn activate_workspace_history_selected(state: &mut AppState) -> bool;
    pub fn open_workspace_history_row(state: &mut AppState, visible_row: usize) -> bool;
    pub fn open_workspace_history_entry(state: &mut AppState, entry_index: usize) -> bool;
    pub fn delete_workspace_history_entry(state: &mut AppState, entry_index: usize, visible_rows: usize) -> bool;
    pub fn apply_workspace_history_search_key(state: &mut AppState, key: SidebarKey, visible_rows: usize) -> bool;
    pub fn apply_workspace_history_paste(state: &mut AppState, text: &str, visible_rows: usize) -> bool;
    ```
- [x] **2.4 对接工作区创建、重命名与右键菜单（`src/app/update/workspace.rs`、`worktree.rs`、`dialog.rs`、`menu.rs`）**
  - `create_workspace_with_cwd` 与 `worktree.rs` 新建工作区时推入 `Effect::RecordWorkspace { path, name }`。
  - `src/app/update/dialog.rs` 的 `commit_rename`：
    - `RenameTarget::Workspace(index)`：除更新内存 `workspace.name` 外，同步更新 `workspace_history` 同路径条目并推入 `Effect::RenameWorkspace`；
    - `RenameTarget::WorkspaceHistory(entry_index)`：更新 `workspace_history` 条目名，同步更新 `state.workspaces` 中已打开的同路径工作区名称（置 `name_is_manual = true`），并推入 `Effect::RenameWorkspace`。
  - `src/app/update/menu.rs`：
    - 新增 `open_workspace_history_menu(state, visible_row, anchor)`：刷新目标条目 `exists = entry.path.is_dir()`，当 `exists == true` 时菜单为 `[OpenWorkspaceHistory, RenameWorkspace]`；当 `exists == false` 时菜单为 `[OpenWorkspaceHistory, RenameWorkspace, DeleteWorkspaceHistory]`；
    - 在 `activate_menu` 中分发 `OpenWorkspaceHistory`、`RenameWorkspace`（針對 `OverlayTarget::WorkspaceHistory`）与 `DeleteWorkspaceHistory`。
- [x] **2.5 Phase 2 单元测试（`src/app/update/tests.rs`）**
  - 测试模式切换、搜索过滤、上下键选择、双击打开已存在/未打开工作区、双击不存在路径弹 Danger Toast、重命名双向同步、失效条目菜单含 `Delete` 及二次确认删除。

---

### Phase 3：UI 渲染层与命中几何（`src/ui/`）

> 目标：在侧栏顶边框渲染 `[≡]` / `[/]` 按钮，在 `src/ui/sidebar/history.rs` 实现历史列表渲染（含搜索框、分割线、失效路径红色字体、滚动条）及命中测试。

- [x] **3.1 新增英文文案常量（`src/ui/text.rs` & `src/ui/overlay/menu.rs` & `src/ui/tooltip.rs`）**
  - `src/ui/text.rs` 新增：
    ```rust
    pub const WORKSPACE_HISTORY_EMPTY: &str = "no workspaces yet";
    pub const WORKSPACE_HISTORY_NO_MATCHES: &str = "no matching workspaces";
    pub const MENU_OPEN_WORKSPACE_HISTORY: &str = "Open";
    pub const MENU_DELETE_WORKSPACE_HISTORY: &str = "Delete";
    ```
  - `TooltipTarget::WorkspaceMode` 在 `text::tooltip_hint` 中映射：
    ```rust
    TooltipTarget::WorkspaceMode => match state.workspace_history.mode {
        WorkspaceMode::Active => "open workspace history",
        WorkspaceMode::History => "open active workspaces",
    },
    ```
  - `src/ui/overlay/menu.rs` 为 `MenuCommand::OpenWorkspaceHistory` 与 `MenuCommand::DeleteWorkspaceHistory` 映射标签。
  - `src/ui/tooltip.rs` 注册 ` (TooltipTarget::WorkspaceMode, area)`。
- [x] **3.2 新建 `src/ui/sidebar/history.rs`（预估 ~260 行）**
  - 实现 `render(frame, area, state) -> Option<(u16, u16)>`：
    - 第 0 行：`> Search...` 搜索框（复用 `widgets::input::view` 与 `build_styled_spans`）；
    - 第 1 行：`├────┤` 分割线；
    - 第 2 行起：历史条目列表（`entry.exists == true` 用 `style::text()` + `style::muted()` 图标；`entry.exists == false` 用 `highlight.patch(style::error())` 红色前景）+ 右侧滚动条。
  - 实现命中几何函数（渲染与鼠标事件严格共用）：
    ```rust
    pub fn workspace_history_scrollbar(view: &ViewLayout, state: &AppState) -> Option<ScrollbarLayout>;
    pub fn workspace_history_visible_rows(view: &ViewLayout, state: &AppState) -> Option<usize>;
    pub fn workspace_history_search_char_at(view: &ViewLayout, state: &AppState, column: u16, row: u16) -> Option<usize>;
    pub fn workspace_history_row_at(view: &ViewLayout, state: &AppState, column: u16, row: u16) -> Option<usize>;
    pub fn workspace_history_blank_at(view: &ViewLayout, state: &AppState, column: u16, row: u16) -> bool;
    ```
- [x] **3.3 更新 `src/ui/sidebar.rs` 与 `src/ui/mod.rs`（预估 +45 行）**
  - `src/ui/sidebar.rs`：
    - 新增 `pub fn workspace_mode_button(view: &layout::ViewLayout) -> Option<Rect>`，位于 `add_workspace_button` 左侧 1 列（`x = add.x - 4, y = sidebar.y, width = 3`）；
    - 绘制 `[≡]`（`WorkspaceMode::Active`）或 `[/]`（`WorkspaceMode::History`），样式 `style::accent()`；
    - `sidebar::render` 根据 `state.workspace_history.mode` 分派上半区渲染，并返回搜索框硬件光标 `Option<(u16, u16)>`；
    - 在 `WorkspaceMode::Active` 专属命中函数（`workspace_item_at`、`workspace_group_toggle_at`、`workspace_scrollbar`）中增加 `state.workspace_history.mode == WorkspaceMode::Active` 守卫，防止在历史视图下误触发活跃工作区点击。
  - `src/ui/mod.rs`：接收 `sidebar::render` 返回的 `sidebar_cursor` 并并入 `base_cursor`。
- [x] **3.4 Phase 3 单元测试（`src/ui/sidebar/tests.rs` & `src/ui/tests.rs`）**
  - 测试 `workspace_mode_button` 几何位置（`[+]` 左侧 1 列）及折叠态隐藏；
  - 测试历史列表渲染（搜索框、分割线、正常项 `style::text()`、路径不存在项 `style::error()` 红色、硬件光标位置）。

---

### Phase 4：键盘与鼠标事件层闭环（`src/input/` & `src/event/`）

> 目标：打通侧栏历史模式的键盘输入、粘贴、鼠标点击/双击、搜索框拖拽选区、滚轮滚动、悬停高亮与右键菜单，同时保持 `src/event/mouse/press.rs` 低于 1000 行。

- [x] **4.1 键盘与粘贴路由（`src/input/mod.rs` & `src/event/input.rs`）**
  - `src/input/mod.rs`：新增 `SidebarKey` 与 `Routed::Sidebar(SidebarKey)`，当 `focus == FocusedContainer::Sidebar` 时映射按键。
  - `src/event/input.rs`：
    - 处理 `Routed::Sidebar(key)`（含 `Ctrl/Cmd+C` 复制搜索框选区或清空搜索词、`Ctrl/Cmd+X` 剪切搜索框选区、`Enter` 打开选中项后调用 `ensure_workspace_visible` 与 `reveal_active_tab`）；
    - `handle_paste_event`：当 `state.sidebar_focused()` 且处于 `WorkspaceMode::History` 时将粘贴文本写入历史搜索框。
- [x] **4.2 鼠标左键、拖拽、双击松开、滚轮、悬停与右键菜单（`src/event/mouse/` & `src/event/overlay.rs`）**
  - `src/event/mouse/hit.rs` & `src/event/mouse/press.rs`：
    - 将侧栏左键按下分支抽取为 `hit::handle_sidebar_press(mouse, state, view, dirty) -> bool`，使 `press.rs` 行数降至 ~950 行；
    - 支持点击 `workspace_mode_button` 切换模式；
    - 支持 `WorkspaceMode::History` 下的滚动条点击/拖拽、搜索框点击定位光标、历史条目按下选中与记录双击时间戳。
  - `src/event/mouse/drag.rs`：
    - 支持历史列表滚动条拖拽与搜索框鼠标拖选；
    - 左键松开时若命中历史条目且未发生拖拽，判定 400ms 同行双击：双击调用 `open_workspace_history_row(state, row)` 并同步视口可见性。
  - `src/event/mouse/wheel.rs`：在 `WorkspaceMode::History` 下滚动历史列表并重算 `hover`。
  - `src/event/mouse/hover.rs`：在 `WorkspaceMode::History` 下更新 `state.workspace_history.hover`。
  - `src/event/overlay.rs`：`open_right_click_menu` 在 `WorkspaceMode::History` 下优先命中历史条目（打开 `open_workspace_history_menu`）或历史空白区（打开 `open_workspace_list_menu`）。
- [x] **4.3 Phase 4 集成与事件测试（`src/event/tests/`）**
  - 测试鼠标点击 `[≡]` 打开历史列表、输入搜索词、双击条目打开工作区并自动切回 `Active` 模式；
  - 测试右键有效条目与右键失效路径条目的菜单项差异及级联删除全链路。

---

## 3. 验证命令

完成所有阶段后按影响范围与全量要求依次执行：

```bash
rtk cargo fmt --check
rtk cargo clippy --all-targets -- -D warnings
rtk cargo test
```

---

## 4. 完成定义（DoD）

- [x] 左侧栏 `Workspaces` 顶边框右上角显示 `[+]─[≡]`（`[+]` 在左、`[≡]` 在右，间隔 1 列），点击 `[≡]` 切换为 `[/]` 并在上半区展示工作区历史列表（下半区 `Agents` 不变）。
- [x] 在 `ws list`（历史列表视图）中通过 `[+]` 或右键 `New Workspace` 新建条目时，校验并信任路径后仅写入历史列表、清空搜索词并自动选中新条目，保持停留在 `ws list` 而不直接打开工作区（对齐右侧 Prompt List）。
- [x] 修复 `startup_trust` 在 linked worktree 启动时将子工作区名写入主工作区路径的命名污染，并在加载历史列表时对已污染的主工作区名称自动自愈回主工作区目录名（如 `lx-tui`）。
- [x] 历史列表（`[≡]`）支持 Git 项目树形结构且移除 `⑂` 图标：自动扫描 `git worktree list` 并合并数据库历史 worktree，父节点显示 `▾/▸ <主工作区名>`（点击箭头可折叠/展开），子节点显示 `├─/└─ <分支短名或自定义名>`；从历史列表打开 linked worktree 时自动补开主工作区父节点。
- [x] 所有显式创建/打开的工作区自动按路径去重持久化到 SQLite，按最近打开时间倒序排列；终端内 `cd` 不污染历史列表。
- [x] 支持顶部搜索框实时过滤、鼠标双击/键盘回车打开、新建工作区、重命名双向同步。
- [x] 新建工作区（`NewWorkspace`）直接在首个弹窗内完成二次确认（主按钮由 `Create` 切换为 `Confirm`），不再调用第二个 `TrustWorkspace` 弹窗。
- [x] 全部工作区均支持级联删除（`Delete`），需经过「右键菜单连续 3 次点击（`Delete` → `Confirm delete (1/2)` → `Confirm delete (2/2)`）+ 最终确认弹窗（`Delete Workspace`）」后执行；若有正在运行的 Agent 流、子 Agent 或本地命令，立即强制中断并级联删除 SQLite 关联数据。
- [x] 所有生产代码单文件 ≤ 1000 行，无 `unwrap()` / `expect()`，`cargo fmt`、`cargo clippy --all-targets -- -D warnings`、`cargo test` 全部通过。
