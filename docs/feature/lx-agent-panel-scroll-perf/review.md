# lx 子 Agent 面板滚动性能与列表对齐 — Review

- **分支**：`feat/lx-agent-panel-scroll-perf`
- **Worktree**：`.worktrees/lx-agent-panel-scroll-perf`
- **基线**：`fac77938`（`dev`，合并前 `dev` 为 `95cf9482`）
- **首次实现提交**：`87bdb43e`（`perf(lx): batch subagent card refresh and align agent list`）
- **Review 修复提交**：`0ef4eab2`（`fix(lx): close agent panel review findings`）
- **合并提交**：`061e8bba`（`Merge branch 'feat/lx-agent-panel-scroll-perf' into dev`）
- **审查日期**：2026-11-10
- **审查范围**：本次提交新增/修改的事件合并、隐藏历史、详情缓存、离屏合并、Agents 列表布局、测试与文档
- **方法**：codegraph / codebase-memory 先定位调用链，再以磁盘当前代码逐行核验；3 个只读 subagent 分别审查事件层、缓存层、列表/文档，所有结论均回到代码与测试复核。

---

## 0. 关键点

本次功能不是单一“列表重绘”问题，而是四层叠加：

1. **delta 到父卡片**：`refresh_subagent_cards` 全量扫描主聊天 + 所有 run 聊天，必须从“每条 delta 一次”改成“每帧一次”。
2. **隐藏主历史**：任务面板打开时主历史不可见，却仍参与 `resolve_lx_history` 排版，并与 Agents 详情抢 `PaneCache` 的单快照槽。
3. **详情测量/绘制**：原 `detail_rows` 用临时 `PaneCache` 全量排版，绘制再排一次；必须共享事件循环缓存。
4. **列表布局**：原列宽逐行动态计算，token 列时有时无，窄窗格整项丢弃；必须固定列、固定层级槽、单调降级。

审查确认首次提交方向正确，但存在 1 个直接抵消性能目标的 P0、2 个滚动/布局正确性 P1，以及若干缓存归属、宽度失效、宽字符与测试有效性问题。以下全部已修复。

---

## 1. 问题清单

### P0-1 空闲列表回投被当成内容变化，Agents 页 60fps 空转重绘

**位置**

- `src/app/update/chat/subagent_cards.rs`：`request_subagent_cards_refresh`
- `src/app/update/chat/subagent.rs`：`set_subagent_list`
- `src/event/chat/process_page.rs`：`refresh_open_task_pages`
- `src/event/mod.rs`：`apply_subagent_effects` 返回值接入 `dirty`

**现象**

只要任务面板停在 `Agents` 且窗格绑定了会话，即使没有任何 subagent、没有任何内容变化，界面也按 `FRAME_INTERVAL` 持续重绘；每帧还会执行一次 `refresh_subagent_cards` 全量扫描。

**错误原因**

`request_subagent_cards_refresh` 返回 `!was_dirty`，`set_subagent_list` 把它并入返回值：

```rust
let cards = request_subagent_cards_refresh(state, pane);
changed || cards
```

而 `refresh_open_task_pages` 每轮事件循环都会压入一个 `ListSubagents`。列表回投内容完全相同时：

1. 上一帧 `flush_subagent_cards` 已把 `cards_dirty` 清为 false；
2. 本帧 `set_subagent_list` 调 request，`was_dirty == false`，返回 true；
3. `apply_subagent_effects` 返回 true → 事件循环 `dirty = true`；
4. 下一帧 flush 清标志，循环回到 1。

“调度信号”被误当成“内容变化信号”，与 `refresh_open_task_pages` 的每轮列表回投形成闭环。原测试 `set_subagent_list_is_idempotent_and_keeps_local_runs` 两次调用之间没有 flush，恰好掩盖了该问题。

**修复**

- `request_subagent_cards_refresh` 改为返回 `()`，禁止调用方并入 `changed`。
- delta/tool 路径只在内容真的变化时 request，并返回原始 `changed`。
- `set_subagent_list` 只在列表元数据变化时 request；返回值只包含 `changed || focused || refresh_status`。
- `sync_agent_ui_queue` 改为返回“状态/队列/blocked 是否真的变化”；`apply_subagent_effects` 对整树 `queued_count` 比较后再写回，并只在变化时 request。
- `flush_subagent_cards` 仍在绘制门控内每帧最多执行一次。

**回归测试**

- `set_subagent_list_is_idempotent_and_keeps_local_runs`：两次相同回投之间执行 flush，第二次必须返回 false，且不得重新置 `cards_dirty`。
- `request_subagent_cards_refresh_only_sets_the_flag`：request 只置位，flush 后复位。

---

### P1-1 离屏合并返回旧快照，却清掉 `rows_dirty` 并写回旧总行数

**位置**

- `src/ui/main_content/lx/history/cache.rs`：`snapshot_source` / `offscreen_stream_throttled`
- `src/ui/main_content/lx/history.rs`：`measure_cached`
- `src/app/update/chat/viewport.rs`：`resolve_task_agents_visible`

**现象**

Agents 详情上翻阅读时，最后一条流式消息在视口下方，80ms 合并窗口内追加内容：滚动条总行数不增长，拖到底被钳在旧上限；窗口结束后画面已经更新，但 `run.total_rows` 仍停留在旧值，直到下一次 delta 才恢复。

**错误原因**

`measure_cached` 调 `snapshot_source`。合并窗口内 `snapshot_source` 返回旧 `Arc<HistorySnapshot>`，但 `resolve_task_agents_visible` 无法区分“命中旧快照”和“刚构建新快照”，仍执行：

```rust
run.total_rows = total;
run.rows_dirty = false;
```

快照重建后，`rows_dirty` 已是 false，resolve 不再测量，旧 total 永久保留。

**修复**

- `PaneCache::snapshot_source_fresh` 返回 `(Arc<HistorySnapshot>, bool)`；`false` 表示命中合并窗口、快照不是最新。
- `snapshot_source` 保留原签名，内部取 `.0`。
- `measure_cached` / `detail_rows` / `agents_rows` / `task_agents_rows` 透传 freshness。
- `resolve_task_agents_visible` 只有 `fresh == true` 时才清 `rows_dirty`；旧快照期间保留 dirty，下一帧继续尝试。

**回归测试**

- `coalesced_measure_keeps_rows_dirty_until_fresh`：
  - 80ms 窗口内 resolve 后 `rows_dirty` 仍为 true、`total_rows` 不变；
  - 窗口结束后重测，`total_rows` 增长且 dirty 复位。

---

### P1-2 窄窗格降级非单调：标题会在时长槽“复活”

**位置**

- `src/ui/main_content/lx/task/agents.rs`：`draw_list`
- `docs/feature/lx-agent-panel-scroll-perf/design.md` §3.5.3

**现象**

宽度从 45 收窄到 33 的过程中，标题先消失、又出现在时长槽、再消失。示例：身份 17 列、状态 7 列、时长 2 列时，标题固定列在 36；宽 33–42 时 age 放不下，标题被塞到 `age_x`，宽 43–44 时 age 放得下、标题又消失，宽 ≥45 时标题恢复。

**错误原因**

旧实现 `title_x = if show_age { x_title } else { age_x }`：age 放不下时主动把标题左移，违背决策 10 的“从右往左丢弃标题 → 时长 → 状态”。设计文档 §3.5.3 第 2 步也写成了回退，属于设计错误。

**修复**

- 标题始终从 `list.x + columns.x_title` 开始；age 放不下时标题也不画。
- 更新 design/task 的降级说明，删除“标题回退到 age_x”。

**回归测试**

- `narrow_width_drops_title_then_age_then_status`：8..50 列 sweep，断言状态/时长按阈值单调出现，33..36 列不得出现标题省略号，标题不得回填时长槽。

---

### P2-1 嵌套卡片刷新不置 `rows_dirty`

**位置**：`src/app/update/chat/subagent_cards.rs` `refresh_subagent_cards`

**错误原因**：函数会刷新每个 run 自己消息流里的 `spawn_agent` / `wait_agent` 卡片，但没有把对应 run 的 `rows_dirty` 置 true。卡片状态变化会改变工具卡行数，详情 `total_rows` 因此滞后。

**修复**：`refresh_messages_with_runs` 对某个 run 返回 true 时，同时 `run.rows_dirty = true`。

**测试**：`refresh_subagent_cards_updates_nested_spawn_cards` 增加断言：刷新前清 dirty，刷新后必须重新置位。

---

### P2-2 resize 后详情总行数不失效

**位置**

- `src/app/update/chat/viewport.rs`：`needs_measure = run.rows_dirty || run.total_rows == 0`
- `src/event/mod.rs`：几何变化分支

**错误原因**

拖动窗格/窗口会改变正文换行与 `total_rows`，但不产生 delta，`rows_dirty` 保持 false。绘制会用新宽度重建快照，`run.total_rows` 却继续沿用旧宽度值；滚动条、PageDown、拖拽钳制、选区几何全部偏差。

**修复**

- 新增 `invalidate_task_agent_rows(state)`，在 `geometry.rects != last_rects || prompt_resize_pending` 时把所有 lx 窗格的 run 置 `rows_dirty = true`。
- resolve 下一帧重新测量。

**测试**：`geometry_invalidation_refreshes_detail_width`：40 列测一次，失效后 80 列重测，断言 total 变化且 dirty 复位。

---

### P2-3 `PaneCache` ready 路径只校验宽度，会跨页复用快照

**位置**：`src/ui/main_content/lx/history/cache.rs` `snapshot_ready_source`

**错误原因**

主历史与 Agents 详情的内容宽度经常完全相同（都等于窗格内容宽度）。任务页开关、高亮 run 切换与下一次 resolve/draw 之间 ≤16ms 的鼠标 hover/press/滚轮，可能用主历史快照解释详情行号，或反之；代码块复制、thinking 命中、滚动条拖拽都可能错位。

**修复**

- `HistorySource::owner()` 返回 `LxChat` 地址。
- `PaneCache` 记录 `snapshot_owner`；`snapshot_ready_source` 与 `snapshot_source_fresh` 都校验 owner。
- owner 变化时清空快照、宽度、非流式指纹与构建时刻。

**测试**：`ready_snapshot_does_not_cross_pages`：先建主历史快照，再用同宽度详情 source 走 ready 路径，断言必须重建且 total 不同。

---

### P2-4 ZWJ emoji 宽度口径不一致

**位置**：`src/ui/main_content/lx/task/agents.rs` `text_width`

**错误原因**

`UnicodeWidthStr::width` 会把 `👨‍👩‍👧` 这类 ZWJ 序列合并为 2 列；`Grid::put` / `clip_ellipsis` 按 `char` 逐格写入，同一串按 4 列占位。列宽按 2 算、实际写 4，导致状态列被覆盖或身份被静默截断。

**修复**

- `text_width` 改为逐字符 `.width()` 求和，与 Grid 口径一致。
- 保留 `UnicodeWidthStr` 仅用于不受该问题影响的短标记测量。

**测试**：`zwj_emoji_identity_keeps_columns_aligned`：emoji 与 ASCII 两个 run 同屏，断言状态列起点一致、右侧哨兵列空白。

---

### P2-5 原测试存在空断言，未覆盖真实降级路径

**位置**

- `src/ui/main_content/lx/task/tests/agent_list.rs`
- `src/app/update/chat/tests/subagent.rs`

**错误原因**

- `draw_list_lines` 使用与 list 完全等宽的 Grid，`Grid::set` 对越界列直接丢弃，`line.chars().count() <= width` 永远成立，无法证明不越界。
- “18 列：身份放得下”不成立：`Turing [executor]` 宽 17，`identity_room` 只有 14，实际走身份裁剪分支，状态/时长/标题三档降级分支根本没执行。
- `set_subagent_list` 幂等测试两次调用之间不 flush，正好掩盖 P0。
- 性能测试直接调 `refresh_subagent_cards`，绕开 request → dirty → flush 链路。

**修复**

- `draw_list_with_overflow` 用 `width + 8` 的 Grid、`list` 只占前 width 列，检查右侧哨兵列。
- 窄窗测试改为 8..50 列 sweep，分别断言 status/age/title 的单调降级。
- 幂等测试在两次回投之间 flush。
- 性能测试改走 `request_subagent_cards_refresh` + `flush_subagent_cards`，断言 `builds == FRAMES`。
- 新增 80ms 合并、宽度失效、跨页快照、emoji、深树、大小写兄弟等用例。

---

### P2-6 `sync_agent_ui_queue` 静默改状态，修复 P0 后可能不再触发重绘

**位置**

- `src/event/chat/subagent/runner.rs` `sync_agent_ui_queue`
- `src/event/chat/subagent/mod.rs` `apply_subagent_effects`

**错误原因**

`sync_agent_ui_queue` 直接写 `run.status` / `run.queued_count` / `blocked_subagents`，返回 `()`；整树 `queued_count` 回写也不比较。修复 P0 前 `set_subagent_list` 每轮请求掩盖了它；修复后无变化的列表回投不再置脏，队列变化可能不重绘。

**修复**

- `sync_agent_ui_queue` 返回 bool，逐字段比较后写。
- `apply_subagent_effects` 比较整树 queue 再写，`changed |= sync...`，变化时 request 卡片刷新。

**验证**：`cargo test app::update::chat::tests` 全绿；P0 幂等测试确认无内容变化不再置脏。

---

### P2-7 面板打开时列表轮询仍每轮访问 SQLite

**位置**：`src/event/mod.rs` `refresh_open_task_pages` 调用点

**错误原因**：即使修复 P0 不再置脏，`refresh_open_task_pages` 仍在每个事件循环压入
`ListSubagents` / `ListSessionProcesses`；下一轮 effect pump 会执行 `load_subagent_tree`、
`tree_queue_states` 等 SQLite 查询。空闲面板因此仍以约 60Hz 轮询数据库。

**修复**：事件循环加 500ms 列表轮询节拍；打开面板时仍由 `open_task_page` 立即压入首次请求。

**验证**：`cargo test` 全绿；进程/子 agent 列表的现有测试直接调用 `refresh_open_task_pages`，
不受节拍影响。

### P2-8 设计/任务文档与实现漂移

**位置**：`design.md`、`task.md`、`verification.md`

**错误原因**

- 状态图标公式仍写 `x_identity - mark_w - CELL_GAP`，实现是 `x_identity - STEP_MARK_COLS`。
- §3.5.3 仍保留“age 放不下时标题回退”的错误降级。
- `list_columns(runs, now_ms)` 实际多一个 `prefixes` 参数。
- 性能测试文档写“工具卡 + thinking”，实际是纯正文消息。
- 测试名称与 task.md 不一致；verification 的 18 列描述不准确。

**修复**：同步更新 design/task/verification，并新增本 review.md。

---

### P3-1 深树窄窗格可能把连接符写进滚动条列/留下裸 `└─`

**位置**：`src/ui/main_content/lx/task/agents.rs` `draw_list`

**错误原因**：先画 gutter，再判断状态图标是否放得下；极窄窗格时 gutter 可写进 `content.right()` 右侧（滚动条列），随后 mark 判断失败 `continue`，留下没有图标收口的连接符。

**修复**：先检查 `gutter_x < right && mark_x + STEP_MARK_COLS <= right`，不满足时只保留选择标记，不画 gutter。

**测试**：`deep_tree_prefix_does_not_leak_into_scrollbar`：6..10 列 + depth 2，检查哨兵列与裸连接符。

---

### P3-2 兄弟末项判断大小写敏感

**位置**：`agents.rs` `tree_prefix`

**错误原因**：父存在性用 `eq_ignore_ascii_case`，判断后续兄弟时用 `==`。同一父路径大小写不一致时，非末项会被画成 `└─`。

**修复**：兄弟父路径统一 `eq_ignore_ascii_case`。

**测试**：`tree_sibling_parent_comparison_is_case_insensitive`。

---

### P3-3 列表时长使用全局时钟而非 `TaskPage.now_ms`

**位置**：`agents.rs` `draw_list`

**错误原因**：`db::now_ms()` 每帧取实时时钟，和进程页的 `page.now_ms`（每秒推进）口径不同；测试中 `started_at_ms=0` 会算出巨大 age，列宽随当前日期变化。

**修复**：统一用 `page.now_ms`。

---

### P3-4 `view_source_plain` 的 `view.scroll` 不读 `HistoryViewport`

**位置**：`src/ui/main_content/lx/history.rs` `view_source_inner`

**错误原因**：详情 source 传了 `HistoryViewport`，但 `view.scroll` 仍从 `source.chat.history_scroll/follow_bottom` 计算（详情里是默认值）。当前调用方都自己重算 top，所以未显形；API 语义是坑。

**修复**：`view_source_inner` 优先使用 `source.viewport` 的 scroll/follow。

---

### P3-5 面板开关后 `viewport_top` 可能残留一帧

**位置**：`task_page.rs` / `subagent.rs` / `viewport.rs`

**错误原因**

面板打开期间不跑 `resolve_lx_history`，`viewport_top` 停在打开前的值。关闭面板后、下一次 resolve 前若立刻触发 `pin_lx_viewport`（^T/卡片点击），可能用旧锚点把视口拉回旧行。

**修复**：`open_task_page` / `open_agent_page` / `close_task_page` 清空 `viewport_top` 与 `suppressed_prompt_header`。

---

### P3-6 性能测试链路与断言偏弱

**位置**：`src/ui/main_content/lx/task/tests/perf.rs`

**错误原因**

- 直接调 `refresh_subagent_cards`，没有覆盖真实的 request → dirty → flush。
- `builds <= FRAMES` 在 0 次重建时也通过，无法证明每帧真的做了工作。
- 6 个 run 中只有选中项被绘制，6×200 对绘制成本约等于 1×200（这是合理的 UI 成本，但文档需说明）。

**修复**

- 改为 `request_subagent_cards_refresh` + `flush_subagent_cards`。
- `builds` 断言改为严格 `== FRAMES`。
- 文档说明消息为纯正文，6 个 run 用于并行 delta 状态压力，绘制只覆盖选中项。

---

## 2. 已核验无问题

- delta/tool/终态路径已无生产代码直接调用 `refresh_subagent_cards`；只由 `flush_subagent_cards` 与测试调用。
- `flush_subagent_cards` 在绘制门控内、所有 resolve/draw 之前执行；同帧最多一次。
- 多窗格 flush 遍历 `all_pane_ids`，隐藏 tab/工作区与嵌套 run 均会处理；`set_subagent_list` 的 session 不匹配早退未被破坏。
- `cards_dirty` 由 `Default` 初始化为 false；`clear_agents` 整体替换；窗格删除即丢弃。
- `resolve_task_agents_visible` 的测量矩形与 `draw_agents_detail` 的绘制矩形同源（`pane_inner_rect → layout.task_output`）。
- 离屏合并只忽略“末条 streaming 消息内容”的变化；通知、compact、模型切换、recap、上一消息变化都会使指纹失效。
- 贴底 `follow_bottom` 时禁用合并，立即重建。
- 列表 `put_clipped` 的 width 参数全部为列数；未发现把绝对 right 当 width 的遗漏。
- 筛选胶囊与 `[esc close]` 绘制/命中共用 `filter_row_rects`，间距恒 2 列；放不下时整组丢弃。
- 滚动时代码块命中、选区、滚动条使用同一份 `run.scroll` 口径。

---

## 3. 修复顺序与验证

1. **P0-1**：request 语义 + set_subagent_list 返回值 + sync 队列变化跟踪。
2. **P1-1**：snapshot freshness + resolve 只在 fresh 时清 dirty。
3. **P1-2**：严格降级，标题不回填。
4. **P2**：嵌套 dirty、resize 失效、snapshot owner、emoji 宽度、测试有效化、文档同步。
5. **P3**：gutter 边界、大小写、now_ms、viewport 语义、面板锚点、性能测试链路。

验证命令与结果见 [`verification.md`](verification.md)：

```bash
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
cargo test --release --lib -- --ignored perf_agents_panel
cargo test --release --lib -- --ignored perf_lx_history
```

---

## 4. 风险与未覆盖

- **已合并到 `dev`（`061e8bba`）**：与 `dev@95cf9482`（subagent mention 特性）合并时处理了 `src/event/chat/subagent/runner.rs`（`sync_agent_ui_queue` 保留 `target.lx.sync_mention_subagent_runs()` 并返回 `changed: bool`）与 `src/event/chat/subagent/settle.rs`（`finish_subagent_with_billing` 保留 `target.lx.sync_mention_subagent_runs()` 并调用 `update::request_subagent_cards_refresh`）两处冲突；`src/app/update/chat/subagent.rs`、`src/event/chat/subagent/mod.rs`、`src/event/mod.rs` 自动合并无误；合并后全量 `cargo test` **5063 passed, 5 ignored, 0 failed**。
- **TUI 交互未自动验证**：滚动是否丝滑、列对齐、窄窗降级仍需用户 `cargo run` 验收。
- **上翻时总行数最多滞后 80ms**：这是设计的合并窗口，不是错误；`rows_dirty` 现在会保留，窗口结束后自动追平。
- **列表列宽随时长位数/筛选集变化整体位移一次**：Q9 已接受；未做固定 age 列宽。
- **任务面板列表改为 500ms 轮询**：打开面板后新启动的进程/子 agent 最多延迟 500ms 出现在
  列表；首次打开仍立即请求。
- **`tree_prefix` 仍为 O(n²)**：当前并发规模小，未在本次修改；run 数很大时可再做一次 O(n) 预计算。
- **列表每帧对全部筛选集重新计算列宽/前缀**：在 P0 修复后只在真正绘制帧发生；若未来 run 数达到数百，可加列表指纹缓存。
