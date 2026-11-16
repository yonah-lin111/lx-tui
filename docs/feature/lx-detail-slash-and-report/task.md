# 任务清单：Agents / BTW 详情输入框 `/` 命令与私聊·汇报重做

- **状态**：**T1–T12 已完成（`detail-slash-report` 分支）；自动化与 CLI 实机验证见 [`verification.md`](verification.md)，TUI 交互/外观项待用户按 §4 实机确认。2026-11-15 追加：详情 `/undo` 更名为 `/rewind`（保留 `/undo` 隐藏别名），动作面板与主聊天对齐，并支持 compact 检查点撤销；同日修复详情压缩无心跳/无计时、后台详情滚动条沿用滞后几何两个 bug，并将主会话/子 agent/BTW 的历史动画判据统一到 `LxChat::is_animating()`。2026-11-16 完成代码 review 与 P0–P3 修复，见 [`review.md`](review.md)。**
- **设计文档**：[`design.md`](design.md)
- **范围**：`src/app/state/lx/`、`src/app/update/`、`src/event/chat/`、`src/db/chat/subagent/`、`src/ui/main_content/lx/`、`src/ui/text/`、`src/chat/agent/report.rs`
- **数据库**：**无迁移**（复用 `notify_parent`、`delete_btw_thread`、`commit_rewind`）。

| 任务 | 内容 | 关键文件 | 验收 |
| :--- | :--- | :--- | :--- |
| **T1** | 详情命令模型：`DetailCommand` / `DetailContext` / 候选过滤 / 整段解析 / 参数提示 | 新增 `src/app/state/lx/slash/composer.rs`；改 `src/app/state/lx/slash.rs`（`SlashEntry::Detail`、`pub mod composer`、`active_argument_hint` 委托） | Agents 只出 `/rewind /compact /report`，BTW 只出 `/rewind /compact /new`；`/report` 支持多行备注；`/rewind x` 不误判为命令；主输入 `/undo` 仍是 `/rewind` 别名 |
| **T2** | composer `/` 面板生命周期：开关、刷新、按键、鼠标、进出编辑/切 tab 清理 | 新增 `src/app/update/chat/composer.rs`；改 `src/app/state/lx.rs`（`slash_enabled`）、`src/app/state/lx/panels.rs`（浮层互斥）、`src/app/update/chat/slash.rs`（分派）、`src/app/update/pane/{task_page.rs,popups.rs}`、`src/app/update/chat/btw/input.rs`、`src/ui/main_content/lx/slash.rs`（`items` 增加 `Detail` 分支） | 在两个 composer 输入 `/` 出现面板；`↑↓/Tab/Enter/Esc` 与鼠标一致；`@` 不与 `/` 抢键；离开编辑后不再残留 |
| **T3** | `/report`：报告请求消息、提交效应、普通追问/steer 永远私聊、删除 `AgentReplyMode` | 新增 `src/app/update/chat/detail_report.rs`；改 `src/chat/agent/report.rs`、`src/app/update/pane/task_page.rs`、`src/app/state/lx/agent.rs`、`src/ui/main_content/lx/task/agents.rs`、`src/ui/text.rs` | 私聊多轮不产生报告；`/report note` 只把报告轮正文交给主 agent；主 agent 自动起轮；胶囊显示 `private` / `steer N` / `compacting`；`Shift+Tab` 不再切模式 |
| **T4** | `/rewind`（Agents）：可撤回判据、预检、四项动作面板、轮次/compact 目标、执行、回投 | 新增 `src/app/state/lx/detail.rs`（`DetailTarget`/`DetailRewindPlan`）、`src/app/update/chat/detail_rewind.rs`、`src/event/chat/detail_rewind.rs`；改 `src/db/chat/subagent/queue.rs`、`src/app/state/lx/rewind.rs`（`RewindPanel::for_detail`/`for_detail_compact`/`RewindScope::Detail`）、`src/app/state/lx/task.rs`、`src/event/chat/rewind.rs`（回执分流）、`src/ui/main_content/lx/slash.rs`（`Detail` scope 文案） | 默认第一项 `Restore files and undo conversation`；快照不可用时含文件动作禁用但面板照常打开；确认后删除最后一轮私聊 + 恢复该轮文件；spawn 轮/报告轮/父 `send_input` 轮不可撤回；`/compact` 后 `/rewind` 恢复检查点；迟到回执不覆盖新操作 |
| **T5** | `/rewind`（BTW）：最新问答与 compact 检查点撤销；删空后线程重置 | 复用 T4 管线；改 `src/app/state/lx/btw.rs`（`reset_after_delete`）、`src/app/update/chat/detail_rewind.rs`、`src/event/chat/detail_rewind.rs` | `/rewind` 删除最后一轮问答或恢复检查点；删空时线程回到空白、列表 `● side thread` 消失；运行中拒绝 |
| **T6** | `/compact`（Agents + BTW）：详情压缩效应、检查点回投、进度 tick、BTW 检查点加载 | 新增 `src/event/chat/detail_compact.rs`；改 `src/app/effects.rs`、`src/event/chat/compact.rs`（`execute_compact` 可见性 + 回执分流）、`src/app/update/chat/mod.rs`（`advance_lx_loading`）、`src/app/update/chat/compact.rs`、`src/event/chat/btw.rs`（`BtwLoadedView` + checkpoints）、`src/app/update/chat/btw/events.rs`、`src/ui/main_content/lx/task/{agents.rs,btw.rs}` | 空闲时压缩成功并出现检查点卡片；无增量提示 `Nothing new to compact.`；失败不改上下文；运行中/有排队时拒绝；`compacting` 徽标与耗时推进 |
| **T7** | BTW `/new`：删除线程、重置视图、刷新 QA 列表 | 新增 `src/event/chat/btw/maintenance.rs`；改 `src/app/effects.rs`（`BtwEffect::DeleteThread`）、`src/event/chat/btw.rs`（分派）、`src/app/update/chat/btw/{mod.rs,events.rs}`、`src/app/state/lx/btw.rs` | `/new` 直接清空当前 QA 线程；回答中拒绝；删除失败保留线程并 Toast |
| **T8** | 文案与键位：新增 `detail` 文案模块、底栏 `[/ commands]` | 新增 `src/ui/text/detail.rs`；改 `src/ui/text.rs`（只加 `pub mod detail;`）、`src/ui/text/btw.rs`、`src/ui/main_content/lx/task/{agents.rs,btw.rs}` | 所有新文案英文；窄宽度整项丢弃不截半截 |
| **T9** | 测试、校验与文档 | 见 §测试清单 | `cargo fmt` / `cargo clippy --all-targets -- -D warnings` / `cargo test` 全绿；实机清单通过；补 `verification.md` |
| **T12** | Review 修复（详见 `review.md`）：自动压缩检查点不进 `/rewind`；执行态迁到 `Lx.detail_rewind`；回执按目标线程/agent 回填且只写目标 composer；无参命令采纳校验整段独立；详情压缩禁止重入、读回失败结束 running；BTW 搜索粘贴切换线程加载历史；steer 重投保持私聊 | 改 `src/app/state/lx/{detail.rs,compact.rs,agent.rs}`、`src/app/update/chat/{composer.rs,detail_rewind.rs,detail_compact.rs,subagent.rs}`、`src/event/chat/{detail_rewind.rs,detail_compact.rs,rewind.rs,subagent/settle.rs}`、`src/db/chat/compact.rs`；测试见 `detail_commands.rs`、`detail_rewind.rs`、`detail_compact.rs`、`subagent_private.rs`、`task_page.rs`、`compact/tests.rs` | 自动检查点不再抢占最后一轮；多行草稿不执行末行命令；BTW 删空只重置目标线程并刷新 QA 列表；压缩中再次 `/compact` 被拒；读回失败不留 `compacting`；私聊 steer 不因报告轮被转发 |
| **T11** | 三处历史动画对齐：新增 `LxChat::is_animating()`（流式状态 / 请求在途 / 本地命令 / 流式 assistant 投影），主会话、子 agent 详情与 BTW 线程统一用它决定步骤标记呼吸 | 改 `src/app/state/lx/chat.rs`、`src/ui/main_content/lx/history/source.rs`；测试见 `task/tests/btw_list.rs` | BTW 线程生成中工具方块/圆点随 `loading_frame` 呼吸，终态恒实心；主会话、子 agent 与 BTW 三相节奏一致 |
| **T10** | 详情压缩心跳与后台滚动几何修复：`has_streaming_lx` 纳入详情压缩/懒加载，心跳推进 `run.chat.loading_frame` 与 BTW 线程帧，聊天栏显示 `Compacting context... (Ns)`；滚动条按下/拖拽改用 `ScrollbarLayout` 的 `total`/`visible` | 改 `src/app/state.rs`、`src/app/state/lx/{detail.rs,btw.rs}`、`src/app/update/chat/{mod.rs,detail_compact.rs}`、`src/app/update/chat/{subagent.rs,btw/mod.rs}`、`src/event/mouse/{hit.rs,drag.rs}`、`src/ui/main_content/lx/chat_bar.rs`；测试见 `detail_commands.rs`、`subagent.rs`、`btw/tests.rs`、`chat_bar/tests/compacting.rs`、`task/tests/agent_list.rs` | 详情 `/compact` 期间菱形呼吸、耗时逐秒推进、聊天栏可见；压缩结束恢复检查点；滚动条首次拖拽/点击即按当前几何生效，无“卡一下再滚一次” |

---

## 文件与模块拆分（遵守 1000 行上限）

| 新文件 | 职责 | 预计行数 |
| :--- | :--- | ---: |
| `src/app/state/lx/slash/composer.rs` | 详情命令枚举、候选过滤、整段解析、参数提示 | ~180 |
| `src/app/state/lx/slash/composer/tests.rs` | 解析/候选单测 | ~160 |
| `src/app/state/lx/detail.rs` | `DetailTarget`、`DetailRewindPlan/Execution`、`DetailRewindKind` | ~80 |
| `src/app/update/chat/composer.rs` | composer `/` 面板刷新、按键、采纳、命令分派 | ~260 |
| `src/app/update/chat/detail_report.rs` | `/report` 提交与文案 | ~90 |
| `src/app/update/chat/detail_rewind.rs` | 撤销预检回投、成功回填、草稿恢复 | ~180 |
| `src/event/chat/detail_rewind.rs` | 目标解析、`execute_rewind` 执行、回执处理 | ~260 |
| `src/event/chat/detail_compact.rs` | 详情压缩准备、完成回投、状态回填 | ~200 |
| `src/event/chat/btw/maintenance.rs` | `/new` 删除线程 | ~70 |
| `src/ui/text/detail.rs` | 新增英文文案 | ~50 |
| 测试新文件 | 见下 | ~600 |

**禁止继续增长的文件**（已超/接近 1000 行）：

- `src/ui/text.rs`（1015）→ 只加 `pub mod detail;`
- `src/db/chat/subagent.rs`（1050）→ 新查询放 `subagent/queue.rs`
- `src/event/chat/btw.rs`（1335）→ 新逻辑放 `btw/maintenance.rs`；只允许新增 `BtwEvent::ThreadDeleted` 枚举变体（约 5 行），不得再塞函数体
- `src/event/mod.rs`（989）→ **不加新 `AppEvent` 变体**，详情回执复用 `RewindFinished` / `CompactFinished` 并按 operation id 分流
- `src/app/update/chat/slash.rs`（903）→ 详情逻辑全部外移
- `src/app/state/lx/slash.rs`（981）→ 只加 `pub mod composer;` 与 `SlashEntry::Detail`
- `src/app/update/pane/task_page.rs`（856）→ 只保留分派调用，具体逻辑在 `chat/composer.rs`
- `src/app/update/chat/mod.rs`（>1000）→ `advance_lx_loading` 只加 5 行接线，细节放 `app/state/lx/detail.rs`

---

## 测试清单

### 状态/解析

- `src/app/state/lx/slash/composer/tests.rs`
  - `agent_context_lists_rewind_compact_report_only`
  - `btw_context_lists_rewind_compact_new_only`
  - `report_takes_multiline_note`
  - `non_report_commands_reject_arguments`
  - `btw_new_accepts_clear_and_reset_aliases`
  - `argument_hint_only_for_report_prefix`
  - `main_context_does_not_expose_detail_commands`

### App/输入

- `src/app/update/chat/tests/detail_commands.rs`
  - `moving_cursor_refreshes_slash_candidates`
  - `multiline_draft_does_not_execute_last_line_command`
  - `btw_rewind_receipt_targets_the_planned_thread_only`
  - `btw_search_paste_loads_the_switched_thread`
  - `detail_compact_finished_without_readback_clears_running`
  - `typing_slash_in_agent_composer_opens_three_commands`
  - `typing_slash_in_btw_composer_opens_three_commands`
  - `enter_accepts_report_and_echoes_argument_hint`
  - `escape_closes_panel_and_keeps_edit_mode`
  - `slash_panel_is_closed_when_leaving_edit_or_switching_tab`
  - `unknown_slash_is_not_sent_to_model`
  - `paste_into_composer_refreshes_slash_panel`
  - `report_idle_agent_emits_send_input_with_notify_parent_true`
  - `report_running_agent_is_rejected_without_effect`
  - `private_follow_up_never_emits_report`
  - `rewind_prepare_shows_four_actions_with_restore_first`
  - `rewind_prepare_for_compact_shows_restore_and_cancel`
  - `rewind_confirm_cancel_has_no_effect`
  - `rewind_confirm_default_enter_emits_execute_effect`
  - `rewind_confirm_disabled_file_action_is_swallowed`
  - `rewind_busy_agent_does_not_open_confirm`
  - `rewind_command_and_hidden_alias_emit_prepare_effect`
  - `compact_command_emits_detail_effect_once`
  - `late_detail_compact_finished_is_ignored`

### DB/事件

- `src/db/chat/compact/tests.rs`
  - `revert_rejects_when_a_later_user_turn_exists`
- `src/db/chat/subagent/tests/rewind.rs`
  - `latest_undoable_turn_requires_human_panel_follow_up`
  - `spawn_input_and_parent_send_input_and_report_turns_are_not_undoable`
- `src/app/update/chat/tests/task_page.rs`
  - `closing_the_page_keeps_detail_rewind_execution`
- `src/event/chat/tests/detail_rewind.rs`
  - `auto_compact_checkpoint_is_not_a_rewind_target`
  - `detail_rewind_deletes_the_last_follow_up_and_reloads_the_run`
  - `detail_rewind_disables_file_actions_when_snapshot_is_unavailable`
  - `detail_rewind_ignores_late_operation_id`
  - `detail_rewind_rejects_busy_agent_and_btw_thread`
  - `detail_rewind_restores_the_latest_compact_checkpoint`
  - `btw_undo_deletes_the_last_turn_and_keeps_the_thread`
  - `btw_undo_of_the_only_turn_resets_the_thread`
- `src/event/chat/tests/detail_compact.rs`
  - `compact_detail_rejects_reentry_while_compacting`
  - `compact_detail_commits_checkpoint_for_child_session`
  - `compact_detail_empty_reports_nothing_to_compact`
  - `compact_detail_busy_agent_is_rejected`
  - `btw_compact_persists_checkpoint_and_reloads_thread`
- `src/event/chat/tests/subagent_private.rs`
  - `report_turn_body_excludes_previous_private_chat`
  - `undelivered_steer_after_report_is_resent_privately`

### BTW 重置

- `src/app/update/chat/btw/tests.rs`
  - `btw_new_emits_delete_thread_and_rejects_while_loading`
  - `thread_deleted_resets_view_and_qalist_mark`

### 渲染/文案

- `src/ui/main_content/lx/task/tests/detail_slash.rs`
  - `agent_badge_shows_private_then_steer_then_compacting`
  - `btw_badge_appends_compacting_only_when_it_fits`
  - `edit_footer_shows_commands_hint`
  - `slash_panel_anchors_above_composer`
- `src/ui/main_content/lx/tests/slash_panels.rs`
  - `detail_commands_render_inline_labels_and_tags`
  - `detail_rewind_panels_render_scope_and_actions`

### 回归

- 主输入 `/`、`/rewind`、`/compact`、AI Web 命令裁剪不受影响。
- 主输入 `/undo` 仍走 `/rewind` 别名。

---

## 执行顺序

1. **T1 + T8**：先落命令模型与文案，纯状态、可独立测试。
2. **T2**：接面板生命周期与按键；用现有 `slash_panels` 测试守住主输入回归。
3. **T3**：删模式、加 `/report`；先保证“私聊永不报告”。
4. **T4 + T5**：撤销（Agents 先，BTW 复用）。
5. **T6**：详情压缩（先 Agents，后 BTW 检查点加载）。
6. **T7**：BTW `/new`。
7. **T9**：全量校验 + 实机复核 + `verification.md`。

每个任务完成即跑对应模块测试，不等到最后。

---

## 验证命令

```bash
cargo fmt
cargo clippy --all-targets -- -D warnings
cargo test
```

按影响范围的最小校验：

```bash
cargo test --lib app::state::lx::slash
cargo test --lib app::update::chat
cargo test --lib event::chat
cargo test --lib ui::main_content::lx
cargo test --lib db::chat::subagent
```

---

## 实机验收（TUI，用户执行 `cargo run`）

1. 打开一个已完成 subagent（`/subagents`），`Shift+Enter` 进入输入框；输入 `/`，只看到 `/rewind` `/compact` `/report`。
2. 连续私聊 3 轮（内容故意写“今天天气不错”之类）；确认主 agent 没有任何报告/自动轮。
3. 输入 `/report 把刚才定位的登录 bug 结论汇报给主 agent`；确认子 agent 输出一份报告，主 agent 只收到报告正文并自动回复；之前的闲聊不在报告里。
4. 让 Turing 私聊改一个文件；`/rewind` → 四项动作面板默认第一项；Enter 后文件恢复、最后一轮问答消失；`/undo` 别名同样可执行。
5. 对 spawn 的任务轮或 `/report` 轮执行 `/rewind`：直接提示 `Nothing to rewind`，不删任何东西。
6. 私聊多轮后 `/compact`：胶囊显示 `compacting`，完成后出现压缩卡片；再执行一次提示 `Nothing new to compact.`。
7. BTW 页：对一个 QA 问几轮，`/rewind` 删除最后一轮；`/new` 清空线程，列表 `● side thread` 消失；回答中执行 `/new` 被拒绝；`/compact` 后再 `/rewind` 走 compact 恢复面板。
8. 窄窗格（约 60 列）下：胶囊、确认面板、底栏提示都不截半截；所有文案英文。

---

## 完成定义（DoD）

- 设计文档中的验收项全部满足（含详情 `/rewind` 与主聊天动作面板对齐、compact 检查点撤销）。
- 新增/修改模块的测试全部通过，且没有把任何文件推过 1000 行。
- 主输入 `/` 命令、`/rewind`、`/compact`、AI Web 命令列表无回归。
- `cargo fmt` / `cargo clippy --all-targets -- -D warnings` / `cargo test` 全绿。
- 补充 `docs/feature/lx-detail-slash-and-report/verification.md`，记录命令、测试数量与实机结果。
- 完成 `review.md` 的 P0–P3 修复与回归测试；已知 P3（更早的自动检查点仍会作为上下文基线、隐藏详情压缩仍按 10Hz 更新耗时、`chat.rs` / `subagent.rs` 在此分支前已超 1000 行）记录在 `review.md` 的遗留项。
