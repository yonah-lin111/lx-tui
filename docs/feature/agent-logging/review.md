# `lx` Agent 日志 — 代码评审与修复报告 (`review.md`)

- **特性目录**：`docs/feature/agent-logging/`
- **Review 基线**：`feat/agent-logging` @ `e2c41ad8`（worktree `.worktrees/agent-logging`）
- **关联文档**：
  - [`design.md`](design.md)
  - [`task.md`](task.md)
  - [`verification.md`](verification.md)
  - 项目规范：`docs/standards/{project-directory-structure,code-writing-standards,tui-design-requirements,html-assets,web-deepseek}.md`
- **Review 范围**：
  - 日志核心、按天轮转、7 天清理、`LX_TUI_LOG*` 环境变量覆盖、权限收紧（0700/0600）与 Panic Hook
  - URL / 请求头 / 错误正文 / 多供应商内联图片（Anthropic、OpenAI、Google Gemini）脱敏与 Payload JSONL 捕获
  - 主对话、TUI 子 Agent、Headless `lx-tui exec`（含子 Agent）的网络请求、轮次与工具生命周期埋点及关联字段一致性
  - `Config`、Web 设置服务（`/api/state`、`/api/config`、`/api/logs/open`）、前端 `assets/settings.{html,css,js}`、CLI `lx-tui settings` 与文档一致性
- **Review 方式**：`codegraph` + `codebase-memory-mcp` 检索，并委派 3 个并行 `general` 子 Agent 分别审计「日志核心与脱敏/Payload」「Agent 请求/轮次/工具/子 Agent 埋点」「配置/设置服务/前端/CLI 与文档」。
- **结论**：共发现 **4 个 P0、4 个 P1、5 个 P2、3 个 P3**，已全部按优先级完成修复并通过回归测试与 CLI 实机验证。

---

## 0. 问题优先级汇总表（P0 -> P3）

| 编号 | 优先级 | 涉及文件与行号 | 关键点 | 状态 |
| :--- | :---: | :--- | :--- | :---: |
| **P0-1** | **P0** | `src/logging/redact.rs:125-151`<br>`src/chat/transport/google/mod.rs:167-178, 231-256` | Google / Gemini `inlineData`（及 `inline_data`）内联图片 base64 未被脱敏，原始图片字节直接落入 `payload-*.jsonl` | 已修复 |
| **P0-2** | **P0** | `src/chat/client/mod.rs:453, 527, 728-737` | `request_error_code` 直接 `Display` 格式化 `reqwest::Error`，会将 Google URL query 中的 `?key=<API_KEY>` 泄漏到诊断日志、`payload-*.jsonl`、DB 与 UI | 已修复 |
| **P0-3** | **P0** | `src/chat/agent/exec/runner.rs:45-103, 491-511`<br>`src/chat/agent/exec/subagent/turn.rs:136-162` | Headless `lx-tui exec` 派生的子 Agent 复用了绑定父会话的 `RealStreamFactory`，导致子 Agent 请求与 Payload 被记为父会话且丢失 `agent_id` / `parent_session` | 已修复 |
| **P0-4** | **P0** | `src/event/chat/subagent/tools.rs:9-152`<br>`src/chat/agent/exec/subagent/turn.rs:252-311` | TUI 与 Headless `exec` 的子 Agent 工具调用完全绕过 `tool_calls.rs`，缺失 `lx::agent::tool` 的 `tool call started` / `settled` 日志 | 已修复 |
| **P1-1** | **P1** | `src/logging/mod.rs:86, 98-101, 173-181` | `LX_TUI_LOG=""`（或纯空白 / `","`）被判定为环境变量已锁定且解析为空指令集，导致全部日志静默关闭并永久锁死设置页热更新 | 已修复 |
| **P1-2** | **P1** | `src/logging/mod.rs:61-65`<br>`src/logging/payload.rs:211-221`<br>`src/settings/service/logs.rs:22-33` | 当天诊断日志与 `payload-*.jsonl` 文件未收紧为 `0600` 权限，且 Payload 追加分两次 `write_all` 存在多进程 JSONL 行交错风险 | 已修复 |
| **P1-3** | **P1** | `src/chat/client/mod.rs:157-201, 269-281`<br>`src/event/chat/tool_calls.rs:271-282`<br>`src/event/chat/subagent/{launch,settle}.rs` | `lx::agent::request` 对 `Option` 字段用 `?` 输出 `Some("sess-...")` 破坏按 `session=` 跨模块检索；默认 `info` 级别下工具结算与子 Agent 轮次起止不可见 | 已修复 |
| **P1-4** | **P1** | `src/config/mod.rs:97-104, 190-194`<br>`src/settings/service/handlers.rs:298-305, 363-366, 457-459` | `Config::from_json` 不接受 `"log_payload": null`；`PUT /api/config` 从 `Config::default()` 重建而非以现有配置为基底合并，部分字段更新会重置其它配置 | 已修复 |
| **P2-1** | **P2** | `src/logging/redact.rs:76-124`<br>`src/logging/payload.rs:68-70` | `omit_inline_images` 对序列化后的请求体 JSON 调用 `sanitize_text`，将代码缩进与多空格压成单空格，且内联图片脱敏后被误标 `truncated: true` | 已修复 |
| **P2-2** | **P2** | `src/chat/client/mod.rs:204-268` | 流式请求重试前未清理上一次失败尝试推入的 `ChatDelta::Finished`，且预先取消的 `cancel` 接收端未在循环入口短路 | 已修复 |
| **P2-3** | **P2** | `src/tui/mod.rs:42-43, 62-63, 157-176` | `install_panic_hook` 在信任页 `init_inline` 与主界面 `init` 中重复包装，导致 Panic 时打印两条重复日志并执行两次终端恢复 | 已修复 |
| **P2-4** | **P2** | `assets/settings.js:405-425`<br>`assets/settings.css:240-242`<br>`src/settings/service/logs.rs:6-19` | 设置页显示当天日志文件路径但 `Copy path` 复制的是目录路径；长路径缺少断行样式；`/api/state` 未暴露环境变量覆盖状态 | 已修复 |
| **P2-5** | **P2** | `assets/settings.js`（1007 行）<br>`src/settings/service/tests.rs`（1524 行） | `assets/settings.js` 超过前端单文件 1000 行上限；`src/settings/service/tests.rs` 超过单测试文件 1500 行上限 | 已修复 |
| **P3-1** | **P3** | `src/event/chat/mod.rs:213-227`<br>`src/event/chat/tool_calls.rs:41-46, 152-184, 288-293` | 取消流请求无日志记录；工具落库失败 `warn!` 的 `target` 混用 `lx::agent` 且缺少 `pane` / `session` 关联字段 | 已修复 |
| **P3-2** | **P3** | `src/logging/payload.rs:172-187` | `PayloadCapture` 输出的 JSONL 记录缺少 `pid` 字段，多进程并发写盘时不便与诊断日志按进程关联 | 已修复 |
| **P3-3** | **P3** | `docs/feature/agent-logging/{design,task,verification}.md` | 设计文档、任务清单与验证报告部分细节与最终实现存在漂移，缺少 `/api/logs/open` 与设置页日志控件测试 | 已修复 |

---

## 1. P0（数据安全泄漏与核心身份错位）

### [P0-1] `src/logging/redact.rs:125-151` — Google / Gemini `inlineData`（及 `inline_data`）内联图片 base64 未被脱敏

- **关键点**：
  - `src/chat/transport/google/mod.rs:167-178, 231-256` 在组装用户图片与 `tool_result` 图片时使用 Gemini 原生 JSON 结构 `{"inlineData": {"mimeType": "image/png", "data": "<base64>"}}`。
  - 原 `omit_images_in_value` 只匹配两类形状：① `data:image/...;base64,...` 字符串；② 同一 JSON 对象内 `type == "base64" | "image"` 且含 `data` 字符串。
  - Gemini 的 `inlineData`（以及蛇形 `inline_data`）内部只有 `mimeType` / `mime_type` 与 `data`，没有 `type` 字段，导致两条规则均不命中，百万字节级图片 base64 原样落入 `payload-*.jsonl`。
- **错误原因**：按 Anthropic 与 OpenAI 的局部 JSON 形状硬编码匹配，未覆盖 Google Gemini 的 `mimeType + data` 结构。
- **修复**：
  - 扩展 `omit_images_in_value`：当 JSON 对象含字符串字段 `data`，且满足 `type` 为 `"base64" | "image" | "input_image"` **或**同对象存在字符串字段 `mimeType` / `mime_type` / `media_type` 时，统一将 `data` 替换为 `[image omitted: N bytes]`。
  - 在 `src/logging/redact/tests.rs::google_inline_data_images_are_replaced` 补充 `inlineData` 与 `inline_data` 脱敏单测。

### [P0-2] `src/chat/client/mod.rs:453, 527, 728-737` — `request_error_code` 直接 `Display` 格式化 `reqwest::Error`，泄漏 Google URL query 中的 `?key=<API_KEY>`

- **关键点**：
  - Google transport 将 API Key 放在 URL query（`?alt=sse&key=<API_KEY>`，`src/chat/transport/google/mod.rs:59-64`）。
  - `request_error_code` 注释写明“不含 URL 与 Key”，但第 735 行兜底分支写了 `format!("request failed: {error}")`。`reqwest::Error` 的 `Display` 实现会附带 `for url (https://...?alt=sse&key=...)`；一旦发生非 `is_timeout()` / `is_connect()` 的请求或流读取错误，API Key 会随 `StopReason::Failed { code }` 写入诊断日志、`payload-*.jsonl` 的 `error` 字段、数据库与 UI。
- **错误原因**：直接格式化 `reqwest::Error` 而未调用 `error.without_url()` 剥离 URL。
- **修复**：
  - `request_error_code` 按值接收 `reqwest::Error` 并调用 `error.without_url()`：`format!("request failed: {}", error.without_url())`。
  - 在 `src/chat/client/tests/retry.rs::request_error_code_strips_url_and_query_key` 增加回归测试，验证含 `?alt=sse&key=secret-g-key` 的请求失败错误码绝不包含 key 或 URL 路径。

### [P0-3] `src/chat/agent/exec/runner.rs:45-103, 491-511`、`src/chat/agent/exec/subagent/turn.rs:136-162` — Headless `lx-tui exec` 子 Agent 请求被记为父会话且丢失 `agent_id` / `parent_session`

- **关键点**：
  - `ExecStreamFactory::stream` 原签名不接收 `StreamLog`；`RealStreamFactory` 在构造时冻结了主会话 `session_id` 并写死 `kind: "exec"`。
  - `ExecSubagentPool` 复用了同一个 `Arc<dyn ExecStreamFactory>`，导致 Headless 子 Agent 发起的所有模型请求与 Payload 记录全部被标成 `kind="exec"`、`session=<父会话 ID>`，`agent_id` 与 `parent_session` 均为 `None`。
- **错误原因**：把单次流式请求的日志身份绑在了进程级共享的 `RealStreamFactory` 实例上，未通过 `ExecStreamFactory` 按调用传入。
- **修复**：
  - 在 `ExecStreamFactory` 新增 `stream_with_log` 方法（提供默认转发实现以保持测试 `ScriptedFactory` 零改动），`RealStreamFactory` 恢复为无状态单元结构体。
  - `exec/runner.rs` 主轮传入 `StreamLog { kind: "exec", session_id: Some(session_id), .. }`；`exec/subagent/turn.rs` 子轮传入 `StreamLog { kind: "subagent", session_id: Some(child_session_id), agent_id: Some(agent_id), parent_session: Some(parent_session_id), .. }`。

### [P0-4] `src/event/chat/subagent/tools.rs:9-152`、`src/chat/agent/exec/subagent/turn.rs:252-311` — TUI 与 Headless `exec` 子 Agent 工具调用缺失 `lx::agent::tool` 日志

- **关键点**：
  - TUI 子 Agent 工具握手在 `tool_calls.rs:22-25, 197-207` 分流至 `subagent/tools.rs::{begin_child_tool, finish_child_tool}`；Headless 子 Agent 工具握手在 `exec/subagent/turn.rs::handle_host` 处理。
  - 这两条路径均未输出 `target: "lx::agent::tool"` 的 `tool call started` 与 `tool call settled` 日志，导致子 Agent 的全部工具调用在诊断日志中不可见。
- **错误原因**：工具落库在主轮、TUI 子轮、Headless 子轮各自独立实现，初版仅在主轮埋点。
- **修复**：
  - 在 `src/event/chat/subagent/tools.rs` 与 `src/chat/agent/exec/subagent/turn.rs` 补齐 `tool call started`（`DEBUG`）与 `tool call settled`（`INFO`）及落库失败 `WARN` 日志，携带 `session`、`agent_id`、`call_id`、`name`、`duration_ms`、`is_error`、`result_bytes`。

---

## 2. P1（核心可观测性、权限安全与热更新锁死）

### [P1-1] `src/logging/mod.rs:86, 98-101, 173-181` — `LX_TUI_LOG=""` 静默关闭全部日志并永久锁死热更新

- **关键点**：
  - 原 `initial_filter` 用 `std::env::var_os("LX_TUI_LOG").is_some()` 判定环境变量是否存在；当 `LX_TUI_LOG=""` 或纯空白时，`EnvFilter::try_from_env` 返回无任何 directive 的空过滤器（所有级别全关），且 `ENV_FILTER_LOCKED` 被设为 `true`，设置页热更新永久失效。
- **错误原因**：未过滤空字符串/纯空白值，也未检查解析出的 `EnvFilter` 是否为空指令集（如 `LX_TUI_LOG=","`）。
- **修复**：
  - 抽出纯函数 `resolve_filter_value(env_value: Option<&str>, config_level: &str) -> (EnvFilter, bool, bool)`：空串或纯空白按未设置处理（回退 `config_level` 且 `locked = false`）；非空但解析失败或指令集为空时回退 `config_level`、记 `invalid = true` 且不锁死热更新。
  - 新增 `src/logging/tests.rs::empty_or_blank_env_filter_does_not_lock_or_silence_logs` 单元测试。

### [P1-2] `src/logging/mod.rs:61-65`、`src/logging/payload.rs:211-221`、`src/settings/service/logs.rs:22-33` — 日志与 Payload 文件未收紧 `0600` 权限，且 Payload 两次 `write_all` 存在多进程交错风险

- **关键点**：
  - `payload-*.jsonl` 与 `lx-tui.log.YYYY-MM-DD` 创建后未调用 `platform::restrict_file_permissions` 收紧为 `0600`。
  - `payload::append_line` 分两次调用 `file.write_all(line.as_bytes())` 与 `file.write_all(b"\n")`，多进程并发追加时换行符前可能被另一进程插入。
  - `/api/logs/open` 在目录尚未创建或被外部删除时直接调用 `reveal_in_file_manager`，macOS `open -R` 返回退出码 1 导致接口报错 500。
- **错误原因**：只在 `logging::init` 对目录做了一次 `0700` 收紧，遗漏了文件级 `0600` 与单缓冲原子写。
- **修复**：
  - `logging::init` 创建 appender 后对当天日志文件调用 `restrict_file_permissions`（`0600`）。
  - `payload::append_line` 确保父目录 `0700`、文件 `0600`，并将 JSON 行与 `\n` 拼入同一 `Vec<u8>` 单次 `write_all` 写入。
  - `logs::open_logs` 打开前先确保目录存在且收紧为 `0700`。
  - 在 `src/logging/payload/tests.rs::enabled_capture_writes_redacted_jsonl` 增加 Unix `0600` 权限断言。

### [P1-3] `src/chat/client/mod.rs:157-201, 269-281`、`src/event/chat/tool_calls.rs:271-282` — `Option` 字段 `?` 格式化破坏 `session=` 检索，且默认 `info` 级别下工具与子 Agent 终态不可见

- **关键点**：
  - `lx::agent` 与 `lx::agent::tool` 输出 `session=sess-...`，而 `lx::agent::request` 使用 `session = ?log.session_id` 输出 `session=Some("sess-...")`，导致无法用 `session=sess-...` 统一检索同一会话的轮次、请求与工具日志；且 `agent request finished` 漏掉了 `generation`、`agent_id`、`parent_session`。
  - `tool call settled` 原为 `DEBUG` 级别，默认 `info` 级别下没有任何工具调用记录；子 Agent 的轮次启动与结算在默认级别下同样无日志。
- **错误原因**：`StreamLog` 可选字符串字段直接用了 Debug `?`；工具结算与子 Agent 轮次终态级别过低或漏埋点。
- **修复**：
  - `StreamLog` 增加 `session_str()` / `agent_str()` / `parent_str()`（无值时为 `"-"`），在 `lx::agent::request` 中统一使用 `%` Display 输出。
  - 将主轮与子轮的 `tool call settled` 提升为 `INFO`（`tool call started` 保持 `DEBUG`），并在 `event/chat/subagent/{launch,settle}.rs` 与 `chat/agent/exec/subagent/{turn,settle}.rs` 补齐 `INFO` / `WARN` 级 `subagent turn started` / `finished` / `failed` 日志。

### [P1-4] `src/config/mod.rs:97-104, 190-194`、`src/settings/service/handlers.rs:298-305, 363-366, 457-459` — `Config::from_json` 不容忍 `log_payload: null` 且 `PUT /api/config` 未以现有配置为基底合并

- **关键点**：
  - `Config::from_json` 对 `"log_level": null` 放行，但对 `"log_payload": null` 报错 `Invalid`，与 `handlers::config_from_json` 行为分叉。
  - `handlers::state_json` 在 `load_or_default` 失败时静默吞错；`PUT /api/config` 从 `Config::default()` 重建配置，若请求仅提交部分字段会把未传字段重置为默认值。
- **错误原因**：两套 JSON 解析对 `Null` 处理不一致，`PUT /api/config` 未先加载磁盘已有配置作为 `base`。
- **修复**：
  - `Config::from_json` 对 `log_payload: null` 同样跳过并保持默认值；`Config::validate` 对 `log_level` 先做 `trim + to_ascii_lowercase` 再匹配。
  - `handlers::state_json` 在配置加载失败时记录 `tracing::warn!`；`PUT /api/config` 以 `load_or_default(state.config_path()).unwrap_or_default()` 为基底合并更新。

---

## 3. P2（Payload 真实性、重试状态、设置页交互与文件行数规范）

### [P2-1] `src/logging/redact.rs:76-124`、`src/logging/payload.rs:68-70` — `omit_inline_images` 误调 `sanitize_text` 压缩 JSON 内多空格，且图片脱敏被误标 `truncated: true`

- **关键点**：
  - 原 `omit_inline_images` 对 `serde_json::to_string` 的输出调用了 `sanitize_text`，把字符串字面量内的连续空格（如代码缩进）压缩成单个空格，破坏了用于复现请求的 JSON 负载。
  - `PayloadCapture::start` 用脱敏前的 `request.body.len() > MAX_REQUEST_BODY_BYTES` 计算 `truncated`，导致包含 >2MB 内联图片的请求在图片被正常替换为几十字节占位符后仍被误标 `truncated: true`。
- **错误原因**：将面向单行错误摘要的 `sanitize_text` 用于 JSON 请求体；截断标志在图片脱敏前计算。
- **修复**：
  - `omit_inline_images` 先通过 `may_contain_inline_image` 快速短路无图请求体（直接走 `truncate_json_text`，零 JSON 解析开销且 100% 保留原格式）；含图请求体替换图片后走 `truncate_json_text`，不再调用 `sanitize_text`。
  - `sanitize_text` 在达到 `max_bytes` 时立即提前 `break`，避免大文本全量遍历。
  - `PayloadCapture::start` 以脱敏后文本是否以 `"…[truncated]"` 结尾判定 `truncated`。

### [P2-2] `src/chat/client/mod.rs:204-268` — 重试前未清除旧 attempt 的 `ChatDelta::Finished`，且预取消通道未在入口短路

- **关键点**：
  - 流读取超时或中断时 `finalize_partial` 会向共享 `deltas` 推入 `ChatDelta::Finished(Failed)`；若满足 `can_retry` 进入下一次 attempt，旧的 `Finished(Failed)` 仍留在 `deltas` 中，导致重试成功后 `deltas` 出现两个终态。
  - 当传入的 `cancel` 接收端初始值已为 `true` 时，`wait_cancel` 只等待 `changed()`，不会在发起网络请求前立即返回 `StopReason::Cancelled`。
- **错误原因**：跨 attempt 复用 `deltas` 时未移除上一次尝试追加的 `Finished`；循环入口未检查 `*cancel.borrow()`。
- **修复**：
  - 在 `run_stream_with_log` 循环入口与重试条件中检查 `*cancel.borrow()`；重试前执行 `deltas.retain(|delta| !matches!(delta, ChatDelta::Finished { .. }))`。
  - 新增 `src/chat/client/tests/retry.rs::pre_cancelled_receiver_aborts_before_network_call` 单测。

### [P2-3] `src/tui/mod.rs:42-43, 62-63, 157-176` — `install_panic_hook` 重复包装导致 Panic 时双倍日志与双倍终端恢复

- **关键点**：
  - 工作区首次启动先经 `Tui::init_inline()`（信任确认页）安装一次 panic hook，确认后再经 `Tui::init()` 又 `take_hook` 包装一层，导致后续若发生 panic 会输出两条 `lx::panic` 日志并执行两次 `restore_raw()`。
- **错误原因**：`install_panic_hook` 未做进程内幂等保护。
- **修复**：使用 `static PANIC_HOOK_ONCE: std::sync::Once = std::sync::Once::new();` 保证只安装一次。

### [P2-4] `assets/settings.js:405-425`、`assets/settings.css:240-242`、`src/settings/service/logs.rs:6-19` — 设置页显示文件路径但复制目录路径，且缺少长路径断行与环境变量锁定状态

- **关键点**：
  - `#log_path` 显示 `logging.file || logging.dir`，而 `currentLogPath` 却取 `logging.dir`，用户点击 `Copy path` 复制到的与界面看到的不一致。
  - `logs::logging_json` 未返回 `LX_TUI_LOG` / `LX_TUI_LOG_PAYLOAD` 环境变量覆盖标志；`#log_path` 缺少长路径自动断行样式。
- **错误原因**：展示字段与复制字段取源不一致；状态接口未暴露 `env_locked`。
- **修复**：
  - `currentLogPath` 与 `#log_path` 统一取 `logging.file || logging.dir || ''`；空路径点击 `Copy path` 时提示 `Log path unavailable`。
  - `logs::logging_json` 增加 `"env_locked"` 与 `"payload_env_locked"` 字段。
  - `assets/settings.css` 增加 `#log_path { overflow-wrap: anywhere; word-break: break-all; }`。

### [P2-5] `assets/settings.js`（1007 行）与 `src/settings/service/tests.rs`（1524 行）超出单文件行数上限

- **关键点**：
  - `assets/settings.js` 1007 行超过 `docs/standards/html-assets.md` 的 1000 行上限；
  - `src/settings/service/tests.rs` 1524 行超过 `AGENTS.md` 的单测试文件 1500 行上限。
- **错误原因**：在已接近上限的文件上直接追加代码而未按子模块拆分。
- **修复**：
  - 收紧 `assets/settings.js` 至 992 行（< 1000 行）；
  - 将配置与日志服务测试拆入子模块测试文件 `src/settings/service/logs/tests.rs`（172 行），使 `src/settings/service/tests.rs` 降至 1427 行（< 1500 行）。

---

## 4. P3（细节打磨与文档同步）

### [P3-1] `src/event/chat/mod.rs:213-227`、`src/event/chat/tool_calls.rs:41-46, 152-184, 288-293` — 取消流与工具失败日志补齐关联字段与统一 `target`

- **关键点与修复**：
  - `ChatStreams::cancel` 成功发送取消信号时记录 `tracing::debug!(target: "lx::agent", pane = key.pane.raw(), session = %key.session, generation = key.generation, "agent turn cancel requested")`。
  - `tool_calls.rs` 与 `subagent/tools.rs` 中所有工具落库失败 `warn!` 统一使用 `target: "lx::agent::tool"` 并补齐 `pane` 与 `session` 字段。

### [P3-2] `src/logging/payload.rs:172-187` — `PayloadCapture` JSONL 记录补充 `pid` 字段

- **关键点与修复**：在 `PayloadCapture::finish` 生成的 JSONL 顶层对象中增加 `"pid": std::process::id()`，与诊断日志的 `role=... pid=...` 直接对齐。

### [P3-3] `docs/feature/agent-logging/{design,task,verification}.md` — 同步更新设计、任务与验证文档

- **关键点与修复**：同步更新 `design.md`、`task.md` 与 `verification.md`，反映本次 Review 修复的全部细节与新增单测。
