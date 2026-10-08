# 工作区固定路径与右键菜单增强 — 执行任务

- 分支：`feature/workspace-context-menu`（已合并进 `dev`，合并提交 `6838c72`）
- 规则：生产代码禁止 `unwrap()/expect()`；用户可见文案收敛至 `ui/text.rs`；样式统一从 `ui/style.rs` 取；测试驱动，按受影响范围严格验证。

## 执行清单

### 1. 文档与需求对齐（Grill Me 阶段）

- [x] 对齐并确认主内容窗格切换路径的实现机制（向 PTY 写入 `cd "<path>"\n`）及二次确认交互
- [x] 对齐并确认工作区新建终端的承载形式（在目标工作区追加独立 Tab，初始 cwd 为固定根路径，直接进入 `PaneView::Terminal`）
- [x] 对齐并确认菜单文案（统一英文风格，缩写为 ws：`New terminal`、`Switch to ws path`）
- [x] 对齐并确认菜单完整显示机制（使用 `UnicodeWidthStr` 自适应撑开，杜绝省略）
- [x] 对齐并确认右键打开 Prompt 联动（`Open prompt`，展开并聚焦，左栏联动工作区，主内容精确对齐窗格 cwd）
- [x] 完成 `design.md` 架构图与完整决策表编写

### 2. 状态层与命令模型（`src/app/`）

- [x] 在 `src/app/overlay.rs` 中扩展 `MenuCommand`（新增 `NewTerminal`、`OpenPrompt`、`SwitchToWorkspaceCwd`）
- [x] 在 `src/app/overlay.rs` 中增加 `ConfirmSwitchCwd` 结构与 `Overlay::ConfirmSwitchCwd` 变体
- [x] 在 `src/app/state.rs` 中增加 Prompt 独立绑定路径支持（如 `prompt_root: Option<PathBuf>`）
- [x] 在 `src/app/update.rs` 中解除工作区 cwd 随终端切换的自动覆盖行为，使工作区路径不可变
- [x] 在 `src/app/update.rs` 中扩展 `open_workspace_menu` 与 `open_pane_menu` 填充新命令
- [x] 在 `src/app/update.rs` 中实现 `create_terminal_in_workspace`、`open_prompt_for_workspace`、`open_prompt_for_pane` 与 `confirm_switch_cwd`
- [x] 状态单元测试：`src/app/state/tests.rs`、`src/app/update/tests.rs`

### 3. 事件与 PTY 交互层（`src/event/`）

- [x] 调整 `src/event/mod.rs` 中的 `poll_process_cwds`，移除对工作区 cwd 的更新循环
- [x] 在 `src/event/mod.rs` 的浮层按键处理与点击处理中，捕获切换路径确认动作并调用 `write_to_pane` 发送 `cd` 命令
- [x] 确保目标工作区右键新建 Tab 后，`reconcile_sessions` 正常为该新窗格派生 PTY 并应用目标工作区固定 cwd
- [x] 事件单元测试：`src/event/tests.rs`

### 4. 渲染与组件层（`src/ui/`）

- [x] 改造 `src/ui/widgets/menu.rs`：使用 `unicode_width::UnicodeWidthStr::width` 计算最宽项与标题视觉列宽，消除不必要省略
- [x] 改造 `src/ui/overlay.rs`：新增 `ConfirmSwitchCwd` 弹窗渲染与按钮命中
- [x] 改造 `src/ui/mod.rs`：Prompt 顶栏路径名展示适配独立的 `prompt_root`
- [x] 完善 `src/ui/text.rs`：集中定义全部新菜单项、弹窗标题与提问文案
- [x] 渲染与文案单元测试：`src/ui/widgets/menu/tests.rs`、`src/ui/text/tests.rs`、`src/ui/overlay/tests.rs`

### 5. 校验与验证

- [x] 精确运行受影响模块单元测试：`cargo test app::`、`cargo test ui::`、`cargo test event::`
- [x] `cargo fmt -- --check`
- [x] `cargo clippy --all-targets -- -D warnings`
