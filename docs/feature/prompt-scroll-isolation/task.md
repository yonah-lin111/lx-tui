# Prompt 容器滚动隔离 — 执行任务

- 关联设计：`docs/feature/prompt-scroll-isolation/design.md`
- 规则：生产代码禁止 `unwrap()` / `expect()`；本任务无新增用户可见文案；不改 DB/schema。
- 前置：**用户审核 design.md 与本文档并确认后才能开始改代码。**

## 0. 需求盘问与规格确认（已完成，待审核）

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

- [ ] `resize_same_size_keeps_manual_scroll`
  - 构造 `prompt(40, 10)`，插入 50 行长文本（光标在文末），`scroll_to(0)`
  - 调用 `resize(40, 10)` 同尺寸
  - 断言 `scroll() == 0` 且 `cursor_cell().is_none()`
- [ ] `resize_shrink_keeps_visible_cursor_visible`
  - 光标在视口内，缩小 `resize` 后断言 `cursor_cell().is_some()`
- [ ] `resize_shrink_keeps_user_scrolled_away_cursor`
  - 光标文末 + `scroll_to(0)`，缩小尺寸后断言 `scroll()` 未被拉回（只钳制）
- [ ] `restore_history_keeps_scrolled_view`
  - 长文本、光标文末、`scroll_to(0)`，`history()` → `restore_history()`
  - 断言 `scroll() == 0`
- [ ] `replace_text_preserving_view_keeps_scroll_and_clamps_cursor`
  - `scroll_to(n)` 后用更短正文替换，断言 scroll 只钳制、不回到文末，光标在字符边界

### 1.2 更新层

#### `src/app/update/prompt/tests.rs`

- [ ] `workspace_switch_preserves_editor_scroll`
  - 工作区 0：长正文滚到顶部（光标文末）
  - `switch_workspace` 到工作区 1，再模拟事件循环的 `resize_panes(&[(state.prompt.id(), Rect::new(..))])`（prompt id 变化后的真实路径）
  - 切回工作区 0，断言 `scroll()` 保持
- [ ] `workspace_switch_preserves_target_tree_scroll`
  - 目标工作区 `prompt_panel.mode = Editor`、`editing = Item(id)`、`scroll = 0`、`selected` 在视口外
  - `swap_display_prompt(0, 1)` 后断言 `panel.scroll == 0`
- [ ] `tree_roundtrip_keeps_scroll_when_cursor_out_of_view`
  - 在现有 `edit_position_and_scroll_preserved_when_switching_to_tree_and_back` 旁补“光标在视口外”的用例
- [ ] `show_prompt_tree_preserves_scroll`
  - 树 `scroll = 0`、`selected` 在下方；`show_prompt_tree` 后断言 `scroll == 0`
- [ ] `background_parked_refresh_preserves_scroll`
  - 停泊草稿 `scroll = 5`；`apply_prompt_tree` 用同一页新正文刷新后仍为 5（或被钳制到新 max）

#### `src/app/update/prompt/pages_view/tests.rs`

- [ ] `workspace_switch_preserves_target_pages_scroll`
  - 目标工作区 Pages 模式、`pages_view.scroll = 0`、`selected` 在下方
  - `swap_display_prompt(0, 1)` 后断言 `pages_view.scroll == 0`
- [ ] `show_prompt_pages_preserves_scroll`
  - `pages_view.scroll = 3`；`show_prompt_pages` 后仍为 3（若越界则钳制），不再重置为 0

#### `src/app/update/prompt/recent/tests.rs`

- [ ] `recent_auto_sync_does_not_scroll`
  - `recent_open = true`、`recent_scroll = 0`、目标行在下方
  - `sync_prompt_recent_selection` 后断言 `recent_scroll == 0`、`recent_active` 正确

### 1.3 事件层 `src/event/tests/`

- [ ] `workspace_item_click_keeps_prompt_scroll`
  - 真实鼠标点击 Workspaces item（`handle_sidebar_press → switch_workspace → resize_panes`）
  - 断言 `state.prompt.scroll()` 不变、`prompt_scrollbar` thumb 位置不变

## 2. 状态层修改

### 2.1 `src/app/prompt.rs`

- [ ] `resize` 改为混合规则：
  - 尺寸未变直接 `return`
  - 尺寸变化：记录 `cursor_cell().is_some()` → 更新 `width/height` → `scroll.min(max_scroll())` → 仅“变化前可见且变化后不可见”时 `scroll_cursor_into_view()`
- [ ] `restore_history`：删除 `self.scroll_cursor_into_view();`
- [ ] 新增 `replace_text_preserving_view(&mut self, text: &str)`：
  - 保存旧 `scroll/cursor`
  - 调用 `set_text(text)` 复用净化/清浮层/清历史/已保存态
  - 光标 `floor_char_boundary` + `clamp_cursor`
  - `scroll = old.min(max_scroll())`

### 2.2 树列表

- [ ] `src/app/prompt/tree/panel.rs::clamp_selection` 删除“选中项滚入视口”两个分支，只保留选中吸附与 `scroll.min(max)`
- [ ] `src/app/update/prompt/tree_view.rs`：
  - [ ] `select_prompt_item` / `select_prompt_temp` / `select_prompt_folder` 只设置 `selected`，删除 `scroll_selection_into_view` 与不再需要的 `visible`
  - [ ] `clamp_prompt_tree_scroll` 改为只钳制；若实现后仍无生产调用方，删除并同步测试
- [ ] 保留 `scroll_selection_into_view` 仅给：`move_prompt_selection`、`apply_prompt_search` 非空过滤、文件夹折叠/展开

### 2.3 Pages 列表

- [ ] `src/app/update/prompt/pages_view.rs`：
  - [ ] `clamp_page_view` 改为只钳制选中/滚动
  - [ ] 新增 `reveal_page_selection`（原 keep_visible 逻辑）
  - [ ] `move_page_selection` / `apply_page_search` / `clear_page_search` 改用 `reveal_page_selection`
  - [ ] `show_prompt_pages` 删除 `pages_view.scroll = 0`，进入后只 `clamp_page_view`
- [ ] `src/app/update/prompt/pages.rs`：
  - [ ] 用户显式新建/打开页（约 `226`、`270`、`500`）继续 `reveal_page_selection`
  - [ ] 删除页（约 `650`）改用 `clamp_page_view`
- [ ] `src/app/update/editor.rs::restore_prompt_view`（约 `438`）继续使用 `clamp_page_view`

### 2.4 Recent 抽屉

- [ ] `src/app/update/prompt/recent.rs::sync_prompt_recent_selection`：
  - 删除 `keep_visible`
  - `recent_scroll = recent_scroll.min(rows.len().saturating_sub(recent_visible.max(1)))`
  - 保留 `recent_active` 更新与空行处理
- [ ] `keep_visible` 仅保留给 `move_prompt_recent`、`scroll_prompt_recent`、`set_prompt_recent_scroll`、`toggle_prompt_recent`

### 2.5 停泊草稿后台刷新

- [ ] `src/app/update/prompt/mod.rs:155-169`：`set_text(&active_content)` → `replace_text_preserving_view(&active_content)`
- [ ] `src/app/update/prompt/save.rs:155-163`：同上
- [ ] `reload_editor_from_active_page`（`pages.rs:681-708`）保持 `set_text`（换文档语义）

## 3. 既有测试调整

- [ ] `src/app/prompt/tests.rs::resize_keeps_cursor_visible_and_guards_zero_size` 按 I4 拆成“尺寸变化且光标原本可见”与“尺寸未变”
- [ ] `src/app/update/prompt/tests.rs::scroll_clamps_and_keeps_selection_visible` 按 `clamp_prompt_tree_scroll` 新语义更新断言
- [ ] `src/app/update/prompt/pages_view/tests.rs` 中依赖 `show_prompt_pages` 重置 scroll 的断言按决策 5 更新
- [ ] `src/app/update/prompt/recent/tests.rs` 中依赖自动同步 `keep_visible` 的断言改为只钳制
- [ ] 其余失败测试必须先判断是“旧 bug 当规格”还是真实回归；前者改写并注明原因

## 4. 文档同步

- [ ] `docs/standards/tui-design-requirements.md`：补一条 prompt 滚动所有权/自动路径只钳制的不变量
- [ ] 本目录 `design.md` / `task.md`：实现后标注完成状态、记录与设计的差异
- [ ] 如实现中发现新的例外，先回到 §6 边界表登记，禁止静默扩大行为

## 5. 验证（按影响范围）

- [ ] `cargo fmt`
- [ ] 精准测试（先跑新增用例）：
  - `cargo test --lib resize_same_size_keeps_manual_scroll`
  - `cargo test --lib resize_shrink_keeps`
  - `cargo test --lib restore_history_keeps_scrolled_view`
  - `cargo test --lib replace_text_preserving_view`
  - `cargo test --lib workspace_switch_preserves`
  - `cargo test --lib show_prompt_tree_preserves_scroll`
  - `cargo test --lib show_prompt_pages_preserves_scroll`
  - `cargo test --lib background_parked_refresh_preserves_scroll`
  - `cargo test --lib recent_auto_sync_does_not_scroll`
  - `cargo test --lib workspace_item_click_keeps_prompt_scroll`
- [ ] 受影响模块回归：
  - `cargo test --lib app::prompt`
  - `cargo test --lib app::update::prompt`
  - `cargo test --lib event::tests::prompt_editor`
- [ ] `cargo clippy --all-targets -- -D warnings`
- [ ] `cargo test`
- [ ] 用户运行 `cargo run` 按 design §9 手工验收

## 6. 提交建议

- 提交 1：状态层（`prompt.rs` / tree / pages / recent）＋对应状态层测试
- 提交 2：调用点（`editor.rs` / `mod.rs` / `save.rs` / `pages.rs`）＋更新层/事件层测试＋文档

## 7. 完成定义

- 设计 §9 手工验收全部通过；
- 新增回归测试覆盖 design §7 全部条目；
- `cargo fmt` / `cargo clippy --all-targets -- -D warnings` / `cargo test` 全绿；
- 本文件所有 checkbox 勾选，design.md 状态改为“已完成”。
