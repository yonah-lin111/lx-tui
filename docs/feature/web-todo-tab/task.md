# Web 控制台 Todo 看板 + TUI 字符统计块移除：执行清单

> 状态：**待用户审核（未开始实现）**
> 日期：2026-11-14
> 设计依据：[design.md](./design.md)
> 检索工具：`codegraph`（project `lx-tui`）、`codebase-memory-mcp`（project `lx-tui` / `Users-yonah-projects-agent-lx-tui`）均已索引本仓库。

---

## 0. 审核门槛 G0

- [ ] 用户审核 `design.md` 与 `task.md`，明确批准实施。
- [ ] 确认 §12 待审核项：默认 `7d`、Overdue 全库口径、Groups 标签、Todo tab 位置、不做 Home 卡片。
- [ ] 开工前确认工作区干净；`SCHEMA_VERSION` 保持 29，不改迁移。

> 任一步发现设计与代码事实冲突，先停下回报，不擅自改边界。

---

## 1. Phase 1：移除 TUI 字符统计块

**目标文件**

- `src/ui/right_panel/todo.rs`
- `src/ui/right_panel/todo/charts.rs`（删除）
- `src/ui/right_panel/todo/tests.rs`
- `src/ui/right_panel/todo/tests/charts.rs`（删除）
- `src/app/state.rs`
- `src/app/state/todo.rs`
- `src/app/state/todo/schedule.rs`
- `src/app/update/todo.rs`
- `src/app/update/todo/key.rs`
- `src/app/update/todo/tests.rs`
- `src/event/mouse/press.rs`
- `src/ui/text/todo.rs`
- `src/ui/text/todo/tests.rs`
- `src/ui/style/todo.rs`
- `src/db/todo/date.rs`

**步骤**

- [ ] 删除 `charts.rs` 整文件与 `tests/charts.rs` 整文件。
- [ ] `todo.rs` 删除 `pub mod charts;`、`pub use charts::{ChartSwitchHit, chart_switch_hit_at};`。
- [ ] 删除 `MIN_STATS_ROWS`、`BAR_WIDTH`；保留 `MIN_SPLIT_BODY`（周感知带降级用）。
- [ ] 删除 `stats_height` / `stats_divider_rect` / `stats_rect`。
- [ ] `list_rect` 改为：主体区扣周带后整块，右侧让出 1 列滚动条槽（见 design §4.3）。
- [ ] `render()` 删除 `charts::render_stats(area, buf, state, focused);`。
- [ ] `tests.rs` 删除 `mod charts;`。
- [ ] `app/state/todo.rs` 删除 `active_chart` 字段、初始化、`TodoChartKind` / `TodoStats` 再导出。
- [ ] `app/state/todo/schedule.rs` 删除 `TodoChartKind`、`TodoStats` 别名、`active_chart/next_chart/prev_chart/set_chart`、`trend_7_days`、`open_count`、`in_progress_count`、`priority_count`、`priority_distribution`；保留 `done_count`（`day_counts` 使用），注释改为周带口径。
- [ ] `app/state.rs` 再导出去掉 `TodoChartKind`。
- [ ] `app/update/todo.rs` 删除 `next_todo_chart` / `prev_todo_chart`。
- [ ] `app/update/todo/key.rs` 删除 `'c' | 'C'` 分支。
- [ ] `app/update/todo/tests.rs` 删除 `chart_cycles_across_three_modes`。
- [ ] `event/mouse/press.rs` 删除 `chart_switch_hit_at` 分支；保留 `focus_todo_from_click` 兜底。
- [ ] `ui/text/todo.rs` 删除 `TODO_STATS_*`、`TODO_CHART_*`、`TODO_BAR_FILLED`、`TODO_BAR_PLANNED`。
- [ ] `ui/text/todo/tests.rs` 删除 `stats_labels_are_lowercase_ascii_words`；`TODO_EDIT_PLACEHOLDER` 断言迁移到独立测试。
- [ ] `ui/style/todo.rs` 删除 `todo_bar_open/active/planned`、`todo_priority_bar`；保留 `todo_bar_done` 并更新注释。
- [ ] `db/todo/date.rs` 删除仅 `trend_7_days` 使用的 `recent_dates`。
- [ ] 新增回归 `list_rect_uses_full_body_below_week_strip`：列表高度 = 主体区 − 周带；滚动条轨道底边 = 列表底边。

**完成条件**

- `cargo test --lib ui::right_panel::todo` 通过。
- `cargo test --lib app::update::todo` 通过。
- 真机 `cargo run`：Todo 面板无统计块、`c` 无反应、列表可滚到底、周带/顺延/编辑正常。

---

## 2. Phase 2：DB 只读查询层

**目标文件**

- `src/db/todo.rs`
- `src/db/todo/query.rs`（新增）
- `src/db/todo/query/tests.rs`（新增）

**步骤**

- [ ] 在 `src/db/todo.rs` 增加 `pub mod query;` 并再导出 `TodoDailyRow` / `TodoFilter` / `TodoGroupRow` / `TodoItemRow` / `TodoPage` / `TodoSummary`。
- [ ] 实现 `TodoFilter`（start/end/status/priority/search）与固定 WHERE 片段。
- [ ] 实现 `Database::query_todos_page(filter, page, page_size) -> Result<TodoPage>`：
  - `COUNT(*)` + `LIMIT/OFFSET`；
  - `ORDER BY entry_date DESC, position ASC, id ASC`；
  - 关联子查询返回 `group_label`（组内最早条目文本）。
- [ ] 实现 `Database::query_todo_summary(filter) -> Result<TodoSummary>`：
  - `total/open/in_progress/done` 用 `SUM(status = '...')`；
  - `overdue` 忽略日期范围，遵守 status/priority/search。
- [ ] 实现 `Database::query_todo_daily(filter) -> Result<Vec<TodoDailyRow>>`：`GROUP BY entry_date ORDER BY entry_date DESC`，`remaining = planned - done`。
- [ ] 实现 `Database::query_todo_groups(filter) -> Result<Vec<TodoGroupRow>>`：
  - `WHERE ... AND group_id IS NOT NULL GROUP BY group_id`；
  - `top_priority` 由 `MIN(CASE priority ...)` 排序后映射 `TodoPriority::ALL`；
  - `label` 取组内最早条目文本。
- [ ] 测试覆盖 design §8.1 全部用例；组测试必须让每组至少 2 条（`save_todo` 会清空单成员组）。

**完成条件**

- `cargo test --lib db::todo::query` 通过。
- 不修改 `SCHEMA_VERSION`、不新增迁移。

---

## 3. Phase 3：Todo JSON API

**目标文件**

- `src/settings/service.rs`
- `src/settings/service/handlers.rs`
- `src/settings/service/todo.rs`（新增）
- `src/settings/service/todo/tests.rs`（新增）

**步骤**

- [ ] `service.rs` 增加 `mod todo;`。
- [ ] `handlers.rs` 增加：
  - `("GET", "/todo")` 返回统一 shell；
  - `(_, path) if path.starts_with("/api/todo") => super::todo::route(request, state),`。
- [ ] `service/todo.rs` 实现：
  - `route`：已知路径校验、GET 校验、参数解析、DB 打开；
  - `/api/todo/overview` → summary + daily + groups；
  - `/api/todo/items` → 分页明细 + groupLabel。
- [ ] 参数严格校验：
  - `startDate`/`endDate` 用 `parse_date` + `days_from_civil`/`civil_from_days` 往返；
  - 倒置自动交换；
  - `status`/`priority` 非法 422；
  - `page` 0/非数字 422；`pageSize` 钳制 1..=200；
  - `q` trim 后最多 200 字符。
- [ ] JSON 字段 camelCase，结构见 design §6.3。
- [ ] 测试覆盖 design §8.2：overview/items/筛选/Overdue/分页/非法参数/405/404/403/静态资源。
- [ ] 同步更新 `src/settings/service/tests.rs`：
  - 顶部 bar 断言增加 `data-view="todo"` / `href="#/todo"`；
  - `media="not all"` 数量 2 → 3；
  - `/settings`、`/usage` 页面循环加 `/todo`；
  - shell 断言加 `/todo.css` / `/todo.js` / `view-todo` / 关键 id；
  - 资源断言加 `todo.css` / `todo.js`（MIME、无 `innerHTML`、无重复 `createSelect`/`showToast`、含 `initTodoView`）；
  - 无外部资源循环加 `todo_css` / `todo_js`。

**完成条件**

- `cargo test --lib settings::service::todo` 通过。
- `cargo test --lib settings::service::tests` 通过。
- `curl` 手工：带 token 的 overview/items 返回预期 JSON；无 token 403。

---

## 4. Phase 4：Web 前端

**目标文件**

- `assets/web.html`
- `assets/web.js`
- `assets/usage.js`（修 `.range-btn` 作用域）
- `assets/todo.css`（新增）
- `assets/todo.js`（新增）
- `src/settings/service/page.rs`
- `tests/web_assets.rs`

**步骤**

### 4.1 壳与路由

- [ ] `web.html` 增加 `todo.css` link（`media="not all" id="css-todo"`）。
- [ ] 顶部 bar 追加 `Todo`（`data-view="todo" href="#/todo"`）。
- [ ] 增加 `view-todo` 区块（结构见 design §5.2，所有 id 不得改名）。
- [ ] 脚本末尾追加 `todo.js`。
- [ ] `web.js`：
  - `VIEWS` 加 `todo`；
  - `VIEW_TITLES` 加 `todo`；
  - `VIEW_CSS` 加 `todo`，`media` 切换改为遍历 `VIEW_CSS`；
  - `viewFromLocation` 支持 `#/todo` 与 `/todo`；
  - `bootWeb` 旧路径规范化加 `/todo`；
  - `showView` 加 `initOnce('todo', window.initTodoView)`。
- [ ] `page.rs` 增加 `/todo.css`、`/todo.js` 两条 `include_str!`。
- [ ] `usage.js` 两处 `.range-btn` 查询改为 `#view-usage .range-btn`。

### 4.2 页面脚本 `todo.js`

- [ ] 定义 `todoState` / `todoSelects` / `todoSearchTimer`（不得与 `usageState`、`selects` 重名）。
- [ ] `todoDateKey` / `todoShiftDate` / `resolveTodoRange` / `todoFilterParams`。
- [ ] `loadTodoDashboard`：并行 overview + items，`requestSeq` 竞态保护，页码钳制。
- [ ] `loadTodoItemsPage`：只请求 items。
- [ ] `renderTodoSummary`：6 张卡（Total/Open/In Progress/Done/Completion/Overdue）。
- [ ] `renderTodoTableSection`：3 tab 计数、激活态、空态。
- [ ] `todoBuildTable` / `todoAppendEmptyRow` / `renderTodosTable` / `renderDailyTable` / `renderGroupsTable` / `renderTodoPagination`。
- [ ] `initTodoDashboard` 绑定：
  - `#view-todo .range-btn`（Custom 显示/隐藏日期框）；
  - `#todo-custom-start` / `#todo-custom-end`；
  - 两个 `createSelect`（Status / Priority）；
  - `#todo-search`（250ms 防抖）；
  - `#todo-refresh`；
  - `#view-todo .todo-tabs .provider-tab`；
  - `#todo-page-prev` / `#todo-page-next`。
- [ ] `initTodoView()` 暴露给 `web.js`。
- [ ] 所有动态文本走 `textContent`，禁止 `innerHTML`。
- [ ] 所有顶层函数/常量避免与前面脚本重名（见 design §5.3）。

### 4.3 样式 `todo.css`

- [ ] 文件头注释「刻意重复，保持视图隔离」。
- [ ] 复制并改名：`.todo-shell` / `.todo-controls` / `.todo-main` / `.todo-content`。
- [ ] 控件：`.range-*` / `.custom-range-wrap` / `.filter-selects` / `.filter-field` / `.header-actions` / `.action-status` / `.status-dot` / `.select-*` / `button.ghost` / `input`。
- [ ] 卡片：`.summary-grid` / `.summary-card` / `.summary-*`，`.todo-summary-grid` 3 列。
- [ ] 表格：`.todo-table-section` / `.todo-tabs` / `.provider-tab` / `.table-wrap` / `.todo-table` / `.cell-truncate` / `.table-empty`。
- [ ] 徽标：`.todo-badge` + `.todo-status-*` + `.todo-priority-*`。
- [ ] 分页：`.pagination-bar` / `.pagination-actions`。
- [ ] 响应式：1100px / 640px 两档。
- [ ] 不含 `http://` / `https://` / 外链字体/图片。

### 4.4 资源契约测试

- [ ] `tests/web_assets.rs` 的 `SCRIPTS` 追加 `todo.js`（在 `usage.js` 之后）。
- [ ] 运行 `cargo test --test web_assets`，确认：
  - HTML 脚本顺序等于 `SCRIPTS`；
  - 单文件与拼接脚本 `node --check` 通过；
  - `todo.js` 的静态 id 全部存在于 `web.html`。

**完成条件**

- `cargo test --test web_assets` 通过。
- `cargo test --lib settings::service::tests` 通过。
- 浏览器手工：Todo 页默认 7d、三表切换、筛选/搜索/翻页/刷新、空态、窄屏正常。

---

## 5. Phase 5：规范文档同步

**目标文件**

- `docs/standards/project-directory-structure.md`
- `docs/standards/tui-design-requirements.md`
- `docs/standards/html-assets.md`

**步骤**

- [ ] `project-directory-structure.md`：
  - `right_panel/todo/` 删除「底部三视图统计图表（charts.rs）」；
  - `service.rs` 段补 `service/todo.rs`；
  - `db/todo` 段补 `query.rs`；
  - `assets/` 段补 `todo.css` / `todo.js`，`web.html` 描述加 Todo。
- [ ] `tui-design-requirements.md` Todo 段落：
  - 「统计不放在工具栏，统一由底部统计块展示」改为「统计交给 Web 控制台 Todo 页」；
  - 删除统计块、三态柱图、轴标签、分割线切图器、`c` 切图、`<7` 不切分等描述；
  - 顺手校正同段已过时的工具栏描述：文档仍写 `[clear done] [del] [edit]`，实际只有 `[+]` / `[sort]`；
  - 点击区域删除「分割线 / 统计块」；
  - 滚轮描述删除「不覆盖底部统计块」。
- [ ] `html-assets.md`：
  - 目录表补 `todo.css` / `todo.js`；
  - `web.html` 描述 Home / Settings / Usage → 加 Todo；
  - 路径与 hash 列表补 `#/todo`、`/todo`；
  - 脚本加载顺序补 `todo.js`；
  - 视图样式隔离说明补 `todo.css`。

**完成条件**

- 三份规范与实现一致；`rg "charts.rs|统计块|切图" docs/standards` 无残留（历史 feature 文档不动）。

---

## 6. Phase 6：全量验证

- [ ] `cargo fmt`
- [ ] `cargo clippy --all-targets -- -D warnings`
- [ ] `cargo test --lib db::todo::query`
- [ ] `cargo test --lib settings::service::todo`
- [ ] `cargo test --test web_assets`
- [ ] `cargo test --lib ui::right_panel::todo`
- [ ] `cargo test --lib app::update::todo`
- [ ] `cargo test`
- [ ] `cargo run` 手工验收（design §8.5）

**完成条件**

- 以上命令全绿；无新增 clippy 警告；TUI 与浏览器手工项全部确认。

---

## 7. 交付物

- [ ] 代码改动按 Phase 1–4 完成。
- [ ] 规范文档按 Phase 5 同步。
- [ ] 测试按 Phase 6 全绿。
- [ ] 在本目录补 `review.md`（实现完成后按项目惯例记录 P0/P1/P2 审查项）。

---

## 8. 明确不做

- Web 写操作（新增/勾选/删除/排序）。
- SVG 图表、自动刷新、Home 页 Todo 卡片。
- Schema 迁移、`SCHEMA_VERSION` 变更。
- TUI 列表/周带/顺延/编辑/聚合组/排序逻辑改造。
