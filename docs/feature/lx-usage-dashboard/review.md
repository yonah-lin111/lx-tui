# lx-usage-dashboard 代码审查报告（Review）

> **审查范围**：`feat/lx-usage-dashboard` 分支相对于 `dev` 的全部新增与改动文件（涵盖 SQLite Schema v26、`src/db/usage/`、`src/chat/usage_recorder.rs`、`src/config/ai/`、`src/settings/service/`、`src/ui/controls.rs`、`src/event/`、`assets/usage.{html,css,js}`、`assets/usage_charts.js`、`assets/settings_*.js` 及设计文档）。
> **排序原则**：按严重程度由高到低分为 `P0`（阻断性功能缺陷/数据损坏）→ `P1`（高优数据准确性/并发竞态/核心交互缺陷）→ `P2`（边界条件/刻度/体验打磨）→ `P3`（代码拆分规范与文档对齐）。

---

## P0 — 阻断性功能缺陷与数据损坏

### P0-1：`resolve_purpose` 分支顺序错误导致所有 `Title` 与 `Recap` 请求被误记为 `Compaction`，且遗漏 `"title_block"`

- **文件与位置**：
  - `src/chat/usage_recorder.rs:77-89`
  - `src/chat/title.rs:200`（`build_title_request`）
  - `src/chat/recap.rs:177`（`build_recap_request`）
  - `src/event/chat/title.rs:97`（`kind: "title_block"`）
- **关键点**：
  生产环境中，所有会话自动标题（`title`）、右侧面板提示词块标题（`title_block`）与轮次摘要（`recap`）的模型流式请求在落库 `usage_log` 时，100% 被归类为 `UsagePurpose::Compaction`（`"compaction"`），`UsagePurpose::Title` 与 `UsagePurpose::Recap` 分支在真实调用链上永远不可达。
- **错误原因**：
  1. `title::build_title_request` 与 `recap::build_recap_request` 为了复用“不挂载工具”的请求构造语义，均将 `ChatRequest.purpose` 设为 `RequestPurpose::CompactSummary`。
  2. `resolve_purpose` 将 `if log.kind == "compact" || matches!(request.purpose, RequestPurpose::CompactSummary)` 放在第一优先级，导致 `log.kind == "title"` 和 `log.kind == "recap"` 的真实请求全部在首个分支短路返回 `UsagePurpose::Compaction`。
  3. 右侧面板 Prompt 模板块生成标题时传入的 `log.kind` 为 `"title_block"`，`resolve_purpose` 未匹配该值。
  4. 单元测试手工构造了 `sample_request(RequestPurpose::Chat)` 而非调用真实的 `build_title_request` / `build_recap_request`，掩盖了该缺陷。
- **修复方案**：
  调整 `resolve_purpose` 判定顺序：优先匹配显式 `log.kind`（`"title" | "title_block" => UsagePurpose::Title`，`"recap" => UsagePurpose::Recap`，`"compact" => UsagePurpose::Compaction`，`"subagent" => UsagePurpose::Subagent`），最后再回退检查 `request.purpose`；在 `src/chat/usage_recorder/tests.rs` 中使用真实 `build_title_request` 与 `build_recap_request` 补充回归测试。

---

### P0-2：快捷连接（Quick Connect）保存时前端漏传 `pricing` 且后端未回退默认单价，导致保存预设后清空内置目录单价

- **文件与位置**：
  - `assets/settings_connect.js:484-508`（`buildPresetModels`）
  - `src/settings/service/models/connect.rs:317-325`（`apply_connect_item`）
- **关键点**：
  1. 在 `Provider -> Quick Connect` 页签下，模型卡片同样渲染了 4 个单价输入框，但 `buildPresetModels(preset)` 组装提交载荷时漏掉了 `pricing: serializePricing(model.pricing, id)`，导致用户在快捷连接预设下修改的单价无法保存。
  2. 更严重的是，当用户首次配置任一快捷连接预设（如填入 API Key）并点击 `Save` 时，前端提交的 `models` 不含 `pricing`；后端 `apply_connect_item` 调用 `parse_models(models, record.models.as_ref())` 时因 `record.models` 为 `None`，将所有模型单价解析为 `None`。随后与 `default_models_for_preset(spec.id)`（含 `models.dev` 单价）比较不相等，将 `pricing: None` 的模型表永久固化写入 `connect.json`，清空了该预设下所有模型的目录单价。
- **错误原因**：
  拆分 `assets/settings_models_card.js` 并在自定义供应商 `buildPayload()` 中接入 `serializePricing` 时，遗漏了 `assets/settings_connect.js` 的 `buildPresetModels`；同时后端 `apply_connect_item` 在 `record.models` 为空时未回退以 `default_models_for_preset(spec.id)` 作为 `parse_models` 的 `existing` 基线。
- **修复方案**：
  1. 在 `assets/settings_connect.js` 的 `buildPresetModels` 中补齐 `pricing: serializePricing(model.pricing, id)`。
  2. 在 `src/settings/service/models/connect.rs` 的 `apply_connect_item` 中，当 `record.models` 为 `None` 时，传入 `&default_models_for_preset(spec.id)` 作为 `parse_models` 的 `existing` 基线，并添加回归测试。

---

## P1 — 高优数据准确性、并发竞态与核心交互缺陷

### P1-1：`/api/usage/*` 查询参数被二次 URL 解码，导致含 `+` 或 `%XX` 的筛选条件失配

- **文件与位置**：
  - `src/settings/service/usage.rs:141-235`
  - `src/settings/service/handlers.rs:167-208`
- **关键点**：
  当筛选参数（如 `provider`、`model`、`workspaceId`、`sessionId`）包含 `+`（如 `Claude+GPT`、`qwen-plus+latest`）或字面量 `%` 时，`/api/usage/*` 会将 `+` 错误替换为空格 `' '`，导致 SQL 精确匹配失配、返回空结果。
- **错误原因**：
  HTTP 路由层 `handlers::parse_request -> split_target` 已经对 `Request.query` 的所有键值执行过一次 `percent_decode`；`src/settings/service/usage.rs` 重复实现了 `url_decode` 并对已解码的 `request.query` 再次解码。
- **修复方案**：
  移除 `src/settings/service/usage.rs` 中的 `url_decode`，直接读取并 `trim()` `request.query` 中的已解码字符串，补充含 `+` 与 `%20` 的查询参数测试。

---

### P1-2：模型单价解析、缓存刷新与存量历史成本回填的 4 个连锁缺陷

- **文件与位置**：
  - `src/config/ai/presets.rs:233-255, 302-304`
  - `src/config/ai/catalog_refresh.rs:310-315`
  - `src/db/migration/schema.rs:713-764`
  - `src/db/usage/mod.rs`
- **关键点**：
  1. **`merge_provider_models` 单价合并条件写反（`presets.rs:302`）**：写成了 `if existing.pricing.is_some() { model.pricing = existing.pricing.clone(); }`，导致旧单价覆盖云端新单价（应为 `if model.pricing.is_none()` 时才保留 `existing.pricing`）。
  2. **旧版 `models_catalog_cache.json` 命中 HTTP 304 导致无单价（`catalog_refresh.rs:310`）**：升级前已落盘的缓存文件带有 `etag` 但不含 `pricing`；启动刷新携带 `If-None-Match` 收到 `304 Not Modified` 后直接复用无单价旧缓存。
  3. **跨供应商单价兜底命中 `$0` 免费渠道（`presets.rs:254`）**：`catalog_model_pricing` 跨供应商兜底按 `BTreeMap` 字典序遍历，排在 `"google"` / `"openai"` 之前的 `"github-copilot"` / `"github-models"` 声明了同名模型（如 `"gpt-5"`、`"gemini-2.5-pro"`）且单价为 `0.0`，导致自定义供应商下的同名模型被按 `$0` 计费。
  4. **历史回填记录成本恒为 `NULL`**：`SCHEMA_V26` 从 `agent_message` 回填的历史记录以及补充单价前的存量记录 `total_cost_usd` 均为 `NULL`，即使目录或配置中已有单价也从未回填。
- **错误原因**：
  合并判空条件取反、ETag 请求未检查缓存是否已含单价、跨供应商遍历未优先匹配正数单价、缺少对 `total_cost_usd IS NULL` 存量记录的成本回填机制。
- **修复方案**：
  1. 将 `merge_provider_models` 修正为 `if model.pricing.is_none() { model.pricing = existing.pricing.clone(); }`。
  2. 在 `catalog_refresh.rs` 中仅当缓存内至少有一个模型含 `pricing.is_some()` 时才发送 `If-None-Match`。
  3. 在 `catalog_model_pricing` 中支持按预设 `spec.name` 匹配供应商，跨供应商兜底时优先匹配含正数单价（`input > 0.0 || output > 0.0`）的官方供应商条目。
  4. 在 `Database` 新增 `backfill_usage_costs`，针对 `total_cost_usd IS NULL` 的存量 `(provider, model)` 组合按已解析正数单价在单事务中批量回填成本。

---

### P1-3：预设供应商名称在实时流与历史回填中口径分裂，且 `usage_recorder` 测试受宿主环境污染

- **文件与位置**：
  - `src/chat/usage_recorder.rs:178-237`
  - `src/settings/service/usage.rs:53-56`
  - `src/config/ai/mod.rs:535-573`
- **关键点**：
  1. `usage_recorder` 调用 `load_runtime_ai`（将 `connect.json` 预设物化为 `id = "anthropic", name = "Anthropic"`），而 `SCHEMA_V26` 历史回填写入 `"anthropic"` 且 `usage.rs` 仅调用 `load_ai`（不含 `connect.json`），导致预设供应商的新记录为 `"Anthropic"`、旧记录为 `"anthropic"`，同一供应商在看板中分裂为两条。
  2. `usage_recorder.rs` 在 `#[cfg(test)]` 分支下仍读取宿主机真实的 `~/.lx-t-dev/config/{ai,connect}.json` 与环境变量，若本机配置了 `ANTHROPIC_API_KEY`，单元测试 `src/chat/usage_recorder/tests.rs:407` 会因 `provider` 变成 `"Anthropic"` 而失败。
- **错误原因**：
  将 `connect.json` 物化出的内置预设展示名与 `ai.json` 中自定义供应商（`provider-xxx`）混为一谈，且测试分支未隔离宿主机配置目录。
- **修复方案**：
  1. 名称解析仅针对 `ai.json` 中用户配置的自定义供应商（将内部 ID 如 `provider-alq374` 映射为 `provider.name`），内置预设保持稳定标识（如 `anthropic`、`openai`），或者当且仅当用户在 `ai.json` 中显式配置了该供应商时才映射其自定义 `name`；单价解析则继续合并 `load_runtime_ai`（覆盖 `ai.json` + `connect.json` + 内置目录）。
  2. 在 `usage_recorder.rs` 的 `#[cfg(test)]` 分支中，不读取宿主机默认 `ai_config_path()`，保证单元测试完全隔离确定。

---

### P1-4：`sync_usage_provider_names` 在每次 `GET /api/usage/*` 无条件执行写操作，且重命名存在同名/互换覆盖风险

- **文件与位置**：
  - `src/db/usage/mod.rs:373-390`
  - `src/settings/service/usage.rs:53-56`
  - `src/settings/service/models/mod.rs:235-247`
- **关键点**：
  1. 前端每次刷新并发请求 `/api/usage/overview` 和 `/api/usage/logs`，每个 `GET` 请求都会逐条执行 `UPDATE usage_log SET provider = ?1 WHERE provider = ?2`，即使匹配 0 行也会在 SQLite 中开启写事务争抢写锁。
  2. `put_settings` 将 `old_name -> new_name` 混入 `BTreeMap` 顺序执行多条 `UPDATE`；若两个供应商互换名称（`A -> B, B -> A`）或曾使用默认名 `"New Provider"`，会造成链式覆盖或误改其他供应商历史记录。
- **错误原因**：
  缺少更新前的只读存在性检查、未使用单条 `CASE WHEN` 原子更新、未排除与其他供应商冲突的 `old_name`。
- **修复方案**：
  1. 在 `sync_usage_provider_names` 中先通过只读查询检查 `usage_log` 是否存在待映射的旧 `provider` 值；若无命中直接返回 `Ok(0)`（零写锁）。
  2. 若有命中，在单事务中用单条 `UPDATE usage_log SET provider = CASE provider WHEN ... END WHERE provider IN (...)` 原子更新。
  3. 在 `put_settings` 中，仅当 `old_name != "New Provider"` 且不与任何其他自定义供应商或内置预设的 `id`/`name` 冲突时，才加入重命名映射。

---

### P1-5：`assets/usage.js` 并发拉取竞态、在途请求静默吞掉用户筛选操作、以及筛选项回退后未触发重拉

- **文件与位置**：
  - `assets/usage.js:300-396`
- **关键点**：
  1. `loadUsageDashboard` 与 `loadUsageLogsPage` 开头使用 `if (usageState.loading) return;` 早退。当首次加载或自动刷新在途时，用户点击时间范围按钮、切换下拉筛选或翻页会先修改 DOM 与 `usageState`，随后请求被静默吞掉；等在途旧请求回包后又用旧数据覆盖界面。
  2. `syncFilterOptions` 发现已选中的 `model` / `provider` / `workspaceId` / `sessionId` 不在当前范围候选集中时，虽然将 `usageState.*` 重置为 `''` 并更新下拉框为 `All`，但未重新发起请求，导致界面显示 `All` 却展示着旧筛选条件下的空数据。
- **错误原因**：
  使用布尔锁阻止新请求，而非使用单调递增的 `requestSeq` 序号丢弃过期响应；`syncFilterOptions` 未向调用方返回重置标记。
- **修复方案**：
  1. 在 `usageState` 中引入 `requestSeq: 0`，移除 `if (usageState.loading) return;`，每次请求递增序号并在 `await` 返回后校验 `reqId === usageState.requestSeq`。
  2. 让 `syncFilterOptions(filters)` 返回是否有筛选项被重置（`boolean`）；若返回 `true`，将 `usageState.page = 1` 并立即重新调用 `loadUsageDashboard()`。
  3. `loadUsageLogsPage(nextPage)` 仅在响应成功且序号匹配时才更新 `usageState.page = targetPage`。

---

### P1-6：时间范围 `endTime` 设为今晚 `23:59:59.999` 导致 `Today` 趋势图画出未来归零小时，且 `All` 范围超过 366 天时截断丢弃近期数据

- **文件与位置**：
  - `assets/usage.js:227-265`（`resolveTimeRange`）
  - `assets/usage_charts.js:170-212`（`fillUsageSeries`）
- **关键点**：
  1. `resolveTimeRange` 将 `today`、`7d`、`30d` 的 `endTime` 设为当天 `23:59:59.999`，导致查看 `Today`（按小时粒度）时，`fillUsageSeries` 会生成当天 `00:00` 至 `23:00` 全部 24 个桶，当前时刻之后的未来小时全部画成断崖归零的折线。
  2. `fillUsageSeries` 从 `rangeStart` 正向循环且硬编码 `i < 366`；当 `All` 或自定义范围跨度超过 366 天时，循环在第 366 天停止，把最近（含今天）的数据全部截断丢弃。
- **错误原因**：
  未对齐 `lx-agent` 将预设时间范围的 `endTime` 钳制到 `Date.now()`；`fillUsageSeries` 超上限时从起点正向截断而非向终点对齐保留最近数据。
- **修复方案**：
  1. `resolveTimeRange` 中 `today` / `7d` / `30d` 的 `endTime` 设为 `now`，`custom` 的 `endTime` 设为 `Math.min(endMs, now)`。
  2. `fillUsageSeries` 中当起止桶数超过 366 时，将起点向终点回推 365 个桶，确保始终保留最近的数据桶。

---

### P1-7：图表重绘（自动刷新 / 筛选切换）与页面滚动时，悬浮 `#chart-tooltip` 卡死残留

- **文件与位置**：
  - `assets/usage_charts.js:216-267`
  - `assets/usage.js:200, 484-513`
- **关键点**：
  当鼠标悬停在图表上触发自动刷新或键盘切换筛选时，`replaceChildren()` 直接移除旧 SVG 节点而不触发 `mouseleave`，导致 `#chart-tooltip` 永久卡在屏幕上；滚动 `.usage-main` 时 tooltip 也不会收起。
- **错误原因**：
  `renderCharts`、`renderEmptyChart` 与全局 `scroll` 事件未主动调用 `hideChartTooltip()`。
- **修复方案**：
  在 `renderCharts()`、`renderEmptyChart()` 以及全局 `scroll` 事件监听中调用 `hideChartTooltip()`。

---

### P1-8：供应商分布图（`Provider Distribution`）按请求数绘图，与 `design.md` D4 及 `lx-agent` 口径不一致

- **文件与位置**：
  - `assets/usage_charts.js:721-756`（`renderProviderDistributionChart`）
  - `docs/feature/lx-usage-dashboard/design.md:53`（决策 D4）
- **关键点**：
  `design.md` D4 与 `lx-agent` 均规定供应商分布图在有单价时按花费（`totalCostUsd`）占比绘图，无单价时自动退回按 Token 总量（`totalTokens`）绘图；但 `renderProviderDistributionChart` 写死了按 `requestCount` 绘图。
- **错误原因**：
  实现 `renderProviderDistributionChart` 时未接入 `hasPricing` 分支逻辑。
- **修复方案**：
  对齐 `renderModelDistributionChart` 与 `lx-agent`：检测 `hasPricing`，有单价时按 `totalCostUsd` 排序绘图（副标题 `Top 8 by cost (USD)`，中心显示总花费），无单价时退回按 `totalTokens` 排序绘图（副标题 `No pricing configured — showing token volume`，中心显示总 Token）。

---

## P2 — 边界条件、坐标轴刻度与交互体验打磨

### P2-1：`title` / `recap` 后台流超时取消漏记日志，以及 `UsageRates` / `record_usage_log` 非有限浮点数防护

- **文件与位置**：
  - `src/event/chat/title.rs:231-248`
  - `src/event/chat/recap.rs:161-178`
  - `src/db/usage/mod.rs:137-170, 460-464`
  - `src/db/migration/schema.rs:734-735`
- **关键点**：
  1. `execute_title_stream` 与 `execute_recap_stream` 使用外层 `tokio::time::timeout(30s / 60s)` 包裹 `run_stream_with_log`（默认 `idle_timeout` 为 300s），超时直接 drop Future 导致不经过 `record_stream_finish`。
  2. `UsageRates::compute` 与 `record_usage_log` 未过滤 `NaN` / `Infinity` 浮点数。
  3. `SCHEMA_V26` 回填 SQL 的 `NULLIF(m.model_provider, '')` 未做 `TRIM`。
- **错误原因**：
  外层 timeout 早于流内部 `idle_timeout`；成本计算仅做了 `.max(0.0)` 未校验 `.is_finite()`。
- **修复方案**：
  将 `execute_title_stream` 与 `execute_recap_stream` 的 `idle_timeout` 钳制为各自超时阈值（30s / 60s），使超时在 `run_stream_with_log` 内部收敛并记录失败日志；在 `UsageRates::compute` 与 `record_usage_log` 中增加 `.is_finite()` 过滤；在 `SCHEMA_V26` 回填 SQL 中使用 `NULLIF(TRIM(...), '')`。

---

### P2-2：`usage_filter_options` 选中会话时锁死其他下拉框，且无标题会话短码回退碰撞为 `"sess-187"`

- **文件与位置**：
  - `src/db/usage/mod.rs:685-777`
- **关键点**：
  1. 查询 `providers`、`models`、`workspaces` 筛选项时带上了 `query.session_id`，导致选中某个 Session 后 Workspace 下拉框只剩当前工作区，无法直接切换工作区。
  2. 无标题或已删会话回退取 `&id[..8]`，而 `lx-tui` 的 `session_id` 形如 `"sess-1876b3..."`，前 8 位在数天内完全相同，导致下拉框出现多个重名的 `"sess-187"`。
- **错误原因**：
  `usage_filter_options` 未在非 Session 维度排除 `session_id`；未考虑 `new_external_id("sess")` 的前缀与高位时间戳格式。
- **修复方案**：
  构造 `base_query = UsageQuery { session_id: None, ..query.clone() }` 用于查询 `providers`、`models`、`workspaces`；无标题会话短码回退格式改为保留前缀与尾部唯一字符（如 `sess-…<末尾6位>`）；工作区排序将仍存在的工作区排在已删除工作区之前。

---

### P2-3：趋势图与柱状图在无成本或低请求数（`maxReq < 4`）时 Y 轴刻度失真

- **文件与位置**：
  - `assets/usage_charts.js:317-357, 477-495`
- **关键点**：
  1. 当所有数据点均无单价（`!hasAnyCost`）时，`renderTrendChart` 右 Y 轴仍显示 `$0`、`$0.0000`、`$0.0001` 等伪造刻度。
  2. 当 `maxReq < 4`（如 1 或 2）时，`renderRequestsChart` 左 Y 轴分 4 等份四舍五入后出现 `0, 0, 1, 1, 1` 的重复整数刻度。
- **错误原因**：
  未在 `!hasAnyCost` 时跳过右轴成本刻度；整数坐标轴未将最大值向上取整为 `ticks` 的整数倍。
- **修复方案**：
  `!hasAnyCost` 时不渲染 `renderTrendChart` 右 Y 轴成本刻度；`renderRequestsChart` 将 `maxReq` 向上取整为 `Math.max(4, Math.ceil(rawMax / 4) * 4)`。

---

### P2-4：设置页模型卡片跨 Tab 保存报错未切回 Custom 页签，且展开 `Advanced` 后修改 `Model ID` 导致卡片自动收起

- **文件与位置**：
  - `assets/settings_models.js:413-418`
  - `assets/settings_models_card.js:166-183`
- **关键点**：
  1. 在 `Custom` 模型卡片输入非法单价后切到 `Quick Connect` 点击 `Save`，`buildPayload` 抛错但未切回 `Custom` 页签，用户看不到报错卡片。
  2. `modelCard` 使用闭包常量 `const key = provider.key + '\u0000' + model.id` 维护 `expandedModels`，修改 `Model ID` 后触发重绘会使已展开的卡片收起。
- **错误原因**：
  `buildPayload` 的 `catch` 分支未调用 `switchProviderTab('custom')`；`idField` 的 `oninput` 未迁移 `expandedModels` 键。
- **修复方案**：
  在 `buildPayload` 的 `catch` 块中调用 `switchProviderTab('custom')`；在 `idField` 修改 `model.id` 时同步迁移 `expandedModels` 中的键。

---

### P2-5：汇总卡 `Requests` 副标题截断、下拉菜单视口右边界溢出与无障碍属性补齐

- **文件与位置**：
  - `assets/usage.js:52-111, 419-466, 610-635, 760-775`
  - `assets/usage.css:729-734`
- **关键点**：
  1. 第 7 张汇总卡 `Requests` 副标题拼接了 4 段统计，在常规宽度下末尾平均耗时被截断且无 `title` 悬浮提示，同时忽略了 `summary.successRate`。
  2. 自绘下拉菜单 `reposition()` 未钳制视口右边界，顶栏右侧 `#auto-refresh-filter`（宽 76px，菜单最小宽 140px）在窄屏下易超出右边界。
  3. `.range-btn` 缺少 `aria-pressed`，截断文本缺少 `title` 属性，窄屏 `<= 640px` 下 `.filter-selects` 变为 4 行单列导致固定顶栏过高。
- **错误原因**：
  副标题字段过多、下拉框 `left` 未取 `window.innerWidth - menuWidth - 8` 上界、响应式断点列数设为 1。
- **修复方案**：
  将 `Requests` 副标题改为展示成功率与平均耗时（如 `success 98.5% · avg 1.2s`），并为所有汇总卡主值/副标题与表格 `.cell-truncate` 设置 `title`；在 `createSelect` 的 `reposition()` 中钳制视口左右边界；为 `.range-btn` 同步维护 `aria-pressed`；在 `@media (max-width: 640px)` 下将 `.filter-selects` 设为 `repeat(2, minmax(0, 1fr))`。

---

## P3 — 代码拆分规范与文档对齐

### P3-1：拆分 `src/config/ai/pricing.rs` 与 `src/ui/controls/tests.rs` 以遵守文件行数与测试组织规范

- **文件与位置**：
  - `src/config/ai/mod.rs`（当前 966 行，逼近 1000 行红线）
  - `src/ui/tests.rs`（当前 3,481 行，超过 1500 行测试上限）
- **关键点**：
  `AGENTS.md` 与 `docs/standards/code-writing-standards.md` 规定生产代码单文件上限 1000 行，测试文件严格跟随被测子模块拆分且单文件不超过 1500 行。
- **错误原因**：
  新增的 `ModelPricing` / 单价与供应商名称解析方法直接写在 `src/config/ai/mod.rs`，新增的 `[USE]` 按钮测试直接追加在根部 `src/ui/tests.rs`。
- **修复方案**：
  1. 将 `ModelPricing`、`ModelConfig::parsed_pricing`、`AiSettings::resolve_model_pricing`、`AiSettings::resolve_provider_name`、`AiSettings::provider_name_map` 拆入 `src/config/ai/pricing.rs`，并在 `src/config/ai/mod.rs` 中 `pub use pricing::ModelPricing;`。
  2. 将 `src/ui/tests.rs` 中归属 `src/ui/controls.rs` 的 `[SET]` / `[USE]` 按钮与侧栏控件测试迁移至独立的 `src/ui/controls/tests.rs`。

---

### P3-2：清理侧栏按钮点击死分支并同步更新 `design.md` 与 `task.md`

- **文件与位置**：
  - `src/event/mouse/press.rs:207-229`
  - `src/event/mod.rs:341-364`
  - `docs/feature/lx-usage-dashboard/design.md`
  - `docs/feature/lx-usage-dashboard/task.md`
- **关键点**：
  1. `press.rs` 中 `settings_button_at` 的 `else` 分支因 `ui::settings_button` 已检查 `state.settings_available` 而成为不可达死代码，且与 `usage_button_at` 写法不一致；`src/event/mod.rs` 在 `settings_service` 为 `None` 时未显示 `TOAST_SETTINGS_UNAVAILABLE`。
  2. `design.md` 与 `task.md` 需同步补充 D6（固定紧凑顶栏）、D7（供应商可读名称解析与存量同步、历史成本回填）以及修正少量字段名描述（`daily`、`RequestPurpose::CompactSummary`、20 列）。
- **错误原因**：
  增量迭代后未及时清理冗余分支与同步设计文档。
- **修复方案**：
  统一精简 `press.rs` 的 `[SET]` / `[USE]` 点击处理并在 `src/event/mod.rs` 补齐不可用 Toast；同步更新 `design.md` 与 `task.md`。
