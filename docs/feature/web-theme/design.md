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
| D9 | 玻璃主题动效：顶栏 tab 与视图内小 tab（range / provider）共用滑动指示块，hover 即预览、离场回落到选中项、文字颜色只在选中态变化；视图切换旧视图立即隐藏、新视图仅做 0.18s 透明度淡入，不对整页施加 transform/filter；下拉菜单 scale 0.97 入场；卡片/按钮/侧栏 hover 统一 0.18s 过渡 | 参考项目的液态手感来自指示块的位移与宽度弹簧；但整页 blur/位移会让带 `backdrop-filter` 的玻璃卡片逐帧重采样，产生卡顿与旧内容残留，因此把动效预算集中在指示块上 |

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

### 动效

- `theme.css` 定义 `--spring-ease: cubic-bezier(.22, 1, .36, 1)`，作为玻璃主题指示块位移/宽度过渡的统一曲线。
- 顶栏 `.web-tab-pill` 绝对定位在 `.web-tabs` 内；`web.js::initSegmentPills` 为 `.range-group` / `.provider-tabs` 注入 `.seg-pill`，`syncSegmentPills` 在视图切换、主题切换、resize 与 `active` 类变化时同步 `translate3d` 与宽度。首次落位用 `no-transition` 直接定位，随后 24ms 解锁过渡，避免从 0 宽度滑入。
- 指示块跟随 hover / focus 预览目标项，`pointerleave` / `focusout` 后回落到 `.active`；文字颜色只在 `.active` 时变化，hover 不改变颜色，避免与选中态抢视觉。
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
