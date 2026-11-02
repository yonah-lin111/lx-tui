# 会话与模板块标题生成（含 Settings Model/Provider 拆分）：设计方案

> 状态：**已在工作树 `.worktrees/title-generation`（分支 `feat/title-generation`）完成实现与测试，待用户确认合并**
>
> 关联任务清单：`docs/feature/title-generation/task.md`
>
> 参考实现：
> - `/Users/yonah/projects/agent/codex-main/codex-rs/tui/src/app/thread_title.rs`
> - `/Users/yonah/projects/agent/lx-agent/src/main/agent/titleGenerator.ts`
> - `/Users/yonah/projects/agent/lx-agent/src/renderer/src/features/markdown/hooks/useMarkdownCommandRunners.ts`

---

## 1. 背景与目标

### 1.1 现状分析

1. **浏览器设置页（`src/settings/` & `assets/`）**：
   - 左侧导航栏目前为 `General`、`Commands`、`Models`、`MCP`、`Skills` 共 5 个 Tab。
   - 原 `Models` 页同时混放了顶部的 `Default model` 下拉框与下方的供应商（Provider）及模型 CRUD 列表，且 `AiSettings` 中尚未支持配置专用的标题生成模型（`titleSummary` 此前作为未知字段在保存时被丢弃）。
2. **`lx` 聊天新对话标题（`src/ui/main_content/lx/` & `src/db/chat/`）**：
   - 新会话首轮用户消息落库时（`Database::begin_turn_with_attachments`），直接截取用户输入前 60 个字符（`single_line_preview(&display_text, 60)`）作为 `agent_session.title`。
   - 缺乏像 `codex-main` 那样的后台异步 LLM 标题提炼能力，导致 `/resume` 历史列表中的会话标题往往是未加提炼的长句截断。
3. **右侧 Prompt 编辑器模板块（`src/ui/prompt/` & `src/app/prompt/`）**：
   - 任务块（`&&& <cmd> --start ... &&& <cmd> --end`）与补充块（`+++ <cmd> --start ... +++ <cmd> --end`）支持在起始行下一行维护结构性标题行 `{title: ...}`。
   - 块内 `/` 斜杠命令面板目前仅有 `/supplement`、`/gitWorkTree` 与 `template` 作用域自定义命令，缺少为光标所在模板块或补充块一键提炼并回写 `{title: ...}` 的 `/title` 命令。

### 1.2 核心目标

1. **设置页拆分 `Model` 与 `Provider` Tab**：
   - 将浏览器设置页左侧 Tab 拆分为 `General`、`Commands`、`Model`、`Provider`、`MCP`、`Skills`。
   - 原 `Models` 页的供应商与模型列表管理调整至 `Provider` Tab（`#tab-provider` / `#view-provider`）。
   - 新增独立的 `Model` Tab（`#tab-model` / `#view-model`），集中管理全局模型配置：**`Default model`**（`defaultModel`）与 **`Title model`**（JSON 键名 `titleSummary`，与 `lx-agent` 保持兼容）。
2. **`lx` 新对话自动生成标题（参考 `codex-main`）**：
   - 当新会话首轮用户消息落库成功（`on_turn_committed` 且 `turn_index == 1`）时，立即在后台并行发起无工具、轻量级的标题生成请求，不阻塞主对话流式输出。
   - 生成前数据库保留首条消息前 60 字符作为初始兜底标题；生成成功后静默更新 `agent_session.title` 及已打开的 `/resume` 历史列表。
   - 严格遵循手动命名优先（防覆盖）：若后台生成期间用户已通过 `/rename`（或 `/resume` 编辑弹窗）手动改名，自动生成的标题直接丢弃。
3. **Prompt 模板块（`&&&` 和 `+++`）新增 `/title` 命令**：
   - 当光标位于 `&&&`（`BlockContext::Template`）或 `+++`（`BlockContext::Supple`）块内时，`/` 候选面板提供内置命令 `/title`。
   - 按 `Enter`（或鼠标点击）确认 `/title` 后：**清空命令所在行的 `/title` 文本，但严禁删除该空行本身（保留行尾 `\n`，光标停在该空行行首）**。
   - 提取光标所在内层块（优先 `+++`，否则外层 `&&&`）的有效正文；若为空则弹出英文 Warning Toast（`"Template block is empty"`）。
   - 发起异步生成期间，将目标块的 `{title: ...}` 行更新为 `{title: Generating title...}`；生成成功后按页与块起始行定位回写 `{title: <生成的标题>}` 并触发页面持久化保存；若失败则恢复原 `{title: ...}` 行并弹出英文 Warning Toast。

---

## 2. 已确认边界（Grill Me 决策表）

| # | 决策维度 | 已确认结论 |
| --- | --- | --- |
| 1 | 设置页导航与模型配置结构 | 在 `src/settings/`（及 `assets/`）左侧导航栏新增 `Model` 和 `Provider` 两个 Tab：将原 `Models` 页面内容（供应商与模型管理）调整为 `Provider` 页；将模型选择设置（`Default model` 和 `Title model`）全部放入 `Model` Tab 中。`AiSettings` 新增 `title_summary: Option<ModelSelection>`（序列化键名 `titleSummary`）。 |
| 2 | `Title model` 选择与回退策略 | 优先使用 `Title model`（`titleSummary`）；若未配置（`None`）则自动回退：<br>1. **`lx` 新对话标题生成**：优先用 `Title model`；未配置则回退使用本轮对话发送时冻结的模型（窗格选中模型或 `Default model`）；若请求失败/超时，静默保留首条消息前 60 字符兜底标题，不弹报错。<br>2. **Prompt 模板块 `/title` 命令**：优先用 `Title model`；未配置则回退使用 `Default model`；若两者均未配置或无有效 Key，弹出英文 Warning Toast（`"No title model or default model configured"`）。 |
| 3 | `lx` 新对话标题触发、展示与防覆盖 | 1. **触发时机**：对齐 `codex-main`，新会话首轮用户消息落库回执（`on_turn_committed` 且 `turn_index == 1`）时立即后台并行触发。<br>2. **展示位置**：仅静默更新 SQLite `agent_session.title` 及已打开的 `/resume` 历史面板缓存，不改动 `lx` 窗格顶栏 `lx-[native]` 与底栏状态条，不弹 Toast。<br>3. **提示词与清洗**：复用 `codex-main` 英文指令（跟随用户语言、单行、≤36 字符、去引号/Markdown/末尾标点，兼容 JSON `{"title":"..."}` 与纯文本并剥离 `<think>` 标签）。<br>4. **防覆盖**：若后台生成返回前用户已手动 `/rename`（或通过 `/resume` 编辑框改名），取消/丢弃自动生成结果，绝不覆盖手动标题。 |
| 4 | Prompt 模板块 `/title` 交互、空行保留与异步回写 | 1. **可见范围**：仅在 `BlockContext::Template`（`&&&`）与 `BlockContext::Supple`（`+++`）内部显示 `/title`。<br>2. **保留空行**：确认命令时仅将 `trigger.line_start..trigger.line_end` 替换为空串 `""`，**保留行尾 `\n`**，光标停在 `trigger.line_start`（该空行行首）。<br>3. **空内容校验**：剥离标题行、`{wt:}` 行、注释行、`/title` 行及未填写的模板空项后，若正文为空，弹出英文 Warning Toast `"Template block is empty"`，不发请求。<br>4. **加载占位与写回**：请求发出时目标块标题行写为 `{title: Generating title...}`；成功返回后替换为 `{title: <title>}` 并同步落库；失败或超时则恢复原 `{title: ...}` 文本并弹出 Warning Toast `"Failed to generate title"`。 |

---

## 3. 核心架构与数据流设计

### 3.1 浏览器设置页：`Model` 与 `Provider` Tab 拆分数据流

```text
[浏览器打开 Settings 页]
       │
       ├─ Left Nav:
       │    ├─ General   (#tab-general  -> #view-general)
       │    ├─ Commands  (#tab-commands -> #view-commands)
       │    ├─ Model     (#tab-model    -> #view-model)     <-- 新增：管理 Default model & Title model
       │    ├─ Provider  (#tab-provider -> #view-provider)  <-- 原 Models 页迁入：管理 Providers & Models
       │    ├─ MCP       (#tab-mcp      -> #view-mcp)
       │    └─ Skills    (#tab-skills   -> #view-skills)
       │
       ▼
GET /api/models/settings (src/settings/service/models/mod.rs)
  └─ 返回 { providers, enabledProviders, defaultModel, titleSummary, streamIdleTimeoutMs, revision }
       │
       ▼
modelsDraft & modelsBaseline (assets/settings_models.js)
  ├─ #view-model    共用 modelsDraft.{defaultModel, titleSummary}，挂载 actionBars.model ('actions-model')
  └─ #view-provider 共用 modelsDraft.providers，挂载 actionBars.provider ('actions-provider')
       │
       ▼
PUT /api/models/settings
  └─ merge_settings 校验 defaultModel 与 titleSummary -> 写入 ai.json -> 发送 SettingsEvent::ModelsChanged
```

### 3.2 `lx` 新对话自动标题生成数据流（对齐 `codex-main`）

```text
[用户在空白 lx 窗格输入首条消息并按 Enter]
       │
       ▼
try_send -> Effect::BeginChatTurn (session_id: None)
       │
       ▼
Persistence::execute_chat_effect -> db.begin_turn_with_attachments
  └─ 创建新会话 sess_xxx，初始 title = single_line_preview(user_text, 60)，turn_index = 1
       │
       ▼
update::on_turn_committed (src/app/update/chat/mod.rs)
  ├─ 绑定 target.lx.chat.session_id = Some(committed.session_id)
  ├─ 入队主对话流 Effect::StartChatStream
  └─ 若 committed.turn_index == 1（新会话首轮）：
       调用 title::maybe_schedule_session_title(state, pane, &committed, &frozen_draft)
         ├─ 解析模型：优先 state.models.settings.title_summary -> 回退本轮发送模型
         ├─ 构造压缩后的单轮 prompt（≤ 960 bytes，剥离 @ 引用标记）
         ├─ 在 state.pending_session_titles 登记 (session_id -> initial_title)
         └─ 入队 Effect::GenerateSessionTitle { workspace, session_id, initial_title, prompt, model }
       │
       ▼
event::chat::title::apply_title_effects (src/event/chat/title.rs)
  └─ tokio::spawn 后台执行轻量级 run_stream（无 tools、30s 超时、收集文本 ≤ 8KiB）
       └─ parse_generated_title 解析并清洗标题（≤ 36 chars）
            └─ 发送 AppEvent::SessionTitleGenerated { workspace, session_id, initial_title, title }
       │
       ▼
handle_app_event (src/event/mod.rs)
  ├─ 若该 session_id 仍在 state.pending_session_titles 中（未被手动 /rename 取消）：
  ├─ 校验 DB 中当前会话标题仍等于 initial_title（双重防覆盖保护）
  ├─ 调用 db.rename_chat_session(&workspace, &session_id, &title)（不改 updated_at_ms）
  └─ 调用 update::on_session_renamed 刷新所有打开中的 /resume 历史面板
```

### 3.3 Prompt 模板块（`&&&` / `+++`）`/title` 命令数据流

```text
[用户在 &&& 或 +++ 块内空行输入 `/title` 并按 Enter 或点击面板]
       │
       ▼
route_slash_panel / select_slash (src/app/update/editor.rs)
  └─ state.prompt.slash_confirm() (src/app/prompt/panel.rs)
       ├─ 命中 SlashInsertion::GenerateBlockTitle
       ├─ 定位光标所在最内层块（优先 `+++` Supple，否则外层 `&&&` Template）
       ├─ 提取块正文（剥离 `/title` 命令行、标题行、`{wt:}`、注释与空模板项）
       ├─ 清空命令行文本但保留空行：splice_text(trigger.line_start, trigger.line_end, "")
       │  （光标定位在 trigger.line_start，行末 `\n` 完整保留）
       ├─ 若提取后正文为空：
       │    登记 PromptTitleNotice::EmptyBlock -> update 层弹出 Warning Toast ("Template block is empty")
       └─ 若正文非空：
            ├─ 记录原 `{title: ...}` 行文本 original_title_line，并将该行改写为 `{title: Generating title...}`
            ├─ 生成唯一 request_id，登记 PendingBlockTitle 请求給 AppState
            └─ update 层消费该请求：
                 ├─ 解析模型：优先 titleSummary -> 回退 defaultModel
                 ├─ 若无可用模型：立即回滚 `{title: ...}` 为 original_title_line，
                 │                弹 Warning Toast ("No title model or default model configured")
                 └─ 若有可用模型：
                      ├─ commit_editing_prompt 同步当前缓冲
                      └─ 入队 Effect::GeneratePromptBlockTitle { request_id, prompt_id, workspace_idx, editing, page_id, block_start_line, original_title_line, content, model }
       │
       ▼
event::chat::title::apply_title_effects (src/event/chat/title.rs)
  └─ tokio::spawn 后台调用 run_stream -> parse_generated_title
       └─ 发送 AppEvent::PromptBlockTitleGenerated { request_id, outcome }
       │
       ▼
update::on_prompt_block_title_generated
  ├─ 定位目标工作区、目标条目/草稿与目标页（无论用户是否已切页或切工作区）：
  ├─ 若 outcome == Some(title)：
  │    将目标块的 `{title: Generating title...}` 替换为 `{title: <title>}`，并触发持久化保存
  └─ 若 outcome == None（失败/超时）：
       将目标块的 `{title: Generating title...}` 恢复为 original_title_line，
       并弹出全局 Warning Toast ("Failed to generate title")
```

---

## 4. 关键模块与关键代码设计

### 4.1 模型配置扩展：`AiSettings.title_summary`（`src/config/ai/mod.rs` & `src/app/state/lx/chat.rs`）

1. **`AiSettings` 新增 `title_summary` 字段**：
   - 在 `src/config/ai/mod.rs` 的 `AiSettings` 中正式纳管 `titleSummary`（不再落入 `extra` 并在保存时被清空）。
   - 在 `AiSettings::validate(&self)` 中复用与 `default_model` 相同的校验规则（抽取出 `validate_model_selection(label, selection, &self.providers, &enabled)` 函数，消除重复代码）。

```rust
// src/config/ai/mod.rs

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AiSettings {
    /// 启用的供应商 id 列表（lx-agent 写作 snake_case `enabled_providers`）。
    #[serde(default, rename = "enabled_providers")]
    pub enabled_providers: Vec<String>,
    #[serde(default)]
    pub providers: BTreeMap<String, ProviderConfig>,
    /// 默认模型选择（lx-agent 写作 camelCase `defaultModel`）。
    #[serde(
        default,
        rename = "defaultModel",
        skip_serializing_if = "Option::is_none"
    )]
    pub default_model: Option<ModelSelection>,
    /// 标题生成模型选择（兼容 lx-agent 的 camelCase `titleSummary`）。
    #[serde(
        default,
        rename = "titleSummary",
        skip_serializing_if = "Option::is_none"
    )]
    pub title_summary: Option<ModelSelection>,
    /// 流式空闲超时毫秒（lx-agent 写作 camelCase `streamIdleTimeoutMs`）。
    #[serde(
        rename = "streamIdleTimeoutMs",
        skip_serializing_if = "Option::is_none"
    )]
    pub stream_idle_timeout_ms: Option<u64>,
    /// 联网搜索 API Key 配置（`ai.webSearch`）。
    #[serde(default, rename = "webSearch")]
    pub web_search: WebSearchConfig,
    /// 未知字段往返保留。
    #[serde(flatten)]
    pub extra: serde_json::Map<String, serde_json::Value>,
}
```

2. **`ModelsSnapshot` 新增标题模型解析方法（`src/app/state/lx/chat.rs`）**：

```rust
// src/app/state/lx/chat.rs

impl ModelsSnapshot {
    /// 解析配置的标题模型（provider 启用、模型存在、有 Key 时返回规范化选择）。
    pub fn title_selection(&self) -> Option<ModelSelection> {
        let selection = self.settings.title_summary.as_ref()?;
        if !self.settings.enabled_providers.contains(&selection.provider) {
            return None;
        }
        let _ = self.endpoint_of(selection)?;
        Some(self.normalize_selection(selection.clone()))
    }

    /// 解析默认模型（provider 启用、模型存在、有 Key 时返回规范化选择）。
    pub fn default_selection(&self) -> Option<ModelSelection> {
        let selection = self.settings.default_model.as_ref()?;
        if !self.settings.enabled_providers.contains(&selection.provider) {
            return None;
        }
        let _ = self.endpoint_of(selection)?;
        Some(self.normalize_selection(selection.clone()))
    }

    /// 为 Prompt 模板块 `/title` 解析有效模型：优先 `titleSummary`，回退 `defaultModel`。
    pub fn resolve_prompt_title_model(&self) -> Option<FrozenModel> {
        let selection = self.title_selection().or_else(|| self.default_selection())?;
        FrozenModel::resolve(self, &selection)
    }

    /// 为 `lx` 新会话自动标题解析有效模型：优先 `titleSummary`，回退会话本轮模型。
    pub fn resolve_session_title_model(&self, fallback: &ModelSnapshot) -> Option<FrozenModel> {
        if let Some(selection) = self.title_selection()
            && let Some(frozen) = FrozenModel::resolve(self, &selection)
        {
            return Some(frozen);
        }
        let fallback_sel = self.normalize_selection(ModelSelection {
            provider: fallback.provider.clone(),
            model: fallback.model.clone(),
            variant: fallback.variant.clone(),
        });
        FrozenModel::resolve(self, &fallback_sel)
    }
}
```

---

### 4.2 设置服务与浏览器前端（`src/settings/service/models/mod.rs` & `assets/*`）

#### 4.2.1 后端 `/api/models/settings` 接口扩展（`src/settings/service/models/mod.rs`）

- `revision_of` 加入 `"titleSummary": &settings.title_summary`。
- `get_settings` 返回 JSON 中增加 `"titleSummary"` 字段（格式与 `"defaultModel"` 相同：`{ provider, model, variant }` 或 `null`）。
- `merge_settings` 解析 `body.get("titleSummary")`（提取通用辅助函数 `parse_model_selection_field(body, "defaultModel")` 与 `parse_model_selection_field(body, "titleSummary")`）。

#### 4.2.2 前端 HTML 结构拆分（`assets/settings.html`）

1. **左侧导航栏 `nav.side`**：
   ```html
   <nav class="side">
     <h1>lx-tui</h1>
     <button id="tab-general" class="active" onclick="showTab('general')">General</button>
     <button id="tab-commands" onclick="showTab('commands')">Commands</button>
     <button id="tab-model" onclick="showTab('model')">Model</button>
     <button id="tab-provider" onclick="showTab('provider')">Provider</button>
     <button id="tab-mcp" onclick="showTab('mcp')">MCP</button>
     <button id="tab-skills" onclick="showTab('skills')">Skills</button>
   </nav>
   ```
2. **新增 `#view-model` 视图**：
   ```html
   <section id="view-model" class="view hidden">
     <div class="view-head">
       <h2>Model</h2>
       <div id="actions-model" class="view-actions"></div>
     </div>
     <div class="models-toolbar">
       <button class="ghost" onclick="loadModels()">Reload</button>
       <span class="status" id="model-settings-status">Loading…</span>
     </div>
     <fieldset>
       <legend>Model Assignment</legend>
       <div class="field-grid">
         <div class="field-wide">
           <label>Default model</label>
           <div id="default-model-host" class="select-host"></div>
         </div>
         <div class="field-wide">
           <label>Title model</label>
           <div id="title-model-host" class="select-host"></div>
         </div>
       </div>
     </fieldset>
   </section>
   ```
3. **原 `#view-models` 调整为 `#view-provider` 视图**：
   - 标题改为 `<h2>Provider</h2>`，操作条挂载点为 `<div id="actions-provider" class="view-actions"></div>`（工具栏不再放 `Default model` 下拉框，仅保留 `Reload`、`#models-status` 与 `#models-revision`）。

#### 4.2.3 前端脚本更新（`assets/settings.js` & `assets/settings_models.js`）

- `assets/settings.js`：
  - `TAB_KEYS = ['general', 'commands', 'model', 'provider', 'mcp', 'skills']`。
  - `showTab(name)` 中映射 `model: 'tab-model'` / `'view-model'` 与 `provider: 'tab-provider'` / `'view-provider'`；当切换到 `'model'` 或 `'provider'` 且 `modelsDraft === null` 时调用 `loadModels()`。
- `assets/settings_models.js`：
  - `snapshotToDraft` 读取 `titleSummary: snapshot.titleSummary || null`。
  - 新增 `isModelSettingsDirty()`（比较 `defaultModel` 与 `titleSummary`）和 `isProviderSettingsDirty()`（比较 `providers`），分别绑定 `actionBars.model`（`#actions-model`）与 `actionBars.provider`（`#actions-provider`），两者的 `save` 均调用 `saveModels`，`reset` 调用 `resetModels`。
  - `renderDefaultModel()` 扩展为同时渲染 `modelSelects.defaultModel`（`#default-model-host`）与 `modelSelects.titleModel`（`#title-model-host`）；当某个 provider 被关闭或删除、或某个 model 被删除时，若 `titleSummary` 指向该项则自动清空并提示 `'Title model cleared ...'`。
  - `buildPayload()` 构造并附上 `titleSummary`。

---

### 4.3 提示词构造与标题解析模块（`src/chat/title.rs`）

新建 `src/chat/title.rs`（并在 `src/chat/mod.rs` 导出），集中实现与 `codex-main/codex-rs/tui/src/app/thread_title.rs` 对齐的纯函数（无 IO，易单测）：

```rust
// src/chat/title.rs

//! 会话与模板块标题生成的提示词组装、请求构造与响应清洗（对齐 codex `thread_title.rs`）。

use crate::chat::events::{ChatMessage, ChatRequest, ContentBlock, Role};

/// 标题最大字符数（Unicode 标量值计数，对齐 codex `THREAD_TITLE_MAX_CHARS = 36`）。
pub const TITLE_MAX_CHARS: usize = 36;
/// 标题生成完整 Prompt 最大字节数（对齐 codex `THREAD_TITLE_PROMPT_MAX_BYTES = 960`）。
pub const TITLE_PROMPT_MAX_BYTES: usize = 960;
/// 标题生成超时秒数（对齐 codex `STRUCTURED_TURN_TIMEOUT = 30s`）。
pub const TITLE_TIMEOUT_SECS: u64 = 30;
/// 标题响应流最大累计字节数（对齐 codex `MAX_OUTPUT_BYTES = 8 KiB`）。
pub const TITLE_MAX_OUTPUT_BYTES: usize = 8 * 1024;

/// 基础英文指令（对齐 codex `thread_title_instructions`）：
/// 要求不超 36 字符、祈使动词开头、保留工单号、跟随用户语言、无引号/Markdown/尾随标点。
pub fn title_instructions() -> &'static str {
    "Generate a concise, single-line task title of at most 36 characters and under five words \
where possible. Start with an imperative verb. Capitalize only the first word unless the user's \
language, proper nouns, acronyms, or code terms require otherwise. Preserve ticket references \
exactly. Write in the user's language. Do not use quotes, markdown, or trailing punctuation. \
Do not answer the request. Output only the title text."
}

/// 组装新对话标题生成 Prompt（总长钳制在 960 字节内，按 UTF-8 字符边界安全截断）。
pub fn session_title_prompt(user_message: &str) -> Option<String> {
    let trimmed = user_message.trim();
    if trimmed.is_empty() {
        return None;
    }
    let prefix = format!("{}\n\nUser prompt:\n", title_instructions());
    let budget = TITLE_PROMPT_MAX_BYTES.saturating_sub(prefix.len());
    let body = truncate_utf8_bytes(trimmed, budget).trim_end();
    if body.is_empty() {
        return None;
    }
    Some(format!("{prefix}{body}"))
}

/// 组装 Prompt 模板块（`&&&` / `+++`）标题生成 Prompt。
pub fn template_block_title_prompt(block_content: &str) -> Option<String> {
    let trimmed = block_content.trim();
    if trimmed.is_empty() {
        return None;
    }
    let prefix = format!("{}\n\nTask template content:\n", title_instructions());
    let budget = TITLE_PROMPT_MAX_BYTES.saturating_sub(prefix.len());
    let body = truncate_utf8_bytes(trimmed, budget).trim_end();
    if body.is_empty() {
        return None;
    }
    Some(format!("{prefix}{body}"))
}

/// 构造无工具、无系统提示负担的单轮轻量级 `ChatRequest`。
pub fn build_title_request(prompt: &str, model: &str) -> ChatRequest {
    ChatRequest {
        model: model.to_string(),
        system: None,
        messages: vec![ChatMessage {
            role: Role::User,
            content: vec![ContentBlock::Text {
                text: prompt.to_string(),
            }],
        }],
        tools: Vec::new(),
        max_output_tokens: Some(128),
        // 标题生成不传高阶思考预算，保持快速响应。
        variant_params: None,
    }
}

/// 解析并清洗模型返回的标题文本（兼容 JSON `{"title":"..."}` 与纯文本返回）：
/// 1. 剥离 `<think>...</think>` / `<thinking>...</thinking>` 标签；
/// 2. 若以 `{` 开头，尝试按 `{"title": "..."}` 解析；否则取首个非空行；
/// 3. 剥离首尾引号（`"`、`'`、`` ` ``、`“`、`”`、`‘`、`’`）与 Markdown 标题 `#` 前缀；
/// 4. 折叠连续空白为单个空格，剥离末尾句号/问号/感叹号（`.`、`?`、`!`、`。`、`？`、`！`）；
/// 5. 替换花括号 `{`、`}`（防止破坏 `{title: ...}` 协议）并按 36 字符上限截断。
pub fn parse_generated_title(raw: &str) -> Option<String> {
    let stripped = strip_think_blocks(raw);
    let trimmed = stripped.trim();
    if trimmed.is_empty() {
        return None;
    }
    let candidate = if trimmed.starts_with('{') {
        serde_json::from_str::<serde_json::Value>(trimmed)
            .ok()
            .and_then(|val| val.get("title").and_then(serde_json::Value::as_str).map(str::to_string))
            .unwrap_or_else(|| first_non_empty_line(trimmed).to_string())
    } else {
        first_non_empty_line(trimmed).to_string()
    };
    sanitize_title(&candidate)
}
```

---

### 4.4 Prompt 模板块 `/title` 命令与空行保留（`src/app/markdown/` & `src/app/prompt/`）

#### 4.4.1 斜杠命令协议新增 `/title`（`src/app/markdown/slash.rs`）

1. 在 `SlashInsertion` 枚举中新增变体：
   ```rust
   /// `/title`：为光标所在的 `&&&` 模板块或 `+++` 补充块生成标题（保留命令所在空行）。
   GenerateBlockTitle,
   ```
2. 在 `SlashItem` 新增构造函数：
   ```rust
   /// 内置 `/title` 候选（仅在 `&&&` 与 `+++` 块内可见）。
   pub fn builtin_title() -> Self {
       Self {
           name: "title".to_string(),
           source: CommandSource::BuiltIn,
           insertion: SlashInsertion::GenerateBlockTitle,
           preview: "Generate block title".to_string(),
           scope: SlashScope::Template,
           selectable: true,
       }
   }
   ```
3. 在 `slash_filter` 的 `BlockContext::Template | BlockContext::Supple` 分支中加入 `candidates.push(SlashItem::builtin_title());`：
   ```rust
   BlockContext::Template | BlockContext::Supple => {
       candidates.push(SlashItem::builtin_title());
       if !context.unclosed_block {
           candidates.push(SlashItem::builtin_supplement());
       }
       if context.worktree_available {
           candidates.push(SlashItem::worktree());
       }
   }
   ```

#### 4.4.2 补充块与模板块的范围及标题行读写协议（`src/app/markdown/blocks.rs` & `src/app/markdown/template.rs`）

1. **嵌套块识别（`+++` 嵌套在 `&&&` 内时的块定位）**：
   - 注意：`src/app/markdown/blocks.rs` 的 `block_scan` 在 `Some(Open::Template(start))` 分支中目前只匹配 `parse_template_end_line`，而 `+++` 块既可能位于 `&&&` 外部，也可能位于 `&&&` 内部！
   - 我们检查现有 `src/app/markdown/template.rs` 与 `src/app/prompt/template.rs`：
     - `copy_text_at_cursor` 先检查 `markdown::supple_block_start_at(&self.text, self.cursor)`，再检查 `markdown::template_block_start_at(&self.text, self.cursor)`。
     - 为确保当 `+++` 写在 `&&&` 内部或外部时，光标在 `+++` 内都能准确识别出内层 `+++` 的起止行与正文，在 `src/app/markdown/supple.rs` 中提供 `find_enclosing_supple_block(text: &str, cursor: usize) -> Option<(usize, usize, bool)>`（扫描未在代码围栏内的 `+++ <cmd> --start` 与 `+++ <cmd> --end` 配对，判断光标所在行是否落在该 `+++` 区间内）。
2. **`{title: ...}` 行读写与保护（`src/app/markdown/template.rs`）**：
   - 扩展 `title_markers(text: &str)`：不仅保护 `parse_template_start_line` 下一行的 `{title: ...}`，同时保护 `parse_supple_start_line` 下一行的 `{title: ...}`。
   - 新增纯函数 `read_block_title_line(text: &str, start_line: usize) -> Option<(usize, String)>` 与 `write_block_title(text: &str, start_line: usize, new_title: &str) -> Option<(std::ops::Range<usize>, String)>`：
     - 若 `start_line + 1` 行已是 `is_title_line`（即 `{title: ...}`）：返回该行的字节范围 `line_start..line_end` 与替换文本 `format!("{indent}{{title: {safe_title}}}")`；
     - 若 `start_line + 1` 行不是 `is_title_line`：在 `start_line` 行尾换行处插入 `\n{indent}{title: {safe_title}}`。

#### 4.4.3 确认 `/title` 时保留空行并登记标题请求（`src/app/prompt/panel.rs` & `src/app/prompt/template.rs`）

当用户在 `&&&` 或 `+++` 块内输入 `/title` 并按 `Enter`（或鼠标点击）触发 `Prompt::slash_confirm(&mut self)` 时：

```rust
// src/app/prompt/panel.rs

markdown::SlashInsertion::GenerateBlockTitle => {
    self.slash = None;
    self.confirm_generate_block_title(&trigger)
}
```

在 `Prompt::confirm_generate_block_title(&mut self, trigger: &markdown::SlashTrigger) -> bool` 中：
1. **定位目标块**：
   - 先用 `markdown::find_enclosing_supple_block(&self.text, self.cursor)` 查找是否处于 `+++` 补充块内；
   - 若不在 `+++` 内，则用 `markdown::template_block_start_at(&self.text, self.cursor)` 查找外层 `&&&` 模板块。
2. **清空 `/title` 命令行文本，但严格保留该空行**：
   - 记录撤销快照：`self.break_group(); self.record(EditKind::Other);`
   - 执行 `self.splice_text(trigger.line_start, trigger.line_end, "");`
   - **关键点**：`trigger.line_start..trigger.line_end` 仅包含该行的 `/` 与 `title` 字符，**不包含** `trigger.line_end` 处的 `\n` 换行符！因此替换为空串 `""` 后，该行自然变成一行空行，不会删除该行或吞掉上下行间距；光标置于 `self.cursor = trigger.line_start`。
3. **提取并清洗块正文**：
   - 提取该块起止行之间的正文（此时 `/title` 已被清空为 `""`），调用 `markdown::copy_template_content(&body)` 剔除 `{title: ...}` 行、`{wt: ...}` 行、注释行以及未填写的空模板列表项（如 `- Reference: `）。
   - 若清洗后的 `cleaned.trim().is_empty()`：
     - 将 `self.pending_title_action = Some(PromptTitleAction::EmptyBlock)` 存入 `Prompt`；
     - 调用 `self.settle()`、设置 `self.panel_suppressed = true` 并返回 `true`（`app::update` 随后取出 `EmptyBlock` 并展示 Warning Toast `"Template block is empty"`，同时刚才清空 `/title` 保留空行的编辑已生效）。
4. **写入加载占位 `{title: Generating title...}` 并登记异步请求**：
   - 记录块起始行 `start_line` 当前标题状态 `original: Option<String>`（若原本有 `{title: old}` 则为 `Some(old)`，否则为 `None`）。
   - 调用 `self.set_block_title_value(start_line, text::PROMPT_TITLE_GENERATING)` 将该块标题行设为 `{title: Generating title...}`（并按编辑区间平移 `self.cursor`，使光标仍然停留在刚才输入 `/title` 的那个空行行首！）。
   - 将 `PromptTitleAction::Generate { block_start_line: start_line, original_title: original, content: cleaned }` 存入 `self.pending_title_action`。

---

### 4.5 状态层与事件层编排（`src/app/update/` & `src/event/chat/title.rs`）

为严格遵守生产代码单文件 ≤ 1000 行规范（当前 `src/app/update/chat/session.rs` 已达 971 行），新增独立子模块：
- `src/app/update/chat/title.rs`（约 180 行）：处理 `lx` 会话自动标题与 Prompt 模板块标题的状态入队、取消与回填。
- `src/event/chat/title.rs`（约 200 行）：消费 `Effect::GenerateSessionTitle` 与 `Effect::GeneratePromptBlockTitle`，在后台 Tokio 任务中调用 `crate::chat::client::run_stream` 执行生成并回投 `AppEvent`。

#### 4.5.1 新增 `Effect` 与 `AppEvent` 变体

```rust
// src/app/effects.rs

/// 后台生成新对话标题（首轮提交后并行触发，不阻塞主对话流）。
GenerateSessionTitle {
    workspace: PathBuf,
    session_id: String,
    /// 首轮入库时的初始兜底标题：回写前核对，若已被用户手动改名则放弃覆盖。
    initial_title: String,
    prompt: String,
    model: crate::app::state::FrozenModel,
},
/// 后台生成 Prompt 模板块（`&&&` / `+++`）标题。
GeneratePromptBlockTitle {
    request_id: u64,
    workspace_index: usize,
    editing: crate::app::prompt::PromptEditing,
    page_id: crate::app::prompt::PromptPageId,
    block_start_line: usize,
    original_title: Option<String>,
    prompt: String,
    model: crate::app::state::FrozenModel,
},
```

```rust
// src/event/mod.rs

/// `lx` 新会话后台自动生成标题完成。
SessionTitleGenerated {
    workspace: PathBuf,
    session_id: String,
    initial_title: String,
    title: Option<String>,
},
/// Prompt 模板块 `/title` 后台生成标题完成。
PromptBlockTitleGenerated {
    request_id: u64,
    workspace_index: usize,
    editing: crate::app::prompt::PromptEditing,
    page_id: crate::app::prompt::PromptPageId,
    block_start_line: usize,
    original_title: Option<String>,
    title: Option<String>,
},
```

#### 4.5.2 手动改名防覆盖保障（`src/app/update/chat/title.rs` & `src/db/chat/mod.rs`）

1. **内存级取消**：
   - `AppState` 维护 `pending_session_titles: HashSet<String>`。
   - 新会话首轮触发 `GenerateSessionTitle` 时插入 `session_id`。
   - 当用户通过 `/rename`（`rename_session`）或 `/resume` 编辑弹窗（`submit_resume_session_edit`）手动修改某会话标题时，立即从 `state.pending_session_titles` 移除该 `session_id`。
2. **数据库级防覆盖校验**：
   - 在 `src/db/chat/mod.rs` 新增 `Database::rename_chat_session_if_title_matches(&self, workspace_path: &Path, session_id: &str, expected_title: &str, new_title: &str) -> Result<bool>`：
     ```sql
     UPDATE agent_session
        SET title = ?1
      WHERE external_id = ?2
        AND workspace_id = ?3
        AND title = ?4
     ```
   - 只有当数据库中的 `title` 仍然等于首轮入库时的 `initial_title` 时才更新为自动生成的标题，从根本上杜绝任何竞态覆盖用户手动命名的可能。

---

## 5. 影响文件与行数预算评估

| 文件路径 | 当前行数 | 预计改动后行数 | 职责说明 |
| --- | --- | --- | --- |
| `src/config/ai/mod.rs` | ~450 | ~480 | `AiSettings` 增加 `title_summary` 字段与校验 |
| `src/settings/service/models/mod.rs` | ~420 | ~460 | GET/PUT `/api/models/settings` 支持 `titleSummary` |
| `assets/settings.html` | 436 | ~465 | 拆分为 `Model` 与 `Provider` 两个 Tab 及视图 |
| `assets/settings.js` | 978 | ~985 | `TAB_KEYS` 与 `showTab` 适配 `model` 和 `provider` |
| `assets/settings_models.js` | 828 | ~880 | 绑定 `actionBars.model` / `actionBars.provider` 与 `titleModel` 下拉框 |
| `src/chat/title.rs`（新建） | 0 | ~220 | 标题生成提示词组装、UTF-8 字节截断、响应解析清洗与单测 |
| `src/app/markdown/slash.rs` | ~600 | ~625 | 新增 `SlashInsertion::GenerateBlockTitle` 与 `builtin_title()` |
| `src/app/markdown/supple.rs` | 127 | ~175 | 新增 `find_enclosing_supple_block` 定位内层 `+++` 块 |
| `src/app/markdown/template.rs` | ~520 | ~580 | 扩展 `title_markers` 支持 `+++`，新增块标题行读写辅助函数 |
| `src/app/prompt/panel.rs` | 435 | ~510 | `confirm_generate_block_title`（清空 `/title` 保留空行、写入加载态） |
| `src/app/update/chat/title.rs`（新建） | 0 | ~210 | 会话自动标题与模板块标题的入队、回填、回滚与防覆盖 |
| `src/event/chat/title.rs`（新建） | 0 | ~190 | 后台 Tokio 任务执行 `run_stream` 标题生成并回投 `AppEvent` |
| `src/db/chat/mod.rs` | ~620 | ~655 | 新增 `rename_chat_session_if_title_matches` 条件更新 |

> 所有生产代码文件均严格控制在 1000 行红线以内，新建子模块均在 200 行左右，职责单一清晰。

---

## 6. 异常处理与边界情况

1. **未配置任何模型**：
   - `lx` 新会话发送本身已要求存在有效模型，因此首轮必然有本轮发送模型兜底；
   - Prompt 模板块 `/title` 若既无 `titleSummary` 也无 `defaultModel`，立即将 `{title: ...}` 恢复原样并弹出英文 Warning Toast `"No title model or default model configured"`。
2. **模板块内含未填写的空占位项**：
   - 如刚插入 `/add` 模板，块内仅有 `# Add Requirement`、`- Reference: ` 等空壳，调用 `clean_template_content` 后若无实质内容（或仅剩默认空模板标题），判定为空块，弹出 `"Template block is empty"`。
3. **输入 `/title` 后空行与光标位置**：
   - 无论 `/title` 位于块内哪一行，确认后只清空该行文字、保留该空行，且即使上方插入或改写了 `{title: Generating title...}`，光标也通过字节偏移补偿稳稳停在原空行行首。
4. **异步生成期间用户切换了 Prompt 页或编辑了正文**：
   - 回填时按 `(workspace_index, editing, page_id, block_start_line)` 定位目标内容（若当前正在编辑该页则直接更新 `state.prompt` 并 `commit_prompt_buffer`，若已切到别的页则更新后台页缓冲并入队 `SavePromptContent`），且只在目标块的标题行仍为 `{title: Generating title...}` 时才覆盖，避免覆盖用户在生成期间手动改写的标题。
