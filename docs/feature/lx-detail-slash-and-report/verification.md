# Agents / BTW 详情输入框 `/` 命令与私聊·汇报重做 — 验证报告

- **验证工作区**：`.worktrees/detail-slash-report`（分支 `detail-slash-report`，基线 `dev@909c2e48`）
- **验证结论**：自动化与 CLI 实机通过；TUI 交互与外观项由用户按 §4 实机确认。
- **数据库**：无迁移；复用 `notify_parent`、`delete_btw_thread`、`commit_rewind`、`revert_compact`。
- **隔离库**：`~/.lx-t-dev/db/worktrees/detail-slash-report-4326b0d1/lx.db`。

## 1. 自动化与 CLI 概览

| 校验 | 命令 | 结果 |
| :--- | :--- | :--- |
| 格式 / 静态检查 | `cargo fmt && cargo clippy --all-targets -- -D warnings` | PASS（0 warning） |
| 全量测试 | `cargo test` | PASS（lib **5622 passed / 6 ignored**；集成测试全通过） |
| 详情命令模型 | `cargo test --lib app::state::lx::slash::composer` | 7 passed |
| composer 与命令分派 | `cargo test --lib app::update::chat::tests::detail_commands` | 29 passed |
| BTW 更新 | `cargo test --lib app::update::chat::btw::tests` | 21 passed |
| 详情撤销事件层 | `cargo test --lib event::chat::tests::detail_rewind` | 8 passed |
| 详情压缩事件层 | `cargo test --lib event::chat::tests::detail_compact` | 6 passed |
| 私聊 / 汇报 | `cargo test --lib event::chat::tests::subagent_private` | 6 passed |
| 可撤回判据 | `cargo test --lib db::chat::subagent::tests::rewind` | 3 passed |
| 检查点撤销防护 | `cargo test --lib db::chat::compact` | 13 passed |
| 详情渲染 | `cargo test --lib ui::main_content::lx::task::tests::detail_slash` | 4 passed |
| 面板内联条目 | `cargo test --lib ui::main_content::lx::tests::slash_panels::detail_commands` | 1 passed |
| 详情 Rewind 面板 | `cargo test --lib ui::main_content::lx::tests::slash_panels::detail_rewind_panels` | 1 passed |
| 详情压缩心跳 | `cargo test --lib detail_compact_ticks` | 2 passed |
| 详情压缩聊天栏 | `cargo test --lib detail_agent_compacting` | 1 passed |
| 详情运行中检查点 | `cargo test --lib detail_running_compact_shows_breathing` | 1 passed |
| 后台滚动条几何 | `cargo test --lib scrollbar_geometry` | 4 passed |

CLI 实机会话（本工作区）：

- 主会话：`sess-18e81e01ec804af0-0`
- 子 agent：`sub-18e81e0280e3cfa0-10`（`/root/turing`，`explorer`，`completed`；child session `sess-18e81e0280dd6700-c`）
- 压缩检查点：`compact-18e81e07a4fc05d8-1`（随后 `--revert` 恢复）

---

## 2. 功能验证项

### V01 — 详情命令模型、命名与整段解析

- **输入内容**：Agents composer 输入 `/`、`/rew`、`/report note`、`/report a\nb`、`/rewind x`；BTW 输入 `/`、`/new`、`/clear`、`/reset`；主输入输入 `/undo`。
- **验证步骤**：执行 `cargo test --lib app::state::lx::slash::composer`。
- **期望结果**：Agents 仅出现 `/rewind /compact /report`，BTW 仅出现 `/rewind /compact /new`；`/undo` 是 `/rewind` 的隐藏别名（不出现在面板，但可解析执行）；`/report` 支持多行备注并显示 `[note]` 提示；`/rewind x`、`/rewinds` 不当作命令；BTW 识别 `/clear`、`/reset` 别名；主输入 `/undo` 仍解析为 `/rewind` 别名（PASS）。

### V02 — composer `/` 面板生命周期

- **输入内容**：两个 composer 输入 `/`、`/rep`；按 `↑↓ / Tab / Enter / Esc`；鼠标点选条目；粘贴 `/rep`；退出编辑、切换 tab；输入 `/nope`。
- **验证步骤**：执行 `cargo test --lib app::update::chat::tests::detail_commands` 与 `cargo test --lib ui::main_content::lx::task::tests::detail_slash::slash_panel_anchors_above_composer`。
- **期望结果**：`/` 打开面板且底边贴 composer 上沿；键盘与鼠标高亮/采纳一致；`@` 面板不与 `/` 抢键；粘贴后候选即时刷新；离开编辑或切 tab 后无残留；未知 `/nope` 被消费并提示，不发给模型；命令目标失效时命令文本也不回落为普通消息（PASS）。

### V03 — 私聊 / `/report` 语义

- **输入内容**：连续私聊文本（如 `今天天气不错`）；`/report check the log`；子 agent 运行中再次 `/report`；先私聊一轮再 `/report`。
- **验证步骤**：执行 `cargo test --lib app::update::chat::tests::detail_commands` 与 `cargo test --lib event::chat::tests::subagent_private`。
- **期望结果**：普通追问与 steer 始终 `notify_parent=false`，主 agent 无报告、无自动轮；`/report` 注入固定标记指令并 `notify_parent=true`；报告正文只含报告轮 assistant 输出，前序闲聊不进入主 agent；父可见任务不被汇报指令覆盖；运行中拒绝且不产生发送/steer 效应（PASS）。

### V04 — Agents 详情 `/rewind`（轮次）

- **输入内容**：人类面板私聊追问（可含文件修改）；spawn 初始任务轮；父 agent `send_input` 轮；`/report` 轮；含轮内 steer 的轮次；缺文件 scope 的轮次；四项动作面板的默认项、`DeleteOnly`、`RevertOnly`、`Cancel`。
- **验证步骤**：执行 `cargo test --lib db::chat::subagent::tests::rewind`、`cargo test --lib event::chat::tests::detail_rewind`、`cargo test --lib app::update::chat::tests::detail_commands`。
- **期望结果**：仅 `position>0`、无 `parent_call_id`、`notify_parent=0` 的最后一轮可撤回；面板与主 `/rewind` 相同的四项动作，默认第一项 `Restore files and undo conversation`；`DeleteOnly` 只删对话、`RevertOnly` 只恢复文件、`Cancel` 无副作用；`/undo` 别名与 `/rewind` 等价；spawn / 父 `send_input` / `/report` / steer 段提示 `Nothing to rewind`；缺 scope 时面板照常打开但含文件动作禁用，`Undo conversation only` 仍可执行；忙碌、迟到 operation id 均拒绝且不覆盖新操作（PASS）。

### V05 — BTW 详情 `/rewind`（轮次与 compact）

- **输入内容**：BTW 线程两轮问答后 `/rewind`；仅一轮问答后 `/rewind`；回答加载中 `/rewind`；线程内存在 compact 检查点。
- **验证步骤**：执行 `cargo test --lib event::chat::tests::detail_rewind`。
- **期望结果**：删除最后一轮问答且保留线程；删空时线程回到空白（`thread_id`/`child_session_id` 清空、消息清空）；检查点比最新问答更晚时恢复 compact 上下文；加载中拒绝；迟到回执忽略（PASS）。

### V06 — 详情 `/compact`

- **输入内容**：已完成子 agent 会话；已完成 BTW 线程；空白摘要；失败摘要；运行中或排队中的目标；迟到 `operation_id`；BTW 线程重新加载。
- **验证步骤**：执行 `cargo test --lib event::chat::tests::detail_compact`、`cargo test --lib app::update::chat::tests::detail_commands`、`cargo test --lib app::update::chat::btw::tests`。
- **期望结果**：摘要成功后在子会话写入一条检查点并回填对应 run/thread，不触碰主会话；空白/失败摘要零写入并提示；运行中、排队中拒绝；迟到回执不覆盖当前执行；BTW 线程加载时恢复检查点卡片；`compacting` 胶囊与耗时 tick 正常（PASS）。

### V07 — 详情 `/rewind` 撤销 compact 检查点

- **输入内容**：子会话已有一条 active 检查点（且比最新用户轮更晚）；检查点执行 `revert_compact`；检查点已被撤销或 revision 变化；自动压缩产生的非持久化检查点。
- **验证步骤**：执行 `cargo test --lib event::chat::tests::detail_rewind`（`detail_rewind_restores_the_latest_compact_checkpoint`）与 `cargo test --lib app::update::chat::tests::detail_commands`（`rewind_prepare_for_compact_shows_restore_and_cancel`）。
- **期望结果**：`/rewind` 预检冻结 `kind=Compact`、检查点 ID 与当前 revision；面板只有 `Restore the context before this checkpoint` 与 `Cancel`；执行走 `revert_compact`，删除检查点并推进 revision，不删消息、不动文件；回投后详情卡片同步移除；自动压缩（`auto=true`）不进入可选目标；revision 变化时失败且零写入（PASS）。

### V08 — BTW `/new`

- **输入内容**：已有线程时 `/new`；回答中 `/new`；无线程时 `/new`；删除失败。
- **验证步骤**：执行 `cargo test --lib app::update::chat::btw::tests`。
- **期望结果**：发出 `BtwEffect::DeleteThread`，成功后线程视图清空、QA 列表刷新（`● side thread` 消失）、lx 窗格内英文 toast；回答中拒绝；无线程提示 `No side thread to clear`；删除失败只提示并保留线程（代码路径审查，未单测）。

### V09 — 文案、胶囊与布局

- **输入内容**：90 / 60 / 34 列窗格；已完成、运行中带待注入 steer、压缩中的 Agents；压缩中的 BTW；composer 底栏；`/` 面板；详情 `/rewind` 面板。
- **验证步骤**：执行 `cargo test --lib ui::main_content::lx::task::tests::detail_slash` 与 `cargo test --lib ui::main_content::lx::tests::slash_panels::detail_commands`。
- **期望结果**：Agents 胶囊依次显示 `private` / `steer N` / `compacting`；BTW 胶囊空间不足时整段丢弃 ` · compacting`，不截半截；底栏显示 `[/ commands]`，不再显示 `Shift+Tab` 模式；`/` 面板贴 composer 上沿；详情命令以内联标签 + `Built-in` 标签渲染，命令名为 `/rewind`；全部文案英文。自动化覆盖胶囊、BTW 34 列降级、底栏与面板锚点；Agents 窄窗格与动作面板由 §4 实机确认（PASS）。

### V11 — 详情压缩心跳、呼吸与聊天栏

- **输入内容**：选中子 agent 详情后执行 `/compact`；等待摘要请求完成（数秒）；期间观察消息列表末尾与输入框上方一行。BTW 线程同样执行一次 `/compact`。
- **验证步骤**：执行 `cargo test --lib detail_compact_ticks`、`cargo test --lib detail_agent_compacting`、`cargo test --lib detail_running_compact`；实机观察 `cargo run` 中的压缩过程。
- **期望结果**：压缩开始后消息列表末尾出现 `◆ Compacting context... · Ns`，菱形随 100ms 心跳在 `◆/◇` 间呼吸，耗时逐秒推进；输入框上方聊天栏同时显示 `Compacting context... (Ns)`，列表态与编辑态都显示；压缩结束后两者消失并出现检查点卡片；BTW 线程同款。此前 `has_streaming_lx` 未包含详情压缩，界面会整段冻结、无呼吸也无耗时（PASS）。

### V12 — 后台详情滚动条几何一致性

- **输入内容**：子 agent 详情（含压缩后检查点）滚动条拖动/轨道点击；BTW 对话区、Processes 输出区同款；滚轮在消息区上滚/下滚。
- **验证步骤**：执行 `cargo test --lib scrollbar_geometry`；实机在 `cargo run` 中拖动滚动条、点击轨道与滚轮。
- **期望结果**：Agents/BTW/Processes 的滚动条按下/拖拽上限与绘制使用同一份 `ScrollbarLayout`（`total`/`visible`），即使 `run`/`thread`/`output_visible` 的缓存行数滞后一帧也不会被旧上限钳回底部；首次拖动/点击立即到位，滚轮每次上移 3 行、贴底自动恢复跟随；Agents 列表沿用自身窗口几何（PASS）。

### V13 — 主会话 / 子 agent / BTW 步骤呼吸对齐

- **输入内容**：主会话、子 agent 详情、BTW 线程各保留一个运行中的工具步骤；BTW 线程生成中不设置 `chat.status.streaming`（只靠消息 `streaming` 表达在途）。
- **验证步骤**：执行 `cargo test --lib btw_streaming_tool_step_breathes`；实机在 BTW 线程生成时观察上方消息列表的步骤标记。
- **期望结果**：三处均按 `LxChat::is_animating()` 判定动画，`loading_frame` 每 `STEP_BREATH_TICKS` 帧翻转 `■/□`（约 400ms 一相）；BTW 的方块不再恒为实心；回答结束后终态恒实心。此前 `from_chat` 只看 `status.streaming`，BTW 从不设置该字段，导致步骤标记整段不呼吸（PASS）。

### V14 — 2026-11-16 review 回归

- **输入内容**：自动压缩检查点 + 更晚的私聊轮；多行草稿末行 `/new` 且 `/` 面板打开；BTW 两个线程、确认撤销后 `Esc` 关闭任务页；压缩中再次 `/compact`；报告轮结束仍有未消费 steer；BTW 搜索粘贴把 active 过滤到另一个 QA。
- **验证步骤**：执行 `cargo test --lib auto_compact_checkpoint_is_not_a_rewind_target`、`cargo test --lib multiline_draft_does_not_execute_last_line_command`、`cargo test --lib btw_rewind_receipt_targets_the_planned_thread_only`、`cargo test --lib closing_the_page_keeps_detail_rewind_execution`、`cargo test --lib compact_detail_rejects_reentry_while_compacting`、`cargo test --lib undelivered_steer_after_report_is_resent_privately`、`cargo test --lib btw_search_paste_loads_the_switched_thread`。
- **期望结果**：自动检查点（`operation_id` 前缀 `autocompact-`）不进入 `/rewind` 目标；多行草稿只提示 `Run this command on its own line`，不执行、不清空；删空只重置 `plan.target` 对应线程并 push `LoadQaList`；关闭任务页后 `Lx.detail_rewind.executing` 保留；压缩中第二次 `/compact` 不覆盖幂等键；steer 重投 `notify_parent=false`；搜索粘贴切换 QA 会发 `LoadThread`（PASS）。

### V10 — 回归与边界

- **输入内容**：主输入 `/`、`/rewind`、`/compact`、AI Web 允许命令；旧库追问行；含 steer 的轮次。
- **验证步骤**：执行 `cargo test` 全量；检查 `src/db` 无新迁移、`src/event/mod.rs` 无新 `AppEvent` 变体。
- **期望结果**：主输入命令集与 `/undo → /rewind` 别名不变；AI Web 命令裁剪不变；无数据库迁移；旧库 `notify_parent` 默认 1 与 steer 段按安全默认拒绝撤销（PASS）。

---

## 3. CLI 实机验证（本工作区）

- **输入内容**：`exec` 提示词“spawn explorer，任务 `Reply with exactly SUBAGENT_CLI_OK...`，wait 后原样回复”；随后执行会话查询、压缩/恢复、回滚预检与清理。
- **验证步骤**：

```bash
./target/debug/lx-tui exec --json \
  "Call spawn_agent with role='explorer', title='detail slash cli check', \
   task='Reply with exactly SUBAGENT_CLI_OK and nothing else.' \
   Then call wait_agent and reply with exactly the subagent output."

./target/debug/lx-tui session list --json
./target/debug/lx-tui session show --last --json
./target/debug/lx-tui session agents --last --tree --json
./target/debug/lx-tui session compact --last --json
./target/debug/lx-tui session compact --last --revert --json
./target/debug/lx-tui session rewind --last --dry-run --json
./target/debug/lx-tui session delete sess-18e81df7b82f7078-0 --json
```

- **期望结果**：`exec` 输出 `SUBAGENT_CLI_OK`，落库会话 `sess-18e81e01ec804af0-0`，子 agent `sub-18e81e0280e3cfa0-10` 为 `completed`；`session show` 显示两条消息与两个工具调用；`session compact` 创建 `compact-18e81e07a4fc05d8-1` 且摘要覆盖整轮，`--revert` 返回 `reverted`；`session rewind --dry-run` 返回 `availability=known-empty`、`revision=3`；清理临时会话成功（PASS）。

---

## 4. TUI 实机清单（待用户执行 `cargo run`）

1. **Agents 命令面板**
   - 输入内容：打开已完成 subagent 的详情，`Shift+Enter`，输入 `/`。
   - 验证步骤：观察候选列表；输入 `/rew` 过滤；按 `↓` 后 `Enter` 采纳 `/report`。
   - 期望结果：仅 `/rewind /compact /report`（`/undo` 不显示但可手输执行）；过滤、移动、采纳正常；采纳 `/report` 后输入框回显 `/report ` 与灰色 `[note]`。
2. **私聊与汇报**
   - 输入内容：连续发送 3 条闲聊；再发送 `/report 汇总刚才的结论`。
   - 验证步骤：观察主 agent 时间线；打开子 agent 详情与报告。
   - 期望结果：3 条闲聊不产生报告/自动轮；`/report` 只把报告轮正文交给主 agent 并自动起轮，闲聊不出现在报告中。
3. **Agents 撤销与 compact 恢复**
   - 输入内容：让子 agent 私聊修改一个文件；输入 `/rewind`；分别执行四项动作；再私聊多轮后 `/compact`，随后 `/rewind`；`/undo` 也试一次。
   - 验证步骤：观察面板、文件与详情时间线；对 spawn 轮或 `/report` 轮执行 `/rewind`。
   - 期望结果：面板四项、默认 `Restore files and undo conversation`；`DeleteOnly` 保留文件、`RevertOnly` 保留对话、`Cancel` 无变化；`/compact` 后 `/rewind` 只显示恢复上下文与取消，执行后检查点卡片消失且消息保留；不可撤回轮提示 `Nothing to rewind`。
4. **详情压缩**
   - 输入内容：私聊多轮后 `/compact`；完成后再次 `/compact`。
   - 验证步骤：观察胶囊、耗时与检查点卡片。
   - 期望结果：胶囊显示 `compacting` 且耗时推进；完成出现检查点；再次执行提示 `Nothing new to compact.`。
5. **BTW 撤销、compact 恢复与清空**
   - 输入内容：BTW 线程问几轮后 `/rewind`；再 `/new`；回答中再试 `/new`；线程内先 `/compact` 再 `/rewind`。
   - 验证步骤：观察最后一轮、线程列表与 QA 列表标记。
   - 期望结果：`/rewind` 删除最后一轮或恢复 compact 上下文；`/new` 清空线程且 `● side thread` 消失；回答中 `/new` 被拒绝。
6. **子 agent 压缩反馈与滚动**
   - 输入内容：进入任一已完成的子 agent 详情，执行 `/compact`；压缩过程中与完成后各滚一次滚轮，并拖动右侧滚动条。
   - 验证步骤：观察消息列表末尾、输入框上方聊天栏、滚动条。
   - 期望结果：压缩中列表末尾出现呼吸的 `◆/◇ Compacting context... · Ns`，聊天栏显示 `Compacting context... (Ns)`；完成后两者消失、检查点卡片出现；首次滚轮/拖动即生效，无“卡一下再滚一次”的迟滞。
7. **BTW 步骤呼吸**
   - 输入内容：在 BTW 线程中提出一个需要只读工具（read/search）的问题。
   - 验证步骤：观察生成过程中 BTW 消息列表的步骤标记。
   - 期望结果：工具方块与思考圆点按与主会话、子 agent 相同的节奏闪烁，结束后恒实心，不再静止。
8. **窄窗格与文案**
   - 输入内容：将窗格缩到约 60 列，重复打开胶囊、动作面板与底栏提示。
   - 验证步骤：观察截断与换行。
   - 期望结果：胶囊、动作面板、底栏提示与 `/` 面板均不截半截；全部文案为英文。

---

## 5. 已知限制

- 含轮内 steer 的轮次不可 `/rewind`：steer 段只覆盖截断之后的消息与文件，按最新段撤销只能回退半个逻辑轮次，因此直接拒绝。
- 旧库（v29 之前）的追问行 `notify_parent` 默认 1，会被判为不可撤回；安全默认，不回填历史。
- `/report` 会额外消耗一次模型轮次（仅显式命令触发）。
- 自动压缩也写检查点表，但不能通过 `/rewind` 恢复（按 `operation_id` 前缀 `autocompact-` 跳过）；它是上下文基线，撤销更早的手动检查点也不会改变 active 指针。
- CLI 无法直接触发面板专属命令（`/rewind`、`/report`、`/compact`、`/new`）；这些路径由 §1 的定向测试覆盖，TUI 交互见 §4。
