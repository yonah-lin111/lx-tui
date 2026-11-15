# Web 主题与顶栏调整

## 目标

- 参考本机 `ufi-ui-prototypes` 的 iOS 26 液态玻璃视觉，为 Web 控制台增加可选主题。
- 顶栏最右侧提供主题 select，选择结果持久化到 `settings.json`。
- 移除顶栏左侧 `lx-tui` 文字，只保留像素 logo；tab 紧邻 logo。
- Default 主题维持现有黑白灰 monospace 外观，不因新增主题回归。

## 决策

| 编号 | 决策 | 理由 |
| --- | --- | --- |
| D1 | 主题仅 `Default` + `Glass` 两档，Glass 只做暗色 | 参考项目的亮色依赖无级滑杆插值，当前需求是 select 切换；先交付稳定的双档，避免把亮度插值引入全局状态 |
| D2 | 全站换肤：Home / Settings / Usage / Todo 共用 `theme.css` 覆盖 | 只换顶栏会让玻璃外壳与直角灰阶内容割裂；统一覆盖令牌可一次覆盖所有视图 |
| D3 | 玻璃规则全部限定在 `html[data-theme="glass"]`，并晚于视图样式表加载 | Default 零影响；视图 CSS 继续刻意隔离，不把主题规则散落各文件 |
| D4 | 壁纸用多层 CSS 渐变模拟，不打包参考项目 5.4MB `bg.jpeg` | 单二进制体积不膨胀，任意分辨率无缩放伪影，离线可用 |
| D5 | Glass 使用 SF Pro 系统字体栈，Default 保留 monospace | 玻璃质感依赖系统字体；开发者控制台的等宽基线只在 Default 保留 |
| D6 | 持久化字段 `web_theme`（`default` / `glass`）写入 `settings.json`，经 `/api/config` 读写 | 与服务端其余配置同源；换浏览器、重启二进制后仍生效 |
| D7 | 页面响应时服务端注入 `data-theme`，`web.js` 启动后再读 `/api/config` 兜底 | 避免刷新时先画默认主题再闪成玻璃主题；兜底修正外部修改 |
| D8 | 顶栏使用自绘 `createSelect`，右侧固定宽度；用户已手动切换后忽略迟到的初始读取，保存失败回滚并 toast | 与 Settings/Usage/Todo 的下拉一致；避免竞态把用户选择回滚，失败不留下“看起来已保存”的假象 |
| D9 | 玻璃主题动效：顶栏 tab、视图内小 tab（range / provider）、设置页左侧竖向 nav、select 菜单与 Commands/Provider/MCP/Skills/Subagents 列表共用滑动指示块，hover 即预览、移入分组内的标题/空白等非条目区域立即回落到选中项、离场同样回落、文字颜色只在选中态变化；视图切换旧视图立即隐藏、新视图仅做 0.18s 透明度淡入，不对整页施加 transform/filter；下拉菜单 scale 0.97 入场；卡片/按钮/侧栏 hover 统一 0.18s 过渡 | 参考项目的液态手感来自指示块的位移与宽度弹簧；但整页 blur/位移会让带 `backdrop-filter` 的玻璃卡片逐帧重采样，产生卡顿与旧内容残留，因此把动效预算集中在指示块上 |
| D10 | 配置写入串行化：服务端 `PUT /api/config` 在读改写外持互斥锁；客户端主题保存按点击顺序串行，失败只回滚到最后一次成功持久化值 | 主题切换与 Settings 保存都写整份 `settings.json`，并发读改写会互相覆盖；临时文件又只按进程号命名，必须保证同一进程内不并发写 |

## 实现

### 配置

- `Config` 新增 `web_theme: String`，默认 `default`；`validate` 只接受 `default|glass`。
- `Config::to_json` / `from_json` 与 `settings/service/handlers.rs::config_json` / `config_from_json` 同步；`null` 视同缺省，字符串做 trim + 小写。
- `PUT /api/config` 继续以磁盘配置为基底合并，只改主题不会覆盖布局字段。

### 首屏与切换

- `page::html_for_theme(theme)` 把静态 `<html data-theme="default">` 替换为持久化值，路由 `GET /`、`/settings`、`/usage`、`/todo` 共用。
- `web.js::initWebTheme` 创建顶栏 select；`applyWebTheme` 只改 `document.documentElement.dataset.theme` 与 select 标签。
- `loadWebTheme` 读 `/api/state` 的 `config.web_theme` 兜底；`webThemeTouched` 阻止迟到响应覆盖用户刚做的选择。
- `saveWebTheme` PUT `{config:{web_theme}}`；失败回滚并 toast `Failed to save theme: ...`。

### 样式

- 新增 `assets/theme.css`，以 `html[data-theme="glass"]` 覆盖 `--bg/--panel/--border/--text/...` 令牌，再覆盖顶栏、卡片、表单、按钮、select 菜单、侧栏、表格、图表提示、toast、骨架与滚动条。
- Glass 壁纸：深色底 + 蓝/紫/青三层径向渐变 + 纵向渐变，`background-attachment: fixed`。
- 顶栏：`backdrop-filter: blur(24px) saturate(180%)`，tab 改为胶囊高亮；主题 select 为胶囊按钮。
- 容器：卡片/字段组 16–20px 圆角、半透明面板、内高光 + 远距柔阴影。
- `prefers-reduced-motion` 下关闭 Home 卡片位移动效。

### 顶栏结构

- `web.html`：`<div class="web-topbar-left">` 包住 logo 与 `.web-tabs`，logo 与 tab 间距 10px；右侧 `#web-theme-select`。
- 移除 `.web-brand` 内 `<span>lx-tui</span>`，保留 `aria-label="lx-tui home"` 与像素 logo。
- `.web-theme-select` 固定 152px，避免 `Theme: Default` / `Theme: Glass` 切换时抖动。
- 主题 select 的下拉选项在 Glass 下与其他 select 完全一致：`web.css` 中 `.web-theme-menu` 的 hover/选中底色只作用于 Default（`html:not([data-theme="glass"])`），Glass 交给统一的 `.seg-pill` 与透明选项，不得叠加旧底色。

### 动效

- `theme.css` 定义 `--spring-ease: cubic-bezier(.22, 1, .36, 1)`，作为玻璃主题指示块位移/宽度过渡的统一曲线。
- 顶栏 `.web-tab-pill` 绝对定位在 `.web-tabs` 内；`web.js::initSegmentPills` 为 `.range-group` / `.provider-tabs` 注入 `.seg-pill`，为 `nav.side` 注入竖向 `.seg-pill`；`syncSegmentPills` 在视图切换、主题切换、resize 与 `active` 类变化时同步位置与尺寸，滚动时指示块随内容一起移动。首次落位用 `no-transition` 直接定位，随后 24ms 解锁过渡，避免从 0 宽度滑入。
- 设置页左侧 `nav.side` 的按钮使用 `width: calc(100% - 16px)` 扣除左右 8px 外边距，避免 `overflow-y:auto` 被撑出横向滚动条；竖向指示块用 `height` + `translateY` 跟随 hover / 选中项，并用 `offsetTop` / `offsetHeight` 量位置，避免菜单 scale 入场时量到缩放后的尺寸。
- `.select-menu` 的选项与 `.command-list` / `.provider-list` 的 `.nav-item` 复用同一套竖向指示块；select 菜单点击、键盘上下选择与选项重绘后都会显式同步，列表整批重绘由 `childList` 变更观察补一次挂载。
- 指示块跟随 hover / focus 预览目标项，`pointerleave` / `focusout` 后回落到 `.active`；`pointerover` 落在分组内的标题、空白等非条目区域时也必须清掉预览，不能停在上一个条目上。文字颜色只在 `.active` 时变化，hover 不改变颜色，避免与选中态抢视觉。
- 视图切换把旧视图立即置为 `display:none`，新视图只加 `glass-enter` 做 0.18s 透明度淡入；不再对整页施加 `filter: blur()` 或位移动画，也不保留离场层。`animationend` 只在事件目标等于视图自身时清理。
- 下拉菜单使用 `scale(.97) → 1` 的 0.2s 入场；按钮/卡片/侧栏 hover 统一 0.18s 颜色与阴影过渡，主按钮按压缩放 0.97。
- `prefers-reduced-motion` 下关闭指示块过渡、视图淡入与菜单入场。

## 验证

- `cargo test --lib config::tests::web_theme_round_trips_and_rejects_unknown_values`
- `cargo test --lib settings::service::logs::tests::web_theme_persists_and_is_injected_into_the_page`
- `cargo test --lib settings::service::tests::page_requires_token_and_serves_self_contained_assets`
- `cargo test --test web_assets`
- 浏览器 harness：Default → Glass 切换、布局与字体、刷新后首屏仍为 Glass、切回 Default 恢复直角 monospace。
- 浏览器 harness：顶栏 / range / provider 指示块 hover 预览、离场回落、文字仅选中态变色；视图切换旧视图立即消失、新视图仅 opacity 淡入。

## 复审修复（2026-11-15）

- **配置保存竞态**：服务端 `PUT /api/config` 在读改写外持 `ServiceState.config_write` 互斥锁；客户端主题保存链按点击顺序串行，失败只回滚到最后一次成功持久化的主题，避免快速切换或与 Settings Save 并发时互相覆盖。
- **超限拆分**：`handlers.rs` 曾因主题配置块达到 1010 行，超过生产代码 1000 行上限；把极简 JSON 解析器拆到 `src/settings/service/handlers/json.rs`。
- **窄屏顶栏**：≤640px 时主题 select 收成 44px 箭头按钮、隐藏标签，tab 容器允许横向滚动，避免 152px 固定宽度把右侧选择器挤出视口。
- **视图切换**：Default 的 `view-in` 改为纯透明度淡入，去掉 4px 位移；主题切换不再触发整页位移。
- **主题令牌**：移除 `theme.css` 中被子元素自身声明覆盖的 `--todo-chart-*` / 未读取的 `--chart-grid`；Todo 徽标改用 `--todo-p0` / `--todo-in-progress`，Glass 下统一到主题色。
- **reduced motion**：`web.css` 增加全局兜底，关闭 toast、状态点脉冲、图表条形与按钮缩放等残余动效。
- **select 键盘**：Tab 关闭菜单；方向键清除 hover 预览并让 active 行滚入菜单可视区；Quick Connect 表单重绘前显式 `destroy()` 旧 select，避免 body 上累积孤立菜单。
- **接受风险**：`loadWebTheme` 启动兜底仍读完整 `/api/state`（含 web_search Key），后续可换专用只读端点；`initOnce` 的异步加载失败仍记为 `done`，依赖各视图行内错误 + Refresh/Reload 重试。
