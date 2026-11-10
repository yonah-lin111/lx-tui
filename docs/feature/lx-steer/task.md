# lx 流中 steer 任务拆解

> 状态：已完成并合并到 `dev`（merge commit `5c4d86f2`；worktree `.worktrees/lx-steer` 与分支 `feat/lx-steer` 已清理）
> 设计依据：[design.md](design.md) · Review 记录：[review.md](review.md) · 验证报告：[verification.md](verification.md)
> 执行位置：原 `.worktrees/lx-steer`（分支 `feat/lx-steer`，已合并至 `dev` 并移除）
> 完成标准：实现 + 测试 + `cargo fmt` + `cargo clippy --all-targets -- -D warnings` + `cargo test` + 提交 + 合并回 `dev`

---

## 阶段 0：纯数据与 DB

### T0.1 `PendingSteer` 与 `build_batch`

**文件**

- `src/app/state/lx/chat/steer.rs`（新增）
- `src/app/state/lx/chat.rs`
- `src/app/state/lx/chat/tests.rs`（或 `chat/steer/tests.rs`）

**关键点**

1. 按 `design.md §5.1` 定义 `PendingSteer`：

```rust
pub struct PendingSteer {
    pub text: String,
    pub attachments: Vec<MessageAttachmentInput>,
    pub model_images: Vec<ModelImage>,
    pub model: ModelSnapshot,
}
```

2. `LxChat` 增加：

```rust
pub pending_steers: Vec<PendingSteer>,
pub promoting_steers: usize,
```

3. `LxChat::new` 初始化；`reset_session_view` 清空/归零。
4. 预算常量：

```rust
pub(crate) const MAX_PENDING_STEERS: usize = 8;
pub(crate) const MAX_PENDING_IMAGE_BYTES: u64 = 40 * 1024 * 1024;
```

5. `build_batch` 严格按 `design.md §5.3`：
   - 至少一条；
   - 文本空行拼接；
   - attachment marker 偏移平移；
   - 图片去重；
   - 文本 2 MiB / 图片 4 张 / 图片字节 40 MiB 上限；
   - 返回 `consumed`；
   - 不修改 pending。

6. 单测覆盖：
   - 纯文本 3 条合并；
   - 引用/图片 marker 平移；
   - 第 5 张图片停止合并；
   - 超长文本停止合并；
   - 图片去重；
   - 空输入返回 None；
   - 模型快照取首条。

**测试**

```bash
cargo test app::state::lx::chat::steer
```

**完成标准**：所有合并/上限分支有单测；`build_batch` 不执行 IO、不依赖 `AppState`。

**依赖**：无。

---

### T0.2 DB `begin_steer_turn`

**文件**

- `src/db/chat/mod.rs`
- `src/db/service/chat.rs`
- `src/db/chat/tests.rs`

**关键点**

1. `MAX_MESSAGE_BYTES` 改为 `pub(crate)`。
2. 新增 `Database::begin_steer_turn`，签名见 `design.md §5.5`。
3. 事务顺序必须严格：
   - 校验 workspace/session/previous assistant streaming；
   - `replace_parts` 写旧片段最终 parts；
   - 旧 assistant `status='complete', stop_reason='steer', completed_at_ms=now`；
   - `rewind::seal_turn_scope`；
   - `next_session_sequence`；
   - `turn_index = MAX(turn_index)+1`；
   - user message + text part + attachments；
   - assistant placeholder（streaming）；
   - `rewind::begin_turn_scope`；
   - session timestamps/revision；
   - commit。
4. 不累计 usage/billing（没有最终 usage，不写假 0）。
5. 复用 `replace_parts` / `next_session_sequence` / `attachment::insert_attachments`，不复制 SQL。
6. `Persistence::begin_steer_turn` 包装，与 `begin_tool_call` 同风格的同步 DB 方法。

**测试**

```bash
cargo test db::chat::steer
```

覆盖：

- 成功：旧 assistant complete + stop_reason；新 user/assistant 同事务；turn_index/sequence 连续；attachments 落库；rewind scope 切换。
- 旧 assistant 非 streaming → `MessageNotStreaming`，无半行。
- workspace/session 不匹配 → 失败，无半行。
- 重复调用 → 失败（`MessageNotStreaming`）。
- 图片附件归档与 `begin_turn_with_attachments` 同口径。

**完成标准**：事务失败可回滚；无 schema 变更；`SCHEMA_VERSION` 仍为 25。

**依赖**：无（可与 T0.1 并行）。

---

## 阶段 1：app 提交、撤回、终态

### T1.1 提交 steer

**文件**

- `src/app/update/chat/steer.rs`（新增）
- `src/app/update/chat/mod.rs`
- `src/app/update/chat/tests/steer.rs`（新增）

**关键点**

1. 抽出普通发送与 steer 共用的准备 helper（`prepare_native_draft`）：
   - `unknown_slash_command`；
   - `prepare_user_message`；
   - 图片模型支持；
   - skill 提示；
   - `ModelSnapshot` 冻结。
   - 只返回纯数据，不改状态。
2. `try_send` 在 `queue_chat_while_shell_runs` 后、busy 判断前调用 `steer::try_submit`。
3. `try_submit` 校验：
   - `PaneView::Lx`、`Entry::Native`、非 shell；
   - `RequestState::Streaming` 且 `key.session == chat.session_id`；
   - 非空草稿；
   - 以 `/` 开头的草稿不 steer：现有 slash 路径优先；走到这里则提示并保留草稿；
   - pending 数量/图片总预算；
   - 模型/workspace 有效。
4. 成功后：
   - `pending_steers.push(PendingSteer { .. })`；
   - `stop_armed_at = None`；
   - `lx.clear()`；
   - `refresh_lx_status`。
5. 校验失败保留草稿。

**测试**

```bash
cargo test app::update::chat::tests::steer
```

覆盖：

- Streaming 接受；
- Committing/Finishing/compact/shell/AI Web 拒绝；
- 空草稿、`/` 开头草稿、无模型、无 workspace、图片不支持；
- pending 上限、图片总预算；
- 多窗格同会话只有 owner 窗格可 steer；
- 成功后输入清空，失败后草稿保留。

**完成标准**：`try_send` 普通发送路径不回归（现有 chat tests 全过）。

**依赖**：T0.1。

---

### T1.2 单次 `Esc` LIFO 逐条撤回 / 拉回输入框

**文件**

- `src/app/update/chat/steer.rs`
- `src/app/update/chat/mod.rs`
- `src/app/update/pane.rs`
- `src/app/update/chat/tests/steer.rs`

**关键点**

1. `steer::recall_last(state, pane)`：
   - 仅 `Streaming`；
   - 按后进先出（`pending_steers.pop()`）弹出最新一条；
   - 调用 `target.lx.end_history_browse()` 与 `target.lx.import_full_text(&pending.text)` 直接覆盖回显到输入框（覆盖前草稿自动压入编辑器 `undo` 栈，绝不将多条 steer 用 `"\n\n"` 合并拼接，也不提前写入持久化 `prompt_history`）；
   - `stop_armed_at = None`；
   - `refresh_lx_status`。
2. `escape_stop`（`src/app/update/chat/mod.rs`）：
   - 当 `ask` 面板未打开时，优先调用 `steer::recall_last`；队列非空时单次 `Esc` 即回退一条并返回 `true`；
   - 仅当 `pending_steers` 排空后，才进入原有的双击 `Esc` 停止生成逻辑。
3. 移除提示区 `×` 按钮鼠标命中与空输入框 `Backspace` 撤回。

**测试**

```bash
cargo test --lib app::update::chat::tests::steer
```

覆盖：

- 流中连续按 `Esc` 按 LIFO 依次回显 `"third"` → `"second"` → `"first"` 且不合并；
- 队列排空后再按两次 `Esc` 才触发停止流；
- 空输入框与非空输入框按 `Backspace` 均不撤回 pending steer；
- `Committing`/`Finishing` 不可撤回。

**完成标准**：不新增全局快捷键；单次 `Esc` 逐条回退与双击 `Esc` 停流干净解耦。

**依赖**：T1.1。

---

### T1.3 终态分流：正常完成提升 / 取消失败退回

**文件**

- `src/app/update/chat/steer.rs`
- `src/app/update/chat/mod.rs`
- `src/app/update/chat/session.rs`
- `src/app/update/workspace.rs`
- `src/app/update/chat/tests/steer.rs`

**关键点**

1. `on_finish_ack`（位于 `src/app/update/chat/steer.rs`）在设置 `Idle` 前读取 `Finishing.reason`。
2. 成功 + `Complete` + pending 非空：
   - `build_batch`；
   - 优先当前有效 selection，为 `None` 时回退 `batch.model`；新模型不支持图片则退回输入框并提示；
   - 无 workspace/session 时退回输入框并提示；
   - `promoting_steers = batch.consumed`；
   - `RequestState::Committing { generation, draft: batch.text }`；
   - push `Effect::BeginChatTurn { draft, attachments: batch.attachments, model, .. }`。
3. `on_turn_committed`：
   - 若 `promoting_steers > 0`：`pending_steers.drain(..promoting_steers)`，归零；**不清输入框**；
   - 否则走现有清草稿逻辑。
4. `on_turn_failed`：
   - 若 `promoting_steers > 0`：归零并 `restore_pending_to_input`（提升失败不自动重试）；
   - 普通发送失败：pending 不动。
5. 成功 + 非 `Complete`，或 `success == false`：
   - `restore_pending_to_input`：按顺序对每条 pending 调用 `import_full_text` 压入编辑器 `undo` 栈并最终回显最新一条（绝不将多条拼接合并，也不丢带附件的前序条目），不自动发送；
   - `promoting_steers = 0`。
6. `on_finish_ack` 顺序：`Idle` → `promote_after_finish`（若启动下一轮）→ 未启动时才 `user_shell::on_user_shell_terminal` → `request_report_drain_if_idle`。pending steer 优先于排队项。
7. `new_session` / `on_session_loaded_full(session_changed)` / `delete_session`：
   - 统计 pending，清空，toast `Discarded N undelivered steer(s)`；
   - 工作区级联删除关闭工作区时保留未展示的 toast 至全局 `state.toast`；
   - `reset_session_view` 兜底清空。

**测试**

```bash
cargo test app::update::chat::tests::steer
cargo test app::update::chat::tests::session
```

覆盖：

- 正常完成 + pending → `BeginChatTurn` effect，draft/attachments/model 正确；
- `on_turn_committed` 消费 pending、不清用户新草稿；
- `BeginChatTurn` 失败 → pending 退回输入框、`promoting_steers` 归零、不自动重试；
- Esc/失败 → 输入框恢复、顺序正确、不自动发送；
- 切换会话/`/new` → pending 清空 + toast；
- 空 pending 无副作用。

**完成标准**：终态路径互斥；不出现 pending 双重消费；用户草稿不被覆盖。

**依赖**：T1.1、T0.2。

---

## 阶段 2：event + agent loop

### T2.1 `AgentHost` 新请求

**文件**

- `src/chat/agent_loop/mod.rs`
- `src/event/chat/host.rs`
- `src/chat/agent_loop/tests.rs`

**关键点**

1. 定义 `SteerInput { text, images, skill_blocks }`。
2. `AgentHostRequest::SteerInputs { reply }` 与 `HasPendingSteers { reply }`。
3. `AgentHost::take_steer_inputs` 默认返回 `Ok(Vec::new())`；`has_pending_steers` 默认 `false`。
4. `AgentHostEvent::SteerInputs { key, reply }` 与 `HasPendingSteers { key, reply }`。
5. `EventHost::send` 映射；发送失败 drop reply，loop 按 `Cancelled`。
6. exec host 兼容：`src/chat/agent/exec/runner.rs`、`src/chat/agent/exec/subagent/turn.rs` 的穷举 match 增加分支：
   - `SteerInputs` → `Ok(Vec::new())`
   - `HasPendingSteers` → `false`
7. 测试 fake host（`agent_loop/tests.rs`、`tests/limits.rs`、`event/chat/tests/mod.rs`）补 match 分支。
8. fake host 测试可返回脚本化 steer / pending 状态。

**测试**

```bash
cargo test chat::agent_loop
cargo test event::chat::tests
```

**完成标准**：现有 host 请求/回执测试全过；默认实现不影响旧测试。

**依赖**：无（可与 T0 并行）。

---

### T2.2 event `deliver`

**文件**

- `src/event/chat/steer.rs`（新增）
- `src/event/chat/mod.rs`
- `src/event/chat/tests/steer.rs`（新增）

**关键点**

1. `deliver(state, streams, persistence, key, reply)` 按 `design.md §7.4` 顺序实现。
2. `HasPendingSteers` 只读分支：key 有效且 pending 非空 → true。
3. 校验 `active_assistant_id`、`db_workspaces`、`frozen`、pending。
3. `build_batch`；图片模型不支持 → `Failed`，pending 保留。
4. `close_stream(Complete)` 后取 parts。
5. `persistence.begin_steer_turn` 同步调用。
6. 成功：
   - 消费 `batch.consumed`；
   - 旧 assistant 视图 complete + duration；
   - push user + assistant 占位；
   - `RequestState::Streaming` 新 assistant id；
   - `turn_started_at_ms`、`turn_elapsed_ms`、`stop_armed_at`、`follow_bottom`、`reset_step_usage`；
   - `revision`、`session_id`；
   - prompt history（无附件）；
   - `recap.on_turn_completed(旧 turn_index)`；
   - `refresh_lx_status`。
7. 失败：`HostError::Failed { code: "steer turn begin failed · ..." }`，不消费 pending。
8. 回执 `Vec<SteerInput>`（当前批次一条）。
9. `handle_agent_host_event` 增加分支。
10. **mutation 归属更新**（`design.md §5.4`）：
    - `HostRecorderFactory.user_message_id` 改 `Arc<RwLock<String>>`；
    - `ChatStreams.mutation_users` 保存同一单元；
    - `start_stream` 注入；
    - `deliver` DB 成功后原地写入 `committed.user_message_id`；
    - `ChatStreams::finish` 清理。

**测试**

```bash
cargo test event::chat::steer
```

覆盖：

- 空 pending → `Ok(empty)`；
- 成功 → assistant id 切换，新消息顺序正确；
- DB 失败 → `Failed`，pending 不消费；
- 图片模型不支持 → `Failed`；
- 旧 assistant 视图 complete；
- prompt history 仅无附件记录；
- 后续 `BeginTool` 使用新 assistant id（与 `begin_tool` 测试联动）；
- steer 后的文件 mutation 使用新 `user_message_id`（recorder 测试）；`/rewind` 到 steer 消息能覆盖 steer 后的文件修改。

**完成标准**：DB 成功前不消费 pending；DB 失败可重试；不产生半截 UI。

**依赖**：T0.2、T2.1。

---

### T2.3 loop 注入点

**文件**

- `src/chat/agent_loop/mod.rs`
- `src/chat/agent_loop/context.rs`
- `src/chat/agent_loop/tests.rs`

**关键点**

1. `can_drain_steer = false`；第一次 `stream_one` 返回后置 true。
2. `awaited_steer = false`；最终分支 `has_pending_steers` 为真时置 true 并 `continue`；下一次 `take_steer_inputs` 为空则直接收轮（用户已撤回），不空跑模型。
3. loop 顶部顺序：cancel → user_shell_updates → `maybe_auto_compact` → `take_steer_inputs` → `stream_one`。
4. 最终分支（无工具调用、Final/None）：只调 `has_pending_steers`；有则 `continue`，回到顶部按「压缩 → 注入」顺序执行；无则 Complete。
5. `context.push_steer_input`：
   - `turns` 追加 user；
   - `items` 追加 `UserText` 或 `UserContent { images }`；
   - skill blocks 追加 `UserText`。
5. 取消/失败映射沿用现有 `TurnSummary`。

**测试**

```bash
cargo test chat::agent_loop
```

覆盖：

- 首次采样前不注入；
- 工具批处理完成后注入；
- auto-compact 完成后注入（用 fake host 记录调用顺序）；
- Final 后 `has_pending_steers` 为真 → `continue`，下一次顶部先压缩再注入；
- 等待期间用户撤回 pending → `take` 为空，直接收轮，不额外采样；
- 取消/失败停止；
- 多批 steer 顺序正确；
- 图片内容进入 `ModelItem::UserContent`；
- skill blocks 紧随 steer。

**完成标准**：不改变现有无 steer 的请求序列；所有旧 loop 测试全过。

**依赖**：T2.1、T2.2。

---

## 阶段 3：UI

### T3.1 pending 提示区布局与绘制

**文件**

- `src/ui/main_content/lx/steer_strip.rs`（新增）
- `src/ui/main_content/lx.rs`
- `src/ui/main_content/lx/tests.rs`
- `src/ui/text.rs`

**关键点**

1. `Layout` 增加 `pending: Option<Rect>`。
2. `layout`：
   - 有 pending 时高度 = `min(len, 3)`；
   - 位置在 chat bar 上方；
   - `history_bottom` / `panel_base` / `top_limit` / `history_rect` 使用 pending 上沿，`history_wheel_region` 仍到输入框上沿；
   - task/ask 打开时隐藏 pending。
3. `steer_strip::height/draw`：
   - 左右内边距 `INDENT = 1`（`start_x = rect.x + 1`，与下方 `chat_bar` 左上角 `loading` 严格对齐）；
   - 单行 ` Steering  <截断文本>  · 1 attachment / N attachments`（无右侧 `×` 关闭按钮）；
   - 文本折叠空白/控制字符；
   - 超过 3 条末行显示 ` +N more pending`；
   - 不进入历史复制。
4. `src/ui/text/steer.rs`（由 `src/ui/text.rs` 重导出）新增英文常量与单复数格式化函数。
5. `build_chat` 调用 `steer_strip::draw`。
6. 被 steer 截断的旧 assistant 隐藏底部统计行（`snapshot.rs` `followed_by_steer`）；steer 用户消息气泡与 sticky 置顶行渲染柔和暗海蓝底色 `Color::Rgb(0x1b, 0x2a, 0x38)`；`@` 面板 `Subagents` 列表条目按存活状态与内置/自定义角色分色。

**测试**

```bash
cargo test --lib ui::main_content::lx::steer_strip
cargo test --lib slash_panels
```

---

## 阶段 4：验证与收尾

### T4.1 精确验证

```bash
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test --lib steer
cargo test --lib event::chat::subagent
cargo test --lib slash_panels
```

### T4.2 回归检查

- `try_send` 普通发送路径（空闲 Enter、shell 命令、`/rename`、shell 运行中聊天入队、图片/技能发送）。
- `on_finish_ack` 普通路径（无 pending）不变化。
- `/new`、`/resume`、删除会话不回归。
- 历史渲染、`/rewind`、`/compact`、recap 与 TUI 子 Agent 委派不回归。

### T4.3 用户手工确认（TUI）

Agent 无法交互式验证 TUI；以下由用户 `cargo run` 确认：

1. 流中按 Enter，输入框上方出现与 `loading` 左对齐的 `Steering` 提示（无 `×` 按钮），历史不立即变化。
2. 下一次工具边界后，提示消失，历史出现暗海蓝底色的 steer 用户消息，被截断的上一段 assistant 不显示底部统计，助手继续输出。
3. 多条 steer 在同一安全边界合并成一条用户消息。
4. 流中按单次 `Esc` 按 LIFO 依次撤回并回显到输入框（绝不合并），排空后再按两次 `Esc` 才停止当前流。
5. Esc 停止后 pending 回到输入框（前序条目可通过 `Ctrl+Z` 回退），不自动发送。
6. 正常完成后 pending 自动成为下一轮。
7. 切换会话后旧 pending 不进入新会话，并有 toast。
8. `@` 面板 `Subagents` 列表中运行中/空闲实例、内置角色与自定义角色显示不同颜色。

---

## 完成定义

- 所有任务测试通过；`cargo clippy --all-targets -- -D warnings` 无警告。
- 无 schema 变更；`SCHEMA_VERSION` 仍为 25。
- 无 `unwrap()` / `expect()` 新增到生产代码。
- `app/` 不新增 IO；`ui/` 不修改状态。
- 用户可见文案全部英文。
- 用户手工确认 8 项通过。
