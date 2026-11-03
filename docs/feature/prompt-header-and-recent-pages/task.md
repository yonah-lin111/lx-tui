# Prompt 顶部控件换位、页面标题展示与底部 Recent Pages 抽屉实施任务

- 状态：**已实现并合入 `dev`**（2026-11-03）
- 关联设计：`docs/feature/prompt-header-and-recent-pages/design.md`

---

## 1. 需求与边界确认（Grill Me 已完成）

- [x] **问题 1（Recent 面板位置与打开方式）**：顶部工具栏新增 `[rec]` 按钮（`Tree` / `Editor` / `Pages` 三种模式常显），绑定快捷键 `Ctrl/Cmd + r`；以底部抽屉形式在 Prompt 面板下方展开，顶部带 `├─ recent ───┤` 分割线，不完全覆盖上方内容。
- [x] **问题 2（抽屉高度与点击行为）**：抽屉最大显示 3 个双行条目（1 行分割线 + 6 行内容 = 7 行），记录不足 2 条时仍保留 2 条高度（5 行），且最多不超过面板内容区高度的一半；鼠标左键单击条目直接跳转到对应 `Prompt + Page` 进入编辑视图，跳转后抽屉保持展开。
- [x] **问题 3（数据范围、排序、时间格式与状态点）**：汇总当前工作区所有已入库 `PromptItem` 的全部 `PromptPage`（不含 `tmp` 草稿），按 `updated_at` 全局倒序排列（上限 20 条）；时间格式采用 `[now]` / `[5m]` / `[2h]` / `[3d]` / `[MM-DD]`；Page 标题与状态小圆点（`○` / `●`）复用 `page_title_status` 规则。
- [x] **问题 4（顶部两行表头重排与当前页面 Title）**：
  - 第 1 行（工具栏）：左组为 `[≡]/[/] [rec] [←] [→]`（`[rec]` 未展开时为默认亮白 `style::text`，展开后为亮青 `style::accent`），右组为 `[refDir] ‹ n/m › ●`（`‹ n/m ›` 移至 `[refDir]` 与保存圆点 `●` 中间，`n/m` 在 `Editor` 下为亮白、`Pages` 下为亮青，且 `‹`、`n/m`、`›` 均带 tooltip）。
  - 第 2 行（分割线标题行）：`Editor` 模式左侧显示 ` prompt名称 · 当前页面title `（优先取 `{title: xxx}` 否则回退 `page.name`）；移除右侧 `[todo]` / `[run]` / `[done]` 状态按钮，改为按模板块状态为 `当前页面title` 着色（无闭合块暗灰，`Todo` 亮白、`InProgress` 黄、`Done` 绿），hover 提示 `cycle block status`、左键点击循环切换状态。
- [x] **问题 5（键盘接管与双行悬停）**：展开 `recent` 抽屉后键盘操作全部归抽屉，默认选中当前编辑页（否则第一条），`↑`/`↓` 移动、`Enter` 打开、`Esc` 或右上角 `[x]` 收起，其余键与粘贴全部吞掉；抽屉内当前正在编辑的 Page 前置 `❯ ` 并加粗；鼠标悬停两行同时高亮。

---

## 2. 实施步骤拆解

### Task 1：数据库与平台层扩展 `updated_at` 时间戳及相对时间格式化
- [x] **1.1 平台层时间辅助函数（`src/platform/environment.rs` & `src/platform/mod.rs`）**
  - 实现 `pub fn current_unix_secs() -> i64`。
  - 实现 `pub fn format_relative_time(updated_secs: i64, now_secs: i64) -> String`（支持 `[now]`、`[Nm]`、`[Nh]`、`[Nd]`、`[MM-DD]`）。
  - 在 `src/platform/environment/tests.rs` 补充各时间区间的单元测试。
- [x] **1.2 数据库层读取 `updated_at`（`src/db/mod.rs`）**
  - 在 `PromptPageRecord` 中新增 `pub updated_at: i64`。
  - 修改 `Database::list_prompts` 的页查询 SQL，增加 `COALESCE(CAST(strftime('%s', pg.updated_at) AS INTEGER), 0)` 并填入 `PromptPageRecord.updated_at`。
  - 在 `src/db/tests.rs` 验证新建页与更新页正文后 `list_prompts` 返回非零且递增的 `updated_at`。
- [x] **1.3 内存模型同步 `updated_at`（`src/app/prompt/tree.rs` & `src/app/update/prompt/`）**
  - 在 `PromptPage` 中新增 `pub updated_at: i64`，提供 `PromptPage::saved_at(id, name, content, updated_at)`，并在 `From<PromptPageRecord>` 中透传 `record.updated_at`。
  - 在 `commit_editing_prompt`（内容发生变更时）、`cycle_page_template_status`、`open_created_page` 中将目标页的 `updated_at` 刷新为 `platform::current_unix_secs()`。

### Task 2：顶部两行表头重排（`[rec]`、页控件移入工具栏、标题行显示 `· PageTitle` 并按模板状态着色）
- [x] **2.1 文案与枚举定义（`src/ui/text.rs`、`src/layout/prompt.rs`、`src/app/tooltip.rs`）**
  - `src/ui/text.rs`：新增 `PROMPT_RECENT_LABEL = "[rec]"`、`PROMPT_RECENT_SECTION_LABEL = " recent "`、`PROMPT_RECENT_CLOSE = "[x]"`、`PROMPT_RECENT_EMPTY = "no recent pages"` 及对应 Tooltip 文案。
  - `src/layout/prompt.rs`：`PromptToolbarButton` 新增 `Recent`。
  - `src/app/tooltip.rs` & `src/ui/tooltip.rs`：`TooltipTarget` 新增 `PromptRecent`、`PromptRecentClose`、`PromptPagePrev`、`PromptPageCounter`、`PromptPageNext` 并接入悬停提示。
- [x] **2.2 工具栏与标题行几何重算（`src/layout/prompt.rs` & `src/ui/prompt/header.rs`）**
  - 在 `src/layout/prompt.rs` 实现 `toolbar_page_controls`（`Editor` 模式位于 `Save` 圆点 `●` 左侧排布 `‹ n/m ›` 且 `n/m` 为默认亮白；`Pages` 模式位于工具栏右侧排布 `n/m` 且为亮青高亮）。
  - 在 `src/layout/prompt.rs` 更新 `prompt_toolbar_button_rect`：
    - 左组：`Mode` (`bar.x`) → `Recent` (`bar.x + 4`) → `Undo` → `Redo`（动态避让右侧页控件左边界，按 `[refDir]` → `[rec]` → `[←] [→]` 顺序隐藏）；
    - 右组：`Folders` (`[refDir]`) 移至工具栏页控件 `‹ n/m ›` 左侧 1 列（`Save` 保持在 `bar.right() - 1`）。
  - 在 `src/ui/prompt/header.rs` 移除标题行右侧 `[todo]` / `[run]` / `[done]` 状态按钮，由 `paint_title_row` 按本页闭合 `&&&` 模板块状态直接为 `· ` 之后的页名着色（无闭合块暗灰，`Todo` 亮白、`InProgress` 黄、`Done` 绿），并由 `status_rect` 返回页名矩形供点击循环状态与 Tooltip 命中。
- [x] **2.3 标题行当前页面 Title 渲染与事件命中（`src/ui/prompt_frame.rs` & `src/event/mouse/press.rs`）**
  - 修改 `src/ui/prompt_frame.rs` 的 `prompt_page_header`：在 `PromptMode::Editor` 下通过 `page_display_title` 解析当前页显示标题（首个闭合 `&&&` 的 `{title:}` 或回退 `page.name`），使分割线标题行左侧渲染 ` prompt名称 · 当前页面title `。
  - 更新 `prompt_page_control_at`：命中行由 `title_row(panel)` 改为 `prompt_toolbar_rect(panel)`；`prompt_status_rect` 命中 `title_row(panel)` 上的页名区域。
  - 在 `docs/standards/tui-design-requirements.md` 同步更新表头两行布局规范说明。

### Task 3：实现 Recent Pages 派生模型与底部抽屉状态管理
- [x] **3.1 Recent Pages 派生与排序逻辑（新建 `src/app/prompt/recent.rs`）**
  - 实现 `RecentPageRow` 与 `recent_page_rows(panel, live_editor, now_secs) -> Vec<RecentPageRow>`：
    - 汇总 `panel.tree.items()` 下所有 `PromptPage`；
    - 按 `updated_at` 降序（次序按 `page.id` 降序）排序，最多取 20 条；
    - 计算每条目的 `page_title`、`status`、`time_label` 与 `current` 标记。
- [x] **3.2 抽屉状态、打开/收起、键盘选中与点击跳转（`src/app/prompt/tree/panel.rs`、`src/app/state.rs`、`src/app/update/prompt/recent.rs`）**
  - `PromptPanel` 新增 `recent_open: bool`、`recent_scroll: usize` 与 `recent_active: Option<usize>`；`AppState` 新增 `prompt_recent_hover: Option<usize>`。
  - 实现 `update::toggle_prompt_recent` / `close_prompt_recent`：切换 `recent_open`，展开时默认选中当前编辑页并收起浮层候选面板，收起时清空 `recent_active`，重算并同步编辑器视口尺寸。
  - 实现 `update::open_recent_page(state: &mut AppState, index: usize) -> bool`：
    - 提交当前未保存编辑缓冲（`commit_editing_prompt`）；
    - 定位目标 `(item_id, page_id)`，更新 `item.active_page = page_id` 并入队 `Effect::SetActivePage`；
    - 绑定 `panel.editing = Some(PromptEditing::Item(item_id))`，切换 `panel.mode = PromptMode::Editor`（保持 `recent_open = true`），加载目标页正文与历史，并让 `recent_active` 跟随重排后的新位置。

### Task 4：底部 Recent 抽屉布局分割、渲染与键盘/鼠标事件接入
- [x] **4.1 主视图与底部抽屉几何分割（`src/layout/prompt.rs` & `src/ui/prompt_frame.rs`）**
  - 在 `src/layout/prompt.rs` 新增 `PromptBodySplit` 与 `prompt_body_split(panel: Rect, recent_open: bool, recent_entries: usize) -> PromptBodySplit`：
    - 计算上方主内容区 `main` / `main_scrollbar`（通过 `ui::prompt_frame::prompt_main_rect` 统一供渲染与事件命中）；
    - 计算下方抽屉 `recent_divider`、`recent_text` 与 `recent_scrollbar`（最多 3 条/7 行、保底 2 条/5 行，且不超过 `body.height / 2`；高度不足时自动压制抽屉）。
- [x] **4.2 底部抽屉渲染（新建 `src/ui/prompt/recent.rs` & 更新 `src/ui/prompt_frame.rs`）**
  - 在 `src/ui/prompt/recent.rs` 实现：
    - `render`：绘制 `├─ recent ─── [x] ─┤` 分割线与关闭按钮、空态 `no recent pages` 或双行条目列表；
    - 第 1 行左画 `❯ `（当前页）+ `prompt_name`，右画 `[time]`（窄屏优先保留名称）；
    - 第 2 行左画 2 列缩进 + `page_title`（muted），右画状态圆点 `○` / `●`；
    - 鼠标悬停或键盘选中（`hover.or(active)`）时双行同时铺选中底色。
  - 在 `src/ui/prompt_frame.rs` 的 `render_body` 中：使用分割后的上方 `split.main` 渲染 `Tree`/`Pages`/`Editor` 及主滚动条；当 `split.recent_divider.is_some()` 时将 `[rec]` 染为亮青、渲染底部 `recent` 抽屉并隐藏上方硬件光标。
- [x] **4.3 快捷键 `Ctrl/Cmd + r`、键盘接管与鼠标滚轮/点击/拖拽/悬停接入（`src/input/mod.rs`、`src/event/input.rs`、`src/event/mouse/`）**
  - `src/input/mod.rs` & `src/app/actions.rs` & `src/app/update/editor.rs`：新增 `EditorCommand::ToggleRecentPages`，绑定 `Ctrl/Cmd + r`（在 `Tree`、`Pages`、`Editor` 下均生效）。
  - `src/event/input.rs`：当抽屉实际展开可见时接管全部 Prompt 键盘与粘贴事件（`↑`/`↓` 选择、`Enter` 打开、`Esc`/`Ctrl+R` 收起，其余键与粘贴吞掉）。
  - `src/event/mouse/press.rs`：接入工具栏 `[rec]` 点击、抽屉右上角 `[x]` 关闭、抽屉条目左键单击跳转与滚动条槽点击；抽屉命中优先于上方视图块按钮，右键在抽屉内直接吞掉。
  - `src/event/mouse/wheel.rs` & `motion.rs` & `drag.rs`：抽屉内滚轮滚动并钳制 `recent_active`，支持双行悬停与独立滚动条拖拽。

---

## 3. 测试与验证计划

- [x] **单元测试与集成测试**：
  1. `src/platform/tests.rs`：验证 `format_relative_time` 对 `<60s` (`[now]`)、分钟 (`[5m]`)、小时 (`[2h]`)、天 (`[3d]`) 及 `>=30d` (`[MM-DD]`) 的格式化准确性。
  2. `src/app/prompt/recent/tests.rs` & `src/app/update/prompt/recent/tests.rs`：验证多 Prompt 多 Page 按 `updated_at` 全局倒序排列、20 条上限截断、实时编辑缓冲标题与状态圆点提取、自动保存与删除条目后 `recent_active` 同步对齐。
  3. `src/ui/prompt/header/tests.rs`、`src/ui/prompt/recent/tests.rs`、`src/layout/tests.rs`、`tests/prompt_border_title.rs`：
     - 验证第 1 行工具栏 `[≡] [rec] [←] [→]` 与 `[refDir] ‹ 2/4 › ●` 的坐标几何、窄屏避让降级、`[rec]` 与 `n/m` 的两态颜色及 Tooltip；
     - 验证第 2 行分割线标题行左侧显示 ` prompt名称 · 当前页面title `，不显示 `[todo]`/`[run]`/`[done]` 按钮，且页名按模板状态着色并支持点击/Tooltip；
     - 验证 `recent` 抽屉展开时主视图与抽屉的上下分割几何、双行渲染、`[x]` 关闭按钮、悬停/键盘高亮与独立滚动条。
  4. `src/event/tests/prompt_recent.rs`：
     - 验证 `Ctrl/Cmd + r`、点击 `[rec]` 与点击 `[x]` 在 `Tree`、`Editor`、`Pages` 三种模式下开关底部抽屉；
     - 验证抽屉展开时 `↑`/`↓`/`Enter`/`Esc` 键盘导航与其余按键/粘贴吞掉，以及矮屏压制抽屉时不吞键；
     - 验证单击抽屉条目跳转、抽屉覆盖区不误触幽灵块按钮或穿透右键菜单、滚轮独立滚动。
- [x] **代码质量门禁**：
  - `rtk cargo fmt --check`
  - `rtk cargo clippy --all-targets -- -D warnings`
  - `rtk cargo test`
