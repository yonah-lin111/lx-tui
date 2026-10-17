# lx Agent 思考生命周期与文件工具：实施任务

> 状态：待用户审核，全部任务未开始。用户确认 `design.md` 与本文件后才修改代码。
>
> 约束：不修改 Git 状态；不自动建分支、暂存或提交。代码仍遵守 `app/` 纯状态、`ui/` 纯渲染、事件层协调 IO。检查单文件 1000 行上限。

## 0. 开工前复核

- [ ] 收集用户对 design/task 的审核意见；把变动边界写回文档后再实施。
- [ ] 只读检查工作树、当前 DB 版本和其他在途变更；不覆盖用户文件。
- [ ] 重读四份项目规范，并用 codegraph + codebase memory 核实待改符号、调用链、索引覆盖情况。
- [ ] 以 Context7 官方协议资料核实四类传输字段与流事件；对照参考仓库的实际版本，不凭记忆实现。
- [ ] 确定 patch 引擎复用方式：评估 Codex apply-patch crate 是否能作为独立最小依赖；若不能，列出 lx 所需 patch 语法子集、fixture 和错误兼容性，避免复制 core。
- [ ] 列出受影响文件和新模块归属；单文件接近 1000 行时先按职责拆分，不叠补丁。

## 1. 复现当前思考状态错误

- [ ] 在 `src/app/state/lx/chat/tests.rs` 与历史渲染测试中构造“reasoning 已结束，assistant 仍 streaming，后续 text/tool 已开始”的状态，证明现有 `reasoning_streaming` 会误判。
- [ ] 在协议 fixture 中加入明确 reasoning-end 与无 end 但后续正文/工具开始两类路径。
- [ ] 记录思考持续时间目前依赖 `ended_at_ms` 的位置；新增完成标记不能破坏时长计算和旧历史显示。

验收：测试先能稳定复现延迟完成；后续修复后同一 fixture 在正文/工具阶段显示思考完成。

## 2. v9 数据模型与迁移

### 2.1 Schema

- [ ] 将当前 `SCHEMA_VERSION=8` 升到 9，新增独立事务迁移，不直接改旧迁移文本；旧库、新库与重复打开都得到相同结构。
- [ ] `agent_tool_call` 增加 assistant 时间线 `position` 和 `interrupted` 状态；保留 `(session_id, call_id)` 唯一约束，增加必要的 `(message_id, position)` 索引/约束。
- [ ] `agent_message_part` 增加独立 `completed_at_ms`；旧 completed/interrupted/failed reasoning 块合理回填完成时间，旧内容和排序不变。
- [ ] 核对 SQLite ALTER/表重建与 CHECK 约束的真实行为，确保失败回滚时 `user_version` 和旧数据不变。
- [ ] 明确跨 part/tool 表的同 position 不变量，由事务分配并测试冲突；不要用时间戳作为唯一排序依据。

### 2.2 CRUD 和载入

- [ ] 为工具调用实现 pending、running、completed、failed、cancelled、interrupted 的条件更新；所有操作校验 workspace/session/message/call ID 归属。
- [ ] pending 写入与步骤位置分配同一事务；完成写入结果/错误、时间与耗时同一事务；重复 ack 幂等，冲突 ack 拒绝。
- [ ] 扩展会话加载，一次性取有序消息、parts 和 tool calls，合并成单条 assistant 时间线；不产生 N+1 查询。
- [ ] 扩展启动恢复：pending/running 工具改 interrupted，消息流状态沿现有逻辑恢复；重复恢复返回零变更。
- [ ] 调整 checkpoint/finish 的 `replace_parts`，保留时间线位置和思考完成字段；迟到 checkpoint 不可覆盖终态或已落库工具。
- [ ] 结果与参数长度上限沿现有 DB 字节限制设计；超限返回可识别错误，不写半行。

验收：临时库测试覆盖 v7 升级、迁移失败回滚、顺序、跨工作区拒绝、状态转移、重复结果、异常恢复和 `/resume` 全量加载。

## 3. 纯事件模型与 app 状态

- [x] 在 `src/chat/events.rs` 增加思考开始/结束、工具参数开始/增量/结束与“本次模型响应结束”纯数据语义；不要混淆响应结束和用户轮次结束。
- [x] 扩展 `ChatPartView` 思考完成状态，增加工具步骤视图，保持每轮只有一条 assistant 消息。
- [x] `app/update/chat` 以 `RequestKey` 过滤旧事件；文本、思考和工具按统一 position 入状态。切换 kind 或收到显式 end 时只关闭当前 reasoning 块一次。
- [x] 工具状态更新只经 app update；pending、running、终态对应纯状态与 DB Effect/ack，不在 update 中执行文件 IO。
- [x] 定义 call ID 重复、旧 generation、同 ID 参数重放、取消后的迟到结果处理规则，并覆盖纯状态测试。
- [x] 保持 `/new`、`/resume`、多窗格、切换模型和 Esc 双击停止的现有交互；生成中 Enter 仍不得提交新轮。

> G2 已实施：事件与常量见 `src/chat/events.rs`；步骤视图/状态机见 `src/app/state/lx/chat/{steps,stream}.rs`；DB→视图投影见 `src/app/update/chat/view.rs`；会话命令拆分到 `src/app/update/chat/session.rs`。差异与限制（item ID 只关闭尾部开放块、UI 工具行为单行占位）记录在 `design.md` §20.1。

验收：无终端、无网络的状态测试证明一轮多步顺序、思考及时完成、工具状态正确、旧事件不污染新轮。

## 4. 四类传输的工具协议

### 4.1 通用请求与解码

- [ ] 扩展 `ChatRequest` 为结构化消息/内容块/工具调用/结果，保留旧纯文本历史的编码结果。
- [ ] 固定 `lx` agent 身份与四工具 schema，合理放入各协议的 system/developer 和 tools 字段；不加入 Codex core、MCP 或 shell 工具。
- [ ] 解码器按供应商 item/index/call ID 聚合跨帧参数，设置参数体上限；完整参数到达前不得执行。
- [ ] 区分 `finish_reason=tool_calls/tool_use` 和最终正常完成；EOF 半截参数、缺失 call ID、重复冲突、非法 JSON 给明确错误。
- [ ] 保留现有 usage、retry、取消、thinking tag 和 provider metadata 处理；已产生工具副作用的响应不得自动重试。

### 4.2 各协议 fixture

- [ ] OpenAI Responses：function call 增量/完成、call ID、function_call_output 续传、reasoning 完成、两次采样。
- [ ] OpenAI Chat Completions：`delta.tool_calls[index]` 跨帧合并、`finish_reason=tool_calls`、role=`tool` 续传、`[DONE]`；compatible 4xx 不支持 tools 明确报错。
- [ ] Anthropic：tool_use block、input_json_delta、content_block_stop、tool_result block、thinking signature 与顺序。
- [ ] Google：functionCall/functionResponse、流快照去重、thought signature、候选/part 顺序与无原生 call ID 映射。
- [ ] 每类覆盖一工具、同响应两工具、reasoning -> tool -> reasoning -> text、取消、无效 JSON、网络 EOF、用量累加。

验收：固定流式 fixture 不访问真实 API 即能完成统一事件和下一次请求编码；真实供应商验证留给用户。

## 5. 文件工具

- [x] `read`：相对/绝对路径、文件行号/范围、目录列表、二进制与超限错误、截断元数据。
- [x] `search`：默认 cwd、显式任意路径、regex/literal、include、忽略规则、结果上限、超时和非法模式；结果稳定排序以便模型使用。
- [x] `edit`：Codex 风格 patch 的 Add/Update/Delete/Move，精确上下文匹配、路径解析、多文件实际变更报告；不触碰 Git 索引。
- [x] `write`：新建或完整覆盖，明确父目录处理、UTF-8、BOM/换行、权限错误和差异统计；原子替换。
- [x] 文件 IO 后台执行，串行调度同响应多个工具；前一次写入结果对后一调用可见（执行层同步阻塞，由调用方 `spawn_blocking`；同文件写按规范化路径加短时锁）。
- [x] 工具输入只接受定义内参数；未知工具名/多余危险参数/无效 JSON 返回结构化失败，不能隐式调用 shell。
- [x] 用临时目录覆盖符号链接、绝对路径、同名文件、已有文件覆盖、文件不存在、patch 上下文不匹配、部分多文件失败与真实变更摘要。

> G3 已实施：执行层在 `src/chat/tools/`（分发、契约、schema、参数校验、路径解析、文本编解码、四工具、Codex 风格补丁引擎、路径写锁、三口径输出），原子写能力在 `src/platform/atomic_write.rs`；补丁语法子集与不做项、两阶段执行与 partial 语义、include glob 相对搜索根等差异记录在 `design.md` §20.2。工具结果对接 DB/事件/loop 仍待 G5/G6。

验收：独立工具测试无需模型/TUI；不会在事件循环中执行阻塞文件操作，错误结果可回传模型。

## 6. Agent loop 与持久化门槛

- [ ] 在 `chat/` 放纯请求/工具编排逻辑，`event/chat` 负责 spawn、取消、DB ack 交接和状态投递；避免 `chat` 直接依赖 AppState。
- [ ] 一次模型响应解析完工具输入后按输出顺序串行：pending DB ack -> UI loading -> running DB ack -> 文件执行 -> 结果 DB ack -> UI 结果。
- [ ] 同响应全部工具结算后形成下一次请求；最终无工具响应才 FinishChatTurn。设置步骤/调用上限并显示原因。
- [ ] pending/running/结果 DB 失败时执行矩阵按 `design.md` 第 11 节处理，尤其“pending 失败不得执行”和“写入成功但结果入库失败不得重试”。
- [ ] 取消先停止未开始调用；已开始 IO 记录真实完成结果后停止后续模型采样。启动恢复不自动重跑任何调用。
- [ ] 统计多次采样 usage；只在整轮结束时提交消息终态和总用量，避免重复累计。
- [ ] 使用假 transport + 临时 DB/目录做端到端可控测试：read -> edit -> read -> 最终回答；同响应两工具；DB ack 失败；崩溃恢复；Esc；旧 generation；上限。

验收：工具首次可见和入库先于文件变更，恢复后记录与文件真实状态一致；无重复执行。

## 7. 上下文与恢复

- [ ] 改 `chat/context` 的轮次表示与预算计算，完整轮次携带有序文字、思考及工具调用/结果；裁剪时整轮删除，不留下孤立结果。
- [ ] 当前轮多个模型步骤使用累计的 in-memory 上下文，不因只筛 `assistant.status='complete'` 丢失已执行工具。
- [ ] `/resume` 加载工具步骤并还原 UI 顺序；后续请求能成对编码 completed/failed 调用及结果。
- [ ] interrupted 调用在历史中明确可见；不给模型伪造成功结果，也不自动执行。跨 provider 恢复仅携带通用字段，不回传不兼容思考签名。
- [ ] 旧无工具会话、新工具会话、被截断历史、工具结果超限及同 call ID 去重均有测试。

验收：恢复后用户说“继续”，模型请求中可检查到原调用和结果，且 ID/顺序正确。

## 8. TUI 展示

- [ ] 将 Gallery agent 样例的样式 token 与行构造抽成真实历史可复用的纯渲染能力；不直接把假数据写进生产 UI。
- [ ] 按工具状态显示 loading/running/completed/failed/interrupted；read 显示范围，search 显示摘要，edit/write 显示变更计数；工具全文不铺满历史。
- [ ] 思考完成由块状态决定；显式折叠/展开、时长与终态保持一致。
- [ ] `history::layout` 与 render 使用同一时间线和行数；置顶用户行、滚动条、鼠标命中及回到底部对工具步骤正确。
- [ ] 新文案集中 `src/ui/text.rs`；宽/窄/极矮窗口、长路径、中文宽字符、多个工具和错误状态都做 buffer/geometry 测试。

验收：自动测试证明几何与状态正确；真实配色、对齐、滚动由用户 `cargo run` 人工验收。

## 9. 精确验证与交付

- [ ] 每完成一个受影响模块，先运行对应精确测试，解决失败后再进入下步；不要无目的重复全量测试。
- [ ] 最终执行项目要求的 `cargo fmt`、`cargo clippy --all-targets -- -D warnings`、`cargo test` 并记录结果；如触碰 `src/tui/` 恢复路径，额外运行 `tui::tests` 顺序测试。
- [ ] 检查新旧源文件行数、模块依赖方向、生产 `unwrap/expect`、tracing 和用户文案位置；没有 shell 注入和 Git 状态修改。
- [ ] 按实际模块更新 `docs/standards/project-directory-structure.md` 与必要的 TUI 规范；不改无关 HTML 资源规范。
- [ ] 给用户真实供应商/终端人工验收清单：四类模型各一次工具调用、思考先完成、loading 先出现、同轮多工具、`/resume`、异常中断后的历史、长路径与窄窗口。
- [ ] 最终报告清楚区分自动验证与未验证的真实 API/TUI 外观，列明风险和实际变更；不自行提交或改分支。

## 10. 关键接口交付清单

以下签名是实施目标，不是让生产代码原样照抄。若选定更简洁的类型，需满足同一不变量并在设计文档记录差异。

### 10.1 聊天事件与模型历史

```rust
// src/chat/events.rs 或其职责子模块
pub enum AgentStreamEvent {
    ReasoningStarted { item_id: String },
    Part { kind: PartKind, text: String, metadata: Option<serde_json::Value> },
    ReasoningEnded { item_id: String, metadata: Option<serde_json::Value> },
    ToolInputStarted { call_id: String, name: String, ordinal: usize },
    ToolInputDelta { call_id: String, chunk: String },
    ToolInputEnded { call_id: String },
    Usage(Usage),
}

pub enum ModelHistoryItem {
    UserText(String),
    AssistantText(String),
    AssistantToolCall { call_id: String, name: String, input: serde_json::Value },
    ToolResult { call_id: String, content: String, is_error: bool },
}
```

- [ ] `ToolInputStarted` 必须有可用于去重的稳定 call ID；Google 若缺失则由 `(request generation, model step, candidate index, part index)` 生成，不能用当前时间。
- [ ] `ToolInputDelta` 按同 ID 追加，达到参数上限立即失败；同 call ID 两个不同 name/ordinal 报协议错误。
- [ ] `ReasoningEnded` 对未开始的 item 不可创建空思考；对已完成 item 幂等。若无 ID 的协议发出结束信号，只关闭当前开放块。
- [ ] 模型历史必须保留调用与结果的结构；不能把 `ToolResult` 串接到 `ChatTurn.text`。

### 10.2 DB 服务与事件握手

```rust
// db/chat: 每个方法先验证 workspace/session/message 归属
fn begin_tool_call(&self, key: &ToolKey, position: i64, name: &str,
                   arguments_json: &str) -> Result<ToolCallTicket>;
fn mark_tool_running(&self, ticket: &ToolCallTicket) -> Result<()>;
fn finish_tool_call(&self, ticket: &ToolCallTicket,
                    result: &ToolExecutionResult) -> Result<()>;
fn load_tool_calls_for_session(&self, workspace: &Path, session_id: &str)
    -> Result<Vec<ToolCallRecord>>;
fn recover_interrupted_tool_calls(&self) -> Result<usize>;
```

- [ ] `begin_tool_call` **事务提交后**才发 ticket；调用方仅有 ticket 时才能执行文件 IO。
- [ ] `mark_tool_running` 必须检查 pending -> running；失败则文件不动。`finish_tool_call` 必须检查 running -> terminal，更新行数为零时解释重复/冲突。
- [ ] 事件层处理 BeginTool 的顺序必须为 DB pending -> AppState pending -> 安排可见帧 -> 回复 ticket；不能先回复 ticket 再等 DB。
- [ ] DB 是最终记录，UI 是投影；DB 写失败后 UI 不得显示 completed 并继续采样。

### 10.3 文件工具输出

```rust
pub enum ToolExecutionResult {
    Read { path: PathBuf, content: String, next_offset: Option<usize> },
    Search { matches: Vec<SearchMatch>, truncated: bool },
    Edit { changes: Vec<FileChange>, partial: bool },
    Write { path: PathBuf, created: bool, added: usize, removed: usize },
    Failed { code: ToolErrorCode, message: String,
             changes: Vec<FileChange> }, // 多文件 patch 允许部分已提交
}
```

- [ ] 工具执行层只返回结构化结果；转给模型的文本由同一格式化函数生成，DB 存规范化结果，UI 从中提取摘要。
- [ ] read offset 固定 1-based，行号与 `next_offset` 同口径；省略 offset 从第 1 行开始。空文件、末行无换行、CRLF、Unicode 宽字符均有 fixture。
- [ ] `write` 空字符串合法；`edit` 无实际变更须明确报错；失败工具不能伪装成完成结果。

## 11. 阶段验收门槛

| 阶段 | 可观察证据 | 未达到时停止下一阶段的条件 |
| --- | --- | --- |
| 思考状态 | 单元测试证明 reasoning end 后 assistant 仍 streaming，而思考已 complete | 仍用消息状态推导思考状态。 |
| 迁移/CRUD | 旧 v8 库数据保留，v9 工具表可记录 interrupted 和 position；重复恢复为 0 | 工具状态不能幂等或迁移破坏旧会话。 |
| 协议 | 四个 transport 的 fixture 均产出相同统一事件序列 | 任一传输仍把原生工具调用判作非法文本。 |
| 工具 | 临时目录中的 read/search/edit/write 可独立执行，变更摘要与磁盘一致 | patch 失败却报告成功，或文件 IO 阻塞主事件循环。 |
| loop | mock 模型先请求 write、再 read、最终回答；DB pending/可见 loading 早于写入 | 文件先变化才出现 pending，或同 call ID 执行两次。 |
| 恢复 | `/resume` 合并原有 part/tool 顺序，模型收到 call/result 成对记录 | 历史工具只显示却不进入上下文，或中断调用被当成功。 |
| UI | Gallery 样式与真实工具状态 buffer 测试均通过 | 滚动条/命中几何与工具行不一致。 |

每阶段只运行能验证该阶段的测试。最终项目规定的 fmt/clippy/test 是交付门槛；真实供应商与终端人工检查明确列为未验证项，直到用户实际运行。

## 完成定义

四种工具在四类传输中完成原生调用和结果续传；一条 assistant 消息按步骤顺序显示并入库；思考在自身结束时完成；pending 先于文件 IO 落库；工具结果可在 `/resume` 后回放给模型；异常重启不重做写入；取消、限额和失败路径遵循设计矩阵；精确回归与项目规定的 fmt/clippy/test 通过。任一项缺失都不得以占位界面或只通过 schema 测试声称完成。
