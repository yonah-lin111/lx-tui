# 全局 `Esc` 取消文字选中与优先级开发任务

状态：已完成实现、Review（P0–P3）修复、合并入 `dev` 主分支并移除 `.worktrees/esc-clear-selection` 工作区与 `feature/esc-clear-selection` 分支（2026-11-06）。
对应设计：`docs/feature/esc-clear-selection/design.md`。
对应评审：`docs/feature/esc-clear-selection/review.md`。

---

## 阶段 1：全局选区查询、清理与单选区互斥基线（`src/app/overlay.rs`、`src/app/state/todo.rs`、`src/app/update/selection.rs`）

- [x] 1.1 扩展 `Overlay`、`TodoPanel`、`Terminal` 与 `AskState` 选区访问辅助方法
  - [x] 在 `src/app/overlay.rs` 的 `Overlay` 上新增 `has_selection(&self) -> bool` 与 `clear_selection(&mut self)`（覆盖 `Rename`、`NewWorkspace`、`WorktreeOpen`、`NewNode`、`SaveDraft`、`ReferencedFolderInput`、`ReferencedFolderSearch`、`ResumeSessionEdit`、`Connect`）
  - [x] 在 `src/app/state/todo.rs` 的 `TodoPanel` 上新增只读方法 `edit_input(&self) -> Option<&TextInput>` 与 `clear_selection(&mut self)`
  - [x] 在 `src/terminal/mod.rs` 的 `Terminal` 上新增零分配同源方法 `has_selection(&self) -> bool`（基于 `to_range(&self.term)`）
  - [x] 在 `src/app/state/lx/ask.rs` 的 `AskState` 上新增 `clear_all_text_selections(&mut self)`（覆盖多问题全部 `AskEditor` 槽位）
- [x] 1.2 补全 `stop_active_selection_drags` 与 `clear_selection`（`src/app/update/selection.rs`）
  - [x] 抽取 `stop_active_selection_drags(state: &mut AppState)`：终止 `selection_autoscroll`、`lx_card_press`、全部 `*_drag` 标记与空选区锚点，保留已成立静态选区
  - [x] 补齐 `clear_selection(state)` 清理 `state.overlay`、`prompt_panel.search` / `pages_view.search`、`workspace_history.search`、`ask.clear_all_text_selections()` 及各窗格 `pane.lx.task` 内联追问框 `agent_reply` 的选区
- [x] 1.3 实现 `has_visible_selection(state: &AppState) -> bool` 与 `escape_clear_selection(state: &mut AppState) -> bool`（`src/app/update/selection.rs` & `src/app/update.rs`）
  - [x] 实现模态浮层打开时仅检测浮层自身输入框选区的遮挡隔离规则
  - [x] 实现无浮层时按容器可见性与 UI 渲染同源范围（`prompt_selection_range`、`terminal_has_selection` 等）检测可见选区
  - [x] 实现 `escape_clear_selection(state)`：重置 `selection_click` 并调用 `stop_active_selection_drags(state)`；有可见选区时清除对应选区并返回 `true`（消费 `Esc`）；无可见选区时保留后台 Tab / 被遮挡容器的静态选区并返回 `false`

---

## 阶段 2：接入最高优先级 `Esc` 拦截、追问框选区闭环与单选区互斥（`src/event/`、`src/app/update/`、`src/ui/`）

- [x] 2.1 在事件入口接入最高优先级裸 `Esc` 拦截（`src/event/input.rs`）
  - [x] 在 `handle_key_event` 顶部（`KeyEventKind::Release` 之后，早于侧栏历史拦截、`input::route`、Prompt recent 抽屉与终端 `Routed::Pane` 写入）拦截裸 `Esc`（`key.code == KeyCode::Esc && key.modifiers.is_empty()`），调用 `update::escape_clear_selection(state)`，返回 `true` 时置 `*dirty = true` 并立即返回
- [x] 2.2 同步 Update 层直接调用入口并修复 `Prompt` 展开粘贴芯片分步 `Esc`（`src/app/update/`）
  - [x] `src/app/update/editor.rs`：`apply_editor` 在收到 `EditorCommand::Escape` 时优先调用 `escape_clear_selection(state)` 并直接返回，确保第一下 `Esc` 仅清正文选区/`preview_selection`/树页搜索框选区，不连带收起已展开粘贴芯片或关闭面板
  - [x] `src/app/update/pane.rs`：`apply_lx_input` 在收到 `LxInput::Escape` 时优先调用 `escape_clear_selection(state)` 并直接返回
  - [x] `src/app/update/dialog.rs`（含 `worktree`、`draft`、`resume_edit`、`connect`）：在收到 `OverlayKey::Esc` 时优先调用 `escape_clear_selection(state)` 并直接返回
  - [x] `src/app/update/todo.rs`：`apply_todo_key` 在收到 `TodoKey::Esc` 时优先调用 `escape_clear_selection(state)` 并直接返回
  - [x] `src/app/update/workspace_history.rs`：`apply_workspace_history_key` 在收到 `OverlayKey::Esc` 时优先调用 `escape_clear_selection(state)` 并直接返回
- [x] 2.3 补齐后台任务页追问框（`TaskPage.agent_reply`）选区渲染与复制/剪切闭环（Review P0-1）
  - [x] `src/ui/main_content/lx/task.rs`：`draw_agent_reply` 按 `reply.selection()` 渲染 `Modifier::REVERSED` 反显
  - [x] `src/event/input.rs`：`copy_lx_selection` 与 `cut_lx_selection` 接入 `task.agent_reply` 的 `selected_text()` 与 `cut_selected()`
- [x] 2.4 补齐跨容器选区启动时的单选区互斥清理（含 Review P1-2）
  - [x] `src/event/overlay.rs`：`handle_overlay_click` 命中浮层输入框开始选区前调用 `update::clear_selection(state)`
  - [x] `src/app/update/menu.rs`、`src/app/update/prompt/pages.rs`：右键菜单 `MenuCommand::SelectAll` 与 `rename_prompt_page_title` 程序化选区前调用 `clear_selection(state)`
  - [x] `src/app/state.rs` / `src/app/update/`：在 `EditorCommand::SelectAll`、`TodoKey::SelectAll`、`OverlayKey::SelectAll`（浮层、Prompt 树/页搜索、侧栏历史搜索）以及 `Lx`/`AskEditor` 首次键盘 `Select*` 扩选时清理其他容器的残留选区

---

## 阶段 3：单元测试、事件级回归测试与静态检查

- [x] 3.1 编写覆盖全部选区场景、优先级与 Review P0–P3 修复项的测试（`src/app/update/selection/tests.rs`、`src/event/tests/terminal.rs`、`src/event/tests/lx_ask_task.rs`）
  - [x] 终端窗格：有非空选区或纯空白拖选反显时按 `Esc` 仅清选区且不向 PTY 发送 `\x1b`；空选区（单击未拖选）按 `Esc` 正常向 PTY 发送 `\x1b`
  - [x] `lx` 页：`state.lx_selection`、`lx.text_selection`、`lx.preview_selection`、`ask.editor()`、`task.agent_reply` 有选区时，按 `Esc` 优先于 `@`/`/`/块面板关闭、`Ask` 退出编辑态、任务页追问框/任务页关闭、空 Shell 退出、Rewind 待命、双 `Esc` 停止生成以及展开粘贴芯片收起
  - [x] `TaskPage.agent_reply`：全选后顶栏渲染 `REVERSED` 反显，支持 `Ctrl+C` 复制、`Ctrl+X` 剪切与 `Esc` 清除
  - [x] `Prompt` 面板：正文选区（含空行/行尾空白拖选反显）、`preview_selection`、树/页搜索框选区在按 `Esc` 时优先于面板关闭、树/页视图退出与展开粘贴芯片收起
  - [x] `Todo` 编辑框、侧栏工作区历史搜索框、模态浮层输入框：有选区时按第一下 `Esc` 仅清选区，第二下 `Esc` 才取消编辑/退出历史/关闭浮层；浮层打开且自身无选区（含 `Overlay::Menu` 右键菜单）时不被底层选区阻塞关闭，且关闭右键菜单后保留底层选区
  - [x] 后台 Tab 静态选区保留与跨容器单选区互斥（含右键菜单 `SelectAll`）
- [x] 3.2 执行精准验证与代码规范检查
  - [x] `rtk cargo fmt --check`
  - [x] `rtk cargo clippy --all-targets -- -D warnings`
  - [x] 运行相关单元测试与事件测试确认全部通过
- [x] 3.3 合并回主工作区 `dev` 并清理工作区与分支
  - [x] 合并 `feature/esc-clear-selection` 到 `dev`，适配与新合入 `/mcp`（`SlashPanel::Mcp`）二级面板的选区键放行与分步 `Esc` 协同
  - [x] 移除 `.worktrees/esc-clear-selection` 工作区、删除 `feature/esc-clear-selection` 分支及对应索引/隔离数据
