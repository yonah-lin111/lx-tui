# 当前工作区 Agent 探测与状态展示 — 设计

- 任务目录：`docs/feature/workspace-agent-status/`
- 状态：**已实施并合并至 dev（merge `45f3681`）；实施差异见第 6 节**
- 参考实现：`herdr-master`（`src/detect/`、`src/platform/`、`src/ui/sidebar.rs`）

## 1. 背景与目标

1. **现状**：lx-tui 侧栏已预留 agents 分区表头（`layout.rs` 中的 `SidebarSections.agents`，中线分割线与折叠按钮），但分区内容当前为空占位（`render_sidebar` 注释："agents 分区暂无内容，保持空占位"），未进行任何 Agent 进程检测或状态追踪。
2. **目标**：参考 `herdr-master`，实现 Agent 识别与生命周期状态检测能力，在左侧 Agents 区域中实时展示当前激活工作区内各窗格运行的 Agent（如 Claude Code, Codex, Antigravity, OpenCode 等）及其运行状态（Idle / Working / Blocked 等），点击条目可切换 Tab 并聚焦跳转到对应窗格。

## 2. 已确认决策（Grill Me 结论）

| # | 决策项 | 结论 | 详细说明 |
|---|--------|------|----------|
| 1 | **Agent 展示作用域 (Scope)** | **当前激活工作区隔离 (A)** | 严格限定展示当前激活工作区内所有 Tab/窗格的 Agent。切换工作区时列表随之切换，与 `prompt-per-workspace` 上下文隔离心智高度一致。 |
| 2 | **Agent 探测方式 (Detection Engine)** | **双重裁决机制 (C)** | ① 前台进程树探测确定 `AgentKind`（杜绝普通 shell 文本误报）；② 结合 PTY 活性与终端屏幕尾部文本模式匹配确定 `AgentState`（Idle / Working / Blocked）。 |
| 3 | **UI 条目展示与交互 (UI & Interaction)** | **语义化卡片 + 导航跳转 (B)** | 展示状态圆点（Working 绿、Blocked 黄/红、Idle 灰）+ Agent 名称 + 所在 Tab/Pane 标签；鼠标悬停高亮反显；点击直达对应 Tab 并聚焦该窗格（若在 Lx 视图自动切回终端视图）；支持滚动条。 |

## 3. 架构与数据结构设计（Data Structures First）

### 3.1 探测模型（`src/detect/mod.rs`）

```rust
/// 支持识别的 Agent 类型
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AgentKind {
    Claude,
    Codex,
    Gemini,
    Antigravity,
    OpenCode,
    Cursor,
    Pi,
    Kimi,
}

impl AgentKind {
    pub fn display_name(self) -> &'static str {
        match self {
            Self::Claude => "claude",
            Self::Codex => "codex",
            Self::Gemini => "gemini",
            Self::Antigravity => "agy",
            Self::OpenCode => "opencode",
            Self::Cursor => "cursor",
            Self::Pi => "pi",
            Self::Kimi => "kimi",
        }
    }
}

/// Agent 运行状态
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentState {
    /// 空闲就绪（提示符可见，等待用户输入）
    Idle,
    /// 运行中（模型生成中、执行工具或命令中）
    Working,
    /// 阻塞卡点（等待用户确认权限、工具调用或回答选项）
    Blocked,
    /// 普通 Shell / 未识别程序
    Unknown,
}

/// 窗格内的 Agent 状态快照
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PaneAgentSnapshot {
    pub kind: AgentKind,
    pub state: AgentState,
    pub title: Option<String>,
}
```

### 3.2 平台前台进程探测（`src/platform/`）

在 `src/platform/mod.rs` 增加对 PTY 子进程前台进程名称/命令行的探测接口：
- `macos.rs`：利用 `proc_listpids(PROC_PGRP_ONLY, ...)` 与 `proc_pidinfo(PROC_PIDTBSDINFO)` 读取前台进程组主进程名称（`comm` / `argv0`）。
- `linux`：读取 `/proc/<pid>/stat` 提取前台进程组 ID `tpgid`，匹配获取其 `comm`。
- `windows` / fallback：无前台进程探测能力时平滑降级（返回 None，不崩溃）。

### 3.3 应用状态挂载（`src/app/`）

遵循纯数据、无 IO、可测试原则：

```rust
// 挂载在 Pane 结构体（src/app/state.rs）
pub struct Pane {
    pub kind: PaneKind,
    pub view: PaneView,
    pub terminal: Terminal,
    pub exited: bool,
    pub cwd_label: Option<String>,
    pub cwd: Option<PathBuf>,
    /// 窗格当前探测到的 Agent 状态（None 表示未启动 Agent 或纯 Shell）
    pub agent: Option<PaneAgentSnapshot>,
}

// AppState 补充侧栏 Agents 分区交互状态（src/app/state.rs）
pub struct AppState {
    // ... 已有字段 ...
    pub agents_collapsed: bool,
    /// 鼠标悬停的 Agent 条目索引
    pub agent_hover: Option<usize>,
    /// Agent 列表滚动偏移
    pub agent_scroll: usize,
    /// 拖拽 agent 滚动条 thumb 抓取偏移
    pub agent_scroll_drag: Option<u16>,
}
```

### 3.4 状态转换与动作（`src/app/update.rs`）

- `update_pane_agent(state: &mut AppState, pane_id: PaneId, snapshot: Option<PaneAgentSnapshot>) -> bool`：纯函数，更新指定窗格的 Agent 快照，若有状态变动返回 `true`。
- `focus_agent_pane(state: &mut AppState, pane_id: PaneId) -> bool`：
  1. 在当前激活工作区中查找该 `pane_id` 所在的 Tab 索引；
  2. 切换 `active_tab` 为该 Tab；
  3. 调用 `layout.focus_pane(pane_id)`；
  4. 若该窗格的视图处于 `PaneView::Lx`，自动切换为 `PaneView::Terminal`；
  5. 收回 prompt 焦点（`prompt_focused = false`）。

### 3.5 事件调度（`src/event/mod.rs`）

- 复用现有 `cwd_check` 类似的去抖逻辑：在 `PtyEvent::Output` 到来后安排去抖扫描（300ms），空闲时低频或事件驱动。
- 在 `poll_active_agents(state, sessions)` 中：
  1. 遍历当前激活工作区中的全部终端窗格；
  2. 通过 `PtySession::process_id` 获取子进程 PID，经 `platform` 查询其前台进程；
  3. 经 `detect::identify_agent` 匹配 Agent 身份；
  4. 若匹配成功，提取终端缓冲区尾部若干行（`terminal.renderable_cells()` 或尾部行切片），调用 `detect::arbitrate_state` 判断 `Idle` / `Working` / `Blocked`；
  5. 调用 `update::update_pane_agent`，若有变更置 `dirty = true` 触发精确重绘。

### 3.6 渲染层（`src/ui/`）

- `src/ui/layout.rs`：`SidebarSections.agents` 矩形保持不变（已支持折叠与中线分割）。
- `src/ui/sidebar.rs`（或拆分 `src/ui/sidebar/agents.rs`）：
  - 收集当前工作区所有带 `Some(agent)` 的窗格列表项，结构为：
    ```rust
    pub struct AgentListItem {
        pub tab_idx: usize,
        pub tab_name: String,
        pub pane_id: PaneId,
        pub snapshot: PaneAgentSnapshot,
    }
    ```
  - 条目渲染（最终实现）：
    - 行首选中标记：当前聚焦窗格（prompt 未持有键盘焦点）对应条目显示 `▸`（默认前景加粗，不额外着色），其余条目保留空列保持对齐；
    - 状态点：Working 绿色加粗 `●`，Blocked 黄色加粗 `●`，Idle 暗灰 `○`；
    - Agent 标识：`claude`, `codex`, `agy` 等（加粗亮显）；
    - 窗格归属：固定右对齐 `[tab N]`（N 为标签位置，1 基），宽度不足先截断 Agent 名称，归属标签不随宽度缩小隐藏；
    - 鼠标悬停行：使用终端原生高亮 `style::selection` 整行背景（不采用聚焦整行底色）。
  - 溢出时右侧 1 列绘制滚动条（复用 `widgets::scrollbar`）。
  - 窗格顶边框：探测到 Agent 的终端窗格在 `[lx]`/`[>_]` 按钮左侧间隔 1 列绘制 ` agent:xxx`（前缀 accent、值 muted）；窗格聚焦时值样式与 prompt 的 `ws:` 值一致（Cyan + BOLD + DIM），失焦回退 muted；空间不足或会覆盖左侧标题时不绘制。
- `src/ui/text.rs`：收敛全部 Agent 状态与空状态文案（`AGENTS_EMPTY = "no agents running"`、`AGENT_SELECTED_MARKER = "▸"`、`PANE_AGENT_PREFIX = "agent:"`）。

## 4. 风险与边界

1. **跨平台进程探测边界**：
   - macOS 必须在 `src/platform/macos.rs` 使用系统 libc 接口，禁止使用外挂命令（如 `ps`）；
   - Linux 读取 `/proc` 必须健壮处理进程短命退出的 IO 错误；
   - 核心模块严禁出现 `#[cfg(target_os)]`。
2. **事件循环性能无阻塞**：
   - 进程扫描仅针对当前激活工作区内的窗格（通常仅 1~4 个），单次耗时控制在 1ms 内；
   - PTY 高频输出期间做 300ms 去抖合并，绝不在每次 `read` 中调用进程扫描。
3. **状态去抖（Anti-Flicker）**：
   - 参考 `herdr`，从 Working 到 Idle 设置短暂的确认窗口（100ms~200ms），防止输出间歇性停顿导致状态圆点疯狂闪烁。
4. **无终端测试保证**：
   - `detect` 模块纯算法函数，使用固定文本切片进行单元测试覆盖；
   - `app` 状态层在无 PTY、无真实进程环境下具备 100% 独立可测性。

## 5. 验收标准

1. **探测精度**：在当前工作区终端中启动 `claude` / `codex` / `antigravity` 等，左侧 Agents 分区在 500ms 内展示对应 Agent 条目。
2. **状态流转**：
   - 模型生成或命令运行中显示 `Working`；
   - 出现询问确认（例如 `y/n` 或工具执行权限）显示 `Blocked`；
   - 输出结束回到提示符显示 `Idle`；
   - 退出 Agent 进程后条目自动移除，空列表展示 `no agents running`。
3. **交互跳转**：
   - 鼠标点击 Agent 项立即切换至所在 Tab 并聚焦到该终端窗格；
   - 若窗格处于 Lx 欢迎页，自动切回终端；
   - 悬停反显与滚动条交互平滑自然。
4. **工程质量**：
   - `cargo fmt`
   - `cargo clippy --all-targets -- -D warnings`
   - `cargo test` 全绿无告警。

## 6. 实施记录（与初稿差异）

- 实施分支 `feat/workspace-agent-status`（工作区 `.worktrees/agents`）已完成并合并至 `dev`（merge `45f3681`），任务清单见 `task.md`。
- 与初稿的差异：
  1. `AgentKind::display_name` 未保留在探测层，Agent 标签收敛到 `ui/text.rs::agent_label`（遵守"用户可见文案集中"规范）。
  2. 平台实现内联于 `src/platform/mod.rs`（macOS `mod macos`、Linux `mod linux`），沿用该文件既有结构，未新建 `macos.rs`/`linux.rs`。
  3. 平台接口简化为 `foreground_process_name(child_pid) -> Option<String>`（只取前台进程组组长，不枚举整组进程）；macOS 在 `proc_pidinfo` 基础上增加 `KERN_PROCARGS2` argv 解析与脚本参数回退，覆盖 `node .../codex.js`、`exec -a codex` 等展示名与可执行名不一致的情况。
  4. 侧栏渲染从 `ui/mod.rs` 拆分到 `src/ui/sidebar.rs`（`ui/mod.rs` 回落至 1000 行以内）。
  5. 条目归属标签最终为 `[tab N]`（不显示窗格标识）；当前聚焦窗格改用行首 `▸` 符号标记，未采用整行选中底色。
  6. Working→Idle 的 100~200ms 防闪烁确认窗口未实现，依赖 PTY 输出后的 300ms 去抖扫描。
  7. 状态仲裁为简化模式匹配（每类 Agent 少量关键提示串 + 转轮符形态），未引入 herdr 的 manifest 规则引擎。
