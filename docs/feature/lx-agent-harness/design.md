# lx Agent Harness 与上下文压缩设计

## 1. 目标与范围

本功能为 lx 页面增加一套接近 Codex harness 的防御性 agent 工作方式，并加入可持久化、可撤销的 `/compact` 上下文压缩能力。

目标包括：

1. 建立名称为 `lx` 的系统提示词，明确工作区边界、工具真实性、先读后改、最小修改、验证结果和错误处理规则。
2. 在每个 turn 的模型上下文中注入受限的电脑环境快照，帮助模型正确选择路径、命令和平台行为。
3. 在执行层对文件工具增加读后版本保护：修改已有文件前必须有对应读取版本，写入时以 CAS 语义核对版本，检测到外部修改则零写入失败。
4. 增加 `/compact` slash 命令。命令只在会话空闲时执行，使用当前会话模型生成摘要，成功后建立可撤销的 compact 检查点。
5. 在消息列表中显示独立的“上下文已压缩”条目；条目默认折叠，复用现有 rewind 提示的时间线、布局、滚动和交互逻辑，并显示实际摘要模型。
6. `/rewind` 能按最新可撤销操作顺序撤销 compact；原始消息保留在数据库，撤销只恢复上下文视图和检查点状态，不重新构造平行撤销系统。

本次不做：

- 不复制 Codex runtime，不引入新的 agent loop。
- 不对任意 shell 脚本增加文件版本保护，不引入 OS 沙箱或命令黑名单。
- 不把环境变量、密钥、令牌、用户目录文件内容或完整进程命令行注入模型。
- 不在渲染层执行 IO 或修改状态。

## 2. 已确认的行为决策

### 2.1 Harness 防御性规则

系统提示词和执行层采用双层防御：提示词指导模型，执行层拒绝不满足前置条件的危险写入。

- 已有文件：修改前必须先通过文件读取工具取得版本 token。
- `edit`、覆盖式 `write`、删除、移动：提交时核对读取 token；文件内容、大小、mtime 或身份与 token 不一致时拒绝写入。
- 新文件：提交时确认目标不存在；目标已出现则拒绝覆盖。
- 检查失败必须零写入，并返回可重试错误，要求模型重新读取后再操作。
- shell 保持现有权限模型；系统提示词建议优先使用文件工具修改代码。

### 2.2 环境快照

每个新 turn 开始时生成一次只读快照，并在该 turn 内固定。快照包含：

- 工作区根目录和当前工作目录；
- 操作系统、架构、终端类型和默认 shell；
- 当前日期、时间和时区；
- 网络可用性状态；
- 受管控后台任务的摘要；
- 工作区范围内的文件系统能力说明。

快照采用结构化、有限长度的文本（可参考 Codex 的 `<environment_context>` 结构），对 XML/控制字符进行转义和长度限制。禁止注入环境变量、凭据、令牌、用户目录内容和完整命令行。

### 2.3 Compact 触发与摘要

- slash 命令为 `/compact`，加入 slash 帮助和命令解析。
- 会话忙碌（流式回复、工具调用或 compact 正在执行）时立即拒绝，不中断当前任务。
- 会话空闲时，读取上一个仍生效 compact 检查点之后至当前最新完整消息的增量范围。
- 使用当前会话已选择的模型发起独立摘要请求。
- 摘要请求不是普通聊天轮次，不作为用户/助手消息追加。
- 摘要正文必须保留用户/助手消息、工具调用结果及关键结论，尤其是文件路径、命令、错误、验证结果、关键决策和未完成事项。
- 系统提示词、环境快照和内部控制字段不重复写入摘要，但由其产生的实际约束和结果必须保留。
- 输出设置明确上限并按优先级压缩；超限或请求失败时整体失败，不静默截断、不写检查点。
- 成功后以事务写入 compact 检查点、上下文指针和消息列表条目；失败时原始历史、上下文指针和折叠状态完全不变。

模型名称必须保存并显示，例如：`上下文已压缩 · 模型：gpt-5 · 10:42`。

### 2.4 消息列表与 Rewind

Compact 条目是独立时间线通知，不进入模型上下文，也不作为普通消息复制选区。默认折叠；选中后沿用现有消息列表的 `Enter`/展开交互切换，展开显示摘要正文、模型、时间和覆盖范围。

撤销采用统一的最新可撤销操作顺序：

1. 先撤销 compact 之后产生的最新普通对话轮次；
2. 没有更晚轮次后，再撤销最近的 compact 检查点；
3. 多次 compact 按从新到旧逐个撤销。

原始消息始终保留在数据库。Compact 的生效状态只由检查点和上下文指针表达；撤销 compact 时恢复此前的上下文指针和完整消息可见状态。不得创建与现有 rewind 文件 scope 平行的撤销系统。

## 3. 数据结构与持久化

### 3.1 Compact 检查点

建议在 chat 数据库增加独立检查点表（名称以现有 schema 命名风格为准）：

- `external_id`：检查点 ID；
- `session_id`、`workspace_id`：会话和工作区归属；
- `sequence`：与消息和 rewind notice 共用时间线编号；
- `base_checkpoint_id`：增量摘要的上一个生效检查点，可为空；
- `start_sequence`、`end_sequence`：摘要覆盖的原始消息范围；
- `summary_text`：摘要正文；
- `model_label`：实际摘要模型显示名；
- `created_at_ms`；
- `state`：`active`、`reverted`；
- `reverted_by_operation_id`：与 rewind 操作关联。

如现有 schema 已有可复用的 notice/operation 表，优先扩展其 kind 和 payload，避免重复保存 session、sequence 和撤销元数据。迁移必须顺延 `SCHEMA_VERSION`，保留旧库升级路径。

### 3.2 内存状态

在 `src/app/state/lx` 增加只读展示所需的 compact 视图和当前生效 checkpoint 引用；状态只由 update/effect 回填，UI 只读。视图至少包含：ID、sequence、模型名、时间、覆盖范围、摘要正文、展开状态。

### 3.3 上下文组装

`src/chat/context::assemble` 在已有预算裁剪之前加入 active compact checkpoint：

```text
system prompt
environment snapshot (fixed for turn)
active compact summary (if any)
messages after checkpoint.end_sequence
```

无 active checkpoint 时保持现有完整消息组装。摘要失败或撤销时回退到原有路径。

## 4. 关键数据流

### 4.1 普通 turn

`slash/session update -> event/chat::assemble_turn_request -> context::assemble -> model request`

环境快照在 turn 创建时注入，文件读版本 token 随工具读取结果回传并登记到本轮工具状态。执行层的 `MutationRecorder` 在提交前验证 token。

### 4.2 `/compact`

`slash parser -> session update -> Compact effect -> event/chat compact handler -> load incremental history -> model summary request -> DB transaction -> app reload -> history render`

DB 事务必须同时完成：检查点写入、sequence 分配、active 指针更新和 compact 展示记录写入。模型请求失败不进入该事务。

### 4.3 `/rewind` compact

`rewind selection -> latest reversible operation lookup -> compact restore plan -> CAS/transaction commit -> reload session view -> history notice`

Compact 撤销不得修改文件 scope。普通 turn 的文件恢复仍使用现有 `RestorePlan`、路径锁、全量预检、补偿写入和 revision 检查。

## 5. 代码落点

- 系统提示词：`src/chat/prompt.rs`，复用现有 `LX_SYSTEM_PROMPT` 和 `platform::environment_summary()` 接点；必要时拆出环境渲染模块。
- Slash 命令：`src/app/state/lx/slash.rs`、`src/app/update/chat/session.rs`。
- Turn 组装：`src/event/chat/mod.rs`、`src/chat/context/mod.rs`。
- Compact effect/事件：沿用现有 `Effect`、`event/chat` 和 `ModelContext`，不复制 agent loop。
- 数据库：`src/db/chat/mod.rs`、`src/db/chat/rewind.rs`、迁移模块和 `src/db/service.rs`。
- Rewind：复用 `src/event/chat/rewind` 的 operation/revision 事务，增加 compact 恢复分支。
- 状态：`src/app/state/lx/chat`、`src/app/state/lx/rewind.rs` 附近。
- 消息列表：`src/ui/main_content/lx/history.rs`、`history/rewind.rs`、`history_hit.rs`；保持 `layout` 与 `render` 共用行数口径。
- 文案：`src/ui/text.rs`；compact 文案与 rewind 文案分离。

## 6. 防御性不变量

1. 渲染函数不做 IO、不改状态。
2. compact 检查点提交具备幂等性；重复完成回执不得重复写入或重复计数。
3. compact 失败不改变任何持久化上下文状态。
4. rewind 目标以稳定 ID、session revision 和 checkpoint ID 冻结；目标变化时零写入。
5. 原始消息只追加、不删除；撤销 compact 只切换 checkpoint 状态和上下文指针。
6. 文件写入必须经过路径锁、版本核对和全量预检；任一路径冲突则整批零写入。
7. `history::layout` 与 `history::render` 对 compact 条目使用同一行数计算，折叠和窄屏换行不得导致滚动漂移。

## 7. 验收标准

- `/compact` 出现在 slash 帮助中，忙碌时明确拒绝，空闲时成功执行。
- 成功条目显示实际模型名、时间、覆盖范围，可折叠/展开；交互与 rewind 条目一致。
- 成功后下一轮模型只收到摘要和检查点后的消息；原始消息仍可从数据库读取。
- `/rewind` 可按最新操作顺序撤销普通轮次和 compact；连续 compact 可逐个恢复。
- 摘要失败、超限、并发修改或目标 revision 变化时不产生部分状态。
- 文件工具在缺少读取 token、token 失效或新文件目标已存在时拒绝写入。
- 环境注入不包含凭据和未授权目录内容，并在 turn 内保持稳定。
- 不破坏现有 rewind 文件恢复、session reload、消息滚动和折叠测试。

