# xAI Grok Web（Web）驱动流程与适配规范

- **基准验证时间**：`2026-11-10`（北京时间 `2026-11-10 CST`）
- **目标站点**：`https://grok.com`
  - 首页：`https://grok.com/`
  - 会话页：`https://grok.com/c/<session_id>`
- **驱动运行时**：`ego-browser nodejs`（隔离任务空间 `SPACE = 'lx-tui-ai-web-grok'`）
- **会话 ID 前缀**：`ai-web-grok-<id>`（CLI `--site` 简写支持 `grok` / `gk`）
- **DeepSeek 驱动**：见 [`docs/web/web-deepseek.md`](web-deepseek.md)
- **Gemini 驱动**：见 [`docs/web/web-gemini.md`](web-gemini.md)
- **特性设计归档**：见 [`docs/feature/lx-ai-web-grok/design.md`](../feature/lx-ai-web-grok/design.md)

本文档完整记录 `lx-tui` 第 4 入口 `Web`（`LxEntry::AiWeb`）对接 xAI Grok 网页端的端到端流程、脚本通信协议、`/rest/auth/get-user` 登录态判定、ProseMirror 编辑器与模型切换交互、CDP WebSocket 原始 Markdown 帧拦截（Channel A）+ DOM `window.__lxHtmlToMarkdown` 兜底采集（Channel B）、CDP 附件上传、生成图片落盘、云端 REST 会话管理、`<grok:render>` 引用卡片映射与 `/rewind` 重试追加规范。当 Grok 网页端后续改版导致选择器、WebSocket 事件或 REST 接口漂移时，以此文档为基准排查与修改。

---

## 1. 模块职责与代码归属

```text
src/
  chat/
    web/
      mod.rs                行协议前缀（@@LX_AI_WEB@@）、流事件枚举（AiWebStreamEvent）、
                            NDJSON 解析（parse_script_line）、脚本拼接（script_source）
                            与 ego-browser 二进制路径探测
      dom_markdown.js       共享自包含 DOM → Markdown 序列化器（lxHtmlToMarkdown + installDomMarkdown）：
                            递归还原标题、粗/斜/删除线、行内代码、Grok 代码块（[data-testid="code-block"] /
                            .not-prose 区分代码块与生成图卡片 / span.font-mono 语言提取 / span.line 逐行拼接）、
                            GFM 表格、列表、引用块与链接，自动跳过 <grok:render> 与 <argument> 节点
      grok_helpers.js       Grok 网页端辅助脚本：输出推送（emit/emitMany）、OneTrust Cookie 弹窗关闭（dismissCookieBanner）、
                            通知权限守卫（installPermissionGuard）、REST 登录态探测（grokLoginState）、
                            模型切换（ensureGrokModel）、CDP 附件同步（syncComposerAttachments）、
                            <grok:render> 引用卡片转 Markdown 链接（replaceGrokCitations）、
                            生成图片 URL 解析与落盘（extractGrokGeneratedImageUrls / saveGeneratedImages）、
                            非对话动作（new / stop / list_sessions / load_session / rename_session / delete_session / download_image）
      grok_script.js        Grok 网页端主脚本：空间初始化、登录校验、会话历史稳定等待（waitGrokHistoryReady）、
                            Channel A（CDP Network.webSocketFrameReceived 实时拦截）+
                            Channel B（window.__lxGrokStream DOM → Markdown 兜底轮询）、
                            普通发送与 /rewind 重试追加
    prepare/
      web.rs                发送前预处理（prepare_ai_web_message）：提取本地文件/图片绝对路径（files），
                            拦截目录/Skill/MCP 引用
  event/
    chat/
      web.rs                子进程编排：启动 ego-browser nodejs、绑定 127.0.0.1:0 临时 HTTP 流监听、
                            拼装 __LX_PARAMS 作用域脚本、取消/停止与 download_image 结果回投
  app/
    state/lx/
      web.rs                站点枚举（AiWebSite::Grok）、模型档位（GrokWebModel: Fast / Build）
                            与云端会话前缀（AI_WEB_GROK_SESSION_PREFIX = "ai-web-grok-"）
    update/
      chat/
        web.rs              纯状态机：内存会话（ai-web-* / ai-web-grok-*）、发送、流状态更新、
                            Grok /rewind 重试追加（保留旧轮次并回填最后一条用户文本）、/resume 历史还原
  ui/
    markdown_render/
      inline.rs             TUI 侧 CommonMark 行内 Markdown 渲染器：支持 *** / ** / __ / ~~ / * / _ 递归嵌套解析、
                            多反引号行内代码（`` `code` ``）、代码段跳过强调闭合与词内下划线保护
```

---

## 2. 进程间通信与脚本注入机制

### 2.1 参数注入（`__LX_PARAMS`）

`src/chat/web/mod.rs` 的 `script_source(AiWebSite::Grok, params)` 将每次执行的参数写入顶层常量 `const __LX_PARAMS = { ... };`，随后依次拼接 `dom_markdown.js`、`grok_helpers.js` 与 `grok_script.js`，经 `ego-browser nodejs` 的 `stdin` 一次性写入：

```javascript
const __LX_PARAMS = {
  action: "chat",           // "chat" | "new" | "stop" | "list_sessions" | "load_session" | "rename_session" | "delete_session" | "download_image"
  model: "Fast",            // "Fast" | "Build"
  thinking: "",             // Grok 无独立思考档位，固定为空串
  prompt: "...",            // 发送给网页端的纯文本提示词（已剥离本地 @file 与 @[refer-*] 标记）
  files: ["/abs/a.png"],    // 本轮需经 CDP 新上传的本地文件/图片绝对路径
  keepFiles: [],            // Grok /rewind 采用重试追加，不支持保留历史附件
  firstTurn: true,          // 是否为当前会话首轮普通发送
  streamUrl: "http://127.0.0.1:<port>/stream/<token>", // 本地回环实时流接收地址
  sessionId: "<uuid>",      // 云端真实会话 UUID（已剥离本地 "ai-web-grok-" 前缀）
  title: "...",             // rename_session 时的目标标题
  sessionsRoot: "...",      // 本地会话归档根目录（用于保存 ~/.lx-t[-dev]/sessions/ai-web-grok-<id>/image/）
  downloadUrl: "...",       // download_image 原图地址（assets.grok.com/.../original_image.jpg）
  fallbackUrl: "...",       // download_image 备用预览地址（assets.grok.com/.../image.jpg）
  downloadDir: "...",       // download_image 目标保存目录
  downloadStem: "...",      // download_image 目标文件名前缀
  editIndex: -1,            // TUI/CLI 下 Grok /rewind 降级为普通追加（-1）；脚本仍保留兼容分支
};
```

### 2.2 本地回环 HTTP 实时流通道

- `run_chat` 在拉起子进程前绑定 `TcpListener::bind("127.0.0.1:0")`，将带一次性 token 的 `http://127.0.0.1:<port>/stream/<token>` 注入 `STREAM_URL`。
- `emitMany()` 优先通过 `fetch(STREAM_URL, { method: 'POST', body: payload })` 实时推送协议行；空闲期每 1s 调用 `checkStreamAlive()` 探测连接存活，失败时立即置 `STREAM_ABORTED = true` 终止轮询。

### 2.3 行协议格式（`@@LX_AI_WEB@@`）

| `event` | 载荷示例 | 触发时机与用途 |
| :--- | :--- | :--- |
| `text` | `{"event":"text","delta":"..."}` | 回答正文 Markdown 增量（Grok 不推送 `thinking` 事件） |
| `image` | `{"event":"image","label":"grok-r1-1.jpg","path":"/abs/...","original_url":"...","preview_url":"...","file_stem":"grok-...-orig"}` | 对话生成图片落盘完成，或 `download_image` 动作完成原图下载 |
| `session` | `{"event":"session","id":"<uuid>","title":"..."}` | 一轮回答完成后从 `/c/<uuid>` URL 与侧栏/标题提取云端会话 ID 和标题 |
| `sessions` | `{"event":"sessions","items":[{"id":"...","title":"...","updated_at_ms":1762500000000}]}` | `/resume` 通过 REST 接口拉取云端会话列表 |
| `session_loaded` | `{"event":"session_loaded","id":"...","title":"...","messages":[...]}` | `/resume` 通过 REST 接口拉取完整分支消息、附件名、转换后的引用链接与生成图片 |
| `login_required` | `{"event":"login_required","url":"https://grok.com/"}` | 检测到未登录（401 或匿名访客），已调用 `handOffTaskSpace()` 移交前台浏览器窗口 |
| `done` | `{"event":"done"}` | 当前脚本动作正常结束 |
| `error` | `{"event":"error","message":"..."}` | 脚本捕获异常并以红色 Toast 提示用户 |

---

## 3. 登录态探测、Cookie 横幅与反爬边界

1. **空间绑定、Cookie 横幅与权限守卫**：
   - 调用 `await activateTaskSpace()` 激活或接管 `SPACE = 'lx-tui-ai-web-grok'`，若当前不在 `https://grok.com` 则调用 `openOrReuseTab(HOME, { wait: true, timeout: 25 })`。
   - 调用 `await dismissCookieBanner()`：若页面存在可见的 OneTrust 拒绝按钮 `.ot-pc-refuse-all-handler`，直接点击关闭（不篡改 CSS `display: none`）。
   - 调用 `await installPermissionGuard()` 屏蔽浏览器 `Notification.requestPermission` 弹窗。
2. **基于 `/rest/auth/get-user` 的权威登录态探测（`grokLoginState()`）**：
   - **为什么不能只看 Cookie 或页面上的 `/sign-in` 链接**：Grok 会为未登录的匿名访客自动分配临时 `userId` Cookie，且已登录页面的部分菜单或页脚仍可能存在 `/sign-in` 链接；同时冷启动或 Cloudflare 质询期间 ProseMirror 尚未挂载。
   - 因此脚本在 20s 窗口内调用同源 `fetch('/rest/auth/get-user', { credentials: 'include' })`：
     - **已登录**：HTTP `200` 且返回 JSON 中 `Boolean(user && user.email)` 为 `true`；
     - **明确未登录**：HTTP `401`，或 HTTP `200` 但 `user.email` 为空（匿名访客 `anonymous: true`）→ 立即跳出轮询，调用 `emitLoginRequired()`（执行 `handOffTaskSpace()` 将可见浏览器移交用户登录或通过 Cloudflare 验证，并回投 `{ event: 'login_required', url: HOME }`）；
     - **网络抖动 / 5xx / Cloudflare 加载中**：在 20s 截止时间前每 300ms 重试；若超时仍失败则回投明确的 `Grok auth check failed (...)` 错误，不伪装成登录墙。
3. **非对话动作免等 Composer 快速分派**：
   - 登录校验通过后，若 `ACTION !== 'chat'`（如 `list_sessions`、`load_session`、`rename_session`、`delete_session`），立即进入 `runNonChatAction` 走同源 REST 接口，无需等待 ProseMirror 编辑器挂载。

---

## 4. 页面 DOM 选择器与 ProseMirror / React 交互速查表（`2026-11-10` 基准）

Grok 输入框基于 tiptap（ProseMirror），对话气泡与代码块基于 React + streamdown。所有发送动作必须通过真实 UI 输入与按钮点击触发（由页面自身生成 WebSocket 帧内的 `castle_request_token` 反爬令牌，严禁自行伪造或重放 WebSocket 请求）。

| 功能区域 | 当前 DOM 选择器 / 判定条件 | 交互方式与注意事项 |
| :--- | :--- | :--- |
| **空白新对话页判定** | `location.pathname === '/' && !document.querySelector('[data-testid="user-message"]')` | 首轮发送（`FIRST_TURN && EDIT_INDEX < 0`）或 `/new` 时若不满足则导航回 `HOME` |
| **底部富文本编辑器（ProseMirror）** | `[contenteditable="true"].ProseMirror, [contenteditable="true"]` | `ensureGrokComposer()` 最多轮询 20s；写入时调用 `editor.focus()`、`document.execCommand('selectAll')`、有文本用 `'insertText'`、纯附件空文本用 `'delete'` 清空旧草稿，并校验 `editor.innerText` |
| **模型档位切换器（Fast / Build / Auto / Expert / Heavy）** | 按钮：`button[aria-label^="型号"], button[aria-label^="Model"]`（优先取 `b.closest('form')`）<br>菜单项：`[role="menuitemradio"]` | `ensureGrokModel()` 先读按钮首词（兼容 `"Build\nBeta"`），已是目标模型则直接返回；否则在 15s 内点击展开菜单、匹配 `Fast` 或 `Build`（`Auto` / `Expert` / `Heavy` 需 SuperGrok 订阅），点击后校验按钮文本并收起残留菜单 |
| **发送按钮与提交确认** | `button[data-testid="chat-submit"]` | `submitGrokPrompt` 点击未禁用的提交按钮后，在 5s 内轮询确认 `[data-testid="user-message"]` / `[data-testid="assistant-message"]` 数量增加或停止按钮激活 |
| **停止生成按钮** | `button[aria-label="停止模型响应"], button[aria-label*="Stop model"]`（过滤 `disabled` 与 `aria-disabled="true"`） | 用于 `ACTION = 'stop'` 以及流式轮询中的 `generating` 状态判定（**严禁**用 `chat-submit` 按钮消失判定生成状态，因为空输入框下提交按钮也会隐藏） |
| **用户消息与助手消息容器** | 用户：`[data-testid="user-message"]`<br>助手：`[data-testid="assistant-message"]` | 会话页进入后先调 `waitGrokHistoryReady()` 等待历史消息数量稳定，再记录 `baselineAssistants = existingResponses.length`，只采集 `messages.length > baselineAssistants` 的新增助手气泡 |
| **助手正文 Markdown 容器** | `.response-content-markdown` | **只读该子容器**，严禁回退读取整个 `[data-testid="assistant-message"]` 的 `textContent`（助手气泡头部含有一个 `.sr-only` 的 `"Grok"` 标题，回退读取会在正文挂载前把 `"Grok"` 误当成回答输出） |
| **代码块 vs `.not-prose` 生成图卡片区分** | 代码块：`[data-testid="code-block"]` 或含 `pre, code, .chat-code-block` 的 `.not-prose`<br>语言标签：`span.font-mono`（排除 `button, pre, code` 内）<br>代码行：`:scope > span.line` | Grok 将代码块与生成图卡片都包裹在 `.not-prose` 容器中：`dom_markdown.js` 遇到 `.not-prose` 时，**仅保留内部含 `pre, code, [data-testid="code-block"], .chat-code-block` 的代码块容器**，自动跳过无代码节点的图片/工具卡片；若 `<code>` 由逐行 `span.line` 组成且无直接文本换行，自动以 `\n` 拼接各行 |
| **附件芯片列表与移除按钮** | 列表：`form [role="list"][aria-label*="对话附件"], form [role="list"][aria-label*="attachments"], form [role="list"][aria-label*="Attachments"]`<br>单项：`[role="listitem"]`<br>移除按钮：`button[aria-label="移除此附件"], button[aria-label*="Remove attachment"], button[aria-label*="Remove"]` | **必须限定在 `form` 内清理**，绝不触碰历史消息气泡内的附件卡片 |
| **隐藏文件上传 `<input>`** | `[data-testid="drop-ui"] input[type="file"]` | 最多轮询 10s 等待挂载，通过 CDP `DOM.setFileInputFiles` 注入本地绝对路径数组 |
| **生成图片节点** | `img[src*="/generated/"], .group\/grok-image img, [data-testid] .group\/image img` | 提取 `https:` 预览图 URL 交由 `saveGeneratedImages` 落盘并推导 `original_image` 地址 |

---

## 5. 流式回答双通道拦截机制（CDP WebSocket 帧 + DOM Markdown Fallback）

### 5.1 为什么采用 CDP WebSocket + DOM 双通道

1. **Channel A（CDP `Network.webSocketFrameReceived` 原始流，主通道）**：
   - Grok 发送消息后通过 WebSocket 推送 JSON 事件帧（`response.created`、`response.chunk`、`response.done`）。
   - 脚本在发送前调用 `await cdp('Network.enable')`，并在轮询循环中通过 `drainEvents()` 读取 `Network.webSocketFrameReceived` 事件，直接获取未经过 DOM 渲染损耗的原始 Markdown 增量、实时联网引用链接与工具步骤切换。
2. **Channel B（DOM `window.__lxHtmlToMarkdown` 轮询，兜底通道）**：
   - 若当前环境不支持 `drainEvents()` 或 WebSocket 帧未被捕获，在发送后超过 `1200ms` 宽限期（`graceOver`）且 `!this.wsSeen`（或生成已停止且 `!this.wsActive`）时，由 `window.__lxGrokStream` 每 60ms 对新增 `[data-testid="assistant-message"]` 下的 `.response-content-markdown` 调用 `window.__lxHtmlToMarkdown(body, { streaming: Boolean(generating) })` 提取 Markdown。
   - **严格互斥**：一旦 `wsSeen` 为真且仍在生成中，Channel B 绝不抢跑推送；一旦 `wsActive` 为真（Channel A 已推送过文本），Channel B 完全停止 `pushText`；反之若 `domStarted` 已率先置位，Channel A 不再混写。

### 5.2 Channel A：WebSocket 事件解析、工具卡片分段与跨帧 `<grok:render>` 扣留

在 `grok_script.js` 主循环中，对每一帧包含 `"response.chunk"` / `"response.done"` / `"response.created"` 的 WebSocket payload：

1. **事件与步骤追踪（`step_id` & `tool_usage_card`）**：
   - 当收到 `evObj.type === 'response.created'` 或 `'response.chunk'` 时，标记 `wsSeen = true`（通知页面内 `window.__lxGrokStream.wsSeen = true`，阻止 DOM 通道在慢首字时抢跑）。
   - 当收到 `evObj.type === 'response.done'` 时，标记 `wsDoneSeen = true`。
   - 读取 `chunk.metadata.step_id`：
     - 若 `chunk.tool_usage_card` 存在（模型调用了联网搜索或代码执行工具）：更新 `wsLastToolStep = Math.max(wsLastToolStep, stepId)`，若 `wsRawBuffer` 非空则置 `wsAfterToolCard = true` 并跳过该卡片帧；
     - 忽略任何 `stepId < wsLastToolStep` 的旧步骤迟到帧；
     - 当工具调用结束、下一段 `CHANNEL_ASSISTANT_RESPONSE` 正文到达且 `wsAfterToolCard === true` 时，自动在 `wsRawBuffer` 末尾补齐 `\n\n` 分段空行，防止工具调用前后的两段话粘连在同一行。
2. **正文与实时引用拼接**：
   - 当 `chunk.text.channel === 'CHANNEL_ASSISTANT_RESPONSE'` 时，将 `chunk.text.text` 追加至 `wsRawBuffer`。
   - 当 `chunk.render_citation && chunk.render_citation.url` 存在时，调用 `citationSourceLabel(url)` 提取域名主干首字母大写标签（如 `Github`、`Wikipedia`），通过 `citationMarkdown(url, label)` 格式化为 `[Label](url)`（前补空格）追加至 `wsRawBuffer`。
3. **跨帧 `<grok:render>` 尾部扣留（`cleanGrokWsBuffer(raw, holdIncomplete)`）**：
   - 剥离已闭合的 `<grok:render ...>...</grok:render>`、`<grok:render .../>` 与 `<argument ...>...</argument>` 标签，并将 `\u2060`（Word Joiner）替换为空格。
   - 当 `!wsDoneSeen`（`holdIncomplete = true`）时：检查末尾最后一个 `<` 及其后缀 `tail`，若 `tail` 是 `<grok:`、`</grok:`、`<argument`、`</argument` 的前缀或以其开头（说明标签跨 WebSocket 帧被截断），先将 `<` 及其后字符扣留不吐出，待后续帧闭合后再处理，彻底杜绝 `<grok:render` 原始 XML 泄漏到 TUI。

### 5.3 Channel B：`window.__lxHtmlToMarkdown` 与流式感知 `stripGrokDirectives`

1. **`extractText(el, generating)`**：
   - 仅查询 `const body = el.querySelector('.response-content-markdown')`；若不存在则返回 `''`。
   - 调用 `window.__lxHtmlToMarkdown(body, { streaming: Boolean(generating) })` 序列化 DOM（自动实现尾部未闭合代码块、表格与行内强调的 holdback），随后经 `stripGrokDirectives(raw, isStreaming)` 清洗残留指令。
2. **流式感知 `stripGrokDirectives(text, streaming)`**：
   - 剥离任何残留的 `<grok:render>` / `<argument>` 标签与未闭合的 `<grok:render\b[^>]*$` 尾部前缀，将连续 3 个以上换行压缩为 `\n\n`；
   - **关键细节**：当 `streaming === true` 时**保留末尾换行与空格**（`return streaming ? cleaned : cleaned.trim()`），避免段落之间的换行在流式拼接时被 `.trim()` 吞掉导致段落粘连。
3. **完成与空回答超时判定**：
   - 生成结束后（`!generating`），由于 Grok 的 streamdown 组件与联网引用角标可能触发末次重排，要求 `sample` 连续稳定 `30` 个 tick（约 `1.8s`）才置 `this.done = true`；
   - 若新助手消息节点已出现、停止按钮已消失，但连续 `15s` 既无正文也无生成图片（例如 Build 画布独立弹窗、触发限流或错误横幅），显式报错 `'Grok response has no text'` 并结束，避免空等 300s。

---

## 6. 文件与图片上传及生成图片持久化

### 6.1 CDP 文件与图片上传（`syncComposerAttachments()`）

1. **循环清空输入框残留附件**：
   - 在 10s 截止时间内循环查询 `form [role="list"][aria-label*="对话附件"] [role="listitem"]`（及英文 `*attachments*` / `*Attachments*`）；
   - 每次点击单项内的 `button[aria-label="移除此附件"], button[aria-label*="Remove attachment"], button[aria-label*="Remove"]` 并等待 180ms，**必须删到剩余芯片数为 `0` 为止**（不設固定循环次数上限，防止超过 5 个残留附件时漏删）。
2. **注入本地文件并等待芯片就绪**：
   - 若 `FILES.length > 0`，在 10s 内轮询 CDP `DOM.querySelectorAll` 定位 `[data-testid="drop-ui"] input[type="file"]`，调用 `DOM.setFileInputFiles` 一次性注入 `FILES`；
   - 在 60s 内每 200ms 轮询输入框附件芯片数，直到 `chips === FILES.length` 才放行填词与发送。

### 6.2 生成图片提取、落盘与原图下载（`grok_helpers.js`）

1. **预览图与原图 URL 映射规则**：
   - Grok 生成图片存储在 `https://assets.grok.com/` 下（相对路径通过 `resolveGrokAssetUrl` 补齐域名前缀）。
   - 预览图 URL 形如 `.../generated/<uuid>/image.jpg`，对应的高清原图 URL 通过 `toGrokOriginalImageUrl` 将末尾 `/image.<ext>` 替换为 `/original_image.<ext>`（在 `load_session` 中优先取 `fileAttachmentAssetMetadata[].auxKeys['original-image']`）。
2. **CDP 下载与归档落盘（`saveGeneratedImages`）**：
   - 通过 CDP `Network.loadNetworkResource`（携带 `includeCredentials: true`）+ `IO.read` 读取图片字节流，嗅探魔数确定真实扩展名（`jpg` / `png` / `webp` / `gif`），写入 `<SESSIONS_ROOT>/ai-web-grok-<sessionId>/image/grok-r<turn>-<idx>.<ext>`。
   - 若本轮为纯生图回答（`!batch.hadText && saved.length > 0`），自动补充一条 `'Generated image'` / `'Generated N images'` 文本事件，随后逐张回投 `image` 事件。

---

## 7. 云端 REST 会话管理、引用卡片映射与 `/rewind` 重试追加

### 7.1 云端 REST API 契约（`2026-11-10` 基准）

Grok 的会话列表、加载、重命名与删除全部在页面上下文中通过同源 `fetch(..., { credentials: 'include' })` 调用 REST API，无需操作侧边栏 DOM 菜单：

1. **拉取历史会话列表（`ACTION = 'list_sessions'`）**：
   - 请求：`GET /rest/app-chat/conversations?pageSize=60&excludeProjects=true`
   - 响应解析：遍历 `data.conversations`，映射为 `{ id: c.conversationId, title: c.title || 'New chat', updated_at_ms: Date.parse(c.modifyTime || c.createTime) }`。若返回 `401` 则触发 `emitLoginRequired()`。
2. **加载单条云端会话历史（`ACTION = 'load_session'`）**：
   - 请求：`GET /rest/app-chat/conversations/<encodeURIComponent(SESSION_ID)>/responses`（同时请求一次 `/rest/app-chat/conversations?pageSize=60&excludeProjects=true` 补齐会话 `title`）。
   - **活跃分支链重建**：以 `data.responses` 末尾最后一个节点为叶节点，沿 `parentResponseId -> byId.get(parentResponseId)` 逆向回溯并 `.reverse()`，得到当前线性历史链 `chain`。
   - **原始 Markdown 与 `<grok:render>` 引用卡片还原（`replaceGrokCitations(message, cardAttachmentsJson)`）**：
     - `/responses` 接口返回的 `r.message` 是**完整的原始 Markdown 源码**，但其中的联网引用以 `<grok:render card_id="..." type="render_inline_citation"><argument name="citation_id">1</argument></grok:render>`（或自闭合 `<grok:render .../>`）形式内嵌。
     - `parseCitationCards(r.cardAttachmentsJson)` 将每张附件卡片 JSON 解析为 `card.id -> card.url` 映射表；
     - `replaceGrokCitations` 将成对及自闭合的 `<grok:render>` 按 `card_id` 替换为 `[SourceLabel](url)` Markdown 链接（若前一个字符非空白则自动前置空格），并彻底剥离任何残留的 `<argument>` 或未闭合 `<grok:*>` 标签与 `\u2060`。
   - **历史生成图片还原**：调用 `extractGrokGeneratedImageUrls(r)` 从 `cardAttachmentsJson`（`generated_image_card` / `render_generated_image`）、`generatedImageUrls` 与 `fileAttachmentsMetadata` 中提取预览图与原图 URL，经 `saveGeneratedImages` 落盘后填入每条消息的 `images` 数组。
3. **重命名云端会话（`ACTION = 'rename_session'`）**：
   - 请求：`PUT /rest/app-chat/conversations/<encodeURIComponent(SESSION_ID)>`
   - 请求体：`{"title": "<TITLE>"}`
4. **删除云端会话（`ACTION = 'delete_session'`）**：
   - 请求：`DELETE /rest/app-chat/conversations/<encodeURIComponent(SESSION_ID)>`
   - 若删除时浏览器正停留在 `/c/<SESSION_ID>`，自动调用 `openOrReuseTab(HOME)` 返回首页。

### 7.2 `/rewind` 重试追加语义（为什么不做原地替换）

- **网页端实测事实**：在 Grok 网页端点击历史用户消息的 `button[aria-label="Edit"]` 并提交后，Grok 后端不会原地替换该轮消息，而是清空旧用户消息正文并在会话末尾**线性追加一条新用户消息与新回答**。
- **TUI 侧一致化设计（`src/app/update/chat/web.rs` 的 `rewind_ai_web`）**：
  - 若最后一条用户消息包含附件：显式拒绝并弹出 Warning Toast `Grok /rewind cannot restore attachments; use /new or send without files`；
  - 若为纯文本用户消息：**不截断已有 `messages`**，将最后一条用户消息的文本回填到底部输入框，`ai_web_edit_user_index` 保持 `None`，并提示 `Grok /rewind appends a retry turn; use /new for a clean chat`；用户修改后按 `Enter` 直接作为新轮次追加发送，保证本地消息树与云端线性历史严格一致。

---

## 8. 改版排障清单

当后续 xAI Grok Web 前端或后端接口更新导致功能异常时，按以下顺序定位并更新 `grok_helpers.js` / `grok_script.js` / `dom_markdown.js`：

1. **误报未登录或 `Grok auth check failed`**：
   - 在浏览器中检查 `GET /rest/auth/get-user` 的响应结构是否仍包含顶层 `email` 字段，或是否被 Cloudflare Turnstile 质询拦截（若遇 Cloudflare 质询，需在移交的 `ego-browser` 窗口中完成一次验证）。
2. **报错 `Grok composer not found` 或 `Grok model "..." not found`**：
   - 检查底部输入框是否仍为 `[contenteditable="true"].ProseMirror`；
   - 检查模型按钮的 `aria-label` 前缀是否仍为 `"型号"` / `"Model"`，以及弹出菜单项是否仍为 `[role="menuitemradio"]`。
3. **发送后卡住或报 `Grok send button not available or message not submitted`**：
   - 检查发送按钮是否仍为 `button[data-testid="chat-submit"]`；
   - 检查用户与助手气泡是否仍使用 `[data-testid="user-message"]` 与 `[data-testid="assistant-message"]`，以及停止按钮 `aria-label` 是否仍匹配 `"停止模型响应"` / `"Stop model"`。
4. **流式输出丢失 Markdown 格式、代码行粘连或泄漏 `<grok:render>` 标签**：
   - 检查 CDP `Network.webSocketFrameReceived` 帧 JSON 中 `event.type === 'response.chunk'`、`chunk.text.channel === 'CHANNEL_ASSISTANT_RESPONSE'` 及 `chunk.render_citation.url` 字段名是否变更；
   - 检查 DOM 兜底路径中助手正文容器是否仍为 `.response-content-markdown`，代码块是否仍带 `[data-testid="code-block"]` 或 `.not-prose` + `<pre><code>`。
5. **附件上传超时（`Timed out waiting for Grok file upload`）**：
   - 检查隐藏文件输入是否仍位于 `[data-testid="drop-ui"] input[type="file"]`；
   - 检查输入框内附件列表的 `role="list"` 与 `aria-label`（当前匹配 `*对话附件*` / `*attachments*` / `*Attachments*`）及删除按钮 `aria-label` 是否变化。
6. **`/resume`、`/rename` 或 `Ctrl+D` 删除失败**：
   - 检查 `/rest/app-chat/conversations` 与 `/rest/app-chat/conversations/<id>/responses` 的路径、请求方法或 JSON 字段（`conversationId`、`responses`、`parentResponseId`、`cardAttachmentsJson`）是否调整。
