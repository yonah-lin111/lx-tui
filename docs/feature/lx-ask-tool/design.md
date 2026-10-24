# lx `ask` 工具设计

> 状态：待用户审核（未开始编码）
> 参考：codex `request_user_input`（`codex-rs/core/src/tools/handlers/request_user_input*.rs`、`codex-rs/collaboration-mode-templates/templates/default.md`）、Claude Code `AskUserQuestion`（选项 `preview` 字符画）
> 适用分支：`dev`，主 checkout

---

## 1. 目标

在 lx 页的 Native 会话里新增第 8 个模型工具 `ask`：模型在「多个合理方案、用户偏好决定结果」时向用户提问；用户在 lx 窗格内作答；答案作为工具结果回给模型，模型按答案继续。

必须支持：

1. 1–3 道题，逐题作答；
2. 单选、多选（每题独立声明）；
3. 每题可带「其他」自由输入，编辑手感与现有 lx 输入框完全一致；
4. 每个选项可带 `preview` 纯文本字符画，用于设计类问题（布局、界面草图、框线图）；
5. 键盘 + 鼠标两套操作；
6. 阻塞整轮，`Esc` 取消整轮，窗格不可见时有标签页标记。

非目标（本版不做）：

- 不新增数据库表/列，不改 `SCHEMA_VERSION`；
- 不做图片/富文本预览，不解析 Markdown；
- 不做超时自动回答、不做倒计时（codex 的 `autoResolutionMs` 已弃用，不跟进）；
- 不做子代理（lx 无子代理），不做 plan/default 模式区分（lx 无模式）；
- 不把 ask 的问题做成全局浮层，不跨窗格聚合。

---

## 2. 已确认的决策（逐条对应盘问）

| # | 决策 | 结论 |
|---|------|------|
| 1 | 阻塞语义 | **阻塞整轮**；作答前模型不再采样 |
| 2 | 界面位置 | **替换 lx 输入框**（同位置、同内外边距） |
| 3 | 预览粒度 | **每个选项一份 `preview`**，纯文本原样渲染 |
| 4 | 多题流程 | **逐题**，`1/3` 进度，可回看 |
| 5 | `Esc` | **取消整轮**；沿用 lx 现有「双击确认」习惯；编辑态 `Esc` 只退出编辑 |
| 6 | 选择模式 | **支持多选**（每题 `multi_select`）+ 自由输入 |
| 7 | 自由输入 | **复用 lx 输入框现有编辑逻辑**；独立缓冲，草稿不搬迁 |
| 8 | 面板尺寸 | 高度上限 **窗格 70%**（最小 10 行）；宽 ≥ 60 列左右分栏 |
| 9 | 校验 | 结构性违规**整次拒绝**，不弹面板；允许 0 选项的纯自由输入题 |
| 10 | 结果与留痕 | 结果 `{"answers":{...}}`；历史卡片**不展示 preview**；复用现有工具调用落库 |
| 11 | 编辑缓冲 | **独立缓冲 + 复用编辑核心**；prompt 草稿永不搬迁 |
| 12 | 鼠标 | **支持**：点选项/点「其他」/点题目标签/点 Submit/滚轮 |
| 13 | 命名与提示词 | 工具名 `ask`，动词 `Ask`，提示词按 codex 口径，`PROMPT_VERSION` → `lx-3` |
| 14 | 鼠标语义 | 单选单击只高亮不提交；多选单击即勾选；`Submit` 是唯一提交入口（`Enter` 等价） |
| 15 | 不可见窗格 | 包含该窗格的**标签页标题加 `?` 标记** |

---

## 3. 不变量（实现时必须守住）

1. **app 纯状态**：`app/` 不持有 tokio 通道、不执行 IO；作答只登记 `Effect`。
2. **ui 纯渲染**：`ui/` 只读状态、只写 `Buffer`，不解析 JSON、不改状态。
3. **DB 是唯一真相**：ask 是普通工具调用；等待期间库里是 `running`，结束后 `completed` / `cancelled` / `failed` / 重启后 `interrupted`。
4. **结果先落库再进上下文**：沿用 `run_agent_turn` 现有顺序，ask 不例外。
5. **不伪造答案**：取消/失败绝不返回空答案数组冒充「用户全不选」。
6. **草稿安全**：作答期间 `Lx.input` 一个字节都不动。
7. **有界**：参数、预览、单题答案、结果 JSON 全部有硬上限；超限显式拒绝或截断标记。
8. **不新增全局浮层槽位**：`AppState.overlay` 不参与。

---

## 4. 数据模型

### 4.1 模型可见参数 Schema

加入 `src/chat/tools/schema.rs` 的 `TOOLS`（7 → 8）：

```json
{
  "type": "object",
  "properties": {
    "questions": {
      "type": "array",
      "minItems": 1,
      "maxItems": 3,
      "description": "Questions to show the user, in order. Prefer 1.",
      "items": {
        "type": "object",
        "properties": {
          "id":       { "type": "string", "description": "Stable snake_case id used to map answers." },
          "header":   { "type": "string", "description": "Short label shown in the panel (12 chars or fewer)." },
          "question": { "type": "string", "description": "One self-contained question sentence." },
          "multi_select": {
            "type": "boolean",
            "description": "Allow selecting more than one option (default false)."
          },
          "options": {
            "type": "array",
            "maxItems": 4,
            "description": "0-4 choices. Put the recommended one first. Do not add an 'Other' option: the client adds free-form input automatically.",
            "items": {
              "type": "object",
              "properties": {
                "label":       { "type": "string", "description": "User-facing label (1-5 words)." },
                "description": { "type": "string", "description": "One short sentence about the impact/tradeoff." },
                "preview":     { "type": "string", "description": "Optional plain-text monospace sketch (layout/diagram). Rendered verbatim." }
              },
              "required": ["label", "description"],
              "additionalProperties": false
            }
          }
        },
        "required": ["id", "header", "question"],
        "additionalProperties": false
      }
    }
  },
  "required": ["questions"],
  "additionalProperties": false
}
```

工具描述（英文，与 schema/其它工具同语言）：

> Ask the user 1-3 short questions and block the turn until they answer. Use it only when a user preference really decides the outcome and the workspace cannot answer it. Each question has 0-4 options (recommended first) plus automatic free-form input; `multi_select` allows several choices. Optional `preview` is a plain-text monospace sketch rendered verbatim (useful for layout/design choices). The result is `{"answers":{"<id>":{"answers":[...]}}}`. Do not ask permission questions.

### 4.2 硬上限（`src/chat/tools/ask.rs`）

| 名字 | 值 | 说明 |
|---|---|---|
| `MAX_ASK_QUESTIONS` | 3 | 题目数 |
| `MAX_ASK_OPTIONS` | 4 | 每题选项数（0 允许） |
| `MAX_ASK_HEADER_CHARS` | 12 | `header` 字符数 |
| `MAX_ASK_QUESTION_CHARS` | 500 | 题干字符数 |
| `MAX_ASK_LABEL_CHARS` | 80 | 选项标签字符数 |
| `MAX_ASK_DESC_CHARS` | 200 | 选项说明字符数 |
| `MAX_ASK_ID_CHARS` | 64 | `id` 字符数 |
| `MAX_ASK_PREVIEW_BYTES` | 4096 | 单个 preview 字节数 |
| `MAX_ASK_PREVIEW_LINES` | 40 | 单个 preview 行数 |
| `MAX_ASK_PREVIEW_LINE_CHARS` | 400 | preview 单行字符数 |
| `MAX_ASK_PREVIEW_TOTAL_BYTES` | 24576 | 整次调用所有 preview 合计字节 |
| `MAX_ASK_ANSWER_CHARS` | 8192 | 单题答案（含手写文本）合计字符 |
| `MAX_ASK_CUSTOM_BYTES` | 8192 | 单题手写文本字节数 |

参数总量另有现有上限 `MAX_TOOL_ARGUMENTS_BYTES = 1 MiB`（`src/chat/events.rs:228`）。

### 4.3 校验规则（纯函数，手工逐字段，风格对齐 `chat/tools/plan_args.rs`）

```rust
pub fn parse_arguments(arguments_json: &str) -> ToolResult<Vec<AskQuestion>>
```

拒绝条件（返回 `ToolFailure::new(ToolErrorCode::InvalidArguments, ...)` 或 `PlanInvalid` 级专用码，消息为稳定英文，带题号/字段名）：

1. 顶层不是 JSON 对象、含未知字段、缺 `questions`；
2. `questions` 不是数组、长度 0 或 > 3；
3. 任一题缺 `id` / `header` / `question`，或出现未知字段；
4. 题内或跨题的 `id` 重复；`id` 非 `[a-z0-9_]` 或超长；
5. `header`/`question`/`label`/`description` 超字符上限；
6. `options` 存在但不是数组、长度 > 4；单个选项缺 `label`/`description` 或含未知字段；
7. 同题内 `label` 重复（大小写敏感）；
8. `preview` 不是字符串、超字节/行/单行长度，或整次合计超 24 KiB；
9. 任何字段含不可见控制字符（`\n` 除外；`\t` 允许，渲染时展开为空格）；
10. `multi_select` 不是布尔。

注意：`options` 缺省或空数组合法（纯自由输入题）。

### 4.4 执行结果

`src/chat/tools/types.rs`：

```rust
pub enum ToolExecutionResult {
    // ...现有分支...
    Ask { questions: Vec<AnsweredQuestion> },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AnsweredQuestion {
    pub id: String,
    pub header: String,
    pub question: String,
    /// 勾选项（按勾选顺序）在前，手写文本（若有）在最后；已去重。
    pub answers: Vec<String>,
}
```

`src/chat/tools/output.rs`：

- `format_for_model`（模型可见）——与 codex 同构：

```json
{"answers":{"layout_choice":{"answers":["双栏","自己画一版"]}}}
```

- `to_result_json`（落库 + UI 解码）——自包含，卡片不需要再读参数：

```json
{"tool":"ask","questions":[{"id":"layout_choice","header":"布局","question":"侧栏用哪种布局？","answers":["双栏"]}]}
```

- `to_summary`：**不改** `ToolSummary`。运行中卡片只显示 `waiting for your answer`；结算后从 detail 取数量。

### 4.5 UI 状态（`src/app/state/lx/ask.rs`）

```rust
#[derive(Debug)]
pub struct AskState {
    /// 归属键（作答 Effect 与归属校验用）。
    pub key: RequestKey,
    pub call_id: String,
    pub questions: Vec<AskQuestion>,     // 来自 chat::tools::ask
    pub current: usize,
    /// 每题高亮行：0..options.len()，最后一行固定是「其他」。
    pub highlight: Vec<usize>,
    /// 每题已勾选项（按下标保存，顺序即勾选顺序）。
    pub selected: Vec<Vec<usize>>,
    /// 每题手写答案（独立编辑缓冲，绝不复用 Lx.input）。
    pub custom: Vec<AskEditor>,
    /// 每题手写文本是否计入答案（提交编辑态即计入；清空即不计入）。
    pub custom_on: Vec<bool>,
    /// 已提交的题答案（回看/重提时保留）。
    pub collected: Vec<Vec<String>>,
    /// 当前题是否处于编辑态。
    pub editing: bool,
    /// 每题预览滚动偏移。
    pub preview_scroll: Vec<usize>,
    /// 每题选项列表滚动偏移。
    pub list_scroll: Vec<usize>,
    /// 内联提示（例如「select at least one option」）。
    pub hint: Option<String>,
}

/// 独立编辑缓冲：包一层 `Lx`，只暴露文本编辑 API。
///
/// 复用理由：软换行、宽字符、撤销/重做、词移动、粘贴等逻辑全部已在 `Lx` 里，
/// 另写一份必然漂移。这里构造一个空 `Lx`（chat/rewind/task 等字段恒为空），
/// 只把 `LxInput` 的编辑子集转发进去。
#[derive(Debug)]
pub struct AskEditor(Lx);
```

`Lx` 增加字段：

```rust
pub struct Lx {
    // ...
    /// 当前挂起的 ask 面板；None 表示输入框正常显示。
    pub ask: Option<AskState>,
}
```

`Lx::new()` 初始化 `ask: None`。

依赖方向：`app/state` → `chat/tools/ask` 只依赖**纯数据类型**（`AskQuestion` / `AskOption`），与现有 `PlanSnapshotView::from_snapshot(&PlanSnapshot)` 同一先例，不引入反向依赖。

---

## 5. 控制流

### 5.1 全链路时序

```text
模型响应含 tool call "ask"
        │
        ▼
run_agent_turn (src/chat/agent_loop/mod.rs:501)
  ├─ host.begin_tool(call)            → DB pending → UI pending → ticket
  ├─ ask::parse_arguments(args)       → 失败：直接 Failed 结果，不弹面板
  ├─ host.ask_user(call_id, qs)       ─────────────┐   (阻塞 await)
  │                                                ▼
  │                     AppEvent::AgentHost(AskUser { key, call_id, questions, reply })
  │                                                │
  │                     event/chat/ask.rs::on_requested
  │                       ├─ active_assistant_id 校验（旧 generation → Err(Cancelled)）
  │                       ├─ 写入 state.pane(key.pane).lx.ask
  │                       └─ streams.asks.insert(key, PendingAsk{ reply })
  │                                                │
  │                     用户按键/鼠标 → app/update/chat/ask.rs（纯状态）
  │                       └─ 最后一题提交 → Effect::AnswerAsk { key, answers }
  │                                                │
  │                     event/chat/mod.rs 效应泵
  │                       ├─ 校验收件 ask 的 call_id 仍匹配
  │                       ├─ systems.asks.remove(key).reply.send(Ok(answers))
  │                       └─ 清除 state 的 ask（双保险）
  │                                                │
  ├─ 收到答案 → ask::answered_result → ToolExecutionResult::Ask
  ├─ host.finish_tool(ticket, Executed(result)) → DB 终态 + 卡片 detail
  └─ context.push_tool_result(...) → 下一次采样
```

取消路径（`Esc` 双击、关窗格、切会话、`/new`）：

```text
Effect::CancelChatStream / reap_stale_streams
  └─ streams.cancel(key)
       ├─ 现有 watch 取消信号
       └─ 新增：pending ask.reply.send(Err(HostError::Cancelled))
              + 清除 state.pane(key.pane).lx.ask
  ▼
run_agent_turn 收到 Err(Cancelled)
  └─ finish_tool(ticket, ToolOutcome::Cancelled) → DB cancelled
     cancel_unstarted(remaining) → TurnSummary::interrupted
```

### 5.2 AgentHost 扩展（`src/chat/agent_loop/mod.rs`）

```rust
pub enum AgentHostRequest {
    BeginTool { call: ReadyToolCall, reply: oneshot::Sender<Result<ToolCallTicket, HostError>> },
    FinishTool { ticket: ToolCallTicket, outcome: ToolOutcome, reply: oneshot::Sender<Result<(), HostError>> },
    /// 新增：把提问交给 UI 并等待作答。
    AskUser {
        call_id: String,
        questions: Vec<AskQuestion>,
        reply: oneshot::Sender<Result<AskAnswers, HostError>>,
    },
    UserShellUpdates { reply: oneshot::Sender<Vec<String>> },
}

/// 用户作答：`id -> 答案数组`（勾选项按顺序，手写排最后）。
pub type AskAnswers = std::collections::BTreeMap<String, Vec<String>>;

pub trait AgentHost: Send {
    // ...现有方法...

    fn ask_user(
        &mut self,
        call_id: String,
        questions: Vec<AskQuestion>,
    ) -> impl Future<Output = Result<AskAnswers, HostError>> + Send
    where
        Self: Sized,
    {
        async move {
            let (reply, receiver) = oneshot::channel();
            self.send(AgentHostRequest::AskUser { call_id, questions, reply });
            receiver.await.unwrap_or(Err(HostError::Cancelled))
        }
    }
}
```

`src/event/chat/host.rs`：`AgentHostEvent::AskUser { key, call_id, questions, reply }`，`EventHost::send` 映射。

### 5.3 `run_agent_turn` 分支（精确插入点）

在现有 `准入二`（`executor.execute_tracked(...)`，`mod.rs:522`）处替换为：

```rust
let result = if call.name == crate::chat::tools::ask::ASK_TOOL {
    match crate::chat::tools::ask::parse_arguments(&call.arguments_json) {
        Err(failure) => failure.into_result(Vec::new()),
        Ok(questions) => match host.ask_user(call.call_id.clone(), questions.clone()).await {
            Ok(answers) => crate::chat::tools::ask::answered_result(&questions, &answers),
            Err(HostError::Cancelled) => {
                // 本轮被取消：只结算这一行，未开始的调用按 cancelled 处理，不再采样。
                let _ = host.finish_tool(ticket, ToolOutcome::Cancelled).await;
                cancel_unstarted(&mut host, remaining).await;
                return TurnSummary::interrupted(usage);
            }
            Err(HostError::Failed { code }) => {
                let failure = crate::chat::tools::ToolFailure::new(
                    crate::chat::tools::ToolErrorCode::Io,
                    code.clone(),
                )
                .into_result(Vec::new());
                let _ = host.finish_tool(ticket, ToolOutcome::Executed(failure)).await;
                return TurnSummary::failed(code, usage);
            }
        },
    }
} else {
    executor
        .execute_tracked(context.cwd(), &call, &ticket)
        .await
};
```

其余顺序（`format_for_model` → `finish_tool` → `push_tool_result`）不变。

### 5.4 `ChatStreams` 扩展（`src/event/chat/mod.rs`）

```rust
pub struct ChatStreams {
    // ...
    /// 每轮最多一个挂起提问（loop 串行）。
    pub(crate) asks: HashMap<RequestKey, PendingAsk>,
}

pub(crate) struct PendingAsk {
    pub call_id: String,
    /// None 表示回执已丢失（窗格关闭等）。
    pub reply: Option<oneshot::Sender<Result<AskAnswers, HostError>>>,
}
```

- `cancel(&key)`：除现有 watch 取消外，取走 `asks.remove(key)`，`reply.send(Err(Cancelled))`。
- `finish(&key)`：取走并丢弃（若仍在，一律 `Err(Cancelled)`），防止悬挂。
- `reap_stale_streams` 已调用 `cancel`，无需重复改。

### 5.5 作答回传（app 纯状态 → 事件层执行）

`src/app/effects.rs`：

```rust
/// 提交一次 ask 作答；事件层据此唤醒 agent loop。
AnswerAsk {
    key: crate::app::state::lx::RequestKey,
    answers: std::collections::BTreeMap<String, Vec<String>>,
},
```

事件层效应泵（`src/event/chat/mod.rs` 中处理 `Effect::*` 的位置）：

```rust
Effect::AnswerAsk { key, answers } => {
    if let Some(pending) = streams.asks.remove(&key) {
        // 防串台：UI 提交的答案必须对应当前挂起的 call。
        if pending.call_id == submitted_call_id {
            if let Some(reply) = pending.reply {
                let _ = reply.send(Ok(answers));
            }
        }
    }
    false
}
```

> 实现细节：`Effect::AnswerAsk` 额外携带 `call_id`，用于与 `PendingAsk.call_id` 比对；不匹配则丢弃（旧面板的迟到提交）。

UI 侧提交时同步 `lx.ask = None`；若回执发送失败（turn 已结束），事件层不报错，UI 也已回到输入框。

---

## 6. 状态机

### 6.1 面板级

```text
        AskUser 到达                   最后一题提交 / 取消
Idle ───────────────▶ Asking ───────────────────────────▶ Idle
                        │
                        │ Esc（第一次）
                        ▼
                    Armed(3s 内) ── Esc ──▶ 取消整轮
```

- 每次 `AskUser` 只允许一个挂起；已有 `lx.ask` 时事件层回 `Err(Failed)`，不覆盖。
- `ChatStreams.finish` / `cancel` / 流结束都清 `lx.ask`（防御性 sweep：`lx.ask.is_some() && !chat.request.is_busy()` 时清除）。

### 6.2 题目级

当前题状态由 `editing` 与 `selected/custom_on` 派生，不另存枚举：

| 状态 | 条件 | 主键位 |
|---|---|---|
| 列表态 | `editing == false` | `↑↓`/`space`/`enter`/`tab`/`←→` |
| 编辑态 | `editing == true` | lx 编辑子集；`enter` 提交文本，`esc` 退出编辑 |

### 6.3 提交规则

1. `Enter` / 点 `Submit`：当前题答案非空 → 写入 `collected[current]`；不是最后一题则 `current += 1`；是最后一题则构造 `answers` map、清 `lx.ask`、推 `Effect::AnswerAsk`。
2. 当前题答案为空 → 设 `hint = "select at least one option"`，不提交、不前进。
3. 单选规则：勾选某选项会清空该题手写答案；提交手写答案会清空该题勾选（保证「单选=一个答案」）。
4. 多选规则：勾选与手写可共存；答案 = 勾选顺序 + 手写（去重）。
5. 回看：`collected[current]` 已存在时，重新进入该题会把勾选/手写恢复为已提交状态（`selected`/`custom` 本来就是就地编辑，天然保留）。

---

## 7. 键位与鼠标契约

### 7.1 键盘（列表态）

| 键 | 行为 |
|---|---|
| `↑` / `↓` / `k` / `j` | 移动高亮（含「其他」行） |
| `Space` | 勾选/取消高亮项；多选=切换，单选=设为唯一勾选；高亮「其他」= 进入编辑态 |
| `1`–`9` | 直选第 N 项（同 Space 的选择语义）；再按同一数字取消（多选） |
| `Enter` | 提交当前题（`Submit`）；高亮「其他」时进入编辑态 |
| `Tab` / `→` | 下一题（保留各题状态） |
| `Shift+Tab` / `←` | 上一题 |
| `PgUp` / `PgDn` | 滚动预览（预览区可见时） |
| `Esc` | 第一次：进入停止待命并显示 `esc again to cancel`；3 秒内第二次：取消整轮 |

### 7.2 键盘（编辑态）

| 键 | 行为 |
|---|---|
| `Enter` | 提交文本为本题手写答案并退出编辑（不提交整题） |
| `Shift+Enter` / `Ctrl+Enter` | 换行 |
| 其余编辑键 | 与 lx 输入框完全一致（`Ctrl+U/K/W/A/E`、`Ctrl+Z/Y`、`Alt+B/F/D`、`Backspace/Delete`、`Home/End`、`←→`、`Ctrl+C` 清空） |
| `Esc` | 退出编辑，文本保留，不取消整轮 |
| `Tab` | 编辑态吞掉（不切题，避免误操作） |

> 映射复用：`src/input/mod.rs::lx_input_key` 不改；`LxInput` 原样进入 `apply_lx_input`，由它把 ask 分支排在最前。

### 7.3 鼠标

| 位置 | 行为 |
|---|---|
| 选项行单击 | 高亮该项 + 切换预览；单选=独占选中、多选切换勾选/取消；**都不提交**（提交只有 Submit/Enter） |
| 「其他」行单击 | 进入编辑态，光标落到文本末尾 |
| 题目标签单击（≥2 题） | 切到该题，状态保留 |
| `Submit` 行单击 | 等价 `Enter` |
| 预览区滚轮 | 只滚预览，不改高亮 |
| 选项区滚轮 | 移动高亮（与命令面板一致） |
| 面板内其它位置 | 吞掉，不穿透到历史卡片 |
| 面板外 | 正常处理（历史点击、滚动、聚焦窗格） |

**命中与渲染共用几何**：`ui/main_content/lx/ask.rs::layout()` 同时被渲染、鼠标命中、滚轮分派调用；禁止在事件层重算。

---

## 8. 渲染

### 8.1 布局

在 `ui/main_content/lx.rs::layout()` 增加分支（优先级：后台任务页 > ask > 正常输入框）：

```rust
let (box_h, visible, scroll) = if lx.task.is_some() {
    (task::list_height(), 0, 0)
} else if lx.ask.is_some() {
    (ask::height(area, lx), 0, 0)
} else {
    /* 现有逻辑 */
};
// input 矩形照旧计算；ask 激活时 out.input = None，out.ask = Some(input_rect)
```

`ask::height`：

```text
max_h = max(10, area.height * 70 / 100)
need  = 2(边框) + 1(标题) + question_lines(min 1, max 3)
        + options_rows + preview_rows(分栏时并列) + editor_rows(编辑态) + 1(底栏)
h     = clamp(need, min(10, area.height), max_h)
```

- `area.height < 12`：只画题干 + 选项 + 底栏；预览不画，底栏显示 `preview hidden (small pane)`。
- 宽度：面板宽度 = 内容宽（与输入框同宽，`MARGIN` 规则一致）。
- `inner.width >= 60`：左右分栏，选项列 `min 24`，预览列取剩余；否则上下排（预览在选项下方），整面板单一滚动区。

### 8.2 面板结构

```text
┌ ASK 1/2 ── 布局 ──────────────────────────────────────────┐
│ 侧栏用哪种布局？                                           │
│  (•) 双栏        │  ┌──────────┬──────────┐              │
│  ( ) 单栏        │  │ 项目 A    │  内容区   │              │
│  ( ) 上下堆叠    │  │ 项目 B    │  内容区   │              │
│  ( ) Other…      │  └──────────┴──────────┘              │
│                                                           │
│ ↑↓ move · space select · enter submit · tab next · esc    │
└───────────────────────────────────────────────────────────┘
```

- 边框复用 `ui/main_content/lx.rs::draw_box` 同款字符与样式。
- 标题：`ASK 1/2 · <header>`；多题时标签行可点击（命中矩形由 `layout()` 给出）。
- 勾选标记：单选 `(•)` / `( )`；多选 `[x]` / `[ ]`；高亮行用强调色 + `❯` 前缀。
- 「其他」行：未输入 `( ) Other…`；已输入 `(•) Other: <文本>`（过长按宽度截断）。
- 题干最多画 3 行，超出末尾 `…`；完整题干在历史卡片展开可见。
- 底部提示行：`esc cancel` 常驻；停止待命时改为 `esc again to cancel`（读 `lx.chat.stop_armed_at`）；内联 `hint` 优先显示在提示行左侧。
- 编辑态：选项列表下方出现编辑框（1–5 行，带上下框线），光标位置由 `ask::render` 返回（相对 `area`），与输入框光标同一套返回通道。
- 所有 chrome 文案为 ASCII（`ui/text/agent.rs` 常量），中文只来自模型提供的题干/选项。

### 8.3 预览渲染

- **原样**：不做 Markdown、不去空格、不折行合并；`\t` 按 4 列展开。
- 宽字符按 `unicode-width` 计列；宽字符后半列写 `CONTINUATION`（`ui/main_content/lx/grid.rs`），不出现半个字。
- 每行超宽截断并加 `…`；总行数超可视行时显示滚动条（复用 `widgets/scrollbar`），滚轮与 `PgUp/PgDn` 驱动 `preview_scroll`。
- 预览为空或选项无 `preview` 时，预览列画 `(no preview)`（dim）。
- 光标/选中高亮不进入预览区。

### 8.4 历史卡片

新增 `ToolDetailView::Ask(AskDetailView)`：

```rust
pub struct AskDetailView {
    pub questions: Vec<AskAnsweredView>,
}
pub struct AskAnsweredView {
    pub header: String,
    pub question: String,
    pub answers: Vec<String>,
}
```

来源两条路径共用：结算时 `detail_of(ToolExecutionResult::Ask)`；重载时 `ToolDetailView::from_result_json` 解析 `"tool":"ask"`。

卡片形态（`ui/main_content/lx/history/tool/ask.rs`）：

```text
■ Ask · 2 questions
  │ 布局 · 双栏
  └─ 主题 · 深色, 用青色
```

- 标题动词：`ui/text/agent.rs::VERB_TOOL_ASK = "Ask"`；行首方块走现有 `mark_of(status)`，**不使用 `?` 图标**。
- 运行中：`■ Ask · waiting for you`，第二行 `└─ 1/2 · 布局`（从 `arguments_json` 派生的题数/标题**不解析**：运行中只显示 `waiting for you`，不猜题数）。
- 折叠态：每题一行 `header · answers`（答案用 `, ` 连接）；超过 3 题不可能（上限 3）。
- 展开态：每题追加完整题干（换行缩进），**不显示 preview**。
- 失败/取消/中断：沿用现有终态配色与错误行；取消不写答案。
- `history::layout` 与 `render` 共用 `ask::rows()`，滚动条/置顶行不漂移。

### 8.5 标签页标记

- 含「挂起 ask 的窗格」的标签页标题追加 `?`（强调色）。
- `ui/tab_bar.rs` 的宽度计算、绘制与命中同步增加 2 列（空格 + 标记）；无 ask 时布局与现在完全一致。
- 标记随 `lx.ask` 清除自动消失；测试覆盖「有/无 ask 的宽度与位置不变性」。

---

## 9. 输入与事件接线

### 9.1 键盘

`src/input/mod.rs::route` **不改**（`pane_lx` 分支已产出 `LxInput`）。

`src/app/update/pane.rs::apply_lx_input` 在「后台任务页」之后、`rewind` 之前插入：

```rust
if state.pane_anywhere(pane_id).is_some_and(|p| p.lx.ask.is_some()) {
    apply_ask_input(state, pane_id, command);
    return;
}
```

`app/update/chat/ask.rs` 只做状态迁移与登记 `Effect`，不碰 IO。

### 9.2 粘贴

`src/event/input.rs::handle_paste_event` 在 lx 分支前插入：

- ask 激活且编辑态：文本插入 `AskState.custom[current]`；
- ask 激活但非编辑态：**吞掉**（绝不写进被隐藏的草稿，也不触发路径粘贴面板）。

### 9.3 鼠标

- `src/event/mouse/press.rs`：在 toast 处理之后、历史命中之前，对每个可见 lx 窗格检查 `ask_rect(pane_content_inner, lx)`；命中 → 聚焦该窗格（必要时切标签）→ `update::apply_ask_click` → `*dirty = true` → `return`（吞掉，不穿透）。
- `src/event/mouse/wheel.rs`：指针在 ask 面板内 → 按 `layout()` 判定预览/选项区 → `update::apply_ask_wheel` → 吞掉。
- 悬停高亮不做（避免与「当前高亮」混淆）。

### 9.4 尺寸同步

`src/app/update/pane.rs::resize_panes`：

```rust
pane.lx.resize(lx_cols, lx_rows);
if let Some(ask) = pane.lx.ask.as_mut() {
    let (w, h) = layout::lx_ask_editor_size(*rect);   // 新增：面板内编辑框宽度/可见行
    ask.resize_editor(w, h);
}
```

编辑缓冲的软换行宽度必须与渲染宽度同源。

---

## 10. 持久化与恢复

- **无数据库改动**：走现有 `agent_tool_call`（`begin_tool_call` / `mark_tool_running` / `finish_tool_call`）。
- `arguments_json`：完整提问，含 `preview`（有 24 KiB 上限）。
- `result_json`：4.4 的自包含结构（含 `header/question`，不含 `preview`）。
- 等待期间状态是 `running`；进程崩溃/强杀后由现有启动恢复逻辑标为 `interrupted`，与其它工具完全一致，**不新增恢复分支**。
- 上下文回放：现有机制回放 assistant tool call 参数 + tool result；`preview` 会随参数进入上下文，最坏 24 KiB，可接受（与 codex/Claude Code 一致）。

---

## 11. 提示词

`src/chat/prompt.rs`：

1. 工具清单从 7 个更新为 8 个，加入 `ask`。
2. 新增一段（对齐 codex `default.md`：先自己判断、能查到的别问、只问影响结果的问题、绝不用它要权限、不要用纯文本写选择题）：

```text
- `ask` blocks the turn until the user answers; the answer arrives as the tool result.
- Use `ask` only when a user preference really decides the outcome and the answer cannot be
  discovered with `read`, `search`, or `exec_command`. Prefer acting on a reasonable
  assumption over stopping to ask. Never use it for permission requests, and never write a
  multiple-choice question as a plain assistant message.
- Ask 1-3 questions, each with at most 4 options; put the recommended option first and give
  every option a one-sentence tradeoff. Do not add an "Other" option: free-form input is
  always available. Set `multi_select: true` only when several choices can coexist.
- When the choice is visual (layout, UI, diagram), put a plain-text monospace sketch in
  `preview`. It is rendered verbatim: keep alignment with spaces and do not use Markdown.
```

3. `PROMPT_VERSION`：`lx-2` → `lx-3`。
4. 现有测试 `prompt_is_lx_specific_and_tool_scoped` 会遍历 `TOOL_NAMES`，加入 `ask` 后必须能在提示里找到（上面的文案已包含）。
5. 新增测试：提示包含「blocks the turn」「permission」「1-3 questions」「preview」四个关键短语；`PROMPT_VERSION == "lx-3"`。

---

## 12. 关键代码改动一览（按文件）

| 文件 | 改动 |
|---|---|
| `src/chat/tools/ask.rs`（新） | 上限常量、`AskQuestion/AskOption/AnsweredQuestion`、`parse_arguments`、`answered_result`、单测 |
| `src/chat/tools/mod.rs` | `mod ask;` 导出；`execute_with` 对 `ask` 返回「必须由 agent loop 执行」 |
| `src/chat/tools/schema.rs` | 第 8 个 `ToolSpec`、参数 builder、文档「七」→「八」 |
| `src/chat/tools/types.rs` | `TOOL_NAMES: [&str; 8]`；`ToolExecutionResult::Ask` |
| `src/chat/tools/output.rs` | `format_for_model` / `to_result_json` 的 Ask 分支 |
| `src/chat/prompt.rs` | 工具清单 + ask 段 + `PROMPT_VERSION` |
| `src/chat/agent_loop/mod.rs` | `AgentHostRequest::AskUser`、`AgentHost::ask_user`、轮次分支 |
| `src/event/chat/host.rs` | `AgentHostEvent::AskUser` 与映射 |
| `src/event/chat/ask.rs`（新） | 请求落状态、取消清状态、作答回执 |
| `src/event/chat/mod.rs` | `ChatStreams.asks`、`cancel/finish`、效应泵 `AnswerAsk`、流结束清状态 |
| `src/app/effects.rs` | `Effect::AnswerAsk { key, call_id, answers }` |
| `src/app/state/lx.rs` | `pub ask: Option<AskState>` + 初始化 |
| `src/app/state/lx/ask.rs`（新） | `AskState`/`AskEditor`/状态迁移辅助 |
| `src/app/update/chat/ask.rs`（新） | 按键、鼠标、滚轮、提交、编辑命令分派 |
| `src/app/update/pane.rs` | ask 输入优先；`resize_panes` 同步编辑框宽度 |
| `src/app/state/lx/chat/tool_view.rs` | `ToolDetailView::Ask` + `from_result_json` |
| `src/app/update/chat/view.rs` | 重载路径无需显式改（走 `from_result_json`） |
| `src/event/chat/tool_calls.rs` | `detail_of` 增加 Ask 分支 |
| `src/event/input.rs` | 粘贴路由 |
| `src/event/mouse/press.rs` / `wheel.rs` | 点击/滚轮命中 |
| `src/ui/main_content/lx.rs` | 布局让位、`ask_rect` 导出、渲染调用、光标 |
| `src/ui/main_content/lx/ask.rs`（新） | 面板几何、绘制、命中、滚动 |
| `src/ui/main_content/lx/history/tool.rs` / `tool/ask.rs`（新） | 卡片 rows/draw |
| `src/ui/text/agent.rs` | `VERB_TOOL_ASK` 与面板/卡片/标签文案常量 |
| `src/ui/tab_bar.rs` | `?` 标记与宽度/命中 |

---

## 13. 边界与失败模式

| 场景 | 行为 |
|---|---|
| 参数非法（超限/未知字段/重复 id） | 不弹面板，`Failed{invalid_arguments}` 返回模型，模型可重试 |
| 用户取消（Esc 双击） | 工具行 `cancelled`，整轮 `interrupted`，模型拿不到答案 |
| 等待中窗格关闭 / 切会话 / `/new` | `ChatStreams.cancel` 唤醒 loop → `cancelled`，UI 清 ask |
| 等待中 app 崩溃 | 重启恢复把 `running` 标 `interrupted` |
| 等待中会话被另一实例接管 | 现有 session guard 机制；本轮最终 `interrupted`/失败，不写假结果 |
| 答案为空（未选就 Enter） | 内联提示，不提交 |
| 手写文本超 8 KiB | 编辑态拒绝继续输入并提示（或提交时截断并标记？→ **拒绝**，保持数据真实） |
| preview 超限 | 整次拒绝（4.3） |
| 0 选项题 | 只剩「其他」行；必须输入文本才能提交 |
| 单选下勾选 + 手写并存 | 互斥：后操作清掉前者 |
| 重复提交（鼠标连点 Submit） | 第一次提交即 `lx.ask = None`；后续点击落在输入框/历史上，天然幂等 |
| 迟到作答（面板已换） | `Effect::AnswerAsk.call_id` 与 `PendingAsk.call_id` 不匹配则丢弃 |
| 极小窗格 | 隐藏预览，保留题干/选项/底栏，底栏提示降级 |
| 宽字符/框线图 | 按 `unicode-width` 绘制，`CONTINUATION` 防半字 |
| `Esc` 单次误触 | 只进停止待命（3 秒），不丢答案 |
| 编辑态 `Esc` | 只退出编辑，文本保留 |

---

## 14. 测试计划（实现阶段逐项落地）

### 14.1 纯函数/单测

- `chat/tools/ask.rs`：合法解析（0/1/4 选项、1/3 题、多选、preview）；每条拒绝规则各一例；控制字符/tab/宽字符；上限边界（恰好等于/超 1）；`answered_result` 的顺序、去重、手写排最后。
- `schema`：`ask` spec 的 required/additionalProperties/描述非空；工具总数 8。
- `output`：`format_for_model` 输出与 codex 同构；`to_result_json` 自包含且不含 preview；结果体可被 `from_result_json` 解回同一 detail。
- `prompt`：关键短语、`PROMPT_VERSION`、`TOOL_NAMES` 全覆盖。

### 14.2 状态机（`app/state/lx/ask` + `app/update/chat/ask`）

- 单选选择互斥、多选切换、手写与勾选互斥（单选）。
- `Enter` 非末题前进、末题推 `Effect::AnswerAsk` 且清 `lx.ask`。
- 空答案不提交并置 hint。
- 编辑态 Enter 提交文本、Esc 只退出编辑、Tab 吞掉。
- Tab/←→ 保留各题状态；修改已答题后重提交覆盖 `collected`。
- 草稿不变量：进入/作答/取消全程 `Lx.input` 与 `cursor` 不变。

### 14.3 事件/loop（无网络）

- `RecordingHost` 增加 ask 支持：断言 `AskUser` 先于 `FinishTool`；答案传入后结果 JSON 正确。
- 取消：`ChatStreams.cancel` 唤醒 `ask_user`；loop 返回 `Interrupted` 且调用落 `cancelled`。
- 归属失效：`AskUser` 时 `active_assistant_id` 为 None → `Err(Cancelled)`，不写 `lx.ask`。
- 迟到作答：`call_id` 不匹配 → 丢弃且不 panic。

### 14.4 渲染/几何

- `ask::layout`：40 行/80 列、20 行/50 列、10 行极小窗格；70% 上限、最小 10 行、60 列分栏阈值。
- 绘制快照式断言：标题/题干折行/单选与多选标记/「其他」行/预览原样对齐/截断省略号/滚动条。
- 历史卡片：运行中、完成、取消、展开/折叠、宽字符、无 preview。
- `rows()` 与 `draw()` 行数一致（延续现有 `tool_geometry` 测试风格）。
- 鼠标命中矩形与绘制矩形逐格一致（含分栏与降级布局）。
- 标签页 `?` 标记：有/无 ask 时标签宽度与点击命中不漂移。

### 14.5 验收命令

```bash
cargo fmt
cargo clippy --all-targets -- -D warnings
cargo test
```

（按影响面至少跑：`cargo test ask`、`cargo test tools`、`cargo test event::chat`、`cargo test ui::main_content::lx`。）

交互与外观由用户 `cargo run` 实机确认（Agent 无法交互式验证 TUI）。

---

## 15. 风险

| 风险 | 影响 | 缓解 |
|---|---|---|
| loop 阻塞等待时取消路径漏一处 → 永久卡住 | 高 | 所有取消入口都走 `ChatStreams::cancel`；`finish` 兜底 drop；测试覆盖 Esc/关窗格/切会话 |
| 复用 `Lx` 作编辑缓冲被误用（chat 字段被写） | 中 | `AskEditor` 只暴露编辑 API；不实现 `DerefMut` |
| 面板几何与命中漂移 | 中 | 单一 `layout()` 供渲染/命中/滚轮；几何测试 |
| `preview` 撑爆上下文 | 中 | 24 KiB 总上限 + 单选项 4 KiB/40 行 |
| 标签宽度改动破坏现有命中 | 中 | 无 ask 时布局逐字段等于现状；快照测试 |
| 等待中切换标签后用户没注意 | 中 | 标签 `?` 标记（Q15） |
| 新增工具改变所有 transport 的工具列表 | 低 | 现有测试遍历 `TOOL_NAMES`；补 ask 分支即可 |

---

## 16. 验收标准

1. 模型调用 `ask` 后，lx 窗格输入框位置出现面板；模型直到用户提交前不再采样。
2. 单选/多选/自由输入/字符画预览全部可用；键位与鼠标契约符合 §7。
3. 答案以 §4.4 的结构回给模型；历史卡片正确留痕且不显示 preview。
4. `Esc` 双击取消整轮；编辑态 `Esc` 只退出编辑；切窗格保留、关窗格/切会话取消。
5. 草稿在任何路径下都不丢。
6. 空闲标签页收到 ask 时标题出现 `?`，切回后可直接作答。
7. 重启后未答完的 ask 显示 `interrupted`，不会永久 `running`。
8. `cargo fmt` / `cargo clippy -D warnings` / `cargo test` 全绿；用户实机确认外观与手感。
