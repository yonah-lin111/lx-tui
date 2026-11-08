# `Web` 接入 Google Gemini 网页端、双站点品牌渐变与 `/model` 模型/思考等级切换 — 执行任务清单 (`task.md`)

- **状态**：已完成实现、CLI 端到端验证与 review 修复（位于 `.worktrees/lx-ai-web-gemini` 工作区 `feat/lx-ai-web-gemini` 分支）
- **设计依据**：[`docs/feature/lx-ai-web-gemini/design.md`](design.md)
- **参考驱动规范**：[`docs/standards/web-deepseek.md`](../../standards/web-deepseek.md)
- **验证报告**：[`verification.md`](verification.md) · **Review 与修复**：[`review.md`](review.md)

---

## 0. 开工门禁与硬性规范

- **门禁**：必须等用户审核并明确确认本目录下的 `design.md` 与 `task.md` 后，才可修改 `src/` 与 `tests/` 代码。
- **架构红线**：
  - `app/` 只管纯内存状态与状态转换，`ui/` 只读渲染，严禁在渲染帧中修改状态或执行 IO；
  - 所有 TUI 可见文案统一使用英文并集中定义在 `src/ui/text.rs`；
  - 生产代码单文件严禁超过 1000 行，严禁使用 `unwrap()` / `expect()`；测试文件跟随子模块拆分；
  - 未获明确授权禁止执行任何修改 Git 状态的命令。

---

## 1. 需求与边界确认清单（Grill Me 已确认）

- [x] **Q1（Gemini 模型切换与思考等级交互方式及范围）**：
  - 在 `Web` 下直接开放并复用 `/model` 两级选择面板（无需配置本地 API Key）；
  - 当选中 `Gemini` 时：第 1 级提供 4 个模型（`3.5 Flash-Lite`、`3.8 Flash`、`3.1 Pro`、`4 Argon`，默认 `3.8 Flash`），回车进入第 2 级选择 3 档思考等级（`low`、`medium`、`high`，默认 `low`）；
  - 当选中 `DeepSeek` 时：第 1 级提供 `DeepSeek`，回车进入第 2 级选择深度思考开关（`deepthink`、`off`，默认 `deepthink`）。
- [x] **Q2（首页 `DeepSeek · Gemini` 品牌渐变配色与选中态视觉样式）**：
  - 在 `Web` 欢迎页下方并排展示 `DeepSeek · Gemini`；
  - 配色与选中态逻辑完全对齐上方 4 个模式：常驻各自品牌 256 色逐字渐变（`DeepSeek` 深海蓝 `Indexed(63)` → 亮青蓝 `Indexed(39)`；`Gemini` 星光蓝 `Indexed(69)` → 紫罗兰 → 珊瑚粉 `Indexed(204)`），选中时**字体加粗（`BOLD`）并在正下方绘制同品牌逐字渐变的 `─` 下划线**。
- [x] **Q3（双站点会话隔离与基础命令对齐）**：
  - `DeepSeek` 使用任务空间 `lx-tui-ai-web-deepseek` 与云端会话前缀 `ai-web-ds-<id>`；`Gemini` 使用任务空间 `lx-tui-ai-web-gemini` 与云端会话前缀 `ai-web-gm-<id>`；
  - 点击切换站点时若站点变化，自动停止在途流、重置跨站点会话视图、清空 `/rewind` 待编辑状态并刷新状态栏；
  - `Gemini` 完整支持 `/new`、`/resume`（含 `Shift+Enter` 重命名、`Ctrl+D` 删除）、`/rename`、`/rewind`（含空闲双击 `Esc` 原地编辑重发）以及 `@` 本地文件/图片上传。
- [x] **Q4（`Web` 下切换 `/model` 的内存保存与状态栏展示）**：
  - 各站点的模型与思考等级独立保存在窗格内存态 `Lx.ai_web_config` 中，不修改本地 `ai.json`、不写 SQLite `agent_model_switch`、不注入 `<system-update>`；
  - 底部状态栏：`Gemini` 显示 `Gemini · <模型名>` + 右侧黄色加粗 `low` / `medium` / `high`；`DeepSeek` 显示 `DeepSeek` + 开启深度思考时右侧黄色加粗 `deepthink`（`off` 时隐藏）。

---

## 2. 分阶段实施步骤拆解

### Phase 1：双站点状态建模、会话前缀与文案常量扩展（`src/ui/text.rs` & `src/app/`）

> 目标：在 `src/ui/text.rs` 定义 Gemini 与模型/思考等级英文文案；新增 `src/app/state/lx/web.rs` 承载双站点枚举、模型与思考等级配置及双前缀解析；扩展 `Effect::StartAiWebStream`。

- [x] **1.1 在 `src/ui/text.rs` 中新增英文文案常量与辅助函数**
  - 新增站点名与模型/思考等级常量：
    - `AI_WEB_SITE_GEMINI = "Gemini"`
    - `GEMINI_WEB_MODEL_FLASH_LITE = "3.5 Flash-Lite"`
    - `GEMINI_WEB_MODEL_FLASH = "3.8 Flash"`
    - `GEMINI_WEB_MODEL_PRO = "3.1 Pro"`
    - `GEMINI_WEB_MODEL_ARGON = "4 Argon"`
    - `GEMINI_THINKING_LOW = "low"`
    - `GEMINI_THINKING_MEDIUM = "medium"`
    - `GEMINI_THINKING_HIGH = "high"`
    - `DEEPSEEK_THINKING_ON = "deepthink"`
    - `DEEPSEEK_THINKING_OFF = "off"`
  - 更新 `ai_web_site_label(site: AiWebSite)` 支持 `AiWebSite::Gemini => AI_WEB_SITE_GEMINI`；
  - 新增 `lx_ai_web_login_required_for(site: AiWebSite) -> &'static str`（区分 `"Please sign in to DeepSeek in ego-browser"` 与 `"Please sign in to Gemini in ego-browser"`）；
  - 更新 `LX_AI_WEB_ONLY_NEW_CMD = "Only /model, /new, /resume, /rename, and /rewind are available in Web"`。
- [x] **1.2 新增 `src/app/state/lx/web.rs` 并接入 `src/app/state/lx.rs`、`src/app/effects.rs`**
  - 在 `src/app/state/lx/web.rs` 中定义：
    - `AI_WEB_SESSION_PREFIX = "ai-web-"`、`AI_WEB_DEEPSEEK_SESSION_PREFIX = "ai-web-ds-"`、`AI_WEB_GEMINI_SESSION_PREFIX = "ai-web-gm-"`；
    - `AiWebSite { Deepseek, Gemini }`（`ALL = [Deepseek, Gemini]`、`remote_session_prefix`、`format_remote_session_id`）；
    - `GeminiWebModel { FlashLite35, Flash38, Pro31, Argon4 }`、`GeminiThinkingLevel { Low, Medium, High }`、`DeepseekThinkingMode { DeepThink, Off }` 与 `AiWebConfig`；
    - `is_ai_web_session(session_id)` 与 `ai_web_remote_session(session_id) -> Option<(AiWebSite, &str)>`。
  - 在 `src/app/state/lx.rs` 中挂载 `pub ai_web_config: AiWebConfig`，并在 `src/app/state.rs` 重导出；
  - 在 `src/app/effects.rs` 的 `Effect::StartAiWebStream` 中增加 `model: String` 与 `thinking: String` 字段。

---

### Phase 2：`Web` 下 `/model` 两级面板、站点切换隔离与状态栏更新（`src/app/state/lx/slash.rs` & `src/app/update/`）

> 目标：打通 `Web` 下的 `/model` 两级选择面板、内存态 `ai_web_config` 更新、底部状态栏展示与跨站点切换隔离。

- [x] **2.1 扩展 `src/app/state/lx/slash.rs` 支持 `Web` 模型候选**
  - 将 `SlashCommand::Model` 加入 `AI_WEB_COMMANDS`；
  - 新增 `ModelPicker::for_ai_web(site: AiWebSite, config: AiWebConfig) -> Self`：
    - `AiWebSite::Gemini` 生成 4 个候选（`3.5 Flash-Lite`、`3.8 Flash`、`3.1 Pro`、`4 Argon`），`current` 指向当前 `config.gemini_model`；
    - `AiWebSite::Deepseek` 生成 1 个候选（`DeepSeek`），`current = Some(0)`。
- [x] **2.2 更新 `src/app/update/chat/slash.rs`、`src/app/update/chat/web.rs` 与 `src/app/update/pane.rs`**
  - `src/app/update/chat/slash.rs`：
    - `open_model_picker` 在 `LxEntry::AiWeb` 下调用 `ModelPicker::for_ai_web(target.lx.ai_web_site, target.lx.ai_web_config)`；
    - `apply_slash_enter` 进入第二级 `ModelPickerStage::Variants` 时，在 `LxEntry::AiWeb` 下按站点返回固定顺序的思考等级列表（Gemini：`["low", "medium", "high"]`；DeepSeek：`["deepthink", "off"]`），并高亮当前 `ai_web_config` 中的思考等级；
    - `finish_model_selection` 在 `LxEntry::AiWeb` 下调用 `apply_ai_web_model_selection`，仅更新 `target.lx.ai_web_config` 并刷新状态栏，不写 `ai.json` / SQLite；
    - `apply_slash_command` 在 `LxEntry::AiWeb` 分支放行 `SlashCommand::Model`。
  - `src/app/update/chat/web.rs`：
    - `apply_ai_web_status`：Gemini 下设置 `model_name = "Gemini · <model>"`、`variant = Some("<thinking>")`；DeepSeek 下设置 `model_name = "DeepSeek"`、`variant = Some("deepthink")`（或 `None`）；
    - `try_send_ai_web`：支持手敲 `/model` 打开选择器；发送消息时将当前站点的 `(model, thinking)` 与助手气泡 `model_label` 一并写入 `ChatMessageView` 与 `Effect::StartAiWebStream`。
  - `src/app/update/pane.rs`：
    - `set_ai_web_site` 在站点发生改变时停止在途流、调用 `reset_session_view()`、清空 `ai_web_edit_user_index` / `ai_web_edit_cloud_files` / `rewind` / `slash`，再更新 `ai_web_site` 并调用 `refresh_lx_status`。

---

### Phase 3：首页 `DeepSeek · Gemini` 品牌渐变配色与下划线渲染（`src/ui/`）

> 目标：拆分 `src/ui/style/lx.rs` 使 `src/ui/style.rs` 降至 1000 行以内，并实现首页 `DeepSeek · Gemini` 的品牌逐字渐变、选中加粗与底部同色渐变 `─` 下划线。

- [x] **3.1 拆分 `src/ui/style/lx.rs` 并新增站点品牌渐变函数**
  - 新建 `src/ui/style/lx.rs`，迁入 `src/ui/style.rs` 底部的 `LX_PINK`、`LX_SKY_BLUE`、`logo_pixel`、`lx_entry_gradient`、`lx_entry_label_style`、`cube_rgb`、`cube_index`、`lerp_indexed`、`lx_input_border`、`SESSION_TITLE_PALETTE`、`session_title_badge`，并在 `src/ui/style.rs` 中 `pub mod lx; pub use lx::*;`；删除未使用的死代码 `lx_entry_border`；
  - 在 `src/ui/style/lx.rs` 中新增 `ai_web_site_gradient(site: AiWebSite)` 与 `ai_web_site_label_style(site, position, length, selected)`：
    - `Deepseek`：`(Color::Indexed(63), Color::Indexed(39))`；
    - `Gemini`：`(Color::Indexed(69), Color::Indexed(204))`。
- [x] **3.2 更新 `src/ui/main_content/lx.rs` 与 `src/ui/main_content/lx/brand.rs`**
  - `src/ui/main_content/lx.rs`：在 `LxEntry::AiWeb` 下计算 `lay.inventory` 时，若空间允许则赋予高度 `1 + brand::UNDERLINE_ROWS`（2 行），确保选中站点的底部 `─` 下划线完整显示；
  - `src/ui/main_content/lx/brand.rs`：`draw_ai_web_sites` 对每个站点逐字符应用 `style::ai_web_site_label_style`，选中站点加粗并在 `site_rect.y + UNDERLINE_ROWS` 绘制同色渐变 `ENTRY_UNDERLINE`（`'─'`），站点间绘制 `style::muted()` 的 `·`。

---

### Phase 4：Gemini 网页驱动脚本与事件层双站点路由（`src/chat/web/` & `src/event/chat/web.rs`）

> 目标：编写 `gemini_helpers.js` 与 `gemini_script.js`，更新 `deepseek_script.js` 支持 `thinking` 开关，并在 `src/event/chat/web.rs` 按站点与会话前缀完成路由。

- [x] **4.1 更新 `src/chat/web/mod.rs`、`deepseek_helpers.js` 与 `deepseek_script.js`**
  - `deepseek_helpers.js`：读取 `MODEL = PARAMS.model || 'DeepSeek'` 与 `THINKING = PARAMS.thinking || 'deepthink'`；
  - `deepseek_script.js`：根据 `THINKING === 'deepthink'` 动态开启或关闭页面上的 `.ds-toggle-button`（「深度思考 / DeepThink」）；
  - `src/chat/web/mod.rs`：通过 `include_str!` 引入 `GEMINI_HELPERS_SCRIPT` 与 `GEMINI_SCRIPT`，提供 `scripts_for_site(site: AiWebSite) -> (&'static str, &'static str)`。
- [x] **4.2 编写 `src/chat/web/gemini_helpers.js` 与 `src/chat/web/gemini_script.js`**
  - 空间管理与安全守卫：绑定 `SPACE = 'lx-tui-ai-web-gemini'`，主页 `https://gemini.google.com/app`；注入 `Notification.requestPermission = () => Promise.resolve('denied')` 阻断权限弹窗；检测登录态并在未登录时 `handOffTaskSpace()` + 回投 `login_required`；
  - 模型与思考等级切换（`ensureGeminiModelAndThinking`）：点开 `[data-test-id="bard-mode-menu-button"]`，选择匹配 `MODEL`（`3.5 Flash-Lite` / `3.8 Flash` / `3.1 Pro` / `4 Argon`）的 `menuitemradio`，再打开「思考等级 / Thinking level」子菜单选择匹配 `THINKING`（`low`→`低/Low`、`medium`→`中/Medium`、`high`→`高/High`）的 `menuitemradio`；
  - 流式回答采集与发送：使用 `document.execCommand('insertText', false, PROMPT)` 满足 CSP TrustedHTML 要求；通过 `StreamGenerate` 拦截 + DOM `<model-response>`（`<thinking-overlay>` / `<model-thoughts>` 与 `<message-content>`）60ms 轮询实时推送 `thinking` 与 `text` 增量，完成后提取 `/app/<id>` 会话 ID 与标题回投 `session` + `done`；
  - 附件上传与 `/rewind` 原地修改：支持 CDP `DOM.setFileInputFiles` 上传本地文件/图片与清理预览卡片；当 `EDIT_INDEX >= 0` 时定位第 `EDIT_INDEX` 条 `<user-query>` 点击 `button[aria-label="修改"]` 原地更新并重发；
  - 云端会话动作：实现 `new`、`stop`、`list_sessions`（侧栏 `a[href*="/app/"]`）、`load_session`（解析历史 `<user-query>` 与 `<model-response>`）、`rename_session`（侧栏菜单重命名）、`delete_session`（侧栏菜单删除）。
- [x] **4.3 更新 `src/event/chat/web.rs` 支持双站点脚本与会话前缀路由**
  - `ScriptParams` 新增 `model: &'a str` 与 `thinking: &'a str`；
  - `script_source(site, &params)` 根据 `site` 拼接对应站点的 `helpers.js` + `script.js`；
  - `start_stream`、`reset_session`、`list_sessions` 按 `site` 执行并包装 `site.format_remote_session_id(&id)`；
  - `load_session`、`rename_session`、`delete_session` 使用 `ai_web_remote_session(session_id)` 自动识别 `Deepseek`（`ai-web-ds-`）或 `Gemini`（`ai-web-gm-`）并调用对应站点脚本；
  - 消除 `src/event/chat/web.rs` 中重复定义的 `is_ai_web_session`，统一调用 `crate::app::state::is_ai_web_session`。

---

### Phase 5：单元测试与精确校验（`src/**/tests.rs`）

> 目标：补齐双站点 UI 渐变/命中、`/model` 两级切换、双站点会话隔离与协议脚本测试，并通过 fmt / clippy / test 校验。

- [x] **5.1 UI 欢迎页双站点渐变、下划线与命中测试（`src/ui/main_content/lx/tests.rs`）**
  - 测试 `LxEntry::AiWeb` 下欢迎页并排渲染 `DeepSeek · Gemini`；
  - 测试 `ai_web_site_hit` 可分别命中 `Deepseek` 与 `Gemini`；
  - 测试选中 `Deepseek` 或 `Gemini` 时文字加粗且 `y + 1` 行渲染对应品牌渐变 `─` 下划线，未选中项保持品牌渐变无下划线。
- [x] **5.2 状态机、`/model` 两级面板与双站点隔离测试（`src/app/update/chat/web/tests.rs`）**
  - 测试 `Gemini` 下打开 `/model` 展示 4 个模型，进入第 2 级展示 `low / medium / high`，确认后更新 `ai_web_config` 与状态栏 `Gemini · 4 Argon` + `high`，且不污染本地 `default_model`；
  - 测试 `DeepSeek` 下打开 `/model` 切换 `deepthink` / `off` 并同步状态栏；
  - 测试 `set_ai_web_site` 在 `Deepseek` 与 `Gemini` 间切换时自动清理跨站点会话与 `/rewind` 编辑状态；
  - 测试 `ai-web-ds-<id>` 与 `ai-web-gm-<id>` 前缀识别与发送时 `StartAiWebStream` 的 `site` / `model` / `thinking` 参数透传。
- [x] **5.3 驱动脚本组装与协议解析测试（`src/chat/web/tests.rs`）**
  - 测试 `scripts_for_site(AiWebSite::Deepseek)` 与 `scripts_for_site(AiWebSite::Gemini)` 非空且包含关键动作入口。
  - 测试 `script_source` 对引号/反引号/`${}`/换行等用户输入的 JSON 转义。

---

### Phase 6：无头 CLI、端到端验证与 review 修复（`src/cli/web.rs` & `src/cli/web/exec.rs`）

> 目标：让双站点全部网页能力可通过 `lx-tui web` 脱离 TUI 验证，并按 `review.md` 的 P0-P3 结论修复。

- [x] **6.1 新增 `lx-tui web` 子命令**
  - `models` / `chat` / `new` / `stop` / `list` / `show` / `rename` / `delete`，支持 `--site`、`-m/--model`、`-t/--thinking`、`-s/--session`、`--first-turn`、`--edit-index`、`--file`、`--keep-file`、`--limit`、`--json`；
  - 会话前缀自动推断站点，`--site` 冲突时拒绝；`--edit-index` 必须绑定 `--session`，`--keep-file` 必须绑定 `--edit-index`，`--first-turn` 不得复用旧会话；
  - 复用 `chat::prepare::prepare_ai_web_message`、`chat::web::script_source` 与 `event::chat::web::spawn_stream_listener`。
- [x] **6.2 编写 `verification.md`**
  - V1～V10 全部包含输入内容、验证步骤、期望结果与真实 CLI 输出；覆盖双站点模型/思考等级、流式对话、文件上传、`/rewind`、历史列表/加载/重命名/删除与禁用引用拦截。
- [x] **6.3 按 `review.md` 修复**
  - P1：`/resume` 迟到结果归属校验、`/model` 站点归属、Gemini `/rewind` 附件集合校验与重置、DeepSeek 越界索引、CLI `/rewind` 参数约束、云端 rename/delete 失败可见、Gemini 模型/思考等级同步校验；
  - P2：上传抽屉误关、单文件输入覆盖、宽松选择器、窄窗格站点裁剪、流监听器 token/大小/超时、stdin 写超时、`load_session` 预算与诊断、DeepSeek 流 runId；
  - P3：死代码/死常量清理、状态栏分隔符收口、过期注释与文档路径更新。
- [x] **6.4 回归验证**
  - `rtk cargo fmt --check`、`rtk cargo clippy --all-targets -- -D warnings`、`rtk cargo test` 全绿；
  - `lx-tui web` 对 DeepSeek / Gemini 的模型切换、流式对话、文件上传、`/rewind`、rename/delete 与错误路径实测通过。

---

## 3. 验证命令

```bash
rtk cargo fmt --check
rtk cargo clippy --all-targets -- -D warnings
rtk cargo test
```

---

## 4. 完成定义（DoD）

- [x] 首页选择 `Web` 时，`DeepSeek` 右侧新增 `Gemini`（`DeepSeek · Gemini`），两者均采用各自品牌 256 色逐字渐变，选中态加粗并在底部渲染同色渐变 `─` 下划线。
- [x] `Web` 下支持 `/model` 两级面板切换模型与思考等级（`Gemini`：4 模型 × `low/medium/high`；`DeepSeek`：`deepthink/off`），并在底部状态栏实时展示模型名与黄色思考等级。
- [x] `Gemini` 网页驱动完整支持流式对话（含思考过程与正文）、模型与思考等级同步、`/new`、`/resume`（列表、加载、重命名、删除）、`/rename`、`/rewind`（原地修改上一条提问）以及本地文件/图片上传。
- [x] 双站点会话前缀（`ai-web-ds-*` / `ai-web-gm-*`）与任务空间完全隔离，切换站点自动清理残留状态。
- [x] `lx-tui web` 无头 CLI 覆盖双站点的模型查询、对话、`/new`、`/stop`、列表、加载、重命名与删除，并通过 `verification.md` 的 V1～V10 实测。
- [x] `review.md` 中 P0/P1/P2 可执行项已修复，P3 死代码/文案/文档问题已清理；其余明确记录的遗留项（共享标签页并发、CLI 编排重复、测试文件拆分）不影响本轮交付。
- [x] 所有受影响生产代码单文件 `< 1000` 行，无 `unwrap()` / `expect()`，`cargo fmt`、`cargo clippy --all-targets -- -D warnings`、`cargo test` 全部通过。
