# Web 控制台 Todo 看板 + TUI 字符统计块移除：执行清单

> 状态：**Phase 1–10 已实现并通过复审修复，已随 `ddd4ec6d` 合并到 `dev`；全量验证通过，真机验收待完成**
> 日期：2026-11-14
> 设计依据：[design.md](./design.md)
> 检索工具：`codegraph`（project `lx-tui`）、`codebase-memory-mcp`（project `lx-tui` / `Users-yonah-projects-agent-lx-tui`）均已索引本仓库。

---

## 0. 审核门槛 G0

- [x] 用户审核 `design.md` 与 `task.md`，明确批准实施。
- [x] 确认 §12 待审核项：默认 `7d`、Overdue 全库口径、Groups 标签、Todo tab 位置；初版不做 Home 卡片，后按 D13 追加实时摘要卡（见 Phase 9）。
- [x] 开工前确认工作区干净；`SCHEMA_VERSION` 保持 29，不改迁移。

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

- [x] 删除 `charts.rs` 整文件与 `tests/charts.rs` 整文件。
- [x] `todo.rs` 删除 `pub mod charts;`、`pub use charts::{ChartSwitchHit, chart_switch_hit_at};`。
- [x] 删除 `MIN_STATS_ROWS`、`BAR_WIDTH`；保留 `MIN_SPLIT_BODY`（周感知带降级用）。
- [x] 删除 `stats_height` / `stats_divider_rect` / `stats_rect`。
- [x] `list_rect` 改为：主体区扣周带后整块，右侧让出 1 列滚动条槽（见 design §4.3）。
- [x] `render()` 删除 `charts::render_stats(area, buf, state, focused);`。
- [x] `tests.rs` 删除 `mod charts;`。
- [x] `app/state/todo.rs` 删除 `active_chart` 字段、初始化、`TodoChartKind` / `TodoStats` 再导出。
- [x] `app/state/todo/schedule.rs` 删除 `TodoChartKind`、`TodoStats` 别名、`active_chart/next_chart/prev_chart/set_chart`、`trend_7_days`、`open_count`、`in_progress_count`、`priority_count`、`priority_distribution`；保留 `done_count`（`day_counts` 使用），注释改为周带口径。
- [x] `app/state.rs` 再导出去掉 `TodoChartKind`。
- [x] `app/update/todo.rs` 删除 `next_todo_chart` / `prev_todo_chart`。
- [x] `app/update/todo/key.rs` 删除 `'c' | 'C'` 分支。
- [x] `app/update/todo/tests.rs` 删除 `chart_cycles_across_three_modes`。
- [x] `event/mouse/press.rs` 删除 `chart_switch_hit_at` 分支；保留 `focus_todo_from_click` 兜底。
- [x] `ui/text/todo.rs` 删除 `TODO_STATS_*`、`TODO_CHART_*`、`TODO_BAR_FILLED`、`TODO_BAR_PLANNED`。
- [x] `ui/text/todo/tests.rs` 删除 `stats_labels_are_lowercase_ascii_words`；`TODO_EDIT_PLACEHOLDER` 断言迁移到独立测试。
- [x] `ui/style/todo.rs` 删除 `todo_bar_open/active/planned`、`todo_priority_bar`；保留 `todo_bar_done` 并更新注释。
- [x] `db/todo/date.rs` 删除仅 `trend_7_days` 使用的 `recent_dates`。
- [x] 新增回归 `list_rect_uses_full_body_below_week_strip`：列表高度 = 主体区 − 周带；滚动条轨道底边 = 列表底边。

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

- [x] 在 `src/db/todo.rs` 增加 `pub mod query;` 并再导出 `TodoDailyRow` / `TodoFilter` / `TodoGroupRow` / `TodoItemRow` / `TodoPage` / `TodoSummary`。
- [x] 实现 `TodoFilter`（start/end/status/priority/search）与固定 WHERE 片段。
- [x] 实现 `Database::query_todos_page(filter, page, page_size) -> Result<TodoPage>`：
  - `COUNT(*)` + `LIMIT/OFFSET`；
  - `ORDER BY entry_date DESC, position ASC, id ASC`；
  - 关联子查询返回 `group_label`（组内最早条目文本）。
- [x] 实现 `Database::query_todo_summary(filter) -> Result<TodoSummary>`：
  - `total/open/in_progress/done` 用 `SUM(status = '...')`；
  - `overdue` 忽略日期范围，遵守 status/priority/search。
- [x] 实现 `Database::query_todo_daily(filter) -> Result<Vec<TodoDailyRow>>`：`GROUP BY entry_date ORDER BY entry_date DESC`，`remaining = planned - done`。
- [x] 实现 `Database::query_todo_groups(filter) -> Result<Vec<TodoGroupRow>>`：
  - `WHERE ... AND group_id IS NOT NULL GROUP BY group_id`；
  - `top_priority` 由 `MIN(CASE priority ...)` 排序后映射 `TodoPriority::ALL`；
  - `label` 取组内最早条目文本。
- [x] 测试覆盖 design §8.1 全部用例；组测试必须让每组至少 2 条（`save_todo` 会清空单成员组）。

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

- [x] `service.rs` 增加 `mod todo;`。
- [x] `handlers.rs` 增加：
  - `("GET", "/todo")` 返回统一 shell；
  - `(_, path) if path.starts_with("/api/todo") => super::todo::route(request, state),`。
- [x] `service/todo.rs` 实现：
  - `route`：已知路径校验、GET 校验、参数解析、DB 打开；
  - `/api/todo/overview` → summary + daily + groups；
  - `/api/todo/items` → 分页明细 + groupLabel。
- [x] 参数严格校验：
  - `startDate`/`endDate` 用 `parse_date` + `days_from_civil`/`civil_from_days` 往返；
  - 倒置自动交换；
  - `status`/`priority` 非法 422；
  - `page` 0/非数字 422；`pageSize` 钳制 1..=200；
  - `q` trim 后最多 200 字符。
- [x] JSON 字段 camelCase，结构见 design §6.3。
- [x] 测试覆盖 design §8.2：overview/items/筛选/Overdue/分页/非法参数/405/404/403/静态资源。
- [x] 同步更新 `src/settings/service/tests.rs`：
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

- [x] `web.html` 增加 `todo.css` link（`media="not all" id="css-todo"`）。
- [x] 顶部 bar 追加 `Todo`（`data-view="todo" href="#/todo"`）。
- [x] 增加 `view-todo` 区块（结构见 design §5.2，所有 id 不得改名）。
- [x] 脚本末尾追加 `todo.js`。
- [x] `web.js`：
  - `VIEWS` 加 `todo`；
  - `VIEW_TITLES` 加 `todo`；
  - `VIEW_CSS` 加 `todo`，`media` 切换改为遍历 `VIEW_CSS`；
  - `viewFromLocation` 支持 `#/todo` 与 `/todo`；
  - `bootWeb` 旧路径规范化加 `/todo`；
  - `showView` 加 `initOnce('todo', window.initTodoView)`。
- [x] `page.rs` 增加 `/todo.css`、`/todo.js` 两条 `include_str!`。
- [x] `usage.js` 两处 `.range-btn` 查询改为 `#view-usage .range-btn`。

### 4.2 页面脚本 `todo.js`

- [x] 定义 `todoState` / `todoSelects` / `todoSearchTimer`（不得与 `usageState`、`selects` 重名）。
- [x] `todoDateKey` / `todoShiftDate` / `resolveTodoRange` / `todoFilterParams`。
- [x] `loadTodoDashboard`：并行 overview + items，`requestSeq` 竞态保护，页码钳制。
- [x] `loadTodoItemsPage`：只请求 items。
- [x] `renderTodoSummary`：6 张卡（Total/Open/In Progress/Done/Completion/Overdue）。
- [x] `renderTodoTableSection`：3 tab 计数、激活态、空态。
- [x] `todoBuildTable` / `todoAppendEmptyRow` / `renderTodosTable` / `renderDailyTable` / `renderGroupsTable` / `renderTodoPagination`。
- [x] `initTodoDashboard` 绑定：
  - `#view-todo .range-btn`（Custom 显示/隐藏日期框）；
  - `#todo-custom-start` / `#todo-custom-end`；
  - 两个 `createSelect`（Status / Priority）；
  - `#todo-search`（250ms 防抖）；
  - `#todo-refresh`；
  - `#view-todo .todo-tabs .provider-tab`；
  - `#todo-page-prev` / `#todo-page-next`。
- [x] `initTodoView()` 暴露给 `web.js`。
- [x] 所有动态文本走 `textContent`，禁止 `innerHTML`。
- [x] 所有顶层函数/常量避免与前面脚本重名（见 design §5.3）。

### 4.3 样式 `todo.css`

- [x] 文件头注释「刻意重复，保持视图隔离」。
- [x] 复制并改名：`.todo-shell` / `.todo-controls` / `.todo-main` / `.todo-content`。
- [x] 控件：`.range-*` / `.custom-range-wrap` / `.filter-selects` / `.filter-field` / `.header-actions` / `.action-status` / `.status-dot` / `.select-*` / `button.ghost` / `input`。
- [x] 卡片：`.summary-grid` / `.summary-card` / `.summary-*`，`.todo-summary-grid` 3 列。
- [x] 表格：`.todo-table-section` / `.todo-tabs` / `.provider-tab` / `.table-wrap` / `.todo-table` / `.cell-truncate` / `.table-empty`。
- [x] 徽标：`.todo-badge` + `.todo-status-*` + `.todo-priority-*`。
- [x] 分页：`.pagination-bar` / `.pagination-actions`。
- [x] 响应式：1100px / 640px 两档。
- [x] 不含 `http://` / `https://` / 外链字体/图片。

### 4.4 资源契约测试

- [x] `tests/web_assets.rs` 的 `SCRIPTS` 追加 `todo.js`（在 `usage.js` 之后）。
- [x] 运行 `cargo test --test web_assets`，确认：
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

- [x] `project-directory-structure.md`：
  - `right_panel/todo/` 删除「底部三视图统计图表（charts.rs）」；
  - `service.rs` 段补 `service/todo.rs`；
  - `db/todo` 段补 `query.rs`；
  - `assets/` 段补 `todo.css` / `todo.js`，`web.html` 描述加 Todo。
- [x] `tui-design-requirements.md` Todo 段落：
  - 「统计不放在工具栏，统一由底部统计块展示」改为「统计交给 Web 控制台 Todo 页」；
  - 删除统计块、三态柱图、轴标签、分割线切图器、`c` 切图、`<7` 不切分等描述；
  - 顺手校正同段已过时的工具栏描述：文档仍写 `[clear done] [del] [edit]`，实际只有 `[+]` / `[sort]`；
  - 点击区域删除「分割线 / 统计块」；
  - 滚轮描述删除「不覆盖底部统计块」。
- [x] `html-assets.md`：
  - 目录表补 `todo.css` / `todo.js`；
  - `web.html` 描述 Home / Settings / Usage → 加 Todo；
  - 路径与 hash 列表补 `#/todo`、`/todo`；
  - 脚本加载顺序补 `todo.js`；
  - 视图样式隔离说明补 `todo.css`。

**完成条件**

- 三份规范与实现一致；`rg "charts.rs|统计块|切图" docs/standards` 无残留（历史 feature 文档不动）。

---

## 6. Phase 6：全量验证

- [x] `cargo fmt`
- [x] `cargo clippy --all-targets -- -D warnings`
- [x] `cargo test --lib db::todo::query`
- [x] `cargo test --lib settings::service::todo`
- [x] `cargo test --test web_assets`
- [x] `cargo test --lib ui::right_panel::todo`
- [x] `cargo test --lib app::update::todo`
- [x] `cargo test`
- [ ] `cargo run` 手工验收（design §8.5）

**完成条件**

- 以上命令全绿；无新增 clippy 警告；TUI 与浏览器手工项全部确认。

---

## 7. Phase 7：追加图表（2026-11-15）

- [x] DB：`query_todo_priority_counts` + 单测（空表、范围/状态/优先级/搜索）。
- [x] API：`overview` 增加 `priority: {p0..p3}` + HTTP 断言。
- [x] 前端：新建 `assets/todo_charts.js`（日趋势分组柱、状态环图、优先级横条、tooltip、空态、分日/周/月分桶）。
- [x] 前端：`web.html` 图表区与脚本顺序、`todo.css` 图表样式、`todo.js` 调用 `renderTodoCharts`。
- [x] 资源契约：`page.rs` 路由、`tests/web_assets.rs` 脚本清单、shell 断言、`/todo_charts.js` 请求测试。
- [x] 文档：`design.md`（D3 作废、D9/D10、§5.7）、`html-assets.md` 脚本顺序。
- [x] 验证：`cargo fmt`、`cargo clippy --all-targets -- -D warnings`、`cargo test`、node 拼接语法检查。

---

## 8. Phase 8：加载态与状态宽度（2026-11-15）

- [x] 全局进度条 `#page-loading`：懒加载视图的 init 返回首次加载 Promise，`web.js` 等待 settle，最短 180ms，不遮挡内容。
- [x] Todo 局部骨架：6 张汇总卡、3 个图表块、状态 legend、表格行；只在首次加载绘制，数据到达淡入一次。
- [x] Usage 局部骨架：8 张汇总卡、5 个图表块、表格行。
- [x] Settings 使用全局进度条（`initSettingsView` 返回 `loadState()`）。
- [x] 状态宽度：Usage/Todo/Settings `.action-status` 不预留固定宽度，统一 `tabular-nums` + `nowrap` 让宽度跟随文案；刷新只转内联 spinner，不改写时间戳。
- [x] 文档：`design.md` D11/D12 与 §5.8、`html-assets.md` 加载态约定。
- [x] 全量验证：`cargo fmt`、`cargo clippy --all-targets -- -D warnings`、`cargo test`、浏览器骨架/图表截图。
- [ ] `cargo run` 手工验收（D11/D12 与图表）。

---

## 9. 交付物

- [x] 代码改动按 Phase 1–4 完成。
- [x] 规范文档按 Phase 5 同步。
- [x] 测试按 Phase 6 全绿。
- [x] 在本目录补 `review.md`（实现完成后按项目惯例记录 P0/P1/P2 审查项）。
- [x] 图表按 Phase 7 完成并验证。
- [x] 加载态与状态宽度按 Phase 8 完成并验证。
- [x] Home Todo 摘要卡按 Phase 9 完成并验证。

---

## 10. Phase 9：Home Todo 摘要卡（2026-11-15 追加）

- [x] 后端：`GET /api/todo/home` 返回 `date`、`summary`（`todayOpen` / `inProgress` / `overdue`）与最多 5 条今日 + 逾期未完成条目；DB 查询 `query_todo_home` 按优先级、日期升序、位置排序。
- [x] Home HTML：宽 Todo 卡 + 右侧 Settings / Usage 入口；统计块、条目列表与行内加载/空态/错误提示。
- [x] `assets/home.js`：只读拉取与渲染；点击条目跳转 `#/todo` 并通过 `todoFocusFromHome` 预填搜索、范围切到 All。
- [x] 样式：Default 方角 + Glass 玻璃卡片；≤720px 单列、≤640px 入口竖排；`prefers-reduced-motion` 关闭过渡。
- [x] 测试：DB 计数/排序/limit、HTTP JSON、`/home.js` 资源契约、脚本拼接解析。
- [ ] `cargo run` 手工验收 Home 卡片与条目跳转。

---

## 11. Phase 10：复审修复（2026-11-15 追加）

- [x] P0：修正 Groups 表 `Items` 与 `Top Priority` 列错位。
- [x] P1：筛选/搜索/翻页后 `.todo-main` 回到顶部；手动 Refresh 保持位置。
- [x] P1：切离 Todo 时隐藏图表 tooltip，避免 `todo.css` 挂起后残留。
- [x] P2：Home 首次失败支持卡内重试；Groups 空态文案改为分组专用。
- [x] P2：Home 跳转立即同步 Todos tab 与分页可见性，慢请求/失败不残留旧 tab。
- [x] P2：Home 计数与列表改为同一读事务。
- [x] P2：补日期倒置、Home 5 条上限、`pageSize` 钳制、`c` 键无副作用、`home.js`/`todo_charts.js` 外部资源断言。
- [x] P2：同步 `design.md` §5.10、§6.4 与本清单；`review.md` 记录本轮 P0–P2 与接受风险。
- [ ] `cargo run` 真机复验 Phase 8/9/10（与 Home 卡片、Groups 表、滚动位置、tooltip 一并确认）。

---

## 12. 明确不做

- Web 写操作（新增/勾选/删除/排序）。
- 图表缩放、图例开关、自动刷新。
- Schema 迁移、`SCHEMA_VERSION` 变更。
- TUI 列表/周带/顺延/编辑/聚合组/排序逻辑改造。
