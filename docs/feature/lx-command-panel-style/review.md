# lx 输入框命令面板样式重设计与 `@` 面板双 Tab — 代码评审报告 (Code Review)

> 评审范围：`.worktrees/lx-command-panel-style`（分支 `feat/lx-command-panel-style`，已合入 `dev`）
>
> 检索与分析工具：`codegraph`（调用链与影响面分析）、`codebase-memory-mcp`（变更爆炸半径与全量符号检索）
>
> 优先级定义：
> - **P0**：功能性缺陷 / 交互穿透与状态失步 Bug（直接影响用户点击行为与面板实时刷新）
> - **P1**：布局遮挡 / 视口与几何边界计算偏差（影响窄窗格显示、过滤体验与组件通用性）
> - **P2**：死代码清理与规范文档同步（移除废弃常量、更新设计规范文档）
>
> 结论：P0 / P1 / P2 全部已修复，回归测试已补齐；`cargo fmt`、`cargo clippy --all-targets -- -D warnings`、`cargo test` 全绿。

---

## P0-1：`lx` 命令面板非条目区域点击穿透底层控件，且路径粘贴面板缺失鼠标命中路由

### 1. 关键点
- **涉及文件**：
  - `src/event/mouse/press.rs`（`handle_mouse_down_left` L614-690）
  - `src/event/mouse/hover.rs`（`handle_mouse_moved` L165-185）
  - `src/event/mouse/hit.rs`
  - `src/ui/main_content/lx.rs`（`paste_layout` / `paste_item_at` / `command_panel_contains`）
  - `src/app/update/pane.rs`（`hover_lx_paste` / `select_lx_paste`）

### 2. 错误原因
1. `src/event/mouse/press.rs` 在处理 `lx` 窗格内鼠标左键按下时，仅检查了 `mention_tab_at`、`mention_item_at`、`refer_item_at` 与 `slash_item_at`。
2. 当用户点击落在已打开的 `@`、`/` 或快捷引用面板矩形（`layout.rect`）内部，但位置处于**顶部蓝色横线空白区**、**空状态/加载状态占位行**（如 `scanning…`、`no matches`、`no matching folders`、`No model configured`、`no sessions yet`）或**右侧滚动条列**时，上述命中函数均返回 `None`。
3. 事件未在浮层处终止，而是直接向下穿透到 `chat_hit`、`entry_hit`、`card_target_at` 与 `begin_lx_selection`，导致点击面板空白/空态行会误切换底层欢迎页的 `Native / Hybrid / Automation Agent` 入口、误折展历史工具卡片或启动底层文本拖拽选区，违反 `docs/standards/tui-design-requirements.md` 中“浮层打开期间鼠标点击只作用于浮层，不穿透底层”的约束。
4. 此外，`lx` 的路径粘贴面板（`render_paste_panel`）仅实现了渲染，未暴露 `paste_layout` / `paste_item_at`，导致在 `lx` 中粘贴路径弹出面板后无法用鼠标悬停或点击选择项。

### 3. 修复方案与状态（已修复）
- [x] 在 `src/ui/main_content/lx.rs` 中抽取 `paste_layout` 与 `paste_item_at`，并提供统一的 `command_panel_contains(area, lx, column, row)` 判断坐标是否落在当前打开的任一 `lx` 输入框命令面板（粘贴 / `@` 提及 / 快捷引用 / `/` 斜杠）矩形内。
- [x] 在 `src/app/update/pane.rs`、`src/event/mouse/hit.rs`、`src/event/mouse/hover.rs` 与 `src/event/mouse/press.rs` 中补齐 `lx` 粘贴面板的悬停与点击选择路由。
- [x] 在 `src/event/mouse/press.rs` 中，当点击未命中具体可点条目但落在 `command_panel_contains` 矩形内时直接消费并返回，彻底阻断向底层历史/入口/选区的点击穿透。
- [x] 在 `src/event/tests/lx_view.rs` 新增点击面板顶栏与空态行不穿透底层入口的回归测试。

---

## P0-2：引用文件夹列表异步加载或增删后，`sync_lx_mention_root` 未返回变更标记导致画面不重绘

### 1. 关键点
- **涉及文件**：
  - `src/app/state/lx/mention.rs`（`LxMention::sync_referenced_folders`）
  - `src/app/state/lx/panels.rs`（`Lx::sync_referenced_folders`）
  - `src/app/update/pane.rs`（`sync_lx_mention_root`）

### 2. 错误原因
1. `LxMention::sync_referenced_folders` 与 `Lx::sync_referenced_folders` 返回值类型为 `()`，未向调用方报告 `referenced_folders` 快照是否发生变化。
2. 在事件主循环 `src/event/mod.rs` 中，`persistence.pump(state)` 从 SQLite 载入或更新 `state.referenced.folders` 后，紧接着调用 `update::sync_all_lx_mention_roots(state)`。
3. 由于 `sync_lx_mention_root` 仅返回 `mention_root_changed || project_root_changed`，当工作区路径未变而仅 `referenced.folders` 发生变化时，`sync_all_lx_mention_roots` 返回 `false`，无法触发 `dirty = true`，导致已停留在 `refDir` Tab 的 `lx` 提及面板不能及时刷新。

### 3. 修复方案与状态（已修复）
- [x] 将 `LxMention::sync_referenced_folders` 与 `Lx::sync_referenced_folders` 改为返回 `bool`（当 `self.referenced_folders != folders` 时更新并返回 `true`，否则返回 `false`）。
- [x] 在 `sync_lx_mention_root` 中将 `pane.lx.sync_referenced_folders(&referenced)` 的返回值并入布尔或结果（`referenced || mention || project`），确保引用文件夹列表变化时立即触发重绘。

---

## P1-1：窄窗格下顶栏 `right_title` 挤丢左侧 `Files`/`refDir` Tab 或覆盖左侧标题

### 1. 关键点
- **涉及文件**：
  - `src/ui/widgets/command_panel.rs`（`layout` L199-224、`render_top_rule` L299-325）

### 2. 错误原因
1. 在 `command_panel::layout` 计算顶部 `tabs` 矩形时，`right_limit` 预先无条件扣除了完整 `view.right_title` 的宽度。当用户在 `@` 面板中进入名称较长的子目录（`right_title` 为目录名）或处于窄窗格时，`right_limit` 大幅左移，导致左侧核心导航 `Files` / `refDir` Tab 因 `x + width >= right_limit` 被直接丢弃不显示。
2. 在 `render_top_rule` 绘制无边框顶栏时，右侧 `right_title` 仅检查了 `rect.width > text_w + 2`，未检查左侧 `tabs` 或 `title`（如 `/rewind` 面板的 `Rewind` 标题）已占用的右边界 `left_used_end`，导致窄宽度下右侧状态文字直接覆盖左侧标题。

### 3. 修复方案与状态（已修复）
- [x] 在 `command_panel::layout` 中，顶栏 `tabs` 仅以面板右缘 `rect.right().saturating_sub(1)` 为界，保证左侧核心导航 Tab 永远优先显示，不被右侧辅助状态挤丢。
- [x] 在 `render_top_rule`（及有边框顶栏）中，记录左侧 `tabs` 或 `title` 的实际右边界 `left_end`；绘制 `right_title` 时要求其左起点必须至少位于 `left_end + 1`（中间至少保留 1 列 `─` 分隔线），空间不足时按剩余预算截断或整段隐藏，绝不覆盖左侧标题或 Tab。
- [x] 在 `src/ui/widgets/command_panel/tests.rs` 新增窄宽度下 `tabs`/`title` 与长 `right_title` 共存不覆盖、不丢 Tab 的回归测试。

---

## P1-2：`LxMention::refresh` 在 `RefDir` Tab 下修改过滤词时未重置滚轮视口且高亮越界直接归零

### 1. 关键点
- **涉及文件**：
  - `src/app/state/lx/mention.rs`（`LxMention::refresh` L260-285）

### 2. 错误原因
1. 用户在 `RefDir` Tab 下使用鼠标滚轮滚动产生显式视口 `ref_viewport = Some(v)` 后，若继续键入或退格修改 `@query`，只要新过滤结果数量 `filtered_count > v`，`refresh` 就未清除 `ref_viewport`，导致过滤词已变但列表视口仍卡在旧的滚轮偏移位置。
2. 当过滤结果缩减使得原 `ref_active >= filtered_count` 时，`refresh` 直接将 `ref_active` 归零（`0`），而非像 `Files` Tab（`active.min(items.len() - 1)`）一样钳制到有效末项 `filtered_count.saturating_sub(1)`。

### 3. 修复方案与状态（已修复）
- [x] 在 `LxMention::refresh` 中检测 `trigger.query` 是否发生变化；若查询词变化则重置 `self.ref_viewport = None`。
- [x] 当 `filtered_count > 0` 且 `self.ref_active >= filtered_count` 时，将 `self.ref_active` 与 `self.ref_anchor` 钳制到 `filtered_count - 1`；仅当 `filtered_count == 0` 时归零并清空 `ref_viewport`。
- [x] 在 `src/app/state/lx/mention/tests.rs` 补充查询词变化重置 `ref_viewport` 与钳制 `ref_active` 的单元测试。

---

## P1-3：`command_panel::panel_width` 在 `borderless` 模式下多计入 4 列边框与内边距

### 1. 关键点
- **涉及文件**：
  - `src/ui/widgets/command_panel.rs`（`panel_width` L380-445）

### 2. 错误原因
1. `panel_width` 末尾固定执行 `((content + 4) as u16)`，其中 `+ 4` 来自有边框模式下的左右边框（2 列）+ `inner` 偏移（1 列）+ 行首空格（1 列）。
2. 在 `borderless = true` 模式下，面板无左右边框且条目直接从 `area.x` 绘制 `❯ `（`marker` 已计入 `content`），固定 `+ 4` 会使自适应宽度面板多出 4 列尾部空白。

### 3. 修复方案与状态（已修复）
- [x] 在 `panel_width` 中按 `view.borderless` 区分额外边框开销（`let frame_pad = if view.borderless { 0 } else { 4 };`），保持自适应宽度紧致准确。

---

## P2：清理废弃文案常量并同步 TUI 设计规范文档

### 1. 关键点
- **涉及文件**：
  - `src/ui/main_content/lx/mention.rs`
  - `src/ui/text.rs`、`src/ui/text/tests.rs`
  - `src/ui/main_content/lx/tests.rs`、`src/ui/main_content/lx/tests/slash_panels.rs`
  - `docs/standards/tui-design-requirements.md`

### 2. 错误原因
1. `lx` 输入框的 `@` 面板顶栏已改为常驻 `Files` / `refDir` 双 Tab，`src/ui/main_content/lx/mention.rs` 传入的 `title: Some(text::LX_MENTION_PANEL_TITLE)` 永远被 `tabs` 覆盖而不生效。
2. `lx` 输入框命令面板移除底边框与 footer 后，`src/ui/text.rs` 中的 `LX_SLASH_PANEL_FOOTER`、`LX_REWIND_PANEL_FOOTER`、`LX_HISTORY_PANEL_FOOTER`、`LX_MENTION_PANEL_TITLE`、`MENTION_PANEL_FOOTER` 在生产代码中已无任何引用，仅残留在测试中。
3. `docs/standards/tui-design-requirements.md` 中关于 `lx` 输入框 `@` 提及面板的描述仍写着旧版“加边框 2 行、顶边左侧标题 `@Command`、底边左侧提示 `[open ⇧↵] [back ^z]`”，未同步为最新的无边框顶线样式与双 Tab 交互。

### 3. 修复方案与状态（已修复）
- [x] 将 `src/ui/main_content/lx/mention.rs` 的 `title` 设为 `None`，移除 `src/ui/text.rs` 中已废弃的 5 个面板标题/页脚常量，并同步清理 `src/ui/text/tests.rs`、`src/ui/main_content/lx/tests.rs` 与 `src/ui/main_content/lx/tests/slash_panels.rs` 中针对死常量的断言。
- [x] 同步更新 `docs/standards/tui-design-requirements.md` 中关于 `lx` 输入框 `@` / `/` 命令面板样式与 Tab 切换的规范说明。
