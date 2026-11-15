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

## 验证

- `cargo test --lib config::tests::web_theme_round_trips_and_rejects_unknown_values`
- `cargo test --lib settings::service::logs::tests::web_theme_persists_and_is_injected_into_the_page`
- `cargo test --lib settings::service::tests::page_requires_token_and_serves_self_contained_assets`
- `cargo test --test web_assets`
- 浏览器 harness：Default → Glass 切换、布局与字体、刷新后首屏仍为 Glass、切回 Default 恢复直角 monospace。
