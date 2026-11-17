# Agents 菜单、拖拽滚动与设置页加载修复 — 执行任务

> 状态：**已完成实现与代码审查修复，待用户手动验收后确认合并**
> 设计依据：[design.md](design.md) · 审查记录：[review.md](review.md)
> 基线：`.worktrees/agents-menu-drag-scroll`（分支 `feature/agents-menu-drag-scroll`）；不自动合并回 `dev`
> 规则：生产代码禁止 `unwrap()` / `expect()`；用户可见文案全部收敛到 `src/ui/text.rs`；`app/` 管状态与更新、`ui/` 只读渲染、`event/` 只分派；OS 专属代码只进 `src/platform/`。

## P0. 基线核对

- [x] 用户确认 design.md 与 task.md（已确认，按 task.md 执行）。
- [x] `git status --short` 记录现状（未改动主 checkout 下的无关目录）。
- [x] 复读 `docs/standards/project-directory-structure.md`、`docs/standards/code-writing-standards.md`、`docs/standards/tui-design-requirements.md`、`docs/standards/html-assets.md`。
- [x] 用 codegraph / codebase memory 复核相关符号与调用点。
- [x] 确认当前 `tests/web_assets.rs` 的 `SCRIPTS` 顺序与 `assets/web.html` 一致。

## P1. 终端 Agent 的 Close

### 1.1 菜单

- [x] `src/app/update/menu.rs` `open_agent_menu`：
  - [x] 命令列表头部加入 `MenuCommand::NewAgent`。
  - [x] 删除 `pane_ids().len() > 1` 的终端 Agent 裁剪逻辑。
  - [x] lx 与终端 Agent 都追加 `MenuCommand::CloseAgent`。
  - [x] 保留 `state.agent_scroll_drag = None`、目标存在性校验与 `bound` 快照。
- [x] 更新函数文档注释：终端 Agent 的 `Close` 不再受唯一窗格限制。

### 1.2 关闭语义

- [x] `src/app/update/menu.rs` 新增 `close_terminal_agent(state, workspace, tab, pane)`：
  - [x] 目标 tab 不存在直接返回。
  - [x] `pane_count > 1`：调用 `close_pane`。
  - [x] `pane_count == 1 && tab_count > 1`：调用 `close_tab`。
  - [x] `pane_count == 1 && tab_count == 1`：`create_tab(state)` 后 `close_tab(state, workspace, tab)`。
- [x] `close_agent` 的 `SidebarAgentKind::Terminal(_)` 分支改为调用 `close_terminal_agent`。
- [x] 保留 `close_agent` 开头的 `agent_entries` 复查，目标消失时不做任何事。
- [x] 确认唯一标签替换后：
  - [x] 新标签为默认 lx 视图。
  - [x] 新标签继承工作区最近模型与 `@` 候选快照。
  - [x] `active_tab` 回到 0，工作区 tab 数仍为 1。
  - [x] 旧 PTY 在下一轮 `reconcile_sessions` 被 `kill`。
  - [x] `agent_hover`、`agent_scroll_drag`、旧 pane 锚定 toast、terminal/lx selection、autoscroll 不残留。

### 1.3 测试

- [x] `src/app/update/menu/tests.rs`：
  - [x] 更新断言包含 `CloseAgent` 与 `NewAgent` 在首位。
  - [x] 新增 `terminal_close_agent_in_sole_pane_closes_tab`。
  - [x] 新增 `terminal_close_agent_in_last_tab_replaces_with_blank_tab`。
  - [x] 保持 `terminal_close_agent_removes_only_target_pane` 覆盖多窗格路径。
- [x] `src/event/tests/agent_menu.rs`：
  - [x] 唯一窗格终端 Agent 菜单包含 `New agent` / `Edit title` / `Close`。
  - [x] 菜单点击 `Close` 仍走行内二次确认。

## P2. New agent 菜单与弹窗

### 2.1 菜单模型与文案

- [x] `src/app/overlay.rs`：`MenuCommand::NewAgent`、`OverlayTarget::AgentsBlank`、`OverlayKind::NewAgent`、`Overlay::NewAgent(NewAgentDialog)`。
- [x] `src/ui/overlay/menu.rs`：`OverlayTarget::AgentsBlank` 标题与 `MenuCommand::NewAgent` 标签。
- [x] `src/ui/text.rs` & `src/ui/text/workspace.rs`：`MENU_AGENT_NEW`、`MENU_TITLE_AGENTS`、`NEW_AGENT_TITLE`、`NEW_AGENT_SEARCH`、`NEW_AGENT_EMPTY`、`NEW_AGENT_NO_MATCH`、`BUTTON_CREATE`、`TOAST_AGENT_LAUNCH_FAILED`。

### 2.2 空白命中与打开菜单

- [x] `src/ui/left_panel/mod.rs` 新增 `agents_blank_at`（排除折叠、滚动条列、条目行）。
- [x] `src/app/update/menu.rs` 新增 `open_agents_blank_menu`。
- [x] `src/event/overlay.rs` `open_right_click_menu`：在命中条目或空白时刷新 `installed_agents` 并打开对应菜单（右键滚动条列不扫 PATH、不穿透）。

### 2.3 已安装探测

- [x] `src/platform/executable.rs`（`find_executable` / `find_executable_in`；跳过空 PATH 段、Unix `libc::access(..., X_OK)`、Windows 单次解析 `PATHEXT`）。
- [x] `src/platform/mod.rs` 导出 `find_executable`。
- [x] `src/detect/mod.rs`：`ALL_AGENT_KINDS`、`launch_names`、`InstalledAgent`、`installed_agents`。
- [x] `src/app/state.rs`：`installed_agents` 初始化为空。
- [x] `src/platform/executable/tests.rs` & `src/detect/tests.rs` 覆盖权限、候选优先级、空 PATH 段与去重。

### 2.4 弹窗状态、搜索、按键与鼠标

- [x] `src/app/overlay.rs`：`NewAgentDialog`（含 `query: TextInput`、`selected`、`viewport`、`filtered_indices()`、`normalize_selection()`、`scroll_viewport` / `set_viewport`）并接入 `Overlay::input` / `input_mut` / `clear_selection`。
- [x] `src/app/update/new_agent.rs` & `src/app/update/dialog.rs`：打开/上下选择/创建/双击/搜索过滤/粘贴/剪切。
- [x] `src/input/mod.rs`：`OverlayKind::NewAgent` 映射导航与单行编辑快捷键。
- [x] `src/event/overlay.rs` & `src/event/input.rs` & `src/event/mouse/{press,drag,wheel}.rs`：搜索框点击拖选、滚轮、滚动条、行双击、`Create`/`Cancel`、`Enter` 后露出新标签。

### 2.5 弹窗渲染

- [x] `src/ui/overlay/new_agent.rs`：搜索框 + `DIVIDER_MID` + 候选列表（溢出色滚动条）+ 呼吸行 + 按钮行；空态/无匹配居中截断；`Create` 置灰；返回硬件光标。
- [x] `src/ui/overlay.rs`：导出命中/布局辅助与接入渲染分派。

### 2.6 启动通道与测试

- [x] `src/app/state/pane.rs`：`PendingAgentLaunch` 与 `Pane.pending_launch`。
- [x] `src/app/update/workspace.rs`：`create_terminal_tab` 与 `create_agent_tab`（`AGENT_LAUNCH_DELAY = 150ms`）。
- [x] `src/app/update/interaction.rs`：`next_deadline` 纳入 `pending_launch.at`。
- [x] `src/event/session.rs` & `src/event/mod.rs`：`flush_pending_agent_launches` 到点写入 `<command>\r`，缺失/失败清队列并 toast。
- [x] 覆盖 `menu/tests`、`new_agent/tests`、`agent_menu`、`new_agent/tests`（UI）、`input/tests/overlay`、`overlay/tests` 与 `session_launch`。

## P3. 拖拽中的滚轮

- [x] `src/event/mouse/wheel.rs`：删除旧的全局早退，增加四个拖拽优先分支（含 `Overlay::is_none`、`WorkspaceMode`、`prompt_panel_active && prompt_tree_mode`、`RightTab::Todo && !prompt_collapsed` 守卫与滚动条列支持）。
- [x] 更新 `src/event/tests/todo.rs`、`src/event/tests/prompt_tree.rs` 并新增工作区活跃/历史拖拽滚轮测试。

## P4. 设置页修复

- [x] `assets/settings_{models, models_card, connect, subagents}.js` 共 64 处 `element(` → `domEl(`。
- [x] `tests/web_assets.rs` 新增 `assets_do_not_call_removed_element_helper`。
- [ ] **手动确认**：用户运行 `cargo run`，打开 `/settings` 确认 Models/Providers、Quick connect、Subagents 正常渲染。

## P5. 增量需求：New agent 搜索与边缘自动滚动（`65fdc1f3`、`8435fb78`、`150de2f2`）

- [x] New agent 搜索输入、命令/别名匹配、过滤后重置首行与 `No matching agents` 空态。
- [x] `src/event/mouse/autoscroll.rs`：工作区活跃/历史、Todo、Prompt 树拖拽上下各 2 行检测区、30ms 节拍边缘自动滚动与落点同步。
- [x] `lx` 主消息列表与后台任务页（Processes/Agents/BTW）选区边缘自动滚动与 `next_deadline` 唤醒。
- [x] 补充 `src/event/tests/mouse_drag.rs` 与 `src/input/tests/overlay.rs` 回归测试。

## P6. Review 缺陷修复与规范收敛（依据 [review.md](review.md)）

- [x] **P0-1**：修复 `scroll_lx_history` 在 `follow_bottom == true` 时恒返回 `true` 导致的 33Hz 底部空转死循环；增加 `Lx::has_history_content()` 覆盖非消息历史滚动。
- [x] **P0-2**：将 `lx` 选区边缘自动滚动执行期迁入 `src/event/mouse/autoscroll.rs`，实时解析 `resolve_task_*_visible` / `history_rows` / `content_geom` 并用 `unit_cell_range` 保留 `Word`/`Line`/表格/代码块选区粒度。
- [x] **P0-3**：将 `Overlay::NewAgent` 接入 `Overlay::input` / `input_mut` / `clear_selection`、`overlay_input_rect`、粘贴/剪切同步及键盘 `Enter` 标签露出；改搜索词同步清空 `new_agent_click` 与 `viewport`。
- [x] **P1-1**：三套 `arm_*` 在同方向边缘移动时保留既有 `next_at` 防止饥饿，仅在首次越界或距离增大时立即跳滚。
- [x] **P1-2**：`autoscroll::drag_active` 校验 `Todo` `dragging || distance > 0` 与 `PromptTree` `enabled && (dragging || distance > 0)`，防止边缘点击抖动误触合并/移动；`WorkspaceHistory` 校验搜索框为空。
- [x] **P1-3**：`arm_drag` 放宽工作区横向限制、将 Todo/Prompt 树放宽至右栏全宽（含滚动条列）并在右栏内纵向钳位保持落点，拖出右栏进入主区取消。
- [x] **P1-4**：移除 `app/overlay.rs` 对 `ui::text::agent_label` 的冗余依赖；为 `NewAgentDialog` 增加 `DIVIDER_MID`、呼吸空行、空态居中截断、滚轮与溢出滚动条、`new_agent_visible_start` 过滤夹取。
- [x] **P1-5**：修复既有 Prompt 选区 `autoscroll_step` 在底部无法上滚、在顶部 30ms 空转的对称边界缺陷。
- [x] **P2-1**：补全 `wheel.rs`/`autoscroll.rs`/`drag.rs` 的浮层/模式守卫，并在 `handle_mouse_down_right`、`open_pane_menu`、`open_prompt_*_menu`、`close_pane`、`close_tab` 对称清理在途拖拽、自动滚动与锚定 toast。
- [x] **P2-2**：右键 Agents 滚动条列不再无条件扫描 PATH；`platform/executable.rs` 跳过空 PATH 段、Unix 使用 `libc::access(X_OK)`、Windows 单次解析 `PATHEXT`。
- [x] **P2-3**：拆分超限文件：`src/app/state.rs` → `state/interaction.rs`（842 行）、`src/app/update/selection.rs` → `selection/{mod, lx, autoscroll}.rs`（490/180/255 行）、`src/input/tests.rs` → `input/tests/{mod, overlay}.rs`（949/583 行）；更新 `dialog.rs` 注释，补齐 `BUTTON_CREATE` 与 `Overlay::NewAgent` 回归测试。
- [x] **P2-4**：同步更新 `docs/standards/tui-design-requirements.md`、`design.md`、`task.md` 与 `review.md`。

## P7. 验证与交付

- [x] `cargo fmt`
- [x] `cargo clippy --all-targets -- -D warnings`（0 errors, 0 warnings）
- [x] `cargo test`（lib `5838 passed` + 全部集成测试通过）
- [x] `cargo test --test web_assets`（3 passed）
- [x] 生产代码无新增 `unwrap()`/`expect()`、可见文案集中 `src/ui/text.rs` 且为英文、OS 代码仅在 `src/platform/`。
- [ ] **手动验收与合并**：待用户退出旧进程并 `cargo run` 验证 TUI 与 `/settings`，确认后合并回 `dev`。

## 执行记录

- 分支：`feature/agents-menu-drag-scroll`（worktree `.worktrees/agents-menu-drag-scroll`）。
- 已提交历史：`87c9fbcb`（P1）、`e5f8823b`（P2）、`6fabb6a9`（P3）、`e8742e7c`（P4）、`a30c6a1e`（文档）、`65fdc1f3`（New agent 搜索）、`8435fb78`（边缘自动滚动）、`150de2f2`（按键路由测试）。
- Review 修复（当前工作区未提交修改）：按 [review.md](review.md) 完成全部 P0–P2 修复与文件拆分，并更新对应设计/任务/规范文档；全量 `cargo fmt`、`cargo clippy --all-targets -- -D warnings`、`cargo test` 与 `cargo test --test web_assets` 全绿。
- 合并回 `dev` 前需用户明确确认。
