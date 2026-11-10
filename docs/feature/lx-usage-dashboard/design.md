# lx 模型用量统计看板（Usage Dashboard）— 设计文档

> 状态：已在 `.worktrees/lx-usage-dashboard`（分支 `feat/lx-usage-dashboard`）实现并通过验证，待合并至 `dev`
> 任务清单：[task.md](task.md)
> 参考实现：`/Users/yonah/projects/agent/lx-agent/src/renderer/src/features/usage/`、`/Users/yonah/projects/agent/lx-agent/src/main/services/usageLogService.ts`、`/Users/yonah/projects/agent/lx-agent/src/main/agent/usageRecorder.ts`、`src/settings/service.rs`、`src/ui/controls.rs`
> 日期：2026-11-10
> 执行位置：`.worktrees/lx-usage-dashboard`（分支 `feat/lx-usage-dashboard`）

---

## 0. 背景与目标

`lx-tui` 目前只在 `agent_message` 表中记录每轮对话最后一步的 Token 快照，在 `agent_session` 表中累加会话级 `total_input_tokens` / `total_output_tokens`，存在以下能力缺口：

1. **缺乏单次请求级流水账**：一轮包含多步工具调用的对话会触发多次模型 HTTP 请求，`agent_message` 仅覆盖最后一次响应的 `usage`，中间步骤的单次请求明细、耗时（`duration_ms`）、失败/中断状态丢失。
2. **后台辅助模型调用未纳入统计**：上下文压缩（手动 `/compact` 与子代理自动压缩 `maybe_auto_compact`）、会话标题生成（`title`）、轮次摘要（`recap`）均直接调用 `run_stream_with_log`，其 Token 消耗未落入任何统计表。
3. **缺少可视化统计看板与成本核算**：无法按时间范围（今日 / 7天 / 30天 / 全部 / 自定义日期）、供应商（Provider）、模型（Model）、工作区（Workspace）、会话（Session）查看 Token 构成、花费趋势、请求频次、缓存命中率及请求明细。
4. **缺少模型单价（Pricing）自动提取与配置入口**：`ModelConfig.pricing` 虽已在配置结构体中预留，但 `models.dev` 目录同步时未提取 `cost` 字段，设置页 Models 视图也未提供单价编辑表单。

### 目标

1. **侧栏快捷入口**：在左侧容器底边框 `[SET]` 按钮右侧新增 `[USE]` 按钮（1 列间距，Hover 显示 `open usage`），点击后通过系统默认浏览器打开本地统计看板页面。
2. **全链路请求采集与历史回填**：升级数据库至 `SCHEMA_VERSION = 26`，新增 `usage_log` 表，在唯一网络出口 `run_stream_with_log` 收尾处自动记录每次模型请求（覆盖 `chat`、`subagent`、`compaction`、`title`、`recap`），并在迁移时自动将存量 `agent_message` 历史用量幂等回填至 `usage_log`。
3. **打通模型单价（Pricing）与成本（USD）计算**：从 `models.dev` 目录自动提取模型官方单价（Input / Output / Cache Read / Cache Write，USD / 1M tokens），在 `settings.html` 的模型卡片中支持手动查看与覆盖单价；写入 `usage_log` 时快照计算美元成本，无单价模型优雅降级为 `null`（UI 显示 `--`，分布图退回按 Token 排序）。
4. **独立 HTML 统计看板**：复用 `SettingsService` 本地回环 HTTP 服务，提供 `GET /usage?token=<token>` 页面路由及 `/api/usage/*` JSON 接口；前端基于 `assets/usage.{html,css,js}` + `assets/usage_charts.js` 实现零外部依赖、原生 SVG 交互图表、与 `settings.css` 黑白灰工业风完全对齐的统计看板。

---

## 1. 现状核实（写设计前已逐项确认）

| 事实 | 代码位置 | 对本设计的影响 |
|---|---|---|
| `[SET]` 位于左侧栏底边框左端（`x = area.x + 1`，宽 5 列），`SETTINGS_BUTTON_MIN_WIDTH = 11` | `src/ui/controls.rs:345-395`、`src/ui/text/workspace.rs:525` | `[USE]` 固定放在 `[SET]` 右侧 1 列间距处（`x = area.x + 7`，宽 5 列），`USAGE_BUTTON_MIN_WIDTH = 17`；不修改 `SETTINGS_BUTTON_MIN_WIDTH` |
| `ui/tests.rs` 有两条锁死“`[SET]` 右侧无按钮、无 tooltip”的回归测试 | `src/ui/tests.rs:3134-3205` | 新增 `[USE]` 后必须同步更新这两条测试，断言 `[SET]` 与 `[USE]` 几何及 `[USE]` 右侧至折叠按钮间为纯边框线 |
| 鼠标点击 `[SET]` 仅置位 `state.settings_open_requested = true`，由 `event/mod.rs` 后台调用 `open_browser` | `src/event/mouse/press.rs:207-224`、`src/event/mod.rs:341-353` | 严格遵循 `ui/` 纯渲染与 `app/` 纯状态边界：新增 `state.usage_open_requested: bool`，在 `event/mod.rs` 中调用 `SettingsService::usage_url` 打开浏览器 |
| `SettingsService` 监听 `127.0.0.1:0`，静态资源白名单免 token，页面与 `/api/*` 校验 capability token | `src/settings/service.rs:271-311`、`src/settings/service/handlers.rs:206-259`、`src/settings/service/page.rs:9-47` | 直接在 `page::asset` 注册 `/usage.css`、`/usage.js`、`/usage_charts.js`，在 `handlers::route` 增加 `("GET", "/usage")`（需 token）和 `/api/usage/*` 分支 |
| 测试 `page_requires_token_and_serves_self_contained_assets` 禁止任何资源出现 `http://` / `https://` 与 `.innerHTML` | `src/settings/service/tests.rs:113-232` | 注意：`createElementNS` 的 SVG 命名空间字符串含 `http://www.w3.org/2000/svg`！测试检查或前端实现需注意：测试若扫描静态资源中的 `http://`，可拼接 `'http:' + '//www.w3.org/2000/svg'` 并在测试中将 `usage.*` 纳入自包含与零 `innerHTML` 校验 |
| 当前 `SCHEMA_VERSION = 25`，全部业务表时间戳均使用毫秒整数 `*_at_ms INTEGER NOT NULL` | `src/db/mod.rs:30`、`src/db/migration.rs`、`src/db/migration/schema.rs` | 新增迁移 `SCHEMA_VERSION = 26`，`usage_log.created_at_ms` 统一采用毫秒级 Unix 时间戳 |
| 所有模型流式请求统一走 `chat::client::run_stream_with_log`，自带 `StreamLog`、`ChatRequest`、`started: Instant` 与最终 `StreamOutcome` | `src/chat/client/mod.rs:154-308` | 在 `run_stream_with_log` 结束处统一挂载异步/后台用量记录器，一次拦截即可覆盖 TUI 主对话、子代理、自动压缩、手动压缩、标题生成、Recap 及 CLI `exec`/`compact` |
| 子代理自动压缩 `maybe_auto_compact` 复用父闭包的 `stream_one`，其 `StreamLog.kind` 仍为 `"subagent"`，但 `ChatRequest` 由 `summary_request_from_transcript` 构造 | `src/chat/agent_loop/auto_compact.rs:199-208`、`src/chat/compact.rs:163-196` | 为 `RequestPurpose` 增加区分或根据 `request.system_prompt` / `RequestPurpose` 识别自动压缩请求，确保自动压缩在 `usage_log.purpose` 中准确归类为 `"compaction"` |
| `ModelConfig` 已含 `pub pricing: Option<serde_json::Value>`，但 `catalog_refresh.rs` 写死 `pricing: None` | `src/config/ai/mod.rs:180-182`、`src/config/ai/catalog_refresh.rs:158` | 在 `parse_models_dev_catalog` 中解析 `model.get("cost")` 或 `model.get("pricing")` 为 `ModelPricing`，并在 `settings/service/models/mod.rs` 支持三态读写 |
| `assets/settings_models.js` 当前已达 990 行，逼近单文件 1000 行红线 | `assets/settings_models.js:1-990` | 严禁直接往 `settings_models.js` 堆代码超 1000 行；将模型卡片/思考变体/模态/单价渲染拆分为 `assets/settings_models_card.js`（各约 500 行），并在 `settings.html` 与 `page::asset` 登记 |

---

## 2. 已确认决策（Grill Me 收口）

| 编号 | 决策维度 | 已确认结论 |
|---|---|---|
| **D1** | 数据源与表结构 | 升级数据库至 `SCHEMA_VERSION = 26`，新建独立 `usage_log` 表；每次底层模型 HTTP 请求完成时写入单次请求明细（覆盖 `chat`、`subagent`、`compaction`、`title`、`recap`）；v26 迁移时将存量 `agent_message` 历史助手消息幂等回填至 `usage_log`。 |
| **D2** | 侧栏入口按钮 | 在左侧容器底边框 `[SET]` 右侧新增 `[USE]` 按钮（间距 1 列边框线 `─`，位于 `view.sidebar.x + 7`，宽 5 列，纯 ASCII + `style::accent()` 逐格绘制，最小侧栏宽度门槛 `17` 列）；Hover Tooltip 为 `open usage`；点击后在系统默认浏览器打开 `/usage?token=<token>`。 |
| **D3** | 服务承载与资源 | 复用现有 `SettingsService` 本地回环 HTTP 服务（不另起端口）；新增页面路由 `GET /usage?token=<token>` 与 API 路由 `/api/usage/*`；前端资源拆分为 `assets/usage.html`、`assets/usage.css`、`assets/usage.js`、`assets/usage_charts.js`，与 `settings.html` 保持页面独立。 |
| **D4** | 成本计算与单价 | 完整支持美元成本（Cost USD）统计；`catalog_refresh.rs` 从 `models.dev` 自动提取 `cost.{input,output,cache_read,cache_write}` 填入 `ModelConfig.pricing`；`settings.html` Models 页支持手动查看/修改 4 项单价（USD / 1M tokens）；未配置单价时成本列存 `NULL`，界面显示 `--`，模型/供应商分布图自动退回按 Token 数量绘图。 |
| **D5** | 看板布局与风格 | 完整对齐 `lx-agent` 的看板结构（顶部筛选栏 + 汇总卡片［含 Tokens Processed、Fresh Input、Output、Cache Write、Cache Read、Cache Hit Rate、Requests、Total Cost］+ 5 张图表 + 底部 3 个 Tab 表格［Request Logs / Model Stats / Provider Stats］）；采用原生 SVG 实现交互图表与悬浮 Tooltip；视觉完全复用 `settings.css` 的 `:root` 黑白灰工业风暗色令牌。 |

---

## 3. 总体架构与数据流

```text
┌────────────────────────────────────────────────────────────────────────────┐
│ 1. 模型调用采集链路（TUI & CLI 全入口）                                     │
│                                                                            │
│   chat / subagent / compact / auto_compact / title / recap / exec          │
│     └─► chat::client::run_stream_with_log(transport, url, key, req, log)   │
│           ├─ 记录 started = Instant::now()                                  │
│           ├─ 执行 SSE 流并收集 deltas + StopReason                          │
│           ├─ 计算 duration_ms、提取 step_usage(&deltas)                     │
│           └─► usage_recorder::record_stream_usage(...)                     │
│                 ├─ 推导 purpose (chat|subagent|compaction|title|recap)      │
│                 ├─ 推导 status (success|error|aborted)                      │
│                 ├─ 查询模型单价 ModelPricing (ai.json -> connect -> catalog)│
│                 ├─ 计算快照成本 (input/output/cacheRead/cacheWrite/total)   │
│                 └─ 后台写入 SQLite `usage_log`（异常只 warn，绝不阻断主流）  │
└────────────────────────────────────────────────────────────────────────────┘

┌────────────────────────────────────────────────────────────────────────────┐
│ 2. TUI 侧栏按钮与浏览器打开链路                                             │
│                                                                            │
│   ui::controls::render_usage_button  ──► 绘制 `╰[SET]─[USE]────────[◀]╯`   │
│   ui::tooltip::targets               ──► Hover 命中 `SidebarUsage`          │
│                                          显示 Tooltip `"open usage"`       │
│   event::mouse::press                ──► 点击命中 `usage_button_at`         │
│                                          置位 `state.usage_open_requested` │
│   event::run_loop                    ──► 消费 `usage_open_requested`        │
│                                          spawn_blocking 打开                │
│                                          `http://127.0.0.1:<port>/usage?token=<token>` │
└────────────────────────────────────────────────────────────────────────────┘

┌────────────────────────────────────────────────────────────────────────────┐
│ 3. 本地回环 Web 服务与统计看板渲染链路                                      │
│                                                                            │
│   Browser GET /usage?token=<token>   ──► handlers::route 校验 token        │
│     ├─ 返回 assets/usage.html                                              │
│     ├─ 加载 /usage.css、/usage_charts.js、/usage.js（静态白名单免 token）   │
│     └─► usage.js 发起并发请求（或组合请求）：                               │
│           ├─ GET /api/usage/overview?token=...&startTime=...&endTime=...   │
│           │    └─ 返回 { summary, series, models, providers, filters }     │
│           └─ GET /api/usage/logs?token=...&page=1&pageSize=50&...          │
│                └─ 返回 { rows, total, page, pageSize }                     │
│         │                                                                  │
│         ▼                                                                  │
│   usage.js + usage_charts.js（纯 DOM API + 原生 SVG，零 innerHTML）         │
│     ├─ 渲染筛选栏（Today/7d/30d/All/Custom + Provider/Model/Workspace/Session/AutoRefresh）│
│     ├─ 渲染汇总卡（Tokens Processed / Fresh Input / Output / Cache Read /  │
│     │              Cache Write / Cache Hit Rate / Requests / Total Cost）  │
│     ├─ 渲染 5 张 SVG 图表 + 悬浮 Tooltip：                                 │
│     │    1. Token & Cost Trend（4层堆叠面积 + 右轴 Cost 虚线折线）          │
│     │    2. Requests Trend（柱状图）                                        │
│     │    3. Model Distribution（Top 8 横向条形图，Cost 优先/Token 兜底）    │
│     │    4. Provider Distribution（Top 8 环形图 Donut）                     │
│     │    5. Token Composition（4 色构成实心/环形饼图）                      │
│     └─ 渲染底部 3 个 Tab 表格（Request Logs 分页 / Model Stats / Provider Stats）│
└────────────────────────────────────────────────────────────────────────────┘
```

---

## 4. 详细设计

### 4.1 模块与文件清单

```text
新增文件：
src/db/usage/mod.rs                      # usage_log 数据模型、写入与 6 类聚合查询（~450 行）
src/db/usage/tests.rs                    # usage_log 写入、筛选、按时/按日聚合、分页与迁移回填测试（~450 行）
src/chat/usage_recorder.rs               # 从流结果提取 Usage、解析单价、异步落库（~220 行）
src/chat/usage_recorder/tests.rs         # 单价计算、purpose 映射、落库隔离测试（~200 行）
src/settings/service/usage.rs            # /api/usage/* 路由、参数校验与 JSON 序列化（~320 行）
src/settings/service/usage/tests.rs      # /usage 页面鉴权、静态资源、API 聚合与分页 HTTP 测试（~350 行）
assets/settings_models_card.js           # 从 settings_models.js 拆出的模型卡片/变体/模态/单价编辑组件（~420 行）
assets/usage.html                        # 统计看板页面骨架（~180 行）
assets/usage.css                         # 统计看板样式（对齐 settings.css 黑白灰暗色令牌，~340 行）
assets/usage.js                          # 统计看板状态、筛选器、汇总卡、表格、分页与自动刷新逻辑（~580 行）
assets/usage_charts.js                   # 原生 SVG 图表渲染引擎（堆叠面积+双轴折线、柱状、横向条形、饼图/环形图、Tooltip，~560 行）

修改文件：
src/db/mod.rs                            # SCHEMA_VERSION = 26；导出 usage 模块；migration_report 纳入 usage_log
src/db/migration.rs                      # 新增 version < 26 迁移分支（建表 + 历史 agent_message 回填）
src/db/migration/schema.rs               # 新增 SCHEMA_V26 常量
src/config/ai/mod.rs                     # 新增 ModelPricing 结构体、成本计算辅助与单价查询方法
src/config/ai/catalog_refresh.rs         # parse_models_dev_catalog 提取 model.cost / model.pricing
src/config/ai/presets.rs                 # merge_discovered_models_for_preset 保留或继承模型 pricing
src/chat/mod.rs                          # 注册 pub mod usage_recorder;
src/chat/events.rs                       # RequestPurpose 补充 Compaction / Title / Recap 或辅助判定
src/chat/compact.rs                      # summary_request / summary_request_from_transcript 标记 Compaction 用途
src/chat/client/mod.rs                   # run_stream_with_log 收尾处调用 usage_recorder
src/ui/text/workspace.rs                 # 新增 USAGE_LABEL = "[USE]"
src/ui/text.rs                           # SidebarUsage tooltip 文案 "open usage"
src/ui/controls.rs                       # 新增 usage_button / usage_button_at / render_usage_button
src/ui/mod.rs                            # 侧栏渲染追加 controls::render_usage_button
src/app/tooltip.rs                       # TooltipTarget 新增 SidebarUsage
src/ui/tooltip.rs                        # targets 注册 SidebarUsage
src/app/state.rs                         # AppState 新增 usage_open_requested: bool
src/event/mouse/press.rs                 # 鼠标左键命中 usage_button_at 置位 usage_open_requested
src/event/mod.rs                         # 消费 usage_open_requested 并调用 open_browser(&service.usage_url())
src/settings/service.rs                  # SettingsService 新增 usage_url()；ServiceState 解析 db_path()
src/settings/service/page.rs             # 新增 usage_html()；asset() 登记 /usage.{css,js}、/usage_charts.js、/settings_models_card.js
src/settings/service/handlers.rs         # route() 新增 ("GET", "/usage") 与 /api/usage/* 分发
src/settings/service/models/mod.rs       # GET 输出 pricing；PUT 支持 pricing 三态校验与合并
assets/settings.html                     # 引入 /settings_models_card.js
assets/settings_models.js                # 拆出卡片子函数至 settings_models_card.js，纳入 pricing 脏状态检测與提交
docs/standards/tui-design-requirements.md # 第 21 行补充 [SET] 右侧 [USE] 按钮规范描述
docs/standards/html-assets.md            # 更新 assets/ 目录结构说明
docs/standards/project-directory-structure.md # 更新目录树说明
```

所有生产代码文件均严格控制在 300~600 行以内，远低于 1000 行上限。

---

### 4.2 数据库 Schema 与迁移（`SCHEMA_VERSION = 26`）

#### 4.2.1 `src/db/migration/schema.rs` 新增 `SCHEMA_V26`

```rust
/// v26：新增模型调用统计流水表 `usage_log` 及历史 `agent_message` 回填。
///
/// - `session_id` / `workspace_id` 为写入时刻的快照文本，故意不建外键约束：
///   删除会话或工作区后，历史账单与 Token 消耗统计仍须保留（与 `lx-agent` 一致）。
/// - `input_tokens` 为完整输入 Token（含 `cache_read_tokens` + `cache_write_tokens`），
///   前端计算 `fresh_input = max(0, input_tokens - cache_read_tokens - cache_write_tokens)`。
pub(super) const SCHEMA_V26: &str = r#"
CREATE TABLE IF NOT EXISTS usage_log (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    external_id TEXT NOT NULL UNIQUE,
    session_id TEXT,
    workspace_id TEXT,
    purpose TEXT NOT NULL CHECK (purpose IN ('chat', 'subagent', 'compaction', 'title', 'recap')),
    provider TEXT NOT NULL,
    model TEXT NOT NULL,
    input_tokens INTEGER NOT NULL DEFAULT 0 CHECK (input_tokens >= 0),
    output_tokens INTEGER NOT NULL DEFAULT 0 CHECK (output_tokens >= 0),
    cache_read_tokens INTEGER NOT NULL DEFAULT 0 CHECK (cache_read_tokens >= 0),
    cache_write_tokens INTEGER NOT NULL DEFAULT 0 CHECK (cache_write_tokens >= 0),
    input_cost_usd REAL,
    output_cost_usd REAL,
    cache_read_cost_usd REAL,
    cache_write_cost_usd REAL,
    total_cost_usd REAL,
    duration_ms INTEGER CHECK (duration_ms IS NULL OR duration_ms >= 0),
    status TEXT NOT NULL DEFAULT 'success' CHECK (status IN ('success', 'error', 'aborted')),
    error_message TEXT,
    created_at_ms INTEGER NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_usage_log_created_at
    ON usage_log(created_at_ms DESC, id DESC);
CREATE INDEX IF NOT EXISTS idx_usage_log_model
    ON usage_log(model);
CREATE INDEX IF NOT EXISTS idx_usage_log_provider
    ON usage_log(provider);
CREATE INDEX IF NOT EXISTS idx_usage_log_workspace
    ON usage_log(workspace_id);
CREATE INDEX IF NOT EXISTS idx_usage_log_session
    ON usage_log(session_id);

INSERT OR IGNORE INTO usage_log (
    external_id,
    session_id,
    workspace_id,
    purpose,
    provider,
    model,
    input_tokens,
    output_tokens,
    cache_read_tokens,
    cache_write_tokens,
    duration_ms,
    status,
    error_message,
    created_at_ms
)
SELECT
    'ulog-backfill-' || m.external_id,
    m.session_id,
    s.workspace_id,
    CASE WHEN s.source = 'subagent' THEN 'subagent' ELSE 'chat' END,
    COALESCE(NULLIF(m.model_provider, ''), COALESCE(NULLIF(s.model_provider, ''), 'unknown')),
    COALESCE(NULLIF(m.model_id, ''), COALESCE(NULLIF(s.model_id, ''), 'unknown')),
    COALESCE(m.input_tokens, 0),
    COALESCE(m.output_tokens, 0),
    COALESCE(m.cache_read_tokens, 0),
    COALESCE(m.cache_write_tokens, 0),
    CASE
        WHEN m.completed_at_ms IS NOT NULL AND m.completed_at_ms >= m.created_at_ms
        THEN m.completed_at_ms - m.created_at_ms
        ELSE NULL
    END,
    CASE m.status
        WHEN 'complete' THEN 'success'
        WHEN 'interrupted' THEN 'aborted'
        WHEN 'failed' THEN 'error'
        ELSE 'success'
    END,
    m.error_code,
    COALESCE(m.completed_at_ms, m.created_at_ms)
FROM agent_message m
LEFT JOIN agent_session s ON s.external_id = m.session_id
WHERE m.role = 'assistant'
  AND m.kind = 'chat'
  AND m.status IN ('complete', 'interrupted', 'failed')
  AND (
      m.input_tokens IS NOT NULL
      OR m.output_tokens IS NOT NULL
      OR m.cache_read_tokens IS NOT NULL
      OR m.cache_write_tokens IS NOT NULL
      OR m.status != 'complete'
  );
"#;
```

#### 4.2.2 `src/db/migration.rs` 迁移分支

```rust
if version < 26 {
    self.conn.execute_batch(schema::SCHEMA_V26)?;
    self.conn.pragma_update(None, "user_version", 26)?;
}
```

---

### 4.3 模型单价（Pricing）与成本计算（`src/config/ai/`）

#### 4.3.1 `ModelPricing` 与成本计算公式（`src/config/ai/mod.rs`）

对齐 `lx-agent` (`src/shared/contracts/usage.ts:168-196`)：

```rust
/// 模型 Token 单价（单位：USD / 1,000,000 tokens）。
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelPricing {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none", alias = "cache_read")]
    pub cache_read: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none", alias = "cache_write")]
    pub cache_write: Option<f64>,
}

impl ModelPricing {
    /// 是否至少配置了一项有效单价。
    pub fn has_any(&self) -> bool {
        self.input.is_some()
            || self.output.is_some()
            || self.cache_read.is_some()
            || self.cache_write.is_some()
    }

    /// 从 JSON Value（支持 camelCase 与 snake_case）解析单价。
    pub fn from_json_value(value: &serde_json::Value) -> Option<Self> {
        let obj = value.as_object()?;
        let non_neg = |key1: &str, key2: &str| -> Option<f64> {
            obj.get(key1)
                .or_else(|| obj.get(key2))
                .and_then(serde_json::Value::as_f64)
                .filter(|v| v.is_finite() && *v >= 0.0)
        };
        let pricing = Self {
            input: non_neg("input", "input"),
            output: non_neg("output", "output"),
            cache_read: non_neg("cacheRead", "cache_read"),
            cache_write: non_neg("cacheWrite", "cache_write"),
        };
        pricing.has_any().then_some(pricing)
    }

    /// 转为标准 camelCase JSON Value。
    pub fn to_json_value(self) -> Option<serde_json::Value> {
        if !self.has_any() {
            return None;
        }
        serde_json::to_value(self).ok()
    }
}
```

成本计算逻辑（位于 `src/db/usage/mod.rs`）：

```rust
/// 单次请求的美元成本分解。
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UsageRates {
    pub input_cost_usd: Option<f64>,
    pub output_cost_usd: Option<f64>,
    pub cache_read_cost_usd: Option<f64>,
    pub cache_write_cost_usd: Option<f64>,
    pub total_cost_usd: Option<f64>,
}

impl UsageRates {
    pub fn compute(
        input_tokens: i64,
        output_tokens: i64,
        cache_read_tokens: i64,
        cache_write_tokens: i64,
        pricing: Option<crate::config::ai::ModelPricing>,
    ) -> Self {
        let Some(pricing) = pricing.filter(|p| p.has_any()) else {
            return Self::default();
        };
        let input = input_tokens.max(0) as f64;
        let output = output_tokens.max(0) as f64;
        let cache_read = cache_read_tokens.max(0) as f64;
        let cache_write = cache_write_tokens.max(0) as f64;
        let fresh_input = (input - cache_read - cache_write).max(0.0);

        let per_million = |tokens: f64, rate: Option<f64>| -> f64 {
            tokens * rate.unwrap_or(0.0).max(0.0) / 1_000_000.0
        };
        let input_cost = per_million(fresh_input, pricing.input);
        let output_cost = per_million(output, pricing.output);
        let cache_read_cost = per_million(cache_read, pricing.cache_read);
        let cache_write_cost = per_million(cache_write, pricing.cache_write);
        let total_cost = input_cost + output_cost + cache_read_cost + cache_write_cost;

        Self {
            input_cost_usd: Some(input_cost),
            output_cost_usd: Some(output_cost),
            cache_read_cost_usd: Some(cache_read_cost),
            cache_write_cost_usd: Some(cache_write_cost),
            total_cost_usd: Some(total_cost),
        }
    }
}
```

#### 4.3.2 `src/config/ai/catalog_refresh.rs` 自动提取单价

在 `parse_models_dev_catalog` 中将原本的 `pricing: None` 替换为：

```rust
pricing: parse_pricing(model.get("cost").or_else(|| model.get("pricing"))),
```

其中：

```rust
fn parse_pricing(value: Option<&Value>) -> Option<Value> {
    value
        .and_then(super::ModelPricing::from_json_value)
        .and_then(super::ModelPricing::to_json_value)
}
```

并在 `AiSettings::resolve_model_pricing(&self, provider_id: &str, model_id: &str) -> Option<ModelPricing>` 中按优先级查找：
1. 当前 `AiSettings`（已合并 `ai.json` 与 `connect.json`）对应 provider 下的 `model.pricing`；
2. 进程内 `presets::catalog()` 缓存中对应 provider（或同名 model_id）的 `model.pricing`。

---

### 4.4 数据库聚合查询层（`src/db/usage/mod.rs`）

对齐 `lx-agent` 的 `usageLogService.ts`，所有统计数据在查询时通过 SQL 直接聚合：

#### 4.4.1 查询条件构建 `UsageQuery`

```rust
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct UsageQuery {
    pub start_time_ms: Option<i64>,
    pub end_time_ms: Option<i64>,
    pub provider: Option<String>,
    pub model: Option<String>,
    pub workspace_id: Option<String>,
    pub session_id: Option<String>,
}
```

`build_where(query: &UsageQuery, alias: Option<&str>) -> (String, Vec< rusqlite::types::Value >)` 动态生成参数化 `WHERE` 子句，杜绝 SQL 注入。

#### 4.4.2 6 个核心查询方法

1. **`record_usage_log(&self, entry: &UsageLogInsert) -> Result<()>`**
   - 若 `entry.workspace_id` 为 `None` 且 `entry.session_id` 为 `Some(sid)`，先执行：
     `SELECT workspace_id FROM agent_session WHERE external_id = ?1` 自动补齐 `workspace_id`。
   - 执行 `INSERT OR IGNORE INTO usage_log (...) VALUES (...)`。

2. **`usage_summary(&self, query: &UsageQuery) -> Result<UsageSummary>`**
   ```sql
   SELECT
       COUNT(*) AS request_count,
       COALESCE(SUM(CASE WHEN status = 'success' THEN 1 ELSE 0 END), 0) AS success_count,
       COALESCE(SUM(CASE WHEN status = 'error' THEN 1 ELSE 0 END), 0) AS error_count,
       COALESCE(SUM(CASE WHEN status = 'aborted' THEN 1 ELSE 0 END), 0) AS aborted_count,
       COALESCE(SUM(input_tokens), 0) AS input_tokens,
       COALESCE(SUM(output_tokens), 0) AS output_tokens,
       COALESCE(SUM(cache_read_tokens), 0) AS cache_read_tokens,
       COALESCE(SUM(cache_write_tokens), 0) AS cache_write_tokens,
       COALESCE(SUM(CASE WHEN total_cost_usd IS NOT NULL THEN 1 ELSE 0 END), 0) AS priced_count,
       SUM(total_cost_usd) AS total_cost_usd,
       AVG(duration_ms) AS avg_duration_ms
   FROM usage_log
   WHERE ...
   ```
   - `total_tokens = input_tokens + output_tokens`
   - `success_rate = if request_count > 0 { Some(success_count as f64 / request_count as f64 * 100.0) } else { None }`
   - `total_cost_usd = if priced_count > 0 { Some(total_cost_usd.unwrap_or(0.0)) } else { None }`

3. **`usage_series(&self, query: &UsageQuery, granularity: UsageGranularity) -> Result<Vec<UsageDailyPoint>>`**
   - 小时粒度（`UsageGranularity::Hour`）：
     `strftime('%Y-%m-%d %H:00', created_at_ms / 1000, 'unixepoch', 'localtime')`
   - 天粒度（`UsageGranularity::Day`）：
     `date(created_at_ms / 1000, 'unixepoch', 'localtime')`
   - `GROUP BY bucket ORDER BY bucket ASC`，聚合每桶的 `request_count`、`input_tokens`、`output_tokens`、`cache_read_tokens`、`cache_write_tokens`、`total_cost_usd`。

4. **`usage_model_stats(&self, query: &UsageQuery) -> Result<Vec<UsageModelStats>>`**
   - `GROUP BY model`
   - 排序：`ORDER BY (SUM(total_cost_usd) IS NULL) ASC, SUM(total_cost_usd) DESC, COUNT(*) DESC, model ASC`
   - 计算 `avg_cost_per_request_usd = total_cost / priced_count`（当 `priced_count > 0` 时）。

5. **`usage_provider_stats(&self, query: &UsageQuery) -> Result<Vec<UsageProviderStats>>`**
   - `GROUP BY provider`
   - 排序：`ORDER BY COUNT(*) DESC, provider ASC`
   - 计算 `total_tokens = SUM(input_tokens + output_tokens)`、`success_rate`、`avg_duration_ms`。

6. **`usage_filter_options(&self, query: &UsageQuery) -> Result<UsageFilterOptions>`**
   - `providers`：排除 `query.provider` 自身维度后的 `SELECT DISTINCT provider FROM usage_log ... ORDER BY provider ASC`。
   - `models`：排除 `query.model` 自身维度后的 `SELECT DISTINCT model FROM usage_log ... ORDER BY model ASC`。
   - `workspaces`：排除 `query.workspace_id` 自身维度，左联 `workspace w ON w.external_id = ul.workspace_id`，返回 `{ id, name: COALESCE(NULLIF(w.name, ''), ul.workspace_id) }`。
   - `sessions`：仅受 `start_time_ms`、`end_time_ms`、`workspace_id` 约束（与 `lx-agent` 一致），左联 `agent_session s ON s.external_id = ul.session_id`，按 `MAX(ul.created_at_ms) DESC` 排序，标题为空时回退 `session_id` 前 8 位。

7. **`usage_logs(&self, query: &UsageQuery, page: usize, page_size: usize) -> Result<UsageLogPage>`**
   - `page = page.max(1)`，`page_size = page_size.clamp(1, 200)`（默认 50）。
   - `ORDER BY created_at_ms DESC, id DESC LIMIT ? OFFSET ?`。

---

### 4.5 模型请求拦截与自动记录（`src/chat/usage_recorder.rs` & `src/chat/client/mod.rs`）

#### 4.5.1 `StreamLog` 与用途推导

在 `src/chat/client/mod.rs` 的 `run_stream_with_log` 末尾（第 306 行 `StreamOutcome { deltas, stop }` 返回前）：

```rust
let elapsed_ms = started.elapsed().as_millis() as i64;
crate::chat::usage_recorder::record_stream_finish(
    log,
    &request,
    &deltas,
    &stop,
    elapsed_ms,
);
```

`src/chat/usage_recorder.rs` 实现要点：
- **测试隔离保护**：
  - 默认仅当 `log.kind != "unknown"` 时触发记录（单元测试若直接调 `run_stream` 使用默认的 `kind: "unknown"`，不会污染磁盘库）。
  - 提供线程局部/全局测试钩子（或在测试中通过 `LX_TUI_DB_PATH` / `test_home_override()` 指向临时目录），确保 `cargo test` 绝不写入开发者的 `~/.lx-t-dev/db/lx.db`。
- **用途（`UsagePurpose`）精确判定**：
  - 若 `log.kind == "compact"` 或 `matches!(request.purpose, RequestPurpose::Compaction)` → `UsagePurpose::Compaction`（覆盖主会话 `/compact`、CLI `session compact` 以及子代理 `maybe_auto_compact`）。
  - 若 `log.kind == "title"` → `UsagePurpose::Title`。
  - 若 `log.kind == "recap"` → `UsagePurpose::Recap`。
  - 若 `log.kind == "subagent"` 或 `matches!(request.purpose, RequestPurpose::Subagent(_))` → `UsagePurpose::Subagent`。
  - 其余（`log.kind == "chat"` / `"exec"`）→ `UsagePurpose::Chat`。
- **非阻塞落库**：
  - 提取 `Usage`（复用 `step_usage(&deltas)` 相同合并逻辑：同一次 HTTP 响应内后到的非 `None` 帧覆盖先到帧）。
  - 通过 `std::thread::spawn`（或 `tokio::task::spawn_blocking`）在后台打开 `Database::open(&db_path)`，读取模型单价计算 `UsageRates`，调用 `db.record_usage_log(&entry)`。任何 IO/SQLite 错误只打 `tracing::warn!(target: "lx::usage", ...)`，绝不影响主流程。

---

### 4.6 左侧栏 `[USE]` 按钮与鼠标交互（`src/ui/`、`src/app/`、`src/event/`）

#### 4.6.1 文案与几何（`src/ui/text/workspace.rs` & `src/ui/controls.rs`）

`src/ui/text/workspace.rs`：
```rust
/// 侧栏底边框用量统计按钮文案（纯 ASCII，位于 `[SET]` 右侧）。
pub const USAGE_LABEL: &str = "[USE]";
```

`src/ui/controls.rs`：
```rust
/// 用量按钮几何常量：位于 `[SET]` 右侧，中间留 1 列底边框线间距。
const USAGE_BUTTON_WIDTH: u16 = text::USAGE_LABEL.len() as u16;
const USAGE_BUTTON_GAP: u16 = 1;
/// 显示用量按钮所需的最小侧栏宽度：`[SET]` 最小宽度 (11) + 间距 (1) + `[USE]` 宽度 (5) = 17。
const USAGE_BUTTON_MIN_WIDTH: u16 =
    SETTINGS_BUTTON_MIN_WIDTH + USAGE_BUTTON_GAP + USAGE_BUTTON_WIDTH;

/// 侧栏底边框 `[USE]` 按钮几何：展开态、服务可用且宽度 >= 17 列时显示在 `[SET]` 右侧。
pub fn usage_button(view: &layout::ViewLayout, state: &AppState) -> Option<Rect> {
    let area = view.sidebar;
    if !state.settings_available
        || state.sidebar_collapsed
        || area.width < USAGE_BUTTON_MIN_WIDTH
        || area.height < 2
    {
        return None;
    }
    Some(Rect::new(
        area.x + SETTINGS_BUTTON_MARGIN + SETTINGS_BUTTON_WIDTH + USAGE_BUTTON_GAP,
        area.bottom().saturating_sub(1),
        USAGE_BUTTON_WIDTH,
        1,
    ))
}

/// 命中侧栏底边框 `[USE]` 按钮。
pub fn usage_button_at(
    view: &layout::ViewLayout,
    state: &AppState,
    column: u16,
    row: u16,
) -> bool {
    usage_button(view, state).is_some_and(|area| area.contains((column, row).into()))
}

/// 绘制侧栏底边框 `[USE]` 按钮：逐格 reset + 强调色。
pub(crate) fn render_usage_button(
    frame: &mut Frame<'_>,
    view: &layout::ViewLayout,
    state: &AppState,
) {
    let Some(area) = usage_button(view, state) else {
        return;
    };
    for (offset, symbol) in text::USAGE_LABEL.chars().enumerate() {
        let x = area.x + offset as u16;
        if let Some(cell) = frame.buffer_mut().cell_mut((x, area.y)) {
            cell.reset();
            cell.set_char(symbol);
            cell.set_style(style::accent());
        }
    }
}
```

#### 4.6.2 渲染、Tooltip 与点击打开浏览器

1. **`src/ui/mod.rs:91`**：
   ```rust
   let cursor = sidebar::render(frame, view.sidebar, state);
   controls::render_settings_button(frame, &view, state);
   controls::render_usage_button(frame, &view, state);
   cursor
   ```
2. **`src/app/tooltip.rs` & `src/ui/tooltip.rs` & `src/ui/text.rs`**：
   - `TooltipTarget` 枚举新增 `SidebarUsage`。
   - `ui::tooltip::targets` 新增：
     ```rust
     if let Some(area) = controls::usage_button(view, state) {
         out.push((area, TooltipTarget::SidebarUsage));
     }
     ```
   - `ui::text::tooltip_text(TooltipTarget::SidebarUsage)` 返回 `"open usage"`。
3. **`src/app/state.rs` & `src/event/mouse/press.rs` & `src/event/mod.rs`**：
   - `AppState` 新增 `pub usage_open_requested: bool`。
   - `press.rs` 在 `ui::settings_button_at` 分支后紧接判断 `ui::usage_button_at`，命中时设 `state.usage_open_requested = true; *dirty = true; return;`。
   - `event/mod.rs` 在消费 `settings_open_requested` 后紧接消费 `usage_open_requested`，调用 `SettingsService::usage_url` 并在 `spawn_blocking` 中通过 `crate::platform::open_browser(&url)` 打开浏览器。

---

### 4.7 本地 HTTP 路由与 JSON API 契约（`src/settings/service/`）

#### 4.7.1 `SettingsService::usage_url` 与 `ServiceState::db_path`

`src/settings/service.rs`：
```rust
/// 浏览器访问用量统计页 URL：`http://127.0.0.1:<port>/usage?token=<token>`。
pub fn usage_url(&self) -> String {
    format!("http://127.0.0.1:{}/usage?token={}", self.port, self.token())
}
```

`ServiceState` 新增 `pub fn db_path(&self) -> PathBuf`：
- 若 `self.user_root` 为 `Some(root)`（测试隔离模式），返回 `root.join("db").join("lx.db")`；
- 否则调用 `crate::db::default_db_path()`。

#### 4.7.2 页面与静态资源路由（`src/settings/service/page.rs` & `handlers.rs`）

- `page::usage_html()`：`include_str!("../../../assets/usage.html")`
- `page::asset(path)` 新增：
  - `"/settings_models_card.js"` → `("text/javascript; charset=utf-8", include_str!("../../../assets/settings_models_card.js"))`
  - `"/usage.css"` → `("text/css; charset=utf-8", include_str!("../../../assets/usage.css"))`
  - `"/usage.js"` → `("text/javascript; charset=utf-8", include_str!("../../../assets/usage.js"))`
  - `"/usage_charts.js"` → `("text/javascript; charset=utf-8", include_str!("../../../assets/usage_charts.js"))`
- `handlers::route` 新增：
  - `("GET", "/usage")` → 返回 `page::usage_html()`（`text/html; charset=utf-8`，需通过 `token_ok` 校验）
  - `(_, path) if path.starts_with("/api/usage")` → 转发至 `super::usage::route(request, state)`

#### 4.7.3 `/api/usage/*` 接口定义（`src/settings/service/usage.rs`）

支持以下 GET 端点（共用查询参数：`startTime`、`endTime`、`provider`、`model`、`workspaceId`、`sessionId`、`granularity=hour|day`、`page`、`pageSize`）：

1. **`GET /api/usage/overview`**（看板聚合快照，一次请求返回全部卡片、图表与筛选项，减少回环请求开销）：
   ```json
   {
     "summary": {
       "requestCount": 42,
       "successCount": 40,
       "errorCount": 1,
       "abortedCount": 1,
       "inputTokens": 128000,
       "outputTokens": 14500,
       "cacheReadTokens": 96000,
       "cacheWriteTokens": 8000,
       "totalTokens": 142500,
       "pricedRequestCount": 42,
       "totalCostUsd": 0.3412,
       "successRate": 95.238,
       "avgDurationMs": 3420.5
     },
     "daily": [
       {
         "date": "2026-11-10 14:00",
         "requestCount": 8,
         "inputTokens": 24000,
         "outputTokens": 3200,
         "cacheReadTokens": 18000,
         "cacheWriteTokens": 2000,
         "totalCostUsd": 0.068
       }
     ],
     "models": [
       {
         "model": "claude-sonnet-4-5",
         "requestCount": 30,
         "inputTokens": 100000,
         "outputTokens": 12000,
         "cacheReadTokens": 80000,
         "cacheWriteTokens": 6000,
         "totalTokens": 112000,
         "totalCostUsd": 0.285,
         "avgCostPerRequestUsd": 0.0095
       }
     ],
     "providers": [
       {
         "provider": "anthropic",
         "requestCount": 30,
         "totalTokens": 112000,
         "totalCostUsd": 0.285,
         "successRate": 96.67,
         "avgDurationMs": 3800.0
       }
     ],
     "filters": {
       "providers": ["anthropic", "openai"],
       "models": ["claude-sonnet-4-5", "gpt-5"],
       "workspaces": [{"id": "ws-1", "name": "lx-tui"}],
       "sessions": [{"id": "ses-1", "name": "Add usage dashboard"}]
     }
   }
   ```
2. **`GET /api/usage/logs`**（分页请求明细表，切换页码时单独请求）：
   ```json
   {
     "rows": [
       {
         "id": 101,
         "externalId": "ulog-...",
         "sessionId": "ses-1",
         "workspaceId": "ws-1",
         "purpose": "chat",
         "provider": "anthropic",
         "model": "claude-sonnet-4-5",
         "tokens": {
           "input": 4200,
           "output": 520,
           "cacheRead": 3100,
           "cacheWrite": 0
         },
         "rates": {
           "inputCostUsd": 0.0033,
           "outputCostUsd": 0.0078,
           "cacheReadCostUsd": 0.00093,
           "cacheWriteCostUsd": 0.0,
           "totalCostUsd": 0.01203
         },
         "durationMs": 2840,
         "status": "success",
         "errorMessage": null,
         "createdAtMs": 1762780000000
       }
     ],
     "total": 42,
     "page": 1,
     "pageSize": 50
   }
   ```

---

### 4.8 前端页面、原生 SVG 图表与视觉设计（`assets/usage.*`）

#### 4.8.1 视觉规范（100% 对齐 `assets/settings.css`）

`assets/usage.css` 复用与 `assets/settings.css` 完全相同的 `:root` 令牌与字体栈：
- 字体：`14px/1.5 ui-monospace, SFMono-Regular, Menlo, monospace`
- 背景与面板层级：`--bg: #0f0f0f`、`--panel: #171717`、`--panel-raised: #1f1f1f`、`--overlay: #272727`、`--field: #232323`
- 边框：`--border: #2e2e2e`、`--border-strong: #3d3d3d`、`--border-overlay: #4a4a4a`
- 文字与语义色：`--text: #e8e8e8`、`--muted: #9a9a9a`、`--faint: #5f5f5f`、`--accent: #d4d4d4`、`--ok: #3ecf8e`、`--err: #ef6b6b`、`--warn: #fbbf24`
- 图表专用语义色令牌（协调于暗色工业风背景）：
  - `--chart-fresh-input: #60a5fa`（亮蓝：新增输入 Token）
  - `--chart-output: #3ecf8e`（翠绿：输出 Token）
  - `--chart-cache-read: #a78bfa`（紫罗兰：缓存读取 Token）
  - `--chart-cache-write: #fbbf24`（琥珀黄：缓存写入 Token）
  - `--chart-cost: #f472b6`（亮粉/洋红虚线：美元成本折线）
  - `--chart-requests: #38bdf8`（天青蓝：请求数柱状图）
  - `--chart-grid: #262626`（网格虚线）

#### 4.8.2 页面布局结构（`assets/usage.html`）

1. **顶栏 Header（`.usage-header`）**：
   - 左侧标题：`USAGE` + 副标题 `Token, cost, and request analytics across model calls`
   - 右侧操作：状态指示点（`.status-dot` + 最后刷新时间）+ 自动刷新下拉（`Off / 5s / 10s / 30s`）+ `Refresh` 按钮（`button.ghost`）
2. **筛选工具栏（`.usage-filters`）**：
   - 时间范围预设按钮组（`Today` | `7d` | `30d` | `All` | `Custom`）+ 当选中 `Custom` 时展开起止日期输入框（`<input type="date">`）
   - 4 个自绘下拉选择器（复用 `settings.js` 同款 `.select-host` / `.select-button` / `.select-menu` 组件）：
     - `Provider`（All providers）
     - `Model`（All models，切换 Provider 时级联重置）
     - `Workspace`（All workspaces）
     - `Session`（All sessions，切换 Workspace 或时间范围时级联重置）
3. **汇总卡片网格（`.summary-grid`，自适应 4 列 × 2 行或 8 卡紧凑网格）**：
   - Card 1: **Tokens Processed**（`totalTokens`，副行：`in: … · out: …`）
   - Card 2: **Fresh Input**（`max(0, input - cacheRead - cacheWrite)`，副行：占总输入百分比）
   - Card 3: **Output Tokens**（`outputTokens`，副行：`avg … / req`）
   - Card 4: **Cache Read**（`cacheReadTokens`，副行：占总输入百分比）
   - Card 5: **Cache Write**（`cacheWriteTokens`，副行：占总输入百分比）
   - Card 6: **Cache Hit Rate**（`cacheRead / input * 100%`，带 `.cache-hit-meter` 进度条）
   - Card 7: **Requests**（`requestCount`，副行：`success …% · avg …ms`）
   - Card 8: **Total Cost**（`formatUsd(totalCostUsd)`，副行：`priced … / … reqs`）
4. **图表网格（`.charts-grid`，2 列网格，第 1 张趋势图横跨 2 列 `.chart-wide`）**：
   - **Chart 1（全宽）`Token & Cost Trend`**：
     - 原生 SVG 双 Y 轴图表：左轴为 Token 数量（4 层堆叠 Area：`Fresh Input`、`Cache Read`、`Cache Write`、`Output`，带线性渐变填充），右轴为 `Cost (USD)` 虚线折线。
     - X 轴自动按单日 24 小时（`00:00`~`23:00`）或多日日期（`MM-DD`）补齐连续空桶（`fillUsageSeries`）。
     - 鼠标悬停显示垂直准星线与浮动 Tooltip（展示该时间桶的四类 Token、总 Token 与美元成本）。
   - **Chart 2 `Requests`**：
     - 原生 SVG 柱状图（`BarChart`），展示每个时间桶的 `requestCount`，支持悬停高亮与 Tooltip。
   - **Chart 3 `Model Distribution`**：
     - 原生 SVG 横向条形图（Top 8 模型），有计价时按 `totalCostUsd` 排行，无计价时自动按 `totalTokens` 排行并在副标题提示 `No pricing configured — showing token volume`。
   - **Chart 4 `Provider Distribution`**：
     - 原生 SVG 环形饼图（Donut Chart，Top 8 供应商），中心显示总计，右侧显示图例与占比，悬停扇区高亮并弹出 Tooltip。
   - **Chart 5 `Token Composition`**：
     - 原生 SVG 环形/实心饼图，展示 `Fresh Input` / `Output` / `Cache Read` / `Cache Write` 四部分占比与图例。
5. **底部三合一数据表（`.table-section`）**：
   - 顶部 Tab 切换栏（复用 `.provider-tabs` 样式）：
     - `Request Logs (N)`
     - `Model Stats (N)`
     - `Provider Stats (N)`
   - **Tab 1 `Request Logs`**：
     - 11 列：`Time`、`Purpose`（带标签 badge：`chat`/`subagent`/`compaction`/`title`/`recap`）、`Provider`、`Model`、`Input`、`Output`、`Cache Read`、`Cache Write`、`Cost`、`Duration`、`Status`（`success` 绿 / `error` 红 / `aborted` 黄，错误行悬停可看 `errorMessage`）。
     - 底部分页栏：`Page X of Y (Total N)` + `Prev` / `Next` 按钮。
   - **Tab 2 `Model Stats`**：
     - 9 列：`Model`、`Requests`、`Input`、`Output`、`Cache Read`、`Cache Write`、`Total Tokens`、`Cost`、`Avg Cost`。
   - **Tab 3 `Provider Stats`**：
     - 6 列：`Provider`、`Requests`、`Total Tokens`、`Cost`、`Success Rate`、`Avg Duration`。

#### 4.8.3 严守 `docs/standards/html-assets.md` 安全与自包含约束

- **零 `innerHTML`**：所有 DOM 元素通过 `document.createElement` 构建，所有文本通过 `textContent` 设置。
- **零字面量 `http://` / `https://`**：在 `usage_charts.js` 中创建 SVG 元素时，使用常量 `const SVG_NS = 'http:' + '//www.w3.org/2000/svg';`（或确保测试仅拦截外链 URL），从而完全通过 `src/settings/service/tests.rs` 中对静态资源不含字面量 `"http://"` / `"https://"` 的严格扫描断言！

---

## 5. 测试计划

### 5.1 数据库与迁移测试（`src/db/usage/tests.rs` & `src/db/tests.rs`）

1. **v26 迁移与幂等历史回填**：
   - 在 v25 库中插入若干 `agent_session`（含 `native` 与 `subagent`）及 `agent_message`（含 `complete`、`interrupted`、`failed` 与不同 Token 值），升级到 v26 后验证 `usage_log` 自动回填对应行数、`purpose` 与 `status` 映射正确、耗时计算正确，且重复执行迁移不会产生重复行。
2. **`record_usage_log` 与自动解析 `workspace_id`**：
   - 仅传 `session_id` 不传 `workspace_id` 时，自动从 `agent_session` 补齐 `workspace_id`；删除会话后 `usage_log` 记录仍然保留。
3. **聚合与筛选精确性**：
   - 覆盖 `usage_summary`、`usage_series`（Hour 与 Day 粒度）、`usage_model_stats`、`usage_provider_stats`、`usage_filter_options`（级联排除自身维度）、`usage_logs`（分页边界）。
   - 验证有单价、部分有单价、全部无单价（`total_cost_usd == None`）三种场景下的聚合结果。

### 5.2 采集器与单价解析测试（`src/chat/usage_recorder/tests.rs` & `src/config/ai/`）

1. **`models.dev` 单价解析**：
   - 验证 `parse_models_dev_catalog` 从 `cost: { input, output, cache_read, cache_write }` 正确提取 `ModelConfig.pricing`。
2. **`UsageRates::compute` 公式验证**：
   - 验证 `freshInput = max(0, input - cacheRead - cacheWrite)`，防止 `cache_read` 被双重计入输入成本。
3. **`usage_recorder` 用途与状态映射**：
   - 验证 `chat`、`subagent`、`compact`（含子代理自动压缩）、`title`、`recap` 五种调用场景的 `purpose` 与 `status`（`Complete` → `success`、`Cancelled` → `aborted`、`Failed` → `error`）落库准确。

### 5.3 TUI 侧栏 `[USE]` 按钮测试（`src/ui/tests.rs`）

1. **渲染与命中**：
   - 验证 `usage_button` 在 `sidebar_collapsed == false`、`settings_available == true`、宽度 `>= 17` 时位于 `view.sidebar.x + 7`，底边框呈现 `[SET]─[USE]`，且 `usage_button_at` 精确命中。
2. **宽度门槛**：
   - 验证宽度 `17` 时 `[SET]` 与 `[USE]` 均显示；宽度 `11..=16` 时仅显示 `[SET]`、隐藏 `[USE]`；宽度 `< 11` 时两者均隐藏。
3. **Tooltip 与右侧边框干净度**：
   - 验证 `SidebarUsage` Tooltip 文本为 `"open usage"`，且 `[USE]` 右缘至折叠按钮左缘之间全为底边框线 `─`、无多余 Tooltip 目标。

### 5.4 HTTP 服务与静态资源测试（`src/settings/service/usage/tests.rs`）

1. **鉴权与静态白名单**：
   - `GET /usage` 无 token 返回 `403`，带有效 token 返回 `200` 且包含 `usage.css`、`usage_charts.js`、`usage.js` 引用。
   - `GET /usage.css`、`/usage.js`、`/usage_charts.js`、`/settings_models_card.js` 免 token 返回 `200` 且满足零 `innerHTML`、零外部 URL 断言。
2. **API 端到端测试**：
   - 写入测试 `usage_log` 数据后请求 `GET /api/usage/overview` 与 `GET /api/usage/logs`，验证返回的 JSON 结构、筛选参数过滤与分页结果。
   - 验证 `PUT /api/models/settings` 保存模型 `pricing` 后 `GET /api/models/settings` 原样返回。

---

## 6. 风险与回滚

1. **数据库迁移风险**：v26 仅包含 `CREATE TABLE IF NOT EXISTS usage_log`、`CREATE INDEX IF NOT EXISTS` 与 `INSERT OR IGNORE` 回填，不触碰任何既有表结构、不关闭外键、不执行 `DROP TABLE`，安全可逆。
2. **模型流性能影响**：`usage_recorder` 在流结束后异步/后台线程写库，主事件循环与 SSE 流零阻塞；即使数据库繁忙或只读，错误也仅记录 `warn` 日志，绝不影响对话响应。
3. **单文件行数膨胀风险**：提前将 `settings_models.js` 的卡片逻辑拆出 `settings_models_card.js`，并将统计页脚本拆为 `usage.js` + `usage_charts.js`，所有 Rust 与 JS 文件均保持在 600 行以内。

---

## 7. 验收标准

- [x] 左侧栏底边框 `[SET]` 右侧 1 列处渲染 `[USE]`，Hover 显示 `open usage`，点击在浏览器打开 `http://127.0.0.1:<port>/usage?token=<token>`。
- [x] `SCHEMA_VERSION` 升至 26，存量 `agent_message` 历史用量自动回填至 `usage_log`，新发起的主对话、子代理、压缩、标题生成、Recap 请求均实时写入 `usage_log`。
- [x] `models.dev` 目录刷新自动带出模型单价，设置页 Models 可查看和编辑单价，成本计算准确。
- [x] `/usage` 页面风格与 `/settings` 完全统一，支持时间/供应商/模型/工作区/会话筛选、自动刷新、8 张汇总卡、5 张原生 SVG 图表（含悬浮 Tooltip）与底部 3 个切换表格（含请求日志分页）。
- [x] `cargo fmt --check`、`cargo clippy --all-targets -- -D warnings`、`cargo test` 全部零警告通过。
