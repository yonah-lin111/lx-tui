# lx 子 Agent 面板滚动性能与列表对齐 — 验证记录

- **分支**：`feat/lx-agent-panel-scroll-perf`
- **Worktree**：`.worktrees/lx-agent-panel-scroll-perf`
- **基线**：`fac77938`（合并前 `dev` 为 `95cf9482`）
- **实现提交**：`87bdb43e`（初版） / `0ef4eab2`（review 修复）
- **合并提交**：`061e8bba`（`Merge branch 'feat/lx-agent-panel-scroll-perf' into dev`）
- **验证日期**：2026-11-10
- **状态**：已完成并合并到 `dev`（worktree 与分支已清理）
- **Review 记录**：[`review.md`](review.md)

---

## 1. 自动化验证

### 1.1 Worktree 验证（合并前）

| 命令 | 结果 |
| --- | --- |
| `cargo fmt --check` | 通过 |
| `cargo clippy --all-targets -- -D warnings` | 通过，无 warning |
| `cargo test` | 4976 passed，0 failed，5 ignored |
| `cargo test --release --lib -- --ignored perf_agents_panel` | 通过；P95 **318µs**（6 × 200，120 帧） |
| `cargo test --release --lib -- --ignored perf_lx_history` | 2 passed（主历史性能未回退） |

### 1.2 合并到 `dev` 后验证（`061e8bba`）

- 合并冲突处理：
  - `src/event/chat/subagent/runner.rs`：`sync_agent_ui_queue` 保留 `target.lx.sync_mention_subagent_runs()` 并返回 `changed: bool`。
  - `src/event/chat/subagent/settle.rs`：`finish_subagent_with_billing` 保留 `target.lx.sync_mention_subagent_runs()`，并将 `refresh_subagent_cards` 替换为 `request_subagent_cards_refresh`。
  - `src/app/update/chat/subagent.rs`、`src/event/chat/subagent/mod.rs`、`src/event/mod.rs` 自动合并后逐项复核无误；所有修改生产文件均控制在 1000 行上限内。
- 验证结果：
  - `cargo fmt --check`：通过
  - `cargo clippy --all-targets -- -D warnings`：通过，0 warnings
  - `cargo test`：**5063 passed，0 failed，5 ignored**（16 个测试套件全部通过）
  - `cargo test --release --lib -- --ignored perf_agents_panel`：通过（0.04s）
  - `cargo test --release --lib -- --ignored perf_lx_history`：2 passed（0.90s）

> `perf_agents_panel` 每帧执行 `request_subagent_cards_refresh + flush_subagent_cards +
> resolve_task_agents_visible + draw_agents_detail`，120 帧快照重建严格等于 120 次，
> 即**每帧一次**，没有同帧双重建。
>
> review 修复前该测试直接调 `refresh_subagent_cards`，且 `builds <= FRAMES` 可被漏重建
> 骗过；现改为真实事件链路并断言等号。

---

## 2. 新增回归覆盖

### 2.1 事件层按帧合并

`app/update/chat/tests/subagent.rs`：

- `parallel_deltas_are_coalesced_into_one_card_refresh`
  - 连续 3 条 delta 后父卡片保持旧值、`cards_dirty == true`；
  - `flush_subagent_cards` 后父卡片拿到 `running`，标志复位；
  - 再次 flush 返回 false，不再全量扫描。
- `request_subagent_cards_refresh_only_sets_the_flag`
  - request 返回 `()`，只置 `cards_dirty`；flush 后复位。

### 2.2 隐藏主历史不排版

`app/update/chat/tests/history.rs`：

- `task_page_skips_hidden_history_layout`
  - 面板打开时 `resolve_lx_history` 不增加 `PaneCache::builds`；
  - 关闭面板后下一帧恢复并只重建一次。

### 2.3 详情共享快照与离屏合并

`ui/main_content/lx/task/tests/agent_list.rs`：

- `detail_measure_and_draw_share_one_snapshot`
  - `detail_rows` + `draw_detail` 只 build 一次；
  - 内容变化后每帧只 build 一次。
- `detail_offscreen_stream_coalesces`
  - 上翻阅读时，80ms 窗口内流式末条变化不重建；
  - 关闭窗口后重建。
- `detail_follow_bottom_never_coalesces`
  - 贴底跟随时立即重建。

### 2.3.1 Review 后补充的回归

`ui/main_content/lx/task/tests/agent_list.rs`：

- `coalesced_measure_keeps_rows_dirty_until_fresh`
  - 80ms 离屏窗口内 resolve 保留 `rows_dirty`，不写旧 total；窗口结束后重测。
- `geometry_invalidation_refreshes_detail_width`
  - 宽度变化后 `total_rows` 必须重算。
- `ready_snapshot_does_not_cross_pages`
  - 主历史与详情同宽度时不得复用对方快照（`snapshot_owner`）。
- `zwj_emoji_identity_keeps_columns_aligned`
  - ZWJ emoji 身份与 ASCII 身份的状态列对齐、无越界。
- `deep_tree_prefix_does_not_leak_into_scrollbar`
  - 极窄深树不留裸连接符、不写滚动条列。
- `tree_sibling_parent_comparison_is_case_insensitive`
  - 同级兄弟父路径大小写不同仍正确判断末项。

`app/update/chat/tests/subagent.rs`：

- `set_subagent_list_is_idempotent_and_keeps_local_runs` 在两次回投之间 flush，
  锁住“无内容变化不得重新置脏”的 P0 修复。
- `refresh_subagent_cards_updates_nested_spawn_cards` 断言嵌套卡片变化会置
  `run.rows_dirty`。

### 2.4 列表固定列

`ui/main_content/lx/task/tests/agent_list.rs`：

- `columns_align_across_identity_widths_and_depths`
  - 不同昵称宽度、不同树深度下身份列、状态列、标题列起点一致。
- `long_title_truncates_with_ellipsis`
  - 超长标题以 `…` 收尾且不越界。
- `narrow_width_drops_title_then_age_then_status`
  - 8..50 列 sweep：标题 → 时长 → 状态单调降级；
  - 33..36 列时标题不得回填时长槽；
  - 身份放不下时用 `…` 截断，状态图标保留；
  - 右侧哨兵列始终空白。
- `filter_pills_keep_gap_before_close`
  - 筛选胶囊与 `[esc close]` 之间固定 2 列。

`ui/main_content/lx/task/tests.rs`：

- `agents_list_indents_subtree_with_fixed_columns`
  - 保留 `├─ / └─` 与孙辈缩进；
  - 第一层与子层身份/状态/标题列对齐；
  - 不再出现 token 列。

---

## 3. 交互验收清单（用户 `cargo run`）

- [ ] 多个 subagent 并行输出时，主消息区不再周期性卡顿。
- [ ] 面板打开时滚动 Agents 详情丝滑；贴底自动跟随，上翻不被拽回。
- [ ] 列表状态图标、身份、状态、时长、标题严格对齐。
- [ ] 树形连接符完整，无半截字符；第一层留空占位。
- [ ] 超长标题显示 `…`，不压住后续内容。
- [ ] 窄窗格按“标题 → 时长 → 状态”顺序降级，状态图标保留。
- [ ] 筛选胶囊与 `[esc close]` 之间有 2 列间距。

---

## 4. 风险观察点

- 父卡片实时动作文案最多延迟一帧（约 16ms）。
- 面板打开期间主聊天 `viewport_top` 不更新，关闭后下一帧重算一次。
- 上翻阅读时滚动条总行数最多滞后 80ms。
- 任务面板列表改为 500ms 轮询；打开后新启动的进程/子 agent 最多延迟 500ms 出现。
- 列宽随筛选结果/时长位数变化，切换筛选或跨过时间阈值时整列位移一次。
