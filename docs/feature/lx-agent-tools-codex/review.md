# Codex 内置 Agent Tool 补齐代码评审报告（Review）

本文档对 `.worktrees/lx-agent-tools-codex`（分支 `feat/lx-agent-tools-codex`）中新增与修改的全部功能代码进行深度走查，按严重程度从 `P0` 到 `P2` 排序列出关键点、错误原因与修复方案。

---

## P0-1：多轮队列连续消费时 `wait_agent` 提前伪唤醒（`settle.rs` / `wait.rs` / `runner.rs`）

- **关键点**：
  当子 agent 队列中有多条待执行输入（或通过 `send_input(interrupt=true)` 中断当前轮并立即启动高优输入）时，第 1 轮在 `finish_subagent` 结算后会同步调用 `schedule_next_queued_input` 启动第 2 轮（`continued == true`）。此时正在等待该子 agent 的 `wait_agent`（尤其是 `wait_for="any"`）必须继续等待第 2 轮完成，不能在两轮交接瞬间提前返回 `timed_out=false, status="running", report=None`。
- **错误原因**：
  1. `src/event/chat/subagent/settle.rs:193` 的通知条件写成了 `if !continued || !matches!(reason, StopReason::Cancelled)`；当第 1 轮以 `StopReason::Complete` 结束且 `continued == true` 时，`!matches!(...)` 为真，错误地向 `handle.done` 发送了完成信号。
  2. 第 1 轮的 `SubagentHandle` 在 `finish_subagent` 末尾被 drop，导致旧 `handle.done`（`watch::Sender<bool>`）被销毁，`wait.rs` 中的 `receiver.changed().await` 收到通道关闭 `Err` 后直接结束等待；而新启动的第 2 轮在 `launch_child_turn` 中创建了全新的 `done` 通道，原等待任务未能无缝衔接。
  3. `src/event/chat/subagent/wait.rs:309` 的 `handle_subagent_wait_ready` 仅在 `auto_continue == true`（即 `wait_for="all"`）时重入 `wait`；当 `wait_for="any"` 被旧通道关闭唤醒时，即使目标仍在跑第 2 轮，也会直接进入 `settle_wait`，返回全为 `running` 且无报告的半途结果。
- **修复方案**：
  1. 在 `finish_subagent` 与 `launch_child_turn` 之间传递并复用同一个 `done: watch::Sender<bool>` 通道；仅在 `!continued`（子 agent 真正停止运行）时才发送 `done.send(true)` 并释放通道。
  2. 在 `handle_subagent_wait_ready` 中增加防御检查：当 `!timed_out` 且 `mode == WaitMode::Any` 时，若所有目标均仍在运行（没有任何一个目标处于非运行态），自动续等当前运行句柄。
  3. 当 `continued == true`（子 agent 仍在连续消费自身队列）时，暂不在第 1 轮交接点触发父会话后台 `drain_reports`，待队列消费完毕（`!continued`）再统一触发。

---

## P0-2：多轮子会话触发运行时自动压缩（`auto_compact`）会丢失当前轮 `send_input` 用户指令（`src/chat/agent_loop/auto_compact.rs`）

- **关键点**：
  子 agent 在第 2 轮或更晚轮次（经 `send_input` 追加）执行期间达到上下文阈值触发 `maybe_auto_compact`（或压缩失败触发 `shrink_uncompressed_to_budget`）时，必须保留**当前轮**的用户输入（`ModelItem::UserText` / `ModelItem::UserContent`），不能丢弃当前轮指令而只留第 0 轮初始任务。
- **错误原因**：
  1. `ModelContext::apply_auto_compact`（`src/chat/agent_loop/auto_compact.rs:65-80`）沿用了单轮子 agent 假设，使用 `self.items.iter().take_while(...)` 只截取了 `self.items` 最开头的第 0 轮用户消息，把排在后面的当前轮 `send_input` 用户消息丢弃，导致压缩后模型丢失当前轮指令。
  2. `ModelContext::shrink_uncompressed_to_budget`（`src/chat/agent_loop/auto_compact.rs:86-114`）从 `leading_count`（第 0 轮用户消息之后）往后删除条目，未保护最后一段当前轮用户消息。
- **修复方案**：
  1. 新增辅助函数定位 `self.items` 中最后一段连续的 `UserText` / `UserContent` 区间（即当前轮的用户输入）。
  2. `apply_auto_compact` 保留当前轮用户输入段（历史轮次及此前工具步骤已汇总进 `compact_summary`）。
  3. `shrink_uncompressed_to_budget` 从最旧的历史条目起按完整 call/result 对裁剪，严格保护当前轮用户输入段不被移除。

---

## P1-1：`send_input` / `resume_agent` 在并发触顶或队首已 `blocked` 时存在先入库后报错的半写问题（`src/event/chat/subagent/runner.rs`）

- **关键点**：
  当目标子 agent 处于 `idle` 状态，但父会话下已有 3 个运行中子 agent（达到 `MAX_CONCURRENT_SUBAGENTS`），或该子 agent 队首已因 `image_missing` 处于 `blocked` 状态时，调用 `send_input` 若返回失败，不得先把输入写入 `agent_subagent_input` 再报错，否则会产生调用方拿不到 `submission_id` 的孤儿排队记录。
- **错误原因**：
  `runner::send_input` 先无条件执行了 `prepare_input_items` 与 `database.enqueue_input`，随后在 `!is_running` 分支调用 `schedule_next_queued_input`；当 `schedule_next_queued_input` 因并发上限（`LIMIT_REACHED_CODE`）或既有队首 `blocked` 返回 `Err` 时，`send_input` 直接向模型返回 `Err`，但新记录已落库且图片已归档。
- **修复方案**：
  在 `send_input` 执行图片归档与入库前，若 `!is_running`：
  1. 先检查该子 agent 队首是否已有 `error_code`（已处于 `blocked`），若有则直接拒绝并提示先用 `cancel_agent_input` 取消阻断项；
  2. 检查父会话当前运行子 agent 数是否已达 `MAX_CONCURRENT_SUBAGENTS`，若已满则直接返回 `LIMIT_REACHED_CODE`（零写库、零残留文件）。
  同理在 `resume` 中，若 `queued_count > 0` 且并发已满，也在修改恢复状态前直接返回 `LIMIT_REACHED_CODE`。

---

## P1-2：`cancel_agent_input` 取消队首 `blocked` 项后未自动恢复后续队列，且 `wait_agent` 状态投影与 `list_agents` 不同源（`src/event/chat/subagent/mod.rs` / `wait.rs`）

- **关键点**：
  1. 当子 agent 因队首输入 `image_missing` 暂停在 `blocked` 状态、且队列后方仍有合法 `queued` 输入时，调用 `cancel_agent_input` 取消队首阻断项后，若该子 agent 仍处于已恢复活动态，应自动认领并启动下一条合法 `queued` 输入。
  2. `wait_agent`（`settle_wait`）、`list_agents` 与 UI `Agents` 面板的 `status` 必须来自同一个状态投影函数。
- **错误原因**：
  1. `mod.rs::cancel_input` 在 `database.cancel_input` 成功后只刷新了 UI 队列计数，未在子 agent 处于 `resumed_subagents` 且当前未运行时调用 `schedule_next_queued_input`。
  2. `wait.rs::settle_wait` 手写了 `running` / `blocked` / `record.status` 三分支，遗漏了 `idle` 与 `interrupted` 投影。
- **修复方案**：
  1. 在 `cancel_input` 中，若目标子 agent 位于 `streams.resumed_subagents` 且当前无运行句柄，调用 `schedule_next_queued_input` 尝试消费下一条排队项，并重新读取最新 `queued_count` 返回。
  2. `wait.rs::settle_wait` 改为调用 `super::projected_agent_status(streams, record, queue_states.get(target))`。

---

## P1-3：子会话跨轮上下文装配对已被预算淘汰的旧轮次执行了归档图片校验，且未复用主上下文的 `assistant_items`（`src/event/chat/subagent/context.rs`）

- **关键点**：
  1. 依据 `design.md` §5.4，只有**在当前压缩检查点与预算保留范围内**的历史消息及当前输入才校验归档图片；已被上下文窗口预算淘汰的极早期轮次即使归档图片缺失，也不应阻断新轮次启动。
  2. `executor` 子 agent 在多轮回放历史 `edit`/`write` 工具结果时，应与主会话一致剥离最高 512 KiB 的 UI `diff` 字段，并在失败调用带已提交变更时保留 `partially applied` 摘要。
- **错误原因**：
  1. `assemble_child_context` 在逆序预算裁剪循环之前，先对所有 `retained` 历史轮次调用了 `replay_persisted_items`（触发磁盘图片校验）。
  2. `src/event/chat/subagent/context.rs` 内部重复实现了简化版 `build_assistant_items`，未调用 `src/chat/context/mod.rs` 中的 `assistant_items`（含 `strip_ui_diff_json` 与 `failure_content`）。
- **修复方案**：
  1. 在 `plan_child_historical_turns` 中先根据 `PersistedInputItem` 估算 token，完成逆序预算筛选后，仅对保留在预算内的 `kept` 历史轮次调用 `replay_persisted_items`。
  2. 将 `src/chat/context/mod.rs::assistant_items` 暴露为 `pub(crate)`，在 `subagent/context.rs` 中直接复用并删除重复函数。

---

## P2-1：子会话懒加载与首轮启动未同步历史 `CompactCheckpoint` 与首轮图片附件徽标（`src/event/chat/subagent/mod.rs` / `runner.rs` / `src/app/update/chat/subagent.rs`）

- **关键点**：
  1. 切换会话或重启后在 `Agents` 面板懒加载子会话（`Effect::LoadSubagentSession`）时，应同时加载该子会话持久化的 `agent_compact_checkpoint` 列表，使详情时间线可见历史压缩块。
  2. 使用 `spawn_agent(items=[...])` 首次带图片启动子 agent 时，`Agents` 面板实时详情中的首条用户消息应立即包含已归档的 `attachments` 元数据。
- **错误原因**：
  1. `Effect::LoadSubagentSession` 仅调用了 `database.load_chat_session`，未调用 `database.list_compact_checkpoints`。
  2. `launch_child_turn` 在 `is_new_agent == true` 分支只使用了内存 `seed_chat_model`（`attachments` 为空），未从已提交的数据库消息中读取附件元数据。
- **修复方案**：
  1. `launch_child_turn` 统一从 `database.load_chat_session` 与 `database.list_compact_checkpoints` 初始化 `child_chat` 与 `ui_chat`（仅在数据库无记录时回退 `seed_chat_model`）。
  2. `Effect::LoadSubagentSession` 与 `on_subagent_loaded` 同步加载并填充 `CompactCheckpoint` 列表。
