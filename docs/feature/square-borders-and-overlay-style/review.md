# 全局直角边框、浮层样式重构与命令面板统一 — 代码评审报告（Code Review）

- **目录**：`docs/feature/square-borders-and-overlay-style/`
- **分支**：`feat/square-borders-and-overlay-style`（工作区 `.worktrees/square-borders-and-overlay-style`）
- **关联文档**：[design.md](design.md)、[task.md](task.md)
- **评审方式**：基于 `codegraph`（`codegraph_explore` 调用链与影响面分析）全链路追踪 `command_panel`、`modal`、`menu`、`toast`、`style` 与 `event/mouse/{press,hover,hit}` 的几何计算、绘制层序和事件拦截
- **状态**：**P0 ~ P2 全部缺陷已定位并完成修复**，回归测试已补齐，`rtk cargo fmt`、`rtk cargo clippy --all-targets -- -D warnings`、`rtk cargo test` 全绿。

---

## 问题分级汇总（P0 → P2）

| 优先级 | 编号 | 模块 / 文件 | 关键问题摘要 | 状态 |
| :--- | :--- | :--- | :--- | :--- |
| **P0** | P0-1 | `src/ui/widgets/command_panel.rs` | `panel_width` 在 `title` 与 `tabs` 同时存在时漏计左侧 `title`（`@Command`）宽度，导致自适应宽度的 Prompt `@` 面板截断丢弃 `Skills` / `MCP` Tab 与右侧目录名；且极矮退化态（`!roomy`）下仍生成并绘制与首条候选重叠的 Tab | 已修复 |
| **P0** | P0-2 | `src/ui/mod.rs`<br>`src/event/mouse/press.rs`<br>`src/event/mouse/hover.rs` | `toast::render` 早于 `overlay::render` 执行，导致模态与全局 Toast 并存时 Toast 被 `modal::dim_background` 压暗或擦除；且 `press.rs` / `hover.rs` 在模态打开时跳过全局 Toast 拦截，点击报错 Toast 反而误关正在编辑的模态弹窗 | 已修复 |
| **P1** | P1-1 | `src/ui/prompt/panels.rs`<br>`src/ui/prompt.rs`<br>`src/event/mouse/hit.rs`<br>`src/event/mouse/press.rs`<br>`src/ui/main_content/lx.rs` | Prompt 命令面板重构为 6 行顶底留白后，未拦截面板非条目区域（标题行、呼吸空行、底栏、空态行、边框）的左键点击，导致点击面板内部空白穿透触发底层块按钮或移走编辑器光标瞬间关闭面板；`lx::command_panel_contains` 亦漏掉 `lx.panel` 块格式面板 | 已修复 |
| **P1** | P1-2 | `src/ui/prompt/panels.rs`<br>`src/ui/widgets/command_panel.rs` | Prompt `@` 提及面板与 `/` 面板的空状态/占位行（`scanning…`、`no matches`、`no matching folders` 等）未设置 `hide_active_marker: true`，被误渲染为高亮选中态底色 | 已修复 |
| **P1** | P1-3 | `src/ui/main_content/lx.rs` | `lx` 的 Markdown 块格式面板 `draw_panel` 使用未裁剪的 `grid.put` 绘制标题、预览与底栏，窄窗格下写穿右边框 `│`；且 `lx::render` 丢弃了 `toast::draw` 返回的遮挡矩形，未隐藏被窗格内 Toast 覆盖的硬件光标 | 已修复 |
| **P2** | P2-1 | `src/ui/overlay/referenced/menu.rs` | 引用文件夹目录弹窗（`ReferencedFolderMenu`）的目录图标仍硬编码为蓝色 `style::accent()`，与黄色目录名 `style::directory()` 色彩割裂；且极矮窗口（`height < 7`）下底栏提示直接覆盖底边框 `└───┘` | 已修复 |
| **P2** | P2-2 | `docs/feature/square-borders-and-overlay-style/{design.md,task.md}`<br>`docs/standards/tui-design-requirements.md` | 设计文档与规范文档中残留多轮视觉迭代前的过渡描述（如旧版横线穿透标题、固定 12 行高度、带 `[✕]` 的模态与 Toast、边框聚焦变色），与最终代码实现不一致 | 已修复 |

---

## 详细分析与修复方案（P0 – P2）

### P0-1：`command_panel::panel_width` 在 `title` 与 `tabs` 共存时漏计 `title` 宽度，且极矮退化态下 Tab 与首条候选重叠

- **关键点**：
  - `src/ui/widgets/command_panel.rs:246-283`（`layout` 中 `tabs` 命中区排布）
  - `src/ui/widgets/command_panel.rs:565-575`（`panel_width` 中 `tabs_header` 宽度计算）
  - `src/ui/prompt/panels.rs:695-696`（Prompt `@` 提及面板同时传入 `title: Some("@Command")` 与 `tabs: Some(&data.tabs)`）
- **错误原因**：
  1. 本次需求在 `@` 提及面板的 Tab（`Files · refDir · Skills · MCP`）左侧新增了 `@Command`（或 `@Command-[wt]` / `@Command-[main]`）粗体标题，使 `view.title` 与 `view.tabs` 同时为 `Some`。
  2. 但 `command_panel::panel_width` 仍沿用旧版“有 `tabs` 时忽略 `title`”的互斥分支，`tabs_header` 仅计算 `total_tabs + separators + right_title`，完全漏计了左侧 `view.title` 及其间隔占用的 `title.width() + 1` 列（9~14 列）。
  3. 当自适应宽度的 Prompt `@` 提及面板处于扫描中（`scanning…`）、空列表（`no matches` / `no skills available` / `no MCP servers configured`）或短文件名场景时，计算出的面板宽度比顶栏实际所需窄 9~14 列；在 `layout()` 排布 Tab 时，`Skills` 与 `MCP` Tab 因越过 `right_limit` 被直接截断丢弃（即使 Prompt 编辑区宽度充裕也无法显示或点击后两个 Tab），进入子目录后的右侧目录名（如 `src`）也被挤压截断为 `s…`。
  4. 此外，当有边框面板处于极矮退化态（`!view.borderless && available_rows < 7`，`top_rows == 1`）时，`layout.inner.y == layout.rect.y + 1`，首条候选直接绘制在 `rect.y + 1` 行；此时 `render()` 跳过了 `render_bordered_header`，但 `layout()` 仍无条件在 `rect.y + 1` 生成 `layout.tabs` 且 `render()` 仍在 `rect.y + 1` 绘制 Tab，导致 Tab 与首条候选重叠，点击首条候选会被 `mention_tab_at` 误判为切换 Tab。
- **修复实现**：
  - 在 `panel_width` 的 `Some(tabs)` 分支中计入 `let title_w = view.title.map_or(0, |title| title.width() + 1);`；
  - 在 `layout()` 中增加 `let show_header = view.borderless || top_rows > 1;` 守卫，仅当独立顶栏行存在时才生成 `layout.tabs`；
  - 新增单元测试 `bordered_panel_with_title_and_tabs_expands_width_and_omits_tabs_when_cramped` 验证。

---

### P0-2：模态弹窗 `modal::dim_background` 晚于全局 `toast::render` 执行，且模态打开期间跳过了全局 Toast 点击与悬停拦截

- **关键点**：
  - `src/ui/mod.rs:110-133`（`toast::render` 与 `overlay::render` 渲染层序及硬件光标遮挡过滤）
  - `src/event/mouse/press.rs:54-124`（`state.overlay.is_some()` 与 `ui::toast::rect` 点击分派顺序）
  - `src/event/mouse/hover.rs:54-123`（模态打开期间对全局 Toast 的 `set_toast_hover` 更新）
- **错误原因**：
  1. 本次将模态弹窗改为调用 `modal::dim_background(frame, frame.area())` 对全屏底层画面施加 `Modifier::DIM` 压暗，并将 Toast 改为无 `[✕]` 按钮、点击任意位置关闭。
  2. 然而在 `src/ui/mod.rs` 中，`toast::render` 先于 `overlay::render` 执行。当模态弹窗与全局 Toast 同时存在时（例如：`NewWorkspace` 输入非法路径回车触发 `directory does not exist` 报错 Toast、`ReferencedFolderSearch` 点击 `[copy]` 复制路径触发 `Copied N chars` Toast、或在任意模态单行输入框中按 `Ctrl+C` 复制选区文本触发 Toast）：
     - 后执行的 `overlay::render` 调用 `dim_background(frame, frame.area())`，将刚绘制的全局 Toast 一并压暗；若弹窗较高延伸至顶部，弹窗的 `clear::render` 还会直接擦除全局 Toast。
     - 同时 `src/ui/mod.rs` 在 `state.overlay.is_some()` 分支直接返回 `overlay_cursor`，未过滤被 `toast_area` 或 `menu_tooltip_area` 覆盖的硬件光标。
  3. 在 `src/event/mouse/press.rs` 中，`if state.overlay.is_some()` 排在 `if let Some(toast) = ui::toast::rect(...)` 之前并直接 `return`。用户在模态打开期间点击屏幕顶部的报错 Toast 试图关闭 Toast 时，会进入 `handle_overlay_click` 被判定为“点击弹窗外部”而直接误关正在编辑的模态弹窗，且 Toast 仍留在屏幕上。
  4. 在 `src/event/mouse/hover.rs` 中，`if state.overlay.is_some()` 提前 `return`，未执行 `set_toast_hover`，导致模态打开期间悬停在 Toast 上无法暂停消失计时。
- **修复实现**：
  - 在 `src/ui/mod.rs` 中将 `toast::render` 移至 `overlay::render` 之后，并对 `overlay_cursor` 与 `prompt_cursor` 统一应用 `tooltip_area` / `menu_tooltip_area` / `toast_area` 遮挡隐藏过滤；
  - 在 `src/event/mouse/press.rs` 中将全局 Toast 拦截移至 `if state.overlay.is_some()` 之前（点击 Toast 盒内关闭 Toast 并消费点击、保持模态打开；点击 Toast 盒外先关闭 Toast 再继续处理模态点击）；
  - 在 `src/event/mouse/hover.rs` 顶部先更新 `set_toast_hover` 再进入 `if state.overlay.is_some()` 分支；
  - 新增回归测试 `clicking_toast_while_modal_open_dismisses_toast_without_closing_modal` 验证。

---

### P1-1：Prompt 命令面板非条目区域（标题行、呼吸空行、底栏、空态行、边框）未拦截左键点击，导致穿透触发底层块按钮或移走编辑器光标关闭面板

- **关键点**：
  - `src/ui/prompt/panels.rs`（`command_panel_contains`）
  - `src/ui/prompt.rs`
  - `src/event/mouse/hit.rs`（`prompt_command_panel_contains`）
  - `src/event/mouse/press.rs:342-352`
  - `src/ui/main_content/lx.rs:383-390`（`lx::command_panel_contains`）
- **错误原因**：
  1. 本次将 Prompt 命令面板对齐 `herdr` 样式后，顶栏占 3 行（顶边框 + 标题/Tab 行 + 1 行呼吸空白）、底栏占 3 行（1 行呼吸空白 + 快捷键提示行 + 底边框），另有扫描中/无匹配的空状态行（`mention_item_at` 返回 `None`）及右侧滚动条列。
  2. `Prompt` 侧在 `press.rs` 中仅检查了 `*_item_at` 与 `mention_tab_at`，未对命令面板外框矩形做兜底包含拦截；同时 `lx::command_panel_contains` 漏掉了同样改为 `rows + 6` 高度的 `lx` Markdown 块格式面板 `layout(area, lx).panel`。
  3. 当用户在 Prompt 命令面板内点击 `@Command` 标题、Tab 分隔点、上下呼吸空行、底栏提示、空状态行或边框时，点击事件直接穿透到底层：若面板正下方恰有模板块/补充块操作按钮（`[todo] [copy] [clean] [del]`），会误触底层隐藏按钮；否则落入 `place_prompt_cursor` 将编辑器光标移到底层被遮挡的文本行，导致正在使用的命令面板瞬间被关闭。
- **修复实现**：
  - 在 `src/ui/prompt/panels.rs` 新增并导出 `command_panel_contains`，并在 `src/event/mouse/hit.rs` 新增 `prompt_command_panel_contains`；
  - 在 `src/event/mouse/press.rs` 的 `panel_item_at` 之后增加 `prompt_command_panel_contains` 分支，命中面板矩形其余区域时聚焦 Prompt 并吞掉点击；
  - 在 `src/ui/main_content/lx.rs::command_panel_contains` 中补入 `layout(area, lx).panel`；
  - 新增回归测试 `clicking_prompt_command_panel_chrome_does_not_penetrate_to_editor_cursor` 验证。

---

### P1-2：Prompt `@` 提及面板与 `/` 面板的空状态/占位行未设置 `hide_active_marker`，被误渲染为高亮选中态

- **关键点**：
  - `src/ui/prompt/panels.rs:280-375, 674-702`（`slash_panel_data`、`slash_view`、`slash_item_at`、`mention_view`）
  - `src/ui/widgets/command_panel.rs:667-768`（`render_items`、`render_item`）
- **错误原因**：
  1. `lx` 的 `@` 与 `/` 面板在空状态/占位行时设置了 `hide_active_marker: true`，而 Prompt 的 `mention_view` 与 `slash_view` 使用了默认的 `hide_active_marker: false`。
  2. 在有边框面板（`!view.borderless`）下，`active == true` 会将整行铺满 `style::overlay_selection()`（`Indexed(240)` 中灰底 + `Indexed(255)` 亮白字）。导致 Prompt `@` 面板的 `scanning…`、`no matches`、`no matching folders`、`no skills available`、`no MCP servers configured` 以及 `/` 面板的 `no git worktrees available` 占位行被整行高亮为选中态，且点击 `/` 占位行还会被 `slash_item_at` 命中。
- **修复实现**：
  - 在 `src/ui/prompt/panels.rs` 的 `mention_view` 中设置 `hide_active_marker: matches!(data.rows.first(), Some(MentionRowText::Empty(_)))`；
  - 在 `slash_panel_data` 中识别 `SlashInsertion::Placeholder`，在 `slash_view` 设置 `hide_active_marker: data.placeholder`，并在 `slash_item_at` 中对 `data.placeholder` 返回 `None`；
  - 在 `command_panel::render_item` 中，当 `placeholder`（`view.hide_active_marker`）为 `true` 时使用 `style::muted()` 置灰渲染文案。

---

### P1-3：`lx` 的 Markdown 块格式面板 `draw_panel` 未按面板宽度裁剪文本，且 `lx::render` 未隐藏被窗格内 Toast 覆盖的硬件光标

- **关键点**：
  - `src/ui/main_content/lx.rs:130-138, 949-995`
- **错误原因**：
  1. `draw_panel` 新增底栏提示 `text::COMMAND_PANEL_FOOTER` 及标题、候选预览绘制时使用了不裁剪宽度的 `grid.put`；当窗格宽度窄于 36 列时，文字会写穿面板右边框 `│` 并溢出到面板右侧。
  2. `lx::render` 调用 `toast::draw(buf, area, toast)` 时丢弃了返回的 `Option<Rect>`，未过滤被窗格内 Toast 覆盖的硬件光标坐标。
- **修复实现**：
  - 在 `draw_panel` 中改用 `grid.put_clipped` 将标题、候选标签、预览与底栏严格限制在 `rect.x + 2 .. rect.right() - 2` 内；
  - 在 `lx::render` 中用 `toast::draw` 返回的矩形过滤 `cursor`，被遮挡时隐藏硬件光标；
  - 在 `lx_md_block_panel_renders_square_borders_and_selection_highlight` 中补充窄窗格右边框完整性断言。

---

### P2-1：引用文件夹目录弹窗（`ReferencedFolderMenu`）目录图标仍硬编码为蓝色 `style::accent()`，且极矮窗口下底栏覆盖底边框

- **关键点**：
  - `src/ui/overlay/referenced/menu.rs:176-193, 243-250`
- **错误原因**：
  1. `style::directory()` 改为目录黄（`Color::Yellow`）后，`@` 提及面板与 `ReferencedFolderSearch` 的目录图标和名称均使用 `style::directory()`，唯独 `ReferencedFolderMenu` 第 248 行将目录图标 `text::REFERENCED_DIR_ICON` 硬编码为 `base.patch(style::accent())`（蓝色），导致同一行图标为蓝、名称为黄。
  2. `render_referenced_menu_footer` 在 `layout.area.height < 7` 时将底栏绘制在 `layout.area.bottom() - 1`（底边框 `└───┘` 所在行），破坏了底边框。
- **修复实现**：
  - 将 `ReferencedFolderMenu` 目录图标样式统一改为 `base.patch(style::directory())`；
  - 在 `render_referenced_menu_footer` 中仅当 `layout.area.height >= 7 && layout.area.width >= 4` 时绘制内框底栏行。

---

### P2-2：特性文档（`design.md` / `task.md`）与规范文档（`tui-design-requirements.md`）同步更新

- **关键点**：
  - `docs/feature/square-borders-and-overlay-style/design.md`
  - `docs/feature/square-borders-and-overlay-style/task.md`
  - `docs/standards/tui-design-requirements.md`
- **错误原因**：
  - 多轮视觉迭代后，`design.md` 前半部分与 `tui-design-requirements.md` 仍残留早期过渡方案（横线穿透标题、固定 12 行高度、带 `[✕]` 的模态与 Toast、边框随焦点变色、目录强调色等）。
- **修复实现**：
  - 全面更新 `design.md`、`task.md` 与 `docs/standards/tui-design-requirements.md`，使其与最终落地实现及本次 Review 修复完全一致。
