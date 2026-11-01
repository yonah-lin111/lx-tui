# lx `web_search` 联网搜索代码评审报告（Review）

> 工作区：`.worktrees/lx-web-search`（分支 `feat/lx-web-search`）
> 范围：配置与 Web 设置页（`src/config/ai/`、`src/settings/service/`、`assets/settings.*`）、工具核心与提示词（`src/chat/tools/web_search/`、`src/chat/`、`src/event/chat/`）、TUI 状态与卡片渲染（`src/app/`、`src/ui/`）及特性文档（`design.md`、`task.md`）。
> 状态：已完成全部 P0–P3 问题定位，按本报告逐项执行修复。

---

## P0（阻断性缺陷：核心逻辑失效 / 状态不一致 / 错误信息丢失）

### P0-1 匿名 Exa 401/403 熔断器在配置非空 Key 时失效（每条查询无条件清零 `EXA_ANON_BLOCKED`）

- **位置**：`src/chat/tools/web_search/client.rs:247-264`
- **关键点**：
  - 在单条查询函数 `search()` 中，只要 `exa_api_key` 非空（默认即带内置 Key），进入分支首行便执行 `EXA_ANON_BLOCKED.store(false, Ordering::Relaxed)`。
  - 紧接着当带 Key 请求因配额或鉴权返回 401/402/403/429 时，第 253 行判断 `!EXA_ANON_BLOCKED.load(...)` 恒为 `true`（因为 4 行前刚被清零），导致每次查询、乃至同一批次 4 条并发查询中的每一条都会重复发起注定 401/403 的匿名 Exa 请求，熔断器永远无法保持生效。
  - 同时 `design.md` §5.3 要求的 `TAVILY_ANON_BLOCKED`（或空 Key 短路）未实现，当用户清空 Tavily Key 时每次回退仍会向 Tavily 发送无 `Authorization` 头的必败请求。
- **错误原因**：
  - 将「熔断器复位」错误地绑定在每次单条查询的 `exa_api_key` 非空分支开头，混淆了「配置持有非空 Key」与「匿名通道可用」两个独立状态；同时遗漏了 Tavily 空 Key 时的短路/熔断保护。
- **修复方案**：
  1. 移除 `search()` 内根据 `!exa_api_key.is_empty()` 无条件重置 `EXA_ANON_BLOCKED` 的逻辑；仅当匿名 Exa 请求成功（2xx）时将 `EXA_ANON_BLOCKED` 置为 `false`，匿名 Exa 返回 401/403 时置为 `true`。
  2. 当 `tavily_api_key.is_empty()` 时直接短路返回 `"Tavily API key is not configured"`，不再发送必败的无认证 HTTP 请求。
  3. 修正并补充连续两次调用下「Key 401 → 匿名 403 触发熔断 → 第二次调用不再打匿名 Exa 直接回退 Tavily」的回归测试。

---

### P0-2 `PUT /api/config` 非原子落盘：先写 `settings.json` 并广播事件，后校验/写 `ai.json`

- **位置**：`src/settings/service/handlers.rs:296-299, 371-417`、`src/config/ai/mod.rs:438-443`
- **关键点**：
  - `put_config` 先将布局配置写入 `settings.json` 并投递 `SettingsEvent::ConfigChanged(config)`，之后才读取、合并、保存 `ai.json`。若 `ai.json` 阶段失败返回 422/500，浏览器收到报错且不更新前端基线，但 `settings.json` 已落盘、TUI 已生效，造成前后端状态撕裂。
  - `json_object_field(&request.body, "web_search")` 在客户端传入非对象类型（如 `"web_search": "invalid"`）时静默返回 `None` 并响应 `200 OK`，未报 `422`。
  - 当磁盘上 `ai.json` 损坏时，`GET /api/state` 吞掉错误回退默认值，而 `PUT /api/config` 却因 `load_ai_settings` 报错返回 422，导致用户无法通过保存操作修复。
  - `save_ai` 使用固定临时文件名 `ai.json.tmp`，多请求并发保存时存在临时文件覆盖竞态。
- **错误原因**：
  - 副作用执行顺序倒置（先落盘广播、后校验合并），且未区分字段「未提供」与「类型错误」。
- **修复方案**：
  1. 在 `put_config` 中先校验 `"web_search"` 字段（若字段存在但非对象，或 Key 含控制字符，立即返回 `422`）并预加载 `AiSettings`（若 `ai.json` 损坏则记录告警并以 `AiSettings::default()` 为基底修复 `web_search`），校验全部通过后再依次写 `settings.json`、`ai.json`，最后统一发送 `ConfigChanged` 与 `ModelsChanged`。
  2. 在 `save_ai` 中为临时文件名追加进程 ID 与原子递增序号，消除并发写冲突。

---

### P0-3 全部搜索源失败时丢弃真实错误原因，仅返回静态文案

- **位置**：`src/chat/tools/web_search/client.rs:181-188, 315-321, 345-347`
- **关键点**：
  - `search()` 已收集并脱敏了各引擎失败原因 `"{exa_message}; {tavily_message}"`，但当全部查询失败时，`execute_with_endpoints` 将 `outcomes` 中的错误详情全部丢弃，只返回固定提示 `"Web search failed across all providers..."`，模型与用户无法区分是「Key 401/429」「网络超时」还是「服务端 5xx」。
  - 当所有查询均因超时失败时，错误码仍返回 `ToolErrorCode::Io` 而非 `ToolErrorCode::Timeout`。
  - HTTP 非 2xx 时直接丢弃了响应体中的错误详情（导致 `parse_tavily_body` 中的 `detail.error` 分支在非 2xx 下不可达）。
- **错误原因**：
  - 汇总失败分支未拼接 `outcomes` 中已脱敏的错误摘要，也未提取非 2xx 响应体中的安全错误字段。
- **修复方案**：
  1. 全部查询失败时，汇总去重后的 `outcomes` 错误详情附在静态引导文案之后；若全部查询均因超时失败，返回 `ToolErrorCode::Timeout`。
  2. HTTP 非 2xx 时尝试从响应体 JSON 提取 `error` / `detail.error` / `message` 简短摘要（剥离控制字符、截断至 200 字符、绝不包含请求 URL 或 Key）并入状态码错误消息。

---

## P1（重要缺陷：上下文预算失控 / 归因失真 / 前端明文残留与 409 假冲突 / 每帧内存分配）

### P1-1 批量多查询缺少单条结果预算，且 `WebSearchToolResult.truncated` 恒为 `false`

- **位置**：`src/chat/tools/web_search/mod.rs:270-287`、`src/chat/tools/web_search/client.rs:196-205`
- **关键点**：
  - `format_outcomes` 对每条查询输出无字符上限，`client.rs` 将 `truncated` 硬编码为 `false`。4 条查询并发返回长正文时，下游 64 KiB 回放预算从末尾一刀切，导致排在后面的查询结果整段丢失；且卡片标题栏的 `results truncated` 与 `ToolSummary.truncated` 形同虚设。
- **错误原因**：
  - 格式化阶段未按单条查询分配字符预算，也未计算真实的 `truncated` 布尔值。
- **修复方案**：
  - 新增 `pub const MAX_QUERY_OUTPUT_CHARS: usize = 12_000`；在 `format_outcomes` 中对超出上限的单条结果裁剪并追加 `\n[...truncated]`，返回 `(text, truncated)` 并写入 `WebSearchToolResult.truncated`。

---

### P1-2 批量查询混合引擎（Exa + Tavily）时结果归因失真

- **位置**：`src/chat/tools/web_search/client.rs:190-195`、`src/chat/tools/output.rs:698-707`
- **关键点**：
  - 当同一批次中部分查询命中 `Exa`、部分回退命中 `Tavily` 时，`WebSearchToolResult.provider` 只取首个成功查询的引擎，抹掉了实际参与的第二个引擎；同时 `outcomes` 在 `WebSearchToolResult` 中重复克隆了一份完整的 `output` 字符串却从未被外部读取。
- **错误原因**：
  - 聚合时仅用 `find_map` 取第一个 `provider`，未检测多引擎混合命中。
- **修复方案**：
  - 为 `WebSearchToolResult` 提供 `provider_label(&self) -> &'static str`（单一引擎返回 `"exa"` / `"tavily"`，混合命中返回 `"exa+tavily"`），`output.rs` 的 `to_result_json` 使用 `result.provider_label()` 写入 `"provider"` 字段，TUI 卡片自动展示 `exa+tavily`。

---

### P1-3 设置页密码框明文状态残留，且保存 `General` 后引发 `Models` 页 409 假冲突

- **位置**：`assets/settings.js:23-35, 356-411`、`src/settings/service/models/mod.rs:37-40`
- **关键点**：
  1. 在 `General` 页点击小眼睛显示 Key 明文后，执行 Save、Reset、Reload 或切换 Tab 时，输入框仍停留在 `type="text"` 明文态，未复位为 `password`。
  2. `Models` 页的 `revision_of` 对整个 `AiSettings`（含 `web_search`）计算哈希；在 `General` 页修改并保存 Web Search Key 后，若切回已打开的 `Models` 页保存模型配置，会触发 `409 Conflict`。
- **错误原因**：
  - 前端未在数据重填/重置/切页时收回明文状态；后端 `revision_of` 把非模型字段 `web_search` 算进了模型乐观锁哈希。
- **修复方案**：
  1. 在 `assets/settings.js` 新增 `resetWebSearchReveals()`，在 `applyGeneralData`、`resetGeneral` 及 `showTab` 切换时强制复位为 `type="password"` 并移除 `.revealed`。
  2. 在 `src/settings/service/models/mod.rs` 的 `revision_of` 中仅对模型相关字段（`default_model`、`providers`、`stream_idle_timeout_ms`、`extra`）计算哈希，彻底隔离 `web_search` 与 `Models` 乐观锁；并补充 `Models` 保存保留 `web_search` 的回归测试。

---

### P1-4 TUI 历史缓存每帧对 `WebSearchDetailView.result` 执行 `join("\n")` 堆分配与全量哈希

- **位置**：`src/ui/main_content/lx/history/cache.rs:641-655`、`src/app/state/lx/chat/tool_view/web_search.rs:99-104`
- **关键点**：
  - `cache.rs` 每帧计算签名时调用 `view.result.join("\n")`，产生不必要的堆分配与大文本遍历；且 `WebSearchDetailView::from_parts` 提取前 40 行时未限制单行最大字符数，渲染层 `sanitize_result_line` 超 8000 字符截断时也未追加 `…`。
- **错误原因**：
  - 未沿用 `cache.rs` 既有的零分配 `write_edges` 采样哈希模式。
- **修复方案**：
  1. 在 `WebSearchDetailView::from_parts` 中对每行限制 `MAX_RESULT_LINE_CHARS`（8000 字符，超限追加 `…`），渲染层 `sanitize_result_line` 超限同步追加 `…`。
  2. 在 `cache.rs` 中改为遍历 `view.result` 逐行调用 `hash.write_usize(line.len())` 与 `hash.write_edges(line)`，零堆分配完成指纹计算。

---

## P2（健壮性与可维护性改进）

### P2-1 参数派生详情视图（`Mcp` / `Subagent` / `WebSearch`）结算保活逻辑重复且无单测

- **位置**：`src/app/update/chat/process.rs:52-63`、`src/app/update/chat/subagent.rs:293-301`、`src/app/state/lx/chat/tool_view.rs`
- **关键点**：
  - 实时流中 `web_search` 失败或取消时，`detail_of` 返回 `None`，依赖 `process.rs` 与 `subagent.rs` 两处硬编码的 `matches!` 保留参数阶段生成的 `WebSearchDetailView`，且缺少针对该保活分支的单元测试。
- **修复方案**：
  - 在 `ToolDetailView` 上定义 `pub const fn preserves_on_empty_settle(&self) -> bool`，统一替换两处硬编码 `matches!`，并新增单元测试验证 `settle_tool_detail` 在收到 `None` 时保留 `WebSearchDetailView`。

---

### P2-2 `parse_arguments` 去重前校验条数上限、取消信号前置短路与 `reqwest::Client` 复用

- **位置**：`src/chat/tools/web_search/mod.rs:183-195`、`src/chat/tools/web_search/client.rs:118-142`
- **关键点**：
  1. `parse_arguments` 在去重前检查 `raw_queries.len() > MAX_QUERIES`，导致含重复项但去重后 `<= 4` 条的合法请求被拒。
  2. `execute_with_endpoints` 在检查 `cancel` 前已 `spawn` 异步任务，且每次调用都重复构建 `reqwest::Client`。
- **修复方案**：
  1. 将 `MAX_QUERIES`（4 条）校验移至去重之后（去重前保留 `MAX_RAW_QUERIES = 16` 防御上限）。
  2. 在 `execute_with_endpoints` 入口处先检查 `*cancel.borrow()` 立即短路返回 `Cancelled`。
  3. 使用 `OnceLock<reqwest::Client>` 复用全局 HTTP 客户端连接池。

---

### P2-3 `output.rs` 中 `to_summary` 误借用 `ToolSummary.path` 存放查询词

- **位置**：`src/chat/tools/output.rs:427-430, 566-571`
- **关键点**：
  - `to_summary` 将首个查询词填入 `ToolSummary.path`，而 `ToolCallView::relativize_paths` 会把 `summary.path` 当作工作区文件路径按 `project_root` 截断改写。
- **修复方案**：
  - `to_summary` 对 `ToolExecutionResult::WebSearch` 设置 `path: None`，仅保留 `matches: Some(result.queries.len())` 与 `truncated: result.truncated`。

---

## P3（代码规范、文件体积与文档同步）

### P3-1 文件行数贴近规范阈值与重复辅助函数清理

- **位置**：`src/event/chat/mod.rs`（999 行）、`src/settings/service/tests.rs`（1248 行）、`src/settings/service/handlers.rs` & `models/mod.rs`
- **关键点**：
  - `src/event/chat/mod.rs` 达 999 行（生产代码上限 1000 行，余量仅 1 行）；`src/settings/service/tests.rs` 达 1248 行（超过测试文件 1000 行建议值）；`ai_path` 在 `handlers.rs` 与 `models/mod.rs` 中重复定义。
- **修复方案**：
  1. 将 `src/settings/service/tests.rs` 中的 Web Search 测试拆分为独立测试子模块 `src/settings/service/tests_web_search.rs`，使主测试文件回到 ~1050 行以内。
  2. 在 `src/settings/service.rs` 中提取共享的 `pub(super) fn ai_path(state: &ServiceState) -> PathBuf`，删除 `handlers.rs` 与 `models/mod.rs` 的重复实现。
  3. 收敛精简 `src/event/chat/mod.rs` 使其降至 980 行以内，并更新 `src/config/ai/mod.rs` 顶部注释使其与实际行为一致。

---

### P3-2 同步更新 `design.md` 与 `task.md`

- **位置**：`docs/feature/lx-web-search/design.md`、`docs/feature/lx-web-search/task.md`
- **关键点**：
  - 同步更新 `design.md` 中的单条查询裁剪预算 `MAX_QUERY_OUTPUT_CHARS`、混合引擎标识 `exa+tavily`、`PUT /api/config` 原子校验顺序、实时结算 `preserves_on_empty_settle` 保活机制及 §7 真实文件行数表；在 `task.md` 中新增 Stage 5（Review 修复项 T5.1–T5.4）并记录验证结果。
