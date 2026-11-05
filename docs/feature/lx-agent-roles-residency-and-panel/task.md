# lx 子 Agent 角色体系扩展、上下文感知、内存驻留与 TUI 交互增强 — 实施任务拆解 (`task.md`)

- **关联设计**：[`design.md`](design.md)
- **前置基线**：建立在已合入 `dev` 的 `docs/feature/lx-agent-hierarchy-and-guards`（提交 `339065ce..dc1208c9`）之上，直接复用 `AgentStatus`、`AgentPath`、`SubagentTokenBudget`、`ForkSpec`、`sort_runs_tree_order`、`subagent_view.rs` 与 `subagent_cards.rs`。
- **执行前提**：用户审核确认本 `design.md` 与 `task.md` 后，按以下任务顺序逐步落地并执行精准校验。

---

## 任务总览

| 任务 ID | 对应模块 | 任务名称 | 涉及核心文件 | 状态 |
| :--- | :--- | :--- | :--- | :--- |
| **T1** | 模块 H & J | 领域层拆分与扩展：抽离 `prompts.rs`、新增内置 `monitor` 角色、`ToolSurface::Monitor` 与结构化消息信封 `envelope.rs` | `src/chat/agent/{envelope.rs, prompts.rs, role.rs, mod.rs}`、`src/config/subagents.rs` | 已完成 |
| **T2** | 模块 G & I | 配置层与角色发现：`lock_model`、`RoleCapabilities`、`DelegationMode`、项目级角色扫描 `discover.rs` 与 `PROMPT_VERSION="lx-12"` | `src/config/subagents.rs`、`src/config/subagents/discover.rs`、`src/chat/tools/{subagent.rs, schema.rs}`、`src/chat/prompt.rs` | 已完成 |
| **T3** | 模块 G, H, I, J | 事件层与无头 `exec`（含大文件拆分）：拆出 `exec/subagent/context.rs` 与 `event/chat/subagent/snapshot.rs`，接入模型锁定、能力裁剪、`<subagents>` 快照、`<turn_aborted>` 与信封 | `src/event/chat/subagent/{snapshot.rs, mod.rs, model_resolve.rs, context.rs, launch.rs}`、`src/chat/agent/exec/subagent/{context.rs, mod.rs, turn.rs}` | 已完成 |
| **T4** | 模块 J & K | App 状态与更新层：空闲子会话 LRU 内存回收（`Residency`）、`AgentFilter`、面板内联追问与 `SendSubagentUserInput` | `src/app/effects.rs`、`src/app/state/lx/{agent.rs, task.rs}`、`src/app/update/chat/subagent.rs`、`src/app/update/pane/task_page.rs` | 已完成 |
| **T5** | 模块 K | TUI 渲染层：`Agents` 面板三态筛选栏、可见父节点树缩进兼容、顶栏内联追问输入框与主界面底边框 `blocked` 提醒 | `src/ui/main_content/lx/{task.rs, task/agents.rs, status.rs}`、`src/ui/text.rs` | 已完成 |
| **T6** | 模块 G, H, I | Web 设置页扩展（`monitor` 内置角色、`delegationMode`、`lockModel` 与能力收紧开关） | `src/settings/service/subagents.rs`、`assets/{settings.html, settings_subagents.js}` | 已完成 |
| **T7** | 全模块 | 单元测试、回归测试补充与 Clippy / Fmt 校验（严守单文件行数上限） | `src/config/subagents/tests.rs`、`src/chat/agent/tests/mod.rs`、`src/event/chat/tests/subagent_roles.rs`、`src/ui/main_content/lx/task/tests.rs` | 已完成 |
| **T8** | 全模块 | Review 专项修复：`trigger_turn` 信封语义、`<turn_aborted>` 空块清理、面板追问目标锁定、worktree 子目录角色发现、`exec` 运行时摘要、粘贴隔离与设置 API 部分更新保持 | `src/db/chat/subagent/queue.rs`、`src/event/chat/subagent/*`、`src/chat/agent/exec/*`、`src/app/*`、`src/settings/service/subagents.rs`、`src/config/subagents/discover.rs` | 已完成 |

---

## 详细任务步骤与关键代码检查点

### Task 1 (T1)：领域层拆分 `prompts.rs`、内置 `monitor` 角色与结构化通信信封 (`envelope.rs`)
- **目标文件**：
  - 新增 `src/chat/agent/prompts.rs`（从 `src/chat/agent/mod.rs` 拆出，使 `mod.rs` 从 937 行降至 ~780 行）
  - 新增 `src/chat/agent/envelope.rs`
  - 修改 `src/config/subagents.rs`
  - 修改 `src/chat/agent/role.rs`、`src/chat/agent/mod.rs`
- **实施细节**：
  1. **内置 `monitor` 角色（模块 H）**：
     - `BaseRole` 新增 `Monitor` 变体（`as_str() -> "monitor"`、`parse("monitor")`、`default_description()`），`path.rs::decode_stored_role_and_path` 自动兼容 `"monitor:/root/..."`；
     - `AgentRole` 新增 `Monitor` 变体，`AgentRole::base()` 对 `Monitor` 返回 `BaseRole::Monitor`，`is_builtin()` 返回 `true`；
     - `ToolSurface` 新增 `Monitor` 变体，对应 `MONITOR_TOOLS: [&str; 4] = ["read", "search", "exec_command", "write_stdin"]`；
     - `ToolSurface::from_role_at_depth` 对 `BaseRole::Monitor` 恒返回 `ToolSurface::Monitor`（任何深度下均不开放写文件与控制工具）；
     - 在 `src/chat/agent/prompts.rs` 放置 `EXPLORER_PROMPT`、`EXECUTOR_PROMPT`、`MONITOR_PROMPT`、`compose_subagent_prompt` 与 `compose_subagent_prompt_with_override`，并在 `mod.rs` 中 `pub use prompts::*;`。
  2. **结构化通信信封（模块 J）**：
     - 在 `src/chat/agent/envelope.rs` 实现 `InterAgentMessageType { Message, NewTask }` 与 `format_inter_agent_envelope(message_type, recipient_path, sender_path, payload)`。
- **验证点**：`cargo test --lib chat::agent` 编译并全绿，`src/chat/agent/mod.rs` < 850 行。

---

### Task 2 (T2)：角色模型锁定、能力收紧、委派模式与项目级角色自动发现
- **目标文件**：
  - 新增 `src/config/subagents/discover.rs`
  - 修改 `src/config/subagents.rs`
  - 修改 `src/chat/tools/subagent.rs`、`src/chat/tools/schema.rs`
  - 修改 `src/chat/prompt.rs`
- **实施细节**：
  1. **配置模型扩展 (`src/config/subagents.rs`)**：
     - 新增 `DelegationMode { Auto, Proactive, Explicit }`（默认 `Auto`），挂入 `SubagentsSection.delegation_mode`；
     - 新增 `RoleCapabilities { allow_web_search: bool, allow_mcp: bool, allow_skills: bool }`（默认全 `true`）；
     - `BuiltinRoleOverrides` 新增 `pub monitor: RoleOverrideConfig`；
     - `RoleOverrideConfig` 与 `CustomRoleConfig` 新增 `pub lock_model: bool`（默认 `false`）与 `#[serde(default, flatten)] pub capabilities: RoleCapabilities`；
     - `ResolvedRoleSpec` 新增 `is_project_scoped: bool`、`lock_model: bool`、`capabilities: RoleCapabilities`；
     - 保留名校验更新为拒绝 `"explorer"` / `"executor"` / `"monitor"` 作为自定义角色名。
  2. **项目级角色自动发现 (`src/config/subagents/discover.rs`)**：
     - 实现 `discover_project_roles(workspace_root: &Path) -> Vec<CustomRoleConfig>` 与 `merge_project_roles(base: &SubagentsSection, workspace: Option<&Path>, workspace_trusted: bool) -> SubagentsSection`；
     - 仅在 `workspace_trusted == true` 时扫描 `<workspace>/.lx/agents/*.json` 与 `<workspace>/.agents/roles/*.json`（单文件 ≤ 64 KiB，最多 32 个），跳过内置同名或全局已存在角色，损坏文件仅 `tracing::warn!`。
  3. **工具声明与系统提示词集成 (`src/chat/tools/subagent.rs`, `schema.rs`, `prompt.rs`)**：
     - `build_spawn_role_guidance` 在 `role.lock_model && role.default_model.is_some()` 时输出 `locked model: ... (cannot be overridden)`，在能力收紧时标注 `[no web_search]` / `[no mcp]` / `[no skills]`；
     - `schema.rs` 新增 `turn_tools_for_subagent(surface, capabilities, mcp, subagents)`，按 `capabilities` 过滤 `web_search` 与 MCP 工具；
     - `prompts.rs` 新增 `delegation_mode_guidance(mode, active_variant)`，生成 `<multi_agent_mode>` 英文段落；
     - `src/chat/prompt.rs` 的 `PROMPT_VERSION` 从 `"lx-11"` 递增至 `"lx-12"`，并在 `# Subagents` 中补充 `monitor` 角色说明。
- **验证点**：`cargo test --lib config::subagents` 与 `cargo test --lib chat::tools` 全绿。

---

### Task 3 (T3)：事件层与无头 `exec` 接入（含 `exec/subagent/context.rs` 与 `event/chat/subagent/snapshot.rs` 拆分）
- **目标文件**：
  - 新增 `src/event/chat/subagent/snapshot.rs`（控制 `event/chat/subagent/mod.rs` < 950 行）
  - 新增 `src/chat/agent/exec/subagent/context.rs`（从 981 行的 `exec/subagent/turn.rs` 拆出，使 `turn.rs` 降至 ~740 行）
  - 修改 `src/event/chat/subagent/{mod.rs, model_resolve.rs, context.rs, launch.rs}`、`src/event/chat/mod.rs`
  - 修改 `src/chat/agent/exec/subagent/{mod.rs, turn.rs}`
- **实施细节**：
  1. **模型锁定与 `monitor` 默认 `"low"` 档位 (`model_resolve.rs` & `exec/subagent/turn.rs`)**：
     - 当 `role_spec.lock_model && role_spec.default_model.is_some()` 时，忽略 `spawn_agent` 传入的 `model` 与 `reasoning_effort`；
     - 当 `role.base() == BaseRole::Monitor` 且无显式 `reasoning_effort` 且角色未配 `variant` 时，若目标模型支持 `"low"` 变体则自动选中 `"low"`。
  2. **能力收紧与通信信封注入 (`event/chat/subagent/context.rs` & `exec/subagent/context.rs`)**：
     - `capture_child_prompt_inputs` 根据 `role_spec.capabilities` 决定是否加载 `skills` 与 `mcp`；
     - `assemble_child_context` / `child_context` 对第 2 轮及之后的子会话跟进输入使用 `format_inter_agent_envelope` 包装 `Message Type: MESSAGE | NEW_TASK` 信封。
  3. **保留被中断轮次并注入 `<turn_aborted>` (`event/chat/subagent/context.rs` & `exec/subagent/context.rs`)**：
     - 在 `plan_child_historical_turns` 与 `child_context` 中允许 `assistant.status` 为 `"interrupted"` 或 `"cancelled"` 的历史轮次保留其已执行工具步骤与文本，并在该轮末尾追加 `SUBAGENT_TURN_ABORTED_MARKER`。
  4. **`known_subagent_lines` 与 `delegation_mode_guidance` 注入 (`event/chat/subagent/snapshot.rs` & `event/chat/mod.rs`)**：
     - 复用 `runner::load_subagent_tree` 与 `projected_agent_status` 查询同一根会话树下非 `Cancelled` 的子 Agent，按 `Running -> Blocked -> Idle -> Interrupted -> Failed -> Completed` 排序，取前 8 条（≤ 1024 B）生成 `<subagents>` 环境快照；
     - 在 `event/chat/mod.rs::start_stream` 中合并项目级角色配置 `merge_project_roles`，并将 `known_subagent_lines` 与 `delegation_mode_guidance` 并入本轮 `runtime_summary`。
- **验证点**：`cargo test --lib event::chat` 与 `cargo test --lib chat::agent::exec` 全绿，所有生产文件严格 < 1000 行。

---

### Task 4 (T4)：App 状态与更新层（LRU 内存驻留回收、`AgentFilter` 与内联追问动作）
- **目标文件**：
  - `src/app/effects.rs`
  - `src/app/state/lx/{agent.rs, task.rs}`
  - `src/app/update/chat/subagent.rs`
  - `src/app/update/pane/task_page.rs`
- **实施细节**：
  1. **空闲子会话 LRU 内存回收 (`src/app/state/lx/agent.rs`)**：
     - `SubagentRun` 新增 `pub last_accessed_ms: i64` 与 `pub fn evict_chat_memory(&mut self)`；
     - `AgentsState` 新增 `pub fn evict_excess_idle_chats(&mut self, protected_agent_id: Option<&str>) -> Vec<String>`（阈值 `MAX_RESIDENT_SUBAGENT_CHATS = 6`）、`pub fn filtered_indices(&self, filter: AgentFilter) -> Vec<usize>` 与 `pub fn blocked_count(&self) -> usize`；
     - 在 `on_subagent_finished`、`on_subagent_loaded` 与 `select_task_agent` 中调用回收。
  2. **`AgentFilter` 与内联追问状态 (`src/app/state/lx/task.rs`)**：
     - 定义 `AgentFilter { All, Active, Done }`；
     - `TaskPage` 新增 `pub agent_filter: AgentFilter` 与 `pub agent_reply: Option<TextInput>`；
     - `TaskPage::selected_agent_index(&self, agents: &AgentsState) -> Option<usize>` 通过 `agents.filtered_indices(self.agent_filter)` 映射回 `agents.runs` 的真实下标。
  3. **面板键盘与鼠标更新 (`src/app/update/pane/task_page.rs`)**：
     - 当 `page.agent_reply.is_some()` 时，拦截文本编辑键、`Enter`（触发 `Effect::SendSubagentUserInput` 并清空输入态）与 `Esc`（仅退出输入态）；
     - 当 `page.agent_reply.is_none()` 且在 `Agents` 页签时：
       - `LxInput::Complete`（`Tab`）切换 `page.agent_filter` 并钳制高亮索引；
       - `LxInput::Submit`（`Enter`）对当前高亮非 `Cancelled` 子 Agent 开启 `page.agent_reply = Some(TextInput::new(""))`。
  4. **事件层执行 `Effect::SendSubagentUserInput` (`src/event/chat/subagent/snapshot.rs`)**：
     - 按 `runner::load_subagent_tree` 解析目标记录（支持多级树孙节点），若目标子 Agent 已结束则先自动 `resume`，随后 `send_input(trigger_turn=true)` 并弹窗格内 lx toast。
- **验证点**：`cargo test --lib app::state` 与 `cargo test --lib app::update` 全绿。

---

### Task 5 (T5)：TUI 渲染层（`Agents` 筛选胶囊、树缩进兼容、内联追问输入框与底边框 `blocked` 提醒）
- **目标文件**：
  - `src/ui/main_content/lx/task.rs`
  - `src/ui/main_content/lx/task/agents.rs`
  - `src/ui/main_content/lx/status.rs`
  - `src/ui/text.rs`
- **实施细节**：
  1. **`TAB_ROW` 筛选胶囊与内联输入框 (`src/ui/main_content/lx/task.rs`)**：
     - 当 `page.tab == TaskTab::Agents` 且 `page.agent_reply` 为 `Some(input)` 时，在 `TAB_ROW` 绘制 `To <nickname>: <input>` 及光标；
     - 否则在 `Processes · Agents` 右侧绘制 `all · active · done` 筛选胶囊，并支持 `TaskHit::AgentFilter(AgentFilter)` 点击命中；
     - 底边框 `LX_TASK_HINTS_AGENTS` 增加 `[filter tab]` 与 `[reply ↵]` 提示。
  2. **过滤列表绘制与树缩进兼容 (`src/ui/main_content/lx/task/agents.rs`)**：
     - `draw_list` 与 `draw_agents_detail` 按 `page.agent_filter` 过滤后的可见子 Agent 列表渲染；
     - `tree_prefix` 检查父节点是否在当前过滤可见切片中，若父节点被过滤隐藏则不画悬空连接符。
  3. **底边框 `blocked` 提醒 (`src/ui/main_content/lx/status.rs`)**：
     - `ChatStatus` 新增 `pub blocked_subagents: usize`；
     - `activity_text` 当 `blocked_subagents > 0` 时追加 `N blocked`（或 `1 subagent blocked`）。
- **验证点**：`cargo test --lib ui::main_content::lx` 全绿。

---

### Task 6 (T6)：Web 设置页扩展 (`src/settings/service/subagents.rs`, `assets/settings*`)
- **目标文件**：
  - `src/settings/service/subagents.rs`
  - `assets/settings.html`
  - `assets/settings_subagents.js`
- **实施细节**：
  1. `src/settings/service/subagents.rs`：
     - `/api/subagents/builtins/{explorer,executor,monitor}` 支持 `monitor` 内置角色覆写与重置；
     - JSON 响应与请求体支持 `delegationMode`、`lockModel`、`allowWebSearch`、`allowMcp`、`allowSkills`。
  2. `assets/settings.html` & `assets/settings_subagents.js`：
     - 全局设置卡增加 `Delegation Mode (Auto / Proactive / Explicit)` 下拉选择；
     - 角色编辑器 `Base Role` 下拉框增加 `monitor (monitor & command)`；
     - 角色编辑器增加 `Lock model & reasoning effort`、`Allow web_search`、`Allow MCP tools`、`Allow skills` 复选框，所有界面文案保持纯英文并接入脏状态批量保存。
- **验证点**：`cargo test --lib settings` 全绿。

---

### Task 7 (T7)：全链路单元测试补充与静态检查收口
- **目标文件**：
  - `src/config/subagents/tests.rs`
  - `src/chat/agent/tests/mod.rs`
  - `src/event/chat/tests/subagent.rs`（注：新增事件层测试全部写在此处，保持 `src/event/chat/subagent/tests.rs` 1475 行不再增长）
  - `src/ui/main_content/lx/task/tests.rs`
- **实施细节**：
  1. 补充 `monitor` 角色工具面（仅 4 项工具、任何深度无控制工具、默认 `"low"` 推理档位）测试；
  2. 补充 `lock_model` 忽略调用方模型覆盖、`RoleCapabilities` 裁剪 `web_search`/MCP/Skills、以及 `.lx/agents/*.json` 受信任工作区发现与未受信任工作区隔离测试；
  3. 补充 `DelegationMode`、`<subagents>` 8 条环境快照、被中断轮次保留与 `<turn_aborted>` 注入测试；
  4. 补充 `format_inter_agent_envelope` 模型侧信封包裹与 `MAX_RESIDENT_SUBAGENT_CHATS = 6` LRU 内存卸载测试；
  5. 补充 `Agents` 面板 `AgentFilter` 切换（含过滤视图下树缩进退化保护）、`Enter` 内联追问自动恢复已结束子 Agent、底边框 `blocked` 计数渲染测试；
  6. 执行 `cargo fmt --check`、`cargo clippy --all-targets -- -D warnings` 与受影响模块 `cargo test`。

---

### Task 8 (T8)：Review 专项修复（P0–P3，提交 `fd720210`）
- **目标文件**：
  - `src/db/chat/subagent/{queue.rs, subagent.rs}`、`src/db/chat/mod.rs`
  - `src/event/chat/subagent/{runner.rs, settle.rs, context.rs, snapshot.rs}`、`src/event/chat/mod.rs`
  - `src/chat/agent/exec/{mod.rs, subagent/*}`、`src/chat/agent/{envelope.rs, prompts.rs}`
  - `src/app/state/lx/{task.rs, agent.rs}`、`src/app/update/{pane/task_page.rs, chat/subagent.rs, chat/task_page.rs}`、`src/event/input.rs`
  - `src/config/subagents/{discover.rs, tests.rs}`、`src/settings/service/subagents.rs`
- **实施细节**：
  1. **P0-1 信封与队列语义**：`dedupe_key` 编码 `send:` / `msg:` 前缀与真实调用方会话；新增 `load_subagent_turn_envelopes`、`has_triggerable_queued_input`；`Message MESSAGE` / `NEW_TASK` 按落库 `trigger_turn` 精确选择；`trigger_turn=false` 在 `Complete` 后保持 `idle` 等待显式唤醒；纯图片跟进也包裹信封；`envelope.rs` 幂等检查同时校验 `Sender` 一致性。
  2. **P0-2 中断空块**：追加 `SUBAGENT_TURN_ABORTED_MARKER` 前剔除空的 `AssistantText("")`，无正文时不补前导换行，避免 Anthropic 空 text block 400。
  3. **P0-3 面板追问**：`TaskPage.agent_reply_target` 锁定 `(agent_id, display_label)`；空回车保持输入态；提交前复查 `cancelled`；状态/顺序变更统一走 `restore_selected_task_agent` 保持筛选视图选中身份。
  4. **P1-1 worktree 角色发现**：新增 `merge_project_roles_for_paths(base, workspace, cwd, trusted)`，按 `cwd → linked worktree root → workspace` 合并，TUI 与 `exec` 全路径统一。
  5. **P1-2 `exec` 运行时摘要**：`lx-tui exec` 主会话同步注入 `DelegationMode` 与 `<subagents>` 快照。
  6. **P1-3/4 交互与边界**：面板打开时粘贴不再穿透隐藏草稿；`scroll_task_list` 使用过滤后长度；面板追问在自动 `resume` 前拦截 `close_agent` 关闭目标。
  7. **P2/P3 完善**：`sync_agent_ui_queue` 刷新 `blocked_subagents`；空筛选渲染空态；角色能力收紧写入 `# Capability Restrictions`；设置 API 部分更新保留 `lockModel` / 三项能力；项目角色 JSON 拒绝未知字段与符号链接目录。
- **验证点**：`cargo fmt --check && cargo clippy --all-targets -- -D warnings` 全绿；新增回归单测与 `lx-tui exec` 实机场景见 `verification.md` V07。
