# lx 消息列表滚动对齐 opencode：设计

> 状态：**已合并到 `dev`（`fc166ad7`）**；P0~P4 完成，P5 未触发；人工实机验收待用户确认。
> 审查记录与修复状态见同目录 [`review.md`](review.md)。
>
> 参考实现：`/Users/yonah/projects/agent/opencode-dev/packages/tui`（以及依赖
> `@opentui/core@0.8.1` 的 `ScrollBoxRenderable`）。
>
> 本文“现状”行号以 2026-11-11 的 `dev` 工作区为基线；实现细节已按 HEAD 同步。

## 0. 已确认的决策

| # | 决策 | 结论 |
|---|------|------|
| 1 | "卡"的表现 | 长会话滚动 + 流式输出时滚动（A+B） |
| 2 | 验收夹具 | 300 条消息 / 600 次工具调用 / ≥3 万渲染行，末条处于流式中；连续滚轮 20 格，单帧 P95 < 16ms、无 > 50ms 长任务、输入到画面 < 50ms |
| 3 | 对齐边界 | 行为语义 + 成本模型对齐；**保留精确绝对行号**，不引入估算总行数 |
| 4 | 锚点粒度 | **块级**：`(message_id, 块序号, 块内行偏移)`；正文块消失回退消息起始行、尾部块回退消息末尾，消息消失保持行号并钳制 |
| 5 | 构建口径 | 验收包含 `cargo run`（debug）：`[profile.dev] opt-level = 1`，debug P95 也须 ≤16ms |
| 6 | 增量重建 | **不做**消息级脏后缀增量；用 `opt-level=1` + 单帧单指纹 + 去 clone + 滚轮路径 O(1) 达成 |
| 7 | 作用域 | 主聊天历史 + 子 agent 详情（共用 `HistorySource`/`PaneCache`）；任务输出、ask、todo、侧栏列表不动 |
| 8 | 滚轮步长 | 保持统一 `WHEEL_LINES = 3`；不新增加速/速度配置（opencode 默认加速关闭） |

## 1. 实测基线（关键事实）

同一套仓库内性能夹具（1400 条消息 / 102,900 渲染行 / 200×50 视口）：

| 场景 | release | debug（现状） | debug + `opt-level=1` |
|---|---|---|---|
| 冷启动全量构建 | 250ms | 1.94s | **355ms** |
| 热缓存滚动 P95 | 3.1ms | 8.3ms | **1.83ms** |
| 流式追加 P95 | 4.0ms | 17.6ms（超预算） | **2.89ms** |
| 展开 2000 行 diff 滚动 P95 | 8.6ms | 37.5ms（超预算） | **8.23ms** |

命令：

```bash
# release
cargo test --release --lib -- --ignored perf_lx_history --nocapture
# debug（当前默认）
cargo test --lib -- --ignored perf_lx_history --nocapture
# debug + opt-level=1（验证用环境变量，最终改为 Cargo.toml）
CARGO_PROFILE_DEV_OPT_LEVEL=1 cargo test --lib -- --ignored perf_lx_history --nocapture
```

用户实际运行的是 `target/debug/lx-tui`（`ps` 已确认），所以"很卡"的第一层原因是
构建配置，而不是滚动算法本身。开发库最大会话只有 11 条消息 / 78 次工具调用 /
约 500KB 工具结果，真实数据量远小于性能夹具；因此本任务**不做**为 1400 条规模
服务的脏后缀增量（决策 6）。

## 2. opencode 的真实滚动实现（核对结论）

`packages/tui` 的 32 处 `<scrollbox>` 全部使用 `@opentui/core` 的
`ScrollBoxRenderable`，但配置并不统一：

- 只有 13 处显式传 `getScrollAcceleration(config)`（默认 `CustomSpeedScroll(3)`，
  即每格 3 行）；其余（子 agent footer、shell 输出、permission、表单、composer tabs、
  stats 等）用 opentui 默认 `LinearScrollAccel`（1 行/格）。
- `stickyScroll` 只出现在 3 处：会话消息列表、子 agent footer、shell 输出弹窗。
- 尾部挂载（40 行起步、每次回填 60 行）、`pinAbove`/`scroll.anchor(node)`、
  `revealOlderRows`/`revealNewerRows` **只在会话消息列表**。
- **没有平滑滚动动画**。opentui 的滚轮逻辑是
  `remainder += sign * event.scroll.delta * accel.tick()`、`step = trunc(remainder)`
  后立即 `scrollTo(position + step)`；加速默认关闭（`MacOSScrollAccel` 仅在配置
  `scroll.acceleration = true` 时启用）。
- 消息列表的"不卡"来自成本模型：只挂载窗口内的行 + 原生贴底跟随 + 布局后锚点，
  而不是动画。

因此 lx 需要对齐的是：

1. 贴底跟随 / 上滚离底 / 回底重新跟随；
2. 内容在上方变化时，阅读位置不被顶走（块级锚点）；
3. 每帧只做与"变化量"相关的排版，不重复做全量工作；
4. debug 构建没有性能惩罚（opencode 跑在 JIT 运行时，不存在 Rust debug 的 10~50 倍放大）。

不需要照搬：尾部窗口挂载、估算总行数、平滑动画、全局加速开关。

## 3. 根因（按影响排序，附代码证据）

### 3.1 debug 构建放大了所有热路径（首要）

`Cargo.toml:47-58` 只给图片解码依赖设了 `opt-level = 3`，主 crate 仍是
`opt-level = 0`。同一份代码 debug 比 release 慢 2~4 倍；流式、展开大 diff
两项直接超过 16ms。

### 3.2 每帧计算 3 次全量列表指纹

调用点：

- `src/app/update/chat/viewport.rs:129`：`resolve_lx_history` → `history_view`
  → `snapshot_source` → `list_signature_source`；
- `src/ui/main_content/lx.rs:675`：`build_chat` → `history::view` → 同上；
- `src/ui/main_content/lx/history/render.rs:43`：`render` → `snapshot_source`
  → 同上。

指纹本身是 O(messages × steps)，且 `cache/tool_signature.rs:63-75` 里 MCP 参数/结果
每次都 `join("\n")` 生成临时 String：

```rust
// tool_signature.rs:63-75（现状）
hash.write_str(&view.server);
hash.write_str(&view.tool);
hash.write_str(&view.arguments.join("\n")); // 每次指纹都分配
hash.write_str(&view.result.join("\n"));    // 每次指纹都分配
```

### 3.3 每次 `history::view` 克隆整条消息行区间

`src/ui/main_content/lx/history.rs:411`：

```rust
messages: snapshot.messages.clone(), // Vec<MessageRows>，每帧 2~3 次分配
```

随后 `sticky::message_at`（`sticky.rs:30-35`）线性扫描，`sticky_candidate`
（`history.rs:439/449`）最多两次反向线性扫描 + `sticky_text` 分配。

### 3.4 工具卡每次绘制都整卡重排版

`BlockPaint::Legacy` 只缓存行数与点击面（`paint.rs:253-274`），绘制仍走
`tool::draw_animated`；命令行卡片在 `tool/command.rs:434-443` 每次都：

```rust
let (params, output) = sections(call, rect.width, folds);
let mut all = params;
all.extend(output);
tree::draw(grid, rect, scroll, row_index, &tree::layout(&all, rect.width).rows);
```

展开 2000 行 diff 时这是单卡最贵的一步（实测 debug 37.5ms / release 8.6ms）。

### 3.5 超预算时逐块全表淘汰

`cache.rs:434-453` 的 `evict` 是 `while bytes > limit { min_by_key(全表) }`，
每次淘汰都要扫全表；`insert_block` 每次插入都可能触发。长会话超过 32MB 后
会形成"重建 → 淘汰 → 再重建"的抖动。

### 3.6 没有阅读锚点

状态只有 `history_scroll`（顶部绝对行）+ `follow_bottom`
（`app/state/lx/chat.rs:513/494`）。历史在上方插入/变高（工具结果落地、展开切换、
后台汇报追加、compact 卡片插入）时，同一个行号指向了不同内容，阅读位置会跳。
opencode 用 `pinAbove()` + `scroll.anchor(node)` 保证"锚定节点在屏幕上的位置不变"。

### 3.7 滚轮事件路径的重复几何

`event/mouse/wheel.rs:567-582` 每格滚轮先 `history_wheel_region`（内部
`lx::layout` → 输入框整段折行），再 `history_rows`（一次完整 view：clone 消息行区间 +
置顶行扫描），然后才置脏；绘制前 `resolve_lx_history` 和 `build_chat` 又各算一次 view。
在 3.2/3.3 未修时，这些重复会随消息数线性增长。

## 4. 设计

### 4.1 构建配置：`[profile.dev] opt-level = 1`

`Cargo.toml`：

```toml
[profile.dev]
opt-level = 1
```

- 保留默认 `debug = true`（完整调试符号）；`[profile.test]` 继承 dev，
  `cargo test` 一并受益。
- 不改 release profile；release 性能已达标。
- 代价：冷编译约 3 分钟（实测 3m13s），之后增量正常。
- 不采用自定义 profile（如 `cargo run --profile fast`）：用户习惯 `cargo run`，
  自定义 profile 会被遗忘，等于没修。

### 4.2 每帧只算一次列表指纹（epoch 记忆）

新增"输入代"概念：事件循环在每轮绘制前把"本轮输入已固定"告诉缓存；同一代内
`(宽度, 数据源 owner)` 相同的指纹只算一次，解析阶段、绘制阶段共用。
全量指纹与「除流式末条外」指纹是两个槽：屏幕外合并判定帧各算一次，
其余帧（贴底/末条可见）只算全量那一次。

`src/ui/main_content/lx/history/cache.rs`：

```rust
pub(crate) struct RenderCache {
    frame: u64,
    /// 本轮输入代；事件循环每轮 +1。
    sig_epoch: u64,
    /// 只有事件循环显式开启时才复用指纹；测试默认关闭，保持现有语义。
    sig_memo_enabled: bool,
    panes: HashMap<PaneId, PaneCache>,
}

impl RenderCache {
    /// 绘制前调用（早于 resolve_lx_history）：声明本轮状态已固定。
    pub(crate) fn prepare_frame(&mut self) {
        self.sig_epoch = self.sig_epoch.wrapping_add(1);
        self.sig_memo_enabled = true;
    }
}

struct SignatureMemo {
    epoch: u64,
    width: u16,
    owner: usize,
    /// 全量列表指纹。
    full: Option<u64>,
    /// 「除流式末条外」指纹（离屏节流判定用），同一 epoch 也只算一次。
    non_stream: Option<u64>,
}

impl PaneCache {
    fn signature_for(&mut self, width: u16, source: HistorySource<'_>) -> u64 {
        if !self.sig_memo_enabled {
            return list_signature_source(source, width);
        }
        let owner = source.owner();
        let stale = self.sig_memo.as_ref().is_none_or(|memo| {
            memo.epoch != self.sig_epoch || memo.width != width || memo.owner != owner
        });
        if stale {
            self.sig_memo = Some(SignatureMemo {
                epoch: self.sig_epoch,
                width,
                owner,
                full: None,
                non_stream: None,
            });
        }
        if let Some(sig) = self.sig_memo.as_ref().and_then(|memo| memo.full) {
            return sig;
        }
        let sig = list_signature_source(source, width);
        if let Some(memo) = self.sig_memo.as_mut() {
            memo.full = Some(sig);
        }
        sig
    }
}
```

`snapshot_source_fresh` 用 `signature_for` 替换直接调用 `list_signature_source`；
`offscreen_stream_throttled` 的 `non_stream_signature_source` 同样走 epoch 记忆
（同一 `SignatureMemo` 的 `non_stream` 槽）。`sig_memo` 在 `PaneCache` 创建时默认为
`None`；`sig_memo_enabled` 由 `RenderCache` 在 `pane()` 时下发（测试不开启）。

**离屏基线延迟计算**：贴底构建时 `snapshot_non_stream` 存 `None`，流式帧不再多算
一次「除流式末条外」全量指纹；用户滚离底部后的第一次尾部增量走正常重建并补基线，
之后才进入 80ms 合并窗口。

**不变量**：本轮**第一次读取指纹之后**，不得再修改任何进入列表指纹的字段。
`prepare_frame` 之后先跑会改状态的 `flush_subagent_cards`，再进 resolve；绘制结束必须
调用 `finish_frame()` 关闭记忆，否则 `tui.draw` 之后到达的鼠标事件会复用上一帧
指纹、命中旧行。测试不调用 `prepare_frame`，行为与现在完全一致。

事件循环（`src/event/mod.rs:519-535`）：

```rust
if dirty && last_draw.elapsed() >= FRAME_INTERVAL {
    ui_cache.prepare_frame();          // 新增：本轮输入代，指纹只算一次
    update::flush_subagent_cards(state);
    update::resolve_lx_rewind_center(state, &geometry.rects, &mut ui_cache);
    update::resolve_lx_history(state, &geometry.rects, &mut ui_cache);
    // ...其余 resolve...
    ui_cache.begin_frame();            // 保持现有 LRU 记账语义不变
    let drawn = tui.draw(|frame| ui::render_cached(frame, state, &config, &mut ui_cache));
    ui_cache.finish_frame();           // 新增：draw 之后事件路径不得复用指纹
    drawn?;
}
```

顺带去掉 MCP 指纹里的两次 `join`（语义不变，减少分配）：

```rust
fn hash_lines(hash: &mut Fnv, lines: &[String]) {
    hash.write_usize(lines.len());
    for line in lines {
        hash.write_str(line);
    }
}
```

### 4.3 `messages` 改为 `Arc`，去掉每帧克隆

`HistorySnapshot`（`snapshot.rs:101-117`）与 `HistoryView`（`history.rs:271-286`）：

```rust
pub(crate) struct HistorySnapshot {
    pub(crate) entries: Vec<SnapEntry>,
    pub(crate) messages: Arc<Vec<MessageRows>>,
    // ...
}

pub struct HistoryView {
    // ...
    pub messages: Arc<Vec<MessageRows>>,
    // ...
}
```

构建期用 `Arc::make_mut(&mut builder.snapshot.messages)[index] = ...`（唯一持有，
不复制）；`view` 里 `snapshot.messages.clone()` 变为 O(1) 的 `Arc` 克隆。
所有读取点（`history.rs:421/439/449`、`history_hit.rs:174`、测试下标）通过 `Deref`
保持不变。

### 4.4 块级阅读锚点

#### 4.4.1 数据结构（不落状态）

实现采用**每帧旧快照 vs 新快照**的对照，不新增 `LxChat` / `SubagentRun` 字段：

- 内容变化时，`PaneCache` 里仍保留上一帧的几何快照；
- resolve 先读旧快照捕获锚点，再让 `history_view` 重建新快照；
- 新快照求同一块的 `new_row`，与捕获的 `row` 相减即为补偿位移；
- 内容没变时快照不重建，锚点行自然不变，无需跨帧保存。

锚点结构与捕获/解析放在 `src/ui/main_content/lx/history/anchor.rs`：

```rust
pub(crate) struct HistoryAnchor {
    pub(crate) entry: AnchorEntry, // 稳定 ID（消息/notice/compact/切换）
    pub(crate) block: usize,       // 块序号（身份失配时的回退）
    source: BlockSource,           // 块身份：优先按它找块，防块前插
    pub(crate) offset: usize,      // 块内行偏移
    pub(crate) row: usize,         // 捕获时的绝对内容行
    pub(crate) width: u16,         // 宽度变化直接放弃补偿
}
```

`AnchorEntry` 只存稳定 ID（`Message{index,id}`、`Notices{last,id}`、
`Compact{index,id}`、`ModelSwitch{last,id}`、`Raw`）。`RunningCompact` 没有稳定
身份，映射为 `Raw`，不补偿。

#### 4.4.2 捕获：读上一帧快照

`resolve_lx_history` 在调用 `history_view` **之前**：

```rust
let rect = history_rect(inner, lx)?;
let anchor = if !lx.chat.follow_bottom {
    history_anchor_before(rect, lx, cache.pane(pane), lx.chat.history_scroll)
} else {
    None
};
let mut view = history_view(inner, lx, cache.pane(pane))?;
```

`history_anchor_before` 只读 `cache.snapshot_cached(width, source)`（同宽度、同
数据源才返回），**绝不构建**；没有旧快照（首帧/resize/切页）就返回 None。

`anchor_at` 的定位顺序：

1. `entries.partition_point(|e| e.start <= scroll)`，落在条目间空行取下一个条目；
2. `blocks.partition_point(|b| b.start <= scroll)`，落在块间空行取下一块，
   末尾呼吸行取最后一块末行；
3. `SnapEntryKind` 映射稳定 ID；ID 在 `snapshot::build` 时存入 `SnapEntry.id`，
   不能用「新数据源 + 旧下标」取 ID（子 agent 恢复可能整体替换消息列表）；
   `RunningCompact` → `Raw`。

#### 4.4.3 补偿：新旧行差加回顶部行

```text
let new_row = history_anchor_row(rect, lx, cache, &anchor); // 新快照
let delta = new_row as isize - anchor.row as isize;
if delta != 0 {
    let max_top = view.total_rows - view.body.height;
    // 基准必须是捕获锚点时的 base_scroll；view.scroll 可能已被新几何钳制，
    // 再叠加 delta 会把同一个收缩算两次。
    let next = (base_scroll as isize + delta).clamp(0, max_top as isize);
    lx.chat.history_scroll = next;
    view = history_view(...); // 置顶行让位可能变，同一快照重算一次
}
// 锚点缺失/条目被删除时也不留越界状态：最终非跟随态同步为 view.scroll。
```

`anchor_row` 的解析规则：

1. `anchor.width != width` → None（resize 按行号重新钳制）；
2. 按 `AnchorEntry` 在新 `source` 里找条目（先验捕获下标 + ID，再全表按 ID）；
3. 先按 `BlockSource` 找块（防止汇报/分组前插导致序号漂移），找不到再退回
   `block` 序号；块身份与序号都失效时：
   - 消息正文块 → 回退 `entry.start`；
   - Stats/Recap/Attachments/Notice 等尾部块 → 回退 `entry.end - 1`；
   - 其他条目 → None；
4. `row = block.start + offset.min(block.rows-1)`。

**关键规则**（与已确认决策一致）：

- 补偿**永不**把 `follow_bottom` 从 `false` 改回 `true`；只有用户自己滚到底
  （`next >= max_top`）才恢复跟随。
- `follow_bottom == true` 时不捕获锚点，保持贴底。
- 宽度变化（resize / 折叠栏变化）`snapshot_cached` 直接失配，不补偿。
- 消息/块被撤销或压缩删除：正文块回退消息起始行，Stats/Recap/Attachments/Notice
  等尾部块回退消息末尾；消息消失保持行号并钳制到 `max_top`。
- 锚点只影响主历史和子 agent 详情；任务输出/ask/todo 不变。

#### 4.4.4 子 agent 详情

`resolve_task_agents_visible` 在 `needs_measure = run.rows_dirty || run.total_rows == 0`
时重建快照：

- 重建前用同样的 `snapshot_cached` + `anchor_at` 捕获 `run.scroll` 所在块；
- `task_agents_rows` 重建后，用 `anchor_row` 求新行并按同样公式补偿 `run.scroll`；
- 快照没重建时不做任何锚点工作（几何不可能变）。

详情区卡片 toggle 后若 `run.follow == true`，按上一帧的 `total_rows - visible`
钉住 `run.scroll` 并置 `follow = false`（对齐主历史的 `pin_lx_viewport`），
下一轮再按锚点补偿；否则展开视口内的卡片会把底部内容顶走。

详情区的 `should_follow` 只由「`run.follow || total <= visible+1`」决定；不再用
`run.scroll >= max` 短路。用户在滚轮/滚动条真正到底时会把 `run.follow` 置 true，
宽度变化或内容收缩造成的 clamp 不会自动恢复跟随。该语义与主历史一致，已有测试钉住。

### 4.5 滚轮路径 O(1)

- 去掉 3.3 的 clone 后，`history_rows` 只做：一次 `view_ready`（Arc 克隆 +
  `message_at` 二分 + 至多一次置顶候选反向扫描）+ 纯算术。
- `sticky::message_at` 改为按 `MessageRows.start` 二分
  （`messages` 按行号递增，见 `snapshot.rs:161-169`）。
- `sticky_candidate` 的反向扫描保留：只在顶行不是用户消息时触发，最多跨越
  若干条消息；如后续实测仍贵，再引入快照内 `prev_user_with_text` 索引（不在本期）。
- `history_wheel_region` 的 `lx::layout` 输入框折行保留现状：真实输入长度有限，
  本期不动；若后续出现"大段粘贴时滚动卡"，单独立项。

### 4.6 缓存淘汰改为单次扫描

`cache.rs:434-453` 改为：

```rust
fn evict(&mut self) {
    if self.bytes <= self.limit {
        return;
    }
    let frame = self.frame;
    let mut victims: Vec<(BlockSlot, u64)> = self
        .blocks
        .iter()
        .filter(|(_, entry)| entry.last_frame < frame)
        .map(|(slot, entry)| (*slot, entry.last_frame))
        .collect();
    victims.sort_unstable_by_key(|(_, last_frame)| *last_frame);
    for (slot, _) in victims {
        if self.bytes <= self.limit {
            break;
        }
        if let Some(entry) = self.blocks.remove(&slot) {
            self.bytes = self.bytes.saturating_sub(entry.bytes);
            self.md_streams.remove(&slot);
        }
    }
}
```

语义不变（仍只淘汰 `last_frame < frame` 的块），最坏复杂度从 O(n·k) 降为 O(n log n)；
预算内零成本。

### 4.7 工具卡绘制缓存（条件项，默认不触发）

Q6 选定 B 时把"工具卡布局缓存"列为手段之一，但实测 `opt-level=1` 后
展开 2000 行 diff 的 P95 已降到 8.7ms（<16ms）。因此本项设**明确触发条件**：

> P0~P2 完成后，用验收夹具复测：
> - debug `perf_lx_history_expanded_diff` P95 > 10ms，或
> - 真实会话中出现单帧 > 16ms 的可见工具卡滚动，
> 才实施本项；否则记录实测数据并关闭该项。
>
> **实测未触发**：debug `perf_lx_history_expanded_diff` P95 = 8.65ms（<10ms），
> 故 P5 关闭，不实施工具卡绘制缓存。

若触发，实施范围限定为：在 `BlockPaint::Legacy` 上增加"已完成卡片的绘制行缓存"
（按 `BlockSlot + sig + width + folds`，动画中的卡片仍走实时绘制），不改工具卡
语义、不改折叠状态、不改点击面几何。

### 4.8 作用域与入口

| 区域 | 是否改动 | 入口 |
|---|---|---|
| 主聊天历史 | 是 | `resolve_lx_history` / `history::view` / `history::render` / 滚轮与滚动条 |
| 子 agent 详情 | 是（共用锚点/快照/指纹） | `resolve_task_agents_visible` / `view_source_plain` / `scroll_agents_detail` |
| 任务输出、进程/Agents 列表、ask、todo、workspace 列表、侧栏 | 否 | 现有 `WHEEL_LINES` 逻辑不变 |

## 5. 行为状态机

| 事件 | follow_bottom | 本帧锚点 | history_scroll |
|---|---|---|---|
| 打开会话 / 首次布局 | true | 不捕获 | `total - visible` |
| 滚轮上滚 | false | 从上一帧快照捕获块级锚点 | 当前行 + delta |
| 滚轮下滚到 `max_top` | true | 不捕获 | `max_top` |
| 流式且贴底 | true | 不捕获 | 每帧 `total - visible` |
| 流式但已上滚 | false | 捕获；尾部在视口下方则 delta=0 | 不变 |
| 上方工具卡变高 | false | 捕获并补偿 | `scroll + (new_row - old_row)` |
| 点击展开/折叠**上方**卡片 | false | 捕获并补偿 | 同上 |
| 锚点块被折叠移除 | false | 正文块回退消息起始行；尾部块回退消息末尾 | 保持并钳制 |
| 撤销/压缩删除锚点消息 | false | 条目找不到 → 放弃 | 保持行号并钳制到 `max_top` |
| 窗口宽度变化 | 不变 | 旧快照宽度失配 → 放弃 | 现有 resize 钳制 |
| 切换会话 | true | 不捕获 | 重置 |
| 点击"回到底部" | true | 不捕获 | `max_top` |

## 6. 关键代码路径（实现清单）

1. `Cargo.toml`：新增 `[profile.dev] opt-level = 1`。
2. `src/event/mod.rs`：绘制块内、resolve 之前调用 `ui_cache.prepare_frame()`。
3. `src/ui/main_content/lx/history/cache.rs`：
   - `RenderCache::{sig_epoch, sig_memo_enabled, prepare_frame, finish_frame}`；
   - `PaneCache::{sig_memo, signature_for, snapshot_source_fresh, snapshot_cached}`；
   - `snapshot_non_stream: Option<u64>`（贴底时延迟计算）；
   - `evict` 单次扫描。
4. `src/ui/main_content/lx/history/cache/tool_signature.rs`：去掉 `join`。
5. `src/ui/main_content/lx/history/snapshot.rs`：
   - `messages: Arc<Vec<MessageRows>>`，构建期 `Arc::make_mut`；
   - `SnapEntry.id`：构建时存稳定 ID，锚点捕获不用新数据源按旧下标取 ID；
   - 占位行规范化为前一条消息的 `end`，保证 `messages.start` 单调；
   - `BlockSource::Plain` 拆成 `Attachments/Stats/RecapRunning`，同一消息内块身份唯一。
6. `src/ui/main_content/lx/history/anchor.rs`（新增）：
   - `HistoryAnchor` / `AnchorEntry`；
   - `anchor_at`（旧快照 → 块级锚点）与 `anchor_row`（新快照 → 新行号）。
7. `src/ui/main_content/lx/history.rs`：
   - `HistoryView.messages: Arc<Vec<MessageRows>>`；
   - `view_source_inner` 用 Arc 克隆；
   - 置顶行判定改二分。
8. `src/ui/main_content/lx/history_hit.rs`：`history_anchor_before` / `history_anchor_row`；
   `chat_hit` / `sticky_prompt_jump` 改用 ready 视图（命中只认已画出几何）。
9. `src/ui/main_content/lx/task/agents.rs`：详情区锚点捕获/解析（复用同一套 anchor 模块）；
   `selection.rs` 详情选区回退改用 `view_source_plain_ready`。
10. `src/app/update/chat/viewport.rs`：
    - `resolve_lx_history`：`snapshot_cached` 捕获 → `history_view` → 补偿 → 重算 view；
    - `resolve_task_agents_visible`：`needs_measure` 时捕获 → `detail_rows` → 补偿。
11. 测试：`cache/tests.rs`、`history/tests/anchor.rs`、`history/tests/sticky.rs`、
    `task/tests/agent_list.rs`、`app/update/chat/tests/viewport_anchor.rs`。

## 7. 验收

### 7.1 自动验证

```bash
cargo fmt
cargo clippy --all-targets -- -D warnings
cargo test

# 性能：release 必须全绿（单线程，避免重型 perf 互相争 CPU）
cargo test --release --lib -- --ignored perf_lx_ --test-threads=1

# 性能：debug + opt-level=1 必须全绿（本任务新增验收）
cargo test --lib -- --ignored perf_lx_ --test-threads=1
```

新增/保留的断言：

- 1400 条 / 10.3 万行：滚动、流式、展开 diff 的 P95 ≤ 16ms（release 与 dev 各跑一遍）；
- 300 条 / 3 万行 / 含一条 500KB 工具结果：同上；
- "无 >50ms 长任务"只约束滚动/流式帧，**不包含**会话首次加载、窗口 resize
  这类一次性全量构建（冷构建 392ms 是已知且可接受的一次性成本）；性能夹具
  已加入 max(≤50ms) 断言；
- 锚点：上方块变高后，锚点块在屏幕上的相对位置不变（±0 行）；
- 锚点：正文块消失回退消息起始行；Stats/Recap/Attachments/Notice 等尾部块
  消失回退消息末尾附近；消息消失保持行号并钳制到 `max_top`；
- 锚点：贴底流式不产生锚点；上滚后不因补偿自动恢复跟随；
- 指纹：同一 epoch 内两次 view 只重建一次；epoch 变化后按新状态重建；
  贴底/末条可见的流式帧只算 1 次全量指纹；屏幕外合并判定帧允许 full + non-stream；
- 现有几何/滚动条/置顶行/thinking 命中断言全部保持通过；
- 「输入到画面 <50ms」依赖真实终端与鼠标事件，保留为人工验收项。

### 7.2 人工验证

用户执行 `cargo run`：

1. 打开一个长会话，连续滚轮上下滚动，确认无明显掉帧；
2. 流式中滚动到中段，确认新内容不会把阅读位置顶走；
3. 滚回底部，确认恢复跟随；
4. 点击上方工具卡展开，确认正在阅读的段落不跳；
5. 子 agent 详情重复 2~4。

## 8. 风险与回滚

| 风险 | 影响 | 缓解 |
|---|---|---|
| `opt-level=1` 冷编译变慢 | 首次约 3 分钟 | 可一行回滚；增量编译正常 |
| 指纹 epoch 复用掩盖状态变化 | 画面滞后一帧 | 只允许事件循环在 resolve/draw 之间开启；测试默认关闭；新增"同 epoch 只重建一次"测试 |
| 锚点与用户滚动打架 | 滚动被拉回 | 惰性捕获（仅用户滚动后的新位置）；补偿永不改 follow；宽度变化丢锚点 |
| `Arc<Vec<MessageRows>>` 改动面 | 编译错误集中暴露 | 纯类型替换，Deref 兼容；几何测试全量验证 |
| 详情区去掉 `scroll >= max` 短路 | 内容收缩后不自动恢复跟随 | 用户主动滚到底/点回到底部会置 `follow=true`；测试钉住，主历史同语义 |
| 淘汰单次扫描 | 极端内存压力下排序开销 | 仅超预算触发；预算内直接返回 |

## 9. 被否决的方案

| 方案 | 否决原因 |
|---|---|
| 尾部挂载窗口 + 估算总行数（Q3-B） | 滚动条、置顶 prompt、撤销居中、选区坐标都依赖精确行号；估算会引入跳不准/滚动条漂移 |
| 消息级脏后缀增量（Q6-A） | 真实会话最大 11 条消息；为 1400 条规模维护消息级脏追踪与行号位移，回归面远大于收益 |
| 平滑动画 / 滚轮加速 | opencode 默认关闭；lx 终端无像素级滚动事件，做加速只能按事件间隔估算，属于体验增强而非对齐 |
| 全列表统一改造（Q7-C） | opencode 自己也不统一（消息列表 3 行、子 agent footer 1 行）；lx 的 `WHEEL_LINES=3` 已统一，改动收益低 |

## 10. 提交拆分

| 提交 | 内容 | 可独立验证 |
|---|---|---|
| P0 | `Cargo.toml` dev profile + 本文档 | debug 性能回归测试 |
| P1 | 指纹 epoch + 去 join + `messages` Arc + 置顶二分 | 缓存单测 + 性能回归 |
| P2 | 块级锚点（主历史 + 详情） | 锚点单测 + 手工验证 |
| P3 | 淘汰单次扫描 | 缓存上限测试 |
| P4 | 300 条验收夹具 + 性能断言 | dev/release 两套命令 |
| P5 | 工具卡绘制缓存（仅当 4.7 触发条件成立） | **未触发，未实施**（expanded diff P95 8.65ms < 10ms） |

实际实现落在 `ed76d856`（P0~P4 合在一起），review 修复在 `c942b28e`，
合并提交为 `fc166ad7`；审查详情见 `review.md`。
