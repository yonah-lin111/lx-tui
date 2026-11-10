# Google Gemini Web（Web）驱动流程与适配规范

- **基准验证时间**：`2026-11-10`（北京时间 `2026-11-10 CST`）
- **目标站点**：`https://gemini.google.com/app`
  - 登录页：`https://accounts.google.com/ServiceLogin`
  - 会话页：`https://gemini.google.com/app/<session_id>`
- **驱动运行时**：`ego-browser nodejs`（隔离任务空间 `SPACE = 'lx-tui-ai-web-gemini'`）
- **会话 ID 前缀**：`ai-web-gm-<id>`
- **DeepSeek 驱动**：见 [`docs/web/web-deepseek.md`](web-deepseek.md)
- **Grok 驱动**：见 [`docs/web/web-grok.md`](web-grok.md)
- **特性设计归档**：见 [`docs/feature/lx-ai-web-gemini/design.md`](../feature/lx-ai-web-gemini/design.md)

本文档完整记录 `lx-tui` 第 4 入口 `Web`（`LxEntry::AiWeb`）对接 Google Gemini 网页端的端到端流程、脚本通信协议、登录与权限拦截、Angular Material DOM 选择器、模型与思考等级切换、DOM → Markdown 流式序列化、CDP 附件上传、生成图片落盘与原图下载、以及云端会话管理与 `/rewind` 原地编辑规范。当 Gemini 网页端后续改版导致选择器或行为漂移时，以此文档为基准排查与修改。

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
                            递归遍历 DOM 还原标题、粗/斜/删除线、行内代码、代码块（code-block / .code-block-decoration）、
                            表格（table-block / table）、有序/无序/任务列表、引用块、链接与 KaTeX/MathML 公式，
                            支持 streaming 尾部未完成语法 holdback，并规避 content-visibility: auto 导致的空 innerText
      gemini_helpers.js     Gemini 网页端辅助脚本：输出推送（emit/emitMany）、通知权限拦截（installPermissionGuard）、
                            新对话重置（ensureFreshChat）、模型与思考档位切换（ensureGeminiModelAndThinking）、
                            CDP 附件上传与预览芯片清理（syncContainerAttachments）、
                            非对话动作（new / stop / list_sessions / load_session / rename_session / delete_session / download_image）
      gemini_images.js      Gemini 网页端图片辅助脚本：图片格式嗅探（detectImageExt）、原图 URL 推导（toGeminiOriginalImageUrl）、
                            CDP 网络流读取（downloadImageBuffer）、生成图落盘归档（saveGeneratedImages）
                            与原图下载（runGeminiImageDownload，含 gemini-session:// 协议与 FIFE gg/ 资源表回溯）
      gemini_script.js      Gemini 网页端主脚本：空间初始化、登录态检测、模型/思考档位同步、
                            window.__lxGeminiStream 流采集器、/rewind 原地编辑（含旧回答重置检测）与普通发送提交
    prepare/
      web.rs                发送前预处理（prepare_ai_web_message）：提取本地文件/图片绝对路径（files）
                            与 /rewind 保留的云端附件名（keep_files），拦截目录/Skill/MCP 引用
  event/
    chat/
      web.rs                子进程编排：启动 ego-browser nodejs、绑定 127.0.0.1:0 临时 HTTP 流监听、
                            拼装 __LX_PARAMS 作用域脚本、取消/停止与 download_image 结果回投（download_result）
  app/
    state/lx/
      web.rs                站点枚举（AiWebSite::Gemini）、模型档位（GeminiWebModel）、思考等级（GeminiThinkingLevel）
                            与云端会话前缀（AI_WEB_GEMINI_SESSION_PREFIX = "ai-web-gm-"）
    update/
      chat/
        web.rs              纯状态机：内存会话（ai-web-* / ai-web-gm-*）、发送、流状态更新、生成图片卡片挂载、
                            /rewind 撤回与云端附件标记回填、首轮云端 ID/标题绑定、/resume 历史还原
  ui/
    markdown_render/
      inline.rs             TUI 侧 CommonMark 行内 Markdown 渲染器：支持 *** / ** / __ / ~~ / * / _ 递归嵌套解析、
                            多反引号行内代码（`` `code` ``）、代码段跳过强调闭合与词内下划线保护
```

---

## 2. 进程间通信与脚本注入机制

### 2.1 参数注入（`__LX_PARAMS`）

`src/chat/web/mod.rs` 的 `script_source(AiWebSite::Gemini, params)` 将每次执行的参数序列化为顶层常量 `const __LX_PARAMS = { ... };`，随后依次拼接 `dom_markdown.js`、`gemini_helpers.js`、`gemini_images.js` 与 `gemini_script.js`（四份脚本共享同一顶层作用域），经 `ego-browser nodejs` 的 `stdin` 一次性写入：

```javascript
const __LX_PARAMS = {
  action: "chat",           // "chat" | "new" | "stop" | "list_sessions" | "load_session" | "rename_session" | "delete_session" | "download_image"
  model: "3.8 Flash",       // "3.5 Flash-Lite" | "3.8 Flash" | "3.1 Pro" | "4 Argon"
  thinking: "low",          // "low" | "medium" | "high"
  prompt: "...",            // 发送给网页端的纯文本提示词（已剥离本地 @file 与 @[refer-*] 标记）
  files: ["/abs/a.png"],    // 本轮需经 CDP 新上传的本地文件/图片绝对路径
  keepFiles: ["old.pdf"],   // /rewind 原地编辑时需保留的云端已上传文件名列表
  firstTurn: true,          // 是否为当前会话首轮普通发送（决定是否先重置回空白新对话）
  streamUrl: "http://127.0.0.1:<port>/stream/<token>", // 本地回环实时流接收地址
  sessionId: "<id>",        // 云端真实会话 ID（已剥离本地 "ai-web-gm-" 前缀）
  title: "...",             // rename_session 时的目标标题
  sessionsRoot: "...",      // 本地会话归档根目录（用于保存 ~/.lx-t[-dev]/sessions/ai-web-gm-<id>/image/）
  downloadUrl: "...",       // download_image 时的原图 URL 或 gemini-session://<sid>/<turn>/<idx>
  fallbackUrl: "...",       // download_image 时的备用预览图 URL
  downloadDir: "...",       // download_image 时的目标保存目录（如 ~/Downloads）
  downloadStem: "...",      // download_image 时的目标文件名前缀（如 gemini-<shortSid>-r1-1-orig）
  editIndex: -1,            // /rewind 原地编辑的目标用户消息 0-based 索引；普通发送为 -1
};
```

### 2.2 本地回环 HTTP 实时流通道（绕过 stdio 管道缓冲）

- `src/event/chat/web.rs` 在拉起 `ego-browser nodejs` 前绑定 `127.0.0.1:0` 临时 HTTP 监听器，并生成一次性随机 token 写入 `STREAM_URL`。
- 脚本内 `emitMany()` 优先通过 `fetch(STREAM_URL, { method: 'POST', body: payload })` 直接向 `lx-tui` 推送协议行；空闲等待期间每 1s 调用 `checkStreamAlive()` 发送空 body POST 探测连接存活，一旦 TUI 端中止流即刻设置 `STREAM_ABORTED = true` 退出循环。

### 2.3 行协议格式（`@@LX_AI_WEB@@`）

脚本输出的每一行业务事件以 `@@LX_AI_WEB@@` 开头，后跟单行 JSON：

| `event` | 载荷示例 | 触发时机与用途 |
| :--- | :--- | :--- |
| `thinking` | `{"event":"thinking","delta":"..."}` | 思考过程 Markdown 增量（映射为 `PartKind::Reasoning`，TUI 默认折叠） |
| `text` | `{"event":"text","delta":"..."}` | 回答正文 Markdown 增量（映射为 `PartKind::Text`） |
| `image` | `{"event":"image","label":"gemini-r1-1.png","path":"/abs/...","original_url":"...","preview_url":"...","file_stem":"gemini-...-orig"}` | 对话生成图片落盘完成，或 `download_image` 动作完成原图下载（事件层在 `download_image` 下将其转换为 `download_result` 回投 UI Toast） |
| `session` | `{"event":"session","id":"<id>","title":"..."}` | 一轮回答完成后从 `/app/<id>` URL 与侧栏/标题提取云端会话 ID 和标题 |
| `sessions` | `{"event":"sessions","items":[{"id":"...","title":"...","updated_at_ms":1762500000000}]}` | `/resume` 从侧边栏提取云端会话列表 |
| `session_loaded` | `{"event":"session_loaded","id":"...","title":"...","messages":[{"role":"assistant","text":"...","images":[...]}]}` | `/resume` 选中会话后拉取完整历史消息、附件名与历史生成图片列表（`images`） |
| `login_required` | `{"event":"login_required","url":"https://gemini.google.com/app"}` | 检测到未登录，已调用 `handOffTaskSpace()` 移交前台浏览器窗口 |
| `done` | `{"event":"done"}` | 当前脚本动作正常结束 |
| `error` | `{"event":"error","message":"..."}` | 脚本捕获异常并以红色 Toast 提示用户 |

---

## 3. 登录态探测与浏览器权限弹窗守卫

1. **空间绑定与导航**：
   - 调用 `await activateTaskSpace()` 激活或接管 `SPACE = 'lx-tui-ai-web-gemini'`。
   - 若当前页面 URL 不以 `https://gemini.google.com` 开头，调用 `await openOrReuseTab(HOME, { wait: true, timeout: 25 })`。
2. **通知权限弹窗守卫（`installPermissionGuard()`）**：
   - Gemini 网页端在发送消息时会调用 `Notification.requestPermission()`，一旦弹出浏览器原生权限请求框会抢走 `ego-browser` 的自动化控制权。
   - 因此每次导航后立即通过 `Page.addScriptToEvaluateOnNewDocument` 与 `js(...)` 注入权限拦截器，将 `Notification.requestPermission` 固定返回 `Promise.resolve('denied')`，并将 `navigator.permissions.query({ name: 'notifications' })` 固定返回 `{ state: 'denied' }`。
3. **20s 冷启动轮询与登录态判定**：
   - Gemini 的 Angular 外壳在冷启动或慢网络下可能晚于标签页 `load` 事件完成挂载。脚本在 20s 窗口内每 200ms 检测一次：
     - 若 `location.hostname === 'accounts.google.com'`，立即判定为未登录（`signIn: true`）；
     - 检查是否存在登录入口链接：`a[href*="accounts.google.com/ServiceLogin"], a[aria-label="登录"]:not([href^="/app/"]), a[aria-label="Sign in"]:not([href^="/app/"])`；
     - 检查是否已挂载输入区：`.ql-editor[contenteditable="true"], rich-textarea, [data-test-id="bard-mode-menu-button"]`；
     - 当 `!signInLink && hasComposer` 时判定已登录（`ready: true`）；当 `signInLink && !hasComposer` 时提前结束等待。
   - 若未登录：调用 `await handOffTaskSpace()` 将浏览器窗口移交用户完成 Google 账号登录，回投 `login_required` 后退出。

---

## 4. 页面 DOM 选择器与 Angular Material 交互速查表（`2026-11-10` 基准）

**CSP `TrustedHTML` 铁律**：`gemini.google.com` 启用了严格的 Content Security Policy `require-trusted-types-for 'script'`。在页面上下文中直接对任何 DOM 节点赋值 `.innerHTML = ...` 或 `.outerHTML = ...` 均会抛出 `TypeError: This document requires 'TrustedHTML' assignment`。所有富文本输入必须通过 `focus()` + `document.execCommand('selectAll', false, null)` + `document.execCommand('insertText', false, text)` 完成，所有 DOM 读取必须只读遍历、严禁克隆后改写 `innerHTML`。

| 功能区域 | 当前 DOM 选择器 / 判定条件 | 交互方式与注意事项 |
| :--- | :--- | :--- |
| **空白新对话页判定** | `(location.pathname === '/app' \|\| location.pathname === '/app/') && document.querySelectorAll('user-query, model-response').length === 0 && Boolean(document.querySelector('.ql-editor[contenteditable="true"], rich-textarea'))` | 首轮发送（`FIRST_TURN && EDIT_INDEX < 0`）或 `/new` 时若不满足则执行重置 |
| **开启新对话按钮** | `a[aria-label*="发起新对话"], button[aria-label*="发起新对话"], a[aria-label*="New chat"], button[aria-label*="New chat"], [data-test-id="new-chat-button"], a[href="/app"]` | 优先点击新对话按钮并轮询 2.5s；若仍未回到空白页则执行 `location.assign(HOME)` 并重新安装权限守卫 |
| **模型与模式菜单按钮** | `[data-test-id="bard-mode-menu-button"], button.input-area-switch` | 15s 轮询等待挂载；当 `aria-expanded !== 'true'` 时点击展开 `.cdk-overlay-container` 菜单 |
| **一级模型档位切换（Fast / Thinking / Pro / Argon）** | `gem-menu-item[role="menuitemradio"], [role="menuitemradio"]`（排除 `.submenu-container` 内部项） | 匹配 `.label` 或 `innerText` 包含目标模型名（`3.5 Flash-Lite`、`3.8 Flash`、`3.1 Pro`、`4 Argon`）；若 `aria-checked !== 'true'` 则点击，随后重开菜单后置校验 `aria-checked === 'true'` |
| **二级思考等级子菜单（Low / Medium / High）** | 入口：`gem-menu-item[role="menuitem"], [role="menuitem"]` 中含 `'思考等级'` / `'thinking level'` / `'reasoning'`<br>选项：`.submenu-container gem-menu-item[role="menuitemradio"]` | 派发 `mouseenter` + `click()` 展开子菜单，匹配 `low`（`低`/`low`）、`medium`（`中`/`medium`）、`high`（`高`/`high`），选中后关闭菜单或点击 `.cdk-overlay-backdrop-showing` |
| **底部富文本输入框（Composer）** | `.ql-editor[contenteditable="true"], rich-textarea [contenteditable="true"]`（回退 `textarea`） | 调用 `editor.focus()`、`document.execCommand('selectAll')`、`document.execCommand('insertText', false, PROMPT)`（空串则 `'delete'`），并派发 `Event('input', { bubbles: true })` |
| **发送按钮与停止按钮** | 发送：`gem-icon-button.send-button button, button.send-button, button[aria-label*="发送"], button[aria-label*="Send"]`<br>停止：`button[aria-label*="停止"], button[aria-label*="Stop"], button.stop-button, .blue-circle.stop-icon` | 点击未禁用的发送按钮，并在 6s 内轮询确认输入框变空（`.ql-blank`）且 `uploader-file-preview` 清零，或停止按钮出现 |
| **用户消息与助手消息节点** | 用户消息：`user-query`（正文位于 `.query-text, .user-query-bubble-with-background`）<br>助手消息：`model-response` | 按轮次索引 `targetIndex` 定位目标 `model-response`；生成中状态由停止按钮存在或 `mr.querySelector('[aria-busy="true"]')` 判定 |
| **助手思考与正文容器（子容器优先）** | 思考：`model-thoughts .thoughts-content, thinking-overlay .thoughts-content`（回退 `model-thoughts, thinking-overlay`）<br>正文：`message-content .markdown`（回退 `message-content`） | **必须坚持子容器优先**：先查 `.thoughts-content` 与 `.markdown` 内层节点，避免外层容器包裹的展开按钮文案（如 `"显示思路"`）或 `.cdk-visually-hidden`（`"Gemini 说"`）混入 |
| **代码块与装饰头** | 容器：`code-block`（含内部 `pre > code[data-test-id="code-content"]` 或 `pre > code`）<br>语言栏：`.code-block-decoration > span, .code-block-decoration span` | `dom_markdown.js` 识别 `code-block` 为代码块容器，从 `.code-block-decoration span` 提取语言名（过滤复制/运行按钮文本），并跳过 `.code-block-decoration` 本身防止语言名重复出现在代码上方 |
| **表格与底栏** | 容器：`table-block` 包裹 `table`<br>底栏：`.table-footer` | `dom_markdown.js` 将 `table-block` 视为透明块容器，递归序列化内部 `table` 为 GFM 表格，并自动跳过 `.table-footer`（防止 `"导出到 Google 表格"` 混入正文） |
| **附件预览芯片（上传区与气泡内）** | 上传区：`uploader-file-preview, [data-test-id*="file-preview"]`<br>历史用户气泡内：`user-query-file-preview button[aria-label], .file-preview-container button[aria-label], .file-preview, img[alt]` | 上传区关闭按钮：`button[aria-label*="关闭/移除/删除/close/Remove/Delete"], .gem-attachment-close-button button`；上传中 spinner：`uploader mat-progress-spinner, uploader-file-preview [role="progressbar"]` |
| **生成图片容器** | `generated-image img, single-image img, .generated-images img` | `dom_markdown.js` 跳过这些节点（不内联成裸 Markdown 图片链接），由 `window.__lxGeminiStream` 单独提取 `currentSrc \|\| src` 交由 `gemini_images.js` 落盘并回投 `image` 事件 |

---

## 5. 流式 DOM → Markdown 采集与增量对齐机制

Gemini 的网络层 `StreamGenerate` 采用多层嵌套的 Google Batch RPC 数组编码，且流式回答期间前端通过 Angular 组件树增量更新 DOM。因此 `gemini_script.js` 使用 `window.__lxHtmlToMarkdown` 在 DOM 层直接还原结构化 Markdown。

### 5.1 `window.__lxHtmlToMarkdown(el, { streaming })` 尾部 Holdback 规则

在 `pollDom()` 中，每 60ms 对目标 `model-response` 调用 `extractCleanText`：

1. **噪音节点精准跳过（`shouldSkipElement`）**：
   - 自动忽略 `button`、`svg`、`mat-icon`、`gem-icon`、`gem-icon-button`、`source-footnote`、`sources-list`、`sources-carousel-inline`、`overview-carousel`、`message-actions`、`response-status`、`generated-image`、`single-image`、`.cdk-visually-hidden`、`.code-block-decoration`、`.table-footer`、`.generated-images`。
2. **流式尾部未完成语法 Holdback（`streaming: true` 且 `isTail(node)`）**：
   - **围栏代码块（`code-block` / `pre`）**：当代码块位于 DOM 树最末尾（其后无任何有意义兄弟节点）且 `streaming: true` 时，只输出开启围栏与已生成代码行（` ```lang\n<code> `），**暂不输出闭合围栏 `\n``` `**；若代码内容尚为空则整块暂扣，避免代码逐行流入时闭合围栏在每一帧被反复追加到中间。当代码块后方出现新段落或 `isGenerating` 变为 `false` 时，自动补齐 `\n``` `。
   - **GFM 表格（`table`）**：当表格位于流末尾且仍在输出第一行表头（`isTail(allRows[0])`）时暂不吐出；从第二行起，扣留正在流式增长的最后一行 `tr`（`if (tableIsTail && isTail(tr)) break;`），确保每一行表格只在整行单元格稳定后才输出。
   - **行内强调 / 行内代码 / 链接 / 数学公式**：位于流末尾的 `<strong>`、`<em>`、`<del>`、`<code>`、`<a href>` 与 KaTeX `<math>` 节点在 `streaming: true` 时只输出左定界符与内容（如 `**bold`、`` `code ``、`[label`、`$$ \n tex`），待节点脱离末尾或流结束时再一次性补齐右定界符。

### 5.2 思考块向正文切换时的立即冲刷（Flushing `thinkText`）

- 在 `pollDom()` 中先提取 `bodyText`（`streaming = isGenerating`），再计算思考块的流式标志：
  ```javascript
  const bodyText = this.extractCleanText(
    mr,
    'message-content .markdown, message-content',
    '.cdk-visually-hidden, source-footnote, sources-carousel-inline, sources-list, button, mat-icon',
    isGenerating
  );
  const thinkStreaming = isGenerating && !bodyText && !this.domText;
  const thinkText = this.extractCleanText(
    mr,
    'model-thoughts .thoughts-content, thinking-overlay .thoughts-content, model-thoughts, thinking-overlay',
    'button, mat-icon, .cdk-visually-hidden',
    thinkStreaming
  );
  if (thinkText) this.pushDom('thinking', thinkText);
  if (bodyText) this.pushDom('text', bodyText);
  ```
- **关键不变量**：一旦 `bodyText` 非空（即模型思考完毕、开始输出正文），`thinkStreaming` 立即变为 `false`，使 `thinkText` 以 `streaming: false` 做最后一次提取并调用 `pushDom('thinking', thinkText)`，从而在第一段 `text` 入队前将思考块末尾暂扣的闭合符号（如 `**` 或 `` ` ``）完整刷出；一旦 `this.domText` 非空，后续帧不再接受任何 `'thinking'` 增量。

### 5.3 `pushDom` 锚点对齐与完成判定

- **三级增量对齐**：
  1. **直接前缀匹配**：若 `current.startsWith(previous)`，直接取 `current.slice(previous.length)`。
  2. **尾部滑动锚点匹配（`[48, 32, 20, 12, 8, 5]`）**：当 Gemini 在段落中间动态插入引用角标或重排 DOM 导致前缀长度变化时，取 `previous` 末尾长度为 `len` 的子串在 `current` 尾部 1024 字符窗口内查找 `lastIndexOf(anchor)`，仅发射锚点之后的新增文本，绝不重发已流过的段落。
  3. **尾部收缩保护与 LCP 行内定界符对齐**：若 `current` 末尾 20 字符已包含在 `previous` 中（例如末尾段落因挂载引用组件短暂收缩），本帧静默跳过并保留 `previous`；否则计算最长公共前缀 `lcp`，跳过行内定界符差异（`/^[*_~\s\x60]+/`）发射真实新增后缀。
- **完成判定**：
  - 当 `!isGenerating`、无未加载完的生成图片（`!pendingImage`，单轮多图最多等待 250 ticks ≈ 15s）、且 `thinkText + bodyText + imageUrls` 连续 10 个 tick（约 600ms）保持稳定并已追平 `domText`（或稳定满 14 ticks）时，标记 `this.done = true`。

---

## 6. 文件/图片上传与生成图片落盘及原图下载

### 6.1 CDP 文件与图片上传（`syncContainerAttachments()`）

1. **清理残留芯片**：扫描底部输入区所有 `uploader-file-preview, [data-test-id*="file-preview"]`，点击其中的移除按钮清空旧附件。
2. **展开上传抽屉并等待 `<input type="file">` 挂载**：
   - Gemini 的 `<uploader>` 组件在空白新对话中是懒加载的。若 `document.querySelectorAll('input[type="file"]').length === 0`，点击 `button[aria-label*="上传和工具"], button[aria-label*="上传"], button[aria-label*="Upload"], simplified-input-menu button`（当 `aria-expanded === 'true'` 时绝不重复点击，防止把正在加载的抽屉关掉），最多轮询 8s。
3. **媒体与文档分流或合并注入**：
   - 若页面只有 1 个 `input[type="file"]`，一次性将 `FILES` 全部注入该节点（分两次调用 `DOM.setFileInputFiles` 会互相覆盖 `FileList`）；
   - 若页面存在多个 `input[type="file"]`，按扩展名将图片/视频注入 `input[type="file"][accept*="image"]`，其余文档注入 `images-files-uploader input[type="file"]`。
4. **等待上传完成**：在 30s 内每 200ms 轮询 `uploader-file-preview` 数量是否 `>= FILES.length` 且不存在 `mat-progress-spinner` / `[role="progressbar"]`，完成后收起上传抽屉。

### 6.2 生成图片持久化与原图下载（`src/chat/web/gemini_images.js`）

1. **实时对话与 `/resume` 生成图片落盘（`saveGeneratedImages`）**：
   - Gemini 生成的图片在 DOM 中表现为 `<generated-image><img>` 或 `<single-image><img>`，其 `src` 可能是 `https://lh3.googleusercontent.com/...` 或 `blob:https://gemini.google.com/...`。
   - 对 `blob:` 图片，脚本先在页面上下文中用 `<canvas>`（`naturalWidth × naturalHeight`）绘制并导出 `data:image/webp;base64,...`；在 `load_session` 下还会从 `performance.getEntriesByType('resource')` 中提取 `https://lh3.googleusercontent.com/gg/...` 真实 FIFE URL 按序配对。
   - 对 `https:` 图片，通过 CDP `Network.loadNetworkResource` + `IO.read`（自动携带浏览器会话 Cookie 并支持最多 5 跳文本 URL 重定向）下载二进制字节流。
   - 根据魔数（JPEG `FF D8` / PNG `89 50 4E 47` / WebP `RIFF...WEBP` / GIF `GIF`）嗅探真实扩展名，保存至 `<SESSIONS_ROOT>/ai-web-gm-<sessionId>/image/gemini-r<turn>-<idx>.<ext>`，并回投 `image` 事件。
2. **高清原图 URL 推导与 `download_image` 动作（`runGeminiImageDownload`）**：
   - **FIFE 原图后缀推导（`toGeminiOriginalImageUrl`）**：先剥离 `?alr=yes` 等查询串，再将末尾尺寸参数（如 `=s1024-rj`）替换为 `=s0-d-I?alr=yes` 请求无损原图。
   - **`gemini-session://<sid>/<turn>/<idx>` 延迟解析**：当首轮生成时 DOM 中仅有 `blob:` URL 时，`original_url` 记为 `gemini-session://<sid>/<turn>/<idx>`。用户在 TUI 中触发原图下载（`ACTION = 'download_image'`）时，脚本自动导航或刷新 `https://gemini.google.com/app/<sid>`，从 DOM `<img>` 或 `performance.getEntriesByType('resource')` 的 `gg/` 条目中解析出永久 FIFE URL，转成 `=s0-d-I?alr=yes` 下载到 `DOWNLOAD_DIR`（若仍为 `blob:` 则回退 `<canvas>` 导出无损 PNG），完成后回投 `image` 事件（由 Rust 事件层转为 `download_result` Toast）。

---

## 7. 云端会话管理与 `/rewind` 原地编辑规范

### 7.1 云端会话管理动作（`gemini_helpers.js`）

1. **拉取历史会话列表（`ACTION = 'list_sessions'`）**：
   - 调用 `ensureSidebarExpanded()`：若侧边栏未渲染 `a.gem-nav-list-item[href^="/app/"], a[href^="/app/"]`，自动点击左上角主菜单按钮（`[data-test-id="side-nav-menu-button"]`）展开侧栏。
   - 遍历所有 `/app/<id>` 链接提取 `{ id, title, updated_at_ms }` 回投 `sessions`。
2. **加载单条云端会话（`ACTION = 'load_session'`）**：
   - 导航至 `https://gemini.google.com/app/<SESSION_ID>`，等待 `user-query, model-response` 节点挂载及生成图片加载完毕，随后调用 `await installDomMarkdown()`。
   - **为什么必须用 `window.__lxHtmlToMarkdown(el, { streaming: false })` 而非 `innerText`**：
     - Gemini 对长对话的历史气泡启用了 CSS `content-visibility: auto`；对于视口之外未完成布局计算的历史消息，浏览器读取 `el.innerText` 会返回空字符串或丢失换行，且 `innerText` 会丢失所有代码块围栏、表格与加粗格式。
     - `window.__lxHtmlToMarkdown` 直接递归读取 DOM 节点的 `nodeValue` / `textContent` 与标签结构，不依赖浏览器布局引擎，即使元素处于屏外 `content-visibility: auto` 状态也能 100% 还原完整 Markdown。
   - 历史生成图片经 `saveGeneratedImages` 落盘（命中本地缓存文件时直接复用，不重复下载），随消息的 `images` 字段一并回投 `session_loaded`。
3. **重命名云端会话（`ACTION = 'rename_session'`）**：
   - 展开侧边栏，定位目标会话链接父节点内的 `button[aria-label*="更多选项"], button[aria-label*="More options"]` 并点击；
   - 在 `.cdk-overlay-container` 菜单中点击「重命名 / Rename」；
   - 在弹窗 `<input>` 中通过原生 `HTMLInputElement.prototype.value` setter 写入 `TITLE`，派发 `InputEvent('input')` + `Event('change')`，点击「重命名 / 保存 / Rename / Save」按钮。
4. **删除云端会话（`ACTION = 'delete_session'`）**：
   - 展开侧边栏并打开目标会话的「更多选项」菜单 → 点击「删除 / Delete」→ 在确认弹窗点击 `[data-test-id="confirm-button"]`（或文本为「删除 / Delete」的按钮）；若当前正停留在该会话 URL，删除后自动导航回 `HOME`。

### 7.2 `/rewind` 原地编辑与 `isGenerating` 重置检测

1. **附件不变性预检**：
   - Gemini 网页端的用户消息原地编辑框只允许修改文本，不支持增删该轮已上传的附件。
   - 当 `EDIT_INDEX >= 0` 时，脚本先比对目标 `<user-query>` 内的已有附件名列表与 `FILES.concat(KEEP_FILES)`：
     - 若附件集合完全一致：滚动到目标 `<user-query>`，点击 `button[aria-label="修改"], button[aria-label*="Edit"]`，在编辑框内通过 `execCommand('insertText')`（若新旧文本完全相同则追加零宽空格 `\u200b` 激活「更新 / Update」按钮）填入 `PROMPT` 并点击「更新 / Update」；
     - 若 `EDIT_INDEX === 0` 且附件集合发生变化：自动调用 `ensureFreshChat()` 开启新对话并通过普通发送流程重新上传附件；
     - 若 `EDIT_INDEX > 0` 且附件集合发生变化：显式返回错误 `Gemini /rewind cannot change attachments of an earlier turn; use /new or send without --file/--keep-file`，防止本地与云端历史分叉。
2. **旧回答残留与 `isGenerating` 重置检测（`editResetSeen`）**：
   - 点击「更新 / Update」后，目标位置 `responses[targetIndex]` 上的旧 `<model-response>` 节点不会立刻从 DOM 树移除。
   - 为防止采集器把旧回答误当成新生成的增量吐出，`window.__lxGeminiStream` 在初始化时记录旧回答快照 `initialThinkText` 与 `initialTargetText`，并将 `editResetSeen` 置为 `false`。
   - 在 `pollDom()` 中，只要 `!this.editResetSeen`：
     - 一旦检测到原节点被卸载（`!mr`），或 `isGenerating` 变为 `true`，或当前提取的 `curBody !== this.initialTargetText` / `curThink !== this.initialThinkText`，立即将 `this.editResetSeen = true` 并清空 `domThink = ''`、`domText = ''`、`started = false`，从下一帧起正式采集新回答。

---

## 8. 改版排障清单

当后续 Google Gemini Web 前端更新导致功能异常时，按以下顺序定位并更新 `gemini_helpers.js` / `gemini_images.js` / `gemini_script.js` / `dom_markdown.js`：

1. **报错 `Please sign in to Gemini` 或卡在 20s 登录检测**：
   - 检查未登录页的登录按钮选择器是否仍匹配 `a[href*="accounts.google.com/ServiceLogin"]`；
   - 检查已登录页的输入框选择器是否仍为 `.ql-editor[contenteditable="true"], rich-textarea`。
2. **报错 `Gemini model menu not found` 或 `thinking level menu not found`**：
   - 检查左下角或输入区模式切换按钮的 `data-test-id`（当前为 `bard-mode-menu-button` 或 `button.input-area-switch`）；
   - 检查弹出菜单中模型项的 `[role="menuitemradio"]` 结构及思考等级子菜单入口文案（当前匹配 `'思考等级'` / `'thinking level'` / `'reasoning'`）。
3. **抛出 `TrustedHTML` 相关异常或输入框无法填入文本**：
   - 检查是否有新增代码使用了 `.innerHTML`；确保所有富文本写入均走 `document.execCommand('insertText')`。
4. **流式回答混入按钮文案、代码语言重复或缺失代码块围栏**：
   - 检查 `<model-response>` 内思考容器（`model-thoughts .thoughts-content`）与正文容器（`message-content .markdown`）类名是否变化；
   - 检查代码块自定义元素是否仍为 `<code-block>`、语言装饰栏是否仍为 `.code-block-decoration`（需在 `dom_markdown.js` 的 `extractCodeLanguage` 提取并在 `shouldSkipElement` 跳过）。
5. **生成图片未显示或原图下载失败**：
   - 检查生成图片容器选择器是否仍为 `generated-image img, single-image img, .generated-images img`；
   - 检查 `performance.getEntriesByType('resource')` 中 FIFE 图片域名前缀是否仍为 `https://lh3.googleusercontent.com/gg/`。
