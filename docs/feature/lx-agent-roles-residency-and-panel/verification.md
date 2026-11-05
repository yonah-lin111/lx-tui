# `lx` Agent 角色体系、上下文感知、内存驻留与 TUI 交互 — 验证报告 (`verification.md`)

## 1. CLI 实机与测试套件概览

| 校验项 | 执行命令 | 结果 |
| :--- | :--- | :--- |
| 格式与静态检查 | `cargo fmt --check && cargo clippy --all-targets -- -D warnings` | 通过（0 警告 / 0 错误） |
| 领域、配置与工具单测 | `cargo test --lib config::subagents && cargo test --lib chat::agent && cargo test --lib chat::tools` | 通过（524 passed） |
| 事件层与无头 `exec` 单测 | `cargo test --lib event::chat && cargo test --lib chat::agent::exec` | 通过（238 passed） |
| App 状态、TUI 面板与设置 API | `cargo test --lib app::state::lx && cargo test --lib app::update && cargo test --lib ui::main_content::lx && cargo test --lib settings` | 通过（1648 passed） |
| `lx-tui exec` CLI 实机持久化多场景检验 | `./target/debug/lx-tui exec --json "..."` & `exec resume --last --json "..."` | 全部通过并已落库（Exit Code 0） |

---

## 2. 贯通验证项（V01–V07）

### V01 — `monitor` 内置角色、默认 `low` 推理档位与两阶段续轮（模块 H & J）
- **输入内容**：依次调用 `spawn_agent(role="monitor", task_name="cmd_watcher", task="Run exec_command echo MONITOR_STAGE_1")` → `wait_agent` → `resume_agent` → `send_input(message="Reply with MONITOR_STAGE_2", trigger_turn=true)` → `wait_agent`。
- **验证步骤**：执行 `./target/debug/lx-tui exec --ephemeral --json "1) Call spawn_agent with role='monitor', title='watch echo', task_name='cmd_watcher'..."` 及 `cargo test --lib chat::agent::tests::monitor_surface_is_leaf_only_with_four_tools`。
- **期望结果**：CLI 实机输出 `"role":"monitor","model":"effort:low"`（自动降档），两阶段分别返回 `MONITOR_STAGE_1` 与 `MONITOR_STAGE_2`，且任何深度下均不含写文件与控制工具（实测通过）。

### V02 — 项目级角色发现（`.lx/agents/*.json` & `.agents/roles/*.json`）与能力裁剪（模块 G）
- **输入内容**：在工作区放置 `.lx/agents/auditor.json`（省略 `name`）与 `.agents/roles/strict_watcher.json`（`baseRole="monitor", allowMcp=false, allowSkills=false`），要求子 Agent 输出本轮被声明的全部工具名。
- **验证步骤**：执行 `./target/debug/lx-tui exec -C "$TMP_WS" --ephemeral --json "Call spawn_agent with role='strict_watcher', title='check strict monitor tools', task_name='watcher2'..."` 及 `cargo test --lib config::subagents::tests::project_roles_discover_only_when_trusted_and_never_shadow_builtins_or_globals`。
- **期望结果**：CLI 实机发现两个目录下的项目角色；开启 MCP 时包含 30+ 个 MCP 工具，设 `allowMcp=false` 后子 Agent 实机回报 `TOOLS: exec_command, read, search, write_stdin`（外部 MCP 与 resource 工具被精确剥离，实测通过）。

### V03 — 角色模型锁定 `lockModel: true` 忽略非法覆盖传参（模块 G）
- **输入内容**：配置 `.lx/agents/locked_scout.json`（`model={"provider":"provider-alq374","model":"cbcn/deepseek-v4.1-flash","variant":"low"}, lockModel=true`），调用 `spawn_agent(role="locked_scout", model="fake_provider/nonexistent_model", reasoning_effort="high")`。
- **验证步骤**：执行 `./target/debug/lx-tui exec -C "$TMP_WS" --ephemeral --json "Call spawn_agent with role='locked_scout', title='test lock', task_name='scout1', model='fake_provider/nonexistent_model', reasoning_effort='high'..."` 及 `cargo test --lib event::chat::tests::subagent_roles::lock_model_ignores_caller_model_and_effort_overrides`。
- **期望结果**：`spawn_agent` 工具描述显式标注 `locked model: ... (low) — cannot be overridden`；CLI 实机静默忽略非法的 `fake_provider/nonexistent_model` 与 `high`，强制以锁定模型 `effort:low` 完成并返回 `LOCKED_MODEL_OK`（实测通过）。

### V04 — `interrupt_agent` 中断恢复、`<turn_aborted>` 注入与 `NEW_TASK` 结构化信封（模块 I & J）
- **输入内容**：派发 `/root/sleeper` 执行长耗时命令 → 立即调用 `interrupt_agent(target="/root/sleeper")` → 调用 `send_input(target="/root/sleeper", message="Confirm whether you see <turn_aborted> and Message Type: NEW_TASK...", trigger_turn=true)` → `wait_agent`。
- **验证步骤**：执行 `./target/debug/lx-tui exec --ephemeral --json "1) Call spawn_agent with role='monitor', title='long command', task_name='sleeper'..."` 及 `cargo test --lib event::chat::tests::subagent_roles::interrupted_turn_is_retained_with_aborted_marker_and_followup_uses_envelope`。
- **期望结果**：CLI 实机子 Agent 在续轮上下文中同时观测到上一轮末尾的 `<turn_aborted>` 标记与本轮的 `Message Type: NEW_TASK`（`Task name: /root/sleeper`, `Sender: /root`）信封，回报 `ABORT_AND_ENVELOPE_OK`（实测通过）。

### V05 — `<multi_agent_mode>` 委派模式、`<subagents>` 快照与空闲子会话 LRU 内存回收（模块 I & J）
- **输入内容**：主会话配置 `DelegationMode`（`Auto` / `Proactive` / `Explicit`）与 10 个不同状态子 Agent；同一窗格加载 7 个已结束子会话 + 1 个运行中子会话后重选被淘汰项。
- **验证步骤**：执行 `cargo test --lib event::chat::tests::subagent_roles::known_subagent_lines_orders_by_status_excludes_cancelled_and_caps_at_eight` 与 `cargo test --lib app::update::chat::tests::subagent::lru_evicts_oldest_idle_chats_beyond_six_and_reloads_on_select`。
- **期望结果**：`<subagents>` 按状态优先级输出最多 8 条非 `cancelled` 快照；超出 `MAX_RESIDENT_SUBAGENT_CHATS = 6` 时自动按 LRU 卸载最久未访问子会话的 `LxChat`，重选时触发 `Effect::LoadSubagentSession` 懒加载（实测通过）。

### V06 — TUI `Agents` 面板三态筛选、内联追问、底栏 `blocked` 提醒与设置 API（模块 K & G）
- **输入内容**：在 `Agents` 面板按 `Tab` 切换 `all · active · done`，按 `Enter` 对已结束子 Agent 发送内联追问 `To <nickname>: `，存在 `status="blocked"` 子 Agent，以及调用 `/api/subagents` 系列接口。
- **验证步骤**：执行 `cargo test --lib app::update::chat::tests::task_page`、`cargo test --lib ui::main_content::lx::task::tests`、`cargo test --lib ui::main_content::lx::status::tests` 与 `cargo test --lib settings::service::tests`。
- **期望结果**：`Tab` 过滤可见项且在父节点被隐藏时省略悬空 `├─/└─`；`Enter` 自动 `resume + send_input`；底栏渲染 `1 blocked`；Web 设置 API 完整读写 `monitor`、`delegationMode`、`lockModel` 与 3 项能力开关（实测通过）。

### V07 — Review 专项修复验证：`MESSAGE` 信封、`trigger_turn=false` 不误触发、子目录 Worktree 角色发现与内联追问目标锁定（P0–P3）
- **输入内容**：1) 调用 `send_input(trigger_turn=false)` 排队消息后由 `resume_agent` 唤醒，检查子 Agent 收到的 `Message Type` 与 `Sender`；2) 在 linked worktree 的子目录 `-C src` 下调用工作树根 `.lx/agents/strict_scout.json`；3) 在 `Active` 过滤下对第 2 个子 Agent 打开内联追问并粘贴多行文本，期间第 1 个子 Agent 完成退出。
- **验证步骤**：执行 `./target/debug/lx-tui exec --json "...send_input(..., trigger_turn=false) -> resume_agent..."`、`./target/debug/lx-tui exec -C src --ephemeral --json "Call spawn_agent with role='strict_scout'..."`，以及 `cargo test --lib event::chat::tests::subagent_roles::trigger_turn_false_stays_idle_after_running_turn_and_uses_message_envelope_on_wake` 与 `cargo test --lib app::update::chat::tests::task_page::inline_reply_locks_target_across_active_filter_changes_and_handles_paste`。
- **期望结果**：`trigger_turn=false` 在运行轮自然完成后保持 `idle (queued_count=1)` 不自动开轮，唤醒后精确包裹 `Message Type: MESSAGE` 与真实 `Sender: /root`；被中断的无文本轮次不产生空 `AssistantText("")`；`-C src` 跨子目录成功解析 worktree 项目角色并返回 `STRICT_SCOUT_OK`；内联追问跨列表重排始终锁定原目标且粘贴不污染底部隐藏草稿（实测通过）。
