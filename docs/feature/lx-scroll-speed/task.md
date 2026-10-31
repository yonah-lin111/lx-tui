# lx 与全局滚动速度统一 — 执行任务清单

- 目录：`docs/feature/lx-scroll-speed/`
- 关联设计：`docs/feature/lx-scroll-speed/design.md`
- 约束：生产代码禁止 `unwrap()` / `expect()`；保持 `app/` 与 `ui/` 边界清晰；用户审核确认本文件与 `design.md` 后方可执行代码修改。

---

## 执行步骤

### 阶段 0：需求盘问与设计确认
- [x] 使用 `codegraph` 与 `codebase-memory-mcp` 检索 `lx`、`prompt` 与全局滚轮链路代码
- [x] 盘问 1（已确认）：`lx` 主聊天历史（含拖拽选区滚轮）与后台任务页 `Agents` 详情滚轮步长统一调整为 `WHEEL_LINES = 3` 行/格
- [x] 盘问 2（已确认）：全局所有可滚动容器统一使用 `WHEEL_LINES = 3` 行（项）/格，保持即时跳行（不加跨帧插值拆帧），保留 `lx` 底部 1 行呼吸位与 `RenderCache` 缓存、`prompt` 底部 `height / 4` 虚拟留白
- [x] 用户审核确认 `docs/feature/lx-scroll-speed/design.md` 与 `docs/feature/lx-scroll-speed/task.md`

---

### 阶段 1：统一 `WHEEL_LINES` 常量与视口起点计算（`src/app/`）
- [x] **任务 1.1**：在 `src/app/update/selection.rs` 中将 `WHEEL_LINES` 可见性从 `pub(crate)` 提升为 `pub const WHEEL_LINES: isize = 3;`（由 `src/app/update.rs` 的 `pub use selection::*;` 导出）
  ```rust
  pub const WHEEL_LINES: isize = 3;
  ```
- [x] **任务 1.2**：在 `src/app/update/todo.rs` 中移除重复定义的私有 `const WHEEL_LINES: usize = 3;`，改用 `super::selection::WHEEL_LINES`
  ```rust
  pub fn scroll_todo(state: &mut AppState, direction: isize) -> bool {
      state.todo.scroll_by(direction.saturating_mul(WHEEL_LINES))
  }
  ```
- [x] **任务 1.3**：在 `src/app/update/chat/ask.rs` 中移除重复定义的 `PREVIEW_STEP` / `WHEEL_STEP`，改用 `super::super::WHEEL_LINES`
- [x] **任务 1.4**：在 `src/app/prompt/mention.rs`（`MentionState::scroll_viewport`）与 `src/app/state/lx/mention.rs`（`LxMention::scroll_viewport`）中，将非 `Files` 标签页的滚轮起点统一为传入的 `base`，避免 3 行/格步长下 `ref_viewport` 超出窗口实际最大起点：
  ```rust
  let max = count.saturating_sub(1) as isize;
  let clamped = (base.min(max as usize) as isize + delta).clamp(0, max) as usize;
  let changed = clamped != base;
  self.ref_viewport = Some(clamped);
  return changed;
  ```

---

### 阶段 2：事件层全局滚轮步长统一（`src/event/mouse/wheel.rs`）
- [x] **任务 2.1**：移除 `src/event/mouse/wheel.rs` 顶部私有的 `const WHEEL_LINES: isize = 3;`，从 `crate::app::update::WHEEL_LINES` 引入，并新增 `vertical_wheel_delta(kind: MouseEventKind) -> Option<isize>`：
  ```rust
  pub(crate) fn vertical_wheel_delta(kind: MouseEventKind) -> Option<isize> {
      vertical_wheel_direction(kind).map(|direction| direction.saturating_mul(WHEEL_LINES))
  }
  ```
- [x] **任务 2.2**：更新 `wheel_lx_selection`（`lx` 拖拽选区滚轮）：
  - `update::scroll_agents_detail(state, pane, step)` 使用 `step = direction.saturating_mul(WHEEL_LINES)`
  - `update::scroll_lx_history(state, pane, step, total, visible)` 使用 `step = direction.saturating_mul(WHEEL_LINES)`
- [x] **任务 2.3**：更新 `handle_mouse_wheel` 中原 1 行/格的滚动分支，全部改为传入 `delta = direction.saturating_mul(WHEEL_LINES)`：
  - `update::scroll_referenced_search(state, delta, layout.start, layout.visible())`
  - `update::scroll_referenced_menu(state, delta, layout.start, layout.visible())`
  - `update::scroll_mention(state, delta, panel.start)`
  - `update::scroll_slash(state, delta, panel.start)`
  - `update::scroll_panel(state, delta, panel.start)`
  - `update::scroll_prompt_tree(state, direction.saturating_mul(WHEEL_LINES), visible)`
  - `update::scroll_lx_mention(state, pane, delta, bar.start)`
  - `update::scroll_lx_refer(state, pane, delta, bar.start)`
  - `update::scroll_agents_detail(state, pane, delta)`
  - `update::scroll_lx_history(state, pane, delta, total, visible)`
  - `update::scroll_lx_slash(state, pane, delta, bar.start)`
  - `update::scroll_workspace_list(state, delta, rows)`
  - `update::scroll_agent_list(state, delta, rows)`

---

### 阶段 3：测试与文档同步
- [x] **任务 3.1**：在 `src/event/tests/lx_view.rs` 新增 `lx_history_wheel_scrolls_by_wheel_lines_and_clamps` 测试，验证 `lx` 主聊天历史滚轮每格移动 3 行及边界钳制、跟随状态切换；在 `src/app/state/lx/mention/tests.rs` 补充非 `Files` 标签页滚轮从 `base` 偏移的断言
- [x] **任务 3.2**：同步更新 `src/event/tests/` 下断言单格滚轮位移的既有测试：
  - `src/event/tests/lx_view.rs`：`lx_mention_wheel_scrolls_panel_not_input`、`lx_history_selection_waits_for_copy_shortcut_and_follows_wheel_scroll`、`lx_task_output_and_agents_detail_selection_scroll_and_copy`
  - `src/event/tests/mouse_click.rs`：`wheel_over_workspace_list_rescrolls_hover`、`wheel_over_agents_section_scrolls_list`
  - `src/event/tests/prompt_menu.rs`：`mouse_hover_over_referenced_menu_after_wheel_keeps_viewport`、`mouse_wheel_over_referenced_menu_scrolls_viewport_not_highlight`
  - `src/event/tests/prompt_tree.rs`：`mouse_wheel_and_hover_and_click_on_block_panel`、`mouse_wheel_over_mention_panel_scrolls_viewport_only`
- [x] **任务 3.3**：同步更新 `docs/standards/tui-design-requirements.md` 中侧栏工作区列表的滚轮步长说明（每格 3 项）

---

### 阶段 4：代码校验
- [x] **任务 4.1**：运行 `cargo fmt`
- [x] **任务 4.2**：运行 `cargo clippy --all-targets -- -D warnings`
- [x] **任务 4.3**：运行受影响模块的精确单元测试（`cargo test --lib event::tests`、`cargo test --lib app::update`、`cargo test --lib app::state::lx::mention`）
- [ ] **任务 4.4**：由用户运行 `cargo run` 实际确认 TUI 滚动手感
