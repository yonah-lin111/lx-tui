# lx Agent 日志 — 执行任务清单

> 状态：已实现，测试通过，等待合并确认
> 设计依据：[design.md](design.md)
> 执行位置：`.worktrees/agent-logging`（分支 `feat/agent-logging`）
> 完成标准：实现 + 测试 + `cargo fmt` + `cargo clippy --all-targets -- -D warnings` + `cargo test` + 提交

## P0. 开工门槛

- [x] 确认当前目录为 `.worktrees/agent-logging`，分支 `feat/agent-logging`。
- [x] `codegraph init` 已完成；codebase-memory 项目 `lx-tui-agent-logging` 已索引。
- [x] 复核 `src/db/mod.rs` 的 `SCHEMA_VERSION` 仍为 25；本任务不改 DB schema。
- [x] 阅读 `docs/standards/project-directory-structure.md`、`docs/standards/code-writing-standards.md`、`docs/standards/html-assets.md`。
- [x] 确认 `git status` 干净（只允许本任务新增文件）。

## P1. 平台路径与 logging 核心

- [x] 新增 `src/platform/log.rs`：
  - [x] `pub fn log_dir() -> io::Result<PathBuf>`：`LX_TUI_LOG_DIR` 优先；测试 `test_home_override()` → `<home>/.lx-t-dev/logs`；否则 `~/.lx-t[-dev]/logs`。
  - [x] 测试 `src/platform/log/tests.rs`：环境根、`LX_TUI_LOG_DIR`、测试 HOME 覆盖；使用 `set_test_env` + Drop 清理，不污染真实 HOME。
- [x] `src/platform/mod.rs` 增加 `pub mod log;`。
- [x] 新增 `src/logging/mod.rs`：
  - [x] 常量 `RETENTION_DAYS=7`、`DEFAULT_LEVEL="info"`、`LEVELS`。
  - [x] `pub struct LogGuard` 持有 `tracing_appender::non_blocking::WorkerGuard`。
  - [x] `pub fn init(config: &Config, role: &'static str) -> LogGuard`：目录创建、0700、启动清理、daily appender、non-blocking、reload layer、`RoleFormat`、payload 开关、失败降级。
  - [x] `pub fn apply(config: &Config)`：更新 payload；无 env 覆盖时 `handle.reload(build_filter(...))`。
  - [x] `pub fn payload_enabled() -> bool`。
  - [x] `pub fn new_request_id() -> String`。
  - [x] `pub fn active_log_dir() -> Option<PathBuf>`、`pub fn active_log_file() -> Option<PathBuf>`。
  - [x] `pub(crate) fn filter_directives(level: &str) -> String`。
  - [x] `pub(crate) fn normalize_level(level: &str) -> &'static str`。
  - [x] `pub(crate) fn resolve_payload(config: &Config) -> bool`（env 优先）。
  - [x] `RoleFormat` 实现 `FormatEvent`，每行前缀 `role=... pid=...`。
  - [x] `cleanup_old_logs(dir, today, keep_days)` + `utc_today()` + `utc_date` + `parse_log_date`。
  - [x] `spawn_cleanup_thread(dir)`：6 小时循环；进程退出自动终止。
- [x] `Cargo.toml`：
  - [x] `tracing-subscriber` features 增加 `reload`。
  - [x] 新增 `tracing-appender = "0.2.3"`。
- [x] `src/lib.rs` 增加 `pub mod logging;`。
- [x] 新增 `src/logging/tests.rs`：filter、level、payload 优先级、request id、日期、清理。

## P2. 配置与设置服务

- [x] `src/config/mod.rs`：
  - [x] `Config` 增加 `log_level: String`、`log_payload: bool`。
  - [x] `Default` 设 `"info"` / `false`。
  - [x] `validate` 校验 level 白名单。
  - [x] `to_json` 追加两字段（保持 `": "` 风格）。
  - [x] `from_json` 支持 `log_level` String/Null、`log_payload` Bool/Number(0|1)/Null。
  - [x] `JsonValue` 增加 `Bool(bool)`；裸词分支识别 `true/false`。
- [x] `src/config/tests.rs`：默认值、round-trip、旧 JSON、非法 level、bool 解析。
- [x] `src/settings/service/handlers.rs`：
  - [x] `JsonValue` 增加 `Bool(bool)`；`value()` 识别 true/false。
  - [x] `config_from_json` 支持两个新键。
  - [x] `config_json` 输出两个新键。
  - [x] `state_json` 追加 `"logging":{"dir":...,"file":...}`（JSON 转义）。
  - [x] 新路由 `("POST", "/api/logs/open") => open_logs(state).await`。
  - [x] `open_logs` 用 `spawn_blocking` 调 `platform::reveal::reveal_in_file_manager`。
- [x] `src/settings/service/logs/tests.rs`：GET state、PUT config 两字段与增量合并、设置页自定义下拉钩子、`/api/logs/open` 鉴权与同源校验。
- [x] `src/event/inventory.rs:64`：`ConfigChanged` 先 `logging::apply(&updated)` 再赋值。

## P3. 启动与 panic

- [x] `src/main.rs`：
  - [x] `run()` 先 `parse_cli`，再 `load_settings`（失败回退 default + 延迟 warn），再 `logging::init(&config, role_of(&action))`。
  - [x] 删除旧 `init_tracing`。
  - [x] `run_tui(config: Config)` 使用传入配置，删除 `Config::default()`。
  - [x] `role_of(&CliAction)`：`Tui => "tui"`，其余 `"cli"`。
  - [x] `_log_guard` 持有到 `run()` 结束。
- [x] `src/tui/mod.rs::install_panic_hook`：
  - [x] `restore_raw()` 前 `tracing::error!(target:"lx::panic", panic, location)`。
  - [x] 恢复顺序不变。
- [x] `cargo test tui::` 通过。

## P4. 网络链路与 payload

- [x] `src/chat/client/mod.rs`：
  - [x] 新增 `pub struct StreamLog`（字段见 design §4.6）。
  - [x] 新增 `pub async fn run_stream_with_log(...)`；旧 `run_stream` 作为 wrapper 调 `StreamLog::default()`。
  - [x] `run_stream_with_log` 生成 `request_id`，记录 request built / attempt / retry / finished。
  - [x] `stream_once` 拆为 wrapper + `stream_once_inner`，wrapper 保证 `PayloadCapture::finish` 一定执行。
  - [x] HTTP 非 2xx 记录 status、`tools_requested`、2KB 脱敏 detail。
  - [x] 发送失败/超时/断流/取消/frame 超限/解码失败/EOF 收尾日志。
  - [x] `sanitize_error_detail` 改为 `sanitize_text(body, 160)` 包装。
- [x] 新增 `src/logging/redact.rs`：`url_origin`、`url_origin_and_path`、`redacted_headers`、`sanitize_text`、`truncate_utf8`、`omit_inline_images`。
- [x] 新增 `src/logging/payload.rs`：
  - [x] `MAX_REQUEST_BODY_BYTES=2MiB`、`MAX_SSE_LINE_BYTES=256KiB`、`MAX_TOTAL_BYTES=20MiB`。
  - [x] `PayloadCapture::{start,response_status,record_frame,response_body,finish}`。
  - [x] `payload-YYYY-MM-DD.jsonl` 追加写；未开启时零文件。
  - [x] 图片占位、单条/总量截断、`truncated` 标记。
- [x] `src/logging/payload/tests.rs`、`src/logging/redact/tests.rs` 覆盖 design §6.2/6.3。
- [x] `src/chat/client/tests/` 增加 payload 开关与脱敏回归；现有 30 处 `run_stream` 调用不改。

## P5. turn、工具、子 agent 与辅助调用

- [x] `src/event/chat/stream_start.rs`（新，承载 `start_stream`）：
  - [x] 按 `assistant_id` 反查 `turn_index`。
  - [x] 构造 `StreamLog { kind:"chat", ... }` 传给 `run_stream_with_log`。
  - [x] `info "agent turn started"`。
  - [x] `report_turn` 增加 `turn_index` 参数，记录成功/失败终态与耗时。
- [x] `src/event/chat/tool_calls.rs`：
  - [x] BeginTool 成功后 debug 日志（pane/session/call_id/name）。
  - [x] FinishTool 日志补真实 `duration_ms = now_ms - ticket.started_at_ms`、pane/session、is_error、result_bytes。
  - [x] 失败 warn 补 pane/session。
- [x] `src/event/chat/subagent/launch.rs`：`StreamLog { kind:"subagent", session=child, parent_session=parent, agent_id }`。
- [x] `src/event/chat/title.rs`、`recap.rs`、`compact.rs`：`StreamLog { kind:"title"/"recap"/"compact" }`，能拿 session 就填。
- [x] `src/chat/agent/exec/runner.rs`：`StreamLog { kind:"exec", session_id }`。
- [x] `src/cli/session/ops.rs`：CLI compact 的 `StreamLog`。

## P6. 设置页与 CLI

- [x] `assets/settings.html`：Downloads 后新增 Logging fieldset（level select、payload checkbox、path、Copy path、Open folder），文案英文。
- [x] `assets/settings.js`：
  - [x] `generalValues()` 收集 `log_level` / `log_payload`。
  - [x] `resetGeneral()` 恢复两字段。
  - [x] `saveGeneral()` 提交两字段（payload 真布尔）。
  - [x] `applyGeneralData()` 设置两字段与 `#log_path`。
  - [x] `copyLogPath()` / `openLogFolder()`。
- [x] `src/cli/settings.rs`：
  - [x] `SetArgs` 增加 `log_level` / `log_payload`。
  - [x] `SET_FLAGS`、`parse_set`、`run_set`、`run_show`、`config_json`（顶层 `logging`）。
- [x] `src/cli/args.rs`：`VALUE_OPTIONS` 加入 `--log-level`、`--log-payload`。
- [x] `src/cli/help.rs`：`SETTINGS_HELP` 补参数。
- [x] `src/cli/tests/config_domains_tests.rs`：show/set/help 覆盖新字段。

## P7. 规范文档

- [x] `docs/standards/code-writing-standards.md`：补日志目录、级别、轮转、payload 脱敏、panic 顺序。
- [x] `docs/standards/project-directory-structure.md`：模块列表增加 `logging/`；`platform/` 条目增加日志路径。

## P8. 验证与收尾

- [x] `cargo fmt`（或 `cargo fmt --check`）。
- [x] `cargo clippy --all-targets -- -D warnings`。
- [x] `cargo test` 全量。
- [x] 定向：`cargo test logging::`、`cargo test settings::`、`cargo test chat::client::`、`cargo test tui::`、`cargo test cli::tests::config_regressions`。
- [x] 检查新增生产文件 < 1000 行、测试文件 < 1500 行；无 `unwrap()/expect()` 进生产代码。
- [x] `git status` 只包含本任务文件。
- [x] `git add` + `git commit`，提交信息：

```text
feat(logging): add rotating agent logs, live level control, and payload capture
```

- [x] 输出变更摘要、验证结果、风险与未验证项。
- [ ] 询问用户是否合并回 `dev`（不自行合并；等待用户确认）。


## 执行记录（2026-11-10）

- 工作区：`.worktrees/agent-logging`，分支 `feat/agent-logging`。
- codegraph 已初始化；codebase-memory 项目 `lx-tui-agent-logging` 已索引（26,794 nodes / 180,120 edges）。
- 新增：`src/logging/{mod,redact,payload}.rs` 及测试、`src/platform/log.rs` 及测试、`src/event/chat/stream_start.rs`、`src/settings/service/logs.rs` 及 `src/settings/service/logs/tests.rs`。
- Review 修复（详见 [`review.md`](review.md)）：
  - **P0**：补齐 Google Gemini `inlineData` / `inline_data` 内联图片 base64 脱敏；`request_error_code` 使用 `error.without_url()` 防止泄漏 Google `?key=`；`ExecStreamFactory` 增加 `stream_with_log` 为 Headless 子 Agent 传递独立 `StreamLog`；补齐 TUI 与 Headless 子 Agent 工具调用的 `lx::agent::tool` 日志。
  - **P1**：修复 `LX_TUI_LOG=""` 空串关闭日志并锁死热更新问题；为日志文件与 `payload-*.jsonl` 补齐 `0600` 权限与单次 `write_all` 原子行追加；统一 `lx::agent::request` 的 `session` / `agent_id` / `parent_session` 为 `%` Display 格式并将 `tool call settled` 及子 Agent 轮次起止提升为 `INFO`；`Config::from_json` 容忍 `log_payload: null` 且 `PUT /api/config` 支持以磁盘配置为基底增量更新。
  - **P2/P3**：修复 `omit_inline_images` 误调 `sanitize_text` 压缩多空格问题；流式重试前清理旧 `ChatDelta::Finished` 并支持预取消通道短路；`install_panic_hook` 改为 `Once` 幂等安装；统一设置页 `#log_path` 展示与复制路径；将 `assets/settings.js` 收紧至 992 行（< 1000）并将日志服务测试拆入 `src/settings/service/logs/tests.rs`（`service/tests.rs` 降至 1427 行 < 1500）；`payload-*.jsonl` 增加 `pid` 字段。
- 验证：
  - `cargo fmt --check` 通过。
  - `cargo clippy --all-targets -- -D warnings` 通过。
  - `cargo test` 通过。
  - 手工与 CLI 实机验证详见 [`verification.md`](verification.md)。
