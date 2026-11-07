# lx `AI Web` 入口与 DeepSeek 网页端接入 — 执行任务清单

> 状态：**已在 `.worktrees/lx-ai-web`（`feature/lx-ai-web`）完成实现、代码审查（[review.md](review.md)）与 P0–P2 修复**（2026-11-07）
> 设计依据：[design.md](design.md) · 审查报告：[review.md](review.md)
> 阶段划分：P0（开工门槛与基线核对）→ P1（`app/` 状态模型与云同步会话更新）→ P2（`ui/` 欢迎页 4 入口、标题、居中加载/空态与底边框渲染）→ P3（`chat/ai_web` 与 `event/` 回环 HTTP 流、云会话与 CDP 文件上传）→ P4（单元测试与精确校验）→ P5（代码审查 P0–P2 问题闭环修复）。

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
