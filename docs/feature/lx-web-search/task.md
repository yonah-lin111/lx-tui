# lx `web_search` 联网搜索任务拆解

> 状态：已在 `.worktrees/lx-web-search`（分支 `feat/lx-web-search`）完成实现、Code Review（`review.md`）与 P0–P3 全部修复，待用户确认合并
> 前置：`docs/feature/lx-web-search/design.md` 与 `docs/feature/lx-web-search/review.md`
> 纪律：按阶段自底向上实施，每完成一个阶段运行对应的精确测试（`rtk cargo test <模块>`），全部完成后执行 `cargo fmt`、`cargo clippy --all-targets -- -D warnings`、`cargo test`。

---

## 0. 审核与基线

- [x] **T0.1** 用户审核确认 `docs/feature/lx-web-search/design.md` 与 `docs/feature/lx-web-search/task.md`。
- [x] **T0.2** 核对当前分支工作树状态与四大规范文档（`project-directory-structure.md`、`code-writing-standards.md`、`tui-design-requirements.md`、`html-assets.md`）。

---

## 1. 配置层与 Web 设置页（`WebSearchConfig` + `assets/settings.*`）

### T1.1 `AiSettings` 新增 `WebSearchConfig` 并同步 `lx-agent` 默认 Key

- [x] **文件**：
  - `src/config/ai/mod.rs`（499 行）
  - `src/config/ai/tests.rs`（533 行）
  - `~/.lx-t-dev/config/ai.json`
- [x] **关键实现**：
  1. 在 `src/config/ai/mod.rs` 中定义常量 `DEFAULT_EXA_API_KEY` 与 `DEFAULT_TAVILY_API_KEY`。
  2. 定义 `WebSearchConfig`（`#[serde(rename_all = "camelCase")]`），包含 `exa_api_key: String`、`tavily_api_key: String` 与 `#[serde(flatten)] pub extra: BTreeMap<String, Value>`，`Default` 返回上述两个默认 Key；当 `ai.json` 中缺失 `webSearch` 节点时反序列化自动取默认值，当用户显式保存为空字符串 `""` 时原样保留空字符串。
  3. 在 `AiSettings` 中增加 `#[serde(default, rename = "webSearch")] pub web_search: WebSearchConfig`，并在 `AiSettings::empty()` 中初始化 `web_search: WebSearchConfig::default()`。
  4. `save_ai` 使用带进程 ID + 原子序号的唯一临时文件 `ai.json.<pid>.<seq>.tmp` 落盘。
  5. 将 `"webSearch": { "exaApiKey": "...", "tavilyApiKey": "..." }` 写入 `~/.lx-t-dev/config/ai.json`。
- [x] **验证**：`rtk cargo test config::ai`（23 passed）

### T1.2 设置服务路由对接（`GET /api/state` 与 `PUT /api/config`）

- [x] **文件**：
  - `src/settings/service.rs`（451 行）
  - `src/settings/service/handlers.rs`（940 行）
  - `src/settings/service/models/mod.rs`（525 行）
  - `src/settings/service/models/tests.rs`（864 行）
  - `src/settings/service/tests_web_search.rs`（新建，335 行）
- [x] **关键实现**：
  1. 在 `src/settings/service.rs` 中提取共享 `pub(super) fn ai_path(state: &ServiceState) -> PathBuf`。
  2. `state_json`：在返回 JSON 中增加 `"web_search":{"exa_api_key":...,"tavily_api_key":...}`。
  3. `put_config`：先执行纯校验（区分 `web_search` 缺失、对象、类型错误 422，拦截含控制字符的 Key）与 `ai.json` 预加载合并（若磁盘 `ai.json` 损坏则告警并以默认基底修复），校验通过后依次写入 `ai.json` 与 `settings.json`，最后统一投递 `SettingsEvent::ConfigChanged` 与 `SettingsEvent::ModelsChanged`。
  4. `models::revision_of`：仅对模型相关字段计算乐观锁哈希，使 General 页修改 `web_search` 不会导致已打开的 Models 页产生假 409 冲突。
- [x] **验证**：`rtk cargo test settings::service`（38 passed）

### T1.3 浏览器设置页 `General -> Web Search` 表单与脚本

- [x] **文件**：
  - `assets/settings.html`（432 行）
  - `assets/settings.js`（978 行）
- [x] **关键实现**：
  1. 在 `assets/settings.html` 的 `#view-general` 中，`Layout` `<fieldset>` 下方新增 `<fieldset><legend>Web Search</legend>`，包含 `#web_search_exa_api_key`（带 `#exa-key-reveal` 按钮）和 `#web_search_tavily_api_key`（带 `#tavily-key-reveal` 按钮）。
  2. 在 `assets/settings.js` 中新增 `WEB_SEARCH_KEYS`，`generalValues()`、`resetGeneral()`、`applyGeneralData(data)`、`saveGeneral()` 同步处理这两个字段，绑定密码显隐切换事件，并在保存/重置/切换 Tab 时通过 `resetWebSearchReveals()` 自动收回明文状态。
- [x] **验证**：`rtk cargo test settings::service`（38 passed）

---

## 2. `web_search` 核心执行层与工具规格（`src/chat/tools/web_search/`）

### T2.1 新建 `src/chat/tools/web_search/mod.rs` 与 `client.rs`

- [x] **文件**：
  - `src/chat/tools/web_search/mod.rs`（新建，352 行）
  - `src/chat/tools/web_search/client.rs`（新建，661 行）
  - `src/chat/tools/web_search/tests.rs`（新建，864 行）
- [x] **关键实现**：
  1. `mod.rs`：定义常量与核心类型，实现严格白名单 `parse_arguments`（支持 `query` 与 `queries` 合并去重、去重后校验 `1..=4` 条、上限 500 字符、控制字符拦截、`num_results`/`numResults` `1..=10`、`type` `auto`/`fast`/`deep`）、单条查询预算裁剪 `MAX_QUERY_OUTPUT_CHARS = 12_000`（超限置 `truncated = true`）与 `provider_label`（支持 `"exa+tavily"`）。
  2. `client.rs`：实现 `OnceLock<reqwest::Client>` 连接池复用、`search_with_exa`（JSON 与 `data: ` SSE 双格式解析，URL 参数 `exaApiKey` 严格脱敏）、`search_with_tavily`（空 Key 短路、Bearer 认证与编号结果格式化）、匿名 401/403 `AtomicBool` 持续熔断、非 2xx 响应体脱敏错误摘要提取、25s 超时、2 MiB 响应上限、`JoinSet` 批量并发与 `*cancel.borrow()` 前置短路 + `tokio::select!` 实时取消。
  3. `tests.rs`：24 个单元测试，通过本地 `TcpListener` 回环假服务验证 Exa JSON/SSE、Exa→Tavily 回退、带坏 Key 时匿名 403 熔断保持、全失败脱敏错误汇总与全超时错误码、单条预算截断、混合引擎 `exa+tavily`、预取消零请求与参数边界。
- [x] **验证**：`rtk cargo test chat::tools`（309 passed）

### T2.2 接入工具注册表、输出格式化、角色白名单与系统提示词

- [x] **文件**：
  - `src/chat/tools/types.rs`（541 行）
  - `src/chat/tools/mod.rs`（201 行）
  - `src/chat/tools/schema.rs`（583 行）
  - `src/chat/tools/output.rs`（791 行）& `src/chat/tools/output/tests.rs`（927 行）
  - `src/chat/agent.rs`（800 行）
  - `src/chat/prompt.rs`（836 行）
  - `src/chat/agent_loop/executor.rs`（582 行）
  - `src/event/chat/mod.rs`（971 行）& `src/event/chat/subagent/runner.rs`（896 行）
- [x] **关键实现**：
  1. `types.rs` / `mod.rs` / `schema.rs` / `output.rs`：注册 `web_search`（`TOOLS` 16→17），实现 `format_for_model`、`to_summary`（`path: None` 避免污染工作区路径归一化）、`to_result_json`（使用 `provider_label()`）与 `shrink_json_text`。
  2. `agent.rs`：`MAIN_TOOLS` (16→17)、`EXPLORER_TOOLS` (2→3)、`EXECUTOR_TOOLS` (6→7) 全部开放 `web_search`，更新 `EXPLORER_PROMPT` 与 `EXECUTOR_PROMPT`。
  3. `prompt.rs`：`PROMPT_VERSION` 升至 `"lx-8"`，`LX_SYSTEM_PROMPT` 新增 `## Web search (\`web_search\`)` 与 `[title](url)` 就近内联引用规范（通过 UTF-16 `< 2000` 码元与 12 KiB 预算测试）。
  4. `executor.rs` & `event/chat/`：`AgentTools` 新增 `with_web_search_keys`，在主 Agent 与子 Agent 启动时注入 `state.models.settings.web_search` 的两把 Key。
- [x] **验证**：`rtk cargo test chat::prompt`（18 passed）、`rtk cargo test chat::agent`（54 passed）

---

## 3. TUI 状态投影、工具卡片与底部状态栏（`src/app/` & `src/ui/`）

### T3.1 新增 `WebSearchDetailView` 与状态流转接入

- [x] **文件**：
  - `src/app/state/lx/chat/tool_view/web_search.rs`（新建，175 行）
  - `src/app/state/lx/chat/tool_view/web_search/tests.rs`（新建，107 行）
  - `src/app/state/lx/chat/tool_view.rs`（885 行）
  - `src/app/state/lx/chat/steps.rs`（963 行）
  - `src/app/update/chat/process.rs`（763 行）& `src/app/update/chat/subagent.rs`（868 行）
  - `src/event/chat/tool_calls.rs`（499 行）
  - `src/app/update/chat/view.rs`（166 行）& `src/app/update/chat/view/tests.rs`（771 行）
- [x] **关键实现**：
  1. 实现 `WebSearchDetailView::from_arguments`、`from_parts`（单行按 `MAX_RESULT_LINE_CHARS = 8_000` 截断补 `…`）、`from_tool`，挂载 `ToolDetailView::WebSearch(WebSearchDetailView)`。
  2. 在 `ToolDetailView` 上新增 `preserves_on_empty_settle()`，统一 `process.rs` 与 `subagent.rs` 在实时流失败/取消时对参数派生视图的保活逻辑。
- [x] **验证**：`rtk cargo test app::state::lx`（188 passed）、`rtk cargo test app::update::chat`（290 passed）

### T3.2 新建 `src/ui/main_content/lx/history/tool/web_search.rs` 与 `chat_bar.rs` 联动

- [x] **文件**：
  - `src/ui/text/agent.rs` & `src/ui/text/chat.rs`
  - `src/ui/main_content/lx/history/tool/web_search.rs`（新建，273 行）
  - `src/ui/main_content/lx/history/tool/web_search/tests.rs`（新建，320 行）
  - `src/ui/main_content/lx/history/tool.rs`（832 行）
  - `src/ui/main_content/lx/history/cache.rs`（856 行）
  - `src/ui/main_content/lx/chat_bar.rs`（600 行）
- [x] **关键实现**：
  1. 实现 `web_search` 树形工具卡片渲染（`■ Web search · 2 queries · exa · 840ms`、`├─` 查询块、`└─` 压暗 Markdown 结果块，两块独立点击折叠）。
  2. `cache.rs` 中对 `WebSearchDetailView.result` 采用逐行 `write_edges` 零堆分配有界哈希，并消除 `Mcp`/`Search`/`WebSearch` 的 tag 冲突。
  3. `chat_bar.rs` 新增 `ActivityLabel::SearchingWeb`，在 `web_search` 处于 Pending/Running 时显示 `Searching the web: "<query>"`。
- [x] **验证**：`rtk cargo test ui::main_content::lx`（542 passed）

---

## 4. 全量验证与交付

- [x] **T4.1** 运行 `rtk cargo fmt`。
- [x] **T4.2** 运行 `rtk cargo clippy --all-targets -- -D warnings`，零警告通过。
- [x] **T4.3** 运行受影响模块全部单元测试与集成测试通过。
- [x] **T4.4** 核查所有新增与修改的生产代码文件行数均低于 1000 行（`src/event/chat/mod.rs` 收敛至 971 行；`src/settings/service/tests_web_search.rs` 独立拆分 335 行）。
- [x] **T4.5** 在本文件末尾回填 `## 实施结果与偏差（回填）`。

---

## 5. Code Review 修复阶段（对照 `review.md` P0–P3）

- [x] **T5.1（P0 修复）**：
  - **P0-1**：修复 `client.rs` 中配置非空 Key 时每条查询无条件清零 `EXA_ANON_BLOCKED` 的熔断失效问题；Tavily 空 Key 直接短路不再发必败请求。
  - **P0-2**：修复 `handlers.rs::put_config` 写操作与校验顺序倒置问题（改为先校验/预合并，再写 `ai.json` 与 `settings.json`，最后发事件），拦截非对象 `web_search` 与含控制字符 Key（返回 422 且不污染 `settings.json`）；`save_ai` 改用带 pid+seq 的唯一 `.tmp` 文件。
  - **P0-3**：修复全引擎失败时丢弃真实错误原因的问题（拼接去重后的脱敏错误摘要与非 2xx 响应体错误字段，全超时返回 `ToolErrorCode::Timeout`）。
- [x] **T5.2（P1 修复）**：
  - **P1-1**：新增单条查询输出上限 `MAX_QUERY_OUTPUT_CHARS = 12_000`，超限裁剪并真实回填 `WebSearchToolResult.truncated`。
  - **P1-2**：新增 `WebSearchToolResult::provider_label()`，批次内混合命中时输出 `"exa+tavily"`。
  - **P1-3**：`assets/settings.js` 新增 `resetWebSearchReveals()` 在保存/重置/切换 Tab 时自动恢复 `type="password"`；`models::revision_of` 排除 `web_search`，消除跨 Tab 保存假 409 冲突。
  - **P1-4**：`cache.rs` 改用逐行 `write_edges` 零分配哈希；`WebSearchDetailView::from_parts` 增加单行 `8_000` 字符截断补 `…`。
- [x] **T5.3（P2 修复）**：
  - **P2-1**：新增 `ToolDetailView::preserves_on_empty_settle()` 统一实时流失败/取消时的视图保活，并补齐回归单测。
  - **P2-2**：`parse_arguments` 改为去重后检查 `MAX_QUERIES = 4`；`execute_with_endpoints` 入口前置检查 `*cancel.borrow()`；`OnceLock<reqwest::Client>` 复用连接池。
  - **P2-3**：`output.rs::to_summary` 对 `WebSearch` 置 `path: None`，防止查询词被当作文件路径改写。
- [x] **T5.4（P3 修复）**：
  - **P3-1**：拆分 `src/settings/service/tests_web_search.rs`（335 行），在 `service.rs` 共享 `ai_path`，将 `src/event/chat/mod.rs` 从 999 行收敛至 971 行。
  - **P3-2**：同步更新 `design.md`、`task.md` 与 `review.md`。
