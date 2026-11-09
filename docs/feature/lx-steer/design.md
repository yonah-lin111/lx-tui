# lx 流中 steer（mid-turn user input）设计

> 状态：待用户评审，未开始编码。
> 参考：codex `turn/steer`（`TurnSteerParams.input` / pending input queue）、opencode `SessionInbox`（`delivery: steer | queue`）。
> 适用分支：`dev`，主 checkout。
> 需求位置：`src/ui/main_content/lx/`（pending 提示区），核心链路落在 app / event / chat / db。

---

## 1. 目标

在 Native 主会话正在生成（`RequestState::Streaming`）时，允许用户继续在 lx 输入框输入并按 Enter：

1. 草稿不打断当前工具/模型请求，作为 **steer** 进入待生效队列；
2. 在当前 agent loop 的**下一个安全边界**注入：本轮工具全部结算 → 自动压缩（如有）→ 注入 steer → 下一次模型采样；
3. 注入成功后，steer 变成一条**普通用户消息**，并开启一条新的 assistant 消息承载后续输出；重启 lx 后仍在历史里；
4. 注入前，输入框上方显示待生效提示区，可撤回并回到输入框编辑；
5. 本轮正常结束但 steer 还没生效时，自动作为下一轮发送；Esc 停止或失败时退回输入框；切换会话/关闭窗格时丢弃并提示。

## 2. 非目标（本版不做）

- 不做 `queue`（下一轮队列）模式；流中 Enter 只有 steer 一种语义。
- 不做未生效 steer 的跨重启恢复；pending 只存在内存（对齐 codex TUI 的 `pending_steers`，不引入 opencode 的持久化 inbox 表）。
- 不支持音频、远程图片 URL、agent mention；lx 当前没有这些输入类型。
- 不支持对子 agent 详情页 steer；子 agent 仍由模型通过 `send_input` 控制。
- 不支持 AI Web 会话 steer；AI Web 没有本地 agent loop。
- 不新增数据库表/列，不修改 `SCHEMA_VERSION`（保持 25）；只新增一个 DB 事务方法。
- 不给 steer 增加特殊的模型提示词包装；模型收到的就是普通 user 消息，`PROMPT_VERSION` 不变。
- 不做任意已生效消息的“中途编辑”；`/rewind` 仍按现有轮次语义工作。

---

## 3. 已确认的决策（逐条对应盘问）

| # | 决策 | 结论 |
|---|------|------|
| 1 | 流中 Enter 语义 | **真 steer**：在当前轮内、下一个安全边界注入；本轮先结束则自动转下一轮 |
| 2 | 生效后的历史形态 | **普通用户消息** + 后续新的 assistant 消息；重启保留，可复制、可 `/rewind` |
| 3 | 轮次终止时未生效项 | 正常完成 → 自动下一轮；Esc/失败 → 退回输入框，不自动发送 |
| 4 | 未生效项 UI | 输入框上方临时提示区（`Steering` + 文本截断），不进历史 |
| 5 | 多条 steer | 同一个边界按提交顺序合并成一条用户消息，只触发一次模型响应 |
| 6 | 内容范围 | **文本 + 图片 + `@` 文件引用 + 技能**；不支持音频/远程 URL/agent mention |
| 7 | 允许状态 | 仅 Native 主会话 `Streaming`；`Committing`/`Finishing`/`/compact`/shell/AI Web 不 steer |
| 8 | 带附件合并 | 与文本一样合并；超过单条上限时只发第一条，其余留到下一个边界 |
| 9 | 撤回/编辑 | v1 支持撤回并拉回输入框编辑；不跨重启 |
| 10 | 撤回具体交互 | 提示行 `×` 鼠标点击；输入框为空时 `Backspace` 撤回最近一条 |
| 11 | 自动压缩 | 等压缩完成后注入，steer 不参与摘要 |
| 12 | 切换会话/关闭窗格 | 丢弃 pending，并 toast 提示；不搬进新会话输入框 |
| 13 | 安全边界 | 本轮工具全部结算之后；绝不在一次模型请求/工具执行中途注入 |

---

## 4. 不变量（实现时必须守住）

1. **只在 `Streaming` 接受 steer**：`RequestState::Streaming { key, .. }` 且窗格视图为 `PaneView::Lx`、`entry == Native`、非 shell 模式。
2. **pending 只存在内存**：`LxChat.pending_steers`；不写 DB、不跨重启、不进入上下文装配。
3. **生效才落库**：DB 成功后 steer 才变成 user message；DB 失败时 pending 保留，本轮按失败收尾。
4. **注入只在安全边界**：一次 `stream_one` 结束且该响应的所有工具调用结算完之后；首次采样前不注入。
5. **先压缩后注入**：顺序固定为 `maybe_auto_compact` → `take_steer_inputs` → 下一次采样。
6. **生效后是普通消息**：关闭当前 assistant 片段（status=`complete`，`stop_reason="steer"`），新 user message + 新 streaming assistant；`turn_index` 递增。
7. **app / ui 边界**：`app/` 只改纯状态并入队效应；`ui/` 只读状态、只写 Buffer；DB/IO 只在 event/db 层。
8. **输入不丢**：正常完成自动下一轮；取消/失败退回输入框；会话切换丢弃必须 toast。
9. **不伪造结果**：关闭 assistant 片段不写假 usage；不把未执行工具标成功。
10. **不改 schema**：不加表/列，不 bump `SCHEMA_VERSION`；只加 `Database::begin_steer_turn` 事务方法。

---

## 5. 数据模型与状态

### 5.1 `PendingSteer`（内存态）

新增 `src/app/state/lx/chat/steer.rs`：

```rust
use std::sync::Arc;

use crate::chat::events::ModelImage;
use crate::db::chat::{MessageAttachmentInput, ModelSnapshot};

/// 一条已提交、尚未注入当前轮的 steer。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingSteer {
    /// 原始输入文本（保留 `@` 引用、skill 标记、图片占位符）。
    pub text: String,
    /// 准备写入数据库的附件（图片/文件/文件夹/skill）。
    pub attachments: Vec<MessageAttachmentInput>,
    /// 准备送给当前模型的图片字节（来自 `PreparedUserMessage::model_images`）。
    pub model_images: Vec<ModelImage>,
    /// 提交时冻结的模型快照；用于本轮结束后的自动下一轮。
    pub model: ModelSnapshot,
}

impl PendingSteer {
    /// 图片数量（按准备给模型的去重图片数；与 `prepare_user_message` 同口径）。
    pub fn image_count(&self) -> usize {
        self.model_images.len()
    }

    /// 已准备图片字节总量（用于 pending 总预算）。
    pub fn image_bytes(&self) -> u64 {
        self.model_images
            .iter()
            .map(|image| u64::try_from(image.bytes.len()).unwrap_or(u64::MAX))
            .sum()
    }
}
```

`LxChat` 新增字段（`src/app/state/lx/chat.rs`）：

```rust
/// 已提交、尚未注入当前轮的 steer（按提交顺序）。
pub pending_steers: Vec<PendingSteer>,
/// 正常结束后正在提升为下一轮的 pending 条数；`on_turn_committed` 据此消费。
pub promoting_steers: usize,
```

- `LxChat::new()`：空 Vec / 0。
- `LxChat::reset_session_view()`：清空并归零（会话级状态不得跨会话继承）。
- 不进入 `ChatMessageView`，不进入 `history::timeline`，不进入 `context::assemble`。

### 5.2 pending 预算

在 `src/app/update/chat/steer.rs` 定义：

```rust
/// 单个窗格最多同时挂起的 steer 条数。
pub(crate) const MAX_PENDING_STEERS: usize = 8;
/// pending steer 图片字节总预算（与单条消息图片预算同一量级，防止内存失控）。
pub(crate) const MAX_PENDING_IMAGE_BYTES: u64 = 40 * 1024 * 1024;
```

- 单条 steer 的文本上限沿用 DB 的 `MAX_MESSAGE_BYTES = 2 MiB`（改为 `pub(crate)` 供 app/event 共用）。
- 单条图片上限仍由 `prepare_user_message` 执行：最多 4 张、单张 20 MiB、单条 40 MiB。
- pending 总数超限：toast 警告，保留草稿，不提交。
- pending 图片总量超限：toast 警告，保留草稿，不提交。

### 5.3 合并规则（`build_batch`）

新增纯函数（放在 `src/app/state/lx/chat/steer.rs`，app 与 event 共用）：

```rust
/// 从 pending 头部尽量合并，返回本批内容与消费条数；不修改输入。
pub struct SteerBatch {
    pub text: String,
    pub attachments: Vec<MessageAttachmentInput>,
    pub model_images: Vec<ModelImage>,
    pub model: ModelSnapshot,
    /// 本批消费了前多少条 pending。
    pub consumed: usize,
}

impl SteerBatch {
    pub fn image_bytes(&self) -> u64 {
        self.model_images
            .iter()
            .map(|image| u64::try_from(image.bytes.len()).unwrap_or(u64::MAX))
            .sum()
    }
}

pub fn build_batch(pending: &[PendingSteer]) -> Option<SteerBatch>;
```

规则（严格按确认决策）：

1. 至少取第 1 条；然后按顺序尝试合并下一条。
2. 文本：`text + "\n\n" + next.text`；后续条目的 `marker_start/marker_end` 全部平移 `前文长度 + 2`。
3. 附件：按条目顺序 append；图片占位标记随文本平移。
4. 图片：合并后去重（按字节）；图片总数超过 4 张、合并文本超过 2 MiB，或合并图片字节超过 `MAX_PENDING_IMAGE_BYTES` 时停止合并。
5. 模型快照取第 1 条（同一轮冻结模型不应变化；debug 断言后续一致）。
6. `consumed` 只包含完整并入本批的条目；未并入的留在 pending，等下一个边界。
7. 单条自身已由提交路径校验；`build_batch` 不会截断，超限只会少合并。

```rust
// 关键实现骨架
let mut batch = SteerBatch::from(&pending[0]);
let mut consumed = 1usize;
for next in &pending[1..] {
    let add = 2 + next.text.len();
    if batch.text.len().saturating_add(add) > MAX_MESSAGE_BYTES {
        break;
    }
    if count_images(&batch.attachments) + next.image_count() > MAX_ATTACHED_IMAGES {
        break;
    }
    if batch.image_bytes().saturating_add(next.image_bytes()) > MAX_PENDING_IMAGE_BYTES {
        break;
    }
    let offset = batch.text.len() + 2;
    let mut text = std::mem::take(&mut batch.text);
    text.push_str("\n\n");
    text.push_str(&next.text);
    batch.text = text;
    for att in &next.attachments {
        let mut att = att.clone();
        att.marker_start = att.marker_start.map(|pos| pos.saturating_add(offset));
        att.marker_end = att.marker_end.map(|pos| pos.saturating_add(offset));
        batch.attachments.push(att);
    }
    for image in &next.model_images {
        if !batch.model_images.iter().any(|existing| existing.bytes == image.bytes) {
            batch.model_images.push(image.clone());
        }
    }
    consumed += 1;
}
Some(SteerBatch { consumed, ..batch })
```

### 5.4 mutation 归属必须跟随当前 user message

这是本设计最容易漏掉的一条：文件 mutation 的 rewind 归属由
`HostRecorderFactory` 在**一轮开始时**绑定的 `user_message_id` 决定（见
`src/event/chat/file_snapshots.rs`）。如果 steer 之后仍沿用旧 id，steer 后的所有
文件修改都会被记到上一段用户消息名下，`/rewind` 会恢复到错误的轮次。

因此：

- `HostRecorderFactory.user_message_id` 改为共享可更新单元：

```rust
// src/event/chat/file_snapshots.rs
pub struct HostRecorderFactory {
    sender: mpsc::UnboundedSender<AppEvent>,
    workspace: PathBuf,
    session_id: String,
    user_message_id: Arc<std::sync::RwLock<String>>,
}

impl MutationRecorderFactory for HostRecorderFactory {
    fn for_call(&self, call_id: &str) -> Arc<dyn MutationRecorder> {
        let user_message_id = self
            .user_message_id
            .read()
            .map(|id| id.clone())
            .unwrap_or_default();
        Arc::new(HostRecorder {
            sender: self.sender.clone(),
            workspace: self.workspace.clone(),
            session_id: self.session_id.clone(),
            user_message_id,
            tool_call_id: call_id.to_string(),
        })
    }
}
```

- `ChatStreams` 增加：

```rust
/// 本轮 mutation 归属的当前 user message id；steer 生效时原地更新。
mutation_users: HashMap<RequestKey, Arc<std::sync::RwLock<String>>>,
```

- `start_stream` 创建 `Arc<RwLock<String>>`，一份交给 `HostRecorderFactory`，一份存入
  `streams.mutation_users`（见 `src/event/chat/mod.rs::start_stream`）。
- `steer::deliver` 在 DB 成功后、回执 loop 前：

```rust
if let Some(current) = streams.mutation_users.get(&key)
    && let Ok(mut id) = current.write()
{
    *id = committed.user_message_id.clone();
}
```

- `ChatStreams::finish` 清理 `mutation_users`。
- 工具执行仍在安全边界之后；`for_call` 是每次工具调用时读取当前 id，因此 steer 后的
  工具自动归到新 user message。

### 5.5 DB：`begin_steer_turn`（无 schema 变更）

新增 `Database::begin_steer_turn`（`src/db/chat/mod.rs`）与 `Persistence` 包装（`src/db/service/chat.rs`）：

```rust
pub fn begin_steer_turn(
    &self,
    workspace: &Path,
    session_id: &str,
    previous_assistant_id: &str,
    previous_parts: &[ChatPart],
    user_text: &str,
    attachments: &[MessageAttachmentInput],
    model: &ModelSnapshot,
) -> Result<TurnCommitted>;
```

事务顺序（必须一个事务，不能拆成 `finish` + `begin_turn`）：

```text
1. ensure_workspace；校验 session 属于 workspace。
2. 校验 previous_assistant：
   SELECT session_id, status FROM agent_message
    WHERE external_id = previous_assistant_id AND session_id = session_id
   且 status = 'streaming'；否则 Err(DbError::MessageNotStreaming / SessionMissing)。
3. 关闭上一段 assistant：
   replace_parts(previous_assistant_id, previous_parts, now)
   UPDATE agent_message
      SET status='complete', stop_reason='steer', completed_at_ms=now
    WHERE external_id=previous_assistant_id AND status='streaming'
   rewind::seal_turn_scope(tx, session_id, previous_assistant_id)
4. 取号：
   next_sequence = next_session_sequence(tx, session_id)
   turn_index    = COALESCE(MAX(turn_index)+1, 0)
   revision      = session.revision + 1
5. 写 user message（role='user', status='complete', sequence=next_sequence）
   写 text part（position=0）
   写附件：attachment::insert_attachments(tx, session_id, user_message_id, attachments)
6. 写 assistant placeholder（role='assistant', status='streaming',
   sequence=next_sequence+1, model_provider/model_id/variant 来自 model）
7. rewind::begin_turn_scope(tx, session_id, user_message_id, now)
8. UPDATE agent_session
      SET updated_at_ms=now, last_message_at_ms=now, revision=revision
    WHERE external_id=session_id
9. commit；返回 TurnCommitted { ... }
```

说明：

- 不累计 usage：上一段没有最终 usage；不写假的 0。
- `stop_reason="steer"` 只出现在已关闭片段的 DB 行；UI 不显示。
- 复用现有 `replace_parts` / `next_session_sequence` / `insert_attachments` / `rewind::seal_turn_scope` / `rewind::begin_turn_scope`，不复制 SQL。
- 当前 `begin_turn_with_attachments` 的标题逻辑不需要（steer 一定已有 session）。
- `MAX_MESSAGE_BYTES` 改为 `pub(crate)`，app/event 共用同一上限。

---

## 6. 提交路径（app 层）

### 6.1 入口

`src/app/update/chat/mod.rs::try_send` 当前顺序：

```text
try_submit_user_shell
try_submit_rename_command
queue_chat_while_shell_runs
收集 context / 判断 busy
busy -> return false（流中 Enter 不发送）
...
```

新增：在 `queue_chat_while_shell_runs` 之后、busy 判断之前调用：

```rust
if steer::try_submit(state, pane) {
    return true;
}
```

`src/app/update/chat/steer.rs::try_submit` 的职责：

1. 只接受 `PaneView::Lx`、`LxEntry::Native`、非 shell 模式；
2. 只接受 `RequestState::Streaming { key, .. }`，且 `key.session == lx.chat.session_id`（`/new` 解绑后的旧 Streaming 不得接受 steer）；
3. 空草稿返回 false（沿用 Enter 无动作）；
4. 以 `/` 开头的草稿不进入 steer：现有 slash 面板/命令路径优先；若走到这里（面板已关闭或命令未消费），给黄色提示并保留草稿。未知 `/` 命令沿用 `unknown_slash_command` 文案；已知命令也不作为 steer 文本发送。
5. `prepare_user_message` 走与普通发送同一条路径：
   - 解析 `@` 引用、图片、skill；
   - 检查模型 `supports_image_input`；
   - skill 截断/读取失败提示沿用 `show_lx_hint`；
6. 检查单条文本 `MAX_MESSAGE_BYTES`、`MAX_PENDING_STEERS` 与 `MAX_PENDING_IMAGE_BYTES`；
7. 冻结 `ModelSnapshot`（从当前有效 selection 解析 `FrozenModel`）；
8. `target.lx.chat.pending_steers.push(PendingSteer { text: prepared.raw_text, attachments: prepared.attachments, model_images: prepared.model_images, model })`；
9. `target.lx.chat.stop_armed_at = None`（用户继续输入即取消停止待命）；
10. `target.lx.clear()`；`refresh_lx_status`；
11. 返回 true。

关键点：

- **清草稿只在 pending 入队成功后**；任何校验失败都保留草稿。
- 不在 app 层发 DB effect；生效由 agent loop 的 host 请求触发。
- 不记录 prompt history（等生效时再记录，避免未生效内容进入 `↑` 历史）。
- `ModelSnapshot` 必须可解析；解析失败按普通发送的 `LX_CHAT_NO_MODEL` 处理。

### 6.2 为什么不在 `try_send` 里直接 `prepare` 再判断

`try_send` 的普通路径会先判断 `busy` 并直接返回，见 `src/app/update/chat/mod.rs`：

```rust
if draft.is_empty() || busy {
    // 流中 Enter 不入队、不造消息。
    return false;
}
```

steer 必须早于这个分支，但又要复用后面的 `prepare_user_message`。实现上抽一个共用 helper（例如 `prepare_native_draft(state, pane) -> Result<PreparedDraft, DraftReject>`），普通发送和 steer 共用；避免复制图片/skill/slash 校验。该 helper 只返回纯数据，不写状态。

---

## 7. 生效路径（event + agent loop）

### 7.1 `AgentHost` 新请求

`src/chat/agent_loop/mod.rs`：

```rust
/// 一次注入当前轮的 steer 内容（已合并）。
pub struct SteerInput {
    pub text: String,
    /// 当前模型可用的图片字节；无图时为空。
    pub images: Vec<crate::chat::events::ModelImage>,
    /// skill 正文块；与 turn 起点 assembly 的注入位置一致。
    pub skill_blocks: Vec<String>,
}

pub enum AgentHostRequest {
    // ...
    SteerInputs {
        reply: oneshot::Sender<Result<Vec<SteerInput>, HostError>>,
    },
    HasPendingSteers {
        reply: oneshot::Sender<bool>,
    },
}

pub trait AgentHost {
    // ...
    fn take_steer_inputs(&mut self)
        -> impl Future<Output = Result<Vec<SteerInput>, HostError>> + Send
    where
        Self: Sized,
    {
        async { Ok(Vec::new()) } // 测试 host 默认无 steer
    }

    fn has_pending_steers(&mut self) -> impl Future<Output = bool> + Send
    where
        Self: Sized,
    {
        async { false }
    }
}
```

`src/event/chat/host.rs`：

- `AgentHostEvent::SteerInputs { key, reply }` 与 `AgentHostEvent::HasPendingSteers { key, reply }`
- `EventHost::send` 把 `AgentHostRequest::SteerInputs` / `HasPendingSteers` 转成事件；
- 发送失败 → drop reply → loop 按 `HostError::Cancelled`（`HasPendingSteers` 按 false）处理（与其它请求一致）；
- **exec host 兼容**：`src/chat/agent/exec/runner.rs` 与 `src/chat/agent/exec/subagent/turn.rs` 的 `handle_host` 是穷举 match，必须新增两个分支并回：
  - `SteerInputs` → `Ok(Vec::new())`
  - `HasPendingSteers` → `false`

  exec / 子 agent 模式不支持用户 steer，但不补分支会编译失败或 panic。

### 7.2 loop 注入顺序

修改 `src/chat/agent_loop/mod.rs::run_agent_turn`：

```rust
let mut can_drain_steer = false; // 首次采样前不注入，避免抢在起始 prompt 前面
let mut awaited_steer = false;   // 最终分支因 pending 而继续；下一次 take 为空则收轮

loop {
    if *cancel.borrow() {
        return TurnSummary::interrupted(usage).with_billing(billing);
    }
    // 现有：本地命令结果
    for text in host.user_shell_updates().await {
        context.push_user_context(text);
    }
    // 现有：自动压缩。必须先于 steer；steer 不参与摘要。
    if let Err(error) =
        maybe_auto_compact(&mut context, &mut host, &stream_one, &cancel).await
    {
        return match error { /* 现有映射 */ };
    }
    // 新增：安全边界注入
    if can_drain_steer {
        match host.take_steer_inputs().await {
            Ok(inputs) if inputs.is_empty() && awaited_steer => {
                // 等待期间用户撤回了 pending：不要再空跑一次模型。
                return TurnSummary::complete(usage).with_billing(billing);
            }
            Ok(inputs) => {
                for input in inputs {
                    context.push_steer_input(input);
                }
                awaited_steer = false;
            }
            Err(HostError::Cancelled) => {
                return TurnSummary::interrupted(usage).with_billing(billing);
            }
            Err(HostError::Failed { code }) => {
                return TurnSummary::failed(code, usage).with_billing(billing);
            }
        }
    }

    let outcome = stream_one(context.next_request(), cancel.clone()).await;
    // ... usage / stop / push_response 不变
    can_drain_steer = true;

    if calls.is_empty() {
        match stop {
            Some(ModelStepStop::Final) | None => {
                // 最终响应后仍有 steer：不在这里注入，回到 loop 顶部，
                // 让 auto-compact 先跑完，再 take_steer_inputs 注入。
                if host.has_pending_steers().await {
                    awaited_steer = true;
                    continue;
                }
                return TurnSummary::complete(usage).with_billing(billing);
            }
            Some(_) => return TurnSummary::failed(UNEXPECTED_STOP_CODE, usage).with_billing(billing),
        }
    }
    // ... 工具批处理不变
}
```

关键点：

- **首次采样前不注入**：`can_drain_steer = false`；第一次 `stream_one` 返回后才置 true。若第一次响应直接 Final 且已有 pending，最终分支只 `continue`，下一轮顶部先压缩再注入。
- **不在工具批处理中途注入**：`take_steer_inputs` 只在 loop 顶部调用；工具循环内的 `begin_tool/finish_tool` 之间没有注入点。
- **先压缩后注入**：`maybe_auto_compact` 在 `take_steer_inputs` 之前；压缩期间到达的 steer 只留在 state，等下一次 loop 顶部。
- **最终分支只探测、不注入**：`has_pending_steers` 不写 DB、不关闭 assistant；回到顶部后仍按「压缩 → 注入」顺序执行，避免刚注入的 steer 被紧接着的压缩摘要吞掉。
- **撤回竞态**：`awaited_steer` 为真但 `take_steer_inputs` 返回空（用户已撤回）时直接收轮，不白跑一次模型。
- 取消/失败时 pending 保留在 state，由终态 handler 按 Q3 处理。

### 7.3 `ModelContext::push_steer_input`

在 `src/chat/agent_loop/context.rs` 新增：

```rust
pub(super) fn push_steer_input(&mut self, input: SteerInput) {
    // turns：旧纯文本视图也要能看到 steer（transports 的 items 为空时才回退）。
    self.request.turns.push(crate::chat::events::ChatTurn {
        role: "user".into(),
        text: input.text.clone(),
        reasoning_metadata: None,
    });
    if input.images.is_empty() {
        self.items.push(crate::chat::events::ModelItem::UserText(input.text));
    } else {
        self.items.push(crate::chat::events::ModelItem::UserContent {
            text: input.text,
            images: input.images,
        });
    }
    // 与 turn 起点 assembly 一致：skill 正文块排在当前用户消息之后。
    for block in input.skill_blocks {
        self.items.push(crate::chat::events::ModelItem::UserText(block));
    }
}
```

- 不修改 `request.tools` / system prompt / model：同一轮内工具与模型冻结不变。
- 不重置 usage/billing：整轮账单继续累计。
- 子 agent loop 复用同一 trait 默认空实现；主轮才由 `EventHost` 真正处理。

### 7.4 event 层处理 `SteerInputs`

新增 `src/event/chat/steer.rs`：

```rust
pub(crate) const STEER_TURN_BEGIN_FAILED_CODE: &str = "steer turn begin failed";
pub(crate) const STEER_IMAGE_UNSUPPORTED_CODE: &str = "steer images unsupported";

pub(crate) fn deliver(
    state: &mut AppState,
    streams: &mut ChatStreams,
    persistence: &Persistence,
    key: RequestKey,
    reply: oneshot::Sender<Result<Vec<SteerInput>, HostError>>,
) -> bool;
```

顺序：

0. `HasPendingSteers` 分支：`active_assistant_id(state, &key).is_some()` 且窗格 `pending_steers` 非空 → `reply.send(true)`；否则 false。只读，不写 DB。
1. `active_assistant_id(state, &key)` 必须匹配当前 `Streaming`；否则 `reply.send(Err(HostError::Cancelled))`，返回 false（不消费 pending）。
2. 从窗格取 `pending_steers`；空则 `reply.send(Ok(Vec::new()))`，返回 false（最常见路径不要求 frozen/workspace）。
3. `build_batch(&pending)`；为空则同上。
4. `streams.db_workspaces.get(&key)` 取 DB workspace；缺失则 `Failed`。
5. `streams.frozen.get(&key.session)` 取冻结模型；缺失则 `Failed`。
6. 若 `batch.model_images` 非空且 `frozen.supports_image_input == false`：`reply.send(Err(HostError::Failed { code: STEER_IMAGE_UNSUPPORTED_CODE }))`，保留 pending。
7. 取当前 assistant 片段 parts：
   - 先在 state 里找到 `assistant_id` 对应消息；
   - `target.lx.chat.close_stream(&StopReason::Complete)` 关闭开放思考/工具（安全兜底）；
   - `chat_message_part_views(message)` 得到 parts。
8. 调 `persistence.begin_steer_turn(...)`：
   - `session_id = key.session`；
   - `previous_assistant_id = assistant_id`；
   - `user_text = batch.text`；
   - `attachments = batch.attachments`；
   - `model = ModelSnapshot` 来自 frozen（不是 pending 里存的那份，防止模型切换残留）。
9. 失败：`reply.send(Err(HostError::Failed { code: format!("{STEER_TURN_BEGIN_FAILED_CODE} · {}", error.code()) }))`；**不消费 pending**；返回 true。
10. 成功：
    - 从 pending 头部移除 `batch.consumed` 条；
    - 旧 assistant 视图置 `status="complete"`，补 `duration_ms`；
    - 取消在途 recap（`recap::cancel_pending_recap`），避免旧回顾贴到推进后的时间线；
    - push 新 user `ChatMessageView`（`committed.user_message_id/sequence/turn_index/attachments`，文本 `batch.text`）；
    - push 新 assistant 占位（`status="streaming"`，`model_label` 用 `state.models.model_display_name`）；
    - `set_message_time(user_message_id/assistant_message_id, committed.created_at_ms)`；
    - `RequestState::Streaming { key, assistant_id: committed.assistant_message_id }`；
    - `turn_started_at_ms = Some(committed.created_at_ms)`、`turn_elapsed_ms = Some(0)`；
    - `stop_armed_at = None`、`follow_bottom = true`、`reset_step_usage()`；
    - `revision`、`session_id` 同步；
    - `prompt_history.record(&batch.text)` + `Effect::RecordPromptHistory`（仅无附件时，与 `on_turn_committed` 同口径）；
    - `recap.on_turn_completed(旧 assistant 的 turn_index)`（只记账，不立即调度）；
    - `refresh_lx_status`。
11. 计算 `skill_blocks = crate::event::chat::skills::reference_blocks(&batch.text)`。
12. `reply.send(Ok(vec![SteerInput { text: batch.text, images: batch.model_images, skill_blocks }]))`。
13. 返回 true（需要下一帧）。

注意：

- 这是与 `BeginTool` 同级的同步 DB 握手；loop 等待 ack 后才继续采样。
- 不要在 handler 里 `await`；DB 是同步调用，与 `begin_tool` 一致。
- 旧 assistant 片段关闭后，`active_assistant_id` 立即指向新 id；后续 `BeginTool`/checkpoint/finish 自动落到新 assistant。
- 旧 assistant 的 parts 必须在关闭前从 state 取全；`close_stream` 只改状态，不改 part 内容。

### 7.5 图片与技能注入

- 图片：`batch.model_images` 直接进入 `ModelItem::UserContent`，不重新读盘；与 `prepare_user_message` 的缩放/去重结果一致。
- 技能：`skills::reference_blocks(&batch.text)` 与 `start_stream` 的 `skill_blocks` 同源；块放在 steer 用户项之后。
- `turns` 只带文本；所有生产 transport 在 `items` 非空时使用 `items`，因此图片不会丢。

---

## 8. 终态与边界

### 8.1 正常完成但 steer 未生效

场景：用户按 Enter 后，loop 的最终分支检查前/检查后极短窗口内 turn 结束。

处理（`src/app/update/chat/mod.rs::on_finish_ack`）：

```text
DB finish ack 成功
  -> RequestState::Idle
  -> 若 Finishing.reason == Complete 且 pending_steers 非空：
       steer::promote_after_finish(state, pane)
         1. build_batch(pending) 取一批（不删除）
         2. target.lx.chat.promoting_steers = batch.consumed
         3. 模型快照：优先当前窗格有效 selection（用户可能在流中切过模型）；
            无有效 selection 时回退 batch.model；新模型不支持图片则把 pending 退回输入框并提示
         4. RequestState::Committing { generation: next_generation, draft: batch.text }
         5. push Effect::BeginChatTurn { pane, generation, workspace, session_id,
                                         draft: batch.text, attachments: batch.attachments,
                                         model }
  -> on_turn_committed：
       if promoting_steers > 0 {
           pending_steers.drain(..promoting_steers);
           promoting_steers = 0;
           // 不清输入框（input 里可能是用户新写的草稿）
       } else { 现有逻辑：匹配冻结草稿才清空 }
  -> on_turn_failed：
       if promoting_steers > 0 {
           promoting_steers = 0;
           // 提升失败：把 pending 退回输入框，避免停在 Idle + pending 的悬空状态
           steer::restore_pending_to_input(state, pane);
       }
       // 普通发送失败：pending 不动（正常不会有）
```

- 若本批因超限只消费了一部分，剩余 pending 会在新轮 `Streaming` 后的下一个安全边界作为 steer 注入；不会丢。
- 迁移路径复用现有 `BeginChatTurn` DB 事务，不新增“提升”专用 DB 方法。
- 提升失败不自动重试，避免 DB 故障时形成发送循环；文本回到输入框由用户决定。
- `on_finish_ack` 的执行顺序：`Idle` → `promote_after_finish`（若成功启动下一轮）→ 只有未启动时才 `user_shell::on_user_shell_terminal` 分派队列 → `request_report_drain_if_idle`。pending steer 优先于排队项。
- `build_batch` 意外返回 None（pending 非空但无法构造）时，直接 `restore_pending_to_input`，不留下悬空 pending。

### 8.2 Esc 停止 / 失败

`on_finish_ack` 成功且 `Finishing.reason != Complete`，或 `on_finish_ack(success=false)`：

```text
steer::restore_pending_to_input(state, pane)
  1. 按提交顺序取出所有 pending（合并文本用空行分隔）；
  2. 若输入框已有草稿：pending 文本放在前面 + "\n\n" + 现有草稿；
  3. lx.import_full_text(merged)；promoting_steers = 0；
  4. 可选 toast：N 条未生效 steer 已退回输入框。
```

- **不自动发送**；用户可编辑后重新 Enter。
- 若 pending 为空，无动作。
- 失败（HTTP/DB）与用户取消同一路径；避免错误重试。

### 8.3 切换会话 / `/new` / 关闭窗格

- `new_session`（`src/app/update/chat/session.rs`）：在 `reset_session_view()` 前统计 pending，清空并 toast。
- `on_session_loaded_full` 的 `session_changed` 分支：清空 pending 并 toast；不搬进新会话输入框。
- 删除当前会话（`delete_session`）同样清空并 toast；随后走现有会话复位路径。
- `LxChat::reset_session_view()`：兜底 `pending_steers.clear(); promoting_steers = 0;`。
- 关闭窗格：状态随窗格销毁，无需 toast。
- toast 文案：`Discarded N undelivered steer(s)`（英文，用户可见文案统一英文）。

### 8.4 撤回 / 编辑

- 提示行右侧 `×`：鼠标点击 → `steer::recall(state, pane, index)`。
- 输入框为空、光标在 0、无选区、Chat 模式、无面板、`RequestState::Streaming` 时按 `Backspace` → 撤回最近一条。
- 撤回 = 从 pending 移除 + 文本按 Q3 规则合并回输入框；不是丢弃。
- `RequestState` 不是 `Streaming` 时（Committing/Finishing）提示区可见但不可撤回，避免与终态提升竞争。
- 撤回与交付竞争：事件循环串行处理，先到者生效；若交付已成功，pending 已被移除，撤回自然落空（可选 toast：`Steer already delivered`）。

---

## 9. UI

### 9.1 pending 提示区的位置

栈顺序（从上到下）：

```text
history
pending steer strip   ← 新增，最多 3 行
chat bar              ← loading / jump to bottom（现有）
input box
status border
```

- 只要 `pending_steers` 非空且未打开 task 页 / ask 面板就显示；`Committing`/`Finishing` 期间仍可见，但 `×`/Backspace 只在 `Streaming` 生效（见 §8.4）。
- 宽度与输入框一致（`input.x/input.width`）。
- `layout` 新增 `pending: Option<Rect>`；渲染用 `history_rect` 的底边改为 pending 上沿（有 pending 时）；`panel_base` / `top_limit` 也贴 pending 上沿；`history_wheel_region` 可继续到输入框上沿（含 pending 与 chat bar），滚轮在提示区上仍滚历史。
- 输入框、聊天栏、历史滚动条几何不变；pending 只是从历史区底部扣掉 N 行。

### 9.2 渲染

新增 `src/ui/main_content/lx/steer_strip.rs`：

```rust
pub(crate) const MAX_VISIBLE: usize = 3;

pub(crate) fn height(lx: &Lx) -> u16;
pub(crate) fn draw(grid: &mut Grid, rect: Rect, lx: &Lx);
pub(crate) fn recall_hit(area: Rect, lx: &Lx, column: u16, row: u16) -> Option<usize>;
```

单行格式：

```text
Steering  <单行截断文本>  · 2 attachments                    ×
```

- 标签 `Steering` 用 `style::muted()`；文本用 `style::muted()`；`×` 用 `style::muted()`（可加 hover 再亮，v1 可不做）。
- 文本预处理：`\n`/`\t`/控制字符 → 空格；连续空白折叠；按可用宽度截断并加 `…`。
- 附件后缀：`attachments` 数量 = `pending.attachments.len()`；为 0 时省略。
- `×` 固定在最右一列（`rect.right()-1`）；宽度不足时优先保 `×` 与 `Steering`，文本整体省略。
- 超过 `MAX_VISIBLE`：显示前 `MAX_VISIBLE` 条；若还有更多，最后一行改为 `+N more pending`（不可点击），保证不无限挤占历史。
- 不写选区块、不进入复制文本（pending 不是历史内容）。

用户可见文案（英文，`src/ui/text.rs`）：

| 常量 | 文案 |
|------|------|
| `LX_STEER_LABEL` | `Steering` |
| `LX_STEER_MORE` | `+{n} more pending` |
| `LX_STEER_ATTACHMENTS` | `{n} attachments` |
| `LX_STEER_DISCARDED` | `Discarded {n} undelivered steer(s)` |
| `LX_STEER_SLASH_UNSUPPORTED` | `Slash commands can't be sent while steering` |
| `LX_STEER_TEXT_LIMIT` | `Steer is too large` |

### 9.3 鼠标命中

`src/event/mouse/press.rs` 在 `chat_hit` 之前、`text_paste_cancel` 之后插入：

```rust
if let Some(index) = state.pane_anywhere(pane).and_then(|target| {
    ui::main_content::lx::steer_strip::recall_hit(
        inner,
        &target.lx,
        mouse.column,
        mouse.row,
    )
}) {
    update::recall_lx_steer(state, pane, index);
    *dirty = true;
    return;
}
```

- 命中只覆盖 `×` 的单列矩形；点文本行其它位置不拦截，继续走历史/输入命中。
- `ui::main_content::lx` re-export `steer_strip::{height, draw, recall_hit}`。

### 9.4 键盘

`src/app/update/pane.rs::apply_lx_input` 在处理完 text paste / path paste / mention / refer / slash 面板之后、主编辑 match 之前：

```rust
let recall_steer = command == LxInput::Backspace
    && lx.input.is_empty()
    && lx.cursor == 0
    && !lx.has_text_selection()
    && lx.input_mode() == crate::app::state::LxInputMode::Chat
    && !lx.shell_mode()
    && lx.panel.is_none()
    && matches!(lx.chat.request, crate::app::state::RequestState::Streaming { .. })
    && !lx.chat.pending_steers.is_empty();
if recall_steer {
    return super::chat::steer::recall_last(state, pane_id);
}
```

- 输入框有内容时 Backspace 仍正常删字；shell 模式在光标 0 仍退出 shell 模式（现有行为优先）。
- 不新增全局快捷键。

### 9.5 历史中的 steer

- 生效后就是普通 `ChatMessageView { role: "user", kind: "chat", status: "complete" }`；现有 user message 渲染、附件芯片、复制、`/rewind` 全部复用。
- 后续 assistant 是新消息，不是旧消息追加；历史按 `sequence` 自然排序。
- 旧 assistant 片段关闭后显示为完整消息；工具卡片、思考块、统计行均按现有逻辑。
- 不新增历史卡片类型。

---

## 10. 关键代码路径

| 层 | 文件 | 改动 |
|----|------|------|
| app 状态 | `src/app/state/lx/chat.rs` | `pending_steers`、`promoting_steers` 字段；new/reset 初始化与清理 |
| app 状态 | `src/app/state/lx/chat/steer.rs` | `PendingSteer`、`SteerBatch`、`build_batch`、预算常量 |
| app 更新 | `src/app/update/chat/steer.rs` | `try_submit`、`recall`、`recall_last`、`restore_pending_to_input`、`promote_after_finish`、`discard_for_session_change` |
| app 更新 | `src/app/update/chat/mod.rs` | `try_send` 挂载 steer；`on_finish_ack` 分流；`on_turn_committed` 消费 `promoting_steers`；`on_turn_failed` 复位 |
| app 更新 | `src/app/update/chat/session.rs` | `/new`、切换会话丢弃 pending + toast |
| app 更新 | `src/app/update/pane.rs` | 空输入 Backspace 撤回 |
| app 更新 | `src/app/update.rs` / `src/app/update/chat/mod.rs` | 新增 `recall_lx_steer` 薄封装（供 event/mouse 调用） |
| 事件层 | `src/event/chat/steer.rs` | `deliver(...)`：校验、合并、DB、状态更新、回执 |
| 事件层 | `src/event/chat/host.rs` | `AgentHostEvent::SteerInputs`；`EventHost::send` 映射 |
| 事件层 | `src/event/chat/file_snapshots.rs` | `HostRecorderFactory.user_message_id` 改为共享 `Arc<RwLock<String>>` |
| 事件层 | `src/event/chat/mod.rs` | `ChatStreams.mutation_users`；`start_stream` 注入；`finish` 清理；`handle_agent_host_event` 增加分支 |
| agent loop | `src/chat/agent_loop/mod.rs` | `SteerInput`、`AgentHostRequest::SteerInputs/HasPendingSteers`、`take_steer_inputs`、loop 注入点 |
| agent loop | `src/chat/agent_loop/context.rs` | `push_steer_input` |
| exec host | `src/chat/agent/exec/runner.rs`、`src/chat/agent/exec/subagent/turn.rs` | 穷举 match 新增分支：空 steer / false |
| DB | `src/db/chat/mod.rs` | `begin_steer_turn`；`MAX_MESSAGE_BYTES` 提升为 `pub(crate)` |
| DB service | `src/db/service/chat.rs` | `Persistence::begin_steer_turn` 包装 |
| UI | `src/ui/main_content/lx.rs` | `Layout.pending`；history/panel/top_limit 几何；draw 调用 |
| UI | `src/ui/main_content/lx/steer_strip.rs` | 新增：height/draw/recall_hit |
| UI 文本 | `src/ui/text.rs` | `LX_STEER_*` 英文常量 |
| 鼠标 | `src/event/mouse/press.rs` | `×` 命中 → recall |
| 测试 | 各模块 `tests.rs` / `tests/` | 见 §11 |

---

## 11. 测试计划

### 11.1 app 提交

- `try_submit` 只在 Native `Streaming` 生效：`Committing`、`Finishing`、`/compact`、shell、AI Web 全部拒绝并保留草稿。
- 空草稿、未知 `/` 命令、无模型、无 workspace、图片模型不支持：拒绝/提示，pending 不增加，草稿保留。
- 成功：pending 增加、输入框清空、`model_images`/attachments 保留、模型快照正确。
- `MAX_PENDING_STEERS` 与图片总量上限：拒绝并保留草稿。
- 多窗格同会话：只有 `Streaming` 的窗格能 steer；其它窗格保持现有 busy 行为。

### 11.2 合并

- 纯文本多条：空行拼接、顺序正确、`consumed` 正确。
- 带 `@` 引用/图片占位符：marker 偏移平移正确；合并后的附件位置落在合并文本内。
- 图片去重；超过 4 张 / 2 MiB 文本 / 40 MiB 图片时停止合并，剩余留在 pending。
- 模型快照取第一条。

### 11.3 DB

- `begin_steer_turn` 原子性：旧 assistant 变 complete + `stop_reason="steer"`；新 user/assistant 同事务出现；`turn_index`/`sequence` 连续；attachments 落库；rewind scope 关闭/开启。
- 旧 assistant 非 streaming → `MessageNotStreaming`，事务回滚，无新行。
- session/workspace 不匹配 → 失败且无半行。
- 重复调用同一 previous assistant → 幂等/失败（按实现选择，建议 `MessageNotStreaming`）。
- 图片附件 `insert_attachments` 归档行为与普通 `begin_turn` 一致。

### 11.4 event / loop

- `deliver` 空 pending：`Ok(empty)`，不写 DB。
- `deliver` 成功：`RequestState.assistant_id` 切到新 id；后续 `BeginTool`/checkpoint/finish 落到新 assistant。
- `deliver` DB 失败：回 `HostError::Failed`，pending 不消费。
- `deliver` 图片模型不支持：`Failed`，pending 保留。
- loop：首次采样前不注入；工具批处理中途不注入；auto-compact 后注入；最终 Final 后注入并继续采样；取消/失败停止。
- `ModelContext`：steer 的 `UserContent`/`UserText` 顺序在工具结果之后、下一次 assistant 之前；skill blocks 紧随 steer。
- fake host：`take_steer_inputs` 返回脚本化输入，验证请求序列。

### 11.5 终态

- 正常完成 + pending：`on_finish_ack` 入队 `BeginChatTurn`；`on_turn_committed` 消费 `promoting_steers`，不清用户新草稿；剩余 pending 保留。
- `BeginChatTurn` 失败：`promoting_steers=0`，pending 保留，可再次触发。
- Esc/失败 + pending：退回输入框，合并顺序正确，不自动发送。
- 切换会话/`/new`：pending 清空 + toast；新会话输入框不被污染。
- 撤回：`×` 与空输入 Backspace；已有草稿时前置合并；`Committing`/`Finishing` 不可撤回。

### 11.6 UI

- `steer_strip::height`：0（无 pending/task/ask）、1..3、超过 3 时 `+N more pending`。
- 绘制：标签、截断、附件计数、`×` 在最右列；宽字符不半写；不进入历史复制。
- `recall_hit`：只命中 `×`；坐标边界；滚动条/历史命中不穿透。
- `layout`：history 底部扣掉 pending 行；chat bar/输入框位置不变；面板贴 pending 上方；task/ask 隐藏。
- 鼠标点击 `×` 只撤回对应项；Backspace 撤回最近一条。

### 11.7 验证命令

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

---

## 12. 风险与回滚

| 风险 | 处理 |
|------|------|
| 旧 assistant 片段关闭与新 user/assistant 插入必须原子 | `begin_steer_turn` 单事务；失败回滚，pending 保留 |
| checkpoint 在 steer 前排队，关闭后写入失败 | `checkpoint` 只对 streaming 生效；关闭后静默失败；最终 parts 由 `begin_steer_turn` 写 |
| 图片在 pending 期间源文件被改/删 | pending 保存 `model_images` 字节，模型注入用内存副本；DB 归档仍按源路径，属现有行为风险，设计记录 |
| 多条 steer 合并 marker 偏移错误 | `build_batch` 纯函数 + 表驱动测试覆盖引用/图片/skill |
| 结束竞态导致 pending 丢失 | `on_finish_ack` 提升/退回；单测覆盖最终检查后的极短窗口 |
| 切换会话把旧 steer 带进新会话 | 显式清空 + toast；`reset_session_view` 兜底 |
| 提示区挤占历史 | 最多 3 行；超出 `+N more pending`；task/ask 隐藏 |
| 不在 `Streaming` 时误撤回 | 仅 `Streaming` 可撤回；Committing/Finishing 只读提示 |
| 模型 prompt 行为变化 | 不新增系统提示；steer 是普通 user 消息，不改 `PROMPT_VERSION` |

回滚策略：

- 功能开关：若需要，可在 `try_send` 的 steer 分支前加配置开关；默认开启。
- 无 schema 变更，回滚代码即可；已产生的 steer 用户消息是普通消息，旧版本可正常读取/继续对话。
- DB 中旧 assistant 的 `stop_reason="steer"` 对旧版本无影响（旧版本只按 status 分组）。

---

## 13. 后续可做（不在本版）

- 未生效 steer 的持久化 inbox（opencode 级跨重启恢复）。
- `queue`（下一轮）与 steer 的快捷键切换。
- 子 agent 的 steer 输入框。
- 音频/远程图片 URL/agent mention。
- 待生效区的滚动/逐条编辑面板。
