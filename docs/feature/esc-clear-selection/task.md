# 全局 `Esc` 取消文字选中与优先级开发任务

状态：已完成并通过单元测试、事件级测试与 clippy 校验（2026-11-06）。
对应设计：`docs/feature/esc-clear-selection/design.md`。

---

## 阶段 1：全局选区查询、清理与单选区互斥基线（`src/app/overlay.rs`、`src/app/state/todo.rs`、`src/app/update/selection.rs`）

- [x] 1.1 扩展 `Overlay` 与 `TodoPanel` 选区访问辅助方法
  - [x] 在 `src/app/overlay.rs` 的 `Overlay` 上新增 `has_selection(&self) -> bool` 与 `clear_selection(&mut self)`（覆盖 `Rename`、`NewWorkspace`、`WorktreeOpen`、`NewNode`、`SaveDraft`、`ReferencedFolderInput`、`ReferencedFolderSearch`、`ResumeSessionEdit`、`Connect`）
  - [x] 在 `src/app/state/todo.rs` 的 `TodoPanel` 上新增只读方法 `edit_input(&self) -> Option<&TextInput>` 与 `clear_selection(&mut self)`
- [x] 1.2 补全 `clear_selection(state: &mut AppState)`（`src/app/update/selection.rs`）
  - [x] 补齐清理 `state.overlay_input_drag = false` 与 `state.overlay.as_mut().map(Overlay::clear_selection)`
  - [x] 补齐清理 `state.prompt_search_drag = false` 及所有工作区 `prompt_panel.search` / `prompt_panel.pages_view.search` 的选区
  - [x] 补齐清理 `state.workspace_history.search_drag = false` 与 `state.workspace_history.search.clear_selection()`
  - [x] 补齐清理各窗格 `pane.lx.task` 内联追问框 `agent_reply` 的选区
- [x] 1.3 实现 `has_visible_selection(state: &AppState) -> bool` 与 `escape_clear_selection(state: &mut AppState) -> bool`（`src/app/update/selection.rs` & `src/app/update.rs`）
  - [x] 实现模态浮层打开时仅检测浮层自身输入框选区的遮挡隔离规则
  - [x] 实现无浮层时按容器可见性（侧栏历史搜索框、右栏 `Prompt` 树/页/正文/展开芯片与 `Todo` 编辑框、主窗格区当前活动 Tab 下所有可见 `Lx`/`Terminal` 窗格）检测非空选区
  - [x] 实现 `escape_clear_selection(state)`：有可见非空选区时清理全部选区与拖选/自动滚动状态并返回 `true`；无非空选区时静默清理残留空锚点/拖选状态并返回 `false`
  - [x] 在 `src/app/update.rs` 中经 `pub use selection::*;` 重导出 `has_visible_selection` 与 `escape_clear_selection`

---

## 阶段 2：接入最高优先级 `Esc` 拦截与各容器单选区互斥（`src/event/` & `src/app/update/`）

- [x] 2.1 在事件入口接入最高优先级裸 `Esc` 拦截（`src/event/input.rs`）
  - [x] 在 `handle_key_event` 顶部（`KeyEventKind::Release` 之后，早于侧栏历史拦截、`input::route`、Prompt recent 抽屉与终端 `Routed::Pane` 写入）拦截裸 `Esc`（`key.code == KeyCode::Esc && key.modifiers.is_empty()`），调用 `update::escape_clear_selection(state)`，返回 `true` 时置 `*dirty = true` 并立即返回
- [x] 2.2 同步 Update 层直接调用入口并修复 `Prompt` 展开粘贴芯片分步 `Esc`（`src/app/update/`）
  - [x] `src/app/update/editor.rs`：`apply_editor` 在收到 `EditorCommand::Escape` 时优先调用 `escape_clear_selection(state)` 并直接返回，确保第一下 `Esc` 仅清正文选区/`preview_selection`/树页搜索框选区，不连带收起已展开粘贴芯片或关闭面板
  - [x] `src/app/update/pane.rs`：`apply_lx_input` 在收到 `LxInput::Escape` 时优先调用 `escape_clear_selection(state)` 并直接返回
  - [x] `src/app/update/dialog.rs`（含 `worktree`、`draft`、`resume_edit`、`connect`）：在收到 `OverlayKey::Esc` 时优先调用 `escape_clear_selection(state)` 并直接返回
  - [x] `src/app/update/todo.rs`：`apply_todo_key` 在收到 `TodoKey::Esc` 时优先调用 `escape_clear_selection(state)` 并直接返回
  - [x] `src/app/update/workspace_history.rs`：`apply_workspace_history_key` 在收到 `OverlayKey::Esc` 时优先调用 `escape_clear_selection(state)` 并直接返回
- [x] 2.3 补齐跨容器选区启动时的单选区互斥清理
  - [x] `src/event/overlay.rs`：`handle_overlay_click` 命中浮层输入框开始选区前调用 `update::clear_selection(state)`
  - [x] `src/app/state.rs` / `src/app/update/`：在 `select_all_prompt`、`TodoKey::SelectAll`、`OverlayKey::SelectAll`（浮层、Prompt 树/页搜索、侧栏历史搜索）以及 `Lx`/`AskEditor` 首次键盘 `Select*` 扩选时清理其他容器的残留选区

---

## 阶段 3：单元测试、事件级回归测试与静态检查

- [x] 3.1 编写覆盖全部选区场景与优先级的测试（`src/app/update/selection/tests.rs` & `src/event/tests/terminal.rs`）
  - [x] 终端窗格：有非空选区按 `Esc` 仅清选区且不向 PTY 发送 `\x1b`；空选区（单击未拖选）按 `Esc` 正常向 PTY 发送 `\x1b`
  - [x] `lx` 页：`state.lx_selection`、`lx.text_selection`、`lx.preview_selection`、`ask.editor()`、`task.agent_reply` 有选区时，按 `Esc` 优先于 `@`/`/`/块面板关闭、`Ask` 退出编辑态、任务页追问框/任务页关闭、空 Shell 退出、Rewind 待命、双 `Esc` 停止生成以及展开粘贴芯片收起
  - [x] `Prompt` 面板：正文选区、`preview_selection`、树/页搜索框选区在按 `Esc` 时优先于面板关闭、树/页视图退出与展开粘贴芯片收起（验证两步 `Esc`：先清选区、再收芯片/退视图）
  - [x] `Todo` 编辑框、侧栏工作区历史搜索框、模态浮层输入框：有选区时按第一下 `Esc` 仅清选区，第二下 `Esc` 才取消编辑/退出历史/关闭浮层；浮层打开且自身无选区时不被底层被遮挡选区吞掉 `Esc`
  - [x] 鼠标按住拖选与自动滚动途中按 `Esc`：立即清除选区并终止拖拽状态，继续移动鼠标不再拉出选区
- [x] 3.2 执行精准验证与代码规范检查
  - [x] `rtk cargo fmt --check`
  - [x] `rtk cargo clippy --all-targets -- -D warnings`
  - [x] 运行相关单元测试与事件测试确认全部通过
