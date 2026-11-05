# 预设 Provider 与 `/connect` 快捷连接（Preset Providers & `/connect`）— 执行任务清单 (`task.md`)

- **状态**：**已在工作区 `.worktrees/lx-provider-connect`（分支 `feature/lx-provider-connect`）全部完成并通过验证，待用户确认合并**
- **设计依据**：`docs/feature/lx-provider-connect/design.md`

---

## 0. 开工门禁与硬性规范

- **门禁**：必须等用户审核并明确确认本目录下的 `design.md` 与 `task.md` 后，才可修改 `src/`、`assets/` 与 `tests/` 代码。
- **架构红线**：
  - `app/` 只管纯内存状态与状态转换，严禁直接读写磁盘、发起网络请求或引用 `ui/`；
  - `ui/` 只读渲染，严禁修改状态或执行 IO；环境变量探测与文件加载统一在 `config/` / `event/` / `settings/` 完成并存入状态快照；
  - 所有 TUI 与浏览器设置页用户可见文案一律使用英文，TUI 文案集中定义在 `src/ui/text.rs`；
  - `lx` 窗格内反馈一律使用 `pane.lx.toast`（`show_lx_toast`），严禁使用全局 `AppState.toast`；
  - 生产代码单文件严禁超过 1000 行，严禁使用 `unwrap()` / `expect()`；
  - 浏览器前端资源严禁使用 `innerHTML`、严禁引入外部 CDN/链接；
  - 未获明确授权禁止执行任何修改 Git 状态的命令。

---

## 1. 需求与边界确认清单（Grill Me 已确认）

- [x] **Q1（预设供应商目录与排序规则）**：完全对齐 `opencode-dev` 的预设供应商目录（含默认 `baseURL`、协议 `ProviderTransport`、环境变量 `env`、认证方式与内置主力模型）；**取消 `Popular` / `Services` 分组，所有预设供应商统一按名称首字母（A-Z）排序**。
- [x] **Q2（认证方式覆盖范围）**：完全对齐 `opencode-dev`：支持 **API Key**、**附加表单字段（URL / Domain / Account ID 等）**、**OAuth URL 授权（Device Code 设备码轮询 + Browser PKCE 本地回环回调）** 以及 **环境变量（`$ENV`）自动探测**；多认证方式时 OAuth 排在 API Key 之前。
- [x] **Q3（TUI `/connect` 界面容器与四步流转）**：在 `src/ui/main_content/lx/` 复用输入框上方无边框 `SlashPanel::Connect` + 底部输入框联动：
  - 阶段 1 `Providers`：A-Z 列表 + 实时打字过滤，已连接项显示 `✓` 与连接方式 tag；点选已连接项可进入 `Select model` / `Reconnect` / `Disconnect` 管理；
  - 阶段 2 `Methods`：多方式时选择认证方式（单方式自动跳过）；
  - 阶段 3 `Auth`：表单/API Key 通过底部输入框单行提交；OAuth URL 在面板内展示授权链接与设备码（`Enter`/`o` 打开浏览器、`c` 复制设备码或链接、后台自动轮询）；
  - 阶段 4 连接成功后在窗格内弹 `Connected <Provider>` Toast，并自动将面板切换为仅过滤该供应商的 `/model` 模型选择器。
- [x] **Q4（设置页 Provider 双 Tab 与编辑区交互）**：
  - 左侧列表顶部新增 `Quick Connect`（默认）与 `Custom` 双 Tab 切换；
  - `Quick Connect` Tab 顶部带搜索框，列出 A-Z 预设供应商并标记 `Connected · API key` / `Connected · OAuth` / `Connected · $ENV` / `Not connected`，隐藏 `+ New Provider`；`Custom` Tab 仅列自定义供应商并保留 `+ New Provider`；
  - 右侧编辑区在 `Quick Connect` 下顶部显示 `Quick Connect Preset` 说明横幅，锁定 `Provider ID` 与 `Type`，提供 OAuth 浏览器授权按钮与 API Key 输入，右上角按钮改为 `Cancel` / `Reset to Preset` / `Disconnect`。
- [x] **Q5（独立 JSON 持久化存储）**：
  - 快捷连接与自定义供应商分文件存储：自定义保持在 `~/.lx-t[-dev]/config/ai.json`，快捷连接独立保存在 **`~/.lx-t[-dev]/config/connect.json`**（`0600` 权限 + 同目录临时文件原子写入），未连接的预设不落盘；
  - 运行时 `load_runtime_ai_settings()` 自动合并两份配置供 `/model`、聊天流、标题生成、子 Agent 与无头 CLI 统一消费。

---

## 2. 分阶段实施步骤拆解

### Phase 1：预设目录提取、`connect.json` 独立持久化与 OAuth 核心层（`src/config/ai/`）

> 目标：构建对齐 `opencode-dev` 的 A-Z 预设供应商目录、独立 `connect.json` 读写与运行时配置合并，以及 OAuth 授权与刷新能力。

- [x] **1.1 生成并嵌入预设供应商目录（`src/config/ai/presets.rs` & `src/config/ai/presets_catalog.json`）**
  - 从 `/Users/yonah/projects/agent/opencode-dev/packages/core/src/models-dev/snapshot.txt` 及 `packages/core/src/plugin/provider/*.ts` 提取兼容供应商与主力非废弃 Chat 模型（含 `limit`、`modalities`、`variants`、环境变量 `env`、认证方法 `methods` 与简介 `note`）。
  - 按 `name` 首字母 A-Z 排序，暴露 `all_presets()`、`find_preset(id)`、`default_models_for_preset(id)`。
- [x] **1.2 实现独立 `connect.json` 存储与运行时合并（`src/config/ai/connect.rs` & `src/config/ai/mod.rs`）**
  - 定义 `ConnectSettings`、`ConnectedPresetRecord`、`OAuthCredential`、`ConnectionSource`。
  - 实现 `connect_config_path()`、`load_connect()`、`save_connect()`（`0600` 权限 + 同目录临时文件原子替换）。
  - 实现 `resolve_preset_connection_source(spec, record)`（优先级：显式 `oauth` > 显式 `apiKey` > 进程环境变量 `$ENV`）。
  - 实现 `load_runtime_ai_settings()`：合并 `ai.json` 自定义供应商与已激活的快捷连接供应商，统一校验 `defaultModel` / `titleSummary`。
- [x] **1.3 实现 OAuth 授权流与凭证刷新（`src/config/ai/oauth.rs`）**
  - 实现 `OAuthFlowKind::DeviceCode`（设备码请求与轮询）与 `OAuthFlowKind::BrowserPkce`（PKCE 生成、`127.0.0.1` 回环监听与 code 换 token）。
  - 实现 `refresh_oauth_credential` 过期前自动刷新并原子写回 `connect.json`。
- [x] **1.4 Phase 1 单元测试（`src/config/ai/tests.rs`）**
  - 验证预设列表 A-Z 排序、ID 唯一性、默认模型与 variants 合法性；
  - 验证 `connect.json` 独立读写不改写 `ai.json`、合并后 `ModelsSnapshot` 可正常解析快捷连接模型；
  - 验证环境变量探测与 `Disconnect` 清理逻辑。

---

### Phase 2：TUI `/connect` 斜杠命令、状态机与事件编排（`src/app/` & `src/event/` & `src/ui/`）

> 目标：在 `lx` 窗格内实现 `/connect` 命令及四步无边框面板交互，连接成功后自动跳转到该供应商的 `/model` 选择器。

- [x] **2.1 新增文案常量与命令定义（`src/ui/text.rs`、`src/app/state/lx/slash.rs`、`src/settings/commands.rs`）**
  - 在 `src/ui/text.rs` 新增 `/connect` 相关英文文案常量（`LX_CONNECT_COMMAND_DESC`、`LX_CONNECT_PANEL_TITLE`、`LX_CONNECT_MANAGE_TITLE`、`LX_CONNECT_METHOD_TITLE`、`LX_CONNECT_OAUTH_FOOTER` 等）。
  - 在 `SlashCommand` 枚举新增 `Connect`，更新 `SlashCommand::ALL`；在 `src/settings/commands.rs` 的 `RESERVED_NAMES` 加入 `"connect"`。
- [x] **2.2 新建 `/connect` 面板状态模型（`src/app/state/lx/connect.rs`）**
  - 定义 `ConnectPanel`、`ConnectStage`（`Providers` / `Manage` / `Methods` / `InputPrompt` / `OAuthWaiting`）、`ConnectProviderEntry`、`ConnectManageAction`、`ConnectPromptStep`。
  - 实现 A-Z 条目构造、实时过滤 `refilter(query)`、高亮窗口移动 `move_active`、`current_index` 与 `is_placeholder`。
  - 在 `SlashPanel`（`src/app/state/lx/slash.rs`）中接入 `Connect(Box<ConnectPanel>)`。
- [x] **2.3 实现 `/connect` 状态更新与 `/model` 联动（`src/app/update/chat/connect.rs`、`src/app/update/chat/slash.rs`、`src/app/update/pane.rs`）**
  - 实现 `open_connect_panel`、`apply_connect_enter`、`apply_connect_esc`、`apply_connect_char_shortcut`（`OAuthWaiting` 下 `o` 打开浏览器、`c` 复制设备码/URL）。
  - 为 `ModelPicker` 新增 `for_provider(settings, current, provider_id)`，支持连接成功后仅展示该供应商的模型列表。
  - 将 `src/app/update/pane.rs`（当前 989 行）中的斜杠面板通用辅助函数下沉至 `src/app/update/chat/slash.rs`，保持 `pane.rs` 低于 950 行。
- [x] **2.4 接入异步效应与事件循环（`src/app/effects.rs`、`src/event/chat/connect.rs`、`src/event/mod.rs`）**
  - 在 `Effect` 中新增 `SavePresetConnection`、`DisconnectPreset`、`StartPresetOAuth`、`CancelPresetOAuth`。
  - 在 `src/event/chat/connect.rs` 执行 `connect.json` 写入与 OAuth 后台任务，完成后回投 `AppEvent::PresetConnected` / `AppEvent::PresetDisconnected` / `AppEvent::PresetOAuthPrompt` / `AppEvent::PresetOAuthFailed`。
- [x] **2.5 实现 `/connect` 面板渲染（`src/ui/main_content/lx/slash.rs`）**
  - 渲染 `SlashPanel::Connect` 各阶段的 `CommandItem` 列表（`Providers` 阶段显示名称、简介与右侧 `✓ API key` / `✓ OAuth` / `$ENV` 标签；`OAuthWaiting` 阶段显示 URL、设备码与等待状态）、动态面板标题与底栏提示。
- [x] **2.6 Phase 2 单元测试**
  - `/connect` 命令匹配、面板打开、A-Z 列表搜索过滤、多阶段 `Enter` 推进与 `Esc` 逐级回退；
  - API Key 提交、已连接项 `Disconnect` / `Reconnect`、连接成功后自动切入 `SlashPanel::Model` 并弹 `pane.lx.toast`。

---

### Phase 3：设置页后端 API 与前端双 Tab 界面（`src/settings/` & `assets/`）

> 目标：在设置页实现 `Quick Connect` / `Custom` 双 Tab 隔离展示、快捷连接专属横幅说明、OAuth 网页端登录及 `connect.json` 独立保存。

- [x] **3.1 扩展设置服务 API（`src/settings/service/models/mod.rs` & `src/settings/service/models/connect.rs` & `src/settings/service/page.rs`）**
  - `GET /api/models/settings` 新增 `quickConnect` 数组返回全部预设供应商及其连接状态，同时保持 `providers` 仅返回 `ai.json` 中的自定义供应商。
  - 在 `src/settings/service/models/connect.rs` 实现：
    - `PUT /api/models/connect`（保存快捷连接项到 `connect.json` 并广播 `ModelsChanged`）；
    - `POST /api/models/connect/disconnect`（断开快捷连接并清理 `connect.json`）；
    - `POST /api/models/connect/oauth/start`、`GET /api/models/connect/oauth/status`、`POST /api/models/connect/oauth/complete`、`DELETE /api/models/connect/oauth`。
  - 在 `src/settings/service/page.rs` 注册 `"/settings_connect.js"`。
  - 更新 `src/settings/service/subagents.rs` 与 `src/event/mod.rs` 的 `load_models_snapshot` 使用合并后的 `load_runtime_ai_settings()`。
- [x] **3.2 更新设置页 HTML 与 CSS（`assets/settings.html` & `assets/settings.css`）**
  - 在 `nav.models-nav` 顶部增加 `Quick Connect` / `Custom` 双 Tab 按钮（`#prov-tab-quick` / `#prov-tab-custom`）与搜索框 `#provider-search`。
  - 在 `#provider-editor` 顶部增加 `#preset-banner`（快捷连接说明横幅）、`#preset-editor-actions`（`Cancel` / `Reset to Preset` / `Disconnect`）、`#preset-oauth-card`（OAuth 授权按钮与设备码/链接展示区）和 `#preset-form-fields`。
  - 在 `assets/settings.css` 增加双 Tab、搜索框、快捷连接横幅与状态徽章样式。
- [x] **3.3 实现前端 `Quick Connect` 交互脚本（`assets/settings_connect.js` & `assets/settings_models.js`）**
  - 实现 `switchProviderTab('quick' | 'custom')`：切换左侧列表数据源、显隐 `+ New Provider` 按钮与右侧编辑区面板。
  - 实现 `Quick Connect` 列表渲染（已连接项置顶 + A-Z 排序，搜索词实时过滤，行尾标签明确区分 `Connected · API key` / `Connected · OAuth` / `Connected · $ENV` / `Not connected`）。
  - 实现右侧 `Quick Connect` 编辑区：渲染预设说明横幅、支持填写 API Key / 附加表单字段、支持点击发起 OAuth 授权并轮询状态、支持 `Disconnect` 与 `Reset to Preset`、接入顶部 `Save` / `Reset` ActionBar。
  - 更新 `Model` Tab 的默认模型与标题模型下拉框，使其同时包含已启用快捷连接供应商的模型与自定义供应商的模型。
- [x] **3.4 Phase 3 单元与集成测试（`src/settings/service/models/tests.rs` & `src/settings/service/tests.rs`）**
  - 验证 `GET /api/models/settings` 返回 `quickConnect` 与 `providers` 分离结构；
  - 验证 `PUT /api/models/connect` 与 `POST /api/models/connect/disconnect` 正确读写 `connect.json` 且不污染 `ai.json`；
  - 验证 `/settings_connect.js` 静态路由、无外部链接、无 `innerHTML` 断言全部通过。

---

## 3. 验证命令

完成所有阶段后按影响范围与全量要求依次执行：

```bash
rtk cargo fmt --check
rtk cargo clippy --all-targets -- -D warnings
rtk cargo test
```

---

## 4. 完成定义（DoD）

- [x] 预设供应商目录与认证方式对齐 `opencode-dev`，按首字母 A-Z 排序、无分组。
- [x] 快捷连接配置独立保存在 `~/.lx-t[-dev]/config/connect.json`，自定义供应商保持在 `~/.lx-t[-dev]/config/ai.json`，运行时自动合并。
- [x] 在 `lx` 窗格输入 `/connect` 可打开无边框快捷连接面板，支持 A-Z 搜索过滤、API Key 输入、附加表单输入、OAuth URL/设备码授权（打开浏览器/复制代码）、已连接项重连/断开，并在连接成功后弹窗格内 Toast 及自动切入该供应商的 `/model` 模型选择面板。
- [x] 浏览器设置页 `Provider` 视图左侧列表具备 `Quick Connect` / `Custom` 双 Tab 切换与搜索过滤，两个列表彻底分开；右侧详情区明确说明快捷连接属性与连接方式，支持一键 `Disconnect`、`Reset to Preset`、OAuth 授权与 API Key 保存。
- [x] 所有生产代码单文件 ≤ 1000 行，无 `unwrap()` / `expect()`，`cargo fmt`、`cargo clippy --all-targets -- -D warnings`、`cargo test` 全部通过。
