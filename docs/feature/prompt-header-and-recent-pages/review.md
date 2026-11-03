# Prompt 顶部控件换位、页面标题展示与底部 Recent Pages 抽屉 代码评审报告 (Code Review)

> **状态**：**全部 P0–P3 项已修复验证并合入 `dev` 分支**（2026-11-03）
> **评审工作区**：`.worktrees/prompt-header-and-recent-pages`（分支 `feat/prompt-header-and-recent-pages`）
> **关联文档**：`docs/feature/prompt-header-and-recent-pages/design.md`、`docs/feature/prompt-header-and-recent-pages/task.md`
> **评审方式**：基于 `codebase-memory-mcp` 知识图谱与 `codegraph` 全链路调用追踪，对标 `docs/standards/` 项目规范逐项审查

本次 Review 覆盖工作区分支 `feat/prompt-header-and-recent-pages` 引入的全部改动（顶部两行表头控件重排、`[rec]` 与页控件 `‹ n/m ›` 两态着色及 Tooltip、分割线标题行展示 `prompt名称 · 当前页面title` 并按模板状态为页名着色/点击循环状态、底部 Recent Pages 抽屉及键盘接管/`[x]` 关闭按钮）。

审查发现的问题按照严重级别从高到低（`P0` → `P1` → `P2` → `P3`）归类如下，明确记录**关键点**、**错误原因**与**修复方案**。

---

## 缺陷清单与优先级排序（P0 → P3）

| 编号 | 优先级 | 涉及模块 / 文件与行号 | 关键点 / 问题摘要 | 状态 |
| :--- | :--- | :--- | :--- | :--- |
| **P0-1** | **P0** | `src/event/paste.rs:138-140`<br>`src/event/mouse/hit.rs:397, 414, 451`<br>`src/event/mouse/press.rs:383-388, 448-461, 885-912`<br>`src/event/mouse/drag.rs:266, 433`<br>`src/event/overlay.rs:480, 529, 555`<br>`src/ui/tooltip.rs:219` | 底部 Recent 抽屉展开时，上方视图的模板块/补充块按钮、命令面板、右键菜单、树拖拽及 Tooltip 仍使用未扣除抽屉高度的完整内容区，导致点击抽屉条目右端或 `[x]` 会穿透误触发不可见的 `[✕]` 块删除按钮，右键穿透打开隐藏行菜单 | 已修复 |
| **P0-2** | **P0** | `src/layout/prompt.rs:439-470`<br>`src/app/update/prompt/recent.rs:29-31, 55-71`<br>`src/event/input.rs:58-80, 218-221`<br>`src/ui/prompt_frame.rs:234-240`<br>`src/app/update/pane.rs:793-806` | 面板过矮导致抽屉被半屏规则压制未渲染（`recent_divider: None`）时，`recent_open` 仍保持为 `true`，导致不可见的抽屉吞掉全部打字与粘贴输入、`Enter` 静默切页、`[rec]` 仍错误高亮为青色 | 已修复 |
| **P1-1** | **P1** | `src/layout/prompt.rs:129-157, 268-364`<br>`src/ui/prompt/header.rs:222-229`<br>`src/event/mouse/press.rs:389-447` | 工具栏页控件 `‹ n/m ›` 与左侧按钮组 `[rec]` / `[←]` / `[→]` 各自独立计算宽度阈值，窄屏（内宽 9–25 或多位页码）下页控件覆盖左侧按钮且点击被错误分派给 `Undo`/`Redo`/`Recent` | 已修复 |
| **P1-2** | **P1** | `src/app/update/prompt/recent.rs:289-295`<br>`src/app/update/prompt.rs:374, 855-931`<br>`src/app/update/prompt/pages.rs:593-673`<br>`src/app/update/workspace.rs:259-276`<br>`src/app/update/right_panel.rs:10-29` | 抽屉展开期间 800ms 自动保存触发 `page.touch(now)` 重排、或删除条目/页使列表收缩时，`recent_active` 裸索引漂移或越过 `rows.len() - 1`；切换工作区/右侧 Tab 时未清理 `prompt_recent_hover` | 已修复 |
| **P2-1** | **P2** | `src/app/prompt/pages_list.rs:18-39`<br>`src/ui/prompt_frame.rs:23-28`<br>`src/ui/prompt/recent.rs:115-141` | `page_title_status` 未校验 `span.closed` 导致页面存在未闭合 `&&&` 时标题与状态色取源不一致；`recent.rs` 窄宽度下无条件先画右侧 `[时间]` 导致 `prompt_name` 可用宽度归零被完全挤没 | 已修复 |
| **P2-2** | **P2** | `src/ui/prompt/tests.rs`（1573 行）<br>`src/ui/prompt/header.rs` | `src/ui/prompt/tests.rs` 达 1573 行超过单测试文件 1500 行红线，且 `header.rs` 子模块的专属测试未按规范拆分至 `src/ui/prompt/header/tests.rs` | 已修复 |
| **P3-1** | **P3** | `src/ui/text.rs:1130-1144`<br>`src/layout/prompt.rs:83-126, 193-213`<br>`src/ui/prompt/header.rs:1-6, 17-19, 52`<br>`docs/feature/prompt-header-and-recent-pages/{design,task}.md` | 移除标题行右侧 `[todo]/[run]/[done]` 状态按钮与旧顶边框页控件后遗留死代码（`TEMPLATE_BUTTON_*`、`template_status_label`、`page_controls*`）、冗余参数 `_status` 及过时注释/设计文档 | 已修复 |

---

## 逐项详细分析（关键点、错误原因与修复方案）

### [P0-1] 底部 Recent 抽屉展开时上方主视图命中区未扣除抽屉高度（幽灵块按钮/右键菜单/拖拽穿透）

- **关键点**：
  - 渲染路径：`src/ui/prompt_frame.rs:244`（`let text_area = split.main;`）
  - 命中路径：`src/event/paste.rs:138-140`（`prompt_text_area`）、`src/event/mouse/hit.rs:397, 414, 451`（`prompt_command_panel_contains` / `template_button_at` / `block_button_at`）、`src/event/mouse/press.rs:359-388, 448-461, 885-912`、`src/event/mouse/drag.rs:266, 433`、`src/event/overlay.rs:480, 529, 555`、`src/ui/tooltip.rs:219`
- **错误原因**：
  1. `ui::prompt_frame::render_body` 使用 `prompt_body_split` 计算出的 `split.main`（已扣除底部 `recent` 抽屉高度）渲染上方 `Editor` / `Tree` / `Pages` 视图；
  2. 但事件命中层的 `paste::prompt_text_area`、`tooltip::targets`、`drag.rs` 树拖拽落点以及 `overlay.rs` 右键菜单仍调用 `layout::prompt_content_rect(view.prompt)` 或 `layout::prompt_body_rect(...)`（未扣除抽屉高度的完整内容区）；
  3. 同时 `press.rs` 中命令面板与 `template_button_at` / `block_button_at` 排在 `prompt_recent_close_at` / `prompt_recent_row_at` / `prompt_recent_contains` 之前。当编辑器内有模板块或补充块起始行滚入 `split.main` 下方（被抽屉覆盖的行范围）时，其右对齐的 `[❐] [⌫] [✕]` 幽灵命中矩形恰好落在抽屉分割线 `[x]` 或条目右侧时间戳/圆点位置——用户点击抽屉条目右端或 `[x]` 时会直接触发 `delete_template_block` **静默删除正文模板块**；
  4. 右键点击抽屉内部时，`handle_mouse_down_right` 未拦截抽屉区域，会穿透打开上方编辑器菜单或被遮挡树/页列表行的上下文菜单（包括删除条目）。
- **修复方案**：
  1. 在 `ui::prompt_frame` 新增统一的 `prompt_main_rect(panel: Rect, state: &AppState) -> Rect`，返回 `prompt_body_split` 扣除抽屉后的 `split.main`；
  2. 将 `paste::prompt_text_area`、`ui::tooltip::targets`、`drag.rs` 树拖拽落点、`overlay.rs`（Tree / Pages / Editor 右键命中区）全部收口到 `prompt_main_rect`；
  3. 在 `press.rs` 中将 `prompt_recent_close_at` / `prompt_recent_row_at` / `prompt_recent_contains` 分支前置到命令面板与模板/补充块按钮之前；在 `open_right_click_menu` 中拦截 `prompt_recent_contains`（抽屉区域内右键直接吞掉）；在 `toggle_prompt_recent` 展开抽屉时清理编辑器浮层面板 `state.prompt.clear_panel()`。

---

### [P0-2] 面板过矮导致抽屉被半屏规则压制隐藏时，仍接管并吞掉全部键盘与粘贴输入

- **关键点**：
  - `src/layout/prompt.rs:439-470`（`prompt_body_split`）
  - `src/app/update/prompt/recent.rs:29-31, 55-71`（`prompt_recent_open`、`toggle_prompt_recent`）
  - `src/event/input.rs:58-80, 218-221`、`src/ui/prompt_frame.rs:234-240, 303-305`、`src/app/update/pane.rs:793-806`
- **错误原因**：
  1. `layout::prompt_body_split` 在内容区高度不足以容纳抽屉最低高度（有条目时 `body.height / 2 < 5`，空态时 `< 2`）时返回 `recent_divider: None`，不渲染抽屉并保留编辑器硬件光标；
  2. 然而 `update::prompt_recent_open(state)` 仅读取布尔字段 `panel.recent_open`，不感知当前面板高度是否足以显示抽屉；`render_body` 也仅按 `panel.recent_open` 将 `[rec]` 按钮染成青色；
  3. 当用户在较矮窗口下按 `Ctrl/Cmd+R`（或展开抽屉后缩小终端窗口）时，界面无抽屉且编辑器光标闪烁，但所有字符输入、粘贴、`Ctrl+S`/`Ctrl+C` 均被 `input.rs` 吞掉，按 `Enter` 更会触发 `open_active_recent_page` 静默切换到其他页面。
- **修复方案**：
  1. 在 `ui::prompt_frame` 提供 `prompt_recent_visible(panel: Rect, state: &AppState) -> bool`（校验 `prompt_recent_open(state)` 且 `split.recent_divider.is_some()`）；
  2. 在 `render_body` 中仅当 `split.recent_divider.is_some()` 时才将 `[rec]` 染为激活青色；
  3. 在 `resize_panes`（`src/app/update/pane.rs`）中，若 `panel.recent_open` 为真但当前几何下 `split.recent_divider.is_none()`，自动调用 `close_prompt_recent(state)` 收起抽屉；
  4. 在 `event/input.rs` 的键盘与粘贴拦截处同时要求 `ui::prompt_frame::prompt_recent_visible(geometry.view.prompt, state)` 为真。

---

### [P1-1] 工具栏页控件 `‹ n/m ›` 与左侧按钮组在窄宽度下互相覆盖且点击分派错位

- **关键点**：
  - `src/layout/prompt.rs:129-157`（`toolbar_page_controls`）、`src/layout/prompt.rs:268-364`（`prompt_toolbar_button_rect`）
  - `src/ui/prompt/header.rs:222-229`、`src/event/mouse/press.rs:389-447`
- **错误原因**：
  1. `toolbar_page_controls` 仅校验 `prev.x >= bar.x + 4`（只避让最左侧模式按钮 `[≡]`/`[/]`），不感知左侧是否展示了 `[rec]`、`[←]`、`[→]`；
  2. `prompt_toolbar_button_rect` 仅按固定内宽阈值（`Editor` 下 `>= 19` 显示 `[rec]`、`>= 13` 显示 `[←][→]`；`Pages` 下 `>= 9` 显示 `[rec]`）摆放左侧按钮，完全不检查右侧页控件 `‹ n/m ›` 的起点列；
  3. 在 `Editor` 内宽 13–25（或多位页码如 `10/20`）以及 `Pages` 内宽 9–12 下，后绘制的页控件直接覆盖在 `[→]`、`[←]` 或 `[rec]` 之上，而 `press.rs` 优先匹配 `prompt_toolbar_button_at`，导致点击可见的页码/箭头实际触发 `Undo`/`Redo`/`Recent`，且违背了 `[refDir]` → `[rec]` → `[←] [→]` 的窄屏隐藏顺序。
- **修复方案**：
  1. 在 `src/layout/prompt.rs` 中计算右侧保留控件（页控件 `‹ n/m ›` 或保存点 `●`）的左边界 `right_limit`；
  2. 左侧按钮严格按可用区间 `bar.x..right_limit` 与降级优先级排布：`Editor` 模式下若完整左组 `[≡] [rec] [←] [→]`（17 列）放不下，先隐藏 `[rec]` 尝试保留 `[≡] [←] [→]`（11 列）；若仍与页控件冲突，再隐藏 `[←] [→]` 仅保留 `[≡]`；`Pages` 模式下若 `[rec]` 与 `n/m` 冲突则隐藏 `[rec]`；
  3. 同时加固 `PromptToolbarButton::Save` 在 `bar.width == 0` 时返回 `None`。

---

### [P1-2] 自动保存重排或条目/页面删除后，`recent_active` 裸索引漂移或越界、跨工作区残留 `hover`

- **关键点**：
  - `src/app/update/prompt/recent.rs:289-295`（`clamp_recent_active`）
  - `src/app/update/prompt.rs:374, 855-931`（`commit_prompt_buffer`、`delete_prompt_item`、`delete_prompt_folder`）
  - `src/app/update/prompt/pages.rs:593-673`（`perform_delete_page`）
  - `src/app/update/workspace.rs:259-276`、`src/app/update/right_panel.rs:10-29`
- **错误原因**：
  1. 抽屉展开期间，若编辑器 800ms 自动保存触发 `commit_prompt_buffer` → `page.touch(now)`，当前页因 `updated_at` 变新而排到 `recent_rows` 第 0 位，但 `panel.recent_active` 仍保留重排前的整数下标，导致高亮跳到其它条目；
  2. 删除条目、文件夹或页面导致 `recent_rows` 变短时，未同步钳制 `recent_active` 与 `recent_scroll`；且 `clamp_recent_active` 未以 `total.saturating_sub(1)` 作为上限，当 `total < visible` 时 `recent_active` 可能越过最后一条，造成界面无高亮但按 `Enter` 打开末尾页；
  3. `switch_workspace` 与 `select_right_tab` 未清空全局 `state.prompt_recent_hover`。
- **修复方案**：
  1. 在 `src/app/update/prompt/recent.rs` 新增 `sync_prompt_recent_selection(state, preferred_target)` 与 `clamp_prompt_recent_bounds(state)`：在 `commit_prompt_buffer` 触发 `page.touch(now)` 前记录当前 `recent_active` 对应的 `(item_id, page_id)`，重排后自动跟随该页新下标；在条目/文件夹/页面删除及树重载后钳制 `recent_active` 与 `recent_scroll` 到 `0..rows.len()`；
  2. `clamp_recent_active` 增加 `total` 参数，严格钳制在 `scroll..=last.min(total - 1)`；
  3. 在 `switch_workspace` 与 `select_right_tab` 中复位 `state.prompt_recent_hover = None`。

---

### [P2-1] 页面标题与状态色取块不一致（未闭合 `&&&` 场景）及窄抽屉时间戳挤没名称

- **关键点**：
  - `src/app/prompt/pages_list.rs:18-39`（`page_title_status`）、`src/ui/prompt_frame.rs:23-28`（`prompt_template_status`）
  - `src/ui/prompt/recent.rs:115-141`（`paint_entry`）
- **错误原因**：
  1. `page_title_status` 查找首个 `span.kind == BlockKind::Template` 时未检查 `span.closed`，而 `prompt_template_status` 与规范均要求只认**已闭合**（`span.closed`）的 `&&&` 模板块。当正文上方有未闭合 `&&&`、下方有已闭合 `&&&` 时，页面标题与状态圆点/标题行状态色取自不同块；
  2. `src/ui/prompt/recent.rs` 绘制条目第 1 行时无条件先在右端画 `row.time_label`，再将 `name_right` 设为 `time_x - 1`。当面板较窄时，时间戳会把 `name_right` 压到 `<= x`，导致 `prompt_name` 宽度为 0、整行只剩 `❯ [MM-DD]`。
- **修复方案**：
  1. 在 `page_title_status` 中增加 `&& span.closed` 过滤，并将 `prompt_template_status` 统一委托给 `page_title_status(state.prompt.text()).1`，消除双份扫描与语义分叉；
  2. 在 `src/ui/prompt/recent.rs` 中仅当扣除左侧前缀与至少 4 列名称预算后仍放得下 `time_label` 时才绘制右侧时间戳，窄屏优先保留 `prompt_name`。

---

### [P2-2] `src/ui/prompt/tests.rs` 行数超限（1573 行 > 1500 行）且 `header.rs` 测试未随子模块拆分

- **关键点**：
  - `src/ui/prompt/tests.rs`（1573 行）、`src/ui/prompt/header.rs`
- **错误原因**：
  1. 项目规范要求单测试文件上限不超过 1500 行，且子模块测试应跟随被测子模块放入 `<module>/<submodule>/tests.rs`；
  2. `header.rs` 的标题行、工具栏按钮与页控件测试一直堆在 `src/ui/prompt/tests.rs` 中，导致根测试文件超出 1500 行上限。
- **修复方案**：
  1. 在 `src/ui/prompt/header.rs` 挂载 `#[cfg(test)] mod tests;`，新建 `src/ui/prompt/header/tests.rs`，将表头、标题行与工具栏页控件相关测试迁入其中，使两个测试文件均远低于 1500 行限制。

---

### [P3-1] 移除标题行状态按钮与旧顶边框页控件后遗留死代码、冗余参数与过时文档

- **关键点**：
  - `src/ui/text.rs:1130-1144`、`src/layout/prompt.rs:83-126`、`src/ui/prompt/header.rs:1-6, 17-19, 52`
  - `docs/feature/prompt-header-and-recent-pages/design.md`、`docs/feature/prompt-header-and-recent-pages/task.md`
- **错误原因**：
  1. 标题行移除 `[todo]/[run]/[done]` 按钮、页控件迁至工具栏后，`text::TEMPLATE_BUTTON_TODO/RUN/DONE`、`text::template_status_label`、`layout::page_controls` / `page_controls_with` 在生产代码中已无任何调用；`header::status_rect` 的 `_status: TemplateStatus` 参数冗余；
  2. `header.rs` 顶部注释及 `design.md`、`task.md` 部分章节仍残留“标题行右侧 `[todo]/[run]/[done]` 状态按钮”等旧描述。
- **修复方案**：
  1. 删除上述死代码与未使用的再导出，简化 `header::status_rect(area, data)` 签名，更新模块注释；
  2. 全面同步 `docs/feature/prompt-header-and-recent-pages/design.md` 与 `docs/feature/prompt-header-and-recent-pages/task.md`。

---

## 修复验证记录

- [x] **[P0-1]** 统一通过 `ui::prompt_frame::prompt_main_rect` 提供扣除底部抽屉后的主视图矩形，`press.rs` 抽屉分支前置，`open_right_click_menu` 拦截抽屉内右键，展开抽屉时收起候选面板；新增回归测试验证抽屉覆盖区不触发幽灵块按钮与错误右键菜单。
- [x] **[P0-2]** 新增 `ui::prompt_frame::prompt_recent_visible`，矮面板压制抽屉时不染色 `[rec]`、不拦截键盘/粘贴，且 `resize_panes` 自动复位被压制的 `recent_open`；新增矮屏按键不吞字回归测试。
- [x] **[P1-1]** `layout::prompt_toolbar_button_rect` 按页控件左边界动态避让并严格遵循 `[refDir]` → `[rec]` → `[←] [→]` 降级顺序；新增窄宽度工具栏零重叠回归测试。
- [x] **[P1-2]** 自动保存 `touch` 与条目/文件夹/页面删除后同步对齐/钳制 `recent_active` 与 `recent_scroll`，工作区与右侧 Tab 切换时清空 `prompt_recent_hover`。
- [x] **[P2-1]** `page_title_status` 统一要求 `span.closed`，窄抽屉优先保留 `prompt_name`。
- [x] **[P2-2]** 拆分 `src/ui/prompt/header/tests.rs`，`src/ui/prompt/tests.rs` 行数降至规范上限以内。
- [x] **[P3-1]** 清理遗留死代码与冗余参数，同步更新 `design.md` 与 `task.md`。
- 质量门禁：`cargo fmt`、`cargo clippy --all-targets -- -D warnings`、`cargo test` 全量通过。
