# Token 速度重算（对齐 opencode v2）、Agent 缓存命中率优化与 `@` 文件列表实时刷新 — 详细设计 (`design.md`)

- **特性目录**：`docs/feature/tps-prompt-cache-and-mention-refresh/`
- **关联任务**：[`task.md`](task.md)
- **目标代码基线**：`dev` 分支 (`726af1d3`，`SCHEMA_VERSION = 33`)
- **参考实现**：
  - `/Users/yonah/projects/agent/opencode-dev`（v2.2.5，`packages/util/src/session-turn.ts`、`packages/ai/README.md`、`packages/ai/src/protocols/{anthropic-messages,open-responses}.ts`、`packages/core/src/filesystem/search.ts`）与上游 `anomalyco/opencode` PR `#63321`（2026-11-11，`fix(session): measure tps from the first streamed token`）/ `#63759`（2026-11-13）
  - `/Users/yonah/projects/agent/codex-main`（`codex-rs/core/src/{client.rs,context/world_state/mod.rs}`、`codex-rs/tui/src/file_search.rs`）
  - `/Users/yonah/projects/agent/openclaude-main`（`src/services/api/{claude.ts,promptCacheBreakDetection.ts}`、`src/hooks/fileSuggestions.ts`）
- **状态**：**待用户审核确认（已通过 Grill Me Q1–Q6 全部边界决策）**

---

## 0. 代码库现状核验与 Grill Me 已确认决策

### 0.1 现状核心痛点（已通过 codegraph / codebase-memory-mcp 核实）

| 领域 | 当前 `lx-tui` 代码事实 | 根因影响 |
| :--- | :--- | :--- |
| **Token 速度 (`tok/s`)** | 1. `src/ui/main_content/lx/history/stats.rs:28-34` 使用 `last_step.output_tokens / (turn_wall_duration_ms / 1000.0)`；<br>2. `src/chat/agent_loop/mod.rs:490-492` 只把最后一步 `usage` 赋给消息；`turn_started_at_ms` 自用户提交后贯穿整轮工具与等待；<br>3. `src/chat/client/mod.rs:165,308` 把失败重试的指数退避睡眠（2s/4s/8s/10s）算进 `usage_log.duration_ms`；<br>4. `agent_message` 表无 `decode_ms` / 整轮输出持久化列，`output_tokens` 被 `context_tokens`（上下文占用%）复用；<br>5. OpenAI Chat/Responses/Anthropic 未单独提取 `reasoning_tokens` / `thinking_tokens` 判断是否出现“隐藏未流式思考”。 | 多步工具调用时分母包含全部工具耗时、TTFT 等待与重试睡眠，分子却只有最后一步输出，`tok/s` 系统性偏低 5–10 倍；重启后也无法复算真实生成速率。 |
| **LLM Prompt Cache 命中率** | 1. **动态环境变量污染前缀**：`stream_start.rs:95` 每轮现采 `<environment_context>`（含分钟级 `<time>` 与动态 `<background_tasks>`）塞入 `PromptParts.tail`，4 类 transport 全插在 **对话最前方（`messages[0]` / `messages[1]`）**；`runtime_summary` 改写内存最后一条 user 消息且不落库；`auto_compact` 改写 `system_prompt`；<br>2. **跨轮工具结果格式分裂**：同轮 `tool_batch.rs:175` 给模型看 `format_for_model` 纯文本，下一轮 `context/mod.rs:882-903` 却从 `result_json` 喂原始 JSON（失败还丢 `Error [code]: `）；<br>3. **零显式缓存控制 & 工具抖动**：Anthropic 全仓 0 处 `cache_control`（命中率恒 0%）；OpenAI/Responses 无 `prompt_cache_key`；`/permission` 切到 `Never` 会调用 `trim_escalation_parameters` 裁剪 `exec_command` schema 导致 `tools` 哈希跳变；<br>4. **不可观测**：缺少类似 `promptCacheBreakDetection` 的前缀哈希断裂诊断。 | 每分钟跳变的时间、任意后台任务变动或每次工具调用后的下一轮，都会导致从头部或上一轮工具起 100% Prefix Cache Miss。 |
| **`@` 文件列表不显示新建文件** | 1. 4 处 `@` 编辑器——主 `lx` 输入框（`LxMention`）、Subagent 常驻追问框（`AgentsState.composers`）、BTW 常驻框（`LxBtw.threads`）、右侧 `Prompt`（`MentionState`）——全部在 `take_scan_request` 里用 `if self.cache.is_some() { return None; }` 死锁缓存；<br>2. `reset_session()` / `escape()` / `clear()` / `suppress()` 只关面板不置脏；<br>3. Agent 执行 `write`/`edit`/`exec_command` 或用户执行 `!cmd` 结束后不通知任何 `@` 缓存失效。 | 除切换工作区 `cwd` 或重启应用外，新建文件永远无法出现在任意一个 `@` 候选列表中。 |

### 0.2 Grill Me 已确认决策汇总

| 问题 | 确认结论 |
| :--- | :--- |
| **Q1：TPS 核心公式** | 对齐 **最新版 opencode v2（PR #63321 / #63759）Decode TPS**：`Σ(output + reasoning) / Σ(stream_end - first_token)`（单次成功尝试内计时，剔除重试退避、工具执行与 TTFT；无 `first_token` 或存在未流式推送的隐藏推理时，该步回退 `attempt_started`；单轮有效 `decode_ms < 250ms` 或 `speed < 0.05` 隐藏速度；页脚不塞 TTFT 文案，总耗时仍显示整轮墙钟）。 |
| **Q2：TPS 持久化与范围** | 覆盖 **主会话 / Subagent 详情 / BTW** 的消息完成态统计行（`complete` 与已有流输出的 `interrupted` 展示，`failed`/`streaming` 不算速度）；保留 `agent_message.{input_tokens, output_tokens}` 专职服务单步 `context_tokens`；通过 **DB v34 迁移** 新增独立字段持久化整轮 `turn_output_tokens`、`decode_ms`、`turn_duration_ms`；修正 `usage_log.duration_ms` 剔除重试退避；不动流中 `chat_bar` 与 Web Usage 页面。 |
| **Q3：提示词前缀与动态上下文顺序** | 保留 `SystemPromptParts::split` `≤ 1900` UTF-16 码元切分（满足 `cbcn/*` 代理兼容）；将 `LX_SYSTEM_PROMPT` + `LX_BACKGROUND_NOTE` + `model_identity` + `AGENTS.md` + `skills` + `mcp` 冻结为**不可变静态前缀**；移除环境快照中每分钟跳变的 `<time>`；将初始环境与动态变化（`cwd`、`/permission`、`<background_tasks>`、`runtime_summary`）后置到**当前用户轮次**并以“仅变才发 Delta + 持久化回放”保证历史字节零改写；子 agent `auto_compact` 摘要注入历史项而非改写系统提示。 |
| **Q4：工具结果跨轮一致性** | 通过 **DB v34 迁移** 为 `agent_tool_call` 新增 `model_output TEXT`，落库保存同轮 `format_for_model` + `truncate_result_for_replay`（64 KiB 上限）最终模型文本；`result_json` 原样保留供 UI 差异卡片使用；`context::replay_pair` 优先读取 `model_output`，仅旧历史行 (`NULL`) 回退到 `strip_ui_diff_json(result_json)`。 |
| **Q5：传输层显式缓存与工具稳定** | 1. Anthropic 默认发送最多 4 个 `cache_control: {"type": "ephemeral"}` 断点（末工具、静态前置规则块、历史滚动断点），支持配置/代理兼容关闭；<br>2. OpenAI 官方 Chat / Responses 发送 `prompt_cache_key`（基于 `session_id`），第三方 `openai-compatible` 默认不盲发以防 400（支持配置显式开启）；<br>3. 移除 `trim_escalation_parameters` 防止 `/permission` 裁剪 `exec_command` schema；<br>4. 保留无签名 `tool_use` 剥离 `thinking` 的防 400 保护；增加前缀哈希变化诊断日志（`lx::cache`）。 |
| **Q6：`@` 列表刷新策略** | 不引入 `notify` 重依赖；4 处 `@` 面板统一采用 **SWR 后台无感重扫 + 按 `@` 会话置脏 + 5s TTL + 工具/Shell 写后置脏**；重扫期间保留旧 `cache` 瞬间过滤（零 `scanning…` 闪烁），后台扫完后平滑替换并尽量按 `path` 维持当前高亮项与视口；同步更新锁定旧契约的单测。 |

---

## 1. 模块一：Token 速度重算（对齐最新版 opencode v2）

### 1.1 opencode v2 核心算法契约

对照 `anomalyco/opencode` PR `#63321`（2026-11-11）与 `#63759`（2026-11-13）以及 `packages/util/src/session-turn.ts`：

$$\text{TPS}_{\text{turn}} = \frac{\sum_{s \in \text{steps}} \text{generated\_tokens}(s)}{\left(\sum_{s \in \text{steps}} \text{decode\_ms}(s)\right) / 1000}$$

其中对单轮内每个已产生输出的有效模型请求步 $s$：
1. **分子 $\text{generated\_tokens}(s)$**：
   - 包含可见输出 token 与思考/推理 token（`output` 已为 inclusive 口径时直接取用，Google 为 `candidatesTokenCount + thoughtsTokenCount`）。
   - 同时显式解析各协议的推理 token（`reasoning_tokens` / `thinking_tokens`）用于**隐藏思考校验**（见下条）。
2. **分母 $\text{decode\_ms}(s)$**：
   - 默认起点为 `first_token_at`（收到该次 HTTP 尝试首个输出事件时刻：非空 `PartKind::Text` / `PartKind::Reasoning` 增量，或 `ToolInputStarted` / `ToolInputDelta`）；
   - 终点为 `stream_ended_at`（本次流读到 EOF / 协议完成帧时刻，**先于**任何工具调度与结算）；
   - **回退条件（避免虚高失真）**：
     - 若某步无 `first_token_at`（如非流式整包），回退到该成功尝试的 `attempt_started`；
     - **隐藏未流式思考保护**（PR `#63321` review 指出的隐患）：若 `usage.reasoning.unwrap_or(0) > 0` 但本步流过程中 `!has_streamed_reasoning`（即服务端把思考藏起来做完了才一次性推文本，`first_token_at` 只落在正文首字上），则强制该步起点回退到 `attempt_started`（防止隐藏思考时间被踢出分母而把 `tok/s` 虚高数倍）。
   - **重试退避隔离**：`attempt_started` 在每次 HTTP 尝试循环内部重置，失败尝试与 `2s/4s/8s/10s` 退避睡眠不计入。
3. **聚合与防爆表护栏**：
   - 必须**先对全轮所有有效步累加分子与分母，再做一次除法**（按各步流时长加权平均）；
   - 若本轮某步缺失合法时长或总有效输出 $\le 0$，或总有效 $\text{decode\_ms} < 250\text{ ms}$（防一次性吞吐包显示 `5000 tok/s`），或计算结果 $< 0.05$ / 非有限数，均不输出 `tok/s`。
4. **耗时 vs 速率解耦**：
   - 统计行末端的**总耗时**（如 `27s`）保持整轮墙钟（扣除用户阻塞交互 `ask` 与审批等待后的工作总耗时）；`tok/s` 仅衡量纯模型生成速率。

### 1.2 数据结构与链路改造

#### 1.2.1 `Usage` 与 `StreamStepMetrics`（`src/db/chat/usage.rs`、`src/chat/events.rs`）

在 `src/db/chat/usage.rs` 扩展 `Usage`（保留公开字段并维持向后兼容构造辅助）：

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Usage {
    pub input: Option<i64>,
    pub output: Option<i64>,
    pub cache_read: Option<i64>,
    pub cache_write: Option<i64>,
    /// 本步包含的思考/推理 token（仅诊断与隐藏思考保护用，不重复加进 inclusive `output`）。
    pub reasoning: Option<i64>,
}
```

> **注意**：`src/` 与测试中存在若干 `Usage { input, output, cache_read, cache_write }` 结构体字面量，需统一补齐 `reasoning: None` 或提供 `..Usage::default()`；`context_tokens()` 口径保持 `input + output` 不变。

在 `src/chat/events.rs` 新增单次请求流指标，并挂到 `StreamOutcome`：

```rust
/// 单次成功模型流尝试的纯耗时窗口与流式特征。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct StreamStepMetrics {
    /// 最终有效尝试从发起 HTTP 请求到流结束的耗时（毫秒，不含重试退避）。
    pub request_ms: i64,
    /// 从首个输出事件（或回退到请求发起）到流结束的纯生成耗时（毫秒）。
    pub decode_ms: i64,
    /// 是否捕获到至少一个真实输出 token 事件。
    pub has_first_token: bool,
    /// 本步流中是否真实收到过思考块内容（用于识别隐藏未流式思考）。
    pub streamed_reasoning: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct StreamOutcome {
    pub deltas: Vec<ChatDelta>,
    pub stop: StopReason,
    /// 本次请求最后一次有效尝试的耗时指标（测试桩可保持 Default）。
    pub metrics: StreamStepMetrics,
}
```

#### 1.2.2 协议推理 Token 解析（`src/chat/transport/`）

| 协议 | 文件与字段 | 逻辑 |
| :--- | :--- | :--- |
| **OpenAI Chat** | `src/chat/transport/mod.rs` (`openai_usage`) | 读取 `completion_tokens_details.reasoning_tokens`（或兼容字段 `reasoning_tokens`）存入 `Usage.reasoning`；`output` 保持 `completion_tokens`（OpenAI/OpenRouter/DeepSeek/Kimi 规范已含 reasoning）。 |
| **OpenAI Responses** | `src/chat/transport/responses/mod.rs:370` | 读取 `output_tokens_details.reasoning_tokens` 存入 `Usage.reasoning`；`output` 保持 `output_tokens`。 |
| **Anthropic Messages** | `src/chat/transport/anthropic/mod.rs:586` | 读取 `usage.output_tokens_details.thinking_tokens`（若代理/新版本返回）存入 `Usage.reasoning`；`output` 保持 `output_tokens`。 |
| **Google Gemini** | `src/chat/transport/google/mod.rs:455` | 将已有 `thoughtsTokenCount` 同步填入 `Usage.reasoning`，`output` 保持 `candidates + thoughts`。 |

#### 1.2.3 客户端精确计时（`src/chat/client/mod.rs`）

在 `run_stream_with_log` 与 `stream_once_inner` 中：
1. `attempt_started = Instant::now()` 放在每次重试循环内（紧接 `stream_once`）；
2. 在 `stream_once_inner` 中维护：
   - `let mut first_token_at: Option<Instant> = None;`
   - `let mut streamed_reasoning = false;`
   - 每次 `decode_frame` / `decoder.finish` 产出的新增 `ChatDelta` 切片中检查：
     ```rust
     fn is_first_output_delta(delta: &ChatDelta) -> bool {
         match delta {
             ChatDelta::Part(part) => !part.text.is_empty(),
             ChatDelta::ToolInputStarted { .. } | ChatDelta::ToolInputDelta { .. } => true,
             _ => false,
         }
     }
     ```
     若 `first_token_at.is_none()` 且存在 `is_first_output_delta`，立刻记录 `first_token_at = Some(Instant::now())`；若出现非空 `PartKind::Reasoning`，置 `streamed_reasoning = true`；
3. 流结束时（`Ok(None)` 或 `finalize_partial` 结束前）记录 `stream_ended = Instant::now()`：
   - `request_ms = stream_ended.saturating_duration_since(attempt_started).as_millis() as i64;`
   - `decode_start = first_token_at.unwrap_or(attempt_started);`
   - `decode_ms = stream_ended.saturating_duration_since(decode_start).as_millis() as i64;`
   - 将最后一次尝试的 `StreamStepMetrics` 返回给 `run_stream_with_log`，并在 `StreamOutcome.metrics` 返回；
4. **修正 `usage_log` 耗时**：`record_stream_finish(log, &request, &deltas, &stop, metrics.request_ms)`（或 `attempt_elapsed_ms`，至少不再把 `2s + 4s + 8s` 退避睡眠写进单次请求统计 `duration_ms`）。

#### 1.2.4 Agent Loop 步级累加与 Steer 隔离（`src/chat/agent_loop/mod.rs`）

在 `TurnSummary` 中新增 `TurnThroughput`：

```rust
/// 一轮内多个成功模型步的吞吐累计（先求和再相除，禁止算术平均）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct TurnThroughput {
    /// 累计已生成 token（output + reasoning），用于分子与 stats_line `out` 展示。
    pub output_tokens: i64,
    /// 累计纯生成耗时（毫秒），已剔除工具耗时、重试退避与可测 TTFT。
    pub decode_ms: i64,
}
```

在 `run_agent_turn` 循环中每步 `stream_one` 返回后：
```rust
fn accumulate_step_throughput(
    throughput: &mut TurnThroughput,
    step_usage: Usage,
    metrics: StreamStepMetrics,
) {
    let Some(output) = step_usage.output.filter(|v| *v > 0) else {
        return;
    };
    let hidden_unstreamed_reasoning =
        step_usage.reasoning.is_some_and(|r| r > 0) && !metrics.streamed_reasoning;
    let step_ms = if hidden_unstreamed_reasoning || !metrics.has_first_token {
        metrics.request_ms
    } else {
        metrics.decode_ms
    };
    if step_ms <= 0 {
        return;
    }
    throughput.output_tokens = throughput.output_tokens.saturating_add(output);
    throughput.decode_ms = throughput.decode_ms.saturating_add(step_ms);
}
```
- **终态约束**：仅当该步不是 `StopReason::Failed` 时累加（若中途某步失败且无部分产出不污染分母；若正常结束或 `Cancelled` 且前序步已有效完成，保留已测步数据）；
- **Steer 边界**：当 `steer` 切出新 assistant 消息段时（`agent_loop/mod.rs:469`），同步重置 `throughput = TurnThroughput::default()`（与 `usage` 同生命周期，保留 `billing`）。

#### 1.2.5 DB v34 持久化、视图投影与 `stats_line` 渲染

1. **`agent_message` 新增列（DB v34，纯 `ALTER TABLE` 增量迁移）**：
   - `turn_output_tokens INTEGER CHECK (turn_output_tokens >= 0)`：本消息全步累计输出 token；
   - `decode_ms INTEGER CHECK (decode_ms >= 0)`：本消息全步累计纯生成耗时（ms）；
   - `turn_duration_ms INTEGER CHECK (turn_duration_ms >= 0)`：本消息结算时的精确墙钟耗时（扣除 `ask`/审批阻塞，ms）。
2. **`ChatMessageView`（`src/app/state/lx/chat/steps.rs`）**：
   - 新增 `pub turn_output_tokens: Option<i64>` 与 `pub decode_ms: Option<i64>`；
   - 原 `input_tokens` / `output_tokens` / `cache_read_tokens` 继续承接最后一步上下文占用；
   - `duration_ms` 在内存实时结算时存入同一套墙钟口径，落库进 `turn_duration_ms`，重载时 `turn_duration_ms.or_else(|| completed - created - ask_wait)` 优先取落库值（保证实时与重载零漂移）。
3. **三条消息结算入口同步透传**：
   - 主会话：`Effect::FinishChatTurn`（`src/app/update/chat/mod.rs`、`src/event/chat/mod.rs`、`src/db/chat/mod.rs`）；
   - 子 Agent：`finish_subagent_with_billing`（`src/event/chat/subagent/settle.rs`、`src/app/update/chat/subagent.rs`、`src/db/chat/subagent.rs`）；
   - 旁路 BTW：`finish_run`（`src/event/chat/btw/run.rs`、`src/app/update/chat/btw/events.rs`、`src/db/chat/btw.rs`）。
4. **`stats_line`（`src/ui/main_content/lx/history/stats.rs`）**：
   ```rust
   pub const MIN_TPS_DECODE_MS: i64 = 250;

   // failed / streaming 消息不出 tok/s（保留 error_code / 耗时提示）。
   let display_output = message.turn_output_tokens.or(message.output_tokens).filter(|t| *t > 0);
   let tps_tokens = message.turn_output_tokens.filter(|t| *t > 0);
   let decode_ms = message.decode_ms.filter(|ms| *ms >= MIN_TPS_DECODE_MS);
   let speed = (message.status != "failed")
       .then(|| tps_tokens.zip(decode_ms))
       .flatten()
       .and_then(|(tokens, ms)| {
           let speed = tokens as f64 / (ms as f64 / 1000.0);
           (speed.is_finite() && speed >= 0.05)
               .then(|| format!("{speed:.1}{}", text::LX_CHAT_STATS_SPEED_SUFFIX))
       });
   ```
   - 旧数据库历史消息（`decode_ms == None`）：不拿整轮墙钟伪算假 `tok/s`（仅展示 `in` / `out` / 耗时）。

---

## 2. 模块二：Agent 工作流与 LLM Prompt Cache 命中率优化

### 2.1 目标与前缀布局原则

理想的多轮/多步请求前缀必须满足 **“从最共享、最不可变到最易变、最新”** 的单调追加结构（参考 `opencode-dev/packages/core/src/session/context.ts` 与 `codex-main/codex-rs/core/src/context/world_state/mod.rs`）：

```text
[1] Tools JSON（内置 + MCP + resources）          ← 全局不变（移除 /permission 裁剪）
[2] Native system slot（<= 1900 UTF-16 units）     ← LX_SYSTEM_PROMPT 首段（固定不变，保 cbcn/* 安全）
[3] messages[0] <system-instructions>              ← LX_SYSTEM_PROMPT 余下 + LX_BACKGROUND_NOTE + model_identity + AGENTS.md + skills + mcp（静态不变！）
[4] 历史轮次 Turns 1..N-1（含每轮冻结的 turn_context、model_output 工具纯文本、user/assistant） ← 字节级 100% 不可变！
[5] 当前第 N 轮上下文（仅变化时追加的 turn_context Delta） + 本轮用户输入 + 本轮工具步 ← 唯一增长尾端
```

### 2.2 核心改造点

#### 2.2.1 冻结静态系统前缀，剥离动态环境快照（`src/chat/prompt.rs`、`src/event/chat/`）

1. **移除每分钟变化的时间字段 `<time>`**：
   - 在 `EnvironmentSnapshot::render_with_sandbox`（`src/chat/prompt.rs:256`）中删除 `<time>HH:MM</time>` 行（保留 `<date>` 与 `<timezone>`；需要精确时分秒时，模型已拥有 `curr_time` 工具，在提示中已有声明）。
   - 同时保留 `MAX_ENVIRONMENT_BYTES = 4096` 与 XML 转义保护。
2. **`PromptParts` 不再默认混入高频变动的 `<environment_context>` 与 `<background_tasks>`**：
   - `compose_system_prompt` 改为纯静态前缀组装：`LX_SYSTEM_PROMPT` + `LX_BACKGROUND_NOTE` + `model_identity` + `instructions` (`AGENTS.md`) + `skills` + `mcp`；
   - **子 Agent 兼容**：`compose_subagent_prompt_with_capabilities` 同样只拼角色提示 + 静态能力限制 + 静态 `model_identity / instructions / skills / mcp`，不把动态环境混进系统前缀。
   - **压缩位置优化**：`agent_loop/context.rs:149` 的 `auto_compact` 摘要与手动 `compact` 摘要不再改写 `system_prompt` 前缀，而是置于**被保留历史窗口的最开头**（作为第一条 `ModelItem::UserText(<compact_summary>...</compact_summary>)`），确保 `tools + system + <system-instructions>` 在压缩前后依然命中同一段静态前缀缓存（在 Anthropic 上该前缀通常 > 4K–15K tokens）。
3. **`SystemPromptParts::split` 切分边界优化（`src/chat/transport/mod.rs:72`）**：
   - 继续严守 `MAX_SYSTEM_MESSAGE_UNITS = 1900` UTF-16 码元上限（保障 `cbcn/*` 代理不被替换）；
   - 在不超过 `1900` 码元的前提下，优先向后退到最近的**段落空行（`\n\n`）或 Markdown 标题行前**切分，避免把 `# Task execution` 等同一规则章节劈成 `system` 和 `user` 两半（无空行时回退原行边界/字符硬切）。

#### 2.2.2 增量环境与运行时摘要持久化（`turn_context` Delta 模型）

新增纯计算模块 `src/chat/context/turn_env.rs`，负责计算每轮的**增量运行上下文**：

1. **数据来源**：
   - 当前环境快照 `env_xml = environment_snapshot_with_sandbox(...)`（含 `workspace_root`、`cwd`、`<sandbox>`、`<background_tasks>`）；
   - 当前运行时摘要 `runtime_summary`（计划快照 + 存活进程 + 活动子 agent + 委派模式）。
2. **Delta 判定逻辑（对齐 `codex` `world_state` SHA1 diff）**：
   - 从本次装配窗口内**之前已完成轮次中最后一条非空 `turn_context`**（或已合并出的基线状态）读取上一次发送给模型的环境与运行时文本快照（可在内存或由最近 `turn_context` XML 块提取 `env` / `runtime` 块对比）：
     - 若是该历史窗口的第 1 轮（含刚 compact 后的新一轮）：写入完整 `<environment_context>...</environment_context>` +（如有）`runtime_summary`；
     - 若后续轮次中 `env_xml` 与上一轮完全一致，则**省略 `env_xml`**（0 token 浪费）；仅当 `cwd`、`/permission`、`<background_tasks>` 发生实质变化时才把新版 `<environment_context>` 放入本轮 `turn_context`；
     - 同理，`runtime_summary` 仅在首次非空或相对上轮发生变化时写入本轮 `turn_context`；
     - 若两者均未变化，本轮 `turn_context = None`。
3. **持久化与回放（DB v34：`agent_message.turn_context TEXT`）**：
   - 在 `begin_turn` / `begin_subagent_turn` / `begin_btw` 提交用户消息时（或 `stream_start` 冻结后写入该 user `agent_message.turn_context`），把本轮计算好的 `turn_context` 原样存入 DB；
   - **关键不变量**：`ModelContext` 不再调用 `absorb_runtime_summary()` 去**原地拼接改写**最后一条用户消息文本！而是在 `context::assemble` 回放每个已完成的 `PlannedTurn`（以及当前最新用户轮）时：
     - 若该轮 user 消息的 `turn_context` 非空，固定在紧邻该用户消息**之前**（或以固定格式前置合并成同一位置 `ModelItem::UserText`，注意若有图片附件 `UserContent` 时放在紧邻前一条 `UserText`）发送，**本轮内多步工具采样 & 未来第 N+1 轮 DB 回放 100% 产生完全相同的字节序列**！
   - `btw` 上下文（`src/chat/btw.rs:114`）：同样把环境快照从 `btw_system_prompt` 挪到 `<btw-context>` 之前或首条 `ModelItem::UserText` 中，保持 `btw` 的 `system_prompt` 纯静态。

#### 2.2.3 工具结果回放单一事实来源（`agent_tool_call.model_output`）

1. **DB Schema v34**：`ALTER TABLE agent_tool_call ADD COLUMN model_output TEXT;`
2. **写入链路**：
   - `src/chat/agent_loop/tool_batch.rs:175` 计算：
     ```rust
     let mut raw = format_for_model(&result);
     let is_error = result.is_failure();
     let mut images = tool_result_images(&result);
     if !supports_image_input && !images.is_empty() {
         images.clear();
         raw.push_str("\n[image omitted: this model does not support image input]");
     }
     let model_output = crate::chat::context::truncate_result_for_replay(&raw);
     ```
   - `ToolOutcome::Executed`（或 `finish_tool` 接口）把 `(result, model_output)` 一并落库到 `agent_tool_call.result_json` 与 `agent_tool_call.model_output`（`model_output` 超出 64 KiB 已被 `truncate_result_for_replay` 裁剪，不会触发 1 MiB 限制）；
   - `context.push_tool_result` 直接使用同一个 `model_output` 字符串！
3. **回放链路（`src/chat/context/mod.rs:863` `replay_pair`）**：
   - 优先使用 `tool.model_output.as_deref().filter(|s| !s.is_empty())`；
   - 若存在关联图片且磁盘文件已缺失，统一追加 `\n[image no longer available]`；
   - 仅当 `tool.model_output` 为 `None`（v34 之前的老库历史行）时，回退至原 `failure_content` / `strip_ui_diff_json`。
   - **效果**：新产生的所有工具调用，在当轮采样 2..K 与未来第 2..M 轮回放中，模型看到的文本 **100% 逐字节相同**。

#### 2.2.4 四大 Transport 显式缓存指令 & 工具 Schema 稳定性

1. **固定 `exec_command` 工具 Schema（`src/chat/tools/schema.rs` & `src/event/chat/stream_start.rs:189`）**：
   - 停止调用 `trim_escalation_parameters(&mut tool_decls)`！无论当前 `/permission` 的 `escalation` 是 `ask` 还是 `never`，`exec_command` 声明的 JSON Schema 始终保持不变；
   - 当 `escalation == Never` 时，由已有的沙箱执行闸门（`src/chat/tools/sandbox_gate.rs` / `agent_loop/sandbox_decision.rs`）拒绝提权请求并返回明确英文错误，同时环境/权限增量 `<system-update>` 已明确告知模型禁止请求提权。
2. **`ChatRequest` 携带会话缓存元数据（`src/chat/events.rs`）**：
   ```rust
   /// 请求级 Prompt Cache 控制选项（由调用方按会话与供应商配置注入）。
   #[derive(Debug, Clone, Default, PartialEq, Eq)]
   pub struct PromptCacheOptions {
       /// 稳定会话亲和键（OpenAI `prompt_cache_key` 使用；None 表示不发送）。
       pub cache_key: Option<String>,
       /// 是否允许发送显式缓存断点（如 Anthropic `cache_control`；默认随传输层安全策略）。
       pub explicit_breakpoints: bool,
       /// 是否允许在 OpenAI 兼容端点发送 `prompt_cache_key`。
       pub send_openai_cache_key: bool,
   }
   ```
   - 在 `stream_start` / `subagent` / `btw` / `exec` 组装 `ChatRequest` 时：
     - `cache_key`: 设为会话稳定键（主会话 `"lx:{session_id}"`，子 Agent `"lx:{session_id}"`，保持长度合规 `<= 64` 字符）；
     - `explicit_breakpoints`: 默认 `true`（若模型/Provider 的 `options` / `requestExtra` 明确设置 `promptCache: false` 则关闭）；
     - `send_openai_cache_key`: 当官方 `openai`（`!compatible`）或 `openai-responses` 且 `base_url` 非已知不支持网关（或配置显式开启 `promptCacheKey: true`）时为 `true`，纯第三方 `openai-compatible` 默认 `false`（防止 HTTP 400）。
3. **Anthropic 显式 4 断点策略（`src/chat/transport/anthropic/mod.rs`）**：
   - 在 `build` 构造完 `tools` 与 `messages` 后，若 `request.cache.explicit_breakpoints == true` 且非一次性摘要压缩/标题请求（`RequestPurpose::CompactSummary | Report` 可按需只缓存 prefix 或跳过），按最多 **4 个断点（`ANTHROPIC_BREAKPOINT_CAP = 4`）** 预算注入 `{"type": "ephemeral"}`：
     1. **断点 1（Tools 末尾）**：`body["tools"]` 数组最后一个元素追加 `"cache_control": {"type": "ephemeral"}`；
     2. **断点 2（静态前置指令块）**：若 `parts.tail_message()` 存在（即 `messages[0]` 为 `<system-instructions>`），将其改写为块数组 `[{"type": "text", "text": tail, "cache_control": {"type": "ephemeral"}}]`（把 `tools + system + static_instructions` 整片长前缀锁进缓存；单块纯文本且在 `messages[0]`，不触发 `system` 2000 码元限制）；若无 `tail` 且 `parts.head` 足够长，则把 `body["system"]` 改为单块 `[{"type": "text", "text": parts.head, "cache_control": {"type": "ephemeral"}}]`；
     3. **断点 3（历史滚动边界）**：当 `messages.len() >= 2` 时，为最后一条稳定消息（多步工具轮中即为最新 `tool_result` 或最新历史消息；实际取 `messages.last_mut()`，并在该消息 `content` 数组最后一个非空 block——若原为字符串则转为 `[{"type": "text", "text": s, "cache_control": ...}]`——注意先统计已有断点不得超过 4 个，且兼容 `thinking` 块不能直接加 `cache_control` 的约束，仅加在 `text` / `tool_result` / `tool_use` 块上！）。
   - **安全保护**：保留现有的 `messages_have_unsigned_assistant_tool_use` 剥离 `thinking` 逻辑，防止因未落库签名触发 Anthropic HTTP 400。
4. **OpenAI Chat / Responses 显式路由键（`openai/mod.rs`, `responses/mod.rs`）**：
   - 当 `request.cache.send_openai_cache_key` 为 `true` 且存在 `cache_key`（且 `requestExtra` 未覆盖）时，写入 `body["prompt_cache_key"] = json!(key)`；
   - 允许通过 `variant_params.requestExtra` 传入 `prompt_cache_retention` 等高级字段；注意 `chatgpt.com/backend-api` 端点保持与现有兼容逻辑一致（`responses/mod.rs:79` 仅在非 `chatgpt.com` 或确认支持时下发，避免 Codex 后端未知字段报错）。
5. **前缀变化诊断日志（`src/chat/cache_diag.rs`）**：
   - 对照 `openclaude-main/src/services/api/promptCacheBreakDetection.ts` 设计轻量化只读哈希观察器（进程内按 `session_id` 存最近一次哈希快照：`tools_hash`、`system_head_hash`、`static_tail_hash`、`prefix_messages_hash`、`prefix_msg_count`）：
   - 每次发出请求时计算当前请求段哈希并与同 `session_id` 上一次请求对比（仅比对 `0..min(prev_count, cur_count - 1)` 前缀区间的消息哈希）；
   - 一旦发现 `tools_hash`、`system_head_hash`、`static_tail_hash` 或历史既有消息哈希发生变化，在 `tracing::debug!` / `warn!`（`target: "lx::cache"`）打印精确告警（哪个段变化、从第几条消息起断裂、上轮 `cache_read` 与本轮变化原因），零额外内存/IO 开销。

---

## 3. 模块三：`@` 文件列表实时刷新（全 4 处输入框统一修复）

### 3.1 现状问题与架构统一

目前项目存在两套结构高度对称的 `@` 提及状态机，覆盖 4 个用户入口：
1. **`LxMention`**（`src/app/state/lx/mention.rs`）：
   - 入口 ①：`lx` 主输入框（`Pane.lx.mention`）
   - 入口 ②：`Agents` 子 Agent 追问输入框（`AgentsState.composers` 内的 `Lx::agent_composer().mention`，常驻 `BTreeMap`）
   - 入口 ③：`BTW` 旁路追问输入框（`LxBtw.threads` 内的 `Lx::composer().mention`，常驻 `BTreeMap`）
2. **`MentionState`**（`src/app/prompt/mention.rs`）：
   - 入口 ④：右侧 `Prompt` 编辑器（`Prompt.mention`）

所有 4 处之所以都无法看到新建文件，是因为共同存在三条死锁：
1. `take_scan_request` 把 `self.cache.is_some()` 当作永久命中（永不重扫）；
2. `@` 会话结束/取消/失焦（`reset_session` / `escape` / `suppress` / `clear`）不把缓存标记为需重新校验；
3. 工具写文件 / Shell 命令结束后没有向这 4 处发送置脏信号。

### 3.2 核心机制：SWR 无感后台重扫 + 按会话置脏 + 5s TTL + 事件置脏

在 `src/app/markdown.rs`（或公共常量模块）统一定义：
```rust
/// `@` 文件缓存保鲜时间（对齐 claude code 5s / opencode 10s）。
pub const MENTION_CACHE_TTL: std::time::Duration = std::time::Duration::from_secs(5);
```

#### 3.2.1 缓存状态字段与刷新判定（`LxMention` & `MentionState` 同构）

在 `LxMention` 与 `MentionState` 中将裸 `cache: Option<Vec<MentionEntry>>` 配套补充元数据：
```rust
/// 缓存是否已被标记为脏（新开 `@` 会话 / 外部写入 / 切换到编辑态时置 true）。
cache_dirty: bool,
/// 最近一次成功写入 `cache` 的单调时钟时刻。
cached_at: Option<Instant>,
```

**SWR 请求触发判定（替换原来的 `if self.cache.is_some() { return None; }`）**：
```rust
fn needs_rescan(&self) -> bool {
    if self.cache.is_none() || self.cache_dirty {
        return true;
    }
    self.cached_at
        .is_none_or(|at| at.elapsed() >= MENTION_CACHE_TTL)
}
```
在 `take_scan_request` 中：
- 先检查常规门控：`tab == Files`（`LxMention`）、`!self.pending`、`!self.suppressed`、`self.trigger.is_some()`、`root` 存在；
- 若 `!self.needs_rescan()` 返回 `None`；
- 否则置 `self.pending = true; self.cache_dirty = false;`，返回 `Some((self.generation, root))`。

> **关键体验点**：因为**不清空 `self.cache`**（只标 `cache_dirty = true`），所以：
> - `view()` / `is_loading()` 只在 `self.cache.is_none()` 时显示 `scanning…`；
> - 只要之前扫过一次，再次敲 `@` 会 **0ms 立即用旧缓存渲染列表**，同时后台线程静默扫描；几十毫秒后新文件平滑并入！

#### 3.2.2 五大置脏时机（涵盖 4 处面板全路径）

1. **开启新的 `@` 会话（Codex 规则）**：
   - 在 `refresh` / `on_edit` 中，记录先前是否处于无触发状态 `let entering_session = self.trigger.is_none();`；
   - 当检测到新的有效 `MentionTrigger` 且 `entering_session` 为真（或者 `trigger.from` 跳到了新的 `@` 位置），**立即置 `self.cache_dirty = true`**；
   - 同时在 `reset_session()` / `escape()` / `suppress()` / `clear()` 结束会话时也置 `self.cache_dirty = true`（双保险，**不清除 `self.cache`**）。
2. **5s TTL 自动过期（Claude Code / OpenCode 规则）**：
   - 只要用户长开 `@` 面板并继续键入字符（或面板在打开状态下经过 5s 再次触发 `take_scan_request`），`cached_at.elapsed() >= 5s` 自动触发一次后台 SWR 刷新。
3. **Subagent / BTW 进入或切入编辑器时置脏**：
   - `enter_task_agent_edit`（`src/app/update/pane/task_page.rs`）：进入某个 Subagent 追问框时调用 `composer.editor_mut().mark_mention_cache_dirty()`；
   - `btw` 打开或切换当前编辑的 QA 线程（`src/app/update/chat/btw/mod.rs` / `input.rs`）：调用 `composer.editor_mut().mark_mention_cache_dirty()`；
   - 同时补全 `sync_btw_composer_context`（`src/app/state/lx/panels.rs:213`），确保 BTW composer 与 Agent composer 一样也能即时同步最新 `referenced_folders`、`skills`、`mcp`。
4. **Agent 工具 & 本地 Shell 写后主动置脏（事件驱动）**：
   - 在 `src/app/update/pane/popups.rs` 新增纯状态函数：
     ```rust
     /// 将工作区内所有可能命中路径变更的 `@` 缓存（主输入框、BTW、Agents 追问与右侧 Prompt）标脏。
     pub fn mark_mention_caches_dirty_for_workspace(state: &mut AppState, changed_root: Option<&Path>)
     ```
     遍历所有工作区的 `PaneView::Lx` 窗格（主编辑器、`btw.threads` 全部线程、`agents.composers` 全部追问框）以及 `state.prompt`，只要其当前有效扫描根未设或与 `changed_root` 同仓库/互为前缀（或保守地全部标记——置脏本身是 O(1) 内存布尔赋值且只在 `@` 打开时才实际扫描！），全部置 `cache_dirty = true`。
   - 调用点：
     - `src/event/chat/tool_calls.rs`（`finish_tool` 结算时，只要工具为 `write`、`edit`、`exec_command`、`write_stdin` 或 `spawn_agent` / `wait_agent` 等可能产生文件变更的工具，且状态为 `completed` / 部分失败写入）；
     - `src/event/chat/subagent/settle.rs`（子 Agent 结束且 `files_changed > 0` 时）；
     - `src/event/chat/user_shell.rs`（用户 `!cmd` 进程进入终态 `exited` / `stopped` 时）。
5. **切换回 `lx` 视图 / 窗口重聚焦时置脏**：
   - 在 `Switch to lx` 动作（`src/app/update/pane.rs`）及右键刷新/工作区切换时顺带标脏该 pane 的 `@` 缓存（切回时一旦敲 `@` 立刻扫最新磁盘状态）。

#### 3.2.3 后台 SWR 回投时的交互稳定性（防用户光标/高亮被抢）

在 `LxMention::apply_entries` 与 `MentionState::apply_entries` 中：
- 写入 `self.cache = Some(entries); self.cached_at = Some(Instant::now()); self.pending = false;`；
- 刷新面板时传入 **`preserve_selection = true`**：
  - 记录刷新前用户当前高亮项的 `active_path: Option<String>`（与 `active` / `anchor` / `viewport`）；
  - 用当前 `trigger.query` 重新调用 `filter_mentions` 得到新 `items`；
  - 若旧高亮 `active_path` 仍在新 `items` 中，自动将 `active` 定位回该路径在新列表里的下标（尽量保持用户没按上下键时选中的还是同一个文件，且视口不被强行重置回 0！）；若用户本来还在第 0 项（或旧选中项已不存在），自然落在第 0 项。
  - 若 `filter_mentions` 在新建文件前本来为空（先前显示 `Empty`），回投包含新文件时面板自动从 `Empty` 恢复为 `Items`！

---

## 4. 关键异常与边界防御矩阵

| 场景 | 预期行为与防线 |
| :--- | :--- |
| **短请求一次性返回（非流式网关，40ms 内返回 200 token）** | `decode_ms = 40 < MIN_TPS_DECODE_MS (250)` → `stats_line` 自动隐藏 `tok/s`，只显示 `in / out / 耗时`，杜绝显示 `5000 tok/s`。 |
| **模型开启深度思考但网关不推送思考流（仅算进 `reasoning_tokens`）** | `usage.reasoning > 0 && !metrics.streamed_reasoning` 命中隐藏思考保护，该步起点回退到 `attempt_started`（含后端隐式思考全过程时长），避免只除以最后半秒正文时间导致速度虚高。 |
| **旧数据库（v33 及更早）历史会话恢复** | `agent_message.decode_ms IS NULL` 时不拿墙钟凑数算假 `tok/s`；`agent_tool_call.model_output IS NULL` 时安全回退 `strip_ui_diff_json(result_json)`；`agent_message.turn_context IS NULL` 时按原结构组装历史。 |
| **第 3 步失败后自动重试成功（中间退避了 2s + 4s）** | `attempt_started` 与 `first_token_at` 只取最后一次成功尝试内的时间；重试退避不污染 `tok/s`，`usage_log.duration_ms` 也同步只记有效请求尝试时长。 |
| **Anthropic 兼容代理 / 9router `cbcn/*` 网关** | 1. `SystemPromptParts::split` 严守 `≤ 1900` UTF-16 码元；<br>2. Anthropic 断点严格限制 `≤ 4` 个且仅标记在 `text` / `tool_result` / `tool_use` / `tools` 末尾（绝不打在 `thinking` 块上）；<br>3. 若供应商配置关闭 `promptCache: false`，零破坏退回不发 `cache_control`。 |
| **第三方 `openai-compatible` 端点拒收未知顶层字段** | `send_openai_cache_key` 仅对官方 OpenAI Chat / Responses 默认开启，第三方 `compatible = true` 默认不发送 `prompt_cache_key`，除非配置显式开启，杜绝 HTTP 400。 |
| **SWR 重扫途中用户已切走/关闭 `@`/切换目录/切换 QA** | `apply_entries` 仍校验 `(generation, root)` 与当前归属（`MentionScanOwner`），过期扫描结果直接丢弃并清理 `pending`，绝不污染当前面板；已激活新目录时保留当前选中路径不乱跳。 |
| **大仓库超过 20,000 条目或新文件刚通过外部脚本创建** | 若仅在编辑期间置脏，SWR 5s TTL + 每次新 `@` 会话必标脏保证了新敲 `@` 必定发起重扫；不修改 `MAX_SCAN_ENTRIES`（防止 100w 文件仓库卡死阻塞线程），继续遵守 `.gitignore` 与重目录忽略。 |

---

## 5. 涉及文件与拆分约束检查

- **生产文件行数红线（≤ 1000 行）**：
  - 所有新增纯逻辑尽量下沉到独立子模块，避免膨胀已有大文件：
    - `src/chat/context/turn_env.rs`（新增，~180 行）：环境与运行时上下文 Delta 计算与基线提取；
    - `src/chat/cache_diag.rs`（新增，~150 行）：会话级前缀哈希计算与 Cache Break 诊断；
    - `src/chat/transport/anthropic/mod.rs`、`src/chat/client/mod.rs`、`src/app/state/lx/mention.rs`、`src/app/prompt/mention.rs`、`src/db/migration.rs`：修改前核对当前行数，超 1000 行时按职责拆出子模块（如 `migration/v34.rs` 或 `anthropic/cache.rs`）。
- **测试组织**：
  - 单元测试同步下沉至对应子模块（如 `src/chat/context/tests/cache_stability.rs`、`src/chat/transport/anthropic/tests.rs`、`src/app/state/lx/mention/tests.rs`、`src/app/prompt/tests/mention_timing.rs`、`src/ui/main_content/lx/history/tests.rs`）；
  - 严格遵守：**无 `unwrap()` / `expect()`、全英文模型/UI 文案、中文注释与 `tracing` 日志、不修改未授权的 Git 状态**。

---

## 6. 验收标准

1. **Token 速度（opencode v2 对齐）**：
   - 多步工具调用轮次（如中间跑 20s 命令），`tok/s` 精确反映模型纯生成速度（`Σ(output+reasoning) / Σ(stream_end - first_token)`），不再被工具执行时间拉低；总耗时仍显示剔除 `ask` 后的整轮工作时间；
   - 关闭重开 `lx-tui` 后重新打开同一历史会话，统计行显示的 `out`、`tok/s` 与耗时与结算时完全一致（主会话、Subagent 详情、BTW 三者一致）；
   - `< 250ms` 爆发包与未流式隐藏思考场景触发防失真保护，`failed` 轮不出假 `tok/s`。
2. **LLM Prompt Cache 命中率优化**：
   - 连续多轮对话 + 多步工具调用之间，`tools`、`system` (`head`)、`<system-instructions>` 与历史已完成轮次的全部 `ModelItem` 序列化字节在相同条件下 **100% 逐字节稳定**（由构造集成测试对比多轮连续 `TransportRequest.body` 前缀直接断言！）；
   - Anthropic 请求包含 ≤4 个合规 `cache_control: {"type": "ephemeral"}` 断点；OpenAI 官方端点携带稳定 `prompt_cache_key`；`/permission` 切换不再改变 `exec_command` 工具 Schema；
   - `cbcn/*` 1900 UTF-16 码元保护与无签名 `tool_use` 剥离 `thinking` 保护 100% 保留。
3. **`@` 文件列表实时刷新**：
   - 在 **主 `lx` 输入框、Subagent 追问框、BTW 输入框、右侧 `Prompt` 编辑器** 4 处，无需切换目录或重启应用：
     - Agent `write`/`edit`/创建新文件后、用户 `!touch` 后、或外部新建文件后，下一次输入 `@`（或长开面板 5s 后）均能实时显示新建文件；
     - 已缓存面板二次输入 `@` 立刻呈现列表、零 `scanning…` 闪烁，且后台更新到达时不打乱用户高亮定位。
4. **工程与规范验证**：
   - `cargo fmt --check`、`cargo clippy --all-targets -- -D warnings` 与覆盖测试全绿，所有改动生产文件行数 ≤ 1000 行。
