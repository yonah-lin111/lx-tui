# `/mcp` 命令与窗格级 MCP 管理 — 执行任务清单 (`task.md`)

- **特性目录**：`docs/feature/lx-mcp-command/`
- **关联设计文档**：[`design.md`](design.md)
- **当前状态**：已完成实现与验证
- **数据库迁移**：无（`SCHEMA_VERSION` 保持不变）

---

## 0. 评审门禁

- [x] 已通过 Grill Me 与用户逐项确认 Q1–Q4 全部设计边界
- [x] 用户已审核并确认 `design.md` 与 `task.md`

---

## 1. 文案与保留命令名层

### 1.1 `src/ui/text.rs`

- [x] 新增 `/mcp` 命令与面板相关英文常量：

```rust
/// `/mcp` 命令描述。
pub const LX_MCP_COMMAND_DESC: &str = "Manage MCP servers for this pane";
/// `/mcp` 面板标题。
pub const LX_MCP_PANEL_TITLE: &str = "MCP servers";
/// `/mcp` 面板底栏按键提示。
pub const LX_MCP_PANEL_FOOTER: &str = "↑↓ select · space/↵ toggle · esc close";
/// `/mcp` 面板空态文案。
pub const LX_MCP_EMPTY: &str = "No MCP servers configured";
/// `/mcp` 面板状态标签。
pub const LX_MCP_STATUS_CONNECTED: &str = "Connected";
pub const LX_MCP_STATUS_CONNECTING: &str = "Connecting";
pub const LX_MCP_STATUS_FAILED: &str = "Failed";
pub const LX_MCP_STATUS_NEEDS_LOGIN: &str = "Needs login";
pub const LX_MCP_STATUS_DISABLED: &str = "Disabled";
pub const LX_MCP_STATUS_DISABLED_SETTINGS: &str = "Disabled in settings";
/// 尝试切换已在全局/项目设置中禁用的 MCP 时的警告提示。
pub const LX_MCP_DISABLED_IN_SETTINGS: &str =
    "This MCP server is disabled in settings. Enable it in Settings first.";
```

### 1.2 `src/settings/commands.rs`

- [x] 在 `RESERVED_NAMES` 中加入 `"mcp"` 与 `"mcps"`，禁止自定义命令重名覆盖。

---

## 2. 状态层：面板模型与窗格级禁用集

### 2.1 新增 `src/app/state/lx/mcp.rs`（含单测 `src/app/state/lx/mcp/tests.rs`）

- [x] 定义 `McpRuntimeState`、`McpCatalogEntry`、`McpServerState`、`McpPanelEntry`、`McpPanel`：

```rust
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum McpRuntimeState {
    #[default]
    Connecting,
    Ready { tools: usize },
    Failed { error: String },
    NeedsLogin,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct McpCatalogEntry {
    pub name: String,
    pub summary: String,
    pub config_enabled: bool,
    pub runtime: McpRuntimeState,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum McpServerState {
    Connected { tools: usize },
    Connecting,
    Failed { error: String },
    NeedsLogin,
    DisabledInPane,
    DisabledInSettings,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct McpPanelEntry {
    pub name: String,
    pub summary: String,
    pub preview: String,
    pub state: McpServerState,
}
```

- [x] 实现 `McpPanel::new`、`McpPanel::sync_entries`（按 server 名保持当前高亮项不漂移）、`McpPanel::refilter`、`McpPanel::visible`、`McpPanel::active_entry`、`McpPanel::clamp_window`、`McpPanel::is_filter_empty`。

### 2.2 修改 `src/app/state/lx/slash.rs`

- [x] 在 `SlashCommand` 枚举与 `SlashCommand::ALL`（长度 9 -> 10）中新增 `SlashCommand::Mcp`（`name() => "mcp"`、`aliases() => &["mcps"]`、`text() => "/mcp"`、`description() => crate::ui::text::LX_MCP_COMMAND_DESC`）。
- [x] 在 `SlashPanel` 中新增 `Mcp(Box<crate::app::state::lx::mcp::McpPanel>)`，并在 `item_count()`、`window()`、`current_index()`、`set_window()` 中补齐分支。

### 2.3 修改 `src/app/state/lx.rs` 与 `src/app/state/lx/mention.rs`

- [x] 在 `Lx` 中新增 `disabled_mcp_servers: std::collections::BTreeSet<String>`，并在 `Lx::build` 中初始化为空集合。
- [x] 在 `Lx` 上新增 `disabled_mcp_servers(&self)`、`is_mcp_server_disabled(&self, name: &str)`、`set_mcp_server_disabled(&mut self, name: &str, disabled: bool) -> bool`。
- [x] 在 `LxMention` 中新增 `disabled_mcp_servers: std::collections::BTreeSet<String>` 与 `set_disabled_mcp_servers`，并在 `LxMention::filtered_mcp_servers(&self)` 中过滤掉 `!self.disabled_mcp_servers.contains(&server.name)`。

### 2.4 修改 `src/app/state.rs` 与 `src/app/effects.rs`

- [x] 在 `AppState` 中新增 `pub mcp_catalog: BTreeMap<PathBuf, Vec<crate::app::state::lx::mcp::McpCatalogEntry>>`。
- [x] 更新 `AppState::sync_lx_inventory(&mut self)`：若 `self.mcp_catalog` 中有对应项目目录，则按每个窗格的 `pane.lx.disabled_mcp_servers()` 独立计算该窗格的 `StartupCount`（剔除被窗格禁用的 MCP）；若无（如纯 `mcp_inventory` 单元测试）则回退原 `self.mcp_inventory`。
- [x] 在 `src/app/effects.rs` 的 `Effect` 枚举中新增：
  ```rust
  /// 刷新指定窗格所属工作区的 MCP 目录快照并同步已打开的 `/mcp` 面板。
  RefreshMcpCatalog { pane: crate::layout::PaneId },
  /// 重启指定窗格所属工作区下的单个 MCP server 连接。
  RestartMcpServer {
      pane: crate::layout::PaneId,
      server: String,
  },
  ```

---

## 3. 应用更新层：面板交互与键盘分派

### 3.1 新增 `src/app/update/chat/mcp.rs`

- [x] 实现 `mcp_root_for_pane`、`build_panel_entries`、`open_mcp_panel`、`toggle_mcp_entry`、`refresh_pane_mcp_panel`、`refresh_all_open_mcp_panels`：
  - `open_mcp_panel(state, pane)`：用当前 `state.mcp_catalog` 与 `pane.lx.disabled_mcp_servers()` 立即打开 `SlashPanel::Mcp`，并压入 `Effect::RefreshMcpCatalog { pane }`；
  - `toggle_mcp_entry(state, pane)`：
    - `DisabledInSettings` -> 弹窗格内警告 Toast（`LX_MCP_DISABLED_IN_SETTINGS`），保持面板打开；
    - `Failed { .. }` -> 将 `state.mcp_catalog` 中该项置为 `Connecting`，压入 `Effect::RestartMcpServer { pane, server }`，刷新面板并保持打开；
    - `DisabledInPane` -> 从 `pane.lx.disabled_mcp_servers` 移除（若底层为 `Failed` 则同步触发 `Effect::RestartMcpServer` 并置 `Connecting`），刷新面板与 `sync_lx_inventory()`；
    - `Connected` / `Connecting` / `NeedsLogin` -> 加入 `pane.lx.disabled_mcp_servers`，刷新面板与 `sync_lx_inventory()`。

### 3.2 修改 `src/app/update/chat/slash.rs` 与 `src/app/update/pane.rs`

- [x] 在 `src/app/update/chat/slash.rs` 中：
  - `apply_slash_enter` 新增 `SlashPanel::Mcp(_) => super::mcp::toggle_mcp_entry(state, pane)`；
  - `apply_slash_command` 新增 `SlashCommand::Mcp => super::mcp::open_mcp_panel(state, pane)`；
  - `refresh_slash_filter` 新增 `Some(SlashPanel::Mcp(panel)) => panel.refilter(&query)`。
- [x] 在 `src/app/update/pane.rs` 的 `if let Some(slash) = lx.slash.as_ref()` 分支中：
  - 拦截 `LxInput::Char(' ') if matches!(slash, super::super::state::SlashPanel::Mcp(_))`：调用 `super::chat::apply_slash_enter(state, pane_id); return;`。

---

## 4. MCP 快照裁剪与事件层接线

### 4.1 修改 `src/mcp/manager/snapshot.rs` 与 `src/mcp/manager/resources.rs`

- [x] 在 `McpToolSnapshot` 上实现 `without_servers(self: &Arc<Self>, disabled_servers: &BTreeSet<String>) -> Arc<Self>`。
- [x] 在 `src/mcp/manager/resources.rs` 的 `connection_for` 中增加 `snapshot.server(server)?;` 门禁，阻止读取已在当前快照中被剔除的 MCP server 资源。

### 4.2 修改 `src/event/chat/mcp.rs`、`src/event/chat/mod.rs`、`src/event/chat/subagent/`

- [x] 在 `src/event/chat/mcp.rs` 中新增 `snapshot_for_turn_with_disabled(persistence, workspace_root, disabled_servers)`。
- [x] 在 `src/event/chat/mod.rs:606` 中改用 `mcp::snapshot_for_turn_with_disabled(persistence, config_root.as_deref(), pane.lx.disabled_mcp_servers())`。
- [x] 在 `src/event/chat/subagent/context.rs` 的 `capture_child_prompt_inputs` 中接收 `parent_pane: PaneId`，读取父窗格的 `disabled_mcp_servers` 并调用 `snapshot_for_turn_with_disabled`（同步更新 `launch.rs` 与 `runner.rs` 的两处调用点）。

### 4.3 修改 `src/event/inventory.rs` 与 `src/event/mod.rs`

- [x] 在 `src/event/inventory.rs` 中新增 `sync_mcp_catalog_for_root(state, root, effective, statuses)`：
  - 从 `effective_for_root(persistence, root)`、`statuses` 与 `global_manager().cached_snapshot(root)` 汇总 `Vec<McpCatalogEntry>` 写入 `state.mcp_catalog`；
  - 在 `initialize_mcp`、`refresh_mcp`、`drain_mcp_events` 中调用同步，并调用 `crate::app::update::chat::mcp::refresh_all_open_mcp_panels(state)`。
- [x] 在 `src/event/inventory.rs` 中新增 `apply_mcp_effects(state, persistence, statuses) -> bool` 处理 `Effect::RefreshMcpCatalog` 与 `Effect::RestartMcpServer`，并在 `src/event/mod.rs` 事件循环中于 `persistence.pump(state)` 之前调用。

---

## 5. UI 渲染层：`src/ui/main_content/lx/slash.rs`

- [x] 在 `items()` 中实现 `SlashPanel::Mcp(panel)` 分支（空态 `LX_MCP_EMPTY`、过滤无果 `LX_SLASH_NO_MATCHES`、正常条目 `CommandItem::Inline { label: &entry.name, preview: &entry.preview, tag: entry.status_tag() }`）。
- [x] 在 `is_placeholder()` 中补充 `SlashPanel::Mcp(p) => p.entries.is_empty() || p.is_filter_empty()`。
- [x] 在 `view()` 中补充 `SlashPanel::Mcp(panel)` 的标题 `LX_MCP_PANEL_TITLE` 与底栏 `LX_MCP_PANEL_FOOTER`。
- [x] 新增 `disabled_mask(panel: &SlashPanel) -> Option<Vec<bool>>`，在 `layout()`、`item_at()`、`render()` 中传入 `CommandPanelView.disabled`，使 `DisabledInSettings` 条目置灰并在聚焦时显示 `style::muted()` 的 `❯` 标记。

---

## 6. CLI 扩展与代码评审修复（`review.md` P1–P3）

- [x] `lx-tui mcp list --disable-mcp <NAME>...` 与 `lx-tui exec [resume] --disable-mcp <NAME>...` 无头 CLI 扩展（`src/cli/{args,help,mcp}.rs`、`src/cli/mcp/exec.rs`、`src/chat/agent/exec/{cli,mod}.rs`）。
- [x] **P1-1**：`refresh_slash_panel` 守卫已打开的二级面板（`/mcp`、`/model`、`/resume`、`/fork`、`/rewind`），输入 `/` 开头的过滤词不再被覆写为 `SlashPanel::Commands`。
- [x] **P1-2**：`Lx::refresh_panel` 与 `open_mcp_panel` 在二级斜杠面板打开期间压制 `LxMention` / `BlockPanel` / `ReferPanel`，输入 `@` 过滤不再唤醒隐藏提及面板抢走按键与鼠标。
- [x] **P1-3**：`src/event/mod.rs` 在 `persistence.pump` 后当 `agents_md_scopes`（含工作区信任状态）变化时同步调用 `prune_mcp_statuses` + `refresh_mcp`，确保授予工作区信任后立即加载项目级 MCP。
- [x] **P2-1**：`src/ui/widgets/command_panel.rs` 对带 `window_anchor` 的斜杠面板在焦点处于 `Disabled in settings` 行时保留 `style::muted()` 的 `❯` 焦点标记。
- [x] **P2-2 & P2-3**：`drain_mcp_events` 在 `Starting`/`Ready` 时重武装 `mcp_toasted`、不覆盖 `entry.config_enabled`，且 `Effect::RefreshMcpCatalog` 同步更新 `@` 候选并触发 `prewarm_projects`。
- [x] **P2-4 & P3-1..P3-4**：`mcp_snapshot_for_turn_async` 全禁用返回空快照并对未知 server 告警、`mcp_root_for_pane` 回退 `workspace_cwd_for_pane`、`sanitize_single_line` 剥离 ANSI 控制序列、`select_lx_slash` 边界校验、`restart_projects` 增加 Tokio runtime 守卫。

---

## 7. 测试任务

- [x] `src/app/state/lx/mcp/tests.rs`：
  - `catalog_to_panel_entry_maps_all_six_states_and_tool_pluralization`（含 ANSI 序列剥离断言）
  - `mcp_panel_refilter_and_sync_entries_preserve_highlighted_server`（含过滤失配重置窗口断言）
- [x] `src/app/state/lx/slash/tests.rs`：
  - `mcp_command_and_mcps_alias_match_in_command_panel`
- [x] `src/mcp/manager/tests.rs`：
  - `snapshot_without_servers_filters_tools_prompts_and_resources`
- [x] `src/app/update/chat/tests/slash_panels.rs`：
  - `mcp_panel_toggles_per_pane_without_affecting_sibling_pane`
  - `mcp_panel_space_and_enter_keep_panel_open_and_persist_across_new_session`
  - `mcp_panel_failed_entry_triggers_restart_and_settings_disabled_shows_warning`
  - `mcp_panel_typing_slash_or_at_filters_without_hijacking_and_rejects_oob_click`
- [x] `src/ui/main_content/lx/tests/slash_panels.rs`：
  - `mcp_panel_renders_statuses_and_dims_settings_disabled_rows`（含 `DisabledInSettings` 焦点 `❯` 断言）
- [x] `src/event/inventory/tests.rs`：
  - `drain_mcp_events_updates_open_mcp_panel_and_suppresses_toast_on_disabled_pane`（含 `Starting -> Failed` Toast 重武装断言）
- [x] `src/chat/agent/exec/tests/cli_tests.rs` & `src/cli/tests/config_domains_tests.rs`：
  - `exec_parses_disable_mcp_flags_and_rejects_invalid_names`（含 `exec resume --disable-mcp`）
  - `mcp_skill_command_subagent_and_settings_crud_work`

---

## 8. 验证命令与完成定义（DoD）

```bash
rtk cargo fmt --all -- --check
rtk cargo clippy --all-targets -- -D warnings
rtk cargo test app::state::lx::mcp
rtk cargo test app::state::lx::slash
rtk cargo test mcp::manager::tests
rtk cargo test app::update::chat::tests::slash_panels
rtk cargo test ui::main_content::lx::tests::slash_panels
rtk cargo test event::inventory::tests
rtk cargo test chat::agent::exec::tests::cli_tests
rtk cargo test cli::tests::config_domains_tests
```

### 完成定义（DoD）
- [x] 所有受影响模块的单元测试与集成测试 100% 通过
- [x] `cargo fmt` 与 `cargo clippy --all-targets -- -D warnings` 零警告
- [x] 所有修改/新增生产文件 ≤ 1000 行，测试文件 ≤ 1500 行
- [x] 无 `unwrap()` / `expect()` 混入生产代码，所有用户可见文案均为英文
