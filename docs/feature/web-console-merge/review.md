# 统一 Web 控制台 — 复审记录

> 复审对象：`feat/web-console-merge`（`7cb05db5`、`a7f67eef`、`a3acca2d`）
> 复审范围：`git diff dev...HEAD`，含 TUI 入口、Rust 服务路由、全部浏览器资源、测试与规范文档
> 复审方式：codegraph / codebase-memory 调用链核对、静态检查、Node 语法检查、1448 / 900 / 640 宽度浏览器实测
> 结论：无 P0；P1 4 项、P2 5 项、P3 4 项，全部已在本分支修复。

## 关键点

1. **入口与路由整体成立**：`[WEB]` 几何、tooltip、事件置位、服务 URL、三路径统一 shell 均一致，旧 `/settings`、`/usage` 可直达并规范化 URL。
2. **合并文档的核心风险在“共享全局 + 视图生命周期”**：脚本同处一个全局作用域、视图首次初始化的失败恢复、切换视图后的后台轮询，是本轮最需要防守的位置。
3. **本次复审发现 1 个入口回归**：`lx-tui settings open` 跟随统一 URL 落到了 Home，而不是 Settings。
4. **浏览器脚本测试覆盖不足**：逐文件 `node --check` 抓不到跨文件重复 `const`，也无法发现 JS 引用了不存在的 DOM id。
5. **文档曾停在第二个提交**：第三个提交的 40px topbar、`.usage-controls`、全局滚动条与复审修复未回写。

## P0 — 阻断

无。

## P1 — 功能回归 / 用户可见错误

### R1. `settings open` 落到 Home 而非 Settings

- **位置**：`src/cli/settings.rs:395`、`src/settings/service.rs:323`
- **现象**：命令行 `lx-tui settings open` 与 JSON 输出的 `url` 都是 `/?token=...`，统一 shell 默认视图是 Home；命令语义从“打开设置页”退化为“打开控制台首页”。
- **原因**：`[WEB]` 默认 Home 的设计覆盖了 `url()` 的全部消费者，CLI 未区分“Home 入口”和“Settings 直达地址”。
- **修复**：新增 `SettingsService::settings_url()`（`url() + "#/settings"`），CLI 与 JSON 输出改用它；`[WEB]` 继续用 `url()` 落 Home。

### R2. Toast 覆盖全局顶栏并吞掉 tab 点击

- **位置**：`assets/web.css:140`、`assets/web.html:25`
- **现象**：toast host 固定在 `top: 16px`，高度约 36px，正好压在 40px 顶栏上；`.toast { pointer-events: auto }`，窄屏时 toast 居中覆盖 `Home / Settings / Usage`，3.2 秒内无法点击。
- **原因**：toast 样式沿用 Settings 旧单页时代（无全局顶栏），合并后未重新定位。
- **修复**：`top: calc(var(--web-topbar-height) + 12px)`。

### R3. Usage 离开视图后仍自动刷新，错误 toast 泄漏到其他视图

- **位置**：`assets/usage.js:576-587`、`:123-160`
- **现象**：开启 Auto 后切到 Home/Settings，定时器继续请求 `/api/usage/*`；请求失败会在当前视图弹出 Usage 的错误 toast。
- **原因**：单页化后没有视图生命周期钩子，旧独立页面的定时器默认“页面一直可见”。
- **修复**：定时器回调与错误 toast 先判断 `#view-usage` 是否 active；离开视图时保留定时器但跳过请求，返回后继续。

### R4. 首次初始化异常后无法重试，且会中断路由

- **位置**：`assets/web.js:249-274`、`assets/settings.js:768-770`、`assets/usage.js:719-721`
- **现象**：`initOnce` 声称失败可重试，但 `initSettingsView` / `initUsageView` 在做事前就把 `__settingsInitialized` / `__usageInitialized` 置为 true；重试直接 return。更严重的是 `bootWeb` 先 `showView(initial)` 再注册 `hashchange`，初始视图抛错后顶栏切页永久失效。
- **原因**：双层初始化守卫语义冲突；启动顺序把“注册路由”放在了“执行初始化”之后。
- **修复**：删除视图内层的一次性标志，由 `initOnce` 唯一负责；先注册 `hashchange` 再 `showView`；初始化失败只 toast，不再向上抛出中断启动。

## P2 — 测试缺口 / 文档漂移 / 状态错误

### R5. Usage 视图 DOM 与三路由一致性缺少回归断言

- **位置**：`src/settings/service/tests.rs:278-300`
- **现象**：删除 `usage.html` 后，测试只断言 `id="view-usage"` 与 `class="usage-controls"`；图表、表格、筛选器的挂载点被误删仍会通过。`/settings`、`/usage` 只做包含断言，未验证与 `/` 字节一致。
- **原因**：迁移测试从旧页面断言收缩为 shell 存在性断言，覆盖过度缩水。
- **修复**：三路径响应与 `page::html()` 全量相等；补 Usage 关键 DOM id 清单（summary/chart/table/filter/pagination）。

### R6. 浏览器脚本测试抓不到跨文件重复全局与失效 DOM id

- **位置**：`tests/web_assets.rs:10-46`
- **现象**：`node --check` 逐文件执行，而 10 个脚本共享同一全局作用域；跨文件重复 `const token` 会让整页 `SyntaxError`，测试仍绿。`getElementById('btn-empty-new-provider')` 这类不存在 id 也无人发现。
- **原因**：测试只覆盖同文件语法，没有按 HTML 加载顺序做联合解析，也没有 JS→HTML 契约检查。
- **修复**：测试从 `web.html` 解析脚本顺序，断言与清单一致，再拼接为单文件 `node --check`；另增 `getElementById` 字面量必须存在于 HTML 的断言。

### R7. 文档停留在第二个提交

- **位置**：`docs/feature/web-console-merge/design.md`、`task.md`、`docs/standards/*`
- **现象**：仍写 `--web-topbar-height: 48px`、`usage-topbar`、`.usage-toolbar`、“主体不动”、主工作区路径；未记录 `a3acca2d` 与复审修复。
- **原因**：第三个提交只改实现与测试，没有回写 feature 文档和现行规范。
- **修复**：同步 `design.md` / `task.md` / `html-assets.md` / `project-directory-structure.md`，并在 `task.md` 增加复审修复清单。

### R8. Usage 分页状态可能与响应页不一致

- **位置**：`assets/usage.js:123-160`、`:170-190`
- **现象**：`loadUsageDashboard` 覆盖 `logPage` 但不采纳响应中的 `page`；筛选后数据变少时，内部 `usageState.page` 仍停在旧页，翻页按钮和“Page N of M”可能错位。
- **原因**：翻页接口显式写 `usageState.page`，总览接口漏了同一同步。
- **修复**：两个加载器都以响应 `page` 为准；总览发现当前页超过总页数时回到最后一页重新拉取。

### R9. 缓存说明与实现不一致

- **位置**：`src/settings/service.rs:443`、`docs/standards/html-assets.md:43`
- **现象**：所有响应统一 `Cache-Control: no-store`，文档却称资源“可分别缓存”。
- **原因**：沿用旧实现，文档按理想状态描述。
- **修复**：文档改为“独立请求、统一 no-store，避免重启后命中旧资源”。

## P3 — 一致性 / 可维护性

### R10. 失效 id 与装饰性分隔条

- `assets/settings_connect.js:103` 查找不存在的 `btn-empty-new-provider`；删除该分支，只保留 `#provider-empty .primary`。
- `assets/web.html:413/544/630` 三个 `.models-resizer` 有拖拽光标和 `role="separator"`，但没有 id、未接线；补齐 id 并让 Provider/MCP/Skills/Subagents 四个列表各自记住宽度。

### R11. 服务不可用文案已过时

- `src/ui/text.rs:227`、`src/event/mod.rs:359` 仍写 “Settings service unavailable”。统一入口后应改为 “Web console unavailable”，日志同步。

### R12. 共享样式与辅助函数重复

- `usage.css` 重复 `color-scheme` / `box-sizing` / `.hidden`；`settings.css` 重复 `box-sizing` / `.hidden` / `.settings-shell main`；`web.css` 重复 `.usage-shell`。
- `settings.js` 的 `element()` 与 `web.js` 的 `domEl()` 完全等价；`web.js` 的 `currentView` 只写不读。
- `settings/service/handlers.rs:134` 的 `let _ = MAX_HEADER_BYTES;` 是无效语句。
- `WEB_LABEL.len()` 按字节算宽度、渲染按字符遍历；因 `const` 上下文不支持 `chars().count()`，保留 `.len()`，并确认标签为 ASCII 单宽字符。

### R13. 可访问性细节

- 顶栏切换时补 `aria-current="page"`；Usage 的 tabpanel 未与 tab 建立完整 `aria-controls` 关联。本轮先补 `aria-current`，其余保持现状，避免扩大改动面。

## 已核实无问题

- 删除符号无残留：`settings_button` / `usage_button` / `usage_url` / `usage_html` / `SidebarSettings` / `SidebarUsage` / `settings.html` / `usage.html` 均无引用。
- TUI `[WEB]` 几何：`x+1`、宽 5、最小侧栏 11、折叠/服务不可用隐藏；命中与渲染共用同一 `Rect`，与折叠按钮不重叠。
- 路由：静态资源白名单先于 token 校验；三路径 token 校验正常；未知资源 404；Content-Length 按字节。
- 脚本加载顺序与 `page::asset` 一致；所有 `onclick` 全局函数存在；无跨文件全局重名。
- 视图 CSS 挂起：body 级 select 菜单、toast、tooltip 的样式与 `.hidden` 均在 `web.css`；切换前关闭浮层。
- 安全：无外部域名、无动态 `innerHTML`、token 仅经 query 传递。
- `usage_charts.js` 的 `hasAnyCost` / `hasCostSeries` 语义正确，重复声明已消除。

## 修复验证

- `cargo fmt --check`、`cargo clippy --all-targets -- -D warnings`：通过。
- `cargo test`：5431 passed / 0 failed（另 6 ignored）。
- `cargo test --test web_assets`：2 passed（逐文件 + 拼接语法、DOM id 契约）。
- 浏览器实测：
  - toast `top=52px`，顶栏 `bottom=40px`，不再重叠。
  - Usage 自动刷新：Home 期间 0 次 `/api/usage/*` 请求，返回 Usage 后恢复。
  - 合成初始化异常：首次 toast 提示、路由仍可切换，第二次进入重试成功。
  - 分页：伪造 `page=5 / total=0` 后自动收敛到第 1 页。
  - MCP resizer 拖拽后只更新 MCP 网格宽度，Provider 不受影响。
  - 1448 / 900 / 640 宽度下 Home / Settings / Usage 无横向溢出，Usage 工具条按预期换行。
- 本次复审修复随 `feat/web-console-merge` 一并提交。
