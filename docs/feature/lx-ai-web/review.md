# lx `AI Web` 特性代码审查报告（Review）

> 审查范围：`.worktrees/lx-ai-web`（分支 `feature/lx-ai-web`，提交 `77f73eb`..`40e5238`）、`.worktrees/ai-web-markdown`（Web Markdown 保真提取与 TUI 渲染增强专项）及特性文档 `docs/feature/lx-ai-web*/{design.md,task.md,review.md}`
> 审查基准：`AGENTS.md`、`docs/standards/project-directory-structure.md`、`docs/standards/code-writing-standards.md`、`docs/standards/tui-design-requirements.md`、[`docs/web/web-deepseek.md`](../../web/web-deepseek.md)、[`docs/web/web-gemini.md`](../../web/web-gemini.md)、[`docs/web/web-grok.md`](../../web/web-grok.md)
> 优先级定义：**P0**（阻断核心流程 / 违背明确需求边界 / 会话状态损坏 / 双通道死锁或思考块分裂）→ **P1**（高优功能缺陷 / Markdown 结构损坏与标记泄漏 / 云端同步与异常兜底缺失）→ **P2**（边界条件、选择器与流式状态机边角问题、代码规范与行数红线）→ **P3**（大块 DOM 差分窗口调优与辅助对齐）

---

## P0（阻断核心流程 / 违背需求边界 / 状态损坏）

### P0-1：`AI Web` 模式下未彻底禁用 `refDir` 及文件夹引用
- **涉及文件**：
  - `src/ui/main_content/lx/mention.rs`
  - `src/app/state/lx/mention.rs`
  - `src/app/state/lx/panels.rs`
  - `src/app/state/lx/refer.rs`
- **关键点**：
  1. `AI Web` 模式下不能发送文件夹、只能发送文件，且不能使用 `refDir` 相关功能。但此前 `@` 提及面板在 `AI Web` 下仍保留了 `refDir` Tab，用户选中引用目录后会插入 `@[refer-folder](path)`，发送时必然报错拦截；
  2. 在 `@` 面板 `Files` Tab 中高亮文件夹按 `Enter` / `Tab` 或鼠标点击确认时，`mention_confirm` 直接插入 `@dir/ ` 目录引用（而非进入目录挑选文件），发送时同样失败；
  3. 切换入口至 `AI Web` 时，`set_lx_entry` 调用了带 `LxMentionView::Hidden` 守卫的 `mention_set_tab`，当 `@` 面板处于关闭态时无法将旧的 `RefDir` / `Skills` / `MCP` Tab 归位到 `Files`；
  4. 字母快捷引用面板（`refer.rs`）与终端路径粘贴面板（`paste_open`）在 `AI Web` 下仍暴露文件夹引用选项。
- **错误原因**：
  - 未在 `@` 面板、目录确认、快捷引用与路径粘贴各入口统一收口“仅限文件、禁用 `refDir` 与文件夹引用”的约束；
  - 入口切换时使用了带可见性守卫的方法重置 Tab。
- **修复方案**：
  - `AI Web` 下 `@` 面板顶栏仅展示 `Files` 单一 Tab，`switch_tab_for_entry` 禁止切换，`set_tab_for_entry` 仅允许 `Files`，并新增无视隐藏态的 `force_tab(MentionTab::Files)` 在切换入口时强制归位；
  - `AI Web` 下在 `Files` Tab 确认目录条目时自动转为进入该目录（`mention_enter_folder`），不插入 `@dir/ `；
  - `AI Web` 下 `refresh_refer_panel` 过滤掉 `ReferTag::Folder` 候选，`paste_open` 在含目录候选时禁用 `Reference` 改写。

---

### P0-2：多文件上传过早判定完成 & 文件名关键字误触发上传失败/超时
- **涉及文件**：
  - `src/chat/ai_web/deepseek_script.js`（拆分后含 `src/chat/ai_web/deepseek_helpers.js`）
- **关键点**：
  1. `syncContainerAttachments` 轮询上传状态时仅判断 `status.count > 0 && !status.uploading` 即返回。当用户一次附加 2 个及以上文件时，第 1 个文件卡片挂载并完成上传而第 2 个尚未挂载时就会提前返回并触发发送；此外清理旧卡片仅固定等待 `220ms`，未确认旧卡片数量降为目标保留数；
  2. 错误检测直接对整张卡片的 `item.innerText`（包含文件名本身）执行正则 `/失败|不支持|超出|过大|error|failed|unsupported|exceed/i` 与 `/上传中|解析中|uploading|parsing/i`。当用户上传名为 `error.rs`、`failed_test.log`、`uploading.md` 等正常文件时，文件名会误触发上传失败抛错或一直被判定为“上传中”直至 30 秒超时。
- **错误原因**：
  - 上传就绪条件未按期望的卡片总数（`keptCount + FILES.length`）校验，清理旧卡片未轮询等待 DOM 移除完成；
  - 状态正则直接匹配包含文件名的整卡文本，未剔除已知文件名。
- **修复方案**：
  - 点击移除不保留的旧卡片后，轮询等待容器内剩余卡片数降至 `keptCount`；
  - `DOM.setFileInputFiles` 注入后，要求 `status.count >= keptCount + FILES.length` 且 `!status.uploading` 才判定完成；
  - 在检测上传态/错误态前先从卡片文本中剥离本次已知文件名（含 `KEEP_FILES` 与 `FILES` 的 basename），并收紧错误关键字匹配。

---

### P0-3：纯图片/纯文件无文字消息（`PROMPT === ""`）发送后误点停止按钮中断回答
- **涉及文件**：
  - `src/chat/ai_web/deepseek_script.js`
- **关键点**：
  - 当用户仅上传图片或文件而不输入文字（`PROMPT === ""`）时，`cleared` 判据 `textEmpty && composerChips.length === 0 && ${PROMPT ? 'true' : 'false'}` 恒为 `false`。若 `Enter` 键已成功提交该纯附件消息，90ms 后主按钮 `.ds-button--primary` 已变为“停止生成”按钮，兜底点击逻辑会立即点击该按钮中止流式回答。
- **错误原因**：
  - 误将 `PROMPT` 非空作为 `Enter` 提交成功的必要条件，未考虑纯附件消息提交后通过底部输入框 `.ds-animated-size-item` 卡片被清空即可判定提交成功。
- **修复方案**：
  - 将 `cleared` 判据改为：`PROMPT` 非空时要求 `textEmpty && composerChips.length === 0`；`PROMPT` 为空但有附件时要求 `composerChips.length === 0`。

---

### P0-4：`AI Web` 与 `Native` 切换未隔离会话状态，且 `!` 本地 Shell 模式泄漏进 `AI Web`
- **涉及文件**：
  - `src/app/update/pane.rs`
  - `src/app/update/chat/mod.rs`
  - `src/app/state/lx.rs`
- **关键点**：
  1. 在 `AI Web` 下产生 `ai-web-*` 会话后（如首轮 `/rewind` 回到空白欢迎页）点击切换回 `Native` 入口时，`chat.session_id` 仍保留 `ai-web-*`。此后在 `Native` 下发送消息会把 `ai-web-*` 传给 SQLite `BeginChatTurn`，导致数据库永久返回 `SessionMissing`（提示 `Send failed, retry`），且底栏状态行因会话前缀继续显示 `DeepSeek`；
  2. 在 `AI Web` 下输入 `!` 会进入本地 Shell 模式，并在 `try_send` 中先于 `LxEntry::AiWeb` 分支执行 `try_submit_user_shell`，触发 SQLite 写入错误或创建孤儿本地会话。
- **错误原因**：
  - `set_lx_entry` 未在 `AI Web` 与其它入口互切时重置不兼容的内存会话视图；
  - `adopt_shell_prefix` 与 `try_send` 未在 `AI Web` 模式下禁用 `!` 本地 Shell 模式。
- **修复方案**：
  - `set_lx_entry` 在 `AI Web` 与非 `AI Web` 之间切换时，停止在途流并重置不兼容的会话状态（`reset_session_view`、清空 `ai_web_edit_user_index` / `ai_web_edit_cloud_files` / `loading_session`），退出 `Shell` 输入模式并强制将 `@` 面板 Tab 归位为 `Files`；
  - `adopt_shell_prefix` 与 `try_send` 在 `LxEntry::AiWeb` 下不进入/不执行本地 `!` Shell 模式。

---

## P1（高优功能缺陷 / 云端同步与边界异常）

### P1-1：`/resume` 云端历史消息执行 `/rewind` 时丢失附件标记，且原地编辑失败静默退化为发新消息
- **涉及文件**：
  - `src/app/update/chat/ai_web.rs`
  - `src/app/state/lx.rs`
  - `src/chat/prepare/ai_web.rs`
  - `src/chat/ai_web/deepseek_script.js`
  - `src/chat/ai_web/deepseek_helpers.js`
- **关键点**：
  1. `/resume` 加载的云端历史消息只把 `item.files` 填入 `ChatMessageView.attachments`（`source_path: None`），正文 `content` 不含 `@[refer-file](...)` / `@[refer-image](...)` 标记。对其执行 `/rewind` 时输入框只恢复了纯文本（纯附件消息甚至恢复为空串），且重发时脚本无条件清空网页编辑框内的旧附件卡片，导致云端原有附件丢失；
  2. 当 `EDIT_INDEX >= 0`（`/rewind` 后重发）时，若网页端未找到目标用户消息或编辑按钮，`submittedViaEdit` 为 `false`，脚本静默回退到底部输入框发送一条全新消息，造成本地截断旧轮次而云端追加新轮次。
- **错误原因**：
  - 云端恢复的消息未合成对应的 `@[refer-*](filename)` 标记供 `/rewind` 回填，也未在重发时区分“保留未删的云端附件卡片（`keepFiles`）”与“关闭已被用户删除的附件卡片”；
  - 原地编辑分支缺少显式错误抛出。
- **修复方案**：
  - `on_ai_web_session_loaded` 为含附件的云端用户消息合成 `@[refer-image](name)` / `@[refer-file](name)` 标记及 `marker_start`/`marker_end`（气泡渲染自动剥离标记并展示附件树，`/rewind` 则完整回填至输入框）；
  - `rewind_ai_web` 记录该轮无本地路径的云端附件文件名到 `lx.ai_web_edit_cloud_files`；重发时 `prepare_ai_web_message` 将仍保留在草稿中的云端文件名放入 `keep_files` 传给脚本；
  - 脚本在原地编辑容器中按 `KEEP_FILES` 精准保留未删的云端卡片、仅点击关闭被删卡片，并注入新增的本地 `FILES`；若 `EDIT_INDEX >= 0` 时未能打开或提交编辑框，直接抛错提示而非回退发送新消息。

---

### P1-2：`AI Web` 下带参数的斜杠命令（如 `/model foo`）绕过拦截被当成普通消息发送
- **涉及文件**：
  - `src/app/update/chat/ai_web.rs`
- **关键点**：
  - `try_send_ai_web` 此前仅当 `!trimmed.chars().any(char::is_whitespace)` 时才按斜杠命令处理，导致用户输入 `/model gpt-4`、`/mcp enable x` 等带参数的本地命令时直接穿过检查，作为聊天文本发给 DeepSeek 网页端。
- **错误原因**：
  - 斜杠命令识别要求整行无空白，未按首词解析命令名。
- **修复方案**：
  - 凡以 `/` 开头的输入（除已前置处理的 `/rename` / `/title` 外），提取首词识别命令：支持的无参命令（`/new`、`/clear`、`/resume`、`/session`、`/rewind`、`/undo`）要求无多余参数，其余已知本地命令统一提示 `LX_AI_WEB_ONLY_NEW_CMD`，未知 `/xxx` 提示 `unknown_command`。

---

### P1-3：云端会话列表/加载失败可能永久卡在 `loading` 或伪造空会话，且删除/加载缺少流互斥与超时
- **涉及文件**：
  - `src/event/chat/ai_web.rs`
  - `src/chat/ai_web/deepseek_helpers.js`
  - `src/app/update/resume_edit.rs`
  - `src/app/update/chat/ai_web.rs`
- **关键点**：
  1. 脚本中 `list_sessions` 与 `load_session` 对 `fetch` 异常使用 `.catch(() => null)` 并兜底返回空数组，导致网络失败或接口异常时伪造“空会话列表”或“0 条消息的空会话”覆盖本地界面；
  2. `run_query` 与 `spawn_action` 无超时控制，且当 `run_query` 返回的事件中不含 `Sessions` / `SessionLoaded`（例如脚本异常退出）时未回投 `ActionFailed`，导致 `loading sessions…` 或 `loading history…` 永久卡死；
  3. `loading_session` 期间未拦截发送；通过 `/resume` 编辑模态删除当前活跃会话时未停止在途流，也未清理 `ai_web_edit_user_index`。
- **错误原因**：
  - 脚本吞掉了 HTTP 异常；事件层缺少进程超时和未收到终态事件时的 `ActionFailed` 兜底；模态删除路径未对齐会话清理逻辑。
- **修复方案**：
  - 脚本校验 `fetch` 响应 `ok` 与 `data.biz_data`，失败时抛错触发 `error` 事件；
  - `run_query` 与 `spawn_action` 增加 30 秒超时并自动终止超时子进程；`list_sessions` / `load_session` 若未收到目标终态事件一律发送 `ActionFailed`；
  - `loading_session` 为真时禁止发送并提示 `Session busy`；删除活跃会话时先 `stop_stream` 并清空 `ai_web_edit_user_index` / `ai_web_edit_cloud_files`。

---

### P1-4：XHR/Fetch 错误响应过早置位 `sseActive` 导致空响应被判为 `Complete`
- **涉及文件**：
  - `src/chat/ai_web/deepseek_script.js`
  - `src/event/chat/ai_web.rs`
- **关键点**：
  - `attachXhr` 在挂钩请求时立即设置 `this.sseActive = true`，若请求返回 4xx/5xx 非流式错误，`readyState === 4` 调 `feedSseChunk('', true)` 会因 `sseActive` 为真直接将 `this.done` 设为 `true` 并禁用 DOM 兜底；同时 `run_chat` 在子进程异常 EOF 时只要收到过任意片段就会按 `StopReason::Complete` 结算。
- **错误原因**：
  - 未收到有效 SSE `data:` 帧前就设置了 `sseActive = true`；事件层未要求显式收到 `AiWebStreamEvent::Done` 才算正常完成。
- **修复方案**：
  - 仅在 `feedSseChunk` 解析到有效 `data:` 行（或 `fetch` 响应 `response.ok` 且为 `text/event-stream`）时置 `this.sseActive = true`；
  - `run_chat` 记录 `saw_done`，仅在收到 `AiWebStreamEvent::Done` 时按 `StopReason::Complete` 结束，未收到 `Done` 而子进程提前退出的按失败提示处理。

---

## P2（代码规范、单文件行数红线、边缘体验与文档同步）

### P2-1：生产文件逼近 1000 行红线 & 内联测试违反项目规范
- **涉及文件**：
  - `src/chat/ai_web/deepseek_script.js`（997 行）→ 拆分为 `deepseek_helpers.js` + `deepseek_script.js`
  - `src/chat/prepare.rs`（901 行）→ 拆分出 `src/chat/prepare/ai_web.rs`
  - `src/app/update/chat/ai_web.rs`（803 行，含 369 行内联测试）→ 测试迁入 `src/app/update/chat/ai_web/tests.rs`
  - `src/chat/ai_web/mod.rs`（237 行，含 105 行内联测试）→ 测试迁入 `src/chat/ai_web/tests.rs`
  - `src/event/chat/ai_web.rs:378`（`#[allow(clippy::too_many_arguments)]` 补充原因注释）
- **关键点**：
  - `AGENTS.md` 与 `docs/standards/code-writing-standards.md` 规定生产文件上限 1000 行（建议拆分后控制在 500 行左右），且生产文件只保留 `#[cfg(test)] mod tests;`，测试实现必须放在同层 `tests.rs`，`#[allow]` 必须附注释说明原因。
- **错误原因**：
  - 多轮迭代直接在原文件尾部追加逻辑与测试。
- **修复方案**：
  - 将 `deepseek_script.js` 按“会话/上传辅助函数（`deepseek_helpers.js`）”与“流式对话主流程（`deepseek_script.js`）”拆分为两个约 500 行的文件；
  - 将 `prepare_ai_web_message` 独立为 `src/chat/prepare/ai_web.rs`，并在 `src/chat/prepare/tests.rs` 补充单元测试；
  - 将 `src/app/update/chat/ai_web.rs` 与 `src/chat/ai_web/mod.rs` 的内联测试分别迁入各自的 `tests.rs`；
  - 为 `#[allow(clippy::too_many_arguments)]` 补充说明注释。

---

### P2-2：纯文件夹附件消息预览误显示 `[File]` & 极窄窗格下 `loading history…` 不显示
- **涉及文件**：
  - `src/ui/text/chat.rs`
  - `src/ui/main_content/lx/history/snapshot.rs`
  - `src/ui/main_content/lx.rs`
- **关键点**：
  1. `snapshot.rs` 在消息正文为空且无图片附件时统一使用 `file_message_preview`（`[File]` / `[Files]`），导致 `Native` 模式下纯文件夹附件消息显示为 `[File]` 而非 `[Folder]`；
  2. `src/ui/main_content/lx.rs` 在 `loading_session` 为真时直接调用 `grid.put_centered(..., LX_SESSION_LOADING, ...)`，当窗格宽度小于 18 列时因未截断而完全不显示。
- **错误原因**：
  - 预览文案未区分 `MessageAttachmentKind::Folder`；居中绘制前未做宽度截断。
- **修复方案**：
  - 在 `src/ui/text/chat.rs` 新增 `folder_message_preview`，在 `snapshot.rs` 中当附件全为文件夹时显示 `[Folder]` / `[Folders]`；
  - 在 `src/ui/main_content/lx.rs` 中先用 `truncate_to_width(text::LX_SESSION_LOADING, usize::from(chat_rect.width))` 截断后再居中绘制。

---

### P2-3：特性文档 `design.md` 与 `task.md` 滞后于代码实现
- **涉及文件**：
  - `docs/feature/lx-ai-web/design.md`
  - `docs/feature/lx-ai-web/task.md`
- **关键点**：
  - `design.md` 仍保留“屏蔽 `/resume`、`/rewind`、`/rename`”、“NDJSON over Stdout + 环境变量传参”、“`lx-[AI Web]` 带方括号标题”、“保留 `refDir` Tab”等过时描述，缺少回环 HTTP 流、`const __LX_PARAMS`、思考块默认折叠、云端会话管理与 1 步 `/rewind`、居中加载态与全局空态居中、以及禁用 `refDir` 仅保留 `Files` Tab 的完整说明；
  - `task.md` P1–P4 缺少上述新增任务与回归测试项，且需新增 P5 记录本次 Review 修复闭环。
- **错误原因**：
  - 后续增量提交未同步回写设计文档与任务清单。
- **修复方案**：
  - 全面更新 `docs/feature/lx-ai-web/design.md` 与 `docs/feature/lx-ai-web/task.md`，并同步至主工作区 `docs/feature/lx-ai-web/`。

---

## Web Markdown 保真提取与流式渲染专项 Review（按 P0 → P3 排序）

> **审查范围**：跨站点 DOM → Markdown 序列化器（`src/chat/web/dom_markdown.js`）、DeepSeek / Gemini / Grok 实时流与历史提取脚本（`src/chat/web/{deepseek,gemini,grok}_{script,helpers}.js`）、以及 TUI 面向阅读的 Markdown 渲染器（`src/ui/markdown_render.rs`、`src/ui/markdown_render/inline.rs`、`src/ui/markdown_render/code_block.rs`）。
> **结论**：共发现 **3 个 P0、4 个 P1、3 个 P2、1 个 P3**（合计 **11 项**），已 **100% 修复并通过回归测试**。

### 专项问题总览表（P0 → P3）

| 编号 | 级别 | 涉及模块 / 文件 | 核心问题描述 | 状态 |
| --- | --- | --- | --- | --- |
| **P0-1** | **P0** | `src/chat/web/gemini_script.js:252-266`<br>`src/chat/web/deepseek_script.js:126-140, 661-664` | 正文流式输出期间思考块仍按 `streaming: true` 序列化，暂缓的闭合 `**` / ```` ``` ```` 在停止生成后晚于 `text` 吐出，分裂出第二个尾部 `○ Thought` 块 | **已修复** |
| **P0-2** | **P0** | `src/chat/web/grok_script.js:347-351, 454-589` | Grok 首段 WS 文本后出现 `tool_usage_card` 中断 Node WS 读取但未复位浏览器 `s.wsActive`，导致 WS 与 DOM 双通道死锁；跨帧分割的 `<grok:render>` / `<argument>` 泄漏原始引用编号 | **已修复** |
| **P0-3** | **P0** | `src/chat/web/deepseek_script.js:564-608, 648-650` | DeepThink 思考超过 1.2s（`sseThinking = true, sseText = false`）时 `tickDom()` 越权启动 DOM 通道；`readFiberMarkdown` 向上 20 层无 DOM 宿主边界校验越界爬到父组件 | **已修复** |
| **P1-1** | **P1** | `src/chat/web/dom_markdown.js:486-513, 563-570` | `<li>` 首块为代码块/数学块/表格/引用时产生 `` - ```rust `` 围栏倒置，列表内 `<h3>` 泄漏顶层标题破坏列表上下文，且列表内围栏代码正文行被多加 `contIndent` 空格 | **已修复** |
| **P1-2** | **P1** | `src/ui/markdown_render/inline.rs:10-231` | `inline_spans` 未递归解析强调块（`**`/`*`/`***`/`~~`）内部的行内代码、链接与内层强调，不支持双反引号 `` ``code`` ``，且 `delimited_emphasis` 在尾随 `***` 与代码内 `**` 处闭合错位 | **已修复** |
| **P1-3** | **P1** | `src/ui/markdown_render.rs:188-189, 338-345, 526-530, 605-629`<br>`src/ui/markdown_render/code_block.rs:124-131` | 列表续段缩进硬编码 `+ 2`（导致有序列表 `1. ` 3 列、任务列表 `[x] ` 4 列错位），`update_list_context` 在内层子列表结束后整栈清空而非逐层 `pop()`，且列表内缩进围栏复制残留前导空格 | **已修复** |
| **P1-4** | **P1** | `src/chat/web/deepseek_script.js:93-125, 422-436`<br>`src/chat/web/deepseek_helpers.js:390-405` | SSE 结束时未刷出被 `holdIncomplete = true` 暂缓的末尾 `[`；`load_session` 历史清洗误删代码块内的 `[citation:...]` 且 `\s*` 吞掉前导换行 `\n` | **已修复** |
| **P2-1** | **P2** | `src/chat/web/dom_markdown.js:110-143, 233`<br>`src/chat/web/gemini_script.js:178-190, 259-264` | `hasMeaningfulContent` 漏判 `<img src="https://...">` 并误判空 KaTeX 节点；`markdown` 代码块流中遇到内层 ```` ``` ```` 时围栏从 3 跳变到 4；Gemini 逗号选择器按文档前序匹配到外层父容器 | **已修复** |
| **P2-2** | **P2** | `src/chat/web/deepseek_script.js:598-608`<br>`src/chat/web/gemini_script.js:207, 229` | DeepSeek SSE 异常断流未发 `finish` 帧时 `tickDom()` 直接 `return` 导致挂死 300s；Gemini `/rewind` 重置检测只看 `announcerBusy` 漏看 `stopBtn` | **已修复** |
| **P2-3** | **P2** | `src/chat/web/grok_script.js:210-225, 250-258, 513-515` | `stripGrokDirectives` 在流式生成期间调用 `.trim()` 吞掉代码行尾缩进空格；早期 `response.created` 帧未同步 `s.wsSeen` | **已修复** |
| **P3-1** | **P3** | `src/chat/web/deepseek_script.js:161, 178`<br>`src/chat/web/gemini_script.js:128, 152`<br>`src/chat/web/grok_script.js:286, 299` | `pushDom` / `pushText` 尾部锚点窗口（`240`）与 LCP 窗口（`40`）偏小，单帧大表格渲染或长链接文本替换时易漏发增量 | **已修复** |

---

### P0 级问题详解（已修复）

#### [P0-1] `src/chat/web/gemini_script.js:252-266`、`src/chat/web/deepseek_script.js:126-140, 661-664` — 正文流式输出期间思考块仍传 `streaming: true` 导致末尾闭合标记分裂出第二个 `○ Thought` 块

- **关键点**：
  - `gemini_script.js` 原先在 `pollDom()` 中对思考块和正文块同时传入 `{ streaming: isGenerating }`；`deepseek_script.js` 的 `tickDom()` 同样对 `thinkEl` 传入 `isStreaming`。
  - 当思考块以粗体（`**总结**`）、行内代码（`` `foo` ``）或围栏代码块（```` ``` ````）结尾时，`lxHtmlToMarkdown` 的 `streaming && isTail(el)` 规则会暂缓输出最末尾的右闭合标记（`**`、`` ` ``、```` ``` ````），直到 `streaming` 变为 `false`。
- **错误原因**：
  - 模型进入正文回答阶段后，网页仍在生成（`isGenerating === true`），但思考块早已结束。因为思考块仍按 `streaming: true` 序列化，其末尾闭合 `**` 被一直扣留到整轮回答彻底停止（`isGenerating === false`）才作为新的 `thinking` delta 发出。
  - Rust 侧 `AppState` 在收到 `text` 增量后若再收到迟到的 `thinking` 增量（`"**"`），会在正文下方新建第二个 `PartKind::Reasoning`，界面底部多出一个空白或只有标点的 `○ Thought · 0s`。
- **影响/复现**：
  - 在 Gemini 或 DeepSeek（DOM 兜底路径）开启思考模式提问，只要思考过程最后一句带粗体或代码片段，回答结束后正文下方就会多出一个孤立的 `○ Thought` 折叠行。
- **修复（已修复）**：
  - 在 `gemini_script.js:258` 与 `deepseek_script.js:662` 中先提取 `bodyText`，将思考块的流式标志收敛为 `const thinkStreaming = isGenerating && !bodyText && !this.domText`（DeepSeek 为 `isStreaming && !bodyText && !this.domTextStarted`）：一旦正文非空，立即以 `streaming: false` 序列化思考块，在第一个 `text` delta 入队前把思考块末尾的闭合标记刷出；
  - 同时在 `gemini_script.js:103`、`deepseek_script.js:129, 139` 的 `push` / `pushDom` 中增加硬门禁：一旦 `domText` / `sseText` / `domTextStarted` 已启动，直接丢弃任何迟到的 `thinking` 增量。

---

#### [P0-2] `src/chat/web/grok_script.js:347-351, 454-589` — Grok 搜索/工具卡片（`tool_usage_card`）中断 WebSocket 流导致双通道死锁，且跨帧分割 `<grok:render>` 泄漏引用编号

- **关键点**：
  1. `grok_script.js` 通过 CDP `Network.webSocketFrameReceived` 监听 `CHANNEL_ASSISTANT_RESPONSE` 实时获取原始 Markdown，并在收到首个 `wsChunk` 时将浏览器侧 `window.__lxGrokStream.wsActive` 置为 `true`（从而屏蔽 `pollDom` 的 `pushText`）。
  2. 原实现一遇到 `chunk.tool_usage_card`（联网搜索、代码执行等工具卡片）且当时已吐出过引导文字，就将 `wsAbortedToDom = true` 并停止在 Node 侧解析后续 WS 帧，但**从未将浏览器内的 `s.wsActive` 重置回 `false`**；
  3. 同时，原先对每个单帧 `chunk.text.text` 单独执行正则剥离 `<grok:render>`，而 Grok WebSocket 经常把 `<grok:render card_id="...">`、`<argument name="citation_id">1</argument>`、`</grok:render>` 拆在 2～4 个连续 WS 帧里发送。
- **错误原因**：
  1. Node 侧停止推送 `wsDelta` 后，浏览器侧 `s.wsActive` 仍为 `true`，`pollDom()` 走到 `else if (this.wsActive) { this.lastText = text; }` 只更新游标不入队，导致工具调用后的整段回答正文在 WS 与 DOM 两个通道同时被丢弃（双通道死锁）；
  2. 逐帧无状态正则无法匹配跨帧拆分的 XML 标签，导致中间帧里的数字 `1`、`2` 直接穿透到 TUI 正文。
- **影响/复现**：
  - 让 Grok 执行任何带联网搜索且先说一句“让我为您搜索…”的回答：搜索完成后 TUI 正文永远停在第一句，且句尾穿插孤立的引用编号数字。
- **修复（已修复）**：
  - 移除 `wsAbortedToDom` 自杀逻辑，改为维护累积缓冲区 `wsRawBuffer`、`wsEmittedCleaned`、`wsLastToolStep` 与 `wsAfterToolCard`（`grok_script.js:448-578`）：遇到 `chunk.tool_usage_card` 时记录 `wsLastToolStep = stepId` 并在已有文本时标记 `wsAfterToolCard = true`，后续新的 `CHANNEL_ASSISTANT_RESPONSE` 块自动补齐 `\n\n` 分段后继续由 WS 通道流式输出；
  - 新增 `cleanGrokWsBuffer(wsRawBuffer, !wsDoneSeen)`（`grok_script.js:454-484`）：在累积缓冲区上剥离完整 `<grok:render>...</grok:render>` 与 `<argument>...</argument>`，并在流未结束（`holdIncomplete = true`）时检测末尾未闭合的 `<grok:` / `</grok:` / `<argument` / `</argument` 前缀予以暂缓，流结束（`wsDoneSeen = true`）再一次性兜底清理残余半标签。

---

#### [P0-3] `src/chat/web/deepseek_script.js:564-608, 648-650` — DeepThink 思考超过 1.2s 时 `tickDom()` 越权启动 DOM 通道 & `readFiberMarkdown` 无边界爬树污染

- **关键点**：
  1. `deepseek_script.js` 原先在 `tickDom()` 中仅当 `this.sseText` 为真时才跳过 DOM 采集，且 `graceOver` 按 `now - this.submittedAt >= 1200` 计算；
  2. `readFiberMarkdown(el)` 原先沿着 `fiber = fiber.return` 无条件向上爬 20 层，只要任意祖先 Fiber 的 `memoizedProps` 含有 `content` 或 `markdown` 字符串就直接返回。
- **错误原因**：
  1. 开启 DeepThink（深度思考）时，模型思考阶段通常持续 5～30 秒（期间 `sseThinking = true` 但 `sseText = false`）。提交 1.2 秒后 `tickDom()` 判定 `graceOver === true` 且 `!this.sseText`，立即启动 DOM 轮询；而一旦 DOM 抢先置位 `domTextStarted = true`，`push()` 中的 `if (kind === 'text' && this.domTextStarted) return;` 就会把后续真正的 SSE 原始 Markdown 正文彻底屏蔽；
  2. React Fiber 树向上 20 层早已越出当前 `.ds-markdown` 宿主节点，爬到外层消息列表或会话容器组件，可能把相邻块或整条消息的旧 `content` 属性误当成当前子节点的 Markdown 返回。
- **影响/复现**：
  - DeepSeek 开启 `deepthink` 且思考超过 1.2 秒时，DOM 通道提前介入抢走控制权；历史/完成态读取 Fiber 时可能拿到外层组件文本造成内容重复。
- **修复（已修复）**：
  - 在 `tickDom()` 入口（`deepseek_script.js:598`）将隔离守卫收紧为 `if (this.sseThinking || this.sseText)`：只要 SSE 通道已产出思考或正文，严禁 DOM 通道再介入采集；同时记录 `sseSeenAt`（`deepseek_script.js:473, 547, 648`），以首个 SSE 事件到达时间计算 1200ms 宽限期；
  - 在 `readFiberMarkdown(el)`（`deepseek_script.js:564-591`）中将最大深度收紧为 `depth < 10`，每步检查宿主边界 `if (depth > 0 && fiber.stateNode instanceof Element && fiber.stateNode !== el && !el.contains(fiber.stateNode)) break;`，且仅接受带 DeepSeek Markdown 组件特征属性（`'citeProps' in props || 'cacheKey' in props || 'mergeConsecutiveCites' in props`）的 `props.content` / `props.markdown`。
  - 新增回归测试 `src/chat/web/tests.rs::deepseek_stream_collector_survives_non_sse_xhr_and_preserves_markdown_fences`（模拟 DeepThink 思考 5000ms 后再发正文，断言 `domTextStarted` 与 `domThinkingStarted` 恒为 `false`）。

---

### P1 级问题详解（已修复）

#### [P1-1] `src/chat/web/dom_markdown.js:486-513, 563-570` — `<li>` 首块为代码块/块级结构时围栏倒置、列表内标题 `###` 破坏层级、以及列表内围栏代码被多加缩进

- **关键点**：
  1. `serializeListItem`（`dom_markdown.js:486-513`）原先对第 0 个块级分段（`i === 0 && seg.kind !== 'list'`）无条件把 `lines[0]` 直接拼在 `indent + marker` 同一行后（如 ```` - ```rust ````），而把闭合围栏放在下一行 `  ``` `；
  2. 当网页在 `<li>` 内部渲染 `<h3>小标题</h3><p>正文</p>` 时，`serializeBlock` 生成 `### 小标题`，拼成 `- ### 小标题` 或在续行输出 `  ### 小标题`；
  3. 列表项内包含多行围栏代码块时，原先对代码块所有续行一律前缀 `contIndent`（如 2 空格），而 TUI `markdown_render` 在围栏内部逐行原样渲染，导致代码正文每行都被多加了 2 个前导空格（复制代码时也带出多余缩进）。
- **错误原因**：
  - 未识别必须独占一行的块级结构（围栏代码块 ```` ``` ````、块级公式 `$$\n`、表格 `| `、引用 `> `），也未区分围栏开闭行与代码正文行的缩进语义；在 `inListItem` 上下文中未降级 ATX 标题标签。
- **影响/复现**：
  - 模型在列表项开头直接贴代码块时，TUI 渲染出未闭合的乱码围栏；列表项内代码块每行代码缩进平白多出 2～4 列。
- **修复（已修复）**：
  - 在 `serializeListItem`（`dom_markdown.js:488-509`）中检测 `isFenceBlock = /^`{3,}/.test(cleaned)` 与 `requiresOwnLine = isFenceBlock || /^(?:\$\$\n|\| |> )/.test(cleaned)`：当首段 `requiresOwnLine` 为真时先输出 `indent + marker.trimEnd() + '\n'` 换行，且对 `isFenceBlock` 仅在首行（开启围栏）与末行（闭合围栏）添加 `contIndent`，代码内部各行保持原始缩进；
  - 在 `serializeBlock`（`dom_markdown.js:563-566`）中当 `ctx.inListItem` 为真且遇到 `h1`–`h6` 时，转为 `wrapInline(el, '**', 'inStrong', ctx).trim()`（粗体小标题），防止 `#` 冲出列表上下文。

---

#### [P1-2] `src/ui/markdown_render/inline.rs:10-231` — 强调块内部未递归解析行内代码/链接/内层强调、不支持双反引号 `` ``code`` ``、且尾随 `***` 闭合错位

- **关键点**：
  1. `inline_spans` 原先匹配到 `**...**`、`*...*`、`***...***`、`~~...~~` 后，直接将内部文本作为单个纯文本 `Span` 推入 `spans`，不再递归调用行内解析；
  2. 行内代码只按单字符 `` ` `` 查找下一个 `` ` ``，不支持 CommonMark 多反引号 `` ``code ` inner`` ``；
  3. `delimited_emphasis` 查找闭合标记时未跳过反引号代码片段，且遇到 `**bold *italic***` 末尾连续 3 个 `*` 时，外层 `**` 在前两个 `*` 处提前闭合，留下孤立的 `*`。
- **错误原因**：
  - 行内解析器原本是单层线性扫描，缺少强调块递归合成函数（`push_nested_spans`）、多反引号配对（`code_span`）以及同字符尾随链对齐逻辑。
- **影响/复现**：
  - 网页 AI 频繁输出 `- **修改 \`config.toml\` 文件**`、`**[官方文档](https://example.com)**` 或 `**bold *italic***`，TUI 中会原样露出反引号 `` `config.toml` ``、方括号 `[官方文档](...)` 或残留 `*`。
- **修复（已修复）**：
  - 在 `src/ui/markdown_render/inline.rs:99-161` 新增 `push_nested_spans` 与 `code_span`：强调块内部递归调用 `inline_spans` 并用 `child_style.patch(outer_style)` 叠加样式（如粗体内的斜体合成为 `md_read_strong_emphasis()`，行内代码保持 `md_read_inline_code()`）；支持任意长度反引号对并按 CommonMark 规则剥离首尾单空格；
  - 在 `delimited_emphasis`（`inline.rs:167-231`）中扫描时先调用 `code_span` 跳过反引号代码段，并在遇到连续同字符闭合链 `run` 时取 `end = i + run - delim_len` 对齐外层闭合边界（单字符 `*`/`_` 仅在 `run_chars == 1 || run_chars == 3` 时闭合）。
  - 新增单元测试 `src/ui/markdown_render/tests.rs::nested_inline_code_links_and_emphasis_render_without_leaking_delimiters`。

---

#### [P1-3] `src/ui/markdown_render.rs:188-189, 338-345, 526-530, 605-629` & `src/ui/markdown_render/code_block.rs:124-131` — 有序/任务列表续段缩进错位、子列表回退清空整栈、以及列表内围栏代码复制残留缩进

- **关键点**：
  1. `render_blocks_stateful`（`markdown_render.rs:338-345`）计算列表续段缩进时原先写死为 `indent(depth) + 2`，而有序列表 `1. ` 标记宽 3 列（`10. ` 宽 4 列）、任务列表 `[x] ` 标记宽 4 列；
  2. `update_list_context`（`markdown_render.rs:605-629`）原先在遇到 `indent < last_content_indent` 的非列表行时直接 `clear()` 整个 `list_indents` 栈，导致二级子列表（缩进 4 列）结束后的顶层列表续段（缩进 2 列）丢失顶层列表上下文，退化到第 0 列；
  3. `code_block.rs:124-131`（`extract_code_block` 与围栏闭合识别）未记录并剥离列表内围栏开启行的前导空格。
- **错误原因**：
  - `ParserState` 缺少记录每层列表项实际标记悬挂宽度的 `list_hangs` 栈；非列表行回退列表层级时未按栈结构逐层 `pop()`；代码块提取器未按 `fence_indent` 剥离列表续行缩进。
- **影响/复现**：
  - 有序列表 `1. **标题**\n   续段` 的续段向左偏 1 列；任务列表 `- [x] **标题**\n      续段` 的续段向左偏 2 列；嵌套子列表后的外层说明段贴到最左第 0 列；复制列表内代码块时每行多出前导空格。
- **修复（已修复）**：
  - 在 `ListItem` 新增 `hang_width(&self) -> u16`（`markdown_render.rs:525-531`），在 `ParserState` 新增 `list_hangs: Vec<u16>`（`markdown_render.rs:189`），续段缩进改为 `indent(depth).saturating_add(hang)`（`markdown_render.rs:342-343`）；
  - `update_list_context`（`markdown_render.rs:620-628`）改为 `while state.list_content_indents.last().is_some_and(|&content| indent < content)` 逐层 `pop()`；
  - 在 `src/ui/markdown_render/code_block.rs:124-131` 记录围栏开启行的 `fence_indent` 并从代码行剥离至多 `fence_indent` 个前导空格。
  - 新增单元测试 `src/ui/markdown_render/tests.rs::ordered_task_list_continuations_and_indented_fences_keep_exact_columns`。

---

#### [P1-4] `src/chat/web/deepseek_script.js:93-125, 422-436` & `src/chat/web/deepseek_helpers.js:390-405` — SSE 结束漏刷末尾暂缓的 `[` & 历史会话清洗破坏代码块与换行

- **关键点**：
  1. `deepseek_script.js` 的 `applyFragmentContent` 在流式期间调用 `this.cleanText(nextRaw, true)`，若回答最后一个字符恰好是 `[`（如数学半开区间 `[0, 1[` 或 `[a`），`holdIncomplete = true` 会将其当作未闭合 `[citation:` 的前缀暂缓，而 SSE `finish` 事件只置 `this.done = true`，从未以 `holdIncomplete = false` 补刷最后残余字符；
  2. `deepseek_helpers.js` 的 `load_session` 原先对整段消息直接执行 `text.replace(/\s*\[(?:citation|reference):[^\]]*\]/g, '')`，既没有跳过 ```` ``` ```` 围栏代码块，又用 `\s*` 吞掉了引用标记前的换行符 `\n`。
- **错误原因**：
  - 缺少流结束时的 `finishSse()` 尾刷步骤；历史加载清洗正则未按围栏代码块分段，且 `\s` 包含 `\n`。
- **影响/复现**：
  - DeepSeek 回答末尾带 `[` 时最后一个字符丢失；`/resume` 恢复含 `[citation:1]` 代码示例的历史会话时代码被篡改，或行首 `[citation:1]` 导致上下两段被合并成一行。
- **修复（已修复）**：
  - 在 `deepseek_script.js:422-436` 新增 `finishSse()`：遍历所有 `text` 片段，用 `this.cleanText(raw, false)` 计算完整文本并补推尚未发出的尾部切片，在 `status === 'FINISHED'`、`event: finish/close`、`[DONE]` 及流 EOF 时统一调用；
  - 在 `deepseek_helpers.js:390-405` 将历史清洗改为与实时流一致的围栏分段 `cleanText`，且仅剥离水平空白 `[ \t]*\[(?:citation|reference):[^\]]*\]`。
  - 在 `src/chat/web/tests.rs::deepseek_stream_collector_survives_non_sse_xhr_and_preserves_markdown_fences` 中验证代码块内 `[citation:1]` 保留且末尾 `区间 [0, 1[` 完整输出。

---

### P2 级问题详解（已修复）

#### [P2-1] `src/chat/web/dom_markdown.js:110-143, 233` & `src/chat/web/gemini_script.js:178-190, 259-264` — `isTail` 内容判定缺口、`markdown` 代码块围栏中途跳变与 Gemini 子选择器顺序反转

- **关键点**：
  1. `dom_markdown.js` 的 `hasMeaningfulContent`（决定节点是否为流式末尾 `isTail`）未处理 `<img src="https://...">`，且对 `<span class="katex">` 未检查内部 TeX 是否非空、对代码块未检查 `<pre>` 是否有内容；
  2. `serializeCodeBlock`（`dom_markdown.js:233`）默认使用 3 个反引号 ```` ``` ````，当代码块语言为 `markdown`/`md`/`mdx` 且流式输出到一半出现内层 ```` ``` ```` 时，外层围栏突然从 ```` ``` ```` 升级为 ` ```` `，破坏已流出的前缀；
  3. `gemini_script.js` 的 `extractCleanText` 原先对逗号选择器直接调用一次 `root.querySelector(selectorList)`，而 CSS 规范对 `a, b` 按 DOM 文档前序返回首个匹配节点，导致 `'model-thoughts .thoughts-content, model-thoughts'` 永远命中外层父元素 `<model-thoughts>`（把标题按钮一并选入）。
- **错误原因**：
  - `hasMeaningfulContent` 缺少对 `<img>`、空公式容器与空代码块的特判；`markdown` 语言代码块未预先分配 4 反引号围栏；误以为原生 `querySelector("child, parent")` 会按选择器书写顺序返回子节点。
- **影响/复现**：
  - 紧跟在图片或未渲染完成的空公式前的粗体节点被误判为 `isTail` 或非 `isTail`；生成 Markdown 示例时围栏长度跳变；Gemini 思考块偶发混入外层容器文本。
- **修复（已修复）**：
  - 在 `dom_markdown.js:119-138` 补齐 `<img>`（`/^https?:\/\//i`）、`isMathContainer`（校验 `annotation` 或文本非空）与 `isCodeBlockContainer`（校验 `pre/code` 非空）的内容判定；
  - 在 `dom_markdown.js:233` 对 `/^(?:markdown|md|mdx)$/i.test(lang)` 自始至终使用至少 4 个反引号（`maxTicks = 3`，`fence = '`'.repeat(maxTicks + 1)`）；
  - 在 `gemini_script.js:180-188` 按逗号拆分 `selectorList` 依次执行 `root.querySelector(sel)`，并将思考块选择器顺序统一为 `'model-thoughts .thoughts-content, thinking-overlay .thoughts-content, model-thoughts, thinking-overlay'`。

---

#### [P2-2] `src/chat/web/deepseek_script.js:598-608` & `src/chat/web/gemini_script.js:207, 229` — DeepSeek SSE 异常断流无 `finish` 帧挂死 300s & Gemini `/rewind` 重置检测漏判 `stopBtn`

- **关键点**：
  1. DeepSeek 的 SSE 流偶发在输出完最后一个 token 后因网络抖动断开且未发送 `event: finish` 帧，此时 `sseText = true` 但 `this.done` 仍为 `false`；`tickDom()` 在 `if (this.sseThinking || this.sseText) return;` 处直接返回，导致主循环空等 300 秒直至超时；
  2. `gemini_script.js` 的 `/rewind` 编辑重置检测（`pollDom` 内 `this.isEdit && !this.editResetSeen`）原先只检查 `announcerBusy`，当 Angular 先挂载停止按钮 `stopBtn` 而尚未设置 `aria-busy="true"` 时会漏判首帧重置。
- **错误原因**：
  - DeepSeek 在 SSE 接管期间缺少基于网页按钮空闲态的兜底完结判定；Gemini 重置条件未复用统一的 `isGenerating = Boolean(stopBtn) || announcerBusy`。
- **影响/复现**：
  - DeepSeek 网络尾帧丢失时回答已完整显示却卡在 `streaming` 状态 5 分钟；Gemini `/rewind` 偶发晚一帧清空游标。
- **修复（已修复）**：
  - 在 `deepseek_script.js:598-608` 的 `if (this.sseThinking || this.sseText)` 分支内检查 `if (idleButton && !stopIcon)`：若连续 15 个 tick（约 900ms）发送按钮处于空闲禁用且无停止图标，自动调用 `this.finishSse()` 正常完结；
  - 在 `gemini_script.js:207, 229` 将 `isGenerating` 提前计算并用于 `if (isGenerating || curBody !== ...)` 重置检测。

---

#### [P2-3] `src/chat/web/grok_script.js:210-225, 250-258, 513-515` — `stripGrokDirectives` 流中调用 `.trim()` 吞掉代码缩进空格 & 早期 WS 帧未同步 `s.wsSeen`

- **关键点**：
  1. `grok_script.js` 的 `stripGrokDirectives(text)` 原先无条件在末尾调用 `.trim()`，当流式光标正好停在代码块新一行前导缩进空格（如 `fn main() {\n    `）时，`.trim()` 会把末尾的 4 个空格全部裁掉；下一帧 `println!()` 到达时基于 `previous` 切片或锚点对齐会丢失缩进或造成错位；
  2. Node 侧收到 `response.created` 早期帧时未同步给浏览器侧 `s.wsSeen`，导致 `pollDom` 在 1.2s 宽限期后可能先于首个文本帧抢跑。
- **错误原因**：
  - 流式中间态不得对末尾空白执行 `.trim()`（只能清理前导空行 `^\n+`）；`wsSeen` 标志未跨上下文透传。
- **影响/复现**：
  - Grok 走 DOM 通道流式输出代码块时偶发丢失行首缩进；慢响应首包超过 1.2s 时 DOM 通道可能与 WS 通道竞争。
- **修复（已修复）**：
  - 将 `stripGrokDirectives(text, streaming)`（`grok_script.js:210-225`）改为仅在 `!streaming`（完成态）时 `.trim()`，流式期间保留尾部空格；
  - 在 Node 侧解析到 `evObj.type === 'response.created' || evObj.type === 'response.chunk'` 时置 `wsSeen = true`（`grok_script.js:513-515`）并同步至 `s.wsSeen`（`grok_script.js:585-587`）。

---

### P3 级问题详解（已修复）

#### [P3-1] `src/chat/web/deepseek_script.js:161, 178`、`src/chat/web/gemini_script.js:128, 152`、`src/chat/web/grok_script.js:286, 299` — 扩充 `pushDom` / `pushText` 尾部锚点窗口与 LCP 闭合窗口

- **关键点**：
  - 三个站点的 `pushDom` / `pushText` 尾部锚点对齐窗口原先为 `pos + len >= current.length - 240`，LCP（最长公共前缀）闭合窗口原先为 `previous.length - lcp <= 40`。
- **错误原因**：
  - 当网页在单个 60ms 采样周期内一次性挂载整行宽表格（连同表头分隔线 `| --- | --- |` 超过 240 字符），或在流末尾将 `[https://...]` 替换为较长的来源标题并补齐 `(url)`（超过 40 字符）时，旧窗口会判定对齐失败，需等待下一帧或漏发中间增量。
- **影响/复现**：
  - 超宽多列表格在 DOM 通道下首行与分隔线偶发漏发一帧；长链接闭合时偶发跳过闭合括号。
- **修复（已修复）**：
  - 将 `deepseek_script.js`、`gemini_script.js`、`grok_script.js` 三处的尾部锚点窗口统一从 `240` 扩充至 `1024`（`pos + len >= current.length - 1024`），并将 LCP 闭合回退窗口统一从 `40` 扩充至 `160`（`previous.length - lcp <= 160`）。
