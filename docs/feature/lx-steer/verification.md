# `lx` 流中 Steer（Mid-Turn User Input）— 验证报告 (`verification.md`)

## 1. CLI 与测试套件执行概览

| 校验维度 | 执行命令 | 结果 |
| :--- | :--- | :--- |
| 格式与静态检查 | `cargo fmt --check && cargo clippy --all-targets -- -D warnings` | PASS（0 warnings / 0 errors） |
| Steer 专项单测套件 | `cargo test --lib steer` | PASS（49 passed / 0 failed） |
| `lx-tui exec` & `session` CLI 实机校验 | `./target/debug/lx-tui exec / session show / fork / rewind` | PASS（Exit Code 0） |

---

## 2. 验证项清单（V01–V09）

### V01 — Pending 批次合并、引用偏移重算、图片去重与预算裁剪
- **输入内容**：3 条文本 `["a", "b", "c"]`；2 条带 `@a` / `@b` 引用标记（`marker_start=4..6`）的待注入项；5 张图片或超 `MAX_MESSAGE_BYTES`（2 MiB）的待注入队列；2 条含相同字节图片的待注入项。
- **验证步骤**：执行 `cargo test --lib app::state::lx::chat::steer::tests`。
- **期望结果**：多条文本按 `"\n\n"` 合并为 `"a\n\nb\n\nc"`；第 2 条附件 `marker_start` 平移为 `12`；相同图片去重为 1 份；超过 4 张图片或 2 MiB 文本时仅消费前缀可容纳条目，其余留在 `pending_steers` 等待下一边界（PASS）。

### V02 — DB `begin_steer_turn` 单事务换轮、图片归档与 Rewind Scope 切换
- **输入内容**：处于 `streaming` 的旧 assistant 消息（含最终 parts）、新 steer 文本与文件/PNG 图片附件；以及旧 assistant 已 `complete`、跨工作区不匹配、重复 steer 或非法 parts 场景。
- **验证步骤**：执行 `cargo test --lib db::chat::tests::steer`。
- **期望结果**：单事务内将旧 assistant 封口为 `status='complete', stop_reason='steer'`（不写假 0 token），旧 `turn_scope` 封口为 `complete`，同事务插入 `turn_index+1` 的新 user（含归档图片附件）与 `streaming` 新 assistant 并开启 `capturing` 新 scope；任何前置校验失败整事务回滚、零脏行残留（PASS）。

### V03 — 流中 `Enter` 提交门控与非法输入拦截
- **输入内容**：Native 主会话分别在 `Streaming`、`Committing`、`/compact`、本地 shell 运行中按 `Enter`；流中输入 `/compact` 斜杠草稿、超 8 条或超 2 MiB / 40 MiB 上限草稿。
- **验证步骤**：执行 `cargo test --lib app::update::chat::tests::steer`。
- **期望结果**：仅 Native 主会话 `Streaming` 时接受输入追加至 `pending_steers` 并清空输入框；`Committing` 不入队；`/` 开头草稿与超限输入弹出窗格内英文 Warning Toast 且保留原草稿（PASS）。

### V04 — 撤回交互（`×` 点击与空输入框 `Backspace`）
- **输入内容**：`pending_steers = ["first", "second"]`；分别在输入框为空（`cursor == 0`）与输入框含 `"draft"` / `"abc"` 时触发 `Backspace` 或按索引调用 `recall`。
- **验证步骤**：执行 `cargo test --lib app::update::chat::tests::steer::empty_input_backspace_recalls_last_pending`、`backspace_with_draft_still_deletes_text` 与 `recall_prepends_pending_to_existing_draft`。
- **期望结果**：空输入框按 `Backspace` 撤回最后一条 `"second"` 回填输入框；非空输入框按 `Backspace` 正常删字（`"abc" -> "ab"`）不动 pending；撤回指定项时按 `"{recalled}\n\n{draft}"` 前置合并到现有草稿前（PASS）。

### V05 — 轮次终态分流：正常完成提升、取消/失败退回与切会话丢弃
- **输入内容**：本轮结束时仍存留未生效 `pending_steers`，分别触发：1) `StopReason::Complete` 正常完成（含收尾期间用户新写草稿 `"fresh draft"`）；2) `StopReason::Cancelled` 或保存失败；3) `/new`、`/resume` 切会话或删除会话。
- **验证步骤**：执行 `cargo test --lib app::update::chat::tests::steer::complete_finish_promotes_pending_to_next_turn`、`promotion_commit_consumes_pending_without_clearing_new_draft`、`cancelled_finish_restores_pending_to_input` 与 `session_change_discards_pending`。
- **期望结果**：正常完成自动发起 `Effect::BeginChatTurn` 提升为下一轮且提交成功后不清空用户在输入框新写的 `"fresh draft"`；取消或失败将全部 pending 按顺序退回输入框且不自动发送；切换/新建/删除会话清空 pending 并弹出 `Discarded N undelivered steer(s)`（PASS）。

### V06 — Agent Loop 安全边界注入、Auto-Compact 顺序与撤回竞态
- **输入内容**：模型流式采样与工具调用序列、触发阈值的 `maybe_auto_compact`、`Final` 响应到达时的 pending 探测，以及压缩等待期间用户撤回全部 pending 的竞态。
- **验证步骤**：执行 `cargo test --lib chat::agent_loop::tests::steer`。
- **期望结果**：首次采样前不注入；工具批次结算后按 `maybe_auto_compact -> take_steer_inputs -> stream_one` 顺序执行，新 steer 不被当次压缩吞入摘要；`Final` 时仅通过 `has_pending_steers` 探测并回跳循环顶部；若等待期间 pending 被撤回（`awaited_steer && batch.is_empty()`）则直接 `Complete` 收轮不空跑模型；图片与 skill 块按序写入 `ModelContext`（PASS）。

### V07 — Event `deliver` 换轮交付、共享 `mutation_users` 更新与 `/rewind` 隔离
- **输入内容**：流中第 1 段工具修改 `before.txt` → 触发 `event::chat::steer::deliver` 注入 steer → 第 2 段工具通过同一 `HostRecorderFactory` 修改 `after.txt` → 对 steer 用户消息生成 `restore_plan`；以及模型不支持图片、DB 失败异常分支。
- **验证步骤**：执行 `cargo test --lib event::chat::tests::steer`。
- **期望结果**：`deliver` 成功后旧 assistant 标为 `complete`、`RequestState::Streaming` 切换至新 `assistant_id`、`HostRecorderFactory` 共享 `mutation_users` 原地更新为新 `user_message_id`；针对 steer 轮次的 `restore_plan` 仅回滚 `after.txt` 而不触碰 `before.txt`；图片不支持或 DB 失败时返回 `HostError::Failed` 且不消费 `pending_steers`（PASS）。

### V08 — TUI `Steering` 提示区渲染、宽字符裁剪与鼠标 `×` 命中
- **输入内容**：0/1/3/5 条 `pending_steers`（含换行/控制字符、中文宽字符 `"中文..."`、极窄 13 列宽度、多附件）、打开 `task`/`ask` 面板，以及鼠标点击最右列 `×` 与正文区域。
- **验证步骤**：执行 `cargo test --lib ui::main_content::lx::steer_strip::tests`。
- **期望结果**：高度封顶 3 行，超过 3 条末行渲染 `+N more pending`（不可点击）；单行渲染 `Steering  <折叠截断正文>  · N attachments  ×`，极窄宽度保留 `×` 且宽字符绝不半写；`task`/`ask` 打开时隐藏；历史区底边精确扣减 pending 高度；`recall_hit` 仅在 `Streaming` 且点击最右列 `×` 时命中对应索引（PASS）。

### V09 — `lx-tui` CLI 实机验证（`exec` / `subagent` / `session show` / `fork` / `rewind`）
- **输入内容**：
  1. 主会话与子 Agent 实机执行：`./target/debug/lx-tui exec --ephemeral --json "Call spawn_agent with role='explorer', title='subagent steer check', task='Reply with SUBAGENT_STEER_OK and nothing else.' Then call wait_agent and reply with its output."`
  2. 含 Steer 换轮记录的会话执行：`session show --last --json`、`session fork --last --before <STEER_USER_ID> --json`、`session rewind <SESSION_ID> --dry-run --json`、`session rewind <SESSION_ID> --mode delete-only --json`。
- **验证步骤**：在当前工作区依次通过 `./target/debug/lx-tui` 执行上述 CLI 命令并校验 JSON 输出与退出码，验证后清理测试会话。
- **期望结果**：`exec` 主轮与子 Agent 轮次正常响应 `SteerInputs`/`HasPendingSteers` 默认分支并输出 `SUBAGENT_STEER_OK`；`session show` 完整展示 4 条消息（两段 user/assistant）；`session fork --before` 在 steer 消息边界精确分叉并返回 `restore_text`；`session rewind` 精确瞄准 `turn_index = 1` 的 steer 轮次回滚且保留首轮对话，全部退出码为 `0`（PASS）。
