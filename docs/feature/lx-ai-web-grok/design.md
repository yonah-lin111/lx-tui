# `Web` 接入 xAI Grok 网页端 — 详细设计 (`design.md`)

- **特性目录**：`docs/feature/lx-ai-web-grok/`
- **关联任务清单**：[`task.md`](task.md)
- **参考实现**：
  - [`docs/standards/web-deepseek.md`](../../standards/web-deepseek.md)
  - [`docs/feature/lx-ai-web/design.md`](../lx-ai-web/design.md)
  - [`docs/feature/lx-ai-web-gemini/design.md`](../lx-ai-web-gemini/design.md)
- **当前状态**：已完成实现、CLI 端到端验证与 Review 修复（见 [`review.md`](review.md)、[`verification.md`](verification.md)），已合并至 `dev` 并移除临时 worktree 与分支；`/rewind` 语义已根据 Grok 实测行为调整为“重试追加”
- **目标**：在 `LxEntry::AiWeb` 下新增第三个站点 `Grok`，保留与 DeepSeek / Gemini 对齐的基础 `/` 命令，并用 `lx-tui web` 做端到端验证。

---

## 0. 现状核实与事实基线（CodeGraph + Codebase Memory + ego-browser 实测）

| 维度 | 实测结果（2026-11-08） | 对本设计的影响 |
| --- | --- | --- |
| **站点枚举** | `src/app/state/lx/web.rs`：`AiWebSite` 仅 `Deepseek` / `Gemini`，`ALL: [AiWebSite; 2]`；`AiWebConfig` 只含 Gemini 与 DeepSeek 配置。 | 扩展为 `Deepseek` / `Gemini` / `Grok`，`ALL: [AiWebSite; 3]`，新增 `GrokWebModel` 与 `grok_model`。 |
| **首页站点行** | `src/ui/main_content/lx/brand.rs:214-300`：`place_ai_web_sites` 按 `AiWebSite::ALL` 动态计算宽度与命中矩形；`draw_ai_web_sites` 按品牌渐变逐字绘制，选中加粗并画下划线。 | 新增 Grok 标签与白到浅灰渐变（`232..=255` 灰阶序号插值）；窄窗格时把站点间隔降为 1 并用 `ai_web_site_label_for_width` 截取前缀字符，保证 8/10 列下三站点均可见且可区分。 |
| **会话前缀与路由** | `src/app/state/lx/web.rs:157-172`：`ai_web_remote_session` 依次识别 `ai-web-ds-` / `ai-web-gm-`；`src/chat/web/mod.rs:19-24`：`scripts_for_site` 二选一。 | 新增 `ai-web-grok-` 前缀与 `GROK_HELPERS_SCRIPT` / `GROK_SCRIPT`。 |
| **脚本参数** | `src/chat/web/mod.rs:170-225`：`ScriptParams` 已含 `model` / `thinking` / `files` / `keep_files` / `session_id` / `edit_user_index`。 | Rust 协议层零改动；Grok 脚本只读 `MODEL`，忽略 `THINKING`。 |
| **状态栏与 `/model`** | `src/app/update/chat/web.rs:26-49` 与 `src/app/update/chat/slash.rs:448-478`：按站点分支。 | 增加 `Grok` 分支：状态栏 `Grok · Fast`，无 variant，单级 `/model`。 |
| **Grok 登录态** | `GET /rest/auth/get-user` 返回 200 且 `email` 非空即已登录；匿名访客也会自动建 `userId`，只看 cookie 会误判。 | 登录判定必须看 `email`，未登录走 `handOffTaskSpace()`。 |
| **Grok 会话 REST** | 列表：`GET /rest/app-chat/conversations?pageSize=60&excludeProjects=true`；历史：`GET /rest/app-chat/conversations/{id}/responses`；改名：`PUT /rest/app-chat/conversations/{id}` body `{title}`；删除：`DELETE /rest/app-chat/conversations/{id}`。均已实测 200。 | `/resume`、`/rename`、`/delete` 全部走 REST，不依赖侧栏菜单的 hover/动画。 |
| **Grok 流式输出** | 发送后走 **WebSocket**，帧内带 `castle_request_token` 反爬令牌；Network 层无法像 DeepSeek 那样拦截 HTTP SSE。 | 不伪造 WebSocket；一律通过 UI 输入 + 发送按钮，正文用 DOM 轮询采集。 |
| **Grok DOM** | 输入：`[contenteditable="true"].ProseMirror`（tiptap）；发送：`button[data-testid="chat-submit"]`（`aria-label="提交"/"Submit"`）；停止：`button[aria-label="停止模型响应"]`；用户消息：`[data-testid="user-message"]`；助手消息：`[data-testid="assistant-message"]`；正文：`.response-content-markdown`。 | 驱动脚本全部基于稳定 `data-testid` 与 `aria-label`，不依赖 Tailwind class。 |
| **Grok 模型菜单** | `button[aria-label^="型号:"]` 打开 `[role="menuitemradio"]` 列表：`Fast`（默认）、`Build`；`Auto` / `Expert` / `Heavy` 标注“需要 SuperGrok”。 | 只暴露 `Fast` / `Build`；`Build` 可选中但按 best-effort 支持。 |
| **Grok 思考内容** | `.thinking-container` 只有 `button[data-testid="canvas-trigger"]`，文案为“工作了 4s”，不是可展开的推理文本。 | 不显示 `Thinking` 块，只推正文；不伪造思考。 |
| **Grok 编辑态** | 点击用户消息内 `button[aria-label="Edit"]` 后，底部 ProseMirror 清空；提交后 Grok **清空旧用户消息正文并追加一条新用户消息 + 新回答**，不是原地替换（实测 `/responses` 为线性追加）。 | `/rewind` 不能做原地替换：Grok 改为“重试追加”——保留旧轮次、回填最后一条用户文本，下一次发送作为新轮次追加；带附件轮次明确拒绝。 |
| **Grok 附件** | 隐藏 `input[type="file"]`（multiple）位于 `[data-testid="drop-ui"]`；附件 chip 容器实际在输入框 `<form>` 内（**不在** `drop-ui` 里），选择器 `form [role="list"][aria-label*="对话附件"]`（英文 `*attachments*`），单项 `[role="listitem"]`，移除按钮 `button[aria-label="移除此附件"]` / `*Remove*`。CDP `DOM.setFileInputFiles` 实测可用，chip 约 200ms 出现。 | 普通发送沿用 `syncContainerAttachments` 思路；清理必须限定在输入框 `form` 内，避免误删消息气泡附件。隐藏 input 需轮询最多 10s，chip 等待最多 60s。 |
| **Cookie 弹窗** | `#onetrust-consent-sdk` 会挡住页面；`.ot-pc-refuse-all-handler` 可关闭。 | 每次进入页面先尝试关闭，失败不阻塞。 |
| **Grok 会话 URL** | `https://grok.com/c/<uuid>`，可能带 `?rid=<uuid>`。 | 会话绑定用 `/c/([0-9a-f-]{8,})/` 提取，忽略 query。 |
| **冷启动/慢网络（最新提交 `d6e8bada`）** | DeepSeek 增加 `ensureDeepSeekComposer()` 20s 轮询；Gemini 登录等待 20s、模式菜单 15s 轮询，避免把“尚未渲染”误判成“元素不存在”。 | Grok 登录检测通过后再等 composer；非 chat 的 REST 动作不等待 composer，避免冷启动拖满 CLI 预算；模型切换前轮询模型按钮/菜单项，禁止单次查询直接报错。 |

---

## 1. Grill Me 已确认决策

| 编号 | 决策维度 | 确认结论 |
| --- | --- | --- |
| **D1** | `/model` 暴露范围 | 只列 `Fast`（默认）与 `Build`；不设第二级思考等级；`Auto` / `Expert` / `Heavy` 因当前账号需要 SuperGrok，不进入候选。 |
| **D2** | 未登录处理 | 必须登录；`GET /rest/auth/get-user` 200 且 `email` 非空才算已登录；未登录打开 `https://grok.com/` 并 `handOffTaskSpace()`，Toast `Please sign in to Grok in ego-browser`。 |
| **D3** | `/rewind` 语义 | Grok 的 Edit 会清空旧用户消息并追加新轮次，无法原地替换。因此 `/rewind` 改为“重试追加”：保留旧轮次、回填最后一条用户文本，下一次发送作为新轮次；Toast 明确提示 `Grok /rewind appends a retry turn; use /new for a clean chat`。带附件的最新用户轮次仍拒绝，提示 `Grok /rewind cannot restore attachments; use /new or send without files`。CLI `--edit-index` 对 Grok 降级为普通追加，输出 `editIndex: null`。 |
| **D4** | 首页顺序与品牌色 | 顺序 `DeepSeek · Gemini · Grok`；Grok 使用 `Indexed(255)` → `Indexed(250)` 白到浅灰渐变，选中加粗 + 下划线。 |
| **D5** | 思考块 | Grok 不显示 `Thinking`，只显示正文；不把“工作了 Ns”伪造成思考内容。 |
| **D6** | 验证范围 | `lx-tui web --site grok` 端到端跑通 `models` / `chat` / `list` / `show` / `rename` / `new` / `stop` / `delete` / `chat --edit-index 0`；测试会话用完即删；TUI 视觉交互由用户 `cargo run` 目测。 |

---

## 2. 模块结构与变更文件清单

```text
src/
  app/
    state.rs                      重导出 GrokWebModel / AI_WEB_GROK_SESSION_PREFIX
    state/
      lx/web.rs                   [改] AiWebSite::Grok、GrokWebModel、AiWebConfig.grok_model、
                                        ai-web-grok- 前缀、ai_web_remote_session、ModelPicker::for_ai_web
      lx/slash.rs                 无逻辑改动（AI_WEB_COMMANDS 白名单已包含 /model /new /resume /rewind /rename）
    update/
      pane.rs                     无改动（set_ai_web_site 对三站点通用）
      chat/
        mod.rs                    [改] 重导出 AI_WEB_GROK_SESSION_PREFIX
        web.rs                    [改] Grok 状态栏、助手标签、/rewind 附件拒绝、前缀重导出
        slash.rs                  [改] Grok /model 单级候选、variant_keys 返回空、finish_model_selection 写 grok_model
        web/tests.rs              [改] Grok 站点切换、/model、/rewind 附件拒绝、状态栏测试
  chat/
    web/
      mod.rs                      [改] 嵌入 GROK_HELPERS_SCRIPT / GROK_SCRIPT，scripts_for_site 三分支
      grok_helpers.js             [新增] Grok 网页驱动辅助：登录、Cookie、模型、附件、会话 REST、动作分发
      grok_script.js              [新增] Grok 网页对话主流程：DOM 轮询流、普通发送、/rewind 重试追加
      tests.rs                    [改] Grok 脚本常量与 bundled 关键字测试
  cli/
    web.rs                        [改] --site grok/gk、site_key、resolve_model_and_thinking
    web/exec.rs                   [改] models 输出 Grok 条目
    help.rs                       [改] --site <deepseek|gemini|grok> 与 Grok 说明
    tests/args_and_help_tests.rs  [改] Grok 参数与 help 快照
  ui/
    text.rs                       [改] AI_WEB_SITE_GROK、Grok 模型标签、登录/回退提示、ai_web_site_label /
                                        ai_web_model_label / lx_ai_web_login_required_for 三分支
    style/lx.rs                   [改] ai_web_site_gradient 增加 Grok 白灰渐变
    main_content/lx/tests.rs      [改] 三站点命中、间距、渐变与下划线测试
docs/
  feature/lx-ai-web-grok/
    design.md                     [本文件]
    task.md                       [执行清单]
```

**不新增数据库迁移**：Grok 与 DeepSeek / Gemini 一样是纯内存会话 + 云端同步，不写 SQLite。

---

## 3. 核心设计详解

### 3.1 站点枚举、模型与配置（`src/app/state/lx/web.rs`）

#### 关键点

1. `AiWebSite::ALL` 扩为三元素，顺序固定 `Deepseek`、`Gemini`、`Grok`。
2. 新增 `AI_WEB_GROK_SESSION_PREFIX = "ai-web-grok-"`；`ai_web_remote_session` 增加 Grok 分支。
3. 新增 `GrokWebModel { Fast, Build }`，`Fast` 为默认值；`id()` 复用 `text::GROK_WEB_MODEL_*`。
4. `AiWebConfig` 增加 `grok_model`；`active_model_and_thinking` 对 Grok 返回 `(grok_model.id(), "")`。
5. `ModelPicker::for_ai_web` 对 Grok 构造两个 `ModelCandidate`（provider 为 `"Grok"`），`current` 指向当前 `grok_model`。

#### 关键代码

```rust
/// 映射自 Grok 云端真实会话的 ID 前缀。
pub const AI_WEB_GROK_SESSION_PREFIX: &str = "ai-web-grok-";

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum AiWebSite {
    #[default]
    Deepseek,
    Gemini,
    Grok,
}

impl AiWebSite {
    pub const ALL: [AiWebSite; 3] =
        [AiWebSite::Deepseek, AiWebSite::Gemini, AiWebSite::Grok];

    pub fn remote_session_prefix(self) -> &'static str {
        match self {
            Self::Deepseek => AI_WEB_DEEPSEEK_SESSION_PREFIX,
            Self::Gemini => AI_WEB_GEMINI_SESSION_PREFIX,
            Self::Grok => AI_WEB_GROK_SESSION_PREFIX,
        }
    }
}

/// Grok 网页端模型档位（对齐 `https://grok.com/` 的模型菜单）。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum GrokWebModel {
    #[default]
    Fast,
    Build,
}

impl GrokWebModel {
    pub const ALL: [GrokWebModel; 2] = [Self::Fast, Self::Build];

    pub fn id(self) -> &'static str {
        match self {
            Self::Fast => text::GROK_WEB_MODEL_FAST,
            Self::Build => text::GROK_WEB_MODEL_BUILD,
        }
    }

    pub fn from_id(id: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|m| m.id().eq_ignore_ascii_case(id.trim()))
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct AiWebConfig {
    pub gemini_model: GeminiWebModel,
    pub gemini_thinking: GeminiThinkingLevel,
    pub deepseek_thinking: DeepseekThinkingMode,
    pub grok_model: GrokWebModel,
}

impl AiWebConfig {
    pub fn active_model_and_thinking(self, site: AiWebSite) -> (&'static str, &'static str) {
        match site {
            AiWebSite::Deepseek => (text::AI_WEB_SITE_DEEPSEEK, self.deepseek_thinking.id()),
            AiWebSite::Gemini => (self.gemini_model.id(), self.gemini_thinking.id()),
            AiWebSite::Grok => (self.grok_model.id(), ""),
        }
    }
}
```

```rust
// ModelPicker::for_ai_web 的 Grok 分支
AiWebSite::Grok => {
    let list: Vec<ModelCandidate> = GrokWebModel::ALL
        .into_iter()
        .map(|model| ModelCandidate {
            selection: ModelSelection {
                provider: text::AI_WEB_SITE_GROK.to_string(),
                model: model.id().to_string(),
                variant: None,
            },
            provider_name: text::AI_WEB_SITE_GROK.to_string(),
            model_name: model.id().to_string(),
        })
        .collect();
    let current_idx = GrokWebModel::ALL
        .into_iter()
        .position(|model| model == config.grok_model);
    (list, current_idx)
}
```

> `/model` 单级生效路径：`src/app/update/chat/slash.rs:120-126` 已有 `if variants.is_empty() { finish_model_selection(...) }`，Grok 的 `variant_keys_for_pane` 返回空即可，不需要改面板状态机。

---

### 3.2 站点路由与 CLI（`src/chat/web/mod.rs`、`src/cli/web.rs`）

#### 关键点

1. `scripts_for_site` 增加 Grok 分支，嵌入两份新脚本。
2. `cli::web::parse_site` 接受 `grok` / `gk`；`site_key` 返回 `"grok"`。
3. `resolve_model_and_thinking` 对 Grok 只接受 `Fast` / `Build`；显式传 `--thinking` 直接报错，因为 Grok 无思考档位。
4. `exec_models` 增加 Grok 条目，`thinkingVariants` 为空数组，`defaultThinking` 为 `""`。

#### 关键代码

```rust
// src/chat/web/mod.rs
pub const GROK_HELPERS_SCRIPT: &str = include_str!("grok_helpers.js");
pub const GROK_SCRIPT: &str = include_str!("grok_script.js");

pub fn scripts_for_site(site: AiWebSite) -> (&'static str, &'static str) {
    match site {
        AiWebSite::Deepseek => (DEEPSEEK_HELPERS_SCRIPT, DEEPSEEK_SCRIPT),
        AiWebSite::Gemini => (GEMINI_HELPERS_SCRIPT, GEMINI_SCRIPT),
        AiWebSite::Grok => (GROK_HELPERS_SCRIPT, GROK_SCRIPT),
    }
}
```

```rust
// src/cli/web.rs
pub fn parse_site(raw: &str) -> Result<AiWebSite, String> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "deepseek" | "ds" => Ok(AiWebSite::Deepseek),
        "gemini" | "gm" => Ok(AiWebSite::Gemini),
        "grok" | "gk" => Ok(AiWebSite::Grok),
        other => Err(format!(
            "invalid '--site' value '{other}'; expected 'deepseek', 'gemini', or 'grok'"
        )),
    }
}

pub fn site_key(site: AiWebSite) -> &'static str {
    match site {
        AiWebSite::Deepseek => "deepseek",
        AiWebSite::Gemini => "gemini",
        AiWebSite::Grok => "grok",
    }
}

// resolve_model_and_thinking 的 Grok 分支
AiWebSite::Grok => {
    let model = match model_opt {
        Some(raw) => GrokWebModel::from_id(raw)
            .ok_or_else(|| {
                format!(
                    "invalid '--model' value '{}' for Grok; expected 'Fast' or 'Build'",
                    raw.trim()
                )
            })?
            .id()
            .to_string(),
        None => GrokWebModel::default().id().to_string(),
    };
    if thinking_opt.is_some() {
        return Err("Grok has no thinking variants; omit '--thinking'".to_string());
    }
    Ok((model, String::new()))
}
```

---

### 3.3 底部状态栏、助手标签与 `/model` 写入（`src/app/update/chat/`）

#### 关键点

1. `apply_ai_web_status` 的 Grok 分支：`model_name = "Grok · Fast"` / `"Grok · Build"`，`variant = None`。
2. `ai_web_assistant_label` 的 Grok 分支返回同一标签，保证气泡顶部与状态栏一致。
3. `variant_keys_for_pane` 对 Grok 返回空 `Vec`；`current_variant_index_for_pane` 返回 `None`。
4. `finish_model_selection` 对 Grok 只写 `ai_web_config.grok_model`，不写 `ai.json`、不写 SQLite、不注入 `<system-update>`。

#### 关键代码

```rust
// src/app/update/chat/web.rs
pub fn apply_ai_web_status(lx: &mut Lx) {
    match lx.ai_web_site {
        AiWebSite::Deepseek => { /* 原逻辑 */ }
        AiWebSite::Gemini => { /* 原逻辑 */ }
        AiWebSite::Grok => {
            lx.chat.status.model_name = Some(text::ai_web_model_label(
                AiWebSite::Grok,
                lx.ai_web_config.grok_model.id(),
            ));
            lx.chat.status.variant = None;
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
fn variant_keys_for_pane(...) -> Vec<String> {
    if target.lx.entry == LxEntry::AiWeb {
        let site = AiWebSite::from_label(&selection.provider).unwrap_or(target.lx.ai_web_site);
        return match site {
            AiWebSite::Gemini => GeminiThinkingLevel::ALL.iter().map(|l| l.id().to_string()).collect(),
            AiWebSite::Deepseek => DeepseekThinkingMode::ALL.iter().map(|m| m.id().to_string()).collect(),
            AiWebSite::Grok => Vec::new(),
        };
    }
    // ...
}

// finish_model_selection 的 AI Web 分支
AiWebSite::Grok => {
    if let Some(model) = GrokWebModel::from_id(&selection.model) {
        target.lx.ai_web_config.grok_model = model;
    }
}
```

---

### 3.4 `/rewind` 语义与附件拒绝（`src/app/update/chat/web.rs`）

#### 关键点

Grok 的 Edit 实测是“清空旧用户消息正文并追加新轮次”，无法原地替换；因此 Grok `/rewind` 采用**重试追加**：

- 带附件的最新用户轮次：清空 `slash`，弹出 Warning Toast，消息与输入内容都不改；
- 纯文本轮次：**不截断旧消息**，回填最后一条用户文本，`ai_web_edit_user_index = None`，下一次发送走普通追加；
- 同时弹出 Warning Toast `Grok /rewind appends a retry turn; use /new for a clean chat`，明确旧轮次会保留。

这样本地消息与云端线性历史始终一致；`/new` 是获得干净会话的路径。

#### 关键代码

```rust
pub fn rewind_ai_web(state: &mut AppState, pane: PaneId) -> bool {
    // ... 原有 busy / last_user_idx 检查 ...
    if target.lx.ai_web_site == AiWebSite::Grok
        && !target.lx.chat.messages[last_user_idx].attachments.is_empty()
    {
        // 带附件：拒绝，消息与输入都不改。
        return true;
    }
    if target.lx.ai_web_site == AiWebSite::Grok {
        let user_text = target.lx.chat.messages[last_user_idx].text();
        {
            let Some(target) = state.pane_mut_anywhere(pane) else {
                return false;
            };
            target.lx.ai_web_edit_user_index = None; // 下一次走普通追加
            target.lx.ai_web_edit_cloud_files.clear();
            target.lx.clear_rewind();
            target.lx.slash = None;
            target.lx.replace_input(&user_text);
            target.lx.chat.follow_bottom = true;
        }
        crate::app::update::clear_lx_selection_for(state, pane);
        super::session::show_lx_hint(
            state,
            pane,
            InputHintLevel::Warning,
            text::LX_AI_WEB_GROK_REWIND_RETRY,
        );
        super::refresh_lx_status(state, pane);
        return true;
    }
    // ... 其它站点走原截断 + 原地编辑逻辑 ...
}
```

---

### 3.5 Grok 网页驱动：登录、模型、动作分发（`src/chat/web/grok_helpers.js`）

#### 关键点

1. **任务空间**：`SPACE = 'lx-tui-ai-web-grok'`，主页 `HOME = 'https://grok.com/'`。
2. **输出协议**：复用 `emit` / `emitMany` / `checkStreamAlive` / `reclaimSpaceTarget` / `activateTaskSpace` / `ensureTaskSpaceControl`。
3. **冷启动/慢网络等待**（对齐最新提交 `d6e8bada`）：Grok 是 tiptap/React 应用，标签页 URL 与登录态可能先于输入框、模型按钮挂载；`ensureGrokComposer()` 轮询 20s 等 `[contenteditable="true"].ProseMirror`，`ensureGrokModel()` 轮询 15s 等模型按钮，禁止单次查询就直接报 “not found”。
4. **Cookie 弹窗**：`dismissCookieBanner()` 点击 `.ot-pc-refuse-all-handler`，失败忽略。
5. **登录检测**：`fetch('/rest/auth/get-user')`，200 且 `email` 非空才算已登录；登录检测最多轮询 20s。**不要**因为页面上出现普通 `sign-in` 链接就提前判未登录（实测会把已登录页误判）；未登录 `handOffTaskSpace()` → `login_required`。
6. **模型同步**：`ensureGrokModel()` 点击 `button[aria-label^="型号"]` / `button[aria-label^="Model"]`，在 `[role="menuitemradio"]` 中按文本匹配 `MODEL`，检查 `aria-checked`，切换后校验按钮标签。
7. **附件同步**：`syncComposerAttachments()` 先删除输入框 `form [role="list"][aria-label*="对话附件"]` / `*attachments*` 内所有 `[role="listitem"]`（`button[aria-label="移除此附件"]` / `*Remove*`），再用 CDP `DOM.setFileInputFiles` 注入 `FILES`；隐藏 input 最多轮询 10s，chip 最多等待 60s（实测 chip 约 200ms 出现，但冷启动/上传处理可能更慢）。
8. **会话动作**：`new` / `stop` / `list_sessions` / `load_session` / `rename_session` / `delete_session` 全部实现；所有 REST 调用必须检查 `res.ok`，401 回 `login_required` 并移交任务空间，其它失败回 `error`，不得静默。
9. **引用标签清洗**：`/responses` 的 `message` 会把网页端引用以 `<grok:render card_id="…" ...><argument name="citation_id">N</argument></grok:render>` 原样返回；`load_session` 必须用同条 response 的 `cardAttachmentsJson` 把 `card_id` 映射成 URL，替换为 `[来源名](url)`，并清掉无映射的残留标签。实时 DOM 正常已渲染为来源名，但采集器仍要兜底剥离原始标签/部分标签。

#### 关键代码

```javascript
const SPACE = 'lx-tui-ai-web-grok';
const HOME = 'https://grok.com/';
const MODEL = PARAMS.model || 'Fast';
const THINKING = PARAMS.thinking || '';   // Grok 无思考档位，保留字段仅为协议对齐

async function dismissCookieBanner() {
  await js(String.raw`
    (() => {
      const reject = document.querySelector('.ot-pc-refuse-all-handler');
      if (!reject) return false;
      const style = getComputedStyle(reject);
      if (style.display === 'none' || style.visibility === 'hidden') return false;
      reject.click();
      return true;
    })()
  `).catch(() => false);
}

async function grokLoginState() {
  return await js(String.raw`
    (async () => {
      try {
        const res = await fetch('/rest/auth/get-user', { credentials: 'include' });
        if (!res.ok) return { ok: false, status: res.status };
        const user = await res.json();
        const email = Boolean(user && user.email);
        return { ok: email, anonymous: !email, status: res.status };
      } catch (error) {
        return { ok: false, status: -1, message: String((error && error.message) || error) };
      }
    })()
  `).catch((error) => ({ ok: false, status: -1, message: String((error && error.message) || error) }));
}

async function ensureGrokComposer() {
  // tiptap/React 冷启动或慢网络：URL 与登录态可能先于 ProseMirror 挂载。
  const deadline = Date.now() + 20000;
  while (Date.now() < deadline) {
    const ready = await js(
      `Boolean(document.querySelector('[contenteditable="true"].ProseMirror, [contenteditable="true"]'))`
    ).catch(() => false);
    if (ready) return true;
    await new Promise((r) => setTimeout(r, 200));
  }
  return false;
}

async function ensureGrokModel() {
  const want = String(MODEL || 'Fast').trim().toLowerCase();
  const matches = (value) =>
    String(value || '').trim().toLowerCase().split(/\s+/)[0] === want; // Build\nBeta

  const current = await js(`/* 读取 composer form 内模型按钮文本 */`).catch(() => '');
  if (matches(current)) return;

  const deadline = Date.now() + 15000;
  let picked = null;
  while (Date.now() < deadline) {
    picked = await js(`
      (() => {
        const buttons = Array.from(document.querySelectorAll('button[aria-label^="型号"], button[aria-label^="Model"]'));
        const btn = buttons.find((b) => b.closest('form')) || buttons[0];
        if (!btn) return { found: false };
        if (btn.getAttribute('aria-expanded') !== 'true') btn.click();
        const items = Array.from(document.querySelectorAll('[role="menuitemradio"]'));
        const match = items.find((item) => (item.innerText || '').trim().toLowerCase() === ${JSON.stringify(want)});
        if (!match) return { found: false };
        if (match.getAttribute('aria-checked') !== 'true') match.click();
        return { found: true };
      })()
    `).catch(() => ({ found: false }));
    if (picked.found) break;
    await new Promise((r) => setTimeout(r, 200));
  }
  if (!picked || !picked.found) throw new Error('Grok model "' + MODEL + '" not found');
  await new Promise((r) => setTimeout(r, 350));
  const label = await js(`/* 读取模型按钮文本 */`).catch(() => '');
  if (!matches(label)) throw new Error('Grok model "' + MODEL + '" was not applied');
}
```

```javascript
async function syncComposerAttachments() {
  // 普通发送：输入区不应残留上一轮附件。
  await js(String.raw`
    (() => {
      const lists = Array.from(document.querySelectorAll(
        'form [role="list"][aria-label*="对话附件"], form [role="list"][aria-label*="attachments"], form [role="list"][aria-label*="Attachments"]'
      ));
      for (const list of lists) {
        for (const item of list.querySelectorAll('[role="listitem"]')) {
          const remove = item.querySelector(
            'button[aria-label="移除此附件"], button[aria-label*="Remove attachment"], button[aria-label*="Remove"]'
          );
          if (remove) remove.click();
        }
      }
      return true;
    })()
  `).catch(() => {});
  if (FILES.length === 0) return;

  // 隐藏 input 可能晚挂载：实现里最多轮询 10s 再取 nodeId。
  const doc = await cdp('DOM.getDocument', { depth: -1 });
  const queried = await cdp('DOM.querySelectorAll', {
    nodeId: doc.root.nodeId,
    selector: '[data-testid="drop-ui"] input[type="file"]',
  });
  const nodeId = (queried.nodeIds || [])[0];
  if (!nodeId) throw new Error('Grok file input not found');
  await cdp('DOM.setFileInputFiles', { nodeId, files: FILES });

  const deadline = Date.now() + 60000;
  while (Date.now() < deadline) {
    await new Promise((r) => setTimeout(r, 200));
    const count = await js(`
      (() => document.querySelectorAll(
        'form [role="list"][aria-label*="对话附件"] [role="listitem"], form [role="list"][aria-label*="attachments"] [role="listitem"], form [role="list"][aria-label*="Attachments"] [role="listitem"]'
      ).length)
    `).catch(() => 0);
    if (count >= FILES.length) return;
  }
  throw new Error('Timed out waiting for Grok file upload');
}
```

```javascript
// list_sessions
const res = await fetch('/rest/app-chat/conversations?pageSize=60&excludeProjects=true', {
  credentials: 'include',
});
if (res.status === 401) {
  await emit({ event: 'login_required', url: HOME });
  return;
}
if (!res.ok) {
  await emit({ event: 'error', message: 'Grok session list failed (' + res.status + ')' });
  return;
}
const data = await res.json();
const items = (data.conversations || []).map((c) => ({
  id: String(c.conversationId || ''),
  title: String(c.title || 'New chat'),
  updated_at_ms: Date.parse(c.modifyTime || c.createTime || '') || 0,
}));
await emit({ event: 'sessions', items });

// load_session：重建当前活跃分支
const res = await fetch(
  '/rest/app-chat/conversations/' + encodeURIComponent(SESSION_ID) + '/responses',
  { credentials: 'include' }
);
if (res.status === 401) {
  await emit({ event: 'login_required', url: HOME });
  return;
}
if (!res.ok) {
  await emit({ event: 'error', message: 'Grok session load failed (' + res.status + ')' });
  return;
}
const data = await res.json();
const all = Array.isArray(data.responses) ? data.responses : [];
const byId = new Map(all.map((r) => [r.responseId, r]));
const chain = [];
let node = all[all.length - 1];
while (node) {
  chain.push(node);
  node = node.parentResponseId ? byId.get(node.parentResponseId) : null;
}
chain.reverse();
const messages = chain.map((r) => ({
  role: r.sender === 'human' ? 'user' : 'assistant',
  thinking: '',
  thinking_ms: null,
  text: String(r.message || ''),
  files: r.sender === 'human'
    ? (r.fileAttachmentsMetadata || []).map((f) => String(f.fileName || '')).filter(Boolean)
    : [],
  created_at_ms: Date.parse(r.createTime || '') || 0,
}));
```

```javascript
// rename_session
const res = await fetch('/rest/app-chat/conversations/' + encodeURIComponent(SESSION_ID), {
  method: 'PUT',
  credentials: 'include',
  headers: { 'content-type': 'application/json' },
  body: JSON.stringify({ title: TITLE }),
});
if (!res.ok) {
  await emit({ event: 'error', message: 'Grok rename failed (' + res.status + ')' });
  return;
}

// delete_session
const res = await fetch('/rest/app-chat/conversations/' + encodeURIComponent(SESSION_ID), {
  method: 'DELETE',
  credentials: 'include',
});
if (!res.ok) {
  await emit({ event: 'error', message: 'Grok delete failed (' + res.status + ')' });
  return;
}
if (currentUrl.includes(SESSION_ID)) {
  await openOrReuseTab(HOME, { wait: true, timeout: 20 });
}
```

---

### 3.6 Grok 对话主流程与 DOM 流采集（`src/chat/web/grok_script.js`）

#### 关键点

1. **登录/动作分流**：先 `activateTaskSpace` → 导航到 `HOME` → `dismissCookieBanner` → 登录检测（401/200 匿名才移交，5xx/网络错误回 `error`）→ 非 `chat` 走 `runNonChatAction` → chat 才等 composer。
2. **新会话**：`FIRST_TURN && EDIT_INDEX < 0` 时 `ensureFreshChat()`；已有 `SESSION_ID` 且 URL 不匹配时导航到 `HOME + 'c/' + SESSION_ID`。
3. **模型同步**：发送前 `ensureGrokModel()`；按钮已是目标模型（允许 `Build` 附带的 `Beta` 文本）直接返回，否则在 15s 内轮询菜单项，失败直接 `error`，不静默沿用旧模型。
4. **普通发送**：`syncComposerAttachments()` 循环删到 0 个 chip，再注入本轮文件并等待 `chips === FILES.length`；ProseMirror `focus()` + `execCommand('selectAll')` + `insertText`（空 prompt 用 `delete` 清空旧草稿）；提交后 5s 内要求 user/assistant 数量增加或出现有效停止按钮，否则立即报错。
5. **`/rewind` 重试追加**：TUI/CLI 对 Grok 不再传 `EDIT_INDEX`；Rust 层把 `/rewind` 降级为“回填最后一条用户文本 + 普通追加”。脚本保留 `EDIT_INDEX` 兼容分支但显式拒绝附件和越界，采集器不得盯 `EDIT_INDEX` 对应的旧回答。
6. **DOM 流采集**：在 `window.__lxGrokStream` 上保存 `baselineAssistants` / `lastText` / `queue` / `stableTicks`；每 60ms 轮询发送后新增的最后一个 `[data-testid="assistant-message"]`，只读 `.response-content-markdown`，用“尾部锚点对齐”计算增量。
7. **完成判定**：有效停止按钮（过滤 disabled/`aria-disabled`）消失 + 文本连续 30 个 tick（约 1.8s）不变；助手节点出现但 15s 无正文时显式报 `Grok response has no text`，不空等 300s。不再把 `chat-submit` 作为完成信号（空 composer 时该按钮会消失）。
8. **会话绑定**：完成后从 `location.pathname.match(/\/c\/([0-9a-f-]{8,})/)` 取 ID；最多等 5s，标题优先侧栏 `a[href^="/c/"]` 文本（过滤 `New chat` 占位），其次 `document.title` 去掉 ` - Grok` 后缀。
9. **思考块**：不采集、不 emit `thinking`。

#### 关键代码

```javascript
async function main() {
  await activateTaskSpace();
  const current = await pageInfo().catch(() => null);
  const currentUrl = (current && current.url) || '';
  let onGrok = false;
  try {
    const parsed = new URL(currentUrl);
    onGrok = parsed.protocol === 'https:' && parsed.hostname === 'grok.com';
  } catch (_) {}
  if (!onGrok) {
    await openOrReuseTab(HOME, { wait: true, timeout: 25 });
  }
  await dismissCookieBanner();

  // 登录只信 API；401/200 匿名才是未登录，5xx/网络错误不能伪装成登录墙。
  let loginState = { ok: false, status: -1 };
  const loginDeadline = Date.now() + 20000;
  while (Date.now() < loginDeadline) {
    loginState = await grokLoginState();
    if (loginState.ok) break;
    if (loginState.status === 401 || loginState.status === 200) break;
    await new Promise((r) => setTimeout(r, 300));
  }
  if (!loginState.ok) {
    if (loginState.status === 401 || loginState.status === 200) {
      await emitLoginRequired();
    } else {
      await emit({ event: 'error', message: 'Grok auth check failed (' + httpDetail(loginState) + ')' });
    }
    return;
  }

  // 非 chat 的 REST 动作不等待 composer，避免冷启动拖满 CLI 预算。
  if (ACTION !== 'chat') {
    await runNonChatAction(currentUrl);
    return;
  }
  if (!(await ensureGrokComposer())) {
    await emit({ event: 'error', message: 'Grok composer not found' });
    return;
  }

  if (FIRST_TURN && EDIT_INDEX < 0) {
    await ensureFreshChat();
    if (!(await ensureGrokComposer())) {
      await emit({ event: 'error', message: 'Grok composer not found after /new' });
      return;
    }
  } else if (SESSION_ID && !currentUrl.includes(SESSION_ID)) {
    await openOrReuseTab(HOME + 'c/' + SESSION_ID, { wait: true, timeout: 25 });
    if (!(await ensureGrokComposer())) {
      await emit({ event: 'error', message: 'Grok composer not found for session' });
      return;
    }
  }

  try {
    await ensureGrokModel();
  } catch (error) {
    await emit({ event: 'error', message: String((error && error.message) || error) });
    return;
  }

  // 安装 DOM 采集器：目标固定为最后一条助手消息。
  const runId = String(Date.now()) + '-' + String(Math.random());
  await js(`
    (() => {
      window.__lxGrokStream = {
        runId: ${JSON.stringify(runId)},
        queue: [],
        done: false,
        started: false,
        lastText: '',
        lastSample: '',
        stableTicks: 0,
        pushText(current) {
          if (!current) return;
          const previous = this.lastText || '';
          if (current === previous) return;
          if (!previous) {
            this.lastText = current;
            this.started = true;
            this.queue.push({ kind: 'text', delta: current });
            return;
          }
          if (current.startsWith(previous)) {
            const delta = current.slice(previous.length);
            this.lastText = current;
            if (delta) {
              this.started = true;
              this.queue.push({ kind: 'text', delta });
            }
            return;
          }
          // 流式 Markdown 重渲染：用尾部锚点在 current 中重新对齐。
          for (const len of [48, 32, 20, 12, 8, 5]) {
            if (previous.length < len) continue;
            const anchor = previous.slice(previous.length - len);
            const pos = current.lastIndexOf(anchor);
            if (pos >= 0 && pos + len >= current.length - 240) {
              const delta = current.slice(pos + len);
              this.lastText = current;
              if (delta) {
                this.started = true;
                this.queue.push({ kind: 'text', delta });
              }
              return;
            }
          }
          if (current.length > previous.length) this.lastText = current;
        },
        pollDom() {
          const messages = Array.from(document.querySelectorAll('[data-testid="assistant-message"]'));
          const last = messages[messages.length - 1];
          const stop = document.querySelector('button[aria-label="停止模型响应"], button[aria-label*="Stop model"]');
          const generating = Boolean(stop);
          if (generating) this.started = true;
          if (!last) return;
          const body = last.querySelector('.response-content-markdown');
          // 只读正文容器：助手消息头部的 sr-only "Grok" 标题不能回退成回答。
          const text = body
            ? (body.innerText || '').replace(/\\r\\n/g, '\\n').trim()
            : '';
          if (text) this.pushText(text);
          const sample = text;
          const idle = !generating;
          if ((this.started || text.length > 0) && idle) {
            if (sample === this.lastSample && text.length > 0) {
              this.stableTicks += 1;
              if (this.stableTicks >= 10) this.done = true;
            } else {
              this.lastSample = sample;
              this.stableTicks = 0;
            }
          } else {
            this.lastSample = sample;
            this.stableTicks = 0;
          }
        },
      };
      return true;
    })()
  `);

  // 提交：普通发送。TUI/CLI 的 Grok /rewind 已在 Rust 层降级为普通追加；
  // 下面的 EDIT_INDEX 分支仅保留兼容，不能再依赖它做原地替换。
  await ensureTaskSpaceControl();
  if (EDIT_INDEX >= 0) {
    const clicked = await js(`
      (() => {
        const users = Array.from(document.querySelectorAll('[data-testid="user-message"]'));
        const target = users[${EDIT_INDEX}];
        if (!target) return false;
        target.scrollIntoView({ block: 'center' });
        const edit = target.parentElement.querySelector('button[aria-label="Edit"]')
          || target.querySelector('button[aria-label="Edit"]');
        if (!edit) return false;
        edit.click();
        return true;
      })()
    `).catch(() => false);
    if (!clicked) {
      await emit({ event: 'error', message: 'Grok edit button not found for /rewind' });
      return;
    }
    await new Promise((r) => setTimeout(r, 500));
    const filled = await js(`
      (() => {
        const editor = document.querySelector('[contenteditable="true"].ProseMirror, [contenteditable="true"]');
        if (!editor) return false;
        editor.focus();
        document.execCommand('selectAll', false, null);
        document.execCommand('insertText', false, ${JSON.stringify(PROMPT)});
        editor.dispatchEvent(new Event('input', { bubbles: true }));
        return true;
      })()
    `).catch(() => false);
    if (!filled) {
      await emit({ event: 'error', message: 'Grok edit composer not found' });
      return;
    }
    await new Promise((r) => setTimeout(r, 300));
    const submitted = await js(`
      (() => {
        const btn = document.querySelector('button[data-testid="chat-submit"]');
        if (!btn) return false;
        btn.click();
        return true;
      })()
    `).catch(() => false);
    if (!submitted) {
      await emit({ event: 'error', message: 'Failed to submit edited Grok prompt' });
      return;
    }
  } else {
    await syncComposerAttachments();
    const filled = await js(`
      (() => {
        const editor = document.querySelector('[contenteditable="true"].ProseMirror, [contenteditable="true"]');
        if (!editor) return false;
        editor.focus();
        document.execCommand('selectAll', false, null);
        document.execCommand('insertText', false, ${JSON.stringify(PROMPT)});
        editor.dispatchEvent(new Event('input', { bubbles: true }));
        return true;
      })()
    `).catch(() => false);
    if (!filled) {
      await emit({ event: 'error', message: 'Grok prompt input not found' });
      return;
    }
    await new Promise((r) => setTimeout(r, 250));
    const sent = await js(`
      (() => {
        const btn = document.querySelector('button[data-testid="chat-submit"]');
        if (!btn) return false;
        btn.click();
        return true;
      })()
    `).catch(() => false);
    if (!sent) {
      await emit({ event: 'error', message: 'Grok send button not available' });
      return;
    }
  }

  const deadline = Date.now() + 300000;
  let lastHeartbeat = Date.now();
  while (Date.now() < deadline && !STREAM_ABORTED) {
    await ensureTaskSpaceControl();
    const batch = await js(`
      (() => {
        const s = window.__lxGrokStream;
        if (!s || s.runId !== ${JSON.stringify(runId)}) {
          return { items: [], done: true, superseded: true };
        }
        s.pollDom();
        return { items: s.queue.splice(0, s.queue.length), done: s.done, superseded: false };
      })()
    `).catch(() => ({ items: [], done: false, superseded: false }));

    if (batch && batch.superseded) return;
    if (batch && batch.items && batch.items.length > 0) {
      lastHeartbeat = Date.now();
      await emitMany(batch.items.map((item) => ({ event: 'text', delta: item.delta })));
    } else if (Date.now() - lastHeartbeat >= 1000) {
      lastHeartbeat = Date.now();
      if (!(await checkStreamAlive())) return;
    }
    if (batch && batch.done) {
      const meta = await js(`
        (() => {
          const m = location.pathname.match(/\\/c\\/([0-9a-f-]{8,})/);
          if (!m) return null;
          const id = m[1];
          const link = Array.from(document.querySelectorAll('a[href^="/c/"]'))
            .find((a) => (a.getAttribute('href') || '').startsWith('/c/' + id));
          let title = link ? (link.innerText || '').trim().replace(/\\s+/g, ' ') : '';
          if (!title) {
            title = (document.title || '').replace(/\\s*-\\s*Grok$/i, '').trim();
          }
          return { id, title };
        })()
      `).catch(() => null);
      if (meta && meta.id) {
        await emit({ event: 'session', id: meta.id, title: meta.title || null });
      }
      await emit({ event: 'done' });
      return;
    }
    await new Promise((r) => setTimeout(r, 60));
  }
  await emit({ event: 'error', message: 'Grok stream timed out' });
}
```

> **不变量**：绝不构造或重放 Grok 的 WebSocket 帧。`castle_request_token` 由页面自身生成，我们只做 UI 驱动；这样既不需要破解反爬，也不会因令牌过期而崩。

---

## 4. 首页品牌渐变（`src/ui/style/lx.rs`）

```rust
pub fn ai_web_site_gradient(site: AiWebSite) -> (Color, Color) {
    match site {
        AiWebSite::Deepseek => (Color::Indexed(63), Color::Indexed(39)),
        AiWebSite::Gemini => (Color::Indexed(69), Color::Indexed(204)),
        // Grok / xAI 黑白品牌：近白到浅灰。
        AiWebSite::Grok => (Color::Indexed(255), Color::Indexed(250)),
    }
}
```

`place_ai_web_sites` / `draw_ai_web_sites` 原本按固定 3 列间隔布局；三站点后窄窗格必须把间隔降为 1，并在宽度不足时用 `ai_web_site_label_for_width` 取前 N 个字符（不用 `…`，否则 8 列的 Gemini/Grok 都会变成 `G…`）。宽 8 得到 `[2,2,2]`，宽 10 得到 `[3,3,2]`。

---

## 5. 测试设计

### 5.1 单元测试

| 文件 | 用例 |
| --- | --- |
| `src/chat/web/tests.rs` | `scripts_for_site(Grok)` 返回 `grok_helpers.js` / `grok_script.js`，包含 `lx-tui-ai-web-grok`、`chat-submit`、`__lxGrokStream`；`script_source` 对 Grok 正确序列化 `model`/`thinking`。 |
| `src/app/update/chat/web/tests.rs` | Grok 站点切换清空会话与 `/rewind`；Grok `/model` 只写 `grok_model`；带附件 `/rewind` 被拒绝；纯文本 `/rewind` 保留旧轮次并回填输入；状态栏 `Grok · Fast`。 |
| `src/ui/main_content/lx/tests.rs` | 三站点命中矩形顺序 `Deepseek` / `Gemini` / `Grok`；Grok 渐变端点色；选中下划线；窄窗格三等分不裁掉 Grok。 |
| `src/cli/tests/args_and_help_tests.rs` | `--site grok` / `gk` 解析；`--model Build`；`--thinking low` 对 Grok 报错；help 文本包含 `grok`。 |

### 5.2 端到端验证（必须执行）

```bash
cargo fmt
cargo clippy --all-targets -- -D warnings
cargo test
cargo build

# 1. 模型清单
./target/debug/lx-tui web models --site grok --json

# 2. 新会话首轮对话（记录返回的 sessionId）
./target/debug/lx-tui web chat --site grok "Reply with the single word OK."

# 3. 会话列表 / 历史
./target/debug/lx-tui web list --site grok
./target/debug/lx-tui web show ai-web-grok-<REMOTE_ID>

# 4. 改名
./target/debug/lx-tui web rename ai-web-grok-<REMOTE_ID> "LX Grok Test" --site grok

# 5. Grok 的 /rewind：Rust 层降级为普通追加；--edit-index 输出 null，旧轮次保留
./target/debug/lx-tui web chat --site grok --session ai-web-grok-<REMOTE_ID> --edit-index 0 "Reply with the single word RETRY."

# 6. 附件上传
./target/debug/lx-tui web chat --site grok --file /tmp/secret.txt "Reply with the secret word in the attachment."

# 7. 新建 / 停止
./target/debug/lx-tui web new --site grok
./target/debug/lx-tui web chat --site grok "Count from 1 to 200 in English words, one per line." &
sleep 4
./target/debug/lx-tui web stop --site grok

# 8. 删除测试会话
./target/debug/lx-tui web delete ai-web-grok-<REMOTE_ID> --site grok
```

验收标准：

- `chat` 输出流式正文并打印 `sessionId: ai-web-grok-<uuid>`；
- `list` / `show` / `rename` / `delete` 返回 200 且结果可见；
- `stop` 后长回答停止增长；
- 附件上传成功，回答能引用附件内容；
- Grok `--edit-index 0` 输出 `editIndex: null`，新回答追加且旧轮次保留（Grok 网页端不支持原地替换）；
- `show` 的标题来自会话列表 REST，不再为 `null`；
- 所有测试会话删除干净，用户原有会话不受影响。

---

## 6. 风险与边界不变量

1. **反爬令牌不变量**：Grok 发送走 WebSocket 且带 `castle_request_token`；驱动只能操作 UI，不得自行构造 WS 帧或复用旧帧。
2. **DOM 选择器漂移**：优先 `data-testid`（`chat-submit`、`user-message`、`assistant-message`、`drop-ui`、`canvas-trigger`），其次中英文 `aria-label` 双匹配；不要依赖 Tailwind class。
3. **冷启动/慢网络误报**（对齐 `d6e8bada`）：tiptap/React 外壳可能晚于标签页 `load` 事件渲染完成；`ensureGrokComposer()` 必须轮询 20s，`ensureGrokModel()` 必须轮询 15s，禁止单次查询就直接报 “not found”。
4. **流式重渲染**：Grok 的 streamdown 会在 Markdown 完成时重排 DOM，必须使用尾部锚点对齐，否则会出现重复段落。
5. **`Build` 模式 best-effort**：`Build` 可能打开 workspace/canvas 或工具卡片；正文仍从 `.response-content-markdown` 提取，若该模式不产出正文，脚本显式报错而不是空回答。
6. **附件边界**：普通发送支持文件/图片；带附件轮次 `/rewind` 拒绝；纯文本 `/rewind` 为重试追加；文件夹、Skill、MCP 仍由 `prepare_ai_web_message` 统一拦截。
7. **登录态误判**：匿名用户也有 `userId`，必须看 `email`；`get-user` 请求失败按未登录处理；**不要**因为页面出现普通 `sign-in` 链接就提前判未登录。
8. **Cookie 弹窗**：只点击拒绝按钮，不修改用户偏好；失败继续，不能用 `display:none` 绕过。
9. **无数据库迁移**：`ai-web-grok-*` 只存在内存；切换入口/站点时由现有 `set_lx_entry` / `set_ai_web_site` 清理。
10. **文件行数**：新增 JS 文件按 helpers/script 拆分，单个生产文件 `< 1000` 行；Rust 改动均为分支追加，不新建超大模块。
11. **正文提取**：只能读 `.response-content-markdown`；禁止回退到助手消息 `innerText`，否则正文挂载前会把 `sr-only` 的 “Grok” 标题当成回答（已实测踩坑）。
12. **附件 DOM**：chip 容器在输入框 `<form>` 内而不是 `drop-ui` 内；清理必须限定 `form`，隐藏 input 轮询 10s，chip 等待 60s。
13. **Grok Edit 不可原地替换**：Edit 提交会清空旧用户消息正文并追加新轮次；任何“EDIT_INDEX 对应旧回答”的采集逻辑都会超时。
14. **引用标签不得泄漏**：历史 `message` 原样带 `<grok:render>`，必须按 `cardAttachmentsJson` 映射为来源链接；实时采集要及时剥离完整标签和尾部未闭合片段，否则 TUI 会显示原始 XML。

---

## 7. 回滚方案

- 代码回滚：本次改动不涉及数据库迁移，直接回退 commit 即可；`ai-web-grok-*` 仅存在于运行时内存，不会污染 SQLite。
- 浏览器侧：删除任务空间 `lx-tui-ai-web-grok` 不影响 DeepSeek / Gemini 的任务空间；Grok 测试会话已在验证阶段删除。
