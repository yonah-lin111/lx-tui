# Grok 接入 Web 入口 — 执行任务清单 (`task.md`)

- **设计文档**：[`design.md`](design.md)
- **当前状态**：已执行并完成端到端验证，结果见 [`verification.md`](verification.md)
- **执行原则**：最小修改；不新增数据库迁移；生产文件 `< 1000` 行；每个阶段结束只跑受影响范围的校验，最终再跑全量。

---

## 阶段 0：执行前检查

- [ ] 确认工作区干净：`git status --short` 无未提交改动（本任务文档除外）。
- [ ] 确认 Grok 登录态：`ego-browser` 任务空间 `lx-tui-ai-web-grok` 可打开 `https://grok.com/` 且 `GET /rest/auth/get-user` 返回 `email`。
- [ ] 确认当前账号模型菜单只有 `Fast` / `Build` 可用；`Auto` / `Expert` / `Heavy` 标注 SuperGrok。
- [ ] 记录要保留的用户原有会话标题，避免验证误删（只删除本轮新建的测试会话）。

---

## 阶段 1：状态模型与站点枚举

### 1.1 `src/ui/text.rs`

- [ ] 新增常量：
  ```rust
  pub const AI_WEB_SITE_GROK: &str = "Grok";
  pub const GROK_WEB_MODEL_FAST: &str = "Fast";
  pub const GROK_WEB_MODEL_BUILD: &str = "Build";
  pub const LX_AI_WEB_GROK_LOGIN_REQUIRED: &str = "Please sign in to Grok in ego-browser";
  pub const LX_AI_WEB_GROK_REWIND_ATTACHMENTS: &str =
      "Grok /rewind cannot restore attachments; use /new or send without files";
  ```
- [ ] `ai_web_site_label` 增加 `AiWebSite::Grok => AI_WEB_SITE_GROK`。
- [ ] `ai_web_model_label` 增加 `Grok => format!("Grok · {model}")`（复用 `LX_CHAT_MODEL_SEP`）。
- [ ] `lx_ai_web_login_required_for` 增加 `Grok => LX_AI_WEB_GROK_LOGIN_REQUIRED`。

### 1.2 `src/app/state/lx/web.rs`

- [ ] 新增 `AI_WEB_GROK_SESSION_PREFIX = "ai-web-grok-"`。
- [ ] `AiWebSite` 增加 `Grok`；`ALL` 改为 `[Deepseek, Gemini, Grok]`。
- [ ] `remote_session_prefix` 增加 Grok 分支。
- [ ] 新增 `GrokWebModel { Fast, Build }`（`Default = Fast`），实现 `ALL` / `id` / `from_id`。
- [ ] `AiWebConfig` 增加 `pub grok_model: GrokWebModel`。
- [ ] `active_model_and_thinking` 增加 `Grok => (grok_model.id(), "")`。
- [ ] `ai_web_remote_session` 增加 `ai-web-grok-` 解析。
- [ ] `ModelPicker::for_ai_web` 增加 Grok 分支（2 个候选，`variant: None`）。

### 1.3 `src/app/state.rs` / `src/app/update/chat/mod.rs` / `src/app/update/chat/web.rs`

- [ ] 重导出 `GrokWebModel` 与 `AI_WEB_GROK_SESSION_PREFIX`。
- [ ] 编译检查：`cargo check`，确认所有 `match AiWebSite` 的穷尽性错误清单。

---

## 阶段 2：状态更新与 `/model`

### 2.1 `src/app/update/chat/web.rs`

- [ ] `apply_ai_web_status` 增加 Grok：`model_name = Some(ai_web_model_label(Grok, grok_model.id()))`，`variant = None`。
- [ ] `ai_web_assistant_label` 增加 Grok 模型名。
- [ ] `rewind_ai_web` 增加 Grok 两段逻辑：
  ```rust
  // 1) 带附件：拒绝，消息与输入不变
  if target.lx.ai_web_site == AiWebSite::Grok
      && !target.lx.chat.messages[last_user_idx].attachments.is_empty()
  {
      // 清 slash，弹 LX_AI_WEB_GROK_REWIND_ATTACHMENTS，原状态不变
      return true;
  }
  // 2) 纯文本：不截断旧轮次，回填最后一条用户文本，edit_user_index = None，
  //    下一次发送走普通追加；弹 LX_AI_WEB_GROK_REWIND_RETRY
  ```

### 2.2 `src/app/update/chat/slash.rs`

- [ ] `variant_keys_for_pane`：`Grok => Vec::new()`。
- [ ] `current_variant_index_for_pane`：Grok 返回 `None`。
- [ ] `finish_model_selection`：Grok 写 `target.lx.ai_web_config.grok_model`，不写 `ai.json` / SQLite。
- [ ] 确认单级生效：`variants.is_empty()` 时直接 `finish_model_selection`，不进入空的第二级。

### 2.3 测试

- [ ] `src/app/update/chat/web/tests.rs` 增加：
  - `grok_single_stage_model_picker_applies_fast_and_build_without_variants`
  - `grok_rewind_with_attachments_is_rejected_and_keeps_messages`
  - `grok_text_rewind_restores_prompt_without_truncating_and_next_send_appends`
  - `switching_ai_web_site_isolates_sessions_and_clears_rewind_state` 补充 Grok 前缀
- [ ] 运行：`cargo test app::update::chat::web::tests -- --nocapture`。

---

## 阶段 3：CLI 与脚本路由

### 3.1 `src/chat/web/mod.rs`

- [ ] 嵌入 `GROK_HELPERS_SCRIPT` / `GROK_SCRIPT`。
- [ ] `scripts_for_site` 增加 Grok 分支。

### 3.2 `src/cli/web.rs`

- [ ] `parse_site` 接受 `"grok" | "gk"`，错误信息改为三个站点。
- [ ] `site_key` 增加 `"grok"`。
- [ ] `resolve_model_and_thinking` 增加 Grok 分支：`Fast` / `Build`；`--thinking` 显式报错。
- [ ] `resolve_session_target` 通过 `ai_web_remote_session` 自动识别 `ai-web-grok-`，无需额外改动。

### 3.3 `src/cli/web/exec.rs`

- [ ] `exec_models` 增加 Grok 条目：
  ```rust
  entries.push(json!({
      "site": "grok",
      "label": text::AI_WEB_SITE_GROK,
      "taskSpace": "lx-tui-ai-web-grok",
      "sessionPrefix": site.remote_session_prefix(),
      "defaultModel": GrokWebModel::default().id(),
      "defaultThinking": "",
      "models": GrokWebModel::ALL.iter().map(|m| m.id()).collect::<Vec<_>>(),
      "thinkingVariants": [],
  }));
  ```
- [ ] 行输出包含 `grok` / 模型名 / 空 variant / 默认标记 / 前缀 / 任务空间。

### 3.4 `src/cli/help.rs`

- [ ] 所有 `--site <deepseek|gemini>` 改为 `<deepseek|gemini|grok>`。
- [ ] 增加说明：Grok 仅支持 `Fast` / `Build`，没有 `--thinking` 档位。

### 3.5 测试

- [ ] `src/cli/tests/args_and_help_tests.rs`：`--site grok` / `gk`、`--model Build`、`--thinking low` 报错、help 文本。
- [ ] 运行：`cargo test cli:: -- --nocapture`。

---

## 阶段 4：Grok 驱动脚本

### 4.1 `src/chat/web/grok_helpers.js`（新增）

必须实现：

- [ ] 常量：`SPACE='lx-tui-ai-web-grok'`、`HOME='https://grok.com/'`、`MODEL`、`PROMPT`、`FILES`、`KEEP_FILES`、`FIRST_TURN`、`STREAM_URL`、`SESSION_ID`、`TITLE`、`EDIT_INDEX`。
- [ ] 复用 `emitMany` / `emit` / `checkStreamAlive` / `reclaimSpaceTarget` / `activateTaskSpace` / `ensureTaskSpaceControl`。
- [ ] `ensureGrokComposer()`：轮询 20s 等待 `[contenteditable="true"].ProseMirror`（对齐 `d6e8bada` 的冷启动修复）。
- [ ] `dismissCookieBanner()`：`.ot-pc-refuse-all-handler`。
- [ ] `grokLoggedIn()`：`GET /rest/auth/get-user` + `email` 非空；登录检测最多轮询 20s。**不要**因页面出现普通 `sign-in` 链接提前判未登录（会误判已登录页）。
- [ ] `ensureFreshChat()`：URL 为 `/` 且无 `[data-testid="user-message"]` 即视为新会话；否则导航 `HOME`。
- [ ] `ensureGrokModel()`：模型按钮轮询 15s；菜单 + `aria-checked` + 后置标签校验。
- [ ] `syncComposerAttachments()`：只在输入框 `form [role="list"][aria-label*="对话附件"]` / `*attachments*` 内清理旧 chip；CDP 注入前轮询隐藏 input 最多 10s；注入后 chip 最多等 60s（不得误删消息气泡里的附件）。
- [ ] `runNonChatAction()`：
  - `new` → `ensureFreshChat`
  - `stop` → `button[aria-label="停止模型响应"]`
  - `list_sessions` → REST 列表；401 回 `login_required`，非 2xx 回 `error`
  - `load_session` → REST `/responses` + parent 链回溯 + 附件名映射；`title` 从会话列表 REST 补；`message` 内的 `<grok:render>` 按 `cardAttachmentsJson` 映射为 `[来源名](url)`；401/非 2xx 同上
  - `rename_session` → `PUT /rest/app-chat/conversations/{id}`；401 回 `login_required`，非 2xx 回 `error`
  - `delete_session` → `DELETE /rest/app-chat/conversations/{id}`；当前页命中则回首页
- [ ] 所有失败路径 `emit({event:'error', message:...})`，不得静默。

### 4.2 `src/chat/web/grok_script.js`（新增）

必须实现：

- [ ] 登录/动作分流、`ensureFreshChat`、`SESSION_ID` 导航、`ensureGrokModel`；登录后再等 composer，导航后重新等 composer。
- [ ] 安装 `window.__lxGrokStream` DOM 采集器（60ms 轮询、尾部锚点对齐、10 tick 稳定判定）；`extractText` 只读 `.response-content-markdown`，**禁止**回退到助手消息 `innerText`（sr-only "Grok" 会被误当回答）。
- [ ] 普通发送：同步附件 → ProseMirror 写入 → 点击 `chat-submit`。
- [ ] `/rewind`：TUI/CLI 已在 Rust 层降级为普通追加；脚本保留 `EDIT_INDEX` 兼容分支，但采集目标恒为“发送前的 assistant 数量”，不得盯 `EDIT_INDEX` 旧回答。
- [ ] 完成时取 `/c/<uuid>` 与标题，`emit session` + `done`。
- [ ] 不 emit `thinking`；不构造 WebSocket。
- [ ] 超时 300s；`STREAM_ABORTED` 时退出。

### 4.3 测试

- [ ] `src/chat/web/tests.rs`：
  - `scripts_for_site(Grok)` 包含 `lx-tui-ai-web-grok`、`chat-submit`、`__lxGrokStream`；
  - `script_source(Grok, ...)` 正确转义 `model`/`thinking`/`prompt`；
  - Grok 的 `thinking` 为 `""`。
- [ ] 运行：`cargo test chat::web::tests -- --nocapture`。

---

## 阶段 5：首页 UI

### 5.1 `src/ui/style/lx.rs`

- [ ] `ai_web_site_gradient` 增加 `AiWebSite::Grok => (Color::Indexed(255), Color::Indexed(250))`。
- [ ] 确认 `ai_web_site_label_style` 复用插值，无需改算法。

### 5.2 `src/ui/main_content/lx/tests.rs`

- [ ] 更新三站点命中顺序：`Deepseek`、`Gemini`、`Grok`。
- [ ] 增加 Grok 渐变端点色断言。
- [ ] 增加窄窗格三等分测试：三个站点都可见、可点击，不被裁掉。
- [ ] 运行：`cargo test ui::main_content::lx::tests -- --nocapture`。

---

## 阶段 6：代码质量与全量校验

- [ ] `cargo fmt`
- [ ] `cargo clippy --all-targets -- -D warnings`
- [ ] `cargo test`
- [ ] `cargo build`

> 只针对受影响模块先跑 `cargo test <module>`；全量校验放在最后，避免每改一行跑一次全量。

---

## 阶段 7：端到端验证（必须逐项执行并记录结果）

### 7.1 准备

- [ ] 确认 `./target/debug/lx-tui` 为新构建。
- [ ] 在 `ego-browser` 中确认 `lx-tui-ai-web-grok` 任务空间可复用登录态。

### 7.2 命令验证

| # | 命令 | 预期 | 结果 |
| --- | --- | --- | --- |
| 1 | `./target/debug/lx-tui web models --site grok --json` | 输出 `grok` / `Fast`,`Build` / 空 `thinkingVariants` / `ai-web-grok-` | [ ] |
| 2 | `./target/debug/lx-tui web chat --site grok "Reply with the single word OK."` | 流式输出 `OK`，打印 `sessionId: ai-web-grok-<uuid>` | [ ] |
| 3 | `./target/debug/lx-tui web list --site grok` | 列表包含上一步标题 | [ ] |
| 4 | `./target/debug/lx-tui web show ai-web-grok-<uuid>` | 显示用户与助手消息 | [ ] |
| 5 | `./target/debug/lx-tui web rename ai-web-grok-<uuid> "LX Grok Test" --site grok` | 成功，`list` 标题更新 | [ ] |
| 6 | `./target/debug/lx-tui web chat --site grok --session ai-web-grok-<uuid> --edit-index 0 "Reply with the single word RETRY."` | 输出 `editIndex: null`，新回答追加，旧轮次保留（Grok 不支持原地替换） | [ ] |
| 7 | `./target/debug/lx-tui web new --site grok` | 浏览器回到 `https://grok.com/` 空白新会话 | [ ] |
| 8 | `./target/debug/lx-tui web chat --site grok "Count from 1 to 200..." &` + 4s 后 `web stop --site grok` | 长回答停止增长 | [ ] |
| 9 | `./target/debug/lx-tui web chat --site grok --file <path> "Reply with the secret word"` | 附件上传成功，回答能引用附件内容 | [ ] |
| 10 | `./target/debug/lx-tui web delete ai-web-grok-<uuid> --site grok` | 成功，`list` 不再包含测试会话 | [ ] |

### 7.3 边界验证

- [ ] **冷启动**：关闭 Grok 所有标签页后执行 `web chat --site grok "hi"`，应等待页面渲染后成功，不得报 `Grok composer not found` 或 `Grok model menu not found`。
- [ ] `--thinking low` 对 Grok 报错：`Grok has no thinking variants; omit '--thinking'`。
- [ ] `--model Expert` 报错：`expected 'Fast' or 'Build'`。
- [ ] `--site gemini --session ai-web-grok-<uuid>` 报站点冲突。
- [ ] 未登录场景：手动在 ego-browser 退出 Grok，再执行 `web chat`，应提示 `Please sign in to Grok in ego-browser` 并移交控制权。
- [ ] 带附件轮次 `/rewind`：TUI 中执行 `/rewind` 应弹 `Grok /rewind cannot restore attachments...`，消息与输入不变。
- [ ] 纯文本轮次 `/rewind`：TUI 中执行 `/rewind` 应保留旧消息、回填输入，并弹 `Grok /rewind appends a retry turn; use /new for a clean chat`；下一次发送追加新轮次。
- [ ] 普通附件发送：`web chat --site grok --file <path> "Summarize"` 能上传并回答。\n- [ ] 引用标签：对带联网引用的会话执行 `web show`，输出不得含 `<grok:render>`，应出现 `[来源名](url)`；实时 `web chat` 的联网回答也不得泄漏原始标签。

### 7.4 清理

- [ ] 删除本轮所有测试会话；保留用户原有会话。
- [ ] 确认 `lx-tui-ai-web-grok` 任务空间只留下空白首页或用户指定页面。
- [ ] 记录验证结果到 `docs/feature/lx-ai-web-grok/verification.md`（执行后新增）。

---

## 阶段 8：交付

- [ ] 汇总改动文件清单。
- [ ] 汇总验证命令与结果。
- [ ] 说明未自动化验证的部分（TUI 鼠标点击、视觉下划线、Toast 位置），交由用户 `cargo run` 目测。
- [ ] 按用户补充要求提交代码；不 push，除非用户明确授权。

---

## 关键不变量清单（执行时逐条对照）

1. `AiWebSite::ALL` 顺序必须是 `Deepseek`、`Gemini`、`Grok`。
2. `ai-web-grok-` 前缀不得与 `ai-web-` / `ai-web-ds-` / `ai-web-gm-` 冲突。
3. Grok `/model` 不得进入第二级空面板。
4. Grok `/rewind` 不得静默丢附件；纯文本轮次必须保留旧轮次并走普通追加（Grok Edit 无法原地替换）。
5. Grok 不得显示伪造的 `Thinking`。
6. 不得构造/重放 Grok WebSocket 帧。
7. 不得写 SQLite / `ai.json` / `<system-update>`。
8. 所有用户可见文案必须是英文，集中在 `src/ui/text.rs`。
9. 生产代码不得出现 `unwrap()` / `expect()`。
10. 单文件 `< 1000` 行。
11. 正文只读 `.response-content-markdown`，禁止回退助手消息 `innerText`。
12. 登录只信 `get-user` 的 `email`，不得因普通 `sign-in` 链接提前判未登录。\n13. 历史/实时回答都不得出现 `<grok:render>` 原始标签；历史标签必须按 `cardAttachmentsJson` 映射为来源链接。
