# lx Agent 命令、进程交互与计划工具：设计草案

> 状态：**已实施**（用户按指令批准直接执行）。实施记录见 `task.md` §10。
> 创建日期：2026-10-19。
> 参考仓库：`/Users/yonah/projects/agent/codex-main`。
> 实际落地与本文档的差异（以代码为准）：
>
> - `chat/process/` 的实际结构是 `mod.rs`（契约）、`manager/mod.rs`（生命周期）、
>   `manager/spawn.rs`（spawn/supervisor）、`output.rs`（有界缓冲与安全文本化）；
> - 进程起始记录在 **工具结算成功后** 登记（而不是 spawn 之后立刻登记），避免
>   BeginTool 门槛未通过时留下孤儿行；
> - `write_stdin` 的 `terminate` 是结果里的 `terminate` 标志（不是第四个工具）；
> - 计划不建表，从最后一条成功 `update_plan` 结果 JSON 恢复（与 §9.3 一致）；
> - 运行上下文摘要并入**本轮最后一条用户消息**（`[local runtime]` 段），
>   不新增 call/result 对。

## 1. 目标与完成标准

为 lx Native Agent 新增 `exec_command`、`write_stdin`、`update_plan`，同时提供对应的 TUI 组件，让用户看清命令、输出、进程状态和任务进度。

现有 `read/search/edit/write` 继续作为文件工具；本期不增加 subagent。参考 Codex 的工具职责和交互语义，在 lx 现有架构内实现，不复制整个 Codex core。

完成标准：

1. 模型能启动命令、获取输出、识别退出码，并继续推理。
2. 尚未退出的命令返回进程会话 ID；同一聊天会话后续轮次可用该 ID 读取输出或写入 stdin。
3. 进程退出与工具调用完成是两套状态；后台进程不会因工具已返回就显示为退出成功。
4. `update_plan` 原子替换当前计划，正确展示待办、进行中、完成三类步骤。
5. 三种新增工具均接入已有 pending → running → 结果持久化握手、模型历史回放和历史展示。
6. 关闭所属会话资源时清理进程；重启仅恢复历史，不自动重跑命令。
7. 新组件有宽、窄、极矮尺寸及实时状态变化的 buffer/geometry 验证；真实终端观感由用户验收。

## 2. 已确认决策

| 编号 | 决策 | 用户确认内容 |
| --- | --- | --- |
| D1 | 首批工具 | `exec_command`、`write_stdin`、`update_plan`。 |
| D2 | 界面范围 | 必须设计并实现对应组件展示。 |
| D3 | 执行权限 | 命令直接执行，无逐次确认弹窗；沙箱后续独立考虑。 |
| D4 | 进程生命周期 | 同一聊天会话内可跨多轮保留。 |
| D5 | 清理时机 | 切换会话、关闭窗格或退出 lx 时清理对应进程。 |
| D6 | 重启行为 | 只恢复历史记录，不自动重启命令。 |
| D7 | 开发门槛 | 先详细设计和任务文档，用户审核确认后才实施。 |
| D8 | 工具展示 | 聊天内紧凑卡片，按需展开详情；命令显示状态、命令及最近 6 行输出，交互显示本次输入或新增输出，计划直接显示步骤和完成进度。 |
| D9 | 停止本轮 | 停止本轮新启动的进程，以及当前正在等待或交互的进程；保留此前启动且本轮未操作的服务。切换会话、关闭窗格或退出 lx 时清理所属全部进程。 |
| D10 | 交互模式 | 同时支持普通管道和独立 PTY；默认 `tty: false`，显式 `tty: true` 启用模拟终端，使用 write_stdin 交互，不占用现有终端窗格。 |

工作目录不是权限隔离：命令以当前用户权限运行，可以访问工作区之外和网络。D3 不代表允许本次开发自行执行 Git 分支、暂存或提交操作；开发期间遵守已有 Git 约束。

## 3. 边界确认收口

已按 grill-me 每轮一个决策完成六轮确认：工具范围、执行权限、进程生命周期、组件展示、停止范围和 PTY 能力。没有仍待单独确认的高风险产品分支。

| 原问题 | 确认结果 | 对应记录 |
| --- | --- | --- |
| P1：组件展示 | 紧凑卡片与按需展开。 | D8 |
| P2：停止范围 | 本轮新进程及当前交互进程，不误杀历史服务。 | D9 |
| P3：PTY | 普通管道和显式 PTY 均支持。 | D10 |

§6–§11 的具体参数、资源上限、组件操作和接口属于本次完整设计提案，随文档一并审核；它们不是另外要求用户逐项回答的问卷。设计审核完成前不进入实现。

## 4. 事实依据与检索记录

已使用 codegraph 与 codebase memory 分析两个仓库，并用索引覆盖检查区分可依赖的结果与需要直接核对的源文件。

### 4.1 lx 当前代码事实

| 路径/符号 | 当前事实 | 对方案的约束 |
| --- | --- | --- |
| `src/chat/tools/schema.rs::available_tools` | 静态表声明四个文件工具。 | 在同一工具规格入口增加三个工具，不维护第二套协议表。 |
| `src/chat/tools/mod.rs::execute` | 同步分发 `read/search/edit/write`。 | 长期进程不得塞入该同步文件执行分支持续等待。 |
| `src/chat/agent_loop/mod.rs::ToolExecutor` | 注入式异步执行接口；生产 `FileTools` 经 `spawn_blocking` 执行文件 IO。 | 扩展生产执行器，文件路径仍复用既有实现。 |
| `src/event/chat/host.rs::EventHost` | BeginTool/FinishTool 经事件循环做 DB、状态和回执。 | 新工具仍受相同执行票据门槛约束。 |
| `src/app/state/lx/chat/steps.rs` | 工具与内容块按 position 形成同一时间线；ToolSummary 偏向文件字段。 | 添加有类型的命令/计划展示数据，避免把进程 ID 塞进 path。 |
| `src/ui/main_content/lx/history/tool.rs::Kind` | 当前只枚举 Read/Search/Edit/Write/Other；标题和详情分两层。 | 新增专属组件分派，不能依赖 Other 显示工具名就算完成。 |
| `src/ui/main_content/lx/history.rs` | 布局、绘制和历史滚动需要统一行数口径。 | 卡片输出及展开状态必须参与同一布局计算。 |
| `src/db/chat/tools.rs` | 参数、结果均有 1 MiB 上限；工具终态校验幂等和冲突。 | 后续进程输出不能修改原工具结果来伪装第二次结算。 |
| `src/chat/context/mod.rs` | 已有结构化历史回放及单条结果 64 KiB 截断口径。 | 新结果复用回放上限，显式标识截断。 |
| `src/pty/mod.rs::PtySession` | 管理每窗格 shell、读线程、写入、resize、kill。 | 可复用底层能力，但不把 agent 进程伪装成用户终端窗格。 |
| `src/db/mod.rs::SCHEMA_VERSION` | 本次核对值为 10。 | 若实施前没有其他迁移，新增版本为 11；开工时再次核对。 |
| `src/input/mod.rs::lx_input_key` | `^t` 切换思考；`^o` 和 Alt+上下当前未映射。 | 卡片操作只在 Native 聊天上下文增加映射，不复用已占用编辑键。 |

注意：codegraph 返回的是当前源代码；部分旧 feature 文档开头仍描述实施前状态，不能据此判断当前没有 agent loop。codebase memory 对 Codex 的覆盖元数据存在缺失和过期，参考相关符号时结合 codegraph 当前源码；`write_stdin.rs` 已直接核对。

### 4.2 Codex 参考点

| 参考路径（相对 codex-main） | 采用的设计 |
| --- | --- |
| `codex-rs/core/src/tools/handlers/unified_exec.rs` | 区分命令参数、等待时间、总超时和 shell 配置。 |
| `codex-rs/core/src/tools/handlers/unified_exec/exec_command.rs` | 命令执行工具与会话化进程管理分开。 |
| `codex-rs/core/src/tools/handlers/unified_exec/write_stdin.rs` | `session_id`、可空 `chars`、等待窗口；空写用于获取后续输出。 |
| `codex-rs/core/src/tools/handlers/plan.rs` | 更新计划通过事件提交；工具回复成功。 |
| `codex-rs/core/src/tools/handlers/plan_spec.rs` | 全量 plan，步骤状态为 pending/in_progress/completed，最多一个进行中。 |

不引入 Codex 的审批、sandbox_permissions、插件、远程环境和多 agent 体系。工具名保持一致，不承诺完全兼容 Codex 所有参数。

### 4.3 第三方接口依据

通过 Context7 核对 [Tokio Child](https://docs.rs/tokio/latest/tokio/process/struct.Child.html) 和 [Command::kill_on_drop](https://docs.rs/tokio/latest/tokio/process/struct.Command.html#method.kill_on_drop)：丢弃句柄默认不会停止进程，kill_on_drop 只作为兜底，正常路径必须显式停止并 wait/reap。异步 kill 与单发停止信号不是同一件事。

当前 Cargo.toml 已有 tokio 1.53.1 与 portable-pty 0.9；普通管道实现需要启用现有 tokio 的 process feature。PTY 复用已有依赖，不新增第二套终端库。最终实现以锁定版本接口为准，文档示例不直接当成可编译代码。

## 5. 架构与数据流

```text
模型输出完整工具参数
  -> agent loop 校验和串行调度
  -> BeginTool：pending 入库、可见状态、running 回执
  -> AgentTools 分派
       文件工具：现有同步执行器 + spawn_blocking
       exec_command/write_stdin：会话持有的 ProcessManager
       update_plan：校验计划，结果内携带完整快照
  -> FinishTool：持久化结果、更新展示状态、回执
  -> 结构化工具结果进入下一次模型请求

ProcessManager 后台输出/退出
  -> 带会话运行代次的 ProcessEvent
  -> 事件层校验归属、更新独立进程记录
  -> app 纯状态更新
  -> ui 只读渲染原命令卡片
```

进程归属于聊天会话的运行实例，而非某一次模型请求。当前 `RequestKey` 适合过滤旧轮次网络和工具回执；跨轮进程事件必须另带稳定归属键，不能被下一轮 generation 变化直接丢弃。

建议归属结构（设计示意，不是已有 API）：

```rust
struct ProcessOwner {
    workspace: PathBuf,
    chat_session_id: String,
    runtime_epoch: String,
}

struct ProcessKey {
    owner: ProcessOwner,
    process_session_id: u64,
}

enum ProcessState {
    Starting,
    Running,
    Exited { code: Option<i32> },
    Stopped,
    TimedOut,
    Interrupted,
    Failed { code: String },
}
```

`process_session_id` 是模型见到的 `session_id`；它不是聊天会话 ID，也不是 OS PID。所有读取、写入和停止都先检查 owner；旧运行代次和其他会话不能操作该进程。已关闭运行实例的迟到事件不允许复活卡片。

同一聊天会话被多个窗格打开时，推荐只允许一个活动进程宿主；其他窗格展示相同持久化快照，不创建第二套进程管理器。实施前核对已有会话占用检查，优先复用。焦点切换本身不等于切换会话，不清理后台进程。

## 6. 工具契约草案

三个工具均使用严格字段白名单和 `additionalProperties: false`，运行时重新校验。以下参数属于待完整文档审核的实现建议。

### 6.1 exec_command

| 参数 | 类型 | 建议语义 |
| --- | --- | --- |
| `cmd` | string，必填 | 原样交给选定 shell 执行；空白命令拒绝。 |
| `workdir` | string，可选 | 默认冻结的工作区 cwd；相对路径仍基于该 cwd；不调用全局 set_current_dir。 |
| `shell` | string，可选 | 默认平台提供的 shell；显式值必须能启动。 |
| `login` | bool，可选 | 默认 false，避免登录配置隐式改变环境和启动耗时。 |
| `tty` | bool，可选 | 默认 false；true 使用专属 PTY，按 D10 执行。 |
| `yield_time_ms` | integer，可选 | 本次工具等待输出/退出的窗口，不代表进程总寿命。 |
| `timeout_ms` | integer，可选 | 显式设置才限制命令总寿命，超时后执行进程清理。 |
| `max_output_tokens` | integer，可选 | 返回模型的文本预算，另受硬字节上限约束。 |

短命令退出后返回 exit_code；长命令等待窗口到期后返回 running 和 session_id。退出码非零是命令已执行且失败，不是工具协议失败；结果必须让模型和 UI 都识别失败。启动失败、参数非法和归属错误则为工具错误。

已启动命令非零退出时，执行结果仍包含完整输出/退出信息，但回传模型的 is_error 为 true，工具视图显示失败，不能只返回 Completed。超时和显式停止同样携带真实输出与原因。等待窗口到期且进程仍运行则工具调用正常完成，关联进程仍 Running。

普通模式明确捕获 stdin/stdout/stderr，不能继承 lx 自己的终端输出。stdout/stderr 按收集器观察到的顺序组成带来源的输出片段，不声称操作系统两个管道具有严格全局时间顺序。PTY 输出天然合流，使用单独来源标记。

PTY 初始大小固定为 120 列、40 行，窗格缩放不改后台程序终端尺寸；展示再按当前窗格换行。PTY 支持读取提示和写入回答，不在聊天卡片内实现 vim/top 一类全屏应用的终端仿真。真实退出状态来自 child.wait，而不是读线程 EOF；输出关闭但子进程未退出时仍记录 Running。

结果示例：

```json
{
  "kind": "exec_command",
  "session_id": 101,
  "process_state": "running",
  "exit_code": null,
  "output": "Server ready on port 3000\n",
  "truncated": false,
  "output_start": 0,
  "output_end": 26
}
```

示例偏移仅说明字段用途；实际偏移按原始输出字节序列计算，不能用字符数或行数代替。

### 6.2 write_stdin

| 参数 | 类型 | 建议语义 |
| --- | --- | --- |
| `session_id` | integer，必填 | 当前聊天运行实例拥有的进程会话 ID。 |
| `chars` | string，可选 | 默认空串；非空时写入 stdin，不自动补换行。 |
| `terminate` | bool，可选 | lx 扩展，默认 false；true 时按进程 ID 显式停止，不能同时传非空 chars。 |
| `yield_time_ms` | integer，可选 | 等待新输出或退出的最长窗口。 |
| `max_output_tokens` | integer，可选 | 与 exec_command 同一文本预算和硬上限。 |

空串仅获取后续输出；`"yes\n"` 明确包含提交换行。PTY 下 `\u0003` 交给终端纪律处理；普通管道下不能把这个字节假装成 SIGINT，停止应走进程管理器的显式清理路径。

每次调用返回尚未交给模型的输出区间和进程当前状态。UI 的实时预览不得消费模型读取游标。输出从缓冲中淘汰时返回 dropped_bytes/truncated；不得静默跳过。退出后允许读取剩余输出；非空写入退出进程返回 process_exited。终态结果保留有界时间或直到所属会话关闭。

`terminate: true` 让模型可以主动关闭上一轮留下的普通管道服务，无需暴露 OS PID 或新增第四个工具。它使用同一归属校验、温和停止/强制终止/reap 流程，返回停止后的状态和最后输出；重复停止已退出进程只返回已知终态，不再发送信号。

单次输出预算不足时，仅推进到实际返回的原始输出区间，剩余部分留给下次读取；缓冲淘汰后从仍保留的起点继续并报告丢失。不得先把游标推进到末尾再裁掉正文。相同 owner 的进程交互串行化，避免重叠读取跳过或重复消费。

### 6.3 update_plan

输入示例：

```json
{
  "explanation": "Implementation finished; checking behavior.",
  "plan": [
    { "step": "Implement command tools", "status": "completed" },
    { "step": "Verify process cleanup", "status": "in_progress" },
    { "step": "Review terminal display", "status": "pending" }
  ]
}
```

每次全量替换，不做文本匹配和隐式增删。最多一个 in_progress；step 去空白后不得为空，未知状态拒绝。建议允许空数组清空计划，但不能将清空解释为所有旧步骤完成。

成功结果携带规范化的完整计划快照和修订标识，模型反馈可简短显示 Plan updated。只有 FinishTool 成功后该快照才成为当前已提交计划。失败保持旧计划，不能先更新 UI 再假装持久化成功。计划属于聊天会话，可跨用户轮次延续。

## 7. 建议资源默认值

以下是工程提案，等待完整文档审核，不是用户已经逐项确认的数字。

| 项目 | 建议值 | 说明 |
| --- | --- | --- |
| exec 初次等待 | 默认 1000 ms，上限 10000 ms | 尽快把后台会话交回模型，UI 持续接收输出。 |
| stdin 等待 | 默认 1000 ms，上限 10000 ms | 允许零等待立即获取已有输出；不靠高频轮询刷屏。 |
| 单次模型输出 | 默认 4000 近似 token，硬上限 64 KiB 文本 | 无新增 tokenizer 依赖；明确使用项目现有估算方式。 |
| 每进程内存输出 | 尾部 256 KiB | 持续 drain，满后淘汰旧数据并累计丢弃字节数。 |
| 每进程持久化输出 | 有界尾部 256 KiB | 卡片展开只能显示保留内容，不能声称完整日志。 |
| 同一聊天运行进程数 | 8 | 超限返回 resource_limit，不自动杀已有服务。 |
| 每次 stdin 输入 | 64 KiB | 写入采用有界异步队列，处理半写和 BrokenPipe。 |
| 计划步骤 | 最多 32 项，每项最多 512 字符 | 限制历史和布局开销；explanation 最多 2048 字符。 |
| 默认卡片输出 | 最近 6 个显示行 | 无输出时显示 Waiting for output，状态仅从进程事件推导。 |
| 输出 UI 合并 | 最多每 100 ms 提交一次有界增量 | 终态立即提交，禁止每个字节投递无界 AppEvent。 |
| 停止升级等待 | 温和停止后最多 1000 ms，再强制终止并 reap | 具体信号和进程树处理留在 platform。 |
| 已退出进程快照 | 每会话最多 32 项，交互保留 10 分钟 | 超期或超量回收管理器句柄，持久化历史仍保留；后续操作返回 process_expired。 |
| 合并输出单批 | 每进程最多 16 KiB，事件通知每进程最多一个在途 | 输出放在有界缓冲，通知只表示可读取，不能塞满无界事件队列。 |

命令默认无总寿命超时，以支持开发服务；用户或模型可设置 timeout_ms。等待窗口到期只结束本次工具等待，不杀进程。

timeout_ms 取值 1 ms–24 小时；yield_time_ms 取值 0–10000 ms；max_output_tokens 取值 1–16000。非法范围拒绝，不静默修正。总进程上限另设为 32 个运行进程，避免多窗格各自 8 个累积失控；达到任一上限均返回 resource_limit。模型文本预算沿用项目字符估算，不声称是真实 tokenizer 计数。

256 KiB 是原始输出缓冲上限；UTF-8/ANSI 增量解码器另仅保留有界的未完成字符/控制序列，不按每次 chunk 独立替换乱码。未知或非法字节在展示/模型文本中做显式可见转换，保留原始偏移语义。控制序列达到长度上限则丢弃该序列并标记，不无限等待终止符。

## 8. 生命周期、取消与恢复

| 触发 | 行为 |
| --- | --- |
| 正常结束一轮模型回答 | 保留仍运行的所属进程。 |
| 下一轮发送消息 | 复用同一运行实例和进程会话 ID。 |
| 只切焦点或切模型 | 保留进程，工具参数仍依据原创建上下文。 |
| `/new`、打开另一个会话 | 关闭旧运行实例，清理其进程并记录终态。 |
| 关闭拥有运行实例的窗格 | 清理对应进程；其他窗格不继承旧 stdin 权限。 |
| 退出 lx | 停止新调用，清理所有进程、落库可获取的终态，再按原顺序恢复终端。 |
| 命令显式超时 | 标记 timed_out，清理进程树，保留尾部输出。 |
| 工具结束落库失败 | 不重试启动或写 stdin；本轮失败，并清理无法可靠登记的新进程。 |
| 异常退出后启动 | 遗留 starting/running 标 interrupted，不用历史 PID 重新连接或杀进程。 |

停止范围按 D9 执行：记录本轮新建进程集合，以及此刻正在等待或交互的进程；收到停止请求时清理两者的并集。本轮曾经读取、但当前已不再等待或交互的旧服务不因这次停止而被清理。区分“停止本轮任务”与“关闭聊天运行实例”：后者清理所属全部进程。现有 Esc 单击待命、双击停止语义保持，由相同停止行为触发后台清理。

清理必须幂等，覆盖启动中、等待中、已退出、stdin 半写及重复关闭。创建进程和登记 owner 之间的失败必须由资源守卫清理。不能仅 drop future，因为后台任务和子进程可能继续运行。

清理失败不把进程标为 Stopped：保留 Interrupted/错误原因并记录 tracing，允许重复清理。关闭流程立即撤销运行实例的调用权限，后台等待清理回执；事件循环继续响应输入和渲染。正常退出在终端恢复之前等待所有清理请求完成，单批最长 3 秒后记录未确认项，不能为持久化失败无限挂住退出。

进程树清理通过 `platform/` 封装 OS 行为；普通管道与 PTY 都测试子进程。显式脱离进程组的守护进程不保证被完整追踪，异常强杀 lx 也不声称可以优雅清理所有后代；恢复时仅报告已知状态，不伪造已终止。

### 8.1 关键执行接口

以下为设计伪代码，说明状态归属和顺序，实际类型根据现有 trait 最小调整：

```rust
struct TurnExecution {
    cancel: watch::Receiver<bool>,
    created: BTreeSet<u64>,
    active_interaction: Option<u64>,
}

impl AgentTools {
    async fn execute(&self, call: &ReadyToolCall) -> ToolExecutionResult {
        match call.name.as_str() {
            "exec_command" => self.processes.exec(call, &self.turn).await,
            "write_stdin" => self.processes.interact(call, &self.turn).await,
            "update_plan" => validate_plan_snapshot(call),
            _ => self.files.execute(&self.cwd, call).await,
        }
    }
}

// 用户停止本轮时，快照和撤权先于 await；结束过的旧轮询不在 active 中。
let targets = turn.created.union_with_active(turn.active_interaction);
processes.stop_owned(targets).await;
```

当前 ToolExecutor 的 execute 没有 cancel 参数；优先让生产 AgentTools 持有本轮取消接收端和 TurnExecution，保持测试可注入，不为了三个工具重写全部 agent loop。BeginTool 票据由原调用链先取得，执行器入口不能被绕过来 spawn。FileTools 的阻塞写入取消语义保持既有约束，不声称本次新增可回滚文件操作。

## 9. 数据持久化与回放

### 9.1 工具历史

继续使用 agent_tool_call 保存三种工具参数、结果与 position。调用在等待窗口内完成时结算一次；后台输出或之后退出不能改写已经返回模型的原始工具结果。write_stdin 每次调用保留独立记录，重放顺序使用既有消息顺序和 position。

### 9.2 独立进程记录

建议增加 agent_process 元数据表，以保存原 exec 工具返回后的进程变化：

| 字段 | 用途 |
| --- | --- |
| chat_session_id、origin_call_id | 关联聊天会话和起始 exec 工具；外键级联删除。 |
| runtime_epoch、process_session_id | 运行实例内进程唯一身份；不接受 OS PID 作为恢复入口。 |
| state、exit_code、stop_reason | 当前进程状态和退出原因；未知退出码保持 NULL。 |
| command、cwd、tty | 展示和重载所需参数快照，按限制长度保存。 |
| output_tail、output_start、output_end、dropped_bytes | 有界输出快照及截断证据。 |
| started_at_ms、ended_at_ms、revision | 时长及拒绝乱序事件所需版本。 |

不为每个输出字节建一行。合并输出快照后台受控落库，退出立即刷新；工具原始结果不可变。新增迁移版本取实施时真实 SCHEMA_VERSION 的下一版，不照抄旧 feature 文档中的版本。

进程输出快照建议每 1000 ms 最多落库一次，终态不等待该周期。所有新快照写入经事件层发给有界串行 DB 后台任务，每个进程最多一个待合并快照，避免在 select/render 路径反复写 256 KiB 文本。单次会话加载批量读取进程记录并与调用按 ID 合并，不产生每卡片一次查询的 N+1。

输出按有界原始字节保存时使用 BLOB，持久化范围明确为 `[output_start, output_end)`；前端展示在加载时生成安全文本，既有工具结果 JSON 仍保存返回模型的文本。revision 用单调整数分配；运行代次不一致和较旧 revision 更新零行，不能覆盖终态。来源标记可随有界片段编码保存，但禁止借此绕开 256 KiB 总限额。

原 exec 工具调用按已有流程先 pending 入库，再登记 starting 进程记录，成功回执后才 spawn；启动失败时进程记录记失败并结算工具。spawn 已成功而后续落库失败时必须停止新进程，禁止盲目重试。进程终态更新带 revision 和 owner 检查。

### 9.3 计划与重载

计划初期不单独建表：从本会话按序最后一条成功 update_plan 结果恢复完整快照。旧计划卡片显示它当时提交的快照，不随最新计划倒改历史。失败、pending、interrupted 更新均不覆盖最后成功计划。

### 9.4 模型上下文

工具 call/result 原始结果按现有结构化历史回放。下一轮另提供有界的存活进程清单及最新计划摘要，避免历史裁剪后模型忘记 session_id；快照属于本地运行上下文，不能伪装为新的工具结果。进程输出仍由 write_stdin 按需获取，不把每次后台输出都自动提交模型或触发新请求。

## 10. 对应 TUI 组件方案（展示方式已确认）

### 10.1 通用规范

组件放在聊天时间线，沿用现有工具标题、层级连接符、语义颜色和空行节奏。常规卡片不新增大边框；状态同时有文字或标记，不能只靠颜色。所有文案集中 `src/ui/text.rs`，界面示例使用英文。

渲染不读数据库、进程或文件，不常规解析巨型 JSON；事件结算和历史加载时生成有类型的视图。输出内容在进入显示视图时去除危险控制序列，不把子进程的 OSC、CSI 或光标移动写到真实终端。

### 10.2 CommandCard：exec_command

```text
■ Run command                        Running · 12s [details]
  │  $ cargo test chat::tools
  │  … +21 more lines
  │  test read::tests::empty_file ... ok
  └─ test result: ok. 24 passed
```

首行进程状态优先，第二行起是**树形体**：命令、cwd、输出共用一根竖线 `│`，最后一行用
`└─` 收口（与 prompt list、文件工具的 `  └─` 同一套直角语法）。两条前缀等宽 5 列，
内容一律从第 5 列起，因此树干不会错位。命令行带 `$ ` 提示符，与输出区分。

折叠只显示最近三行输出，被省略的行数用 `… +N more lines` **显式**标在树体里——绝不
静默丢行（对齐 codex 的 `… +N lines` 口径）。cwd 只在展开时出现。

默认最多三行输出预览；后续后台输出更新原卡片，不无限追加时间线步骤。工具返回而进程
仍运行时，标题仍显示 Running，**方块继续呼吸**（不因工具已结算而静止）；退出码 0 显示
Exited 0，非零显示 exit N；停止、超时、中断分别显示。运行不足一秒不追加时长（避免
`Running · 1ms` 噪声与逐帧抖动）。

详情原位展开，显示完整保留的命令、cwd、起止时间、退出原因和有界输出。超过保留范围明确显示 Earlier output discarded。折叠显示最近三行，展开允许查看全部保留文本，滚动仍由外层历史管理，不额外引入日志浮层。

鼠标点击标题或 `[details]` 展开，再次点击标题或 `[collapse]` 收起；点输出文本不触发折叠。纯键盘入口是 Native 聊天中的 `^o`：切换当前选中工具详情，未选中时选最近一张可展开卡片。Alt+上下按时间线选择上一/下一张可展开卡片，保证选中标题可见；输入草稿及光标保持。已有浮层、补全或命令面板打开时继续保持原优先级，不处理这组卡片操作。不把上述键写进后台 stdin。

展开状态仅存于窗格内存，用 `(session_id, message_id, call_id)` 定位；不依赖易变化的数组下标，重启或切换会话后默认折叠。选择标记复用现有样式，鼠标悬停不自动移动外层滚动位置。运行时输出只更新数据，显示行生成按输出 revision/可用宽度/展开状态失效，避免每帧全量解析日志。

### 10.3 ProcessInteractionCard：write_stdin

```text
■ Send input                        Running [details]
  │  $ cargo test chat::tools
  │  stdin: y\n
  └─ No new output
```

与命令卡片共用**同一套树形语法**：命令、`stdin:` 输入预览、本次输出依次排在同一根
`│` 下，最后一行用 `└─` 收口。输入永远排在输出之前（先看发了什么，再看回什么）。

空 chars 显示 Read output，非空显示 Send input，terminate 显示 Stop command。关联原命令，细节中可显示进程会话 ID；技术 ID 不作为主标题。输入预览限长且转义换行，不将完整 stdin 内容自动重复展示到正文。无新增输出显示 No new output；运行状态变化仍要显示。详情使用 CommandCard 相同的原位展开操作，展示该次保留结果，不读取最新进程缓冲代替快照。

每次真实工具调用保留时间线记录，但后台输出事件不额外生成 write_stdin 卡片。该卡片保留调用时的输出快照；原 CommandCard 则反映进程最新状态，不能把所有历史交互卡片都改成相同最新日志。

### 10.4 PlanCard：update_plan

```text
■ Plan                                 1/3 completed
  │  [x] Implement command tools
  │  [>] Verify process cleanup
  │  [ ] Review terminal display
  └─ Implementation finished; checking behavior.
```

三张卡片（命令 / 交互 / 计划）共用同一套树形读法：标题是 `■ 动词`，正文全部挂在
`  │  ` 下，**整张卡片只有一根树干**——最后一行（解释行，或没有解释时的最后一条步骤）
用 `  └─ ` 收口。

完成项次要色，进行项强调，待办普通文本；三种状态都有 ASCII 标记。计数只认 completed，不把 in_progress 算完成。解释可省略；空计划显示 Plan cleared。计划更新不表示 lx 自动执行了某个步骤，列表不提供用户点击改状态的功能。

### 10.5 窄屏、滚动和交互

1. 收缩顺序：隐藏 cwd/时长等次要信息 → 缩短命令 → 压缩预览；保留状态标记和关键退出信息。
2. 中文、emoji、组合字符按终端列宽处理，不能按字节裁切。单行超长输出按视图宽度换行后计数，预览上限按显示行计。
3. 历史滚动统一由外层管理，卡片及原位展开详情都不嵌套滚轮区域。
4. 用户上滚后新输出不抢回底部；跟随最新时按实际布局保持底部可见。展开/折叠需要保持可见锚点。
5. 输入草稿与编辑光标保留；卡片选择/详情操作不把按键直接转发到后台 shell。
6. 复用既有 loading 帧和状态样式，不增加装饰动画；键盘触发立即反馈。

### 10.6 视图模型建议

```rust
enum ToolDetailView {
    File(FileToolDetailView),
    Command(CommandView),
    ProcessInteraction(ProcessInteractionView),
    Plan(PlanSnapshotView),
    Unknown,
}

struct CommandView {
    process_session_id: Option<u64>,
    command: String,
    cwd: String,
    state: ProcessStateView,
    output_preview: Vec<String>,
    truncated: bool,
}
```

上述类型是职责示意。实施时优先复用现有文件摘要类型，以最小字段迁移为准；不为文件工具强造一套平行展示模型。纯 layout/rows 与 render 共用行生成口径，实际接口根据现有 history 模块确定。

### 10.7 状态呈现矩阵

| 情况 | CommandCard | 模型结果 | 历史交互卡片 |
| --- | --- | --- | --- |
| 参数尚未闭合 | 仅显示接收中，不解析半截 cmd | 不执行 | 无输出占位 |
| 已 pending/running，尚未 spawn | Starting | 等待真实结果 | 不伪造 session_id |
| 本次工具已返回，进程仍活跃 | Running + 最新输出 | session_id + running | 保留本次快照 |
| 退出 0 | Exited 0 | exit_code=0、is_error=false | 保留调用时状态 |
| 非零退出 | Failed · exit N | exit_code=N、is_error=true | 后续轮询可观察失败终态 |
| 显式停止 | Stopped | 原因及剩余输出 | Stop command 的本次结果 |
| 显式超时 | Timed out | timeout 原因、is_error=true | 本次结果如实反映 |
| lx 异常恢复/清理未确认 | Interrupted | 不宣称进程已成功结束 | 历史结果不改写 |
| 输出被淘汰 | 状态照常 + Earlier output discarded | truncated/dropped_bytes | 保留该次截断证据 |

布局统一从安全视图生成行，再由同一行序列计算高度和绘制。宽度为零立即返回零行；极窄时优先状态标记，其次状态词，最后命令。时长由事件 tick 提供 now_ms，不在 ui 获取时钟。交互卡片详情的 stdin 文本同样受保留上限，不直接绘制控制字符。

## 11. 模块归属与预计改动

| 范围 | 职责 |
| --- | --- |
| `src/chat/tools/` | 三工具 schema、参数、规范化结果、模型输出格式。 |
| `src/chat/agent_loop/` | 生产执行器分派、运行上下文注入、取消和工具回执。 |
| `src/chat/process/`（拟新增） | 会话进程管理、输出缓冲、stdin、生命周期；仅在实施时创建实际模块。 |
| `src/pty/` | PTY 句柄持有、读写和真实 wait；按职责抽取可复用底层能力，避免反向依赖 chat。 |
| `src/input/`、`src/event/mouse/` | Native 内 ^o/Alt+上下与卡片命中分派，保持既有面板优先级。 |
| `src/platform/` | shell 启动差异、进程组/树终止和 OS 门控。 |
| `src/event/chat/` | 运行实例宿主、进程事件过滤、DB/状态握手、关闭清理。 |
| `src/app/state/lx/chat/`、`src/app/update/chat/` | 进程和计划纯视图、展开状态及纯状态转换。 |
| `src/db/` | 进程表迁移、批量快照、归属检查、终态和启动恢复。 |
| `src/chat/context/` | 三工具回放、存活进程和计划的有界运行上下文。 |
| `src/ui/main_content/lx/history/` | 三种组件及布局，必要时按职责拆分 tool.rs。 |
| `src/ui/text.rs`、既有样式及 gallery | 统一文案、复用状态样式、代表性样例。 |
| `docs/standards/` | 实施完成后同步真实目录、依赖和组件规则。 |

chat 不依赖 app/ui/event；回调只传纯数据。PTY 实现留在 pty，因此新增必要的 `chat -> pty` 依赖，只使用 pty 的公开通用命令会话接口；pty 不反向导入 chat，原每窗格 PtySession 行为保持。app 不执行 IO；ui 不修改状态。单文件上限 1000 行，拆分后子文件约 500 行。不得为沙箱、subagent 或插件预建空抽象。

## 12. 验证与验收

精确自动校验覆盖：

- 参数/schema 与三种结果的模型格式、JSON、视图往返；现有文件工具保持行为。
- 短命令、非零退出、长命令返回 ID、跨轮 stdin/空读、普通管道和选定 PTY 能力。
- stdin 半写、BrokenPipe、无效 ID、跨会话访问、过期运行代次、取消与启动竞态。
- stdout/stderr 同时大量输出不死锁；环形缓冲截断有证据；UTF-8 跨块和 ANSI 控制序列安全处理。
- 等待窗口不终止进程；显式超时清理；关闭会话和正常退出清理进程树并回收。
- DB pending/running 回执前不 spawn/不写 stdin；结果落库失败不重复执行。
- 进程状态记录与工具终态独立；退出事件乱序、重复提交和恢复幂等。
- 计划全量替换、最多一个进行中、清空、失败保留旧值、重载恢复。
- 当前支持的各 transport 工具声明、call/result 续传、混合文件/命令/计划历史。
- 三种组件宽/窄/极矮、长命令、中文、无输出、大输出、所有终态、展开后几何与滚动一致。

执行受影响模块的 cargo test、cargo fmt 和项目规定 clippy；实现前从模块路径确定过滤器，不用占位测试名声称通过。提交前按项目规范处理全量门槛，本次不自动提交。若改动 tui 恢复路径，必须保留恢复顺序并运行 tui::tests；优先通过事件层清理减少终端生命周期改动。

人工验收：用户运行 cargo run，查看长测试、开发服务跨轮、命令退出/失败、计划更新、窄窗格和日志展开。真实密钥和生产服务不作为 fixture。

## 13. 风险与已知限制

- 无沙箱是已确认边界，文件/网络副作用不能由停止操作自动回滚。
- 进程崩溃或 lx 被强杀可能留下无法优雅清理的后代；不能用历史 PID 盲杀复用后的系统进程。
- 输出保留有界，详情只能显示保留尾部；丢弃必须显式提示。
- PTY 能力已按 D10 确认，但会改变缓冲、颜色及 stdout/stderr 分离语义，不能承诺与管道逐字节一致，也不提供聊天内全屏终端仿真。
- command/stdin/output 可能含用户数据；日志仅记录稳定 ID、状态和错误码，不重复记录原始参数与输出。
- codegraph/codebase memory 是结构证据，不替代编译、测试和手工外观验收。

## 14. 后台任务页面（窗格内独立页）

### 14.1 问题

本 feature 的 D9 停止语义只覆盖**本轮**：Esc 停止清「本轮新建 ∪ 当前活动交互」，而
清理整个会话只在「切会话 / `/new` / 关窗格 / 退出」时发生。于是用户留在会话里时，
自己起的服务（`npm run dev`、`cargo watch`…）没有任何办法停掉；一旦撞到
`max_running` 上限，也只会收到 `resource_limit` 错误而没有出路。

更根本的是**没有痕迹**：进程一直活着，界面上却什么都不显示，用户根本不会想到去清理。
所以只加一个"清理入口"不够，还得让"有几个在跑"随时可见。

### 14.2 形态：输入框位置的组件（不是页面）

演进过程值得记下来，因为两次都被用户判定为"错的方向"：

1. 第一版：`/stop` 两级浮层。**不合格**——全局覆盖屏幕。
2. 第二版：挂在 `Lx::task` 上的"窗格内整页"，`lx.rs::render` 早返回整页接管。
   **仍不合格**——把整个 pane 都遮住了，消息列表被自己重画的一份替代。
3. 第三版（当前）：**只替换输入框容器的组件**。`lx.task` 有值时，`layout` 把输入框
   矩形的高度换成面板高度，`build_chat` 在同一个矩形里画面板而不是输入框；
   历史区、置顶行、滚动条、聊天栏全部**照旧**走原本路径。

状态依然挂在 `Lx::task`（`Option<TaskPage>`）上，而不是 `app::overlay`——这带来一个
重要性质：**进入面板不销毁 lx 页状态**：

```text
Lx {
    input, cursor, scroll, undo, ...      // 原样保留
    chat: LxChat { session_id, messages, history_scroll, follow_bottom, ... }
    task: Option<TaskPage>                // 仅新增这一个字段
}
```

`esc` / 顶部 `↑` 关闭即回到离开时的样子，**不需要保存/恢复任何状态**。这也满足
"保证 lx 页面不被清理"的要求：没有任何清理路径会碰到 `pane.lx`。

### 14.3 版式：**替换输入框容器的组件**

面板不是独立页面，也**不整页接管**。它是「占输入框那个矩形」的组件：

- `lx.rs::layout` 在算输入框高度时分支：`lx.task` 有值时用 `task::height(page)`，
  恒为 `1 顶边框 + 1 tab 行 + LIST_MAX`（`LIST_MAX = 6`，比 opencode 的
  `maxHeight={5}` 再高一档——面板高度固定，多一点列表区不会让布局跳动）。
  **高度固定，不随条目数变化**：0 条与 6 条时面板一样高。否则打开面板会让上方
  消息列表被挤动一下再弹回来，空态也会孤零零吊在面板顶上。
- **顶边框**：面板首行是一条横线，把面板与上方消息列表分开。它画在面板矩形
  **内部**（`border_title()` 同色系），不与窗格边框重叠，因此不是早期那个双重边框。
- **tab 配色**：当前 tab 用 `style::selected_item()`（青底黑字 + BOLD），
  与**顶部标签栏的激活态完全同一套样式**——两处 tab 视觉一致，用户一眼就知道
  是同一个 tab 控件；非当前 tab 用 `muted()`。
- **键位提示统一为 `[功能 键]`**：功能名在前、快捷键在后，方括号包住整条。
  右上角 `[close esc]`；底边框 `[select ↑↓]  [stop ^d]  [tabs ←→]  [back esc]`。
  每条提示是一个整体（`text::Hint`），既表达"这是个可点的键位"，也避免
  "只有键位"与"功能+键"两套写法并存。
  窄窗格**整项丢弃**（从右往左），绝不把 `[back esc]` 截成半截——
  前两版实测过截尾会切出 `[esc] b`，比不显示更糟。
- **tab 与 `[esc]` 可点击**：`task::hit(rect, page, col, row)` 返回 `TaskHit::Tab(i)`
  或 `TaskHit::Close`；几何由 `tab_row_rects()` 统一给出，**渲染与命中共用同一份**，
  不会"画在这、点在那"。点击接到 `update::select_task_tab` / `update::close_task_page`。
- `build_chat` 在同一个 `lay.input` 矩形里，画输入框还是画面板，二选一。
- 其余一切照旧：历史区、置顶行、滚动条、聊天栏全部走原本路径。

```text
╭─ lx-[Native Agent] ─────────────────────── ws:…command-plan-tools ─╮
│ > 你是谁                                                            │
│ ■ 我是 Claude，一个由 Anthropic 开发的 AI 助手。                      │
│                                                                     │
│ Processes · Agents                                            esc   │
│ ❯ while true; do read -r line      /workspace/web      running 32s   │
│   npm run dev                      /workspace/web      running 1m30s │
╰─ ↑↓ select  ^d stop  tabs ←→  esc ─────────────────────────────────╯
```

**几何只有一处真相**：`lay.input` 既是「输入框矩形」，也是「历史区底边」。
换掉它的高度公式，历史区自动让位，不需要在面板里再算一遍 `history_rect`。

- 底边框：面板打开时**跳过 `status::render`**，整行归面板 hints——因此面板打开期间
  **不显示**模型名 / 用量 / 进程计数（用户明确要求）。关闭面板后自动恢复。
- 面板内**不画任何框**：窗格边框就是唯一一层框（早先画完整圆角内框，与窗格边框叠成
  双重边框，是用户报过的 bug）。
- 列表封顶 5 行，不额外加 padding / gap 空行——opencode 的 padding 是 Web 布局的呼吸感，
  在字符终端里会白吃对话空间。
- 列表：`❯ 命令 · cwd · running Ns`。命令列**定宽**、cwd 从固定偏移起，各行 cwd 左缘
  对齐成列；窄窗格先丢 cwd（识别进程靠命令），再硬切命令，时长始终可见。
- `Agents` tab 是**预留位**：列表恒空并显示 `no subagents yet`，后续在此显示 subagent 内容。

**面板打开期间每轮刷新进程列表。** `ListSessionProcesses` 原来只在
`open_task_page` 时发一次——面板**打开之后**才启动的进程永远进不了列表（用户报的
"启动任务后没有在面板中显示"）。现在 `event::chat::refresh_open_task_pages` 每轮为
每个「面板打开且绑定会话」的窗格压一次请求；列表回投幂等（`set_processes` 按内容
比较），无变化不置脏，因此不会引起重绘风暴。

**未绑定会话时不允许打开面板。** 面板列的是「本会话的后台进程」，而进程运行实例挂在
会话上（`ChatStreams::runtimes`）。空白草稿既没有可列的进程，也发不出有效的
`ListSessionProcesses`——开了只会显示一个永远空着的列表，让人误以为"后台什么都没跑"。
`open_task_page` 在 `lx.chat.session_id.is_none()` 时直接返回 `false`，不创建面板、
也不发效应，`↓` 因此是无副作用的空操作。

### 14.4 键位（对齐 opencode `composer.*`）

| 键 | 动作 | opencode 对应 |
|---|---|---|
| `↑` | 上移高亮；**已在顶部或列表为空 → 回到输入框** | `composer.shell.up` |
| `↓` | 下移高亮（到末尾即停） | `composer.shell.down` |
| `PageUp` / `PageDown` | 翻上方消息列表（辅助键，不写进底边框提示） | 消息区滚动 |
| `^d` | 停止高亮进程 | `composer.shell.kill` |
| `←` / `→` | 换 tab | composer 内联 `left`/`right` |
| `esc` | **无条件**回到输入框 | `escape` → Close composer |
| 空草稿 `↓` | 进入面板 | `session.child.first = down` |

**「回到输入框」= 关闭面板**（`pane.lx.task = None`）。面板占的就是输入框的矩形，
两者物理上不可能同时可见，所以没有"切焦点"的中间态；草稿、光标、历史滚动本来就没被
销毁，关掉即原样恢复（`opening_the_page_keeps_every_other_lx_field` 已锁住）。

**`move_active` 不循环**是 `↑` 退出的前提：面板要能分辨"上移成功"与"已经在顶部、
无处可去"，后者才该退出。循环会把顶部绕回底部，用户永远退不出去。

`esc` 保留为无条件退出：列表有 5 条时，让用户连按 5 次 `↑` 才能退出是不可接受的。

### 14.5 底部计数指标

输入框底边框右端（用量右侧）显示 `1 process` / `3 processes`，**0 个时整段隐藏**
（对齐 opencode footer 的 `N shell`，同样单复数与 0 隐藏）。窄到放不下时指标让位给模型名。

指标由 `event::chat::sync_background_task_counts` **每轮同步**，与页面是否打开无关——
这一条针对的正是「进程还活着但界面上没痕迹」这个根因。

### 14.6 非零退出不是失败（真实误判）

用户报告：一张 `■ Run command` 卡片是**红色**的，但命令本身完全正常。

真实会话记录（`agent_tool_call` id=98）：

```text
cmd:  pgrep -af "IFS= read" 2>/dev/null; echo "---"; ps -ef | grep -c "[r]ead -r line"
out:  35293
      ---
      0
exit: 1
```

`grep -c` / `pgrep` 在**没有匹配**时返回 1——这是"没找到"这个答案的正常表达，
不是命令坏了。模型自己读懂了（回答里写"退出码 1 表示未找到"），但界面把它渲染成了
红色失败卡片。

根因是三层把"进程以非零码退出"当成了"工具失败"：

| 层 | 原来的判定 | 后果 |
|---|---|---|
| `ProcessResult::is_failure` | `Exited { code != 0 }` → true | 工具结算为 `failed` |
| `apply_process_exit` | 非零退出写 `call.error = "exit 1"` | 步骤被标记为有错误 |
| `ToolStatus::Failed` → `StepMark::Failed` | 红方块 + 红色动词 | 界面显示红叉 |

修法是把**「进程级失败」与「命令退出码」彻底分开**：

- 进程级失败 = 超时 / 被中断 / 启动失败（`is_process_failure`）。这些仍然是红的。
- 命令以非零码退出 = 命令跑完了并给出了结果（`is_nonzero_exit`）。**不是失败**：
  工具结算为 `completed`、不写失败原因、方块保持工具色。

退出码本身**照常显示**在卡片标题上（`exit 1`），信息一点没丢——模型与用户都能看到。
这与 codex 的口径一致：它的 exec 渲染对 `exit 1` 的 search 类命令用 `dim()` 而不是
`red()`，注释写着 "Search exit 1 can mean no matches; report the code without calling
it a failure"。codex 在协议层仍把非零退出作为 `RespondToModel` 回给模型（模型能看到
退出码并自行判断），同样没有把它渲染成红色失败。

回归测试铺在三层：`process/manager`（真实子进程 `grep -c` 无匹配）、`agent_loop/executor`、
`event/chat` 端到端（落库 `completed`）、UI 渲染（方块不是红色，且 `exit 1` 可见）。

**历史记录的兼容**：改动前写库的旧记录仍标着 `failed`，只修判定会让老会话继续显示红色。
因此历史回放（`update::chat::view::tool_view_of`）对进程工具**按结果体重判**：
结果体里的 `process_state` + `exit_code` 是自洽的，`exited` + 非零 → `completed`，
并丢弃那句已经作废的 `command exited with code N`。这样**不需要写库迁移**，旧会话打开即正确。
真正的失败（`timed_out` / `interrupted` / `failed`）保持 `failed`，有反向测试守着。

### 14.7 卡片树形：只用 `│` 与 `└─`

命令与交互卡片统一成一根树干：

```text
■ Send input                                Stopped [details]
  │  $ while true; do read -r line; echo "got: $line"; done
  │  stdin: hello lx\n
  │  hello lx
  │  got: hello lx
  └─ hello lx
```

- **中间行一律 `  │  `**，**最后一行** `  └─ `。两条前缀等宽 5 列，正文永远从第 5 列起。
- `│` 与 `└` 落在同一列，树干连成一条线。
- 命令 → stdin → 输出 → 提示**全部串在同一根树干上**，只有一个收口。

曾经试过给每个「节点」开头再加 `├─`（命令一个节点、输出一个节点），结果一段输出里
同时出现 `├─` / `│` / `└─` 三种符号，比原来更花。用户明确要求恢复统一版式：
**一树干到底，只有末行收口**。回归测试 `tree_never_mixes_branch_symbols` 锁住
「不出现 `├─`」。

### 14.8 `Running` 停不下来（真实 bug）

用户报告：`Run command` 与 `Send input` 卡片上的 `Running · 4m 44s` 一直是 `Running`，
进程明明已经关了，时长还在涨。

真实数据（会话 `sess-18e01a856ea4e7f8-1`）：

```text
agent_tool_call:  100 exec_command  process_state=running
                  101 write_stdin   process_state=running
                  102 write_stdin   process_state=running
agent_process:    session_id=1  state=stopped  started=…831050  ended=…890382
```

三个根因，各自独立：

1. **卡片只在本轮 streaming 消息里找**。`apply_process_output` / `apply_process_exit`
   用 `streaming_message_mut()`，只认「最后一条 `status == "streaming"` 的助手消息」。
   模型把进程交回后本轮很快结束，消息转 `complete`；此后后台终态事件找不到那张卡片，
   `state` 永远停在 `Running`。而时长是 `now - started_at_ms` 每帧实时算的，于是一路涨。
   → 改为按 `call_id` 扫**所有**窗格的所有消息（`call_id` 在会话内唯一，DB 有唯一约束）。

2. **`write_stdin` 卡片从来收不到终态**。`apply_process_*` 只处理 `Command` 视图，
   `Interaction` 分支压根没有。三张 `Send input` 卡片因此永久 `Running`。
   → 新增 `apply_interaction_exit`，由 `handle_process_event` 在终态时一并收敛。
   只改 `state`，保留当次交互的输出正文（那是一次的「事实记录」）。

3. **历史回放照抄结果 JSON 的 `process_state`**。结果 JSON 记的是**结算那一刻**
   （通常是 `running`），进程之后可能早已退出。重开旧会话时卡片又疯跑。
   → `reconcile_process_records` 在 `LoadChatSession` 后用 `agent_process` 的真实记录
   覆盖卡片状态与起止时刻。没有记录时保持原样，绝不凭空捏造终态。

4. **`write_stdin` 在进程早已终态之后才结算**。真实时序（会话
   `sess-18e022434523f4f8-1`）：进程 14 在 `…356694` 已 `stopped`，而 `write_stdin`
   卡片 115 在 `…326003` 才结算——它读到的是 `running` 快照，此后**不会再有任何事件**
   去改它（终态事件早已过去），卡片就永远显示 `Running`。
   → 结算时（`finish_tool` 的 ②b1）按**会话运行实例的真实快照**再校正一次
   （`apply_process_snapshot`）：终态与运行态都覆盖，只改 `state`/`exit_code`，
   不动输出正文。

**时长只属于面板。** 消息列表里的卡片不再显示运行时长——历史记录里每帧变一个数
会让滚动与复制都不稳定，而"还有几个服务在跑、各自跑了多久"正是面板该回答的问题。
面板的 `now_ms` 由 `event::chat::tick_task_pages` 每轮推进（按秒对齐，同一秒内不重绘）；
它原来只在**列表回投**时更新，进程不变就会冻住时长。

回归测试四层：`process_cards`（轮次结束后仍收到终态/输出、交互卡片收敛）、
`view::tests`（回放按进程记录校正、活着的进程不许判死、无记录不改动）、
外加真实会话形状的临时验证（三张卡片全部收敛到 `Stopped`）。

### 14.9 刷新时机（踩过的坑）

停止是**异步**的（真实退出码只能由 `wait` 得到）。第一版在按 `^d` 后同步重拉列表，结果
读到的是**停止前**的快照：进程还在列表里，看起来像「停不掉」。实测复现后改为由
**进程终态事件**驱动刷新（`process_events::refresh_task_pages`）：进程真正退出时把该条目
从任何打开着的页面里移除并钳制高亮，同时把变更计入 `changed` 让事件循环置脏重绘。

## 15. 与 codex harness 的对照

以 `/Users/yonah/projects/agent/codex-main` 的 `codex-rs` 为参照（`core/src/tools/handlers/unified_exec*`、
`core/src/unified_exec/*`、`tui/src/exec_cell/*`），逐项核对本 feature 的工具实现。

### 15.1 已对齐的语义

| 维度 | codex | lx（本 feature） |
| --- | --- | --- |
| 工具集合 | `exec_command` / `write_stdin`（`update_plan` 独立为 control tool） | 同名三工具，`update_plan` 同样只改计划、不产生文件副作用 |
| 等待窗口 vs 进程寿命 | `yield_time_ms` 只结束本次等待，不杀进程 | 同；只有 `timeout_ms` 或显式停止才清理 |
| 跨轮会话复用 | 返回 `session_id`，后续用 `write_stdin` 续读 | 同，且进程归**聊天运行实例**持有（跨轮、跨请求代次） |
| 增量输出游标 | `output_start` / `output_end` 为原始字节偏移 | 同名同义，且模型游标与 UI 预览游标分离 |
| 丢弃证据 | `output_omitted_bytes` + 显式 omission marker | `dropped_bytes` + `Earlier output discarded` / `… +N more lines` |
| 非零退出语义 | 命令**跑完且失败**，带真实输出与退出码 | 同（`is_failure` 由进程状态推导，不是工具异常） |
| `tty` | 独立 PTY（固定尺寸） | 同（固定 120×40） |
| `login` 默认 | 默认登录 shell；`login: false` 关闭 | 已对齐为默认 `true` |
| 首轮等待默认 | `exec_command` 10s（有效范围 250–30000） | 已对齐为 10s / 上限 30000 |
| 交互等待默认 | `write_stdin` 250ms | 独立常量 `DEFAULT_STDIN_YIELD_MS = 250` |
| 分块标识 | `chunk_id`（6 位十六进制） | 同名，6 位十六进制，每次调用不同 |
| 墙钟时间 | `wall_time_seconds`（实际等了多久） | 同名，内部存整数毫秒、JSON 输出秒 |
| 输出预算 | `max_output_tokens`，超出截断并显式标记 | 同名，另有 `MAX_MODEL_OUTPUT_BYTES` 硬字节上限 |

### 15.2 有意不跟随的部分

- **沙箱与审批**：codex 走 `SandboxManager` + approval（`sandbox_permissions` / `justification` /
  `prefix_rule`）。lx 本阶段明确不做沙箱、不逐次弹窗（D1），因此 schema 里不出现这些字段——
  声明了就必须实现，否则等于骗模型。
- **并行工具调用**：codex handler 声明 `supports_parallel_tool_calls = true`。lx 的
  `run_agent_turn` **同一响应内串行执行**（顺序即供应商 ordinal，前一结果对后一调用可见），
  这与 lx 的 BeginTool/FinishTool 双门槛、以及「进程工具的停止集合」语义一致；改成并行会
  同时动摇这两条不变量，不在本 feature 范围内。
- **工具路由层**：codex 有 `registry` / `router` / `orchestrator` / `runtimes` 四层抽象以支持
  多 sandbox runtime、MCP、插件与 hook。lx 是单二进制、固定七工具，分派就是
  `AgentTools::execute` 里的一次 `match`；引入同样的分层只会为不存在的扩展点付维护成本。
- **结果形状**：codex 给模型的是 `Chunk ID: … / Wall time: … / Process exited with code N` 的
  人读文本头 + 裸输出；lx 给的是**唯一结构化 JSON**（`tool`/`session_id`/`process_state`/
  `output_start`… 加上 `chunk_id`、`wall_time_seconds`）。选择 JSON 的原因：UI 视图、DB
  落库与历史回放共用同一形状（`ToolDetailView::from_result_json`），不需要第二套解析。
- **`environment_id` / 多环境**：codex 支持多 environment 与远程执行器；lx 只有冻结工作区
  一个 cwd（D2），不引入该字段。
- **transcript 全屏**：codex 用 `ctrl+t` 打开 transcript overlay 看完整输出；lx 选择**原位展开**
  （D7），滚动仍由外层历史统一管理，不新增日志浮层。

### 15.3 渲染层的口径差异（对齐的部分）

- codex `compact_command_lines`：`• Running <cmd>` 头 + `  └ ` / `    ` 前缀的输出块，
  只留最近 `DETAIL_PREVIEW_LINES = 3` 行，超出显示 `… +N lines (ctrl+t to view transcript)`。
  lx 采用同款「3 行预览 + 显式省略行数」，但把整块挂在**一根不断开的树干**下（`│` 中间行 /
  `└─` 收口），与 lx 已有的 prompt list 语法统一。
- codex 行首是单色 `•`；lx 沿用既有 `■/□` 呼吸方块（运行中呼吸、终态静止），并把状态文字
  留在右上角而不是塞进动词。

## 16. 审核门槛与实施结论

D1–D10 已确认并全部落地。实施验证：

- `cargo test`：全部单元测试 + 16 个集成测试二进制通过；
- `cargo fmt --check`：通过；
- `cargo clippy --all-targets -- -D warnings`：无告警；
- SCHEMA_VERSION 实际为 **11**（新增 `agent_process`）；
- 仍待用户 `cargo run` 验收：真实终端观感（颜色/对齐/窄窗格）与跨轮服务保留的
  实际操作体验。
