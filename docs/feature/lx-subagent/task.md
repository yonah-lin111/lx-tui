# lx Subagent（explorer / executor）— 执行任务

> 状态：**实现完成并合并到 `dev`（2026-10-28）；评审与缺陷修复见 [review.md](review.md)**。
> 本清单保留原始执行计划与历史 checkbox，不作为当前完成度声明。
> 规则：生产代码禁止 `unwrap()/expect()`；用户可见文案集中在 `ui/text.rs`；模型可见文本一律英文；
> 生产代码单文件 ≤1000 行（测试文件不受限）；每节**先写测试再写实现**。
> 每节完成后按影响范围跑 `cargo fmt` + `cargo clippy --all-targets -- -D warnings` + `cargo test`。

## 0. 分支与基线

- [ ] `git worktree add .worktrees/lx-subagent -b feat/lx-subagent dev`（worktree 隔离库自动生效）
- [ ] 在该 worktree 内跑一次 codebase memory 索引（新路径需要独立索引）
- [ ] 记录基线：`cargo test` 全绿、`cargo clippy --all-targets -- -D warnings` 无警告
- [ ] 确认 `SCHEMA_VERSION == 15`（`src/db/mod.rs:26`）、`PROMPT_VERSION == "lx-4"`（`src/chat/prompt.rs:28`）
- [ ] 确认 `TOOL_NAMES.len() == 8`、`TOOLS.len() == 8`、`agent_subagent` 零引用（design §0）

---

## 1. 领域层：`src/chat/agent.rs`（新文件，~450 行）

纯数据，无 IO、无 `app/`/`ui/` 依赖。

- [ ] `AgentRole::{Explorer, Executor}`：`ALL` / `as_str` / `parse` / `system_prompt` / `tools`
- [ ] `ToolSurface::{Main, Explorer, Executor}`：`names()` / `from_role()`
  - `Main` = 11 项；`Explorer` = `["read","search"]`；`Executor` = `["read","search","edit","write","exec_command","write_stdin"]`
- [ ] 常量：`MAX_CONCURRENT_SUBAGENTS = 3`、`MAX_WAIT_TARGETS = 8`、`DEFAULT_WAIT_TIMEOUT_MS = 300_000`、`MAX_WAIT_TIMEOUT_MS = 3_600_000`、`MAX_REPORT_BYTES = 8 * 1024`、`MAX_TASK_CHARS = 8_000`
- [ ] 稳定错误码：`LIMIT_REACHED_CODE` / `NOT_FOUND_CODE` / `FINISHED_CODE` / `UNAVAILABLE_CODE`
- [ ] `SubagentOp` + `parse_control(name, json) -> Option<Result<SubagentOp, ToolFailure>>`
      （白名单字段；`task` 非空 ≤8000；`targets` 1..=8；`timeout_ms` 钳到 `[1, 3_600_000]`）
- [ ] `SubagentOutcome` / `AgentStatusView` + `into_result()` / `to_json()` / `from_json()`
- [ ] `format_report_envelope(&SubagentReportView) -> String`（XML 包裹 + 转义；风格对齐 `context::format_user_shell`，`chat/context/mod.rs:388`）
- [ ] 角色提示常量 `EXPLORER_PROMPT` / `EXECUTOR_PROMPT`（design §7.2 / §7.3 原文）
- [ ] `compose_subagent_prompt(role, environment)`

测试（`agent/tests.rs`）：
- [ ] `AgentRole::parse` 双向一致；未知值 `None`
- [ ] 三个 `ToolSurface` 名单**逐字**断言
- [ ] `parse_control`：合法 / 缺字段 / 多余字段 / 超长 task / 非法 role / 负 timeout / targets 超 8
- [ ] `SubagentOutcome` 序列化往返
- [ ] `format_report_envelope` 对 `&<>` 与换行做转义，正文截断带 `[truncated]`
- [ ] 角色提示不含中文（复用 `prompt.rs` 的 `has_cjk` 思路）

---

## 2. 工具 schema：`src/chat/tools/subagent.rs`（新）+ `schema.rs` + `types.rs`

- [ ] 三个 `ToolSpec` 构造函数：`spawn_agent` / `wait_agent` / `close_agent`（design §5.3）
- [ ] `schema.rs:25` `static TOOLS: [ToolSpec; 11]`（现有 8 项原样 + 3 项）
- [ ] `schema.rs:126` `available_tools()` 保留；新增 `pub fn tools_for(surface: ToolSurface) -> Vec<&'static ToolSpec>`
- [ ] `types.rs:10` `TOOL_NAMES` 保持 8 项；新增 `pub const SUBAGENT_TOOL_NAMES: [&str; 3]`

测试：
- [ ] `tools_for(Main)` 顺序 == `TOOL_NAMES ++ SUBAGENT_TOOL_NAMES`
- [ ] `tools_for(Explorer)` == `["read","search"]`（顺序）
- [ ] `tools_for(Executor)` == 6 项且不含 `ask`/`update_plan`/`spawn_agent`
- [ ] 新 schema `additionalProperties: false`、`required` 与常量同源

---

## 3. transport 按工具面裁剪

- [ ] `chat/events.rs:168` `RequestPurpose` 加 `Subagent(AgentRole)`；新增 `tools_surface()`，`includes_tools()` 改为委托
- [ ] `transport/tools/mod.rs`：四个函数改为 `(surface: ToolSurface) -> Value`
- [ ] `transport/{openai,responses,anthropic,google}/mod.rs`：`if let Some(surface) = request.tools_surface()`；`tools_requested: request.tools_surface().is_some()`
- [ ] 全部 `ChatRequest` 构造点适配（`context/mod.rs:239`、`event/chat/mod.rs:851` 附近、测试 fixture）

测试：
- [ ] 四协议 × 三工具面：名称集合与数量逐字一致
- [ ] `CompactSummary` 不带工具（既有测试保持）
- [ ] explorer 请求体里**不存在** `edit`/`write`/`exec_command` schema
- [ ] 主 agent 请求体里存在 3 个控制工具

---

## 4. 持久化 v16

- [ ] `src/db/mod.rs:26` `SCHEMA_VERSION` 15 → 16
- [ ] `src/db/migration.rs`：`migrate()` 末尾加 `if version < 16` 分支，三条 `ADD COLUMN` 各带 `column_exists` 守卫（design §4.2）
- [ ] `src/db/chat/subagent.rs`（新，~400 行）：`SubagentRecord` / `SubagentCommitted` / `ReportPayload` + 六个方法（design §4.3）
- [ ] `src/db/chat/mod.rs:55` `ChatMessage` 加 `subagent_reports: Option<String>`；`:220` SELECT 加列
- [ ] `src/db/chat/mod.rs:160` `list_chat_sessions` 加 `AND source = 'native'`

测试（`db/chat/subagent/tests.rs` + `db/tests.rs`）：
- [ ] v15 → v16 升级保留数据；重复迁移幂等；`PRAGMA integrity_check` 通过
- [ ] `begin_subagent_turn` 写全 `source='subagent'` / `agent_role` / `agent_subagent` / `spawn_message_id`
- [ ] `list_chat_sessions` 不返回子会话
- [ ] `load_chat_session` 能加载子会话并带回 `subagent_report_json`
- [ ] `begin_report_batch`：user/assistant 同 `turn_index`，`subagent_report_json` 数组，`plan_turns` 可回放；提交即标记 `report_delivered=1`
- [ ] 旧 user-only 报告行不被 `plan_turns` 回放，UI 时间线隐藏
- [ ] token 不串：子会话 `total_*` 增长时父会话不变
- [ ] `mark_report_delivered` 幂等；不改 `report_json`
- [ ] 级联删除：删父会话 → 子会话 / `agent_subagent` / 子消息全消失
- [ ] `recover_running_subagents`：running → cancelled、streaming → interrupted

---

## 5. loop 与事件层

### 5.1 loop（`src/chat/agent_loop/mod.rs`）

- [ ] `AgentHostRequest::Subagent { op, reply }` + `AgentHost::subagent()` 默认方法（照抄 `ask_user`）
- [ ] `run_agent_turn`（`:560` 的 `ask` 分支之后）加控制工具分支：`parse_control` → `host.subagent(op)` → 结果进上下文
- [ ] 子 turn 内 `parse_control` 命中控制工具 → `UNAVAILABLE_CODE` 失败（禁止递归兜底）

### 5.2 事件层（`src/event/chat/subagent.rs`，新，~600 行）

- [ ] `SubagentHandle` + `ChatStreams.{subagents, frozen}`（design §6.1）
- [ ] `handle_subagent_op(state, streams, persistence, sender, key, op, reply)`：**先落库再回执**
- [ ] `spawn`：并发检查 → `begin_subagent_turn` → 注册 handle → `tokio::spawn` 子 turn → 回执
- [ ] 子 turn 装配：`RequestPurpose::Subagent(role)` + 独立 `ReadRegistry` + 独立 `SessionRuntime` + `try_lock_session(child_session)`
- [ ] 子 delta 分流（`handle_net_event` 最开头）+ 独立 checkpoint 落库（design §5.4）
- [ ] 工具握手子分支：`begin_child_tool` / `finish_child_tool`（不查窗格，design §5.6）
- [ ] `wait`：`watch` 完成信号 + `tokio::sleep` + `select!`；超时不写 `error_text`
- [ ] `close`：发 cancel → 子会话 `cancelled` → **不生成报告**
- [ ] 报告：`finish_subagent_turn` 只写 `report_json`；父轮/命令结束且 `idle` 时 `begin_report_batch`（批量大卡 + assistant，design §5.5）
- [ ] `wait` 消费报告只进工具结果；主历史不插消息；旧 user-only 报告行 UI 隐藏
- [ ] `request_report_drain_if_idle` 在父轮 ack、shell 终态、会话加载三处触发；用户排队输入优先
- [ ] `ChatStreams::finish(&key)` 清理子 handle（`event/chat/mod.rs:230`）
- [ ] `reap_orphan_runtimes`（`:752`）加运行中子会话白名单
- [ ] `runtime_context_summary` 加在册 subagent 段（`MAX_CONCURRENT_SUBAGENTS` 条）
- [ ] `recover_startup` 调 `recover_running_subagents`

### 5.3 效应

- [ ] `Effect::{ListSubagents, LoadSubagentSession, CancelSubagents}`（`app/effects.rs`）
- [ ] `db/service.rs`：`execute` 里 warn + false，`is_network_effect` 返回 true
- [ ] 事件循环 `apply_process_effects` 同级新增 `apply_subagent_effects`

测试：
- [ ] 第 4 个 spawn → `LIMIT_REACHED_CODE`，同批其余调用照常执行
- [ ] wait 超时 → `timed_out: true`、无 `error_text`、轮次继续
- [ ] close 后无报告、无汇报块
- [ ] 恰好一次：自动投递后 wait → `report = None`
- [ ] Esc 只取消主轮与 `wait_agent` 回执；所有 running 子 handle 仍在、子 turn 未收到 cancel
- [ ] Agents 面板 `^d` 只强停当前高亮 running 子 agent；其他 running 子 agent 继续，partial 保留、不自动起报告轮
- [ ] 子 delta 不进主对话（主消息数不变）
- [ ] **窗格已关闭**时子 turn 的工具握手仍成功
- [ ] explorer 注入 `edit` → `UNAVAILABLE_CODE`、无文件 IO
- [ ] `reap_orphan_runtimes` 不回收运行中子会话；结束后回收
- [ ] 重启恢复：running → cancelled、streaming → interrupted、不重跑

---

## 6. app 状态与更新

- [ ] `src/app/state/lx/agent.rs`（新，~350 行）：`SubagentRun` / `SubagentReportView` / `AgentsState`
- [ ] `Lx` 加 `pub agents: AgentsState`（`state/lx.rs:58` + `build()`）；会话复位清空
- [ ] `ChatMessageView` 加 `pub reports: Vec<SubagentReportView>`；`update/chat/view.rs:13` 解析 `subagent_report_json`（损坏 → 空 vec）
- [ ] `ToolDetailView::Subagent(SubagentDetailView)`（`state/lx/chat/tool_view.rs:307`）+ `from_result_json` 解析
- [ ] `TaskPage` 加 `agents_window` / `selected_agent` / `agents_scroll` / `agents_visible` / `agents_follow`；`len/selected/move_active/clamp` 接受 `&AgentsState`
- [ ] `age_label(ms)` 抽成自由函数，`ProcessEntry::age_label` 转调
- [ ] `src/app/update/chat/subagent.rs`（新，~350 行）：`on_subagent_delta` / `on_subagent_tool_pending|running|settled` / `set_subagent_list` / `on_subagent_loaded` / `on_subagent_finished` / `on_report_batch_committed` / `scroll_agents_detail` / `move_selected_agent` / `stop_selected_subagent`
- [ ] `resolve_task_agents_visible`（`app/update/chat/viewport.rs`，类比 `resolve_task_output_visible`）

测试：
- [ ] 两个 run 交错 delta 不串
- [ ] `set_subagent_list` 幂等
- [ ] `TaskPage` 按 tab 取集合；切 tab 不丢进程高亮（既有测试扩一条）
- [ ] `reports` 损坏 JSON → 空 vec
- [ ] 会话复位清空 `Lx.agents`

---

## 7. `HistorySource` 抽取（**单独一次提交**）

- [ ] `ui/main_content/lx/history/source.rs`（新）：`HistorySource<'a>` + `From<&Lx>` + `from_chat`（design §8.1）
- [ ] `snapshot.rs:104` `build`、`:290` `Builder::message` 改吃 `source`
- [ ] `history.rs:283` `has_history`、`:285` `view` 改吃 `source`
- [ ] `render.rs:19` / `paint.rs:119` / `sticky.rs` / `cache.rs:140,314` 改吃 `source`
- [ ] `history_hit.rs` 4 处传 `HistorySource::from(lx)`
- [ ] `render_history_for_test` 签名**不变**（内部转换）

测试：
- [ ] **等价性**：抽取前后主窗格 `total_rows`、消息区间、块键一致（既有 40+ 渲染测试零改动通过）

---

## 8. UI

### 8.1 委派卡片（`history/tool/subagent.rs`，新 ~180 行）

- [ ] `rows()`（`history/tool.rs:107`）与 `draw_animated()`（`:182`）各加一支
- [ ] 折叠 2 行：`■ Spawn agent` / `  └─ explorer · <task 首行>`
- [ ] 展开：`task` 全文 / `agent_id` / `targets` / 每个目标 `status`
- [ ] `cache.rs` 的 `tool_signature` 加该变体字段
- [ ] 标题文案常量进 `ui/text.rs`

### 8.2 汇报块（`history/report.rs`，新 ~200 行）

- [ ] `snapshot.rs::message()`：`reports` 非空 → 跳过用户气泡、逐条画汇报块；`is_user` 同时置 false
- [ ] 头部固定 2 行：`▲ Subagent report · completed` / `  └─ explorer · <title>`；折叠正文预览 2 行
- [ ] 展开用 `paint::draw_markdown` 渲染正文
- [ ] 展开状态复用 `detail_expanded`，槽位 `format!("report:{agent_id}")`
- [ ] `cache.rs` 新增 `REPORT_SLOT_BLOCK`
- [ ] 多报告 → 多块，块间空行只经 `spacer()`
- [ ] `failed`/`cancelled` 的 `StepMark` 与颜色；字形固定 `▲`，不得回落为工具方块
- [ ] 后台报告批次卡在 assistant 前；`wait_agent` 消费的报告只在工具卡展开态显示正文
- [ ] `Wait agents` 头部显示 wait 条件与 timeout（all/any/指定目标，all 显示 until done）；目标行只保留静态绿/黄/红方块
- [ ] 统计行仅在 `subagents_running == false` 且下一条不是可见报告时显示；wait/自动轮期间 loading 持续
- [ ] 用户显式停止主轮后仅剩后台任务/子 agent 时不再显示输入框上方 loading；主轮自然结束仍保留等待 loading；底边框上下文右侧始终显示 `N process · N subagents`
- [ ] 测试：批次多报告一张消息、报告卡在回答前、旧 wait 残留隐藏、等待 loading 文案

### 8.3 `Agents` 页签（`ui/main_content/lx/task/agents.rs`，新 ~400 行）

- [ ] `task.rs:220` 的 `TaskTab::Agents` 分支改为 `agents::draw_list` / `agents::draw_detail`
- [ ] 列表：`StepMark` + 角色 + 状态 + 时长 + task 首行
- [ ] 详情：`HistorySource::from_chat` → `history::view/render/scrollbar`
- [ ] 跟随 / 上翻停跟；`PageUp/PageDown` 翻详情
- [ ] Agents 面板 `^d` → `Effect::CancelSubagents { agent_id: Some(..) }`（只停高亮项）
- [ ] `LX_TASK_HINTS_AGENTS` 扩到 4 项；文案常量补齐

测试：
- [ ] 宽度 0 时行为 0 行；宽字符/超长 task 不越界
- [ ] 汇报块折叠/展开行数与 `rows()` 同源；展开每次固定追加 24 行，点 `… N more lines` 继续追加
- [ ] 列表窄宽度整项丢弃

---

## 9. 提示词与版本

- [ ] `chat/prompt.rs:28` `PROMPT_VERSION` → `"lx-5"`；`:422` 测试同步
- [ ] `LX_SYSTEM_PROMPT` 插 `# Subagents` 节（design §7.1 原文）
- [ ] `# Tools` 段更新为 11 个工具
- [ ] 不引入 codex 的三条冲突条款

测试：
- [ ] 主提示含 `# Subagents` / `spawn_agent` / `wait_agent` / `close_agent`
- [ ] 主提示**不含** `wait for them before yielding` / `only role becomes to coordinate`
- [ ] `prompt_stays_small` 仍通过（<12 288 / <16 384）
- [ ] `AGENT_NAME` 首行规则仍成立

---

## 10. 收尾

- [ ] `cargo fmt`
- [ ] `cargo clippy --all-targets -- -D warnings`
- [ ] `cargo test`
- [ ] 单文件行数检查（生产代码 ≤1000）
- [ ] 在库副本上验证 v16：`user_version`、数据量、`PRAGMA integrity_check`
- [ ] 请用户 `cargo run` 做交互验收（design §10.7 的七个动作）

---

## 依赖顺序（不可交换）

```text
1 领域层 ─┬─> 2 schema ──> 3 transport
          └─> 4 持久化 v16
                    ↓
              5 loop + 事件层
                    ↓
              6 app 状态/更新
                    ↓
        7 HistorySource（单独提交）──> 8 UI
                    ↓
              9 提示词（与 2 同批落地）
                    ↓
              10 收尾
```

理由：2/3 依赖 1 的 `ToolSurface`；5 依赖 4 的 `begin_subagent_turn`；8 依赖 6 与 7；9 若早于 2 落地，提示会承诺不存在的工具。

## 预估

| 节 | 文件数 | 估算行数 |
|----|--------|----------|
| 1 领域层 | 2 | ~450 |
| 2 schema | 3 | ~260 |
| 3 transport | 6 | ~150 改动 |
| 4 持久化 | 3 | ~450 |
| 5 loop/事件 | 4 | ~700 |
| 6 app | 4 | ~750 |
| 7 HistorySource | 6 | ~200 改动 |
| 8 UI | 4 | ~800 |
| 9 提示词 | 1 | ~90 |
| 合计 | ~33 | ~3900 |
