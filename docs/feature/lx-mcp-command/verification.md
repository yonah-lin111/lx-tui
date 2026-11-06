# `/mcp` 命令与窗格级 MCP 管理 — 验证报告 (`verification.md`)

- **特性目录**：`docs/feature/lx-mcp-command/`
- **关联文档**：[`design.md`](design.md) · [`task.md`](task.md)
- **验证工作区**：`.worktrees/lx-mcp-command`（分支 `feature/lx-mcp-command`）
- **验证结论**：全部 8 个验证项通过（`PASS`）

---

## 0. `lx-tui` CLI 工具盘点与补齐判定

在执行验证前，先对 `lx-tui` 现有无头 CLI 工具链（`src/cli/` 与 `src/chat/agent/exec/`）进行了能力核查：

| CLI 子命令 / 能力 | 原有状态 | 判定与动作 | 说明 |
|---|---|---|---|
| `lx-tui mcp add-stdio` / `add-http` / `enable` / `disable` / `show` / `remove` / `probe` | 已存在（`src/cli/mcp.rs`） | **直接复用** | 支持 User 与 Project 双作用域的 MCP 增删改查、全局启停与连通性探测 |
| `lx-tui command create` | 已存在（`src/cli/command.rs`） | **直接复用** | 调用 `settings::commands::validate_name`，可直接验证 `mcp` / `mcps` 保留词拦截 |
| `lx-tui mcp list --disable-mcp <NAME>...` | **缺失**（原 `mcp list` 仅展示磁盘静态 `enabled`） | **已新增 CLI 选项** | 在 `src/cli/mcp.rs` 与 `src/cli/mcp/exec.rs` 中新增 `--disable-mcp <NAME>`（可重复或逗号分隔），支持在不改写磁盘 `mcp.json` 的前提下投影窗格级禁用视图（表格显示 `no (pane)`，JSON 输出 `paneDisabled` 与 `effectiveEnabled`） |
| `lx-tui exec --disable-mcp <NAME>...`（含 `exec resume`） | **缺失**（原无头执行无法按单轮/单会话屏蔽特定 MCP） | **已新增 CLI 选项** | 在 `src/chat/agent/exec/cli.rs` 与 `src/chat/agent/exec/mod.rs` 中新增 `--disable-mcp <NAME>`，在单次无头会话轮次中通过 `McpToolSnapshot::without_servers` 裁剪指定 MCP，不触碰磁盘 `mcp.json` |

---

## 1. 逐项验证明细

### 验证项 V1：内置命令 `/mcp` 与别名 `/mcps` 保留名保护（防自定义命令覆盖）

- **输入内容**：
  - 自定义命令名：`mcp`、`mcps`
  - 命令参数：`--kind agentMD --description "test" --content "body"`
- **验证步骤**：
  1. 使用隔离环境目录初始化并信任测试工作区：
     ```bash
     export HOME="$VERIFY_DIR/home"
     export LX_TUI_DB_PATH="$VERIFY_DIR/lx.db"
     ./target/debug/lx-tui workspace trust -C "$VERIFY_DIR/ws"
     ```
  2. 使用 `lx-tui command create` 尝试创建名为 `mcp` 与 `mcps` 的自定义斜杠命令：
     ```bash
     ./target/debug/lx-tui command create mcp --kind agentMD --description "test" --content "body" -C "$VERIFY_DIR/ws"
     ./target/debug/lx-tui command create mcps --kind agentMD --description "test" --content "body" -C "$VERIFY_DIR/ws"
     ```
- **期望结果**：
  - 两条命令均被拒绝，返回退出码 `1`，错误信息分别为 `lx-tui: invalid command name: mcp is reserved` 与 `lx-tui: invalid command name: mcps is reserved`。
- **实际验证结果（PASS）**：
  ```text
  lx-tui: invalid command name: mcp is reserved
  lx-tui: invalid command name: mcps is reserved
  ```

---

### 验证项 V2：User / Project 双作用域 MCP 注册与全局设置禁用（`Disabled in settings` 基线）

- **输入内容**：
  - User 作用域启用服务：`alpha`（`stdio: npx -y @modelcontextprotocol/server-memory`）
  - Project 作用域启用服务：`beta`（`http: https://mcp.example.com/sse`）
  - User 作用域全局禁用服务：`gamma`（`stdio: echo disabled-srv`，执行 `mcp disable gamma`）
- **验证步骤**：
  1. 通过 `lx-tui mcp` CLI 注册 3 个服务并将 `gamma` 设为全局禁用：
     ```bash
     ./target/debug/lx-tui mcp add-stdio alpha --command npx --arg -y --arg @modelcontextprotocol/server-memory -C "$VERIFY_DIR/ws"
     ./target/debug/lx-tui mcp add-http beta --url https://mcp.example.com/sse --scope project -C "$VERIFY_DIR/ws"
     ./target/debug/lx-tui mcp add-stdio gamma --command echo --arg disabled-srv -C "$VERIFY_DIR/ws"
     ./target/debug/lx-tui mcp disable gamma -C "$VERIFY_DIR/ws"
     ```
  2. 执行 `./target/debug/lx-tui mcp list -C "$VERIFY_DIR/ws"` 查看合并后的工作区 MCP 列表。
- **期望结果**：
  - `alpha`（`user`）与 `beta`（`project`）的 `ENABLED` 列为 `yes`；
  - `gamma`（`user`）的 `ENABLED` 列为 `no`（对应 TUI `/mcp` 面板中的 `[Disabled in settings]`）。
- **实际验证结果（PASS）**：
  ```text
  NAME   SCOPE    TRANSPORT  ENABLED  TARGET                                      OVERRIDES
  alpha  user     stdio      yes      npx -y @modelcontextprotocol/server-memory  -
  beta   project  http       yes      https://mcp.example.com/sse                 -
  gamma  user     stdio      no       echo disabled-srv                           -
  ```

---

### 验证项 V3：窗格级禁用投影与磁盘配置零副作用（`lx-tui mcp list --disable-mcp`）

- **输入内容**：
  - 窗格级禁用参数：`--disable-mcp alpha`
  - 已配置服务：`alpha`（启用）、`beta`（启用）、`gamma`（全局禁用）
- **验证步骤**：
  1. 执行带 `--disable-mcp alpha` 的人类可读表格输出与 `--json` 结构化输出：
     ```bash
     ./target/debug/lx-tui mcp list --disable-mcp alpha -C "$VERIFY_DIR/ws"
     ./target/debug/lx-tui mcp list --disable-mcp alpha --json -C "$VERIFY_DIR/ws"
     ```
  2. 执行 `./target/debug/lx-tui mcp show alpha -C "$VERIFY_DIR/ws" --json` 核查磁盘 `mcp.json` 中 `alpha` 的持久化状态。
- **期望结果**：
  - 表格中 `alpha` 的 `ENABLED` 列显示为 `no (pane)`，`beta` 保持 `yes`，`gamma` 保持 `no`；
  - JSON 输出中 `alpha` 的 `"enabled": true`、`"paneDisabled": true`、`"effectiveEnabled": false`；
  - `mcp show alpha --json` 中 `"enabled": true` 保持不变，证明窗格级禁用不改写磁盘 `mcp.json`。
- **实际验证结果（PASS）**：
  ```text
  NAME   SCOPE    TRANSPORT  ENABLED    TARGET                                      OVERRIDES
  alpha  user     stdio      no (pane)  npx -y @modelcontextprotocol/server-memory  -
  beta   project  http       yes        https://mcp.example.com/sse                 -
  gamma  user     stdio      no         echo disabled-srv                           -
  ```
  `mcp show alpha --json` 输出确认 `"enabled": true` 未被修改。

---

### 验证项 V4：无头单轮执行支持按轮次禁用指定 MCP（`lx-tui exec --disable-mcp`）

- **输入内容**：
  - CLI 参数：`lx-tui exec --disable-mcp alpha,beta --disable-mcp=gamma "run check"`
  - 异常输入：`lx-tui exec --disable-mcp ""`、`lx-tui exec --disable-mcp "bad name"`
- **验证步骤**：
  1. 执行 `./target/debug/lx-tui exec --help` 与 `./target/debug/lx-tui help exec resume` 检查 CLI 帮助文档；
  2. 运行 CLI 参数解析与校验测试：
     ```bash
     rtk cargo test chat::agent::exec::tests::cli_tests::exec_parses_disable_mcp_flags_and_rejects_invalid_names
     ```
- **期望结果**：
  - `exec --help` 与 `exec resume --help` 均展示 `--disable-mcp <NAME>` 选项说明；
  - 逗号分隔与多次传入的 `--disable-mcp` 去重合并为 `["alpha", "beta", "gamma"]`，空名称与非法字符名称返回明确错误。
- **实际验证结果（PASS）**：
  - `lx-tui exec --help` 正确展示 `--disable-mcp <NAME>          Disable MCP server(s) for this turn only (repeatable/comma-separated)`；
  - 单元测试 `exec_parses_disable_mcp_flags_and_rejects_invalid_names` 通过。

---

### 验证项 V5：`McpToolSnapshot::without_servers` 对工具、系统提示与 Resources 的裁剪隔离

- **输入内容**：
  - 包含 `github`（含工具 `mcp__github__list_issues`、instructions、`has_resources: true`）与 `codegraph`（含工具 `mcp__codegraph__explore`、instructions、`has_resources: false`）的 `McpToolSnapshot`
  - 窗格禁用集合：`{"github"}`
- **验证步骤**：
  1. 运行快照裁剪测试：
     ```bash
     rtk cargo test mcp::manager::tests::snapshot_without_servers_filters_tools_prompts_and_resources
     ```
- **期望结果**：
  - 当禁用集合为空时，`without_servers` 零拷贝返回原 `Arc`（`Arc::ptr_eq` 为 `true`）；
  - 当禁用 `github` 后：
    - `filtered.tool("mcp__github__list_issues")` 为 `None`，仅保留 `mcp__codegraph__explore`；
    - `filtered.server("github")` 为 `None`，`filtered.has_resources()` 变为 `false`（阻止 `read_resource` 访问被禁用的 server）；
    - `filtered.instructions_prompt()` 不再包含 `### github` 段落，仅保留 `### codegraph`。
- **实际验证结果（PASS）**：
  - `mcp::manager::tests::snapshot_without_servers_filters_tools_prompts_and_resources` 通过。

---

### 验证项 V6：多窗格（Pane）独立开关隔离、`Space`/`Enter` 连续切换与跨 `/new` 会话保持

- **输入内容**：
  - 同一工作区下的两个分屏窗格 `pane1` 与 `pane2`
  - 在 `pane1` 执行 `/mcp` 打开面板，按 `Space` / `Enter` 切换 `alpha`，随后执行 `/new` 新建会话，再从 `pane1` 分屏出 `pane3`
- **验证步骤**：
  1. 运行窗格状态更新与隔离测试：
     ```bash
     rtk cargo test app::update::chat::tests::slash_panels::mcp_panel_
     ```
- **期望结果**：
  - 在 `pane1` 禁用 `alpha` 后，`pane1` 的首页计数 `startup.mcp.total` 从 `2` 变为 `1`、`@` 提及 `filtered_mcp_servers()` 排除 `alpha`、面板内状态变为 `[Disabled]` 且**面板保持打开**；
  - 同一工作区的 `pane2` 完全不受影响（`startup.mcp.total == 2`，`@` 提及仍含 `alpha`）；
  - 在 `pane1` 执行 `/new` 开启新会话后，`alpha` 仍保持禁用；而从 `pane1` 新分屏出的 `pane3` 默认全开（`disabled_mcp_servers` 为空）；
  - 对 `Failed` 条目按 `Enter` 发出 `Effect::RestartMcpServer` 并立即切换为 `Connecting`；对 `DisabledInSettings` 条目按 `Space` 在窗格内弹出警告 Toast 且状态不变。
- **实际验证结果（PASS）**：
  - `mcp_panel_toggles_per_pane_without_affecting_sibling_pane`、`mcp_panel_space_and_enter_keep_panel_open_and_persist_across_new_session`、`mcp_panel_failed_entry_triggers_restart_and_settings_disabled_shows_warning` 全部通过。

---

### 验证项 V7：后台 `McpEvent::Status` 实时刷新已打开的 `/mcp` 面板并按窗格抑制错误 Toast

- **输入内容**：
  - `pane1` 已禁用 `beta` 并打开 `/mcp` 面板，`pane2` 保持 `beta` 启用
  - 后台通道推入：`McpEvent::Status { server: "alpha", status: Ready { tools: 5 } }` 与 `McpEvent::Status { server: "beta", status: Failed { error: "timeout" } }`
- **验证步骤**：
  1. 运行事件层库存同步测试：
     ```bash
     rtk cargo test event::inventory::tests::drain_mcp_events_updates_open_mcp_panel_and_suppresses_toast_on_disabled_pane
     ```
- **期望结果**：
  - `pane1` 已打开的 `/mcp` 面板实时将 `alpha` 更新为 `Connected { tools: 5 }`（预览 `5 tools · stdio: alpha`），`beta` 仍保持 `DisabledInPane`；
  - `beta` 的启动失败 Toast 被抑制在已禁用它的 `pane1` 之外（`pane1.lx.toast.is_none()`），而在仍启用 `beta` 的 `pane2` 内正常弹出（`pane2.lx.toast.is_some()`）。
- **实际验证结果（PASS）**：
  - `drain_mcp_events_updates_open_mcp_panel_and_suppresses_toast_on_disabled_pane` 通过。

---

### 验证项 V8：`/mcp` 内联面板 UI 渲染与 `[Disabled in settings]` 置灰样式

- **输入内容**：
  - 包含 `codegraph`（`Connected { tools: 2 }`）、`memory`（`DisabledInPane`）、`github`（`DisabledInSettings`）三行的 `SlashPanel::Mcp`
- **验证步骤**：
  1. 运行 TUI 渲染缓冲区测试：
     ```bash
     rtk cargo test ui::main_content::lx::tests::slash_panels::mcp_panel_renders_statuses_and_dims_settings_disabled_rows
     ```
- **期望结果**：
  - 面板紧贴输入框上方无边框渲染，顶栏显示 `MCP servers`，底栏显示 `↑↓ select · space/↵ toggle · esc close`；
  - 右侧对齐渲染 `[Connected]`、`[Disabled]`、`[Disabled in settings]` 标签；
  - `DisabledInSettings` 行使用 `style::muted()` 置灰前景渲染。
- **实际验证结果（PASS）**：
  - `mcp_panel_renders_statuses_and_dims_settings_disabled_rows` 通过。
