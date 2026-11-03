# lx 上下文窗口统计单步重构：代码评审

> 评审对象：`dev` 分支 `950a66a8..51f31675`（功能提交 `f22603fa`，合并提交 `51f31675`）。
> 评审日期：2026-11-03。
> 评审方式：`codegraph` + `codebase-memory-mcp` 调用链核查、逐文件 diff 复核、CLI/DB 实测结果对照。
> 关联文档：[design.md](./design.md)、[task.md](./task.md)。

## 1. 评审范围与结论

本次功能把上下文占用从“整轮多步累加 input”改为“最后一次模型响应的 input + output”，并补上了 Anthropic 缓存口径、流式分帧合并、`/compact` 边界过滤。核心修复方向正确，CLI 实测与全量测试均通过。

但逐链路复核发现：**单步 usage 被同时用到了不该用的地方**——`TurnSummary.usage` 既承担“上下文占用”，又被 `Database::finish` 当作“整轮账单”写入 `agent_session.total_*`，还被 CLI 当作 “tokens used” 展示。这违反了已确认的 Q1 边界（上下文占用单步、会话累计账单跨步求和），属于本次重构引入的静默数据回归。

未发现 P0（崩溃/数据损坏/安全问题）。以下按优先级排列。

| 编号 | 问题 | 状态 |
| --- | --- | --- |
| P1-1 | 会话累计账单与 CLI 统计被单步 usage 覆盖 | 已修复 |
| P1-2 | `step_usage` 未在主轮开始时重置 | 已修复 |
| P2-1 | 文档与实际符号不一致 | 已修复 |
| P2-2 | rewind 重载路径缺少压缩边界测试 | 已修复 |
| P3-1 | 细节与可读性 | 已修复 |

---

## 2. 问题清单

### P1-1 会话累计账单与 CLI 统计被单步 usage 覆盖（数据回归）

**关键点**

- `TurnSummary` 只有一个 `usage` 字段，`run_agent_turn` 现在用最后一次采样覆盖它。
- `Database::finish` 用同一个 `usage` 同时写：
  - `agent_message.input_tokens / output_tokens`（消息级，应该单步）；
  - `agent_session.total_input_tokens / total_output_tokens`（会话级，应该整轮累计）。
- CLI `HumanOutputProcessor` 的 `tokens used` 与 JSONL `turn.completed.usage` 也读这个字段。

**错误原因**

原实现 `add_usage` 跨步累加，同时服务“消息级”和“会话级”两个口径。本次修复只把 `TurnSummary.usage` 改成单步，却没有为会话级/CLI 保留累计值，导致：

- 51 步工具轮次的会话账单从约 270 万 token 变成最后一步的约 5 万；
- CLI 的 `tokens used` 从“整轮消耗”变成“最后一次请求的上下文”；
- 与已确认的 Q1 决策冲突（`agent_session.total_*` 必须继续跨步累计）。

**修复方案**

- `TurnSummary` 增加 `billing: Usage`：`usage` 保持最后一次响应（上下文/消息口径），`billing` 跨步累加（账单/CLI 口径）。
- `Database::finish` 拆成 `finish`（兼容旧调用，billing = usage）与 `finish_with_billing`（消息写 usage、会话累计写 billing）。
- `ChatNetEvent::Finished`、`Effect::FinishChatTurn`、`on_stream_finished` 同时携带两个值；CLI 与子 agent 会话累计改用 billing。
- 新增测试锁定：多步轮次消息行 = 最后一步，`agent_session.total_*` = 各步之和。

### P1-2 `step_usage` 未在主轮开始时重置，丢弃的终态可能把上一步 usage 带进新轮

**关键点**

- `LxChat.step_usage` 只在 `finish_response`、`close_stream`、`seed_chat_model`（仅子 agent 初始种子）与 `input_tokens_reset` 中清空。
- 主轮 `on_turn_committed` 直接进入 `RequestState::Streaming`，没有清空 `step_usage`。
- `on_stream_finished` 在 request key 不匹配（窗格切换、旧 generation、会话切换）时提前返回，不会执行 `close_stream`。

**错误原因**

如果上一轮因为切窗格/切会话导致终态被丢弃，`step_usage` 可能残留 `{input: X}` 或 `{output: Y}`。下一轮第一帧若只报单侧字段（Anthropic 的 `message_delta` 只报 output），会短暂算出 `X + 新 output` 的错误占用，直到 input 帧到达才纠正。虽然窗口很短，但违反了“每个采样步从空聚合槽开始”的不变量。

**修复方案**

- `on_turn_committed` 在进入 Streaming 前调用 `reset_step_usage()`。
- 新增测试：预先塞入脏 `step_usage`，提交新轮后断言已清零；再发一个只有 output 的 Usage，断言上下文不包含旧 input。

### P2-1 文档与实际符号不一致

**关键点**

- design/task 中多处写 `on_session_rewound`，实际函数是 `on_session_reloaded`。
- design 4.4 仍写 `CompactState`，实际类型是 `LxCompact`。
- design 4.4 声称 `recompute_context_tokens` 会设置 `last_tokens_model = message.model_key`，但 `ChatMessageView` 没有 `model_key` 字段；实现只重算 `context_tokens`，模型键由 `refresh_status` 维护。

**错误原因**

实现阶段为控制 `ChatMessageView` 的改动面，选择了“只重算分子”的方案，但设计文档未同步；后续维护者按文档找符号会落空。

**修复方案**

- 文档统一改为实际符号与职责：`on_session_reloaded`、`LxCompact`、只重算 `context_tokens`。
- 在 review/task 中记录该偏差及原因。

### P2-2 rewind 重载路径缺少压缩边界回归测试

**关键点**

- `on_session_reloaded` 已改为 `recompute_context_tokens(&target.lx.compact)`，但测试只覆盖了会话加载与 compact 完成/撤销。
- 该路径同时补齐了 `last_tokens_model`，没有测试锁定。

**错误原因**

rewind 是用户删除轮次后恢复上下文的入口，最容易出现“被删轮次用量残留”或“压缩前高占用复活”。缺少测试时，后续改动容易破坏边界过滤。

**修复方案**

- 在 `src/app/update/chat/tests/rewind.rs` 增加：带 active compact 检查点的会话重载后，只取边界之后的最后一条 assistant；边界覆盖全部消息时清空。

### P3-1 细节与可读性

**关键点**

- `seed_chat_model` 使用 `crate::db::chat::Usage::default()` 全限定路径，而同文件其他位置可直接用 `Usage`。
- `on_stream_finished` 在 usage 全未知时保留流中已知占用，这是设计意图，但没有测试锁定“取消发生在 usage 之前不清空”。

**错误原因**

不影响正确性，但降低一致性；缺少测试会让“保留已知值”的意图在后续重构中被误删。

**修复方案**

- 统一使用 `Usage::default()`。
- 增加一个取消/无 usage 的回归断言（可并入现有取消测试）。

---

## 3. 修复优先级与执行顺序

1. P1-1：补 `billing` 链路（agent loop → event → app → effect → DB/CLI），补多步账单测试。
2. P1-2：主轮开始清空 `step_usage`，补脏槽测试。
3. P2-1：同步 design/task 文档。
4. P2-2：补 rewind 边界测试。
5. P3-1：统一写法，补取消保留占用的断言。

## 4. 验证要求

- `cargo fmt`。
- `cargo clippy --all-targets -- -D warnings`。
- `cargo test` 全量通过。
- 多步 CLI 实测：`turn.completed.usage`（上下文）保持最后一次请求，`agent_session.total_*` 为各步之和。

## 5. 修复结果（2026-11-03）

- **P1-1 已修复**：`TurnSummary` 新增 `billing`，`run_agent_turn` 跨步累加；`ChatNetEvent::Finished`、`Effect::FinishChatTurn`、`on_stream_finished`、`Database::finish_with_billing` 全链路携带两个口径；消息行写 `usage`，`agent_session.total_*` 写 `billing`；CLI 的 `tokens used` 与 JSONL `turn.completed` 改用 `billing`。
- **P1-2 已修复**：`on_turn_committed` 进入 Streaming 前调用 `reset_step_usage()`；新增脏槽回归测试。
- **P2-1 已修复**：design/task 统一为 `on_session_reloaded`、`LxCompact`，并说明 `recompute_context_tokens` 只重算 `context_tokens`。
- **P2-2 已修复**：新增 `reload_after_rewind_respects_compact_boundary`（边界后取最后一条 assistant；边界覆盖全部时清空）。
- **P3-1 已修复**：`seed_chat_model` 统一 `Usage::default()`；新增“取消发生在 usage 之前保留已知上下文”的断言。
- **验证**：`cargo fmt` 通过；`cargo clippy --all-targets -- -D warnings` 通过；`cargo test` 4442 passed / 0 failed / 2 ignored。
- **端到端测试**：`event::chat::tests::flow::write_then_read_then_final_answer_persists_in_order` 同时断言 `usage = 4/3`（最后一步）与 `billing = 14/5`（整轮累计）；`db::chat::tests::finish_with_billing_keeps_message_usage_and_session_totals_separate` 锁定落库拆分。
