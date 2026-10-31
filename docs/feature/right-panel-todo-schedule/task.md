# 右侧面板 Todo 日程增强与多图表统计 — 执行任务清单

> 状态：全部阶段、补充需求及评审修复均已完成并通过验证。
> 关联文档：`docs/feature/right-panel-todo-schedule/design.md`、`docs/feature/right-panel-todo-schedule/review.md`
> 规则：生产代码禁止 `unwrap()/expect()`；错误使用 `Result` 与 `?`；文案一律英文；严格遵守单文件 ≤1000 行限制。

---

## 阶段划分与任务清单

### 阶段 0：需求盘问与规格确认（已完成）
- [x] 盘问 1：数据模型采用“按所选单日切片展示（单日日程模式）”
- [x] 盘问 2：未完成顺延扫描范围为“所有历史过去日期（`< 今天`）的未完成待办”
- [x] 盘问 3：顺延交互采用“列表顶部 Rollover Banner 紧凑横幅（显示数量 + 一键顺延 + 关闭）”
- [x] 盘问 4：引入 P0 / P1 / P2 / P3 四档优先级，顶部工具栏新增 `[sort]` 排序按钮
- [x] 盘问 5：底部提供 3 种图表（三态完成率、优先级分布、近 7 日趋势），支持切图控件主动切换
- [x] 盘问 6：一周快速点击感知带（Week Strip）放置在工具栏分割线下方、列表上方
- [x] 盘问 7：优先级徽标采用独立前景与背景配色，区分明显且绝不影响字体对比度；支持鼠标点击循环与键盘 `p` 键

---

### 阶段 1：底层数据模型与持久化迁移（Database Layer）
- [x] 1.1 顺延数据库版本 `SCHEMA_VERSION`: 20 → 21（`src/db/mod.rs`）
- [x] 1.2 编写 `SCHEMA_V21_TODO_INDEXES` 与 `migrate_v21_todo_schedule`（`src/db/migration.rs`, `src/db/migration/schema.rs`）：
  - 增加字段：`entry_date TEXT NOT NULL DEFAULT ''`、`priority TEXT NOT NULL DEFAULT 'P1'`、`completed_date TEXT`；
  - 迁移历史存量待办至今天本地日期，已完成项回填 `completed_date`；
  - 废弃 `idx_todo_position` 全局索引，创建 `idx_todo_date_position`、`idx_todo_date_status` 与 `idx_todo_entry_date` 复合索引；
- [x] 1.3 更新 `TodoRecord` 与 `TodoPriority` 枚举定义（`src/db/todo.rs`）
- [x] 1.4 实现数据库 CRUD 与查询方法扩展（`src/db/todo.rs`）：
  - `list_todos_by_date(entry_date)`
  - `list_past_incomplete_todos(today)`
  - `rollover_todos_to_date(ids, target_date)`
  - `reorder_todos(entry_date, ordered_ids)`
  - `delete_todos_with_status_on(entry_date, status)`
  - `query_range_stats(start_date, end_date)`
- [x] 1.5 编写持久化层单元测试（`src/db/todo/tests.rs`），覆盖迁移、按日过滤、跨日移动与重排

---

### 阶段 2：状态层与业务逻辑（App State & Update Layer）
- [x] 2.1 实现纯函数日期工具集（`src/db/todo/date.rs`）：
  - `today_date()`、`shift_date()`、`week_dates()`、`weekday_short()`、`weekday_initial()`、`day_of_month()`、`recent_dates()`
- [x] 2.2 扩展 `TodoPanel` 状态结构体（`src/app/state/todo.rs`）：
  - 增加 `selected_date`、`today_date`、`other_dates`、`active_chart`、`rollover_dismissed`
  - 增加 `TodoChartKind` 枚举及其切换方法（`next()`, `prev()`）
- [x] 2.3 实现待办重排与一键排序纯函数（对齐 `lx-agent` 规则：未完成在前 > P0..P3 > 原 position > id）
- [x] 2.4 实现行为更新逻辑（`src/app/update/todo.rs`）：
  - `select_todo_date(state, date)`：切换选中日期并重载条目
  - `shift_todo_week(state, delta_weeks)`：按周快速前后翻页
  - `sort_todos(state)`：触发当前查看日的一键重排并登记 Effect
  - `cycle_todo_priority(state)` / `click_todo_priority(state, display)`：循环切换条目优先级并登记 Effect
  - `execute_todo_rollover(state)`：将历史未完成条目转移至今天
  - `dismiss_todo_rollover(state)`：关闭顺延横幅
  - `next_todo_chart(state)` / `prev_todo_chart(state)`：切换当前显示的图表
- [x] 2.5 键盘事件映射（`src/app/update/todo.rs`）：
  - 聚焦 Todo 面板时支持 `p` 键换档优先级、`s` 键排序、`c` 键切图表、`[` / `]` 切周
- [x] 2.6 编写状态与行为层单元测试（`src/app/update/todo/tests.rs`, `src/app/state/todo/tests.rs`）

---

### 阶段 3：样式与视觉设计（UI Style Layer）
- [x] 3.1 增加优先级徽标样式函数（`src/ui/style/todo.rs`）：
  - `todo_priority_badge(priority, status)`：定义 P0（暗红底+鲜红字）、P1（暗黄底+琥珀字）、P2（暗蓝底+天蓝字）、P3（暗灰底+亮银灰字）
  - 保证字符前景与背景具备充足对比度，已完成项自动压暗
- [x] 3.2 增加周感知带高亮与当日指示样式（`style::todo_week_day_selected`, `style::todo_week_day_today`）
- [x] 3.3 增加顺延横幅提醒条样式（`style::todo_rollover_banner`, `style::todo_rollover_button`）
- [x] 3.4 增加图表切换控件与多图表配色（`style::todo_priority_bar`, `style::todo_bar_planned`）

---

### 阶段 4：界面组件绘制与点击命中（UI Rendering & Hit-testing）
- [x] 4.1 拆分与重构 Todo 渲染子模块，保证单文件远低于 1000 行限制：
  - `src/ui/right_panel/todo.rs`（主布局、工具栏 `[+]`/`[edit]`/`[del]`/`[sort]`/`[clear done]`、列表行与优先级徽标）
  - `src/ui/right_panel/todo/schedule.rs`（表头分割线日期与周跨度标签；一周快速点击栏；历史顺延提示横幅）
  - `src/ui/right_panel/todo/charts.rs`（分割线切图控件与 3 款图表：三态完成率、优先级分布、7 日趋势）
- [x] 4.2 统一几何计算与鼠标命中系统（`src/event/mouse/press.rs`）
- [x] 4.3 编写 UI 渲染与命中单元测试（`src/ui/right_panel/todo/tests.rs`）

---

### 阶段 5：补充需求与代码评审修复（Supplementary & Review Fixes）
- [x] 5.1 周感知带上方分割线内嵌日期与周跨度：左侧显示 `yyyy/mm/dd`，右侧显示 `DD-DD`（如 `26-01`）
- [x] 5.2 周感知带移除日期号数，仅展示纯英文星期（`Mon`..`Sun` / `M`..`S`）与今日点标 `•`
- [x] 5.3 图表切换按钮中间注明统计范围是当天还是一周（`today · status 1/3`、`today · priority 2/3`、`week · 7d trend 3/3`）
- [x] 5.4 修复图表切换按钮两侧边框分割线颜色不一致 bug，并剥离 `‹`/`›` 箭头继承的底层 `DIM` 修饰符
- [x] 5.5 编写代码评审报告 `docs/feature/right-panel-todo-schedule/review.md`，按 P0–P2 修复全部 10 项问题并补充回归测试

---

### 阶段 6：精确校验与验收（Verification）
- [x] 6.1 代码格式规范检查：`cargo fmt --check`
- [x] 6.2 编译器与 Clippy 静态检查：`cargo clippy --all-targets -- -D warnings`
- [x] 6.3 单元测试与集成测试验证：`cargo test todo`（140 passed）
- [ ] 6.4 提交用户人工 `cargo run` 确认 TUI 观感与是否合并分支
