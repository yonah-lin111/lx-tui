# 全局直角边框、浮层透明背景与 lx 命令面板顶部空行 — 执行任务清单

- **任务目录**：`docs/feature/square-borders-and-overlay-style/`
- **设计文档**：`docs/feature/square-borders-and-overlay-style/design.md`
- **工程约束**：严格遵守 `AGENTS.md` 及 `docs/standards/*` 规范；保持 `app/` 与 `ui/` 边界清晰；生产代码禁止 `unwrap()` / `expect()`；所有样式集中于 `src/ui/style.rs`；前端资源遵循 `docs/standards/html-assets.md`。

---

## 阶段一：需求质询与方案设计（已完成）

- [x] 使用 `codegraph` 与 `codebase-memory-mcp` 检索全项目边框类型（`BorderType::Rounded`、手绘圆角字符 `╭ ╮ ╰ ╯`、`assets/settings.css`）、`src/ui/main_content/lx/` 命令面板布局、以及 `style::overlay_panel()` 浮层背景色调用链
- [x] **Grill Me 问题 1**：确认 `src/ui/main_content/lx/` 的 `@` 和 `/` 命令面板上方空行位置为**面板顶部蓝色横线的外侧上方**（即清底区域顶部多留 1 行空行，把上方聊天内容与面板顶线隔开）
- [x] **Grill Me 问题 2**：确认移除所有浮层（模态框、右键菜单、页码菜单、引用文件夹弹窗、Prompt 命令/提及/块格式面板、Tooltip）的面板背景色（保持 `clear` 清底 + 透明背景），并保留选中行/确认行的行高亮底色
- [x] **Grill Me 问题 3**：确认 TUI 全部边框（含 Prompt 内部模板块/补充块）改为直角 `┌ ┐ └ ┘`，且 `assets/settings.css` 移除所有容器/输入框/按钮/卡片/菜单/标签/滚动条的圆角（仅保留两处状态小圆点的 `border-radius: 50%`）
- [x] 编写详细设计文档 `docs/feature/square-borders-and-overlay-style/design.md` 与任务文档 `docs/feature/square-borders-and-overlay-style/task.md`

---

## 阶段二：全局边框调整为直角（待用户确认后执行）

### 2.1 TUI `Block::bordered()` 组件改用直角（默认 `BorderType::Plain`）
- [x] **`src/ui/sidebar.rs`**：`render` 移除 `.border_type(BorderType::Rounded)` 及未使用的 `BorderType` 导入
- [x] **`src/ui/panes.rs`**：`render_panes` 移除 `.border_type(BorderType::Rounded)` 及未使用的 `BorderType` 导入
- [x] **`src/ui/right_panel.rs`**：`render` 移除 `.border_type(BorderType::Rounded)` 及未使用的 `BorderType` 导入
- [x] **`src/ui/widgets/modal.rs`**：`render` 移除 `.border_type(BorderType::Rounded)` 及未使用的 `BorderType` 导入
- [x] **`src/ui/widgets/menu.rs`**：`render_with_confirm` 移除 `.border_type(BorderType::Rounded)` 及未使用的 `BorderType` 导入
- [x] **`src/ui/widgets/command_panel.rs`**：`render` 移除 `.border_type(BorderType::Rounded)` 及未使用的 `BorderType` 导入
- [x] **`src/ui/overlay/page_menu.rs`**：`render_page_menu` 移除 `.border_type(ratatui::widgets::BorderType::Rounded)`
- [x] **`src/ui/overlay/referenced/menu.rs`**：`render_referenced_menu` 移除 `.border_type(BorderType::Rounded)` 及未使用的 `BorderType` 导入
- [x] **`src/ui/overlay/referenced/search.rs`**：`render_referenced_search` 移除 `.border_type(BorderType::Rounded)` 及未使用的 `BorderType` 导入
- [x] **`src/ui/toast.rs`**：`render` 移除 `.border_type(BorderType::Rounded)` 及未使用的 `BorderType` 导入
- [x] **`src/ui/tooltip.rs`**：`render` 移除 `.border_type(BorderType::Rounded)` 及未使用的 `BorderType` 导入

### 2.2 TUI 手绘边框字符改为直角 `┌ ┐ └ ┘`
- [x] **`src/ui/main_content/lx.rs`**：`draw_box` 四角由 `'╭' / '╮' / '╰' / '╯'` 改为 `'┌' / '┐' / '└' / '┘'`，并清空框内单元格为空格 `' '` 与 `Style::default()`
- [x] **`src/ui/main_content/lx/toast.rs`**：`draw_border` 四角由 `'╭' / '╮' / '╰' / '╯'` 改为 `'┌' / '┐' / '└' / '┘'`
- [x] **`src/ui/prompt.rs`**：`paint_block_border` 中 `BlockRole::Start` 与 `BlockRole::End` 四角由 `'╭' / '╮' / '╰' / '╯'` 改为 `'┌' / '┐' / '└' / '┘'`

### 2.3 浏览器设置页样式改为直角
- [x] **`assets/settings.css`**：移除 `fieldset`、`input, textarea`、`button.primary`、`button.ghost`、`button.danger`、`.nav-item`、`.nav-item .tag`、`.nav-item.new`、`.editor-card`、`.editor-empty`、`.editor-body::-webkit-scrollbar-thumb`、`.select-button`、`.select-menu`、`.select-option`、`.toast`、`.provider-list::-webkit-scrollbar-thumb`、`.input-action`、`.card`、`.model-card`、`.fetch-candidates` 共 20 处的 `border-radius` 属性；仅保留 `nav.side button::before` 与 `.status-dot` 的 `border-radius: 50%`

---

## 阶段三：`src/ui/main_content/lx/` 命令面板上方增加 1 行空行（待用户确认后执行）

- [x] **`src/ui/widgets/command_panel.rs`**：
  - 新增 `const TOP_BLOCK_ROWS: u16 = 2;`（1 行顶部空行 + 1 行顶部蓝色分割线），并将 `MIN_BORDERLESS_HEIGHT` 更新为 `TOP_BLOCK_ROWS + 1`（3 行）
  - `layout` 中无边框模式 `frame_rows` 设为 `TOP_BLOCK_ROWS + footer_rows`，`tabs` 纵坐标在 `view.borderless` 下设为 `rect.y.saturating_add(1)`
  - `render_top_rule` 将蓝色 `─` 横线、左侧 `title` 与右侧 `right_title` 绘制在 `rect.y.saturating_add(1)` 行（`rect.y` 行由 `clear::render_buffer` 保持为空白行）
  - `borderless_inner_rect` 起点 `y` 设为 `rect.y.saturating_add(TOP_BLOCK_ROWS)`，高度扣除 `TOP_BLOCK_ROWS` 与 `footer_rows`
- [x] **`src/ui/main_content/lx/mention.rs`、`src/ui/main_content/lx/slash.rs`、`src/ui/main_content/lx/refer.rs`**：
  - 将 `pub(super) const PANEL_BORDER_ROWS: u16` 从 `1` 更新为 `2`（同步生效于 `mention.rs`、`slash.rs` 与 `refer.rs` 的最大高度 `cap` 计算）
  - 在 `CommandPanelView` 新增 `fixed_height: bool` 并在 `mention.rs`、`slash.rs`、`refer.rs` 中设为 `true`，使 `@` 和 `/` 命令面板高度固定为最大高度（12 行，空间不足时按可用空间夹取），过滤或切换 Tab 时不再自适应变化
- [x] **`src/ui/main_content/lx.rs`**：
  - `paste_panel_rows` 中 `if region_h < 2` 更新为 `if region_h < 3`

---

## 阶段四：移除浮层背景颜色（待用户确认后执行）

- [x] **`src/ui/style.rs`**：
  - 将 `pub fn overlay_panel() -> Style` 改为返回 `Style::default()`（无背景色、终端默认前景色），保留 `overlay_selection()` 与 `menu_confirm()`
- [x] **`src/ui/widgets/modal.rs`、`src/ui/widgets/menu.rs`、`src/ui/widgets/command_panel.rs`、`src/ui/tooltip.rs`**：
  - 移除 `Block::bordered()` 上的 `.style(style::overlay_panel())`
  - `command_panel.rs` 滚动条统一使用透明底 `scrollbar::render_buffer(buf, scrollbar)`
- [x] **`src/ui/overlay/page_menu.rs`、`src/ui/overlay/referenced/menu.rs`、`src/ui/overlay/referenced/search.rs`**：
  - 移除 `Block::bordered()` 上的 `.style(style::overlay_panel())`
  - `page_menu.rs` 中 `render_chip` 禁用态改用 `style::muted_on_border()`、启用态改用 `style::accent()`；未选中页行使用 `style::text()`
  - `referenced/menu.rs` 与 `referenced/search.rs` 中未悬停/未选中行使用 `style::text()`，滚动条改用透明底 `widgets::scrollbar::render(frame, &scrollbar)`

---

## 阶段五：测试更新、文档同步与精准验证（待用户确认后执行）

- [x] 更新单元测试与集成测试中的圆角字符断言、无边框面板高度与顶线行号断言、浮层透明背景断言及 `settings.css` 样式断言
- [x] 同步更新 `docs/standards/tui-design-requirements.md`
## 阶段六：补充需求（顶栏/底栏框、快捷键格式统一、移除模板块底色）

- [x] 为模态弹窗、右键菜单、页码菜单、引用文件夹弹窗、Toast、Tooltip、Prompt 命令面板与 `lx` md 块面板统一顶栏/底栏（标题行保留左右边框、不画横线，标题下方与快捷键上方各一条 `├───┤` 分割线，内容与分割线之间不留空行）
- [x] 将所有快捷键文案统一为 `符号 功能`（纯文本底栏用 ` · ` 分隔，可点击按钮保留 `[符号 功能]`）
- [x] 移除 `src/ui/prompt.rs` 中模板块/补充块的整行背景色渲染 `paint_block_background`
- [x] 更新并补充对应测试，完成 `cargo fmt`、`cargo clippy`、`cargo test` 与 git commit

