# 工具执行：只读调用并行化 + codex 工具面对齐审计（设计方案）

> 状态：**已实现并通过定向验证（branch `feat/agent-tool-parallel-readonly`，待用户确认合并）**
>
> 关联文档：
> - 任务清单：`docs/feature/agent-tool-parallel-readonly/task.md`
> - 代码规范：`docs/standards/code-writing-standards.md`
> - 参考实现：`/Users/yonah/projects/agent/codex-main/codex-rs/core/src/tools/`
>
> 结论先行：**本次不新增任何工具**；唯一实现改动是把“同一响应内连续的只读内置工具调用”改为最多 4 路并行，其余工具与执行机制逐项写明“不采纳 + 理由”。

---

## 1. 背景与目标

### 1.1 现状分析

lx 当前的同一响应工具执行是**严格串行**的：

- `src/chat/agent_loop/mod.rs:632` 明确写着“同一响应内**串行**执行，顺序即供应商 ordinal；前一结果对后一调用可见”；
- 每个调用依次走 `begin_tool`（DB pending + UI running）→ `execute_tracked`（文件 IO 在 `spawn_blocking`）→ `finish_tool`（结果落库 + 终态）→ `push_tool_result`；
- 模型一次响应里发 3~5 个 `read` / `search` 时，这些本来互不依赖的查询被逐个等待，`search` 单次超时上限 10 秒，最坏情况会叠加成几十秒。

对照 codex：

- `codex-rs/core/src/tools/parallel.rs` 的 `ToolCallRuntime` 用 `parallel_execution: Arc<RwLock<()>>` 区分读写：只读调用共享读锁并行，写调用拿写锁串行；
- 每个 handler 通过 `supports_parallel_tool_calls` 表态（例如 `get_context_remaining` 返回 `true`）；
- 执行结果与 tool call 通过 call ID 关联，不依赖完成顺序。

### 1.2 核心目标

1. 同一响应内**连续的**只读内置调用（`read` / `search` / `web_search` / `curr_time`）最多 4 路并行执行。
2. 执行可以乱序完成，但 **Begin / Finish / DB 落库 / 模型上下文顺序必须严格按 ordinal**。
3. 取消语义不退化：Esc 后已启动的调用跑完并如实落库；未启动的调用标 `cancelled`；不再采样。
4. 不改变工具面、工具描述、数据库 schema、UI 组件与配置项。

### 1.3 非目标

- 不新增 `tool_search`、`get_context_remaining`、`new_context`、`sleep`、`view_image`、`send_message_to_user`、Code Mode 等工具。
- 不并行 MCP 工具、进程工具、写工具、计划工具、提问工具与 subagent 控制工具。
- 不引入沙箱、审批、pre/post tool hooks、per-tool telemetry 等 codex 专有机制。
- 不改变 `edit` 的读后版本保护，也不放宽补丁匹配规则。
- 不做全局并发控制（跨会话/跨子 agent），只做单批次上限 4。

---

## 2. codex 工具面审计（逐项结论）

### 2.1 工具对照

| codex 工具 | codex 行为 | lx 现状 | 结论 |
| :--- | :--- | :--- | :--- |
| `apply_patch` | `*** Begin Patch` 多文件补丁，支持 Add/Update/Delete/Move/End of File | `edit`（`src/chat/tools/edit.rs`）已实现同一套补丁语法，另加 `read_versions` 读后版本保护 | **保留**，不重复新增 |
| `exec_command` / `write_stdin`（unified_exec） | 统一执行、PTY、长驻会话、增量读写 | `exec_command` / `write_stdin` 已支持 PTY（固定 120x40）、yield 窗口、跨轮 `session_id` | **保留** |
| `view_image` | 读取本地图片并以 `detail` 控制精度 | `read` 已支持 PNG/JPEG/GIF/WebP/BMP/TIFF，并把图片归档进会话 | **不新增**：语义已被 `read` 覆盖，新增只会让模型在两个入口间犹豫 |
| `update_plan` | 规范化计划快照、最多一个 `in_progress` | `update_plan` 完全同构，带修订号与落库 | **保留** |
| `request_user_input` | 阻塞提问、选项 + 自由输入 | `ask` 已支持 1-3 题、0-4 选项、多选、preview | **保留** |
| `multi_agents` / `multi_agents_v2` | 派生、发消息、等待、打断、恢复、关闭、列举 | lx 已有 8 个 subagent 控制工具（`spawn_agent`/`send_input`/`interrupt_agent`/`resume_agent`/`wait_agent`/`close_agent`/`list_agents`/`cancel_agent_input`） | **保留** |
| `mcp` / `mcp_resource` | MCP 工具调用与 resources 列举/读取 | lx 已有 MCP 工具动态声明 + `list_resources`/`list_resource_templates`/`read_resource` | **保留** |
| `current_time` | 当前时间 | `curr_time` | **保留** |
| `tool_search` | BM25 检索延迟暴露的工具，下一轮再声明 | lx 每轮全量声明 MCP 工具，没有延迟暴露层 | **不采纳**：需要把“工具面冻结 + 下一轮重新声明”整条链路改造，收益只在 MCP 工具数量极大时成立；当前没有证据 |
| `get_context_remaining` | 查询剩余 token | lx 在 90% 阈值自动压缩（`AUTO_COMPACT_PERCENT`），模型不需要主动查询 | **不采纳**：多一个工具只会增加每轮声明噪音，且压缩由 loop 自动完成 |
| `new_context` | 开启新上下文窗口，不摘要 | lx 的 `/compact` 与自动压缩都带检查点/摘要，模型主动清窗会破坏回放语义 | **不采纳** |
| `sleep` | 主动等待 | `exec_command` 的 yield 窗口与 `write_stdin` 已能等待长任务 | **不采纳**：纯空转，浪费一轮采样与用户时间 |
| `send_message_to_user_async` | 轮内异步通知用户 | lx 的轮内 assistant 正文本身就会实时显示 | **不采纳** |
| `request_permissions` | 申请沙箱升级权限 | lx 没有沙箱，也不向模型承诺沙箱 | **不采纳**：能力不存在，声明就是欺骗 |
| `extension_tools` / 插件安装 | 扩展工具与插件市场 | lx 没有插件系统 | **不采纳** |
| Code Mode（`exec` / `wait`） | 在 JS 沙箱里批量调用工具 | lx 没有 JS runtime，工具调用必须逐卡可见 | **不采纳**：与 TUI 工具卡可视化、mutation 记录、取消语义全部冲突 |
| `test_sync` / `wait_for_environment` | 测试专用 / 环境托管 | 不适用 | **不采纳** |

### 2.2 执行机制对照

| codex 机制 | lx 现状 | 结论 |
| :--- | :--- | :--- |
| 只读调用并行（`parallel.rs` 读锁） | 全部串行 | **采纳**（本设计唯一实现项） |
| 工具注册表 + 暴露模式（direct/deferred/code_mode/hidden） | `ToolSurface` 白名单 + 每轮冻结声明 | 不采纳：现有白名单已覆盖角色收紧，改造收益不足 |
| per-tool cancellation token | `watch` 取消 + Begin/Finish 票据 | 已具备；本次保持 |
| pre/post tool hooks | 无 | 不采纳：lx 没有 hook 运行时 |
| approvals / sandboxing | 无，且不承诺 | 不采纳 |
| 参数 diff 流式消费（UI 实时预览） | 已有 tool input 流式展示 + mutation 快照 | 已具备 |
| 输出截断/限额 | `format_for_model` 统一口径 + 读写硬上限 | 已比 codex 更集中，保持 |
| 执行耗时遥测 | 卡片状态行已记录 duration | 本次只加一条 batch 级 `tracing::debug!`，不新增指标系统 |

---

## 3. 当前实现事实（改哪里、为什么）

| 位置 | 事实 | 本次动作 |
| :--- | :--- | :--- |
| `src/chat/agent_loop/mod.rs:632-792` | 串行工具循环；Begin/Finish 与结果入上下文都在这里 | 插入只读批次分支；串行尾部抽成公共提交函数 |
| `src/chat/agent_loop/mod.rs:794` | `cancel_unstarted` 把未开始调用记 `cancelled` | 原样复用 |
| `src/chat/agent_loop/mod.rs:814` | `ready_calls` 已按 `ordinal` 排序，并校验参数是 JSON 对象 | 不改；批次天然按 ordinal 取连续段 |
| `src/chat/agent_loop/mod.rs:326` | `ToolExecutor`：`&self` + `Send + Sync`，可安全并发调用 | 不改 trait，直接用 `join_all` |
| `src/chat/agent_loop/mod.rs:487` | `tool_result_images` 把归档图片读成 `ModelImage` | 移入新子模块，供串行/并行共用 |
| `src/chat/tools/types.rs:10` | `TOOL_NAMES` 是 10 个内置工具名的唯一清单 | 新增 `PARALLEL_SAFE_TOOL_NAMES` 与判定函数 |
| `src/chat/tools/schema.rs:25` | `TOOLS` 是 18 个工具规格（10 内置 + 8 subagent） | 不改 |
| `src/chat/agent_loop/executor.rs:293` | 文件工具在 `spawn_blocking` 执行，读版本表 `ReadRegistry` 用 `Mutex` 保护 | 并发安全，不改 |
| `src/chat/agent_loop/tests.rs:282` | `RecordingExecutor` 可直接执行真实文件工具 | 新测试另加可观测并发度的 `ProbeExecutor` |

---

## 4. 方案设计

### 4.1 并行安全分类（唯一来源）

在 `src/chat/tools/types.rs` 增加常量，并在 `src/chat/tools/mod.rs` 重导出：

```rust
/// 同一响应内允许并行执行的内置只读工具（顺序仅用于可读性）。
///
/// 约束：
/// - 必须是 [`TOOL_NAMES`] 的子集（测试锁定）；
/// - 不得包含任何会产生文件/进程/会话副作用的工具；
/// - MCP 工具名带 `server__tool` 前缀，不会命中本表。
pub const PARALLEL_SAFE_TOOL_NAMES: [&str; 4] =
    ["read", "search", "web_search", "curr_time"];

/// 工具名是否允许进入并行只读批次。
pub fn is_parallel_safe(name: &str) -> bool {
    PARALLEL_SAFE_TOOL_NAMES.contains(&name)
}
```

为什么放在 `chat/tools` 而不是 loop：

- `TOOL_NAMES` 已经在这里，工具面的“事实”不应散落到编排层；
- 未来新增只读工具时，改一处并补一个成员测试即可；
- loop 只依赖 `is_parallel_safe(name)`，不重复维护匹配表。

**分类边界（重要）**：

| 工具 | 并行 | 原因 |
| :--- | :---: | :--- |
| `read` | ✅ | 只读；`ReadRegistry` 内部 `Mutex` |
| `search` | ✅ | 只读；单次 10s 超时、文件数/命中数有硬上限 |
| `web_search` | ✅ | 只读网络；自身超时 + 单次最多 4 个 query |
| `curr_time` | ✅ | 纯时钟读取 |
| `edit` / `write` | ❌ | 写磁盘，必须让后续调用看见结果 |
| `exec_command` / `write_stdin` | ❌ | 跨轮进程句柄、`TurnExecution.active_interaction` 是单值 |
| `update_plan` | ❌ | 修订号单调递增，不能并发分配 |
| `ask` | ❌ | 需要 `&mut host` 阻塞等待用户 |
| 8 个 subagent 控制工具 | ❌ | 池状态、并发上限、幂等去重键都要求串行决策 |
| MCP 工具 / resources | ❌ | 第一版不碰外部 server 的并发语义与独立取消 |

### 4.2 批次成形规则

规则只有两条：

1. **只合并相邻调用**：遇到任何非并行安全工具就切断批次，该工具按原串行路径执行，后续只读调用另起一批。
2. **单批最多 4 个**：`MAX_PARALLEL_TOOL_CALLS = 4`。

示例（同一响应，按 ordinal）：

```text
read(c0) read(c1) write(c2) search(c3) search(c4) read(c5)
  └──── batch A ────┘  serial     └──── batch B ────┘  batch C
```

为什么必须“相邻”而不是“把所有只读调用收集起来一起跑”：

- 语义要求“前一结果对后一调用可见”。`read A → edit A → read A` 中第二个 `read` 必须看到 `edit` 的结果；
- 如果把第二个 `read` 提前到 `edit` 之前执行，模型会拿到旧内容，随后基于旧版本写入，直接触发读后版本保护失败，甚至覆盖错误内容；
- 相邻批次是**不用引入依赖图**就能保证可见性的最简规则。

### 4.3 执行流程

```text
响应就绪（ready_calls 已按 ordinal 排序）
  │
  ├─ 取首个调用
  │    ├─ 非并行安全 → 原串行路径（Begin → 校验 → 执行 → Commit）
  │    └─ 并行安全   → 收集相邻只读调用，最多 4 个
  │
  ├─ 批次：逐个 host.begin_tool（拿到 ticket）
  │    ├─ 任一 begin 失败 → 已 begin 的全部按 Cancelled 结算 → 返回
  │    └─ begin 后 cancel 已置位 → 全部按 Cancelled 结算 → 返回 Interrupted
  │
  ├─ 逐个校验 allowed（本轮冻结工具面）
  │    └─ 不允许的调用不执行，直接生成 UNAVAILABLE 失败结果
  │
  ├─ join_all 并发执行（最多 4 个 future）
  │
  ├─ 按 ordinal 逐个 Commit：
  │    format_for_model → finish_tool(Executed) → push_tool_result
  │    └─ 任一 finish 失败 → 剩余 ticket 按 Cancelled 结算 → 返回
  │
  └─ 批次完成 → 检查 cancel → 未取消则继续下一个批次/串行调用
```

关键点：

- **执行顺序与结算顺序分离**：future 谁先完成都无所谓，结果按 `begun` 的原始顺序 zip 回来，再逐个 `finish_tool`，因此 DB 与 UI 看到的顺序仍是 ordinal。
- **begin 全部先做**：让所有调用都先进入 `running` 可见态，用户能看到“4 个读同时跑”，而不是等前一个结束才出现下一个卡片。
- **不允许的调用也走 Commit**：仍然 `begin → finish(Executed{failure})`，保持现有“失败结果也成对落库”的契约。

### 4.4 关键代码骨架

新增 `src/chat/agent_loop/tool_batch.rs`（预估 150~180 行，含注释）：

```rust
use futures::future::{BoxFuture, join_all};
use tokio::sync::watch;

use super::context::ModelContext;
use super::{AgentHost, HostError, ReadyToolCall, ToolExecutor, ToolOutcome};
// `tool_result_images` 由 `mod.rs` 移入本文件，紧挨 `commit_result`。
use crate::chat::agent::UNAVAILABLE_CODE;
use crate::chat::tools::{
    ToolErrorCode, ToolExecutionResult, ToolFailure, format_for_model,
};

/// 单批最多并行的只读调用数。
pub(super) const MAX_PARALLEL_TOOL_CALLS: usize = 4;

/// 批次执行结局：由调用方映射成 `TurnSummary`。
pub(super) enum BatchOutcome {
    Completed,
    Interrupted,
    Failed(String),
}

/// 执行一批相邻只读调用；`calls` 里的 `allowed` 已由调用方按冻结工具面判定。
pub(super) async fn execute_batch<H, E>(
    host: &mut H,
    context: &mut ModelContext,
    executor: &E,
    calls: Vec<(ReadyToolCall, bool)>,
    cancel: &watch::Receiver<bool>,
) -> BatchOutcome
where
    H: AgentHost,
    E: ToolExecutor,
{
    let cwd = context.cwd().to_path_buf();

    // 1) 全部先 begin：任何一个失败都要把已 begin 的票据结算掉，不能留 pending 行。
    let mut begun: Vec<(ReadyToolCall, crate::db::chat::ToolCallTicket, bool)> =
        Vec::with_capacity(calls.len());
    for (call, allowed) in calls {
        match host.begin_tool(call.clone()).await {
            Ok(ticket) => begun.push((call, ticket, allowed)),
            Err(error) => {
                cancel_tickets(host, begun.into_iter().map(|(_, ticket, _)| ticket)).await;
                return outcome_of(error);
            }
        }
    }
    if *cancel.borrow() {
        cancel_tickets(host, begun.into_iter().map(|(_, ticket, _)| ticket)).await;
        return BatchOutcome::Interrupted;
    }

    // 2) 并发执行；不允许的调用用 ready future 直接产出失败结果，绝不进 executor。
    //    允许的调用仍先走可取消入口：web_search 的 Esc 取消必须直达 HTTP 请求。
    let futures: Vec<BoxFuture<'_, ToolExecutionResult>> = begun
        .iter()
        .map(|(call, ticket, allowed)| {
            let future: BoxFuture<'_, ToolExecutionResult> = if *allowed {
                Box::pin(run_one(executor, &cwd, call, ticket, cancel.clone()))
            } else {
                Box::pin(std::future::ready(unavailable_result()))
            };
            future
        })
        .collect();
    let results = join_all(futures).await;

    // 3) 按 ordinal 顺序结算；finish 失败时把剩余票据按 Cancelled 收尾。
    let mut iter = begun.into_iter().zip(results);
    while let Some(((call, ticket, _), result)) = iter.next() {
        if result.error_code() == Some(ToolErrorCode::Cancelled) {
            let _ = host.finish_tool(ticket, ToolOutcome::Cancelled).await;
            cancel_tickets(host, iter.map(|((_, ticket, _), _)| ticket)).await;
            return BatchOutcome::Interrupted;
        }
        if let Err(error) =
            commit_result(host, context, &call, ticket, result, executor.supports_image_input())
                .await
        {
            cancel_tickets(host, iter.map(|((_, ticket, _), _)| ticket)).await;
            return outcome_of(error);
        }
    }
    BatchOutcome::Completed
}

/// 可取消入口优先（`web_search` / MCP），其余工具回落到带票据的通用执行入口。
async fn run_one<E: ToolExecutor>(
    executor: &E,
    cwd: &Path,
    call: &ReadyToolCall,
    ticket: &crate::db::chat::ToolCallTicket,
    cancel: watch::Receiver<bool>,
) -> ToolExecutionResult {
    match executor.execute_mcp_cancellable(cwd, call, cancel).await {
        Some(result) => result,
        None => executor.execute_tracked(cwd, call, ticket).await,
    }
}

/// 把 host 错误映射成批次结局：只有 `Cancelled` 代表用户中断。
fn outcome_of(error: HostError) -> BatchOutcome {
    match error {
        HostError::Cancelled => BatchOutcome::Interrupted,
        HostError::Failed { code } => BatchOutcome::Failed(code),
    }
}

/// 把未执行的票据按 cancelled 结算；错误只记 debug，不重试。
async fn cancel_tickets<H: AgentHost>(
    host: &mut H,
    tickets: impl Iterator<Item = crate::db::chat::ToolCallTicket>,
) {
    for ticket in tickets {
        if host
            .finish_tool(ticket, ToolOutcome::Cancelled)
            .await
            .is_err()
        {
            tracing::debug!(target: "lx::agent", "failed to settle cancelled tool ticket");
        }
    }
}

/// 工具面不允许的调用：与串行路径同一条失败结果。
fn unavailable_result() -> ToolExecutionResult {
    ToolFailure::new(ToolErrorCode::Io, UNAVAILABLE_CODE).into_result(Vec::new())
}
```

公共提交函数（串行路径也改用它，消除两份 `format → finish → push`）：

```rust
/// 把一次执行结果按固定口径提交：格式化 → 落库终态 → 进入模型上下文。
///
/// 提交失败直接返回 `HostError`，调用方必须停止本轮且不重试。
pub(super) async fn commit_result<H>(
    host: &mut H,
    context: &mut ModelContext,
    call: &ReadyToolCall,
    ticket: crate::db::chat::ToolCallTicket,
    result: ToolExecutionResult,
    supports_image_input: bool,
) -> Result<(), HostError>
where
    H: AgentHost,
{
    let mut content = format_for_model(&result);
    let is_error = result.is_failure();
    let mut images = tool_result_images(&result);
    if !supports_image_input && !images.is_empty() {
        images.clear();
        content.push_str("\n[image omitted: this model does not support image input]");
    }
    host.finish_tool(ticket, ToolOutcome::Executed(result)).await?;
    context.push_tool_result(&call.call_id, content, is_error, images);
    Ok(())
}
```

`mod.rs` 的主体只增加一个分支（其余串行逻辑保持原样）：

```rust
// 同一响应内：相邻只读调用并行（最多 4 个），其余调用仍串行。
let mut remaining = calls.into_iter().peekable();
while let Some(call) = remaining.next() {
    if *cancel.borrow() {
        cancel_unstarted(&mut host, std::iter::once(call).chain(remaining)).await;
        return TurnSummary::interrupted(usage).with_billing(billing);
    }

    if tools::is_parallel_safe(&call.name) {
        let mut batch = vec![call];
        while batch.len() < tool_batch::MAX_PARALLEL_TOOL_CALLS
            && remaining
                .peek()
                .is_some_and(|next| tools::is_parallel_safe(&next.name))
        {
            if let Some(next) = remaining.next() {
                batch.push(next);
            }
        }
        let batch = batch
            .into_iter()
            .map(|call| {
                let allowed = call_allowed(&context, &call.name);
                (call, allowed)
            })
            .collect();
        match tool_batch::execute_batch(&mut host, &mut context, &executor, batch, &cancel).await {
            BatchOutcome::Completed => {}
            BatchOutcome::Interrupted => {
                cancel_unstarted(&mut host, remaining).await;
                return TurnSummary::interrupted(usage).with_billing(billing);
            }
            BatchOutcome::Failed(code) => {
                return TurnSummary::failed(code, usage).with_billing(billing);
            }
        }
        // 批次执行期间按了 Esc：已启动的已如实落库，未启动的全部退单。
        if *cancel.borrow() {
            cancel_unstarted(&mut host, remaining).await;
            return TurnSummary::interrupted(usage).with_billing(billing);
        }
        continue;
    }

    // ……原串行路径不变，仅把尾部替换为：
    // tool_batch::commit_result(...).await
}
```

### 4.5 顺序与取消语义

| 场景 | 行为 |
| :--- | :--- |
| 正常完成 | 执行完成顺序任意；Begin 顺序、Finish 顺序、DB 顺序、上下文顺序全部按 ordinal |
| 批次执行中 Esc | 已 begin 的调用全部跑完并按真实结果 `Executed` 落库；批次结束后停止采样；未开始的调用记 `cancelled` |
| begin 失败（第 k 个） | 前 k-1 个已 begin 但未执行 → 全部按 `Cancelled` 结算；不执行任何 IO；按原逻辑返回 Interrupted/Failed |
| begin 后、执行前 Esc | 本批全部按 `Cancelled` 结算（零 IO）；后续调用按 `cancelled`；返回 Interrupted |
| finish 失败（第 k 个） | 前 k-1 个已提交；第 k 个失败即停止；剩余已执行但无法落库的票据按 `Cancelled` 收尾（与现有“落库失败不重试”一致） |
| 工具级失败 | 与串行一致：`failed` 结果照样 Commit、进入上下文，轮次继续 |

### 4.6 为什么用 `futures::future::join_all`

`join_all` 需要 `futures` 直接依赖。核查结果：

- `Cargo.lock:759` 已有 `futures 0.3.34`（传递依赖），新增直接声明**不引入任何新的编译单元**；
- 备选 A：`tokio::task::JoinSet` 要求 `E: 'static`，必须给 `run_agent_turn` 加泛型约束并把执行器塞进 `Arc`，为 4 个并发改公共契约不划算；
- 备选 B：手写 `poll_fn` 聚合器，等于重新实现 `join_all`，waker 处理容易出错。

因此 `Cargo.toml` 增加一行：

```toml
# 只读工具并行聚合：已在依赖树（Cargo.lock 0.3.34），仅显式声明。
futures = "0.3"
```

### 4.7 模块拆分（1000 行上限）

`src/chat/agent_loop/mod.rs` 当前 948 行。把批次执行、提交函数、图片提取移入 `tool_batch.rs` 后：

- `mod.rs`：减少约 30 行（`tool_result_images` + 串行尾部），增加约 40 行（批次分支 + `call_allowed`），净增约 10 行，仍低于 1000；
- `tool_batch.rs`：约 170 行，单一职责（“一批只读调用的执行与结算”）；
- 测试放 `src/chat/agent_loop/tests/parallel.rs`，在 `tests.rs` 的 `mod` 列表追加 `mod parallel;`。

### 4.8 可观测性

仅在批次结束时加一条 debug 日志，不含参数与文件内容：

```rust
tracing::debug!(
    target: "lx::agent",
    batch = begun.len(),
    "parallel read-only tool batch settled"
);
```

不新增 metrics、不写 DB、不改卡片状态机。

---

## 5. 测试计划

### 5.1 新增单元测试（`src/chat/agent_loop/tests/parallel.rs`）

用一个 `ProbeExecutor` 观测并发度：`AtomicUsize` 记录当前在跑数量与历史峰值；`execute` 内先自增、`tokio::task::yield_now().await` 两次、再自减，返回 `ToolExecutionResult::Search { matches: vec![], truncated: false, files_scanned: 0 }`。`yield_now` 让 `join_all` 在同一个任务里依次 poll 所有 future，**不依赖墙钟，稳定可复现**。

| 测试名 | 场景 | 断言 |
| :--- | :--- | :--- |
| `parallel_reads_overlap_and_keep_ordinal_order` | 一次响应 3 个 `read` + 最终回复 | 峰值并发 = 3；执行顺序 c0,c1,c2；Trace 为 Begin×3 → Finish×3；`items_of(1)` 的 ToolResult 顺序 c0,c1,c2 |
| `parallel_batch_is_capped_at_four` | 一次响应 5 个 `read` | 峰值并发 = 4；Trace 为 Begin×4 → Finish×4 → Begin×1 → Finish×1 |
| `mutating_call_splits_batches` | `read → write → read` | 峰值并发 = 1；Trace 严格按 read/write/read 三段 |
| `cancel_during_batch_settles_started_and_cancels_unstarted` | 5 个 `read`，第 1 个执行时触发 cancel | 前 4 个 `Finish(..., false)`（真实结果已落库）；第 5 个 `Finish(..., true)`；`TurnOutcome::Interrupted`；只采样 1 次 |
| `begin_failure_cancels_already_begun_batch` | 3 个 `read`，第 3 次 begin 注入失败 | 前 2 个 `Finish(..., true)`；执行器 0 次调用；不产生第 3 个 Finish；轮次失败/中断与注入错误一致 |

### 5.2 分类测试（`src/chat/tools/tests.rs`）

| 测试名 | 断言 |
| :--- | :--- |
| `parallel_safe_tools_are_a_subset_of_tool_names` | `PARALLEL_SAFE_TOOL_NAMES` 每一项都在 `TOOL_NAMES` 中 |
| `parallel_safe_tools_are_exactly_the_read_only_builtins` | 集合等于 `{read, search, web_search, curr_time}` |
| `mcp_qualified_names_are_not_parallel_safe` | `server__read`、`mcp__search` 均返回 false |

### 5.3 现有测试影响

- `executes_tool_calls_in_ordinal_order_not_arrival_order`（`tests.rs:493`）：`write` 是屏障，Trace 仍为 `Begin(write) → Finish(write) → Begin(read) → Finish(read)`，**预期不变**。
- 其余 loop 测试多为单调用或写+读组合，不受影响。
- 事件层测试（`src/event/chat/tests/`）大多每步单调用；并行后同一帧可能出现多个 running 卡片，但状态机按 `call_id` 更新，不需要改 UI。
- 需要完整跑 `cargo test event::chat`，若出现“只允许一个 running 卡片”的隐藏假设，按既有卡片模型修正测试而不是回退并行。

### 5.4 手工验收

1. `cargo run`，在 lx 窗格输入：“同时读取 README、Cargo.toml、src/main.rs 这三个文件”。
2. 观察 3 张工具卡几乎同时进入 running，完成时间接近单个最慢调用，而不是三次之和。
3. 在批次执行中按 Esc：已开始的卡片有真实结果，未开始的显示 cancelled，轮次立即停止。
4. 执行一次 `read → edit → read` 的连续修改任务，确认第二次 `read` 看到的是 `edit` 之后的内容，读后版本保护不报错。

---

## 6. 风险与规避

| 风险 | 影响 | 规避 |
| :--- | :--- | :--- |
| 4 路 `search` 同时扫盘 | 磁盘抖动、单次结果变慢 | 批次上限 4；`search` 本身 10s 超时、8 MiB 单文件上限、20k 文件上限 |
| Begin 逐个等待帧回执 | 每批最多 4 帧（约 64ms）启动延迟 | begin 必须串行：`begin_tool_call` 按 `MAX(position)+1` 分配工具位置，并发 begin 会竞争 timeline position；4 帧可接受 |
| 4 路 `read` 大文件 | 额外内存约 4 × 16 MiB | 输入上限 16 MiB；超限直接 `too_large` |
| 4 路 `web_search` × 内部 4 query | 最多 16 个并发 HTTP | `web_search` 自带超时；实测若过多再单独把 `web_search` 移出并行表 |
| 多子 agent 同时并行 | 最坏 4 子 agent × 4 并发 | 单批上限固定；不做全局池，避免引入跨会话调度器 |
| 同一文件被并发 read | 版本 token 竞争 | `ReadRegistry` 为 `Mutex`，记录同一 token；读后版本保护不受影响 |
| finish 失败留下 pending 行 | DB 状态悬挂 | 批次结算失败时把剩余票据按 `Cancelled` 收尾 |
| Esc 响应变慢 | 最多等到最慢的 `search` 返回 | 与现有串行语义一致（当前也 await 单次工具返回）；不做“丢弃在途结果”的假取消 |
| 事件层测试依赖单 running 卡片 | CI 偶发失败 | 先跑 `cargo test event::chat`；按 `call_id` 更新是既有实现，若测试断言过强则修测试 |

---

## 7. 验收标准

1. `cargo fmt` 无 diff。
2. `cargo clippy --all-targets -- -D warnings` 全绿。
3. 定向测试通过：
   - `cargo test chat::agent_loop::tests::parallel`
   - `cargo test chat::agent_loop`
   - `cargo test chat::tools`
   - `cargo test event::chat`
4. 无新增工具名、无工具描述改动、无 DB schema 改动、无配置项、无 UI 组件改动。
5. 手工验收 4 条全部通过（见 5.4）。

---

## 8. 明确不做（防止 scope creep）

- 不并行 MCP、不并行进程、不并行写工具、不并行 subagent 控制。
- 不做工具延迟暴露（`tool_search`）、不做上下文查询/重置工具。
- 不做 codex 的 hooks、approvals、sandbox、telemetry、Code Mode。
- 不引入全局并发池、不做用户可调并发数。
- 不重构 `ready_calls`、不改 `ToolExecutor` trait、不改事件层 UI 状态机。
