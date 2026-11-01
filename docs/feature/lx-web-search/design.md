# lx `web_search` 联网搜索功能设计

> 状态：已在工作区 `.worktrees/lx-web-search`（分支 `feat/lx-web-search`）完成实现与测试，待用户确认合并
> 范围：`src/config/ai/`、`src/settings/service/`、`assets/settings.{html,js}`、`src/chat/tools/web_search/`、`src/chat/agent.rs`、`src/chat/prompt.rs`、`src/chat/agent_loop/`、`src/app/state/lx/chat/`、`src/app/update/chat/`、`src/event/chat/`、`src/ui/main_content/lx/`
> 参考：
> - `codex-main`（`/Users/yonah/projects/agent/codex-main/codex-rs/ext/web-search/web_run_description.md`、`codex-rs/ext/web-search/src/tool.rs`、`codex-rs/tui/src/history_cell/search.rs`、`codex-rs/protocol/src/models.rs`）
> - `lx-agent`（`/Users/yonah/projects/agent/lx-agent/src/main/agent/tools/webSearch.ts`、`~/.lx/config/ai.json`）

---

## 1. 目标与已确认边界

### 1.1 核心目标

为 `lx-tui` 新增内置只读联网搜索工具 **`web_search`**，融合 `codex-main` 的模型侧决策边界、批量查询能力、内联引用规范与 TUI 状态表达，以及 `lx-agent` 的 **Exa MCP + Tavily REST** 双引擎回退执行链路，并在本地设置页（`assets/settings.html`）支持配置这两个搜索引擎的 API Key，同时将 `lx-agent` 的两把 Key 同步至本项目配置与默认值中，实现零配置开箱即用。

### 1.2 Grill Me 已确认的关键决策清单

| # | 决策维度 | 已确认结论 | 依据与取舍说明 |
|---|----------|------------|----------------|
| 1 | **工具范围与参数 Schema** | 新增单一内置工具 **`web_search`**（不引入 `webfetch` 及 OpenAI 私有网关的天气/股票/体育等子命令）；同时支持单条 `query` 与批量 `queries`（1~4 条并发检索），可选 `num_results`（1~10，默认 8）与 `type`（`"auto" \| "fast" \| "deep"`，默认 `"auto"`）。完整移植 `codex-main` 的搜索决策边界与 Markdown 内联引用规范。 | 兼顾 `codex-main` 单次调用并发检索多主题的高效性与 `lx-agent` Exa/Tavily 引擎的真实参数能力。 |
| 2 | **设置页位置与 API Key 落地** | 1. 在 `assets/settings.html` 的 **`General` 标签页**（`Layout` 下方）新增 `<fieldset><legend>Web Search</legend>`，提供 **Exa API key** 与 **Tavily API key** 两个带明文显隐切换（小眼睛按钮）的密码框；<br>2. 持久化在 `config/ai.json` 的 `ai.webSearch` 节点（`exaApiKey`、`tavilyApiKey`），与 `lx-agent` 100% 兼容；<br>3. 将 `lx-agent` 的两个 Key 既写入当前项目的配置文件，又作为 `WebSearchConfig::default()` 的内置默认初值。 | `General` 是打开设置页的首屏，无需为两个输入框单开空白 Tab；代码默认初值 + 配置文件双保险保证任何环境开箱即用，且用户在网页修改保存后以用户配置为准。 |
| 3 | **主/子 Agent 权限分配** | 开放给 **`Main`、`Explorer`、`Executor` 全部三个角色**（`MAIN_TOOLS` 16→17，`EXPLORER_TOOLS` 2→3，`EXECUTOR_TOOLS` 6→7），并在主/子 Agent 英文系统提示词中同步补充 `web_search` 的使用与引用规则，递增 `PROMPT_VERSION`（`lx-7` → `lx-8`）。 | `web_search` 是纯只读无副作用工具；允许 `explorer` 在独立上下文消化冗长网页结果可大幅节省主会话上下文，允许 `executor` 现场查证 API 签名可避免无意义中断。 |
| 4 | **TUI 历史卡片与状态栏展示** | 1. **历史卡片**：遵循 `lx-tui` 直角树规范（`tree::layout`），标题行运行中显示 `□ Web search · Searching…`（呼吸方块），完成态显示 `■ Web search · 2 queries · exa · 840ms`（单条省略 `1 query`，显示命中的引擎名与耗时）；`├─` 参数块显示查询词与非默认参数（独立可折叠）；`└─` 结果块默认折叠预览前 3 行（`… N more lines`），点击独立展开，内容采用思考块同款压暗 Markdown（`muted_markdown_lines`）；<br>2. **底部聊天栏（`chat_bar`）**：当 `web_search` 处于 `Pending`/`Running` 时，动态显示 `Searching the web: <query>`（对齐 `codex-main` 的实时状态摘要）。 | 保持 `lx-tui` 所有工具卡片的交互与配色一致性（方块标记、翡翠绿/橙黄/红语义色、`├─`/`└─` 独立折叠、结果压暗不抢正文焦点），同时让用户可随时点开核查搜索到的原始链接与摘要。 |
| 5 | **异步取消、双引擎回退与批量容错** | 1. **原生 async + `Esc` 立即取消**：在 `AgentTools::execute_mcp_cancellable` 中接入 `web_search`，通过 `tokio::select!` 监听轮次 `cancel: watch::Receiver<bool>`，按 `Esc` 立即中断 HTTP 请求并结算为 `Cancelled` / `Interrupted`；<br>2. **逐级回退**：单条查询按 `Exa (带 Key) → Exa (无 Key 匿名兜底，401/403 内存熔断) → Tavily (带 Key)` 顺序执行，单次请求超时 25 秒；<br>3. **批量查询部分成功保留**：`queries` 多条并发执行时，只要 ≥1 条成功即算工具执行成功（`Completed`），将成功结果与单条失败原因合并回传模型；仅当全部查询均失败时才返回 `ToolExecutionResult::Failed`。 | 网络 IO 不占用阻塞线程池；`Esc` 零延迟响应；Exa Key 额度耗尽或超时自动切 Tavily；批量检索不因单条冷门词超时而丢弃其余已搜出的有效资料。 |

### 1.3 非目标（本版不做）

- 不引入数据库 Schema 变更（不改 `src/db/migration.rs` 的 `SCHEMA_VERSION`，`web_search` 完全复用现有的 `agent_tool_call` 表与 `result_json` 列）；
- 不引入 `webfetch` 任意网页爬虫工具，也不引入 OpenAI 私有的 `finance`/`weather`/`sports`/`screenshot` 命令；
- 不新增第三方重依赖（直接复用项目已有的 `reqwest`、`tokio`、`serde`、`serde_json`）。

---

## 2. 参考实现对比与架构取舍

| 维度 | `codex-main` 实现 | `lx-agent` 实现 | `lx-tui` 本次设计取舍 |
|------|-------------------|-----------------|-----------------------|
| **模型暴露方式** | Hosted `type: "web_search"` 或 Standalone `web.run`（支持单次传多条 `search_query`） | 单一函数工具 `web_search`（参数 `query`, `numResults`, `type`） | 函数工具 `web_search`（兼容 OpenAI Chat / Responses / Anthropic / Google 四大传输层），参数支持 `query` 或 `queries`（最多 4 条）+ `num_results` + `type`。 |
| **搜索引擎后端** | OpenAI 内部私有网关 `SearchClient` | **Exa MCP**（`https://mcp.exa.ai/mcp`）优先，失败回退 **Tavily REST**（`https://api.tavily.com/search`） | 完全采用 `lx-agent` 的 **Exa → Tavily** 双引擎回退协议、超时（25s）、SSE/JSON 双格式解析与匿名 401/403 熔断机制。 |
| **API Key 存储** | 无（走 ChatGPT 账号 OAuth / OpenAI Key） | `~/.lx[-dev]/config/ai.json` 的 `ai.webSearch.{exaApiKey, tavilyApiKey}` | 存储在 `~/.lx-t[-dev]/config/ai.json` 的 `ai.webSearch.{exaApiKey, tavilyApiKey}`，并将 `lx-agent` 的两个 Key 作为默认初值与初始配置写入。 |
| **提示词与引用规范** | `web_run_description.md`：详尽的 `<situations_where_you_must_browse_the_internet>`、强制 `[title](url)` 就近引用、禁止文末堆砌链接、25 词直接引用上限 | 简短两句话工具描述 | 吸收 `codex-main` 的决策边界与 Markdown 就近引用规范（去除 OpenAI 品牌特定规则，适配 `lx` 身份与 UTF-16 系统提示预算）。 |
| **TUI 渲染** | `WebSearchCell`：`Searching the web for <q>` / `Searched the web for <q>` + 底部状态栏 `Web search: <q>` | React 组件 `AgentWebSearchBlock`：仅展示搜索词徽标 | 标题行 + `chat_bar` 吸收 `codex-main` 的实时状态语义；卡片主体遵循 `lx-tui` 的 `├─` 参数块 + `└─` 压暗 Markdown 可展开结果块。 |

---

## 3. 整体数据流与模块边界

```text
┌──────────────────────────────────────────────────────────────────────────────┐
│ 1. 配置与设置页链路                                                           │
│                                                                              │
│ ~/.lx-t[-dev]/config/ai.json (ai.webSearch.{exaApiKey, tavilyApiKey})        │
│   ▲                                      ▲                                   │
│   │ load_ai / save_ai                    │ Models PUT 保留 current.web_search│
│   ▼                                      │                                   │
│ src/config/ai/mod.rs (AiSettings.web_search: WebSearchConfig, 含内置默认 Key) │
│   ▲                                                                          │
│   │ GET /api/state & PUT /api/config (同时读写 settings.json 与 ai.json)     │
│   ▼                                                                          │
│ assets/settings.html (General -> Web Search fieldset) + assets/settings.js   │
└──────────────────────────────────────────────────────────────────────────────┘

┌──────────────────────────────────────────────────────────────────────────────┐
│ 2. Agent 轮次与工具执行链路                                                   │
│                                                                              │
│ Model ──(tool_call: "web_search", args)──► agent_loop::run_agent_turn        │
│   │                                                                          │
│   ├─► 1. host.begin_tool(call)                                               │
│   │      ├─ DB: INSERT agent_tool_call (status='pending')                    │
│   │      ├─ AppState: ToolCallView::accept_arguments -> WebSearchDetailView  │
│   │      └─ UI 帧渲染后推进 status='running'，chat_bar 显示 Searching the web│
│   │                                                                          │
│   ├─► 2. executor.execute_mcp_cancellable (识别 "web_search")                │
│   │      └─ chat::tools::web_search::execute(args, config, cancel)           │
│   │         ├─ 解析并校验 query / queries (1..=4), num_results (1..=10), type │
│   │         ├─ 并发执行每条 query (tokio::select! 绑定 cancel 信号):          │
│   │         │    ├─ 尝试 Exa MCP (https://mcp.exa.ai/mcp, 25s 超时)          │
│   │         │    ├─ 若配置 Key 失败且非取消，或无 Key 未熔断，尝试回退       │
│   │         │    └─ 回退 Tavily REST (https://api.tavily.com/search, 25s)    │
│   │         └─ 组装 ToolExecutionResult::WebSearch(WebSearchToolResult)      │
│   │                                                                          │
│   └─► 3. host.finish_tool(ticket, Executed(result))                          │
│          ├─ DB: UPDATE agent_tool_call SET status='completed', result_json   │
│          ├─ AppState: settle_tool_detail -> ToolDetailView::WebSearch        │
│          ├─ UI: 渲染 ■ Web search · N queries · exa · 840ms 树形折叠卡片     │
│          └─ Context: format_for_model(&result) 注入下一轮模型采样             │
└──────────────────────────────────────────────────────────────────────────────┘
```

### 架构不变量（必须守住）

1. **`app/` 与 `ui/` 纯净性**：`app/` 不发网络请求、不读磁盘配置文件；`ui/` 只读 `ToolCallView` 与 `WebSearchDetailView`，不在渲染热路径解析 JSON。
2. **文件行数红线**：生产代码单文件严格不超过 1000 行；新增的联网搜索执行逻辑独立放置在 `src/chat/tools/web_search/` 子模块，UI 详情视图独立放置在 `src/app/state/lx/chat/tool_view/web_search.rs`，UI 卡片渲染独立放置在 `src/ui/main_content/lx/history/tool/web_search.rs`，每个新增文件控制在 200~450 行之间。
3. **修改 `Models` 设置不丢失 `webSearch` 配置**：`PUT /api/models/settings` 在合并 `AiSettings` 时，必须保留磁盘已有的 `current.web_search`；修改 `General` 页的 `PUT /api/config` 更新 `ai.json` 的 `web_search` 时，必须保留已有的 `providers`、`enabled_providers`、`default_model` 等全部模型配置，并向事件循环投递 `SettingsEvent::ModelsChanged` 刷新内存中的 `AiSettings` 快照。
4. **系统提示词 UTF-16 码元上限**：`src/chat/prompt.rs` 首段系统提示经过 `SystemPromptParts::split` 切分后，单条 native system 消息严格 `< 2000` UTF-16 码元，完整系统提示严格 `< 12 KiB`。

---

## 4. 详细设计：配置层与 Web 设置页

### 4.1 复制自 `lx-agent` 的默认 API Keys 与 `WebSearchConfig`

在 `src/config/ai/mod.rs` 中定义 `WebSearchConfig`，并将从 `/Users/yonah/.lx/config/ai.json` 提取的两把 Key 定义为公开默认常量：

```rust
/// 默认内置的 Exa API Key（复制自 lx-agent `ai.webSearch.exaApiKey`）。
pub const DEFAULT_EXA_API_KEY: &str = "515df894-bc88-4bee-a2ff-f20451c63516";

/// 默认内置的 Tavily API Key（复制自 lx-agent `ai.webSearch.tavilyApiKey`）。
pub const DEFAULT_TAVILY_API_KEY: &str =
    "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";

/// 联网搜索配置（对应 `config/ai.json` 的 `ai.webSearch` 节点，与 lx-agent 同构）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WebSearchConfig {
    /// Exa MCP API Key；允许为空字符串（表示用户显式清空，仅走匿名直连或回退 Tavily）。
    #[serde(default = "default_exa_api_key")]
    pub exa_api_key: String,
    /// Tavily Search API Key；允许为空字符串。
    #[serde(default = "default_tavily_api_key")]
    pub tavily_api_key: String,
}

fn default_exa_api_key() -> String {
    DEFAULT_EXA_API_KEY.to_string()
}

fn default_tavily_api_key() -> String {
    DEFAULT_TAVILY_API_KEY.to_string()
}

impl Default for WebSearchConfig {
    fn default() -> Self {
        Self {
            exa_api_key: default_exa_api_key(),
            tavily_api_key: default_tavily_api_key(),
        }
    }
}

impl WebSearchConfig {
    /// 归一化（去除首尾空白）。
    pub fn normalized(&self) -> Self {
        Self {
            exa_api_key: self.exa_api_key.trim().to_string(),
            tavily_api_key: self.tavily_api_key.trim().to_string(),
        }
    }
}
```

在 `AiSettings` 结构体中新增字段（取代原先落入 `extra` map 的未类型化处理）：

```rust
pub struct AiSettings {
    #[serde(default, rename = "enabled_providers")]
    pub enabled_providers: Vec<String>,
    #[serde(default)]
    pub providers: BTreeMap<String, ProviderConfig>,
    #[serde(
        default,
        rename = "defaultModel",
        skip_serializing_if = "Option::is_none"
    )]
    pub default_model: Option<ModelSelection>,
    #[serde(
        rename = "streamIdleTimeoutMs",
        skip_serializing_if = "Option::is_none"
    )]
    pub stream_idle_timeout_ms: Option<u64>,
    /// 联网搜索 API Key 配置（`ai.webSearch`）。
    #[serde(default, rename = "webSearch")]
    pub web_search: WebSearchConfig,
    #[serde(flatten)]
    pub extra: serde_json::Map<String, serde_json::Value>,
}
```

同时，在开发环境配置文件 `~/.lx-t-dev/config/ai.json`（以及若存在 `~/.lx-t/config/ai.json`）中写入 `webSearch` 节点：
```json
"webSearch": {
  "exaApiKey": "515df894-bc88-4bee-a2ff-f20451c63516",
  "tavilyApiKey": "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"
}
```

### 4.2 设置后端接口扩展（`src/settings/service/handlers.rs`）

1. **`GET /api/state`**：
   除了返回原有的 `config` 和 `commands` 外，读取 `ai_path(state)`（即 `<user_root>/config/ai.json`）中的 `web_search` 配置，在响应 JSON 中增加 `"web_search"` 对象：
   ```json
   {
     "config": { ... },
     "web_search": {
       "exa_api_key": "515df894-bc88-4bee-a2ff-f20451c63516",
       "tavily_api_key": "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"
     },
     "commands": [ ... ],
     "project_available": false
   }
   ```
2. **`PUT /api/config`**：
   - 先执行纯校验与合并：解析 `config` 并校验 `"web_search"` 字段（若字段存在但非 JSON Object，或 Key 字符串含控制字符，立即返回 `422`，不写任何文件）；
   - 若携带 `"web_search"` 对象（`{"exa_api_key": "...", "tavily_api_key": "..."}`），预加载当前 `ai_path(state)` 的 `AiSettings`（文件不存在或 JSON 损坏时记录 `tracing::warn!` 并以 `AiSettings::empty()` 为基底修复），合并更新 `ai_settings.web_search`（保留 `webSearch` 内未知扩展字段 `extra`），先调用 `crate::config::ai::save_ai(&ai_path, &ai_settings)`（使用带进程 ID + 原子序号的唯一 `.tmp` 文件原子落盘，0600 权限）；
   - 随后保存 `config` 到 `settings.json`，最后统一发送 `SettingsEvent::ConfigChanged(config)` 与 `SettingsEvent::ModelsChanged(Box::new(ai_settings))` 通知事件循环实时更新内存配置。
3. **`GET /api/models/settings` 与 `PUT /api/models/settings`**：
   - `merge_settings` 基于 `current.unwrap_or_default()` 修改 `providers` / `default_model`，天然保留 `settings.web_search` 不被覆盖；
   - `revision_of` 仅对模型相关字段（`default_model`、`enabled_providers`、`providers`、`stream_idle_timeout_ms`）计算乐观锁哈希，使 General 页修改 `web_search` 不会导致已打开的 Models 页产生假 409 冲突。

### 4.3 浏览器设置页前端（`assets/settings.html` & `assets/settings.js`）

#### `assets/settings.html`（在 `#view-general` 的 `Layout` `<fieldset>` 下方新增）

```html
    <fieldset>
      <legend>Web Search</legend>
      <div class="field-grid">
        <div class="field-wide">
          <label for="web_search_exa_api_key">Exa API key</label>
          <div class="input-with-action">
            <input id="web_search_exa_api_key" type="password" autocomplete="off" placeholder="Exa API key (optional, falls back to anonymous or Tavily)">
            <button id="exa-key-reveal" type="button" class="input-action" aria-label="Show API key" aria-pressed="false">
              <svg viewBox="0 0 16 16" aria-hidden="true" focusable="false"><path d="M8 3.2C4.7 3.2 2.2 5.7 1.3 8c.9 2.3 3.4 4.8 6.7 4.8s5.8-2.5 6.7-4.8C13.8 5.7 11.3 3.2 8 3.2Z"/><circle cx="8" cy="8" r="2.1"/></svg>
            </button>
          </div>
        </div>
        <div class="field-wide">
          <label for="web_search_tavily_api_key">Tavily API key</label>
          <div class="input-with-action">
            <input id="web_search_tavily_api_key" type="password" autocomplete="off" placeholder="Tavily API key (fallback search provider)">
            <button id="tavily-key-reveal" type="button" class="input-action" aria-label="Show API key" aria-pressed="false">
              <svg viewBox="0 0 16 16" aria-hidden="true" focusable="false"><path d="M8 3.2C4.7 3.2 2.2 5.7 1.3 8c.9 2.3 3.4 4.8 6.7 4.8s5.8-2.5 6.7-4.8C13.8 5.7 11.3 3.2 8 3.2Z"/><circle cx="8" cy="8" r="2.1"/></svg>
            </button>
          </div>
        </div>
      </div>
    </fieldset>
```

#### `assets/settings.js`（General 控制器与明文显隐切换）

```javascript
const WEB_SEARCH_KEYS = ['web_search_exa_api_key', 'web_search_tavily_api_key'];

function bindPasswordReveal(inputId, buttonId) {
  const input = document.getElementById(inputId);
  const button = document.getElementById(buttonId);
  if (!input || !button) { return; }
  button.addEventListener('click', () => {
    const shown = input.type === 'text';
    input.type = shown ? 'password' : 'text';
    button.setAttribute('aria-pressed', shown ? 'false' : 'true');
    button.setAttribute('aria-label', shown ? 'Show API key' : 'Hide API key');
  });
}

function generalValues() {
  const values = {};
  for (const key of CONFIG_KEYS) {
    values[key] = document.getElementById(key).value;
  }
  for (const key of WEB_SEARCH_KEYS) {
    const el = document.getElementById(key);
    values[key] = el ? el.value : '';
  }
  return values;
}

async function saveGeneral() {
  const config = {};
  for (const key of CONFIG_KEYS) {
    config[key] = Number(document.getElementById(key).value);
  }
  const web_search = {
    exa_api_key: (document.getElementById('web_search_exa_api_key').value || '').trim(),
    tavily_api_key: (document.getElementById('web_search_tavily_api_key').value || '').trim()
  };
  try {
    await api('/api/config', {method: 'PUT', body: JSON.stringify({config, web_search})});
    showToast('Settings saved', 'ok');
    await loadGeneral();
  } catch (error) {
    showToast(error.message, 'error');
    throw error;
  }
}
```

---

## 5. 详细设计：`web_search` 工具规格、提示词与执行引擎

### 5.1 工具名称与 JSON Schema（`src/chat/tools/schema.rs`）

- 工具名：`pub const WEB_SEARCH_TOOL: &str = "web_search";`
- `TOOLS` 数组容量：`16` → `17`（紧跟 `search` 之后或 `curr_time` 之后；放在 `search` 之后最符合认知顺序：`read`, `search`, `web_search`, `edit`, `write`...）。
- **工具描述（英文，融合 `codex-main` 与 `lx-agent`）**：

```rust
ToolSpec::new(
    "web_search",
    "Search the public internet for up-to-date, external, or specialized information not present in \
     the local workspace (such as recent releases, news, documentation, API changes, prices, or \
     facts that may have changed). Pass either a single `query` string or `queries` (1-4 distinct \
     queries executed concurrently in one call). Prefer batching related queries in `queries` to \
     gather evidence faster. When your final response relies on web results, cite every supported \
     claim inline with a Markdown link `[descriptive source title](https://...)` placed right after \
     the sentence or clause — never use bare URLs and never collect links at the end of the message.",
    web_search_parameters,
)
```

- **参数 JSON Schema（`web_search_parameters`）**：
  为兼容所有模型传输层（尤其是 Google Gemini 与 OpenAI strict schema 对顶层 `oneOf` 的限制），顶层固定为 `"type": "object"`、`additionalProperties: false`，`query` 与 `queries` 均为可选字段，并在描述与运行时 `parse_arguments` 中强制要求「`query` 与 `queries` 至少提供一个（若同时提供则合并去重，总数不超过 4 条）」：

```rust
fn web_search_parameters() -> Value {
    use super::web_search::{
        DEFAULT_NUM_RESULTS, MAX_NUM_RESULTS, MAX_QUERIES, MAX_QUERY_CHARS, MIN_NUM_RESULTS,
    };
    json!({
        "type": "object",
        "properties": {
            "query": {
                "type": "string",
                "description": format!(
                    "Single search query (1-{MAX_QUERY_CHARS} chars). Provide either `query` or `queries`."
                )
            },
            "queries": {
                "type": "array",
                "minItems": 1,
                "maxItems": MAX_QUERIES,
                "items": { "type": "string" },
                "description": format!(
                    "Batch of 1-{MAX_QUERIES} search queries executed concurrently in a single call. \
                     Use this when you need to look up multiple topics or angles at once."
                )
            },
            "num_results": {
                "type": "integer",
                "minimum": MIN_NUM_RESULTS,
                "maximum": MAX_NUM_RESULTS,
                "description": format!(
                    "Maximum number of results per query ({MIN_NUM_RESULTS}-{MAX_NUM_RESULTS}, default {DEFAULT_NUM_RESULTS})."
                )
            },
            "type": {
                "type": "string",
                "enum": ["auto", "fast", "deep"],
                "description": "Search depth mode: `auto` (default), `fast`, or `deep`."
            }
        },
        "additionalProperties": false
    })
}
```

### 5.2 参数解析与边界常量（`src/chat/tools/web_search/mod.rs`）

```rust
pub const WEB_SEARCH_TOOL: &str = "web_search";
pub const MAX_QUERIES: usize = 4;
pub const MAX_QUERY_CHARS: usize = 500;
pub const MIN_NUM_RESULTS: usize = 1;
pub const MAX_NUM_RESULTS: usize = 10;
pub const DEFAULT_NUM_RESULTS: usize = 8;
pub const WEB_SEARCH_TIMEOUT_MS: u64 = 25_000;

/// 搜索深度档位（对齐 lx-agent Exa 参数）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SearchDepth {
    #[default]
    Auto,
    Fast,
    Deep,
}

impl SearchDepth {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::Fast => "fast",
            Self::Deep => "deep",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "auto" => Some(Self::Auto),
            "fast" => Some(Self::Fast),
            "deep" => Some(Self::Deep),
            _ => None,
        }
    }
}

/// 归一化后的 `web_search` 请求参数。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WebSearchArgs {
    /// 去空白、去重后的查询列表（1..=MAX_QUERIES）。
    pub queries: Vec<String>,
    /// 每条查询的最大结果数（1..=10，默认 8）。
    pub num_results: usize,
    /// 搜索深度模式（默认 Auto）。
    pub search_type: SearchDepth,
}
```

**参数校验规则（`parse_arguments`）**：
1. 白名单字段校验：`["query", "queries", "num_results", "type"]`（同时兼容模型偶发传入 camelCase `"numResults"`：白名单包含 `"numResults"`，若 `"num_results"` 与 `"numResults"` 同时出现且不一致则报 `invalid_arguments`）。
2. 提取 `query`（若存在，须为字符串）与 `queries`（若存在，须为字符串数组）：
   - 每条查询 `trim()` 后若为空串，报 `"search query must not be empty"`；
   - 字符数 `chars().count() > MAX_QUERY_CHARS (500)`，报 `"search query exceeds 500 characters"`；
   - 含除 `\t` 外的控制字符，报 `"search query contains control characters"`；
   - 按出现顺序去重后，若总数为 0，报 `"web_search requires `query` or non-empty `queries`"`；
   - 若去重前或去重后条数 `> MAX_QUERIES (4)`，报 `"web_search accepts at most 4 queries per call"`。
3. `num_results`：默认 `8`，超出 `1..=10` 报 `"num_results must be between 1 and 10"`。
4. `type`：默认 `"auto"`，非 `"auto" | "fast" | "deep"` 报 `"type must be one of: auto, fast, deep"`。

### 5.3 双引擎执行、熔断与并发容错（`src/chat/tools/web_search/client.rs`）

#### 数据结构

```rust
/// 实际命中返回结果的搜索引擎提供商。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum WebSearchProvider {
    Exa,
    Tavily,
}

impl WebSearchProvider {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Exa => "exa",
            Self::Tavily => "tavily",
        }
    }

    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "exa" => Some(Self::Exa),
            "tavily" => Some(Self::Tavily),
            _ => None,
        }
    }
}

/// 单条查询的执行结局。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct QuerySearchOutcome {
    pub query: String,
    /// 成功时的提供商；单条失败时为 None。
    pub provider: Option<WebSearchProvider>,
    /// 成功时的格式化 Markdown 检索文本。
    pub output: String,
    /// 单条失败时的错误说明（部分成功场景下保留）。
    pub error: Option<String>,
}

/// 整次 `web_search` 调用的结构化结果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WebSearchToolResult {
    pub queries: Vec<String>,
    pub num_results: usize,
    pub search_type: SearchDepth,
    /// 主要提供商（首个成功查询使用的提供商）。
    pub provider: WebSearchProvider,
    /// 按查询顺序对应的结果列表。
    pub outcomes: Vec<QuerySearchOutcome>,
    /// 模型可见的组合输出文本（亦用于 UI 结果区渲染）。
    pub text: String,
    pub truncated: bool,
    pub duration_ms: u64,
}
```

#### 单条查询的双引擎回退策略（对齐 `lx-agent/src/main/agent/tools/webSearch.ts`）

1. **Exa MCP 调用（`search_with_exa`）**：
   - Endpoint：
     - 有 Key：`https://mcp.exa.ai/mcp?exaApiKey=<url_encoded_key>`
     - 无 Key：`https://mcp.exa.ai/mcp`
   - Headers：`Accept: application/json, text/event-stream`、`Content-Type: application/json`
   - Body：
     ```json
     {
       "jsonrpc": "2.0",
       "id": 1,
       "method": "tools/call",
       "params": {
         "name": "web_search_exa",
         "arguments": {
           "query": "<query>",
           "numResults": 8,
           "type": "auto",
           "livecrawl": "fallback"
         }
       }
     }
     ```
   - 响应解析（`parse_exa_response_text`）：
     - 先尝试将完整 body 作为 JSON 解析 `McpToolCallResponse`；若 `error.message` 存在则返回错误；否则提取 `result.content` 中首个 `type == "text"` 的非空文本；
     - 若整段非直接 JSON，按行扫描 `data: ` 前缀的 SSE 行并解析，取首个非空文本；
     - HTTP 200 但内容为空时，返回 `"No search results found. Please try a different query."`。
2. **匿名直连熔断与空 Key 短路机制**：
   - 维护进程内原子标记 `EXA_ANON_BLOCKED: AtomicBool`；
   - 当无 Key 匿名 Exa 请求收到 HTTP `401` 或 `403` 时，将 `EXA_ANON_BLOCKED` 置为 `true`；仅当后续匿名 Exa 请求成功（2xx）时重置为 `false`（配置非空 Key 失败回退时绝不清空已触发的匿名熔断，防止每次查询重复触发必败的匿名 403 请求）；
   - 若调用时配置了 Exa Key，但因 Key 失效/超额返回 `401`/`402`/`403`/`429` 且 `EXA_ANON_BLOCKED` 为 `false`，先尝试一次 Exa 无 Key 匿名直连，再回退到 Tavily；
   - Tavily 要求必须提供非空 `tavily_api_key`，若为空则直接短路返回 `"Tavily API key is not configured"`，不发起无认证请求。
3. **Tavily REST 调用（`search_with_tavily`）**：
   - Endpoint：`https://api.tavily.com/search`
   - Headers：`Accept: application/json`、`Content-Type: application/json`、`Authorization: Bearer <tavily_api_key>`
   - Body：
     ```json
     {
       "query": "<query>",
       "max_results": 8,
       "search_depth": "<basic | advanced>",
       "include_answer": false
     }
     ```
     *注：当 `search_type == SearchDepth::Deep` 时，`search_depth` 映射为 `"advanced"`，其余（`Auto` / `Fast`）映射为 `"basic"`。*
   - 响应解析（`parse_tavily_response`）：
     - 若顶层 `answer` 为非空字符串，作为第一段；
     - 遍历 `results` 数组，每项按 `{index + 1}. {title}\n{url}\n{content}` 组装，以 `\n\n` 连接；
     - 若无结果，返回 `"No search results found."`。
4. **超时与 `Esc` 取消**：
   - 入口先检查 `*cancel.borrow()`，若已取消则零请求直接返回 `ToolErrorCode::Cancelled`；
   - 每次单引擎请求复用进程级 `OnceLock<reqwest::Client>` 连接池，并使用 `tokio::time::timeout(Duration::from_millis(25_000), request_future)`；
   - 外层包裹 `tokio::select! { biased; _ = wait_cancelled(&mut cancel) => return cancelled_result(), res = search_future => res }`；
   - 一旦收到取消信号，立即返回 `ToolErrorCode::Cancelled`，`run_agent_turn` 据此将工具调用落库为 `cancelled` 并立即结束当前轮次（`TurnSummary::interrupted`）。
5. **批量并发、单条预算与部分成功聚合**：
   - 每条查询输出受 `MAX_QUERY_OUTPUT_CHARS = 12_000` 预算保护（超出时在单条末尾追加 `\n[...truncated]` 并将 `WebSearchToolResult.truncated` 置为 `true`），确保 4 条并发查询合计稳态落在 64 KiB 回放预算内；
   - 当 `queries.len() == 1` 时，`text` 直接为单条查询的 `output`；
   - 当 `queries.len() > 1` 时，使用 `tokio::task::JoinSet` 同时发起所有查询，按原始 `queries` 顺序聚合成清晰的分段 Markdown：
     ```markdown
     ### 1. "ratatui 0.29 migration guide"
     <output of query 1>

     ### 2. "ratatui 0.29 breaking changes"
     <output of query 2>
     ```
   - 若同一批次中同时命中了 `Exa` 与 `Tavily`，`WebSearchToolResult::provider_label()` 返回 `"exa+tavily"`，卡片标题据实展示双引擎标识；
   - 若其中某条查询失败但至少有 1 条成功，失败项输出 `[Search failed: <error>]`，整次调用仍返回 `ToolExecutionResult::WebSearch`（状态为 `Completed`）；
   - 若所有查询全部失败，汇总去重后的脱敏错误详情附在提示后；若全部因超时失败则返回 `ToolErrorCode::Timeout`，否则返回 `ToolErrorCode::Io`。

### 5.4 系统提示词更新（`src/chat/prompt.rs` & `src/chat/agent.rs`）

1. **`PROMPT_VERSION`**：从 `"lx-7"` 递增为 `"lx-8"`。
2. **`LX_SYSTEM_PROMPT`（`src/chat/prompt.rs`）**：
   - 在 `# Tools` 的工具列表中加入 `web_search`；
   - 新增 `## Web search (\`web_search\`)` 章节（精炼移植 `codex-main` 的决策边界与引用规范，严格控制在 ~900 字符以内，确保 `LX_SYSTEM_PROMPT` 首段 `< 2000` UTF-16 码元且总长远低于 12 KiB）：
     ```markdown
     ## Web search (`web_search`)

     - Use `web_search` when information may have changed recently or is not in the workspace: news, latest releases, library/API docs, prices, schedules, or any fact you are not fully certain about. Obey explicit user requests to search or not search.
     - Pass `queries` (up to 4 queries in one call) to batch related lookups concurrently instead of issuing sequential single-query calls.
     - Cite every web-supported claim inline with a Markdown link `[descriptive source title](https://...)` placed right after the sentence or clause it supports. Never use bare URLs, never link to generic search pages, and never collect citations at the end of the response.
     - Keep direct quotes under 25 words per source; summarize and paraphrase in your own words.
     ```
3. **`EXPLORER_PROMPT` 与 `EXECUTOR_PROMPT`（`src/chat/agent.rs`）**：
   - `EXPLORER_TOOLS`：`["read", "search", "web_search"]`；在 `EXPLORER_PROMPT` 中说明拥有 `read`, `search`, `web_search`，对外网事实须附 `[title](url)` 链接。
   - `EXECUTOR_TOOLS`：`["read", "search", "web_search", "edit", "write", "exec_command", "write_stdin"]`；在 `EXECUTOR_PROMPT` 中说明在需要核实外部库/API 用法时可调用 `web_search`。

---

## 6. 详细设计：TUI 视图状态与工具卡片渲染

### 6.1 详情视图模型（`src/app/state/lx/chat/tool_view/web_search.rs`）

参照 `src/app/state/lx/chat/tool_view/mcp.rs` 的结构，新建 `web_search.rs` 并挂载为 `ToolDetailView::WebSearch(WebSearchDetailView)`：

```rust
/// `web_search` 工具卡片详情视图（纯数据，渲染层只读）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WebSearchDetailView {
    /// 本次检索的查询词列表（1..=4）。
    pub queries: Vec<String>,
    /// 搜索深度模式（`"auto"` / `"fast"` / `"deep"`）。
    pub search_type: String,
    /// 每条查询请求的最大结果数。
    pub num_results: usize,
    /// 命中的搜索引擎（`"exa"` / `"tavily"`）；Pending/Running 或失败时为 None。
    pub provider: Option<String>,
    /// 结果正文行（最多保留前 40 行供展开阅读，超出计数记入 `result_more`）。
    pub result: Vec<String>,
    /// 超出保留窗口的结果行数。
    pub result_more: usize,
    /// 调用耗时（毫秒）；Pending/Running 时为 None。
    pub duration_ms: Option<u64>,
    /// 结果是否被截断。
    pub truncated: bool,
}
```

- **实时流式阶段（`Pending` / `Running`）**：
  在 `ToolCallView::accept_arguments` 中，当 `self.name == "web_search"` 时，调用 `WebSearchDetailView::from_arguments(&self.arguments_json)` 解析出 `queries`、`search_type`、`num_results`，使卡片在网络请求进行期间立即展示出正在检索的查询词。
- **实时结算与历史重载阶段（`Completed` / `Failed`）**：
  - `to_result_json` 生成落库 JSON：
    ```json
    {
      "tool": "web_search",
      "queries": ["..."],
      "num_results": 8,
      "search_type": "auto",
      "provider": "exa",
      "text": "...",
      "truncated": false,
      "duration_ms": 840
    }
    ```
  - **实时结算保活与历史重载回退**：
    - 实时流中若工具失败或被取消（`detail == None`），`process.rs` 与 `subagent.rs` 通过 `ToolDetailView::preserves_on_empty_settle()` 保留 `accept_arguments` 阶段构建的 `WebSearchDetailView`（查询词不丢失）；
    - 历史重载（`view.rs`）调用 `WebSearchDetailView::from_tool(result_json, arguments_json)` 从落库 JSON 与调用参数还原完整视图（每行结果按 `MAX_RESULT_LINE_CHARS = 8_000` 截断补 `…`，`history/cache.rs` 逐行 `write_edges` 零分配计算指纹）。

### 6.2 历史工具卡片布局（`src/ui/main_content/lx/history/tool/web_search.rs`）

卡片布局严格复用 `tree::layout(&sections, width)`：

```text
运行中（单条查询）：
□ Web search · Searching…
  └─ "ratatui 0.29 release notes"

运行中（多条查询 + 非默认参数）：
□ Web search · 2 queries · Searching…
  └─ "ratatui 0.29 migration guide" · deep · 5 results
     "ratatui 0.29 breaking changes"

完成态（默认折叠，预览前 3 行）：
■ Web search · 2 queries · exa · 840ms
  ├─ "ratatui 0.29 migration guide"
  │  "ratatui 0.29 breaking changes"
  └─ 1. Ratatui 0.29.0 Release Notes
     https://ratatui.rs/highlights/v029/
     Ratatui 0.29 introduces new layout constraints...
     … 24 more lines

完成态（点击 └─ 展开结果块）：
■ Web search · 2 queries · exa · 840ms
  ├─ "ratatui 0.29 migration guide"
  │  "ratatui 0.29 breaking changes"
  └─ 1. Ratatui 0.29.0 Release Notes
     https://ratatui.rs/highlights/v029/
     Ratatui 0.29 introduces new layout constraints...
     (完整保留的压暗 Markdown 行)
```

- **标题行状态词（`title_status`）**：
  - `Pending` / `Running` / `InputStreaming`：多条查询时显示 `"N queries · Searching…"`，单条查询时显示 `"Searching…"`；
  - `Completed`：组合 `[queries_count_if_gt_1, provider, duration]`，例如 `"exa · 840ms"` 或 `"2 queries · exa · 1.2s"`，若 `truncated` 则追加 `"results truncated"`；
  - `Failed` / `Interrupted`：遵循 `header_with_verb` 统一规则，整行染红/黄，错误信息显示在 `└─` 节点。
- **独立折叠区域（`CardRegion`）**：
  - `CardSection::Body`（`├─` 查询参数块）：折叠预览前 3 行，当查询行超过 3 行时可独立点击展开/折叠；
  - `CardSection::Output`（`└─` 搜索结果块）：折叠预览前 3 行（`Preview::head(3)`），超出时自动显示 `… N more lines`（`style::fold_hint()`），点击该区域只切换 `folds.output`，内容统一使用 `muted_markdown_lines(&raw_source, body_width)` 渲染。

### 6.3 底部聊天状态栏联动（`src/ui/main_content/lx/chat_bar.rs`）

参考 `codex-main` 的 `WebSearchCell` 与 `agent_status_feed.rs`：
- 当当前流式 assistant 消息存在处于 `Pending` 或 `Running` 状态的 `"web_search"` 工具调用时：
  - 若 `WebSearchDetailView` 中有查询词（取第一条查询词 `q`），`chat_bar` 显示动态扫描高亮文案：`Searching the web: "q"`（若共 N 条且 N > 1，显示 `Searching the web: "q" (+N-1 more)`）；
  - 若参数尚未解析出查询词，显示 `Searching the web...`。

---

## 7. 涉及文件与实测单文件行数核查

遵守 `docs/standards/code-writing-standards.md`：「生产代码单文件上限 1000 行，超限必须按职责拆分，拆分子文件控制在 500 行左右；测试文件严格跟随子模块拆分」。

| 文件路径 | 实测行数 | 职责与拆分说明 |
|----------|----------|----------------|
| `src/config/ai/mod.rs` | 499 行 | 新增 `WebSearchConfig`（含 `extra` 未知字段保留）、`DEFAULT_EXA_API_KEY`、`DEFAULT_TAVILY_API_KEY`、唯一 `.tmp` 原子落盘 |
| `src/settings/service.rs` | 451 行 | 提取共享 `ai_path` 辅助函数，挂载 `tests_web_search` |
| `src/settings/service/handlers.rs` | 940 行 | `state_json` 输出 `web_search`；`put_config` 先校验合并、再写 `ai.json` 与 `settings.json`、最后广播事件 |
| `src/settings/service/tests_web_search.rs` | **新建** 335 行 | 独立 Web Search 设置服务集成测试（原子校验、损坏修复、模型配置保留） |
| `assets/settings.html` | 432 行 | General 页新增 `Web Search` fieldset（Exa / Tavily 密码框 + 显隐按钮） |
| `assets/settings.js` | 978 行 | General 读写与脏检测纳入两个 Web Search Key，绑定明文切换按钮与 `resetWebSearchReveals` 自动复位 |
| `src/chat/tools/web_search/mod.rs` | **新建** 352 行 | `web_search` 参数模型 `WebSearchArgs`、`SearchDepth`、去重后条数校验、单条 `12_000` 字符预算裁剪与 `provider_label` |
| `src/chat/tools/web_search/client.rs` | **新建** 661 行 | `OnceLock<Client>` 复用、Exa MCP + Tavily REST 异步请求、SSE/JSON 解析、匿名 401/403 熔断、错误摘要提取与 `Esc` 取消 |
| `src/chat/tools/web_search/tests.rs` | **新建** 864 行 | 参数校验、Exa JSON/SSE、Tavily、回退与持续熔断、单条预算截断、混合引擎、预取消与超时单测 |
| `src/chat/tools/schema.rs` | 583 行 | `TOOLS` 16→17，新增 `web_search` `ToolSpec` 与 `web_search_parameters` |
| `src/chat/tools/types.rs` | 541 行 | `TOOL_NAMES` 增加 `"web_search"`，`ToolExecutionResult` 新增 `WebSearch(Box<WebSearchToolResult>)` |
| `src/chat/tools/mod.rs` | 201 行 | 导出 `web_search` 模块，`execute_with` 守卫分支包含 `WEB_SEARCH_TOOL` |
| `src/chat/tools/output.rs` | 791 行 | `format_for_model`、`to_summary`（`path: None`）、`to_result_json`（`provider_label`）与 `shrink_json_text` |
| `src/chat/agent.rs` | 800 行 | `MAIN_TOOLS` (16→17)、`EXPLORER_TOOLS` (2→3)、`EXECUTOR_TOOLS` (6→7) 及子 Agent 提示词更新 |
| `src/chat/prompt.rs` | 836 行 | `PROMPT_VERSION` `lx-7`→`lx-8`，`LX_SYSTEM_PROMPT` 增加 `web_search` 工具与内联引用规范 |
| `src/chat/agent_loop/executor.rs` | 582 行 | `AgentTools` 注入 `with_web_search_keys`，在 `execute_mcp_cancellable` 与 `run` 中分派 `web_search` |
| `src/app/state/lx/chat/tool_view/web_search.rs` | **新建** 175 行 | `WebSearchDetailView`：从参数与 `result_json` 构造详情视图（单行上限 `8_000` 字符补 `…`） |
| `src/app/state/lx/chat/tool_view/web_search/tests.rs` | **新建** 107 行 | `WebSearchDetailView` 参数解析、结果窗口与超长行截断单测 |
| `src/app/state/lx/chat/tool_view.rs` | 885 行 | 挂载 `WebSearchDetailView`、`ToolDetailView::WebSearch` 与 `preserves_on_empty_settle` |
| `src/ui/main_content/lx/history/tool/web_search.rs` | **新建** 273 行 | `web_search` 卡片布局、标题状态、`├─` 查询块与 `└─` 压暗 Markdown 结果块渲染与点击区域 |
| `src/ui/main_content/lx/history/tool/web_search/tests.rs` | **新建** 320 行 | 卡片渲染、独立折叠点击区域、空查询回退偏移与窄宽度（36 列）折行单测 |
| `src/ui/main_content/lx/history/tool.rs` | 832 行 | 分派 `ToolDetailView::WebSearch` 的 `rows`、`draw_animated`、`has_expandable_content`、`card_regions` |
| `src/ui/main_content/lx/history/cache.rs` | 856 行 | `ToolDetailView::WebSearch` 逐行 `write_edges` 零分配有界哈希指纹 |
| `src/ui/main_content/lx/chat_bar.rs` | 600 行 | `ActivityLabel` 新增 `SearchingWeb(Option<String>)`，展示 `Searching the web: ...` |
| `src/event/chat/mod.rs` | 971 行 | 注入 `web_search` Key 并收敛冗余行数，保持在 1000 行上限以内 |

---

## 8. 异常、安全与边界条件

1. **API Key 安全与脱敏**：
   - 日志（`tracing`）与返回给模型的错误消息严禁包含 `exa_api_key`、`tavily_api_key` 或 `?exaApiKey=...`（`transport_message` 与 `error_detail` 双重清洗脱敏）。
   - `config/ai.json` 保存时使用带进程 ID + 原子序号的唯一临时文件并走 `platform::write_file_atomic` 保持 `0600` 权限。
2. **超大响应截断保护**：
   - 单次 HTTP 响应体读取上限设为 `2 MiB`，防止异常端点返回无限流撑爆内存；
   - 单条查询结果上限 `MAX_QUERY_OUTPUT_CHARS = 12_000` 字符（4 条并发稳态 ≤ 48 KiB）；
   - `to_result_json` 沿用 `MAX_RESULT_JSON_BYTES`（1 MiB）与 `shrink_json_text` 截断保护；模型回放沿用 `MAX_TOOL_RESULT_REPLAY_BYTES`（64 KiB）。
3. **测试隔离（禁止单测发起真实外网请求）**：
   - `web_search::client` 支持通过 `WebSearchEndpoints` 注入本地回环 `TcpListener` 假服务，所有单元测试均在本地回环或内存假实现上完成，零外网依赖、毫秒级执行。
