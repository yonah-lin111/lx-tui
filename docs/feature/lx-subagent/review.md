# lx-subagent 功能评审

状态：**评审完成，P0~P2 列出的缺陷已修复；功能已合并到 `dev`，worktree 与分支已移除**（2026-10-28）

范围：`feat/lx-subagent` 相对 `dev` 的全部改动；合并后以 `dev` 当前树为准。

方法：codegraph 结构定位 → 逐条阅读 agent loop / event / app / UI / DB 实现 → 用新增回归测试锁定失败场景。只记录能给出触发条件与根因的问题，不收录纯风格意见。

## 0. 结论

子 agent 的 spawn / wait / close / 报告投递主链路成立，Esc 只停主轮、`^d` 只停选中子 agent 的职责也已经分开。问题集中在两类：

1. **几何与命中契约**：历史快照的点击面顺序、Agents 详情的 sticky 复用、汇报展开批次的指纹。
2. **跨会话/跨角色边界**：`/resume` 后旧 agents 残留、迟到的 `StartChatStream` 跨会话启动、执行层只依赖模型不越界。

没有发现会直接崩溃或静默破坏文件数据的 P0 问题；P1 均为用户可见的错误行为。

---

## 1. 关键链路

### spawn 与子轮

- `spawn_agent` 在 DB 事务中创建子会话、子 user/assistant 消息与 `agent_subagent` 行，再创建 `SubagentRun`（UI）和 `SubagentHandle`（事件层）。
- 子轮 delta 按 `child_session_id` 分流；子工具握手用 `child_assistant_id` 自证归属，窗格关闭后仍可落库。
- 子轮终态先写 DB，再更新 UI、发送 `done` watch 通知、最后移除句柄。

### wait / close

- `wait_agent` 目标在 `all/any` 下等待 `done` watch；超时由事件层读 DB 结算。
- `all` 模式超时后自动续等，`any` 模式返回第一个完成目标。
- 报告通过 `report_delivered` 闸门领取：后台自动轮与 `wait_agent` 共用同一判据。

### 主历史与 Agents 详情

- 主历史与 Agents 详情都走 `HistorySource` → `snapshot` → `render`；行数、点击面、滚动条必须同源。
- `snapshot.cards` 的命中规则是**取第一条重叠区域**；因此注册顺序是行为契约，不是实现细节。
- 汇报块展开统计保存在 `LxChat.report_expand_steps`，每次追加固定 24 行，再次点击提示行继续追加。

---

## 2. 缺陷与修复

### P0

无。

### P1-1 汇报块追加批次没有进入列表指纹，展开后覆盖后续块

- **触发**：展开汇报块后点击 `… N more lines`；下一次绘制多画 24 行，但快照仍使用旧行数，后面的汇报块被覆盖，视觉上出现正文逐字交错。
- **根因**：`list_signature_source` 只哈希了汇报块的 `detail_is_expanded`，漏了 `report_expand_steps`。快照缓存因此命中旧几何，而 `paint` 的绘制路径读取了新的批次数。
- **修复**：把追加批次数写入列表指纹；新增 `report_more_expansion_rebuilds_snapshot_geometry`，断言总行数增加 24、后续消息起点同步下移 24。
- **关键文件**：`src/ui/main_content/lx/history/cache.rs`、`src/ui/main_content/lx/history/cache/tests.rs`。

### P1-2 agent 链接被整卡折叠面吞掉

- **触发**：子 agent 结束（report 非空）后点击 `spawn_agent` / `wait_agent` 卡片里的 `└─ explorer · title` 行；Agents 面板不打开，卡片只是折叠/展开。
- **根因**：快照先登记整卡 `CardKind::Tool` 折叠面，再登记 `CardKind::AgentLink`；`card_at_source` 用 `find` 取第一条重叠区域，整卡面覆盖了链接行。
- **修复**：在快照中先登记 agent link，再登记整卡折叠面；新增 `agent_link_wins_over_the_whole_card_fold_region` 直接走真实 `card_target_at` 命中路径。
- **关键文件**：`src/ui/main_content/lx/history/snapshot.rs`、`src/ui/main_content/lx/history/tests/cards.rs`。

### P1-3 Agents 详情预留了置顶行，却从不绘制

- **触发**：Agents 页 transcript 高于视口且顶行落在 assistant 消息上；详情区首行空白，最后一行无法滚到，滚动条 thumb 少一行。
- **根因**：Agents 详情复用了主历史的 `view_source`，sticky 逻辑把 `body` 下移一行并减高；`draw_detail` 只画 `view.body`，从不画置顶 prompt。`resolve_task_agents_visible` 又按完整矩形高度记录可见行数。
- **修复**：新增 `view_source_plain`（不计算 sticky），Agents 的绘制、点击面与滚动条统一使用它；新增 `plain_history_view_never_reserves_a_sticky_row` 锁定 `body == area` 且 `sticky == None`。
- **关键文件**：`src/ui/main_content/lx/history.rs`、`src/ui/main_content/lx/task/agents.rs`、`src/ui/main_content/lx/history_hit.rs`。

### P1-4 `/resume` 后旧会话的 agents 仍留在新会话

- **触发**：会话 A 的子 agent 仍在跑，`/resume` 打开会话 B；Agents 列表、底边框计数、`^d` 目标仍可能来自 A，`^d` 还会弹出“stopping”但没有实际停止。
- **根因**：`on_session_loaded_full` 没有 `clear_agents()`，也没有复位 `main_stopped` / `stop_armed_at`；`set_subagent_list` 的 `parent_session` 参数未使用，旧列表和迟到的 spawn 都可追加到新会话。
- **修复**：切会话时清理 agents、重置停止闸门与一次性 focus；`SubagentRun` 增加 `parent_session_id`，`on_subagent_started` / `set_subagent_list` 校验当前窗格会话，不匹配直接丢弃。
- **关键文件**：`src/app/update/chat/session.rs`、`src/app/update/chat/subagent.rs`、`src/app/state/lx/agent.rs`；回归用例在 `app/update/chat/tests/subagent.rs`。

### P1-5 迟到的 `StartChatStream` 可以跨会话启动

- **触发**：报告批次提交并压入 `StartChatStream` 后，同一轮事件循环里用户 `/resume` 或 `/new`；下一轮 `persistence.pump` 先加载新会话，`start_stream` 再用新会话消息装配旧 key。
- **根因**：`start_stream` 完全不校验 `RequestState`、当前 `session_id` 和 `assistant_id`，只检查同 key 是否已有取消句柄。
- **修复**：启动前校验窗格会话、当前 `Streaming{key, assistant_id}` 与 assistant 占位（必须仍为 streaming）；不匹配直接丢弃效应。新增 `stale_start_after_session_switch_is_dropped`。
- **关键文件**：`src/event/chat/mod.rs`、`src/event/chat/tests/workspace.rs`。

### P1-6 子 agent 的工具面只在请求里声明，执行层不校验

- **触发**：explorer 模型幻觉/被提示词诱导，发出 `write` / `edit` / `exec_command` / `update_plan`；这些工具不在它声明的工具面里，但仍会被执行器分派。
- **根因**：`ToolSurface` 只影响 transport 的 tools 字段；`run_agent_turn` 只对子 agent 的控制工具和 `ask` 做特殊处理，其余调用直接交给执行器。
- **修复**：执行每个工具前用 `context.request.tools_surface()` 校验工具名；不在当前角色工具面内按工具级失败回给模型（仍走 begin/finish 落库，不执行 IO）。新增 explorer 调用 `write` 的回归用例。
- **关键文件**：`src/chat/agent_loop/mod.rs`、`src/chat/agent_loop/tests.rs`。

### P1-7 子 agent 的命令卡片收不到后台输出与退出事件

- **触发**：子 agent 执行 `exec_command`，等待窗口到期返回 Running，进程随后继续输出并退出；主历史卡片正常收敛，Agents 详情里的卡片永久显示 Running。
- **根因**：`for_each_tool_mut`、`apply_interaction_exit`、`apply_process_snapshot` 只扫描 `pane.lx.chat.messages`，从不扫描 `pane.lx.agents.runs[*].chat.messages`；子轮结算也没有按真实运行快照校正、没有登记 `agent_process` 记录。
- **修复**：进程事件遍历所有窗格的主历史和子 agent transcript；子轮 `FinishTool` 成功后补 `apply_process_snapshot` 与 `register_process_record`。新增 `background_output_updates_subagent_command_card`。
- **关键文件**：`src/app/update/chat/process.rs`、`src/event/chat/subagent/tools.rs`、`src/event/chat/tool_calls.rs`。

### P2-1 `wait_agent any` 遗留 watcher，`all` 小超时形成忙轮询

- **触发**：`wait_for=any, timeout_ms=5000` 反复等待一个长跑目标；每次超时都留下未取消的 watcher 任务。`wait_for=all, timeout_ms=1` 时事件层按 1ms 重新查库并 spawn。
- **根因**：`Any` 分支用 `tokio::spawn` 启动 N 个 receiver 任务，超时/父轮取消后没有 abort；`All` 自动续等直接复用 `timeout_ms` 作为重检周期。
- **修复**：`Any` 改用 `JoinSet` 并在超时/父轮取消时 `abort_all`；`All` 自动续等的重检周期至少 500ms。现有 `wait_agent` 的 all/any/超时用例仍全部通过。
- **关键文件**：`src/event/chat/subagent/wait.rs`。

### P2-2 汇报块每次快照重建重复全量 markdown 排版

- **触发**：汇报较多的会话任一增量/展开都会重建快照；`rows`、`hint_row`、`has_more` 各自调用一次 `body_lines`，同一份报告被排版三次。
- **根因**：快照构建把“行数、提示行、可展开性”拆成三个查询函数，没有共享一次排版结果。
- **修复**：新增 `report_block::measure`，一次返回 `rows / hint_row / has_more`；snapshot 只调用一次。原有 `rows` / `hint_row` / `has_more` 保留给单测并标 `cfg(test)`。
- **关键文件**：`src/ui/main_content/lx/history/report.rs`、`snapshot.rs`。

---

## 3. 已核对但未列入缺陷的部分

- **Search 卡片**：摘要、命中行 `path:line: text`、拖拽/`Ctrl+C` 复制与 `results truncated` 提示均走普通历史网格；行数由 `rows`/`draw` 同一布局给出。未发现本次改动引入的几何漂移。
- **wait / spawn 卡片**：wait 头部只显示等待条件（`all · until done`、`any · 5m` 等），目标行只用静态绿/黄/红方块；spawn 派出后为完成绿，不随子 agent running 呼吸。agent 链接偏移与绘制行一致。
- **底边框状态**：模型名、上下文占用、后台任务与子 agent 计数按“指标优先、模型名降级”的顺序绘制，窄宽度不会写出边框。
- **停止语义**：主轮 Esc 不级联取消子 agent；Agents 页 `^d` 只对高亮且 running 的 run 发取消。主轮取消后的自动报告轮由 `main_stopped` 闸门挡住，等下一次用户轮再投递。

---

## 4. 验证

- `cargo fmt`：通过。
- `cargo clippy --all-targets -- -D warnings`：通过。
- `cargo test`：与 unified-text-selection 合并后 `dev` 全量 **3760 passed / 0 failed / 2 ignored**（含全部集成套件）。
- 与统一文本选择兼容：Agents 列表行支持拖动选字，未位移单击仍切换 agent；详情正文可拖动选字，
  可折叠卡片同样拖动选字，单击延迟 250ms 才切换折叠，双击/三击取消折叠并选词/选行；已有选区时点击先清选区，同一次点击仍切换；Agents 详情滚动条、页签与 `[close esc]` 保持控件优先。
- 新增/更新的回归用例：
  - `report_more_expansion_rebuilds_snapshot_geometry`
  - `agent_link_wins_over_the_whole_card_fold_region`
  - `plain_history_view_never_reserves_a_sticky_row`
  - `loading_another_session_clears_agents_and_resets_gates`
  - `stale_subagent_replies_are_ignored_by_session`
  - `stale_start_after_session_switch_is_dropped`
  - `tools_outside_the_role_surface_are_never_executed`
  - `background_output_updates_subagent_command_card`

## 5. 未纳入本次修复的已知风险

以下不是本次 P0~P2 的已修项，确认存在但需要产品/架构决策，暂不顺手改动：

- **报告批次在提交时即标记 `report_delivered`**：如果报告自动轮随后失败/被 Esc 中断，这批报告不会重放；这是为了避免重复生成报告卡片的取舍。若要求“模型一定看到报告”，需要改成成功后再标记，并设计失败去重/重试。
- **启动恢复跨实例**：`recover_running_subagents` 目前先于 session guard 扫描，另一实例正在跑的子 agent 也可能被标 cancelled；需要按 guarded session 收窄。
- **Agents 页每轮列表轮询**：面板打开时每轮发 `ListSubagents`，空闲也会维持高频轮询；可改为低频定时器或生命周期事件触发。
- **`close_agent` 对已结束目标返回 not found**：终态句柄已移除，`FINISHED_CODE` 分支实际不可达；清理语义需要 DB 兜底。
- TUI 交互与观感仍需用户 `cargo run` 手验：agent 链接、Agents 详情滚动、子 agent 命令卡片收敛、`wait_agent any/all`。
