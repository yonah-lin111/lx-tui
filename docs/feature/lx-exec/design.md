# lx Exec（终端无头直接执行 Agent）— 设计

- 分支：`feat/lx-exec`（独立 worktree `.worktrees/lx-exec`）
- 基线：功能分支基于 `dev @ b8df8c0d`（`SCHEMA_VERSION = 22`，`PROMPT_VERSION = "lx-9"`）
- 状态：**已在 `feat/lx-exec` 实现并通过验证（2026-11-01），待用户确认合并**
- 参考：`codex-main/codex-rs/exec/`（`cli.rs`、`lib.rs`、`event_processor.rs`、`event_processor_with_human_output.rs`、`event_processor_with_jsonl_output.rs`、`exec_events.rs`）

---

## 0. 现状核实（Codegraph & Codebase Memory 事实基线）

在编写本设计前，已对 `lx-tui` 与 `codex-main` 进行代码图谱核实，核心事实如下：

| 维度 | `lx-tui` 当前事实（已核实） | `codex-main` 参考实现（已核实） |
|------|----------------------------|--------------------------------|
| 入口与 CLI 解析 | `src/main.rs:14-50` 完全不解析 `std::env::args()`，未引入 `clap`；启动直接执行 `startup_trust` → `Tui::init()` → `event::run()` | `codex-rs/cli/src/main.rs` 顶层分发 `Subcommand::Exec`，支持 `codex exec [OPTIONS] [PROMPT]` 与 `codex exec resume` |
| Agent 循环解耦 | `src/chat/agent_loop/mod.rs:645` 的 `run_agent_turn` 只依赖 `ModelContext`、`AgentHost`、`ToolExecutor`、`stream_one` 与 `watch::Receiver<bool>`，**零依赖 `app/` 与 `ui/`** | `codex-rs/exec/src/lib.rs` 通过事件流驱动 `EventProcessor`（Human / JSONL 两套实现），不启动 TUI |
| 工具面（`ToolSurface`） | `src/chat/agent/mod.rs:80-141` 定义 `Main`（17 项，含 `ask` 和 7 个子 agent 控制工具）、`Explorer`（3 项）、`Executor`（7 项） | `exec` 模式默认 `AskForApproval::Never`，对 `ToolRequestUserInput`（`ask_user`）直接拒绝，防止无头挂起 |
| 持久化与恢复 | `src/db/mod.rs:27` `SCHEMA_VERSION = 22`；`Database::begin_turn_with_attachments`、`checkpoint`、`begin_tool_call`、`finish_tool_call`、`finish` 均直接操作 SQLite，不依赖 TUI 窗格 | 默认持久化会话并支持 `codex exec resume [SESSION_ID \| --last] [PROMPT]`；`--ephemeral` 跳过持久化 |
| 标准输出隔离 | `docs/standards/code-writing-standards.md:33` 规定 TUI 运行期间禁止写 `stdout`；无头 CLI 模式不启动 `ratatui`，需在 `chat::agent::exec` 模块显式承担终端标准流输出 | `codex-rs/exec` 人类可读模式将进度/思考/工具打到 `stderr`，非 TTY 管道下最终回复打到 `stdout`；`--json` 模式将 JSONL 事件打到 `stdout` |

---

## 1. 目标与范围

参照 `codex-main/codex-rs/exec`，在 `src/chat/agent/`（及相关编排层）实现**不用打开 `lx-tui` 全屏界面、直接在终端运行 `lx` agent** 的无头执行模式（`lx-tui exec`）：

1. **无参保持 TUI，带 `exec` 直跑**：
   - `lx-tui`（无参数）：完全保持现有行为，进入全屏 TUI。
   - `lx-tui exec [OPTIONS] [PROMPT]`：不初始化 `Tui`（不开启备用屏幕与 raw mode），直接在当前终端运行 `lx` agent 轮次，执行完毕后退出。
   - `lx-tui exec resume [SESSION_ID | --last] [OPTIONS] [PROMPT]`：在终端恢复已有会话（或最近一次会话）继续执行新一轮。
2. **输入源支持（对齐 Codex `resolve_root_prompt`）**：
   - 支持位置参数 `PROMPT`；
   - 未传 `PROMPT` 或传入 `"-"` 时从管道 `stdin` 读取提示词；
   - 同时提供位置参数 `PROMPT` 且 `stdin` 为管道时，自动将管道内容包裹为 `<stdin>\n...\n</stdin>` 追加在提示词末尾；
   - 支持 `@path`、`@[refer-*]` 引用解析与 `-i / --image <FILE>` 本地图片附件。
3. **双输出模式与 `stdout` / `stderr` 严格分流**：
   - **默认人类可读模式（`HumanOutput`）**：头部摘要（workdir / model / provider / session id）、思考过程、工具调用开始与完成（含耗时与退出码/变更文件）、子 agent 状态、token 统计全部输出到 `stderr`；当 `stdout` 是管道/重定向时，将最终回复正文纯净输出到 `stdout`（TTY 下最终回复同样在 `stderr` 呈现）；支持 `-o / --output-last-message <FILE>` 将最终回复写入指定文件。
   - **JSONL 结构化流模式（`--json`）**：向 `stdout` 逐行输出强类型 JSON 事件（`thread.started`、`turn.started`、`item.started`、`item.updated`、`item.completed`、`turn.completed`、`turn.failed`、`error`）。
4. **无头工具面与子 Agent 完整支持**：
   - 新增 `ToolSurface::ExecMain`（16 项内置工具 = `Main` 的 17 项去掉交互式 `ask`），执行层兜底拒绝 `ask`（返回 `"ask is not supported in exec mode; proceed with best-effort assumptions"`），杜绝无头挂起。
   - 完整支持文件工具（`read`/`search`/`edit`/`write`）、进程工具（`exec_command`/`write_stdin`）、计划工具（`update_plan`）、时间与联网搜索（`curr_time`/`web_search`）、MCP 工具以及子 agent 并发（`spawn_agent`/`wait_agent`/`send_input`/`resume_agent`/`close_agent`/`list_agents`/`cancel_agent_input`，最多 3 并发）。
5. **会话持久化与 `--ephemeral` 无痕模式**：
   - 默认写入当前工作区对应的 SQLite 数据库（`source = 'native'`），与 TUI `/resume` 列表完全互通；
   - 指定 `--ephemeral` 时使用内存数据库（`Database::open_in_memory()`），全程不写磁盘历史。
6. **工作区免交互放行、`Ctrl+C` 优雅中断与标准退出码**：
   - `exec` 模式跳过交互式启动信任门；
   - 监听 `tokio::signal::ctrl_c()`，触发时中断网络流、等待在途原子文件写入收尾、终止 `ProcessManager` 全部子进程与运行中子 agent、将 DB 轮次与工具状态结算为 `interrupted` / `cancelled`；
   - 退出码：成功 `0`，运行/参数/模型错误 `1`，`Ctrl+C` 中断 `130`。

### 本次不做

- 不引入外部重型 CLI 解析依赖（如 `clap` derive 宏全家桶），使用标准库 `std::env::args_os` 手写零依赖、可完全单元测试的纯函数解析器（符合 `docs/standards/code-writing-standards.md:66` “先确认标准库与现有依赖无法覆盖；禁止为小功能引入重依赖”）。
- 不做 `codex exec review` 专用代码审查子命令或 `codex exec fork` 会话分叉子命令。
- 不做 `--output-schema` 结构化 JSON Schema 强制输出约束。
- 不修改现有 SQLite `SCHEMA_VERSION`（当前 `v22` 表结构已完全满足 `exec` 会话、消息、工具与子 agent 存储，**零数据库迁移**）。

---

## 2. 已确认决策（Grill Me 收口，5 轮）

| # | 决策项 | 结论 |
|---|--------|------|
| 1 | CLI 触发与参数 | 采用 `lx-tui exec` 子命令触发无头执行；无参保持直进全屏 TUI；支持位置提示词、管道 `stdin`、`-C/--cd <DIR>`、`-m/--model <MODEL>`、`-o/--output-last-message <FILE>`、`-i/--image <FILE>`、`--color <auto\|always\|never>` |
| 2 | 终端输出与分流 | 默认人类可读模式：过程信息（配置头、思考、工具、子 agent、token 用量）走 `stderr`，最终回复在管道重定向下纯净写 `stdout`；支持 `--json` 模式向 `stdout` 逐行输出 JSONL 事件流 |
| 3 | 工具面与子 Agent | 禁止交互式 `ask` 工具（`ToolSurface::ExecMain` 不暴露 `ask`，执行层二次兜底拒绝）；完整保留文件、进程、计划、联网、MCP 与最多 3 并发子 agent 能力 |
| 4 | 会话持久化与恢复 | 默认落库到当前工作区 SQLite（与 TUI `/resume` 互通），支持 `lx-tui exec resume [--last \| <SESSION_ID>] [PROMPT]` 续跑；支持 `--ephemeral` 纯内存无痕运行 |
| 5 | 信任门、中断与退出码 | 显式运行 `exec` 直接放行不弹交互信任页；`Ctrl+C` 优雅取消网络流、收割 `ProcessManager` 子进程与子 agent、DB 落库 `interrupted`；退出码 `0`（成功）/ `1`（失败）/ `130`（SIGINT 中断） |

---

## 3. 模块划分与文件布局

遵循 `docs/standards/project-directory-structure.md` 与单文件 ≤ 1000 行（子模块控制在 ~400-500 行）规范，在 `src/chat/agent/` 下组织 CLI 解析、输出事件模型与无头宿主运行器：

```text
src/
  main.rs                           修改：入口解析 CliAction（Tui / Exec / Help / Version），分发 TUI 或 exec::run_main
  chat/
    agent/
      mod.rs                        修改：ToolSurface 新增 ExecMain（16 项，不含 ask），导出 exec 子模块
      exec/
        mod.rs                      新增：exec 顶层入口 run_main、工作区/模型/数据库初始化、历史图片水合、
                                     Skills 目录与 `$skill` 引用块注入、Ctrl+C 信号与退出码收敛
        cli.rs                      新增：零依赖命令行参数解析（ExecCli / ExecCommand / ColorMode）与 stdin 提示词合成
        events.rs                   新增：JSONL 强类型事件定义（ThreadEvent / ThreadItem / ThreadItemDetails / ExecUsage）
        processor.rs                新增：EventProcessor trait、HumanOutputProcessor（stderr 进度 + stdout 终态）与 JsonlOutputProcessor
        runner.rs                   新增：ExecHost（实现 AgentHost）+ 无头会话状态机（直接驱动 Database、ProcessManager、McpManager 与子 agent）
        mutation.rs                 新增：文件变更持久化桥（ExecMutationEvent 与 MutationRecorderFactory）
        subagent/
          mod.rs                    新增：无头子 agent 池状态与控制入口（spawn / send_input / resume / wait / close / list / cancel_input）
          turn.rs                   新增：子轮上下文装配、四段提示词注入、图片归档与历史图片回放
          settle.rs                 新增：`wait_agent` 超时续等 `auto_continue`、子轮终态结算与批量关闭
        tests/
          mod.rs                    新增：单元测试入口
          fixtures.rs               新增：运行器与子 agent 测试共用脚手架（TestDir / ScriptedFactory / PNG 夹具）
          cli_tests.rs              新增：CLI 参数、stdin 组合、resume 参数解析测试
          processor_tests.rs        新增：Human / JSONL 输出格式化与 stdout/stderr 隔离测试
          events_tests.rs           新增：JSONL 事件形状与工具摘要映射测试
          runner_tests.rs           新增：假模型流下的单轮、工具调用、ask 拒绝、ephemeral、resume 与 Ctrl+C 中断测试
          subagent_tests.rs         新增：子 agent 派发、并发上限、等待续等与结算测试
```

### 依赖方向核对

- `src/chat/agent/exec/` 仅依赖：
  - `crate::chat`（`agent_loop`、`client`、`context`、`events`、`prepare`、`process`、`prompt`、`tools`、`transport`）
  - `crate::config`（`Config`、`ai::AiSettings`、`ModelSelection`）
  - `crate::db`（`Database`、`ModelSnapshot`、`chat::*`）
  - `crate::mcp`、`crate::skills`、`crate::git`、`crate::platform`
- **绝对不依赖 `crate::app`、`crate::ui`、`crate::tui`、`crate::event`、`crate::input`、`crate::pty`、`crate::terminal`**。
- 完全符合 `docs/standards/project-directory-structure.md:169` 中 `chat -> db, config, platform, serde/reqwest` 的依赖方向！

---

## 4. 关键数据结构与核心代码设计

### 4.1 工具面扩展（`src/chat/agent/mod.rs`）

在 `ToolSurface` 中新增 `ExecMain`，从 `MAIN_TOOLS`（17 项）中剔除 `"ask"`，保留其余 16 项工具（含全部 7 个子 agent 控制工具）：

```rust
/// 工具面：决定一次请求声明哪些工具。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolSurface {
    /// TUI 主 agent：现有内置工具 + 7 个控制工具（17 项）。
    Main,
    /// CLI `exec` 无头主 agent：剔除交互式 `ask` 后的全部主工具（16 项，decision 3）。
    ExecMain,
    /// 只读调查（3 项）。
    Explorer,
    /// 有边界的实现（7 项）。
    Executor,
}

/// CLI `exec` 主 agent 工具名（16 项：不含 `ask`，顺序与 `MAIN_TOOLS` 一致）。
const EXEC_MAIN_TOOLS: [&str; 16] = [
    "read",
    "search",
    "web_search",
    "edit",
    "write",
    "exec_command",
    "write_stdin",
    "update_plan",
    "curr_time",
    "spawn_agent",
    "send_input",
    "resume_agent",
    "wait_agent",
    "close_agent",
    "list_agents",
    "cancel_agent_input",
];

impl ToolSurface {
    pub const fn names(self) -> &'static [&'static str] {
        match self {
            Self::Main => &MAIN_TOOLS,
            Self::ExecMain => &EXEC_MAIN_TOOLS,
            Self::Explorer => &EXPLORER_TOOLS,
            Self::Executor => &EXECUTOR_TOOLS,
        }
    }
}
```

同时在 `src/chat/events.rs` 的 `RequestPurpose` 中新增 `ExecTurn`（或让 `RequestPurpose::tools_surface()` 对 `ExecTurn` 返回 `Some(ToolSurface::ExecMain)`），保证四套 transport（OpenAI Chat / OpenAI Responses / Anthropic / Google）在构造请求工具列表时自动剔除 `ask`。

在 `src/chat/agent_loop/mod.rs` 中，当 `call.name == "ask"` 时：
1. 由于 `ExecMain` 未声明 `ask`，`allowed` 检查（`agent_loop/mod.rs:739`）已会拦截；
2. `ExecHost::ask_user` 额外兜底返回 `Err(HostError::Failed { code: EXEC_ASK_UNSUPPORTED_CODE.to_string() })`，其中：
   ```rust
   pub const EXEC_ASK_UNSUPPORTED_CODE: &str =
       "ask is not supported in exec mode; proceed with best-effort assumptions";
   ```

---

### 4.2 CLI 参数模型与解析器（`src/chat/agent/exec/cli.rs`）

支持顶层路由 `CliAction` 与 `ExecCli`：

```rust
use std::ffi::OsString;
use std::path::PathBuf;

/// 顶层命令行动作。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CliAction {
    /// 无参数：进入全屏 TUI（默认既有行为）。
    Tui,
    /// `lx-tui exec ...`：终端无头执行 agent。
    Exec(ExecCli),
    /// `-h` / `--help` 或 `lx-tui help [exec]`。
    Help(HelpTopic),
    /// `-V` / `--version`。
    Version,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HelpTopic {
    Root,
    Exec,
    ExecResume,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ColorMode {
    #[default]
    Auto,
    Always,
    Never,
}

/// `lx-tui exec` 命令行参数（对齐 `codex-rs/exec/src/cli.rs`，decision 1/2/4）。
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ExecCli {
    /// 子命令：`None` 表示新建一轮会话，`Some(Resume)` 表示恢复既有会话。
    pub command: Option<ExecCommand>,
    /// 位置参数提示词；为 `None` 或 `Some("-")` 时从 stdin 读取。
    pub prompt: Option<String>,
    /// `-i, --image <FILE>`：附加到首条用户消息的本地图片路径（可多次指定或逗号分隔）。
    pub images: Vec<PathBuf>,
    /// `-m, --model <MODEL>`：覆盖默认模型（支持 `<provider>/<model>[#<variant>]` 或纯 `<model_id>`）。
    pub model: Option<String>,
    /// `-C, --cd <DIR>`：执行工作目录根（默认当前进程 `cwd`）。
    pub cwd: Option<PathBuf>,
    /// `--json`：向 stdout 输出 JSONL 事件流。
    pub json: bool,
    /// `-o, --output-last-message <FILE>`：将最终 assistant 文本消息写入文件。
    pub last_message_file: Option<PathBuf>,
    /// `--ephemeral`：使用内存数据库，不落盘持久化会话文件。
    pub ephemeral: bool,
    /// `--color <auto|always|never>`：stderr 人类可读输出的 ANSI 颜色控制。
    pub color: ColorMode,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExecCommand {
    /// `lx-tui exec resume [SESSION_ID] [--last] [PROMPT]`
    Resume(ResumeArgs),
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ResumeArgs {
    /// 指定恢复的 `session_id`（如 `sess-...`）。
    pub session_id: Option<String>,
    /// `--last`：自动选择当前工作区最近更新的一条会话。
    pub last: bool,
    /// 续跑的新提示词。
    pub prompt: Option<String>,
    /// `-i, --image <FILE>`：续跑附加图片。
    pub images: Vec<PathBuf>,
}
```

#### 提示词与管道 `stdin` 合并算法（对齐 `codex-rs/exec/src/lib.rs:2419-2468`）

```rust
/// Stdin 读取策略。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StdinBehavior {
    /// 未传位置 prompt：若 stdin 是管道则读取，若是 TTY 则报错缺少提示词。
    RequiredIfPiped,
    /// 位置 prompt 为 `"-"`：强制从 stdin 读到 EOF。
    Forced,
    /// 已传位置 prompt：若 stdin 是管道则读取并作为 `<stdin>` 上下文追加；若是 TTY 则跳过。
    OptionalAppend,
}

/// 将位置 prompt 与管道 stdin 组合为最终用户消息文本。
pub fn prompt_with_stdin_context(prompt: &str, stdin_text: &str) -> String {
    if stdin_text.trim().is_empty() {
        return prompt.to_string();
    }
    let mut combined = format!("{prompt}\n\n<stdin>\n{stdin_text}");
    if !stdin_text.ends_with('\n') {
        combined.push('\n');
    }
    combined.push_str("</stdin>");
    combined
}

/// 纯函数组合校验：根据 `prompt_arg`、`stdin_is_terminal` 与读取到的 `stdin_data` 产出最终 prompt。
pub fn resolve_prompt_input(
    prompt_arg: Option<&str>,
    stdin_is_terminal: bool,
    read_stdin: impl FnOnce() -> std::io::Result<String>,
) -> Result<String, String> {
    match prompt_arg {
        Some(prompt) if prompt != "-" => {
            if prompt.trim().is_empty() {
                return Err("Prompt must not be empty.".to_string());
            }
            if !stdin_is_terminal {
                let stdin_text = read_stdin().map_err(|err| format!("failed to read stdin: {err}"))?;
                Ok(prompt_with_stdin_context(prompt, &stdin_text))
            } else {
                Ok(prompt.to_string())
            }
        }
        Some("-") => {
            let stdin_text = read_stdin().map_err(|err| format!("failed to read stdin: {err}"))?;
            if stdin_text.trim().is_empty() {
                return Err("No prompt provided on stdin.".to_string());
            }
            Ok(stdin_text)
        }
        None => {
            if stdin_is_terminal {
                return Err(
                    "No prompt provided. Either specify one as an argument or pipe the prompt into stdin."
                        .to_string(),
                );
            }
            let stdin_text = read_stdin().map_err(|err| format!("failed to read stdin: {err}"))?;
            if stdin_text.trim().is_empty() {
                return Err("No prompt provided on stdin.".to_string());
            }
            Ok(stdin_text)
        }
    }
}
```

#### 模型参数 `-m / --model` 解析规则

支持两种写法，与 `ModelsSnapshot` 无缝对接：
1. **完整路径** `<provider>/<model>[#<variant>]`（例如 `anthropic/claude-sonnet-4#high` 或 `openai/gpt-5`）：直接在 `settings.providers[provider].models[model]` 中查找；
2. **短模型 ID** `<model>[#<variant>]`（例如 `claude-sonnet-4`）：优先在 `default_model` 所属 provider 下匹配，未命中则按 `enabled_providers` 顺序查找第一个包含该 `model_id` 且已配置 API Key 的 provider。
3. 未指定 `-m` 时：使用 `ModelsSnapshot::default_selection()`；若恢复会话（`resume`）且原会话记录了 `model_provider` + `model_id`，优先沿用会话模型（若显式传了 `-m`，则写入一条 `record_model_switch` 并切换为新模型）。

---

### 4.3 输出事件协议与双模式处理器（`events.rs` & `processor.rs`）

#### 4.3.1 JSONL 事件结构（`src/chat/agent/exec/events.rs`，对齐 `codex-rs/exec/src/exec_events.rs`）

```rust
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// `--json` 模式下每行输出一条的顶层事件（`#[serde(tag = "type")]`）。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "type")]
pub enum ThreadEvent {
    #[serde(rename = "thread.started")]
    ThreadStarted { thread_id: String },
    #[serde(rename = "turn.started")]
    TurnStarted,
    #[serde(rename = "item.started")]
    ItemStarted { item: ThreadItem },
    #[serde(rename = "item.updated")]
    ItemUpdated { item: ThreadItem },
    #[serde(rename = "item.completed")]
    ItemCompleted { item: ThreadItem },
    #[serde(rename = "turn.completed")]
    TurnCompleted { usage: ExecUsage },
    #[serde(rename = "turn.failed")]
    TurnFailed { error: ThreadError },
    #[serde(rename = "error")]
    Error { message: String },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ThreadItem {
    pub id: String,
    #[serde(flatten)]
    pub details: ThreadItemDetails,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ThreadItemDetails {
    AgentMessage {
        text: String,
    },
    Reasoning {
        text: String,
    },
    CommandExecution {
        command: String,
        aggregated_output: String,
        exit_code: Option<i32>,
        status: ItemStatus,
    },
    FileChange {
        changes: Vec<FileChangeEntry>,
        status: ItemStatus,
    },
    ToolCall {
        name: String,
        arguments: Value,
        summary: Option<String>,
        status: ItemStatus,
    },
    McpToolCall {
        server: String,
        tool: String,
        arguments: Value,
        status: ItemStatus,
        error: Option<String>,
    },
    CollabToolCall {
        tool: String,
        agent_id: Option<String>,
        role: Option<String>,
        title: Option<String>,
        status: ItemStatus,
    },
    WebSearch {
        query: String,
        status: ItemStatus,
    },
    TodoList {
        items: Vec<TodoEntry>,
    },
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ItemStatus {
    InProgress,
    Completed,
    Failed,
    Cancelled,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FileChangeEntry {
    pub path: String,
    pub kind: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TodoEntry {
    pub step: String,
    pub status: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct ExecUsage {
    pub input_tokens: i64,
    pub cached_input_tokens: i64,
    pub cache_write_input_tokens: i64,
    pub output_tokens: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ThreadError {
    pub message: String,
}
```

#### 4.3.2 `EventProcessor` 抽象与人类可读格式（`src/chat/agent/exec/processor.rs`）

定义统一的 `ExecEventProcessor` trait，支持注入自定义 `Write` 句柄以便在单元测试中捕获 `stdout` 与 `stderr` 字节流：

```rust
pub trait ExecEventProcessor: Send {
    /// 打印启动头部信息（workdir / model / provider / session_id / prompt）。
    fn on_session_started(&mut self, meta: &SessionHeader<'_>);
    /// 模型思考块闭合时回调。
    fn on_reasoning(&mut self, text: &str);
    /// 模型正文块闭合（中间步骤或最终步骤）时回调。
    fn on_assistant_text(&mut self, text: &str);
    /// 工具开始执行时回调。
    fn on_tool_started(&mut self, call: &ReadyToolCall);
    /// 工具完成结算时回调。
    fn on_tool_finished(&mut self, call: &ReadyToolCall, outcome: &ToolOutcome, duration_ms: i64);
    /// 子 agent 控制工具完成时回调。
    fn on_subagent_finished(&mut self, call: &ReadyToolCall, result: & Result<SubagentOutcome, HostError>, duration_ms: i64);
    /// 告警或非致命通知（如自动 compact）。
    fn on_warning(&mut self, message: &str);
    /// 整轮结束（Complete / Interrupted / Failed）：刷出最终 stdout 与 `-o` 文件。
    fn on_turn_finished(&mut self, summary: &TurnSummary, final_message: Option<&str>);
}
```

`JsonlOutputProcessor` 维护在途工具映射，保证同一工具调用的 `item.started` 与 `item.completed` 使用同一个稳定 `item.id`：

```rust
pub struct JsonlOutputProcessor<WOut: Write + Send> {
    stdout: WOut,
    last_message_file: Option<PathBuf>,
    next_item_id: u64,
    last_emitted_text: Option<String>,
    /// 在途工具调用：`call.call_id -> item.id`，保证 started / completed 同 id。
    active_tool_items: HashMap<String, String>,
}

impl<WOut: Write + Send> JsonlOutputProcessor<WOut> {
    fn on_tool_started(&mut self, call: &ReadyToolCall) {
        let id = self.next_item_id();
        self.active_tool_items.insert(call.call_id.clone(), id.clone());
        self.emit(&ThreadEvent::ItemStarted {
            item: ThreadItem { id, details: item_details_for_start(call) },
        });
    }

    fn on_tool_finished(&mut self, call: &ReadyToolCall, outcome: &ToolOutcome, _duration_ms: i64) {
        // 同一 call_id 复用 started 分配的 id；无前置 started 时回退新分配。
        let id = self
            .active_tool_items
            .remove(&call.call_id)
            .unwrap_or_else(|| self.next_item_id());
        self.emit(&ThreadEvent::ItemCompleted {
            item: ThreadItem { id, details: item_details_for_finish(call, outcome) },
        });
    }
}
```

**人类可读模式（`HumanOutputProcessor`）在 `stderr` 的输出样式（对齐 Codex `EventProcessorWithHumanOutput` 与 `lx` 步骤标记语义）**：

```text
lx-tui exec v0.1.0
--------
workdir: /Users/yonah/projects/agent/lx-tui
model: claude-sonnet-4
provider: anthropic
session id: sess-192f8a1b2c
--------
user
查看 Cargo.toml 的版本号并检查 src/main.rs

● Thinking...
  Checking Cargo.toml and src/main.rs to verify package version and entry point.

■ read Cargo.toml
  └─ Read 57 lines (12ms)

■ exec_command `cargo check --quiet` in /Users/yonah/projects/agent/lx-tui
  └─ succeeded in 420ms

■ spawn_agent explorer · inspect cli modules
  └─ spawned sub-1 (explorer)

■ wait_agent (all)
  └─ completed: sub-1 (explorer)

lx
Cargo.toml 的版本号为 `0.1.0`。

tokens used: 2,140 (in 1,980 · cached 1,024 · out 160)
```

**`stdout` / `stderr` 分流与 `-o` 落盘规则**：
1. 在 `HumanOutputProcessor` 中，所有过程行一律写入 `stderr`（`writeln!(self.stderr, ...)`）。
2. 当整轮结束且 `final_message` 非空时：
   - 若 `stdout_is_terminal == false`（即用户使用了管道 `lx-tui exec "..." | jq` 或重定向 `> out.txt`），将 `final_message` 原样写入 `stdout`（`writeln!(self.stdout, "{msg}")`），**不带任何前缀、颜色转义码或统计尾巴**。
   - 若指定了 `-o / --output-last-message <PATH>`，通过 `platform::write_file_atomic` 将 `final_message` 写入目标文件（自动创建父目录）。
3. 在 `JsonlOutputProcessor`（`--json`）中：
   - 每一条 `ThreadEvent` 序列化为单行 JSON 写入 `stdout` 并立即 `flush()`；
   - 若指定了 `-o / --output-last-message <PATH>`，同样在结束时把最后一条 `agent_message` 写入该文件。

---

## 5. 关键数据流与无头宿主状态机（`runner.rs` & `mutation.rs` & `subagent/`）

### 5.1 整体时序图

```text
main()
  └─> cli::parse_args(env::args_os())
        ├─ CliAction::Tui  ──> 既有全屏 TUI 启动链路（零改动）
        └─ CliAction::Exec(exec_cli)
             └─> exec::run_main(exec_cli)
                   1. 解析 cwd（-C 优先，回退 current_dir）并规范化
                   2. 解析 prompt + 管道 stdin + -i 图片附件（prepare_user_message）
                   3. 加载 AiSettings 与 ModelsSnapshot，按 -m、会话模型或默认配置解析 FrozenModel
                   4. 解析 linked worktree（缺省时按 cwd 推导隔离库）并打开 Database（--ephemeral 走 open_in_memory）
                   5. 若为 resume：定位目标 session_id（--last 取 list_chat_sessions 首条），校验会话锁
                   6. 构建 Tokio runtime，await mcp_snapshot_for_turn_async（McpManager::snapshot_for）取真实 MCP 快照
                   7. assemble_exec_turn_with_snapshot：开启轮次 begin_turn_with_attachments、装配系统提示（compose_system_prompt）、
                      AGENTS.md 指令、skills_catalog_prompt 与 $skill 引用块、历史上下文，并 hydrate_turn_images 水合历史图片
                   8. 启动 ExecRunner（持有 Database、ProcessManager、ReadRegistry、SubagentPool、EventProcessor）
                   9. tokio::spawn(run_agent_turn(context, ExecHost, AgentTools, stream_one, cancel_rx))
                  10. 主循环 select! 处理：
                        - StreamDelta（累积 reasoning/text part、节流 checkpoint 落库）
                        - AgentHostRequest（BeginTool / FinishTool / AskUser / Subagent / UserShellUpdates）
                        - MutationHostEvent（文件修改前后的快照 Prepare / Commit）
                        - SubagentNetEvent（后台子 agent 流事件与完成通知）
                        - Ctrl+C 信号（置位 cancel_tx = true，等待 turn 收尾并清理进程）
                  11. TurnSummary 返回后：
                        - database.finish(workspace, assistant_id, &parts, &outcome, usage)
                        - 关闭并收割所有存活的子 agent 与 ProcessManager 后台进程
                        - processor.on_turn_finished(&summary, final_message)
                        - 返回 ExitCode（Complete -> 0, Interrupted -> 130, Failed -> 1）
```

### 5.2 `ExecHost` 与 `ExecRunner` 无头事件循环

在 TUI 模式下，`EventHost` 把请求包装成 `AppEvent` 发给 TUI 主循环并等待下一帧绘制后才回执 `BeginTool`。而在无头 `exec` 模式下，**不存在帧绘制门槛**：`BeginTool` 只需在同一事件协调器内完成 `database.begin_tool_call`（pending）→ 立即 `database.mark_tool_running`（running）→ 触发 `processor.on_tool_started(&call)` → 立即通过 `oneshot` 回执 `ToolCallTicket`！

```rust
/// loop 与无头 ExecRunner 之间的内部消息。
pub(crate) enum ExecLoopEvent {
    /// 主 agent 单步流式增量（用于实时记录 text / reasoning part 并定期 checkpoint）。
    MainDelta(Vec<ChatDelta>),
    /// AgentHost 请求（BeginTool / FinishTool / AskUser / Subagent / UserShellUpdates）。
    Host(AgentHostRequest),
    /// 文件修改前后快照记录（`mutation.rs` 的持久化桥，供 `/rewind` 撤销使用）。
    Mutation(ExecMutationEvent),
    /// 运行时自动压缩通知。
    AutoCompact {
        notice: AutoCompactNotice,
        reply: oneshot::Sender<Result<(), HostError>>,
    },
    /// 子 agent 内部事件（流增量、工具握手、轮次结束）。
    Subagent(ExecSubagentEvent),
}
```

#### 流式采样包装器 `stream_one`

`run_agent_turn` 接收的 `stream_one` 闭包在调用 `crate::chat::client::run_stream` 时，通过回调把收到的每个 `ChatDelta` 实时转发给 `ExecLoopEvent::MainDelta`：
- 当收到 `ChatDelta::Part(part)` 时，`ExecRunner` 实时更新内存中的 `parts: Vec<ChatPart>`；
- 当一个 `PartKind::Reasoning` 块结束（切换到 `PartKind::Text` 或 `ToolInputStarted` 或 `ResponseFinished`）时，立即调用 `processor.on_reasoning(&reasoning_text)` 实时打印到终端；
- 当 `parts` 累计新增字节达到 `8192` 字节或距上次落库超过 `500ms` 时，调用 `database.checkpoint(&workspace, &assistant_id, &parts)` 落库。

#### `BeginTool` 与 `FinishTool` 无头握手代码

```rust
fn handle_begin_tool(
    &mut self,
    call: ReadyToolCall,
    reply: oneshot::Sender<Result<ToolCallTicket, HostError>>,
) {
    let result = self
        .db
        .begin_tool_call(
            &self.workspace,
            &self.session_id,
            &self.assistant_id,
            &call.call_id,
            &call.name,
            &call.arguments_json,
        )
        .and_then(|ticket| {
            self.db.mark_tool_running(&self.workspace, &ticket)?;
            Ok(ticket)
        });
    match result {
        Ok(ticket) => {
            self.processor.on_tool_started(&call);
            self.active_calls.insert(call.call_id.clone(), call);
            let _ = reply.send(Ok(ticket));
        }
        Err(error) => {
            tracing::warn!(target: "lx::exec", %error, "begin_tool_call failed");
            let _ = reply.send(Err(HostError::Failed {
                code: crate::event::chat::host::BEGIN_TOOL_FAILED_CODE.to_string(),
            }));
        }
    }
}

fn handle_finish_tool(
    &mut self,
    ticket: ToolCallTicket,
    outcome: ToolOutcome,
    reply: oneshot::Sender<Result<(), HostError>>,
) {
    let db_result = crate::event::chat::tool_calls::to_db_result(&outcome);
    let duration_ms = crate::db::now_unix_ms().saturating_sub(ticket.started_at_ms).max(0);
    match self.db.finish_tool_call(&self.workspace, &ticket, &db_result) {
        Ok(()) => {
            if let Some(call) = self.active_calls.remove(&ticket.call_id) {
                self.processor.on_tool_finished(&call, &outcome, duration_ms);
            }
            let _ = reply.send(Ok(()));
        }
        Err(error) => {
            tracing::warn!(target: "lx::exec", %error, "finish_tool_call failed");
            let _ = reply.send(Err(HostError::Failed {
                code: crate::event::chat::host::FINISH_TOOL_FAILED_CODE.to_string(),
            }));
        }
    }
}
```

### 5.3 无头子 Agent 调度（`src/chat/agent/exec/subagent/`）

在 `exec` 模式下，主 agent 调用 `spawn_agent`、`wait_agent`、`send_input`、`resume_agent`、`close_agent`、`list_agents`、`cancel_agent_input` 时，由 `ExecRunner` 内的 `ExecSubagentPool` 处理（池状态与控制入口在 `subagent/mod.rs`，子轮上下文装配与图片归档在 `subagent/turn.rs`，等待续等与终态结算在 `subagent/settle.rs`）：
1. **并发控制**：严格执行 `MAX_CONCURRENT_SUBAGENTS = 3`（运行中子 agent 达 3 个时返回 `LIMIT_REACHED_CODE`）。
2. **独立隔离**：每个子 agent 拥有：
   - 独立的 SQLite 子会话（`database.begin_subagent_turn_with_items`，`source = 'subagent'`）；
   - 独立的 `ProcessManager` 与 `ReadRegistry`（通过 `crate::chat::tools::read_registry(&child_session_id)`）；
   - 按角色裁剪的 `ToolSurface::from_role(role)`（`Explorer` 仅 3 个只读工具，`Executor` 7 个工具，均无控制工具，杜绝递归）。
3. **等待与结算（`wait_agent`）**：
   - 挂起等待通道 `PendingWait { targets, mode, auto_continue, timeout_ms, deadline, reply }`；
   - 当任一目标子 agent 结束（或全部结束、或 `tokio::time::sleep_until(deadline)` 到期）时唤醒，读取子会话最终报告 `ReportPayload`，标记 `mark_report_delivered` 并构造 `SubagentOutcome::Waited` 返回给主 agent；
   - `wait_agent(all)` 且 `mode == WaitMode::All` 时启用 `auto_continue`：单次超时到期但仍有目标在途时自动重新派生等待任务，不向模型返回 `timed_out: true`；
   - `send_input(interrupt = true)` 打断运行中子轮时记录 `interrupted` 标记，`finish` 据此立即调度队首排队输入；`close_agent` 时清理该标记，避免被取消的子轮继续消费队列。
4. **主轮结束清理**：
   - 当主 agent 轮次结束或收到 `Ctrl+C` 时，对所有仍在运行的子 agent 发送 `cancel.send(true)`，等待其异步任务收尾并关闭其专属 `ProcessManager`。

### 5.4 Review 修复后的装配与结算约束（P0-2 / P0-3 / P1-1 / P1-2 / P1-5）

**主轮自动压缩必须落库检查点**：`ExecLoopEvent::AutoCompact` 分支不再只打印告警，而是由 `commit_main_compact` 读取会话最大 `sequence`、`db.active_checkpoint` 与 `db.session_revision`，构造 `CompactCommit` 后调用 `db.commit_compact(&workspace, &commit)`；落库成功才 `processor.on_warning(...)`，失败向 `reply` 回传 `Err(HostError::Failed)`，让 loop 感知检查点未持久化。`ExecHost` 上不保留 `run_compact_summary` 等无调用方的方法与字段。

**最终回复只认最后一个工具之后的文本**：`ExecRunner` 维护 `last_tool_position: Option<i64>`（工具启动时更新为当前 `position`）。`final_message()` 仅在 `TurnOutcome::Complete` 且 `part.position > last_tool_position` 时提取最后一个非空 `PartKind::Text`；工具前的说明文字已在工具启动时刷到 `stderr`，不再重复写入 `stdout` / `-o` 文件。

**冷启动装配顺序**：`run_main` 先构建 Tokio 多线程 runtime，再在运行时内 `block_on(mcp_snapshot_for_turn_async(...))`（内部调用 `McpManager::snapshot_for(&workspace, &servers, cancel)` 等待 MCP 握手完成）取得真实快照，最后才调用 `assemble_exec_turn_with_snapshot` 开轮装配；不得在 runtime 创建前或依赖后台 `prewarm` 拿快照。

**路径与上下文基准**：`-i / --image` 相对路径一律按解析后的 `-C` 目录以 `cwd.join(path)` 加载；`linked_worktree` 缺省时在开发环境按 `cwd` 调用 `git::linked_worktree_root` 推导隔离库；`assemble_exec_turn_with_snapshot` 调用 `hydrate_turn_images` 水合历史轮次图片（模型不支持图片时降级为 `ERROR: Cannot read "..."` 文本），并在装配时注入 `skills_catalog_prompt(workspace, false)` 与用户 `$skill` 点名正文块。

文件变更持久化桥独立在 `mutation.rs`：worker 在路径锁内同步等待 `ExecMutationEvent::Prepare` / `Commit` 回执，事件循环消失时按取消收尾，避免 worker 永久阻塞。

---

## 6. 会话恢复（`resume`）与 `--ephemeral` 详细设计

### 6.1 `lx-tui exec resume` 寻址逻辑

```rust
pub fn resolve_resume_session(
    db: &Database,
    workspace: &Path,
    args: &ResumeArgs,
) -> Result<String, String> {
    if let Some(id) = args.session_id.as_deref() {
        let messages = db
            .load_chat_session(workspace, id)
            .map_err(|err| format!("database error: {err}"))?;
        if messages.is_none() {
            return Err(format!("session '{id}' not found in workspace {}", workspace.display()));
        }
        return Ok(id.to_string());
    }
    if args.last {
        let sessions = db
            .list_chat_sessions(workspace, 0, 1)
            .map_err(|err| format!("database error: {err}"))?;
        let Some(latest) = sessions.into_iter().next() else {
            return Err(format!("no existing sessions found in workspace {}", workspace.display()));
        };
        return Ok(latest.external_id);
    }
    Err("Specify a session ID or pass `--last` to resume the most recent session.".to_string())
}
```

- **跨进程会话锁保护**：恢复已有会话或启动新会话时，调用 `crate::platform::try_lock_session(&session_id)`。若该会话正在某个打开的 TUI 窗格中流式运行（锁被占用），`exec resume` 立即报错退出：`"session '<id>' is currently active in another lx process"`，防止两个进程同时写同一个会话。
- **遗留状态清理**：若恢复的会话上次因异常崩溃残留了 `streaming` 消息或 `running` 工具，在开启新 turn 前先调用 `db.recover_session(workspace, &session_id)` 将旧未完成项结算为 `interrupted` / `cancelled`。

### 6.2 `--ephemeral` 无痕模式

- 在 `src/db/mod.rs` 新增 `Database::open_in_memory() -> Result<Self>`（底层调用 `Connection::open_in_memory()` 并执行 `migrate()`）。
- 当 `cli.ephemeral == true` 时：
  - 使用 `Database::open_in_memory()` 构造内存数据库；
  - `run_agent_turn`、工具调用门票校验、文件快照、子 agent 子会话等全套 SQL 逻辑**零分支复用**，无需写两套有无 DB 的代码；
  - 进程退出后内存库自然释放，磁盘上的 `~/.lx-t[-dev]/db/lx.db` 不产生任何记录。
- 若同时传入 `resume` 与 `--ephemeral`，CLI 解析器直接报错拒绝：`"--ephemeral cannot be combined with 'resume'"`.

---

## 7. `Ctrl+C` 信号中断与退出码规范（Decision 5）

### 7.1 退出码常量

```rust
/// 正常完成。
pub const EXIT_OK: i32 = 0;
/// 参数错误、配置缺失、网络/协议错误或轮次失败。
pub const EXIT_ERR: i32 = 1;
/// 用户按下 Ctrl+C（SIGINT）中断。
pub const EXIT_INTERRUPTED: i32 = 130;
```

### 7.2 `Ctrl+C` 优雅停机序列

```rust
let (cancel_tx, cancel_rx) = watch::channel(false);
let cancel_for_signal = cancel_tx.clone();
tokio::spawn(async move {
    if tokio::signal::ctrl_c().await.is_ok() {
        let _ = cancel_for_signal.send(true);
    }
});
```

当用户按下 `Ctrl+C` 时：
1. `cancel_tx.send(true)` 广播取消信号；
2. 若当前正阻塞在 `wait_agent`，`ExecSubagentPool` 立即唤醒该等待并返回 `Err(HostError::Cancelled)`；
3. 若当前正在执行 MCP 工具，`execute_mcp_cancellable` 观察到 `cancel_rx` 变化，立即发送 MCP `notifications/cancelled`；
4. 若当前正在执行阻塞文件 IO（`edit` / `write`），`run_agent_turn` 等待该单次原子写完成后记录真实结果，随后把同一批次未开始的工具标为 `ToolOutcome::Cancelled` 并返回 `TurnSummary::interrupted(usage)`；
5. `ExecRunner` 收到 `TurnSummary::interrupted` 后：
   - 向所有运行中的子 agent 发送取消并等待收尾；
   - 调用 `process_manager.shutdown_all().await` 终止所有由 `exec_command` 启动的后台子进程；
   - 调用 `db.finish(&workspace, &assistant_id, &parts, &TurnOutcome::Interrupted, usage)` 将本轮持久化为 `interrupted`；
   - `processor.on_turn_finished` 在 `stderr` 打印 `turn interrupted`（JSONL 模式输出 `{"type":"turn.failed","error":{"message":"turn interrupted"}}`）；
   - `main.rs` 以退出码 `130` 退出。

---

## 8. 入口改造（`src/main.rs`）

保持 `src/main.rs` 薄入口职责（< 100 行）：

```rust
fn main() {
    match run() {
        Ok(code) if code == 0 => {}
        Ok(code) => std::process::exit(code),
        Err(error) => {
            tracing::error!(error = %error, "lx-tui exited with error");
            eprintln!("lx-tui: {error}");
            std::process::exit(1);
        }
    }
}

fn run() -> Result<i32, Box<dyn std::error::Error>> {
    init_tracing()?;
    let action = match lx_tui::chat::agent::exec::parse_cli(std::env::args_os()) {
        Ok(action) => action,
        Err(usage_error) => {
            eprintln!("lx-tui: {usage_error}");
            return Ok(lx_tui::chat::agent::exec::EXIT_ERR);
        }
    };
    match action {
        CliAction::Help(topic) => {
            print!("{}", lx_tui::chat::agent::exec::help_text(topic));
            Ok(lx_tui::chat::agent::exec::EXIT_OK)
        }
        CliAction::Version => {
            println!("lx-tui {}", env!("CARGO_PKG_VERSION"));
            Ok(lx_tui::chat::agent::exec::EXIT_OK)
        }
        CliAction::Exec(exec_cli) => {
            let linked = database_worktree();
            lx_tui::chat::agent::exec::run_main(exec_cli, linked.as_deref())
        }
        CliAction::Tui => {
            run_tui()?;
            Ok(lx_tui::chat::agent::exec::EXIT_OK)
        }
    }
}
```

---

## 9. 不变量与边界约束

1. **TUI 路径零回归**：无参数运行 `lx-tui` 时，完全走原有 `run_tui()` 路径，终端初始化/恢复顺序（关闭鼠标上报 → drain → 离开备用屏 → `disable_raw_mode`）一行不改。
2. **模块边界清晰**：`src/chat/agent/exec/` 绝不导入 `app/`、`ui/`、`tui/`、`event/mod.rs`（若需共用 `BeginTool` 错误码常量或 `MutationRecorder` 契约，将纯类型下沉或从 `chat` 层引用）。
3. **无 `ask` 挂起保证**：`ToolSurface::ExecMain` 不包含 `"ask"`，且 `ExecHost::ask_user` 恒返回 `Err(HostError::Failed)`，双保险确保任何模型都无法在无头模式下挂起等待终端输入。
4. **`stdout` 纯净度不变量**：
   - 在默认人类可读模式下，所有进度、思考、工具日志、警告与统计**只允许写 `stderr`**；`stdout` 仅用于输出最终回复正文（当 `stdout` 非 TTY 时）或 `--help` / `--version`。
   - 在 `--json` 模式下，`stdout` 每一行必须是合法的单行 `ThreadEvent` JSON 对象。
5. **生产代码质量红线**：
   - 生产代码严禁出现 `unwrap()` / `expect()`；
   - 生产代码单文件行数严格 ≤ 1000 行（按职责拆分子模块）；测试文件跟随被测子模块拆分，单文件 < 1000 行；
   - 所有模型可见文本与终端用户可见文案（CLI help、报错提示、进度标签）一律使用英文，中文仅用于代码注释与 `tracing` 说明。

---

## 10. 测试矩阵

1. **CLI 解析与 Stdin 合成测试（`src/chat/agent/exec/tests/cli_tests.rs`）**：
   - 无参数 → `CliAction::Tui`
   - `-h` / `--help` / `exec --help` / `exec resume --help` / `-V` → 对应 `Help` / `Version`
   - `exec "fix bug"`、`-C dir`、`-m provider/model#variant`、`--json`、`-o out.txt`、`--ephemeral`、`-i a.png -i b.png`、`--color never` 组合解析
   - `exec resume sess-123 "continue"`、`exec resume --last "continue"` 解析
   - 非法 flag、`--ephemeral` 与 `resume` 互斥、空 prompt、TTY 下未传 prompt 的报错信息断言
   - `resolve_prompt_input`：仅位置参数、仅管道 stdin、`"-"` 强制 stdin、位置参数 + 管道 stdin 自动拼接 `<stdin>...</stdin>`
2. **输出处理器与事件映射测试（`src/chat/agent/exec/tests/processor_tests.rs` & `events_tests.rs`）**：
   - `HumanOutputProcessor`：捕获内存 `stderr` 与 `stdout` 缓冲区，验证 `stdout_is_terminal = false` 时过程全在 `stderr`、`stdout` 仅有最终回复正文；验证 `--color never` 下不含 `\x1b[` ANSI 转义序列
   - `-o / --output-last-message`：验证最终回复原子写入目标文件
   - `JsonlOutputProcessor`：验证输出的每一行均可反序列化为 `ThreadEvent`，顺序符合 `thread.started` → `turn.started` → `item.*` → `turn.completed`，且同一 `call_id` 的 `item.started` / `item.completed` 共享同一 `item.id`
3. **工具面与 Schema 测试（`src/chat/agent/tests.rs` & `src/chat/transport/tools/tests.rs`）**：
   - `ToolSurface::ExecMain.names()` 包含 16 项，确切不含 `"ask"`，包含 `"spawn_agent"` 等全部 16 项
   - 四种协议（OpenAI / Responses / Anthropic / Google）在 `ExecMain` 下生成的工具声明不含 `"ask"`
4. **无头运行器端到端单元测试（`src/chat/agent/exec/tests/runner_tests.rs`）**：
   - 使用脚本化假 `stream_one`（无需真实网络）：
     - 单步纯文本完成 → DB 落库 `complete`、退出码 `0`、输出最终文本
     - 多步工具调用（`read` + `edit` + `exec_command`）→ 文件真实修改、DB 记录 `agent_tool_call`、`stderr` 包含工具摘要
     - 模型幻觉调用 `ask` → 被拦截并返回工具失败，模型在下一步正常完成，不挂起
     - 子 agent 派发与等待（`spawn_agent` + `wait_agent`）→ 子 agent 完成后报告返回主 agent 并完成整轮
     - `--ephemeral` 模式 → 磁盘 DB 无新增会话，内存正常跑完
     - `exec resume --last` → 在第一轮会话基础上续跑第二轮，`turn_index` 递增且历史上下文可回放
     - `Ctrl+C` 取消信号触发 → 在途轮次立即中断，DB 状态为 `interrupted`，返回退出码 `130`
5. **子 Agent 池测试（`src/chat/agent/exec/tests/subagent_tests.rs`）**：
   - `wait_agent(all)` 超时且仍有在途子轮 → `auto_continue` 续等，不返回 `timed_out`
   - `send_input(interrupt = true)` 打断活跃子轮后，排队输入在 `finish` 中立即调度；`close_agent` 后不再消费队列
   - 并发上限、`resume_agent`、`cancel_agent_input` 与报告领取路径

---

## 11. 风险与回滚

| 风险 | 影响 | 缓解措施 |
|------|------|----------|
| `event/chat/` 与 `chat/agent/exec/` 存在部分逻辑重叠（如工具落库转换、文件快照记录） | 重复代码或行为漂移 | 将纯数据转换函数（如 `ToolOutcome -> ToolCallResult`、快照 `MutationRecorder` 逻辑）提取为 `chat` / `db` 可调用的公共辅助函数，两侧同源调用 |
| `exec_command` 启动的后台进程在 `exec` 退出后残留 | 孤儿进程占用端口或 CPU | 在 `ExecRunner` 的 `Drop` 与收尾路径中显式调用 `ProcessManager::shutdown_all()`，确保无论正常结束、报错还是 `Ctrl+C` 均终止全部子进程 |
| 同时在 TUI 和 `exec resume` 操作同一会话 | SQLite 并发写导致 sequence 或状态错乱 | 复用 `platform::try_lock_session(&session_id)` 会话级文件锁，锁冲突时立即拒绝并给出清晰英文提示 |

**回滚方案**：
- 本功能**不涉及任何 SQLite Schema 迁移**（`SCHEMA_VERSION` 保持 `22`）；
- 如需回滚，仅需移除 `src/chat/agent/exec/` 目录并还原 `src/main.rs` 与 `src/chat/agent/mod.rs` 的少量入口分发代码，对既有数据库与 TUI 功能零破坏。

---

## 12. 实施顺序

```text
Phase 1: ToolSurface::ExecMain + RequestPurpose::ExecTurn + Database::open_in_memory
   │
   ▼
Phase 2: CLI 解析器（src/chat/agent/exec/cli.rs）+ Stdin 合成 + 单元测试
   │
   ▼
Phase 3: 输出事件与双模式处理器（events.rs + processor.rs：Human / JSONL / -o 文件落盘）
   │
   ▼
Phase 4: 无头宿主与子 Agent 调度（runner.rs + mutation.rs + subagent/{mod,turn,settle}.rs：ExecHost + ExecRunner + ExecSubagentPool）
   │
   ▼
Phase 5: 顶层编排与入口接线（exec/mod.rs + src/main.rs）+ 端到端回归测试
```
