# Agents 菜单、拖拽滚动与设置页加载修复 — 设计

> 状态：**已完成实现与代码审查修复，待用户确认合并**
> 关联任务：[task.md](task.md) · [review.md](review.md)
> 基线：`.worktrees/agents-menu-drag-scroll`（分支 `feature/agents-menu-drag-scroll`，切自 `dev`）
> 范围：侧栏 `Agents` 右键菜单、新建 Agent 模态（含实时搜索与滚动）、列表拖拽滚轮与边缘自动滚动、`lx` 选区边缘自动滚动、浏览器设置页 JS 修复

## 1. 背景与目标

本次包含以下同属交互与稳定性闭环的改动：

1. **终端 Agent 也能 Close**：当前只有 lx 条目稳定提供 `Close`；opencode/claude/codex 等终端 Agent 在“标签内唯一窗格”时没有 `Close`，因为底层禁止删除最后一个窗格。
2. **新增 `New agent` 入口与可搜索模态**：左侧 Agents 条目菜单与 Agents 空白区域都能右键新建 Agent；弹窗按本机 `PATH` 探测 8 类 CLI coding agent，顶部提供单行搜索框（按实际命令与启动别名/稳定名不区分大小写子串过滤，支持点击拖选、粘贴/剪切、滚轮与溢出滚动条），选中后新建标签并启动。
3. **拖拽列表允许滚轮与边缘自动滚动**：左侧工作区活跃/历史列表、右侧 Todo、右侧 Prompt 树在拖拽过程中允许滚轮滚动视口，并在上下各 2 行检测区与外溢区域支持 30ms 节拍边缘自动滚动，滚动后按指针当前位置刷新落点。
4. **`lx` 主内容区选区边缘自动滚动**：`lx` 主消息列表与后台任务页（`Processes` 输出、`Agents` 详情、`BTW` 对话）支持在上下各 2 行检测区与外溢区域自动滚动并保持多击（`Word` / `Line` / 表格 / 代码块）选区粒度。
5. **修复设置页 `element is not defined`**：模型/供应商、Quick connect、模型卡片与 Subagents 页面共用的 `element()` 全局函数已在重构中删除，调用点漏改。

目标不是重写侧栏或终端架构，而是在现有 `app/` 纯状态、`ui/` 纯渲染、`event/` 只分派的结构内做最小闭环。

## 2. 已确认决策

| 编号 | 问题 | 最终结论 |
|---|---|---|
| Q1 | 终端 Agent 的 `Close` 语义 | 关窗格 → 唯一窗格则关标签 → 唯一标签则先新建空白标签再关旧标签；工作区始终保留至少一个 tab |
| Q2 | 新建弹窗候选列表 | 只列 PATH 中真实安装的 8 类 CLI Agent；同一 Agent 只出现一次；不列 lx；空列表显示 `No coding agents installed`，过滤无命中显示 `No matching agents` |
| Q3 | `New agent` 菜单位置 | Agent 条目菜单第一项；Agents 列表空白区域右键只弹单项 `New agent`；滚动条列与折叠按钮不算空白（且仅真正打开菜单时扫描 PATH） |
| Q4 | 新建弹窗交互 | 顶部单行搜索框（匹配实际命令与启动别名/稳定名称）+ 分割线 + 单选列表 + 呼吸行 + 按钮；文本变化高亮回首行、重置 `viewport` 与双击记录；支持鼠标点击/拖选/粘贴/剪切/滚轮/滚动条；`↑/↓` 移动高亮；`Enter`/`Create`/双击创建并在标签栏溢出时自动滚入可见；`Esc` 有选区先清选区、再按关闭 |
| Q5 | 拖拽与选区自动滚动 | 工作区活跃/历史、Todo、Prompt 树支持拖拽滚轮与边缘自动滚动；`lx` 主消息列表与后台任务页（Processes/Agents/BTW）选区支持边缘自动滚动；上下各 2 行检测区、30ms 一步、越靠外越快（1..=3 行），同向横向移动不推迟 `next_at`；Todo/Prompt 拖出右栏水平外取消，右栏内（含滚动条列）纵向钳位保持落点 |
| Q6 | Agent 启动方式 | 新标签先起普通 shell，150ms 后 PTY 就绪写入 `<可执行文件名>\r`（`next_deadline` 纳入唤醒）；agent 退出后回到 shell，窗格不自动关闭 |
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
if let Some(index) = ui::agent_item_index_at(view, state, column, row) {
    state.installed_agents = crate::detect::installed_agents();
    update::open_agent_menu(state, index, (column, row));
    return true;
}
if ui::agents_blank_at(view, state, column, row) {
    state.installed_agents = crate::detect::installed_agents();
    update::open_agents_blank_menu(state, state.active_workspace, (column, row));
    return true;
}
```

顺序必须保持在 Prompt/Todo/窗格分支之前，避免 Agents 空白点击穿透到底层主区；且仅在真正打开 Agents 菜单时扫描 `PATH`，避免右键滚动条列触发无效磁盘探测。

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

- 跳过 `PATH` 中由前后或连续分隔符产生的空段，避免退化成按 TUI 进程当前工作目录查找相对路径。
- 非 Windows 平台（`#[cfg(not(windows))]`）：先校验 `metadata().is_file()`，再用 `libc::access(c_path, libc::X_OK) == 0` 检验当前有效用户的实际可执行权限（避免 root 属主 `0o700` 等情况误判）。
- Windows 平台（`#[cfg(windows)]`）：在遍历 `PATH` 前单次解析 `PATHEXT`（默认 `.exe`/`.cmd`/`.bat`/`.com`），显式带扩展名按原样校验 `is_file()`，无扩展名依次拼接扩展名校验。

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

### 4.4 New agent 模态（含实时搜索与滚动）

#### 4.4.1 状态

`src/app/overlay.rs`：

```rust
#[derive(Debug)]
pub struct NewAgentDialog {
    pub workspace: usize,
    pub agents: Vec<crate::detect::InstalledAgent>,
    pub query: TextInput,
    pub selected: usize,
    pub viewport: Option<usize>,
}
```

- `filtered_indices()`：对 `agent.command` 与 `detect::launch_names(agent.kind)` 全部别名（含稳定小写名称，避免 `app -> ui::text` 依赖扩散）做大小写不敏感子串匹配。
- `OverlayKind::NewAgent`、`Overlay::NewAgent(NewAgentDialog)`、`Overlay::kind()` 同步增加，并完整接入 `Overlay::input()`、`Overlay::input_mut()`、`Overlay::clear_selection()`（支持终端粘贴、剪切、选区复制、`Esc` 先清选区及搜索框内鼠标点击/拖选）。

#### 4.4.2 打开

`src/app/update/new_agent.rs`（新文件）：

- `open_new_agent_dialog(state, workspace)`：校验工作区，快照 `installed_agents`，重置 `new_agent_click` 与 `new_agent_scroll_drag`。
- `activate_menu` 增加 `(Some(MenuCommand::NewAgent), OverlayTarget::Agent { workspace, .. } | OverlayTarget::AgentsBlank { workspace }) => open_new_agent_dialog(state, *workspace)`。

#### 4.4.3 键盘与粘贴/剪切

- `src/input/mod.rs` `overlay_key`：`OverlayKind::NewAgent` 映射 `Esc` / `Enter` / `Up` / `Down`，其余走 `overlay_text_edit_key(key)` 支持完整单行编辑与词/剪切快捷键。
- `src/app/update/dialog.rs`：前置分支拦截 `Overlay::NewAgent` 交由 `apply_new_agent_key`；文本编辑、`apply_overlay_paste` 与 `cut_overlay_selected_text` 改变搜索词时均调用 `dialog.normalize_selection()`（`selected = 0; viewport = None`）并清空 `state.new_agent_click` 与 `state.new_agent_scroll_drag`。
- `src/event/input.rs`：键盘 `Enter` 创建并激活新标签后调用 `ensure_workspace_visible` 与 `reveal_active_tab`，标签栏溢出时自动滚入可见。

#### 4.4.4 鼠标

- `src/event/overlay.rs`：`overlay_input_rect` 返回 `new_agent_search_rect(&shell)`，复用公共输入框点击/拖选处理。点击行单选、400ms 同行双击立即创建；点击 `Create` 创建；点击 `Cancel` 或弹窗外关闭；点击内部空白保持打开。
- `src/event/mouse/wheel.rs` / `press.rs` / `drag.rs`：支持弹窗内滚轮滚动（不改 `selected`）与矮屏溢出滚动条点击/拖拽（`new_agent_scroll_drag`）。

#### 4.4.5 渲染与几何

`src/ui/overlay/new_agent.rs`（新文件）：

- 宽度固定 48；高度随快照条目总数计算为 `rows.clamp(1, 8) + 8`（内框依次为搜索行 + `DIVIDER_MID` 分隔线 + 候选列表 + 呼吸空行 + 底部 `create` / `cancel` 按钮；矮屏下逐级省略呼吸行与分隔线）。
- 列表窗口起点 `new_agent_visible_start` 按过滤结果数夹紧高亮或取显式 `viewport`；内容溢出时右侧 1 列绘制 `widgets::scrollbar` thumb。
- 左侧显示 `text::agent_label(kind)`，右侧在 `command != label` 时压暗显示实际命令名；无安装显示 `No coding agents installed`，有安装无命中显示 `No matching agents`（均居中截断）且 `create` 按钮置灰。
- 经公共 `render_input` 返回硬件光标位置。

#### 4.4.6 创建与启动

`src/app/update/workspace.rs`：

```rust
pub(crate) const AGENT_LAUNCH_DELAY: Duration = Duration::from_millis(150);

pub fn create_agent_tab(
    state: &mut AppState,
    workspace: usize,
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

- `PendingAgentLaunch` 放在 `src/app/state/pane.rs`，`Pane::new()` 初始化为 `None`。
- `src/app/update/interaction.rs` 的 `next_deadline` 纳入 `pending_launch.at` 确保空闲时按时唤醒写入命令。

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

### 4.7 列表拖拽与 `lx` 选区的边缘自动滚动

新增 `src/event/mouse/autoscroll.rs` 并在 `src/app/state/interaction.rs` / `src/app/update/selection/` 统一提供边界计算与自动滚动调度：

- **检测区与节拍**：
  - `autoscroll_edge(rect, row)` 将视口上下各 2 行（`AUTOSCROLL_BAND_ROWS = 2`）作为检测区；越出视口时 `distance > 0`，先在首次越界/距离增大时立即按 `edge_scroll_lines`（`3..=15` 行）步进一次，随后每 `30ms` 步进 `1 + distance / 3`（夹在 `1..=3` 行）。
  - 同方向边缘内横向移动或抖动**保留原有 `next_at`**，避免不断重置节拍导致自动滚动饥饿或反复暴跳。
  - `next_deadline` 纳入 `drag_autoscroll.next_at` 与 `lx_selection_autoscroll.next_at`，空闲等待时按时唤醒事件循环。
- **四个列表拖拽表面**（`DragScrollSurface`：`WorkspaceActive`、`WorkspaceHistory`、`Todo`、`PromptTree`）：
  - `WorkspaceActive`/`WorkspaceHistory` 横向不限；`Todo`/`PromptTree` 仅要求指针横向留在 `view.right_panel`（包含滚动条列）内、纵向越出列表时按列表首/末行钳位保持目标反馈，横向拖出右栏进入主区时返回 `None` / `Outside` 取消落点并停止自动滚动。
  - `Todo` 与 `PromptTree` 仅在已进入 `dragging`（或已垂直越出列表且 `enabled`）时才激活自动滚动，防止边缘两行内的普通点击/双击因 1 列横向抖动被误升级为合并/移动。
  - `wheel.rs` 与 `tick_drag` 统一检查 `state.overlay.is_none()`、`WorkspaceMode`、`RightTab::Todo && !prompt_collapsed` 与 `prompt_panel_active && prompt_tree_mode`；`handle_mouse_down_right`、`open_prompt_*_menu`、`open_pane_menu`、`close_pane` 与 `close_tab` 对称清理在途拖拽、自动滚动与锚定 toast。
- **`lx` 主内容区选区自动滚动**（`LxScrollSurface`：`History`、`TaskOutput`、`TaskAgents`、`TaskBtw`）：
  - 统一放在 `event/mouse/autoscroll.rs` 执行，每次步进前先校验 `overlay.is_none()`、窗格视图与活动页签匹配，并调用 `history_rows` 或 `resolve_task_{output,agents,btw}_visible` 解析实时可见行数与总行数（解决 `rows_dirty` / `visible == 0` 失效及 `chat_bar`/`sticky` 高度冻结问题）。
  - 滚动后实时调用 `content_geom` + `unit_cell_range` + `drag_lx_selection_range` 推进终点，完整保留 `Character` / `Word` / `Line` / 表格 / 代码块选区粒度。
  - 修复 `scroll_lx_history` 在 `follow_bottom == true` 时恒返回 `true` 的贴底判定 bug（并同步修复 Prompt 选区 `autoscroll_step` 在底部无法上滚、顶部死循环的对称缺陷），触底/触顶立即返回 `false` 并清空计划。

## 5. 文件变更清单

### 新增

- `src/app/state/interaction.rs`（自 `state.rs` 拆出交互/拖拽/自动滚动类型）
- `src/app/update/selection/{mod.rs, lx.rs, autoscroll.rs}`（自 `selection.rs` 按职责拆分）
- `src/app/update/new_agent.rs`、`src/app/update/new_agent/tests.rs`
- `src/ui/overlay/new_agent.rs`、`src/ui/overlay/new_agent/tests.rs`
- `src/platform/executable.rs`、`src/platform/executable/tests.rs`
- `src/event/mouse/autoscroll.rs`
- `src/event/tests/session_launch.rs`、`src/event/tests/mouse_drag.rs`
- `src/input/tests/{mod.rs, overlay.rs}`（自超限 `src/input/tests.rs` 拆分）
- `docs/feature/agents-menu-drag-scroll-and-settings-fix/{design.md, task.md, review.md}`

### 修改

- `src/app/state.rs`、`src/app/state/lx.rs`、`src/app/state/pane.rs`
- `src/app/overlay.rs`、`src/app/overlay/tests.rs`
- `src/app/update.rs`、`src/app/update/dialog.rs`、`src/app/update/interaction.rs`、`src/app/update/menu.rs`、`src/app/update/menu/tests.rs`、`src/app/update/prompt/drag.rs`、`src/app/update/prompt/menu.rs`、`src/app/update/quit.rs`、`src/app/update/todo/drag.rs`、`src/app/update/workspace.rs`、`src/app/update/workspace_history/mod.rs`、`src/app/update/workspace_list.rs`
- `src/app/update/chat/viewport.rs`、`src/app/update/chat/tests/history.rs`、`src/app/update/selection/tests.rs`
- `src/detect/mod.rs`、`src/detect/tests.rs`
- `src/platform/mod.rs`
- `src/event/mod.rs`、`src/event/session.rs`、`src/event/overlay.rs`、`src/event/input.rs`
- `src/event/mouse/{mod.rs, press.rs, drag.rs, wheel.rs}`
- `src/event/tests.rs`、`src/event/tests/{agent_menu.rs, overlay.rs, prompt_tree.rs, todo.rs, workspace_history.rs}`
- `src/input/mod.rs`
- `src/ui/left_panel/{mod.rs, history.rs, tests.rs}`、`src/ui/main_content/tests.rs`、`src/ui/mod.rs`、`src/ui/overlay.rs`、`src/ui/text.rs`、`src/ui/text/workspace.rs`、`src/ui/text/tests.rs`
- `assets/settings_{models, models_card, connect, subagents}.js`
- `tests/web_assets.rs`
- `docs/standards/tui-design-requirements.md`

## 6. 异常与边界

| 场景 | 行为 |
|---|---|
| PATH 中没有任何支持的 Agent | 弹窗显示 `No coding agents installed`，`create` 不可用，`cancel` 可关 |
| 搜索过滤无命中 | 弹窗显示 `No matching agents`，`create` 不可用；改词/粘贴/剪切回到首行 |
| `opencode` 与 `opencode2` 同时安装 | 只显示一个 `opencode` 条目，启动命中的第一个候选名 |
| 右键 Agents 滚动条列 | 不打开菜单，不触发无效 PATH 扫描 |
| 菜单打开后目标工作区消失 | `open_new_agent_dialog` 校验 workspace，失败不打开 |
| 弹窗打开后工作区消失 | `create_agent_tab` 返回 None，弹窗关闭但不创建标签 |
| PTY spawn/写入失败 | 清 pending，保留空白终端，显示 danger toast |
| 终端 Agent 是唯一窗格 | 关标签；唯一标签则替换为空白 lx 标签 |
| 终端 Agent 是唯一窗格且 lx 同窗格有会话 | 关闭窗格/标签会销毁 lx 视图，DB 会话仍可通过 `/resume` 恢复（沿用现有 D3 语义） |
| 拖拽滚轮/自动滚动到达列表边界 | 钳制停止不空转；`Workspace` 落点吸附首/末条，`Todo`/`Prompt` 在右栏内钳位保持落点、拖出右栏取消 |
| `lx` 选区自动滚动贴底/触顶 | 立即返回 `false` 并停止计划；保留多击 `Word`/`Line`/表格/代码块选区粒度 |
| 弹窗打开时滚轮或定时 tick | overlay 优先，拖拽与选区自动滚动不拦截且自愈停止 |
| 设置页真实 API 加载失败 | 仍显示 `Failed to load: ...`，但不再是 `element is not defined` |

## 7. 测试计划

### 7.1 状态/更新层

- `open_agent_menu`、`close_terminal_agent`、`open_agents_blank_menu`、`open_new_agent_dialog`、`create_selected_agent`、搜索过滤（命令 + 别名）、粘贴/剪切/双击重置、`scroll_lx_history` 贴底不死循环与非消息历史滚动、Prompt 选区上下边界、同方向 `next_at` 不推迟、`flush_pending_agent_launches`。

### 7.2 UI 层

- `agents_blank_at`、NewAgent 弹窗几何（含分隔线、呼吸行、短屏降级、滚动条与 `filtered` 边界夹取）、`BUTTON_CREATE`/`NEW_AGENT_*` 文案。

### 7.3 事件层与集成

- 右键条目/空白/滚动条列（不扫 PATH）、点击 `New agent` 打开弹窗、搜索框点击拖选/`Ctrl+C`/首次 `Esc` 清选区、滚轮滚动、键盘 `Enter` 露出溢出标签、列表拖拽滚轮与边缘自动滚动（防抖、滚动条列、纵向钳位、右栏外取消、右键清理）、`lx` 主消息与后台任务页 `Processes`/`Agents`/`BTW` 选区自动滚动（含 `Word`/`Line` 保留与 `rows_dirty` 解析）、`platform/executable` 空 PATH 段过滤、`tests/web_assets.rs`。

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
- 不做 Agent 安装/卸载管理，只做 PATH 探测与启动。
- 不修改设置页错误提示的双写行为。
- 不自动合并回 `dev`：完成后先询问用户。
