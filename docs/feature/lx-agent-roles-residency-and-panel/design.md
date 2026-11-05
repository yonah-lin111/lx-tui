# lx 子 Agent 角色体系扩展、上下文感知、内存驻留与 TUI 交互增强 — 详细设计 (`design.md`)

- **特性目录**：`docs/feature/lx-agent-roles-residency-and-panel/`
- **关联任务**：[`task.md`](task.md)
- **已合入前置基线（严禁重复）**：`docs/feature/lx-agent-hierarchy-and-guards/`（提交 `339065ce..dc1208c9` 已合入 `dev`，包含模块 A–F：`fork_turns` 历史清洗、`SubagentTokenBudget` 与半程汇报、`send_input.trigger_turn` 与 `changed_paths`、`AgentStatus` 7 态枚举、`wait_agent(all)` 实时列表 / `live_activity` / DFS 树缩进 / `/subagents`、`AgentPath` 多级树与 `max_depth`）
- **参考仓库**：`/Users/yonah/projects/agent/codex-main`
  - `codex-rs/core/assets/agent/builtins/awaiter.toml`、`codex-rs/core/src/agent/{role.rs, child_config.rs}`
  - `codex-rs/agent-roles/src/{discovery.rs, loader.rs, agent_role_config.rs}`
  - `codex-rs/core/src/session/{multi_agents.rs, world_state.rs}`
  - `codex-rs/core/src/context/{multi_agent_mode_instructions.rs, inter_agent_message.rs}`
  - `codex-rs/core/src/tasks/mod.rs`（`interrupted_turn_history_marker`）
  - `codex-rs/core/src/agent/control/{delivery.rs, residency.rs}`
  - `codex-rs/tui/src/app/{agents_overview.rs, agents_overview_view.rs, agent_navigation.rs}`、`codex-rs/tui/src/bottom_pane/pending_thread_approvals.rs`
- **目标范围**：`src/chat/agent/`、`src/chat/tools/`、`src/config/subagents.rs`、`src/event/chat/subagent/`、`src/app/`、`src/ui/main_content/lx/`、`src/settings/`、`assets/`
- **当前状态**：**已实现、已通过 P0–P3 审查修复，验证结果见 [`verification.md`](verification.md)**

---

## 0. 代码库事实核验与合并分支兼容性基线

### 0.1 与刚合入分支 (`339065ce..dc1208c9`) 的衔接与兼容点

核验刚合入 `dev` 的 `lx-agent-hierarchy-and-guards` 及其 P0–P3 审查修复提交（`276d25e8`、`dc1208c9`），本次设计做了以下 5 项精确兼容：

1. **大文件行数红线防护（生产代码 ≤ 1000 行，测试代码 ≤ 1500 行）**：
   - 合入后当前文件真实行数：`src/chat/agent/exec/subagent/turn.rs` 已达 **981 行**、`src/event/chat/subagent/mod.rs` 已达 **948 行**、`src/chat/agent/mod.rs` 已达 **937 行**、`src/event/chat/subagent/runner.rs` 已达 **926 行**、`src/event/chat/subagent/tests.rs` 已达 **1475 行**；
   - **模块化拆分方案**：
     - 从 `src/chat/agent/mod.rs`（937 行）拆出 `src/chat/agent/prompts.rs`（承载 `EXPLORER_PROMPT`、`EXECUTOR_PROMPT`、`MONITOR_PROMPT`、`compose_subagent_prompt*` 与 `delegation_mode_guidance`，`mod.rs` 通过 `pub use prompts::*;` 重导出，降至 ~780 行）；
     - 从 `src/chat/agent/exec/subagent/turn.rs`（981 行）拆出 `src/chat/agent/exec/subagent/context.rs`（承载 `child_context`、图像附件回放、`turn_aborted` 标记与信封包裹，`turn.rs` 降至 ~730 行）；
     - 新增 `src/event/chat/subagent/snapshot.rs`（承载 `known_subagent_lines` `<subagents>` 树快照与 `send_user_input_from_panel` 面板追问调度，避免 `event/chat/subagent/mod.rs` 突破 1000 行）；
     - 新增事件层测试一律写入 `src/event/chat/tests/subagent.rs`（当前 555 行），严禁继续追加到已达 1475 行的 `src/event/chat/subagent/tests.rs`。
2. **复用已合入的多级树查询与三合一寻址设施**：
   - 直接复用 `src/event/chat/subagent/runner.rs` 新增的 `load_subagent_tree(database, workspace, session_id)` 与 `resolve_tree_target(&tree, target)`，以及 `src/chat/agent/status.rs` 的 `projected_agent_status()`、`<rollout_budget>`、`decode_stored_role_and_path`；
   - `decode_stored_role_and_path` 通过 `BaseRole::parse(left)` 解码，扩展 `BaseRole::Monitor` 后自动支持 `"monitor:/root/watcher"` 的零迁移持久化与恢复。
3. **兼容已拆分的 `subagent_view.rs` 与 `subagent_cards.rs`**：
   - 提交 `276d25e8` 已将 `SubagentDetailView` 抽至 `src/app/state/lx/chat/subagent_view.rs`，将卡片刷新抽至 `src/app/update/chat/subagent_cards.rs`；本设计涉及的狀態与更新调用直接对接新模块路径。
4. **兼容 `AgentsState::sort_runs_tree_order()` 先序树重排与树形缩进**：
   - 提交 `276d25e8` 在 `src/app/state/lx/agent.rs:358` 新增了按 `agent_path` 先序（DFS）重排的 `sort_runs_tree_order()`；
   - 模块 K 的 `AgentFilter`（`All / Active / Done`）在保持 `runs` 先序树顺序不变的前提下过滤可见项；当在 `Active` 或 `Done` 过滤视图下某孙节点的父节点被过滤隐藏时，`task/agents.rs::tree_prefix` 自动判断“父节点是否在当前过滤可见集中”，若父节点不可见则不画悬空的 `├─ / └─ `，保证树枝视觉严密。
5. **系统提示词版本递增（`PROMPT_VERSION: "lx-11" -> "lx-12"`）**：
   - 合入分支已将 `src/chat/prompt.rs:28` 的 `PROMPT_VERSION` 升至 `"lx-11"`；本次修改 `# Subagents` 章节（纳入 `monitor` 角色说明与 `<multi_agent_mode>` 委派模式）同步将 `PROMPT_VERSION` 递增为 **`"lx-12"`**。

### 0.2 核心差距对照表（排除模块 A–F 后）

| 核验维度 | 当前 `dev` 分支真实状态（含文件与行号） | 对比 `codex-main` 的核心差距与本次设计切入点 |
| :--- | :--- | :--- |
| **内置角色与长耗时命令监控** | `src/config/subagents.rs:59` 的 `BaseRole` 与 `src/chat/agent/role.rs:19` 的 `AgentRole` 仅支持 `Explorer`（只读 3 工具，无命令执行权）与 `Executor`（7/15 工具，含写文件权）。 | 对齐 `codex-main` 的 `awaiter.toml`（按用户决策命名为 **`monitor`**）：新增第 3 个内置底座角色 `BaseRole::Monitor` / `AgentRole::Monitor`，仅开放 `read`、`search`、`exec_command`、`write_stdin` 4 个工具（严禁写文件与递归派发），默认使用 `"low"` 推理档位与确定性轮询提示词。 |
| **角色模型锁定、能力收紧与项目级发现** | `src/config/subagents.rs:209` 的 `CustomRoleConfig` 仅存于全局 `subagents.json`；`src/event/chat/subagent/model_resolve.rs:45` 无条件允许 `spawn_agent` 的 `model` 参数覆盖角色默认模型；角色无法单独禁用 `web_search` / `mcp` / `skills`。 | 对齐 `codex-main` 的 `agent/role.rs` 与 `agent-roles/discovery.rs`：<br>1. 新增 `lock_model: bool`，开启后忽略调用方 `model`/`reasoning_effort` 覆盖并在工具描述标注锁定；<br>2. 新增 `allow_web_search` / `allow_mcp` / `allow_skills` 细粒度能力收紧开关（只减不增）；<br>3. 受信任工作区自动发现 `<workspace>/.lx/agents/*.json`（兼容 `.agents/roles/*.json`）。 |
| **委派模式、`<subagents>` 环境快照与中断标记** | `src/event/chat/subagent/mod.rs:906` 的 `running_subagent_lines` 仅枚举 `status == "running"` 的内存句柄，子 Agent 转为 `idle`/`completed` 后从环境消失；`src/event/chat/subagent/context.rs:515` 与 `exec/subagent/turn.rs:79` 直接跳过 `assistant.status != "complete"` 的被中断轮次。 | 对齐 `codex-main` 的 `multi_agents.rs`、`world_state.rs:130` 与 `tasks/mod.rs:103`：<br>1. 支持 `DelegationMode`（`auto` / `proactive` / `explicit`）；<br>2. 升级为 `known_subagent_lines`，注入含 `running`/`idle`/`interrupted`/`completed` 的 `<subagents>` 快照（上限 8 条 / 1024 B）；<br>3. 子会话保留被中断轮次的已完成步骤并注入 `<turn_aborted>` 英文引导标记。 |
| **Agent 间结构化信封与内存驻留回收** | `src/event/chat/subagent/context.rs:363` 将 `send_input` 的后续输入作为裸文本直接喂给模型；`src/app/state/lx/agent.rs:325` 的 `AgentsState.runs` 永久保留所有已加载子会话的完整 `LxChat` 消息树与 `PaneCache`。 | 对齐 `codex-main` 的 `inter_agent_message.rs` 与 `residency.rs`：<br>1. 新增 `src/chat/agent/envelope.rs`，对非首轮 `send_input` 在模型请求侧自动包裹 `Message Type: MESSAGE \| NEW_TASK` 结构化信封（UI 气泡保持干净原文）；<br>2. 实现空闲子会话 LRU 驻留回收（`MAX_RESIDENT_SUBAGENT_CHATS = 6`），超限自动卸载 `chat`/`cache`，选中时从 SQLite 懒加载。 |
| **TUI `Agents` 面板筛选、内联追问与阻塞提醒** | `src/ui/main_content/lx/task/agents.rs` 与 `src/app/update/pane/task_page.rs` 仅支持上下浏览与 `^d`/`^a` 停止，不支持状态过滤、不支持在面板内直接给子 Agent 发消息；`status.rs:324` 不提示 `blocked` 子 Agent。 | 对齐 `codex-main` TUI 能力：<br>1. `Agents` 页签支持 `Tab` 键与鼠标切换 `all · active · done` 三态筛选；<br>2. 按 `Enter` 激活顶栏单行内联输入 `To <nickname>: `，直接发送 `send_input`（对已结束子 Agent 自动先 `resume` 再发，支持深度 1 与深度 2 节点）；<br>3. 主界面底边框 `status.rs` 新增 `N blocked` 异常提醒。 |

---

## 1. 目标与非目标

### 1.1 目标（Goals）
1. **模块 H — 内置 `monitor` 角色（长耗时命令与进程监控专职 Agent）**：
   - `BaseRole` 与 `AgentRole` 新增 `Monitor`（名称 `"monitor"`），对应新增 `ToolSurface::Monitor`（仅开放 `read`、`search`、`exec_command`、`write_stdin` 4 项工具）；
   - 任何深度下均不开放写文件工具与子 Agent 控制工具（恒为叶子节点）；
   - 未显式指定推理强度时默认优先选取 `"low"`（模型不支持时回退默认档位），内置对齐 Codex `awaiter.toml` 的英文系统提示词。
2. **模块 G — 角色模型锁定（`lock_model`）、细粒度能力收紧与项目级角色自动发现**：
   - 在 `RoleOverrideConfig` 与 `CustomRoleConfig` 新增 `lock_model: bool`、`allow_web_search: bool`、`allow_mcp: bool`、`allow_skills: bool`；
   - `lock_model == true` 且角色配了 `model` 时，在 `spawn_agent` 工具描述追加 `[locked model: ...]`，并在运行时静默忽略调用方传入的 `model` / `reasoning_effort`；
   - 收紧开关只减不增，在子 Agent 工具声明构建与系统提示组装时精确剥离对应能力；
   - 当 `workspace_trusted == true` 时，自动扫描 `<workspace>/.lx/agents/*.json` 与 `<workspace>/.agents/roles/*.json`（复用 `CustomRoleConfig` JSON 结构，零新 crate 依赖）。
3. **模块 I — 委派积极度模式（`DelegationMode`）、全量 `<subagents>` 环境快照与 `<turn_aborted>` 中断标记**：
   - `SubagentsSection` 新增 `delegation_mode: DelegationMode`（`auto` | `proactive` | `explicit`，默认 `auto`），在高阶思考档位或显式配置下向主会话注入对应英文委派策略提示；
   - 将 `running_subagent_lines` 升级为 `known_subagent_lines`（抽入 `src/event/chat/subagent/snapshot.rs`），向主会话环境摘要注入 `<subagents>` 列表（含 `running`/`idle`/`interrupted`/`completed` 可复用子 Agent，最多 8 条 / 1024 B）；
   - 修复子会话历史回放（交互式 `context.rs` 与无头 `exec/subagent/context.rs`）丢弃被中断轮次的问题，保留被中断轮次的已完成部分并在下一轮注入 `<turn_aborted>` 英文引导标记。
4. **模块 J — 结构化通信信封（`InterAgentMessage`）与空闲子会话 LRU 内存驻留回收（`Residency`）**：
   - 新增 `src/chat/agent/envelope.rs`，对后续 `send_input` 投递的输入在组装 `ModelContext` 时按 `trigger_turn` 包裹 `Message Type: MESSAGE` 或 `Message Type: NEW_TASK` 信封，保持 DB 与 TUI 历史原文干净；
   - `AgentsState` 实现 `MAX_RESIDENT_SUBAGENT_CHATS = 6` 的 LRU 内存回收：自动卸载超出上限的空闲子会话 `LxChat` 与 `PaneCache`，再次选中时复用 `Effect::LoadSubagentSession` 从 SQLite 懒加载。
5. **模块 K — TUI `Agents` 面板状态筛选、内联追问与主界面阻塞提醒（`src/ui/main_content/lx/`）**：
   - `TaskPage` 支持 `AgentFilter`（`All` / `Active` / `Done`），可通过 `Tab` 键或点击 `all · active · done` 切换；
   - 在 `Agents` 列表选中子 Agent 按 `Enter` 打开单行内联输入框 `To <nickname>: `，回车发送（对已结束子 Agent 自动 `resume + send_input`），`Esc` 退出输入态；
   - 主界面底边框 `status.rs` 在存在 `blocked` 子 Agent 时追加 `N blocked` 提醒。

### 1.2 非目标（Non-Goals）
- **不重复实现 `lx-agent-hierarchy-and-guards` 的任何功能**：直接复用已合入的 `AgentStatus`、`AgentPath`、`SubagentTokenBudget`、`ForkSpec`、`send_input.trigger_turn`、`changed_paths`、`wait_agent(all)`、`live_activity`、`sort_runs_tree_order` 与 `/subagents`。
- **不引入 `toml` 等新外部 crate**：项目级角色配置文件统一采用与 `~/.lx-t/config/*.json` 同源的 `serde_json` 格式。
- **不破坏数据库 Schema 兼容性**：所有新增能力均复用现有 `SCHEMA_VERSION = 22` 表结构，无需新增数据库迁移。

---

## 2. Grill Me 已确认决策汇总表

| 编号 | 决策维度 | 用户已确认结论 |
| :--- | :--- | :--- |
| **Q1** | **功能模块范围** | **G + H + I + J + K 全部纳入**：角色锁定/能力收紧/项目级发现（G）、内置 `monitor` 角色（H）、委派模式/`<subagents>` 快照/`<turn_aborted>` 标记（I）、结构化通信信封与 LRU 内存驻留回收（J）、TUI `Agents` 面板筛选/内联追问/底栏阻塞提醒（K）。 |
| **Q2** | **第 3 内置角色命名与权限边界（模块 H）** | 1. 角色名称定为 **`monitor`**（`BaseRole::Monitor` / `AgentRole::Monitor` / `ToolSurface::Monitor`）；<br>2. 仅开放 `read`、`search`、`exec_command`、`write_stdin` **4 个工具**（严禁写文件与递归派发，恒为叶子节点）；<br>3. 默认优先使用 `"low"` 推理强度（不支持时回退模型默认），内置确定性轮询与指数退避等待英文提示词；设置页 `baseRole` 同步支持 `monitor`。 |
| **Q3** | **模型锁定、能力收紧与项目级角色发现（模块 G）** | 1. 角色配置新增 `lock_model: bool`（默认 `false`）：开启且配了 `model` 时，工具描述标注 `[locked model: ...]`，运行时静默忽略调用方传入的 `model`/`reasoning_effort`；<br>2. 新增 `allow_web_search`、`allow_mcp`、`allow_skills`（默认均 `true`），只能在 `base_role` 权限上做减法；<br>3. 受信任工作区自动扫描 `<workspace>/.lx/agents/*.json` 与 `<workspace>/.agents/roles/*.json`（上限 64 KiB/文件、最多 32 个，全局同名优先，禁止覆盖内置角色）。 |
| **Q4** | **委派模式、`<subagents>` 快照与 `<turn_aborted>`（模块 I）** | 1. `subagents.json` 新增 `delegation_mode`: `auto`（默认，高推理档位自动切 `proactive`）\| `proactive` \| `explicit`；<br>2. 环境摘要升级为 `known_subagent_lines`，注入含 `running`/`idle`/`interrupted`/`completed` 的 `<subagents>` 快照（最多 8 条 / 1024 B）；<br>3. 子会话回放保留被中断轮次的已完成部分，并在其后注入英文 `<turn_aborted>` 引导标记。 |
| **Q5** | **结构化消息信封与 LRU 内存回收（模块 J）** | 1. 仅对后续 `send_input` 在组装模型请求时包裹 `Message Type: MESSAGE \| NEW_TASK\nTask name: ...\nSender: ...\nPayload:\n...`，DB 与 TUI 保持干净原文；<br>2. `AgentsState` 设定非运行态已加载聊天树上限 `MAX_RESIDENT_SUBAGENT_CHATS = 6`，超出时按 LRU 自动卸载 `chat` 与 `cache` 并置 `loaded = false`，再次选中时从 SQLite 懒加载。 |
| **Q6** | **TUI `Agents` 面板筛选、内联追问与阻塞提醒（模块 K）** | 1. `Agents` 页签支持 `Tab` 键与鼠标点击切换 `All / Active / Done` 筛选；<br>2. 选中子 Agent 按 `Enter` 在面板顶栏展开单行输入 `To <nickname>: `，回车发送（对已结束子 Agent 自动 `resume + send_input(trigger_turn=true)` 并弹窗格内 lx toast），`Esc` 退出输入态；<br>3. 主界面底边框 `status.rs` 在存在 `blocked` 子 Agent 时追加 `N blocked` 提醒。 |

---

## 3. 架构与模块详细设计（含关键代码）

### 3.1 模块 H：内置 `monitor` 角色 (`src/config/subagents.rs`, `src/chat/agent/role.rs`, `src/chat/agent/prompts.rs`)

#### 3.1.1 扩展 `BaseRole`、`AgentRole` 与 `ToolSurface::Monitor`
在 `src/config/subagents.rs` 中扩展 `BaseRole`（由于 `src/chat/agent/path.rs:129` 的 `decode_stored_role_and_path` 直接调用 `BaseRole::parse`，新增 `"monitor"` 分支后，DB `agent_path` 列的 `"monitor:/root/..."` 编码/解码天然生效）：

```rust
/// 子 agent 的基础权限档位（决定工具面与是否允许递归派发）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum BaseRole {
    /// 只读调查（`read`, `search`, `web_search`）。
    #[default]
    Explorer,
    /// 读写执行（`read`, `search`, `web_search`, `edit`, `write`, `exec_command`, `write_stdin`）。
    Executor,
    /// 长耗时命令与进程监控（`read`, `search`, `exec_command`, `write_stdin`；无写文件与派发权限）。
    Monitor,
}

impl BaseRole {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Explorer => "explorer",
            Self::Executor => "executor",
            Self::Monitor => "monitor",
        }
    }

    pub fn parse(raw: &str) -> Option<Self> {
        match raw.trim() {
            "explorer" => Some(Self::Explorer),
            "executor" => Some(Self::Executor),
            "monitor" => Some(Self::Monitor),
            _ => None,
        }
    }

    pub const fn default_description(self) -> &'static str {
        match self {
            Self::Explorer => {
                "Read-only codebase investigator (`read`, `search`, `web_search`). \
                 Fast and authoritative for scoped questions; spawn multiple in parallel \
                 for independent lookups."
            }
            Self::Executor => {
                "Read-write worker (`read`, `search`, `web_search`, `edit`, `write`, \
                 `exec_command`, `write_stdin`). Assign clear file ownership when running \
                 in parallel."
            }
            Self::Monitor => {
                "Long-running command and process monitor (`exec_command`, `write_stdin`, \
                 `read`, `search`). Use for lengthy builds, test suites, or background process \
                 polling without file-modification risk."
            }
        }
    }
}
```

在 `src/chat/agent/mod.rs` 中定义 `ToolSurface::Monitor` 与 `MONITOR_TOOLS`：

```rust
/// monitor 工具名白名单（4 项：只读文件检索 + 命令执行/轮询，严禁 edit/write 与控制工具）。
pub const MONITOR_TOOLS: [&str; 4] = ["read", "search", "exec_command", "write_stdin"];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolSurface {
    Main,
    Explorer,
    Executor,
    ExecutorCoordinator,
    /// 专职监控子 agent：4 项工具，任何深度均不含写文件与控制工具。
    Monitor,
    ExecMain,
}

impl ToolSurface {
    pub const fn from_role_at_depth(role: AgentRole, depth: usize, max_depth: usize) -> Self {
        match role.base() {
            BaseRole::Explorer => Self::Explorer,
            BaseRole::Monitor => Self::Monitor,
            BaseRole::Executor => {
                if depth < max_depth {
                    Self::ExecutorCoordinator
                } else {
                    Self::Executor
                }
            }
        }
    }
}
```

#### 3.1.2 抽离 `src/chat/agent/prompts.rs` 并定义 `MONITOR_PROMPT`
为将 `src/chat/agent/mod.rs`（当前 937 行）控制在 800 行以内，把子 Agent 角色提示词与组装函数抽至 `src/chat/agent/prompts.rs`（由 `mod.rs` `pub use prompts::*;` 导出）：

```rust
/// monitor 角色提示（对齐 Codex awaiter.toml，专职确定性命令执行与状态轮询）。
pub const MONITOR_PROMPT: &str = r#"You are a monitor subagent: a deterministic command runner and process watcher working inside the user's workspace.

Your job is to execute or monitor the specific command, test suite, build, or background process assigned to you and report its terminal outcome accurately.

Rules:
- Use `exec_command` and `write_stdin` to launch or poll the target task, and `read` / `search` only to inspect relevant logs or output files.
- If a process is still running, continue polling with `write_stdin` using progressively longer yield timeouts (exponential backoff) until it reaches a terminal state.
- Do NOT hallucinate completion, do NOT perform unrelated exploration, and never attempt to modify source files.
- When the monitored task finishes (or fails), return a concise, factual report containing the exit status, key log excerpts, and failure diagnostics."#;
```

在 `src/event/chat/subagent/model_resolve.rs`（及 `exec/subagent/turn.rs`）中，当 `role.base() == BaseRole::Monitor` 且调用方 `reasoning_effort` 与角色 `default_model.variant` 均未指定时，若目标模型 `variants` 包含 `"low"`，自动将生效 `variant` 设为 `"low"`（否则保持默认）。

---

### 3.2 模块 G：角色模型锁定、细粒度能力收紧与项目级角色发现 (`src/config/subagents.rs`, `src/config/subagents/discover.rs`)

#### 3.2.1 角色配置结构扩展 (`RoleCapabilities` & `lock_model`)
在 `src/config/subagents.rs` 中为 `RoleOverrideConfig`、`CustomRoleConfig` 与 `ResolvedRoleSpec` 增加能力收紧与模型锁定字段（全部带 `#[serde(default)]` 与默认 `true`/`false` 函数，100% 兼容已有 `subagents.json`）：

```rust
const fn default_true() -> bool {
    true
}

/// 角色在 `BaseRole` 基础上的细粒度能力收紧配置（只减不增）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RoleCapabilities {
    /// 是否允许使用 `web_search` 工具（仅对原本含 `web_search` 的 Explorer/Executor 有效，Monitor 恒无）。
    #[serde(default = "default_true")]
    pub allow_web_search: bool,
    /// 是否允许加载并调用外部 MCP 工具。
    #[serde(default = "default_true")]
    pub allow_mcp: bool,
    /// 是否允许在系统提示词中注入 Skills 目录。
    #[serde(default = "default_true")]
    pub allow_skills: bool,
}

impl Default for RoleCapabilities {
    fn default() -> Self {
        Self {
            allow_web_search: true,
            allow_mcp: true,
            allow_skills: true,
        }
    }
}
```

在 `RoleOverrideConfig` 与 `CustomRoleConfig` 中新增：
```rust
    /// 是否锁定角色的模型与思考等级（为 true 且配置了 `model` 时，忽略 `spawn_agent` 的覆盖传参）。
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub lock_model: bool,
    /// 细粒度能力收紧开关（默认全部开启）。
    #[serde(default, flatten)]
    pub capabilities: RoleCapabilities,
```

`BuiltinRoleOverrides` 新增 `pub monitor: RoleOverrideConfig`，`ResolvedRoleSpec` 同步新增：
```rust
pub struct ResolvedRoleSpec {
    pub name: String,
    pub base_role: BaseRole,
    pub is_builtin: bool,
    /// 是否来自项目级 `.lx/agents/*.json` 或 `.agents/roles/*.json`（设置页标记只读来源标签）。
    pub is_project_scoped: bool,
    pub description: String,
    pub system_prompt_override: Option<String>,
    pub default_model: Option<ModelSelection>,
    pub lock_model: bool,
    pub capabilities: RoleCapabilities,
    pub nickname_candidates: Vec<String>,
}
```

#### 3.2.2 模型锁定与能力收紧的执行点
1. **`build_spawn_role_guidance` (`src/chat/tools/subagent.rs`)**：
   - 当 `role.lock_model && role.default_model.is_some()` 时，模型说明格式输出为：
     ` (base: explorer, locked model: openai/gpt-4o (low) — cannot be overridden)`；
   - 当 `capabilities` 关闭了某项能力时，追加简短提示（如 `[no web_search, no mcp]`）。
2. **`resolve_subagent_model` (`src/event/chat/subagent/model_resolve.rs` & `exec/subagent/turn.rs`)**：
   ```rust
   let (effective_model_arg, effective_effort_arg) =
       if role_spec.lock_model && role_spec.default_model.is_some() {
           (None, None)
       } else {
           (model_override, reasoning_effort)
       };
   ```
3. **工具面与提示词收紧 (`src/event/chat/subagent/context.rs` & `src/chat/tools/schema.rs`)**：
   - 新增 `turn_tools_for_subagent(surface: ToolSurface, capabilities: RoleCapabilities, mcp: &McpToolSnapshot, subagents: Option<&SubagentsSection>) -> Vec<ToolDecl>`：
     - 当 `!capabilities.allow_mcp` 时，传入空的 `&McpToolSnapshot::default()`；
     - 当 `!capabilities.allow_web_search` 时，从返回的 `Vec<ToolDecl>` 中 `retain(|decl| decl.name != "web_search")`；
   - 在 `capture_child_prompt_inputs` 中：
     - 当 `!capabilities.allow_skills` 时，`skills = None`；
     - 当 `!capabilities.allow_mcp` 时，`mcp = Arc::new(McpToolSnapshot::default())`。

#### 3.2.3 项目级角色自动发现 (`src/config/subagents/discover.rs`)
对齐 `src/skills/discover.rs` 与 `src/chat/agent/instructions.rs` 的安全边界：
- **扫描目录**：仅当工作区已信任（`is_trusted(cwd) || is_trusted(workspace)`）时，按优先级依次扫描：
  1. 窗格/`exec -C` 执行目录 `cwd`（支持仓库子目录）；
  2. `cwd` 所属的 linked worktree 根（`git::linked_worktree_root(cwd)`）；
  3. 会话仓库根 `workspace`（主 checkout 或普通仓库根）。
  每个根目录内再依次扫描 `.lx/agents/` 与 `.agents/roles/`；同名角色按上述优先级先到先得，全局 `subagents.json` 与内置角色名最高优先。TUI 主会话、TUI 子 Agent 解析与 `lx-tui exec` 共用 `merge_project_roles_for_paths`，避免“声明角色用 worktree、实际 spawn 用主 checkout”的路径分裂。
- **文件约束**：
  - 仅读取顶层 `.json` 普通文件（拒绝符号链接文件与符号链接目录，拒绝隐藏文件）；
  - 单文件大小上限 `MAX_ROLE_FILE_BYTES = 64 * 1024`（64 KiB），单工作区最多加载 `MAX_PROJECT_ROLES = 32` 个角色；
  - 若 JSON 内省略 `"name"` 字段，默认取文件名去掉 `.json` 后缀；
  - 复用 `CustomRoleConfig::normalized()` 与单角色校验逻辑；
  - **未知字段拒绝**：反序列化使用 `#[serde(flatten)] extra` 收集未知键；只要出现未知字段（例如把 `allowMcp` 误写成 `allow_mcp`），整份文件记录 `tracing::warn!` 后跳过，防止能力收紧因拼写错误静默 fail-open；
  - **防覆盖保护**：若角色名为 `"explorer"` / `"executor"` / `"monitor"`，或与全局 `subagents.json` 中已定义的自定义角色重名，跳过该文件并记录 `tracing::warn!`（全局用户配置优先于仓库配置）。

---

### 3.3 模块 I：委派积极度模式、`<subagents>` 环境快照与 `<turn_aborted>` 中断标记

#### 3.3.1 委派积极度模式 `DelegationMode` (`src/config/subagents.rs`, `src/chat/agent/prompts.rs`)
在 `src/config/subagents.rs` 中定义：

```rust
/// 主 agent 的子任务委派积极度模式（对齐 Codex `MultiAgentMode`）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum DelegationMode {
    /// 默认智能模式：当主会话推理档位为 `high`/`xhigh`/`max` 时自动采用 `Proactive`，否则保持标准平衡策略。
    #[default]
    Auto,
    /// 主动并行拆解：面对多模块调查、独立文件修改或长耗时验证时优先并行委派子 agent。
    Proactive,
    /// 仅显式委派：仅在用户明确要求委派/子 agent，或需要后台盯长耗时进程时才调用 `spawn_agent`。
    Explicit,
}
```

在 `src/chat/agent/prompts.rs` 中提供 `delegation_mode_guidance(mode: DelegationMode, active_variant: Option<&str>) -> Option<&'static str>`：
- 当解析为生效 `Proactive`（`mode == Proactive` 或 `mode == Auto && matches!(active_variant, Some("high" | "xhigh" | "max"))`）时，在主会话本轮 `runtime_summary` 或系统提示词追加：
  ```xml
  <multi_agent_mode>
  Mode: proactive. When a task spans multiple independent codebase areas, distinct implementation modules, or long-running test/build verification, proactively decompose the work and delegate in parallel using `spawn_agent` (`explorer`, `executor`, `monitor`). Reuse existing subagents listed in `<subagents>` via `send_input` or `resume_agent` whenever their prior context applies.
  </multi_agent_mode>
  ```
- 当 `mode == Explicit` 时，追加：
  ```xml
  <multi_agent_mode>
  Mode: explicit. Only spawn subagents when the user explicitly asks for delegation/parallelism or when monitoring a long-running background command with `monitor`. For routine lookups and edits, perform the work directly yourself.
  </multi_agent_mode>
  ```

#### 3.3.2 已知子 Agent 环境快照 `known_subagent_lines` (`src/event/chat/subagent/snapshot.rs`)
新建 `src/event/chat/subagent/snapshot.rs`，直接复用 `runner::load_subagent_tree` 与 `projected_agent_status` 构建结构化 `<subagents>` 快照（上限 `MAX_SNAPSHOT_SUBAGENTS = 8` 条，总字节上限 `MAX_SNAPSHOT_SUBAGENT_BYTES = 1024`）：

```rust
pub const MAX_SNAPSHOT_SUBAGENTS: usize = 8;
pub const MAX_SNAPSHOT_SUBAGENT_BYTES: usize = 1024;

/// 生成当前根会话树下已知子 agent 的结构化环境摘要（含 running / idle / interrupted / completed，排除 cancelled）。
pub(crate) fn known_subagent_lines(
    streams: &ChatStreams,
    database: Option<&Database>,
    workspace: Option<&Path>,
    parent_session: &str,
) -> Vec<String> {
    // 1. 若有 DB 与 workspace，调用 runner::load_subagent_tree(database, workspace, parent_session) 获取整棵树记录，
    //    并调用 super::projected_agent_status(streams, record, queue) 投影实时 AgentStatus；
    //    若无 DB（纯内存测试），回退遍历 streams.subagents 中的运行句柄。
    // 2. 过滤掉 AgentStatus::Cancelled（已关闭不可复用）。
    // 3. 按优先级排序：Running (0) -> Blocked (1) -> Idle (2) -> Interrupted (3) -> Failed (4) -> Completed (5)，同级按 created_at_ms 倒序。
    // 4. 输出格式（严格限制 ≤ 8 条且总长 ≤ 1024 B）：
    // <subagents>
    // - path=/root/turing id=sub_123 nickname=Turing role=explorer status=idle · Auth flow lookup
    // </subagents>
}
```

#### 3.3.3 子会话 `<turn_aborted>` 中断历史标记 (`src/event/chat/subagent/context.rs` & `src/chat/agent/exec/subagent/context.rs`)
在交互式 `src/event/chat/subagent/context.rs` 的 `plan_child_historical_turns` 与无头 `src/chat/agent/exec/subagent/context.rs` 的 `child_context` 中：
1. 将原本的 `if user.status != "complete" || assistant.status != "complete" { continue; }` 放宽为允许 `assistant.status` 为 `"complete"`、`"interrupted"` 或 `"cancelled"`（只要 `user.status == "complete"`）；
2. 当历史轮次的 `assistant.status` 为 `"interrupted"` 或 `"cancelled"` 时，在该轮 `assistant_items` 尾部追加 `ModelItem::UserText(SUBAGENT_TURN_ABORTED_MARKER.to_string())`（并在 `turns` 的 `assistant.text` 尾部附上标记），对齐 Codex `TurnAborted::INTERRUPTED_GUIDANCE`：
   ```rust
   pub const SUBAGENT_TURN_ABORTED_MARKER: &str = "<turn_aborted>\n\
   The previous turn was interrupted before completion. Any tool calls shown above were already \
   executed; do not repeat completed edits. Follow the new instructions below.\n\
   </turn_aborted>";
   ```

---

### 3.4 模块 J：结构化通信信封 (`InterAgentMessage`) 与空闲子会话 LRU 内存驻留回收 (`Residency`)

#### 3.4.1 结构化 Agent 间通信信封 (`src/chat/agent/envelope.rs`)
新建 `src/chat/agent/envelope.rs`，1:1 对齐 `codex-main/codex-rs/core/src/context/inter_agent_message.rs`：

```rust
//! Agent 间结构化消息信封（仅在组装模型请求时包裹，保持 DB 与 UI 文本干净）。

/// 消息投递类型（对齐 Codex `InterAgentMessageType`）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InterAgentMessageType {
    /// `send_input(trigger_turn = false)`：仅入队补充上下文。
    Message,
    /// `send_input(trigger_turn = true)`：唤醒或分配新子任务。
    NewTask,
}

impl InterAgentMessageType {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Message => "MESSAGE",
            Self::NewTask => "NEW_TASK",
        }
    }

    pub const fn from_trigger_turn(trigger_turn: bool) -> Self {
        if trigger_turn {
            Self::NewTask
        } else {
            Self::Message
        }
    }
}

/// 将发送给子 agent 的后续跟进文本包裹为带归属路径的结构化信封。
pub fn format_inter_agent_envelope(
    message_type: InterAgentMessageType,
    recipient_path: &str,
    sender_path: &str,
    payload: &str,
) -> String {
    let trimmed = payload.trim();
    if trimmed.starts_with("Message Type: ") {
        return payload.to_string();
    }
    format!(
        "Message Type: {}\nTask name: {}\nSender: {}\nPayload:\n{}",
        message_type.as_str(),
        recipient_path.trim(),
        sender_path.trim(),
        trimmed
    )
}
```

- **接入点 (`src/event/chat/subagent/context.rs` & `src/chat/agent/exec/subagent/context.rs`)**：
  - 首轮 `spawn_agent` 的初始任务（即子会话自身的第 1 条非克隆用户消息）保持原始 `user_text` 不变；
  - 后续通过 `send_input` 投递的跟进轮次，在组装 `ChatTurn` 与 `ModelItem` 发送给模型时包裹结构化信封；类型由落库的 `trigger_turn` 决定：`trigger_turn=true → NEW_TASK`，`trigger_turn=false → MESSAGE`；
  - **零 schema 变更的触发类型持久化**：`send_input` 的 `dedupe_key` 前缀编码 `send:<caller_session>:...`（唤醒）或 `msg:<caller_session>:...`（仅入队），`SubagentInputRecord::trigger_turn()` / `caller_session_id()` 与 `Database::load_subagent_turn_envelopes` 从该列派生当前轮及历史轮的 `(MessageType, sender_session)`；
  - **发送方真实归因**：`Sender` 取实际调用方会话的树路径（`resolve_session_tree_root(caller_session)`），而非目标子 Agent 的 `parent_session`；从 TUI 面板跨层追问或跨分支 `send_input` 时不会误标发送方；
  - **纯图片跟进**：`ModelItem::UserContent` 文本为空时，以 `"[N image(s)]"` 作为 payload 包裹信封头，保证图片轮同样携带归属信息；
  - **幂等与防伪造**：若 payload 已以 `Message Type: ` 开头，仅当其中 `Sender: {sender}` 与本次调用一致时才原样返回，否则重新包裹，避免历史文本伪造发送方；
  - `trigger_turn=false` 仅入队：运行轮自然完成后 `settle.rs` 仅在队列存在可触发项（`has_triggerable_queued_input`，即非 `msg:` 前缀）时才自动续轮；否则子 Agent 保持 `idle`（`queued_count > 0`），等待显式 `resume_agent` 或下一条 `trigger_turn=true` 输入；
  - 数据库 `agent_message` 与 TUI `SubagentRun.chat` 中存储的仍是原始干净文本，用户点开 `Agents` 详情面板不会看到协议头噪音。

#### 3.4.2 空闲子会话 LRU 内存驻留回收 (`src/app/state/lx/agent.rs`)
在 `src/app/state/lx/agent.rs` 中设定驻留常量与回收方法：

```rust
/// 内存中同时保留完整 `LxChat` 消息树的非运行态子 agent 最大数量（对齐 Codex `V2Residency`）。
pub const MAX_RESIDENT_SUBAGENT_CHATS: usize = 6;
```

1. `SubagentRun` 新增 `pub last_accessed_ms: i64`（初始化为 `completed_at_ms.unwrap_or(started_at_ms)`，每次在 `Agents` 面板被选中、收到新消息或完成加载时更新）；
2. 在 `SubagentRun` 上新增卸载方法：
   ```rust
   /// 卸载空闲子会话的内存消息树（保留元数据、Token 统计与汇报摘要，后续选中时从 DB 懒加载）。
   pub fn evict_chat_memory(&mut self) {
       if self.is_running() || !self.loaded {
           return;
       }
       self.chat = LxChat::new();
       self.compact = LxCompact::default();
       self.loaded = false;
       self.total_rows = 0;
       self.rows_dirty = true;
   }
   ```
3. 在 `AgentsState` 上新增 `evict_excess_idle_chats(&mut self, protected_agent_id: Option<&str>) -> Vec<String>`：
   - 找出所有 `!run.is_running() && run.loaded && Some(run.agent_id.as_str()) != protected_agent_id` 的候选下标；
   - 若已加载的非运行态子会话总数 `> MAX_RESIDENT_SUBAGENT_CHATS`，按 `last_accessed_ms` 升序（最久未访问优先）依次调用 `evict_chat_memory()`，直至驻留数 `<= MAX_RESIDENT_SUBAGENT_CHATS`；
   - 在 `on_subagent_finished`、`on_subagent_loaded` 与 `select_task_agent` 尾部自动调用该回收函数。

---

### 3.5 模块 K：TUI `Agents` 面板状态筛选、内联追问与主界面阻塞提醒 (`src/ui/main_content/lx/`, `src/app/`)

#### 3.5.1 `AgentFilter` 三态筛选与树缩进兼容 (`src/app/state/lx/task.rs`, `src/ui/main_content/lx/task/agents.rs`)
在 `src/app/state/lx/task.rs` 中定义：

```rust
/// Agents 面板列表的状态筛选器。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AgentFilter {
    /// 全部子 agent。
    #[default]
    All,
    /// 仅活动态（`running` / `idle` / `blocked`）。
    Active,
    /// 仅已结束态（`completed` / `failed` / `interrupted` / `cancelled`）。
    Done,
}

impl AgentFilter {
    pub const ALL: [Self; 3] = [Self::All, Self::Active, Self::Done];

    pub const fn label(self) -> &'static str {
        match self {
            Self::All => "all",
            Self::Active => "active",
            Self::Done => "done",
        }
    }

    pub const fn next(self) -> Self {
        match self {
            Self::All => Self::Active,
            Self::Active => Self::Done,
            Self::Done => Self::All,
        }
    }

    pub fn matches(self, run: &crate::app::state::SubagentRun) -> bool {
        let status = crate::chat::agent::AgentStatus::parse(&run.status)
            .unwrap_or(crate::chat::agent::AgentStatus::Completed);
        match self {
            Self::All => true,
            Self::Active => matches!(
                status,
                crate::chat::agent::AgentStatus::Running
                    | crate::chat::agent::AgentStatus::Idle
                    | crate::chat::agent::AgentStatus::Blocked
            ),
            Self::Done => matches!(
                status,
                crate::chat::agent::AgentStatus::Completed
                    | crate::chat::agent::AgentStatus::Failed
                    | crate::chat::agent::AgentStatus::Interrupted
                    | crate::chat::agent::AgentStatus::Cancelled
            ),
        }
    }
}
```

- `TaskPage` 新增：
  - `pub agent_filter: AgentFilter`（默认 `AgentFilter::All`）；
  - `pub agent_reply: Option<crate::app::overlay::TextInput>`（内联追问输入框，`None` 表示普通列表浏览态）；
  - `pub agent_reply_target: Option<(String, String)>`（打开输入态时锁定的 `(agent_id, display_label)`，防止列表重排/状态更新后追问串发到其他子 Agent）。
- **筛选视图下的选中稳定性**：
  - 所有对 `runs` 顺序或状态的修改（`set_subagent_list`、`on_subagent_started`、`on_subagent_finished`）都先捕获当前选中的 `agent_id`，修改后再通过 `restore_selected_task_agent` 映射回新的过滤下标；禁止在状态变更后重新读取 `filtered_indices` 再决定选中项；
  - `TaskPage::agent_reply_target` 在点击其他列表行、切换筛选、切换 Tab 时同步清空 `agent_reply`，避免“隐形输入态”；
- **与 `sort_runs_tree_order` 及 `tree_prefix` 的兼容**：
  - `AgentsState::filtered_indices(filter) -> Vec<usize>` 按先序保留满足 `filter.matches(run)` 的真实下标；
  - `TaskPage::len(agents)` 在 `TaskTab::Agents` 下返回 `agents.filtered_indices(self.agent_filter).len()`；
  - `TaskPage::selected_agent_index(agents) -> Option<usize>` 将 `self.agents_window.active`（过滤后列表中的第 N 项）映射回 `agents.runs` 的真实下标；
  - 在 `task/agents.rs::tree_prefix` 中，传入过滤后的可见切片 `visible_runs: &[&SubagentRun]`：仅当 `run.depth() >= 2` 且其父路径 `parent_path(&run.agent_path)` 也在 `visible_runs` 中时才绘制 `├─ / └─ ` 缩进连接符，防止父节点被过滤隐藏时出现悬空树枝。

#### 3.5.2 面板内联追问与自动恢复 (`Enter` 激活输入，`Effect::SendSubagentUserInput`)
1. **进入与退出内联输入态 (`src/app/update/pane/task_page.rs`)**：
   - 当 `page.tab == TaskTab::Agents` 且 `page.agent_reply.is_none()` 时，按 **`Enter` (`LxInput::Submit`)**：
     - 取当前高亮的 `SubagentRun`；若其状态为 `"cancelled"`，弹窗格内 toast `"Cannot send input to a closed subagent"`；
     - 否则初始化 `page.agent_reply = Some(TextInput::new(""))`；
   - 当 `page.agent_reply.is_some()` 时：
     - `TAB_ROW` 替换为渲染 `To <nickname>: <input_text>`（带光标渲染）；
     - 字符输入、退格、左右移动、行首行尾、词删除全部委托给 `TextInput`；
     - 按 **`Esc` (`LxInput::Cancel`)**：置 `page.agent_reply = None`（仅退出内联输入，不关闭 `TaskPage`）；
     - 按 **`Enter` (`LxInput::Submit`)**：若输入非空，取出文本并发出 `Effect::SendSubagentUserInput { pane, session_id, agent_id, text }`，同时置 `page.agent_reply = None`。
2. **事件层自动恢复并投递 (`src/event/chat/subagent/snapshot.rs`)**：
   - 提交时优先使用 `agent_reply_target` 锁定的 `agent_id`（缺失时才回退当前高亮），保证输入期间列表重排/过滤变化不会改变收件人；
   - 空回车保持输入态，不发送空消息，也不误退；
   - 若锁定目标已变为 `cancelled`（`close_agent` 关闭），弹窗格内 toast `"Cannot send input to a closed subagent"` 并丢弃本条输入；
   - 粘贴（`Cmd+V`）在输入态单行化写入 `agent_reply`；浏览态由 `apply_task_paste` 直接吞掉，禁止穿透污染隐藏的主聊天草稿框；
   - 处理 `Effect::SendSubagentUserInput` 时：
     - 先通过 `runner::load_subagent_tree` + `runner::resolve_tree_target` 找到目标 `SubagentRecord`（支持深度 1 直接子节点与深度 2 孙节点，取其真实 `record.parent_session_id` 构造 `RequestKey`）；
     - 若目标子 Agent 尚未处于 `running` 或 `resumed_subagents`（即已 `completed` / `interrupted` / `failed`），先调用 `runner::resume(state, streams, persistence, &key, &record.agent_id)` 将其恢复为活动态；
     - 紧接着调用 `runner::send_input(..., &[SubagentInputItem::Text { text }], false, true)` 投递并立即触发新一轮；
     - 成功后在窗格内显示 lx toast：`format!("Sent input to {}", display_name)`。

#### 3.5.3 主界面底边框 `blocked` 提醒 (`src/ui/main_content/lx/status.rs`)
1. `AgentsState` 新增纯查询方法 `pub fn blocked_count(&self) -> usize`（统计 `run.status == "blocked"` 的数量）；
2. `ChatStatus` 新增 `pub blocked_subagents: usize`（在 `refresh_status` 中赋值为 `pane.lx.agents.blocked_count()`）；
3. `status.rs::activity_text(tasks, subagents, blocked_subagents, available)`：
   - 当 `blocked_subagents > 0` 时，在候选标签中加入 `text::lx_subagent_blocked_label(blocked_subagents)`（如 `"1 subagent blocked"` 或与 running 合并为 `"2 subagents · 1 blocked"`），让用户在主界面一眼感知阻塞异常。

---

## 4. 模块划分与单文件行数预算（已对齐合并后最新行号）

严格遵守项目规范（**生产代码单文件上限 1000 行，拆分子文件控制在 500 行左右；测试单文件 ≤ 1500 行**）：

| 文件路径 | 变更类型 | 当前行数 → 预计行数 | 职责说明 |
| :--- | :--- | :--- | :--- |
| `src/chat/agent/envelope.rs` | 新增 | 0 → ~90 行 | `InterAgentMessageType` 与 `format_inter_agent_envelope` 结构化消息信封 |
| `src/chat/agent/prompts.rs` | 新增（从 `mod.rs` 拆出） | 0 → ~210 行 | `EXPLORER_PROMPT`、`EXECUTOR_PROMPT`、`MONITOR_PROMPT`、`compose_subagent_prompt*`、`delegation_mode_guidance` |
| `src/chat/agent/mod.rs` | 修改（瘦身） | 937 → ~780 行 | 导出 `envelope`/`prompts`、新增 `ToolSurface::Monitor` 与 `MONITOR_TOOLS` |
| `src/chat/agent/role.rs` | 修改 | 155 → ~190 行 | `AgentRole::Monitor`、`tool_surface_at_depth` 支持 `Monitor` |
| `src/config/subagents/discover.rs` | 新增 | 0 → ~160 行 | 受信任工作区 `.lx/agents/*.json` 与 `.agents/roles/*.json` 自动发现与合并 |
| `src/config/subagents.rs` | 修改 | 610 → ~720 行 | `BaseRole::Monitor`、`RoleCapabilities`、`lock_model`、`DelegationMode` 及校验 |
| `src/chat/tools/subagent.rs` | 修改 | 235 → ~270 行 | `build_spawn_role_guidance` 展示 `locked model` 与能力收紧说明 |
| `src/chat/tools/schema.rs` | 修改 | 260 → ~310 行 | `turn_tools_for_subagent` 按 `RoleCapabilities` 裁剪 `web_search` 与 MCP 工具 |
| `src/chat/prompt.rs` | 修改 | 834 → ~845 行 | `PROMPT_VERSION` 升至 `"lx-12"`，更新 `# Subagents` 章节说明 |
| `src/event/chat/subagent/snapshot.rs` | 新增 | 0 → ~180 行 | `known_subagent_lines`（8 条 `<subagents>` 树快照）与 `send_user_input_from_panel`（面板内联追问自动恢复投递） |
| `src/event/chat/subagent/mod.rs` | 修改（瘦身） | 948 → ~910 行 | 迁出 `running_subagent_lines` 至 `snapshot.rs`，分发 `Effect::SendSubagentUserInput` |
| `src/event/chat/subagent/model_resolve.rs` | 修改 | 294 → ~340 行 | `lock_model` 强制锁定模型/思考等级、`Monitor` 默认 `"low"` 档位回退 |
| `src/event/chat/subagent/context.rs` | 修改 | 617 → ~680 行 | 按 `RoleCapabilities` 剥离 skills/mcp、非首轮 `send_input` 信封包裹、被中断轮次保留与 `<turn_aborted>` 注入 |
| `src/chat/agent/exec/subagent/context.rs` | 新增（从 `turn.rs` 拆出） | 0 → ~270 行 | 无头模式 `child_context`、图像附件重放、`<turn_aborted>` 注入与信封包裹 |
| `src/chat/agent/exec/subagent/turn.rs` | 修改（瘦身） | 981 → ~740 行 | 迁出 `child_context` 与图像重放至 `exec/subagent/context.rs`，接入 `lock_model` 与 `Monitor` 默认档位 |
| `src/app/state/lx/agent.rs` | 修改 | 410 → ~490 行 | `MAX_RESIDENT_SUBAGENT_CHATS = 6` LRU 内存回收 `evict_excess_idle_chats`、`filtered_indices`、`blocked_count` |
| `src/app/state/lx/task.rs` | 修改 | 411 → ~490 行 | `AgentFilter`（`All`/`Active`/`Done`）、`agent_reply: Option<TextInput>` 与过滤索引映射 |
| `src/app/update/pane/task_page.rs` | 修改 | 193 → ~280 行 | `Tab` 切换筛选、`Enter` 激活/提交内联追问、`Esc` 取消输入 |
| `src/ui/main_content/lx/task.rs` | 修改 | 589 → ~670 行 | `TAB_ROW` 渲染筛选胶囊 `all · active · done` 与内联输入框 `To <nickname>: ` |
| `src/ui/main_content/lx/task/agents.rs` | 修改 | 276 → ~320 行 | 基于 `AgentFilter` 过滤后的子 Agent 列表渲染与可见父节点树缩进判定 |
| `src/ui/main_content/lx/status.rs` | 修改 | 434 → ~460 行 | 底边框 `activity_text` 增加 `blocked_subagents` 提醒 |
| `src/settings/service/subagents.rs` & `assets/settings*` | 修改 | 各 <560 行 | Web 设置页支持 `monitor` 底座、`delegationMode`、`lockModel` 与 3 个能力收紧开关 |
| `src/event/chat/tests/subagent.rs` | 修改 | 555 → ~780 行 | 承载新增的事件层单测（不动已达 1475 行的 `event/chat/subagent/tests.rs`） |

---

## 5. 风险、边界条件与验证策略

1. **防权限逃逸（Capability Narrowing Invariant）**：
   - `RoleCapabilities` 仅做减法：`BaseRole::Monitor` 即使 `allow_web_search = true` 也不会获得 `web_search`（因为 `ToolSurface::Monitor` 本身不含 `web_search`）；`Explorer` 与 `Monitor` 永远不会获得写文件或递归控制工具。
2. **项目级角色文件安全隔离**：
   - 未受信任的工作区（`workspace_trusted == false`）绝不读取 `.lx/agents/*.json` 或 `.agents/roles/*.json`；
   - 项目级角色禁止覆盖内置角色名（`explorer` / `executor` / `monitor`）及全局用户自定义角色，防止恶意仓库劫持默认角色提示词。
3. **多级树下的面板内联追问与过滤缩进**：
   - 在 `Agents` 面板对深度 2 的孙节点（如 `/root/coord/leaf`）按 `Enter` 追问时，事件层通过 `record.parent_session_id`（即 `/root/coord` 的会话 ID）入队并唤醒，不会误挂到根会话名下；
   - 当过滤为 `Active` 或 `Done` 导致父节点不在可见列表时，子节点自动省略悬空的 `├─ / └─ ` 前缀。
4. **精准验证命令**：
   - `cargo test --lib config::subagents`
   - `cargo test --lib chat::agent`
   - `cargo test --lib event::chat`
   - `cargo test --lib app::update`
   - `cargo test --lib ui::main_content::lx`
   - `cargo clippy --all-targets -- -D warnings`
   - `cargo fmt --check`
