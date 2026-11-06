# `/mcp` 命令与窗格级 MCP 管理 — 代码评审报告 (`review.md`)

- **特性目录**：`docs/feature/lx-mcp-command/`
- **关联文档**：[`design.md`](design.md) · [`task.md`](task.md) · [`verification.md`](verification.md)
- **评审范围**：`git diff dev...HEAD`（包含状态层、更新层、输入路由、MCP 快照裁剪、事件层库存同步、UI 渲染层与 CLI `--disable-mcp` 扩展）
- **评审汇总**：
  - **P0（崩溃 / 数据损坏 / 安全逃逸）**：0 项
  - **P1（核心功能缺陷 / 状态互窜 / 能力不一致）**：3 项（P1-1 ～ P1-3）
  - **P2（重要交互缺陷 / 反馈缺失 / 边界泄漏）**：4 项（P2-1 ～ P2-4）
  - **P3（边缘条件 / 健壮性打磨 / 代码整洁度）**：4 项（P3-1 ～ P3-4）
- **当前状态**：全部 P1–P3 问题已定位并完成修复与回归测试验证，已合并入主工作区 `dev` 分支并清理 worktree

---

## P0 级问题

无。生产代码无 `unwrap()` / `expect()`，无 SQLite Schema 变更，无跨窗格进程误杀。

---

## P1 级问题（核心功能与状态一致性缺陷）

### P1-1：`/mcp`（及二级斜杠面板）过滤词以 `/` 开头时会被 `SlashPanel::Commands` 强行覆盖

- **关键点**：
  - `src/app/update/chat/slash.rs:501-565`（`refresh_slash_panel`）
  - `src/app/state/lx/slash.rs:866-880`（`slash_trigger`）
- **错误原因**：
  `refresh_slash_panel` 在执行 `refresh_slash_filter(state, pane)` 更新二级面板过滤视图后，没有检查当前 `target.lx.slash` 是否已经处于二级面板（`SlashPanel::Mcp` / `Model` / `History` / `Fork` / `Rewind`），而是继续对输入框内容调用 `slash_trigger`。由于 MCP 传输摘要常包含路径（如 `/usr/local/bin/...`），一旦用户在 `/mcp` 面板打开时输入 `/` 开头的过滤词，`slash_trigger` 立即返回 `Some`，且 `/` 匹配全部内置命令，导致 `target.lx.slash` 被无条件覆写为 `Some(SlashPanel::Commands { .. })`，`/mcp` 面板直接丢失。
- **修复方案**：
  在 `refresh_slash_panel` 完成 `refresh_slash_filter(state, pane)` 后增加守卫：当 `target.lx.slash` 当前为二级面板（非 `None` 且非 `SlashPanel::Commands`）时直接返回，禁止命令候选触发逻辑覆写已打开的二级面板。

---

### P1-2：`/mcp` 面板打开期间输入 `@` 会唤醒底层隐藏的 `@` 提及面板并劫持键盘与鼠标路由

- **关键点**：
  - `src/app/state/lx/panels.rs:29-48`（`Lx::refresh_panel`）
  - `src/app/update/pane.rs:211-249`（`apply_lx_input` 中 `mention` 优先于 `slash`）
  - `src/event/mouse/press.rs:744-772`（鼠标点击命中 `mention` 优先于 `slash`）
  - `src/app/update/chat/mcp.rs:39-53`（`open_mcp_panel`）
- **错误原因**：
  MCP 的命令摘要大量包含 npm scoped 包名（如 `@modelcontextprotocol/server-memory`）。当用户在 `/mcp` 面板中输入 `@` 过滤时，`Lx::refresh_panel` 无条件调用 `self.mention.unsuppress()` 与 `self.mention.refresh(...)`。虽然渲染层后绘制的 `SlashPanel::Mcp` 在视觉上遮住了 `LxMention`，但键盘处理（`pane.rs:211`）和鼠标命中（`press.rs:744`）均优先检查 `mention_visible` / `mention_open`，导致 `Left`/`Right`/`Up`/`Down`/`Enter`/`Esc` 全部被底层看不见的 `@` 提及面板抢走。
- **修复方案**：
  1. 在 `Lx::refresh_panel` 中，当 `self.slash.is_some()` 时立即压制提及与附属面板（`self.mention.suppress(); self.panel = None; self.refer = None; self.scroll_cursor_into_view(); return;`）；
  2. 在 `open_mcp_panel` 打开面板时主动关闭 `mention` / `panel` / `refer`，确保 `SlashPanel` 与其他输入面板严格互斥。

---

### P1-3：工作区授予信任（`GrantTrust`）后未刷新 `mcp_catalog` 与 `@` 候选，导致项目级 MCP 在 `/mcp` 中缺失

- **关键点**：
  - `src/event/mod.rs:302-307`（`roots != mcp_roots` 检查早于 `persistence.pump`）
  - `src/event/mod.rs:391-405`（`persistence.pump` 执行 `Effect::GrantTrust`）
  - `src/event/inventory.rs:265-291`（`refresh_mcp`）
- **错误原因**：
  当用户打开未信任工作区并点击确认信任时，`apply_trust_decision` 先将工作区加入 `state.workspaces` 并压入 `Effect::GrantTrust`。下一轮事件循环顶部检测到 `roots != mcp_roots` 调用 `initialize_mcp` 时，`Effect::GrantTrust` 尚未被循环中段的 `persistence.pump(state)` 落库，`persistence.is_trusted(root)` 仍为 `false`，项目级 `.lx-t/mcp.json` 中的 MCP 被全部跳过；等到 `GrantTrust` 在 `pump` 中落库后，`mcp_roots` 已不再变化，导致 `mcp_catalog`、首页计数与 `@` 候选永远缺失项目级 MCP（而发起聊天时 `snapshot_for_turn` 实时读库却能拿到，造成 UI 与实际状态不一致）。
- **修复方案**：
  在 `src/event/inventory.rs` 中新增 `mcp_scopes(state, persistence) -> Vec<(PathBuf, bool)>`（携带每个项目根及其 `is_trusted` 状态），在 `src/event/mod.rs` 的 `persistence.pump(state)` 之后（与 `agents_md_scopes` 同一位置）检测 `current_mcp_scopes != mcp_scopes`；一旦项目根或信任状态变化，立即调用 `prune_mcp_statuses` 与 `refresh_mcp` 同步目录并后台预热。

---

## P2 级问题（重要交互缺陷与边界问题）

### P2-1：`[Disabled in settings]` 行在获得焦点时隐藏了行首 `❯` 标记，导致键盘导航光标消失

- **关键点**：
  - `src/ui/widgets/command_panel.rs:117`、`:682-689`、`:730-737`
  - `src/ui/main_content/lx/slash.rs:196-206`（`disabled_mask`）
- **错误原因**：
  `SlashPanel::Mcp` 允许用户通过上下方向键移动到 `[Disabled in settings]` 行并按 `Space`/`Enter` 弹出提示 Toast。由于二级斜杠面板采用 `borderless: true`（无整行背景色高亮，仅靠行首 `❯` 指示当前焦点行），而 `CommandPanelView.disabled` 会令 `is_active = !is_disabled && index == view.active` 为 `false`，导致方向键移到 `[Disabled in settings]` 行时行首 `❯` 消失，用户完全看不到当前焦点在哪一行。
- **修复方案**：
  在 `CommandPanelView` 中新增 `pub dimmed: Option<&'a [bool]>`（与不可选的 `disabled` 区分）：`dimmed` 行将文本与标签渲染为 `style::muted()` 置灰样式，但在 `index == view.active` 时依然保留行首 `❯` 焦点标记（置灰样式），既保证既有 `disabled` 单元测试兼容，又让 `/mcp` 中的 `[Disabled in settings]` 行在导航时焦点清晰可见。

---

### P2-2：手动触发 `Failed` 服务重连或服务恢复后再次报相同错误时，`mcp_toasted` 去重集未清理导致静默无提示

- **关键点**：
  - `src/event/inventory.rs:111-208`（`drain_mcp_events`）
  - `src/event/inventory.rs:425-455`（`apply_mcp_effects`）
- **错误原因**：
  1. `mcp_toasted: HashSet<(PathBuf, String, String)>` 只在首次报错时插入 `(project, server, error)`，在收到 `ServerStatus::Starting`、`ServerStatus::Ready` 时从不清理该 `(project, server)` 的历史记录。当用户在 `/mcp` 中对 `Failed` 条目按 `Enter` 触发重连，若再次因相同原因（如命令不存在或端口未开）失败，`mcp_toasted.insert` 返回 `false`，不会弹出任何反馈 Toast；
  2. 当该项目下所有窗格都禁用了某个 MCP 时，循环跳过了所有窗格，但 `mcp_toasted.insert` 已经提前执行并消费了该键。
- **修复方案**：
  1. 在 `drain_mcp_events` 收到 `ServerStatus::Starting` 或 `ServerStatus::Ready` 时，执行 `mcp_toasted.retain(|(p, s, _)| p != &project || s != &server)`；
  2. 先筛选出未禁用该 MCP 的目标窗格列表，仅当目标窗格非空且 `mcp_toasted.insert(...)` 成功时才投递 Toast。

---

### P2-3：`McpEvent::Status` 会将已全局禁用的目录项强行改回 `config_enabled = true`，且 `RefreshMcpCatalog` 未触发预热

- **关键点**：
  - `src/event/inventory.rs:142-169`（`drain_mcp_events`）
  - `src/event/inventory.rs:415-423`（`Effect::RefreshMcpCatalog`）
- **错误原因**：
  1. `drain_mcp_events` 在收到 `Starting` / `Ready` / `Failed` / `NeedsLogin` 时无条件写入了 `entry.config_enabled = true`。若用户在设置页将该 MCP 设为禁用后，此前在途的事件晚到，会把 `/mcp` 面板中的 `[Disabled in settings]` 错误改回启用态；
  2. `Effect::RefreshMcpCatalog` 仅调用 `sync_mcp_catalog_for_root` 而未调用 `prewarm_projects` 与 `apply_mcp_mentions`，导致运行期间外部修改 `mcp.json` 新增的 server 在打开 `/mcp` 面板时永远停在 `Connecting` 而不发起连接。
- **修复方案**：
  1. 在 `drain_mcp_events` 中仅更新 `entry.runtime`，不修改 `entry.config_enabled`（`config_enabled` 唯一权威来源为 `sync_mcp_catalog_for_root`）；
  2. 在 `Effect::RefreshMcpCatalog` 中同步更新 `@` 候选并调用 `super::chat::mcp::prewarm_projects(persistence, &[root])`。

---

### P2-4：无头模式 `lx-tui exec --disable-mcp` 在全禁用时回退进程缓存，且对未知 MCP 名称静默忽略

- **关键点**：
  - `src/chat/agent/exec/mod.rs:94-114`、`:637-664`
  - `src/cli/mcp/exec.rs:147-196`
- **错误原因**：
  1. `mcp_snapshot_for_turn_async` 在过滤后若没有任何 `enabled` server，走 `else { manager.cached_snapshot(workspace) }`；若同进程缓存非空且 server 是因配置 `enabled: false` 而非在 `disabled_mcp` 集合中，可能残留旧缓存；
  2. 当 `--disable-mcp <NAME>` 传入当前工作区未配置的 server 名称时，完全静默，无日志或告警提示；
  3. 仅测试使用的同步 `assemble_exec_turn` / `mcp_snapshot_for_turn` 未标 `#[cfg(test)]`。
- **修复方案**：
  1. 当过滤后无任何 `enabled` server 时，直接返回 `Arc::new(crate::mcp::McpToolSnapshot::default())`；
  2. 对 `--disable-mcp` 中未命中生效配置的名称记录 `tracing::warn!`；
  3. 为 `assemble_exec_turn` 与 `mcp_snapshot_for_turn` 添加 `#[cfg(test)]`。

---

## P3 级问题（边缘条件与健壮性打磨）

### P3-1：`mcp_root_for_pane` 回退到了 `active_workspace` 而非目标窗格所属工作区

- **关键点**：`src/app/update/chat/mcp.rs:15-19`
- **错误原因**：`state.chat_session_key_for_pane(pane).or_else(|| state.active_workspace().repo_key().map(PathBuf::from))` 在目标窗格无仓库键时错误回退到了当前激活工作区。
- **修复方案**：改为 `state.chat_session_key_for_pane(pane).or_else(|| state.workspace_cwd_for_pane(pane))`。

---

### P3-2：`sanitize_single_line` 未清理控制字符与 ANSI 转义序列

- **关键点**：`src/app/state/lx/mcp.rs:109-115`
- **错误原因**：子进程 stderr 报错常含 ANSI 颜色码（`\x1b[31m...`）或控制字符，直接拼入单行 `preview` 会污染终端渲染。
- **修复方案**：在 `sanitize_single_line` 中先剥离 ANSI CSI/OSC 转义序列并过滤控制字符，再取首个非空行。

---

### P3-3：`select_lx_slash` 未校验 `index` 边界，且 `McpPanel::sync_entries` 在高亮项被过滤掉时未重置窗口

- **关键点**：`src/app/update/pane/popups.rs:180-187`、`src/app/state/lx/mcp.rs:162-176`
- **错误原因**：
  1. `select_lx_slash` 未检查 `index < panel.item_count()`，当点击索引失效时仍触发 `apply_slash_enter` 切换旧高亮项；
  2. `McpPanel::sync_entries` 在原 `active_name` 不在过滤结果中时未将 `window` 重置为首项。
- **修复方案**：
  1. 在 `select_lx_slash` 中校验 `index < panel.item_count()`，越界直接返回 `false`；
  2. 在 `McpPanel::sync_entries` 中若原 `active_name` 未找到，重置 `self.window = PanelWindow::default()`。

---

### P3-4：`restart_projects` 缺少 Tokio Runtime 守卫，且 `toggle_pane_view` 切到终端时未关闭 `/mcp` 面板

- **关键点**：`src/event/chat/mcp.rs:100-113`、`src/app/update/pane.rs:658-675`
- **错误原因**：
  1. `restart_projects` 直接调用 `tokio::spawn`，在无 Tokio runtime 的同步单元测试中调用 `apply_mcp_effects` 会 panic；
  2. `toggle_pane_view` 切换到终端视图时未清理已打开的 `SlashPanel::Mcp`。
- **修复方案**：
  1. 在 `restart_projects` 入口增加 `if tokio::runtime::Handle::try_current().is_err() { return; }`；
  2. 在 `toggle_pane_view` 中切换到终端视图时若 `pane.lx.slash` 为 `Some(_)` 则关闭并清理。
