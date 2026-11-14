# 侧栏 Agents 条目第二行标题与右键菜单 — 设计

> 状态：**待用户审核**（审核通过前不修改生产代码）
> 关联任务：[task.md](task.md)
> 依据：用户需求 + 11 轮 Grill Me 收口结论
> 说明：用户提到的 `ref-content/ui/left_panel/` 在当前仓库与工作区中不存在；本设计以用户给出的行格式示例为唯一视觉依据。

---

## 1. 背景与目标

当前左侧栏 `Agents` 分区每个条目只有一行：

```text
❯ ● lx-native            [tab 1]
  ○ claude               [tab 2]
```

用户要求：

1. 每个条目增加第二行，显示 Agent 标题；
2. 第一行把 `[tab N]` 改为 `[T1 P1]`，同时标识 Tab 与 Pane；
3. 条目支持右键菜单：编辑标题、关闭条目（行内二次确认）；
4. 所有对 Agent 条目的修改**只存在于内存**，不写 DB、不修改会话标题、不修改终端 OSC 标题、不影响 `/resume` 与输入框右上角标题胶囊。

目标形态：

```text
❯ ● lx-native            [T1 P1]
  └─ Fix login bug
  ○ claude               [T1 P2]
  └─ (no title)
```

右键菜单：

```text
┌─ claude ──────┐
│ Edit title    │
│ Close         │
└───────────────┘
```

第一次点击 `Close` 后：

```text
│ Confirm close │
```

第二次点击/回车才真正执行；切换选中项或关闭菜单自动取消确认。

---

## 2. 现状核实（Codegraph / Codebase Memory）

| 事实 | 现有代码位置 | 对本设计的影响 |
|---|---|---|
| Agents 条目是派生视图，没有独立持久化实体 | `AppState::agent_entries`（`src/app/state.rs`）遍历当前工作区各 Tab、各 Pane，按 `pane.lx.has_active_session()` 与 `pane.agent.is_some()` 生成 | “编辑标题/关闭”不能假设存在 Agent 记录；标题覆盖必须挂在 Pane/会话的运行时状态上 |
| `SidebarAgentEntry` 只含 `tab_index / pane_id / kind / state` | `src/app/state/pane.rs` | 需要补 `pane_index` 供 `[T1 P1]` 使用 |
| `AgentListItem` 与 `agent_items` 只承载一行数据 | `src/ui/left_panel/mod.rs` | 需要补 `pane_index` 与第二行标题（借用 `&str`，避免每帧分配） |
| lx 会话标题已有内存字段 | `LxChat::session_title`（`src/app/state/lx/chat.rs`） | lx 条目的默认标题直接读它，不改它 |
| 终端 Agent 的 OSC 标题已进入快照 | `PaneAgentSnapshot.title`（`src/detect/mod.rs`），由 `poll_active_agents` 从 `terminal.title()` 捕获 | 终端条目的默认标题直接读它，不改它 |
| 工作区、Tab、提示词已有统一重命名浮层 | `Overlay::Rename` / `RenameTarget` / `commit_rename`（`src/app/overlay.rs`、`src/app/update/dialog.rs`） | Agent 标题编辑复用该浮层，只新增目标变体 |
| 右键菜单已有统一下拉组件与行内二次确认 | `MenuCommand::requires_confirm`、`Menu::confirming`、`menu_labels`（`src/app/overlay.rs`、`src/ui/overlay/menu.rs`） | 不需要新增弹窗组件 |
| 右键分派当前没有任何 Agent 分支 | `open_right_click_menu`（`src/event/overlay.rs`） | 需要在工作区命中之后新增 Agent 条目命中 |
| `close_pane` 拒绝关闭标签页最后一个窗格 | `Tab::remove_pane` / `TileLayout::remove_pane`（`src/app/state/pane.rs`、`src/tiling.rs`） | 终端 Agent 是唯一窗格时不能提供 `Close`，否则菜单点了没反应 |
| lx `/new` 已有完整“停流 + 解绑会话 + 回欢迎页”路径 | `update::new_session`（`src/app/update/chat/session.rs`） | lx 条目的 `Close` 直接复用，保证 DB 历史保留 |
| Agents 滚动与命中按“一行一个条目”计算 | `agent_list_rows` / `agent_item_index_at` / `agent_scrollbar_for`（`src/ui/left_panel/mod.rs`） | 必须改成“两行一个条目”的条目槽位模型 |
| 侧栏 Agents 区已有内存悬停/滚动/拖拽状态 | `AppState::agent_hover` / `agent_scroll` / `agent_scroll_drag` | 沿用，不新增滚动状态 |

---

## 3. 已确认决策（Grill Me 收口）

| 编号 | 决策维度 | 确认结论 |
|---|---|---|
| D1 | `Delete` 的语义 | 不删除任何内容，只“关闭”；菜单文案统一使用 `Close` |
| D2 | lx 条目的 `Close` | 只关闭当前会话视图，等价 `/new`：停流、解绑 `session_id`、回到空白欢迎页；**不删 DB 会话，不删历史消息**；窗格和同窗格终端 Agent 保留 |
| D3 | 终端 Agent 的 `Close` | 关闭其所属窗格并结束进程；同窗格 lx 会话会随窗格一起关闭，但 DB 历史仍在 `/resume` |
| D4 | 标题编辑范围 | lx 与终端 Agent 都提供 `Edit title`；全部只改侧栏显示 |
| D5 | 标题修改的持久化 | **纯内存**：不写 DB、不改 `LxChat::session_title`、不改输入框右上角标题胶囊、不改 `/resume` 列表、不向终端进程写任何内容 |
| D6 | 默认标题来源 | `lx-*` 读 `chat.session_title`；终端 Agent 读 `PaneAgentSnapshot.title`（终端 OSC 标题） |
| D7 | 空标题与清空编辑 | 默认标题为空时第二行显示 `└─ (no title)`；`Edit title` 输入非空 = 设置内存覆盖，输入空 = 清除覆盖并回落默认标题 |
| D8 | 内存覆盖生命周期 | 覆盖按“当前 Agent 条目身份”保存：切 Tab、切工作区、窗格重排后仍保留；条目消失（lx 会话关闭/切换、终端 Agent 退出或种类变化、窗格/工作区关闭）即清除；进程重启全部丢失 |
| D9 | `Close` 二次确认形式 | 菜单行内二次确认：`Close` → `Confirm close`，与现有 `CloseWorkspace` / `CloseTab` / `ClosePane` 一致 |
| D10 | `[T1 P1]` 编号规则 | `T` = 当前工作区内 Tab 位置（1 基）；`P` = 窗格在该 Tab `layout.pane_ids()` 树序中的位置（1 基）；同一窗格的 lx 与终端 Agent 两行显示相同编号；标签右对齐，名称先截断 |
| D11 | 终端 Agent 是标签页唯一窗格时 | 菜单只显示 `Edit title`，不显示 `Close`；关闭整个 Tab 仍走标签栏右键菜单 |
| D12 | 编辑标题交互 | 复用 `Overlay::Rename` 居中单行输入框；`Enter` 保存，`Esc` 取消 |
| D13 | lx 条目在生成中执行 `Close` | 允许；复用 `/new` 的停流收尾，已落库消息保留 |
| D14 | 标题覆盖与默认标题的关系 | 覆盖优先；清除覆盖后立即回落到实时默认标题（lx 自动标题后续变化仍会显示，终端 OSC 标题变化也会显示） |

---

## 4. 数据模型与状态设计

### 4.1 `SidebarAgentEntry` 增加 `pane_index`

`src/app/state/pane.rs`：

```rust
/// 侧栏 `Agents` 分区的单行条目数据。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SidebarAgentEntry {
    pub tab_index: usize,
    /// 窗格在该标签页 `layout.pane_ids()` 树序中的位置（1 基，`[T1 P1]` 的 P）。
    pub pane_index: usize,
    pub pane_id: PaneId,
    pub kind: SidebarAgentKind,
    pub state: AgentState,
}
```

`src/app/state.rs` 的 `agent_entries()` 改为按树序枚举：

```rust
pub fn agent_entries(&self) -> Vec<SidebarAgentEntry> {
    let workspace = self.active_workspace();
    let mut entries = Vec::new();
    for (tab_index, tab) in workspace.tabs.iter().enumerate() {
        for (pane_offset, pane_id) in tab.layout.pane_ids().into_iter().enumerate() {
            let Some(pane) = tab.pane(pane_id) else {
                continue;
            };
            let pane_index = pane_offset + 1;
            if pane.lx.has_active_session() {
                entries.push(SidebarAgentEntry {
                    tab_index,
                    pane_index,
                    pane_id,
                    kind: SidebarAgentKind::Lx {
                        entry: pane.lx.entry,
                        ai_web_site: pane.lx.ai_web_site,
                    },
                    state: pane.lx.agent_state(),
                });
            }
            if let Some(snapshot) = pane.agent.as_ref() {
                entries.push(SidebarAgentEntry {
                    tab_index,
                    pane_index,
                    pane_id,
                    kind: SidebarAgentKind::Terminal(snapshot.kind),
                    state: snapshot.state,
                });
            }
        }
    }
    entries
}
```

**关键点**：

- `pane_index` 基于 `layout.pane_ids()` 的树序，不是 `PaneId` 原始值；关闭其他窗格后不会出现 `[T1 P7]`。
- 同一窗格两条记录共享同一个 `pane_index`。
- `agent_entry_count()` 不需要改；它只计数，不关心序号。

### 4.2 内存标题覆盖的存放位置

覆盖分两处存放，均属于运行时状态，不进入 DB：

1. **lx 会话覆盖**：放在 `LxChat` 内，跟随会话视图生命周期。

`src/app/state/lx/chat.rs`：

```rust
pub struct LxChat {
    // ...
    /// 侧栏 Agents 条目的临时显示标题（仅内存；不改会话标题、不落库、
    /// 不影响输入框右上角标题胶囊与 `/resume` 列表）。
    pub agent_title_override: Option<String>,
}
```

`LxChat::new()` 初始化为 `None`。

`LxChat::reset_session_view()` 末尾增加：

```rust
self.agent_title_override = None;
```

`on_session_loaded_full`（`src/app/update/chat/session.rs`）的 `session_changed` 分支增加：

```rust
target.lx.chat.agent_title_override = None;
```

这样 `/new`、删除会话、切换 lx 模式、切换 AI Web 站点、`/resume` 切换到另一条会话、rewind 删除会话等路径都会清空覆盖。

2. **终端 Agent 覆盖**：放在 `Pane` 上，并带 `AgentKind` 标签。

`src/app/state/pane.rs`：

```rust
pub struct Pane {
    // ...
    /// 侧栏 Agents 中终端 Agent 的临时显示标题（仅内存；Agent 种类变化、
    /// 退出或窗格关闭时清除）。
    pub terminal_agent_title: Option<(AgentKind, String)>,
}
```

`Pane::new()` 初始化为 `None`。

`update_pane_agent`（`src/app/update/pane.rs`）在写入快照前清理：

```rust
pub fn update_pane_agent(
    state: &mut AppState,
    id: PaneId,
    snapshot: Option<PaneAgentSnapshot>,
) -> bool {
    let Some(pane) = state.pane_mut_anywhere(id) else {
        return false;
    };
    if pane.agent == snapshot {
        return false;
    }
    let previous_kind = pane.agent.as_ref().map(|snapshot| snapshot.kind);
    let next_kind = snapshot.as_ref().map(|snapshot| snapshot.kind);
    if previous_kind != next_kind {
        pane.terminal_agent_title = None;
    }
    pane.agent = snapshot;
    true
}
```

**关键点**：

- 终端 Agent 同行更新 state（比如 `Idle → Working`）或 OSC 标题变化时，覆盖保留。
- Agent 退出（`Some → None`）或切换成另一种 Agent（`claude → codex`）时，覆盖清除。
- 窗格关闭时 `Pane` 整体销毁，覆盖自然消失。
- 不使用全局 `HashMap`，避免工作区索引变化、PaneId 复用和跨重启残留问题。

### 4.3 标题解析

`src/app/state/pane.rs` 的 `impl Pane` 新增：

```rust
impl Pane {
    /// 侧栏 Agents 条目的第二行标题：内存覆盖优先，其次 lx 会话标题 /
    /// 终端 Agent 的 OSC 标题；空标题返回 None。
    pub fn agent_display_title(&self, kind: SidebarAgentKind) -> Option<&str> {
        match kind {
            SidebarAgentKind::Lx { .. } => self
                .lx
                .chat
                .agent_title_override
                .as_deref()
                .or(self.lx.chat.session_title.as_deref()),
            SidebarAgentKind::Terminal(agent) => self
                .terminal_agent_title
                .as_ref()
                .filter(|(kind, _)| *kind == agent)
                .map(|(_, title)| title.as_str())
                .or_else(|| {
                    self.agent
                        .as_ref()
                        .filter(|snapshot| snapshot.kind == agent)
                        .and_then(|snapshot| snapshot.title.as_deref())
                })
                .filter(|title| !title.is_empty()),
        }
    }

    /// 写入/清除侧栏标题覆盖；目标 Agent 已不在该窗格时忽略。
    pub fn set_agent_title_override(
        &mut self,
        kind: SidebarAgentKind,
        title: Option<String>,
    ) {
        let title = title.filter(|title| !title.trim().is_empty());
        match kind {
            SidebarAgentKind::Lx { .. } if self.lx.has_active_session() => {
                self.lx.chat.agent_title_override = title;
            }
            SidebarAgentKind::Terminal(agent)
                if self.agent.as_ref().is_some_and(|snapshot| snapshot.kind == agent) =>
            {
                self.terminal_agent_title = title.map(|title| (agent, title));
            }
            _ => {}
        }
    }
}
```

**关键点**：

- lx 覆盖不校验 `LxEntry` / `AiWebSite`，因为所有会切换这些身份的路径都会清空会话视图并调用 `reset_session_view`。
- 终端覆盖带 `AgentKind` 校验，避免 `claude` 的缓存标题被 `codex` 复用。
- 返回 `Option<&str>` 供 UI 直接借用，避免每帧克隆字符串。

### 4.4 UI 条目模型

`src/ui/left_panel/mod.rs`：

```rust
/// Agents 分区条目：标签索引、标签内窗格序号、窗格标识、Agent 种类、
/// 运行状态与第二行标题；渲染与命中一一对应。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AgentListItem<'a> {
    pub tab_index: usize,
    pub pane_index: usize,
    pub pane_id: PaneId,
    pub kind: SidebarAgentKind,
    pub state: AgentState,
    /// 第二行标题：内存覆盖优先，其次会话标题 / 终端 OSC 标题。
    pub title: Option<&'a str>,
}

/// 当前激活工作区的 Agents 条目（按标签与树序，同一窗格先 lx 后终端 Agent）。
pub fn agent_items<'a>(state: &'a AppState) -> Vec<AgentListItem<'a>> {
    state
        .agent_entries()
        .into_iter()
        .map(|entry| {
            let title = state
                .pane_anywhere(entry.pane_id)
                .and_then(|pane| pane.agent_display_title(entry.kind));
            AgentListItem {
                tab_index: entry.tab_index,
                pane_index: entry.pane_index,
                pane_id: entry.pane_id,
                kind: entry.kind,
                state: entry.state,
                title,
            }
        })
        .collect()
}
```

**关键点**：

- `AgentListItem` 带生命周期，但仍是 `Copy`，测试与调用方改动最小。
- 标题借用 `AppState`，不引入每帧字符串分配。
- `agent_target_at` 只使用 `pane_id` / `kind.target_view()`，不受影响。

---

## 5. 菜单模型与更新逻辑

### 5.1 新增菜单目标与命令

`src/app/overlay.rs`：

```rust
/// 菜单与关闭确认作用目标：在既有工作区/Tab/Pane/提示词之外，新增
/// 侧栏 Agents 条目（同一窗格可能同时有 lx 与终端两条，必须带 kind）。
pub enum OverlayTarget {
    // ...
    Agent {
        workspace: usize,
        tab: usize,
        pane: PaneId,
        kind: SidebarAgentKind,
    },
}

pub enum MenuCommand {
    // ...
    /// 侧栏 Agents：编辑标题（纯内存覆盖）。
    EditAgentTitle,
    /// 侧栏 Agents：关闭条目（lx 关会话视图，终端 Agent 关窗格）。
    CloseAgent,
}

impl MenuCommand {
    pub fn requires_confirm(self) -> bool {
        matches!(
            self,
            // ...
            Self::CloseAgent
        )
    }
}
```

`RenameTarget` 增加：

```rust
pub enum RenameTarget {
    // ...
    AgentTitle {
        workspace: usize,
        tab: usize,
        pane: PaneId,
        kind: SidebarAgentKind,
    },
}
```

`RenameTarget` 仍可保持 `Copy`，因为 `SidebarAgentKind` 是 `Copy`。

### 5.2 打开 Agent 菜单

`src/app/update/menu.rs` 新增：

```rust
/// 打开 Agents 条目右键菜单：`Edit title` 恒可用；`Close` 对 lx 恒可用，
/// 终端 Agent 仅在其窗格不是当前标签页唯一窗格时可用。
pub fn open_agent_menu(state: &mut AppState, index: usize, anchor: (u16, u16)) {
    let Some(entry) = state.agent_entries().get(index).copied() else {
        return;
    };
    state.agent_scroll_drag = None;
    let workspace = state.active_workspace;
    let mut commands = vec![MenuCommand::EditAgentTitle];
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
    state.overlay = Some(Overlay::Menu(Menu {
        anchor,
        target: OverlayTarget::Agent {
            workspace,
            tab: entry.tab_index,
            pane: entry.pane_id,
            kind: entry.kind,
        },
        commands,
        selected: 0,
        confirming: false,
    }));
}
```

**关键点**：

- 命中下标来自 `ui::agent_item_index_at`，与渲染同源。
- 菜单目标记录 `workspace/tab/pane/kind`，同一窗格两条记录不会串。
- `Close` 的可用性在“打开菜单时”判定；菜单存续期间窗格被外部关闭的竞态，由执行时的二次校验兜底。

### 5.3 菜单命令执行

`src/app/update/menu.rs` 的 `activate_menu` 新增分支：

```rust
(Some(MenuCommand::EditAgentTitle), OverlayTarget::Agent { workspace, tab, pane, kind }) => {
    let current = state
        .workspaces
        .get(workspace)
        .and_then(|workspace| workspace.tabs.get(tab))
        .and_then(|tab| tab.pane(pane))
        .and_then(|pane| pane.agent_display_title(kind))
        .unwrap_or_default()
        .to_string();
    state.overlay = Some(Overlay::Rename(Rename {
        target: RenameTarget::AgentTitle { workspace, tab, pane, kind },
        input: TextInput::new(current),
    }));
}
(Some(MenuCommand::CloseAgent), OverlayTarget::Agent { workspace, tab, pane, kind }) => {
    close_agent(state, workspace, tab, pane, kind);
}
```

关闭逻辑：

```rust
/// 关闭一个 Agents 条目。执行前再次确认目标仍存在，避免菜单打开期间
/// Agent 退出/会话切换导致误关。
fn close_agent(
    state: &mut AppState,
    workspace: usize,
    tab: usize,
    pane: PaneId,
    kind: SidebarAgentKind,
) {
    if state.active_workspace != workspace
        || !state
            .agent_entries()
            .iter()
            .any(|entry| entry.tab_index == tab && entry.pane_id == pane && entry.kind == kind)
    {
        return;
    }
    match kind {
        SidebarAgentKind::Lx { .. } => {
            // 等价 `/new`：停流、解绑当前会话、回空白欢迎页；DB 历史保留。
            let _ = super::chat::new_session(state, pane);
        }
        SidebarAgentKind::Terminal(_) => {
            // 菜单只在该窗格可关闭时提供 Close；close_pane 仍保留最后窗格保护。
            super::workspace::close_pane(state, workspace, tab, pane);
        }
    }
}
```

**关键点**：

- lx 关闭复用 `update::new_session`，它内部先 `stop_stream`（生成中也能关），再 `reset_session_view`，不触发 `DeleteChatSession`。
- 终端关闭复用 `close_pane`，不新增进程信号路径。
- 目标二次校验避免“菜单打开后 Agent 退出、同一窗格换了另一个 Agent，Close 误杀新 Agent”。

### 5.4 重命名提交

`src/app/update/dialog.rs` 的 `commit_rename` 调整：

```rust
fn commit_rename(state: &mut AppState) {
    let Some(Overlay::Rename(rename)) = state.overlay.as_ref() else {
        return;
    };
    let name = rename.input.text().trim().to_string();
    let target = rename.target;
    // 只有 Agent 标题允许空值：空 = 清除内存覆盖，回落默认标题。
    let allows_empty = matches!(target, RenameTarget::AgentTitle { .. });
    if name.is_empty() && !allows_empty {
        return;
    }
    match target {
        // ...既有 Workspace / WorkspaceHistory / Tab / PromptFolder / PromptItem 分支...
        RenameTarget::AgentTitle { workspace, tab, pane, kind } => {
            if let Some(target) = state
                .workspaces
                .get_mut(workspace)
                .and_then(|workspace| workspace.tabs.get_mut(tab))
                .and_then(|tab| tab.pane_mut(pane))
            {
                target.set_agent_title_override(kind, (!name.is_empty()).then_some(name));
            }
        }
    }
    state.overlay = None;
}
```

**关键点**：

- 其他目标继续保持“空名不保存、浮层不关闭”的既有语义。
- Agent 标题空值即清除覆盖，不需要额外提示或错误。
- 只写 `LxChat.agent_title_override` 或 `Pane.terminal_agent_title`，不触发任何 `Effect`。

### 5.5 菜单文案与标题

`src/ui/text/workspace.rs` 新增：

```rust
/// Agents 条目第二行：无标题占位。
pub const AGENT_NO_TITLE: &str = "(no title)";
/// Agents 条目右键菜单。
pub const MENU_AGENT_EDIT_TITLE: &str = "Edit title";
pub const MENU_AGENT_CLOSE: &str = "Close";
```

`src/ui/overlay/menu.rs`：

```rust
MenuCommand::EditAgentTitle => text::MENU_AGENT_EDIT_TITLE,
MenuCommand::CloseAgent => {
    if confirming {
        text::MENU_CONFIRM_CLOSE
    } else {
        text::MENU_AGENT_CLOSE
    }
}
```

菜单边框标题使用第一行的 Agent 名称：

```rust
OverlayTarget::Agent { kind, .. } => text::sidebar_agent_label(*kind).to_string(),
```

**理由**：标题可能被用户改成很长或与第二行重复；边框标题保持稳定身份名，便于识别目标。

---

## 6. 渲染设计

### 6.1 位置标签

`src/ui/text/workspace.rs`：

```rust
/// Agents 条目归属标签 `[T1 P1]`：T 为标签位置、P 为窗格树序位置（均 1 基）。
pub fn agent_location(tab_index: usize, pane_index: usize) -> String {
    format!(
        "[T{} P{}]",
        tab_index.saturating_add(1),
        pane_index.saturating_add(1)
    )
}
```

测试中的 `text::agent_location(0)` 全部改为 `text::agent_location(0, 0)` 等实际窗格序号。

### 6.2 两行条目

`agent_list_items` 的每个 `ListItem` 由两行组成：

```text
第一行：marker(2) + dot(1) + space(1) + name + padding + [T1 P1]
第二行：TREE_LEAF("  └─ ") + title + padding
```

示意：

```text
❯ ● lx-native            [T1 P1]
  └─ Fix login bug
```

- 第二行前缀复用 `text::agent::TREE_LEAF`（`"  └─ "`，5 列），`└` 正好落在第一行状态圆点列。
- 标题按**显示宽度**截断：`text::ellipsize_width(title, width - prefix_width)`，中文/emoji 不会撑破侧栏。
- 标题与前缀使用 `style::muted()`，并 `patch(highlight)` 叠加悬停背景；标题短时补空格到整行宽度，保证 hover 高亮铺满整行。
- 无标题时显示 `text::AGENT_NO_TITLE`，样式同为 muted。
- 第一行位置标签固定右对齐；名称超宽先截断名称，`[T1 P1]` 不丢。

关键代码：

```rust
fn agent_list_items<'a>(
    state: &'a AppState,
    items: &[AgentListItem<'a>],
    width: usize,
) -> Vec<ListItem<'a>> {
    // ...第一行 marker/dot/name/location...
    let mut lines = vec![Line::from(spans)];

    let title = item
        .title
        .filter(|title| !title.is_empty())
        .unwrap_or(text::AGENT_NO_TITLE);
    let prefix = text::agent::TREE_LEAF;
    let title_width = width.saturating_sub(prefix.width());
    let title = text::ellipsize_width(title, title_width);
    let padding = width.saturating_sub(prefix.width() + title.width());
    lines.push(Line::from(vec![
        Span::styled(prefix, style::muted().patch(highlight)),
        Span::styled(title, style::muted().patch(highlight)),
        Span::styled(" ".repeat(padding), Style::default().patch(highlight)),
    ]));

    rows.push(ListItem::new(lines).style(highlight));
}
```

### 6.3 两行条目的滚动与命中模型

每个条目固定占 2 行，滚动状态仍以**条目**为单位（`AppState.agent_scroll`）。

新增辅助：

```rust
/// Agents 分区可见的完整条目数；每个条目固定 2 行。
fn agent_visible_items(list: Rect) -> usize {
    (usize::from(list.height) / 2).max(1)
}

/// Agents 实际渲染区：高度向下取偶，避免底部出现半个条目。
fn agent_content_rect(list: Rect) -> Rect {
    let visible = agent_visible_items(list);
    let height = u16::try_from(visible.saturating_mul(2))
        .unwrap_or(u16::MAX)
        .min(list.height);
    Rect {
        height,
        ..list
    }
}
```

调整点：

1. `agent_list_rows(view, state)` 返回 `agent_visible_items(list)`，语义从“可见行数”改为“可见条目数”。
2. `agent_scrollbar_for` 使用 `agent_content_rect(list)` 作为轨道区域，`visible = agent_visible_items(list)`。
3. `render_agents` 在 `agent_content_rect(area)` 内渲染 `List`，底部不足一个条目的行留空。
4. `agent_item_index_at`：

```rust
let content = agent_content_rect(list);
if !content.contains((column, row).into()) {
    return None;
}
let row_offset = usize::from(row - content.y);
let slot = state.agent_scroll.saturating_add(row_offset / 2);
(slot < total).then_some(slot)
```

这样：

- 第一行和第二行都命中同一条目；
- 底部留空行不命中；
- 滚动条、滚轮、拖拽、悬停全部沿用原路径，只是 `visible` 单位从“行”变成“条目”。

**边界说明**：当 Agents 区高度为奇数时，最底部 1 行留空；这是为了避免标题被裁成半行。侧栏高度极小时仍允许渲染第一行，行为与现有窄屏退化一致。

---

## 7. 事件接入

### 7.1 右键

`src/event/overlay.rs` 的 `open_right_click_menu`，在工作区列表命中之后新增：

```rust
if let Some(index) = ui::agent_item_index_at(view, state, column, row) {
    update::open_agent_menu(state, index, (column, row));
    return true;
}
```

顺序放在 `workspace_item_at` / `workspace_list_blank_at` 之后、右栏与主区窗格之前。Agents 区与工作区列表矩形不重叠，顺序不会互相截胡。

### 7.2 左键

左键继续走 `handle_sidebar_press` → `ui::agent_target_at` → `update::focus_agent_pane_view`。由于 `agent_item_index_at` 已改成两行映射，点击第一行或第二行都能聚焦同一条目，无需新增分支。

### 7.3 悬停 / 滚轮

`event/mouse/hover.rs` 与 `event/mouse/wheel.rs` 已经统一调用 `ui::agent_item_index_at`，改一处即可让两行同时高亮、悬停与滚动后重算正确。

---

## 8. 边界与不变量

1. **绝不写 DB**：`Edit title` 只改内存；`Close` 只调用 `new_session` / `close_pane`；不新增 `Effect`，不新增迁移。
2. **绝不改会话真实标题**：`LxChat::session_title`、输入框右上角标题胶囊、`/resume` 列表保持原样。
3. **绝不改终端进程**：不向 PTY 写标题、不发信号；终端 OSC 标题只读。
4. **条目身份**：同一窗格的 lx 与终端两行必须能区分；菜单目标与覆盖都带 `SidebarAgentKind`。
5. **条目消失即覆盖失效**：
   - lx：`reset_session_view` / 会话切换清空；
   - 终端：`update_pane_agent` 在 Agent 退出或种类变化时清空。
6. **最后窗格保护**：终端 Agent 是标签页唯一窗格时不提供 `Close`；`close_pane` 自己的保护保留。
7. **二次确认**：`CloseAgent` 必须进入 `requires_confirm`；`EditAgentTitle` 不进入。
8. **空标题**：第二行始终存在，默认标题为空时显示 `(no title)`；`Edit title` 空输入清除覆盖。
9. **窄侧栏**：名称先截断，`[T1 P1]` 保留；第二行标题按显示宽度截断。
10. **渲染只读**：`ui/` 只读 `AppState`，不修改覆盖、不执行关闭。

---

## 9. 变更文件清单

```text
src/
  app/
    overlay.rs                     新增 OverlayTarget::Agent、MenuCommand::{EditAgentTitle, CloseAgent}、
                                   RenameTarget::AgentTitle、requires_confirm
    state/
      pane.rs                      SidebarAgentEntry 增加 pane_index；Pane 增加 terminal_agent_title；
                                   agent_display_title / set_agent_title_override
      lx/
        chat.rs                     LxChat 增加 agent_title_override；new / reset_session_view 初始化与清理
    state.rs                       agent_entries 枚举 pane_index
    update/
      pane.rs                      update_pane_agent 在 Agent 种类变化/退出时清除终端覆盖
      chat/session.rs              on_session_loaded_full 的 session_changed 分支清除 lx 覆盖
      menu.rs                      open_agent_menu、close_agent、activate_menu 分支
      dialog.rs                    commit_rename 支持 AgentTitle（含空值清除）
  ui/
    left_panel/mod.rs              AgentListItem 增加 pane_index/title；两行渲染；两行命中与滚动模型
    overlay/menu.rs                Agent 菜单标题与命令文案
    text/workspace.rs              agent_location 新格式、AGENT_NO_TITLE、菜单文案
  event/
    overlay.rs                     右键命中 Agents 条目并打开菜单
tests（按模块归属拆分）：
  src/app/state/pane/tests.rs      （新增文件）标题解析、覆盖生命周期、Agent 种类切换清理
  src/app/update/menu/tests.rs     （新增文件）菜单命令集合、行内确认、关闭分派、重命名提交
  src/ui/left_panel/tests.rs       两行渲染、`[T1 P1]`、悬停、命中、滚动、窄侧栏
  src/event/tests/agent_menu.rs    （新增文件）右键打开、终端唯一窗格无 Close、点击两行跳转
  src/ui/text/tests.rs             新文案与 location 格式
```

> 说明：现有 `src/app/update/tests.rs` 已超过 6000 行，本次不继续往该文件堆测试，新增测试按模块拆到 `menu/tests.rs`、`state/pane/tests.rs` 与 `event/tests/agent_menu.rs`。

---

## 10. 测试计划（审核通过后执行）

### 10.1 状态与标题

- `AgentListItem.title`：
  - lx session_title 有值 → 第二行取它；
  - lx session_title 为空 → `None`；
  - 终端 OSC title 有值 → 取它；
  - OSC title 为空字符串 → `None`。
- 内存覆盖：
  - lx 设置覆盖后优先于 session_title；
  - 清除覆盖后回落到 session_title；
  - 终端覆盖只在同 `AgentKind` 下生效；
  - `update_pane_agent` 在 `claude → codex`、`claude → None` 时清空覆盖；
  - `reset_session_view` 与 `on_session_loaded_full` 切换会话后清空 lx 覆盖；
  - 覆盖不修改 `session_title` 本身。

### 10.2 菜单

- `open_agent_menu`：
  - lx 条目命令为 `[EditAgentTitle, CloseAgent]`；
  - 终端条目的标签页有 >1 窗格时命令为 `[EditAgentTitle, CloseAgent]`；
  - 终端条目是唯一窗格时命令只有 `[EditAgentTitle]`；
  - 菜单目标包含正确的 workspace/tab/pane/kind。
- `activate_menu`：
  - `EditAgentTitle` 打开 `Overlay::Rename` 并预填当前显示标题；
  - `CloseAgent` 第一次只设置 `confirming`，第二次才执行；
  - 切换选中项后 `confirming` 复位；
  - lx `CloseAgent` 调用 `/new` 语义：`session_id=None`、消息清空、DB 效果队列中没有 `DeleteChatSession`；
  - 终端 `CloseAgent` 关闭目标窗格；
  - 菜单打开后目标消失，执行时不误关其他 Agent。
- `commit_rename`：
  - AgentTitle 输入 `my title` → 覆盖生效；
  - AgentTitle 输入空 → 清除覆盖、浮层关闭；
  - 其他 RenameTarget 输入空仍保持浮层不关闭。

### 10.3 渲染与命中

- 第一行以 `[T1 P1]` 结尾；`T`/`P` 按位置递增；
- 同一窗格两行编号相同；
- 第二行以 `"  └─ "` 开头；无标题显示 `(no title)`；
- 宽字符标题按显示宽度截断，不越界；
- 悬停时两行都有 selection 背景；
- 点击第一行/第二行都命中同一条目；
- 底部留空行不命中；
- 滚动条可见条目数按 `高度 / 2` 计算；
- 窄侧栏名称截断但 `[T1 P1]` 保留。

### 10.4 事件

- 右键 Agents 条目打开菜单，且不会错误打开工作区/Tab/Pane 菜单；
- 右键第二行同样打开菜单；
- 左键两行都聚焦对应 Pane 与目标视图；
- 滚轮滚动后悬停按新偏移重算。

### 10.5 精确验证命令

```bash
cargo fmt
cargo clippy --all-targets -- -D warnings
cargo test --lib ui::left_panel::
cargo test --lib ui::text::
cargo test --lib app::state::pane::
cargo test --lib app::update::menu::
cargo test --lib event::tests::agent_menu::
cargo test --lib event::tests::mouse_click::
```

---

## 11. 验收标准

- 侧栏 Agents 每个条目固定两行，第二行为标题或 `(no title)`。
- 第一行显示 `[T1 P1]`，T/P 均为 1 基位置，同一窗格两行一致。
- 右键条目打开菜单，包含 `Edit title`；可关闭的条目包含 `Close`。
- `Close` 需要行内二次确认，执行后只关闭、不删 DB。
- `Edit title` 复用居中重命名浮层；空输入清除覆盖。
- 所有标题修改只在内存，重启后丢失；终端/会话真实标题完全不变。
- 最后窗格保护的终端 Agent 不显示 `Close`。
- 两行条目的悬停、点击、右键、滚轮、滚动条全部正确。
- `cargo fmt`、`cargo clippy --all-targets -- -D warnings`、受影响模块测试全部通过。
