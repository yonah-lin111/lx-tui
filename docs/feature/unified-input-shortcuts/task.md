# 全应用输入框快捷键与交互统一开发任务

状态：已完成并验证（lib 全部通过；3 个既有集成测试在基线同样失败，详见本次变更说明）。
对应设计：`docs/feature/unified-input-shortcuts/design.md`。

---

## 阶段 1：`TextInput` 拆分与核心编辑能力完备化（`src/app/overlay/` & `src/app/actions.rs`）

- [x] 1.1 扩展动作枚举（`src/app/actions.rs` & `src/input/mod.rs`）
  - [x] 在 `OverlayKey` 中新增 `Cut`、`WordLeft`、`WordRight`、`DeleteWordBackward`、`DeleteWordForward`
  - [x] 在 `TodoKey` 中新增 `Cut`、`WordLeft`、`WordRight`、`DeleteWordBackward`、`DeleteWordForward`
  - [x] 在 `LxInput` 中新增 `Cut`
  - [x] 在 `Routed`（`src/input/mod.rs`）中新增 `Cut`
- [x] 1.2 拆分并增强 `src/app/overlay/text_input.rs`
  - [x] 将 `TextInput` 从 `src/app/overlay.rs` 拆至 `src/app/overlay/text_input.rs`，并在 `src/app/overlay.rs` 中 `pub use text_input::TextInput;`
  - [x] 实现 `can_undo(&self) -> bool` 与 `reset_history(&mut self)`
  - [x] 实现 `cut_selected(&mut self) -> Option<String>`（记录单步撤销并删除选区）
  - [x] 复用 `crate::app::prompt::text::{word_start_before, word_end_after}` 实现 `move_word_left`、`move_word_right`、`delete_word_backward`、`delete_word_forward`
  - [x] 实现基于显示列宽折行的 `move_up_wrapped(&mut self, width: usize)` 与 `move_down_wrapped(&mut self, width: usize)`
  - [x] 实现统一按键消费入口 `apply_edit_key(&mut self, key: OverlayKey) -> bool`
- [x] 1.3 补充 `src/app/overlay/tests.rs` 单元测试
  - [x] 测试按词左右移动、向前/向后删词（含中英文及有选区优先删选区）
  - [x] 测试 `cut_selected` 与单步 `undo` 恢复
  - [x] 测试 `move_up_wrapped` / `move_down_wrapped` 跨折行移动与首末行边界钳制

---

## 阶段 2：全浮层弹窗按键与粘贴统一（`src/app/update/` & `src/event/input.rs`）

- [x] 2.1 统一浮层键盘路由（`src/input/mod.rs`）
  - [x] 抽取 `overlay_text_edit_key(key: KeyEvent) -> Option<OverlayKey>`
  - [x] 在 `Rename`、`NewWorkspace`、`NewNode`、`SaveDraft`、`WorktreeOpen`、`ReferencedFolderInput`、`ReferencedFolderSearch`、`ResumeSessionEdit` 中统一复用该映射
  - [x] 在 `ReferencedFolderSearch` 中将 `Ctrl/Cmd+Z` 映射为 `OverlayKey::Undo`、`Ctrl/Cmd+Shift+Z` / `Ctrl+Y` 映射为 `OverlayKey::Redo`，`Ctrl/Cmd+C` 保持 `OverlayKey::Copy`
- [x] 2.2 修复并统一各浮层 Update 分发（`src/app/update/`）
  - [x] `src/app/update/dialog.rs`：修复 `ReferencedFolderInput` 漏转发 `Undo/Redo/LineStart/End/DeleteToLine*` 的问题，统一使用 `TextInput::apply_edit_key`
  - [x] `src/app/update/referenced/search.rs`：`edit_referenced_search` 接入 `TextInput::apply_edit_key`；在 `dialog.rs` 中实现 `OverlayKey::Undo` 优先撤销 `search.query`（`can_undo()` 为真时），无历史可撤时回退目录 `referenced_search_back`；`goto_referenced_root` 切换目录时调用 `search.query.reset_history()`
  - [x] `src/app/update/worktree.rs`、`src/app/update/resume_edit.rs`、`src/app/update/prompt/draft.rs`：统一复用 `TextInput::apply_edit_key`
  - [x] 新增 `update::apply_overlay_paste(state: &mut AppState, text: &str) -> bool` 与 `update::cut_overlay_selected_text(state: &mut AppState) -> Option<String>`
- [x] 2.3 打通全浮层粘贴与剪切事件（`src/event/input.rs`）
  - [x] 在 `handle_paste_event` 中调用 `update::apply_overlay_paste(state, &text)`，使全部 8 个输入浮层支持粘贴
  - [x] 在 `Routed::Overlay` 中支持 `OverlayKey::Cut` 剪切选区到剪贴板并弹 Toast，且在 `OverlayKey::Clear | OverlayKey::Copy` 有选区时优先复制选区文本

---

## 阶段 3：`Todo` 编辑框、`Ask` 手写框、`Prompt` 搜索框与 `Lx` 剪切（`app/`、`ui/`、`event/`）

- [x] 3.1 升级 `Todo` 编辑框（`src/input/mod.rs`、`src/app/state/todo.rs`、`src/app/update/todo.rs`、`src/ui/right_panel/todo.rs`）
  - [x] `todo_key` 补齐词移动（`Ctrl/Alt+Left/Right`、`Alt+B/F`）、删词（`Ctrl+W`、`Ctrl/Alt+Backspace`、`Alt+D`、`Ctrl+Delete`）与 `Ctrl/Cmd+X`（`TodoKey::Cut`）
  - [x] `viewport_metrics` 与 `TodoPanel::sync_viewport` 同步 `edit_text_width: usize`
  - [x] `apply_todo_edit_key` 接入词移动、删词、`Cut` 以及 `TodoKey::Up` / `TodoKey::Down`（调用 `move_up_wrapped` / `move_down_wrapped`）
  - [x] `event/input.rs` 中处理 `TodoKey::Cut` 复制选区至剪贴板并删除选区
- [x] 3.2 修复 `Ask` 手写输入框并支持 `Lx` / `Ask` 剪切（`src/app/state/lx.rs`、`src/app/state/lx/ask.rs`、`src/app/update/chat/ask.rs`、`src/event/input.rs`）
  - [x] `src/app/update/chat/ask.rs`：在 `apply_edit_input` 中处理 `LxInput::Down` 调用 `editor.move_down()`（不改动 `apply_editor_edit` 与 `Lx` 主输入框路由）
  - [x] `src/app/state/lx.rs` & `src/app/state/lx/ask.rs`：新增 `delete_selected` / `cut_selected`
  - [x] `src/input/mod.rs` & `src/event/input.rs`：`lx_input_key` 映射 `Ctrl/Cmd+X` 为 `LxInput::Cut`，在 `event/input.rs` 中实现 `cut_lx_selection`（支持 `Ask` 编辑态与 `Lx` 主输入框选区剪切）
- [x] 3.3 补齐 `Prompt` 树搜索框 / 页搜索框完整快捷键与鼠标选区（`src/app/update/prompt/`、`src/ui/prompt/`、`src/event/`）
  - [x] `src/app/update/prompt/tree_view.rs`：`apply_prompt_search` 补齐 `InsertText`（修复树搜索粘贴）、`Undo`、`Redo`、`LineStart/End`、`DeleteToLineStart/End`、`WordLeft/Right`、`DeleteWordBackward/Forward`
  - [x] `src/app/update/prompt/pages_view.rs`：`apply_page_search` 补齐 `WordLeft/Right`、`DeleteWordBackward/Forward`
  - [x] `src/ui/prompt/tree.rs` & `src/ui/prompt/pages.rs`：`paint_search_row` 使用 `widgets::input::build_styled_spans` 渲染选区反色高亮；暴露搜索框字符命中纯函数 `search_char_at` / `search_drag_char_at`
  - [x] `src/event/mouse/press.rs` & `src/event/mouse/drag.rs`：支持点击/双击/三击与拖拽选中 `Prompt` 树/页顶部搜索框文本
  - [x] `src/event/input.rs`：`Routed::Copy` 在树/页模式下若搜索框有选区则优先复制选区（无选区才清空搜索词）；处理 `Routed::Cut` 支持树/页搜索框及 `Prompt` 正文编辑器的选区剪切

---

## 阶段 4：精准验证与回归测试

- [x] 4.1 补充并运行受影响模块的单元测试：
  - `cargo test app::overlay`
  - `cargo test input::tests`
  - `cargo test app::update`
  - `cargo test event::tests`
- [x] 4.2 运行代码格式与静态检查：
  - `cargo fmt --check`
  - `cargo clippy --all-targets -- -D warnings`

---

## 阶段 5：`Lx` 主输入框键盘选区与选择删除

- [x] 5.1 新增 `LxInput` 选区命令：`SelectLeft/Right`、`SelectWordLeft/Right`、`SelectHome/End`、`SelectUp/Down`、`SelectAll`
- [x] 5.2 `lx_input_key` 映射：`Shift+←/→/↑/↓/Home/End` 扩展选区；`Shift+Ctrl/Alt+←/→` 与 `Shift+Alt+B/F` 按词扩展；`Ctrl+A` 保持回行首，`Ctrl/Cmd+Shift+A` 全选
- [x] 5.3 `Lx` 增加选区扩展方法（保留鼠标选区模型，按字符/词/视觉行扩展，不触发面板刷新清空选区）
- [x] 5.4 `Backspace`/`Delete`/`Ctrl+U`/`Ctrl+K`/`Ctrl+W`/`Alt+D` 优先删除选区；输入字符、粘贴、换行替换选区并只记一步撤销
- [x] 5.5 `AskEditor` 同步委托选区方法，共享同一套 `LxInput` 编辑分派
- [x] 5.6 补充测试：键盘选区删除/替换、Shift 路由、Ask 选区删除，以及 `apply_editor_edit` 全命令一致性

---

## 阶段 6：全选快捷键统一

- [x] 6.1 `Ctrl+A` 在所有输入框保持“回到行首”（含恢复 `Lx`、`Todo`、浮层、Prompt 搜索与编辑器）
- [x] 6.2 `Ctrl/Cmd+Shift+A` 统一映射为全选：`TextInput` 浮层、`Todo` 编辑框、`Prompt` 树/页搜索框、`Prompt` 编辑器、`Lx` 与 `Ask`
- [x] 6.3 补充路由与行为测试，确保 `Ctrl+A` / `Ctrl+Shift+A` 不再互相覆盖
