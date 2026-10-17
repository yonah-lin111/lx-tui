# lx Agent 思考生命周期与文件工具：设计方案

> 状态：待用户审核。本文只定义方案；审核确认前不修改生产代码、数据库迁移或测试。
>
> 任务范围：lx Native Agent；参考 Codex agent loop 与补丁语义、OpenCode 的跨协议事件归一化和工具状态流。Agent 名称为 `lx`。不引入或复制 Codex core。

## 1. 背景与目标

当前 lx 只有一次模型流：`BeginChatTurn -> StartChatStream -> ChatDelta -> FinishChatTurn`。`src/chat/events.rs` 只表达文字、思考、用量和结束；适配器对部分原生工具事件会拒绝。`src/app/state/lx/chat.rs` 的 `reasoning_streaming` 将最新思考块是否仍在进行等同于整条 assistant 消息是否 `streaming`。因此思考结束、正文或工具开始之后，UI 仍可能把思考显示为进行中，直到整轮流结束。

目标是让 lx Native Agent 能在同一轮中反复执行“模型流 -> 工具 -> 将结果送回模型 -> 模型流”，提供 `read`、`search`、`edit`、`write` 四种文件能力，并按发生顺序显示、落库和恢复。思考块在自身结束时完成，不等待整个 assistant 回答结束。现有 OpenAI、Anthropic、Google、OpenAI-compatible 模型配置继续可用；兼容接口拒绝原生工具参数时向用户报告能力错误。

成功标准：

1. `lx` 可完成至少一次和多次工具调用的完整轮次；同一模型响应中的多个调用串行执行，顺序与模型给出的顺序一致。
2. 工具进入可见 loading/pending 状态并成功入库后，才开始文件操作；状态再进入 running，最终记录 completed/failed/cancelled。入库失败时不得执行文件操作。
3. 思考结束事件、后续正文/工具边界、流终态均能把对应思考块完成；正文或工具仍在进行时，先前思考不再显示进行中。
4. `/resume` 后能按顺序显示已落库的步骤，下一轮模型上下文能重建已完成的工具调用及结果。异常退出遗留调用显示中断，不自动重做文件操作。
5. 文件工具默认相对路径基于当前窗格所属工作区 cwd；绝对路径可访问整个文件系统。第一版不限制路径、不做逐次确认；沙盒是后续独立任务。
6. 工具视觉遵循现有 UI Gallery 的 agent 样例；`ui/` 仍只读状态，所有文件与数据库 IO 在事件/服务层。

## 2. 已确认边界

| 决策 | 本期口径 |
| --- | --- |
| Agent loop | 在 lx 内实现 Codex 风格的多步循环，复用其行为设计；不复制、链接 Codex core。 |
| 协议 | OpenAI Responses、OpenAI Chat Completions/compatible、Anthropic Messages、Google Gemini 都须支持工具调用。具体请求字段和续传块遵循各自协议。 |
| 工具 | `read`、`search`、`edit`、`write`。`edit` 使用 Codex 风格 patch，包含新增、修改、删除、移动。`write` 创建或覆盖整个文本文件。 |
| 权限 | 所有路径可访问，无工作区边界、确认弹窗和临时沙盒。文件系统自身权限错误正常返回工具失败。 |
| 顺序 | 同一响应内调用逐个执行；前一调用的结果可被后一调用观察。全部结果形成下一次模型请求上下文。 |
| 持久化 | 先记录 loading/pending，后执行；结果落库。重启时 pending/running 标记中断，不自动重跑。 |
| 消息 | 每个用户轮次仍只有一条 assistant 消息，内部步骤按真实先后顺序展示和回放。 |
| 思考 | 优先采用协议明确结束事件；缺少事件时，下一段正文或工具开始即完成；流终态兜底。 |
| 外观 | 参考 `src/ui/overlay/ui_gallery/agent.rs` 的 Read/Search/Edit/Write/Thinking 样例。 |

## 3. 现有实现与参考依据

### 3.1 lx 的核心数据流

`src/app/update/chat/mod.rs::try_send` 先入队 `BeginChatTurn`。数据库 `begin_turn` 在一个事务内建 user 和 streaming assistant。`src/event/chat.rs::start_stream` 从状态装配 `ChatRequest`，调用 `chat::client::run_stream`；网络增量回投事件循环，`on_stream_delta` 修改内存状态，checkpoint 周期性落盘；`on_stream_finished` 入队 `FinishChatTurn`。`src/chat/context/mod.rs::assemble` 仅识别完整的 user/assistant 对，历史只含正文；工具表目前不参与会话加载和上下文组装。

v7 已有 `agent_tool_call`，含 `call_id`、`name`、`arguments_json`、status、结果和时间，但没有业务 CRUD、回放加载或与文字/思考统一排序的字段。`agent_message_part` 只接受 `text` 和 `reasoning`；`ended_at_ms` 当前记录最后一次增量的时间，不表示思考已完成。启动恢复只将 `agent_message.status='streaming'` 标为 interrupted。

### 3.2 参考代码的适用部分

| 来源 | 采用的思想 | 不照搬的内容 |
| --- | --- | --- |
| Codex `codex-rs/core/src/session/turn.rs` | 模型响应后调度工具、保留调用/结果、再发下一次采样；以最终无工具响应结束轮次。 | 整套 Session/TurnContext/工具路由、审批、沙盒、MCP、压缩与插件。Codex core 当前只接受 Responses wire API，不能直接接管 lx 的四种传输。 |
| Codex `codex-rs/apply-patch` | 带上下文的 `*** Begin Patch` 语法，以及 Add/Update/Delete/Move 行为与失败说明。 | 完整 CLI/审批/沙盒运行时。若作为独立 crate 可用，实施时评估直接依赖其补丁解析/应用库；否则在 lx 内实现必要兼容子集并用固定 fixture 验证，不复制整个 core。 |
| OpenCode `packages/core/src/session/message-updater.ts` | reasoning 有独立开始/结束，工具经历输入中、running、completed/failed 状态。 | TypeScript/Effect 运行时和会话服务。 |
| OpenCode `packages/core/src/session/runner/publish-llm-event.ts` | 不同供应商事件归一化，按 call ID 聚合参数片段，结束后才执行。 | 全部插件和模型能力矩阵。 |
| lx UI Gallery `src/ui/overlay/ui_gallery/agent.rs` | 图标、动词、路径、行范围、diff 数字的样式与行结构。 | Gallery 假数据；真实 UI 只读聊天状态。 |

协议字段在实施前以各供应商文档与现有适配器核实；本次已通过 Context7 查阅 Anthropic streaming/tool use、Google function calling、OpenAI streaming tool_calls。Google 文档同时覆盖旧 generateContent 与较新的 Interactions API；本任务沿用 lx 当前 generateContent 端点，不无故迁移接口。

## 4. 总体结构

```text
app/update (纯状态/Effect)       db/service (事务与加载)
           ^                           ^
           |                           |
event/chat (单事件循环、启动/取消、按 key 归属、状态回投)
           |                           |
           v                           v
chat/agent_loop (异步编排) ----> chat/tools (后台文件 IO)
           |
           v
chat/transport/{openai,responses,anthropic,google}
           |
           v
统一 ChatDelta/StepOutcome (纯数据)

ui/main_content/lx/history (只读步骤状态)
```

模块方向保持 `app -> chat` 仅依赖纯事件类型，`chat` 不读取 AppState；`event` 拿到冻结的模型、工作区 cwd、历史快照后启动异步 loop。文件工具在 `chat/tools` 或按现有目录边界放置的独立文件操作模块执行；它不持有 TUI 状态。阻塞文件 IO 使用 `tokio::task::spawn_blocking`，避免事件循环停顿。`db/` 独占 SQLite，`ui/` 不查库。

一轮逻辑：

```text
BeginChatTurn DB ack
  -> 建立 agent loop，冻结 provider/model/variant/cwd
  -> 发送模型请求，流式投递 reasoning/text/tool input 事件
  -> 模型响应要求工具：封闭当前 reasoning/text 块
  -> 对每个 call 按顺序：
       完整参数校验 -> DB pending ack -> UI loading
       -> DB running ack -> 后台执行 -> DB completed/failed ack -> UI 结果
  -> 将本次模型的全部 call/result 加到请求历史
  -> 再次发送模型请求
  -> 无工具的完成响应：关闭内容块 -> FinishChatTurn
```

UI loading 必须在开始文件 IO 前成为可观察状态。数据库 pending ack 是执行门槛；状态更新由事件循环做。网络任务不得绕过事件循环直接修改 AppState。`running` 可紧接 pending，但至少让 pending 事件被处理并请求一帧渲染；不能靠固定 sleep 保证可见性。若 TUI 因最小帧间隔合并两次状态，UI 仍应至少显示调用中的转轮直到结果返回。

## 5. 统一事件与状态机

### 5.1 模型流事件

扩展 `chat::events` 的纯数据事件，不把供应商 JSON 传播到 `app`：

| 事件 | 必要字段 | 语义 |
| --- | --- | --- |
| `ReasoningStarted` | source/item ID，时间 | 开新思考块；仅协议有独立起始时显式发出，普通 delta 可隐式起始。 |
| `Part(Reasoning/Text)` | 文本增量、已有 provider 元数据 | 增量追加到当前块；不同 kind 新建块。 |
| `ReasoningEnded` | source/item ID，最终元数据 | 关闭对应思考块，完成时间只写一次。 |
| `ToolInputStarted` | call ID、name、序号 | 建立正在接收参数的工具步骤；此时不能执行。 |
| `ToolInputDelta` | call ID、参数片段 | 以 ID/序号聚合；可用于 loading 描述，须限制缓冲大小。 |
| `ToolInputEnded` | call ID、完整 JSON/校验结果 | 参数接收完成，允许进入持久化和调度。 |
| `Usage` | 当前响应用量 | 多次模型请求的用量累加；未知值保持 None。 |
| `ResponseFinished` | stop reason | 结束的是一次模型响应；`tool_calls/tool_use` 表示要继续 loop，不等于用户轮次完成。 |

此表是设计上的统一语义，不强制一个枚举承载所有阶段。实际 Rust 类型应根据现有 `ChatDelta`、`StreamOutcome` 保持最小变更。call ID 用供应商原生 ID；若 Google 无 ID，则以本轮响应编号、part 序号生成稳定内部 ID，并在该轮回传时维持映射。任何未知工具名与无效 JSON 都记为失败工具结果并回传模型，不能执行任意名称或把未经校验的内容当 shell 命令。

### 5.2 思考生命周期

每个 reasoning 块状态为 `streaming -> complete | interrupted`。单独记录 `completed_at_ms`（或等价显式完成字段）；`ended_at_ms` 保留原有“最后一段增量”含义。状态转换规则：

1. 首个 reasoning delta 建块，设置 `started_at_ms`；持续增量只更新内容和最后增量时间。
2. 协议明确 reasoning end 时立即设置完成时间；重复 end 幂等。
3. 在仍开放的 reasoning 后开始正文、工具输入或下一个 reasoning 块时，先关闭旧块；事件先后顺序是唯一依据。
4. 响应结束或取消时关闭仍开放的块；成功为 complete，失败/取消为 interrupted。若网络在 end 事件前断开，UI 保留已生成文本，且不将其显示成永远进行中。
5. `reasoning_streaming` 只看该块是否未完成且属于当前在途请求，不再以 assistant 整体 `streaming` 作为唯一条件。用户显式展开/折叠状态继续覆盖默认值。
6. 思考签名等 provider metadata 仍绑定到原块；跨 provider 的旧思考原文不注入新请求，避免泄露或签名错误。

### 5.3 工具状态

`input_streaming -> pending -> running -> completed | failed | cancelled | interrupted` 是 UI 语义；DB 可沿用 v7 的 `pending/running/completed/failed/cancelled`，但需新增 `interrupted` 终态约束，或将异常恢复映射成明确的 `cancelled` 并保留独立错误码。推荐新增 `interrupted`，语义与“用户取消”不同。`input_streaming` 只在内存，尚无完整参数和副作用；pending 必须已入库。未成功落库的输入阶段在终态前只形成错误记录或终止本轮，不触发文件操作。

每次转换带 `(RequestKey, assistant_id, call_id)`；过期网络事件、重复结果和错窗格结果均忽略。DB 以 `(session_id, call_id)` 唯一约束保证幂等。执行失败是工具失败结果，通常继续把失败回传模型，由模型解释或修正；DB 无法写入则停止本轮，避免“已执行但无调用记录”。用户 `Esc` 停止时停止后续工具与采样；已开始的阻塞写入无法保证中途回滚，完成后仍必须记录真实结果，不能谎报取消。

## 6. 数据模型与迁移

当前 `src/db/mod.rs` 的 `SCHEMA_VERSION` 已是 8；本期需要 **v9** 事务迁移，不覆写 v7/v8 历史。v8 只增加了 part 起止时间字段，不能把本期字段并入旧迁移。

### 6.1 统一顺序

保持每轮一条 assistant 消息。`agent_message_part.position` 从“仅内容块的连续下标”扩展为“assistant 时间线位置”；`agent_tool_call` 增加 `position INTEGER NOT NULL`，与 part 的 position 在同一条 message 内按事件发生顺序分配。两表分别有 `(message_id, position)` 唯一约束；跨表位置冲突由 DB 服务在事务内分配和检查。老消息无工具，原 part position 不变，天然可回放。app 侧只暴露一个按 position 排序的步骤列表，不再分别维护两个可能漂移的 UI 顺序。DB 查询可分别读取两表，但投影到 AppState 时必须合并。写 checkpoint/finish 时必须保留已分配位置，不能再按 `enumerate()` 将 part 压回连续下标；已有 `replace_parts` 流程须随之调整。工具行增添 name/arguments/status/result 等详情，不再把工具结果复制为普通 text part。

若实施时发现现有 `replace_parts` 大范围依赖连续位置，可改用独立时间线表，需在文档更新并先获得审阅；不得用毫秒时间戳排序，因为并发事件和快速连续事件可能相同毫秒。工具位置在 `ToolInputStarted` 时预留于内存，在 pending 入库时确定，确保其展示位于后续正文之前。

### 6.2 思考完成字段

`agent_message_part` 增 `completed_at_ms`，非 reasoning 行保持 NULL。旧库历史中 assistant 已终态的 reasoning 按原 `ended_at_ms` 或消息终态时间回填完成时间；旧库 streaming 由现有恢复过程转 interrupted，同时封闭其 reasoning。当前内存 `ChatPartView` 与 DB `ChatPart` 同步承载该字段。历史迁移不改变原内容、provider metadata、顺序和持续时间口径。

### 6.3 工具 CRUD 与恢复

DB API 至少包含 `begin_tool_call`、`mark_tool_running`、`finish_tool_call`、`load_tool_calls_for_session`、`recover_interrupted_tool_calls`。所有写入需按 workspace、session、assistant、call ID 校验归属；返回可区分的失败，不能静默忽略。`begin_tool_call` 的 pending 行同时写入规范化参数、位置、创建时间。`finish_tool_call` 原子写结果/错误、终态和结束时间；重复同结果终态幂等，不同结果的重复终态拒绝。`load_chat_session` 载入 parts 与 tool calls，并按位置合并为内存步骤视图；避免每条消息逐条查询。

启动恢复在同一受控服务路径内将 pending/running 标 interrupted，消息流终态仍走现有恢复。恢复动作幂等，不会重新调度工具。中断工具的结果字段不可被填成成功；下一轮可以告诉模型“此前调用中断，结果未知”，但不会构造假的成功调用/结果对。用户再次请求时模型可自行读取文件确认现状。

### 6.4 数据量与敏感内容

`arguments_json` 和 `result_json` 必须有限长，既防止无限 SSE 参数耗尽内存，也避免聊天库膨胀。建议单次参数上限 1 MiB、单次工具回传模型内容上限 64 KiB，超限给确定错误/截断标志；具体值实施时按已有 `MAX_MESSAGE_BYTES` 和供应商上下文预算核对。`read/search` 结果与 `write/edit` 内容可能包含敏感文件，既然用户确认全路径可访问，数据库会存必要的调用参数和工具结果；UI 只展示摘要，不在错误码或 tracing 中打印全文、密钥和大块内容。工具输出截断必须显式告诉模型，不能悄悄省略。

## 7. Agent loop 与上下文

### 7.1 循环控制

后台 loop 持有不可变的冻结模型配置、workspace cwd、会话上下文和取消信号。它一次只向事件循环投递纯数据；事件循环在 DB ack 后回复可继续执行的信号。`ResponseFinished` 的 stop reason 明确要求工具时，先检查所有工具输入已经闭合且参数可解析，然后逐个执行；每个工具得到结果后加入同一轮请求上下文。所有工具完成后才发下一次模型请求。最终无工具、正常 stop 才结束用户轮次。

设置最大模型步骤数与工具调用数作为资源保护，推荐每轮最多 24 次模型响应、64 个工具调用；到上限时停止执行并生成失败终态及可见原因。它是运行时保护，不是对路径的限制。取消发生在等待网络/DB ack/文件任务之间时都要停止后续工作；文件任务已完成则按真实结果结算后退出。流失败自动重试规则沿用现状，但只有在该响应尚未产出内容/工具输入、且没有任何本响应的文件副作用时才可重试，绝不能重放已执行工具。

### 7.2 `lx` 指令与工具定义

新增稳定的 agent 身份提示，名称为 `lx`；只描述当前文件工具、相对路径基准、简明执行方式与结果回报。可参考 Codex harness 对读文件、搜索、修改和工具输出的描述，但不嵌入 Codex 品牌或其整份系统指令。提示文本作为 chat 层请求装配数据，经四类 transport 转成对应 system/developer 指令。模型工具 schema 固定英文稳定名称 `read`、`search`、`edit`、`write`；输入错误提供清楚的英文工具反馈，用户可见固定文案集中在 `src/ui/text.rs`。

### 7.3 上下文回放

现有 `assemble` 只保留完整轮次并估算正文 token，需改成以完整 user/assistant 轮次为单位裁剪，同时把该轮有序 text/reasoning/tool call/result 放进请求表示。历史里已完成且有结果的调用必须以供应商对应的 assistant tool call + tool result 成对编码；失败调用可将错误作为 tool result；interrupted/pending/running 不伪造结果。若历史因预算删掉一轮，连同该轮所有工具调用/结果一起删掉，禁止 orphan result。当前轮在工具之间的多次采样必须保留本轮已经发生的 call/result；不能重新从 DB 的“仅已完成轮次”列表组装而丢失正在进行的步骤。跨 provider 恢复时如目标协议无法原样表示旧 provider metadata，只使用规范化的工具 ID/name/input/result 和可见正文，省略不兼容的 reasoning 签名。

工具结果回放需要考虑单次模型请求的上限；token 估算须包含工具参数和返回内容，`read/search` 的截断标志与实际截断内容一同回放。旧会话没有工具行时请求结果应与原有行为等价。

## 8. 四类协议适配

所有协议保持当前 endpoint、认证、variant 参数和文字/思考流式行为；只扩展工具 schema、工具事件解析与结果续传。必须以真实协议 fixture 驱动测试，不靠字符串包含判断。

| 传输 | 发工具定义 | 收工具输入 | 回传结果及继续 | 关键边界 |
| --- | --- | --- | --- | --- |
| OpenAI Responses | `tools` function definitions | response item / function call arguments 增量与完成事件；按 item/call ID 聚合 | `function_call_output` 与对应 `call_id`，再发后续请求 | 完成事件与 response 完成分开；保留输出项顺序。 |
| OpenAI Chat Completions / compatible | `tools` + `tool_choice` | `choices[].delta.tool_calls[]` 按 index 合并 ID、name、arguments | assistant 的 `tool_calls` 后接 role=`tool`、`tool_call_id` | `finish_reason=tool_calls` 是工具阶段，`[DONE]` 是本次响应完成；无工具支持时明确报错。 |
| Anthropic Messages | `tools` 的 input_schema | `content_block_start(tool_use)` + `input_json_delta` + `content_block_stop` | assistant tool_use block 后接 user tool_result block | 按 block index 绑定 ID；thinking signature 和 tool_use 顺序不能丢。 |
| Google Gemini | `tools.functionDeclarations` | candidate parts 中 `functionCall`；流式重复快照要去重 | model functionCall part 后接 functionResponse part | 保留 thought signature/parts 顺序；现有 generateContent 流端点继续使用。 |

协议明确 reasoning end 时发 `ReasoningEnded`；没有则靠后续非 reasoning 事件封闭。未知 stop reason、半截参数 JSON、同 ID 冲突、EOF 未完成、供应商返回多个候选等情形不能执行不确定的文件操作。兼容接口对 `tools` 字段返回 4xx 时，失败原因要表述为“该接口不支持工具调用”或原协议错误，不把请求悄悄降级为纯文字。

## 9. 文件工具契约

所有路径按如下规则解析：绝对路径原样使用；相对路径以发送时冻结的工作区 cwd 解析，后续 shell `cd`、切换焦点或工作区不会改变本轮基准。无 cwd 时本轮失败并提示，不能退回进程 cwd 随机执行。允许符号链接指向任意位置，访问范围由 OS 权限决定。读取、搜索、补丁和写入都在后台执行。

| 工具 | 输入 | 输出/失败语义 |
| --- | --- | --- |
| `read` | `path`，可选 `offset`/`limit`（行） | 文件内容带行号和截断标志；目录可列出条目。不存在、不可读、二进制不支持、偏移越界返回工具错误。默认和最大返回量明确限制。 |
| `search` | `pattern`，可选 `path`（默认 `.`）、`include`、`literal`、`case_sensitive`、`limit` | 默认从工作区搜索，返回路径/行号/匹配行，尊重忽略规则；显式路径可到任意位置。超时、非法正则和截断明确区分。优先复用项目已有 `ignore`，若需外部 `rg` 则作为进程工具执行且不依赖 shell 插值。 |
| `edit` | Codex 风格完整 patch 文本 | 解析后按 Add/Update/Delete/Move 执行；上下文不匹配就报错，不能猜测替换。执行前校验 patch 格式与目标路径；返回每文件变更及行数统计。多文件补丁若底层不是原子操作，必须保留已实际提交的变更信息并在结果中说明，不谎报整体回滚。 |
| `write` | `path`、完整 `content` | 创建或覆盖 UTF-8 文本；父目录按明确工具契约创建，文件权限错误返回失败；返回创建/覆盖标记和差异统计。不能静默把 append 当 overwrite。 |

写入应尽可能按同目录临时文件 + rename 原子替换，兼顾已有权限与换行/BOM；实现时先核对 `platform/` 的现有文件原子保存能力及跨平台行为。`edit` 的 Delete/Move 是明确授权的工具行为，不借用 `git` 命令，也不自动改变 Git 索引、分支、暂存区或提交。目录搜索限定默认遍历规模，避免误从 `/` 无限遍历；显式指定广域路径时达到上限返回截断而非无限执行。文件操作错误结果须含路径和原因，但不得把完整敏感文件内容写到日志。

## 10. UI 与交互

历史列表以同一条 assistant 消息的有序步骤为输入。思考标题沿现有折叠逻辑，在 complete 时切换完成态；工具 loading/running 使用与 Gallery 相同的图标、动词、路径色和转轮语义。已完成 `read` 头部给行范围；`search` 不直接铺开命中正文；`edit/write` 头部显示新增/删除行数；失败状态用现有 agent error 色，并显示精简原因。工具结果全文只进入模型上下文/持久层，不直接在 TUI 历史中铺成巨大内容。

渲染与命中共用 `history::layout/view` 的步骤行数计算，避免滚动条、置顶用户消息和点击思考位置失配。窄窗格先截断路径，再隐藏次要计数，图标与状态仍可辨；超长无空格文件名按单元格列宽裁剪。`ui/` 不执行文件/DB/network IO，不改变思考或工具状态。`src/ui/text.rs` 维护全部新用户可见文案。TUI 实际颜色、间距和交互由用户运行 `cargo run` 人工验收。

## 11. 失败与一致性矩阵

| 场景 | 要求结果 |
| --- | --- |
| 参数流未闭合或 JSON 无效 | 不执行文件 IO；记录工具失败并回传错误，若连 call ID/name 都不可信则协议失败结束轮次。 |
| pending 入库失败 | 不执行文件 IO；轮次失败、内存 UI 给错误。 |
| running 入库失败 | 不执行文件 IO；pending 在恢复时可转 interrupted。 |
| 文件操作失败 | 记录 failed 和错误结果；正常情况下继续让模型处理。 |
| 文件操作成功但结果入库失败 | 不重试写入；停止本轮并显示“结果持久化失败，文件状态需核查”。重启后调用按 interrupted 处理，不自动重跑。 |
| 结果入库成功但 UI ack 丢失 | DB 为真相，幂等加载恢复；防止重复执行同 call ID。 |
| `Esc` 中断等待中的工具 | 尚未执行则 cancelled；已开始的操作结束后记录真实结果，停止后续调用和模型采样。 |
| 网络断开且已执行工具 | 不自动重试该模型响应；保留工具结果，整轮失败/中断供用户续问。 |
| 两次同 ID、乱序或旧 generation 事件 | 按 request key 和 call ID 去重/拒绝；不产生第二次文件副作用。 |
| 应用异常退出 | 启动时将 pending/running 工具标 interrupted；不自动执行。 |
| 供应商不支持 tools | 显示可理解的能力错误，不降级为纯聊天；旧会话仍可读取。 |

## 12. 验证与交付

实现前先补当前思考状态延迟完成的复现测试，以及工具输入/DB ack/取消的状态测试。协议使用固定 SSE/JSON fixture 覆盖四类传输的单工具、多工具、参数跨帧、正文/思考交错、错误与 EOF；文件工具用临时目录验证 patch 的四类动作、路径解析、串行可见性、权限和失败后实际变更；DB 迁移测试覆盖旧库升级、重复打开、工具状态机、重启恢复、顺序与 `/resume` 回放；UI 用 buffer/geometry 测试覆盖 loading、完成、错误、长路径和极窄视口。

按受影响范围运行精确测试；完成后遵循项目规范执行 `cargo fmt`、`cargo clippy --all-targets -- -D warnings`、`cargo test`。本设计不修改 `src/tui/` 恢复路径；若实现触及它，必须额外保留并执行 `tui::tests` 的恢复顺序回归。真实供应商和 TUI 外观由用户运行 `cargo run` 验收，自动测试不得声称完成了这两项人工验证。

## 13. 风险与后续

1. **无沙盒的全路径写入**：这是已确认的首版行为。路径错误可造成工作区外修改；UI 必须显示绝对目标路径和真实变更摘要。后续沙盒需单独设计，不能在本期偷偷加入权限限制。
2. **跨协议工具历史**：工具 call/result 在模型供应商间转换可能损失私有 metadata；规范化字段和成对回放是最低保证。Anthropic thinking signature、Google thought signature 仅在本协议可安全继续时携带。
3. **多文件 patch 非原子**：须报告部分成功，不假设可回滚。工具结果和 DB 记录都反映已发生的文件变化。
4. **模型循环资源消耗**：步骤/调用上限、输出截断和搜索超时按第 7/9 节实施；达到上限可见且可恢复。
5. **现有会话兼容**：旧 v7 聊天没有工具时间线，迁移保持顺序和内容；回放不能丢掉旧完整轮次。

审核通过后依 `task.md` 执行。若实施调查发现与本设计相矛盾的协议事实或数据约束，先更新这两份文档并说明差异，再继续相应改动。

## 14. 关键类型与不变量

下列代码是**拟采用的结构示意**，不是已实现代码。字段名可在实施时按 Rust 现有风格微调，但不变量不可改变。当前真实类型是 `ChatMessageView { parts: Vec<ChatPartView> }`、`ChatTurn { role, text, reasoning_metadata }`；本期需显式迁移其调用点。

### 14.1 AppState 只有一条步骤时间线

```rust
#[derive(Debug, Clone)]
pub enum AssistantStepView {
    Part { position: i64, part: ChatPartView },
    Tool { position: i64, call: ToolCallView },
}

pub struct ChatMessageView {
    // 既有 id/role/status/turn_index/model/usage 字段保留。
    pub steps: Vec<AssistantStepView>,
}

pub struct ChatPartView {
    pub kind: PartKind,
    pub content: String,
    pub provider_metadata_json: Option<String>,
    pub started_at_ms: Option<i64>,
    pub ended_at_ms: Option<i64>,       // 最后一次增量，不是“已完成”
    pub completed_at_ms: Option<i64>,   // reasoning 的独立完成时刻
}

pub struct ToolCallView {
    pub call_id: String,
    pub name: String,
    pub arguments_json: String,
    pub status: ToolStatus,
    pub summary: Option<ToolSummary>, // UI 摘要，不装入完整文件内容
    pub error: Option<String>,
}
```

`AssistantStepView` 是 UI、鼠标命中、历史滚动和当前轮上下文的共同顺序来源；DB 的 part/tool 两表是持久化表示。任何一方都不能在 UI 内按时间戳再次排序。对于内存中的 streaming 消息，插入新步骤时 position = 已有最大 position + 1；同 kind 的相邻文本增量只修改原 part，保持 position 不变。DB 服务在事务中再验证 position 无冲突；失败即中止该工具，不“修正”成另一位置。`PartKind` 仍只有 Text/Reasoning；工具是独立步骤，不能伪装成文字 part。

### 14.2 请求与结果必须保留结构

```rust
pub enum ModelItem {
    UserText(String),
    AssistantText(String),
    AssistantToolCall { call_id: String, name: String, input: serde_json::Value },
    ToolResult { call_id: String, content: String, is_error: bool },
}

pub struct ModelStepResult {
    pub items: Vec<ModelItem>,           // 供应商产生的顺序
    pub calls: Vec<ReadyToolCall>,        // 只有完整且已校验参数的调用
    pub usage: Usage,
    pub stop: ModelStepStop,             // Final / ToolCalls / Failed / Cancelled
}
```

现有 `ChatTurn { role, text }` 不足以表达 tool call/result，不能把工具结果拼接进一段 assistant 文本充数。若历史裁剪保留某次工具调用，必须同时保留其结果；如果失败工具结果确已写库，它仍作为 `is_error=true` 的成对结果回放。正在接收参数的 call 不在 `ready calls` 内，不得触发文件 IO。

### 14.3 思考完成的关键判断

```rust
fn close_open_reasoning(message: &mut ChatMessageView, now_ms: i64) {
    if let Some(AssistantStepView::Part { part, .. }) = message.steps.last_mut()
        && part.kind == PartKind::Reasoning
        && part.completed_at_ms.is_none()
    {
        part.completed_at_ms = Some(now_ms);
    }
}

fn reasoning_streaming(message: &ChatMessageView, part_index: usize) -> bool {
    matches!(message.steps.get(part_index),
        Some(AssistantStepView::Part { part, .. })
            if part.kind == PartKind::Reasoning
                && part.completed_at_ms.is_none()
                && message.status == "streaming")
}
```

实际实现需使用现有索引/ID 类型，且允许协议显式 end 关闭**对应**的 reasoning 块，而非无条件只关闭末尾块。示例只说明“完成字段”和“判断条件”的区别；后续 tool/text 事件到达前调用 close，不能等 `on_stream_finished`。若同一帧包含 reasoning end 与 tool start，按帧内顺序处理，完成时间只记录一次。

## 15. DB 迁移与执行握手的关键代码

### 15.1 v8 -> v9 迁移草案

v8 的 `agent_tool_call.status` CHECK 不含 interrupted，SQLite 不能直接修改 CHECK；因此需在 `Database::migrate` 的同一事务中重建该表。以下是重要步骤，建表时须完整复制 v8 的所有列、外键和索引，并在测试里比较数据：

```sql
ALTER TABLE agent_message_part ADD COLUMN completed_at_ms INTEGER;

-- 先给旧工具行分配非冲突 position：同 message 先放既有 part，
-- 旧工具按 started_at_ms、id 排在后面。该回填只作用于老库。
CREATE TABLE agent_tool_call_v9 (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    external_id TEXT NOT NULL UNIQUE,
    session_id TEXT NOT NULL REFERENCES agent_session(external_id) ON DELETE CASCADE,
    message_id TEXT NOT NULL REFERENCES agent_message(external_id) ON DELETE CASCADE,
    call_id TEXT NOT NULL,
    namespace TEXT,
    name TEXT NOT NULL,
    arguments_json TEXT NOT NULL DEFAULT '{}',
    position INTEGER NOT NULL CHECK (position >= 0),
    status TEXT NOT NULL CHECK
        (status IN ('pending','running','completed','failed','cancelled','interrupted')),
    result_json TEXT,
    error_text TEXT,
    started_at_ms INTEGER NOT NULL,
    completed_at_ms INTEGER,
    duration_ms INTEGER CHECK (duration_ms >= 0),
    UNIQUE(session_id, call_id),
    UNIQUE(message_id, position)
);
```

`INSERT ... SELECT` 回填时按旧行主键稳定排序，position 从该 message 的 `MAX(agent_message_part.position)+1` 起递增。先写完、核对行数与外键，再删除旧表、改名新表、重建索引，最后更新 `user_version=9` 并提交。旧表若存在 pending/running，迁移只搬运状态；启动恢复随后标 interrupted。不能在迁移中执行文件操作。对已终态消息的旧 reasoning part，以 `COALESCE(ended_at_ms, agent_message.completed_at_ms)` 回填 `completed_at_ms`；还在 streaming 的 part 由恢复路径补齐。

工具位置的事务分配伪 SQL：

```sql
SELECT 1 FROM agent_message_part WHERE message_id = ?1 AND position = ?2
UNION ALL
SELECT 1 FROM agent_tool_call WHERE message_id = ?1 AND position = ?2;
-- 有行即拒绝；无行才在同一事务 INSERT pending。
```

DB API 不得向调用方返回一个布尔值掩盖失败原因。`begin_tool_call` 返回 `Result<ToolCallTicket, DbError>`；ticket 只在事务 commit 后发放。`finish_tool_call` 用 `WHERE status='running'` 条件更新，并在受影响行数为零时区分“相同结果重复 ack”和“非法状态转移”。

### 15.2 先展示 loading，再调用文件工具

`chat/agent_loop` 通过 channel 请求事件层处理持久化，不直接持有 `Database` 或 `AppState`。示意消息：

```rust
enum AgentHostRequest {
    BeginTool {
        key: RequestKey,
        assistant_id: String,
        call: ReadyToolCall,
        reply: tokio::sync::oneshot::Sender<Result<ToolCallTicket, ToolStartError>>,
    },
    FinishTool {
        key: RequestKey,
        call_id: String,
        result: ToolExecutionResult,
        reply: tokio::sync::oneshot::Sender<Result<(), ToolFinishError>>,
    },
}
```

事件循环收到 BeginTool 后，先执行 DB pending 事务；成功才把 pending 加入 AppState，安排下一帧，并回复 ticket。若目标窗格当前可见，回复应发生在该帧绘制之后，确保用户先看到 loading；窗格不可见时无需等待不可见的一帧，但 pending 状态仍已落库，切回时能看到。紧接着转 running 并执行文件工具。渲染 ack 由事件层发送，`ui/` 不参与状态写入或 IO。取消/关闭窗格可能使等待中的 oneshot 断开，此时不得执行工具。**不得用 sleep(100ms) 假装 loading 已显示。**

文件操作结束后，先写真实 completed/failed 结果，再通知 UI；若结果落库失败，不能给模型一个“成功”工具结果，更不能重新运行 write/edit。事件层要把此轮标失败并显示“文件状态需核查”。

### 15.3 串行 loop 的关键分支

```rust
async fn run_agent_turn(mut context: ModelContext, host: AgentHost) -> TurnOutcome {
    for step in 0..MAX_MODEL_STEPS {
        let response = match stream_one_model_response(&context, &host).await {
            Ok(value) => value,
            Err(error) => return TurnOutcome::Failed(error),
        };
        context.append_model_items(response.items);
        if response.calls.is_empty() {
            return match response.stop {
                ModelStepStop::Final => TurnOutcome::Complete,
                other => TurnOutcome::Failed(unexpected_stop(other)),
            };
        }
        if step + 1 == MAX_MODEL_STEPS { return TurnOutcome::Failed(step_limit()); }
        for call in response.calls {
            if host.cancelled() { return TurnOutcome::Interrupted; }
            let ticket = match host.begin_tool(call.clone()).await {
                Ok(ticket) => ticket,
                Err(error) => return TurnOutcome::Failed(error),
            };
            let result = execute_file_tool(ticket, call).await;
            if let Err(error) = host.finish_tool(&result).await {
                return TurnOutcome::Failed(error);
            }
            context.append_tool_result(result);
        }
    }
    TurnOutcome::Failed(step_limit())
}
```

这段故意只示范关键分支：真实代码还需累计 usage、区分工具错误与 DB 错误、处理 `Esc`/网络取消，以及在 `response.calls` 非空时确认 stop reason 真的是工具阶段。工具执行顺序取供应商响应里的 ordinal，**不是** HashMap 遍历顺序。等待 DB ack、渲染 ack 和网络时不得跨 `.await` 持有 `AppState` 可变借用或同步锁。模型请求不能绕过现有 `RequestKey` 代次过滤。

## 16. 四工具的模型可见契约

四类供应商都使用 JSON/function tool schema。Codex 的 `apply_patch` 在 Codex 自身是 freeform tool；多数其他供应商只接受 JSON 参数，因此 lx 的 `edit` 用**一个字符串字段 `patch` 承载相同补丁文本**，执行行为参考 Codex，不能假称 wire 格式也相同。

```json
{"name":"read","parameters":{"type":"object","properties":{"path":{"type":"string"},"offset":{"type":"integer","minimum":1},"limit":{"type":"integer","minimum":1}},"required":["path"]}}
{"name":"search","parameters":{"type":"object","properties":{"pattern":{"type":"string"},"path":{"type":"string"},"include":{"type":"string"},"literal":{"type":"boolean"},"case_sensitive":{"type":"boolean"},"limit":{"type":"integer","minimum":1}},"required":["pattern"]}}
{"name":"edit","parameters":{"type":"object","properties":{"patch":{"type":"string"}},"required":["patch"]}}
{"name":"write","parameters":{"type":"object","properties":{"path":{"type":"string"},"content":{"type":"string"}},"required":["path","content"]}}
```

所有 schema 的 `additionalProperties=false`；运行时仍须做同样校验，不信任模型一定遵守 schema。`offset` 以零为首行还是一为首行必须在实现中固定并写入工具描述；建议对齐 Codex 常见行号展示，输入 offset 为 1-based，返回行号也是 1-based，省略表示从首行读。`limit`、参数字节数、结果字节数都有上限；给模型的结果应含 `truncated`/`next_offset`，使它可继续读取。`search` 默认路径为 `.`，不是整个文件系统；显式绝对路径允许跨工作区。`write` 的空内容是合法的清空文件，不能因 `content.is_empty()` 拒绝。`edit` 的 Add/Update/Delete/Move 都需要返回实际变更文件、每文件新增/删除行数和部分成功标记。

### 16.1 补丁实例与注意点

```text
*** Begin Patch
*** Update File: src/config.rs
*** Move to: src/settings/config.rs
@@
-const TIMEOUT: u64 = 30;
+const TIMEOUT: u64 = 45;
*** Add File: src/settings/README.md
+Settings module.
*** Delete File: obsolete.txt
*** End Patch
```

`Move to` 紧跟 `Update File`，位于 `@@` 之前；此顺序已与 Codex apply-patch 测试中的真实样例核对。实施时以选定 patch 引擎的解析结果为准并设置合法 fixture。补丁路径从冻结 cwd 解析，绝对路径若 patch 引擎本身不支持，则在 lx 封装层安全转换且在工具描述写明；不能通过拼接 shell 命令扩展。每个目标写入前确认原文上下文仍匹配，避免模型基于旧文件覆盖用户刚改的内容。多文件 patch 可能部分成功，结果只报告真实成功的文件，后续调用可读取其当前状态。

## 17. 协议事件的具体映射

下面的 JSON 是**关键字段示意**，正式 fixture 必须采用当前协议真实 SSE 帧和边界标记，不将这些片段当完整 API 请求。

| wire 事件 | 统一事件 | 执行门槛 |
| --- | --- | --- |
| Responses function call item added | `ToolInputStarted(call_id,name,ordinal)` | 尚不可执行。 |
| Responses arguments delta/done | `ToolInputDelta` / `ToolInputEnded` | done 后校验 JSON。 |
| Chat Completions `delta.tool_calls[index]` | 按 index 合并 ID/name/arguments；首片发 Started | `finish_reason=tool_calls` 且各参数完整。 |
| Anthropic `content_block_start(tool_use)` / `input_json_delta` / `content_block_stop` | Started / Delta / Ended | block stop 后可校验。 |
| Gemini `functionCall` part | Started + Ended（已有完整 args） | 同一 candidate 的重复快照只执行一次。 |

OpenAI Chat Completions 的关键帧：

```json
{"choices":[{"delta":{"tool_calls":[{"index":0,"id":"call_a","function":{"name":"read","arguments":"{\"path\":"}}]},"finish_reason":null}]}
{"choices":[{"delta":{"tool_calls":[{"index":0,"function":{"arguments":"\"src/main.rs\"}"}}]},"finish_reason":"tool_calls"}]}
```

Anthropic 的关键顺序是 `content_block_start(tool_use id/name)` -> 多个 `input_json_delta.partial_json` -> `content_block_stop` -> `message_delta.stop_reason=tool_use` -> `message_stop`；下一次请求必须把 assistant 的 tool_use block 与 user 的 tool_result block 成对送回。Gemini 的 model functionCall part 与 functionResponse part 也必须按顺序成对，不能压成普通文字。Responses 的 function call 输出项必须用原 `call_id` 回传 `function_call_output`。工具 call ID 在同一 session 内若与旧 ID 碰撞，应标协议错误，绝不能复用旧工具行。

## 18. 精确验收样例

| 样例 | 事件/DB 检查 | 文件/UI 检查 |
| --- | --- | --- |
| reasoning -> text（流未结束） | reasoning.completed_at_ms 非 NULL，assistant.status 仍 streaming | 思考标题已完成，正文继续增长。 |
| reasoning -> read loading | reasoning 完成时间早于 tool pending 时间；pending 行已 commit | read 尚未执行，历史先显示 loading。 |
| write -> read 同响应 | tool position 单调，call ID 唯一，两个结果都入库 | read 返回刚写的内容。 |
| update patch 上下文失配 | 工具 failed，模型收到错误结果 | 原文件字节不变。 |
| write 完成、DB 结果失败 | DB 留 pending/running 待恢复；轮次失败 | 文件可能已变化，绝不重复 write；UI 明示需核查。 |
| 崩溃后 `/resume` | 未结算工具变 interrupted，已完成调用/结果成对重建 | 不自动写文件，历史顺序不乱。 |
| 历史预算裁剪 | 被移除轮次的 call/result 一并消失 | 当前轮工具结果仍可见。 |
| 兼容接口拒绝 tools | 轮次 failed，错误码可见 | 无文件操作，不回退成普通聊天。 |

实施中的每个 fixture 均应断言“有没有文件副作用”和“工具是否先持久化”，不只断言最终回答。`src/ui/overlay/ui_gallery/agent/tests.rs` 的样式断言只能作为 UI 视觉基线；它不能证明真实 agent loop 或 DB 顺序。

## 19. 各传输请求与续传样例

lx 配置层有四类 provider transport；`src/chat/transport` 实际有五条路径：官方 OpenAI Chat Completions、OpenAI Responses、OpenAI-compatible（与官方 Chat Completions 共用编码/解码器）、Anthropic、Google。下面只展示与工具相关的字段，省略认证、模型参数和 SSE 外壳；这些是**待 fixture 验证的目标形状**，不能直接复制成完整请求。

### 19.1 OpenAI Chat Completions 与 compatible

当前 `openai::build` 把 `request.turns` 映射为 `{role,content}` 并请求 `chat/completions`；工具化后必须在 body 加 `tools`，后续响应把 call/result 编成两条相邻消息：

```json
{
  "tools": [{"type":"function","function":{"name":"read","description":"Read a file","parameters":{"type":"object","properties":{"path":{"type":"string"}},"required":["path"],"additionalProperties":false}}}],
  "messages": [
    {"role":"user","content":"Read src/main.rs"},
    {"role":"assistant","content":null,"tool_calls":[{"id":"call_a","type":"function","function":{"name":"read","arguments":"{\"path\":\"src/main.rs\"}"}}]},
    {"role":"tool","tool_call_id":"call_a","content":"1: fn main() {}"}
  ]
}
```

**注意点：** `arguments` 在 wire 中是 JSON 字符串，在 lx 内只解析一次为 JSON 值；不能再对字符串二次编码。compatible 服务可能不接受 `stream_options.include_usage` 或 tools；后者 4xx 要明确归类，不可静默移除 tools 重试。相同 `index` 的增量合并，同次响应多个 `index` 按 index 顺序调度；`[DONE]` 只结束本次流，不直接宣告用户轮次结束。

### 19.2 OpenAI Responses

当前 `responses::build` 把 turns 映射为 `input`；工具化后将函数定义放 `tools`，下一次采样的输入中保留函数调用输出项：

```json
{
  "tools": [{"type":"function","name":"read","description":"Read a file","parameters":{"type":"object","properties":{"path":{"type":"string"}},"required":["path"],"additionalProperties":false}}],
  "input": [
    {"role":"user","content":"Read src/main.rs"},
    {"type":"function_call","call_id":"call_a","name":"read","arguments":"{\"path\":\"src/main.rs\"}"},
    {"type":"function_call_output","call_id":"call_a","output":"1: fn main() {}"}
  ]
}
```

**注意点：** `response.function_call_arguments.delta` 需按 item/call ID 合并，`done` 只是参数完成；只有整个 response 成功完成且调用集合一致时才能开始调度。流式 `response.completed` 与 assistant 最终回答不是同一层终态。若官方协议返回 provider 执行的 hosted tool 项，本期不得误当成本地 read/search/edit/write 再执行。

### 19.3 Anthropic Messages

当前 `anthropic::build` 只把 turns 变成纯文字 content。工具化后 assistant 的 tool_use block 与 user 的 tool_result block 相邻：

```json
{
  "tools": [{"name":"read","description":"Read a file","input_schema":{"type":"object","properties":{"path":{"type":"string"}},"required":["path"]}}],
  "messages": [
    {"role":"user","content":"Read src/main.rs"},
    {"role":"assistant","content":[{"type":"tool_use","id":"toolu_a","name":"read","input":{"path":"src/main.rs"}}]},
    {"role":"user","content":[{"type":"tool_result","tool_use_id":"toolu_a","content":"1: fn main() {}"}]}
  ]
}
```

**注意点：** `content_block_index` 决定 tool_use 与 thinking/text 的真实顺序；`input_json_delta` 可能与其他 block 交错，不能用单个全局参数缓冲。当前 decoder 已绑定 thinking signature 的 block index，工具化不能打乱这个映射。同 provider 多步续传若需要原样回传 thinking block/signature，必须保证签名与原上下文匹配；跨 provider 只回放规范化工具信息。`stop_reason=tool_use` 表示继续 agent loop。

### 19.4 Google Gemini generateContent

当前 `google::build` 调用 `streamGenerateContent?alt=sse`，把 assistant role 映射成 `model`。沿用此 API，在请求加 `tools.functionDeclarations`，将 model functionCall 与 user functionResponse 成对回传：

```json
{
  "tools": [{"functionDeclarations":[{"name":"read","description":"Read a file","parameters":{"type":"object","properties":{"path":{"type":"string"}},"required":["path"]}}]}],
  "contents": [
    {"role":"user","parts":[{"text":"Read src/main.rs"}]},
    {"role":"model","parts":[{"functionCall":{"name":"read","args":{"path":"src/main.rs"}}}]},
    {"role":"user","parts":[{"functionResponse":{"name":"read","response":{"content":"1: fn main() {}"}}}]}
  ]
}
```

**注意点：** Gemini 的流帧可能给累计候选快照，必须以 candidate/part 位置或稳定 ID 去重，绝不能每帧重复执行同一个 functionCall。某些模型带 `thoughtSignature`，同 provider 续传时保存并回传其必要位置；没有 ID 时内部合成 call ID 只用于 lx DB/去重，不伪造供应商未定义的 ID 字段。不同 Gemini API 版本的 `functionResponse.response` 包装必须以当前 `streamGenerateContent` 官方 fixture 确认。

## 20. 文件归属与实施注意点

| 文件/目录 | 预计改动 | 必须维持的边界 |
| --- | --- | --- |
| `src/chat/events.rs` | 统一事件、结构化模型历史/工具结果 | 纯数据，不导入 app/ui/db 服务。 |
| `src/chat/transport/{openai,responses,anthropic,google}` | 请求 tools、解析参数和续传结果 | 保留现有认证、variant、SSE 完成性判断。 |
| `src/chat/client/` | 将一次流结果交给 agent loop，延续无内容重试规则 | 已执行工具后不自动重试；不直接写 AppState。 |
| `src/chat/agent_loop/`（按需） | 逐次采样、串行工具调度、取消与限额 | 单模块职责明确，勿复制 Codex core。 |
| `src/chat/tools/`（按需） | read/search/edit/write 后台执行 | 相对路径冻结 cwd；不触碰 Git 状态；结构化结果。 |
| `src/event/chat.rs` | host 请求/DB ack/可见 loading 握手、消息归属 | 唯一事件循环，严格 `RequestKey` 过滤。 |
| `src/app/state/lx/chat.rs`、`src/app/update/chat/` | 有序步骤视图、思考独立终态、纯状态机 | 不执行文件/SQLite IO。 |
| `src/app/effects.rs` | Begin/Running/FinishTool 效应或等价数据 | effect 必须可关联 session/message/call ID。 |
| `src/db/mod.rs`、`src/db/chat/`、`src/db/service.rs` | v9 迁移、条件 CRUD、恢复与按序加载 | 只在 db 模块使用 rusqlite。 |
| `src/chat/context/` | 完整轮次与工具成对回放、预算裁剪 | 工具结果不拼入 assistant 普通文本。 |
| `src/ui/main_content/lx/history.rs`、`src/ui/text.rs` | 真实工具行、状态与尺寸 | 只读 state，复用 Gallery 样式 token。 |

**输入与并发：** TUI 可切换工作区和窗格，但在途轮次绑定发送时的 workspace cwd、会话 ID、pane ID、generation；后续焦点变化不影响工具目标。多个不同窗格的 agent loop 可以并发，每个 loop 内工具串行；同一文件被两个 loop 同时写入时本期无跨窗格锁，文件修改工具应在写前重读并验证上下文，write 是明确整文件覆盖。若实施判断跨窗格竞态会造成无法接受的数据损坏，应以按规范化路径的短时写锁保护文件变更，不把锁扩展成全局模型采样串行。

**取消与退出：** 不得在 `spawn_blocking` 开始后假设 `JoinHandle::abort` 已回滚文件写入；只有任务返回后才能记录真实结果。关闭 TUI 时不新增工具任务，已开始任务的收尾与进程强退之间有天然不确定性，DB 恢复只标 interrupted。不要为本任务改动 `src/tui/` 的鼠标上报/原始模式恢复顺序。

**错误与日志：** `Result` 错误应区分协议、参数、文件系统、DB 四类。工具返回给模型的错误可含路径和有限的原因文本；`tracing` 只记 call ID、工具名、状态和耗时，不打印参数全文、文件内容、API Key。生产代码不使用 `unwrap/expect`，用户可见短文案集中 `src/ui/text.rs`。

### 20.1 已实施（G2：统一纯事件模型 + 有序步骤时间线 + 思考独立完成态）

实际落地与上文示意的差异（均为等价实现，不改变不变量）：

- `ChatDelta` 在既有 `Part`/`Usage`/`Finished` 之上**最小扩展**：新增 `ReasoningEnded { item_id: Option<String> }`、`ToolInputStarted { call_id, name, ordinal }`、`ToolInputDelta { call_id, chunk }`、`ToolInputEnded { call_id, arguments_json }`、`ResponseFinished { stop: ModelStepStop }`，`ModelStepStop = Final | ToolCalls | Failed | Cancelled`。未引入独立的 `ReasoningStarted`：思考块由首个 reasoning `Part` delta 隐式开启（与现有解码器一致）。
- `ChatDelta::is_incremental()` / `payload_bytes()` 作为客户端回投与 checkpoint 字节统计的统一口径：整轮终态 `Finished` 不进即时增量批，`ResponseFinished` 进。
- 新增常量 `MAX_TOOL_ARGUMENTS_BYTES`（1 MiB，与 `db::chat::MAX_TOOL_ARGUMENT_BYTES` 由测试锁定一致）、`MAX_SUMMARY_CHARS`（256）与稳定错误码 `TOOL_ARGUMENTS_TOO_LARGE_CODE` / `TOOL_ARGUMENTS_INVALID_CODE`。
- `ModelItem` 与 `ChatRequest::items` 已加入；当前各 transport 仍按 `turns` 编码，`items` 与 `turns` 在纯文本历史下等价（工具化编码在协议阶段接入）。
- `AssistantStepView` / `ToolCallView` / `ToolStatus` / `ToolSummary` 落在 `app/state/lx/chat/steps.rs`；流事件转换在 `app/state/lx/chat/stream.rs`；DB→视图投影在 `app/update/chat/view.rs`（`ChatMessageView::steps` 是唯一顺序来源，`parts()`/`text_parts()`/`reasoning_parts()`/`tools()`/`step_at()` 为只读辅助）。
- `reasoning_streaming(message, step)` 只看步骤是 Reasoning 且 `completed_at_ms` 为空、消息 `status == "streaming"`；不再要求它是最后一个 reasoning 块或最后一条消息。
- **限制**：内存只保留“当前开放思考块”这一个状态，`ReasoningEnded { item_id: Some(..) }` 的 item ID 仅作诊断（`tracing::trace`），实际关闭尾部开放块；按 item ID 索引与并发多 item 结束留待后续阶段。
- UI 本组只做编译所需最小改造：工具步骤以**单行摘要**占位（`工具名 · 状态 · 路径 · 行范围 · 计数`，带 `TODO(G4)` 注释），行数口径与渲染共享 `tool_rows()`，保证不 panic、不吞行、不与几何漂移；真实工具行由 G4 实现。

### 20.2 已实施（G3：`read` / `search` / `edit` / `write` 执行层）

落地位置 `src/chat/tools/`（`mod.rs` 分发 + `types.rs` 契约 + `schema.rs` 规格 + `args.rs` 参数校验 + `path.rs` 路径解析 + `text.rs` 编解码 + `read.rs` / `search.rs` / `patch.rs` / `edit.rs` / `write.rs` / `lock.rs` / `output.rs`），原子写能力落 `src/platform/atomic_write.rs`。与 §9 / §16 / §20 设计一致的实现，及少量等价差异：

- **契约**：`ToolExecutionResult` 在 §10.3 基础上补齐 `Read` 的 `kind`（文件/目录）、`start_line` / `end_line` / `total_lines` 与 `SearchMatch::line_truncated`，使 `format_for_model` 无需再回头解析参数。
- **三口径**：模型文本、UI 摘要、DB 结果 JSON 全部由 `output.rs` 生成（`format_for_model` / `to_summary` / `to_result_json`）。`chat/tools` 不依赖 `app`，故摘要类型 `chat::tools::ToolSummary` 与 `app::state::ToolSummary` 字段对齐但独立存在，由事件层做一次字段映射，避免 `chat -> app` 反向依赖。
- **补丁引擎**：在 lx 内实现 Codex 风格语法子集（`*** Begin Patch`/`*** End Patch`、`*** Add File`、`*** Update File` + `*** Move to` + `@@` 块、`*** Delete File`），未依赖 Codex crate。**不做项**：`*** Environment ID`、`*** End of File`、统一 diff 头（`---`/`+++`/`@@ -1,2 +1,3 @@` 行号形式）、模糊匹配（Codex 的 rstrip/trim/Unicode 标点归一化回退）。上下文一律**精确且唯一**匹配，多处或不匹配即 `PatchContextMismatch`。
- **两阶段执行**：先解析补丁 + 解析全部路径 + 纯路径级检查（Add 目标不得已存在、Update/Delete 必须是已存在普通文件、Move 目标不得是目录），再逐个 hunk 读取并落盘。格式与路径问题在零副作用阶段返回；上下文失配发生在提交阶段，因此**前面已成功的文件会如实出现在 `Failed{changes}` 里**（与 §13 风险 3 的“多文件非原子”一致）。
- **`partial` 语义**：`Edit.partial` 表示“本次补丁里有 hunk 应用后内容与原文相同（无实际变更）”。所有 hunk 都无变更时整体返回 `NoChanges`。`Delete` 的 `removed` 是被删除文件的原文本行数（二进制内容按 0 计）。
- **落库 JSON**：成功结果是 `{"tool": <名字>, ...}`；失败结果是 `{"tool": "error", "code", "message", "changes"}`。`ToolExecutionResult::Failed` 按 §10.3 的契约不携带工具名，因此失败 JSON 用 `"error"` 标记；调用方（事件层/DB 层）本来就知道 name，需要时可按调用侧补齐。
- **原子写与锁**：`platform::write_file_atomic` 为同目录临时文件（`.{name}.tmp.{pid}.{seq}`）+ `sync_all` + rename，沿用既有文件权限；Unix 用 `rename` 覆盖，非 Unix 退化为“删除目标再改名”（Windows 语义限制，已在注释与文档写明）。写操作按**规范化路径**加短时锁（`lock::with_path_lock`，锁表惰性建立、闭包内持有），不做全局串行。目标为符号链接时写入其解析后的真实文件（原子替换不能作用于链接本身）。
- **编码保留**：`text::SourceText` 在读取时记录 BOM 与主要换行风格，写回时还原；`edit`/`write` 都不会把 CRLF 变 LF 或丢掉 BOM。新建文件统一 LF、无 BOM。
- **`read`**：目录列表同样支持 1-based `offset`/`limit` 与 `next_offset`（条目按名排序，`[dir]`/`[file]` 前缀），使超大目录可分页。单行超过 `MAX_READ_LINE_CHARS`（4000 字符）时截断并置 `truncated`。
- **`search`**：遍历复用 `ignore::WalkBuilder`（`git_ignore(true)`、`require_git(false)`、不跟随符号链接、`max_depth(32)`），`include` 用 `ignore::overrides` 的白名单语义且**相对搜索根**匹配；结果为 `(path, line)` 稳定排序。上限为 `MAX_SEARCH_RESULTS`(200)、`MAX_SEARCH_FILES`(20000)、`SEARCH_TIMEOUT`(10s)、单文件 `MAX_SEARCH_FILE_BYTES`(8 MiB)、单行 `MAX_SEARCH_LINE_CHARS`(400)；任一触发都置 `truncated`。二进制（含 NUL）与非 UTF-8 文件静默跳过。正则使用 `regex` crate；它在依赖树中已由 `globset`/`ignore`/`termwiz` 引入，本轮只是显式声明为直接依赖（`Cargo.lock` 仅新增一行，无新 crate）。
- **参数校验**：`args.rs` 按字段白名单解析，未知字段/类型错误/0 或负整数一律 `InvalidArguments`，错误消息只含字段名与期望类型，不回显参数内容；参数体超过 1 MiB 直接 `TooLarge`。未知工具名回显名字截断到 64 字符。
- **路径**：`path::resolve` 先校验 `ctx.cwd`（非空、绝对、存在且为目录），不满足即 `PathUnresolvable`；随后绝对路径原样、相对路径拼接。**未**做任何工作区边界限制（与 §9 一致）。
- **日志**：工具执行层**不记录**参数与文件内容；唯一日志是 `platform::atomic_write` 的权限复制失败（`tracing::debug!`，只含错误与固定文案）。

### 20.3 已实施（G4：四类 transport 的工具协议）

落地位置：`src/chat/prompt.rs`（`lx` 身份提示）、`src/chat/transport/tools/`（四工具 wire 形状 + 历史归并 + 跨帧参数缓冲）、四个 `src/chat/transport/{openai,responses,anthropic,google}/mod.rs`（请求编码与解码）、`src/chat/client/`（重试闸门与 HTTP 错误归类）。

**请求侧**

- 四工具定义恒发；`ChatRequest::items` 非空时按 call/result 结构编码，为空时逐字保持旧的 `turns` 纯文本编码（每类协议各有回归测试）。
- system 指令：OpenAI Chat `messages[0].role=system`、Responses 顶层 `instructions`、Anthropic 顶层 `system`、Google `systemInstruction.parts[0].text`；文本是 `LX_SYSTEM_PROMPT`（只描述 lx、四工具、相对路径基准与执行/回报要求，不含品牌或整份系统指令）。
- `tool_choice` 未强制设置（保持端点默认 auto）。
- **能力类失败归类**：`TransportRequest` 新增 `tools_requested: bool`；带 tools 的 4xx/501（400/404/405/409/415/422/501）错误码前缀为 `tool calls not supported by endpoint · http <status> · <detail>`，401/403/408/429/5xx 保持原 `http <status>` 语义。**没有**任何“移除 tools 重试”的降级路径。

**解码侧（wire 事件 → 统一 delta）**

| 传输 | wire 事件 | 统一 delta |
| --- | --- | --- |
| Chat Completions | `choices[].delta.tool_calls[index]`（id/function.name/function.arguments 分片） | 首个可识别分片发 `ToolInputStarted{ordinal=index}`，其后每片 `ToolInputDelta` |
| Chat Completions | `finish_reason="tool_calls"` | 闭合全部调用：`ToolInputEnded` × N（按 index 升序）+ `ResponseFinished{ToolCalls}` |
| Chat Completions | `finish_reason ∈ {stop,length,content_filter}` 且无工具 | `ResponseFinished{Final}` + `Finished{Complete}` |
| Chat Completions | `data: [DONE]` | 仅置完成位（不代表用户轮次结束） |
| Responses | `response.output_item.added`（`item.type=="function_call"`） | `ToolInputStarted{call_id=item.call_id, ordinal=output_index}` |
| Responses | `response.function_call_arguments.delta` / `.done` | `ToolInputDelta` / `ToolInputEnded`（done 的 `arguments` 为权威值） |
| Responses | `response.completed` | 先补发未闭合调用的 `ToolInputEnded`，再 `ResponseFinished{ToolCalls/Final}` + `Finished{Complete}`（usage 先于终态） |
| Anthropic | `content_block_start(type="tool_use")` | `ToolInputStarted{call_id=block.id, name=block.name, ordinal=index}` |
| Anthropic | `content_block_delta(type="input_json_delta")` | `ToolInputDelta{chunk=partial_json}`（按 block index 绑定） |
| Anthropic | `content_block_stop`（tool_use / thinking） | `ToolInputEnded` / `ReasoningEnded{item_id=index}` |
| Anthropic | `message_delta.delta.stop_reason="tool_use"` / `"end_turn"` | `ResponseFinished{ToolCalls}` / `{Final}` |
| Anthropic | `message_stop` | `Finished{Complete}` |
| Google | `candidates[].content.parts[].functionCall{name,args}` | `ToolInputStarted` + `ToolInputEnded`（args 已是完整对象，序列化为 JSON 字符串） |
| Google | `finishReason ∈ {STOP,SAFETY,RECITATION,MAX_TOKENS}` | 有 functionCall → `ResponseFinished{ToolCalls}`，否则 `{Final}`；随后 `Finished{Complete}` |

**与设计 §17 / §19 的偏差（均为等价或更严格）：**

1. **参数非法 JSON 直接 `Err`，不发 `ToolInputEnded`**。§17 允许“发 Ended 让 G2 状态层判失败”，但四类协议在 `ToolInputEnded` 之前都已拿到**权威完整串**（Chat/Anthropic 在闭合事件、Responses 在 `arguments.done`、Google 在完整对象），非法即协议级问题，直接失败更早、更不可能把不可解析调用推进到 `Pending`。超限与半截参数同理直接 `Err`（`TOOL_ARGUMENTS_INVALID_CODE` / `TOOL_ARGUMENTS_TOO_LARGE_CODE`）。
2. **EOF 时未闭合工具输入**由新增的 `StreamDecoder::has_open_tool_input()` 检出，client 以 `tool input truncated` 失败，不补造 `ToolInputEnded`。
3. **hosted tool 处理**：Responses 的 `web_search_call` / `file_search_call` / `code_interpreter_call` / `image_generation_call` / `mcp_call` / `mcp_list_tools` / `mcp_approval_request` / `computer_call` / `local_shell_call` 一律协议错误（不当成本地文件工具）；Anthropic 的 `server_tool_use` / `web_search_tool_result` / `code_execution_tool_result` block 同样拒绝。普通输出项（`message` / `reasoning`）忽略。
4. **Google 去重与合成 ID**：内部 call ID 为 `gemini-{candidate index}-{part index}-{name}`（不含时间戳），并按 `(candidate, part, name, args 序列化)` 二次去重累计快照；该 ID **不写入**任何供应商字段（`functionResponse` 只带 `{name, response}`）。
5. **Google `functionResponse.response` 包装**：成功为 `{"result": <文本>}`，失败为 `{"error": <文本>}`（用协议在 `functionResponse` 里保留结构的最简包装，失败不伪装成功）。
6. **Google `finishReason="MAX_TOKENS"` 也结束本次响应**（按 `Final`）；旧实现只在 STOP/SAFETY/RECITATION 结束，未知原因等 EOF。
7. **Chat Completions 的 `finish_reason="tool_calls"` 同时置完成位**：`finish_reason` 已是协议级响应结束信号，缺少 `[DONE]` 时不应误判 `stream ended without completion`。旧的 `unsupported finish_reason` 与 `tool calls are not supported` 拒绝分支已删除；未知 `finish_reason` 仍是协议错误。
8. **已废弃的 `delta.function_call`**（无 index/ID）明确协议错误，不猜调用。
9. **重试闸门**：`can_retry` 要求“零增量 + 零工具输入 + 临时故障（timeout/connection failed/429/5xx）”。任何 `ToolInputStarted/Delta/Ended`、任何正文/思考增量、任何 `ResponseFinished` 都会永久关闭本响应的重试；`StreamOutcome` 未改动（仍是 deltas + stop）。
10. **Responses `response.incomplete`**：无未闭合工具时按旧的 `ResponseFinished{Final}` + `Finished{Complete}` 处理；有未闭合调用则协议错误。
11. **Anthropic `cache_creation_input_tokens`** 仍按旧行为映射进 `Usage.cache_write`；`message_start` 的 input usage 与 `message_delta` 的 output usage 分两条增量上报，解码器只报本响应值（累加由上层负责）。
