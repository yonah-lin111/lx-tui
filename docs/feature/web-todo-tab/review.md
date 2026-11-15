# Web 控制台 Todo 看板 + TUI 字符统计块移除：复审记录

> 日期：2026-11-15
> 分支：`web-todo-tab`（worktree `.worktrees/web-todo-tab`）
> 合并提交：`ddd4ec6d`（`merge: web todo dashboard and glass theme into dev`）
> 范围：`dev...HEAD` 全部生产代码、测试与 feature/standards 文档；重点覆盖 Todo DB/API、Home 摘要卡、Todo 视图交互、TUI 统计块移除
> 方式：3 个只读 subagent 并行审查（Rust/API、前端交互、样式/TUI/文档），结论回到 HEAD 代码与定向测试复核
> 结论：**P0 1 项、P1 2 项、P2 12 项，可修项已在本分支落地并随 `ddd4ec6d` 合并到 `dev`；人工真机验收待确认**

## 关键点

1. **Todo 看板是纯只读链路**：`/api/todo/overview`、`/api/todo/items`、`/api/todo/home` 只发 SELECT，写操作仍归 TUI。
2. **Home 摘要卡是新数据面**：计数与列表必须同一读快照；列表固定最多 5 条，排序口径为优先级 → 日期升序 → 位置。
3. **Todo 交互的隐患在“筛选/翻页/跳转后的视图状态”**：滚动位置、tab 选中态、分页可见性与请求竞态要一起收敛。
4. **Groups 表是独立列契约**：表头与行数据必须严格同序；Rust 测试无法覆盖 DOM 列序，需要浏览器 harness。
5. **TUI 统计块删除是干净的**：`charts.rs`、`TodoStats`、`active_chart`、`c` 切图与相关测试均已移除，无死字段/死引用。
6. **文档漂移集中在加载态、Home 卡片与目录地图**：D12/D13 与 standards 必须同步，否则后续会按旧标准改回固定宽度。

## 验证结果

| 项目 | 结果 |
| --- | --- |
| `cargo test --lib db::todo::query::tests` | 15 passed |
| `cargo test --lib settings::service::todo::tests` | 9 passed（含 Home limit、日期倒置、pageSize 钳制） |
| `cargo test --lib app::update::todo::tests::list_c_key...` | 1 passed |
| `cargo test --lib settings::service::tests::page_requires...` | 1 passed |
| `cargo test --test web_assets` | 2 passed（脚本拼接 `node --check`） |
| `cargo test` | 全量通过（lib 5568 passed，集成/文档测试通过） |
| `cargo clippy --all-targets -- -D warnings` | 通过 |
| 浏览器 harness | Groups 列序、Home 重试、Todo 滚动回顶、tooltip 切视图隐藏、500px 布局通过 |
| `cargo run` 真机 | 待用户确认列表/周带/顺延/编辑与 Home 卡片 |

## P0（已修复）

### P0-1 Groups 表 `Items` 与 `Top Priority` 两列数据错位

**位置**

- `assets/todo.js` 的 `renderGroupsTable`
- 规格：`docs/feature/web-todo-tab/design.md` §5.4

**现象**

表头为 `Date / Group / Items / Top Priority / Open / Doing / Done`，但每行按 `date → group → topPriority → items → open → inProgress → done` 追加。结果 `Items` 列显示 P0–P3 徽标，`Top Priority` 列显示条目数；后三列恰好仍对齐，肉眼不易第一眼发现。

**错误原因**

实现时把优先级单元格插在条目数之前，表头却按设计文档的 `Items → Top Priority` 顺序生成；两端没有共享列定义，也没有 DOM 列序测试。

**修复**

按表头顺序重排单元格：`items` 在 `topPriority` 之前，并在代码旁注明列序必须与表头一致。

**验证**

浏览器 harness 直接渲染 Groups：表头 `Date/Group/Items/Top Priority/Open/Doing/Done`，首行单元格 `2026-11-15 / Review web Todo dashboard tables / 3 / P0 / 2 / 1 / 0`，`Items` 与 `Top Priority` 均为 `num`/徽标正确列。

## P1（已修复）

### P1-1 筛选/翻页后 `.todo-main` 停在旧滚动位置

**位置**

- `assets/todo.js` 的 `loadTodoDashboard` / `loadTodoItemsPage`
- `assets/todo.css` 的 `.todo-main`

**现象**

分页条位于滚动容器 `.todo-main` 底部；滚到底部点 Next 后，新页仍停在原像素位置，用户看到的是第 2 页中后段，容易误判“没有翻页”。从 Home 点条目跳转时也可能落在表格之外。

**错误原因**

渲染函数只替换 DOM，不调整滚动容器；50 行/页时新旧页高度接近，浏览器不会自动回顶。

**修复**

新增 `todoScrollToTop()`：筛选/搜索参数变化时在 `loadTodoDashboard` 渲染后回顶；翻页在 `loadTodoItemsPage` 渲染后回顶。手动 Refresh 的筛选参数不变，保持当前位置。

**验证**

harness 将 `.todo-main.scrollTop` 设为 300 后调用 `loadTodoItemsPage(2)`，渲染完成 `scrollTop == 0`；筛选变更同样回顶，手动 Refresh 保持。

### P1-2 离开 Todo 后图表 tooltip 残留并失去样式

**位置**

- `assets/todo_charts.js` 的 `todoChartHideTooltip`
- `assets/web.js` 的 `showView`

**现象**

悬停图表后通过键盘/浏览器后退切走，`#view-todo` 的 `mouseleave` 不保证触发；tooltip 挂在 `document.body`，其定位与尺寸样式只在 `todo.css`。视图 CSS 被 `media="not all"` 挂起后，tooltip 变成 body 末尾的未样式化可见块。

**错误原因**

`showView` 只对 Usage 调用了 `hideChartTooltip()`，Todo 的 tooltip 没有纳入视图切换清理。

**修复**

`showView` 在 `view !== 'todo'` 时调用 `todoChartHideTooltip()`，与 Usage 路径对齐。

**验证**

harness 手动显示 tooltip 后切到 Home：`hidden=true`、`display:none`，不再占据 body 布局。

## P2（已修复）

### P2-1 Home 计数与列表不在同一读快照

**位置**

- `src/db/todo/query.rs` 的 `query_todo_home`

**现象**

TUI 在两次 SELECT 之间写入/完成/删除 todo 时，Home 可能显示 `todayOpen=4` 但列表只有 3 条，或计数与条目不一致。

**错误原因**

计数与列表各自执行一条 SQL，WAL 下每条语句独立取快照。

**修复**

`query_todo_home` 用 `unchecked_transaction()` 包住计数与列表，提交前保持同一 deferred read transaction。

**验证**

定向测试通过；并发窗口从代码层消除。Overview 的四段查询仍各自独立，见“接受风险”。

### P2-2 Groups 空态文案误导

**位置**

- `assets/todo.js` 的 `renderGroupsTable`

**现象**

有 Todo 但都没有 `group_id` 时，Groups 表显示 `No todos in the selected range.`，而 Todos/Daily 明明有数据。

**错误原因**

复用了 Todos 的默认空态文案。

**修复**

Groups 传入 `No grouped todos in the selected range.`。

### P2-3 Home 跳转在慢请求/失败时残留旧 tab

**位置**

- `assets/todo.js` 的 `todoFocusFromHome` / `renderTodoTableSection`

**现象**

在 Daily/Groups tab 离开 Todo，从 Home 点条目；搜索框已预填，但 tab 仍停在旧的 Daily，分页仍隐藏，直到异步渲染完成；失败时长期不一致。

**错误原因**

`todoFocusFromHome` 只改 `todoState.activeTab`，tab class 与分页可见性由后续数据渲染顺带更新。

**修复**

抽出 `todoSyncTableTabState()`，在 `todoFocusFromHome` 立即同步三个 tab 的 `active/aria-selected` 与分页可见性。

### P2-4 Home 首次失败没有重试入口

**位置**

- `assets/home.js` 的 `homeSetNote` / `loadHomeSummary`

**现象**

首次加载失败只显示 `Todo unavailable.`；已在 `#/` 时点 Home tab 不触发 hashchange，只能先切走再切回。

**错误原因**

错误提示是纯文本，没有把 note 变成可操作入口。

**修复**

首次失败时 note 增加 `role="button"`、`tabindex=0`、点击/Enter/Space 重试；已有数据刷新失败仍保留旧列表，不显示重试。

**验证**

harness 屏蔽 `/api/todo/home`：显示 `Todo unavailable.` 且 `role=button`；解除屏蔽后点击 note，恢复 5 条列表。

### P2-5 关键行为缺少回归测试

**位置**

- `src/settings/service/todo/tests.rs`
- `src/settings/service/tests.rs`
- `src/app/update/todo/tests.rs`

**现象**

- `startDate > endDate` 的交换只在服务层实现，无测试；DB 测试反而断言“不交换”。
- Home 服务测试种子恰好 5 条未完成，`LIMIT 5` 失效也会通过。
- HTTP 层 `pageSize` 钳制无断言。
- `home.js` / `todo_charts.js` 未纳入“无外部资源”循环。
- 删除 `c` 切图后没有“`c` 被吞掉”的回归测试。

**错误原因**

实现与测试同步演进时只补了正向路径，没有为“服务层归一化”“上限截断”“资源白名单”“已删除快捷键”建立反向断言。

**修复**

补：日期倒置等价、Home 6 条截到 5 条、`pageSize=0/999` 钳制、`home.js`/`todo_charts.js` 外部资源断言、列表态 `c` 无副作用且编辑态仍可输入。

### P2-6 文档与实现漂移

**位置**

- `docs/feature/web-todo-tab/task.md` §0
- `docs/feature/web-todo-tab/design.md` §5.9/§6.4
- `docs/standards/html-assets.md`
- `docs/standards/project-directory-structure.md`
- `docs/standards/tui-design-requirements.md`

**现象**

- §0 仍写“不做 Home 卡片”，与 Phase 9 冲突。
- `.action-status` 标准要求预留最小宽度，与 D12 相反。
- 目录规范缺 `home.js`、`theme.css` 与 `/api/todo/home`。
- TUI 规范一句内“聚焦”与“不聚焦”自相矛盾。
- `page=0` 的实现文案比设计表格更具体。

**错误原因**

功能迭代按提交粒度只回写部分文档；Home 卡片、状态宽度与主题资源是在后续提交追加的，早期 checklist/目录地图没有同步更新。

**修复**

§0 标注 D13 变更；标准改为宽度跟随文案；目录规范补资源与路由；TUI 规范改为“吞掉但仍聚焦容器”；设计 §6.4 单列 `page=0` 文案；本文件重写为当前复审记录。

### P2-7 `todo.css` 表头 `position: sticky` 是死声明

**位置**

- `assets/todo.css` 的 `.todo-table th`

**现象**

`.table-wrap` 只设 `overflow-x:auto`，按 CSS 规则 `overflow-y` 计算为 `auto`，成为最近滚动容器；它纵向不滚动，`sticky top:0` 永远不触发。

**错误原因**

从 Usage 复制表格样式时保留了 `sticky`，但 Todo 的纵向滚动发生在 `.todo-main`，`.table-wrap` 不是纵向滚动上下文。

**修复**

删除 `position: sticky; top: 0;`，避免误导后续维护者。

## P2（接受）

### P2-8 Overview 四段查询不在同一读事务

`overview_json` 连续执行 summary/priority/daily/groups 四条查询。TUI 并发写入时可能出现短暂不一致，下次刷新自愈。**接受**：当前个人 Todo 量级与请求频率下影响极小；把四段查询合并需要重构查询层或暴露事务 API，收益不抵改动面。

### P2-9 服务层测试跨午夜偶发漂移

测试种子用 `today_date()` 计算日期，handler 在请求时再次调用；若跨本地 00:00，Home/Overdue/Daily 断言会漂移。**接受**：窗口极窄，CI 不在午夜定时跑；彻底修复需要给 `ServiceState` 注入可测试时钟，暂不扩大改动。

### P2-10 `group_id` 无索引

分页/Home/Groups 的 `group_label` 关联子查询按 `group_id` 过滤，但 schema 无该索引。**接受**：当前数据量小（人类手写清单）；新增索引需迁移并顺延 `SCHEMA_VERSION`，留到数据量有实际压力时再做。

### P2-11 `initOnce` 异步失败记为 `done`

见 web-theme 复审 P2-6。Todo 首次加载失败有行内 banner + Refresh，**接受**当前语义，避免 loader reject 引发重复绑定或未处理 rejection。

### P2-12 Home 请求悬挂无超时

`loadHomeSummary` 用 `homeState.loading` 防重入；fetch 若长时间悬挂，重试会被挡住。**接受**：本地回环请求通常毫秒级；后续给 `apiGet` 加 AbortController 超时再统一处理。

---

## 历史修复（2026-11-14）

初版实现已处理：Usage 全局 `.range-btn` 误绑 Todo、跨文件全局作用域重名、聚合组测试假数据、Overdue 在 Today 下归零、DB 读系统时钟、日期宽松校验。详见本文件历史版本与 `design.md` D1–D12。

## 已核实无问题

- SQL 口径：Today = 今日未完成；In progress = 今日及更早进行中；Overdue = 今天之前未完成；Home 列表排除 done/future，按优先级、日期升序、位置排序。
- Home 路由、非法参数 422、非 GET 405、未知路径 404、DB 错误 500；token 校验先于 `/api/todo/*` 分派。
- 生产代码无 `unwrap()/expect()`；SQL 用户输入全部 `params!` 绑定。
- TUI 删除彻底：`charts.rs` / `tests/charts.rs` 已删，`TodoStats`、`active_chart`、`TodoChartKind`、`c` 切图与相关样式/文案无残留引用；列表、周带、顺延、行内编辑测试全绿。
- HTML 无重复 id；脚本顺序与 `tests/web_assets.rs` 一致；`getElementById` 静态引用均存在。
- 浏览器脚本无 `innerHTML`/`eval`，文本统一 `textContent`。
