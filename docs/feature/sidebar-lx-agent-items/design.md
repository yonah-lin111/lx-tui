# 左侧侧栏 Agents 分区新增 lx 模式条目 — 设计文档

> 状态：**已在 `.worktrees/sidebar-lx-agent-items` 完成实现、代码复审与 P0–P3 闭环修复，待用户确认合并**（2026-11-09）
> 规范依据：
> - [`AGENTS.md`](../../../AGENTS.md)
> - [`docs/standards/project-directory-structure.md`](../../standards/project-directory-structure.md)
> - [`docs/standards/code-writing-standards.md`](../../standards/code-writing-standards.md)
> - [`docs/standards/tui-design-requirements.md`](../../standards/tui-design-requirements.md)
> 配套文档：[task.md](task.md) · [review.md](review.md)。

---

## 0. 现状核实（基于 Codegraph 与 Codebase Memory 核查）

| 事实 | 现有代码位置 | 对本设计的影响 |
|---|---|---|
| `AppState::agent_panes(&self)` 目前仅遍历当前激活工作区各 Tab 的窗格，筛选 `pane.agent.is_some()`（即终端前台进程识别到的外部 CLI Agent），返回 `Vec<(usize, PaneId)>`，每个窗格至多产生 1 条记录 | `src/app/state.rs:756-767` | 无法收录已打开会话的 `lx` 模式条目，也不支持同一窗格在 `lx` 与终端外部 Agent 同时启用时展示 2 行 |
| `ui::sidebar::agent_items` 与 `AgentListItem` 强绑定 `PaneAgentSnapshot`（仅含 `detect::AgentKind`），行文本固定调用 `text::agent_label(snapshot.kind)` | `src/ui/sidebar.rs:505-533, 648-691`、`src/ui/text/workspace.rs:31-43` | 需将侧栏条目模型升级为统一描述“内置 `lx` 模式条目”与“终端外部 Agent 条目”的结构，并提供 `lx-*` 模式名称映射函数 |
| `ui::sidebar::agent_list_items` 中行首选中标记 `❯` 仅判断 `item.tab_index == active_tab && item.pane_id == focused`，未区分同一窗格的 `PaneView::Lx` 与 `PaneView::Terminal` | `src/ui/sidebar.rs:665-669` | 当同一窗格同时存在 `lx-*` 与终端外部 Agent 两行时，若不按 `pane.view == item.target_view()` 区分，会导致两行同时出现 `❯` 或在切到终端普通 shell 时误标 `lx-*` 行 |
| `ui::sidebar::agent_item_at` 仅返回 `Option<PaneId>`，`update::pane::focus_agent_pane` 点击后无条件将 `PaneView::Lx` 切回 `PaneView::Terminal` | `src/ui/sidebar.rs:603-611`、`src/app/update/pane.rs:911-930`、`src/event/mouse/hit.rs:64-71` | 点击 `lx-*` 条目时如果仍走旧逻辑，会错误切到终端视图；需携带目标视图 `PaneView`，点击 `lx-*` 行切到 `PaneView::Lx`，点击终端 Agent 行切到 `PaneView::Terminal` |
| `Lx` 的会话开启状态（`session_id`、`loading_session`、`request.is_busy()`、`status.streaming`、`has_running_shell()`）及运行/阻塞状态（`has_pending_ask()`、`agents.blocked_count()`、`compact.is_running()`、`rewind.is_executing()`、`agents.running_count()`）均已在 `app::state::lx` 中完备维护 | `src/app/state/lx.rs:101-170, 336-339`、`src/app/state/lx/chat.rs:423-425, 679-717`、`src/app/state/lx/compact.rs:105-107`、`src/app/state/lx/rewind.rs:256-258`、`src/app/state/lx/agent.rs:451-461` | 可直接在 `Lx` 上提供纯只读派生方法 `has_active_session(&self) -> bool` 与 `agent_state(&self) -> AgentState`，零额外状态冗余 |

---

## 1. 已确认决策汇总（Grill Me 收口）

| 编号 | 决策维度 | 确认结论 |
|---|---|---|
| D1 | **收录条件（何时显示 `lx-*` 条目）** | **仅当窗格的 `lx` 已打开会话（session 启用）时才加入左侧 `Agents` 列表**。<br>1. 空白欢迎页（新建窗格尚未发消息、执行 `/new` 重置后、删除当前会话后、或切换不兼容模式/站点清空会话后）不显示在 `Agents` 列表中；<br>2. 一旦打开或建立会话（`chat.session_id.is_some()`、`/resume` 正在加载 `chat.loading_session`、首轮发送在途 `chat.request.is_busy()` / `chat.status.streaming`、或本地 `!` 命令在途 `chat.has_running_shell()`），立即在左侧 `Agents` 列表显示对应模式的 `lx-*` 条目，即使切到 `[>_]` 终端视图只要后台会话未关闭也继续保留。 |
| D2 | **单窗格双行并存与排序** | **同一窗格支持最多显示 2 条独立可点击的 item（占 2 行）**：<br>1. 如果窗格的 `lx` 会话已启用，生成 1 条 `lx-*` 行（目标视图 `PaneView::Lx`）；<br>2. 如果该窗格后台终端同时探测到外部 CLI Agent（`pane.agent.is_some()`，如 `claude`、`codex`），再生成 1 条外部 Agent 行（目标视图 `PaneView::Terminal`）；<br>3. 列表整体按 `(tab_index, layout.pane_ids() 树序)` 排列；同一窗格同时有两行时，固定先排 `lx-*` 行、紧接着排终端外部 Agent 行，两行右侧均显示 `[tab N]`。 |
| D3 | **各模式的名称文案** | **统一使用全小写 `lx-<模式>`，其中 `Web` 模式携带当前选中的具体站点后缀**：<br>- `LxEntry::Native` ➔ `lx-native`<br>- `LxEntry::Hybrid` ➔ `lx-hybrid`<br>- `LxEntry::Automation` ➔ `lx-automation`<br>- `LxEntry::AiWeb` + `AiWebSite::Deepseek` ➔ `lx-web-deepseek`<br>- `LxEntry::AiWeb` + `AiWebSite::Gemini` ➔ `lx-web-gemini`<br>- `LxEntry::AiWeb` + `AiWebSite::Grok` ➔ `lx-web-grok` |
| D4 | **状态圆点（`○` / 绿 `●` / 黄 `●`）映射规则** | 对齐外部 Agent 的三态语义（阻塞优先于运行，其余为空闲）：<br>1. **阻塞 `●`（黄实心，`AgentState::Blocked`）**：`lx.has_pending_ask()`（`ask` 提问面板挂起等待用户作答）或 `lx.agents.blocked_count() > 0`（子 agent 队首输入阻塞）；<br>2. **运行中 `●`（绿实心，`AgentState::Working`）**：`lx.chat.status.streaming`、`lx.chat.request.is_busy()`、`lx.chat.has_running_shell()`、`lx.compact.is_running()`、`lx.rewind.is_executing()`，或 `!lx.chat.main_stopped && lx.agents.running_count() > 0`；<br>3. **空闲 `○`（空心灰，`AgentState::Idle`）**：其余会话待命状态。 |
| D5 | **行首 `❯` 标记、文字配色与点击跳转** | 1. **`❯` 标记**：严格匹配“当前激活 Tab 的焦点窗格 + 当前可见视图”（`item.tab_index == active_tab && item.pane_id == focused && pane.view == item.target_view()`），同一时刻至多一行带 `❯`；<br>2. **文字样式**：`lx-*` 名称与现有 `claude` / `codex` 一致，统一使用 `style::strong()`（亮白加粗），不作渐变染色；<br>3. **左键点击行为**：点击 `lx-*` 行切换到所属 Tab、聚焦该窗格并切到 `PaneView::Lx`；点击外部 Agent 行切换到所属 Tab、聚焦该窗格并切到 `PaneView::Terminal`。 |

---

## 2. 模块结构与变更文件清单

所有改动严格保持 `app/`（纯状态与更新）、`event/`（事件分派）与 `ui/`（纯只读渲染）边界清晰，不触碰数据库与外部 IO，且各文件行数均控制在规范上限内：

```text
src/
  app/
    state/
      lx.rs                     Lx 新增 has_active_session(&self) -> bool 与 agent_state(&self) -> AgentState
      pane.rs                   新增 SidebarAgentKind 与 SidebarAgentEntry 类型定义（描述侧栏 Agents 条目的种类、目标视图与状态）
    state.rs                    重导出 SidebarAgentKind / SidebarAgentEntry；扩展 agent_entries(&self) -> Vec<SidebarAgentEntry> 并同步更新 agent_panes(&self)
    update/
      pane.rs                   新增 focus_agent_pane_view(state, pane, view) 支撑按条目目标视图切换（Lx ↔ Terminal），focus_agent_pane 保持兼容
      workspace_list.rs         agent_scroll_max 使用 agent_entries().len()（与 agent_panes().len() 保持一致）
      tests.rs                  新增/更新 agent_panes 与 focus_agent_pane_view 针对 lx 会话与双行并存的单元测试
  event/
    mouse/
      hit.rs                    handle_sidebar_press 调用 ui::agent_target_at + update::focus_agent_pane_view
    tests/
      mouse_click.rs            新增鼠标点击侧栏 lx-* 条目与同窗格终端 Agent 条目互相切换视图的集成测试
  ui/
    mod.rs                      重导出 sidebar::agent_target_at
    sidebar.rs                  AgentListItem 升级为承载 SidebarAgentKind；更新 agent_items、agent_scrollbar_for、agent_item_index_at、agent_target_at、agent_list_items
    sidebar/
      tests.rs                  更新 add_agent 夹具并补充 lx-* 各模式渲染、双行并存、状态圆点联动、❯ 焦点视图跟随与点击命中单测
    text/
      workspace.rs              新增 LX_AGENT_* 常量与 lx_agent_label(entry, site) / sidebar_agent_label(kind) 映射函数
```

---

## 3. 核心设计详解与关键代码

### 3.1 `Lx` 会话启用判定与运行状态投影（`src/app/state/lx.rs`）

在 `src/app/state/lx.rs` 的 `impl Lx` 中新增两个纯只读方法：

```rust
    /// 是否已打开或正在建立 lx 会话（侧栏 `Agents` 列表收录条件）。
    ///
    /// 空白欢迎页（尚未发送消息、`/new` 重置后、跨模式/站点重置后或会话删除后）返回 `false`；
    /// 一旦已绑定 `session_id`、正在加载历史会话（`/resume`）、首次提交请求在途（`Committing`），
    /// 或有本地 `!` 命令在途，即视为会话已打开。
    pub fn has_active_session(&self) -> bool {
        self.chat.session_id.is_some()
            || self.chat.loading_session
            || matches!(
                self.chat.request,
                crate::app::state::lx::chat::RequestState::Committing { .. }
            )
            || self.chat.has_running_shell()
    }

    /// 映射当前 lx 会话的运行状态（供侧栏 `Agents` 圆点着色：阻塞优先于运行，其余为空闲）。
    pub fn agent_state(&self) -> crate::detect::AgentState {
        use crate::detect::AgentState;
        let request_busy = self
            .chat
            .request
            .is_busy_for_session(self.chat.session_id.as_deref());
        let streaming = match &self.chat.request {
            crate::app::state::lx::chat::RequestState::Streaming { key, .. }
            | crate::app::state::lx::chat::RequestState::Finishing { key, .. } => {
                self.chat.session_id.as_deref() == Some(key.session.as_str())
            }
            _ => self.chat.status.streaming,
        };
        if self.has_pending_ask() || self.agents.blocked_count() > 0 {
            AgentState::Blocked
        } else if streaming
            || request_busy
            || self.chat.loading_session
            || self.chat.has_running_shell()
            || self.compact.is_running()
            || self.rewind.is_executing()
            || (!self.chat.main_stopped && self.agents.running_count() > 0)
        {
            AgentState::Working
        } else {
            AgentState::Idle
        }
    }
```

**关键点说明**：
- **生命周期闭环**：
  - 用户在 `Native` 或 `Web` 模式下按 `Enter` 发送首条消息时，`try_send` / `try_send_ai_web` 会立即将 `request` 置为 `Committing` / `Streaming`（或绑定 `session_id`），`has_active_session()` 立即变为 `true`，同时 `agent_state()` 返回 `AgentState::Working`（绿点 `●`）。
  - 本轮回答完成后，`session_id` 保持为 `Some(...)`，`has_active_session()` 仍为 `true`，`agent_state()` 回落为 `AgentState::Idle`（空心 `○`）。
  - 用户通过 `/resume` 恢复历史会话时，`resume_session` 先置 `loading_session = true`，加载完成后 `on_session_loaded_full` 绑定 `session_id = Some(...)`，全程保持在 `Agents` 列表中。
  - 用户执行 `/new`（`new_session`）、删除当前会话（`on_session_deleted`）、或在 `Native` 与 `Web` 之间/不同 `Web` 站点之间切换导致 `reset_session_view()` 时，`session_id` 清为 `None` 且无在途请求，`has_active_session()` 立即变为 `false`，条目自动从侧栏移除。

---

### 3.2 侧栏 Agent 条目模型与枚举收集（`src/app/state/pane.rs` & `src/app/state.rs`）

在 `src/app/state/pane.rs` 中定义 `SidebarAgentKind` 与 `SidebarAgentEntry`：

```rust
use super::lx::{AiWebSite, LxEntry};
use crate::detect::{AgentKind, AgentState, PaneAgentSnapshot};

/// 侧栏 `Agents` 分区条目的身份类型。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SidebarAgentKind {
    /// 窗格内置 lx 会话（携带当前入口模式与 AI Web 站点）。
    Lx {
        entry: LxEntry,
        ai_web_site: AiWebSite,
    },
    /// 窗格终端前台进程探测到的外部 CLI Agent。
    Terminal(AgentKind),
}

impl SidebarAgentKind {
    /// 该条目对应的窗格主内容视图（供行首 `❯` 标记比对与左键点击跳转使用）。
    pub const fn target_view(self) -> PaneView {
        match self {
            Self::Lx { .. } => PaneView::Lx,
            Self::Terminal(_) => PaneView::Terminal,
        }
    }
}

/// 侧栏 `Agents` 分区的单行条目数据。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SidebarAgentEntry {
    pub tab_index: usize,
    pub pane_id: PaneId,
    pub kind: SidebarAgentKind,
    pub state: AgentState,
}
```

在 `src/app/state.rs` 的 `impl AppState` 中，提供 `agent_entries(&self) -> Vec<SidebarAgentEntry>`，并让 `agent_panes(&self) -> Vec<(usize, PaneId)>` 由其派生以保持现有调用方兼容：

```rust
    /// 当前工作区内所有启用的 Agent 条目（按标签与窗格树序排列；同一窗格先排 lx 会话、再排终端外部 Agent）。
    ///
    /// Agents 分区渲染、命中与滚动计数共用唯一数据源。
    pub fn agent_entries(&self) -> Vec<SidebarAgentEntry> {
        let workspace = self.active_workspace();
        let mut entries = Vec::new();
        for (tab_index, tab) in workspace.tabs.iter().enumerate() {
            for pane_id in tab.layout.pane_ids() {
                let Some(pane) = tab.pane(pane_id) else {
                    continue;
                };
                if pane.lx.has_active_session() {
                    entries.push(SidebarAgentEntry {
                        tab_index,
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
                        pane_id,
                        kind: SidebarAgentKind::Terminal(snapshot.kind),
                        state: snapshot.state,
                    });
                }
            }
        }
        entries
    }

    /// 当前工作区内 Agents 分区各行对应的（标签索引，窗格标识），与 [`Self::agent_entries`] 一一对应。
    pub fn agent_panes(&self) -> Vec<(usize, PaneId)> {
        self.agent_entries()
            .into_iter()
            .map(|entry| (entry.tab_index, entry.pane_id))
            .collect()
    }
```

**关键点说明**：
- 当一个窗格的 `lx` 打开了 session 且终端里也运行了 `claude` 时，`agent_entries()` 会为该窗格按序生成 2 条记录：第 1 条是 `SidebarAgentKind::Lx { .. }`，第 2 条是 `SidebarAgentKind::Terminal(AgentKind::Claude)`。
- `agent_panes().len()` 与 `agent_entries().len()` 恒等，因此 `workspace_list::agent_scroll_max`、滚动条 `total` 计数与滚轮偏移天然支持单窗格 2 行！

---

### 3.3 名称文案映射（`src/ui/text/workspace.rs`）

在 `src/ui/text/workspace.rs` 中新增 `lx-*` 模式标签常量与统一映射函数：

```rust
use crate::app::state::{AiWebSite, LxEntry, SidebarAgentKind};

/// 侧栏 Agents 分区中 lx 各模式的稳定小写名称。
pub const LX_AGENT_NATIVE: &str = "lx-native";
pub const LX_AGENT_HYBRID: &str = "lx-hybrid";
pub const LX_AGENT_AUTOMATION: &str = "lx-automation";
pub const LX_AGENT_WEB_DEEPSEEK: &str = "lx-web-deepseek";
pub const LX_AGENT_WEB_GEMINI: &str = "lx-web-gemini";
pub const LX_AGENT_WEB_GROK: &str = "lx-web-grok";

/// lx 入口模式与 AI Web 站点对应的侧栏 Agent 名称标签。
pub const fn lx_agent_label(entry: LxEntry, site: AiWebSite) -> &'static str {
    match entry {
        LxEntry::Native => LX_AGENT_NATIVE,
        LxEntry::Hybrid => LX_AGENT_HYBRID,
        LxEntry::Automation => LX_AGENT_AUTOMATION,
        LxEntry::AiWeb => match site {
            AiWebSite::Deepseek => LX_AGENT_WEB_DEEPSEEK,
            AiWebSite::Gemini => LX_AGENT_WEB_GEMINI,
            AiWebSite::Grok => LX_AGENT_WEB_GROK,
        },
    }
}

/// 侧栏 Agents 条目的显示名称（统一全小写英文）。
pub const fn sidebar_agent_label(kind: SidebarAgentKind) -> &'static str {
    match kind {
        SidebarAgentKind::Lx { entry, ai_web_site } => lx_agent_label(entry, ai_web_site),
        SidebarAgentKind::Terminal(agent) => agent_label(agent),
    }
}
```

**关键点说明**：
- 终端窗格右上角边框的 `agent:<name>` 标签（`src/ui/panes.rs:render_pane_agent_label`）继续调用原有的 `text::agent_label(agent.kind)`，不受影响。
- 左侧 `Agents` 列表调用 `text::sidebar_agent_label(item.kind)`，实现 `lx-native`、`lx-hybrid`、`lx-automation`、`lx-web-deepseek`、`lx-web-gemini`、`lx-web-grok` 与外部 Agent 名称的统一渲染。

---

### 3.4 侧栏渲染、焦点标记 `❯` 与命中检测（`src/ui/sidebar.rs` & `src/ui/mod.rs`）

在 `src/ui/sidebar.rs` 中更新 `AgentListItem`、`agent_items`、命中函数与 `agent_list_items`：

```rust
use crate::app::state::{AppState, FocusedContainer, PaneView, SidebarAgentKind, WorkspaceMode};

/// Agents 分区条目：标签索引、窗格标识、Agent 种类与运行状态；渲染与命中一一对应。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AgentListItem {
    pub tab_index: usize,
    pub pane_id: PaneId,
    pub kind: SidebarAgentKind,
    pub state: crate::detect::AgentState,
}

/// 当前激活工作区的 Agents 条目（按标签与树序，同一窗格先 lx 后终端 Agent）。
pub fn agent_items(state: &AppState) -> Vec<AgentListItem> {
    state
        .agent_entries()
        .into_iter()
        .map(|entry| AgentListItem {
            tab_index: entry.tab_index,
            pane_id: entry.pane_id,
            kind: entry.kind,
            state: entry.state,
        })
        .collect()
}

/// Agents 条目命中：返回被点窗格标识及目标视图 `(PaneId, PaneView)`；未命中返回 None。
pub fn agent_target_at(
    view: &layout::ViewLayout,
    state: &AppState,
    column: u16,
    row: u16,
) -> Option<(PaneId, PaneView)> {
    let index = agent_item_index_at(view, state, column, row)?;
    state
        .agent_entries()
        .get(index)
        .map(|entry| (entry.pane_id, entry.kind.target_view()))
}

/// Agents 条目命中：返回被点窗格标识；未命中返回 None。
pub fn agent_item_at(
    view: &layout::ViewLayout,
    state: &AppState,
    column: u16,
    row: u16,
) -> Option<PaneId> {
    agent_target_at(view, state, column, row).map(|(pane, _)| pane)
}
```

在 `agent_list_items` 中，行首 `❯` 标记严格比对 `item.tab_index == active_tab && item.pane_id == focused && focused_view == Some(item.kind.target_view())`：

```rust
fn agent_list_items<'a>(
    state: &'a AppState,
    items: &[AgentListItem],
    width: usize,
) -> Vec<ListItem<'a>> {
    let active_tab = state.active_workspace().active_tab;
    let focused = state.active_tab().layout.focus();
    let focused_view = state.active_tab().pane(focused).map(|pane| pane.view);
    let mut rows = Vec::with_capacity(items.len());
    for (slot, item) in items.iter().enumerate() {
        let highlight = if state.agent_hover == Some(slot) {
            style::selection()
        } else {
            Style::default()
        };
        let is_active_item = item.tab_index == active_tab
            && item.pane_id == focused
            && focused_view == Some(item.kind.target_view());
        let marker = if is_active_item {
            format!("{} ", text::AGENT_SELECTED_MARKER)
        } else {
            "  ".to_string()
        };
        let dot = format!("{} ", text::agent_status_dot(item.state));
        let location = text::agent_location(item.tab_index);
        let fixed = marker.chars().count() + dot.chars().count() + location.chars().count();
        let name = text::ellipsize(
            text::sidebar_agent_label(item.kind),
            width.saturating_sub(fixed + 1),
        );
        let padding = width.saturating_sub(fixed + name.chars().count());
        let marker_style = style::strong().patch(highlight);
        // 状态圆点特殊色叠在行底之上：hover 只提供背景，不覆盖运行/阻塞状态色。
        let dot_style = highlight.patch(style::agent_status(item.state));
        let spans = vec![
            Span::styled(marker, marker_style),
            Span::styled(dot, dot_style),
            Span::styled(name, style::strong().patch(highlight)),
            Span::styled(" ".repeat(padding), Style::default().patch(highlight)),
            Span::styled(location, style::muted().patch(highlight)),
        ];
        rows.push(ListItem::new(Line::from(spans)).style(highlight));
    }
    rows
}
```

---

### 3.5 点击条目按目标视图跳转（`src/app/update/pane.rs` & `src/event/mouse/hit.rs`）

在 `src/app/update/pane.rs` 中新增 `focus_agent_pane_view`：

```rust
/// 点击 Agents 条目：切到所属标签、聚焦窗格、将窗格主视图切至条目对应的 `target_view`（`Lx` 或 `Terminal`）、收回其它容器焦点；返回是否成功。
pub fn focus_agent_pane_view(state: &mut AppState, pane: PaneId, target_view: PaneView) -> bool {
    let Some(tab_index) = state
        .active_workspace()
        .tabs
        .iter()
        .position(|tab| tab.pane(pane).is_some())
    else {
        return false;
    };
    state.active_workspace_mut().active_tab = tab_index;
    if state
        .active_tab()
        .pane(pane)
        .is_some_and(|target| target.view != target_view)
    {
        toggle_pane_view(state, pane);
    }
    super::focus::focus_pane(state, pane);
    clear_selection(state);
    true
}

/// 点击终端 Agent 条目：切到所属标签、聚焦窗格、切回终端视图并收回其它容器焦点；返回是否成功。
pub fn focus_agent_pane(state: &mut AppState, pane: PaneId) -> bool {
    focus_agent_pane_view(state, pane, PaneView::Terminal)
}
```

在 `src/event/mouse/hit.rs` 的 `handle_sidebar_press` 中改用 `ui::agent_target_at` 与 `update::focus_agent_pane_view`：

```rust
pub(crate) fn handle_sidebar_press(
    state: &mut AppState,
    view: &ui::layout::ViewLayout,
    column: u16,
    row: u16,
    dirty: &mut bool,
) -> bool {
    if let Some((pane, target_view)) = ui::agent_target_at(view, state, column, row) {
        if update::focus_agent_pane_view(state, pane, target_view) {
            ensure_workspace_visible(state, view);
            reveal_active_tab(state, view);
            *dirty = true;
        }
        return true;
    }
    // ...
```

**关键点说明**：
- 当点击 `lx-*` 条目时，`target_view` 为 `PaneView::Lx`：若目标窗格原先处于 `PaneView::Terminal`，会调用 `toggle_pane_view(state, pane)` 切回 `PaneView::Lx`（同时自动清理终端选区、拖拽状态、悬空 toast，并同步提及面板与块命令面板）。
- 当点击外部 CLI Agent（如 `claude`）条目时，`target_view` 为 `PaneView::Terminal`：若目标窗格原先处于 `PaneView::Lx`，会调用 `toggle_pane_view(state, pane)` 切到 `PaneView::Terminal`（同时取消在途粘贴面板与提及面板、清除 `pane.lx.toast`）。

---

## 4. 边界条件与回归保障

1. **空白欢迎页不产生冗余条目**：
   - `AppState::demo()` 初始状态或新建 Tab/分屏后，`pane.lx.has_active_session()` 为 `false`，若未挂载外部 Agent，侧栏 `Agents` 仍渲染 `no agents running` 空态。
2. **现有侧栏测试夹具兼容**：
   - `src/ui/sidebar/tests.rs` 中的 `add_agent` 夹具原本在 `AppState::demo()`（默认 `PaneView::Lx`）上直接挂载终端 `PaneAgentSnapshot`。为匹配真实运行态（终端 Agent 运行于 `PaneView::Terminal`）及 D5 的视图严格对应规则，在 `add_agent` 夹具中同步将目标窗格 `pane.view` 设为 `PaneView::Terminal`，确保所有既有终端 Agent 单测语义严密。
3. **窄侧栏截断保障**：
   - 最长名称 `lx-web-deepseek`（15 字符）在侧栏宽度收窄时，由 `text::ellipsize` 优先截断名称末尾并保留右侧 `[tab N]` 归属标签不丢失。
4. **会话关闭/重置即时清理**：
   - `/new`、删除会话、跨 `Native`/`AiWeb` 切换或切换 `AiWebSite` 触发 `reset_session_view()` 后，`lx-*` 条目立即从 `agent_entries()` 消失，且每帧末尾 `event/mod.rs` 的 `clamp_agent_scroll` 自动夹取滚动偏移并清理越界 `agent_hover` 与 `agent_scroll_drag`。

---

## 5. 复审修复摘要（对应 `review.md` P0 → P3）

- **P0-1（流中 `/new` / 删除会话 / 切换会话的请求归属隔离）**：在 `RequestState` 新增 `is_busy_for_session(session_id)`，并在 `reset_session_view()` 复位 `status.streaming = false` 及未建流的 `Committing` 状态；`has_active_session()` 与 `agent_state()` 严格按当前 `session_id` 判定在途请求，确保流中 `/new` 后侧栏条目立即消失且旧请求不污染新会话。
- **P1-1（`/resume` 失败复位、加载态圆点与跨模式清理）**：`load_chat_session_into_pane` 失败分支复位 `loading_session = false`；`agent_state()` 将 `loading_session` 纳入 `Working`；`on_ai_web_session_loaded` 同步 `entry = LxEntry::AiWeb`；`set_lx_entry`、`set_ai_web_site` 与 `on_rewind_finished`（`session_deleted`）统一补齐 `clear_agents()`、`compact.reset()`、`model_switch.reset()`。
- **P1-2（高频几何与悬停查询免分配优化）**：在 `AppState` 新增 `agent_entry_count(&self) -> usize`，使 `agent_scroll_max`、`clamp_agent_scroll`、`agent_scrollbar`、`agent_item_index_at` 零分配且免去 `agent_state()` 全消息遍历。
- **P2-1（列表收缩与容器关闭时的悬停/拖拽态清理）**：`clamp_agent_scroll` 在滚动回退或槽位越界时清空 `agent_hover`、在无需滚动条时清空 `agent_scroll_drag`；`close_workspace`、`close_tab`、`close_pane` 同步复位侧栏 `Agents` 悬停与拖拽态。
- **P3-1（旧兼容接口测试门控与 `const fn` 收敛）**：将 `ui::agent_item_at` 与 `update::focus_agent_pane` 标记为 `#[cfg(test)]`，并将 `agent_label` 与 `sidebar_agent_label` 声明为 `pub const fn`。
