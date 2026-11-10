# `lx` Agent 日志 — 验证报告 (`verification.md`)

- **特性目录**：`docs/feature/agent-logging/`
- **关联文档**：[`design.md`](design.md) · [`task.md`](task.md) · [`review.md`](review.md)
- **验证工作区**：原 `.worktrees/agent-logging`（分支 `feat/agent-logging`）及合并后的主工作区 `dev`（merge commit `b9ea251c`；worktree 与分支已清理）
- **验证结论**：全部 10 个验证项与合并后回归测试通过（`PASS`）

---

## 1. CLI 实机与测试套件概览

| 校验项 | 执行命令 | 结果 |
| :--- | :--- | :--- |
| 代码格式与静态检查 | `rtk cargo fmt --check && rtk cargo clippy --all-targets -- -D warnings` | 通过（0 警告 / 0 错误） |
| 平台日志目录推导单测 | `rtk cargo test --lib platform::log` | 通过（3 passed） |
| 日志核心、脱敏与 Payload 单测 | `rtk cargo test --lib logging` | 通过（24 passed） |
| 配置模型与 JSON 布尔解析单测 | `rtk cargo test --lib config::tests` | 通过（13 passed） |
| Web 设置服务与日志 API 单测 | `rtk cargo test --lib settings::service` | 通过（56 passed） |
| 网络流式客户端与重试单测 | `rtk cargo test --lib chat::client` | 通过（37 passed） |
| TUI Panic Hook 与恢复顺序单测 | `rtk cargo test --lib tui::tests` | 通过（1 passed） |
| CLI 设置读写与越界拦截单测 | `rtk cargo test --lib cli::tests::config_domains_tests` | 通过（4 passed） |
| `lx-tui settings` & `lx-tui exec` CLI 实机联调 | `./target/debug/lx-tui settings ...` & `./target/debug/lx-tui exec --ephemeral --json ...` | 通过（Exit Code 0） |

---

## 2. 逐项验证明细

### V01 — 日志目录推导、权限收紧（0700）与 `LX_TUI_LOG_DIR` 覆盖
- **输入内容**：开发/生产模式下的 `HOME` 目录，以及显式环境变量 `LX_TUI_LOG_DIR="$TMP/logs"`。
- **验证步骤**：执行 `rtk cargo test --lib platform::log::tests`，并以 `HOME="$TMP/home" LX_TUI_LOG_DIR="$TMP/logs" ./target/debug/lx-tui --help` 启动 CLI 后检查目录权限。
- **期望结果**：默认推导 `~/.lx-t[-dev]/logs`，设置 `LX_TUI_LOG_DIR` 时优先使用覆盖路径，且日志目录权限收紧为 `0700`（`drwx------`，实测通过）。

### V02 — 按天滚动命名、7 天保留期自动清理与非日志文件保护
- **输入内容**：日志目录预置 8 天前过期文件（`lx-tui.log.2020-01-01`、`payload-2020-01-01.jsonl`）、3 天前近期文件与非日志文件 `notes.txt`。
- **验证步骤**：执行 `rtk cargo test --lib logging::tests::cleanup_removes_only_expired_matching_files`，并在预置过期文件的目录上运行 `./target/debug/lx-tui settings show --json`。
- **期望结果**：早于 7 天保留期的 `lx-tui.log.*` 与 `payload-*.jsonl` 被自动删除，7 天内文件与 `notes.txt` 原样保留（实测通过）。

### V03 — 多入口共享按天日志、`role` / `pid` 前缀与追加写
- **输入内容**：同一日志目录下连续执行 `./target/debug/lx-tui --help` 与 `./target/debug/lx-tui settings show`。
- **验证步骤**：读取生成的 `lx-tui.log.YYYY-MM-DD` 文件内容与行数。
- **期望结果**：两次启动均以追加模式写入同一按天文件（不截断历史），每行以 `role=cli pid=<PID>` 前缀开头（TUI 入口为 `role=tui`，实测通过）。

### V04 — `lx-tui settings` CLI 日志配置读写、重置与非法值拦截
- **输入内容**：
  1. 合法参数：`settings set --log-level debug --log-payload true`
  2. 非法参数：`settings set --log-level verbose`、`settings set --log-payload maybe`
  3. 重置命令：`settings reset`
- **验证步骤**：在隔离 `HOME` 下依次执行 `./target/debug/lx-tui settings set/show/reset` 及 `rtk cargo test --lib cli::tests::config_domains_tests`。
- **期望结果**：合法参数写入 `settings.json` 且 `settings show --json` 返回 `"logging": {"level": "debug", "payload": true}`；非法参数返回非零退出码并报错拦截；`reset` 恢复默认 `"info"` 与 `false`（实测通过）。

### V05 — 日志级别作用域隔离与 `LX_TUI_LOG` / `LX_TUI_LOG_PAYLOAD` 环境变量优先级及空值保护
- **输入内容**：`settings.json` 配置 `log_level="debug", log_payload=false`，同时分别设置 `LX_TUI_LOG=error`、`LX_TUI_LOG=""` 与 `LX_TUI_LOG_PAYLOAD=1`。
- **验证步骤**：执行 `rtk cargo test --lib logging::tests`，并以 `LX_TUI_LOG=error ./target/debug/lx-tui settings show` 实机运行。
- **期望结果**：设置页等级生成 `lx=<level>,lx_tui=<level>,warn`（第三方依赖固定 `warn`）；`LX_TUI_LOG=""` 空串按未设置处理且不锁死热更新；当存在非空 `LX_TUI_LOG=error` 时环境变量优先生效（实测通过）。

### V06 — URL、请求头、错误正文与多供应商内联 Base64 图片（Anthropic / OpenAI / Gemini）脱敏
- **输入内容**：含 `userinfo` 与 `?key=secret#frag` 的 URL、带 `?key=secret-g-key` 的 `reqwest::Error`、含 `Authorization` / `x-api-key` / `Cookie` / `Content-Type` 的请求头、含控制字符的超长错误文本、含 Anthropic `source.data`、OpenAI `data:image/png;base64,...` 及 Google Gemini `inlineData` / `inline_data` 的请求体 JSON。
- **验证步骤**：执行 `rtk cargo test --lib logging::redact::tests` 与 `rtk cargo test --lib chat::client::tests::retry::request_error_code_strips_url_and_query_key`。
- **期望结果**：URL 仅保留 `scheme://host[:port]/path`；`reqwest::Error` 经 `without_url()` 剥离查询参数；敏感请求头值替换为 `[redacted]`、白名单头保留；三类供应商内联图片统一替换为 `[image omitted: N bytes]` 且保留 JSON 代码缩进原样（实测通过）。

### V07 — Payload JSONL 捕获、0600 文件权限、默认关闭零落盘与 2MB / 256KB / 20MB 体积上限
- **输入内容**：`enabled=false` 请求、`enabled=true` 正常 SSE 流、超 2MB 请求体、超 256KB 单条 SSE `data` 帧。
- **验证步骤**：执行 `rtk cargo test --lib logging::payload::tests`。
- **期望结果**：未开启时不创建 `payload-*.jsonl`；开启时以 `0600` 权限单次原子追加写入合法 JSONL（含 `pid` 与 `request_id`）；超限请求体与超限 SSE 帧自动截断并标记 `"truncated": true`（实测通过）。

### V08 — `lx-tui exec` CLI 实机全链路验证（轮次、HTTP 502 重试、工具调用与 Payload 落盘）
- **输入内容**：通过 `lx-tui model` 配置本地 Mock Anthropic 端点（URL 带 `?key=super-secret-query`，API Key 为 `sk-ant-super-secret-key`），开启 `log_level=debug` 与 `log_payload=true`；Mock 服务端第 1 次返回 `502 upstream bad gateway`，第 2 次返回 `shell` `tool_use` SSE 流，第 3 次返回文本完成流。
- **验证步骤**：执行 `./target/debug/lx-tui exec --ephemeral --json "Run shell printf"`，检查 `lx-tui.log.YYYY-MM-DD` 与 `payload-YYYY-MM-DD.jsonl`。
- **期望结果**：
  - 诊断日志按序记录：`agent turn started` → `agent request built`（`request_id=req-...`）→ `agent http error response`（502）→ `agent request retrying`（同 `request_id`，`attempt=1`）→ `agent request finished`（`attempts=2`）→ `tool call started` / `tool call settled` → 第二轮 `agent request built` / `finished` → `agent turn finished`。
  - `payload-YYYY-MM-DD.jsonl` 写入 3 条记录：第 1 条 `attempt=0, status=502, error="http 502 · upstream bad gateway"`，后两条 `status=200, sse_count=6, error=null`。
  - 诊断日志与 Payload 文件均不包含 `sk-ant-super-secret-key` 与 `super-secret-query`（实测通过）。

### V09 — Web 设置服务 API（`/api/state`、`/api/config`、`/api/logs/open`）与热更新
- **输入内容**：`GET /api/state`、`PUT /api/config`（含全量更新、增量更新与非法 `"log_level":"verbose"`）、`POST /api/logs/open`（无 token 与跨源请求）及设置页 HTML/JS 静态资源。
- **验证步骤**：执行 `rtk cargo test --lib settings::service::logs::tests`。
- **期望结果**：`GET /api/state` 返回 `log_level`、`log_payload` 与 `logging.{dir,file,env_locked,payload_env_locked}`；合法 `PUT /api/config` 落盘并发出 `SettingsEvent::ConfigChanged` 触发 `logging::apply` 热更新，增量更新保留已有布局字段；非法级别返回 HTTP `422`；`/api/logs/open` 拒绝无 token 与跨源请求（`403`）；设置页通过 `createSelect` 挂载 `#log-level-host`，无原生 `<select>`（实测通过）。

### V10 — TUI Panic Hook 幂等安装与终端恢复顺序
- **输入内容**：`Once` 幂等安装的 panic 钩子与终端原始模式恢复序列。
- **验证步骤**：执行 `rtk cargo test --lib tui::tests`。
- **期望结果**：`install_panic_hook` 经 `Once` 保证仅包装一次，在调用 `restore_raw()` 前先以 `tracing::error!(target: "lx::panic", ...)` 记录 panic 消息与 `file:line:column` 位置，且终端恢复顺序（关鼠标 → drain → 退备用屏与括号粘贴 → 关 raw mode）严格保持不变（实测通过）。
