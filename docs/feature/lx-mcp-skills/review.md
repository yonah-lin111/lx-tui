# lx MCP 与 Skills — 功能评审（Review）

> 审查对象：`feat/lx-mcp-skills`（相对 `dev` 基线 `e9cf47d6`，现已合并入 `dev`）。
> 关联文档：[design.md](design.md)、[task.md](task.md)、[verification.md](verification.md)。
> 状态：**评审完成，P0 ~ P2 全部缺陷已修复，功能已合并到 `dev`，worktree 与分支已清理**（2026-10-30）。

## 0. 评审结论

MCP 客户端/管理器、OAuth 发现与 PKCE 登录、工具结果图片跨四类协议编码、会话图片目录搬迁（v20）、Skills 四根发现与 `.agents` 默认禁用、设置页统一保存以及 TUI 首页计数/卡片折叠的主干架构成立。

结合 `codegraph` 与 `codebase-memory-mcp` 追踪核心数据流后，发现问题主要集中在以下四类边界：

1. **多工作区与异步 Git 归位联动**：linked worktree 在 `GitRefreshed` 到达后改变 `repo_key`，导致首页与 `@` 面板的 Skills 计数被重置为 `0`；
2. **MCP 协议错误与取消边界**：resources 三工具漏接 `Esc` 取消与超时保护；普通工具调用收到 JSON-RPC 业务错误（如 `-32602`）时误杀整个连接并把 server 永久标为 `Failed`；`tools/list_changed` 与失败断连未同步刷新 `snapshot_cache`；
3. **跨轮上下文与卡片状态机**：子 agent 历史轮次漏做工具图片 3 轮裁剪与每张图 1600 tokens 估算；Gemini 漏调 `trim_for_gemini`；MCP 卡片在 `Running`/`Failed` 阶段丢失类型化视图退化为裸名文件卡，且按 `^o` 误打开后台进程面板；
4. **设置页前端状态与工程规范**：MCP/Skills 编辑器切换条目漏刷脏状态、硬编码覆盖用户自定义超时、只读 `.agents` 未锁 `Scope` 下拉框、重复创建 `.select-menu` 泄漏 DOM，以及部分生产文件超 1000 行、新增模块内联 `mod tests`。

以下按 **P0 → P1 → P2** 优先级排序列出关键点、触发条件、错误原因与修复实现。

---

## 1. 核心数据流（Codegraph 梳理）

```text
启动 / 工作区变化 / GitRefreshed
  event::run_loop
    ├─ chat::mcp::project_roots(state) ──▶ inventory::initialize_mcp ──▶ McpManager::prewarm
    │     └─ McpEvent::{Status, StartupProgress} ──▶ inventory::drain_mcp_events ──▶ AppState.mcp_inventory
    ├─ inventory::skill_roots(state)   ──▶ inventory::spawn_skills_preload (spawn_blocking)
    │     └─ AppEvent::SkillsLoaded ──▶ inventory::apply_skills_loaded ──▶ AppState.skill_inventory + pane.lx.mention_skills
    └─ AppState::sync_lx_inventory()（每帧同步到所有窗格的 pane.lx.startup）

聊天轮次（主会话 / 子 agent）
  start_stream / launch_child_turn
    ├─ skills::catalog_prompt(config_root) ──▶ ## Skills（8000 字符预算）
    ├─ skills::reference_blocks(draft)     ──▶ <skill> 当轮全文注入（≤8000 字节）
    ├─ mcp::snapshot_for_turn(config_root) ──▶ McpToolSnapshot（冻结 tools + ## MCP servers）
    ├─ context::assemble / assemble_child_context
    │     ├─ 历史工具图片裁剪（最近 3 个含图轮次 / 不支持图片模型全部降级文本占位）
    │     └─ turn_tools(surface, &mcp_snapshot) ──▶ ChatRequest.tools: Arc<Vec<ToolDecl>>
    └─ run_agent_turn
          ├─ execute_mcp_cancellable（mcp__* + list_resources / list_resource_templates / read_resource）
          │     └─ McpManager::send_cancellable（tool_timeout_sec + Esc notifications/cancelled）
          └─ FinishTool ──▶ result_json 落盘（图片归档 session/<sid>/image/）──▶ McpDetailView
```

---

## 2. 问题清单与根因分析（P0 – P2）

### P0（阻断性缺陷：功能失效、轮次挂死、状态丢失）

#### P0-1：linked worktree 在 `GitRefreshed` 后或打开多工作区时，Skills 计数被重置为 `0 skills` 且 `@` 面板丢失项目 skill

- **关键点**：`src/event/mod.rs`（`run_loop` 资源装载触发）、`src/event/inventory.rs`（`spawn_skills_preload` / `apply_skills_loaded`）、`src/app/state.rs`（`sync_lx_inventory`）。
- **触发条件**：
  1. 在任意 linked git worktree（如 `.worktrees/lx-mcp-skills`）下启动 `lx`；
  2. 打开多个不同仓库的工作区，或钉住右侧 Prompt 面板（`prompt_pinned = Some(_)`）后切换活动工作区。
- **错误原因**：
  1. 启动首帧 `workspace.git` 尚未由后台 `git worktree list` 解析完成，此时 `workspace.repo_key()` 返回 worktree 自身路径 `Some(worktree_cwd)`，`spawn_skills_preload` 将扫描结果写入 `skill_inventory[Some(worktree_cwd)]`。约 20ms 后 `AppEvent::GitRefreshed` 到达，`workspace.repo_key()` 变为仓库主 checkout `Some(repo_root)`。事件循环对 MCP 用 `if roots != mcp_roots` 触发了补预热，但对 Skills 只检查了 `prompt_workspace != last_prompt_workspace`（工作区索引仍为 `0`），未触发重扫。下一帧 `sync_lx_inventory()` 按新键 `Some(repo_root)` 查 `skill_inventory` 得到 `None`，直接把窗格的 `startup.skills` 覆写回 `0 skills`，且 `@` 面板拿不到主仓库的 skills。
  2. `spawn_skills_preload` 原实现只扫描 `state.prompt_owner().repo_key()` 单个根，未遍历所有已打开工作区的 `repo_key()`。
- **修复实现**：
  - 在 `src/event/inventory.rs` 新增 `skill_roots(state)` 收集所有已打开工作区的去重 `repo_key()`；
  - `spawn_skills_preload` 遍历全部 `skill_roots(state)`（对已有条目保持当前展示不闪烁回 `loading`，仅对未装载的新根置 `loading = true`）；
  - 在 `src/event/mod.rs` 跟踪 `skill_roots` 变化，当 `GitRefreshed` 或新建工作区改变根集合时立即置 `skills_reload = true`；
  - 新增单元测试 `event::inventory::tests::skill_roots_tracks_git_refresh_and_multiple_workspaces`。

#### P0-2：MCP `list_resources` / `list_resource_templates` / `read_resource` 不响应 `Esc` 取消且无调用超时

- **关键点**：`src/chat/agent_loop/executor.rs`（`execute_mcp_cancellable` / `execute_mcp_resource`）、`src/mcp/manager.rs`（`list_resources_inner` / `read_resource`）、`src/mcp/mod.rs`。
- **触发条件**：模型调用 `list_resources`、`list_resource_templates` 或 `read_resource` 时目标 MCP server 响应缓慢或挂起，用户按 `Esc` 取消。
- **错误原因**：
  1. `execute_mcp_cancellable` 仅用 `crate::mcp::is_mcp_tool(&call.name)`（只匹配 `mcp__*` 前缀）判断是否进入可取消执行路径，导致三个 resources 工具回落到无取消通道的 `execute_tracked`；
  2. `McpManager::list_resources_inner` 与 `read_resource` 直接 `await` `connection.client.list_resources(...)` / `read_resource(...)`，既未通过 `send_cancellable_request` 监听 `cancel.changed()` 发送 `notifications/cancelled`，也未施加 `tool_timeout_sec` 超时限制；
  3. `execute_mcp_resource` 将 `duration_ms` 传为 `None`，导致 resources 工具卡片永远缺失耗时显示。
- **修复实现**：
  - 在 `src/mcp/mod.rs` 新增 `is_mcp_or_resource_tool`；
  - `execute_mcp_cancellable` 覆盖三个 resources 工具并将 `cancel` 传入 `execute_mcp_resource`，同时记录真实执行耗时 `duration_ms`；
  - `McpManager` 抽取出共用的 `send_cancellable` 方法，`call_tool`、`list_resources_inner`、`read_resource` 统一走 `send_cancellable_request` + `tool_timeout_sec` + `cancel.changed()`（取消时下发 `notifications/cancelled` 并返回 `McpCallOutcome::Cancelled`）；
  - 在 `src/mcp/manager/integration_tests.rs` 的 `resources_are_listed_and_read_from_the_fixture_server` 增加慢资源读取取消断言。

#### P0-3：MCP 工具调用返回 JSON-RPC 业务错误（如 `-32602 Invalid params`）时误杀整个连接并永久标为 `Failed`

- **关键点**：`src/mcp/manager.rs`（`call_tool` / `send_cancellable` / `mark_failed`）。
- **触发条件**：模型向 MCP 工具传入了不符合预期类型的参数或触发单次调用级 JSON-RPC 错误（`rmcp::ServiceError::McpError`）。
- **错误原因**：原 `call_tool` 在 `Ok(Err(error))` 分支未区分 JSON-RPC 协议层业务错误（`ServiceError::McpError`，此时底层 stdio/HTTP 传输完全健康）与传输断开错误（`TransportClosed` / `TransportSend` 等），无条件调用 `self.mark_failed(...)`。`mark_failed` 会立即调用 `connection.stop()` 终止 MCP 子进程、把状态改为不自动重试的 `ServerStatus::Failed`，并在 TUI 弹出 `"MCP server \"<name>\" failed to start: ..."` 错误 toast，导致该 server 在后续轮次中彻底瘫痪。
- **修复实现**：
  - 在 `McpManager::send_cancellable` 中显式匹配 `Ok(Err(rmcp::ServiceError::McpError(error)))`，仅返回当次工具级失败 `McpCallOutcome::Failure { kind: McpFailureKind::Server, .. }`，不调用 `mark_failed`、不停止连接；
  - 仅在传输发送失败、连接关闭或其它传输异常时调用 `fail_transport` / `mark_failed`；
  - 新增集成测试 `mcp::manager::integration_tests::rpc_error_from_tool_call_keeps_server_ready`。

---

### P1（主要缺陷：上下文语义偏差、跨轮缓存失步、UI 与设置页交互错误）

#### P1-1：子 agent 历史上下文未裁剪超过 3 轮的工具图片、未在纯文本模型下剥离工具图片，且漏算每张工具图片 1600 tokens

- **关键点**：`src/event/chat/subagent/context.rs`（`plan_child_historical_turns` / `estimate_items_tokens`）、`src/chat/context/mod.rs`（`trim_tool_images_in_turn_items`）。
- **触发条件**：子 agent 连续多轮调用返回图片的工具（或 `read` 读取本地图片）超过 3 轮，或会话中途切换为不支持图片输入的模型。
- **错误原因**：
  1. `subagent/context.rs::plan_child_historical_turns` 调用 `assistant_items` 还原历史工具图片后，未执行主会话 `context/mod.rs` 中的「只保留最近 3 个含工具图片轮次」与「`!supports_image_input` 清空全部工具图片并追加占位」逻辑（违反 `design.md` §5.4 第 5 条）；
  2. `subagent/context.rs::estimate_items_tokens` 在 `ModelItem::ToolResult` 分支忽略了 `images` 字段，每张工具图片少算 1600 tokens，导致子 agent 上下文预算裁剪与自动压缩阈值失真。
- **修复实现**：
  - 在 `src/chat/context/mod.rs` 抽出共用函数 `trim_tool_images_in_turn_items`，由主会话与子 agent 历史规划共同调用；
  - 在 `subagent/context.rs::estimate_items_tokens` 的 `ToolResult` 分支补上 `(images.len() as u64) * 1600`。

#### P1-2：Google Gemini 工具声明编码未对动态工具调用 `trim_for_gemini`

- **关键点**：`src/chat/transport/tools/mod.rs`（`google_tools`）、`src/mcp/schema.rs`（`trim_for_gemini`）。
- **触发条件**：使用 Gemini 模型并启用含 `additionalProperties`、`$ref`、`minLength` 等 JSON Schema 关键字的 MCP 工具或 resources 工具。
- **错误原因**：`src/mcp/schema.rs` 实现了 `trim_for_gemini` 并公开导出，但 `google_tools` 直接透传了 `tool.parameters`，生产代码无任何调用点（违反 `design.md` §4.5 第 9 条）。
- **修复实现**：
  - 在 `google_tools` 中对动态工具（`crate::mcp::is_mcp_or_resource_tool(&tool.name)`）调用 `crate::mcp::trim_for_gemini(&tool.parameters)`，内置工具保持原样以维持无图片 wire 快照逐字节一致；
  - 新增单元测试 `chat::transport::google::tests::google_tools_trims_unsupported_json_schema_keywords_for_mcp_tools`。

#### P1-3：`tools/list_changed`、单 server 启动完成与连接失败未及时同步 `McpManager::snapshot_cache`，且 OAuth 刷新分支悬空在 `Starting`

- **关键点**：`src/mcp/manager.rs`（`LxClientHandler::on_tool_list_changed`、`finish_start`、`mark_failed`、`restart`、`refresh_cached_snapshot`）。
- **触发条件**：
  1. MCP server 发送 `notifications/tools/list_changed` 后用户发起下一轮对话；
  2. 同一项目配置了快慢多个 server，快 server 已就绪而慢 server 仍在启动；
  3. OAuth HTTP server 调用失败触发 `mark_failed` 内的 `oauth::refresh`。
- **错误原因**：
  1. `snapshot_for_turn` 同步读取 `cached_snapshot(root)`，而 `tools_changed` 原先只在异步 `snapshot_for` 尾部消费，导致紧随其后的首轮拿到旧缓存；且 `finish_start`、`mark_failed`、`restart` 均未刷新 `snapshot_cache`；
  2. `mark_failed` 在 `oauth::refresh` 成功后把 `slot.status` 设为 `ServerStatus::Starting` 并丢弃旧连接，却没有发起重连，导致槽位永久悬空在 `Starting`、首页卡在 `loading mcp`。
- **修复实现**：
  - 抽出 `McpManager::refresh_cached_snapshot`，在 `finish_start`、`mark_failed`、`restart`、`snapshot_for` 及 `LxClientHandler::on_tool_list_changed` 回调中即时刷新 `snapshot_cache`；
  - `mark_failed` 在 OAuth `refresh` 成功后立即尝试用新凭据重连，成功则置 `Ready`、失败则转入 `NeedsLogin` / `Failed`，不再遗留无任务的 `Starting` 状态；
  - 补强 `mcp::manager::integration_tests::tool_list_changed_replaces_the_connection_tools_for_later_snapshots`，真实改写工具列表文件并校验 `cached_snapshot` 已同步替换。

#### P1-4：MCP 工具在 `Pending` / `Running` / `Failed` / `Cancelled` / `Interrupted` 阶段丢失类型化卡片，且按 `^o` 误打开后台任务面板

- **关键点**：`src/app/state/lx/chat/tool_view/mcp.rs`、`src/app/state/lx/chat/steps.rs`（`accept_arguments`）、`src/app/update/chat/view.rs`、`src/app/update/chat/process.rs`（`settle_tool_detail` / `open_task_for_selected_tool`）、`src/ui/main_content/lx/history/tool/mcp.rs`。
- **触发条件**：
  1. MCP 工具处于运行中（`Pending` / `Running`）或以失败/超时/中断/取消结束；
  2. 焦点窗格选中或最近一张可展开卡片为 MCP 工具卡片时按 `Ctrl+O`。
- **错误原因**：
  1. `ToolCallView::accept_arguments` 只为子 agent 工具预构造 `detail`，未为 MCP 与 resources 工具预构造 `McpDetailView`；且 `McpDetailView::from_parts` 只认 `"kind": "mcp"` 的成功结果 JSON，失败时 `settle_tool_detail` 还会用 `None` 覆盖已有 `detail`。导致运行中与失败态的 MCP 工具回落到通用文件卡片，标题退化为裸 `■ mcp__server__tool`，参数块 `├─` 消失，失败原因也无法在 MCP 卡片的 `└─` 结果区展示；
  2. `open_task_for_selected_tool`（`^o`）只豁免了 `file_view()`，对 `Mcp`（及 `Search` / `Subagent`）卡片会调用 `open_task_page_at(state, pane, None)` 误打开后台进程面板。
- **修复实现**：
  - 新增 `McpDetailView::from_arguments` 与 `McpDetailView::from_tool`，在 `accept_arguments` 与 `view.rs` 历史重载中为 `mcp__*` 及 resources 三工具挂载 `McpDetailView`；`settle_tool_detail` 与 `on_subagent_tool_settled` 在失败结果 `detail == None` 时保留已有的 `Mcp` / `Subagent` 视图；
  - `ui::main_content::lx::history::tool::mcp::result_section` 在 `Failed` / `Interrupted` 且无结果正文时渲染 `call.error`；
  - `open_task_for_selected_tool` 对 `Mcp` / `Search` / `Subagent` 卡片直接执行 `toggle_tool_detail`；
  - 新增测试 `running_and_failed_mcp_cards_keep_formatted_header_and_arguments` 与 `ctrl_o_on_mcp_card_toggles_section_instead_of_opening_task_page`。

#### P1-5：设置页 MCP / Skills 切换条目脏状态残留、保存覆盖自定义超时、只读 `.agents` 未锁 `Scope` 以及 `.select-menu` DOM 泄漏

- **关键点**：`assets/settings.js`（`createSelect`）、`assets/settings_mcp.js`（`showMcpEditor` / `buildMcpConfig`）、`assets/settings_skills.js`（`showSkillEditor` / `skillDirty` / `setSkillEnabled`）。
- **触发条件**：
  1. 在 MCP 页修改某 server 后未保存，点击左侧另一 server 或 `+ New server`；
  2. 编辑配置了非默认 `startup_timeout_sec` / `tool_timeout_sec` / `enabled_tools` 的 MCP server 并保存；
  3. 在 Skills 页查看 `.agents (read-only)` skill 时修改 `Scope` 下拉框，或在左右两侧切换启用复选框；
  4. 多次点击不同 MCP server 或 skill。
- **错误原因**：
  1. `showMcpEditor` 重置 `mcpBaseline` 后未调用 `refreshActionBars()`，导致 tab 黄点与 `Unsaved changes` 状态残留；
  2. `buildMcpConfig()` 写死 `startup_timeout_sec: 30, tool_timeout_sec: 300` 且丢弃 `enabled_tools`；
  3. `showMcpEditor` / `showSkillEditor` 在编辑已有条目时未禁用 `Scope` 选择器；`skillDirty()` 未排除只读 `.agents` 条目；`#skill-enabled` 在新建模式未禁用，且左右两侧启用复选框切换后未双向同步；
  4. `createSelect` 每次调用都在 `document.body` 上追加新的 `.select-menu`，重绑同一 `host` 时未移除旧节点。
- **修复实现**：
  - `createSelect` 支持 `disabled` 配置并在重绑同一 `host` 时清理旧 `.select-menu` 节点与 `openSelects` 引用；
  - `showMcpEditor` 末尾调用 `refreshActionBars()`，非新建模式禁用 `Scope`；`buildMcpConfig()` 保留已有 server 的 `startup_timeout_sec`、`tool_timeout_sec` 与 `enabled_tools`；
  - `showSkillEditor` 非新建模式禁用 `Scope`、新建模式禁用 `#skill-enabled`；`skillDirty()` 对只读条目恒返回 `false`；`setSkillEnabled` 同步刷新列表与右侧复选框。

---

### P2（代码规范、模块体积与文档一致性）

#### P2-1：受影响生产文件超过 1000 行上限

- **关键点**：`src/event/mod.rs`（原 1151 行）、`src/event/chat/mod.rs`（原 1069 行）、`src/app/state/lx/chat/tool_view.rs`（原 1018 行）、`src/app/update/chat/mod.rs`（原 1005 行）、`src/mcp/manager.rs`（原 989 行）。
- **错误原因**：在本分支向事件循环、聊天轮次启动与工具详情视图追加 MCP/Skills 逻辑时直接堆入原文件，超出 `AGENTS.md` 规定的生产文件 1000 行硬上限。
- **修复实现**：
  - `src/event/mod.rs`：将设置页/MCP 事件排空与项目窗格查找下沉至 `src/event/inventory.rs`，将 prompt worktree 与引用目录后台请求下沉至 `src/event/commands.rs`（`src/event/mod.rs` 降至 **955 行**）；
  - `src/event/chat/mod.rs`：将用户图片字节按需装配 `hydrate_item_images` 下沉至 `src/event/chat/timeline.rs`（`src/event/chat/mod.rs` 降至 **995 行**）；
  - `src/app/state/lx/chat/tool_view.rs`：将 `McpDetailView` / `McpImageView` 拆至子模块 `src/app/state/lx/chat/tool_view/mcp.rs`（`tool_view.rs` 降至 **869 行**）；
  - `src/app/update/chat/mod.rs`：移除重复的 `warnings` 提示分支（降至 **999 行**）；
  - `src/mcp/manager.rs`：将 resources 三工具拆至 `src/mcp/manager/resources.rs`，抽取 `send_cancellable` 与 `refresh_cached_snapshot` 并外移单元测试（`manager.rs` 降至 **801 行**）。

#### P2-2：新增模块将单元测试内联在生产文件中，未分离到 `tests.rs`

- **关键点**：`src/mcp/manager.rs`、`src/mcp/oauth.rs`、`src/mcp/result.rs`、`src/mcp/schema.rs`、`src/mcp/naming.rs`、`src/platform/mcp_process.rs`、`src/platform/path_text.rs`、`src/ui/main_content/lx/history/tool/mcp.rs`、`src/chat/context/mod.rs`。
- **错误原因**：新增模块直接将 `#[cfg(test)] mod tests { ... }` 写在生产文件末尾，违反 `docs/standards/code-writing-standards.md`「主文件只保留 `#[cfg(test)] mod tests;` 声明，测试放同层或同名子目录 `tests.rs`」规范。
- **修复实现**：将上述 9 处内联测试全部拆入对应的 `<module>/tests.rs`（`chat/context/mod.rs` 的 `tool_image_replay_tests` 并入 `src/chat/context/tests/tools.rs`），生产文件仅保留 `#[cfg(test)] mod tests;`。

#### P2-3：`try_send` 对同一 skill 警告重复触发两次 `show_lx_hint`

- **关键点**：`src/app/update/chat/mod.rs`（`try_send`）、`src/chat/prepare.rs`。
- **错误原因**：`prepare_user_message` 同时填充 `warnings` 与结构化 `skill_notices`，`try_send` 先后对 `warnings.first()` 和 `skill_notices.first()` 调用 `show_lx_hint` 写同一个 `pane.lx.toast` 槽位，前一次写入被立即覆盖。
- **修复实现**：`try_send` 仅通过结构化 `skill_notices` + `ui::text::skills` 渲染 pane 内 toast。

#### P2-4：feature 文档仅在主 checkout 未跟踪目录且与最新实现存在漂移

- **关键点**：`docs/feature/lx-mcp-skills/{design.md,task.md,verification.md,review.md}`。
- **错误原因**：
  1. 文档最初建在主工作区，未放入 `.worktrees/lx-mcp-skills/docs/feature/lx-mcp-skills/`；
  2. `design.md` 仍保留初稿设定（`SCHEMA_VERSION = 19`、`PROMPT_VERSION = lx-6`、skill 在 `/` 面板、`SKILL.md` 区分大小写、`.agents` 默认开启、中文 toast、设置页内联 Save 按钮），未同步后续迭代决策。
- **修复实现**：修订 `design.md` 与 `task.md`，编写本 `review.md`，并将四份文档同步放置在 worktree 与主工作区的 `docs/feature/lx-mcp-skills/` 下。

---

## 3. 已复核确认的不变量

1. **模块依赖边界**：`src/mcp/` 与 `src/skills/` 不依赖 `chat`/`app`/`ui`/`event`/`settings`；`rmcp` 仅在 `src/mcp/` 引入；`app/` 无 IO，渲染层 `ui/` 只读状态。
2. **turn 内快照冻结**：`ChatRequest.tools` 与系统提示在 turn 起点冻结，`ModelContext::next_request` 跨多步采样复用同一份。
3. **工具越权拦截**：执行层按本轮冻结的 `context.request.tools` 校验工具名，未声明工具一律拒绝。
4. **工具图片归档与隐私**：图片字节归档到 `~/.lx-t[-dev]/session/<sid>/image/`，SQLite `result_json` 只存路径与尺寸；MCP `bearer_token` / `client_secret` / OAuth token 不进日志、toast、`GET /api/mcp` 响应与模型上下文。
5. **模型与界面文案语言**：模型可见文本与 TUI / 浏览器设置页所有用户可见文案均为英文。

---

## 4. 验证结果

- `cargo fmt`：通过。
- `cargo clippy --all-targets -- -D warnings`：通过（0 warning）。
- `cargo test`：全量通过（含新增的 `rpc_error_from_tool_call_keeps_server_ready`、`resources_are_listed_and_read_from_the_fixture_server` 取消断言、`google_tools_trims_unsupported_json_schema_keywords_for_mcp_tools`、`running_and_failed_mcp_cards_keep_formatted_header_and_arguments`、`ctrl_o_on_mcp_card_toggles_section_instead_of_opening_task_page`、`skill_roots_tracks_git_refresh_and_multiple_workspaces`）。
- `node --check assets/settings.js assets/settings_mcp.js assets/settings_skills.js`：全部通过。
- 仍需人工确认项：运行 `cargo run` 复核 TUI 首页计数间距、分屏窗格计数、MCP 工具卡片独立折叠点击面及浏览器设置页实际交互（详见 [verification.md](verification.md)）。
