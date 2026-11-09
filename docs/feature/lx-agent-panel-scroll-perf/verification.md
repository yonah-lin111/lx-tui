# lx 子 Agent 面板滚动性能与列表对齐 — 验证记录

- **分支**：`feat/lx-agent-panel-scroll-perf`
- **Worktree**：`.worktrees/lx-agent-panel-scroll-perf`
- **基线**：`fac77938`（`dev`）
- **验证日期**：2026-11-09
- **状态**：自动化验证通过；交互与视觉待用户 `cargo run` 验收

---

## 1. 自动化验证

| 命令 | 结果 |
| --- | --- |
| `cargo fmt --check` | 通过 |
| `cargo clippy --all-targets -- -D warnings` | 通过，无 warning |
| `cargo test` | 4970 passed，0 failed，5 ignored |
| `cargo test --release --lib -- --ignored perf_agents_panel` | 通过；P95 **656µs**（6 × 200，120 帧） |
| `cargo test --release --lib -- --ignored perf_lx_history` | 2 passed（主历史性能未回退） |

> `perf_agents_panel` 每帧同时执行 `refresh_subagent_cards + resolve_task_agents_visible + draw_agents_detail`，120 帧快照重建 120 次，即**每帧一次**，没有同帧双重建。

---

## 2. 新增回归覆盖

### 2.1 事件层按帧合并

`app/update/chat/tests/subagent.rs`：

- `parallel_deltas_are_coalesced_into_one_card_refresh`
  - 连续 3 条 delta 后父卡片保持旧值、`cards_dirty == true`；
  - `flush_subagent_cards` 后父卡片拿到 `running`，标志复位；
  - 再次 flush 返回 false，不再全量扫描。
- `request_subagent_cards_refresh_marks_only_once`
  - 首次请求 true，重复请求 false。

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

### 2.4 列表固定列

`ui/main_content/lx/task/tests/agent_list.rs`：

- `columns_align_across_identity_widths_and_depths`
  - 不同昵称宽度、不同树深度下身份列、状态列、标题列起点一致。
- `long_title_truncates_with_ellipsis`
  - 超长标题以 `…` 收尾且不越界。
- `narrow_width_drops_title_then_age_then_status`
  - 18 列：状态/时长/标题整列丢弃，身份与状态图标保留；
  - 8 列：身份用 `…` 截断，状态图标仍在。
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
- 列宽随筛选结果/时长位数变化，切换筛选或跨过时间阈值时整列位移一次。
