# 任务清单：Agents / BTW 详情输入框 `/` 命令与私聊·汇报重做

- **状态**：**待用户审核；审核通过前不执行任何代码改动。**
- **设计文档**：[`design.md`](design.md)
- **范围**：`src/app/state/lx/`、`src/app/update/`、`src/event/chat/`、`src/db/chat/subagent/`、`src/ui/main_content/lx/`、`src/ui/text/`、`src/chat/agent/report.rs`
- **数据库**：**无迁移**（复用 `notify_parent`、`delete_btw_thread`、`commit_rewind`）。

| 任务 | 内容 | 关键文件 | 验收 |
| :--- | :--- | :--- | :--- |
| **T1** | 详情命令模型：`DetailCommand` / `DetailContext` / 候选过滤 / 整段解析 / 参数提示 | 新增 `src/app/state/lx/slash/composer.rs`；改 `src/app/state/lx/slash.rs`（`SlashEntry::Detail`、`pub mod composer`、`active_argument_hint` 委托） | Agents 只出 `/undo /compact /report`，BTW 只出 `/undo /compact /new`；`/report` 支持多行备注；`/undo x` 不误判为命令；主输入 `/undo` 仍是 `/rewind` 别名 |
| **T2** | composer `/` 面板生命周期：开关、刷新、按键、鼠标、进出编辑/切 tab 清理 | 新增 `src/app/update/chat/composer.rs`；改 `src/app/state/lx.rs`（`slash_enabled`）、`src/app/state/lx/panels.rs`（浮层互斥）、`src/app/update/chat/slash.rs`（分派）、`src/app/update/pane/{task_page.rs,popups.rs}`、`src/app/update/chat/btw/input.rs`、`src/ui/main_content/lx/slash.rs`（`items` 增加 `Detail` 分支） | 在两个 composer 输入 `/` 出现面板；`↑↓/Tab/Enter/Esc` 与鼠标一致；`@` 不与 `/` 抢键；离开编辑后不再残留 |
| **T3** | `/report`：报告请求消息、提交效应、普通追问/steer 永远私聊、删除 `AgentReplyMode` | 新增 `src/app/update/chat/detail_report.rs`；改 `src/chat/agent/report.rs`、`src/app/update/pane/task_page.rs`、`src/app/state/lx/agent.rs`、`src/ui/main_content/lx/task/agents.rs`、`src/ui/text.rs` | 私聊多轮不产生报告；`/report note` 只把报告轮正文交给主 agent；主 agent 自动起轮；胶囊显示 `private` / `steer N` / `compacting`；`Shift+Tab` 不再切模式 |
| **T4** | `/undo`（Agents）：可撤回判据、预检、两行确认、执行、回投 | 新增 `src/app/state/lx/detail.rs`（`DetailTarget`/`DetailUndoPending`）、`src/app/update/chat/detail_undo.rs`、`src/event/chat/detail_undo.rs`；改 `src/db/chat/subagent/queue.rs`、`src/app/state/lx/rewind.rs`（`RewindPanel::for_detail`/`RewindScope::Detail`）、`src/app/state/lx/task.rs`、`src/event/chat/rewind.rs`（回执分流）、`src/ui/main_content/lx/slash.rs`（`Detail` scope 文案） | 默认高亮 `Cancel`；确认后删除最后一轮私聊 + 恢复该轮文件；spawn 轮/报告轮/父 `send_input` 轮不可撤回；快照不可用直接拒绝；迟到回执不覆盖新操作 |
| **T5** | `/undo`（BTW）：对话-only 撤销；删空后线程重置 | 复用 T4 管线；改 `src/app/state/lx/btw.rs`（`reset_after_delete`）、`src/app/update/chat/detail_undo.rs` | `/undo` 删除最后一轮问答；删空时线程回到空白、列表 `● side thread` 消失；运行中拒绝 |
| **T6** | `/compact`（Agents + BTW）：详情压缩效应、检查点回投、进度 tick、BTW 检查点加载 | 新增 `src/event/chat/detail_compact.rs`；改 `src/app/effects.rs`、`src/event/chat/compact.rs`（`execute_compact` 可见性 + 回执分流）、`src/app/update/chat/mod.rs`（`advance_lx_loading`）、`src/app/update/chat/compact.rs`、`src/event/chat/btw.rs`（`BtwLoadedView` + checkpoints）、`src/app/update/chat/btw/events.rs`、`src/ui/main_content/lx/task/{agents.rs,btw.rs}` | 空闲时压缩成功并出现检查点卡片；无增量提示 `Nothing new to compact.`；失败不改上下文；运行中/有排队时拒绝；`compacting` 徽标与耗时推进 |
| **T7** | BTW `/new`：删除线程、重置视图、刷新 QA 列表 | 新增 `src/event/chat/btw/maintenance.rs`；改 `src/app/effects.rs`（`BtwEffect::DeleteThread`）、`src/event/chat/btw.rs`（分派）、`src/app/update/chat/btw/{mod.rs,events.rs}`、`src/app/state/lx/btw.rs` | `/new` 直接清空当前 QA 线程；回答中拒绝；删除失败保留线程并 Toast |
| **T8** | 文案与键位：新增 `detail` 文案模块、底栏 `[/ commands]` | 新增 `src/ui/text/detail.rs`；改 `src/ui/text.rs`（只加 `pub mod detail;`）、`src/ui/text/btw.rs`、`src/ui/main_content/lx/task/{agents.rs,btw.rs}` | 所有新文案英文；窄宽度整项丢弃不截半截 |
| **T9** | 测试、校验与文档 | 见 §测试清单 | `cargo fmt` / `cargo clippy --all-targets -- -D warnings` / `cargo test` 全绿；实机清单通过；补 `verification.md` |

---

## 文件与模块拆分（遵守 1000 行上限）

| 新文件 | 职责 | 预计行数 |
| :--- | :--- | ---: |
| `src/app/state/lx/slash/composer.rs` | 详情命令枚举、候选过滤、整段解析、参数提示 | ~180 |
| `src/app/state/lx/slash/composer/tests.rs` | 解析/候选单测 | ~160 |
| `src/app/state/lx/detail.rs` | `DetailTarget`、`DetailUndoPending/Execution`、`DetailCompactKind` | ~80 |
| `src/app/update/chat/composer.rs` | composer `/` 面板刷新、按键、采纳、命令分派 | ~260 |
| `src/app/update/chat/detail_report.rs` | `/report` 提交与文案 | ~90 |
| `src/app/update/chat/detail_undo.rs` | 撤销预检回投、成功回填、草稿恢复 | ~180 |
| `src/event/chat/detail_undo.rs` | 目标解析、`execute_rewind` 执行、回执处理 | ~260 |
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
  - `agent_context_lists_undo_compact_report_only`
  - `btw_context_lists_undo_compact_new_only`
  - `report_takes_multiline_note`
  - `non_report_commands_reject_arguments`
  - `btw_new_accepts_clear_and_reset_aliases`
  - `argument_hint_only_for_report_prefix`
  - `main_context_does_not_expose_detail_commands`

### App/输入

- `src/app/update/chat/tests/detail_commands.rs`
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
  - `undo_prepare_shows_two_row_confirm_with_cancel_first`
  - `undo_confirm_cancel_has_no_effect`
  - `undo_confirm_down_enter_emits_execute_effect`
  - `undo_busy_agent_does_not_open_confirm`
  - `compact_command_emits_detail_effect_once`
  - `late_detail_compact_finished_is_ignored`

### DB/事件

- `src/db/chat/subagent/tests/undo.rs`
  - `latest_undoable_turn_requires_human_panel_follow_up`
  - `spawn_input_and_parent_send_input_and_report_turns_are_not_undoable`
- `src/event/chat/tests/detail_undo.rs`
  - `detail_undo_restores_files_and_deletes_the_last_follow_up`
  - `detail_undo_rejects_when_files_snapshot_is_unavailable`
  - `detail_undo_ignores_late_operation_id`
  - `btw_undo_deletes_the_last_turn_and_keeps_the_thread`
  - `btw_undo_of_the_only_turn_resets_the_thread`
- `src/event/chat/tests/detail_compact.rs`
  - `compact_detail_commits_checkpoint_for_child_session`
  - `compact_detail_empty_reports_nothing_to_compact`
  - `compact_detail_busy_agent_is_rejected`
  - `btw_compact_persists_checkpoint_and_reloads_thread`
- `src/event/chat/tests/subagent_private.rs`
  - `report_turn_body_excludes_previous_private_chat`

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

### 回归

- 主输入 `/`、`/rewind`、`/compact`、AI Web 命令裁剪不受影响。
- `/undo` 在主输入仍走 `/rewind` 别名。

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

1. 打开一个已完成 subagent（`/subagents`），`Shift+Enter` 进入输入框；输入 `/`，只看到 `/undo` `/compact` `/report`。
2. 连续私聊 3 轮（内容故意写“今天天气不错”之类）；确认主 agent 没有任何报告/自动轮。
3. 输入 `/report 把刚才定位的登录 bug 结论汇报给主 agent`；确认子 agent 输出一份报告，主 agent 只收到报告正文并自动回复；之前的闲聊不在报告里。
4. 让 Turing 私聊改一个文件；`/undo` → 确认面板默认 `Cancel`；直接 Enter 无变化；`↓ ↵` 后文件恢复、最后一轮问答消失。
5. 对 spawn 的任务轮或 `/report` 轮执行 `/undo`：直接提示 `Nothing to undo`，不删任何东西。
6. 私聊多轮后 `/compact`：胶囊显示 `compacting`，完成后出现压缩卡片；再执行一次提示 `Nothing new to compact.`。
7. BTW 页：对一个 QA 问几轮，`/undo` 删除最后一轮；`/new` 清空线程，列表 `● side thread` 消失；回答中执行 `/new` 被拒绝。
8. 窄窗格（约 60 列）下：胶囊、确认面板、底栏提示都不截半截；所有文案英文。

---

## 完成定义（DoD）

- 设计文档中的 8 条验收全部满足。
- 新增/修改模块的测试全部通过，且没有把任何文件推过 1000 行。
- 主输入 `/` 命令、`/rewind`、`/compact`、AI Web 命令列表无回归。
- `cargo fmt` / `cargo clippy --all-targets -- -D warnings` / `cargo test` 全绿。
- 补充 `docs/feature/lx-detail-slash-and-report/verification.md`，记录命令、测试数量与实机结果。
