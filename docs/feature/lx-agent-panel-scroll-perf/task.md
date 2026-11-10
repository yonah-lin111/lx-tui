# lx 子 Agent 面板滚动性能与列表对齐 — 实施任务

- **目录**：`docs/feature/lx-agent-panel-scroll-perf/`
- **关联设计**：[`design.md`](design.md)
- **验证记录**：[`verification.md`](verification.md)
- **Review 记录**：[`review.md`](review.md)
- **分支**：`feat/lx-agent-panel-scroll-perf`
- **Worktree**：`.worktrees/lx-agent-panel-scroll-perf`
- **状态**：已完成并合并到 `dev`（merge commit `061e8bba`；worktree 与分支已清理）

> 下方复选框保留为分阶段实施清单；实际实现、review 修复与验证结果以
> [`review.md`](review.md) / [`verification.md`](verification.md) 为准。

---

## 0. 前置条件

- [ ] 用户审核通过 `design.md` 与本文档。
- [ ] 确认 worktree 与分支已创建：
  - `git worktree list` 包含 `.worktrees/lx-agent-panel-scroll-perf  [feat/lx-agent-panel-scroll-perf]`
- [ ] 完成新工作区的 codegraph / codebase-memory 初始化：
  - `codegraph init -y`（在 worktree 根目录执行）
  - `codebase-memory-mcp.index_repository({ repo_path: "<worktree 绝对路径>", mode: "full" })`
- [ ] 建立基线：修改前先跑一次现有测试与性能测试，记录数字：
  ```bash
  cargo test
  cargo test --release -- --ignored perf_lx_history
  ```
- [ ] 不自动 `git commit` / `git push`：完成实现与验证后交给用户审核，提交动作需另行授权。

---

## 1. 阶段一：事件层按帧合并卡片刷新

### 1.1 新增状态字段

文件：`src/app/state/lx/agent.rs`

- [ ] 在 `AgentsState` 增加：
  ```rust
  /// 父消息里的 `spawn_agent` / `wait_agent` 卡片待回填。
  pub cards_dirty: bool,
  ```
- [ ] 确认 `AgentsState::default()` 自动为 `false`（`#[derive(Default)]`）。
- [ ] 如有手写构造 `AgentsState { runs }` 的地方，补 `cards_dirty: false`。
  - 检索：`grep -rn "AgentsState {" src --include='*.rs'`

### 1.2 新增 request / flush

文件：`src/app/update/chat/subagent_cards.rs`

- [ ] 新增 `request_subagent_cards_refresh`：
  ```rust
  pub fn request_subagent_cards_refresh(state: &mut AppState, pane: PaneId) {
      let Some(target) = state.pane_mut_anywhere(pane) else {
          return;
      };
      target.lx.agents.cards_dirty = true;
  }
  ```
- [ ] 新增 `flush_subagent_cards`：
  ```rust
  pub fn flush_subagent_cards(state: &mut AppState) -> bool {
      let panes: Vec<PaneId> = state
          .all_pane_ids()
          .into_iter()
          .filter(|pane| {
              state
                  .pane_anywhere(*pane)
                  .is_some_and(|target| target.lx.agents.cards_dirty)
          })
          .collect();
      let mut changed = false;
      for pane in panes {
          if let Some(target) = state.pane_mut_anywhere(pane) {
              target.lx.agents.cards_dirty = false;
          }
          changed |= refresh_subagent_cards(state, pane);
      }
      changed
  }
  ```
- [ ] 给 `refresh_subagent_cards` 补文档注释：**只允许 `flush_subagent_cards` 与显式同步场景调用，delta 路径禁止直接调用**。

### 1.3 替换 delta 路径调用

文件：`src/app/update/chat/subagent.rs`

- [ ] `on_subagent_delta` 末尾（约 L126）：
  ```rust
  if changed {
      request_subagent_cards_refresh(state, pane);
  }
  changed
  ```
  **禁止**把 request 的返回值并入 `changed`；置位是调度信号，不是内容变更。
- [ ] `on_subagent_tool_pending`（约 L177）同样替换。
- [ ] `on_subagent_tool_running`（约 L192）同样替换。
- [ ] `on_subagent_tool_settled`（约 L246）同样替换。
- [ ] `set_subagent_list` 的卡片刷新（约 L309）只在列表元数据真的变化时 request，
  返回值只包含 `changed || focused || refresh_status`。
- [ ] 检查 `on_subagent_finished` 内部是否有直接刷新；有则改 request。

文件：`src/app/update/chat/session.rs`

- [ ] L266 的直接调用改为 request。

文件：`src/app/update/chat/process.rs`

- [ ] L71 的直接调用改为 request。

文件：`src/app/update/chat/mod.rs`

- [ ] L506 的直接调用改为 request。
- [ ] 从 `subagent` 模块重新导出 `request_subagent_cards_refresh` / `flush_subagent_cards`，供事件层使用。

文件：`src/event/chat/subagent/settle.rs`

- [ ] L261 的直接调用改为 `update::request_subagent_cards_refresh`。

### 1.4 事件循环 flush

文件：`src/event/mod.rs`

- [ ] 在 `if dirty && last_draw.elapsed() >= FRAME_INTERVAL` **内部、所有 resolve/draw 之前**加入：
  ```rust
  // 同帧内的 delta 只做一次委派卡片对账；视觉延迟最多一帧。
  update::flush_subagent_cards(state);
  ```
- [ ] 确认该位置在 `tokio::select!` 排空消息之后、`tui.draw` 之前；`cards_dirty`
  只允许在会置循环 `dirty` 的 AppEvent/effect 分支中置位。

### 1.5 阶段一测试

文件：`src/app/update/chat/tests/subagent.rs`

- [ ] 新增 `parallel_deltas_are_coalesced_into_one_card_refresh`：
  - 构造 3 个 running run；
  - 连续调用 `on_subagent_delta` 3 次；
  - 断言 `cards_dirty == true`，父卡片尚未更新；
  - 调用 `flush_subagent_cards`，断言父卡片状态更新、`cards_dirty == false`；
  - 再次 flush 返回 false。
- [ ] 新增 `request_subagent_cards_refresh_only_sets_the_flag`：
  - request 返回 `()`；调用后 `cards_dirty == true`，flush 后复位。
- [ ] `set_subagent_list_is_idempotent_and_keeps_local_runs` 在两次回投之间执行 flush：
  - 第二次内容相同的回投必须返回 false，且不得重新置 `cards_dirty`。
- [ ] 跑：
  ```bash
  cargo test app::update::chat::tests::subagent
  ```

---

## 2. 阶段二：面板打开时跳过主聊天排版

文件：`src/app/update/chat/viewport.rs`

- [ ] `resolve_lx_history` 的过滤条件改为：
  ```rust
  .filter(|target| {
      target.view == crate::app::state::PaneView::Lx && target.lx.task.is_none()
  })
  ```
- [ ] 更新函数文档：说明面板打开时主历史不绘制，跳过排版；关闭面板后下一帧恢复。
- [ ] 检查 `resolve_lx_rewind_center` 是否需要在面板打开时跳过（当前只在一次性请求时触发，保留现状）。

### 2.1 阶段二测试

文件：`src/app/update/chat/tests/history.rs` 或 `tests/viewport_anchor.rs`

- [ ] 新增 `task_page_skips_hidden_history_layout`：
  - 打开 `TaskPage`；
  - 记录 `PaneCache::builds()`；
  - 连续调用 `resolve_lx_history`；
  - 断言 builds 不增加（主历史未被排版）。
- [ ] 新增 `closing_task_page_restores_history_viewport`：
  - 面板打开时上翻/新增内容；
  - 关闭面板后调用 `resolve_lx_history`；
  - 断言 `follow_bottom` 与 `history_scroll` 语义正确。
- [ ] 跑：
  ```bash
  cargo test app::update::chat::tests
  ```

---

## 3. 阶段三：详情区共享 PaneCache

### 3.1 缓存测量 API

文件：`src/ui/main_content/lx/history.rs`

- [ ] 删除 `pub(crate) fn measure(width, source)`。
- [ ] 新增：
  ```rust
  pub(crate) fn measure_cached(
      width: u16,
      source: HistorySource<'_>,
      cache: &mut PaneCache,
  ) -> usize {
      if width == 0 {
          return 0;
      }
      cache.snapshot_source(width, source).total_rows
  }
  ```
- [ ] 确认全仓无 `history::measure` 残留调用：
  `grep -rn "history::measure" src --include='*.rs'`

### 3.2 详情测量

文件：`src/ui/main_content/lx/task/agents.rs`

- [ ] `detail_rows` 签名改为：
  ```rust
  pub(crate) fn detail_rows(
      rect: Rect,
      run: &SubagentRun,
      cache: &mut crate::ui::PaneCache,
  ) -> usize
  ```
- [ ] 内部改用 `history::measure_cached(content_width, source, cache)`。
- [ ] 在测量前写 `cache.set_viewport_rows(usize::from(rect.height))`：`resolve` 先于绘制，首帧也要有视口行数，离屏合并才可判定。
- [ ] 构造 source 时预留 `with_viewport`（阶段四落地；阶段三先保持 `from_chat` 也可）。

文件：`src/ui/main_content/lx/task.rs`

- [ ] `agents_rows` 增加 `cache: &mut crate::ui::PaneCache` 参数并透传。

文件：`src/ui/main_content/lx.rs`

- [ ] `task_agents_rows` 增加 `cache: &mut PaneCache` 参数并透传。

文件：`src/app/update/chat/viewport.rs`

- [ ] `resolve_task_agents_visible` 增加 `cache: &mut crate::ui::RenderCache` 参数。
- [ ] 测量调用改成：
  ```rust
  .and_then(|target| {
      crate::ui::main_content::lx::task_agents_rows(
          inner,
          &target.lx,
          cache.pane(*pane),
      )
  })
  ```

文件：`src/event/mod.rs`

- [ ] 调用点改为：
  ```rust
  update::resolve_task_agents_visible(state, &geometry.rects, &mut ui_cache);
  ```

### 3.3 阶段三测试

文件：`src/ui/main_content/lx/task/tests/agent_list.rs`（新建）

- [ ] 在 `src/ui/main_content/lx/task/tests.rs` 顶部加 `mod agent_list;`。
- [ ] 新增 `resolve_then_draw_builds_snapshot_once`：
  - 构造一个含 100 条消息的 run；
  - `PaneCache::default()`，先 `detail_rows`，再 `draw_detail`；
  - 断言 `cache.builds()` 只增加 1。
- [ ] 新增 `content_change_rebuilds_once_per_frame`：
  - 改一条消息内容；
  - 再次 detail_rows + draw_detail；
  - 断言 builds 增加 1。
- [ ] 跑：
  ```bash
  cargo test ui::main_content::lx::task::tests::agent_list
  ```

---

## 4. 阶段四：详情区离屏流式合并

### 4.1 视口类型

文件：`src/ui/main_content/lx/history/source.rs`

- [ ] 新增：
  ```rust
  #[derive(Debug, Clone, Copy)]
  pub(crate) struct HistoryViewport {
      pub scroll: usize,
      pub follow_bottom: bool,
  }
  ```
- [ ] `HistorySource` 增加 `pub viewport: Option<HistoryViewport>`。
- [ ] `From<&Lx>` 填主聊天视口。
- [ ] `from_chat` 填 `None`。
- [ ] 新增 `with_viewport`：
  ```rust
  pub(crate) fn with_viewport(mut self, viewport: HistoryViewport) -> Self {
      self.viewport = Some(viewport);
      self.coalesce_offscreen = true;
      self
  }
  ```

文件：`src/ui/main_content/lx/history/cache.rs`

- [ ] `offscreen_stream_throttled` 改为读 `source.viewport`：
  ```rust
  let Some(viewport) = source.viewport else {
      return false;
  };
  if self.offscreen_coalesce.is_zero()
      || !source.coalesce_offscreen
      || viewport.follow_bottom
      || self.viewport_rows == 0
  {
      return false;
  }
  // ...
  last.start >= viewport.scroll.saturating_add(self.viewport_rows)
  ```

文件：`src/ui/main_content/lx/task/agents.rs`

- [ ] `detail_rows` 构造 source 时加：
  ```rust
  .with_viewport(HistoryViewport {
      scroll: run.scroll,
      follow_bottom: run.follow,
  })
  ```
- [ ] `draw_detail` 同样加，`scroll` 用传入的当前顶部偏移。
- [ ] 确认命中/取词路径（`history_hit.rs`、`selection.rs`）仍用 `from_chat`，不参与合并。

### 4.2 阶段四测试

文件：`src/ui/main_content/lx/history/tests/perf.rs` 或 `task/tests/agent_list.rs`

- [ ] 新增 `detail_offscreen_stream_coalesces`：
  - `PaneCache` 设置 `set_offscreen_coalesce(Duration::from_millis(80))`；
  - run 上翻（`follow = false`），流式末条位于视口下方；
  - 连续追加内容并 build；
  - 断言 80ms 内 builds 不增加；
  - 超过窗口后 builds 增加。
- [ ] 新增 `detail_follow_bottom_never_coalesces`：
  - `follow = true`；
  - 每次追加都重建。
- [ ] 跑：
  ```bash
  cargo test ui::main_content::lx::history::tests
  ```

---

## 5. 阶段五：列表固定列对齐

文件：`src/ui/main_content/lx/task/agents.rs`

- [ ] 新增 `ListColumns` 结构与 `list_columns(runs, prefixes, now_ms)`。
- [ ] `tree_prefix` 保持现有“父节点不可见则不画连接符”的逻辑；返回的字符串只写入层级槽。
- [ ] `draw_list` 重写为固定列：
  - 选择标记 `❯` 固定在 `list.x`；
  - 层级槽起点 `list.x + MARKER_COLS`，宽度 `columns.gutter`；
  - 状态图标起点 = `list.x + columns.x_identity - STEP_MARK_COLS`；
  - 身份、状态、时长、标题按 `ListColumns` 的 x 坐标绘制；
  - 时长在 `age_w` 内右对齐；
  - 标题用 `clip_ellipsis`，宽度 = `list.right() - title_x`。
- [ ] 删除 token 列相关代码：
  - `run.total_tokens()`
  - `token_label` / `token_w` / `show_token` / `fixed_with_token`
- [ ] 实现降级顺序（设计 §3.5.3）：
  1. 标题放不下 → 不画标题；
  2. 时长放不下 → 不画时长；标题仍固定在 `x_title`，不会回填到时长槽；
  3. 状态放不下 → 不画状态文字，图标保留；
  4. 身份放不下 → 只画标记 / 层级槽 / 状态图标；
  5. 身份极端窄时用 `…` 截断。
- [ ] 新增 `clip_ellipsis` helper（`UnicodeWidthChar`）。
- [ ] 确认 `ROLE_MIN` / `STATUS_MIN` 仍然保留为最小列宽；不再使用的常量删除。
- [ ] 确认 `total_tokens` 若在别处无引用，不要动其他模块。

文件：`src/ui/main_content/lx/task.rs`

- [ ] `filter_row_rects` 的 `right` 改为：
  ```rust
  let right = close.map_or(rect.right(), |close| close.x.saturating_sub(GAP));
  ```
- [ ] 确认 `filter_row_rects` 的命中路径（`hit`）与绘制共用该函数。

### 5.1 阶段五测试

文件：`src/ui/main_content/lx/task/tests/agent_list.rs`

- [ ] `columns_align_across_identity_widths_and_depths`：
  - 构造 `Turing [executor]`、`Curie [executor]`、`Ada [executor]` 等不同宽度身份与 depth 1/2/3；
  - 断言所有行的身份、状态、标题列起点一致。
- [ ] `deep_tree_prefix_does_not_leak_into_scrollbar`：
  - 6..10 列窄窗 + depth 2；
  - 层级槽/状态图标放不下时不得留下裸 `└─`，不得写入滚动条列。
- [ ] `long_title_truncates_with_ellipsis`：
  - 超长标题；
  - 断言含 `…`，且右侧哨兵列保持空白。
- [ ] `narrow_width_drops_title_then_age_then_status`：
  - 8..50 列 sweep；
  - 断言标题 → 时长 → 状态的单调降级，标题不得回填时长槽；
  - 状态图标始终存在。
- [ ] `coalesced_measure_keeps_rows_dirty_until_fresh`：
  - 80ms 离屏窗口内 resolve 不得清 `rows_dirty`、不得写旧 `total_rows`；
  - 窗口结束后重测。
- [ ] `geometry_invalidation_refreshes_detail_width`：
  - 宽度变化后 `total_rows` 必须重算。
- [ ] `ready_snapshot_does_not_cross_pages`：
  - 主历史与详情同宽度时，ready 路径不得复用对方快照。
- [ ] `filter_pills_keep_gap_before_close`：
  - 断言 `all · active · done` 右端与 `[esc close]` 至少 `GAP` 列。
- [ ] 跑：
  ```bash
  cargo test ui::main_content::lx::task::tests::agent_list
  ```

---

## 6. 阶段六：性能回归

文件：`src/ui/main_content/lx/task/tests/perf.rs`（新建）

- [ ] 在 `src/ui/main_content/lx/task/tests.rs` 顶部加 `mod perf;`。
- [ ] 实现 `perf_agents_panel`，`#[ignore = "性能回归：cargo test --release -- --ignored perf_agents_panel"]`。
- [ ] 构造 6 个 `SubagentRun`，每个 200 条消息：
  - 用户/助手交替；
  - 助手包含正文、thinking、`read` 卡片、`exec_command` 卡片；
  - 正文足够长，触发折行与 markdown 排版。
- [ ] 每帧：
  - 给每个 run 追加一行流式正文；
  - `cache.advance_frame()`；
  - 调用 `detail_rows`（共享 cache）+ `draw_detail`；
  - 记录耗时。
- [ ] 统计 120 帧 P95，断言 `<= 16ms`。
- [ ] 断言 `cache.builds()` 增量不超过帧数（不允许同帧双重建）。
- [ ] 在文件头写明验收口径与命令。
- [ ] 可选：同时构造主聊天 1400 条消息，验证 `resolve_lx_history` 在面板打开时 builds 不增长。
- [ ] 跑：
  ```bash
  cargo test --release -- --ignored perf_agents_panel
  ```

> 如果 P95 超标：先检查是否仍有同帧双重建、是否遗漏 `cards_dirty` 合并；仍超标再按设计决策 11 追加“内容版本号”（本次不预先实现）。

---

## 7. 阶段七：全量验证与文档

- [ ] `cargo fmt`
- [ ] `cargo clippy --all-targets -- -D warnings`
- [ ] `cargo test`
- [ ] `cargo test --release -- --ignored perf_agents_panel`
- [ ] `cargo test --release -- --ignored perf_lx_history`（确认主历史性能未回退）
- [ ] 更新本文档状态为“已实现”，补验证结果（可新建 `verification.md`，参照 `docs/feature/lx-agent-roles-residency-and-panel/verification.md`）。
- [ ] 不自动提交；把改动、测试输出、性能数字交给用户审核。

---

## 8. 验收清单

- [ ] 多个 subagent 并行输出时，主消息区不再周期性卡顿。
- [ ] 面板打开时，主聊天不再被排版（`PaneCache::builds` 可证明）。
- [ ] Agents 详情一次 delta 最多重建一次快照。
- [ ] 详情上翻阅读时，流式更新不会每帧重排；贴底跟随时立即跟随。
- [ ] 列表所有行的状态图标、身份、状态、时长、标题列严格对齐。
- [ ] 树形连接符保留，第一层留空占位，不再出现半截字符。
- [ ] token 列已移除。
- [ ] 标题溢出以 `…` 收尾。
- [ ] 窄窗格按“标题 → 时长 → 状态”顺序降级，状态图标保留。
- [ ] 筛选胶囊与 `[esc close]` 之间至少 2 列间距。
- [ ] 现有测试全绿，新增性能测试 P95 ≤ 16ms。
- [ ] `cargo fmt` / `cargo clippy --all-targets -- -D warnings` / `cargo test` 全部通过。

---

## 9. 明确不做

- 不引入滚动加速度或帧间插值；
- 不修改 `WHEEL_LINES`；
- 不修改 Processes 页布局；
- 不修改数据库 Schema / 迁移；
- 不重构 `HistorySnapshot` / `BlockPaint` 缓存结构；
- 不引入内容版本号（性能测试不达标再追加）；
- 不自动提交或合并分支。
