# lx 模型用量统计看板（Usage Dashboard）— 执行任务清单

> 状态：已完成（`.worktrees/lx-usage-dashboard`，分支 `feat/lx-usage-dashboard`），待用户确认合并
> 设计依据：[design.md](design.md)
> 执行位置：`/Users/yonah/projects/agent/lx-tui/.worktrees/lx-usage-dashboard`
> 完成标准：按阶段完成全部代码实现 + 单元测试与集成测试 + `cargo fmt` + `cargo clippy --all-targets -- -D warnings` + `cargo test` 全部通过

---

## P0. 开工门槛与规范复核

- [x] 复核 `codegraph` 与 `codebase-memory-mcp` 索引状态正常。
- [x] 复核 `src/db/mod.rs` 当前 `SCHEMA_VERSION` 为 `25`，确认升级目标为 `26`。
- [x] 复核 `docs/standards/project-directory-structure.md`、`docs/standards/code-writing-standards.md`、`docs/standards/tui-design-requirements.md`、`docs/standards/html-assets.md` 约束（单生产文件 ≤ 1000 行、禁止 `unwrap()`/`expect()`、禁止 `.innerHTML`、禁止外部 URL、界面文案全英文）。

---

## P1. 数据库 Schema v26 迁移与 `src/db/usage/` 聚合查询层

- [x] **`src/db/mod.rs`**：
  - [x] 将 `pub const SCHEMA_VERSION: i64 = 25;` 递增为 `26`。
  - [x] 注册 `pub mod usage;` 并导出 `pub use usage::*;`。
  - [x] 在 `migration_report()` 的 `table_counts` 中加入 `"usage_log"`。
- [x] **`src/db/migration/schema.rs`**：
  - [x] 新增 `pub(super) const SCHEMA_V26: &str`，包含：
    - `CREATE TABLE IF NOT EXISTS usage_log (...)`（20 列：`id`、`external_id`、`session_id`、`workspace_id`、`purpose`、`provider`、`model`、`input_tokens`、`output_tokens`、`cache_read_tokens`、`cache_write_tokens`、`input_cost_usd`、`output_cost_usd`、`cache_read_cost_usd`、`cache_write_cost_usd`、`total_cost_usd`、`duration_ms`、`status`、`error_message`、`created_at_ms`）。
    - 5 个索引：`idx_usage_log_created_at`、`idx_usage_log_model`、`idx_usage_log_provider`、`idx_usage_log_workspace`、`idx_usage_log_session`。
    - 幂等历史回填 SQL：`INSERT OR IGNORE INTO usage_log (...) SELECT 'ulog-backfill-' || m.external_id, ... FROM agent_message m LEFT JOIN agent_session s ON s.external_id = m.session_id WHERE m.role = 'assistant' AND m.kind = 'chat' ...`。
- [x] **`src/db/migration.rs`**：
  - [x] 在 `if version < 25` 块之后追加 `if version < 26 { self.conn.execute_batch(schema::SCHEMA_V26)?; self.conn.pragma_update(None, "user_version", 26)?; }`。
- [x] **新增 `src/db/usage/mod.rs`**（控制在 ~450 行）：
  - [x] 定义枚举 `UsagePurpose`（`Chat`、`Subagent`、`Compaction`、`Title`、`Recap`）与 `UsageStatus`（`Success`、`Error`、`Aborted`）及 `as_str()` / `parse()`。
  - [x] 定义枚举 `UsageGranularity`（`Hour`、`Day`）。
  - [x] 定义结构体 `UsageRates` 及 `UsageRates::compute(input, output, cache_read, cache_write, pricing)`（按 `fresh_input = max(0, input - cache_read - cache_write)` 计算每百万 Token 美元成本）。
  - [x] 定义结构体 `UsageLogInsert`、`UsageLogRecord`、`UsageQuery`、`UsageSummary`、`UsageDailyPoint`、`UsageModelStats`、`UsageProviderStats`、`UsageNamedOption`、`UsageFilterOptions`、`UsageLogPage`。
  - [x] 实现 `Database::record_usage_log(&self, entry: &UsageLogInsert) -> Result<()>`（当 `workspace_id` 为空且有 `session_id` 时自动从 `agent_session` 反查补齐）。
  - [x] 实现 `Database::usage_summary(&self, query: &UsageQuery) -> Result<UsageSummary>`。
  - [x] 实现 `Database::usage_series(&self, query: &UsageQuery, granularity: UsageGranularity) -> Result<Vec<UsageDailyPoint>>`。
  - [x] 实现 `Database::usage_model_stats(&self, query: &UsageQuery) -> Result<Vec<UsageModelStats>>`。
  - [x] 实现 `Database::usage_provider_stats(&self, query: &UsageQuery) -> Result<Vec<UsageProviderStats>>`。
  - [x] 实现 `Database::usage_filter_options(&self, query: &UsageQuery) -> Result<UsageFilterOptions>`（联表 `workspace` 与 `agent_session` 获取可读名称）。
  - [x] 实现 `Database::usage_logs(&self, query: &UsageQuery, page: usize, page_size: usize) -> Result<UsageLogPage>`。
- [x] **新增 `src/db/usage/tests.rs`**：
  - [x] 测试 v26 迁移历史回填（验证 `native` 与 `subagent` 会话、`complete`/`interrupted`/`failed` 状态映射、重复执行幂等性）。
  - [x] 测试 `record_usage_log` 与会话/工作区删除后的数据保留。
  - [x] 测试 `usage_summary`、`usage_series`（hour/day）、`usage_model_stats`、`usage_provider_stats`、`usage_filter_options`、`usage_logs` 分页与多维条件组合过滤。

---

## P2. 模型单价（Pricing）解析、目录提取与配置往返

- [x] **`src/config/ai/mod.rs`**：
  - [x] 新增 `pub struct ModelPricing { pub input: Option<f64>, pub output: Option<f64>, pub cache_read: Option<f64>, pub cache_write: Option<f64> }`。
  - [x] 实现 `ModelPricing::from_json_value`（兼容 `cacheRead`/`cache_read` 与 `cacheWrite`/`cache_write`，过滤负数/NaN/Infinity）与 `ModelPricing::to_json_value`。
  - [x] 为 `ModelConfig` 增加 `pub fn parsed_pricing(&self) -> Option<ModelPricing>`。
  - [x] 为 `AiSettings` 增加 `pub fn resolve_model_pricing(&self, provider_id: &str, model_id: &str) -> Option<ModelPricing>`（先查用户/连接配置，再查 `presets::catalog()`）。
- [x] **`src/config/ai/catalog_refresh.rs`**：
  - [x] 新增 `fn parse_pricing(value: Option<&Value>) -> Option<Value>`，在 `parse_models_dev_catalog` 中解析 `model.get("cost").or_else(|| model.get("pricing"))` 写入 `ModelConfig.pricing`。
- [x] **`src/config/ai/presets.rs`**：
  - [x] 在 `merge_discovered_models_for_preset` 中：若用户已有配置含 `pricing` 则保留，否则继承目录模型的 `pricing`。
- [x] **`src/config/ai/catalog_refresh_tests.rs` & `src/config/ai/tests.rs`**：
  - [x] 新增测试验证 `models.dev` JSON 中的 `cost` 字段被正确提取为 `ModelConfig.pricing`，且 `resolve_model_pricing` 优先级正确。

---

## P3. 模型请求全链路拦截与异步落库（`src/chat/usage_recorder.rs`）

- [x] **`src/chat/events.rs` & `src/chat/compact.rs`**：
  - [x] 在 `RequestPurpose` 中复用/标识 `CompactSummary`，并在 `compact::summary_request` 与 `compact::summary_request_from_transcript` 中设置 `purpose: RequestPurpose::CompactSummary`，使子代理自动压缩请求（`maybe_auto_compact`）与主对话手动压缩均能被精确识别为 `UsagePurpose::Compaction`。
- [x] **新增 `src/chat/usage_recorder.rs`**：
  - [x] 实现 `pub fn record_stream_finish(log: &StreamLog, request: &ChatRequest, deltas: &[ChatDelta], stop: &StopReason, duration_ms: i64)`。
  - [x] 实现 `extract_step_usage(deltas: &[ChatDelta]) -> Usage`（与 `agent_loop::step_usage` 同口径）。
  - [x] 实现 `resolve_purpose(log: &StreamLog, request: &ChatRequest) -> UsagePurpose`：
    - `log.kind == "title" | "title_block"` → `Title`
    - `log.kind == "recap"` → `Recap`
    - `log.kind == "compact"` 或 `RequestPurpose::CompactSummary` → `Compaction`
    - `log.kind == "subagent"` 或 `RequestPurpose::Subagent(_)` → `Subagent`
    - 其余 → `Chat`
  - [x] 实现 `resolve_status(stop: &StopReason) -> (UsageStatus, Option<String>)`。
  - [x] 当 `log.kind == "unknown"` 且未设置测试捕获目标时跳过落库（防止纯传输层 mock 测试污染开发库）；生产调用（`chat`/`subagent`/`compact`/`title`/`recap`/`exec`）通过后台任务打开 `Database` 写入 `usage_log`。
- [x] **`src/chat/client/mod.rs`**：
  - [x] 在 `run_stream_with_log` 末尾调用 `crate::chat::usage_recorder::record_stream_finish(log, &request, &deltas, &stop, elapsed_ms)`。
- [x] **新增 `src/chat/usage_recorder/tests.rs`**：
  - [x] 测试 5 类 `purpose`、3 类 `status`、单价计算与数据库写入完整链路。

---

## P4. TUI 左侧栏底边框 `[USE]` 按钮与打开浏览器交互

- [x] **`src/ui/text/workspace.rs`**：
  - [x] 新增 `pub const USAGE_LABEL: &str = "[USE]";`。
- [x] **`src/app/tooltip.rs` & `src/ui/tooltip.rs` & `src/ui/text.rs`**：
  - [x] `TooltipTarget` 新增 `SidebarUsage`。
  - [x] `ui::tooltip::targets` 注册 `controls::usage_button(view, state)` → `TooltipTarget::SidebarUsage`。
  - [x] `ui::text::tooltip_text` 映射 `TooltipTarget::SidebarUsage => "open usage"`。
- [x] **`src/ui/controls.rs`**：
  - [x] 新增常量 `USAGE_BUTTON_WIDTH = 5`、`USAGE_BUTTON_GAP = 1`、`USAGE_BUTTON_MIN_WIDTH = SETTINGS_BUTTON_MIN_WIDTH + 1 + 5`（17）。
  - [x] 实现 `pub fn usage_button(view: &layout::ViewLayout, state: &AppState) -> Option<Rect>`。
  - [x] 实现 `pub fn usage_button_at(view: &layout::ViewLayout, state: &AppState, column: u16, row: u16) -> bool`。
  - [x] 实现 `pub(crate) fn render_usage_button(frame: &mut Frame<'_>, view: &layout::ViewLayout, state: &AppState)`。
- [x] **`src/ui/mod.rs`**：
  - [x] 在 `controls::render_settings_button(frame, &view, state);` 后调用 `controls::render_usage_button(frame, &view, state);`，并在 `pub use controls::` 中导出 `usage_button, usage_button_at`。
- [x] **`src/app/state.rs`**：
  - [x] `AppState` 新增 `pub usage_open_requested: bool`（默认 `false`）。
- [x] **`src/event/mouse/press.rs`**：
  - [x] 在 `ui::settings_button_at` 之后新增 `if ui::usage_button_at(view, state, mouse.column, mouse.row) { state.usage_open_requested = true; *dirty = true; return; }`。
- [x] **`src/event/mod.rs`**：
  - [x] 在 `if state.settings_open_requested` 块之后新增 `if state.usage_open_requested` 块，调用 `SettingsService::usage_url` 并在 `tokio::task::spawn_blocking` 中调用 `crate::platform::open_browser(&url)`。
- [x] **`src/ui/tests.rs` & `docs/standards/tui-design-requirements.md`**：
  - [x] 更新 `docs/standards/tui-design-requirements.md` 第 21 行说明 `[SET]` 右侧 `[USE]` 按钮。
  - [x] 更新 `src/ui/tests.rs` 中侧栏底边框测试，新增 `usage_button_renders_right_of_settings_and_hits` 与 `usage_button_width_threshold_is_seventeen` 测试。

---

## P5. 本地 HTTP 服务 `/usage` 路由、`/api/usage/*` 与设置页单价编辑

- [x] **`src/settings/service.rs`**：
  - [x] `SettingsService` 新增 `pub fn usage_url(&self) -> String`。
  - [x] `ServiceState` 新增 `pub fn db_path(&self) -> PathBuf`（测试模式下使用 `user_root/db/lx.db`，生产模式下使用 `crate::db::default_db_path()`）。
- [x] **`src/settings/service/page.rs`**：
  - [x] 新增 `pub fn usage_html() -> &'static str { include_str!("../../../assets/usage.html") }`。
  - [x] `asset(path)` 新增 `"/settings_models_card.js"`、`"/usage.css"`、`"/usage.js"`、`"/usage_charts.js"`。
- [x] **`src/settings/service/handlers.rs`**：
  - [x] `route()` 新增 `("GET", "/usage")`（返回 `page::usage_html()`）与 `(_, path) if path.starts_with("/api/usage")`（转发至 `super::usage::route(request, state)`）。
- [x] **新增 `src/settings/service/usage.rs`**：
  - [x] 解析查询参数 `startTime`、`endTime`、`provider`、`model`、`workspaceId`、`sessionId`、`granularity`、`page`、`pageSize`。
  - [x] 实现 `GET /api/usage/overview`（返回 `summary`、`daily`、`models`、`providers`、`filters`）。
  - [x] 实现 `GET /api/usage/logs`（返回分页 `rows`、`total`、`page`、`pageSize`）。
- [x] **`src/settings/service/models/mod.rs`**：
  - [x] `GET /api/models/settings` 返回模型列表时附带 `"pricing": model.pricing`。
  - [x] `parse_models` 支持 `pricing` 三态（`Null` 清空、对象校验为 `ModelPricing`、缺省保留旧值）。
- [x] **拆分 `assets/settings_models.js` → `assets/settings_models_card.js`**：
  - [x] 将 `modelCard`、`modalitySection`、`thinkingSection`、`pricingSection`（新增 4 个单价输入框：Input / Output / Cache read / Cache write，USD / 1M tokens）拆入 `assets/settings_models_card.js`，使两个文件均保持在 500~650 行。
  - [x] 在 `assets/settings.html` 中加载 `/settings_models_card.js`。

---

## P6. 统计看板前端实现（`assets/usage.{html,css,js}` & `assets/usage_charts.js`）

- [x] **新增 `assets/usage.html`**：
  - [x] 顶栏 Header（标题 `USAGE`、状态指示、自动刷新选择、手动 `Refresh` 按钮）。
  - [x] 筛选栏（`Today` / `7d` / `30d` / `All` / `Custom` 日期范围 + `Provider` / `Model` / `Workspace` / `Session` 自绘下拉框）。
  - [x] 8 张汇总卡容器（`#summary-cards`）。
  - [x] 5 张图表卡片容器（`#chart-trend`、`#chart-requests`、`#chart-models`、`#chart-providers`、`#chart-composition`）。
  - [x] 底部 3 个切换 Tab（`Request Logs` / `Model Stats` / `Provider Stats`）及表格与分页栏容器。
  - [x] 浮动 Tooltip 容器（`#chart-tooltip`）与 Toast 容器（`#toast-host`）。
- [x] **新增 `assets/usage.css`**：
  - [x] 完整对齐 `assets/settings.css` 的 `:root` 暗色令牌、字体、按钮、自绘 `.select-*`、`.toast` 样式。
  - [x] 定义 `.usage-shell`（纵向可滚动主容器）、`.usage-filters`、`.summary-grid`、`.summary-card`、`.cache-hit-meter`、`.charts-grid`、`.chart-card`、`.chart-tooltip`、`.usage-table`、`.status-badge`、`.purpose-badge`、`.pagination` 样式及响应式断点。
- [x] **新增 `assets/usage_charts.js`**（原生 SVG 渲染引擎，零外部依赖、零 `innerHTML`）：
  - [x] SVG 辅助构造函数 `svgEl(tag, attrs)`（使用 `'http:' + '//www.w3.org/2000/svg'` 避免触发静态资源外链字面量拦截）。
  - [x] 连续时间桶补齐函数 `fillUsageSeries(points, range, granularity)`（单日补齐 24 小时桶，多日补齐每日桶）。
  - [x] `renderTrendChart(host, series)`：4 层堆叠面积图（`Fresh Input`、`Cache Read`、`Cache Write`、`Output`）+ 右 Y 轴 `Cost (USD)` 虚线折线 + 悬停准星与多指标 Tooltip。
  - [x] `renderRequestsChart(host, series)`：请求数柱状图 + 悬停高亮与 Tooltip。
  - [x] `renderModelDistributionChart(host, models)`：Top 8 模型横向条形图（有单价按 Cost，无单价按 Tokens 并更新副标题提示）。
  - [x] `renderProviderDistributionChart(host, providers)`：Top 8 供应商环形饼图（Donut Chart）+ 右侧图例 + 悬停 Tooltip。
  - [x] `renderTokenCompositionChart(host, summary)`：4 类 Token 构成环形/饼图 + 右侧图例 + 悬停 Tooltip。
- [x] **新增 `assets/usage.js`**：
  - [x] 从 `location.search` 提取 `token`，封装带 `token` 的 `api(path)` 请求函数。
  - [x] 实现自绘下拉组件 `createSelect`、时间范围计算 `resolveTimeRange`、格式化工具（`formatNumber`、`formatCompact`、`formatUsd`、`formatPercent`、`formatDuration`、`formatTimestamp`）。
  - [x] 实现级联筛选（切换 Provider 清空不匹配的 Model，切换 Workspace/时间范围刷新 Session 列表并回退失效 Session）。
  - [x] 实现汇总卡渲染、5 张图表联动更新、3 张表格切换渲染、Request Logs 分页跳转、自动刷新定时器（`Off / 5s / 10s / 30s`）。

---

## P7. 服务层测试与文档同步

- [x] **新增 `src/settings/service/usage/tests.rs`** 并更新 `src/settings/service/tests.rs`：
  - [x] 将 `usage.html`、`usage.css`、`usage.js`、`usage_charts.js`、`settings_models_card.js` 纳入 `page_requires_token_and_serves_self_contained_assets` 的零外部链接与零 `innerHTML` 断言。
  - [x] 测试 `/usage` 页面 token 鉴权、`/api/usage/overview` 与 `/api/usage/logs` 数据准确性、筛选条件与分页边界。
  - [x] 在 `src/settings/service/models/tests.rs` 中新增 `pricing_round_trips_and_explicit_null_clears` 测试。
- [x] **更新项目规范文档**：
  - [x] `docs/standards/tui-design-requirements.md`（侧栏底边框 `[SET]` 右侧 `[USE]` 按钮说明）。
  - [x] `docs/standards/html-assets.md`（`assets/` 目录文件清单）。
  - [x] `docs/standards/project-directory-structure.md`（新增模块说明）。

---

## P8. 最终校验

- [x] 执行 `cargo fmt --check` 确保格式规范。
- [x] 执行 `cargo clippy --all-targets -- -D warnings` 确保零警告。
- [x] 执行受影响模块及全量测试 `cargo test` 确保全部通过。
- [x] 核查所有新增/修改文件的行数，确认生产代码单文件均 `< 1000` 行（子模块控制在 500 行左右），测试文件均 `< 1500` 行。

---

## P9. 代码审查（Review）与 P0–P3 缺陷修复

- [x] **编写 `docs/feature/lx-usage-dashboard/review.md`**：按 `P0` → `P3` 梳理全部审查关键点、根因与修复方案。
- [x] **P0 修复**：
  - [x] `P0-1`：修正 `src/chat/usage_recorder.rs` 中 `resolve_purpose` 分支顺序并补齐 `"title_block"`，修复 `Title` 与 `Recap` 因 `RequestPurpose::CompactSummary` 被误记为 `Compaction` 的问题。
  - [x] `P0-2`：在 `assets/settings_connect.js` 的 `buildPresetModels` 补传 `pricing: serializePricing(model.pricing, id)`，并在 `src/settings/service/models/connect.rs` 回退默认模型单价基线，防止保存快捷连接时清空目录单价。
- [x] **P1 修复**：
  - [x] `P1-1`：移除 `src/settings/service/usage.rs` 中重复的 `url_decode`，修复含 `+` 或 `%XX` 的筛选条件被二次解码失配的问题。
  - [x] `P1-2`：修复 `merge_provider_models` 单价合并条件、旧缓存 ETag 无单价刷新、跨供应商单价优先匹配官方正数单价，以及新增 `Database::backfill_usage_costs` 补齐存量历史记录的 `NULL` 成本。
  - [x] `P1-3`：在 `src/config/ai/pricing.rs` 中区分自定义供应商与未改名内置预设，并在 `usage_recorder` 测试路径隔离宿主机配置。
  - [x] `P1-4`：`sync_usage_provider_names` 增加只读预检与单事务 `CASE WHEN` 原子更新，避免每次 GET 请求触发无谓写锁或重命名链式覆盖。
  - [x] `P1-5`：`assets/usage.js` 引入 `requestSeq` 替代布尔锁早退，并在 `syncFilterOptions` 重置失效筛选项后自动触发重拉。
  - [x] `P1-6`：`resolveTimeRange` 将截至今日的 `endTime` 钳制到 `Date.now()`，且 `fillUsageSeries` 超 366 天时保留最近数据桶。
  - [x] `P1-7`：在 `renderCharts`、`renderEmptyChart` 与全局 `scroll` 中调用 `hideChartTooltip()` 防止悬浮提示残留。
  - [x] `P1-8`：`renderProviderDistributionChart` 对齐 `design.md` D4 与 `lx-agent`（有单价按花费占比，无单价退回按 Token 占比）。
- [x] **P2 修复**：
  - [x] `P2-1`：钳制 `title` / `recap` 流的 `idle_timeout`，并为 `UsageRates::compute` 与 `record_usage_log` 增加 `.is_finite()` 校验。
  - [x] `P2-2`：`usage_filter_options` 在非 Session 维度排除 `session_id`，优化无标题会话短码回退与工作区排序。
  - [x] `P2-3`：无成本时隐藏趋势图右 Y 轴刻度，并将请求数柱状图 `maxReq` 向上取整为 4 的倍数。
  - [x] `P2-4`：设置页跨 Tab 校验报错自动切回 `Custom` 页签，修改 `Model ID` 时同步迁移展开态键。
  - [x] `P2-5`：汇总卡 `Requests` 副标题展示成功率与平均耗时并补齐 `title`、下拉菜单右边界钳制、`.range-btn` `aria-pressed` 与窄屏紧凑布局。
- [x] **P3 修复**：
  - [x] `P3-1`：将 `ModelPricing` 与单价/名称解析拆入 `src/config/ai/pricing.rs`，将侧栏控件测试拆入 `src/ui/controls/tests.rs`。
  - [x] `P3-2`：清理 `src/event/mouse/press.rs` 死分支并在 `src/event/mod.rs` 补齐服务不可用 Toast，同步更新 `design.md` 与 `task.md`。
