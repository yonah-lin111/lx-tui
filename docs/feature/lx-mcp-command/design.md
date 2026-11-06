# `/mcp` 命令与窗格级 MCP 管理 — 详细技术设计 (`design.md`)

- **特性目录**：`docs/feature/lx-mcp-command/`
- **关联任务清单**：[`task.md`](task.md)
- **当前状态**：待用户审核确认
- **参考实现**：`/Users/yonah/projects/agent/opencode-dev`
  - `packages/tui/src/component/dialog-mcp.tsx`（TUI `/mcps` 面板：名称排序、模糊过滤、状态标签、原地切换/重连）
  - `packages/app/src/providers/connect/mcp-dialog.tsx`（App `/mcp` 面板）
  - `packages/app/src/new-session/mcp.ts`（会话/草稿级 MCP 开关控制）
- **目标范围**：
  - `src/app/state/lx/mcp.rs`（新增：`/mcp` 面板状态、条目状态机、工作区 MCP 目录项 `McpCatalogEntry`）
  - `src/app/state/lx/slash.rs`（注册 `SlashCommand::Mcp` 及别名 `"mcps"`，扩展 `SlashPanel::Mcp`）
  - `src/app/state/lx.rs`（`Lx` 窗格新增 `disabled_mcp_servers: BTreeSet<String>` 及读写接口）
  - `src/app/state/lx/mention.rs`（`LxMention` 过滤当前窗格已禁用的 MCP server）
  - `src/app/state.rs`（`AppState` 新增 `mcp_catalog: BTreeMap<PathBuf, Vec<McpCatalogEntry>>`，`sync_lx_inventory` 按窗格禁用集合计算 `pane.lx.startup.mcp`）
  - `src/app/effects.rs`（新增 `Effect::RefreshMcpCatalog` 与 `Effect::RestartMcpServer`）
  - `src/app/update/chat/mcp.rs`（新增：打开 `/mcp` 面板、原地切换窗格级启用/禁用、触发失败重连、实时同步已打开面板）
  - `src/app/update/chat/slash.rs`（接入 `SlashCommand::Mcp`、`SlashPanel::Mcp` 的 `Enter`/过滤分派）
  - `src/app/update/pane.rs`（`SlashPanel::Mcp` 打开时拦截 `Space` 键触发原地切换，保持面板不关闭）
  - `src/mcp/manager/snapshot.rs` & `src/mcp/manager/resources.rs`（`McpToolSnapshot::without_servers` 过滤窗格已禁用的 MCP 工具、系统提示与 resources 访问）
  - `src/event/chat/mcp.rs`、`src/event/chat/mod.rs`、`src/event/chat/subagent/{context,launch,runner}.rs`（主 Agent 与派生 Subagent 按所属窗格 `disabled_mcp_servers` 裁剪本轮冻结的 `McpToolSnapshot`）
  - `src/event/inventory.rs` & `src/event/mod.rs`（维护 `state.mcp_catalog`、处理 `RefreshMcpCatalog`/`RestartMcpServer` 效应、在 `McpEvent::Status` 到达时实时刷新已打开的 `/mcp` 面板）
  - `src/ui/main_content/lx/slash.rs` & `src/ui/text.rs`（渲染 `SlashPanel::Mcp`，提供英文文案常量与 `disabled` 掩码）
  - `src/settings/commands.rs`（将 `"mcp"` 与 `"mcps"` 加入 `RESERVED_NAMES`）
- **架构约束**：
  - 严格保持 `app/`（纯状态与更新）与 `ui/`（纯渲染）边界清晰；渲染只读状态，禁止在渲染中修改状态或执行 IO；
  - 所有 TUI 用户可见文案与提示词一律英文，集中在 `src/ui/text.rs`；
  - 生产代码禁止 `unwrap()` / `expect()`，单文件行数上限 1000 行；
  - 不修改 SQLite 表结构（`SCHEMA_VERSION` 保持不变）。

---

## 0. 现状核实与事实基线

通过 `codegraph` 与 `codebase-memory-mcp` 对 `opencode-dev` 和 `lx-tui` 进行完整核查，核心现状如下：

| 维度 | `opencode-dev` 现状 | `lx-tui` 现状（文件:行号） | 本次设计落点 |
|---|---|---|---|
| **命令入口** | App 端注册 `/mcp`（`use-session-commands.tsx:571`），TUI 端注册 `/mcps`（`packages/tui/src/app.tsx:966`） | `SlashCommand` 枚举（`src/app/state/lx/slash.rs:77-110`）含 9 个命令，无 `/mcp` | 新增 `SlashCommand::Mcp`，主命令名 `"mcp"`，别名 `&["mcps"]`，同时支持 `/mcp` 与 `/mcps` |
| **面板形态** | `DialogMcp`（`packages/tui/src/component/dialog-mcp.tsx:36-175`）展示名称 + 右侧状态，支持搜索过滤与原地切换 | `src/ui/main_content/lx/slash.rs:186-296` 使用无边框 `CommandPanelView` 贴合当前窗格输入框上沿渲染二级面板 | 新增 `SlashPanel::Mcp(Box<McpPanel>)`，在 `src/ui/main_content/lx/slash.rs` 中内联渲染，天然归属当前窗格 |
| **MCP 连接池** | `packages/core/src/mcp/index.ts` 按项目目录管理连接 | `McpManager`（`src/mcp/manager.rs:105`）按 `(PathBuf, String)` 维护进程级连接池与 `snapshot_cache` | 保持 `McpManager` 共享连接池不断开；在 `McpToolSnapshot` 层提供 `without_servers` 按窗格禁用集零成本/按需过滤 |
| **窗格状态** | 新会话草稿支持独立开关（`packages/app/src/new-session/mcp.ts:11`） | `Lx`（`src/app/state/lx.rs:99-166`）仅持有 `startup.mcp` 计数与 `mention.mcp_servers`，无禁用集合 | 在 `Lx` 上新增 `disabled_mcp_servers: BTreeSet<String>`，跟随 Pane 内存生命周期常驻 |
| **聊天与子 Agent 注入** | Turn 启动时按启用状态组装工具表 | `src/event/chat/mod.rs:606` 与 `src/event/chat/subagent/context.rs:215` 调用 `mcp::snapshot_for_turn` | 传入窗格的 `disabled_mcp_servers`，同时过滤工具声明（`tools`）、系统提示（`## MCP servers`）与 `resources` 工具 |
| **`@` 提及与首页计数** | 状态变更实时广播刷新 UI | `LxMention::filtered_mcp_servers`（`src/app/state/lx/mention.rs:145`）返回全部；`sync_lx_inventory`（`src/app/state.rs:508`）所有同工作区窗格共用同一 `StartupCount` | `LxMention` 过滤掉当前窗格已禁用项；`sync_lx_inventory` 按每个窗格自己的禁用集独立计算 `pane.lx.startup.mcp` |

---

## 1. 背景与目标 / 非目标

### 1.1 目标

1. **新增 `/mcp`（别名 `/mcps`）内置斜杠命令**：在 `lx` 窗格输入框输入 `/mcp` 或 `/mcps` 可打开当前窗格专属的 MCP 管理面板。
2. **窗格（Pane）级 MCP 禁用隔离**：
   - 每个 `Lx` 窗格独立维护 `disabled_mcp_servers: BTreeSet<String>`；
   - 在某窗格禁用某个 MCP 后，仅对该窗格发起的主对话、该窗格派生的 Subagent、`@` 提及面板的 MCP Tab、以及该窗格首页的 `N mcp` 统计计数生效；
   - 不杀死底层共享的 `McpManager` 连接进程，不改写磁盘 `mcp.json`，同一工作区下的其他窗格不受影响，重新启用时零冷启动延迟立即恢复。
3. **对齐 `opencode-dev` 的内联面板体验与原地操作**：
   - 紧贴当前窗格输入框上沿渲染无边框命令面板（`SlashPanel::Mcp`），输入框全文实时模糊过滤 MCP 名称与摘要；
   - 单行三段展示：左侧 `label`（MCP 名称）、中间 `preview`（工具数 + 传输摘要，失败时显示单行错误摘要）、右侧 `tag`（`[Connected]` / `[Connecting]` / `[Failed]` / `[Needs login]` / `[Disabled]` / `[Disabled in settings]`）；
   - 按 `Space`、`Enter`、`Tab` 或鼠标左键点击条目可原地切换启用/禁用（或对 `Failed` 条目触发后台重连），**面板保持打开**支持连续操作，按 `Esc` 关闭面板并清空过滤词。
4. **后台事件实时联动**：当 `/mcp` 面板打开期间收到 `McpEvent::Status` 或设置页变更事件，面板内的状态标签、工具数、错误摘要及窗格首页计数实时刷新。

### 1.2 非目标

1. 不在 TUI `/mcp` 面板内新增/删除 MCP 配置或编辑 JSON 参数（增删改配置仍由浏览器设置页与 `lx-tui mcp` CLI 负责）。
2. 不将窗格级 `disabled_mcp_servers` 写入磁盘 `mcp.json` 或 SQLite 数据库（遵循窗格纯内存生命周期）。
3. 不改变 `McpManager` 的跨窗格共享进程池模型。

---

## 2. 已确认的边界决策（Grill Me 结论表）

| 编号 | 决策问题 | 已确认方案 | 核心理由与行为约束 |
|---|---|---|---|
| **Q1** | Pane 级禁用 MCP 的作用机制与隔离边界 | **Pane 级纯逻辑屏蔽，底层共享连接池常驻、不改磁盘配置** | 每个 `Lx` 窗格独立维护 `disabled_mcp_servers`。禁用后从该窗格的主对话、派生 Subagent、`## MCP servers` 系统提示、`@` 提及 MCP 列表与首页 `N mcp` 计数中剔除；其他窗格不受影响，再次启用时无需等待进程重启。 |
| **Q2** | `/mcp` 面板的 UI 承载形态与单行展示内容 | **窗格内联 `SlashPanel::Mcp` + 左名称、中摘要/错误、右状态标签** | 复用 `src/ui/main_content/lx/slash.rs` 的无边框内联面板，不遮挡其他分屏；输入框直接作为搜索框；单行使用 `CommandItem::Inline { label, preview, tag }`。 |
| **Q3** | 面板内按键交互与特殊状态（失败/全局禁用）处理 | **`Space`/`Enter` 原地连续切换且保持面板打开；`Failed` 支持重连；全局禁用置灰区分** | 按 `Space`/`Enter`/点击原地切换后不关面板（`Esc` 才关闭）；未禁用的 `Failed` 条目触发后台重连（变为 `Connecting`，再按可切换为窗格禁用）；`mcp.json` 中 `enabled: false` 的条目置灰显示 `[Disabled in settings]` 并拦截开启。 |
| **Q4** | Pane 级禁用状态的生命周期 | **绑定在 `Lx` 窗格实例上跨会话保持，新分屏默认全开，后台事件实时刷新** | 在同一 Pane 内执行 `/new`、`/resume`、`/fork` 时保留 `disabled_mcp_servers`；分屏/新 Tab 的新窗格默认空集合（全开）；`McpEvent::Status` 实时刷新已打开的 `/mcp` 面板。 |

---

## 3. 核心架构与数据流

### 3.1 整体数据流图

```text
┌──────────────────────────────────────────────────────────────────────────────┐
│ 磁盘配置层 (src/config/mcp.rs)                                               │
│ user: config/mcp.json  +  project: config/mcp/projects/<scope>.json          │
└───────────────────────────────┬──────────────────────────────────────────────┘
                                │ effective_for_root(persistence, root)
                                ▼
┌──────────────────────────────────────────────────────────────────────────────┐
│ 事件与状态汇聚层 (src/event/inventory.rs + src/app/state.rs)                 │
│                                                                              │
│  McpEvent::Status ──► AppState.mcp_catalog: BTreeMap<PathBuf, Vec<McpCatalogEntry>>
│  (Ready{tools} /      ├── name, summary, config_enabled                      │
│   Starting /          └── runtime: Connecting | Ready{tools} | Failed{error} │
│   Failed{error} /                  | NeedsLogin                              │
│   NeedsLogin)                                                                │
└───────┬───────────────────────┬──────────────────────────────────────────────┘
        │                       │
        │ 实时同步               │ 按窗格裁剪 (排除 pane.lx.disabled_mcp_servers)
        ▼                       ▼
┌───────────────────────┐ ┌────────────────────────────────────────────────────┐
│ 窗格交互层 (Lx)        │ │ 轮次冻结与执行层 (src/event/chat/)                  │
│                       │ │                                                    │
│ Lx.disabled_mcp_servers │ 1. 主轮次: start_stream (event/chat/mod.rs)        │
│   ├── SlashPanel::Mcp │ │    mcp::snapshot_for_turn_with_disabled(...)       │
│   │   Space/Enter 切换│ │ 2. 子轮次: capture_child_prompt_inputs             │
│   │   Failed 触发重连 │ │    继承父窗格 parent_key.pane 的禁用集合            │
│   ├── LxMention       │ │ 3. McpToolSnapshot::without_servers(disabled)      │
│   │   过滤 @ MCP 候选 │ │    ├── 剔除被禁用 server 的 ToolDecl               │
│   └── Lx.startup.mcp  │ │    ├── 剔除 ## MCP servers 对应提示段落            │
│       窗格独立计数    │ │    └── 拦截被禁用 server 的 resources 读写         │
└───────────────────────┘ └────────────────────────────────────────────────────┘
```

### 3.2 `/mcp` 面板单条目状态转换机

对于任意配置中的 MCP Server，其在当前窗格 `/mcp` 面板中的展示状态 `McpServerState` 由 **全局配置开关（`config_enabled`）**、**当前窗格禁用集（`lx.disabled_mcp_servers`）** 与 **底层运行态（`McpRuntimeState`）** 共同决定：

```text
1. config_enabled == false
   ──► McpServerState::DisabledInSettings ("[Disabled in settings]", 置灰只读)
       └─ 按 Space/Enter：保持不变，弹出窗格内 Warning Toast 提示去设置页开启

2. config_enabled == true && lx.disabled_mcp_servers.contains(name)
   ──► McpServerState::DisabledInPane ("[Disabled]")
       └─ 按 Space/Enter：从 lx.disabled_mcp_servers 移除（恢复在本窗格启用）
          ├─ 若底层 runtime 为 Failed：同时触发 Effect::RestartMcpServer 重连，转为 Connecting
          └─ 否则：立即恢复为底层当前状态（Connected / Connecting / NeedsLogin）

3. config_enabled == true && !lx.disabled_mcp_servers.contains(name)
   ├─ runtime == Ready { tools } ──► McpServerState::Connected { tools } ("[Connected]")
   │    └─ 按 Space/Enter：加入 lx.disabled_mcp_servers ──► DisabledInPane ("[Disabled]")
   ├─ runtime == Connecting      ──► McpServerState::Connecting ("[Connecting]")
   │    └─ 按 Space/Enter：加入 lx.disabled_mcp_servers ──► DisabledInPane ("[Disabled]")
   ├─ runtime == NeedsLogin      ──► McpServerState::NeedsLogin ("[Needs login]")
   │    └─ 按 Space/Enter：加入 lx.disabled_mcp_servers ──► DisabledInPane ("[Disabled]")
   └─ runtime == Failed { error }──► McpServerState::Failed { error } ("[Failed]")
        └─ 按 Space/Enter：触发 Effect::RestartMcpServer 后台重连，runtime 置为 Connecting
           （此时再按一次 Space/Enter 即可将其加入 lx.disabled_mcp_servers 变为 DisabledInPane）
```

---

## 4. 详细技术设计与关键代码

### 4.1 状态层新增模块：`src/app/state/lx/mcp.rs`

由于 `src/app/state/lx/slash.rs` 已有 870 行，遵循「单文件上限 1000 行、子模块控制在 500 行以内」规范，将 `/mcp` 面板状态与工作区 MCP 目录项独立拆入 `src/app/state/lx/mcp.rs`（约 240 行），并在 `src/app/state/lx.rs` 与 `src/app/state.rs` 中导出。

```rust
//! `/mcp` 命令面板状态与工作区 MCP 目录快照。
//!
//! 纯状态层：不直接读取磁盘配置或访问 `McpManager`；
//! 由事件层在启动、配置变更与 `McpEvent::Status` 到达时同步 `McpCatalogEntry`，
//! 并结合每个窗格的 `Lx::disabled_mcp_servers` 投影为面板条目。

use std::collections::BTreeSet;

use super::slash::{PanelWindow, best_score};
use crate::ui::text;

/// 工作区单个 MCP server 的底层运行状态（跨同项目窗格共享）。
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum McpRuntimeState {
    /// 正在启动或重连中。
    #[default]
    Connecting,
    /// 已连接就绪（携带可用工具数量）。
    Ready { tools: usize },
    /// 启动或运行失败（携带错误原因）。
    Failed { error: String },
    /// 需要 OAuth 登录授权。
    NeedsLogin,
}

/// 工作区生效配置中的一条 MCP server 目录项（存入 `AppState::mcp_catalog`）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct McpCatalogEntry {
    /// MCP server 唯一名称。
    pub name: String,
    /// 传输摘要（如 `stdio: npx …` 或 `https://…`）。
    pub summary: String,
    /// 配置文件（`mcp.json`）中的 `enabled` 字段。
    pub config_enabled: bool,
    /// 底层运行状态（仅 `config_enabled == true` 时有意义）。
    pub runtime: McpRuntimeState,
}

/// 单个 MCP server 在特定窗格 `/mcp` 面板中的展示状态。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum McpServerState {
    /// 在本窗格启用且底层已就绪。
    Connected { tools: usize },
    /// 在本窗格启用且底层正在连接。
    Connecting,
    /// 在本窗格启用但底层连接失败。
    Failed { error: String },
    /// 在本窗格启用但需要 OAuth 登录。
    NeedsLogin,
    /// 在当前窗格被手动禁用（底层连接保持不变）。
    DisabledInPane,
    /// 在全局/项目设置中被禁用（`mcp.json` 的 `enabled: false`），本窗格只读置灰。
    DisabledInSettings,
}

/// `/mcp` 面板中的单行条目。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct McpPanelEntry {
    pub name: String,
    pub summary: String,
    pub preview: String,
    pub state: McpServerState,
}

impl McpPanelEntry {
    /// 由工作区目录项与当前窗格禁用集合构造面板条目。
    pub fn from_catalog(entry: &McpCatalogEntry, pane_disabled: &BTreeSet<String>) -> Self {
        let state = if !entry.config_enabled {
            McpServerState::DisabledInSettings
        } else if pane_disabled.contains(&entry.name) {
            McpServerState::DisabledInPane
        } else {
            match &entry.runtime {
                McpRuntimeState::Ready { tools } => McpServerState::Connected { tools: *tools },
                McpRuntimeState::Connecting => McpServerState::Connecting,
                McpRuntimeState::Failed { error } => McpServerState::Failed {
                    error: sanitize_single_line(error),
                },
                McpRuntimeState::NeedsLogin => McpServerState::NeedsLogin,
            }
        };
        let preview = build_preview(&entry.summary, &state);
        Self {
            name: entry.name.clone(),
            summary: entry.summary.clone(),
            preview,
            state,
        }
    }

    /// 是否因全局/项目设置禁用而在面板中置灰只读。
    pub fn is_settings_disabled(&self) -> bool {
        matches!(self.state, McpServerState::DisabledInSettings)
    }

    /// 右对齐状态标签文案。
    pub fn status_tag(&self) -> &'static str {
        match &self.state {
            McpServerState::Connected { .. } => text::LX_MCP_STATUS_CONNECTED,
            McpServerState::Connecting => text::LX_MCP_STATUS_CONNECTING,
            McpServerState::Failed { .. } => text::LX_MCP_STATUS_FAILED,
            McpServerState::NeedsLogin => text::LX_MCP_STATUS_NEEDS_LOGIN,
            McpServerState::DisabledInPane => text::LX_MCP_STATUS_DISABLED,
            McpServerState::DisabledInSettings => text::LX_MCP_STATUS_DISABLED_SETTINGS,
        }
    }
}

fn sanitize_single_line(raw: &str) -> String {
    raw.lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .unwrap_or_default()
        .to_string()
}

fn build_preview(summary: &str, state: &McpServerState) -> String {
    match state {
        McpServerState::Connected { tools } => {
            let tool_label = if *tools == 1 {
                "1 tool".to_string()
            } else {
                format!("{tools} tools")
            };
            if summary.is_empty() {
                tool_label
            } else {
                format!("{tool_label} · {summary}")
            }
        }
        McpServerState::Failed { error } if !error.is_empty() => error.clone(),
        _ => summary.to_string(),
    }
}

/// `/mcp` 窗格内联面板状态。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct McpPanel {
    /// 当前工作区的全部 MCP 条目（按名称字典序）。
    pub entries: Vec<McpPanelEntry>,
    /// 滚动与高亮窗口。
    pub window: PanelWindow,
    /// 当前过滤词（来自输入框全文）。
    pub filter: String,
    /// 过滤后的条目在 `entries` 中的索引。
    pub filtered: Vec<usize>,
}

impl McpPanel {
    /// 以初始条目列表创建面板（默认高亮第 0 项，不过滤）。
    pub fn new(entries: Vec<McpPanelEntry>) -> Self {
        let len = entries.len();
        Self {
            entries,
            window: PanelWindow::new(0),
            filter: String::new(),
            filtered: (0..len).collect(),
        }
    }

    /// 同步最新条目（原地切换或后台事件到达时调用），保留当前高亮的 server 名与过滤词。
    pub fn sync_entries(&mut self, entries: Vec<McpPanelEntry>) {
        let active_name = self.active_entry().map(|entry| entry.name.clone());
        self.entries = entries;
        let query = self.filter.clone();
        self.apply_filter(&query);
        if let Some(name) = active_name
            && let Some(pos) = self
                .filtered
                .iter()
                .position(|&idx| self.entries.get(idx).is_some_and(|e| e.name == name))
        {
            self.window.set_active(pos);
        }
        self.clamp_window();
    }

    /// 按输入框全文重算过滤结果。
    pub fn refilter(&mut self, query: &str) {
        if self.filter != query {
            self.filter = query.to_string();
            self.window = PanelWindow::default();
        }
        self.apply_filter(query);
    }

    fn apply_filter(&mut self, query: &str) {
        if query.trim().is_empty() {
            self.filtered = (0..self.entries.len()).collect();
            self.clamp_window();
            return;
        }
        let query = query.trim().to_lowercase();
        let mut scored: Vec<(u32, usize)> = self
            .entries
            .iter()
            .enumerate()
            .filter_map(|(index, entry)| {
                best_score(
                    &[&entry.name, &entry.summary, entry.status_tag()],
                    &query,
                )
                .map(|score| (score, index))
            })
            .collect();
        scored.sort_by(|left, right| right.0.cmp(&left.0).then_with(|| left.1.cmp(&right.1)));
        self.filtered = scored.into_iter().map(|(_, index)| index).collect();
        self.clamp_window();
    }

    /// 过滤后可见条目列表（供渲染与命中使用）。
    pub fn visible(&self) -> Vec<&McpPanelEntry> {
        self.filtered
            .iter()
            .filter_map(|index| self.entries.get(*index))
            .collect()
    }

    /// 当前高亮条目。
    pub fn active_entry(&self) -> Option<&McpPanelEntry> {
        self.filtered
            .get(self.window.active)
            .and_then(|index| self.entries.get(*index))
    }

    /// 钳制窗口范围。
    pub fn clamp_window(&mut self) {
        self.window.clamp(self.filtered.len());
    }

    /// 是否有过滤词但无匹配结果。
    pub fn is_filter_empty(&self) -> bool {
        !self.filter.trim().is_empty() && self.filtered.is_empty()
    }
}
```

---

### 4.2 命令注册与 `SlashPanel` 扩展：`src/app/state/lx/slash.rs`

1. 在 `SlashCommand` 枚举中新增 `Mcp`：
```rust
pub enum SlashCommand {
    Model,
    Resume,
    Fork,
    New,
    Rewind,
    Compact,
    Subagents,
    Rename,
    Connect,
    /// `/mcp`（别名 `/mcps`）：管理当前窗格的 MCP servers。
    Mcp,
}
```
2. 更新 `SlashCommand::ALL` 长度为 10，并实现各契约方法：
   - `name()`：`Self::Mcp => "mcp"`
   - `aliases()`：`Self::Mcp => &["mcps"]`
   - `text()`：`Self::Mcp => "/mcp"`
   - `description()`：`Self::Mcp => crate::ui::text::LX_MCP_COMMAND_DESC`
3. 在 `SlashPanel` 枚举中新增 `Mcp(Box<crate::app::state::lx::mcp::McpPanel>)`：
```rust
pub enum SlashPanel {
    Commands {
        items: Vec<SlashEntry>,
        window: PanelWindow,
        query: String,
    },
    Model(Box<ModelPicker>),
    History(Box<HistoryPanel>),
    Fork(Box<ForkPanel>),
    Rewind(Box<crate::app::state::lx::rewind::RewindPanel>),
    /// `/mcp` 窗格级 MCP 管理面板。
    Mcp(Box<crate::app::state::lx::mcp::McpPanel>),
}
```
4. 在 `SlashPanel` 的 `item_count()`、`window()`、`current_index()`、`set_window()` 中穷尽匹配 `Self::Mcp(panel)`：
   - `item_count()`：`Self::Mcp(panel) => panel.filtered.len()`
   - `window()`：`Self::Mcp(panel) => panel.window`
   - `current_index()`：`Self::Mcp(_) => None`
   - `set_window()`：`Self::Mcp(panel) => panel.window = window`
5. 在 `src/settings/commands.rs:222` 的 `RESERVED_NAMES` 中加入 `"mcp"` 与 `"mcps"`，防止自定义命令覆盖。

---

### 4.3 `Lx` 窗格与 `@` 提及状态：`src/app/state/lx.rs` & `src/app/state/lx/mention.rs`

1. **`src/app/state/lx.rs`**：
   - 在 `Lx` 结构体中新增字段：
     ```rust
     /// 本窗格手动禁用的 MCP server 名称集合（仅内存，跨会话保持，新分屏默认为空）。
     disabled_mcp_servers: std::collections::BTreeSet<String>,
     ```
   - 提供公开方法（自动同步给 `self.mention`）：
     ```rust
     /// 当前窗格已禁用的 MCP server 集合。
     pub fn disabled_mcp_servers(&self) -> &std::collections::BTreeSet<String> {
         &self.disabled_mcp_servers
     }

     /// 判断指定 MCP server 在当前窗格是否被禁用。
     pub fn is_mcp_server_disabled(&self, name: &str) -> bool {
         self.disabled_mcp_servers.contains(name)
     }

     /// 切换或设置指定 MCP server 在当前窗格的禁用状态；返回状态是否发生变化。
     pub fn set_mcp_server_disabled(&mut self, name: &str, disabled: bool) -> bool {
         let changed = if disabled {
             self.disabled_mcp_servers.insert(name.to_string())
         } else {
             self.disabled_mcp_servers.remove(name)
         };
         if changed {
             let masked = crate::app::paste::mask_pasted_text(&self.input).into_owned();
             self.mention
                 .set_disabled_mcp_servers(self.disabled_mcp_servers.clone(), &masked, self.cursor);
         }
         changed
     }
     ```
2. **`src/app/state/lx/mention.rs`**：
   - 在 `LxMention` 中新增字段 `disabled_mcp_servers: std::collections::BTreeSet<String>`；
   - 新增 `set_disabled_mcp_servers` 方法并在 `filtered_mcp_servers(&self)` 中过滤：
     ```rust
     pub fn filtered_mcp_servers(&self) -> Vec<&McpMentionEntry> {
         let query = self
             .trigger
             .as_ref()
             .map(|t| t.query.as_str())
             .unwrap_or("");
         let query_lower = query.to_lowercase();
         self.mcp_servers
             .iter()
             .filter(|server| !self.disabled_mcp_servers.contains(&server.name))
             .filter(|server| {
                 query_lower.is_empty()
                     || server.name.to_lowercase().contains(&query_lower)
                     || server.detail.to_lowercase().contains(&query_lower)
             })
             .collect()
     }
     ```
   - `LxMention::mcp_servers(&self)` 仍返回未过滤的原始 `&self.mcp_servers`，从而保证 `Tab::split_pane`（`src/app/state/pane.rs:96`）复制候选列表到新分屏时，新分屏拥有完整候选且默认不禁用任何 MCP（符合 Q4）。

---

### 4.4 `AppState` 目录与窗格独立计数：`src/app/state.rs`

1. 在 `AppState` 中新增字段：
   ```rust
   /// 按项目根汇总的生效 MCP server 目录与实时运行状态（供 `/mcp` 面板与窗格独立计数使用）。
   pub mcp_catalog: BTreeMap<PathBuf, Vec<crate::app::state::lx::mcp::McpCatalogEntry>>,
   ```
2. 新增辅助方法 `mcp_startup_count_for_pane(root: Option<&Path>, disabled: &BTreeSet<String>) -> StartupCount`：
   - 若 `self.mcp_catalog.get(root)` 存在：
     遍历 `entry in catalog.iter().filter(|e| e.config_enabled && !disabled.contains(&e.name))`：
     - `total += 1`
     - `Ready { .. } => ready += 1`
     - `Failed { .. } | NeedsLogin => errors += 1`
     - `Connecting => starting += 1`
     - `loading = starting > 0 || (total > 0 && ready.saturating_add(errors) < total)`
   - 若 `self.mcp_catalog.get(root)` 不存在（例如直接写 `state.mcp_inventory` 的既有单元测试）：
     回退读取 `self.mcp_inventory.get(root).copied().unwrap_or_default()`，但若 `disabled` 非空则安全扣减，保证既有测试 100% 兼容。
3. 在 `AppState::sync_lx_inventory(&mut self)` 中，每个窗格按自己的 `pane.lx.disabled_mcp_servers()` 计算 `StartupCounts.mcp`：
   ```rust
   let mcp = Self::compute_pane_mcp_count(
       &self.mcp_catalog,
       &self.mcp_inventory,
       root.as_deref(),
       pane.lx.disabled_mcp_servers(),
   );
   ```

---

### 4.5 应用更新层与键盘路由：`src/app/update/chat/mcp.rs`、`slash.rs`、`pane.rs`

1. **新增 `src/app/update/chat/mcp.rs`**：
   ```rust
   //! `/mcp` 面板打开、原地切换与重连分派。

   use std::path::PathBuf;

   use crate::app::effects::Effect;
   use crate::app::state::lx::mcp::{
       McpCatalogEntry, McpPanel, McpPanelEntry, McpRuntimeState, McpServerState,
   };
   use crate::app::state::lx::slash::SlashPanel;
   use crate::app::state::{AppState, InputHintLevel};
   use crate::layout::PaneId;
   use crate::ui::text;

   use super::session::show_lx_hint;

   /// 解析窗格所属的 MCP 配置根（仓库主 checkout 优先，回退工作区根）。
   pub fn mcp_root_for_pane(state: &AppState, pane: PaneId) -> Option<PathBuf> {
       state
           .chat_session_key_for_pane(pane)
           .or_else(|| state.active_workspace().repo_key().map(PathBuf::from))
   }

   /// 按当前 `state.mcp_catalog` 与窗格禁用集构建 `McpPanelEntry` 列表。
   pub fn build_panel_entries(state: &AppState, pane: PaneId) -> Vec<McpPanelEntry> {
       let Some(root) = mcp_root_for_pane(state, pane) else {
           return Vec::new();
       };
       let Some(catalog) = state.mcp_catalog.get(&root) else {
           return Vec::new();
       };
       let Some(target) = state.pane_anywhere(pane) else {
           return Vec::new();
       };
       let disabled = target.lx.disabled_mcp_servers();
       catalog
           .iter()
           .map(|entry| McpPanelEntry::from_catalog(entry, disabled))
           .collect()
   }

   /// `/mcp` 打开窗格内联 MCP 管理面板。
   pub fn open_mcp_panel(state: &mut AppState, pane: PaneId) -> bool {
       if !state
           .pane_anywhere(pane)
           .is_some_and(|target| target.view == crate::app::state::PaneView::Lx)
       {
           return false;
       }
       let entries = build_panel_entries(state, pane);
       let Some(target) = state.pane_mut_anywhere(pane) else {
           return false;
       };
       target.lx.slash = Some(SlashPanel::Mcp(Box::new(McpPanel::new(entries))));
       state.effects.push(Effect::RefreshMcpCatalog { pane });
       true
   }

   /// 在 `/mcp` 面板中对当前高亮项执行切换或重连（Enter / Space / 鼠标点击），面板保持打开。
   pub fn toggle_mcp_entry(state: &mut AppState, pane: PaneId) -> bool {
       let Some(active) = state.pane_anywhere(pane).and_then(|target| match &target.lx.slash {
           Some(SlashPanel::Mcp(panel)) => panel.active_entry().cloned(),
           _ => None,
       }) else {
           return true;
       };

       match active.state {
           McpServerState::DisabledInSettings => {
               show_lx_hint(
                   state,
                   pane,
                   InputHintLevel::Warning,
                   text::LX_MCP_DISABLED_IN_SETTINGS,
               );
               return true;
           }
           McpServerState::Failed { .. } => {
               // 当前窗格已启用但底层连接失败：触发后台重连，状态立即切为 Connecting。
               mark_catalog_connecting(state, pane, &active.name);
               state.effects.push(Effect::RestartMcpServer {
                   pane,
                   server: active.name,
               });
           }
           McpServerState::DisabledInPane => {
               // 重新在当前窗格启用；若底层此前处于 Failed，顺带触发后台重连。
               if let Some(target) = state.pane_mut_anywhere(pane) {
                   target.lx.set_mcp_server_disabled(&active.name, false);
               }
               if is_catalog_failed(state, pane, &active.name) {
                   mark_catalog_connecting(state, pane, &active.name);
                   state.effects.push(Effect::RestartMcpServer {
                       pane,
                       server: active.name,
                   });
               }
           }
           McpServerState::Connected { .. }
           | McpServerState::Connecting
           | McpServerState::NeedsLogin => {
               // 在当前窗格禁用（不杀后台连接）。
               if let Some(target) = state.pane_mut_anywhere(pane) {
                   target.lx.set_mcp_server_disabled(&active.name, true);
               }
           }
       }

       refresh_pane_mcp_panel(state, pane);
       state.sync_lx_inventory();
       true
   }

   /// 刷新指定窗格已打开的 `SlashPanel::Mcp` 条目（保持高亮项与过滤词）。
   pub fn refresh_pane_mcp_panel(state: &mut AppState, pane: PaneId) -> bool {
       let is_mcp_open = state
           .pane_anywhere(pane)
           .is_some_and(|target| matches!(target.lx.slash, Some(SlashPanel::Mcp(_))));
       if !is_mcp_open {
           return false;
       }
       let entries = build_panel_entries(state, pane);
       if let Some(target) = state.pane_mut_anywhere(pane)
           && let Some(SlashPanel::Mcp(panel)) = target.lx.slash.as_mut()
       {
           panel.sync_entries(entries);
           return true;
       }
       false
   }

   /// 刷新所有打开了 `SlashPanel::Mcp` 的窗格。
   pub fn refresh_all_open_mcp_panels(state: &mut AppState) -> bool {
       let panes = state.all_pane_ids();
       let mut changed = false;
       for pane in panes {
           changed |= refresh_pane_mcp_panel(state, pane);
       }
       changed
   }
   ```

2. **键盘路由（`src/app/update/pane.rs:280-360`）**：
   - 在 `if let Some(slash) = lx.slash.as_ref()` 分支中：
     - 当 `matches!(slash, SlashPanel::Mcp(_))` 且 `command == LxInput::Char(' ')` 时：
       直接调用 `super::chat::apply_slash_enter(state, pane_id); return;`（空格键直接切换当前高亮 MCP，不写入输入框）；
     - 其余字符编辑键照常放行到 `apply_editor_edit` -> `after_editor_edit` -> `refresh_slash_panel` -> `refresh_slash_filter`，在 `refresh_slash_filter` 中新增：
       `Some(SlashPanel::Mcp(panel)) => panel.refilter(&query),`
     - `LxInput::Escape`：落入 `_ => { lx.slash = None; lx.clear(); }`，关闭面板并清空过滤词。

---

### 4.6 `McpToolSnapshot` 窗格级过滤与 Turn 注入：`src/mcp/` & `src/event/chat/`

1. **`src/mcp/manager/snapshot.rs`**：
   在 `McpToolSnapshot` 上新增 `without_servers` 方法：
   ```rust
   /// 返回排除了 `disabled_servers` 后的快照；若无交集则直接克隆原 `Arc`（零拷贝）。
   pub fn without_servers(
       self: &Arc<Self>,
       disabled_servers: &std::collections::BTreeSet<String>,
   ) -> Arc<Self> {
       if disabled_servers.is_empty()
           || !self
               .servers
               .iter()
               .any(|server| disabled_servers.contains(&server.name))
       {
           return Arc::clone(self);
       }
       Arc::new(Self {
           project: self.project.clone(),
           tools: self
               .tools
               .iter()
               .filter(|tool| !disabled_servers.contains(&tool.server))
               .cloned()
               .collect(),
           servers: self
               .servers
               .iter()
               .filter(|server| !disabled_servers.contains(&server.name))
               .cloned()
               .collect(),
       })
   }
   ```
2. **`src/mcp/manager/resources.rs:165`**：
   在 `connection_for` 中增加 `snapshot.server(server)?` 校验：
   ```rust
   async fn connection_for(
       &self,
       snapshot: &McpToolSnapshot,
       server: &str,
   ) -> Option<Arc<Connection>> {
       let project = snapshot.project()?;
       snapshot.server(server)?;
       let slots = self.slots.lock().await;
       slots
           .get(&(project.to_path_buf(), server.to_string()))
           .and_then(|slot| slot.connection.clone())
   }
   ```
   这样即使模型在 `read_resource` 中硬编码了已被当前窗格禁用的 `server` 名，也会因 `snapshot.server(server)` 为 `None` 而被安全拦截。
3. **`src/event/chat/mcp.rs`**：
   新增 `snapshot_for_turn_with_disabled`：
   ```rust
   pub(crate) fn snapshot_for_turn_with_disabled(
       persistence: &Persistence,
       workspace_root: Option<&Path>,
       disabled_servers: &std::collections::BTreeSet<String>,
   ) -> Arc<McpToolSnapshot> {
       snapshot_for_turn(persistence, workspace_root).without_servers(disabled_servers)
   }
   ```
4. **主对话与 Subagent 接线**：
   - `src/event/chat/mod.rs:606`：
     ```rust
     let mcp_snapshot = mcp::snapshot_for_turn_with_disabled(
         persistence,
         config_root.as_deref(),
         pane.lx.disabled_mcp_servers(),
     );
     ```
   - `src/event/chat/subagent/context.rs:183`（`capture_child_prompt_inputs`）：
     新增 `parent_pane: PaneId` 参数，从 `state.pane_anywhere(parent_pane)` 读取 `disabled_mcp_servers`，调用 `snapshot_for_turn_with_disabled`，使该窗格派生的所有 Subagent 严格继承父窗格的 MCP 禁用集合。

---

### 4.7 事件层目录同步与效应执行：`src/event/inventory.rs` & `src/event/mod.rs`

1. **`src/event/inventory.rs`**：
   - 新增 `sync_mcp_catalog_for_root(state, persistence, root, statuses)`：
     读取 `effective_for_root(persistence, root)`，结合 `statuses` 与 `global_manager().cached_snapshot(root)`（用于读取已就绪 server 的精确 `tools` 数量），构建 `Vec<McpCatalogEntry>` 写入 `state.mcp_catalog`。
   - 在 `initialize_mcp`、`refresh_mcp`、`drain_mcp_events` 中同步更新 `state.mcp_catalog`，并调用 `crate::app::update::chat::mcp::refresh_all_open_mcp_panels(state)` 与 `state.sync_lx_inventory()`。
   - 新增 `apply_mcp_effects(state, persistence, statuses) -> bool`：
     消费 `Effect::RefreshMcpCatalog { pane }` 与 `Effect::RestartMcpServer { pane, server }`：
     - `RefreshMcpCatalog { pane }`：重读该窗格所属项目的 `effective_for_root`，刷新 `state.mcp_catalog`、面板条目与首页计数；
     - `RestartMcpServer { pane, server }`：对该窗格所属项目调用 `super::chat::mcp::restart_projects(persistence, &[root], Some(&server))`。

---

### 4.8 UI 渲染层与文案：`src/ui/main_content/lx/slash.rs` & `src/ui/text.rs`

1. **`src/ui/text.rs` 新增英文文案常量**：
   ```rust
   /// `/mcp` 命令描述。
   pub const LX_MCP_COMMAND_DESC: &str = "Manage MCP servers for this pane";
   /// `/mcp` 面板标题。
   pub const LX_MCP_PANEL_TITLE: &str = "MCP servers";
   /// `/mcp` 面板底部快捷键提示。
   pub const LX_MCP_PANEL_FOOTER: &str = "↑↓ select · space/↵ toggle · esc close";
   /// `/mcp` 空状态（未配置任何 MCP server）。
   pub const LX_MCP_EMPTY: &str = "No MCP servers configured";
   /// `/mcp` 状态标签。
   pub const LX_MCP_STATUS_CONNECTED: &str = "Connected";
   pub const LX_MCP_STATUS_CONNECTING: &str = "Connecting";
   pub const LX_MCP_STATUS_FAILED: &str = "Failed";
   pub const LX_MCP_STATUS_NEEDS_LOGIN: &str = "Needs login";
   pub const LX_MCP_STATUS_DISABLED: &str = "Disabled";
   pub const LX_MCP_STATUS_DISABLED_SETTINGS: &str = "Disabled in settings";
   /// 尝试在窗格内切换已被全局配置禁用的 MCP 时的提示。
   pub const LX_MCP_DISABLED_IN_SETTINGS: &str =
       "This MCP server is disabled in settings. Enable it in Settings first.";
   ```
2. **`src/ui/main_content/lx/slash.rs`**：
   - 在 `items()` 中新增 `SlashPanel::Mcp(panel)` 分支：
     ```rust
     SlashPanel::Mcp(panel) => {
         if panel.entries.is_empty() {
             return vec![CommandItem::Inline {
                 label: text::LX_MCP_EMPTY,
                 preview: "",
                 tag: "",
             }];
         }
         if panel.is_filter_empty() {
             return vec![CommandItem::Inline {
                 label: text::LX_SLASH_NO_MATCHES,
                 preview: "",
                 tag: "",
             }];
         }
         panel
             .visible()
             .into_iter()
             .map(|entry| CommandItem::Inline {
                 label: entry.name.as_str(),
                 preview: entry.preview.as_str(),
                 tag: entry.status_tag(),
             })
             .collect()
     }
     ```
   - 在 `is_placeholder()` 中加入：
     `SlashPanel::Mcp(p) => p.entries.is_empty() || p.is_filter_empty(),`
   - 新增 `disabled_mask(panel: &SlashPanel) -> Option<Vec<bool>>`：
     当 `SlashPanel::Mcp(panel)` 非空态时，生成 `panel.visible().into_iter().map(McpPanelEntry::is_settings_disabled).collect()`，传入 `CommandPanelView.disabled`，使 `[Disabled in settings]` 行自动以 `style::muted()` 置灰渲染。
   - 在 `view()` 中新增 `SlashPanel::Mcp(panel)` 的标题（`LX_MCP_PANEL_TITLE`）与底栏（`LX_MCP_PANEL_FOOTER`）。

---

## 5. 文件改动清单与行数预算

| 文件路径 | 操作 | 现有行数 | 预计改动后行数 | 预算合规（生产 ≤1000 / 测试 ≤1500） |
|---|---|---:|---:|---|
| `src/app/state/lx/mcp.rs` | 新增 | 0 | ~220 | 合规 |
| `src/app/state/lx/mcp/tests.rs` | 新增 | 0 | ~180 | 合规 |
| `src/app/state/lx/slash.rs` | 修改 | 870 | ~895 | 合规 |
| `src/app/state/lx.rs` | 修改 | 695 | ~730 | 合规 |
| `src/app/state/lx/mention.rs` | 修改 | 445 | ~470 | 合规 |
| `src/app/state.rs` | 修改 | 815 | ~865 | 合规 |
| `src/app/effects.rs` | 修改 | 320 | ~335 | 合规 |
| `src/app/update/chat/mcp.rs` | 新增 | 0 | ~165 | 合规 |
| `src/app/update/chat/slash.rs` | 修改 | 700 | ~710 | 合规 |
| `src/app/update/pane.rs` | 修改 | 780 | ~790 | 合规 |
| `src/mcp/manager/snapshot.rs` | 修改 | 81 | ~110 | 合规 |
| `src/mcp/manager/resources.rs` | 修改 | 287 | ~290 | 合规 |
| `src/event/chat/mcp.rs` | 修改 | 113 | ~130 | 合规 |
| `src/event/chat/mod.rs` | 修改 | 850 | ~855 | 合规 |
| `src/event/chat/subagent/context.rs` | 修改 | 265 | ~275 | 合规 |
| `src/event/chat/subagent/launch.rs` | 修改 | 290 | ~295 | 合规 |
| `src/event/chat/subagent/runner.rs` | 修改 | 880 | ~885 | 合规 |
| `src/event/inventory.rs` | 修改 | 440 | ~520 | 合规 |
| `src/event/mod.rs` | 修改 | 640 | ~648 | 合规 |
| `src/ui/main_content/lx/slash.rs` | 修改 | 296 | ~340 | 合规 |
| `src/ui/text.rs` | 修改 | 1240 | ~1265 | 合规（常量表文件） |
| `src/settings/commands.rs` | 修改 | 280 | ~282 | 合规 |

---

## 6. 测试矩阵

1. **状态与过滤单测（`src/app/state/lx/mcp/tests.rs` & `src/app/state/lx/slash/tests.rs`）**：
   - `/mcp` 与别名 `/mcps` 在 `SlashPanel::commands_matching` 中均可命中 `SlashCommand::Mcp`；
   - `McpPanelEntry::from_catalog` 正确映射 6 种状态（`Connected` 含单复数 `1 tool`/`N tools`、`Connecting`、`Failed` 单行化错误摘要、`NeedsLogin`、`DisabledInPane`、`DisabledInSettings`）；
   - `McpPanel::refilter` 与 `sync_entries` 保持当前高亮 server 名称不漂移。
2. **窗格隔离与交互单测（`src/app/update/chat/tests/slash_panels.rs`）**：
   - 在 Pane 1 执行 `/mcp` 打开面板，按 `Space` 或 `Enter` 切换某个 `Connected` MCP 为 `Disabled`，面板保持打开；
   - Pane 1 的 `disabled_mcp_servers`、面板状态标签、`@` 提及 `filtered_mcp_servers` 与 `pane.lx.startup.mcp` 同步更新，而同工作区的 Pane 2 完全不受影响；
   - 在同一 Pane 内执行 `/new` 后，`disabled_mcp_servers` 依然保留；分屏新建的 Pane 默认不禁用任何 MCP；
   - 对 `Failed` 条目按 `Space`/`Enter` 发出 `Effect::RestartMcpServer` 并切为 `Connecting`；对 `DisabledInSettings` 条目按 `Space`/`Enter` 弹出窗格内警告 Toast 且不改变状态。
3. **快照裁剪单测（`src/mcp/manager/tests.rs`）**：
   - `McpToolSnapshot::without_servers` 剔除指定 server 的工具与 `ServerSnapshot`，`instructions_prompt()` 不再包含被禁用的 server，当唯一含 resources 的 server 被禁用时 `has_resources()` 返回 `false`。
4. **UI 渲染单测（`src/ui/main_content/lx/tests/slash_panels.rs`）**：
   - `SlashPanel::Mcp` 渲染顶栏标题 `MCP servers`、底栏 `↑↓ select · space/↵ toggle · esc close`、右对齐状态标签 `[Connected]`/`[Disabled]`，且 `[Disabled in settings]` 条目置灰且不画 `❯` 标记。

---

## 7. 风险与回滚策略

- **风险 1**：既有测试直接向 `state.mcp_inventory` 插入计数而未填充 `state.mcp_catalog`。
  - **缓解**：`AppState::sync_lx_inventory` 在 `mcp_catalog` 无对应项目记录时回退使用 `mcp_inventory`，确保所有既有测试零改动通过。
- **风险 2**：面板打开期间输入框输入空格被当成过滤字符导致无法用空格切换。
  - **缓解**：MCP server 名称规范（`[A-Za-z0-9_-]`）严格不含空格；在 `apply_lx_input` 中针对 `SlashPanel::Mcp` 显式拦截 `LxInput::Char(' ')` 转发给 `apply_slash_enter`。
- **回滚策略**：本特性全部为内存态与 UI/事件接线改动，无数据库迁移（`SCHEMA_VERSION` 不变）与磁盘格式变更，可随时单次 commit 完整回滚。
