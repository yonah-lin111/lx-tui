# 预设 Provider 与 `/connect` 快捷连接（Preset Providers & `/connect`）— 详细设计 (`design.md`)

- **特性目录**：`docs/feature/lx-provider-connect/`
- **关联任务清单**：`docs/feature/lx-provider-connect/task.md`
- **当前状态**：**已在工作区 `.worktrees/lx-provider-connect`（分支 `feature/lx-provider-connect`）完成实现与测试验证，待用户确认合并**
- **参考实现**：`/Users/yonah/projects/agent/opencode-dev`
  - TUI `/connect`：`packages/tui/src/component/dialog-integration.tsx`、`packages/tui/src/app.tsx:1018-1030`
  - 预设目录与协议映射：`packages/core/src/models-dev.ts`、`packages/core/src/plugin/models-dev.ts`、`packages/core/src/variant.ts`、`packages/core/src/models-dev/snapshot.txt`
  - 供应商认证插件：`packages/core/src/plugin/provider/{opencode,openai,chatgpt,github-copilot,xai,poe,gitlab,digitalocean,cloudflare-ai-gateway,azure}.ts`
  - 设置页供应商管理：`packages/app/src/settings/providers/providers.tsx`、`packages/app/src/providers/connect/dialog.tsx`
- **目标范围**：
  - `src/config/ai/`（`mod.rs`、新增 `presets.rs`、新增 `connect.rs`、新增 `oauth/{mod,browser,device}.rs`、新增 `verify.rs`、新增 `catalog_refresh.rs`）
  - `src/app/`（`state.rs`、`effects.rs`、`state/lx/slash.rs`、新增 `state/lx/connect.rs`、`update/pane.rs`、`update/chat/slash.rs`、新增 `update/chat/connect.rs`）
  - `src/event/`（`mod.rs`、`inventory.rs`、新增 `chat/connect.rs`）
  - `src/ui/`（`text.rs`、新增 `text/connect.rs`、新增 `overlay/connect.rs`）
  - `src/settings/`（`commands.rs`、`service.rs`、`service/page.rs`、`service/models/mod.rs`、新增 `service/models/connect.rs`）
  - `assets/`（`settings.html`、`settings.css`、`settings_models.js`、新增 `settings_connect.js`）
- **架构约束**：
  - 严格保持 `app/`（纯状态与更新，禁止 IO）、`ui/`（纯渲染只读状态，禁止修改状态或执行 IO）、`event/` & `settings/`（异步网络与持久化编排）边界清晰；
  - 所有 TUI 与浏览器设置页用户可见文案一律使用英文，TUI 文案集中定义在 `src/ui/text.rs`；
  - 生产代码单文件行数上限 1000 行（当前 `assets/settings_models.js` 997 行、`assets/settings.js` 1000 行、`src/app/update/chat/connect.rs` 932 行，接近上限，后续改动必须按职责拆分），禁止 `unwrap()` / `expect()`；
  - 浏览器前端资源遵循 `docs/standards/html-assets.md`，严禁外部链接、严禁 `innerHTML`，新增脚本 `assets/settings_connect.js` 在 `src/settings/service/page.rs` 静态白名单注册。

---

## 0. 现状核实与事实基线（CodeGraph + Codebase Memory 实测）

| 维度 | 现状实测（带文件与行号） | 对本特性的设计影响 |
| --- | --- | --- |
| **`opencode-dev` 预设目录与 `/connect`** | `packages/core/src/plugin/models-dev.ts:40-58` 将 `models.dev` 快照转换为带 `key` + `env` 方法的预设集成；`packages/core/src/plugin/provider/*.ts` 为 `opencode`、`openai`、`github-copilot`、`xai`、`poe`、`gitlab`、`digitalocean`、`cloudflare-ai-gateway` 等注册 OAuth（Browser PKCE / Device Code）与附加 `form` 字段（如 Enterprise URL、Account ID）；`dialog-integration.tsx:1249-1283` 连接成功后立即跳转到该 Provider 的 `DialogModel` 选择模型。 | 1. 提取并内联对齐 `opencode-dev` 的预设供应商目录（含默认 `baseURL`、协议 `transport`、环境变量 `env`、认证方法 `methods` 与内置主干模型）；<br>2. 按用户确认，**取消 `Popular`/`Services` 分组，统一按供应商名称首字母（A-Z）排序**；<br>3. 完整支持 API Key、附加表单字段（如 URL/Account ID）、OAuth（浏览器回调 / 设备码 URL 认证）与环境变量 `$ENV` 探测；<br>4. 连接成功后自动切入该供应商的 `/model` 选择面板。 |
| **`lx-tui` 模型配置存储** | `src/config/ai/mod.rs:309-350, 581-636`：当前所有自定义供应商与全局 `defaultModel`/`titleSummary` 均保存在单一 `~/.lx-t[-dev]/config/ai.json` 中（`0600` 权限原子写）。 | 按用户确认：**快捷连接（Quick Connect）的 JSON 与自定义（Custom）分离存储**。自定义供应商继续存 `config/ai.json`；快捷连接的凭证（API Key / OAuth Token）、启用态、端点覆盖与模型增补独立存放在 `~/.lx-t[-dev]/config/connect.json`（同为 `0600` 原子写）。运行时 `ModelsSnapshot` 自动合并两者。 |
| **`lx-tui` 斜杠命令与面板体系** | `src/app/state/lx/slash.rs:75-107` 定义 `SlashCommand` 枚举（当前 8 个：`Model, Resume, Fork, New, Rewind, Compact, Subagents, Rename`）；`src/ui/main_content/lx/slash.rs` 渲染输入框上方的 `SlashPanel`，`src/ui/overlay/connect.rs` 渲染 `/connect` 居中模态弹窗。 | 1. `SlashCommand` 新增 `Connect`（`/connect`），`RESERVED_NAMES`（`src/settings/commands.rs:222`）同步加入 `"connect"`；<br>2. 新增居中模态弹窗 **`Overlay::Connect(Box<ConnectPanel>)`**（`src/ui/overlay/connect.rs`），由 `src/event/overlay.rs` 路由键盘/鼠标；连接成功后关闭弹窗并把 `pane.lx.slash` 切到 `SlashPanel::Model(ModelPicker::for_provider(...))`。 |
| **设置页 Provider 视图与 API** | `src/settings/service/models/mod.rs:18-25` 提供 `GET/PUT /api/models/settings` 与 `POST /api/models/fetch`；`assets/settings.html` 与 `assets/settings_models.js`（997 行）渲染 `#provider-list`。 | 1. 左侧列表 `nav.models-nav` 顶部新增 `Quick Connect` / `Custom` 双 Tab 切换与搜索过滤框；<br>2. 右侧详情区为 `Quick Connect` 项展示专属预设说明横幅、连接状态标签、OAuth 登录弹窗/按钮、`Disconnect` 与 `Reset to Preset` 动作；<br>3. 前端逻辑拆入 `assets/settings_connect.js`，后端新增 `src/settings/service/models/connect.rs` 处理 `connect.json` 读写与 OAuth 会话。 |
| **单文件行数水位** | `src/app/update/pane.rs`：823 行；`src/app/state/lx/slash.rs`：909 行；`src/config/ai/mod.rs`：799 行；`src/settings/service/models/mod.rs`：601 行。 | 严守 1000 行红线：新增逻辑全部落入职责清晰的子模块（`config/ai/{presets,connect}.rs`、`config/ai/oauth/{mod,browser,device}.rs`、`app/state/lx/connect.rs`、`app/update/chat/connect.rs`、`event/chat/connect.rs`、`ui/overlay/connect.rs`、`settings/service/models/connect.rs`、`assets/settings_connect.js`），并将 `pane.rs` 中斜杠面板辅助函数下沉到 `update/chat/slash.rs`，确保所有生产文件 ≤ 1000 行。 |

---

## 1. 背景与目标 / 非目标

### 1.1 目标
1. **对齐 `opencode-dev` 的预设供应商目录与多认证能力**：
   - 内置与 `opencode-dev` 对齐的预设供应商目录（含默认 `baseURL`、传输协议 `ProviderTransport`、环境变量名列表 `env`、认证方式 `methods`、简介说明 `note` 及预置模型配置）；
   - 预设供应商列表统一按**名称首字母（A-Z，不区分大小写）排序**，**不设 `Popular` / `Services` 分组**；
   - 支持 **API Key 认证**、**带附加 URL/参数表单的认证**（如 GitHub Copilot Enterprise URL、GitLab Instance URL、Cloudflare Account/Gateway ID、自定义 Base URL）以及 **OAuth URL 认证**（Device Code 设备码流与本地回环 PKCE 浏览器授权流）和 **环境变量 `$ENV` 自动探测**。
2. **新增 `lx` 窗格 `/connect` 斜杠命令**：
   - 在 `lx` 窗格支持 `/connect` 命令，打开居中模态弹窗 `Overlay::Connect(Box<ConnectPanel>)`（`src/ui/overlay/connect.rs`），弹窗内自带输入框与状态行；
   - 完整支持「A-Z 搜索选择供应商 → 已连接项管理（`Reconnect` / `Disconnect`）或认证方式选择（OAuth 优先、API Key 次之）→ 表单/API Key 输入或 OAuth URL/设备码展示（`Enter`/`o` 打开浏览器、`c` 复制设备码或链接、后台轮询）→ 成功后弹窗格内 Toast 并自动切换到该供应商的 `/model` 选择器」。
3. **独立 `connect.json` 持久化存储**：
   - 快捷连接供应商的配置与凭证独立保存在 `~/.lx-t[-dev]/config/connect.json`（`0600` 权限、同目录临时文件原子写入），与自定义供应商的 `~/.lx-t[-dev]/config/ai.json` 物理隔离；
   - 未连接的预设供应商仅驻留二进制只读内存，不落盘；运行时模型快照 `ModelsSnapshot` 自动合并 `connect.json`（及命中环境变量的预设）与 `ai.json`。
4. **设置页 Provider 双 Tab（`Quick Connect` / `Custom`）与快捷连接专属说明**：
   - 在 `assets/settings.html` 的 Provider 列表顶部增加 `Quick Connect` 与 `Custom` 两个 Tab 切换，使快捷连接与自定义供应商彻底分列；
   - `Quick Connect` Tab 提供搜索过滤框，每项明确标注 `Connected · API key` / `Connected · OAuth` / `Connected · $ENV` / `Not connected`；
   - 右侧编辑区在选中 `Quick Connect` 项时，顶部展示醒目的 `Quick Connect Preset` 说明横幅（说明其为内置快捷连接及支持的认证方式），提供 API Key 输入、OAuth 浏览器授权按钮、`Disconnect`（断开并清理凭证）与 `Reset to Preset`（恢复预设地址与模型），并隐藏自定义专属的 `+ New Provider` / `Duplicate` / `Delete`。

### 1.2 非目标
- 不依赖外部运行时 SDK（如 AWS Bedrock SigV4 原生二进制 SDK 或本地 `claude` CLI 子进程代理），仅对兼容 HTTP/SSE 传输协议的预设供应商提供开箱即用连接。
- 不改变现有 `ai.json` 的磁盘结构，用户已配置的自定义 Provider（如 `provider-alq374`）零迁移成本继续在 `Custom` Tab 下工作。

---

## 2. 已确认的边界决策汇总（Grill Me 结论表）

| 编号 | 决策维度 | 已确认方案 |
| --- | --- | --- |
| **Q1** | **预设供应商目录与排序规则** | 直接对齐 `opencode-dev` 的预设供应商目录（名称、协议、默认 `baseURL`、环境变量名、认证方法、主力模型及 `variants`/`limit`/`modalities`）；**不设 `Popular` / `Services` 分组，所有预设供应商统一按名称首字母（A-Z）排序**。 |
| **Q2** | **认证方式覆盖范围** | 完全对齐 `opencode-dev`：支持 **API Key**、**附加表单字段（URL / Domain / Account ID 等）**、**OAuth URL 授权（含 Device Code 设备码轮询与本地回环 Browser PKCE 回调）** 以及 **进程环境变量（`$ENV`）自动探测**。当同一供应商有多种交互式认证方式时，OAuth 排在 API Key 之前。 |
| **Q3** | **TUI `/connect` 界面容器与四步流转** | 收口为居中模态弹窗 `Overlay::Connect(Box<ConnectPanel>)`（`src/ui/overlay/connect.rs`），弹窗内自带输入框与列表区：<br>1. `Providers`：A-Z 列表 + 弹窗输入框实时过滤，已连接项显示 `✓` 与连接方式 tag；点选已连接项进入 `Reconnect` / `Disconnect` 管理；<br>2. `Methods`：多认证方式时列出选择，单方式自动跳过；<br>3. `Auth`：表单/API Key 通过弹窗输入行提交；OAuth URL 在面板内展示授权链接与设备码（`Enter`/`o` 打开浏览器、`c` 复制设备码或 URL、后台自动轮询）；<br>4. 连接成功后关闭弹窗、在窗格内弹 `Connected <Provider>` Toast，并将 `pane.lx.slash` 切换为仅过滤该供应商的 `/model` 模型选择器。 |
| **Q4** | **设置页 Provider 双 Tab 与编辑区交互** | 1. 左侧列表顶部新增 `Quick Connect`（默认）与 `Custom` 双 Tab 切换；<br>2. `Quick Connect` Tab 顶部带搜索框，列出 A-Z 预设供应商并标记 `Connected · API key` / `Connected · OAuth` / `Connected · $ENV` / `Not connected`，隐藏 `+ New Provider`；`Custom` Tab 仅列自定义供应商并保留 `+ New Provider`；<br>3. 右侧编辑区在 `Quick Connect` 下顶部展示 `Quick Connect Preset` 说明横幅，锁定 `Provider ID` 与 `Type`，提供 OAuth 授权按钮与 API Key 输入，右上角按钮改为 `Cancel` / `Reset to Preset` / `Disconnect`。 |
| **Q5** | **独立 JSON 持久化存储与兼容** | **快捷连接与自定义供应商分文件存储**：<br>1. 自定义供应商与全局 `defaultModel`/`titleSummary`/`webSearch` 保持在 `~/.lx-t[-dev]/config/ai.json`；<br>2. 快捷连接的已连接凭证（API Key / OAuth Token）、启用开关、Base URL 覆盖与模型增补独立存储在 **`~/.lx-t[-dev]/config/connect.json`**（`0600` 权限 + 同目录原子写）；未连接的预设不落盘；<br>3. 运行时 `load_runtime_ai_settings()` 将内置预设 + `connect.json` + 环境变量探测与 `ai.json` 合并为统一 `AiSettings` 快照，对 `/model`、聊天流、标题生成、子 Agent、无头 CLI 完全透明。 |
| **Q6** | **`/connect` 居中模态弹窗（参考 `refdir`）** | 将 `/connect` 从输入框上方无边框 `SlashPanel` 改为**独立的居中模态弹窗 `Overlay::Connect(Box<ConnectPanel>)`**（尺寸 `72 × 20`，完全参考 `src/ui/overlay/referenced/{search,menu,input}.rs`）：<br>1. 背景压暗（`widgets::modal::dim_background`）+ 强调色边框（`Block::bordered().border_style(style::accent())`）；<br>2. 弹窗内部顶部自带 `>` 单行输入框（`dialog::render_input`），光标直接聚焦在弹窗内：在 `Providers` 阶段用于实时过滤供应商，在 `InputPrompt` 阶段用于输入/粘贴 API Key 或表单参数；<br>3. 输入框下方预留实时状态/错误行（校验中显示 `Verifying API key…`，失败显示红字错误并保留已输入文本重试，不关闭弹窗）；<br>4. 列表区复用 `refdir` 的 `❯` 选中标记、悬停底色（`style::overlay_selection()`）、右侧连接状态标签、右缘滚动条与底栏快捷键提示。 |
| **Q7** | **API Key 保存语义（对齐 `opencode-dev`）** | 完全对齐 `opencode-dev`（`Integration.connection.key`）：保存 API Key 时仅校验非空与前置表单/Base URL 合法性，**不做阻塞网络鉴权探测**，直接保存凭证并进入模型选择。 |
| **Q8** | **拉取最新模型（对齐 `opencode-dev` 双源同步）** | 1. **云端 `models.dev` 目录同步**：支持从 `https://models.opencode.ai/experimental/v2.0/api.json?type=chat,decision,image`（回退 `https://models.dev/api.json`）拉取最新预设模型目录（含 `limit`、`modalities`、`variants`），带 `ETag` 缓存在 `~/.lx-t[-dev]/config/models_catalog_cache.json` 并热更新内存目录；<br>2. **全协议 `/models` 实时发现**：支持 `openai`、`openai-compatible`、`anthropic`、`google` 四种协议拉取远端模型并与目录元数据合并；<br>3. **触发入口**：TUI 启动后台静默刷新 + `/connect` 连接成功时自动同步 + `/connect` 弹窗内按 `Ctrl+R`（或管理菜单 `Refresh models`）手动刷新 + 设置页全协议 `Fetch models`。 |

---

## 3. 核心架构与数据流

```text
┌──────────────────────────────────────────────────────────────────────────────────────────┐
│ 1. 双文件配置与运行时合并链路 (`src/config/ai/`)                                         │
│                                                                                          │
│   内置只读预设目录 (PRESET_PROVIDERS, A-Z 排序)                                          │
│          │                                                                               │
│          ├──► 叠加 `~/.lx-t[-dev]/config/connect.json` (已连接凭证: API Key / OAuth / 覆盖)│
│          ├──► 探测 `std::env::var(env_name)` (未存凭证但命中环境变量时标记 Connected·$ENV) │
│          │                                                                               │
│          ▼                                                                               │
│   已激活的快捷连接 Provider 集合  ───┐                                                   │
│                                      ├──► `load_runtime_ai_settings()`                   │
│   `~/.lx-t[-dev]/config/ai.json`  ───┘      (合并 providers + enabled_providers,         │
│   (Custom Providers + defaultModel)          校验 defaultModel / titleSummary)           │
│                                                     │                                    │
│                                                     ▼                                    │
│                                        `ModelsSnapshot` (TUI / CLI / Subagent 统一消费)   │
├──────────────────────────────────────────────────────────────────────────────────────────┤
│ 2. TUI `/connect` 四步状态机链路 (`src/app/` ↔ `src/event/` ↔ `src/ui/overlay/`)      │
│                                                                                          │
│   用户输入 `/connect` + Enter                                                            │
│     └─► `open_connect_panel(state, pane)`                                                │
│           └─► `Overlay::Connect(Box<ConnectPanel>)` (居中模态弹窗)                      │
│                 ├─ [已连接项 Enter] ──► `ConnectStage::Manage { provider_id }`           │
│                 │                        ├─ `Reconnect` ──► 进入 Methods / Auth          │
│                 │                        └─ `Disconnect` ──► `Effect::DisconnectPreset`  │
│                 └─ [未连接项 Enter] ──► (单方法跳过 Methods)                              │
│                       ├─ `ConnectMethod::ApiKey { form }`                                │
│                       │    ├─ 若有 form 字段 ──► `ConnectStage::FormField` (底部输入)     │
│                       │    └─ `ConnectStage::ApiKeyInput` (底部输入 API Key + Enter)     │
│                       │         └─► `Effect::SavePresetConnection`                       │
│                       └─ `ConnectMethod::OAuth { id, label, kind, form }`                │
│                            ├─ 若有 form 字段 ──► `ConnectStage::FormField`               │
│                            └─ `Effect::StartPresetOAuth`                                 │
│                                 └─► `ConnectStage::OAuthWaiting { url, code, mode }`     │
│                                       ├─ `Enter`/`o`: `platform::open_browser(&url)`     │
│                                       ├─ `c`: 复制 `user_code` 或 `url` + Toast          │
│                                       └─ 后台完成回调 `AppEvent::PresetOAuthFinished`    │
│                                                                                          │
│   连接保存完成 (`on_preset_connected`):                                                  │
│     ├─ 刷新 `state.models.settings` + 通知设置页服务                                     │
│     ├─ 窗格内弹 Toast: `Connected <Provider Name>`                                       │
│     └─ 自动切换面板为 `SlashPanel::Model(ModelPicker::for_provider(provider_id))`        │
├──────────────────────────────────────────────────────────────────────────────────────────┤
│ 3. 浏览器设置页双 Tab 链路 (`src/settings/service/models/` ↔ `assets/settings_*.js`)     │
│                                                                                          │
│   `GET /api/models/settings`                                                             │
│     └─► 返回 `{ providers (Custom), quickConnect (Preset 全集 + 连接状态), ... }`        │
│                                                                                          │
│   前端左侧 `nav.models-nav`:                                                             │
│     ├─ Tab 1: `Quick Connect` (默认)                                                     │
│     │    ├─ 顶部搜索框 `#provider-search` 实时过滤 A-Z 预设列表                          │
│     │    ├─ 条目 Tag: `Connected · API key` / `Connected · OAuth` / `$ENV` / `Not connected`│
│     │    └─ 右侧详情区:                                                                  │
│     │         ├─ 顶部横幅 `#preset-banner`: 说明快捷连接来源与认证方式                   │
│     │         ├─ OAuth 授权区 (若支持): `Sign in with <Method>` 按钮 + 设备码/URL 状态条 │
│     │         ├─ API Key & 附加表单字段 + Base URL + 内置/自定义 Models                  │
│     │         └─ 动作按钮: `Cancel` / `Reset to Preset` / `Disconnect`                   │
│     └─ Tab 2: `Custom`                                                                   │
│          ├─ 仅展示 `ai.json` 中的自定义 Provider + 底部 `+ New Provider`                 │
│          └─ 右侧详情区保持原有 `Cancel` / `Duplicate` / `Delete` 与完整自定义表单        │
└──────────────────────────────────────────────────────────────────────────────────────────┘
```

---

## 4. 详细技术设计与关键代码

### 4.1 预设目录与独立 `connect.json` 存储设计（`src/config/ai/presets.rs` & `src/config/ai/connect.rs`）

#### 关键点
1. **预设供应商静态目录（`src/config/ai/presets.rs`）**：
   - 数据源直接从 `opencode-dev` 的 `models-dev/snapshot.txt` 与 `packages/core/src/plugin/provider/*.ts` 提取，编译期嵌入并按 `name.to_ascii_lowercase()` **A-Z 首字母排序**；
   - 每个 `PresetProviderSpec` 包含：
     - `id: &'static str`（如 `"anthropic"`, `"deepseek"`, `"github-copilot"`, `"google"`, `"openai"`, `"opencode"`, `"opencode-go"`, `"openrouter"`, `"vercel"`, `"xai"` 等全量兼容供应商）；
     - `name: &'static str`；
     - `transport: ProviderTransport`（`openai` / `anthropic` / `google` / `openai-compatible`）；
     - `default_base_url: &'static str`（含模板占位符如 `https://${accountId}.gateway.ai.cloudflare.com/v1/${gatewayId}/openai` 时由表单字段替换）；
     - `note: &'static str`（设置页快捷连接说明文案，对齐 `opencode-dev` 的 `PROVIDER_NOTES` 与协议说明）；
     - `env_vars: &'static [&'static str]`（如 `["ANTHROPIC_API_KEY"]`）；
     - `methods: &'static [PresetAuthMethod]`（`OAuth` 排在前，`ApiKey` 排在后）；
     - `default_models: &'static [PresetModelSpec]`（从 `opencode-dev` 提取的非废弃 Chat 模型，含 `id`、`name`、`transport`、`limit`、`modalities` 及按 `variant.ts` 规则生成的思考 `variants`）。
2. **独立配置文件 `~/.lx-t[-dev]/config/connect.json`（`src/config/ai/connect.rs`）**：
   - 路径函数：`pub fn connect_config_path() -> Result<PathBuf, AiConfigError>` 返回 `user_data_root()?.join("config").join("connect.json")`；
   - 仅存储**已连接或用户自定义修改过**的预设供应商记录 `BTreeMap<String, ConnectedPresetRecord>`；
   - 权限与原子写复用 `save_ai` 同款的 `0600` 权限与同目录临时文件 `connect.json.<pid>.<seq>.tmp` + `platform::rename_over`。
3. **运行时合并（`load_runtime_ai_settings`）**：
   - 读取 `ai.json`（自定义供应商 + `defaultModel` / `titleSummary` / `titleLanguage` / `streamIdleTimeoutMs` / `webSearch`）；
   - 读取 `connect.json` 并结合 `PRESET_PROVIDERS` 与当前进程环境变量 `std::env::var`：
     - 若某预设供应商在 `connect.json` 中有有效凭证（非空 `apiKey` 或有效 `oauth.access_token`），或者未在 `connect.json` 中显式禁用且进程环境变量命中 `env_vars`，则物化为 `ProviderConfig` 并注入运行时 `AiSettings.providers`（若 `enabled == true` 则同步加入 `enabled_providers`）；
   - `defaultModel` 与 `titleSummary` 的合法性校验针对**合并后的完整 `providers` 集合**执行，因此默认模型可以自由指向快捷连接供应商或自定义供应商。

#### 关键代码

```rust
// src/config/ai/presets.rs

use super::{ModelConfig, ModelLimit, ModelTransport, ProviderTransport};

/// 预设认证表单字段定义（对齐 opencode-dev Form.Field）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PresetFormField {
    pub key: &'static str,
    pub label: &'static str,
    pub placeholder: &'static str,
    pub required: bool,
    /// 可选枚举项 `(label, value, description)`；为空表示自由文本输入。
    pub options: &'static [(&'static str, &'static str, &'static str)],
    /// 条件显隐：`Some((dep_key, expected_value))` 时仅当前置字段等于该值才询问。
    pub when_eq: Option<(&'static str, &'static str)>,
}

/// OAuth 授权模式（对齐 opencode-dev）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OAuthFlowKind {
    /// OAuth 2.0 Device Authorization Grant（展示 URL + 设备码，后台轮询 token）。
    DeviceCode {
        device_code_url: &'static str,
        token_url: &'static str,
        client_id: &'static str,
        scope: &'static str,
    },
    /// 本地回环浏览器 PKCE 授权（启动 127.0.0.1 临时回调端口，打开浏览器并自动接收 code）。
    BrowserPkce {
        authorize_url: &'static str,
        token_url: &'static str,
        client_id: &'static str,
        scope: &'static str,
        fixed_port: Option<u16>,
    },
    /// 浏览器授权码手动粘贴模式（展示 URL，用户在输入框粘贴 code）。
    AuthorizationCode {
        authorize_url: &'static str,
        token_url: &'static str,
        client_id: &'static str,
        scope: &'static str,
    },
}

/// 预设供应商支持的单种交互式连接方式（OAuth 在前，ApiKey 在后）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PresetAuthMethod {
    OAuth {
        id: &'static str,
        label: &'static str,
        flow: OAuthFlowKind,
        form: &'static [PresetFormField],
        /// OAuth 成功后若需覆盖默认 baseURL（如 GitHub Copilot / OpenCode Console）。
        base_url_override: Option<&'static str>,
    },
    ApiKey {
        label: &'static str,
        form: &'static [PresetFormField],
    },
}

/// 预设供应商静态规格（按名称 A-Z 排序）。
#[derive(Debug, Clone)]
pub struct PresetProviderSpec {
    pub id: &'static str,
    pub name: &'static str,
    pub transport: ProviderTransport,
    pub default_base_url: &'static str,
    pub note: &'static str,
    pub env_vars: &'static [&'static str],
    pub methods: &'static [PresetAuthMethod],
}

/// 返回按名称首字母（A-Z）排序的全部预设供应商规格。
pub fn all_presets() -> &'static [PresetProviderSpec] {
    &PRESET_PROVIDERS
}

/// 按 ID 查找预设供应商规格。
pub fn find_preset(id: &str) -> Option<&'static PresetProviderSpec> {
    PRESET_PROVIDERS.iter().find(|spec| spec.id == id)
}

/// 获取某预设供应商的内置默认模型表（从编译期嵌入的 opencode 快照按需物化）。
pub fn default_models_for_preset(id: &str) -> std::collections::BTreeMap<String, ModelConfig> {
    // 从内置精简 JSON 快照按 provider id 解析出 ModelConfig 集合（含 limit/modalities/variants）
    ...
}
```

```rust
// src/config/ai/connect.rs

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use serde::{Deserialize, Serialize};
use super::{AiConfigError, ModelConfig, ProviderConfig, ProviderOptions};

/// OAuth 落盘凭证（保存在 0600 权限的 connect.json 中）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OAuthCredential {
    pub method_id: String,
    pub access_token: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub refresh_token: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires_at_ms: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub account_label: Option<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub metadata: BTreeMap<String, String>,
}

/// 单个快捷连接供应商在 `connect.json` 中的持久化记录。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConnectedPresetRecord {
    pub id: String,
    #[serde(default = "default_enabled")]
    pub enabled: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api_key: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub oauth: Option<OAuthCredential>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub form_answers: BTreeMap<String, String>,
    /// 用户覆盖的 Base URL；为 None 时取预设 `default_base_url`（或表单展开值）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base_url: Option<String>,
    /// 用户自定义增删改后的模型表；为 None 时直接使用预设内置模型表。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub models: Option<BTreeMap<String, ModelConfig>>,
}

fn default_enabled() -> bool {
    true
}

/// `~/.lx-t[-dev]/config/connect.json` 顶层结构。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConnectSettings {
    #[serde(default)]
    pub providers: BTreeMap<String, ConnectedPresetRecord>,
}

/// 快捷连接状态来源（用于 `/connect` 面板与设置页标签展示）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConnectionSource {
    ApiKey,
    OAuth { label: Option<String> },
    Env { var_name: String },
}

impl ConnectionSource {
    /// 面板/设置页展示的短标签（如 `"API key"`、`"OAuth"`、`"$OPENAI_API_KEY"`）。
    pub fn summary_label(&self) -> String {
        match self {
            Self::ApiKey => "API key".to_string(),
            Self::OAuth { label: Some(label) } if !label.is_empty() => format!("OAuth · {label}"),
            Self::OAuth { .. } => "OAuth".to_string(),
            Self::Env { var_name } => format!("${var_name}"),
        }
    }
}

/// `~/.lx-t[-dev]/config/connect.json` 路径。
pub fn connect_config_path() -> Result<PathBuf, AiConfigError> {
    Ok(super::user_data_root()?.join("config").join("connect.json"))
}
```

---

### 4.2 OAuth 授权流与凭证刷新（`src/config/ai/oauth/{mod,browser,device}.rs` & `src/event/chat/connect.rs`）

#### 关键点
1. **对齐 `opencode-dev` 的三类 OAuth 流程**：
   - **Device Code 流（`OAuthFlowKind::DeviceCode`）**：
     - 覆盖 `opencode` / `opencode-go`（`https://opencode.ai/auth/device/code`）、`github-copilot`（`https://github.com/login/device/code` 或 Enterprise 域名）、`xai`（`https://auth.x.ai/oauth2/device/code`）、`openai` headless device code；
     - 发起 POST 获取 `device_code`、`user_code`、`verification_uri` / `verification_uri_complete`、`interval`；
     - 向 UI 返回 `OAuthPrompt { attempt_id, url, instructions: "Enter code: XXXX-XXXX", user_code: Some(code), mode: Auto }`；
     - 后台 task 按 `interval + 3s` 轮询 token 端点，收到 `access_token` 后（如 GitHub Copilot 额外校验 `/copilot_internal/user` 获取动态 `apiEndpoint`）写入 `connect.json` 并派发成功事件。
   - **Browser PKCE 流（`OAuthFlowKind::BrowserPkce`）**：
     - 覆盖 `openai`（`https://auth.openai.com`）、`poe`（`https://poe.com/oauth/authorize`）、`gitlab`（`https://gitlab.com/oauth/authorize` 或自托管实例）、`digitalocean`；
     - 生成 PKCE `verifier` + `S256 challenge` 与随机 `state`，在 `127.0.0.1` 绑定回调端口（指定端口或 `:0` 随机端口），等待浏览器重定向 `/auth/callback?code=...&state=...`，返回友好 HTML 成功页后用 `code` 换取 `access_token` + `refresh_token`。
   - **取消与超时清理**：
     - 每个进行中的 OAuth 尝试绑定 `CancellationToken` 与 10 分钟过期时间；用户在 `/connect` 面板按 `Esc` 或在设置页关闭弹窗时立即 `cancel()`，释放本地回调端口与轮询任务。
2. **Token 自动刷新**：
   - 在聊天请求发起前或加载运行时快照时，若活跃预设连接为 `OAuth` 且 `expires_at_ms` 距当前时间不足 5 分钟（且存在 `refresh_token`），自动调用 `refresh_oauth_credential` 刷新并原子写回 `connect.json`。

---

### 4.3 TUI `/connect` 命令与四步面板状态机（`src/app/state/lx/connect.rs` & `src/app/update/chat/connect.rs` & `src/ui/overlay/connect.rs`）

#### 关键点
1. **斜杠命令注册（`src/app/state/lx/slash.rs` & `src/settings/commands.rs`）**：
   - `SlashCommand` 新增 `Connect`：
     - `name() -> "connect"`
     - `aliases() -> &[]`
     - `text() -> "/connect"`
     - `description() -> crate::ui::text::LX_CONNECT_COMMAND_DESC`（`"Connect a provider with an API key or browser sign-in"`）
   - `RESERVED_NAMES`（`src/settings/commands.rs:222`）加入 `"connect"`。
2. **`ConnectPanel` 状态机（`src/app/state/lx/connect.rs`）**：
   - 挂在居中模态弹窗 `Overlay::Connect(Box<ConnectPanel>)` 下，由 `src/ui/overlay/connect.rs` 渲染、`src/event/overlay.rs` 路由键盘与鼠标；
   - 包含 5 个阶段枚举 `ConnectStage`：
     1. `Providers`：展示按 A-Z 排序的全部预设供应商 `Vec<ConnectProviderItem>`。每项包含 `id`、`name`、`connected: Option<ConnectionSource>`。弹窗输入框文本作为实时搜索词（不区分大小写匹配 `name` 与 `id`）。已连接项在左侧或标签展示 `✓ Connected (API key / OAuth / $ENV)`。
     2. `Manage { provider_id, provider_name, source }`：当用户在 `Providers` 阶段对**已存有凭证（API Key 或 OAuth）**的供应商按 `Enter` 时进入，提供 3 个动作项：
        - `Select model`（直接打开该供应商的 `/model` 选择器）
        - `Reconnect / Replace credentials`（重新走认证流程）
        - `Disconnect`（清除 `connect.json` 中的凭证并弹 Toast `Disconnected <Name>`）
     3. `Methods { provider_id, provider_name, methods, window }`：当供应商具有 `> 1` 种交互式认证方法时展示（OAuth 在前、API Key 在后）；若只有 1 种方法则直接跳过进入阶段 4。
     4. `InputPrompt { provider_id, provider_name, method_index, step, answers, error }`：
        - 当需要填写前置 `form` 字段（如选择 `GitHub.com` vs `GitHub Enterprise`、输入 `Enterprise URL` 或 `Account ID`）或最终输入 `API key` 时使用；
        - 若当前字段有 `options`（如 `GitHub.com` / `GitHub Enterprise`），在面板列表渲染可选项供 `↑↓` + `Enter` 选择；
        - 若当前字段为自由文本或 `API key`，面板展示 1 行提示说明（如 `Enter API key for Anthropic (paste and press Enter)`，若有校验错误则显示错误提示），用户直接在弹窗输入框输入/粘贴文本并按 `Enter` 提交！
     5. `OAuthWaiting { attempt_id, provider_id, provider_name, url, user_code, instructions, error }`：
        - 面板展示 3 条操作/状态行：
          - 行 1：`Open in browser`（右侧 preview 显示截断后的 `url`，按 `Enter` 或 `o` 调用 `platform::open_browser` 打开浏览器）；
          - 行 2：`Copy code / link`（若有 `user_code` 显示 `Code: XXXX-XXXX`，按 `c` 或选中回车复制到剪贴板并弹 Toast）；
          - 行 3：状态行 `Waiting for authorization…`（或失败原因）。
   - **级间 `Esc` 回退与列表焦点记忆**：
     - 在 `OAuthWaiting` 或首步 `InputPrompt` 按 `Esc`：触发 `Effect::CancelPresetOAuth { attempt_id }`（如在 OAuth 等待中）；若该供应商有多种认证方法则退回二级 `Methods` 并恢复原方法高亮，单方法则退回一级 `Providers` 并保持之前选中的供应商高亮与可见窗口；
     - 在 `Methods` / `Manage` 按 `Esc`：清空弹窗输入框，退回一级 `Providers` 阶段并恢复选中之前的供应商；
     - 在 `Providers` 阶段按 `Esc`：清空弹窗输入框并关闭 `/connect` 弹窗（重开 `/connect` 时根据 `state.last_connect_provider` 自动恢复上次选中的供应商）。
3. **连接成功后无缝跳转 `/model` 与模态 Toast 分派规则**：
   - 当 API Key 保存成功或 OAuth 授权完成时，触发 `on_preset_connected(state, pane, provider_id, ...)`：
     - 重新加载合并后的 `state.models.settings` 与 `state.connect`，后台静默触发 `Effect::RefreshModelsCatalog { pane: None, provider_id: Some(...) }`（错误 API Key 仅在实际发消息时报错）；
     - 若当前 `state.overlay` 仍为该 `pane` 的 `Overlay::Connect`，则关闭 `Overlay::Connect`（`state.overlay = None`），构造仅过滤该 `provider_id` 的 `ModelPicker` 并切入 `SlashPanel::Model`；
     - **模态 Toast 路由规则**：点击后马上关闭全部模态弹窗（`state.overlay == None`，如连接成功返回 `lx` 窗格）时，Toast 经 `show_toast_at` 落回打开弹窗的源容器（`pane.lx.toast`）；若关闭二级视图后仍停留在第一级模态弹窗（如 `Manage` 点击 `Disconnect` 退回一级 `Providers` 列表）或弹窗未关闭（如打开浏览器、复制授权码、手动刷新模型），Toast 一律显示在全局位置（`AppState.toast` 且 `anchor: None`，屏幕顶部正中央）。

#### 关键代码

```rust
// src/app/state/lx/connect.rs

use std::collections::BTreeMap;
use crate::app::state::lx::slash::PanelWindow;
use crate::config::ai::connect::ConnectionSource;
use crate::config::ai::presets::{PresetAuthMethod, PresetFormField};

/// `/connect` 面板中的单个预设供应商条目（已按 A-Z 排序）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConnectProviderEntry {
    pub id: String,
    pub name: String,
    pub note: String,
    pub source: Option<ConnectionSource>,
    pub methods: &'static [PresetAuthMethod],
}

/// 已连接供应商的管理动作。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConnectManageAction {
    SelectModel,
    Reconnect,
    Disconnect,
}

/// 表单/API Key 输入子阶段。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConnectPromptStep {
    /// 正在填写第 `field_index` 个前置表单字段。
    FormField {
        field_index: usize,
        field: PresetFormField,
        option_window: PanelWindow,
    },
    /// 正在通过底部输入框输入 API Key。
    ApiKey,
}

/// `/connect` 面板的所处阶段。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConnectStage {
    /// 阶段 1：A-Z 预设供应商列表（支持底部输入框实时过滤）。
    Providers,
    /// 阶段 1b：点选已连接供应商后的管理菜单。
    Manage {
        provider_id: String,
        provider_name: String,
        source: ConnectionSource,
        actions: Vec<ConnectManageAction>,
        window: PanelWindow,
    },
    /// 阶段 2：多种认证方式选择（OAuth 在前，API Key 在后）。
    Methods {
        provider_id: String,
        provider_name: String,
        methods: &'static [PresetAuthMethod],
        window: PanelWindow,
    },
    /// 阶段 3a：前置表单字段选择/输入或 API Key 输入。
    InputPrompt {
        provider_id: String,
        provider_name: String,
        method_index: usize,
        step: ConnectPromptStep,
        answers: BTreeMap<String, String>,
        error: Option<String>,
    },
    /// 阶段 3b：等待 OAuth 浏览器/设备码授权完成。
    OAuthWaiting {
        attempt_id: String,
        provider_id: String,
        provider_name: String,
        url: String,
        user_code: Option<String>,
        instructions: String,
        window: PanelWindow,
    },
}

/// `/connect` 命令面板完整状态。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConnectPanel {
    pub entries: Vec<ConnectProviderEntry>,
    pub filtered: Vec<usize>,
    pub filter: String,
    pub window: PanelWindow,
    pub stage: ConnectStage,
}
```

---

### 4.4 设置页后端 HTTP API 设计（`src/settings/service/models/mod.rs` & `src/settings/service/models/connect.rs`）

#### 关键点
1. **扩展 `GET /api/models/settings` 响应结构**：
   - 保持原有 `providers` 字段只返回 **`ai.json` 中的自定义供应商（Custom Providers）**，确保现有自定义供应商保存/编辑逻辑 100% 兼容且互不污染；
   - 新增 `quickConnect` 数组字段，返回按 A-Z 排序的全部预设供应商视图 `QuickConnectProviderView`：
     - `id`、`name`、`type`、`note`、`defaultBaseUrl`、`baseUrl`、`enabled`；
     - `connected: bool`、`connectionType: "apiKey" | "oauth" | "env" | null`、`connectionLabel: string`（如 `"Connected · API key"`、`"Connected · OAuth"`、`"Connected · $OPENAI_API_KEY"`、`"Not connected"`）；
     - `apiKey`（已存明文 Key，仅回环 + token 鉴权）、`formAnswers`、`methods`（供前端渲染认证方式说明、表单项与 OAuth 按钮）；
     - `models`（当前生效模型列表：若用户在 `connect.json` 自定义过则取自定义，否则取预设默认模型列表）与 `defaultModels`（供前端 `Reset to Preset` 一键还原）。
   - `revision_of` 同时纳入 `connect.json` 内容计算 FNV-1a 64 哈希，防止并发修改丢失。
2. **新增快捷连接专属 API 路由（`src/settings/service/models/connect.rs`）**：
   - `PUT /api/models/connect`：保存单个或全量快捷连接供应商配置到 `connect.json`（支持设置/更新 `apiKey`、`formAnswers`、`baseUrl`、`enabled`、`models`），保存成功后广播 `SettingsEvent::ModelsChanged` 通知 TUI 立即热更新；
   - `POST /api/models/connect/disconnect`：入参 `{"providerId": "..."}`，清除该预设的凭证；若命中进程环境变量 `$ENV` 则保留 `enabled: false` 记录阻断自动探测，否则从 `connect.json` 移除；若 `ai.json` 的 `defaultModel` / `titleSummary` 恰好指向该供应商，则安全清空悬空选择（失败返回 500 且不静默吞错），广播 `SettingsEvent::ModelsChanged`；
   - `POST /api/models/connect/oauth/start`：入参 `{"providerId": "...", "methodId": "...", "answers": {...}}`，启动 OAuth Device Code 或 Browser PKCE 流程，返回 `{"attemptId", "url", "userCode", "instructions", "mode"}`；
   - `GET /api/models/connect/oauth/status?attemptId=...`：前端每 800ms 轮询 OAuth 状态，返回 `{"status": "pending" | "complete" | "failed", "error"?: "..."}`；完成时自动写入 `connect.json` 并广播 `SettingsEvent::ModelsChanged`；
   - `POST /api/models/connect/oauth/complete`：用于 `mode == "code"` 时手动提交授权码；
   - `DELETE /api/models/connect/oauth?attemptId=...`：取消进行中的 OAuth 授权。
   - `POST /api/models/fetch` 增强：当传入 `providerId` 属于预设供应商时，自动从 `connect.json` + 预设默认配置读取 `baseUrl` / `apiKey` / `oauth.access_token` 拉取远端 `/v1/models`。

#### 关键代码

```rust
// src/settings/service/models/connect.rs

use super::super::{Request, Response, ServiceState, SettingsEvent};
use crate::config::ai::connect::{
    ConnectSettings, ConnectedPresetRecord, load_connect, save_connect,
};
use crate::config::ai::presets::{PresetAuthMethod, all_presets, default_models_for_preset, find_preset};

/// 构造 `GET /api/models/settings` 中的 `quickConnect` JSON 数组。
pub fn quick_connect_json(connect: &ConnectSettings) -> String {
    let mut items = Vec::with_capacity(all_presets().len());
    for spec in all_presets() {
        let record = connect.providers.get(spec.id);
        let source = super::resolve_preset_connection_source(spec, record);
        let connected = source.is_some();
        let enabled = record.map(|r| r.enabled).unwrap_or(connected);
        let base_url = record
            .and_then(|r| r.base_url.as_deref())
            .unwrap_or(spec.default_base_url);
        let api_key = record
            .and_then(|r| r.api_key.as_deref())
            .unwrap_or("");
        let models = record
            .and_then(|r| r.models.clone())
            .unwrap_or_else(|| default_models_for_preset(spec.id));
        // 序列化单个 QuickConnectProviderView JSON
        items.push(encode_quick_connect_item(
            spec,
            connected,
            source.as_ref(),
            enabled,
            base_url,
            api_key,
            record,
            &models,
        ));
    }
    format!("[{}]", items.join(","))
}
```

---

### 4.5 浏览器设置页双 Tab 与快捷连接 UI 设计（`assets/settings.html`、`assets/settings.css`、`assets/settings_models.js`、`assets/settings_connect.js`）

#### 关键点
1. **左侧列表双 Tab 切换与搜索过滤（`assets/settings.html` & `assets/settings.css`）**：
   - 在 `nav.models-nav` 顶部（`#provider-list` 上方）新增：
     - `.provider-tabs`：两个 Tab 按钮 `#prov-tab-quick`（`Quick Connect`，默认激活）和 `#prov-tab-custom`（`Custom`），带数量角标（如 `Quick Connect` 显示已连接数 `2/217`，`Custom` 显示自定义数量）；
     - `#provider-search-wrap`：包含搜索输入框 `#provider-search`（`placeholder="Filter providers…"`），在 `Quick Connect` Tab 下常显（`Custom` Tab 下同样可用或按需过滤）；
   - 当处于 `Quick Connect` Tab 时：
     - `#provider-list` 仅渲染预设供应商列表（**不与自定义供应商混排**）；
     - 已连接的预设供应商置顶（内部按 A-Z 排序），未连接的按 A-Z 排列在下方；
     - 每个列表项右侧 `.tag` 明确显示连接状态：
       - 已连接：`Connected · API key` / `Connected · OAuth` / `Connected · $ENV`（使用强调色/成功色修饰）；
       - 已连接但被手动关闭 `Enabled`：`Connected · off`；
       - 未连接：`Not connected`；
     - 底部 `#new-provider-row`（`+ New Provider`）在 `Quick Connect` Tab 下隐藏（`hidden = true`），仅在切换到 `Custom` Tab 时显示。
2. **右侧编辑区快捷连接专属说明与控件（`assets/settings.html` & `assets/settings_connect.js`）**：
   - **快捷连接说明横幅 `#preset-banner`**（位于 `#provider-editor` 顶部，仅在选中 `Quick Connect` 项时显示）：
     - 左侧显示 `Quick Connect Preset` 徽章 + 预设简介 `spec.note` + 支持的连接方式说明（如 `Supports: OAuth browser sign-in · API key · Env ($OPENAI_API_KEY)`）；
     - 右侧显示当前连接状态徽章（如 `● Connected via API key` 或 `○ Not connected`）。
   - **OAuth 快捷授权卡片区 `#preset-oauth-section`**（仅当该预设包含 OAuth 方法时显示）：
     - 为每个 OAuth 方法渲染操作按钮（如 `Sign in with ChatGPT` / `Login with GitHub Copilot` / `OpenCode Console account`）；
     - 点击后在内联状态区展示授权链接、`Copy Code / URL`、`Open Browser` 按钮与轮询进度 `Waiting for browser confirmation…`，授权成功后自动刷新列表与状态为 `Connected · OAuth`。
   - **预设附加表单字段区 `#preset-form-fields`**（仅当预设方法含 `form` 字段如 `enterpriseUrl`、`accountId`、`gatewayId` 时动态生成 DOM 输入框）。
   - **右上角动作栏按钮按 Tab 动态切换**：
     - `Custom` Tab：保持 `Cancel` / `Duplicate` / `Delete`；
     - `Quick Connect` Tab：切换为 `Cancel` / `Reset to Preset`（恢复默认 `Base URL` 与内置 `Models`） / `Disconnect`（仅在 `connected` 时可点，点击调用 `/api/models/connect/disconnect` 清空凭证并断开）。
   - **Model Tab（默认模型/标题模型选择器）联动**：
     - `renderDefaultModel()` 同时汇总**已连接且已启用**的 `Quick Connect` 供应商模型与**已启用**的 `Custom` 供应商模型，下拉列表中为快捷连接供应商附带分组标识，确保在设置页也能直接将快捷连接的模型设为 `Default model` 或 `Title summary model`。

#### 关键代码

```html
<!-- assets/settings.html (Provider 视图左栏与右栏头部结构) -->
<div class="models-grid">
  <nav class="models-nav" aria-label="Providers">
    <div class="provider-tabs" role="tablist" aria-label="Provider source">
      <button class="provider-tab active" id="prov-tab-quick" type="button" role="tab" aria-selected="true" onclick="switchProviderTab('quick')">
        Quick Connect <span class="provider-tab-count" id="prov-tab-quick-count">0</span>
      </button>
      <button class="provider-tab" id="prov-tab-custom" type="button" role="tab" aria-selected="false" onclick="switchProviderTab('custom')">
        Custom <span class="provider-tab-count" id="prov-tab-custom-count">0</span>
      </button>
    </div>
    <div class="provider-search-wrap" id="provider-search-wrap">
      <input class="input input-sm" id="provider-search" type="search" placeholder="Filter providers…" oninput="onProviderSearchInput(this.value)">
    </div>
    <div class="provider-list" id="provider-list"></div>
    <button class="nav-item nav-new hidden" id="new-provider-row" type="button" onclick="newProvider()">+ New Provider</button>
  </nav>
  <div class="models-resizer" id="models-resizer" role="separator" aria-orientation="vertical" aria-label="Resize provider list"></div>
  <div>
    <div class="card empty" id="provider-empty">
      <p>Select a provider on the left to view or connect.</p>
    </div>
    <div id="provider-editor" class="hidden">
      <div class="preset-banner hidden" id="preset-banner">
        <div class="preset-banner-main">
          <div class="preset-banner-title-row">
            <span class="preset-badge">Quick Connect</span>
            <span class="preset-status-badge" id="preset-status-badge">Not connected</span>
          </div>
          <p class="preset-banner-desc" id="preset-banner-desc"></p>
          <p class="preset-banner-methods" id="preset-banner-methods"></p>
        </div>
      </div>
      <div class="editor-actions" id="custom-editor-actions">
        <button class="btn" id="cancel-provider-btn" type="button" onclick="cancelProvider()">Cancel</button>
        <button class="btn" id="duplicate-provider-btn" type="button" onclick="duplicateProvider()">Duplicate</button>
        <button class="btn btn-danger" id="delete-provider-btn" type="button" onclick="deleteProvider()">Delete</button>
      </div>
      <div class="editor-actions hidden" id="preset-editor-actions">
        <button class="btn" id="cancel-preset-btn" type="button" onclick="cancelPresetProvider()">Cancel</button>
        <button class="btn" id="reset-preset-btn" type="button" onclick="resetPresetProvider()">Reset to Preset</button>
        <button class="btn btn-danger" id="disconnect-preset-btn" type="button" onclick="disconnectPresetProvider()">Disconnect</button>
      </div>
      <!-- OAuth 快捷登录卡片（仅支持 OAuth 的预设显示） -->
      <div class="card hidden" id="preset-oauth-card">
        <div class="card-title">Quick Sign-in (OAuth / URL)</div>
        <div id="preset-oauth-methods" class="preset-oauth-methods"></div>
        <div id="preset-oauth-pending" class="preset-oauth-pending hidden"></div>
      </div>
      ...
```

---

### 4.6 四协议思考、签名、Token 统计与工具 Schema 对齐（对齐 `opencode-dev`）

#### 协议矩阵

| 协议 | 文件 | 思考参数 | 签名 / 历史回放 | Token 统计 |
| --- | --- | --- | --- | --- |
| **Anthropic Messages** | `src/chat/transport/anthropic/mod.rs` | Claude 4.6 / 4.7 / 5.x（含 Fable / Mythos）走 `thinking: {"type":"adaptive","display":"summarized"}` + `output_config.effort`；Claude 3.7/4.0/4.1/4.5 与 Kimi / MiniMax / GLM 等兼容端点走 `thinking: {"type":"enabled","budget_tokens":...}`；`thinkingBudget` / `reasoningEffort` / `effort` 均映射到同一套预算。 | thinking 块通过 `content_block_index` 绑定 `signature` / `signature_delta`；请求带 thinking 或命中 `anthropic.com` 时附加 `interleaved-thinking-2025-05-14` beta 头；若历史存在不含 thinking 块的 assistant `tool_use`（工具续轮），自动省略手动 `thinking`，避免 HTTP 400。 | 读取 `message_start.message.usage` 与 `message_delta.usage`，合并 input/output/cache_read。 |
| **OpenAI Responses** | `src/chat/transport/responses/mod.rs` | `reasoning.effort` 来自 `reasoningEffort`；未指定 summary 时显式补 `summary: "auto"`，确保流中下发思考摘要；有 reasoning 时附带 `include: ["reasoning.encrypted_content"]`。 | `response.output_item.done` 同时从顶层 `item_id` 与 `item.id` 解析 ordinal，修复无 `output_index` 帧的 id 映射；function call 回放保持 `call_id` 与输出项一致。 | `response.completed.usage` 映射 input/output；reasoning token 计入 output。 |
| **OpenAI Chat Completions / Compatible** | `src/chat/transport/openai/mod.rs` | `reasoningEffort` 直出；兼容 `enable_thinking` + `thinking_budget`（Qwen/Alibaba）、`thinking` 对象（DeepSeek V3.2 / Kimi K2.5 / GLM-5）与 OpenRouter `reasoning` 对象。 | DeepSeek / Kimi / Moonshot / Zhipu 开启思考并调用工具时，历史 assistant 消息必须携带 `reasoning_content`；编码器按需补齐该字段（含空串占位），避免工具续轮 400。流式解析兼容 `reasoning_content` / `reasoning` / `reasoning_text` 与 OpenRouter `reasoning_details` 数组。 | 解析 `usage.prompt_tokens/completion_tokens`，并兼容 `prompt_tokens_details.cached_tokens` 与 reasoning token 明细。 |
| **Google Gemini** | `src/chat/transport/google/mod.rs` | 思考参数默认 `includeThoughts: true`（模型未声明时也请求思考块），映射 `thinkingLevel` / `thinkingBudget`。 | 历史 `functionCall` 回放附带 `thoughtSignature`，保证 Gemini 3 等要求签名的模型可续轮；内置工具 schema 先经 `src/mcp/schema.rs` 裁剪。 | output = `candidatesTokenCount` + `thoughtsTokenCount`；cache_read 优先读顶层 `cachedContentTokenCount`，回退 `promptTokensDetails.cachedTokens`。 |

#### 工具 Schema 与模型目录

1. **工具 Schema 对齐（`src/mcp/schema.rs`、`src/chat/transport/tools/mod.rs`）**：
   - `trim_for_gemini` 将 `anyOf` / `oneOf` 中的可空联合扁平化为主类型并置 `nullable: true`，为 `type: "object"` 补齐 `properties`，并过滤 `required` 中不存在的字段；
   - 内置工具（shell / read / write / edit / search 等）在发送前统一裁剪 Gemini 不支持的 JSON Schema 关键字，避免 `400 INVALID_ARGUMENT`。
2. **思考等级与目录同步（`src/config/ai/verify.rs`、`src/config/ai/catalog_refresh.rs`、`src/config/ai/presets.rs`）**：
   - `protocol_reasoning_variants(provider_id, model_id, transport)` 按供应商 + 协议生成 `low` / `medium` / `high`（必要时 `none` / `xhigh` / `max`）参数，供云端目录与 `/models` 发现共用；
   - `catalog_refresh.rs` 合并 `models.dev` 快照时保留模型既有 `transport` 与默认 `variant`，仅对 `openai` 官方新增且未标注 transport 的模型默认 `OpenaiResponses`；
   - `presets.rs` 的预设模型表与运行时 reasoning variants 使用同一映射，避免设置页展示与实际请求不一致。

#### 验证覆盖

- `src/chat/transport/anthropic/tests.rs`、`src/chat/transport/responses/tests.rs`、`src/chat/transport/openai/tests.rs`、`src/chat/transport/google/tests.rs` 覆盖 adaptive / manual thinking、unsigned tool history 省略、`reasoning_content` 回放、`includeThoughts`、`thoughtSignature` 与 token 统计；
- `src/mcp/schema/tests.rs` 覆盖 Gemini nullable union 扁平化与 required 过滤；
- `src/config/ai/verify_tests.rs`、`src/config/ai/catalog_refresh_tests.rs`、`src/config/ai/presets_tests.rs` 覆盖 reasoning variants 与目录合并保留规则。

---

## 5. 涉及文件与行数现状（生产代码 ≤ 1000 行、测试 ≤ 1500 行）

| 文件路径 | 操作 | 现状行数 | 说明（生产代码 ≤ 1000 行；测试文件 ≤ 1500 行） |
| --- | --- | --- | --- |
| `src/config/ai/mod.rs` | 修改 | 799 | 导出 `presets`/`connect`/`oauth` 子模块，提供运行时合并入口与 `validate_base_url` |
| `src/config/ai/presets.rs` | 新建 | 734 | A-Z 预设供应商目录、协议映射、默认模型与 `resolve_preset_base_url` |
| `src/config/ai/presets_catalog.json` | 新建 | 数据文件 | 从 `opencode-dev` 提取的预设供应商与模型快照（`include_str!` 编译期嵌入） |
| `src/config/ai/connect.rs` | 新建 | 591 | `connect.json` 数据结构、`0600` 原子读写、环境变量探测与 `ProviderConfig` 物化 |
| `src/config/ai/oauth/mod.rs` | 新建 | 426 | OAuth 公共类型、Token 刷新（`refresh_oauth_credential`、`ensure_connect_oauth_fresh`） |
| `src/config/ai/oauth/browser.rs` | 新建 | 671 | Browser PKCE / 授权码本地回环回调与隐式流处理 |
| `src/config/ai/oauth/device.rs` | 新建 | 453 | Device Code 授权、轮询与总超时 |
| `src/config/ai/verify.rs` | 新建 | 506 | 四协议远端模型发现、`protocol_reasoning_variants` 与连接校验 |
| `src/config/ai/catalog_refresh.rs` | 新建 | 516 | `models.dev` 云端目录同步、ETag 缓存与已连接模型刷新（写前重读 `connect.json`） |
| `src/app/state/lx/connect.rs` | 新建 | 495 | `ConnectPanel`、`ConnectStage`、`ConnectProviderEntry` 状态机、供应商焦点记忆与过滤/导航纯函数 |
| `src/app/update/chat/connect/mod.rs` | 新建 | 534 | `/connect` 弹窗按键/鼠标导航、阶段推进、级间 `Esc` 回退与表单/API Key 提交 |
| `src/app/update/chat/connect/events.rs` | 新建 | 550 | `/connect` 异步回投（连接成功/断开/失败、OAuth 启动/失败、模型刷新）与 `show_toast_at` 分派 |
| `src/event/chat/connect.rs` | 新建 | 428 | 异步执行 `StartPresetOAuth`、`SavePresetConnection`、`DisconnectPreset`、聊天前 OAuth 刷新与回投事件 |
| `src/event/overlay.rs` | 修改 | 619 | `Overlay::Connect` 键盘/鼠标路由（点击仅移动焦点，再次点击已选中行或按 `Enter` 才确认） |
| `src/event/inventory.rs` | 修改 | 490 | `ModelsChanged` 同步重载 `connect.json`、环境变量快照与已打开弹窗 |
| `src/ui/overlay/connect.rs` | 新建 | 569 | 渲染 `Overlay::Connect(Box<ConnectPanel>)` 各阶段列表、输入行、状态行与快捷键提示 |
| `src/ui/text/connect.rs` | 新建 | 101 | `/connect` 英文文案常量与 `lx_connect_*` 文案函数（从 `text.rs` 拆出） |
| `src/ui/text.rs` | 修改 | 1470 | 承载其余既有文案，`/connect` 文案已迁出到 `text/connect.rs` |
| `src/settings/commands.rs` | 修改 | — | `RESERVED_NAMES` 新增 `"connect"` |
| `src/settings/service/page.rs` | 修改 | — | 静态资源白名单新增 `"/settings_connect.js"` |
| `src/settings/service/models/mod.rs` | 修改 | 625 | `GET /api/models/settings` 附带 `quickConnect`、支持未保存预设 `Fetch models` 与 `/api/models/connect/*` 路由 |
| `src/settings/service/models/connect.rs` | 修改 | 645 | `PUT /api/models/connect`（首次填 Key 自动启用）、`disconnect`、OAuth start/status/complete/cancel |
| `src/settings/service/models/tests.rs` | 修改 | 1029 | Models API 测试（含 revision 兼容、无 `innerHTML` 资源检查） |
| `src/settings/service/models/tests/connect.rs` | 修改 | 615 | Quick Connect / 首次连接自动启用 / 未保存预设 Fetch models / OAuth 场景测试 |
| `src/settings/service/tests.rs` | 修改 | 1493 | 设置页资源、路由与视图回归测试（含 `settings.js` 无 `innerHTML` 检查） |
| `assets/settings.html` | 修改 | — | Provider 双 Tab、搜索框、快捷连接横幅、OAuth 卡片与 `<script src="/settings_connect.js">` |
| `assets/settings.css` | 修改 | — | `.provider-tabs`、`.preset-banner`、`.preset-oauth-*` 等样式 |
| `assets/settings_models.js` | 修改 | 935 | Quick Connect / Custom 协同；`saveModels` 先保存预设再提交 `ai.json`，`fetchModels` 传递 `formAnswers` |
| `assets/settings_connect.js` | 新建 | 632 | Quick Connect 列表、搜索过滤、局部状态刷新 `refreshQuickConnectState`、OAuth 轮询、保存/断开/重置 |
| `assets/settings.js` | 修改 | 948 | 全局 toast/select/操作条；`createActionBar` 暴露 `saveFn` 去重，`replaceChildren` 替代 `innerHTML` |
| `assets/oauth_callback.html` | 新建 | 36 | OAuth 浏览器回调结果页静态 HTML 模板（`include_str!` 加载） |
| `assets/oauth_implicit.html` | 新建 | 44 | OAuth 隐式授权流（fragment 转发）静态 HTML 模板（`include_str!` 加载） |

> 说明：`src/ui/text.rs` 为既有历史文件，本次已将全部 `/connect` 文案迁入 `src/ui/text/connect.rs`；该文件剩余部分不在本特性范围内，后续按职责继续拆分。

---

## 6. 测试矩阵与风险回滚

### 6.1 单元测试与集成测试矩阵
1. **配置与预设目录测试（`src/config/ai/tests.rs`）**：
   - `all_presets()` 非空且严格按 `name` 首字母 A-Z 升序排列（不区分大小写），无重复 `id`；
   - 每个预设供应商的 `default_base_url` 与默认模型列表均可正常解析并通过 `ProviderConfig` 校验；
   - `connect.json` 原子保存、权限 `0600`、API Key / OAuth 凭证往返读写；
   - `load_runtime_ai_settings()` 正确合并 `ai.json`（Custom）与 `connect.json`（Quick Connect），且 `defaultModel` 指向快捷连接模型时通过校验；
   - 环境变量探测：当某预设未在 `connect.json` 存 Key 但环境变量存在时，识别为 `ConnectionSource::Env`。
2. **TUI `/connect` 状态机与交互测试（`src/app/update/chat/tests/connect.rs`、`src/ui/overlay/connect_tests.rs`）**：
   - 输入 `/connect` 命中 `SlashCommand::Connect`，采纳后打开 `Overlay::Connect(Box<ConnectPanel>)`（`Providers` 阶段，A-Z 排序）；
   - 在 `Providers` 阶段输入实时过滤供应商名称/ID，按 `Esc` 关闭弹窗；
   - 选择单方法（仅 API Key）未连接供应商直接进入 `InputPrompt(ApiKey)`，输入 Key 按回车触发 `Effect::SavePresetConnection`；
   - 选择多方法供应商（如 `openai` / `github-copilot`）先进入 `Methods`（OAuth 在前、API Key 在后），`Esc` 退回 `Providers`；
   - 选择已连接供应商进入 `Manage`（`Select model` / `Reconnect` / `Disconnect`）；
   - `on_preset_connected` 触发后弹出窗格内 `Connected <Name>` Toast，关闭弹窗并将 `pane.lx.slash` 自动切换为仅过滤该供应商的 `SlashPanel::Model`。
3. **设置页 HTTP API 与前端资源测试（`src/settings/service/models/tests.rs`、`src/settings/service/tests.rs`）**：
   - `GET /api/models/settings` 同时返回 `providers`（仅 Custom）与 `quickConnect`（A-Z 预设列表及连接状态）；
   - `PUT /api/models/connect` 写入 `connect.json`、不污染 `ai.json`，并投递 `SettingsEvent::ModelsChanged`；
   - `POST /api/models/connect/disconnect` 清理 `connect.json` 记录并在 `defaultModel` 悬空时自动清理；
   - `/settings_connect.js` 在静态资源白名单中可正常访问，且不含外部 URL、不含 `innerHTML`。

### 6.2 风险与回滚
- **配置文件隔离安全**：快捷连接全部写入新增的 `~/.lx-t[-dev]/config/connect.json`，绝不破坏原有的 `~/.lx-t[-dev]/config/ai.json`；若回滚版本，旧二进制直接忽略 `connect.json` 并按原样读取 `ai.json`。
- **OAuth 端口与任务泄漏防护**：本地回环回调服务仅绑定 `127.0.0.1`，且绑定 10 分钟超时与 `CancellationToken`；用户关闭面板、切换会话或退出应用时自动终止监听。
