# Token 速度重算、Agent 缓存优化与 `@` 列表刷新 — 执行任务清单 (`task.md`)

- **状态**：**待用户审核确认**（审核通过后方可按 Phase 1 ～ Phase 5 开始修改代码）
- **设计依据**：[`design.md`](design.md)

---

## 0. 开工门禁与规范红线

- [ ] **0.1 审核门禁**：用户审核确认 `design.md` 与 `task.md` 后再进入编码阶段。
- [ ] **0.2 硬性规范约束**：
  - 生产代码单文件 ≤ **1000 行**（接近超限的模块如 `src/db/migration.rs`、`src/chat/context/mod.rs`、`src/app/state/lx/mention.rs`、`src/app/prompt/mention.rs` 优先按职责拆分子模块至 ~500 行左右）；
  - 测试文件严格跟随被测子模块目录结构（单文件 ≤ 1500 行），修 Bug 先补复现单测；
  - 生产代码禁止 `unwrap()` / `expect()`，错误统一 `Result` + `?`；
  - 模型可见文本、工具报错与 TUI 用户可见文案一律英文，代码注释与 `tracing` 日志使用简体中文；
  - `app/` 保持纯内存状态无 IO，`ui/` 保持纯只读渲染；
  - 严禁未经用户明确授权执行任何写 Git 操作（不 commit / checkout / reset）；
  - 改完后精确执行 `cargo fmt`、`cargo clippy --all-targets -- -D warnings` 与受影响模块 `cargo test`（并视需要跑对应集成测试）。

---

## Phase 1：数据库 Schema v34 增量迁移与持久化基础 (`src/db/`)

> **目标**：通过纯增量 `ALTER TABLE` 完成 v34 迁移，同时服务“Token 速度持久化”与“Prompt Cache 稳定回放（`turn_context` + `model_output`）”，零破坏旧库兼容性。

- [ ] **1.1 推进 Schema 版本与迁移实现 (`src/db/mod.rs`、`src/db/migration.rs` / `src/db/migration/schema.rs`)**
  - `src/db/mod.rs:33` 将 `SCHEMA_VERSION` 从 `33` 升至 `34`。
  - 在 `src/db/migration/schema.rs` 建表定义中同步加入新列；并在 `src/db/migration.rs`（若文件逼近 1000 行则拆出 `src/db/migration/v34.rs`）实现幂等 `migrate_v34_tps_and_cache`：
    - `agent_message` 新增 4 个可空列（不重建表）：
      - `turn_output_tokens INTEGER CHECK (turn_output_tokens >= 0)`
      - `decode_ms INTEGER CHECK (decode_ms >= 0)`
      - `turn_duration_ms INTEGER CHECK (turn_duration_ms >= 0)`
      - `turn_context TEXT`
    - `agent_tool_call` 新增 1 个可空列：
      - `model_output TEXT`
  - 迁移后执行 `ensure_no_foreign_key_violation` 并将 `PRAGMA user_version` 推进到 `34`。
- [ ] **1.2 扩展 `db::chat` 读写模型 (`src/db/chat/usage.rs`, `mod.rs`, `tools.rs`, `subagent.rs`, `btw.rs`, `fork.rs`)**
  - `src/db/chat/usage.rs`：`Usage` 增加 `pub reasoning: Option<i64>`（兼容更新字面量构造处或补充构造辅助），保持 `context_tokens()` 仍为 `input + output`。
  - `src/db/chat/mod.rs`：
    - 定义 `TurnStats { pub output_tokens: Option<i64>, pub decode_ms: Option<i64>, pub duration_ms: Option<i64> }`（或放入 `Usage` / `TurnOutcome` 旁独立结构）。
    - `ChatMessage` 新增 `turn_output_tokens: Option<i64>`、`decode_ms: Option<i64>`、`turn_duration_ms: Option<i64>`、`turn_context: Option<String>`。
    - `begin_turn` / `begin_turn_with_context`（及 `begin_subagent_turn` / `begin_btw`）支持写入用户消息的 `turn_context`（或新增事务辅助 `set_user_turn_context` 在 `stream_start` 冻结后立即落库）。
    - `finish_with_optional_parts_in_tx`（及 `subagent.rs` / `btw.rs` 对应结算方法）写入 `turn_output_tokens`、`decode_ms`、`turn_duration_ms`；`load_chat_session` 全量读出这 4 列。
    - `fork.rs`：历史克隆复制 `agent_message` 与 `agent_tool_call` 时一并复制 `turn_output_tokens`、`decode_ms`、`turn_duration_ms`、`turn_context` 与 `model_output`！
  - `src/db/chat/tools.rs`：
    - `ToolCallRecord` 与 `ToolCallResult` 新增 `pub model_output: Option<String>`。
    - `finish_tool_call` 将 `model_output` 写入 `agent_tool_call.model_output`（`settle_without_update` 幂等判定对齐）；`load_tool_calls_for_session` 查出 `t.model_output`。
- [ ] **1.3 Phase 1 单元测试 (`src/db/migration/tests.rs`, `src/db/chat/tests.rs`)**
  - 验证 v33（及更早版本）旧库幂等升级到 v34，现有数据 100% 保留、外键无违规。
  - 验证新字段往返存取、`fork` 复制保留及旧库 `NULL` 字段安全读取。

---

## Phase 2：Token 速度重算（对齐最新版 opencode v2）（`src/chat/`, `src/event/`, `src/app/`, `src/ui/`）

> **目标**：精准采集每步 `first_token` → `stream_end` 与全轮 `(output + reasoning)` 累计，剔除重试/工具/TTFT，修复主会话、Subagent 与 BTW 的统计行。

- [ ] **2.1 协议层解析推理 Token (`src/chat/transport/`)**
  - `src/chat/transport/mod.rs` (`openai_usage`)：提取 `completion_tokens_details.reasoning_tokens`（及 `reasoning_tokens` 兼容键）填充 `Usage.reasoning`。
  - `src/chat/transport/responses/mod.rs`：提取 `output_tokens_details.reasoning_tokens` 填充 `Usage.reasoning`。
  - `src/chat/transport/anthropic/mod.rs`：提取 `output_tokens_details.thinking_tokens`（若存在）填充 `Usage.reasoning`。
  - `src/chat/transport/google/mod.rs`：将已有 `thoughtsTokenCount` 填入 `Usage.reasoning`，`output` 保持 `candidates + thoughts`。
- [ ] **2.2 流式客户端精确计时 & 修复 `usage_log` 耗时 (`src/chat/events.rs`, `src/chat/client/mod.rs`)**
  - `src/chat/events.rs`：`StreamStepMetrics { request_ms, decode_ms, has_first_token, streamed_reasoning }` 加入 `StreamOutcome`（带 `#[derive(Default)]` 兼容现有构造）。
  - `src/chat/client/mod.rs`：
    - `stream_once_inner` 记录本尝试的 `attempt_started`、首个真实输出增量 `first_token_at`（非空 `Text`/`Reasoning` 或 `ToolInputStarted`/`ToolInputDelta`）、`streamed_reasoning` 及结束 `stream_ended`；
    - 若某次尝试失败并走指数退避重试，上一轮失败尝试计时作废，仅返回最终尝试的 `StreamStepMetrics`；
    - `record_stream_finish` 改传 `metrics.request_ms`（若尚未发起即取消传 `0`），使 `usage_log.duration_ms` 不再被重试退避睡眠撑大。
- [ ] **2.3 Agent Loop 跨步吞吐聚合 (`src/chat/agent_loop/mod.rs`)**
  - 定义 `TurnThroughput { pub output_tokens: i64, pub decode_ms: i64 }`，挂在 `TurnSummary.throughput`。
  - 实现 `accumulate_step_throughput`：
    - 仅在步非 `Failed` 且 `output > 0` 时累计；
    - 触发隐藏未流式思考保护（`reasoning > 0 && !streamed_reasoning`）或 `!has_first_token` 时用 `metrics.request_ms`，否则用 `metrics.decode_ms`；
    - `steer` 分段切换时重置 `throughput`（同 `usage`）。
- [ ] **2.4 事件层与 App 状态/UI 渲染打通 (`src/event/chat/`, `src/app/`, `src/ui/main_content/lx/history/stats.rs`)**
  - 主会话（`src/event/chat/mod.rs`、`src/app/update/chat/mod.rs`、`src/app/effects.rs`）：`on_chat_finished` 与 `Effect::FinishChatTurn` 透传 `TurnThroughput` 与扣除 ask 等待后的 `turn_duration_ms`。
  - 子 Agent（`src/event/chat/subagent/settle.rs`、`src/app/update/chat/subagent.rs`）：`finish_subagent_with_billing` 透传 `TurnThroughput` 与扣等待后 `turn_duration_ms` 到内存 `SubagentRun.chat` 消息及 DB。
  - BTW（`src/event/chat/btw/run.rs`、`src/app/update/chat/btw/events.rs`）：`finish_run` 透传 `TurnThroughput` 与 `turn_duration_ms` 到内存与 DB。
  - `src/app/update/chat/view.rs`：`view_message_of` 从 `ChatMessage` 还原 `turn_output_tokens`、`decode_ms` 与 `duration_ms`（优先 `turn_duration_ms`，回退 `completed - created - Σask`）。
  - `src/ui/main_content/lx/history/stats.rs`：
    - 展示的 `out` 优先取 `turn_output_tokens.or(output_tokens)`；
    - `tok/s` 使用 `turn_output_tokens` 与 `decode_ms >= 250` 计算（`failed` / `streaming` / 缺 `decode_ms` 不计算），保留 `speed >= 0.05` 与 `{:.1} tok/s`。
- [ ] **2.5 Phase 2 单元测试**
  - `src/chat/client/tests/`：验证 TTFT 等待 2s + 流吐字 1s 时 `decode_ms ≈ 1000`、`request_ms ≈ 3000`；验证失败重试退避 2s 不算入成功尝试耗时。
  - `src/chat/agent_loop/tests/`：验证 2 步任务中间工具耗时 10s 时，`TurnSummary.throughput` 只累加两步输出与两步纯流耗时（按步加权之和，不平均）、验证隐藏思考未推流时回退 `request_ms`。
  - `src/ui/main_content/lx/history/tests.rs` & `src/app/update/chat/tests/`：更新原单步速度断言，新增多步加权 TPS、<250ms 隐藏保护、v33 历史无 `decode_ms` 降级与重启重载一致性测试。

---

## Phase 3：Agent 工作流与 LLM Prompt Cache 优化 (`src/chat/`, `src/event/chat/`)

> **目标**：冻结静态前缀与工具 Schema、将动态环境移至当前轮增量落库并统一 `format_for_model` 回放，补齐 Anthropic/OpenAI 显式缓存指令与前缀断裂诊断。

- [ ] **3.1 静态提示词拆分与移除易变 `<time>` (`src/chat/prompt.rs`, `src/chat/transport/mod.rs`, `src/chat/agent/prompts.rs`, `src/chat/btw.rs`)**
  - `src/chat/prompt.rs`：
    - `EnvironmentSnapshot::render_with_sandbox` 移除每分钟变化的时间行 `<time>...</time>`（保留 `<date>` 与 `<timezone>`）。
    - `PromptParts::render_tail_with` / `compose_system_prompt` 与 `compose_subagent_prompt_with_capabilities` **不再**在系统前缀尾部默认插入动态环境快照（仅测试兼容入口 `system_prompt_with(Some(env))` 可保留或迁移测试，确保生产 `stream_start` / `subagent` / `exec` / `btw` 传 `environment: None` 并改走 `turn_context`）。
    - `btw_system_prompt` 变为纯静态只读指令（环境片段改为首条上下文 `ModelItem` 携带）。
  - `src/chat/transport/mod.rs`：`SystemPromptParts::split` 在 `<= 1900` UTF-16 码元前提下优先退回到 `\n\n` 段落边界切分（找不到空行才回退行边界），避免规则章节被拦腰劈断。
- [ ] **3.2 增量 `turn_context` 模块与装配集成 (`src/chat/context/turn_env.rs`, `src/chat/context/mod.rs`, `src/chat/agent_loop/context.rs`)**
  - 新增 `src/chat/context/turn_env.rs`：
    - 提供 `build_turn_context_delta(previous_env: Option<&str>, current_env: &str, previous_runtime: Option<&str>, current_runtime: Option<&str>) -> Option<String>`，以及从历史消息 `turn_context` 集合提取最近基线的纯函数。
    - 仅在首轮或字段发生实质变化（`cwd`、`/permission`、`<background_tasks>`、计划/存活进程）时生成结构化 `<turn_context>`（含变化段）；未变化返回 `None`。
  - `src/chat/context/mod.rs`（注意控制行数 ≤ 1000，必要时拆出辅助子模块）：
    - `PlannedTurn` 增加 `pub turn_context: Option<String>`；
    - `assemble` 在回放每个历史 `PlannedTurn` 及当前轮草稿时，若存在 `turn_context`，固定插在该轮 user 输入前同一稳定 `ModelItem::UserText` 槽位中，确保跨轮回放与当轮实时构造字节 100% 一致；
    - 移除 `ModelContext::absorb_runtime_summary()` 对最后一条用户消息的原地改写逻辑！
  - `src/chat/agent_loop/context.rs`：
    - `auto_compact` 触发或从子 Agent `compact_summary` 恢复时，不再把 `<compact_summary>` 拼进 `system_prompt`，改为置于 `items` 头部（紧随 `<system-instructions>` 之后，历史正文之前）。
- [ ] **3.3 工具结果回放单一口径 (`src/chat/agent_loop/tool_batch.rs`, `src/chat/context/mod.rs`, `src/event/chat/tool_calls.rs`)**
  - `tool_batch.rs:175` 计算经 `truncate_result_for_replay` 对齐的 `model_output`，同时传给 `host.finish_tool` 落库与 `context.push_tool_result`。
  - `context/mod.rs::replay_pair` 优先读取 `tool.model_output`，仅 `None`（老库历史行）回退 `failure_content` / `strip_ui_diff_json`。
- [ ] **3.4 固定工具 Schema + 显式 Provider 缓存 + 诊断 (`src/chat/tools/schema.rs`, `src/chat/transport/`, `src/chat/cache_diag.rs`, `src/event/chat/`)**
  - 移除 `stream_start.rs:189`（及 `exec` / `subagent` 对应位置）的 `trim_escalation_parameters` 裁剪（保留该函数或删除死代码前核对所有调用点及单测），确保 `/permission` 切换期间 `exec_command` schema 始终恒定。
  - `ChatRequest` 新增 `pub cache: PromptCacheOptions`（由 `stream_start` / `subagent` / `btw` / `exec` 按会话 ID 与供应商类型填充）。
  - `src/chat/transport/anthropic/mod.rs`（必要时拆出 `cache.rs`）：在 `RequestPurpose` 适用请求上注入 ≤ 4 个 `cache_control: {"type": "ephemeral"}` 断点（末工具 + `<system-instructions>` 或 `system` 块 + 最新稳定非 `thinking` 消息块）。
  - `src/chat/transport/openai/mod.rs` 与 `responses/mod.rs`：在官方端点（或配置开启）下发送 `prompt_cache_key`，第三方 `openai-compatible` 默认跳过防 400。
  - 新增 `src/chat/cache_diag.rs`：计算 `(tools_hash, system_head_hash, static_tail_hash, prefix_messages_hash)` 并在 `client/mod.rs` 请求前记录前缀意外变更的 `tracing` 诊断日志（`target: "lx::cache"`）。
- [ ] **3.5 Phase 3 单元测试与多轮字节一致性测试**
  - `src/chat/context/tests/cache_stability.rs`：端到端构造 **第 1 轮（含 2 次工具调用）→ 第 2 轮（无工具）→ 第 3 轮（切换权限与新后台任务）** 的真实请求，对比四类 Transport 序列化后的 JSON bytes，断言第 N+1 轮的前缀字节 **100% 逐字节包含** 第 N 轮完整历史前缀！
  - `src/chat/transport/anthropic/tests.rs`：断言 `cache_control` 断点数量 ≤ 4、不落在 `thinking` 块、关闭开关时不注入。
  - `src/chat/transport/openai/tests.rs` & `responses/tests.rs`：断言官方端点发 `prompt_cache_key`、`compatible = true` 默认不发、`SystemPromptParts` 1900 码元保护与转义测试保持全绿。

---

## Phase 4：`@` 文件列表实时刷新（全 4 处面板统一修复）（`src/app/`, `src/event/`）

> **目标**：修复主输入框、Subagent 追问、BTW 与右侧 `Prompt` 4 处 `@` 列表不显示新建文件的 Bug，实现零闪烁 SWR 后台刷新。

- [ ] **4.1 改造 `LxMention` SWR 与置脏状态机 (`src/app/state/lx/mention.rs`, `src/app/state/lx/panels.rs`)**
  - 新增 `MENTION_CACHE_TTL = Duration::from_secs(5)`、`cache_dirty: bool`（初始 `true`）、`cached_at: Option<Instant>` 与 `pub fn mark_cache_dirty(&mut self)`。
  - 更新 `take_scan_request`：以 `self.cache.is_none() || self.cache_dirty || self.cached_at.is_none_or(|t| t.elapsed() >= MENTION_CACHE_TTL)` 判定重扫（**不清空 `self.cache`**，仅 `self.cache.is_none()` 时返回 `LxMentionView::Loading`）。
  - 在 `refresh`（新进入 `@` 会话或切换 `trigger.from` 时）、`reset_session`、`escape`、`suppress` 中置 `self.cache_dirty = true`。
  - 在 `apply_entries` 中写入 `cached_at = Some(Instant::now())`；若面板本来已打开（SWR 后台更新），通过保存并恢复当前高亮 `entry.path` 与可见偏移避免打断用户上下选择。
  - 补全 `sync_btw_composer_context` 同步完整上下文数据。
- [ ] **4.2 同步改造右侧 `Prompt` 的 `MentionState` (`src/app/prompt/mention.rs`, `src/app/prompt.rs`)**
  - 对称加入 `cache_dirty: bool`、`cached_at: Option<Instant>`、`pub fn mark_cache_dirty(&mut self)`。
  - 更新 `take_scan_request` 为 SWR + `cache_dirty` + 5s TTL；在 `refresh` 新入 `@`、`reset_session`、`escape`、`clear` 中置 `self.cache_dirty = true`（不清空 `self.cache`）。
  - `apply_entries` 记录 `cached_at` 并在已有面板时按路径稳定保持高亮。
- [ ] **4.3 打通事件层主动置脏通道 (`src/app/update/pane/popups.rs`, `task_page.rs`, `btw/`, `src/event/chat/`)**
  - 在 `src/app/update/pane/popups.rs` 新增 `mark_mention_caches_dirty_for_workspace(state, changed_root)`，统一置脏主输入框、全部 `btw.threads`、`agents.composers` 及右侧 `state.prompt` 的文件提及缓存。
  - 接入调用点：
    - `src/app/update/pane/task_page.rs`（`enter_task_agent_edit` 进入 Subagent 追问框时）；
    - `src/app/update/chat/btw/{mod,input}.rs`（开启或切入 BTW 线程编辑时）；
    - `src/app/update/pane.rs`（`Switch to lx` 切换视图时）；
    - `src/event/chat/tool_calls.rs`（`write`/`edit`/`exec_command`/`write_stdin`/`spawn_agent`/`wait_agent` 等工具结算完成时）；
    - `src/event/chat/subagent/settle.rs`（子 Agent 完成且 `files_changed > 0` 时）；
    - `src/event/chat/user_shell.rs`（本地 `!cmd` 进程退出或停止时）。
- [ ] **4.4 Phase 4 单元测试与契约更新**
  - 更新 `src/app/state/lx/mention/tests.rs`（修正原 `typing_filters_candidates_without_rescan`：校验同一次 `@` 会话内极短时间连续打字不重复发起扫描、而新开 `@` 会话 / 标脏 / 超 5s TTL 触发 SWR 后台重扫，且重扫期间面板无 `Loading` 闪烁、已有列表可继续过滤、到达新文件后按路径保持高亮）。
  - 同步更新 `src/app/prompt/tests.rs`、`src/app/prompt/mention/tests.rs` 与 `src/app/prompt/tests/mention_timing.rs`。
  - 新增 4 处入口的回归测试：主 `lx`、Subagent 常驻追问框、BTW 常驻框与右侧 `Prompt` 在不换 `root` 情况下均可刷新出新建文件（包括 `write` 工具回调与 `!cmd` 触发后的置脏验证）。

---

## Phase 5：全量联调检查与精确验收

- [ ] **5.1 静态规范检查**
  - 检查所有改动及新增生产文件行数均 `<= 1000` 行，测试文件遵循子模块拆分且无大单体堆积。
  - 检查无 `unwrap()` / `expect()` 泄漏到生产代码中，无中文文案泄漏到模型提示或 TUI 用户可见文本。
- [ ] **5.2 精准编译与测试门禁**
  - `cargo fmt --check`
  - `cargo clippy --all-targets -- -D warnings`
  - 分模块与集成验证：
    - `cargo test db::migration` & `cargo test db::chat`
    - `cargo test chat::client` & `cargo test chat::transport` & `cargo test chat::context` & `cargo test chat::agent_loop` & `cargo test chat::prompt`
    - `cargo test app::state::lx::mention` & `cargo test app::prompt` & `cargo test app::update::chat`
    - `cargo test ui::main_content::lx` & `cargo test event::chat`
- [ ] **5.3 撰写验收记录 `verification.md` 并交付用户交互确认（TUI 界面效果由用户运行 `cargo run` 实测确认）**
