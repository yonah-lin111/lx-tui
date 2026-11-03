# `lx-streaming-perf` 代码审查（review.md）

> 审查范围：`feat/lx-streaming-perf` 分支相对 `dev` 的全部改动（提交 `062aed2b` + 本次 review 修复）。
> 审查方式：`git diff` 逐文件阅读 + `codegraph` / `codebase-memory` 调用链核对 + 定向回归测试。
> 结论：**无 P0**；发现 2 个 P1、2 个 P2、3 个 P3（P3 为可接受限制或文档漂移）。
> 本文档按优先级记录「关键点 / 错误原因 / 修复与验证」，修复完成后回填结果。

---

## P0（崩溃、数据损坏、可见内容错误）

**无。** 增量渲染与全量渲染在逐字节流式输入下逐行逐样式一致；二级缓存原地覆写不改变行数与命中语义；未发现 panic 路径（生产代码无 `unwrap()` / `expect()`）。

---

## P1-1：选区几何仍在鼠标回调里同步重建快照

### 关键点

- `selection::content_geom` 是滚轮与拖拽选区的事件路径：`event/mouse/wheel.rs:116`（拖选时滚轮）、`event/mouse/drag.rs:375`（拖选移动）、`event/mouse/press.rs:872`（按下开始选区）。
- 它内部走的是 `history_view` / `view_source_plain`（完整签名校验 + 按需重建快照），**没有**接入本次新增的 `snapshot_ready_source`。
- 因此：流式生成期间按住拖选或滚轮滚动时，每个鼠标事件仍可能同步跑一次 `HistorySnapshot` 构建 + 增量 Markdown 更新——正是本次优化要消除的卡顿源。

### 错误原因

就绪快照迁移只覆盖了 `history_rows`（滚轮）、`history_scrollbar`（滚动条）、`card_at_source`（点击/Hover）和 `task_agents_*`，**漏掉了共用的 `content_geom` 入口**；`content_geom` 同时被绘制路径（`lx.rs:112` 选区反显）复用，调用方分散在 `selection.rs`、`event/mouse/*`，迁移时没有按入口统一收口。

### 修复

- `history_hit::history_view_ready` 提升为 `pub(super)`，供 `selection.rs` 复用。
- `selection::content_geom` 主聊天分支改走 `history_view_ready`。
- Agents 详情分支优先使用 `resolve_task_agents_visible` 每帧维护的 `run.total_rows` / `run.scroll` / `run.follow`（与绘制同口径，零重排）；仅当 `rows_dirty` 或尚未量行时回退到完整视图，避免冷启动/测试直接构造时几何缺失。
- 回归测试：`selection_geometry_reads_ready_snapshot`（内容变化后选区几何不触发快照重建）。

### 验证

- `cargo test --lib selection_geometry_reads_ready_snapshot` 通过。
- 原事件回归 `event::tests::lx_ask_task::lx_task_output_and_agents_detail_selection_scroll_and_copy` 与 `task_output_and_agents_copy_text_across_scroll_without_scrollbar` 均通过。

---

## P1-2：性能回归基准被「降级」而不是修复

### 关键点

- 上一提交把 `perf_lx_history` 的行数断言从 `> 90_000` 改成 `> 60_000`，因为夹具实际只有 64,400 行。
- 该用例的验收口径是「1000 条消息量级、约 10 万渲染行、200×50 视口」，降低阈值后它不再覆盖设计文档承诺的压力规模，后续回归可能在大历史下悄悄超预算。

### 错误原因

用户消息默认折叠为 5 行 + 折叠提示（`user_message_expanded` 相关逻辑），`BODY_LINES = 70` 的夹具实际只产生 64,400 行；原断言在改动前的 `dev` 分支就已失效（已在 dev 复现同样数值）。上一提交选择了「让测试通过」而不是「恢复夹具规模」。

### 修复

- `BODY_LINES` 70 → 125，夹具恢复为 **102,900 行**，断言恢复 `> 90_000`。
- 实测仍满足预算：滚动 P95 ≈ **1.92ms**、流式追加 P95 ≈ **3.72ms**（预算 16ms）。

### 验证

```
cargo test --release --lib -- --ignored perf_lx_history --nocapture
```

---

## P2-1：屏幕外流式合并对子 agent 详情的数据源不安全

### 关键点

- `PaneCache::offscreen_stream_throttled` 用 `source.chat.history_scroll` 判断「流式消息是否在视口下方」。
- 主聊天符合该口径；但 Agents 详情区的滚动位置在 `SubagentRun::scroll`，`LxChat::history_scroll` 不是它的真实视口。
- 一旦详情区的 `chat.follow_bottom` 变为 `false`（当前实现恰好不会），合并逻辑会拿错误的视口做判断，可能在流式消息可见时延迟刷新，最长一个 80ms 窗口。

### 错误原因

`HistorySource` 没有表达「哪个滚动字段是权威的」；`from_chat` 与 `From<&Lx>` 共用同一套字段，合并判定默认按主聊天布局假设。

### 修复

- `HistorySource` 新增 `coalesce_offscreen: bool`：主聊天为 `true`，`from_chat`（Agents 详情）为 `false`。
- `offscreen_stream_throttled` 首先要求 `source.coalesce_offscreen`。

### 验证

- `offscreen_stream_coalesces_within_window` 通过（主聊天窗口内合并、非流式尾部变化立即重建）。
- 全量 `cargo test` 通过。

---

## P2-2：增量绘制的「前缀 / 尾部」交界缺少像素级回归

### 关键点

- `draw_markdown` 从 `BlockPaint::Markdown { prefix, tail }` 两段读取行：`offset < prefix.len()` 取前缀，否则取尾部。
- 既有测试只验证布局行数/性能，没有任何用例断言滚动到前缀与尾部交界处时实际画出的文本；字段错位、索引差一这类问题不会被发现。

### 错误原因

上一提交为绘制路径引入了新的双段读取，但测试只覆盖了「尾部行」（短正文）与「行数一致」，没有构造「前缀非空 + 视口跨交界」的场景。

### 修复

- 新增 `long_markdown_scroll_reads_prefix_and_tail`：80 行段落（全部固化进前缀）+ 末尾表格（留在尾部），滚动到第 78 行，断言屏幕行依次为 `line 78`、`line 79`、表格上边框与表格数据行。

### 验证

- `cargo test --lib long_markdown_scroll_reads_prefix_and_tail` 通过。

---

## P3-1：单行超长内容仍会每帧重排尾部（已接受限制）

- **关键点**：稳定边界只在 `\n` 处推进；若模型连续输出 30,000 字符而**没有换行**，每帧都要对整条逻辑行重跑 `inline_spans` + `wrap_md_line`，该极端形状仍是 O(N²)。
- **错误原因**：词优先软换行的断点依赖整条逻辑行，提交半行需要「按词回滚」，实现复杂度与收益不成比例。
- **处理**：记录为已知限制，不修改。正常模型输出按行/段落递增；本次实测的长思考与 1,500 行正文均已消除该问题。
- **后续建议**：若真实会话出现单行超长流式，可在 `IncrementalMarkdownState` 中增加「最后一个空白之后的文本不固化」的按词稳定边界。

## P3-2：文档与实现漂移（已修复）

- `design.md` §5.1 写的是 `pub(super) fn reasoning_body_visible`，实际为 `pub(crate)`；
- §8.1 写「在 `history.rs` 新增 `history_view_ready`」，实际 wrapper 在 `history_hit.rs`，核心入口是 `history::view_ready`；
- §8.2 未记录 `coalesce_offscreen` 数据源开关。
- **修复**：同步更新 `design.md` / `task.md`。

## P3-3：就绪视图每次克隆 `messages`（已接受）

- **关键点**：`view_source_inner` 为置顶行计算返回 `messages: Vec<MessageRows>`，`history_view_ready` 每次滚轮/拖拽事件都会克隆一遍（1400 条消息约几十 KB/事件）。
- **错误原因**：`HistoryView` 早期按值持有点击/置顶几何；迁移到就绪快照后没有一起改成共享存储。
- **处理**：当前量级不构成掉帧源（实测滚动 P95 1.92ms），不在本次扩大 API 改动；后续可改为 `Arc<[MessageRows]>` 或让 `history_rows`/`history_scrollbar` 只返回标量。

---

## 修复执行清单

| 优先级 | 问题 | 状态 |
|--------|------|------|
| P0 | 无 | — |
| P1-1 | 选区几何未走就绪快照 | ✅ 已修复 + 回归测试 |
| P1-2 | 性能基准规模被降级 | ✅ 已恢复 102,900 行并实测 |
| P2-1 | Agents 详情屏幕外合并口径错误 | ✅ 已加 `coalesce_offscreen` 开关 |
| P2-2 | 前缀/尾部交界无像素级测试 | ✅ 已新增渲染回归 |
| P3-1 | 单行超长内容 O(N²) | 记录限制，不修改 |
| P3-2 | 文档漂移 | ✅ 已更新 design/task |
| P3-3 | `messages` 每事件克隆 | 记录限制，不修改 |
