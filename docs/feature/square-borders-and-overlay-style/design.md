# 全局直角边框、浮层样式重构与命令面板统一 — 详细设计文档

- **分支**：`feat/square-borders-and-overlay-style`
- **工作区**：`.worktrees/square-borders-and-overlay-style`
- **关联文档**：[task.md](task.md)、[review.md](review.md)
- **参考规范**：
  - `docs/standards/project-directory-structure.md`
  - `docs/standards/code-writing-standards.md`
  - `docs/standards/tui-design-requirements.md`
  - `docs/standards/html-assets.md`
  - `/Users/yonah/projects/agent/herdr-master`（视觉与浮层布局参考基线）

---

## 1. 背景与目标

本次特性对 `lx-tui` 的容器边框、模态/菜单/Toast 浮层以及 `lx` / Prompt 命令面板进行系统性重构，对齐 `herdr-master` 的克制硬朗美学：

1. **全局直角边框**：TUI 全量组件（侧栏、主区窗格、右侧容器、模态弹窗、右键菜单、页码菜单、引用文件夹弹窗、命令面板、Toast、Tooltip、Prompt 模板块/补充块、`lx` 块格式面板）统一由圆角 `╭ ╮ ╰ ╯` 改为直角 `┌ ┐ └ ┘`；浏览器设置页（`assets/settings.css`）移除除状态圆点（`border-radius: 50%`）以外的全部圆角。
2. **分区边框静态暗灰**：移除左侧栏、中间主窗格与右侧容器的聚焦边框变色（`style::border(_focused)` 恒为 `style::muted()`），边框内嵌标题（`border_title`）与标签前缀（`border_label_prefix`）保持静态淡青（`Cyan + DIM`、不加粗）。
3. **浮层透明底与 `herdr` 布局重构**：
   - 浮层面板本体移除自绘深色背景（`style::overlay_panel()` 返回 `Style::default()`），配合 `clear` 清底保持终端原生透明背景；
   - **右键菜单**：移除标题栏与 `├───┤` 分割线，简化为纯列表框（`height = items + 2`），每项左侧 1 列内边距，点击菜单外或按 `Esc` 关闭；
   - **模态弹窗**：打开时调用 `modal::dim_background` 压暗全屏底层画面；移除 `├───┤` 分割线与右上角 `[✕]` 关闭按钮，标题位于内框首行粗体显示（危险确认弹窗使用红框 + 红色粗体标题 `render_danger`），标题下留 1 行呼吸空白；单行输入框整行铺暗色 `surface` 底条（`Indexed(236)`）；底部采用实心胶囊按钮（` ↵ save ` / ` ^c clear ` / ` esc cancel `，无方括号）；
   - **Toast（全局与 `lx` 窗格内）**：移除 `├───┤` 分割线与 `[✕]` 按钮，点击任意位置即关闭；首行前缀语义色圆点 `●`（无标题 3 行高 `│ ● <message> │`，有标题 4 行高 `│ ● <title> │` + 缩进次行 `│   <message> │`）；全局 Toast 晚于 `overlay::render` 绘制并优先响应点击/悬停，确保模态打开期间触发的报错/复制 Toast 不被压暗且点击 Toast 不误关模态。
4. **`lx` 与 Prompt 命令面板统一与目录配色**：
   - `lx` 的 `@`、`/`、粘贴与快捷引用面板采用 `borderless: true` 无边框样式：第 0 行为独立纯粹的强调色横线 `─`（作为浮层上边界，无左右和底框），第 1 行为干净的标题/Tab 文本行（无横线穿透文字，与 Prompt 内框首行一致），底部留 1 行空白间隔 + 1 行快捷键提示；
   - `@` 命令面板在 Tab（`Files · refDir · Skills · MCP`）左侧统一显示粗体 `@Command`（Prompt 选定取根时为 `@Command-[main]` / `@Command-[wt]`），`panel_width` 同时计入 `title` 与 `tabs` 宽度；
   - `@` 命令面板 Files / refDir 候选以及引用文件夹弹窗（`ReferencedFolderMenu` / `ReferencedFolderSearch`）中的文件夹图标与名称统一改为目录黄（`style::directory()` = `Color::Yellow`），不再使用蓝色；
   - Prompt 与 `lx` 命令面板全矩形范围（含标题行、呼吸空行、底栏提示、空状态行、边框与滚动条）均拦截左键点击，杜绝穿透触发底层块按钮或移走编辑器光标。

---

## 2. 已确认设计决策汇总

| 编号 | 决策问题 | 最终确认方案 |
| :--- | :--- | :--- |
| **Q1** | 直角边框在 TUI 与浏览器设置页的边界 | TUI 所有 `Block::bordered()` 与手绘框全部改为直角 `┌ ┐ └ ┘`；`assets/settings.css` 移除 20 处圆角，仅保留 `nav.side button::before` 与 `.status-dot` 的 `border-radius: 50%`。 |
| **Q2** | 浮层背景色与模态压暗 | 所有浮层面板移除 `Indexed(236)` 背景色，保持 `Color::Reset` 透明底；模态弹窗打开时通过 `modal::dim_background` 压暗底层画面；模态单行输入框铺 `Indexed(236)` 暗色底条；保留列表选中行 `overlay_selection()`（`Indexed(240)`）与菜单二次确认行 `menu_confirm()`（`Indexed(160)`）底色。 |
| **Q3** | 模态、右键菜单与 Toast 关闭交互 | 统一移除右上角 `[✕]` 关闭按钮与内部 `├───┤` 横向分割线；模态与菜单通过点击外部、`Esc` 或底部取消按钮关闭；Toast 通过点击任意位置（盒内消费、盒外关闭后继续派发）关闭。 |
| **Q4** | 左、中、右分区边框聚焦表现 | 移除分区边框聚焦变色：`style::border(_focused)` 恒为静态暗灰 `style::muted()`；内嵌标题与标签前缀保持静态淡青 `Cyan + DIM`。 |
| **Q5** | `lx` 与 Prompt 命令面板顶部对齐方案 | 顶线独立于文字行：`lx` 无边框面板第 0 行为纯 `─` 强调色横线，第 1 行为干净的 `@Command   Files · refDir · Skills · MCP`（或 `/Command` 等）文字行，无横线刺穿文字，与 Prompt 有边框面板的内框首行排版完全对齐。 |
| **Q6** | `@` 面板文件夹字体颜色 | 文件夹图标 `🗁` 与目录名统一使用目录黄 `Color::Yellow`（`style::directory()`），普通文件保持终端默认文字色。 |

---

## 3. 核心架构与视觉布局

### 3.1 模态弹窗、右键菜单与 Toast 布局（对齐 `herdr-master`）

```text
模态弹窗（普通 / 危险确认）：                  右键菜单（纯列表，高 = N + 2）：
┌──────────────────────────────────────┐      ┌────────────────────────┐
│ Rename workspace                     │      │ Split right            │
│                                      │      │ Split down             │
│ > my-workspace█                      │      │ Open prompt            │
│                                      │      │ Close                  │
│      ↵ save    ^c clear    esc cancel│      └────────────────────────┘
└──────────────────────────────────────┘

Toast（无标题 3 行 / 有标题 4 行）：
┌────────────────────────┐                    ┌──────────────────────────────┐
│ ● Copied 12 chars      │                    │ ● Compact failed             │
└────────────────────────┘                    │   context window exceeded    │
                                              └──────────────────────────────┘
```

### 3.2 `lx` 与 Prompt 命令面板顶部对齐示意

```text
lx 无边框 @ 提及面板（borderless = true）：
────────────────────────────────────────────────────────────────────────────
 @Command  Files · refDir · Skills · MCP                                src
❯ 🗁 components/
    └─ src/ui
  • mod.rs
    └─ src/ui

[←→ tabs] [↑↓ select] [↵ insert] [⇧↵ open] [^z back] [esc close]

Prompt 有边框 @ 提及面板（borderless = false）：
┌────────────────────────────────────────────────────┐
│ @Command  Files · refDir · Skills · MCP        src │
│                                                    │
│  🗁 components/                                     │
│    └─ src/ui                                       │
│                                                    │
│ [←→ tabs] [↑↓ select] [↵ insert] [⇧↵ open] [esc]   │
└────────────────────────────────────────────────────┘
```

---

## 4. 模块级实现细节

### 4.1 样式层（`src/ui/style.rs`）

- `border(_focused: bool) -> Style`：恒返回 `muted()`。
- `overlay_panel() -> Style`：返回 `Style::default()`（透明背景）。
- `directory() -> Style`：返回 `Style::default().fg(Color::Yellow)`（目录黄，不使用蓝色）。
- 新增模态胶囊按钮与输入条语义样式：
  - `button_primary()`：`Color::Black` on `Color::Cyan` + `BOLD`
  - `button_danger()`：`Color::Indexed(255)` on `Color::Red` + `BOLD`
  - `button_secondary()`：`Color::Indexed(255)` on `Color::Indexed(238)` + `BOLD`
  - `modal_input()`：`Color::Indexed(255)` on `Color::Indexed(236)`
  - `modal_input_placeholder()`：`Color::Indexed(244)` on `Color::Indexed(236)`

### 4.2 模态、右键菜单与 Toast（`src/ui/widgets/{modal,menu}.rs`、`src/ui/toast.rs`、`src/ui/main_content/lx/toast.rs`、`src/ui/mod.rs`）

- **`modal.rs`**：
  - `dim_background(frame, area)`：为全屏已有单元格叠加 `Modifier::DIM`。
  - `layout(screen, width, height)`：`height >= 6` 时顶部预留 3 行（`┌──┐` + 粗体标题行 + 1 行呼吸空白），`inner` 从 `area.y + 3` 起算、高度为 `area.height - 4`。
  - `render` / `render_danger`：压暗背景、清底、绘制直角边框与 `area.y + 1` 行的粗体标题。
- **`menu.rs`**：
  - `layout`：`height = labels.len() + 2`，`item_top = area.y + 1`，移除标题行与分割线。
- **`toast.rs` & `lx/toast.rs`**：
  - `draw_frame` 在 `(area.x + 2, area.y + 1)` 绘制语义色圆点 `●`；有标题时标题在 `(area.x + 4, area.y + 1)` 加粗显示、正文在 `area.y + 2` 以 `style::muted()` 显示；无标题时正文在 `(area.x + 4, area.y + 1)` 加粗（全局）或常规文本色（`lx` 窗格内折行）显示。
- **渲染与事件层序（`src/ui/mod.rs`、`src/event/mouse/{press,hover}.rs`）**：
  - `ui::render_cached` 中 `toast::render` 置于 `overlay::render` 之后，保证模态打开期间弹出的全局 Toast 不被 `dim_background` 压暗或被模态框擦除；硬件光标在 `state.overlay` 存在与否两分支下统一过滤 `tooltip_area`、`menu_tooltip_area` 与 `toast_area`。
  - `press.rs` 与 `hover.rs` 将全局 Toast 命中置于 `state.overlay.is_some()` 之前，点击 Toast 本身仅关闭 Toast 并消费事件，绝不误关底层模态弹窗。

### 4.3 命令面板统一与防穿透（`src/ui/widgets/command_panel.rs`、`src/ui/prompt/panels.rs`、`src/ui/main_content/lx.rs`）

- **顶栏排布与自适应宽度**：
  - `render_top_rule`（`borderless = true`）：`rect.y` 绘制强调色 `─` 顶线，`rect.y + 1` 绘制干净的 `title`（粗体）、`tabs`（`·` 分隔）与 `right_title`（右对齐 `muted`）。
  - `render_bordered_header`（`borderless = false`）：`rect.y + 1` 绘制内框首行 `title`、`tabs` 与 `right_title`，`rect.y + 2` 留 1 行呼吸空白；当 `available_rows < 7`（`!roomy`）退化为 `top_rows = 1` 时跳过顶栏与 `layout.tabs` 生成，避免与首条候选重叠。
  - `panel_width`：当 `view.tabs` 与 `view.title` 同时存在时，`tabs_header` 累加 `title.width() + 1 + total_tabs + separators + right_title`，确保自适应宽度面板完整容纳 `@Command` 与全部 4 个 Tab。
- **空状态置灰与面板防穿透**：
  - Prompt `mention_view` 与 `slash_view` 在空状态/占位行时设置 `hide_active_marker: true`，`render_item` 对占位行不铺 `overlay_selection()` 底色并以 `style::muted()` 置灰显示。
  - `ui::prompt::command_panel_contains` 与 `ui::main_content::lx::command_panel_contains`（含 `lx.panel`）覆盖全部打开的命令面板矩形，在 `press.rs` 中拦截面板非条目区域的左键点击，杜绝穿透触发底层按钮或移走编辑器光标。
