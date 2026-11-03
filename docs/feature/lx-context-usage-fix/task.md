# lx 上下文窗口统计单步重构：任务清单

> 状态：**已合并到 dev**（2026-11-03，合并提交 `51f31675`）；自动验证与 CLI 实测通过。
> 日期：2026-11-03。
> 设计依据：[design.md](./design.md)。

## 1. 准备与设计阶段（已完成）

- [x] 使用 `codegraph` 与 `codebase-memory-mcp` 检索 `lx-tui`、`opencode-dev`、`codex-main` 与 `openclaude-main` 的上下文统计实现。
- [x] 查询 `~/.lx-t-dev/db/lx.db` 分析会话 `sess-18e42d519de95ed0-1` 第 4 轮 51 次工具调用导致 `2,696,606` tokens（100%）的根因。
- [x] 使用 `grill-me` 逐题与用户确认边界（单步记录口径、Anthropic 缓存归一化与分帧合并、`/compact` 边界过滤、历史测试消息处理）。
- [x] 编写详细设计文档 `docs/feature/lx-context-usage-fix/design.md` 与任务文档 `docs/feature/lx-context-usage-fix/task.md`。
- [x] 用户批准执行（2026-11-03）。

---

## 2. 审核门槛 G0（已通过）

用户已确认按本设计执行：
1. **Agent Loop 单步覆盖**：`run_agent_turn` 中每次采样用最新一步的 `step_usage` 覆盖本轮 `usage`，不再跨工具步累加 `input_tokens`。
2. **Anthropic 协议输入归一化**：`message_start` 解码出的 `Usage.input` 统一为 `input_tokens + cache_read_input_tokens + cache_creation_input_tokens`。
3. **流式分帧合并**：`LxChat` 新增 `step_usage` 合并同一次响应内分帧到达的 `input` 与 `output`，并在 `ResponseFinished` 时重置。
4. **Compact 边界感知恢复**：新增 `LxChat::recompute_context_tokens(&LxCompact)`，在加载会话、撤销轮次、完成/撤销 `/compact` 及同步检查点时统一按 `active_upto_sequence` 过滤并重算 `context_tokens`。

---

## 3. 执行任务拆分（已完成）

### Task 1：新建 Git Worktree 并初始化代码索引 MCP
- [x] 在 `.worktrees/lx-context-usage-fix` 新建工作区与分支 `fix/lx-context-usage`。
- [x] 对该 worktree 执行 `codegraph init`（680 files / 18,453 nodes）与 `codebase-memory-mcp index_repository`（16,452 nodes / 129,715 edges）。

### Task 2：Anthropic 输入归一化与 Agent Loop 单步 Usage 重构
- **目标文件**：
  - `src/chat/transport/anthropic/mod.rs`
  - `src/chat/agent_loop/mod.rs`
- **实际改动**：
  - `message_start` 的 `input` 改为 `input_tokens + cache_read_input_tokens + cache_creation_input_tokens`（saturating）。
  - `add_usage` / `add_optional` 删除，替换为 `step_usage(&[ChatDelta]) -> Option<Usage>`：同一次响应内分帧字段合并，跨步只保留最后一次有效快照。
  - `TurnSummary` 文档同步改为「最后一次模型响应」。
- **测试验证**：
  - `src/chat/transport/anthropic/tests.rs`：`usage_from_message_start_and_delta` 断言完整输入 `60 = 50 + 7 + 3`。
  - `src/chat/agent_loop/tests/limits.rs`：
    - `usage_keeps_last_step_only_and_merges_split_frames`（step 2 = input 5 + output 7，不累加 step 1 的 10/2）。
    - `usage_keeps_previous_step_when_last_step_reports_nothing`（最后一步无 usage 时保留上一步）。

### Task 3：流式分帧合并与 Compact 边界上下文重算
- **目标文件**：
  - `src/db/chat/usage.rs`（新增 `Usage::context_tokens`）
  - `src/app/state/lx/chat/usage.rs`（新增 `LxChat` 占用相关方法）
  - `src/app/state/lx/chat.rs`
  - `src/app/state/lx/chat/stream.rs`
  - `src/app/state/lx/agent.rs`
  - `src/app/state/lx/compact.rs`
  - `src/app/update/chat/mod.rs`
  - `src/app/update/chat/session.rs`
  - `src/app/update/chat/rewind.rs`
  - `src/app/update/chat/compact.rs`
- **实际改动**：
  - `LxChat` 新增 `step_usage: Usage`；`absorb_usage_delta` 合并分帧字段并即时计算 `input + output`；`reset_step_usage` 在 `finish_response` / `close_stream` / 新轮开始 / `input_tokens_reset` 时清空。
  - `on_stream_delta` 改调 `absorb_usage_delta`；`on_stream_finished` 用终态单步 usage 同步 `context_tokens`。
  - `LxCompact::active_upto_sequence()` 返回最新 active 检查点的 `end_sequence`。
  - `recompute_context_tokens(&LxCompact)` 倒序扫描 `messages`，只取边界之后最后一条带用量 assistant；无匹配时清空。
  - `on_session_loaded_full`、`on_session_rewound`、`on_compact_finished`、`on_compact_reverted`、`set_checkpoints` 统一重算并刷新状态行；`on_session_rewound` 补齐 `last_tokens_model`。
- **测试验证**：
  - `src/app/update/chat/tests.rs`：`stream_usage_merges_split_frames_per_step`（同一步 100+20=120；下一步 300+50=350，不累加）。
  - `src/app/state/lx/chat/tests.rs`：分帧合并、边界后取最近 assistant、边界覆盖全部时清空、撤销检查点不构成边界、无检查点时取最近 assistant。
  - `src/app/update/chat/tests/compact.rs`：压缩完成清空旧占用、撤销压缩恢复 220、检查点晚到重算。
  - `src/app/state/lx/compact/tests.rs`：`active_upto_sequence` 取最新 active 检查点的 `end_sequence`。
  - `src/event/chat/tests/flow.rs`：端到端终态 usage 改为最后一次采样（4/3）。

### Task 4：代码规范检查、测试与提交
- [x] `cargo fmt` 通过。
- [x] `cargo clippy --all-targets -- -D warnings` 通过。
- [x] `cargo test` 全量通过（4375 passed / 0 failed / 2 ignored；`tests/prompt_copy.rs` 首次运行出现一次与本次改动无关的 pbcopy 时序抖动，单独重跑通过）。
- [x] 生产文件行数复核：`src/app/state/lx/chat.rs` 958 行、`src/db/chat/mod.rs` 981 行，均 < 1000；新增子模块分别承载占用逻辑与 `Usage` 定义。
- [x] 在 worktree 分支中提交代码（提交 `f22603fa`）。
- [x] CLI 实测（`lx-tui exec`，默认 `gemini/gemini-3.8-flash`）：
  - 3 次模型请求的多步工具轮次：`turn.completed.input_tokens = 23,486`（旧逻辑会累加约 6–7 万）。
  - 2 次模型请求的持久化轮次：`turn.completed.input_tokens = 21,142`；隔离库 `agent_message.input_tokens = 21142`、`agent_session.total_input_tokens = 21142`，确认落库也是单步。
- [x] 合并回 `dev`（合并提交 `51f31675`），合并后重新执行 `cargo fmt`、`cargo clippy --all-targets -- -D warnings`、`cargo test` 均通过。
- [x] 移除 worktree `lx-context-usage-fix`、分支 `fix/lx-context-usage` 及其隔离开发库。
