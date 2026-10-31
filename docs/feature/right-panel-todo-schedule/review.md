# 右侧面板 Todo 日程增强与多图表统计 — 评审记录

- 评审对象：`feat/right-panel-todo-schedule`（基线 `dev`）
- 评审范围：`git diff dev...HEAD` 涉及的 27 个文件（持久化迁移、纯函数公历计算、状态与行为层、UI 拆分模块及单元测试）
- 记录口径：**只收会导致错误行为、唯一索引冲突、数据误删、交互错乱、视觉串色、规范违背的真实问题**；风格偏好一律不收
- 修复后状态：**P0（2 项）、P1（4 项）、P2（4 项）共 10 项问题已全部修复并补充回归测试锁死**

---

## 0. 结论速览

| 严重度 | 数量 | 代表性问题 | 状态 |
| --- | --- | --- | --- |
| P0 | 2 | `rollover_todos_to_date` 临时偏移区间与 `renumber_date` 碰撞触发唯一索引错误；`Effect::DeleteTodosWithStatus` 缺失 `entry_date` 导致跨日误删 | 已修复并锁回归 |
| P1 | 4 | 幽灵行打开时 `set_scroll`/`scrollbar` 漏计幽灵槽位；幽灵行在选中行之前时选中底色错位；`pending_g` 未在非 `g` 键时复位；切图器与分割线标签样式污染 | 已修复并锁回归 |
| P2 | 4 | `clear_done_todos` 早退分支吞掉 `commit_todo_edit` 变更；跨日 `set_today_date` 未复位 `rollover_dismissed` 且 `replace` 未预排当日 `position`；窄横幅截断半截单词；死常量与设计文档未同步 | 已修复并锁回归 |

---

## P0（数据损坏 / 唯一索引冲突 / 跨日误删）

### P0-1 `rollover_todos_to_date` 临时位置偏移与 `renumber_date` 碰撞，非密集 position 下触发唯一索引失败

- **位置**：`src/db/todo.rs:308-327`（`save_todo` 插入分支）、`src/db/todo.rs:522-564`（`rollover_todos_to_date`）
- **关键点**：
  1. `save_todo` 的 `None`（新增行）分支在 `shift_positions_from` + `INSERT` 后未调用 `renumber_date`，若调用方传入的 `record.position` 大于当前该日行数（如空表插入 `position = 1`），库内会留下稀疏 `position`；
  2. `rollover_todos_to_date` 使用 `RENUMBER_OFFSET + base_count + offset`（其中 `base_count = SELECT COUNT(*)`）暂存迁入行，紧接着调用 `renumber_date(target_date)` 将目标日期原有行（`position < RENUMBER_OFFSET`）更新为 `position + RENUMBER_OFFSET`。
- **错误原因**：当目标日期存在 1 条 `position = 1` 的记录时，`base_count = 1`。`rollover_todos_to_date` 将顺延的第 0 条记录更新为 `position = 1_000_000 + 1 + 0 = 1_000_001`；随后 `renumber_date` 将目标日期原有记录更新为 `1_000_000 + 1 = 1_000_001`，直接撞上 `idx_todo_date_position (entry_date, position)` 唯一索引，导致整个顺延事务失败回滚。
- **修复**：
  1. `save_todo` 的 `None` 分支尾部补调 `renumber_date(&transaction, &entry_date)?`，从源头保证库内任意日期下的 `position` 恒为 `0..n` 密集递增；
  2. `rollover_todos_to_date` 改用与 `RENUMBER_OFFSET (1_000_000)` 完全隔离的 `RESTORE_OFFSET (2_000_000) + offset` 暂存迁入行，删去多余的 `SELECT COUNT(*)`，且 `ORDER BY position ASC` 天然保证目标日原有行在前、顺延行在后。
- **回归测试**：`db::todo::tests::rollover_does_not_collide_when_target_date_had_sparse_position`。

---

### P0-2 `Effect::DeleteTodosWithStatus` 缺失 `entry_date`，执行期读取动态选中日期导致跨日误删

- **位置**：`src/app/effects.rs:285`、`src/app/update/todo.rs:659-674`、`src/db/service.rs:1101-1106`
- **关键点**：`Effect::ReorderTodos` 与 `Effect::RolloverTodos` 均在生成效应时绑定了日期字段，唯独 `Effect::DeleteTodosWithStatus` 只携带 `status: TodoStatus`，而在 `Persistence::execute` 执行期才读取 `state.todo.selected_date()`。
- **错误原因**：若在同一轮事件泵送前，用户在日期 A 触发 `clear_done_todos` 后紧接着切换到日期 B，内存里清空的是日期 A 的已完成项，持久化泵却读取最新的 `selected_date`（日期 B）把数据库中日期 B 的已完成项删除，造成日期 B 数据丢失、日期 A 已完成项重启后复活。
- **修复**：为 `Effect::DeleteTodosWithStatus` 增加 `entry_date: String` 字段，在 `clear_done_todos` 入队时冻结当时的 `selected_date`，`Persistence::execute` 严格按效应携带的 `entry_date` 执行 `delete_todos_with_status_on`。
- **回归测试**：`app::update::todo::tests::clear_done_removes_only_done_and_can_be_undone`。

---

## P1（交互错乱 / 视口跳变 / 渲染串色）

### P1-1 新建幽灵行打开时，`set_scroll` 与 `scrollbar` 漏计幽灵槽位，向下滚轮反将幽灵行踢出视口

- **位置**：`src/app/state/todo.rs:547-559`、`src/ui/right_panel/todo.rs:322-328`
- **关键点**：`set_viewport_rows` 与 `scroll_to_edit` 均按 `items.len() + ghost` 计算可滚动总行数，但 `TodoPanel::set_scroll`（滚轮与滚动条拖拽入口）与 `todo::scrollbar` 仍按 `items.len()` 计算。
- **错误原因**：列表溢出且在末尾打开新建幽灵行时，`scroll_to_edit` 已将视口滚到 `items.len() + 1 - rows` 使幽灵行可见；此时若用户向下滚动鼠标滚轮（`scroll_todo` → `set_scroll`），`set_scroll` 的上限 `max` 少算了 1 行幽灵槽位，反而把 `scroll` 往回钳小 1 行，导致正在输入的幽灵行瞬间掉出视口底部消失；同时当 `items.len() == visible` 时新建幽灵行不显示滚动条。
- **修复**：新增 `TodoPanel::display_len(&self) -> usize`（`self.items.len() + ghost`），在 `set_scroll`、`set_viewport_rows` 与 `todo::scrollbar` 中统一使用 `display_len()`。
- **回归测试**：`app::state::todo::tests::set_scroll_and_replace_and_today_rollover_invariants`。

---

### P1-2 `render_rows` 用显示下标计算 `row_selected`，幽灵行位于选中行之前时选中背景错位到上一行

- **位置**：`src/ui/right_panel/todo.rs:434`、`src/ui/right_panel/todo.rs:451-471`
- **关键点**：`state.todo.selected()` 存的是**列表下标**（`record_index`）。`render_rows` 在绘制行首 `❯` 标记时使用 `state.todo.selected() == Some(record_index)`，却在扣除幽灵行偏移之前用**显示下标** `index` 计算了 `row_selected = state.todo_focused() && state.todo.selected() == Some(index)`。
- **错误原因**：当新建幽灵行插入位置 `at <= selected` 时，后续条目的显示下标 `index = record_index + 1`，导致 `row_selected` 命中在 `record_index - 1` 那一行上，出现“前一行染了选中背景色、后一行画着 `❯` 箭头”的分裂现象。
- **修复**：将 `row_selected` 的计算移至 `record_index` 换算之后，统一按 `state.todo_focused() && state.todo.selected() == Some(record_index)` 判定。
- **回归测试**：`ui::right_panel::todo::tests::ghost_row_before_selected_keeps_selection_highlight_aligned_and_scrollbar_counts_ghost`。

---

### P1-3 列表态 `gg` 跳首的挂起标记 `pending_g` 未被非 `g` 按键与鼠标操作复位

- **位置**：`src/app/update/todo.rs:90-142`、`src/app/update/todo.rs:231-280`、`src/app/update/todo.rs:463-553`
- **关键点**：`apply_todo_command` 注释声明“中间任何非 `g` 命令都会经由 `pending_g` 重置”，但实际在 `j`/`k`/`d`/`e` 以及 `Up`/`Down`/`Space`/`Tab` 和鼠标点击分支中均未调用 `set_pending_g(false)`。
- **错误原因**：用户误按一次 `g` 后，继续通过 `j`/`k`/`Down` 移动、按 `Space` 切换状态或用鼠标点击行，之后再次按单次 `g` 时仍会误触发 `gg` 直接跳回首行。
- **修复**：在 `apply_todo_list_key` 入口处对所有非 `TodoKey::Char('g')` 按键统一复位 `state.todo.set_pending_g(false)`，并在鼠标行点击、优先级点击、顺延与切图入口同步复位。
- **回归测试**：`app::update::todo::tests::pending_g_is_cleared_by_non_g_keys_and_clear_done_preserves_edit_commit`。

---

### P1-4 图表切换器 `‹` / `›` 箭头与分割线日期标签未先重置单元格样式，继承分割线底层修饰符

- **位置**：`src/ui/right_panel/todo/charts.rs:142-176`、`src/ui/right_panel/todo/schedule.rs:160-200`
- **关键点**：
  1. 此前 `render_chart_switcher` 因 `whole` 包含外围空格导致左侧 `─` 的 `Modifier::DIM` 被洗掉产生左右边框色差；收敛 `whole` 至 `prev.x..=next.x` 后，若移除 `buf.set_style(whole, Style::reset())`，由于 ratatui `set_style` 是增量 patch 语义，`‹` / `›` 箭头会继承 `render_rule` 留下的 `Modifier::DIM`（变成 `BOLD | DIM`）；
  2. 同理，`render_divider_labels` 在聚焦态（分割线为 `Cyan + BOLD`）下若不先重置 `left_rect` / `right_rect`，写入 `style::border_title()`（`Cyan + DIM`）会残留 `BOLD` 修饰符。
- **修复**：
  1. `chart_switcher_rects` 保持 `whole` 严格等于 `prev.x..=next.x`（绝不触碰 `prev.x - 1` 与 `next.x + 1` 的 `─` 边框格），并在 `render_chart_switcher` 入口对 `whole` 执行 `buf.set_style(whole, Style::reset())`，中间文本采用 `style::muted_on_border()`；
  2. `render_divider_labels` 对 `left_rect` 与 `right_rect` 先执行 `Style::reset()` 再写入 `style::border_title()`。
- **回归测试**：`ui::right_panel::todo::tests::chart_switcher_and_three_chart_modes_render`（同时锁死两侧 `─` 边框样式一致与 `‹`/`›` 不含 `Modifier::DIM`）。

---

## P2（边界状态遗漏 / 窄宽截断 / 死常量与文档同步）

### P2-1 `clear_done_todos` 在提前返回分支丢弃了 `commit_todo_edit` 的变化返回值

- **位置**：`src/app/update/todo.rs:659-667`
- **关键点**：`clear_done_todos` 首行执行 `let _ = commit_todo_edit(state);`，随后若 `done_count() == 0` 直接返回 `false`。
- **错误原因**：当列表仅有一条已完成项、用户双击该项清空文本后触发 `clear_done_todos` 时，`commit_todo_edit` 会删除该空文本条目并返回 `true`，此时 `done_count()` 变为 `0`，函数返回 `false` 吞掉了本次编辑提交产生的变化。
- **修复**：将 `commit_todo_edit(state)` 的返回值保存在 `changed` 中，早退分支返回 `changed`。
- **回归测试**：`app::update::todo::tests::pending_g_is_cleared_by_non_g_keys_and_clear_done_preserves_edit_commit`。

---

### P2-2 `set_today_date` 跨日推进未重置 `rollover_dismissed`，且 `replace` 对当前选中日未按 `position` 预排

- **位置**：`src/app/state/todo.rs:171-178`、`src/app/state/todo.rs:471-495`
- **关键点**：
  1. `set_today_date` 在今日日期推进时未将 `self.rollover_dismissed` 重置为 `false`；
  2. `replace` 对归档日期 `other_dates` 执行了 `list.sort_by_key(|item| item.position)`，但对当前选中日期 `self.items` 漏掉了按 `position` 排序，直接按入参切片顺序调用了 `normalize_positions()`。
- **错误原因**：前一天忽略过顺延横幅后，跨入新一天时横幅仍被永久抑制；测试或外部注入非升序 `position` 的当日记录时顺序被改写。
- **修复**：`set_today_date` 在日期实际变化时重置 `self.rollover_dismissed = false`；`replace` 对 `self.items` 先执行 `self.items.sort_by_key(|item| item.position)` 再调用 `normalize_positions()`。
- **回归测试**：`app::state::todo::tests::set_scroll_and_replace_and_today_rollover_invariants`。

---

### P2-3 顺延横幅在窄宽度（14..18 列）下截断出半截英文单词

- **位置**：`src/ui/right_panel/todo/schedule.rs:288-302`
- **关键点**：当面板较窄（`text_limit` 在 3..7 列之间）时，原逻辑固定格式化 `"! {count} past"` 并交由 `set_stringn` 截断，画面会出现 `"! 3 pas"` 等半截单词。
- **错误原因**：违背 `docs/standards/tui-design-requirements.md`“放不下则降级或隐藏，杜绝半截残句”规范。
- **修复**：增加三档完整文案阶梯（`"! {count} past unfinished"` → `"! {count} past"` → `"!{count}"`），按显示宽度匹配，若最短完整文案仍放不下则不绘制残句。

---

### P2-4 `src/ui/text/todo.rs` 图表标题常量未在渲染层引用，以及 `design.md` / `task.md` 未同步补充需求

- **位置**：`src/ui/text/todo.rs:39-48`、`src/ui/right_panel/todo/charts.rs:24-63`、`docs/feature/right-panel-todo-schedule/{design.md,task.md}`
- **关键点**：`charts.rs` 直接硬编码了 `"today · status"` 等字符串字面量，导致 `text/todo.rs` 中的旧标题常量悬空；同时 `design.md` 与 `task.md` 尚未同步补充需求（分割线日期与周跨度、纯英文星期感知带、切图器范围标注及边框颜色修复）。
- **修复**：将切图器完整/简短标题与顺延 Toast 文案统一收口至 `src/ui/text/todo.rs` 并由 `charts.rs`、`update/todo.rs` 引用；同步更新 `design.md` 与 `task.md`。
