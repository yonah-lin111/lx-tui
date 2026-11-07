# lx `AI Web` 入口与 DeepSeek 网页端接入（基于 `ego-browser`）— 设计文档

> 状态：**已在 `.worktrees/lx-ai-web`（`feature/lx-ai-web`）完成实现、代码审查（[review.md](review.md)）与回归修复**（2026-11-07）
> 执行任务清单见 [task.md](task.md)，审查报告见 [review.md](review.md)。

---

## 0. 现状核实与设计基线

| 事实 | 代码位置 | 对本设计的影响 |
|---|---|---|
| `LxEntry` 新增第 4 个同行入口 `LxEntry::AiWeb`，且前三个入口标签收短为 `"Native"`、`"Hybrid"`、`"Automation"`；窗格左上角边框标题统一为无方括号格式 `lx-<entry>`（如 `lx-Native`、`lx-AI Web`） | `src/app/state/lx.rs`、`src/ui/text.rs`、`src/ui/panes.rs` | 欢迎页 4 个入口单行紧凑横排，窗格顶栏标题清晰一致 |
| `ui::main_content::lx::layout` 与 `place_entries` 按 `labels.len()` 动态计算横排总宽与竖排总高 | `src/ui/main_content/lx.rs` | 确保 4 个入口居中与窄屏折行准确 |
| 欢迎页入口标签下方固定有 1 行 `lay.inventory` 槽位，在 `Native` 等入口下渲染 `{agents_md} AGENTS.md · {mcp} mcp · {skills} skills` | `src/ui/main_content/lx.rs`、`src/ui/main_content/lx/brand.rs` | 当 `lx.entry == LxEntry::AiWeb` 时，复用 `lay.inventory` 几何槽位改为渲染可点击的 AI Web 站点选择器（首期 `DeepSeek`），不展示 `AGENTS.md / mcp / skills` |
| 窗格底边框状态栏 `status::render` 从 `lx.chat.status` 读取 `model_name`、`provider_name`、`variant`、`context_limit`、`input_tokens` | `src/ui/main_content/lx/status.rs`、`src/app/update/chat/ai_web.rs` | 当 `lx.entry == LxEntry::AiWeb` 时，`apply_ai_web_status` 将 `status.model_name` 设为当前选中站点名（`"DeepSeek"`），并将 `provider_name`、`variant`、`context_limit`、`input_tokens` 置 `None` |
| `AI Web` 采用纯内存视图 + 云端会话同步，不写本地 SQLite；也不加载 `lx` 系统提示词、`AGENTS.md`、MCP、Skills 与本地工具（包括禁用 `!` 本地 Shell 模式与 `refDir` 引用目录） | `src/app/update/chat/mod.rs`、`src/app/update/chat/ai_web.rs` | 与 `Native` 本地数据库会话严格隔离，互切入口时自动清理不兼容的会话状态 |
| 本机 `ego-browser` 支持 `ego-browser nodejs` 执行确定性脚本，通过 `useOrCreateTaskSpace('lx-tui-ai-web-deepseek')` 复用用户登录态，并通过 CDP `DOM.setFileInputFiles` 注入本地文件 | `src/event/chat/ai_web.rs`、`src/chat/ai_web/deepseek_script.js` | Rust 事件层通过本地回环 HTTP 监听（`127.0.0.1:0` `streamUrl`）+ stdout/stderr 双通道接收 `@@LX_AI_WEB@@` NDJSON 事件流，实现零延迟流式思考与正文渲染 |

---

## 1. 已确认决策汇总（Grill Me 收口）

| 编号 | 决策维度 | 确认结论 |
|---|---|---|
| D1 | 技术路线选择 | 采用**真实浏览器流拦截 + `lx-tui` 原生字符界面渲染**。100% 复用 `PaneView::Lx` 的流式 Markdown 与默认折叠的 `Thinking` 思考块（生成中 `● Thinking`，完成后 `○ Thought · Xs`）。 |
| D2 | 入口定位与上下文边界 | 在 `LxEntry` 新增第 4 个入口 `AiWeb`（文案 `AI Web`），前三个入口标签收短为 `Native`、`Hybrid`、`Automation`，窗格标题为 `lx-<entry>`。进入 `AI Web` 后仅做**纯网页对话**，不加载 `lx` 系统提示词、`AGENTS.md`、MCP、Skills、本地工具及 `!` Shell 模式。 |
| D3 | 实时流传输与参数隔离 | Rust 事件层每次请求绑定本地回环 HTTP 监听器（`127.0.0.1:0`），通过块级常量 `const __LX_PARAMS = { ... };` 前置注入脚本参数（避免守护进程内 `process.env` 跨轮污染），脚本通过 `fetch(STREAM_URL)` 实时推送 `@@LX_AI_WEB@@` 行协议。 |
| D4 | 登录态检测与未登录跳转 | 复用 `ego-browser` 任务空间（`lx-tui-ai-web-deepseek`）。若检测到未登录，自动导航至 `https://chat.deepseek.com/sign_in` 并调用 `handOffTaskSpace` 交还用户登录，同时在 `lx` 窗格内弹出英文 Toast 提示。 |
| D5 | 云端会话管理与 1 步 `/rewind` | `AI Web` 下开放 `/new`（`/clear`）、`/resume`（`/session`）、`/rename`（`/title`）与 `/rewind`（`/undo` 及空闲双击 `Esc`），全部与 DeepSeek 网页端云同步（会话列表、历史恢复、标题修改、会话删除、以及通过 `edit_user_index` 原地编辑上一轮用户消息重发）。 |
| D6 | 加载态与全局空态居中 | `/resume` 会话列表加载中居中显示 `loading sessions…`，恢复历史对话期间在聊天视口正中央显示 `loading history…`；所有面板、弹窗、列表的空状态与加载状态文案统一居中渲染（输入框占位提示保持左对齐）。 |
| D7 | 文件/图片上传与禁用文件夹/`refDir` | `AI Web` 下仅允许上传文件与图片，**禁止发送文件夹且禁用 `refDir` 相关功能**：`@` 面板仅展示 `Files` 单一 Tab（在 `Files` 中确认目录会自动进入该目录挑选文件，不插入 `@dir/`），快捷引用与路径粘贴面板不提供文件夹引用。发送与 `/rewind` 重发时通过 CDP `DOM.setFileInputFiles` 同步网页端附件卡片，支持按 `keepFiles` 精准保留或移除云端已有附件卡片。 |

---

## 2. 模块结构与变更文件清单

严格遵守 `app/`（纯状态与更新）、`event/`（IO 与子进程流调度）、`ui/`（纯只读渲染）与 `chat/`（协议、预处理与脚本定义）模块边界，所有生产代码单文件均 `< 1000` 行，测试代码分离至同层 `tests.rs`：

```text
src/
  app/
    effects.rs                  Effect::StartAiWebStream { key, site, prompt, files, keep_files, edit_user_index } 与 ResetAiWebSession
    state/
      lx.rs                     LxEntry::AiWeb、AiWebSite::Deepseek；Lx 新增 ai_web_site、ai_web_edit_user_index、ai_web_edit_cloud_files
      lx/chat.rs                LxChat.loading_session 加载态；is_ai_web_session 识别
      lx/mention.rs             switch_tab_for_entry / set_tab_for_entry / force_tab / active_is_directory（AI Web 下仅保留 Files Tab）
      lx/panels.rs              AI Web 下确认目录转为进入目录；路径粘贴在含目录时禁用 Reference 改写
      lx/refer.rs               AI Web 下快捷引用面板过滤掉文件夹候选
      lx/slash.rs               HistoryPanel.loading；command_entries_for 在 AI Web 下开放 /new、/resume、/rename、/rewind
    update/
      pane.rs                   set_lx_entry 在 AI Web 与其它入口互切时清理不兼容会话状态并强制归位 MentionTab::Files；新增 set_ai_web_site
      resume_edit.rs            /resume 编辑模态支持 AI Web 会话改名与删除（删除活跃会话时先停止在途流）
      chat/
        mod.rs                  try_send 优先分流 LxEntry::AiWeb（并在 AI Web 下屏蔽 ! Shell）；on_stream_finished 结算内存态会话
        ai_web.rs               AI Web 发送、/new、/resume、/rename、1 步 /rewind、云端历史加载与附件标记合成
        ai_web/tests.rs         AI Web 状态机、命令、上传、/rewind 与入口隔离单元测试
        rewind.rs               空闲双击 Esc 在 AI Web 下路由至 rewind_ai_web
        session.rs              new_session、resume_session、rename_current_session、on_session_deleted 适配 AI Web
        slash.rs                apply_slash_command 路由 AI Web 支持的四个命令并拦截其余本地命令
  chat/
    mod.rs                      导出 ai_web 与 prepare::ai_web
    prepare.rs                  通用引用解析与剥离；委托 prepare/ai_web.rs 处理 AI Web 消息准备
    prepare/
      ai_web.rs                 prepare_ai_web_message：解析本地文件/图片与保留的云端附件名 keep_files，拦截目录/Skill/MCP
      tests.rs                  补充 prepare_ai_web_message 单元测试
    ai_web/
      mod.rs                    嵌入脚本常量、AiWebSessionSummary / AiWebHistoryMessage / AiWebStreamEvent 与 NDJSON 解析
      tests.rs                  协议行解析单元测试
      deepseek_helpers.js       DeepSeek 网页驱动辅助函数：输出发射、新对话切换、CDP 附件上传与卡片清理、会话列表/加载/改名/删除
      deepseek_script.js        DeepSeek 网页对话主流程：SSE/XHR/fetch 流采集、原地编辑（edit_message）与底部输入框提交
  event/
    mod.rs                      AppEvent::AiWeb(AiWebEvent)
    mouse/press.rs              欢迎页 ai_web_site_hit 鼠标左键点击切换
    chat/
      mod.rs                    分派 AI Web 效应与网络事件
      ai_web.rs                 管理 ego-browser nodejs 子进程、30s 查询超时、本地回环 HTTP 流监听器与 const __LX_PARAMS 脚本拼接
  ui/
    panes.rs                    窗格左上角标题统一为 lx-<entry>（无方括号）
    text.rs                     收短入口标签（Native/Hybrid/Automation/AI Web）、新增加载态与 AI Web 提示文案
    text/chat.rs                新增 file_message_preview 与 folder_message_preview
    style.rs                    lx_entry_gradient 支持 LxEntry::AiWeb
    main_content/
      lx.rs                     4 入口动态间距、AI Web 站点行命中与渲染、loading_session 居中绘制（支持窄窗格截断）
      lx/brand.rs               icon_pixel 支持 LxEntry::AiWeb；draw_ai_web_sites 渲染站点行
      lx/mention.rs             AI Web 下顶栏仅展示 Files 单一 Tab
      lx/slash.rs               HistoryPanel.loading 居中渲染 loading sessions…
      lx/history/snapshot.rs    用户消息气泡剥离引用标记、渲染附件树行，区分图片/文件夹/文件空正文预览
      lx/history/cache/signature.rs 思考块在流式与完成态下默认折叠，手动展开可跨完成态保持
```

---

## 3. 核心设计详解

### 3.1 状态模型与入口隔离（`src/app/state/lx.rs`、`src/app/update/pane.rs`）

1. **`LxEntry` 四入口与短标签**：
   - `LxEntry::ALL = [Native, Hybrid, Automation, AiWeb]`，标签依次为 `"Native"`、`"Hybrid"`、`"Automation"`、`"AI Web"`。
   - 窗格左上角边框标题统一由 `format!("lx-{}", text::lx_entry_label(entry))` 生成（如 `lx-Native`、`lx-AI Web`）。
2. **`Lx` 新增 AI Web 状态字段**：
   - `pub ai_web_site: AiWebSite`（默认 `AiWebSite::Deepseek`）；
   - `pub ai_web_edit_user_index: Option<usize>`：记录 `/rewind` 后下一轮发送需要在网页端原地编辑的用户消息索引（0 基）；
   - `pub ai_web_edit_cloud_files: Vec<String>`：记录 `/rewind` 的轮次上原有的云端附件文件名（供重发时比对保留或移除网页端旧卡片）。
3. **跨入口会话严格隔离**：
   - 会话 ID 前缀：新建未绑定的本地预览会话使用 `ai-web-<pane>-<gen>`，绑定 DeepSeek 云端会话 UUID 后使用 `ai-web-ds-<uuid>`。
   - 当通过 `set_lx_entry` 在 `LxEntry::AiWeb` 与其它入口之间切换时：若存在在途流则先停止流，若当前持有的 `chat.session_id` 与目标入口不兼容则自动执行 `reset_session_view()` 并清空 `ai_web_edit_user_index`、`ai_web_edit_cloud_files` 与 `loading_session`，同时强制将 `@` 面板 Tab 重置为 `MentionTab::Files` 并退出 `Shell` 输入模式。

---

### 3.2 实时流协议：本地回环 HTTP 监听 + `const __LX_PARAMS`（`src/event/chat/ai_web.rs`）

1. **块级参数注入（防跨轮污染）**：
   - `ego-browser nodejs` 在同一任务空间守护进程内执行脚本时，修改 `process.env` 会在后续轮次残留。因此 `script_source` 每次生成脚本时在顶部拼接块级 `const __LX_PARAMS = { action, prompt, files, keepFiles, firstTurn, streamUrl, sessionId, title, editIndex };`，随后拼接 `deepseek_helpers.js` 与 `deepseek_script.js`。
2. **本地回环 HTTP 实时流 + stdout/stderr 双重回退**：
   - 启动 `chat` 流前，Rust 绑定 `TcpListener::bind("127.0.0.1:0")` 并传入 `streamUrl`。
   - 脚本在浏览器页面内挂载 `XMLHttpRequest` 与 `window.fetch` 拦截器（监听 `/api/v0/chat/completion`、`/api/v0/chat/edit_message`、`/api/v0/chat/regenerate`）及 DOM 兜底观察器，每 60ms 将新增的 `thinking` / `text` 增量通过 `fetch(STREAM_URL, { method: 'POST', body })` 实时推回 Rust，彻底绕过 `ego-browser` 子进程 stdio 缓冲。
   - 仅当收到有效 SSE `data:` 帧（或 `response.ok` 的流式响应）时才置 `sseActive = true`；仅当收到显式 `AiWebStreamEvent::Done` 时才按 `StopReason::Complete` 结算。
3. **思考块默认折叠**：
   - `PartKind::Reasoning` 在流式生成期间与生成完成后均默认折叠（生成中显示 `● Thinking`，完成后显示 `○ Thought · Xs`），用户点击可随时展开/折叠。

---

### 3.3 云端会话管理（`/new`、`/resume`、`/rename`、`/rewind`）与居中加载态

1. **`/new`（`/clear`）**：
   - 清空当前窗格内存会话视图、重置 `ai_web_edit_user_index` 与 `ai_web_edit_cloud_files`，并后台执行 `action: "new"` 让 DeepSeek 网页开启新对话。
2. **`/resume`（`/session`）与居中加载态**：
   - 打开 `HistoryPanel { loading: true, .. }`（居中渲染 `loading sessions…`）并触发 `action: "list_sessions"` 调用 `/api/v0/chat_session/fetch_page`。
   - 选中历史会话后，设置 `lx.chat.loading_session = true`（在聊天视口正中央渲染 `loading history…`）并触发 `action: "load_session"` 调用 `/api/v0/chat/history_messages`，按 `parent_id` 链表还原全部历史轮次与附件列表。
   - 在 `/resume` 面板中按 `Shift+Enter` 打开编辑模态：支持修改会话标题（同步调用 `/api/v0/chat_session/update_title`）或按 `Ctrl+D` 删除会话（同步调用 `/api/v0/chat_session/delete`，若删除当前活跃会话则先停止在途流并解绑）。
   - 所有后台查询均受 30 秒超时保护，且若未收到预期终态事件必定回投 `ActionFailed` 清除 `loading` 状态。
3. **1 步 `/rewind`（`/undo` / 空闲双击 `Esc`）**：
   - 截断最新一轮问答，将该轮用户原始草稿（含 `@[refer-*]` 标记）回填至输入框，并记录 `ai_web_edit_user_index = Some(remaining_users)`；
   - 下一轮发送时，脚本在网页端定位第 `editIndex` 条用户消息，点击编辑按钮打开 `textarea[name="user query"]` 原地修改并提交（若打开或提交编辑框失败则显式报错，绝不静默退化为底部发新消息）。

---

### 3.4 文件与图片上传、禁用 `refDir`/文件夹及 `/rewind` 附件同步

1. **禁用 `refDir` 与文件夹引用**：
   - `AI Web` 模式下只能发送文件与图片，不能发送文件夹，也不能使用 `refDir` 功能：
     - `@` 提及面板在 `AI Web` 下仅展示 `Files` 单一 Tab（隐藏 `refDir`、`Skills`、`MCP`），禁用左右切 Tab；
     - 在 `@` 面板 `Files` Tab 中对目录条目按 `Enter`/`Tab` 或鼠标点击时，自动转为进入该目录（`mention_enter_folder`），不插入 `@dir/` 目录引用；
     - 字母快捷面板（`refer.rs`）在 `AI Web` 下过滤掉所有目录候选，终端路径粘贴面板（`paste_open`）在候选包含目录时禁用 `Reference` 改写；
     - 发送时 `prepare_ai_web_message` 拦截任何手写的 `@[refer-folder]`、`@dir/`、`@[refer-skill]`、`@[refer-mcp]` 并给出英文提示（保留草稿）。
2. **气泡附件树渲染与网页文本清洗**：
   - `prepare_ai_web_message` 解析草稿中的 `@[refer-image](...)`、`@[refer-file](...)`、`@相对路径`、`@[ref-content]/...` 与 `@ref/...`，生成 `ChatMessageView.attachments`（在气泡下方渲染 `├─` / `└─` 附件树行，无正文时按类型显示 `[Image]` / `[File]` 预览），同时剥离引用标记、展开折叠粘贴块得到发往网页输入框的 `web_prompt` 与去重本地绝对路径列表 `files`。
   - `/resume` 加载云端历史消息时，为带 `files` 的用户消息自动合成 `@[refer-image](name)` / `@[refer-file](name)` 标记与 `marker_start`/`marker_end`（气泡展示时自动剥离标记，`/rewind` 时则完整回填输入框）。
3. **CDP 文件注入与 `/rewind` 附件精准保留/移除**：
   - 在普通发送或 `/rewind` 原地编辑重发时，`syncContainerAttachments` 先扫描目标容器内的 `.ds-animated-size-item` 卡片：
     - 若为原地编辑且存在 `keepFiles`（用户在输入框中保留未删的云端历史附件名），则按文件名 1:1 保留对应卡片，仅点击关闭被用户删掉的卡片，并轮询等待旧卡片数降至 `keptCount`；
     - 随后通过 CDP `DOM.setFileInputFiles` 注入本次新增的本地文件列表 `FILES`，轮询等待卡片总数达到 `keptCount + FILES.length` 且全部完成上传/解析（检测状态前先剔除已知文件名，避免 `error.rs` 等文件名误触发错误正则），最后触发发送。

---

## 4. 风险与边界分析

1. **`ego-browser` 未安装或守护进程异常**：
   - 子进程启动失败或 30 秒查询超时时，自动终止子进程并回投英文提示 `ego-browser is not installed or unavailable` 或失败 Toast，不阻塞 TUI 事件循环。
2. **DeepSeek 网页端未登录或文件被拒**：
   - 未登录时自动打开登录页并 `handOffTaskSpace`，弹出 `Please sign in to DeepSeek in ego-browser`；网页端拒绝文件（格式不支持或超大）时捕获错误并在 `lx` 窗格内弹出红色 Toast。
3. **与 `Native` 模式完全隔离**：
   - 切换入口时自动清理跨入口不兼容的会话状态與输入模式，确保两者的状态栏、斜杠命令、提及面板与本地 SQLite 互不污染。
