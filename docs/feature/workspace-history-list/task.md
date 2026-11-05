# 左侧栏工作区历史列表（Workspace History List）— 执行任务清单 (`task.md`)

- **状态**：**已合入 `dev`（`5061509c`，合并提交 `07553c8c`）；原 worktree 与分支已移除**
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

- [x] **Q1（数据来源与生命周期）**：所有显式创建/打开过的工作区自动持久化到 SQLite `workspace` 表（与活跃列表共用 `position` 全局顺序）；活跃列表 `Close Workspace` 仅移除内存工作区，保留历史记录。
- [x] **Q2 / Q3（完整 CRUD 边界）**：全部历史条目支持搜索查看（Read）、新建工作区（Create）、重命名（Update/Rename）、双击/回车/右键打开（Open）与级联删除（Delete）。删除需右键菜单 3 击（Delete → 1/2 → 2/2）+ 最终弹窗确认；父项会一并删除同仓库 linked worktree，确认文案明确显示数量。
- [x] **Q4（按钮位置、图标与视图切换）**：顶边框右侧为 `[+]─[≡]`（`[+]` 在左、`[≡]`/`[/]` 在右）；默认显示 `[≡]`，进入历史列表后显示 `[/]`；仅原位切换侧栏上半区 `Workspaces`，下半区 `Agents` 不变；点击 `[/]`、按 `Esc` 或成功打开历史工作区后自动返回活跃列表视图。
- [x] **Q5（列表结构、Git 树与搜索过滤）**：顶部为 `> search workspaces…` 搜索框 + `├────┤` 分割线；Git 仓库以主工作区为父项（`▾/▸ <主工作区名>`），下挂 `git worktree list` 与数据库历史 worktree（`├─/└─ <分支短名或自定义名>`），移除 `⑂`/`•` 图标；实时不区分大小写匹配名称、分支短名与路径。
- [x] **Q6（去重、重命名同步、失效路径高亮与级联删除）**：
  - 按规范化路径 `path` 去重，终端内 `cd` 不产生新历史记录；显式 “Sync workspace to terminal cwd” 会登记历史；
  - 活跃列表与历史列表重命名双向同步并持久化到 `workspace.name`；
  - 双击已打开的工作区直接切换过去，不重复新建；打开后焦点交回主区；
  - **路径不存在（`!path.is_dir()`）的历史条目使用 `style::error()`（红色）字体展示，双击弹 `"Directory not found"`；全部条目支持 3 击 + 最终弹窗级联删除；仍存在的 linked worktree 写入 `workspace_hidden`，重启后不自动复活；未信任条目打开前先走信任检查。**
- [x] **Q7（全局排序与拖拽）**：`workspace.position` 是活跃/历史唯一顺序源；两个列表均支持 Git 整组块拖拽，抖动不写库，搜索期间禁用；拖拽结束通过 `Effect::SaveWorkspaceOrder` 持久化。
- [x] **Q8（信任与隐藏）**：历史条目携带 `trusted` 快照；`TrustTrigger::OpenWorkspaceHistory` 在未信任时弹信任浮层；`workspace_hidden`（v24）持久化删除的 live worktree，`RecordWorkspace` 时解除隐藏。


---

## 2. 分阶段实施步骤拆解

### Phase 1：数据库层重构与工作区历史读写（`src/db/`）

> 目标：将工作区 DB 操作从 950 行的 `src/db/mod.rs` 拆出到 `src/db/workspace.rs`，补齐历史记录、重命名保真与级联删除接口，并接入 `Persistence` 服务。

- [x] **1.1 拆分并实现 `src/db/workspace.rs` 与更新 `src/db/mod.rs`**
  - 迁出 `is_trusted`、`trust_workspace`、`ensure_workspace`、`workspace_external_id`；`ensure_workspace` 保持 `DO NOTHING`，不覆盖用户重命名。
  - 实现 `WorkspaceRecord { name, path, position, trusted }` 与 `record_workspace_open`、`rename_workspace`、`list_workspaces`、`delete_workspace`。
  - 新增 `workspace_row_path`（精确 + canonical 等价匹配）、`save_workspace_order`、`hide_workspace` / `unhide_workspace` / `hidden_workspace_paths`。
  - v23 增加 `position` 并回填；v24 增加 `workspace_hidden`。
- [x] **1.2 新增持久化效应 `src/app/effects.rs` 与 `src/db/service.rs` 处理**
  - `RecordWorkspace`、`RenameWorkspace`、`DeleteWorkspaceHistory`、`SaveWorkspaceOrder`、`LoadWorkspaceHistory`，以及 `CheckTrust` 的 `OpenWorkspaceHistory` 触发。
  - `Persistence::pump` 首次启动自动登记初始工作区并加载历史列表。
  - `DeleteWorkspaceHistory`：分页循环取全量会话并中断子 Agent；CASCADE 删除；仍存在的 linked worktree 写入隐藏表；最后一个打开工作区保留“已信任 + 隐藏”行；重命名/删除写失败弹 Danger Toast。
  - `RecordWorkspace`：登记后解除隐藏；`SaveWorkspaceOrder`：写回 0..n 并 reload。
- [x] **1.3 Phase 1 单元测试**
  - position 追加/重复打开不改序/拖拽写回；v22→v23 回填与 v23→v24 建表；
  - canonical 等价路径的 trust/rename/delete 去重；仓库子目录不被误判为 worktree；
  - 隐藏路径持久化/解除隐藏；最后打开工作区删除后保留已信任隐藏行；
  - `delete_workspace` 级联删除 folders/prompts/referenced/sessions。


---

### Phase 2：应用状态模型与更新逻辑（`src/app/`）

> 目标：建立纯内存状态 `WorkspaceHistoryPanel`，实现模式切换、搜索过滤、双击打开、重命名同步、失效条目级联删除及菜单对接。

- [x] **2.1 新建 `src/app/state/workspace_history.rs` 并挂载到 `AppState`**
  - `WorkspaceMode`（Active/History）、`WorkspaceHistoryEntry`（含 `position`、`trusted`、`git`）、`WorkspaceHistoryPanel`（含 drag/dragging/drag_moved/reveal_selected/hidden_worktrees 等）。
  - 实现 `filtered_indices`、`visible_rows`、`tree_order`、`drag_block`、`delete_worktree_count`、`reorder_entries`/`normalize`、`clamp`（返回是否有变化）、`scroll_selected_into_view`、`refresh_existence`。
- [x] **2.2 扩展浮层与菜单目标**
  - `OverlayTarget::WorkspaceHistory(entry_index)`、`MenuCommand::OpenWorkspaceHistory` / `DeleteWorkspaceHistoryStep1/Step2`、`RenameTarget::WorkspaceHistory`、`TooltipTarget::WorkspaceMode`。
  - 删除采用菜单内状态机 + 最终 `ConfirmClose`；父项确认文案包含 linked worktree 数量。
- [x] **2.3 新建 `src/app/update/workspace_history.rs`**
  - 模式切换/搜索/导航/双击/打开；`open_workspace_history_entry` 对未信任条目发 `CheckTrust`，授予后 `open_workspace_history_trusted` 续作。
  - 排序同步：`sync_active_order_to_history`（未匹配项保持槽位）、`sync_history_entries_from_active`、`persist_workspace_order`。
  - 拖拽：`begin/drag/end_workspace_history_drag`，`drag_moved` 控制是否写库。
  - 删除：`delete_workspace_history_entry` 中断 Agent/子 Agent/Shell、关闭打开窗口、最后一工作区走隐藏行保护。
- [x] **2.4 对接创建、重命名、新建确认与菜单**
  - `create_workspace_with_cwd` / worktree / `sync_workspace_cwd` 推入 `RecordWorkspace`；`sync_workspace_cwd` 先 canonicalize。
  - `commit_rename` 双向同步；历史列表/活跃列表都落库。
  - `NewWorkspace` 单弹窗内联确认；编辑、粘贴、剪切均复位确认态。
  - `open_workspace_history_menu` 全部条目提供 Open/Rename/Delete；3 击 + 最终弹窗。
- [x] **2.5 Phase 2 单元测试**
  - 模式切换、搜索、导航、双击/Enter、失效路径 Toast；
  - 未信任条目只发 CheckTrust；打开已打开工作区后焦点回 Panes；
  - 历史/活跃拖拽双向同步、整组移动、未匹配项保持槽位、抖动不写库；
  - 3 击删除、父项 worktree 数量文案、最后打开工作区保留隐藏行；
  - NewWorkspace 输入/粘贴/剪切复位确认态。


---

### Phase 3：UI 渲染层与命中几何（`src/ui/`）

> 目标：侧栏顶边框 `[+]─[≡]`，历史列表搜索框、Git 树、失效路径红色、拖拽反显、滚动条与命中几何。

- [x] **3.1 英文文案常量**
  - 历史空态/无匹配、菜单 Open/Rename/Delete 分步确认、删除工作区/父项组确认、Tooltip、写失败/最后工作区 Toast。
- [x] **3.2 新建 `src/ui/sidebar/history.rs`**
  - 搜索框 + 分割线 + 树形条目（`▾/▸`、`├─/└─`）+ 滚动条；
  - 失效路径红色、hover 保留红字、拖拽整块反显、宽字符按显示列截断；
  - 命中：`workspace_history_scrollbar`、`visible_rows`、`search_char_at`、`row_at`、`blank_at`、`group_toggle_at`（仅 `list.x+2`）、`drop_index`（空白吸附最后一条）。
  - 滚动偏移按 `total - viewport` 夹取；搜索态禁用折叠。
- [x] **3.3 更新 `src/ui/sidebar.rs` 与 `src/ui/mod.rs`**
  - `[+]` 在左、`[≡]`/`[/]` 在右；Active/History 分派；树形父/子渲染与命中；
  - Active 列表分组箭头命中改为 `list.x+2`；侧栏折叠时 Active/Agents 命中返回 None。
- [x] **3.4 Phase 3 单元测试**
  - 按钮几何/折叠隐藏；历史树连接符、红字、拖拽反显、搜索框光标；
  - 箭头列、滚动夹取、空白拖拽吸附、搜索态禁用折叠；父项删除确认文案。


---

### Phase 4：键盘与鼠标事件层闭环（`src/input/` & `src/event/`）

> 目标：打通侧栏历史模式的键盘输入、粘贴、鼠标点击/双击、搜索框拖选、整组拖拽排序、滚轮、悬停与右键菜单。

- [x] **4.1 键盘与粘贴路由（`src/input/mod.rs` & `src/event/input.rs`）**
  - `SidebarKey` / `Routed::Sidebar` 仅在侧栏聚焦、历史模式且未折叠时生效；`Ctrl/Cmd+C` 复制选区或清空搜索、`Ctrl/Cmd+X` 剪切；粘贴压平换行。
- [x] **4.2 鼠标按下/拖拽/松开/滚轮/悬停/右键（`src/event/mouse/` & `src/event/overlay.rs`）**
  - `press.rs` 入口清理上一轮可能丢失 Up 的手势状态（搜索拖选/滚动条/行拖拽）；`hit.rs` 按滚动条 → 搜索框 → 折叠箭头 → 条目行分派。
  - 双击以条目路径 + 400ms 窗口在第二次按下判定；滚动/真正拖拽/重载使点击序列失效。
  - `drag.rs`：历史条目以 Git 整组块拖拽；`drag_moved` 只在顺序变化时置位，松开才写库；搜索期间禁用。
  - `wheel.rs`/`hover.rs`：历史滚动、悬停；拖拽/搜索拖选期间抑制 hover 与 tooltip；折叠侧栏不参与命中。
  - `overlay.rs`：右键历史条目打开全部 CRUD 菜单；空白区打开 New Workspace。
- [x] **4.3 Phase 4 集成与事件测试**
  - 点击 `[≡]`、输入过滤、双击打开并切回 Active；打开已打开工作区焦点回 Panes。
  - 3 击 + 最终弹窗删除全链路；历史/活跃拖拽互同步；抖动不写库。
  - 折叠侧栏下左右两列不命中隐藏列表/Agents；滚动后双击不误开另一条目。


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
- [x] 所有显式创建/打开的工作区自动按路径去重持久化到 SQLite，按全局 `position` 顺序排列（拖拽可重排）；终端内 `cd` 不污染历史列表。
- [x] 支持顶部搜索框实时过滤、鼠标双击/键盘回车打开、新建工作区、重命名双向同步。
- [x] 新建工作区（`NewWorkspace`）直接在首个弹窗内完成二次确认（主按钮由 `Create` 切换为 `Confirm`），不再调用第二个 `TrustWorkspace` 弹窗。
- [x] 全部工作区均支持级联删除（`Delete`），需经过「右键菜单连续 3 次点击（`Delete` → `Confirm delete (1/2)` → `Confirm delete (2/2)`）+ 最终确认弹窗（`Delete Workspace`）」后执行；若有正在运行的 Agent 流、子 Agent 或本地命令，立即强制中断并级联删除 SQLite 关联数据；父项确认文案显示同仓库 worktree 数量。
- [x] 活跃列表与历史列表共用 `workspace.position`（v23），任一侧拖拽都写回并保持相对顺序；未匹配历史的活跃工作区保持原槽位。
- [x] 历史打开未信任条目先走 `CheckTrust`；删除仍存在的 linked worktree 写入 `workspace_hidden`（v24），重启后不自动复活；最后一个打开工作区删除后保留“已信任 + 隐藏”行。
- [x] 侧栏折叠、滚动偏移夹取、选中项揭示、宽字符截断、折叠箭头命中列、Ctrl+X 确认态复位等边界修复完成。
- [x] 所有生产代码单文件 ≤ 1000 行，无 `unwrap()` / `expect()`，`cargo fmt`、`cargo clippy --all-targets -- -D warnings`、`cargo test` 全部通过。
- [x] 合并 `dev` 后修复两处 PTY 集成测试与旧交互假设的冲突：`NewWorkspace` 改为内联 Confirm，侧栏 `[≡]` 与 Prompt `[≡]` 同名按钮按最右出现消歧。
