# 右侧面板 Todo 日程增强、多图表统计与右键上下文菜单 — 评审记录

- 评审对象：`feat/right-panel-todo-schedule`（基线 `dev`）
- 评审范围：`git diff dev...HEAD` 涉及的全部文件（数据库迁移、纯函数公历计算、状态与行为层、Todo 右键上下文菜单、工具栏精简、`Clear Done` 下线、UI 拆分模块及单元测试）
- 记录口径：**只收会导致错误行为、唯一索引冲突、数据错位/误删、交互错乱、视觉串色、死代码残留与规范违背的真实问题**；风格偏好一律不收
- 修复后状态：**P0（2 项）、P1（5 项）、P2（4 项）共 11 项问题已全部修复并补充回归测试锁死**

---

## 0. 结论速览

| 严重度 | 数量 | 代表性问题 | 状态 |
| --- | --- | --- | --- |
| P0 | 2 | 右键菜单入口先 `commit_todo_edit` 再算 `row_at`/`list_index`，幽灵行或空编辑存在时选错/误删相邻条目；`rollover_todos_to_date` 临时偏移区间与 `renumber_date` 碰撞触发唯一索引错误 | 已修复并锁回归 |
| P1 | 5 | 删除条目未重置双击序列 `todo_row_click` 导致上移条目被单击误触发编辑，且浮层打开时未清 `todo_hover`；幽灵行打开时 `set_scroll`/`scrollbar` 漏计槽位；幽灵行位于选中行之前时选中背景错位；`pending_g` 未在非 `g` 操作时复位；切图器与分割线标签样式污染 | 已修复并锁回归 |
| P2 | 4 | 下线 `Clear Done` 与精简工具栏后残留 `take_with_status` 及废弃 Tooltip 常量，且 Todo 菜单文案未收口至 `ui/text/todo.rs`；跨日 `set_today_date` 未复位 `rollover_dismissed` 且 `replace` 未预排当日 `position`；窄横幅截断半截单词；图表标题常量收口与设计文档同步 | 已修复并锁回归 |

---

## P0（数据错位 / 误删错误条目 / 唯一索引冲突）

### P0-1 Todo 列表右键菜单在换算行下标前先执行了 `commit_todo_edit`，幽灵行或空编辑销毁时导致右键选错/误删相邻条目

- **位置**：`src/event/overlay.rs:535-556`、`src/app/update/todo.rs:664-698`
- **关键点**：
  1. 左键行点击 `click_todo_row` 与优先级徽标点击 `click_todo_priority` 均严格遵守“**必须在 `commit_todo_edit` 之前**通过 `resolve_display_index` 将屏幕显示下标换算为条目 `id`，提交后再按 `id` 找回最新列表下标”的不变式；
  2. 右键菜单入口 `open_right_click_menu` 却在进入 `effective_list_rect` 分支的第一行直接调用了 `update::commit_todo_edit(state)`，随后才调用 `row_at` 与 `list_index` 计算命中行。
- **错误原因**：
  1. **幽灵行在被点行上方**：假设列表有 `["a", "b", "c"]`，用户在 `"a"` 后按 `o` 打开空白新建幽灵行（占显示槽位 1，`"b"`、`"c"` 顺延到显示槽位 2、3），随后右键点击显示槽位 2 的 `"b"`。由于 `commit_todo_edit` 先一步销毁了空白幽灵行，`"b"`、`"c"` 立即退回槽位 1、2；紧接着 `row_at` 在槽位 2 命中 `"c"`，导致用户对 `"b"` 的右键菜单错误绑定到 `"c"`（一旦确认 `Delete` 就会直接误删 `"c"`；若 `"b"` 是末行则会越界退化为 `TodoBlank` 空白菜单）；
  2. **既有条目清空文本后右键下方行**：假设用户双击编辑第 0 行 `"a"` 并清空文本（`commit_todo_edit` 会删除 `"a"`），随后右键点击第 1 行 `"b"`。先执行的 `commit_todo_edit` 删除 `"a"` 使 `"b"` 上移到第 0 行，随后的 `row_at` 在第 1 行错误命中 `"c"` 或越界变为空白菜单；
  3. **直接右键点击空白幽灵行本身**：用户在空白幽灵行（显示槽位 1）上点右键，`commit_todo_edit` 销毁幽灵行后下方的 `"b"` 顶替到槽位 1，导致对空白幽灵行的右键错误弹出 `"b"` 的条目菜单。
- **修复**：
  1. 在 `src/app/update/todo.rs` 中新增 `right_click_todo_list(state: &mut AppState, display: Option<usize>, anchor: (u16, u16))` 统一收口右键菜单业务；
  2. **在 `commit_todo_edit` 之前**解析 `display`：区分“命中新建幽灵行（含草稿是否非空）”与“命中既有条目（预先提取 `id`）”；
  3. 执行 `commit_todo_edit` 与 `focus_todo` 后，按 `id` 在最新列表中重定位并选中目标行（若点中空白幽灵行、已被空编辑删除的行或列表下方空白，则打开 `TodoBlank` 菜单）。
- **回归测试**：`app::update::todo::tests::right_click_todo_list_with_open_ghost_or_empty_edit_targets_exact_item`。

---

### P0-2 `rollover_todos_to_date` 临时位置偏移与 `renumber_date` 碰撞，非密集 `position` 下触发唯一索引失败

- **位置**：`src/db/todo.rs:308-327`（`save_todo` 插入分支）、`src/db/todo.rs:465-507`（`rollover_todos_to_date`）
- **关键点**：
  1. `save_todo` 的 `None`（新增行）分支在 `shift_positions_from` + `INSERT` 后未调用 `renumber_date`，若调用方传入的 `record.position` 大于当前该日行数（如空表插入 `position = 1`），库内会留下稀疏 `position`；
  2. `rollover_todos_to_date` 原先使用 `RENUMBER_OFFSET + base_count + offset`（其中 `base_count = SELECT COUNT(*)`）暂存迁入行，紧接着调用 `renumber_date(target_date)` 将目标日期原有行（`position < RENUMBER_OFFSET`）更新为 `position + RENUMBER_OFFSET`。
- **错误原因**：当目标日期存在 1 条 `position = 1` 的记录时，`base_count = 1`。`rollover_todos_to_date` 将顺延的第 0 条记录更新为 `position = 1_000_000 + 1 + 0 = 1_000_001`；随后 `renumber_date` 将目标日期原有记录更新为 `1_000_000 + 1 = 1_000_001`，直接撞上 `idx_todo_date_position (entry_date, position)` 唯一索引，导致整个顺延事务失败回滚。
- **修复**：
  1. `save_todo` 的 `None` 分支尾部补调 `renumber_date(&transaction, &entry_date)?`，从源头保证库内任意日期下的 `position` 恒为 `0..n` 密集递增；
  2. `rollover_todos_to_date` 改用与 `RENUMBER_OFFSET (1_000_000)` 完全隔离的 `RESTORE_OFFSET (2_000_000) + offset` 暂存迁入行，删去多余的 `SELECT COUNT(*)`，且 `ORDER BY position ASC` 天然保证目标日原有行在前、顺延行在后。
- **回归测试**：`db::todo::tests::rollover_does_not_collide_when_target_date_had_sparse_position`。

---

## P1（交互错乱 / 悬停残留 / 视口跳变 / 渲染串色）

### P1-1 删除条目后未清空双击计时 `todo_row_click` 与越界 `todo_hover`，且浮层菜单打开时未清除 `todo_hover`

- **位置**：`src/app/update/todo.rs:664-698, 735-743`、`src/app/update/menu.rs:449-471`、`src/event/mouse/hover.rs:112-115`
- **关键点**：
  1. `remove_todo_at` 删除条目使下方行整体前移一位后，未重置 `state.todo_row_click = None`，也未清理越界的 `state.todo_hover`；
  2. `src/event/mouse/hover.rs` 在 `state.overlay.is_some()` 早退分支中仅清除了 `set_prompt_tree_hover(state, None)`，漏掉了 `set_todo_hover(state, None)`；
  3. `activate_menu` 在执行 `CycleTodoPriority` / `DeleteTodo` / `SortTodos` 时未统一确保 `commit_todo_edit` 与 `focus_todo`。
- **错误原因**：
  1. 用户单击第 0 行 `"a"`（记录 `todo_row_click = Some((0, t0))`）后立即按 `d` 删除 `"a"`，第 1 行 `"b"` 顶替到第 0 行；若用户在双击时间窗内再次单击第 0 行想要选中 `"b"`，残留的 `todo_row_click` 会把对 `"b"` 的首次单击误判为双击，直接闯入 `"b"` 的行内编辑态；
  2. 鼠标悬停在 Todo 某行后右键打开上下文菜单，当鼠标移入菜单项或移出列表时，`hover.rs` 因 `state.overlay.is_some()` 提前返回而不清理 `todo_hover`，导致底层列表一直残留旧行的悬停高亮；若删除末行后 `todo_hover` 越界还会留下悬空索引。
- **修复**：
  1. `remove_todo_at` 清零 `state.todo_row_click = None`，并在 `state.todo_hover.is_some_and(|h| h >= state.todo.len())` 时清零 `state.todo_hover`；
  2. `open_todo_item_menu` 与 `open_todo_blank_menu` 统一复位 `pending_g`、`todo_row_click` 与 `todo_hover`；`hover.rs` 在 `state.overlay.is_some()` 分支补齐 `update::set_todo_hover(state, None)`；
  3. `activate_menu` 的 Todo 命令分支统一确保提交在途编辑并聚焦 Todo 面板。
- **回归测试**：`app::update::todo::tests::delete_resets_double_click_sequence_and_clamps_hover`。

---

### P1-2 新建幽灵行打开时，`set_scroll` 与 `scrollbar` 漏计幽灵槽位，向下滚轮反将幽灵行踢出视口

- **位置**：`src/app/state/todo.rs:557-570`、`src/ui/right_panel/todo.rs:299-315`
- **关键点**：`set_viewport_rows` 与 `scroll_to_edit` 均按 `items.len() + ghost` 计算可滚动总行数，但 `TodoPanel::set_scroll`（滚轮与滚动条拖拽入口）与 `todo::scrollbar` 原先仍按 `items.len()` 计算。
- **错误原因**：列表溢出且在末尾打开新建幽灵行时，`scroll_to_edit` 已将视口滚到 `items.len() + 1 - rows` 使幽灵行可见；此时若用户向下滚动鼠标滚轮（`scroll_todo` → `set_scroll`），`set_scroll` 的上限 `max` 少算了 1 行幽灵槽位，反而把 `scroll` 往回钳小 1 行，导致正在输入的幽灵行瞬间掉出视口底部消失；同时当 `items.len() == visible` 时新建幽灵行不显示滚动条。
- **修复**：新增 `TodoPanel::display_len(&self) -> usize`（`self.items.len() + ghost`），在 `set_scroll`、`set_viewport_rows` 与 `todo::scrollbar` 中统一使用 `display_len()`。
- **回归测试**：`app::state::todo::tests::set_scroll_and_replace_and_today_rollover_invariants`。

---

### P1-3 `render_rows` 用显示下标计算 `row_selected`，幽灵行位于选中行之前时选中背景错位到上一行

- **位置**：`src/ui/right_panel/todo.rs:434-456`
- **关键点**：`state.todo.selected()` 存的是**列表下标**（`record_index`）。`render_rows` 在绘制行首 `❯` 标记时使用 `state.todo.selected() == Some(record_index)`，却在扣除幽灵行偏移之前用**显示下标** `index` 计算了 `row_selected = state.todo_focused() && state.todo.selected() == Some(index)`。
- **错误原因**：当新建幽灵行插入位置 `at <= selected` 时，后续条目的显示下标 `index = record_index + 1`，导致 `row_selected` 命中在 `record_index - 1` 那一行上，出现“前一行染了选中背景色、后一行画着 `❯` 箭头”的分裂现象。
- **修复**：将 `row_selected` 的计算移至 `record_index` 换算之后，统一按 `state.todo_focused() && state.todo.selected() == Some(record_index)` 判定。
- **回归测试**：`ui::right_panel::todo::tests::ghost_row_before_selected_keeps_selection_highlight_aligned_and_scrollbar_counts_ghost`。

---

### P1-4 列表态 `gg` 跳首的挂起标记 `pending_g` 未被非 `g` 按键与鼠标操作复位

- **位置**：`src/app/update/todo.rs:90-142, 234-280, 466-558`
- **关键点**：`apply_todo_command` 注释声明“中间任何非 `g` 命令都会经由 `pending_g` 重置”，但原先在 `j`/`k`/`d`/`e` 以及 `Up`/`Down`/`Space`/`Tab` 和鼠标点击分支中均未调用 `set_pending_g(false)`。
- **错误原因**：用户误按一次 `g` 后，继续通过 `j`/`k`/`Down` 移动、按 `Space` 切换状态或用鼠标点击行，之后再次按单次 `g` 时仍会误触发 `gg` 直接跳回首行。
- **修复**：在 `apply_todo_list_key` 入口处对所有非 `TodoKey::Char('g')` 按键统一复位 `state.todo.set_pending_g(false)`，并在鼠标左/右键点击、优先级点击、顺延与切图入口同步复位。
- **回归测试**：`app::update::todo::tests::pending_g_is_cleared_by_non_g_keys_and_clear_done_preserves_edit_commit`。

---

### P1-5 图表切换器 `‹` / `›` 箭头与分割线日期标签未先重置单元格样式，继承分割线底层修饰符

- **位置**：`src/ui/right_panel/todo/charts.rs:135-178`、`src/ui/right_panel/todo/schedule.rs:160-192`
- **关键点**：
  1. `render_chart_switcher` 收敛 `whole` 至 `prev.x..=next.x` 后，若不对 `whole` 执行 `buf.set_style(whole, Style::reset())`，由于 ratatui `set_style` 是增量 patch 语义，`‹` / `›` 箭头会继承 `render_rule` 留下的 `Modifier::DIM`（变成 `BOLD | DIM`）；
  2. 同理，`render_divider_labels` 在聚焦态（分割线为 `Cyan + BOLD`）下若不先重置 `left_rect` / `right_rect`，写入 `style::border_title()`（`Cyan + DIM`）会残留 `BOLD` 修饰符。
- **修复**：
  1. `chart_switcher_rects` 保持 `whole` 严格等于 `prev.x..=next.x`（绝不触碰 `prev.x - 1` 与 `next.x + 1` 的 `─` 边框格），并在 `render_chart_switcher` 入口对 `whole` 执行 `buf.set_style(whole, Style::reset())`，中间文本采用 `style::muted_on_border()`；
  2. `render_divider_labels` 对 `left_rect` 与 `right_rect` 先执行 `Style::reset()` 再写入 `style::border_title()`。
- **回归测试**：`ui::right_panel::todo::tests::chart_switcher_and_three_chart_modes_render`。

---

## P2（死代码残留 / 模块归属偏离 / 窄宽截断 / 边界状态）

### P2-1 下线 `Clear Done` 与精简工具栏后残留 `TodoPanel::take_with_status` 及废弃 Tooltip 常量，且 Todo 菜单文案未收口至 `ui/text/todo.rs`

- **位置**：`src/app/state/todo.rs:730-737`、`src/app/state/todo/tests.rs:140-170`、`src/ui/text/todo.rs:68-71`、`src/ui/text.rs:339-343, 1037-1038`
- **关键点**：
  1. 下线 `Clear Done` 功能后，`TodoPanel::take_with_status`（注释明确标注 `（[clear done]）`）已无任何生产调用方，仅剩孤立单元测试；
  2. `src/ui/text/todo.rs` 中仍残留已移除工具栏按钮的 `TODO_TOOLTIP_EDIT`、`TODO_TOOLTIP_DELETE`、`TODO_TOOLTIP_CLEAR_DONE` 三个死常量（因 `src/ui/text.rs` 使用 `pub use todo::*;` 公开重导出，编译器未触发 `dead_code` 告警）；
  3. 新增的 7 个 Todo 右键菜单文案常量（`MENU_TODO_*`、`MENU_TITLE_TODO_*`）被直接放入已超 1300 行的 `src/ui/text.rs`，未按模块边界归入 `src/ui/text/todo.rs`；同时 `src/app/update/todo.rs:474` 注释仍残留 `[edit]` 旧文案。
- **错误原因**：物理移除 `Clear Done` 与 `[edit]`/`[del]` 时只清理了 DB/Effect/Update/Tooltip 枚举，漏清理了 `state` 层辅助函数与 `ui/text/todo.rs` 重导出常量。
- **修复**：
  1. 物理删除 `TodoPanel::take_with_status` 及其孤立测试 `take_with_status_removes_only_matching`；
  2. 从 `src/ui/text/todo.rs` 删除 `TODO_TOOLTIP_EDIT`、`TODO_TOOLTIP_DELETE`、`TODO_TOOLTIP_CLEAR_DONE`；
  3. 将 `MENU_TODO_NEW`、`MENU_TODO_EDIT`、`MENU_TODO_PRIORITY`、`MENU_TODO_DELETE`、`MENU_TODO_SORT`、`MENU_TITLE_TODO_ITEM`、`MENU_TITLE_TODO_BLANK` 迁入 `src/ui/text/todo.rs`（经 `ui/text.rs` 的 `pub use todo::*;` 对外路径保持不变），并修正过时注释。

---

### P2-2 `set_today_date` 跨日推进未重置 `rollover_dismissed`，且 `replace` 对当前选中日未按 `position` 预排

- **位置**：`src/app/state/todo.rs:171-182`、`src/app/state/todo.rs:480-506`
- **关键点**：
  1. `set_today_date` 在今日日期推进时未将 `self.rollover_dismissed` 重置为 `false`；
  2. `replace` 对归档日期 `other_dates` 执行了 `list.sort_by_key(|item| item.position)`，但对当前选中日期 `self.items` 漏掉了按 `position` 排序，直接按入参切片顺序调用了 `normalize_positions()`。
- **错误原因**：前一天忽略过顺延横幅后，跨入新一天时横幅仍被永久抑制；测试或外部注入非升序 `position` 的当日记录时顺序被改写。
- **修复**：`set_today_date` 在日期实际变化时重置 `self.rollover_dismissed = false`；`replace` 对 `self.items` 先执行 `self.items.sort_by_key(|item| item.position)` 再调用 `normalize_positions()`。
- **回归测试**：`app::state::todo::tests::set_scroll_and_replace_and_today_rollover_invariants`。

---

### P2-3 顺延横幅在窄宽度（14..18 列）下截断出半截英文单词

- **位置**：`src/ui/right_panel/todo/schedule.rs:280-303`
- **关键点**：当面板较窄（`text_limit` 在 3..7 列之间）时，原逻辑固定格式化 `"! {count} past"` 并交由 `set_stringn` 截断，画面会出现 `"! 3 pas"` 等半截单词。
- **错误原因**：违背 `docs/standards/tui-design-requirements.md`“放不下则降级或隐藏，杜绝半截残句”规范。
- **修复**：增加三档完整文案阶梯（`"! {count} past unfinished"` → `"! {count} past"` → `"!{count}"`），按显示宽度匹配，若最短完整文案仍放不下则不绘制残句。

---

### P2-4 `src/ui/text/todo.rs` 图表标题常量收口与 `design.md` / `task.md` 同步

- **位置**：`src/ui/text/todo.rs:36-47`、`src/ui/right_panel/todo/charts.rs:24-56`、`docs/feature/right-panel-todo-schedule/{design.md,task.md}`
- **关键点**：将切图器完整/简短标题与顺延 Toast 文案统一收口至 `src/ui/text/todo.rs` 并由 `charts.rs`、`update/todo.rs` 引用；同步更新 `design.md` 与 `task.md`，完整反映工具栏精简、`Clear Done` 下线与 Todo 右键上下文菜单设计。
