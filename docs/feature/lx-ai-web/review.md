# lx `AI Web` 特性代码审查报告（Review）

> 审查范围：`.worktrees/lx-ai-web`（分支 `feature/lx-ai-web`，提交 `77f73eb`..`40e5238`）及特性文档 `docs/feature/lx-ai-web/{design.md,task.md}`
> 审查基准：`AGENTS.md`、`docs/standards/project-directory-structure.md`、`docs/standards/code-writing-standards.md`、`docs/standards/tui-design-requirements.md`
> 优先级定义：**P0**（阻断核心流程 / 违背明确需求边界 / 会话状态损坏）→ **P1**（高优功能缺陷 / 云端同步与异常兜底缺失）→ **P2**（代码规范、单文件行数红线、边缘体验与文档漂移）

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
