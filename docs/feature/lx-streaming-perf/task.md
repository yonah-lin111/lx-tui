# lx 长思考与流式输出卡顿优化任务清单（`lx-streaming-perf`）

> 状态：**已实施并验证（A/B/C/D 全部完成）**
>
> 对应设计文档：`docs/feature/lx-streaming-perf/design.md`

---

## 执行规则

1. **审核前置**：本清单与 `design.md` 经用户确认后方可执行代码改动。
2. **行数红线**：所有生产 `.rs` 文件严格 < 1000 行，新拆分子模块控制在 ~500 行左右。
3. **安全红线**：生产代码零 `unwrap()` / `expect()`；渲染层只读 `Lx` 状态，不做 IO。
4. **精确验证**：每阶段完成后使用 `rtk cargo test`（针对受影响模块）与 `rtk cargo clippy --all-targets -- -D warnings` 校验。

---

## A. 缓存底座拆分与同槽位原地覆写（`src/ui/main_content/lx/history/cache*`）

- [x] **A1. 拆分 `history/cache/signature.rs`（控制单文件 ~460 行）**
  - 将 `Fnv`、`list_signature_source`、`step_signature`、`part_block_signature`、`shell_block_signature`、`report_block_signature`、`notice_block_signature`、`stats_block_signature`、`rewind_block_signature`、`compact_block_signature`、`model_switch_block_signature` 从 `src/ui/main_content/lx/history/cache.rs` 拆入 `src/ui/main_content/lx/history/cache/signature.rs`。
  - 新增 `non_stream_signature_source(source, width)`：计算除正在流式的末条消息之外的列表结构指纹，供屏幕外流式节流判定使用。

- [x] **A2. 实现思考块（Thinking）Claude Code 模式签名与可见性判定**
  - 在 `cache/signature.rs` 实现 `reasoning_body_visible(source, message_index, step_index) -> bool`：当且仅当 `reasoning_expanded && !reasoning_streaming` 时返回 `true`。
  - 在 `step_signature` 中对 `PartKind::Reasoning` 实施流式隔离：
    - 当 `streaming == true` 时，不写入 `part.content.len()`、`part.content` 首尾字节、`started_at_ms`、`ended_at_ms`；
    - 当 `streaming == false` 且 `!body_visible`（折叠态）时，仅写入起止耗时时间戳（供 `Thought · 6.3s` 使用），不写入 `part.content`；
    - 仅当 `body_visible == true`（已完成且展开）时才写入 `part.content.len()` 与 `write_edges(&part.content)`。

- [x] **A3. 改造 `PaneCache.blocks` 为按物理槽位 `BlockSlot` 原地覆写**
  - 在 `history/cache.rs` 定义 `BlockSlot { message_index, step_index, width }`，`BlockEntry` 内保存 `sig: u64`。
  - `PaneCache::block(key, build)` 按 `key.slot()` 查询：若 `entry.sig == key.sig` 则直接命中；若 `sig` 变化则原地替换该槽位的 `BlockEntry`，同步扣减旧 `bytes` 并加上新 `bytes`，消除流式过程中同一块产生数百个 `BlockKey` 残影的问题。

---

## B. 块级增量 Markdown 渲染器（`src/ui/markdown_render/incremental.rs` & `paint.rs`）

- [x] **B1. 实现 `src/ui/markdown_render/incremental.rs`（约 240 行）**
  - 定义 `IncrementalMarkdownState`：维护 `width`、`committed_bytes`、`committed_hash`、`list_indents`、`open_fence_lang`、`stable_lines: Arc<Vec<MdLine>>`、`stable_bytes`。
  - 实现 `update(&mut self, source: &str, width: u16) -> (Arc<Vec<MdLine>>, usize, Vec<MdLine>)`：
    - 校验前缀哈希与宽度，若匹配则仅扫描 `&source[committed_bytes..]`；
    - 对已换行的段落、标题、分割线、图片、引用、列表项，以及未闭合围栏代码块（`open_fence_lang`）内部的已完成代码行，直接通过 `Arc::make_mut(&mut self.stable_lines)` 原地追加，更新 `committed_bytes` 与 `stable_bytes`；
    - 对末尾未闭合的表格（`|` 开头）与未闭合的缩进代码块（4 空格缩进）实施 Holdback，与最后一行未换行文本一起作为 `tail` 渲染返回。

- [x] **B2. 升级 `BlockPaint::Markdown` 与 `PaneCache::markdown_block`**
  - 在 `src/ui/main_content/lx/history/paint.rs` 中将 `BlockPaint::Markdown` 改为 `{ prefix: Arc<Vec<MdLine>>, prefix_bytes: usize, tail: Vec<MdLine> }`。
  - 更新 `BlockPaint::rows()`、`BlockPaint::bytes()` 与 `draw_markdown()`，按视口偏移无缝读取 `prefix` 与 `tail`。
  - 在 `PaneCache` 中维护 `md_streams: HashMap<BlockSlot, IncrementalMarkdownState>`；在 `snapshot_source` 重建前先释放旧快照引用，并在 `markdown_block` 更新前先取出旧槽位 `BlockEntry`，确保 `Arc::strong_count(&state.stable_lines) == 1`（零克隆追加）。
  - 在 `src/ui/main_content/lx/history/snapshot.rs` 的 `build_step` 中：
    - 助手 `PartKind::Text` 接入 `self.cache.markdown_block(...)`；
    - 助手 `PartKind::Reasoning` 接入 `reasoning_body_visible(...)`。

---

## C. 滚动条拖拽、滚轮与屏幕外流式解耦（`history.rs` & `history_hit.rs`）

- [x] **C1. 新增 `PaneCache::snapshot_ready_source` 与 `history::view_ready`**
  - 在 `history/cache.rs` 实现 `snapshot_ready_source(width, source)`：若当前已存在同宽度（`snapshot_width == Some(width)`）的就绪快照，直接返回 `Arc::clone(snapshot)`；仅在未初始化或宽度变化时回退调用 `snapshot_source`。
  - 在 `history.rs` 实现 `view_ready` / `view_source_ready`，在 `history_hit.rs` 中让 `history_rows`（滚轮）、`history_scrollbar`（滚动条渲染与拖拽）、`fold_target_at`（鼠标移动 Hover）使用就绪快照，彻底消除鼠标事件回调内的同步重排阻塞。

- [x] **C2. 实现滚离底部时的屏幕外流式快照节流**
  - 在 `PaneCache::snapshot_source` 中，当 `!source.chat.follow_bottom`、已有同宽度快照、`non_stream_signature_source` 未变（仅流式末条消息变化）、末条流式消息位于当前视口下方屏幕外、且距上次构建不足 `80ms` 时，直接复用已有 `Arc<HistorySnapshot>`。

---

## D. 单元测试、等价性测试与 Release 性能基准验证

- [x] **D1. 增量 Markdown 逐行等价性单元测试（`src/ui/markdown_render/tests.rs`）**
  - 覆盖标题、段落、行内标记、嵌套列表、任务列表、引用、围栏代码块、缩进代码块、表格、水平线；验证任意步长流式切片下的 `IncrementalMarkdownState` 输出与全量 `render_wrapped` 100% 一致。

- [x] **D2. 思考块流式隔离与同槽位缓存单测（`src/ui/main_content/lx/history/cache/tests.rs` & `history/tests/`）**
  - 新增 `streaming_thinking_does_not_rebuild_snapshot_or_render_body`：验证思考流式期间追加内容与更新时间戳不触发快照重建，展开标记在流式完成后生效。
  - 新增 `streaming_text_replaces_slot_in_place`：验证流式正文追加 100 次后 `PaneCache.blocks` 仅保留 1 个槽位。
  - 更新 `history/tests/transcript.rs` 中已完成思考块的 `completed_at_ms` 测试夹具。

- [x] **D3. 新增流式长思考与长正文性能回归测试（`src/ui/main_content/lx/history/tests/perf.rs`）**
  - 新增 `#[ignore]` 测试 `perf_lx_long_thinking_and_streaming`：
    - 模拟 30,000 字符长思考（500 帧流式 delta）+ 1,500 行 Markdown/代码长正文（300 帧流式 delta + 滚动条位置变化）；
    - 验证思考流式期间快照重建次数为 0、单帧 P95 ≤ 2ms；长正文流式 + 滚动单帧 P95 ≤ 8ms。

- [x] **D4. 代码规范与性能实测收尾**
  - 运行 `rtk cargo fmt --check`、`rtk cargo clippy --all-targets -- -D warnings`、`rtk cargo test --lib history`、`rtk cargo test --lib markdown_render`；
  - 运行 `rtk cargo test --release --lib -- --ignored perf_lx_` 记录实测性能数据并回填至本文档末尾。

---

## 性能记录（2026-11 实测）

| 场景 | 规模 | 优化前基线 | 优化后结果 | 预算目标 |
|------|------|------------|------------|----------|
| 长思考流式（折叠/点击展开） | 30,000 字符 / 500 帧 | 每帧重建快照（展开态 O(N²) 卡死） | P95 0.21ms，重建 0 次 | 重建 0 次，P95 ≤ 2ms |
| 长正文 Markdown + 代码块流式 + 滚动 | 1,500 行 / 300 帧 | 每帧全量重排全文 + 堆积 300 份缓存 | P95 0.19ms，单槽位 | 槽位 = 1，P95 ≤ 8ms |
| 1400 条消息热缓存滚动（既有回归） | 64,400 行 / 120 帧 | P95 ~1.95ms | P95 2.00ms（流式追加 3.89ms） | P95 ≤ 16ms |
| 展开 2000 行文件差异滚动 | 2,106 行 / 120 帧 | P95 ~6.4ms | P95 6.61ms | P95 ≤ 16ms |
