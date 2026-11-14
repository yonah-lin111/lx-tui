# Web 控制台 Todo 看板 + TUI 字符统计块移除：审查记录

> 日期：2026-11-14
> 分支：`web-todo-tab`（worktree `.worktrees/web-todo-tab`）
> 设计：[design.md](./design.md)；执行清单：[task.md](./task.md)

## 验证结果

| 命令 | 结果 |
| --- | --- |
| `cargo test --lib db::todo` | 38 passed |
| `cargo test --lib settings::service::todo` | 5 passed |
| `cargo test --lib settings::service::tests::page_requires` | 1 passed |
| `cargo test --test web_assets` | 2 passed |
| `cargo test --lib ui::right_panel::todo` | 38 passed |
| `cargo test --lib app::update::todo` | 53 passed |
| `cargo test --lib event::tests::todo` | 22 passed |
| `cargo clippy --all-targets -- -D warnings` | 通过 |

## P0（已修复）

1. **`usage.js` 全局 `.range-btn` 选择器误绑 Todo 按钮**：Todo 页有同名的 range 按钮，点击会触发 Usage 的筛选与请求。已把 Usage 初始化与 active 同步两处查询改为 `#view-usage .range-btn`。
2. **跨文件全局作用域重名**：`todo.js` 若使用 `usageState` / `selects` / `buildTable` 等名字会让拼接后的脚本 `SyntaxError`。全部顶层状态与函数统一 `todo` 前缀；`tests/web_assets.rs` 的拼接 `node --check` 通过。
3. **聚合组测试假数据**：`save_todo` 会清理单成员组，逐条保存无法构造组。测试改为先落库、再用 `sync_date_todos` 写回 `group_id`。

## P1（已修复）

1. **Overdue 在 `Today` 下归零**：汇总查询的 Overdue 忽略 `startDate`/`endDate`，只遵守 status/priority/search，并在 `status=done` 时归零；HTTP 测试覆盖。
2. **DB 读取系统时钟导致测试漂移**：`query_todo_summary(filter, today)` 的 `today` 由 service 传入 `today_date()`，持久层不读时钟。
3. **日期校验不严格**：`parse_date` 接受 `2026-1-1` 之类宽松写法；service 增加定长 `YYYY-MM-DD` 检查，并用公历往返拒绝 `2026-02-31`。

## P1（遗留风险，接受）

1. `All` 范围下 Daily/Groups 全量聚合、`q` 对 `text` 做 `instr(lower(...))` 全扫描；当前 Todo 量级（人类手写清单）可接受，后续数据量上来再加时间上限或索引。
2. `Groups` 的 `label` / `date` 取组内最早条目（不受筛选影响），成员计数受筛选影响；与设计 SQL 一致，但筛选后标题可能指向被过滤掉的成员。
3. 服务端 `pageSize` 越界钳制到 `1..=200`，`page` 仅校验 `>=1`；超大页码返回空页而不是 422，前端按 `totalPages` 钳制。

## P2（已知取舍）

1. `assets/todo.css` 刻意复制 Usage 的控件/卡片/表格样式（D6），两处独立维护；文件头已注明原因。
2. `todo.js` 复用 `web.js` 的 `createSelect` / `showToast` / `domEl` 与 `usage.js` 的 `apiGet`；依赖顺序由 HTML 与 `tests/web_assets.rs` 固定。
3. Todo 页无自动刷新，只保留手动 Refresh；只读接口不会与 TUI 内存态热同步。
4. 未新增 Home 页 Todo 入口卡片。
5. TUI 与浏览器外观/交互仍需用户 `cargo run` 真机验收（Agent 无法交互式验证）。
