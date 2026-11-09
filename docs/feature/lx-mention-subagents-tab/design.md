# lx 输入框 `@` 命令新增 `Subagents` Tab — 详细设计 (`design.md`)

- **特性目录**：`docs/feature/lx-mention-subagents-tab/`
- **关联任务**：[`task.md`](task.md)
- **目标范围**：`src/app/markdown/`、`src/app/prompt/mention.rs`、`src/app/state/lx/`、`src/app/update/`、`src/ui/mention.rs`、`src/ui/main_content/lx/mention.rs`、`src/ui/text/workspace.rs`、`src/chat/prepare/`、`src/event/inventory.rs`、`src/event/chat/`、`src/db/service/chat.rs`
- **当前状态**：**已完成**

---

## 0. 代码库现状核验与文件行数红线检查

在编写本设计前，已通过 `codegraph` 与 `codebase-memory-mcp` 核实所有相关模块的真实结构与行数（严格遵守生产文件 ≤ 1000 行、测试文件 ≤ 1500 行规范）：

| 事实维度 | 文件路径与行号 | 当前行数 | 对本设计的影响与处理策略 |
| :--- | :--- | :--- | :--- |
| **`@` 面板顶栏 Tab 枚举** | `src/app/prompt/mention.rs:25-58` | 642 行 | `MentionTab` 当前含 `Files` / `RefDir` / `Skills` / `Mcp` 4 项，被 `lx` 主输入框与右侧 `Prompt` 编辑器共用。本次新增 `Subagents` 变体，并区分右侧 `Prompt` 的 4-Tab 循环（`PROMPT_ALL`）与 `lx` 主输入框的 5-Tab 循环（`ALL`）。 |
| **`lx` 提及面板纯状态机** | `src/app/state/lx/mention.rs:28-626`<br>`src/app/state/lx/mention/tests.rs:1-542` | 626 行<br>542 行 | `LxMention` 管理 `Files` / `RefDir` / `Skills` / `Mcp` 候选与高亮窗口。在 `LxMention` 中新增 `subagent_roles` 与 `subagent_runs` 缓存、`filtered_subagents()` 及确认插入逻辑，改后约 ~740 行（远低于 1000 行上限）。 |
| **`lx` 面板包装方法与同步入口** | `src/app/state/lx/panels.rs:10-260` | 308 行 | `Lx::refresh_panel` 在编辑或光标移动时触发 `@` 面板重算。在此处及子 Agent 状态更新处同步 `self.agents.runs` 中的存活实例至 `self.mention`。 |
| **结构化引用语法 `@[refer-*]`** | `src/app/markdown/refer.rs:8-41`<br>`src/app/markdown/refer_shortcut.rs:27-55`<br>`src/ui/markdown.rs:55-62` | 206 行<br>492 行<br>386 行 | `ReferKind` 当前支持 `File` / `Folder` / `Image` / `Skill` / `Mcp`。新增 `ReferKind::Subagent`（`"subagent"`），自动获得编辑器语法高亮、整块退格原子删除（`reference_before_cursor`）与正文字母快捷引用能力。 |
| **`@` 条目双行渲染组件** | `src/ui/mention.rs:15-136`<br>`src/ui/main_content/lx/mention.rs:119-292` | 136 行<br>302 行 | `ui::mention` 提供 `file_row` / `folder_row` / `skill_row` / `mcp_row`。新增 `subagent_row`，在 `main_content/lx/mention.rs` 的 `panel_data` 中挂载第 5 个 Tab `Subagents`。 |
| **子 Agent 角色定义与项目级发现** | `src/config/subagents.rs:373-570`<br>`src/config/subagents/discover.rs:171-210` | 776 行<br>220 行 | `SubagentsSection::all_roles()` 返回内置角色（`explorer` / `executor` / `monitor`）与自定义角色（含 `is_project_scoped` 标记）的 ` Vec<ResolvedRoleSpec>`；直接复用，无需改动配置 schema。 |
| **当前会话存活子 Agent 状态** | `src/app/state/lx/agent.rs:40-505`<br>`src/app/state/lx/task.rs:42-92` | 505 行<br>310 行 | `pane.lx.agents.runs` 按 `agent_path` 先序树保存 `SubagentRun`；`AgentFilter::Active` 定义存活态为 `Running \| Idle \| Blocked`（排除 `Completed` / `Failed` / `Interrupted` / `Cancelled`）。 |
| **发送准备与当轮上下文注入** | `src/chat/prepare.rs:135-218`<br>`src/chat/prepare/web.rs:92-97`<br>`src/event/chat/subagent/snapshot.rs:1-160`<br>`src/event/chat/mod.rs:580-645` | 623 行<br>285 行<br>228 行<br>895 行 | `prepare_user_message` 将 `ReferKind::Subagent` 同 `ReferKind::Mcp` 作为纯文本语法糖随正文入库（零 DB 迁移）；`prepare_ai_web_message` 拦截为 `LocalReferenceUnsupported`；`event/chat/subagent/snapshot.rs` 提取 `@[refer-subagent](...)` 注入当轮英文调度提示。 |

---

## 1. 目标与非目标

### 1.1 目标（Goals）
1. **在 `main_content/lx` 输入框的 `@` 命令面板新增 `Subagents` Tab**：
   - 顶栏顺序：`Files` → `refDir` → `Skills` → `MCP` → `Subagents`；
   - 支持键盘 `←` / `→` 循环切换、鼠标点击 Tab 切换、`↑` / `↓` 移动高亮、滚轮滚动视口、`Enter` / `Tab` / 鼠标点击确认插入、`Esc` 关闭。
2. **三类候选同源汇聚、分级排序与实时模糊过滤**：
   - **当前 session 存活的 subagent**（`running` / `idle` / `blocked`）置顶；
   - **内置角色**（`explorer`、`executor`、`monitor`）排中间；
   - **自定义角色**（已信任工作区的项目级 `.lx/agents/*.json` & `.agents/roles/*.json` + 全局用户级 `config/subagents.json`）排末尾；
   - 支持按昵称、`agent_id`、角色名、状态、任务标题、角色描述及类别词实时大小写不敏感过滤。
3. **结构化引用标记 `@[refer-subagent](...)` 与主 Agent 自动协同语义**：
   - 选中内置/自定义角色插入 `@[refer-subagent](<role_name>) `；
   - 选中当前会话存活实例插入 `@[refer-subagent](<nickname_or_id>:<role_name>) `；
   - 支持编辑器高亮、尾随空格处一次 `Backspace` 整块原子删除、以及正文中输入≥2字符触发快捷补全；
   - 发送消息时交由主 `lx` Agent 统一编排，并在当轮请求自动注入轻量英文 `<subagent_reference>` 上下文提示（点名角色引导 `spawn_agent`，点名存活实例引导 `send_input` / `wait_agent`）。
4. **严密的作用域隔离**：
   - 右侧 `Prompt` 编辑器维持原有 4 个 Tab（`Files` / `refDir` / `Skills` / `MCP`）不变，但底层支持 `@[refer-subagent](...)` 的高亮与整块退格删除；
   - `AiWeb`（网页端 AI）入口下 `@` 面板仍仅显示 `Files` Tab，手动粘贴 `@[refer-subagent](...)` 发送时按 `LocalReferenceUnsupported` 拦截并弹窗格内 Toast。

### 1.2 非目标（Non-Goals）
- **不在 UI 层绕过主 Agent 直接向子会话私发消息**：直接私聊子 Agent 已由 `/subagents`（`TaskPage::Agents`）面板的 `Enter` 内联回复框承担；主输入框的 `@` 提及服务于主会话的多工具/多 Agent 编排。
- **不修改 SQLite 数据库 Schema**：`@[refer-subagent](...)` 与 `@[refer-mcp](...)` 一致存储在用户消息正文中，当轮提示块按需在内存生成，无需新增 `SCHEMA_VERSION` 迁移。

---

## 2. Grill Me 已确认决策汇总表

| 编号 | 决策维度 | 用户已确认结论 |
| :--- | :--- | :--- |
| **Q1** | **插入标记格式与发送执行语义** | 1. **选中内置/自定义角色**：插入 `@[refer-subagent](<role_name>) `（如 `@[refer-subagent](explorer) `）；<br>2. **选中当前 session 存活实例**：插入 `@[refer-subagent](<nickname_or_id>:<role_name>) `（如 `@[refer-subagent](Turing:explorer) `）；<br>3. **发送语义**：消息发给主 `lx` Agent，并在组装当轮模型请求时自动注入英文 `<subagent_reference>` 引导块（角色 → 优先 `spawn_agent(role=...)`；存活实例 → 优先对该实例 `send_input` / `wait_agent`）。 |
| **Q2** | **存活判定口径、排序顺序与行展示格式** | 1. **存活口径**：严格对齐 `AgentFilter::Active`，仅纳入当前会话 `pane.lx.agents.runs` 中状态为 **`running`、`idle`、`blocked`** 的子 Agent（已结束的 `completed`/`failed`/`cancelled`/`interrupted` 不展示）；<br>2. **排列顺序**：**当前 session 存活实例置顶** → **内置角色**（`explorer`、`executor`、`monitor`）→ **自定义角色**（先项目级 `project`，后全局级 `user`）；<br>3. **双行展示格式**：<br>   - 存活实例：`◆ <nickname_or_id> [<role>]` + `Active · <status> · <title>`<br>   - 内置角色：`◆ <role_name>` + `Built-in · <description>`<br>   - 自定义角色：`◆ <role_name>` + `Custom (<project\|user> · <base_role>) · <description>` |
| **Q3** | **Tab 命名、位置与跨面板/入口边界** | 1. **名称与位置**：命名为 **`Subagents`**，排在 `MCP` 之后（`Files` → `refDir` → `Skills` → `MCP` → `Subagents`），空态文案为 `no matching subagents`；<br>2. **作用范围**：仅在 `main_content/lx` 主输入框（非 `AiWeb` 入口）展示 `Subagents` Tab；右侧 `Prompt` 面板保持 4 个 Tab 不变，但共享 `@[refer-subagent](...)` 语法高亮与整块退格删除；<br>3. **`AiWeb` 限制**：`AiWeb` 入口仅展示 `Files` Tab，若粘贴 `@[refer-subagent](...)` 发送则按 `LocalReferenceUnsupported` 拦截。 |

---

## 3. 架构与模块详细设计（含关键代码）

### 3.1 引用语法与编辑器生态扩展 (`src/app/markdown/refer.rs`, `refer_shortcut.rs`, `src/ui/markdown.rs`, `src/chat/prepare/`)

#### 3.1.1 扩展 `ReferKind::Subagent` (`src/app/markdown/refer.rs`)
在 `ReferKind` 中新增 `Subagent` 变体，对应标记 `@[refer-subagent](...)`：

```rust
/// 引用类型；与 `@[refer-*]` 标记一一对应。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ReferKind {
    File,
    Folder,
    Image,
    /// 用户显式点名的 skill（`@[refer-skill](<SKILL.md 绝对路径>)`）。
    Skill,
    /// 用户显式点名的 MCP server（`@[refer-mcp](<server_name>)`）。
    Mcp,
    /// 用户显式点名的 subagent 角色或存活实例（`@[refer-subagent](<role | nickname:role>)`）。
    Subagent,
}

impl ReferKind {
    /// 标记中的稳定类型名。
    pub fn as_str(self) -> &'static str {
        match self {
            Self::File => "file",
            Self::Folder => "folder",
            Self::Image => "image",
            Self::Skill => "skill",
            Self::Mcp => "mcp",
            Self::Subagent => "subagent",
        }
    }

    /// 从标记类型名解析。
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "file" => Some(Self::File),
            "folder" => Some(Self::Folder),
            "image" => Some(Self::Image),
            "skill" => Some(Self::Skill),
            "mcp" => Some(Self::Mcp),
            "subagent" => Some(Self::Subagent),
            _ => None,
        }
    }
}
```

#### 3.1.2 快捷引用标签与编辑器高亮 (`src/app/markdown/refer_shortcut.rs`, `src/ui/markdown.rs`)
1. 在 `src/app/markdown/refer_shortcut.rs` 的 `ReferTag::label` 中补充分支：
   ```rust
   Self::Refer(ReferKind::Subagent) => "refer-subagent",
   ```
2. 在 `src/ui/markdown.rs` 的 `refer_token` 中将 `ReferKind::Subagent` 归入 `TokenKind::ReferFile`（与 `Skill` / `Mcp` 同款亮绿下划线引用样式）：
   ```rust
   fn refer_token(kind: markdown::ReferKind) -> TokenKind {
       match kind {
           markdown::ReferKind::File => TokenKind::ReferFile,
           markdown::ReferKind::Folder => TokenKind::ReferFolder,
           markdown::ReferKind::Image => TokenKind::ReferImage,
           markdown::ReferKind::Skill
           | markdown::ReferKind::Mcp
           | markdown::ReferKind::Subagent => TokenKind::ReferFile,
       }
   }
   ```
3. **自动获得的编辑器能力**：
   - `parse_references` 自动识别 `@[refer-subagent](...)`；
   - 当光标紧跟 `@[refer-subagent](...) ` 尾随空格时，按 `Backspace` 由现有的 `reference_before_cursor` 一步原子删除整块标记（含可撤销快照）。

#### 3.1.3 发送预处理 (`src/chat/prepare.rs`, `src/chat/prepare/web.rs`)
1. 在 `src/chat/prepare.rs` 的 `prepare_user_message` 中：
   ```rust
   ReferKind::Mcp | ReferKind::Subagent => {
       // MCP 与 Subagent 引用为编辑器纯文本语法糖（已记入 claimed_ranges），随正文原样发送。
   }
   ```
2. 在 `src/chat/prepare/web.rs` 的 `prepare_ai_web_message` 中：
   ```rust
   ReferKind::Skill | ReferKind::Mcp | ReferKind::Subagent => {
       return Err(AiWebPrepareError::LocalReferenceUnsupported);
   }
   ```

---

### 3.2 `@` 面板 Tab 模型与 Subagent 候选数据结构 (`src/app/prompt/mention.rs`, `src/app/state/lx/mention.rs`)

#### 3.2.1 扩展 `MentionTab` 并隔离 `Prompt`（4 Tab）与 `Lx`（5 Tab） (`src/app/prompt/mention.rs`)

```rust
/// @ 命令面板顶栏 Tab。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum MentionTab {
    #[default]
    Files,
    RefDir,
    Skills,
    Mcp,
    Subagents,
}

impl MentionTab {
    /// `lx` 主输入框完整 Tab 列表（5 项）。
    pub const ALL: [Self; 5] = [
        Self::Files,
        Self::RefDir,
        Self::Skills,
        Self::Mcp,
        Self::Subagents,
    ];

    /// 右侧 `Prompt` 编辑器 Tab 列表（保持 4 项，不含会话态 `Subagents`）。
    pub const PROMPT_ALL: [Self; 4] = [
        Self::Files,
        Self::RefDir,
        Self::Skills,
        Self::Mcp,
    ];

    /// 由 `lx` 顶栏 Tab 下标（0..5）解析。
    pub fn from_index(index: usize) -> Option<Self> {
        Self::ALL.get(index).copied()
    }

    /// 由右侧 `Prompt` 顶栏 Tab 下标（0..4）解析。
    pub fn from_prompt_index(index: usize) -> Option<Self> {
        Self::PROMPT_ALL.get(index).copied()
    }

    /// `lx` 主输入框向后循环切换（5 Tab）。
    pub fn next(self) -> Self {
        match self {
            Self::Files => Self::RefDir,
            Self::RefDir => Self::Skills,
            Self::Skills => Self::Mcp,
            Self::Mcp => Self::Subagents,
            Self::Subagents => Self::Files,
        }
    }

    /// `lx` 主输入框向前循环切换（5 Tab）。
    pub fn prev(self) -> Self {
        match self {
            Self::Files => Self::Subagents,
            Self::RefDir => Self::Files,
            Self::Skills => Self::RefDir,
            Self::Mcp => Self::Skills,
            Self::Subagents => Self::Mcp,
        }
    }

    /// 右侧 `Prompt` 编辑器向后循环切换（4 Tab）。
    pub fn next_prompt(self) -> Self {
        match self {
            Self::Files => Self::RefDir,
            Self::RefDir => Self::Skills,
            Self::Skills => Self::Mcp,
            Self::Mcp | Self::Subagents => Self::Files,
        }
    }

    /// 右侧 `Prompt` 编辑器向前循环切换（4 Tab）。
    pub fn prev_prompt(self) -> Self {
        match self {
            Self::Files | Self::Subagents => Self::Mcp,
            Self::RefDir => Self::Files,
            Self::Skills => Self::RefDir,
            Self::Mcp => Self::Skills,
        }
    }
}
```

- 在 `src/app/prompt/mention.rs` 的 `MentionState`（右侧 `Prompt` 专用）中：
  - `switch_tab` 使用 `self.tab.next_prompt()` / `self.tab.prev_prompt()`；
  - `set_tab` 若传入 `MentionTab::Subagents` 直接返回 `false`；
  - `src/app/update/editor.rs::switch_mention_tab` 使用 `MentionTab::from_prompt_index(tab)`。

#### 3.2.2 定义 Subagent 候选条目模型 (`src/app/state/lx/mention.rs`)

在 `src/app/state/lx/mention.rs`（或由 `src/app/prompt/mention.rs` 导出）定义纯数据候选结构：

```rust
/// `@` 面板 `Subagents` Tab 中的存活实例快照（由当前窗格 `agents.runs` 同步）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SubagentRunMentionEntry {
    /// 稳定 agent_id。
    pub agent_id: String,
    /// 可选昵称（如 `"Turing"`）。
    pub nickname: Option<String>,
    /// 角色名（如 `"explorer"`）。
    pub role: String,
    /// 活动状态（`"running"` / `"idle"` / `"blocked"`）。
    pub status: String,
    /// 任务短标题。
    pub title: String,
}

impl SubagentRunMentionEntry {
    /// 引用标记中使用的目标标识：优先昵称，无昵称回退 `agent_id`。
    pub fn target_key(&self) -> &str {
        self.nickname
            .as_deref()
            .map(str::trim)
            .filter(|name| !name.is_empty())
            .unwrap_or(self.agent_id.as_str())
    }

    /// 首行展示标签：`<nickname_or_id> [<role>]`。
    pub fn display_label(&self) -> String {
        format!("{} [{}]", self.target_key(), self.role)
    }

    /// 确认后写入 `@[refer-subagent](...)` 括号内的目标串：`<nickname_or_id>:<role>`。
    pub fn reference_target(&self) -> String {
        format!("{}:{}", self.target_key(), self.role)
    }
}

/// `@` 面板 `Subagents` Tab 归一化后的可展示候选（存活实例 / 内置角色 / 自定义角色）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SubagentMentionItem {
    /// 当前会话存活的 subagent 实例（`running` / `idle` / `blocked`）。
    Active(SubagentRunMentionEntry),
    /// 内置或自定义角色规格。
    Role(crate::config::subagents::ResolvedRoleSpec),
}

impl SubagentMentionItem {
    /// 确认选择时写入 `@[refer-subagent](<target>)` 的 `<target>`。
    pub fn reference_target(&self) -> String {
        match self {
            Self::Active(run) => run.reference_target(),
            Self::Role(role) => role.name.clone(),
        }
    }

    /// 查询词匹配（大小写不敏感）。
    pub fn matches_query(&self, query_lower: &str) -> bool {
        if query_lower.is_empty() {
            return true;
        }
        match self {
            Self::Active(run) => {
                run.target_key().to_lowercase().contains(query_lower)
                    || run.agent_id.to_lowercase().contains(query_lower)
                    || run.role.to_lowercase().contains(query_lower)
                    || run.status.to_lowercase().contains(query_lower)
                    || run.title.to_lowercase().contains(query_lower)
                    || "active".contains(query_lower)
            }
            Self::Role(role) => {
                let scope_tag = if role.is_builtin {
                    "built-in builtin"
                } else if role.is_project_scoped {
                    "custom project"
                } else {
                    "custom user"
                };
                role.name.to_lowercase().contains(query_lower)
                    || role.base_role.as_str().contains(query_lower)
                    || role.description.to_lowercase().contains(query_lower)
                    || scope_tag.contains(query_lower)
            }
        }
    }
}
```

#### 3.2.3 `LxMention` 中的聚合、排序、过滤与确认插入 (`src/app/state/lx/mention.rs`)

1. **字段与默认初始化**：
   - `LxMention` 新增：
     ```rust
     /// 当前可用 subagent 角色列表（内置 + 自定义）；默认预填 3 个内置角色。
     subagent_roles: Vec<crate::config::subagents::ResolvedRoleSpec>,
     /// 当前会话存活的 subagent 运行实例列表（仅 `running` / `idle` / `blocked`）。
     subagent_runs: Vec<SubagentRunMentionEntry>,
     ```
   - 在 `Default for LxMention` 中将 `subagent_roles` 初始化为 `crate::config::subagents::SubagentsSection::default().all_roles()`（即 `explorer`、`executor`、`monitor` 三个内置角色开箱即用，即使无外部事件注入也始终可选）。
2. **排序与过滤 `filtered_subagents(&self) -> Vec<SubagentMentionItem>`**：
   - 按固定三段顺序组装并按 `query_lower` 过滤：
     1. **当前 session 存活实例**：`self.subagent_runs.iter().cloned().map(SubagentMentionItem::Active)`；
     2. **内置角色**：`self.subagent_roles.iter().filter(|r| r.is_builtin).cloned().map(SubagentMentionItem::Role)`；
     3. **自定义角色**：先项目级 `self.subagent_roles.iter().filter(|r| !r.is_builtin && r.is_project_scoped)`，再用户级 `self.subagent_roles.iter().filter(|r| !r.is_builtin && !r.is_project_scoped)`。
3. **`secondary_count`、`is_open`、`view` 与 `confirm` 扩展**：
   - `secondary_count()` 新增 `MentionTab::Subagents => self.filtered_subagents().len()`；
   - `is_open()` 与 `view()` 中将 `MentionTab::Subagents` 并入 `MentionTab::RefDir | MentionTab::Skills | MentionTab::Mcp | MentionTab::Subagents` 分支；
   - `confirm()` 新增分支：
     ```rust
     MentionTab::Subagents => {
         let item = self.filtered_subagents().get(self.ref_active)?.clone();
         let target = item.reference_target();
         crate::app::markdown::format_reference(
             crate::app::markdown::ReferKind::Subagent,
             Path::new(&target),
         )
         .unwrap_or_else(|| format!("@[refer-subagent]({target})"))
     }
     ```

---

### 3.3 状态同步链路 (`src/app/state/lx.rs`, `src/app/update/chat/`, `src/event/inventory.rs`, `src/db/service/chat.rs`)

#### 3.3.1 窗格内当前会话存活实例同步 (`Lx::sync_mention_subagent_runs`)
在 `src/app/state/lx/panels.rs` 中为 `Lx` 提供同步方法：

```rust
/// 从 `self.agents.runs` 提取当前会话存活（`running` / `idle` / `blocked`）的 subagent，
/// 同步至 `@` 提及面板；内容变化时自动重算面板。
pub fn sync_mention_subagent_runs(&mut self) -> bool {
    let active_runs: Vec<super::mention::SubagentRunMentionEntry> = self
        .agents
        .filtered_indices(super::task::AgentFilter::Active)
        .into_iter()
        .filter_map(|index| self.agents.runs.get(index))
        .map(|run| super::mention::SubagentRunMentionEntry {
            agent_id: run.agent_id.clone(),
            nickname: run.nickname.clone(),
            role: run.role.as_str().to_string(),
            status: run.status.clone(),
            title: run.display_title().to_string(),
        })
        .collect();
    let masked = crate::app::paste::mask_pasted_text(&self.input).into_owned();
    self.mention
        .sync_subagent_runs(active_runs, &masked, self.cursor)
}
```

调用点覆盖全部生命周期：
1. `Lx::refresh_panel(&mut self)`：每次用户输入或移动光标触发 `@` 面板重算前，先调 `self.sync_mention_subagent_runs()`；
2. `Lx::clear_agents(&mut self)`：切会话或清空时同步清空 `mention` 中的 `subagent_runs`；
3. `src/app/update/chat/subagent.rs`：在 `on_subagent_started`、`on_subagent_finished`、`set_subagent_list` 更新 `target.lx.agents.runs` 后调用 `target.lx.sync_mention_subagent_runs()`；
4. `src/db/service/chat.rs::load_chat_session_into_pane`：在 `/resume` 或切换会话加载完成后，推入 `Effect::ListSubagents { pane, parent_session: session_id.to_string() }`，使恢复会话中仍处于 `idle` / `blocked` 的子 Agent 立即同步到 `pane.lx.agents.runs` 与 `@` 面板。

#### 3.3.2 内置与自定义角色同步 (`src/event/inventory.rs`, `src/app/state/pane.rs`)
1. 在 `src/event/inventory.rs` 新增 `refresh_subagent_mentions(state: &mut AppState, persistence: &Persistence)`：
   ```rust
   /// 刷新所有打开窗格的 `@` 面板 Subagents 角色候选（内置角色 + 全局自定义 + 已信任工作区项目级角色）。
   pub(crate) fn refresh_subagent_mentions(state: &mut AppState, persistence: &Persistence) {
       for workspace in &mut state.workspaces {
           let config_root = workspace
               .repo_key()
               .map(Path::to_path_buf)
               .or_else(|| workspace.cwd.clone());
           let workspace_root = workspace.cwd.clone();
           let trusted = workspace_root
               .as_deref()
               .is_some_and(|cwd| persistence.is_trusted(cwd))
               || config_root
                   .as_deref()
                   .is_some_and(|root| persistence.is_trusted(root));
           let effective = crate::config::subagents::merge_project_roles_for_paths(
               &state.subagents_config,
               config_root.as_deref(),
               workspace_root.as_deref(),
               trusted,
           );
           let roles = effective.all_roles();
           for tab in &mut workspace.tabs {
               for id in tab.layout.pane_ids() {
                   if let Some(pane) = tab.pane_mut(id) {
                       pane.lx.set_mention_subagent_roles(roles.clone());
                   }
               }
           }
       }
   }
   ```
2. 触发时机：
   - TUI 启动初始化（`src/event/mod.rs` 中紧随 `refresh_agents_md`）；
   - 设置页保存 Subagents 配置（`src/event/inventory.rs` 中 `SettingsEvent::SubagentsChanged` 到达时）；
   - 工作区切换、cwd 变化或信任状态变更（`src/event/mod.rs` 中 `skills_reload` 或 `current_agents_scopes != agents_scopes` 时）；
   - 分割窗格时（`src/app/state/pane.rs`），与 `mention_skills()` / `mention_mcp_servers()` 一起将 `mention_subagent_roles()` 复制给新窗格。

---

### 3.4 TUI 渲染层与英文文案设计 (`src/ui/text/workspace.rs`, `src/ui/mention.rs`, `src/ui/main_content/lx/mention.rs`)

#### 3.4.1 新增用户可见英文文案 (`src/ui/text/workspace.rs`)
```rust
pub const MENTION_TAB_SUBAGENTS: &str = "Subagents";
pub const MENTION_SUBAGENTS_EMPTY: &str = "no matching subagents";
pub const MENTION_SUBAGENT_ACTIVE_LABEL: &str = "Active";
pub const MENTION_SUBAGENT_BUILTIN_LABEL: &str = "Built-in";
pub const MENTION_SUBAGENT_CUSTOM_LABEL: &str = "Custom";
```

#### 3.4.2 新增 `ui::mention::subagent_row` (`src/ui/mention.rs`)
```rust
/// 构造 subagent 候选条目展示：首行 `◆ label`，次行类别与状态/描述明细。
pub fn subagent_row(label: &str, detail: &str, detail_width: usize) -> MentionRow {
    MentionRow {
        icon: text::MENTION_SKILL_ICON,
        is_directory: false,
        is_skill: true,
        label: label.to_string(),
        detail: text::truncate_to_width(detail, detail_width),
    }
}

/// 组装 subagent 候选项的 `(label, detail)` 文案。
pub fn subagent_item_text(
    item: &crate::app::state::SubagentMentionItem,
) -> (String, String) {
    let sep = text::LX_CHAT_TOOL_STATUS_SEP;
    match item {
        crate::app::state::SubagentMentionItem::Active(run) => {
            let label = run.display_label();
            let detail = if run.title.trim().is_empty() {
                format!("{}{sep}{}", text::MENTION_SUBAGENT_ACTIVE_LABEL, run.status)
            } else {
                format!(
                    "{}{sep}{}{sep}{}",
                    text::MENTION_SUBAGENT_ACTIVE_LABEL,
                    run.status,
                    run.title.trim()
                )
            };
            (label, detail)
        }
        crate::app::state::SubagentMentionItem::Role(role) => {
            let prefix = if role.is_builtin {
                text::MENTION_SUBAGENT_BUILTIN_LABEL.to_string()
            } else {
                let scope = if role.is_project_scoped {
                    "project"
                } else {
                    "user"
                };
                format!(
                    "{} ({scope}{sep}{})",
                    text::MENTION_SUBAGENT_CUSTOM_LABEL,
                    role.base_role.as_str()
                )
            };
            let detail = if role.description.trim().is_empty() {
                prefix
            } else {
                format!("{prefix}{sep}{}", role.description.trim())
            };
            (role.name.clone(), detail)
        }
    }
}
```

#### 3.4.3 `main_content/lx/mention.rs` 挂载 `Subagents` Tab (`src/ui/main_content/lx/mention.rs`)
1. 在 `panel_data` 的非 `AiWeb` 分支追加第 5 个 `PanelTab`：
   ```rust
   PanelTab {
       label: text::MENTION_TAB_SUBAGENTS,
       active: tab == MentionTab::Subagents,
   },
   ```
2. 在 `match (tab, lx.mention().view(&lx.input, lx.cursor))` 中新增分支：
   ```rust
   (MentionTab::Subagents, LxMentionView::Empty | LxMentionView::Loading) => {
       Some(status(text::MENTION_SUBAGENTS_EMPTY))
   }
   (MentionTab::Subagents, LxMentionView::Items) => {
       let filtered = lx.mention().filtered_subagents();
       if filtered.is_empty() {
           return Some(status(text::MENTION_SUBAGENTS_EMPTY));
       }
       Some(PanelData {
           rows: filtered
               .iter()
               .map(|item| {
                   let (label, detail) = mention::subagent_item_text(item);
                   PanelRow::File(mention::subagent_row(&label, &detail, detail_width))
               })
               .collect(),
           active: lx.mention().ref_active(),
           anchor: Some(lx.mention().ref_anchor()),
           viewport: lx.mention().ref_viewport(),
           scope: None,
           interactive: true,
           tabs,
       })
   }
   ```

---

### 3.5 发送时当轮模型上下文提示注入 (`src/event/chat/subagent/snapshot.rs`, `src/event/chat/mod.rs`)

当用户发送包含 `@[refer-subagent](...)` 的消息时，在 `src/event/chat/subagent/snapshot.rs` 中解析引用并生成英文 `<subagent_reference>` 提示块，与 `skill_blocks` 一起作为当轮附加用户上下文发给主模型（不写库、不污染气泡展示）：

```rust
/// 从用户当轮草稿中提取 `@[refer-subagent](...)` 引用，生成英文调度提示块（仅当轮注入，不落库）。
pub(crate) fn subagent_reference_blocks(
    draft_text: &str,
    effective_subagents: &crate::config::subagents::SubagentsSection,
    runs: &[crate::app::state::SubagentRun],
) -> Vec<String> {
    let mut blocks = Vec::new();
    let mut seen = std::collections::HashSet::new();
    let masked = crate::app::paste::mask_pasted_text(draft_text);
    for reference in crate::app::markdown::parse_references(&masked) {
        if reference.kind != crate::app::markdown::ReferKind::Subagent {
            continue;
        }
        let raw_target = reference.path.trim();
        if raw_target.is_empty() || !seen.insert(raw_target.to_string()) {
            continue;
        }
        // 1. 优先匹配 `<nickname_or_id>:<role>` 格式或当前会话中的存活 subagent 实例
        let instance_key = raw_target
            .split_once(':')
            .map(|(head, _)| head.trim())
            .unwrap_or(raw_target);
        if let Some(run) = runs.iter().find(|run| {
            crate::app::state::AgentFilter::Active.matches(run)
                && (run.agent_id.eq_ignore_ascii_case(instance_key)
                    || run
                        .nickname
                        .as_deref()
                        .is_some_and(|nick| nick.eq_ignore_ascii_case(instance_key)))
        }) {
            let nickname = run.nickname.as_deref().unwrap_or(run.agent_id.as_str());
            blocks.push(format!(
                "<subagent_reference>\n\
                 kind: active_instance\n\
                 reference: @[refer-subagent]({raw_target})\n\
                 agent_id: {}\n\
                 nickname: {nickname}\n\
                 role: {}\n\
                 status: {}\n\
                 agent_path: {}\n\
                 guidance: The user explicitly referenced this active subagent in the current session. \
                 Prefer interacting with it directly using `send_input` (target=\"{}\") or `wait_agent` \
                 instead of spawning a duplicate subagent.\n\
                 </subagent_reference>",
                run.agent_id,
                run.role.as_str(),
                run.status,
                run.agent_path,
                run.agent_id,
            ));
            continue;
        }

        // 2. 匹配内置或自定义角色（或存活实例已结束时回退到其 `<role>`）
        let role_name = raw_target
            .split_once(':')
            .map(|(_, role)| role.trim())
            .unwrap_or(raw_target);
        if let Some(spec) = effective_subagents.resolve_role(role_name) {
            let scope = if spec.is_builtin {
                "built-in"
            } else if spec.is_project_scoped {
                "custom (project)"
            } else {
                "custom (user)"
            };
            blocks.push(format!(
                "<subagent_reference>\n\
                 kind: role\n\
                 reference: @[refer-subagent]({raw_target})\n\
                 role: {}\n\
                 base_role: {}\n\
                 scope: {scope}\n\
                 description: {}\n\
                 guidance: The user explicitly referenced the `{}` subagent role. \
                 Delegate the relevant task by calling `spawn_agent` with `role: \"{}\"`.\n\
                 </subagent_reference>",
                spec.name,
                spec.base_role.as_str(),
                spec.description,
                spec.name,
                spec.name,
            ));
        }
    }
    blocks
}
```

在 `src/event/chat/mod.rs::start_stream` 中：
```rust
let mut turn_reference_blocks = skills::reference_blocks(&draft_text);
turn_reference_blocks.extend(subagent::snapshot::subagent_reference_blocks(
    &draft_text,
    &effective_subagents,
    &pane.lx.agents.runs,
));
```
随后将 `&turn_reference_blocks` 传给 `assemble_turn_request_with_context`。

---

## 4. 边界条件与测试矩阵

| 场景 | 预期行为 | 覆盖测试位置 |
| :--- | :--- | :--- |
| **默认开箱状态** | 新建 `Lx` 实例输入 `@` 切到 `Subagents` Tab，无需外部异步事件即可看到 `explorer`、`executor`、`monitor` 三个内置角色。 | `src/app/state/lx/mention/tests.rs` |
| **三类候选排序与存活过滤** | `agents.runs` 中同时存在 `running`、`idle`、`blocked`、`completed`、`cancelled` 实例，且配置含项目级与用户级自定义角色时：仅 `running`/`idle`/`blocked` 出现在顶部，其后为 3 个内置角色，再后为项目级自定义角色与用户级自定义角色。 | `src/app/state/lx/mention/tests.rs` |
| **确认插入与原子退格删除** | 选中存活实例插入 `@[refer-subagent](Turing:explorer) `，选中角色插入 `@[refer-subagent](explorer) `；紧跟末尾空格按一次 `Backspace` 整块删除，按 `Ctrl+Z` 一步还原。 | `src/app/state/lx/mention/tests.rs` |
| **左右方向键 5-Tab vs 4-Tab 循环** | `lx` 主输入框在 `Files` 按 `←` 跳到 `Subagents`，在 `Subagents` 按 `→` 跳回 `Files`；右侧 `Prompt` 编辑器在 `Files` 按 `←` 仍跳到 `Mcp`，不进入 `Subagents`。 | `src/app/state/lx/mention/tests.rs`、`src/app/prompt/tests.rs` |
| **`AiWeb` 入口防护** | `AiWeb` 入口下 `mention_switch_tab` / `mention_set_tab(MentionTab::Subagents)` 返回 `false`，顶栏仅渲染 `Files`；含 `@[refer-subagent](...)` 的草稿在 `prepare_ai_web_message` 返回 `Err(AiWebPrepareError::LocalReferenceUnsupported)`。 | `src/chat/prepare/tests.rs`、`src/app/state/lx/mention/tests.rs` |
| **当轮模型上下文提示块生成** | 草稿含 `@[refer-subagent](Turing:explorer)` 与 `@[refer-subagent](monitor)` 时，`subagent_reference_blocks` 分别产出 `active_instance`（含 `agent_id` 与 `send_input` 引导）和 `role`（含 `spawn_agent` 引导）的英文 `<subagent_reference>` 块，重复点名自动去重。 | `src/event/chat/tests/subagent_roles.rs` |
