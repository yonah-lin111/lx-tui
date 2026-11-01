# lx Exec（终端无头直接执行 Agent）— 执行任务

> 状态：**已在 `.worktrees/lx-exec`（分支 `feat/lx-exec`）全部实现并通过验证（2026-11-01）**。
> 规则：生产代码禁止 `unwrap()/expect()`；用户可见文案与模型提示词一律英文；中文仅用于代码注释与日志；
> 生产代码单文件 ≤ 1000 行（拆分子模块控制在 ~300–520 行），测试文件严格跟随被测子模块组织；每节**先写测试再写实现**。
> 每节完成后按影响范围跑 `cargo fmt` + `cargo clippy --all-targets -- -D warnings` + `cargo test`。

---

## 0. 基线核对

- [x] 确认当前分支与工作区状态，确认 `SCHEMA_VERSION == 22`（`src/db/mod.rs:27`）、`PROMPT_VERSION == "lx-9"`（`src/chat/prompt.rs`）
- [x] 确认 `MAIN_TOOLS.len() == 17`（`src/chat/agent/mod.rs:90`），包含 `"ask"` 与 7 个子 agent 控制工具
- [x] 确认本功能无需修改 `SCHEMA_VERSION`（零表结构迁移）

---

## 1. 工具面与内存库支持：`src/chat/agent/mod.rs` + `src/chat/events.rs` + `src/db/mod.rs`（修改，~80 行）

- [x] `src/chat/agent/mod.rs`：
  - `ToolSurface` 新增变体 `ExecMain`
  - 新增常量 `EXEC_MAIN_TOOLS: [&str; 16]`（等于 `MAIN_TOOLS` 去掉 `"ask"`，顺序保持一致）
  - `ToolSurface::names(self)` 对 `Self::ExecMain` 返回 `&EXEC_MAIN_TOOLS`
  - 新增稳定错误码 `pub const EXEC_ASK_UNSUPPORTED_CODE: &str = "ask is not supported in exec mode; proceed with best-effort assumptions";`
- [x] `src/chat/events.rs`：
  - `RequestPurpose` 新增 `ExecTurn`
  - `RequestPurpose::tools_surface(self)` 对 `Self::ExecTurn` 返回 `Some(ToolSurface::ExecMain)`
- [x] `src/db/mod.rs`：
  - `Database::open_in_memory() -> Result<Self>`：使用 `Connection::open_in_memory()` 开启外键并执行 `migrate()`，供 `--ephemeral` 模式与测试使用

测试（`src/chat/agent/tests.rs` & `src/chat/transport/tools/tests.rs` & `src/db/tests.rs`）：
- [x] `ToolSurface::ExecMain.names()` 长度断言为 16，不包含 `"ask"`，包含其余全部 16 项工具
- [x] 四协议（OpenAI / Responses / Anthropic / Google）在 `RequestPurpose::ExecTurn` 下生成的工具声明不含 `"ask"`
- [x] `Database::open_in_memory()` 成功完成完整 schema 迁移且支持 `begin_turn` / `begin_tool_call` / `finish`

---

## 2. CLI 参数解析与 Stdin 合成：`src/chat/agent/exec/cli.rs`（新文件，~420 行）

纯函数解析器，不引入外部重型 CLI 库，无 IO 副作用（stdin 通过注入闭包读取）。

- [x] 定义 `CliAction`（`Tui` / `Exec(ExecCli)` / `Help(HelpTopic)` / `Version`）
- [x] 定义 `ExecCli`、`ExecCommand::Resume(ResumeArgs)`、`ColorMode`（`Auto` / `Always` / `Never`）
- [x] 实现 `pub fn parse_cli<I, T>(args: I) -> Result<CliAction, String>`（`I: IntoIterator<Item = T>, T: Into<OsString>`）：
  - 无参 → `Ok(CliAction::Tui)`
  - 顶层 `-h` / `--help` / `help` → `Ok(CliAction::Help(HelpTopic::Root))`
  - 顶层 `-V` / `--version` → `Ok(CliAction::Version)`
  - `exec` 子命令支持：
    - `-C <DIR>` / `--cd <DIR>`
    - `-m <MODEL>` / `--model <MODEL>`
    - `-o <FILE>` / `--output-last-message <FILE>`
    - `-i <FILE>` / `--image <FILE>`（支持多次传入）
    - `--json`
    - `--ephemeral`
    - `--color <auto|always|never>`
    - `-h` / `--help` → `Ok(CliAction::Help(HelpTopic::Exec))`
    - `resume` 子命令：`[SESSION_ID]`、`--last`、`-i/--image`、`[PROMPT]`，以及 `resume --help`
  - 校验 `--ephemeral` 与 `resume` 互斥；未知选项返回清晰英文错误
- [x] 实现 `pub fn help_text(topic: HelpTopic) -> &'static str`（纯英文 CLI 帮助文本）
- [x] 实现 `pub fn prompt_with_stdin_context(prompt: &str, stdin_text: &str) -> String`
- [x] 实现 `pub fn resolve_prompt_input(...) -> Result<String, String>`（支持位置参数、管道 stdin、`"-"` 强制 stdin、位置参数 + 管道 `<stdin>` 自动追加）
- [x] 实现模型选择解析辅助函数 `pub fn resolve_model_selection(models: &ModelsSnapshot, raw_model: Option<&str>, session_model: Option<(&str, &str, Option<&str>)>) -> Result<ModelSelection, String>`

测试（`src/chat/agent/exec/tests/cli_tests.rs`）：
- [x] 空参数返回 `CliAction::Tui`
- [x] `exec` 全量参数组合（短选项、长选项、`--key=value` 形式、位置参数前后顺序）解析正确
- [x] `exec resume --last "fix tests"` 与 `exec resume sess-abc "fix tests"` 解析正确
- [x] `--ephemeral` 搭配 `resume` 报错拒绝；未知参数报错拒绝
- [x] `resolve_prompt_input` 四种场景（纯位置参数、纯管道、`"-"`、位置参数 + 管道追加 `<stdin>`）及空输入报错断言
- [x] `resolve_model_selection` 支持 `provider/model#variant`、`provider/model`、短 `model_id` 以及默认模型回退

---

## 3. 输出事件模型与双模式处理器：`src/chat/agent/exec/events.rs` + `processor.rs`（新文件，~280 行 + ~480 行）

### 3.1 `src/chat/agent/exec/events.rs`（~280 行）
- [x] 定义 JSONL 序列化类型：`ThreadEvent`、`ThreadItem`、`ThreadItemDetails`、`ItemStatus`、`FileChangeEntry`、`TodoEntry`、`ExecUsage`、`ThreadError`
- [x] 实现从 `ReadyToolCall` / `ToolOutcome` / `SubagentOutcome` / `Usage` 到 `ThreadItemDetails` 与 `ExecUsage` 的纯函数映射（自动识别 `exec_command` / `write_stdin` 为 `CommandExecution`，`edit` / `write` 为 `FileChange`，MCP 工具为 `McpToolCall`，子 agent 工具为 `CollabToolCall`，`web_search` 为 `WebSearch`，`update_plan` 为 `TodoList`）

### 3.2 `src/chat/agent/exec/processor.rs`（~480 行）
- [x] 定义 `SessionHeader<'a>` 与 `ExecEventProcessor` trait
- [x] 实现 `HumanOutputProcessor<WOut: Write + Send, WErr: Write + Send>`：
  - 根据 `ColorMode` 与 `stderr_is_terminal` 控制 ANSI 样式（bold / dim / cyan / green / red / yellow）
  - `on_session_started`：向 `stderr` 输出 `workdir`、`model`、`provider`、`session id` 与用户 prompt 摘要
  - `on_reasoning`：向 `stderr` 输出 `● Thinking...` 与壓暗的思考内容
  - `on_tool_started` / `on_tool_finished` / `on_subagent_finished`：向 `stderr` 输出 `■ <tool>` 与 `└─ <summary> (<duration>ms)`
  - `on_assistant_text`：在 TTY 模式下向 `stderr` 输出中间或最终回复文本
  - `on_turn_finished`：
    - 向 `stderr` 输出 `tokens used` 统计行（或失败/中断错误提示）
    - 当 `!stdout_is_terminal` 且存在 `final_message` 时，将纯文本 `final_message` 写入 `stdout`
    - 当配置了 `last_message_file` 时，调用 `platform::write_file_atomic` 将 `final_message` 写入目标路径
- [x] 实现 `JsonlOutputProcessor<WOut: Write + Send>`：
  - 维护单调自增的 `next_item_id`（`"item_0"`, `"item_1"`, ...）
  - 在各个生命周期节点向 `stdout` 写入单行 JSON 并 `flush()`
  - 同样支持 `last_message_file` 落盘

测试（`src/chat/agent/exec/tests/processor_tests.rs`）：
- [x] `HumanOutputProcessor` 在非 TTY `stdout` 下：`stderr` 包含完整工具与思考进度，`stdout` **只包含**最终回复文本（逐字节相等）
- [x] `HumanOutputProcessor` 在 `ColorMode::Never` 下输出不含 `\x1b[`
- [x] `-o / --output-last-message` 正确创建父目录并写入最终回复文本
- [x] `JsonlOutputProcessor` 输出的每行均可由 `serde_json::from_str::<ThreadEvent>` 解析，且覆盖 `thread.started` → `turn.started` → `item.started` → `item.completed` → `turn.completed` 全链路

---

## 4. 无头运行器与子 Agent 调度：`src/chat/agent/exec/runner.rs` + `subagent.rs`（新文件，~520 行 + ~450 行）

### 4.1 `src/chat/agent/exec/runner.rs`（~520 行）
- [x] 定义 `ExecLoopEvent` 与实现 `AgentHost` 的 `ExecHost`（包含 `MutationRecorder` 的无头通道实现，支持文件修改前后的 DB 快照记录）
- [x] 实现 `ExecRunner`：
  - 持有 `Database`、`workspace: PathBuf`、`cwd: PathBuf`、`session_id: String`、`user_message_id: String`、`assistant_id: String`、`parts: Vec<ChatPart>`、`ProcessManager`、`ReadRegistry`、`Box<dyn ExecEventProcessor>`
  - 处理 `ExecLoopEvent::MainDelta`：按 `PartKind` 聚合流式文本与思考块，闭合时通知 `processor`，按 8 KiB / 500ms 节流调用 `db.checkpoint`
  - 处理 `AgentHostRequest::BeginTool`：`db.begin_tool_call` → `db.mark_tool_running` → `processor.on_tool_started` → 回执 `ToolCallTicket`
  - 处理 `AgentHostRequest::FinishTool`：`db.finish_tool_call` → `processor.on_tool_finished` → 回执 `Ok(())`
  - 处理 `AgentHostRequest::AskUser`：直接回执 `Err(HostError::Failed { code: EXEC_ASK_UNSUPPORTED_CODE.into() })`
  - 处理 `AgentHostRequest::UserShellUpdates`：回执空 `Vec::new()`
  - 处理 `ExecLoopEvent::AutoCompact`：写入 `db.insert_compact_checkpoint` 并通知 `processor.on_warning`
  - 收尾清理：调用 `db.finish`、关闭子 agent 池、调用 `process_manager.shutdown_all().await`

### 4.2 `src/chat/agent/exec/subagent.rs`（~450 行）
- [x] 实现 `ExecSubagentPool`：
  - 支持 `SubagentOp::{Spawn, SendInput, Resume, Wait, Close, List, CancelInput}`
  - `Spawn`：检查并发数 `< MAX_CONCURRENT_SUBAGENTS`（3） → `db.begin_subagent_turn_with_items` → 创建子会话专属 `ProcessManager` 与 `ReadRegistry` → `tokio::spawn` 运行子 agent 的 `run_agent_turn`
  - 子 agent 的 `BeginTool` / `FinishTool` 落库到子会话记录
  - `Wait`：支持 `WaitMode::All` 与 `WaitMode::Any` 及超时定时器；子 agent 完成时提取报告 `ReportPayload`、标记 `mark_report_delivered` 并唤醒等待者
  - `shutdown_all`：取消所有在途子 agent 并终止其 `ProcessManager`

测试（`src/chat/agent/exec/tests/runner_tests.rs`）：
- [x] 单轮纯文本响应：DB 消息状态为 `complete`，`stdout` 得到最终回复，返回 `EXIT_OK`
- [x] 工具调用链（`read` → `edit` → `exec_command`）：文件落盘修改成功，DB `agent_tool_call` 记录 3 条 `completed`
- [x] `ask` 工具拦截测试：假模型流发出 `ask` 调用时收到失败回执并继续下一轮完成，不阻塞
- [x] 子 agent 协同测试：假模型流发出 `spawn_agent(explorer)` + `wait_agent` → 子 agent 执行 `read` 后汇报 → 主 agent 拿到报告并完成
- [x] 并发上限测试：连续 `spawn_agent` 第 4 个时返回 `LIMIT_REACHED_CODE`

---

## 5. 顶层编排与入口接线：`src/chat/agent/exec/mod.rs` + `src/main.rs`（新文件 ~350 行 + 修改 ~40 行）

- [x] `src/chat/agent/exec/mod.rs`：
  - 导出 `CliAction`、`ExecCli`、`HelpTopic`、`EXIT_OK`、`EXIT_ERR`、`EXIT_INTERRUPTED`、`parse_cli`、`help_text`、`run_main`
  - 实现 `pub fn run_main(cli: ExecCli, linked_worktree: Option<&Path>) -> Result<i32, Box<dyn std::error::Error>>`：
    1. 解析工作目录 `cwd`（`cli.cwd` 优先，回退 `std::env::current_dir()`）并解析工作区根 `workspace`
    2. 读取并合成 prompt 与 `-i / --image` 附件（调用 `prepare_user_message`）
    3. 加载 `Config`、`AiSettings` 与 `ModelsSnapshot`；初始化全局 `AGENTS.md`、加载工作区指令 `load_instructions` 与 `skills::catalog_prompt`
    4. 打开数据库（`cli.ephemeral` 走 `Database::open_in_memory()`，否则走 `Database::open_default(linked_worktree)`）
    5. 若为 `ExecCommand::Resume`：定位目标 `session_id`（指定 ID 或 `--last`），检查 `try_lock_session` 与恢复遗留未完成状态
    6. 解析 `ModelSelection` 与 `FrozenModel`；调用 `db.begin_turn_with_attachments` 开启本轮
    7. 构造 Tokio 多线程 Runtime，注册 `tokio::signal::ctrl_c()` 监听，启动 `ExecRunner`
    8. 根据 `TurnOutcome` 返回 `EXIT_OK` (0)、`EXIT_ERR` (1) 或 `EXIT_INTERRUPTED` (130)
- [x] `src/main.rs`：
  - 接入 `parse_cli(std::env::args_os())`，按 `CliAction` 分发 `Help` / `Version` / `Exec` / `Tui`

测试（`src/chat/agent/exec/tests/runner_tests.rs`）：
- [x] `--ephemeral` 模式不污染磁盘数据库
- [x] `resume --last` 跨轮次恢复上下文测试
- [x] `Ctrl+C`（取消通道触发）中断在途轮次，DB 记录 `interrupted` 且返回 `EXIT_INTERRUPTED` (130)

---

## 6. 收尾与验证

- [x] `cargo fmt --check`
- [x] `cargo clippy --all-targets -- -D warnings`
- [x] `cargo test`（重点运行 `chat::agent::exec`、`chat::agent`、`chat::transport::tools`、`db` 相关测试）
- [x] 检查所有新增/修改的生产代码文件行数均 ≤ 1000 行，且无 `unwrap()` / `expect()`
- [x] 核对 `docs/standards/project-directory-structure.md` 是否需要同步补充 `chat/agent/exec/` 说明

---

## 依赖顺序（不可交换）

```text
1. 工具面 ExecMain + open_in_memory
        │
        ├─> 2. CLI 解析器与 Stdin 合成 (cli.rs)
        │
        ├─> 3. 输出事件与处理器 (events.rs + processor.rs)
        │
        └─> 4. 无头运行器与子 Agent 池 (runner.rs + subagent.rs)
                    │
                    ▼
            5. 顶层编排与 main.rs 入口接线 (exec/mod.rs + main.rs)
                    │
                    ▼
            6. 格式化 / Clippy / 单元与集成测试验证
```

## 预估工作量

| 节 | 模块 / 文件 | 性质 | 估算行数 |
|----|-------------|------|----------|
| 1 | `src/chat/agent/mod.rs`、`src/chat/events.rs`、`src/db/mod.rs` | 修改 | ~80 |
| 2 | `src/chat/agent/exec/cli.rs` | 新文件 | ~420 |
| 3 | `src/chat/agent/exec/events.rs`、`src/chat/agent/exec/processor.rs` | 新文件 | ~760 |
| 4 | `src/chat/agent/exec/runner.rs`、`src/chat/agent/exec/subagent.rs` | 新文件 | ~970 |
| 5 | `src/chat/agent/exec/mod.rs`、`src/main.rs` | 新文件 + 修改 | ~390 |
| 测试 | `src/chat/agent/exec/tests/{mod,cli_tests,processor_tests,runner_tests}.rs` | 新文件 | ~950 |
| **合计** | **7 个生产文件（5 新 + 3 改）+ 4 个测试文件** | | **~3,570** |
