# `lx` Agent 角色体系、上下文感知、内存驻留与 TUI 交互 — 验证报告 (`verification.md`)

## 1. CLI 实机与测试套件概览

| 校验项 | 执行命令 | 结果 |
| :--- | :--- | :--- |
| 格式与静态检查 | `cargo fmt --check && cargo clippy --all-targets -- -D warnings` | 通过（0 警告 / 0 错误） |
| 领域、配置与工具单测 | `cargo test --lib config::subagents && cargo test --lib chat::agent && cargo test --lib chat::tools` | 通过（524 passed） |
| 事件层与无头 `exec` 单测 | `cargo test --lib event::chat && cargo test --lib chat::agent::exec` | 通过（238 passed） |
| App 状态、TUI 面板与设置 API | `cargo test --lib app::state::lx && cargo test --lib app::update && cargo test --lib ui::main_content::lx && cargo test --lib settings` | 通过（1648 passed） |
| `lx-tui exec` CLI 实机验证 | `./target/debug/lx-tui exec --ephemeral --json "..."` | 通过（Exit Code 0） |

---

## 2. 贯通验证项（V01–V06）

### V01 — CLI 实机：`monitor` 内置角色、默认 `low` 推理档位与 `NEW_TASK` 续轮信封（模块 H & J）
- **输入内容**：提示词指令依次执行：`spawn_agent(role="monitor", task_name="cmd_watcher", task="Run exec_command echo MONITOR_STAGE_1")` → `wait_agent(["/root/cmd_watcher"])` → `resume_agent("/root/cmd_watcher")` → `send_input(target="/root/cmd_watcher", message="Reply with MONITOR_STAGE_2", trigger_turn=true)` → `wait_agent`。
- **验证步骤**：执行 `./target/debug/lx-tui exec --ephemeral --json "1) Call spawn_agent with role='monitor', title='watch echo', task_name='cmd_watcher', task='Run exec_command with cmd=\"echo MONITOR_STAGE_1\" and reply with the exact output'..."` 及 `cargo test --lib chat::agent::tests::monitor_surface_is_leaf_only_with_four_tools`。
- **期望结果**：`spawn_agent` 事件输出 `role="monitor"`、`model="effort:low"`（自动降档），子 Agent 仅暴露 `read/search/exec_command/write_stdin` 4 工具并跑通命令；第 2 轮 `send_input` 在模型请求侧自动包裹 `Message Type: NEW_TASK` 信封并返回 `MONITOR_STAGE_1` 与 `MONITOR_STAGE_2`（实测通过）。

### V02 — CLI 实机：项目级角色自动发现（`.lx/agents/*.json`）与未信任工作区隔离（模块 G）
- **输入内容**：在工作区创建 `.lx/agents/auditor.json`（省略 `name`，配置 `baseRole="explorer"`、`allowWebSearch=false`、`allowMcp=false`、`allowSkills=false`），发起 `spawn_agent(role="auditor", task_name="proj_auditor")`。
- **验证步骤**：执行 `./target/debug/lx-tui exec -C "$TMP_WS" --ephemeral --json "Call spawn_agent with role='auditor', title='audit', task_name='proj_auditor'..."` 及 `cargo test --lib config::subagents::tests::project_roles_discover_only_when_trusted_and_never_shadow_builtins_or_globals`。
- **期望结果**：受信任工作区自动从文件名推断角色名 `auditor` 并注入 `spawn_agent`，实机返回 `PROJECT_ROLE_OK`；未信任工作区返回 0 个项目角色，且同名内置/全局角色永不被覆盖（实测通过）。

### V03 — 角色模型锁定 `lock_model` 与细粒度能力收紧（模块 G）
- **输入内容**：角色配置 `model="openai/gpt-4.1-mini (low)"`、`lock_model=true`、`allow_web_search=false`、`allow_mcp=false`、`allow_skills=false`；调用 `spawn_agent` 时显式传入覆盖参数 `model="anthropic/claude-sonnet-4-5"`、`reasoning_effort="high"`。
- **验证步骤**：执行 `cargo test --lib event::chat::tests::subagent_roles::lock_model_ignores_caller_model_and_effort_overrides` 与 `cargo test --lib event::chat::tests::subagent_roles::role_capabilities_strip_web_search_mcp_and_skills`。
- **期望结果**：`spawn_agent` 描述标注 `locked model: ... — cannot be overridden [no web_search, no mcp, no skills]`；运行时静默忽略调用方覆盖模型，子会话工具声明与系统提示剔除 `web_search`、MCP 与 Skills（实测通过）。

### V04 — `<multi_agent_mode>` 委派模式、`<subagents>` 环境快照与 `<turn_aborted>` 中断标记（模块 I & J）
- **输入内容**：主会话处于 `Auto`（配 `high` 推理档位）/ `Proactive` / `Explicit` 模式；根会话树含 10 个不同状态的子 Agent；子会话上一轮被 `interrupt_agent` 中断后再次 `send_input` 续跑。
- **验证步骤**：执行 `cargo test --lib event::chat::tests::subagent_roles::known_subagent_lines_orders_by_status_excludes_cancelled_and_caps_at_eight` 与 `cargo test --lib event::chat::tests::subagent_roles::interrupted_turn_is_retained_with_aborted_marker_and_followup_uses_envelope`。
- **期望结果**：主会话注入 `<multi_agent_mode>` 策略段与最多 8 条非 `cancelled` 的 `<subagents>` 快照（按 `Running -> Blocked -> Idle -> Interrupted -> Failed -> Completed` 排序）；被中断子轮次保留已执行步骤并追加 `<turn_aborted>` 标记，DB 与 TUI 消息原文保持无协议头污染（实测通过）。

### V05 — 空闲子会话 LRU 内存驻留回收（`MAX_RESIDENT_SUBAGENT_CHATS = 6`）与重选懒加载（模块 J）
- **输入内容**：同一窗格内先后加载 7 个已结束子会话 + 1 个运行中子会话，随后在 `Agents` 面板重新选中被最早淘汰的第 1 个子会话。
- **验证步骤**：执行 `cargo test --lib app::update::chat::tests::subagent::lru_evicts_oldest_idle_chats_beyond_six_and_reloads_on_select`。
- **期望结果**：超出 6 个空闲上限时自动卸载最久未访问子会话的 `LxChat` 消息树（置 `loaded=false`，运行中与当前选中项受保护）；重新选中该项时自动发出 `Effect::LoadSubagentSession` 从 SQLite 懒加载恢复（实测通过）。

### V06 — TUI `Agents` 面板三态筛选、内联追问自动恢复、底栏 `blocked` 提醒与设置页 API（模块 K & G）
- **输入内容**：在 `Agents` 面板按 `Tab` 轮转 `all · active · done`，对已结束子 Agent 按 `Enter` 输入追问并回车，按 `Esc` 取消输入；存在 `status="blocked"` 子 Agent；调用 `PUT /api/subagents/builtins/monitor` 与 `PUT /api/subagents/settings`。
- **验证步骤**：执行 `cargo test --lib app::update::chat::tests::task_page`、`cargo test --lib ui::main_content::lx::task::tests`、`cargo test --lib ui::main_content::lx::status::tests` 与 `cargo test --lib settings::service::tests`。
- **期望结果**：`Tab` 过滤可见项且在父节点被隐藏时自动省略悬空 `├─/└─`；`Enter` 提交发出 `Effect::SendSubagentUserInput` 自动先 `resume` 再发 `send_input`；主界面底栏显示 `1 blocked`；Web 设置 API 完整读写 `monitor`、`delegationMode`、`lockModel` 与能力开关（实测通过）。
