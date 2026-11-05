# `lx` Agent 多级编排、预算护栏与 TUI 增强 — 验证报告

## 1. CLI 实机与全量套件验证概览

| 校验项 | 执行命令 | 结果 |
| :--- | :--- | :--- |
| 代码格式与静态检查 | `cargo fmt -- --check && cargo clippy --all-targets -- -D warnings` | 通过（0 警告 / 0 错误） |
| 领域、解析、预算与报告单测 | `cargo test --lib chat::agent && cargo test --lib chat::agent_loop && cargo test --lib chat::tools` | 通过（555 passed） |
| 数据库分叉清洗与递归树单测 | `cargo test --lib db::chat` | 通过（119 passed） |
| 事件层与无头 `exec` 协同单测 | `cargo test --lib event::chat` | 通过（122 passed） |
| App 状态、TUI 卡片与面板单测 | `cargo test --lib app::state::lx && cargo test --lib app::update && cargo test --lib ui::main_content::lx` | 通过（1567 passed） |
| 配置与 Web 设置 API 单测 | `cargo test --lib config::subagents && cargo test --lib settings` | 通过（71 passed） |
| `lx-tui exec` CLI 实机多级编排 | `./target/debug/lx-tui exec --ephemeral --json "..."` | 通过（Exit Code 0） |

---

## 2. 分模块验证清单

### V01 — `fork_turns` 轮次截取与分叉历史三项清洗（模块 A）
- **输入内容**：父会话含 3 个已完成轮次（带 `reasoning` 思考块、非零 `input_tokens/output_tokens` 及 `<subagent_report>` 标记）与 1 个流式轮次；调用 `spawn_agent({"role":"explorer","task":"inspect","fork_turns":2})`。
- **验证步骤**：执行 `cargo test --lib db::chat::subagent::tests::fork_turns_copies_only_recent_completed_turns_and_sanitizes_history`。
- **期望结果**：子会话仅克隆最近 2 个已完成轮次，跳过流式轮次、剥离全部 `reasoning`/`thinking` 分片、克隆消息 4 个 Token 计费字段清零为 `NULL`、父级汇报标记清洗为普通 `chat` 消息（实测通过）。

### V02 — `ForkSpec` 参数归一化与边界校验（模块 A）
- **输入内容**：`fork_turns: 0`、`fork_turns: 5`（未传 `fork_context`）、`fork_turns: 65`（越界）。
- **验证步骤**：执行 `cargo test --lib chat::agent::tests::fork_spec_normalizes_params` 与 `cargo test --lib chat::agent::tests::parse_spawn_accepts_task_name_fork_turns_and_max_tokens`。
- **期望结果**：`fork_turns=0` 关闭分叉；`1..=64` 自动开启 `fork_context=true` 并映射为 `ForkSpec::RecentTurns(n)`；`>64` 拒绝并返回参数校验错误（实测通过）。

### V03 — `SubagentTokenBudget` 25% / 10% 阈值提醒与 0% 耗尽熔断（模块 B）
- **输入内容**：子 Agent 配置 `max_tokens = 1000`，分三步分别消耗 `760`（剩 24%）、`150`（剩 9%）、`100`（剩 0%）tokens。
- **验证步骤**：执行 `cargo test --lib chat::agent_loop::tests::token_budget_injects_reminders_at_25_and_10_percent_remaining` 与 `cargo test --lib chat::agent_loop::tests::token_budget_exhaustion_stops_the_turn_after_current_tool_step`。
- **期望结果**：剩余 24% 与 9% 时各向子会话注入一次 `<rollout_budget>` 收敛提醒；耗尽至 0% 时在当前工具步结算后终止本轮并标记 `budget_exceeded = true`（实测通过）。

### V04 — `interrupt_agent` 半程汇报 vs `close_agent` 抑制汇报（模块 B）
- **输入内容**：两个运行中子 Agent `/root/hanger` 与 `/root/closer`，分别调用 `interrupt_agent({"target":"/root/hanger"})` 与 `close_agent({"target":"/root/closer"})` 后执行 `wait_agent`。
- **验证步骤**：执行 `cargo test --lib chat::agent::exec::tests::subagent_tests::interrupt_agent_by_path_produces_partial_report_while_close_suppresses_report`。
- **期望结果**：`/root/hanger` 转为 `status=idle` 并产出含 `[Partial report (interrupted)]` 的半程汇报；`/root/closer` 被取消且不生成任何汇报（实测通过）。

### V05 — `send_input(trigger_turn=false)` 仅入队不唤醒与非法组合拦截（模块 C）
- **输入内容**：对 `idle` 子 Agent `/root/reader` 调用 `send_input({"target":"/root/reader","message":"follow-up","trigger_turn":false})`；以及非法组合 `{"target":"Turing","message":"x","interrupt":true,"trigger_turn":false}`。
- **验证步骤**：执行 `cargo test --lib chat::agent::exec::tests::subagent_tests::send_input_with_trigger_turn_false_queues_without_waking_idle_subagent` 与 `cargo test --lib chat::agent::tests::parse_send_input_supports_trigger_turn`。
- **期望结果**：`trigger_turn=false` 仅入队（`queued_count=1`、`status=idle`），不触发子 Agent 新轮次；`interrupt=true && trigger_turn=false` 直接报参数错误拒绝（实测通过）。

### V06 — 报告 `changed_paths` 落库、信封拼装与 `wait_agent` 回传（模块 C）
- **输入内容**：`executor` 子 Agent `/root/patcher` 调用 `write` 写入 `patched.txt` 后完成，父会话调用 `wait_agent({"targets":["/root/patcher"]})`。
- **验证步骤**：执行 `cargo test --lib chat::agent::exec::tests::subagent_tests::spawn_with_task_name_and_list_agents_prefix_and_changed_paths` 与 `cargo test --lib ui::main_content::lx::history::tool::subagent::tests::expanded_report_lists_changed_paths`。
- **期望结果**：`SubagentReportView.changed_paths`、`<subagent_report>` 信封、`wait_agent` 返回结果与 TUI 展开卡片均包含 `Changed files: patched.txt`（实测通过）。

### V07 — `AgentStatus` 7 态强类型枚举与投影状态收敛（模块 D）
- **输入内容**：7 种状态字面量（`running`/`idle`/`blocked`/`completed`/`failed`/`cancelled`/`interrupted`）及运行态、暂停态、队首阻塞态组合。
- **验证步骤**：执行 `cargo test --lib chat::agent::tests::agent_status_roundtrips_and_projects`。
- **期望结果**：`AgentStatus::parse` / `as_str` 双向无损转换；`projected_agent_status` 按“运行中 > 队首阻塞 > 已恢复/中断暂停 (`idle`) > DB 终态”准确投影（实测通过）。

### V08 — TUI `wait_agent(all)` 卡片实时列表与 `spawn_agent` 实时活动（模块 E）
- **输入内容**：未传 `targets` 的 `wait_agent({})` 卡片，以及正在执行工具/思考的 `spawn_agent` 卡片。
- **验证步骤**：执行 `cargo test --lib app::update::chat::tests::subagent::wait_all_card_tracks_every_running_subagent`、`cargo test --lib ui::main_content::lx::history::tool::subagent::tests::wait_all_renders_every_agent_row_and_link` 与 `cargo test --lib ui::main_content::lx::history::tool::subagent::tests::spawn_card_shows_live_activity_only_while_running`。
- **期望结果**：`wait_agent(all)` 逐行渲染本会话所有被等待子 Agent 及可点击跳转链接；运行中 `spawn_agent` 卡片展示 muted 单行实时动作，结束后自动隐藏（实测通过）。

### V09 — TUI `Agents` 面板树缩进、Token 用量列、`^d`/`^a` 快捷键与 `/subagents` 命令（模块 E）
- **输入内容**：含深度 1（`/root/turing`）与深度 2（`/root/turing/curie`）的 `Agents` 列表；在 `Agents` 页签分别按下 `Ctrl+D`、`Ctrl+A`；在输入框执行 `/subagents` 与 `/agents`。
- **验证步骤**：执行 `cargo test --lib ui::main_content::lx::task::tests::agents_list_indents_subtree_and_shows_tokens`、`cargo test --lib app::update::chat::tests::task_page::ctrl_a_on_agents_tab_stops_all_subagents` 与 `cargo test --lib app::update::chat::tests::task_page::subagents_slash_command_opens_the_agents_tab`。
- **期望结果**：孙辈节点显示 `└─ ` 树形缩进与 `1.8k tok` 用量列；`^d` 停止选中项及其子树，`^a` 停止全部运行中子 Agent；`/subagents` 与 `/agents` 直接打开后台面板 `Agents` 页签（实测通过）。

### V10 — `lx-tui exec` CLI 实机检验：路径寻址、`path_prefix` 过滤与二级递归派发（模块 F）
- **输入内容**：
  1. 单级路径寻址提示词：`"Call spawn_agent with role='explorer', title='check status', task_name='status_checker', max_tokens=20000, task='Reply with OK'. Then call list_agents with path_prefix='status_checker' and wait_agent with targets=['/root/status_checker']. Finally reply with the agent_path."`
  2. 二级递归派发提示词：`"Call spawn_agent with role='executor', title='coordinator', task_name='coord', fork_turns=1, task='Call spawn_agent with role=explorer, title=leaf, task_name=leaf, task=\"Reply LEAF_OK\", then call wait_agent for /root/coord/leaf and reply with its output'. Then call wait_agent with targets=['/root/coord'] and reply with the result."`
- **验证步骤**：在工作区运行 `./target/debug/lx-tui exec --ephemeral --json "<Prompt>"`。
- **期望结果**：
  1. 单级 CLI 输出 `collab_tool_call` 事件：`spawn_agent` 分配 `/root/status_checker`，`list_agents(path_prefix="status_checker")` 命中该节点，`wait_agent(targets=["/root/status_checker"])` 返回 `status="completed"` 与 `OK`（实测 Exit Code 0）。
  2. 二级 CLI 输出：根会话派发深度 1 `Executor`（`/root/coord`，`fork_context=true`），该 `Executor` 拥有 `ExecutorCoordinator` 工具面并成功派发深度 2 `Explorer`（`/root/coord/leaf`），逐级 `wait_agent` 收回 `LEAF_OK`（实测 Exit Code 0）。

### V11 — 最大深度守卫 `max_depth`、整树并发上限与 `close_agent` 级联关闭（模块 F）
- **输入内容**：深度已达 `max_depth` 的 `Executor` 再次调用 `spawn_agent`；并发数达 4 时发起第 5 个 `spawn_agent`；对父节点 `/root/coord` 调用 `close_agent`。
- **验证步骤**：执行 `cargo test --lib event::chat::tests::subagent::nested_spawn_allocates_paths_and_enforces_the_depth_limit`、`cargo test --lib chat::agent::exec::tests::subagent_tests::spawning_a_fifth_concurrent_subagent_is_rejected_with_limit_reached` 与 `cargo test --lib event::chat::tests::subagent::close_agent_cascades_to_descendants_and_suppresses_reports`。
- **期望结果**：深度超限返回 `"Agent depth limit reached. Solve the task yourself."`；第 5 个并发子 Agent 返回 `"subagent concurrency limit reached"`；关闭 `/root/coord` 级联取消 `/root/coord/*` 全部后代且不产出汇报（实测通过）。

### V12 — Web 设置页 `/api/subagents` 配置往返与越界拦截（模块 G）
- **输入内容**：`PUT /api/subagents/general` 传入 `{"maxConcurrent":12,"maxDepth":3,"defaultMaxTokens":100000}`，以及越界参数 `maxDepth: 0 / 4`、`maxConcurrent: 13`。
- **验证步骤**：执行 `cargo test --lib settings::service::tests::subagents_round_trip_depth_and_token_budget` 与 `cargo test --lib settings::service::tests::subagents_reject_out_of_range_depth_and_concurrency`。
- **期望结果**：合法配置在 `GET /api/subagents` 与全量 `PUT /api/subagents` 中无损往返（传 `null` 可清除 `defaultMaxTokens`）；越界参数返回 HTTP `422 Unprocessable Entity` 且不污染原配置（实测通过）。

---

## 3. 代码审查（P0–P3）修复与回归验证

### P0-1 — 嵌套 `Executor` 无参 `wait_agent({})` 自等待死锁与跨层报告误消费
- **输入内容**：深度 1 `Executor`（`/root/coord`）派发深度 2 `Explorer`（`/root/coord/leaf`）后调用无参 `wait_agent({})`。
- **验证步骤**：执行 `cargo test --lib event::chat::tests::subagent::nested_executor_wait_all_only_waits_for_descendants_not_itself`、`cargo test --lib chat::agent::exec::tests::subagent_tests::nested_executor_wait_all_and_grandchild_queued_input_succeed` 及 `./target/debug/lx-tui exec --ephemeral --json "..."` 实机多级 `wait_agent({})`。
- **期望结果**：`wait_agent(all)` 仅等待调用者严格后代（排除调用者自身），显式传自身直接报错拒绝，且 `settle_wait` 仅消费直接子节点的待投递报告（实测通过）。

### P0-2 — TUI 深度 2 孙节点 `on_subagent_started` 会话归属拒绝修复
- **输入内容**：深度 1 `Executor`（`child_session = "sess-d1"`）派发深度 2 孙节点（`parent_session_id = "sess-d1"`）。
- **验证步骤**：执行 `cargo test --lib app::update::chat::tests::subagent::depth_two_spawn_matches_inner_parent_session_and_sorts_tree`。
- **期望结果**：根窗格识别 `pane.lx.agents.by_session(&run.parent_session_id)` 并将孙节点纳入 `runs`，后续流式增量与终态正常更新（实测通过）。

### P1-1 — TUI 多步累计 Token 预算耗尽（`budget_exceeded`）透传与跨轮预算保持
- **输入内容**：子 Agent 配置 `max_tokens = 2000`，分两步各消耗 `1200` tokens（单步 `< 2000`、累计 `2400 >= 2000`）触发预算熔断。
- **验证步骤**：执行 `cargo test --lib event::chat::tests::subagent::multi_step_token_budget_exceeded_marks_subagent_idle_with_partial_report`。
- **期望结果**：`ChatNetEvent::Finished` 透传 `summary.budget_exceeded = true`，结算为 `idle` 并产出 `[Partial report (budget_exceeded)]`（实测通过）。

### P1-2 — 无头 `exec` 跨层队列续跑、真实 `parent_session_id` 归属与相对路径寻址
- **输入内容**：深度 1 `/root/coord` 对相对路径 `"leaf"` 调用 `send_input`，或对 `"./worker/"` / `"/root/worker/"` 调用 `resume_agent` / `send_input` / `wait_agent`。
- **验证步骤**：执行 `cargo test --lib chat::agent::exec::tests::subagent_tests::nested_executor_wait_all_and_grandchild_queued_input_succeed` 及 `./target/debug/lx-tui exec --ephemeral --json` 实机验证。
- **期望结果**：孙节点首轮结束后按真实 `parent_session_id` 自动唤醒排队输入，相对路径与尾斜杠规范化寻址一致命中目标（实测通过）。

### P1-3 & P1-4 — TUI `Agents` 列表树形先序（DFS）排序与子会话内部卡片回填
- **输入内容**：依次派发 `/root/a`、`/root/b` 后再由 `/root/a` 派发 `/root/a/c`；以及深度 1 子会话内部的 `spawn_agent` 卡片。
- **验证步骤**：执行 `cargo test --lib app::update::chat::tests::subagent::depth_two_spawn_matches_inner_parent_session_and_sorts_tree` 与 `cargo test --lib app::update::chat::tests::subagent::refresh_subagent_cards_updates_nested_spawn_cards`。
- **期望结果**：`runs` 按树先序重排为 `[/root/a, /root/a/c, /root/b]` 并正确绘制 `├─ ` / `└─ `；子会话内部卡片同步回填孙节点状态与报告（实测通过）。
