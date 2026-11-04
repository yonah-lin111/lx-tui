# lx Subagent 进阶能力与设置页配置 — 详细设计 (`design.md`)

- **特性目录**：`docs/feature/lx-subagent-advanced/`
- **关联任务**：[`task.md`](task.md)
- **参考仓库**：`/Users/yonah/projects/agent/codex-main`（`codex-rs/core/src/agent/{role.rs,control.rs,registry.rs,child_config.rs}`、`codex-rs/agent-roles/`、`codex-rs/core/src/tools/handlers/multi_agents*`、`codex-rs/tui/src/multi_agents.rs`）
- **目标范围**：`src/chat/agent/`、`src/chat/tools/`、`src/config/`、`src/db/chat/`、`src/event/chat/subagent/`、`src/app/`、`src/ui/main_content/lx/`、`src/settings/`、`assets/`
- **当前状态**：**已实现并通过全部测试与 Clippy 校验（待用户确认合并）**

---

## 0. 代码库事实核验与前置基线

通过 `codegraph` 与 `codebase-memory-mcp` 检索核实，当前代码库真实状态如下（设计严格建立在以下事实之上，不凭空假设）：

| 核验维度 | 代码库当前真实状态 | 设计影响与应对策略 |
| :--- | :--- | :--- |
| **数据库 Schema 版本** | `src/db/mod.rs:27` 当前 `SCHEMA_VERSION = 22`。`src/db/migration/schema.rs:192-209` 中 `agent_subagent` 表**已预留** `nickname TEXT`、`role TEXT`、`agent_path TEXT`、`model_provider TEXT`、`model_id TEXT`、`variant TEXT` 列（均无角色名 `CHECK` 约束）；`agent_session` 表（`schema.rs:122-124`）也**已预留** `agent_nickname TEXT`、`agent_role TEXT`、`agent_path TEXT` 列。 | **零整表重建、零破坏性迁移**！可直接复用 `agent_subagent.{nickname, role, agent_path, model_provider, model_id, variant}` 与 `agent_session.{agent_nickname, agent_role, agent_path}`。其中 `agent_path` 用于持久化子 Agent 创建时冻结的基础权限档位（`"explorer"` 或 `"executor"`），確保自定义角色删除后仍可安全恢复。 |
| **角色枚举硬编码与 DB 过滤** | `src/chat/agent/mod.rs:37` 定义了 `pub enum AgentRole { Explorer, Executor }`（`Copy`）；`src/db/chat/subagent.rs:425,457` 在 `owned_subagent` 与 `list_subagents` 中写死了 `if role.as_deref().and_then(AgentRole::parse).is_some()`，非内置角色名会被直接丢弃。 | 将**权限档位**（`BaseRole::Explorer \| Executor`）与**角色身份**（`AgentRole`，支持内置角色与任意合法 `[a-z0-9_]+` 自定义角色名）解耦；移除 `db/chat/subagent.rs` 中丢弃非内置角色行的过滤逻辑。 |
| **上下文分叉 (`fork`)** | `src/db/chat/fork.rs:118` 已实现 `fork_chat_session`，具备克隆消息、part、tool_call、attachment、compact_checkpoint、model_switch 及重写图片目录的完整能力；但内部调用了 `ensure_no_streaming`（拒绝存在 `status = 'streaming'` 消息的源会话）。 | 主 Agent 在自身流式轮次中调用 `spawn_agent(fork_context=true)` 时，父会话当前轮次必为 `streaming`。需复用 `fork.rs` 底层复制辅助函数，以**父会话当前流式轮次的用户消息 `sequence` 为上界（`m.sequence < boundary`）**，仅克隆此前已完成的完整历史轮次，再追加子 Agent 的首条任务消息。 |
| **子 Agent 工具面** | `src/chat/agent/mod.rs:93` 当前 `MAIN_TOOLS` 为 17 项（10 基础工具 + 7 控制工具），`EXEC_MAIN_TOOLS` 为 16 项。 | 新增 `interrupt_agent` 后，`MAIN_TOOLS` 扩为 **18 项**，`EXEC_MAIN_TOOLS` 扩为 **17 项**；子 Agent（`Explorer` 3 项、`Executor` 7 项）依然不包含任何控制工具，禁止递归派发。 |
| **设置页架构** | `src/settings/service.rs` + `assets/settings.{html,css,js}` + `assets/settings_{models,mcp,skills}.js`，无构建步骤，经 `include_str!` 编译进二进制（见 `src/settings/service/page.rs`）。配置文件位于 `<data>/config/*.json`（0600 原子写）。 | 新增 `<data>/config/subagents.json`（`src/config/subagents.rs`）、后端路由 `/api/subagents*`（`src/settings/service/subagents.rs`）与前端静态模块 `/settings_subagents.js`（`assets/settings_subagents.js`），在 `assets/settings.html` 左侧导航栏增加 `Subagents` 页签。 |

---

## 1. 目标与非目标

### 1.1 目标（Goals）
1. **上下文分叉 (`fork_context`)**：`spawn_agent` 新增可选 `fork_context: bool`（默认 `false`）。为 `true` 时，在单事务内将父会话当前流式轮次之前的全部已完成历史（消息、步骤、工具调用、已归档图片、压缩检查点、模型切换记录）克隆进新建子会话，随后追加本次子任务作为新轮次起点。
2. **模型与思考等级三级覆写 (`model` / `reasoning_effort`)**：
   - 支持在 `spawn_agent` 传入可选 `model`（支持 `"provider/model"` 或同 provider 下 `"model"` 简写）与 `reasoning_effort`（对应模型 `variants` key）。
   - 支持在设置页为每个角色配置默认模型与思考等级。
   - 严格执行 **“工具显式传参 > 角色设置页默认配置 > 父会话当前冻结模型”** 三级回退；显式指定或角色绑定的模型未启用/缺 Key/不支持输入模态时，直接返回明确英文工具错误拒绝启动，绝不静默降级。
3. **子 Agent 昵称系统 (`nickname`) 与双向寻址**：
   - 同一父会话内为每个新派生的子 Agent 自动分配唯一可读昵称（优先取角色自定义 `nickname_candidates`，回退内置科学家/数学家名字池 `Turing`, `Curie`, `Euler`, `Noether` 等，耗尽后自动追加 `#2`, `#3`）。
   - 持久化至 `agent_subagent.nickname` 与 `agent_session.agent_nickname`。
   - `spawn_agent`、`list_agents`、`wait_agent` 等结果中均包含 `nickname`；`send_input`、`wait_agent`、`interrupt_agent`、`close_agent`、`resume_agent` 支持传入 `agent_id`（`sub_xxx`）或 `nickname`（当前父会话内大小写不敏感匹配）双向寻址。
4. **独立中断工具 (`interrupt_agent`)**：
   - 新增主 Agent 控制工具 `interrupt_agent(target)`：中止目标子 Agent 正在执行的轮次，保留已生成的流式部分并落库为 `interrupted`；
   - **保留子 Agent 的活动待命身份（`idle`）**，主 Agent 后续可直接 `send_input` 而无需先 `resume_agent`；
   - **暂停自动消费后续排队队列**（未执行的 `queued` 输入原样保留，等下一次 `send_input` 或 `resume_agent` 触发时继续消费）；对已处于 `idle` 的子 Agent 调用幂等返回。
5. **自定义角色与内置角色覆写（`src/settings/` Subagents 页面 + `config/subagents.json`）**：
   - 内置角色 `explorer` 与 `executor` 支持在设置页覆写默认模型、思考等级、描述与系统提示词，并支持一键恢复默认（Reset）。
   - 支持新建、编辑、删除自定义角色（名称满足 `[a-z0-9_]{1,32}`），每个自定义角色必须绑定基础权限档位（`explorer` 只读或 `executor` 读写），并配置描述、系统提示词、可选默认模型/思考等级与可选昵称池。
   - 支持配置全局单会话最大并发子 Agent 数 `max_concurrent`（默认 `3`，合法范围 `1..=8`）。
   - 动态将已配置角色的描述列表注入 `spawn_agent` 的工具 Schema 描述，供主 Agent 按职责选用。
6. **TUI 界面升级 (`src/ui/main_content/lx/`)**：
   - 委派工具卡片（`history/tool/subagent.rs`）、主动汇报块（`history/report.rs`）与 `Agents` 面板列表（`task/agents.rs`）统一展示 `Nickname [role] · title`（如 `Turing [explorer] · Inspect render`），并在覆写模型时展示紧凑模型标签。

### 1.2 非目标（Non-Goals）
- **不开放子 Agent 递归派发**：无论内置角色还是自定义角色，其工具面只允许映射 `Explorer`（只读）或 `Executor`（读写），绝不包含 `spawn_agent` 等控制工具。
- **不开放逐工具自由勾选权限**：自定义角色只绑定 `explorer`（只读）或 `executor`（读写+进程）两档安全基线，避免破坏只读隔离契约。
- **不破坏历史兼容性**：不删除或重建 `agent_subagent` / `agent_session` 表；历史旧数据无 `nickname` 时平滑退化为 `[role] · title` 或 `role · title`。

---

## 2. Grill Me 已确认决策表

| # | 决策维度 | 最终确认结论 |
| :--- | :--- | :--- |
| **Q1** | **需求范围** | 5 项能力全部纳入（1. `fork_context` 上下文分叉、2. `model`/`reasoning_effort` 覆写、3. `nickname` 昵称系统、4. `interrupt_agent` 独立中断、5. 自定义角色与内置角色配置），并在 `src/settings/` 新增 Subagents 配置页。 |
| **Q2** | **设置页与角色边界** | 统一持久化至 `<data>/config/subagents.json`：<br>1. 内置角色（`explorer`/`executor`）不可删改名，可覆写默认模型、思考等级、描述、系统提示词，可一键重置；<br>2. 自定义角色绑定 `explorer`（只读）或 `executor`（读写）权限基线，含描述、提示词、默认模型/思考等级、昵称池；<br>3. 全局并发上限 `max_concurrent` 可配置（默认 `3`，范围 `1..=8`）。 |
| **Q3** | **`fork_context` 范围** | `spawn_agent(fork_context=true)` 仅克隆父会话**当前流式轮次之前所有已完成的完整历史轮次**（含消息、工具记录、归档图片、压缩检查点），不克隆当前正在流式生成的半截轮次，最后追加本次 `task`/`items` 作为子会话最新用户指令。 |
| **Q4** | **模型覆写优先级与格式** | 严格按 **“工具显式传参 (`spawn_agent`) > 角色设置页默认配置 (`subagents.json`) > 父会话当前冻结模型”** 三级回退。`model` 参数兼容 `"provider/model"` 与同供应商下 `"model"` 简写；目标模型不存在、未启用或无 API Key 时直接报错拒绝启动，不静默降级。 |
| **Q5** | **`interrupt_agent` 语义** | 仅打断目标子 Agent 当前运行轮次并保留已生成半截输出，同时**保留 `idle` 活动态**（后续可直接 `send_input`），但**暂停自动消费排队队列**（`queued` 项原样保留，待下次 `send_input` 或 `resume_agent` 再继续）；若目标本就处于 `idle`，调用幂等返回。 |
| **Q6** | **昵称分配、双向寻址与 UI** | 1. 同一父会话内按序分配唯一昵称（角色昵称池优先，回退内置科学家名字池，耗尽加 `#2`），存入 `agent_subagent.nickname`；<br>2. 控制工具 `target`/`id` 参数支持 `agent_id` 或 `nickname`（父会话内大小写不敏感）双向寻址；<br>3. TUI 卡片、汇报块与 Agents 列表展示为 `Turing [explorer] · title`。 |
| **Q7** | **自定义角色持久化与删除后兼容** | 1. `agent_subagent.role` 存角色名字符串，`agent_subagent.agent_path` 与 `agent_session.agent_path` 存创建时冻结的基础权限档位（`"explorer"` / `"executor"`）；<br>2. 自定义角色在设置页被删除后，历史卡片与 `Agents` 列表照常显示 `[role]`；调用 `resume_agent` 时按其冻结的基础权限档位与内置提示词平滑恢复。 |

---

## 3. 架构与模块详细设计

### 3.1 领域层：角色模型、昵称池与控制协议 (`src/chat/agent/`)

#### 3.1.1 基础权限档位 `BaseRole` 与动态角色标识 `AgentRole`
为同时支持内置角色零歧义分发与用户自定义角色，并避免全局大量结构体的泛型传染，在 `src/chat/agent/role.rs`（由 `src/chat/agent/mod.rs` 拆分导出）中定义：

```rust
/// 子 agent 的底层安全权限基线（仅此两档，决定工具白名单与默认内置提示词）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum BaseRole {
    /// 只读调查：`read` / `search` / `web_search`（及只读 MCP）。
    Explorer,
    /// 有边界实现：`read` / `search` / `web_search` / `edit` / `write` / `exec_command` / `write_stdin`。
    Executor,
}

impl BaseRole {
    pub const ALL: [BaseRole; 2] = [Self::Explorer, Self::Executor];

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Explorer => "explorer",
            Self::Executor => "executor",
        }
    }

    pub fn parse(text: &str) -> Option<Self> {
        match text.trim() {
            "explorer" => Some(Self::Explorer),
            "executor" => Some(Self::Executor),
            _ => None,
        }
    }

    pub const fn default_prompt(self) -> &'static str {
        match self {
            Self::Explorer => super::EXPLORER_PROMPT,
            Self::Executor => super::EXECUTOR_PROMPT,
        }
    }

    pub const fn default_description(self) -> &'static str {
        match self {
            Self::Explorer => "Read-only codebase and image investigator (`read`, `search`, `web_search`).",
            Self::Executor => "Bounded implementation worker with file and process tools (`edit`, `write`, `exec_command`).",
        }
    }

    pub const fn tool_surface(self) -> super::ToolSurface {
        match self {
            Self::Explorer => super::ToolSurface::Explorer,
            Self::Executor => super::ToolSurface::Executor,
        }
    }
}

/// 子 agent 角色标识（内置 `"explorer"` / `"executor"` 或自定义角色名如 `"reviewer"`）。
///
/// 保持 `PartialEq`/`Eq`/`Clone`，并提供 `AgentRole::EXPLORER` / `AgentRole::EXECUTOR`
/// 关联常量与 `as_str(&self) -> &str`，最大限度兼容现有调用点。
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct AgentRole {
    /// 稳定角色名（全小写 `[a-z0-9_]{1,32}`）。
    name: String,
    /// 冻结的基础权限档位（内置角色固定对应，自定义角色由配置或 DB 快照决定）。
    base: BaseRole,
}

impl AgentRole {
    pub const EXPLORER_NAME: &'static str = "explorer";
    pub const EXECUTOR_NAME: &'static str = "executor";

    pub fn explorer() -> Self {
        Self {
            name: Self::EXPLORER_NAME.to_string(),
            base: BaseRole::Explorer,
        }
    }

    pub fn executor() -> Self {
        Self {
            name: Self::EXECUTOR_NAME.to_string(),
            base: BaseRole::Executor,
        }
    }

    /// 构造自定义或从 DB 还原的角色（`base` 缺失时按角色名推断，未知则默认 `Explorer` 最安全只读档位）。
    pub fn from_name_and_base(name: impl Into<String>, base: Option<BaseRole>) -> Option<Self> {
        let name = name.into();
        if !is_valid_role_name(&name) {
            return None;
        }
        let resolved_base = match name.as_str() {
            "explorer" => BaseRole::Explorer,
            "executor" => BaseRole::Executor,
            _ => base.unwrap_or(BaseRole::Explorer),
        };
        Some(Self {
            name,
            base: resolved_base,
        })
    }

    pub fn as_str(&self) -> &str {
        &self.name
    }

    pub const fn base(&self) -> BaseRole {
        self.base
    }

    pub fn is_builtin(&self) -> bool {
        matches!(self.name.as_str(), "explorer" | "executor")
    }

    pub const fn tools(&self) -> &'static [&'static str] {
        self.base.tool_surface().names()
    }
}

/// 校验角色名：1..=32 字符，仅允许小写字母 `a-z`、数字 `0-9` 与下划线 `_`，且首字符为字母。
pub fn is_valid_role_name(name: &str) -> bool {
    let bytes = name.as_bytes();
    if bytes.is_empty() || bytes.len() > 32 || !bytes[0].is_ascii_lowercase() {
        return false;
    }
    bytes
        .iter()
        .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || *b == b'_')
}
```

#### 3.1.2 昵称池与分配算法 (`src/chat/agent/nickname.rs`)
对齐 `codex-main` (`codex-rs/core/src/agent/registry.rs`)：
- 内置候选昵称池（`DEFAULT_NICKNAME_CANDIDATES`）：
  ```rust
  pub const DEFAULT_NICKNAME_CANDIDATES: &[&str] = &[
      "Turing", "Curie", "Euler", "Noether", "Gauss", "Lovelace", "Feynman", "Shannon",
      "Hilbert", "Dirac", "Riemann", "Poincare", "Pascal", "Hopper", "vonNeumann", "Knuth",
      "Dijkstra", "Hamming", "Ramanujan", "Galois", "Archimedes", "Kepler", "Maxwell", "Faraday",
  ];
  ```
- **分配函数 `allocate_nickname(used: &[String], custom_candidates: &[String]) -> String`**：
  1. 候选列表优先使用非空的 `custom_candidates`（角色专属昵称池），否则使用 `DEFAULT_NICKNAME_CANDIDATES`。
  2. 第一轮遍历候选池，查找在 `used` 中**大小写不敏感未出现**的首个名字（如 `"Turing"`）。
  3. 若候选池一轮用尽，从序号 `round = 2` 起递增生成 `"{candidate} #{round}"`（如 `"Turing #2"`），直到找到未占用的唯一昵称。
- **展示格式化辅助函数 `format_agent_identity(nickname: Option<&str>, role: &str) -> String`**：
  - 有非空 `nickname` 时返回 `"{nickname} [{role}]"`（如 `"Turing [explorer]"`）；
  - 无 `nickname`（历史旧记录）时返回 `"{role}"`（如 `"explorer"`）。

#### 3.1.3 控制协议扩展 (`SubagentOp` 与 `SubagentOutcome`)
在 `src/chat/agent/mod.rs` 与 `src/chat/agent/parse.rs` 中扩展：

```rust
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SubagentOp {
    /// 派发一个子 agent。
    Spawn {
        /// 角色名（由解析层校验语法，事件层结合 `SubagentsConfig` 校验存在性与权限基线）。
        role: AgentRole,
        /// 面向用户的短标题。
        title: String,
        /// 任务文本摘要。
        task: String,
        /// 有序结构化输入项。
        items: Vec<SubagentInputItem>,
        /// 是否克隆父会话当前流式轮次之前的已完成历史。
        fork_context: bool,
        /// 可选模型覆写（`"provider/model"` 或 `"model"`）。
        model: Option<String>,
        /// 可选思考等级覆写（对应模型 `variants` key）。
        reasoning_effort: Option<String>,
    },
    /// 向活动或已恢复的子 agent 发送输入（`target` 支持 `agent_id` 或 `nickname`）。
    SendInput {
        target: String,
        items: Vec<SubagentInputItem>,
        interrupt: bool,
    },
    /// 中断子 agent 当前正在运行的轮次，保留其 `idle` 活动态并暂停后续队列消费。
    Interrupt {
        target: String,
    },
    /// 显式恢复终态或重启后的子 agent（`id` 支持 `agent_id` 或 `nickname`）。
    Resume {
        id: String,
    },
    /// 等待子 agent 结束（`targets` 支持 `agent_id` 或 `nickname`）。
    Wait {
        targets: Vec<String>,
        mode: WaitMode,
        timeout_ms: u64,
    },
    /// 停止一个子 agent（`target` 支持 `agent_id` 或 `nickname`）。
    Close {
        target: String,
    },
    /// 列出当前父会话下的全部子 agent。
    List,
    /// 取消一条尚未开始的排队输入。
    CancelInput {
        submission_id: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SubagentOutcome {
    Spawned {
        agent_id: String,
        nickname: Option<String>,
        role: AgentRole,
        title: String,
        model: Option<String>,
        reasoning_effort: Option<String>,
        fork_context: bool,
    },
    InputAccepted {
        submission_id: String,
        agent_id: String,
        nickname: Option<String>,
        status: String,
        queued_count: usize,
        interrupt: bool,
    },
    Interrupted {
        agent_id: String,
        nickname: Option<String>,
        previous_status: String,
        queued_count: usize,
    },
    Resumed {
        agent_id: String,
        nickname: Option<String>,
        status: String,
        queued_count: usize,
    },
    Waited {
        timed_out: bool,
        agents: Vec<AgentStatusView>,
        all: bool,
        mode: WaitMode,
    },
    Closed {
        agent_id: String,
        nickname: Option<String>,
        previous_status: String,
    },
    Listed {
        agents: Vec<AgentListItemView>,
    },
    InputCancelled {
        submission_id: String,
        agent_id: String,
        status: String,
        queued_count: usize,
    },
}
```
同时 `AgentStatusView`、`AgentListItemView` 与 `SubagentReportView` 均增加：
- `pub nickname: Option<String>`
- `pub model_label: Option<String>`（可选，用于记录子 Agent 的模型展示标签，如 `"gpt-4.1-mini (high)"`）
并在 `to_json` / `from_json` 中对缺失字段使用 `unwrap_or_default()`，保证旧数据库记录 100% 兼容反序列化。

---

### 3.2 配置层：`<data>/config/subagents.json` (`src/config/subagents.rs`)

遵循 `src/config/skills.rs` 与 `src/config/mcp.rs` 同一套约定：文件缺失返回默认配置，损坏报错，保存走 `temp -> 0600 -> rename` 原子写。

#### 3.2.1 JSON 结构示例
```json
{
  "subagents": {
    "maxConcurrent": 3,
    "builtins": {
      "explorer": {
        "description": null,
        "systemPrompt": null,
        "model": {
          "provider": "openai",
          "model": "gpt-4.1-mini",
          "variant": "low"
        },
        "nicknameCandidates": []
      },
      "executor": {
        "description": null,
        "systemPrompt": null,
        "model": null,
        "nicknameCandidates": []
      }
    },
    "roles": [
      {
        "name": "reviewer",
        "baseRole": "explorer",
        "description": "Strict code reviewer focusing on regressions, security, and edge cases.",
        "systemPrompt": "You are a senior code reviewer working inside the user's workspace...",
        "model": {
          "provider": "anthropic",
          "model": "claude-sonnet-4-5",
          "variant": "high"
        },
        "nicknameCandidates": ["Dijkstra", "Hoare", "Liskov"]
      }
    ]
  }
}
```

#### 3.2.2 Rust 数据结构与校验规则 (`src/config/subagents.rs`)
```rust
pub const DEFAULT_MAX_CONCURRENT_SUBAGENTS: usize = 3;
pub const MIN_CONCURRENT_SUBAGENTS: usize = 1;
pub const MAX_CONCURRENT_SUBAGENTS_LIMIT: usize = 8;
pub const MAX_ROLE_DESCRIPTION_CHARS: usize = 500;
pub const MAX_ROLE_PROMPT_CHARS: usize = 16_000;
pub const MAX_NICKNAME_CANDIDATES: usize = 32;
pub const MAX_NICKNAME_CHARS: usize = 24;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SubagentsConfigFile {
    #[serde(default)]
    pub subagents: SubagentsSection,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SubagentsSection {
    #[serde(default = "default_max_concurrent")]
    pub max_concurrent: usize,
    #[serde(default)]
    pub builtins: BuiltinRoleOverrides,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub roles: Vec<CustomRoleConfig>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BuiltinRoleOverrides {
    #[serde(default)]
    pub explorer: RoleOverrideConfig,
    #[serde(default)]
    pub executor: RoleOverrideConfig,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RoleOverrideConfig {
    /// 覆写角色简短描述（None 表示使用内置默认描述）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// 覆写系统提示词（None 表示使用内置 `EXPLORER_PROMPT` / `EXECUTOR_PROMPT`）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub system_prompt: Option<String>,
    /// 默认模型与思考等级（None 表示继承父会话模型）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<ModelSelection>,
    /// 自定义昵称候选列表（空数组表示使用全局默认科学家昵称池）。
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub nickname_candidates: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CustomRoleConfig {
    /// 唯一角色名（`[a-z0-9_]{1,32}`，禁止与 `"explorer"` / `"executor"` 重名）。
    pub name: String,
    /// 基础权限档位：`explorer`（只读）或 `executor`（读写）。
    pub base_role: BaseRole,
    /// 必填角色描述（注入 `spawn_agent` 工具说明）。
    pub description: String,
    /// 必填角色系统提示词。
    pub system_prompt: String,
    /// 可选默认模型与思考等级。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<ModelSelection>,
    /// 可选专属昵称池。
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub nickname_candidates: Vec<String>,
}
```
- **统一解析入口 `SubagentsSection::resolve_role(&self, name: &str) -> Option<ResolvedRoleSpec>`**：
  - 输入 `"explorer"` 或 `"executor"`：合并 `BuiltinRoleOverrides` 与内置默认值，返回 `ResolvedRoleSpec { role: AgentRole, description, system_prompt, default_model, nickname_candidates }`。
  - 输入自定义角色名：在 `self.roles` 中查找同名项并返回 `ResolvedRoleSpec`。
  - 未找到返回 `None`（`spawn_agent` 报错 `unknown subagent role '<name>'`；`resume_agent` 则走 DB 冻结的 `base_role` 兜底恢复）。

---

### 3.3 工具契约与动态 Schema (`src/chat/tools/subagent.rs` & `schema.rs`)

#### 3.3.1 新增与扩展的工具契约
1. **`spawn_agent` 扩展参数**：
   - `role`: `string`（必填，描述中动态列出 `explorer`, `executor` 及所有已配置的自定义角色与描述；Schema 类型为 `string`，移除死板的固定 2 项 `enum` 以支持自定义角色，或者由 `tools_for_surface_with_subagents` 动态生成 `enum` 列表）。
   - `title`: `string`（必填，1..=80 字符）。
   - `task` / `items`（二选一互斥）。
   - `fork_context`: `boolean`（可选，默认 `false`）— `"When true, clones all completed turns from the parent conversation into the subagent before appending this task. Defaults to false (clean context)."`
   - `model`: `string`（可选）— `"Optional model override in 'provider/model' or 'model' format. Omit to use the role's configured default model or inherit your current model."`
   - `reasoning_effort`: `string`（可选）— `"Optional reasoning effort / variant override (for example 'low', 'medium', 'high', 'max'). Omit to use the role's default or inherit."`
2. **`interrupt_agent` 新增工具**：
   - 名称：`"interrupt_agent"`
   - 描述：`"Interrupt a running subagent's current turn by `target` (`agent_id` or `nickname`), preserving its partial output while keeping the agent in `idle` state ready for follow-up `send_input`. Any remaining queued inputs are paused until you send new input or call `resume_agent`."`
   - 参数：`{"type": "object", "additionalProperties": false, "required": ["target"], "properties": {"target": {"type": "string", "minLength": 1, "description": "Target subagent `agent_id` or `nickname` in this conversation."}}}`
   - 返回：`{"tool": "interrupt_agent", "agent_id": "sub_...", "nickname": "Turing", "previous_status": "running", "queued_count": 1}`
3. **双向寻址说明更新**：
   - `send_input.target`、`wait_agent.targets`、`close_agent.target`、`resume_agent.id` 的描述更新为支持传入 `agent_id` 或 `nickname`。

---

### 3.4 数据库与持久化层 (`src/db/chat/subagent.rs` & `fork.rs`)

#### 3.4.1 复用现有列（无需新增表或破坏性迁移）
核查 `src/db/migration/schema.rs:192-209`：
- `agent_subagent` 表已有列：`external_id`, `parent_session_id`, `child_session_id`, `spawn_message_id`, `status`, `nickname`, `role`, `agent_path`, `model_provider`, `model_id`, `variant`, `title`, `report_json`, `report_delivered`, `created_at_ms`, `updated_at_ms`, `completed_at_ms`。
- 写入约定：
  - `agent_subagent.nickname` / `agent_session.agent_nickname`：存分配的唯一昵称（如 `"Turing"`）。
  - `agent_subagent.role` / `agent_session.agent_role`：存角色名字符串（如 `"explorer"`, `"reviewer"`）。
  - `agent_subagent.agent_path` / `agent_session.agent_path`：存基础权限档位 `"explorer"` 或 `"executor"`（供自定义角色被删后恢复权限与内置提示词）。
  - `agent_subagent.{model_provider, model_id, variant}`：存该子 Agent 实际生效的模型快照（以前虽有列但查询未读出，本次纳入 `SubagentRecord` 读取）。

#### 3.4.2 双向寻址解析 `resolve_owned_subagent`
在 `src/db/chat/subagent.rs` 新增统一寻址方法：
```rust
/// 按 `agent_id` 精确匹配或按 `nickname`（当前父会话内 ASCII 大小写不敏感）解析归属子 agent。
pub fn resolve_owned_subagent(
    &self,
    workspace: &Path,
    parent_session: &str,
    target_or_nickname: &str,
) -> Result<Option<SubagentRecord>> {
    let key = target_or_nickname.trim();
    if key.is_empty() {
        return Ok(None);
    }
    // 1. 优先按 external_id 精确查找
    if let Some(record) = self.owned_subagent(workspace, parent_session, key)? {
        return Ok(Some(record));
    }
    // 2. 回退按父会话内 nickname 大小写不敏感匹配
    let all = self.list_subagents(workspace, parent_session)?;
    Ok(all.into_iter().find(|rec| {
        rec.nickname
            .as_deref()
            .is_some_and(|nick| nick.eq_ignore_ascii_case(key))
    }))
}
```

#### 3.4.3 `fork_context = true` 的子会话事务克隆
在 `src/db/chat/subagent.rs`（调用 `src/db/chat/fork.rs` 导出的内部事务复制函数 `copy_completed_history_to_child`）：
1. **确定复制上界 `boundary_sequence`**：
   - 查询父会话中当前最早一条 `status = 'streaming'` 消息对应的本轮起始 `sequence`（即该 `turn_index` 下 `role = 'user'` 的 `sequence`，或最小 `streaming` 的 `sequence - 1`）；若父会话无 `streaming` 消息（如测试直接调 DB），则取 `i64::MAX`。
   - 仅选取父会话中 `sequence < boundary_sequence` 且 `status <> 'streaming'` 的已完成消息行。
2. **图片归档目录克隆**：
   - 在进入 SQLite 写事务前，若 `fork_context == true`，先调用 `fork::copy_images(parent_session, &child_session_id)` 将父会话已归档图片复制到子会话目录，获取路径重写对 `(old_prefix, new_prefix)`；再把本次 `spawn_agent` 的新输入图片归档到同一 `&child_session_id` 目录。
3. **单事务写入顺序**：
   - 插入 `agent_session`（`source = 'subagent'`）。
   - 若 `fork_context == true`：调用 `copy_completed_history_to_child(&transaction, parent_session, &child_session_id, boundary_sequence, &image_rewrite)` 克隆历史 `agent_message`、`agent_message_part`、`agent_tool_call`、`agent_message_attachment`、`agent_compact_checkpoint`、`agent_model_switch`，并返回 `(next_sequence, next_turn_index)`。
   - 若 `fork_context == false`：`(next_sequence, next_turn_index) = (0, 0)`。
   - 以 `(next_sequence, next_turn_index)` 插入本次子任务的 `user` 消息与 `next_sequence + 1` 的 `streaming` `assistant` 消息，建立文件 `begin_turn_scope`，插入 `agent_subagent` 与首条 `agent_subagent_input`。

---

### 3.5 事件层与运行时编排 (`src/event/chat/subagent/` & `src/chat/agent/exec/subagent/`)

#### 3.5.1 模型与思考等级三级解析 (`resolve_subagent_model`)
在 `src/event/chat/subagent/runner.rs` 中新增纯解析函数：
```rust
/// 三级优先级解析子 agent 冻结模型：
/// 1. `spawn_agent` 工具显式参数 (`model_override`, `effort_override`)
/// 2. 角色配置默认模型 (`role_spec.default_model`)
/// 3. 父会话当前冻结模型 (`parent_frozen`)
pub(crate) fn resolve_subagent_model(
    models: &ModelsSnapshot,
    parent_frozen: &FrozenModel,
    role_default: Option<&ModelSelection>,
    model_override: Option<&str>,
    effort_override: Option<&str>,
) -> Result<FrozenModel, String> {
    // 第 1 步：确定基础 (provider, model, base_variant)
    let mut selection = if let Some(raw_model) = model_override.map(str::trim).filter(|s| !s.is_empty()) {
        parse_model_override_str(models, &parent_frozen.provider, raw_model)?
    } else if let Some(role_sel) = role_default {
        validate_role_model_selection(models, role_sel)?
    } else {
        ModelSelection {
            provider: parent_frozen.provider.clone(),
            model: parent_frozen.model.clone(),
            variant: parent_frozen.variant.clone(),
        }
    };

    // 第 2 步：若工具显式指定了 reasoning_effort，覆盖 variant 并严格校验
    if let Some(effort) = effort_override.map(str::trim).filter(|s| !s.is_empty()) {
        let provider_cfg = models
            .settings
            .providers
            .get(&selection.provider)
            .ok_or_else(|| format!("provider '{}' is not configured", selection.provider))?;
        let model_cfg = provider_cfg
            .models
            .get(&selection.model)
            .ok_or_else(|| format!("model '{}/{}' is not found", selection.provider, selection.model))?;
        if !model_cfg.variants.contains_key(effort) {
            let supported: Vec<&str> = model_cfg.variants.keys().map(String::as_str).collect();
            return Err(format!(
                "reasoning_effort '{}' is not supported by model '{}/{}' (available: {})",
                effort,
                selection.provider,
                selection.model,
                if supported.is_empty() { "none".to_string() } else { supported.join(", ") }
            ));
        }
        selection.variant = Some(effort.to_string());
    } else if model_override.is_some() {
        // 显式换了模型但未传 effort：归一化到目标模型自带默认 variant
        selection = models.normalize_selection(selection);
    }

    // 第 3 步：若最终就是父模型且无任何覆盖，直接复用 parent_frozen；否则解析新 FrozenModel
    if selection.provider == parent_frozen.provider
        && selection.model == parent_frozen.model
        && selection.variant == parent_frozen.variant
    {
        return Ok(parent_frozen.clone());
    }
    FrozenModel::resolve(models, &selection).ok_or_else(|| {
        format!(
            "model '{}/{}' is unavailable (provider disabled or API key missing)",
            selection.provider, selection.model
        )
    })
}
```
- **字符串格式解析 `parse_model_override_str`**：
  - 若包含 `'/'`（如 `"openai/gpt-4.1-mini"` 或 `"9router/gemini/gemini-4-argon"`，按首个 `'/'` 切分 `(provider, model)`，若首个切分 provider 不存在则在已启用 providers 中精确匹配 `model_id == raw_model`）；
  - 若不含 `'/'`：优先在 `parent_provider` 下查找 `raw_model`；若不存在，在 `models.settings.enabled_providers` 中查找唯一匹配的 `(provider, raw_model)`，若多于一个匹配则报错提示使用 `"provider/model"` 完整格式。

#### 3.5.2 `interrupt_agent` 运行时状态转换
在 `src/event/chat/subagent/mod.rs` 与 `settle.rs` 中：
- 新增 `ChatStreams.paused_by_interrupt: HashSet<String>`（区别于 `send_input(interrupt=true)` 使用的 `interrupted_subagents`）。
- 当调用 `interrupt_agent(target)` 时：
  1. 通过 `resolve_owned_subagent` 解析出 `record`（不存在报 `NOT_FOUND_CODE`）。
  2. 计算当前 `queued_count`。
  3. 若 `streams.subagents` 中存在该 `agent_id` 且 `status == STATUS_RUNNING`：
     - 将 `record.agent_id` 加入 `streams.paused_by_interrupt` 与 `streams.resumed_subagents`；
     - 发送 `handle.cancel.send(true)` 触发流式取消；
     - 返回 `SubagentOutcome::Interrupted { agent_id, nickname: record.nickname, previous_status: "running", queued_count }`。
  4. 当被中断的子轮进入 `finish_subagent_with_billing(..., StopReason::Cancelled, ...)` 时：
     - 检查 `let was_paused_by_interrupt = streams.paused_by_interrupt.remove(&handle.agent_id);`
     - 若为 `true`：**保留 `streams.resumed_subagents.insert(handle.agent_id.clone())`（维持 `idle` 活动态），且不调用 `schedule_next_queued_input_with_done`（暂停自动消费后续队列）**！
  5. 若目标当前不在运行但已在 `streams.resumed_subagents`（即处于 `idle` 态）：
     - 幂等返回 `SubagentOutcome::Interrupted { agent_id, nickname: record.nickname, previous_status: "idle", queued_count }`。
  6. 若目标处于已结束/已关闭终态（既非 `running` 也非 `idle`）：
     - 返回错误 `NOT_ACTIVE_CODE`（提示先 `resume_agent`）。

---

### 3.6 设置页后端与前端 (`src/settings/service/subagents.rs` & `assets/`)

#### 3.6.1 REST API 路由 (`src/settings/service/subagents.rs`)
在 `src/settings/service.rs` 的路由分发表中挂载 `/api/subagents`：
- `GET /api/subagents`：
  - 返回当前 `SubagentsConfigFile` 内容、内置角色的默认描述与默认系统提示词（供前端展示 placeholder 与 Reset 对比）、以及当前已启用供应商的模型与 `variants` 列表。
- `PUT /api/subagents/general`：
  - 请求体：`{"maxConcurrent": 3}`，校验 `1..=8` 后保存 `config/subagents.json`，并通过 `state.sender().send(SettingsEvent::SubagentsUpdated)` 通知 TUI 主循环热更新 `state.subagents_config`。
- `PUT /api/subagents/builtins/<role>`（`<role>` 为 `explorer` 或 `executor`）：
  - 保存内置角色的覆写项（`description`, `systemPrompt`, `model`, `nicknameCandidates`），空字符串自动归一化为 `None`。
- `DELETE /api/subagents/builtins/<role>`：
  - 一键重置该内置角色的全部覆写项为默认值。
- `POST /api/subagents/roles` / `PUT /api/subagents/roles/<name>` / `DELETE /api/subagents/roles/<name>`：
  - 新建、更新、删除自定义角色；校验 `name` 合法性（`[a-z0-9_]{1,32}` 且非 `"explorer"`/`"executor"`）、`baseRole`（`"explorer"`/`"executor"`）、`description` 与 `systemPrompt` 非空且不超长。

#### 3.6.2 前端界面 (`assets/settings.html` + `assets/settings_subagents.js`)
遵循 `docs/standards/html-assets.md`（纯原生 HTML/CSS/JS、全英文 UI 文案、复用 `Commands` / `Skills` 页的左列表右编辑器双栏布局）：
- 左侧导航栏 `nav.side` 新增 `<button id="tab-subagents" onclick="showTab('subagents')">Subagents</button>`。
- 顶部 `Concurrency` 卡片：`Max concurrent subagents per session (1–8)` 数字输入框 + `Save`。
- 下方双栏网格（复用 `.commands-grid` 样式）：
  - 左栏显示 `Built-in Roles`（`explorer`、`executor` 带 `built-in` 徽标）与 `Custom Roles` 列表，底部 `+ New role` 按钮。
  - 右栏编辑器包含：
    - `Name`（内置角色只读，自定义角色可编辑）
    - `Base permission`（`Explorer (read-only)` / `Executor (read & write)`，内置角色锁定只读）
    - `Default model`（下拉选择 `Inherit parent model` 或已启用供应商的模型）
    - `Default reasoning effort`（根据所选模型的 `variants` 动态联动下拉，或 `Inherit / model default`）
    - `Nickname candidates`（逗号分隔可选昵称，如 `Turing, Curie, Euler`）
    - `Description`（单行/多行描述，展示在 `spawn_agent` 工具说明中）
    - `System prompt`（多行 `<textarea>`，内置角色未覆写时填入内置默认提示词并提供 `Reset to default` 按钮）。

---

### 3.7 TUI 渲染层升级 (`src/ui/main_content/lx/`)

所有改动严格保持 `ui/` 纯只读渲染，不执行任何 IO，所有英文常量集中定义在 `src/ui/text.rs`：

1. **委派工具卡片 (`src/ui/main_content/lx/history/tool/subagent.rs`)**：
   - 新增 `"interrupt_agent"` 分支，标题动词 `VERB_TOOL_INTERRUPT_AGENT = "Interrupted"`。
   - `agent_line(view)` 升级：
     ```rust
     /// 格式：`Turing [explorer] · title`（无 nickname 时回退 `explorer · title`；
     /// 若有 model_label 且为 spawn_agent，追加 ` · <model_label>`）。
     fn agent_line(view: &SubagentDetailView) -> String {
         let role = view.role.as_ref().map_or("", |r| r.as_str());
         let identity = format_agent_identity(view.nickname.as_deref(), role);
         let title = view
             .title
             .as_deref()
             .map(|t| single_line(t, TEXT_SAMPLE_CHARS))
             .unwrap_or_default();
         let mut parts = Vec::new();
         if !identity.is_empty() {
             parts.push(identity);
         } else if let Some(id) = view.agent_id.as_deref().filter(|s| !s.is_empty()) {
             parts.push(id.to_string());
         }
         if !title.is_empty() {
             parts.push(title);
         }
         if let Some(model) = view.model_label.as_deref().filter(|s| !s.is_empty()) {
             parts.push(model.to_string());
         }
         if view.fork_context {
             parts.push(text::agent::FORKED_CONTEXT_TAG.to_string()); // "forked"
         }
         parts.join(text::agent::SPAWN_ROLE_SEP)
     }
     ```
2. **主动汇报块 (`src/ui/main_content/lx/history/report.rs`)**：
   - `agent_line(report)` 升级为 `format_agent_identity(report.nickname.as_deref(), report.role.as_str())` + ` · ` + `report.title`。
3. **右侧面板 `Agents` 页签列表 (`src/ui/main_content/lx/task/agents.rs`)**：
   - 列表行身份列由 `run.role.as_str()` 升级为 `run.display_identity()`（有昵称时显示 `Turing [explorer]`，无昵称时显示 `explorer`），窄窗格截断优先级保持：`身份段 > 状态段 > 时长段 > 标题段`。

---

## 4. 关键异常与边界防御矩阵

| 场景 | 预期行为与状态保证 |
| :--- | :--- |
| `spawn_agent(fork_context=true)` 时父会话正处于第 1 轮（此前无已完成轮次） | `boundary_sequence = 0`，克隆 0 条历史消息，直接插入子任务首轮消息，等价于空白上下文启动，不报错。 |
| `spawn_agent(fork_context=true)` 时父会话包含带归档图片或自动压缩检查点的历史轮次 | 已归档图片文件复制到 `<images_root>/<child_session_id>/` 并重写附件路径；压缩检查点按 `sequence < boundary` 一并复制，子会话后续上下文装配直接继承压缩检查点。 |
| `spawn_agent` 传入未知角色名（如 `"hacker"`） | 事件层结合当前 `SubagentsSection` 校验失败，返回工具错误 `"unknown subagent role 'hacker'"`，不写库、不扣并发槽。 |
| `spawn_agent` 传入未启用或不存在的 `model` / 不支持的 `reasoning_effort` | `resolve_subagent_model` 返回明确英文错误，工具调用标为 `failed`，不创建子会话。 |
| 设置页删除了自定义角色 `reviewer`，随后主 Agent 对已存在的 `reviewer` 子 Agent 调用 `resume_agent` 或 `send_input` | `owned_subagent` 从 DB 读出 `role = "reviewer"` 及 `agent_path = "explorer"`（冻结的基础权限档位）；运行时按 `BaseRole::Explorer` 的只读工具白名单与默认提示词恢复运行，绝不报错或丢弃历史。 |
| 主 Agent 调用 `interrupt_agent("Turing")`（目标正在运行且队列中有 2 条 `queued` 输入） | 当前运行轮次立即取消并保存已生成内容为 `interrupted`；`Turing` 保持 `idle` 态，2 条 `queued` 输入保留在队列中但暂停自动启动；工具返回 `previous_status: "running", queued_count: 2`。 |
| 主 Agent 随后向被中断的 `"Turing"` 调用 `send_input` 或 `resume_agent` | 立即唤醒消费队列中的排队输入或新输入，无需额外解锁步骤。 |

---

## 5. 验收标准

1. **工具面与 Schema**：主 Agent 拥有 18 个工具（含新增 `interrupt_agent` 及扩展后的 `spawn_agent`），`ExecMain` 拥有 17 个工具；子 Agent 工具面严格按 `BaseRole` 隔离（`Explorer` 3 项、`Executor` 7 项），自定义角色同样受执行层双保险拦截。
2. **上下文分叉 (`fork_context`)**：`fork_context=true` 准确克隆父会话当前流式轮次之前的已完成轮次（文本、工具、图片、压缩检查点），不包含父会话当前 streaming 轮次；子会话第一轮请求上下文包含克隆的历史轮次。
3. **模型与思考等级覆写**：严格满足 `spawn_agent 参数 > 角色默认配置 > 父会话当前模型` 优先级；卡片与 DB 记录真实生效的模型与 variant；非法模型/variant 明确报错拒绝。
4. **昵称与双向寻址**：同父会话内昵称唯一且持久化；`spawn_agent` / `list_agents` 返回 `nickname`；所有控制工具支持用 `agent_id` 或 `nickname`（大小写不敏感）寻址；TUI 卡片、汇报块与 `Agents` 列表显示 `Nickname [role] · title`。
5. **独立中断 (`interrupt_agent`)**：运行中调用可中止当前轮、保留半截输出、维持 `idle` 态并暂停后续排队输入；`idle` 态调用幂等；终态未恢复调用返回 `NOT_ACTIVE_CODE`。
6. **设置页 Subagents Tab**：支持配置 `max_concurrent`（`1..=8`）、覆写与重置内置角色（`explorer`/`executor`）、增删改自定义角色，保存至 `<data>/config/subagents.json` 并实时通知 TUI 生效。
7. **代码规范门禁**：单生产文件 ≤ 1000 行，测试跟随子模块拆分，无 `unwrap()`/`expect()`，模型与 UI 可见文本全英文，`cargo fmt`、`cargo clippy --all-targets -- -D warnings` 与受影响模块 `cargo test` 全部通过。
