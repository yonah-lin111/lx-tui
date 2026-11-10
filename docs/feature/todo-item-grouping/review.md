# 右侧 Todo 面板条目聚合与拆分（Todo Item Grouping）— 评审记录

- 评审对象：`feat/todo-item-grouping`（工作区 `.worktrees/todo-item-grouping`，基线 `dev`）
- 评审范围：`git diff dev...HEAD` 涉及的全部文件（Schema v27 迁移、`src/db/todo.rs`、`src/db/service.rs`、`src/app/state/todo*`、`src/app/update/todo*`、`src/ui/right_panel/todo*`、`src/event/mouse/*`、`src/cli/todo*` 及测试）
- 记录口径：**只收会导致数据错位、唯一不变量破坏、孤儿组残留、交互失灵、视觉串色、拖拽状态泄漏、CLI 状态失真与规范违背的真实问题**；风格偏好一律不收
- 修复后状态：**P0（3 项）、P1（4 项）、P2（2 项）、P3（1 项）共 10 项问题已全部修复并补充回归测试锁死，已合并至 `dev`**

---

## 0. 结论速览

| 严重度 | 数量 | 代表性问题 | 状态 |
| --- | --- | --- | --- |
| P0 | 3 | `resolve_display_index` 未拦截 `display == ghost` 将幽灵行错位映射为下方实体条目，且 `begin_todo_row_press` 用提交前旧下标登记拖拽导致误触发跨行合并；删除 2 人组中 1 条后执行撤销（Undo）导致原聚合组在内存中永久丢失、SQLite `restore_todos` 残留单条孤儿组；`reorder_todos` / `renumber_date` / `save_todo` 未完整闭合“同组连续”与“无单条孤儿组”数据库不变量 | 已修复并锁回归 |
| P1 | 4 | 聚合组状态锚点处于编辑态或落于组内幽灵行时，整组唯一的 `[ ]` 状态框点击失效且组内幽灵行按 `Tab` 无法切状态；真机拖拽待办行时未清理/屏蔽 `todo_hover` 导致被拖源行叠加选中高亮底色；切右栏 Tab/折叠右栏/焦点切换/按键/右键菜单/滚轮未与 `todo_drag` 联动；CLI `todo group` / `update --status done` / `rollover --id` 在已有组场景下打乱顺序、覆盖历史 `completed_date` 或拆散同组未完成项 | 已修复并锁回归 |
| P2 | 2 | `merge_with_next` 与 `ungroup_at` 未置 `follow_selected = true`，合并后折行增高可能将选中项挤出视口；`src/cli/help.rs` 的 `TODO_HELP` 漏列 `group`/`ungroup` 子命令且 `ungroup` 多 ID 报错失真 | 已修复并锁回归 |
| P3 | 1 | `src/db/service.rs` 达 995 行，逼近生产代码单文件 1000 行硬红线 | 已修复（拆分子模块） |

---

## P0（数据错位 / 核心不变量破坏 / 误触发跨行合并）

### P0-1 `resolve_display_index` 未拦截新建幽灵行槽位（`display == ghost`），且 `begin_todo_row_press` 用提交前旧下标登记 `from_display`，导致下标错位与误触发跨行合并

- **位置**：`src/app/update/todo.rs:94-100`、`src/app/update/todo/drag.rs:11-28`
- **关键点**：
  1. `ui::right_panel::todo::list_index` 明确处理了 `Some(at) if display == at => None`，但 `app::update::todo::resolve_display_index` 仅写了 `Some((ghost, true)) if display > ghost => display - 1`；
  2. `begin_todo_row_press` 先调用 `click_todo_row(state, display, now)`（内部会调用 `commit_todo_edit`），随后却无条件用**提交前**的 `from_display: display` 登记 `state.todo_drag`。
- **错误原因**：
  1. 当在非末尾条目后按 `o` / `Shift+Enter` 打开新建幽灵行时（例如 2 条列表中在下标 `0` 后新建，`ghost = 1 < state.todo.len() = 2`），对幽灵行自身 `display = 1` 调用 `resolve_display_index` 会落入 `_ => display` 分支并返回 `Some(1)`（即幽灵行下方的实体条目 `1`），而非 `None`。这会导致以幽灵行下标调用 `click_todo_status`、`click_todo_priority` 或 `drag_todo_to` 时直接误改或误并下方实体条目；
  2. 当上方存在空白幽灵行或被清空文本的编辑行时，用户左键点击下方 `display = 1` 的条目 B，`commit_todo_edit` 会销毁上方行并使条目 B 前移到 `display = 0`（原 `display = 2` 的条目 C 上移到 `display = 1`）。此时 `begin_todo_row_press` 仍记录 `from_display = 1`，只要鼠标后续移回条目 B（`display = 0 != 1`）或在松手前发生移动，就会误激活 `dragging = true` 并把相邻条目合并。
- **修复**：
  1. 在 `resolve_display_index` 中补齐 `Some((ghost, true)) if display == ghost => return None` 分支，与 `ui::right_panel::todo::list_index` 严格同源；
  2. 在 `begin_todo_row_press` 调用 `click_todo_row` 前记录 `let had_edit = state.todo.is_editing();`：若本次点击消费了在途编辑提交（行下标可能因幽灵行消失或空编辑删除而位移），则本次按下仅完成提交与选中、不登记 `state.todo_drag`；否则以提交后真实的选中下标登记 `from_display`。
- **回归测试**：`app::update::todo::tests::grouping::resolve_display_index_rejects_ghost_slot_and_press_during_edit_does_not_arm_stale_drag`。

---

### P0-2 删除 2 人聚合组子项后执行撤销（Undo），内存聚合组丢失且 `Database::restore_todos` 在 SQLite 中残留单条孤儿组

- **位置**：`src/app/update/todo.rs:551-607`、`src/app/state/todo.rs:544-605`、`src/db/todo.rs:416-500`
- **关键点**：
  1. 2 人聚合组 `(a, b)`（`group_id = Some("g1")`）中删除 `a` 时，`TodoPanel::remove(0)` 内部的 `normalize_positions()` 会因剩余同组成员数 `count == 1 < 2` 将幸存项 `b` 的 `group_id` 自动抹为 `None`；
  2. `remove_todo_at` 原先仅将 `vec![removed]`（即 `a`）存入撤销槽 `undo`；
  3. `Database::restore_todos` 在插回记录并执行 `apply_positions` 后未调用 `cleanup_singleton_groups(&transaction, &date)`。
- **错误原因**：
  1. 执行 `undo_todo_delete`（或 `TodoPanel::restore`）时，`a`（携带 `group_id = Some("g1")`）被插回列表，但此时列表中的幸存伙伴 `b.group_id` 已是 `None`。紧接着 `restore` 末尾调用 `normalize_positions()`，发现全列表只有 `a` 这一条记录携带 `"g1"`（`count == 1 < 2`），于是把刚插回的 `a.group_id` 也清除为 `None`，导致删后撤销永久丢失原 2 人聚合组；
  2. 若直接调用 `Database::restore_todos(&[a])`（或恢复非当前选中日期的记录），SQLite 中 `a.group_id` 被写回 `"g1"` 而 `b.group_id` 仍为 `NULL`，在数据库中留下 `count == 1` 的非法孤儿组，违反核心不变量 #2（最小规模 `>= 2`）。
- **修复**：
  1. 在 `remove_todo_at` 调用 `state.todo.remove(index)` 之前，若被删条目所在聚合组长度为 2，将该组的幸存伙伴条目快照（保留原 `group_id`）一并存入 `undo` 槽；
  2. 在 `TodoPanel::restore` 中，若待恢复记录的 `id` 已存在于当前列表（或归档日期列表），将既有条目的 `group_id`、`status`、`completed_date` 原地恢复后再执行 `normalize_positions()`，并在 `undo_todo_delete` 中追加 `sync_current_date_todos(state)`；
  3. 在 `Database::restore_todos` 每个日期分支的 `apply_positions(&transaction, &ordered)?;` 之后调用 `cleanup_singleton_groups(&transaction, &date)?;`。
- **回归测试**：`app::update::todo::tests::grouping::undo_delete_in_two_item_group_restores_group_in_memory_and_db`、`db::todo::tests::restore_todos_cleans_up_singleton_group_when_partner_missing`。

---

### P0-3 `Database::reorder_todos`、`renumber_date` 与 `save_todo` 未完整闭合“同组连续”与“无单条孤儿组”数据库不变量

- **位置**：`src/db/todo.rs:286-326, 503-528, 656-667`
- **关键点**：
  1. `Database::reorder_todos` 与 `renumber_date` 仅按传入 ID 顺序或 `position ASC` 赋位，未将相同 `group_id` 的成员归拢连续；
  2. `Database::save_todo` 在同日更新分支（`old_date == entry_date`）与新增插入分支（`None`）未调用 `cleanup_singleton_groups(&transaction, &entry_date)`，且在跨日移动分支（`old_date != entry_date`）仅清理了 `old_date` 而未清理目标日期 `entry_date`。
- **错误原因**：
  1. 当用户通过 `lx-tui todo reorder` 或顺延操作传入把同组成员隔开的 ID 序列时，SQLite 会直接落库为非连续的同组位置（如 `[a(g1, pos=0), standalone(None, pos=1), b(g1, pos=2)]`），违反设计文档核心不变量 #3（同组连续不变量），导致 CLI `list` 输出分裂、后续 `ungroup` 按 `rposition` 算错组尾；
  2. 当通过 `save_todo` 将 2 人组中某一条的 `group_id` 改为 `None`（或将带 `group_id` 的单条记录跨日移入新日期）时，SQLite 中会残留 `count == 1` 的单条孤儿组。
- **修复**：
  1. 在 `src/db/todo.rs` 中新增 `compact_grouped_ids(transaction, entry_date, ordered_ids)`，在 `reorder_todos` 与 `renumber_date` 调用 `apply_positions` 前按首次出现位置将同 `group_id` 成员归拢为连续区间；
  2. 在 `save_todo` 的同日更新、跨日移动与新增插入三个分支末尾统一调用 `cleanup_singleton_groups(&transaction, &entry_date)?`。
- **回归测试**：`db::todo::tests::reorder_and_save_todo_enforce_contiguous_and_non_singleton_group_invariants`。

---

## P1（交互失灵 / 拖拽串色与状态泄漏 / CLI 行为失真）

### P1-1 聚合组状态锚点处于行内编辑态或落于组内新建幽灵行时，整组唯一的 `[ ]` 状态框点击失效；且组内幽灵行按 `Tab` 无法切换状态

- **位置**：`src/ui/right_panel/todo.rs:436-449`（`status_at`）、`src/app/update/todo/group.rs:29-70`（`cycle_editing_status`、`click_todo_status`）
- **关键点**：
  1. 聚合组仅在 `offset == (count - 1) / 2` 的锚点子项首行绘制唯一 `[ ]` 状态框；
  2. `status_at` 原先因 `item::editing_row(state, record).is_some()` 屏蔽了编辑行，且对幽灵行调用 `display_record` 返回 `None`；
  3. `cycle_editing_status`（编辑态按 `Tab`）在 `edit.target` 为 `None` 时直接返回 `false`。
- **错误原因**：
  1. 在 2 人组 `[A, B]` 中，当用户双击编辑非锚点项 `B` 时点击 `A` 行上的 `[ ]` 可正常提交并切换整组状态；但当用户编辑的是锚点项 `A`（或普通单条）时，界面依然在该行首画着 `[ ]`，点击它却因 `editing_row.is_some()` 返回 `None` 而完全无响应（对比 `priority_at` 并未屏蔽编辑行）；
  2. 当用户在 2 人组首项 `A` 上按 `o` / `Shift+Enter` 插入组内新建幽灵行时，显示序列变为 3 行 `[A(0), 幽灵行(1), B(2)]`，整组唯一的 `[ ]-├─` 状态框转移到了 `display = 1`（幽灵行）上，此时 `display_record(state, 1)` 返回 `None`，导致界面上唯一可见的状态框无法点击；
  3. 在组内幽灵行输入期间按 `Tab` 键，因 `edit.target` 为 `None` 无法循环切换所在聚合组的状态。
- **修复**：
  1. `status_at` 移除 `editing_row.is_some()` 限制，且只要 `item.role.is_status_anchor()` 为真（无论是实体记录还是幽灵行）均返回 `Some(item.display)`；
  2. `click_todo_status` 在提交编辑前识别 `display`：若命中组内幽灵行，提取其 `group_id`，提交后定位该组成员（或刚由非空幽灵行生成的条目）完成整组状态循环；
  3. `cycle_editing_status` 支持 `edit.target == None`：切换 `edit.initial_status`，若 `edit.group_id` 为 `Some(gid)` 则同步循环该组全体既有成员的状态并推送 `Effect::SaveTodo`。
- **回归测试**：`app::update::todo::tests::grouping::status_click_and_tab_work_on_editing_anchor_and_in_group_ghost_row`。

---

### P1-2 真机鼠标拖拽待办行期间未清理/屏蔽 `state.todo_hover`，导致被拖拽置灰的源行始终叠加选中高亮底色

- **位置**：`src/ui/right_panel/todo/item.rs:268-286`、`src/app/update/todo/drag.rs:30-75`、`src/event/mouse/hover.rs:292-305, 349-353`
- **关键点**：
  1. 真机交互中鼠标左键按下某行前必先触发 `MouseEventKind::Moved`，使 `state.todo_hover == Some(source_idx)`；
  2. `item.rs:275` 原先计算 `let hovered = state.todo_hover == Some(index)` 未检查 `active_drag.is_none()`，且 `selected: row_selected || hovered || drag_target_highlighted`。
- **错误原因**：
  尽管 `row_selected` 特意排除了 `!drag_source_dimmed`，但由于拖动过程中 `state.todo_hover` 仍停留在 `Some(source_idx)`，`hovered` 为 `true`，导致被拖拽源行在真机拖拽全程都被铺上 `style::selection()` 选中底色，与目标合并组的高亮混在一起；同时 `hover.rs` 在计算 `todo_hover` 与 `tooltip_target` 时漏判了 `state.todo_drag.is_some()`。
- **修复**：
  1. `src/ui/right_panel/todo/item.rs` 中将 `hovered` 收敛为 `active_drag.is_none() && state.todo_hover == Some(index)`；
  2. `drag_todo_to` 激活跨行拖拽与 `finish_todo_drag` 结束时清理 `state.todo_hover = None`；
  3. `src/event/mouse/hover.rs` 在 `todo_hover` 与 `tooltip_target` 分支补齐 `|| state.todo_drag.is_some()` 抑制守卫。
- **回归测试**：`ui::right_panel::todo::tests::grouping::dragging_source_row_suppresses_hover_selection_background`。

---

### P1-3 切换右栏 Tab、折叠右栏、切走焦点、键盘按键、右键菜单及滚轮事件未与 `state.todo_drag` 完整联动

- **位置**：`src/app/update/right_panel.rs:10-30`、`src/app/update.rs:57-65`、`src/app/update/focus.rs:15-30`、`src/app/update/editor.rs:681-685`、`src/app/update/todo.rs:42-55, 513-548`、`src/app/update/todo/key.rs:17-25`、`src/event/mouse/press.rs:950-978`、`src/event/mouse/wheel.rs:245`、`src/event/mouse/drag.rs:381-387, 580-585`
- **关键点**：
  1. `select_right_tab`、`Action::TogglePrompt`、`focus_panes`、`focus_sidebar`、`focus_prompt`、`blur_todo`、`apply_todo_key`、`open_todo_item_menu`、`open_todo_blank_menu` 及 `handle_mouse_down_right` 均未清理 `state.todo_drag`；
  2. `handle_mouse_drag` 与 `handle_mouse_up_left` 在处理 `state.todo_drag` 时未检查右栏是否仍处于展开的 `RightTab::Todo` 且无模态/菜单浮层；
  3. `handle_mouse_wheel` 仅拦截了 `state.prompt_drag.is_some()`，未拦截 `state.todo_drag.is_some()`。
- **错误原因**：
  若用户在左键按住待办行期间通过快捷键切换到 `Prompt`、折叠右栏、按下右键弹出上下文菜单或滚动鼠标滚轮，残留的 `state.todo_drag` 会在后台继续响应 `view.prompt` 矩形内的鼠标移动/释放并静默合并不可见的 Todo 条目，或因滚轮改变 `scroll` 导致拖拽源与落点错位。
- **修复**：
  1. 在上述所有容器切换、折叠、失焦、按键及右键入口统一置 `state.todo_drag = None`（并在 `select_right_tab` / `TogglePrompt` 中同步清理 `todo_scroll_drag`、`todo_row_click`、`todo_input_drag`）；
  2. 在 `handle_mouse_drag` 与 `handle_mouse_up_left` 的 `todo_drag` 分支校验 `state.overlay.is_none() && !state.prompt_collapsed && state.right_tab == RightTab::Todo`，不满足时立即清空 `state.todo_drag = None` 并返回；
  3. 在 `handle_mouse_wheel` 补齐 `|| state.todo_drag.is_some()` 早退守卫。
- **回归测试**：`event::tests::todo::tab_switch_collapse_wheel_and_right_click_abort_todo_drag_cleanly`。

---

### P1-4 CLI `lx-tui todo group` / `update --status done` / `rollover --id` 在已有聚合组场景下破坏组内顺序、覆盖历史 `completed_date` 或拆散同组未完成项

- **位置**：`src/cli/todo/exec.rs:150-172, 288-339, 442-478`
- **关键点与错误原因**：
  1. `exec::group`：当首项 `a` 已属于聚合组 `[a, b]`、用户执行 `lx-tui todo group a c` 将 `c` 并入该组时，`target_member_ids` 先初始化为 `resolved_ids`（`[a, c]`）再把原组其余成员 `b` 追加到末尾，变成 `[a, c, b]`，打乱了原组 `[a, b]` 的内部顺序；且输出 JSON 的 `"ids"` 与计数只报告了命令行传入的 2 个 ID 而非实际成组的 3 个成员；
  2. `exec::update`：当传入 `--status done` 时无条件执行 `record.completed_date = Some(today_date())`，而随后的 `sync_group_status` 对同组已处于 `Done` 状态的成员保留了历史 `completed_date`，导致同组成员之间 `completed_date` 不一致，违背核心不变量 #4；
  3. `exec::rollover`：当通过 `--id` 指定顺延某条处于聚合组内的未完成待办时，未自动连带同日同组的其余未完成成员，导致顺延后原组被拆成跨日单条孤儿并双双丢失 `group_id`。
- **修复**：
  1. `exec::group` 先按当日 `position` 顺序收集首项所属聚合组的全部既有成员，再按参数顺序追加新并入的条目（若新并入条目本身属于另一聚合组，则一并按序吸收该组成员），并在返回 JSON 的 `"ids"` 与提示文案中输出完整成组成员 ID 列表；
  2. `exec::update` 对齐 `set_status`：仅当 `record.status != TodoStatus::Done || record.completed_date.is_none()` 时将 `completed_date` 设为 `Some(today_date())`，且 `sync_group_status` 统一将同组全部成员的 `completed_date` 对齐到同一值；
  3. `exec::rollover` 在 `--id` 模式下，若命中的未完成待办属于某个 `group_id`，自动将该日同组其余未完成成员一并纳入顺延列表。
- **回归测试**：`cli::tests::prompt_and_todo_tests::cli_todo_group_preserves_existing_member_order_and_rollover_keeps_group`。

---

## P2（视口跟随缺失 / CLI 帮助与参数校验）

### P2-1 `TodoPanel::merge_with_next` 与 `ungroup_at` 未置 `follow_selected = true`

- **位置**：`src/app/state/todo/group.rs:97-126, 207-219`
- **关键点与错误原因**：
  单条待办合并入组后行首前缀从 9/6 列变为 12/9 列（可用正文宽缩窄 3 列），原本 1 行的条目可能折为 2~3 行并将当前选中项挤出视口底边。`merge_item_into` 与 `split_from_group` 均设置了 `self.follow_selected = true; self.scroll_into_view(self.rows);`，但 `merge_with_next` 与 `ungroup_at` 漏设了 `self.follow_selected = true`——若用户此前滚动过鼠标滚轮（`follow_selected == false`）再按 `m` 合并，下一帧 `resolve_todo_viewport` 不会将选中项滚回视口。
- **修复**：在 `merge_with_next` 与 `ungroup_at` 产生变化时统一设置 `self.follow_selected = true; self.scroll_into_view(self.rows);`。
- **回归测试**：`app::state::todo::tests::merge_with_next_and_ungroup_re_enable_follow_selected`。

---

### P2-2 `src/cli/help.rs` 的 `TODO_HELP` 漏列 `group` / `ungroup` 子命令，且 `exec::ungroup` 多 ID 参数报错失真

- **位置**：`src/cli/help.rs:193-216`、`src/cli/todo/exec.rs:342-350`
- **关键点与错误原因**：
  1. `src/cli/help.rs` 的 `TODO_HELP` 未包含新增的 `group` 与 `ungroup` 子命令用法说明；
  2. `exec::ungroup` 在用户传入 `--id id1 --id id2` 时落入 `else` 分支错误提示 `"missing <TODO_ID>"`，且同时传位置参数与 `--id` 时会静默忽略 `--id`。
- **修复**：
  1. 在 `src/cli/help.rs` 的 `TODO_HELP` 中补充 `group` 与 `ungroup` 子命令说明；
  2. 在 `exec::ungroup` 中合并收集 `command.words` 与 `command.ids`，数量为 0 报 `missing <TODO_ID>`、`> 1` 报 `expected exactly one <TODO_ID>`。
- **回归测试**：`cli::tests::prompt_and_todo_tests::cli_todo_group_ungroup_and_status_sync_across_group`。

---

## P3（代码结构与文件行数水位治理）

### P3-1 `src/db/service.rs` 达 995 行，逼近生产代码单文件 1000 行硬红线

- **位置**：`src/db/service.rs:1-995`
- **关键点与错误原因**：
  新增 `Effect::SyncDateTodos` 分支后，`src/db/service.rs` 行数达到 995 行，距离 `AGENTS.md` 规定的生产代码单文件 1000 行上限仅剩 5 行余量。
- **修复**：参照 `src/db/service/chat.rs`，将 `src/db/service.rs` 中的工作区、提示词树、引用文件夹与 Todo 写库分派逻辑拆分为子模块 `src/db/service/workspace.rs`，使 `src/db/service.rs` 降至 ~600 行以内、子模块 ~400 行。
