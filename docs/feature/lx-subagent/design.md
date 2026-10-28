# lx Subagent（explorer / executor）— 设计

- 分支：`feat/lx-subagent`（功能已完成并合并回 `dev`；独立 worktree 与分支已移除）
- 基线：功能分支基于 `dev @ 3a2642e`，合并提交见 `dev` 历史
- 状态：**已实现并合并到 `dev`（2026-10-28）；评审与缺陷修复见 [review.md](review.md)**
- 参考：`codex-main` 的 `agent/role.rs`、`tools/handlers/multi_agents*`；Claude Code 的 `Task` 工具语义

## 0. 旧版作废说明（核实后的事实纠正）

旧 `design.md` / `task.md` 有 5 处与当前代码不符，**本文以此为准**：

| 旧说法 | 当前事实（已核实） |
|--------|-------------------|
| dev 仍是 `SCHEMA_VERSION = 14`，本分支取 16 并跳过 15 | `src/db/mod.rs:26` 已是 **15**，`SCHEMA_V15_TODO`（`migration.rs:519`）已在 dev 执行。本分支取 **16**，正常递进，无跳过 |
| `task.md` 7.3 `HistorySource` 抽取已 `[x]` | **不存在**。`snapshot::build(width, lx: &Lx, cache)`（`snapshot.rs:104`）仍直接吃 `&Lx`；`HistoryView` 是滚动几何类型（`history.rs:230`） |
| `TOOL_NAMES` 扩到 12、含 `send_input` | 现为 8 项（`types.rs:10`）；本次砍掉 `send_input`，最终 **11 项** |
| `agent_subagent` 表已可用 | 表在 v7 建好（`migration.rs:206`），**全代码库零引用**、零数据 |
| `TaskPage::len()` 已支持 Agents | `task.rs:108` 对 `TaskTab::Agents` 恒返回 0（预留位） |

## 1. 目标与范围

给 lx 增加「主 agent 委派子 agent」：

1. 两个内置角色：`explorer`（真只读调查）与 `executor`（有边界的实现/修复）。
2. 主 agent 通过工具派活，**派完可继续干活**；需要时用 `wait_agent` 同步等待。
3. 每会话并发上限 **3**，超限硬拒绝。
4. `Agents` 页签：列表（角色/状态/时长/任务首行）+ 高亮项的**完整消息流**（复用主历史渲染与滚动）。
5. 主 agent 消息列表新增两类组件：**委派卡片**（工具卡，方块 `■`）与**汇报块**（子 agent 主动汇报，上三角 `▲`）。
6. 子 agent 的完整过程独立落库（子会话），可回看；主 agent 上下文只收「结论 + 结构化尾巴」。

本次**不做**：

- 子 agent 递归（子工具面无 `spawn_agent`，执行层也兜底拒绝）。
- `send_input`（纠偏）——改用 `close_agent` + 重新 `spawn_agent`。
- 文件所有权锁、三方合并。
- 子会话的 `/compact`、`/rewind`、侧栏列表项。
- 自动重跑未完成的 subagent。
- 角色级模型/思考等级覆盖；角色级步数上限。
- 修改 `agent_message.kind` 的 CHECK 约束（不重建该表）。

## 2. 已确认决策（Grill Me 收口，31 轮）

| # | 决策项 | 结论 |
|---|--------|------|
| 1 | 存储 | 每个 subagent = 独立子会话（`source='subagent'` + 自己的 message/part/tool_call），`agent_subagent` 挂到父会话；**落库** |
| 2 | 重启恢复 | **懒加载**：启动不扫子会话；进 `Agents` 页签只读元数据，选中某项才 `load_chat_session`。启动时把 `pending/running` → `cancelled`、子会话 streaming 消息 → `interrupted`，**不重跑** |
| 3 | 报告投递 | 父窗格 `Idle`、无用户排队提交 → 把所有未投递报告合并成一条报告消息 + 一个自动 assistant；用户输入优先，父忙则本轮结束后再 drain；不注入用户轮 |
| 4 | 并发 | 常量 `MAX_CONCURRENT_SUBAGENTS = 3`，**硬拒绝**（工具级失败），不配置化、不排队 |
| 5 | 渲染复用 | 新增 `HistorySource<'a>` 数据源类型，`HistoryView`（滚动几何）**保持原名不变**。子 agent 详情区用同一套 `snapshot`/`render`/`scrollbar` |
| 6 | 并发写冲突 | 不新增锁。子会话用**自己的** `ReadRegistry`，靠读后版本 token 拦旧读写入；提示词划边界 |
| 7 | 只读保证 | **双保险**：schema 按 `ToolSurface` 裁剪 + 执行层白名单兜底（`UNAVAILABLE_CODE`） |
| 8 | UI 形态 | 委派 = 工具卡片（一个 `ToolDetailView::Subagent` 变体，按 `call.name` 取标题）；汇报 = 消息内独立块；**不合并、不原地变形** |
| 9 | executor 工具面 | `read/search/edit/write/exec_command/write_stdin`（**6 项，无 `update_plan`、无 `ask`**）。不给子 agent 计划槽，避免覆盖主 agent 的 `lx.chat.plan` |
| 10 | 报告行 | 只有**后台自动轮**才落一条内部 user 消息（`subagent_report_json` 为数组），UI 画大号汇报块；`wait_agent` 已消费的报告只进工具结果，不落消息 |
| 11 | 工具集 | 主 agent 11 项 = 现有 8 + `spawn_agent` / `wait_agent` / `close_agent` |
| 12 | 窗格不存在 | 不在该窗格自动起轮；报告留在 DB（`report_delivered=0`），下次打开会话时由 idle drain 处理；面板仍能看到它跑完 |
| 13 | 进程回收 | 运行中的子会话加进 `reap_orphan_runtimes` 的 `bound` 白名单；子 turn 结束即从 `streams.subagents` 移除，下次 sweep 回收 |
| 14 | 工具握手 | 子 turn 的 `BeginTool`/`FinishTool` **不查 `AppState` 窗格**，用「子会话 id + `child_assistant_id` + `db_workspaces[child_key]`」自证 |
| 15 | 模型 | spawn 时冻结主 agent 当前 `FrozenModel`；不可用则 spawn 失败，不降级 |
| 16 | 报告尾巴 | 只 3 项：`status` / `files_changed` / `duration_ms`。正文上限 8 KiB，超限截断标 `[truncated]` |
| 17 | 步数上限 | 沿用 `MAX_MODEL_STEPS = 24`（`agent_loop/mod.rs:46`），不按角色区分 |
| 18 | 面板滚动 | **几何共用、状态分开**：`Agents` 用 `HistorySource` + `history::view` + `follow` 语义；`Processes` 保留 `output/output_scroll` |
| 19 | files_changed | `SubagentHandle.changed_paths: HashSet<PathBuf>`，在 mutation 结算处按**去重路径**累计；新增/删除/移动都算 |
| 20 | 汇报挂载 | 后台批次报告卡在 assistant 之前；重启由 `load_chat_session` 还原。无同轮 assistant 的旧 wait 残留 user-only 行 UI 隐藏 |
| 21 | `wait_agent` | 保留。默认 5 分钟、上限 1 小时；`wait_for=all` 超时后自动续等直到目标结束；`any` 超时返回 `timed_out: true`，**不是错误**、不写 `error_text` |
| 22 | `close_agent` | 保留。复用 `watch` 取消通道；关闭**不产生报告** |
| 23 | 停止语义 | Esc 只取消主轮（含释放正在等待的 `wait_agent` 调用），**不取消任何 running 子 agent**；它们继续跑，结果留在 Agents 面板。Agents 面板 `^d` 只强停当前高亮的 running 子 agent，其余继续跑；partial 留在面板，不自动重跑/起轮；主 agent 下一次用户消息唤醒 |
| 24 | 会话删除 | DB 外键已级联；内存态显式取消 `streams.subagents` 中该会话的子 turn、清 `Lx.agents` |
| 25 | 双窗格同会话 | 报告只投给 spawn 时的原始窗格（`SubagentHandle.parent.pane`）；另一窗格靠懒加载看历史，不接实时增量、不自动起轮 |
| 26 | 子 delta | 独立分支：节流常量（500ms / 4KiB）复用，落库直接调 `persistence.checkpoint_parts`，**不走** `schedule_checkpoint` |
| 27 | 迁移 | v16 三条 `ADD COLUMN`，各带 `column_exists` 守卫，**不重建任何表** |
| 28 | 面板列表 | 一条 SQL 联查 `agent_subagent JOIN agent_session`；消息流选中才加载 |
| 29 | 列表上限 | 保留该会话全部 subagent，不设上限、不自动清理 |
| 30 | 运行上下文 | 在册 subagent 摘要并入 `runtime_context_summary`（最多 3 条），避免历史裁剪后丢失 `agent_id` |
| 31 | Git | 独立 worktree + 分支 `feat/lx-subagent`；功能完成后已合并回 `dev`，worktree/分支已移除 |
| 32 | 统计行 | 统计行是任务收尾信息：本会话仍有子 agent 在跑，或后面还有可见报告批次时，中间 assistant 都不显示；最后一个子 agent/报告结束后的 assistant 才显示 |
| 33 | 状态提示 | 主轮在途（含 `wait_agent` 调用）与报告 drain/自动轮期间显示输入框上方 loading；主轮自然结束但后台任务/子 agent 仍在跑时保留输入框上方 loading；**用户显式停止主轮后**不显示 loading，改在上下文占用右侧显示 `N process · N subagents`；用户排队输入优先 |

## 3. 领域类型（`src/chat/agent.rs`，新文件）

纯数据，无 IO、无 `app/`/`ui/` 依赖。

```rust
/// 内置子 agent 角色。名字是稳定契约（工具参数 / DB `agent_role` / 提示 / 日志），
/// 不得改名；改名等于换角色，必须同时递增 `PROMPT_VERSION`。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentRole { Explorer, Executor }

impl AgentRole {
    pub const ALL: [AgentRole; 2] = [Self::Explorer, Self::Executor];
    /// 稳定存储/协议值：`explorer` / `executor`。
    pub const fn as_str(self) -> &'static str { .. }
    pub fn parse(text: &str) -> Option<Self> { .. }
    /// 角色系统提示（英文，见 §7）。
    pub const fn system_prompt(self) -> &'static str { .. }
    /// 该角色可用工具名白名单（decision 7/9）。
    pub const fn tools(self) -> &'static [&'static str] { .. }
}

/// 工具面：决定一次请求声明哪些工具。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolSurface { Main, Explorer, Executor }

impl ToolSurface {
    /// 主 = 11 项（8 + 3 控制工具）；Explorer = 2；Executor = 6。
    pub const fn names(self) -> &'static [&'static str] { .. }
    pub const fn from_role(role: AgentRole) -> Self { .. }
}

/// 每会话并发上限（decision 4）。
pub const MAX_CONCURRENT_SUBAGENTS: usize = 3;
/// 单次 `wait_agent` 的目标数上限。
pub const MAX_WAIT_TARGETS: usize = 8;
/// `wait_agent` 默认 / 上限超时（毫秒，decision 21）。
pub const DEFAULT_WAIT_TIMEOUT_MS: u64 = 300_000;
pub const MAX_WAIT_TIMEOUT_MS: u64 = 3_600_000;
/// 报告正文上限（decision 16）。
pub const MAX_REPORT_BYTES: usize = 8 * 1024;
/// `spawn_agent` 的 task 文本上限。
pub const MAX_TASK_CHARS: usize = 8_000;

/// 稳定错误码（进 `ToolErrorCode::Io` 的 message，不回显内部路径）。
pub const LIMIT_REACHED_CODE: &str = "subagent concurrency limit reached";
pub const NOT_FOUND_CODE: &str = "subagent not found";
pub const FINISHED_CODE: &str = "subagent already finished";
pub const UNAVAILABLE_CODE: &str = "subagent tools unavailable";
```

工具面名单（逐字，测试锁死）：

```text
Main     = [read, search, edit, write, exec_command, write_stdin, update_plan, ask,
            spawn_agent, wait_agent, close_agent]                 // 11
Explorer = [read, search]                                          // 2
Executor = [read, search, edit, write, exec_command, write_stdin]  // 6
```

### 3.1 控制工具参数与结果

```rust
/// 三个控制工具之一；由 loop 转投事件层执行（与 `ask` 同一分支位置）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SubagentOp {
    Spawn { role: AgentRole, task: String },
    Wait { targets: Vec<String>, timeout_ms: u64 },
    Close { target: String },
}

/// 操作结果：工具结果 JSON 与 UI 详情共用的唯一来源。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SubagentOutcome {
    Spawned { agent_id: String, role: AgentRole },
    Waited { timed_out: bool, agents: Vec<AgentStatusView> },
    Closed { agent_id: String, previous_status: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentStatusView {
    pub agent_id: String,
    pub role: AgentRole,
    /// running / completed / failed / cancelled
    pub status: String,
    /// 本次等待结算到的报告正文；未结束或已投递过为 None。
    pub report: Option<String>,
}

/// 工具名 → 操作。非控制工具返回 None；参数非法返回 Err(ToolFailure)。
pub fn parse_control(name: &str, arguments_json: &str) -> Option<Result<SubagentOp, ToolFailure>>;
/// 结果序列化（UI 详情、历史恢复、模型回执同源）。
impl SubagentOutcome {
    pub fn into_result(self) -> crate::chat::tools::ToolExecutionResult;
    pub fn to_json(self) -> serde_json::Value;
    pub fn from_json(value: &serde_json::Value) -> Option<Self>;
}
```

`parse_control` 规则：只接受白名单字段（多余字段 → `InvalidArguments`）；`task` 非空且 ≤ `MAX_TASK_CHARS`；`targets` 非空且 ≤ `MAX_WAIT_TARGETS`；`timeout_ms` 钳到 `[1, MAX_WAIT_TIMEOUT_MS]`。

### 3.2 报告信封

```rust
/// 注入父会话的报告文本。风格对齐 `context::format_user_shell`
/// （`chat/context/mod.rs:388`）：纯文本 + XML 包裹 + 元字符转义。
pub fn format_report_envelope(report: &SubagentReportView) -> String;
```

形状：

```text
<subagent_report agent_id="sub-…" role="explorer" status="completed"
                 files_changed="2" duration_ms="14200">
…正文（≤ 8 KiB，已截断时末尾带 [truncated]）…
</subagent_report>
```

正文必须过 `sanitize_control` + XML 转义；`agent_id`/`role`/`status` 是受控枚举，不转义也必须过白名单校验。

## 4. 持久化（v16）

### 4.1 现有表（已核实，零引用）

```sql
-- src/db/migration.rs:206
CREATE TABLE IF NOT EXISTS agent_subagent (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    external_id TEXT NOT NULL UNIQUE,
    parent_session_id TEXT NOT NULL REFERENCES agent_session(external_id) ON DELETE CASCADE,
    child_session_id  TEXT NOT NULL UNIQUE REFERENCES agent_session(external_id) ON DELETE CASCADE,
    spawn_message_id  TEXT REFERENCES agent_message(external_id) ON DELETE SET NULL,
    status TEXT NOT NULL CHECK (status IN ('pending','running','blocked','completed','failed','cancelled')),
    nickname TEXT, role TEXT, agent_path TEXT,
    model_provider TEXT, model_id TEXT, variant TEXT,
    created_at_ms INTEGER NOT NULL, updated_at_ms INTEGER NOT NULL, completed_at_ms INTEGER,
    CHECK (parent_session_id <> child_session_id)
);
```

`agent_session.source` 的 CHECK 已含 `'subagent'`（`migration.rs:443`），`agent_role` / `agent_path` / `agent_nickname` 三列已存在。

### 4.2 v16 迁移

```rust
// src/db/migration.rs
const SCHEMA_VERSION: i64 = 16; // src/db/mod.rs:26，15 → 16

// migrate() 末尾新增：
if version < 16 {
    let transaction = self.conn.unchecked_transaction()?;
    // 只加列，不重建表（AGENTS.md 警告的整表重建一律避开）。
    for (table, column, ddl) in [
        ("agent_subagent", "report_json",       "ALTER TABLE agent_subagent ADD COLUMN report_json TEXT;"),
        ("agent_subagent", "report_delivered",  "ALTER TABLE agent_subagent ADD COLUMN report_delivered INTEGER NOT NULL DEFAULT 0;"),
        ("agent_message",  "subagent_report_json", "ALTER TABLE agent_message ADD COLUMN subagent_report_json TEXT;"),
    ] {
        if !column_exists(&transaction, table, column)? {
            transaction.execute_batch(ddl)?;
        }
    }
    ensure_no_foreign_key_violation(&transaction)?;
    transaction.pragma_update(None, "user_version", SCHEMA_VERSION)?;
    transaction.commit()?;
}
```

`report_json` 存 `SubagentReportView` 的 JSON（含已截断正文）；`report_delivered` 是「恰好一次」的唯一判据。

### 4.3 新增 DB 接口（`src/db/chat/subagent.rs`，新文件）

```rust
/// 一个 subagent 的元数据行（列表用；不含消息流）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SubagentRecord {
    pub agent_id: String,
    pub child_session_id: String,
    pub parent_session_id: String,
    pub spawn_message_id: Option<String>,
    pub role: AgentRole,
    pub status: String,
    pub task_title: String,       // agent_session.title（= task 首行）
    pub created_at_ms: i64,
    pub completed_at_ms: Option<i64>,
    pub report_json: Option<String>,
    pub report_delivered: bool,
}

/// spawn 的提交结果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SubagentCommitted {
    pub agent_id: String,
    pub child_session_id: String,
    pub child_assistant_id: String,
    pub created_at_ms: i64,
}

impl Database {
    /// spawn：一个事务内建子会话 + user 消息 + streaming assistant + agent_subagent 行。
    pub fn begin_subagent_turn(
        &self,
        workspace: &Path,
        parent_session: &str,
        role: AgentRole,
        task: &str,
        model: &ModelSnapshot,
        spawn_message_id: Option<&str>,
    ) -> Result<SubagentCommitted>;

    /// 子 turn 终态：子 assistant 消息 finish（复用 `finish`）+ `agent_subagent` 状态/报告。
    pub fn finish_subagent_turn(
        &self,
        workspace: &Path,
        agent_id: &str,
        child_assistant_id: &str,
        parts: &[ChatPart],
        outcome: &TurnOutcome,
        usage: Usage,
        report: Option<&ReportPayload>,   // { body, files_changed, duration_ms }
    ) -> Result<()>;

    /// 列表：一条 SQL 联查元数据（decision 28）。
    pub fn list_subagents(&self, workspace: &Path, parent_session: &str) -> Result<Vec<SubagentRecord>>;

    /// 后台批次：一条 user 报告消息（`subagent_report_json` 数组）+ 同轮 assistant，
    /// 并在事务内把涉及的 agent 标记 `report_delivered=1`（§5.5）。
    pub fn begin_report_batch(
        &self,
        workspace: &Path,
        session_id: &str,
        envelope: &str,
        reports_json: &str,
        agent_ids: &[String],
        model: &ModelSnapshot,
    ) -> Result<TurnCommitted>;

    /// 恰好一次：0 → 1 时返回 true；已投递返回 false（幂等）。
    pub fn mark_report_delivered(&self, workspace: &Path, agent_id: &str) -> Result<bool>;

    /// 启动恢复：running/pending → cancelled，子会话 streaming → interrupted。
    pub fn recover_running_subagents(&self) -> Result<usize>;
}
```

`list_subagents` 核心 SQL：

```sql
SELECT a.external_id, a.child_session_id, a.parent_session_id, a.spawn_message_id,
       a.role, a.status, s.title, a.created_at_ms, a.completed_at_ms,
       a.report_json, a.report_delivered
FROM agent_subagent a
JOIN agent_session s ON s.external_id = a.child_session_id
WHERE a.parent_session_id = ?1
ORDER BY a.created_at_ms, a.id
```

### 4.4 已有函数的改动

| 位置 | 改动 |
|------|------|
| `src/db/chat/mod.rs:160` `list_chat_sessions` | SQL 加 `AND source = 'native'`（子会话不进 `/resume` 列表） |
| `src/db/chat/mod.rs:201` `load_chat_session` | **不改**：按 id 加载，子会话同样可读 |
| `src/db/chat/mod.rs:55` `ChatMessage` | 新增 `pub subagent_reports: Option<String>`（原始 `subagent_report_json`） |
| `src/db/chat/mod.rs:220` `load_chat_session` 的 SELECT | 加 `subagent_report_json` 列（索引 18） |
| `src/db/mod.rs:26` `SCHEMA_VERSION` | 15 → 16 |

## 5. 关键数据流

### 5.1 spawn

```text
主 agent 采样 → 响应含 spawn_agent{role,task}
  → run_agent_turn（agent_loop/mod.rs:560 的 ask 分支之后）识别控制工具
  → host.subagent(op).await（新增 AgentHostRequest::Subagent）
  → 事件循环 handle_subagent_op(Spawn):
      ① 并发检查：本会话 running 数 >= 3 → 回 LIMIT_REACHED_CODE（工具级失败，轮次继续）
      ② persistence.begin_subagent_turn → 子会话 + 两条消息 + agent_subagent 行
      ③ 注册 SubagentHandle（含 child_assistant_id / cancel / changed_paths）
         并在 Lx.agents 插入 SubagentRun（chat 由子 turn 增量填充）
      ④ tokio::spawn 子 turn（§5.2）
      ⑤ 立即回执 {agent_id, role}（不等待）
  → loop 正常 finish_tool 落库 → 委派卡片结算
```

`spawn_message_id` 沿用主轮同一来源 `active_assistant_id(state, &key)`（`event/chat/mod.rs:799`）。

### 5.2 子 turn

```rust
// src/event/chat/subagent.rs
let child_key = RequestKey { pane: parent.pane, session: child_session.clone(), generation };
let frozen = parent_frozen.clone();                    // decision 15
let reads = crate::chat::tools::read_registry(&child_session);   // 独立登记表
let turn = runtime.turn(&child_key.generation.to_string());
let executor = AgentTools::new(runtime.manager.clone(), turn)
    .with_recorder(Arc::new(file_snapshots::HostRecorderFactory::new(
        sender.clone(), db_workspace.clone(), child_session.clone(), user_message_id,
    )))
    .with_reads(reads);

let mut request = ChatRequest {
    turns: vec![ChatTurn { role: "user".into(), text: task.clone(), reasoning_metadata: None }],
    items: vec![ModelItem::UserText(task.clone())],
    model: frozen.snapshot(),
    variant_params: frozen.variant_params.clone(),
    purpose: RequestPurpose::Subagent(role),           // ← 决定工具面
    system_prompt: Some(compose_subagent_prompt(role, environment.as_deref())),
};
let context = ModelContext::new(request, cwd).with_runtime_summary(None);
let summary = host::drive_agent_turn(child_key, context, sender, executor, stream_one, cancel).await;
```

**不继承父历史**（decision 1/15）：只给角色提示 + 环境快照 + task。

子 turn 的系统提示**不含**主会话的后台任务摘要与 plan：环境快照由 spawn 时抓取的 `EnvironmentFacts` 现算，`background_tasks` 传空。

### 5.3 工具面裁剪（transport）

```rust
// src/chat/events.rs:168
pub enum RequestPurpose {
    #[default] Chat,
    CompactSummary,
    Subagent(AgentRole),          // 新增
}
impl RequestPurpose {
    /// 工具面：None = 不声明工具（摘要请求）。
    pub fn tools_surface(self) -> Option<ToolSurface> {
        match self {
            Self::Chat => Some(ToolSurface::Main),
            Self::Subagent(role) => Some(ToolSurface::from_role(role)),
            Self::CompactSummary => None,
        }
    }
    /// 保留旧名：等价于 `tools_surface().is_some()`。
    pub fn includes_tools(self) -> bool { self.tools_surface().is_some() }
}
```

四个 transport 全部改成按 surface 取表：

```rust
// src/chat/transport/tools/mod.rs:46 起
pub(crate) fn openai_chat_tools(surface: ToolSurface) -> Value { ... tools_for(surface) ... }
pub(crate) fn openai_responses_tools(surface: ToolSurface) -> Value { ... }
pub(crate) fn anthropic_tools(surface: ToolSurface) -> Value { ... }
pub(crate) fn google_tools(surface: ToolSurface) -> Value { ... }

// 各 transport 的 build：
if let Some(surface) = request.tools_surface() {
    body["tools"] = ..._tools(surface);
}
tools_requested: request.tools_surface().is_some(),   // transport/mod.rs:148
```

`schema.rs`：

```rust
// 8 项扩到 11 项（顺序即呈现顺序）
static TOOLS: [ToolSpec; 11] = [ /* 现有 8 项不变 */,
    spawn_agent_spec(), wait_agent_spec(), close_agent_spec() ];

/// 主工具视图（提示词/测试用；保持既有语义）。
pub fn available_tools() -> &'static [ToolSpec] { &TOOLS }

/// 按工具面过滤：顺序跟随 `TOOLS`，集合等于 `surface.names()`。
pub fn tools_for(surface: ToolSurface) -> Vec<&'static ToolSpec> {
    TOOLS.iter().filter(|spec| surface.names().contains(&spec.name)).collect()
}
```

`types.rs`：`TOOL_NAMES` 保持 8 项（既有提示与测试依赖），新增：

```rust
/// 三个 subagent 控制工具（顺序即 `TOOLS` 中的顺序）。
pub const SUBAGENT_TOOL_NAMES: [&str; 3] = ["spawn_agent", "wait_agent", "close_agent"];
```

控制工具 schema（`src/chat/tools/subagent.rs`，新文件）：

```jsonc
// spawn_agent
{ "type":"object", "additionalProperties": false,
  "required":["role","task"],
  "properties": {
    "role": {"type":"string","enum":["explorer","executor"]},
    "task": {"type":"string","minLength":1,"maxLength":8000,
             "description":"Goal, boundary (files/modules owned by this agent), and deliverable."} } }

// wait_agent
{ "type":"object", "additionalProperties": false,
  "required":["targets"],
  "properties": {
    "targets": {"type":"array","minItems":1,"maxItems":8,"items":{"type":"string"}},
    "timeout_ms": {"type":"integer","minimum":1,"maximum":3600000,"default":300000} } }

// close_agent
{ "type":"object", "additionalProperties": false,
  "required":["target"],
  "properties": { "target": {"type":"string","minLength":1} } }
```

### 5.4 增量路由与 checkpoint

```rust
// src/event/chat/mod.rs:1188 handle_net_event —— 最开头分流
if streams.subagents.contains_key(&key.session) {
    return handle_subagent_net_event(state, streams, persistence, key, event);
}
```

子分支：

```rust
ChatNetEvent::Delta { key, deltas } => {
    // ① 节流常量复用；counts 键沿用 key
    let bytes: usize = deltas.iter().map(ChatDelta::payload_bytes).sum();
    let entry = streams.checkpoints.entry(key.clone())
        .or_insert_with(|| (Instant::now(), 0));
    entry.1 += bytes;
    if entry.1 >= CHECKPOINT_BYTES || entry.0.elapsed() >= CHECKPOINT_INTERVAL {
        *entry = (Instant::now(), 0);
        // ② 直接落子会话：不用 schedule_checkpoint（它按窗格找消息）
        if let (Some(handle), Some(workspace)) =
            (streams.subagents.get(&key.session), streams.db_workspaces.get(&key))
        {
            let parts = update::subagent_parts(state, &key.session, &handle.child_assistant_id);
            state.effects.push(Effect::CheckpointChatTurn {
                workspace: workspace.clone(),
                assistant_id: handle.child_assistant_id.clone(),
                parts,
            });
        }
    }
    // ③ UI：只写 Lx.agents，绝不进 on_stream_delta
    update::on_subagent_delta(state, &key.session, &deltas)
}
ChatNetEvent::Finished { key, reason, usage } => {
    finish_subagent_turn(state, streams, persistence, &key.session, reason, usage)
}
```

### 5.5 报告生成、落库与投递

报告在子 turn 结束后产生。**后台主动汇报**与 **wait 消费**是两条互斥路径：

```text
① 子 turn 终态（两路共用）
   finish_subagent_turn：子 assistant 终态 + usage + agent_subagent.status/report_json
   不立即写父消息；只更新 Agents 面板与句柄终态。

② wait 消费（前台等待）
   settle_wait：读 report_delivered=0 的 report_json，mark_report_delivered
   → 报告正文随 wait_agent 工具结果返回模型
   → 主历史不插报告消息；正文只在 wait 工具卡展开态显示。

③ 后台主动汇报（自动轮）
   finish_subagent / 本轮 on_finish_ack 后调用 drain_reports：
     - 仅父窗格 RequestState::Idle、queue 为空；
     - 取全部 report_delivered=0 的报告（一条或多条）；
     - begin_report_batch：一条 user 报告消息（subagent_report_json 数组）
       + 同轮 streaming assistant，并在同一事务标记 report_delivered=1；
     - 报告卡在该轮 assistant 之前显示；模型看到的 envelope 是本轮 user 正文。
   父窗格忙时报告留在 DB，等父轮结束/用户轮结束后再 drain；用户输入优先。
```

**为什么批量合并**：同一个 idle 窗口内完成的多个子 agent 只产生一条报告消息 + 一个 assistant，避免报告各起一轮、loading 连续闪烁。批次提交即标记已投递：消息本身已经是可回放上下文，失败重试不应再生成第二份卡片；这与旧的“成功后才 mark”语义不同，换取的是不重复报告。

**用户输入优先**：`drain_reports` 在 `Idle`、无排队聊天提交时才启动；用户排队的聊天先执行，报告轮排在其后。本地 shell 不占 `RequestState`，可与报告轮并行（shell 结束不阻塞报告）。报告不再注入用户轮 `runtime_summary`（旧路径 B 已删除）。

**统计行**：统计行是任务收尾信息，不是每条中间 assistant 的用量条。`HistorySource.subagents_running` 为 true 时（主轮结束但子 agent 仍在跑，或报告自动轮期间）一律不画；下一条是可见报告消息时也不画（这轮只是报告链路中间态）。最后一个子 agent/报告结束后的 assistant 才显示。旧 wait 残留报告行已隐藏，不再挡住宿主统计。`wait_agent` 运行中主轮仍是 streaming，输入框上方 loading 照常显示；主轮自然结束后后台子 agent 仍撑等待 loading，只有用户显式停止主轮后才隐藏，后者改由上下文占用右侧的运行指标表达。

**旧历史**：旧版 wait 残留的 user-only 报告消息（同轮无 assistant）不在主时间线渲染，也不被 `plan_turns` 回放；不删除 DB 行。

### 5.6 工具握手（子会话自证）

```rust
// src/event/chat/tool_calls.rs:14 begin_tool —— 最前面分流
if let Some(handle) = streams.subagents.get(&key.session) {
    let Some(workspace) = streams.db_workspaces.get(&key).cloned() else {
        let _ = reply.send(Err(HostError::Cancelled));
        return false;
    };
    // 不查 AppState 窗格（decision 14）：子会话跨窗格存活。
    return begin_child_tool(state, streams, persistence, key, handle, workspace, call, reply);
}
```

子分支与主分支的唯一差别：

| 环节 | 主轮 | 子 turn |
|------|------|--------|
| 归属校验 | `active_assistant_id(state, key)` | `handle.child_assistant_id`（spawn 时落库） |
| 工作区 | `streams.db_workspaces[key]` | 同（子 key 单独登记） |
| 时间线对齐 | `tool_step_position` / `parts_before`（读窗格消息） | 读 `Lx.agents[child].chat`（未加载时跳过——DB 分配 `MAX(position)+1`） |
| UI pending/running | `update::on_tool_pending/running` | `update::on_subagent_tool_pending/running`（写 `run.chat`） |
| 结算 | `update::on_tool_settled` + `settle_tool_detail` | 同名子会话版本 |
| 进程校正 | `streams.runtimes.get(key.session)` | 同（子会话有自己的 runtime） |
| 计划提交 | `commit_plan(pane, ..)` | **不提交**（executor 无 `update_plan`；防御性忽略） |
| mutation 路径统计 | — | `handle.changed_paths.insert(target_path)` |

`FinishTool` 同理：`finish_child_tool` 先落库再更新 `run.chat`，回执 `Ok`。

### 5.7 wait / close

```text
wait：
  ① 校验 targets（1..=8、去重、全部属于本会话且父会话匹配）
  ② 取每个 handle 的完成 watch 接收端 + 单一 sleep(timeout)
  ③ tokio::select! 等「全部完成」或「超时」
  ④ 已终结且 report_delivered=0 的：正文塞进 AgentStatusView.report，mark_report_delivered
  ⑤ 回执 { timed_out, agents }；超时不写 error_text

close：
  ① handle.cancel.send(true)
  ② 子 turn 在下一个检查点返回 Interrupted → 子 assistant 标 interrupted
  ③ agent_subagent.status = 'cancelled'，**不生成 report_json、不生成报告行**
  ④ 从 streams.subagents 移除 → runtime 由下一次 sweep 回收
```

### 5.8 重启恢复

```rust
// src/event/chat/rewind.rs::recover_startup 里追加
if let Some(database) = persistence.database() {
    // 幂等；不重跑（decision 2）
    let _ = database.recover_running_subagents();
}
```

SQL：

```sql
UPDATE agent_subagent
   SET status = 'cancelled', completed_at_ms = ?now
 WHERE status IN ('pending', 'running');

UPDATE agent_message SET status = 'interrupted'
 WHERE status = 'streaming'
   AND session_id IN (SELECT child_session_id FROM agent_subagent);
```

不 touch `report_delivered`：恢复后仍可投递（下次用户消息那一轮）。

### 5.9 生命周期与回收

| 事件 | 行为 |
|------|------|
| 主 agent Esc | `streams.cancel(&parent_key)` 只取消主 key 与正在等待的 wait 回执；所有 running 子 handle 原样（decision 23），由 Agents 面板 `^d` 按选中项停止 |
| Agents 面板 `^d` | 只取消当前高亮的 running 子 handle；其他 running 子 agent 继续；partial transcript 留在面板；不自动起报告轮，下一次用户消息唤醒主 agent |
| 关窗格 / 切会话 | 子 agent **继续跑**（decision 1/14）；`Lx.agents` 随窗格状态消失，DB 仍写 |
| 会话删除 | DB 外键级联；事件层额外：`streams.subagents` 中父会话命中的 handle 发 cancel、`Lx.agents` 清空 |
| 子 turn 结束 | `streams.finish(&child_key)` → 从 `subagents`/`frozen`/`subagent_inputs` 移除；runtime 由 sweep 回收 |
| `reap_orphan_runtimes` | `bound` 白名单 = 窗格会话 ∪ `streams.subagents` 中 `status == running` 的会话（decision 13） |
| 退出 lx | 既有退出路径回收全部 runtime；子 agent 随进程结束 |

`reap_orphan_runtimes` 改动（`event/chat/mod.rs:752`）：

```rust
let mut bound: Vec<String> = state.all_pane_ids().into_iter()
    .filter_map(|pane| state.pane_anywhere(pane))
    .filter_map(|pane| pane.lx.chat.session_id.clone())
    .collect();
// 运行中的子会话同样「有主」：否则它的一轮工具进程会被下一次 sweep 杀掉。
bound.extend(
    streams.subagents.iter()
        .filter(|(_, handle)| handle.status == STATUS_RUNNING)
        .map(|(session, _)| session.clone()),
);
```

### 5.10 运行上下文摘要

```rust
// src/event/chat/mod.rs::runtime_context_summary —— 追加一段（decision 30）
if !running_subagents.is_empty() {
    lines.push("[local runtime] running subagents (use wait_agent / close_agent):".into());
    for (agent_id, role, task_head) in running_subagents.iter().take(MAX_CONCURRENT_SUBAGENTS) {
        lines.push(format!("- agent_id={agent_id} role={role} · {task_head}"));
    }
}
```

`start_stream` **不再**把 pending 报告塞进用户轮运行时摘要；报告由 idle 时的 `DrainSubagentReports` 独立起轮（§5.5）。

## 6. 运行态与展示态

### 6.1 事件层（`src/event/chat/subagent.rs`，新文件）

```rust
pub const STATUS_RUNNING: &str = "running";

/// 一个在跑的子 agent 句柄（只存在于事件循环）。
pub(crate) struct SubagentHandle {
    /// 父请求键（窗格 + 父会话 + 代次）：报告投递路由与 UI 归属。
    pub parent: RequestKey,
    pub agent_id: String,
    pub child_session_id: String,
    /// spawn 落库的 assistant 行：子 turn 所有工具归属校验的锚点。
    pub child_assistant_id: String,
    pub role: AgentRole,
    pub task: String,
    /// 取消通道；close / 会话删除 / 退出时发送。
    pub cancel: watch::Sender<bool>,
    /// 完成通知：wait_agent 用。
    pub done: watch::Sender<bool>,
    /// running / completed / failed / cancelled
    pub status: &'static str,
    pub started_at_ms: i64,
    /// 去重后的改动路径（decision 19）。
    pub changed_paths: std::collections::HashSet<PathBuf>,
    /// 结算后的报告正文（供 wait_agent 领取）。
    pub report: Option<String>,
}
```

`ChatStreams` 新增（`event/chat/mod.rs:99`）：

```rust
/// 子 agent 句柄：`child_session_id -> handle`。
pub(crate) subagents: HashMap<String, SubagentHandle>,

/// spawn 时冻结的模型（decision 15 的唯一来源）。
pub(crate) frozen: HashMap<String, crate::app::state::FrozenModel>,
```

`ChatStreams::finish(&key)` 追加：

```rust
// 子轮结束：句柄与冻结模型一起移除；主轮 key 不受影响（session 不同）。
if self.subagents.remove(&key.session).is_some() {
    self.frozen.remove(&key.session);
}
```

子 turn 的 `cancel` 通道也要进 `streams.cancels`（Esc 走不到它，但 `finish` 的统一清理依赖它）；**不要**放进 `pending_guards`（session guard 只对主会话有意义；子会话是新建的本地行，仍需 `try_lock_session(child_session)`，见 §11 风险表）。

### 6.2 app 层（`src/app/state/lx/agent.rs`，新文件）

```rust
/// 一个 subagent 的展示态。
pub struct SubagentRun {
    pub agent_id: String,
    pub child_session_id: String,
    pub role: AgentRole,
    pub task: String,
    /// running / completed / failed / cancelled
    pub status: String,
    pub started_at_ms: i64,
    pub completed_at_ms: Option<i64>,
    /// 完整消息模型（decision 5/18）：复用 `LxChat` 的字段与增量方法。
    pub chat: LxChat,
    /// 子会话无 compact，恒为空（`HistorySource` 需要）。
    pub compact: LxCompact,
    /// 消息流是否已从 DB 懒加载（历史 run）。
    pub loaded: bool,
    /// 最终报告（完成后才有）。
    pub report: Option<SubagentReportView>,
    pub input_tokens: Option<i64>,
    pub output_tokens: Option<i64>,
    /// 详情区滚动状态（decision 18：与 Processes 的 output_scroll 分开）。
    pub scroll: usize,
    pub visible: usize,
    pub follow: bool,
}

/// 汇报块数据源（父消息 `subagent_report_json` 反序列化）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SubagentReportView {
    pub agent_id: String,
    pub role: AgentRole,
    pub status: String,
    pub files_changed: usize,
    pub duration_ms: Option<i64>,
    pub body: String,
    pub delivered_at_ms: i64,
}

/// 面板数据。
#[derive(Debug, Default)]
pub struct AgentsState {
    pub runs: Vec<SubagentRun>,
}
```

挂载：`Lx` 新增 `pub agents: AgentsState`（`app/state/lx.rs:58`，`build()` 里 `AgentsState::default()`）。会话复位时清空：

```rust
// LxChat::reset_session_view 的调用点（会话切换/新建）同时清 Lx.agents
pub fn clear_agents(&mut self) { self.agents = AgentsState::default(); }
```

`TaskPage`（`app/state/lx/task.rs`）新增：

```rust
/// Agents 页签的窗口状态（与进程列表的 `window` 分开，decision 18）。
pub agents_window: PanelWindow,
/// 选中 subagent 的 agent_id（一次性定位 + 高亮）。
pub selected_agent: Option<String>,
/// Agents 页签当前显示的消息流行几何（渲染回写，翻页/滚动条用）。
pub agents_scroll: usize,
pub agents_visible: usize,
pub agents_follow: bool,
```

`len()` / `selected()` / `move_active()` 改为按当前 tab 取集合：

```rust
pub fn len(&self, agents: &AgentsState) -> usize {
    match self.tab {
        TaskTab::Processes => self.processes.len(),
        TaskTab::Agents => agents.runs.len(),
    }
}
```

> **这是本次唯一会改动 `TaskPage` 现有测试签名的点**：`len/selected/move_active/clamp` 三个方法多一个 `&AgentsState` 参数。选择把 `AgentsState` 留在 `Lx` 而不是塞进 `TaskPage`，是为了让面板关闭时 subagent 数据依然存在（decision 12：面板只是视图）。

### 6.3 app 更新层（`src/app/update/chat/subagent.rs`，新文件）

```rust
/// 子会话增量 → 对应 run.chat（按 child_session_id 定位，找不到即忽略）。
pub fn on_subagent_delta(state: &mut AppState, child_session: &str, deltas: &[ChatDelta]) -> bool;

/// 子 turn 工具状态（pending/running/settled），写 run.chat 的步骤时间线。
pub fn on_subagent_tool_pending(state: &mut AppState, child_session: &str, call_id: &str) -> bool;
pub fn on_subagent_tool_running(state: &mut AppState, child_session: &str, call_id: &str) -> bool;
pub fn on_subagent_tool_settled(
    state: &mut AppState, child_session: &str, call_id: &str,
    status: ToolStatus, summary: Option<ToolSummary>,
    result_json: Option<String>, error: Option<String>,
    detail: Option<ToolDetailView>,
) -> bool;

/// 列表元数据回投（幂等：内容相同返回 false）。
pub fn set_subagent_list(state: &mut AppState, pane: PaneId, parent_session: &str, records: &[SubagentRecord]) -> bool;

/// 懒加载回投：把历史消息流填进对应 run（保留已有 live 数据：只在 `!loaded` 时覆盖）。
pub fn on_subagent_loaded(state: &mut AppState, pane: PaneId, agent_id: &str, messages: &[ChatMessage]) -> bool;

/// 子 turn 终态 + 报告 → run 状态与 report。
pub fn on_subagent_finished(
    state: &mut AppState, child_session: &str, status: &str,
    report: Option<SubagentReportView>, usage: Usage,
) -> bool;

/// 后台报告批次已提交：报告消息（多卡）+ 同轮 streaming assistant。
pub fn on_report_batch_committed(
    state: &mut AppState, pane: PaneId, committed: &TurnCommitted,
    reports: Vec<SubagentReportView>,
) -> bool;

/// 详情区滚动（滚轮/PageUp/PageDown），follow 语义对齐主历史。
pub fn scroll_agents_detail(state: &mut AppState, pane: PaneId, delta: isize) -> bool;
/// 高亮切换（↑/↓），沿 `PanelWindow` 语义。
pub fn move_selected_agent(state: &mut AppState, pane: PaneId, delta: isize) -> bool;
/// Agents 面板 `^d`：强停当前高亮的 running 子 agent → `Effect::CancelSubagents { agent_id: Some(..) }`。
pub fn stop_selected_subagent(state: &mut AppState, pane: PaneId) -> bool;
```

### 6.4 新效应（`src/app/effects.rs`）

```rust
/// 列出父会话的 subagent 元数据（打开 Agents 页签 / 刷新）。
ListSubagents { pane: crate::layout::PaneId, parent_session: String },
/// 懒加载某个 subagent 的完整消息流。
LoadSubagentSession { pane: crate::layout::PaneId, agent_id: String, child_session: String },
/// 取消子 turn：`agent_id` 为 Some 时只停选中的那个，None 时停全部（会话删除）。
CancelSubagents { session_id: String, agent_id: Option<String> },
```

三者在 `db/service.rs` 的 `execute` 与 `is_network_effect` 里都按「事件层效应」处理（与 `ListSessionProcesses` / `StopSessionProcess` 同款：泵里 `warn!` + `false`，`is_network_effect` 返回 true）。`ListSubagents` 与 `LoadSubagentSession` 由事件层直接查库回投（不经过 `Effect` 的持久化泵）。

## 7. 提示词

### 7.1 主 agent 新增 `# Subagents` 节

插入 `LX_SYSTEM_PROMPT`（`chat/prompt.rs:49`）的 `# Tools` 之前；`PROMPT_VERSION` `lx-4` → `lx-5`。

```text
# Subagents

Three tools let you hand work to a subagent: `spawn_agent`, `wait_agent`, `close_agent`.
Each subagent starts with a clean context. It sees only its role prompt, the environment
snapshot, and the task you write — not this conversation.

- `explorer` — read-only investigation. It has `read` and `search` and nothing else. Use it
  for specific questions about the codebase: where a flow lives, who calls something, how a
  module is wired.
- `executor` — implementation with a boundary. It has the file and process tools, no `ask`,
  no plan tool, and no ability to spawn agents. Use it for one well-defined change or fix.

Delegate when the work is independent, has a clear boundary, and would otherwise fill your
context with detail you do not need. Do not delegate a single-file change you can make in a
couple of calls, anything that needs the user's input, or work you cannot describe precisely.

Write the task with three things: the goal, the boundary (which files or modules belong to
this agent), and the deliverable. A subagent that has to guess will guess wrong.

Rules:
- At most 3 subagents may run at once per conversation; a fourth call is rejected.
- Subagents run in the same workspace and can see each other's edits. Always tell each one it
  is not alone in the codebase and must not revert or overwrite changes it did not make.
- You do not have to wait. Start a subagent, keep working, and call `wait_agent` when you need
  the result — or let the report arrive on its own.
- A report is short by design: the conclusion plus what it touched. The full trace stays in its
  own session; do not ask for a replay.
- `close_agent` stops a subagent and produces no report.
```

删除与「自由活动」冲突的 codex 条款（不引入）：`wait for them before yielding`、`your only role becomes to coordinate`、`close_agent 收尾`。

### 7.2 `explorer` 角色提示（`chat/agent.rs::EXPLORER_PROMPT`）

```text
You are an explorer: a read-only investigator working inside the user's workspace.

You answer one specific question about the codebase. You have `read` and `search` and nothing
else — you cannot modify files, run commands, or spawn agents.

# How you work

- Start from the question, not from the repository. Work out which paths answer it, then read them.
- Follow the code, not the docs: a comment can be stale, the call graph cannot.
- Cite evidence as `path:line`. A claim without a location is a guess; say so if you cannot find one.
- Stop when the question is answered. Do not audit unrelated code, do not suggest refactors, and
  do not write patches — the caller decides what to do with your findings.
- If the question cannot be answered with the tools you have (it needs a command, a test run, or a
  file change), say exactly that and what would be needed. Do not pretend you checked.

# Final message

Your final message is the report and the only thing the caller reads. Lead with the answer, then
the evidence, then anything the caller must know to act on it. Keep it under roughly 40 lines.
State explicitly what you could not verify.
```

### 7.3 `executor` 角色提示（`chat/agent.rs::EXECUTOR_PROMPT`）

```text
You are an executor: a coding agent working inside the user's workspace on one assigned task.

# Your boundary

You were given a goal, a boundary, and a deliverable. Work inside that boundary. If the task turns
out to need changes outside it, stop and report why instead of widening the scope yourself.

You are not alone in this workspace. Other agents may be editing other files at the same time.
Never revert, reformat, or overwrite changes you did not make; if a file changed under you, re-read
it and work with the current content.

# How you work

- Read before you change: `edit` and `write` reject a file you have not read in this session or
  that changed on disk after the read.
- Prefer the smallest exact `edit` over rewriting a file. Do not refactor, rename, or reformat
  anything the task did not ask for.
- Fix the root cause instead of the symptom.
- Verify with the narrowest check that covers what you changed, using the project's own commands.
  Report exactly what you ran and what it returned.
- Do not commit, amend, create branches, or run destructive git commands.
- You cannot ask the user anything. If a decision genuinely blocks you, do the safe thing, finish
  what is unambiguous, and report the open question.

# Final message

Your final message is the report and the only thing the caller reads. State what changed (with
paths), what you verified (with commands and results), and what remains open. Never claim a check
passed that you did not run.
```

### 7.4 组装

```rust
/// 子 agent 系统提示：角色提示 + 环境快照（同格式、**不含**父会话的后台任务与 plan）。
pub fn compose_subagent_prompt(role: AgentRole, environment: Option<&str>) -> String {
    format!("{}\n\n{}\n", role.system_prompt(), environment.unwrap_or_default())
}
```

**预算**：当前 `LX_SYSTEM_PROMPT` 7 784 字节，新增 `# Subagents` 节约 1.6 KB，仍在 12 288 字节上限内（`prompt_stays_small` 测试）。`# Tools` 段同步改成 11 个工具名（现有测试逐名断言，会强制提醒）。

> **注意 `SystemPromptParts::split` 的 1900 UTF-16 单位首段上限**（`transport/mod.rs:35`）：新增内容在 `# Tools` 之前，可能把首段边界推进到新节中间。这是既有机制（尾段自动包成 `<system-instructions>` user 消息），**不需要改动**，但测试要断言「首段仍以 `You are lx,` 开头」（既有测试已锁）。

## 8. UI

### 8.1 `HistorySource<'a>`（渲染复用，decision 5）

**现状**：`snapshot::build(width, lx: &Lx, cache)`、`render.rs`、`paint.rs`、`sticky.rs` 都直接吃 `&Lx`；`history.rs::view(area, lx, cache)` 也是。实测渲染只用 `lx.chat`（120 处）与 `lx.compact`（20 处），`lx.task`/`lx.rewind` 只出现在测试里。

**做法**（不动 `HistoryView` 这个名字）：

```rust
// src/ui/main_content/lx/history/source.rs（新文件）
/// 历史渲染的只读数据源：主窗格从 `&Lx` 构造，子 agent 从 `&SubagentRun` 构造。
///
/// 渲染层只读：不克隆 `LxChat`、不访问 DB、不做 IO。
#[derive(Debug, Clone, Copy)]
pub(crate) struct HistorySource<'a> {
    pub messages: &'a [ChatMessageView],
    pub notices: &'a [RewindNoticeView],
    pub checkpoints: &'a [CompactNoticeView],
    /// 折叠状态 / loading 相位（子 agent 用自己的 `LxChat`）。
    pub chat: &'a LxChat,
    pub compact: &'a LxCompact,
}

impl<'a> From<&'a Lx> for HistorySource<'a> {
    fn from(lx: &'a Lx) -> Self {
        Self {
            messages: &lx.chat.messages,
            notices: &lx.chat.notices,
            checkpoints: &lx.compact.checkpoints,
            chat: &lx.chat,
            compact: &lx.compact,
        }
    }
}

impl<'a> HistorySource<'a> {
    /// 只有一份 `LxChat` 的数据源（`Agents` 详情区）。
    pub(crate) fn from_chat(chat: &'a LxChat, compact: &'a LxCompact) -> Self {
        Self { messages: &chat.messages, notices: &chat.notices, checkpoints: &compact.checkpoints, chat, compact }
    }
}
```

调用点改造（**只改签名，不改逻辑**）：

| 位置 | 改动 |
|------|------|
| `history/snapshot.rs:104` | `build(width: u16, source: HistorySource<'_>, cache)`；内部 `lx.chat` → `source.chat`、`lx.compact` → `source.compact`、`lx.chat.messages[i]` → `source.messages[i]` |
| `history/snapshot.rs:290` `Builder::message` | 同上（接收 `source`） |
| `history.rs:285` `view(area, lx, cache)` | 加 `source: HistorySource<'_>` 参数（第 2 位），内部同上 |
| `history.rs:283` `has_history(lx)` | 改成 `has_history(source)`；`timeline::has_history` 同步 |
| `history/render.rs:19` `render(grid, rect, lx, scroll, cache)` | 第 3 参改 `source` |
| `history/paint.rs:119` `BlockPaint::draw(..., lx, ...)` | 改 `source` |
| `history/sticky.rs` | 同上 |
| `history/cache.rs:314` `list_signature(lx, width)` | 改 `list_signature(source, width)`；新增 `reports` 签名字段 |
| `history/cache.rs:140` `PaneCache::snapshot(width, lx)` | 改 `snapshot(width, source)` |
| `history_hit.rs` 的 4 处调用 | 传 `HistorySource::from(lx)` |
| `ui/main_content/lx.rs:924` `render_history_for_test` | **签名不变**，内部 `HistorySource::from(lx)` |

**等价性保证**：先提交「只改签名」这一步，`cargo test` 全绿（既有 40+ 渲染测试零改动）；再接 `Agents` 详情区。这样 `HistorySource` 的问题不会和 subagent 逻辑混在一次提交里。

### 8.2 委派卡片（`history/tool/subagent.rs`）

`ToolDetailView` 新增一个变体（`app/state/lx/chat/tool_view.rs:307`）：

```rust
pub enum ToolDetailView {
    Command(CommandView),
    Interaction(ProcessInteractionView),
    Plan(PlanSnapshotView),
    File(super::file_diff::FileDiffDetailView),
    Ask(AskDetailView),
    Subagent(SubagentDetailView),   // 新增
}

/// `spawn_agent` / `wait_agent` / `close_agent` 共用（按 `name` 取标题）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SubagentDetailView {
    pub role: Option<AgentRole>,
    pub task: Option<String>,       // 全文（展开显示）
    pub agent_id: Option<String>,
    pub targets: Vec<String>,
    pub timed_out: bool,
    pub agents: Vec<AgentStatusView>,
    /// spawn 展开态显示的最终报告（wait 的报告在 agents[].report）。
    pub report: Option<SubagentReportView>,
    pub closed_status: Option<String>,
}
```

渲染：

```text
■ Spawn agent
  └─ explorer · 查一下 compact 触发链上谁改了 revision
```

- 第 1 行标题按 `call.name`：`spawn_agent` → `Spawn agent`、`wait_agent` → `Wait agents`、`close_agent` → `Close agent`。
- `spawn_agent` 是“已派出”的动作：工具返回后方块与动词立即用完成绿，不跟随子 agent 的 running 状态黄色呼吸；只有子 agent 失败才转红。
- 第 2 行：`role · task 首行`（`spawn`）/ wait 条件（`wait`）/ `agent_id · previous_status`（`close`）。
- `wait_agent` 头部右侧显示等待停止参数而不是 `running/completed`：`all · until done`、`any · 5m`、`2 targets · all · until done`、`1 target · any · 5m`；`timed_out` 追加 `· timed out`，时间来自本次调用的 `timeout_ms`。
- wait 的目标行只显示静态绿/黄/红方块（完成/等待或取消/失败）与 `role · title`，不再显示 `completed`/`waiting`/`failed` 文案，也不参与呼吸动画。
- 折叠态只显示目标/摘要行；目标行不显示 `status` 文字，只用绿/黄/红静态方块表达；展开显示 `task` 全文、`agent_id`、`targets` 与报告正文。
- `wait_agent` 展开态在目标列表之后按 agent 追加 `Report · role · title` + 报告正文；`spawn_agent` 展开态同样显示最终报告。`wait` 消费的报告因此不需要另插主历史消息。
- `rows()` 分派加一支（`history/tool.rs:107`）；`draw_animated` 加一支（`:182`）。
- `cache.rs` 的 `tool_signature` 加该变体的字段（`role`/`task`/`agent_id`/`targets`/`wait_for`/`timeout_ms`/`timed_out`/`agents` 的 report 正文/`report`）。
- 文案常量进 `ui/text.rs`：`LX_TOOL_SPAWN_AGENT` / `LX_TOOL_WAIT_AGENTS` / `LX_TOOL_CLOSE_AGENT`。

### 8.3 汇报块（`history/report.rs`）

```text
▲ Subagent report · completed
  └─ explorer · compact 的触发点
     report body preview line 1
     report body preview line 2
     … 8 more lines
```

- **不是主 agent 的工具调用**：它是子 agent 主动汇报块，`StepKind::Report` → 上三角 `▲`；颜色沿用工具档（完成绿、失败红、取消角色色），仅字形与工具方块区分。
- 数据来自 `ChatMessageView.reports`；`snapshot.rs::message()` 里，若 `reports` 非空：
  - **不画用户气泡**（跳过 `is_user` 分支），逐条 render 汇报块；
  - `is_user` 仍记 `true`？→ **否**：改为 `messages` 行的 `is_user = role == "user" && shell.is_none() && reports.is_empty()`，使置顶行/用户气泡逻辑自然跳过它。
- 头部固定 2 行（标题 + `└─ agent名 · title`）；折叠正文预览 2 行，展开用 `paint::draw_markdown` 渲染正文；**每次展开固定追加 24 行**，仍有剩余时末行保留 `… N more lines`，点击该提示行再追加一批，直到全部显示；点正文其他位置收起整块。
- 展开状态复用 `LxChat::detail_expanded`，槽位用 `format!("report:{agent_id}")`，`CardSection::Body`。
- 一条消息多报告 → 多个块，块间由 `spacer()` 空一行（**只有 snapshot 累加空行**，绘制路径不另算）。
- 失败/取消：`failed` → `StepMark::Failed`（红）、`cancelled` → `StepMark::Cancelled`（角色色），字形仍是 `▲`。
- **顺序**：后台批次里报告卡固定在该轮 assistant **之前**；assistant 在任务收尾（无 running 子 agent）后显示统计行。wait 已消费的报告不进入主历史，只在 `wait_agent` 工具卡展开态显示正文。
- **统计行**：只在 `subagents_running == false` 且下一条不是可见报告时显示；主轮或报告中间轮在子 agent/报告链路未收尾时都不显示。
- 新增块键 `REPORT_SLOT_BLOCK: usize = usize::MAX - 5`（`cache.rs:43`）。

### 8.4 `Agents` 页签（`ui/main_content/lx/task/agents.rs`，新文件）

版式：**详情区占原消息列表矩形，列表占原输入框矩形**（与 `Processes` 完全同一套两段式）。

```text
（详情区 = 原历史区矩形；走 history::render）
● explorer · running · 12s · 查一下 compact 触发链
  └─ 正在读取 src/chat/context/mod.rs …
■ Read  src/chat/agent_loop/mod.rs · 120-260
…
──────────────────────────────────────────────
 Agents    Processes
❯ ● explorer   running    14s   查一下 compact 触发链
  ● executor   completed   3s   修掉 clippy 警告
 esc close   ↑↓ select   ^d stop   tabs ←→
```

- 列表行：`StepMark::glyph`（`Running` / `Done` / `Failed` / `Interrupted` / `Cancelled`）+ 角色 + 状态词 + 时长 + `task` 首行。
- 时长格式复用 `ProcessEntry::age_label` 的口径 → 抽成 `pub(crate) fn age_label(ms: i64) -> String`（`state/lx/task.rs`），`ProcessEntry::age_label` 转调它。
- 列表窗口沿用 `PanelWindow`（`window_start` 按 `page.processes.len()` → 改为按当前 tab 的 len）。
- 详情区：`HistorySource::from_chat(&run.chat, &run.compact)` → `history::view` / `render` / `scrollbar`。滚动条与主历史**同一个组件**（`ui/widgets/scrollbar.rs`）。
- 跟随：`run.follow = true` 时贴尾；用户上翻即停（与 `TaskPage::set_output` 的 `was_at_tail` 同语义）。可见行数由 `resolve_task_agents_visible`（事件循环在绘制前写入，类比 `resolve_task_output_visible`）维护。
- 键盘：`↑/↓` 切高亮（不循环，到顶再上一个退出面板 —— 沿用既有 `move_active` 语义）；`PageUp/PageDown` 翻详情区；`^d` → 只强停当前高亮的 running 子 agent（`Effect::CancelSubagents { agent_id: Some(..) }`）；`←/→` 切 tab（既有）。
- 键位提示：`LX_TASK_HINTS_AGENTS` 从 2 项扩到 4 项（`select ↑↓` / `stop ^d` / `tabs ←→` / `back esc`）。
- 文案常量（`ui/text.rs:766` 附近）：补 `LX_TASK_AGENT_STATUS_*`、`LX_TASK_AGENT_ROLE_*`、`LX_TASK_AGENTS_LOADING`、`LX_REPORT_*`。

### 8.5 空行与字形规则（不可违反）

1. 委派卡片是**工具调用** → `■`/`□`，由 `StepMark::glyph(StepKind::Tool, ..)` 选字形。
2. 汇报块是**子 agent 主动汇报** → `▲`/`△`，由 `StepKind::Report` 选字形；不与工具方块或正文圆点混用。
3. 同一条消息内任意两块之间（含合并文件工具组）空一行；空行**只由 `history::snapshot` 累加**，绘制路径不得另算。
4. 渲染层不得写死任何标记字符。

## 9. 不变量与边界

1. **渲染只读**：`ui/` 不新增对 DB / 进程 / `ChatStreams` 的访问；子 agent 数据只能来自 `AppState`。
2. **持久化只在事件层**：`run_agent_turn` 与 `AgentTools` 不直接开 DB 会话；spawn/报告投递全走 `AgentHost` 或 `Persistence`。
3. **报告恰好一次**：`report_delivered` 是唯一判据；`wait_agent` 与自动投递共享同一闸门。
4. **子会话隔离**：写工具用 `read_registry(child_session)`；父会话读过的文件不为子 agent 放行写入。
5. **禁止递归**：子工具面无 `spawn_agent`；即使模型伪造调用，`parse_control` 在子 turn 里返回 `UNAVAILABLE_CODE`（双保险）。
6. **Esc 不杀子 agent**：`cancel` 按 `RequestKey` 取消主轮与正在等待的 wait 回执；所有 running 子 handle 继续，由 Agents 面板 `^d` 精确停止。
7. **重启不重跑**：任何副作用都不能因为重启被二次执行。
8. **进程不误杀**：运行中子会话必须进 `reap_orphan_runtimes` 的 `bound` 白名单。
9. **无 `unwrap`/`expect`**：所有新代码用 `Result` + `?`；越界一律钳制。
10. **单文件 ≤1000 行**：`task.rs` 已 509 行，`Agents` 页签必须落 `task/agents.rs`；`chat/agent.rs` 超限按 `agent/{role,op,report}.rs` 拆。
11. **模型可见文本一律英文**；中文只进注释与 `ui/text.rs`。

## 10. 测试矩阵

### 10.1 领域层（`src/chat/agent.rs` 单测）

- `AgentRole::parse` 与 `as_str` 双向一致；未知值 `None`。
- 三个 `ToolSurface` 名单**逐字**断言（decision 7 的落地证据）。
- `parse_control`：合法 / 缺字段 / 多余字段 / 超长 task / 非法 role / 负 timeout / targets 超 8 项。
- `SubagentOutcome` 序列化 → 反序列化往返一致（历史恢复依赖）。

### 10.2 schema / transport

- `tools_for(Main)` 名称与顺序 == `TOOL_NAMES ++ SUBAGENT_TOOL_NAMES`。
- 四协议 × 三工具面：数量与名称逐字一致。
- `CompactSummary` 仍不带工具（既有测试保持）。
- explorer 请求体里**不存在** `edit`/`write`/`exec_command` 的 schema。
- 全部新 schema `additionalProperties: false`。

### 10.3 DB（`src/db/chat/subagent.rs` 单测 + `db/tests.rs`）

- v15 → v16 升级保留数据；`PRAGMA integrity_check` 通过；`column_exists` 守卫重复执行幂等。
- `begin_subagent_turn` 写 `source='subagent'`、`agent_role`、`agent_subagent` 行、spawn_message_id。
- `list_chat_sessions` 不返回 `source='subagent'`。
- `load_chat_session` 能按 id 加载子会话（含 `subagent_report_json`）。
- `begin_report_batch` 后：该轮 user/assistant 同 `turn_index`，`subagent_report_json` 为数组，`plan_turns` 能回放（断言组装结果含信封）。
- 批次提交即把参与 agent 标记 `report_delivered=1`；无同轮 assistant 的旧 user-only 报告行不被 `plan_turns` 回放、UI 也隐藏。
- token 不串：子会话 `total_*` 增加时父会话不变。
- `mark_report_delivered` 幂等（第二次返回 false，不改 `report_json`）。
- 级联：删父会话 → 子会话、`agent_subagent`、子消息全消失。
- `recover_running_subagents`：running → cancelled、streaming → interrupted、不重跑。

### 10.4 loop 与事件层

- 第 4 个 `spawn_agent` 返回 `LIMIT_REACHED_CODE`，**同一响应内其余调用照常执行**。
- `wait_agent` 超时 → `timed_out: true`、无 `error_text`、轮次继续。
- `close_agent` 后无报告、无汇报块。
- 恰好一次：先自动投递，再 `wait` → 第二次 `report = None`。
- 父轮结束于 `Finishing/Idle` 后才到达的报告：不会插在最终回答后，空闲时由 `DrainSubagentReports` 合并成一条大卡消息 + 一个 assistant。
- `wait` 消费报告：主历史不出现报告消息行，正文只在该 `wait_agent` 卡展开态。
- Esc 取消主轮后：`wait_agent` 回执立即取消；被等待的子 handle 仍在运行、未收到取消信号；所有 running 子 agent 都保留。
- Agents 面板 `^d`：仅当前高亮 running 子 handle 收到 cancel，其他 running 子 agent 继续；partial 保留，未自动起报告轮。
- 子 turn 的 `Delta` 不进主对话（断言主消息数不变、`Lx.agents` 对应 run 增长）。
- 子 turn 的工具握手在**窗格已关闭**时仍成功（`begin_tool` 子分支不查窗格）。
- 执行层兜底：向 explorer 的 run 注入 `edit` 调用 → `UNAVAILABLE_CODE`，无文件 IO。
- `reap_orphan_runtimes`：运行中子会话的 runtime 不被回收；子 turn 结束后下一次 sweep 回收。

### 10.5 app 层

- 两个 run 交错 delta 不串（按 `child_session_id` 定位）。
- `set_subagent_list` 幂等（内容不变返回 false）。
- `TaskPage::len/selected/move_active` 按 tab 取集合；切 tab 不丢进程列表高亮（既有测试扩一条）。
- `ChatMessageView.reports` 反序列化：空/损坏 JSON → 空 vec，不 panic。
- 会话复位清空 `Lx.agents`。

### 10.6 UI

- 委派卡片/汇报块在宽度 0 时行数为 0；宽字符、超长 task 不越界。
- 汇报块展开/折叠行数与 `rows()` 唯一来源一致。
- `HistorySource` 等价：抽取前后主窗格快照 `total_rows` 与块键一致（在抽取那一步单独提交）。
- `Agents` 列表窄宽度下整项丢弃（对齐 `draw_footer` 既有规则）。

### 10.7 交互（用户自行 `cargo run`，AGENTS.md 规定 Agent 不做）

派 explorer → 看委派卡片 → 打开 `Agents` 看实时流 → 主 agent 继续干活 → 汇报块出现 → wait 中 Esc 只停主轮、所有子 agent 继续 → Agents 面板 `^d` 强停当前选中 → 关窗格再回来（面板里仍能看到 partial）→ 重启后打开面板看到 `cancelled` 的完整历史。

## 11. 风险与回滚

| 风险 | 影响 | 缓解 |
|------|------|------|
| `HistorySource` 抽取波及渲染热路径 | 主界面渲染/缓存回归 | 抽取单独一次提交，只改签名；先跑全量渲染测试；`render_history_for_test` 签名不变 |
| 子 turn 与主 turn 共享 `AppEvent` | 增量误入主对话 | 分流判据只认 `key.session ∈ streams.subagents`；补「主消息数不变」断言 |
| `reap_orphan_runtimes` 误杀子会话进程 | 后台任务消失 | 运行中子会话进 `bound`；补单测 |
| 子会话的 session guard | 子 turn 若走 `try_lock_session` 失败会直接失败；但 `begin_subagent_turn` 已在本进程建行，另一实例不会持有该子会话 guard | 子 turn 启动时 `try_lock_session(child_session)`；失败按 spawn 失败回执（不半途死） |
| `begin_report_batch` 的 sequence/turn_index 分配 | 唯一约束冲突 / 轮次分组错乱 | 只在父窗格 `Idle` 且无排队提交时调用（此时不可能有轮次插入）；DB 测试锁「user 与 assistant 同 turn_index」 |
| v16 迁移 | 旧库升级失败 | 仅 `ADD COLUMN` + `column_exists`；`integrity_check` 测试 |
| 报告投递打断用户输入 | 自动起轮抢焦点 | 仅 `Idle` 且无排队提交；用户输入优先，报告延后 drain |
| 提示词膨胀 | 超 12 KiB 预算 | `prompt_stays_small` 测试兜底 |

回滚：删除 `docs/feature/lx-subagent` 与本设计新增文件即可；v16 三列对旧二进制无影响（旧版遇到更高 `user_version` 报 `UnsupportedSchema`，不会写坏库）。

## 12. 实施顺序

```text
1 领域层（chat/agent.rs）
   ├─> 2 schema（tools/subagent.rs + schema.rs + types.rs）
   │     └─> 3 transport 按工具面裁剪
   └─> 4 持久化 v16（migration + db/chat/subagent.rs + list_chat_sessions 过滤）
              └─> 5 loop + 事件层（spawn/turn/delta/handshake/wait/close/报告投递/恢复）
                        └─> 6 app 状态与更新
                                  ├─> 7 HistorySource 抽取（单独提交）─> 8 UI（卡片/汇报块/Agents 页签）
                                  └─> 9 提示词（可与 7 并行，但必须与 2 同一次提交落地）
                                                         └─> 10 收尾验证
```

理由：2/3 依赖 1 的 `ToolSurface`（没有它无法证明 explorer 真只读）；5 依赖 4 的 `begin_subagent_turn`；8 依赖 6 与 7；9 若早于 2 落地，提示会承诺不存在的工具。
