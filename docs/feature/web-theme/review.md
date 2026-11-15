# Web 主题与顶栏调整：复审记录

> 日期：2026-11-15
> 分支：`web-todo-tab`（worktree `.worktrees/web-todo-tab`）
> 合并提交：`ddd4ec6d`（`merge: web todo dashboard and glass theme into dev`）
> 范围：`dev...HEAD` 中主题、顶栏、分段控件、select、Quick Connect 表单与相关测试/文档
> 方式：3 个只读 subagent 并行审查（Rust/API、前端交互、样式与文档），结论回到 HEAD 代码与定向测试复核
> 结论：**无 P0；P1 4 项、P2 7 项，可修项已在本分支落地并随 `ddd4ec6d` 合并到 `dev`；人工真机验收待确认**

## 关键点

1. **主题持久化链路成立**：`Config.web_theme` → `settings.json` → 页面 `data-theme` 首屏注入 → `/api/state` 兜底；用户手动切换后，迟到读取不会回滚。
2. **风险集中在“整份配置读改写”**：主题 select 与 Settings Save 都通过 `PUT /api/config` 写同一个 `settings.json`；并发请求会互相覆盖，必须串行化。
3. **动效预算集中在指示块**：顶栏/range/provider/nav/select 共用 `.seg-pill`，hover 预览、离场回落、文字仅选中态变色；视图切换只做透明度淡入。
4. **主题切换不只换颜色**：还重新挂载 pill、同步尺寸并清理进场类；任何遗留动画都会表现为旧内容或位移。
5. **窄屏与 reduced-motion 是本次新增回归面**：152px 固定主题选择器在小视口会溢出；新增的 toast/状态点/图表/按钮动效必须服从系统降低动效设置。
6. **select 是共享组件**：菜单挂在 `body`，主题 select、Todo 筛选、Quick Connect 表单都复用；宿主重绘时必须显式销毁菜单。

## 验证结果

| 项目 | 结果 |
| --- | --- |
| `cargo fmt` | 通过 |
| `cargo clippy --all-targets -- -D warnings` | 通过 |
| `cargo test --lib config::tests::web_theme...` | 通过 |
| `cargo test --lib settings::service::logs::tests::web_theme...` | 通过 |
| `cargo test --lib settings::service::tests::page_requires...` | 通过 |
| `cargo test --test web_assets` | 通过（脚本拼接 `node --check`） |
| `cargo test` | 全量通过（lib 5568 passed，集成/文档测试通过） |
| 浏览器 harness | Glass/Default 切换、主题 select 菜单与 pill、500px 顶栏、Todo 令牌、Home 重试、滚动/tooltip 路径均通过 |
| 真机 `cargo run` | 待用户确认观感与持久化 |

## P1（已修复）

### P1-1 配置写入并发覆盖：主题与 Settings Save 可能互相丢失

**位置**

- `assets/web.js` 的 `initWebTheme` / `saveWebTheme`
- `src/settings/service/handlers.rs` 的 `put_config`
- `src/config/mod.rs` 的 `save` / `temp_path`

**现象**

快速连续切换主题，或切换主题的同时 Settings 正在 Save，最终 `settings.json` 可能落成旧值；后到的失败回滚还会把用户最新选择改回去。

**错误原因**

- 每次 `PUT /api/config` 都独立执行「读磁盘 base → 合并 → 写整份配置」，服务端连接各自 `tokio::spawn`，没有互斥。
- `config::save` 的临时文件名只含进程号；同进程并发写会命中同一个临时文件。
- 客户端每次切换都立即发 PUT，没有顺序保证；失败回滚使用发起时捕获的 `previous`，不判断是否已有更新选择。

**修复**

- `ServiceState` 增加 `config_write: tokio::sync::Mutex<()>`；`PUT /api/config` 在锁内读改写，串行化主题与 Settings Save。
- 客户端主题保存改为 Promise 链串行；`webThemeSaveSeq` 只允许最新一次失败回滚，并回滚到 `webThemePersisted`（最后一次成功落盘值）。
- 初始兜底读取成功时同步 `webThemePersisted`。

**验证**

- 代码审查确认所有 `PUT /api/config` 都经过同一把锁；主题保存链按点击顺序执行。
- 浏览器 harness 连续切换 Default/Glass 后刷新，首屏主题与最后选择一致。

### P1-2 `handlers.rs` 超过生产代码 1000 行硬上限

**位置**

- `src/settings/service/handlers.rs`（复审前 1010 行）
- 新增 `src/settings/service/handlers/json.rs`

**现象**

主题配置块把 `handlers.rs` 从 dev 的 994 行推到 1010 行，违反项目「生产代码单文件上限 1000 行」规范。

**错误原因**

路由、HTTP 解析、配置合并、命令 CRUD 与极简 JSON 解析长期堆在同一文件；本次主题字段新增成为压过上限的最后一根稻草。

**修复**

把 `JsonValue`、`JsonParser`、`json_object_pairs`、`json_object_field` 整体迁到 `handlers/json.rs`，父模块只 `use` 所需符号；解析器内部方法降为模块私有。

**验证**

`wc -l` 确认 `handlers.rs` 回到 772 行；`cargo clippy --all-targets -- -D warnings` 通过。

### P1-3 窄屏顶栏溢出，主题选择器被裁掉

**位置**

- `assets/web.css` 的 `.web-topbar` / `.web-tabs` / `.web-theme-select`
- `@media (max-width: 640px)`

**现象**

主题 select 固定 152px，加上 logo 与 4 个 tab，在约 500px 视口下右侧选择器被挤出屏幕；`body` 又是 `overflow:hidden`，无法横向滚动补救。

**错误原因**

主题 select 按桌面宽度设计为固定宽度，窄屏媒体查询只缩小了 padding，没有处理固定宽度与 tab 总宽。

**修复**

≤640px 时主题 select 收成 44px 箭头按钮并隐藏标签（`aria-label="Theme"` 仍保留可访问名），`.web-tabs` 允许横向滚动并隐藏滚动条。

**验证**

浏览器 harness 500px：`scrollWidth == clientWidth == 500`，主题 select 右缘 488px，标签 `display:none`，截图确认未裁切。

### P1-4 标准与实现漂移：视图位移与状态宽度

**位置**

- `assets/web.css` 的 `view-in`
- `docs/standards/html-assets.md` 的 `.action-status`
- `docs/standards/tui-design-requirements.md` 的 Todo 空白点击

**现象**

- Default 的 `view-in` 含 4px `translateY`，与「视图切换只做透明度淡入」的约定冲突；从 Glass 切回 Default 时动画属性变化会让整页重播位移。
- `html-assets.md` 仍要求 `.action-status` 预留最小宽度，与 D12 及三份视图 CSS 相反。
- TUI 规范同一句里既说空白点击聚焦容器，又说“不聚焦”，与代码和回归测试相反。

**错误原因**

功能迭代先改实现，规范文档停留在旧版本；动效约定只写进 Glass 章节，Default 的旧动画没有被一起收敛。

**修复**

`view-in` 改为纯透明度淡入；`html-assets.md` 改为宽度跟随文案、`tabular-nums` + `nowrap`；TUI 规范改为“仍聚焦容器，但不进入编辑、不产生数据动作、不穿透”。

**验证**

harness 切换 Default/Glass 无整页位移；文档与 `settings.css` / `usage.css` / `todo.css` 的实现一致。

## P2（已修复）

### P2-1 主题图表令牌是死声明，Todo 徽标硬编码颜色

**位置**

- `assets/theme.css` 的 `--todo-chart-*` / `--chart-grid`
- `assets/todo.css` 的 `.todo-charts` 与 `.todo-status-in_progress` / `.todo-priority-p0`

**现象**

`.todo-charts` 自身重新声明 `--todo-chart-planned/done`，元素自身声明压过 `html[data-theme="glass"]` 继承值，Glass 覆盖完全不生效；`--chart-grid` 无人读取；进行中/P0 徽标写死色值，与 Home 卡使用 `--err` 的 P0 徽标不一致。

**错误原因**

令牌定义位置与读取位置不一致（定义在根，覆盖在子元素）；组件规则绕过令牌直接写色值。

**修复**

令牌集中到 `.todo-main`，主题在 `html[data-theme="glass"] .todo-main` 覆盖；删除 `.todo-charts` 的重复声明与未使用的 `--chart-grid`；徽标改用 `--todo-in-progress` / `--todo-p0`。

**验证**

harness Glass：`--todo-chart-planned=#64d2ff`、`--todo-p0=#ff6961`，P0 徽标实际颜色为 `rgb(255, 105, 97)`。

### P2-2 reduced-motion 覆盖不全

**位置**

- `assets/web.css` 的 `@media (prefers-reduced-motion: reduce)`
- `assets/todo.css` 的 `.status-dot.loading`、优先级条
- `assets/theme.css` 的 logo/按钮缩放

**现象**

系统开启降低动效后，状态点脉冲、优先级条宽度过渡、按钮按压缩放与 toast 进退场仍在播放。

**错误原因**

reduced-motion 规则按新增组件逐个补，遗漏了旧视图 CSS 与主题覆盖里的动效。

**修复**

在 `web.css` 增加 `*, *::before, *::after` 全局兜底：动画时长降到 0.01ms、迭代 1 次、过渡时长 0.01ms、`scroll-behavior:auto`；保留原有针对性规则。

### P2-3 select 键盘交互不完整

**位置**

- `assets/web.js` 的 `createSelect` / `renderMenu` / `segmentTarget`

**现象**

- 打开菜单后按 Tab 不会关闭，菜单悬空到下一次鼠标点击。
- 鼠标停在某选项时按方向键，胶囊仍停在被 hover 的行，Enter 却选中 `.active` 行。
- 长菜单里连续按方向键，`.active` 行可能滚出 `max-height` 可视区。

**错误原因**

键盘路径只更新 `active` 索引，没有清除 hover 预览、没有滚动活动行，也没有处理 Tab 失焦。

**修复**

Tab 直接 `close()`；方向键先 `clearSegmentPreview(..., 'hover')` 再重绘；`renderMenu` 用 `offsetTop/offsetHeight` 把 active 行夹进菜单滚动区。

### P2-4 Quick Connect 表单重绘泄漏 select 菜单

**位置**

- `assets/settings_connect.js` 的 `renderPresetFormFields`
- `assets/web.js` 的 `createSelect`

**现象**

预设表单任一下拉变化会重绘字段；旧 `.select-host` 被丢弃，但菜单挂在 `body` 上，越积越多，且每个菜单都带 MutationObserver/ResizeObserver 与 pill。

**错误原因**

`createSelect` 只在同一宿主再次初始化时清理旧菜单；宿主被整块删除时没有销毁入口。

**修复**

`createSelect` 实例增加 `destroy()`（关闭、移除 body 菜单、清空宿主引用）；`renderPresetFormFields` 在清空宿主前遍历 `.select-host` 调用 `destroy()`。

### P2-5 主题兜底读取完整 `/api/state`

**位置**

- `assets/web.js` 的 `loadWebTheme`
- `src/settings/service/handlers.rs` 的 `state_json`

**现象**

每次启动都请求完整 state，Home/Todo/Usage 也会把 `web_search` 的 API Key 与命令列表读进内存；落在 Settings 时还会重复请求一次。

**错误原因**

服务端没有独立的 `GET /api/config`，主题兜底复用了现成的 `/api/state`。

**处置**

**接受**。请求同源、token 保护，且首屏主题已由服务端注入；后续若增加 `GET /api/config` 再替换。当前不改接口面，避免为兜底扩大改动。

### P2-6 `initOnce` 异步失败仍记为 `done`

**位置**

- `assets/web.js` 的 `initOnce`
- `assets/settings.js` / `assets/todo.js` / `assets/usage.js` 的 loader

**现象**

首次进入视图时数据请求失败，loader 在内部 catch 后正常 resolve，`__webInit[key]` 变成 `done`；再次进入不会自动重试，只能点视图内 Refresh/Reload。

**错误原因**

`initOnce` 用 Promise reject 判断失败，但各视图为保证错误提示不冒泡，把异常消化在 loader 内。

**处置**

**接受**。三个视图都有行内错误提示和显式 Refresh/Reload；把 loader 改为 reject 会引入重复绑定事件与未处理 rejection 的风险。后续可在 loader 返回 `{ok:false}` 并让 `initOnce` 识别。

### P2-7 主题文档缺少复审记录

**位置**

- `docs/feature/web-theme/design.md`
- `docs/feature/web-theme/task.md`
- `docs/standards/project-directory-structure.md`

**现象**

目录规范没有 `theme.css` / `home.js`，design/task 未记录配置互斥、窄屏与 reduced-motion 修复。

**错误原因**

主题功能按提交增量交付，文档只覆盖首版令牌与动效；复审修复与 Home 资源是在后续提交追加的，没有回写目录地图与执行清单。

**修复**

design 增加 D10 与“复审修复”章节；task 增加复审清单；目录规范补 `theme.css`、`home.js` 与 `/api/todo/home`；新增本文件。该文件同时覆盖 `home.js`（Todo 特性）与 `theme.css`（主题特性）两份资源。

---

## 已核实无问题

- `theme.css` 顶层选择器均以 `html[data-theme="glass"]` 限定，Default 零污染；加载顺序在四张视图样式表之后。
- `.web-tab-pill` / `.seg-pill` 的横向 `transform/width`、竖向 `translateY/height` 与 `offsetTop/offsetHeight` 坐标系一致；隐藏视图量不到尺寸时回落 `opacity:0`，由 ResizeObserver 补量。
- `bindSegmentGroup` 有 `dataset.segmentBound` 守卫，主题切换反复 `initSegmentPills` 不会重复绑定。
- 主题 select 菜单选项在 Glass 下只由 `.seg-pill` 表达 hover/选中，Default 才使用选项底色；此前 `.web-theme-menu` 特异性覆盖已修正。
- 主题保存失败会回滚并 toast；迟到 `/api/state` 响应不会覆盖手动选择。
