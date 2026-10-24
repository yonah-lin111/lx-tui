# lx Agent Harness 与上下文压缩：实现评审

> 审查对象：worktree `.worktrees/lx-agent-harness`，分支 `feat/lx-agent-harness`，
> 基线 `dev` @ `3379621`，共 6 个提交、114 个文件、+8445/-274。
> 审查基线：同目录 [design.md](design.md)、[task.md](task.md)。
> 结论：持久化事务、检查点编号、时间线布局这几块结构正确；但**冻结系统提示在采样链路里被丢掉**，
> 导致环境快照与 compact 摘要都到不了模型；`edit` 的 `*** Move to:` 被持久层无条件拒绝。
> 这两项都是会直接让功能失效的缺陷。另有若干多窗格/迟到回执/UI 几何问题。
> 行号是**修复前** HEAD 的定位，修复后按函数名检索。

## 1. 关键链路

```text
普通轮次
  start_stream
    ├─ compact = pane.lx.compact.active()            # 取最新 active 检查点
    ├─ environment_snapshot(capture(), workspace, workspace, tasks)
    ├─ assemble ── 过滤 sequence <= checkpoint.end_sequence
    │              └─ compose_system_prompt(environment, compact_summary)
    ├─ ChatRequest.system_prompt = Some(冻结文本)
    └─ run_agent_turn
         └─ ModelContext::next_request()             # ← 每次采样都走这里
              └─ system_prompt: None                 # ← C1：冻结文本在这里丢失
  transport ── resolve_system_prompt()
               None → prompt::system_prompt()        # 现场采集，无 workspace/无摘要

/compact
  slash → compact_command（仅本 pane 空闲检查）→ ExecuteCompact
  event/chat/compact
    load_chat_session → playable_messages → incremental(base.end+1 ..)
    → summary_request（专用 system、无工具）→ run_stream
    → commit_compact（revision CAS + 共享 sequence 取号）
    → CompactFinished{pane, session, workspace, outcome}   # ← 无 operation_id
    → 回填检查点 / revision / 其他绑定同会话的 pane

compact 撤销
  select_latest（用户消息 sequence vs active 检查点 sequence）
  → RevertCompact{workspace, checkpoint, expected_revision, operation_id}
  → revert_compact（state→reverted、写 agent_rewind_operation、revision+1）
  → CompactReverted{pane, session, outcome}              # ← 无 workspace、无 operation_id
```

## 2. 缺陷

### C1（严重）冻结系统提示在第一次采样前就被清空，环境快照与 compact 摘要从未到达模型

`src/chat/agent_loop/mod.rs:351`

```rust
fn next_request(&self) -> ChatRequest {
    ChatRequest {
        turns: self.request.turns.clone(),
        items: self.items.clone(),
        model: self.request.model.clone(),
        variant_params: self.request.variant_params.clone(),
        purpose: crate::chat::events::RequestPurpose::Chat,
        system_prompt: None,
    }
}
```

错误原因：`run_agent_turn` 的**每一次**采样（包括第一次）都调用 `context.next_request()`
（`src/chat/agent_loop/mod.rs:462`）。这里把 `system_prompt` 写成 `None`，于是四类 transport
（`resolve_system_prompt`）全部回退到 `prompt::system_prompt()`：现场重新采集环境、没有
workspace/cwd/后台任务、**也没有 compact 摘要**。

失败场景：
1. 执行 `/compact` 成功后，下一轮只用 `items` 里 `sequence > checkpoint.end_sequence` 的消息，
   摘要从未进入 system 段——模型永远看不到压缩前的上下文。`/compact` 等于把上下文删了。
2. 环境快照（`start_stream` 里冻结的那份）从未被任何一次采样使用；`<workspace_root>`、
   `<cwd>`、`<background_tasks>` 全是空的。
3. `task.md` B 第 5 条声称“`ModelContext::next_request` 原样复用该字段”，与代码相反；
   `agent_loop` 测试全部用 `system_prompt: None` 构造上下文，所以测不出来。

修复：`next_request` 改为 `system_prompt: self.request.system_prompt.clone()`；补一条
“两次采样拿到的 system prompt 完全相同且包含摘要”的回归测试。

### C2（严重）`*** Move to:` 在持久层被无条件拒绝，移动功能完全不可用

`src/chat/tools/edit.rs:562`

```rust
changes.push(PathMutation {
    operation: MutationOperation::MoveDestination,
    path: target.clone(),
    target: destination.clone(),
    before: destination_before,
    planned_after: mutation::planned_regular(&before_image, encoded.clone()),
    expected_version: None,
});
```

`src/db/chat/rewind.rs:407`

```rust
if change.operation != "create" && change.expected_version.is_none() {
    return Err(DbError::StaleRead);
}
```

错误原因：`MoveDestination::as_str()` 是 `"move_dst"`，不是 `"create"`。生产 recorder 会把
`expected_version: None` 原样送到 `prepare_mutations`，事务在**写盘前**返回 `StaleRead`。
单元测试用 `ToolContext::new(cwd)`（无 registry）且不接 recorder，所以 `move_to_*` 全绿，
生产路径却必然失败。

失败场景：模型读到文件后发 `*** Update File: a\n*** Move to: b\n@@ ...`（含纯移动），
工具返回 `stale_read`，文件不移动；模型按提示重读再试仍然失败，形成死循环。

修复：
1. `move_dst` 在语义上是“新建目标”，持久层的 token 必填规则应豁免它（与 `create` 同档），
   或为它写入 `Missing` 镜像的 token；
2. 同一个锁内必须补 `read_version::require_missing(destination)`。当前只在
   `resolve_paths`（锁外）检查 `destination.exists()`，解析之后、rename 之前别人新建的
   文件会被静默覆盖，违反“新文件目标必须不存在、冲突零写入”；
3. 补一条走真实 recorder/DB 的回归测试（现有 `edit/tests.rs` 覆盖不到持久层）。

### M1（主要）compact 回执没有 operation id，迟到回执会踩掉新一次执行

`src/event/mod.rs:111` 与 `src/event/chat/compact.rs:102`

`CompactFinished { pane, session_id, workspace, outcome }` 与
`CompactReverted { pane, session_id, outcome }` 都不携带发起时的 `operation_id`；
`CompactRequest.operation_id` 只用于 DB 幂等，没有参与回执匹配。

失败场景：
1. 会话 S 发起压缩 A；
2. 用户切到 T，再切回 S（`compact.reset()` 清掉 running）；
3. 用户在 S 发起压缩 B；
4. A 失败回执到达 → `on_compact_failed`（`src/app/update/chat/compact.rs:150`）不校验会话、
   更不校验 operation，直接 `finish()` 清掉 B 的计时与 running，并弹一条错误；
   此时 B 仍在跑，用户还能再发一次 `/compact`。

修复：两个事件都携带 `operation_id`；`Committed/Failed/Empty` 只在该 pane 的
`compact.running == Some(operation_id)` 时改执行态；`CompactReverted` 只在该 pane 的
`rewind.executing.operation_id` 匹配时生效（与 `RewindFinished` 的 S8 修复同口径）。

### M2（主要）compact 撤销回执重新解析 workspace，切工作区后误清检查点

`src/event/chat/compact.rs:330`

```rust
let checkpoints = persistence
    .database()
    .and_then(|db| {
        let workspace = state.chat_session_key_for_pane(pane)?;
        db.compact_checkpoints(&workspace, session_id).ok()
    })
    .unwrap_or_default();
```

错误原因：撤销请求已经冻结了 `workspace`（`CompactRevertRequest.workspace`），但
`CompactReverted` 事件把它丢了，回执阶段按“当前窗格”重新解析。撤销执行期间切换
工作区/标签页/仓库后，DB 归属解析指向另一个 workspace，`compact_checkpoints` 返回空，
`on_compact_reverted` 用空列表覆盖 UI，同时提示“撤销成功”。

修复：`CompactReverted` 携带请求里的冻结 workspace，回执只读冻结值（与 `RewindFinished`
的 M3 修复一致）。

### M3（主要）回执读库失败被 `unwrap_or_default/0` 吞掉，revision 被写成 0

`src/event/chat/compact.rs:272` 与 `:330`

```rust
let checkpoints = persistence.database()
    .and_then(|db| db.compact_checkpoints(workspace, session_id).ok())
    .unwrap_or_default();
let revision = persistence.database()
    .and_then(|db| db.session_revision(session_id).ok())
    .unwrap_or(0);
```

错误原因：提交/撤销已经成功，但回填读库失败时被当成“空列表 + revision 0”。空列表会
直接清掉界面上的检查点；revision 0 会被冻结进下一次 `/compact` 或 `/rewind` 的
`expected_revision`，之后所有 CAS 都失败，直到下一次完整重载。

修复：读库失败时保留现有检查点与 revision（不改写），只结束执行态；不要在成功路径上
用 `unwrap_or(0)` 覆盖单调递增的 revision。

### M4（主要）`/compact` 的忙碌检查是 pane 级，不是会话级

`src/app/update/chat/compact.rs:24` 与 `src/app/update/chat/mod.rs:87`

```rust
if !matches!(target.lx.chat.request, RequestState::Idle) || target.lx.compact.is_running() {
    // 只看当前 pane
}
```

错误原因：design §2.3 要求“会话忙碌（流式回复、工具调用或 compact 正在执行）时拒绝”。
同一会话可以被多个 lx 窗格共享：A 窗格压缩期间，B 窗格仍可发送普通轮次；`begin_turn`
会 `revision+1`，A 的 `commit_compact` 随后以 `RewindTargetChanged` 失败。反向同理，
两个窗格可以同时发起压缩，后提交的一方必败。

修复：两处都按 `session_id` 扫描所有窗格（复用 rewind `execute_blocker` 的口径）；
`has_streaming_lx` 的语义保持不变。

### M5（主要）`^o` 不认 Alt+↑/↓ 选中的检查点

`src/app/update/chat/process.rs:507`

```rust
let Some((message_id, call_id)) = target_key else {
    if let Some(checkpoint_id) = latest_compact_checkpoint(&target.lx) {
        return super::compact::toggle_checkpoint(state, pane, &checkpoint_id);
    }
    ...
};
```

错误原因：`Alt+↑/↓` 在无工具卡片时把目标写进 `lx.chat.selected_compact`
（`process.rs:544`），但 `^o` 永远取 `.checkpoints.last()`，从不看选中值。多检查点时
选中较早一条再按 `^o`，展开的是另一条；`selected_compact` 没有任何读取者，是半截功能。
`task.md` G 声称“Alt+↑/↓ 选择后 `^o`”已实现，与代码不符。

修复：`^o` 优先 `selected_compact`，没有选中再回退最近一条；补“选中 c1 后 ^o 只切换 c1”
的测试。

### M6（主要）撤销 compact 失败被报成“压缩失败”

`src/event/chat/compact.rs:355`

```rust
crate::app::update::on_compact_failed(state, pane, &code);
```

错误原因：`on_compact_failed` 固定使用 `LX_COMPACT_FAILED`
（“Compact failed; conversation was left unchanged.”）。撤销失败时用户看到的是“压缩失败”，
与刚刚执行的动作无关，且 `on_compact_failed` 还会清 `running`（撤销根本不置位）。

修复：新增 `LX_COMPACT_REWIND_FAILED` 与独立的失败处理（清 `rewind.executing`/面板、
保留检查点、提示“restore failed”）；不要再复用压缩失败文案。

### M7（主要）`history.rs` 超过 1000 行上限

`src/ui/main_content/lx/history.rs`：dev 971 行 → 本次 1044 行，违反
`docs/standards/code-writing-standards.md` 的单文件上限。compact 条目的引入把
`timeline`/布局分支继续堆在同一个文件里。

修复：把时间线合并（`TimelineEntry` / `timeline()` 及其测试）拆到
`src/ui/main_content/lx/history/timeline.rs`，主文件回落到 1000 行以内。

### m1（次要）系统提示说“Read or search”，但 `search` 不产生版本 token

`src/chat/prompt.rs:55`

```text
- Read or search a file before modifying it. `edit`/`write` reject a file that was not read ...
```

`search` 不登记 `ReadRegistry`、也不返回 `file_version`。模型按提示先 search 再 edit 会拿到
`stale_read`，白白多一轮。改为“Read a file before modifying it; search results cannot be
used as a read version.”。

### m2（次要）`revert_compact` 的幂等分支不校验 operation id

`src/db/chat/compact.rs:263`

```rust
if state == COMPACT_REVERTED {
    // 幂等重放：同一次操作已撤销，原样返回。
    let checkpoint = checkpoint_by_id(&transaction, &id)?...;
    transaction.commit()?;
    return Ok(checkpoint);
}
```

错误原因：注释说“同一 operation”，代码却对任意 operation 返回 `Ok`。另一个 operation
（例如第二个窗格对同一检查点的撤销）会得到**假成功**，且不会写入自己的审计行。
修复：仅当 `reverted_by_operation_id == Some(revert.operation_id)` 时幂等返回，否则
`RewindTargetChanged`。

### m3（次要）死代码

- `app::update::chat::compact::revert_request`（`src/app/update/chat/compact.rs:167`）
  只被测试调用，生产路径走 `select_latest` + `apply_compact_enter`；
- `Database::latest_active_checkpoint`（`src/db/chat/compact.rs:333`）无调用者；
- `LX_COMPACT_TOO_LARGE`、`LX_COMPACT_COLLAPSED_HINT`（`src/ui/text.rs:717/725`）无渲染点。

修复：删除，或把真正的生产路径收敛到这些函数上；不要留两套并行实现。

### m4（次要）`history/compact.rs` 的行数计算写法

`src/ui/main_content/lx/history/compact.rs:24`

```rust
for line in super::wrap(&notice.summary, width) {
    let _ = line;
    total += 1;
}
```

等价于 `total += super::wrap(...).len()`。改成后者，少一次误导性循环。

### m5（次要）环境快照截断不是“按行”，会留下残缺 XML

`src/chat/prompt.rs:191` 的实现按字节截断并追加 `<!-- environment truncated -->`，
`</environment_context>` 很可能被截掉；`MAX_ENVIRONMENT_BYTES` 的文档却写“按行丢弃尾部”。
模型看到的是一段不闭合的结构。修复：按行截断并保留闭合标签（或显式补 `</environment_context>`）。

### m6（次要）lx toast 的宽高几何与实际绘制不一致

`src/ui/main_content/lx/toast.rs:46`

```rust
let wanted = ...max_line_width().saturating_add(PADDING);   // PADDING = 2
let body_width = usize::from(width.saturating_sub(PADDING)).max(1);
let rows = text::wrap(&toast.message, body_width).len();    // 按 width-2 折行
...
let text_width = usize::from(body_area.width.saturating_sub(2)).max(1); // 实际 width-4
let lines = text::wrap(&toast.message, text_width);
```

错误原因：
1. 全局 toast 的盒宽是 `文本 + 4`（2 边框 + 2 内边距），lx toast 只加了 2，盒宽比需要窄 2 列；
2. 估高按 `width-2` 折行，绘制按 `width-4` 折行，长文案会多出一行，然后被
   `take(body_area.height)` 静默切掉——注释说的“截断并加省略号”根本没有实现；
3. `y = area.y + 1`、`height` 只夹到 `area.height`，`y+height` 未夹到 `area.bottom()`；
   窗格内容区只有 3 行时，盒底边会画到窗格边框上，违反“只在内容区内”。

修复：盒宽用 `文本 + 4`；`rect` 与 `draw` 共用同一个 `text_width = width - 4`；超出行数
用 `text::ellipsize` 收尾；`y` 取 `min(area.y + 1, area.bottom() - height)`。

### m7（次要）环境采集在事件循环里做有界网络探测

`src/event/chat/mod.rs:877` 的 `EnvironmentFacts::capture()` 会调用
`network_available()`（`src/platform/environment.rs`）：最多两个地址 × 80 ms，缓存 30 s。
`start_stream` 在事件循环里执行，最坏会有约 160 ms 的输入卡顿，与“阻塞操作放后台”的
规范相抵。建议拆成“只读缓存 + 后台刷新”，或把探测挪进网络任务。

### m8（风险，暂不修）输入超限丢老轮次会留下摘要缺口

`src/chat/compact.rs:146` 的 `bounded_transcript` 在超过 `MAX_TRANSCRIPT_CHARS` 时从最老
完整轮次开始丢弃，并只在转写里写一行 `[N older turn(s) were omitted ...]`。被丢的轮次
既不在新摘要里，也因为 `sequence <= end_sequence` 不会在后续装配中回放——模型上下文里
永久缺失这一段（DB 原文仍在）。`task.md` 已把“最老轮次丢弃并标注”写成既定行为，因此本次
不改变语义；但至少应在 compact 条目上显示“partial range”，不要让用户以为摘要覆盖完整。
若要彻底消除缺口，只能分块摘要或输入超限直接失败。

## 3. 已验证正确的部分

- `commit_compact`：检查点插入、共享 `sequence = MAX+1`、`revision+1` 在同一事务；同
  `operation_id` 重放直接返回既有行，不重复取号。
- `revert_compact`：只改 `state`/`reverted_by_operation_id` 与 revision，不删消息、不碰
  文件 scope；只允许撤销最新 active，revision 不符零写入。
- compact 失败/空增量路径：不写检查点、不改 revision、原消息保留（真实 mock SSE + 文件 DB）。
- `read_version` 的两种视图（followed/entry）与路径锁/DB 事务内的二次核对；新建目标在锁内
  再查一次不存在。
- compact 时间线条目：layout `total_rows` 与 render 推进行数一致，折叠/展开/窄屏换行都有断言；
  compact 不合并、已撤销条目保留并标注。
- lx toast 的“不越出窗格”方向正确（窗格内绘制、命中与绘制共用矩形、hover 冻结、点外关闭
  不吞点击），问题仅在 m6 的宽高细节。
- 迁移 v14 只增表，`user_version` 与建表同事务推进；与 dev 的 v13 不冲突。

## 4. 修复清单

| 编号 | 修复 |
| --- | --- |
| C1 | `ModelContext::next_request` 保留 `request.system_prompt`；补采样间提示稳定的测试 |
| C2 | 持久层把 `move_dst` 视为 create 类豁免 token 规则；锁内 `require_missing(destination)`；补真实 recorder/DB 测试 |
| M1 | `CompactFinished`/`CompactReverted` 携带 `operation_id`；迟到回执按执行中 id 丢弃 |
| M2 | `CompactReverted` 携带冻结 `workspace`，回执只读它 |
| M3 | 回执读库失败时保留现有检查点与 revision，不写 0/空列表 |
| M4 | compact 发起与发送的忙碌检查按会话扫描所有窗格 |
| M5 | `^o` 优先 `selected_compact`，回退最近检查点 |
| M6 | 新增撤销失败文案与独立处理路径 |
| M7 | 时间线拆分到 `history/timeline.rs`，`history.rs` 回到 1000 行以内 |
| m1 | 修正系统提示的 search/read 口径 |
| m2 | `revert_compact` 幂等分支校验 operation id |
| m3 | 删除死函数与死文案（或收敛到唯一实现） |
| m4 | 简化 `compact::rows` 的折行计数 |
| m5 | 环境快照按行截断并保留闭合标签 |
| m6 | lx toast 宽高与折行宽度统一，补省略号与纵向夹取 |
| m7 | 网络探测移出事件循环（缓存只读 + 后台补测） |
| m8 | 暂不修；在文档上标注为已知风险 |

## 5. 修复落地

以上 C/M 项与 m1–m7 已修复，m8 保持为已知风险：

| 编号 | 实际修改 |
| --- | --- |
| C1 | `ModelContext::next_request` 复用 `request.system_prompt`；新增 `frozen_system_prompt_is_reused_by_every_sampling` |
| C2 | `prepare_mutations` 把 `move_dst` 与 `create` 同档；`edit` 移动分支锁内 `require_missing`；新增 `prepare_allows_move_destination_without_version` |
| M1 | `CompactFinished`/`CompactReverted` 增加 `operation_id`；迟到回执按执行中 id 丢弃；新增 `late_compact_failure_for_another_operation_is_ignored` |
| M2 | `CompactReverted` 增加冻结 `workspace`，回执只读它 |
| M3 | 回填读失败时保留现有检查点与 revision；撤销回填失败时本地 `mark_reverted`，上下文指针立刻回退 |
| M4 | `/compact` 与发送都按会话扫描所有窗格；新增两个同会话多窗格测试 |
| M5 | `^o` 优先 `selected_compact`；新增 `ctrl_o_toggles_the_selected_checkpoint` |
| M6 | 新增 `on_compact_revert_failed` 与 `LX_COMPACT_REWIND_FAILED` |
| M7 | `timeline` 拆到 `history/timeline.rs`（`history.rs` 959 行） |
| m1 | 系统提示改为“先 read；search 不带版本” |
| m2 | `revert_compact` 幂等分支校验 `reverted_by_operation_id`；新增 `revert_idempotency_requires_the_same_operation` |
| m3 | 删除 `revert_request`、`latest_active_checkpoint`、两个死文案；原测试改走真实选择/面板路径 |
| m4 | `compact::rows` 简化为一次 `wrap(...).len()` |
| m5 | 环境快照按行截断并补 `</environment_context>` |
| m6 | toast 盒宽 `+4`、估高与绘制共用 `width-4`、溢出加 `…`、纵向夹在内容区内；新增 3 条几何测试 |
| m7 | `EnvironmentFacts::capture` 只读缓存；过期时触发一次后台探测并返回 `unknown` |

## 6. 验证

```text
cargo fmt --check                                通过
cargo clippy --all-targets -- -D warnings        通过（0 warning）
cargo test                                       3199 passed (lib) + 全部集成测试通过，exit 0
```

新增/更新的回归测试：

- `frozen_system_prompt_is_reused_by_every_sampling`
- `prepare_allows_move_destination_without_version`
- `revert_idempotency_requires_the_same_operation`
- `late_compact_failure_for_another_operation_is_ignored`
- `compact_is_rejected_while_another_pane_on_the_same_session_is_busy`
- `sending_is_rejected_while_another_pane_compacts_the_same_session`
- `ctrl_o_toggles_the_selected_checkpoint`
- `compact_revert_is_context_only` / `compact_revert_requires_active_checkpoint`
- `box_width_reserves_borders_and_padding` / `overflowing_body_is_ellipsized` / `short_pane_keeps_the_box_inside`
- `cached_network_reads_the_warmed_cache_without_probing`

TUI 观感（toast 折行、compact 展开、窄窗格）仍需用户 `cargo run` 人工确认；m8
（输入超限丢老轮次造成摘要缺口）按 `task.md` 的既定行为保留。
