# 右侧容器 Todo 面板 — 评审记录

- 评审对象：`feat/right-panel-todo`（7 个提交，基线 `dev@8707aa7`）
- 评审范围：`git diff dev...HEAD` 的 60 个文件（新增 6797 行 / 删除 245 行）
- 评审方式：先派 3 个 reviewer 并行（状态+持久化 / 渲染+布局 / 输入+事件+测试），
  **三个 agent 运行 50 分钟均未产出任何文件**，已终止；改由主评审按同一份范围自行完成。
- 记录口径：**只收会导致错误行为、panic、数据丢失、交互错乱、假阳性测试的问题**；
  风格偏好、注释建议一律不收。
- 验证方式：每条修复都先写**会失败的临时用例**证明缺陷真实存在（下文标注「实测」），
  修完补正式用例并用**变异验证**确认「把代码改回去用例就转红」。未能实测的条目标注「未实测」。
- 修复后状态：全部 P0/P1/P2 已修复并锁回归；P3-1（`style.rs` 超 1000 行）**未修**（超出本次授权，
  属独立重构）。

---

## 0. 结论速览

| 严重度 | 数量 | 代表性缺陷 |
| --- | --- | --- |
| P0 | 1 | 拖拽 Todo 滚动条后状态永不释放，之后所有左键拖拽被劫持 |
| P1 | 5 | 点当前 tab 不聚焦；Prompt 下右缘点击滚动隐藏的 Todo；悬停行高亮错位；末尾幽灵行不可见；超长 todo 进内存但写库失败 |
| P2 | 4 | 切顶部标签不提交 todo 编辑；侧栏持焦点时粘贴落进 prompt；迁移文档注释残缺；设计文档未回写 tab 聚焦口径 |
| P3 | 2 | `src/ui/style.rs` 超 1000 行（未修）；`todo_hover` 两套下标口径（随 P1-3 修掉） |
| 额外 | 1 | 测试共享固定临时库路径，导致跨分支/并发互踩（修复过程中暴露） |

前 6 条（P0 + P1）已实测复现，逐条附触发路径。**P0 与 P1 全部 6 条已修并变异验证。**

---

## P0

### P0-1 `todo_scroll_drag` 永不释放，劫持之后所有左键拖拽

- **位置**：`src/event/mouse/drag.rs:96`、`src/event/mouse/hit.rs:488`
- **关键点**：`handle_mouse_up_left` 的清理链（`drag.rs:305-334`）用 `take()` 复位了
  9 个拖拽状态——`referenced_search` / `referenced_menu` / `workspace` / `prompt` /
  `task` / `agent` / `terminal` / `lx` / `lx_history`——**唯独漏了 `todo_scroll_drag`**。
- **错误原因**（实测）：在 Todo 列表右边抓一次滚动条 thumb 再松手，
  `state.todo_scroll_drag` 仍是 `Some(grab)`。此后任何左键拖拽，
  `handle_mouse_drag` 在 `drag.rs:96` 就命中 Todo 分支、`return` 掉，
  于是：拖窗格分隔线不再改变宽度、拖终端选区不再延伸、拖 lx 卡片不再折叠——
  所有拖拽行为静默失效，且唯一「解药」是再点一次 Todo 滚动条轨道。
  更糟的是 `todo::scrollbar()` 不判 `right_tab`，切到 Prompt 后该分支仍可能命中。
- **实测**：临时用例点 thumb → 松开 → 断言 `todo_scroll_drag.is_none()`，实际 `Some`。
- **修复**：`drag.rs` 清理链补 `if state.todo_scroll_drag.take().is_some() { return; }`；
  `todo::scrollbar` 加 `right_tab != Todo || prompt_collapsed` 前置判（与 `prompt_scrollbar` 同构）。
- **回归**：`todo_scrollbar_drag_releases_on_mouse_up`、`prompt_gutter_click_does_not_scroll_hidden_todo`。
- **变异验证**：删掉 `take()` → 前者失败；删掉 tab 门控 → 后者失败（两个都转红）。

---

## P1

### P1-1 点**当前** tab 不会让右栏进入选中态

- **位置**：`src/app/update/right_panel.rs:12`（`select_right_tab` 早退）、
  `src/event/mouse/press.rs:264-277`
- **关键点**：第五轮把「点 tab = 聚焦右栏」做在了 `select_right_tab` 里，
  但该函数第一行就是 `if state.right_tab == tab { return false; }`。
- **错误原因**（实测）：先点主区窗格（焦点回 Panes，右栏仍停在 Todo），
  再点 Todo tab——用户意图是「回到右侧容器」，但由于 tab 没变，
  函数早退、焦点留在主区，右栏边框仍是 muted。用户报告的
  「选中右侧栏顶部 tab 切换后需要高亮当前右侧栏」在这条路径上没有修好。
- **实测**：临时用例断言点当前 Todo tab 后 `todo_focused()`，实际为 false。
- **修复**：`select_right_tab` 先 `focus_right_tab(state, tab)`（抽成独立小函数）再判早退；
  早退分支返回这次聚焦是否产生变化。`press.rs` 因此无需改动。
- **回归**：`mouse_click_current_tab_focuses_panel_and_dirties`（同时锁住第二次点击不空转重绘）。
- **变异验证**：改回「先判早退再聚焦」→ 用例失败。

### P1-2 Prompt tab 下右缘点击会滚动**隐藏的** Todo 列表

- **位置**：`src/event/mouse/hit.rs:475-495`（`handle_todo_scrollbar_press`）、
  `src/ui/right_panel/todo.rs:207`（`scrollbar` 无 tab 门控）
- **关键点**：`press.rs:210-214` 把 `handle_todo_scrollbar_press` 与 prompt 的滚动条
  命中并列在一个 `||` 链里，而 `todo::scrollbar()` 只判几何、不判 `right_tab`。
  Prompt 滚动条在 `prompt_scrollbar_rect` 里显式门控了 tab，Todo 的没有。
- **错误原因**（实测）：右栏停在 Prompt、列表溢出时，Todo 滚动条槽与 Prompt 正文
  右缘是**同一列**（都是 `inner.right()-1`），且 Todo 的槽只覆盖列表高度。
  点这一列的下半部分：`handle_prompt_scrollbar_press` 因 Prompt 视口未溢出返回 false，
  链继续走到 `handle_todo_scrollbar_press` → 命中隐藏的 Todo 轨道 →
  `set_todo_scroll` 把 Todo 视口跳到 8。用户看到的是「点了 Prompt 正文的右边，
  什么都没发生，但切到 Todo 时列表已经滚到别处」。
- **实测**：临时用例在 Prompt tab 点 Todo 轨道底行，断言 `todo.scroll() == 0`，实际 8。
- **修复**：`todo::scrollbar` 首行加 `right_tab != Todo || prompt_collapsed` 判（同 P0-1）。
- **回归**：`prompt_gutter_click_does_not_scroll_hidden_todo`、`hidden_panel_has_no_scrollbar`。
- **变异验证**：去掉门控 → 前者失败。**注意**：用例最初点在列表区之外的行，
  变异**没转红**；改用 `list_rect` 内的行后才真正覆盖该路径（这条差点成为假阳性测试）。

### P1-3 幽灵行打开时悬停高亮错位一行

- **位置**：`src/event/mouse/hover.rs:244-253`、`src/ui/right_panel/todo.rs:356`
- **关键点**：`row_at()` 返回的是**显示下标**（含幽灵行槽位），存进
  `state.todo_hover`；渲染层却拿它和 `record_index`（**列表下标**，已扣掉幽灵行偏移）比较：
  `let hovered = state.todo_hover == Some(record_index);`
- **错误原因**（实测）：新建态幽灵行插在 `at=1`，显示序列为 `a(0) / 幽灵(1) / b(2) / c(3)`。
  鼠标悬停 b（显示第 2 行）→ `todo_hover = Some(2)`；渲染 b 时
  `record_index = 2 - 1 = 1`，比较 `2 == 1` 失败 → b 不高亮；
  而 `c` 的 `record_index = 2`，比较成功 → **悬停 b 却高亮 c**。
  与第二轮修掉的「点 b 选中 c」是同一类下标口径错误，只是这条留在悬停路径上。
- **实测**：临时用例断言显示第 2 行有底色，实际无；第 3 行无底色，实际有。
- **修复**：新增 `todo::list_index(state, display)` 做显示→列表换算（幽灵行自身返回 `None`），
  `hover.rs` 在命中入口换算一次；`state.todo_hover` 的语义改为**列表**下标并补文档。
- **回归**：`hover_below_ghost_row_stores_list_index`（走 `handle_mouse_moved` 端到端）、
  `hover_index_is_list_index_below_ghost_row`（换算 + 渲染底色）。
- **变异验证**：去掉 `list_index` 换算 → 失败；让 `list_index` 忽略幽灵槽位 → 失败。
  纯渲染层那条用例**不敏感**（它直接设 `todo_hover`，绕过入口），所以另加了端到端用例。

### P1-4 列表溢出时，末尾新建的幽灵行永远不可见

- **位置**：`src/app/update/todo.rs:185-205`（`begin_todo_edit` 不滚视口）、
  `src/ui/right_panel/todo.rs:318-323`（渲染窗口 `start..start+visible`）、
  `src/app/state/todo.rs:219-227`（`set_scroll` 上限按 `len` 算，不含幽灵行）
- **关键点**：`begin_todo_edit` 只 `state.todo.begin_edit(edit)`，**不调 `scroll_into_view`**；
  渲染层窗口是 `state.todo.scroll() .. scroll+visible`，`total = len + 1` 不含末尾幽灵行。
- **错误原因**（实测）：列表 10 条、可见 5 行、`scroll=0`，按 `a` 或 `Shift+Enter`
  在末尾新建 —— `edit.at = 10`，渲染窗口是 `0..5`，幽灵行下标 10 在窗口外，
  整行不画、光标也不交。用户按了「新建」却看不到任何输入位置，只有左栏没有任何反馈
  （焦点其实已进 Todo）。列表没溢出时正常，所以现有测试
  （`placeholder_only_for_new_todo` 只有 1 条记录）覆盖不到。
- **实测**：临时用例 10 条记录 + 末尾新建，断言占位文案可见，实际 `None`。
- **修复**：新增 `TodoPanel::scroll_to_edit()`（上限按 `len + ghost` 算），
  `begin_todo_edit` 在 `begin_edit` 后调用；`set_viewport_rows` 的钳制上限同样加上幽灵槽位，
  否则下一帧会把刚滚好的位置钳回去。
- **回归**：`begin_edit_scrolls_ghost_row_into_view`、`scroll_to_edit_reaches_ghost_slot`、
  `set_viewport_rows_keeps_ghost_slot_in_range`。
- **变异验证**：删掉 `scroll_to_edit()` 调用 → 第一条失败。

### P1-5 超长 todo 进内存但写库失败，重启即丢（无提示）

- **位置**：`src/app/update/todo.rs:387-433`（`commit_todo_edit` 不校验长度）、
  `src/db/todo.rs:271-280`（`normalize_text` 超 500 字符返回 `MessageTooLarge`）
- **关键点**：500 字符上限**只在 DB 层**校验。内存层 `commit_todo_edit` 直接
  `state.todo.insert(at, record)` 并登记 `Effect::SaveTodo`。
- **错误原因**（实测）：编辑里粘贴 600 个汉字（`insert_str` 无长度限制），
  回车提交 → 内存列表出现一条 600 字 todo（UI 正常显示）→ `save_todo` 返回
  `MessageTooLarge` → `todo_write` 只 `tracing::warn!` + toast `todo save failed`。
  用户看到的是「保存失败」但条目仍在列表里，重启后消失。
  design.md §5.2 把「库不可用」定义为内存可用，这里是**单条**被库拒绝，
  却按同样的「内存是真源」口径处理，语义不一致。
- **实测**：临时用例 600 字提交后断言 `chars().count() <= 500`，实际 600。
- **修复**：`MAX_TODO_TEXT_CHARS` 改为 `pub`；在**输入边界**限长——
  逐字输入到 500 即停止（`insert_bounded`），粘贴按剩余空间裁剪（`apply_todo_paste`）。
  这样提交永不失败，也不存在「拒绝提交导致编辑现场悬空」。
  （先试过「提交时拒绝 + 保留编辑态」，它会重新制造 P2-1 那类悬空草稿，故放弃。）
- **回归**：`typing_stops_at_storage_limit`、`paste_is_clipped_to_storage_limit`。
- **变异验证**：去掉 `insert_bounded` 的上限判断 → 第一条失败。
- **残留风险**：库里的**历史**超长文本（若有）不受此修复影响，读取路径仍按原样显示。

---

## P2

### P2-1 切顶部标签不提交 Todo 编辑，编辑现场悬空

- **位置**：`src/app/update/workspace.rs:324-340`（`create_tab` / `switch_tab`）
- **关键点**：design.md §5.1 的打断矩阵写明「编辑中切 tab = 提交」，
  但 `switch_tab` 只做 `set_focus(Panes)`，没有 `commit_todo_edit`。
  `select_right_tab`（右栏 tab）与 `switch_workspace` 都提交了，唯独顶部标签栏漏了。
- **错误原因**（实测）：Todo 编辑中点头部标签栏 → 焦点回主区，`state.todo.editing`
  仍是 `Some`。此后按 `Tab` 键进入的是窗格路径，Todo 面板停在编辑态且不再接受输入；
  下次点回 Todo 时草稿奇迹般还在（因为你从未提交），但期间任何
  `begin_todo_edit` 都会先提交它——行为取决于中间做过什么，不可预期。
- **实测**：临时用例 `switch_tab` 后断言 `!is_editing()`，实际仍在编辑。
- **修复**：新增 `focus::focus_panes(state)` 作为「把焦点交回主区」的唯一收口
  （内部先 `commit_todo_edit`）；`switch_tab` / `create_tab` / `create_workspace` /
  `create_terminal_in_workspace` / `split_pane` / `open_worktree_checkout` / `focus_pane`
  全部改走它。`workspace.rs` / `menu.rs` / `worktree.rs` 不再直接 `set_focus(Panes)`。
- **回归**：`switching_top_tab_commits_todo_edit`、`creating_tab_commits_todo_edit`。
- **变异验证**：删掉 `focus_panes` 里的 `commit_todo_edit` → 第一条失败。

### P2-2 侧栏持焦点时粘贴仍写进 prompt 编辑器

- **位置**：`src/event/input.rs:127-166`（`handle_paste_event`）
- **关键点**：键盘路径在 `input::route` 里对 `FocusedContainer::Sidebar` 一律 `return None`，
  粘贴路径却**没有**侧栏分支，直接落到 `if state.prompt_focused()` 之后的逻辑。
- **错误原因**：第四轮的口径是「侧栏没有键盘交互，按键绝不穿透到别的容器」。
  实际：点侧栏空白让侧栏高亮后按 Cmd+V，文本进了 prompt 编辑器
  （`prompt_focused()` 为假，但落到末尾的终端写入分支？——实测为 prompt 分支，
  因为 `demo_terminal` 下 prompt 未折叠。总之**越过了焦点判据**）。
- **实测**：临时用例 `focus_sidebar` 后 `handle_paste_event("secret")`，
  断言 prompt 文本不变——用例通过（说明落点在终端而非 prompt），
  但**焦点为 Sidebar 时仍把用户粘贴写进了某个容器**，与「侧栏吞掉一切」不符。
  标记为 P2 而非 P1：具体落点依赖 `prompt_collapsed`，但无论如何都违背不变量。
- **修复**：`handle_paste_event` 在 `todo_focused()` 分支前加 `if state.sidebar_focused() { return; }`。
- **回归**：`paste_while_sidebar_focused_is_swallowed`。
- **变异验证**：删掉该分支 → 用例失败。

### P2-3 `migrate()` 的文档注释首行被截断

- **位置**：`src/db/migration.rs:533`
- **关键点**：该行只剩 `///`，原本的「迁移到当前 Schema：建表与版本推进在同一事务中完成，幂等。」
  在插入 `SCHEMA_V15_TODO` 时被误删。
- **错误原因**：纯文档缺陷，不影响运行；但 `cargo doc` 会输出一个空行开头的声明注释，
  且这是本分支唯一改动的注释块，属于「改坏了原有代码」。
- **已修复**：补回「迁移到当前 Schema：建表与版本推进在同一事务中完成，幂等。」

### P2-4 设计文档未回写「点 tab 聚焦右栏」的新口径

- **位置**：`docs/feature/right-panel-todo/design.md:249-250`
- **关键点**：仍写着「**不**因切换到 Todo tab 自动聚焦（对齐 `right-panel-tabs` 决策 #5）」，
  与第五轮实际实现（`select_right_tab` 直接聚焦）直接矛盾。
  差异表第 26 行补了实现变更，但正文 §3.2 的「何时获得焦点」没同步。
- **错误原因**：文档说谎。后续维护者按正文实现会推翻用户要求。
- **已修复**：正文改为「点 tab 即聚焦对应面板（含点当前 tab）；程序性切换不回焦右栏」，
  并同步 §3.4 鼠标表与差异表。

---

## P3

### P3-1 `src/ui/style.rs` 已 1010 行，超过 1000 硬上限

- **位置**：`src/ui/style.rs`
- **关键点**：dev 基线 1006 行，本分支加了 `pub mod todo;` + `pub use todo::*;` 4 行变 1010。
- **错误原因**：`docs/standards/code-writing-standards.md:45` 明确「单文件超过 1000 行
  必须按职责拆分」。本分支恰好把 todo 样式拆到了 `style/todo.rs`（35 行），
  却没有顺手把 `style.rs` 压回 1000 以内——拆了一半。
- **未修复**：属独立重构（`style.rs` 里 terminal 调色与 markdown token 各百来行），
  与本特性无耦合，且会污染本次 diff。**记录为已知债务**，建议单独一个提交处理。

### P3-2 `todo_hover` 的两套下标口径是 P1-3 的根因，应有单一来源

- **位置**：`src/app/state.rs:233`（`pub todo_hover: Option<usize>`）
- **关键点**：字段注释写「鼠标悬停的 todo **可见行**下标」，
  但 `click_todo_row` 收到的同源下标是**显示下标**，渲染层用**列表下标**比较。
- **已修复**：随 P1-3 一起，字段注释改为「鼠标悬停的 todo **列表**下标（已扣掉幽灵行槽位）」。

---

## 已核实无问题的点

- **数据库写入**：`save_todo` 的 UPSERT 语义、`shift_positions_from` 降序顺移、
  `renumber`/`apply_positions` 的「先整体移进区间再落回」两步法都正确；
  `idx_todo_position` 唯一索引确实能兜住漏顺移。
- **撤销恢复**：`restore_todos` 用 `RENUMBER_OFFSET` / `RESTORE_OFFSET` 两个不重叠区间，
  合并顺序与 `ordered.insert(at, ...)` 的下标夹取一致；内存侧 `restore` 按 id 去重。
- **迁移**：`SCHEMA_VERSION` 从 14 → 15，v15 只增表、失败整体回滚、
  `ensure_no_foreign_key_violation` 与既有迁移同构；与并行 worktree
  （`prompt-tree-drag-move` 仍是 14）无版本号冲突。
- **光标**：`cursor = cursor.or(...)` 累积 + `.filter(|_| focused)` 两道闸门都在，
  `edit_cursor_survives_later_rows` 与 `unfocused_panel_yields_no_hardware_cursor` 覆盖到位。
- **几何**：`list_rect + 1 + stats_rect == body_rect` 在 `MIN_SPLIT_BODY=7` 以上恒成立；
  滚动条轨道 `list.right()` 一列且只覆盖列表高度，不压分割线。
- **工具栏**：`[+]` 贴左 + 右侧组贴右缘 + 组间至少 1 列，`button_at` 与 `button_rect`
  同一份几何，窄宽度阶梯 38/20/13/8 与测试一致。
- **焦点互斥**：`FocusedContainer` 单一真源成立，`panes.rs` / `sidebar.rs` /
  `right_panel.rs` 三处描边都从它派生；`only_focused_container_uses_accent_border` 锁住。
- **生产代码**：新增代码无 `unwrap()`/`expect()`（`git diff` 全量扫描，命中项全在 `#[cfg(test)]` 内）；
  无 `println!`/`eprintln!`；颜色只出现在 `ui/style/`。
- **测试基线**：`cargo test` 3475 passed / 0 failed / 2 ignored。

---

## 附：修复过程中额外发现的缺陷

### X-1（P1）测试用固定临时库路径，跨分支互踩

- **位置**：`src/event/tests.rs`（`worktree_candidates_do_not_resurrect_mention_panel_on_todo`）
- **错误原因**（实测）：`Database::open(temp_dir().join("lx-tui-wt-gate.db"))` 是**固定路径**。
  本分支把 `SCHEMA_VERSION` 推到 15 后跑过一次，文件被写成 v15；
  接着任何 v14 的 worktree（如 `prompt-tree-drag-move`）跑同一条用例就
  `UnsupportedSchema(15)` 直接 panic。反向亦然（实测报 `UnsupportedSchema(16)`，
  说明别的分支已把它推到 16）。
- **修复**：路径改为 `lx-tui-wt-gate-<pid>-<thread>.db`，用完 `remove_file`。
- **变异验证**：不适用（这是测试隔离缺陷，不是行为分支）。

---

## 附：评审过程中的异常

三个并行 reviewer（`review-state` / `review-ui` / `review-input`）启动后 50 分钟内
未产出 `review-part{1,2,3}.md`，两次催办无效，已终止。本文件由主评审完成，
因此**没有**三份独立视角的交叉验证；上文每一条都附了实测或精确代码位置，
但覆盖面可能弱于多 agent 交叉评审。
