# `lx` 流中 Steer（Mid-turn User Input）代码审查报告（`review.md`）

- **审查范围**：`feat/lx-steer` 工作区（`/Users/yonah/projects/agent/lx-tui/.worktrees/lx-steer`）相对基线 `95cf9482` 的全部新增与修改代码、测试及特性文档。
- **严重度分级**：`P0`（崩溃 / 数据损坏 / 安全越权）→ `P1`（核心功能缺陷 / 上下文或用户输入丢失 / 账单统计错乱）→ `P2`（边界条件漏洞 / 状态一致性风险 / 工程规范超限）→ `P3`（细节打磨 / 死代码清理 / 文档同步）。

---

## P0（阻塞级：崩溃 / 数据损坏）

- **无 `P0` 问题**：生产代码无 `unwrap()` / `expect()` / `panic!`，`Database::begin_steer_turn` 采用单事务原子提交（写旧 assistant 最终 parts → 封口旧 assistant `stop_reason='steer'` 与文件修改 scope → 插入新 `user` 与 `streaming` `assistant` → 开启新文件修改 scope → 递增 `revision`），任一步失败整体回滚，不会产生半轮脏数据。

---

## P1（严重功能缺陷 / 输入或上下文丢失）

### P1-1：`event::chat::steer::deliver` 遗漏 `@[refer-subagent](...)` 引用块注入
- **关键点**：`src/event/chat/steer.rs:217-222`（对比普通开轮 `src/event/chat/mod.rs:592-629`）
- **错误原因**：
  普通轮次 `start_stream` 在构造注入模型的 `skill_blocks` 时，会同时调用 `skills::reference_blocks(&draft_text)` 与 `subagent::snapshot::subagent_reference_blocks(&draft_text, &effective_subagents, &pane.lx.agents.runs)`，将用户通过 `@` 面板 `Subagents` 页签点名的存活/历史子 Agent 实例或角色规格（`<subagent_reference>`）注入本轮上下文。而 `event::chat::steer::deliver` 仅调用了 `super::skills::reference_blocks(&text)`，漏掉了 `subagent_reference_blocks`，导致用户在流中 steer 点名 `@subagent` 时模型完全收不到 `<subagent_reference>` 上下文块。
- **修复方案**：
  在 `src/event/chat/steer.rs` 中按与 `start_stream` 相同的受信任工作区口径解析 `effective_subagents` 与当前窗格 `pane.lx.agents.runs`，调用 `super::subagent::snapshot::subagent_reference_blocks(&text, &effective_subagents, &runs)` 并追加到 `skill_blocks`。

### P1-2：`run_agent_turn` 在注入 Steer 切换到新 Assistant 后未重置单步 `usage`
- **关键点**：`src/chat/agent_loop/mod.rs:554-565`
- **错误原因**：
  `run_agent_turn` 中的局部变量 `usage` 记录当前 assistant 消息最后一次模型响应的上下文 Token 用量（最终落库到当前 `assistant_id` 的 `input_tokens / output_tokens / cache_read_tokens`），`billing` 则跨步累加整轮账单。当 `host.take_steer_inputs().await` 返回非空 `inputs` 时，事件层 `deliver` 已将上一段 assistant 封口并切换到新的 `assistant_message_id`（UI 侧也执行了 `reset_step_usage()`），但 `run_agent_turn` 未将 `usage` 归零。若新 assistant 的首次 `stream_one` 在收到任何 usage 增量前被用户取消（`StopReason::Cancelled`）或网络报错（`StopReason::Failed`），`TurnSummary` 会把上一段已被截断的旧 assistant 的 `usage` 错误写入新 assistant 消息。
- **修复方案**：
  在 `src/chat/agent_loop/mod.rs` 的 `take_steer_inputs` 分支中，当 `!inputs.is_empty()` 时重置 `usage = Usage::default()`（保留 `billing` 继续跨段累计整轮账单）。

### P1-3：`restore_pending_to_input` 丢弃非末条带附件的 Steer，且 `recall_last` 过早将未发送草稿写入持久化 `prompt_history`
- **关键点**：`src/app/update/chat/steer.rs:166-225`
- **错误原因**：
  1. `restore_pending_to_input` 通过 `std::mem::take` 清空全部 `pending_steers` 后，只将最后一条调用 `import_full_text(&last.text)` 回显到输入框，前序条目仅在 `item.attachments.is_empty()` 时写入 `prompt_history`。一旦前序 steer 含有 `@file`、`@folder`、`@skill` 或图片标记（`attachments` 非空），在流取消或失败恢复时既不在输入框、也不在编辑器 `undo` 栈或 `prompt_history` 中，直接静默丢失，违反“用户输入不丢”不变量。
  2. `recall_last` 在用户按单次 `Esc` 将尚未发送落库的 steer 撤回输入框修改时，立即写入了 `state.prompt_history` 并派发 `Effect::RecordPromptHistory`，违反“仅在 DB 确认入库后才写入持久化发送历史”的契约。
- **修复方案**：
  1. `recall_last` 弹出最新一条 `pending.text` 并调用 `target.lx.import_full_text(&text)` 覆盖回显到输入框（`import_full_text` 内部自动调用 `self.snapshot()` 将覆盖前的输入框内容压入编辑器 `undo` 栈，绝不拼接合并，也不污染未发送的持久化 `prompt_history`）。
  2. `restore_pending_to_input` 按顺序对每条 `pending` 依次调用 `target.lx.import_full_text(&item.text)`，使所有前序条目（无论是否含 `@` 附件）完整压入编辑器 `undo` 栈，最终输入框停留回显最新一条（同样不将多条合并拼接）。

---

## P2（边界条件漏洞 / 状态一致性 / 工程规范）

### P2-1：`event::chat::steer::deliver` 缺少 `pane.lx.chat.session_id == Some(key.session)` 归属校验
- **关键点**：`src/event/chat/steer.rs:42-48, 139`
- **错误原因**：
  `active_assistant_id(state, &key)` 仅检查 `pane.lx.chat.request` 的 `key`，未核对 `pane.lx.chat.session_id`。`has_pending` 校验了 `pane.lx.chat.session_id.as_deref() == Some(key.session.as_str())`，但 `deliver` 遗漏了该检查。若窗格会话已切换而旧流恰在收尾前请求 `SteerInputs`，`deliver` 可能在空队列时误回 `Ok(Vec::new())` 或在非空时把 `pane.lx.chat.session_id` 改写回旧会话。
- **修复方案**：
  在 `event::chat::steer::deliver` 入口处校验 `pane.lx.chat.session_id.as_deref() == Some(key.session.as_str())`；不匹配时立即回复 `Err(HostError::Cancelled)` 并返回 `false`。

### P2-2：`build_batch` 在图片去重前检查数量与字节上限，导致含重复图片的 Steer 被过早拆批
- **关键点**：`src/app/state/lx/chat/steer.rs:80-90, 104-112`
- **错误原因**：
  `build_batch` 合并多条 `PendingSteer` 时会按 `existing.bytes == image.bytes` 对 `model_images` 去重，但预算门控直接用去重前的 `batch.model_images.len() + next.model_images.len() > MAX_ATTACHED_IMAGES` 与 `batch.image_bytes() + next.image_bytes() > MAX_PENDING_IMAGE_BYTES` 判断。当两条 steer 引用了同一张图片（如第 1 条 3 张图、第 2 条复用其中 1 张 + 新增 1 张，去重后共 4 张）时，会被误判为 `3 + 2 > 4` 而拆批。
- **修复方案**：
  在 `build_batch` 循环中先筛选出 `next.model_images` 相对当前 `batch.model_images`（及 `next` 自身）的新增不重复图片列表与增量字节数，再与 `MAX_ATTACHED_IMAGES`（4 张）和 `MAX_PENDING_IMAGE_BYTES`（40 MiB）比较。

### P2-3：`rebuild_steered_user_messages`、`followed_by_steer` 受流中 `user_shell` 消息干扰，且子 Agent 分叉历史加载漏调重建
- **关键点**：
  - `src/app/state/lx/chat.rs:971-986`
  - `src/ui/main_content/lx/history/snapshot.rs:887-891`
  - `src/event/chat/subagent/launch.rs:113-116`、`src/app/update/chat/subagent.rs:651-655`
- **错误原因**：
  1. `rebuild_steered_user_messages` 仅检查 `message.role != "user"`，未排除 `message.kind != "chat" || message.shell.is_some()`。在旧库兼容分支（前一条消息为 `assistant` 且 `stop_reason == "steer"`）下，若流中执行过本地 `!` 命令（`user_shell`），紧跟在被截断 assistant 后的 `user_shell` 会被误标为 steer 用户消息，而随后的真实 steer 用户消息漏标。
  2. `snapshot.rs` 中判断被截断 assistant 是否隐藏底部统计行的 `followed_by_steer` 仅取 `source.messages.get(message_index + 1)`；若中间夹了流中执行的 `user_shell` 消息，`followed_by_steer` 变为 `false`，导致被截断 assistant 重新露出底部统计行。
  3. 子 Agent 使用 `fork_context: true` 继承父会话历史并在 `Agents` 面板展示时，`launch.rs` 与 `on_subagent_loaded` 未调用 `rebuild_steered_user_messages`。
- **修复方案**：
  1. `rebuild_steered_user_messages` 仅针对普通聊天用户消息（`message.role == "user" && message.kind == "chat" && message.shell.is_none()`），且向前查找最近一条非 `user_shell` 消息判定 `stop_reason == "steer"`。
  2. `snapshot.rs` 中 `followed_by_steer` 跳过紧邻的 `user_shell` 消息后检查下一条聊天消息是否为 `is_steered_user_message`。
  3. 在 `src/event/chat/subagent/launch.rs` 与 `src/app/update/chat/subagent.rs::on_subagent_loaded` 中补充调用 `rebuild_steered_user_messages`。

### P2-4：`promote_after_finish` 未实现 `batch.model` 回退，且工作区级联删除会话时 Toast 落入即将销毁的窗格
- **关键点**：`src/app/update/chat/steer.rs:257-266`、`src/app/update/chat/session.rs:345`
- **错误原因**：
  1. `PendingSteer.model` 与 `SteerBatch.model` 冻结了提交时的 `ModelSnapshot` 作为本轮结束后提升为下一轮的回退模型，但 `promote_after_finish` 只读取了 `target.lx.chat.effective_selection(&state.models)`，未在 `None` 时回退到 `batch.model`。
  2. 工作区历史级联删除会话时，`on_session_deleted` 调用 `discard_for_session_change` 将 `Discarded N undelivered steer(s)` 写入即将被 `close_workspace` 销毁的窗格 `pane.lx.toast`。
- **修复方案**：
  1. `promote_after_finish` 在 `effective_selection` 为 `None` 时使用 `batch.model` 构造 `ModelSelection` 解析 `FrozenModel`（解析成功时同步写回 `target.lx.chat.selection`）。
  2. 在 `close_workspace` 移除工作区前，若被移除窗格存有未显示的 `lx.toast` 且当前无全局 Toast，将其提升至全局 `state.toast`（或仅对存活窗格显示窗格内 Toast）。

### P2-5：4 个生产代码文件因新增 Steer 逻辑超过 1000 行规范上限
- **关键点**：
  - `src/db/chat/mod.rs`（1113 行 > 1000 行）
  - `src/ui/text.rs`（1035 行 > 1000 行）
  - `src/app/update/chat/mod.rs`（1033 行 > 1000 行）
  - `src/app/state/lx/chat.rs`（1026 行 > 1000 行）
- **错误原因**：
  `AGENTS.md` 与 `docs/standards/code-writing-standards.md` 规定生产代码单文件上限为 1000 行。新增 `begin_steer_turn`、steer 文案常量、终态分流与 `steered_user_messages` 方法直接追加在已接近 1000 行的模块根文件中，导致超限。
- **修复方案**：
  - 将 `Database::begin_steer_turn` 拆入 `src/db/chat/steer.rs`；
  - 将 steer 相关文案常量与格式化函数拆入 `src/ui/text/steer.rs`（`src/ui/text.rs` 重导出保持调用方路径不变）；
  - 将 `LxChat` 的 steer 辅助方法移入 `src/app/state/lx/chat/steer.rs`；
  - 将 `src/app/update/chat/mod.rs` 中的 steer 辅助分支收敛至 `src/app/update/chat/steer.rs`，确保上述 4 个生产文件全部回到 1000 行以内。

---

## P3（细节打磨 / 死代码清理 / 文档同步）

### P3-1：单附件提示文案 `1 attachments` 单复数语法修正与死导出清理
- **关键点**：`src/ui/text.rs:324-326`、`src/app/update/chat/mod.rs:89-92`
- **错误原因**：
  1. `lx_steer_attachments(1)` 返回 `"1 attachments"`，英文单复数不严谨。
  2. 移除鼠标点击 `×` 与空输入 `Backspace` 撤回后，`src/app/update/chat/mod.rs` 仍保留了未使用的公开重导出 `recall_last_lx_steer`。
- **修复方案**：
  1. `lx_steer_attachments(count)` 在 `count == 1` 时返回 `"1 attachment"`，其余返回 `"{count} attachments"`。
  2. 清理 `src/app/update/chat/mod.rs` 中未使用的 `recall_last_lx_steer` 导出。

### P3-2：`chat_user_steer_bg` 复用 `style/lx.rs` 命名色板常量
- **关键点**：`src/ui/style.rs:755-757`、`src/ui/style/lx.rs:357`
- **错误原因**：
  `chat_user_steer_bg` 直接写死 `Color::Rgb(0x1b, 0x2a, 0x38)`，与 `src/ui/style/lx.rs` 中 `SESSION_TITLE_PALETTE` 的暗海蓝底色重复定义。
- **修复方案**：
  在 `src/ui/style/lx.rs` 定义 `pub const LX_STEER_USER_BG: Color = Color::Rgb(0x1b, 0x2a, 0x38);` 并在 `SESSION_TITLE_PALETTE` 与 `chat_user_steer_bg()` 中共用。

### P3-3：同步更新 `docs/feature/lx-steer/design.md` 与 `docs/feature/lx-steer/task.md`
- **关键点**：`docs/feature/lx-steer/design.md`、`docs/feature/lx-steer/task.md`
- **错误原因**：
  经过后续交互迭代（移除 `×` 按钮与空输入 `Backspace`、改为单次 `Esc` LIFO 逐条覆盖回显、禁止多条合并、`Steering` 左缩进 1 列对齐 `loading`、Steer 用户消息暗海蓝底色 `#1b2a38`、隐藏被截断旧 Assistant 底部统计行、TUI 子 Agent `SteerInputs` 分流、`@` 面板 `Subagents` 按类型与状态分色），`design.md` 与 `task.md` 仍保留初版描述。
- **修复方案**：
  全量更新 `docs/feature/lx-steer/design.md`、`docs/feature/lx-steer/task.md` 与 `docs/feature/lx-steer/verification.md`，使三份文档与最终代码实现完全一致。
