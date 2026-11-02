# Todo 面板折行缩进、后台任务回到底部与 Ask 交互重构代码审查 (Code Review)

> **分支**：`feat/todo-task-and-ask-ux`（worktree `.worktrees/todo-task-and-ask-ux`）
> **关联文档**：`docs/feature/todo-task-and-ask-ux/design.md`、`docs/feature/todo-task-and-ask-ux/task.md`

本次 Review 覆盖工作区分支 `feat/todo-task-and-ask-ux` 引入的全部改动（右侧面板 Todo 条目 1~3 行折行缩进与行内编辑、后台任务面板 `Processes · Agents` 回到底部按钮、`ask` 面板全宽折行/原地 `Other...` 编辑/统一滚动条/`opencode-dev` 键盘交互对齐及等待期暂停计时）。

审查发现的问题按照严重级别从高到低（`P0` → `P1` → `P2`）归类如下，明确记录**关键点**、**错误原因**与**修复方案**。

---

## 缺陷清单与优先级排序

### [P0-1] `TodoPanel` 滚动、视口钳制与滚动条混用「逻辑条目数」与「屏幕视觉行数」，导致多行展开后底部待办被截断或无法滚出

- **关键点**：
  - `src/app/state/todo.rs`：`TodoPanel::set_viewport_rows`、`TodoPanel::set_scroll`、`TodoPanel::scroll_to_edit`、`TodoPanel::scroll_into_view`
  - `src/app/update/todo.rs`：`resolve_todo_viewport`
  - `src/ui/right_panel/todo.rs`：`layout_visible_items`、`scrollbar`、`render_edit_item`
- **错误原因**：
  1. `resolve_todo_viewport` 在每帧绘制前将 `visible_rows_for(panel, state)`（即列表区的**屏幕物理行数** `list.height`）写入 `TodoPanel.rows`。
  2. 但 `TodoPanel.scroll`、`selected`、`items.len()`、`display_len()` 的单位全部是**逻辑条目下标**，而每个条目在折行后实际占用 `1..=3` 个屏幕视觉行：
     - **键盘 `↑`/`↓` 导航失灵与底部裁剪**：`scroll_into_view(rows)` 按 `selected >= self.scroll + rows` 与 `max = self.items.len().saturating_sub(rows)` 计算。例如列表区高 `6` 行，存在 `4` 条各占 `3` 行的待办（共 `12` 个视觉行）：视口仅能容纳条目 `0` 和 `1`（占满 `6` 行），当按 `↓` 选中条目 `2` 或 `3` 时，因 `selected (3) < scroll (0) + rows (6)` 且 `max = 4.saturating_sub(6) = 0`，`scroll` 恒停在 `0`，条目 `2`、`3` 落在 `list.bottom()` 之外完全不可见。
     - **滚轮与滚动条失效**：`set_scroll` 与 `set_viewport_rows` 将最大滚动偏移钳死在 `display_len().saturating_sub(self.rows)`；`todo::scrollbar` 传入 `total = state.todo.display_len()`、`visible = visible_rows_for(panel, state)`。当 `4` 条 `3` 行待办放入 `6` 行高的列表时，`total (4) <= visible (6)`，既不渲染滚动条，滚轮也因 `max == 0` 无法滚动。
     - **底部行内编辑展开被截断且光标错位**：在视口底部条目进入编辑并输入到第 2~3 行时，`scroll_to_edit` 不会因为条目高度扩至 2~3 行而上推 `scroll`；`layout_visible_items` 将 `rendered_rows` 裁剪为 `1`，而 `render_edit_item` 将落入第 2/3 行的 `cursor_y` 强行 `.min(list.bottom() - 1)` 拍回第 1 行，导致硬件光标叠在首行文字上。
- **修复方案**：
  - 在 `src/ui/right_panel/todo.rs` 中根据当前面板宽度与各条目折行高度（`item_rows`）统一计算视口窗口，并由 `resolve_todo_viewport` 在每帧绘制前同步进 `TodoPanel`：
    1. 计算能完整显示底部最后一个条目的最大起始条目下标 `max_scroll`（从末项向上累加 `item_rows` 不超过 `list.height` 的最靠前条目下标）以及总视觉行数；
    2. 当存在焦点条目（正在编辑的 `edit` 条目优先，其次 `selected` 条目）且键盘导航移动或正在编辑时，若该条目的任意视觉行超出 `[scroll, visible_end)`，自动调整 `scroll` 使其全部 `1..=3` 个视觉行完整落入视口内；
    3. `todo::scrollbar` 基于总视觉行数（或与 `max_scroll` 对齐的视觉偏移）渲染滚动条，确保多行展开溢出时滚动条、滚轮与 `↑`/`↓` 键盘导航三者口径一致。

---

### [P0-2] `Ask` 面板高亮停在 `Other...`（非编辑态）键入数字 `1..9` 时误触发选项快捷选择并直接提交，无法以数字开头直接输入自定义答案

- **关键点**：
  - `src/app/update/chat/ask.rs`：`type_key`（第 `91–137` 行）
  - 对照 `opencode-dev/packages/tui/src/routes/session/form.tsx`：`keymap.intercept`（第 `243–255` 行）
- **错误原因**：
  1. 在 `opencode-dev` 中，当焦点停在 `Other...`（`other() == true`）时，`keymap.intercept` 优先于数字快捷键层拦截**所有非空格可打印字符**（包含 `'1'..'9'`），直接追加进 `custom` 并置 `editing = true`。
  2. 但在 `src/app/update/chat/ask.rs::type_key` 中，条件写成了 `if on_other && !('1'..='9').contains(&ch)`：
     - 用户将高亮移到 `Other...` 准备输入数字开头的自定义答案（如 `"100ms"`、`"2 columns"`、`"3"`）时，按下首字符 `'1'` 会跳过 `on_other` 原地编辑分支，直接落入数字快捷选择（`index = 0`），把高亮强行移回第 1 个选项并调用 `enter(state, pane)`；在单选题下会**瞬间以第 1 个选项提交整题并关闭面板**。
     - 若按下大于选项数的数字（如 2 个选项时在 `Other...` 上按 `'8'`），跳过编辑分支后因 `index > ask.other_row()` 直接返回 `false`，字符 `'8'` 被静默吞掉。
- **修复方案**：
  - 在 `src/app/update/chat/ask.rs::type_key` 中移除 `&& !('1'..='9').contains(&ch)`，改为 `if on_other { ... }`：当高亮停在 `Other...` 行时，任何非控制字符一律原地进入行内编辑并写入首字符，仅当高亮停在普通选项行时 `1..9` 才作为选项快捷键。

---

### [P0-3] `Ask` 面板行内 `Other...` 输入框的软折行宽度（`AskEditor.width`）与实际渲染宽度（`content_w`）不同源，导致行尾 1~3 列字符被裁切与折行跳变

- **关键点**：
  - `src/layout.rs`：`LX_ASK_EDITOR_INSET`、`LX_ASK_EDITOR_ROWS`、`lx_ask_editor_size`
  - `src/event/chat/ask.rs`：`request`
  - `src/ui/main_content/lx/ask.rs`：`draw_other`、`desc_width`、`place_rows`、`list_window`
  - `src/app/update/chat/viewport.rs`：`resolve_ask_list_window`
- **错误原因**：
  1. `src/ui/main_content/lx/ask.rs` 中行内 `Other...` 的真实正文列宽 `content_w` 为 `row.rect.width - ROW_PREFIX(6)`（内容溢出出现右侧滚动条时 `row.rect.width` 再减 1 列）。
  2. 但 `src/layout.rs` 仍保留了旧版底部独立编辑框的 `LX_ASK_EDITOR_INSET = 4` 与 `LX_ASK_EDITOR_ROWS = 5`；且当列表溢出出现滚动条时，`place_rows` 将行宽缩窄 1 列，但未同步更新 `AskEditor` 的内部折行宽度。
  3. 导致编辑态下 `AskEditor::visual_rows()` 按更宽的列数（多出 2~3 列）计算软折行，而 `draw_other` 按 `content_w` 裁剪绘制，**每行末尾的 2~3 个字符在编辑态下被直接裁掉不可见**，硬件光标卡在最右格；按 `Enter` 提交退出编辑态后改用 `wrap(&text, desc_width)` 按真实宽度折行，文本折行位置与行数发生跳变。
- **修复方案**：
  1. 将 `src/layout.rs` 中的 `LX_ASK_EDITOR_INSET` 更新为 `6`（对齐 `ROW_PREFIX`）、`LX_ASK_EDITOR_ROWS` 更新为 `3`（对齐 `AskState::MAX_OTHER_EDIT_ROWS`）；修正 `src/event/chat/ask.rs::request` 的初始宽度计算；
  2. 在 `src/ui/main_content/lx/ask.rs::list_window` 与 `src/app/update/chat/viewport.rs::resolve_ask_list_window` 每帧绘制前同步列表视口时，一并将扣减滚动条列后的真实 `editor_w = row_width.saturating_sub(ROW_PREFIX).max(1)` 同步给 `ask.resize_editor(editor_w, AskState::MAX_OTHER_EDIT_ROWS as u16)`，确保折行与渲染逐列同源。

---

### [P0-4] `TaskTab::Agents` 详情在收起卡片、窗口变高或内容不足一屏时，`run.follow` 卡死为 `false` 且滚轮/翻页无法恢复，导致 `↓ jump to bottom` 永久悬空显示

- **关键点**：
  - `src/app/state/lx/task.rs`：`TaskPage::is_at_bottom`
  - `src/app/update/chat/viewport.rs`：`resolve_task_agents_visible`
  - `src/app/update/chat/subagent.rs`：`scroll_agents_detail`
  - `src/app/update/pane.rs`：`resize_panes`
- **错误原因**：
  1. `TaskPage::is_at_bottom` 在 `TaskTab::Agents` 下仅判断 `run.chat.messages.is_empty() || run.follow`，未检查 `run.scroll >= max`。
  2. 在 `resolve_task_agents_visible` 中，当 `run.follow == false`（用户曾上滚过）且因折叠工具卡片或窗口拉高导致新的 `max = total.saturating_sub(visible) <= run.scroll`（甚至 `max == 0`，内容已不足一屏）时，代码仅将 `run.scroll` 钳制到 `max`，**没有同步将 `run.follow` 恢复为 `true`**。
  3. 一旦进入 `run.follow == false && run.scroll == max` 状态，用户继续向下滚轮或按 `PageDown` 调用 `scroll_agents_detail` 时，因 `current == max`、`next == max`，`moved = next != current || (run.follow && next < max)` 为 `false` 并提前返回，永远走不到 `run.follow = next >= max`。结果 `run.follow` 永久卡死在 `false`，即使整个子 Agent 详情不足一屏，`↓ jump to bottom` 仍一直显示。
  4. 此外，`resize_panes` 在窗格尺寸变化时未将 `run.rows_dirty` 置为 `true`，导致窗格变宽后 `run.total_rows` 未重测。
- **修复方案**：
  1. 在 `resolve_task_agents_visible` 中，当钳制后 `run.scroll >= max` 时同步将 `run.follow` 置为 `true`；
  2. 在 `scroll_agents_detail` 中将 `run.follow != next_follow` 纳入 `moved` 判定，使已在 `max` 处但 `!run.follow` 时向下滚动能恢复 `run.follow = true`；
  3. 在 `TaskPage::is_at_bottom` 中增加 `run.visible > 0 && run.scroll >= run.total_rows.saturating_sub(run.visible)` 兜底；
  4. 在 `resize_panes` 中同步遍历 `pane.lx.agents.runs` 置 `run.rows_dirty = true`。

---

### [P1-1] `multiline_view` / `multiline_char_index_at` 在满宽行（`col == width`）与超 3 行视口下存在光标重叠、末字符无法拖选及点击首行视口跳变

- **关键点**：
  - `src/ui/widgets/input.rs`：`TextInput`、`multiline_view`、`multiline_char_index_at`、`wrapped_lines`、`cursor_position`
  - `src/ui/right_panel/todo.rs`：`render_edit_item`、`edit_input_char_at`
  - `src/ui/main_content/lx/ask.rs`：`other_window`
- **错误原因**：
  1. **满宽行末字符无法通过鼠标拖选/点击行尾覆盖**：当行宽恰好等于 `width` 时，`edit_input_char_at` 限制 `rel_col <= width - 1`，而 `multiline_char_index_at` 在最后一个字符处满足 `current_col + cells > target_col` 提前返回最后一个字符**之前**的索引，永远无法到达 `line.end_char`。
  2. **满宽尾行光标与末字符重叠**：当光标位于末尾且该行恰好占满 `width` 列时，`cursor_position` 返回 `cursor_col == width`，被 `render_edit_item` 的 `.min(list.right() - 1)` 钳回 `width - 1`，与倒数第一个字符（或 CJK 宽字符右半格）重叠。
  3. **超 3 行编辑时点击可见第 1、2 行导致视口跳回第 0 行**：`multiline_view` 和 `ask::other_window` 无状态地用 `if cursor_row >= max_rows { cursor_row + 1 - max_rows } else { 0 }` 计算窗口起点。当文本有 4~5 行且视口显示第 `1..=3` 行（`start_row = 1`）时，用户鼠标点击视口顶行（`cursor_row = 1`），`cursor_row < 3` 立即触发 `start_row` 跳回 `0`，画面在鼠标下瞬间跳行，导致后续拖拽选区错位。
- **修复方案**：
  1. 当尾行占满 `width` 列（`col == width` 且无软折行续行）时，允许光标软折到下一视觉行首列 `col = 0`（或为行尾点击/拖选开放 `target_col >= width` 到达 `line.end_char`）；
  2. 为 `TextInput` 引入滑动视口起点 `scroll_row: usize`（仅在 `cursor_row < scroll_row` 时上滚到 `cursor_row`、在 `cursor_row >= scroll_row + max_rows` 时下滚到 `cursor_row + 1 - max_rows`）；在 `AskEditor` 中同样复用内部 `Lx` 维护的滑动 `scroll()`（初始化高度为 `MAX_OTHER_EDIT_ROWS = 3`），消除点击视口上半部时的跳行。

---

### [P1-2] `TodoEdit` 与 `AskEditor` 鼠标拖拽选区未对输入框边界做坐标钳制，且 `AskEditor` 选区未接入全局 `clear_selection` 互斥

- **关键点**：
  - `src/event/mouse/drag.rs`：`handle_mouse_drag`（第 `267–304` 行）
  - `src/event/mouse/press.rs`：`handle_mouse_down_left`（第 `528–554` 行）
  - `src/ui/right_panel/todo.rs`：`edit_input_char_at`
  - `src/ui/main_content/lx/ask.rs`：`ask_editor_cell_at`
  - `src/app/update/selection.rs`：`clear_selection`
- **错误原因**：
  1. `drag.rs` 处理 `state.todo_input_drag` 和 `state.ask_input_drag` 时直接调用了严格的点击命中函数 `edit_input_char_at` 与 `ask_editor_cell_at`。当用户在第 2、3 行向左拖拽选区进入左侧缩进空白区（Todo 9 列、Ask 6 列）或上下/向右拖出编辑行边界时，函数返回 `None`，选区无法延伸到行首/行尾；且 `drag.rs:267` 的终端鼠标转发守卫漏检了 `!state.todo_input_drag && state.ask_input_drag.is_none()`。
  2. 点击 `ask` 编辑框开始选区时未调用 `update::clear_selection(state)`；而 `selection.rs::clear_selection` 也未清除 `state.ask_input_drag` 与 `AskEditor` 的选区。若用户在 `Other` 中选过文字，再去上方聊天历史拖选文字按 `Ctrl+C`，`copy_lx_selection` 会优先复制 `AskEditor` 的残留选区。
- **修复方案**：
  1. 新增带边界钳制的 `todo::edit_input_drag_char_at` 与 `ask::ask_editor_drag_cell_at` 供 `drag.rs` 使用（将 `row` 钳制到编辑项可见行范围内，将 `column < text_x` 钳制到 `0`、`column >= right` 钳制到行尾），并在拖拽期间无条件消费事件；
  2. 在 `press.rs` 点击 `ask` 编辑框时调用 `update::clear_selection(state)`，在 `selection.rs::clear_selection`（及 `begin_edit` / `cancel_edit` / `commit_edit`）中同步清除 `state.ask_input_drag` 与 `AskEditor` 选区。

---

### [P1-3] `Ask` 面板滚轮产生显式 `viewport = Some(...)` 后，原位激活 `Other...`、行内打字或重选选项时未归零 `viewport`，且 `cursor_cell` 在部分滚出时可能越界到页脚

- **关键点**：
  - `src/app/state/lx/ask.rs`：`set_highlight`、`begin_edit`、`choose`
  - `src/app/update/chat/ask.rs`：`apply_edit_input`、`apply_ask_paste`
  - `src/ui/main_content/lx/ask.rs`：`cursor_cell`
- **错误原因**：
  1. 当题目有较长 `preview` 时，用户滚轮下翻使 `ask.viewport` 变为 `Some(offset)`。若此时高亮原本就在 `Other...`（或某个选项）上，用户直接按 `Enter`/`Space` 或键入字符开始编辑 `Other...`，`begin_edit`、`set_highlight`（因 `index == highlight` 提前返回）和 `apply_edit_input` 均未将 `ask.viewport` 重置为 `None`，导致 `Other...` 仍停在视口外盲打。
  2. `cursor_cell` 只查找 `RowKind::Other { line: 0 }` 的 `y` 坐标并直接加 `line`。当 `line: 0` 恰好在 `lay.options` 最后一行而光标在 `line: 1` 时，`y + 1` 会越界落到下方的 `lay.footer` 上；而当 `line: 0` 滚出视口顶部、`line: 1` 仍在视口内时，`find(line: 0)` 返回 `None` 导致光标消失。
- **修复方案**：
  1. 在 `begin_edit`、`choose`、`set_highlight`、`apply_edit_input`、`apply_ask_paste` 中统一重置 `self.viewport = None`；
  2. 在 `cursor_cell` 中直接查找光标所在视觉行 `RowKind::Other { line } if line == cursor_line` 的矩形，直接使用其 `target.rect.y`。

---

### [P1-4] `TaskPage` 中 `chat_bar` 显隐改变输出区高度 1 行在 `max_scroll == 1` 边界处产生自锁，且 `ask` 等待耗时补偿未在会话重载路径扣除

- **关键点**：
  - `src/app/state/lx/task.rs`：`TaskPage::resize_output_viewport`
  - `src/app/update/chat/viewport.rs`：`resolve_task_output_visible`、`resolve_task_agents_visible`
  - `src/app/update/chat/view.rs`：`view_message_of`
  - `src/app/state/lx/ask.rs`：`AskState::compensate_turn_clock`
- **错误原因**：
  1. 当进程日志或子 Agent 详情总行数恰好等于无 `chat_bar` 时的完整输出区高度 `V_full` 时，一旦因切换前状态或上滚使 `chat_bar` 出现，输出区高度缩减为 `V_full - 1`，`max_scroll` 变为 `1`，导致 `chat_bar` 因自己占用的 1 行而认为内容未触底、无法消失；或上滚 1 格再下滚 1 格时差 1 行回不到底。
  2. `AskState::compensate_turn_clock` 仅将等待时间加到了内存中的 `chat.turn_started_at_ms`，而数据库持久化 `agent_message` 存的是原始 `created_at_ms` 与 `completed_at_ms`。一旦触发会话重载（`/resume`、`/compact`、`/rewind` 或重启），`view_message_of` 用 `completed_at_ms - created_at_ms` 重算 `duration_ms`，用户作答 `ask` 的等待时长被重新计入 Agent 耗时。
- **修复方案**：
  1. 在 `resize_output_viewport` 与 `resolve_task_agents_visible` 中，若当前未触底但总行数不超过无 `chat_bar` 时的完整高度 `visible + 1`，直接自动吸附触底；向下滚动时若剩余距离仅剩 `chat_bar` 占用的 1 行也直接触底。
  2. 在 `view_message_of` 中计算 `duration_ms` 时，扣除该消息下所有 `tool.name == ASK_TOOL` 的 `tool.duration_ms`；并在 `compensate_turn_clock` 中同步刷新内存 `turn_elapsed_ms`。

---

### [P1-5] 受改动文件超出 `docs/standards/code-writing-standards.md` 的单文件行数上限

- **关键点**：
  - `src/ui/main_content/lx.rs`：**1,065 行**（超过生产代码单文件 `1,000` 行硬上限）
  - `src/event/tests/lx_view.rs`：**1,534 行**（超过测试代码单文件 `1,500` 行硬上限）
  - `src/ui/right_panel/todo/tests.rs`：**1,132 行**（超过单测试文件 `1,000` 行建议线，且包含应归属 `charts` / `schedule` 子模块的测试）
- **错误原因**：
  - 本次特性在 `lx.rs`、`lx_view.rs` 与 `todo/tests.rs` 中追加逻辑与测试后，未按 `docs/standards/code-writing-standards.md` 拆分超限文件。
- **修复方案**：
  - 将 `src/ui/main_content/lx.rs` 中的面板几何/输入转发辅助拆入子模块，使 `lx.rs` 降至 1,000 行以内；
  - 将 `src/event/tests/lx_view.rs` 中的 `ask` / `task` 测试拆分为独立测试文件（如 `src/event/tests/lx_ask.rs`）；
  - 将 `src/ui/right_panel/todo/tests.rs` 中的 `charts` 与 `schedule` 测试下沉至对应子模块的测试文件中，使各测试文件均控制在 1,000 行以内。

---

### [P2-1] 后台任务面板打开时，鼠标在 `chat_bar`（`↓ jump to bottom` 行）上滚轮穿透到隐藏输入框，且输入框候选浮层未在 `TaskPage` 下隐退

- **关键点**：
  - `src/event/mouse/wheel.rs`：`handle_mouse_wheel`（第 `459–543` 行）
  - `src/ui/main_content/lx.rs`：`layout`（第 `544–550`、`580–588` 行）
- **错误原因**：
  1. 后台任务面板打开时，`task_rects.output` 下沿停在 `bar.y`（不含 `chat_bar` 行），而 `task_rects.list` 上沿在 `bar.y + 1`。当鼠标停在 `↓ jump to bottom` 所在行滚动滚轮时，既不命中 `output` 也不命中 `list`，最终穿透到底部调用 `scroll_lx_input` 滚动隐藏的输入框草稿。
  2. 当输入框存在已打开的 `@` 提及或 `refer` 候选浮层时进入 `TaskPage`，`lx.rs::layout` 仍设置 `out.input` 与 `out.panel`，使候选浮层继续画在任务面板上方并拦截 `↓ jump to bottom` 的悬停与点击。
- **修复方案**：
  - 在 `wheel.rs` 中将任务面板输出区滚轮命中下沿延伸至 `task_rects.list.y`（覆盖 `chat_bar` 行）并吞掉事件；
  - 在 `lx.rs::layout` 中当 `lx.task.is_some()` 时不设置 `out.input` 与输入框专属浮层 `out.panel`。

---

### [P2-2] `TaskPage` 残留无读取者的死字段 `agents_scroll`/`agents_follow`，导致键盘 `↑`/`↓` 切换子 Agent 时未重置目标 `SubagentRun.follow`

- **关键点**：
  - `src/app/state/lx/task.rs`：`TaskPage`（`agents_scroll`、`agents_follow`、`move_active_len`）
  - `src/app/update/pane/task_page.rs`：`apply_task_page_input`
- **错误原因**：
  - `TaskPage` 上的 `agents_scroll` 与 `agents_follow` 在生产代码中无任何读取点；鼠标点击子 Agent（`select_task_agent`）会重置选中 `run.follow = true; run.scroll = usize::MAX;`，但键盘 `↑`/`↓` 切换子 Agent 只修改了死字段，未重置新选中 `SubagentRun` 的 `follow` 与 `scroll`。
- **修复方案**：
  - 删除 `TaskPage` 上的死字段 `agents_scroll` 与 `agents_follow`；在键盘 `↑`/`↓` 切换子 Agent 时同步重置选中 `SubagentRun` 的 `follow = true; scroll = usize::MAX;`。

---

### [P2-3] 极窄宽度下 `todo::render_edit_item` 缺少 `list.right() <= text_x` 保护、`text::agent` 残留死代码常量，以及文档/注释残留旧版胶囊按钮描述

- **关键点**：
  - `src/ui/right_panel/todo.rs`：`render_edit_item`
  - `src/ui/text/agent.rs`：`ASK_TITLE`、`ASK_TITLE_SEP`、`ask_position`
  - `docs/feature/todo-task-and-ask-ux/{design.md,task.md}` 及 `src/ui/main_content/lx/ask.rs`、`src/app/update/chat/ask.rs`、`src/app/state/lx/ask.rs` 注释
- **错误原因**：
  1. `render_edit_item` 在列表宽度 `<= 9` 列（`list.right() <= text_x`）时未提前返回 `None`，导致光标被钳到左侧状态框/优先级徽标上；
  2. `ASK_TITLE`、`ASK_TITLE_SEP` 与 `ask_position` 在顶部改为 Tab 栏后已无任何调用点；
  3. 移除行内 ` ↵ submit ` 胶囊按钮后，`design.md`、`task.md` 及源码模块注释仍残留旧版胶囊按钮与移除 `Confirm` Tab 的描述。
- **修复方案**：
  - 在 `render_edit_item` 开头恢复 `if list.right() <= text_x { return None; }` 守卫；
  - 删除 `src/ui/text/agent.rs` 中的死常量 `ASK_TITLE`、`ASK_TITLE_SEP` 与死函数 `ask_position`；
  - 全面更新 `design.md`、`task.md` 与 `ask` 相关源码注释，保持文档、注释与代码实现 100% 一致。

---

## 修复与验证记录

以上 `P0-1` ~ `P0-4`、`P1-1` ~ `P1-5`、`P2-1` ~ `P2-3` 共 12 项审查问题均已全部修复并补充回归单元测试：

| 检查项 | 结果 |
| :--- | :--- |
| `rtk cargo fmt --check` | 通过 |
| `rtk cargo clippy --all-targets -- -D warnings` | 0 警告通过 |
| `rtk cargo test todo` | 149 passed |
| `rtk cargo test ask` | 263 passed |
| `rtk cargo test task_panel` | 14 passed |
| `rtk cargo test event::tests::lx` | 30 passed |
| 单文件行数规范检查 | 所有修改的生产文件与测试文件均 `< 1,000` 行 |
