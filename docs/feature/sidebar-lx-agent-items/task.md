# 左侧侧栏 Agents 分区新增 lx 模式条目 — 执行任务清单

> 状态：**已在 `.worktrees/sidebar-lx-agent-items` 完成实现与验证，待用户确认合并**（2026-11-09）
> 设计依据：[design.md](design.md)
> 阶段划分：P0（开工门槛与基线核对）→ P1（`app` 层状态派生与视图跳转）→ P2（`ui` 与 `event` 层渲染、文案与点击接入）→ P3（单元测试与改后精确校验）。

---

## P0. 开工门槛与基线核对

- [x] 用户审核确认 [design.md](design.md) 与本 `task.md` 后开始执行。
- [x] 严格遵守 `AGENTS.md` 与 `docs/standards/*.md` 全部规范（保持 `app/` 与 `ui/` 边界清晰，生产代码禁止 `unwrap()` / `expect()`，TUI 文案 100% 英文，单文件 `< 1000` 行）。

---

## P1. `app` 层状态派生与视图跳转（`src/app/`）

- [x] **在 `Lx` 上新增会话启用与状态派生方法（`src/app/state/lx.rs`）**：
  - 新增 `pub fn has_active_session(&self) -> bool`：当 `self.chat.session_id.is_some() || self.chat.loading_session || self.chat.status.streaming || self.chat.request.is_busy() || self.chat.has_running_shell()` 时返回 `true`。
  - 新增 `pub fn agent_state(&self) -> crate::detect::AgentState`：
    - `self.has_pending_ask() || self.agents.blocked_count() > 0` 返回 `AgentState::Blocked`；
    - `self.chat.status.streaming || self.chat.request.is_busy() || self.chat.has_running_shell() || self.compact.is_running() || self.rewind.is_executing() || (!self.chat.main_stopped && self.agents.running_count() > 0)` 返回 `AgentState::Working`；
    - 其余返回 `AgentState::Idle`。
- [x] **新增 `SidebarAgentKind` 与 `SidebarAgentEntry` 并扩展 `AppState`（`src/app/state/pane.rs`、`src/app/state.rs`）**：
  - 在 `src/app/state/pane.rs` 定义 `SidebarAgentKind`（`Lx { entry: LxEntry, ai_web_site: AiWebSite }` 与 `Terminal(AgentKind)`，含 `pub const fn target_view(self) -> PaneView`）和 `SidebarAgentEntry { pub tab_index: usize, pub pane_id: PaneId, pub kind: SidebarAgentKind, pub state: AgentState }`。
  - 在 `src/app/state.rs` 重导出 `SidebarAgentKind` 与 `SidebarAgentEntry`，新增 `pub fn agent_entries(&self) -> Vec<SidebarAgentEntry>`（按 Tab 与窗格树序遍历；若 `pane.lx.has_active_session()` 先推入 `Lx` 条目，若 `pane.agent.is_some()` 再推入 `Terminal` 条目，同一窗格最多 2 行），并让 `agent_panes(&self)` 由 `agent_entries()` 映射生成。
- [x] **支持按条目目标视图聚焦切换（`src/app/update/pane.rs`）**：
  - 新增 `pub fn focus_agent_pane_view(state: &mut AppState, pane: PaneId, target_view: PaneView) -> bool`：定位所属 Tab 并切换 `active_tab`，当 `target.view != target_view` 时调用 `toggle_pane_view(state, pane)` 完成 `Lx` ↔ `Terminal` 切换及选区/面板/toast 清理，随后聚焦窗格并清理选区。
  - `focus_agent_pane(state, pane)` 委托调用 `focus_agent_pane_view(state, pane, PaneView::Terminal)`。

---

## P2. `ui` 与 `event` 层渲染、文案与点击接入（`src/ui/`、`src/event/`）

- [x] **新增 `lx-*` 侧栏名称常量与映射函数（`src/ui/text/workspace.rs`）**：
  - 新增常量 `LX_AGENT_NATIVE`（`"lx-native"`）、`LX_AGENT_HYBRID`（`"lx-hybrid"`）、`LX_AGENT_AUTOMATION`（`"lx-automation"`）、`LX_AGENT_WEB_DEEPSEEK`（`"lx-web-deepseek"`）、`LX_AGENT_WEB_GEMINI`（`"lx-web-gemini"`）、`LX_AGENT_WEB_GROK`（`"lx-web-grok"`）。
  - 新增 `pub const fn lx_agent_label(entry: LxEntry, site: AiWebSite) -> &'static str` 与 `pub fn sidebar_agent_label(kind: SidebarAgentKind) -> &'static str`。
- [x] **更新侧栏 `Agents` 列表渲染与命中（`src/ui/sidebar.rs`、`src/ui/mod.rs`）**：
  - 将 `AgentListItem` 更新为包含 `tab_index: usize`、`pane_id: PaneId`、`kind: SidebarAgentKind`、`state: AgentState`。
  - `agent_items(state)` 直接从 `state.agent_entries()` 构造。
  - 新增 `pub fn agent_target_at(view: &layout::ViewLayout, state: &AppState, column: u16, row: u16) -> Option<(PaneId, PaneView)>` 并在 `src/ui/mod.rs` 重导出；`agent_item_at` 复用 `agent_target_at` 返回 `Option<PaneId>`。
  - 在 `agent_list_items` 中：
    - 行首 `❯` 标记条件更新为 `item.tab_index == active_tab && item.pane_id == focused && focused_view == Some(item.kind.target_view())`；
    - 名称使用 `text::sidebar_agent_label(item.kind)`，样式保持 `style::strong().patch(highlight)`；
    - 状态圆点使用 `text::agent_status_dot(item.state)` 与 `style::agent_status(item.state)`。
- [x] **更新侧栏鼠标左键点击处理（`src/event/mouse/hit.rs`）**：
  - 在 `handle_sidebar_press` 中使用 `ui::agent_target_at(view, state, column, row)` 获取 `(pane, target_view)`，调用 `update::focus_agent_pane_view(state, pane, target_view)`。

---

## P3. 单元测试与改后精确校验

- [x] **补充与更新单元测试**：
  - `src/ui/sidebar/tests.rs`：
    - 更新 `add_agent` 夹具将目标窗格设为 `PaneView::Terminal`；
    - 新增测试验证 `lx` 未开 session 时不显示在 `Agents`，打开 session 后按模式显示 `lx-native` / `lx-hybrid` / `lx-automation` / `lx-web-deepseek` / `lx-web-gemini` / `lx-web-grok`；
    - 新增测试验证同一窗格同时启用 `lx` 会话与终端外部 Agent（如 `claude`）时渲染 2 行，且 `❯` 标记随 `PaneView::Lx` / `PaneView::Terminal` 切换精确落到对应行；
    - 新增测试验证 `lx-*` 状态圆点随 Idle（`○`）、Streaming/Busy（绿 `●`）、Pending Ask（黄 `●`）实时变化。
  - `src/app/update/tests.rs`：
    - 新增测试验证 `focus_agent_pane_view` 在点击 `PaneView::Lx` 条目时从终端切回 `Lx`，点击 `PaneView::Terminal` 条目时从 `Lx` 切到终端。
  - `src/event/tests/mouse_click.rs`：
    - 新增测试验证在同一窗格并存 `lx-native` 与终端 Agent 两行时，鼠标左键分别点击第 0 行和第 1 行可精确切换窗格视图与 `❯` 标记。
- [x] **执行精确校验命令（优先使用 `rtk`）**：
  - `rtk cargo test ui::sidebar::`
  - `rtk cargo test app::update::tests::`
  - `rtk cargo test event::tests::mouse_click::`
  - `rtk cargo clippy --all-targets -- -D warnings`
  - `rtk cargo fmt --check`
