# 全局 `Esc` 取消文字选中与优先级设计（参考 OpenCode）

状态：已实现并通过单元测试、事件级测试与 clippy 校验（2026-11-06）。

---

## 1. 背景、OpenCode 参考实现与现状问题分析

### 1.1 OpenCode（`opencode-dev`）参考实现分析

经 `codebase-memory-mcp` 与源码核验，`opencode-dev` 在 TUI 层通过全局最高优先级按键拦截器实现 `Esc` 取消文字选中：

1. **根组件最高优先级拦截注册（`packages/tui/src/app.tsx:591-598`）**：
   ```ts
   // Selection copy/dismiss must precede both app bindings and the terminal pane's raw key forwarding.
   const offSelectionKeys = keymap.intercept(
     "key",
     ({ event }) => {
       Selection.handleSelectionKey(renderer, toast, event, clipboard, copyOnSelectEnabled())
     },
     { priority: 101 },
   )
   ```
   - `priority: 101` 是整个 OpenCode TUI 中优先级最高的拦截器，排在**终端窗格原始按键转发（`priority: 100`）**、**Devtools 栏（`priority: 10`）**、**Pending 键序列清理（`priority: 0`）** 以及所有 **Layer 命令（Modal 弹窗关闭、Autocomplete 补全面板隐藏、退出 Shell 模式、`session.interrupt` 双 `Esc` 中断会话）** 之前。

2. **核心处理逻辑（`packages/tui/src/util/selection.ts:65-101`）**：
   ```ts
   export function handleSelectionKey(
     renderer: Renderer,
     toast: Toast,
     event: SelectionKeyEvent,
     clipboard: ClipboardService,
     copyOnSelect: boolean,
   ) {
     const selection = renderer.getSelection()
     if (!selection) return
     const focus = renderer.currentFocusedEditor
     const editing = focus?.hasSelection() && selection.selectedRenderables.includes(focus)

     if (event.ctrl && (event.name === "c" || event.baseCode === 99 || event.baseCode === 67)) {
       ...
     }

     if (event.name === "escape") {
       const text = selection.isStart && selection.behavior === "cell" ? "" : selection.getSelectedText()
       renderer.clearSelection()
       if (!text) return
       event.preventDefault()
       event.stopPropagation()
       return
     }

     if (editing) return

     renderer.clearSelection()
   }
   ```
   - **核心语义（先清选区、按非空决定是否消费 `Esc`）**：
     1. 只要按下 `escape`：首先无条件调用 `renderer.clearSelection()` 清理选区状态；
     2. 若选区包含**非空文本**（`text` 非空，且不是仅单击起点的空单元格）：调用 `event.preventDefault()` + `event.stopPropagation()` **吞掉本次 `Esc` 按键**，不关闭弹窗、不隐藏补全面板、不退出编辑态、不中断会话、也不向终端 PTY 发送 `\x1b`；
     3. 若选区为空（例如仅单击未拖出文字或空拖拽）：清理残留锚点后直接 `return`，**不消费 `Esc`**，允许 `Esc` 继续向下派发（如关闭弹窗或向终端发送 `\x1b`）。

---

### 1.2 `lx-tui` 现状与缺陷根因

经 `codegraph` 与 `codebase-memory-mcp` 全量盘点，`lx-tui` 当前共有 **11 类支持文字选中的区域**，但各区域对 `Esc` 的处理分散且存在多处缺失与优先级倒置：

| 区域 | 选区状态字段 | 现状按 `Esc` 的问题 |
| --- | --- | --- |
| **1. `lx` 主内容区 / 后台任务输出区** | `state.lx_selection: Option<Selection>` | **完全不响应 `Esc` 清选区**。按 `Esc` 会直接跳过主内容选区去关面板、退任务页、退出空 Shell、触发 Rewind 待命或双 `Esc` 停止生成。 |
| **2. `lx` 底部多行输入框** | `pane.lx.text_selection: Option<(usize, usize, SelectionUnit)>` | **完全不响应 `Esc` 清选区**（`apply_lx_input` 的 `LxInput::Escape` 分支未清理 `text_selection`）。按 `Esc` 会直接关 `@`/`/` 面板、收起展开芯片、退出 Shell 或触发双 `Esc` 停止生成。 |
| **3. `lx` 展开粘贴芯片预览区** | `pane.lx.preview_selection: Option<(usize, usize)>` | 优先级排在最后（`src/app/update/pane.rs:395`）：若同时开着 `@`/`/`/块命令面板或处于历史浏览态/Rewind 待命，按 `Esc` 会先关面板或触发 Rewind，而不是先清预览选区。 |
| **4. `Ask` 卡片「其他」手写输入框** | `ask.editor().text_selection`（`AskEditor(Lx)`） | 在编辑态选中文字后按 `Esc`（`src/app/update/chat/ask.rs:274`），会直接调用 `cancel_edit()` **连带退出编辑态**，无法仅取消文字选中而继续编辑。 |
| **5. 终端窗格（Terminal Pane）** | `pane.terminal.selection_text()` + `state.terminal_selection` | 按 `Esc` 走 `Routed::Pane(bytes)`（`src/event/input.rs:241-247`）：在清掉终端选区的同时**直接把 `\x1b` 写入了 PTY**，导致用户在终端复制/选词后按 `Esc` 想消掉高亮时误触发终端内程序（如 `vim`、TUI 工具或 shell）的 `Esc`。 |
| **6. 右栏 `Prompt` 正文编辑器** | `state.selection: Option<Selection>` | 1. 排在 `recent` 抽屉、树视图、页列表视图、`@`/`/`/粘贴/快捷引用/块命令面板之后（`src/app/update/editor.rs:44-76`），面板打开时按 `Esc` 先关面板而不清选区；<br>2. 无面板时（`editor.rs:94, 138-141`），按一次 `Esc` 在 `clear_selection(state)` 清掉正文选区后**未返回**，继续落入 `EditorCommand::Escape` 把所有已展开的粘贴芯片（`clear_expanded_pastes()`）一并收起。 |
| **7. 右栏 `Prompt` 展开粘贴芯片预览区** | `state.prompt.preview_selection: Option<(usize, usize)>` | 排在所有 Prompt 子面板之后；面板打开时按 `Esc` 无法先清预览选区。 |
| **8. 右栏 `Prompt` 树/页列表搜索框** | `prompt_panel.search: TextInput`、`prompt_panel.pages_view.search: TextInput` | 搜索框有选区时按 `Esc`（`editor.rs:307, 330`），直接退出树视图/页列表视图回到正文编辑，而不是先取消搜索框内的文字选中。 |
| **9. 右栏 `Todo` 内联编辑框** | `state.todo.edit_input(): Option<&TextInput>` | 编辑框有选区时按 `Esc`（`src/app/update/todo.rs:344`），直接调用 `cancel_todo_edit` 丢弃编辑并退出编辑态，而不是先取消文字选中。 |
| **10. 左栏工作区历史搜索框** | `state.workspace_history.search: TextInput` | 搜索框有选区时按 `Esc`（`src/app/update/workspace_history.rs:899`），直接调用 `exit_workspace_history` 退出历史列表模式，而不是先取消搜索框文字选中。 |
| **11. 9 个含 `TextInput` 的模态浮层（`Overlay`）** | `Rename`、`NewWorkspace`、`WorktreeOpen`、`NewNode`、`SaveDraft`、`ReferencedFolderInput`、`ReferencedFolderSearch`、`ResumeSessionEdit`、`Connect`（输入阶段） | 输入框内有文字选区时按 `Esc`（`src/app/update/dialog.rs:24-147`），直接关闭弹窗或回退上一级弹窗，而不是先取消输入框内的文字选中。 |

此外，`src/app/update/selection.rs` 中的全局清理函数 `clear_selection(state)` 目前漏清理了：
- `state.overlay` 内的 `TextInput` 选区及 `state.overlay_input_drag`；
- `workspace.prompt_panel.search` 与 `workspace.prompt_panel.pages_view.search` 选区及 `state.prompt_search_drag`；
- `state.workspace_history.search` 选区及 `state.workspace_history.search_drag`；
- `pane.lx.task` 内联追问框 `agent_reply` 的 `TextInput` 选区；
- 且浮层鼠标点选（`src/event/overlay.rs:45`）和各容器键盘扩选（`Ctrl/Cmd+Shift+A`、`Shift+方向键`）在启动选区时未清理其他容器的残留选区。

---

## 2. 已确认的边界决策（Grill-Me 闭环）

| 编号 | 决策点 | 已确认方案与行为规范 |
| --- | --- | --- |
| **D1** | **全局最高优先级拦截（消费按键、不穿透）** | 完全对齐 OpenCode：在所有支持文字选中的区域，只要存在**非空文字选区**，按下裸 `Esc`（`KeyCode::Esc` 且 `modifiers.is_empty()`）时**最优先仅清除选区并消费该次按键**，不触发后续任何 `Esc` 动作（不关弹窗、不关 `@`/`/`/块面板、不退编辑态、不退树/页/历史视图、不中断流式生成、不触发 Rewind、不向终端发送 `\x1b`）；若选区为空（如单击未拖出文字），清理残留状态后不消费 `Esc`，继续走原有 `Esc` 逻辑。 |
| **D2** | **可见性判定与全局单选区互斥** | 1. **只响应当前可见选区**：<br>   - 当打开模态弹窗（`state.overlay.is_some()`）时，只检查该弹窗自身的输入框是否有非空选区（绝不让被弹窗挡住的底层窗格旧选区偷偷吃掉用于关弹窗的 `Esc`）；<br>   - 当未打开弹窗时，只检查当前界面上**处于可见状态**的区域（如未折叠的侧栏历史搜索框、未折叠的右栏当前活动 Tab 与当前视图模式、未折叠的主窗格区域中当前活动 Tab 的可见窗格，且排除被后台任务页或 Ask 列表态遮挡的底层隐藏 `lx` 草稿）。<br>2. **全局单选区互斥**：补全 `clear_selection(state)` 覆盖全部选区槽位与拖拽标记；在浮层鼠标点击选区、各输入框 `SelectAll`（`Ctrl/Cmd+Shift+A`）及 `Lx`/`AskEditor` 首次键盘 `Shift+方向键` 扩选时统一清理其他容器选区，保证同一时刻全界面至多存在一处选区。 |
| **D3** | **鼠标按住拖选/自动滚动途中按 `Esc`** | 按下 `Esc` 时，在清除高亮选区的同时**立即终止所有正在进行的鼠标拖选标记与边缘自动滚动状态**（`lx_input_drag`、`overlay_input_drag`、`todo_input_drag`、`prompt_search_drag`、`ask_input_drag`、`terminal_selection`、`workspace_history.search_drag`、`selection_autoscroll`、`lx.preview_drag`、`prompt.preview_drag`）。即使手指尚未松开鼠标左键并继续移动鼠标，也不会重新拉出选区，必须重新按下鼠标左键才开始新的选择。 |
| **D4** | **展开粘贴芯片（Expanded Paste Chip）分步 `Esc`** | 在 `lx` 输入框或右侧 `Prompt` 编辑器中，当存在已展开的长文本粘贴芯片且同时有文字被选中（无论选区在芯片内部 `preview_selection`、输入框正文 `lx.text_selection` / `state.selection`，还是 `lx` 消息区 `state.lx_selection`）时，严格分两步：<br>1. **第一下 `Esc`**：仅清除文字选区，展开的粘贴芯片保持展开状态；<br>2. **第二下 `Esc`**（已无文字选区）：才执行原有的 `clear_expanded_pastes()` 收起展开的粘贴芯片（同时修复 `Prompt` 中原本按一次 `Esc` 会把正文选区和展开芯片一起收起的问题）。 |

---

## 3. 架构设计与优先级链

### 3.1 改造后的全局 `Esc` 优先级链（从高到低）

```text
裸 Esc 按下（KeyCode::Esc, modifiers.is_empty(), kind != Release）
 │
 ├─► [Priority 1 - 新增最高优先级] update::escape_clear_selection(state)
 │     ├─ 若 state.overlay.is_some()：
 │     │    └─ 检查当前浮层可见输入框是否有非空选区：
 │     │         ├─ 有非空选区 → 清除浮层选区与 overlay_input_drag，返回 true（消费 Esc，弹窗不关）
 │     │         └─ 无非空选区 → 清理残留锚点/drag，返回 false（继续走下方浮层 Esc 逻辑）
 │     └─ 若 state.overlay.is_none()：
 │          └─ 检查当前可见区域（侧栏历史搜索框 / 右栏当前视图 / 主区活动 Tab 各可见窗格）是否存在非空选区：
 │               ├─ 存在非空选区 → 调用 clear_selection(state) 清空全部选区与拖拽状态，返回 true（消费 Esc）
 │               └─ 无非空选区   → 清理可能残留的空选区锚点/拖拽状态，返回 false（继续走下方原有 Esc 路由）
 │
 ├─► [Priority 2] 侧栏工作区历史模式（exit_workspace_history）
 ├─► [Priority 3] 模态浮层（apply_overlay_key：Connect / WorktreeOpen / ResumeSessionEdit / Referenced* / Trust / SaveDraft / NewNode / Menu / Rename / NewWorkspace / ConfirmClose）
 ├─► [Priority 4] 右栏 Prompt（recent 抽屉收起 > 树视图退出 > 页列表退出 > 粘贴面板取消 > @ 提及关闭 > 快捷引用关闭 > / 面板关闭 > 块面板关闭 > 收起展开粘贴芯片）
 ├─► [Priority 5] 右栏 Todo（编辑态取消编辑 cancel_todo_edit > 列表态失焦回主区 blur_todo）
 ├─► [Priority 6] 主内容 lx 窗格（后台任务页追问框关闭/任务页关闭 > Ask 编辑态退出/列表态双 Esc 停止 > Rewind 取消选中/待命 > 历史浏览退出 > 文本粘贴/路径粘贴/@/快捷引用/斜杠/块面板关闭 > 收起展开粘贴芯片 > 空 Shell 退出 > 双 Esc 停止生成）
 └─► [Priority 7] 终端窗格（编码 0x1b 写入 PTY）
```

### 3.2 涉及文件与职责边界

所有改动文件均远低于单文件 1000 行上限，保持 `app/`（纯状态与更新）与 `event/`（事件分派）边界清晰：

```text
src/app/
  ├── overlay.rs                      <-- Overlay 新增 clear_selection(&mut self) 与 has_selection(&self) -> bool
  ├── state/
  │   └── todo.rs                     <-- TodoPanel 新增只读访问器 edit_input(&self) -> Option<&TextInput>
  └── update/
      ├── mod.rs (src/app/update.rs)  <-- 重导出 escape_clear_selection 与 has_visible_selection
      ├── selection.rs                <-- [核心] 补全 clear_selection(state)；新增 has_visible_selection(state)
      │                                   与 escape_clear_selection(state) -> bool
      ├── dialog.rs                   <-- apply_overlay_key 入口：OverlayKey::Esc 优先走 escape_clear_selection；
      │                                   OverlayKey::SelectAll 先清理其它容器选区
      ├── editor.rs                   <-- apply_editor 入口：EditorCommand::Escape 优先走 escape_clear_selection
      │                                   并直接返回（修复正文选区与展开芯片被同一次 Esc 一起收起的问题）；
      │                                   EditorCommand::SelectAll 先清理其它容器选区
      ├── pane.rs                     <-- apply_lx_input 入口：LxInput::Escape 优先走 escape_clear_selection；
      │                                   首次键盘扩选（Select*）时清理其它容器选区
      ├── chat/ask.rs                 <-- apply_ask_input 入口：Ask 编辑态首次键盘扩选时清理其它容器选区
      ├── todo.rs                     <-- apply_todo_key 入口：TodoKey::Esc 优先走 escape_clear_selection；
      │                                   TodoKey::SelectAll 先清理其它容器选区
      └── workspace_history.rs        <-- apply_workspace_history_key 入口：OverlayKey::Esc 优先走 escape_clear_selection；
                                          OverlayKey::SelectAll 先清理其它容器选区
src/event/
  ├── input.rs                        <-- handle_key_event 顶部：裸 Esc 最优先调用 update::escape_clear_selection(state)，
  │                                   命中时置 *dirty = true 并直接返回（拦截终端 PTY、recent 抽屉、侧栏历史与全部路由）
  └── overlay.rs                      <-- handle_overlay_click：点击浮层输入框开始选区前调用 update::clear_selection(state)
```

---

## 4. 关键代码设计

### 4.1 `Overlay` 选区辅助方法（`src/app/overlay.rs`）

```rust
impl Overlay {
    /// 当前浮层是否存在可见且非空的文本选区。
    pub fn has_selection(&self) -> bool {
        self.input().is_some_and(TextInput::has_selection)
    }

    /// 清除当前浮层内单行输入框的文本选区（不受 validating 状态限制）。
    pub fn clear_selection(&mut self) {
        match self {
            Self::Rename(r) => r.input.clear_selection(),
            Self::NewWorkspace(ws) => ws.input.clear_selection(),
            Self::WorktreeOpen(w) => w.query.clear_selection(),
            Self::NewNode(n) => n.name.clear_selection(),
            Self::SaveDraft(d) => d.name.clear_selection(),
            Self::ReferencedFolderInput(i) => i.input.clear_selection(),
            Self::ReferencedFolderSearch(s) => s.query.clear_selection(),
            Self::ResumeSessionEdit(r) => r.input.clear_selection(),
            Self::Connect(panel) => panel.input.clear_selection(),
            _ => {}
        }
    }
}
```

### 4.2 `TodoPanel` 只读输入框访问器（`src/app/state/todo.rs`）

```rust
impl TodoPanel {
    /// 编辑态下的输入框只读引用；非编辑态返回 `None`。
    pub fn edit_input(&self) -> Option<&TextInput> {
        self.editing.as_ref().map(|edit| &edit.input)
    }

    /// 编辑态下的输入框可变引用；非编辑态返回 `None`。
    pub fn edit_input_mut(&mut self) -> Option<&mut TextInput> {
        self.editing.as_mut().map(|edit| &mut edit.input)
    }
}
```

### 4.3 全局选区清理、可见选区判定与 `escape_clear_selection`（`src/app/update/selection.rs`）

```rust
/// 清除全应用所有容器的文本选区以及正在进行的鼠标拖选/自动滚动状态。
pub fn clear_selection(state: &mut AppState) {
    state.selection = None;
    state.selection_autoscroll = None;
    state.lx_selection = None;
    state.lx_card_press = None;
    state.lx_scroll_drag = None;
    state.lx_history_scroll_drag = None;
    state.task_agents_scroll_drag = None;
    state.lx_input_drag = None;
    state.overlay_input_drag = false;
    state.ask_input_drag = None;
    state.todo_input_drag = false;
    state.prompt_search_drag = false;
    state.workspace_history.search_drag = false;
    state.workspace_history.search.clear_selection();
    if let Some(overlay) = state.overlay.as_mut() {
        overlay.clear_selection();
    }
    state.prompt.clear_preview_selection();
    if let Some(input) = state.todo.edit_input_mut() {
        input.clear_selection();
    }
    for workspace in &mut state.workspaces {
        workspace.prompt_panel.search.clear_selection();
        workspace.prompt_panel.pages_view.search.clear_selection();
        for tab in &mut workspace.tabs {
            for id in tab.layout.pane_ids() {
                if let Some(target) = tab.pane_mut(id) {
                    target.lx.clear_text_selection();
                    target.lx.clear_preview_selection();
                    if let Some(ask) = target.lx.ask.as_mut()
                        && let Some(editor) = ask.editor_mut()
                    {
                        editor.clear_text_selection();
                    }
                    if let Some(task) = target.lx.task.as_mut()
                        && let Some(reply) = task.agent_reply.as_mut()
                    {
                        reply.clear_selection();
                    }
                }
            }
        }
    }
    clear_terminal_selection(state);
}

/// 当前界面是否存在对用户可见的非空文本选区。
///
/// 规则（对齐 Grill-Me D1/D2）：
/// 1. 模态浮层打开时，仅检查该浮层自身的输入框选区；被浮层遮挡的底层容器选区不计入。
/// 2. 无模态浮层时，按各容器可见性检查：
///    - 左栏：未折叠且处于工作区历史列表模式时，检查 `workspace_history.search`；
///    - 右栏：未折叠时，按 `right_tab`（`Prompt` 树/页/正文与展开芯片，或 `Todo` 编辑框）检查；
///    - 主窗格区：未折叠时，遍历当前活动标签页中的所有可见窗格：
///      - `PaneView::Lx`：检查主内容区 `state.lx_selection`；若后台任务页打开则检查 `agent_reply`；
///        若 `ask` 处于编辑态则检查 `ask.editor()`；若 `task` 与 `ask` 均未遮挡输入框，则检查
///        `lx.preview_selection()` 与 `lx.text_selection_range()`；
///      - `PaneView::Terminal`：检查 `terminal_selection_text(state, id)`。
pub fn has_visible_selection(state: &AppState) -> bool {
    if let Some(overlay) = state.overlay.as_ref() {
        return overlay.has_selection();
    }

    // 1. 左侧栏工作区历史搜索框
    if !state.sidebar_collapsed
        && super::workspace_history_mode(state)
        && state.workspace_history.search.has_selection()
    {
        return true;
    }

    // 2. 右侧栏（Prompt / Todo）
    if !state.prompt_collapsed {
        match state.right_tab {
            crate::app::state::RightTab::Prompt => {
                if super::prompt_tree_mode(state) {
                    if state.prompt_owner().prompt_panel.search.has_selection() {
                        return true;
                    }
                } else if super::prompt_pages_mode(state) {
                    if state
                        .prompt_owner()
                        .prompt_panel
                        .pages_view
                        .search
                        .has_selection()
                    {
                        return true;
                    }
                } else if state.prompt.preview_selection().is_some()
                    || prompt_selection_bounds(state).is_some()
                {
                    return true;
                }
            }
            crate::app::state::RightTab::Todo => {
                if state
                    .todo
                    .edit_input()
                    .is_some_and(|input| input.has_selection())
                {
                    return true;
                }
            }
        }
    }

    // 3. 主窗格区（当前活动标签下的所有可见窗格）
    if !state.panes_collapsed {
        for id in state.active_tab().layout.pane_ids() {
            let Some(pane) = state.pane_anywhere(id) else {
                continue;
            };
            match pane.view {
                crate::app::state::PaneView::Lx => {
                    if state
                        .lx_selection
                        .is_some_and(|sel| sel.pane() == id && sel.range().is_some())
                    {
                        return true;
                    }
                    if let Some(task) = pane.lx.task.as_ref() {
                        if task
                            .agent_reply
                            .as_ref()
                            .is_some_and(|reply| reply.has_selection())
                        {
                            return true;
                        }
                    } else if let Some(ask) = pane.lx.ask.as_ref() {
                        if ask.editing
                            && ask
                                .editor()
                                .is_some_and(|ed| ed.text_selection_range().is_some())
                        {
                            return true;
                        }
                    } else if pane.lx.preview_selection().is_some()
                        || pane.lx.text_selection_range().is_some()
                    {
                        return true;
                    }
                }
                crate::app::state::PaneView::Terminal => {
                    if terminal_selection_text(state, id).is_some() {
                        return true;
                    }
                }
            }
        }
    }

    false
}

/// 按下 `Esc` 时尝试取消当前可见的文字选区（对齐 OpenCode `handleSelectionKey` 优先级 101）。
///
/// - 若存在可见的非空文本选区：清除选区并终止所有拖选/自动滚动状态，返回 `true`（消费该次 `Esc`）。
/// - 若不存在可见的非空文本选区：清理可能残留的空选区锚点与拖选状态，返回 `false`（不消费 `Esc`，继续后续路由）。
pub fn escape_clear_selection(state: &mut AppState) -> bool {
    let had_selection = has_visible_selection(state);
    if state.overlay.is_some() {
        state.overlay_input_drag = false;
        if let Some(overlay) = state.overlay.as_mut() {
            overlay.clear_selection();
        }
        return had_selection;
    }
    if had_selection || has_any_selection_or_drag(state) {
        clear_selection(state);
    }
    had_selection
}

/// 是否存在任何残留的空选区锚点或正在进行的文本拖选标记（用于无非空文本时的静默清理）。
fn has_any_selection_or_drag(state: &AppState) -> bool {
    state.selection.is_some()
        || state.selection_autoscroll.is_some()
        || state.lx_selection.is_some()
        || state.terminal_selection.is_some()
        || state.lx_input_drag.is_some()
        || state.overlay_input_drag
        || state.ask_input_drag.is_some()
        || state.todo_input_drag
        || state.prompt_search_drag
        || state.workspace_history.search_drag
}
```

### 4.4 事件层最高优先级拦截（`src/event/input.rs`）

在 `handle_key_event` 入口（紧随 `KeyEventKind::Release` 过滤之后，早于 `workspace_history`、`input::route`、模态浮层、Prompt recent 抽屉以及终端 `Routed::Pane` 转发）：

```rust
pub(crate) fn handle_key_event(
    key: KeyEvent,
    state: &mut AppState,
    sessions: &mut HashMap<PaneId, PtySession>,
    geometry: &Geometry,
    cache: &mut ui::RenderCache,
    dirty: &mut bool,
) {
    let view = &geometry.view;
    if key.kind == KeyEventKind::Release {
        return;
    }

    // 对齐 OpenCode (priority 101)：裸 Esc 最优先取消当前可见文字选区并消费按键，
    // 绝不穿透到终端 PTY、模态浮层关闭、自动补全面板关闭、退出编辑态或双 Esc 中断。
    if key.code == KeyCode::Esc
        && key.modifiers.is_empty()
        && update::escape_clear_selection(state)
    {
        *dirty = true;
        return;
    }
```

### 4.5 Update 层直接调用入口同步接入与 Prompt 展开芯片修复

为保证直接调用 `update::apply_*` 的单元测试与事件层具备完全一致的 `Esc` 优先级语义，同时修复 `Prompt` 正文选区与展开芯片被同一次 `Esc` 一并收起的问题：

1. **`src/app/update/editor.rs`（`apply_editor`）**：
   ```rust
   pub fn apply_editor(state: &mut AppState, command: EditorCommand) {
       if command == EditorCommand::Escape && super::escape_clear_selection(state) {
           return;
       }
   ```
   - 当 `Prompt` 正文选区（`state.selection`）、展开粘贴芯片预览选区（`preview_selection`）或树/页搜索框选区非空时，`EditorCommand::Escape` 在入口即清除选区并返回，**不会再穿透到底部 `state.prompt.clear_expanded_pastes()` 或关闭面板/退出树与页视图**；
   - 当无任何选区时，才走原有的面板关闭、树/页退出或 `state.prompt.clear_expanded_pastes()`。

2. **`src/app/update/pane.rs`（`apply_lx_input`）**：
   ```rust
   pub fn apply_lx_input(state: &mut AppState, command: LxInput) {
       if command == LxInput::Escape && super::escape_clear_selection(state) {
           return;
       }
   ```
   - 确保直接调用 `apply_lx_input(state, LxInput::Escape)` 时，`lx` 主内容选区（`state.lx_selection`）、`lx` 输入框选区（`lx.text_selection`）、展开粘贴预览选区（`lx.preview_selection`）、`Ask` 手写框选区（`ask.editor()`）均最优先被清除并消费 `Escape`。

3. **`src/app/update/dialog.rs`（`apply_overlay_key`）、`src/app/update/todo.rs`（`apply_todo_key`）、`src/app/update/workspace_history.rs`（`apply_workspace_history_key`）**：
   - 分别在收到 `OverlayKey::Esc` / `TodoKey::Esc` 时优先检查 `if super::escape_clear_selection(state) { return ...; }`。

---

## 5. 测试与验证计划

1. **全场景单元测试与事件级测试**：
   - **终端窗格**：终端存在非空选区时按 `Esc`，选区被清除且不向 PTY 发送 `\x1b`；终端选区为空（仅单击）时按 `Esc`，`\x1b` 正常透传给 PTY。
   - **`lx` 主内容区与输入框**：
     - `state.lx_selection` 非空且流式生成中（或开着 `@`/`/` 面板、空 Shell 模式、Rewind 可待命）时按第一下 `Esc`：仅清除 `state.lx_selection`，面板不关、不退出 Shell、不进入停止/Rewind 待命；按第二下 `Esc` 才执行原操作；
     - `lx.text_selection`（通过鼠标或 `Shift+←` / `Ctrl+Shift+A` 选中）非空且开着 `@`/`/` 面板或流式生成中时按第一下 `Esc`：仅清除 `lx.text_selection`；
     - `lx` 展开粘贴芯片：芯片展开且存在 `preview_selection` 或 `lx.text_selection` 时，第一下 `Esc` 仅清选区（芯片保持展开），第二下 `Esc` 才收起展开芯片。
   - **`Ask` 手写输入框**：在 `Ask` 编辑态选中文本后按第一下 `Esc`，仅清除手写框选区且保持 `ask.editing == true`；按第二下 `Esc` 才退出编辑态回到选项列表。
   - **右侧 `Prompt` 面板**：
     - 正文有选区且存在已展开粘贴芯片（或打开了 `@`/`/` 面板）时：第一下 `Esc` 仅清正文选区，展开芯片与面板保持打开；第二下 `Esc` 才关闭面板或收起展开芯片；
     - 树视图 / 页列表视图顶部搜索框有选区时：第一下 `Esc` 仅清除搜索框选区，保持在树/页列表视图；第二下 `Esc` 才返回编辑视图。
   - **右侧 `Todo` 编辑框**：编辑态输入框有选区时，第一下 `Esc` 仅清除选区，保持编辑态与已输入内容；第二下 `Esc` 才取消编辑。
   - **左侧栏工作区历史搜索框**：搜索框有选区时，第一下 `Esc` 仅清除搜索框选区，保持在历史列表模式；第二下 `Esc` 才退出历史列表模式。
   - **模态浮层（`Overlay`）**：
     - 浮层输入框（如 `Rename`、`WorktreeOpen`、`Connect` API Key 等）有选区时，第一下 `Esc` 仅清除浮层输入框选区，浮层不关闭；第二下 `Esc` 才关闭/回退浮层；
     - 底层窗格有残留选区但当前浮层无选区时，按第一下 `Esc` 直接关闭/回退浮层（不被底层被遮挡选区误吞）。
   - **鼠标按住拖选途中按 `Esc`**：拖选中按 `Esc` 立即清空选区并重置 `*_drag` 与 `selection_autoscroll`，后续继续触发 `MouseEventKind::Drag` 不会重新拉出选区。
2. **精确静态检查**：
   - `rtk cargo fmt --check`
   - `rtk cargo clippy --all-targets -- -D warnings`
   - 针对受影响模块执行精准 `rtk cargo test`。
