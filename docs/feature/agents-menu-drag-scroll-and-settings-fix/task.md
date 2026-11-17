# Agents 菜单、拖拽滚轮与设置页加载修复 — 执行任务

> 状态：**待用户审核，未开始实现**
> 设计依据：[design.md](design.md)
> 基线：`.worktrees/agents-menu-drag-scroll`（分支 `feature/agents-menu-drag-scroll`）；每个任务完成后提交一次；不自动合并回 `dev`
> 规则：生产代码禁止 `unwrap()` / `expect()`；用户可见文案全部收敛到 `src/ui/text.rs`；`app/` 管状态与更新、`ui/` 只读渲染、`event/` 只分派；OS 专属代码只进 `src/platform/`。

## P0. 基线核对

- [ ] 用户确认 design.md 与 task.md（已确认，按 task.md 执行）。
- [ ] `git status --short` 记录现状（允许存在用户未跟踪的 `docs/feature/sandbox-permissions/`，不得改动；该目录只在主 checkout，不在本 worktree）。
- [ ] 复读 `docs/standards/project-directory-structure.md`、`docs/standards/code-writing-standards.md`、`docs/standards/tui-design-requirements.md`、`docs/standards/html-assets.md`。
- [ ] 用 codegraph / codebase memory 复核以下符号与调用点：
  - `open_agent_menu`、`activate_menu`、`close_agent`、`close_pane`、`close_tab`、`create_tab`、`create_terminal_in_workspace`
  - `MenuCommand`、`OverlayTarget`、`OverlayKind`、`Overlay`
  - `AgentTitleDialog`、`apply_agent_title_key`、`agent_title_shell`、`agent_title_row_at`
  - `reconcile_sessions`、`write_to_pane`、`PtySession::spawn`
  - `WorkspaceHistoryPanel.drag`、`workspace_drag`、`TodoDrag`、`PromptDrag`
  - `handle_mouse_wheel`、`vertical_wheel_delta`、`scroll_workspace_list`、`scroll_workspace_history`、`scroll_todo`、`scroll_prompt_tree`
  - `assets/settings_*.js` 中的 `element(` 调用点
- [ ] 确认当前 `tests/web_assets.rs` 的 `SCRIPTS` 顺序与 `assets/web.html` 一致。

## P1. 终端 Agent 的 Close

### 1.1 菜单

- [ ] `src/app/update/menu.rs` `open_agent_menu`：
  - [ ] 命令列表头部加入 `MenuCommand::NewAgent`。
  - [ ] 删除 `pane_ids().len() > 1` 的终端 Agent 裁剪逻辑。
  - [ ] lx 与终端 Agent 都追加 `MenuCommand::CloseAgent`。
  - [ ] 保留 `state.agent_scroll_drag = None`、目标存在性校验与 `bound` 快照。
- [ ] 更新函数文档注释：终端 Agent 的 `Close` 不再受唯一窗格限制。

### 1.2 关闭语义

- [ ] `src/app/update/menu.rs` 新增 `close_terminal_agent(state, workspace, tab, pane)`：
  - [ ] 目标 tab 不存在直接返回。
  - [ ] `pane_count > 1`：调用 `close_pane`。
  - [ ] `pane_count == 1 && tab_count > 1`：调用 `close_tab`。
  - [ ] `pane_count == 1 && tab_count == 1`：`create_tab(state)` 后 `close_tab(state, workspace, tab)`。
- [ ] `close_agent` 的 `SidebarAgentKind::Terminal(_)` 分支改为调用 `close_terminal_agent`。
- [ ] 保留 `close_agent` 开头的 `agent_entries` 复查，目标消失时不做任何事。
- [ ] 确认唯一标签替换后：
  - [ ] 新标签为默认 lx 视图。
  - [ ] 新标签继承工作区最近模型与 `@` 候选快照。
  - [ ] `active_tab` 回到 0，工作区 tab 数仍为 1。
  - [ ] 旧 PTY 在下一轮 `reconcile_sessions` 被 `kill`。
  - [ ] `agent_hover`、`agent_scroll_drag`、旧 pane 锚定 toast、terminal selection/scroll drag、selection autoscroll 不残留。

### 1.3 测试

- [ ] `src/app/update/menu/tests.rs`：
  - [ ] 原 `terminal_agent_menu_omits_close_for_last_pane` 改为断言包含 `CloseAgent`。
  - [ ] 原 `terminal_agent_menu_offers_close_only_with_multiple_panes` 保持包含 `CloseAgent`，并断言 `NewAgent` 在首位。
  - [ ] 新增 `terminal_close_agent_in_sole_pane_closes_tab`：多标签、唯一窗格，关闭后 tab 数减一。
  - [ ] 新增 `terminal_close_agent_in_last_tab_replaces_with_blank_tab`：唯一标签，关闭后仍 1 个 tab、根窗格 `view == PaneView::Lx`、无 agent。
  - [ ] 新增 `terminal_close_agent_removes_only_target_pane` 继续覆盖多窗格路径。
- [ ] `src/event/tests/agent_menu.rs`：
  - [ ] 原 `terminal_agent_in_sole_pane_offers_only_edit_title` 改为包含 `Edit title` + `Close` + `New agent`。
  - [ ] 菜单点击 `Close` 仍走行内二次确认。

## P2. New agent 菜单与弹窗

### 2.1 菜单模型

- [ ] `src/app/overlay.rs`：
  - [ ] `MenuCommand::NewAgent`。
  - [ ] `OverlayTarget::AgentsBlank { workspace: usize }`。
  - [ ] `MenuCommand::NewAgent` 不进入 `requires_confirm()`。
  - [ ] `OverlayKind::NewAgent`。
  - [ ] `Overlay::NewAgent(NewAgentDialog)`。
  - [ ] `Overlay::kind()` 映射。
- [ ] `src/ui/overlay/menu.rs` `menu_title`：
  - [ ] `OverlayTarget::AgentsBlank { .. } => text::MENU_TITLE_AGENTS`。
- [ ] `src/ui/overlay/menu.rs` `menu_labels`：
  - [ ] `MenuCommand::NewAgent => text::MENU_AGENT_NEW`。
- [ ] `src/ui/text.rs` 新增：
  - [ ] `MENU_AGENT_NEW = "New agent"`
  - [ ] `MENU_TITLE_AGENTS = "Agents"`
  - [ ] `NEW_AGENT_TITLE = "New agent"`
  - [ ] `NEW_AGENT_EMPTY = "No coding agents installed"`
  - [ ] `BUTTON_CREATE = "Create"`
  - [ ] `TOAST_AGENT_LAUNCH_FAILED = "Failed to start agent"`

### 2.2 空白命中与打开菜单

- [ ] `src/ui/left_panel/mod.rs` 新增 `agents_blank_at`：
  - [ ] `agents_list_rect` 不存在返回 false。
  - [ ] `list.contains` 不成立返回 false。
  - [ ] `agent_scrollbar` 命中返回 false。
  - [ ] `agent_item_index_at` 命中返回 false。
  - [ ] 其余（条目下方空行、空列表、底部半个条目）返回 true。
- [ ] `src/app/update/menu.rs` 新增 `open_agents_blank_menu(state, workspace, anchor)`：
  - [ ] 清 `agent_scroll_drag`、`agent_hover`。
  - [ ] `commands = vec![MenuCommand::NewAgent]`。
  - [ ] `target = OverlayTarget::AgentsBlank { workspace }`。
- [ ] `src/event/overlay.rs` `open_right_click_menu`：
  - [ ] 在 Agents 分支前刷新 `state.installed_agents = crate::detect::installed_agents()`。
  - [ ] 先命中 `agent_item_index_at` → `open_agent_menu`。
  - [ ] 再命中 `agents_blank_at` → `open_agents_blank_menu`。
  - [ ] 空白分支必须在 Prompt/Todo/窗格分支之前 `return true`。
- [ ] 确认右键滚动条列、折叠按钮不会打开 New agent 菜单。

### 2.3 已安装探测

- [ ] `src/platform/executable.rs`（新增）：
  - [ ] `find_executable(names: &[&str]) -> Option<(String, PathBuf)>`。
  - [ ] `find_executable_in(names, path)` 可测试入口。
  - [ ] Unix：`metadata().is_file()` + `permissions().mode() & 0o111 != 0`。
  - [ ] Windows：按 `PATHEXT` 或无扩展名逐个检查。
  - [ ] 不依赖外部命令，不新增依赖。
- [ ] `src/platform/mod.rs`：
  - [ ] `mod executable;`
  - [ ] `pub use executable::find_executable;`
- [ ] `src/detect/mod.rs`：
  - [ ] `ALL_AGENT_KINDS: [AgentKind; 8]`。
  - [ ] `launch_names(kind) -> &'static [&'static str]`，别名与 `identify_agent` 对齐。
  - [ ] `InstalledAgent { kind, command }`。
  - [ ] `installed_agents() -> Vec<InstalledAgent>`，按 `ALL_AGENT_KINDS` 固定顺序。
  - [ ] 更新模块文档：进程识别是纯算法；安装探测经 platform 层。
- [ ] `src/app/state.rs`：
  - [ ] `pub installed_agents: Vec<crate::detect::InstalledAgent>`。
  - [ ] `new()` / `demo()` 初始化为空。
- [ ] 单元测试：
  - [ ] 临时目录内放可执行文件与不可执行文件，验证 Unix 权限判定。
  - [ ] 多个候选名只返回第一个。
  - [ ] 空 PATH / 不存在返回 None。
  - [ ] `installed_agents()` 用注入的候选顺序去重（同 AgentKind 只一项）。

### 2.4 弹窗状态与按键

- [ ] `src/app/overlay.rs` 新增 `NewAgentDialog`：
  - [ ] `workspace: usize`
  - [ ] `agents: Vec<InstalledAgent>`
  - [ ] `selected: usize`
  - [ ] `new(workspace, agents)`
  - [ ] `selected_agent()`
  - [ ] `move_selection(delta)` / `select(index)`（钳制，不循环）
- [ ] `src/app/update/new_agent.rs`（新增）：
  - [ ] `open_new_agent_dialog(state, workspace)`：校验 workspace，快照 `installed_agents`，重置 `new_agent_click`。
  - [ ] `move_new_agent_selection(state, delta)`：空列表 no-op，选中钳制。
  - [ ] `set_new_agent_selection(state, index)`：越界 no-op。
  - [ ] `new_agent_double_click(state, index, now)`：400ms 窗口，同 index 第二次返回 true。
  - [ ] `create_selected_agent(state)`：取选中项，关弹窗，调用 `create_agent_tab`；空列表/目标失效 no-op。
  - [ ] `apply_new_agent_key(state, key)`：Esc/Up/Down/Enter 分派，其余忽略。
- [ ] `src/app/update.rs` 导出新模块与函数。
- [ ] `src/app/update/dialog.rs`：
  - [ ] 在 `Overlay::AgentTitle` 前置分支后增加 `Overlay::NewAgent` 分支，调用 `apply_new_agent_key` 并返回 None。
  - [ ] `OverlayKey::Enter` 的穷尽列表加入 `OverlayKind::NewAgent`。
- [ ] `src/input/mod.rs`：
  - [ ] `overlay_key` 增加 `OverlayKind::NewAgent`：Esc/Up/Down/Enter，其余 None。
- [ ] `src/app/state.rs`：
  - [ ] `pub new_agent_click: Option<(usize, Instant)>`。
  - [ ] 默认 `None`。
- [ ] `src/app/update/menu.rs` `activate_menu`：
  - [ ] `(MenuCommand::NewAgent, OverlayTarget::Agent { workspace, .. } | OverlayTarget::AgentsBlank { workspace })` → `open_new_agent_dialog`。

### 2.5 弹窗渲染

- [ ] `src/ui/overlay/new_agent.rs`（新增）：
  - [ ] `NEW_AGENT_WIDTH = 48`、`NEW_AGENT_MAX_ROWS = 8`、按钮 `[Create, Cancel]`。
  - [ ] `new_agent_shell(screen, row_count)`：高度 `row_count.clamp(1, 8) + 5`。
  - [ ] `new_agent_max_rows(shell)`。
  - [ ] `new_agent_visible_start(dialog, max_rows)`：保证选中项可见。
  - [ ] `new_agent_row_at(shell, dialog, column, row) -> Option<usize>`。
  - [ ] `new_agent_button_at(shell, column, row) -> Option<NewAgentButton>`。
  - [ ] `render_new_agent(frame, screen, dialog)`：
    - [ ] `modal::render` 标题 `New agent`。
    - [ ] 空态显示 `No coding agents installed`。
    - [ ] 每行左侧 `agent_label(kind)`；右侧命令名仅在 `command != label` 时压暗显示。
    - [ ] 选中行整行高亮。
    - [ ] 空列表时 `Create` 压暗。
    - [ ] 返回 None。
- [ ] `src/ui/overlay.rs`：
  - [ ] `mod new_agent;`
  - [ ] 导出 `NewAgentButton`、`new_agent_shell`、`new_agent_row_at`、`new_agent_button_at`。
  - [ ] `render` 增加 `Overlay::NewAgent(dialog) => render_new_agent(...)`。
- [ ] 确认 `Overlay::input()` / `input_mut()` / `clear_selection()` 不新增分支，NewAgent 无文本输入。

### 2.6 鼠标

- [ ] `src/event/overlay.rs` `handle_overlay_click` 增加 `Some(Overlay::NewAgent(dialog))`：
  - [ ] `new_agent_shell` 失败则关闭弹窗。
  - [ ] `modal_dismissed` 处理点击外部关闭。
  - [ ] 行命中：双击立即 `apply_overlay_key(Enter)`，否则只 `set_new_agent_selection`。
  - [ ] `Create` → `apply_overlay_key(Enter)`。
  - [ ] `Cancel` → `close_overlay`。
  - [ ] 弹窗内部空白保持打开。
- [ ] `overlay_input_rect` 不新增分支（NewAgent 无输入框）。

### 2.7 启动通道

- [ ] `src/app/state/pane.rs`：
  - [ ] `PendingAgentLaunch { command: String, at: Instant }`。
  - [ ] `Pane.pending_launch: Option<PendingAgentLaunch>`。
  - [ ] `Pane::new()` 初始化为 `None`。
- [ ] `src/app/update/workspace.rs`：
  - [ ] 把 `create_terminal_in_workspace` 主体抽成 `create_terminal_tab(state, target) -> Option<PaneId>`。
  - [ ] `create_terminal_in_workspace` 保留为 `let _ = create_terminal_tab(...)`。
  - [ ] `AGENT_LAUNCH_DELAY = Duration::from_millis(150)`。
  - [ ] `create_agent_tab(state, workspace, kind, command) -> Option<PaneId>`：
    - [ ] `create_terminal_tab` 失败返回 None。
    - [ ] 根窗格写 `pending_launch`。
    - [ ] `pane.agent = None`。
    - [ ] 保持 `view = Terminal` 与聚焦。
- [ ] `src/event/session.rs`：
  - [ ] `flush_pending_agent_launches(state, sessions) -> bool`：
    - [ ] 只处理 `PaneKind::Terminal`。
    - [ ] `launch.at > now` 跳过。
    - [ ] 到点后取走 command。
    - [ ] session 存在且 `write` 成功：写入 `command + "\r"`。
    - [ ] session 缺失或写入失败：清 pending，`show_toast_at(Some(pane), TOAST_AGENT_LAUNCH_FAILED)`。
- [ ] `src/event/mod.rs`：
  - [ ] `reconcile_sessions` 后调用 `flush_pending_agent_launches`，有变化置 `dirty = true`。
- [ ] 确认写入字节使用 `\r`，与 `src/input/encode.rs` 的 Enter 编码一致。

### 2.8 测试

- [ ] `src/app/update/menu/tests.rs`：
  - [ ] `lx_agent_menu_offers_edit_and_close_without_binding` 断言首项为 `NewAgent`。
  - [ ] 新增 `agents_blank_menu_offers_only_new_agent`。
  - [ ] 新增 `new_agent_menu_command_opens_dialog`。
- [ ] `src/app/update/new_agent/tests.rs`（或同层 `tests.rs`）：
  - [ ] 空列表：Enter no-op，Esc 关闭。
  - [ ] 多列表：Up/Down 钳制、Enter 创建、选中项正确。
  - [ ] 双击窗口：同 index 400ms 内 true，超时 false。
  - [ ] 目标 workspace 失效：不打开/不创建。
  - [ ] 创建后新 tab 根窗格 `pending_launch.command` 正确、`at` 在未来。
- [ ] `src/event/tests/agent_menu.rs`：
  - [ ] 右键 Agents 空白打开单项菜单。
  - [ ] 右键第一/第二行都打开条目菜单。
  - [ ] 点击 `New agent` 打开弹窗。
  - [ ] 单击行只选中、双击行创建。
  - [ ] `Create` / `Cancel` 鼠标路径。
- [ ] `src/ui/overlay/new_agent/tests.rs`：
  - [ ] 0/1/8 行几何。
  - [ ] 短屏夹取不 panic。
  - [ ] 行命中、按钮命中。
  - [ ] 空态、选中高亮、命令名右对齐。
- [ ] `src/input/tests.rs` / `src/app/overlay/tests.rs`：
  - [ ] 补 `OverlayKind::NewAgent` 到穷尽列表测试。
- [ ] `src/event/session.rs` 测试（若已有 session 测试模块）：
  - [ ] `flush_pending_agent_launches` 时间门槛与写入字节。
  - [ ] session 缺失清队列并 toast。

## P3. 拖拽中的滚轮

### 3.1 代码

- [ ] `src/event/mouse/wheel.rs`：
  - [ ] 删除 `prompt_drag.is_some() || todo_drag.is_some()` 的全局早退。
  - [ ] 在 `let Geometry { ... }` 后增加四个拖拽分支（`state.overlay.is_none()` 前置）：
    - [ ] 工作区活跃：`scroll_workspace_list` + `workspace_drop_index` + `drag_workspace_to`。
    - [ ] 工作区历史：`scroll_workspace_history` + `workspace_history_drop_index` + `drag_workspace_history_to`。
    - [ ] Prompt 树：`scroll_prompt_tree` + `drop_hit_at` + `drag_prompt_to`。
    - [ ] Todo：`scroll_todo` + `row_at` + `drag_todo_to`。
  - [ ] 每个分支末尾 `return`，拖拽中滚轮不穿透。
  - [ ] 分支内不写 hover；hover 由 hover.rs 在拖拽期统一置空。
- [ ] 确认 `scroll_*` 系列只改视口、不改 `selected`。
- [ ] 确认工作区拖拽滚轮后立即重排（`drag_workspace_to` 是实时重排），历史列表不持久化直到 `end_workspace_history_drag`。
- [ ] 确认 Todo/Prompt 松手时使用当前 `target_index` / `drop_mark`，不读取滚轮前的旧值。

### 3.2 测试

- [ ] 改写 `src/event/tests/todo.rs:1179-1237`：
  - [ ] 保留切 Tab / 折叠 / 右键取消拖拽的清理断言。
  - [ ] 将“滚轮被禁止”改为“滚轮改变 `todo.scroll`，`todo_drag.target_index` 跟随指针新行”。
- [ ] 改写 `src/event/tests/prompt_tree.rs:645-697`：
  - [ ] 拖拽中滚轮改变 Prompt 树 `scroll`。
  - [ ] `drop_mark` 按新滚动偏移刷新。
  - [ ] 松开后落到新目标。
- [ ] 新增 `src/event/tests/workspace_history.rs` 或 `src/event/tests/mouse_click.rs`：
  - [ ] 活跃列表拖拽 + 滚轮：`workspace_scroll` 改变，拖拽块位置跟随。
  - [ ] 历史列表拖拽 + 滚轮：`workspace_history.scroll` 改变，`drag` 索引有效。
  - [ ] 滚轮在列表外：不改变列表 scroll，拖拽现场保留。
- [ ] 更新 `src/ui/left_panel/history.rs` / `left_panel/mod.rs` 相关落点测试（如需要）。

## P4. 设置页修复

### 4.1 替换

- [ ] `assets/settings_models.js`：4 处 `element(` → `domEl(`。
- [ ] `assets/settings_models_card.js`：39 处。
- [ ] `assets/settings_connect.js`：18 处。
- [ ] `assets/settings_subagents.js`：3 处。
- [ ] `grep -rn "\belement\b" assets/settings_models.js assets/settings_models_card.js assets/settings_connect.js assets/settings_subagents.js` 必须输出 0 行。
- [ ] 不新增 `const element = domEl` 别名。
- [ ] 不改 `loadModels` 的错误双写逻辑。

### 4.2 回归测试

- [ ] `tests/web_assets.rs` 新增 `assets_do_not_call_removed_element_helper`：
  - [ ] 断言 `web.js` 仍定义 `function domEl(`。
  - [ ] 遍历 `SCRIPTS`。
  - [ ] 断言每个脚本不含 `element(`。
  - [ ] 失败信息指向 `domEl()`。
- [ ] 确认 `document.createElement(` 不会误报。
- [ ] 保留现有 Node 语法检查、脚本顺序检查、`getElementById` 契约检查。

### 4.3 手动确认

- [ ] 用户运行 `cargo run`，打开 `/settings`。
- [ ] Models/Providers 标签不再出现 `Failed to load: element is not defined`。
- [ ] Quick connect 标签可正常渲染供应商列表。
- [ ] 模型卡片（Advanced/Duplicate/Remove 等按钮）正常渲染。
- [ ] Subagents 标签可正常渲染角色列表。

## P5. 验证

- [ ] `cargo fmt`
- [ ] `cargo clippy --all-targets -- -D warnings`
- [ ] `cargo test`
- [ ] 定向测试：
  - [ ] `cargo test --lib app::update::menu`
  - [ ] `cargo test --lib app::update::new_agent`
  - [ ] `cargo test --lib event::tests::agent_menu`
  - [ ] `cargo test --lib event::tests::todo`
  - [ ] `cargo test --lib event::tests::prompt_tree`
  - [ ] `cargo test --test web_assets`
- [ ] 检查无生产代码 `unwrap()` / `expect()`。
- [ ] 检查所有新增用户可见文案都在 `src/ui/text.rs`，且为英文。
- [ ] 检查 OS 专属代码只在 `src/platform/`。
- [ ] 检查新增生产文件行数（建议 500 行左右，硬上限 1000）。
- [ ] 检查测试文件按模块职责放置，不堆积在根 `tests.rs`。

## P6. 交付

- [ ] 输出修改文件清单与关键 diff 说明。
- [ ] 输出验证命令与结果。
- [ ] 列出未自动验证的 TUI/浏览器交互点，交由用户 `cargo run` 确认。
- [ ] 每个任务完成后单独提交（P1、P2、P3、P4 各一次）。
- [ ] 全部完成后询问用户是否把 `feature/agents-menu-drag-scroll` 合并回 `dev`；用户确认前不合并。

## 执行记录

- 分支：`feature/agents-menu-drag-scroll`（worktree `.worktrees/agents-menu-drag-scroll`）。
- 提交：
  - `87c9fbcb` P1 终端 Agent 菜单 Close 与关闭语义
  - `e5f8823b` P2 New agent 菜单、PATH 探测、弹窗与启动队列
  - `6fabb6a9` P3 拖拽中的滚轮（工作区活跃/历史、Todo、Prompt 树）
  - `e8742e7c` P4 设置页 `element(` → `domEl(` 与资源回归测试
- 验证：`cargo fmt`、`cargo clippy --all-targets -- -D warnings`、`cargo test` 全部通过（5811 lib + 各集成测试）。
- 未自动验证：TUI 菜单/弹窗/拖拽交互外观，浏览器 `/settings` 三个标签的真实渲染，需用户 `cargo run` 确认。
- 合并回 `dev` 前需用户确认。
