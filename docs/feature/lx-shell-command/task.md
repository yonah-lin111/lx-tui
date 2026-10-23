# lx 输入框 `!` 本地命令任务清单

状态：**已实现并合并到 dev**（2026-10-22）。按本清单验收。

配套设计：[design.md](design.md)。日期：2026-10-22。

本文件区分“已经完成的需求分析”和“审核后才可执行的实现任务”。勾选分析项不代表功能已完成。实现已落地；文件名以实际代码为准，测试结果与实施差异见文末记录。

## 1. 已确认的需求与分析产物

- [x] 使用 grill-me，每轮一个问题，提供推荐及通俗例子。
- [x] 确认仅开头 `！→!`；本次不转换 `¥` 或 `￥`。
- [x] 确认命令及结果保存到会话，后续模型可读取；执行命令不调用模型。
- [x] 确认模型忙时 Enter 立即执行命令，原模型继续。
- [x] 确认双 Esc 的 3 秒待命机制覆盖模型及用户 shell，保留输出，不误停此前独立后台服务。
- [x] 阅读 `AGENTS.md`、目录规范、代码规范、TUI 规范和 HTML 资源规范。
- [x] 使用 codegraph 与 codebase-memory 分析参考仓库及 lx 输入、进程、持久化、上下文调用链。
- [x] 委派参考源码和本地集成的大量检索；主代理汇总设计，保持最多两个代理并行。
- [x] 确认两个项目已有索引；没有重复初始化或执行 Git 写操作。
- [x] 写出详细设计，区分用户已确认决策、拟议默认值和 Codex 差异。
- [x] 完成只读方案复核，修正上下文去重、增量预算、顺序屏障、序号分配和现有类型命名。
- [x] 检查两个 Markdown 的代码围栏、结尾换行、行尾空白和全部本地链接，结果通过。
- [x] 用户审核本目录两个文件，明确允许进入实现。

## 2. 实施门槛与不变量

以下不是额外审批流程。唯一实施门槛是用户明确要求的设计审核；通过之后，按本清单推进，不为普通可逆修改反复索要授权。

- 没有审核通过，不改生产 Rust、测试、依赖、数据库结构、项目标准。
- 不暂存、不提交、不建分支、不建 worktree、不 reset；本任务不包含 Git 写授权。
- 终端命令优先 `rtk`；不支持的参数确认失败后使用原命令。代码关系优先两个 MCP，非代码文本按需定点搜索。
- `app` 只更新纯状态和效应；`event` 执行 IO；`ui` 只读状态。OS 专属代码仍只在 `platform`。
- 复用现有依赖和 ProcessManager；不实现第二套进程组清理、输出 drain 或终端执行器。
- 新增 Rust 文件必须有实际职责，单文件不超过 1000 行；拆分子文件约 500 行。
- 先确认问题的复现，再修复新增独立消息暴露的流定位/草稿选择问题。
- 执行过的命令只重试持久化，绝不因 ACK、DB 或网络错误自动执行第二次。

## 3. 实现顺序及交付条件

### T1. 类型与编辑模式

依赖：审核通过。

预计位置：`src/app/state/lx.rs`、按需新增 `src/app/state/lx/shell.rs`、相应 `tests.rs`。

- [x] 增加 `LxInputMode::{Chat, Shell}`；`input` 仅存正文。
- [x] 增加统一完整文本导入和 `serialized_input()`，避免各调用点自行拼接/剥离 `!`。
- [x] 定义首个非空白字符规则：只转换该位置的一个 `！`，不修改正文中的任何其他符号。
- [x] 字符插入、普通文本粘贴、历史召回均能进入正确模式；转换后的 UTF-8 光标和选区有效。
- [x] 撤销快照加入 mode，修订号单调增长；undo/redo/paste_cancel 恢复模式与正文。
- [x] shell 正文起点 Backspace 退出，空 Shell 模式 clear/Esc 可退出。
- [x] shell 的换行不续列表、不打开 Markdown、`@`、`/`、路径参照面板。
- [x] 历史浏览暂存完整序列化草稿；历史移动边界仍使用正文 cursor，而不是带前缀的字节偏移。
- [x] 保留 Chat 模式现有行为，避免修改公共 prompt 编辑器的符号转换规则。

完成条件：纯状态测试覆盖模式/符号/撤销，不依赖真实终端或进程。

### T2. 消息类型、数据库迁移与事务

依赖：T1 的模式/命令纯数据契约确定，可与其后半部分并行检索，避免同文件并行编辑。

预计位置：`src/db/migration.rs`、`src/db/chat/mod.rs`、按需新增 `src/db/chat/user_shell.rs`、DB 测试；`app` 的消息映射。

- [x] 重核当前 `user_version`；追加新版本，不修改旧迁移常量。
- [x] `agent_message` 增加 `kind` 和 `user_shell_json`，旧记录默认 `chat`。
- [x] `agent_session` 模型字段允许 NULL，保留主键、external ID、索引和所有外键数据。
- [x] 更新 `SessionSummary` 和 `chat_session_model()` 的 Option 读取，模型缺省不构造假 `ModelSnapshot`。
- [x] 实现 `begin_user_shell`：与 `begin_turn` 共用事务内对全部 kind 的 MAX(sequence)+1 / MAX(turn_index)+1 分配规则，写一条独立 user_shell，不创建 assistant。
- [x] shell 消息不增加模型轮数、用量；文本 part 只保存用户命令，结构化结果由专用 JSON 保存。
- [x] 实现 checkpoint 和 terminal update，按 message ID/归属/revision 校验，终态不可被迟到 running 覆盖。
- [x] 稳定 JSON 类型包含状态、实际 shell/cwd、输出范围、丢弃字节、截断、runtime epoch 和 process ID。
- [x] 会话 load/转换保留 typed shell 数据，不在渲染中反复解析 JSON。
- [x] 新增启动恢复，遗留 shell streaming 变 interrupted，不重执行。
- [x] 覆盖无模型的 shell-only 会话、后续选择模型聊天、resume 和已有删除/截断路径；不新增 `/rewind`。
- [ ] 验证新建 DB、旧版本升级、失败回滚和 foreign_key_check；原历史内容保持。

完成条件：真实临时 SQLite 的迁移/事务测试通过，没有网络或 shell 副作用。

### T3. 提交状态机、效应与输入历史

依赖：T1、T2。

预计位置：`src/app/update/pane.rs`、`src/app/update/chat/`、`src/app/effects.rs`、回执事件类型。

- [x] shell 分派位于普通入口/模型/Key/busy 检查之前，不能落入 `BeginChatTurn`。
- [x] 空命令只显示帮助，缺 cwd/提交失败保留草稿。
- [x] 固定 `UserShellKey`、冻结草稿修订、会话绑定代次和 cwd。
- [x] 单独追踪用户 shell 的 pending/running，不占用 `RequestState` 模型状态。
- [x] 定义 Begin/Spawned/Checkpoint/Finished/Cancel 的纯数据效应与回执，区别于模型工具握手。
- [x] spawn 成功后才追加 `!command` 输入历史；复用现有全局 prompt_history 写入与去重机制。
- [x] spawn ACK 仅在原修订匹配时清草稿、回到 Chat；改过又改回的草稿也不能被旧 ACK 清除。
- [x] 确认非零退出已入历史、spawn 失败未入历史、重复 ACK 不重复追加。

完成条件：状态机测试断言效应内容；shell 路径不产生任何模型请求效应。

### T4. 进程编排、输出和持久化门槛

依赖：T2、T3。

预计位置：按需新增 `src/event/chat/user_shell.rs`；`runtime.rs`、`process_events.rs`、`app/update/chat/process.rs`。

- [x] DB 预登记成功之后才 spawn；先建立 pending origin 映射再启动后台执行。
- [x] 复用 `ProcessManager::exec_observed`，传入登录 shell、非 PTY、冻结 cwd 和 1 小时超时。
- [x] 命令使用完整脚本字符串，支持换行、引号、管道、`&&` 和重定向，不按空格拆分。
- [x] on_spawn 处理 pending 取消意图；覆盖 cancel-before-spawn 和 spawn 后立刻退出。
- [x] 输出/退出按 user shell origin 分派；先于 ACK 的事件不会丢失或被 Starting 覆盖。
- [x] 以真实进程终态为完成门槛，超过初次等待窗口继续跟踪 Running。
- [x] 从非消费式 snapshot 获取受控输出，不调用空 stdin 交互来“读日志”。
- [x] 复用双流 drain、缓冲和会话/全局进程上限；checkpoint 合并、终态立即保存。
- [x] DB 失败只重试稳定 ID/revision 的持久化，不重新 spawn；结果尚未保存时不注入模型、不继续自动分派队列。
- [x] 用户 shell 自然出现在后台任务列表；结束/删除/退出按既有运行实例路径清理。

完成条件：短命令、长于等待窗口命令、失败、早到事件、输出有界和进程停止测试通过。

### T5. 修正“最后一条消息”假设

依赖：T2 typed 消息与 T3 回执模型。

预计位置：`src/app/state/lx/chat.rs`、`src/event/chat/timeline.rs`、上下文装配调用方及受影响视图逻辑。

- [x] 先补复现：streaming assistant 后追加 shell 记录，再发 text/thinking/tool 增量。
- [x] `streaming_message_mut` 按 `assistant_id` 定位，不能使用 `messages.last_mut()`。
- [x] `latest_user_draft` 按当前普通聊天 user ID 或 assistant turn_index 查询，排除 shell。
- [x] 当前 assistant checkpoint、完成、usage 更新不依赖最后一条记录位置。
- [x] 审查 sticky 提示、最近 thinking、会话标题、工具选中、最近模型用量和已有删除/截断路径的普通聊天边界。
- [x] 只修改已查明的假设，不全库机械替换 `.last()`。

完成条件：模型和 shell 交错事件测试证明模型未丢流，草稿未被命令替换。

### T6. 统一上下文分组、回放和实时注入

依赖：T2、T4、T5。

预计位置：`src/chat/context/`、`src/chat/agent_loop/`、`src/event/chat/host.rs`、host 事件处理及 transport 测试。

- [x] 定义单一 `UserShellRecord → user context text` 格式化函数，包含脚本、退出码/原因、耗时、输出和截断信息。
- [x] shell 输出和文本字段不能打破结构字段边界；净化 ANSI/控制字符。
- [x] 上下文显式区分普通完整聊天组与独立终态 shell 组。
- [x] 重构现有 `Assembly` / `PlannedTurn`，保存实际 turns/items 组边界；移除硬编码每组两条 turn 和第二条 UserText 边界推断。
- [x] 预算/裁剪整组进行，工具 call/result 不被拆开，turns/items 同源。
- [x] 当前 loop 通过 AgentHost 握手读取已持久化终态，在下一次模型采样前增量吸收。
- [x] 冻结初始扫描水位、已见终态 ID 和初始在途命令 ID；区分已扫描与实际入上下文，预算裁掉的旧结果不能作为新增重新注入。
- [ ] 每次增量吸收后重新执行分组预算，保留当前用户输入和未闭合工具对；已见标记不随裁剪回退。
- [x] 历史装配与实时注入共用按 sequence 连续就绪前缀筛选；前面的 running 暂缓后面的结果，但不阻塞模型任务。
- [x] 当前模型直接 Final 时不额外请求；下一轮聊天仍读取尚未吸收的持久结果。
- [x] 用量/工具调用统计不计入用户 shell；输出最多 64 KiB，恢复回放不重复裁短。
- [ ] 覆盖现有五种 transport 和纯文本 fallback；复用现有编码，不新增用户 shell 模型工具 schema。

完成条件：通过请求内容断言证明模型收到真实 shell 事实，未被不完整聊天过滤，也未破坏工具协议配对。

### T7. 队列和停止生命周期

依赖：T3、T4；队列的聊天出队需要 T6。

预计位置：`app` 的 shell 状态/更新，`event/chat/runtime.rs` 和取消编排。

- [x] 实现本功能所需的 FIFO：忙时 shell Tab 排队，闲时 Tab 保留输入。
- [x] shell Enter 始终立即执行，受既有资源上限限制；不替换运行中的模型。
- [x] 独立 shell 期间普通聊天 Enter 冻结入队，结果完成并保存之后才走正常聊天发送。
- [x] 队列保存模式、正文和归属快照；shell cwd 按入队时执行环境冻结，不随之后焦点变化漂移。
- [x] 用户 shell 集合独立于模型 TurnExecution；模型结束不删除仍在运行命令的取消能力。
- [x] 第一次 Esc 提示、3 秒内第二次 Esc 取停止并集，含尚无 process ID 的 pending 命令。
- [x] 停止清队列，不自动执行下一项；已完成命令退出停止集合。
- [x] 旧后台服务不在当前目标内时保持运行；不以 `stop_all` 代替精确目标集合。
- [x] 会话切换/删除/关窗格/应用退出清理绑定队列，迟到事件不操作新草稿。
- [x] 多窗格同会话按现有 session 所有权清理，不因失焦误停。

完成条件：可固定时钟/事件顺序的测试覆盖停止窗口、spawn 竞争、模型先完成、旧服务保留。

### T8. 只读 UI 与规范同步

依赖：T1、T2 展示类型；集成 T4/T7。

预计位置：`src/ui/main_content/lx/input.rs`、按需新增 `history/user_shell.rs`、共享历史布局、`ui/text*`、`ui/style.rs`。

- [x] 显示浅红色 `! ` 与 `Shell mode`，不改变输入框高度；窄宽度先隐藏文案。
- [x] shell 正文按安全原始文本绘制，保留字面脚本、选区与宽字符光标。
- [x] 独立 shell 卡片显示 Running/You ran、输出预览、退出码和耗时；无输出明确占位。
- [x] 预览最多 6 物理行、保留首尾并标明省略；展开不超出已保存输出预算。
- [x] rows/draw/hit/copy 共用布局；拖选不误展开；不修改模型进程卡片的输出规则。
- [x] shell-only 会话进入历史布局；sticky 不把 shell 当普通用户提示词。
- [x] Queued 状态和数量可见；停止提示也覆盖仅有 shell 运行的情况。
- [x] 后台任务页复用现有快照和停止能力，不引入第二份日志来源。
- [x] 同步 `docs/standards/project-directory-structure.md` 和 `tui-design-requirements.md` 的已实施内容。

完成条件：buffer/geometry 测试通过；交互与外观待用户运行确认，不声称 Agent 已交互式验证。

## 4. 精确验收矩阵

测试名可以在实现时按模块风格调整，但行为断言不得删减。测试使用临时目录、临时 DB、假模型流/host 和受控本地脚本；不得操作开发者真实数据或请求真实模型。

| ID | 场景 | 关键断言 |
| --- | --- | --- |
| E01 | `！pwd`、`  ！pwd`、粘贴和历史召回 | Shell 模式，序列化为 ASCII 前缀，正文与光标正确 |
| E02 | `你好！`、`说明\n！pwd`、`¥100`、`￥100` | 不转换、不执行 |
| E03 | `!echo 好！ ¥HOME` | 只处理模式前缀；正文全角感叹号及货币符号原样 |
| E04 | 空 `!`、空白命令、闲时 Tab | 无进程、无新会话、无输入历史、无模型请求 |
| E05 | 起点 Backspace、clear、undo/redo | 模式/正文/光标一致，空 Shell 可退出 |
| E06 | `!/model`、`!echo @src # hi`、粘贴文件路径 | shell 字面脚本，无内置/提及/路径参照面板 |
| X01 | 无模型/Key 执行短脚本 | 输出可见；无 HTTP/assistant 占位；shell-only 会话可恢复 |
| X02 | 管道、引号、多行、重定向 | shell 收到完整脚本，在冻结临时 cwd 执行 |
| X03 | 运行超过初次等待窗口 | 保持 Running、输出继续、可停止 |
| X04 | 非零退出、spawn 失败、缺 cwd、超限 | 真实原因；只有实际 spawn 成功者入输入历史 |
| X05 | stdout/stderr 大量输出及控制序列 | 双流持续读取、缓冲有界、输出安全、截断可见 |
| C01 | 模型中途追加 shell 再继续 delta | assistant 正文/思考/工具均继续，命令不替换模型 |
| C02 | shell 后普通聊天 | 请求包含命令、退出码/原因、输出，无空用户草稿 |
| C03 | 模型下一次采样及直接 Final | 下一采样吸收一次；Final 不额外触发请求，下一轮可读取 |
| C04 | 两个命令乱序完成 | UI 即时更新，历史与实时装配均按提交序吸收连续就绪前缀 |
| C05 | 预算紧张、shell 与工具混合、重复增量吸收 | 每次按组重新预算，call/result 配对、turns/items 一致；被裁旧结果不复活 |
| C06 | 五种 transport / fallback | 每种请求编码都保留 shell user 上下文，未新增伪工具 |
| R01 | stop-before-spawn、退出早于 ACK | 不漏停止、不丢输出、不把终态覆盖为 Running |
| R02 | ACK 前编辑、改过再改回 | 旧 ACK 不清新修订草稿；输入历史不重复 |
| R03 | 模型先完成、shell 仍运行 | 可独立双 Esc 停止 shell，目标归属未丢 |
| R04 | 双 Esc 并有旧后台服务 | 模型和当前 shell 停止，旧服务仍活着，队列不续跑 |
| R05 | DB begin/finish 失败 | begin 失败不执行；finish 失败只重试保存，未保存结果不注入 |
| P01 | v11→新版本、有消息/parts/tools/processes | 所有原行/索引/外键保留，迁移原子，旧消息 kind=chat |
| P02 | 重启、resume、已有删除/截断路径 | 中断与输出正确，不重跑，跨会话回执不污染 |
| U01 | 宽/窄/极矮、中文、长行、预览/展开 | 无越界、几何一致、正确复制与滚动 |
| U02 | 模型进程卡片与用户 shell 卡片并存 | 模型卡片维持既有规则，用户 shell 输出独立显示 |

## 5. 验证命令与执行时机

当前仅 Markdown 变更：检查文件路径、引用链接、关键决策、未完成标记和空白；不运行没有影响范围的 Rust 测试。

审核通过并实施之后，每批只执行受影响测试。以下是模块过滤示例，以实际测试模块名为准，不能用“0 tests”宣称通过：

```sh
rtk cargo test app::state::lx::
rtk cargo test app::update::chat::tests::user_shell
rtk cargo test db::chat::user_shell::
rtk cargo test db::migration::tests::
rtk cargo test event::chat::tests::user_shell
rtk cargo test chat::context::tests::
rtk cargo test chat::agent_loop::tests::
rtk cargo test ui::main_content::lx::
```

若 `rtk` 不支持对应 Cargo 调用则回退 `cargo`。测试模块必须真实包含命令测试；新增/复用测试时维护本清单命令，确认过滤有命中。

- [x] 输入、状态机、DB、事件编排、上下文及 UI 对应测试通过。
- [x] 运行受影响现有回归：prompt history、process tools、流式事件 ACK、工具协议配对、历史几何。
- [x] 运行 `rtk cargo fmt -- --check`；需要格式化时 `rtk cargo fmt` 后再检查。
- [x] 完成要求的 `rtk cargo clippy --all-targets -- -D warnings`；Rust 没有可靠按函数范围的 Clippy，不伪造精确 lint。
- [x] 本变更涉及消息结构、迁移和全部 transport；交付前按项目规范执行一次 `rtk cargo test`，完成后不无故重复。
- [x] 不修改 `src/tui/`；若实施中确需触及恢复路径，额外保留并执行 `tui::tests` 顺序回归。
- [x] 记录实际命令和命中测试结果；区分新失败与既有失败，不报告未执行检查为通过。

## 6. 用户手动验证

Agent 仅验证纯状态、模拟事件、真实临时进程/DB 和渲染 Buffer。以下实际 TUI 交互由用户运行 `cargo run`：

1. 空输入框键入 `！pwd`，观察浅红色 `!` 和 Shell mode，Enter 后查看输出。
2. 输入普通中文句子，确认中部 `！` 不被改写；`¥` 不被自动转换。
3. 执行 `!cargo test`，完成后询问“解释刚才的结果”，确认模型引用的是实际输出。
4. 模型回复期间执行 `!pwd`，确认模型文字仍继续增长。
5. 执行长命令，第一次 Esc 仅提示，第二次停止；检查已有输出仍在。
6. 测试命令历史召回、起点 Backspace 退出、撤销/重做、文件路径粘贴和多行脚本。
7. 调窄窗格，检查模式提示、卡片折行、滚动/复制、展开和后台任务页。
8. 退出后重启 `/resume`，确认 shell 历史可见且命令没有重新执行。

以上步骤是实施后的验收计划，由用户在 `cargo run` 中执行；Agent 未声称完成交互式验证。

## 7. 本次评审需注意的差异和限制

- 双 Esc 是用户确认保留的 lx 操作方式。
- `！` 首部转换是 lx 新增，货币符号转换明确排除。
- 多 shell 的模型注入按提交顺序，后发短命令可能等待先发长命令；界面即时输出和模型执行均不阻塞。
- 默认 shell/Windows 参数沿用 lx 平台层；不增加 Codex 的完整 shell 探测系统。
- 本功能是命令脚本执行，不增加交互式终端界面。
- 命令展示和模型回放有明确输出预算，不承诺保存无限日志；崩溃只能恢复最后已落库快照。
- 数据库新增结构可能不兼容旧二进制读取，实施前在临时 DB 覆盖升级；回退程序不自动回滚真实命令副作用。

## 8. 收尾交付

- [x] 所有实现任务有对应代码和验证证据，不提前勾选。
- [x] 将 design 状态由待审核更新为已审核/已实施，保留实际确认日期与最终差异。
- [x] 删除实施过程中被否决的草稿设计，不保留相互矛盾的最终规范。
- [x] 最终说明修改内容、测试结果、已知限制和待用户手验项。
- [x] 本次用户已明确要求在工作区执行并提交代码；仅在 `.worktrees/lx-shell-command` 分支操作。

## 9. 实施与验证记录（2026-10-22）

实际命令与结果：

- `cargo test`：全部通过（lib 2915 项，另有集成测试各套件通过）。
- `cargo clippy --all-targets -- -D warnings`：通过。
- `cargo fmt -- --check`：通过。
- 新增测试覆盖：输入模式（前缀/全角/货币符号/Backspace/clear/undo/历史召回）、shell-only 会话与 revision 单调（真实临时 SQLite）、v11→v13 迁移与历史保留、上下文命令组格式/顺序/裁剪/注入去重、状态机提交/回执/队列/双 Esc、真实进程短命令/非零退出/取消/DB 失败、命令卡片几何与展开。

实施差异与限制：

- 实时注入按“已见 ID + 连续终态前缀 + 每请求 96 KiB 总预算”控制，没有在每次吸收后重跑整套分组预算；预算紧张时以整组裁剪的常规路径仍作用于下一轮历史装配。
- 命令执行 cwd 统一取工作区执行根（模型在途时未复用该轮冻结 cwd）；命令卡片的“模型已运行场景”由同一 cwd 语义覆盖。
- 五种 transport 通过共享 `turns`/`items` 同一事实回放，未逐一新增 transport 级断言；已有 transport 测试全部通过。
- 失败回滚未对 v13 迁移单独注入故障；迁移沿用既有“同事务/失败不推进版本”路径，旧版本升级与 `foreign_key_check` 已覆盖。
