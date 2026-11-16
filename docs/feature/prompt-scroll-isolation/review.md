# Prompt 容器滚动隔离 — Review

- 审查对象：`fix/prompt-scroll-isolation`（`dev...HEAD`）
- 关联：`design.md`、`task.md`
- 审查方式：逐条核对 design §4 不变量与 §5 调用点分类；用 codegraph / codebase-memory 追踪 `scroll_cursor_into_view`、`scroll_selection_into_view`、`keep_visible`、`clamp_*` 的全部生产调用点；运行新增与既有回归测试验证行为。
- 结论：主复现链路（工作区切换拽回光标）已修复，I1 / I3 / I4 成立；I5 与 I2 的“几何变化”各有一处缺口，另有测试与文档偏差。已按 P0 → P2 修复并补齐验证。

## 1. 关键点核对

| 不变量 | 结论 | 证据 |
|--------|------|------|
| I1 所有权：编辑器 / 树 / 页 / Recent 各自持有滚动 | 成立 | 四份滚动状态分别位于 `Prompt`、`PromptPanel`、`PageListView`、`PromptPanel.recent_scroll`，无跨容器读写 |
| I2 自动路径只钳制 | 除“几何变化”外成立 | 工作区切换、树/页重载、保存回执、Recent 自动同步均已改为只钳制；`resize_panes` 的真实尺寸变化未回写列表上限（见 P1-1） |
| I3 用户滚动意图 | 成立 | 滚轮 / 滚动条 / ↑↓ / 搜索过滤 / 编辑与折叠展开仍保留 reveal |
| I4 尺寸混合规则 | 成立 | `Prompt::resize` 同尺寸直接返回；尺寸变化仅“变化前光标可见”时继续跟随 |
| I5 换文档例外 | 停泊草稿换页被破坏 | `apply_prompt_tree` 把页身份切换并入“同页刷新”分支（见 P0-1） |

## 2. 问题清单

### P0-1 停泊草稿换页仍按“保留视口”刷新，违反 I5（已修复）

- 位置：`src/app/update/prompt/mod.rs::apply_prompt_tree` 的停泊草稿刷新分支。
- 现象：同一 prompt 键的另一个面板切换了“上次打开的页”时，后台 `apply_prompt_tree` 把停泊草稿刷新成新页正文，但保留旧页的光标偏移与滚动；切回该工作区后光标落在新页正文中段，滚动停在旧页阅读位置。
- 错误原因：实现把 `page_switched` 与“同一页正文更新”合并进同一个 `replace_text_preserving_view` 调用。该方法的语义是 design 决策 8 / I5 的“同页后台刷新”；页身份变化属于换文档，必须走 `set_text`（光标文末、滚到光标、清空撤销历史）。
- 影响：换页语义回归，与 design 决策 8 直接冲突。无数据损坏（正文仍是新页内容），但用户切回后从错误光标位置开始编辑，撤销历史语义也被“保留视口”掩盖成不一致状态。
- 修复：拆分两个分支——`page_switched` 用 `set_text`，同页正文更新才用 `replace_text_preserving_view`。
- 验证：新增 `parked_page_switch_uses_new_document_view`。

### P1-1 真实尺寸变化后列表滚动未钳制，违反 I2 的“几何变化只钳制”（已修复）

- 位置：`src/app/update/pane.rs::resize_panes`（prompt 分支）；涉及 `PromptPanel::scroll`、`PageListView::scroll`、`PromptPanel.recent_scroll`。
- 现象：窗口变高后树 / 页列表 / Recent 抽屉的可见行数变大、`max` 变小，但滚动偏移保留旧值并可能越界。渲染层 `effective_scroll` 会把越界值按末行显示，表现为“贴底 + 空白”；首次 ↑/↓、滚轮或翻页会从越界状态开始钳制，产生跳位。
- 错误原因：实现删除了 `clamp_prompt_tree_scroll`（确认无生产调用方），但没有为“几何变化”补一个只钳制的调用点；`resize_panes` 只同步了编辑器尺寸与 `recent_visible`，没有回写三个列表的新上限。design §4 I2 明确把“几何变化同步”列为只钳制路径，实现漏掉了这个入口。
- 影响：容器状态与视口不自洽；真实缩放（窗口、侧栏拖宽、Recent 开合）后的第一次导航出现跳位。属于“平时被渲染层钳制掩盖、交互时暴露”的缺陷。
- 修复：新增 `app::update::prompt::clamp_prompt_list_scroll(state)`：树用只钳制版 `clamp_selection`，页仅在存在编辑绑定时用 `clamp_page_view`（树视图下页列表休眠，不能按 0 行清零），Recent 按 `recent_page_count` 与 `recent_visible` 钳制；在 `resize_panes` 更新 prompt 尺寸与 `recent_visible` 后调用。
- 验证：新增 `resize_panes_clamps_prompt_tree_scroll`、`clamp_prompt_list_scroll_clamps_all_views`、`clamp_prompt_list_scroll_keeps_dormant_pages_scroll`。

### P2-1 设计要求的用例缺失，task.md 却标记完成（已修复）

- 位置：`docs/feature/prompt-scroll-isolation/task.md` §1.2、`design.md` §7.2。
- 现象：`workspace_switch_preserves_target_tree_scroll` 在实际测试代码中不存在（`workspace_switch_preserves_target_pages_scroll` 存在）；task.md 已勾选。
- 错误原因：实现时用 `show_prompt_tree_preserves_scroll_without_revealing_selection` 覆盖了“进入树视图不 reveal”，但漏掉了“切换工作区恢复目标树只钳制”这条独立路径，勾选时未回查。
- 影响：目标工作区树滚动在 `swap_display_prompt` 路径上缺少回归保护。
- 修复：补 `workspace_switch_preserves_target_tree_scroll`；修正 task.md 中的测试名与勾选状态。

### P2-2 停泊草稿保存回执刷新缺少独立用例（已修复）

- 位置：`src/app/update/prompt/save.rs::mark_prompt_page_persisted`。
- 现象：`background_parked_refresh_preserves_scroll` 只覆盖 `apply_prompt_tree`；`mark_prompt_page_persisted` 的同页刷新路径没有测试。design §7.2 要求两条路径都覆盖。
- 错误原因：两条后台路径共用 `replace_text_preserving_view`，实现时按“同一逻辑”只写了一条用例，未按调用点分别验证。
- 影响：保存回执路径若条件写错（如 `sharing` 判断或修订号）不会被测试发现。
- 修复：新增 `parked_save_receipt_preserves_scroll`。

### P3-1 文档与实现细节不一致（已修复）

- 位置：`docs/feature/prompt-scroll-isolation/task.md` §2.1、§1.2。
- 现象：文档写 `replace_text_preserving_view` 内部“调用 `set_text(text)`”，实际实现是抽出私有 `replace_text` 复用共同部分；多处测试名与代码不一致；design 状态仍停留在“已实现，待手工验收”。
- 错误原因：实现重构后未回写文档，测试命名在落地时调整但未同步清单。
- 影响：后续维护者按文档读代码会产生误导，验收清单不可信。
- 修复：更新 task.md / design.md 的实现记录、测试名与状态，并在 review.md 记录修复过程。

## 3. 修复记录

- P0-1（`add1e82d`）：`apply_prompt_tree` 拆分换页 / 同页刷新分支；新增 `parked_page_switch_uses_new_document_view`。
- P1-1（`add1e82d`）：新增 `clamp_prompt_list_scroll` 并在 `resize_panes` 更新尺寸 / `recent_visible` 后调用；新增 `resize_panes_clamps_prompt_tree_scroll`、`clamp_prompt_list_scroll_clamps_all_views`。
- P2-1 / P2-2（`add1e82d`）：补 `workspace_switch_preserves_target_tree_scroll`、`parked_save_receipt_preserves_scroll`。
- P3-1：同步 `design.md` / `task.md` / 本文件。

验证：

- `cargo test --lib -q -- parked_page_switch_uses_new_document_view parked_save_receipt_preserves_scroll workspace_switch_preserves_target_tree_scroll resize_panes_clamps_prompt_tree_scroll clamp_prompt_list_scroll_clamps_all_views`：5 passed；`clamp_prompt_list_scroll_keeps_dormant_pages_scroll` 单独通过。
- `cargo test --lib -q app::update::prompt`：137 passed。
- `cargo test --lib`：5660 passed，0 failed，6 ignored。
- `cargo clippy --all-targets -- -D warnings`、`cargo fmt`：通过。
- 全量 `cargo test`（含集成测试）：见提交前验证记录。

## 4. 未覆盖与手工验收

- 以下行为按设计决策保留，需 `cargo run` 手工确认：
  - 进入树 / Pages 视图不 reveal 当前选中项 / 当前页；
  - Recent 自动同步不跟随 active 行；
  - 窗口缩放时“变化前光标可见才继续跟随”的混合手感；
  - 左侧 Workspaces / Agents / 历史列表行为不变。
