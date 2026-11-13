# 只读工具并行 — 验证报告 (`verification.md`)

- **特性目录**：`docs/feature/agent-tool-parallel-readonly/`
- **关联文档**：[`design.md`](design.md) · [`task.md`](task.md)
- **验证工作区**：`.worktrees/agent-tool-parallel-readonly`（分支 `feat/agent-tool-parallel-readonly`）
- **代码提交**：`a1f56ed2`（实现）· `f6be8b61`（首轮文档）· `9bbc175c` / `81a3cf07` / `e0cd17c2` / `65913e2a`（测试与验证）· `c08c9148`（评审修复）· `14ec2776`（评审文档）
- **测试二进制**：`./target/debug/lx-tui`
- **验证结论**：16 / 16 项通过（PASS）

---

## 1. 验证环境与总览

| 项目 | 命令 / 环境 | 结果 |
| :--- | :--- | :--- |
| 格式 | `cargo fmt --check` | 通过 |
| 静态检查 | `cargo clippy --all-targets -- -D warnings` | 通过 |
| Agent loop | `cargo test --lib chat::agent_loop` | 71 passed |
| 工具层 | `cargo test --lib chat::tools` | 335 passed |
| 事件层回归 | `cargo test --lib event::chat` | 155 passed |
| CLI 实机 | 隔离 `HOME` + `LX_TUI_DB_PATH` + 本地 Anthropic SSE Mock | Exit Code 0 |

CLI 实机使用本地 Mock 模型，目的是消除外部网络与真实模型随机性；会话、工具行与结果均真实写入 worktree 隔离库 `~/.lx-t-dev/db/worktrees/agent-tool-parallel-readonly-e50690f2/lx.db`。

---

## 2. 逐项验证明细

### V01 — 并行白名单与分类边界
- **输入内容**：`read` / `search` / `web_search` / `curr_time`；串行工具 `write` / `edit` / `exec_command` / `write_stdin` / `update_plan` / `ask`；MCP 限定名 `server__read` / `mcp__search`；subagent 工具 `spawn_agent` / `cancel_agent_input`。
- **验证步骤**：执行 `cargo test --lib chat::tools::tests::parallel`。
- **期望结果**：白名单精确等于 4 个只读内置工具、是 `TOOL_NAMES` 子集；串行工具、MCP 限定名与 subagent 工具全部返回 `false`。
- **结果**：PASS（3 passed）。

### V02 — 相邻只读调用重叠执行，Begin / Finish / 上下文保持 ordinal
- **输入内容**：同一响应内 3 个相邻 `read`（`c0` / `c1` / `c2`）。
- **验证步骤**：执行 `cargo test --lib chat::agent_loop::tests::parallel::parallel_reads_overlap_and_keep_ordinal_order`。
- **期望结果**：执行峰值并发为 3；Trace 为 `Begin c0,c1,c2` 后 `Finish c0,c1,c2`；模型上下文中的工具结果顺序为 `c0,c1,c2`。
- **结果**：PASS。

### V03 — 单批上限固定为 4
- **输入内容**：同一响应内 5 个相邻 `read`。
- **验证步骤**：执行 `cargo test --lib chat::agent_loop::tests::parallel::parallel_batch_is_capped_at_four`；CLI 实机同样返回 5 个 `read`。
- **期望结果**：单测峰值并发为 4；CLI 事件为 4 个 `item.started` → 4 个 `item.completed` → 第 5 个 `item.started` / `item.completed`；调试日志出现 `batch=4` 与 `batch=1`。
- **结果**：PASS（单测 1 passed；CLI 日志实测 `batch=4`、`batch=1`）。

### V04 — 写工具与非白名单工具是串行屏障
- **输入内容**：`read → write → read`；`exec_command` / `update_plan` / `ask` 等非白名单调用。
- **验证步骤**：执行 `cargo test --lib chat::agent_loop::tests::parallel::mutating_call_splits_batches` 与 V01 分类测试。
- **期望结果**：执行峰值为 1；每个调用按 `Begin` / `Finish` 成对串行，批次在写工具处切断。
- **结果**：PASS。

### V05 — 乱序完成仍按 ordinal 结算
- **输入内容**：两个相邻 `read`，其中 `c0` 等待 `c1` 完成后才返回。
- **验证步骤**：执行 `cargo test --lib chat::agent_loop::tests::parallel::reversed_completion_still_commits_in_ordinal_order`。
- **期望结果**：实际完成顺序为 `c1,c0`；`Finish` 顺序与模型上下文顺序仍为 `c0,c1`。
- **结果**：PASS。

### V06 — 批次执行中 Esc
- **输入内容**：5 个相邻 `read`；第一批 4 个执行完成后触发取消。
- **验证步骤**：执行 `cargo test --lib chat::agent_loop::tests::parallel::cancel_during_batch_settles_started_and_cancels_unstarted`。
- **期望结果**：轮次为 `Interrupted` 且不再采样；`c0`–`c3` 落真实结果；`c4` 仅 `Begin` 后以 `Cancelled` 结算，不执行。
- **结果**：PASS（`request_count == 1`）。

### V07 — begin 完成后、执行前取消
- **输入内容**：3 个相邻 `read`，第 2 次 `begin_tool` 成功后触发取消。
- **验证步骤**：执行 `cargo test --lib chat::agent_loop::tests::parallel::cancel_after_begins_settles_whole_batch_without_execution`。
- **期望结果**：轮次为 `Interrupted` 且不再采样；3 个票据全部按 `Cancelled` 收尾；执行器零调用。
- **结果**：PASS。

### V08 — 批次内某个执行结果报告 cancelled
- **输入内容**：3 个相邻 `read`，`c1` 的执行器返回 `ToolErrorCode::Cancelled`。
- **验证步骤**：执行 `cargo test --lib chat::agent_loop::tests::parallel::cancelled_execution_result_keeps_sibling_results`。
- **期望结果**：`c0` / `c2` 提交真实结果；只有 `c1` 票据按 `Cancelled` 结算；轮次为 `Interrupted` 且不再采样。
- **结果**：PASS（评审 P1 修复后）。

### V09 — 批次 begin 中途失败
- **输入内容**：3 个相邻 `read`，第 3 次 `begin_tool` 注入失败。
- **验证步骤**：执行 `cargo test --lib chat::agent_loop::tests::parallel::begin_failure_cancels_already_begun_batch`。
- **期望结果**：轮次失败且不再采样；不执行任何调用；已 begin 的 `c0` / `c1` 票据按 `Cancelled` 收尾。
- **结果**：PASS。

### V10 — 批次 finish 中途失败
- **输入内容**：3 个相邻 `read`，第 1 次 `finish_tool` 注入失败。
- **验证步骤**：执行 `cargo test --lib chat::agent_loop::tests::parallel::finish_failure_keeps_remaining_batch_results`。
- **期望结果**：轮次失败且不再采样；`c0` 落库失败不重试；`c1` / `c2` 已执行结果仍按真实结果结算。
- **结果**：PASS（评审 P2 修复后）。

### V11 — 冻结工具面之外的调用不执行
- **输入内容**：同一响应内 `read` + `search`；本轮冻结工具面只声明 `read`。
- **验证步骤**：执行 `cargo test --lib chat::agent_loop::tests::parallel::call_outside_frozen_tool_surface_is_not_executed`。
- **期望结果**：只有 `read` 进入执行器；`search` 仍走 `begin` / `finish`，结果为 `UNAVAILABLE_CODE`，不是 `cancelled`。
- **结果**：PASS。

### V12 — 可取消入口优先，其余回落通用入口
- **输入内容**：批次内 `web_search` + `read`；执行器对 `web_search` 返回可取消结果，对 `read` 返回 `None`。
- **验证步骤**：执行 `cargo test --lib chat::agent_loop::tests::parallel::cancellable_entry_wins_and_others_fall_back`。
- **期望结果**：`web_search` 只经过 `execute_mcp_cancellable`；`read` 回落到 `execute_tracked` / `execute`；两者均正常结算。
- **结果**：PASS。

### V13 — 文本模型的图片结果降级
- **输入内容**：`read` 返回图片归档；`supports_image_input() == false`。
- **验证步骤**：执行 `cargo test --lib chat::agent_loop::tests::parallel::image_results_are_downgraded_for_text_only_models`。
- **期望结果**：模型上下文中的图片数组为空；正文追加 `[image omitted: this model does not support image input]`。
- **结果**：PASS。

### V14 — CLI 在工作区创建并持久化会话
- **输入内容**：
  ```bash
  HOME=<临时配置目录> LX_TUI_ENV=dev \
  LX_TUI_DB_PATH=~/.lx-t-dev/db/worktrees/agent-tool-parallel-readonly-e50690f2/lx.db \
  ./target/debug/lx-tui exec -C .worktrees/agent-tool-parallel-readonly --json \
    "Read five source files for the parallel verification report."
  ```
  本地 Mock Anthropic 首轮返回 5 个 `read`，次轮返回最终文本。
- **验证步骤**：执行上述命令后，用同一 `HOME` / `LX_TUI_DB_PATH` 执行 `lx-tui session list -C .worktrees/agent-tool-parallel-readonly --json` 与 `lx-tui session show --last -C .worktrees/agent-tool-parallel-readonly --tools --json`。
- **期望结果**：Exit Code 0；`session list` 返回新建会话；`session show` 返回 user + assistant 两条消息，assistant 携带 5 个 `read` 工具行且按 ordinal 排列。
- **结果**：PASS（session `sess-18e78e5ad106e5b8-0`，5 个 `read` 全部 `completed`）。

### V15 — CLI 实机事件顺序与并发批次
- **输入内容**：V14 的 JSONL stdout 与 `LX_TUI_LOG=debug` 日志。
- **验证步骤**：解析 `item.started` / `item.completed` 序列，并检索 `lx::agent` 的批次结算日志。
- **期望结果**：4 个 `item.started` 全部先于任何 `item.completed`，随后第 5 个 `item.started` / `item.completed`；日志出现 `parallel read-only tool batch settled batch=4` 与 `batch=1`。
- **结果**：PASS。

### V16 — 事件层 + DB 的四路 read 多 running 握手
- **输入内容**：同一响应内 4 个相邻 `read`（`r0`–`r3`），真实 `Persistence` 与临时工作区。
- **验证步骤**：执行 `cargo test --lib event::chat::tests::flow::four_reads_in_one_response_create_dense_tool_positions`。
- **期望结果**：4 个 `Begin` 全部先于 `Finish`；Finish 按 ordinal；工具位置稠密连续；4 行 `completed` 且结果体齐备；4 张卡片进入终态。
- **结果**：PASS（新增于评审 P2-2）。

---

## 3. 未覆盖与说明

- 真实 TUI 的手工验收仍需用户运行 `cargo run` 完成：3 个并发读卡片、批次中 Esc、`read → edit → read`、5 个读拆 4 + 1。
- CLI 实机使用本地 Mock 模型，不验证真实供应商网络行为；网络重试与流式协议由既有测试覆盖。
- 本特性不涉及 DB schema、UI 与配置项，无迁移或回滚数据清理。

