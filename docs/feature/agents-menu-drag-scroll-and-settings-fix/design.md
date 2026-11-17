# Agents 菜单、拖拽滚轮与设置页加载修复 — 设计

> 状态：**已实现，待用户确认合并**
> 关联任务：[task.md](task.md)
> 基线：`.worktrees/agents-menu-drag-scroll`（分支 `feature/agents-menu-drag-scroll`，切自 `dev`）
> 范围：侧栏 `Agents` 右键菜单、新建 Agent 模态、可拖拽列表滚轮、浏览器设置页 JS 修复

## 1. 背景与目标

本次包含四个相互独立但同属一个交互面的改动：

1. **终端 Agent 也能 Close**：当前只有 lx 条目稳定提供 `Close`；opencode/claude/codex 等终端 Agent 在“标签内唯一窗格”时没有 `Close`，因为底层禁止删除最后一个窗格。
2. **新增 `New agent` 入口与模态**：右侧 Agents 条目菜单与 Agents 空白区域都能右键新建 Agent；弹窗只列本机 PATH 中已安装的 CLI coding agent，选中后新建标签并启动。
3. **拖拽列表允许滚轮**：左侧工作区活跃/历史列表、右侧 Todo、右侧 Prompt 树在拖拽过程中允许用滚轮滚动视口，并按指针当前位置刷新落点。
4. **修复设置页 `element is not defined`**：模型/供应商、Quick connect、模型卡片与 Subagents 页面共用的 `element()` 全局函数已在重构中删除，调用点漏改。

目标不是重写侧栏或终端架构，而是在现有 `app/` 纯状态、`ui/` 纯渲染、`event/` 只分派的结构内做最小闭环。

## 2. 已确认决策

| 编号 | 问题 | 最终结论 |
|---|---|---|
| Q1 | 终端 Agent 的 `Close` 语义 | 关窗格 → 唯一窗格则关标签 → 唯一标签则先新建空白标签再关旧标签；工作区始终保留至少一个 tab |
| Q2 | 新建弹窗候选列表 | 只列 PATH 中真实安装的 8 类 CLI Agent；同一 Agent 只出现一次；不列 lx；空列表显示 `No coding agents installed` |
| Q3 | `New agent` 菜单位置 | Agent 条目菜单第一项；Agents 列表空白区域右键只弹单项 `New agent`；滚动条列与折叠按钮不算空白 |
| Q4 | 新建弹窗交互 | 单选列表；↑/↓ 或单击只移动高亮；`Enter` / `Create` 创建；`Esc` / `Cancel` 关闭；双击行等价于创建；默认高亮第一项 |
| Q5 | 拖拽滚轮覆盖范围 | 工作区活跃列表、工作区历史列表、Todo 列表、Prompt 树；只改视口、不改选中项；滚轮后按指针当前位置刷新落点 |
| Q6 | Agent 启动方式 | 新标签先起普通 shell，PTY 就绪后写入 `<可执行文件名>\r`；agent 退出后回到 shell，窗格不自动关闭 |
| Q7 | 设置页修复范围 | 四个 JS 文件全部把 `element(` 改为 `domEl(`；补资源回归测试；错误提示双写行为不变 |
| Q8 | 实现位置 | 在 `.worktrees/agents-menu-drag-scroll` 的 `feature/agents-menu-drag-scroll` 分支实现；每个任务（P1-P4）完成后提交一次；全部完成后由用户决定是否合并回 `dev` |

## 3. 现状核对

### 3.1 Agents 菜单

- `src/app/update/menu.rs:233-274` `open_agent_menu`：命令顺序为 `EditAgentTitle` → 可选 `OpenPrompt` → 可选 `RemoveAgentTitle` → 可选 `CloseAgent`。
- `src/app/update/menu.rs:250-257`：

```rust
let closable = match entry.kind {
    SidebarAgentKind::Lx { .. } => true,
    SidebarAgentKind::Terminal(_) => state
        .workspaces
        .get(workspace)
        .and_then(|workspace| workspace.tabs.get(entry.tab_index))
        .is_some_and(|tab| tab.layout.pane_ids().len() > 1),
};
if closable {
    commands.push(MenuCommand::CloseAgent);
}
```

这是终端 Agent 没有 `Close` 的直接原因。

- `src/app/update/menu.rs:278-303` `close_agent`：lx 走 `chat::new_session`（等价 `/new`），终端走 `close_pane`。
- `src/app/state/pane.rs:173-180` `Tab::remove_pane` 与 `src/tiling.rs:188-205` `TileLayout::remove_pane` 都拒绝删除最后一个窗格。
- `src/app/update/workspace.rs:456-476` `close_tab` 拒绝关闭最后一个标签；`:424-455` `close_workspace` 拒绝关闭最后一个工作区。
- `src/app/overlay.rs:449` `MenuCommand::CloseAgent` 在 `requires_confirm` 中，关闭需要菜单行内二次确认。

### 3.2 右侧菜单与空白区域

- `src/event/overlay.rs:592-724` `open_right_click_menu`：先命中标签栏、工作区历史、工作区列表、Agents 条目、Prompt 树、Todo、页列表、编辑视图与窗格。
- `src/event/overlay.rs:621-625` 只在命中 `agent_item_index_at` 时打开 `open_agent_menu`；Agents 空白区域目前没有分支，右键会继续向下穿透。
- `src/ui/left_panel/mod.rs:614-648`：`agents_section_at` 命中整个分区；`agent_item_index_at` 排除滚动条列与底部半个条目。
- `src/app/update/menu.rs:77-98` `open_workspace_list_menu` 是空白区域单项菜单的现成范式。

### 3.3 新建 Agent 的现有能力

- `src/detect/mod.rs:5-14` `AgentKind` 固定 8 种：`Claude / Codex / Gemini / Antigravity / OpenCode / Cursor / Pi / Kimi`。
- `src/detect/mod.rs:42-54` `identify_agent` 已有进程别名表；`src/ui/text/workspace.rs:40-51` `agent_label` 已有稳定小写标签。
- `src/app/update/workspace.rs:179-214` `create_terminal_in_workspace`：新建标签、根窗格 `view = Terminal`、激活目标工作区、聚焦窗格。
- `src/app/update/workspace.rs:360-386` `create_tab`：新建标签默认 lx 视图，并继承模型与 `@` 候选快照。
- `src/pty/mod.rs:30-79` `PtySession::spawn` 只启动 `$SHELL`，不接收命令参数。
- `src/event/session.rs:144-185` `reconcile_sessions` 每个事件循环按状态懒启动 PTY；新建标签不会立刻有 PTY。
- `src/event/session.rs:203-209` `write_to_pane` 向已存在的 PTY 写入字节。
- 结论：需要“待写入命令”队列，等 PTY 就绪后再写；不能在创建标签的动作里直接 `write_to_pane`。

### 3.4 拖拽与滚轮

- `src/event/mouse/wheel.rs:246-250`：

```rust
// 拖动中禁止改视口：落点与阈值都按绝对行号换算，滚动会让指针原地"跨行"
// （design §1.3 明确不做边缘自动滚动）。
if state.prompt_drag.is_some() || state.todo_drag.is_some() {
    return;
}
```

这是 Todo 与 Prompt 树拖拽中滚轮完全失效的唯一全局闸门。

- 工作区活跃列表：`src/app/state.rs:334-339` 的 `workspace_drag / workspace_dragging / workspace_drag_moved`；`src/app/update/workspace_list.rs:97-177`；落点 `src/ui/left_panel/mod.rs:452-471`。
- 工作区历史列表：`src/app/state/workspace_history.rs:90-94`；`src/app/update/workspace_history/mod.rs:136-239`；落点 `src/ui/left_panel/history.rs:380-396`。
- 两个工作区列表的滚轮分支在 `src/event/mouse/wheel.rs:629-647`，当前没有判断拖拽状态；滚动后只重算 hover，不立即重算落点。
- Todo：`src/app/state.rs:158-171` `TodoDrag`、`:403-404` `todo_drag`；`src/app/update/todo/drag.rs:12-100`；滚轮分支 `src/event/mouse/wheel.rs:394-403`。
- Prompt 树：`src/app/update/prompt/drag.rs`；滚轮分支 `src/event/mouse/wheel.rs:425-445`；落点 `src/ui/right_panel/prompt/tree.rs:298-313`。
- 现有反向测试：`src/event/tests/todo.rs:1179-1237` 断言“拖拽待办行期间禁止滚轮改写视口”；`src/event/tests/prompt_tree.rs:645-697` 断言 Prompt 拖拽期间滚轮不改视口。本次需求会推翻这两条断言，需要同步改写。

### 3.5 设置页 `element is not defined`

- `assets/web.js:5-11` 定义唯一 DOM helper：

```javascript
function domEl(tag, className, text) {
  const node = document.createElement(tag);
  if (className) { node.className = className; }
  if (text !== undefined && text !== null) { node.textContent = String(text); }
  return node;
}
```

- `assets/web.html:968-980` 的脚本顺序为 `web.js` → `settings.js` → `settings_models_card.js` → `settings_models.js` → `settings_connect.js` → …；`domEl` 在第一个脚本就绪，问题不是加载顺序。
- 全仓已无 `function element` 定义。残留调用 64 处：
  - `assets/settings_models.js:208,212,215,216`（4 处）
  - `assets/settings_models_card.js:138…492`（39 处）
  - `assets/settings_connect.js:128…334`（18 处）
  - `assets/settings_subagents.js:368,372,373`（3 处）
- `assets/settings_models.js:125-151` `loadModels` 的 `.catch` 会把同一个 `ReferenceError` 同时写入 `#models-status` 与 `#model-settings-status`，所以界面上出现两次 `Failed to load: element is not defined`。
- 回归来源：`ec5246a1` 删除 `settings.js` 的 `element` 并把 `settings.js` 内部调用改为 `domEl`，另外四个文件漏改；`tests/web_assets.rs` 只校验 `getElementById` 的静态 id，不校验全局 helper。

## 4. 设计

### 4.1 终端 Agent 的 Close

#### 4.1.1 菜单

`open_agent_menu` 不再按窗格数裁剪终端 Agent：

```rust
let mut commands = vec![MenuCommand::NewAgent, MenuCommand::EditAgentTitle];
if agent_title_prompt(...).is_some() {
    commands.push(MenuCommand::OpenPrompt);
}
if state.agent_title_override(entry.pane_id, entry.kind).is_some() {
    commands.push(MenuCommand::RemoveAgentTitle);
}
// 所有条目都有 Close：lx 重置会话视图，终端 Agent 关闭所属窗格/标签。
commands.push(MenuCommand::CloseAgent);
```

`CloseAgent` 继续保留 `requires_confirm()` 的行内二次确认，不改成单击即关。

#### 4.1.2 执行语义

```rust
fn close_terminal_agent(state: &mut AppState, workspace: usize, tab: usize, pane: PaneId) {
    let Some(pane_count) = state
        .workspaces
        .get(workspace)
        .and_then(|workspace| workspace.tabs.get(tab))
        .map(|tab_state| tab_state.layout.pane_ids().len())
    else {
        return;
    };
    let Some(tab_count) = state.workspaces.get(workspace).map(|workspace| workspace.tabs.len()) else {
        return;
    };
    if pane_count > 1 {
        close_pane(state, workspace, tab, pane);
    } else if tab_count > 1 {
        close_tab(state, workspace, tab);
    } else {
        // 唯一标签：先建空白标签（默认 lx 欢迎页），再删旧标签。
        // create_tab 会把新标签追加到末尾并激活；旧标签索引在唯一标签场景下必为 0。
        create_tab(state);
        close_tab(state, workspace, tab);
    }
}
```

关键点：

- `close_agent` 已先校验 `state.active_workspace == workspace`，因此最后一种路径里的 `create_tab` 目标就是该工作区。
- `create_tab` 会从旧标签根窗格复制 `@` 候选快照与工作区最近模型；新标签是干净 lx 页。
- `close_tab` 删除旧标签后，新标签索引回到 0，`active_tab` 自动回退，不会留下空工作区。
- 旧窗格的 PTY 会话在下一轮 `reconcile_sessions` 中因 pane 不存在被 `kill`。
- 目标存在性校验沿用 `close_agent` 开头的 `agent_entries` 复查，避免菜单打开期间目标消失后误关。

`close_agent` 的分支保持：

```rust
match kind {
    SidebarAgentKind::Lx { .. } => super::chat::new_session(state, pane),
    SidebarAgentKind::Terminal(_) => close_terminal_agent(state, workspace, tab, pane),
}
```

### 4.2 New agent 菜单

#### 4.2.1 菜单模型

新增：

```rust
pub enum MenuCommand {
    // ...
    /// 侧栏 Agents：选择已安装的 coding agent 并新建标签启动。
    NewAgent,
}

pub enum OverlayTarget {
    // ...
    /// 侧栏 Agents 列表空白区域。
    AgentsBlank { workspace: usize },
}
```

- `MenuCommand::NewAgent` **不**进入 `requires_confirm()`；它只打开弹窗，不直接执行副作用。
- `OverlayTarget::AgentsBlank { workspace }` 固化空白区域所属工作区，菜单存续期间切工作区也不会串目标。

#### 4.2.2 打开菜单

新增 `open_agents_blank_menu`：

```rust
pub fn open_agents_blank_menu(state: &mut AppState, workspace: usize, anchor: (u16, u16)) {
    state.agent_scroll_drag = None;
    state.agent_hover = None;
    state.overlay = Some(Overlay::Menu(Menu {
        anchor,
        target: OverlayTarget::AgentsBlank { workspace },
        commands: vec![MenuCommand::NewAgent],
        selected: 0,
        confirming: false,
    }));
}
```

条目菜单在 `commands` 头部插入 `MenuCommand::NewAgent`。

#### 4.2.3 空白命中

新增 `ui::agents_blank_at`：

```rust
pub fn agents_blank_at(
    view: &layout::ViewLayout,
    state: &AppState,
    column: u16,
    row: u16,
) -> bool {
    let Some(list) = agents_list_rect(view, state) else {
        return false;
    };
    if !list.contains((column, row).into()) {
        return false;
    }
    if agent_scrollbar(view, state)
        .is_some_and(|bar| bar.area.contains((column, row).into()))
    {
        return false;
    }
    agent_item_index_at(view, state, column, row).is_none()
}
```

事件层在 `open_right_click_menu` 中：

```rust
if ui::agents_section_at(view, state, column, row) {
    // 每次右键都刷新安装快照，避免长时间运行后 PATH 变化不生效。
    state.installed_agents = crate::detect::installed_agents();
}
if let Some(index) = ui::agent_item_index_at(view, state, column, row) {
    update::open_agent_menu(state, index, (column, row));
    return true;
}
if ui::agents_blank_at(view, state, column, row) {
    update::open_agents_blank_menu(state, state.active_workspace, (column, row));
    return true;
}
```

顺序必须保持在 Prompt/Todo/窗格分支之前，避免 Agents 空白点击穿透到底层主区。

### 4.3 已安装 Agent 探测

#### 4.3.1 类型与固定顺序

`src/detect/mod.rs`：

```rust
pub const ALL_AGENT_KINDS: [AgentKind; 8] = [
    AgentKind::Claude,
    AgentKind::Codex,
    AgentKind::Gemini,
    AgentKind::Antigravity,
    AgentKind::OpenCode,
    AgentKind::Cursor,
    AgentKind::Pi,
    AgentKind::Kimi,
];

/// 启动命令候选名：安装探测、弹窗显示与启动命令共用同一来源。
pub const fn launch_names(kind: AgentKind) -> &'static [&'static str] {
    match kind {
        AgentKind::Claude => &["claude", "claude-code"],
        AgentKind::Codex => &["codex"],
        AgentKind::Gemini => &["gemini", "gemini-cli"],
        AgentKind::Antigravity => &["agy", "antigravity", "antigravity-cli"],
        AgentKind::OpenCode => &["opencode", "opencode2", "open-code"],
        AgentKind::Cursor => &["cursor-agent", "cursor"],
        AgentKind::Pi => &["pi", "pi-coding-agent"],
        AgentKind::Kimi => &["kimi", "kimi-code"],
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstalledAgent {
    pub kind: AgentKind,
    /// PATH 中实际命中的候选名（如 `opencode2`），直接作为 shell 命令写入。
    pub command: String,
}

pub fn installed_agents() -> Vec<InstalledAgent> {
    ALL_AGENT_KINDS
        .iter()
        .filter_map(|kind| {
            let (command, _path) = crate::platform::find_executable(launch_names(*kind))?;
            Some(InstalledAgent { kind: *kind, command })
        })
        .collect()
}
```

同一 `AgentKind` 在 `ALL_AGENT_KINDS` 里只出现一次，因此 `opencode` 与 `opencode2` 同时存在时只生成一个 `OpenCode` 条目，启动命中的第一个候选名。

#### 4.3.2 平台查找

`src/platform/executable.rs`（新文件，OS 分支只在这里）：

```rust
/// 在 PATH 中查找第一个可执行的候选名；返回命中候选名与绝对路径。
pub fn find_executable(names: &[&str]) -> Option<(String, PathBuf)> {
    let path = std::env::var_os("PATH")?;
    find_executable_in(names, &path)
}

fn find_executable_in(names: &[&str], path: &OsStr) -> Option<(String, PathBuf)> {
    for dir in std::env::split_paths(path) {
        for name in names {
            #[cfg(unix)]
            {
                let candidate = dir.join(name);
                if is_executable_file(&candidate) {
                    return Some(((*name).to_string(), candidate));
                }
            }
            #[cfg(windows)]
            {
                // 依次尝试 `.exe` / `.cmd` / `.bat` / `.com` 与无扩展名。
                for candidate in windows_candidates(&dir, name) {
                    if candidate.is_file() {
                        return Some(((*name).to_string(), candidate));
                    }
                }
            }
        }
    }
    None
}

#[cfg(unix)]
fn is_executable_file(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    path.metadata()
        .is_ok_and(|meta| meta.is_file() && meta.permissions().mode() & 0o111 != 0)
}
```

不把绝对路径写进启动命令：新标签的 shell 继承同一 `PATH`，写候选名即可；这样用户移动安装目录后行为不变。

#### 4.3.3 缓存位置

`AppState` 新增：

```rust
/// 最近一次右键 Agents 分区时扫描到的已安装 CLI Agent 快照。
pub installed_agents: Vec<crate::detect::InstalledAgent>,
```

- 右键 Agents 时由事件层刷新，`app/` 不做 IO。
- `AppState::demo()` / `new()` 默认空列表；弹窗测试直接注入固定列表，不依赖开发机 PATH。
- 右键之外的入口不重新扫描；弹窗生命周期很短，可接受。

### 4.4 New agent 模态

#### 4.4.1 状态

`src/app/overlay.rs`：

```rust
#[derive(Debug)]
pub struct NewAgentDialog {
    pub workspace: usize,
    pub agents: Vec<crate::detect::InstalledAgent>,
    pub selected: usize,
}

impl NewAgentDialog {
    pub fn new(workspace: usize, agents: Vec<InstalledAgent>) -> Self {
        Self { workspace, agents, selected: 0 }
    }

    pub fn selected_agent(&self) -> Option<&InstalledAgent> {
        self.agents.get(self.selected)
    }
}
```

`OverlayKind::NewAgent`、`Overlay::NewAgent(NewAgentDialog)`、`Overlay::kind()` 同步增加。该弹窗没有文本输入，`input()/input_mut()/clear_selection()` 不新增分支（现有通配臂返回 None/空操作）。

#### 4.4.2 打开

`src/app/update/new_agent.rs`（新文件）：

```rust
pub fn open_new_agent_dialog(state: &mut AppState, workspace: usize) {
    if state.workspaces.get(workspace).is_none() {
        return;
    }
    state.overlay = Some(Overlay::NewAgent(NewAgentDialog::new(
        workspace,
        state.installed_agents.clone(),
    )));
}
```

`activate_menu` 增加：

```rust
(
    Some(MenuCommand::NewAgent),
    OverlayTarget::Agent { workspace, .. } | OverlayTarget::AgentsBlank { workspace },
) => {
    open_new_agent_dialog(state, *workspace);
}
```

#### 4.4.3 键盘

`src/input/mod.rs` `overlay_key`：

```rust
OverlayKind::NewAgent => match key.code {
    KeyCode::Esc => Some(OverlayKey::Esc),
    KeyCode::Enter => Some(OverlayKey::Enter),
    KeyCode::Up => Some(OverlayKey::Up),
    KeyCode::Down => Some(OverlayKey::Down),
    _ => None,
},
```

`src/app/update/dialog.rs` 在通用分派之前拦截：

```rust
if matches!(state.overlay, Some(Overlay::NewAgent(_))) {
    apply_new_agent_key(state, key);
    return None;
}
```

`apply_new_agent_key`：

```rust
match key {
    OverlayKey::Esc => close_overlay(state),
    OverlayKey::Up => { move_new_agent_selection(state, -1); }
    OverlayKey::Down => { move_new_agent_selection(state, 1); }
    OverlayKey::Enter => create_selected_agent(state),
    _ => {}
}
```

#### 4.4.4 鼠标

`src/event/overlay.rs` 增加 `Some(Overlay::NewAgent(dialog))` 分支：

- 点击 `shell.area` 外：关闭。
- 点击行：`set_new_agent_selection`；用 `new_agent_click` 记录 400ms 内同一下标双击，双击立即 `apply_overlay_key(Enter)`。
- 点击 `Create`：`apply_overlay_key(Enter)`。
- 点击 `Cancel`：`close_overlay`。
- 弹窗内部空白：保持打开。

`AppState` 增加：

```rust
/// New agent 弹窗行双击判定。
pub new_agent_click: Option<(usize, Instant)>,
```

默认 `None`；`open_new_agent_dialog` 重置。

#### 4.4.5 渲染与几何

`src/ui/overlay/new_agent.rs`（新文件）：

- 宽度固定 48；高度 `行数 + 5`，行数夹在 1..=8；短屏由 `widgets::modal::layout` 自动夹取。
- 标题 `New agent`；内容行首列显示 `text::agent_label(kind)`，右侧可选显示实际命令名（与标签不同才显示，例如 `cursor` / `cursor-agent`）。
- 选中行整行高亮；无 agent 时显示 `No coding agents installed`，`Create` 按钮压暗且不响应。
- 底部按钮 `Create` / `Cancel`；按钮与行命中函数与 `AgentTitle` 一样走 `widgets::modal`。
- 返回 `None`（无硬件光标）。

`src/ui/overlay.rs` 增加模块声明、导出与 `render` 分支。

#### 4.4.6 创建与启动

`src/app/update/workspace.rs`：

```rust
pub(crate) const AGENT_LAUNCH_DELAY: Duration = Duration::from_millis(150);

pub fn create_agent_tab(
    state: &mut AppState,
    workspace: usize,
    kind: AgentKind,
    command: &str,
) -> Option<PaneId> {
    let pane = create_terminal_tab(state, workspace)?;
    let pane_state = state.pane_mut_anywhere(pane)?;
    pane_state.pending_launch = Some(PendingAgentLaunch {
        command: command.to_string(),
        at: Instant::now() + AGENT_LAUNCH_DELAY,
    });
    pane_state.agent = None;
    Some(pane)
}
```

- 把 `create_terminal_in_workspace` 的主体抽成 `create_terminal_tab(...) -> Option<PaneId>`，原函数保留为薄包装，行为不变。
- `PendingAgentLaunch` 放在 `src/app/state/pane.rs`：

```rust
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingAgentLaunch {
    pub command: String,
    pub at: Instant,
}
```

- `Pane` 增加 `pub pending_launch: Option<PendingAgentLaunch>`，`Pane::new()` 初始化为 `None`。

`create_selected_agent`：

```rust
let Some(Overlay::NewAgent(dialog)) = state.overlay.as_ref() else {
    return;
};
let Some(agent) = dialog.selected_agent().cloned() else {
    return;
};
let workspace = dialog.workspace;
state.overlay = None;
if create_agent_tab(state, workspace, agent.kind, &agent.command).is_none() {
    // 目标工作区已消失：不留下半成品。
    return;
}
```

#### 4.4.7 PTY 写入

`src/event/session.rs` 新增：

```rust
pub(crate) fn flush_pending_agent_launches(
    state: &mut AppState,
    sessions: &mut HashMap<PaneId, PtySession>,
) -> bool {
    let now = Instant::now();
    let mut launches = Vec::new();
    for id in state.all_pane_ids() {
        let Some(pane) = state.pane_mut_anywhere(id) else { continue };
        if pane.kind != PaneKind::Terminal {
            continue;
        }
        let Some(launch) = pane.pending_launch.as_ref() else { continue };
        if launch.at > now {
            continue;
        }
        let command = launch.command.clone();
        pane.pending_launch = None;
        launches.push((id, command));
    }
    let mut changed = false;
    for (id, command) in launches {
        let bytes = format!("{command}\r").into_bytes();
        match sessions.get_mut(&id) {
            Some(session) if session.write(&bytes).is_ok() => {
                changed = true;
            }
            _ => {
                // PTY 启动/写入失败：清掉队列并 toast，保留空白终端。
                update::show_toast_at(
                    state,
                    Some(id),
                    Toast::new(ToastKind::Danger, text::TOAST_AGENT_LAUNCH_FAILED, Some(id), Instant::now()),
                );
                changed = true;
            }
        }
    }
    changed
}
```

事件循环在 `reconcile_sessions` 之后调用：

```rust
session::reconcile_sessions(state, &mut sessions, &sender);
if session::flush_pending_agent_launches(state, &mut sessions) {
    dirty = true;
}
```

`\r` 与 `input/encode.rs` 的 Enter 编码一致。150ms 延迟让 shell 完成启动与首屏提示，避免命令插在 shell 初始化中间。

失败语义：

- PATH 探测到但启动前被删除：shell 会打印 `command not found`，弹窗已关闭，标签保留。
- PTY spawn 失败：`reconcile_sessions` 已记录错误；`flush` 检测不到 session，清队列并 toast。
- PTY 写入失败：toast，标签保留，用户仍可在终端里手动输入。

### 4.5 拖拽中的滚轮

#### 4.5.1 去掉全局闸门

删除 `wheel.rs:246-250` 的 `prompt_drag || todo_drag` 早退，改成四个“拖拽现场优先”的分支，位置在 `let Geometry { ... }` 之后、所有 overlay/普通分支之前，并且只在 `state.overlay.is_none()` 时生效：

```rust
if state.overlay.is_none() && state.workspace_drag.is_some() {
    if ui::workspace_section_at(view, state, mouse.column, mouse.row)
        && let Some(delta) = vertical_wheel_delta(mouse.kind)
        && let Some(rows) = ui::workspace_list_rows(view, state)
        && update::scroll_workspace_list(state, delta, rows)
    {
        if let Some(target) = ui::workspace_drop_index(view, state, mouse.row) {
            update::drag_workspace_to(state, target);
        }
        *dirty = true;
    }
    return;
}

if state.overlay.is_none() && state.workspace_history.drag.is_some() {
    if ui::workspace_section_at(view, state, mouse.column, mouse.row)
        && let Some(delta) = vertical_wheel_delta(mouse.kind)
        && let Some(rows) = ui::workspace_history_visible_rows(view, state)
        && update::scroll_workspace_history(state, delta, rows)
    {
        if let Some(target) = ui::workspace_history_drop_index(view, state, mouse.row) {
            update::drag_workspace_history_to(state, target);
        }
        *dirty = true;
    }
    return;
}

if state.overlay.is_none() && state.prompt_drag.is_some() {
    let area = ui::right_panel::prompt::frame::prompt_main_rect(view.right_panel, state);
    if area.contains((mouse.column, mouse.row).into())
        && let Some(direction) = vertical_wheel_direction(mouse.kind)
    {
        let visible = update::prompt_tree_visible_rows(state);
        if update::scroll_prompt_tree(state, direction.saturating_mul(WHEEL_LINES), visible) {
            let panel = &state.prompt_owner().prompt_panel;
            let hit = ui::right_panel::prompt::tree::drop_hit_at(
                area, panel, mouse.column, mouse.row,
            );
            update::drag_prompt_to(state, hit);
            *dirty = true;
        }
    }
    return;
}

if state.overlay.is_none() && state.todo_drag.is_some() {
    let list = ui::right_panel::todo::effective_list_rect(view.right_panel, state);
    if state.right_tab == RightTab::Todo
        && !state.prompt_collapsed
        && list.contains((mouse.column, mouse.row).into())
    {
        if let Some(direction) = vertical_wheel_direction(mouse.kind)
            && update::scroll_todo(state, direction)
        {
            let hit = ui::right_panel::todo::row_at(
                view.right_panel, state, mouse.column, mouse.row,
            );
            update::drag_todo_to(state, hit);
            *dirty = true;
        }
    }
    return;
}
```

要点：

- **只改视口**：`scroll_*` 系列本身不移动选中项；不新增“滚轮改选中”。
- **落点立即刷新**：工作区列表的拖拽是“实时重排”，滚轮后必须立刻按新 `scroll` + 指针行重新算 `drop_index`；Todo/Prompt 只刷新预览目标，松手时才提交。
- **拖拽期间滚轮不穿透**：分支末尾 `return`，避免拖动中滚轮去滚终端或浮层。
- **overlay 优先**：`state.overlay.is_none()` 保证弹窗滚轮不被拖拽现场拦截；正常路径中按下已会清 `todo_drag`，这是双保险。
- **拖拽中悬停**：工作区列表滚轮分支不再写 hover；拖拽期间 hover 由 `hover.rs` 统一置空。Todo/Prompt 同理。

#### 4.5.2 受影响的现有测试

- `src/event/tests/todo.rs:1179-1237`：原断言“拖拽期间禁止滚轮”改为“拖拽期间滚轮改变 `todo.scroll`，且 `target_index` 跟随指针新行”。
- `src/event/tests/prompt_tree.rs:645-697`：改为“滚轮改变 Prompt 树 `scroll`，`drop_mark` 跟随新指针行”。
- 新增工作区活跃/历史拖拽滚轮测试：拖住条目，滚一屏，断言 `workspace_scroll`/`workspace_history.scroll` 改变且拖拽顺序或落点跟随。
- 保留“切 Tab / 折叠右栏 / 右键取消 Todo 拖拽”的清理断言，避免滚轮分支破坏既有收尾路径。

### 4.6 设置页修复

#### 4.6.1 代码改动

逐文件把 `element(` 替换为 `domEl(`，只改调用名，不改参数与逻辑：

| 文件 | 处数 |
|---|---|
| `assets/settings_models.js` | 4 |
| `assets/settings_models_card.js` | 39 |
| `assets/settings_connect.js` | 18 |
| `assets/settings_subagents.js` | 3 |

不新增 `const element = domEl` 兼容别名，避免把已删除的旧全局名重新固化。`domEl` 签名与旧 `element` 完全兼容（唯一差异是显式 `String(text)`）。

#### 4.6.2 回归测试

`tests/web_assets.rs` 新增：

```rust
#[test]
fn assets_do_not_call_removed_element_helper() {
    assert!(
        read("web.js").contains("function domEl("),
        "web.js must keep the shared domEl helper"
    );
    for name in SCRIPTS {
        let js = read(name);
        assert!(
            !js.contains("element("),
            "{name} calls removed global element(); use domEl()"
        );
    }
}
```

`document.createElement(` 的大小写是 `Element(`，不会误报；`domEl(` 不含 `element(`。现有 Node 语法检查、脚本顺序检查与 `getElementById` 契约检查全部保留。

`assets/settings_models.js` 的错误双写行为不动：真实加载失败时仍同时更新两个状态位，本次只修 `ReferenceError`。

## 5. 文件变更清单

### 新增

- `src/app/update/new_agent.rs`
- `src/ui/overlay/new_agent.rs`
- `src/platform/executable.rs`
- （可能）`src/platform/executable/tests.rs` 或在 `src/platform/tests.rs` 内补测试
- `docs/feature/agents-menu-drag-scroll-and-settings-fix/design.md`、`task.md`

### 修改

- `src/app/state.rs`：`installed_agents`、`new_agent_click`
- `src/app/state/pane.rs`：`PendingAgentLaunch`、`Pane.pending_launch`
- `src/app/overlay.rs`：`OverlayKind::NewAgent`、`Overlay::NewAgent`、`MenuCommand::NewAgent`、`OverlayTarget::AgentsBlank`、`NewAgentDialog`
- `src/app/update/menu.rs`：菜单命令顺序、空白菜单、`NewAgent` 分派
- `src/app/update/workspace.rs`：`create_terminal_tab`、`create_agent_tab`
- `src/app/update/dialog.rs`：NewAgent 按键前置分支与穷尽列表
- `src/app/update.rs`：导出 `new_agent`、`create_agent_tab`
- `src/detect/mod.rs`：`ALL_AGENT_KINDS`、`launch_names`、`InstalledAgent`、`installed_agents`
- `src/platform/mod.rs`：导出 `executable::find_executable`
- `src/event/mod.rs`：事件循环调用 `flush_pending_agent_launches`
- `src/event/session.rs`：`flush_pending_agent_launches`
- `src/event/overlay.rs`：Agents 空白分支、NewAgent 点击、安装快照刷新
- `src/event/mouse/wheel.rs`：四个拖拽滚轮分支
- `src/input/mod.rs`：`OverlayKind::NewAgent` 按键映射
- `src/ui/left_panel/mod.rs`：`agents_blank_at`
- `src/ui/overlay.rs`：NewAgent 渲染分派与导出
- `src/ui/text.rs`：菜单、弹窗、toast 文案
- `assets/settings_models.js`
- `assets/settings_models_card.js`
- `assets/settings_connect.js`
- `assets/settings_subagents.js`
- `tests/web_assets.rs`

## 6. 异常与边界

| 场景 | 行为 |
|---|---|
| PATH 中没有任何支持的 Agent | 弹窗显示 `No coding agents installed`，`Create` 不可用，`Cancel` 可关 |
| `opencode` 与 `opencode2` 同时安装 | 只显示一个 `opencode` 条目，启动命中的第一个候选名 |
| 菜单打开后目标工作区消失 | `open_new_agent_dialog` 校验 workspace，失败不打开 |
| 弹窗打开后工作区消失 | `create_agent_tab` 返回 None，弹窗关闭但不创建标签 |
| PTY spawn/写入失败 | 清 pending，保留空白终端，显示 danger toast |
| 终端 Agent 是唯一窗格 | 关标签；唯一标签则替换为空白 lx 标签 |
| 终端 Agent 是唯一窗格且 lx 同窗格有会话 | 关闭窗格/标签会销毁 lx 视图，DB 会话仍可通过 `/resume` 恢复（沿用现有 D3 语义） |
| 拖拽滚轮到达列表边界 | `scroll_*` 钳制，落点不越界；工作区落点空白吸附末条 |
| 拖拽中指针移出列表后滚轮 | 该次滚轮被拖拽分支吞掉，不滚动其他面板；拖拽现场保留 |
| 弹窗打开时滚轮 | overlay 分支优先，拖拽分支不拦截 |
| 设置页真实 API 加载失败 | 仍显示 `Failed to load: ...`，但不再是 `element is not defined` |

## 7. 测试计划

### 7.1 状态/更新层

- `open_agent_menu` 对 lx 与终端 Agent 都包含 `NewAgent` 与 `CloseAgent`。
- 终端 Agent 唯一窗格菜单仍包含 `CloseAgent`。
- `close_terminal_agent` 三条路径：多窗格、唯一窗格多标签、唯一标签。
- `open_agents_blank_menu` 命令只有 `NewAgent`，目标为 `AgentsBlank { workspace }`。
- `open_new_agent_dialog` 空列表、多列表、目标失效。
- `create_selected_agent` 设置 `pending_launch`、`view = Terminal`、激活新标签。
- `flush_pending_agent_launches` 在 `at` 之前不写、之后写入 `command\r`；session 缺失时清队列并 toast。

### 7.2 UI 层

- `agents_blank_at`：条目行、滚动条列、折叠按钮、空列表、条目下方空白。
- NewAgent 弹窗几何：0/1/8 行、短屏夹取、行命中、按钮命中、选中高亮、空态。
- 菜单标题与标签：`Agents`、`New agent`。

### 7.3 事件层

- 右键条目第一/第二行都打开条目菜单，首项 `New agent`。
- 右键 Agents 空白打开单项菜单；右键滚动条列不打开。
- 点击 `New agent` 打开弹窗；↑/↓/Enter/Esc；单击选中、双击创建；Create/Cancel 按钮。
- 拖拽滚轮：工作区活跃、历史、Todo、Prompt 树各一条；断言视口变化与落点刷新。
- 设置页资源测试：四文件不再含 `element(`。

### 7.4 验证命令

```bash
cargo fmt
cargo clippy --all-targets -- -D warnings
cargo test
```

TUI 交互与外观由用户运行 `cargo run` 确认；浏览器设置页由用户打开 `/settings` 确认模型、Quick connect、Subagents 三个标签不再报 `element is not defined`。

## 8. 不做的事

- 不引入新依赖（不引入 `which`/`tempfile`）。
- 不改 `PtySession::spawn` 的“只起 shell”语义。
- 不把 lx 放进 New agent 列表。
- 不做拖拽边缘自动滚动（本次只做滚轮）。
- 不做 Agent 安装/卸载管理，只做 PATH 探测与启动。
- 不修改设置页错误提示的双写行为。
- 不自动合并回 `dev`：完成后先询问用户。
