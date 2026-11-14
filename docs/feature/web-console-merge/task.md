# 统一 Web 控制台 — 任务清单

> 状态：**已合并到 `dev`**（merge `bc5d6f3a`）。
> 设计文档：[design.md](design.md)
> 复审记录：[review.md](review.md)
> 日期：2026-11-14
> 实现位置：linked worktree `/Users/yonah/projects/agent/lx-tui/.worktrees/web-console-merge`（合并后已移除）

---

## 0. 审核门禁

- [x] **T0.1** 用户确认 `[WEB]` 作为唯一入口单词。
- [x] **T0.2** 用户确认 `/`、`/settings`、`/usage` 三个地址统一返回 `web.html`，旧地址按 pathname 直达对应视图。
- [x] **T0.3** 用户确认首页复用 LX 像素 logo（内联 SVG），并保留 Settings / Usage 两张入口卡。
- [x] **T0.4** 用户确认顶部 bar 为 `Home / Settings / Usage`，Settings 左栏与 Usage 筛选栏保留。
- [x] **T0.5** 用户确认删除 Settings 左栏 `lx-tui` 标题与 Usage 顶部 `USAGE` 标题行。
- [x] **T0.6** 用户确认 CSS 采用 `media="not all"` 挂起方案，不做全量 CSS 合并。
- [x] **T0.7** 用户确认 `[WEB]` 每次点击固定落 Home，不做"记住上次视图"。

> 以上全部勾选后，才允许进入 T1。

---

## 1. TUI 单入口

### T1.1 文案常量

- [x] 修改 `src/ui/text/workspace.rs`：
  - 删除 `pub const SETTINGS_LABEL: &str = "[SET]";`
  - 删除 `pub const USAGE_LABEL: &str = "[USE]";`
  - 新增：
    ```rust
    /// 侧栏底边框统一 Web 控制台入口按钮文案（纯 ASCII）。
    pub const WEB_LABEL: &str = "[WEB]";
    ```
  - 注释从"prompt 底边框左侧设置按钮"改为"侧栏底边框统一 Web 控制台入口"。

- [x] 验收：`rg "SETTINGS_LABEL|USAGE_LABEL" src/` 无结果（历史 docs 除外）。

### T1.2 按钮几何、命中与绘制

- [x] 重写 `src/ui/controls/footer.rs`：
  - 文件头注释改为"侧栏底边框统一 Web 控制台 `[WEB]` 按钮几何、命中与绘制"。
  - 删除 `settings_button` / `settings_button_at` / `render_settings_button`。
  - 删除 `usage_button` / `usage_button_at` / `render_usage_button`。
  - 新增 `web_button(view, state) -> Option<Rect>`：
    - 条件：`state.settings_available && !state.sidebar_collapsed && area.width >= 11 && area.height >= 2`。
    - `Rect::new(area.x + 1, area.bottom() - 1, 5, 1)`。
  - 新增 `web_button_at(view, state, column, row) -> bool`。
  - 新增 `render_web_button(frame, view, state)`：逐格 `reset + set_char + style::accent()`。
  - 常量：
    ```rust
    const WEB_BUTTON_WIDTH: u16 = text::WEB_LABEL.len() as u16; // 5
    const WEB_BUTTON_MARGIN: u16 = 1;
    const WEB_BUTTON_MIN_WIDTH: u16 = WEB_BUTTON_WIDTH + 3 + 1 + 2; // 11
    ```
- [x] 修改 `src/ui/controls/mod.rs`：
  ```rust
  pub(crate) use footer::render_web_button;
  pub use footer::{web_button, web_button_at};
  ```
- [x] 修改 `src/ui/mod.rs:87-88`：
  ```rust
  controls::render_web_button(frame, &view, state);
  ```
  删除第二行 `render_usage_button`。

- [x] 验收：`rg "settings_button|usage_button|render_settings_button|render_usage_button" src/` 无结果。

### T1.3 Tooltip

- [x] `src/app/tooltip.rs`：
  - 删除 `SidebarSettings`、`SidebarUsage`。
  - 新增：
    ```rust
    /// 侧栏底边框统一 Web 控制台入口 `[WEB]`。
    SidebarWeb,
    ```
- [x] `src/ui/text.rs`：
  ```rust
  TooltipTarget::SidebarWeb => "open web",
  ```
- [x] `src/ui/tooltip.rs`：
  ```rust
  if let Some(area) = ui::web_button(view, state) {
      targets.push((TooltipTarget::SidebarWeb, area));
  }
  ```
  删除两个旧 `if let` 块。

- [x] 验收：`rg "SidebarSettings|SidebarUsage" src/` 无结果。

### T1.4 状态与事件

- [x] `src/app/state.rs`：
  - 删除 `settings_open_requested`、`usage_open_requested`。
  - 新增：
    ```rust
    /// `[WEB]` 点击请求：事件层取走后打开统一 Web 控制台。
    pub web_open_requested: bool,
    ```
  - 初始化处两个 `false` 改一个 `web_open_requested: false`。
- [x] `src/event/mouse/press.rs:215-224`：
  ```rust
  if ui::web_button_at(view, state, mouse.column, mouse.row) {
      state.web_open_requested = true;
      *dirty = true;
      return;
  }
  ```
- [x] `src/event/mod.rs:345-369`：把两元素循环改为单次：
  ```rust
  // 侧栏底边框 `[WEB]` 点击：后台打开统一 Web 控制台，不阻塞事件循环。
  if std::mem::take(&mut state.web_open_requested) {
      if let Some(url) = settings_service.as_ref().map(SettingsService::url) {
          tokio::task::spawn_blocking(move || {
              if let Err(error) = crate::platform::open_browser(&url) {
                  tracing::warn!(%error, "opening web console failed");
              }
          });
      } else {
          // 保留现有"服务不可用"toast 文案与锚点逻辑
      }
  }
  ```

- [x] 验收：`rg "settings_open_requested|usage_open_requested" src/` 无结果。

### T1.5 TUI 测试

- [x] 重写 `src/ui/controls/tests.rs`：
  - 文件头注释改为 `[WEB]`。
  - `usage_button_renders_right_of_settings_and_hits` → `web_button_renders_and_hits`：
    - 断言 `web_button` 的 `x = view.left_panel.x + 1`、`y = bottom - 1`、`width = 5`。
    - 5 列命中；`x-1`、`right`、上一行不命中。
    - 渲染文本 == `text::WEB_LABEL`。
    - 折叠态、`settings_available = false` 时 `None`。
  - `usage_button_width_threshold_is_seventeen` → `web_button_width_threshold_is_eleven`：
    - 宽度 10：`None`。
    - 宽度 11：`Some` 且命中。
  - `sidebar_bottom_border_has_no_button_after_settings` → `sidebar_bottom_border_has_web_button`：
    - `bottom[1..6] == "[WEB]"`。
    - `[WEB]` 右侧至折叠按钮全为 `─`。
    - 断言不包含 `[SET]`、`[USE]`。
  - `no_tooltip_target_points_at_the_removed_ui_button` → `tooltip_target_points_at_web_button`：
    - `tooltip_hint(SidebarWeb) == "open web"`。
    - 5 列命中 `Some(SidebarWeb)`。
    - 右侧至折叠按钮无 tooltip 目标。
- [x] 重写 `src/event/tests/mouse_click.rs:883-931`：
  ```rust
  #[test]
  fn mouse_down_on_web_button_requests_browser_open() {
      // ...
      let web_btn = ui::web_button(&geo.view, &state).expect("[WEB] visible");
      handle_terminal_event(mouse(MouseEventKind::Down(MouseButton::Left), web_btn.x, web_btn.y), ...);
      assert!(state.web_open_requested);
      assert!(dirty);
  }
  ```

- [x] 验收：
  ```bash
  cargo test ui::controls
  cargo test event::tests::mouse_click
  ```

---

## 2. 服务端路由与资源

### T2.1 页面与资源表

- [x] 修改 `src/settings/service/page.rs`：
  - `html()` 改为 `include_str!("../../../assets/web.html")`。
  - 删除 `usage_html()`。
  - `asset()` 新增：
    ```rust
    "/web.css" => Some(("text/css; charset=utf-8", include_str!("../../../assets/web.css"))),
    "/web.js" => Some(("text/javascript; charset=utf-8", include_str!("../../../assets/web.js"))),
    ```
  - 保留 `settings.css`、`settings*.js`、`usage.css`、`usage*.js`。
  - 文件头注释更新为"统一 Web 控制台资源"。
- [x] 修改 `src/settings/service/handlers.rs:241-245`：
  ```rust
  ("GET", "/") | ("GET", "/settings") | ("GET", "/usage") => {
      Response::ok("text/html; charset=utf-8", page::html().to_string())
  }
  ```
- [x] 修改 `src/settings/service.rs`：
  - 保留 `url()`，注释改为"统一 Web 控制台 URL"。
  - 删除 `usage_url()`。

- [x] 验收：`rg "usage_html|usage_url" src/` 无结果。

### T2.2 HTTP 测试

- [x] 修改 `src/settings/service/tests.rs`：
  - `/` 无 token → 403，带 token → 200。
  - 新增：`/settings?token=...`、`/usage?token=...` 均 200。
  - 页面断言保留 Settings 全部标记。
  - 新增：
    ```rust
    assert!(html.contains("id=\"view-home\""));
    assert!(html.contains("id=\"view-settings\""));
    assert!(html.contains("id=\"view-usage\""));
    assert!(html.contains("/web.css"));
    assert!(html.contains("/web.js"));
    assert!(html.contains("/settings.css"));
    assert!(html.contains("/usage.css"));
    ```
  - 删除 `service.usage_url()` 断言。
  - 自包含扫描列表加入 `web_html` / `web_css` / `web_js`；旧 `usage_html` 变量改为同一 shell，避免重复扫描。
- [x] 修改 `src/settings/service/usage/tests.rs:387-393`：
  ```rust
  let port = service.port();
  let token = service.token().to_string();
  ```

- [x] 验收：
  ```bash
  cargo test settings::service
  ```

---

## 3. `web.html` 统一 shell

### T3.1 创建 `assets/web.html`

- [x] 以 `assets/settings.html` 为主体，按以下顺序拼装：
  1. `<head>`：`<title>lx-tui</title>`；`/web.css`；`/settings.css` + `media="not all" id="css-settings"`；`/usage.css` + `media="not all" id="css-usage"`。
  2. 顶部内联 SVG `<defs><g id="lx-logo-art">`（路径数据见 `design.md` §6.2）。
  3. `#toast-host`、`#chart-tooltip`。
  4. `<header class="web-topbar">`：品牌区 + `Home / Settings / Usage` 三个按钮。
  5. `#web-app` 内三个 `<section class="web-view">`：
     - `#view-home`：hero + 两张卡。
     - `#view-settings`：`.settings-shell` 包住原 `nav.side`（删除 `<h1>lx-tui</h1>`）、`side-resizer`、`<main class="settings-main">`（原 7 个 section）。
     - `#view-usage`：`.usage-shell` 包住 `.usage-controls` 与 `usage-main`；删除第二层 `usage-topbar` / `.header-titles` / `h1 USAGE` / subtitle，把时间范围、筛选与 `.header-actions` 合并为一条内容区工具条。
  6. 脚本顺序：
     ```html
     <script src="/web.js"></script>
     <script src="/settings.js"></script>
     <script src="/settings_models_card.js"></script>
     <script src="/settings_models.js"></script>
     <script src="/settings_connect.js"></script>
     <script src="/settings_mcp.js"></script>
     <script src="/settings_skills.js"></script>
     <script src="/settings_subagents.js"></script>
     <script src="/usage_charts.js"></script>
     <script src="/usage.js"></script>
     ```
- [x] 迁移时逐段核对所有 `id` / `class` / `onclick` 原样保留。
- [x] 删除 `assets/settings.html`、`assets/usage.html`。
- [x] 检查：
  ```bash
  rg "http://|https://|innerHTML" assets/web.html
  ```
  必须无结果（SVG 不写 `xmlns`）。

### T3.2 Home 文案与卡片

- [x] 副标题：`Local agent workspace · configuration and usage in one place`。
- [x] Settings 卡：`Models, providers, MCP servers, skills, and subagents.` / `Open settings →`。
- [x] Usage 卡：`Token usage, cost, and request history across model calls.` / `Open usage →`。
- [x] 卡片与顶部 tab 使用原生 hash 链接：`href="#/settings"`、`href="#/usage"`、`href="#/"`；品牌链接回 `#/`。不写内联 `onclick`，由 `hashchange` 驱动视图切换。

- [x] 验收：浏览器 Home 页不发起任何 `/api/*` 请求（DevTools Network 确认）。

---

## 4. `web.css`

### T4.1 创建 `assets/web.css`

- [x] 写入 `design.md` §6.3 的 `:root` 令牌（`--web-topbar-height: 40px`）、`html/body`、`.web-topbar`、`.web-brand`、`.web-tabs`、`#web-app`、`.web-view`、`.home-*`、`.settings-shell`。
- [x] 增加全局 `.hidden` 与统一滚动条（`*` + `::-webkit-scrollbar*`）；视图 CSS 不再重复定义。
- [x] toast host 定位到顶栏下方（`calc(var(--web-topbar-height) + 12px)`），不遮挡 tab。
- [x] 迁移共享反馈样式（从 `settings.css` / `usage.css` 删除，只保留这里一份）：
  - `.toast-host` / `.toast` / `.toast.ok` / `.toast.error` / `.toast.leaving`。
  - `@keyframes toast-in` / `@keyframes dot-pulse`。
- [x] **不迁移** `.select-*`：两套 CSS 的下拉菜单存在细节差异（Usage 选项 12px + 省略，Settings 继承 14px），保留在各自视图 CSS，避免视觉变化。
- [x] 增加 `body.resizing { cursor: col-resize; user-select: none; }`（Settings resizer 继续给 body 加类）。
- [x] 窄屏：
  ```css
  @media (max-width: 640px) {
    .home-cards { grid-template-columns: 1fr; }
    .home-logo { width: 140px; }
    .web-topbar { padding: 0 12px; }
    .web-tab { padding: 0 10px; }
  }
  ```

- [x] 验收：`rg "toast-host" assets/web.css` 命中，且 `.select-` 不在 `web.css`。

### T4.2 清理 `settings.css`

- [x] 删除 `:root` 中的基础令牌（保留 Settings 专属令牌，如有）。
- [x] 删除 `html, body { height: 100%; }` 与 `body { ... }`。
- [x] `body.resizing` 规则删除（已移入 `web.css`）。
- [x] `nav.side` 增加 `height: 100%; overflow-y: auto;`。
- [x] 删除 `nav.side h1` 规则。
- [x] `main { ... }` → `.settings-shell main { ... }`；复审后 shell 规则只保留在 `web.css`。
- [x] 删除 `.toast*` 与 `@keyframes toast-in` / `@keyframes dot-pulse`。
- [x] 删除 `.editor-body` / `.provider-list` / `.select-menu` / `.fetch-candidates` 的局部滚动条规则。
- [x] **保留** `.select-*` 块。
- [x] 删除 `* { box-sizing }`、普通 `.hidden` 与重复的 `.settings-shell main`。

### T4.3 清理 `usage.css`

- [x] 删除 `:root` 基础令牌与 `color-scheme`，保留 `--chart-*`、`--z-tooltip` 等 Usage 专属令牌。
- [x] 删除 `html, body`、`body { ... }`、`* { box-sizing }` 与全局 `.hidden`。
- [x] `.usage-shell { height: 100vh; }` → `height: 100%;`，保留 flex 列布局。
- [x] 删除 `.header-titles`、`.usage-header h1`、`.usage-header .subtitle`。
- [x] 删除 `.usage-topbar` / `.topbar-inner` / `.usage-toolbar` / `.usage-filters` / `.filter-range-row`。
- [x] 新增 `.usage-controls`（可换行工具条）与 `.header-actions { margin-left:auto; }`。
- [x] 删除 `.usage-main` / `.table-wrap` / `.select-menu` 的局部滚动条规则。
- [x] 删除 `.toast*` 与 `@keyframes toast-in` / `@keyframes dot-pulse`。
- [x] **保留** `.select-*` 块。

- [x] 验收：
  ```bash
  rg "height: 100vh|usage-topbar|usage-toolbar|usage-filters" assets/usage.css
  rg "scrollbar" assets/settings.css assets/usage.css
  ```
  第一条无结果；第二条只允许 `.command-list` 的隐藏滚动条规则。

---

## 5. `web.js` 共享核心与路由

### T5.1 创建 `assets/web.js`

- [x] 写入 `design.md` §6.5 的完整代码：
  - `token`。
  - `domEl`（从 `usage_charts.js` 迁入）。
  - `toastHost` / `showToast`。
  - `openSelects` / `createSelect`：
    - 以 `usage.js` 版本为基础（`aria-expanded`、宽度夹取、`replaceChildren`）。
    - `choose()` 末尾：
      ```javascript
      if (typeof refreshActionBars === 'function') { refreshActionBars(); }
      ```
  - `VIEWS` / `viewFromLocation` / `showView` / `initOnce` / `bootWeb`。
- [x] 确保 `showView`：
  - 切换 `.web-view.active`。
  - 切换 `.web-tab.active`。
  - 切换 `#css-settings` / `#css-usage` 的 `media`。
  - 首次进入调用 `window.initSettingsView` / `window.initUsageView`。
  - 更新 `document.title`。
- [x] `bootWeb`：
  - 旧 pathname 规范化为 `/?token=...#/<view>`。
  - `showView(initial)`。
  - 监听 `hashchange`。

- [x] 验收：
  ```bash
  rg "innerHTML" assets/web.js
  ```
  无结果。

### T5.2 清理 `usage_charts.js`

- [x] 删除 `function domEl(...)`（第 23-28 行）。
- [x] 确认所有 `domEl` 调用在 `web.js` 加载后执行。

### T5.3 清理 `settings.js`

- [x] 删除 `const token = ...`。
- [x] 删除 `const toastHost = ...` 与 `document.body.appendChild(toastHost)`。
- [x] 删除 `function showToast(...)`。
- [x] 删除 `const openSelects = ...`。
- [x] 删除 `function createSelect(...)`（整段）。
- [x] 文件尾初始化块包进：
  ```javascript
  function initSettingsView() {
    if (window.__settingsInitialized) { return; }
    window.__settingsInitialized = true;
    // SIDE_MIN_WIDTH / SIDE_MAX_WIDTH / LIST_MIN_WIDTH / LIST_MAX_WIDTH / cmdListWidth
    // setupResizer(...) ×3
    // actionBars.general / actionBars.commands
    // initLoggingSelect(); initCommandSelects(); loadState();
  }
  ```
- [x] 确保 `window.initSettingsView` 可被 `web.js` 访问（顶层函数声明即可）。
- [x] 删除文件末尾直接执行的 `loadState()`。

- [x] 验收：
  ```bash
  rg "^(const token|function showToast|const openSelects|function createSelect)" assets/settings.js
  ```
  无结果。

### T5.4 清理 `usage.js`

- [x] 删除 `const token = ...`。
- [x] 删除 `const openSelects = ...`。
- [x] 删除 `function showToast(...)`。
- [x] 删除 `function createSelect(...)`（整段）。
- [x] 文件尾：
  ```javascript
  function initUsageView() {
    if (window.__usageInitialized) { return; }
    window.__usageInitialized = true;
    initUsageDashboard();
  }
  ```
- [x] 删除文件末尾直接执行的 `initUsageDashboard();`。

- [x] 验收：
  ```bash
  rg "^(const token|function showToast|const openSelects|function createSelect)" assets/usage.js
  ```
  无结果。

### T5.5 重复全局终检

- [x] 扫描：
  ```bash
  for f in assets/web.js assets/settings*.js assets/usage*.js; do
    grep -oE '^(function|const|let|var) +[A-Za-z_$][A-Za-z0-9_$]*' "$f" | awk '{print $NF}' | sed "s|^|$(basename $f):|"
  done | sort -t: -k2 | awk -F: '{a[$2]=a[$2]" "$1} END {for (k in a) if (split(a[k],parts," ")>1) print k": "a[k]}'
  ```
  确认无重复声明（允许函数名在 `typeof` 守卫下重名的情况必须人工核对）。

---

## 6. 文档与技能指南

### T6.1 `docs/standards/html-assets.md`

- [x] 目录表新增：
  - `web.html`：统一 Web 控制台（Home / Settings / Usage 单文档）。
  - `web.css`：全局 bar、Home 与共享组件样式。
  - `web.js`：共享核心、hash 路由与懒初始化。
- [x] 说明 `settings.css` / `usage.css` 是视图级样式，按 `media="not all"` 挂起，同一时间只应用一套。
- [x] 更新脚本加载顺序：`web.js` → `settings.js` → `settings_models_card.js` → … → `usage_charts.js` → `usage.js`。
- [x] 说明旧 `/settings`、`/usage` 路径由同一 `web.html` 承接。
- [x] 更新"编辑与验证"中的页面测试位置说明。

### T6.2 `docs/standards/tui-design-requirements.md`

- [x] 把第 21 行中的：
  - 设置按钮 `[SET]` + 用量按钮 `[USE]`（最小宽度 17、两个 tooltip、两个页面）
  - 改为：
    - 统一按钮 `[WEB]`（纯 ASCII、逐格强调色、最小侧栏宽度 11、tooltip `open web`）。
    - 点击在系统默认浏览器打开统一 Web 控制台（Home 默认，顶部可切 Settings / Usage）。
    - `ui/` 不做 IO，仍由事件层打开。

### T6.3 `src/skills/assets/lx-guide/references/commands-and-mentions.md`

- [x] 第 81 行：
  - `[SET]` → `[WEB]`。
  - 说明打开的是统一 Web 控制台，包含 Home / Settings / Usage。

- [x] 验收：
  ```bash
  rg "\[SET\]|\[USE\]" docs/standards src/skills/assets
  ```
  无结果（历史 `docs/feature/*` 不改）。

---

## 7. 验证与收尾

### T7.1 自动验证

- [x] `cargo fmt`
- [x] `cargo clippy --all-targets -- -D warnings`
- [x] `cargo test`

### T7.2 定向测试

- [x] `cargo test ui::controls`
- [x] `cargo test event::tests::mouse_click`
- [x] `cargo test settings::service`
- [x] `cargo test settings::service::usage`
- [x] `cargo test --test web_assets`

### T7.3 浏览器手工验收（用户执行 `cargo run`）

- [x] 侧栏底边框只有 `[WEB]`，hover 显示 `open web`。
- [x] 点击后默认 Home，LX 像素 logo 清晰、无外链。
- [x] 顶部 `Home / Settings / Usage` 切换正常。
- [x] Settings 7 个页签、保存/重置/toast 正常；左栏无重复 `lx-tui` 标题。
- [x] Usage 筛选、图表、表格、分页正常；顶部无重复 `USAGE` 大标题。
- [x] 直接访问 `/settings`、`/usage` 落在正确视图并规范化 URL。
- [x] 浏览器前进/后退可切换视图。
- [x] 窄窗口布局无溢出。
- [x] DevTools Console 零 `SyntaxError` / 零运行时错误；Network 无外部域名。

### T7.4 清理

- [x] 确认 `assets/settings.html`、`assets/usage.html` 已删除。
- [x] 确认无 `[SET]` / `[USE]` 残留（历史 docs 除外）。
- [x] 确认 `usage_url()`、`usage_html()`、`settings_open_requested`、`usage_open_requested`、`SidebarSettings`、`SidebarUsage` 全部删除。
- [x] 已按项目 Git 规范提交：`7cb05db5`（统一控制台）、`a7f67eef`（脚本语法回归测试）、`a3acca2d`（紧凑顶栏与 Usage 工具条）。

### T7.5 复审修复（`review.md`）

- [x] R1：新增 `SettingsService::settings_url()`；`lx-tui settings open` 与 JSON `url` 直达 `#/settings`。
- [x] R2：toast 移到顶栏下方，不再遮挡 / 吞掉 tab 点击。
- [x] R3：Usage 自动刷新在视图非 active 时跳过请求与错误 toast。
- [x] R4：删除视图内层 `__*Initialized` 守卫；`bootWeb` 先注册 `hashchange` 再初始化，失败可重试。
- [x] R5：三路由响应与 `page::html()` 字节一致；补 Usage DOM 挂载点断言。
- [x] R6：`tests/web_assets.rs` 按 HTML 顺序拼接脚本后 `node --check`，并校验 `getElementById` 静态 id。
- [x] R7：同步 `design.md` / `task.md` / `html-assets.md` / `project-directory-structure.md`。
- [x] R8：分页以响应 `page` 为准；超出总页数时回到最后一页。
- [x] R9：文档改为统一 `Cache-Control: no-store`。
- [x] R10：删除失效 `btn-empty-new-provider` 查找；MCP / Skills / Subagents resizer 接线并补 id。
- [x] R11：不可用提示改为 “Web console unavailable”。
- [x] R12：清理重复 `color-scheme` / `box-sizing` / `.hidden` / shell 规则、`element()`、`currentView`、无效 `MAX_HEADER_BYTES` 语句。
- [x] R13：顶栏 tab 切换补 `aria-current="page"`。
- [x] 复审验证：`cargo fmt`、`cargo clippy --all-targets -- -D warnings`、`cargo test`、浏览器 1448 / 900 / 640 宽度回归。

### T7.6 合并与清理

- [x] `feat/web-console-merge` 以 `--no-ff` 合并到 `dev`（merge `bc5d6f3a`），无冲突。
- [x] 合并后在主工作区重跑 `cargo fmt --check`、`cargo clippy --all-targets -- -D warnings`、`cargo test`。
- [x] 更新 `design.md` / `task.md` / `review.md` 的合并状态并提交。
- [x] 删除 worktree `.worktrees/web-console-merge`、分支 `feat/web-console-merge` 与隔离库 `~/.lx-t-dev/db/worktrees/web-console-merge-1472ed18/`。

---

## 8. 完成定义（DoD）

1. 侧栏只有一个 `[WEB]` 入口，点击默认打开 Home。
2. `web.html` 是唯一 HTML 文档，顶部 bar 可切 Home / Settings / Usage。
3. Home 展示 LX 像素 logo 与两张入口卡，且不发 API 请求。
4. Settings / Usage 原有功能零回归，旧 URL 仍可直达对应视图。
5. `cargo fmt`、`cargo clippy --all-targets -- -D warnings`、`cargo test` 全部通过。
6. 浏览器控制台无报错，网络无外部请求。
7. 现行规范与技能指南已同步，历史 `docs/feature/*` 未改动。
