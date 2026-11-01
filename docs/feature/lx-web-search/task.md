# lx `web_search` 联网搜索任务拆解

> 状态：已在 `.worktrees/lx-web-search`（分支 `feat/lx-web-search`）完成实现与测试，待用户确认合并
> 前置：`docs/feature/lx-web-search/design.md` 已获用户确认。
> 纪律：按阶段自底向上实施，每完成一个阶段运行对应的精确测试（`rtk cargo test <模块>`），全部完成后执行 `cargo fmt`、`cargo clippy --all-targets -- -D warnings`、`cargo test`。

---

## 0. 审核与基线

- [x] **T0.1** 用户审核确认 `docs/feature/lx-web-search/design.md` 与 `docs/feature/lx-web-search/task.md`。
- [x] **T0.2** 核对当前分支工作树状态与四大规范文档（`project-directory-structure.md`、`code-writing-standards.md`、`tui-design-requirements.md`、`html-assets.md`）。

---

## 1. 配置层与 Web 设置页（`WebSearchConfig` + `assets/settings.*`）

### T1.1 `AiSettings` 新增 `WebSearchConfig` 并同步 `lx-agent` 默认 Key

- [x] **文件**：
  - `src/config/ai/mod.rs`
  - `src/config/ai/tests.rs`
  - `~/.lx-t-dev/config/ai.json`
- [x] **关键实现**：
  1. 在 `src/config/ai/mod.rs` 中定义常量 `DEFAULT_EXA_API_KEY` 与 `DEFAULT_TAVILY_API_KEY`。
  2. 定义 `WebSearchConfig`（`#[serde(rename_all = "camelCase")]`），包含 `exa_api_key: String` 与 `tavily_api_key: String`，`Default` 返回上述两个默认 Key；当 `ai.json` 中缺失 `webSearch` 节点时反序列化自动取默认值，当用户显式保存为空字符串 `""` 时原样保留空字符串。
  3. 在 `AiSettings` 中增加 `#[serde(default, rename = "webSearch")] pub web_search: WebSearchConfig`，并在 `AiSettings::empty()` 中初始化 `web_search: WebSearchConfig::default()`。
  4. 将 `"webSearch": { "exaApiKey": "...", "tavilyApiKey": "..." }` 写入 `~/.lx-t-dev/config/ai.json`。
- [x] **验证**：`rtk cargo test config::ai`（22 passed）

### T1.2 设置服务路由对接（`GET /api/state` 与 `PUT /api/config`）

- [x] **文件**：
  - `src/settings/service/handlers.rs`
  - `src/settings/service/tests.rs`
- [x] **关键实现**：
  1. 在 `handlers.rs` 中实现 `ai_path(state)` 与 `load_ai_settings` / `merge_web_search`。
  2. `state_json`：在返回 JSON 中增加 `"web_search":{"exa_api_key":...,"tavily_api_key":...}`。
  3. `put_config`：保存 `config` 到 `settings.json` 并投递 `SettingsEvent::ConfigChanged(config)`；当请求体携带 `"web_search"` 时，读取现有 `AiSettings`（保留全部 providers / models），更新 `settings.web_search` 后调用 `crate::config::ai::save_ai(&ai_path, &settings)`，并投递 `SettingsEvent::ModelsChanged(Box::new(settings))`。
- [x] **验证**：`rtk cargo test settings::service`（34 passed）

### T1.3 浏览器设置页 `General -> Web Search` 表单与脚本

- [x] **文件**：
  - `assets/settings.html`
  - `assets/settings.js`
  - `src/settings/service/tests.rs`
- [x] **关键实现**：
  1. 在 `assets/settings.html` 的 `#view-general` 中，`Layout` `<fieldset>` 下方新增 `<fieldset><legend>Web Search</legend>`，包含 `#web_search_exa_api_key`（带 `#exa-key-reveal` 按钮）和 `#web_search_tavily_api_key`（带 `#tavily-key-reveal` 按钮）。
  2. 在 `assets/settings.js` 中新增 `WEB_SEARCH_KEYS`，`generalValues()`、`resetGeneral()`、`applyGeneralData(data)`、`saveGeneral()` 同步处理这两个字段，并绑定密码显隐切换事件。
  3. 在 `src/settings/service/tests.rs` 中补充页面结构、状态获取与保存不丢模型配置的测试断言。
- [x] **验证**：`rtk cargo test settings::service`（34 passed）

---

## 2. `web_search` 核心执行层与工具规格（`src/chat/tools/web_search/`）

### T2.1 新建 `src/chat/tools/web_search/mod.rs` 与 `client.rs`

- [x] **文件**：
  - `src/chat/tools/web_search/mod.rs`（新建，298 行）
  - `src/chat/tools/web_search/client.rs`（新建，552 行）
  - `src/chat/tools/web_search/tests.rs`（新建，604 行）
- [x] **关键实现**：
  1. `mod.rs`：定义常量与核心类型，实现严格白名单 `parse_arguments`（支持 `query` 与 `queries` 合并去重、1..=4 条、上限 500 字符、控制字符拦截、`num_results`/`numResults` 1..=10、`type` `auto`/`fast`/`deep`）与 `format_outcomes`。
  2. `client.rs`：实现 `search_with_exa`（JSON 与 `data: ` SSE 双格式解析，URL 参数 `exaApiKey` 日志脱敏）、`search_with_tavily`（Bearer 认证与编号结果格式化）、匿名 401/403 `AtomicBool` 熔断、25s 超时、2 MiB 响应上限、`JoinSet` 批量并发与 `tokio::select!` 监听 `cancel` 立即中断。
  3. `tests.rs`：18 个单元测试，通过本地 `TcpListener` 回环假服务验证 Exa JSON/SSE、Exa→Tavily 回退、匿名 401 熔断、批量部分成功保留与 `cancel` 信号中断。
- [x] **验证**：`rtk cargo test chat::tools`（300 passed）

### T2.2 接入工具注册表、输出格式化、角色白名单与系统提示词

- [x] **文件**：
  - `src/chat/tools/types.rs`
  - `src/chat/tools/mod.rs`
  - `src/chat/tools/schema.rs`
  - `src/chat/tools/output.rs`
  - `src/chat/agent.rs`
  - `src/chat/prompt.rs`
  - `src/chat/agent_loop/executor.rs`
  - `src/event/chat/mod.rs` & `src/event/chat/subagent/runner.rs`
- [x] **关键实现**：
  1. `types.rs` / `mod.rs` / `schema.rs` / `output.rs`：注册 `web_search`（`TOOLS` 16→17），实现 `format_for_model`、`to_summary`、`to_result_json` 与 `shrink_json_text`。
  2. `agent.rs`：`MAIN_TOOLS` (16→17)、`EXPLORER_TOOLS` (2→3)、`EXECUTOR_TOOLS` (6→7) 全部开放 `web_search`，更新 `EXPLORER_PROMPT` 与 `EXECUTOR_PROMPT`。
  3. `prompt.rs`：`PROMPT_VERSION` 升至 `"lx-8"`，`LX_SYSTEM_PROMPT` 新增 `## Web search (\`web_search\`)` 与 `[title](url)` 就近内联引用规范（通过 UTF-16 `< 2000` 码元与 12 KiB 预算测试）。
  4. `executor.rs` & `event/chat/`：`AgentTools` 新增 `with_web_search_keys`，在主 Agent 与子 Agent 启动时注入 `state.models.settings.web_search` 的两把 Key。
- [x] **验证**：`rtk cargo test chat::prompt`（18 passed）、`rtk cargo test chat::agent`（54 passed）

---

## 3. TUI 状态投影、工具卡片与底部状态栏（`src/app/` & `src/ui/`）

### T3.1 新增 `WebSearchDetailView` 与状态流转接入

- [x] **文件**：
  - `src/app/state/lx/chat/tool_view/web_search.rs`（新建，165 行）
  - `src/app/state/lx/chat/tool_view/web_search/tests.rs`（新建，89 行）
  - `src/app/state/lx/chat/tool_view.rs`
  - `src/app/state/lx/chat/steps.rs`
  - `src/event/chat/tool_calls.rs`
  - `src/app/update/chat/view.rs` & `src/app/update/chat/view/tests.rs`
- [x] **关键实现**：
  1. 实现 `WebSearchDetailView::from_arguments`、`from_parts`、`from_tool`，挂载 `ToolDetailView::WebSearch(WebSearchDetailView)`。
  2. `ToolCallView::accept_arguments`、`tool_calls::detail_of`、`view::tool_view_of` 完整接入，支持运行中立即展示查询词及无 `result_json`（中断/取消）时回退 `arguments_json`。
- [x] **验证**：`rtk cargo test app::state::lx`（187 passed）、`rtk cargo test app::update::chat`（289 passed）

### T3.2 新建 `src/ui/main_content/lx/history/tool/web_search.rs` 与 `chat_bar.rs` 联动

- [x] **文件**：
  - `src/ui/text/agent.rs` & `src/ui/text/chat.rs`
  - `src/ui/main_content/lx/history/tool/web_search.rs`（新建，271 行）
  - `src/ui/main_content/lx/history/tool/web_search/tests.rs`（新建，261 行）
  - `src/ui/main_content/lx/history/tool.rs`
  - `src/ui/main_content/lx/chat_bar.rs`
- [x] **关键实现**：
  1. 实现 `web_search` 树形工具卡片渲染（`■ Web search · 2 queries · exa · 840ms`、`├─` 查询块、`└─` 压暗 Markdown 结果块，两块独立点击折叠）。
  2. `chat_bar.rs` 新增 `ActivityLabel::SearchingWeb`，在 `web_search` 处于 Pending/Running 时显示 `Searching the web: "<query>"`。
- [x] **验证**：`rtk cargo test ui::main_content::lx`（540 passed）

---

## 4. 全量验证与交付

- [x] **T4.1** 运行 `rtk cargo fmt`。
- [x] **T4.2** 运行 `rtk cargo clippy --all-targets -- -D warnings`，零警告通过。
- [x] **T4.3** 运行受影响模块全部单元测试与集成测试通过。
- [x] **T4.4** 核查所有新增与修改的生产代码文件行数均低于 1000 行（新增子模块均控制在 165~552 行，且测试文件严格分离至 `tests.rs`）。
- [x] **T4.5** 在本文件末尾回填 `## 实施结果与偏差（回填）`。

---

## 实施结果与偏差（回填）

1. **实施分支与工作区**：在 `.worktrees/lx-web-search`（分支 `feat/lx-web-search`）完成全部编码与测试，主工作区保持干净，并将 `lx-agent` 的 Exa / Tavily Key 同步写入 `~/.lx-t-dev/config/ai.json` 与 `WebSearchConfig::default()`。
2. **核心结构改动**：
   - 配置与设置页：`src/config/ai/mod.rs`（`WebSearchConfig`）、`src/settings/service/handlers.rs`（`GET /api/state` 与 `PUT /api/config` 读写 `ai.webSearch` 并投递 `ModelsChanged`）、`assets/settings.{html,js}`（General 页 `Web Search` 密码框与明文显隐切换）。
   - 工具执行与提示词：`src/chat/tools/web_search/{mod.rs,client.rs,tests.rs}`（Exa MCP + Tavily REST 双引擎回退、匿名 401/403 熔断、最多 4 条并发查询、25s 超时、`Esc` 立即取消）、`src/chat/tools/{schema,types,output,mod}.rs`、`src/chat/agent.rs`（`Main`/`Explorer`/`Executor` 全部开放）、`src/chat/prompt.rs`（`PROMPT_VERSION = "lx-8"`）。
   - TUI 视图与渲染：`src/app/state/lx/chat/tool_view/web_search.rs`（`WebSearchDetailView`）、`src/ui/main_content/lx/history/tool/web_search.rs`（独立折叠树形卡片 + 压暗 Markdown 结果）、`src/ui/main_content/lx/chat_bar.rs`（`Searching the web: "<query>"`）。
3. **与原始设计的微小偏差**：
   - 耗时格式化直接复用了已有的 `ui::text::agent::duration_label(u64)`（与 MCP 卡片完全一致），未重复造 `text::format_duration_ms`。
   - 为满足 `history/cache.rs` 穷尽匹配及 `process.rs`/`subagent.rs` 对卡片详情保留的统一约束，同步在 `cache.rs`、`process.rs`、`subagent.rs` 中补齐了 `ToolDetailView::WebSearch` 分支。
