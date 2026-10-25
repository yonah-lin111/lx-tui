# 右侧统一容器与 Prompt / Todo 标签页 — 执行任务

- 任务目录：`docs/feature/right-panel-tabs/`
- 规则：严格遵守 `AGENTS.md`；生产代码禁止 `unwrap()` / `expect()`（测试除外）；用户可见文案进
  `ui/text.rs`；颜色只从 `ui/style.rs` 取；`app/` 与 `ui/` 边界不得打破；测试驱动（先红后绿）。
- 需求原文：把 prompt 容器顶部的 title 和分页挪到功能按钮下一行的边框里面；把 prompt 容器改成统一的
  右侧容器（后续会加很多面板），prompt 抽成其中一个独立面板（含编辑页、顶部操作栏、列表管理页）；
  容器顶部边框加 `Prompt` / `Todo` 两个 tab，默认 `Prompt`，Todo 只占位；点 Todo 后打开空白容器
  （顶部功能按钮栏为空，不与 prompt 冲突）；底部内容不随 tab 变化。
- 前置：用户已审核并批准同目录 `design.md` 与本清单（13 条 Grill Me 边界已逐条确认）。

## 执行清单

### 1. 需求与边界对齐（Grill Me 阶段，已完成）

- [x] 质问 1：容器只保留一层边框，prompt 成为容器内子面板 → 采纳推荐。
- [x] 质问 2：`right_tab` 全局一份、不按工作区隔离、不持久化 → 采纳推荐。
- [x] 质问 3：切换 tab 的瞬时状态收起、内容保留 → 采纳推荐。
- [x] 质问 4：切走时页重命名**提交**（非空写回），瞬时指针（焦点/浮层/选区/悬停）清空 → 采纳推荐。
- [x] 质问 5：删除树视图固定标题 `Prompts`，树视图标题行为纯 `─` → 采纳推荐。
- [x] 质问 6：tab 刻在顶边框线上，不加 `▪` 图标与 `|` 分隔符，高亮复用 `selected_item` /
      `selection` / `muted` → 采纳推荐（用户补充：tab 高亮参考容器顶部 tab 样式）。
- [x] 质问 7：新增 `src/ui/right_panel.rs` 承载容器，`prompt_frame.rs` 收敛为面板体 → 采纳推荐。
- [x] 质问 8：新几何放渲染层（`ui/right_panel.rs`、`ui/prompt/header.rs`），`layout.rs` 一行不加
      （该文件已 999 行）→ 采纳推荐。
- [x] 质问 9：Todo 下键盘退回聚焦窗格，点击 Todo 一律吞掉、不穿透 → 采纳推荐。
- [x] 质问 10：`right_tab` 不持久化，启动恒为 `Prompt` → 用户确认。
- [x] 质问 11：保留 `prompt_left` 交换，整个容器（含 tab）换边 → 用户确认。
- [x] 质问 12：Todo 内容区纯空白、零占位文案 → 用户确认。
- [x] 质问 13：title + 分页内嵌在**按钮行下方的分割线**上（方案 A）→ 用户选择 A。
- [x] 用户审核 `design.md` 与本清单，确认后开始实施。（**待用户确认**）

### 2. 复现测试（先红）

- [x] `src/ui/right_panel/tests.rs`（新）：`tabs_render_on_top_border_with_accent_active`、
      `tabs_drop_trailing_when_narrow`、`tab_hit_areas`、`todo_body_is_blank_with_plain_divider`、
      `bottom_identity_survives_tab_switch`。
- [x] `src/app/update/tests.rs`：`select_right_tab_commits_rename_and_clears_transient_state`、
      `select_right_tab_preserves_prompt_content`、`select_right_tab_same_tab_is_noop`、
      `focus_prompt_switches_back_to_prompt_tab`。
- [x] `src/event/tests.rs`：`mouse_click_right_tab_switches_panel`、
      `mouse_click_todo_body_swallowed_without_focus_or_cursor`、
      `mouse_click_prompt_toolbar_ignored_on_todo_tab`、`mouse_wheel_over_todo_does_not_scroll_prompt`、
      `mouse_motion_tracks_right_tab_hover`。
- [x] 确认上述用例在改动前确实失败（红），并记录失败原因。

### 3. 状态层（`src/app/`）

- [x] `src/app/state.rs`：
  - [x] 定义 `RightTab { Prompt, Todo }`（`Default = Prompt`），带文档注释说明"纯视图偏好"。
  - [x] `AppState` 新增 `pub right_tab: RightTab` 与 `pub right_tab_hover: Option<RightTab>`。
  - [x] `AppState::demo()` 初始化为 `RightTab::Prompt` / `None`。
- [x] `src/app/update/right_panel.rs`（新，约 60 行）：
  - [x] `select_right_tab(state, tab) -> bool`：先 `finish_page_rename`，再置 tab；
        切到非 Prompt 时清 `prompt_focused` / `clear_selection` / `prompt.clear_panel()` / `prompt_tree_hover`。
  - [x] `set_right_tab_hover(state, hover) -> bool`（变化才返回 true）。
  - [x] `prompt_panel_active(state) -> bool`（`right_tab == Prompt && !prompt_collapsed`）。
- [x] `src/app/update.rs`：`mod right_panel;` + `pub use right_panel::*;`。
- [x] `src/app/update/editor.rs::focus_prompt`：聚焦时一并把 `right_tab` 置回 `Prompt`
      （保证 `prompt_focused ⇒ right_tab == Prompt` 不变量）。
- [x] `src/app/actions.rs`：**不加**新 `Action` 变体——tab 切换带参数（`RightTab`），
      而 `Action` 是 `Copy` 无参枚举；由事件层直接调 `update::select_right_tab`（与
      `toggle_prompt_panel_mode` 等既有做法一致）。

### 4. 布局与渲染（`src/ui/`）

- [x] `src/ui/right_panel.rs`（新，约 220 行）：
  - [x] `tab_label` / `tab_rects` / `tab_at` / `tab_row_at` / `paint_tabs`（纯函数，绘制与命中共用几何）。
  - [x] `render(frame, view, state)`：折叠窄条 → `Block` → tab → 底边框 → 按 `right_tab` 分发。
  - [x] `render_empty_body`（Todo）：只画工具栏下一行的分割线。
  - [x] `render_bottom` / `prompt_identity`：从 `prompt_frame.rs` 原样搬入。
  - [x] `#[cfg(test)] mod tests;` + 同目录 `tests.rs`。
- [x] `src/ui/prompt_frame.rs`：
  - [x] `render_prompt` → `render_body(frame, area, state)`：去掉 `Block`、去掉顶边框标题、去掉底边框；
        改为"先 `render_header` 画分割线 → 再 `paint_title_row` 覆盖标题与页控件"。
  - [x] 树视图 `TitleRow` 变体改为"无标题"（不再引用 `PROMPT_TREE_BORDER_TITLE`）。
  - [x] `prompt_scrollbar` 增加 `right_tab == Prompt` 闸门。
  - [x] `prompt_page_control_at` 改用 `prompt::title_row(area)` 作为控件区矩形。
  - [x] 新增 `pub fn title_row(panel) -> Option<Rect>`（整行版本，复用 `prompt_divider_rect` 的行号）。
  - [x] 新增 `pub fn prompt_title_at(panel, column, row) -> bool`（标题行整行命中，供"点标题行聚焦"用）。
- [x] `src/ui/prompt/header.rs`：按 `design.md` §4.3.3 重命名 6 个符号；`page_controls` 算术不动。
- [x] `src/ui/prompt.rs`：`render_divider` 改 `pub(crate)`；`render_toolbar` 不动。
- [x] `src/ui/text.rs`：新增 `RIGHT_TAB_PROMPT_LABEL` / `RIGHT_TAB_TODO_LABEL`，删除 `PROMPT_TREE_BORDER_TITLE`。
- [x] `src/ui/mod.rs`：`pub mod right_panel;`，`render_prompt` 调用改为 `right_panel::render`。
- [x] `src/ui/mod.rs` 顶部浮层 tooltip 判断：`Overlay::PageMenu(...)` 等分支不变（浮层未改）。

### 5. 事件与交互（`src/event/`）

- [x] `src/event/mouse/press.rs`：按 `design.md` §4.4.1 插入 tab 命中、tab 行空白吞掉、
      `!prompt_panel_active` 整块吞掉三段分支；其余 prompt 分支保持原顺序。
- [x] `src/event/mouse/hover.rs`：新增 tab 悬停计算与 `set_right_tab_hover`；树视图行悬停闸门改为
      `!update::prompt_panel_active(state)`。
- [x] `src/event/mouse/hit.rs`：
  - [x] `pane_content_inner`：prompt 且 `!prompt_panel_active` 时返回零矩形。
  - [x] 删除 `prompt_border_at`（改用 `ui::right_panel::tab_row_at`）。
- [x] `src/event/mouse/wheel.rs`：prompt 分支加 `prompt_panel_active` 闸门。
- [x] `src/event/mouse/drag.rs`：prompt 光标跟随分支加同一闸门。
- [x] `src/event/overlay.rs`：提示词树右键菜单分支加同一闸门。
- [x] `src/event/session.rs`：**不改**（rects 仍需 prompt 列，保证分割线拖拽与 toast 锚点）。

### 6. 测试与文档

- [x] 迁移/更新既有用例：`src/ui/prompt/tests.rs`（标题行落点、重命名光标、树视图标题）、
      `src/ui/tests.rs::prompt_renders_toolbar_and_divider_rows`、
      `src/event/tests.rs` 中页控件两例的命中行号。
- [x] 补齐第 2 节列出的新用例，全部转绿。
- [x] 同步 `docs/standards/tui-design-requirements.md` 第 25 行（按 `design.md` §7 五点改写）。
- [x] 回写 `docs/feature/right-panel-tabs/design.md` 的"实施记录（与设计的差异）"（若实现期有偏差）。

### 7. 精确验证

- [x] `cargo fmt`
- [x] `cargo clippy --all-targets -- -D warnings`
- [x] `cargo test ui::right_panel`、`cargo test ui::prompt`、`cargo test layout::`、`cargo test app::update`、
      `cargo test event::`
- [x] `cargo test`（全量）
- [x] 交回用户 `cargo run` 人工验收视觉与交互（Agent 无法交互式验证 TUI）。

## 评审与修复

`review.md` 记录了 3 个并行 reviewer 的全部发现与逐条修复，含变异验证结果。
本轮修复的核心是 3 个真缺陷：

1. `paint_tabs` 的样式继承（非激活 tab 继承聚焦边框的青色加粗，两个 tab 一起发亮）；
2. 点"当前 tab"不提交页重命名（`select_right_tab` 早退）且无条件置脏；
3. 异步 worktree 回调在 Todo 期间复活隐藏的提及面板（`clear()` 不清 cache）。

另修正 2 条**空转测试**（"滚动保留"实为 `0 == 0`；5 条 `is_none()` 断言的是
从未打开过的面板）与 1 处**文档谎言**（底边框 `ws:路径末段名`，代码里不存在）。

## 完成记录

- 分支：`feat/right-panel-tabs`（worktree：`.worktrees/right-panel-tabs`，基于 `dev` @ `98388df`）。
- 验证：`cargo fmt` 通过；`cargo clippy --all-targets -- -D warnings` 通过；`cargo test` 全绿
  （lib 3377 passed / 0 failed / 2 ignored，其余 15 个测试目标全 ok）。
- 与设计的差异（详见 `design.md` §9）：
  1. `tab_rects` 的右边界从 `>` 收紧为 `>=`：否则 tab 恰好顶到 `row.right()` 时会盖掉右边角 `╮`。
  2. `press.rs` 的「`!prompt_panel_active` 整块吞掉」额外加了 `view.prompt.contains(...)` 前置条件，
     否则折叠态（`prompt_panel_active` 同为假）会把整个屏幕的点击吞掉。
  3. 修复 `paint_tabs` 的样式继承缺陷：`muted()` / `selection()` 不带完整前景，直接叠加会继承
     聚焦边框的 `accent()`（Cyan 加粗），导致非激活 tab 也在发亮。改为每格先 `Style::reset()`
     再套 token，并有聚焦态回归用例锁住。
  4. `src/ui/tooltip.rs` 的 prompt 工具栏/块按钮 tooltip 一并加 `right_tab == Prompt` 闸门，
     否则 Todo 下悬停空白处会弹出按钮提示（渲染层已不画按钮）。
  5. tab 点击分支改为「先 `finish_page_rename` 再 `select_right_tab`，两者都无变化才不置脏」：
     `select_right_tab` 对同一 tab 早退，点当前 tab 原本既不提交重命名也不退出输入态，
     且无条件置脏会白重绘一次。已补两条回归用例
     （`mouse_click_current_tab_commits_page_rename`、
     `mouse_click_current_tab_without_changes_is_not_dirty`）。
- 未做项：无。`src/layout.rs` 保持 999 行未动；`docs/standards/tui-design-requirements.md` 已按
  `design.md` §7 的五点同步。
- 待人工验收：tab 高亮观感、Todo 空白容器、点击不穿透（Agent 无法交互式验证 TUI，需 `cargo run`）。
