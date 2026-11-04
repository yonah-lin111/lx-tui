# lx 子 Agent 多级树、资源护栏、历史清洗与 TUI 增强 — 详细设计 (`design.md`)

- **特性目录**：`docs/feature/lx-agent-hierarchy-and-guards/`
- **关联任务**：[`task.md`](task.md)
- **参考仓库**：`/Users/yonah/projects/agent/codex-main`
  - `codex-rs/core/src/agent/{control.rs, registry.rs, status.rs, history.rs, role.rs, child_config.rs}`
  - `codex-rs/core/src/agent/control/{spawn.rs, completion.rs, budget.rs, legacy.rs, interrupt.rs}`
  - `codex-rs/core/src/rollout_budget.rs`、`codex-rs/core/src/context/rollout_budget.rs`
  - `codex-rs/protocol/src/agent_path.rs`
  - `codex-rs/tui/src/{multi_agents.rs, slash_command.rs}`、`codex-rs/tui/src/app/{agent_navigation.rs, agent_status_feed.rs}`
- **目标范围**：`src/chat/agent/`、`src/chat/agent_loop/`、`src/chat/tools/`、`src/config/subagents.rs`、`src/db/chat/`、`src/event/chat/subagent/`、`src/app/`、`src/ui/main_content/lx/`、`src/settings/`、`assets/`
- **当前状态**：**设计完成，待用户审核确认后执行**

---

## 0. 代码库事实核验与前置基线

通过 `codegraph` 与 `codebase-memory-mcp` 检索核实，当前代码库真实状态与关键差距如下：

| 核验维度 | 当前 `lx-tui` 真实状态（含文件与行号） | 对比 `codex-main` 的核心差距与设计切入点 |
| :--- | :--- | :--- |
| **递归深度与工具面** | `src/chat/agent/mod.rs:39-105` 定义了 4 个固定 `ToolSurface`（`Main`/`ExecMain`/`Explorer`/`Executor`）；`src/chat/agent_loop/mod.rs:778` 写死 `if context.is_subagent()` 拦截所有控制工具调用，深度硬编码为 1。 | 对齐 `codex-main` 的 `agent_max_depth` 与 `AgentPath`：支持可配置 `max_depth`（`1..=3`，默认 `2`）；当 `BaseRole::Executor` 所处深度 `< max_depth` 时开放控制工具（`ToolSurface::ExecutorCoordinator`），到达 `max_depth` 自动收紧为无控制工具的 `ToolSurface::Executor`，`Explorer` 恒为只读 3 工具。 |
| **路径存储与数据库兼容** | `src/db/migration/schema.rs:192-209` 中 `agent_subagent` 与 `agent_session` 表**均已有 `agent_path TEXT` 列**；但 `src/db/chat/subagent.rs:314` 与 `record.rs:100` 目前仅把 `agent_path` 当作 `base_role`（存 `"explorer"` 或 `"executor"`）使用。 | **零破坏性 Schema 迁移**：将 `agent_path` 列的存储格式升级为自解释兼容串 `"{base_role}:{canonical_path}"`（如 `"executor:/root/turing"`），解析时兼容旧记录纯 `"explorer"` / `"executor"`（自动回退为 `/root/<nickname_or_id>`），同时完整持久化权限基线与规范路径。 |
| **分叉历史拷贝 (`fork`)** | `src/db/chat/fork.rs:400-601` 的 `copy_completed_history_to_child` 全量拷贝父会话 `< boundary` 的所有消息与所有 `agent_message_part`（包含冗长的 `reasoning` 思考块）及父级 token 字段，不支持限制最近轮次数。 | 对齐 `codex-main` 的 `agent/history.rs`（`prepare_forked_history`）：支持 `fork_turns: Option<u32>`（只截取最近 `N` 个已完成轮次），并在拷贝时**剥离父级 `reasoning` 思考块、清零克隆消息的 token 计费字段、清除父级子 Agent 汇报标记**。 |
| **Token 预算与中断半程汇报** | `src/chat/agent_loop/mod.rs:674` 无任何预算或提醒机制；`src/event/chat/subagent/settle.rs:63` 对 `StopReason::Cancelled`（含 `interrupt_agent`）直接跳过报告生成（`report_view = None`），且报告只有 `files_changed: usize` 数字。 | 1:1 对齐 `codex-main` 的 `RolloutBudget`：不设死板的步数上限，支持可选 `max_tokens` 预算，在剩余 **25% / 10%** 时注入 `<rollout_budget>` 催促子 Agent 收尾，耗尽时优雅熔断；`interrupt_agent` 与预算熔断均产出含 `changed_paths: Vec<String>` 的**半程汇报（Partial Report）**。 |
| **消息投递与状态枚举** | `send_input` 只要子 Agent 处于 `idle` 就强制立即启动新轮次；状态字符串（`"running"`/`"idle"` 等）散落在 `event/chat/subagent/mod.rs:44`、`exec/subagent/mod.rs:36`、`app/state/lx/agent.rs:33` 三处。 | 在 `send_input` 增加 `trigger_turn: bool`（默认 `true`，`false` 对齐 `QueueOnly` 仅入队不唤醒）；新增 `src/chat/agent/status.rs` 强类型 `AgentStatus` 统一收口状态枚举与投影函数。 |
| **TUI 卡片与 `Agents` 面板** | `src/ui/main_content/lx/history/tool/subagent.rs:287` 在 `wait_agent(all)` 时只画一行静态 `■ All`（`app/update/chat/subagent.rs:188` 跳过刷新）；主历史看不到运行中子 Agent 的实时动作；`task/agents.rs` 无层级缩进、无 Token 显示、`^d` 只停单个却标为 `stop all`。 | `wait_agent(all)` 改为实时展示所有被等待子 Agent 节点；`spawn_agent` 卡片运行中显示单行实时动作预览（如 `$ cargo test`）；`Agents` 面板支持父子树形缩进、Token 用量展示、`[stop ^d] [stop all ^a]` 双键位，并新增 `/subagents` 斜杠命令。 |

---

## 1. 目标与非目标

### 1.1 目标（Goals）
1. **模块 A — 分叉历史清洗与按轮次分叉 (`fork_turns` + History Sanitization)**：
   - `spawn_agent` 新增可选参数 `fork_turns: u32`（`0..=64`），兼容原有 `fork_context: bool`；
   - 分叉拷贝时自动剥离父会话 `reasoning` 思考块、清零克隆历史消息上的 token 计费字段、清除父级 `subagent_report_json`。
2. **模块 B — 对齐 `codex-main` 的 Token 预算护栏与半程汇报 (`RolloutBudget` + Partial Report)**：
   - 支持在 `subagents.json` 配置可选全局默认预算 `default_max_tokens: Option<i64>`，并在 `spawn_agent` 支持可选 `max_tokens: u64` 单次覆写；
   - 剩余额度跌破 **25%** 与 **10%** 两个阈值时，向子 Agent 注入 `<rollout_budget>` 提醒；耗尽 100% 额度时自动中止当前轮次（标记为 `interrupted` + `budget_exceeded`）并保持 `idle` 可恢复状态；
   - 子 Agent 因 `interrupt_agent`、Token 预算耗尽或运行失败停止时，自动提取已生成的最后文本段、已完成工具进展与已修改文件列表生成半程汇报（仅 `close_agent` 不生成报告）。
3. **模块 C — 双模消息投递 (`trigger_turn`) 与报告变更文件明细 (`changed_paths`)**：
   - `send_input` 新增可选 `trigger_turn: bool`（默认 `true`；`false` 仅入队 `queued` 不唤醒 `idle` 子 Agent；禁止 `interrupt=true` 与 `trigger_turn=false` 同传）；
   - `SubagentReportView` 与 `<subagent_report>` 新增 `changed_paths: Vec<String>`（最多 32 条去重相对路径），保留 `files_changed: usize` 总数。
4. **模块 D — 状态机与领域逻辑收口 (`AgentStatus`)**：
   - 新增 `src/chat/agent/status.rs`，定义强类型 `AgentStatus`（`Running / Idle / Blocked / Completed / Failed / Cancelled / Interrupted`）及统一的 `projected_agent_status()`，消除 TUI 与无头 `exec` 的重复裸字符串逻辑。
5. **模块 E — TUI 呈现与交互补齐 (`src/ui/main_content/lx/`)**：
   - 修复 `wait_agent(all)` 只显示 `■ All` 的缺陷，实时回填并逐行渲染所有被等待子 Agent 的状态、标题、点击跳转与展开报告；
   - 运行中的 `spawn_agent` 卡片在主历史展示单行实时动作摘要（Live Activity，如 `$ cargo test`、`edit src/a.rs`、`Thinking...`），完成后自动收起；
   - `Agents` 面板（`Ctrl+O`）支持按 `agent_path` 层级渲染树形缩进（`├─ ` / `└─ `）、展示紧凑 Token 用量，并区分 `[stop ^d]`（停止选中项及其子树）与 `[stop all ^a]`（停止全部运行中子 Agent）；
   - 新增 `/subagents`（别名 `/agents`）斜杠命令直达 `Agents` 面板。
6. **模块 F — 多级 Agent 树、规范路径与级联管控 (`AgentPath` + `max_depth`)**：
   - 新增 `src/chat/agent/path.rs` 实现 `AgentPath`（根节点 `/root`，子路径 `/root/turing/curie`）；`spawn_agent` 新增可选 `task_name`，未传时由 `nickname` 自动派生蛇形路径段；
   - 支持配置 `max_depth`（`1..=3`，默认 `2`）与整树共享 `max_concurrent`（`1..=12`，默认 `4`）；仅 `BaseRole::Executor` 在 `depth < max_depth` 时可继续派发子 Agent；
   - 控制工具支持 `agent_id` / `nickname` / `agent_path`（绝对或相对路径）三合一寻址，`list_agents` 支持可选 `path_prefix` 过滤，`close_agent` / `interrupt_agent` 支持沿树级联停止存活后代。

### 1.2 非目标（Non-Goals）
- **不赋予只读 `Explorer` 派发权限**：无论处于第几层，`BaseRole::Explorer` 永远只拥有只读调查工具，不参与递归派发。
- **不引入硬编码步数一刀切（`max_steps`）**：严格对齐 `codex-main`，采用 Token 预算阈值提醒 + 耗尽熔断，不因机械步数中断正常多文件任务。
- **不进行破坏性数据库整表重建**：完全复用现有 `SCHEMA_VERSION = 22` 的表结构与 `agent_path` 列，100% 兼容已有历史会话数据。

---

## 2. Grill Me 已确认决策汇总表

| 编号 | 决策维度 | 用户已确认结论 |
| :--- | :--- | :--- |
| **Q1** | **功能模块范围** | **A + B + C + D + E + F 全部纳入**：分叉清洗与 `fork_turns`（A）、Token 预算护栏与半程汇报（B）、`trigger_turn` 与报告文件列表（C）、`AgentStatus` 强类型收口（D）、TUI 卡片/面板/命令增强（E）、多级 Agent 树与 `AgentPath`（F）。 |
| **Q2** | **多级树深度、角色权限与整树并发** | 1. `max_depth` 范围 `1..=3`，默认 `2`；到达 `max_depth` 时移除控制工具，强行调用返回 `"Agent depth limit reached. Solve the task yourself."`；<br>2. 仅 `BaseRole::Executor` 在 `< max_depth` 时可继续派发，`BaseRole::Explorer` 永远不派发；<br>3. `max_concurrent`（范围 `1..=12`，默认 `4`）按**整棵根会话树（Root Session Tree）**共享统计；关闭父级子 Agent 时级联关闭其所有存活后代。 |
| **Q3** | **路径生成、三合一寻址与 `list_agents`** | 1. 根节点固定 `/root`；`spawn_agent` 新增可选 `task_name`（`[a-z0-9_]{1,32}`），未传则由 `nickname` 自动转小写蛇形生成路径段（如 `/root/turing/curie`）；<br>2. 所有控制工具 `target` 支持 `agent_id` / `nickname` / `agent_path`（绝对或相对当前调用者）三合一寻址；<br>3. `list_agents` 新增可选 `path_prefix`；DB `agent_path` 列采用零迁移兼容格式 `"{base_role}:{canonical_path}"`。 |
| **Q4** | **分叉轮次 (`fork_turns`) 与历史清洗** | 1. `spawn_agent` 新增可选 `fork_turns: u32`（`0..=64`），`N > 0` 截取父会话最近 `N` 个已完成轮次，兼容保留 `fork_context: bool`；<br>2. 分叉强制执行三项清洗：剥离 `reasoning`/`thinking` 思考块、克隆消息的 token 计费字段置 `NULL`、清除父级 `subagent_report_json`。 |
| **Q5** | **Token 预算护栏与半程汇报** | 1. 1:1 对齐 `codex-main`：不设步数限制，支持可选 `max_tokens`（全局 `default_max_tokens` + `spawn_agent` 单次 `max_tokens` 覆写，默认 `None` 不限，推荐开启基准值 `100_000`）；<br>2. 剩余额度跌破 **25% / 10%** 时在下一次采样前注入 `<rollout_budget>` 提醒；耗尽时以 `interrupted` (`budget_exceeded`) 熔断；<br>3. `interrupt_agent` / 预算熔断 / 失败均自动产出包含已完成正文与 `changed_paths` 的半程汇报；仅 `close_agent` 不产出报告。 |
| **Q6** | **双模投递 (`trigger_turn`)、变更文件列表与状态收口** | 1. `send_input` 新增可选 `trigger_turn: bool`（默认 `true`；`false` 仅入队 `queued` 不唤醒 `idle` 子 Agent）；<br>2. `SubagentReportView` 新增 `changed_paths: Vec<String>`（最多 32 条相对路径）；<br>3. 新增 `src/chat/agent/status.rs` 强类型 `AgentStatus` 枚举统一收口状态与投影。 |
| **Q7** | **TUI 交互与视觉呈现** | 1. `wait_agent(all)` 卡片实时逐行展示所有被等待子 Agent 及其状态与报告；<br>2. 主历史 `spawn_agent` 卡片在运行中展示单行实时动作摘要（Live Activity）；<br>3. `Agents` 面板列表支持多级树缩进（`├─ ` / `└─ `）、紧凑 Token 用量、以及 `[stop ^d] [stop all ^a]` 双键位；<br>4. 新增 `/subagents`（别名 `/agents`）斜杠命令直接打开 `Agents` 面板。 |

---

## 3. 架构与模块详细设计（含关键代码）

### 3.1 模块 D & F：`AgentStatus` 强类型状态机与 `AgentPath` 层级路径 (`src/chat/agent/status.rs`, `src/chat/agent/path.rs`)

#### 3.1.1 强类型状态枚举 `AgentStatus` (`src/chat/agent/status.rs`)
消除原本分散在 `event/chat/subagent/mod.rs`、`chat/agent/exec/subagent/mod.rs` 与 `app/state/lx/agent.rs` 中的裸字符串常量，统一在领域层定义：

```rust
//! 子 agent 生命周期状态与投影规则（TUI 与无头 exec 共用唯一真相）。

use serde::{Deserialize, Serialize};

/// 子 agent 对外可见的统一状态。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AgentStatus {
    /// 正在执行某轮采样或工具调用。
    Running,
    /// 处于活动待命态（已显式恢复或被 `interrupt_agent` 暂停，可随时接收输入）。
    Idle,
    /// 队首输入校验失败（如图片缺失），等待取消该条输入。
    Blocked,
    /// 最新一轮正常完成且未处于挂起待命态。
    Completed,
    /// 最新一轮执行失败（协议错误、模型报错或未恢复异常）。
    Failed,
    /// 被 `close_agent` 主动关闭（不产生报告）。
    Cancelled,
    /// 被 `interrupt_agent`、预算熔断或崩溃恢复中断。
    Interrupted,
}

impl AgentStatus {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Running => "running",
            Self::Idle => "idle",
            Self::Blocked => "blocked",
            Self::Completed => "completed",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
            Self::Interrupted => "interrupted",
        }
    }

    pub fn parse(raw: &str) -> Option<Self> {
        match raw.trim() {
            "running" | "pending" => Some(Self::Running),
            "idle" => Some(Self::Idle),
            "blocked" => Some(Self::Blocked),
            "completed" | "complete" => Some(Self::Completed),
            "failed" => Some(Self::Failed),
            "cancelled" => Some(Self::Cancelled),
            "interrupted" => Some(Self::Interrupted),
            _ => None,
        }
    }

    /// 是否仍在运行中。
    pub const fn is_running(self) -> bool {
        matches!(self, Self::Running)
    }

    /// 是否已到达终态（可供 `wait_agent` 结算）。
    ///
    /// 注意：`Running` 不是终态；`Idle` 若是由 `interrupt_agent` 或 `budget_exceeded`
    /// 刚停下的轮次，对等待该轮结果的 `wait_agent` 而言已停止运行，由调用方结合 `is_running` 判定。
    pub const fn is_settled(self) -> bool {
        !matches!(self, Self::Running)
    }
}

/// 统一投影函数：由运行时在途标记、恢复集合、队首阻塞状态与 DB 持久状态计算出对外 `AgentStatus`。
pub fn projected_agent_status(
    is_running_in_memory: bool,
    is_resumed_or_paused: bool,
    has_blocked_head: bool,
    db_status: &str,
) -> AgentStatus {
    if is_running_in_memory {
        return AgentStatus::Running;
    }
    if has_blocked_head {
        return AgentStatus::Blocked;
    }
    if is_resumed_or_paused {
        return AgentStatus::Idle;
    }
    AgentStatus::parse(db_status).unwrap_or(AgentStatus::Completed)
}
```

#### 3.1.2 层级规范路径 `AgentPath` 与 DB 零迁移编解码 (`src/chat/agent/path.rs`)
参考 `codex-main/codex-rs/protocol/src/agent_path.rs`，定义树路径与深度计算：

```rust
//! 层级化 Agent 路径（`/root`、`/root/turing`、`/root/turing/curie`）与零迁移存储编解码。

use crate::config::subagents::BaseRole;

/// 根会话固定路径。
pub const ROOT_AGENT_PATH: &str = "/root";
/// 单个路径段最大长度。
pub const MAX_PATH_SEGMENT_LEN: usize = 32;
/// 默认最大递归深度（1 = 仅主 Agent 派发子 Agent；2 = 子 Agent 可派发孙 Agent）。
pub const DEFAULT_MAX_AGENT_DEPTH: usize = 2;
/// 允许配置的最大递归深度上限。
pub const MAX_AGENT_DEPTH_LIMIT: usize = 3;
/// 超过最大深度时的稳定错误文本（对齐 codex-main）。
pub const DEPTH_LIMIT_REACHED_CODE: &str = "Agent depth limit reached. Solve the task yourself.";

/// 校验单个 `task_name` / 路径段：1..=32 字符，仅允许 `[a-z0-9_]`，且不得为 `"root"`。
pub fn is_valid_path_segment(segment: &str) -> bool {
    let bytes = segment.as_bytes();
    if bytes.is_empty() || bytes.len() > MAX_PATH_SEGMENT_LEN || segment == "root" {
        return false;
    }
    bytes
        .iter()
        .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || *b == b'_')
}

/// 将昵称（如 `"Turing"`、`"Turing #2"`、`"vonNeumann"`）规范化为合法路径段（`"turing"`、`"turing_2"`、`"von_neumann"`）。
pub fn slugify_segment(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len().min(MAX_PATH_SEGMENT_LEN));
    let mut prev_underscore = false;
    for ch in raw.trim().chars() {
        if ch.is_ascii_alphanumeric() {
            if ch.is_ascii_uppercase() && !out.is_empty() && !prev_underscore {
                // 驼峰如 vonNeumann -> von_neumann
            }
            out.push(ch.to_ascii_lowercase());
            prev_underscore = false;
        } else if !out.is_empty() && !prev_underscore {
            out.push('_');
            prev_underscore = true;
        }
        if out.len() >= MAX_PATH_SEGMENT_LEN {
            break;
        }
    }
    while out.ends_with('_') {
        out.pop();
    }
    if out.is_empty() || out == "root" {
        "agent".to_string()
    } else {
        out
    }
}

/// 计算路径所处深度：`/root` -> 0，`/root/turing` -> 1，`/root/turing/curie` -> 2。
pub fn path_depth(agent_path: &str) -> usize {
    let trimmed = agent_path.trim().trim_end_matches('/');
    if trimmed.is_empty() || trimmed == ROOT_AGENT_PATH {
        return 0;
    }
    trimmed
        .strip_prefix("/root/")
        .map_or(1, |rest| rest.split('/').filter(|s| !s.is_empty()).count())
}

/// 在父路径下拼接子路径段，并确保在 `existing_paths` 中唯一（冲突时自动追加 `_2`, `_3`）。
pub fn allocate_child_path(
    parent_path: &str,
    preferred_segment: &str,
    existing_paths: &[String],
) -> String {
    let parent = {
        let p = parent_path.trim().trim_end_matches('/');
        if p.is_empty() || !p.starts_with("/root") {
            ROOT_AGENT_PATH
        } else {
            p
        }
    };
    let base_seg = if is_valid_path_segment(preferred_segment) {
        preferred_segment.to_string()
    } else {
        slugify_segment(preferred_segment)
    };
    let candidate = format!("{parent}/{base_seg}");
    if !existing_paths.iter().any(|p| p.eq_ignore_ascii_case(&candidate)) {
        return candidate;
    }
    let mut suffix = 2usize;
    loop {
        let next = format!("{parent}/{base_seg}_{suffix}");
        if !existing_paths.iter().any(|p| p.eq_ignore_ascii_case(&next)) {
            return next;
        }
        suffix = suffix.saturating_add(1);
    }
}

/// 编码 `BaseRole` 与 `agent_path` 到数据库 `agent_path` 列：`"executor:/root/turing"`。
pub fn encode_stored_role_and_path(base: BaseRole, canonical_path: &str) -> String {
    format!("{}:{}", base.as_str(), canonical_path.trim())
}

/// 从数据库 `agent_path` 列解码 `(Option<BaseRole>, Option<String>)`：
/// - 新格式 `"executor:/root/turing"` -> `(Some(Executor), Some("/root/turing"))`
/// - 旧格式 `"explorer"` / `"executor"` -> `(Some(Explorer/Executor), None)`
/// - 纯路径 `"/root/turing"` -> `(None, Some("/root/turing"))`
pub fn decode_stored_role_and_path(stored: Option<&str>) -> (Option<BaseRole>, Option<String>) {
    let Some(raw) = stored.map(str::trim).filter(|s| !s.is_empty()) else {
        return (None, None);
    };
    if let Some((left, right)) = raw.split_once(':') {
        let base = BaseRole::parse(left);
        let path = right.trim();
        let path_opt = (!path.is_empty()).then(|| path.to_string());
        return (base, path_opt);
    }
    if let Some(base) = BaseRole::parse(raw) {
        return (Some(base), None);
    }
    if raw.starts_with('/') {
        return (None, Some(raw.to_string()));
    }
    (None, None)
}
```

#### 3.1.3 三合一寻址器（ID / 昵称 / 相对或绝对路径）
当调用者（位于 `caller_path`，主 Agent 为 `"/root"`，子 Agent 为如 `"/root/turing"`）传入 `target` 时，按以下确定性优先级匹配同一根会话树下的目标子 Agent：
1. **精确 `agent_id` 匹配**：`record.agent_id == target`；
2. **规范路径匹配**：
   - 若 `target` 以 `"/root/"` 开头：精确匹配 `record.agent_path`（大小写不敏感）；
   - 若 `target` 不含 `'/'` 或为相对路径（如 `"curie"` 或 `"turing/curie"`）：先拼接为 `format!("{caller_path}/{target}")` 匹配直接子路径，未命中再匹配 `format!("/root/{target}")`；
3. **昵称匹配**：`record.nickname` 大小写不敏感匹配 `target`。

---

### 3.2 模块 F：多级 Agent 树、角色递归权限与级联控制 (`src/chat/agent/mod.rs`, `src/db/chat/subagent.rs`, `src/event/chat/subagent/`)

#### 3.2.1 扩展 `ToolSurface` 支持未达深度上限的协调型 `Executor`
在 `src/chat/agent/mod.rs` 中新增 `ToolSurface::ExecutorCoordinator`（拥有 `Executor` 的 7 个读写/命令工具 + 8 个子 Agent 控制工具 = 15 项工具，不含交互式 `ask` 与主会话专属 `update_plan`）：

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolSurface {
    /// 主 agent：18 项工具（10 基础 + 8 控制）。
    Main,
    /// 只读调查：3 项工具（`read`, `search`, `web_search`），任何深度均不含控制工具。
    Explorer,
    /// 叶节点或已达 `max_depth` 的实现者：7 项工具（`read`, `search`, `web_search`, `edit`, `write`, `exec_command`, `write_stdin`）。
    Executor,
    /// 未达 `max_depth` 的实现者（`depth < max_depth`）：7 项实现工具 + 8 项子 agent 控制工具（共 15 项）。
    ExecutorCoordinator,
    /// CLI exec 无头主 agent：17 项工具（剔除 `ask`）。
    ExecMain,
}

const EXECUTOR_COORDINATOR_TOOLS: [&str; 15] = [
    "read",
    "search",
    "web_search",
    "edit",
    "write",
    "exec_command",
    "write_stdin",
    "spawn_agent",
    "send_input",
    "interrupt_agent",
    "resume_agent",
    "wait_agent",
    "close_agent",
    "list_agents",
    "cancel_agent_input",
];
```

- **工具面选取函数**：
  ```rust
  impl AgentRole {
      pub const fn tool_surface_at_depth(self, depth: usize, max_depth: usize) -> ToolSurface {
          match self.base() {
              BaseRole::Explorer => ToolSurface::Explorer,
              BaseRole::Executor => {
                  if depth < max_depth {
                      ToolSurface::ExecutorCoordinator
                  } else {
                      ToolSurface::Executor
                  }
              }
          }
      }
  }
  ```
- **`run_agent_turn` 递归守卫更新 (`src/chat/agent_loop/mod.rs:776`)**：
  将原本一刀切的 `if context.is_subagent()` 替换为检查当前会话是否被授予控制工具（或深度是否超限）：
  - 若 `context.subagent_depth >= context.max_subagent_depth` 或当前角色 `base == BaseRole::Explorer`：调用 `spawn_agent` 返回 `ToolFailure::new(ToolErrorCode::Io, DEPTH_LIMIT_REACHED_CODE)`；
  - 若 `context.subagent_depth < context.max_subagent_depth` 且 `base == BaseRole::Executor`：允许通过 `host.subagent_with_call` 正常派发与管控其子 Agent！

#### 3.2.2 根会话树（Root Session Tree）溯源与整树并发 / 级联关闭
在 `src/db/chat/subagent.rs` 中新增树查询辅助方法（递归 CTE 沿 `agent_subagent.child_session_id -> parent_session_id` 向上找根、向下找全部后代）：

```rust
impl Database {
    /// 向上追溯任意会话所属的根会话 ID（主会话返回自身）及其所处深度（根为 0，子为 1，孙为 2）与父路径。
    pub fn resolve_session_tree_root(
        &self,
        workspace: &Path,
        session_id: &str,
    ) -> Result<(String, usize, String)> {
        let mut current = session_id.to_string();
        let mut depth = 0usize;
        let mut current_path = crate::chat::agent::ROOT_AGENT_PATH.to_string();
        // 最多向上追溯 8 层防环
        for _ in 0..8 {
            let row: Option<(String, Option<String>, Option<String>)> = self
                .conn
                .query_row(
                    r#"SELECT parent_session_id, agent_path, nickname
                       FROM agent_subagent
                       WHERE child_session_id = ?1"#,
                    params![current],
                    |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
                )
                .optional()?;
            let Some((parent_id, stored_path, nickname)) = row else {
                break;
            };
            if depth == 0 {
                let (_, decoded_path) =
                    crate::chat::agent::decode_stored_role_and_path(stored_path.as_deref());
                current_path = decoded_path.unwrap_or_else(|| {
                    let seg = nickname.as_deref().unwrap_or(&current);
                    format!("/root/{}", crate::chat::agent::slugify_segment(seg))
                });
            }
            current = parent_id;
            depth = depth.saturating_add(1);
        }
        Ok((current, depth, current_path))
    }

    /// 列出同一根会话树下的全部子孙 `SubagentRecord`（按创建时间升序）。
    pub fn list_subagent_tree(
        &self,
        workspace: &Path,
        root_session: &str,
    ) -> Result<Vec<SubagentRecord>> {
        let workspace_external = self
            .workspace_external_id(workspace)?
            .ok_or(DbError::WorkspaceMissing)?;
        let mut stmt = self.conn.prepare(
            r#"WITH RECURSIVE tree(
                   external_id, child_session_id, parent_session_id, spawn_message_id,
                   role, status, title, created_at_ms, completed_at_ms, report_json,
                   report_delivered, nickname, agent_path, model_provider, model_id, variant
               ) AS (
                   SELECT a.external_id, a.child_session_id, a.parent_session_id, a.spawn_message_id,
                          a.role, a.status, a.title, a.created_at_ms, a.completed_at_ms, a.report_json,
                          a.report_delivered, a.nickname, a.agent_path, a.model_provider, a.model_id, a.variant
                   FROM agent_subagent a
                   JOIN agent_session p ON p.external_id = a.parent_session_id
                   WHERE a.parent_session_id = ?1 AND p.workspace_id = ?2
                   UNION ALL
                   SELECT c.external_id, c.child_session_id, c.parent_session_id, c.spawn_message_id,
                          c.role, c.status, c.title, c.created_at_ms, c.completed_at_ms, c.report_json,
                          c.report_delivered, c.nickname, c.agent_path, c.model_provider, c.model_id, c.variant
                   FROM agent_subagent c
                   JOIN tree t ON c.parent_session_id = t.child_session_id
               )
               SELECT * FROM tree ORDER BY created_at_ms ASC, external_id ASC"#,
        )?;
        let rows = stmt.query_map(params![root_session, workspace_external], record::read_subagent_row)?;
        let mut out = Vec::new();
        for item in rows {
            let (rec, _) = item?;
            out.push(rec);
        }
        Ok(out)
    }
}
```

- **整树并发限制**：在 `spawn` 时，统计同一 `root_session_id` 下所有 `running` 的子孙 Agent 数量 `running_count_in_tree`，若 `>= max_concurrent`（默认 `4`，范围 `1..=12`），返回 `LIMIT_REACHED_CODE`。
- **级联关闭（`close_agent`）**：当对某个子 Agent（如 `/root/turing`）调用 `close_agent` 时，先从 `list_subagent_tree` 中找出其自身及所有 `agent_path` 以 `"/root/turing/"` 为前缀（或 `parent_session_id` 链归属它）的后代，逐个发送 `cancel` 并置 `cancelled`，防止孤儿孙 Agent 继续在后台跑。

---

### 3.3 模块 A：分叉历史清洗与按轮次分叉 (`fork_turns` + History Sanitization)

#### 3.3.1 分叉规格 `ForkSpec`
在 `src/chat/agent/mod.rs` 中定义：

```rust
/// 子 agent 历史分叉策略。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ForkSpec {
    /// 不继承父会话历史（默认）。
    None,
    /// 继承父会话当前流式轮次之前的全部已完成轮次（`fork_context = true` 且未指定 `fork_turns`）。
    AllCompleted,
    /// 仅继承父会话当前流式轮次之前的最近 `N` 个已完成轮次（`1..=64`）。
    RecentTurns(u32),
}

impl ForkSpec {
    pub const fn is_enabled(self) -> bool {
        !matches!(self, Self::None)
    }

    pub fn from_params(fork_context: bool, fork_turns: Option<u32>) -> Self {
        match fork_turns {
            Some(0) => Self::None,
            Some(n) => Self::RecentTurns(n.min(MAX_FORK_TURNS)),
            None if fork_context => Self::AllCompleted,
            None => Self::None,
        }
    }
}
```

#### 3.3.2 轮次窗口计算与三项强制清洗 (`src/db/chat/fork.rs`)
在 `src/db/chat/fork.rs` 中扩展 `copy_completed_history_to_child`：

```rust
/// 计算子会话分叉的 `[start_sequence, boundary_sequence)` 半开区间：
/// - `ForkSpec::AllCompleted`：`start_sequence = 0`；
/// - `ForkSpec::RecentTurns(n)`：取 `< boundary` 的最近 `n` 个不同 `turn_index` 的最小 `sequence`。
pub(super) fn completed_fork_window(
    conn: &Connection,
    session_id: &str,
    spec: crate::chat::agent::ForkSpec,
) -> Result<(i64, i64)> {
    let boundary = completed_boundary_sequence(conn, session_id)?;
    let start_seq = match spec {
        crate::chat::agent::ForkSpec::None => return Ok((boundary, boundary)),
        crate::chat::agent::ForkSpec::AllCompleted => 0,
        crate::chat::agent::ForkSpec::RecentTurns(turns) => {
            let mut stmt = conn.prepare(
                r#"SELECT DISTINCT turn_index
                   FROM agent_message
                   WHERE session_id = ?1 AND sequence < ?2 AND status <> 'streaming'
                   ORDER BY turn_index DESC
                   LIMIT ?3"#,
            )?;
            let turn_indices: Vec<i64> = stmt
                .query_map(params![session_id, boundary, i64::from(turns)], |r| r.get(0))?
                .collect::<std::result::Result<Vec<_>, _>>()?;
            if let Some(&min_turn) = turn_indices.last() {
                conn.query_row(
                    r#"SELECT COALESCE(MIN(sequence), 0)
                       FROM agent_message
                       WHERE session_id = ?1 AND turn_index >= ?2 AND sequence < ?3"#,
                    params![session_id, min_turn, boundary],
                    |r| r.get(0),
                )?
            } else {
                boundary
            }
        }
    };
    Ok((start_seq, boundary))
}
```

并在 `copy_completed_history_to_child` 中执行三项清洗：
1. **清零克隆消息的 token 字段**：插入子会话 `agent_message` 时，`input_tokens, output_tokens, cache_read_tokens, cache_write_tokens` 全部绑定 `None::<i64>`（不把父会话的 token 账单带进子会话）；
2. **剥离父级 `reasoning` 思考块**：`copy_parts_sanitized` 的 SQL 增加过滤条件 `AND p.kind NOT IN ('reasoning', 'thinking')`，只复制 `text` 内容块；
3. **剥离父级子 Agent 汇报标记**：`subagent_report_json` 恒置 `None`，`kind == "subagent_report"` 转为 `"chat"`。

---

### 3.4 模块 B：对齐 `codex-main` 的 Token 预算护栏与半程汇报 (`src/chat/agent/budget.rs`)

#### 3.4.1 `SubagentTokenBudget` 与阈值提醒 (`src/chat/agent/budget.rs`)
1:1 对齐 `codex-main` 的 `RolloutBudget`（`codex-rs/core/src/rollout_budget.rs` & `context/rollout_budget.rs`）：

```rust
//! 子 agent Token 预算记账与 `<rollout_budget>` 阈值提醒（对齐 codex-main RolloutBudget）。

use crate::chat::events::Usage;

/// 默认提醒阈值比例：剩余 25% 与剩余 10% 时各注入一次提醒。
const REMINDER_RATIOS: [f64; 2] = [0.25, 0.10];

#[derive(Debug, Clone)]
pub struct SubagentTokenBudget {
    limit_tokens: i64,
    used_tokens: i64,
    /// 已触发过的提醒档位索引（0 = 未触发，1 = 已触发 25%，2 = 已触发 10%）。
    delivered_stage: usize,
}

impl SubagentTokenBudget {
    pub fn new(limit_tokens: i64) -> Option<Self> {
        (limit_tokens > 0).then_some(Self {
            limit_tokens,
            used_tokens: 0,
            delivered_stage: 0,
        })
    }

    /// 累加单步采样消耗的 token（input + output），返回是否已耗尽全部预算。
    pub fn record_step_usage(&mut self, step: Usage) -> bool {
        let step_total = step
            .input
            .unwrap_or(0)
            .max(0)
            .saturating_add(step.output.unwrap_or(0).max(0));
        self.used_tokens = self.used_tokens.saturating_add(step_total);
        self.used_tokens >= self.limit_tokens
    }

    pub const fn remaining_tokens(&self) -> i64 {
        (self.limit_tokens - self.used_tokens).max(0)
    }

    /// 检查是否跨过新的剩余阈值（25% / 10%）；若是则返回待注入的 `<rollout_budget>` 英文提醒文本。
    pub fn take_pending_reminder(&mut self) -> Option<String> {
        let remaining = self.remaining_tokens();
        if remaining == 0 {
            return None;
        }
        let mut crossed_stage = 0usize;
        for (idx, ratio) in REMINDER_RATIOS.iter().enumerate() {
            let threshold = ((self.limit_tokens as f64) * ratio).floor() as i64;
            if remaining <= threshold {
                crossed_stage = idx + 1;
            }
        }
        if crossed_stage > self.delivered_stage {
            self.delivered_stage = crossed_stage;
            Some(format!(
                "<rollout_budget>\nYou have {} tokens left in your token budget (limit: {}). Wrap up active investigation and produce your final summary soon.\n</rollout_budget>",
                remaining, self.limit_tokens
            ))
        } else {
            None
        }
    }
}
```

- **在 `run_agent_turn` (`src/chat/agent_loop/mod.rs`) 中的接入点**：
  1. 每步采样 `stream_one` 返回并更新 `billing` 后，若 `context.token_budget` 存在：
     - 调用 `budget.record_step_usage(latest)`；
     - 若返回 `true`（预算已满）且本步模型仍要求继续调用工具（未主动 `Final` 收尾），在完成或取消当前工具批次后返回 `TurnSummary::budget_exceeded(usage).with_billing(billing)`；
     - 若未耗尽且 `budget.take_pending_reminder()` 返回 `Some(reminder)`，在下一轮采样前调用 `context.push_user_context(reminder)` 注入提醒。

#### 3.4.2 半程汇报（Partial Report）生成规则 (`src/event/chat/subagent/settle.rs` & `exec/subagent/settle.rs`)
区分 **`close_agent`（销毁关闭，不生成报告）** 与 **`interrupt_agent` / `budget_exceeded` / `failed`（保留半程汇报）**：
- 在 `ChatStreams`（及 `ExecSubagentPool`）中维护 `closed_subagents: HashSet<String>`：仅当 `close_agent` 触发取消时将 `agent_id` 加入 `closed_subagents`。
- 在 `finish_subagent_with_billing` 结算时：
  - 若 `closed_subagents.remove(&handle.agent_id)` 为 `true`：不生成报告（维持 `close_agent` 契约）；
  - 其余所有情况（`Complete`、`Failed`、`interrupt_agent` 触发的 `Cancelled`、以及 `budget_exceeded`）：均生成 `SubagentReportView`！
  - 若子 Agent 被打断时尚未输出总结文本（`report_body(&handle)` 为空），则自动合成为：
    ```rust
    fn synthesize_partial_report_body(
        last_text: &str,
        completed_tools: usize,
        changed_paths: &[String],
        reason_label: &str,
    ) -> String {
        if !last_text.trim().is_empty() {
            return format!("[Partial report ({reason_label})]\n{}", last_text.trim());
        }
        let files_note = if changed_paths.is_empty() {
            "no files modified".to_string()
        } else {
            format!("modified {} file(s): {}", changed_paths.len(), changed_paths.join(", "))
        };
        format!(
            "[Partial report ({reason_label})] Completed {completed_tools} tool step(s); {files_note}."
        )
    }
    ```

---

### 3.5 模块 C：双模消息投递 (`trigger_turn`) 与报告变更文件明细 (`changed_paths`)

#### 3.5.1 `send_input` 的 `trigger_turn: bool` 参数
- `SubagentOp::SendInput` 扩展为：
  ```rust
  SendInput {
      target: String,
      items: Vec<SubagentInputItem>,
      interrupt: bool,
      /// 是否在子 agent 空闲时立即唤醒启动新轮次（默认 `true`；`false` 仅入队 `queued`）。
      trigger_turn: bool,
  }
  ```
- **参数校验 (`src/chat/agent/parse.rs`)**：
  - `trigger_turn` 可选布尔值，默认 `true`；
  - 若 `interrupt == true && !trigger_turn`，直接返回 `ToolFailure::invalid("cannot set `interrupt: true` when `trigger_turn` is false")`。
- **运行时行为 (`src/event/chat/subagent/mod.rs` & `exec/subagent/mod.rs`)**：
  - 当 `trigger_turn == false` 时，调用 `database.enqueue_subagent_input_with_items(...)` 将消息持久化为 `queued`，更新 UI 的 `queued_count`，并直接返回 `SubagentOutcome::InputAccepted { status: projected_status, queued_count, interrupt: false }`，**不调用 `schedule_next_queued_input`**，即使子 Agent 此时处于 `idle` 或 `completed` 也不唤醒；
  - 当后续对该子 Agent 调用 `resume_agent` 或 `send_input(trigger_turn=true)` 时，之前积攒在队列中的 `queued` 输入按 FIFO 顺序被唤醒消费。

#### 3.5.2 报告变更文件明细 `changed_paths: Vec<String>` (`src/db/chat/subagent.rs`, `src/chat/agent/report.rs`)
在 `src/db/chat/subagent.rs` 中新增 `list_changed_paths`（ alongside 现有的 `count_changed_paths`）：

```rust
/// 最多在单份报告中附带的变更文件路径条数。
pub const MAX_REPORT_CHANGED_PATHS: usize = 32;

impl Database {
    /// 查询该子轮（`child_user_message_id`）实际发生变更的去重文件路径列表（按首次触碰顺序，相对 workspace 収短）。
    pub fn list_changed_paths(
        &self,
        workspace: &Path,
        child_session: &str,
        child_user_message_id: &str,
    ) -> Result<(usize, Vec<String>)> {
        let total = self.count_changed_paths(workspace, child_session, child_user_message_id)?;
        if total == 0 {
            return Ok((0, Vec::new()));
        }
        let mut stmt = self.conn.prepare(
            r#"SELECT DISTINCT f.file_path
               FROM agent_file_mutation f
               JOIN agent_turn_file_scope s ON s.id = f.scope_id
               WHERE s.session_id = ?1 AND s.user_message_id = ?2
               ORDER BY f.id ASC
               LIMIT ?3"#,
        )?;
        let rows = stmt.query_map(
            params![child_session, child_user_message_id, MAX_REPORT_CHANGED_PATHS as i64],
            |r| r.get::<_, String>(0),
        )?;
        let mut paths = Vec::new();
        for row in rows {
            let raw = row?;
            let rel = crate::app::state::relativize_project_path(&raw, workspace);
            paths.push(rel);
        }
        Ok((total, paths))
    }
}
```

- `SubagentReportView` 新增 `pub changed_paths: Vec<String>`（JSON 序列化字段 `"changed_paths"`，反序列化缺失时默认为空 `Vec::new()`，100% 兼容旧报告 JSON）。
- `format_report_envelope` 在 `<subagent_report ...>` 中：若 `changed_paths` 非空，在正文前附一行 `Changed files: a.rs, b.rs`（超 32 条追加 `(+N more)`），方便父 Agent 直接读取而不必再跑 `git status`。

---

### 3.6 模块 E：TUI 呈现与交互补齐 (`src/ui/main_content/lx/`, `src/app/`)

#### 3.6.1 `wait_agent(all)` 实时展示所有被等待目标 (`src/app/update/chat/subagent.rs` & `src/ui/main_content/lx/history/tool/subagent.rs`)
1. **修复 `refresh_subagent_cards` (`src/app/update/chat/subagent.rs:188`)**：
   移除原本对 `if view.all || targets.is_empty() { continue; }` 的跳过逻辑！当 `view.all` 为真且 `wait_agent` 工具尚未结算（或刚启动）时，自动从 `pane.lx.agents.runs` 中收集当前会话树下所有子 Agent 的 `AgentStatusView` 填入 `view.agents`；
2. **修复 `history/tool/subagent.rs:286` 的 `wait_agent` 树体构建**：
   ```rust
   "wait_agent" => {
       let display_ids: Vec<String> = if !view.all && !view.targets.is_empty() {
           view.targets.clone()
       } else {
           view.agents.iter().map(|a| a.agent_id.clone()).collect()
       };
       if display_ids.is_empty() {
           let color = all_color(view, call.status);
           built.nodes.push(Node::new(vec![
               Segment::new(format!("{} ", text::agent::STEP_MARK_FILLED), color.style()),
               Segment::new(text::agent::WAIT_ALL.to_string(), style::text()),
           ]));
       } else {
           for target in &display_ids {
               let index = built.nodes.len();
               built.nodes.push(target_node(view, target));
               built.links.push((index, target.clone()));
           }
       }
       // 展开态照常渲染每个已有报告的目标的 report_node
   }
   ```

#### 3.6.2 主历史 `spawn_agent` 卡片的实时动作摘要（Live Activity）
1. 在 `SubagentRun` (`src/app/state/lx/agent.rs`) 上提供纯计算方法 `live_activity(&self) -> Option<String>`：
   - 仅当 `self.is_running()` 为 `true` 时提取；
   - 逆序查看 `self.chat.messages` 中最新一条 `assistant` 消息的 `steps`：
     - 若有处于 `Pending` / `Running` 的工具调用：
       - `exec_command` / `write_stdin` → 提取 `$ <command 首行>`；
       - `read` / `edit` / `write` → 提取 `<verb> <path>`（如 `edit src/auth.rs`）；
       - `search` / `web_search` → 提取 `search <pattern>`；
       - `spawn_agent` / `wait_agent` → 提取 `spawn <title>` / `waiting for subagents`；
     - 若最后一步是未闭合的 `PartKind::Reasoning` → 返回 `"Thinking..."`；
     - 若正在输出正文 → 返回最新一行非空文本预览。
2. `SubagentDetailView` 新增 `pub live_activity: Option<String>`（仅内存展示态，由 `refresh_subagent_cards` 在每次 `on_subagent_delta` / `on_subagent_tool_*` 时同步刷新，子 Agent 结束时置 `None`）。
3. 在 `src/ui/main_content/lx/history/tool/subagent.rs` 的 `"spawn_agent"` 分支中：
   ```rust
   if let Some(activity) = view.live_activity.as_deref().filter(|s| !s.is_empty()) {
       built.nodes.push(Node::text(
           single_line(activity, TEXT_SAMPLE_CHARS),
           style::muted(),
       ));
   }
   ```

#### 3.6.3 `Agents` 面板多级树缩进、Token 用量与双停止键位 (`src/ui/main_content/lx/task/agents.rs`, `src/app/update/pane/task_page.rs`)
1. **父子树形缩进**：
   - `SubagentRun` 新增 `pub agent_path: String`（如 `"/root/turing"`、 `"/root/turing/curie"`）；
   - 在 `task/agents.rs::draw_list` 中，根据 `run.depth()`（`path_depth(&run.agent_path)`）：
     - `depth <= 1`（直接子 Agent）：无树连接符前缀，保持原列对齐；
     - `depth >= 2`（孙 Agent）：在步骤标记前渲染 `(depth - 1)` 级树形缩进（同父末项用 `└─ `，非末项用 `├─ `，样式 `style::chat_notice_connector()`），直观表达父子从属关系。
2. **紧凑 Token 用量段**：
   - `SubagentRun::total_tokens(&self) -> Option<i64>` 计算 `input_tokens + output_tokens`（运行中从 `run.chat.step_usage` 实时读取，终态从 `run.input_tokens` / `run.output_tokens` 读取）；
   - 在 `draw_list` 中位于耗时 `age` 与 `title` 之间渲染紧凑格式（如 `14.2k tok`，复用 `text::format_token_count`），窄窗格下优先于 `title` 让位隐藏。
3. **停止键位修正（`^d` 停当前项及后代 vs `^a` 停全部）**：
   - `LxInput` 新增 `StopAllSubagents`（绑定 `Ctrl+A`，仅在 `TaskPage` 打开且处于 `Agents` 页签时生效，不影响输入框的 `Ctrl+A` 行首快捷键）；
   - `^d` (`LxInput::StopProcess`)：停止当前高亮的子 Agent（若其有存活后代一并级联停止）；
   - `^a` (`LxInput::StopAllSubagents`)：停止当前根会话树下**全部**正在运行的子 Agent；
   - 底边框提示文案更新为：`[tabs ←→] [select ↑↓] [page pgup/pgdn] [stop ^d] [stop all ^a] [close esc]`（遵循 `ui::text::key` 单宽快捷键符号规范）。

#### 3.6.4 新增 `/subagents`（别名 `/agents`）斜杠命令 (`src/app/state/lx/slash.rs`, `src/app/update/chat/session.rs`)
- `SlashCommand` 枚举新增 `Subagents` 变体：
  - `name()` → `"/subagents"`
  - `aliases()` → `&["/agents"]`
  - `description()` → `"Open background subagents panel"`
- 在 `execute_slash_command` 中命中 `SlashCommand::Subagents` 时，关闭 `/` 面板、打开 `TaskPage` 并将 `page.tab` 设为 `TaskTab::Agents`。

---

### 3.7 Web 设置页与配置模型扩展 (`src/config/subagents.rs`, `src/settings/service/subagents.rs`, `assets/`)

在 `SubagentsSection` (`src/config/subagents.rs`) 中新增两个字段（带 `#[serde(default)]` 保证旧 `subagents.json` 无缝升级）：

```rust
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SubagentsSection {
    /// 整棵根会话树的最大并发运行子 agent 数（1..=12，默认 4）。
    #[serde(default = "default_max_concurrent")]
    pub max_concurrent: usize,
    /// 最大递归派发深度（1..=3，默认 2：主 agent -> 子 agent -> 孙 agent）。
    #[serde(default = "default_max_depth")]
    pub max_depth: usize,
    /// 可选单轮默认 Token 预算上限（None 表示不限；开启时正整数，如 100000）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_max_tokens: Option<u64>,
    #[serde(default)]
    pub builtins: BuiltinRoleOverrides,
    #[serde(default)]
    pub roles: Vec<CustomRoleConfig>,
}
```

- `assets/settings.html` 与 `assets/settings_subagents.js` 在全局设置卡片中增加 `Max Depth (1–3)` 与 `Default Token Budget (optional)` 输入控件，全部文案使用英文。

---

## 4. 模块划分与单文件行数预算

严格遵守项目规范（**生产代码单文件上限 1000 行，拆分子文件控制在 500 行左右；测试镜像下沉且单文件 ≤ 1000 行**）：

| 文件路径 | 变更类型 | 预计行数 | 职责说明 |
| :--- | :--- | :--- | :--- |
| `src/chat/agent/status.rs` | 新增 | ~130 行 | `AgentStatus` 强类型枚举、序列化、终态判定与 `projected_agent_status` 统一投影 |
| `src/chat/agent/path.rs` | 新增 | ~220 行 | `AgentPath` 规范化、深度计算、`allocate_child_path`、三合一寻址与 DB 零迁移编解码 |
| `src/chat/agent/budget.rs` | 新增 | ~140 行 | `SubagentTokenBudget` 记账、25%/10% 阈值判定与 `<rollout_budget>` 英文提醒生成 |
| `src/chat/agent/mod.rs` | 修改 | ~820 行 | 导出 `status`/`path`/`budget`、新增 `ToolSurface::ExecutorCoordinator`、`ForkSpec`、扩展 `SubagentOp` 与 `SubagentOutcome` |
| `src/chat/agent/parse.rs` | 修改 | ~580 行 | 解析 `spawn_agent` 的 `task_name`/`fork_turns`/`max_tokens`、`send_input` 的 `trigger_turn`、`list_agents` 的 `path_prefix` |
| `src/chat/agent/report.rs` | 修改 | ~240 行 | `SubagentReportView` 新增 `changed_paths` 与半程汇报合成器 `synthesize_partial_report_body` |
| `src/chat/agent_loop/mod.rs` | 修改 | ~890 行 | 接入 `SubagentTokenBudget` 提醒与耗尽熔断、按 `subagent_depth < max_depth` 放行协调型 `Executor` 控制工具 |
| `src/config/subagents.rs` | 修改 | ~440 行 | 新增 `max_depth`（`1..=3`）、`default_max_tokens`、扩展 `max_concurrent`（`1..=12`）及校验 |
| `src/db/chat/fork.rs` | 修改 | ~830 行 | `completed_fork_window` 按轮次截取 + 三项历史清洗（剥离 `reasoning`、清零克隆 token、清汇报标记） |
| `src/db/chat/subagent.rs` | 修改 | ~680 行 | `resolve_session_tree_root`、`list_subagent_tree`、`list_changed_paths`、存储 `"{base_role}:{agent_path}"` |
| `src/event/chat/subagent/` | 修改 | 各 <750 行 | 整树并发计数、多级寻址、级联关闭、`trigger_turn=false` 仅入队、半程汇报生成 |
| `src/chat/agent/exec/subagent/` | 修改 | 各 <900 行 | 无头模式同步支持 `ForkSpec`、预算护栏、多级树寻址、`trigger_turn` 与半程汇报 |
| `src/app/state/lx/agent.rs` | 修改 | ~320 行 | `SubagentRun` 新增 `agent_path`、`live_activity()`、`total_tokens()`、`depth()` |
| `src/app/update/chat/subagent.rs` | 修改 | ~760 行 | `refresh_subagent_cards` 支持 `wait_agent(all)` 目标回填与 `spawn_agent` 的 `live_activity` 刷新 |
| `src/ui/main_content/lx/history/tool/subagent.rs` | 修改 | ~810 行 | `wait_agent(all)` 逐目标树渲染、`spawn_agent` 实时活动行、报告变更文件展示 |
| `src/ui/main_content/lx/task/agents.rs` | 修改 | ~280 行 | 多级子 Agent 树形缩进（`├─ ` / `└─ `）、紧凑 Token 用量列渲染 |
| `src/app/state/lx/slash.rs` & `task_page.rs` | 修改 | 各 <450 行 | 新增 `/subagents`（别名 `/agents`）命令与 `^a` 停止全部运行中子 Agent |
| `assets/settings.html` & `assets/settings_subagents.js` | 修改 | 各 <500 行 | 设置页新增 `Max Depth` 与 `Default Token Budget` 配置项 |

---

## 5. 风险、边界条件与验证策略

1. **防无限套娃与并发爆炸**：
   - 数据库 `resolve_session_tree_root` 硬限最多向上追溯 8 跳防环；
   - `max_depth` 硬钳制在 `1..=3`，且到达 `max_depth` 的子 Agent 请求体中根本不声明 8 个控制工具，执行层二次拦截返回 `DEPTH_LIMIT_REACHED_CODE`；
   - `max_concurrent` 对整棵 `root_session` 树生效，子 Agent 派发孙 Agent 与主 Agent 派发子 Agent 共享同一个并发池。
2. **父级关闭时的孤儿进程/孙 Agent 清理**：
   - 当主 Agent 调用 `close_agent("/root/turing")` 时，先递归收集 `/root/turing` 下的所有后代（如 `/root/turing/curie`），先取消并关闭后代进程管理器与流，再关闭父节点。
3. **旧数据库与旧会话 100% 兼容**：
   - `decode_stored_role_and_path` 同时识别旧格式 `"explorer"` 与新格式 `"explorer:/root/turing"`；
   - `SubagentReportView::from_value` 对缺失的 `changed_paths` 默认赋空数组。
4. **精准验证命令**：
   - `cargo test --lib chat::agent`
   - `cargo test --lib db::chat::subagent`
   - `cargo test --lib event::chat`
   - `cargo test --lib ui::main_content::lx`
   - `cargo clippy --all-targets -- -D warnings`
   - `cargo fmt --check`
