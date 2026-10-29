# 右键菜单行内二次确认与 Toast 样式重设计 — 执行任务清单

- **任务目录**：`docs/feature/menu-inline-confirm-and-toast-style/`
- **设计文档**：`docs/feature/menu-inline-confirm-and-toast-style/design.md`
- **参考项目**：`@[refer-folder](/Users/yonah/projects/agent/lx-agent)`
- **工程约束**：严格遵守 `AGENTS.md` 及 `docs/standards/*` 规范；生产代码禁止 `unwrap()` / `expect()`；所有文案集中于 `src/ui/text.rs`；所有样式集中于 `src/ui/style.rs`；测试文件严格跟随被测模块目录结构。

---

## 阶段一：需求质询与方案设计（已完成）

- [x] 使用 `codegraph` 与 `codebase-memory-mcp` 检索 `lx-tui` 的右键菜单、确认浮层、Toast 渲染及 `lx-agent` 的内联确认实现
- [x] **Grill Me 问题 1**：确认右键菜单行内二次确认的命令范围（覆盖 `Overlay::Menu` 中全部 7 个需确认命令，移除 `ConfirmSwitchCwd` 与 `ConfirmSyncWorkspaceCwd` 弹窗）
- [x] **Grill Me 问题 2**：确认菜单项进入待确认态时的文案（`Confirm ...`）、红底白字加粗样式（`style::menu_confirm()`）及菜单宽度自适应不截断规则
- [x] **Grill Me 问题 3**：确认待确认态在焦点移动（`↑/↓` 或鼠标悬停至其他菜单项）时自动复位、鼠标移出菜单外保持、以及 `Sync ws to terminal path` 二次确认后衔接信任检查直接同步的流程
- [x] **Grill Me 问题 4**：确认移除全局与 lx 窗格内 Toast 的背景色，并采用全新三档专属边框/标题/`[✕]` 配色（`Default` 薰衣草紫 `Indexed(141)`、`Warning` 琥珀橙 `Indexed(214)`、`Danger` 珊瑚玫红 `Indexed(204)`）
- [x] 编写详细设计文档 `design.md` 与任务拆分文档 `task.md`

---

## 阶段二：右键菜单行内二次确认实现（已完成）

### 2.1 状态与行为层（`src/app/`）
- [x] **`src/app/overlay.rs`**：
  - 在 `Menu` 结构体中新增 `pub confirming: bool` 字段
  - 为 `MenuCommand` 实现 `pub fn requires_confirm(self) -> bool`（涵盖 `CloseWorkspace`、`CloseTab`、`ClosePane`、`DeleteFolder`、`DeletePrompt`、`SwitchToWorkspaceCwd`、`SyncWorkspaceToTerminalCwd`）
  - 从 `OverlayKind` 与 `Overlay` 中移除已废弃的 `ConfirmSwitchCwd` 与 `ConfirmSyncWorkspaceCwd` 变体及结构体（保留 `ConfirmClose` 供页码菜单非空页删除使用）
- [x] **`src/app/update/menu.rs` & `src/app/update/prompt.rs`**：
  - 更新所有 `Overlay::Menu(Menu { ... })` 构造处，初始化 `confirming: false`
  - 在 `move_menu_selection` 与 `set_menu_selection` 中：当选中索引改变时重置 `menu.confirming = false`
  - 改造 `activate_menu(state: &mut AppState) -> Option<(PaneId, Vec<u8>)>`：
    - 首次触发 `requires_confirm()` 命令时，设 `menu.confirming = true`、`state.menu_hover = true` 并保持菜单打开
    - 第二次触发时关闭菜单，直接调用 `close_workspace` / `close_tab` / `close_pane` / `delete_prompt_folder` / `delete_prompt_item` / `switch_to_workspace_cwd` / `Effect::CheckTrust`
- [x] **`src/app/update/workspace.rs` & `src/app/update/trust.rs`**：
  - 将 `open_confirm_switch_cwd` + `confirm_switch_cwd` 合并为直接执行的 `switch_to_workspace_cwd`
  - 将 `open_confirm_sync_workspace_cwd` + `confirm_sync_workspace_cwd` 合并为直接执行的 `sync_workspace_cwd`
  - 在 `resume_trust` 的 `TrustTrigger::SyncWorkspaceCwd` 分支中直接调用 `super::sync_workspace_cwd(state, workspace, pane)`
- [x] **`src/app/update/dialog.rs` & `src/event/overlay.rs` & `src/input/mod.rs`**：
  - `apply_overlay_key` 中 `OverlayKind::Menu` 透传 `activate_menu(state)` 的返回值
  - `handle_overlay_click` 中 `Some(Overlay::Menu(menu))` 命中条目后返回 `update::activate_menu(state)`
  - 清理 `ConfirmSwitchCwd` 与 `ConfirmSyncWorkspaceCwd` 的按键路由与点击处理分支

### 2.2 渲染与样式层（`src/ui/`）
- [x] **`src/ui/text.rs`**：
  - 新增 `MENU_CONFIRM_CLOSE`（`"Confirm close"`）、`MENU_CONFIRM_DELETE`（`"Confirm delete"`）、`MENU_CONFIRM_SWITCH_TO_WORKSPACE_CWD`（`"Confirm switch to ws path"`）、`MENU_CONFIRM_SYNC_WS_TO_TERMINAL_CWD`（`"Confirm sync ws to terminal path"`）
  - 清理不再使用的 `CONFIRM_SWITCH_CWD_TITLE`、`CONFIRM_SYNC_WS_CWD_TITLE` 及对应问题生成函数
- [x] **`src/ui/style.rs`**：
  - 新增 `pub fn menu_confirm() -> Style`（`fg: Indexed(255)`、`bg: Indexed(160)`、`BOLD`）
- [x] **`src/ui/overlay/menu.rs`、`src/ui/widgets/menu.rs`、`src/ui/overlay.rs`、`src/ui/overlay/dialog.rs`**：
  - `menu_labels(menu: &Menu)` 根据 `menu.confirming && menu.selected == index` 动态返回确认文案，`menu_layout` 据此自适应计算菜单宽度，保证确认文案完整显示不截断
  - `widgets::menu::render_with_confirm` 支持传入 `confirming: Option<usize>`，为待确认项渲染 `style::menu_confirm()`
  - 清理 `src/ui/overlay/dialog.rs` 与 `src/ui/overlay.rs` 中已废弃的 `render_confirm_switch_cwd`、`render_confirm_sync_workspace_cwd` 及其 shell 函数

---

## 阶段三：Toast 移除背景色与全新边框色系实现（已完成）

- [x] **`src/ui/style.rs`**：
  - 新增 `pub fn toast_border(kind: ToastKind) -> Style`：
    - `ToastKind::Default` → `Color::Indexed(141)`（薰衣草紫）
    - `ToastKind::Warning` → `Color::Indexed(214)`（琥珀金橙）
    - `ToastKind::Danger` → `Color::Indexed(204)`（珊瑚玫红）
- [x] **`src/ui/toast.rs`**：
  - `render` 中移除 `.style(style::overlay_panel())`，保留 `clear::render(frame, area)` 清底，使 Toast 背景保持终端默认透明底色
  - `border_style` 与 `title_style` 统一走 `style::toast_border(kind)`
  - 右上角 `[✕]` 关闭按钮样式改为 `border_style(toast.kind)`（与边框同色，不再使用 `style::accent()` 青色）
- [x] **`src/ui/main_content/lx/toast.rs`**：
  - `draw` 中移除 `style::overlay_panel()` 的双层循环底色填充，仅保留 `clear::render_buffer(buf, box_area)` 清底
  - 右上角 `[✕]` 关闭按钮样式改为 `border_style(toast.kind)`

---

## 阶段四：测试更新、文档同步与精准校验（已完成）

- [x] 更新并补充单元测试：
  - `src/app/overlay/tests.rs`（`menu_command_requires_confirm_for_destructive_and_cwd_actions`）
  - `src/app/update/tests.rs` & `src/app/update/prompt/tests.rs` & `src/app/update/trust/tests.rs`（右键菜单两步确认、切换高亮重置确认态、路径同步信任续作）
  - `src/ui/overlay/tests.rs`（`confirming_menu_item_renders_confirm_label_widens_box_and_uses_danger_style`）
  - `src/event/tests/overlay.rs`（`switch_cwd_menu_item_first_click_arms_confirm_second_click_returns_cd_command`、`mouse_click_close_tab_menu_item_twice_closes_tab_inline`）
  - `src/ui/toast/tests.rs`、`src/ui/main_content/lx/toast/tests.rs`、`src/ui/style/tests.rs`、`src/ui/tests.rs`（断言 Toast 无背景色 `Color::Reset`、三档边框/标题/`[✕]` 为 `Indexed(141/214/204)` 且不与主色调 `Cyan` 重叠）
- [x] 同步更新 `docs/standards/tui-design-requirements.md` 中关于右键菜单二次确认与 Toast 背景/边框配色的规范描述
- [x] 执行代码格式化与静态检查：
  - `rtk cargo fmt --check` 通过
  - `rtk cargo clippy --all-targets -- -D warnings` 0 warnings 通过
  - 针对受影响模块执行精准测试校验全部通过

---

## 阶段五：Code Review 审查、P0–P2 修复与合并清理（已完成）

- [x] 结合 `codegraph` 与 `codebase-memory-mcp` 进行全量审查并输出 `docs/feature/menu-inline-confirm-and-toast-style/review.md`
- [x] **P0 修复**（`src/event/mouse/hover.rs`）：修复鼠标在确认态拓宽区域内悬停切换条目时，因菜单宽度收缩导致指针落在收缩后菜单外却残留 `menu_hover = true` 假高亮及点击落空的竞态问题，并新增回归测试 `hover_in_widened_confirm_region_clears_hover_when_menu_shrinks_back`
- [x] **P1-1 修复**（`src/event/input.rs`、`src/event/mouse/press.rs`）：收紧 `ensure_workspace_visible` 与 `reveal_active_tab` 触发守卫，仅在右键菜单命令真正完成二次确认提交并关闭菜单时触发，避免首次点击进入确认态或 `↑/↓` 导航时强行回滚侧栏/标签栏视口，并新增回归测试 `first_click_on_confirm_item_does_not_yank_sidebar_scroll_to_active_workspace`
- [x] **P1-2 修复**（`src/app/update/dialog.rs`、`src/ui/overlay/dialog.rs`、`src/ui/text.rs`、测试集）：清理 `ConfirmClose` 中已迁移至右键菜单行内确认的 5 类死分支及 8 个废弃文案常量/函数，将全部相关单测与 `tests/prompt_management.rs` 集成测试对齐至真实行内二次确认路径
- [x] **P2 修复**（`src/ui/main_content/lx/toast.rs`、`src/app/trust.rs`、`src/app/update/trust.rs`）：同步更新残留的旧配色与旧弹窗流程文档注释
- [x] 合并分支 `feat/menu-inline-confirm-and-toast-style` 至主工作区 `dev`，清理 `.worktrees/menu-inline-confirm-and-toast-style` 工作区、分支及隔离数据库
