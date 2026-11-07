# lx `AI Web` 入口与 DeepSeek 网页端接入（基于 `ego-browser`）— 设计文档

> 状态：**已在 `.worktrees/lx-ai-web`（`feature/lx-ai-web`）完成实现与测试，待用户确认合并**（2026-11-06）
> 执行任务清单见 [task.md](task.md)。

---

## 0. 现状核实（写设计前已逐项确认）

| 事实 | 现有代码位置 | 对本设计的影响 |
|---|---|---|
| `LxEntry` 目前仅包含 `Native`、`Hybrid`、`Automation` 三项，`LxEntry::ALL` 固定长度 3 | `src/app/state/lx.rs:64-75` | 新增第 4 个同行入口 `LxEntry::AiWeb`（界面标签 `AI Web`，窗格标题 `lx-[AI Web]`） |
| `ui::main_content::lx::layout` 与 `place_entries` 中入口间距计算硬编码了 `.saturating_mul(2)`（假定 3 个入口、2 个间隔），竖排高度硬编码了 `.saturating_mul(3)` | `src/ui/main_content/lx.rs:379-390, 465-469` | 必须改为按 `labels.len()` 与 `labels.len().saturating_sub(1)` 动态计算横排总宽与竖排总高，确保 4 个入口居中与折行准确 |
| 欢迎页入口标签下方固定有 1 行 `lay.inventory` 槽位，当前无条件调用 `brand::draw_inventory` 渲染 `{agents_md} AGENTS.md · {mcp} mcp · {skills} skills` | `src/ui/main_content/lx.rs:442-450, 515-517`、`src/ui/main_content/lx/brand.rs:140-190` | 当 `lx.entry == LxEntry::AiWeb` 时，复用 `lay.inventory` 几何槽位改为渲染可点击的 AI Web 站点选择器（首期 `DeepSeek`），不再展示 `AGENTS.md / mcp / skills` |
| 窗格底边框状态栏 `status::render` 从 `lx.chat.status` 读取 `model_name`、`variant`、`context_limit`、`input_tokens` 与后台任务数 | `src/ui/main_content/lx/status.rs:70-185`、`src/app/state/lx/chat.rs:920-975` | 当 `lx.entry == LxEntry::AiWeb` 时，`refresh_lx_status` 将 `status.model_name` 设为当前选中站点名（`"DeepSeek"`），并将 `variant`、`context_limit`、`input_tokens` 置 `None`，底边框仅显示站点名与流状态 |
| `update::chat::try_send` 在 `entry != LxEntry::Native` 时一律拦截并弹出 `Native Agent only` | `src/app/update/chat/mod.rs:159-168` | 为 `LxEntry::AiWeb` 开辟独立的纯内存提交与流式分支 `try_send_ai_web`：不校验本地 API 模型与工作区路径、不注入 `lx` 系统提示词与本地工具、不写 SQLite |
| 本机 `ego-browser`（`/Users/yonah/.local/bin/ego-browser`）支持 `ego-browser nodejs` 标准输出实时行流刷新，`useOrCreateTaskSpace` 自动继承用户登录态，实测 `https://chat.deepseek.com/` 已具合法 `localStorage.userToken` 与输入框 `textarea[name="search"]` | 本机实测验证 | Rust 事件层直接通过 `tokio::process::Command` 启动 `ego-browser nodejs` 执行内置确定性脚本，拦截网页 SSE/DOM 流并以 NDJSON 行协议回传 `ChatDelta` |

---

## 1. 已确认决策汇总（Grill Me 收口）

| 编号 | 决策维度 | 确认结论 |
|---|---|---|
| D1 | 技术路线选择 | 采用**路线 B（真实浏览器流拦截 + `lx-tui` 原生字符界面渲染）**。不在终端走图形协议画网页像素，100% 复用 `PaneView::Lx` 的流式 Markdown 与 `Thinking` 思考折叠块。 |
| D2 | 入口定位与上下文边界 | 在 `LxEntry` 新增第 4 个同行入口 `AiWeb`（英文文案 `AI Web`）。进入后仅做**纯网页对话**，不加载 `lx` 系统提示词、`AGENTS.md`、MCP、Skills 与本地文件/终端工具。 |
| D3 | 底层通信实现 | **零 Agent 确定性脚本**：内置在 `lx-tui` Rust 进程中，通过子进程调用本机 `ego-browser nodejs` 驱动 `https://chat.deepseek.com`，注入 `fetch`/`XHR` 拦截器与 DOM 观察器实时输出 NDJSON 行流。 |
| D4 | 登录态检测与未登录跳转 | 复用 `ego-browser` 任务空间（`lx-tui-ai-web-deepseek`）。若检测到未登录（无 `localStorage.userToken` 或路径为 `/sign_in`），自动导航至 `https://chat.deepseek.com/sign_in` 并调用 `handOffTaskSpace` 交还用户在 `ego-browser` 窗口登录，同时在 `lx` 窗格内弹出英文 Toast 提示。 |
| D5 | 欢迎页与状态栏展示 | 切到 `AI Web` 时，欢迎页入口下方原本显示 `AGENTS.md · mcp · skills` 的行改为展示可点击切换的站点项（首期 `DeepSeek`）；窗格底边框状态栏显示当前站点名 `DeepSeek`，隐藏 API Token 占用百分比。 |
| D6 | 会话生命周期与命令裁剪 | `AI Web` 采用**窗格内存态会话**（多轮上下文由 DeepSeek 网页端维护），**不写入本地 SQLite**（零数据库 Schema 变更）。`/new` 清空当前窗格消息并在网页端开启新对话；在 `AI Web` 下屏蔽仅对本地 DB/Agent 生效的 `/model`、`/resume`、`/fork`、`/rewind`、`/compact`、`/subagents`、`/rename`、`/connect`、`/mcp`（命令面板在 `AI Web` 下仅保留 `/new`）。 |

---

## 2. 模块结构与变更文件清单

严格遵守 `app/`（纯状态与更新）、`event/`（IO 与子进程流调度）、`ui/`（纯只读渲染）与 `chat/`（协议与脚本定义）模块边界，单文件均控制在 1000 行以内：

```text
src/
  app/
    effects.rs                  Effect 新增 StartAiWebStream { key, site, prompt } 與 ResetAiWebSession { site }
    state/
      lx.rs                     LxEntry 新增 AiWeb；新增 AiWebSite 枚举（首期 Deepseek）；Lx 新增 ai_web_site: AiWebSite
      lx/slash.rs               SlashPanel::open_commands 按 LxEntry 过滤可用命令（AiWeb 下仅暴露 /new）
    update/
      pane.rs                   set_lx_entry 切换入口时同步刷新底边框状态栏；新增 set_ai_web_site
      chat/
        mod.rs                  try_send 新增 LxEntry::AiWeb 分支；on_stream_finished 在 AiWeb 内存会话下直接回 Idle（不入队 FinishChatTurn DB 写效应）；refresh_lx_status 按 entry 区分展示
        ai_web.rs               新增：AI Web 纯内存提交、/new 网页会话重置、未登录 Toast 提示与状态行同步
        session.rs              new_session 在 LxEntry::AiWeb（或存在 AI Web 会话）时入队 Effect::ResetAiWebSession
        slash.rs                apply_slash_command 在 LxEntry::AiWeb 下拦截不支持的 Agent 专属斜杠命令并给出英文提示
  ui/
    text.rs                     新增 LX_ENTRY_AI_WEB ("AI Web")、LX_ENTRY_AI_WEB_DESC、AI_WEB_SITE_DEEPSEEK ("DeepSeek") 及 AI Web 提示文案
    style.rs                    lx_entry_gradient 支持 LxEntry::AiWeb（冰蓝→主粉渐变，与前三个入口互不相同）
    main_content/
      lx.rs                     修复 place_entries / layout 多入口动态间距计算；新增 ai_web_site_hit 命中测试；build 在 AiWeb 下绘制站点选择行
      lx/brand.rs               icon_pixel 支持 LxEntry::AiWeb；新增 draw_ai_web_sites 渲染可点击站点标签与选中下划线
      lx/tests.rs               更新入口数量断言（3 -> 4），新增 AI Web 欢迎页渲染、站点命中与底边框状态单测
  chat/
    mod.rs                      导出 pub mod ai_web
    ai_web/
      mod.rs                    AI Web 站点脚本生成、NDJSON 行协议解析（thinking / text / login_required / error / done）与单元测试
      deepseek_script.js        嵌入式的 ego-browser DeepSeek 驱动与流拦截脚本（经 include_str! 编译进二进制）
  event/
    mouse/
      press.rs                  欢迎页鼠标左键点击新增 ai_web_site_hit 分支
    chat/
      mod.rs                    处理 Effect::StartAiWebStream 与 Effect::ResetAiWebSession，管理 ego-browser 子进程流与取消
```

---

## 3. 核心设计详解

### 3.1 状态模型扩展（`src/app/state/lx.rs`）

1. **`LxEntry` 扩展**：
   ```rust
   /// lx 页四个入口：原生 agent、混合 agent、自动化 agent、AI Web。
   #[derive(Debug, Clone, Copy, PartialEq, Eq)]
   pub enum LxEntry {
       Native,
       Hybrid,
       Automation,
       AiWeb,
   }

   impl LxEntry {
       /// 四个入口的显示顺序。
       pub const ALL: [LxEntry; 4] = [
           LxEntry::Native,
           LxEntry::Hybrid,
           LxEntry::Automation,
           LxEntry::AiWeb,
       ];
   }
   ```
2. **`AiWebSite` 枚举**：
   ```rust
   /// AI Web 支持的目标站点（首期接入 DeepSeek，后续可扩展 ChatGPT / Gemini / Grok）。
   #[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
   pub enum AiWebSite {
       #[default]
       Deepseek,
   }

   impl AiWebSite {
       pub const ALL: [AiWebSite; 1] = [AiWebSite::Deepseek];
   }
   ```
3. **`Lx` 结构体新增字段**：
   - `pub ai_web_site: AiWebSite`（默认 `AiWebSite::Deepseek`）。
4. **入口切换隔离（`src/app/update/pane.rs`）**：
   - `set_lx_entry` 在切换 `pane.lx.entry` 时调用 `crate::app::update::chat::refresh_lx_status(state, id)`，使窗格底边框立即切换为 `DeepSeek`（`AiWeb`）或恢复 API 模型名（`Native`）。

---

### 3.2 欢迎页与底边框 UI 渲染（`src/ui/`）

1. **文案定义（`src/ui/text.rs`）**：
   - `pub const LX_ENTRY_AI_WEB: &str = "AI Web";`
   - `pub const LX_ENTRY_AI_WEB_DESC: &str = "chat directly with web AI models";`
   - `pub const AI_WEB_SITE_DEEPSEEK: &str = "DeepSeek";`
   - `pub const LX_AI_WEB_LOGIN_REQUIRED: &str = "Please sign in to DeepSeek in ego-browser";`
   - `pub const LX_AI_WEB_EGO_UNAVAILABLE: &str = "ego-browser is not installed or unavailable";`
   - `pub const LX_AI_WEB_ONLY_NEW_CMD: &str = "Only /new is available in AI Web";`
2. **调色板与渐变（`src/ui/style.rs` & `src/ui/main_content/lx/brand.rs`）**：
   - `lx_entry_gradient(LxEntry::AiWeb)` 返回 `(LX_SKY_BLUE, LX_PINK)`（与 `Native` 的 `(LX_PINK, LX_SKY_BLUE)`、`Hybrid` 的 `(168, 195)`、`Automation` 的 `(195, 168)` 形成互异的四组端点，满足 `entry_label_gradients_are_distinct`）。
   - `brand::icon_pixel(LxEntry::AiWeb, ch)` 对调主色槽位（`'p' -> 'b'`, `'b' -> 'p'`, `'n' -> 'c'`, `'c' -> 'n'`），保持 LX 像素图标形状不变且四入口配色各异。
3. **动态入口布局与站点选择行（`src/ui/main_content/lx.rs` & `brand.rs`）**：
   - 修复 `layout` 与 `place_entries` 中硬编码的 `2` 个間隔与 `3` 行高度，改为基于 `labels.len()` 动态计算。
   - 当 `lx.entry == LxEntry::AiWeb` 时，`lay.inventory` 所在行调用 `brand::draw_ai_web_sites`：居中绘制 `AiWebSite::ALL`（首期为 `DeepSeek`），选中站点使用强调色加粗，并提供 `ai_web_site_hit(area, lx, column, row) -> Option<AiWebSite>` 供鼠标点击切换。
4. **底边框状态栏（`src/app/update/chat/mod.rs`）**：
   - 在 `refresh_lx_status` 中：若 `target.lx.entry == LxEntry::AiWeb`，将 `target.lx.chat.status` 设为：
     - `model_name = Some(text::ai_web_site_label(target.lx.ai_web_site).to_string())`（即 `"DeepSeek"`）
     - `variant = None`、`context_limit = None`、`input_tokens = None`
     - `streaming = target.lx.chat.request.is_busy()`

---

### 3.3 纯内存会话生命周期与斜杠命令裁剪（`src/app/update/chat/`）

1. **纯内存提交（`src/app/update/chat/ai_web.rs`）**：
   - 在 `LxEntry::AiWeb` 下按 `Enter` 发送时：
     - 不检查 `state.models` 是否有 API Key，不检查工作区路径；
     - 若 `target.lx.chat.session_id.is_none()`，分配内存会话标识 `format!("ai-web-{}", generation)`；
     - 直接在内存中向 `target.lx.chat.messages` 推入 `user` 消息与 `streaming` 状态的 `assistant` 占位消息（`model_label: Some("DeepSeek".into())`），将 `target.lx.chat.request` 置为 `RequestState::Streaming { key, assistant_id }`；
     - 入队 `Effect::StartAiWebStream { key, site: target.lx.ai_web_site, prompt: draft }`。
2. **流增量与终态结算**：
   - 流式增量完全复用现有的 `on_stream_delta(state, &key, &deltas)`：`PartKind::Reasoning` 自动渲染为可折叠的 `Thinking` 块，`PartKind::Text` 自动渲染为 GitHub Dark 语法高亮的 Markdown 正文！
   - 在 `on_stream_finished` 中：若 `key.session.starts_with("ai-web-")`（或当前为 `AiWeb` 内存会话），关闭流并更新消息终态与耗时后，**直接将 `target.lx.chat.request` 置回 `RequestState::Idle`**，不入队 `Effect::FinishChatTurn`（不写 SQLite）。
3. **`/new` 与命令过滤**：
   - `SlashPanel::open_commands` 在 `LxEntry::AiWeb` 下仅列出 `SlashCommand::New`；
   - 手动输入 `/model`、`/resume` 等其余内置命令时提示 `Only /new is available in AI Web`；
   - 在 `LxEntry::AiWeb` 下执行 `/new` 时，除清空窗格内存消息回到欢迎页外，入队 `Effect::ResetAiWebSession { site }` 通知后台在 `ego-browser` 的 DeepSeek 页面开启新对话。

---

### 3.4 `ego-browser` DeepSeek 驱动与流式拦截协议（`src/chat/ai_web/` & `src/event/chat/`）

1. **进程通信协议（NDJSON over Stdout）**：
   - Rust 事件层通过 `tokio::process::Command` 启动 `ego-browser nodejs`（优先查找 `PATH` 中的 `ego-browser`，回退 `~/.local/bin/ego-browser`），通过环境变量 `LX_AI_WEB_ACTION`（`"chat"` 或 `"new"`）与 `LX_AI_WEB_PROMPT` 传入参数，脚本源码由 `include_str!("deepseek_script.js")` 经 stdin 喂入。
   - 脚本向 stdout 输出以 `@@LX_AI_WEB@@` 为前缀的单行 JSON 事件（过滤掉 `ego-browser` 自身的 `[ego-browser:notice]` 提示）：
     - `{"event":"login_required","url":"https://chat.deepseek.com/sign_in"}`：检测到未登录，已跳转登录页并调用 `handOffTaskSpace`；
     - `{"event":"thinking","delta":"..."}`：深度思考增量文本；
     - `{"event":"text","delta":"..."}`：回答正文增量文本；
     - `{"event":"done"}`：回答生成完毕；
     - `{"event":"error","message":"..."}`：页面异常或被用户接管。
2. **DeepSeek 页面登录检测与新对话触发（`deepseek_script.js`）**：
   - 调用 `useOrCreateTaskSpace('lx-tui-ai-web-deepseek')` 与 `openOrReuseTab('https://chat.deepseek.com', { wait: true, timeout: 20 })`。
   - **未登录判定**：检查页面 `location.pathname.startsWith('/sign_in')` 或 `!localStorage.getItem('userToken')`。若未登录，导航到 `https://chat.deepseek.com/sign_in`，调用 `await handOffTaskSpace()` 把浏览器窗口交还用户，输出 `login_required` 事件后退出。
   - **开启新对话（`action == "new"`）**：点击页面“开启新对话”按钮或导航回 `https://chat.deepseek.com/`，输出 `done` 后退出。
3. **双保险流式提取（SSE Hook + DOM 增量同步）**：
   - 在发送消息前，在页面内安装 `window.__lxDeepSeekHook`：
     - **通道 1（网络流拦截）**：包装 `window.fetch` 与 `XMLHttpRequest.prototype.open/send`，拦截 `/api/v0/chat/completion` 的 `text/event-stream` 响应流，解析 DeepSeek 的 JSON Patch / SSE `v` 字段（区分 `thinking_content` 与 `content` 分片）推入队列；
     - **通道 2（DOM 增量兜底）**：同时采样最新一条 `.ds-message` 中的思考容器（`.ds-think-content`）与正文容器（`.ds-markdown`）文本增量，若网络流因页面内部封装变化未命中，自动由 DOM 差量补齐，确保 100% 稳定拿到流式回复。
   - 填入 `textarea[name="search"]`（或页面主 `textarea`）并触发回车/点击发送按钮，主循环每 80ms 轮询页面队列并实时通过 `process.stdout.write` 刷出 `@@LX_AI_WEB@@` JSON 行，直到生成结束（发送区按钮恢复空闲态且流结束）。
4. **取消支持（双击 `Esc`）**：
   - 用户在流式生成中双击 `Esc` 触发 `Effect::CancelChatStream { key }` 时，事件层触发取消通道，直接 `kill()` 在途的 `ego-browser nodejs` 子进程，并异步发送一次停止生成点击，窗格状态立即结转为 `interrupted`。

---

## 4. 风险与边界分析

1. **`ego-browser` 未安装或未启动**：
   - 当本机找不到 `ego-browser` 二进制或子进程启动失败时，事件层捕获错误并在当前 `lx` 窗格内弹出红色 Toast（`ego-browser is not installed or unavailable`），消息标为 `failed`，绝不 panic 或阻塞 TUI 事件循环。
2. **用户在 `ego-browser` 窗口手动接管控制权**：
   - 当用户在 `ego-browser` GUI 中点击接管页面时，`ego-browser` 抛出 `user is controlling` 错误，脚本捕获后映射为友好提示并结束当前轮次，遵守 `ego-browser` 不强抢控制权的规范。
3. **与 `Native Agent` 完全隔离**：
   - 若窗格在 `Native Agent` 下已有会话，用户在欢迎页（空白态）切到 `AI Web` 聊天，或通过 `/new` 重置后切回 `Native Agent`，两者的状态栏、斜杠命令与数据库互不污染。
4. **`AI Web` 图片与文件上传及 `/rewind` 附件撤销同步**：
   - **入口复用与 `@` 面板裁剪**：复用 `@` 提及（`Files` 工作区文件、`refDir` 引用文件夹）与终端拖拽/粘贴路径机制；在 `LxEntry::AiWeb` 下 `@` 面板仅展示 `Files` 与 `refDir` 两个 Tab（隐藏 `Skills` 与 `MCP`），且发送时拦截目录/本地 Skill 引用并提示仅支持上传单个文件或图片。
   - **消息展示与网页文本清洗**：TUI 侧复用 `ChatMessageView.attachments`（自动剥离正文中的 `@[refer-*](...)` / `@path` 原始标记，并在用户消息下方渲染 `├─` / `└─` 附件树行，同时保留原始草稿供 `/rewind` 退回输入框）；发往网页 `textarea` 的文本剥离引用标记并展开折叠粘贴块；`/resume` 恢复云端会话时同步映射云端历史消息附带的文件名。
   - **CDP 文件注入与 `/rewind` 附件撤销**：通过 `ego-browser` 的 CDP `DOM.setFileInputFiles` 将本地文件绝对路径直接注入 DeepSeek 网页的 `<input type="file" multiple>`，等待 `.ds-animated-size-item` 上传/解析就绪后再触发发送；在执行 `/rewind` 后重发（原地编辑 `edit_user_index`）或普通发送前，脚本先清空目标输入容器内已有的旧 `.ds-animated-size-item` 附件卡片，再按本次草稿保留的附件列表重新注入，确保在 TUI 中删改图片/文件后网页端同步撤下且不重复上传。
