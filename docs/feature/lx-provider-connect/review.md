# 预设 Provider 与 `/connect` 快捷连接及四协议对齐 — 代码审查报告 (`review.md`)

- **特性目录**：`docs/feature/lx-provider-connect/`
- **审查工作区**：已合并入主工作区 `dev` 分支（原工作区 `.worktrees/lx-provider-connect` 与分支 `feature/lx-provider-connect` 已移除）
- **基线范围**：`dev` 合并变更集（涵盖预设目录、`connect.json`、OAuth、TUI `Overlay::Connect`、模态 Toast 路由规则、设置页双 Tab、CLI `model` 运行时合并、云端与 `/models` 同步、以及 `anthropic`/`openai-responses`/`openai`/`google` 四协议修复）
- **修复状态**：**全部 P0–P3 审查项（含第一轮与第二轮深度复审）已完成修复，并通过 `cargo fmt`、`cargo clippy --all-targets -- -D warnings` 与 `OPENAI_API_KEY=dummy cargo test`（4763 passed across all suites, 0 failed）验证**
- **优先级定义**：
  - **P0（阻断级 / 必然导致功能不可用、首次连接无法选用或请求 4xx/500）**
  - **P1（高优先级 / 违反架构边界、特定供应商认证或协议续轮失败、设置页草稿丢失）**
  - **P2（中优先级 / 并发覆盖风险、状态不同步、Toast 锚点或层级回退边界缺陷）**
  - **P3（低优先级 / 代码规范、文件行数红线、HTML/OS 隔离与文档同步）**

---

## P0（阻断级缺陷）

### P0-1：OAuth Token 自动刷新未覆盖聊天轮次与优先刷新顺序，且刷新响应未返回新 `refresh_token` 时会丢失原 `refresh_token`
- **涉及位置**：
  - `src/config/ai/oauth/mod.rs`（`credential_needs_refresh`、`refresh_oauth_credential`、`ensure_connect_oauth_fresh`）
  - `src/event/chat/connect.rs`（`refresh_models`、`load_models_snapshot`）、`src/event/chat/mod.rs`（`chat_api_key`）、`src/settings/service/models/mod.rs`（`fetch_models`）、`src/chat/agent/exec/mod.rs`
- **关键点**：
  - `design.md` §4.2.2 要求在运行时加载、聊天采样或 `/models` 发现前，若活跃预设连接为 OAuth 且 `expires_at_ms` 距当前不足 5 分钟（且存在 `refresh_token`），自动调用 `refresh_oauth_credential` 刷新并原子写回 `connect.json`；
  - 部分 OAuth 供应商（如 OpenAI、GitLab、Poe）刷新响应仅返回新的 `access_token`，不重复返回 `refresh_token`。
- **错误原因**：
  1. 初始实现中 `credential_needs_refresh` 与 `refresh_oauth_credential` 未在生产路径调用；且 `refresh_models`（处理 `Effect::RefreshModelsCatalog`）先调 `refresh_connected_provider_models(...).await?`（用过期 Token 探测 `/models` 直接报错返回），后调 `ensure_connect_oauth_fresh`，导致过期后点 `Refresh models` 无法触发刷新；
  2. TUI 长时间运行不重启时，`StartChatStream` 直接使用内存 `FrozenModel` 中的旧 `api_key`，未在发起采样前刷新即将过期的 OAuth 凭证；
  3. `refresh_oauth_credential` 用响应中的 `refresh_token`（`None`）覆盖了旧的 `credential.refresh_token`，且丢失了原有的 `metadata`（如 GitHub Copilot `apiEndpoint`）与 `account_label`。
- **修复方案**：
  1. 在 `refresh_oauth_credential` 中保留旧 `refresh_token`、`account_label` 并合并 `metadata`；
  2. 在 `src/event/chat/connect.rs` 的 `refresh_models` 中**先**执行 `ensure_connect_oauth_fresh(&ai_path).await`，再探测 `/models`；
  3. 在 TUI 聊天流启动（`src/event/chat/mod.rs` 的 `chat_api_key`）、设置页 `fetch_models`（刷新后同步广播 `SettingsEvent::ModelsChanged`）及无头执行入口统一接入 `ensure_connect_oauth_fresh`。

---

### P0-2：网页设置页首次用 API Key 连接预设会保存为 `enabled: false`，导致新连接的供应商不进入 `enabled_providers`
- **涉及位置**：
  - `src/settings/service/models/connect.rs`（`preset_json`、`apply_connect_item`）
  - `assets/settings_connect.js`（`renderPresetEditor`、`savePresetDrafts`）
  - `src/config/ai/connect.rs`（`merge_runtime_ai_settings`）
- **关键点**：
  - 未连接且无 `$ENV` 的预设在 `GET /api/models/settings` 中返回 `connected = false`、`enabled = false`，页面 `Enabled` 复选框默认未勾选；
  - 用户在网页设置页 Quick Connect 首次粘贴 API Key 并点击顶部 `Save` 后，该供应商应当立即启用并出现在 TUI `/model` 与设置页 `Default model` 下拉框中。
- **错误原因**：
  - 前端 `savePresetDrafts()` 无条件发送 `enabled: !!preset.enabled`（`false`），后端 `apply_connect_item` 如实写入 `record.enabled = false`；
  - `resolve_preset_connection_source_with_env` 看到有 `api_key` 将徽章标为 `Connected · API key`，但 `merge_runtime_ai_settings` 仅将 `enabled == true` 的供应商放入 `enabled_providers`，导致用户看到“已连接”却在 `/model` 与默认模型列表中找不到该供应商。
- **修复方案**：
  1. 后端 `apply_connect_item`：记录修改前是否已有凭证 `had_credentials`；当此前无有效凭证（`!had_credentials`）而本次提交了非空 `api_key` 时，强制将 `record.enabled = true`；
  2. 前端 `assets/settings_connect.js`：当用户在未连接预设的 `#prov-key` 输入非空 API Key 时自动勾选 `#prov-enabled`，且 `savePresetDrafts()` 对首次填入 API Key 的未连接预设强制发送 `enabled: true`。

---

### P0-3：`openai` 传输层在 `compatible == false` 时忽略配置的 `base_url`，且云端目录合并曾将非官方 `Openai` 预设模型强改 `OpenaiResponses`
- **涉及位置**：
  - `src/chat/transport/openai/mod.rs:48-52`（`build`）
  - `src/config/ai/presets.rs:232-277`（`update_catalog_models`、`merge_provider_models`）
- **关键点**：
  - `presets_catalog.json` 中 `github-copilot`、`opencode`、`cloudflare-ai-gateway`、`vercel` 等网关型预设的顶层 `ProviderTransport` 为 `Openai`（即 `compatible == false`），其下许多模型 `transport` 为 `None`（走 `/chat/completions`），并使用各自网关的 `base_url`（如 `https://opencode.ai/zen/v1`、`https://api.githubcopilot.com`）。
- **错误原因**：
  1. `src/chat/transport/openai/mod.rs` 在 `compatible == false` 时无条件使用硬编码常量 `https://api.openai.com/v1`，完全忽略了 `req.base_url`，导致 `opencode` / `github-copilot` 下所有非 Responses 模型请求全部发往 `api.openai.com`；
  2. `update_catalog_models` 曾对所有 `spec.transport == ProviderTransport::Openai` 的供应商默认赋予 `ModelTransport::OpenaiResponses`，导致非 OpenAI 官方供应商的模型被错误改写为 `/responses`。
- **修复方案**：
  1. `src/chat/transport/openai/mod.rs` 在 `!req.base_url.trim().is_empty()` 时始终使用配置的 `req.base_url`，仅当 `req.base_url` 为空时回退到 `https://api.openai.com/v1`；
  2. `src/config/ai/presets.rs` 将默认赋予 `ModelTransport::OpenaiResponses` 限定为 `provider_id == "openai"`，其余供应商严格保留 `existing.transport` 或 `None`。

---

### P0-4：OpenCode Zen / Go 设备码登录 `HTTP 302` 与 OpenAI ChatGPT OAuth 端点/回调主机不匹配
- **涉及位置**：
  - `src/config/ai/presets_catalog.json`（`opencode`、`opencode-go`、`xai`、`poe`、`digitalocean`）
  - `src/config/ai/oauth/device.rs`、`src/config/ai/oauth/browser.rs`、`src/config/ai/oauth/mod.rs`
  - `src/chat/transport/responses/mod.rs`
- **关键点**：
  - 对照 `opencode-dev`：OpenCode Console 设备码端点位于 `https://opencode.ai/console/auth/device/{code,token}` 且要求 JSON 请求体；
  - OpenAI ChatGPT Plus/Pro OAuth 的 `access_token` 必须请求 `https://chatgpt.com/backend-api/codex`（带 `store: false`），且 `auth.openai.com` 固定端口回调要求 `http://localhost:1455/auth/callback`。
- **错误原因**：
  1. `opencode` / `opencode-go` 旧路径 `/auth/device/code` 返回 `HTTP 302 -> /console/login`，且相对 `verification_uri_complete` 未拼接域名；
  2. `oauth/browser.rs` 固定端口回调使用了 `127.0.0.1` 而非 `localhost`；
  3. `credential_from_token_for` 未为 `openai` OAuth 写入 `apiEndpoint = "https://chatgpt.com/backend-api/codex"` 与 JWT `chatgptAccountId`，`responses/mod.rs` 亦未对 `chatgpt.com/backend-api` 附加 `"store": false`。
- **修复方案**：
  1. 更新 `opencode` / `opencode-go` 设备码端点为 `/console/auth/device/{code,token}`，使用 JSON 请求体并由 `resolve_verification_url` 补全相对验证 URL；
  2. `oauth/browser.rs` 在 `fixed_port.is_some()` 时使用 `http://localhost:{port}/auth/callback`；
  3. `credential_from_token_for` 为 `openai` OAuth 注入 `apiEndpoint` 与 JWT 解析出的 `chatgptAccountId`，并在 `responses/mod.rs` 针对 `chatgpt.com/backend-api` 自动设置 `"store": false`。

---

### P0-5：Anthropic 传输层对非 Claude 4.6+/5.x 模型误发 `adaptive` thinking，且手动 `thinking: enabled` 在无签名历史工具续轮中触发 400
- **涉及位置**：
  - `src/chat/transport/anthropic/mod.rs`、`src/config/ai/verify.rs`
- **关键点**：
  - Anthropic Messages API 仅在 Claude 4.6 / 4.7 / 5.x（含 Fable / Mythos）支持 `thinking: {"type": "adaptive", "display": "summarized"}` + `output_config.effort`；Claude 3.7 / 4.0 / 4.1 / 4.5 与第三方 Anthropic 兼容端点只支持 `thinking: {"type": "enabled", "budget_tokens": ...}`；
  - 当历史 `messages` 含有无签名 `assistant` `tool_use` 块时，开启手动 `thinking: enabled` 会触发 HTTP 400。
- **错误原因**：
  - 反向判断导致第三方 Anthropic 兼容模型误发 `adaptive`；工具调用续轮未检测历史无签名 `tool_use`。
- **修复方案**：
  - 改为正向判定 `supports_adaptive_thinking(model_id)`；当 `messages_have_unsigned_assistant_tool_use(&messages)` 为真且处于手动 `thinking: enabled` 时自动省略本轮 `thinking` 字段。

---

## P1（高优先级缺陷）

### P1-1：`models.dev` v2 云端目录将图片输入模态表示为 `"image/png"` 等 MIME，导致同步后图片输入能力失效
- **涉及位置**：
  - `src/config/ai/catalog_refresh.rs`（`parse_modalities`）
  - `src/config/ai/mod.rs`（`ModelConfig::supports_image_input`）
- **关键点**：
  - `models.opencode.ai/experimental/v2.0/api.json` 返回的 `modalities.input` 使用 `"image/png"`、`"image/jpeg"` 等 MIME 字符串。
- **错误原因**：
  - `parse_modalities` 原样保留了 `"image/*"`，而 `ModelConfig::supports_image_input` 仅判断 `item == "image"`，导致云端目录同步后所有多模态模型的图片输入支持被误判为 `false`。
- **修复方案**：
  - 在 `parse_modalities` 中将所有 `"image/*"` 归一化为 `"image"` 并去重；同时在 `ModelConfig::supports_image_input` 中兼容 `item == "image" || item.starts_with("image/")`。

---

### P1-2：Anthropic 与 Google Gemini 传输层将工具结果后的 `UserText` 合并进同一条 `tool_result` / `functionResponse` 消息导致 400
- **涉及位置**：
  - `src/chat/transport/anthropic/mod.rs`（`messages_of`、`push_block`）
  - `src/chat/transport/google/mod.rs`（`contents_of`、`push_parts`）
- **关键点**：
  - Google Gemini 拒绝在包含 `functionResponse` 的 `user` Content 中混入普通 `text`/`inlineData` part（`gemini.ts:330-333`）；Anthropic 同样不接受在 `tool_result` 消息后直接把下一条独立用户文本并入同一 `user` 消息。
- **错误原因**：
  - `push_block` 与 `push_parts` 仅检查上一条消息 `role == "user"` 即无条件合并，当工具返回后紧跟本地命令输出或用户文本时产生了非法混排结构。
- **修复方案**：
  - `anthropic/mod.rs`：处理 `ModelItem::UserText` 时，若上一条 `user` 消息已包含 `tool_result` 块则新起一条 `user` 消息（同一 `ToolResult` 自身附带的图片块仍留在同条消息内）；
  - `google/mod.rs`：仅当上一条 `user` 消息全部为 `functionResponse` 且新 parts 也全部为 `functionResponse`（或双方均不含 `functionResponse`）时才合并。

---

### P1-3：设置页 Quick Connect 在首次保存前点击 `Fetch models` 必报 422 `"provider not found"`，且未传递 `formAnswers`
- **涉及位置**：
  - `src/settings/service/models/mod.rs`（`FetchBody`、`fetch_models`）
  - `assets/settings_models.js`（`fetchModels`）
- **关键点**：
  - 用户在设置页选中尚未连接的预设供应商、填入 API Key（及表单字段）后直接点击 `Fetch models` 验证并拉取模型。
- **错误原因**：
  - 后端 `fetch_models` 直接调用 `materialize_preset_provider`（要求 `connect.json` 已有已保存凭证），未保存时返回 `None` 并报 422 `"provider not found"`；且前端 `fetchModels` 未发送 `formAnswers`。
- **修复方案**：
  - 前端 `fetchModels` 发送 `formAnswers: provider.formAnswers || {}`；
  - 后端 `fetch_models` 在 `find_preset(provider_id)` 命中时，若磁盘尚未保存连接，合并请求 `formAnswers` 并通过 `resolve_preset_base_url` 与请求 `apiKey` 直接发起 `/models` 探测。

---

### P1-4：设置页切换 Tab 会静默取消进行中的 OAuth 登录，且 `Disconnect` / OAuth 完成调用全量 `loadModels()` 会冲掉 Custom Tab 未保存草稿
- **涉及位置**：
  - `assets/settings_connect.js`（`switchProviderTab`、`disconnectPresetProvider`、`startPresetOAuthPoll`、`refreshQuickConnectState`）
  - `assets/settings_models.js`（`saveModels`）
- **关键点**：
  - 用户在等待 OAuth 授权期间可能切换到 `Custom` Tab 查看配置，或在 `Custom` Tab 有未保存修改时到 `Quick Connect` 执行 `Disconnect` / OAuth 登录。
- **错误原因**：
  1. `switchProviderTab` 无条件调用 `dismissPresetOAuth(false)` 发送 `DELETE /api/models/connect/oauth` 终止后台授权；
  2. `disconnectPresetProvider` 与 OAuth 完成回调直接调用 `loadModels()` 用服务端快照覆盖整个 `modelsDraft`，冲掉 `Custom` Tab 未保存编辑；
  3. 清空已连接预设的 API Key 并同时保存其他改动时，旧草稿的 `defaultModel` 未先与最新 `quickConnect` 状态同步。
- **修复方案**：
  1. `switchProviderTab(tab)` 在 `activeProviderTab === tab` 时早退，且切换 Tab 时保留进行中的 `presetOAuth`；
  2. 新增 `refreshQuickConnectState(targetPresetId)`，仅增量刷新 `modelsRevision`、`modelsSnapshot.quickConnect` 与未脏预设草稿，并自动清理指向已断开预设的 `defaultModel` / `titleSummary`，保护 `Custom` Tab 草稿不丢失。

---

### P1-5：TUI `/connect` 异步事件回投缺少阶段/窗格关联校验，且单击列表行、环境变量隔离与 Disconnect 悬空引用清理需对齐规范
- **涉及位置**：
  - `src/app/update/chat/connect/mod.rs`、`src/app/update/chat/connect/events.rs`、`src/event/chat/connect.rs`
  - `src/event/overlay.rs`、`src/app/state/lx/connect.rs`、`src/config/ai/connect.rs`
- **关键点**：
  - 异步 `Connected` / `Disconnected` / `Failed` 回投到达时，用户可能已关闭 `/connect`（甚至打开了其他模态弹窗）或切到了其他供应商阶段；
  - 鼠标单击仅移动焦点，再次点击已选中行或按 `Enter` 才执行；`app/` 保持纯内存状态不调 `std::env::var`；`Disconnect` 需同步清理 `ai.json` 悬空引用并在 `$ENV` 存在时写 `enabled: false`。
- **错误原因**：
  1. `on_preset_connected` 无条件置 `state.overlay = None` 并清空 `lx` 输入；`on_preset_disconnected` 无条件将面板重置回 `Providers`；`Failed` 未携带 `provider_id`；
  2. `Manage` 阶段点击 `Disconnect` 未置 `validating` 防重。
- **修复方案**：
  1. `on_preset_connected` 仅当 `state.overlay` 仍为该 `pane` 的 `Overlay::Connect` 时才关闭弹窗、清空输入并打开 `/model`；否则仅静默更新快照并按 `show_toast_at` 提示；
  2. `on_preset_disconnected` 仅在面板处于 `Providers` 或同 `provider_id` 的 `Manage` 阶段时退回列表，其余阶段只调 `refresh_entries`；`Manage` 触发 `Disconnect` 时设置 `validating = true` 与 `"Disconnecting…"`；
  3. `ConnectEvent::Failed` 携带 `provider_id: Option<String>`，仅在当前面板 `provider_id()` 匹配时才写内联错误。

---

## P2（中优先级缺陷）

### P2-1：`show_toast_at` 在锚点窗格已关闭时未清空 `toast.anchor`，导致降级全局时 `ui::toast::rect` 返回 `None`
- **涉及位置**：
  - `src/app/update/interaction.rs`（`show_toast_at`）
  - `src/ui/toast.rs`（`rect`）
- **关键点**：
  - 模态弹窗 Toast 规则要求：弹窗未关闭或退回一级弹窗时走全局 `anchor: None`；弹窗全部关闭时走打开弹窗的源容器，若源窗格已关闭则降级全局屏幕顶部居中。
- **错误原因**：
  - 当 `anchor = Some(pane)` 的窗格已被关闭时，`show_toast_at` 调用 `show_toast(state, toast)` 但未将 `toast.anchor` 清为 `None`，导致 `ui::toast::rect` 查找不到该 `pane` 而返回 `None`。
- **修复方案**：
  - 在 `show_toast_at` 中，当 `pane != state.prompt.id()` 且 `state.pane_anywhere(pane).is_none()` 时，先置 `toast.anchor = None` 再调用 `show_toast(state, toast)`。

---

### P2-2：TUI `/connect` 在多认证方式供应商的 `OAuthWaiting` / `InputPrompt` 按 `Esc` 直接跳回 `Providers`，未逐级退回 `Methods`
- **涉及位置**：
  - `src/app/update/chat/connect/mod.rs`（`apply_connect_esc`）
  - `src/app/state/lx/connect.rs`（`refresh_entries`）
- **错误原因**：
  - `apply_connect_esc` 对所有非 `Providers` 阶段一律调用 `reset_to_providers()`，多方法供应商（如 OpenAI）从第 3 级按 `Esc` 跳过了第 2 级 `Methods`；
  - `refresh_entries` 仅在过滤词为空时调用 `select_provider`，且列表上下移动未同步记录 `last_connect_provider`。
- **修复方案**：
  - 多方法供应商在 `OAuthWaiting` 或首步 `InputPrompt` 按 `Esc` 先退回 `ConnectStage::Methods` 并恢复原 `method_index` 高亮，再按 `Esc` 退回 `Providers` 并高亮该供应商；
  - `refresh_entries` 无论过滤词是否为空均调用 `select_provider` 恢复选中；列表移动高亮时同步更新 `state.last_connect_provider`。

---

### P2-3：设置页 `persist_oauth_credential` 吞掉 `load_ai` 错误且未清理 `ai.json` 悬空选择，前端 OAuth 轮询对 404 无限重试
- **涉及位置**：
  - `src/settings/service/models/connect.rs`（`persist_oauth_credential`）
  - `assets/settings_connect.js`（`startPresetOAuthPoll`）
- **错误原因**：
  1. `persist_oauth_credential` 用 `load_ai(ai_file).ok().flatten().unwrap_or_else(AiSettings::empty)` 静默吞错且未将清理悬空模型后的 `ai.json` 写回磁盘；
  2. `startPresetOAuthPoll` 用 `.catch(() => {})` 忽略了 404 或连续错误，会话失效后永远停在 pending。
- **修复方案**：
  1. `persist_oauth_credential` 对 `load_ai` 错误记录 `tracing::warn!` 并中止广播不完整快照，且在 `sanitize_dangling_selections` 变更时原子写回 `ai.json`；
  2. `startPresetOAuthPoll` 遇到 404 或连续 3 次错误时停止轮询、清理 `presetOAuth` 并弹出错误 Toast。

---

### P2-4：OpenAI Chat Completions 在 `require_reasoning` 为真时遗漏纯文本 `assistant` 历史消息的 `reasoning_content`
- **涉及位置**：
  - `src/chat/transport/openai/mod.rs`（`messages_of`）
- **错误原因**：
  - Kimi / DeepSeek thinking 模式要求开启思考时历史中所有 `assistant` 消息均带 `reasoning_content`；原先仅在 `flush_assistant_tools` 中补充，纯文本 `assistant` 消息未补充。
- **修复方案**：
  - 当 `include_empty_reasoning` 为真时，对 `ModelItem::AssistantText` 生成的 `assistant` 消息同样注入 `"reasoning_content": ""`。

---

### P2-5：Google Gemini 流式解析未完整覆盖 `PROHIBITED_CONTENT` / `BLOCKLIST` / `MALFORMED_FUNCTION_CALL` / `OTHER` 等 `finishReason`
- **涉及位置**：
  - `src/chat/transport/google/mod.rs`
- **错误原因**：
  - 仅识别 `STOP` / `MAX_TOKENS` / `SAFETY` / `RECITATION`，其余 `finishReason` 未触发结束或协议错误，导致最终报 `"stream ended without completion"`。
- **修复方案**：
  - 将 `SAFETY | RECITATION | PROHIBITED_CONTENT | BLOCKLIST | SPII | IMAGE_SAFETY | MALFORMED_FUNCTION_CALL` 映射为带明确原因的协议错误，将 `STOP | MAX_TOKENS | OTHER | FINISH_REASON_UNSPECIFIED` 映射为正常完成。

---

### P2-6：后台模型刷新并发安全、设置页保存顺序、预设物化值不落盘与单元测试环境隔离
- **涉及位置**：
  - `src/config/ai/catalog_refresh.rs`、`src/config/ai/presets.rs`、`src/settings/service/models/connect.rs`、`assets/settings.js`、`assets/settings_models.js`
- **修复方案**：
  - `refresh_connected_provider_models` 在网络探测完成后重读最新 `connect.json` 再合并模型表；所有写路径使用严格 `load_connect`；
  - `put_connect` 对比默认/物化 Base URL 与默认模型表，相等时保持 `None`；
  - `build_catalog()` 在 `#[cfg(not(test))]` 下才叠加本机磁盘缓存 `models_catalog_cache.json`。

---

## P3（代码规范、文件行数、HTML/OS 隔离与文档同步）

### P3-1：OAuth 回调页 HTML 内联、`/dev/urandom` 直接访问，以及 `connect.rs` / `settings*.js` 接近 1000 行上限
- **涉及位置**：
  - `src/config/ai/oauth/browser.rs`、`assets/oauth_callback.html`、`assets/oauth_implicit.html`
  - `src/platform/mod.rs`、`src/config/ai/oauth/mod.rs`
  - `src/app/update/chat/connect/mod.rs`（534 行）、`src/app/update/chat/connect/events.rs`（550 行）
  - `assets/settings.js`（948 行）、`assets/settings_models.js`（935 行）、`assets/settings_connect.js`（632 行）
- **错误原因**：
  1. `oauth/browser.rs` 将回调 HTML/CSS/JS 内联在 `.rs` 中，违反 `docs/standards/html-assets.md`；
  2. `oauth/mod.rs` 直接读取 `/dev/urandom`，违反 OS 代码集中在 `src/platform/` 的规范；
  3. `src/app/update/chat/connect.rs`（951 行）、`assets/settings.js`（1000 行）、`assets/settings_models.js`（997 行）逼近 1000 行上限。
- **修复方案**：
  1. 将 OAuth 回调页拆入 `assets/oauth_callback.html` 与 `assets/oauth_implicit.html`，经 `include_str!` 加载；
  2. 在 `src/platform/mod.rs` 新增 `fill_random_bytes`，移除 `oauth/mod.rs` 中的 `/dev/urandom` 直接读取；
  3. 将 `src/app/update/chat/connect.rs` 拆分为 `connect/mod.rs` 与 `connect/events.rs`；将预设草稿辅助函数整理至 `assets/settings_connect.js` 并清理死代码，使全部生产文件均远低于 1000 行上限。

---

### P3-2：鼠标左键按下未清理模态滚动条拖拽位、`ReferencedFolder*` 复制丢弃源容器锚点、Quick Connect 空态按钮显隐
- **涉及位置**：
  - `src/event/mouse/press.rs`
  - `src/app/update/referenced.rs`、`src/app/update/referenced/search.rs`
  - `assets/settings_connect.js`
- **修复方案**：
  1. 在 `src/event/mouse/press.rs` 左键按下入口清理 `connect_scroll_drag`、`referenced_search_scroll_drag` 与 `referenced_menu_scroll_drag`；
  2. `ReferencedFolderMenu` 与 `ReferencedFolderSearch` 复制动作将 `anchor` 设为 `Some(state.prompt.id())`，交由 `show_toast_at` 按弹窗在屏态统一分派；
  3. `assets/settings_connect.js` 在 `Quick Connect` Tab 下隐藏空态 `#btn-empty-new-provider` 按钮。
