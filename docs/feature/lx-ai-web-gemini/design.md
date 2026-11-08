# `Web` 接入 Google Gemini 网页端、双站点品牌渐变与 `/model` 模型/思考等级切换 — 详细设计 (`design.md`)

- **特性目录**：`docs/feature/lx-ai-web-gemini/`
- **关联任务清单**：[`task.md`](task.md)
- **参考驱动规范**：[`docs/standards/web-deepseek.md`](../../standards/web-deepseek.md) · [`docs/feature/lx-ai-web/design.md`](../lx-ai-web/design.md)
- **当前状态**：已完成实现、CLI 端到端验证与 review 修复（位于 `.worktrees/lx-ai-web-gemini` 工作区 `feat/lx-ai-web-gemini` 分支）；验证见 [`verification.md`](verification.md)，问题清单与修复见 [`review.md`](review.md)
- **目标范围**：
  - `src/app/state/lx/web.rs`（新增子模块：`AiWebSite::Gemini`、`GeminiWebModel`、`GeminiThinkingLevel`、`DeepseekThinkingMode`、`AiWebConfig` 及双站点会话前缀解析）
  - `src/app/state/lx.rs`（挂载 `ai_web_config: AiWebConfig`，重导出 `web::*`）
  - `src/app/state/lx/slash.rs`（`Web` 下开放 `/model` 命令并为 `Gemini` / `DeepSeek` 构造免 API Key 的内置 `ModelCandidate` 与思考等级列表）
  - `src/app/effects.rs`（`Effect::StartAiWebStream` 新增 `model: String` 与 `thinking: String` 字段；`Effect::ListChatSessions` 携带或由站点路由）
  - `src/app/update/pane.rs`（`set_ai_web_site` 跨站点切换时自动停止在途流、清理跨站点会话与 `/rewind` 编辑态并刷新状态栏）
  - `src/app/update/chat/web.rs`（支持 `ai-web-ds-<id>` 与 `ai-web-gm-<id>` 双前缀识别、`apply_ai_web_status` 展示站点模型名与黄色思考等级、`try_send_ai_web` 开放 `/model` 并向 `StartAiWebStream` 透传当前站点模型与思考等级）
  - `src/app/update/chat/slash.rs`（`open_model_picker`、`variant_keys`、`default_variant_index`、`finish_model_selection` 在 `LxEntry::AiWeb` 下读写内存态 `ai_web_config`，不写本地 `ai.json` / SQLite、不注入 `<system-update>`）
  - `src/chat/web/mod.rs`、`src/chat/web/gemini_helpers.js`、`src/chat/web/gemini_script.js`、`src/chat/web/deepseek_script.js`（新增 Gemini 网页驱动脚本，支持模型与思考等级切换、基础命令、流式思考与正文、文件上传及 `/rewind` 原地修改；DeepSeek 脚本支持按 `thinking` 参数开关 DeepThink）
  - `src/event/chat/web.rs`（按 `AiWebSite` 或会话 ID 前缀路由 `Deepseek` / `Gemini` 脚本与前缀，向 `__LX_PARAMS` 注入 `model` 与 `thinking`）
  - `src/ui/text.rs`、`src/ui/style.rs` & `src/ui/style/lx.rs`、`src/ui/main_content/lx.rs`、`src/ui/main_content/lx/brand.rs`（首页 `DeepSeek · Gemini` 并排渲染、专属品牌 256 色逐字渐变、选中加粗 + 底部同色渐变 `─` 下划线；拆分 `src/ui/style/lx.rs` 使 `src/ui/style.rs` 降至 1000 行以内）
- **架构约束**：
  - 严格保持 `app/`（纯状态与更新，禁止 IO）与 `ui/`（纯渲染只读状态，禁止修改状态或执行 IO）边界清晰；
  - 所有 TUI 用户可见文案一律使用英文并集中定义在 `src/ui/text.rs`；
  - 生产代码单文件行数上限 1000 行（`src/ui/style.rs` 现状 1168 行，本次将 `lx` 品牌渐变与会话胶囊配色抽入 `src/ui/style/lx.rs`），禁止 `unwrap()` / `expect()`。

---

## 0. 现状核实与事实基线（CodeGraph + Codebase Memory + `ego-browser` 实测）

| 维度 | 现状实测（带文件与行号） | 对本特性的设计影响 |
| --- | --- | --- |
| **站点枚举与首页站点行** | `src/app/state/lx.rs:83-93`：`AiWebSite` 仅含 `Deepseek`，`AiWebSite::ALL = [AiWebSite::Deepseek]`；`src/ui/main_content/lx/brand.rs:215-267`：`draw_ai_web_sites` 使用单色 `style::accent()` / `style::muted()` 渲染且无下划线。 | 1. `AiWebSite` 扩展为 `[Deepseek, Gemini]`；<br>2. `draw_ai_web_sites` 改为与上方 4 个入口（`draw_entries`，`brand.rs:289-329`）完全同构的「**常驻品牌逐字渐变 + 选中 `BOLD` + 选中正下方 `y + 1` 行同色渐变 `─` 下划线 + `·` 暗色分隔符**」；<br>3. `src/ui/main_content/lx.rs:463-469` 在 `LxEntry::AiWeb` 下为 `lay.inventory` 分配 `1 + UNDERLINE_ROWS`（2 行）高度以容纳选中下划线。 |
| **上方 4 模式渐变算法** | `src/ui/style.rs:1050-1124`：`lx_entry_gradient` 返回 `(Color::Indexed(a), Color::Indexed(b))`，由 `lerp_indexed` 在 256 色 cube（`16..=231`）内按 RGB 线性插值逐字符着色。 | 新增 `ai_web_site_gradient(site: AiWebSite) -> (Color, Color)` 与 `ai_web_site_label_style(site, position, length, selected) -> Style`，直接复用 `lerp_indexed`：<br>• `Deepseek`：`(Color::Indexed(63), Color::Indexed(39))`（深海钴蓝 `#5f5fff` → 电光青蓝 `#00afff`）；<br>• `Gemini`：`(Color::Indexed(69), Color::Indexed(204))`（星光蓝 `#5f87ff` → 紫罗兰 → 珊瑚粉 `#ff5f87`）。 |
| **`Web` 下 `/model` 与状态栏** | `src/app/state/lx/slash.rs:677-696` 与 `src/app/update/chat/slash.rs:418-437`：当前 `Web` 仅开放 `/new`、`/resume`、`/rewind`、`/rename`，拦截了 `/model`；`src/app/update/chat/web.rs:34-41` 将 `status.model_name` 写死为站点名且 `status.variant = None`。 | 1. 在 `Web` 下开放 `/model`（加入白名单，支持两级面板选择模型与思考等级）；<br>2. `apply_ai_web_status` 根据当前站点展示：<br>  - `Gemini`：`model_name = Some("Gemini · <Model>")`（如 `"Gemini · 3.8 Flash"`），`variant = Some("<level>")`（`"low"` / `"medium"` / `"high"`）；<br>  - `DeepSeek`：`model_name = Some("DeepSeek")`，开启深度思考时 `variant = Some("deepthink")`，关闭时 `variant = None`。 |
| **双站点会话前缀与事件分派** | 基线 `src/app/update/chat/ai_web.rs`（现已重命名为 `web.rs`）固定 `ai-web-ds-` 前缀；`src/event/chat/web.rs:232, 303, 366, 380, 795` 固定注入 `DEEPSEEK_HELPERS_SCRIPT` + `DEEPSEEK_SCRIPT`。 | 1. 区分双站点云端会话前缀：`Deepseek` 为 `"ai-web-ds-"`，`Gemini` 为 `"ai-web-gm-"`；新增 `ai_web_remote_session(session_id) -> Option<(AiWebSite, &str)>`；<br>2. `start_stream`、`reset_session`、`list_sessions` 按 `pane.lx.ai_web_site` 选择站点脚本；`load_session`、`rename_session`、`delete_session` 按 `session_id` 前缀自动识别所属站点并选择对应脚本。 |
| **Gemini 网页端真实 DOM 与安全策略（`ego-browser` 实测）** | 实测 `https://gemini.google.com/app`：<br>1. 模式选择器按钮：`[data-test-id="bard-mode-menu-button"]`，点开后一级 `menuitemradio` 含 `3.5 Flash-Lite`、`3.8 Flash`、`3.1 Pro`、`4 Argon`；底部 `menuitem` `"思考等级"` / `"Thinking level"` 展开二级菜单含 `低 (Low)`、`中 (Medium)`、`高 (High)`。<br>2. 页面开启了 **CSP TrustedHTML**（直接赋 `innerHTML` 会抛 `TypeError: This document requires 'TrustedHTML' assignment`），且首次发消息会调用 `Notification.requestPermission()` 触发浏览器权限弹窗抢走 `ego-browser` 控制权。<br>3. 输入框为 `.ql-editor[contenteditable="true"]`（位于 `rich-textarea` 内），用户消息为 `<user-query>`（含 `button[aria-label="修改"]`），助手消息为 `<model-response>`（含 `<thinking-overlay>` / `<model-thoughts>` 与 `<message-content>`），侧边栏历史链接为 `a[href*="/app/"]`。 | 1. `gemini_script.js` 发送前先注入 `Notification.requestPermission = () => Promise.resolve('denied')` 阻断权限弹窗；<br>2. 富文本编辑器统一通过 `focus()` + `document.execCommand('selectAll')` + `document.execCommand('insertText', false, PROMPT)` 写入文本，严禁直接赋值 `innerHTML`；<br>3. 双通道采集流式输出：拦截 `/_/BardChatUi/data/assistant.lamda.BardFrontendService/StreamGenerate` 的 XHR/fetch 流 + 60ms DOM 轮询 `<model-response>` 的思考块与 `<message-content>`。 |

---

## 1. 已确认的边界决策汇总（Grill Me 结论表）

| 编号 | 决策维度 | 已确认方案 |
| --- | --- | --- |
| **Q1** | **Gemini 模型切换与思考等级交互方式及范围** | 1. **直接复用 `/model` 两级选择面板**：在 `Web` 模式下开放 `/model` 命令，无需配置本地 API Key。<br>2. **`Gemini` 站点候选项**：第 1 级展示 4 个模型（`3.5 Flash-Lite`、`3.8 Flash`、`3.1 Pro`、`4 Argon`，默认 `3.8 Flash`）；回车进入第 2 级选择 3 档思考等级（`low`、`medium`、`high`，默认 `low`）。<br>3. **`DeepSeek` 站点候选项**：第 1 级为 `DeepSeek`，回车进入第 2 级选择思考开关（`deepthink`、`off`，默认 `deepthink`）。 |
| **Q2** | **首页 `DeepSeek · Gemini` 品牌渐变配色与选中视觉** | 1. **排列与结构**：在首页选中 `Web` 时，下方站点行并排显示 `DeepSeek · Gemini`。<br>2. **专属品牌 256 色渐变**：<br>  - `DeepSeek`：`Color::Indexed(63)`（深海钴蓝 `#5f5fff`）→ `Color::Indexed(39)`（电光青蓝 `#00afff`）；<br>  - `Gemini`：`Color::Indexed(69)`（星光蓝 `#5f87ff`）→ `Color::Indexed(204)`（珊瑚粉 `#ff5f87`，插值经过紫罗兰）。<br>3. **选中与未选中视觉对齐上方 4 模式**：未选中时保持各自品牌逐字渐变（常规字重、无下划线）；选中时**字体加粗（`BOLD`）并在正下方（`y + 1` 行）绘制同品牌逐字渐变的 `─` 下划线**。 |
| **Q3** | **双站点会话隔离、空间隔离与基础命令对齐** | 1. **独立任务空间与会话前缀**：<br>  - `DeepSeek`：任务空间 `lx-tui-ai-web-deepseek`，云端会话前缀 `ai-web-ds-<id>`；<br>  - `Gemini`：任务空间 `lx-tui-ai-web-gemini`，云端会话前缀 `ai-web-gm-<id>`。<br>2. **切换站点自动清场**：点击切换 `DeepSeek` / `Gemini` 时，若站点发生变化，立即停止在途流、重置当前窗格跨站点会话视图、清空 `/rewind` 待编辑状态并刷新状态栏。<br>3. **基础命令 1:1 复刻**：`Gemini` 完整支持 `/new`（`/clear`）、`/resume`（`/session`，含 `Shift+Enter` 重命名、`Ctrl+D` 删除）、`/rename`（`/title`）、`/rewind`（`/undo` 及空闲双击 `Esc` 原地修改上一轮提问）以及 `@` 本地文件/图片上传。 |
| **Q4** | **`Web` 下切换 `/model` 的状态保存与状态栏展示** | 1. **纯内存态分站点独立记忆**：在 `Lx.ai_web_config` 中分别为 `Gemini`（默认 `3.8 Flash` + `low`）和 `DeepSeek`（默认 `DeepSeek` + `deepthink`）保存当前模型与思考等级；**不写本地 `ai.json`、不写 SQLite `agent_model_switch`、不注入 `<system-update>`**。<br>2. **底部状态栏格式**：<br>  - `Gemini`：显示 `Gemini · <模型名>`（如 `Gemini · 3.8 Flash`、`Gemini · 4 Argon`）+ 右侧黄色加粗思考等级（`low` / `medium` / `high`）；<br>  - `DeepSeek`：显示 `DeepSeek` + 开启深度思考时右侧黄色加粗 `deepthink`（`off` 时不显示 variant）。<br>3. **生效时机**：切换后状态栏立即更新，下一轮发送消息时由驱动脚本自动在网页端同步对应模型与思考等级后再提交。 |

---

## 2. 模块结构与变更文件清单

```text
src/
  app/
    effects.rs                    Effect::StartAiWebStream 增加 model 与 thinking 字段
    state.rs                      重导出 AiWebSite、GeminiWebModel、GeminiThinkingLevel、DeepseekThinkingMode、AiWebConfig
    state/
      lx.rs                       Lx 结构体新增 pub ai_web_config: AiWebConfig；声明 pub mod web
      lx/web.rs                [新增] AiWebSite(Deepseek, Gemini)、GeminiWebModel、GeminiThinkingLevel、
                                  DeepseekThinkingMode、AiWebConfig 及会话前缀工具函数
      lx/slash.rs                 AI_WEB_COMMANDS 纳入 SlashCommand::Model；新增 ModelPicker::for_ai_web(site, config)
    update/
      pane.rs                     set_ai_web_site 切换站点时停止在途流、重置跨站点会话视图与编辑态、刷新状态栏
      chat/
        web.rs                 双站点前缀解析 ai_web_remote_session、apply_ai_web_status 渲染模型+variant、
                                  try_send_ai_web 放行 /model 并透传 model/thinking
        web/tests.rs           补充 Gemini 站点切换、/model 切换、会话前缀、状态栏与命令测试
        slash.rs                  open_model_picker / apply_slash_enter / finish_model_selection 支持 LxEntry::AiWeb
  chat/
    web/
      mod.rs                      嵌入 GEMINI_HELPERS_SCRIPT 与 GEMINI_SCRIPT，提供 scripts_for_site(AiWebSite)
      deepseek_helpers.js         读取 PARAMS.thinking（默认 'deepthink'）
      deepseek_script.js          按 PARAMS.thinking 同步网页端「深度思考」按钮开启或关闭
      gemini_helpers.js           [新增] Gemini 网页端辅助函数：输出发射、新对话重置、模式/思考等级切换、
                                  CDP 文件上传与芯片清理、会话列表/加载/重命名/删除
      gemini_script.js            [新增] Gemini 网页端对话主流程：登录态探测、权限弹窗拦截、模式同步、
                                  StreamGenerate 流采集 + DOM 兜底、/rewind 原地修改与富文本提交
      tests.rs                    补充 Gemini 脚本参数与协议解析单元测试
  event/
    chat/
      web.rs                   支持按 AiWebSite 或会话前缀（ai-web-ds- / ai-web-gm-）选择驱动脚本与封装前缀；
                                  ScriptParams 新增 model 与 thinking 字段
  ui/
    text.rs                       新增 AI_WEB_SITE_GEMINI = "Gemini"、Gemini 模型与思考等级文案常量、
                                  更新 LX_AI_WEB_ONLY_NEW_CMD 提示文案
    style.rs                      将 lx 品牌与胶囊配色拆入 src/ui/style/lx.rs 并 pub use lx::*（使 style.rs < 1000 行）
    style/
      lx.rs                       [新增] 承载 lx 入口渐变、Web 双站点品牌渐变（ai_web_site_gradient /
                                  ai_web_site_label_style）、256 色插值与会话标题胶囊配色
    main_content/
      lx.rs                       LxEntry::AiWeb 下 lay.inventory 高度扩为 1 + UNDERLINE_ROWS 以容纳站点下划线
      lx/brand.rs                 draw_ai_web_sites 渲染 DeepSeek · Gemini 品牌逐字渐变、选中加粗与底部同色渐变下划线
      lx/tests.rs                 补充 Gemini 站点命中、品牌渐变样式与下划线渲染单元测试
```

---

## 3. 核心架构与数据流

```text
┌────────────────────────────────────────────────────────────────────────────────────────────┐
│ 1. 欢迎页双站点展示与切换（DeepSeek · Gemini）                                              │
│                                                                                            │
│   Native · Hybrid · Automation · Web                                                      │
│                                  ──────                                                    │
│             DeepSeek · Gemini           <-- 各自常驻品牌逐字渐变；选中项 BOLD + 底部 ─ 下划线 │
│                        ──────                                                              │
│                                                                                            │
│   鼠标点击 "Gemini" (event::mouse::press -> ui::main_content::lx::ai_web_site_hit)          │
│     └─► app::update::set_ai_web_site(state, pane, AiWebSite::Gemini)                       │
│          ├─ 若站点变化：stop_stream + reset_session_view + 清空 ai_web_edit_*               │
│          ├─ pane.lx.ai_web_site = AiWebSite::Gemini                                        │
│          └─ refresh_lx_status -> 底栏显示 "Gemini · 3.8 Flash" + 黄色 "low"                 │
├────────────────────────────────────────────────────────────────────────────────────────────┤
│ 2. Web 下 `/model` 两级模型与思考等级切换（纯内存态，不污染本地 ai.json / SQLite）         │
│                                                                                            │
│   用户输入 `/model` 回车 -> open_model_picker(state, pane)                                  │
│     ├─ Gemini 站点:                                                                        │
│     │    Stage 1 (Models)  : [3.5 Flash-Lite, 3.8 Flash, 3.1 Pro, 4 Argon]                 │
│     │    Stage 2 (Variants): [low, medium, high]                                           │
│     └─ DeepSeek 站点:                                                                      │
│          Stage 1 (Models)  : [DeepSeek]                                                    │
│          Stage 2 (Variants): [deepthink, off]                                              │
│                                                                                            │
│   Stage 2 回车确认 -> finish_ai_web_model_selection(state, pane, selection)                │
│     ├─ 更新 pane.lx.ai_web_config (Gemini 模型+思考等级 / DeepSeek 思考模式)               │
│     └─ refresh_lx_status -> 底栏实时更新为如 "Gemini · 4 Argon" + 黄色 "high"              │
├────────────────────────────────────────────────────────────────────────────────────────────┤
│ 3. 发送、流式回传與云端会话操作（按站点与前缀自动路由）                                      │
│                                                                                            │
│   try_send_ai_web(state, pane, draft, ...)                                                 │
│     └─► Effect::StartAiWebStream {                                                         │
│           key, site: Gemini, model: "4 Argon", thinking: "high",                           │
│           prompt, files, keep_files, edit_user_index                                       │
│         }                                                                                  │
│     └─► event::chat::web::run_chat                                                        │
│           ├─ 根据 site 选择 gemini_helpers.js + gemini_script.js                           │
│           ├─ 注入 __LX_PARAMS { action, model, thinking, prompt, files, keepFiles, ... }   │
│           ├─ ego-browser 任务空间 'lx-tui-ai-web-gemini' 自动切换模型与思考等级并发送       │
│           ├─ 实时回传 thinking / text 增量                                                 │
│           └─ 首轮结束回传 session { id, title } -> 绑定为 "ai-web-gm-<id>"                 │
└────────────────────────────────────────────────────────────────────────────────────────────┘
```

---

## 4. 详细技术设计与关键代码

### 4.1 双站点、模型与思考等级状态定义（`src/app/state/lx/web.rs`）

#### 关键点
1. 新建 `src/app/state/lx/web.rs`，集中管理 `AiWebSite`、`GeminiWebModel`、`GeminiThinkingLevel`、`DeepseekThinkingMode`、`AiWebConfig` 以及会话 ID 前缀解析函数。
2. 明确区分两个站点的云端会话前缀：
   - `AI_WEB_DEEPSEEK_SESSION_PREFIX = "ai-web-ds-"`
   - `AI_WEB_GEMINI_SESSION_PREFIX = "ai-web-gm-"`
   - `ai_web_remote_session(session_id: &str) -> Option<(AiWebSite, &str)>`：同时返回所属站点与剥离前缀后的真实云端会话 ID。

#### 关键代码（`src/app/state/lx/web.rs`）

```rust
use crate::ui::text;

/// Web 内存会话的通用 ID 前缀（不写库，仅供流事件归属判断）。
pub const AI_WEB_SESSION_PREFIX: &str = "ai-web-";

/// 映射自 DeepSeek 云端真实会话的 ID 前缀。
pub const AI_WEB_DEEPSEEK_SESSION_PREFIX: &str = "ai-web-ds-";

/// 映射自 Gemini 云端真实会话的 ID 前缀。
pub const AI_WEB_GEMINI_SESSION_PREFIX: &str = "ai-web-gm-";

/// Web 支持的目标站点。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum AiWebSite {
    #[default]
    Deepseek,
    Gemini,
}

impl AiWebSite {
    pub const ALL: [AiWebSite; 2] = [AiWebSite::Deepseek, AiWebSite::Gemini];

    /// 该站点关联云端真实会话时的本地 ID 前缀。
    pub fn remote_session_prefix(self) -> &'static str {
        match self {
            Self::Deepseek => AI_WEB_DEEPSEEK_SESSION_PREFIX,
            Self::Gemini => AI_WEB_GEMINI_SESSION_PREFIX,
        }
    }

    /// 将云端真实会话 ID 包装为带站点前缀的本地会话 ID。
    pub fn format_remote_session_id(self, remote_id: &str) -> String {
        format!("{}{remote_id}", self.remote_session_prefix())
    }
}

/// Gemini 网页端支持的模型档位（对齐 `https://gemini.google.com/app` 模式选择器）。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum GeminiWebModel {
    FlashLite35,
    #[default]
    Flash38,
    Pro31,
    Argon4,
}

impl GeminiWebModel {
    pub const ALL: [GeminiWebModel; 4] = [
        Self::FlashLite35,
        Self::Flash38,
        Self::Pro31,
        Self::Argon4,
    ];

    /// 模型唯一键（用于 `ModelSelection.model` 与脚本参数 `PARAMS.model`）。
    pub fn id(self) -> &'static str {
        match self {
            Self::FlashLite35 => text::GEMINI_WEB_MODEL_FLASH_LITE,
            Self::Flash38 => text::GEMINI_WEB_MODEL_FLASH,
            Self::Pro31 => text::GEMINI_WEB_MODEL_PRO,
            Self::Argon4 => text::GEMINI_WEB_MODEL_ARGON,
        }
    }

    pub fn from_id(id: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|m| m.id().eq_ignore_ascii_case(id.trim()))
    }
}

/// Gemini 网页端支持的思考等级（对齐模式选择器中的「思考等级：低 / 中 / 高」）。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum GeminiThinkingLevel {
    #[default]
    Low,
    Medium,
    High,
}

impl GeminiThinkingLevel {
    pub const ALL: [GeminiThinkingLevel; 3] = [Self::Low, Self::Medium, Self::High];

    pub fn id(self) -> &'static str {
        match self {
            Self::Low => text::GEMINI_THINKING_LOW,
            Self::Medium => text::GEMINI_THINKING_MEDIUM,
            Self::High => text::GEMINI_THINKING_HIGH,
        }
    }

    pub fn from_id(id: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|l| l.id().eq_ignore_ascii_case(id.trim()))
    }
}

/// DeepSeek 网页端深度思考模式。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum DeepseekThinkingMode {
    #[default]
    DeepThink,
    Off,
}

impl DeepseekThinkingMode {
    pub const ALL: [DeepseekThinkingMode; 2] = [Self::DeepThink, Self::Off];

    pub fn id(self) -> &'static str {
        match self {
            Self::DeepThink => text::DEEPSEEK_THINKING_ON,
            Self::Off => text::DEEPSEEK_THINKING_OFF,
        }
    }

    pub fn from_id(id: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|m| m.id().eq_ignore_ascii_case(id.trim()))
    }
}

/// 单窗格内各 Web 站点的独立模型与思考等级配置（纯内存态，不落库）。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct AiWebConfig {
    pub gemini_model: GeminiWebModel,
    pub gemini_thinking: GeminiThinkingLevel,
    pub deepseek_thinking: DeepseekThinkingMode,
}

/// 会话 ID 是否属于 Web 内存会话。
pub fn is_ai_web_session(session_id: &str) -> bool {
    session_id.starts_with(AI_WEB_SESSION_PREFIX)
}

/// 从本地会话 ID 解析所属站点与云端真实会话 ID。
pub fn ai_web_remote_session(session_id: &str) -> Option<(AiWebSite, &str)> {
    if let Some(id) = session_id
        .strip_prefix(AI_WEB_DEEPSEEK_SESSION_PREFIX)
        .filter(|id| !id.is_empty())
    {
        return Some((AiWebSite::Deepseek, id));
    }
    if let Some(id) = session_id
        .strip_prefix(AI_WEB_GEMINI_SESSION_PREFIX)
        .filter(|id| !id.is_empty())
    {
        return Some((AiWebSite::Gemini, id));
    }
    None
}
```

---

### 4.2 首页 `DeepSeek · Gemini` 品牌渐变配色与下划线选中视觉（`src/ui/style/lx.rs` & `src/ui/main_content/lx/`）

#### 关键点
1. **抽离 `src/ui/style/lx.rs`**：将 `src/ui/style.rs:1023-1165` 的 `lx` 品牌渐变与会话胶囊配色迁入 `src/ui/style/lx.rs`，由 `src/ui/style.rs` 通过 `pub mod lx; pub use lx::*;` 重导出，使 `src/ui/style.rs` 降至 ~1010 行以内（严格 `< 1000` 行）。同时清理未使用的死代码 `lx_entry_border`。
2. **双站点品牌 256 色渐变定义**：
   - `AiWebSite::Deepseek`：`(Color::Indexed(63), Color::Indexed(39))`（深海钴蓝 `#5f5fff` → 电光青蓝 `#00afff`）；
   - `AiWebSite::Gemini`：`(Color::Indexed(69), Color::Indexed(204))`（Gemini 星光蓝 `#5f87ff` → 紫罗兰 → 珊瑚粉 `#ff5f87`）。
3. **选中下划线与布局高度同步**：
   - `src/ui/main_content/lx.rs:463-469`：当 `lx.entry == LxEntry::AiWeb` 时，`lay.inventory` 的高度设为 `1 + brand::UNDERLINE_ROWS`（若底部只剩 1 行空间则降级为 1 行）；
   - `src/ui/main_content/lx/brand.rs:241-267`：`draw_ai_web_sites` 按字符逐格调用 `style::ai_web_site_label_style(*site, position, length, is_selected)`；当 `*site == selected` 且 `rect.height > UNDERLINE_ROWS` 时，在 `site_rect.y + UNDERLINE_ROWS` 绘制同色渐变的 `ENTRY_UNDERLINE`（`'─'`），相邻站点间绘制 `style::muted()` 的 `·` 分隔符。

#### 关键代码（`src/ui/style/lx.rs` & `src/ui/main_content/lx/brand.rs`）

```rust
// src/ui/style/lx.rs
/// Web 站点标签的品牌渐变端点色（256 色 cube 段）。
///
/// - DeepSeek：深海钴蓝（`Indexed 63`, `#5f5fff`）→ 电光青蓝（`Indexed 39`, `#00afff`）
/// - Gemini  ：星光蓝（`Indexed 69`, `#5f87ff`）→ 珊瑚粉紫（`Indexed 204`, `#ff5f87`）
pub fn ai_web_site_gradient(site: crate::app::state::AiWebSite) -> (Color, Color) {
    match site {
        crate::app::state::AiWebSite::Deepseek => (Color::Indexed(63), Color::Indexed(39)),
        crate::app::state::AiWebSite::Gemini => (Color::Indexed(69), Color::Indexed(204)),
    }
}

/// Web 站点标签第 `position` 个字符（共 `length` 个）的样式：品牌渐变取色，选中加粗。
pub fn ai_web_site_label_style(
    site: crate::app::state::AiWebSite,
    position: usize,
    length: usize,
    selected: bool,
) -> Style {
    let (from, to) = ai_web_site_gradient(site);
    let t = if length <= 1 {
        0.0
    } else {
        position as f32 / (length - 1) as f32
    };
    let base = Style::default().fg(lerp_indexed(from, to, t));
    if selected {
        base.add_modifier(Modifier::BOLD)
    } else {
        base
    }
}
```

```rust
// src/ui/main_content/lx/brand.rs
/// 绘制 Web 站点选择行：采用各站点品牌渐变色，选中项加粗并在下方绘制同色渐变下划线。
pub(super) fn draw_ai_web_sites(
    grid: &mut Grid,
    rect: Rect,
    selected: crate::app::state::AiWebSite,
) {
    let placed = place_ai_web_sites(rect);
    let can_underline = rect.height > UNDERLINE_ROWS;
    for (index, (site, site_rect)) in placed.iter().enumerate() {
        let label =
            text::truncate_to_width(text::ai_web_site_label(*site), usize::from(site_rect.width));
        let length = label.chars().count();
        let is_selected = *site == selected;
        for (position, ch) in label.chars().enumerate() {
            let cell_style =
                style::ai_web_site_label_style(*site, position, length, is_selected);
            grid.set(
                site_rect.x.saturating_add(position as u16),
                site_rect.y,
                ch,
                cell_style,
            );
        }
        if is_selected && can_underline {
            for position in 0..length {
                let cell_style = style::ai_web_site_label_style(*site, position, length, false);
                grid.set(
                    site_rect.x.saturating_add(position as u16),
                    site_rect.y + UNDERLINE_ROWS,
                    ENTRY_UNDERLINE,
                    cell_style,
                );
            }
        }
        if let Some((_, next_rect)) = placed.get(index + 1) {
            let gap = next_rect.x.saturating_sub(site_rect.right());
            grid.put(
                site_rect.right().saturating_add(gap / 2),
                site_rect.y,
                text::LX_ENTRY_SEPARATOR,
                style::muted(),
            );
        }
    }
}
```

---

### 4.3 `Web` 下 `/model` 两级面板与状态栏集成（`src/app/state/lx/slash.rs` & `src/app/update/chat/`）

#### 关键点
1. **开放 `/model` 白名单**：
   - `src/app/state/lx/slash.rs:679`：`AI_WEB_COMMANDS` 扩为 `[SlashCommand::Model, SlashCommand::New, SlashCommand::Resume, SlashCommand::Rewind, SlashCommand::Rename]`；
   - `src/app/update/chat/slash.rs:418-437`：在 `LxEntry::AiWeb` 分支中允许 `SlashCommand::Model => open_model_picker(state, pane)`；
   - `src/app/update/chat/web.rs:76-97`：手敲 `/model` 单行无参提交时直接调用 `super::slash::open_model_picker(state, pane)`。
2. **构造 `Web` 专属 `ModelPicker`**：
   - 在 `ModelPicker` 中新增 `ModelPicker::for_ai_web(site: AiWebSite, config: AiWebConfig) -> Self`：
     - 当 `site == AiWebSite::Gemini` 时，生成 4 个 `ModelCandidate`（`provider_name = "Gemini"`，`model_name` 依次为 `"3.5 Flash-Lite"`、`"3.8 Flash"`、`"3.1 Pro"`、`"4 Argon"`），`current` 指向 `config.gemini_model`；
     - 当 `site == AiWebSite::Deepseek` 时，生成 1 个 `ModelCandidate`（`provider_name = "DeepSeek"`，`model_name = "DeepSeek"`），`current = Some(0)`。
   - 在 `src/app/update/chat/slash.rs` 中：
     - `variant_keys` 对 `Gemini` 返回 `["low", "medium", "high"]`（按强度序而非字典序！），对 `DeepSeek` 返回 `["deepthink", "off"]`；
     - `default_variant_index` 与 `current_variant` 直接从窗格的 `pane.lx.ai_web_config` 读取当前选中的思考等级下标；
     - `finish_model_selection` 检测到 `target.lx.entry == LxEntry::AiWeb` 时，仅更新 `target.lx.ai_web_config` 并调用 `refresh_lx_status(state, pane)`，**不写 `ai.json`、不入队 `SetChatSessionModel`、不插入 `ModelSwitchNoticeView`**。
3. **底部状态栏 `apply_ai_web_status`**：
   - `AiWebSite::Gemini`：`status.model_name = Some(format!("Gemini · {}", lx.ai_web_config.gemini_model.id()))`，`status.variant = Some(lx.ai_web_config.gemini_thinking.id().to_string())`；
   - `AiWebSite::Deepseek`：`status.model_name = Some("DeepSeek".to_string())`，`status.variant = (lx.ai_web_config.deepseek_thinking == DeepseekThinkingMode::DeepThink).then(|| "deepthink".to_string())`。

#### 关键代码（`src/app/update/chat/web.rs` & `src/app/update/chat/slash.rs`）

```rust
// src/app/update/chat/web.rs
/// 状态行展示站点/模型名、思考等级（variant）与流状态，隐藏 API 供应商与上下文占用。
pub fn apply_ai_web_status(lx: &mut Lx) {
    match lx.ai_web_site {
        crate::app::state::AiWebSite::Deepseek => {
            lx.chat.status.model_name = Some(text::AI_WEB_SITE_DEEPSEEK.to_string());
            lx.chat.status.variant = match lx.ai_web_config.deepseek_thinking {
                crate::app::state::DeepseekThinkingMode::DeepThink => {
                    Some(text::DEEPSEEK_THINKING_ON.to_string())
                }
                crate::app::state::DeepseekThinkingMode::Off => None,
            };
        }
        crate::app::state::AiWebSite::Gemini => {
            lx.chat.status.model_name = Some(format!(
                "{} · {}",
                text::AI_WEB_SITE_GEMINI,
                lx.ai_web_config.gemini_model.id()
            ));
            lx.chat.status.variant = Some(lx.ai_web_config.gemini_thinking.id().to_string());
        }
    }
    lx.chat.status.provider_name = None;
    lx.chat.status.context_limit = None;
    lx.chat.status.input_tokens = None;
    lx.chat.status.streaming = lx.chat.request.is_busy();
}
```

```rust
// src/app/update/chat/slash.rs
fn apply_ai_web_model_selection(
    state: &mut AppState,
    pane: PaneId,
    selection: crate::config::ai::ModelSelection,
) -> bool {
    let Some(target) = state.pane_mut_anywhere(pane) else {
        return false;
    };
    match target.lx.ai_web_site {
        crate::app::state::AiWebSite::Gemini => {
            if let Some(model) = crate::app::state::GeminiWebModel::from_id(&selection.model) {
                target.lx.ai_web_config.gemini_model = model;
            }
            if let Some(level) = selection
                .variant
                .as_deref()
                .and_then(crate::app::state::GeminiThinkingLevel::from_id)
            {
                target.lx.ai_web_config.gemini_thinking = level;
            }
        }
        crate::app::state::AiWebSite::Deepseek => {
            if let Some(mode) = selection
                .variant
                .as_deref()
                .and_then(crate::app::state::DeepseekThinkingMode::from_id)
            {
                target.lx.ai_web_config.deepseek_thinking = mode;
            }
        }
    }
    super::refresh_lx_status(state, pane);
    true
}
```

---

### 4.4 站点切换隔离（`src/app/update/pane.rs`）

#### 关键点
当用户在欢迎页点击切换 `DeepSeek` 与 `Gemini`（或通过 `set_ai_web_site` 切换站点）时：
- 若目标 `site` 与当前 `target.lx.ai_web_site` 相同，不做任何状态破坏；
- 若不同：先停止在途流（`chat::stop_stream`），清空跨站点会话视图（`reset_session_view()`）、清空 `/rewind` 待编辑状态（`ai_web_edit_user_index = None`、`ai_web_edit_cloud_files.clear()`）、清除撤销与斜杠面板，再写入新 `ai_web_site` 并刷新状态栏。

#### 关键代码（`src/app/update/pane.rs`）

```rust
/// 切换指定 lx 窗格在 Web 入口下的目标站点，并隔离跨站点残留会话状态。
pub fn set_ai_web_site(state: &mut AppState, pane: PaneId, site: crate::app::state::AiWebSite) {
    let changed = state
        .pane_anywhere(pane)
        .is_some_and(|p| p.lx.ai_web_site != site);
    if !changed {
        return;
    }
    if state
        .pane_anywhere(pane)
        .is_some_and(|p| p.lx.chat.request.is_busy())
    {
        chat::stop_stream(state, pane);
    }
    if let Some(target) = state.pane_mut_anywhere(pane) {
        target.lx.ai_web_site = site;
        target.lx.chat.reset_session_view();
        target.lx.ai_web_edit_user_index = None;
        target.lx.ai_web_edit_cloud_files.clear();
        target.lx.clear_rewind();
        target.lx.slash = None;
    }
    chat::refresh_lx_status(state, pane);
}
```

---

### 4.5 Gemini Web 嵌入式驱动脚本（`src/chat/web/gemini_helpers.js` & `gemini_script.js`）

#### 关键点
1. **任务空间与登录态探测**：
   - 空间名：`SPACE = 'lx-tui-ai-web-gemini'`，主页：`HOME = 'https://gemini.google.com/app'`；
   - 登录态判定：检查 `location.hostname === 'gemini.google.com'` 且页面不存在指向 `accounts.google.com/ServiceLogin` 的登录按钮、且存在 `.ql-editor[contenteditable="true"]` 或 `[data-test-id="bard-mode-menu-button"]`。未登录时调用 `handOffTaskSpace()` 并回投 `{ event: 'login_required', url: HOME }`。
2. **拦截通知权限弹窗（防止夺取 `ego-browser` 控制权）**：
   - 每次进入页面立即注入：
     ```javascript
     if (typeof Notification !== 'undefined') {
       Notification.requestPermission = () => Promise.resolve('denied');
     }
     ```
3. **模型与思考等级自动同步（`ensureGeminiModelAndThinking(MODEL, THINKING)`）**：
   - 点击 `[data-test-id="bard-mode-menu-button"]` 打开模式菜单；
   - 在 `.cdk-overlay-container [role="menuitemradio"]` 中查找文本包含目标 `MODEL`（如 `'3.5 Flash-Lite'`、`'3.8 Flash'`、`'3.1 Pro'`、`'4 Argon'`）的项，若未处于选中态（`aria-checked !== 'true'` 且内部无 `check` 图标）则点击切换；
   - 重新打开（或保持）模式菜单，触发底部「思考等级 / Thinking level」子菜单（`[role="menuitem"]`），在子菜单 `[role="menuitemradio"]` 中匹配目标 `THINKING`：
     - `'low'` → 匹配 `'低'` 或 `'Low'`
     - `'medium'` → 匹配 `'中'` 或 `'Medium'`
     - `'high'` → 匹配 `'高'` 或 `'High'`
   - 点击对应思考等级后按 `Escape` 关闭残留浮层。
4. **CSP TrustedHTML 安全文本写入与提交**：
   - 严禁设置 `.innerHTML`；统一对 `.ql-editor[contenteditable="true"]` 调用 `focus()` → `document.execCommand('selectAll', false, null)` → `document.execCommand('insertText', false, PROMPT)` 并派发 `input` 事件；
   - 点击发送按钮 `button.send-button, button[aria-label*="发送"], button[aria-label*="Send"]`（或派发 `Enter` 键）。
5. **流式回答双通道采集（XHR/fetch `StreamGenerate` + DOM `<model-response>` 轮询）**：
   - 监控最新一条 `<model-response>`：
     - 思考增量：采集 `<model-thoughts>` / `<thinking-overlay>` / `.thoughts-content` 的文本增量推送 `thinking` 事件；
     - 正文增量：采集 `<message-content>`（剔除 `.cdk-visually-hidden` 的 `"Gemini 说"` 标签与引用角标按钮文本）推送 `text` 事件；
     - 完成判定：当停止按钮（`button[aria-label*="停止"], button[aria-label*="Stop"], .stop-icon`）消失、`model-response` 的 `.model-response-label-announcer` `aria-busy="false"` 且连续 6 个采样周期（360ms）文本稳定时，提取 URL `/app/([a-zA-Z0-9]+)` 的会话 ID 与侧栏 `a[href*="/app/<id>"]` 标题，回投 `session` 与 `done`。
6. **CDP 文件/图片上传与 `/rewind` 原地修改**：
   - 文件上传：若页面尚无 `input[type="file"]`，先点击 `button[aria-label*="上传和工具"], button[aria-label*="Upload"]` 触发挂载文件输入或通过 CDP `DOM.setFileInputFiles` 注入本地绝对路径列表 `FILES`，轮询等待预览容器 `.file-preview-container, .uploader-file-preview-container` 加载完成；
   - `/rewind` 原地修改（`EDIT_INDEX >= 0`）：定位第 `EDIT_INDEX` 个 `<user-query>`，点击其 `button[aria-label="修改"], button[aria-label*="Edit"]`，在展开的编辑文本框中通过原生 setter / `execCommand` 填入新 `PROMPT` 并点击确认按钮（`"更新"` / `"Update"` / `"发送"` / `"Send"`）。
7. **云端会话管理（`list_sessions` / `load_session` / `rename_session` / `delete_session`）**：
   - `list_sessions`：从侧边栏 `a[href*="/app/"]` 提取全部历史会话 `{ id, title, updated_at_ms }`（若侧边栏收起则先展开或提取 DOM 中的导航项）；
   - `load_session`：导航至 `https://gemini.google.com/app/<sessionId>`，等待 `<user-query>` 与 `<model-response>` 渲染完毕，按轮次提取用户文本、附件文件名、助手思考内容与正文 Markdown，回投 `session_loaded`；
   - `rename_session`：定位侧边栏对应会话项右侧的「更多选项」按钮（`button[aria-label*="更多选项"], button[aria-label*="More options"]`）→ 点击「重命名 / Rename」→ 在弹窗输入框填入 `TITLE` 并点击确认；
   - `delete_session`：定位侧边栏对应会话项右侧的「更多选项」按钮 → 点击「删除 / Delete」→ 在确认弹窗点击「删除 / Delete」，若当前正停留在该会话 URL 则导航回 `https://gemini.google.com/app`。

---

## 5. 风险与边界不变量分析

1. **`Notification.requestPermission` 权限弹窗拦截不变量**：
   - Gemini 网页端在首次发送提示词时会请求浏览器通知权限，一旦弹出原生权限框会导致 `ego-browser` 将任务空间控制权移交用户并报错中断。`gemini_script.js` 必须在任何交互前覆写 `Notification.requestPermission` 与 `navigator.permissions.query`。
2. **CSP `TrustedHTML` 合规不变量**：
   - `gemini.google.com` 强制开启 Trusted Types，脚本内任何 DOM 修改不得使用 `.innerHTML = ...` 或 `.outerHTML = ...`，一律使用 `document.execCommand('insertText', ...)` 或原生 `HTMLTextAreaElement.prototype.value` setter。
3. **跨站点与跨模式零污染不变量**：
   - `Web` 下 `/model` 修改仅写入 `pane.lx.ai_web_config`，绝不触发 `Effect::SaveDefaultModel` 或 `Effect::SetChatSessionModel`；
   - `ai-web-ds-*` 与 `ai-web-gm-*` 前缀严格区分会话归属，切换站点时强制清理上一站点的内存会话视图与 `/rewind` 索引。
4. **单文件行数上限不变量**：
   - 拆分 `src/ui/style/lx.rs` 与 `src/app/state/lx/web.rs`，确保所有生产 `.rs` 文件严格 `< 1000` 行。

---

## 6. 无头验证 CLI：`lx-tui web`

为让 `DeepSeek` 与 `Gemini` 的全部网页能力可脱离 TUI 做端到端验证，新增顶层子命令 `lx-tui web`：

```text
src/cli/
  web.rs            子命令解析（models / chat / new / stop / list / show / rename / delete）、
                    --site 站点推断、模型/思考等级校验、会话前缀与 --site 冲突拦截
  web/exec.rs       执行器：复用 chat::web 的嵌入式脚本与 event::chat::web 的流监听器，
                    输出人类可读表格或 --json
```

约束：

- `chat --edit-index <N>` 必须同时提供 `--session <SESSION_ID>`，`--keep-file` 必须同时提供 `--edit-index`，`--first-turn` 不得与 `--session` / `--edit-index` 组合，避免误改/误清云端会话；
- 站点由 `--site` 或 `ai-web-ds-` / `ai-web-gm-` 前缀推断；两者冲突时直接报错；
- `--file` 复用 `chat::prepare::prepare_ai_web_message` 的目录/Skill/MCP 预检与 CDP 上传路径；
- 流式事件经 `spawn_stream_listener` 的本地回环 HTTP 实时采集，URL 带一次性 token，仅本进程可写入。

---

## 7. Review 修复（详见 `review.md`）

本轮 review 在 `design.md` 实现之外补齐了以下行为，均已在 `verification.md` 回归：

1. **异步 `/resume` 归属校验**：`on_ai_web_session_loaded` 仅在 `chat.loading_session == true` 时应用结果，用户切换入口/站点后迟到的云端历史被丢弃；
2. **`/model` 站点归属**：`finish_model_selection` / `variant_keys_for_pane` / `current_variant_index_for_pane` 优先用候选的 `provider` 反解站点（`AiWebSite::from_label`），避免面板存活期间站点变化时写错配置；
3. **`/rewind` 附件与重置**：Gemini 的原地编辑只改文本，编辑前按附件全名集合校验——集合一致则保留原附件，第 0 轮集合变化回退新云端会话，更早轮次集合变化显式报错；旧回答重置后清空采集游标；DeepSeek 越界索引改为显式错误；
4. **云端动作失败可见**：DeepSeek/Gemini 的 rename/delete 在菜单、HTTP 或确认按钮失败时回投 `error`，事件层弹 toast，CLI 非零退出；
5. **模型/思考等级同步校验**：Gemini 菜单缺失、模型不存在、思考等级菜单或目标项缺失时返回 error，不再静默沿用旧设置；模型选择后通过重开菜单的 `aria-checked` 做后置校验；
6. **上传稳健性**：抽屉已展开时不再重复点击；单文件输入时合并 media + document 一次注入；
7. **流监听器安全**：token 路径校验、16 KiB header / 1 MiB body 上限、5 秒读超时；
8. **超时**：stdin 写入统一 30 秒；`load_session` 查询预算提升到 60 秒并保留 stderr 首条诊断。
