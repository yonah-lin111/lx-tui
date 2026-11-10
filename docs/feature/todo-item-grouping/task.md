# 右侧 Todo 面板条目聚合与拆分（Todo Item Grouping）执行清单

- **任务目录**：`docs/feature/todo-item-grouping/`
- **设计文档**：`docs/feature/todo-item-grouping/design.md`
- **评审文档**：`docs/feature/todo-item-grouping/review.md`
- **当前状态**：已完成（Phase 1~5 实现与 Phase 6 代码评审 P0~P3 共 10 项修复及回归测试全部通过）
- **核心开发铁律**：
  - 生产代码禁止 `unwrap()` / `expect()`，错误统一用 `Result` + `?` 传播；
  - 所有 TUI 可见文案（菜单项、提示）必须使用英文并统一定义在 `src/ui/text/todo.rs`；
  - 所有语义样式必须从 `src/ui/style/todo.rs` 获取；
  - `app/`（纯状态与更新）与 `ui/`（纯只读渲染）边界严格隔离；
  - 生产代码单文件硬上限 1000 行，拆分后子文件控制在 500 行左右，测试文件同步按职责镜像拆分。

---

## 需求摘要

在 `src/ui/right_panel/todo/` 及配套状态/持久层中，支持将多条一起执行的 Todo 条目聚合成一个共享状态的大条目，并支持单条拆出与整组解散：

```text
    ┌─ P1 写登录接口
[ ]-├─ P1 写登录单元测试
    └─ P2 修首页样式
```

---

## 执行清单（Phased Checklist）

### Phase 1：数据库 Schema v26 升级与 `group_id` 持久化

- [x] **1.1 升级 `SCHEMA_VERSION` 与迁移脚本**
  - 文件：`src/db/mod.rs`、`src/db/migration/schema.rs`、`src/db/migration.rs`
  - 关键改动：
    - `SCHEMA_VERSION` 从 `25` 提升为 `26`；
    - `SCHEMA_V15_TODO`（新库建表）与 `SCHEMA_V26_TODO_GROUP`（增量索引 `idx_todo_date_group(entry_date, group_id)`）补充 `group_id TEXT`；
    - `Database::migrate` 新增 `if version < 26 { self.migrate_v26_todo_group()?; }`，通过 `column_exists(&transaction, "todo", "group_id")` 幂等执行 `ALTER TABLE todo ADD COLUMN group_id TEXT;`。
- [x] **1.2 扩展 `TodoRecord` 与 `Database` 读写方法**
  - 文件：`src/db/todo.rs`、`src/app/effects.rs`、`src/db/service.rs`
  - 关键改动：
    - `TodoRecord` 新增 `pub group_id: Option<String>` 与链式构造器 `pub fn with_group_id(mut self, group_id: Option<String>) -> Self`；
    - 更新 `map_todo_row`、`list_todos`、`list_todos_by_date`、`list_past_incomplete_todos`、`todo_by_id`、`save_todo`、`restore_todos`，完整读写 `group_id`；
    - 新增 `Database::sync_date_todos(&self, entry_date: &str, records: &[TodoRecord]) -> Result<()>`，在单事务内原子同步某日所有条目的 `status`、`priority`、`completed_date`、`group_id` 与密集 `position`；
    - `Effect` 新增 `SyncDateTodos { entry_date: String, records: Vec<TodoRecord> }` 并在 `src/db/service.rs` 接线。
- [x] **1.3 数据库层单元测试**
  - 文件：`src/db/todo/tests.rs`、`src/db/migration/tests.rs`
  - 验证 v25 -> v26 迁移幂等性、`group_id` 读写以及 `sync_date_todos` 唯一索引安全。

---

### Phase 2：状态层子模块拆分与聚合组核心算法

- [x] **2.1 拆分 `src/app/state/todo.rs`（967 行 -> 3 个子文件）**
  - 文件：
    - `src/app/state/todo.rs`（约 480 行）：保留 `TodoPanel` 主结构、视口滚动、编辑现场 `TodoEdit`、基础增删改；
    - `src/app/state/todo/group.rs`（新，约 280 行）：聚合组角色投影、不变量归一化、合并/拆分/保组排序；
    - `src/app/state/todo/schedule.rs`（新，约 220 行）：跨日切换、历史未完成顺延、7 日周带与趋势统计。
- [x] **2.2 实现聚合组不变量与操作方法（`src/app/state/todo/group.rs`）**
  - 关键数据结构与方法：
    ```rust
    pub enum TodoGroupRole {
        Single,
        Member {
            group_id: String,
            offset: usize,
            count: usize,
        },
    }
    ```
  - 实现 `normalize_slice_groups_and_positions`：保证 ① 同组必连续；② 同组必同状态；③ `count < 2` 自动清除 `group_id`；④ `position == index`；
  - 实现 `TodoPanel::group_role(&self, index: usize) -> TodoGroupRole`；
  - 实现 `TodoPanel::merge_with_next(&mut self, index: usize) -> bool`（向下聚合相邻项/组，对齐当前状态）；
  - 实现 `TodoPanel::merge_item_into(&mut self, from_index: usize, target_index: usize) -> bool`（拖拽将任意项并入目标项/组，对齐目标状态）；
  - 实现 `TodoPanel::split_from_group(&mut self, index: usize) -> bool`（将当前子项移出聚合组到组末下方，若原组仅剩 1 条自动退化）；
  - 实现 `TodoPanel::ungroup_at(&mut self, index: usize) -> bool`（整组原地解散）；
  - 改造 `TodoPanel::cycle_status(&mut self, index: usize) -> Vec<TodoRecord>`（同组成员状态与 `completed_date` 一起循环切换）；
  - 改造 `TodoPanel::sort_by_priority(&mut self) -> Option<Vec<String>>`（聚合组按组内最高优先级整体参与外部排序，组内成员按优先级升序重排）；
  - 扩展 `TodoEdit` 增加 `group_id: Option<String>` 与 `initial_status: TodoStatus`，支持在组内按 `o` / `Shift+Enter` 直接插入同组子项。
- [x] **2.3 状态层单元测试**
  - 文件：`src/app/state/todo/tests.rs`
  - 覆盖相邻合并、跨行拖拽合并、单条拆出、2 条组拆出/删除自动退化、整组解散、整组状态循环、保组优先级排序。

---

### Phase 3：行为更新层拆分、拖拽状态机、快捷键与右键菜单

- [x] **3.1 拆分 `src/app/update/todo.rs`（949 行 -> 4 个子文件）**
  - 文件：
    - `src/app/update/todo.rs`（约 360 行）：焦点、点击、编辑提交/取消、右键菜单组装、视口同步；
    - `src/app/update/todo/group.rs`（新，约 180 行）：`group_todo_with_next`、`split_todo_from_group`、`ungroup_todo_all`、`click_todo_status`；
    - `src/app/update/todo/drag.rs`（新，约 180 行）：`TodoDrag` 状态机（`begin_todo_row_press`、`drag_todo_to`、`finish_todo_drag`）；
    - `src/app/update/todo/key.rs`（新，约 260 行）：列表态（新增 `m` / `u` / `U`）与编辑态按键处理。
- [x] **3.2 扩展右键菜单与文案**
  - 文件：`src/ui/text/todo.rs`、`src/app/overlay.rs`、`src/app/update/menu.rs`
  - 新增 `MENU_TODO_GROUP_NEXT = "Group with next"`、`MENU_TODO_SPLIT_GROUP = "Split from group"`、`MENU_TODO_UNGROUP_ALL = "Ungroup all"`；
  - `MenuCommand` 新增 `GroupTodoWithNext`、`SplitTodoFromGroup`、`UngroupTodoAll` 并在 `execute_menu_command` 中分派；
  - `open_todo_item_menu` 根据目标条目是否有下一项、是否处于聚合组内动态生成菜单项。
- [x] **3.3 接入鼠标事件层（点击状态框 + 拖拽并组）**
  - 文件：`src/app/state.rs`、`src/event/mouse/press.rs`、`src/event/mouse/drag.rs`
  - `AppState` 新增 `pub todo_drag: Option<TodoDrag>`；
  - `press.rs`：新增 `ui::right_panel::todo::status_at` 命中分支 -> `update::click_todo_status`；条目行点击改为 `update::begin_todo_row_press`；
  - `drag.rs`：拖动阶段处理 `state.todo_drag` 实时更新落点高亮，松开阶段调用 `update::finish_todo_drag` 完成拖拽合并或清理状态。

---

### Phase 4：新 Item UI 渲染与几何命中层

- [x] **4.1 拆分 `src/ui/right_panel/todo.rs`（947 行 -> 拆出 `item.rs`）**
  - 文件：
    - `src/ui/right_panel/todo.rs`（约 460 行）：面板布局、工具栏、`VisibleList` 几何、命中函数（`row_at` / `status_at` / `priority_at` / `edit_input_char_at`）、滚动条；
    - `src/ui/right_panel/todo/item.rs`（新，约 360 行）：`RowView` 组装、树形连接符 `┌─ / ├─ / └─ / │` 绘制、共享状态框锚点绘制、优先级徽标与折行正文/行内编辑绘制。
- [x] **4.2 实现单条与聚合组自适应列宽与树形连接符渲染**
  - 普通单条：首行缩进 9 列（`❯ [ ] P1 `）、续行缩进 6 列（`      `）；
  - 聚合组子项：首行缩进 12 列（`❯ [ ]-├─ P1 ` 或 `      ┌─ P1 `）、续行缩进 9 列（非末项 `      │  `，末项 `         `）；
  - `viewport_metrics` 与 `layout_visible_items` 按条目是否属于聚合组（含组内新建幽灵行）分别传入对应的首行/续行宽度，确保折行行数、滚动条高度、光标坐标与点击命中 100% 同源；
  - 实现 `status_at`（命中普通项首行或聚合组锚点行的 `[ ]` 3 列区域）与 `priority_at`（按单条 `+6..+8` / 组内 `+9..+11` 精确命中首行 `P0..P3`）；
  - 支持拖拽过程中的视觉反馈（被拖源行 `style::muted()`，目标合并行/组 `style::selection()`）。
- [x] **4.3 UI 渲染与事件集成测试**
  - 文件：`src/ui/right_panel/todo/tests.rs`、`src/event/tests/todo.rs`
  - 验证 2 条/3 条聚合组连接符字形、共享 `[ ]-` 位置、非末项折行 `│` 与末项折行留空、状态框与优先级徽标点击命中、鼠标拖拽合并与双击编辑互不干扰。

---

### Phase 5：`lx-tui todo` CLI 同步与全量回归验证

- [x] **5.1 扩展 `lx-tui todo` CLI**
  - 文件：`src/cli/todo.rs`、`src/cli/todo/exec.rs`、`src/cli/tests/prompt_and_todo_tests.rs`
  - JSON 输出包含 `group_id`；
  - `done` / `start` / `reopen` / `update --status` 对同组成员联动更新；
  - 新增 `lx-tui todo group` 与 `lx-tui todo ungroup` 子命令并补充 CLI 测试。
- [x] **5.2 代码规范与测试验证**
  - 核对所有新增/修改文件的行数均在规范上限内（生产文件 < 1000 行，子模块 ~500 行）；
  - 执行 `cargo fmt --check`；
  - 执行 `cargo clippy --all-targets -- -D warnings`；
  - 执行定向与全量 `cargo test`。

---

### Phase 6：代码评审（`review.md`）与 P0~P3 缺陷收敛

- [x] **6.1 P0 修复（数据错位、2 人组删后撤销丢组、DB 同组连续与孤儿清理不变量）**
  - **P0-1**：`resolve_display_index` 补齐 `Some((ghost, true)) if display == ghost => return None`；`begin_todo_row_press` 在消费在途编辑提交时不登记陈旧 `todo_drag`；
  - **P0-2**：`remove_todo_at` 删除 2 人组子项时连带保存幸存伙伴快照，`TodoPanel::restore` 原地恢复伙伴 `group_id`，`Database::restore_todos` 末尾执行 `cleanup_singleton_groups`；
  - **P0-3**：`src/db/todo.rs` 新增 `compact_grouped_ids` 在 `reorder_todos`/`renumber_date`/`restore_todos` 中强制保持同组连续，`save_todo` 全分支补齐 `cleanup_singleton_groups`。
- [x] **6.2 P1 修复（状态锚点编辑/幽灵行点击、拖拽源悬停串色、`todo_drag` 跨容器清理、CLI 保序与顺延）**
  - **P1-1**：`status_at`、`click_todo_status` 与 `cycle_editing_status` 支持编辑态锚点行与组内新建幽灵行锚点点击/`Tab` 循环整组状态；
  - **P1-2**：`item.rs` 拖拽期间抑制 `todo_hover` 选中底色，`hover.rs` 在 `todo_drag.is_some()` 时屏蔽悬停与 Tooltip；
  - **P1-3**：`select_right_tab`、`TogglePrompt`、`focus_*`、`apply_todo_key`、右键菜单、`handle_mouse_drag`/`up` 与 `wheel.rs` 完整联动清理/拦截 `todo_drag`；
  - **P1-4**：CLI `exec::group` 保留既有组内部顺序并返回完整成员 ID、`exec::update` 保留既有 `Done` 完成日并整组同步、`exec::rollover --id` 连带同组未完成成员。
- [x] **6.3 P2 & P3 修复（视口跟随、CLI Help 与 `src/db/service.rs` 拆分）**
  - **P2-1**：`merge_with_next` 与 `ungroup_at` 置 `follow_selected = true` 并滚入视口；
  - **P2-2**：`src/cli/help.rs` 补齐 `group` / `ungroup` 用法，`exec::ungroup` 合并校验位置参数与 `--id`；
  - **P3-1**：将 `src/db/service.rs`（原 995 行）拆分为 `src/db/service.rs`（609 行）+ `src/db/service/workspace.rs`（446 行）。
