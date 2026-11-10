# 右侧 Todo 面板条目聚合与拆分（Todo Item Grouping）设计文档

- **状态**：已合并至 `dev`（Merged into `dev` — 详见 `review.md`）
- **任务目录**：`docs/feature/todo-item-grouping/`
- **任务范围**：
  - 数据库与服务层：`src/db/mod.rs`、`src/db/migration.rs`、`src/db/migration/schema.rs`、`src/db/todo.rs`、`src/db/service.rs`、`src/db/service/workspace.rs`（新）、`src/app/effects.rs`
  - 状态层（含按职责拆分子模块）：`src/app/state.rs`、`src/app/state/todo.rs`、`src/app/state/todo/group.rs`（新）、`src/app/state/todo/schedule.rs`（新）
  - 行为层（含按职责拆分子模块）：`src/app/update/todo.rs`、`src/app/update/todo/group.rs`（新）、`src/app/update/todo/drag.rs`（新）、`src/app/update/todo/key.rs`（新）、`src/app/update/right_panel.rs`、`src/app/update/focus.rs`、`src/app/update/editor.rs`、`src/app/update.rs`、`src/app/overlay.rs`、`src/app/update/menu.rs`
  - 渲染层（含按职责拆分子模块）：`src/ui/right_panel/todo.rs`、`src/ui/right_panel/todo/item.rs`（新）、`src/ui/right_panel/todo/schedule.rs`、`src/ui/right_panel/todo/charts.rs`、`src/ui/overlay/menu.rs`、`src/ui/text/todo.rs`、`src/ui/style/todo.rs`
  - 事件与输入层：`src/event/mouse/press.rs`、`src/event/mouse/drag.rs`、`src/event/mouse/hover.rs`、`src/event/mouse/wheel.rs`
  - CLI 层：`src/cli/todo.rs`、`src/cli/todo/exec.rs`、`src/cli/help.rs`
- **数据库变更**：`SCHEMA_VERSION` `26 -> 27`（`todo` 表新增可空列 `group_id TEXT` 与索引 `idx_todo_date_group(entry_date, group_id)`）
- **非目标**：
  - 不引入多层嵌套（禁止“聚合组套聚合组”）；
  - 不额外创建占高 1 行的虚拟父标题（聚合组直接由组内子项组成）；
  - 不改变底部 3 张统计图表与顶部 7 日周带的计数口径（仍按实际 `TodoRecord` 条数统计）。
- **参考实现**：
  - 右侧 Todo 面板渲染与命中：`/Users/yonah/projects/agent/lx-tui/src/ui/right_panel/todo.rs`
  - 右侧 Prompt 树拖拽状态机：`/Users/yonah/projects/agent/lx-tui/src/app/update/prompt/drag.rs`
  - 聊天工具树连接符规范：`/Users/yonah/projects/agent/lx-tui/src/ui/main_content/lx/history/tool/tree.rs`

---

## 1. 背景与目标

### 1.1 现状分析

1. **扁平独立模型（`src/db/todo.rs:121-133`、`src/app/state/todo.rs:105-138`）**：
   - 当前每条 `TodoRecord` 仅有 `(id, entry_date, text, status, priority, completed_date, position)`，在 `TodoPanel.items` 中按当日 `position == index` 扁平排布；
   - 用户在实际工作中常将 2~N 个关联待办一起并行推进，现有设计只能逐条单独切换 `[ ] -> [~] -> [x]`，也无法在视觉上表达“这几项是一起执行的整体”。
2. **单条固定 9 列前缀布局（`src/ui/right_panel/todo.rs:212-236, 721-795`）**：
   - 每条待办左侧固定画 `❯ [ ] P1 正文`（首行缩进 9 列、折行续行缩进 6 列），无组连接符槽位，无法绘制多条聚合的 `┌─ / ├─ / └─` 结构。
3. **仅支持按钮/快捷键排序，无行拖拽（`src/event/mouse/press.rs:285-352`、`src/event/mouse/drag.rs:396-408`）**：
   - 当前鼠标左键点击待办行立即在 `Down` 阶段执行 `click_todo_row`（选中/双击编辑），拖拽仅处理滚动条与输入框文本选区，不支持将不相邻的两条待办拖拽聚合。
4. **单文件行数逼近 1000 行硬红线**：
   - `src/app/state/todo.rs`（967 行）、`src/app/update/todo.rs`（949 行）、`src/ui/right_panel/todo.rs`（947 行）均已逼近项目规范 1000 行上限，本次新增聚合/拆分能力前必须同步按职责拆分子模块，使单文件回归 400~600 行区间。

### 1.2 核心目标

1. **支持待办聚合（Group）与拆分（Split / Ungroup）**：
   - 支持把多条待办聚合成一个共享执行状态的大块；
   - 支持“单条移出聚合组（Split from group）”与“整组全部解散（Ungroup all）”，且当组内成员因拆分或删除只剩 1 条时，自动退化回普通独立待办。
2. **全新聚合组 Item UI 设计**：
   - 聚合组左侧共用唯一一个状态框 `[ ]-` / `[~]-` / `[x]-`（垂直居中偏上锚定在 `offset = (count - 1) / 2` 子项首行），通过 `┌─ / ├─ / └─` 树形括号将各子项的 `P0~P3` 徽标与正文串联起来；
   - 非末尾子项发生 2~3 行折行时，续行左侧延续 `│` 竖线保持树干不断开；末尾子项（`└─`）折行时续行左侧留空。
3. **鼠标拖拽 + 右键菜单 + 键盘快捷键三路闭环**：
   - 鼠标按住任意条目 A 拖到目标条目/聚合组 B 上松手，即可将 A 移至 B 所在组并聚合；
   - 列表态快捷键 `m`（与下一项/组聚合）、`u`（将当前子项移出聚合组）、`U`（解散整个聚合组）与右键菜单一一对应。

### 1.3 非目标

1. 不支持嵌套组（组内成员只能是 `TodoRecord`，不能再包含子组）。
2. 不引入组名称/父标题行（拆分组时 100% 无损还原为原待办条目）。
3. 不提供跨日期聚合（聚合组严格限制在同一 `entry_date` 内；跨日顺延未完成项时，同组未完成项保持原 `group_id` 一起顺延到今日）。

---

## 2. 已确认决策表（Grill Me 7 轮确认）

以下 7 项核心边界均已通过 Grill Me 逐轮与用户确认：

| # | 决策维度 | 已确认结论 |
|---|---|---|
| 1 | **聚合数据模型与 UI 形态** | 采用**单层平级绑定组（共享 `group_id: Option<String>`，无额外父标题行）**；聚合为一个大 item，左侧共用一个 `[ ]`，右侧用 `┌─ / ├─ / └─` 连接各子项的 `P0~P3` 与正文。 |
| 2 | **焦点、状态、编辑与优先级粒度** | **状态整组联动 + 焦点按子项定位**：按 `Space`/`Tab` 或点击左侧 `[ ]` 时整组所有成员同步切换 `Todo -> InProgress -> Done`；`j/k` 与鼠标点击落在具体子项上，`Enter`/双击只编辑该子项正文，`p`/点 `P(n)` 只切换该子项优先级。 |
| 3 | **聚合与拆分的触发方式** | ① **鼠标拖拽**：按住条目 A 拖到条目/组 B 上松手即并入 B 组；② **快捷键与右键菜单**：`m` / `Group with next` 向下合并，`u` / `Split from group` 拆出当前子项，`U` / `Ungroup all` 解散整组。 |
| 4 | **新 Item UI 对齐与折行细节** | ① `[ ]-` 固定画在组内第 `(N - 1) / 2` 条子项首行（2 条落首条 `-┌─`，3 条落中条 `-├─`）；② 普通单条保持紧凑 `❯ [ ] P1 正文`，与聚合组的 `❯` 和 `[ ]` 列严格垂直对齐；③ 组内非末尾子项折行时续行画 `│` 不断线，末尾子项（`└─`）折行时续行留空。 |
| 5 | **合并状态对齐与 `[sort]` 排序不变量** | ① **同组同状态**：将条目并入目标项/组时，被并入条目的 `status`（及 `completed_date`）自动对齐目标项/组；② **同组连续不拆散**：点击 `[sort]`（或按 `s`）时，聚合组作为整体按**组内最高优先级（最小 `TodoPriority`）**参与外部排序，同时组内子项按优先级排整齐。 |
| 6 | **组内删除与向下新建行为** | ① **删除（`d` / 右键 Delete）**：仅删除当前选中的单条子项；若删后该组仅剩 1 条，剩余条目自动清除 `group_id` 退化为普通单条；② **向下插入（`o` / `Shift+Enter`）**：光标在组内子项上时，直接在组内该子项下方新建一条继承该组 `group_id` 与 `status` 的同组子项；`[+]` 与 `a` 始终在列表末尾新建独立普通待办。 |
| 7 | **统计图表与 CLI 口径** | ① 底部 3 张图表与顶部周带继续按实际子项（`TodoRecord`）条数统计；② DB 升至 `v27`（`todo.group_id`），CLI JSON 输出新增 `group_id`，并支持 `lx-tui todo group` / `ungroup` 子命令与状态整组联动。 |

---

## 3. 目标布局与交互设计

### 3.1 列表视觉布局（ASCII 详图）

```text
╭─ Prompt ─ Todo ────────────────────────╮
│[+]                              [sort] │ ← 工具栏行
├ yyyy/mm/dd ──────────────────── DD-DD ─┤ ← 分割线行
│ ‹ Mon  Tue• Wed  Thu  Fri  Sat  Sun ›  │ ← 周感知带
│   [ ] P0 先修线上紧急崩溃              │ ← 普通单条（Ungrouped，紧凑 9 列前缀）
│   [ ]-┌─ P1 写登录接口                 │ ← 2 条聚合组：anchor = (2-1)/2 = 0，首条画 `[ ]-┌─ `
│       └─ P1 写登录单元测试             │   末条画 `    └─ `
│       ┌─ P1 改造订单表结构，补充索引   │ ← 3 条聚合组（含折行）：首条画 `    ┌─ `
│       │  并兼容旧版历史数据迁移        │   非末条折行续行画 `    │  `，正文对齐本条 P1 列
│ ❯ [~]-├─ P1 编写订单迁移脚本           │   anchor = (3-1)/2 = 1，中条画 `[~]-├─ `，当前选中带 `❯`
│       └─ P2 更新接口文档并通知前端联   │   末条画 `    └─ `
│          调确认字段                    │   末条折行续行画 `       `（空白），正文对齐本条 P2 列
├─────────────────── ‹ today · status 1/3 ›┤
│ total 6 · doing 3 · done 0 0%          │
╰─ lx-tui · b:dev ─────────────────── [-][x]╯
```

### 3.2 列级几何分解（绘制与命中共用唯一几何源）

设列表内容区矩形为 `list = effective_list_rect(panel, state)`，起始列为 `list.x`：

#### （A）普通单条（`GroupRole::Single`）

| 列偏移（相对 `list.x`） | 宽度 | 首行（`line_offset == 0`） | 折行续行（`line_offset >= 1`） | 说明 |
|---|---|---|---|---|
| `+0` | 1 (`MARKER_WIDTH`) | `❯`（选中）或 `' '` | `' '` | 选中标记列 |
| `+1` | 1 | `' '` | `' '` | 标记与状态框间距 |
| `+2..+5` | 3 (`STATUS_WIDTH`) | `[ ]` / `[~]` / `[x]` | `'   '` | 状态框（点击可循环状态） |
| `+5` | 1 | `' '` | `' '` | 状态框与优先级间距 |
| `+6..+8` | 2 (`PRIORITY_WIDTH`) | `P0`..`P3` | 正文续行起点（`cont_x = list.x + 6`） | 首行画徽标，续行从 `+6` 起写正文 |
| `+8` | 1 | `' '` | 正文续行内容 | 优先级与正文间距 |
| `+9..right` | `text_width` | 首行正文起点（`text_x = list.x + 9`） | 正文续行内容 | 最多 3 行，第 3 行超长用 `…` 截断 |

#### （B）聚合组子项（`GroupRole::Member { offset, count, is_status_anchor }`，`count >= 2`）

| 列偏移（相对 `list.x`） | 宽度 | 首行（`line_offset == 0`） | 折行续行（`line_offset >= 1`） | 说明 |
|---|---|---|---|---|
| `+0` | 1 (`MARKER_WIDTH`) | `❯`（当前选中子项）或 `' '` | `' '` | 仅选中子项首行显示 `❯` |
| `+1` | 1 | `' '` | `' '` | 间距 |
| `+2..+5` | 3 (`STATUS_WIDTH`) | `is_status_anchor` 为真时画 `[ ]`/`[~]`/`[x]`，否则画 `'   '` | `'   '` | 整组唯一状态框，锚定在 `offset == (count - 1) / 2` |
| `+5..+9` | 4 (`GROUP_BRANCH_WIDTH`) | 见下方连接符表 | 非末项画 `" │  "`，末项画 `"    "` | 树形括号与跨行竖线槽 |
| `+9..+11` | 2 (`PRIORITY_WIDTH`) | `P0`..`P3` | 正文续行起点（`cont_x = list.x + 9`） | 子项独立优先级徽标；续行与本条 `P(n)` 左对齐 |
| `+11` | 1 | `' '` | 正文续行内容 | 间距 |
| `+12..right` | `grouped_text_width` | 首行正文起点（`text_x = list.x + 12`） | 正文续行内容 | 子项独立最多 3 行折行 |

#### （C）聚合组连接符 4 列精确字形表（相对 `list.x + 5`，宽 4 列）

| 子项位置 | 是否为状态锚点（`offset == (count - 1) / 2`） | 首行 4 列字形 | 折行续行（`line_offset >= 1`）4 列字形 |
|---|---|---|---|
| 首项（`offset == 0`） | 是（仅 `count == 2` 时发生） | `"-┌─ "` | `" │  "` |
| 首项（`offset == 0`） | 否（`count >= 3`） | `" ┌─ "` | `" │  "` |
| 中间项（`0 < offset < count - 1`） | 是 | `"-├─ "` | `" │  "` |
| 中间项（`0 < offset < count - 1`） | 否 | `" ├─ "` | `" │  "` |
| 末项（`offset == count - 1`） | 否（`count >= 2` 恒成立） | `" └─ "` | `"    "` |

- **连接符样式**：连接符（`-┌─ `、` ┌─ `、`-├─ `、` ├─ `、` └─ `、` │  `）统一使用与整组状态一致的 `status_style(row.status)`（未开始 `todo_status_open()` 正文色、进行中 `todo_status_active()` 强调色、已完成 `todo_status_done()` 压暗色，但连接符不叠加 `CROSSED_OUT` 删除线，避免横竖线与删除线糊在一起）。

### 3.3 键盘交互矩阵（列表态）

| 按键 | 动作 | 详细行为说明 |
|---|---|---|
| `j` / `↓`、`k` / `↑` | 按子项上下移动焦点 | 逐条遍历 `TodoPanel.items`（含聚合组内各子项） |
| `Space` / `Tab` | 循环切换状态 | 若当前选中项属于聚合组，**整组所有成员同步循环** `todo -> in_progress -> done -> todo`；若为普通单条，仅切换该条 |
| `m` | 向下合并（Merge with next） | 将当前选中项（或所在组）与紧邻的下一项（或下一组）聚合成同一组，状态以当前选中项/组为准 |
| `u` | 拆出当前子项（Split item） | 将当前选中的组内子项移出聚合组（挪到该组紧邻下方并清除 `group_id`）；若原组只剩 1 条，原组自动解除 `group_id` |
| `U`（`Shift+u`） | 解散当前整组（Ungroup all） | 将当前选中项所在聚合组的全部成员一次性清除 `group_id`，原地恢复为多条独立普通待办 |
| `Enter` / `e` | 编辑当前子项正文 | 仅对当前选中子项开启行内多行编辑（聚合组内编辑时保持 12/9 列缩进与树形连接符） |
| `Shift+Enter` / `o` | 在当前项后新建 | 若当前项在聚合组内，新幽灵行**直接作为同组子项**插在当前子项下方（继承 `group_id` 与 `status`）；否则新建普通独立项 |
| `a` | 列表末尾追加新建 | 始终在列表末尾新建独立的普通待办（`group_id = None`） |
| `p` / `P` | 循环当前子项优先级 | 仅切换当前选中子项的 `P0 -> P1 -> P2 -> P3 -> P0` |
| `d` | 删除当前子项 | 仅删除当前选中子项；若所属聚合组删后只剩 1 条，剩余那条自动退化为 `group_id = None` |
| `s` / `S` | 按优先级排序 | 未完成优先；聚合组作为不可分割整体按**组内最高优先级**参与外部排序，组内子项同步按优先级升序重排 |

### 3.4 鼠标交互与右键菜单矩阵

| 鼠标操作 | 命中区域 | 行为说明 |
|---|---|---|
| 左键单击 | 状态框列（`list.x + 2..list.x + 5`，普通项首行或聚合组锚点行） | 直接选中并循环切换该条/该聚合组的状态（`click_todo_status`） |
| 左键单击 | 优先级徽标列（普通项 `list.x + 6..8`，组内项 `list.x + 9..11`，首行） | 选中该子项并循环切换该子项优先级（`click_todo_priority`） |
| 左键按下→未跨行松开 | 条目任意视觉行（非状态框/优先级徽标） | 单击选中该子项 + 聚焦；400ms 内同位置二次点击进入该子项行内编辑 |
| 左键按住→跨行拖动→松开 | 从条目 A 拖到条目 B（含 B 所在聚合组） | 拖动中 A 置灰、目标 B（或 B 所在整组）高亮预览；松手后将 A 并入 B 所在聚合组（若 B 原为普通单条则与 A 组成新聚合组，A 排在 B 组末尾，状态对齐 B） |
| 右键点击 | 普通单条 | 弹出菜单：`Edit` / `Group with next`（当下方有条目时显示）/ `Cycle priority` / `Delete`（二次确认） |
| 右键点击 | 聚合组内子项 | 弹出菜单：`Edit` / `Group with next`（当该组下方仍有其他条目/组时显示）/ `Split from group` / `Ungroup all` / `Cycle priority` / `Delete`（二次确认） |

---

## 4. 架构设计与关键代码

### 4.1 数据库层（`src/db/`）

#### 4.1.1 Schema 升级（`SCHEMA_VERSION = 27`）

- `src/db/mod.rs:33`：
  ```rust
  pub const SCHEMA_VERSION: i64 = 27;
  ```
- `src/db/migration/schema.rs`：
  - 更新 `SCHEMA_V15_TODO`（供全新库直接建出完整列）与新增 `SCHEMA_V27_TODO_GROUP`：
    ```rust
    /// v27 增量 Schema：`todo` 表新增可空聚合组标识 `group_id` 与按日组索引。
    pub(super) const SCHEMA_V27_TODO_GROUP: &str = r#"
    CREATE INDEX IF NOT EXISTS idx_todo_date_group ON todo(entry_date, group_id);
    "#;
    ```
- `src/db/migration.rs`：
  ```rust
  if version < 27 {
      self.migrate_v27_todo_group()?;
  }

  /// v27：`todo` 表新增可空列 `group_id`（同组待办共享稳定组 ID，独立待办为 NULL）及按日组索引。
  fn migrate_v27_todo_group(&mut self) -> Result<()> {
      let transaction = self.conn.unchecked_transaction()?;
      transaction.execute_batch(SCHEMA_V26)?;
      if !column_exists(&transaction, "todo", "group_id")? {
          transaction.execute_batch("ALTER TABLE todo ADD COLUMN group_id TEXT;")?;
      }
      transaction.execute_batch(SCHEMA_V27_TODO_GROUP)?;
      ensure_no_foreign_key_violation(&transaction)?;
      transaction.pragma_update(None, "user_version", 27)?;
      transaction.commit()?;
      Ok(())
  }
  ```

#### 4.1.2 `TodoRecord` 与整日同步写库 API（`src/db/todo.rs`）

- `TodoRecord` 新增 `pub group_id: Option<String>`：
  ```rust
  #[derive(Debug, Clone, PartialEq, Eq)]
  pub struct TodoRecord {
      pub id: String,
      pub entry_date: String,
      pub text: String,
      pub status: TodoStatus,
      pub priority: TodoPriority,
      pub completed_date: Option<String>,
      pub position: i64,
      /// 所属聚合组稳定 ID（`tgrp-<nanos>-<seq>`）；独立条目为 `None`。
      pub group_id: Option<String>,
  }

  impl TodoRecord {
      pub fn with_group_id(mut self, group_id: Option<String>) -> Self {
          self.group_id = group_id;
          self
      }
  }
  ```
- `map_todo_row` 与所有 `SELECT`（`list_todos`、`list_todos_by_date`、`list_past_incomplete_todos`、`todo_by_id`）、`save_todo`、`restore_todos` 均读写第 8 列 `group_id`。
- 新增 `Database::sync_date_todos(&self, entry_date: &str, records: &[TodoRecord]) -> Result<()>`：
  - 当执行聚合、拆分、整组状态循环、排序或删除触发孤儿组退化时，同一天内多条记录的 `position`、`group_id`、`status`、`completed_date` 会同时变化；
  - `sync_date_todos` 在单个事务内：
    1. 先将该日既有行的 `position` 整体加上 `RENUMBER_OFFSET` 避开 `idx_todo_date_position` 唯一索引冲突；
    2. 按传入的 `records` 顺序逐条更新 `status`、`priority`、`completed_date`、`group_id`、`position = 0..n`、`updated_at_ms`；
    3. 提交事务。
  ```rust
  /// 单事务同步指定日期下全部条目的顺序、聚合组归属、状态与优先级。
  pub fn sync_date_todos(&self, entry_date: &str, records: &[TodoRecord]) -> Result<()> {
      let transaction = self.conn.unchecked_transaction()?;
      transaction.execute(
          "UPDATE todo SET position = position + ?1 WHERE entry_date = ?2",
          params![RENUMBER_OFFSET, entry_date],
      )?;
      let now = now_ms();
      for (index, record) in records.iter().enumerate() {
          let completed_date = if record.status == TodoStatus::Done {
              Some(
                  record
                      .completed_date
                      .clone()
                      .unwrap_or_else(|| entry_date.to_string()),
              )
          } else {
              None
          };
          transaction.execute(
              "UPDATE todo
               SET status = ?1, priority = ?2, completed_date = ?3,
                   group_id = ?4, position = ?5, updated_at_ms = ?6
               WHERE id = ?7 AND entry_date = ?8",
              params![
                  record.status.as_str(),
                  record.priority.as_str(),
                  completed_date,
                  record.group_id.as_deref(),
                  index as i64,
                  now,
                  record.id.as_str(),
                  entry_date,
              ],
          )?;
      }
      renumber_date(&transaction, entry_date)?;
      transaction.commit()?;
      Ok(())
  }
  ```
- `Effect`（`src/app/effects.rs`）：新增 `SyncDateTodos { entry_date: String, records: Vec<TodoRecord> }`（或在 `src/db/service.rs` 中以 3 行分派调用 `db.sync_date_todos(&entry_date, &records)`）。

---

### 4.2 状态层设计与子模块拆分（`src/app/state/todo*`）

为遵守单文件 ≤ 1000 行（拆分目标 ~500 行）规范，将 `src/app/state/todo.rs`（原 967 行）拆分为：
1. `src/app/state/todo.rs`（~480 行）：`TodoPanel` 核心结构、选区/视口滚动、编辑现场 `TodoEdit`、基础 CRUD；
2. `src/app/state/todo/group.rs`（~280 行）：聚合组角色投影 `TodoGroupRole`、不变量归一化、合并/拆分/拖拽并组、整组状态循环、保组优先级排序；
3. `src/app/state/todo/schedule.rs`（~220 行）：跨日切换、历史未完成顺延、7 日周带与图表统计投影。

#### 4.2.1 聚合组角色投影与编辑现场扩展

```rust
/// 条目在列表中的聚合角色（由 `TodoPanel::group_role(index)` O(N) 纯函数派生，不冗余存储）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TodoGroupRole {
    /// 独立普通待办（`group_id == None`）。
    Single,
    /// 聚合组内成员（`count >= 2`）。
    Member {
        group_id: String,
        /// 在组内的 0 基下标（`0..count`）。
        offset: usize,
        /// 组内总成员数（含正在组内新建的幽灵槽位时由渲染层扩展）。
        count: usize,
    },
}

impl TodoGroupRole {
    /// 是否承载整组唯一的状态框 `[ ]`（固定在 `(count - 1) / 2`）。
    pub fn is_status_anchor(&self) -> bool {
        match self {
            Self::Single => true,
            Self::Member { offset, count, .. } => *offset == (*count - 1) / 2,
        }
    }
}
```

`TodoEdit` 扩展两个字段以支持组内 `o` / `Shift+Enter` 新建幽灵行直接呈现为组内子项：
```rust
#[derive(Debug)]
pub struct TodoEdit {
    pub target: Option<String>,
    pub input: TextInput,
    pub at: usize,
    pub scroll_row: usize,
    /// 新建幽灵行归属的聚合组 ID（`o` / `Shift+Enter` 在组内触发时继承该组 ID；`a` / `[+]` 为 None）。
    pub group_id: Option<String>,
    /// 新建幽灵行的初始状态（组内新建时继承该组当前状态；普通新建为 `TodoStatus::Todo`）。
    pub initial_status: TodoStatus,
}
```

#### 4.2.2 聚合组核心不变量归一化 `normalize_groups_and_positions`

任何会改变列表结构的操作（加载 `replace`、插入 `insert`、删除 `remove`、合并、拆分、排序、顺延）末尾统一调用 `normalize_slice_groups_and_positions(items, date, today)`，强制保证以下 **4 条不变量**：

1. **同组必连续**：若相同 `group_id` 的条目被非同组条目隔开，自动将该组后续成员归拢到首次出现的位置紧邻下方，保持组内相对顺序；
2. **同组必同状态**：同一 `group_id` 的所有成员强制对齐到该组首条成员的 `status` 与 `completed_date`；
3. **禁止单条孤儿组**：统计每个 `group_id` 的成员数，若 `count < 2`，自动将其 `group_id` 置为 `None`（退化回普通单条）；
4. **位置密集**：`item.position = index as i64`。

```rust
pub(super) fn normalize_slice_groups_and_positions(
    items: &mut Vec<TodoRecord>,
    date: &str,
    today: &str,
) {
    // 1. 同组归拢连续：按首次出现顺序把同 group_id 成员聚拢在一起
    let mut compacted: Vec<TodoRecord> = Vec::with_capacity(items.len());
    let mut visited = vec![false; items.len()];
    for i in 0..items.len() {
        if visited[i] {
            continue;
        }
        visited[i] = true;
        let gid = items[i].group_id.clone();
        compacted.push(items[i].clone());
        if let Some(ref group_id) = gid {
            for j in (i + 1)..items.len() {
                if !visited[j] && items[j].group_id.as_deref() == Some(group_id.as_str()) {
                    visited[j] = true;
                    compacted.push(items[j].clone());
                }
            }
        }
    }
    *items = compacted;

    // 2. 统计各 group_id 成员数，消除 count < 2 的孤儿组，并统一同组状态
    let mut counts: BTreeMap<String, (usize, TodoStatus, Option<String>)> = BTreeMap::new();
    for item in items.iter() {
        if let Some(ref gid) = item.group_id {
            counts
                .entry(gid.clone())
                .and_modify(|(c, _, _)| *c += 1)
                .or_insert_with(|| {
                    let completed = (item.status == TodoStatus::Done).then(|| {
                        item.completed_date
                            .clone()
                            .unwrap_or_else(|| today.to_string())
                    });
                    (1, item.status, completed)
                });
        }
    }

    for (index, item) in items.iter_mut().enumerate() {
        item.position = index as i64;
        if item.entry_date.is_empty() {
            item.entry_date = date.to_string();
        }
        if let Some(ref gid) = item.group_id.clone() {
            if let Some((count, group_status, group_completed)) = counts.get(gid) {
                if *count < 2 {
                    item.group_id = None;
                } else {
                    item.status = *group_status;
                    item.completed_date = group_completed.clone();
                }
            }
        }
    }
}
```

#### 4.2.3 聚合、拆分、状态联动与保组排序算法（`src/app/state/todo/group.rs`）

1. **向下聚合 `merge_with_next(&mut self, index: usize) -> bool`**：
   - 找到 `index` 所在组（或单条）的末尾下标 `group_end`；若 `group_end + 1 >= self.items.len()`，返回 `false`；
   - 确定 `target_group_id`（若当前项已有 `group_id` 则复用，否则调 `new_external_id("tgrp")` 生成新 ID）与基准状态 `base_status = self.items[index].status`；
   - 若 `group_end + 1` 属于另一个聚合组 `other_gid`，将该 `other_gid` 的全部成员并入 `target_group_id` 并同步 `base_status`；若为独立单条，则将该单条并入 `target_group_id` 并同步 `base_status`。
2. **拖拽合并 `merge_item_into(&mut self, from_index: usize, target_index: usize) -> bool`**：
   - 当 `from_index == target_index` 或两者已属同一 `group_id` 时返回 `false`（Noop）；
   - 目标基准状态 `target_status = self.items[target_index].status`，目标组 ID `target_gid = self.items[target_index].group_id.clone().unwrap_or_else(|| new_external_id("tgrp"))`；
   - 将 `self.items[target_index]` 所在组（若原本为单条）赋予 `target_gid`；
   - 取出 `from_index` 条目，设置 `group_id = Some(target_gid.clone())`、`status = target_status`，插入到目标组末尾，调用 `normalize_positions()`（若 `from_index` 原属旧组且旧组只剩 1 条，归一化会自动将旧组那 1 条退化为普通单条），并将选中焦点跟随到移动后的 `from` 条目。
3. **单条拆出 `split_from_group(&mut self, index: usize) -> bool`**：
   - 若 `self.items.get(index)?.group_id.is_none()`，返回 `false`；
   - 找到该组的末尾下标 `group_end`，将 `index` 处的 `group_id` 置为 `None`，并移动到 `group_end` 处（脱离到原聚合组紧邻下方），归一化（若原组原本只有 2 条，剩下一条自动清除 `group_id`），选中保持在该脱离条目上。
4. **整组解散 `ungroup_at(&mut self, index: usize) -> bool`**：
   - 取 `gid = self.items.get(index)?.group_id.clone()?`；
   - 将所有 `item.group_id.as_deref() == Some(gid.as_str())` 的条目置 `item.group_id = None`，原地保留各自位置、状态与优先级。
5. **整组状态循环 `cycle_status(&mut self, index: usize) -> Vec<TodoRecord>`**：
   - 取 `next_status = self.items.get(index)?.status.cycle()`；
   - 若该项有 `group_id`，同组所有成员同步更新为 `next_status` 与对应 `completed_date`，返回全部受影响记录；否则仅更新该单条并返回单元素 `Vec`。
6. **保组优先级排序 `sort_by_priority(&mut self) -> Option<Vec<String>>`**：
   - 将 `self.items` 切分为若干连续块 `Block`（独立单条为 1 条块，同 `group_id` 为多条块）；
   - 每个聚合组内部先按 `(priority ASC, position ASC, id ASC)` 排序；
   - 各块之间按 `(is_done ASC, min_priority ASC, first_position ASC, first_id ASC)` 排序；
   - 展平回 `self.items` 并重新归一 `position`。

---

### 4.3 行为层与拖拽状态机（`src/app/update/todo*`）

将 `src/app/update/todo.rs`（原 949 行）拆分为：
1. `src/app/update/todo.rs`（~360 行）：焦点管理、鼠标点击行/状态/优先级、编辑生命周期、右键菜单入口、视口同步；
2. `src/app/update/todo/group.rs`（~180 行）：聚合/拆分动作函数（`group_todo_with_next`、`split_todo_from_group`、`ungroup_todo_all`）与 DB 同步；
3. `src/app/update/todo/drag.rs`（~180 行）：`TodoDrag` 鼠标按下/跨行拖动/释放合并状态机；
4. `src/app/update/todo/key.rs`（~260 行）：列表态与编辑态键盘分派。

#### 4.3.1 `TodoDrag` 状态机（对齐 `PromptDrag`，不破坏双击编辑）

在 `AppState`（`src/app/state.rs`）新增：
```rust
/// Todo 列表行拖拽合并现场。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TodoDrag {
    /// 按下时的源条目 ID。
    pub source_id: String,
    /// 按下时的显示下标。
    pub from_display: usize,
    /// 按下时刻（用于未跨行松开时的单击/双击判定）。
    pub pressed_at: Instant,
    /// 是否已跨行激活拖拽。
    pub dragging: bool,
    /// 当前悬停的可合并目标列表下标（同条目或同组不标记）。
    pub target_index: Option<usize>,
}
```

- **Mouse Down（`press.rs`）**：
  - 命中状态框 `status_at` → 立即执行 `click_todo_status`；
  - 命中优先级徽标 `priority_at` → 立即执行 `click_todo_priority`；
  - 命中条目行 `row_at` → 调用 `begin_todo_row_press(state, display, Instant::now())`：
    - 若点中正在编辑的行，仅聚焦；
    - 否则先按 ID 锚定被点条目，提交在途编辑，选中并聚焦该条目，登记 `state.todo_drag = Some(TodoDrag { dragging: false, .. })`；
    - 若与上一次点击满足双击窗口（`<= 400ms` 且同一 `display`），直接开启行内编辑并清空 `todo_drag`。
- **Mouse Drag（`drag.rs`）**：
  - 当 `state.todo_drag.is_some()` 时，根据 `ui::right_panel::todo::row_at(view.prompt, state, mouse.column, mouse.row)` 解析当前显示行；
  - 一旦离开 `from_display`，置 `dragging = true`；若落点指向另一条可合并的条目（非自身且非同组），置 `target_index = Some(idx)`，渲染层将该目标条目（及其所在聚合组全员）铺上高亮底色，源条目置 `style::muted()`。
- **Mouse Up（`drag.rs`）**：
  - `finish_todo_drag(state, hit_display)`：取出 `state.todo_drag.take()`；
  - 若 `drag.dragging` 为真且 `target_index` 有效，调用 `state.todo.merge_item_into(source_idx, target_idx)` 并推送 `Effect::SyncDateTodos`！

---

### 4.4 渲染层与子模块拆分（`src/ui/right_panel/todo*`）

将 `src/ui/right_panel/todo.rs`（原 947 行）拆分为：
1. `src/ui/right_panel/todo.rs`（~460 行）：面板纵向分块几何、工具栏按钮、可见列表布局 `layout_visible_items`、命中测试（`row_at` / `status_at` / `priority_at` / `edit_input_char_at`）、滚动条；
2. `src/ui/right_panel/todo/item.rs`（~360 行）：单条与聚合组 `RowView` 组装、树形连接符绘制、状态框与优先级徽标绘制、多行折行正文与行内编辑渲染；
3. `src/ui/right_panel/todo/schedule.rs`（325 行，保持）：周感知带与历史顺延横幅；
4. `src/ui/right_panel/todo/charts.rs`（484 行，保持）：3 张统计图表。

#### 4.4.1 关键渲染与几何代码（`src/ui/right_panel/todo/item.rs`）

```rust
/// 聚合组分支连接符槽宽（列）：`"-┌─ "` / `" ┌─ "` / `"-├─ "` / `" ├─ "` / `" └─ "` / `" │  "` 均为 4 列。
pub(super) const GROUP_BRANCH_WIDTH: u16 = 4;

/// 条目优先级徽标起始 X：普通单条为 `list.x + 6`，聚合组子项为 `list.x + 9`。
pub(super) fn row_priority_x_for(list: Rect, grouped: bool) -> u16 {
    let base = list
        .x
        .saturating_add(MARKER_WIDTH)
        .saturating_add(1)
        .saturating_add(STATUS_WIDTH);
    if grouped {
        base.saturating_add(GROUP_BRANCH_WIDTH)
    } else {
        base.saturating_add(1)
    }
}

/// 条目首行正文起始 X：普通单条为 `list.x + 9`，聚合组子项为 `list.x + 12`。
pub(super) fn row_text_x_for(list: Rect, grouped: bool) -> u16 {
    row_priority_x_for(list, grouped)
        .saturating_add(PRIORITY_WIDTH)
        .saturating_add(1)
}

/// 聚合组子项首行与折行续行的 4 列连接符。
pub(super) fn group_connector_glyph(
    offset: usize,
    count: usize,
    is_anchor: bool,
    line_offset: usize,
) -> &'static str {
    if line_offset > 0 {
        return if offset + 1 < count { " │  " } else { "    " };
    }
    if offset == 0 {
        if is_anchor { "-┌─ " } else { " ┌─ " }
    } else if offset + 1 < count {
        if is_anchor { "-├─ " } else { " ├─ " }
    } else {
        " └─ "
    }
}
```

---

### 4.5 右键菜单、文案与 CLI 扩展

1. **文案常量（`src/ui/text/todo.rs`，全英文）**：
   ```rust
   pub const MENU_TODO_GROUP_NEXT: &str = "Group with next";
   pub const MENU_TODO_SPLIT_GROUP: &str = "Split from group";
   pub const MENU_TODO_UNGROUP_ALL: &str = "Ungroup all";
   ```
2. **菜单命令（`src/app/overlay.rs` & `src/app/update/menu.rs`）**：
   - `MenuCommand` 新增 `GroupTodoWithNext`、`SplitTodoFromGroup`、`UngroupTodoAll`；
   - `open_todo_item_menu` 按目标条目所处位置动态组装可见菜单项：
     - 若目标项/组下方仍有其他条目：包含 `MenuCommand::GroupTodoWithNext`；
     - 若目标项处于聚合组内（`group_id.is_some()`）：包含 `MenuCommand::SplitTodoFromGroup` 与 `MenuCommand::UngroupTodoAll`。
3. **CLI 扩展（`src/cli/todo.rs` & `src/cli/todo/exec.rs`）**：
   - `todo_json` 输出增加 `"group_id": todo.group_id`；
   - `lx-tui todo done|start|reopen|update --status`：若目标条目属于某个 `group_id`，同日同组所有条目同步更新状态；
   - `lx-tui todo delete`：删后对该日执行孤儿组检查（同组只剩 1 条时自动清空其 `group_id`）；
   - 新增子命令 `lx-tui todo group --id <ID> --id <ID>...`（将同日 2 条及以上待办聚合成一组，状态对齐第一条）与 `lx-tui todo ungroup <ID> [--all]`（默认拆出单条，`--all` 解散整组）。

---

### 4.6 核心不变量清单

1. **单层聚合不变量**：只有 `group_id: Option<String>`，不存在嵌套组或空组。
2. **最小规模不变量**：任何时刻，任一日期下相同 `group_id` 的条目数必 `>= 2`；一旦减至 `1`，立即自动清除 `group_id` 退化为 `Single`。
3. **同组连续不变量**：同一日期下相同 `group_id` 的条目在 `items` 中下标必构成连续区间 `start..start+count`。
4. **同组同状态不变量**：同一日期下相同 `group_id` 的条目 `status` 与 `completed_date` 恒一致。
5. **状态框唯一锚点不变量**：大小为 `count` 的聚合组仅在 `offset == (count - 1) / 2` 的子项首行绘制并响应 `[ ]` 状态框。
6. **折行树干连续不变量**：组内 `offset < count - 1` 的子项折行时，续行 `+5..+9` 列恒绘 `" │  "`；末项 `offset == count - 1` 折行时，续行 `+5..+9` 列恒绘 `"    "`。
7. **渲染与命中同源不变量**：`layout_visible_items` 是行高、Y 坐标、列偏移（单条 9/6 列、组内 12/9 列）的唯一计算源。

---

## 5. 边界条件与降级矩阵

| 场景 | 行为与降级处理 |
|---|---|
| 对列表最后一条（或最后一个聚合组）按 `m` | 无下一项可合并，无操作返回 `false`（不置脏、不写库） |
| 对普通单条按 `u` 或 `U` | 非聚合组，无操作返回 `false` |
| 2 条聚合组中删除 1 条（`d` 或空文本提交） | 剩余 1 条自动清除 `group_id`，UI 立即从 `-┌─` 恢复为普通单条 `[ ] P1 ...` |
| 2 条聚合组中对任一条按 `u`（拆出单条） | 拆出后两边各剩 1 条，双双自动恢复为普通单条，等价于解散该 2 人组 |
| 在聚合组内按 `o` / `Shift+Enter` 后按 `Esc` 取消 | 幽灵行丢弃，原聚合组连接符与状态锚点立即恢复原状 |
| 在聚合组内按 `o` 新建时面板宽度较窄 | 幽灵行同样按聚合组子项缩进（首行 12 列、续行 9 列）折行，树形括号实时包含幽灵行 |
| 拖拽未跨行即松开鼠标 | 不触发合并，按原单击/双击（400ms 窗口）逻辑选中或进入行内编辑 |
| 历史未完成顺延到今天（Rollover） | 过去某天的未完成聚合组全员顺延到今日末尾，保留原 `group_id` 并在今日继续维持同一聚合块 |

| 2 条聚合组中删除 1 条后按撤销（Undo） | `remove_todo_at` 连带保存因孤儿退化被抹掉 `group_id` 的幸存伙伴快照，`restore` 原地恢复伙伴 `group_id` 并完整还原 2 人聚合组 |
| 聚合组状态锚点处于行内编辑或落在组内新建幽灵行上 | `status_at` 与 `click_todo_status` / `cycle_editing_status` 仍准确响应整组唯一 `[ ]` 状态框点击与 `Tab` 键循环 |
| 拖拽中途切 Tab / 折叠右栏 / 按键 / 右键菜单 / 滚轮 | 立即中止并清理 `state.todo_drag` 与 `todo_hover`，禁止在隐藏或浮层状态下误合并待办 |

---

## 6. 测试计划

1. **数据库与迁移测试（`src/db/todo/tests.rs`、`src/db/migration/tests.rs`）**：
   - `migrate_v26_to_v27_adds_group_id_column_and_preserves_rows`
   - `save_and_sync_date_todos_persists_group_id_and_positions`
   - `restore_todos_cleans_up_singleton_group_when_partner_missing`
   - `reorder_and_save_todo_enforce_contiguous_and_non_singleton_group_invariants`
2. **状态层与不变量测试（`src/app/state/todo/tests.rs`）**：
   - `merge_with_next_groups_two_singles_and_aligns_status`
   - `merge_item_into_moves_non_adjacent_item_into_target_group`
   - `split_from_group_detaches_item_and_degenerates_singleton_group`
   - `ungroup_at_dissolves_entire_group_in_place`
   - `cycle_status_updates_all_members_of_group`
   - `remove_in_two_item_group_degenerates_survivor_to_single`
   - `sort_by_priority_keeps_group_contiguous_and_orders_by_highest_member_priority`
   - `merge_with_next_and_ungroup_re_enable_follow_selected`
3. **行为与事件测试（`src/app/update/todo/tests/grouping.rs`、`src/event/tests/todo.rs`）**：
   - `key_m_u_upper_u_merge_split_and_ungroup_todos`
   - `key_o_inside_group_inserts_new_member_inheriting_group_and_status`
   - `mouse_drag_todo_onto_another_row_merges_into_group_without_breaking_double_click`
   - `right_click_grouped_todo_exposes_split_and_ungroup_commands`
   - `resolve_display_index_rejects_ghost_slot_and_press_during_edit_does_not_arm_stale_drag`
   - `undo_delete_in_two_item_group_restores_group_in_memory_and_db`
   - `status_click_and_tab_work_on_editing_anchor_and_in_group_ghost_row`
   - `tab_switch_collapse_wheel_and_right_click_abort_todo_drag_cleanly`
4. **UI 渲染与几何测试（`src/ui/right_panel/todo/tests/grouping.rs`）**：
   - `two_and_three_item_groups_render_shared_status_and_tree_connectors`
   - `grouped_wrapped_item_draws_vertical_trunk_on_non_last_and_blank_on_last`
   - `mixed_single_and_grouped_items_align_status_and_hit_priority_badges_accurately`
   - `dragging_source_row_suppresses_hover_selection_background`
5. **CLI 测试（`src/cli/tests/prompt_and_todo_tests.rs`）**：
   - `cli_todo_group_ungroup_and_status_sync_across_group`
   - `cli_todo_group_preserves_existing_member_order_and_rollover_keeps_group`

---

## 7. 文件行数水位与拆分结果

| 文件路径 | 改造前行数 | 改造后行数 | 说明 |
|---|---|---|---|
| `src/app/state/todo.rs` | 967 | 652 | 拆出 `group.rs` 与 `schedule.rs` |
| `src/app/state/todo/group.rs`（新） | 0 | 370 | 聚合角色、不变量归一、合并/拆分/保组排序 |
| `src/app/state/todo/schedule.rs`（新） | 0 | 333 | 跨日、顺延、周带与统计投影 |
| `src/app/update/todo.rs` | 949 | 666 | 拆出 `group.rs`、`drag.rs`、`key.rs` |
| `src/app/update/todo/group.rs`（新） | 0 | 217 | 合并/拆分/整组解散与状态框点击 |
| `src/app/update/todo/drag.rs`（新） | 0 | 102 | 鼠标拖拽合并状态机 |
| `src/app/update/todo/key.rs`（新） | 0 | 361 | 列表态与编辑态按键分派 |
| `src/ui/right_panel/todo.rs` | 947 | 668 | 拆出 `item.rs` |
| `src/ui/right_panel/todo/item.rs`（新） | 0 | 498 | 单条与聚合组渲染、连接符、折行与行内编辑 |
| `src/db/service.rs` | 989 | 609 | 拆出 `workspace.rs` |
| `src/db/service/workspace.rs`（新） | 0 | 446 | 工作区、提示词树、引用文件夹与 Todo 写库分派 |
| `src/db/todo.rs` | 614 | 753 | `group_id` 持久化、`sync_date_todos` 与同组连续归拢 |
