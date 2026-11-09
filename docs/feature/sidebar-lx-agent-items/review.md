# 左侧栏 `Agents` 分区展示 `lx` 模式条目 — 代码评审报告 (`review.md`)

- **评审范围**：`feat/sidebar-lx-agent-items` 分支（worktree：`.worktrees/sidebar-lx-agent-items`），初版实现提交 `476f2fc`（对比基准：`dev` @ `42e70535`）
- **特性目录**：`docs/feature/sidebar-lx-agent-items/`
- **合并状态**：已在 `.worktrees/sidebar-lx-agent-items` 完成 P0–P3 全部问题修复与回归测试，待用户确认后合并回 `dev`
- **关联文档**：[design.md](design.md) · [task.md](task.md)
- **评审方式**：使用 `codegraph` 与 `codebase-memory-mcp` 检索调用链与爆炸半径，并委派并行子 Agent 分别对 `Lx` 会话生命周期状态机、侧栏渲染/命中/滚动钳制与跨容器交互进行深度源码审计
- **结论**：共发现 **5 项问题（P0 1 项、P1 2 项、P2 1 项、P3 1 项）**，已全部闭环修复并通过回归测试。

## 优先级定义

| 级别 | 判定标准 |
| --- | --- |
| **P0** | 违反核心设计不变量（D1/D4），导致流中重置/切换会话后侧栏 `lx-*` 条目残留或将空闲会话错误标为 `Working` |
| **P1** | 特定异常或跨模式路径下产生永久幽灵条目、模式标签失真，或在主事件循环/悬停路径引入高频 `O(messages)` 重复扫描的性能回归 |
| **P2** | 列表收缩或关闭工作区/标签/窗格时，侧栏 `Agents` 悬停高亮或滚动条拖拽态残留 |
| **P3** | API 边界防呆与可维护性改进（如未加 `#[cfg(test)]` 门控的兼容旧接口、`const fn` 收敛） |

---

## 问题分级总览（按 P0 → P3 排序）

| 级别 | 编号 | 模块 / 文件 | 关键点摘要 | 状态 |
| --- | --- | --- | --- | --- |
| **P0** | **P0-1** | `src/app/state/lx.rs`、`src/app/state/lx/chat.rs` | 流式生成或提交中执行 `/new`、删除会话、跨模式/站点重置或切换会话后，旧会话后台收尾请求导致 `lx-*` 条目未立即移除或将新会话误标为 `Working` | 已修复 |
| **P1** | **P1-1** | `src/db/service/chat.rs`、`src/app/state/lx.rs`、`src/app/update/chat/web.rs`、`src/app/update/pane.rs`、`src/event/chat/rewind.rs` | `/resume` 加载失败未复位 `loading_session` 导致侧栏永久残留幽灵条目；加载期间圆点误显 `Idle`；云端会话恢复与跨模式重置存在状态清理缺口 | 已修复 |
| **P1** | **P1-2** | `src/app/state.rs`、`src/app/update/workspace_list.rs`、`src/ui/sidebar.rs` | 滚动钳制、滚动条计算与鼠标悬停每次均构造全量 `Vec<SidebarAgentEntry>` 并触发 `agent_state()` 全消息扫描 | 已修复 |
| **P2** | **P2-1** | `src/app/update/workspace_list.rs`、`src/app/update/workspace.rs` | `Agents` 条目数缩减或关闭激活工作区/标签/窗格时，未清理越界的 `agent_hover` 与失效的 `agent_scroll_drag` | 已修复 |
| **P3** | **P3-1** | `src/ui/sidebar.rs`、`src/ui/mod.rs`、`src/app/update/pane.rs`、`src/ui/text/workspace.rs` | 丢弃目标视图的旧接口 `agent_item_at` 与强制切终端的 `focus_agent_pane` 仍对生产代码公开暴露；标签映射函数可收敛为 `const fn` | 已修复 |

---

## 详细问题分析与错误原因

### P0-1：流式生成或提交中执行 `/new`、删除会话或切换会话后，`lx-*` 条目未立即消失且旧请求污染当前状态

- **涉及位置**：
  - `src/app/state/lx.rs:346-370`（`Lx::has_active_session`、`Lx::agent_state`）
  - `src/app/state/lx/chat.rs:296-311`、`679-706`（`RequestState`、`LxChat::reset_session_view`）
- **关键点**：
  - 设计文档 D1 明确要求：当执行 `/new` 清空重置、会话被删除、或切换入口/站点触发 `reset_session_view()` 解绑会话后，对应 `lx-*` 条目必须**立即**从侧栏 `Agents` 分区移除。
  - 初版实现中，若 `lx` 正在流式生成（`RequestState::Streaming`）时执行 `/new`（`new_session`）、删除会话（`on_session_deleted`）、切换 AI Web 站点（`set_ai_web_site`）或跨 AI Web 模式切换（`set_lx_entry`），主视图已回到空白欢迎页，但侧栏 `Agents` 仍继续显示 `lx-*` 条目且圆点呈绿色 `Working`，直到后台取消流与 DB 落库收尾彻底完成才消失。
  - 同理，若旧会话后台取消尚未完成时窗格切换到另一个空闲会话 `sess-2`，旧会话的 `RequestState::Streaming` / `Finishing` 会把 `sess-2` 的侧栏圆点错误渲染为绿色 `Working`。
- **错误原因**：
  1. `new_session` 等重置路径调用 `stop_stream` 仅入队 `Effect::CancelChatStream` / `CancelChatCommit`；随后 `reset_session_view()` 只清空 `self.session_id = None` 与 `self.messages`，刻意保留 `self.request` 为 `Streaming { key, .. }` / `Finishing { key, .. }` 以供后台按原 `key.session` 完成中断落库，同时漏掉了 `self.status.streaming = false` 以及对尚未建流的 `RequestState::Committing` 的复位。
  2. 初版 `Lx::has_active_session()` 直接判断 `self.chat.status.streaming || self.chat.request.is_busy()`，未区分“首次建流前的 `Committing`”与“已解绑旧会话的 `Streaming` / `Finishing` 后台收尾”；`Lx::agent_state()` 同样未校验 `self.chat.request` 中的 `key.session` 是否等于当前 `self.chat.session_id`。
- **修复措施**：
  1. 在 `RequestState`（`src/app/state/lx/chat.rs`）上新增 `pub fn is_busy_for_session(&self, session_id: Option<&str>) -> bool`：仅当处于 `Committing`，或 `Streaming` / `Finishing` 的 `key.session` 与当前 `session_id` 一致时返回 `true`。
  2. 在 `LxChat::reset_session_view()` 中复位 `self.status.streaming = false`，且当 `self.request` 处于尚未建流的 `Committing` 阶段时将其直接置回 `RequestState::Idle`。
  3. 将 `Lx::has_active_session()` 收敛为 `self.chat.session_id.is_some() || self.chat.loading_session || matches!(self.chat.request, RequestState::Committing { .. }) || self.chat.has_running_shell()`，并在 `Lx::agent_state()` 中通过 `is_busy_for_session(self.chat.session_id.as_deref())` 与会话归属比对判定运行态。
- **验证**：
  - `src/ui/sidebar/tests.rs::slash_new_and_site_switch_while_streaming_immediately_remove_lx_agent_row`

---

### P1-1：`/resume` 加载失败遗留永久幽灵条目、加载中状态圆点误显 `Idle` 及跨模式状态清理缺口

- **涉及位置**：
  - `src/db/service/chat.rs:409-424`（`Persistence::load_chat_session_into_pane`）
  - `src/app/state/lx.rs:355-378`（`Lx::agent_state`）
  - `src/app/update/chat/web.rs:436-439`（`on_ai_web_session_loaded`）
  - `src/app/update/pane.rs:716-723`、`752-760`（`set_lx_entry`、`set_ai_web_site`）
  - `src/event/chat/rewind.rs:210-216`（`on_rewind_finished`）
- **关键点**：
  1. 当 `/resume` 选中的历史会话因外部删除或数据库读取失败而无法加载时，`load_chat_session_into_pane` 弹出错误 Toast 后返回 `true`，导致 `target.lx.chat.loading_session` 永远停留在 `true`，侧栏 `Agents` 永久残留一条无法消失的幽灵 `lx-*` 条目。
  2. `Lx::has_active_session()` 将 `loading_session == true` 视为会话已打开，但初版 `Lx::agent_state()` 未将 `loading_session` 纳入 `Working` 分支，导致 `/resume` 加载期间侧栏圆点显示为空心 `○`（Idle）而非实心绿点 `●`（Working）。
  3. `on_ai_web_session_loaded` 在加载 `ai-web-*` 会话时仅设置了 `ai_web_site` 而未同步设置 `entry = LxEntry::AiWeb`；同时 `set_lx_entry`、`set_ai_web_site` 与 `on_rewind_finished`（`session_deleted` 分支）在重置会话时漏清 `clear_agents()`、`compact.reset()`、`model_switch.reset()`，旧子 Agent 或压缩状态可能残留并污染侧栏圆点。
- **错误原因**：
  1. `Effect::LoadChatSession`（`src/db/service/chat.rs:584`）仅在 `!loaded` 时复位 `loading_session = false`，而 `load_chat_session_into_pane` 的失败分支因为向窗格写入了错误 Toast 返回了 `true`（通知调用方重绘），遗漏了在失败分支内部复位 `loading_session = false`。
  2. `Lx::agent_state()` 的 `Working` 条件漏掉了 `self.chat.loading_session`。
  3. `set_lx_entry`、`set_ai_web_site` 与 `on_rewind_finished` 的会话解绑清理未与 `new_session` / `on_session_deleted` 保持同一套复位口径。
- **修复措施**：
  1. 在 `load_chat_session_into_pane` 失败分支（`src/db/service/chat.rs:412`）显式设置 `target.lx.chat.loading_session = false`。
  2. 在 `Lx::agent_state()` 的 `Working` 分支加入 `self.chat.loading_session`。
  3. 在 `on_ai_web_session_loaded` 中同步设置 `target.lx.entry = LxEntry::AiWeb`；在 `set_lx_entry`（不兼容解绑分支）、`set_ai_web_site` 与 `on_rewind_finished`（`session_deleted` 分支）中补齐 `clear_agents()`、`compact.reset()` 与 `model_switch.reset()`。
- **验证**：
  - `src/db/service/resume_tests.rs::resume_missing_session_clears_loading_flag_and_removes_sidebar_entry`
  - `src/ui/sidebar/tests.rs::lx_agent_dot_tracks_idle_working_and_blocked_states`
  - `src/ui/sidebar/tests.rs::slash_new_and_site_switch_while_streaming_immediately_remove_lx_agent_row`

---

### P1-2：侧栏滚动钳制、滚动条计算与鼠标悬停高频路径重复构造 `Vec<SidebarAgentEntry>` 并全量扫描消息

- **涉及位置**：
  - `src/app/state.rs:757-814`（`AppState::agent_entries`、`AppState::agent_entry_count`、`AppState::agent_panes`）
  - `src/app/update/workspace_list.rs:33-35`、`60-67`（`agent_scroll_max`、`clamp_agent_scroll`）
  - `src/ui/sidebar.rs:538-608`、`620-640`（`agent_scrollbar`、`agent_scrollbar_for`、`agent_item_index_at`、`render_agents`）
- **关键点**：
  - 主事件循环每轮 `clamp_agent_scroll`、侧栏渲染 `agent_scrollbar_for`、鼠标移动 `agent_item_index_at`（单次悬停调用 2 次）均只需知道“当前工作区的 Agent 条目总数”，却全部调用了 `state.agent_panes().len()`。
  - 初版实现将 `agent_panes()` 改为由 `self.agent_entries()` 映射生成，而 `agent_entries()` 会为每个活跃 `lx` 窗格计算 `pane.lx.agent_state()`；当会话处于 Idle 时，`agent_state()` 会执行 `self.chat.has_running_shell()` 遍历 `self.chat.messages` 全部历史消息。在长对话且侧栏展开时，每次终端 PTY 输出或鼠标移动都会触发多次 `O(messages)` 全量遍历与堆分配。
- **错误原因**：
  - 未区分“仅需条目计数的几何/边界查询”与“需要完整 `(SidebarAgentKind, AgentState)` 的列表渲染”，让高频计数路径复用了重度状态派生函数 `agent_entries()`。
- **修复措施**：
  1. 在 `AppState`（`src/app/state.rs`）上新增零分配、免状态推导的 `pub fn agent_entry_count(&self) -> usize`：仅统计 `pane.lx.has_active_session()` 与 `pane.agent.is_some()`（且已绑定 `session_id` 时 `has_active_session()` 首项短路返回 `true`，`O(1)` 不遍历消息）。
  2. `workspace_list::agent_scroll_max`、`workspace_list::clamp_agent_scroll`、`sidebar::agent_scrollbar`、`sidebar::agent_item_index_at` 统一改用 `state.agent_entry_count()`；`agent_scrollbar_for` 接收已计算的 `total: usize` 参数，避免单次 `agent_item_index_at` 或 `render_agents` 重复统计。
- **验证**：
  - `src/ui/sidebar/tests.rs` 全部 18 项测试与 `src/event/tests/mouse_click.rs` 全部 21 项测试通过。

---

### P2-1：`Agents` 列表条目减少或关闭工作区/标签/窗格时未同步清理 `agent_hover` 与 `agent_scroll_drag`

- **涉及位置**：
  - `src/app/update/workspace_list.rs:60-78`（`clamp_agent_scroll`）
  - `src/app/update/workspace.rs:369-424`（`close_workspace`、`close_tab`、`close_pane`）
- **关键点**：
  - 当用户在 `lx` 中执行 `/new`、删除会话或终端外部 Agent 退出导致 `Agents` 条目数减少时，`clamp_agent_scroll` 仅钳制了 `agent_scroll`，未清理已越界（`slot >= total`）或因滚动偏移回退而错位的 `agent_hover`，也未在列表不再溢出（`max == 0`）时清理 `agent_scroll_drag`。若之后列表再次增长，旧 `agent_hover` 会在未悬停行上产生幽灵高亮。
  - 此外，`close_workspace` 在关闭当前激活工作区并切换到相邻工作区时，未像 `switch_workspace` 那样复位 `agent_scroll`、`agent_hover` 与 `agent_scroll_drag`；`close_tab` 与 `close_pane` 在关闭当前工作区的标签或窗格时也未复位 `agent_hover` 与 `agent_scroll_drag`。
- **错误原因**：
  - `clamp_agent_scroll` 早先只处理 `state.agent_scroll > max`；`close_workspace` 直接修改 `state.active_workspace` 而绕过了 `switch_workspace` 的侧栏状态复位。
- **修复措施**：
  1. 在 `clamp_agent_scroll` 中：当 `state.agent_scroll > max` 发生回退或 `state.agent_hover.is_some_and(|slot| slot >= total)` 时，将 `state.agent_hover` 置为 `None`；当 `max == 0` 时通过 `state.agent_scroll_drag.take()` 清理滚动条拖拽态。
  2. 在 `close_workspace`（`removed_active` 分支）中复位 `state.agent_scroll = 0`、`state.agent_hover = None`、`state.agent_scroll_drag = None`；在 `close_tab` 与 `close_pane`（`workspace == state.active_workspace`）中复位 `state.agent_hover = None` 与 `state.agent_scroll_drag = None`。
- **验证**：
  - `src/app/update/tests.rs::clamp_agent_scroll_and_close_containers_clear_stale_agent_hover_and_drag`

---

### P3-1：丢弃目标视图的旧接口 `agent_item_at` 与 `focus_agent_pane` 未限制为测试专用，且标签映射函数可收敛为 `const fn`

- **涉及位置**：
  - `src/ui/sidebar.rs:610-618`（`agent_item_at`）
  - `src/ui/mod.rs:29-39`（`pub use sidebar::agent_item_at`）
  - `src/app/update/pane.rs:940-943`（`focus_agent_pane`）
  - `src/ui/text/workspace.rs:39`、`73`（`agent_label`、`sidebar_agent_label`）
- **关键点**：
  - `ui::agent_item_at` 丢弃了条目对应的 `PaneView`，`update::focus_agent_pane` 则无条件将窗格切往 `PaneView::Terminal`。当前生产代码已全部迁移至 `ui::agent_target_at` 与 `update::focus_agent_pane_view`，旧接口仅剩历史单元测试调用；若继续向生产代码公开暴露，后续调用方误用会再次导致点击 `lx-*` 条目被强制切到终端视图。
  - `text::agent_label` 与 `text::sidebar_agent_label` 均为纯编译期常量模式匹配，可与 `lx_agent_label` 一致声明为 `pub const fn`。
- **错误原因**：
  - 初版实现为兼容既有单元测试保留了原函数签名，但未添加 `#[cfg(test)]` 编译门控。
- **修复措施**：
  1. 为 `ui::sidebar::agent_item_at`、`src/ui/mod.rs` 中的 `pub use sidebar::agent_item_at` 以及 `app::update::pane::focus_agent_pane` 加上 `#[cfg(test)]` 门控。
  2. 将 `ui::text::workspace::agent_label` 与 `sidebar_agent_label` 提升为 `pub const fn`。
- **验证**：
  - `rtk cargo clippy --all-targets -- -D warnings` 零警告通过，生产构建与测试构建均编译正常。

---

## 复审后验证

在 `.worktrees/sidebar-lx-agent-items` 工作区执行精确验证：

- `rtk cargo fmt --check`：通过
- `rtk cargo clippy --all-targets -- -D warnings`：通过（0 warnings, 0 errors）
- `rtk cargo test --lib ui::sidebar::tests::`：18 passed, 0 failed
- `rtk cargo test --lib app::update::tests::focus_agent_pane`：3 passed, 0 failed
- `rtk cargo test --lib app::update::tests::agent_panes`：1 passed, 0 failed
- `rtk cargo test --lib app::update::tests::clamp_agent_scroll`：1 passed, 0 failed
- `rtk cargo test --lib event::tests::mouse_click::`：21 passed, 0 failed
- `rtk cargo test --lib db::service::resume_tests::`：11 passed, 0 failed
