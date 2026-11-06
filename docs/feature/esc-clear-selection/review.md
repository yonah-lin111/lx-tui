# 全局 `Esc` 取消文字选中与优先级功能代码评审（Review）

评审范围：`.worktrees/esc-clear-selection`（分支 `feature/esc-clear-selection`，对比 `dev`）。
评审方式：结合 `codegraph`、`codebase-memory-mcp` 与 3 个并行 `general` 子 Agent，对全界面 11 类可选区容器的**可见性判定 vs 实际渲染同源性**、**全局单选区互斥**、**`Ctrl+C` / `Ctrl+X` 复制剪切闭环**、**鼠标拖拽与自动滚动中止**以及**按键路由优先级**进行全链路交叉核验。

---

## 问题分级总览（按 P0 → P3 排序）

| 级别 | 编号 | 模块 / 文件 | 关键点摘要 | 状态 |
| :--- | :--- | :--- | :--- | :--- |
| **P0** | **P0-1** | `src/ui/main_content/lx/task.rs`<br>`src/event/input.rs`<br>`src/app/update/pane/task_page.rs` | 后台任务页内联追问框（`TaskPage.agent_reply`）支持 `SelectAll` 并计入 `has_visible_selection`，但 UI 从未渲染选区高亮且不支持 `Ctrl+C` / `Ctrl+X` 复制剪切，导致“隐形选区”吞掉第一次 `Esc`。 | 已修复 |
| **P1** | **P1-1** | `src/terminal/mod.rs`<br>`src/app/update/selection.rs`<br>`src/app/update/editor.rs` | 终端与 `Prompt` 正文的选区可见性判定使用“非空文本/字节”而非“渲染高亮范围”，导致空白/空行/行尾选区在屏幕上可见反显，按 `Esc` 却不消费按键（穿透写入 PTY 或连带收起展开粘贴芯片）。 | 已修复 |
| **P1** | **P1-2** | `src/app/update/menu.rs`<br>`src/app/update/prompt/pages.rs` | 右键菜单 `MenuCommand::SelectAll` 与 `rename_prompt_page_title` 程序化选区未调用 `clear_selection(state)`，绕过全局单选区互斥，导致多容器选区高亮并存。 | 已修复 |
| **P2** | **P2-1** | `src/app/update/selection.rs` | `escape_clear_selection` 在无可见选区（`had_selection == false`）时仍无条件执行全局 `clear_selection(state)`，误清后台标签页/折叠容器保留的静态选区；且浮层分支未终止底层残留拖选与自动滚动，未重置多击计数 `selection_click`。 | 已修复 |
| **P2** | **P2-2** | `src/app/state/lx/ask.rs`<br>`src/app/update/selection.rs` | `AskState` 多问题面板在 `clear_selection` 时仅清理当前激活题目的 `editor_mut()`，未清理其余题目槽位中可能残留的手写框选区。 | 已修复 |
| **P3** | **P3-1** | `src/event/overlay.rs`<br>`src/event/input.rs` | `handle_overlay_click` 存在无用变量绑定 `let _ = area;`；`overlay_selected_text` 重复手写 9 分支 match，未复用 `Overlay::input()`。 | 已修复 |
| **P3** | **P3-2** | `src/app/update/selection/tests.rs`<br>`src/event/tests/terminal.rs` | 缺少针对 `agent_reply` 选区渲染与复制、终端/Prompt 空白选区 `Esc` 拦截、右键菜单 `SelectAll` / 页面重命名单选区互斥、`Overlay::Menu` 右键菜单两步 `Esc` 及 `selection_click` 重置的回归测试。 | 已修复 |

---

## 详细问题分析与错误原因

### P0-1：后台任务页内联追问框（`TaskPage.agent_reply`）存在“隐形选区”吞掉 `Esc` 且不可复制/剪切

- **涉及位置**：
  - `src/app/update/pane/task_page.rs:112-115`
  - `src/app/update/selection.rs:444-451`
  - `src/ui/main_content/lx/task.rs:499-527`（`draw_agent_reply`）
  - `src/event/input.rs:424-441`（`copy_lx_selection`）、`465-518`（`cut_lx_selection`）
- **关键点**：
  1. 本分支在 `apply_agent_reply_input` 中新增了 `LxInput::SelectAll => reply.select_all()`，并在 `has_visible_selection` 中将 `task.agent_reply.has_selection()` 判定为可见选区；
  2. 但 `draw_agent_reply` 在绘制追问框文本时只判断了 `if index == cursor`，完全没有读取 `reply.selection()` 绘制反显；
  3. 同时 `copy_lx_selection` 与 `cut_lx_selection` 在 `task_open`（`target.lx.task.is_some()`）时直接跳过了输入框复制/剪切分支。
- **错误原因**：
  只把 `TaskPage.agent_reply` 接入了“全选 + `Esc` 判可见选区清理”半条链路，遗漏了 UI 渲染层的 `reply.selection()` 反显绘制以及事件层 `Ctrl+C` / `Ctrl+X` 的复制与剪切支持。结果用户在追问框按 `Ctrl/Cmd+Shift+A` 后界面毫无反色变化，按第一次 `Esc` 被“看不见的选区”吞掉（追问框不关），再打字还会静默整段替换。
- **修复措施**：
  1. 在 `src/ui/main_content/lx/task.rs` 的 `draw_agent_reply` 中读取 `reply.selection()`，当字符索引落在 `[start, end)` 内时附加 `Modifier::REVERSED`；
  2. 在 `src/event/input.rs` 的 `copy_lx_selection` 与 `cut_lx_selection` 中支持 `task.agent_reply` 的选区复制（`selected_text()`）与剪切（`cut_selected()`）。

---

### P1-1：终端与 `Prompt` 正文的选区可见性判定与 UI 渲染层不同源（空白/行尾选区穿透）

- **涉及位置**：
  - `src/terminal/mod.rs:318-323`
  - `src/ui/main_content.rs:165-170`
  - `src/app/update/selection.rs:413-415, 466-470, 710-732`
  - `src/ui/prompt.rs:688-696`
  - `src/app/update/editor.rs:80-98`
- **关键点**：
  1. **终端窗格**：UI 渲染层（`src/ui/main_content.rs:165-170`）按 `content.selection.contains(indexed.point)` 对选区内的所有单元格（含空白格）渲染 `Modifier::REVERSED` 反显；而 `has_visible_selection` 调用 `terminal_selection_text(state, id)`，其内部调用 `self.term.selection_to_string().filter(|text| !text.trim().is_empty())`。这带来两个问题：
     - 拖选终端空白区域（尚未松开或选中纯空白格）时，屏幕上有明确的反显块，但 `terminal_selection_text` 返回 `None`，按 `Esc` 既清除了反显又把 `\x1b` 发送给了 PTY；
     - 每次按 `Esc` 都会将整个终端选区（可能跨数千行回滚）格式化并分配为 `String`，仅用于判断 `.is_some()`。
  2. **Prompt 正文**：UI 渲染层（`src/ui/prompt.rs:688-696` `paint_selection`）与鼠标松开保留逻辑（`end_selection_drag`）均以 `Selection::range().is_some()`（即 `prompt_selection_range(state).is_some()`）为准，行尾空白、空行与块顶边框行都会被反显保留；但 `has_visible_selection` 却使用了 `prompt_selection_bounds(state).is_some()`（要求映射出的源码字节范围 `start < end`）。当用户在空行或行尾空白拖出反显选区后按 `Esc`，`prompt_selection_bounds` 返回 `None`，导致第一次 `Esc` 在清除反显的同时直接收起已展开粘贴芯片或关闭面板，违反 D4 两步 `Esc` 契约。
- **错误原因**：
  将“剪贴板可复制出非空白字符串”的复制语义误当作“屏幕上存在可见选区高亮”的几何渲染语义。
- **修复措施**：
  1. 在 `src/terminal/mod.rs` 的 `Terminal` 上新增零分配方法 `has_selection(&self) -> bool`（通过 `self.term.selection.as_ref().and_then(|s| s.to_range(&self.term)).is_some()` 判定，与 `renderable_content().selection` 完全同源：单击未拖动时为 `false`，拖出反显时为 `true`），并在 `src/app/update/selection.rs` 新增 `terminal_has_selection(state, pane)` 供 `has_visible_selection` 调用；
  2. 在 `has_visible_selection` 中将 `prompt_selection_bounds(state).is_some()` 替换为 `prompt_selection_range(state).is_some()`；并在 `src/app/update/editor.rs` 中补充当 `prompt_selection_range(state).is_some()`（即使字节区间为空的纯空白/行尾选区）收到普通编辑/移动键时同样清理选区。

---

### P1-2：右键菜单 `Select All` 与 `rename_prompt_page_title` 程序化选区遗漏全局单选区互斥

- **涉及位置**：
  - `src/app/update/menu.rs:566-569`
  - `src/app/update/prompt/pages.rs:410, 425-429`
- **关键点**：
  1. `src/app/update/menu.rs` 在处理右键菜单 `(Some(MenuCommand::SelectAll), OverlayTarget::PromptPage { .. })` 时直接调用 `state.select_all_prompt()`，未先调用 `clear_selection(state)`；
  2. `src/app/update/prompt/pages.rs` 的 `rename_prompt_page_title` 在通过 `Selection::programmatic_range` 选中页面标题前仅执行了 `state.selection = None`，未清理其他容器选区或当前页展开粘贴芯片的 `preview_selection`。
- **错误原因**：
  单选区互斥清理仅加在了键盘 `EditorCommand::SelectAll` 入口，遗漏了右键菜单触发的 `SelectAll` 与 `RenamePage` 程序化选区入口。
- **修复措施**：
  1. 在 `src/app/update/menu.rs` 的 `MenuCommand::SelectAll` 分支先调用 `clear_selection(state)`；
  2. 在 `src/app/update/prompt/pages.rs` 的 `rename_prompt_page_title` 中将 `state.selection = None;` 替换为 `crate::app::update::clear_selection(state);`。

---

### P2-1：`escape_clear_selection` 在无可见选区时过度清理后台静态选区，且浮层分支未终止残留拖拽与多击计数

- **涉及位置**：
  - `src/app/update/selection.rs:327-370, 482-493`
- **关键点**：
  1. 当 `had_selection == false`（当前可见区域无任何选区）时，`escape_clear_selection` 仍无条件执行 `clear_selection(state)`，导致用户在当前无选区的窗格按 `Esc`（例如在 Tab 2 的终端 vim 中按 `Esc`）时，后台 Tab 1 或已折叠面板中保留的选区被静默清空，且与 `design.md` 的设计意图不符；
  2. 当 `state.overlay.is_some()` 时，`escape_clear_selection` 提前返回，仅清理了 `state.overlay_input_drag = false`，未终止底层可能残留的 `selection_autoscroll` 与各容器 `*_drag` 标记；
  3. `clear_selection` 未重置 `state.selection_click = None`，双击选词后立即按 `Esc` 取消选区并在多击时间窗（400ms）内再次单击同一位置，会被误判为三击选整行。
- **错误原因**：
  未将“终止进行中的拖拽/自动滚动与空锚点（`stop_active_selection_drags`）”和“清除全应用已成立选区（`clear_selection`）”拆分开。
- **修复措施**：
  1. 在 `src/app/update/selection.rs` 中抽取 `stop_active_selection_drags(state: &mut AppState)`：重置 `selection_autoscroll`、`selection_click`、`lx_card_press` 及全部 `*_drag` 标记，并收尾进行中的拖拽（空锚点直接清除，已成立的非空选区仅结束 `dragging` 态而不销毁）；
  2. 在 `escape_clear_selection(state)` 中：
     - 先调用 `stop_active_selection_drags(state)` 确保任何情况下的拖拽、自动滚动与空锚点都被立即终止；
     - 若 `state.overlay.is_some()`：若 `had_selection` 为真则调用 `overlay.clear_selection()`，返回 `had_selection`（保留底层被遮挡选区）；
     - 若 `state.overlay.is_none()`：仅当 `had_selection` 为真时调用 `clear_selection(state)`，否则保持后台 Tab/折叠容器的静态选区不被无故清空。

---

### P2-2：`Ask` 多问题面板在 `clear_selection` 时未清理非当前题目的手写框选区

- **涉及位置**：
  - `src/app/state/lx/ask.rs:482-490`
  - `src/app/update/selection.rs:355-359`
- **关键点**：
  `AskState::custom` 按题目下标持有多个 `AskEditor`，`ask.editor_mut()` 仅返回 `self.custom.get_mut(self.current)`。全局 `clear_selection` 只清理了当前题手写框，未遍历 `custom` 清理所有题目槽位。
- **错误原因**：
  忽略了多问题 `Ask` 卡片包含多个 `AskEditor` 实例。
- **修复措施**：
  在 `AskState` 上新增 `pub fn clear_all_text_selections(&mut self)` 遍历 `self.custom` 清理所有题目的选区，并在 `clear_selection(state)` 中调用。

---

### P3-1：`src/event/overlay.rs` 与 `src/event/input.rs` 冗余代码清理

- **涉及位置**：
  - `src/event/overlay.rs:26-57`
  - `src/event/input.rs:521-534`
- **关键点**：
  1. `handle_overlay_click` 中闭包返回 `(area, char_idx)` 后又写 `let _ = area;`，属于重构残留；
  2. `overlay_selected_text` 手写了 9 个 `Overlay` 变体的 match，与 `Overlay::input()` 重复。
- **修复措施**：
  1. `handle_overlay_click` 闭包直接返回 `char_idx`；
  2. `overlay_selected_text` 简化为 `state.overlay.as_ref()?.input()?.selected_text()`。

---

### P3-2：回归测试补齐

- **涉及位置**：
  - `src/app/update/selection/tests.rs`
  - `src/event/tests/terminal.rs`
- **新增/扩充测试点**：
  1. `TaskPage.agent_reply` 全选后的 UI 渲染（验证 `Modifier::REVERSED` 高亮）、`Ctrl+C` 复制、`Ctrl+X` 剪切与 `Esc` 清除；
  2. 终端纯空白拖选按 `Esc` 消费按键且不向 PTY 写入 `\x1b`；
  3. `Prompt` 空行/行尾空白拖选按 `Esc` 优先清除反显且不收起已展开粘贴芯片；
  4. 右键菜单 `MenuCommand::SelectAll` 与 `rename_prompt_page_title` 触发时自动清除其他容器选区；
  5. `Overlay::Menu` 右键菜单打开且底层存在选区时，第一下 `Esc` 仅关闭右键菜单并保留底层选区，第二下 `Esc` 才清除底层选区；
  6. 后台 Tab 存在保留选区、当前活动 Tab 无选区时，按 `Esc` 不销毁后台 Tab 选区；双击选词后按 `Esc` 重置 `selection_click` 计数。
