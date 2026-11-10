# lx Agent 日志 — 设计文档

> 状态：已完成并合并到 `dev`（merge commit `b9ea251c`；worktree `.worktrees/agent-logging` 与分支 `feat/agent-logging` 已清理）
> 任务清单：[task.md](task.md) · Review 记录：[review.md](review.md) · 验证报告：[verification.md](verification.md)
> 参考实现：`opencode-dev/packages/util/src/observability/logging.ts`、`codex-main/codex-rs/state/src/log_db.rs`、`codex-main/codex-rs/tui/src/startup_orchestration.rs`
> 日期：2026-11-10
> 执行位置：原 `.worktrees/agent-logging`（分支 `feat/agent-logging`，已合并至 `dev` 并移除）

## 0. 背景与目标

当前 `lx-tui` 的日志固定写到 `std::env::temp_dir()/lx-tui.log`，每次启动 `File::create` 截断，同步写、无轮转；网络主链路（`chat/client::stream_once`）没有日志；DB 不保存原始请求/响应。排查 provider 兼容性、SSE 断流、重试、工具失败时缺少可回溯证据。

目标：

1. 诊断日志落盘到稳定目录，按天轮转、保留 7 天，支持设置页热调级别。
2. 一轮对话、一次网络请求、一次工具调用可以用 `session_id` + `turn_index` + `request_id` 串起来。
3. 默认只记录元数据；完整请求/响应通过设置页或环境变量显式开启，单独 JSONL，脱敏且限体积。
4. panic 现场先落日志再恢复终端。
5. 所有入口（TUI / `lx exec` / `lx settings` / `lx model`）共用同一套日志，不互相截断。

成功标准：

- `~/.lx-t-dev/logs/lx-tui.log.YYYY-MM-DD` 可稳定找到；重启不丢历史；7 天前的文件自动清理。
- 设置页保存 `debug` 后新日志立即变详细；保存 `info` 后立即降噪，无需重启。
- 开启 payload 后 `payload-YYYY-MM-DD.jsonl` 只包含脱敏后的请求体与原始 SSE `data` 行；图片 base64 被替换；超限记录带 `truncated=true`。
- panic 日志包含 message 与 `file:line:column`，且终端恢复顺序不变。
- 现有 176 处 tracing 调用与全部测试不受破坏。

## 1. 现状核实（写设计前已逐项确认）

| 事实 | 代码位置 | 对本设计的影响 |
|---|---|---|
| 日志固定写 `$TMPDIR/lx-tui.log`，`File::create` 截断，同步 `Mutex<File>` | `src/main.rs:86-96` | 必须改成稳定目录 + 追加 + 按天轮转；`File::create` 必须移除 |
| 级别只读 `LX_TUI_LOG`，默认 `info`；`try_init` 错误被静默吞掉 | `src/main.rs:89-94` | 需要从 `settings.json` 读初始级别；`try_init` 失败必须显式注释 |
| `init_tracing` 在 `parse_cli` 之前调用 | `src/main.rs:30` | 需要先解析 action 得到 `role`，再初始化日志 |
| TUI 启动用 `Config::default()`，完全不读 `settings.json` | `src/main.rs:51` | 启动改为 `load_settings()`，否则日志等级重启不生效 |
| `tracing-subscriber` 只启用 `env-filter` | `Cargo.toml:19` | `reload` 模块随 `std` 默认可用，无需新增 feature；热更新直接使用 `reload::Layer` |
| 无 `tracing-appender` | `Cargo.toml`、`Cargo.lock` | 新增依赖，用于按天轮转 + 非阻塞写；避免 panic 时 Mutex 自锁 |
| panic hook 先恢复终端、后调用默认 hook | `src/tui/mod.rs:157-164` | 日志必须在 `restore_raw()` 之前写，否则恢复失败会丢现场 |
| 唯一网络出口 `run_stream` 零日志 | `src/chat/client/mod.rs:77-136` | 诊断日志与 payload 捕获都落在这里 |
| 请求只构建一次，重试复用同一个 `TransportRequest` | `src/chat/client/mod.rs:86-124` | `request_id` 在 `run_stream` 入口生成，attempt 递增 |
| `TransportRequest` 只含 `url/headers/body/tools_requested` | `src/chat/transport/mod.rs:138-147` | `body` 已是 JSON 字符串；payload 直接使用，不重新序列化 |
| Google transport 把 API Key 放在 URL query | `src/chat/transport/google/tests.rs:158-160` | 日志与 payload 的 URL 必须丢弃 query |
| Anthropic 请求头含 `x-api-key` | `src/chat/transport/anthropic/mod.rs:148-151` | header 只记录白名单，不记录值 |
| `SseFrame` 只保留去前缀的 `data`，多行用 `\n` 连接 | `src/chat/sse/mod.rs:6-13, 47-94` | payload 记录 `frame.data` 即为原始载荷；注释心跳不记录 |
| 流启动点没有 `turn_index`，只有 `RequestKey{pane,session,generation}` | `src/app/state/lx/chat.rs:244-250` | 在 `start_stream` 按 `assistant_id` 反查消息视图拿 `turn_index` |
| 工具 Begin/Finish 在事件层有 key/ticket | `src/event/chat/tool_calls.rs:53-76, 201-266` | 工具日志落事件层，不在 agent_loop 里猜会话 |
| `Config` 是扁平 JSON，只支持 `u64|string|null` | `src/config/mod.rs:282-288, 359-385` | 新增 `log_level: String`、`log_payload: bool`；两个解析器都要加 `Bool` |
| 设置服务 PUT 从 `Config::default()` 起步 | `src/settings/service/handlers.rs:455-486` | 设置页 JS 必须同时提交新字段，否则保存 General 会把日志设置重置 |
| 设置页 General 的字段收集/恢复/保存 | `assets/settings.js:325-451` | 新增 Logging fieldset 必须同步四处逻辑 |
| `ConfigChanged` 在事件循环内直接赋值 | `src/event/inventory.rs:64` | 热更新挂点：赋值后调用 `logging::apply(&updated)` |
| `reveal_in_file_manager` 已存在，要求后台线程 | `src/platform/reveal.rs:7-50` | 设置页 Open folder 通过 `spawn_blocking` 调用 |
| 权限收紧工具已存在（目录 0700 / 文件 0600） | `src/platform/mod.rs:135-172` | 日志目录创建后调用 `restrict_dir_permissions` |
| DB 无 raw request/response 表 | `src/db/migration/schema.rs` | 本次不改 DB schema，payload 走文件 |

## 2. 已确认决策（Grill Me 收口）

| 编号 | 决策维度 | 已确认结论 |
|---|---|---|
| D1 | 记录粒度 | 默认只记录诊断日志 + 结构化请求元数据；原始请求/响应体由显式开关开启 |
| D2 | 落盘位置 | `~/.lx-t[-dev]/logs/`；按天轮转；保留 7 天；`LX_TUI_LOG_DIR` 可覆盖目录 |
| D3 | 覆盖范围 | 只覆盖 agent 请求链路与故障面；不记录按键、重绘、每个流式 token |
| D4 | 关联方式 | 统一日志文件 + 每条记录带 `session_id`、`turn_index`、`request_id`、`pane`；请求与轮次用 span |
| D5 | payload 开关 | 设置页 + 环境变量 `LX_TUI_LOG_PAYLOAD`；单独 `payload-YYYY-MM-DD.jsonl`；请求头脱敏；同样保留 7 天 |
| D6 | 生效时机 | 设置保存后立即生效；环境变量优先于设置页 |
| D7 | panic | panic 现场写入日志，先写日志再恢复终端 |
| D8 | 查看入口 | 设置页 Logging 区域显示目录/文件，提供 `Open folder` 与 `Copy path` |
| D9 | 级别作用域 | 设置页等级只作用于 `lx_tui` 与显式 `lx::*` target；第三方库固定 `warn`；`LX_TUI_LOG` 可做完整 EnvFilter 覆盖 |
| D10 | 脱敏 | header 白名单；URL 丢弃 query；错误正文截断 2KB；prompt/文件内容/工具参数默认不落 |
| D11 | 入口 | TUI 与全部 CLI 子命令共用按天日志，带 `role=tui|cli`；追加不截断 |
| D12 | 启动配置 | 启动时完整加载 `settings.json` 并在 TUI 使用，失败回退默认 + warn |
| D13 | payload 内容 | 请求体 JSON（图片 base64 替换为占位）+ 原始 SSE `data` 行 + 状态码；不额外存拼装回复；单请求体 2MB、单 SSE 行 256KB、单请求总量 20MB，超限 `truncated=true` |

## 3. 总体架构与数据流

```text
main::run
  ├─ parse_cli(args) -> CliAction
  ├─ config::load_settings() -> Config（失败回退 default + 延迟 warn）
  ├─ logging::init(&config, role = tui|cli)
  │    ├─ platform::log::log_dir()
  │    │    ├─ LX_TUI_LOG_DIR（最高优先）
  │    │    ├─ 测试 HOME 覆盖
  │    │    └─ ~/.lx-t[-dev]/logs
  │    ├─ cleanup_old_logs(dir, 7d)
  │    ├─ tracing_appender daily: lx-tui.log.YYYY-MM-DD
  │    ├─ reload::Layer<EnvFilter>（热更新句柄）
  │    ├─ RoleFormat（每行前缀 role=... pid=...）
  │    └─ PayloadSink 全局开关
  ├─ run_tui(config) / run_cli(action)
  └─ LogGuard 持有 WorkerGuard 到进程结束

agent 一轮：
on_turn_committed
  └─ Effect::StartChatStream { key, assistant_id }
      └─ start_stream
          ├─ 反查 turn_index
          ├─ info "agent turn started"
          ├─ StreamLog { kind: chat, pane, session, turn_index, generation }
          └─ tokio::spawn
              └─ host::drive_agent_turn
                  └─ run_stream_with_log
                      ├─ request_id 生成
                      ├─ build_request
                      ├─ info "agent request built"
                      ├─ attempt loop
                      │   ├─ PayloadCapture::start（仅开关开启）
                      │   ├─ stream_once_inner
                      │   │   ├─ HTTP status / error detail
                      │   │   └─ SseParser frames -> PayloadCapture
                      │   ├─ debug "agent stream attempt finished"
                      │   └─ warn "agent request retrying"（可重试时）
                      ├─ info "agent request finished"
                      └─ PayloadCapture::finish -> payload-YYYY-MM-DD.jsonl
```

## 4. 详细设计

### 4.1 模块与文件布局

新增：

```text
src/logging/mod.rs          # init / reload / filter / role format / request_id / cleanup
src/logging/redact.rs       # URL/header/文本脱敏与截断、图片占位
src/logging/payload.rs      # PayloadCapture、JSONL 写入、体积上限
src/logging/tests.rs        # 纯函数与临时目录测试
src/logging/redact/tests.rs # 脱敏场景测试（按场景拆分）
src/logging/payload/tests.rs
src/platform/log.rs         # log_dir()：环境根 + LX_TUI_LOG_DIR + 测试覆盖
```

依赖方向：`main -> logging`、`chat/event -> logging`、`logging -> platform + tracing/tracing-subscriber/serde_json`。`logging` 不反向依赖 `app/ui`；不引入 `crate::layout`（`StreamLog.pane` 用 `u32`）。

`src/logging/mod.rs` 控制在 500 行左右；`payload.rs` 与 `redact.rs` 各自独立，避免单文件超 1000 行。

### 4.2 日志目录、文件命名与保留

`src/platform/log.rs`：

```rust
/// 日志根目录：生产 `~/.lx-t/logs`，开发 `~/.lx-t-dev/logs`。
pub fn log_dir() -> io::Result<PathBuf> {
    if let Some(dir) = std::env::var_os("LX_TUI_LOG_DIR").filter(|v| !v.is_empty()) {
        return Ok(PathBuf::from(dir));
    }
    #[cfg(test)]
    if let Some(home) = super::test_home_override() {
        return Ok(home.join(".lx-t-dev").join("logs"));
    }
    let home = super::home_dir()
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "home directory unavailable"))?;
    let root = if super::dev_environment() { ".lx-t-dev" } else { ".lx-t" };
    Ok(home.join(root).join("logs"))
}
```

命名：

- 诊断日志：`lx-tui.log.YYYY-MM-DD`（`tracing_appender::rolling::daily`，prefix `lx-tui.log`）。
- payload：`payload-YYYY-MM-DD.jsonl`。
- 日期统一 UTC；payload 文件名由自写 `utc_date()` 生成，避免新增 `chrono`/`time` 直接依赖。

初始化时：

```rust
let dir = crate::platform::log::log_dir()
    .unwrap_or_else(|_| std::env::temp_dir().join("lx-tui-logs"));
std::fs::create_dir_all(&dir)?;
let _ = crate::platform::restrict_dir_permissions(&dir); // 有意忽略：权限收紧失败不回退日志
cleanup_old_logs(&dir, RETENTION_DAYS);
```

保留策略：启动时清理一次，并启动一个 6 小时循环的后台清理线程（`std::thread::spawn`，进程退出即终止）。只删除匹配 `lx-tui.log.YYYY-MM-DD` / `payload-YYYY-MM-DD.jsonl` 且日期早于“今天 - 6 天”的文件；解析失败的文件不删。删除失败忽略并继续（日志清理不能阻断启动）。

```rust
pub(crate) fn cleanup_old_logs(dir: &Path, today: (i32, u32, u32), keep_days: u32) {
    let Some(cutoff) = utc_date_days(today).checked_sub(i64::from(keep_days.saturating_sub(1))) else { return };
    for entry in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        let Some(date) = parse_log_date(&name) else { continue };
        if utc_date_days(date) < cutoff {
            let _ = std::fs::remove_file(entry.path()); // 有意忽略：并发/权限失败不阻断
        }
    }
}
```

### 4.3 tracing subscriber 组装与热更新

`Cargo.toml`：

```toml
tracing-subscriber = { version = "0.3", features = ["env-filter"] } # reload 随 std 可用
tracing-appender = "0.2" # 实际锁定 0.2.5
```

`src/logging/mod.rs` 核心：

```rust
const RETENTION_DAYS: u32 = 7;
const DEFAULT_LEVEL: &str = "info";
const LEVELS: [&str; 6] = ["trace", "debug", "info", "warn", "error", "off"];

static RELOAD: OnceLock<reload::Handle<EnvFilter, Registry>> = OnceLock::new();
static ACTIVE_DIR: OnceLock<PathBuf> = OnceLock::new();
static PAYLOAD: AtomicBool = AtomicBool::new(false);
static ENV_FILTER_LOCKED: AtomicBool = AtomicBool::new(false);

pub struct LogGuard {
    _worker: tracing_appender::non_blocking::WorkerGuard,
}

pub fn init(config: &Config, role: &'static str) -> LogGuard {
    let dir = crate::platform::log::log_dir()
        .unwrap_or_else(|_| std::env::temp_dir().join("lx-tui-logs"));
    let _ = std::fs::create_dir_all(&dir);
    let _ = crate::platform::restrict_dir_permissions(&dir);
    cleanup_old_logs(&dir, utc_today(), RETENTION_DAYS);
    let _ = ACTIVE_DIR.set(dir.clone());
    spawn_cleanup_thread(dir.clone());

    let appender = tracing_appender::rolling::Builder::new()
        .rotation(tracing_appender::rolling::Rotation::DAILY)
        .filename_prefix("lx-tui.log")
        .build(&dir);
    let (writer, worker) = tracing_appender::non_blocking(appender);

    let (filter_layer, handle) = reload::Layer::new(initial_filter(config));
    let format = tracing_subscriber::fmt::format::Format::default()
        .with_target(true)
        .with_ansi(false);
    let layer = tracing_subscriber::fmt::layer()
        .event_format(RoleFormat { role, inner: format })
        .with_writer(writer);

    // 有意忽略：重复初始化（测试/嵌入）时保留已有 subscriber。
    let _ = tracing_subscriber::registry()
        .with(filter_layer)
        .with(layer)
        .try_init();
    let _ = RELOAD.set(handle);

    PAYLOAD.store(resolve_payload(config), Ordering::Relaxed);
    if env_filter_was_invalid() {
        tracing::warn!(target: "lx::logging", "LX_TUI_LOG is invalid; using settings log level");
    }
    LogGuard { _worker: worker }
}
```

过滤器：

```rust
pub(crate) fn filter_directives(level: &str) -> String {
    // `lx::*` 是现有显式 target；`lx_tui` 是 crate 默认 target。
    format!("lx={level},lx_tui={level},warn")
}

fn build_filter(level: &str) -> EnvFilter {
    let level = normalize_level(level);
    EnvFilter::new(filter_directives(&level))
}
```

- 设置页等级只提高/降低 `lx` 与 `lx_tui`；`reqwest`/`hyper`/`rustls`/`rusqlite` 等保持 `warn`。
- `LX_TUI_LOG` 存在时使用完整 EnvFilter，并设置 `ENV_FILTER_LOCKED=true`；此后设置页保存只更新 payload 开关，不覆盖级别（下次不带环境变量启动才生效）。
- 非法等级在 `Config::validate` 拒绝；运行期 `apply` 再做一次兜底，非法值回退 `info`。

热更新：

```rust
pub fn apply(config: &Config) {
    PAYLOAD.store(resolve_payload(config), Ordering::Relaxed);
    if ENV_FILTER_LOCKED.load(Ordering::Relaxed) {
        return;
    }
    if let Some(handle) = RELOAD.get()
        && let Ok(filter) = EnvFilter::try_new(filter_directives(&normalize_level(&config.log_level)))
    {
        let _ = handle.reload(filter); // 有意忽略：过滤层已失效时保持旧过滤器
    }
}
```

`src/event/inventory.rs:64` 改为：

```rust
SettingsEvent::ConfigChanged(updated) => {
    crate::logging::apply(&updated);
    *config = updated;
}
```

每行带 `role` 与 `pid`：自定义 `FormatEvent` 包装默认格式：

```rust
struct RoleFormat {
    role: &'static str,
    inner: tracing_subscriber::fmt::format::Format<tracing_subscriber::fmt::format::Full>,
}

impl<S, N> tracing_subscriber::fmt::FormatEvent<S, N> for RoleFormat
where
    S: tracing::Subscriber + for<'a> tracing_subscriber::registry::LookupSpan<'a>,
    N: for<'a> tracing_subscriber::fmt::format::FormatFields<'a> + 'static,
{
    fn format_event(
        &self,
        ctx: &tracing_subscriber::fmt::FmtContext<'_, S, N>,
        mut writer: tracing_subscriber::fmt::format::Writer<'_>,
        event: &tracing::Event<'_>,
    ) -> std::fmt::Result {
        write!(writer, "role={} pid={} ", self.role, std::process::id())?;
        self.inner.format_event(ctx, writer, event)
    }
}
```

`request_id`：

```rust
static REQUEST_SEQ: AtomicU64 = AtomicU64::new(0);

pub fn new_request_id() -> String {
    let seq = REQUEST_SEQ.fetch_add(1, Ordering::Relaxed);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or_default();
    let mixed = nanos ^ seq.wrapping_mul(0x9e37_79b9_7f4a_7c15);
    format!("req-{:08x}", mixed as u32)
}
```

### 4.4 设置项与配置解析

`src/config/mod.rs`：

```rust
pub struct Config {
    // ...现有字段
    /// 日志级别：trace/debug/info/warn/error/off。
    pub log_level: String,
    /// 是否记录原始请求/响应 payload。
    pub log_payload: bool,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            // ...
            log_level: "info".into(),
            log_payload: false,
        }
    }
}
```

`validate` 增加：

```rust
if !matches!(self.log_level.as_str(), "trace" | "debug" | "info" | "warn" | "error" | "off") {
    return Err(ConfigError::Invalid(
        "log_level must be one of trace|debug|info|warn|error|off".into(),
    ));
}
```

`to_json` 在 `image_download_dir` 后追加（保持 `": "` 风格）：

```text
  "image_download_dir": {},
  "log_level": {},
  "log_payload": {}
```

`from_json` 增加：

```rust
if key == "log_level" {
    match value {
        JsonValue::String(level) => {
            config.log_level = level.trim().to_ascii_lowercase();
            continue;
        }
        JsonValue::Null => continue,
        _ => return Err(ConfigError::Invalid("log_level must be a string".into())),
    }
}
if key == "log_payload" {
    config.log_payload = json_bool(value)
        .ok_or_else(|| ConfigError::Invalid("log_payload must be a boolean".into()))?;
    continue;
}
```

极简解析器增加 `Bool(bool)`：

```rust
enum JsonValue { Number(u64), String(String), Bool(bool), Null, Other }

// 裸词分支
let word = &self.bytes[start..self.index];
Ok(match word {
    b"true" => JsonValue::Bool(true),
    b"false" => JsonValue::Bool(false),
    b"null" => JsonValue::Null,
    _ => JsonValue::Other,
})
```

`settings/service/handlers.rs` 的 `JsonValue` 同步增加 `Bool(bool)`，`config_from_json` 支持 `log_level`（String/Null）与 `log_payload`（Bool，兼容 Number 0/1），`config_json` 输出两字段。`state_json` 追加：

```json
"logging": {"dir": "<log dir>", "file": "<today log file>"}
```

`logging` 暴露：

```rust
pub fn active_log_dir() -> Option<PathBuf>; // ACTIVE_DIR
pub fn active_log_file() -> Option<PathBuf>; // dir.join(format!("lx-tui.log.{date}"))
```

### 4.5 启动配置加载

`src/main.rs` 重构：

```rust
fn run() -> Result<i32, Box<dyn std::error::Error>> {
    let action = match parse_cli(std::env::args_os()) {
        Ok(action) => action,
        Err(usage_error) => {
            eprintln!("lx-tui: {usage_error}");
            return Ok(EXIT_ERR);
        }
    };
    let (config, config_error) = match lx_tui::config::load_settings() {
        Ok(config) => (config, None),
        Err(error) => (Config::default(), Some(error)),
    };
    let _log_guard = lx_tui::logging::init(&config, role_of(&action));
    if let Some(error) = config_error {
        tracing::warn!(target: "lx::config", %error, "settings.json load failed; using defaults");
    }
    tracing::info!(target: "lx::logging", "lx-tui starting");
    match action {
        CliAction::Tui => {
            run_tui(config)?;
            Ok(EXIT_OK)
        }
        other => run_cli(other, database_worktree().as_deref()),
    }
}

fn role_of(action: &CliAction) -> &'static str {
    if matches!(action, CliAction::Tui) { "tui" } else { "cli" }
}
```

`run_tui(config: Config)` 删除 `let config = Config::default();`，其余不变。日志初始化失败时 `logging::init` 内部降级到临时目录或 `io::sink()`，不阻断任何子命令。

### 4.6 关联字段与 `StreamLog`

在 `src/chat/client/mod.rs` 新增（只用原始类型，不依赖 `layout`）：

```rust
/// 一次网络流的关联快照；由调用点构造。
#[derive(Debug, Clone, Default)]
pub struct StreamLog {
    pub kind: &'static str,          // chat / subagent / title / recap / compact / auto_compact / exec
    pub pane: Option<u32>,
    pub session_id: Option<String>,
    pub turn_index: Option<i64>,
    pub generation: Option<u64>,
    pub agent_id: Option<String>,
    pub parent_session: Option<String>,
}
```

保留旧签名给测试，生产走新函数：

```rust
pub async fn run_stream(/* 现有参数 */) -> StreamOutcome {
    run_stream_with_log(/* 现有参数 */, &StreamLog::default()).await
}

pub async fn run_stream_with_log(
    transport_kind: &str,
    base_url: &str,
    api_key: Option<&str>,
    idle_timeout: Duration,
    request: ChatRequest,
    log: &StreamLog,
    cancel: watch::Receiver<bool>,
    on_delta: impl FnMut(&[ChatDelta]),
) -> StreamOutcome;
```

主对话在 `start_stream` 反查 `turn_index`：

```rust
let turn_index = state.pane_anywhere(key.pane).and_then(|pane| {
    pane.lx.chat.messages.iter()
        .find(|message| message.id == assistant_id)
        .map(|message| message.turn_index)
});
let stream_log = StreamLog {
    kind: "chat",
    pane: Some(key.pane.raw()),
    session_id: Some(key.session.clone()),
    turn_index,
    generation: Some(key.generation),
    ..StreamLog::default()
};
```

子 agent 在 `launch.rs` 构造 `kind="subagent"`，`session_id=child_key.session`，`parent_session=parent_key.session`，`agent_id=record.agent_id`。标题/recap/压缩/CLI exec 在各自调用点构造对应 `kind`，能拿到 session 就填。

### 4.7 网络链路日志注入点

`run_stream_with_log`：

| 时机 | 级别 | 字段 |
|---|---|---|
| 请求构建成功 | info | `request_id`、`kind`、`pane`、`session`、`turn`、`generation`、`transport`、`provider`、`model`、`variant`、`purpose`、`turns`、`items`、`tools`、`body_bytes`、`system_bytes`、`url_host` |
| 每次 attempt 开始 | debug | `request_id`、`attempt`、`idle_timeout_ms`、`body_bytes` |
| attempt 结束 | debug | `request_id`、`attempt`、`reason`、`attempt_deltas`、`elapsed_ms` |
| 可重试 | warn | `request_id`、`attempt`、`reason`、`backoff_ms` |
| 最终结束 | info | `request_id`、`attempts`、`reason`、`deltas`、`elapsed_ms` |
| 构建失败 | warn | `transport`、`code` |

`stream_once_inner`：

| 时机 | 级别 | 字段 |
|---|---|---|
| HTTP 非 2xx | warn | `request_id`、`status`、`tools_requested`、`detail`（2KB 脱敏） |
| 请求发送失败/超时 | warn | `request_id`、`code` |
| 首字节超时 | warn | `request_id`、`timeout_ms` |
| chunk 读取超时/断流 | warn | `request_id`、`code`、`bytes_read` |
| frame 超限 | warn | `request_id`、`max_frame_bytes` |
| 解码失败 | warn | `request_id`、`code`、`frame_event`、`frame_bytes` |
| EOF 收尾 | debug | `request_id`、`residual`、`open_tool_input`、`has_completion`、`reason` |
| 取消 | debug | `request_id`、`stage`（send/stream/backoff） |

关键代码（不打印完整 URL）：

```rust
tracing::info!(
    target: "lx::agent::request",
    request_id = %request_id,
    kind = log.kind,
    pane = ?log.pane,
    session = ?log.session_id,
    turn = ?log.turn_index,
    generation = ?log.generation,
    transport = transport_kind,
    provider = %request.model.provider,
    model = %request.model.model,
    variant = ?request.model.variant,
    purpose = ?request.purpose,
    turns = request.turns.len(),
    items = request.items.len(),
    tools = request.tools.len(),
    body_bytes = built.body.len(),
    system_bytes = request.system_prompt.as_ref().map_or(0, String::len),
    url_host = %crate::logging::redact::url_origin(&built.url),
    "agent request built"
);
```

错误正文：

```rust
let detail = crate::logging::redact::sanitize_text(&body, 2048);
tracing::warn!(
    target: "lx::agent::request",
    request_id = %request_id,
    status = status.as_u16(),
    tools_requested = built.tools_requested,
    detail = %detail,
    "agent http error"
);
```

`src/chat/client/mod.rs` 现有的 `sanitize_error_detail` 改为 `sanitize_text(body, 160)` 的薄包装，DB 语义不变。

### 4.8 payload 捕获

`src/logging/payload.rs`：

```rust
pub const MAX_REQUEST_BODY_BYTES: usize = 2 * 1024 * 1024;
pub const MAX_SSE_LINE_BYTES: usize = 256 * 1024;
pub const MAX_TOTAL_BYTES: usize = 20 * 1024 * 1024;

pub struct PayloadCapture {
    enabled: bool,
    request_id: String,
    meta: StreamLog,
    attempt: u32,
    request: Option<PayloadRequest>,
    status: Option<u16>,
    sse: Vec<SseRecord>,
    bytes: usize,
    truncated: bool,
}

impl PayloadCapture {
    pub fn start(built: &TransportRequest, log: &StreamLog, request_id: &str, attempt: u32) -> Self;
    pub fn response_status(&mut self, status: u16);
    pub fn record_frame(&mut self, frame: &SseFrame);
    pub fn response_body(&mut self, body: &str);
    pub fn finish(self, error: Option<&str>);
}
```

JSONL 记录形状：

```json
{
  "ts": "2026-11-10T12:34:56.789Z",
  "request_id": "req-1a2b3c4d",
  "kind": "chat",
  "pane": 1,
  "session_id": "ses-...",
  "turn_index": 3,
  "generation": 7,
  "attempt": 0,
  "request": {
    "method": "POST",
    "url": "https://api.anthropic.com/v1/messages",
    "headers": {"content-type": "application/json", "anthropic-version": "2023-06-01"},
    "body": { "model": "...", "messages": ["..."] }
  },
  "response": {
    "status": 200,
    "sse": [{"event": "content_block_delta", "data": "..."}],
    "body": null
  },
  "error": null,
  "truncated": false
}
```

实现要点：

1. `start` 时若 `!payload_enabled()` 返回 `enabled=false`，后续方法全部 no-op，不分配大对象。
2. 请求体：`omit_inline_images(&built.body)`，再做 2MB 截断；截断置 `truncated=true`。
3. header 白名单：`content-type`、`accept`、`user-agent`、`anthropic-version`、`anthropic-beta`、`openai-beta`、`x-goog-api-client`。其余只记录名字，不记录值。
4. URL：`url_origin_and_path()` 丢弃 query/fragment/userinfo；Google 的 `?key=` 不会落盘。
5. SSE：每个 `frame` 记录 `event` 与 `data`；单条 `data` 超 256KB 截断；累计超 20MB 后停止追加并置 `truncated=true`。
6. 写文件：一次请求结束（成功/失败/取消）同步追加一行 JSON 到 `payload-YYYY-MM-DD.jsonl`。payload 是显式调试模式，允许同步写；不引入后台线程，避免生命周期与丢记录问题。
7. 目录不存在时 `create_dir_all`；写失败只 `tracing::debug!` 一次，不递归写 payload。

`stream_once` 改造为包装器，保证所有 early return 都落到 `finish`：

```rust
async fn stream_once(/* 现有参数 */, capture: &mut PayloadCapture) -> StopReason {
    let reason = stream_once_inner(/* 现有参数 */, capture).await;
    let error = (!matches!(reason, StopReason::Complete)).then(|| reason.as_str());
    capture.finish(error);
    reason
}
```

`stream_once_inner` 内：
- HTTP 响应后 `capture.response_status(status.as_u16())`。
- 非 2xx 读到 body 后 `capture.response_body(&detail)`。
- 每个 frame 在 `decode_frame` 前 `capture.record_frame(&frame)`。

### 4.9 脱敏规则

`src/logging/redact.rs`：

```rust
pub fn url_origin(url: &str) -> String;
pub fn url_origin_and_path(url: &str) -> String;
pub fn redacted_headers(headers: &[(String, String)]) -> Vec<(String, String)>;
pub fn sanitize_text(text: &str, max_bytes: usize) -> String;
pub fn truncate_utf8(text: &str, max_bytes: usize) -> &str;
pub fn omit_inline_images(body_json: &str) -> String;
```

规则：

- URL：只保留 `scheme://host[:port]/path`；`userinfo`、`?query`、`#fragment` 全部丢弃。解析失败时返回 `<invalid-url>`。
- header：只保留白名单键；`authorization`、`x-api-key`、`api-key`、`cookie`、`set-cookie` 一律跳过；未知键只记录名字（值写 `[redacted]`）。
- 文本：控制字符（除 `\n`/`\t`）替换为空格，连续空白压成单行；按 UTF-8 边界截断到 `max_bytes`，超限追加 `…[truncated]`。
- 图片：递归遍历 JSON；字符串值以 `data:image/` 开头且包含 `;base64,` 时替换为 `[image omitted: N bytes]`；对象满足 `{"type":"base64","data":...}` 或 `{"type":"image","source":{"data":...}}` 时替换 `data`。解析失败时对原文做 2MB 截断，不做正则猜测。
- 默认日志不记录 `system_prompt` 内容、`turns` 文本、`items` 文本、工具 `arguments_json`、工具结果 `result_json`；只记录长度/数量/名称/状态。

### 4.10 turn、工具与 panic 日志

turn：

- `start_stream` 顶部：`info!(target:"lx::agent", pane, session, generation, turn, provider, model, "agent turn started")`。
- `report_turn` 增加 `turn_index: Option<i64>` 参数：`info!`（成功）或 `warn!`（失败）记录 `reason`、`usage_input/output`、`billing`、`elapsed_ms`、`deltas`。

工具（`src/event/chat/tool_calls.rs`）：

- BeginTool 成功后：`debug!(target:"lx::agent::tool", pane, session, call_id, name, "tool call started")`。
- FinishTool 落库成功后：保留 debug，但字段改为真实执行耗时 `crate::db::now_ms().saturating_sub(ticket.started_at_ms)`，并加 `pane/session/result_bytes/is_error`。
- 现有失败 warn 补充 `pane/session`。

panic（`src/tui/mod.rs`）：

```rust
fn install_panic_hook() {
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |panic_info| {
        tracing::error!(
            target: "lx::panic",
            panic = %panic_info,
            location = panic_info.location().map(|location| format!(
                "{}:{}:{}", location.file(), location.line(), location.column()
            )).as_deref(),
            "panic; restoring terminal"
        );
        let _ = restore_raw();
        default_hook(panic_info);
    }));
}
```

非阻塞 writer 保证 panic 时不会与日志写入线程争抢 `Mutex<File>` 而死锁。恢复顺序不变。

### 4.11 设置页 Logging 区域与打开目录

`assets/settings.html` 在 Downloads fieldset 之后插入：

```html
<fieldset>
  <legend>Logging</legend>
  <div class="field-grid">
    <div>
      <label for="log_level">Log level</label>
      <select id="log_level">
        <option value="trace">trace</option>
        <option value="debug">debug</option>
        <option value="info">info</option>
        <option value="warn">warn</option>
        <option value="error">error</option>
        <option value="off">off</option>
      </select>
    </div>
    <div class="field-wide">
      <label class="field-hint">
        <input id="log_payload" type="checkbox">
        Log request/response payloads (may contain prompts and code)
      </label>
    </div>
    <div class="field-wide">
      <label>Log files</label>
      <code id="log_path">-</code>
      <div class="actions">
        <button type="button" class="ghost" onclick="copyLogPath()">Copy path</button>
        <button type="button" class="ghost" onclick="openLogFolder()">Open folder</button>
      </div>
    </div>
  </div>
</fieldset>
```

`assets/settings.js`：

- `generalValues()` 增加 `log_level`（`.value`）与 `log_payload`（`.checked`）。
- `resetGeneral()` / `applyGeneralData()` 同步恢复。
- `saveGeneral()` 发送 `config.log_level` 与 `config.log_payload`（真布尔）。
- `applyGeneralData()` 用 `data.logging.file || data.logging.dir` 更新 `#log_path`，并同步保存 `currentLogPath`。
- `copyLogPath()`：`navigator.clipboard.writeText(currentLogPath)`，空路径或失败时弹出错误 toast。
- `openLogFolder()`：`api('/api/logs/open', {method: 'POST', body: '{}'})`，成功/失败 toast。
- 所有文案英文。

服务端：

- `GET /api/state` 追加 `"logging": {"dir": "...", "file": "...", "env_locked": false, "payload_env_locked": false}`。
- `POST /api/logs/open`：token + 同源校验通过后，确保目录存在且收紧为 `0700`，再经 `tokio::task::spawn_blocking(move || platform::reveal::reveal_in_file_manager(&dir))` 打开；成功 `{"ok":true}`，失败 500。
- 路径来自 `logging::active_log_dir()`；为空时回退 `platform::log::log_dir()`。

### 4.12 CLI 设置

`src/cli/settings.rs`：

- `SetArgs` 增加 `log_level: Option<String>`、`log_payload: Option<bool>`。
- `--log-level <trace|debug|info|warn|error|off>`；`--log-payload <true|false>`。
- `VALUE_OPTIONS`（`src/cli/args.rs`）同步加入两个参数，避免被当成子命令。
- `run_set` 写入并 `validate`；`run_show` 人类可读输出增加 `Log level:`、`Log payload:`。
- `config_json` 增加顶层：

```json
"logging": {"level": "info", "payload": false}
```

- `SETTINGS_HELP`（`src/cli/help.rs`）补参数说明。

### 4.13 规范文档更新

- `docs/standards/code-writing-standards.md` 的“错误处理与日志”增加：
  - 日志目录/文件名/轮转/保留；
  - 级别语义（设置页只控制 `lx`/`lx_tui`，依赖 `warn`）；
  - 默认不记录 prompt/文件内容/工具参数；payload 仅显式开启；
  - panic 必须先写日志再恢复终端。
- `docs/standards/project-directory-structure.md` 模块列表增加 `logging/`，`platform/` 条目增加日志路径。

### 4.14 失败降级与错误处理

- `logging::init` 不返回 `Err`：目录创建/权限/文件打开失败时退回 `temp_dir()/lx-tui-logs`；再失败用 `std::io::sink()` 作为 writer，应用继续启动。
- 所有 `let _ = ...` 必须带注释说明“有意忽略”的原因。
- 生产代码禁止 `unwrap()/expect()`；测试允许。
- payload 写失败只降级为 debug 日志，不影响请求结果。
- `ConfigChanged` 热更新失败保持旧 filter，不 panic。

## 5. 关键代码清单（新增 / 修改）

新增：

- `src/logging/mod.rs`
- `src/logging/redact.rs`
- `src/logging/payload.rs`
- `src/logging/tests.rs`
- `src/logging/redact/tests.rs`
- `src/logging/payload/tests.rs`
- `src/platform/log.rs`
- `src/platform/log/tests.rs`

修改：

- `Cargo.toml`：新增 `tracing-appender`（0.2，锁定 0.2.5）；`tracing-subscriber` feature 不变。
- `src/main.rs`：先解析 action、加载配置、初始化日志、`run_tui(config)`。
- `src/lib.rs`：导出 `logging`。
- `src/platform/mod.rs`：导出 `log`。
- `src/config/mod.rs`：两个字段、`Bool`、序列化/解析/校验。
- `src/config/tests.rs`：新字段测试。
- `src/settings/service/handlers.rs`：`Bool`、配置字段、`logging` state 路由。
- `src/settings/service/logs.rs`（新）：`logging_json` 与 `/api/logs/open` 处理器。
- `src/settings/service/tests.rs`：GET/PUT 覆盖新字段。
- `src/event/inventory.rs`：`ConfigChanged` 调用 `logging::apply`。
- `src/event/chat/mod.rs`：turn 日志、`report_turn` 参数、`stream_start` 子模块声明。
- `src/event/chat/stream_start.rs`（新）：`start_stream` 主体、`turn_index` 反查、`StreamLog` 构造。
- `src/event/chat/tool_calls.rs`：工具日志字段。
- `src/event/chat/subagent/launch.rs`：子 agent `StreamLog`。
- `src/event/chat/title.rs`、`recap.rs`、`compact.rs`：辅助调用 `StreamLog`。
- `src/chat/client/mod.rs`：`StreamLog`、`run_stream_with_log`、attempt 日志、payload 包装。
- `src/chat/client/tests/mod.rs` / `tools.rs`：如需适配 wrapper，保持旧签名不改。
- `src/chat/events.rs`：无结构变更（复用 `StopReason::as_str`）。
- `src/chat/agent/exec/runner.rs`、`src/cli/session/ops.rs`：`StreamLog`。
- `src/cli/settings.rs`、`src/cli/args.rs`、`src/cli/help.rs`：CLI 参数与 show 输出。
- `src/tui/mod.rs`：panic 日志。
- `assets/settings.html`、`assets/settings.js`：Logging 区域。
- `docs/standards/code-writing-standards.md`、`docs/standards/project-directory-structure.md`。

## 6. 测试计划

### 6.1 纯逻辑（`src/logging/tests.rs`）

- `filter_directives("debug") == "lx=debug,lx_tui=debug,warn"`；`off` 同理。
- `normalize_level`：大小写、空白、非法值回退 `info`。
- `resolve_payload`：默认 settings；`LX_TUI_LOG_PAYLOAD=1` 强制开；`=0` 强制关。
- `new_request_id`：格式 `req-[0-9a-f]{8}`，连续 1000 次不重复。
- `utc_date`：epoch 0 → `1970-01-01`；已知时间戳 → 固定日期。
- `cleanup_old_logs`：临时目录构造 8 天前/今天文件，删除旧文件、保留新文件、忽略非法文件名。

### 6.2 脱敏（`src/logging/redact/tests.rs`）

- URL：`https://user:pass@host:8443/v1/messages?key=secret#x` → `https://host:8443/v1/messages`。
- header：`Authorization` / `x-api-key` / `cookie` 不出现在结果；白名单键保留值。
- `sanitize_text`：控制字符替换、2KB 截断、UTF-8 边界不 panic。
- `omit_inline_images`：Anthropic `source.data`、OpenAI `image_url` data URL 都变成 `[image omitted: N bytes]`；普通文本不变；非法 JSON 回退原文截断。

### 6.3 payload（`src/logging/payload/tests.rs`）

- 未开启时 `finish` 不创建文件。
- 开启时写入 `payload-YYYY-MM-DD.jsonl`，一行合法 JSON，含 `request_id`/`attempt`/`status`。
- 请求体超过 2MB → `truncated=true` 且 body 长度受限。
- SSE 单条超过 256KB → 单条截断；累计超过 20MB → 停止追加。
- 图片占位出现在最终 JSON 中。

### 6.4 配置与设置（`src/config/tests.rs`、`src/settings/service/tests.rs`）

- `Config::default()`：`log_level=="info"`、`log_payload==false`。
- `to_json/from_json` round-trip；旧 JSON 缺字段取默认；`log_payload: true` 可解析；非法 level 报错。
- `settings.json` 保存文本包含 `"log_level": "debug"`（冒号后空格）与 `"log_payload": true`。
- GET `/api/state` 含 `"log_level":"info"`、`"log_payload":false`、`"logging":{"dir":...}`。
- PUT `/api/config` 保存后 `ConfigChanged` 携带新字段。

### 6.5 网络日志与行为回归（`src/chat/client/tests/`）

- 现有 `run_stream` 测试全部保持通过（旧签名 wrapper）。
- 新增 `payload_capture_is_off_by_default`、`payload_capture_writes_redacted_request`（用临时 `LX_TUI_LOG_DIR` + 全局开关测试隔离）。
- 重试、取消、HTTP 错误、SSE 断流的现有断言不变。

### 6.6 TUI 与 CLI

- `cargo test tui::`：panic hook 改动不破坏恢复顺序测试。
- `src/cli/tests/config_regressions_tests.rs`：`lx settings show` JSON 含 `logging`；`lx settings set --log-level debug --log-payload true` 落盘；`--help` 含新参数。
- `cargo fmt --check`、`cargo clippy --all-targets -- -D warnings`、`cargo test`。

## 7. 风险与回滚

| 风险 | 影响 | 缓解 |
|---|---|---|
| 新增 `tracing-appender` | 依赖体积 + 编译时间 | 标准 crate，仅用于非阻塞按天写；理由写入 commit |
| 启动加载 settings 改变现有布局行为 | 用户下次启动布局可能与之前不同 | 这正是设置页承诺的效果；失败回退默认并 warn |
| `Config` 解析器不支持 bool | 设置页保存 422 | 两个解析器都加 `Bool`；PUT/GET 测试覆盖 |
| 多进程共写同一按天文件 | 行可能交错 | 每行带 `role/pid/session/request_id`；非阻塞写入 |
| payload 含敏感内容 | 泄露 prompt/代码 | 默认关闭；设置页警告；header/URL/图片脱敏；7 天清理 |
| panic hook 重复安装 | 同一次 panic 记两条 | 可接受；不改既有安装点 |
| 热更新与 `LX_TUI_LOG` 冲突 | 设置页改了不生效 | 明确 env 优先；设置页保存仍落盘，下次生效 |

回滚：删除 `src/logging/` 与 `src/platform/log.rs`，恢复 `main.rs` 的旧 `init_tracing`、移除 `Config` 两字段与设置页字段即可；无 DB schema 变更，无数据迁移。

## 8. 验收标准

1. `cargo fmt --check` 通过。
2. `cargo clippy --all-targets -- -D warnings` 通过。
3. `cargo test` 全量通过。
4. 手工验证（用户执行 `cargo run`）：
   - 启动后 `~/.lx-t-dev/logs/lx-tui.log.YYYY-MM-DD` 存在且追加；
   - 设置页 Logging 区域显示路径，`Copy path` 可用，`Open folder` 能打开目录；
   - 切成 `debug` 后发起一轮对话，日志出现 `agent turn started` / `agent request built` / `agent request finished`；
   - 开启 payload 后出现 `payload-YYYY-MM-DD.jsonl`，其中无 `Authorization` / `x-api-key` / `?key=`，图片为占位；
   - `lx exec` 产生的日志同文件、`role=cli`。
5. 重启后日志等级仍生效；设置页改级别无需重启。
6. panic 日志包含 message 与 `file:line:column`，终端恢复顺序不变。
