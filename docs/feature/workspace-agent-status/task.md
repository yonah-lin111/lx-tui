# 当前工作区 Agent 探测与状态展示 — 执行任务

- 任务目录：`docs/feature/workspace-agent-status/`
- 规则：严格遵守 `AGENTS.md`；禁止 `unwrap()` / `expect()`；所有用户可见文案收敛至 `ui/text.rs`；样式统一从 `ui/style.rs` 取；测试驱动，按受影响范围严格验证。

## 执行清单

### 1. 需求与边界对齐（Grill Me 阶段）
- [x] 质问 1：确认 Agent 作用域（严格限定当前激活工作区）
- [x] 质问 2：确认探测引擎选型（双重裁决：进程树定身份 + 尾部屏幕与活性定状态）
- [x] 质问 3：确认 UI 条目展现形式与交互行为（状态/名称/位置卡片，点击一键切Tab聚焦目标窗格）
- [x] 完成 `design.md` 架构与数据流定稿
- [x] 用户审核确认设计，授权进入实施

### 2. 平台与探测层（`src/platform/`、`src/detect/`）
- [x] `src/platform/mod.rs`：声明 `foreground_process_name(child_pid: u32) -> Option<String>`
- [x] `src/platform/macos.rs`：实现 macOS 下前台进程组主进程获取（`proc_listpids` + `proc_pidinfo`）
- [x] `src/platform/linux.rs` / fallback：实现 Linux `/proc` 读取与平台降级桩
- [x] `src/detect/mod.rs`：创建独立探测模块，定义 `AgentKind`、`AgentState`、`PaneAgentSnapshot`
- [x] `src/detect/mod.rs`：实现已知 Agent 进程名匹配器与终端尾部状态模式匹配（Idle/Working/Blocked）
- [x] 探测层单元测试：进程名匹配测试、屏幕尾部行状态匹配测试（无终端依赖）

### 3. 状态层（`src/app/`）
- [x] `src/app/state.rs`：`Pane` 结构体增加 `agent: Option<PaneAgentSnapshot>` 字段
- [x] `src/app/state.rs`：`AppState` 增加 `agent_hover: Option<usize>`、`agent_scroll: usize`、`agent_scroll_drag: Option<u16>`
- [x] `src/app/update.rs`：实现 `update_pane_agent` 纯更新函数
- [x] `src/app/update.rs`：实现 `focus_agent_pane`（切 Tab、聚焦窗格、Lx 转终端视图、收回 prompt 焦点）
- [x] 状态层单元测试：Agent 状态挂载测试、跨 Tab 聚焦跳转测试

### 4. 事件调度与去抖（`src/event/`）
- [x] `src/event/mod.rs`：新增 `poll_active_agents(state, sessions)`，仅扫描当前激活工作区内各窗格
- [x] `src/event/mod.rs`：接入 PTY 输出后的去抖调度（`agent_check`，合并高频扫描）
- [x] 状态变更时触发 `dirty = true` 驱动重绘

### 5. 渲染与交互（`src/ui/`）
- [x] `src/ui/text.rs`：新增 Agent 相关文案（空状态文案、状态标签等）
- [x] `src/ui/sidebar.rs`（或拆分 `src/ui/sidebar/agents.rs`）：实现 `render_agents_section`
- [x] 条目排版：状态圆点着色、Agent 名称亮显、所在 Tab 标识弱化显示、hover 整行高亮反显
- [x] 滚动支持：内容溢出时渲染右侧滚动条（复用 `widgets::scrollbar`）与滚轮事件支持
- [x] 交互事件：左键点击命中 `agent_item_at` 触发 `update::focus_agent_pane`
- [x] 渲染层单元测试：列表排版计算、点击坐标命中测试

### 6. 验证与回归
- [x] `cargo fmt`
- [x] `cargo clippy --all-targets -- -D warnings`
- [x] `cargo test` 精确与全量测试全绿

## 完成记录

- 实施分支：`feat/workspace-agent-status`（工作区 `.worktrees/agents`），已合并至 `dev`（merge `45f3681`）；工作区与分支已移除。实施差异见 `design.md` 第 6 节。
- 验证：`cargo fmt`、`cargo clippy --all-targets -- -D warnings`、`cargo test` 全绿（810 lib + 30 集成，含合并 dev 上"移除 lx 页面动画"后的回归）。
- 合并前迭代补充：窗格顶边框 `agent:xxx` 标记（聚焦值与 prompt `ws:` 值同款样式）、Agents 条目行首 `▸` 选中标记（不额外着色）、归属标签简化为 `[tab N]`。
