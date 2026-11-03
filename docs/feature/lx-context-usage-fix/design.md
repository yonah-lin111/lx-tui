# lx 上下文窗口统计单步重构：设计方案

> 状态：**已合并到 dev**（2026-11-03，合并提交 `51f31675`）；自动验证与 CLI 实测通过。
> 日期：2026-11-03。
> 项目：`/Users/yonah/projects/agent/lx-tui`。
> 配套执行清单：[task.md](./task.md)。

## 1. 问题背景与根因分析

### 1.1 现象还原（会话 `sess-18e42d519de95ed0-1`）

在开发库 `~/.lx-t-dev/db/lx.db` 中查询会话 `sess-18e42d519de95ed0-1`（模型 `cbcn/deepseek-v4.1-flash`，配置 `context_limit = 1,000,000`）：

| sequence | role | turn_index | status | input_tokens | output_tokens | cache_read_tokens | 工具调用数 |
| --- | --- | --- | --- | --- | --- | --- | --- |
| 0 | user | 0 | complete | NULL | NULL | NULL | 0 |
| 1 | assistant | 0 | complete | 22,089 | 204 | 16,384 | 0 |
| 2 | user | 1 | complete | NULL | NULL | NULL | 0 |
| 3 | assistant | 1 | complete | 22,273 | 553 | 16,384 | 0 |
| 4 | user | 2 | complete | NULL | NULL | NULL | 0 |
| 5 | assistant | 2 | interrupted | 121,131 | 754 | 86,656 | 10 |
| 7 | user | 3 | complete | NULL | NULL | NULL | 0 |
| 8 | assistant | 3 | complete | 257,523 | 5,480 | 199,680 | 9 |
| 9 | user | 4 | complete | NULL | NULL | NULL | 0 |
| **10** | **assistant** | **4** | **complete** | **2,696,606** | **66,519** | **2,396,160** | **51** |

- **为什么显示 100%**：
  第 4 轮连续执行了 51 次工具调用。`src/chat/agent_loop/mod.rs` 中的 `add_usage` 将 51 次 HTTP 请求的 `input_tokens` 全部累加（$51 \times \approx 52,874 = 2,696,606$），并在整轮结束时写入 `agent_message.input_tokens`。会话恢复（`on_session_loaded_full`）或撤销（`on_session_rewound`）时直接取最后一条 assistant 消息的 `input_tokens + output_tokens = 2,763,125`，除以 `1,000,000` 得 `276%`，被 `percent.min(100)` 截断显示为 `2.8M(100%)`。
- **为什么重新发送后变为 8%**：
  新消息发起单次请求时，`on_stream_delta`（`src/app/update/chat/mod.rs:416`）收到单次请求的 `ChatDelta::Usage`（真实上下文约 `80,000` tokens），覆写 `context_tokens` 后计算 $80,000 / 1,000,000 = 8\%$。

### 1.2 现有实现的 4 处缺陷

1. **多步工具循环把每步输入累加覆盖成了单条消息的 `usage`**：
   - `src/chat/agent_loop/mod.rs:680, 957`：`add_usage` 把每一步的 `input / output / cache_read / cache_write` 全部求和作为 `TurnSummary.usage`，既写入了 `agent_message`，也写入了 `agent_session.total_*`。
   - 对比 `opencode-dev`（`packages/client/src/solid/usage.ts`）、`codex-main`（`codex-rs/protocol/src/protocol.rs:2505` `last_token_usage`）与 `openclaude-main`（`src/utils/tokens.ts:77, 278`）：三者均严格按**单次（最后一步）API 响应**记录消息级/当前上下文 token，绝不跨工具步骤累加 `input_tokens`。
2. **Anthropic 协议的 `input_tokens` 未包含缓存输入（口径不一致）**：
   - OpenAI（`prompt_tokens`）、OpenAI Responses（`input_tokens`）和 Google Gemini（`promptTokenCount`）返回的 `input` **已经包含**缓存命中 token（`cached_tokens` 是其子集）。
   - 但 Anthropic `message_start` 返回的 `input_tokens` 仅代表**未命中缓存也未写入缓存的尾部输入**，完整 Prompt 输入长度为 `input_tokens + cache_read_input_tokens + cache_creation_input_tokens`（参考 `opencode-dev/packages/tui/src/util/session.ts:53` 与 `openclaude-main/src/utils/tokens.ts:77`）。
   - 现有 `src/chat/transport/anthropic/mod.rs:283` 直接把裸 `input_tokens` 赋给 `Usage.input`，导致命中缓存时：
     - 底边框上下文占用严重偏小（漏算了缓存命中的大头）；
     - 消息底部统计行 `stats_line` 计算缓存命中率 `cache_read * 100 / input` 时分母比分子还小，被 `clamp(0, 100)` 硬卡在 `100% cached`。
3. **流式分帧上报（Anthropic / Google）时单步 `input` 与 `output` 割裂**：
   - Anthropic 在 `message_start` 帧上报 `input`，在末尾 `message_delta` 帧上报 `Usage { input: None, output: Some(out), .. }`。
   - `src/app/update/chat/mod.rs:419` 中 `if let Some(input) = usage.input` 在收到 `message_delta` 时因为 `usage.input` 为 `None` 而直接跳过，导致 Anthropic 流式期间及完成瞬间 `context_tokens` 永远漏算 `output_tokens`。
4. **未遵守 `/compact` 压缩边界**：
   - `opencode-dev`（`lastAssistantWithUsage`）与 `openclaude-main`（`getMessagesAfterCompactBoundary`）只取最后一次生效压缩之后的 assistant 消息。
   - 现有 `lx-tui` 在 `/compact` 完成、`on_session_loaded_full` 或 `on_session_rewound` 时忽略了 `lx.compact.active_upto_sequence()`，导致压缩后仍显示压缩前的高占用百分比。

---

## 2. 已确认决策记录（Grill-Me）

| 编号 | 决策内容 | 用户确认 |
| --- | --- | --- |
| D1 | **单步重构**：对齐 OpenCode / Claude / Codex，消息级 `Usage`（`agent_message` 的 `input_tokens`、`output_tokens`、`cache_read_tokens`、`cache_write_tokens`）重构为记录**最近一次（最后一步）API 响应**的单步快照，不再跨工具步累加；历史测试脏数据无需兼容保留。 | 用户确认：“使用推荐”、“重构为单步的，之前的历史消息也可以不要了，反正也是测试消息” |
| D2 | **Anthropic 输入归一化与单步分帧合并**：Anthropic 的 `Usage.input` 归一化为完整 Prompt 输入（`input_tokens + cache_read_input_tokens + cache_creation_input_tokens`），与 OpenAI / Gemini 口径统一；同一次 HTTP 响应内分帧到达的 `Usage` 合并为完整单步后再更新 `context_tokens = input + output`。 | 问题 2，用户回答“使用推荐” |
| D3 | **压缩边界过滤（`/compact`）**：计算/恢复 `context_tokens` 时，严格过滤掉 `sequence <= active_upto_sequence`（已生效压缩边界及之前）的旧消息；刚完成 `/compact` 且尚无新回复时清空 `context_tokens`（隐藏旧百分比），撤销压缩时按新边界恢复。 | 问题 3，用户回答“使用推荐” |
| D4 | **零数据库表结构变更**：复用现有 `agent_message` 四列存最后一步单步 `Usage`；`agent_session` 的 `total_input_tokens / total_output_tokens` 同样无需改表，不递增 `SCHEMA_VERSION`。 | 问题 4，用户确认重构为单步 |

---

## 3. 代码检索与影响范围（CodeGraph + Codebase Memory）

已使用 `codegraph_explore` 与 `codebase-memory-mcp` 核查调用链与爆炸半径：

| 模块 / 文件 | 关键符号 | 变更职责 |
| --- | --- | --- |
| `src/chat/transport/anthropic/mod.rs` | `Decoder::decode_frame` (`"message_start"`, `"message_delta"`) | 将 `message_start` 的 `input` 归一化为 `input_tokens + cache_read + cache_write`（统一各协议 `Usage.input` 为完整 Prompt tokens）。 |
| `src/chat/agent_loop/mod.rs` | `step_usage`, `run_agent_turn`, `TurnSummary` | 将跨步累加的 `add_usage` 替换为单步合并函数 `step_usage(&outcome.deltas) -> Option<Usage>`；每次模型采样若返回有效 `Usage`，直接覆盖本轮当前单步 `usage`。 |
| `src/app/state/lx/chat.rs` | `LxChat::step_usage`, `LxChat::recompute_context_tokens` | 新增流中当前采样步的 `step_usage: Usage` 聚合槽（在 `ResponseFinished` / 新流开始时重置），确保分帧到达的 `input` 与 `output` 合并计算 `context_tokens`；新增按生效 compact 边界重算 `context_tokens` 的纯函数方法（模型键由 `refresh_status` 维护）。 |
| `src/app/update/chat/mod.rs` | `on_stream_delta`, `on_stream_finished` | `ChatDelta::Usage` 通过 `step_usage` 合并同一步内的 `input` 与 `output` 更新 `context_tokens`；`ResponseFinished` 重置单步槽供下一步采样使用；`on_stream_finished` 用最终单步 `usage` 落入 `message` 并同步 `context_tokens`。 |
| `src/app/update/chat/session.rs` | `on_session_loaded_full` | 改调 `target.lx.chat.recompute_context_tokens(&target.lx.compact)` 统一恢复上下文占用。 |
| `src/app/update/chat/rewind.rs` | `on_session_rewound` | 改调 `target.lx.chat.recompute_context_tokens(&target.lx.compact)`（同时补齐此前遗漏的 `last_tokens_model` 恢复）。 |
| `src/app/update/chat/compact.rs` | `on_compact_finished`, `on_compact_reverted`, `set_checkpoints` | 检查点变更后调用 `recompute_context_tokens(&target.lx.compact)` 与 `refresh_lx_status`，使压缩完成立即清空旧百分比、撤销压缩或加载检查点立即按边界重算。 |

---

## 4. 关键设计与核心代码

### 4.1 Anthropic Prompt Token 口径归一化（`src/chat/transport/anthropic/mod.rs`）

OpenAI、Responses 与 Google 的 `Usage.input` 均已包含 `cache_read`。为了让上层（`context_tokens` 与 `stats_line` 缓存命中率 `cache_read / input`）无需感知底层协议差异，在 Anthropic 解码器 `message_start` 处将 `input` 统一为完整输入 token 数：

```rust
"message_start" => {
    // Anthropic 的 input_tokens 仅为未缓存尾部；完整 prompt 输入需加上 cache_read 与 cache_write，
    // 对齐 OpenAI prompt_tokens / OpenCode / Claude getTokenCountFromUsage 口径。
    if let Some(usage) = value
        .get("message")
        .and_then(|message| message.get("usage"))
    {
        let raw_input = json_i64(usage, "input_tokens");
        let cache_read = json_i64(usage, "cache_read_input_tokens");
        let cache_write = json_i64(usage, "cache_creation_input_tokens");
        if raw_input.is_some() || cache_read.is_some() || cache_write.is_some() {
            let total_input = raw_input
                .unwrap_or(0)
                .saturating_add(cache_read.unwrap_or(0))
                .saturating_add(cache_write.unwrap_or(0));
            out.push(ChatDelta::Usage(crate::db::chat::Usage {
                input: Some(total_input),
                output: None,
                cache_read,
                cache_write,
            }));
        }
    }
}
```

### 4.2 Agent Loop 单步 Usage 提取与覆盖（`src/chat/agent_loop/mod.rs`）

在同一次 `stream_one` 调用内，流可能发出多个 `ChatDelta::Usage`（如 Anthropic 先在 `message_start` 发 `input/cache_read/cache_write`，后在 `message_delta` 发 `output`；或 Google 每帧携带最新 `usageMetadata`）。

- **同一步内（单次 `stream_one`）**：后到达的非 `None` 字段覆盖同一步内先到达的字段（合并成完整的单次 API 响应 `Usage`）。
- **跨步之间（多步工具循环）**：若当前步产出了有效 `Usage`，直接**覆盖**整轮的 `usage`（若新一步在首字节前就被取消、未产出任何 `Usage`，则保留上一步已完成的 `usage`）。

```rust
/// 从单次模型响应的增量流中合并该步的 Usage（分帧上报的字段合并为一次完整快照）。
fn step_usage(deltas: &[ChatDelta]) -> Option<Usage> {
    let mut step = Usage::default();
    let mut seen = false;
    for delta in deltas {
        let ChatDelta::Usage(part) = delta else {
            continue;
        };
        seen = true;
        if part.input.is_some() {
            step.input = part.input;
        }
        if part.output.is_some() {
            step.output = part.output;
        }
        if part.cache_read.is_some() {
            step.cache_read = part.cache_read;
        }
        if part.cache_write.is_some() {
            step.cache_write = part.cache_write;
        }
    }
    seen.then_some(step)
}
```

在 `run_agent_turn` 循环中：

```rust
let outcome = stream_one(context.next_request(), cancel.clone()).await;
if let Some(latest) = step_usage(&outcome.deltas) {
    usage = latest;
}
```

### 4.3 流式期间的分帧合并与单步重置（`src/app/state/lx/chat.rs` & `src/app/update/chat/mod.rs`）

在 `LxChat` 中维护当前正在流式接收的模型采样步的临时 `step_usage: Usage`：

1. 新轮次开始（`begin_turn` / `finish_response` / `close_stream`）时，`step_usage` 重置为 `Usage::default()`。
2. `on_stream_delta` 收到 `ChatDelta::Usage(usage)` 时：
   - 将非 `None` 的 `input` / `output` / `cache_read` / `cache_write` 合入 `target.lx.chat.step_usage`；
   - 只要 `step_usage.input` 或 `step_usage.output` 有值，立即计算单步上下文占用：
     ```rust
     pub fn absorb_usage_delta(&mut self, delta: &Usage) {
         if delta.input.is_some() {
             self.step_usage.input = delta.input;
         }
         if delta.output.is_some() {
             self.step_usage.output = delta.output;
         }
         if delta.cache_read.is_some() {
             self.step_usage.cache_read = delta.cache_read;
         }
         if delta.cache_write.is_some() {
             self.step_usage.cache_write = delta.cache_write;
         }
         if self.step_usage.input.is_some() || self.step_usage.output.is_some() {
             let input = self.step_usage.input.unwrap_or(0).max(0);
             let output = self.step_usage.output.unwrap_or(0).max(0);
             self.context_tokens = Some(u64::try_from(input.saturating_add(output)).unwrap_or(u64::MAX));
         }
     }
     ```
3. 当收到 `ChatDelta::ResponseFinished` 时，除关闭思考块外，重置 `self.step_usage = Usage::default()`，保证下一步工具采样开始时不会把上一步的 `output` 混进新一步。

### 4.4 遵循 `/compact` 压缩边界的上下文恢复（`LxChat::recompute_context_tokens`）

对齐 OpenCode 的 `lastAssistantWithUsage(messages, boundary)`：

- 从 `LxCompact::active_upto_sequence()` 取得当前生效压缩检查点覆盖的最大消息 `sequence`（无生效检查点时为 `None`）。
- 倒序扫描 `self.messages`，只考虑：
  1. `sequence > upto_sequence`（位于最新生效压缩边界之后）；
  2. `role == "assistant"`；
  3. `input_tokens.is_some() || output_tokens.is_some()`。
- 找到后：`context_tokens = Some((input + output) as u64)`。
- 若未找到（例如全新会话，或刚完成 `/compact` 后尚无新回复）：`context_tokens = None`。
- `last_tokens_model` 由 `refresh_status` 按当前选择维护；会话加载/撤销时仍从 DB 消息快照回填，用于切模型清空判断。

```rust
/// 按当前消息列表与生效的 `/compact` 检查点边界重算上下文窗口占用。
///
/// 对齐 OpenCode `lastAssistantWithUsage`：只取最新生效压缩检查点之后的最后一条
/// 带用量的 assistant 消息；压缩后尚无新回复时清空占用分子。
pub fn recompute_context_tokens(&mut self, compact: &LxCompact) {
    let boundary = compact.active_upto_sequence();
    let recent = self.messages.iter().rev().find(|message| {
        message.role == "assistant"
            && boundary.is_none_or(|upto| message.sequence > upto)
            && (message.input_tokens.is_some() || message.output_tokens.is_some())
    });
    self.context_tokens = recent.map(|message| {
        let input = message.input_tokens.unwrap_or(0).max(0);
        let output = message.output_tokens.unwrap_or(0).max(0);
        u64::try_from(input.saturating_add(output)).unwrap_or(u64::MAX)
    });
}
```

并在以下 5 个入口统一调用 `recompute_context_tokens` + `refresh_lx_status`：
1. `on_session_loaded_full`（`src/app/update/chat/session.rs`）
2. `on_session_rewound`（`src/app/update/chat/rewind.rs`）
3. `on_compact_finished`（`src/app/update/chat/compact.rs`）
4. `on_compact_reverted`（`src/app/update/chat/compact.rs`）
5. `set_checkpoints`（`src/app/update/chat/compact.rs`，解决 `db/service/chat.rs` 中先 `on_session_loaded_full` 后 `set_compact_checkpoints` 的顺序问题）

---

## 5. 边界条件与不变量验证

1. **多步工具循环后中途取消/失败**：
   - 若第 1 步采样成功（`input=10k, output=200`），执行工具后在第 2 步采样尚未返回 `Usage` 前用户按 Esc 取消，`step_usage` 对第 2 步返回 `None`，保留第 1 步的 `Usage(10k, 200)` 落库，上下文占用仍准确反映已知的最后一次模型窗口状态。
2. **切换模型（`/model`）**：
   - `recompute_context_tokens` 同步设置 `last_tokens_model`（连同 `on_session_rewound` 之前漏设 `last_tokens_model` 的隐患一并修复），随后 `refresh_status` 比对当前生效模型的 `token_key`，若切换了模型则自动清空 `context_tokens`。
3. **文件行数规范**：
   - 受影响生产文件均保持在 1000 行上限内；对应单元测试随模块就近放置在各自的 `tests` 子模块中。
