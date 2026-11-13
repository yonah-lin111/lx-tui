# Todo 聚合组优先级展示 + 工作区模型记忆：验证报告

- 分支：`feat/todo-group-priority-and-ws-model`
- Worktree：`.worktrees/todo-group-priority-and-ws-model`
- 日期：2026-11-12
- 设计依据：[design.md](./design.md) / [task.md](./task.md)
- 检索工具：`codegraph`（worktree `.codegraph/`）+ `codebase-memory-mcp`（project `lx-tui-todo-group-priority-and-ws-model`）

## 1. 自动化验证

| 命令 | 结果 |
| --- | --- |
| `cargo fmt --check` | 通过 |
| `cargo clippy --all-targets -- -D warnings` | 通过 |
| `cargo test --lib` | **5331 passed / 0 failed / 6 ignored** |
| `cargo test` | **lib 5331 passed / 6 ignored；集成测试 63 passed / 2 ignored；0 failed** |

### 1.1 新增/更新的关键用例

| 区域 | 用例 |
| --- | --- |
| Todo 聚合组渲染 | `two_and_three_item_groups_render_shared_status_and_tree_connectors`：2 条组 `❯ [ ] P1 ┌─ login api`、3 条组锚点行 `  [~] P0 ├─ migrate` |
| 组内最高优先级 | `group_priority_picks_highest_and_breaks_ties_by_first_member`：`P0<P1<P2<P3`、并列取最靠前、跳过幽灵行 |
| 徽标命中/点击归属 | `mixed_single_and_grouped_items_align_status_and_hit_priority_badges_accurately`：单条与组锚点共用 `+6..+8`，锚点行返回组内最高优先级条目的 display |
| 折行续行 | `grouped_wrapped_item_draws_vertical_trunk_on_non_last_and_blank_on_last`：`│` 与 `├` 同列（`+9`），续行正文 `+12` |
| 回退链 | `fallback_selection_prefers_valid_default_model`、`..._uses_first_valid_recent_...`、`..._falls_back_to_first_available_model`、`..._is_none_without_usable_model` |
| 最近使用 | `remember_model_prepends_dedupes_overrides_variant_and_caps`：去重前置、variant 覆盖、10 条上限 |
| 窗格记忆 | `effective_selection_prefers_valid_pane_selection` |
| 发送记录 | `try_send_pins_fallback_selection_and_records_recent_model`：钉住实际使用的 selection、recent 队首、只发 `SaveModelChoice{default_model: None}` |
| `/resume` 记录 | `session_loaded_records_recent_model_without_touching_default` |
| 工作区刷新 | `switch_and_create_workspace_refresh_active_lx_status_model`：只刷目标工作区，其它窗格 `omitted_turns_notice` 保留 |
| 配置兼容 | `recent_models_round_trip_and_empty_list_is_omitted`、`stale_recent_models_are_preserved_and_do_not_participate_in_validation` |
| 设置页合并 | `put_without_recent_models_preserves_existing_list` |
| 工作区记忆 | `create_tab_inherits_workspace_model_over_default`、`split_pane_inherits_workspace_model_over_default`、`model_selection_updates_workspace_model_memory` |
| 历史新建工作区 | `opening_new_history_entry_initializes_lx_status` |
| 失效选择回退 | `effective_selection_falls_back_when_pane_provider_disabled_or_key_missing`、`fallback_selection_normalizes_stale_recent_variant` |
| `/resume` 过滤 | `session_loaded_skips_invalid_model_in_recent`、`session_loaded_records_recent_model_without_touching_default` |
| 同 session 同步 | `session_model_syncs_to_other_panes_viewing_same_session` |
| Todo 点击端到端 | `clicking_group_badge_cycles_and_selects_badge_owner`、`clicking_group_badge_with_ghost_commits_then_updates_owner` |
| Todo 边界 | `group_priority_handles_n_gt_3_with_highest_at_last`、`grouped_narrow_panel_renders_without_panic`、`standalone_new_ghost_row_does_not_draw_dead_priority_badge`、`status_hit_does_not_escape_list_bounds` |

## 2. 手工验收（待用户执行 `cargo run`）

1. Todo 面板建 2 条 / 3 条同组待办：
   - 目标格式：`❯ [~] P1 ├─ bbbb`（锚点行）；首尾行只有 `┌─` / `└─`；
   - 徽标取组内最高优先级；
   - 点击徽标：循环切换徽标来源条目的 P 值，并选中它；
   - 长文本折行：`│` 与 `├` 同列、续行正文与首行正文对齐；
   - 双击行内编辑（1~3 行）与 3 行视口正常。
2. 模型：
   - A 工作区 `/model` 选 Claude，切到从未选过模型的 B：状态行显示 `defaultModel`，不再 `no model`；
   - 只 `/resume` 一个 GPT 会话后切到 C（`defaultModel` 为空）：显示 GPT；
   - 在 B `/model` 换模型后切回 A：A 仍显示 Claude；
   - 重启：`ai.json` 的 `recentModels` 保留，新工作区按 recent 兜底；
   - 设置页保存模型配置后 `recentModels` 未被清空。

## 3. 已知边界

- 工作区记忆是进程内 `Workspace.lx_model`（不跨重启）；新窗格/标签继承它，窗格级 `chat.selection` 可覆盖。跨重启只恢复全局 `recentModels`。
- 显式选择缺失时，回退链最后一级会让第一个可用模型成为 `/model` 的“当前项”。
- 无模型可用（未连接/无 Key/全部失效）时仍显示 `no model`，这是正确结果。
- 切换工作区/标签只刷新当前标签的 lx 状态行；后台标签在切到它时刷新，避免误消费尚未展示的 `omitted_turns_notice`。
- 同一 session 开在多个窗格时，发送 ack 与 `/resume` 会同步模型选择。
- 配置持久化并发（P3-6/P3-8）与 Todo 大组性能（P3-1）未在本次修复；见 review.md。

## 4. 追补：新建窗格/标签显示 `no model`（2026-11-12）

- **根因**：`Pane::new()` 默认 `PaneView::Lx`，而 `LxChat::new()` 的 `status` 是空的；`create_tab`、右键菜单 `split_pane`、`toggle_pane_view`（终端切回 lx）以及 worktree 直接激活新工作区都没有调用 `refresh_lx_status`，于是新 lx 窗格一直停在 `no model`。opencode 的当前选择是实时 memo，不存在这段缓存空窗。
- **修复**：上述创建/切回路径立即刷新目标窗格的 lx 状态；只刷新窗格，不消耗其它窗格的一次性上下文省略提示。
- **新增测试**：`create_tab_initializes_lx_status_model`、`split_pane_from_menu_initializes_lx_status_model`、`toggling_terminal_pane_to_lx_initializes_status_model`。
- **复测**：`cargo fmt --check`、`cargo clippy --all-targets -- -D warnings` 通过；lib **5331 passed / 6 ignored**；全量集成测试通过（lib 0 failed，集成 63 passed / 2 ignored）。

## 5. Review 修复（2026-11-12）

详见 [review.md](./review.md)。已修：

- **P1-1** 工作区历史新建并激活后补刷状态；新增 `opening_new_history_entry_initializes_lx_status`。
- **P1-2** `Workspace.lx_model` 工作区记忆；新标签/分割窗格继承，新增 3 条测试。
- **P1-3** `effective_selection` 复用 `validated_selection`；供应商停用/无 Key 回退，variant 失效规范化。
- **P1-4** Todo 徽标点击端到端测试（普通组 + 幽灵锚点）。
- **P2-1** `/resume` 失效模型不写 recent/工作区记忆；修正空断言。
- **P2-3** `refresh_fallback_lx_status` 同步空白窗格状态行。
- **P2-4** 只刷当前标签，`switch_tab` 补刷；同索引 `switch_workspace` 跳过刷新。
- **P3** 独立幽灵行死徽标、`status_at` 边界、同 session 多窗格同步、Todo 边界测试。

未修（记录在 review.md）：P3-1 Todo 大组性能、P3-6 配置无锁读改写、P3-7 ack 前记 recent、P3-8 悬空 default 阻塞 recent 落盘。
