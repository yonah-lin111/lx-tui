# 右侧容器 Todo 面板 — 执行任务

- 任务目录：`docs/feature/right-panel-todo/`
- 分支：`feat/right-panel-todo`（worktree：`.worktrees/right-panel-todo`，基于 `dev`）
- 规则：严格遵守 `AGENTS.md`；生产代码禁止 `unwrap()` / `expect()`（测试除外）；
  用户可见文案进 `ui/text.rs`（超行数上限则进 `ui/text/todo.rs`）；颜色只从 `ui/style.rs` 取；
  `app/` 不碰 rusqlite，`ui/` 不改状态不做 IO；测试驱动（先红后绿）。
- 需求原文（用户）：参考 lx-agent 的 todo 编写功能，在右侧容器的 todo 列表中实现 todo 编写；
  支持 CRUD；显示 todo 图标（字符构成）；输入交互与快捷键参考 lx-agent；功能按钮区加按钮；
  todo list 加滚动条；另外把 prompt 区域置顶与全选的字符改成英文。
- 前置：用户已审核并批准同目录 `design.md` 与本清单（17 条 Grill Me 边界已逐条确认）。

## 执行清单

### 1. 需求与边界对齐（Grill Me 阶段，已完成）

- [x] 质问 1：落 SQLite → 用户修正为「全局一份，不按工作区区分」。
- [x] 质问 2：扁平三态 + `position`，不做父子层级 → 采纳推荐。
- [x] 质问 3：inline 单行编辑，Esc 撤销式取消 → 采纳推荐。
- [x] 质问 4：`todo_focused` 独立布尔，与 `prompt_focused` 平行互斥 → 采纳推荐。
- [x] 质问 5：只改 `[pin]`/`[unpin]`/`[all]`，其余按钮保持原字符 → 用户选 A。
- [x] 质问 6：纯用户清单，agent 不读不写 → 采纳推荐。
- [x] 质问 7：`Space` 循环三态 + 按键表 → 采纳推荐（用户对问题 6 回复「使用推荐」，按上下文归入此条）。
- [x] 质问 8：逐条提交落库，失败只 toast 不回滚 → 用户确认「对」。
- [x] 质问 9：`position` 密集整数 + 事务内顺移；不做拖拽与 `J`/`K` 重排 → 采纳推荐。
- [x] 质问 10：右侧 1 列滚动条 + 滚轮 3 行；滚轮不动高亮，键盘移动自动入视口 → 采纳推荐。
- [x] 质问 11：4 个按钮；`[clear done]` 不确认、只删 done、可撤销；无效置灰；空态 `no todos yet` → 采纳推荐。
- [x] 质问 12：`[ ]`/`[~]`/`[x]` + `❯`，不引入圆点/方块字形 → 采纳推荐。
- [x] 质问 13：Esc 取消、切 tab/点别行提交；空输入不建、清空即删；提交 trim → 采纳推荐。
- [x] 质问 14：单槽删除撤销；`Ctrl+Z` 无内容时吞掉不穿透 → 采纳推荐。
- [x] 质问 15：切工作区完全不变；编辑中切工作区走提交 → 用户选 A。
- [x] 质问 16：todo 跟随所在数据库，不做跨 worktree 同步 → 采纳推荐。
- [x] 质问 17：先出 design.md + task.md，用户审核后开工 → 用户确认「对」。
- [x] 用户审核 `design.md` 与本清单，确认后开始实施（用户下达「执行 task」指令）。

### 2. 复现测试（先红）

- [x] `src/db/todo/tests.rs`（新，18 条）：§6.1 全部 8 条。
- [x] `src/db/migration/tests.rs`：`v14_database_upgrades_to_v15_with_todo_table`。
- [x] `src/app/update/todo/tests.rs`（新，13 条）。
- [x] `src/ui/right_panel/todo/tests.rs`（新，7 条）。
- [x] `src/event/tests.rs`：新增 5 条 + 改写 3 条既有「吞掉」用例。
- [x] `src/ui/text/tests.rs`：改写 `prompt_button_icons_are_single_width_and_non_pua`、
      新增 `todo_labels_align_to_three_columns`。
- [ ] 确认上述用例在改动前确实失败（红），记录失败原因。

### 3. 数据库层（`src/db/`）

- [ ] `src/db/migration.rs`：新增 `SCHEMA_V15_TODO` 常量（表 + 唯一索引），
      `migrate()` 尾部按 v14 写法追加 `if version < 15 { ... }`。**不改任何历史迁移文本**。
- [ ] `src/db/mod.rs`：`SCHEMA_VERSION = 15`；`pub mod todo;`。
- [ ] `src/db/todo.rs`（新，约 200 行）：`list_todos` / `save_todo`（UPSERT）/
      `delete_todo` / `delete_todos_with_status` / `restore_todos`；
      文本 trim 校验、500 字符上限、`now_ms()`；顺移用「SELECT id ORDER BY position DESC
      + 逐条 UPDATE」（不依赖 `SQLITE_ENABLE_UPDATE_DELETE_LIMIT`）。
- [ ] `TodoStatus` / `TodoRecord` 定义在 `db/todo.rs`，`app` 侧只做类型转换使用
      （与 `app/state/lx/compact.rs:6` 引用 `db::chat::CompactCheckpoint` 同口径）。

### 4. 状态层（`src/app/`）

- [ ] `src/app/state/todo.rs`（新，约 250 行）：`TodoPanel`、`TodoEdit`、`TodoEditTarget`，
      含 `clamp_selection` / `scroll_into_view` / `visible_rows` 等纯方法。
- [ ] `src/app/state.rs`：`pub mod todo;` + `pub use todo::{TodoEdit, TodoEditTarget, TodoPanel};`；
      `AppState` 新增 `todo_focused` / `todo` / `todo_scroll_drag` / `todo_hover`；
      `demo()` 初始化；`#[cfg(test)]` 下既有 `state` 构造点同步（若有手写全字段的地方）。
- [ ] `src/app/effects.rs`：新增 4 个变体 `SaveTodo` / `DeleteTodo` /
      `DeleteTodosWithStatus` / `RestoreTodos`。
- [ ] `src/app/update/todo.rs`（新，约 400 行）：§4.5 全部函数 + `apply_todo_key` /
      `apply_todo_paste` / `apply_todo_list` / `fail_todo_write`。
- [ ] `src/app/update.rs`：`mod todo;` + `pub use todo::*;`。
- [ ] `src/app/update/right_panel.rs::select_right_tab`：切走 Todo 时 `commit_todo_edit` 并清
      `todo_focused` / `todo_hover`；切走 Prompt 的既有逻辑保持。
- [ ] `src/app/update/editor.rs::focus_prompt`：追加 `todo_focused = false`。
- [ ] `src/app/update/pane.rs:68` 附近：聚焦窗格时清 `todo_focused`。
- [ ] `src/app/update/workspace.rs::switch_workspace`：切工作区前 `commit_todo_edit`
      （注意：它现在也会 `prompt_focused = false`，Todo 的列表/选中/滚动/焦点**不得**被清）。
- [ ] `src/app/update.rs::apply(Action::TogglePrompt)`：折叠时若 `todo_focused` 则先提交再失焦。
- [ ] `src/app/actions.rs`：**不加** `Action` 变体（todo 命令带字符，走 `Routed::Todo`）。
- [ ] `src/app/update/interaction.rs`：`tick` / `next_deadline` **不改**（Todo 无定时逻辑）。

### 5. 输入与事件层（`src/input/`、`src/event/`）

- [ ] `src/input/mod.rs`：`Routed::Todo(TodoKey)`、`TodoKey` 枚举、`todo_key(key)` 无状态映射；
      `route` 新增尾参 `todo_focused: bool`，插在 `prompt_focused` 分支之后、`pane_lx` 之前；
      `Ctrl+Q` 优先级不变。
- [ ] `src/input/tests.rs` / `process_keys.rs` / `db/service/resume_tests.rs`：
      更新 `route` 调用点（补 `false` 实参），新增 Todo 路由用例。
- [ ] `src/event/input.rs`：`route` 调用补实参；`match` 新增 `Routed::Todo`；
      粘贴路径在 `prompt_focused` 分支前新增 `todo_focused` 分支。
- [ ] `src/event/mouse/press.rs`：§4.9 的命中链改造（按钮 → 行 → 滚动条 → 空白吞掉），
      插在既有「整块吞掉」分支之前。
- [ ] `src/event/mouse/hit.rs`：新增 `handle_todo_scrollbar_press`。
- [ ] `src/event/mouse/wheel.rs`：Todo 分支改为滚动列表。
- [ ] `src/event/mouse/hover.rs`：新增 `todo_hover` 计算与 `set_todo_hover`。
- [ ] `src/event/mouse/drag.rs`：新增 `todo_scroll_drag` 分支；`handle_mouse_up_left` 补 `take()`。

### 6. 渲染层（`src/ui/`）

- [ ] `src/ui/right_panel.rs`：改为目录模块（`pub(crate) mod todo;`），Todo 分支调
      `todo::render(frame, area, state)`；删除 `render_empty_body`。
- [ ] `src/ui/right_panel/todo.rs`（新，约 350 行）：§4.8 全部几何 + 绘制 + 命中 + 光标坐标。
- [ ] `src/ui/text.rs`（或 `src/ui/text/todo.rs`）：§4.10 全部常量 + tooltip 文案；
      改 `PROMPT_SELECT_ALL_LABEL` / `PROMPT_UNPINNED_LABEL` / `PROMPT_PINNED_LABEL`。
- [ ] `src/ui/style.rs`（或 `src/ui/style/todo.rs`）：§4.11 三个 token。
- [ ] `src/ui/tooltip.rs`：Todo 按钮 tooltip 目标（`TooltipTarget::TodoButton`）。
- [ ] `src/ui/mod.rs`：**预期无改动**——Todo 光标坐标走 `right_panel::render` 的返回值，
      与 `prompt_cursor` 同一条链路（`src/ui/mod.rs:96`）；若实测被 tooltip 判定吞掉再补条件。
- [ ] `src/layout.rs`：只改 `PROMPT_SELECT_ALL_WIDTH`(3→5) / `PROMPT_PIN_WIDTH`(3→7)
      两个数值，**零新增行**；派生阈值随之变为 19/27/18/33（见 `design.md` §4.10 的表），
      并确认「已知副作用」贴中记载的 `[dir]` 阈值上升确实发生、且行为符合预期。

### 7. 服务层（`src/db/service.rs`）

- [ ] `execute` 新增 4 个 `Effect` 分支（各 ≤ 6 行，超则抽 `db/service/todo.rs`）。
- [ ] `Persistence::load_todos(&self, state: &mut AppState)`。
- [ ] `src/main.rs:31` 附近追加 `persistence.load_todos(&mut state);`。
- [ ] `Persistence::shutdown`：`commit_editing_prompt` 之后追加 `update::commit_todo_edit(state)`。

### 8. 测试与文档

- [ ] 补齐 §2 列出的全部用例并转绿；改写因语义变化而失效的既有用例
      （`src/event/tests.rs:4980-5020`、`src/ui/tests.rs:1437-1441`、
      `src/ui/right_panel/tests.rs` 中 Todo 空白体相关断言）。
- [ ] 同步 `docs/standards/tui-design-requirements.md:25`（按 `design.md` §7 三点改写）
      与工具栏文案段。
- [ ] 回写本目录 `design.md` §9「实施记录（与设计的差异）」。

### 9. 精确验证

- [ ] `cargo fmt`
- [ ] `cargo clippy --all-targets -- -D warnings`
- [ ] `cargo test db::todo`、`cargo test db::migration`、`cargo test app::update::todo`、
      `cargo test ui::right_panel`、`cargo test input::`、`cargo test event::`
- [ ] `cargo test`（全量）
- [ ] 交回用户 `cargo run` 人工验收 §5.3 的 7 条视觉要点（Agent 无法交互式验证 TUI）。

## 评审与修复

实施期自查发现并修掉的真缺陷（无并行 reviewer，全部由我自己在验证过程中发现）：

1. **新建 todo 时屏幕空白**：编辑现场只渲染既有条目，新建没有行可画。加「幽灵行」（`render_rows` 的 `pending_new` 分支）。
2. **点行后下标错位**：`select_todo_row` 先提交再按下标选中，而提交会让幽灵行消失、后续下标整体前移。改为先判「点的是不是正在编辑的那一行」。
3. **`[pin]` 留下残影**：按钮宽度改为最宽态 7 后，`[pin]` 只写 5 列，后 2 列保留上一帧的 `n]`。`render_toolbar` 改为整格填充。
4. **`restore` 造重复条目**：撤销槽被恢复两次会在列表里插出两份同 id 记录。改为按 id 去重。
5. **按钮亮着却点不动**：渲染层的置灰判定与点击判定是两份代码。统一为 `update::todo_button_enabled`。
6. **`set_viewport_rows` 恒返回 false**：只改了 scroll 就返回 false，视口变化不置脏。改为「rows 变了就返回 true」。
7. **滚动条列被正文覆盖**：`write_string` 用帧右缘当上限，长条目会盖掉 thumb。改为 `write_string_in` 按列表右缘裁剪。


## 完成记录

- 分支：`feat/right-panel-todo`（worktree：`.worktrees/right-panel-todo`，基于 `dev` @ `8707aa7`）。**尚未合并**，等用户确认。
- Schema：`SCHEMA_VERSION` 14 → **15**（新增全局 `todo` 表 + `idx_todo_position` 唯一索引）。
- 新增文件：`src/db/todo.rs`(+tests)、`src/app/state/todo.rs`(+tests)、`src/app/update/todo.rs`(+tests)、
  `src/ui/right_panel/todo.rs`(+tests)、`src/ui/text/todo.rs`(+tests)、`src/ui/style/todo.rs`。
- 验证：`cargo fmt --check` 通过；`cargo clippy --all-targets -- -D warnings` 通过；
  `cargo test` 全绿（lib **3433 passed / 0 failed / 2 ignored**，其余 15 个测试目标全 ok）。
- 文件行数：`src/layout.rs` 保持 **999** 行（零新增行，只改 2 个常量值）；
  `src/ui/text.rs` 1269、`src/ui/style.rs` 1010（新增代码已拆入子模块）、`src/db/service.rs` 1273。
- 未做项：无。
- 待人工验收（`cargo run`）：§5.3 的 7 条视觉要点，尤其是 `[pin]`/`[all]` 与旧图标的观感对比，
  以及 `[dir]` 从内宽 27 涨到 33 后「面板宽 29~34 时 `[dir]` 消失」这一已知副作用。
