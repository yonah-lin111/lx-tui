# 工具执行：只读调用并行化 — 任务清单

> 状态：**已完成并通过定向验证（branch `feat/agent-tool-parallel-readonly`，commit `a1f56ed2`，待用户确认合并；结果见第 10 节）**
>
> 对应设计：`docs/feature/agent-tool-parallel-readonly/design.md`
>
> 总原则：一次只做一件事，每步都能独立编译/测试；不新增工具、不改 DB schema、不改 UI、不加配置项。

---

## 0. 执行前检查

- [x] 用户已确认 `design.md` 的 8 条关键决策：
  1. 只改有明确缺口的少数项，不做全面对齐；
  2. 唯一执行改动是“只读工具并行”，工具面结论为不新增；
  3. 并行白名单只有 `read` / `search` / `web_search` / `curr_time`；
  4. 批次中 Esc：已启动跑完并如实落库，未启动标 `cancelled`；
  5. 单批上限固定 4；
  6. 改动落点允许进入 `src/chat/agent_loop/`；
  7. 用 `futures::future::join_all`，显式新增 `futures = "0.3"`；
  8. 不改事件层、MCP 并发、工具描述、配置项。
- [x] 当前分支干净；记录 `git status --short`，确认没有用户未提交的改动会被覆盖。
- [x] 只跑定向验证（本项目较大，禁止无差别全量构建）。

---

## 1. 增加并行安全分类（`chat/tools`）

### 文件

- `src/chat/tools/types.rs`
- `src/chat/tools/mod.rs`
- `src/chat/tools/tests.rs`

### 改动

1. 在 `types.rs` 的 `TOOL_NAMES` 之后新增：

```rust
/// 同一响应内允许并行执行的内置只读工具。
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

2. `mod.rs` 的 `pub use types::{...}` 中补上 `PARALLEL_SAFE_TOOL_NAMES` 与 `is_parallel_safe`。
3. `tests.rs` 新增 3 个测试（见 design §5.2）：
   - 子集关系；
   - 精确集合；
   - MCP 限定名不命中。

### 完成标准

- `cargo test chat::tools::tests::parallel_safe` 通过（3 个测试）。
- `cargo clippy --all-targets -- -D warnings` 无新增告警。

---

## 2. 声明 `futures` 直接依赖

### 文件

- `Cargo.toml`

### 改动

在 `tokio-stream` 之后加一行，并附注释说明“已在依赖树，仅显式声明”：

```toml
# 只读工具并行聚合：已在依赖树（Cargo.lock 0.3.34），不新增编译单元。
futures = "0.3"
```

### 完成标准

- `cargo check` 通过。
- `git diff Cargo.lock` 只多出 `lx-tui` 包依赖列表里的 `futures` 一项，不新增任何 `[[package]]`。

---

## 3. 新建批次执行模块

### 文件

- 新增 `src/chat/agent_loop/tool_batch.rs`
- `src/chat/agent_loop/mod.rs`（仅加 `mod tool_batch;` 与 import）

### 内容

按 design §4.4 落地，按以下顺序写：

1. `MAX_PARALLEL_TOOL_CALLS: usize = 4`；
2. `BatchOutcome` 枚举（`Completed` / `Interrupted` / `Failed(String)`）；
3. `outcome_of(HostError)`；
4. `cancel_tickets(host, tickets)`：对未执行票据逐个 `finish_tool(Cancelled)`，错误只记 debug，不重试；
5. `execute_batch(...)`：
   - 先全部 `begin_tool`；失败时把已 begin 票据按 `Cancelled` 结算；
   - begin 后检查 `cancel`；
   - 用 `BoxFuture` + `join_all` 并发执行；`allowed == false` 的调用用 `std::future::ready(unavailable_result())`，绝不进 executor；
   - 按原始顺序 `zip(results)`，逐个 `commit_result`；
   - `commit_result` 失败时把剩余票据按 `Cancelled` 收尾；
6. `commit_result(...)`：`format_for_model` → 图片降级 → `finish_tool(Executed)` → `push_tool_result`；
7. 把 `mod.rs:487` 的 `tool_result_images` 原样移入本文件（`pub(super)` 或私有均可，只有本文件用）。

### 关键约束

- `unavailable_result()` 必须与现有串行路径完全一致：

```rust
ToolFailure::new(ToolErrorCode::Io, crate::chat::agent::UNAVAILABLE_CODE)
    .into_result(Vec::new())
```

- `commit_result` 的图片降级文案必须逐字保持：

```text
\n[image omitted: this model does not support image input]
```

- 不在批次内写任何 `tracing::info!`/`warn!`；只在批次结束时写一条 `debug!`，字段仅 `batch` 数量。

### 完成标准

- `cargo check` 通过。
- 文件行数 < 300；`mod.rs` 行数 < 1000。
- 生产代码无 `unwrap()` / `expect()`。

---

## 4. 接入 `run_agent_turn` 主循环

### 文件

- `src/chat/agent_loop/mod.rs`

### 改动

1. 模块文档与 `run_agent_turn` 文档：
   - `mod.rs:1` 的“**串行**工具调度”改为“只读调用并行、其余串行”；
   - `mod.rs:507` 的“串行工具调度”同步更新；
   - `ReadyToolCall` 的 `ordinal` 注释（`mod.rs:78`）从“同一响应内串行执行顺序即此顺序”改为“同一响应内的结算顺序即此顺序”。
2. 抽出 `call_allowed`：

```rust
/// 本轮冻结工具面是否允许该调用（与串行路径同一判定）。
fn call_allowed(context: &ModelContext, name: &str) -> bool {
    if context.request.tools.is_empty() {
        context
            .request
            .tools_surface_at_depth(context.subagent_depth, context.max_subagent_depth)
            .is_some_and(|surface| surface.names().contains(&name))
    } else {
        context.request.tools.iter().any(|decl| decl.name == name)
    }
}
```

3. `let mut remaining = calls.into_iter();` 改为 `let mut remaining = calls.into_iter().peekable();`。
4. 循环顶部插入 design §4.4 的批次分支：
   - 首个调用 `is_parallel_safe` 时收集相邻只读调用，最多 4 个；
   - 计算每个调用的 `allowed`；
   - 调 `tool_batch::execute_batch`；
   - `Completed` 后检查 `cancel`，未取消则 `continue`；
   - `Interrupted` / `Failed` 按现有方式 `cancel_unstarted` + 返回 `TurnSummary`。
5. 串行路径的 `let allowed = ...` 替换为 `call_allowed(&context, &call.name)`。
6. 串行路径尾部（`mod.rs:757-791`）替换为：

```rust
if let Err(error) = tool_batch::commit_result(
    &mut host,
    &mut context,
    &call,
    ticket,
    result,
    executor.supports_image_input(),
)
.await
{
    return match error {
        HostError::Cancelled => TurnSummary::interrupted(usage).with_billing(billing),
        HostError::Failed { code } => TurnSummary::failed(code, usage).with_billing(billing),
    };
}
```

7. 保留 `ask` 失败分支与 MCP `Cancelled` 分支的现有结算方式，不要顺手改成 `commit_result`（它们一个不推上下文、一个要记 `Cancelled`）。

### 完成标准

- `cargo check` 通过。
- 现有 `chat::agent_loop` 测试全绿，尤其：
  - `executes_tool_calls_in_ordinal_order_not_arrival_order`
  - `tool_calls_and_results_are_paired_in_next_request`
  - 取消、ask、MCP 相关用例。

---

## 5. 新增并行编排测试

### 文件

- 新增 `src/chat/agent_loop/tests/parallel.rs`
- `src/chat/agent_loop/tests.rs`：`mod` 列表追加 `mod parallel;`

### 测试夹具

`ProbeExecutor`：

```rust
struct ProbeExecutor {
    running: AtomicUsize,
    peak: AtomicUsize,
    executed: Arc<Mutex<Vec<String>>>,
    cancel: Option<watch::Sender<bool>>,
}

impl ToolExecutor for ProbeExecutor {
    async fn execute(&self, _cwd: &Path, call: &ReadyToolCall) -> ToolExecutionResult {
        let now = self.running.fetch_add(1, Ordering::SeqCst) + 1;
        self.peak.fetch_max(now, Ordering::SeqCst);
        self.executed.lock().unwrap().push(call.call_id.clone());
        // 让 join_all 有机会在同一个任务里 poll 到所有 future，稳定制造重叠。
        tokio::task::yield_now().await;
        tokio::task::yield_now().await;
        self.running.fetch_sub(1, Ordering::SeqCst);
        if let Some(cancel) = &self.cancel {
            let _ = cancel.send(true);
        }
        ToolExecutionResult::Search {
            matches: Vec::new(),
            truncated: false,
            files_scanned: 0,
        }
    }
}
```

### 测试清单

按 design §5.1 至少实现 5 个用例；实际落地 10 个（新增乱序完成、finish 失败、工具面外调用、可取消入口优先、图片降级）。断言必须精确到 Trace 顺序与 `Finish(call_id, cancelled)` 布尔值：

1. `parallel_reads_overlap_and_keep_ordinal_order`
2. `parallel_batch_is_capped_at_four`
3. `mutating_call_splits_batches`
4. `cancel_during_batch_settles_started_and_cancels_unstarted`
5. `begin_failure_cancels_already_begun_batch`
6. `reversed_completion_still_commits_in_ordinal_order`
7. `finish_failure_cancels_remaining_batch_tickets`
8. `call_outside_frozen_tool_surface_is_not_executed`
9. `cancellable_entry_wins_and_others_fall_back`
10. `image_results_are_downgraded_for_text_only_models`

### 完成标准

- `cargo test chat::agent_loop::tests::parallel` 全绿。
- `parallel_reads_overlap_and_keep_ordinal_order` 的峰值并发断言为 3，不是“大约 3”。
- 取消用例只采样 1 次（`script.request_count() == 1`）。

---

## 6. 受影响路径回归

### 命令

```bash
cargo fmt
cargo clippy --all-targets -- -D warnings
cargo test chat::agent_loop
cargo test chat::tools
cargo test event::chat
```

### 关注点

- 事件层 `ack` 测试：Begin 成批出现后，是否仍逐帧 ack；
- `process_tools`：进程工具仍是串行屏障，`TurnExecution.active_interaction` 不被并发污染；
- `ask` / MCP：仍走原分支，不进入批次；
- 子 agent：共享同一 loop，行为随批次生效，但控制工具仍是屏障。

### 完成标准

- 以上命令全绿；若有失败，先判断是测试假设过强还是实现错误，禁止直接回退并行方案。

---

## 7. 手工验收（用户执行）

- [ ] `cargo run`，让模型一次读 3 个文件：3 张卡片几乎同时 running，总耗时接近最慢单个。
- [ ] 批次执行中按 Esc：已启动卡片有真实结果，未启动为 cancelled，轮次立即停止。
- [ ] `read → edit → read`：第二次 read 看到 edit 后的内容，无版本保护误报。
- [ ] 5 个并发 read：观察为 4 + 1 两批，无第 5 个同时 running。

---

## 8. 交付物

- [x] 代码：`chat/tools` 分类、`agent_loop/tool_batch.rs`、`agent_loop/mod.rs` 接入。
- [x] 测试：`tests/parallel.rs` + 分类测试。
- [x] 文档：本目录 `design.md` / `task.md` 状态更新为“已完成”，补齐实际 commit 与验证结果。
- [x] 不提交：临时调试日志、benchmark 脚本、未使用依赖。

---

## 9. 回滚方案

改动集中在三个文件，可直接 revert：

1. `src/chat/agent_loop/tool_batch.rs`（新增文件，删除即可）；
2. `src/chat/agent_loop/mod.rs`（恢复 `remaining` 为 `into_iter()`，删批次分支，恢复串行尾部）；
3. `src/chat/tools/types.rs` / `mod.rs`（删除常量与判定函数）；
4. `Cargo.toml`（删除 `futures` 行；`Cargo.lock` 无变化）。

无 DB migration、无持久化数据格式变化，回滚不需要清理任何数据。

---

## 10. 执行结果（branch `feat/agent-tool-parallel-readonly`，commits `a1f56ed2` / `f6be8b61` / `9bbc175c`）

### 代码

- `src/chat/tools/types.rs` / `mod.rs`：新增 `PARALLEL_SAFE_TOOL_NAMES` 与 `is_parallel_safe` 并重导出。
- `src/chat/agent_loop/tool_batch.rs`：新增 193 行，负责批次 begin、并发执行、ordinal 结算与票据收尾。
- `src/chat/agent_loop/mod.rs`：接入批次分支与 `call_allowed`，串行尾部复用 `commit_result`；964 行（< 1000）。
- `Cargo.toml` / `Cargo.lock`：显式声明 `futures = "0.3"`；锁文件只在 `lx-tui` 依赖列表新增一项，无新 `[[package]]`。

### 测试

- `src/chat/agent_loop/tests/parallel.rs`：10 个编排测试（重叠、上限 4、写屏障、批次取消、begin 失败、乱序完成、finish 失败、工具面外调用、可取消入口优先、图片降级）。
- `src/chat/tools/tests.rs`：3 个分类测试（子集、精确集合、MCP/subagent 不命中）。
- 测试夹具 `RecordingHost` 扩展 `with_begin_failure_after(n, error)`，可注入“第 n+1 次 begin 失败”。

### 验证

- `cargo fmt`：无 diff。
- `cargo clippy --all-targets -- -D warnings`：通过。
- `cargo test --lib chat::agent_loop`：69 passed。
- `cargo test --lib chat::tools`：335 passed（含新增分类测试）。
- `cargo test --lib event::chat`：154 passed。
- CLI 实机：在 worktree 内用 `lx-tui exec` 创建会话并触发 5 个相邻 `read`，观测 `batch=4` + `batch=1`；详见 [`verification.md`](verification.md)。

### 与设计的差异（已同步回 design §4.4）

批次内的单次执行不是直接 `execute_tracked`，而是 `run_one`：先调用 `execute_mcp_cancellable`，返回 `Some` 时采用（`web_search` 的 Esc 取消必须直达 HTTP 请求），否则回落到 `execute_tracked`。串行路径原本就是这个顺序，批次不能降级。

### 未完成

- 手工验收（第 7 节）由用户运行 `cargo run` 确认。
- 合并到 `dev` 前需用户确认。
