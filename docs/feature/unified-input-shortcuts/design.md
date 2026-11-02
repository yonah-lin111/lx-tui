# 全应用输入框快捷键与交互统一设计

状态：已实现并通过单元测试与 clippy 校验。

## 1. 背景、问题根因与验收口径

### 1.1 现状问题与根因分析

经 `codegraph` 与 `codebase-memory-mcp` 全量审计，应用中除主聊天输入框（`Lx`）与右侧 `Prompt` 正文编辑器外的其余输入框，在粘贴、快捷键路由、按词操作、选区剪切及跨行导航上存在碎片化与断路缺陷：

1. **大部分模态弹窗粘贴被直接吞掉（`src/event/input.rs:166-186`）**：
   - `handle_paste_event` 仅硬编码放行了 `ReferencedFolderInput` 和 `ReferencedFolderSearch`；
   - 其余 **6 个含 `TextInput` 的弹窗**（`Rename` 重命名工作区/标签/提示词、`NewWorkspace`、`NewNode`、`SaveDraft`、`WorktreeOpen`、`ResumeSessionEdit`）命中 `if state.overlay.is_some() { return; }` 后将粘贴内容直接丢弃，导致重命名 `ws item` 等场景无法粘贴。
2. **快捷键路由与分发断路（`Ctrl+Z` / `Ctrl+U` / `Ctrl+K` 等失效）**：
   - `ReferencedFolderInput`：虽然 `src/input/mod.rs:298-305` 解析了 `Undo`、`Redo`、`LineStart/End`、`DeleteToLineStart/End`，且底层 `src/app/update/referenced.rs:196` 实现了处理，但中间层 `src/app/update/dialog.rs:65-72` 的 `match` 分支漏写了这 6 个变体，导致按键落入 `_ => {}` 被静默丢弃。
   - `ReferencedFolderSearch`：`src/input/mod.rs:322` 将 `Ctrl+Z` 硬编码为 `OverlayKey::Back`（目录回退），未映射 `Redo`，且 `src/app/update/referenced/search.rs:287-305` 未接入 `Undo`、`Redo`、`LineStart/End`、`DeleteToLineStart/End`。
   - `apply_prompt_search`（`src/app/update/prompt/tree_view.rs:194`）：提示词树顶部搜索框未处理 `InsertText`（导致树视图下无法粘贴搜索词）、`Undo`、`Redo`、`LineStart/End`、`DeleteToLineStart/End`、`WordLeft/Right`、`DeleteWordBackward/Forward`。
3. **单行输入底层 `TextInput` 能力缺失（`src/app/overlay.rs:531`）**：
   - 8 个模态弹窗、右侧 `Todo` 编辑框、`Prompt` 树/页搜索框共用的 `TextInput` 缺少主输入框具备的**按词移动**（`word_left` / `word_right`）与**按词删除**（`delete_word_backward` / `delete_word_forward`）；
   - `overlay_key`（`src/input/mod.rs:260`）与 `todo_key`（`src/input/mod.rs:76`）也未映射 `Ctrl/Alt + Left/Right`、`Alt + B/F`、`Ctrl + W`、`Ctrl/Alt + Backspace`、`Alt + D`、`Ctrl + Delete`。
4. **选区剪切（`Ctrl+X` / `Cmd+X`）全局缺失 & `Prompt` 搜索框缺失鼠标选区**：
   - 全应用所有输入框（含 `Lx` 主输入框、`Ask` 手写框、`Todo` 编辑框、所有弹窗输入框、`Prompt` 搜索框与编辑器）均支持鼠标拖拽选区和 `Ctrl+C` 复制选区，但均未绑定 `Ctrl+X` / `Cmd+X` 剪切选区。
   - `Prompt` 树列表与页列表顶部的搜索框（`panel.search` / `panel.pages_view.search`）缺失鼠标点击落点、双击选词、三击全选、拖拽选区及反色渲染。
5. **折行多行输入框（`Ask` 手写框、`Todo` 编辑框）的 `↑` / `↓` 导航缺陷**：
   - `Ask` 手写框（`src/app/update/chat/ask.rs:271`）：`LxInput::Down` 落入 `apply_editor_edit`（`src/app/update/pane.rs:470`），而 `apply_editor_edit` 为保护 `Lx` 主输入框“空草稿按 `↓` 打开后台任务页”的特殊路由，特意未处理 `LxInput::Down`，导致 `Ask` 手写框折行或多行时按 `↓` 完全无反应。
   - `Todo` 编辑框（`src/app/update/todo.rs:462`）：显式将 `TodoKey::Up | TodoKey::Down` 返回 `false` 吞掉，而 UI 层最多软折行展示 3 行（`MAX_TODO_ITEM_ROWS = 3`），折行后无法用 `↑` / `↓` 跨视觉行移动光标。

### 1.2 验收目标

1. **全量输入框覆盖（12 处单行/内联输入框 + 主输入框剪切）**：
   - 8 个模态弹窗：`Rename`、`NewWorkspace`、`NewNode`、`SaveDraft`、`WorktreeOpen`、`ReferencedFolderInput`、`ReferencedFolderSearch`、`ResumeSessionEdit`；
   - 4 个面板内联输入框：右侧 `Todo` 编辑框、`Ask` 工具手写输入框、右侧 `Prompt` 树搜索框、右侧 `Prompt` 页搜索框；
   - `Lx` 主输入框：保持现有历史/任务页/面板路由不变，补充 `Ctrl+X` 剪切、`Ctrl+A` 全选、`Shift+方向键` 键盘选区，以及选区的删除/替换语义。
   - `Prompt` 正文编辑器：保持现有全部交互与特殊路由不变，仅补充有选区时的 `Ctrl+X` / `Cmd+X` 剪切能力。
2. **统一快捷键与鼠标选区基线**：
   - 终端粘贴 `Event::Paste` 全覆盖（有选区时替换选区并记一步 `Undo`）；
   - `Ctrl/Cmd+Z` 撤销，`Ctrl+Y` / `Ctrl/Cmd+Shift+Z` 重做；
   - `Home` / `Ctrl+A` 行首，`End` / `Ctrl+E` 行尾，`Ctrl+U` 删至行首，`Ctrl+K` 删至行尾；
   - `Ctrl/Alt+Left` / `Alt+B` 左移一词，`Ctrl/Alt+Right` / `Alt+F` 右移一词（注：`Prompt` 面板中 `Ctrl/Cmd+Left/Right` 保留为切页，按词移动走 `Alt+Left/Right` 与 `Alt+B/F`）；
   - `Ctrl+W` / `Ctrl/Alt+Backspace` 向前删词，`Alt+D` / `Ctrl+Delete` 向后删词；
   - 鼠标单击定位、双击选词、三击全选、拖拽选区；有选区时 `Ctrl/Cmd+C` 复制选区、`Ctrl/Cmd+X` 剪切选区（复制进剪贴板 + 删除选区记入一步 `Undo`）。

---

## 2. 已确认的产品与技术决策（Grill-Me 闭环）

| 编号 | 决策项 | 结论与具体行为 |
| --- | --- | --- |
| **D1** | **输入框统一范围与 `Lx` 隔离保护** | 覆盖全部 8 个弹窗输入框、`Todo` 编辑框、`Ask` 手写框、`Prompt` 树/页搜索框；**绝不改动 `Lx` 主输入框现有的 `Up/Down` 历史/任务页路由、`/` 与 `@` 面板优先级**。`Lx` 增量支持 `Ctrl+X` 剪切、`Ctrl+A` 全选、`Shift+方向键` 选区扩展（含按词、行首尾与跨视觉行），并让 `Backspace`/`Delete`/删词/删行与输入、粘贴、换行优先作用于选区。 |
| **D2** | **`ReferencedFolderSearch` 的 `Ctrl+Z` / `Ctrl+C` 优先级** | 采用“输入框有状态时优先输入框，否则回落列表操作”：<br>1. **`Ctrl/Cmd+Z`**：当 `search.query.can_undo()` 为真时，优先撤销搜索框文本并重算过滤；当撤销栈为空时，才执行 `referenced_search_back` 回退上一级目录。`Shift+Enter` 进入子目录或回退目录时重置 `query` 历史。<br>2. **`Ctrl/Cmd+C`**：当 `search.query.has_selection()` 为真时，优先复制搜索框选中文本；无选区时保持复制下方列表高亮项的 Markdown 引用。 |
| **D3** | **`Ask` 与 `Todo` 折行多行框的 `↑`/`↓` 行为** | 1. **`Ask` 手写框**：在 `apply_edit_input`（`src/app/update/chat/ask.rs`）内单独处理 `LxInput::Down`（调用 `editor.move_down()`），不触碰 `apply_editor_edit`，避免影响 `Lx` 主输入框。<br>2. **`Todo` 编辑框**：在 `TodoPanel` 同步记录编辑框可用列宽 `edit_text_width`，按 `↑`/`↓` 时在软折行视觉行间移动光标（首行再按 `↑` 落行首、末行再按 `↓` 落行尾），绝不退出编辑态或切换待办项。 |
| **D4** | **`Ctrl/Cmd+X` 选区剪切与 `Prompt` 搜索框鼠标选区** | 1. 所有输入框（含 `Lx` 主输入框、`Ask`、`Todo`、8 个弹窗、`Prompt` 树/页搜索框及 `Prompt` 编辑器）在存在有效选区时，按 `Ctrl/Cmd+X` 将选中文本复制到剪贴板（弹 Toast）并删除选区（记入一步 `Undo`）；无选区时 `Ctrl/Cmd+X` 为 no-op。<br>2. 为 `Prompt` 树搜索框与页搜索框补齐鼠标单击落点、双击选词、三击全选、拖拽选区及选区反色渲染。 |

---

## 3. 架构设计与数据流

### 3.1 模块修改与拆分结构

遵循 `AGENTS.md` 与 `docs/standards/code-writing-standards.md`“生产代码单文件上限 1000 行，拆分后子文件控制在 500 行左右”的规范：
- `src/app/overlay.rs` 当前已达 867 行，本次为 `TextInput` 扩展词操作、折行上下移动与通用按键分发后将接近上限，因此按职责将 `TextInput` 拆分为独立子模块 `src/app/overlay/text_input.rs`，由 `src/app/overlay.rs` 通过 `pub use text_input::TextInput;` 重导出，保持对外 API 零破坏。

```text
src/app/
  ├── actions.rs                      <-- OverlayKey / TodoKey / LxInput 新增 Cut 与词移动/删词变体；Routed 新增 Cut
  ├── overlay.rs                      <-- 保留各浮层结构体定义，新增 Overlay::input_mut() 等辅助方法，重导出 TextInput
  ├── overlay/
  │   ├── text_input.rs               <-- [拆分+增强] TextInput：新增 word_left/right、delete_word_backward/forward、
  │   │                                   cut_selected、can_undo、reset_history、move_up/down_wrapped、apply_edit_key
  │   └── tests.rs                    <-- 补充词操作、剪切、折行上下移动、apply_edit_key 单元测试
  ├── state.rs                        <-- AppState 新增 prompt_search_drag: bool 标记
  ├── state/
  │   ├── lx.rs                       <-- Lx 新增 delete_selected() / cut_selected()（不改任何现有路由）
  │   ├── lx/ask.rs                   <-- AskEditor 新增 cut_selected()
  │   └── todo.rs                     <-- TodoPanel 维护 edit_text_width: usize（由 sync_viewport 同步）
  └── update/
      ├── dialog.rs                   <-- 统一调用 apply_overlay_paste 与 TextInput::apply_edit_key，修复 ReferencedFolderInput 漏配
      ├── referenced.rs               <-- 复用 TextInput::apply_edit_key，编辑时自动清理旧 error
      ├── referenced/search.rs        <-- 接入完整编辑键，实现 Ctrl+Z 优先撤销 query 再回退目录，换根时重置 query 历史
      ├── worktree.rs                 <-- 复用 TextInput::apply_edit_key
      ├── resume_edit.rs              <-- 复用 TextInput::apply_edit_key
      ├── prompt/draft.rs             <-- NewNode / SaveDraft 复用 TextInput::apply_edit_key
      ├── prompt/tree_view.rs         <-- apply_prompt_search 补齐 InsertText、Undo/Redo、LineStart/End、删行、词操作、Cut
      ├── prompt/pages_view.rs        <-- apply_page_search 补齐词移动/删词、Cut
      ├── todo.rs                     <-- apply_todo_edit_key 接入词操作、Cut 与基于 edit_text_width 的 Up/Down 跨折行移动
      └── chat/ask.rs                 <-- apply_edit_input 补齐 LxInput::Down -> editor.move_down()
src/input/
  ├── mod.rs                          <-- 抽取单行编辑通用快捷键映射，补齐 overlay_key / todo_key / lx_input_key / route 的 Cut 与词操作
  └── tests.rs                        <-- 补充各容器与浮层的快捷键路由测试
src/event/
  ├── input.rs                        <-- handle_paste_event 覆盖全部 8 个输入浮层；handle_key_event 统一处理各容器 Cut 与选区 Copy
  └── mouse/
      ├── press.rs                    <-- Prompt 树/页搜索框支持鼠标单击/双击/三击选区
      └── drag.rs                     <-- Prompt 树/页搜索框支持鼠标拖拽延伸选区与释放清理
src/ui/
  ├── prompt/tree.rs                  <-- paint_search_row 支持选区反色渲染，新增 search_char_at / search_drag_char_at 几何命中
  ├── prompt/pages.rs                 <-- paint_search_row 支持选区反色渲染
  └── right_panel/todo.rs             <-- viewport_metrics 返回 text_width 供 Todo 编辑态折行导航使用
```

### 3.2 核心数据流

#### 流程 1：全浮层粘贴分发（`Event::Paste`）
1. 终端产生 `TerminalEvent::Paste(text)` $\rightarrow$ `event::input::handle_paste_event`；
2. 若 `state.overlay.is_some()`：调用 `update::apply_overlay_paste(state, &text)`：
   - `Rename(r)` $\rightarrow$ `r.input.insert_str(text)`
   - `NewWorkspace(ws)` $\rightarrow$ `ws.input.insert_str(text)`
   - `NewNode(n)` $\rightarrow$ `n.name.insert_str(text)`
   - `SaveDraft(d)` $\rightarrow$ `d.name.insert_str(text)`
   - `WorktreeOpen(w)` $\rightarrow$ 若 `w.query.insert_str(text)` 为真，调用 `w.normalize_selection()`
   - `ReferencedFolderInput(i)` $\rightarrow$ 若 `!i.validating`，调用 `i.input.insert_str(text)` 并清空 `i.error = None`
   - `ReferencedFolderSearch(s)` $\rightarrow$ 若 `s.query.insert_str(text)` 为真，调用 `update::refresh_referenced_search(state)`
   - `ResumeSessionEdit(r)` $\rightarrow$ 若 `!r.confirming_delete`，调用 `r.input.insert_str(text)` 并清空 `r.hint = None`
   - 其余无输入框浮层（`Menu`、`ConfirmClose` 等）吞掉粘贴返回 `false`，绝不穿透到底层终端。

#### 流程 2：`ReferencedFolderSearch` 的 `Ctrl+Z` 与 `Ctrl+C` 智能分流
1. **按下 `Ctrl/Cmd+Z`**：
   - `input::overlay_key` 在 `OverlayKind::ReferencedFolderSearch` 下将未带 `Shift` 的 `Ctrl/Cmd+Z` 映射为 `OverlayKey::Undo`，带 `Shift` 的 `Ctrl/Cmd+Shift+Z` 或 `Ctrl+Y` 映射为 `OverlayKey::Redo`；
   - 在 `src/app/update/dialog.rs` 处理 `Overlay::ReferencedFolderSearch` 时：
     - 当收到 `OverlayKey::Undo`：先检查 `search.query.can_undo()`；
     - 若为 `true`，调用 `super::edit_referenced_search(state, OverlayKey::Undo)` 撤销搜索词并重算过滤列表；
     - 若为 `false`（搜索框无编辑历史可撤），调用 `super::referenced_search_back(state)` 回退上一级目录；
   - 在 `goto_referenced_root`（`Shift+Enter` 进入子目录或回退目录）中，调用 `search.query.clear()` 后紧跟 `search.query.reset_history()`，确保新目录层级的搜索框处于干净初始态。
2. **按下 `Ctrl/Cmd+C`**：
   - 在 `src/event/input.rs` 的 `Routed::Overlay(key)` 入口：
     - 无论是 `OverlayKey::Clear` 还是 `OverlayKey::Copy`，只要 `overlay_selected_text(state)` 返回 `Some(text)`（即当前浮层输入框存在鼠标选区），优先执行 `copy_with_toast(state, &text, None)` 并直接返回；
     - 若无选区，`OverlayKey::Copy` 继续进入 `apply_overlay_key` 执行 `copy_referenced_entry_reference(state)` 复制列表高亮项引用。

#### 流程 3：全局有选区剪切（`Ctrl/Cmd+X`）
1. 路由层（`src/input/mod.rs`）：
   - `overlay_key`：在全部 8 个含输入框的 `OverlayKind` 中，`Ctrl/Cmd+X` 映射为 `OverlayKey::Cut`；
   - `todo_key`：`Ctrl/Cmd+X` 映射为 `TodoKey::Cut`；
   - `lx_input_key`：`Ctrl/Cmd+X` 映射为 `LxInput::Cut`；
   - `route`（`FocusedContainer::Prompt`）：`Ctrl/Cmd+X` 映射为 `Routed::Cut`。
2. 事件与状态更新层（`src/event/input.rs` + `src/app/update/`）：
   - **Overlay**：调用 `update::cut_overlay_selected_text(state)`，若返回 `Some(text)`（内部完成 `push_undo` + `delete_selected` + 必要的列表重算如 `WorktreeOpen::normalize_selection` / `ReferencedFolderSearch::refilter`），则调用 `copy_with_toast(state, &text, None)`；
   - **Todo**：调用 `update::cut_todo_selected_text(state)`，若返回 `Some(text)`，则调用 `copy_with_toast(state, &text, Some(state.prompt.id()))`；
   - **Lx / Ask**：调用 `cut_lx_selection(state)`：
     - 若 `pane.lx.ask` 处于 `editing` 态，调用 `ask.editor_mut().and_then(AskEditor::cut_selected)`；
     - 否则调用 `pane.lx.cut_selected()`，成功后调用 `update::refresh_slash_panel(state, pane)`；
     - 若切出非空文本，调用 `copy_with_toast(state, &text, Some(pane))`；
   - **Prompt（树搜索 / 页搜索 / 正文编辑）**：
     - 树模式：调用 ` panel.search.cut_selected()`，若有选区则重算树过滤并 `copy_with_toast`；
     - 页模式：调用 `panel.pages_view.search.cut_selected()`，若有选区则重算页过滤并 `copy_with_toast`；
     - 正文编辑模式：若 `prompt_selection_bounds(state)` 存在 `(start, end)`，先取 `prompt_copy_text(state)`，再执行 `state.prompt.replace_range(start, end, "")` 与 `clear_selection(state)`，并调用 `copy_with_toast`。

---

## 4. 关键代码设计

### 4.1 `TextInput` 扩展能力（`src/app/overlay/text_input.rs`）

```rust
use crate::app::actions::OverlayKey;
use crate::app::prompt::text::{
    char_width, offset_at_col, word_end_after, word_start_before,
};

impl TextInput {
    /// 是否存在可撤销的历史步。
    pub fn can_undo(&self) -> bool {
        !self.undo_stack.is_empty()
    }

    /// 清空撤销与重做历史（用于目录切换等重置输入框初始状态的场景）。
    pub fn reset_history(&mut self) {
        self.undo_stack.clear();
        self.redo_stack.clear();
    }

    /// 剪切当前选区文本：有选区时记录一步撤销、删除选区并返回被剪切文本；无选区返回 None。
    pub fn cut_selected(&mut self) -> Option<String> {
        let selected = self.selected_text()?;
        self.push_undo();
        self.delete_selected();
        Some(selected)
    }

    /// 光标左移一个词（清除选区）。
    pub fn move_word_left(&mut self) {
        self.clear_selection();
        let byte = self.byte_offset(self.cursor);
        let target = word_start_before(&self.text, byte);
        self.cursor = self.text[..target].chars().count();
    }

    /// 光标右移一个词（清除选区）。
    pub fn move_word_right(&mut self) {
        self.clear_selection();
        let byte = self.byte_offset(self.cursor);
        let target = word_end_after(&self.text, byte);
        self.cursor = self.text[..target].chars().count();
    }

    /// 向前删除一个词（Ctrl+W / Alt+Backspace；有选区时删除选区）。
    pub fn delete_word_backward(&mut self) {
        if self.has_selection() {
            self.push_undo();
            self.delete_selected();
            return;
        }
        let byte = self.byte_offset(self.cursor);
        let start = word_start_before(&self.text, byte);
        if start < byte {
            self.push_undo();
            self.text.replace_range(start..byte, "");
            self.cursor = self.text[..start].chars().count();
        }
    }

    /// 向后删除一个词（Alt+D / Ctrl+Delete；有选区时删除选区，光标保持不动）。
    pub fn delete_word_forward(&mut self) {
        if self.has_selection() {
            self.push_undo();
            self.delete_selected();
            return;
        }
        let byte = self.byte_offset(self.cursor);
        let end = word_end_after(&self.text, byte);
        if byte < end {
            self.push_undo();
            self.text.replace_range(byte..end, "");
        }
    }

    /// 在给定折行宽度 `width` 下将光标上移一个视觉行（首行再上移则跳至行首）。
    pub fn move_up_wrapped(&mut self, width: usize) {
        self.clear_selection();
        let lines = self.wrapped_ranges(width.max(1));
        let (row, col) = self.cursor_wrapped_pos(&lines);
        if row == 0 {
            self.cursor = 0;
        } else {
            let prev = &lines[row - 1];
            let slice = &self.text[prev.start_byte..prev.end_byte];
            let rel_byte = offset_at_col(slice, col);
            self.cursor = prev.start_char + slice[..rel_byte].chars().count();
        }
    }

    /// 在给定折行宽度 `width` 下将光标下移一个视觉行（末行再下移则跳至行尾）。
    pub fn move_down_wrapped(&mut self, width: usize) {
        self.clear_selection();
        let lines = self.wrapped_ranges(width.max(1));
        let (row, col) = self.cursor_wrapped_pos(&lines);
        if row + 1 >= lines.len() {
            self.cursor = self.text.chars().count();
        } else {
            let next = &lines[row + 1];
            let slice = &self.text[next.start_byte..next.end_byte];
            let rel_byte = offset_at_col(slice, col);
            self.cursor = next.start_char + slice[..rel_byte].chars().count();
        }
    }

    /// 统一处理单行浮层编辑键；返回是否消费了该按键。
    pub fn apply_edit_key(&mut self, key: OverlayKey) -> bool {
        match key {
            OverlayKey::Char(ch) => self.insert_char(ch),
            OverlayKey::Clear => self.clear(),
            OverlayKey::Cut => {
                self.cut_selected();
            }
            OverlayKey::Backspace => self.backspace(),
            OverlayKey::Delete => self.delete(),
            OverlayKey::Left => self.move_left(),
            OverlayKey::Right => self.move_right(),
            OverlayKey::Home | OverlayKey::LineStart => self.move_home(),
            OverlayKey::End | OverlayKey::LineEnd => self.move_end(),
            OverlayKey::WordLeft => self.move_word_left(),
            OverlayKey::WordRight => self.move_word_right(),
            OverlayKey::DeleteToLineStart => self.delete_to_line_start(),
            OverlayKey::DeleteToLineEnd => self.delete_to_line_end(),
            OverlayKey::DeleteWordBackward => self.delete_word_backward(),
            OverlayKey::DeleteWordForward => self.delete_word_forward(),
            OverlayKey::Undo => {
                self.undo();
            }
            OverlayKey::Redo => {
                self.redo();
            }
            _ => return false,
        }
        true
    }
}
```

### 4.2 浮层与 Todo 键盘路由统一（`src/input/mod.rs`）

抽取单行输入框通用编辑快捷键映射函数 `overlay_text_edit_key(key: KeyEvent) -> Option<OverlayKey>`，供全部 8 个输入类 `OverlayKind` 共用，彻底消除漏配：

```rust
/// 单行输入浮层共用的文本编辑按键映射。
fn overlay_text_edit_key(key: KeyEvent) -> Option<OverlayKey> {
    let plain = !key
        .modifiers
        .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT | KeyModifiers::SUPER);
    let shift = key.modifiers.contains(KeyModifiers::SHIFT);
    let ctrl = key
        .modifiers
        .intersects(KeyModifiers::CONTROL | KeyModifiers::SUPER)
        && !key.modifiers.contains(KeyModifiers::ALT);
    let alt_only = key.modifiers == KeyModifiers::ALT;
    let ctrl_only = key.modifiers == KeyModifiers::CONTROL;

    match key.code {
        KeyCode::Char('z' | 'Z') if ctrl && shift => Some(OverlayKey::Redo),
        KeyCode::Char('z' | 'Z') if ctrl => Some(OverlayKey::Undo),
        KeyCode::Char('y' | 'Y') if ctrl => Some(OverlayKey::Redo),
        KeyCode::Char('c' | 'C') if ctrl => Some(OverlayKey::Clear),
        KeyCode::Char('x' | 'X') if ctrl => Some(OverlayKey::Cut),
        KeyCode::Char('a' | 'A') if ctrl => Some(OverlayKey::LineStart),
        KeyCode::Char('e' | 'E') if ctrl => Some(OverlayKey::LineEnd),
        KeyCode::Char('u' | 'U') if ctrl => Some(OverlayKey::DeleteToLineStart),
        KeyCode::Char('k' | 'K') if ctrl => Some(OverlayKey::DeleteToLineEnd),
        KeyCode::Char('w' | 'W') if ctrl => Some(OverlayKey::DeleteWordBackward),
        KeyCode::Char('b' | 'B') if alt_only => Some(OverlayKey::WordLeft),
        KeyCode::Char('f' | 'F') if alt_only => Some(OverlayKey::WordRight),
        KeyCode::Char('d' | 'D') if alt_only => Some(OverlayKey::DeleteWordForward),
        KeyCode::Backspace if ctrl_only || alt_only => Some(OverlayKey::DeleteWordBackward),
        KeyCode::Delete if ctrl_only => Some(OverlayKey::DeleteWordForward),
        KeyCode::Backspace => Some(OverlayKey::Backspace),
        KeyCode::Delete => Some(OverlayKey::Delete),
        KeyCode::Left if ctrl_only || alt_only => Some(OverlayKey::WordLeft),
        KeyCode::Right if ctrl_only || alt_only => Some(OverlayKey::WordRight),
        KeyCode::Left if plain => Some(OverlayKey::Left),
        KeyCode::Right if plain => Some(OverlayKey::Right),
        KeyCode::Home => Some(OverlayKey::Home),
        KeyCode::End => Some(OverlayKey::End),
        KeyCode::Char(ch) if plain => Some(OverlayKey::Char(ch)),
        _ => None,
    }
}
```

### 4.3 `Ask` 手写框下移修复与 `Lx` 剪切能力（`src/app/update/chat/ask.rs` & `src/app/state/lx.rs`）

在 `src/app/update/chat/ask.rs` 的 `apply_edit_input` 中单独拦截 `LxInput::Down`：

```rust
        // ↓ 在手写编辑框内：下移一个视觉行（已在末行时停在末尾，不离开编辑框）。
        LxInput::Down => state
            .pane_mut_anywhere(pane)
            .and_then(|target| target.lx.ask.as_mut())
            .and_then(AskState::editor_mut)
            .is_some_and(|editor| {
                editor.move_down();
                true
            }),
```

在 `src/app/state/lx.rs` 中为 `Lx` 新增 `delete_selected` 与 `cut_selected`（不触碰任何现有方法）：

```rust
    /// 删除输入框当前选中的文本区间并记录一步撤销；无选区返回 false。
    pub fn delete_selected(&mut self) -> bool {
        let Some((start, end)) = self.text_selection_range() else {
            self.text_selection = None;
            return false;
        };
        self.refer_suppressed = false;
        self.snapshot();
        self.input.replace_range(start..end, "");
        self.cursor = start;
        self.adopt_shell_prefix();
        self.refresh_panel();
        true
    }

    /// 剪切输入框当前选中的文本：返回被剪切文本并将删除记入一步撤销；无选区返回 None。
    pub fn cut_selected(&mut self) -> Option<String> {
        let text = self.selected_text()?;
        self.delete_selected();
        Some(text)
    }
```

---

## 5. 边界条件与回归防护

1. **`Lx` 主输入框零回归保证**：
   - 不修改 `apply_lx_input` 中 `Up/Down` 历史浏览与打开后台任务页的逻辑；
   - 不修改 `apply_editor_edit` 的签名或分支，保持 `apply_editor_edit_matches_lx_input_for_every_editor_command` 单测 100% 绿灯；
   - `LxInput::Cut` 仅在 `pane.lx.text_selection_range().is_some()`（或 `ask.editing` 有选区）时生效，无选区时完全无副作用。
2. **弹窗编辑保护态守卫**：
   - `ReferencedFolderInput` 处于 `validating == true` 时拒绝粘贴与编辑；编辑或粘贴后自动清除上一次校验遗留的 `error` 提示；
   - `ResumeSessionEdit` 处于 `confirming_delete == true`（二次确认删除态）时拒绝粘贴与编辑，防止“要删的会话”和“正在改的标题”混淆。
3. **长度上限守卫**：
   - `Todo` 编辑框粘贴与输入继续严格遵守 `MAX_TODO_TEXT_CHARS = 500` 字符上限；
   - `Ask` 手写框粘贴与输入继续严格遵守 `MAX_ASK_CUSTOM_BYTES = 2048` 字节上限。
4. **单文件行数红线**：
   - `src/app/overlay.rs` 拆出 `src/app/overlay/text_input.rs`，保证所有修改后生产文件均远低于 1000 行上限。
