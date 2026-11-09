# lx 流中 steer 任务拆解

> 前置：`design.md` 已获用户确认后才开始编码。
> 纪律：每个任务完成即跑该任务列出的精确测试；不做全量重复验证；不修改 Git 状态（除用户明确授权）。

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

### T1.2 撤回 / 拉回输入框

**文件**

- `src/app/update/chat/steer.rs`
- `src/app/update/pane.rs`
- `src/app/update.rs` 或 `src/app/update/chat/mod.rs`（`recall_lx_steer` 薄封装）
- `src/app/update/chat/tests/steer.rs`

**关键点**

1. `steer::recall(state, pane, index)`：
   - 仅 `Streaming`；
   - 从 `pending_steers` 移除指定项；
   - 文本按 Q3 规则合并回输入框：
     - 输入为空：`pending.text`；
     - 输入非空：`pending.text + "\n\n" + input`；
   - `import_full_text`；
   - `refresh_lx_status`。
2. `steer::recall_last` 取最后一条。
3. `apply_lx_input` 在面板处理之后、主编辑 match 之前：
   - 条件：`Backspace` + 空输入 + cursor 0 + 无选区 + Chat + 非 shell + 无 md 面板 + `Streaming` + pending 非空；
   - 调 `recall_last` 并 return。
4. 鼠标 `×` 命中后调 `recall`（UI 阶段 T3.2 接线）。
5. 撤回与生效竞争：事件循环串行，先到者生效；撤回后 pending 不存在则无动作。

**测试**

```bash
cargo test app::update::chat::tests::steer
cargo test app::update::tests
```

覆盖：

- 空输入 Backspace 撤回最近一条；
- 有内容时 Backspace 正常删字；
- shell 模式 Backspace 仍退出 shell；
- 已有草稿时前置合并；
- `Committing`/`Finishing` 不可撤回；
- 两次 Backspace 撤回两条。

**完成标准**：不新增全局快捷键；现有输入编辑测试全过。

**依赖**：T1.1。

---

### T1.3 终态分流：正常完成提升 / 取消失败退回

**文件**

- `src/app/update/chat/steer.rs`
- `src/app/update/chat/mod.rs`
- `src/app/update/chat/session.rs`
- `src/app/update/chat/tests/steer.rs`

**关键点**

1. `on_finish_ack` 在设置 `Idle` 前读取 `Finishing.reason`。
2. 成功 + `Complete` + pending 非空：
   - `build_batch`；
   - 优先当前有效 selection，回退 `batch.model`；新模型不支持图片则退回输入框并提示；
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
   - `restore_pending_to_input`：全部 pending 按顺序合并到输入框前面，不自动发送；
   - `promoting_steers = 0`。
6. `on_finish_ack` 顺序：`Idle` → `promote_after_finish`（若启动下一轮）→ 未启动时才 `user_shell::on_user_shell_terminal` → `request_report_drain_if_idle`。pending steer 优先于排队项。
7. `new_session` / `on_session_loaded_full(session_changed)` / `delete_session`：
   - 统计 pending，清空，toast `Discarded N undelivered steer(s)`；
   - `reset_session_view` 兜底清空。
8. 关闭窗格：随状态销毁，无额外逻辑。

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
   - `history_bottom` / `panel_base` / `top_limit` / `history_rect` / `history_wheel_region` 使用 pending 上沿；
   - task/ask 打开时隐藏 pending。
3. `steer_strip::height/draw/recall_hit`：
   - 单行 `Steering  <截断文本>  · N attachments  ×`；
   - 文本折叠空白/控制字符；
   - `×` 最右列；
   - 超过 3 条显示 `+N more pending`；
   - 不进入历史复制。
4. `ui/text.rs` 新增英文常量：
   - `LX_STEER_LABEL`
   - `LX_STEER_MORE`
   - `LX_STEER_ATTACHMENTS`
   - `LX_STEER_DISCARDED`
   - `LX_STEER_SLASH_UNSUPPORTED`
5. `build_chat` 调用 `steer_strip::draw`。

**测试**

```bash
cargo test ui::main_content::lx::steer_strip
cargo test ui::main_content::lx
```

覆盖：

- 高度 0/1/3/超过 3；
- 宽度不足时保 `×`；
- 宽字符不半写；
- history 底部扣减正确；
- task/ask 隐藏；
- 文本截断与附件计数文案。

**完成标准**：不与 chat bar/输入框/面板重叠；现有布局测试全过。

**依赖**：T0.1。

---

### T3.2 鼠标 `×` 命中

**文件**

- `src/ui/main_content/lx/steer_strip.rs`
- `src/ui/main_content/lx.rs`
- `src/event/mouse/press.rs`
- `src/event/tests/`

**关键点**

1. `recall_hit` 只覆盖 `×` 列，返回可见 pending 的原索引。
2. `press.rs` 在 `chat_hit` 前、`text_paste_cancel` 后插入命中分支。
3. 调 `update::recall_lx_steer(state, pane, index)`，置 dirty 并 return。
4. 不拦截文本行其它位置，避免影响历史选区/滚动。

**测试**

```bash
cargo test event::mouse
cargo test ui::main_content::lx::steer_strip
```

覆盖：

- 点击 `×` 撤回对应项；
- 点击文本/空白不撤回；
- 任务页/ask 无命中；
- 坐标边界。

**完成标准**：鼠标点击不穿透到历史/输入框；现有鼠标测试全过。

**依赖**：T1.2、T3.1。

---

### T3.3 输入框 Backspace 接线

**文件**

- `src/app/update/pane.rs`
- `src/app/update/chat/steer.rs`
- `src/app/update/chat/tests/steer.rs`

**关键点**

1. 在处理完所有面板分支之后、主编辑 match 之前判断。
2. 条件见 `design.md §9.4`。
3. 调 `steer::recall_last` 并 return。
4. 不改变 shell 模式光标 0 退出 shell 的现有行为。

**测试**

```bash
cargo test app::update::chat::tests::steer
cargo test app::update::tests
```

**完成标准**：现有输入编辑/mention/slash 测试全过。

**依赖**：T1.2、T3.1（提示区可见性不影响键盘，但需 pending 状态）。

---

## 阶段 4：验证与收尾

### T4.1 精确验证

```bash
cargo fmt
cargo clippy --all-targets -- -D warnings
cargo test app::state::lx::chat::steer
cargo test app::update::chat::steer
cargo test app::update::chat::tests::steer
cargo test db::chat::steer
cargo test event::chat::steer
cargo test chat::agent_loop
cargo test ui::main_content::lx::steer_strip
cargo test event::mouse
```

### T4.2 回归检查

- `try_send` 普通发送路径：
  - 空闲 Enter；
  - shell 命令；
  - `/rename`；
  - shell 运行中聊天入队；
  - 图片/技能发送。
- `on_finish_ack` 普通路径（无 pending）不变化。
- `/new`、`/resume`、删除会话不回归。
- 历史渲染、`/rewind`、`/compact`、recap 不回归。

### T4.3 用户手工确认（TUI）

Agent 无法交互式验证 TUI；以下由用户 `cargo run` 确认：

1. 流中按 Enter，输入框上方出现 `Steering` 提示，历史不立即变化。
2. 下一次工具边界后，提示消失，历史出现普通用户消息，助手继续输出。
3. 多条 steer 合并成一条用户消息。
4. `×` 点击 / 空输入 Backspace 能撤回并回到输入框。
5. Esc 停止后 pending 回到输入框，不自动发送。
6. 正常完成后 pending 自动成为下一轮。
7. 切换会话后旧 pending 不进入新会话，并有 toast。
8. 重启 lx 后已生效 steer 仍在历史，未生效 pending 不保留。

---

## 完成定义

- 所有任务测试通过；`cargo clippy --all-targets -- -D warnings` 无警告。
- 无 schema 变更；`SCHEMA_VERSION` 仍为 25。
- 无 `unwrap()` / `expect()` 新增到生产代码。
- `app/` 不新增 IO；`ui/` 不修改状态。
- 用户可见文案全部英文。
- 用户手工确认 8 项通过。
