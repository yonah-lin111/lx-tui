# 全局直角边框、浮层样式重构与命令面板统一 — 执行任务清单

- **任务目录**：`docs/feature/square-borders-and-overlay-style/`
- **设计文档**：[design.md](design.md)
- **评审报告**：[review.md](review.md)
- **工程约束**：严格遵守 `AGENTS.md` 及 `docs/standards/*` 规范；保持 `app/` 与 `ui/` 边界清晰；生产代码禁止 `unwrap()` / `expect()`；所有样式集中于 `src/ui/style.rs`；前端资源遵循 `docs/standards/html-assets.md`。

---

## 阶段一：需求质询与方案设计（已完成）

- [x] 使用 `codegraph` 与 `codebase-memory-mcp` 检索全项目边框类型（`BorderType::Rounded`、手绘圆角字符 `╭ ╮ ╰ ╯`、`assets/settings.css`）、`src/ui/main_content/lx/` 命令面板布局、以及 `style::overlay_panel()` 浮层背景色调用链
- [x] 确认 TUI 全部边框（含 Prompt 内部模板块/补充块）改为直角 `┌ ┐ └ ┘`，且 `assets/settings.css` 移除所有容器/输入框/按钮/卡片/菜单/标签/滚动条的圆角（仅保留两处状态小圆点的 `border-radius: 50%`）
- [x] 确认移除所有浮层（模态框、右键菜单、页码菜单、引用文件夹弹窗、Prompt 命令/提及/块格式面板、Tooltip）的面板背景色（保持 `clear` 清底 + 透明背景），并保留选中行/确认行的行高亮底色
- [x] 确认对齐 `herdr-master` 重构模态弹窗、右键菜单、Toast、分区静态边框及 `lx` / Prompt 命令面板样式
- [x] 编写详细设计文档 `docs/feature/square-borders-and-overlay-style/design.md` 与任务文档 `docs/feature/square-borders-and-overlay-style/task.md`

---

## 阶段二：全局边框调整为直角与静态暗灰分区边框（已完成）

### 2.1 TUI `Block::bordered()` 组件改用直角（默认 `BorderType::Plain`）
- [x] **`src/ui/sidebar.rs`、`src/ui/panes.rs`、`src/ui/right_panel.rs`**：移除 `.border_type(BorderType::Rounded)`
- [x] **`src/ui/widgets/{modal,menu,command_panel}.rs`**：移除 `.border_type(BorderType::Rounded)`
- [x] **`src/ui/overlay/{page_menu,referenced/menu,referenced/search}.rs`**：移除 `.border_type(BorderType::Rounded)`
- [x] **`src/ui/toast.rs`、`src/ui/tooltip.rs`**：移除 `.border_type(BorderType::Rounded)`

### 2.2 TUI 手绘边框字符改为直角 `┌ ┐ └ ┘` 与移除块背景色
- [x] **`src/ui/main_content/lx.rs`**：`draw_box` 四角改为 `'┌' / '┐' / '└' / '┘'`，并清空框内单元格为空格 `' '` 与 `Style::default()`
- [x] **`src/ui/prompt.rs`**：`paint_block_border` 四角改为 `'┌' / '┐' / '└' / '┘'`，移除模板块与补充块的整行背景填充 `paint_block_background`

### 2.3 移除分区边框聚焦变色
- [x] **`src/ui/style.rs`**：`style::border(_focused: bool)` 恒返回 `muted()`（静态暗灰），移除左侧栏、中间主窗格与右侧容器边框的聚焦变青；边框内嵌标题（`border_title`）与标签前缀（`border_label_prefix`）保持静态淡青（`Cyan + DIM`）

### 2.4 浏览器设置页样式改为直角
- [x] **`assets/settings.css`**：移除 20 处容器、输入框、按钮、卡片、下拉菜单、标签、Toast 与滚动条滑块的 `border-radius`；仅保留 `nav.side button::before` 与 `.status-dot` 的 `border-radius: 50%`

---

## 阶段三：对齐 `herdr-master` 重构浮层（模态框、右键菜单、Toast）（已完成）

- [x] **浮层透明底色（`src/ui/style.rs`）**：`overlay_panel()` 返回 `Style::default()`（`Color::Reset`），配合 `clear` 清底保持终端透明背景
- [x] **右键菜单（`src/ui/widgets/menu.rs`）**：移除顶部标题栏与 `├───┤` 分割线，改为纯列表框（`height = items + 2`），每项左侧 1 列内边距（`│ Rename │`），点击外部或按 `Esc` 关闭
- [x] **模态弹窗（`src/ui/widgets/modal.rs`、`src/ui/overlay/{dialog,worktree,save_draft,referenced/*}.rs`）**：
  - 打开模态时调用 `modal::dim_background` 压暗底层画面
  - 移除 `├───┤` 分割线与右上角 `[✕]` 按钮；标题位于内框首行粗体显示（危险确认弹窗 `render_danger` 使用红框 + 红色粗体标题），标题下留 1 行呼吸空白
  - 单行输入框整行铺暗色 `surface` 底条（`style::modal_input()`，`Indexed(236)`）
  - 底部按钮改为实心胶囊按钮（` ↵ save ` / ` ^c clear ` / ` esc cancel `，无方括号）：主按钮 `button_primary()`（青底黑字粗体）、危险主按钮 `button_danger()`（红底白字粗体）、次按钮 `button_secondary()`（`Indexed(238)` 暗灰底白字粗体）
- [x] **Toast（`src/ui/toast.rs`、`src/ui/main_content/lx/toast.rs`、`src/event/mouse/press.rs`）**：
  - 移除 `├───┤` 分割线与 `[✕]` 关闭按钮，点击 Toast 本身或外部即关闭
  - 无标题 Toast 为 3 行高：`│ ● <message> │`（圆点取语义色 Cyan/Yellow/Red，正文加粗）
  - 有标题 Toast 为 4 行高：首行 `│ ● <title> │`（圆点语义色，标题加粗），次行 `│   <message> │`（缩进 2 列与标题对齐，次要色 `muted`）

---

## 阶段四：`lx` 与 Prompt 命令面板统一与目录配色调整（已完成）

- [x] **`lx` 无边框命令面板顶部对齐（`src/ui/widgets/command_panel.rs`、`src/ui/main_content/lx/{mention,slash,refer}.rs`、`src/ui/main_content/lx.rs`）**：
  - `lx` 的 `@`、`/`、粘贴与快捷引用面板采用 `borderless: true`：第 0 行为独立纯粹的强调色横线 `─`（作为浮层上边界，无左右和底框），第 1 行为干净的标题/Tab 文本行（无横线穿透文字，与 Prompt 内框首行一致）
  - 面板高度按候选数自适应收缩（上限 `MAX_POPUP_ROWS`），条目左侧使用 `❯` 标记（当前生效项 `current` 用蓝色 `❯` 与蓝色名称，正在选择项 `active` 用默认色 `❯`）
- [x] **`@` 命令面板 `@Command` 标题与目录黄色（`src/ui/widgets/command_panel.rs`、`src/ui/prompt/panels.rs`、`src/ui/main_content/lx/mention.rs`、`src/ui/style.rs`）**：
  - 在 `@` 面板 Tab（`Files · refDir · Skills · MCP`）左侧增加粗体 `@Command` 标题（`lx` 与 Prompt 均对齐）
  - 将 `style::directory()` 由蓝色（`Cyan`）改为目录黄（`Color::Yellow`），普通文件保持终端默认文字色

---

## 阶段五：Code Review 缺陷修复与全量验证（已完成）

- [x] **编写评审报告**：输出 `docs/feature/square-borders-and-overlay-style/review.md`（按 P0 → P2 排序记录关键点、错误原因与修复方案）
- [x] **修复 P0-1**：`command_panel::panel_width` 在 `tabs` 与 `title` 共存时计入 `title` 宽度；退化态（`!roomy`）下不生成与首条候选重叠的 `layout.tabs`
- [x] **修复 P0-2**：`src/ui/mod.rs` 将 `toast::render` 移至 `overlay::render` 之后（防止 `dim_background` 压暗或覆盖全局 Toast）并统一过滤被遮挡的硬件光标；`press.rs` 与 `hover.rs` 将全局 Toast 拦截置于 `state.overlay.is_some()` 之前
- [x] **修复 P1-1**：新增 `ui::prompt::command_panel_contains` 与 `hit::prompt_command_panel_contains`，在 `press.rs` 拦截 Prompt 命令面板非条目区域的左键点击防止穿透；`lx::command_panel_contains` 补入 `lx.panel`
- [x] **修复 P1-2**：Prompt `mention_view` 与 `slash_view` 对空状态/占位行设置 `hide_active_marker: true`，并在 `command_panel::render_item` 中以 `style::muted()` 置灰渲染占位行
- [x] **修复 P1-3**：`lx::draw_panel` 改用 `grid.put_clipped` 防止窄窗格下文字写穿右边框 `│`；`lx::render` 过滤被窗格内 `lx.toast` 覆盖的硬件光标
- [x] **修复 P2-1 & P2-2**：`ReferencedFolderMenu` 目录图标统一改用 `style::directory()` 且退化态不覆盖底边框；同步更新 `design.md`、`task.md` 与 `docs/standards/tui-design-requirements.md`
- [x] **验证结果**：`rtk cargo fmt`、`rtk cargo clippy --all-targets -- -D warnings`、`rtk cargo test` 全量通过
- [x] **合并入主工作区**：合并分支至主工作区 `dev`，移除工作区 `.worktrees/square-borders-and-overlay-style`、分支 `feat/square-borders-and-overlay-style` 与隔离库
