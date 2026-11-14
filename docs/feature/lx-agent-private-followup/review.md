# 代码审查报告：BTW / Agents 自然对话与 Agents 私聊 / 汇报 / steer

> 审查范围：`1a5f50c7..HEAD`（`feat/btw-agent-natural-dialogue`）。
> 审查方式：逐文件 diff 核对 + 事件/DB 链路推演 + 三路并行只读审查 + 全量测试。
> 审查日期：2026-11-14。
> 结论：**P0/P1 全部修复；P2 已修复可复现项；P3 修复低成本项，剩余为存量文件拆分与边界说明。**

## P0（数据损坏 / 崩溃）

### P0-1 私聊转录按字节截断会 panic

- **位置**：`src/db/chat/subagent.rs` `private_turn_tail`。
- **现象**：中文/emoji 私聊超过 4 KiB 时，`String::truncate(4096)` 可能切在 UTF-8 字符中间，触发 `is_char_boundary` 断言，主事件循环或 exec 轮次直接崩溃。
- **错误原因**：用固定字节下标截断 `String`，没有回退到字符边界；原测试只覆盖 ASCII。
- **修复**：改为按 `agent_message.sequence` 取轮次边界，保留最新内容；截断时从尾部回退到 UTF-8 边界，并把 `… [truncated]` 计入 4 KiB 预算。
- **验证**：新增 `private_turn_tail_truncates_multibyte_text_at_a_char_boundary`、`private_turn_tail_uses_sequence_boundaries_and_survives_multibyte_truncation`。

## P1（主路径功能错误 / 隐私边界）

### P1-1 私聊文本经 `run.task` 泄漏进主 agent 的 `@` 引用块

- **位置**：`src/event/chat/subagent/launch.rs`、`src/event/chat/subagent/snapshot.rs`。
- **现象**：每次追问都无条件把本轮输入写入 `run.task` / `handle.task`；`instance_reference_block` 会输出 `task:`，`<subagents>` 快照在标题为空时也回退到 `task`。用户在主对话 `@Turing` 时，主模型会收到之前的私聊原文。
- **错误原因**：`task` 同时承担“最新任务展示”和“父可见引用字段”两种职责，私聊轮没有做策略隔离。
- **修复**：新增 `visible_task`：只有 `notify_parent=true` 才用最新输入覆盖，私聊轮保留原任务或非私聊标题；`run.task` 与 `handle.task` 同源。
- **验证**：`private_follow_up_keeps_the_parent_visible_task`；`private_panel_follow_up_stays_out...` 仍通过。

### P1-2 report 模式 steer 后报告只覆盖最后一段

- **位置**：`src/event/chat/subagent/settle.rs`、`src/event/chat/steer.rs`、`src/db/chat/subagent.rs`。
- **现象**：steer 会把 `handle.child_user_message_id` 切到新 user 消息；结算时文件统计、报告正文和工具数都只取该片段，steer 之前的工作从报告中消失。
- **错误原因**：`child_user_message_id` 同时承担“mutation 归属锚点”和“整轮报告基准”；steer 为了切换 mutation 归属把它改掉后，报告基准被切碎。
- **修复**：`SubagentHandle` 增加 `turn_user_message_ids`（claim 的 user + 每次 steer 的 user）；新增 `Database::list_changed_paths_for_turn` 取并集；报告正文拼接本轮全部 assistant 片段，工具数跨片段统计；报告行的 `child_user_message_id` 锚回本轮输入。
- **验证**：`report_mode_steer_keeps_both_segments_in_the_report`、`list_changed_paths_for_turn_unions_steer_segments`。

### P1-3 迟到 `@` 扫描被丢弃后 `pending` 未清，Files Tab 永久 Loading

- **位置**：`src/app/update/pane/popups.rs`、`src/app/state/lx/mention.rs`。
- **现象**：扫描中切换 agent / QA 或退出编辑，结果回投时归属校验失败直接 `return`，原 composer 的 `pending` 保持 `true`；再次进入并输入 `@` 永远不会发起新扫描，面板一直显示 Loading。
- **错误原因**：`pending` 只在“结果被接受”的 `apply_entries` 里清零，丢弃路径没有对应清理。
- **修复**：新增 `LxMention::abandon_scan`（generation/root 匹配才清在途标记）与 `Lx::abandon_mention_scan`；Agent 与 BTW 的丢弃路径都清原 owner 的 `pending`。
- **验证**：`late_mention_result_is_dropped_after_switching_agent` 追加“丢弃后可重新发起扫描”断言。

## P2（常见边界 / 可维护性）

### P2-1 Shift+Enter 在 `@` 面板无目录可进时被吞掉

- **位置**：`src/app/update/pane/task_page.rs`、`src/app/update/chat/btw/input.rs`。
- **现象**：面板可见但高亮普通文件、非 Files Tab 或 Loading/Empty 时，`mention_enter_folder()` 返回 false，外层仍返回已消费；用户无法用 Shift+Enter 换行。
- **错误原因**：把“面板可见”误当成“进入目录一定成功”，忽略返回值。
- **修复**：返回值 false 时回落 `apply_editor_edit(..., EnterFolder)`（即换行），Agent 与 BTW 同步。
- **验证**：`shift_enter_falls_back_to_newline_when_mention_target_is_not_a_directory`。

### P2-2 steer 草稿在容量/失败/超长场景先被清空

- **位置**：`src/app/update/pane/task_page.rs`、`src/event/chat/subagent/panel.rs`。
- **现象**：`submit_agent_edit` 先入队 effect 再无条件清空 composer；事件层满 8 条、模型解析失败或文本超限时只 toast/静默返回，文本无法恢复。超长 steer 还会让整轮失败。
- **错误原因**：容量/长度校验在事件层，草稿清空在提交侧，两者没有同步；事件层为填 `PendingSteer.model` 还依赖当前 provider 可解析。
- **修复**：提交侧先校验 `MAX_MESSAGE_BYTES` 与 `MAX_PENDING_STEERS`，失败保留草稿并 toast；事件层去掉不必要的 `FrozenModel::resolve`；无 run/无模型/非 live 时退化为排队追问并给可见反馈。
- **验证**：`full_steer_queue_keeps_the_agent_draft` 覆盖满队列保留草稿；超长文本与事件层回退共用同一前置/回退校验。

### P2-3 每帧复制 skills/MCP 并重算 20k 条提及缓存

- **位置**：`src/event/mod.rs`、`src/app/state/lx/panels.rs`、`src/app/state/lx/mention.rs`。
- **现象**：`sync_all_lx_mention_roots` 每帧调用 `sync_agent_composer_context`，`set_skills` / `set_mcp_servers` 无条件 `refresh`；面板打开时会对最多 20,000 条路径重复过滤排序。
- **错误原因**：setter 没有相等性短路，上下文同步挂在了每帧路径上。
- **修复**：`set_skills` / `set_mcp_servers` / `set_disabled_mcp_servers` 内容相同直接返回；`McpMentionEntry` 补 `PartialEq`。
- **验证**：全量测试；大仓库输入延迟需 TUI 实机复核。

### P2-4 同一会话双窗格时 steer 静默丢失

- **位置**：`src/event/chat/subagent/panel.rs`、`src/event/chat/steer.rs`。
- **现象**：A 窗格持有子轮 handle，B 窗格打开同一会话时，steer 挂到 B 的 run，但 `deliver_child` 只从 `handle.parent.pane`（A）取 pending；B 的 pending 永远不消费。
- **错误原因**：pending 的写入与读取按窗格定位，而不是按子会话/handle 定位。
- **修复**：当前窗格没有该 run 时退化为排队追问（`send_from_panel`），至少不丢文本；未把 pending 重构到 handle。
- **验证**：代码路径审查；双窗格 TUI 场景需手工复核。

### P2-5 settle 兜底会在停止/中断/失败/预算后自动重启 agent

- **位置**：`src/event/chat/subagent/settle.rs`。
- **现象**：未消费的 steer 一律落成 `SendSubagentUserInput`，而 `send_from_panel` 会 `resume` 并 `trigger_turn`；用户按 Ctrl+D 停止后 agent 又跑一轮，绕过暂停语义。
- **错误原因**：兜底没有检查 `was_closed` / `was_paused_by_interrupt` / `budget_hit` / 终态原因。
- **修复**：只有正常 `Complete` 且未关闭/未暂停/未熔断才自动起新追问；其他情况把文本退回 composer 并 toast，不自动重启。
- **验证**：`abnormal_settle_restores_undelivered_steer_to_the_composer`；原 `settle_moves_undelivered_steers_into_a_new_follow_up` 保持通过。

### P2-6 私聊转录用入队时间戳定边界、全量加载会话、截头不截尾

- **位置**：`src/db/chat/subagent.rs`。
- **现象**：`created_at_ms` 不能代表执行顺序（插队/同毫秒），可能取到空 tail 或吞掉下一轮；`load_chat_session` 为 4 KiB 文本加载整棵消息树；截断保留开头，模型看不到最新的 assistant 结论。
- **错误原因**：用时间近似顺序；复用了重型会话加载；先到先得的头部截断。
- **修复**：改用 `agent_message.sequence` 求起止；只查区间内 `kind='text'` 的 part；从尾部保留最新内容并回退 UTF-8 边界。
- **验证**：`private_turn_tail_uses_sequence_boundaries_and_survives_multibyte_truncation`、`private_turn_tail_truncates_multibyte_text_at_a_char_boundary`。

### P2-7 `private_turn_tail` 查询失败被压成“没有私聊”

- **位置**：`src/event/chat/subagent/mod.rs`、`src/chat/agent/exec/subagent/mod.rs`。
- **现象**：`.ok().flatten()` 把 DB 错误、会话缺失、边界找不到都变成无 `private_tail`，模型会回答“没有私聊”，用户无法诊断。
- **错误原因**：错误与空值共用 `Option`。
- **修复**：失败时 `tracing::warn!` 记录 agent_id 与错误；模型侧仍按“无内容”降级，避免整个列表失败。
- **验证**：代码审查；日志路径。

### P2-8 关键失败模式缺少测试

- **错误原因**：原测试只覆盖 happy path，P0/P1/P2 都落在未覆盖分支。
- **修复/新增**：UTF-8 截断、序列边界、steer 片段并集、report 模式双片段、异常 settle 退回草稿、`include_private=false` 全链路隐藏、迟到扫描恢复、Shift+Enter 回落、满队列保留草稿、`/` 路由。
- **验证**：`cargo test` 全绿（见下方）。

## P3（质量 / 一致性）

### P3-1 子 agent steer 的 user 消息未标记、UI 片段未收尾

- **修复**：`apply_steer_segment` 先 `close_stream`，并 `mark_steered_user_message`；live 与 reload 的 steer 底色一致。
- **状态**：已修复。

### P3-2 Agents composer 未继承窗格禁用的 MCP 集合

- **修复**：`sync_agent_composer_context` 复制 `disabled_mcp_servers`。
- **状态**：已修复。

### P3-3 BTW `BtwTargetView` 的 `key` / `label` 已无读取方

- **修复**：删除死字段，`resolve_target_views` 不再为展示标签查 `list_subagents`。
- **状态**：已修复。

### P3-4 生产/测试文件行数红线

- **修复**：`runner.rs` 1003 → 953（`sync_agent_ui_queue` 移到 `snapshot.rs`）；`subagent_roles.rs` 1545 → 1391（私聊测试迁到 `subagent_private.rs`）。
- **遗留**：`event/mod.rs` 1002、`ui/text.rs` 1016 为分支前已超限文件，本次不扩大拆分范围；`event/chat/btw.rs` 为存量超限，design 已注明暂不拆。

### P3-5 文档与实现不一致

- **修复**：`AgentListItemView` 注释、`DetailComposer::new_agent` 注释、BTW 目标注释已更新；`verification.md` M10 改为“模型回复引用私聊内容，工具卡片只显示 agent 元数据”；工具描述统一为 “explicitly asks”。
- **状态**：已修复。

### P3-6 `/` 面板测试未走真实路由

- **修复**：新增 `agent_composer_slash_does_not_open_panels`，走 `apply_lx_input` 并断言 slash/markdown/paste/refer 全部关闭。
- **状态**：已修复。

### P3-7 exec 忽略 claim 的 `notify_parent`

- **修复**：`ChildHandle` 增加 `notify_parent`，`claim_and_launch` 透传；exec 结算沿用 claim 时冻结的策略。
- **状态**：已修复。

### P3-8 其他已核对但未改动项

- `SCHEMA_V19` 不含 `notify_parent`：fresh DB 会执行 v29 `ALTER`，`column_exists` 幂等；不是缺陷。
- 私聊轮后 `run.report` 保留上一份报告：语义上仍是“最新报告”，不构成私聊泄漏；如需清空应单独定义。
- `event/mod.rs` / `ui/text.rs` 超 1000 行：存量问题，建议单独拆分任务。

## 已验证正确的主不变量

- v29 迁移幂等，旧行默认 `notify_parent=1`，claim 返回存储值。
- 私聊轮 `Complete/Failed/Cancelled/budget` 均经过 `!was_closed && notify_parent` 主闸门，不生成报告、不触发父自动轮。
- steer 不改变本轮冻结的策略；`begin_steer_turn` 单事务关闭旧片段、写新 user/assistant、封口/开新 scope。
- pending 在 DB 成功前不消费；正常路径不会重复投递。
- `list_agents(include_private=false)` 惰性求值，不执行私聊查询；`true` 时模型可见文本包含 `private_tail`。
- `include_private` 参数为模型可控，不是硬隔离；系统提示要求仅用户明确要求时查看。
- 新增生产代码无 `unwrap()` / `expect()`，用户可见文案为英文，UI 渲染只读状态。

## 合并状态（2026-11-14）

已合并到 `dev`（merge `bd8668d0`），功能分支已删除；主工作区 `cargo fmt`、`cargo clippy --all-targets -- -D warnings`、`cargo test`（5433 passed / 0 failed / 6 ignored）通过。
