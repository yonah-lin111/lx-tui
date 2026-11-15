# Web 控制台 Todo 看板 + TUI 字符统计块移除：设计方案

> 状态：**Phase 1–10 已实现并通过复审修复；全量验证与真机验收待完成**
> 日期：2026-11-14（2026-11-15 追加图表与加载体验需求，见 D9–D12 与 §5.7/§5.8）
> 项目：`/Users/yonah/projects/agent/lx-tui`（分支 `dev`，工作区干净）
> 配套执行清单：[task.md](./task.md)
> 代码检索：`codegraph`（project `lx-tui`，indexed）+ `codebase-memory-mcp`（project `lx-tui` / `Users-yonah-projects-agent-lx-tui`，indexed）
> 参考实现：`assets/usage.js`、`assets/usage.css`、`src/settings/service/usage.rs`、`src/db/usage/`

---

## 1. 需求与验收

### 1.1 需求原文

1. **移除** `src/ui/right_panel/todo/` 的字符表格。
2. 在 `assets/web.html` 顶部 bar 新增 `Todo` tab。
3. Todo 页展示**几张 todo 表格**，支持**条件筛选**，内容与交互参考 Usage 页。

### 1.2 本次口径（已逐题确认）

- 「字符表格」= Todo 面板底部的**字符统计块**：`total · doing · done` 摘要行 + 三张字符柱状图 + 分割线上的 `‹ title n/3 ›` 切换器。**不删**列表、工具栏、周感知带、顺延横幅、行内编辑。
- Web Todo 页**只读**：只做查询、筛选、分页、统计展示，不新增、不勾选、不删除。
- 图表按 2026-11-15 追加需求执行：卡片下常驻三图（日趋势、状态环图、优先级横条），跟随全部筛选；详见 §5.7。
- TUI 删除统计块后，列表独占主体区剩余高度。

### 1.3 验收标准

**TUI**

- Todo 面板不再出现 `├ ─ ┤` 统计分割线、`total/doing/done` 摘要、`█`/`░` 柱图、`‹ … 1/3 ›` 切换器。
- `c` 键不再有切图语义；点击原统计块区域只让容器进入选中态，不产生数据动作。
- 列表、周感知带、顺延横幅、`[+]` / `[sort]`、行内编辑、滚动条行为不变。
- 空列表仍显示 `no todos yet`。

**Web**

- 顶部 bar 出现 `Todo`，hash 路由 `#/todo` 可直达；`/todo` 直接访问同样返回统一 shell。
- 默认 `7d`，日期范围 `Today / 7d / 30d / All / Custom`、`Status`、`Priority`、文本搜索可组合筛选。
- 6 张汇总卡：Total / Open / In Progress / Done / Completion / Overdue。
- 图表：日趋势（Planned vs Done，自动分日/周/月）、状态环图、P0–P3 横条；随筛选与日期范围更新。
- 3 张表：`Todos`（分页）、`Daily`、`Groups`；空范围有空态文案。
- 所有请求带 token；接口只读；无外部资源；无 `innerHTML`。

---

## 2. 已确认决策（Grill-Me）

| 编号 | 决策 | 理由 |
| --- | --- | --- |
| D1 | 只删 TUI 底部字符统计块（`charts.rs` 及其几何/状态/快捷键/命中），保留 Todo 列表与编辑 | Web 有真正的表格，TUI 保留「按日改状态」核心交互 |
| D2 | Web Todo 页**只读** | TUI 内存态不会热同步外部写库，双写必然互相覆盖 |
| D3 | 页面组成：顶部控件 + 6 张汇总卡 + **3 张图表** + 3 个 tab 表格（2026-11-15 追加需求，原「不画图表」作废） | 表格仍是明细入口；图表补回被删字符图的趋势/分布感知 |
| D4 | 日期范围一律按 `entry_date` | 未完成条目没有 `completed_date`，按完成日期筛会漏掉全部待办 |
| D5 | `Overdue` 卡片忽略日期范围、遵守 Status/Priority/搜索筛选，统计全库 `entry_date < 今天 && status != done`；`Daily` 表改叫 `Remaining`（Planned − Done） | `Today` 范围下历史欠账天然为 0，卡片会失效；Daily 一行代表某天，「剩余」才有意义 |
| D6 | 新建独立 `assets/todo.css`，按需复制 Usage 的控件/卡片/表格样式，并注释「刻意重复，保持视图隔离」 | 遵守 `docs/standards/html-assets.md` 的视图级样式隔离；不把 Usage 控件抽到 `web.css`，避免大范围回归 |
| D7 | 服务端做筛选、聚合与分页；前端只在翻页时单独请求 items | 与 Usage 的 `/api/usage/overview` + `/api/usage/logs` 同构，避免把全表拉到浏览器 |
| D8 | 默认范围 `7d`、默认 tab `Todos`、页大小 50 | 与 Usage 默认一致；50 条足够一屏翻页 |
| D9 | 图表三张：日趋势分组柱（Planned vs Done，tooltip 带 Remaining）、状态环图、P0–P3 优先级横条；放在汇总卡下、表格上常驻，跟随全部筛选 | 覆盖时间/状态/优先级三个维度；只有优先级分布需要补聚合 |
| D10 | 新建独立 `assets/todo_charts.js`（顶层名字 `todo` 前缀），不调用 `usage_charts.js` 的全局函数；长范围由前端分桶并补零（≤62 天按日、≤400 天按周、更久按月） | 遵守视图隔离（D6 同理）；不改 API 粒度、不截断历史；避免污染刚验收的 Usage |
| D11 | 加载体验：懒加载视图首次初始化时显示顶部 2px 进度条（不遮内容、不阻塞操作、最短 180ms）；Todo/Usage 同时渲染与最终布局同尺寸的局部骨架；Settings 只用进度条。已有数据时刷新只转状态行内联 spinner，不改写时间戳 | 遮住半成品画面又保持页面可操作；骨架保证容器高度不跳；刷新不再闪文案 |
| D12 | 状态指示器统一 `tabular-nums` + `nowrap`，宽度跟随文案（不预留 `min-width`）；容器整体右对齐，右侧操作按钮仍固定在右缘 | `Loading… / Updated HH:MM:SS / Saved / Unsaved changes` 切换时状态区不再留固定空白；数字等宽避免逐秒抖动 |
| D13 | Home 新增宽 Todo 摘要卡：Today / In progress / Overdue 计数 + 今日与逾期未完成前 5 条；右侧竖排 Settings / Usage 入口。新增只读 `GET /api/todo/home`，条目按优先级、日期升序、位置排序；点击条目跳到 Todo 并预填文本搜索、范围切到 All | Home 从静态入口升级为概览页；独立端点一次返回精确计数与列表，避免复用 items 分页后客户端排序漏掉旧逾期项 |

---

## 3. 现状与关键事实

### 3.1 TUI Todo 面板

| 位置 | 现状 |
| --- | --- |
| `src/ui/right_panel/todo.rs:6` | `pub mod charts;` 暴露统计块 |
| `src/ui/right_panel/todo.rs:15` | `pub use charts::{ChartSwitchHit, chart_switch_hit_at};` |
| `src/ui/right_panel/todo.rs:46-49` | `MIN_STATS_ROWS = 3`、`BAR_WIDTH = 3`，仅统计块使用 |
| `src/ui/right_panel/todo.rs:64-75` | `stats_height()`：主体区扣周带后与列表平分 |
| `src/ui/right_panel/todo.rs:78-94` | `list_rect()`：主体区扣周带、分割线、统计块，并让出最右 1 列滚动条槽 |
| `src/ui/right_panel/todo.rs:110-128` | `stats_divider_rect()` / `stats_rect()` |
| `src/ui/right_panel/todo.rs:594-610` | `render()` 末尾调用 `charts::render_stats(...)` |
| `src/ui/right_panel/todo/charts.rs` | 整文件 485 行：三张图、摘要行、切图器、轴标签 |
| `src/app/state/todo.rs:98-99` | `TodoPanel.active_chart: TodoChartKind` |
| `src/app/state/todo/schedule.rs:14-53` | `TodoChartKind` 与 `next/prev/number` |
| `src/app/state/todo/schedule.rs:134-155` | `active_chart` / `next_chart` / `prev_chart` / `set_chart` |
| `src/app/state/todo/schedule.rs:277-332` | `trend_7_days` / `open_count` / `in_progress_count` / `priority_count` / `priority_distribution`（仅 charts 使用） |
| `src/app/update/todo.rs:284-300` | `next_todo_chart` / `prev_todo_chart` |
| `src/app/update/todo/key.rs:317-321` | `c` / `C` 切图 |
| `src/event/mouse/press.rs:334-344` | 点击切图器分支 |
| `src/ui/text/todo.rs:28-59` | `TODO_STATS_*` / `TODO_CHART_*` / `TODO_BAR_*` |
| `src/ui/style/todo.rs:33-51,77-85` | `todo_bar_open/active/planned`、`todo_priority_bar` 仅 charts 使用；`todo_bar_done` 仍被周感知带使用 |
| `src/ui/right_panel/todo/tests/charts.rs` | 312 行统计块测试，整文件删除 |
| `src/app/update/todo/tests.rs:798-817` | `chart_cycles_across_three_modes`，删除 |
| `src/ui/text/todo/tests.rs:29-48` | `stats_labels_are_lowercase_ascii_words`，删除/迁移 `TODO_EDIT_PLACEHOLDER` 断言 |

**当前列位（保留不动）**：`❯`（1）+ 空格（1）+ `[ ]`（3）+ 空格（1）+ `P0`（2）+ 空格（1）+ 树形连接符（3）+ 正文。普通单条正文 `list.x + 9`，续行 `list.x + 6`；聚合组子项正文 `list.x + 12`。

### 3.2 Web 控制台

| 位置 | 现状 |
| --- | --- |
| `assets/web.html:28-38` | 顶部 bar：Home / Settings / Usage |
| `assets/web.html:8-9` | `settings.css`、`usage.css` 以 `media="not all"` 挂起 |
| `assets/web.html:826-835` | 脚本顺序：`web.js` → settings 系列 → `usage_charts.js` → `usage.js` |
| `assets/web.js:209-211` | `VIEWS = ['home','settings','usage']`、`VIEW_TITLES`、`VIEW_CSS` |
| `assets/web.js:213-222` | `viewFromLocation()`：hash 与旧路径 `/settings`、`/usage` |
| `assets/web.js:242-247` | 样式表 `media` 切换与 `initOnce('settings'/'usage')` |
| `src/settings/service/page.rs:8-71` | `html()` 与 `asset()` 白名单 |
| `src/settings/service/handlers.rs:220-252` | 路由分派；`/api/usage` 走 `super::usage::route` |
| `src/settings/service/usage.rs` | `/api/usage/*` 的参数解析与聚合模式（本次照抄结构） |
| `assets/usage.js` | 筛选状态机、range 按钮、自绘 select、分页、竞态序号 `requestSeq` |
| `assets/usage.css` | `.usage-controls` / `.range-btn` / `.select-*` / `.summary-*` / `.usage-table` / `.pagination-*` |
| `tests/web_assets.rs:14-26` | `SCRIPTS` 顺序与 DOM id 契约 |
| `src/settings/service/tests.rs:90-140,278-420` | shell/tab/资源断言，需同步扩展 |

### 3.3 数据库

`todo` 表字段（`SCHEMA_VERSION = 29`，**本次不改 schema**）：

```text
id TEXT PK, entry_date TEXT, text TEXT, status TEXT('todo'|'in_progress'|'done'),
priority TEXT('P0'..'P3'), completed_date TEXT NULL, position INTEGER,
group_id TEXT NULL, created_at_ms INTEGER, updated_at_ms INTEGER
```

- `src/db/todo.rs:126-140` `TodoRecord` 与字段一一对应。
- `src/db/todo.rs:209-253` 已有全表/按日/历史未完成查询。
- `src/db/todo.rs:579-603` 已有 `query_range_stats`。
- 聚合组只在同一 `entry_date` 内连续；成员数 < 2 时写操作会把 `group_id` 清空（`cleanup_singleton_groups`）。

### 3.4 运行约束

- Web 服务只监听 `127.0.0.1:0`，每次运行随机 capability token；静态资源免 token，页面与 JSON API 必须带 token。
- 视图样式按 `media="not all"` 隔离；`web.css` 只放外壳、共享 toast、滚动条与 `.hidden`。
- 资源零外部依赖；用户可见文案英文；动态文本用 `textContent`，禁止 `innerHTML`。
- 前端脚本共享同一全局作用域，跨文件不得重复声明顶层 `const` / `let` / `function`（`tests/web_assets.rs` 会拼接后 `node --check`）。

---

## 4. TUI 改造方案

### 4.1 删除清单

| 文件 | 处理 |
| --- | --- |
| `src/ui/right_panel/todo/charts.rs` | **整文件删除** |
| `src/ui/right_panel/todo.rs` | 删除 `pub mod charts;`、`pub use charts::{...}`、`MIN_STATS_ROWS`、`BAR_WIDTH`、`stats_height`、`stats_divider_rect`、`stats_rect`；简化 `list_rect`；`render` 去掉 `charts::render_stats` |
| `src/ui/right_panel/todo/tests.rs` | 删除 `mod charts;` |
| `src/ui/right_panel/todo/tests/charts.rs` | **整文件删除** |
| `src/app/state/todo.rs` | 删除 `active_chart` 字段、初始化与 `TodoChartKind` / `TodoStats` 再导出 |
| `src/app/state/todo/schedule.rs` | 删除 `TodoChartKind`、`TodoStats` 别名、`active_chart/next_chart/prev_chart/set_chart`、`trend_7_days`、`open_count`、`in_progress_count`、`priority_count`、`priority_distribution`；保留 `done_count`（`day_counts` 周感知带使用） |
| `src/app/state.rs` | 再导出去掉 `TodoChartKind` |
| `src/app/update/todo.rs` | 删除 `next_todo_chart` / `prev_todo_chart` |
| `src/app/update/todo/key.rs` | 删除 `'c' | 'C'` 分支 |
| `src/app/update/todo/tests.rs` | 删除 `chart_cycles_across_three_modes` |
| `src/event/mouse/press.rs` | 删除 `chart_switch_hit_at` 点击分支 |
| `src/ui/text/todo.rs` | 删除 `TODO_STATS_*`、`TODO_CHART_*`、`TODO_BAR_FILLED`、`TODO_BAR_PLANNED` |
| `src/ui/text/todo/tests.rs` | 删除统计文案/柱图断言；`TODO_EDIT_PLACEHOLDER` 断言迁到按钮/占位测试 |
| `src/ui/style/todo.rs` | 删除 `todo_bar_open` / `todo_bar_active` / `todo_bar_planned` / `todo_priority_bar`；保留 `todo_bar_done`（周感知带）并更新注释 |
| `src/db/todo/date.rs` | 删除仅 `trend_7_days` 使用的 `recent_dates` |
| `docs/standards/project-directory-structure.md` | 删除 `charts.rs` 描述；补 `db/todo/query.rs`、`service/todo.rs`、`assets/todo.*` |
| `docs/standards/tui-design-requirements.md` | 删除 Todo 统计块/切图器相关描述，改为「统计交给 Web Todo 页」；顺手把同段已过时的工具栏描述（文档仍写 `[clear done] [del] [edit]`，实际只有 `[+]` / `[sort]`）校正为当前实现 |
| `docs/standards/html-assets.md` | 补 `todo.css` / `todo.js`，更新 shell、脚本顺序与视图隔离说明 |

### 4.2 布局变化

删除前（统计块与列表平分主体区）：

```text
┌─ Prompt │ Todo ───────────────────────────┐
│ [+]                              [sort]   │
├─ 2026/11/14 ────────────────────── 10-16 ─┤
│ Mon  Tue  Wed  Thu  Fri  Sat  Sun         │  ← 周感知带
│ ❯ [ ] P1  write report                    │
│   [~] P2  review PR                       │
│   [x] P3  inbox                           │
├───────────────────────────────────────────┤  ← 统计分割线 + ‹ today · status 1/3 ›
│ total 3 · doing 1 · done 1 33%            │
│      ███          ███          ███        │
│ open 1        doing 1        done 1       │
└───────────────────────────────────────────┘
```

删除后（列表独占）：

```text
┌─ Prompt │ Todo ───────────────────────────┐
│ [+]                              [sort]   │
├─ 2026/11/14 ────────────────────── 10-16 ─┤
│ Mon  Tue  Wed  Thu  Fri  Sat  Sun         │
│ ❯ [ ] P1  write report                    │
│   [~] P2  review PR                       │
│   [x] P3  inbox                           │
│                                           │
│                                           │
└───────────────────────────────────────────┘
```

### 4.3 关键代码

`src/ui/right_panel/todo.rs`：

```rust
// 删除：pub mod charts; / pub use charts::{ChartSwitchHit, chart_switch_hit_at};
// 删除：MIN_STATS_ROWS、BAR_WIDTH
// 保留：MIN_SPLIT_BODY（周感知带仍按它降级）

/// 列表区矩形：主体区扣掉顶部周感知带，并让出最右 1 列滚动条槽。
pub fn list_rect(panel: Rect) -> Rect {
    let body = body_rect(panel);
    let strip = u16::from(week_strip_rect(panel).is_some());
    Rect {
        y: body.y.saturating_add(strip),
        width: body.width.saturating_sub(1),
        height: body.height.saturating_sub(strip),
        ..body
    }
}

pub(crate) fn render(frame: &mut Frame<'_>, area: Rect, state: &AppState) -> Option<(u16, u16)> {
    let header = super::geometry::header_rect(area)?;
    let focused = state.todo_focused();
    prompt::render_divider(area, header, frame.buffer_mut(), focused);
    let buf = frame.buffer_mut();
    render_toolbar(area, buf, state);
    schedule::render_divider_labels(area, buf, state);
    schedule::render_week_strip(area, buf, state);
    schedule::render_rollover_banner(area, buf, state);
    let cursor = item::render_rows(area, buf, state).filter(|_| focused);
    if let Some(bar) = scrollbar(area, state) {
        widgets::scrollbar::render_buffer(buf, &bar);
    }
    cursor
}
```

`src/app/state/todo/schedule.rs` 保留：

```rust
/// 查询指定日期的 `(计划总数, 已完成数)`。
pub fn day_counts(&self, date: &str) -> (usize, usize) { ... }

/// 完成项数量（周感知带按日统计用）。
pub fn done_count(&self) -> usize { ... }
```

### 4.4 交互影响

- `c` / `C` 从 Todo 键盘映射移除；按键落回未映射分支，被吞掉，不写终端。
- 点击原统计块区域仍命中 `focus_todo_from_click`（容器内空白），只聚焦、不改数据。
- 滚动条轨道现在覆盖列表区整高（周带以下），不再在统计分割线处截断。
- 面板高度不足时，只有周感知带按 `MIN_SPLIT_BODY = 7` 隐藏；列表始终占满剩余高度。

### 4.5 TUI 测试调整

- 删除 `src/ui/right_panel/todo/tests/charts.rs` 与 `mod charts;`。
- `src/app/update/todo/tests.rs` 删除 `chart_cycles_across_three_modes`。
- `src/ui/text/todo/tests.rs` 删除 `stats_labels_are_lowercase_ascii_words`，把 `TODO_EDIT_PLACEHOLDER` 非空断言并入 `todo_buttons_are_ascii_words` 或新增 `todo_placeholder_is_ascii`。
- 保留 `src/ui/right_panel/todo/tests/schedule.rs`、`grouping.rs`、列表/编辑/滚动条测试；几何断言均从 `list_rect`/`visible_rows` 派生，删除统计块后应自动适配。
- 新增一条回归：`list_rect` 高度 = `body_rect` 高度 − 周带高度，且 `scrollbar` 轨道底边 = 列表底边（原 `scrollbar_track_stops_above_stats` 的替代口径）。

---

## 5. Web 前端方案

### 5.1 路由与资源

**`assets/web.html`**

- `<head>` 增加：

```html
<link rel="stylesheet" href="/todo.css" media="not all" id="css-todo">
```

- 顶部 bar 追加（保持现有 Home / Settings / Usage 顺序，Todo 放最后）：

```html
<a class="web-tab" data-view="todo" href="#/todo">Todo</a>
```

- `#web-app` 末尾、`view-usage` 之后增加 `view-todo`（结构见 5.2）。
- 脚本列表末尾追加：

```html
<script src="/todo.js"></script>
```

**`assets/web.js`**

```js
const VIEWS = ['home', 'settings', 'usage', 'todo'];
const VIEW_TITLES = {
  home: 'lx-tui',
  settings: 'lx-tui · Settings',
  usage: 'lx-tui · Usage',
  todo: 'lx-tui · Todo'
};
const VIEW_CSS = { settings: 'css-settings', usage: 'css-usage', todo: 'css-todo' };

function viewFromLocation() {
  // ...
  if (path === '/settings') { return 'settings'; }
  if (path === '/usage') { return 'usage'; }
  if (path === '/todo') { return 'todo'; }
  return 'home';
}

// 视图样式表统一按 VIEW_CSS 切换，不再写死 settings/usage。
for (const [name, id] of Object.entries(VIEW_CSS)) {
  const link = document.getElementById(id);
  if (link) { link.media = name === view ? '' : 'not all'; }
}

if (view === 'todo') { initOnce('todo', window.initTodoView); }
```

`bootWeb()` 的旧路径规范化条件补 `/todo`。

**`src/settings/service/page.rs`**

```rust
"/todo.css" => Some((
    "text/css; charset=utf-8",
    include_str!("../../../assets/todo.css"),
)),
"/todo.js" => Some((
    "text/javascript; charset=utf-8",
    include_str!("../../../assets/todo.js"),
)),
```

**`src/settings/service/handlers.rs`**

```rust
("GET", "/") | ("GET", "/settings") | ("GET", "/usage") | ("GET", "/todo") => {
    Response::ok("text/html; charset=utf-8", page::html().to_string())
}
// ...
(_, path) if path.starts_with("/api/todo") => super::todo::route(request, state),
```

### 5.2 HTML 结构（关键 id）

```html
<section id="view-todo" class="web-view" aria-label="Todo">
  <div class="todo-shell">
    <section class="todo-controls" aria-label="Todo controls">
      <div class="range-group" role="group" aria-label="Date range">
        <button type="button" class="range-btn" data-range="today" aria-pressed="false">Today</button>
        <button type="button" class="range-btn active" data-range="7d" aria-pressed="true">7d</button>
        <button type="button" class="range-btn" data-range="30d" aria-pressed="false">30d</button>
        <button type="button" class="range-btn" data-range="all" aria-pressed="false">All</button>
        <button type="button" class="range-btn" data-range="custom" aria-pressed="false">Custom</button>
      </div>
      <div id="todo-custom-range" class="custom-range-wrap hidden">
        <input type="date" id="todo-custom-start" aria-label="Start date">
        <span class="range-sep">to</span>
        <input type="date" id="todo-custom-end" aria-label="End date">
      </div>
      <div class="filter-selects todo-filter-selects">
        <div class="filter-field"><div id="todo-filter-status" class="select-host"></div></div>
        <div class="filter-field"><div id="todo-filter-priority" class="select-host"></div></div>
        <div class="filter-field todo-search-field">
          <input id="todo-search" type="search" placeholder="Search todo text…" aria-label="Search todo text">
        </div>
      </div>
      <div class="header-actions">
        <span id="todo-status" class="action-status">
          <span id="todo-status-dot" class="status-dot clean"></span>
          <span id="todo-last-updated">Loading…</span>
        </span>
        <button id="todo-refresh" type="button" class="ghost">Refresh</button>
      </div>
    </section>
    <main class="todo-main">
      <div class="todo-content">
        <div id="todo-error" class="todo-error hidden" role="alert"></div>
        <section id="todo-summary-cards" class="summary-grid todo-summary-grid" aria-label="Todo summary"></section>
        <section class="todo-charts" aria-label="Todo charts">
          <div class="todo-charts-grid">
            <article class="chart-card chart-wide">
              <div class="chart-head">
                <div>
                  <div class="chart-title">Daily activity</div>
                  <div class="chart-subtitle" id="todo-trend-subtitle">Planned vs done</div>
                </div>
                <div class="chart-legend">
                  <span class="legend-item"><span class="legend-swatch swatch-planned"></span>Planned</span>
                  <span class="legend-item"><span class="legend-swatch swatch-done"></span>Done</span>
                </div>
              </div>
              <div id="todo-chart-trend" class="chart-body chart-body-wide"></div>
            </article>
            <article class="chart-card">
              <div class="chart-head">
                <div>
                  <div class="chart-title">Status distribution</div>
                  <div class="chart-subtitle">Open / in progress / done</div>
                </div>
              </div>
              <div id="todo-chart-status" class="chart-body"></div>
              <div id="todo-status-legend" class="donut-legend"></div>
            </article>
            <article class="chart-card">
              <div class="chart-head">
                <div>
                  <div class="chart-title">Priority distribution</div>
                  <div class="chart-subtitle">P0–P3 in selected range</div>
                </div>
              </div>
              <div id="todo-chart-priority" class="chart-body"></div>
            </article>
          </div>
        </section>
        <section class="todo-table-section" aria-label="Todo tables">
          <div class="provider-tabs todo-tabs" role="tablist" aria-label="Todo tables">
            <button type="button" class="provider-tab active" data-tab="todos" id="tab-btn-todos" role="tab" aria-selected="true">Todos (0)</button>
            <button type="button" class="provider-tab" data-tab="daily" id="tab-btn-daily" role="tab" aria-selected="false">Daily (0)</button>
            <button type="button" class="provider-tab" data-tab="groups" id="tab-btn-groups" role="tab" aria-selected="false">Groups (0)</button>
          </div>
          <div id="todo-table-container" class="table-wrap" role="tabpanel"></div>
          <div id="todo-pagination" class="pagination-bar">
            <span id="todo-pagination-info" class="pagination-info">Page 1 of 1 · 0 todos</span>
            <div class="pagination-actions">
              <button type="button" id="todo-page-prev" class="ghost" disabled>Prev</button>
              <button type="button" id="todo-page-next" class="ghost" disabled>Next</button>
            </div>
          </div>
        </section>
      </div>
    </main>
  </div>
</section>
```

> 所有 `getElementById('静态 id')` 必须在上表出现；`tests/web_assets.rs::browser_scripts_reference_existing_ids` 会校验。

### 5.3 交互流程

状态机（`assets/todo.js`）：

```js
const todoState = {
  rangePreset: '7d',
  customStart: '',
  customEnd: '',
  status: '',
  priority: '',
  search: '',
  activeTab: 'todos',
  page: 1,
  pageSize: 50,
  loading: false,
  requestSeq: 0,
  overview: null,
  itemPage: null
};
const todoSelects = {};
let todoSearchTimer = null;
```

- 初始化：默认自定义日期 = `today - 6` 到 `today`；绑定 range 按钮、日期输入、两个 `createSelect`、搜索框（250ms 防抖）、Refresh、3 个 tab、Prev/Next；首次 `loadTodoDashboard()`。
- `loadTodoDashboard()`：`requestSeq` 竞态保护；并行请求 `overview` + `items`；如果 `page > totalPages`，钳到最后一页并重载；渲染卡片与表格；状态点 `clean` + `Updated HH:MM:SS`。
- 翻页 `loadTodoItemsPage(page)`：只请求 `/api/todo/items`，不重算 overview。
- 筛选变化：重置 `page = 1` 后 `loadTodoDashboard()`；搜索输入防抖 250ms。
- tab 切换：只用已缓存数据重绘，不发请求。
- 错误：`todo-error` 横幅 + 状态点 `error`；仅当 `#view-todo` 处于 active 时弹 toast（对齐 `isUsageViewActive`）。
- 无自动刷新（D3 之外的低风险取舍，刷新按钮足够）。

**关键命名约束**：`todo.js` 与前面的脚本共享全局作用域，必须避开 `usage.js` 已占用的顶层名：`usageState`、`selects`、`apiGet`、`buildTable`、`appendEmptyRow`、`renderTableSection`、`renderPaginationBar`、`resolveTimeRange`、`loadUsageDashboard` 等。本方案统一加 `todo` 前缀或改用具名局部函数（如 `todoBuildTable`、`renderTodoTableSection`、`resolveTodoRange`）。`apiGet`、`domEl`、`showToast`、`createSelect` 复用已有共享函数。

**必须同步修 `usage.js` 的全局选择器**：当前 `document.querySelectorAll('.range-btn')` 会绑定到 Todo 页的 range 按钮，点击 Todo 的 `Today` 会触发 Usage 的筛选与请求。改为 `document.querySelectorAll('#view-usage .range-btn')`（两处：初始化与点击后同步 active）。

### 5.4 三张表

| Tab | 列 | 数据来源 | 分页 |
| --- | --- | --- | --- |
| `Todos` | Date、Status、Priority、Text、Group、Completed | `/api/todo/items` | 是，50/页 |
| `Daily` | Date、Planned、Done、Completion、Remaining | `overview.daily` | 否 |
| `Groups` | Date、Group、Items、Top Priority、Open、Doing、Done | `overview.groups` | 否 |

- `Status` 徽标：`todo` / `in progress` / `done`，CSS 类 `todo-badge todo-status-{name}`。
- `Priority` 徽标：`P0..P3`，CSS 类 `todo-badge todo-priority-{p0..p3}`；P0 红、P1 黄、P2 正文、P3 压暗。
- `Text` 用 `cell-truncate`（`max-width: 420px`）+ `title` 放全文；不展开行。
- `Group` 显示组内首条文本（服务端 `groupLabel`），`title` 放原始 `groupId`；无组显示 `--`。
- `Completed` 显示 `completedDate` 或 `--`。
- 空态文案：`No todos in the selected range.`
- Tab 标签带计数：`Todos (total)`、`Daily (daily.length)`、`Groups (groups.length)`。

### 5.5 关键前端代码

```js
function resolveTodoRange(preset, customStart, customEnd) {
  const today = todoDateKey(new Date());
  if (preset === 'all') { return {startDate: undefined, endDate: undefined}; }
  if (preset === 'today') { return {startDate: today, endDate: today}; }
  if (preset === '7d' || preset === '30d') {
    const days = preset === '7d' ? 7 : 30;
    return {startDate: todoShiftDate(today, -(days - 1)), endDate: today};
  }
  let start = customStart || todoShiftDate(today, -6);
  let end = customEnd || today;
  if (start > end) { const tmp = start; start = end; end = tmp; }
  return {startDate: start, endDate: end};
}

async function loadTodoDashboard() {
  const reqId = ++todoState.requestSeq;
  todoState.loading = true;
  setTodoStatus('loading', 'Refreshing…');
  const range = resolveTodoRange(todoState.rangePreset, todoState.customStart, todoState.customEnd);
  const base = todoFilterParams(range);
  try {
    const [overview, itemPage] = await Promise.all([
      apiGet('/api/todo/overview', base),
      apiGet('/api/todo/items', Object.assign({}, base, {
        page: todoState.page,
        pageSize: todoState.pageSize
      }))
    ]);
    if (reqId !== todoState.requestSeq) { return; }
    todoState.overview = overview || {};
    todoState.itemPage = itemPage || {rows: [], total: 0, page: 1, pageSize: todoState.pageSize};
    const totalPages = Math.max(1, Math.ceil(Number(todoState.itemPage.total || 0) / todoState.pageSize));
    if (todoState.page > totalPages) {
      todoState.page = totalPages;
      return loadTodoDashboard();
    }
    renderTodoSummary(todoState.overview.summary || null);
    renderTodoTableSection();
    setTodoError('');
    setTodoStatus('clean', 'Updated ' + new Date().toLocaleTimeString('en-US', {hour12: false}));
  } catch (error) {
    if (reqId !== todoState.requestSeq) { return; }
    setTodoStatus('error', 'Load failed');
    setTodoError('Failed to load todos: ' + error.message);
    if (isTodoViewActive()) { showToast(error.message, 'error'); }
  } finally {
    if (reqId === todoState.requestSeq) { todoState.loading = false; }
  }
}

function todoFilterParams(range) {
  return {
    startDate: range.startDate,
    endDate: range.endDate,
    status: todoState.status || undefined,
    priority: todoState.priority || undefined,
    q: todoState.search.trim() || undefined
  };
}
```

### 5.6 CSS 方案

新建 `assets/todo.css`，**独立一份**，从 `usage.css` 复制本次用到的部分并改名：

- 外壳：`.todo-shell` / `.todo-controls` / `.todo-main` / `.todo-content`（与 `.usage-*` 同构）。
- 控件：`.range-group` / `.range-btn` / `.custom-range-wrap` / `.range-sep` / `.filter-selects` / `.filter-field` / `.header-actions` / `.action-status` / `.status-dot` / `.select-*` / `button.ghost` / `input`。
- 卡片：`.summary-grid` / `.summary-card` / `.summary-label` / `.summary-value` / `.summary-sub`；`.todo-summary-grid { grid-template-columns: repeat(3, minmax(0, 1fr)); }`。
- 表格：`.todo-table-section`（`.card` 等价）/ `.todo-tabs` / `.provider-tab` / `.table-wrap` / `.todo-table`（列宽、sticky 表头、`th.num` / `td.num`、`cell-truncate`、`table-empty`）。
- 徽标：`.todo-badge` + `.todo-status-*` / `.todo-priority-*`。
- 分页：`.pagination-bar` / `.pagination-actions`。
- 响应式：`@media (max-width: 1100px)` 卡片 2 列、筛选 2 列；`@media (max-width: 640px)` 卡片 1 列、控件内边距收紧。
- 文件顶部注释：`/* 与 usage.css 的控件/卡片/表格样式刻意重复：视图样式按 media 隔离，任一视图改样式不得连累另一视图。 */`

> 不复用 `usage.css`，不把控件抽到 `web.css`；这是 D6 明确决策。

### 5.7 图表（2026-11-15 追加）

- **位置**：`#todo-summary-cards` 与 `.todo-table-section` 之间常驻，不随 tab 切换隐藏。
- **三张图**：
  1. `Daily activity`（宽卡）：Planned vs Done 分组竖柱；hover 显示 Planned / Done / Remaining；空范围显示 `No todos in the selected range.`。
  2. `Status distribution`：Open / In Progress / Done 三片环图，中心显示完成率；右侧/下方 legend 带计数与占比。
  3. `Priority distribution`：P0–P3 横条，显示计数与占比；零值也保留轨道。
- **数据来源**：`overview.summary`、`overview.priority`（新增）、`overview.daily`。三图都随日期范围 / Status / Priority / 搜索筛选变化；`Overdue` 卡片仍按 D5 忽略日期范围。
- **趋势分桶**：`daily` 只含有数据的日期，前端按跨度补零并分桶——≤62 天按日、≤400 天按周（周一对齐）、更久按月；`All` 以 `daily` 首尾日期为边界；副标题动态显示 `daily` / `weekly` / `monthly`。日期一律按本地 `YYYY-MM-DD` 手工拆段构造 `Date`，避免 UTC 偏移。
- **代码组织**：`assets/todo_charts.js`，对外只暴露 `renderTodoCharts(overview, range)`；SVG 原语、tooltip、空态、分桶都在文件内，顶层符号统一 `todo` 前缀；`assets/todo.js` 在 `loadTodoDashboard` 完成 summary 渲染后调用。`assets/todo.css` 追加图表样式与颜色令牌，不依赖 `usage.css` 是否处于激活状态。
- **视觉**：复用 Usage 的暗色卡片语言（`chart-card` / `chart-head` / `chart-title` / `chart-subtitle` / `legend-swatch` / `chart-svg` / `chart-empty` / 固定定位 tooltip），零外部依赖、零 `innerHTML`；≤1100px 图表单列，≤640px 收紧内边距。

> 这是 D9/D10；原 D3「不画图表」已作废。

### 5.8 加载态与尺寸稳定（2026-11-15 追加）

- **全局进度条**：`#page-loading`（`assets/web.css` 的 `.page-loading`）固定在顶部 bar 以下 2px，覆盖内容区顶部但不遮挡内容；懒加载视图的 `init<View>View` 返回首次数据加载 Promise，`web.js::initOnce` 在 Promise settle 后淡出；最短展示 180ms，避免快请求闪一下；底层细线 + 循环亮带保证任何时刻都有可见反馈。切页时立刻收起旧进度条并按新视图重新判断。
- **局部骨架**：Todo 首次加载绘制 6 张卡片、3 个图表块（带横向网格线的图表骨架）、状态 legend 与表格骨架；Usage 绘制 8 张卡片、5 个图表块与表格骨架。骨架复用 `.summary-card` / `.chart-body` / 表格行高，保证数据到达前后高度一致；扫光错峰、`prefers-reduced-motion` 下关闭动效；骨架切换真实数据时只淡入一次。
- **刷新不骨架化**：只有首次进入（容器为空）才画骨架；点击 Refresh / 改筛选保留旧数据并更新状态点，避免闪屏。
- **状态平滑**：已有数据时刷新只显示 `.status-spinner`，不再把 `Updated HH:MM:SS` 改成 `Refreshing…`；数据返回后一次性更新时间戳。spinner 用 opacity 过渡，快速请求只留下极轻微的呼吸感。
- **状态宽度**：Usage/Todo 与 Settings 的 `.action-status` 不预留最小宽度，宽度跟随文案；配合 `font-variant-numeric: tabular-nums` 与 `nowrap`，状态区不再出现固定空白，容器右对齐让右侧按钮位置保持不变。
- **失败兜底**：初始化 Promise 失败时关闭进度条并保留错误 banner/toast；`window.__webInit[key]` 标为 `failed`，下次切回该视图可重试。

### 5.9 Home Todo 摘要卡（2026-11-15 追加）

- **布局**：Home hero 下改为 `1.6fr / 1fr` 两列。左侧宽卡展示 Todo；右侧竖排 Settings / Usage 入口。窄屏（≤720px）单列，入口卡改为横排（≤640px 再竖排）。hero 用 auto 外边距居中，内容超高时退化为顶部对齐，不裁切。
- **只读数据**：`home.js` 请求 `GET /api/todo/home`，不提供新增/勾选/删除。首次加载在卡内显示 `Loading…`，失败显示 `Todo unavailable.`；重新回到 Home 会刷新，已有数据时失败保留旧列表并提示 `Could not refresh todos.`。
- **卡片结构**：`Today` = 今日未完成（todo + in_progress）；`In progress` = 今日及更早仍在进行中的条目；`Overdue` = 今天之前仍未完成的条目；三项互不排斥。列表合并今日 + 逾期未完成，按优先级（P0→P3）、`entry_date` 升序、`position` 升序取前 5 条；`done` 与未来日期不出现。
- **条目点击**：跳转 `#/todo`，调用 `todo.js::todoFocusFromHome(text)`：预填搜索框、范围按钮切到 `All`，已初始化时立即重载；未初始化时由 `initOnce` 带着状态完成首次加载。
- **视觉**：Default 主题沿用方角与 monospace；Glass 主题复用卡片玻璃材质，统计块 12px 圆角、条目行 10px 圆角，hover 只变背景；`prefers-reduced-motion` 下关闭条目与卡片过渡。

### 5.10 复审修复（2026-11-15）

- **Groups 列序**：表头为 Date / Group / Items / Top Priority / Open / Doing / Done，行数据按同序追加；此前 `priorityTd` 插在 `items` 之前，导致 Items 显示优先级徽标、Top Priority 显示条目数（P0）。
- **滚动位置**：筛选、搜索、翻页成功后 `.todo-main` 回到顶部；手动 Refresh 保持当前滚动位置。
- **图表 tooltip**：切离 Todo 视图时调用 `todoChartHideTooltip()`，避免 `todo.css` 被挂起后 tooltip 以未样式化块残留在 body。
- **Home 失败重试**：首次加载失败时卡内提示可点击/回车重试；已有数据刷新失败保留旧列表。
- **读取一致性**：`query_todo_home` 的计数与列表在同一 deferred read transaction 内读取，避免 TUI 并发写入时数字与条目对不上。
- **空态与跳转**：Groups 空态使用 `No grouped todos in the selected range.`；Home 跳转在请求返回前同步 Todos tab 与分页可见性。
- **测试**：补日期倒置、Home 5 条上限、HTTP `pageSize` 钳制、`/home.js` 与 `/todo_charts.js` 外部资源断言、`c` 键无副作用。

---

## 6. 服务端 API 方案

### 6.1 路由

| 方法 | 路径 | 说明 |
| --- | --- | --- |
| GET | `/api/todo/overview` | 汇总卡、Daily、Groups 一次返回 |
| GET | `/api/todo/items` | 分页 Todo 明细 |
| GET | `/api/todo/home` | Home 摘要：Today / In progress / Overdue 计数 + 今日与逾期前 5 条 |

新增 `src/settings/service/todo.rs`，在 `service.rs` 声明 `mod todo;`，在 `handlers.rs` 加前缀分派。非 GET → 405；未知 `/api/todo/*` → 404；参数非法 → 422；DB 错误 → 500。

### 6.2 查询参数

| 参数 | 类型 | 缺省 | 校验 |
| --- | --- | --- | --- |
| `startDate` | `YYYY-MM-DD` | 无下界 | 严格日历校验；与 `endDate` 倒置时交换 |
| `endDate` | `YYYY-MM-DD` | 无上界 | 同上 |
| `status` | `todo` / `in_progress` / `done` | 全部 | 非法值 422 |
| `priority` | `P0`..`P3` | 全部 | 非法值 422 |
| `q` | 文本子串 | 全部 | `trim`，超过 200 字符截断 |
| `page` | `>= 1` | 1 | 0 或非数字 422 |
| `pageSize` | `1..=200` | 50 | 越界钳制 |

严格日期校验：`parse_date(y,m,d)` 后经 `days_from_civil` → `civil_from_days` 往返，拒绝 `2026-02-31`。

### 6.3 响应 JSON

`GET /api/todo/overview`：

```json
{
  "summary": {
    "total": 20,
    "open": 5,
    "inProgress": 3,
    "done": 12,
    "overdue": 3
  },
  "priority": {"p0": 3, "p1": 9, "p2": 6, "p3": 2},
  "daily": [
    {"date": "2026-11-14", "planned": 4, "done": 1, "remaining": 3}
  ],
  "groups": [
    {
      "groupId": "tgrp-...",
      "date": "2026-11-14",
      "label": "login api",
      "items": 3,
      "open": 1,
      "inProgress": 1,
      "done": 1,
      "topPriority": "P1"
    }
  ]
}
```

`GET /api/todo/items`：

```json
{
  "rows": [
    {
      "id": "todo-...",
      "date": "2026-11-14",
      "text": "write report",
      "status": "in_progress",
      "priority": "P1",
      "groupId": "tgrp-...",
      "groupLabel": "login api",
      "completedDate": null,
      "position": 0
    }
  ],
  "total": 20,
  "page": 1,
  "pageSize": 50
}
```

字段名统一 camelCase；`status` / `priority` 用库内原值（`todo` / `in_progress` / `done`、`P0`..`P3`）。`groupId` / `groupLabel` / `completedDate` 缺失为 `null`。`priority` 为按当前筛选（含日期范围）统计的 P0–P3 计数，顺序固定，空表全零。

`GET /api/todo/home`：

```json
{
  "date": "2026-11-15",
  "summary": {"todayOpen": 4, "inProgress": 1, "overdue": 1},
  "items": [
    {
      "id": "todo-...",
      "date": "2026-11-14",
      "text": "write report",
      "status": "todo",
      "priority": "P1",
      "groupId": null,
      "groupLabel": null,
      "completedDate": null,
      "position": 0
    }
  ]
}
```

`todayOpen` 只统计今日未完成；`inProgress` 与 `overdue` 覆盖今天及更早；`items` 固定最多 5 条，按优先级、日期升序、位置排序，不返回 done/future。

### 6.4 错误语义

| 场景 | 状态码 | body |
| --- | --- | --- |
| 非 GET | 405 | `{"error":"method not allowed"}` |
| 未知路径 | 404 | `{"error":"not found"}` |
| 日期/status/priority 非法 | 422 | `{"error":"invalid query parameter '<name>'"}` |
| `page=0` | 422 | `{"error":"query parameter 'page' must be >= 1"}` |
| `page` 非数字 / `pageSize` 非数字 | 422 | `{"error":"invalid query parameter '<name>'"}` |
| DB 打开/查询失败 | 500 | `{"error":"<message>"}` |

### 6.5 关键服务端代码

```rust
// src/settings/service/todo.rs
pub(super) fn route(request: &Request, state: &ServiceState) -> Response {
    let known = matches!(
        request.path.as_str(),
        "/api/todo/overview" | "/api/todo/items" | "/api/todo/home"
    );
    if !known { return Response::error(404, "not found"); }
    if request.method != "GET" { return Response::error(405, "method not allowed"); }
    let params = match parse_params(&request.query) {
        Ok(params) => params,
        Err(message) => return Response::error(422, &message),
    };
    let db = match Database::open(&state.db_path()) {
        Ok(db) => db,
        Err(error) => return Response::error(500, &error.to_string()),
    };
    match request.path.as_str() {
        "/api/todo/overview" => overview_json(&db, &params.filter),
        "/api/todo/home" => home_json(&db),
        _ => items_json(&db, &params),
    }
}
```

```rust
fn parse_params(query: &HashMap<String, String>) -> Result<ParsedTodoParams, String> {
    let mut start_date = parse_date_param(query, "startDate")?;
    let mut end_date = parse_date_param(query, "endDate")?;
    if let (Some(start), Some(end)) = (&start_date, &end_date)
        && start > end
    {
        std::mem::swap(&mut start_date, &mut end_date);
    }
    let status = parse_status(query)?;
    let priority = parse_priority(query)?;
    let search = query
        .get("q")
        .map(|raw| raw.trim().chars().take(200).collect::<String>())
        .filter(|value| !value.is_empty());
    let page = parse_usize(query, "page")?.unwrap_or(1);
    let page_size = parse_usize(query, "pageSize")?.unwrap_or(50).clamp(1, 200);
    Ok(ParsedTodoParams {
        filter: TodoFilter { start_date, end_date, status, priority, search },
        page,
        page_size,
    })
}
```

---

## 7. DB 查询层

### 7.1 新增文件与结构

`src/db/todo.rs` 增加：

```rust
pub mod query;
pub use query::{TodoDailyRow, TodoFilter, TodoGroupRow, TodoItemRow, TodoPage, TodoSummary};
```

`src/db/todo/query.rs`：

```rust
//! Todo 看板只读查询：筛选、分页、汇总、按日聚合与聚合组统计。

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TodoFilter {
    pub start_date: Option<String>,
    pub end_date: Option<String>,
    pub status: Option<TodoStatus>,
    pub priority: Option<TodoPriority>,
    pub search: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TodoItemRow {
    pub record: TodoRecord,
    pub group_label: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TodoPage {
    pub rows: Vec<TodoItemRow>,
    pub total: usize,
    pub page: usize,
    pub page_size: usize,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct TodoSummary {
    pub total: usize,
    pub open: usize,
    pub in_progress: usize,
    pub done: usize,
    pub overdue: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TodoDailyRow {
    pub date: String,
    pub planned: usize,
    pub done: usize,
    pub remaining: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TodoGroupRow {
    pub group_id: String,
    pub date: String,
    pub label: String,
    pub items: usize,
    pub open: usize,
    pub in_progress: usize,
    pub done: usize,
    pub top_priority: TodoPriority,
}
```

> JSON 序列化在 service 层用 `serde_json::json!` 组装，DB 结构不引入 serde，保持持久层纯粹。
>
> 追加（2026-11-15）：`Database::query_todo_priority_counts(&filter) -> Result<[usize; 4]>`，固定返回 `P0..P3` 计数；SQL 为 `SELECT priority, COUNT(*) FROM todo {FILTER_WHERE} GROUP BY priority`，空表全零，未知优先级忽略。

### 7.2 SQL 口径

固定 WHERE（`?1..?5` = start/end/status/priority/search）：

```sql
WHERE (?1 IS NULL OR entry_date >= ?1)
  AND (?2 IS NULL OR entry_date <= ?2)
  AND (?3 IS NULL OR status = ?3)
  AND (?4 IS NULL OR priority = ?4)
  AND (?5 IS NULL OR instr(lower(text), lower(?5)) > 0)
```

**Todos 分页**：

```sql
SELECT t.id, t.entry_date, t.text, t.status, t.priority, t.completed_date,
       t.position, t.group_id,
       (SELECT t2.text FROM todo t2
         WHERE t2.group_id = t.group_id
         ORDER BY t2.entry_date ASC, t2.position ASC, t2.id ASC
         LIMIT 1) AS group_label
FROM todo t
WHERE (...)
ORDER BY t.entry_date DESC, t.position ASC, t.id ASC
LIMIT ?6 OFFSET ?7
```

`total` 用同一 WHERE 的 `COUNT(*)`。

**Summary**：

```sql
SELECT COUNT(*),
       SUM(status = 'todo'),
       SUM(status = 'in_progress'),
       SUM(status = 'done')
FROM todo
WHERE (...)
```

`SUM` 用 `COALESCE(..., 0)`；空结果时 total = 0。

**Overdue**（忽略日期范围，遵守 status/priority/search）：

```sql
SELECT COUNT(*)
FROM todo
WHERE entry_date < ?1
  AND status != 'done'
  AND (?2 IS NULL OR status = ?2)
  AND (?3 IS NULL OR priority = ?3)
  AND (?4 IS NULL OR instr(lower(text), lower(?4)) > 0)
```

**Daily**：

```sql
SELECT entry_date, COUNT(*), SUM(CASE WHEN status = 'done' THEN 1 ELSE 0 END)
FROM todo
WHERE (...)
GROUP BY entry_date
ORDER BY entry_date DESC
```

**Groups**：

```sql
SELECT group_id,
       MIN(entry_date) AS group_date,
       COUNT(*) AS items,
       SUM(status = 'todo') AS open,
       SUM(status = 'in_progress') AS in_progress,
       SUM(status = 'done') AS done,
       MIN(CASE priority WHEN 'P0' THEN 0 WHEN 'P2' THEN 2 WHEN 'P3' THEN 3 ELSE 1 END) AS rank,
       MIN(position) AS first_position,
       (SELECT t2.text FROM todo t2
         WHERE t2.group_id = todo.group_id
         ORDER BY t2.entry_date ASC, t2.position ASC, t2.id ASC
         LIMIT 1) AS label
FROM todo
WHERE (...) AND group_id IS NOT NULL
GROUP BY group_id
ORDER BY group_date DESC, first_position ASC
```

`rank` → `TodoPriority::ALL[rank]`（`P0,P1,P2,P3` 顺序与 `ALL` 一致）。

### 7.3 关键实现

```rust
impl Database {
    /// 按筛选分页读取 Todo 明细；组标签取组内最早条目的文本。
    pub fn query_todos_page(
        &self,
        filter: &TodoFilter,
        page: usize,
        page_size: usize,
    ) -> Result<TodoPage> {
        let total: i64 = self.conn.query_row(
            &format!("SELECT COUNT(*) FROM todo {}", where_clause()),
            params![
                filter.start_date.as_deref(),
                filter.end_date.as_deref(),
                filter.status.map(TodoStatus::as_str),
                filter.priority.map(TodoPriority::as_str),
                filter.search.as_deref(),
            ],
            |row| row.get(0),
        )?;
        let offset = page.saturating_sub(1).saturating_mul(page_size);
        let mut statement = self.conn.prepare(&format!(
            "SELECT t.id, t.entry_date, t.text, t.status, t.priority, t.completed_date,
                    t.position, t.group_id,
                    (SELECT t2.text FROM todo t2
                      WHERE t2.group_id = t.group_id
                      ORDER BY t2.entry_date ASC, t2.position ASC, t2.id ASC
                      LIMIT 1)
             FROM todo t {} ORDER BY t.entry_date DESC, t.position ASC, t.id ASC
             LIMIT ?6 OFFSET ?7",
            where_clause()
        ))?;
        let rows = statement.query_map(
            params![
                filter.start_date.as_deref(),
                filter.end_date.as_deref(),
                filter.status.map(TodoStatus::as_str),
                filter.priority.map(TodoPriority::as_str),
                filter.search.as_deref(),
                page_size as i64,
                offset as i64,
            ],
            |row| {
                Ok(TodoItemRow {
                    record: map_todo_row(row)?,
                    group_label: row.get(8)?,
                })
            },
        )?;
        // rows.collect -> TodoPage { rows, total, page, page_size }
    }
}
```

> `where_clause()` 返回固定 SQL 片段（列名不带表别名）；主查询只多一个 `todo t` 别名，SQLite 仍能解析未限定列名。`params!` 直接传 5/7 个参数，不引入 `params_from_iter`。

### 7.4 排序与口径

- Todos：`entry_date DESC` → 新日期在前；同日按 `position ASC` 保持 TUI 顺序。
- Daily：`entry_date DESC`，与 Todos 一致。
- Groups：`group_date DESC, first_position ASC`。
- `Overdue` 不受 `startDate`/`endDate` 影响；受 `status`/`priority`/`q` 影响。
- `Completion` 在前端算 `done / total * 100`，`total = 0` 显示 `--`。
- 搜索只匹配 `text`，大小写不敏感仅对 ASCII 生效（SQLite `lower()` 限制），中文子串正常匹配。

---

## 8. 测试计划

### 8.1 DB 查询单测（`src/db/todo/query/tests.rs`）

在临时目录打开 `Database`，用 `TodoRecord::new_on` + `save_todo` 造数据：

- 范围筛选：`entry_date` 闭区间；`start > end` 由 service 层交换，DB 层不交换。
- status / priority / q 组合筛选。
- `q` 大小写不敏感（ASCII）与中文子串。
- 分页：`page=1/2`、`total`、`pageSize` 边界。
- 汇总：total/open/inProgress/done；空表全 0。
- Overdue：忽略日期范围、遵守 status/priority/q。
- Daily：planned/done/remaining，空表返回空 Vec。
- Groups：需要至少 2 条同组记录（`save_todo` 会把单成员组清空）；断言 label 取组内首条文本、topPriority 取最高优先级、计数正确；独立条目不出现在 Groups。
- 排序：Todos `date DESC, position ASC`；Daily `date DESC`。

### 8.2 HTTP 集成测试（`src/settings/service/todo/tests.rs`）

复用 `tests_helpers::{body, get, request, status}` 与 `start_in`：

- `GET /api/todo/overview?token=...` 返回 summary/daily/groups 三段；字段名与 camelCase 断言。
- `GET /api/todo/items?...&page=2&pageSize=2` 返回第二页、total 正确。
- 组合筛选：`startDate`/`endDate`/`status`/`priority`/`q`。
- `Overdue` 在 `startDate=today` 下仍能统计历史未完成。
- 非法参数：`startDate=2026-13-01`、`status=nope`、`priority=P9`、`page=0` → 422。
- 非 GET → 405；未知 `/api/todo/nope` → 404；无 token → 403。
- `GET /todo` 返回统一 shell；`GET /todo.css` / `/todo.js` 返回正确 MIME。
- `todo.js` 不含 `innerHTML`、不含 `function createSelect` / `function showToast`、包含 `function initTodoView`。

### 8.3 浏览器资源契约（`tests/web_assets.rs`）

- `SCRIPTS` 追加 `todo.js`（顺序在 `usage.js` 之后）。
- `browser_scripts_reference_existing_ids` 自动覆盖 `todo.js` 的所有 `getElementById('...')`。
- `node --check` 单文件与拼接脚本必须通过（防 `const`/`function` 重名）。

### 8.4 TUI 回归

- `cargo test --lib ui::right_panel::todo`：列表、周带、顺延、聚合组、编辑、滚动条、几何。
- 新增 `list_rect_uses_full_body_below_week_strip`：无统计块后列表高度 = 主体区 − 周带。
- `cargo test --lib app::update::todo`：删除 chart 测试后其余按键/鼠标行为不变。
- `cargo test --lib ui::text::todo`：文案断言更新后通过。

### 8.5 手工验收

1. `cargo run`：
   - Todo 面板底部不再有统计块；列表可滚动到底；`c` 无反应；点原统计区只聚焦。
   - 周感知带、顺延横幅、`[+]`、`[sort]`、双击编辑、`Space` 循环状态正常。
2. 浏览器打开 `[WEB]` → 顶部 `Todo`：
   - 默认 `7d`，三张表可切换；筛选/搜索/翻页/刷新正常。
   - `Today` 下 Overdue 仍显示历史欠账；`All` 下数据最多。
   - 窄屏（< 1100px / < 640px）卡片与筛选换行不溢出。
3. 无 token 直接访问 `/api/todo/overview` → 403。

---

## 9. 影响范围与风险

### 9.1 文件清单

**新增**

- `assets/todo.css`
- `assets/todo.js`
- `src/db/todo/query.rs`（+ `src/db/todo/query/tests.rs`）
- `src/settings/service/todo.rs`（+ `src/settings/service/todo/tests.rs`）
- `docs/feature/web-todo-tab/{design,task}.md`

**修改**

- `assets/web.html`、`assets/web.js`、`assets/usage.js`（range 选择器作用域）
- `src/settings/service.rs`、`src/settings/service/handlers.rs`、`src/settings/service/page.rs`
- `src/settings/service/tests.rs`
- `src/db/todo.rs`、`src/db/todo/date.rs`
- `src/ui/right_panel/todo.rs`、`src/ui/right_panel/todo/tests.rs`
- `src/app/state.rs`、`src/app/state/todo.rs`、`src/app/state/todo/schedule.rs`
- `src/app/update/todo.rs`、`src/app/update/todo/key.rs`、`src/app/update/todo/tests.rs`
- `src/event/mouse/press.rs`
- `src/ui/text/todo.rs`、`src/ui/text/todo/tests.rs`
- `src/ui/style/todo.rs`
- `tests/web_assets.rs`
- `docs/standards/project-directory-structure.md`、`docs/standards/tui-design-requirements.md`、`docs/standards/html-assets.md`

**删除**

- `src/ui/right_panel/todo/charts.rs`
- `src/ui/right_panel/todo/tests/charts.rs`

### 9.2 风险

| 风险 | 影响 | 处置 |
| --- | --- | --- |
| TUI 内存态与 Web 读库不同步 | 浏览器看到的不是 TUI 未提交的编辑 | 只读 + 文档说明；TUI 编辑仍按现有 800ms/焦点切换落库 |
| `All` 范围数据量大 | Daily/Groups 全量渲染变慢 | 人类级 todo 量可控；后续需要再加分页/上限，本次不做 |
| CSS 重复 | 两处维护 | D6 明确取舍；注释说明，视图隔离优先 |
| `usage.js` 全局 `.range-btn` 选择器 | 点击 Todo 筛选会误触发 Usage | 本次一并改为 `#view-usage .range-btn` |
| 前端全局作用域重名 | `node --check` 直接失败 | 统一 `todo` 前缀；复用 `apiGet`/`domEl`/`showToast`/`createSelect` |
| 删除统计块后几何测试漂移 | 列表高度变化 | 所有断言从 `list_rect`/`visible_rows` 派生；新增全高回归 |
| 日期严格校验 | `2026-02-31` 被拒 | 用 `days_from_civil` 往返校验 |
| token 泄露 | 无 | 页面与 API 仍逐请求校验；静态资源不含数据 |

### 9.3 回滚

- 纯新增 + 删除，无 schema 迁移；`git revert` 单个提交即可回滚。
- 不触碰 `SCHEMA_VERSION`（保持 29），不存在库升级不可逆问题。

---

## 10. 不做的事

- 不做 Web 端新增/勾选/删除/拖拽排序（只读）。
- 不做 SVG 图表、自动刷新、Home 页新增 Todo 卡片。
- 不改 `todo` 表结构、不新增迁移。
- 不改 TUI 列表、周感知带、顺延横幅、编辑、聚合组、排序逻辑。
- 不改 lx agent 的 `update_plan` / 工具卡片 todo。
- 不引入 Tailwind、npm、外部字体或图片。

---

## 11. 验证命令

```bash
cargo fmt
cargo clippy --all-targets -- -D warnings
cargo test --lib db::todo::query
cargo test --lib settings::service::todo
cargo test --test web_assets
cargo test --lib ui::right_panel::todo
cargo test --lib app::update::todo
cargo test
```

手工：

```bash
cargo run
# 打开 [WEB] → Todo，检查默认 7d、筛选、三张表、翻页、空态、窄屏
```

---

## 12. 待用户审核项

- D8 默认 `7d` / 默认 `Todos` / 页大小 50。
- `Overdue` 卡片固定显示「全库历史未完成」，在 `Today` 下也不归零。
- `Groups` 的组标签取「组内首条文本」，不显示原始 `tgrp-...`。
- Todo tab 放在顶部 bar 最右侧（Home / Settings / Usage / Todo）。
- Home 页新增 Todo 摘要卡（D13）：今日/逾期计数与条目，点击条目跳转 Todo 并按文本搜索。

审核通过后按 [task.md](./task.md) 执行。
