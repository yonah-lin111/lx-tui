# DeepSeek Web（Web）驱动流程与适配规范

- **基准验证时间**：`2026-11-10`（北京时间 `2026-11-10 CST`）
- **目标站点**：`https://chat.deepseek.com`
  - 登录页：`https://chat.deepseek.com/sign_in`
  - 会话页：`https://chat.deepseek.com/a/chat/s/<session_id>`
- **驱动运行时**：`ego-browser nodejs`（隔离任务空间 `SPACE = 'lx-tui-ai-web-deepseek'`）
- **会话 ID 前缀**：`ai-web-ds-<id>`
- **Gemini 驱动**：见 [`docs/web/web-gemini.md`](web-gemini.md)（设计归档见 [`docs/feature/lx-ai-web-gemini/design.md`](../feature/lx-ai-web-gemini/design.md)）
- **Grok 驱动**：见 [`docs/web/web-grok.md`](web-grok.md)（设计归档见 [`docs/feature/lx-ai-web-grok/design.md`](../feature/lx-ai-web-grok/design.md)）

本文档完整记录 `lx-tui` 第 4 入口 `Web`（`LxEntry::AiWeb`）对接 DeepSeek 网页端的端到端流程、脚本通信协议、DOM 选择器、React 事件触发方式、SSE JSON Patch 流解析规则、DOM → Markdown 兜底序列化、CDP 附件上传与云端 REST API 契约。当 DeepSeek 网页端后续改版导致选择器或接口漂移，或需要将同一套 `Web` 架构复刻到其它网页端 AI 站点时，以此文档为基准排查与修改。

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
                            递归还原标题、粗/斜/删除线、行内代码、代码块（动态反引号围栏 + 语言识别）、
                            GFM 表格（<br> 与 ｜ 转义）、有序/无序/任务列表、引用块、链接、图片与 KaTeX/MathML 公式，
                            支持 streaming 尾部未完成语法 holdback
      deepseek_helpers.js   嵌入式辅助脚本：输出推送（emit/emitMany）、新对话重置（ensureFreshChat）、
                            CDP 附件上传与芯片清理（syncContainerAttachments）、
                            非对话动作（new / stop / list_sessions / load_session / rename_session / delete_session）
      deepseek_script.js    嵌入式主脚本：空间初始化、登录态检测、DeepThink 开关同步、
                            XHR/fetch SSE 拦截器（Channel A）+ DOM/React Fiber 兜底轮询（Channel B）、
                            原地编辑（/rewind）与普通发送提交
      gemini_helpers.js     Gemini 网页端辅助脚本：模型/思考档位切换、附件同步、会话动作
      gemini_images.js      Gemini 网页端图片辅助脚本：预览落盘、原图 URL 推导、FIFE gg/ 资源匹配与 download_image 动作
      gemini_script.js      Gemini 网页端主脚本：DOM → Markdown 流采集、普通发送与 /rewind 原地编辑
      grok_helpers.js       Grok 网页端辅助脚本：登录判定、Fast/Build 切换、附件同步、REST 历史、引用链接映射与生图落盘
      grok_script.js        Grok 网页端主脚本：CDP WebSocket 帧拦截 + DOM → Markdown 兜底流采集、普通发送与 /rewind 重试追加
    prepare/
      web.rs                发送前预处理（prepare_ai_web_message）：提取本地文件/图片绝对路径（files）
                            与 /rewind 保留的云端附件名（keep_files），拦截目录/Skill/MCP 引用
  event/
    chat/
      web.rs                子进程编排：启动 ego-browser nodejs、绑定 127.0.0.1:0 临时 HTTP 流监听、
                            拼装 __LX_PARAMS 作用域脚本、取消/停止与超时保护
  app/
    update/
      chat/
        web.rs              纯状态机：内存会话（ai-web-* / ai-web-ds-* / ai-web-gm-* / ai-web-grok-*）、
                            发送、流状态更新、/rewind 撤回与云端附件标记回填、首轮云端 ID/标题绑定、/resume 历史还原
  ui/
    markdown_render/
      inline.rs             TUI 侧 CommonMark 行内 Markdown 渲染器：支持 *** / ** / __ / ~~ / * / _ 递归嵌套解析、
                            多反引号行内代码（`` `code` ``）、代码段跳过强调闭合与词内下划线保护
```

---

## 2. 进程间通信与脚本注入机制

### 2.1 参数注入（`__LX_PARAMS`）

`ego-browser nodejs` 在持久化的后台 daemon worker 中执行脚本。`src/chat/web/mod.rs` 的 `script_source()` 将每次执行的参数写入顶层常量 `const __LX_PARAMS = { ... };`，随后依次拼接 `dom_markdown.js`、`deepseek_helpers.js` 与 `deepseek_script.js`，经子进程 `stdin` 一次性写入并 `shutdown`：

```javascript
const __LX_PARAMS = {
  action: "chat",           // "chat" | "new" | "stop" | "list_sessions" | "load_session" | "rename_session" | "delete_session"
  model: "DeepSeek",        // 站点模型标识
  thinking: "deepthink",    // "deepthink" | "off"
  prompt: "...",            // 发送给网页端的纯文本提示词（已剥离本地 @file 与 @[refer-*] 标记）
  files: ["/abs/a.png"],    // 本轮需经 CDP 新上传的本地文件/图片绝对路径
  keepFiles: ["old.pdf"],   // /rewind 原地编辑时需保留的云端已上传文件名列表
  firstTurn: true,          // 是否为当前会话首轮普通发送（决定是否先重置回空白新对话）
  streamUrl: "http://127.0.0.1:<port>/stream/<token>", // 本地回环实时流接收地址
  sessionId: "<uuid>",      // 云端真实会话 UUID（已剥离本地 "ai-web-ds-" 前缀）
  title: "...",             // rename_session 时的目标标题
  sessionsRoot: "...",      // 本地会话归档根目录
  downloadUrl: "",          // download_image 原图地址（DeepSeek 暂不使用）
  fallbackUrl: "",          // download_image 备用预览地址
  downloadDir: "",          // download_image 目标目录
  downloadStem: "",         // download_image 目标文件名前缀
  editIndex: -1,            // /rewind 原地编辑的目标用户消息 0-based 索引；普通发送为 -1
};
// 随后依次拼接 dom_markdown.js、deepseek_helpers.js 与 deepseek_script.js
```

### 2.2 本地回环 HTTP 实时流通道（绕过 stdio 管道缓冲）

- **问题根因**：`ego-browser nodejs` 的 `process.stdout.write` 经过多层进程管道转发时存在块缓冲，若仅靠 `stdout` 读取会导致思考与回答增量在生成期间卡顿，直到流结束才一次性刷出。
- **方案**：`run_chat` 在拉起子进程前绑定 `TcpListener::bind("127.0.0.1:0")`，将 `http://127.0.0.1:<port>/stream/<token>` 注入 `STREAM_URL`。脚本内 `emitMany()` 优先通过 `fetch(STREAM_URL, { method: 'POST', body: payload })` 直接向 `lx-tui` 推送每一批协议行；仅当 `STREAM_URL` 为空时回退到 `process.stdout.write`。
- **生命周期与心跳**：脚本循环期间每 1s 调用 `checkStreamAlive()` 向 `STREAM_URL` 发送空 body POST 探测连接存活；当 TUI 取消流或 `ego-browser nodejs` 退出时，流立即中止并回收临时 HTTP 监听器。

### 2.3 行协议格式（`@@LX_AI_WEB@@`）

脚本输出的每一行业务事件以 `@@LX_AI_WEB@@` 开头，后跟单行 JSON（非此前缀的日志如 `[ego-browser:notice]` 会被忽略或仅记入 `tracing::debug!`）：

| `event` | 载荷示例 | 触发时机与用途 |
| :--- | :--- | :--- |
| `thinking` | `{"event":"thinking","delta":"..."}` | 深度思考增量（映射为 `PartKind::Reasoning`，TUI 默认折叠） |
| `text` | `{"event":"text","delta":"..."}` | 回答正文 Markdown 增量（映射为 `PartKind::Text`） |
| `session` | `{"event":"session","id":"<uuid>","title":"..."}` | 一轮回答完成后从 URL 与侧栏提取云端会话 ID 和自动标题 |
| `sessions` | `{"event":"sessions","items":[{"id":"...","title":"...","updated_at_ms":1762500000000}]}` | `/resume` 拉取云端会话列表 |
| `session_loaded` | `{"event":"session_loaded","id":"...","title":"...","messages":[...]}` | `/resume` 选中会话后拉取完整分支消息与附件名 |
| `login_required` | `{"event":"login_required","url":"https://chat.deepseek.com/sign_in"}` | 检测到未登录，已切到登录页并 `handOffTaskSpace()` 移交前台 |
| `done` | `{"event":"done"}` | 当前脚本动作正常结束 |
| `error` | `{"event":"error","message":"..."}` | 脚本捕获异常并以红色 Toast 提示用户 |

---

## 3. 登录态探测与 Bearer Token 提取

1. **空间绑定与导航**：
   - 调用 `await activateTaskSpace()`（回收或复用 `lx-tui-ai-web-deepseek` 任务空间）。
   - 若当前页面 URL 不以 `https://chat.deepseek.com` 开头，调用 `await openOrReuseTab(HOME, { wait: true, timeout: 25 })`。
2. **登录态检查**：
   ```javascript
   Boolean(localStorage.getItem('userToken')) && !location.pathname.startsWith('/sign_in')
   ```
   - 若返回 `false`：导航至 `https://chat.deepseek.com/sign_in`，调用 `await handOffTaskSpace()` 打开可见浏览器窗口供用户扫码/密码登录，回投 `login_required` 后立即退出。
3. **Bearer Token 提取（供会话管理 REST API 使用）**：
   - DeepSeek Web 将鉴权 Token 存储在 `localStorage['userToken']` 的 JSON 字符串中，格式为 `{"value":"<token>", ...}`：
     ```javascript
     const tokenObj = JSON.parse(localStorage.getItem('userToken') || '{}');
     const token = tokenObj.value || '';
     // 请求头携带：headers: { authorization: 'Bearer ' + token }
     ```

---

## 4. 页面 DOM 选择器与 React 交互速查表（`2026-11-10` 基准）

DeepSeek Web 基于 React 构建，部分组件（如开启新对话、编辑按钮、删除附件芯片、发送按钮）直接调用原生 `.click()` 可能被合成事件系统忽略，因此脚本中优先查找元素上的 `__reactProps$*` 属性并调用 `.onClick({ stopPropagation() {}, preventDefault() {} })`，不存在时再回退 `.click()`。

| 功能区域 | 当前 DOM 选择器 / 判定条件 | 交互方式与注意事项 |
| :--- | :--- | :--- |
| **空白新对话页判定** | `location.pathname === '/' && document.querySelectorAll('[data-virtual-list-item-key]').length === 0` | 首轮发送（`FIRST_TURN && EDIT_INDEX < 0`）或 `/new` 时若不满足则执行重置 |
| **开启新对话按钮** | `[tabindex="0"], button, a, [role="button"], div, span` 中 `innerText.trim()` 为 `'开启新对话'` / `'新对话'` / `'New chat'` | 优先取带 `__reactProps*.onClick` 的节点触发点击；若 250ms 后仍未回到空白 `/` 则兜底 `openOrReuseTab(HOME)` |
| **深度思考（DeepThink）开关** | `.ds-toggle-button` 中 `innerText.trim()` 为 `'深度思考'` 或 `'DeepThink'` | 检查 `className.includes('ds-toggle-button--selected')` 与 `THINKING !== 'off'` 是否一致，不一致时 `.click()` 切换并等待 300ms |
| **对话消息虚拟列表项** | `[data-virtual-list-item-key]` | 含 `.ds-markdown, .ds-think-content` 子节点的为**助手消息**；不含的为**用户消息** |
| **助手思考与正文容器（Channel B 兜底）** | 思考：`.ds-think-content .ds-markdown`（回退 `.ds-think-content`）<br>正文：`.ds-markdown.ds-assistant-message-main-content`（回退不属于 `.ds-think-content` 的 `.ds-markdown`） | 仅当 Channel A 未产生任何 `sseThinking` 或 `sseText` 时启用；通过 `window.__lxHtmlToMarkdown(el, { streaming })` 与 `readFiberMarkdown(el)` 提取完整 Markdown |
| **底部主输入框（Composer）** | `textarea[name="search"]`（回退 `textarea`） | `ensureDeepSeekComposer()` 最多轮询 20s 等待挂载；用 `HTMLTextAreaElement.prototype.value` 的原生 setter 赋值并派发 `input` 事件 |
| **主发送 / 停止按钮** | 发送可用态：`.ds-button--primary:not(.ds-button--disabled)`（且无 `.ds-icon-stop`）<br>停止按钮：`.ds-button--primary .ds-icon-stop` 所在按钮<br>空闲禁用态：`.ds-button--primary.ds-button--disabled` | 普通发送在 8s 窗口内优先触发可用发送按钮的 `__reactProps*.onClick`，或向 `textarea` 派发 `Enter` 键事件，直到输入框与芯片清空或出现 `.ds-icon-stop` / `stream.sseActive` |
| **用户消息原地编辑入口** | 目标用户消息节点内 `.ds-button, button, [role="button"]` 的 `btns[1] \|\| btns[btns.length - 1]` | `btns[0]` 为复制按钮，`btns[1]` 为编辑按钮；触发 `__reactProps*.onClick` 后等待 120ms 渲染编辑框 |
| **原地编辑输入框与确认发送** | 编辑框：`textarea[name="user query"]`<br>确认按钮：同容器内 `innerText.trim()` 为 `'发送'` 或 `'Send'` 的按钮 | 原生 setter 写入新 `PROMPT`，派发 `input` + `change` 事件，再触发 `'发送'`/`'Send'` 按钮的 `__reactProps*.onClick` |
| **附件芯片容器** | `.ds-animated-size-item` | 在 `[data-virtual-list-item-key]` 外部的属于底部 Composer；在内部的属于消息气泡或原地编辑框 |
| **附件芯片文件名提取** | 图片：`item.querySelector('img[alt]').getAttribute('alt')`<br>文件：`item.innerText.split('\n')[0].trim()` | 用于 `/rewind` 后重发时与 `KEEP_FILES` 比对保留或关闭 |
| **附件芯片删除按钮** | `item.querySelectorAll('[tabindex="0"]')` 中含 `svg[viewBox="0 0 14 14"]` 的元素 | 触发 `__reactProps*.onClick` 移除不需要的芯片，并轮询等待芯片数降至 `keptCount` |
| **附件上传状态判定** | 上传中：剔除已知文件名后匹配 `/上传中\|解析中\|uploading\|parsing/i`、含 `svg[viewBox="0 0 20 20"][role="img"]` 或 React Fiber `file.status !== 'SUCCESS'`<br>失败：匹配 `/上传失败\|解析失败\|不支持\|超出\|过大\|upload failed\|parse failed\|unsupported\|exceed/i` 或 Fiber `hasError` | 必须先从 `item.innerText` 中剔除已知文件名再做正则检测，避免文件名自身包含关键字导致误判 |
| **隐藏文件上传 `<input>`** | 普通发送：页面最后一个 `input[type="file"]`<br>原地编辑：`[data-virtual-list-item-key] input[type="file"]` 第一个节点 | 通过 CDP `DOM.setFileInputFiles` 直接注入本地绝对路径数组 |
| **侧栏会话标题提取** | `location.pathname.match(/\/s\/([a-zA-Z0-9-]+)/)` 提取 `id`，再查 `a[href*="<id>"]` 的 `innerText.trim()` | 一轮对话完成后回投 `session` 事件，将本地临时会话升级为 `ai-web-ds-<id>` 并更新右上角标题胶囊 |

---

## 5. 流式回答双通道拦截机制（SSE + DOM/Fiber Markdown Fallback）

### 5.1 为什么采用双通道与严格互斥

1. **Channel A（网络层 XHR / fetch 拦截，主通道）**：
   - DeepSeek Web 通过 `XMLHttpRequest`（或 `fetch`）发起流式请求，响应 `Content-Type: text/event-stream`。
   - 直接解析 SSE 原始 JSON Patch 能实时获取**未丢失任何语法结构的 Markdown 源码**（代码块围栏、表格、列表缩进、行内强调）与精确的 `THINK` / `RESPONSE` 片段边界。
2. **Channel B（DOM `window.__lxHtmlToMarkdown` + React Fiber 兜底通道）**：
   - 当网络层 SSE 未能挂载（如响应流不可读）且超过 `1200ms` 宽限期（`graceOver`）或生成已结束时，Channel B 每 60ms 扫描新增的 `[data-virtual-list-item-key]` 节点并提取思考与正文 Markdown。
   - **严格互斥规则**：
     - 一旦 Channel A 已经推送过任何思考或正文（`this.sseThinking || this.sseText`），`tickDom()` **立即完全禁用 Channel B 的 DOM 采样**，仅在检测到发送按钮恢复空闲且无停止图标连续 15 个 tick（约 900ms）时调用 `this.finishSse()` 兜底收尾；
     - 反之，若 Channel B 已率先开始推送（`this.domTextStarted` 或 `this.domThinkingStarted`），Channel A 的 `push()` 会拒绝同类型的延迟帧，防止双通道混写导致文本重复。

### 5.2 拦截的流式 API 路径与 `attachXhr` 防误锁机制

包装 `XMLHttpRequest.prototype.open/send` 与 `window.fetch`（通过 `XMLHttpRequest.prototype.open.__lxWrapped` 与 `window.fetch.__lxWrapped` 防止重复包装，每次对话挂载最新的 `window.__lxDeepSeekStream` 实例），匹配以下流式端点：

- `/api/v0/chat/completion`（普通发送对话）
- `/api/v0/chat/edit_message`（`/rewind` 后原地编辑用户消息并重发）
- `/api/v0/chat/regenerate`（重新生成回答）
- `/api/v0/chat/continue` / `/api/v0/chat/resume_stream`（续写或恢复流）

**`attachXhr` 忽略非 SSE JSON 质询响应**：
- DeepSeek 发送前或触发风控质询时，同一路径可能先返回一个 `application/json` 的 PoW challenge 响应，随后再重放真正的 `text/event-stream` 请求。
- `attachXhr` 在 `pull()` 中先检查 `Content-Type` 与前 256 字节是否匹配 `/(?:^|\n)(?:event|data|:)/`（`looksSse`）：**非 SSE 的 JSON/质询响应直接忽略，不绑定 `this.activeXhr`**，从而保证紧随其后的真实 SSE 重放请求能够正常接管流。

### 5.3 DeepSeek SSE JSON Patch 协议与代码块感知 `cleanText`

- **帧结构**：
  - `event: delta`（或省略 `event:` 行）+ `data: <JSON>`：增量补丁帧。
  - `event: finish` / `event: close` 或 `data: [DONE]`：流结束标志。
- **Patch 对象字段**：
  - `p`（path）：目标字段路径（如 `"response/fragments/-1/content"`，前导 `/` 自动归一化并补齐 `response/` 前缀）。若连续多帧修改同一路径，后续帧会省略 `p` 和 `o`，解析器通过 `deltaState = { op: 'SET', path: '' }` 继承上一帧状态。
  - `o`（operation）：`'SET'` | `'APPEND'` | `'BATCH'`。当 `o === 'BATCH'` 时，`v` 为子 Patch 数组，使用独立的 `childState` 递归展开并拼接父路径。
  - `v`（value）：补丁值。
- **核心路径处理规则**：
  1. `p === 'response'`（或顶层对象含 `v.response`）：初始化或更新 `fragments` 数组；仅当不存在 `fragments` 时才回退处理标量 `v.thinking_content` / `v.content`（防止 `fragments` 与标量字段重复推送）；若 `v.status === 'FINISHED'`，调用 `this.finishSse()`。
  2. `p === 'response/status'` 且 `v === 'FINISHED'`：调用 `this.finishSse()`。
  3. `p === 'response/fragments'`：`APPEND` 或 `SET` 片段数组（`THINK` / `THINKING` / `REASONING` 映射为 `'thinking'`，`RESPONSE` / `TEXT` / `TEMPLATE_RESPONSE` 映射为 `'text'`，`SEARCH` 等其它片段忽略）。
  4. `p === 'response/fragments/<index>/<field>'`：支持负数索引（`-1` 代表末尾片段）；更新 `content` 或 `thinking_content` 并计算相对 `fragment.emitted` 的增量；多个非空 `THINK` 片段之间自动前置 `\n\n` 分隔。
- **代码块感知的引用角标清洗（`cleanText(text, holdIncomplete)`）与 `finishSse` 尾帧冲刷**：
  - `cleanText` 使用正则 `/\x60{3}[\s\S]*?(?:\x60{3}|$)/g` 切分三反引号围栏代码块：**围栏代码块内部的内容原样保留**（绝不误删代码中的 `[citation:...]` 或 `[reference:...]` 字面量），仅对围栏外的普通正文调用 `cleanPlain`。
  - `cleanPlain(text, holdIncomplete)` 剥离 `/\[(?:citation|reference):[^\]]*\]/g`；当流式进行中（`holdIncomplete = true`）且末尾存在未闭合的 `[`、其后缀与 `reference:` 或 `citation:` 呈前缀匹配时，暂时扣留 `[` 及其后字符不吐出，待后续帧补全 `]` 后整体剥离。
  - **`finishSse()` 尾帧冲刷**：当收到 `FINISHED`、`event: finish/close`、`[DONE]` 或连接正常结束时，`finishSse()` 遍历所有 `'text'` 片段，以 `cleanText(raw, false)`（关闭 `holdIncomplete`）重新计算全文，将流末尾因未闭合 `[` 扣留的普通文本（如 `[ref` 结尾的正文）一次性冲刷出去，随后标记 `this.done = true`。

### 5.4 Channel B（DOM → Markdown 与 React Fiber 兜底）细则

当网络层 SSE 未生效而回退到 Channel B 时：

1. **目标容器精确选择器**：
   - 思考容器：`target.querySelector('.ds-think-content .ds-markdown') || target.querySelector('.ds-think-content')`
   - 正文容器：`target.querySelector('.ds-markdown.ds-assistant-message-main-content')`，若无则回退查找第一个 `!el.closest('.ds-think-content')` 的 `.ds-markdown`。
2. **`readFiberMarkdown(el)` 与 DOM 宿主边界检查**：
   - 沿元素上的 `__reactFiber$*` / `__reactInternalInstance$*` 向上最多回溯 10 层 `fiber.return`；
   - **DOM 宿主边界守卫**：当 `depth > 0` 且 `fiber.stateNode instanceof Element`、同时 `fiber.stateNode !== el && !el.contains(fiber.stateNode)` 时**立即停止向上回溯**，防止正文的 `.ds-markdown` 向上爬到外层气泡容器时误读到思考块的 Fiber props（或反之）；
   - 若命中含 `citeProps` / `cacheKey` / `mergeConsecutiveCites` 的 Markdown 组件 props，直接读取其 `props.content` 或 `props.markdown` 原始 Markdown 字符串。
3. **`window.__lxHtmlToMarkdown(el, { streaming })` 与思考块切换冲刷**：
   - 流式进行中（`streamingFlag = true`）优先调用 `window.__lxHtmlToMarkdown(el, { streaming: true })`：自动跳过 `.ds-markdown-cite`、复制按钮与代码块横幅（`.md-code-block-banner-wrap`），提取 `.md-code-block-infostring` 语言名，并对位于 DOM 树最末尾（`isTail`）的未闭合代码块、表格首行、行内强调/代码/链接实施尾部 holdback。
   - **正文开始前冲刷思考块**：计算 `const thinkStreaming = isStreaming && !bodyText && !this.domTextStarted;`——一旦正文 `bodyText` 非空或已开始输出思考之后的文本，立即以 `streaming = false` 读取 `thinkEl`，确保思考块末尾被 holdback 的闭合标记在转入正文前完整刷出。
   - **`pushDom` 增量对齐**：支持前缀追加、尾部锚点重对齐（`[48, 32, 20, 12, 8, 5]` 窗口，抵消流式重渲染导致的局部偏移）与最长公共前缀（LCP）行内标记闭合对齐。

---

## 6. 文件与图片上传及 `/rewind` 附件同步流程

### 6.1 TUI 侧发送前预处理（`src/chat/prepare/web.rs`）

- **仅允许文件与图片**：支持输入框 `@path/to/file`、粘贴图片芯片 `@[paste-image #N](path)`、文件引用芯片 `@[refer-file](path)`、图片引用芯片 `@[refer-image](path)`。
- **严禁文件夹、Skill 与 MCP**：
  - `@` 提及面板在 `LxEntry::AiWeb` 下只渲染 `Files` 标签页（隐藏 `refDir` / `Skills` / `MCP`），在 `Files` 中对目录按 `Enter`/`Tab` 只进入子目录（`mention_enter_folder`），不插入 `@dir/` 引用；
  - `refer` 面板过滤掉全部目录项；`paste_open` 弹窗对目录禁用 `Reference`；
  - `prepare_ai_web_message` 遇到目录路径、`@[refer-folder]`、`@[skill:...]` 或 `@[mcp:...]` 直接返回明确报错。
- **本地新文件 vs 云端已存在附件分流**：
  - 当处于 `/rewind` 后的编辑态（`ai_web_edit_user_index.is_some()`）时，`pane.lx.ai_web_edit_cloud_files` 保存了被撤回轮次原有的云端附件文件名列表。
  - 若输入框中的 `@[refer-file](name)` / `@[refer-image](name)` 命中 `ai_web_edit_cloud_files`（以多重集计数匹配），归入 `keep_files`（告知网页端保留对应芯片，无需重新从本地磁盘读取）；
  - 其余引用解析为本地存在的绝对路径，归入 `files`（由 CDP 注入上传）。

### 6.2 网页端 CDP 上传与编辑框芯片同步（`syncContainerAttachments(inEditBox)`）

1. **阶段一：清理多余芯片**
   - 定位作用域：普通发送（`inEditBox = false`）扫描整个 `document` 中不属于 `[data-virtual-list-item-key]` 的 `.ds-animated-size-item`；原地编辑（`inEditBox = true`）扫描 `textarea[name="user query"]` 所在 `[data-virtual-list-item-key]` 内的 `.ds-animated-size-item`。
   - 遍历每个芯片提取文件名：若 `inEditBox` 为真且文件名在 `KEEP_FILES` 剩余列表中，则保留该芯片（`keptCount += 1`）；否则查找芯片内带 `svg[viewBox="0 0 14 14"]` 的 `[tabindex="0"]` 关闭按钮并触发点击。
   - 若关闭了任何芯片（`closedCount > 0`），在 3s 内轮询等待剩余芯片数降至 `keptCount`。
2. **阶段二：CDP 注入新文件并等待解析完成**
   - 若 `FILES.length > 0`，调用：
     ```javascript
     const doc = await cdp('DOM.getDocument', { depth: -1 });
     const selector = inEditBox ? '[data-virtual-list-item-key] input[type="file"]' : 'input[type="file"]';
     const queried = await cdp('DOM.querySelectorAll', { nodeId: doc.root.nodeId, selector });
     const targetNodeId = inEditBox ? queried.nodeIds[0] : queried.nodeIds[queried.nodeIds.length - 1];
     await cdp('DOM.setFileInputFiles', { nodeId: targetNodeId, files: FILES });
     ```
   - 在 30s 超时内每 150ms 轮询作用域内的 `.ds-animated-size-item`，同时结合 DOM 状态文本与 React Fiber 的 `file.status` / `snapshot.fileUploadInfo` 校验，直到芯片总数 `>= keptCount + FILES.length`、无错误状态且全部完成上传与解析后再填词发送。

---

## 7. 云端会话管理与 `/rewind` 同步规范

### 7.1 会话 ID 命名空间与状态隔离

- **TUI 内存会话 ID 规则**：
  - 未绑定云端的临时新会话：`ai-web-1`、`ai-web-2`……
  - 已绑定 DeepSeek 云端 UUID 的会话：`ai-web-ds-<remote_uuid>`（通过 `AI_WEB_DEEPSEEK_SESSION_PREFIX = "ai-web-ds-"` 转换）。
- **与本地入口及跨站点隔离**：`set_lx_entry` 与 `set_ai_web_site` 在切换入口或切换 `DeepSeek / Gemini / Grok` 站点时，自动停止在途流、清空跨侧会话状态、关闭残留面板并重置 `/rewind` 待编辑状态。

### 7.2 云端 REST API 契约（`2026-11-10` 基准）

所有接口均在 `ego-browser` 页面上下文中通过 `fetch` 同源调用，请求头携带 `authorization: 'Bearer ' + token`：

1. **拉取历史会话列表（`ACTION = 'list_sessions'`，对应 `/resume` 或 `/session`）**：
   - 请求：`GET /api/v0/chat_session/fetch_page?lte_cursor.pinned=false`
   - 响应解析：读取 `resp.data.biz_data.chat_sessions`，映射为：
     ```javascript
     {
       id: String(s.id || ''),
       title: String(s.title || 'New chat'),
       updated_at_ms: Math.round(Number(s.updated_at || 0) * 1000),
     }
     ```
2. **加载单条云端会话历史（`ACTION = 'load_session'`，对应 `/resume` 回车选中）**：
   - 先导航标签页至 `https://chat.deepseek.com/a/chat/s/<sessionId>`。
   - 并发请求：
     - `GET /api/v0/chat/history_messages?chat_session_id=<encodeURIComponent(sessionId)>`
     - `GET /api/v0/chat_session/fetch_page?lte_cursor.pinned=false`（用于查标题）
   - **激活分支链回溯**：
     - `historyResp.data.biz_data.chat_messages` 包含该会话下所有历史编辑分支的消息节点（每条含 `message_id` 与 `parent_id`）。
     - 必须以数组末尾最后一条消息为当前分支叶节点，沿 `parent_id -> byId.get(parent_id)` 逆向回溯至根节点后再 `.reverse()`，得到与网页端当前视图完全一致的 `activeChain`。
   - **消息、附件与围栏感知的 `cleanText` 解析**：
     - `deepseek_helpers.js` 中的 `cleanText(raw)` 同样采用**三反引号围栏感知（fence-aware）**拆分：先以 `/\x60{3}[\s\S]*?(?:\x60{3}|$)/g` 保护代码块原样输出，仅对代码块外部的文本调用 `stripCitePlain`（剥离 `/[ \t]*\[(?:citation|reference):[^\]]*\]/g`），防止历史回答代码块内的 `[citation:...]` 示例被误删。
     - `USER` 消息：收集 `msg.files`、`frag.files` 与 `frag.type === 'FILE'` 的 `file_name || name` 去重填入 `files[]`，拼接 `frag.type === 'REQUEST'` 的文本填入 `text`。TUI 收到后将 `files[]` 合成为 `@[refer-image](name)` / `@[refer-file](name)` 附加在用户消息末尾。
     - `ASSISTANT` 消息：拼接 `frag.type === 'THINK'` 的内容（经 fence-aware `cleanText`）填入 `thinking`，读取 `frag.elapsed_secs` 乘 `1000` 转为 `thinking_ms`（使历史思考块显示 `○ Thought · Xs` 并支持点击展开），拼接 `frag.type === 'RESPONSE'` 的内容（经 fence-aware `cleanText`）填入 `text`。
3. **重命名云端会话（`ACTION = 'rename_session'`，对应 `/rename` 或 `/title`）**：
   - 请求：`POST /api/v0/chat_session/update_title`
   - 请求体：`{"chat_session_id": "<sessionId>", "title": "<TITLE>"}`
4. **删除云端会话（`ACTION = 'delete_session'`，对应 `/resume` 列表按 `Ctrl+D`）**：
   - 请求：`POST /api/v0/chat_session/delete`
   - 请求体：`{"chat_session_ids": ["<sessionId>"]}`
   - 若删除前浏览器正停留在该会话 URL，删除后自动导航回 `https://chat.deepseek.com` 首页。

### 7.3 `/rewind`（`/undo` 及空闲双击 `Esc`）原地编辑闭环

1. 用户在 `Web`（DeepSeek）下执行 `/rewind`（或 `/undo`、空闲双击 `Esc`）时：
   - 定位最后一条非本地命令的用户消息 `last_user_idx`；
   - 提取该条消息中的云端附件文件名列表存入 `pane.lx.ai_web_edit_cloud_files`；
   - 截断 `messages.truncate(last_user_idx)`：若截断后 `messages.is_empty()`（即撤回全部轮次回到欢迎首页），同步清空 `session_title = None` 与 `session_title_palette = 0`，移除输入框右上角标题胶囊；
   - 计算剩余用户消息条数 `remaining_users`，设置 `pane.lx.ai_web_edit_user_index = Some(remaining_users)`；
   - 将原用户消息全文（含附件芯片标记）回填至底部输入框。
2. 用户修改文本或增删附件芯片后再次按 `Enter` 发送：
   - `try_send_ai_web` 消费 `ai_web_edit_user_index` 与 `ai_web_edit_cloud_files`，将其作为 `editIndex`、`keepFiles` 与新上传 `files` 传给 `deepseek_script.js`；
   - 网页端定位第 `EDIT_INDEX` 条用户消息，点击编辑按钮打开 `textarea[name="user query"]`，调用 `syncContainerAttachments(true)` 同步增删芯片，填入新文本并点击 `'发送'`/`'Send'`，触发 `/api/v0/chat/edit_message` SSE 流实时回传新回答。

---

## 8. 改版排障清单与新站点复刻指南

### 8.1 DeepSeek 网页端改版排障速查

当后续 DeepSeek Web 前端更新导致功能异常时，按以下顺序定位并更新 `deepseek_helpers.js` / `deepseek_script.js` / `dom_markdown.js`：

1. **报错 `DeepSeek composer not found` 或无法自动发送**：
   - 在 `ego-browser` 中检查主输入框 `<textarea>` 的 `name` 属性（当前为 `name="search"`）及发送按钮类名（当前为 `.ds-button--primary`）。
2. **回答不再流式输出，或缺失思考过程 / Markdown 格式丢失**：
   - 检查 Network 面板中流式请求路径是否仍为 `/api/v0/chat/completion`，以及前置质询请求是否干扰了 `attachXhr` 的 `looksSse` 判定；
   - 检查 SSE 帧中 `response/fragments` 的字段名是否变更；
   - 检查页面上的「深度思考」按钮类名是否仍为 `.ds-toggle-button` / `.ds-toggle-button--selected`；
   - 检查助手气泡 DOM 类名是否仍为 `.ds-think-content` 与 `.ds-markdown.ds-assistant-message-main-content`，以及代码块语言标签类名是否仍为 `.md-code-block-infostring`。
3. **`/rewind` 后重发报错 `DeepSeek /rewind target turn not found` 或 `Failed to edit message in DeepSeek Web`**：
   - 检查消息列表项属性是否仍为 `[data-virtual-list-item-key]`；
   - 检查用户消息下方操作栏中「编辑」按钮的位置（当前为 `btns[1]`）与点开后编辑框的选择器（当前为 `textarea[name="user query"]`）及发送按钮文本（`'发送'` / `'Send'`）。
4. **文件/图片上传超时或删不掉旧附件**：
   - 检查附件芯片容器类名（当前为 `.ds-animated-size-item`）、删除图标 SVG 的 `viewBox`（当前为 `0 0 14 14`）以及上传中 spinner 的 SVG 选择器（当前为 `svg[viewBox="0 0 20 20"][role="img"]`）。
5. **`/resume`、`/rename` 或 `Ctrl+D` 删除失败**：
   - 检查 `localStorage.getItem('userToken')` 的键名或 JSON 结构是否变化；
   - 检查 `/api/v0/chat_session/fetch_page`、`/api/v0/chat/history_messages`、`/api/v0/chat_session/update_title`、`/api/v0/chat_session/delete` 四个接口的 URL 或 `biz_data` 响应层级是否调整。

### 8.2 复刻到其它 Web 站点的标准步骤

若需新增或替换其它网页端 AI 站点（如 ChatGPT Web、Claude Web、Kimi Web 等）：

1. **保持 Rust 侧协议、DOM → Markdown 序列化器与状态机不变**：
   - `src/chat/web/mod.rs` 的 `@@LX_AI_WEB@@` NDJSON 协议、`src/chat/web/dom_markdown.js` 的 `window.__lxHtmlToMarkdown` 序列化器、`src/event/chat/web.rs` 的回环 HTTP 实时流监听器与 `__LX_PARAMS` 注入器、以及 `src/app/update/chat/web.rs` 的状态机均与具体站点解耦，可直接复用。
2. **新增站点枚举与驱动脚本**：
   - 在 `AiWebSite` 中添加对应站点变体与会话前缀；
   - 参照 `deepseek_helpers.js` / `deepseek_script.js`（或 [`web-gemini.md`](web-gemini.md)、[`web-grok.md`](web-grok.md)）实现新站点的动作集（`chat`、`new`、`stop`、`list_sessions`、`load_session`、`rename_session`、`delete_session`、可选 `download_image`），对齐相同的 `__LX_PARAMS` 输入与 `emit(...)` 输出事件即可。
