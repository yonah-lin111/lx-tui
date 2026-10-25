# lx 历史性能优化 —— 代码评审

> 评审范围：`feat/lx-history-perf` 相对 `dev` 的全部改动（55 文件）。
> 评审对象是**已提交的代码**，不是设计文档；每条结论都给了可复现的失败场景。
>
> 严重度：**P0** 会崩/丢数据/明显错误；**P1** 用户可见的行为错误或规范违背；
> **P2** 边界缺陷、性能/资源隐患、文档与实现矛盾；**P3** 死代码、注释漂移、可读性。

## 0. 结论

缓存骨架（一级快照 + 二级 paint + 视口裁剪）是对的，行数单一口径也守住了：
`snapshot.rows == paint.rows() == draw 推进行数` 在三层裁剪下仍然成立，这是整套改动能站住的前提。

问题集中在**三处**：

1. **命中路径的 u16 截断**：`card_at` 把**内容行**先转成屏幕 `Rect` 再回判，
   超过 65535 行的历史整块失去点击能力（本分支的目标规模就是 10 万行）。
2. **缓存上限在稳定态不生效**：`evict()` 只在「重建快照」与「插入新块」时调用，
   两次重建之间即便超限也无人回收。
3. **文档与实现已经分叉**：`docs/standards/` 仍把「折叠内容 hover 提亮」写成硬规则
   （本分支已删除该行为），`■` 也仍被写成通知标题的字形（已改圆点）。
   规范是给下一个 agent 读的，漂移的规范会被当成需求重新实现回来。

外加一批**死代码**：hover 提亮移除后，`CardHit.fold` 与整条 `fold_region` 链已无消费者，
却仍在快照里逐块计算。

第二轮复核又抓到一处**同类命中缺陷**：普通工具详情恰好折成 3 个物理行时，
「能不能展开」的判据被提示行抵消，卡片显示 `… 1 more line` 却没有点击面（§12.1）。

---

## 1. P1：`card_at` 的内容行 → 屏幕矩形换算在 65535 行处截断

**关键代码** `src/ui/main_content/lx/history_hit.rs:174-198`：

```rust
let content_row = view.scroll.saturating_add(usize::from(row.saturating_sub(view.body.y)));
let region = snapshot.cards.iter().find(|region| {
    content_row >= region.card_start
        && content_row < region.card_start.saturating_add(region.card_rows)
})?;
let rect_at = |start: usize, rows: usize| -> Rect {
    Rect::new(
        view.body.x,
        view.body.y.saturating_add(history::rows_to_u16(start)), // ← start 是绝对内容行
        width,
        history::rows_to_u16(rows),
    )
};
let card = screen_rect(&view, hit.card)?;
card.contains((column, row).into()).then_some(hit)
```

**错误原因**：`region.card_start` 是**内容行绝对下标**，`rows_to_u16` 在 65535 处饱和
（`rows.rs:12`）。历史超过 65535 行后，`rect.y` 恒定饱和在 65535，
`screen_rect` 里 `offset = rect.y - view.body.y = 65535`，
`skip = view.scroll - 65535 ≥ rect.height` → 返回 `None` → 点击直接丢弃。

**失败场景**：`perf_lx_history` 的会话就是 100,800 行。滚到第 70000 行，
点击任意 thinking 头部或工具卡片 → 无任何反应（折叠展不开）。
这正是用户上一轮报的「点击折叠内容无法展开」的同一类症状，只是触发条件是长历史。

**这行判断本身是冗余的**：`content_row` 的区间判断已经等价于「点击落在该块的可见行内」，
`screen_rect` + `contains` 只多引入了 u16 截断。1 万行内看不出来，10 万行必现。

**修复方向**：在 `usize` 空间算可见偏移（`start.saturating_sub(view.scroll)`），
再把**已裁剪的相对偏移**转 u16；或干脆去掉 `screen_rect` 回判。
须保留 `CardHit.card` 的屏幕矩形语义（`open_entry.rs:393`、`event/tests.rs:4945`
用 `card.card.y` 构造点击坐标）。

---

## 2. P1：二级缓存上限在「稳定态」不生效

**关键代码** `src/ui/main_content/lx/history/cache.rs:181-197`：

```rust
fn evict(&mut self) {
    while self.bytes > self.limit {
        let victim = self.blocks.iter()
            .filter(|(_, entry)| entry.last_frame < frame)
            .min_by_key(|(_, entry)| entry.last_frame)...
    }
}
```

调用点只有三处：`snapshot()` 重建后、`insert_block()` 后、`set_limit()`（测试）。

**错误原因**：一次冷启动会把整段历史的块以 `last_frame = frame` 插进来，
`evict` 的 `last_frame < frame` 过滤让**本帧没有任何块可淘汰**，
于是 `bytes` 停在上限之上。此后只要用户不再产生新内容、也不重建快照，
`evict` 就再也没被调用过——32MB 上限形同虚设。

**失败场景**：冷启动一套 40MB 的历史（当前 fixture 是 13.58MB，
把正文块放大 3 倍即可复现），随后静置不动：进程 RSS 长期持有 40MB 而不回落。
流式期间每帧都会重建快照，所以这条路径「看起来」正常，掩盖了静置态的问题。

**修复方向**：`render()` 末尾（`mark_used` 之后）调用一次 `cache.evict()`——
此时 LRU 时间戳才反映真实使用情况，且超限才扫描，无额外开销。

---

## 3. P1：`docs/standards/` 与实现分叉（hover 提亮、圆点/方块）

本分支 F1~F8 删除了「工具卡片与本地命令卡片的折叠内容随 hover 提亮」，
并把正文/思考改成了圆点。但规范里仍是旧口径：

| 位置 | 现状（规范） | 实现 |
|------|--------------|------|
| `tui-design-requirements.md:116` | 「折叠内容在**鼠标悬停**时从 `muted()` 提亮为终端默认前景……末尾的 `… N more lines` 提示行……一起提亮」 | 已删除，折叠恒 `muted()` |
| `tui-design-requirements.md:116` | 「输出区折叠 dim、悬停或展开为终端默认前景（没达到折叠额度时不响应 hover）」 | 已删除，只看展开 |
| `tui-design-requirements.md:118` | 「hover 命中区是整块折叠内容、但只有这一行提亮」 | 已删除 |
| `tui-design-requirements.md:116` | 通知标题「渲染为 `■ <标题>`——行首标记是与聊天步骤同一个**方块**」 | 已是圆点 `●` |

**错误原因**：`AGENTS.md` 明确「以上文档是项目规范的唯一来源」。规范说 hover 提亮是行为，
下一个 agent 读到就会把这个功能加回来，与用户的「不要再提亮」直接冲突——
上一轮的「点击展不开」bug 就是这样在规范与代码之间来回漂移产生的。

**修复方向**：把上述四处改成「折叠内容**不随 hover 变色**，只有展开才升为默认前景」，
通知标题字形改 `●`。同时 `history.rs` 的模块注释（8~16、59、68~77、96、120 行）
仍写「方块」，属同一类漂移，一并改。

---

## 4. P2：文件差异的指纹不含正文，行数可能漂移

**关键代码** `cache.rs:495-507`：

```rust
Some(FileDiffView::Complete { hunks, simplified }
   | FileDiffView::Truncated { hunks, simplified }) => {
    hash.write_u8(2);
    hash.write_u8(u8::from(*simplified));
    hash.write_usize(hunks.len());
    for hunk in hunks { hash.write_usize(hunk.lines.len()); } // ← 只有行数
}
```

**错误原因**：`BlockPaint::Legacy` 缓存的是**行数**，而差异正文的行数取决于
「每行文本按列宽折行后有几行」。只哈希 `hunks.len()` 与 `hunk.lines.len()` 时，
**同样形状、不同内容宽度**的两份差异指纹相同 → 命中旧块 → 行数停留在旧值。

**失败场景**：`write` 工具对同名文件写两次，两次都是 1 个 hunk、各 20 行，
但第二次每行长 200 字符（折行后 40 行）。第二次的卡片仍按 20 行排版，
`render` 实际画 40 行 → 后续所有块下移 20 行，滚动条与命中矩形全部错位。

（`design.md §13.3` 只写了「等长改写且首尾 64 字节相同会漏判」，
低估了这条：差异正文**一个字节都没进指纹**。）

**修复方向**：把 `hunk.lines.len()` 换成「每行的文本长度 + kind」——
仍是 O(行数) 的整数哈希，不做全文扫描，足够区分折行结果。

---

## 5. P2：hover 提亮删除后留下的死代码链

`CardHit.fold` 现在没有任何生产读者（`history_hit.rs` 只用 `hit.card`），
但它背后挂着一条仍在逐块计算的链：

```
tool::fold_region → FoldRegion → file_diff::folded_rows / file_summary::foldable_rows
        ↑
tool::CardRegion.fold  →  snapshot::CardRegion.{fold_start, fold_rows}  →  CardHit.fold
```

**错误原因**：`merge_group` 与 `tool_cards` 每帧（快照重建时）为每个卡片
额外跑一次「折叠态整块排版」只为填一个没人读的矩形——`file_diff::folded_rows`
会真的把整份差异按折叠态 layout 一遍。

**修复方向**：删掉 `CardHit.fold`、`snapshot::CardRegion.fold_*`、`tool::CardRegion.fold`、
`FoldRegion`/`fold_region`/`folded_rows`/`foldable_rows`，以及 `tool/tests` 里对应的两条断言。
预计净减 80 行左右，与「优雅就是删掉没用的代码」一致。

---

## 6. P2：`perf_lx_history` 的性能数字不覆盖工具卡片

`history/tests/perf.rs:39-50` 的压力会话只有正文 + thinking，没有一条工具步骤。
而工具卡片全部走 `BlockPaint::Legacy`（每次绘制重新排版），是本次**唯一没进二级缓存**的路径。

「热缓存滚动 P95 1.05ms」只回答了 markdown/thinking 的成本；
真实会话里工具卡片占比很高，这个数字不能代表用户看到的滚动手感。
`perf_lx_history_expanded_diff` 只量了「一个展开的大差异」。

**修复方向**：在 `history()` fixture 里给每条 assistant 消息插 2~3 个工具步骤
（`read`/`exec_command` 各一），重跑并把新数字写回 `design.md §13.2`。
若 P95 超预算，正好给 C3~C8 提供了「必须做」的依据。

---

## 7. P2：`notice.rs` 两处 `StepKind` 传参不一致

`src/ui/main_content/lx/notice.rs:60` 与 `:76`：

```rust
fn mark_style(self) -> Style { self.mark().style(StepKind::Tool, false, false) } // ①
fn marker_segment(self) -> String { self.mark().segment(StepKind::Body, 0, false) } // ②
```

**错误原因**：通知是**文字数据**，两处都该传 `Body`。今天 `Failed`/`Interrupted`
是跨角色覆盖色，`kind` 不参与决策，所以看不到差异；但这正是
「字形按 kind 分派」改动的同一个雷区——下一次有人给 `StepKind::Tool` 加分支，
通知标题就会一半方块一半圆点。

**修复方向**：`mark_style` 也传 `StepKind::Body`，并删掉那段「只为满足签名」的注释。

---

## 8. P2：`view()` 每帧把所有卡片连同 `String` 一起克隆一遍（生产路径无人消费）

**关键代码** `history.rs:293-296` 与 `layout.rs:22-59`：

```rust
pub(super) fn view(area: Rect, lx: &Lx, cache: &mut PaneCache) -> HistoryView {
    let layout = layout(area, lx, cache);      // ← 每帧完整 to_layout
    let total = layout.total_rows;
    // ……只用了 layout.total_rows 与 layout.messages
```

而 `to_layout` 会为**每一张卡片**构造 `CardHit { kind: region.kind.clone(), .. }`，
`CardKind::Tool` / `CardKind::Shell` 各带两个 `String`（`message_id`、`call_id`）。

**错误原因**：命中和绘制都已经改成直接读快照（`snapshot.cards`），
`HistoryLayout.cards` / `.thinking` 现在**只有测试在读**；生产路径只剩
`view()` 与 `history_center_target()` 两个调用方，两者都只要 `total_rows` + `messages`。
于是一个 1400 条消息、每轮 3 次工具调用的会话，每次 `view()` 白克隆约 2100 张卡片、
约 4200 次 `String` 分配；而 `view()` 每帧被 `resolve_lx_history`、`build_chat`、
`history_scrollbar` 以及每次鼠标事件的 `card_at` 各调一次，合计 4~5 次/帧。

**失败场景**：工具密集的会话（正是 §6 指出的 `perf` fixture 没覆盖的那类）
滚轮每格触发一次 `history_rows` → `view()`，多出数千次分配；
16ms 预算里白白烧掉近 1ms，且随工具调用数线性增长。

**修复方向**：`view()` 与 `history_center_target()` 直接读 `cache.snapshot(width, lx)`
取 `total_rows`/`messages`；`layout()`/`to_layout()`/`HistoryLayout` 降级为测试专用
（`#[cfg(test)]`），生产路径不再构造 `CardHit`。

---

## 9. P3：注释与死参数

| 位置 | 问题 |
|------|------|
| `history.rs:8-16,59,68-77,96,120` | 模块注释仍写「方块标记」「静态方块」，与圆点规则冲突（见 §3） |
| `snapshot.rs:596` | `tail()` 末尾 `let _ = lx;`——参数已无用，应删参数 |
| `snapshot.rs:587` | 统计行块复用 `BlockSource::Notice`，语义错位（`Static` 不看 source，改天有人给它加分支就踩到） |
| `history.rs:202` | `needs_spacer()` 恒 `true` 且无参数；注释已解释「不做死抽象」，但现在它就是死抽象，调用点直接写 `self.spacer()` 更诚实 |
| `paint.rs:203-211` | `folds_at` 用 `source.message_index()` 反查消息，只为拿 `message.id`；可改用 `message_index` 直取 |

---

## 10. 逐项核对过的、确认没问题的点

- **行数单一口径**：`Snapshot.rows == paint.rows()`；`UserText`/`Markdown`/`Static` 用
  `lines.len()`，`Thinking` 用 `1 + body.len()` 且 `draw_thinking` 恰好推进 1 + body 行；
  `Legacy` 的 rows 由 `tool::rows`/`user_shell::rows`/`notice::rows` 给出，与各自 draw 的
  推进行数一致（`tool_geometry.rs` 已在 4/8/20/40/72 五种宽度 × 展开态上逐一比对）。
- **三层裁剪**：entry → block → row，`render` 返回值仍等于 `snapshot.total_rows`。
- **空行归属**：条目间空行记在前一条 `end` 之外（`sticky::message_at` 落在空行时返回
  `None`），与 dev 完全一致；消息内空行由 `steps`/`tail`/`merge_group` 各自 `spacer()`，
  `every_block_is_separated_by_a_blank_row` 已覆盖「正文 + 合并组 + thinking + 正文」。
- **指纹不包含每帧变化项**：`hover`/`loading_frame`/`follow_bottom`/`history_scroll`/
  `suppressed_prompt_header`/`rewind.selected` 均不进指纹（`frame_only_state_does_not_rebuild` 钉住）。
- **`block()` 命中不刷新 `last_frame`**：这是 LRU 能工作的关键，且已注释说明。
- **`begin_frame` 与 `resolve_*` 的先后**：resolve 期间 `pane.frame` 仍是 F-1，
  当前帧新建的块因此**不会**在构建过程中淘汰上一帧画过的块；顺序虽微妙但结论正确，
  建议在 `RenderCache::pane` 上补一句注释固定这个前提。
- **渲染只读**：`ui/` 侧只写 `PaneCache`（缓存本身），未回写 `app/` 状态；
  `viewport_top`/`suppressed_prompt_header` 仍由事件循环写。
- **两个 `ui::render` 入口**：`render`（三参）只在测试与 `toast`/`tooltip` 单测里用，
  生产事件循环走 `render_cached`；临时缓存不推进帧号也不会失控（单帧生命周期）。
- **`chat_geometry_is_live` 收口**：任务面板打开时所有历史命中入口统一失效，没有漏网。
- **展开判据不再把提示行算作内容**：`params_expandable` / `output_expandable` /
  `has_more_content` / `detail_content_rows` 四处口径一致，上一轮的「点了展不开」已修。
- **死悬停接口已清干净**：`tool_hovered_section` / `tool_is_hovered` / `shell_hovered_section`
  与 `Region.hoverable` 均已删除，无残留调用点。

---

## 11. 修复清单（本次执行）

| # | 严重度 | 文件 | 内容 |
|---|--------|------|------|
| 1 | P1 | `history_hit.rs` | 命中的可见偏移改在 `usize` 空间计算，去掉 u16 截断 |
| 2 | P1 | `history/cache.rs`、`render.rs` | `render` 末尾按真实使用情况回收，稳定态也守住 32MB |
| 3 | P1 | `docs/standards/tui-design-requirements.md`、`history.rs` | hover 提亮与 `■` 通知字形改回实现的真实口径 |
| 4 | P2 | `history/cache.rs` | 文件差异指纹补上每行文本长度，修行数漂移 |
| 5 | P2 | `history/{cards,snapshot}.rs`、`tool.rs`、`file_diff.rs`、`file_summary.rs` | 删除 `fold` 死链 |
| 6 | P2 | `history/tests/perf.rs` | 压力会话加入工具卡片，重测 |
| 7 | P2 | `notice.rs` | `StepKind` 传参统一为 `Body` |
| 8 | P2 | `history.rs`、`history/layout.rs`、`history_hit.rs` | `view()`/`history_center_target()` 直读快照，`layout` 链降为测试专用 |
| 9 | P3 | 多处 | 注释与死参数清理 |
| 10 | P1 | `tool/file_summary.rs`、`tool.rs` | 详情恰好折成 3 行时「能展开」判据失效（见 §12） |
| 11 | P2 | `history_hit.rs` | `history_rows` / `history_center_target` 补 `chat_geometry_is_live` 守卫 |
| 12 | — | `history/tests.rs` | 1763 行超上限，按主题拆成 `tests/{cards,compact,rewind,sticky,transcript}.rs` |

---

## 12. 第二轮复核补记（2026-10-25）

### 12.1 P1：普通工具详情恰好折成 3 个物理行时，卡片可展开却不可点

**关键代码** `tool/file_summary.rs:504-512` 与 `tool.rs:432-435`：

```rust
pub(super) fn detail_content_rows(call: &ToolCallView, width: u16, expanded: bool) -> usize {
    detail_rows(call, width, expanded)
        .saturating_sub(usize::from(!expanded && omitted_rows(call, width) > 0))
}

fn omitted_rows(call: &ToolCallView, width: u16) -> usize {
    detail_rows(call, width, true).saturating_sub(detail_rows(call, width, false))
}

// tool.rs
Some(File(_)) | None => {
    file_summary::detail_content_rows(call, width, true)
        > file_summary::detail_content_rows(call, width, false)
}
```

**错误原因**：`detail()` 的折叠态行数 = `shown + (剩余 > 0 ? 1 : 0)`，其中
`shown = min(total, DETAIL_PREVIEW_ROWS)`。`omitted_rows` 想用「展开行数 − 折叠行数」
反推「有没有提示行」，但当 `total == fold_budget + 1`（即 **3 行**：2 行正文 + 1 行提示、
展开 3 行正文）时差值恰好为 0 —— 提示行**顶掉了一行正文**，差值被抵消，
于是 `detail_content_rows` 两侧都返回 3，`3 > 3` 为假 → `card_regions` 返回空。

**失败场景**：宽度 80（详情可用 76 列）的 `read` 带约 160 字符路径，折叠态显示
`… 1 more line`，但整卡没有点击面，点击只会起选区——正是用户报过的「点折叠内容展不开」
的同一类症状，只是触发条件是「详情行数刚好卡在折叠额度 + 1」。

**修复方向**：判据改成比较**真正画出的内容行数**——折叠侧取 `shown`（不含提示行），
展开侧取 `total`。这两个值 `detail()` 内部已经算好（`shown`/`total` 局部变量），
只是没有暴露出来；把落行数、`shown`、`total` 一起从一个内部函数返回，比在外部用
两次 `detail_rows` 相减去反推「有没有提示行」更诚实，也少一遍重复排版。

### 12.2 P2：32MB 上限的语义与 `copy_text` 的临时缓存（记录，未改结构）

- **预算按窗格计**：`PaneCache::limit` 恒为 `MAX_CACHED_BYTES`，每个 lx 窗格各 32MB，
  N 个窗格就是 N×32MB；设计 §7 写的是 32MB 总量。
- **上限只约束「过期代」**：`SnapBlock.paint` 与缓存 `blocks` 指向同一个 `Arc<BlockPaint>`，
  当前快照持有的那份内存不会被淘汰回收。所以实际占用 ≈ 当前快照 + ≤32MB 旧代，
  这个界仍然有效，但「32MB 封顶」的说法比实现强。
- **`copy_text` 的临时缓存**（`lx.rs:307`）`frame == 0`，`evict` 的 `last_frame < frame`
  恒假 → 一次调用把整段历史的 `BlockPaint` 全部物化且不设上限（旧实现只算行数）。
  实测 1400 条消息的会话约 13.6MB，属一次性峰值、函数返回即释放。

**不本次修的原因**：把预算挪到 `RenderCache` 全局记账、让快照按需物化 paint，都是
改数据结构而非改 bug；当前规模下没有可观测的故障，先记录口径，避免规范继续写成
「32MB 封顶」。

### 12.3 复核中发现、但属于既有问题（不在本分支范围）

| 位置 | 问题 |
|------|------|
| `history_hit.rs:59`、`:106-108` | 置顶行/居中/跳转用整个历史区高度，渲染与滚轮用 `body.height`；置顶行显示时 `max_top` 差 1 行（dev 同样存在） |
| `history/layout.rs:35` | `to_layout` 仍把绝对内容行交给 `rows_to_u16`，>65535 行时矩形饱和（与 §1 同源）。`layout` 已降为 `#[cfg(test)]`，测试用小内容，不构成生产缺陷；若将来重新暴露，必须改用**裁剪后的相对偏移**（与 §1 的 `clip` 同法） |
| `tool.rs:503-513` | `write_stdin`（Interaction）运行中不登记整卡点击面，短命令时整卡不可点（Command 分支有） |
| `event/mouse/hover.rs:157-170` | 指针从思考块移入 `/` 面板时提前返回，旧 `hover=Thinking` 残留（dev 同样存在） |
| `event/mouse/*.rs` | `chat_geometry_is_live` 为假时仍先求值 `cache.pane(pane)`，只是写入上一帧帧号，无泄漏 |
