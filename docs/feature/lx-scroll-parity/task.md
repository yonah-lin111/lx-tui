# lx 消息列表滚动对齐 opencode：任务清单

> 状态：**已实现（`ed76d856`）**；P0~P4 完成，P5 未触发；待用户实机验收 / 待合并。
> 审查记录与修复状态见同目录 [`review.md`](review.md)。
>
> 设计依据见同目录 `design.md`。所有"现状"行号以 2026-11-11 的 `dev` 工作区为基线；
> 实现细节已按 HEAD 同步。

## 执行约束

- 不新增 `sidebar` / `prompt_frame` / `panes` 等顶层模块；改动落在现有
  `app/`（状态与更新）、`ui/main_content/lx/`（纯渲染）、`event/`（输入）边界内。
- 生产代码禁止 `unwrap()` / `expect()`；错误用 `Result` 与 `?` 传播。
- 不在渲染路径写状态；锚点状态只由事件循环的 `resolve_*` 写入。
- 日志只用 `tracing`；禁止 `println!` / `eprintln!`。
- 单文件生产代码 ≤1000 行；测试跟随被测模块目录，单测试文件 ≤1500 行。
- 本任务无数据库 schema 变更；不触碰 Git 状态。

## P0：构建配置（先拿到可用基线）

### 目标

让 `cargo run`（debug）与 `cargo test`（dev profile）不再把热路径放大 2~4 倍，
为后续优化提供真实可感知的基线。

### 文件

- `Cargo.toml`

### 步骤

1. 在 `[profile.dev.package.*]` 之前新增：

   ```toml
   [profile.dev]
   opt-level = 1
   ```

2. 确认没有显式覆盖 `[profile.test]` 的 `opt-level`（当前没有；test 继承 dev）。
3. 不改 release profile。

### 验收

```bash
cargo test --lib -- --ignored perf_lx_history --nocapture
```

预期（与已验证的环境变量运行一致）：

| 项 | 目标 |
|---|---|
| `perf_lx_history` cold | ≤ 500ms（实测 355ms） |
| 滚动 P95 | ≤ 4ms（实测 1.83ms） |
| 流式 P95 | ≤ 8ms（实测 2.89ms） |
| `perf_lx_history_expanded_diff` P95 | ≤ 12ms（实测 8.23ms） |

> 冷编译约 3 分钟属预期；如果团队无法接受，回滚只需删除这 3 行并记录原因。
> 表中数字是**观测目标**；自动回归硬门禁仍是 16ms（见 `design.md` §7.1）。
> `[profile.dev] opt-level=1` 作用于整个 dev 依赖图，不只本 crate。

## P1：单帧单指纹 + 去掉整表克隆

### 目标

把每帧 3 次全量列表指纹降为 1 次（屏幕外合并判定帧为 full + non-stream
各一次，属不同指纹）；去掉每次 view 的 `Vec<MessageRows>` 克隆；
去掉 MCP 指纹的 `join` 分配；置顶行查找改二分。

### 文件

- `src/event/mod.rs`
- `src/ui/main_content/lx/history/cache.rs`
- `src/ui/main_content/lx/history/cache/tests.rs`
- `src/ui/main_content/lx/history/cache/tool_signature.rs`
- `src/ui/main_content/lx/history/snapshot.rs`
- `src/ui/main_content/lx/history.rs`
- `src/ui/main_content/lx/history/sticky.rs`

### 步骤 1：指纹 epoch

1. `RenderCache` 增加字段：

   ```rust
   pub(crate) struct RenderCache {
       frame: u64,
       sig_epoch: u64,
       sig_memo_enabled: bool,
       panes: HashMap<PaneId, PaneCache>,
   }
   ```

2. 新增：

   ```rust
   /// 绘制前调用（早于 resolve_lx_history）：声明本轮输入已固定。
   pub(crate) fn prepare_frame(&mut self) {
       self.sig_epoch = self.sig_epoch.wrapping_add(1);
       self.sig_memo_enabled = true;
   }
   ```

3. `RenderCache::pane()` 把 `sig_epoch` / `sig_memo_enabled` 下发到 `PaneCache`
   （`PaneCache` 新增同名字段，默认 `0` / `false`）。
4. `PaneCache` 增加：

   ```rust
   struct SignatureMemo {
       epoch: u64,
       width: u16,
       owner: usize,
       full: Option<u64>,
       non_stream: Option<u64>,
   }

   fn signature_for(&mut self, width: u16, source: HistorySource<'_>) -> u64 {
       if !self.sig_memo_enabled {
           return list_signature_source(source, width);
       }
       // epoch / width / owner 任一不匹配就换新 memo
       // full 槽命中直接返回；否则计算一次并写入。
   }

   fn non_stream_signature_for(&mut self, width: u16, source: HistorySource<'_>) -> u64 {
       // 与 signature_for 同一 memo，non_stream 槽；未开启 epoch 时直接计算。
   }
   ```

5. `snapshot_source_fresh` 中 `list_signature_source` 替换为
   `self.signature_for(width, source)`；`offscreen_stream_throttled` 中的
   `non_stream_signature_source` 替换为 `self.non_stream_signature_for(width, source)`。
   两个槽共用一条 memo，同一 epoch 内各自只算一次。

   **借用顺序**：`offscreen_stream_throttled` 改为 `&mut self`，在完成
   `follow_bottom` / 视口行数 / 时间窗等早退判断**之后**才调用
   `self.non_stream_signature_for(width, source)`；随后再借用 `self.snapshot` 读
   末条消息。不要把 non_stream 计算提前到函数入口（贴底流式帧会白算一次全量）。
   快照构建完成后写 `self.snapshot_non_stream` 时，同一输入代会命中 memo，不重复计算。
6. `snapshot_non_stream` 改为 `Option<u64>`：构建时 `follow_bottom == true`
   （或 `coalesce_offscreen == false`）存 `None`，不计算；`offscreen_stream_throttled`
   遇到 `None` 直接返回 false，强制先正常重建一次并补基线。这样贴底流式每帧
   只算 1 次全量指纹（实测 1400 条夹具 stream P95 从 4.0ms 降到 3.0ms）。
7. `src/event/mod.rs:519-535` 绘制块：在 `flush_subagent_cards` 之前插入
   `ui_cache.prepare_frame();`，`begin_frame()` 位置不动；`tui.draw` 之后调用
   `ui_cache.finish_frame()` 再 `drawn?`，保证 draw 之后的鼠标事件现算指纹。
8. 不变量写进注释：首次读取指纹之后不得再修改进入指纹的字段；`finish_frame`
   关闭记忆。`RenderCache::finish_frame` 清空各窗格 memo 并置 `sig_memo_enabled=false`。

### 步骤 2：`messages` 改 `Arc`

1. `snapshot.rs`：

   ```rust
   pub(crate) struct HistorySnapshot {
       pub(crate) entries: Vec<SnapEntry>,
       pub(crate) messages: Arc<Vec<MessageRows>>,
       // ...
   }
   ```

   `build` 初始化：`messages: Arc::new(vec![MessageRows { .. }; source.messages.len()])`；
   写入处用 `Arc::make_mut(&mut builder.snapshot.messages)[index] = ...`。
2. `history.rs`：

   ```rust
   pub struct HistoryView {
       // ...
       pub messages: Arc<Vec<MessageRows>>,
   }
   ```

   `view_source_inner` 的 `snapshot.messages.clone()` 现在是 O(1)。
3. 所有读取点保持 `view.messages[...]` / `&view.messages` 不变（`Arc` 有 `Deref`）。

### 步骤 3：去掉 MCP `join`

`cache/tool_signature.rs:63-75`：

```rust
fn hash_lines(hash: &mut Fnv, lines: &[String]) {
    hash.write_usize(lines.len());
    for line in lines {
        hash.write_str(line);
    }
}

// MCP 分支
hash_lines(&mut hash, &view.arguments);
hash_lines(&mut hash, &view.result);
```

> 指纹值会变化一次（缓存首次失效），不影响正确性；不要改动 `Fnv` 的写入顺序之外
> 的语义。

### 步骤 4：置顶行二分

`history/sticky.rs:30-35` 的 `message_at` 改为：

```rust
pub(super) fn message_at(rows: &[MessageRows], scroll: usize) -> Option<(usize, usize)> {
    let index = rows.partition_point(|range| range.start <= scroll);
    let index = index.checked_sub(1)?;
    let range = rows.get(index)?;
    (scroll < range.end).then_some((index, scroll - range.start))
}
```

注意：生产里被时间线跳过的 legacy wait 行最初是 `(0,0)`，会让 `messages.start`
不单调。`snapshot::build` 收尾必须把占位规范化为前一条消息的 `end`
（见 `snapshot.rs`），否则 `partition_point` 会把占位当成「最后一条 `start <= scroll`」
并返回 None。规范化后 `start == end` 的零行占位自然被 `scroll < range.end` 排除。

### 测试

- `cache/tests.rs` 新增：
  - `signature_memo_reuses_within_epoch`：同一 epoch 连续两次 `snapshot_source`
    只 +1 次 `signature_evaluations`、不重建；新 epoch + 状态变化才重建；
  - `signature_memo_disabled_by_default`：不开启 epoch 时保持现有
    "每次改内容都重建"语义（现有测试应全部保持通过）。
- `history/tests/sticky.rs` 新增：
  - `message_at_skips_zero_row_placeholders`；
  - `snapshot_normalizes_legacy_placeholders_for_binary_search`（真实 `(0,0)`
    占位、中部 + 连续/末尾）。
- 现有 `cache/tests.rs` 的 `builds` 断言不得修改（legacy 占位起始行的断言随
  规范化更新为 `start == prev.end`）。

### 验收

```bash
cargo test --lib ui::main_content::lx::history
cargo test --lib app::update::chat
```

## P2：块级阅读锚点（核心）

### 目标

- 用户上滚阅读后，历史上方内容变化（工具输出落地、展开/折叠、汇报追加、
  compact 卡片插入）不移动阅读位置。
- 贴底跟随时行为不变；回到底部恢复跟随。
- 锚点块/消息消失时按设计回退，不 panic、不越界。

### 文件

- `src/ui/main_content/lx/history/anchor.rs`（新增：`HistoryAnchor` / `AnchorEntry` / `anchor_at` / `anchor_row`）
- `src/ui/main_content/lx/history/cache.rs`（`snapshot_cached` / owner / epoch）
- `src/ui/main_content/lx/history/snapshot.rs`（`SnapEntry.id`、占位规范化、`BlockSource`）
- `src/ui/main_content/lx/history_hit.rs`（主历史锚点入口、ready 命中）
- `src/ui/main_content/lx/history.rs`（`HistoryView.messages: Arc`、置顶二分）
- `src/app/update/chat/viewport.rs`（主历史/详情捕获 + 补偿）
- `src/app/update/chat/subagent.rs`（详情卡片 toggle pin）
- `src/ui/main_content/lx/task/agents.rs`（详情锚点/测量）
- `src/app/update/chat/tests/viewport_anchor.rs`
- `src/ui/main_content/lx/history/tests/anchor.rs`（注册到 `history/tests.rs`）
- `src/ui/main_content/lx/task/tests/agent_list.rs`（详情锚点/pin）

### 步骤 1：锚点模块（不落状态）

新增 `src/ui/main_content/lx/history/anchor.rs`：

```rust
pub(crate) struct HistoryAnchor {
    pub(crate) entry: AnchorEntry, // Message{index,id} / Notices / Compact / ModelSwitch / Raw
    pub(crate) block: usize,       // 块序号回退
    source: BlockSource,           // 块身份：优先按它找块
    pub(crate) offset: usize,
    pub(crate) row: usize,
    pub(crate) width: u16,
}
```

- `anchor_at(snapshot, scroll, width)`：在旧快照上二分条目/块，映射稳定 ID；
  空行取下一块，末尾呼吸行取最后一块。
- `anchor_row(snapshot, source, anchor, width)`：宽度不符 → None；按 ID 找条目；
  先按 `BlockSource` 找块，再退回序号；正文块消失且是消息条目 → `entry.start`，
  Stats/Recap/Attachments/Notice 等尾部块 → `entry.end - 1`，否则 None。
- `SnapEntry` 新增 `id: Option<String>`：在 `snapshot::build` 时从**同一份数据源**
  取 ID。锚点捕获在旧快照上做，若用新数据源按旧下标取 ID，子 agent 恢复整体替换
  消息列表时会锚到另一条消息。
- `LxChat` / `SubagentRun` 不新增锚点字段：锚点是每帧局部值。

### 步骤 2：旧快照读取入口

`src/ui/main_content/lx/history/cache.rs` 新增：

```rust
pub(crate) fn snapshot_cached(
    &self,
    width: u16,
    source: HistorySource<'_>,
) -> Option<Arc<HistorySnapshot>> {
    if self.snapshot_owner != Some(source.owner()) || self.snapshot_width != Some(width) {
        return None;
    }
    self.snapshot.as_ref().map(|(_, snapshot)| Arc::clone(snapshot))
}
```

它**只读不建**：本帧状态已变，但缓存里仍是上一帧几何；宽度/数据源失配直接返回
None（resize、切页不做补偿）。

`history_hit.rs` 增加两个入口（主历史），`task/agents.rs` 增加详情区版本：

```rust
pub(crate) fn history_anchor_before(
    rect: Rect, lx: &Lx, cache: &mut PaneCache, scroll: usize,
) -> Option<HistoryAnchor>;                 // snapshot_cached + anchor_at

pub(crate) fn history_anchor_row(
    rect: Rect, lx: &Lx, cache: &mut PaneCache, anchor: &HistoryAnchor,
) -> Option<usize>;                         // snapshot_ready_source + anchor_row
```

### 步骤 3：主历史补偿

`resolve_lx_history`（`viewport.rs:113` 起）：

```rust
let rect = history_rect(inner, lx)?;
let base_scroll = lx.chat.history_scroll;
// 1) 重建前：从上一帧快照捕获当前顶部行所在块
let anchor = if !lx.chat.follow_bottom {
    history_anchor_before(rect, lx, cache.pane(pane), base_scroll)
} else {
    None
};
// 2) 正常路径：可能重建快照
let mut view = history_view(inner, lx, cache.pane(pane))?;
// 3) 重建后：同一块的新行 vs 旧行，差值加回顶部行
if let Some(new_row) = anchor.as_ref().and_then(|a| history_anchor_row(rect, lx, cache.pane(pane), a)) {
    let delta = new_row as isize - anchor.row as isize;
    if delta != 0 {
        let max_top = view.total_rows.saturating_sub(usize::from(view.body.height));
        // 用捕获锚点时的 base_scroll；view.scroll 可能已被新几何钳制。
        let next = (base_scroll as isize + delta).clamp(0, max_top as isize) as usize;
        if next != base_scroll {
            lx.chat.history_scroll = next;
            view = history_view(inner, lx, cache.pane(pane))?; // 置顶行让位可能变
        }
    }
}
```

注意：

- 补偿写入 `history_scroll`，**不要**改 `follow_bottom`；
- 重算 view 只允许一次（避免抖动）；
- `viewport_top` 在最后写入，且以最终 `view.scroll` 为准；非跟随态还要把
  `history_scroll` 同步钳到 `view.scroll`（锚点缺失/条目删除时不能留越界旧值）；
- 宽度变化由 `snapshot_cached` 失配自然跳过，不用额外复位；
- 点击展开/折叠前若走了 `pin_lx_viewport`，它已把 `follow_bottom=false` 且
  `history_scroll=viewport_top`，下一轮 resolve 从旧快照捕获的正是展开前的块。
- 事件路径的 `chat_hit` / `sticky_prompt_jump` / 详情选区回退改用
  `history_view_ready` / `view_source_plain_ready`：命中只认屏幕上已画出的几何，
  也避免 `finish_frame` 之后事件路径重建快照、吞掉下一帧的锚点。

### 步骤 4：子 agent 详情补偿

`resolve_task_agents_visible`（`viewport.rs:211` 起）：

1. `needs_measure = run.rows_dirty || run.total_rows == 0`；为 true 且
   `!run.follow` 时，用 `task_agents_anchor_before(inner, &lx, cache, run.scroll)`
   从旧快照捕获锚点；
2. `task_agents_rows` 重建后，用 `task_agents_anchor_row(...)` 求新行；
3. 在 `should_follow == false` 时把 `delta` 加回 `next_scroll`（同样的 clamp）；
4. `should_follow` 只由 `run.follow || total <= visible+1` 决定，不再用
   `run.scroll >= max` 短路；用户滚轮/滚动条到底时才把 `run.follow` 置 true。
5. 卡片 toggle 后若 `run.follow == true`：按上一帧 `total_rows - visible` 钉住
   `run.scroll` 并置 `follow=false`（对齐 `pin_lx_viewport`），下一轮再补偿。
6. 不改 `run.follow` 的补偿结论；快照没重建时不做锚点工作。

### 步骤 5：边界与复位

- 宽度变化：`snapshot_cached` 失配 → 不补偿，沿用现有 resize 钳制。
- 消息/块被撤销/压缩删除：正文块回退消息起始行；Stats/Recap/Attachments/Notice
  等尾部块回退消息末尾；`anchor_row` 返回 None 时保持行号并钳到 `max_top`。
- 切换会话：状态整体替换，局部锚点天然不存在；owner 用 session id/agent id
  稳定身份，不依赖 `LxChat` 地址。
- 回到 `max_top`：`scroll_lx_history` / `set_lx_history_scroll` 置
  `follow_bottom = true`，下一轮不捕获锚点。

### 测试

`history/tests/anchor.rs`（新增，6 个）：

1. `anchor_block_stays_after_above_growth`：块内非零 offset，上方用户消息变高后锚点行同步下移。
2. `anchor_falls_back_to_message_start_when_block_gone`：正文块序号越界 → `entry.start`。
3. `tail_block_disappearance_falls_back_near_message_end`：Stats 消失 → 消息末尾附近。
4. `anchor_returns_none_when_message_gone`：删除锚点消息 → `None`，行号钳制。
5. `anchor_dropped_on_width_change`：宽度变化 → `None`。
6. `anchor_at_clamps_beyond_last_block`：末尾呼吸行锚到最后一块。

`history/tests/sticky.rs`：

6. `message_at_skips_zero_row_placeholders`：零行占位不被当成顶行所属消息。
7. `snapshot_normalizes_legacy_placeholders_for_binary_search`：真实 `(0,0)`
   legacy 占位（中部 + 连续/末尾）被规范化为前一条消息的 `end`，二分仍命中真实消息。

`app/update/chat/tests/viewport_anchor.rs`：

7. `reading_compensates_when_content_above_grows`：阅读态上方 +5 行 →
   `history_scroll` +5、`follow_bottom` 保持 false、`viewport_top` 同口径。
8. `following_ignores_above_growth`：贴底跟随时上方变高 → 继续贴底。

`task/tests/agent_list.rs`：

9. `agents_detail_anchor_compensates_when_above_grows`：详情区上方 +5 行 →
   `run.scroll` +5，`run.follow` 保持 false。
10. `toggling_detail_card_while_following_pins_viewport`：贴底展开卡片先钉住
    `total-visible` 并退出跟随，补偿后顶部行不动。

`cache/tests.rs`：

11. `signature_memo_reuses_within_epoch`：同一 epoch 多次 view 只算一次指纹、不重建；
    新 epoch 状态变化才重建。
12. `finish_frame_disables_signature_memo`：draw 之后改状态必须重建（事件路径不复用）。
13. `offscreen_coalescing_builds_baseline_after_scrolling_away`：贴底构建无基线 →
    滚离后首次增量重建一次补基线 → 之后才进入合并窗口。
14. `signature_memo_disabled_by_default`：测试默认保持原语义。

### 验收

```bash
cargo test --lib app::update::chat
cargo test --lib ui::main_content::lx
cargo test --lib ui::main_content::lx::task
```

人工：`cargo run` 按 `design.md` §7.2 的 5 步操作。

## P3：缓存淘汰单次扫描

### 文件

- `src/ui/main_content/lx/history/cache.rs`

### 步骤

1. 按 `design.md` §4.6 替换 `evict`。
2. 保留 `reclaim` 调用点（`render.rs:66-68`）与"本帧画过的块不淘汰"语义。
3. 保留 `insert_block` 中的 `evict()`（构建期也要守预算）。

### 测试

- `cache/tests.rs` 现有上限测试全部保持通过：
  `steady_state_reclaim_enforces_the_limit`、`render_reclaims_after_marking_used_blocks`、
  `visible_blocks_survive_eviction`、`eviction_keeps_rows_identical_after_rebuild`
  已覆盖「本帧使用不淘汰」「按 `last_frame` 升序」「预算内零成本」。

### 验收

```bash
cargo test --lib ui::main_content::lx::history::cache
cargo test --release --lib -- --ignored perf_lx_ --test-threads=1
```

## P4：验收夹具与性能断言

### 文件

- `src/ui/main_content/lx/history/tests/perf.rs`
- `src/ui/main_content/lx/history/tests.rs`（夹具辅助）

### 步骤

1. 新增 `realistic_history()` 夹具：
   - 300 条消息（用户/助手交替）；
   - 每条助手 4 次工具调用（read + exec_command 交替），150 条助手共 600 次；
   - 其中第一条助手命令的输出 10,000 行（实测 569,999 字节）；
   - 实测渲染行数 31,800（断言 ≥ 30,000）。
2. 新增 `#[ignore]` 用例 `perf_lx_history_realistic`：
   - 缓存层每帧 `view×2 + render`（与事件循环的 resolve/build/draw 取数同源）；
     不含 `resolve_lx_history` 的锚点扫描，锚点开销由滚动单测/人工验收覆盖；
   - 120 帧滚动 + 120 帧流式追加；夹具末条 `status="streaming"`；
   - 用真实 `RenderCache::{prepare_frame, pane, begin_frame, finish_frame}` 驱动；
   - 滚动帧断言 `builds` 不增长，`evals` 精确 +120；
   - 流式帧断言 `builds` 精确 +120、`evals` 精确 +120（贴底场景）；
   - 滚动/流式采样额外断言 max ≤50ms；
   - P95 ≤ 16ms（release 与 debug 各跑）；
   - 末尾断言 `cache.bytes() <= MAX_CACHED_BYTES`。
3. 保留现有 `perf_lx_history` / `perf_lx_history_expanded_diff` /
   `perf_lx_long_thinking_and_streaming`：不改预算，不删断言；三者同样加 max 断言。
4. 慢帧观测未实施：人工验收为准，避免生产日志噪音；输入到画面 <50ms 未自动化。

### 验收

```bash
cargo test --release --lib -- --ignored perf_lx_ --test-threads=1
cargo test --lib -- --ignored perf_lx_ --test-threads=1
```

记录输出到本文末尾"性能记录"。`perf_lx_` 同时覆盖
`perf_lx_history` / `perf_lx_history_realistic` / `perf_lx_history_expanded_diff` /
`perf_lx_long_thinking_and_streaming`；`--test-threads=1` 避免重型 perf 互相争 CPU。

## P5：工具卡绘制缓存（条件项）

> **未触发，不实施**（2026-11-11 实测 debug `perf_lx_history_expanded_diff`
> P95 = 8.65ms < 10ms；真实会话未出现可见工具卡滚动单帧 >16ms）。
> 只有满足以下任一条件才启动：
> - debug `perf_lx_history_expanded_diff` P95 > 10ms；
> - 真实会话出现可见工具卡滚动单帧 > 16ms。

若触发：

### 文件

- `src/ui/main_content/lx/history/paint.rs`
- `src/ui/main_content/lx/history/paint/legacy.rs`
- `src/ui/main_content/lx/history/snapshot.rs`
- 对应工具卡模块（`tool/command.rs`、`tool/file_diff.rs` 等）

### 约束

1. 只缓存**已完成且不在动画中**的卡片绘制行；`loading_frame` 变化的动态头部
   仍实时生成。
2. 缓存键包含 `BlockSlot + sig + width + folds`；折叠/展开、宽度变化必须失效。
3. 不改点击面几何、不改行数来源、不改动画语义。
4. 先做 `exec_command` + `file_diff` 两类，其余类型保持现状；性能收益以
   `perf_lx_history_expanded_diff` 复测为准。

### 验收

- 同一夹具 debug P95 ≤ 8ms 或相对 P4 基线下降 ≥ 30%；
- 折叠/展开/流式卡片视觉与交互回归通过。

## 全量验证命令

```bash
cargo fmt
cargo clippy --all-targets -- -D warnings
cargo test

# 性能（必须先做完 P0；单线程）
cargo test --release --lib -- --ignored perf_lx_ --test-threads=1
cargo test --lib -- --ignored perf_lx_ --test-threads=1
```

## 人工验收（用户执行）

```bash
cargo run
```

1. 长会话连续滚轮上下滚动 → 无明显掉帧、无"追不上手速"；
2. 流式中滚到中段 → 新内容不顶走阅读位置；
3. 滚回底部 → 恢复跟随；
4. 展开上方工具卡 → 正在阅读的段落不跳；
5. 子 agent 详情重复 2~4；
6. 点击"回到底部" → 立即贴底并恢复跟随。

## 性能记录（执行后填写）

| 日期 | 构建 | 夹具 | 冷构建 | 滚动 P95 | 流式 P95 | expanded diff P95 | 备注 |
|---|---|---|---|---|---|---|---|
| 2026-11-11 | release | 1400 / 102.9k | 250ms | 3.1ms | 4.0ms | 8.6ms | 基线 |
| 2026-11-11 | debug | 1400 / 102.9k | 1.94s | 8.3ms | 17.6ms | 37.5ms | 基线 |
| 2026-11-11 | dev (opt1) | 1400 / 102.9k | 383ms | 2.01ms | 3.02ms | 8.65ms | P1~P3 后 |
| 2026-11-11 | dev (opt1) | 300 / 31.8k | 240ms | 0.38ms | 0.73ms | — | P4 验收夹具，evals=241 |
| 2026-11-11 | release | 300 / 31.8k | 164ms | 0.34ms | 0.68ms | — | P4 验收夹具，evals=241 |
| 2026-11-11 | release | 1400 / 102.9k | 258ms | 1.96ms | 2.93ms | 8.07ms | 最终回归 |
| 2026-11-12 | dev (opt1) | 300 / 31.8k | 222ms | 0.37ms | 0.72ms | — | review 修复后，builds=121/evals=241 |
| 2026-11-12 | release | 1400 / 102.9k | 230ms | 1.92ms | 2.83ms | 6.82ms | review 修复后，单线程 |

## 完成定义（DoD）

- [x] P0 合入（本分支），两类性能命令达到表中目标；
- [x] 指纹：贴底/末条可见流式帧 1 次全量；屏幕外合并判定帧 full + non-stream；
- [x] `HistoryView` 不再克隆消息行区间（`Arc`）；
- [x] 主历史与子 agent 详情具备块级锚点，边界/尾部块/owner 测试通过；
- [x] 贴底/离底/回底语义有测试钉住；详情与主历史一致，不因 clamp 自动跟随；
- [x] 现有几何、滚动条、置顶行、thinking 命中、选区测试全部通过；
- [x] `cargo fmt` / `clippy -D warnings` / `cargo test` 全绿；
- [ ] 人工验收 6 步由用户确认（需 `cargo run`）；
- [x] 性能记录表填写完整。
