# lx `AI Web` 入口与 DeepSeek 网页端接入 — 执行任务清单

> **命名迁移提示**：该特性完成后，入口 `AI Web` 已统一更名为 `Web`，Rust 模块路径迁移至 `src/*/web*`，驱动规范迁移至 [`docs/web/web-deepseek.md`](../../web/web-deepseek.md)（并新增 [`docs/web/web-gemini.md`](../../web/web-gemini.md) 与 [`docs/web/web-grok.md`](../../web/web-grok.md)）；本文保留实现时的历史命名，并增补 P6 Web Markdown 保真提取与 TUI 渲染增强及专项 Review 闭环。

> 状态：**已完成实现、代码审查（[review.md](review.md)）、P0–P3 修复并合并至 `dev`**（2026-11-10 更新 Markdown 保真闭环）
> 设计依据：[design.md](design.md) · 审查报告：[review.md](review.md) · 驱动规范：[`docs/web/web-deepseek.md`](../../web/web-deepseek.md) · [`docs/web/web-gemini.md`](../../web/web-gemini.md) · [`docs/web/web-grok.md`](../../web/web-grok.md)
> 阶段划分：P0（开工门槛与基线核对）→ P1（`app/` 状态模型与云同步会话更新）→ P2（`ui/` 欢迎页 4 入口、标题、居中加载/空态与底边框渲染）→ P3（`chat/ai_web` 与 `event/` 回环 HTTP 流、云会话与 CDP 文件上传）→ P4（单元测试与精确校验）→ P5（代码审查 P0–P2 问题闭环修复与规范沉淀）→ P6（Web Markdown 保真提取、流式渲染增强与专项 Review P0–P3 修复闭环）。

---

## P0. 开工门槛与基线核对

- [x] 用户审核确认 [design.md](design.md) 与本 `task.md` 后开始执行。
- [x] 遵守 `AGENTS.md` 与 `docs/standards/*.md` 全部规范：生产代码禁止 `unwrap()` / `expect()`，TUI 用户可见文案 100% 英文并集中在 `src/ui/text.rs`，保持 `app/` 与 `ui/` 纯函数边界，生产代码单文件 `< 1000` 行且测试分离至同层 `tests.rs`。

---

## P1. 状态模型与云同步会话更新（`src/app/`）

- [x] **扩展 `LxEntry` 与 `AiWebSite`（`src/app/state/lx.rs`、`src/app/effects.rs`）**：
  - 在 `LxEntry` 中新增 `AiWeb` 变体，更新 `LxEntry::ALL` 为 4 项 `[Native, Hybrid, Automation, AiWeb]`。
  - 新增 `AiWebSite` 枚举（首期 `Deepseek`）并在 `Lx` 结构体中新增 `ai_web_site`、`ai_web_edit_user_index` 与 `ai_web_edit_cloud_files`。
  - 在 `Effect` 中新增 `StartAiWebStream { key, site, prompt, files, keep_files, edit_user_index }` 与 `ResetAiWebSession { site }`。
- [x] **实现 `AI Web` 发送、状态栏刷新、云会话管理与 `/rewind`（`src/app/update/chat/ai_web.rs`、`mod.rs`、`session.rs`、`slash.rs`、`rewind.rs`、`resume_edit.rs`）**：
  - `try_send` 在 `LxEntry::AiWeb` 时调用 `try_send_ai_web`：创建内存态用户与助手消息，入队 `Effect::StartAiWebStream`，不触发任何 SQLite 写效应；在 `AI Web` 下屏蔽 `!` 本地 Shell 模式。
  - `refresh_lx_status` 在 `LxEntry::AiWeb` 时将窗格底边框状态栏 `model_name` 设为 `"DeepSeek"`，清空 `provider_name`、`variant`、`context_limit`、`input_tokens`。
  - 在 `AI Web` 下开放 `/new`（`/clear`）、`/resume`（`/session`）、`/rename`（`/title`）与 `/rewind`（`/undo` 及空闲双击 `Esc`），全部与 DeepSeek 网页端云同步（会话列表、历史恢复、`Shift+Enter` 编辑模态改名、`Ctrl+D` 删除、以及 1 步撤回后原地编辑重发）；对不支持的本地命令（含带参命令如 `/model foo`）弹出英文提示。

---

## P2. 欢迎页 4 入口布局、标题、居中空态/加载态与底边框渲染（`src/ui/`）

- [x] **收短入口标签、无方括号标题与渐变（`src/ui/text.rs`、`src/ui/panes.rs`、`src/ui/style.rs`、`src/ui/main_content/lx/brand.rs`）**：
  - 将 4 个入口标签定义为 `"Native"`、`"Hybrid"`、`"Automation"`、`"AI Web"`；窗格左上角边框标题统一为 `lx-<entry>`（无方括号）。
  - 为 `LxEntry::AiWeb` 配置独立渐变色与像素图标配色。
- [x] **修复欢迎页多入口动态间距并新增站点选择行（`src/ui/main_content/lx.rs`、`src/ui/main_content/lx/brand.rs`）**：
  - 将 `place_entries` 与 `layout` 中的入口数改为按 `labels.len()` 动态计算。
  - 当 `lx.entry == LxEntry::AiWeb` 时，在 `lay.inventory` 槽位绘制 `DeepSeek` 站点选择行并提供 `ai_web_site_hit` 鼠标命中。
- [x] **思考块默认折叠与全局空态/加载态居中（`src/ui/main_content/lx/history/`、`src/ui/main_content/lx/slash.rs`、`src/ui/overlay/`、`src/ui/prompt/`、`src/ui/sidebar*.rs`）**：
  - `PartKind::Reasoning` 在流式生成中和完成后均保持默认折叠（`● Thinking` → `○ Thought · Xs`），手动展开跨完成态保持。
  - `/resume` 列表加载中居中显示 `loading sessions…`，会话历史加载中在聊天视口正中央显示 `loading history…`（窄窗格自动截断）；统一将所有面板、弹窗、列表的空状态与加载状态文案居中渲染。

---

## P3. `ego-browser` DeepSeek 回环 HTTP 流、云会话与图片/文件上传（`src/chat/`、`src/event/`）

- [x] **编写嵌入式 DeepSeek 驱动脚本与协议解析（`src/chat/ai_web/mod.rs`、`deepseek_helpers.js`、`deepseek_script.js`）**：
  - 通过块级 `const __LX_PARAMS` 注入每轮参数，经本地回环 HTTP POST（`streamUrl`）+ stdout/stderr 回退实时推送 `thinking` / `text` / `session` / `sessions` / `session_loaded` / `login_required` / `done` / `error` 事件。
  - 拦截 `/api/v0/chat/completion`、`/api/v0/chat/edit_message`、`/api/v0/chat/regenerate` 的 XHR/fetch SSE 流并附带 DOM 兜底。
- [x] **实现 `AI Web` 图片/文件上传、禁用 `refDir`/文件夹及 `/rewind` 附件同步（`src/chat/prepare/ai_web.rs`、`src/ui/main_content/lx/mention.rs`、`deepseek_helpers.js`）**：
  - `AI Web` 下禁用 `refDir` 与文件夹发送：`@` 面板仅展示 `Files` 单一 Tab，在 `Files` 中确认目录自动转为进入目录；快捷引用与路径粘贴面板不提供文件夹引用。
  - `prepare_ai_web_message` 解析本地文件/图片引用与 `/rewind` 保留的云端附件名 `keep_files`，填充 `ChatMessageView.attachments` 并剥离正文引用标记。
  - 脚本通过 CDP `DOM.setFileInputFiles` 注入本地文件，按 `keepFiles` 精准保留或关闭编辑框内旧附件卡片，等待全部卡片（`keptCount + FILES.length`）就绪后发送。

---

## P4. 单元测试与精确校验

- [x] 更新并新增单元测试：
  - `src/ui/main_content/lx/tests.rs` 与 `tests/slash_panels.rs`：4 入口欢迎页、站点命中、`lx-<entry>` 标题、居中 `loading sessions…` 与 `loading history…`。
  - `src/ui/main_content/lx/history/tests.rs`：思考块流中与完成后默认折叠、手动展开保持。
  - `src/chat/ai_web/tests.rs` 与 `src/chat/prepare/tests.rs`：协议行解析、`prepare_ai_web_message` 文件/图片解析、云端附件保留与目录拦截。
  - `src/app/update/chat/ai_web/tests.rs`：`AI Web` 发送、`/new`、`/resume`、`/rename`、`/rewind`（含本地与云端附件增删同步）、带参斜杠命令拦截、入口切换会话隔离、`@` 面板仅保留 `Files` Tab 及目录确认进入子目录。

---

## P5. 代码审查（[review.md](review.md)）P0–P2 修复闭环

- [x] **P0-1**：`AI Web` 下彻底禁用 `refDir` 与文件夹引用（`@` 面板仅保留 `Files` Tab、确认目录转为进入目录、入口切换强制重置 Tab、过滤快捷引用与粘贴面板的目录候选）。
- [x] **P0-2**：修复多文件上传 `status.count > 0` 提前返回问题（改为轮询旧卡片降至 `keptCount` 并等待 `status.count >= keptCount + FILES.length`），并在状态检测前剥离已知文件名防止 `error.rs` 等误判失败。
- [x] **P0-3**：修复纯图片/纯文件无文字消息（`PROMPT === ""`）发送后误点停止按钮中断回答的问题。
- [x] **P0-4**：修复 `AI Web` 与 `Native` 互切入口时的会话状态泄漏，并在 `AI Web` 下屏蔽 `!` 本地 Shell 模式。
- [x] **P1-1**：为 `/resume` 云端历史消息合成 `@[refer-*](filename)` 标记，支持 `/rewind` 后按 `keepFiles` 精准保留/移除云端卡片；原地编辑失败时显式报错，禁止静默退化为底部发新消息。
- [x] **P1-2**：拦截 `AI Web` 下所有带参数的斜杠命令（如 `/model foo`），防止误发给网页端。
- [x] **P1-3**：为 `list_sessions` / `load_session` 增加 HTTP 响应校验、30 秒进程超时与未命中终态时的 `ActionFailed` 兜底；删除活跃会话时先停止在途流。
- [x] **P1-4**：仅在收到有效 SSE 数据帧时置 `sseActive = true`，并要求显式收到 `Done` 事件才按 `Complete` 结算。
- [x] **P2-1 ~ P2-3**：拆分 `deepseek_helpers.js` + `deepseek_script.js` 与 `src/chat/prepare/ai_web.rs`，将生产文件内联测试迁入同层 `tests.rs`，修复纯文件夹预览 `[Folder]` 与窄窗格 `loading history…` 截断，并同步更新 `design.md`、`task.md` 与 `review.md`。
- [x] **交互回归与规范沉淀**：修复 `/rewind` 撤销全部轮次回到欢迎首页时输入框右上角会话标题胶囊未清空的问题（`ca5e6c1`），并编写 [`docs/web/web-deepseek.md`](../../web/web-deepseek.md) 规范文档（`57f019a`）。

---

## P6. Web Markdown 保真提取、TUI 渲染增强与专项 Review（[review.md](review.md)）P0–P3 修复闭环

- [x] **新增跨站点共享 DOM → Markdown 序列化器（`src/chat/web/dom_markdown.js`、`src/chat/web/mod.rs`）**：
  - 实现 `lxHtmlToMarkdown(root, { streaming })` 并通过 `installDomMarkdown()` 注入 `window.__lxHtmlToMarkdown`，支持标题、粗体/斜体/删除线、行内代码、围栏代码块（DeepSeek/Gemini/Grok 语言提取与 `markdown` 四反引号围栏）、GFM 表格、无序/有序/任务列表（含多段 `<li>` 续行缩进对齐）、数学公式（KaTeX/MathML）与流式尾部闭合标记暂缓（`isTail`）。
- [x] **DeepSeek / Gemini / Grok 全链路移除 `innerText` 降级，统一保真提取 Markdown（`deepseek_script.js`、`deepseek_helpers.js`、`gemini_script.js`、`gemini_helpers.js`、`grok_script.js`）**：
  - DeepSeek：拦截非 SSE 质询 XHR，SSE 通道与 DOM 通道严格互斥（`sseThinking || sseText`），DOM 兜底优先读受边界保护的 `readFiberMarkdown`（`depth < 10`）与 `window.__lxHtmlToMarkdown`，`finishSse()` 补刷暂缓的末尾 `[`，`load_session` 仅在代码块外剥离 `[citation:...]` / `[reference:...]` 且只吃水平空白 `[ \t]*`。
  - Gemini：按子选择器优先顺序定位 `.thoughts-content` 与 `message-content .markdown`，在流式与 `/resume` 历史加载中全面使用 `window.__lxHtmlToMarkdown`。
  - Grok：新增 CDP `Network.webSocketFrameReceived` 帧采集（直接读取 `CHANNEL_ASSISTANT_RESPONSE` 原始 Markdown 与 `render_citation` 链接）+ `window.__lxHtmlToMarkdown` DOM 兜底。
- [x] **TUI 面向阅读的 Markdown 渲染器增强（`src/ui/markdown_render.rs`、`src/ui/markdown_render/inline.rs`、`src/ui/markdown_render/code_block.rs`）**：
  - `inline_spans` 支持强调块（`***`、`**`/`__`、`~~`、`*`/`_`）内部递归解析行内代码、链接与内层强调，支持多反引号 `` ``code`` ``，并在尾随 `***` 与反引号内 `**` 上精准对齐闭合边界。
  - `ParserState` 新增 `list_hangs` 栈与逐层 `pop()` 回退，使无序（2 列）、有序（`N. ` 宽度）与任务（4 列）列表续段严格对齐正文列，并在 `code_block.rs` 中剥离列表内围栏代码块的前导缩进。
- [x] **专项 Review P0–P3 全部 11 项问题修复与回归测试（详见 [review.md](review.md) §2）**：
  - **P0-1**：修复 `gemini_script.js` / `deepseek_script.js` 正文开始后思考块仍传 `streaming: true` 导致末尾闭合 `**` / ```` ``` ```` 迟到分裂出第二个 `○ Thought` 块的问题。
  - **P0-2**：修复 `grok_script.js` 搜索/工具卡片（`tool_usage_card`）在首段文本后出现导致 WS 与 DOM 双通道死锁，以及跨 WS 帧分割 `<grok:render>` / `<argument>` 泄漏引用编号的问题。
  - **P0-3**：修复 `deepseek_script.js` DeepThink 思考超过 1.2s 时 `tickDom()` 越权抢跑，以及 `readFiberMarkdown` 向上 20 层越界爬到父组件 `content` 的问题。
  - **P1-1**：修复 `dom_markdown.js` 列表项首块为代码块/数学块/表格/引用时围栏倒置（`` - ```rust ``）、列表内标题 `###` 破坏层级、以及列表内代码块正文行被错误加上 `contIndent` 缩进的问题。
  - **P1-2**：修复 `src/ui/markdown_render/inline.rs` 粗体/斜体内嵌套代码与链接不解析、双反引号不支持、`***` 尾随闭合错位问题。
  - **P1-3**：修复 `src/ui/markdown_render.rs` 有序/任务列表续段缩进硬编码 `+ 2` 及子列表结束后外层列表栈被整栈清空的问题。
  - **P1-4**：修复 `deepseek_script.js` `finishSse()` 未补刷末尾 `[` 及 `deepseek_helpers.js` 历史清洗破坏代码块内 `[citation:...]` 与前导换行的问题。
  - **P2-1 ~ P2-3 & P3-1**：修复 `dom_markdown.js` `hasMeaningfulContent` 图片/空公式判定与 `markdown` 围栏中途跳变、Gemini 子选择器顺序、DeepSeek SSE 断流无 `finish` 兜底、Grok `stripGrokDirectives` 流中 `.trim()` 吞行首缩进、以及三站点 `pushDom`/`pushText` 锚点窗口（`1024`）与 LCP 窗口（`160`）扩充；新增 `src/chat/web/tests.rs` 与 `src/ui/markdown_render/tests.rs` 全套回归测试。
