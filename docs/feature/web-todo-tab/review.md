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
4. 初版未新增 Home 页 Todo 卡片；2026-11-15 追加 D13 摘要卡（只读、点击条目跳转搜索）。
5. TUI 与浏览器外观/交互仍需用户 `cargo run` 真机验收（Agent 无法交互式验证）。

---

## 2026-11-15 追加：图表、加载态、状态宽度

### 验证结果

| 项目 | 结果 |
| --- | --- |
| `cargo test --lib db::todo::query` | 13 passed（含优先级分布 1 条） |
| `cargo test --lib settings::service::todo` | 5 passed（overview 含 `priority` 断言） |
| `cargo test --lib settings::service::tests::page_requires...` | 1 passed |
| `cargo test --test web_assets` | 2 passed（脚本拼接 `node --check`） |
| `cargo test` | 全量通过（含上述新增断言） |
| `cargo clippy --all-targets -- -D warnings` | 通过 |
| 浏览器 harness（`ego-browser`，API 延迟 900ms） | 顶部 2px 进度条 + 骨架可见；数据到达后趋势图/环图/优先级横条全部渲染；刷新期间时间戳不变、spinner 显示 |
| `.action-status` 宽度 | `Updated 10:33:10` / 刷新中 / 恢复后均为 136px |

### P0（已处理）

1. **用户截图中 Todo 图表不显示**：现场运行的二进制早于 `assets/todo.css`、`assets/todo_charts.js` 落盘（`include_str!` 资源随编译固定），HTML 已是新版而样式/脚本还是旧版。当前源码在 harness 中三图正常渲染；需重启 `cargo run` 后复验。
2. **js 拼接作用域**：`todo_charts.js` 顶层符号全部 `todo` 前缀，`web_assets` 拼接解析通过。

### P1（已处理）

1. **首屏高度跳变**：Todo/Usage 首次加载渲染与最终布局同尺寸的局部骨架；顶部 2px 进度条等待 init Promise settle（最短 180ms），不遮挡内容；骨架切真实数据淡入一次；刷新保留旧数据不骨架化。
2. **状态文案快速切换**：已有数据时刷新只显示行内 `.status-spinner`，保持上一次 `Updated HH:MM:SS` 不变，数据返回后一次性更新时间；`.action-status` 预留 `16ch/15ch + 30px`（含 spinner 槽位），实测宽度恒为 136px。

### P2（接受）

1. `All` 跨越十年以上时月度柱会非常细，暂未加年度聚合或最大桶数上限。
2. 全局遮罩只在视图首次初始化时出现；已初始化视图之间的切换不做假加载。
3. 骨架扫光、顶部进度条与视图淡入遵循 `prefers-reduced-motion`。

### 后续变更（2026-11-15）

- 手动验收后取消 `.action-status` 的 `min-width` 预留：状态宽度跟随文案，Settings / Usage / Todo 的按钮组仍因容器右对齐固定在右缘；上文 136px 实测仅代表旧实现。
