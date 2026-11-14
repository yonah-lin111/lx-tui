# 统一 Web 控制台（`[WEB]` 入口 + Home / Settings / Usage 单页）— 设计文档

> 状态：**已实施**（分支 `feat/web-console-merge`，待合并审核）。
> 任务清单：[task.md](task.md)
> 日期：2026-11-14
> 执行位置：主工作区 `/Users/yonah/projects/agent/lx-tui`
> 适用规范：`docs/standards/html-assets.md`、`docs/standards/tui-design-requirements.md`

---

## 0. 背景与目标

### 0.1 现状

TUI 左侧栏底边框有两个独立按钮：

- `[SET]`：打开 `SettingsService::url()`（`GET /?token=...`，返回 `assets/settings.html`）。
- `[USE]`：打开 `SettingsService::usage_url()`（`GET /usage?token=...`，返回 `assets/usage.html`）。

两个页面各自维护一套 HTML / CSS / JS，存在以下问题：

1. **入口分裂**：两个按钮、两个地址，用户需要先判断"设置"还是"用量"。
2. **品牌与标题重复**：Settings 左栏有 `lx-tui` 标题，Usage 顶部有 `USAGE` 大标题；两页各自承担品牌展示。
3. **全局命名冲突**：`settings.js` 与 `usage.js` 同时定义 `token`、`showToast`、`createSelect`、`openSelects`，直接合并进同一个文档会抛 `SyntaxError`（重复 `const` 声明）。
4. **CSS 通用选择器冲突**：两套 CSS 都定义 `body` / `main` / `h2` / `button` / `input` / `table` / `.hidden` / `.status` / `.toast` / `.select-*`，同时生效会互相污染。
5. **缺少首页**：点击入口后直接进入设置页，没有一个包含项目 logo 的默认落地页。

### 0.2 目标

1. **单一入口**：侧栏底边框只保留一个 `[WEB]` 按钮，tooltip 为 `open web`；点击后打开统一的 Web 控制台。
2. **统一页面**：`settings.html` 与 `usage.html` 合并为一个 `web.html`，顶部全局 bar 切换 `Home` / `Settings` / `Usage`。
3. **首页**：新增 Home 视图，默认打开，展示项目 LX 像素 logo、字标、副标题与 Settings / Usage 两张入口卡。
4. **零外部依赖**：继续满足 `assets/` 零外链、零构建步骤、`include_str!` 编译进二进制。
5. **功能零回归**：Settings 的 7 个二级页签、Usage 的筛选/图表/表格/分页行为保持不变。

### 0.3 非目标

- 不修改任何 `/api/*` 契约、数据库结构、配置格式。
- 不新增 Rust 依赖、前端依赖或构建步骤。
- 不改造 Settings / Usage 的业务逻辑与数据流。
- 不引入登录、远程访问或非回环监听。
- 不改历史 `docs/feature/*` 记录（只更新现行规范与技能指南）。

---

## 1. 现状核实（写设计前已逐项确认）

| 事实 | 代码位置 | 对本设计的影响 |
|---|---|---|
| `[SET]` / `[USE]` 位于侧栏**底边框**左端，不是一个按钮 | `src/ui/controls/footer.rs:1-111` | 合并为一个 `[WEB]` 按钮，几何与命中函数随之收敛 |
| 按钮文案为纯 ASCII 常量 | `src/ui/text/workspace.rs:530-534` | 改为 `WEB_LABEL: &str = "[WEB]"`，宽度仍为 5 列 |
| 渲染调用两次 | `src/ui/mod.rs:87-88` | 改为一次 `controls::render_web_button(...)` |
| Tooltip 目标与文案分别为两个 | `src/app/tooltip.rs:62-65`、`src/ui/tooltip.rs:125-130`、`src/ui/text.rs:877-878` | 合并为 `TooltipTarget::SidebarWeb` + `"open web"` |
| 点击只置位状态，事件层统一开浏览器 | `src/event/mouse/press.rs:215-224`、`src/app/state.rs:357-362`、`src/event/mod.rs:345-369` | 两个请求位合并为 `web_open_requested` |
| 服务地址：`url()` = `/?token=`，`usage_url()` = `/usage?token=` | `src/settings/service.rs:323-335` | 保留 `url()`，删除 `usage_url()` |
| 路由：`GET /` 返回 settings，`GET /usage` 返回 usage | `src/settings/service/handlers.rs:241-245` | 三个路径统一返回 `web.html`，由前端决定初始视图 |
| 页面资源表按白名单登记 | `src/settings/service/page.rs:9-68` | 新增 `/web.css`、`/web.js`；保留 settings/usage 资源 |
| 资源规模：settings.html 646 行、settings.css 243 行、settings.js 989 行、usage.html 157 行、usage.css 734 行、usage.js 903 行、usage_charts.js 831 行 | `wc -l assets/*` | 不能把两套 JS 原样拼在一个文档；必须消重与懒初始化 |
| 重复全局：`token`、`showToast`、`createSelect`、`openSelects` | `assets/settings.js:1,54,66,67`、`assets/usage.js:4,26,28,41` | 统一迁移到 `web.js`，两页脚本删除本地定义 |
| `domEl` 定义在 `usage_charts.js` | `assets/usage_charts.js:23-28` | 迁移到 `web.js`，`usage_charts.js` 删除本地定义 |
| Settings CSS：`body { display:flex }`、`main { flex:1... }`、`.hidden` | `assets/settings.css:38,50,243` | 外层 shell 改为 `.settings-shell`；body 布局移到 `web.css` |
| Usage CSS：`.usage-shell { height:100vh }`、`.usage-header` 标题行、`.hidden` | `assets/usage.css:51-58,97-119,719` | 改为 `height:100%`；删除标题行；视图切换用 `.web-view.active`，不依赖 `.hidden` |
| TUI 测试锁死两个按钮的几何、阈值 17、tooltip | `src/ui/controls/tests.rs:43-227`、`src/event/tests/mouse_click.rs:883-931` | 全部改为单按钮 `[WEB]`、阈值 11 |
| HTTP 测试断言 `usage_url()` 与两个页面资源 | `src/settings/service/tests.rs:44-290`、`src/settings/service/usage/tests.rs:387-393` | 改为统一 shell + `service.token()` |
| 现行规范与技能指南仍写 `[SET]` | `docs/standards/tui-design-requirements.md:21`、`src/skills/assets/lx-guide/references/commands-and-mentions.md:81` | 同步改为 `[WEB]` 与统一 Web 控制台 |

---

## 2. 已确认决策（Grill Me 收口）

| 编号 | 决策维度 | 已确认结论 |
|---|---|---|
| **D1** | 入口单词 | `[WEB]`。TUI 标签、顶部栏品牌入口、文档统一使用 `Web` / `[WEB]`；后续所有 HTML 页面都从这个入口进入。 |
| **D2** | 旧地址兼容 | `/`、`/settings`、`/usage` 都返回同一个 `web.html`；前端按 `location.pathname` 决定初始视图，进入后顶部 bar 用 hash 切换（`#/`、`#/settings`、`#/usage`）。旧地址是"后门"，新入口是"正门"，两边到达同一套页面。 |
| **D3** | 首页 logo | 复用 TUI 现有 LX 像素字标（`src/ui/main_content/lx/brand.rs` 的 `ICON_LX`，20×14 像素），首页用内联 SVG 按原像素网格重绘；配色映射为固定 hex（`k` `#00005f`、`p` `#ffafd7`、`n` `#d75f87`、`b` `#87d7ff`、`c` `#d7ffff`），不引入图片文件或外链。 |
| **D4** | 顶部 bar | 左侧品牌区（缩小版 LX 图标 + `lx-tui` 字标，点击回 Home），右侧三个 tab：`Home` / `Settings` / `Usage`。Settings 保留现有左侧 7 个二级页签；Usage 保留现有筛选栏作为二级栏。 |
| **D5** | 首页内容 | 居中 hero（大号 LX 像素 logo + `lx-tui` 字标 + 一行副标题），下方两张等宽入口卡：`Settings` 与 `Usage`；卡片可点击，效果等同顶部 tab。首页纯静态，不发任何 API 请求。 |
| **D6** | 视觉调整 | 删除 Settings 左栏原有的 `lx-tui` 标题；删除 Usage 顶部原有的 `USAGE` 大标题与副标题，只保留状态、自动刷新、Refresh 与筛选栏。表单、图表、按钮、分组、滚动区的位置和样式一律不动。 |
| **D7** | CSS 组织 | 保留 `settings.css` / `usage.css` 的主体不动，新增 `web.css` 负责全局 bar、Home 与视图容器；三个 `<link>` 同时引入，非当前视图的样式表用 `media="not all"` 挂起，切换视图时只改 `media` 属性，不销毁节点、不重新请求。 |
| **D8** | 默认落点 | `[WEB]` 每次点击都固定落 Home（`/?token=...`，不带 hash）；只有用户手动保留的 `/settings`、`/usage` 旧地址或页面内 hash 才直达对应视图。不做"记住上次视图"。 |

---

## 3. 总体架构

```text
┌──────────────────────────────────────────────────────────────────────────────┐
│ TUI 侧                                                                       │
│                                                                              │
│  ui::controls::render_web_button        ──► 绘制侧栏底边框 `╰[WEB]────[◀]╯` │
│  ui::tooltip::targets                   ──► Hover 命中 SidebarWeb            │
│                                              显示 `open web`                  │
│  event::mouse::press                    ──► 置位 state.web_open_requested    │
│  event::run_loop                        ──► 消费请求，后台 open_browser(      │
│                                              SettingsService::url())          │
└──────────────────────────────────────────────────────────────────────────────┘
                                      │
                                      ▼
┌──────────────────────────────────────────────────────────────────────────────┐
│ 本地回环服务（SettingsService，127.0.0.1:0，capability token）                │
│                                                                              │
│  GET /          ┐                                                           │
│  GET /settings  ├──► 200 text/html，统一返回 assets/web.html                 │
│  GET /usage     ┘                                                           │
│                                                                              │
│  GET /web.css         text/css        （全局 shell / Home / 共享组件）        │
│  GET /web.js          text/javascript （共享核心 + 路由 + 懒初始化）          │
│  GET /settings.css    text/css        （仅 Settings 视图激活）                │
│  GET /settings*.js    text/javascript （Settings 业务脚本）                   │
│  GET /usage.css       text/css        （仅 Usage 视图激活）                   │
│  GET /usage*.js       text/javascript （Usage 业务脚本）                      │
└──────────────────────────────────────────────────────────────────────────────┘
                                      │
                                      ▼
┌──────────────────────────────────────────────────────────────────────────────┐
│ web.html 单文档                                                              │
│                                                                              │
│  ┌────────────────────────────────────────────────────────────────────────┐  │
│  │ web-topbar: [LX] lx-tui                 Home | Settings | Usage       │  │
│  ├────────────────────────────────────────────────────────────────────────┤  │
│  │ #view-home     （默认，纯静态 hero + 两张卡）                           │  │
│  │ #view-settings （.settings-shell: nav.side + side-resizer + main）      │  │
│  │ #view-usage    （.usage-shell: usage-topbar + usage-main）              │  │
│  └────────────────────────────────────────────────────────────────────────┘  │
│                                                                              │
│  web.js                                                                      │
│    ├─ token / showToast / createSelect / openSelects / domEl（唯一来源）     │
│    ├─ 路由：#/ → Home， #/settings → Settings， #/usage → Usage              │
│    ├─ CSS 激活：settings.css / usage.css 的 media 切换                       │
│    └─ 懒初始化：首次进入 Settings/Usage 才调用 initSettingsView/initUsageView│
└──────────────────────────────────────────────────────────────────────────────┘
```

### 3.1 路由矩阵

| 浏览器地址 | 初始视图 | 激活 CSS | 首次进入初始化 | 备注 |
|---|---|---|---|---|
| `/?token=...` | Home | 仅 `web.css` | 无 | `[WEB]` 固定打开这里 |
| `/?token=...#/settings` | Settings | `web.css` + `settings.css` | `initSettingsView()` | 顶部 tab / 首页卡 |
| `/?token=...#/usage` | Usage | `web.css` + `usage.css` | `initUsageView()` | 顶部 tab / 首页卡 |
| `/settings?token=...` | Settings | 同上 | 同上 | 旧地址；启动时重写为 `/?token=...#/settings` |
| `/usage?token=...` | Usage | 同上 | 同上 | 旧地址；启动时重写为 `/?token=...#/usage` |
| 未知 hash（如 `#/foo`） | Home | 仅 `web.css` | 无 | 不报错，回落 Home |

### 3.2 视图切换时序

```text
bootWeb()
  ├─ 从 location.search 读取 token（web.js 顶部）
  ├─ 从 location.pathname 读取旧地址（/settings、/usage）
  ├─ 从 location.hash 读取 #/settings、#/usage
  ├─ 若 pathname 是旧地址：history.replaceState 到 /?token=...#/<view>
  ├─ showView(initial, { canonicalize:false })
  │    ├─ 切 topbar active
  │    ├─ 切 .web-view.active
  │    ├─ 切 settings.css / usage.css 的 media
  │    ├─ 首次进入时 initOnce('settings'|'usage')
  │    └─ 更新 document.title
  └─ 监听 hashchange → showView(viewFromLocation())
```

---

## 4. TUI 侧设计

### 4.1 常量与几何

`src/ui/text/workspace.rs`：

```rust
/// 侧栏底边框统一 Web 控制台入口按钮。
/// 纯 ASCII，宽度稳定、无 emoji 呈现；后续所有 HTML 页面都从这里进入。
pub const WEB_LABEL: &str = "[WEB]";
```

`src/ui/controls/footer.rs`：

```rust
/// 侧栏底边框 `[WEB]` 按钮几何：展开态且宽度足够时贴靠左边框（与右端折叠按钮同列）。
/// 折叠态、过窄或服务不可用时隐藏；渲染与鼠标命中共用。
pub fn web_button(view: &layout::ViewLayout, state: &AppState) -> Option<Rect> {
    let area = view.left_panel;
    if !state.settings_available
        || state.sidebar_collapsed
        || area.width < WEB_BUTTON_MIN_WIDTH
        || area.height < 2
    {
        return None;
    }
    Some(Rect::new(
        area.x + WEB_BUTTON_MARGIN,
        area.bottom().saturating_sub(1),
        WEB_BUTTON_WIDTH,
        1,
    ))
}

pub fn web_button_at(view: &layout::ViewLayout, state: &AppState, column: u16, row: u16) -> bool {
    web_button(view, state).is_some_and(|area| area.contains((column, row).into()))
}

const WEB_BUTTON_WIDTH: u16 = text::WEB_LABEL.len() as u16; // 5
const WEB_BUTTON_MARGIN: u16 = 1;
/// 按钮 + 右端折叠按钮与其边距的安全余量：5 + 3 + 1 + 2 = 11。
const WEB_BUTTON_MIN_WIDTH: u16 = WEB_BUTTON_WIDTH + 3 + 1 + 2;
```

绘制沿用现有逐格 `reset + accent` 方式：

```rust
pub(crate) fn render_web_button(
    frame: &mut Frame<'_>,
    view: &layout::ViewLayout,
    state: &AppState,
) {
    let Some(area) = web_button(view, state) else {
        return;
    };
    for (offset, symbol) in text::WEB_LABEL.chars().enumerate() {
        let x = area.x + offset as u16;
        if let Some(cell) = frame.buffer_mut().cell_mut((x, area.y)) {
            cell.reset();
            cell.set_char(symbol);
            cell.set_style(style::accent());
        }
    }
}
```

**关键点**：

- `[WEB]` 与旧 `[SET]` 同为 5 列，按钮 x 坐标不变（`area.x + 1`）。
- 最小侧栏宽度门槛从旧 `[USE]` 的 17 降为 11：宽度 11–16 时只显示 `[WEB]`，不再有第二个按钮。
- `[WEB]` 右侧至折叠按钮之间应全部是底边框线 `─`，不得有残留点击区或 tooltip 目标。

### 4.2 状态、事件与 tooltip

`src/app/state.rs`：

```rust
/// 本地 Web 控制台服务是否可用；决定底边框 `[WEB]` 按钮的可用态。
pub settings_available: bool,
/// `[WEB]` 点击请求：事件层取走后打开浏览器。
pub web_open_requested: bool,
```

初始化时 `web_open_requested: false`，替换原 `settings_open_requested` / `usage_open_requested` 两处。

`src/event/mouse/press.rs`：

```rust
if ui::web_button_at(view, state, mouse.column, mouse.row) {
    state.web_open_requested = true;
    *dirty = true;
    return;
}
```

`src/event/mod.rs`：

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
        tracing::warn!("settings web service is unavailable");
        update::show_toast(
            state,
            crate::app::toast::Toast::new(
                /* 现有不可用提示文案保持不变 */
            ),
        );
    }
}
```

`src/app/tooltip.rs`：

```rust
/// 侧栏底边框统一 Web 控制台入口 `[WEB]`。
SidebarWeb,
```

`src/ui/text.rs`：

```rust
TooltipTarget::SidebarWeb => "open web",
```

`src/ui/tooltip.rs`：

```rust
if let Some(area) = ui::web_button(view, state) {
    targets.push((TooltipTarget::SidebarWeb, area));
}
```

`src/ui/mod.rs`：

```rust
controls::render_web_button(frame, &view, state);
```

**关键点**：

- `ui/` 仍然只读状态、只写 Buffer；不执行任何 IO。
- 两个旧请求位与两个旧 tooltip 目标必须彻底删除，避免死代码与重复按钮。

### 4.3 TUI 测试改造

`src/ui/controls/tests.rs` 由 4 条测试改为：

1. `web_button_renders_and_hits`
   - `web_button` 返回 `Rect { x: view.left_panel.x + 1, y: bottom - 1, width: 5, height: 1 }`。
   - 按钮 5 列全部命中 `web_button_at`；左右相邻列不命中。
   - 渲染文本等于 `text::WEB_LABEL`。
   - 折叠态、`settings_available = false` 时隐藏。

2. `web_button_width_threshold_is_eleven`
   - 宽度 10：隐藏。
   - 宽度 11：恰好显示并命中。
   - 不再存在 17 阈值。

3. `sidebar_bottom_border_has_web_button`
   - `[WEB]` 右侧至折叠按钮之间全为 `─`。
   - 断言不再出现 `[SET]` / `[USE]`。

4. `tooltip_target_points_at_web_button`
   - `text::tooltip_hint(TooltipTarget::SidebarWeb, &state) == "open web"`。
   - 按钮 5 列命中 `Some(TooltipTarget::SidebarWeb)`。
   - 按钮右侧至折叠按钮之间无 tooltip 目标。

`src/event/tests/mouse_click.rs`：

```rust
#[test]
fn mouse_down_on_web_button_requests_browser_open() {
    let web_btn = ui::web_button(&geo.view, &state).expect("[WEB] visible");
    handle_terminal_event(mouse(MouseEventKind::Down(MouseButton::Left), web_btn.x, web_btn.y), ...);
    assert!(state.web_open_requested);
    assert!(dirty);
}
```

---

## 5. 服务端设计

### 5.1 路由

`src/settings/service/handlers.rs`：

```rust
match (request.method.as_str(), request.path.as_str()) {
    ("GET", "/") | ("GET", "/settings") | ("GET", "/usage") => {
        Response::ok("text/html; charset=utf-8", page::html().to_string())
    }
    // ... 其余路由保持不变
}
```

**关键点**：

- 三个路径都返回同一份 `web.html`，由前端 `web.js` 读取 `location.pathname` / `location.hash` 决定初始视图。
- `/api/usage/*` 路由、token 校验、同源写校验全部不变。

### 5.2 `page.rs`

```rust
/// 统一 Web 控制台正文：Home / Settings / Usage 三视图在同一个文档内切换。
pub fn html() -> &'static str {
    include_str!("../../../assets/web.html")
}

pub fn asset(path: &str) -> Option<(&'static str, &'static str)> {
    match path {
        "/web.css" => Some((
            "text/css; charset=utf-8",
            include_str!("../../../assets/web.css"),
        )),
        "/web.js" => Some((
            "text/javascript; charset=utf-8",
            include_str!("../../../assets/web.js"),
        )),
        // settings.css / settings*.js / usage.css / usage*.js 保持原样
        _ => None,
    }
}
```

**关键点**：

- 删除 `usage_html()`；不再单独嵌入 `usage.html`。
- `settings.html`、`usage.html` 两个旧文件在任务完成后删除（内容已迁入 `web.html`），避免"三份真相"。
- 资源白名单仍免 token；页面与 API 仍逐请求校验 token。

### 5.3 `SettingsService`

```rust
/// 浏览器访问统一 Web 控制台 URL：`http://127.0.0.1:<port>/?token=<token>`。
pub fn url(&self) -> String {
    format!("http://127.0.0.1:{}/?token={}", self.port, self.token())
}
```

删除 `usage_url()` 及其文档注释。

### 5.4 HTTP 测试

`src/settings/service/tests.rs`：

- `/` 无 token → 403；带 token → 200。
- `GET /settings?token=...`、`GET /usage?token=...` 都 → 200，且返回同一份 shell 标记。
- 页面断言继续覆盖 Settings 的原有标记（`General`、`Commands`、`view-model`、`default-model-host`、`MCP`、`view-skills` 等），因为它们仍在 `web.html` 的 `#view-settings` 内。
- 新增断言：
  - `html.contains("id=\"view-home\"")`
  - `html.contains("id=\"view-settings\"")`
  - `html.contains("id=\"view-usage\"")`
  - `html.contains("/web.css")`、`html.contains("/web.js")`
  - `html.contains("/settings.css")`、`html.contains("/usage.css")`
- 删除 `service.usage_url()` 断言；改为断言 `service.url()` 仍为 `/?token=`。
- 自包含扫描列表加入 `web_html`、`web_css`、`web_js`，移除重复的 `usage_html`（同一份 shell 只扫一次）。

`src/settings/service/usage/tests.rs`：

```rust
let port = service.port();
let token = service.token().to_string();
```

不再从 `usage_url()` 字符串里切 token。

---

## 6. 前端设计

### 6.1 `web.html` 结构

```html
<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>lx-tui</title>
<link rel="stylesheet" href="/web.css">
<link rel="stylesheet" href="/settings.css" media="not all" id="css-settings">
<link rel="stylesheet" href="/usage.css" media="not all" id="css-usage">
</head>
<body>
<!-- 像素 logo 定义：首页 hero 与顶部品牌区共用 -->
<svg class="lx-art" width="0" height="0" aria-hidden="true" focusable="false">
  <defs>
    <g id="lx-logo-art">
      <path fill="#00005f" d="..."/>
      <path fill="#ffafd7" d="..."/>
      <path fill="#d75f87" d="..."/>
      <path fill="#87d7ff" d="..."/>
      <path fill="#d7ffff" d="..."/>
    </g>
  </defs>
</svg>

<div id="toast-host" class="toast-host" aria-live="polite"></div>
<div id="chart-tooltip" class="chart-tooltip" role="tooltip" hidden></div>

<header class="web-topbar">
  <a class="web-brand" href="#/" aria-label="lx-tui home">
    <svg class="web-brand-logo" viewBox="0 0 20 14" aria-hidden="true"><use href="#lx-logo-art"/></svg>
    <span>lx-tui</span>
  </a>
  <nav class="web-tabs" aria-label="Web console">
    <a class="web-tab active" data-view="home" href="#/">Home</a>
    <a class="web-tab" data-view="settings" href="#/settings">Settings</a>
    <a class="web-tab" data-view="usage" href="#/usage">Usage</a>
  </nav>
</header>

<div id="web-app">
  <section id="view-home" class="web-view active" aria-label="Home">
    <div class="home-shell">
      <div class="home-hero">
        <svg class="home-logo" viewBox="0 0 20 14" role="img" aria-label="lx-tui logo"><use href="#lx-logo-art"/></svg>
        <h1>lx-tui</h1>
        <p>Local agent workspace · configuration and usage in one place</p>
      </div>
      <div class="home-cards">
        <a class="home-card" href="#/settings">
          <span class="home-card-title">Settings</span>
          <span class="home-card-desc">Models, providers, MCP servers, skills, and subagents.</span>
          <span class="home-card-action">Open settings →</span>
        </a>
        <a class="home-card" href="#/usage">
          <span class="home-card-title">Usage</span>
          <span class="home-card-desc">Token usage, cost, and request history across model calls.</span>
          <span class="home-card-action">Open usage →</span>
        </a>
      </div>
    </div>
  </section>

  <section id="view-settings" class="web-view" aria-label="Settings">
    <div class="settings-shell">
      <nav class="side">
        <!-- 原来的 7 个二级页签；删除 <h1>lx-tui</h1> -->
        <button id="tab-general" class="active" onclick="showTab('general')">General</button>
        <!-- Commands / Model / Provider / MCP / Skills / Subagents -->
      </nav>
      <div class="side-resizer" id="side-resizer" role="separator" aria-orientation="vertical" aria-label="Resize sidebar"></div>
      <main class="settings-main">
        <!-- 原 settings.html 的 7 个 <section class="view"> 原样迁入 -->
      </main>
    </div>
  </section>

  <section id="view-usage" class="web-view" aria-label="Usage">
    <div class="usage-shell">
      <header class="usage-topbar">
        <div class="topbar-inner">
          <!-- 删除 header-titles / h1 USAGE / subtitle -->
          <div class="usage-toolbar">
            <div class="header-actions">
              <!-- usage-status / auto-refresh / refresh-btn 原样保留 -->
            </div>
          </div>
          <section class="usage-filters" aria-label="Usage filters">
            <!-- 原 usage.html 的筛选栏原样迁入 -->
          </section>
        </div>
      </header>
      <main class="usage-main">
        <!-- 原 usage.html 的 usage-content 原样迁入 -->
      </main>
    </div>
  </section>
</div>

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
</body>
</html>
```

**关键点**：

- 所有 `id` 保持不变（`view-general`、`actions-general`、`side-resizer`、`custom-start`、`filter-provider`、`chart-trend`、`table-container` 等），业务 JS 不需要改选择器。
- `#view-usage` 与 `#view-settings` 用 `.web-view.active` 控制显示，不再依赖 `.hidden`，避免和两套页面内部的 `.hidden` 规则混淆。
- `web.js` 必须第一个加载：它先定义 `token` / `showToast` / `createSelect` / `openSelects` / `domEl` 与路由函数；页面脚本后续只定义业务函数，不立即发请求。
- 首页不引用任何图表或设置 DOM。

### 6.2 首页与像素 logo

#### 首页布局

- `.home-shell`：`height:100%; display:flex; flex-direction:column; align-items:center; justify-content:center; gap:40px; padding:48px 24px; overflow-y:auto;`
- `.home-hero`：居中列，`gap:12px`。
- `.home-logo`：`width:200px; height:auto; image-rendering:pixelated;`（20×14 像素，SVG 等比缩放）。
- `.home-hero h1`：`font-size:32px; letter-spacing:.06em;`，`lx-tui` 字标。
- `.home-hero p`：`color:var(--muted); font-size:13px;`。
- `.home-cards`：`display:grid; grid-template-columns:repeat(2, minmax(0,1fr)); gap:16px; width:min(760px,100%);`
- `.home-card`：`display:flex; flex-direction:column; gap:8px; text-align:left; padding:20px; border:1px solid var(--border-strong); background:var(--panel); color:var(--text); text-decoration:none; cursor:pointer;`
- `.home-card-title`：`font-size:16px; font-weight:600;`
- `.home-card-desc`：`color:var(--muted); font-size:13px;`
- `.home-card-action`：`color:var(--text); font-size:12px; margin-top:auto;`
- `.home-card:hover`：`border-color:var(--accent); background:var(--panel-raised);`
- 窄屏 `@media (max-width: 640px)`：卡片单列，logo `width:140px`。

#### logo SVG（关键代码）

在 `web.html` 顶部定义一次，hero 与 topbar 通过 `<use href="#lx-logo-art">` 复用。以下 `d` 由 `ICON_LX` 逐像素横向游程生成，**形状与 TUI 完全一致**：

```html
<svg class="lx-art" width="0" height="0" aria-hidden="true" focusable="false">
  <defs>
    <g id="lx-logo-art">
      <path fill="#00005f" d="M1 0h3v1h-3zM7 0h5v1h-5zM16 0h4v1h-4zM0 1h1v1h-1zM4 1h1v1h-1zM7 1h1v1h-1zM11 1h1v1h-1zM15 1h1v1h-1zM19 1h1v1h-1zM0 2h1v1h-1zM4 2h1v1h-1zM7 2h1v1h-1zM11 2h1v1h-1zM15 2h1v1h-1zM19 2h1v1h-1zM0 3h1v1h-1zM4 3h1v1h-1zM7 3h1v1h-1zM11 3h1v1h-1zM14 3h1v1h-1zM19 3h1v1h-1zM0 4h1v1h-1zM4 4h1v1h-1zM7 4h2v1h-2zM13 4h1v1h-1zM18 4h1v1h-1zM0 5h1v1h-1zM4 5h1v1h-1zM8 5h1v1h-1zM12 5h1v1h-1zM17 5h1v1h-1zM0 6h1v1h-1zM4 6h1v1h-1zM9 6h1v1h-1zM11 6h1v1h-1zM16 6h1v1h-1zM0 7h1v1h-1zM4 7h1v1h-1zM9 7h1v1h-1zM11 7h1v1h-1zM15 7h1v1h-1zM0 8h1v1h-1zM4 8h1v1h-1zM9 8h1v1h-1zM14 8h1v1h-1zM17 8h1v1h-1zM0 9h1v1h-1zM4 9h1v1h-1zM8 9h1v1h-1zM13 9h1v1h-1zM17 9h1v1h-1zM0 10h1v1h-1zM4 10h1v1h-1zM7 10h1v1h-1zM12 10h1v1h-1zM18 10h1v1h-1zM0 11h1v1h-1zM4 11h1v1h-1zM7 11h1v1h-1zM12 11h2v1h-2zM15 11h1v1h-1zM19 11h1v1h-1zM0 12h1v1h-1zM6 12h1v1h-1zM10 12h2v1h-2zM15 12h1v1h-1zM19 12h1v1h-1zM0 13h11v1h-11zM16 13h4v1h-4z"/>
      <path fill="#ffafd7" d="M1 1h2v1h-2zM16 1h3v1h-3zM1 2h2v1h-2zM16 2h2v1h-2zM1 3h2v1h-2zM15 3h2v1h-2zM1 4h2v1h-2zM14 4h2v1h-2zM1 5h2v1h-2zM13 5h2v1h-2zM1 6h2v1h-2zM12 6h2v1h-2zM1 7h2v1h-2zM12 7h2v1h-2zM1 8h2v1h-2zM10 8h2v1h-2zM16 8h1v1h-1zM1 9h2v1h-2zM9 9h2v1h-2zM16 9h1v1h-1zM1 10h2v1h-2zM8 10h2v1h-2zM17 10h1v1h-1zM1 11h2v1h-2zM5 11h2v1h-2zM8 11h3v1h-3zM7 12h1v1h-1z"/>
      <path fill="#d75f87" d="M18 2h1v1h-1zM17 3h2v1h-2zM16 4h2v1h-2zM15 5h2v1h-2zM14 6h2v1h-2zM14 7h1v1h-1zM12 8h2v1h-2zM15 8h1v1h-1zM11 9h2v1h-2zM14 9h2v1h-2zM10 10h2v1h-2zM13 10h4v1h-4zM11 11h1v1h-1zM16 11h3v1h-3zM8 12h2v1h-2zM16 12h3v1h-3z"/>
      <path fill="#87d7ff" d="M3 1h1v1h-1zM3 2h1v1h-1zM8 2h3v1h-3zM3 3h1v1h-1zM8 3h3v1h-3zM3 4h1v1h-1zM9 4h4v1h-4zM3 5h1v1h-1zM9 5h3v1h-3zM3 6h1v1h-1zM10 6h1v1h-1zM3 7h1v1h-1zM10 7h1v1h-1zM3 8h1v1h-1zM3 9h1v1h-1zM3 10h1v1h-1zM3 11h1v1h-1zM1 12h5v1h-5z"/>
      <path fill="#d7ffff" d="M8 1h3v1h-3z"/>
    </g>
  </defs>
</svg>
```

**关键点**：

- 不写 `xmlns`，避免触发"资源内不得出现 `http://`"的测试扫描；HTML5 内联 SVG 不需要该属性。
- 用 `<g>` + `<use>` 复用，避免首页和 topbar 各复制一份 280 个像素。
- 颜色取 256 色对应固定 hex，与 TUI `style::logo_pixel` 的 `k/p/n/b/c` 色相一致。

### 6.3 `web.css`

职责：

1. `:root` 基础令牌（从 `settings.css` / `usage.css` 的重复 `:root` 上移）：`--bg`、`--panel`、`--panel-raised`、`--overlay`、`--hover`、`--active`、`--field`、`--field-hover`、`--option-hover`、`--option-selected`、`--border`、`--border-strong`、`--border-overlay`、`--text`、`--muted`、`--faint`、`--accent`、`--accent-ink`、`--ok`、`--err`、`--warn`、`--shadow-raise`、`--shadow-overlay`、`--z-menu`、`--z-toast`、`--web-topbar-height: 48px`。
2. 全局 `html, body`、字体、背景、`body` 纵向 flex。
3. `.web-topbar`、`.web-brand`、`.web-brand-logo`、`.web-tabs`、`.web-tab`。
4. `#web-app`、`.web-view`、`.web-view.active`。
5. `.home-shell`、`.home-hero`、`.home-logo`、`.home-cards`、`.home-card`。
6. 共享反馈样式：`.toast-host` / `.toast` / `@keyframes toast-in` / `@keyframes dot-pulse`（两套 CSS 中这两块完全一致，上移到 `web.css` 只保留一份；Home 上的错误 toast 也能正常显示）。
7. **不迁移** `.select-*` 样式：两套 CSS 的下拉菜单存在细节差异（Usage 的选项为 `font-size:12px` + 单行省略，Settings 继承 14px），为满足"原页面样式不动"，`.select-*` 仍留在各自视图 CSS 中；`web.js` 只统一行为，不统一视觉。
8. 视图外壳：`.settings-shell`、`.usage-shell { height:100% }` 的覆盖（`settings.css` / `usage.css` 中删除 `height:100vh` 与 body 布局）。
9. 窄屏媒体查询。

关键代码：

```css
:root {
  color-scheme: dark;
  --bg: #0f0f0f;
  --panel: #171717;
  --panel-raised: #1f1f1f;
  --overlay: #272727;
  --hover: #212121;
  --active: #2b2b2b;
  --field: #232323;
  --field-hover: #2f2f2f;
  --option-hover: #575757;
  --option-selected: #666666;
  --border: #2e2e2e;
  --border-strong: #3d3d3d;
  --border-overlay: #4a4a4a;
  --text: #e8e8e8;
  --muted: #9a9a9a;
  --faint: #5f5f5f;
  --accent: #d4d4d4;
  --accent-ink: #141414;
  --ok: #3ecf8e;
  --err: #ef6b6b;
  --warn: #fbbf24;
  --shadow-raise: 0 1px 2px rgba(0, 0, 0, .45);
  --shadow-overlay: inset 0 1px 0 rgba(255, 255, 255, .06), 0 2px 6px rgba(0, 0, 0, .5), 0 16px 40px rgba(0, 0, 0, .6);
  --z-menu: 60;
  --z-toast: 1000;
  --web-topbar-height: 48px;
}

* { box-sizing: border-box; }
html, body { height: 100%; }
body {
  margin: 0;
  font: 14px/1.5 ui-monospace, SFMono-Regular, Menlo, monospace;
  display: flex;
  flex-direction: column;
  overflow: hidden;
  background: var(--bg);
  color: var(--text);
}

.web-topbar {
  flex: 0 0 var(--web-topbar-height);
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 24px;
  padding: 0 20px;
  background: var(--panel);
  border-bottom: 1px solid var(--border);
}
.web-brand {
  display: inline-flex;
  align-items: center;
  gap: 10px;
  color: var(--text);
  text-decoration: none;
  font-weight: 600;
  letter-spacing: .04em;
}
.web-brand-logo { width: 30px; height: auto; }
.web-tabs { display: flex; align-items: stretch; gap: 4px; height: 100%; }
.web-tab {
  display: inline-flex;
  align-items: center;
  border-bottom: 2px solid transparent;
  color: var(--muted);
  font: inherit;
  padding: 0 14px;
  text-decoration: none;
  cursor: pointer;
}
.web-tab:hover { color: var(--text); background: var(--hover); }
.web-tab.active { color: var(--text); border-bottom-color: var(--accent); font-weight: 600; }

#web-app { flex: 1 1 auto; min-height: 0; position: relative; }
.web-view { position: absolute; inset: 0; display: none; }
.web-view.active { display: block; }

.settings-shell { display: flex; height: 100%; overflow: hidden; }
.settings-shell main { flex: 1; min-width: 0; display: flex; flex-direction: column; overflow: hidden; padding: 24px; }
.usage-shell { height: 100%; }
```

### 6.4 `settings.css` / `usage.css` 的最小改动

只做"为嵌进 shell 让路"的改动，不重写业务样式：

`settings.css`：

- 删除文件顶部重复的 `:root` 基础令牌（保留任何 Settings 特有的令牌，若有）。
- 删除 `html, body { height:100% }` 与 `body { ... display:flex ... }`；字体/背景由 `web.css` 统一。
- `body.resizing` 改为 `.settings-shell.resizing` 或在 `web.js`/`settings.js` 中继续给 `document.body` 加类（`web.css` 增加 `body.resizing { cursor: col-resize; user-select: none; }`）。
- `nav.side` 增加 `height:100%; overflow-y:auto;`。
- 删除 `nav.side h1` 规则（HTML 已删除 h1）。
- `main { ... }` 改为 `.settings-shell main { ... }`。
- 删除 `.toast-host` / `.toast*` / `@keyframes toast-in` / `@keyframes dot-pulse`（迁入 `web.css`）。
- **保留** `.select-*` 块（与 Usage 版本存在细节差异，不合并，避免视觉变化）。
- 其余字段、命令、模型、MCP、Skills、Subagents 样式保持不变。

`usage.css`：

- 删除文件顶部重复的 `:root` 基础令牌；保留图表语义色令牌（`--chart-*`）。
- 删除 `html, body` 与 `body { ... }`；字体/背景由 `web.css` 统一。
- `.usage-shell { height: 100vh; }` → `height: 100%;`。
- 删除 `.header-titles`、`.usage-header h1`、`.usage-header .subtitle`（HTML 已删除标题行）。
- `.usage-header` 改为 `.usage-toolbar`：`display:flex; justify-content:flex-end; align-items:center; flex-wrap:wrap; gap:8px;`。
- 删除 `.toast-host` / `.toast*` / `@keyframes toast-in` / `@keyframes dot-pulse`（迁入 `web.css`）。
- **保留** `.select-*` 块（与 Settings 版本存在细节差异，不合并，避免视觉变化）。
- 其余筛选、卡片、图表、表格、分页样式保持不变。

**关键点**：

- `settings.css` 仅在 Settings 视图激活时生效，`usage.css` 仅在 Usage 视图激活时生效；两套通用选择器不会同时命中。
- 共享组件只保留 `web.css` 一份，避免两页 toast/select 样式漂移。
- 图表色令牌仍归 `usage.css`，因为只有 Usage 视图使用。

### 6.5 `web.js`（共享核心 + 路由）

```javascript
// ===== 共享核心：token / DOM 帮助函数 / toast / 自绘 select =====
const token = new URLSearchParams(location.search).get('token') || '';

function domEl(tag, className, text) {
  const node = document.createElement(tag);
  if (className) { node.className = className; }
  if (text !== undefined && text !== null) { node.textContent = String(text); }
  return node;
}

function toastHost() {
  let host = document.getElementById('toast-host');
  if (!host) {
    host = domEl('div', 'toast-host');
    host.id = 'toast-host';
    host.setAttribute('aria-live', 'polite');
    document.body.appendChild(host);
  }
  return host;
}

function showToast(message, kind) {
  const host = toastHost();
  const item = domEl('div', 'toast' + (kind ? ' ' + kind : ''), message);
  host.appendChild(item);
  setTimeout(() => {
    item.classList.add('leaving');
    setTimeout(() => item.remove(), 200);
  }, 3200);
}

const openSelects = new Set();

function createSelect(host, config) {
  if (!host) { return null; }
  const optionsConfig = config || {};
  // ...（采用 usage.js 版本：aria-expanded、菜单宽度夹取、replaceChildren）
  // choose() 末尾保留：
  //   if (typeof refreshActionBars === 'function') { refreshActionBars(); }
  // 以维持 Settings 操作条脏状态刷新。
}

// ===== 路由 =====
const VIEWS = ['home', 'settings', 'usage'];
const VIEW_TITLES = { home: 'lx-tui', settings: 'lx-tui · Settings', usage: 'lx-tui · Usage' };
const VIEW_CSS = { settings: 'css-settings', usage: 'css-usage' };
let currentView = 'home';

function viewFromLocation() {
  if (location.hash === '#' || location.hash === '#/') { return 'home'; }
  const hash = location.hash.replace(/^#\/?/, '');
  if (VIEWS.includes(hash)) { return hash; }
  const path = location.pathname.replace(/\/+$/, '') || '/';
  if (path === '/settings') { return 'settings'; }
  if (path === '/usage') { return 'usage'; }
  return 'home';
}

function showView(view) {
  if (!VIEWS.includes(view)) { view = 'home'; }
  currentView = view;
  for (const name of VIEWS) {
    document.getElementById('view-' + name).classList.toggle('active', name === view);
    const tab = document.querySelector('.web-tab[data-view="' + name + '"]');
    if (tab) { tab.classList.toggle('active', name === view); }
  }
  for (const name of ['settings', 'usage']) {
    const link = document.getElementById(VIEW_CSS[name]);
    if (link) { link.media = name === view ? '' : 'not all'; }
  }
  if (view === 'settings') { initOnce('settings', window.initSettingsView); }
  if (view === 'usage') { initOnce('usage', window.initUsageView); }
  document.title = VIEW_TITLES[view];
}

function initOnce(key, fn) {
  window.__webInit = window.__webInit || {};
  if (window.__webInit[key] === 'done' || window.__webInit[key] === 'running') { return; }
  if (typeof fn !== 'function') { return; }
  window.__webInit[key] = 'running';
  try {
    fn();
    window.__webInit[key] = 'done';
  } catch (error) {
    window.__webInit[key] = 'failed';
    showToast('Failed to initialise ' + key, 'error');
    throw error;
  }
}

function bootWeb() {
  const initial = viewFromLocation();
  const path = location.pathname.replace(/\/+$/, '') || '/';
  // 旧地址统一重写为 /?token=...#/<view>，之后切换只改 hash。
  if (path === '/settings' || path === '/usage') {
    const hash = initial === 'home' ? '#/' : '#/' + initial;
    history.replaceState(null, '', '/' + location.search + hash);
  }
  showView(initial);
  window.addEventListener('hashchange', () => showView(viewFromLocation()));
}

if (document.readyState === 'loading') {
  document.addEventListener('DOMContentLoaded', bootWeb);
} else {
  bootWeb();
}
```

**关键点**：

- `token` 只定义一次；所有页面脚本的函数在运行时通过全局词法环境读取。
- `domEl` 从 `usage_charts.js` 上移；`usage_charts.js` 删除本地定义。
- `createSelect` 采用 usage.js 的完整实现（`aria-expanded`、菜单宽度夹取、`replaceChildren`），并保留 Settings 需要的 `refreshActionBars()` 守卫调用。相对旧 Settings 版本，唯一行为差异是下拉菜单最小宽度 140、靠近视口边缘时自动夹取（Usage 现有行为），不改变选项内容与选择结果；两套 `.select-*` 视觉样式仍分别留在各自视图 CSS。
- `initOnce` 在失败时允许下次重试，避免一次异常把视图永久卡死。
- `history.replaceState` 只用于旧地址规范化；正常 tab 切换用 `location.hash`，因此浏览器前进/后退可用。
- 不写 `innerHTML`，动态文本全部 `textContent`，保持现有 XSS 约束。

### 6.6 `settings.js` 改动

1. 删除文件顶部：
   - `const token = ...`
   - `const toastHost = ...` / `document.body.appendChild(toastHost)` / `function showToast(...)`
   - `const openSelects = ...` / `function createSelect(...)`
2. 保留 `function api(path, options)`、`showTab`、`createActionBar`、`loadState` 等业务函数。
3. 把文件尾部的初始化执行块包进全局函数：

```javascript
// 原尾部：
// const SIDE_MIN_WIDTH = ...;
// setupResizer(...); setupResizer(...); setupResizer(...);
// actionBars.general = createActionBar(...);
// actionBars.commands = createActionBar(...);
// initLoggingSelect();
// initCommandSelects();
// loadState();

function initSettingsView() {
  if (window.__settingsInitialized) { return; }
  window.__settingsInitialized = true;

  const SIDE_MIN_WIDTH = 140;
  const SIDE_MAX_WIDTH = 360;
  const LIST_MIN_WIDTH = 140;
  const LIST_MAX_WIDTH = 560;
  let cmdListWidth = 220;

  setupResizer(/* ... */);
  setupResizer(/* ... */);
  setupResizer(/* ... */);
  actionBars.general = createActionBar('actions-general', { isDirty: generalDirty, save: saveGeneral, reset: resetGeneral });
  actionBars.commands = createActionBar('actions-commands', { isDirty: commandsDirty, save: saveCommand, reset: resetCommandEditor });
  initLoggingSelect();
  initCommandSelects();
  loadState();
}
```

**关键点**：

- `showTab` 仍可被 HTML 内联 `onclick` 调用；首次进入 Settings 时 `initSettingsView()` 已先执行，`actionBars` 不为空。
- 二级页签切换不重新拉取 General/Commands；行为与现在一致。
- 如果用户从未进入 Settings，不会请求 `/api/state`，首页保持纯静态。

### 6.7 `usage.js` 改动

1. 删除文件顶部：
   - `const token = ...`
   - `const openSelects = ...`
   - `function showToast(...)`
   - `function createSelect(...)`
2. 保留 `usageState`、`selects`、所有业务函数。
3. 文件尾部：

```javascript
// 原：initUsageDashboard();

function initUsageView() {
  if (window.__usageInitialized) { return; }
  window.__usageInitialized = true;
  initUsageDashboard();
}
```

**关键点**：

- 首次进入 Usage 才发起 `/api/usage/overview` 等请求。
- Usage 内部筛选、图表、表格、分页状态在视图切换后保留（DOM 不销毁）。
- 默认 `Auto = Off`，切走视图不会产生额外轮询；若用户开启自动刷新，切走后定时器仍按原逻辑运行（不在本任务改变该行为）。

### 6.8 `usage_charts.js` 改动

删除本地 `function domEl(...)`（第 23-28 行），改用 `web.js` 提供的全局 `domEl`。

**关键点**：

- `web.js` 必须先于 `usage_charts.js` 加载，脚本顺序已在 `web.html` 固定。
- 其余 SVG 图表代码不变。

### 6.9 初始化与切换时序（细化）

```text
[浏览器] GET /?token=...
   │
   ├─ HTML 解析到 </body>，依次加载 web.js、settings.js、...、usage.js
   │    ├─ web.js：定义 token / domEl / showToast / createSelect / showView
   │    │         在 DOMContentLoaded 时 bootWeb()
   │    ├─ settings.js：只定义函数与常量；initSettingsView 待调用
   │    └─ usage.js：只定义函数与常量；initUsageView 待调用
   │
   └─ DOMContentLoaded → bootWeb()
        ├─ initial = home
        ├─ showView('home')
        │    ├─ 三个 view 中只有 #view-home 有 .active
        │    ├─ settings.css / usage.css 的 media 均为 not all
        │    └─ 不发任何 fetch
        └─ 监听 hashchange

[用户点击顶部 Settings]
   └─ location.hash = '#/settings'
        └─ hashchange → showView('settings')
             ├─ #view-settings.active，其余隐藏
             ├─ settings.css 的 media = ''（应用）
             ├─ usage.css 的 media = 'not all'（挂起）
             ├─ initOnce('settings', initSettingsView)
             │    ├─ setupResizer / actionBars / init selects
             │    └─ loadState() → GET /api/state?token=...
             └─ document.title = 'lx-tui · Settings'
```

---

## 7. 测试与验收

### 7.1 Rust 单元测试

- `cargo test ui::controls`：`[WEB]` 几何、阈值 11、底边框无残留、tooltip。
- `cargo test event::tests::mouse_click`：点击置位 `web_open_requested`。
- `cargo test settings::service`：三个路径同一 shell、资源白名单、无外链、无 `innerHTML`、`service.url()`。
- `cargo test settings::service::usage`：`/api/usage/*` 与页面资源仍正常（token 改从 `service.token()` 取）。

### 7.2 浏览器手工验收

1. 运行 `cargo run`，侧栏底边框只显示 `[WEB]`；悬停显示 `open web`。
2. 点击 `[WEB]`，浏览器打开 `http://127.0.0.1:<port>/?token=...`，默认看到 Home：LX 像素 logo、`lx-tui` 字标、两张卡。
3. 点顶部 `Settings`：进入设置页，左侧 7 个二级页签正常，General 数据加载正常；侧栏无 `lx-tui` 重复标题。
4. 点顶部 `Usage`：进入用量看板，时间范围、Provider/Model/Workspace/Session 筛选、图表、表格、分页正常；顶部无 `USAGE` 重复大标题。
5. 顶部 `Home` / 浏览器后退：视图切换正确，Settings/Usage 已加载状态保留。
6. 直接访问 `/settings?token=...` 与 `/usage?token=...`：分别落在 Settings / Usage，地址被规范化为 `/?token=...#/...`。
7. 窗口缩窄：Home 卡片单列；Settings 左栏可滚动；Usage 筛选栏换行正常。
8. 任意视图触发保存/刷新 toast：toast 正常显示，不因 CSS 切换丢失样式。
9. 查看页面源码/网络：无任何外部域名请求；CSS/JS 均来自本地路由。

### 7.3 命令

```bash
cargo fmt
cargo clippy --all-targets -- -D warnings
cargo test
```

按影响范围优先执行：

```bash
cargo test ui::controls
cargo test event::tests::mouse_click
cargo test settings::service
```

---

## 8. 风险与回滚

| 风险 | 影响 | 缓解 |
|---|---|---|
| CSS 通用选择器互相污染 | Settings/Usage 视觉错乱 | 非当前视图的样式表 `media="not all"` 物理隔离；只保留当前视图一套 |
| `settings.js` / `usage.js` 仍有未发现的重复全局 | 页面 `SyntaxError`，整页白屏 | 迁移前用声明扫描确认重复项；迁移后浏览器控制台零报错作为验收项 |
| 懒初始化顺序错误 | Settings/Usage 空白或按钮无响应 | `initOnce` 在首次进入视图时调用；`actionBars` 在 `showTab` 前创建；手工验收覆盖 |
| 旧地址兼容 | 旧书签 404 或落错视图 | `/settings`、`/usage` 都返回同一 shell；启动时按 pathname 选视图并规范化 URL |
| Usage `height:100vh` 未改 | 顶部 bar 下方出现纵向滚动条/内容被裁 | 改为 `height:100%`，由 `.web-view` 提供高度 |
| 首页 SVG 体积 | HTML 变大 | 使用 `<g>` + `<use>` 复用，只在文档中出现一份路径数据 |
| 回滚 | — | 本任务只涉及 UI/资源/路由，无数据库与配置迁移；回滚对应提交即可恢复 `[SET]/[USE]` 与两个页面 |

---

## 9. 文件清单

### 9.1 Rust

- `src/ui/text/workspace.rs`：`SETTINGS_LABEL` / `USAGE_LABEL` → `WEB_LABEL`。
- `src/ui/controls/footer.rs`：两个按钮几何/命中/绘制 → 一个 `web_button` 系列。
- `src/ui/controls/mod.rs`：导出更新。
- `src/ui/mod.rs`：渲染调用一次。
- `src/ui/tooltip.rs`：tooltip 目标合并。
- `src/ui/text.rs`：tooltip 文案 `open web`。
- `src/app/tooltip.rs`：`SidebarSettings` / `SidebarUsage` → `SidebarWeb`。
- `src/app/state.rs`：两个请求位 → `web_open_requested`。
- `src/event/mouse/press.rs`：点击分支合并。
- `src/event/mod.rs`：消费请求并打开统一 URL。
- `src/settings/service.rs`：删除 `usage_url()`。
- `src/settings/service/handlers.rs`：三个路径统一返回 `web.html`。
- `src/settings/service/page.rs`：`web.html` / `web.css` / `web.js` 登记，删除 `usage_html()`。
- `src/ui/controls/tests.rs`：按钮测试重写。
- `src/event/tests/mouse_click.rs`：点击测试重写。
- `src/settings/service/tests.rs`：HTTP/资源断言更新。
- `src/settings/service/usage/tests.rs`：token 获取方式更新。

### 9.2 前端资源

- 新增 `assets/web.html`：统一 shell + Home + Settings + Usage。
- 新增 `assets/web.css`：全局 bar / Home / 共享组件 / 视图外壳。
- 新增 `assets/web.js`：共享核心 + 路由 + 懒初始化。
- 修改 `assets/settings.js`：删除重复全局，尾部初始化包进 `initSettingsView`。
- 修改 `assets/settings.css`：移除 body/root/共享组件，改 `.settings-shell`。
- 修改 `assets/usage.js`：删除重复全局，尾部初始化包进 `initUsageView`。
- 修改 `assets/usage.css`：移除 body/root/共享组件、标题行样式，`.usage-shell` 高度。
- 修改 `assets/usage_charts.js`：删除本地 `domEl`。
- 删除 `assets/settings.html`、`assets/usage.html`（内容已合并）。

### 9.3 文档与技能

- 更新 `docs/standards/html-assets.md`：目录表与加载顺序、媒体挂起规则、统一入口。
- 更新 `docs/standards/tui-design-requirements.md`：`[SET]/[USE]` → `[WEB]`。
- 更新 `src/skills/assets/lx-guide/references/commands-and-mentions.md`：`[SET]` → `[WEB]`，说明统一 Web 控制台。

---

## 10. 关键不变量（实现时不得破坏）

1. **渲染纯函数**：`ui/` 不执行 IO；点击只置位状态，浏览器由事件层打开。
2. **token 语义不变**：页面 `GET` 与所有 API 仍逐请求校验 capability token；静态资源仍免 token。
3. **单入口**：侧栏底边框不得再出现第二个 Web 相关按钮。
4. **默认 Home**：`[WEB]` 打开的 URL 不带 hash；Home 不发任何 API 请求。
5. **零外部资源**：`web.html` / `web.css` / `web.js` 不得出现 `http://` / `https://`、CDN、外链字体或图片。
6. **无 `innerHTML`**：动态文本一律 `textContent`。
7. **用户可见文案英文**：Home / 顶部 bar / tooltip / 错误提示均为英文；代码注释中文。
8. **现有功能零回归**：Settings 7 个页签、Usage 筛选/图表/表格/分页、保存/重置/toast 行为不变。

---

## 附录 A：Home 文案定稿

| 位置 | 文案 |
|---|---|
| `<title>`（Home） | `lx-tui` |
| 顶部品牌 | `lx-tui` |
| Home 副标题 | `Local agent workspace · configuration and usage in one place` |
| Settings 卡片标题 | `Settings` |
| Settings 卡片描述 | `Models, providers, MCP servers, skills, and subagents.` |
| Settings 卡片动作 | `Open settings →` |
| Usage 卡片标题 | `Usage` |
| Usage 卡片描述 | `Token usage, cost, and request history across model calls.` |
| Usage 卡片动作 | `Open usage →` |
| tooltip | `open web` |

## 附录 B：旧地址兼容示例

| 用户输入 | 规范化后 | 落地视图 |
|---|---|---|
| `http://127.0.0.1:3456/?token=abc` | 不变 | Home |
| `http://127.0.0.1:3456/?token=abc#/settings` | 不变 | Settings |
| `http://127.0.0.1:3456/settings?token=abc` | `/?token=abc#/settings` | Settings |
| `http://127.0.0.1:3456/usage?token=abc` | `/?token=abc#/usage` | Usage |
| `http://127.0.0.1:3456/?token=abc#/unknown` | `/?token=abc#/`（可选规范化） | Home |
