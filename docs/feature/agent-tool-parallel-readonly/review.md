# 只读工具并行 — 代码评审 (`review.md`)

- **特性目录**：`docs/feature/agent-tool-parallel-readonly/`
- **评审对象**：branch `feat/agent-tool-parallel-readonly`（`dev..HEAD`）
- **修复提交**：`c08c9148`（代码与测试）· `14ec2776` / `d652e177`（评审文档）
- **合并**：fast-forward 至 `dev`（`d652e177`）；工作区与分支已删除
- **关联文档**：[`design.md`](design.md) · [`task.md`](task.md) · [`verification.md`](verification.md)
- **评审方式**：codegraph / codebase-memory 结构检索 + 逐行核对 + 定向测试
- **结论**：无 P0；P1 1 项、P2 3 项、P3 2 项。P1 / P2 已修复，P3 一项修复、一项保留。

---

## 1. 关键不变量

| 不变量 | 结论 |
| :--- | :--- |
| 只有 `read` / `search` / `web_search` / `curr_time` 可进入批次，且必须相邻 | 成立；MCP 限定名、写工具、进程工具、`ask`、subagent 控制工具均为屏障 |
| 单批最多 4 个 | 成立；5 个相邻 `read` 拆成 4 + 1 |
| Begin 串行、Finish / DB / 上下文按 ordinal | 成立；`join_all` 只影响完成时刻，不改变 `begun` 的 zip 顺序 |
| 已启动调用如实落库，未启动调用记 `cancelled` | 修复后成立；Cancelled 结果只作用于自身 |
| 不放宽工具面、不改 DB schema / UI / 配置 | 成立；批次内 `allowed == false` 仍产出 `UNAVAILABLE_CODE` |
| 串行路径行为不退步 | 成立；单工具批次复用同一 `commit_result` 与图片降级口径 |

---

## 2. 问题清单

### P0

无。未发现数据损坏、越权执行、跨会话串写或状态机死锁。

### P1-1 批次内某个 `Cancelled` 结果会丢弃同批已完成的真实结果

- **位置**：`src/chat/agent_loop/tool_batch.rs:82-106`
- **现象**：批次 `[web_search, read]` 中 `web_search` 因 Esc 返回 `Cancelled` 时，排在它后面的 `read` 已经执行完毕，却被 `cancel_tickets(iter...)` 统一记成 `cancelled`，真实结果被丢弃。
- **错误原因**：`futures::future::join_all` 返回时**所有** future 都已结束；把“某个调用结果 Cancelled”误当成“整批还没执行”，于是把后续真实结果退单。违反 design §1.2 目标 3 与 §4.5。
- **影响**：DB / UI 对“是否执行过”的描述不实；`read` 已经登记版本、消耗 IO，模型却拿不到结果；手工验收“已开始卡片有真实结果”失败。
- **修复**：只把该 ticket 记 `Cancelled` 并继续按 ordinal 提交后续结果；全部结算后返回 `Interrupted`。测试 `cancelled_execution_result_keeps_sibling_results` 锁定该行为。
- **验证**：Trace 由 `c0 false / c1 true / c2 true` 改为 `c0 false / c1 true / c2 false`，轮次仍为 `Interrupted`。

### P2-1 单个 `finish_tool` 失败会把其余已执行结果误记为 cancelled

- **位置**：`src/chat/agent_loop/tool_batch.rs:96-102`、`settle_remaining`（`:116-148`）
- **现象**：第 k 个票据落库失败后，后面的票据即使已有真实结果也统一 `Cancelled`。
- **错误原因**：把“一行写入失败”错误泛化为“整批无法落库”。行级失败（终态冲突、单条结果过大）不代表其它行不能写。
- **影响**：DB / UI 再次出现假 `cancelled`，已经执行的读盘 / 搜索被丢弃。
- **修复**：失败票据按契约不重试；`settle_remaining` 对其余结果 best-effort 调用 `commit_result`，失败只记 `warn`，最终仍按原始错误结束本轮。
- **验证**：`finish_failure_keeps_remaining_batch_results` 断言 c1 / c2 按真实结果结算。

### P2-2 缺少事件层 + DB 的多 running 集成测试

- **位置**：`src/chat/agent_loop/tests/parallel.rs` 全部使用 `RecordingHost`；没有真实 `Persistence` 下的多 begin 用例。
- **现象**：位置分配、逐帧 ack、四张 running 卡片只被 CLI happy path 覆盖，CI 无法捕获回归。
- **错误原因**：编排层测试的 ticket 位置由 `issued.len()` 伪造，绕过了 `begin_tool_call` 的 `MAX(position)+1` 与 `checkpoint_before_tool`。
- **修复**：新增 `src/event/chat/tests/flow.rs:185` 的 `four_reads_in_one_response_create_dense_tool_positions`，断言 4 个 begin 全部先于 finish、位置稠密连续、4 行 completed、4 张卡片终态。
- **验证**：该用例通过；`event::chat` 155 passed。

### P2-3 4 路图片 `read` 会放大解码内存

- **位置**：`src/chat/tools/read.rs:27-31`、`:116-119`；design §6 风险表。
- **现象**：单张图片解码上限 32M 像素（约 128 MiB 缓冲），4 路并行可把峰值放大到数百 MiB；原设计只按文本 `4 × 16 MiB` 估算。
- **错误原因**：并行白名单包含 `read`，但风险表只考虑了文本文件，没有覆盖图片解码路径。
- **影响**：大图批量读取时内存压力显著，多个子 agent 同时运行会进一步放大。
- **修复**：`IMAGE_READ_GATE` 串行化 `read_image` 的解码阶段；图片单文件 20 MiB 上限保持不变，缓存命中仍走快路径。
- **验证**：`chat::tools` 335 passed；现有图片读取测试不受影响。

### P3-1 begin 阶段按 Esc 仍会完成剩余 begin

- **位置**：`src/chat/agent_loop/tool_batch.rs:51-59`
- **现象**：批次 begin 循环只在全部 begin 后检查 cancel；Esc 落在 begin 期间会多写最多 3 行 pending → cancelled。
- **错误原因**：提前退出会让尚未 begin 的批次内调用没有终态行；要保持“每个已声明调用都有记录”就必须补一轮 begin + cancel。
- **结论**：保留现状。多写的是 `cancelled` 终态行，不执行 IO，也不改变轮次中断；记录在 design §9。

### P3-2 `mark_tool_running` 失败会留下永久 pending 行

- **位置**：`src/event/chat/tool_calls.rs:191-202`（BTW 路径 `:168-176` 同类）
- **现象**：pending 已提交、running 推进失败时只回 `HostError::Failed`，没有退单；同进程内重载会看到永久 pending 卡片。
- **错误原因**：失败分支漏调 `cancel_pending`；启动恢复只能兜底跨进程崩溃，兜不住同进程内的这次失败。
- **影响**：批次 begin-all-first 会放大该窗口，最多同时留下多行 pending。
- **修复**：两个 `mark_tool_running` 失败分支都调用 `cancel_pending`，把已有 pending 行落为 `cancelled`。
- **验证**：既有 `running_failure_has_no_file_side_effect` 仍通过；该分支为防御性退单，未新增直接注入 DB 更新失败的用例（需要触发器级故障注入，收益低于改动量）。

---

## 3. 已核实无问题

- **位置分配**：`begin_tool_call` 在同一事务内取 `parts ∪ tools` 的 `MAX(position)+1`，连续 begin 得到稠密位置；4-read 集成测试已覆盖。
- **事件层 ack**：每个 `begin_tool` 都等待回执，同一时刻最多一个 `PendingBegin`；事件层按 `call_id` 推进，不假设只有一个 running 工具。
- **状态机**：`mark_tool_running` / `finish_tool_call` 按 `(session, message, call_id)` 匹配，DB 允许同一消息多行 running；恢复逻辑统一把 pending/running 转 interrupted。
- **工具面**：`is_parallel_safe` 仅为 4 个内置工具；MCP 名称带 `mcp__` 前缀不会命中；`allowed == false` 不进入 executor；控制 / `ask` / subagent 仍是串行屏障。
- **并发安全**：`ReadRegistry` 为 `Mutex`，`search` 无共享状态，`web_search` 共享连接池且取消走 `JoinSet::abort_all`，`AgentTools` 满足 `Send + Sync`，文件工具在 `spawn_blocking` 执行。
- **串行等价**：单工具批次仍走 `execute_mcp_cancellable → execute_tracked`、`format_for_model → finish_tool → push_tool_result`，图片降级文案一致。

---

## 4. 修复后验证

| 命令 | 结果 |
| :--- | :--- |
| `cargo fmt --check` | 通过 |
| `cargo clippy --all-targets -- -D warnings` | 通过 |
| `cargo test --lib chat::agent_loop` | 71 passed |
| `cargo test --lib chat::tools` | 335 passed |
| `cargo test --lib event::chat` | 155 passed |
| `cargo test --lib chat::agent_loop::tests::parallel` | 12 passed |

---

## 5. 剩余风险

- `web_search` 4 路 × 每调用最多 4 query 的外部限流风险没有实测数据；保留并行表，按 design §6 的预案可在限流时单独移出。
- `read` 文本路径仍会为版本 token 再读磁盘（既有实现），并行只是提高触发频率；不在本次修复范围。
- begin 阶段的 Esc 延迟最多多出 3 次 begin（约 48ms）；无 IO，仅为终态记录。
- CLI 实机使用本地 Mock 模型，不覆盖真实供应商网络与真实 TUI 手工交互。
