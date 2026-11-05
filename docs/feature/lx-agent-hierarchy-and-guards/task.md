# lx 子 Agent 多级树、资源护栏、历史清洗与 TUI 增强 — 实施任务拆解 (`task.md`)

- **关联设计**：[`design.md`](design.md)
- **执行前提**：用户审核确认本 `design.md` 与 `task.md` 后，按以下任务顺序逐步落地并执行精准校验。

---

## 任务总览

| 任务 ID | 对应模块 | 任务名称 | 涉及核心文件 | 状态 |
| :--- | :--- | :--- | :--- | :--- |
| **T1** | 模块 D & F | 领域层状态机 `AgentStatus`、层级路径 `AgentPath` 与预算器 `SubagentTokenBudget` | `src/chat/agent/{status.rs, path.rs, budget.rs, mod.rs}` | 已完成 |
| **T2** | 模块 A, B, C, F | 控制工具契约、参数解析、Schema 与报告结构扩展 | `src/chat/agent/{parse.rs, report.rs}`、`src/chat/tools/{subagent.rs, schema.rs}`、`src/config/subagents.rs` | 已完成 |
| **T3** | 模块 A, C, F | 数据库分叉历史清洗 (`fork_turns`)、树溯源查询与变更文件列表 | `src/db/chat/{fork.rs, subagent.rs}`、`src/db/chat/subagent/record.rs` | 已完成 |
| **T4** | 模块 B & F | `run_agent_turn` 预算阈值提醒、耗尽熔断与多级协调工具面门控 | `src/chat/agent_loop/{mod.rs, context.rs}`、`src/chat/context/mod.rs` | 已完成 |
| **T5** | 模块 B, C, F | 交互式事件层 & 无头 `exec` 池：多级寻址、整树并发、级联关闭、`trigger_turn` 与半程汇报 | `src/event/chat/subagent/{mod.rs, runner.rs, launch.rs, settle.rs, wait.rs}`、`src/chat/agent/exec/subagent/{mod.rs, turn.rs, settle.rs}` | 已完成 |
| **T6** | 模块 E | App 状态与 TUI 渲染：`wait_agent(all)` 实时列表、`spawn_agent` 实时活动、`Agents` 树缩进/Token/`^a` 与 `/subagents` | `src/app/state/lx/{agent.rs, slash.rs, chat/subagent_view.rs}`、`src/app/update/chat/{subagent.rs, subagent_cards.rs, task_page.rs, slash.rs}`、`src/ui/main_content/lx/{history/tool/subagent.rs, task/agents.rs}`、`src/ui/text/mod.rs` | 已完成 |
| **T7** | 模块 B & F | Web 设置页扩展（`maxDepth`、`maxConcurrent`、`defaultMaxTokens`） | `src/settings/service/subagents.rs`、`assets/{settings.html, settings_subagents.js}` | 已完成 |
| **T8** | 全模块 | 单元测试、CLI 实机验证、P0–P3 审查修复与 `verification.md` 报告 | `src/chat/agent/tests/`、`src/chat/agent/exec/tests/subagent_tests.rs`、`src/db/chat/subagent/tests.rs`、`src/event/chat/tests/subagent.rs`、`src/app/update/chat/tests/subagent.rs`、`docs/feature/lx-agent-hierarchy-and-guards/verification.md` | 已完成 |

---

## 详细任务步骤与关键代码检查点

### Task 1 (T1)：领域层 `AgentStatus`、`AgentPath` 与 `SubagentTokenBudget`
- **目标文件**：
  - 新增 `src/chat/agent/status.rs`
  - 新增 `src/chat/agent/path.rs`
  - 新增 `src/chat/agent/budget.rs`
  - 修改 `src/chat/agent/mod.rs`、`src/chat/agent/role.rs`
- **实施细节**：
  1. 在 `status.rs` 实现 `AgentStatus`（`Running`, `Idle`, `Blocked`, `Completed`, `Failed`, `Cancelled`, `Interrupted`）及 `projected_agent_status()` 纯函数；
  2. 在 `path.rs` 实现 `ROOT_AGENT_PATH` (`"/root"`)、`is_valid_path_segment`、`slugify_segment`、`path_depth`、`allocate_child_path`、`encode_stored_role_and_path` / `decode_stored_role_and_path` 以及 `DEPTH_LIMIT_REACHED_CODE`；
  3. 在 `budget.rs` 实现 `SubagentTokenBudget`（`record_step_usage`、25% / 10% 剩余阈值触发 `<rollout_budget>` 英文提醒）；
  4. 在 `mod.rs` 与 `role.rs` 新增 `ToolSurface::ExecutorCoordinator`（15 项工具：7 项 Executor 工具 + 8 项控制工具）与 `AgentRole::tool_surface_at_depth(depth, max_depth)`；
  5. 定义 `ForkSpec`（`None` / `AllCompleted` / `RecentTurns(u32)`）。
- **验证点**：`cargo test --lib chat::agent` 编译并全绿。

---

### Task 2 (T2)：控制工具参数解析、Schema、报告结构与配置模型扩展
- **目标文件**：
  - `src/config/subagents.rs`
  - `src/chat/agent/parse.rs`
  - `src/chat/agent/report.rs`
  - `src/chat/tools/subagent.rs`、`src/chat/tools/schema.rs`
- **实施细节**：
  1. **`src/config/subagents.rs`**：
     - `MAX_CONCURRENT_SUBAGENTS_LIMIT` 扩至 `12`，`DEFAULT_MAX_CONCURRENT_SUBAGENTS` 设为 `4`（兼容旧配置 `3`）；
     - `SubagentsSection` 新增 `max_depth: usize`（默认 `2`，校验范围 `1..=3`）与 `default_max_tokens: Option<u64>`（校验 `> 0`）。
  2. **`src/chat/agent/parse.rs` & `src/chat/tools/subagent.rs`**：
     - `spawn_agent` 参数白名单新增可选：
       - `task_name: Option<String>`（校验 `is_valid_path_segment`）；
       - `fork_turns: Option<u32>`（校验 `0..=64`）；
       - `max_tokens: Option<u64>`（校验 `>= 1000`）；
     - `send_input` 参数白名单新增可选 `trigger_turn: bool`（默认 `true`；当 `interrupt == true && !trigger_turn` 时返回 `ToolFailure::invalid`）；
     - `list_agents` 参数白名单新增可选 `path_prefix: Option<String>`；
     - `SubagentOutcome::Spawned`、`AgentListItemView`、`AgentStatusView` 新增 `agent_path: Option<String>`。
  3. **`src/chat/agent/report.rs`**：
     - `SubagentReportView` 新增 `pub changed_paths: Vec<String>`；
     - `format_report_envelope` 在 `<subagent_report>` 属性中包含 `path="..."`（若有），并在正文中输出 `Changed files: ...`；
     - 新增 `synthesize_partial_report_body` 供中断/预算耗尽时生成半程汇报正文。
- **验证点**：`cargo test --lib chat::agent::tests` 与 `cargo test --lib config::subagents` 全绿。

---

### Task 3 (T3)：数据库层分叉历史清洗 (`fork_turns`)、树查询与变更文件明细
- **目标文件**：
  - `src/db/chat/fork.rs`
  - `src/db/chat/subagent.rs`
  - `src/db/chat/subagent/record.rs`
- **实施细节**：
  1. **`src/db/chat/fork.rs`**：
     - 实现 `completed_fork_window(conn, session_id, fork_spec)`，支持截取最近 `N` 个已完成 `turn_index` 的消息范围 `[start_seq, boundary)`；
     - 在 `copy_completed_history_to_child` 中执行三项清洗：
       - 克隆消息的 `input_tokens / output_tokens / cache_read_tokens / cache_write_tokens` 全部写 `NULL`；
       - `copy_parts` 过滤 `p.kind NOT IN ('reasoning', 'thinking')`；
       - `subagent_report_json` 置 `NULL`。
  2. **`src/db/chat/subagent/record.rs` & `src/db/chat/subagent.rs`**：
     - `SubagentRecord` 新增 `pub agent_path: String`（通过 `decode_stored_role_and_path` 解码，旧记录无路径时回退为 `format!("/root/{}", slugify_segment(nickname_or_id))`）；
     - `ReportPayload` 新增 `pub changed_paths: Vec<String>`；
     - 新增 `resolve_session_tree_root`（向上找根会话 ID、当前深度与当前节点路径）与 `list_subagent_tree`（递归 CTE 查询整棵树所有子孙记录）；
     - 新增 `list_changed_paths` 从 `agent_file_mutation` + `agent_turn_file_scope` 提取最多 32 条去重相对路径。
- **验证点**：`cargo test --lib db::chat::subagent` 与 `cargo test --lib db::chat::fork` 全绿。

---

### Task 4 (T4)：`run_agent_turn` 预算护栏与多级协调工具面接入
- **目标文件**：
  - `src/chat/context/mod.rs`
  - `src/chat/agent_loop/mod.rs`
- **实施细节**：
  1. 在 `ModelContext` 中增加：
     - `subagent_depth: usize`（主会话为 `0`，子为 `1`，孙为 `2`）；
     - `max_subagent_depth: usize`（来自 `SubagentsConfig.max_depth`）；
     - `token_budget: Option<SubagentTokenBudget>`。
  2. 在 `run_agent_turn` 循环中：
     - 每步采样收到 `step_usage` 后调用 `token_budget.record_step_usage(latest)`；
     - 若返回 `true`（达到预算上限）：在当前步工具结算完毕后停止循环，返回 `TurnSummary::budget_exceeded(usage).with_billing(billing)`；
     - 若未耗尽且 `token_budget.take_pending_reminder()` 返回 `Some(reminder)`：调用 `context.push_user_context(reminder)` 注入 `<rollout_budget>` 提醒；
     - 控制工具拦截逻辑改为：当 `context.is_subagent()` 时，若 `call.name == "spawn_agent"` 且 `context.subagent_depth >= context.max_subagent_depth`，返回 `DEPTH_LIMIT_REACHED_CODE`；若当前角色的 `tool_surface_at_depth` 不含控制工具，返回 `UNAVAILABLE_CODE`；否则允许协调型 `Executor` 正常调用 `host.subagent_with_call`。
- **验证点**：`cargo test --lib chat::agent_loop` 全绿。

---

### Task 5 (T5)：交互式事件层与无头 `exec` 池的多级协同、双模投递与半程汇报
- **目标文件**：
  - `src/event/chat/subagent/{mod.rs, runner.rs, launch.rs, settle.rs, wait.rs, context.rs}`
  - `src/chat/agent/exec/subagent/{mod.rs, turn.rs, settle.rs}`
- **实施细节**：
  1. **多级 Spawn 与整树并发守卫**：
     - 调用 `database.resolve_session_tree_root` 获取 `(root_session_id, caller_depth, caller_path)`；
     - 检查 `caller_depth + 1 > max_depth`，超限返回 `DEPTH_LIMIT_REACHED_CODE`；
     - 统计同一 `root_session_id` 下的运行中子孙数 `running_count_in_tree >= max_concurrent`，超限返回 `LIMIT_REACHED_CODE`；
     - 调用 `allocate_child_path(&caller_path, task_name_or_nickname, &existing_paths)` 分配唯一 `agent_path` 并持久化。
  2. **三合一寻址与 `list_agents(path_prefix)`**：
     - 抽出公共解析函数 `resolve_target_in_tree(records, caller_path, target)`，在 `send_input` / `wait_agent` / `interrupt_agent` / `close_agent` / `resume_agent` 中统一调用；
     - `list_agents` 支持按 `path_prefix`（或调用者所在子树）过滤返回。
  3. **级联关闭与中断**：
     - `close_agent` 找出目标及其子树下的所有后代，将它们全部加入 `closed_subagents`（标记不生成报告）、发送 `cancel_tx` 取消在途轮次并在 DB 置 `cancelled`；
     - `interrupt_agent` 中断目标（及其正在运行的后代），保留 `idle` 状态与半程汇报。
  4. **`send_input(trigger_turn)`**：
     - 当 `trigger_turn == false` 时仅入队并同步 UI `queued_count`，不调用 `schedule_next_queued_input`。
  5. **半程汇报生成**：
     - 在 `settle.rs` 中，只要非 `closed_subagents` 触发的结束（即 `Complete` / `Failed` / `interrupt_agent` / `BudgetExceeded`），均调用 `list_changed_paths` 与 `synthesize_partial_report_body` 生成并落库 `SubagentReportView`。
- **验证点**：`cargo test --lib event::chat` 与 `cargo test --lib chat::agent::exec` 全绿。

---

### Task 6 (T6)：TUI 状态与渲染层增强 (`src/ui/main_content/lx/` & `src/app/`)
- **目标文件**：
  - `src/app/state/lx/{agent.rs, slash.rs}`
  - `src/app/state/lx/chat/tool_view.rs`
  - `src/app/update/chat/{subagent.rs, session.rs}`
  - `src/app/update/pane/task_page.rs`
  - `src/input/mod.rs`
  - `src/ui/main_content/lx/history/tool/subagent.rs`
  - `src/ui/main_content/lx/history/report.rs`
  - `src/ui/main_content/lx/task/agents.rs`
  - `src/ui/text.rs`、`src/ui/text/agent.rs`
- **实施细节**：
  1. **`wait_agent(all)` 实时列表**：
     - `app/update/chat/subagent.rs::refresh_subagent_cards` 对 `view.all` 动态回填被等待子 Agent 列表；
     - `ui/main_content/lx/history/tool/subagent.rs` 对非空 `view.agents` 逐行渲染目标状态与点击链接。
  2. **主历史 `spawn_agent` 卡片实时动作摘要**：
     - `SubagentRun::live_activity(&self)` 从 `run.chat` 提取最新运行中工具/思考/正文摘要；
     - `refresh_subagent_cards` 同步到 `SubagentDetailView.live_activity`，在 `spawn_agent` 卡片下以 `style::muted()` 单行渲染，完成后自动消失。
  3. **`Agents` 面板树形缩进、Token 用量与 `[stop ^d] [stop all ^a]`**：
     - `task/agents.rs` 按 `run.depth()` 渲染 `├─ ` / `└─ ` 树形连接符，并在耗时与标题之间展示紧凑 Token 数；
     - `task_page.rs` 新增 `LxInput::StopAllSubagents`（`Ctrl+A`）停止全部运行中子 Agent，`^d` 停止当前选中子 Agent（及其子树），更新 `ui/text.rs` 底栏键位说明。
  4. **新增 `/subagents`（别名 `/agents`）斜杠命令**：
     - `SlashCommand::Subagents` 注册并映射至打开 `TaskPage` 的 `TaskTab::Agents`。
- **验证点**：`cargo test --lib ui::main_content::lx` 与 `cargo test --lib app::update` 全绿。

---

### Task 7 (T7)：Web 设置页扩展 (`assets/settings*`)
- **目标文件**：
  - `src/settings/service/subagents.rs`
  - `assets/settings.html`
  - `assets/settings_subagents.js`
- **实施细节**：
  1. 在 `assets/settings.html` 的 Subagents 全局配置区增加 `Max Depth (1–3)` 与 `Default Token Budget` 输入框；
  2. 在 `assets/settings_subagents.js` 中读写 `maxDepth`、`maxConcurrent`（上限 12）与 `defaultMaxTokens` 并接入脏状态检测与批量保存。
- **验证点**：`cargo test --lib settings` 全绿。

---

### Task 8 (T8)：全链路单测补充、P0–P3 审查修复与静态检查收口
- **目标文件**：
  - `src/chat/agent/tests/mod.rs`
  - `src/chat/agent/exec/tests/subagent_tests.rs`
  - `src/db/chat/subagent/tests.rs`
  - `src/event/chat/tests/subagent.rs`
  - `src/app/update/chat/tests/subagent.rs`
  - `src/ui/main_content/lx/task/tests.rs`
  - `docs/feature/lx-agent-hierarchy-and-guards/verification.md`
- **实施细节**：
  1. 补充多级路径分配、三合一寻址（`id → path → nickname`）、深度超限拦截（`DEPTH_LIMIT_REACHED_CODE`）测试；
  2. 补充 `fork_turns` 轮次裁剪与思考块剥离、token 置空测试；
  3. 补充 `SubagentTokenBudget` 25%/10% `<rollout_budget>` 注入与多步累计耗尽熔断 + 半程汇报生成测试；
  4. 补充 `send_input(trigger_turn=false)` 仅入队不唤醒测试，以及嵌套 `Executor` 无参 `wait_agent({})` 严格后代过滤与孙节点队列自动续跑测试；
  5. 补充 `wait_agent(all)` 逐目标渲染、`spawn_agent` 实时活动行（含子会话内部卡片刷新）、`Agents` 面板 DFS 树形排序与缩进、`/subagents` 命令测试；
  6. 递增 `PROMPT_VERSION` 至 `"lx-11"`，完成 `verification.md` 报告，执行 `cargo fmt`、`cargo clippy --all-targets -- -D warnings`、受影响模块全量 `cargo test` 及 `./target/debug/lx-tui exec` CLI 实机校验。
