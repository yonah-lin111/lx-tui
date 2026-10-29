# 右键菜单行内二次确认与 Toast 样式重设计 — 代码评审报告 (Code Review)

> 评审范围：`.worktrees/menu-inline-confirm-and-toast-style`（分支 `feat/menu-inline-confirm-and-toast-style`）
>
> 检索与分析工具：`codegraph`（调用链与影响面分析）、`codebase-memory-mcp`（全量符号与引用检索）
>
> 优先级定义：
> - **P0**：功能性缺陷 / 交互竞态 Bug（直接影响用户正常点击与视觉状态一致性）
> - **P1**：副作用越界 / 遗留死代码与失真测试（影响滚动视口稳定性、架构整洁度与测试有效性）
> - **P2**：注释与文档同步（代码注释残留旧逻辑描述）

---

## P0：鼠标在确认态拓宽区域内悬停切换条目时，菜单瞬时收缩导致悬停假阳性与点击落空

### 1. 关键点
- **涉及文件**：
  - `src/event/mouse/hover.rs`（`handle_mouse_moved` L55-66）
  - `src/ui/overlay/menu.rs`（`menu_labels`）
  - `src/event/tests/overlay.rs`

### 2. 错误原因
1. 当用户首次点击需确认的菜单项（如 `Switch to ws path` 或 `Sync ws to terminal path`）进入 `menu.confirming = true` 时，文案切换为 `Confirm switch to ws path`（菜单框由 21 列拓宽至 29 列）或 `Confirm sync ws to terminal path`（由 28 列拓宽至 36 列）；若菜单靠近屏幕右缘，左边界 `x` 还会向左夹取偏移 8 列。
2. 当用户在拓宽后的菜单框内移动鼠标到其他非确认项（如 `Split right` 或 `Open prompt`），且鼠标列坐标恰好位于新拓宽出的 8 列区域内时：
   - `handle_mouse_moved` 先按 `confirming == true` 的拓宽布局 `layout` 判定 `ui::widgets::menu::item_at` 命中新条目，并调用 `update::set_menu_selection(state, index)`；
   - `set_menu_selection` 立即把 `menu.confirming` 复位为 `false` 并将 `state.menu_hover` 设为 `true`；
   - 下一帧渲染时，由于 `menu.confirming` 已为 `false`，菜单宽度瞬间缩回初始宽度，**菜单框从鼠标指针下方直接缩走**；
   - 此时鼠标指针实际已处于收缩后菜单框的**外部**，但 `state.menu_hover` 仍为 `true`（视觉上新条目错误保持悬停高亮），且用户随即左键点击时，`handle_overlay_click` 按收缩后的布局判定点击落在菜单外（`!layout.area.contains(...)`），**直接关闭了菜单而未执行目标命令**。

### 3. 修复方案与状态（已修复）
- [x] 在 `src/event/mouse/hover.rs` 处理 `Overlay::Menu` 时：调用 `update::set_menu_selection(state, index)` 后，若 `menu.confirming` 复位引起菜单布局收缩，立即基于更新后的 `menu` 重新计算 `menu_layout`；若当前鼠标坐标已不再命中收缩后的条目（`ui::widgets::menu::item_at(&new_layout, mouse.column, mouse.row).is_none()`），立即调用 `update::clear_menu_hover(state)` 清除悬停态，杜绝“指针在菜单外却高亮菜单项”的假阳性。
- [x] 在 `src/event/tests/overlay.rs` 新增针对确认态拓宽区悬停切换条目的回归测试 `hover_in_widened_confirm_region_clears_hover_when_menu_shrinks_back`。

---

## P1-1：右键菜单首次触发行内确认或键盘导航时，误触发侧栏与标签栏视口强制回滚

### 1. 关键点
- **涉及文件**：
  - `src/event/input.rs`（`Routed::Overlay` L90-105）
  - `src/event/mouse/press.rs`（`handle_mouse_down` L90-105）

### 2. 错误原因
1. `src/event/input.rs` 与 `src/event/mouse/press.rs` 在处理浮层事件前记录 `was_menu = matches!(state.overlay, Some(Overlay::Menu(_)))`，随后无论菜单是否真正执行并关闭，都调用 `ensure_workspace_visible(state, view)` 与 `reveal_active_tab(state, view)`（在 `src/event/mouse/press.rs:104` 中，`reveal_active_tab` 甚至脱离了 `if` 条件，对任意浮层点击都无条件触发）。
2. 当用户滚动左侧工作区列表或顶部标签栏到**非当前激活**的工作区/标签页，右键打开菜单并点击一次 `Close`（进入 `Confirm close` 待确认态，菜单仍保持打开）或按 `↑/↓` 键移动菜单高亮时：
   - 此时尚未发生任何工作区或标签页的关闭/切换；
   - 但 `ensure_workspace_visible` 和 `reveal_active_tab` 却被过早触发，强行把侧栏滚动位置或标签栏滚动位置拉回当前的 `active_workspace` / `active_tab`，导致用户正在二次确认的目标工作区/标签滚出视野。

### 3. 修复方案与状态（已修复）
- [x] 在 `src/event/input.rs` 与 `src/event/mouse/press.rs` 中，仅当菜单或新建/信任工作区浮层**真正完成提交并关闭**（例如 `!matches!(state.overlay, Some(Overlay::Menu(_)))` 且在键盘路径下为 `key == OverlayKey::Enter`）时，才调用 `ensure_workspace_visible(state, view)` 与 `reveal_active_tab(state, view)`。
- [x] 将 `src/event/mouse/press.rs` 中无条件执行的 `reveal_active_tab(state, view)` 移入同一守卫块内，并移除已无意义的 `was_confirm`（`ConfirmClose` 现仅用于删除 Prompt 页，不改变工作区或标签栏滚动）。
- [x] 在 `src/event/tests/overlay.rs` 新增非激活工作区右键菜单首次点击进入确认态时不回滚视口的回归测试 `first_click_on_confirm_item_does_not_yank_sidebar_scroll_to_active_workspace`。

---

## P1-2：`Overlay::ConfirmClose` 残留 5 个已废弃目标的死代码分支，且多处测试仍绕过行内确认直接构造旧弹窗

### 1. 关键点
- **涉及文件**：
  - `src/app/update/dialog.rs`（`confirm_close` L343-373）
  - `src/ui/overlay/dialog.rs`（`confirm_text`、`confirm_question` L233-306）
  - `src/ui/text.rs`（`CONFIRM_CLOSE_*`、`CONFIRM_DELETE_PROMPT_TITLE`、`CONFIRM_DELETE_FOLDER_TITLE`、`confirm_close_question`、`confirm_delete_question`、`confirm_delete_folder_question`）
  - `src/app/update/tests.rs`、`src/app/update/prompt/tests.rs`、`src/ui/overlay/tests.rs`、`src/ui/tests.rs`、`src/event/tests/focus_and_paste.rs`、`tests/prompt_management.rs`

### 2. 错误原因
1. 经过 `codebase-memory-mcp` 与 `codegraph` 全库检索确认：生产代码中构造 `Overlay::ConfirmClose` 的**唯一入口**是 `src/app/update/prompt/pages.rs:554`（`delete_current_page`，即页码菜单顶边 `[del]` 按钮删除非空页，目标固定为 `OverlayTarget::PromptPage`）。
2. 原先由右键菜单触发的 5 类目标（`Workspace`、`Tab`、`Pane`、`PromptFolder`、`PromptItem`）已全部改为 `Overlay::Menu` 行内确认，在生产环境中**永远不会**再进入 `Overlay::ConfirmClose`。
3. 然而：
   - `src/app/update/dialog.rs::confirm_close` 与 `src/ui/overlay/dialog.rs::confirm_text` / `confirm_question` 仍保留了这 5 类目标的分支处理，以及 `src/ui/text.rs` 中仅服务这些死分支的 8 个常量与函数；
   - `src/app/update/tests.rs`（9 个测试）、`src/app/update/prompt/tests.rs`（1 个测试）、`src/ui/overlay/tests.rs`（4 个测试）仍在手动构造 `Overlay::ConfirmClose(ConfirmClose { target: OverlayTarget::Workspace/Tab/Pane/PromptFolder })`，测试的是已废弃的旧弹窗路径，而非真实的行内二次确认路径；
   - 集成测试 `tests/prompt_management.rs` 中的 `run_menu_command` 只执行了一次 `update::activate_menu(state)`，未适配 `requires_confirm()` 的二段确认，且缺少对提示词/文件夹行内确认删除的端到端断言。

### 3. 修复方案与状态（已修复）
- [x] 清理 `src/app/update/dialog.rs::confirm_close`、`src/ui/overlay/dialog.rs::confirm_text` / `confirm_question` 中针对 `Workspace`、`Tab`、`Pane`、`PromptFolder`、`PromptItem` 的死分支，使 `ConfirmClose` 聚焦于唯一真实生产场景 `OverlayTarget::PromptPage`；同步移除 `src/ui/text.rs` 中不再使用的 8 个旧确认弹窗常量与函数。
- [x] 将 `src/app/update/tests.rs` 与 `src/app/update/prompt/tests.rs` 中测试工作区/标签/窗格关闭的用例全部改为走真实的右键菜单两步确认，将 `src/ui/overlay/tests.rs`、`src/ui/tests.rs`、`src/event/tests/focus_and_paste.rs` 中的 `ConfirmClose` 测试目标统一对齐为真实的 `OverlayTarget::PromptPage`。
- [x] 更新 `tests/prompt_management.rs::run_menu_command` 支持二段确认，并补充行内确认删除的端到端测试断言。

---

## P2：部分模块注释残留旧版配色与弹窗流程描述

### 1. 关键点
- **涉及文件**：
  - `src/ui/main_content/lx/toast.rs`（`border_style` 注释 L202）
  - `src/app/trust.rs`（`TrustTrigger::SyncWorkspaceCwd` 注释 L16）
  - `src/app/update/trust.rs`（`resume_trust` 注释 L21）

### 2. 错误原因
- `src/ui/main_content/lx/toast.rs` 的 `border_style` 注释仍写着 `（默认强调色 / 警告黄 / 危险红）`，与新的三档独立 256 色（薰衣草紫 / 琥珀金橙 / 珊瑚玫红）不一致；
- `src/app/trust.rs` 与 `src/app/update/trust.rs` 的注释仍写着 `信任后进入同步确认`，而实际流程已改为在右键菜单内先完成行内二次确认、信任通过后直接同步工作区路径。

### 3. 修复方案与状态（已修复）
- [x] 更新上述三处文档注释，确保注释与最新实现完全一致。
- [x] `rtk cargo fmt`、`rtk cargo clippy --all-targets -- -D warnings` 及受影响单元/集成测试全部通过。
