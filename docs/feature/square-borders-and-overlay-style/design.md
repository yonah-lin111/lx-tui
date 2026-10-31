# 全局直角边框、浮层透明背景与 lx 命令面板顶部空行 — 详细设计方案

> 状态：已完成实现与测试验证（位于 `.worktrees/square-borders-and-overlay-style` 工作区）
>
> 涉及模块：`src/ui/sidebar.rs`、`src/ui/panes.rs`、`src/ui/right_panel.rs`、`src/ui/toast.rs`、`src/ui/tooltip.rs`、`src/ui/prompt.rs`、`src/ui/prompt/panels.rs`、`src/ui/main_content/lx.rs`、`src/ui/main_content/lx/{mention,slash,refer,toast}.rs`、`src/ui/widgets/{command_panel,menu,modal,scrollbar}.rs`、`src/ui/overlay/{page_menu,referenced}.rs`、`src/ui/style.rs`、`assets/settings.css`、`docs/standards/tui-design-requirements.md`

---

## 1. 背景与需求目标

当前 `lx-tui` 在界面边框、浮层背景与命令面板间距上存在以下三处需要统一调整的设计细节：

1. **边框圆角需统一调整为直角**：
   - TUI 侧：侧栏（`Workspaces`）、主区窗格、右侧面板（`Prompt`/`Todo`）、模态弹窗、右键菜单、命令面板、全局与窗格内 Toast、Tooltip、lx 页 Markdown 块命令面板、以及 Prompt 编辑器内的模板块（`&&&`）与补充块（`%%%`）目前均使用圆角边框（`BorderType::Rounded` 或手绘 `'╭' / '╮' / '╰' / '╯'`）。
   - 浏览器设置页（`assets/settings.html` 关联的 `assets/settings.css`）：表单 `fieldset`、卡片 `.card` / `.model-card` / `.editor-card` / `.editor-empty`、输入框 `input` / `textarea`、按钮 `button.*`、导航项 `.nav-item`、标签 `.tag`、下拉菜单 `.select-button` / `.select-menu` / `.select-option`、`.toast`、以及滚动条滑块均设置了 `border-radius: 3px/4px/6px`。
   - **目标**：将 TUI 与浏览器设置页的所有容器、组件边框统一改为干脆利落的直角（TUI 使用 `BorderType::Plain` / `'┌' '┐' '└' '┘'`，设置页移除容器/控件圆角，仅保留两处状态圆点的 `50%` 圆形）。

2. **`src/ui/main_content/lx/` 输入框的 `@` 和 `/` 命令面板上方增加一行空行**：
   - 当前在 lx 输入框弹出无边框命令面板（`@` 提及面板、`/` 斜杠面板及同款的快捷引用、路径粘贴面板）时，面板底部在候选列表与快捷键提示之间留有 1 行空行（`FOOTER_BLOCK_ROWS = 2`），但**面板顶部蓝色横线（`── Commands ──` / `── Files refDir ... ──`）直接紧贴上方聊天历史或欢迎页内容**，没有空白隔离带，视觉上显得拥挤。
   - **目标**：在面板顶部蓝色横线的外侧上方增加 1 行空白行（由浮层清底擦除底层字符），使面板顶线与上方聊天记录/欢迎页内容之间始终隔开 1 行空行。

3. **移除模态弹窗、右键菜单、`src/ui/prompt/` 命令面板及 md 格式面板的背景颜色**：
   - 当前除 Toast 与 lx 无边框命令面板外，其余浮层（居中模态框、右键菜单、页码菜单、引用文件夹菜单与搜索弹窗、Prompt 右栏的 `@` 提及 / `/` 斜杠 / 快捷引用 / 粘贴 / Markdown 块命令面板、以及 Tooltip）均通过 `style::overlay_panel()` 铺设了 `Color::Indexed(236)` 的深灰不透明底色。
   - **目标**：移除所有上述浮层的面板背景颜色（仅通过 `clear::render` / `clear::render_buffer` 擦除底层文字，保持终端原生透明底色），同时保留菜单/面板中当前选中项、悬停项（`style::overlay_selection()`：`Indexed(240)` 底 + `Indexed(255)` 字）以及右键菜单行内二次确认项（`style::menu_confirm()`：`Indexed(160)` 深红底 + `Indexed(255)` 字）的整行高亮底色。

---

## 2. 已确认边界（Grill Me 决策汇总）

| 编号 | 决策问题 | 最终确认方案 |
| --- | --- | --- |
| **Q1** | `src/ui/main_content/lx/` 中 `@` 和 `/` 命令面板“上方多一行空行”的具体位置 | **位置 B（面板顶线外侧上方）**：在无边框命令面板顶部蓝色 `─` 横线的上方多留 1 行空白隔离带（纳入面板矩形顶部的清底范围），将上方聊天历史/欢迎页文字与面板顶部蓝色横线隔开 1 行空行。同步覆盖共用 `borderless: true` 的 `@`、`/`、快捷引用与路径粘贴面板。 |
| **Q2** | 移除浮层背景颜色的覆盖范围与选中高亮保留方式 | **全量浮层透明底 + 保留选中行高亮**：模态弹窗、右键菜单、页码菜单、引用文件夹菜单/搜索弹窗、`src/ui/prompt/` 下的所有浮层面板（`@`、`/`、md 块命令、快捷引用、粘贴面板）以及 Tooltip 统一移除 `overlay_panel` 面板背景色，仅用 `clear` 擦除底层字符并保持终端默认透明背景；选中行/悬停行继续保留 `style::overlay_selection()` 底色，右键菜单二次确认行继续保留 `style::menu_confirm()` 红底。 |
| **Q3** | 直角边框在 TUI 内部特殊块与浏览器设置页（`settings.html` / `settings.css`）的细节边界 | **TUI 全量直角 + 设置页仅状态圆点保留圆形**：<br>1. TUI 所有 `Block::bordered()` 面板/弹窗/菜单/Toast/Tooltip、lx 页 md 块面板及 Toast、Prompt 编辑器内的 `&&&` 模板块与 `%%%` 补充块全部由 `╭ ╮ ╰ ╯` 改为直角 `┌ ┐ └ ┘`。<br>2. `assets/settings.css` 中所有容器、卡片、输入框、按钮、导航项、标签、下拉菜单、Toast、滚动条滑块的 `border-radius` 全部移除（直角），仅保留侧栏未保存状态点（`nav.side button::before`）和顶栏状态点（`.status-dot`）的 `border-radius: 50%`。 |

---

## 3. 核心架构与视觉布局变化

### 3.1 TUI 全局直角边框示意

```text
修改前（圆角 BorderType::Rounded / 手绘 ╭ ╮ ╰ ╯）：
╭─ Workspaces ───────────[+]─╮╭─ lx-[Native Agent] ────────────── [>_] ─╮
│ ❯   ψ lx-tui *             ││                                         │
╰─[SET]──────────────────[◀]─╯╰─ model · 12k(6%) ───────────────────────╯

修改后（直角 BorderType::Plain / 手绘 ┌ ┐ └ ┘）：
┌─ Workspaces ───────────[+]─┐┌─ lx-[Native Agent] ────────────── [>_] ─┐
│ ❯   ψ lx-tui *             ││                                         │
└─[SET]──────────────────[◀]─┘└─ model · 12k(6%) ───────────────────────┘
```

### 3.2 `lx` 无边框命令面板（`@` / `/`）顶部新增空行结构

```text
修改前（TOP_BLOCK_ROWS = 1，顶线紧贴上方历史内容）：
  ● Here is the previous assistant message...
── Commands ──────────────────────────────────────────────────  <-- rect.y（顶线直接贴着上一行历史）
❯ /compact          compact conversation history                <-- inner.y = rect.y + 1
  /clear            clear current session
                                                                <-- rect.bottom() - 2（底部空行）
[select ↑↓] [run ↵] [close esc]                                 <-- rect.bottom() - 1（底部提示）
──────────────────────────────────────────────────────────────  <-- input.y（输入框顶边）
> /

修改后（TOP_BLOCK_ROWS = 2，顶线上方新增 1 行清底空行）：
  ● Here is the previous assistant message...
                                                                <-- rect.y（新增：1 行清底空白隔离行）
── Commands ──────────────────────────────────────────────────  <-- rect.y + 1（顶部蓝色横线 + Title/Tabs）
❯ /compact          compact conversation history                <-- inner.y = rect.y + 2
  /clear            clear current session
                                                                <-- rect.bottom() - 2（底部空行）
[select ↑↓] [run ↵] [close esc]                                 <-- rect.bottom() - 1（底部提示）
──────────────────────────────────────────────────────────────  <-- input.y（输入框顶边）
> /
```

### 3.3 浮层移除背景色后的渲染管线

```text
[绘制浮层：Modal / Menu / CommandPanel / PageMenu / ReferencedMenu / ReferencedSearch / Tooltip]
       │
       ▼
1. clear::render(frame, area) 或 clear::render_buffer(buf, rect)
   - 将浮层矩形内所有单元格 reset() 为空格 ' '、前景 Color::Reset、背景 Color::Reset（透明）
   - 自动擦除浮层左右边缘切半的底层宽字符（中文/emoji）
       │
       ▼
2. 绘制直角边框 Block::bordered()（默认 BorderType::Plain）
   - 不再调用 .style(style::overlay_panel())，保持背景 Color::Reset
       │
       ▼
3. 绘制内部条目与控件
   - 未选中/普通文本：style::text() / style::muted() / style::accent()（背景保持透明 Color::Reset）
   - 选中/悬停条目行：整行填充 style::overlay_selection()（bg: Indexed(240), fg: Indexed(255)）
   - 右键菜单待确认行：整行填充 style::menu_confirm()（bg: Indexed(160), fg: Indexed(255), BOLD）
   - 滚动条：统一使用透明底渲染 scrollbar::render / scrollbar::render_buffer（仅绘制 thumb '▐'）
```

---

## 4. 模块级详细设计与关键代码

### 4.1 全局直角边框改造（TUI + Web 设置页）

#### 4.1.1 Ratatui `Block::bordered()` 组件（11 处）

`ratatui::widgets::Block::bordered()` 的默认 `border_type` 即为 `BorderType::Plain`（直角 `┌ ─ ┐ │ └ ┘`）。我们将以下 11 处的 `.border_type(BorderType::Rounded)` 移除（或显式脱离 `BorderType::Rounded` 导入）：

| 序号 | 文件路径 | 所在函数 | 行号 | 说明 |
| --- | --- | --- | --- | --- |
| 1 | `src/ui/sidebar.rs` | `render` | L24-25 | 左侧 Workspaces / Agents 侧栏外框 |
| 2 | `src/ui/panes.rs` | `render_panes` | L49-50 | 主区域平铺窗格（lx 视图与终端视图）外框 |
| 3 | `src/ui/right_panel.rs` | `render` | L91-92 | 右侧 `Prompt` / `Todo` 容器外框 |
| 4 | `src/ui/widgets/modal.rs` | `render` | L40-41 | 通用居中模态容器（重命名、新建、确认、信任、worktree、保存草稿、会话编辑） |
| 5 | `src/ui/widgets/menu.rs` | `render_with_confirm` | L92-93 | 右键上下文菜单外框 |
| 6 | `src/ui/widgets/command_panel.rs` | `render` | L255-256 | 有边框命令面板（Prompt 右栏 `@`、`/`、md 块命令、快捷引用、粘贴面板）外框 |
| 7 | `src/ui/overlay/page_menu.rs` | `render_page_menu` | L202-203 | Prompt 页码菜单外框 |
| 8 | `src/ui/overlay/referenced/menu.rs` | `render_referenced_menu` | L119-120 | 引用文件夹目录弹窗外框 |
| 9 | `src/ui/overlay/referenced/search.rs` | `render_referenced_search` | L167-168 | 引用文件夹内容搜索弹窗外框 |
| 10 | `src/ui/toast.rs` | `render` | L75-76 | 全局 Toast 提示框 |
| 11 | `src/ui/tooltip.rs` | `render` | L61-62 | 全局按钮悬停 Tooltip 框 |

**关键代码（以 `src/ui/sidebar.rs`、`src/ui/panes.rs`、`src/ui/right_panel.rs` 为例）**：

```rust
// src/ui/sidebar.rs
let block = Block::bordered()
    .border_style(style::border(state.sidebar_focused()))
    .title(Span::styled(
        format!(" {} ", text::SIDEBAR_TITLE),
        style::border_title(),
    ));

// src/ui/panes.rs
let block = Block::bordered()
    .border_style(style::border(focused))
    .title(title_line);

// src/ui/right_panel.rs
let border = Block::bordered()
    .border_style(style::border(focused));
```

#### 4.1.2 手绘网格/缓冲区边框（3 处）

1. **`src/ui/main_content/lx.rs`（lx 页 Markdown 格式面板 `draw_box`）**：
   - 将四角字符由 `'╭' / '╮' / '╰' / '╯'` 改为 `'┌' / '┐' / '└' / '┘'`。
   - 同时在 `draw_box` 内将矩形内部单元格重置为空格 `' '` 与 `Style::default()`（透明清底），防止在聊天历史模式下打开 md 格式面板时底层历史文字穿透面板空白处。

```rust
/// 直角细线边框与内部清底（md 面板用）。
fn draw_box(grid: &mut Grid, rect: Rect, border: Style) {
    if rect.width < 2 || rect.height < 2 {
        return;
    }
    let right = rect.right() - 1;
    let bottom = rect.bottom() - 1;
    for y in rect.y + 1..bottom {
        for x in rect.x + 1..right {
            grid.set(x, y, ' ', Style::default());
        }
    }
    grid.set(rect.x, rect.y, '┌', border);
    grid.set(right, rect.y, '┐', border);
    grid.set(rect.x, bottom, '└', border);
    grid.set(right, bottom, '┘', border);
    for x in rect.x + 1..right {
        grid.set(x, rect.y, '─', border);
        grid.set(x, bottom, '─', border);
    }
    for y in rect.y + 1..bottom {
        grid.set(rect.x, y, '│', border);
        grid.set(right, y, '│', border);
    }
}
```

2. **`src/ui/main_content/lx/toast.rs`（lx 窗格内 Toast `draw_border`）**：
   - 将四角字符由 `'╭' / '╮' / '╰' / '╯'` 改为 `'┌' / '┐' / '└' / '┘'`。

```rust
/// 直角边框（只画四边与四角；不依赖 `Frame`，直接写 Buffer）。
fn draw_border(buf: &mut Buffer, area: Rect, kind: ToastKind) {
    let border = border_style(kind);
    let right = area.right().saturating_sub(1);
    let bottom = area.bottom().saturating_sub(1);
    for x in area.x..=right {
        put(buf, x, area.y, '─', border);
        put(buf, x, bottom, '─', border);
    }
    for y in area.y..=bottom {
        put(buf, area.x, y, '│', border);
        put(buf, right, y, '│', border);
    }
    put(buf, area.x, area.y, '┌', border);
    put(buf, right, area.y, '┐', border);
    put(buf, area.x, bottom, '└', border);
    put(buf, right, bottom, '┘', border);
}
```

3. **`src/ui/prompt.rs`（Prompt 编辑器模板块 `&&&` 与补充块 `%%%` 的 `paint_block_border`）**：
   - 将起始行四角 `'╭' / '╮'` 改为 `'┌' / '┐'`，结束行四角 `'╰' / '╯'` 改为 `'└' / '┘'`。

```rust
        Some(BlockRole::Start) => {
            put(left, '┌');
            put(left + 1, '─');
            put(right, '┐');
            for x in painted..buttons_start.min(right) {
                put(x, '─');
            }
        }
        Some(BlockRole::Middle) => {
            put(left, '│');
            put(right, '│');
        }
        Some(BlockRole::End) => {
            put(left, '└');
            put(left + 1, '─');
            put(right, '┘');
            for x in painted..right {
                put(x, '─');
            }
        }
```

#### 4.1.3 浏览器设置页样式（`assets/settings.css`）

移除 `assets/settings.css` 中除 `nav.side button::before`（L46）与 `.status-dot`（L60）之外的所有 20 处 `border-radius` 属性：

```css
/* 移除以下选择器的 border-radius 属性，使其全部保持直角： */
fieldset { border: 1px solid var(--border); margin: 0 0 16px; padding: 12px; background: var(--panel); }
input, textarea { width: 100%; padding: 6px 8px; border: 1px solid var(--border-strong); font: inherit; background: var(--field); color: var(--text); }
button.primary { background: var(--accent); border: 0; color: var(--accent-ink); padding: 7px 14px; cursor: pointer; font-weight: 600; }
button.ghost { background: none; border: 1px solid var(--border-strong); padding: 6px 12px; cursor: pointer; color: var(--text); }
button.danger { background: none; border: 1px solid var(--err); color: var(--err); padding: 6px 12px; cursor: pointer; }
.nav-item { display: grid; grid-template-columns: auto minmax(0, 1fr); align-items: center; column-gap: 6px; row-gap: 2px; width: 100%; padding: 6px 8px; border: 0; background: none; color: var(--muted); font: inherit; text-align: left; cursor: pointer; }
.nav-item .tag { grid-column: 2; grid-row: 2; justify-self: start; font-size: 11px; border: 1px solid var(--border-strong); padding: 1px 6px; color: var(--muted); }
.nav-item.new { margin-top: 8px; flex-shrink: 0; border-top: 1px solid var(--border); padding-top: 10px; }
.editor-card { display: flex; min-height: 0; flex-direction: column; overflow: hidden; border: 1px solid var(--border); background: var(--panel); padding: 12px; }
.editor-empty { display: flex; min-height: 280px; flex: 1 1 auto; flex-direction: column; align-items: center; justify-content: center; gap: 10px; border: 1px dashed var(--border-strong); color: var(--muted); font-size: 12px; text-align: center; }
.editor-body::-webkit-scrollbar-thumb { background: var(--border-strong); }
.select-button { display: flex; width: 100%; align-items: center; justify-content: space-between; gap: 8px; padding: 6px 8px; border: 1px solid var(--border-strong); background: var(--field); color: var(--text); font: inherit; cursor: pointer; text-align: left; }
.select-menu { position: fixed; z-index: var(--z-menu); left: 0; right: auto; max-height: 240px; overflow-y: auto; padding: 4px; border: 1px solid var(--border-overlay); background: var(--overlay); box-shadow: var(--shadow-overlay); scrollbar-width: thin; }
.select-option { display: block; width: 100%; padding: 6px 10px; border: 0; background: none; color: var(--text); font: inherit; text-align: left; cursor: pointer; }
.toast { padding: 8px 14px; border: 1px solid var(--border-overlay); border-left: 3px solid var(--accent); background: var(--overlay); color: var(--text); font-size: 12px; box-shadow: var(--shadow-overlay); pointer-events: auto; animation: toast-in .16s ease-out; }
.provider-list::-webkit-scrollbar-thumb { background: var(--border-strong); }
.input-action { position: absolute; top: 50%; right: 4px; display: flex; width: 28px; height: 24px; align-items: center; justify-content: center; padding: 0; border: 0; background: none; color: var(--muted); cursor: pointer; transform: translateY(-50%); }
.card { border: 1px solid var(--border); background: var(--panel); padding: 12px; margin-bottom: 12px; }
.model-card { display: flex; flex-direction: column; gap: 8px; border: 1px solid var(--border); background: var(--panel-raised); padding: 8px; }
.fetch-candidates { display: flex; flex-direction: column; gap: 4px; max-height: 220px; overflow-y: auto; margin-bottom: 8px; border: 1px dashed var(--border-strong); padding: 8px; scrollbar-width: thin; }
```

---

### 4.2 `src/ui/main_content/lx/` 无边框命令面板顶部增加 1 行空行

#### 4.2.1 `src/ui/widgets/command_panel.rs` 几何与绘制调整

1. **顶部行数常量与最小高度**：
   - 新增 `const TOP_BLOCK_ROWS: u16 = 2;`（无边框面板顶部块行数：第 0 行为空白间隔行，第 1 行为顶部蓝色 `─` 分割线及标题/Tab）。
   - `const MIN_BORDERLESS_HEIGHT: u16 = TOP_BLOCK_ROWS + 1;`（即 3 行：1 行顶部空行 + 1 行顶部蓝色横线 + 至少 1 行内容）。
2. **`layout` 计算**：
   - `let frame_rows: u16 = if view.borderless { TOP_BLOCK_ROWS + footer_rows } else { 2 };`
   - `tabs` 的纵坐标：当 `view.borderless` 为 `true` 时，Tab 位于顶部蓝色横线所在行 `rect.y.saturating_add(1)`；有边框模式仍位于 `rect.y`。
   - `borderless_inner_rect`：内容区起点从 `rect.y.saturating_add(TOP_BLOCK_ROWS)` 开始，高度为 `rect.height.saturating_sub(TOP_BLOCK_ROWS).saturating_sub(footer_rows)`。
3. **`render_top_rule` 绘制行下移 1 行**：
   - `clear::render_buffer(buf, layout.rect)` 会将 `rect.y..rect.bottom()` 全部清空为空白透明单元格，因此 `rect.y` 自然成为 1 行干净的空白隔离带。
   - `render_top_rule` 在 `let rule_y = rect.y.saturating_add(1);` 上绘制蓝色 `─` 横线、左侧 `title` 与右侧 `right_title`。

**关键代码（`src/ui/widgets/command_panel.rs`）**：

```rust
/// 无边框面板顶部块的行数：1 行上方空行间隔 + 1 行顶部分割线。
const TOP_BLOCK_ROWS: u16 = 2;
/// 无边框面板最小高度（顶部 2 行 + 1 行内容）。
const MIN_BORDERLESS_HEIGHT: u16 = TOP_BLOCK_ROWS + 1;
/// 无边框面板底部提示块的行数：1 行空行间隔 + 1 行快捷键提示。
const FOOTER_BLOCK_ROWS: u16 = 2;

pub fn layout(area: Rect, view: &CommandPanelView<'_>) -> Option<PanelLayout> {
    let footer_rows: u16 = if view.borderless && view.footer.is_some() {
        FOOTER_BLOCK_ROWS
    } else {
        0
    };
    let frame_rows: u16 = if view.borderless {
        TOP_BLOCK_ROWS + footer_rows
    } else {
        2
    };
    let min_height = if view.borderless {
        MIN_BORDERLESS_HEIGHT + footer_rows
    } else {
        MIN_HEIGHT
    };
    // ...
    let tab_y = if view.borderless {
        rect.y.saturating_add(1)
    } else {
        rect.y
    };
    let tabs = match view.tabs {
        Some(tabs) if !tabs.is_empty() => {
            let mut out = Vec::with_capacity(tabs.len());
            let mut x = rect.x.saturating_add(2);
            let right_limit = rect.right().saturating_sub(1);
            for tab in tabs {
                let width = u16::try_from(UnicodeWidthStr::width(tab.label))
                    .unwrap_or(u16::MAX)
                    .saturating_add(2);
                if x.saturating_add(width) > right_limit {
                    break;
                }
                out.push(Rect::new(x, tab_y, width, 1));
                x = x.saturating_add(width).saturating_add(1);
            }
            out
        }
        _ => Vec::new(),
    };
    // ...
}

fn render_top_rule(
    buf: &mut Buffer,
    layout: &PanelLayout,
    view: &CommandPanelView<'_>,
    has_tabs: bool,
) {
    let rect = layout.rect;
    let rule_y = rect.y.saturating_add(1);
    for x in rect.x..rect.right() {
        if let Some(cell) = buf.cell_mut((x, rule_y)) {
            cell.set_char('─');
            cell.set_style(style::accent());
        }
    }
    let mut left_end = rect.x;
    if has_tabs {
        if let Some(last_tab) = layout.tabs.last() {
            left_end = last_tab.right();
        }
    } else if let Some(title) = view.title {
        let text = format!(" {title} ");
        let x = rect.x.saturating_add(1);
        let max_w = usize::from(rect.right().saturating_sub(1).saturating_sub(x));
        if max_w > 0 {
            let draw_w = u16::try_from(text.width().min(max_w)).unwrap_or(u16::MAX);
            buf.set_style(Rect::new(x, rule_y, draw_w, 1), Style::reset());
            buf.set_stringn(x, rule_y, &text, max_w, style::muted());
            left_end = x.saturating_add(draw_w);
        }
    }
    if let Some(right_title) = view.right_title {
        let right_edge = rect.right().saturating_sub(1);
        let min_x = left_end.saturating_add(1);
        let avail = usize::from(right_edge.saturating_sub(min_x));
        if avail >= 3 {
            let clipped = text::truncate_to_width(right_title, avail.saturating_sub(2));
            let text = format!(" {clipped} ");
            let text_w = u16::try_from(text.width()).unwrap_or(u16::MAX);
            if text_w <= right_edge.saturating_sub(min_x) {
                let x = right_edge.saturating_sub(text_w);
                buf.set_style(Rect::new(x, rule_y, text_w, 1), Style::reset());
                buf.set_stringn(x, rule_y, &text, usize::from(text_w), style::muted());
            }
        }
    }
}

/// 无边框面板内容区矩形：扣除顶部两行（1 行空行 + 1 行分割线）与底部可选快捷键提示行，左右不留边框。
fn borderless_inner_rect(rect: Rect, footer_rows: u16) -> Rect {
    Rect::new(
        rect.x,
        rect.y.saturating_add(TOP_BLOCK_ROWS),
        rect.width,
        rect.height
            .saturating_sub(TOP_BLOCK_ROWS)
            .saturating_sub(footer_rows),
    )
}
```

#### 4.2.2 `src/ui/main_content/lx/{mention,slash,refer}.rs` 与 `src/ui/main_content/lx.rs` 高度固定最大与预算同步

- **`CommandPanelView::fixed_height`**：在 `CommandPanelView` 新增 `pub fixed_height: bool`；当 `fixed_height = true` 且 `max_height = Some(cap)` 时，`command_panel::layout` 直接以 `cap` 作为目标高度（仅受锚点上方可用空间 `space` 夹取），不再随当前候选条目行数收缩。内部不足 8 行的剩余区域由 `clear::render_buffer` 保持为空白，且 `item_at` 仅命中实际可见条目行。
- 在 `src/ui/main_content/lx/mention.rs` 中：
  `pub(super) const PANEL_BORDER_ROWS: u16 = 2;`（1 行顶部空行间隔 + 1 行顶部蓝色横线）。
  同时在 `mention.rs`、`slash.rs` 与 `refer.rs` 的 `view` 中设置 `fixed_height: true`，使 `@`、`/`（含 `/model`、`/resume`、`/rewind`）及快捷引用面板高度固定为最大高度 `8 + 2 + 2 = 12` 行，过滤或切换 Tab 时不再自适应跳动。
- 在 `src/ui/main_content/lx.rs` 的 `paste_panel_rows` 中：
  将最小上方高度守卫 `if region_h < 2` 同步为 `if region_h < 3`（对齐 `MIN_BORDERLESS_HEIGHT`）。

---

### 4.3 移除浮层背景颜色（Modal / Menu / Prompt Panels / PageMenu / Referenced / Tooltip）

#### 4.3.1 `src/ui/style.rs` 样式收敛

- 将 `style::overlay_panel()` 调整为透明底与终端默认前景（`Style::default()`），或移除对 `Color::Indexed(236)` 背景色的设置：
  ```rust
  /// 浮层面板基础样式：不铺设背景色（配合 `clear` 清底保持终端原生透明底色），使用终端默认前景。
  pub fn overlay_panel() -> Style {
      Style::default()
  }
  ```
- 保留 `style::overlay_selection()`（`fg: Color::Indexed(255), bg: Color::Indexed(240)`）与 `style::menu_confirm()`（`fg: Color::Indexed(255), bg: Color::Indexed(160), BOLD`）供选中/悬停/确认行高亮使用。

#### 4.3.2 各浮层组件绘制更新

1. **`src/ui/widgets/modal.rs`**：
   - `clear::render(frame, shell.area)` 擦除底层文字并重置为透明背景；
   - `Block::bordered()` 移除 `.style(style::overlay_panel())`。
2. **`src/ui/widgets/menu.rs`**：
   - `clear::render(frame, layout.area)` 擦除底层文字；
   - `Block::bordered()` 移除 `.style(style::overlay_panel())`；
   - 菜单项：`confirming` 行用 `style::menu_confirm()`，`selected` 行用 `style::overlay_selection()`，其余未选中行用 `style::text()`（无背景色）。
3. **`src/ui/widgets/command_panel.rs`（服务于 `src/ui/prompt/panels.rs` 的 `@`、`/`、md 格式面板、快捷引用、粘贴面板）**：
   - `clear::render_buffer(buf, layout.rect)` 擦除底层文字；
   - 有边框分支（`!view.borderless`）的 `Block::bordered()` 移除 `.style(style::overlay_panel())`；
   - 滚动条：无论 `view.borderless` 与否，统一调用 `scrollbar::render_buffer(buf, scrollbar)`（透明底，仅绘制 thumb `'▐'`）。
4. **`src/ui/overlay/page_menu.rs`**：
   - `Block::bordered()` 移除 `.style(style::overlay_panel())`；
   - 顶部边框按钮 `render_chip`：不可用时使用 `style::muted_on_border()`（显式 `fg: Color::Reset + DIM` 且移除粗体，避免继承聚焦边框的 Cyan 加粗），可用时使用 `style::accent()`；
   - 页面列表行：选中/悬停行用 `style::overlay_selection()`，未选中行用 `style::text()`。
5. **`src/ui/overlay/referenced/menu.rs` & `src/ui/overlay/referenced/search.rs`**：
   - `Block::bordered()` 移除 `.style(style::overlay_panel())`；
   - 滚动条改为 `widgets::scrollbar::render(frame, &scrollbar)`（透明底）；
   - 未悬停/未选中行基础样式改为 `style::text()`（透明底），悬停/选中行保持 `style::overlay_selection()`。
6. **`src/ui/tooltip.rs`**：
   - `clear::render(frame, area)` 清底后，`Block::bordered()` 移除 `.style(style::overlay_panel())`。

### 4.4 浮层顶栏/底栏（标题行 + 标题下方 `├──┤`，快捷键上方 `├──┤`）

除 `lx` 输入框上方的无边框命令面板外，所有有 `title` 或底部快捷键/按钮的浮层（右键菜单、页码菜单、引用文件夹菜单/搜索弹窗、居中模态弹窗、全局与 `lx` 窗格内 Toast、Tooltip、Prompt 有边框命令面板、`lx` Markdown 块面板）：
- **顶栏**：`┌───┐`（`y`）+ 标题行（`y + 1`：左侧标题、右侧关闭按钮，中间留空不画横线）+ `├───┤`（`y + 2`），内容区从 `y + 3` 开始；标题行左右外框 `│` 贯通。
- **底栏**：`├───┤`（`bottom - 3`）+ `│ footer / buttons │`（`bottom - 2`）+ `└───┘`（`bottom - 1`）；内容与分割线之间**不留空行**。
- 关闭按钮 `[✕]` 与左右边框各留 1 列内边距（`right - 5` 起 3 列）。
- 无底栏的浮层（菜单、toast、tooltip）在标题分割线下直接接内容，底部无多余空行。

### 4.5 统一快捷键文案格式与移除 Prompt 模板块/补充块背景色

1. **快捷键显示格式统一**：
   - 纯文本底栏说明：`符号 功能 · 符号 功能`（如 `↑↓ select · ↵ confirm · esc cancel`、`↵ open · ^c copy · ^d del · esc close`）。
   - 可点击带快捷键按钮：`[符号 功能]`（如 `[↵ save]`、`[^c clear]`、`[esc cancel]`、`[↑↓ select]`）。
2. **移除 Prompt 模板块与补充块整行背景色**：
   - `src/ui/prompt.rs` 移除 `paint_block_background` 调用，模板块（`&&&`）与补充块（`%%%`）仅保留状态色直角边框与语法高亮，保持透明底色，彻底消除弹出透明浮层时切穿深色块造成的透明破洞。

---

## 5. 影响面与测试验证计划

1. **受影响测试更新**：
   - **直角边框断言**：更新 `src/ui/tests.rs`、`src/ui/widgets/modal/tests.rs`、`src/ui/widgets/menu/tests.rs`、`src/ui/widgets/command_panel/tests.rs`、`src/ui/overlay/tests.rs`、`src/ui/prompt/tests.rs`、`src/ui/right_panel/tests.rs`、`src/ui/tooltip/tests.rs`、`src/event/tests/lx_toast.rs`、`tests/prompt_border_title.rs`、`tests/prompt_template_commands.rs` 中断言 `'╭' / '╮' / '╰' / '╯'` 的用例为 `'┌' / '┐' / '└' / '┘'`。
   - **无边框命令面板顶部空行断言**：更新 `src/ui/widgets/command_panel/tests.rs`、`src/ui/main_content/lx/tests.rs`、`src/ui/main_content/lx/tests/slash_panels.rs` 中关于无边框面板高度、顶线行坐标（`rect.y + 1`）及顶部空行（`rect.y` 为空白）的断言。
   - **浮层透明背景断言**：更新 `src/ui/overlay/tests.rs`、`src/ui/style/tests.rs`、`src/ui/widgets/command_panel/tests.rs`、`src/ui/widgets/menu/tests.rs` 中检查浮层背景色为 `Color::Indexed(236)` 的断言为 `Color::Reset`，同时验证选中行仍为 `Color::Indexed(240)`。
   - **浏览器设置页 CSS 断言**：在 `src/settings/service/tests.rs` 中补充验证容器/控件已移除圆角、仅状态圆点保留 `border-radius: 50%`。
2. **文档同步**：
   - 同步更新 `docs/standards/tui-design-requirements.md` 中关于边框直角、浮层透明底色与 lx 命令面板顶部空行的规范说明。
3. **验证命令**：
   - `rtk cargo fmt --check`
   - `rtk cargo clippy --all-targets -- -D warnings`
   - 针对受影响模块运行 `rtk cargo test`
