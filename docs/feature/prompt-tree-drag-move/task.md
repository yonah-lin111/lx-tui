# Prompt 列表拖拽移动（进/出文件夹）— 执行任务

- 任务目录：`docs/feature/prompt-tree-drag-move/`
- 规则：严格遵守 `AGENTS.md`；生产代码禁止 `unwrap()` / `expect()`（测试除外）；用户可见文案进
  `ui/text.rs`；颜色只从 `ui/style.rs` 取；`app/` 与 `ui/` 边界不得打破（渲染只读、状态只在事件层改）；
  测试驱动（先红后绿）；单文件超 1000 行必须按职责拆分。
- 需求原文：在右侧 prompt 列表页，参考左侧 ws 的拖拽移动交互，为 prompt 添加拖拽移动，支持拖拽进/出文件夹。
- 前置：用户已审核并批准同目录 `design.md` 与本清单（7 条 Grill Me 边界已逐条确认）。

## 执行清单

### 1. 需求与边界对齐（Grill Me 阶段，已完成）

- [x] 质问 1：只做 reparent，不引入 `sort_order`、不写迁移 → 采纳推荐。
- [x] 质问 2：文件夹可拖；进文件夹要求目标为根层、被拖为根层；子文件夹不接收；≤2 层不变 → 采纳推荐。
- [x] 质问 3：落点 = 文件夹行→该文件夹 / 条目行→其所在文件夹 / 空白→根层 / Section 不响应 /
      树外取消；不自动展开折叠文件夹 → 采纳推荐。
- [x] 质问 4：点击语义从 Down 推迟到 Up，Down 只选中并登记起点，Drag 跨行才置 dragging → 采纳推荐。
- [x] 质问 5：不做乐观更新；`Effect::MovePrompt`/`MoveFolder` → pump 写库 → `reload_tree`；
      失败补 Danger toast；Temp/Section 不可拖；只改 `folder_id`/`parent_folder_id`/`updated_at` → 采纳推荐。
- [x] 质问 6：拖动中仅被拖行 `style::selection()`；不画预览；非法落点松手 toast；树外静默取消 → 采纳推荐。
- [x] 质问 7：带子文件夹的文件夹拖进文件夹 → 拒绝 + toast（否则第三层不渲染） → 采纳推荐。
- [x] 默认处理（未单独提问）：搜索词非空禁用拖拽；拖动期抑制 hover/tooltip。
- [x] 用户审核 `design.md` 与本清单，确认后开始实施。

### 2. 复现测试（先红）

> 先写测试并确认失败，记录失败原因，再动实现。

- [x] `src/ui/prompt/tree/tests.rs`：
  - [x] `drop_hit_at_distinguishes_row_blank_and_outside`
  - [x] `dragged_row_gets_selection_background`
- [x] `src/app/update/prompt/tests.rs`（或 `src/app/update/prompt/drag/tests.rs`）：
  - [x] `down_does_not_toggle_folder_until_up`
  - [x] `drag_arms_only_after_row_change`
  - [x] `drop_on_item_uses_container_folder`
  - [x] `drop_on_blank_moves_to_root`
  - [x] `move_folder_into_root_folder_is_allowed`
  - [x] `move_folder_into_subfolder_is_rejected_depth`
  - [x] `move_folder_with_children_is_rejected_nested`
  - [x] `move_onto_self_and_same_container_is_noop`
  - [x] `finish_after_drag_does_not_toggle_nor_open_editor`
  - [x] `drag_disabled_when_search_active`
  - [x] `temp_row_never_produces_effect`
- [x] `src/db/tests.rs`：
  - [x] `move_prompt_updates_folder_and_persists`
  - [x] `move_prompt_rejects_foreign_or_missing_folder`
  - [x] `move_folder_rejects_depth_children_and_self`
  - [x] `move_folder_to_root_clears_parent`
- [x] `src/event/tests.rs`：
  - [x] `mouse_drag_prompt_item_into_folder_queues_move_effect`
  - [x] `mouse_drag_prompt_row_does_not_open_editor`
  - [x] 改造 `prompt_tree_double_click` helper（`:3502`）为 Down+Up 序列，
        连带修 `mouse_double_click_prompt_item_row_opens_editor`（`:3524`）、
        `mouse_double_click_prompt_temp_row_opens_editor`（`:3615`）、
        `mouse_double_click_prompt_folder_toggles_once`（`:3633`）。
- [x] `tests/prompt_management.rs`：`prompt_drag_move_persists_across_reload`。

### 3. 状态层（`src/app/`）

- [x] `src/app/state.rs`：
  - [x] 新增 `PromptDragSource { Folder(i64), Item(i64) }`（带文档注释：`Temp` 无持久身份，不进入）。
  - [x] 新增 `PromptDrag { workspace, source: Option<PromptDragSource>, from_row, dragging, enabled }`。
  - [x] `AppState` 新增 `pub prompt_drag: Option<PromptDrag>`；`AppState::demo()` 置 `None`。
- [x] `src/app/update/prompt/drag.rs`（新，约 180 行）：
  - [x] `begin_prompt_drag(state, row) -> bool`：只选中 + 登记；Section 直接 false；
        `source = None` 当 Temp；`enabled = !search_active`。
  - [x] `drag_prompt_to(state, hit: PromptDropHit) -> bool`：仅置 `dragging`，不提交。
  - [x] `finish_prompt_drag(state, hit) -> bool`：`take()` 清场 → 未拖动走
        `prompt_row_double_click` / `click_prompt_row`；拖动走 `prompt_drop_container` + `queue_move`。
  - [x] `prompt_drop_container(state, hit) -> Result<Option<i64>, PromptMoveReject>`（纯函数，规则表见 design §3.3）。
  - [x] `queue_move(...)`：目标容器与当前相同 → false；否则 push `Effect::MovePrompt/MoveFolder`。
  - [x] `show_reject_toast`：`ToastKind::Danger` + `Some(state.prompt.id())` 锚点。
  - [x] `PromptMoveReject { Depth, Nested, Noop }`。
  - [x] `drag.workspace != state.prompt_workspace()` 时整体丢弃。
- [x] `src/app/update/prompt.rs`：`mod drag;` + `pub use drag::*;`。
- [x] `src/app/update/prompt/tree_view.rs`：`PROMPT_DOUBLE_CLICK_WINDOW` 的可见性确认（`drag.rs` 复用）。
- [x] 清场点各加 `state.prompt_drag = None;`：
  - [x] `src/app/update/right_panel.rs:10 select_right_tab`
  - [x] `src/app/update/workspace.rs:299 switch_workspace`
  - [x] `src/app/update/prompt.rs:237 toggle_prompt_panel_mode`
- [x] `src/app/effects.rs`：新增 `MovePrompt { workspace, id, folder_id }` /
      `MoveFolder { workspace, id, parent_id }`（紧跟 `RenameFolder`/`DeleteFolder`，`src/app/effects.rs:53`）。

### 4. 持久层（`src/db/`）

- [x] `src/db/mod.rs`：
  - [x] `pub fn move_prompt(&self, id: i64, folder_id: Option<i64>) -> Result<()>`：
        `UPDATE workspace_prompt SET folder_id = ?, updated_at = datetime('now')`；
        文件夹解析与 `create_prompt`（`:518`）同款校验（外键 external_id、同工作区、缺失 → `FolderMissing`）。
  - [x] `pub fn move_folder(&self, id: i64, parent_id: Option<i64>) -> Result<()>`：
        被拖文件夹有子文件夹 → `FolderHasChildren`；`parent == id` → `DepthExceeded`；
        `parent.parent_folder_id.is_some()` → `DepthExceeded`；被拖 `parent_folder_id.is_some()` → `DepthExceeded`。
  - [x] 新增 `DbError::FolderHasChildren` + `Display`（`:108` 附近）+ `code()`（`:155` 附近）。
  - [x] **不改** `SCHEMA_VERSION`（保持 14），不加迁移。
- [x] `src/db/service.rs`：
  - [x] 紧跟 `RenameFolder` 分支（`:589`）新增 `MovePrompt` / `MoveFolder` 分支：
        `with_db` → `reload_tree` → 失败补 toast。
  - [x] 确认 `is_network_effect`（`:1148`）**不**包含二者。

### 5. 事件与渲染

- [x] `src/event/mouse/press.rs:388` 分支：Down 改为 `select_prompt_row` + `begin_prompt_drag`；
      删除此处的 `prompt_row_double_click` / `open_prompt_row` / `click_prompt_row` 调用。
- [x] `src/event/mouse/drag.rs`：
  - [x] `handle_mouse_drag` 在 `workspace_drag` 分支（`:193`）之后、窗格转发（`:201`）之前
        插入 `prompt_drag` 分支（`drop_hit_at` → `drag_prompt_to` → return）。
  - [x] `handle_mouse_up_left` 在 `prompt_scroll_drag.take()`（`:309`）之后插入 `prompt_drag` 分支
        （`drop_hit_at` → `finish_prompt_drag` → return）。
- [x] `src/event/mouse/hover.rs`：
  - [x] `:243` 树 hover 条件追加 `|| state.prompt_drag.is_some()`。
  - [x] `:261` tooltip 条件追加 `|| state.prompt_drag.is_some()`。
- [x] `src/ui/prompt/tree.rs`：
  - [x] 新增 `PromptDropHit { Row(usize), Root, Outside }` + `pub fn drop_hit_at(...)`（与 `row_at` `:171` 同源换算）。
  - [x] `render(...)` 增加 `drag: Option<usize>` 参数，`paint_row`（`:187`）增加 `dragging: bool`，
        整行 `style::selection()`（复用 `fill_row` `:291`）。
- [x] `src/ui/prompt_frame.rs:179`：调用点补 `state.prompt_drag.and_then(...)`。
- [x] `src/ui/prompt/tree/tests.rs`：20 处 `render(...)` 调用补 `None`。
- [x] `src/ui/text.rs`（`:533` 附近）：
  - [x] `TOAST_PROMPT_MOVE_FAILED` = `"Failed to move; unchanged"`
  - [x] `TOAST_PROMPT_MOVE_DEPTH` = `"Cannot move: folders support two levels"`
  - [x] `TOAST_PROMPT_MOVE_NESTED` = `"Cannot move: folder contains subfolders"`

### 6. 文档同步

- [x] `docs/standards/tui-design-requirements.md`「交互」补 prompt 列表拖拽口径：
      reparent 语义、行级阈值、整行底色、拖拽期抑制 hover、非法落点松手 toast、不做预览/自动展开。
- [x] 本目录 `task.md` 勾选并补「完成记录」。

### 7. 精确验证（只跑受影响范围）

- [x] `cargo fmt`
- [x] `cargo clippy --all-targets -- -D warnings`
- [x] `cargo test prompt`（状态/渲染/DB/事件中与 prompt 相关的用例）
- [x] `cargo test --test prompt_management`
- [x] 交互与外观由用户 `cargo run` 确认（见 design §8.6）

## 完成记录

（实施后填写：分支、worktree、关键偏差、测试数量、遗留手工验收项。）

- 分支：`feat/prompt-tree-drag-move`（worktree：`.worktrees/prompt-tree-drag-move`，
  基于 `dev` @ `8707aa7`）。**待用户确认后合并**。
- 迁移：无（`SCHEMA_VERSION` 保持 14，未新增迁移；`DbError::FolderHasChildren` 仅内存语义）。
- 与设计的偏差（详见 `design.md` §11）：
  1. `PromptDropHit` 定义在 `app/prompt/tree.rs` 而非 `ui/`：`app/` 只允许依赖 `ui::text`，
     反向依赖会破坏分层；几何换算仍在 `ui::prompt::tree::drop_hit_at`。
  2. `prompt_drop_container` 收 `&PromptDrag`（`finish` 已 `take()` 掉现场），语义不变。
  3. **修正设计的真缺陷**：`Root` 落点也算跨行。原口径下"末行条目拖进空白回根层"没有中间行可跨，
     `dragging` 永不为真、松手退化成单击，需求原文的"拖出文件夹"最常见路径不可达
     （`drop_on_blank_moves_to_root` 先红后绿为证）。
  4. 额外清场：`handle_mouse_down_left` 入口丢弃残留现场（拖动中指针移出窗口会丢 Up，
     否则下一次无关的 Up 会按旧行号误点）；回归用例 `mouse_down_discards_stale_prompt_drag`。
  5. `move_prompt` 在 `Some(folder)` 路径要求条目存在（`FolderMissing`），`None` 路径保持原风格。
- 测试：`cargo test` 全绿 —— lib 3399 passed / 0 failed / 2 ignored，15 个集成目标全 ok。
  新增：状态层 12 例、渲染 2 例、DB 4 例、事件 3 例、集成 1 例；
  `cargo clippy --all-targets -- -D warnings` 无告警；`cargo fmt` 通过。
  注：`event::tests::worktree_candidates_do_not_resurrect_mention_panel_on_todo` 曾因
  `$TMPDIR/lx-tui-wt-gate.db` 遗留（`user_version=15`，来自另一分支的旧测试库）报
  `UnsupportedSchema`，删除该临时文件后转绿；与本次改动无关。
- 手工验收项（Agent 无法交互式验证 TUI，需 `cargo run`）：
  1. 拖条目进文件夹、拖回根层，重启后仍在原容器。
  2. 拖根层文件夹进另一根层文件夹、拖回根层。
  3. 子文件夹作落点、含子文件夹的文件夹拖进文件夹 → 底部 Danger toast。
  4. 单击文件夹只在松手时折叠/展开；双击条目仍进编辑器；拖到树外松手无任何提示。
