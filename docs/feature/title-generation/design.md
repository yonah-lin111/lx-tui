# 会话与模板块标题生成（含 Settings Model/Provider 拆分）：设计方案

> 状态：**已完成并合并至主工作区 `dev` 分支（原工作区 `.worktrees/title-generation` 与分支 `feat/title-generation` 已清理）**
>
> 关联文档：
> - 任务清单：`docs/feature/title-generation/task.md`
> - 代码审查：`docs/feature/title-generation/review.md`
>
> 参考实现：
> - `/Users/yonah/projects/agent/codex-main/codex-rs/tui/src/app/thread_title.rs`
> - `/Users/yonah/projects/agent/lx-agent/src/main/agent/titleGenerator.ts`
> - `/Users/yonah/projects/agent/lx-agent/src/renderer/src/features/markdown/hooks/useMarkdownCommandRunners.ts`

---

## 1. 背景与目标

### 1.1 现状分析

1. **浏览器设置页（`src/settings/` & `assets/`）**：
   - 左侧导航栏此前为 `General`、`Commands`、`Models`、`MCP`、`Skills` 共 5 个 Tab。
   - 原 `Models` 页同时混放了顶部的 `Default model` 下拉框与下方的供应商（Provider）及模型 CRUD 列表，且 `AiSettings` 中尚未支持配置专用的标题生成模型（`titleSummary`）与标题输出语言（`titleLanguage`）。
2. **`lx` 聊天新对话标题（`src/ui/main_content/lx/` & `src/db/chat/`）**：
   - 新会话首轮用户消息落库时（`Database::begin_turn_with_attachments`），仅截取用户输入前 60 个字符作为 `agent_session.title`，且 `lx` 输入框右上角没有当前会话标题展示。
3. **右侧 Prompt 编辑器模板块（`src/ui/prompt/` & `src/app/prompt/`）**：
   - 任务块（`&&& <cmd> --start ... &&& <cmd> --end`）与补充块（`+++ <cmd> --start ... +++ <cmd> --end`）支持 `{title: ...}` 结构性标题，但块内 `/` 斜杠命令面板缺少一键提炼并回写块标题的 `/title` 命令。

### 1.2 核心目标

1. **设置页拆分 `Model` 与 `Provider` Tab**：
   - 将浏览器设置页左侧 Tab 拆分为 `General`、`Commands`、`Model`、`Provider`、`MCP`、`Skills`。
   - 原 `Models` 页的供应商与模型列表管理调整至 `Provider` Tab（`#tab-provider` / `#view-provider`）。
   - 新增独立的 `Model` Tab（`#tab-model` / `#view-model`），分为两个上下独立的 `<fieldset>` 卡片：
     - **`Default Model`**：配置 `Default model`（`defaultModel`）；
     - **`Title Generation`**：配置 `Title model`（`titleSummary`，兼容 `lx-agent`）与 `Title language`（`titleLanguage`，默认 `auto` 自动跟随用户输入语言，可选 `zh-CN`、`en`、`zh-TW`、`ja`、`ko`）。
2. **`lx` 新对话自动生成标题与输入框右上角柔和标题胶囊**：
   - 当新会话首轮用户消息落库成功（`on_turn_committed` 且 `turn_index == 0`）时，立即在后台并行发起无工具、轻量级的标题生成请求，不阻塞主对话流式输出。
   - 生成前数据库保留首条消息前 60 字符作为初始兜底标题；生成成功后更新 `agent_session.title`、已打开的 `/resume` 历史列表，并在 **`lx` 输入框右上角顶边框内（`rect.y` 最右内缩 1 列 `─`）** 显示会话标题胶囊（` <title> `）；若模型请求失败或拒答被拦截，则自动回退使用首轮干净预览标题更新顶边框胶囊。
   - 胶囊配色收口于 `src/ui/style.rs`（`SESSION_TITLE_PALETTE` 与 `session_title_badge`），结合 Claude Code 暗色胶囊与 LX 品牌色调，采用 **8 套低饱和柔和暗底（`bg` 相对亮度 `<= 0.04`）+ 同色系柔和粉彩字体色（`fg`，对比度 `5.5..=10.0`、常规字重不加粗，显式剥离 `DIM` / `REVERSED` / `HIDDEN`）**，按会话锁定配色（相邻会话不撞色、同会话改名不闪烁）；`/rename` 改名与 `/resume` 恢复历史会话同步显示，`/new` 新建或删除会话时清空隐藏。
   - 严格遵循手动命名优先（防覆盖）：若后台生成期间用户已通过 `/rename`、`/resume` 编辑弹窗手动改名或删除会话，自动生成的标题直接丢弃。
3. **Prompt 模板块（`&&&` 和 `+++`）新增 `/title` 命令**：
   - 当光标位于 `&&&`（`BlockContext::Template`）或 `+++`（`BlockContext::Supple`）块内时，`/` 候选面板提供内置命令 `/title`。
   - 确认 `/title` 后：**清空命令所在行的 `/title` 文本，但保留该空行本身（保留行尾 `\n`，光标停在该空行行首）**。
   - 提取光标所在最内层块（优先 `+++`，否则外层 `&&&`）的有效正文；若为空则弹出英文 Warning Toast（`"Template block is empty"`）。
   - 发起异步生成期间，仅在内存编辑器中将目标块的标题更新为 `{title: Generating title...}`（不提前将瞬态占位符写入 SQLite），并在 `AppState.pending_prompt_titles` 中按 `request_id` 登记；生成完成后校验 `request_id` 写回 `{title: <生成的标题>}` 并一次性持久化保存（同时兼容起始行内联标题与下一行独立标题行，同步 `PromptMode::Pages` 与停泊工作区缓冲）；若失败则恢复原 `{title: ...}` 并弹出 Warning Toast。

---

## 2. 已确认边界（Grill Me 决策表）

| # | 决策维度 | 已确认结论 |
| --- | --- | --- |
| 1 | 设置页导航与模型配置结构 | 左侧导航栏拆分为 `Model` 和 `Provider` 两个 Tab：原供应商与模型 CRUD 放入 `Provider` 页；`Default Model` 与 `Title Generation`（`Title model` + `Title language`）分为两个独立 `<fieldset>` 放入 `Model` Tab。`AiSettings` 新增 `title_summary: Option<ModelSelection>` 与 `title_language: Option<TitleLanguage>`，加载时采用「宽容加载、严格保存」（悬空 `titleSummary` 或未知 `titleLanguage` 自动降级清理，不阻塞整份 `ai.json` 加载）。 |
| 2 | `Title model` 选择与三级回退策略 | 优先使用 `Title model`（`titleSummary`）；若未配置或不可用则自动回退：<br>1. **`lx` 新对话标题生成**：`titleSummary` $\to$ 窗格当前有效选择（校验启用与 API Key）$\to$ `defaultModel`；若流式生成失败或拒答，回退使用清洗后的首轮预览标题展示在顶边框胶囊中，不弹报错。<br>2. **Prompt 模板块 `/title` 命令**：`titleSummary` $\to$ `defaultModel`；若均未配置或无有效 Key，立即回滚标题行并弹出 Warning Toast `"No title model or default model configured"`。 |
| 3 | `lx` 新对话标题触发、胶囊展示与防覆盖 | 1. **触发时机**：新会话首轮用户消息落库回执（`on_turn_committed` 且 `turn_index == 0`）时立即后台并行触发。<br>2. **展示位置**：更新 SQLite `agent_session.title`、已打开的 `/resume` 历史面板，以及 `lx` 输入框右上角顶边框内的柔和暗色标题胶囊（` <title> `）。<br>3. **提示词与防拒答清洗**：全英文系统提示词（`TITLE_SYSTEM_PROMPT` + `title_instructions_for`），明确要求无论编码任务、提问、问候或身份询问均提炼简短主题，严禁拒答；`parse_generated_title` 支持剥离 `<think>`、```` ```json ```` 围栏与 `Title:` 前缀，并在截断前拦截多语言拒答元解释文本。<br>4. **防覆盖**：内存 `pending_session_titles` + SQLite `WHERE title = ?4` 双重校验，手动 `/rename`、`/resume` 改名或删除会话立即作废在途请求。 |
| 4 | Prompt 模板块 `/title` 交互、空行保留与异步回写 | 1. **可见范围**：仅在 `BlockContext::Template`（`&&&`）与 `BlockContext::Supple`（`+++`）内部显示 `/title`。<br>2. **保留空行**：确认命令时仅将 `trigger.line_start..trigger.line_end` 替换为空串 `""`，**保留行尾 `\n`**，光标停在空行行首。<br>3. **加载占位与 `request_id` 回写**：生成期间仅在内存展示 `{title: Generating title...}`（不提前落库占位符）；通过 `AppState.pending_prompt_titles` 校验 `request_id`，兼容 `PromptMode::Editor`、`PromptMode::Pages` 及停泊工作区缓冲，完成后再落盘最终标题或回滚 `original_title`。 |

---

## 3. 核心架构与数据流设计

### 3.1 浏览器设置页：`Model` 与 `Provider` Tab 拆分数据流

```text
[浏览器打开 Settings 页]
       │
       ├─ Left Nav:
       │    ├─ General   (#tab-general  -> #view-general)
       │    ├─ Commands  (#tab-commands -> #view-commands)
       │    ├─ Model     (#tab-model    -> #view-model)     <-- Default Model & Title Generation 两个独立 fieldset
       │    ├─ Provider  (#tab-provider -> #view-provider)  <-- 供应商与模型 CRUD
       │    ├─ MCP       (#tab-mcp      -> #view-mcp)
       │    └─ Skills    (#tab-skills   -> #view-skills)
       │
       ▼
GET /api/models/settings (src/settings/service/models/mod.rs)
  └─ 返回 { providers, enabledProviders, defaultModel, titleSummary, titleLanguage, streamIdleTimeoutMs, revision }
       │
       ▼
modelsDraft & modelsBaseline (assets/settings_models.js)
  ├─ #view-model    管理 defaultModel / titleSummary / titleLanguage，仅列出已启用 provider 的模型
  ├─ #view-provider 管理 providers CRUD，新增/抓取/改名模型后自动同步刷新 Model 页下拉框
  └─ saveAllDirty   按 save 函数去重，单次 PUT 提交并保留已有合法 variant 与顶层未知扩展字段
```

### 3.2 `lx` 新对话自动标题生成与顶边框胶囊数据流

```text
[用户在 lx 窗格发送新会话首条消息]
       │
       ▼
Persistence::execute_chat_effect -> db.begin_turn_with_attachments
  └─ 创建新会话 sess_xxx，初始 title = single_line_preview(user_text, 60)，turn_index = 0
       │
       ▼
update::on_turn_committed (src/app/update/chat/mod.rs)
  └─ 若 committed.turn_index == 0：
       调用 title::maybe_schedule_session_title(state, pane, &committed, &frozen_draft)
         ├─ 三级模型解析：titleSummary -> 窗格当前有效模型 -> defaultModel
         ├─ 按 TitleLanguage（默认 Auto 识别简中/繁中/日/韩/英）构造 ≤ 960 bytes 英文提示词
         ├─ 在 state.pending_session_titles 登记 session_id
         └─ 入队 Effect::GenerateSessionTitle { workspace, session_id, initial_title, prompt, model }
       │
       ▼
event::chat::title::apply_title_effects (src/event/chat/title.rs)
  └─ tokio::spawn 后台执行 run_stream（TITLE_SYSTEM_PROMPT、无 tools、30s 超时、≤ 8KiB）
       └─ parse_generated_title 剥离 <think> / 代码围栏 / Title: 前缀并拦截拒答话术
            └─ 发送 AppEvent::TitleGenerated(TitleGeneratedEvent::Session { ... })
       │
       ▼
event::chat::title::handle_session_title_generated
  ├─ 校验 take_pending_session_title（若已被手动 /rename 或删除则直接丢弃）
  ├─ 若 title 为 None（失败或拒答被拦截）：回退用 parse_generated_title(initial_title) 更新窗格顶边框胶囊
  └─ 若 title 为 Some(new_title)：
       ├─ db.rename_chat_session_if_title_matches（仅当 DB title == initial_title 时更新）
       └─ update::apply_generated_session_title 更新窗格 LxChat.session_title 胶囊与 /resume 面板
```

### 3.3 Prompt 模板块（`&&&` / `+++`）`/title` 命令数据流

```text
[用户在 &&& 或 +++ 块内输入 `/title` 并确认]
       │
       ▼
Prompt::confirm_generate_block_title (src/app/prompt/panel.rs)
  ├─ 定位光标所在最内层块（优先 `+++` Supple，否则外层 `&&&` Template）
  ├─ 清空命令行文本但保留空行：splice_text(trigger.line_start, trigger.line_end, "")
  ├─ 提取并清洗块正文（extract_block_title_source）
  ├─ 若正文为空：登记 PromptTitleAction::EmptyBlock -> 弹 Warning Toast "Template block is empty"
  └─ 若正文非空：
       ├─ 读取原块标题（兼容起始行内联 `{title: ...}` 与下一行独立 `{title: ...}`）
       ├─ 在内存缓冲中将块标题设为 `{title: Generating title...}`（不提前写 DB）
       └─ 登记 PromptTitleAction::Generate { block_start_line, original_title, content }
       │
       ▼
update::chat::title::flush_prompt_title_action
  ├─ 解析模型：优先 titleSummary -> 回退 defaultModel（无模型则立即回滚并弹 Toast）
  ├─ 生成递增 request_id，在 state.pending_prompt_titles 记录 PendingPromptTitle（自动覆盖同块旧请求并保留初始 original_title）
  └─ 入队 Effect::GeneratePromptBlockTitle
       │
       ▼
event::chat::title::handle_prompt_block_title_generated -> update::on_prompt_block_title_generated
  ├─ 校验 state.pending_prompt_titles.remove(&request_id) 命中（过期请求直接丢弃）
  ├─ 按 workspace_cwd + workspace_index 解析目标工作区，同步更新显示缓冲（含 Editor 与 Pages 模式）、停泊工作区缓冲及页树
  ├─ 生成成功：写回 `{title: <title>}` 并触发持久化保存
  └─ 生成失败：恢复为 original_title 并弹出 Warning Toast "Failed to generate title"
```

---

## 4. 影响文件与模块职责清单

| 文件路径 | 职责说明 |
| --- | --- |
| `src/config/ai/mod.rs` & `tests.rs` | `TitleLanguage` 枚举、`AiSettings.{title_summary, title_language}`、加载时 `sanitize_optional_selections` 容错与严格保存校验 |
| `src/settings/service/models/mod.rs` & `tests.rs` | `GET`/`PUT` `/api/models/settings` 支持 `titleSummary` 与 `titleLanguage`，保留顶层 `extra` 未知字段 |
| `assets/settings.html` / `settings.js` / `settings_models.js` / `settings.css` | 拆分 `Model`（`Default Model` 与 `Title Generation` 独立 fieldset）与 `Provider` Tab，跨 Tab 下拉框同步、保留自定义 `variant`、去重保存 |
| `src/chat/title.rs` & `src/chat/title/tests.rs` | 纯英文系统提示词、多语言 `Auto` 识别、代码块围栏与 `Title:` 剥离、截断前多语言拒答拦截 |
| `src/app/markdown/{slash,supple,template}.rs` | `/title` 斜杠候选、内层 `+++` 定位、兼容内联与独立 `{title: ...}` 行的读写与保护 |
| `src/app/prompt/{mod,panel,template}.rs` | 确认 `/title` 保留空行、内存加载占位与光标偏移补偿 |
| `src/app/state.rs` & `src/app/state/lx/chat.rs` | `PendingPromptTitle`、`LxChat.{session_title, session_title_palette}`、三级标题模型回退链 |
| `src/app/update/chat/title.rs` & `tests.rs` | 会话标题与模板块标题的调度、`request_id` 校验、跨模式/跨工作区缓冲回填与回滚 |
| `src/event/chat/title.rs` & `tests.rs` | `TitleGeneratedEvent`、后台 30s 超时流式请求与事件分发 |
| `src/ui/style.rs` & `src/ui/main_content/lx/{input,grid}.rs` | 8 套柔和暗底粉彩字胶囊语义配色（`SESSION_TITLE_PALETTE` / `session_title_badge`）、零宽字符对齐与右上角顶边框胶囊渲染 |
| `src/db/chat/mod.rs` & `src/db/service/chat.rs` | `chat_session_title` 查询、`rename_chat_session_if_title_matches` 条件防覆盖更新、`/resume` 恢复会话标题回显 |
