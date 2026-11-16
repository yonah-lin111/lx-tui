# 任务清单：主 agent 停止面板、用户主动打断语义与 subagent/BTW 报告填充

> 状态：**已实施完成（分支 `feature/lx-stop-panel-and-report-fill`），待用户实机验收后合并**。
> 规则：生产代码禁止 `unwrap()/expect()`；所有用户可见文案集中在 `src/ui/text.rs` / `src/ui/text/detail.rs`；模型可见文本用英文；先补复现测试，再改实现。
>
> **2026-11-16 修订**：`/report` 已改为与 `/compact` 同管线的一次性总结（§8）。下方 §1-§7 中关于“报告轮 / report_to_input / BTW 多目标批次”的描述仅保留为实施历史，以 §8 与代码为准。
>
> 实施差异（以代码为准）：
> - 报告回执由事件层直接调用 `app::update::on_subagent_report_ready` / `on_btw_report_ready`，没有反向 `Effect::ReportReady`。
> - BTW 报告通过 `BtwSubmission.report_to_main_input` + `event/chat/btw/run.rs` 的批次聚合实现，没有单独的 `SendBtwReport` 效应。
> - `ChatStreams.report_to_input` 按 `agent_id` 标记；事件层额外校验 DB 队列，确保报告轮排在空闲后的队首。
> - 用户停止状态通过 `event/chat/subagent/lifecycle.rs` 维护；BTW 起停与结算拆到 `event/chat/btw/run.rs`，满足单文件行数上限。

---

## 0. 前置确认

- [ ] 用户审核 `design.md` 的 21 项决策与 §9 三项待确认。
- [ ] 确认只改当前 `dev` 工作区，不新建 worktree、不改 Git 状态。
- [ ] 确认无数据库迁移；`user_stopped`、`report_to_input`、`stopped_by_user` 仅内存。

---

## 1. 文件与模块拆分

| 文件 | 变更 | 预计行数 |
|---|---|---|
| `src/app/state/lx/stop.rs`（新） | `StopAction` / `StopAvailability` / `StopPanel` | ~180 |
| `src/app/state/lx/report.rs`（新） | `ReportAction` / `ReportPanel` | ~120 |
| `src/app/state/lx/slash.rs` | `SlashPanel::{Stop,Report}`、item_count/window/move 等 | +60 |
| `src/app/state/lx/agent.rs` | `SubagentRun.stopped_by_user`、`stoppable_count` | +40 |
| `src/app/update/chat/stop.rs`（新） | `apply_enter`、停止动作分派、计数 Toast | ~180 |
| `src/app/update/chat/detail_report.rs` | `/report` 重写：二级面板、Resend/Regenerate | ~260 |
| `src/app/update/chat/report_fill.rs`（新） | 主输入框芯片插入/替换 | ~160 |
| `src/app/update/chat/mod.rs` | `escape_stop` 重写、`STOP_ARM_WINDOW`、模块声明 | ~40 |
| `src/app/update/chat/rewind.rs` | `arm()` 避让、删除撤销提示 | -6 |
| `src/app/update/pane.rs` | `SlashPanel::Stop` Esc 分支 | +4 |
| `src/app/update/pane/task_page.rs` | `stopped_by_user` 允许编辑/发送 | +10 |
| `src/app/update/chat/task_page.rs` | `stop_selected_subagent` 改效应 | +10 |
| `src/app/update/chat/composer.rs` | 详情 composer Report 面板路由 | +30 |
| `src/app/effects.rs` | `StopSubagentsByUser` / `StopAllSessionProcesses` / `SendSubagentReport` / `SendBtwReport` / `CancelUserShells.scope` | +40 |
| `src/event/chat/subagent/mod.rs` | `user_stopped`、`stop_subagents_by_user`、`SubagentHandle.stopped_by_user` | ~220 |
| `src/event/chat/subagent/runner.rs` | `send_input`/`resume` 唤醒、`report_to_input` 传递 | ~80 |
| `src/event/chat/subagent/settle.rs` | 用户停止不产报告、报告轮捕获 | ~90 |
| `src/event/chat/subagent/snapshot.rs` | panel 输入允许 `user_stopped`、list 结果标记 | ~60 |
| `src/event/chat/subagent/wait.rs` | `stopped_by_user` 投影 | ~20 |
| `src/event/chat/btw.rs` | `SendBtwReport`、多目标报告聚合 | ~180 |
| `src/event/chat/btw/*` | `BtwSubmission.report_to_main_input`、batch 结算 | ~80 |
| `src/event/chat/process_page.rs` | `StopAllSessionProcesses` | ~50 |
| `src/event/chat/mod.rs` | `ChatStreams.user_stopped` / `report_to_input` | +10 |
| `src/db/chat/subagent/queue.rs` | `cancel_queued_inputs_for_agent` | ~50 |
| `src/chat/agent/mod.rs` | `AgentStatusView.stopped_by_user`、serialization | ~40 |
| `src/chat/agent/report.rs` | `PANEL_REPORT_MARKER`、报告请求正文 | ~30 |
| `src/chat/prompt.rs` | 用户主动停止规则、`PROMPT_VERSION` lx-15 | +6 |
| `src/app/paste/folded.rs` | 两个 `FoldedKind`、`source`、parser/escape、`encode_report` | ~180 |
| `src/app/paste.rs` | 重新导出 `encode_report` / 新 `FoldedKind` | +4 |
| `src/app/state/lx/folded.rs` | `Lx::insert_report_chip`（snapshot/光标/顶部插入/同源替换） | ~80 |
| `src/ui/style.rs` | 两种报告芯片配色 | +10 |
| `src/ui/main_content/lx/slash.rs` | Stop/Report items/view/title/footer | ~120 |
| `src/ui/text.rs` / `src/ui/text/detail.rs` | 新增/删除文案 | +60 |
| `src/app/update/chat/user_shell.rs` | `CancelUserShells.scope` 支持 ShellsOnly | ~40 |

> 新文件超过 500 行时按职责再拆；生产文件不得超过 1000 行。

---

## 2. 执行顺序

### 阶段 A：折叠芯片底座（独立、可先测）

1. `FoldedKind` 增加 `SubagentReport` / `BtwReport`：
   - `tag_name` / `chip_word` / `label`；
   - `CLOSE_TAG_SUFFIXES` 增加两个闭合标签；
   - `escape_body` / `unescape_body` 覆盖新标签。
2. `FoldedToken` 增加 `source: Option<String>`：
   - `compute_id` 纳入 source；
   - 现有 `encode_pasted_kind` 产 `None`。
3. 新 `encode_report(kind, source, body)`：
   - `<pasted-subagent-report source="..." lines="N">`；
   - `<pasted-btw-report source="..." lines="N">`；
   - source 做最小转义/拒绝非法字符。
4. 解析器：
   - 新分支解析 source 属性；
   - 旧 text/code/log 语法不变；
   - 报告 token 的 `body/line_count/range` 正确。
5. `style::pasted_chip` / `pasted_bg` 新配色。
6. 测试：
   - 往返、标签、单/多行、source、闭合标签转义；
   - 旧 token 回归；
   - 配色不重叠。

### 阶段 B：停止面板状态与 UI

1. 新增 `src/app/state/lx/stop.rs`：
   - `StopAction`、`StopAvailability`、`StopPanel`；
   - `from_lx` / `any` / `enabled` / `active_action`。
2. `Lx::stoppable_count`（或 `AgentsState::stoppable_count`）。
3. `SlashPanel::Stop` 变体与所有 match 补齐。
4. `ui/main_content/lx/slash.rs`：
   - Stop items（含禁用原因、计数 tag）；
   - 标题/底边/右上角作用域；
   - 默认高亮第一项。
5. `app/update/pane.rs` Esc 分支：
   - `SlashPanel::Stop(_) => lx.slash = None`，不清输入框。
6. `app/update/chat/slash.rs` `apply_slash_enter`：
   - `SlashPanel::Stop(_) => stop::apply_enter(...)`。
7. 测试：
   - 默认项、禁用、上下跳过、Esc 关闭、鼠标命中。

### 阶段 C：双击 Esc 路由与停止执行

1. `app/update/chat/mod.rs::escape_stop` 重写：
   - steer 召回优先；
   - `StopAvailability.any()`；
   - 第一次待命 + `LX_STOP_ARMED`；
   - 第二次打开 `SlashPanel::Stop`，不执行。
2. `rewind::arm()`：
   - 有可停止工作时返回 `None`；
   - 删除 `LX_REWIND_ARMED_HINT` 提示。
3. 新增 `src/app/update/chat/stop.rs`：
   - `apply_enter` 重新计算可用性；
   - 动作分派；
   - 关闭面板；
   - 计数 Toast。
4. 新效应：
   - `StopSubagentsByUser`；
   - `StopAllSessionProcesses`；
   - `CancelUserShells.scope`（All / ShellsOnly）。
5. 事件层：
   - `stop_subagents_by_user`（当前会话树、目标子树、取消队列、UI stopped）；
   - `stop_all_session_processes`；
   - `cancel_user_shells` 支持 scope。
6. 测试：
   - 主轮/子 agent/shell/process 四类组合；
   - `Stop all` / `Stop main` / `Stop subagents` / `Stop shells`；
   - 作用域不跨 pane；
   - 目标自然结束。

### 阶段 D：用户主动停止语义

1. `ChatStreams.user_stopped: HashSet<String>`。
2. `SubagentHandle.stopped_by_user` / `ChildHandle.stopped_by_user`。
3. `finish_subagent_with_billing`：
   - 用户停止不产报告；
   - 不设置 `paused_by_interrupt`/`resumed`。
4. `Event::StopSubagentsByUser` 处理：
   - `Some(id)` 子树；
   - `None` 全树；
   - `cancel_queued_inputs_for_agent`。
5. `AgentStatusView` / `AgentListItemView` 增加 `stopped_by_user`，JSON 仅 true 时输出。
6. `settle_wait` / `list_agents` 投影。
7. `send_input` / `resume` / `send_user_input_from_panel` 唤醒路径。
8. `SubagentRun.stopped_by_user` + UI `stopped` 文案。
9. `stop_selected_subagent` 改效应；`enter_task_agent_edit` / `submit_agent_edit` 放行 `stopped_by_user`。
10. `src/chat/prompt.rs` 新增规则；`PROMPT_VERSION = "lx-15"`；更新测试。
11. 测试：
    - wait/list 标记；
    - 不自动恢复；
    - 面板输入唤醒；
    - `send_input` 唤醒；
    - 队列取消；
    - 旧 close/session delete 行为不变。

### 阶段 E：Agents `/report` 与报告轮

1. `DetailCommand::BTW` 加 `Report`；`parse` 支持 BTW report。
2. `PANEL_REPORT_MARKER`；`report_request_message` 首行切换；`visible_task` 识别新旧 marker。
3. 新效应 `SendSubagentReport`。
4. 事件层：
   - 校验目标空闲/归属；
   - enqueue + `notify_parent=false`；
   - `ChatStreams.report_to_input` 按 submission id 标记；
   - `launch_child_turn` / handle `report_to_input`。
5. `finish_subagent_with_billing`：
   - 事件层直接调用 `app::update::on_subagent_report_ready`（app→event 的 `Effect` 不能反向 push）；
   - app 层组装来源头、正文、changed paths；
   - 不写 pending report；
   - 失败/空正文 Toast。
6. `app/update/chat/detail_report.rs` 重写：
   - 无上一轮报告 → 发效应；
   - 有上一轮报告 → 打开 `ReportPanel`。
7. 测试：
   - 报告轮落库、`notify_parent=0`；
   - 不自动 `drain_reports`；
   - 主输入框芯片；
   - 失败语义。

### 阶段 F：报告二级面板与主输入框填充

1. `src/app/state/lx/report.rs`：
   - `ReportAction` / `ReportPanel` / `active_action` / `enabled`。
2. `SlashPanel::Report` 变体 + UI items/view/title/footer。
3. 详情 composer 路由：
   - `apply_slash_input` Esc/Enter/其它键；
   - `apply_enter` → `detail_report::apply_confirm`；
   - `close_all` 清理。
4. `detail_report::apply_confirm`：
   - Target 仍活跃校验；
   - `Resend` 直接填；
   - `Regenerate` 发效应；
   - `Cancel` 关闭。
5. `report_fill::fill_main_input` + `Lx::insert_report_chip`：
   - 会话匹配；
   - 同源替换；
   - 顶部插入 + `\n\n`；
   - 光标平移；
   - undo 快照 / `touch` / `scroll_cursor_into_view` 收进 `app/state/lx/folded.rs`（`snapshot` 是 `pub(super)`，update 层不能直接调）。
6. 测试：
   - 默认 Resend；
   - Resend 无模型效应；
   - Regenerate 带 note；
   - 目标切换；
   - 同源替换/不同源共存/顶部插入/光标；
   - 旧芯片兼容。

### 阶段 G：BTW `/report` 与多目标合并

1. `BtwSubmission.report_to_main_input` + `BtwStreams.report_batches`。
2. `SendBtwReport` 效应：
   - 解析当前线程目标；
   - 无目标 → 默认助手；
   - 多目标 → 全部提交；
   - 标记 report batch。
3. 结算聚合：
   - 每个 recipient 成功 → section；
   - 失败/超时 → failed 占位；
   - 全部收口 → `BtwReportReady`；
   - 全失败 → Toast，不填。
4. `detail_report` BTW 分支：
   - 无上一轮报告 → `SendBtwReport`；
   - 有 → `ReportPanel`。
5. `last_report_body` BTW：
   - 当前线程最后一条 user 消息 marker；
   - 对应 assistant 正文；
   - 多目标合并正文。
6. 测试：
   - 默认助手；
   - 单目标；
   - 多目标合并；
   - 部分失败/全失败；
   - 报告轮可 `/rewind`；
   - 备注只作用于 Regenerate。

### 阶段 H：`/rewind` 报告轮

1. 确认新报告轮 `notify_parent=0` 后 `latest_undoable_private_turn` 可返回。
2. 确认 BTW `latest_user_turn` 路径可删除报告轮。
3. 旧 `notify_parent=1` 报告轮保持不可撤销。
4. 测试：
   - Agents 报告轮 `/rewind`；
   - BTW 报告轮 `/rewind`；
   - 主输入框芯片不变；
   - 删除后再次 `/report` 不弹二级面板。

### 阶段 I：文案、渲染与回归

1. 新增 `ui/text` 文案；删除 `LX_REWIND_ARMED_HINT`。
2. Stop / Report 面板渲染测试。
3. 全量受影响测试。
4. `cargo fmt` / `cargo clippy --all-targets -- -D warnings` / `cargo test`。
5. 用户 `cargo run` 实机验收。

---

## 3. 测试清单（按文件）

### `src/app/state/lx/stop/tests.rs`（新）

- `stop_panel_defaults_to_all`
- `stop_availability_disables_missing_targets`
- `move_skips_disabled_actions`
- `cancel_is_always_enabled`

### `src/app/state/lx/report/tests.rs`（新）

- `report_panel_defaults_to_resend`
- `resend_disabled_without_last_report`
- `note_only_used_by_regenerate`

### `src/app/update/chat/tests/stop.rs`（新）

- `idle_without_background_arms_rewind_without_hint`
- `busy_first_escape_arms_stop_and_second_opens_panel`
- `subagent_only_opens_stop_panel`
- `shell_only_opens_stop_panel`
- `stop_all_emits_all_effects`
- `stop_main_only_stops_main`
- `stop_subagents_only_stops_subagents`
- `stop_shells_keeps_chat_queue`
- `stop_panel_is_scoped_to_current_session`

### `src/app/update/chat/tests/report.rs`（新/扩展）

- `agent_report_without_previous_starts_model_turn`
- `agent_report_with_previous_opens_panel`
- `report_resend_fills_without_model_call`
- `report_regenerate_uses_note`
- `report_cancel_has_no_effect`
- `btw_report_defaults_to_btw_assistant`
- `btw_report_multi_target_merges_sections`
- `btw_report_partial_failure_keeps_success`
- `btw_report_all_failed_shows_toast`

### `src/app/paste/folded/tests.rs`

- `report_token_roundtrip_with_source`
- `report_token_label_singular_and_plural`
- `report_token_escapes_close_tag`
- `legacy_paste_tokens_still_parse`
- `same_source_replaces_existing_report_token`

### `src/app/update/chat/tests/task_page.rs`

- `user_stopped_agent_can_enter_edit_and_send`
- `plain_cancelled_agent_still_blocks_input`
- `stop_selected_subagent_emits_by_user_effect`

### `src/event/chat/tests/subagent.rs`

- `user_stop_cancels_subtree_and_queue`
- `user_stop_no_report_and_no_auto_parent_turn`
- `wait_result_marks_stopped_by_user`
- `list_agents_marks_stopped_by_user`
- `send_input_wakes_user_stopped_agent`
- `resume_wakes_user_stopped_agent`
- `close_agent_and_session_delete_still_hard_cancel`

### `src/event/chat/tests/detail_rewind.rs`（扩展）

- `agent_report_turn_is_undoable`
- `btw_report_turn_is_undoable`
- `legacy_notify_parent_report_turn_is_not_undoable`

### `src/ui/main_content/lx/tests/slash_panels.rs`（扩展）

- `stop_panel_renders_five_actions_and_default`
- `report_panel_renders_three_actions_and_default`
- `report_chip_colors_are_distinct`

### `src/chat/prompt.rs` tests

- `prompt_documents_user_stopped_subagents`
- `prompt_version_is_lx_15`

---

## 4. 验证命令

```bash
cargo fmt
cargo clippy --all-targets -- -D warnings
cargo test
```

受影响范围优先：

```bash
cargo test --lib stop
cargo test --lib report
cargo test --lib folded
cargo test --lib subagent
cargo test --lib detail_rewind
cargo test --lib slash_panels
```

最终提交前跑全量 `cargo test`。

---

## 5. 实机验收（用户执行 `cargo run`）

### 5.1 停止面板

1. 主 agent 空闲、无后台任务：双击 Esc → `/rewind`；确认没有旧提示。
2. 让主 agent 跑长任务：第一次 Esc 出现 `Press Esc again to stop…`；第二次弹出停止面板；默认 `Stop all`。
3. 上下移动，确认 `Stop main response` / `Stop all subagents` / `Stop all shells` 的禁用状态符合当前后台。
4. 选 `Stop all`：主轮、subagent、`!` shell、`exec_command` 后台进程都停；BTW 不停。
5. 另一个窗格起后台任务，确认当前窗格停止不影响它。

### 5.2 用户停止

1. Agents 页签选中 Curie，`^d`；状态显示 `stopped`。
2. 主 agent 若在等待，确认不自动恢复；输入“让 Curie 继续检查 auth.rs”，确认 Curie 被唤醒并收到新话。
3. 不输入继续指令，确认主 agent 不自行恢复。
4. 在 Curie 面板输入框直接发消息，确认自动恢复。
5. 停止前给 Curie 排 2 条追问；停止后确认队列被取消；恢复后只跑新消息。

### 5.3 报告

1. Curie 空闲，`/report 总结登录 bug`；确认 Curie 生成报告，主输入框顶部出现 `[Pasted subagent report N lines]`，与草稿空一行；主 agent 不自动起轮。
2. 展开芯片，确认 `[Subagent report] Curie · ...` 与 `Changed files`。
3. 再次 `/report`，确认二级面板三项、默认 `Resend last report`。
4. `Resend` 不产生模型轮；`Regenerate` 追加新报告轮并替换同源芯片；`Cancel` 无副作用。
5. BTW 默认无目标 `/report`，确认由 BTW 默认助手生成；同时 `@Curie @Turing` 再 `/report`，确认合并成一份、按目标分节。
6. 让一个 BTW 目标失败，确认失败占位；全部失败确认不填芯片。
7. 对 Agents / BTW 报告轮执行 `/rewind`，确认面板消息删除、主输入框芯片保留。
8. 确认 subagent 报告芯片为亮品红/深紫，BTW 为亮蓝/深蓝。

---

## 6. 完成定义（DoD）

- [ ] `design.md` 的 21 项决策全部落实，无未决边界。
- [ ] 无数据库迁移，`SCHEMA_VERSION` 不变。
- [ ] 停止面板五项动作、默认 `Stop all`、禁用态、作用域正确。
- [ ] 用户主动停止：不自动恢复、等待结果标记、队列取消、面板/主 agent 带话唤醒均可用。
- [ ] `/report` 只填主输入框，不自动起父轮；二级面板三项；同源芯片替换；顶部空行规则。
- [ ] subagent / BTW 报告芯片标签、颜色、来源头、变更文件摘要正确。
- [ ] BTW 多目标合并、部分失败占位、全失败不填。
- [ ] 新报告轮可 `/rewind`，且不动主输入框芯片；旧报告轮不追溯。
- [ ] 空闲 `/rewind` 提示已移除。
- [ ] `cargo fmt` / `cargo clippy --all-targets -- -D warnings` / `cargo test` 全绿。
- [ ] 用户 `cargo run` 实机验收通过。

---

## 7. 验收修复（2026-11-16）

- **`/report` 不生成**：`Effect::SendSubagentReport` / `StopSubagentsByUser` / `StopAllSessionProcesses`
  未进入持久化泵的网络效应白名单，被 `Persistence::pump` 静默吞掉；已补入
  `src/db/service.rs::is_network_effect`，并加事件层回归测试覆盖「先 pump、再执行」顺序。
- **BTW `/report` 无历史线程**：允许新 QA 直接 `/report`，由事件层就地创建 BTW 线程；
  启动失败也计入报告批次（失败占位），不会永久等待。
- **报告轮认领竞态**：`launch_child_turn` 只在被认领输入正文是报告 marker 时才消费
  `ChatStreams.report_to_input`；普通排队项抢先时保留标记给真正的报告轮。
- **路径省略**：`read` 工具路径改为中间省略（保留开头目录与末尾文件名），
  `docs/standards/tui-design-requirements.md` 同步。
- **消息底色**：Agents 私聊（暗紫）、流中 steer（暗海蓝）、BTW 用户消息（暗琥珀）
  与主会话用户底色互不相同；私聊标记由 DB `agent_subagent_input` 判据重建。

---

## 8. 报告语义修订与输入框 ↓（2026-11-16）

### 8.1 `/report` 改为一次性总结

- [x] `agent_compact_checkpoint` 增加 `kind`（schema v30；旧行默认 `compact`）。
- [x] 新增 `REPORT_SYSTEM_PROMPT` / `report_request_from_transcript` / `validate_report`。
- [x] `CompactRequest` 增加 `kind` 与 `note`；`execute_compact` 按 kind 分支并调用 `commit_report`。
- [x] Agents `SendSubagentReport` 与 BTW `BtwEffect::Report` 统一走 `detail_report::prepare`。
- [x] 删除旧报告轮链路：`ChatStreams.report_to_input`、`SubagentHandle.report_to_input`、BTW `report_batches` / `BtwReportBatch`。
- [x] `LxCompact` 增加 `report` 检查点视图与 `running_report` 执行态；chat bar / BTW 胶囊显示 reporting 文案。
- [x] `/rewind` 识别报告检查点：`RewindTargetKind::Report` + 报告专用面板文案，执行删除检查点。
- [x] 旧 `notify_parent=1` 报告轮保持历史行为，不追溯。

### 8.2 报告时间线与二级面板（2026-11-16 验收修复）

- [x] 报告检查点与 compact 共用按 `sequence` 交错的时间线，不固定在消息列表底部。
- [x] BTW 乐观消息（新问答）使用消息/撤销提示/检查点的最大序号 +1，后续问答排在报告之后。
- [x] `/report` 只有报告仍是时间线最后一条时才打开二级面板；其后已有新消息时直接重新总结。
- [x] 回归测试：报告卡片交错、BTW 追问序号、Agents/BTW 最后一条非报告时跳过面板。

### 8.3 输入框 ↓

- [x] 主输入框：光标在正文末尾时 `↓` 打开后台任务页；末行中间先落到行尾，第二次才换页。
- [x] `apply_editor_edit` 统一处理 `↓`，BTW / Agents composer 同样支持末行中间落行尾。
- [x] `tui-design-requirements.md` 同步入口文案。

### 8.4 测试

- [x] `report_commits_a_report_checkpoint_not_context`：报告检查点不参与 `active_checkpoint`，可被 `revert_compact` 删除。
- [x] `report_generates_one_shot_checkpoint_and_fills_main_input`：真实 mock SSE 端到端，子会话不新增消息，主输入框芯片生成。
- [x] `finish_generated_fills_from_report_checkpoint`：app 层来源头 + 变更文件 + 芯片。
- [x] `detail_rewind_targets_the_latest_report_checkpoint`：`/rewind` 选中报告检查点并打开报告专用面板。
- [x] BTW 一次性总结、无线程拒绝、输入框 ↓ 的回归测试。

### 8.5 验证

- `cargo fmt --check`
- `cargo clippy --all-targets -- -D warnings`
- `cargo test`
