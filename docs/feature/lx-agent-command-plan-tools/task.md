# lx Agent 命令、进程交互与计划工具：执行任务

> 状态：**已实施**（D1–D10 全部落地，`cargo test` / `cargo fmt --check` /
> `cargo clippy --all-targets -- -D warnings` 全绿）。真实终端观感仍需用户 `cargo run` 验收。
> 对应设计：`design.md`。
> 用户已确认三工具、直接执行、跨轮保留及切换/关闭清理、紧凑卡片与展开、本轮停止范围，以及普通管道和独立 PTY。

## 0. 阶段与审核记录

- [x] 使用 grill-me，每次仅提出一个具体问题，提供推荐与通俗例子，不使用提问工具。
- [x] 确认 `exec_command/write_stdin/update_plan`，不加 subagent。
- [x] 确认新增对应 TUI 组件。
- [x] 确认命令直接执行、无逐次弹窗、沙箱后续考虑。
- [x] 确认同会话跨轮保留，切换会话/关窗格/退出清理，重启不重跑。
- [x] 阅读项目规范，结合 codegraph 和 codebase memory 定位核心入口和数据关系。
- [x] 创建独立 feature 文件夹，编写详细设计与任务草案。
- [x] 确认 P1（design D8）：聊天内紧凑卡片、最近 6 行命令输出、按需展开详情、交互记录和计划步骤进度。
- [x] 确认 P2（design D9）：停止本轮新进程及当前正在等待/交互的进程，保留此前未参与当前操作的服务。
- [x] 确认 P3（design D10）：默认普通管道，显式 tty=true 使用独立 PTY，通过 write_stdin 交互。
- [x] 将所有确认结论写回 design/task，收拢 P1/P2/P3，补齐待完整文档审核的参数、限额和界面操作。
- [x] Context7 核对 Tokio 子进程 drop/kill/wait 语义，核实已有依赖和 schema 基线。
- [x] 用户审核并明确批准完整 design.md/task.md（本次按指令直接执行，不再走 grill-me）。

执行门槛已满足。开发流程按项目规范执行：需求 → 检索 → 设计 → 审核 → 实现 → 精确验证；
本次实现使用 codegraph 与 codebase memory 实际检索，并在 `.worktrees/lx-agent-command-plan-tools`
独立工作区完成（不在主检出上改动）。

## 1. 实施前复核

- [x] 复核四份规范及待修改目录的局部 AGENTS 指令。
- [x] 重新通过 codegraph/codebase memory 定位实施时符号，检查索引覆盖与源文件新鲜度。
- [x] 对照本次参考的 Codex 工具源码确认参数语义差异：`session_id` 可空 `chars` 与等待窗口沿用；
      未引入 `sandbox_permissions` / 审批；`terminate` 是 lx 扩展。tokio `Child`/`kill`/
      `wait`/`start_kill`/`process_group` 与 portable-pty `Child::try_wait`/`ChildKiller`
      已按锁定版本源码核对（`tokio 1.53.1`、`portable-pty 0.9`）。
- [x] 启用现有 tokio 的 process feature；复用 portable-pty 0.9（PTY 走 `Child::try_wait` + `clone_killer`）；未新增依赖。
- [x] 记录实际 SCHEMA_VERSION = **11**；复用既有会话占用检查与 `CancelChatStream` 取消路径。
- [x] 列出受影响模块与测试过滤器；按要求在 `.worktrees` 新工作区执行（完成后询问是否合并）。

产出：更新设计 §4/§11 的真实路径和关键接口，不根据旧文档猜测版本。

## 2. 纯数据契约与工具规格

- [x] 在既有 schema 入口增加三个工具，字段白名单、类型、默认值、上限与英文描述一致（上限常量单一来源）。
- [x] 区分聊天会话 ID、进程 session_id、OS PID 与 runtime_epoch（`ProcessOwner`/`ProcessResult`）。
- [x] 明确 `ProcessState`、`ProcessResult`、`PlanSnapshot` 与稳定错误码。
- [x] 扩展 `format_for_model` / `to_result_json` / 视图入口，结果 JSON 只有一处生成。
- [x] `ToolDetailView::{Command, Interaction, Plan}` 类型化视图；文件摘要不塞进程/步骤字段。
- [x] 参数校验拒绝未知字段、空白 cmd、越界等待/输出预算、超长 stdin、非法计划；`terminate` 与非空 `chars` 互斥。
- [x] 覆盖错误情况下无进程副作用（`invalid_process_arguments_never_spawn_a_process`）与无计划提交。

关键验收：同一规范化结果在模型回放、持久化和 UI 有一致含义；退出码非零不会被当成成功命令。

## 3. 进程管理与执行

- [x] 实现会话级 `ProcessManager`（`event/chat/runtime.rs`）；`AgentTools` 跨轮注入同一实例。
- [x] 进程 spawn 只在 BeginTool 取得 ticket 之后发生；起始记录在工具结算成功后登记。
- [x] `exec_command` 使用冻结 cwd 或显式 `workdir`（相对路径基于工作区），不调用 `set_current_dir`。
- [x] pipe 与独立 PTY（120×40）；真实退出码来自 `wait`/`try_wait`，读端 EOF 不改状态；进程组终止封装在 `platform::terminate_process_group`。
- [x] 两路异步 drain；等待窗口到期返回 `session_id` 与 `running`（`stdout_and_stderr_are_both_drained`）。
- [x] `write_stdin` 校验 owner（未知/过期 ID 明确报错），支持空读、精确输入、BrokenPipe 映射为 `stdin_failed`。
- [x] 普通管道不把 `\u0003` 当信号；停止统一走 `stop_handle`（温和→强制→reap）。
- [x] `terminate: true` 按已校验 ID 停止；重复停止只返回已知终态（`terminate_stops_a_running_process_and_is_idempotent`）。
- [x] PTY 固定 120×40；卡片只显示安全文本；用户终端窗格不受影响。
- [x] 停止/reap/Drop 兜底；登记失败立即停止新进程。
- [x] 覆盖短命令、非零退出、等待不杀进程、跨轮继续交互与无效 ID（31 个进程测试）。

关键验收场景：第一轮启动服务返回 ID；第二轮使用同一 ID 获取新输出；期间没有重复启动服务。

## 4. 输出缓冲、事件与限额

- [x] 有界尾部 256 KiB、绝对字节偏移与 `dropped_bytes`。
- [x] 模型游标与 UI 预览分离（`ui_preview_does_not_consume_the_model_cursor`）。
- [x] 持续 drain；UI 输出通知每进程最多一个在途，事件只带 ID 不带原始输出。
- [x] 跨 chunk UTF-8、ANSI/OSC、`\r` 与超长序列安全化（12 个输出缓冲测试）。
- [x] 单次输出受 token 估算/64 KiB/回放上限约束；只推进实际返回区间。
- [x] 每会话 8 / 全局 32 运行进程、stdin 64 KiB、输出 256 KiB、终态 32 项/10 分钟。
- [x] 输出通知每进程最多一个在途；控制序列 256 字节上限，超限丢弃。
- [x] 测试双流大量输出、持续输出、无输出、截断、多字节边界与事件合并。

关键验收：持续打印的进程不会无限占用内存、卡死事件循环或让新输出抢走用户历史滚动位置。

## 5. 持久化与恢复

- [x] v11 迁移新增 `agent_process`（归属 + `(runtime_epoch, process_session_id)` 唯一 + 会话级联删除）。
- [x] 工具结果不可变；进程输出/退出只写 `agent_process`。
- [x] 受控快照落库，revision 拒绝乱序，终态立即提交，每进程每秒最多一次合并。
- [x] 快照写经事件层受控路径；重载一次批量查询（`load_processes_for_session`）。
- [x] 进程快照按 `origin_call_id` 关联原卡片；`write_stdin` 保留当时结果快照。
- [x] 启动恢复标 `interrupted`，不重跑、不用旧 PID。
- [x] 工具 finish 落库失败即停止本轮，不重试执行或写入。
- [x] 覆盖迁移/新库/版本升级、跨会话拒绝、崩溃恢复与终态冲突（8 个 DB 进程测试）。

关键验收：exec 调用已 completed 但进程仍 running 的记录可以正确恢复；重启显示 interrupted 而非 Exited 0。

## 6. 生命周期集成

- [x] 进程事件带 runtime_epoch；工具回执仍用 `RequestKey`。
- [x] 下一轮复用管理器；切模型/焦点切换不清理（`runtimes_survive_a_completed_turn`）。
- [x] `/new`、切换会话、关窗格、会话删除与退出触发清理（`reap_orphan_runtimes` + 退出 3 秒有界等待）。
- [x] Esc 停止按 D9 清理「本轮新建 ∪ 当前活动交互」（`TurnExecution::stop_targets`）。
- [x] 复用既有「同会话仅一个在途窗格」占用规则。
- [x] 旧实例迟到事件按 `runtime_by_session_id` 过滤，不复活进程。
- [x] 测试超时升级、重复关闭、双进程 reap 与关闭清理。
- [x] 退出 3 秒有界等待；未确认保留 `Interrupted` 并 tracing 记录，不伪报 Stopped。
- [x] 未触及 `tui/` 恢复路径；`tui::tests` 仍全绿。

关键验收：正常结束模型回答保留服务；切换会话确实停止它；旧事件不会污染新会话。

## 7. 计划更新与会话上下文

- [x] 全量校验 plan、最多一个 in_progress、长度限制与清空语义（9 个计划契约测试）。
- [x] 结果携带完整快照 + revision；`FinishTool` 落库成功后才更新当前计划。
- [x] 重载只认最后一条**成功**的 update_plan（`restore_plan`）。
- [x] 历史卡片各自保留该次快照（类型化视图来自各自结果 JSON）。
- [x] 下一轮加入有界存活进程清单与当前计划（`runtime_context_summary`，≤8 进程 / ≤16 步）。
- [x] call/result 顺序与 transport 结构不变；后台输出只更新数据。
- [x] 覆盖计划校验、提交失败、跨轮复用、重载与混合工具。

关键验收：计划显示与持久化结果一致；用户可区分“模型列出的待办”和“已经真正执行的工具”。

## 8. 三类展示组件

### 8.1 CommandCard

- [x] 命令卡片显示状态、命令、cwd（展开）、尾部输出、退出码、时长与截断证据。
- [x] 后台输出更新原卡片；running 不显示已退出（`background_output_updates_the_original_card_only`）。
- [x] 原位展开 + 标题行鼠标命中；展开状态按 `(message_id, call_id)` 保存。
- [x] `^o` 展开当前/最近工具、Alt+↑/↓ 选择（`input/process_keys.rs`）；面板优先级保持。
- [x] 失败/停止/超时/中断分别展示；无输出显示 `Waiting for output`。

### 8.2 ProcessInteractionCard

- [x] 空写 `Read output`、非空 `Send input`、terminate `Stop command`，并关联原命令。
- [x] 输入预览限长并转义换行；保留该次输出快照与状态。
- [x] 无输出显示 `No new output`；后台增量不新增时间线步骤。
- [x] 交互卡片与命令卡片共用原位展开，显示本次调用快照。

### 8.3 PlanCard

- [x] `[ ]` / `[>]` / `[x]` 三类标记 + `N/M completed` 完成计数。
- [x] 长步骤按显示列换行、解释行、空计划 `Plan cleared`、历史多次更新各自保留。
- [x] 不提供点击勾选改状态的功能。

### 8.4 树形版式（三卡统一）

- [x] 三张卡片共用一套树形语法：`■ 动词` 标题 + `  │  ` 中间行 + `  └─ ` 收口，**整卡只有一根树干**。
- [x] 两条前缀等宽 5 列，命令/输入/输出/步骤逐行左对齐（`tree_prefixes_are_equal_width_so_content_stays_aligned`）。
- [x] 命令折叠保留最近 3 行，省略行数显式标为 `… +N more lines`（`folded_preview_states_how_many_lines_are_hidden`）。
- [x] 命令行带 `$ ` 提示符；交互卡片输入永远排在输出之前（`interaction_body_keys_input_before_output_under_the_same_tree`）。
- [x] 计划卡片：有解释时解释行收口，无解释时最后一条步骤收口（`plan_steps_use_the_tree_and_leaf_lands_on_the_explanation`）。
- [x] 进程仍在跑时方块继续呼吸（不因工具已结算而静止）；终态停为实心（`running_process_keeps_breathing_even_after_the_tool_settled`）。
- [x] 运行不足一秒不追加时长（避免 `Running · 1ms` 噪声与逐帧抖动）。

### 8.5 共用布局与 gallery

- [x] 接入 history 同一 `rows`/`layout`/`render` 口径；命中区与绘制共用行生成。
- [x] 文案集中在 `ui/text/agent.rs`；样式复用既有语义 token。
- [x] 宽/窄/零宽/极矮、中文宽字符、超长命令与展开的几何一致性测试。
- [x] 验证 `^o`/Alt 键与浮层优先级、普通方向键仍编辑草稿；点输出正文不折叠。
- [x] UI gallery 新增 `command` / `process` / `plan` 三项，直接调用生产渲染函数。
- [x] 滚动仍由外层历史统一管理（未引入嵌套滚动区）；用户上滚位置不变。

关键验收：不是只显示工具名；三种工具都能从界面看懂其输入、当前状态与结果。

### 8.6 展开后视口不跳走（缺陷修复）

- [x] 修复：点击可展开内容（思考块 / 工具卡片）后视口跳回历史最顶部，导致「点了展不开」。
- [x] 根因：`follow_bottom == true` 时 `history_scroll` 是**过期值**（通常是 0），旧实现
      切换展开时直接把它当顶部行用。
- [x] 方案：新增 `LxChat::viewport_top`（事件循环每帧绘制前写入）与 `pin_lx_viewport`
      （切换前把当前顶部行固化、退出跟随）；`toggle_thinking_at` / `toggle_latest_thinking` /
      `toggle_tool_detail` 内部调用，未知目标不改动状态。
- [x] 附带修复：`chat_hit` 的命中限定在 `view.body` 内，内容行换算出的屏幕行不会再
      溢出到聊天栏/输入框上变成幽灵命中。
- [x] 回归测试：`src/app/update/chat/tests/viewport_anchor.rs`（4 项）。

## 9. 校验与人工验收

- [x] 执行 `cargo test`：**2655 单元测试 + 16 个集成测试二进制全绿**；`cargo fmt --check` 与 `cargo clippy --all-targets -- -D warnings` 无告警。
- [x] 执行 `cargo fmt`，`cargo fmt --check` 通过。
- [x] 执行 `cargo clippy --all-targets -- -D warnings`，无既有问题需说明。
- [x] 全量 gate 已执行：`cargo fmt --check`、`cargo clippy --all-targets -- -D warnings`、
      `cargo test`（全部通过）。提交动作按用户指令在工作区完成后执行，合并需用户确认。
- [x] 单文件均 ≤ 1000 行（拆分 `manager/spawn.rs`、`event/chat/tool_calls.rs`、`timeline.rs`）；`app`/`ui` 不持有进程句柄、不做 IO。
- [x] 同步 `docs/standards/project-directory-structure.md`（v11、进程/计划模块与七工具）。
- [x] 已按证据逐项勾选（未验证项保持未勾选：真实终端观感、真实跨进程重启观感）。
- [ ] 用户 cargo run 验收命令卡片、交互记录、计划、详情与窄窗格。
- [ ] 用户验收开发服务跨轮保留、关会话清理、Esc 停止及重启恢复。

## 10. 完成定义与交付记录

只有 design §1 的完成标准全部满足，才可把本任务标已实施。交付应包含实际改动、针对性验证、已知进程清理限制、未验证平台和需用户确认的终端观感。

当前执行记录（实施完成）：

- 代码检索：codegraph（`available_tools` / `ToolExecutor` / `EventHost` / `history/tool.rs` 等）
  与 codebase memory（项目索引定位）均已实际使用。
- 独立工作区：`.worktrees/lx-agent-command-plan-tools`（分支 `feat/lx-agent-command-plan-tools`）。
- 生产改动：
  - `src/chat/plan.rs`、`src/chat/process/{mod,manager,manager/spawn,output}.rs`：计划契约与
    会话进程管理器（pipe/PTY、双流 drain、有界缓冲、stdin、等待窗口、停止/reap）。
  - `src/chat/tools/{schema,types,output,process_args,plan_args,mod}.rs`：七工具 schema、
    参数校验、唯一结果 JSON 与三种呈现口径。
  - `src/chat/agent_loop/executor.rs`：生产执行器 `AgentTools`（文件/进程/计划分派）与
    本轮停止集合 `TurnExecution`。
  - `src/event/chat/{runtime,process_events,tool_calls,timeline}.rs`：会话运行实例、进程事件
    落库与 UI 更新、Begin/Finish 握手拆分、时间线辅助。
  - `src/db/{mod,migration,service}.rs`、`src/db/chat/process.rs`：SCHEMA_VERSION 10 → **11**、
    `agent_process` 表、revision 单调快照、启动恢复 `interrupted`。
  - `src/app/state/lx/chat/tool_view.rs`、`src/app/update/chat/process.rs`：类型化卡片视图、
    展开状态、键盘选择、计划提交与重载恢复。
  - `src/ui/main_content/lx/history/tool/{command,plan}.rs` + `history.rs`/`lx.rs`：命令卡片、
    交互卡片、计划卡片、标题行命中；`src/ui/overlay/ui_gallery/agent.rs` 复用生产渲染。
  - `src/input`、`src/app/actions.rs`：Native 内 `^o` 与 Alt+↑/↓。
  - `src/platform/mod.rs`：`terminate_process_group`（Unix 进程组 / Windows `taskkill /T`）。
  - `src/chat/prompt.rs`：系统提示更新为真实可用的七个工具。
- 测试：新增计划契约、输出缓冲、进程管理器（真实子进程）、参数校验、类型化视图、
  DB 进程持久化、生产执行器、命令/交互/计划卡片渲染与几何、应用层纯状态、
  进程工具端到端（真实 DB 握手 + 跨轮复用）与键位映射，共 **+120 余项**。
- 验证：`cargo test` 2655 单元 + 16 集成二进制全绿；`cargo fmt --check` 与
  `cargo clippy --all-targets -- -D warnings` 无告警。

后续修订记录（对照 codex harness 与用户反馈）：

- **后台任务改为独立页面（移除 `/stop`，按用户反馈重做）**：
  - 问题：lx 此前只有「切会话 / `/new` / 退出」才清理进程，用户留在会话里时**没有任何**
    办法停掉自己起的进程；`max_running` 报 `resource_limit` 时也无出路。
  - 第一版做成 `/stop` 两级浮层，用户判定不合格并给出两条明确要求，据此重做：
    1. 必须是**独立页面**，且**只在该窗格内全屏**（不能全局覆盖，也不能清理 lx 页）；
    2. 移除 `/stop` 命令，改为输入框底部用量**右侧的计数指标**，点它/按 `↓` 进页面；
    3. 页面里要**预留 subagent 的位置**（后续在此显示 subagent 内容）。
  - 形态（对齐 opencode 的 composer 多 tab）：顶边框 `Background` + 右侧 tab 指示
    `Processes · Agents`，主体是当前 tab 的列表，底边框是键位提示。
  - **不做全局覆盖**：页面挂在 `Lx::task`（`Option<TaskPage>`），绘制时只接管本窗格内容区
    （`ui/main_content/lx.rs::render` 早返回）。因此 `pane.lx` 的会话、消息、草稿、光标、
    滚动偏移全部原样保留，`esc` 返回即回到离开时的样子——**不需要保存/恢复任何状态**，
    也绝不会清理 lx 页。测试 `opening_the_page_keeps_every_other_lx_field` 逐字段锁住。
  - 键位（对齐 opencode `composer.*`）：`↑/↓` 移动、`^d` 停止高亮进程（`composer.shell.kill`）、
    `←/→` 换 tab、`esc` 返回。页面整页接管按键，字符不会漏进草稿。
  - 入口：**空草稿按 `↓`**（沿用「历史先接管、否则继续」结构，有草稿时 `↓` 仍是光标移动），
    与 opencode `session.child.first = down` 同一机制。
  - 底部指标：输入框底边框右端（用量右侧）显示 `1 process` / `3 processes`，**0 个时整段隐藏**；
    窄到放不下时让位给模型名。由 `event::chat::sync_background_task_counts` 每轮同步，
    与页面是否打开无关——这补上了"进程还活着但界面上没痕迹"的根因。
  - 数据来源：`Effect::ListSessionProcesses` / `Effect::StopSessionProcess` 由事件层执行
    （`app/` 与 `db/` 都不持有进程句柄），与 `/resume` 同一套取数模式。
  - **刷新时机**：列表由**进程终态事件**驱动（`process_events::refresh_task_pages`），
    不同步重读——停止是异步的，同步刷新会读到停止前的快照（实测踩到并修掉）。
  - `Agents` tab 是**预留位**：列表恒空并显示 `no subagents yet`，后续在此显示 subagent 内容。
  - 测试：`app/state/lx/task/tests.rs`（10 项状态模型）+ `app/update/chat/tests/task_page.rs`
    （16 项：进入/退出不丢 lx 状态、列表回投、停止效应、`↓` 入口、按键不漏进草稿、tab 导航）
    + `ui/main_content/lx/task/tests.rs`（8 项渲染，含窄窗格降级与 tab 占位）
    + `status/tests.rs`（4 项底部指标）+ 真实子进程端到端 1 项。

- **非零退出被误判为失败（bug，按用户反馈修复）**：
  - 现象：一张正常的 `■ Run command` 卡片显示成**红色**，标题为 `exit 1`。
  - 真实会话（`agent_tool_call` id=98）：`pgrep -af "IFS= read" ; echo ---; ps -ef |
    grep -c "[r]ead -r line"`，输出 `35293 / --- / 0`，退出码 1——`grep -c` 无匹配的正常返回。
    模型自己正确读懂了，界面却把它标成失败。
  - 根因：三层都把「进程以非零码退出」等同于「工具失败」——`ProcessResult::is_failure`
    → 工具结算 `failed`；`apply_process_exit` 写入 `error = "exit 1"`；`ToolStatus::Failed`
    → 红方块 + 红色动词。
  - 修法：拆成两个互不替代的概念。
    - **进程级失败**（`is_process_failure`）= 超时 / 中断 / 启动失败 → 仍然红。
    - **非零退出**（`is_nonzero_exit`）= 命令跑完并给出结果 → 结算 `completed`、
      不写失败原因、方块保持工具色；**退出码照常显示**在标题行（`exit 1`）。
  - 与 codex 口径一致：其 exec 渲染对 search 类命令的 `exit 1` 用 `dim()` 而非 `red()`，
    注释为 "Search exit 1 can mean no matches; report the code without calling it a failure"。
  - 回归测试四层：`process/manager`（真实子进程 `grep -c` 无匹配）、`agent_loop/executor`、
    `event/chat` 端到端（落库 `completed`）、UI 渲染（方块非红且 `exit 1` 可见），
    另加"超时仍是红"的反向断言防止放宽过头。

- **展开后视口跳走（bug）**：新增 `LxChat::viewport_top` + `pin_lx_viewport`，三个切换入口
  内部先锚定；`chat_hit` 命中限定在 body 内。测试 `app/update/chat/tests/viewport_anchor.rs`。
- **三卡改为树形版式**：`ui/text/agent.rs` 新增 `TREE_BAR` / `TREE_LEAF`（等宽 5 列）与
  `EARLIER_OUTPUT_HINT` / `COMMAND_PROMPT_PREFIX`；`history/tool/{command,plan}.rs` 重写为
  「标题 + 树体」，`rows` 与 `draw` 共用同一份 `BodyRow` 构造（行数口径唯一来源）。
- **对齐 codex harness**：
  - `login` 默认改为 `true`；`exec_command` 默认等待 10s、上限 30000（`write_stdin` 独立
    默认 250ms，`DEFAULT_STDIN_YIELD_MS`）；
  - 结果新增 `chunk_id`（6 位十六进制，`generate_chunk_id`）与 `wall_time_ms`（JSON 输出为
    `wall_time_seconds`），模型文本同步显示；
  - `ToolExecutionResult::Process` 装箱（新增字段后触发 clippy `large_enum_variant`）。
  - 有意不跟随：沙箱/审批、并行工具调用、registry/router/runtimes 分层、transcript 全屏、
    `environment_id`（理由见 design §14.2）。
- 已知限制：无沙箱（D3 已确认）；输出保留有界，详情只能显示保留尾部；PTY 不做全屏
  终端仿真；显式脱离进程组的守护进程不保证被完整追踪；异常强杀 lx 不声称能优雅清理后代。
- 未验证：真实终端观感（颜色/对齐/窄窗格）与真实跨重启的用户操作体验，需用户 `cargo run` 验收。
