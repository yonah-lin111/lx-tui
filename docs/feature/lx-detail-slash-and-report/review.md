# Agents / BTW 详情 `/` 命令 — 代码 Review

- **Review 日期**：2026-11-16
- **范围**：`detail-slash-report` 相对 `dev` 的全部改动（详情 `/rewind` `/compact` `/report` `/new`、动画/心跳/滚动修复）。
- **方法**：逐文件代码路径推演 + codegraph 调用关系 + 定向测试；每条结论都尽量落到现有测试或新增回归测试。
- **结论**：没有发现崩溃级 P0；但有 6 个会破坏核心语义或造成不可逆副作用的 P0，已在本次修复。P1–P3 中除“遗留项”与明确标注为部分覆盖的条目外均已修复。
- **合并**：修复已随 `detail-slash-report` 于 2026-11-16 合并进 `dev`（merge `f7d53b9d`）；合并后复跑 `cargo fmt --check`、clippy 与全量测试（lib **5641 passed / 6 ignored**，集成测试全通过）。

---

## P0 — 合并前必须修复

### P0-1 自动压缩检查点抢占 `/rewind` 目标

- **位置**：`src/event/chat/detail_rewind.rs::prepare`、`src/db/chat/compact.rs`、`src/app/state/lx/compact.rs::from_record`。
- **关键点**：`/rewind` 直接取 `compact_checkpoints().pop()`，只比较 `sequence > latest_user_turn.sequence`。
- **错误原因**：自动压缩同样写 `agent_compact_checkpoint`（`operation_id` 前缀 `autocompact-`）。自动压缩发生在本轮结束之后，其 `sequence` 必然大于该轮用户消息，于是 `/rewind` 弹出 `RestoreCompact` 而不是删除最后一轮私聊；执行后还会删掉自动检查点，让后续上下文回退到更早基线。`from_record` 又把 `auto` 硬编码为 `false`，重载后自动检查点被渲染成伪造的手动范围。
- **修复**：新增 `CompactCheckpoint::is_auto()`；详情预检只接受“最新 active 且非自动”的检查点；`from_record` 同步设置 `auto`；主聊天 `select_compact` 改用 `LxCompact::rewindable()`，同样跳过自动检查点。
- **回归**：`auto_compact_checkpoint_is_not_a_rewind_target`、`rewindable_skips_auto_checkpoints`。

### P0-2 无参命令采纳绕过“整段独立”校验

- **位置**：`src/app/update/chat/composer.rs::accept`。
- **关键点**：只有带参命令（`/report`）调用 `standalone_line()`；`/rewind` `/compact` `/new` 直接清空整框并执行。
- **错误原因**：`/` 面板按光标所在行触发。多行草稿最后一行是 `/new` 时，面板照常出现，Enter/Tab/鼠标采纳会删除线程并清掉整框正文。
- **修复**：所有命令采纳前统一校验 `standalone_line`；失败只提示 `Run this command on its own line`，保留草稿。
- **回归**：`multiline_draft_does_not_execute_last_line_command`。

### P0-3 撤销执行态挂在 `TaskPage`，关闭任务页即丢失

- **位置**：`src/app/state/lx/task.rs`（原字段）、`src/event/chat/rewind.rs::handle_rewind_finished`。
- **关键点**：`detail_rewind_pending/executing` 存在可被 `task.take()` 销毁的 `TaskPage` 上。
- **错误原因**：确认撤销后按 `Esc` 关闭后台页，回执到达时找不到执行态，详情回执落进主聊天 `/rewind` 分支：失败回执会在主输入区显示错位提示，成功回执也不会回填 detail run/thread。
- **修复**：执行态迁到 `Lx.detail_rewind`（`pending`/`executing`），跨任务页开关存活；`handle_rewind_finished` 先按执行态分流，并对 `detail-rewind-` operation id 前缀兜底忽略，绝不落进主聊天。
- **回归**：`closing_the_page_keeps_detail_rewind_execution`。

### P0-4 BTW 删空回执重置“当前线程”而不是目标线程

- **位置**：`src/app/update/chat/detail_rewind.rs::on_succeeded`。
- **关键点**：`active_thread_mut()` + `session_deleted` 时跳过 `thread_id` 校验。
- **错误原因**：撤销执行期间切到另一个 QA，回执会把无关线程 `reset_after_delete()`；真正的目标线程保留已删除会话的陈旧视图。另外删空后没有 `LoadQaList`，列表 `● side thread` 标记不消失。
- **修复**：按 `plan.target` 的 `thread_id` 用 `thread_by_thread_id_mut` 定位；删空只重置该线程并 push `LoadQaList`。
- **回归**：`btw_rewind_receipt_targets_the_planned_thread_only`。

### P0-5 撤销成功后的草稿回填走 `active_editor()`

- **位置**：`src/app/update/chat/detail_rewind.rs::on_succeeded`。
- **关键点**：回填使用 `lx.active_editor()` / `with_active_editor()`，不在编辑态时会回退到主输入框。
- **错误原因**：用户在确认后、回执前退出编辑或切到另一个 agent/QA，被撤回的私聊原文会写进主会话输入框或另一个 composer。
- **修复**：只操作 `plan.target` 自己的 composer（`agents.composer_mut(agent_id)` / `btw.thread_by_thread_id_mut(thread_id).composer`），目标不存在时不动任何草稿。
- **回归**：`btw_rewind_receipt_targets_the_planned_thread_only`（断言主输入框保持 `main draft`）。

### P0-6 详情 `/compact` 可重入，覆盖幂等键

- **位置**：`src/event/chat/detail_compact.rs::agent_busy/btw_busy`、`src/app/state/lx/compact.rs::start`。
- **关键点**：忙碌判据不包含 `compact.is_running()`，`LxCompact::start` 无条件覆盖 `running`。
- **错误原因**：压缩中再次 `/compact` 会覆盖 operation id；两次请求冻结同一 revision，先提交者成功、后提交者失败，失败回执匹配到被覆盖后的 operation，UI 显示“失败”但检查点已写入，且旧回执永远对不上。
- **修复**：忙碌判据加入 `compact.is_running()`；`LxCompact::start` 在已有 `running` 时返回 `false`，不再覆盖。
- **回归**：`compact_detail_rejects_reentry_while_compacting`、`start_refuses_to_overwrite_running`。

---

## P1 — 高概率状态错误

### P1-1 `Committed` 读回失败会让 `compacting` 永久驻留

- **位置**：`src/event/chat/detail_compact.rs::handle_finished`。
- **关键点**：读 `compact_checkpoints` 失败直接 `return false`，既没结束 `running` 也没回投。
- **错误原因**：`any_detail_compacting()` 永远为真，`has_streaming_lx` 永远为真，心跳每 100ms 推进并重绘；徽标永久 `compacting`，只能重启恢复。
- **修复**：读回失败时仍调用 `on_detail_compact_finished(..., None)`；App 层 `finish()` 但保留旧检查点卡片。

### P1-2 `Empty` / `Failed` 回执不重量总行数

- **位置**：`src/app/update/chat/detail_compact.rs::handle_finished`。
- **关键点**：`rows_dirty = true` 只写在 `notices: Some` 的 Committed 分支。
- **错误原因**：运行中的 compact 行消失后，`total_rows`、滚动上限与贴底判定仍按“压缩中”的旧值，滚轮会多一行/看不到底。
- **修复**：`finish()` 后无论结果统一置 `rows_dirty = true`。

### P1-3 报告轮结束后未消费 steer 被继承 `notify_parent=true`

- **位置**：`src/event/chat/subagent/settle.rs`。
- **关键点**：重投 steer 时使用 `notify_parent: handle.notify_parent`。
- **错误原因**：报告轮（或 spawn 轮）结束后重投的私聊 steer 会被当成新一轮报告，私聊回答被转发给父 agent。
- **修复**：固定 `notify_parent: false`；`/report` 是一轮性命令，不具粘性。
- **回归**：`undelivered_steer_after_report_is_resent_privately`。

### P1-4 报告正文锚点缺失时回退整段历史

- **位置**：`src/event/chat/subagent/settle.rs::turn_messages`。
- **关键点**：找不到本轮 user message 时 `map_or(&handle.chat.messages, ...)`。
- **错误原因**：一次锚点缺失会把所有历史 assistant 文本（含私聊）当报告发给父 agent。
- **修复**：锚点缺失返回空切片并 `tracing::warn`，报告正文只可能变少，不会越界泄露。

### P1-5 BTW 搜索粘贴切换线程后不加载历史

- **位置**：`src/app/update/pane/task_page.rs` 的 BTW 粘贴分支。
- **关键点**：`sync_selection_after_filter()` 的返回值被丢弃。
- **错误原因**：粘贴把 active 过滤到另一个 QA 后只 `ensure_thread`，不发 `LoadThread`；`!loaded` 让详情永久显示 loading 并参与心跳空转。键盘输入路径有 `switch_qa`，粘贴路径漏了。
- **修复**：拿到 switched 的 qa_id 后走同一个 `switch_qa`。
- **回归**：`btw_search_paste_loads_the_switched_thread`。

### P1-6 子 agent 终态不归还 `RequestState`

- **位置**：`src/app/update/chat/subagent.rs::on_subagent_finished`。
- **关键点**：只清 `status.streaming`，`request` 仍是 `Streaming`。
- **错误原因**：`LxChat::is_animating()` 包含 `request.is_busy()`，所有运行过一次的子 agent 历史永远被判定为动画中，共享判据被污染。
- **修复**：终态显式 `request = RequestState::Idle`；回归断言 `!run.chat.is_animating()`。

### P1-7 LRU 卸载会丢弃在途压缩状态

- **位置**：`src/app/state/lx/agent.rs`。
- **关键点**：`evict_chat_memory` / `evict_excess_idle_chats` 只排除 running 与 `!loaded`。
- **错误原因**：已完成但正在 compact 的 run 会被卸载，`compact.running` 随之清空；检查点已提交但回执匹配失败，卡片不出现。
- **修复**：两处都排除 `compact.is_running()`；压缩开始时 `touch_access()`。

---

## P2 — 失败路径与交互细节

### P2-1 撤销成功回执读库失败会用空列表覆盖视图

- **位置**：`src/event/chat/detail_rewind.rs::handle_finished`。
- **错误原因**：`load_chat_session(...).ok().flatten().unwrap_or_default()`，一次瞬时读失败就把已成功撤销后的消息清空。
- **修复**：读失败走失败提示并保留现有视图；只有 `session_deleted` 才允许空消息。

### P2-2 目标解析失败静默消费命令

- **位置**：`detail_rewind::prepare`、`detail_compact::request/prepare`、`detail_report::submit`、`event/chat/btw/maintenance.rs`。
- **错误原因**：命令已清空，目标不存在时只 `return false`，没有任何 Toast；`/new` 在 DB/workspace 缺失时也不回投失败。
- **修复**：统一补 `Nothing to rewind` / `Cannot compact this conversation` / `Cannot report from this agent` / `Could not clear the side thread`。

### P2-3 成功 Toast 声称“restored files”但实际没有文件变更

- **位置**：`detail_rewind::on_succeeded`。
- **错误原因**：只按 `plan.action` 选固定文案，忽略 `RewindOutcome` 的 `restored_files` / `already_restored`。
- **修复**：复用 `text::rewind_notice(action, restored, already_restored)`。

### P2-4 撤销确认面板挡不住粘贴/剪切

- **位置**：`src/event/paste.rs`、`src/event/input.rs`。
- **错误原因**：编辑键被模态面板吞掉，但剪贴板走 `active_editor` 直接改草稿；回执的 `input_at_open` 比对因此失真。
- **修复**：主/详情 rewind 执行中或 `SlashPanel::Rewind` 打开时吞掉粘贴与剪切。

### P2-5 未知命令清空草稿，已知命令误报未知

- **位置**：`src/app/update/chat/composer.rs::try_submit`。
- **错误原因**：未知命令分支先 `clear` 再提示；`/rewind` 带尾随换行/参数时走 `unknown_command`，文案错误且草稿丢失。
- **修复**：未知命令保留草稿（与主输入一致）；已知命令但非整段独立时提示 `Run this command on its own line`。

### P2-6 `revert_compact` 不校验更晚的用户轮

- **位置**：`src/db/chat/compact.rs::revert_compact`。
- **错误原因**：只校验“没有更晚的 active 检查点”，不校验检查点之后是否又出现用户轮；预检与执行之间的写入窗口会把上下文指针回退到新轮之前。
- **修复**：事务内增加 `agent_message` 最新用户轮检查，命中返回 `RewindTargetChanged`。
- **回归**：`revert_rejects_when_a_later_user_turn_exists`。

---

## P3 — 性能、组织与文档

### P3-1 `has_streaming_lx` 与 `advance_lx_loading` 的 BTW 判据不一致

- **位置**：`src/app/state.rs`、`src/app/update/chat/mod.rs`。
- **错误原因**：前者扫描所有线程，后者只看 active；后台线程在跑时每 100ms 空唤醒但不推进任何帧。
- **修复**：`has_streaming_lx` 只看当前显示线程；隐藏线程的完成由事件回投。

### P3-2 隐藏详情压缩仍以 10Hz 全量重绘

- **位置**：`src/app/state/lx/detail.rs::tick_detail_compacting`、`AppState::has_streaming_lx`。
- **说明**：即使 Agents 面板关闭，详情压缩仍每秒推进 10 次并置 dirty。耗时显示需要刷新，但视觉动画可只在可见时推进。
- **状态**：**遗留**。需要区分“更新耗时”与“推进可见动画”，改动面较大；当前不影响正确性，只浪费 CPU。

### P3-3 `is_animating()` 每次构造历史源都扫消息

- **位置**：`src/app/state/lx/chat.rs`。
- **说明**：`HistorySource` 在绘制、几何与命中路径会被多次构造，大 transcript 下是 O(n) × 多次。
- **缓解**：改为从消息尾部反向扫描；流式 assistant 通常在末尾，常态 O(1)。彻底方案是缓存 `streaming_messages` 计数，暂不做。

### P3-4 生产文件行数

- **说明**：`src/app/state/lx/chat.rs`（1077）与 `src/app/update/chat/subagent.rs`（1043）在本分支前已超过 1000 行，本分支只做了少量增量。按规范应拆分为 `chat/animation.rs` 等子模块，但不属于本次 review 的功能修复。
- **状态**：**遗留**（分支前既存）。

### P3-5 文档曾声称自动压缩不写数据库

- **位置**：`design.md` / `verification.md` / 代码注释。
- **错误原因**：自动压缩实际通过 `commit_compact` 落库；旧文档误导了 `/rewind` 的目标判断。
- **修复**：文档与注释改为“自动压缩也写检查点表，但按 `autocompact-` 前缀跳过，不可撤销”。

### P3-6 测试命名与覆盖

- **说明**：`compact_detail_empty_reports_nothing_to_compact` 实际覆盖的是“空白摘要导致 Failed”，不是 `CompactOutcome::Empty`；BTW compact 撤销此前没有事件层用例。
- **状态**：本次补了自动检查点、重入、读回失败、错误线程、多行命令、steer 重投、粘贴切换等回归；`Empty` 路径与 BTW compact 撤销仍建议后续补齐。

---

## 遗留项（明确不本次修复）

1. **隐藏详情压缩的 10Hz 刷新**：正确性无影响，优化需要把“耗时更新”与“可见动画”拆开。
2. **`chat.rs` / `subagent.rs` 超 1000 行**：分支前既存；拆分属于结构重构，不与本次功能修复混做。
3. **BTW `read` 工具停在 pending**：实机探针发现工具执行/权限链路问题，与本次动画/命令改动无关，未处理。
4. **主聊天也受自动检查点影响**：本次已用 `LxCompact::rewindable()` 一并修复；但历史会话里已存在的自动检查点仍会作为上下文基线，这是预期数据。
5. **`CompactOutcome::Empty` 与 BTW compact 撤销的事件层用例**：现有 `compact_detail_empty_reports_nothing_to_compact` 实际覆盖 Failed 路径，建议后续补真正的 Empty 与 BTW compact 回执测试。

---

## 验证

- `cargo fmt`
- `cargo clippy --all-targets -- -D warnings`（0 warning）
- `cargo test`：lib **5622 passed / 6 ignored**，集成测试全通过。
- 新增回归：`auto_compact_checkpoint_is_not_a_rewind_target`、`multiline_draft_does_not_execute_last_line_command`、`btw_rewind_receipt_targets_the_planned_thread_only`、`closing_the_page_keeps_detail_rewind_execution`、`compact_detail_rejects_reentry_while_compacting`、`undelivered_steer_after_report_is_resent_privately`、`btw_search_paste_loads_the_switched_thread`、`detail_compact_finished_without_readback_clears_running`、`revert_rejects_when_a_later_user_turn_exists`。
- TUI 交互/外观仍需用户按 `verification.md` §4 实机确认。
