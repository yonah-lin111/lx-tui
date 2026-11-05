# 预设 Provider 与 `/connect` 快捷连接及四协议对齐 — 代码审查报告 (`review.md`)

- **特性目录**：`docs/feature/lx-provider-connect/`
- **审查工作区**：`.worktrees/lx-provider-connect`（分支 `feature/lx-provider-connect`）
- **基线范围**：`dev..HEAD`（涵盖预设目录、`connect.json`、OAuth、TUI `Overlay::Connect`、设置页双 Tab、云端与 `/models` 同步、以及 `anthropic`/`openai-responses`/`openai`/`google` 四协议修复）
- **修复状态**：**全部 P0–P3 审查项已完成修复并通过 `cargo fmt`、`cargo clippy --all-targets -- -D warnings` 与 `OPENAI_API_KEY=dummy cargo test --lib`（4675 passed, 0 failed）验证**
- **优先级定义**：
  - **P0（阻断级 / 必然导致功能不可用或请求 4xx/500）**
  - **P1（高优先级 / 违反架构边界、特定供应商认证或断开失效、单测受宿主环境污染）**
  - **P2（中优先级 / 并发覆盖风险、状态不同步、边界条件缺陷）**
  - **P3（低优先级 / 代码规范、文案组织与文档同步）**

---

## P0（阻断级缺陷）

### P0-1：OAuth Token 自动刷新未接线，且刷新响应未返回新 `refresh_token` 时会丢失原 `refresh_token`
- **涉及位置**：
  - `src/config/ai/oauth/mod.rs:219-283`（`credential_needs_refresh`、`refresh_oauth_credential`）
  - `src/event/chat/connect.rs:390-410`、`src/settings/service/models/mod.rs:471-493`、`src/chat/agent/exec/mod.rs:439-444`
- **关键点**：
  - `design.md` §4.2.2 明确要求在运行时加载、聊天采样或 `/models` 发现前，若活跃预设连接为 OAuth 且 `expires_at_ms` 距当前不足 5 分钟（且存在 `refresh_token`），自动调用 `refresh_oauth_credential` 刷新并原子写回 `connect.json`。
  - OAuth 刷新接口在部分供应商（如 OpenAI、GitLab、Poe）响应中仅返回新的 `access_token`，不重复返回 `refresh_token`。
- **错误原因**：
  1. `credential_needs_refresh` 与 `refresh_oauth_credential` 仅在单元测试中被调用，生产路径（TUI 启动/刷新、聊天请求前、设置页 `Fetch models`）完全未调用，导致短效 OAuth Token 过期后所有请求直接报 HTTP 401；
  2. `refresh_oauth_credential` 调用 `credential_from_token` 构造新凭证时直接用响应中的 `refresh_token`（`None`）覆盖了旧的 `credential.refresh_token`，且丢失了原有的 `metadata`（如 GitHub Copilot `apiEndpoint`）与 `account_label`。
- **修复方案**：
  1. 在 `refresh_oauth_credential` 中保留旧 `refresh_token`（`refreshed.refresh_token = refreshed.refresh_token.or_else(|| credential.refresh_token.clone())`）、`account_label` 并合并 `metadata`；
  2. 新增 `ensure_connect_oauth_fresh(ai_path: &Path) -> bool`，遍历 `connect.json` 中已启用且 `credential_needs_refresh` 为真的 OAuth 记录执行刷新并原子写回，在 TUI 模型刷新/连接加载、设置页 `fetch_models` 及无头执行入口统一接入。

---

### P0-2：云端目录合并将 `github-copilot` / `opencode` 等非 OpenAI 官方供应商的无 `transport` 模型强制改为 `OpenaiResponses`
- **涉及位置**：
  - `src/config/ai/presets.rs:232-247, 250-277`（`update_catalog_models`、`merge_provider_models`）
- **关键点**：
  - `presets_catalog.json` 中 `openai`、`github-copilot`、`opencode`、`cloudflare-ai-gateway` 等预设的顶层 `ProviderTransport` 均为 `Openai`。
  - 但只有 `openai` 官方预设下未显式标注 `transport` 的模型默认走 `ModelTransport::OpenaiResponses`；`github-copilot` 与 `opencode` 下的 Claude / Gemini / Kimi / MiniMax 等模型在 `models.dev` 中不带 `api.responses`，应当保持 `model.transport = None`（走 Chat Completions）或保持原有 `existing.transport`。
- **错误原因**：
  - `update_catalog_models` 用 `spec.transport == ProviderTransport::Openai` 判断 `is_openai_provider`，并在 `merge_provider_models` 中对所有 `is_openai_provider && model.transport.is_none()` 强制赋值为 `Some(ModelTransport::OpenaiResponses)`，导致启动同步 `models.dev` 目录后，`github-copilot` 与 `opencode` 下的非 Responses 模型全部被改写为 POST `/responses`，请求直接 404/400。
- **修复方案**：
  - 将 `is_openai_provider` 精确限定为 `provider_id == "openai"`，且当 `existing` 模型已存在时严格保留 `existing.transport`（仅对 `openai` 官方新增且未标注 `transport` 的模型默认赋予 `ModelTransport::OpenaiResponses`）。

---

### P0-3：Anthropic 传输层对非 Claude 4.6+/5.x 模型误发 `adaptive` thinking，且手动 `thinking: enabled` 在无签名历史工具续轮中触发 400
- **涉及位置**：
  - `src/chat/transport/anthropic/mod.rs:88-163`
  - `src/config/ai/verify.rs:449-452`
- **关键点**：
  - Anthropic Messages API 仅在 Claude 4.6 / 4.7 / 5.x（含 Fable / Mythos）上支持 `thinking: {"type": "adaptive", "display": "summarized"}` + `output_config: {"effort": ...}`；
  - 其他走 Anthropic 兼容协议的模型（如 Claude 3.7 / 4.0 / 4.1 / 4.5，以及 Kimi Coding、MiniMax、Z.ai Anthropic 兼容端点）只支持手动 `thinking: {"type": "enabled", "budget_tokens": ...}`；
  - 此外，Anthropic 官方规范要求：当请求开启手动 `thinking: {"type": "enabled"}` 且 `messages` 中最后一条 `assistant` 消息包含 `tool_use` 时，该 `assistant` 消息必须以带有效 `signature` 的 `thinking` 块开头；而 `lx-tui` 为支持跨厂商切模型，设计上不在历史 `ModelItem` 中回放 `reasoning` 块。
- **错误原因**：
  1. `anthropic/mod.rs` 用 `is_manual_thinking_claude` 做反向判断，导致所有非 Claude 模型（如 Kimi / MiniMax / GLM 的 Anthropic 端点）命中 `else` 分支被发送 `thinking: {"type": "adaptive"}` + `output_config`，触发 HTTP 400；
  2. 当使用 Claude 3.7 / 4.0 / 4.5 等手动 thinking 模型执行工具调用后进入第 2 步（回传 `tool_result`）时，`messages` 中已存在不含 `thinking` 块的 `assistant` `tool_use` 消息，此时继续发送 `thinking: {"type": "enabled"}` 会被 Anthropic 拒绝（HTTP 400 `Expected 'thinking' or 'redacted_thinking', but found 'tool_use'`）。
- **修复方案**：
  1. 改为正向判定 `supports_adaptive_thinking(model_id)`（仅匹配 Claude 4.6 / 4.7 / 5.x / Fable / Mythos）；其余模型一律将 `effort` 映射为 `thinking: {"type": "enabled", "budget_tokens": ...}`；
  2. 在 `anthropic::build` 中检查 `messages`：若当前配置生成了手动 `thinking: {"type": "enabled"}`，但 `messages` 中已存在不含 `thinking` 块的 `assistant` `tool_use` 历史消息（即处于工具调用续轮），自动在该步省略 `thinking` 字段（保留 `max_tokens`），避免无签名历史触发 400。

---

## P1（高优先级缺陷）

### P1-1：TUI `/connect` 模态弹窗内鼠标单击列表行会立即触发 `Enter` 执行动作（单击 `Disconnect` 直接删凭证）
- **涉及位置**：
  - `src/event/overlay.rs:358-361`（`click_connect`）
  - `src/app/update/chat/connect.rs:106-119`（`set_connect_selection`）
- **关键点**：
  - `set_connect_selection` 注释明确约定：“点击只移动 `❯`，不自动执行动作（与 `refdir` 菜单单击选中语义一致）”。
- **错误原因**：
  - `src/event/overlay.rs:360` 在调用 `set_connect_selection` 后又无条件调用了 `apply_connect_overlay_key(state, KeyCode::Enter, KeyModifiers::NONE)`，导致用户在 `Manage` 阶段单击 `Disconnect` 会直接删除凭证，在 `Providers` 阶段单击任意行会直接发起连接。
- **修复方案**：
  - 采用与 `refdir` 一致的选中/确认语义：若点击行**不是**当前 `active` 行，仅调用 `set_connect_selection` 移动 `❯` 焦点；仅当用户再次点击**已处于 `active` 焦点**的同一行（或按 `Enter`）时才触发 `Enter` 确认动作。

---

### P1-2：`app/` 状态层直接调用 `std::env::var` 违反纯状态边界，且导致单元测试受宿主环境变量污染而失败
- **涉及位置**：
  - `src/app/state/lx/connect.rs:132-135, 188-189`
  - `src/app/update/chat/connect.rs:33, 789`
  - `src/settings/service/models/tests/connect.rs:274-305`
- **关键点**：
  - 项目规范严格要求 `app/` 保持纯内存状态与更新，不得直接读取系统环境或磁盘 IO；测试必须与宿主开发机的环境变量隔离（hermetic）。
- **错误原因**：
  - `ConnectPanel::new` 与 `ConnectPanel::refresh_entries` 直接调用了 `std::env::var`；当宿主环境存在 `OPENAI_API_KEY` 时，`OPENAI_API_KEY=dummy cargo test --lib multi_method_provider_opens_methods_with_oauth_first` 与 `disconnect_removes_preset_and_clears_dangling_default_model` 均因为意外命中 `$OPENAI_API_KEY` 而失败。
- **修复方案**：
  1. 在 `AppState`（`src/app/state.rs`）中维护预设环境变量快照 `connect_env: BTreeMap<String, String>`（由 `config::ai::collect_preset_env_snapshot()` 在启动与刷新时采集注入；测试中 `AppState::new` 默认为空表，彻底与宿主环境隔离）；
  2. `ConnectPanel::new` 与 `refresh_entries` 改为接收 `&BTreeMap<String, String>` 快照，移除 `app/` 层所有 `std::env::var` 调用；
  3. 设置服务测试（`src/settings/service/models/tests/connect.rs`）选用不依赖常用宿主环境变量的预设或注入隔离环境校验。

---

### P1-3：TUI `Disconnect` 未清理 `ai.json` 悬空 `defaultModel`，且当环境变量同名存在时 `Disconnect` 无法断开连接
- **涉及位置**：
  - `src/event/chat/connect.rs:380-388`（`disconnect_preset`）
  - `src/settings/service/models/connect.rs:277-339`（`disconnect`、`commit_connect_change`）
  - `src/app/update/chat/connect.rs:198-205`
- **关键点**：
  - 当默认模型指向某快捷连接供应商时，断开该供应商后必须同步清理 `ai.json` 中悬空的 `defaultModel` / `titleSummary`，否则后续 `load_ai`（如打开设置页 `GET /api/models/settings`）会因悬空模型校验失败返回 HTTP 500；
  - 当某预设同时存在已保存凭证（API Key / OAuth）与进程环境变量 `$ENV`（或仅通过 `$ENV` 连接）时，点击 `Disconnect` 应当使该预设处于断开/禁用状态。
- **错误原因**：
  1. TUI 的 `disconnect_preset` 只修改了 `connect.json`，未像设置页那样清理并原子写回 `ai.json`；
  2. 设置页与 TUI 在 `has_credentials` 为真时直接执行 `settings.providers.remove(provider_id)`；若此时进程中恰好有同名 `$ENV`，删除记录后 `resolve_preset_connection_source` 立即回退命中 `$ENV`，导致断开操作失效；
  3. TUI 在仅通过 `$ENV` 连接时未在 `Manage` 菜单提供 `Disconnect` 选项。
- **修复方案**：
  1. 抽取公共函数 `config::ai::disconnect_preset_on_disk(ai_path, provider_id)`：若该预设命中环境变量，则保留记录并设 `enabled = false`、清空 `api_key` 与 `oauth`；否则移除记录；随后校验并写回清理悬空 `defaultModel`/`titleSummary` 后的 `ai.json`；
  2. TUI 与设置页统一调用该函数，且 TUI `Manage` 菜单对 `$ENV` 连接同样开放 `Disconnect`。

---

### P1-4：设置页保存順序（先 `ai.json` 后 `connect.json`）及 API Key / OAuth 凭证未互斥导致保存 422 或新 Key 不生效
- **涉及位置**：
  - `assets/settings_models.js:791-796`（`saveModels`）
  - `src/settings/service/models/connect.rs:220-227, 463-466`
- **关键点**：
  - 用户在设置页连接一个新的 Quick Connect 供应商并同时将其模型选为 `Default model` 后点击 `Save`；
  - 用户从 OAuth 切换为填写 API Key（或从 API Key 切换为 OAuth）。
- **错误原因**：
  1. `saveModels()` 先执行 `PUT /api/models/settings`（校验 `ai.json` 的 `defaultModel` 是否存在于磁盘上的 `connect.json`），后执行 `savePresetDrafts()`（写入 `connect.json`），导致第一步因 `connect.json` 尚未写入而报 HTTP 422；
  2. `put_connect` 写入非空 `api_key` 时未清空 `record.oauth`，`persist_oauth_credential` 写入 `oauth` 时未清空 `record.api_key`；由于 `resolve_preset_connection_source` 优先级为 `OAuth > ApiKey`，新填写的 API Key 会被旧 OAuth 凭证静默压住。
- **修复方案**：
  1. 调整 `assets/settings_models.js` 中 `saveModels()` 顺序：先执行 `await savePresetDrafts()` 落盘 `connect.json`，再在自定义供应商或模型设置脏时提交 `PUT /api/models/settings`；
  2. 在 `src/settings/service/models/connect.rs` 中确保写入非空 `api_key` 时置 `record.oauth = None`，写入 `oauth` 时置 `record.api_key = None`。

---

### P1-5：DigitalOcean 隐式 OAuth 回调被前置 `state` 校验拦截、Device Code 轮询缺少超时、GitHub Copilot Enterprise 授权域名未展开
- **涉及位置**：
  - `src/config/ai/oauth/browser.rs:212-234`
  - `src/config/ai/oauth/device.rs:45, 92-98, 169-205`
  - `src/config/ai/presets.rs:317-325`
- **关键点**：
  - DigitalOcean 采用隐式授权流（`token_url == ""`），浏览器首次 `GET /auth/callback` 时 `access_token` 与 `state` 位于 URL `#fragment` 中（服务端仅能收到无 query 的 GET，随后由返回的 HTML 脚本解析 fragment 并 `POST /auth/token`）；
  - Device Code 轮询需要受 10 分钟总超时保护；
  - GitHub Copilot Enterprise 用户填写 `enterpriseUrl` 后，Device Code 请求应发往企业域名而非固定 `github.com`，且默认 API 端点回退应为 `https://copilot-api.{domain}`。
- **错误原因**：
  1. `browser.rs:214` 在判断 `flow.token_url.is_empty()` 之前就检查了 `query.get("state")`，导致 DigitalOcean 的 `GET /auth/callback` 必然因无 query `state` 返回 400；
  2. `device.rs:169` 的 `poll_device_token` 循环只检查 `cancel.is_cancelled()`，未检查 `OAUTH_TIMEOUT` 截止时间；
  3. `device.rs` 未根据 `github-copilot` 的 `enterpriseUrl` 替换 `github.com` 域名，`presets.rs:321` 回退地址误写为 `https://api.{domain}`。
- **修复方案**：
  1. 在 `browser.rs` 中将 `if flow.token_url.is_empty()` 分支移到 `query.get("state")` 校验之前（`state` 仍在随后的 `POST /auth/token` 中严格校验）；
  2. 在 `device.rs` 的 `poll_device_token` 中加入 `Instant::now() >= deadline`（`OAUTH_TIMEOUT`）超时退出；
  3. 对 `github-copilot` 根据 `enterpriseUrl` 展开设备码与 Token URL，并将 `presets.rs` 中的回退地址修正为 `https://copilot-api.{domain}`。

---

## P2（中优先级缺陷）

### P2-1：后台模型刷新与写入路径可能覆盖并发修改或损坏的 `connect.json`
- **涉及位置**：
  - `src/config/ai/catalog_refresh.rs:350-414`（`refresh_connected_provider_models`）
  - `src/event/chat/connect.rs:327`、`src/settings/service/models/connect.rs:457`
- **错误原因**：
  1. `refresh_connected_provider_models` 在网络探测前读取 `connect.json`，耗时数秒探测完毕后将旧快照直接写回，若期间用户新增或修改了连接凭证会被旧快照覆盖；
  2. 写入路径使用了 `load_connect_or_default`，当 `connect.json` 出现语法损坏时会静默当成空表覆盖写回。
- **修复方案**：
  1. `refresh_connected_provider_models` 在网络探测完成后、写盘前重新调用 `load_connect` 读取最新 `connect.json`，仅将探测到的 `models` 合并进对应 provider 记录；
  2. 所有写路径改为使用严格 `load_connect`（仅文件不存在时初始化默认空表，文件损坏时返回明确错误拒绝覆盖）。

---

### P2-2：设置页修改连接后 TUI `state.connect` 未同步，且连接成功后未自动触发远端模型同步
- **涉及位置**：
  - `src/event/inventory.rs:66-70`、`src/app/update/chat/slash.rs:382-391`
  - `src/app/update/chat/connect.rs:731-762`（`on_preset_connected`）
- **错误原因**：
  1. `SettingsEvent::ModelsChanged` 仅更新了 `state.models.settings`，未重新加载 `state.connect` 与 `state.connect_env`，导致用户在网页设置页连接/断开供应商后，TUI `/connect` 弹窗仍显示旧状态；
  2. `on_preset_connected` 未入队 `Effect::RefreshModelsCatalog { provider_id: Some(provider_id) }`，与 `design.md` Q8“连接成功时自动同步”不一致。
- **修复方案**：
  1. 在 `SettingsEvent::ModelsChanged` 时同步重读 `connect.json` 与预设环境变量快照，并调用 `apply_models_changed_with_connect` 刷新 `state.connect`、`state.connect_env` 及已打开的 `Overlay::Connect`；
  2. 在 `on_preset_connected` 中自动推入 `Effect::RefreshModelsCatalog { provider_id: Some(provider_id) }`。

---

### P2-3：用户主动取消 OAuth 仍弹红色错误 Toast，且设置页 `PendingConnectOAuthMap` 终态条目未清理
- **涉及位置**：
  - `src/app/update/chat/connect.rs:858-888`（`on_preset_oauth_failed`）
  - `src/settings/service/models/connect.rs:477-492`（`oauth_status`）
- **错误原因**：
  1. 用户按 `Esc` 或点击模态框外部取消 OAuth 时，后台任务因 `cancel` 令牌触发返回 `Err("OAuth flow was cancelled")`，`on_preset_oauth_failed` 未区分用户取消与真实失败，无条件弹出红色错误 Toast；
  2. 设置页 `oauth_status` 在返回 `complete` 或 `failed` 终态后未从 `PendingConnectOAuthMap` 移除该 `attempt_id`，长期运行会累积内存。
- **修复方案**：
  1. 在 `on_preset_oauth_failed` 中若面板已不处于该 `attempt_id`（用户已主动退出该 OAuth 等待阶段）或错误为取消，则静默忽略不弹错误 Toast；
  2. 在 `oauth_status` 返回 `Complete` 或 `Failed` 时从 `PendingConnectOAuthMap` 移除已完成条目。

---

### P2-4：设置页将物化后的动态 Base URL / 默认模型表误存为用户覆盖值，且未校验模板占位符
- **涉及位置**：
  - `src/settings/service/models/connect.rs:91-97, 240-256`
  - `src/settings/service/models/mod.rs:453-493`
- **错误原因**：
  1. `preset_json` 返回物化后的 `baseUrl`（含 OAuth `apiEndpoint` 或展开后的模板 URL）与默认 `models`，前端保存时原样传回，后端将其写入 `record.base_url` 与 `record.models`，导致后续预设模型更新或动态端点变化被冻结；
  2. `put_connect` 未校验连接启用时模板占位符（如 `${accountId}`）是否已全部展开。
- **修复方案**：
  1. 在 `put_connect` 中：若传入的 `baseUrl` 等于 `spec.default_base_url` 或等于按表单/OAuth 展开后的默认地址，则保持 `record.base_url = None`；若传入的 `models` 与 `default_models_for_preset(spec.id)` 相等，则保持 `record.models = None`；
  2. 当该预设处于已连接状态时，使用 `resolve_preset_base_url` 校验最终地址不含未解析的 `${...}` 且通过 `validate_base_url`。

---

### P2-5：单元测试受本机 `~/.lx-t-dev/config/models_catalog_cache.json` 磁盘缓存影响、Responses `output_item.done` 缺少 `item.id` 映射、`saveAllDirty` 重复调用
- **涉及位置**：
  - `src/config/ai/presets.rs:507-518`
  - `src/chat/transport/responses/mod.rs:342-358, 435-445`
  - `assets/settings.js:250-266`、`assets/settings_models.js:975-984`
- **错误原因**：
  1. `build_catalog()` 在 `#[cfg(test)]` 下也会加载开发机真实的 `models_catalog_cache.json`，导致单测结果受本机缓存文件状态影响；
  2. Responses API 的 `response.output_item.done` 事件将 `id` 放在 `item.id` 而非顶层 `item_id`，当帧内不含 `output_index` 时 `ordinal_of` 无法命中；
  3. `createActionBar` 每次包装新的 `() => controller.save()` 闭包，导致 `saveAllDirty` 的 `seenSaves` `Set` 去重失效。
- **修复方案**：
  1. `build_catalog()` 在 `#[cfg(not(test))]` 下才自动叠加磁盘缓存（缓存合并逻辑由专门单元测试覆盖）；
  2. `responses/mod.rs` 的 `ordinal_of` 同时支持读取 `value.get("item").and_then(|item| json_str(item, "id"))`；
  3. `assets/settings.js` 的 `createActionBar` 直接保存 `controller.save` 引用（或以 `controller` 去重），避免重复提交。

---

## P3（代码规范、文案组织与文档同步）

### P3-1：`src/ui/text.rs` 超出 1000 行上限，需将 `/connect` 新增文案拆入 `src/ui/text/connect.rs`
- **涉及位置**：
  - `src/ui/text.rs:869-968`
- **错误原因**：
  - `src/ui/text.rs` 追加 `/connect` 文案后达到 1568 行；而 `src/ui/text/` 已采用子模块模式（`agent.rs`、`chat.rs`、`mcp.rs`、`skills.rs`、`todo.rs`）。
- **修复方案**：
  - 新建 `src/ui/text/connect.rs` 承载全部 `LX_CONNECT_*` 与 `lx_connect_*` 文案函数，并在 `src/ui/text.rs` 中 `pub mod connect; pub use connect::*;`；同时将 `LX_CONNECT_VERIFYING` 更名为语义准确的 `"Saving API key…"`。

---

### P3-2：`assets/settings.js:82` 存在 `host.innerHTML = ''`，且 `src/ui/overlay/connect.rs` Method 行长标签未按右侧 tag 裁剪
- **涉及位置**：
  - `assets/settings.js:82`
  - `src/ui/overlay/connect.rs:361-371`
- **错误原因**：
  1. `assets/settings.js:82` 使用了 `host.innerHTML = ''` 清空节点，违反 `docs/standards/html-assets.md` 禁止出现 `innerHTML` 的规范；
  2. `src/ui/overlay/connect.rs:361-371` 渲染认证方法行时先绘制右侧 tag，再对整行 `row` 绘制左侧 label，未用 `paint_tag` 返回的 `right` 边界裁剪 label 宽度。
- **修复方案**：
  1. 将 `assets/settings.js:82` 改为 `host.replaceChildren()`，并在 `src/settings/service/tests.rs` 中将无 `innerHTML` 检查覆盖到 `settings.js`；
  2. 在 `src/ui/overlay/connect.rs` 中使用 `right` 裁剪方法行的左侧 label 区域。

---

### P3-3：`design.md` 与 `task.md` 存在过期描述（仍写 `SlashPanel::Connect`、缺少四协议修复章节与最新文件表）
- **涉及位置**：
  - `docs/feature/lx-provider-connect/design.md`
  - `docs/feature/lx-provider-connect/task.md`
- **错误原因**：
  - Q6 改为 `Overlay::Connect` 居中模态弹窗、Q7 移除保存时网络探测、Q8 新增云端与四协议 `/models` 同步、以及四套传输协议（`anthropic`/`openai-responses`/`openai`/`google`）思考与工具 Schema 对齐完成后，`design.md` 的 §0/§1/§3/§4.3/§5/§6 与 `task.md` 的 Phase 2/Phase 4 未完整同步更新。
- **修复方案**：
  - 全面更新 `design.md` 与 `task.md`，统一为 `Overlay::Connect` 架构，新增 Phase 4（四协议思考、签名、Token 统计与工具 Schema 兼容修复）设计与任务记录，并校准全部文件路径与行数统计。
