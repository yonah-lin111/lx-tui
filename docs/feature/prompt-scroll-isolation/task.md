# Prompt 容器滚动隔离 — 执行任务

- 关联设计：`docs/feature/prompt-scroll-isolation/design.md`
- Review 报告：`docs/feature/prompt-scroll-isolation/review.md`
- 状态：**已合并到 `dev`（2026-11-16，merge `5babd147`）；自动化验证全绿，TUI 交互/外观由用户实机确认。**
- 规则：生产代码禁止 `unwrap()` / `expect()`；本任务无新增用户可见文案；不改 DB/schema。
- 前置：**用户审核 design.md 与本文档并确认后才能开始改代码。**

## 0. 需求盘问与规格确认（已完成）

- [x] 盘问 1：主复现 = 编辑正文视口；树 / Pages / Recent 一并纳入
- [x] 盘问 2：真实尺寸变化采用混合 C（变化前光标可见才继续保持可见）
- [x] 盘问 3：全局口径——自动路径不得改列表滚动（含进入树视图的选中项 reveal），只钳制
- [x] 盘问 4：自动恢复编辑视图也不回拉光标（工作区切换、树/页 → 编辑返回）
- [x] 盘问 5：进入树 / Pages 视图保留上次滚动位置，只钳制；Pages 不再重置为 0
- [x] 盘问 6：Recent 自动同步只更新 active + 钳制，不 `keep_visible`
- [x] 盘问 7：只修右侧 prompt 面板；左侧栏行为不变
- [x] 盘问 8：停泊草稿同一页后台刷新保留 scroll；页身份切换仍走 `set_text`
- [x] **用户审核通过（2026-11-16，指示按文档执行）**

## 1. 回归测试（先红后绿）

### 1.1 状态层 `src/app/prompt/tests.rs`

- [x] `resize_same_size_keeps_manual_scroll`
  - 构造 `prompt(40, 10)`，插入 50 行长文本（光标在文末），`scroll_to(0)`
  - 调用 `resize(40, 10)` 同尺寸
  - 断言 `scroll() == 0` 且 `cursor_cell().is_none()`
- [x] `resize_shrink_keeps_visible_cursor_visible`
  - 光标在视口内，缩小 `resize` 后断言 `cursor_cell().is_some()`
- [x] `resize_shrink_keeps_user_scrolled_away_cursor`
  - 光标文末 + `scroll_to(0)`，缩小尺寸后断言 `scroll()` 未被拉回（只钳制）
- [x] `restore_history_keeps_scrolled_view`
  - 长文本、光标文末、`scroll_to(0)`，`history()` → `restore_history()`
  - 断言 `scroll() == 0`
- [x] `replace_text_preserving_view_keeps_scroll_and_clamps_cursor`
  - `scroll_to(n)` 后用更短正文替换，断言 scroll 只钳制、不回到文末，光标在字符边界

### 1.2 更新层

#### `src/app/update/prompt/tests.rs`

- [x] `workspace_switch_preserves_editor_scroll`
  - 工作区 0：长正文滚到顶部（光标文末）
  - `switch_workspace` 到工作区 1，再模拟事件循环的 `resize_panes(&[(state.prompt.id(), Rect::new(..))])`（prompt id 变化后的真实路径）
  - 切回工作区 0，断言 `scroll()` 保持
- [x] `workspace_switch_preserves_target_tree_scroll`
  - 目标工作区 `prompt_panel.mode = Editor`、`editing = Item(id)`、`scroll = 0`、`selected` 在视口外
  - `swap_display_prompt(0, 1)` 后断言 `panel.scroll == 0`
- [x] `tree_roundtrip_keeps_scroll_when_cursor_out_of_view`
  - 在现有 `edit_position_and_scroll_preserved_when_switching_to_tree_and_back` 旁补“光标在视口外”的用例
- [x] `show_prompt_tree_preserves_scroll`
  - 树 `scroll = 0`、`selected` 在下方；`show_prompt_tree` 后断言 `scroll == 0`
- [x] `background_parked_refresh_preserves_scroll`
  - 停泊草稿 `scroll = 5`；`apply_prompt_tree` 用同一页新正文刷新后仍为 5（或被钳制到新 max）
- [x] `parked_page_switch_uses_new_document_view`（review P0-1）
  - 停泊草稿所在页被同键面板切换后，正文刷新按 `set_text` 语义把光标放文末并滚到光标，不保留旧页视口
- [x] `parked_save_receipt_preserves_scroll`（review P2-2）
  - 停泊草稿经 `mark_prompt_page_persisted` 同页刷新后仍保留 `scroll`
- [x] `resize_panes_clamps_prompt_tree_scroll`（review P1-1）
  - 真实尺寸变化后树偏移钳制到新上限
- [x] `clamp_prompt_list_scroll_clamps_all_views`（review P1-1）
  - 树 / 页 / Recent 三个容器同时按新可见行数钳制

#### `src/app/update/prompt/pages_view/tests.rs`

- [x] `workspace_switch_preserves_target_pages_scroll`
  - 目标工作区 Pages 模式、`pages_view.scroll = 0`、`selected` 在下方
  - `swap_display_prompt(0, 1)` 后断言 `pages_view.scroll == 0`
- [x] `show_prompt_pages_preserves_scroll`
  - `pages_view.scroll = 3`；`show_prompt_pages` 后仍为 3（若越界则钳制），不再重置为 0

#### `src/app/update/prompt/recent/tests.rs`

- [x] `recent_auto_sync_does_not_scroll`
  - `recent_open = true`、`recent_scroll = 0`、目标行在下方
  - `sync_prompt_recent_selection` 后断言 `recent_scroll == 0`、`recent_active` 正确

### 1.3 事件层 `src/event/tests/`

- [x] `workspace_item_click_keeps_prompt_scroll`
  - 真实鼠标点击 Workspaces item（`handle_sidebar_press → switch_workspace → resize_panes`）
  - 断言 `state.prompt.scroll()` 不变、`prompt_scrollbar` thumb 位置不变

## 2. 状态层修改

### 2.1 `src/app/prompt.rs`

- [x] `resize` 改为混合规则：
  - 尺寸未变直接 `return`
  - 尺寸变化：记录 `cursor_cell().is_some()` → 更新 `width/height` → `scroll.min(max_scroll())` → 仅“变化前可见且变化后不可见”时 `scroll_cursor_into_view()`
- [x] `restore_history`：删除 `self.scroll_cursor_into_view();`
- [x] 新增 `replace_text_preserving_view(&mut self, text: &str)`：
  - 保存旧 `scroll/cursor`
  - 复用私有 `replace_text`（与 `set_text` 相同的净化/清浮层/清历史/已保存态部分）
  - 光标 `floor_char_boundary` + `clamp_cursor`
  - `scroll = old.min(max_scroll())`

### 2.2 树列表

- [x] `src/app/prompt/tree/panel.rs::clamp_selection` 删除“选中项滚入视口”两个分支，只保留选中吸附与 `scroll.min(max)`
- [x] `src/app/update/prompt/tree_view.rs`：
  - [x] `select_prompt_item` / `select_prompt_temp` / `select_prompt_folder` 只设置 `selected`，删除 `scroll_selection_into_view` 与不再需要的 `visible`
  - [x] `clamp_prompt_tree_scroll`：实现后确认仍无生产调用方，已删除并同步移除测试断言
- [x] 保留 `scroll_selection_into_view` 仅给：`move_prompt_selection`、`apply_prompt_search` 非空过滤、文件夹折叠/展开

### 2.3 Pages 列表

- [x] `src/app/update/prompt/pages_view.rs`：
  - [x] `clamp_page_view` 改为只钳制选中/滚动
  - [x] 新增 `reveal_page_selection`（原 keep_visible 逻辑）
  - [x] `move_page_selection` / `apply_page_search` / `clear_page_search` 改用 `reveal_page_selection`
  - [x] `show_prompt_pages` 删除 `pages_view.scroll = 0`，进入后只 `clamp_page_view`
- [x] `src/app/update/prompt/pages.rs`：
  - [x] 用户显式新建/打开页（约 `226`、`270`、`500`）继续 `reveal_page_selection`
  - [x] 删除页（约 `650`）改用 `clamp_page_view`
- [x] `src/app/update/editor.rs::restore_prompt_view`（约 `438`）继续使用 `clamp_page_view`

### 2.4 Recent 抽屉

- [x] `src/app/update/prompt/recent.rs::sync_prompt_recent_selection`：
  - 删除 `keep_visible`
  - `recent_scroll = recent_scroll.min(rows.len().saturating_sub(recent_visible.max(1)))`
  - 保留 `recent_active` 更新与空行处理
- [x] `keep_visible` 仅保留给 `move_prompt_recent`、`scroll_prompt_recent`、`set_prompt_recent_scroll`、`toggle_prompt_recent`

### 2.5 停泊草稿后台刷新

- [x] `src/app/update/prompt/mod.rs`：停泊草稿按页身份分流——`page_switched` 保持 `set_text(&active_content)`（换文档，I5），同一页正文更新才用 `replace_text_preserving_view(&active_content)`（review P0-1）
- [x] `src/app/update/prompt/save.rs:155-163`：保存回执只服务同页刷新，使用 `replace_text_preserving_view`
- [x] `reload_editor_from_active_page`（`pages.rs:681-708`）保持 `set_text`（换文档语义）

### 2.6 几何变化统一钳制（review P1-1）

- [x] 新增 `src/app/update/prompt/mod.rs::clamp_prompt_list_scroll(state)`：
  - 树：只钳制版 `PromptPanel::clamp_selection`
  - 页：仅在存在编辑绑定时 `clamp_page_view`；树视图下页列表休眠，保留上次滚动
  - Recent：`recent_page_count.min(MAX_RECENT_PAGES)` 与 `recent_visible` 钳制 `recent_scroll`
- [x] `src/app/update/pane.rs::resize_panes` 在更新 prompt 尺寸与 `recent_visible` 后调用；不 reveal 选中项

## 3. 既有测试调整

- [x] `src/app/prompt/tests.rs::resize_keeps_cursor_visible_and_guards_zero_size` 按 I4 拆成“尺寸变化且光标原本可见”与“尺寸未变”
- [x] `src/app/update/prompt/tests.rs::scroll_clamps_and_keeps_selection_visible` 移除已删除的 `clamp_prompt_tree_scroll` 断言
- [x] `src/app/update/prompt/pages_view/tests.rs` 中依赖 `show_prompt_pages` 重置 scroll 的断言按决策 5 更新
- [x] `src/app/update/prompt/recent/tests.rs` 中依赖自动同步 `keep_visible` 的断言改为只钳制
- [x] 其余失败测试必须先判断是“旧 bug 当规格”还是真实回归；前者改写并注明原因

## 4. 文档同步

- [x] `docs/standards/tui-design-requirements.md`：补一条 prompt 滚动所有权/自动路径只钳制的不变量
- [x] 本目录 `design.md` / `task.md`：实现后标注完成状态、记录与设计的差异
- [x] 如实现中发现新的例外，先回到 §6 边界表登记，禁止静默扩大行为

## 5. 验证（按影响范围）

- [x] `cargo fmt`
- [x] 精准测试（先跑新增用例）：
  - `cargo test --lib resize_same_size_keeps_manual_scroll`
  - `cargo test --lib resize_shrink_keeps`
  - `cargo test --lib restore_history_keeps_scrolled_view`
  - `cargo test --lib replace_text_preserving_view`
  - `cargo test --lib workspace_switch_preserves`
  - `cargo test --lib show_prompt_tree_preserves_scroll`
  - `cargo test --lib show_prompt_pages_preserves_scroll`
  - `cargo test --lib background_parked_refresh_preserves_scroll`
  - `cargo test --lib parked_page_switch_uses_new_document_view`
  - `cargo test --lib parked_save_receipt_preserves_scroll`
  - `cargo test --lib resize_panes_clamps_prompt_tree_scroll`
  - `cargo test --lib clamp_prompt_list_scroll_clamps_all_views`
  - `cargo test --lib clamp_prompt_list_scroll_keeps_dormant_pages_scroll`
  - `cargo test --lib recent_auto_sync_does_not_scroll`
  - `cargo test --lib workspace_item_click_keeps_prompt_scroll`
- [x] 受影响模块回归：
  - `cargo test --lib app::prompt`
  - `cargo test --lib app::update::prompt`
  - `cargo test --lib event::tests::prompt_editor`
- [x] `cargo clippy --all-targets -- -D warnings`
- [x] `cargo test`
- [x] 用户运行 `cargo run` 按 design §9 手工验收（2026-11-16 用户指示合并；后续交互/外观问题单独修复）

## 6. 提交建议

- 提交 1：状态层（`prompt.rs` / tree / pages / recent）＋对应状态层测试
- 提交 2：调用点（`editor.rs` / `mod.rs` / `save.rs` / `pages.rs`）＋更新层/事件层测试＋文档
- 提交 3（review 后）：review.md、P0/P1 修复与补齐测试、design/task 同步

## 7. 完成定义

- 设计 §9 手工验收全部通过；
- 新增回归测试覆盖 design §7 全部条目；
- `cargo fmt` / `cargo clippy --all-targets -- -D warnings` / `cargo test` 全绿；
- 本文件所有 checkbox 勾选，design.md 状态改为“已合并到 dev”。

## 8. 实现记录与差异

- `clamp_prompt_tree_scroll` 删除（无生产调用方），原测试断言同步移除；几何变化改由 `clamp_prompt_list_scroll` 统一只钳制。
- 文件夹折叠/展开在两处 toggle 后显式调用 `scroll_selection_into_view`，保持原有“用户导航跟随选中项”行为。
- `recent/tests.rs::sync_uses_cached_visible_entries` 原断言按旧 `keep_visible` 语义写成，已按决策 6 改为只钳制。
- `event/tests/todo.rs::mouse_right_click_prompt_tree_on_todo_opens_todo_menu` 原坐标依赖“进入树视图 reveal 选中项”的旧滚动偏移，改为扫描真实可开菜单的树行。
- Review 修复：停泊草稿页身份切换改回 `set_text`（P0-1）；`resize_panes` 接入 `clamp_prompt_list_scroll`（P1-1）；补齐目标树滚动、保存回执、页切换、几何钳制测试（P2-1/P2-2）。详见 `review.md`。
- 合并：2026-11-16 以 `--no-ff` 合并 `fix/prompt-scroll-isolation` → `dev`（merge `5babd147`），无冲突；合并后主工作区复跑 `cargo fmt --check`、`cargo clippy --all-targets -- -D warnings` 与全量 `cargo test` 通过。
