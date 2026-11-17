# Review：停止面板、用户停止语义与 subagent/BTW 报告填充

> 状态：**P0–P3 已修复并通过自动化验证，已合并到 `dev`（merge commit `4580ba3a`），待用户实机验收**。

## 0. 范围与方法

- 分支：`feature/lx-stop-panel-and-report-fill`（含 review 修复；分支与 worktree 已移除，内容在 `dev`）。
- 范围：双击 Esc 停止面板、用户主动停止 subagent、`wait_agent` 等待语义、Agents/BTW `/report` 一次性总结、报告检查点/芯片/`/rewind`、BTW 多目标执行与启动恢复。
- 方法：按「停止与等待」「报告管线」「BTW 与恢复」三条链路并行静态审查，再逐项在 worktree 内复核代码、补回归测试并修复。
- 基线：`cargo fmt --check`、`cargo clippy --all-targets -- -D warnings`、`cargo test`（5725 passed / 6 ignored）。

## 1. 结论

| 级别 | 数量 | 已修复 | 保留 |
|---|---|---|---|
| P0 | 1 | 1 | 0 |
| P1 | 6 | 6 | 0 |
| P2 | 13 | 12 | 1（跨实例排队恢复，见 P2-13） |
| P3 | 13 | 12 | 1（既有 flaky 测试，见 P3-12） |

核心问题集中在三类：**多写路径绕过统一结算/提交约束**（报告提交、BTW 批次、死句柄）、**程序化写输入框绕过编辑入口的保护**（Shell 模式、浮层偏移、会话归属）、**停止/唤醒状态只更新一半**（内存标记与 UI 投影、失败回滚）。

---

## 2. P0

### P0-1 报告芯片在 Shell 模式下会被当作命令执行

- **关键点**：`insert_report_chip` 是程序化路径，不检查输入框模式。用户在 `!` Shell 模式下时，报告完成会把 `<pasted-subagent-report …>` 插到 shell 缓冲区顶部；按 Enter 后 `expanded_input()` 把整份报告正文交给 shell。
- **错误原因**：`begin_text_paste` 明确拒绝 `Shell` 模式，但自动报告写芯片绕过了同一个保护；报告正文常含命令/代码块。
- **修复**：`Lx::insert_report_chip` 在 `plain || shell_mode` 时直接返回 `false`，保留检查点不写芯片；不自动切换模式。
- **验证**：`report_chip_is_rejected_in_shell_mode`。

## 3. P1

### P1-1 报告正文只持久化模型原文，卡片/Resend 丢失来源头与变更文件

- **关键点**：提交时 `summary_text` 只有模型正文；来源头和 `Changed files` 只在回执时组装进瞬时芯片。展开报告卡片看不到来源头，`Resend last report` 用 `format_report_body(..., &[])` 重建，丢掉变更文件；重启后同样不完整。
- **错误原因**：报告正文在 commit 之后才组装，组装结果没有回写检查点；卡片、芯片、Resend 读取三份不同数据。
- **修复**：`CompactRequest` 冻结 `report_header` / `report_changed_paths` / `report_changed_total`；`run_compact` 在写库前拼成最终正文（含 `Changed files`），再做控制字符净化与 8 KiB 截断。卡片、芯片、Resend 统一读 `summary_text`。
- **验证**：`report_generates_one_shot_checkpoint_and_fills_main_input` 断言检查点正文含来源头且与芯片正文完全一致；`detail_commands`/`report_fill` 相关测试。

### P1-2 总结期间的新问答会让旧报告排到时间线末尾

- **关键点**：BTW 写入（`accept_btw_batch` / `start_btw_recipient`）只更新 `updated_at_ms`，不推进 `agent_session.revision`；`commit_checkpoint` 只校验 revision。报告发起后用户继续追问，旧 transcript 的报告仍会提交成功并排在更新问答之后，`Resend` 还会把它当“最新报告”。
- **错误原因**：报告提交没有校验 `end_sequence` 之后是否出现新消息；BTW 路径绕过了统一 revision 防护。
- **修复**：`commit_checkpoint` 对 `kind = report` 在事务内增加 `EXISTS(agent_message.sequence > end_sequence)` 校验，命中返回 `RewindTargetChanged`；`accept_btw_batch` 同时推进子会话 revision 作为第二道防护。
- **验证**：`report_commit_rejects_messages_after_the_summarized_range`。

### P1-3 ask 面板期间双击 Esc 打开不可见停止面板，之后可能误执行 `Stop all`

- **关键点**：ask 面板整块接管输入且不渲染 slash 浮层。第二次 Esc 仍会把 `SlashPanel::Stop` 写进 `lx.slash`；用户作答后面板突然出现，下一次 Enter 默认执行 `Stop all`。
- **错误原因**：停止面板复用 `SlashPanel`，但 ask 的输入/渲染优先级更高，且 ask 清理路径不清 `lx.slash`。
- **修复**：`escape_stop` 在 `lx.ask.is_some()` 时保持 ask 原有语义——第二次 Esc 直接取消整轮，不打开面板；ask 建立时同时关闭遗留 `Stop` 面板。
- **验证**：`double_escape_with_pending_ask_stops_the_turn_without_a_hidden_panel`、`escape_arms_then_cancels_on_second_press`。

### P1-4 BTW 失败路径不清理 `active_batches`，线程队列永久卡死

- **关键点**：批次终态只在 `finish_run` 清 `active_batches`。守卫失败、认领失败、模型/上下文装配失败等路径直接 `fail_btw_recipient`/`finish_btw_recipient` 后返回，占位仍在；之后该线程所有提交都被 `pump` 跳过，问题永远 `queued`。
- **错误原因**：批次终态记账只写在一处，其它终态转换绕过它。
- **修复**：`pump` 开泵前 `release_finished_batches` 按 `(thread, batch)` 回收“无在途 run 且无待启动项”的占位；`finish_run` 结算失败时也主动清占位并刷新 QA 列表。
- **验证**：`event::chat::tests::btw` 多目标回归 + `db::chat::btw` 批次测试。

### P1-5 用户停止 BTW 回答后批次显示 `failed`（红叉）

- **关键点**：`StopReason::Cancelled → TurnOutcome::Interrupted`，但 `aggregate_batch_status` 把 `interrupted` 计入 `failed`；`^d` 停止后 QA 行立即刷新成红色失败。
- **错误原因**：聚合函数没有区分“用户停止/崩溃截断”和“执行失败”。
- **修复**：聚合新增 `interrupted` 分类：完成 + 任意非完成 = `partial`；全失败 = `failed`；全中断 = `interrupted`；全取消 = `cancelled`。
- **验证**：`interrupted_recipient_marks_batch_interrupted_not_failed`。

### P1-6 停止时直接删除“死句柄”，迟到结算找不到句柄，子轮 DB 永久 `streaming`

- **关键点**：`cancel.send(true)` 失败说明子 loop 已退出但 `Finished` 可能仍在队列。旧实现删除句柄；迟到 `Finished` 不再走 subagent 路由，`finish_subagent_with_billing` 因句柄缺失直接返回，assistant 消息与 `agent_subagent` 行永久 `streaming/running`。
- **错误原因**：把“取消信号发送失败”当成“句柄可丢弃”，但结算路径把“句柄缺失”当成“无需结算”。
- **修复**：发送失败时不删句柄，改为直接调用 `finish_subagent_with_billing(StopReason::Cancelled)` 结算（已标记 `stopped_by_user`，不产报告），同时唤醒等待者。
- **验证**：`stop_settles_a_handle_whose_cancel_receiver_is_already_gone`。

## 4. P2

### P2-1 报告芯片可被带进另一个会话

- **关键点**：报告回执用“当前会话”自校验；`/resume` 面板打开或 `loading_session` 期间仍会写入旧会话草稿，随后切会话把芯片带过去。
- **错误原因**：报告请求/回执不冻结原始父会话，输入草稿又是 pane 全局。
- **修复**：`fill_main_input` 在 `loading_session` 或 `/resume` 历史面板打开时拒绝写入；会话切换/`/new` 时 `strip_report_chips` 剥除旧会话报告芯片并清空撤销栈。
- **验证**：`strip_report_chips_removes_all_report_tokens_and_keeps_draft`。

### P2-2 报告正文未净化控制字符

- **关键点**：普通输入和自动汇报都走 `sanitize`，一次性报告把模型原文直接编码进输入框，ANSI/`\r`/NUL 会破坏布局并让行数失真。
- **修复**：提交前 `sanitize_report_text`（复用汇报的 ANSI/控制字符剥离），再按 `MAX_REPORT_BYTES` 截断。
- **验证**：报告 e2e + 既有 `truncate_report_body` 测试。

### P2-3 程序化插入报告后未失效 `@`/slash 浮层偏移

- **关键点**：输入框顶部插入改变所有字节偏移，但 `mention.trigger`/slash 候选仍指向旧区间；按 Enter 可能切坏芯片 token。
- **修复**：`insert_report_chip` 改写后清 `panel`/`slash`/`refer` 并 `mention.suppress()`，与 `begin_text_paste` 一致。
- **验证**：芯片相关既有测试 + 新增 Shell/剥离测试。

### P2-4 唤醒后 UI 仍显示 `stopped`

- **关键点**：`send_input`/`resume` 清 `streams.user_stopped`，但 `SubagentRun.stopped_by_user` 只在列表刷新时回写；spawn/wait 卡片可能长期显示 `stopped`。
- **修复**：唤醒成功路径统一 `clear_user_stopped_ui`；`launch_child_turn` 复用同一投影。
- **验证**：`send_input_wakes_user_stopped_agent` / `resume_wakes_user_stopped_agent` 增加 UI 断言。

### P2-5 唤醒失败会提前丢掉 `user_stopped`，失败后无法再从面板输入

- **关键点**：旧实现先移除标记再做可失败的 DB 恢复/图片归档/并发校验；失败后 agent 回落到 DB `cancelled`，面板输入被误判“已关闭”。
- **修复**：标记只在入队/调度成功后解除；`send_input` 调度失败回滚，`resume` 各失败分支保留标记并撤回 `resumed`。
- **验证**：`send_input`/`resume` 唤醒测试；失败路径由代码审查覆盖。

### P2-6 `Stop all` 在无 subagent 时仍入队停止，成功结果被 `Nothing to stop` 覆盖

- **关键点**：只停主轮/shell 时，`Stop all` 无条件追加 `StopSubagentsByUser`，事件层空集合回 Warning。
- **修复**：仅当 `availability.subagents > 0` 时入队；`StopSubagents` 动作保持原判据。
- **验证**：`stop_all_without_subagents_skips_the_subagent_effect`。

### P2-7 仅 subagent/后台任务运行时，停止待命提示永不过期

- **关键点**：`stop_armed_at` 的清理嵌在 `if streaming` 分支；只有子 agent 在跑时提示留在输入框上方，第二次 Esc 语义也错乱。
- **修复**：`clear_expired_arms` 每个 tick 无条件清理过期 `stop_armed_at`；顺带关闭“已无可停止目标”的停止面板。
- **验证**：停止面板既有测试 + 代码路径复核。

### P2-8 `/resume` 后运行中的 subagent 对停止面板不可见

- **关键点**：切会话清空 `lx.agents.runs`，但事件层句柄仍在跑；未重新请求列表时 `StopAvailability` 为 0，双击 Esc 退化到 `/rewind`。
- **修复**：`on_session_loaded` 在会话变化后入队 `ListSubagents`，列表回投同时回写 `stopped_by_user`。
- **验证**：`on_session_loaded` 相关测试全量通过。

### P2-9 嵌套 agent 的 `wait_for=all` 卡片列出整棵树

- **关键点**：`view.all` 分支不按 `caller_path` 过滤，子 Executor 的等待卡会显示自己、兄弟与叔伯。
- **修复**：全量分支只保留 `is_strict_descendant_path(caller_path, run.agent_path)` 的 run，已有条目按 id 保鲜。
- **验证**：`nested_executor_wait_all_only_waits_for_descendants_not_itself` + 卡片测试。

### P2-10 `Finishing` 被当作可停止主轮

- **关键点**：`StopAvailability.main` 用 `is_busy()`，包含只等 DB ack 的 `Finishing`；面板显示可停，但 `stop_stream` 对 `Finishing` 是 no-op。
- **修复**：`main` 只认 `Streaming | Committing`，与执行动作一致。
- **验证**：停止可用性测试。

### P2-11 `list_agents` 模型文本缺少 `stopped_by_user`

- **关键点**：JSON 带该字段，但 `format_for_model` 只打印 `status=stopped`；系统提示承诺 `list_agents` 标记，模型按文档过滤会看不到。
- **修复**：列表行在 `true` 时追加 `stopped_by_user=true`。
- **验证**：`subagent_list_marks_user_stopped_targets`。

### P2-12 BTW 单目标启动失败复用“提交失败/草稿保留”文案并清空批次等待

- **关键点**：多目标批次已 accepted、草稿已清；某个回答者启动失败却发 `BtwEvent::Failed`，提示“草稿保留”并把 `pending_recipients` 归零。
- **修复**：新增 `BtwEvent::RecipientFailed`，应用层只递减该回答者的等待计数并显示 `BTW target failed to start`，不清提交态；`Failed` 只留给真正提交失败。
- **验证**：`event::chat::tests::btw` 多目标测试。

### P2-13 跨实例启动恢复仍会取消另一个实例的 `queued` BTW 回答者（保留）

- **关键点**：跨进程 guard 只在回答者真正启动时获取；全 `queued` 的批次不持锁，第二个 lx 实例启动恢复可把它们的 `queued` 行改写为 `cancelled`。
- **错误原因**：批次所有权只覆盖 running 阶段，queue 阶段是空窗；恢复扫描与加锁之间还有 TOCTOU。
- **状态**：**未修复（已知限制）**。彻底修复需要在受理批次时获取并持有 child-session guard 到整批结束，涉及 `BtwPending` 生命周期、提交失败语义与跨进程锁持有时间，风险高于本轮的展示/恢复修复。当前恢复已区分 `queued → cancelled`、`running → interrupted`，不会把另一个实例正在流式输出的回答误标为中断。
- **后续建议**：在 `BtwPending` 上持有 `Arc<FileGuard>`（同批次共享），`submit` 受理前获取，启动时移交 `BtwRun`；获取失败在 `accept_btw_batch` 之前返回提交失败（草稿仍保留）。

## 5. P3

### P3-1 停止目标仍可能领到旧报告

- **关键点**：`settle_wait` 只按 `!is_running` 领取未投递报告；被停止目标若有更早轮次的 pending report，会返回 `status=stopped` + 旧 `report`，与工具文案“无报告”矛盾。
- **修复**：`streams.user_stopped.contains(target)` 时不领取，留给以后非停止的等待。
- **验证**：停止结算测试。

### P3-2 `user_stopped` 只在唤醒时清理

- **关键点**：关闭/删除会话后标记仍留在内存，随会话数增长；旧会话重新加载前可能残留 `stopped` 投影。
- **修复**：`cancel_for_session` 清理被关闭子树与显式目标的标记。

### P3-3 Agents 页与工具卡片的 `stopped` 步骤标记不一致

- **关键点**：Agents 页把 `stopped` 归入黄色 `Interrupted`，卡片归入 `Cancelled`。
- **修复**：Agents 页 `cancelled | stopped` 统一 `StepMark::Cancelled`。

### P3-4 批次收敛会重写已完成批次的 `completed_at_ms`，且不刷新线程排序时间

- **关键点**：`sync_btw_batch_status` 无条件写 `completed_at_ms`；重复 fail/cancel 会把完成时间改晚，`agent_btw_thread.updated_at_ms` 也不更新。
- **修复**：收敛放进事务，`UPDATE ... WHERE status IN ('queued','running')`，只在批次真正进入终态时写完成时间并刷新线程排序时间。
- **验证**：BTW 批次/恢复测试全量通过。

### P3-5 `(+N more)` 用被 `LIMIT 32` 截断后的长度计算

- **关键点**：`list_all_changed_paths` 返回 `(total, 截断列表)`，调用方丢掉 total；100 个文件时显示 `(+24 more)`。
- **修复**：`report_changed_total` 随请求冻结，`format_report_body` 用 `total - 8` 计算。
- **验证**：报告 e2e + `format_report_body` 相关断言。

### P3-6 BTW 报告失败仍显示 `Report unavailable`

- **关键点**：`LX_BTW_REPORT_UNAVAILABLE` 已定义但无人使用；设计/验收要求 BTW 专用文案。
- **修复**：回执按 `thread_by_child_session` 判定 BTW，Empty/Failed 使用 `BTW report unavailable`。
- **验证**：`btw_report_starts_without_creating_messages`。

### P3-7 一次性报告未执行 8 KiB 上限

- **关键点**：`validate_report` 只限制 24k 字符，超过自动汇报 `MAX_REPORT_BYTES` 3 倍以上。
- **修复**：提交前 `truncate_report_body`。

### P3-8 `/report` 备注长度校验顺序不一致

- **关键点**：Agents 分支先看上一份报告再校验备注，超长备注会打开二级面板；BTW 分支顺序相反。
- **修复**：备注校验提前到 `last_report` 之前，`Regenerate` 再复核一次。

### P3-9 提示词把 `wait_for=all` 的停止说成“立即返回”

- **关键点**：`all` 模式还有剩余目标时会继续等待，提示词“returns immediately”与工具语义冲突。
- **修复**：改为“被停止目标按 `status=stopped` 结算；`wait_for=all` 仍等其余目标”，长度保持在 12 KiB 预算内。
- **验证**：`prompt_stays_small`、提示词契约测试。

### P3-10 停止面板在目标自然结束后仍停留

- **关键点**：面板打开后工作自然结束，面板仍吞按键，Enter 只提示 `Nothing to stop`。
- **修复**：tick 中 `StopAvailability::any()` 为 false 时自动关闭停止面板。

### P3-11 旧版 `/report` 轮仍进入新报告 transcript

- **关键点**：带 `[Report for main agent]` / `[Report to parent agent]` 的 user 消息及回答仍是“完整轮次”，新报告会总结上一份报告。
- **修复**：`playable_messages` 识别 `is_report_request` 并按 `turn_index` 整轮排除（回答一并排除）。
- **说明**：旧面板报告轮仍按既有测试作为可撤销私有轮次保留（`notify_parent=0` 语义），本次未改变 `/rewind` 行为。
- **验证**：`playable_messages_skip_report_request_turns`。

### P3-12 全量并行下 `user_shell::short_script_runs_persists_output_without_model` 偶发失败（既有）

- **关键点**：全量测试并行时偶发断言 `wc -l` 输出为空；单独运行稳定通过。
- **错误原因**：`ProcessManager` 的 pipe reader 与 `child.wait()` 无同步，终态快照可能先于输出 drain 发出；测试在收到终态后立即断言，晚到的 Output 事件未处理。该竞态在 `dev` 已存在，非本分支引入。
- **状态**：**未修复（既有问题）**。本轮 3 次全量运行均通过；建议后续在 supervisor 中等待 stdout/stderr drain（带短超时）或让测试在终态后继续排空输出事件。

### P3-13 死代码与注释漂移

- **关键点**：`BtwStreams.last_status` 只写不读；`BtwSubmission.thread_id` 从不读取；`finish_run` 重复 `runs.remove`；`Some(sender.clone())` 不可失败绑定；`stop_thread` 的 `state` 参数未使用；`SendSubagentReport` / `SubagentRun.compact` / `task.md` 仍写旧语义。
- **修复**：删除死状态与冗余代码，统一 `subagent_report_header` / `btw_report_header`，同步 `task.md`（v30 迁移、`lx-16`）。
- **说明**：`report_request_message` / `PANEL_REPORT_MARKER` 保留用于识别历史报告轮，不再产生新报告消息。

---

## 6. 修复后验证

```bash
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
```

结果：`cargo test` 5725 passed / 6 ignored；`clippy -D warnings` 通过；`fmt --check` 通过。

新增/更新的关键回归测试：

- 停止语义：`user_stop_settles_an_in_flight_any_wait`、`stop_settles_a_handle_whose_cancel_receiver_is_already_gone`、`double_escape_with_pending_ask_stops_the_turn_without_a_hidden_panel`。
- BTW：`interrupted_recipient_marks_batch_interrupted_not_failed`、`startup_recovery_keeps_mixed_completed_and_interrupted_batch_partial`。
- 报告：`report_commit_rejects_messages_after_the_summarized_range`、`report_generates_one_shot_checkpoint_and_fills_main_input`（卡片/芯片同源断言）。
- 输入框：`report_chip_is_rejected_in_shell_mode`、`strip_report_chips_removes_all_report_tokens_and_keeps_draft`。
- 模型文本：`subagent_list_marks_user_stopped_targets`、`subagent_wait_marks_user_stopped_targets_and_skips_retry`。

## 7. 保留风险

1. **P2-13 跨实例 queued 批次**：见上，需在受理阶段持有 child-session guard。
2. **P3-12 既有 flaky 测试**：进程输出 drain 竞态，与本功能无关但会影响全量验证的稳定性。
3. **停止不产生半程报告**：用户停止后主 agent 只能看到 `status=stopped`，拿不到停止前的部分输出；若需要，需另开“停止时生成半程报告”的语义。
4. **恢复 subagent 后不会自动续上已结算的等待**：用户恢复后需要重新 `wait_agent`；当前未实现等待自动重连。

---

## 8. 合并与并行迁移提醒

- 已合并到 `dev`（merge commit `4580ba3a`）；合并后全量验证 5771 passed / 6 ignored。
- 合并时补齐 `dev` 新增测试的 `CompactNoticeView.report` 字段（语义冲突，无文本冲突）。
- 并行分支 `feature/sandbox-permissions` 同样声明 `SCHEMA_VERSION = 30`；该分支合并前必须先顺延为 31，否则先启动的二进制会把库标成 30，sandbox 迁移会被静默跳过。
