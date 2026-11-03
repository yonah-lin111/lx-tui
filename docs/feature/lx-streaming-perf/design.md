# lx 长思考与流式输出卡顿优化设计（`lx-streaming-perf`）

> 状态：**已合并到 dev**（2026-11-03，合并提交 `980308ce`）；自动验证通过（4391 passed / 3 ignored）。
>
> 参考仓库：
> - `/Users/yonah/projects/agent/codex-main`（`codex-rs/tui`）
> - `/Users/yonah/projects/agent/opencode-dev`（`packages/tui` & `packages/session-ui`）
> - `/Users/yonah/projects/agent/openclaude-main`（Claude Code 流式与 Thinking 机制）
>
> 目标代码位置：`src/ui/main_content/lx/`、`src/ui/markdown_render/`、`src/event/mouse/`

---

## 0. 已确认的决策（Grill Me 边界确认结论）

| # | 决策维度 | 确认结论 |
|---|----------|----------|
| 1 | **思考块（Thinking）流式展示与展开策略** | **采用 Claude Code 模式**：思考步骤仍在生成中（`reasoning_streaming == true`）时，界面恒定只渲染单行头部 `○ Thinking`（呼吸圆点 + 文案），**不流式渲染思考正文**；即使用户在思考流式期间点击切换了展开标记，思考正文的 Markdown 排版也延迟到该段思考完成瞬间一次性静态生成。思考未展开或仍在流式期间，列表与块指纹忽略 `part.content` 与 `ended_at_ms` 变化，做到思考流式期间 **0 次快照重建、0 次 Markdown 解析**。 |
| 2 | **长正文（Markdown / 代码块）流式解析与缓存策略** | **采用块级增量 Markdown 解析 + 同槽位原地覆写缓存（参考 Codex & Claude Code）**：将流式正文拆分为「已稳定前缀（Stable Prefix）」与「活跃尾部（Active Tail）」，已完成段落与已换行围栏代码行沉淀进 `Arc<Vec<MdLine>>` 增量复用，每帧只解析尾部未稳定区间（复杂度从 $O(N)$ 降为 $O(\text{tail})$）；`PaneCache` 对同一 `(message_index, step_index, width)` 原地覆写，消除流式历史残影堆积与 `evict()` 全表线性扫描风暴。 |
| 3 | **滚动条拖拽、滚轮与屏幕外流式解耦策略** | **鼠标交互直接走已就绪快照 + 滚离底部时对屏幕外流式降频同步**：滚轮（`wheel.rs`）、拖拽滚动条（`drag.rs`）、悬停命中（`hover.rs`）优先读取 `PaneCache` 中同宽度的已就绪快照（`snapshot_ready`），不在鼠标回调线程内阻塞执行快照重建；用户滚离底部（`follow_bottom == false`）且流式块位于视口下方屏幕外时，几何快照按时间窗口（80ms）节流合并更新。 |
| 4 | **优化范围、文档落点与量化性能指标** | 聚焦 `lx` 主会话与子 Agent 详情页共用的历史渲染链路（不改动 DB schema、终端 PTY、侧边栏）；单文件严格控制在 1000 行上限内（拆分 `cache.rs` 与 `markdown_render.rs`，子文件控制在 ~500 行）；在 `history/tests/perf.rs` 新增长思考流式与长正文边流式边滚性能回归测试（单帧 `draw` P95 ≤ 8ms，思考流式期间快照重建 0 次）。 |

---

## 1. 问题现象与根因深度剖析

### 1.1 问题现象

在 `src/ui/main_content/lx/` 中，当 Agent 进行长思考（Reasoning，如 DeepSeek-R1 / Claude Thinking / o3 动辄 10,000～30,000 字符）或长正文/长代码块流式输出（Text，数百至上千行）时，出现明显界面卡顿，尤其在滚轮滚动历史消息或鼠标按住滚动条上下拖动时严重掉帧、粘滞。

### 1.2 核心根因定位（结合 `codegraph` 与 `codebase-memory` 追踪）

```
SSE 流式 Delta (100~300 次/秒)
  │
  ▼
event/mod.rs:569  receiver.recv() + try_recv() 排空通道
  │  target.lx.chat.append_stream_delta() → part.content.push_str(delta), part.ended_at_ms = Some(now)
  │  dirty = true
  ▼
event/mod.rs:494  每 16ms (FRAME_INTERVAL) 或鼠标事件触发：
  ├─ [瓶颈 8] mouse/wheel.rs:553 / drag.rs:237 / hover.rs:241
  │     └─ 每次鼠标微动调用 history_view → cache.snapshot_source() 撞上失效指纹，在输入回调中同步重排！
  ├─ [瓶颈 1] history/cache.rs:465 step_signature()
  │     └─ 无条件写入 part.content.len() + write_edges(&part.content) + ended_at_ms
  │     └─ 哪怕 Thinking 处于折叠态（只画 1 行 `○ Thinking`），每个思考 token 都击穿整表 list_signature！
  ├─ [瓶颈 2] history/snapshot.rs:116 snapshot::build()
  │     └─ 每帧重新遍历 timeline(source)，重新分配 entries / messages / cards / thinking 数组
  ├─ [瓶颈 3 & 4] history/snapshot.rs:692-707 build_step()
  │     └─ BlockKey 含 sig，流式每帧 100% Cache Miss！
  │     └─ 对累计全文（数万字）从第 0 行重新执行 markdown_render::render_wrapped() → O(N²) 爆炸！
  └─ [瓶颈 7] history/cache.rs:203 insert_block() & evict()
        └─ 同一流式步骤产生几百个不同 sig 的 BlockKey 塞入 HashMap，撑爆 32MB 触发 O(N) 全表扫描驱逐！
```

#### 根因 1：折叠态与流式态思考块（Thinking）无效击穿整表签名（`history/cache.rs:459-473`）

当前 `step_signature` 实现：

```rust
// src/ui/main_content/lx/history/cache.rs:459-473（现状）
AssistantStepView::Part { part, .. } => {
    hash.write_u8(0);
    hash.write_u8(match part.kind {
        crate::chat::events::PartKind::Text => 0,
        crate::chat::events::PartKind::Reasoning => 1,
    });
    hash.write_usize(part.content.len());
    hash.write_edges(&part.content);
    hash.write_i64(part.started_at_ms.unwrap_or(-1));
    hash.write_i64(part.ended_at_ms.unwrap_or(-1));
    hash.write_i64(part.completed_at_ms.unwrap_or(-1));
    hash.write_u8(u8::from(
        source.chat.reasoning_expanded(message_index, step_index),
    ));
    // ...
}
```

- **问题 A（折叠态无效失效）**：当思考块处于折叠态时，`snapshot.rs:705` 生成的 `BlockPaint::Thinking { body: Vec::new() }` 恒为 **1 行**（生成中画 `○ Thinking`，完成后画 `● Thought · 6.3s`）。但在生成期间，每收到一个思考 token，`part.content.len()`、首尾 64 字节和 `part.ended_at_ms` 都在变，导致 `list_signature_source` 每 16ms 失效一次，迫使整张 `HistorySnapshot` 每秒重建 60 次！
- **问题 B（展开态 $O(N^2)$ 崩溃）**：一旦用户在思考期间展开思考块，每 16ms 都会对数万字符的 `part.content` 从头执行 `markdown_render::render_wrapped`，单帧耗时从 <1ms 飙升至数十毫秒。

#### 根因 2：流式正文（Markdown）全量重解析与全量折行（`markdown_render.rs:103-192, 632-703`）

当前 `markdown_render::render_wrapped` 每次接收完整 `&part.content`：
1. `source.split('\n').collect()` 将全文切分为数千行；
2. 从第 0 行起全量重跑块级解析（标题、列表、代码块语法高亮 `highlight_line`、表格、引用）和行内解析 `inline_spans`；
3. `wrap_md_line` 将每一行的所有 `spans` 展开为 `Vec<(char, Style)>` 单字符数组并重新分配 `String`。
- 在流式生成长度为 $L$ 的回答时，累计解析复杂度为 $\sum_{i=1}^{K} O(L_i) = O(L^2)$。

#### 根因 3：二级缓存 `PaneCache.blocks` 键膨胀与拷贝开销（`history/cache.rs:28-33, 203-249`）

- `BlockKey` 定义为 `{ message_index, step_index, width, sig }`。
- 流式追加过程中，同一个 `(message_index, step_index, width)` 在 500 帧里会产生 **500 个不同的 `sig`**，向 `self.blocks: HashMap<BlockKey, BlockEntry>` 插入 500 份历史中间残影！
- 一个长回答的 500 个中间版本大小之和为 $\sum_{i=1}^{500} \text{size}(i) \approx 250 \times \text{final\_size}$，极易冲破 `MAX_CACHED_BYTES`（32MB），进而每帧触发 `evict()` 对 `HashMap` 做 $O(N)$ 全表线性扫描。

#### 根因 4：鼠标滚轮/拖拽滚动条在输入事件中同步触发重排（`wheel.rs:553`, `drag.rs:237`）

- 触控板滚轮和鼠标拖拽滚动条会产生 60～120Hz 的 `TerminalEvent::Mouse`。
- `wheel.rs:553` 调用 `history_rows`，`drag.rs:237` 调用 `history_scrollbar`，`hover.rs:241` 调用 `fold_target_at`，三者内部全部调用 `history_view` → `cache.snapshot_source`。
- 在流式输出期间，后台 `try_recv()` 刚把 `part.content` 改掉，紧接着到来的鼠标拖拽事件就会在 `snapshot_source` 中撞上签名不匹配，被迫在鼠标事件处理函数里同步跑完快照构建与 Markdown 解析，造成滚动条拖动手感明显卡顿。

---

## 2. 参考项目核心机制对照（`codex-main` / `opencode-dev` / `openclaude-main`）

| 机制维度 | `codex-main` (`codex-rs/tui`) | `opencode-dev` & `openclaude-main` (Claude Code) | 本项目 (`lx-tui`) 借鉴与落地方案 |
|----------|-------------------------------|--------------------------------------------------|----------------------------------|
| **思考块流式处理** | `chatwidget/streaming.rs:333`：流式期间完全不往历史区写正文行，完成后归档为折叠单元。 | `openclaude-main/src/utils/messages/streaming.ts:178`：收到 `thinking_delta` 时**完全不向 UI 分发思考正文**，仅显示 `Thinking` 指示器；等思考块完成时才一次性挂载静态全文。 | **对齐 Claude Code**：`reasoning_streaming == true` 时恒定不渲染思考正文（`body: Vec::new()`），签名忽略 `content` 与 `ended_at_ms`；思考完成且用户展开时才一次性渲染全文。 |
| **Markdown 增量渲染** | `streaming/render.rs:136` & `markdown_stream.rs:87`：维护已提交稳定行前缀，对未闭合顶层围栏代码块（`OpenCodeFence`）直接按行追加，对表格（`TableHoldback`）暂扣在尾部。 | `openclaude-main/src/components/Markdown.tsx:186`：维护单调递增的 `stablePrefixRef`，已稳定前缀缓存复用，每帧只解析 `unstableSuffix`。 | **实现 `IncrementalMarkdown`**：按行扫描识别稳定边界（段落/标题/列表/引用/已换行围栏代码行直接入稳定前缀 `Arc<Vec<MdLine>>`；未完结表格與缩进代码块保留在尾部），每帧只解析尾部并利用 `Arc::make_mut` 零拷贝追加前缀。 |
| **流式缓存管理** | `chatwidget/transcript.rs:8`：活跃流式单元使用单一槽位 + `active_cell_revision` 原地替换，不堆积历史中间版本。 | Solid Store 按叶子节点原地更新，不创建多份历史 DOM 快照。 | **同槽位原地覆写**：`PaneCache.blocks` 以 `BlockSlot { message_index, step_index, width }` 为键、`sig` 存于 `BlockEntry` 内，流式增长时原地替换旧条目并扣减旧字节数，杜绝键膨胀。 |
| **输入与滚动解耦** | `app/startup.rs:1629` & `frame_requester.rs:105`：用户输入事件优先处理，重绘统一合并到帧调度点。 | `useVirtualScroll.ts` & `routes/session/index.tsx`：滚动读取当前已挂载几何；用户滚离底部时锁定视口，不被底部流式拉扯。 | **鼠标读取就绪快照 + 屏幕外流式节流**：鼠标滚轮/拖拽滚动条/Hover 读取 `PaneCache` 已就绪快照；当 `!follow_bottom` 且变化仅发生在视口下方流式块时，快照重建节流至 80ms 一次。 |

---

## 3. 目标与非目标

### 3.1 目标

1. **思考流式零开销**：模型输出任意长度思考（哪怕 50,000 字）期间，无论用户是否点击过展开，流式阶段只展示 `○ Thinking` 单行头部，`HistorySnapshot` 重建次数为 **0**，Markdown 解析次数为 **0**；思考完成瞬间若处于展开态则一次性完成静态渲染。
2. **正文流式 $O(\text{tail})$ 增量排版**：流式正文（含超长围栏代码块、列表、段落）每帧只解析新增的稳定行与尾部未闭合区，并通过 `Arc<Vec<MdLine>>` 原地扩展稳定前缀，消除整篇克隆与全量重排。
3. **二级缓存零残影堆积**：同一 `(message_index, step_index, width)` 在 `PaneCache.blocks` 中永远只占 1 个槽位，流式更新原地替换。
4. **滚动条拖拽与滚轮丝滑响应**：鼠标滚轮、滚动条拖拽、Hover 命中在流式期间零排版阻塞；用户向上滚离底部阅读历史时，屏幕下方的流式增量按 80ms 节流同步几何。
5. **严格遵守工程规范**：所有修改后的 `.rs` 生产文件 < 1000 行（拆分文件控制在 ~500 行），零 `unwrap()` / `expect()`，现有测试与新增基准测试全部通过。

### 3.2 非目标

- 不修改数据库表结构（`SCHEMA_VERSION` 不变）与消息持久化逻辑。
- 不修改已完成态下 Markdown 的视觉渲染结果（增量渲染结果与全量 `render_wrapped` 逐行逐样式完全一致）。
- 不改动终端 PTY 视图、右侧 Prompt 面板、设置页。

---

## 4. 总体架构与数据流

```
┌──────────────────────────────────────────────────────────────────────────────┐
│                              Event Loop (event/mod.rs)                       │
│                                                                              │
│  1. 鼠标事件 (wheel / drag / hover)                                          │
│     └─► history_hit::* ──► PaneCache::snapshot_ready_source()                │
│                            (同宽度直接返回已就绪 Arc<HistorySnapshot>，0 重排)│
│                                                                              │
│  2. 帧绘制前同步 (16ms FRAME_INTERVAL)                                       │
│     └─► resolve_lx_history() & lx::render()                                  │
│           └─► PaneCache::snapshot_source(width, source)                      │
│                 ├─ A. list_signature_source(source, width)                   │
│                 │     • Thinking 流式中/折叠中：忽略 content 与 ended_at_ms  │
│                 │       ──► 签名不变 ──► O(1) 直接返回缓存快照！             │
│                 ├─ B. 屏幕外流式节流检查 (offscreen_stream_throttled)        │
│                 │     • 若 !follow_bottom 且仅视口下方流式块变化且 < 80ms    │
│                 │       ──► 直接复用上一帧快照，不抢占滚动帧！               │
│                 └─ C. snapshot::build()                                      │
│                       └─► build_step()                                       │
│                             ├─ Reasoning:                                    │
│                             │    streaming || !expanded ──► 空 body (0 解析) │
│                             │    !streaming && expanded ──► 一次性缓存渲染   │
│                             └─ Text (Assistant):                             │
│                                  PaneCache::markdown_block()                 │
│                                  └─► IncrementalMarkdown::update()           │
│                                        ├─ 已完成行/代码行 ──► Arc::make_mut  │
│                                        │  追加到 stable_lines (零克隆旧行)   │
│                                        └─ 尾部未完成块 ──► 仅解析 tail       │
└──────────────────────────────────────────────────────────────────────────────┘
```

---

## 5. 核心机制设计一：思考块（Thinking）流式隔离与签名优化（Claude Code 模式）

### 5.1 思考正文可见性判定（`reasoning_body_visible`）

在 `HistorySource` / `LxChat` 的渲染口径上，思考块正文是否应当被排版并显示，取决于两个条件同时成立：
1. **该思考块已完成流式生成**：`!source.chat.reasoning_streaming(message_index, step_index)`
2. **用户已展开该思考块**：`source.chat.reasoning_expanded(message_index, step_index)`

定义辅助函数（位于 `src/ui/main_content/lx/history/cache/signature.rs` 与 `snapshot.rs` 共用）：

```rust
/// 思考块正文是否应当排版并展示（对齐 Claude Code：流式生成期间恒不流式渲染思考正文，
/// 待该思考块完成后若处于展开态再一次性静态渲染）。
pub(crate) fn reasoning_body_visible(
    source: super::source::HistorySource<'_>,
    message_index: usize,
    step_index: usize,
) -> bool {
    source.chat.reasoning_expanded(message_index, step_index)
        && !source.chat.reasoning_streaming(message_index, step_index)
}
```

### 5.2 思考块签名优化（`cache/signature.rs`）

修改 `step_signature` 中对 `PartKind::Reasoning` 的哈希策略：

```rust
AssistantStepView::Part { part, .. } => {
    hash.write_u8(0);
    match part.kind {
        crate::chat::events::PartKind::Text => {
            hash.write_u8(0);
            hash.write_usize(part.content.len());
            hash.write_edges(&part.content);
            if message.role == "user" {
                hash.write_u8(u8::from(source.chat.user_message_expanded(&message.id)));
                for (msg, slot, section, is_exp) in &source.chat.detail_expanded {
                    if msg == &message.id && *section == CardSection::Body && *is_exp {
                        hash.write_str(slot);
                    }
                }
            }
        }
        crate::chat::events::PartKind::Reasoning => {
            hash.write_u8(1);
            let streaming = source.chat.reasoning_streaming(message_index, step_index);
            let expanded = source.chat.reasoning_expanded(message_index, step_index);
            let body_visible = expanded && !streaming;
            hash.write_u8(u8::from(streaming));
            hash.write_u8(u8::from(expanded));
            hash.write_u8(u8::from(body_visible));
            if !streaming {
                // 仅在思考完成后，头部才显示 `Thought · 6.3s`（依赖起止时间戳）。
                // 流式期间只显示 `Thinking`，忽略每个 delta 刷新的 ended_at_ms！
                hash.write_i64(part.started_at_ms.unwrap_or(-1));
                hash.write_i64(part.ended_at_ms.unwrap_or(-1));
                hash.write_i64(part.completed_at_ms.unwrap_or(-1));
            }
            if body_visible {
                // 仅在思考已完成且展开时，才将正文字节纳入指纹触发一次性 Markdown 排版。
                hash.write_usize(part.content.len());
                hash.write_edges(&part.content);
            }
        }
    }
}
```

### 5.3 快照构建同步（`history/snapshot.rs`）

在 `build_step` 中：

```rust
crate::chat::events::PartKind::Reasoning => {
    let body_visible = super::cache::reasoning_body_visible(history, message_index, step_index);
    BlockPaint::Thinking {
        body: if body_visible {
            markdown_render::render_wrapped(
                &part.content,
                width.saturating_sub(super::STEP_GUTTER),
            )
        } else {
            Vec::new()
        },
    }
}
```

**效果验证**：
- 当模型流式输出 30,000 字思考时：`streaming` 为 `true`，`body_visible` 为 `false`。500 个思考 delta 到达期间，`step_signature` 和 `list_signature_source` **保持完全恒定**，`PaneCache::snapshot_source` 100% 命中 `self.snapshot`，**快照重建次数 = 0，Markdown 解析次数 = 0**。
- 当思考块完成（`completed_at_ms` 写入或进入下一阶段 step）：`streaming` 变为 `false`，签名变化 1 次：
  - 若未展开：仅更新头部为 `● Thought · Xs`，`body` 仍为空（0 次 Markdown 解析）；
  - 若用户在思考期间或之后点击了展开：`body_visible` 变为 `true`，执行 **1 次** `render_wrapped` 并缓存进 `PaneCache.blocks`。

---

## 6. 核心机制设计二：块级增量 Markdown 解析与零拷贝前缀复用

### 6.1 为什么 `lx-tui` 的 Markdown 渲染器天然适合精确增量解析？

分析 `src/ui/markdown_render.rs` 的 `render_blocks(source, width, expand_rules)`（第 113–192 行）：
- **单行独立块**：标题（`heading`）、水平线（`is_rule`）、图片（`image`）、普通段落（`inline_spans`）均按单行 `\n` 切分，行与行之间无回溯依赖。
- **跨行状态块 1 — 引用块（`blockquote`）**：每行 `>` 独立生成 `MdLine::hanging`，不依赖后续 `>` 行。
- **跨行状态块 2 — 列表项（`list_item`）**：仅依赖缩进栈 `list_indents: Vec<usize>`（遇到非空非列表行即清空 `list_indents.clear()`）。
- **跨行状态块 3 — 围栏代码块（`fenced_code`）**：由于 `lx` 移除了代码块的上下边框盒与语言头（`render_code` 第 298–307 行），围栏代码块内的每一行完整代码都是独立调用 `MdLine::spans(0, highlight::highlight_line(lang, line))` 再经 `wrap_md_line` 折行！
- **仅有的两个需要前瞻闭合的块**：
  1. **表格（`table::table`）**：连续以 `|` 开头的行，列宽取决于整张表的所有行，必须在遇到非 `|` 行后才定型。
  2. **缩进代码块（`indented_code`）**：连续 4 空格缩进行（中间允许夹空行），必须在遇到非缩进非空行后才定型。

### 6.2 增量解析状态机设计（`src/ui/markdown_render/incremental.rs`）

新增子模块 `src/ui/markdown_render/incremental.rs`，提供 `IncrementalMarkdownState`：

```rust
//! 流式 Markdown 增量渲染器：维护已定型行的 `Arc<Vec<MdLine>>` 稳定前缀，
//! 每帧仅推进新完成的稳定行并重排尾部未定型区（Active Tail）。

use std::sync::Arc;
use super::{MdLine, highlight, render_blocks, wrap_md_line};

/// 单个流式 Markdown 块的增量解析状态。
#[derive(Debug, Clone, Default)]
pub(crate) struct IncrementalMarkdownState {
    /// 上一次处理的渲染宽度（宽度变化时全量重置）。
    width: u16,
    /// 已固化进 `stable_lines` 的源码字节偏移（恒位于 `\n` 之后）。
    committed_bytes: usize,
    /// 已固化前缀的 FNV 指纹（用于校验新源码是否严格以前缀追加）。
    committed_hash: u64,
    /// 在 `committed_bytes` 处的列表缩进栈状态。
    list_indents: Vec<usize>,
    /// 在 `committed_bytes` 处若处于未闭合的 ``` 围栏代码块内，记录其语言标签（`""` 表示无语言）。
    open_fence_lang: Option<String>,
    /// 已固化的渲染行（跨帧通过 `Arc::make_mut` 原地追加，零克隆）。
    stable_lines: Arc<Vec<MdLine>>,
    /// `stable_lines` 的近似字节数账本（增量累加，避免每帧重扫）。
    stable_bytes: usize,
}
```

#### 稳定行推进算法（`advance_stable`）与尾部解析（`render_tail`）：

1. **前缀一致性校验**：
   - 检查 `width == self.width` 且 `source.len() >= self.committed_bytes` 且 `source.is_char_boundary(self.committed_bytes)` 且 `hash_prefix(&source[..self.committed_bytes]) == self.committed_hash`。
   - 若不满足（如宽度改变或内容被重写），清空 `self` 从偏移 `0` 开始。
2. **寻找最后一个换行符 `last_newline`**：
   - 在 `&source[self.committed_bytes..]` 中查找已经包含 `\n` 的完整行。
   - 最后一个 `\n` 之后的半行文本属于未完成行，永远留在 `tail` 中。
3. **扫描完整行推进 `committed_bytes`**：
   - 若当前处于 `open_fence_lang = Some(lang)`（围栏代码块内部）：
     - 逐行读取已换行的完整行 `line`：
     - 若 `line.trim_end().trim_start().starts_with("```")`：闭合围栏（`self.open_fence_lang = None`），推进 `committed_bytes`；
     - 否则：直接调用 `highlight::highlight_line(lang_opt, line)` + `wrap_md_line` 将生成的 `MdLine` 推入 `Arc::make_mut(&mut self.stable_lines)`，累加 `self.stable_bytes`，推进 `committed_bytes`！
     - **关键收益**：哪怕模型流式输出一个 1,000 行的巨型代码块，每一行代码只要换行就立即固化进 `stable_lines`，后续帧耗时恒为 $O(1)$！
   - 若当前处于普通顶层状态（`open_fence_lang == None`）：
     - 遇到开启围栏 `trimmed.trim_start().starts_with("```")` 的完整行：记录 `open_fence_lang = Some(lang)`，清空 `list_indents`，推进 `committed_bytes`；
     - 遇到表格行（`trimmed.trim_start().starts_with('|')`）或缩进代码行（`is_indented_code(line)`）：
       - 若该表格或缩进代码块在 `last_newline` 之前**尚未遇到结束行**（即一直延续到最后一个完整行），则**在此停住（Holdback）**，将该未闭合表格/缩进代码块留在 `tail` 中解析；
       - 若在后续完整行中已经遇到非表格/非缩进代码行（已闭合），则整块表格/缩进代码块一次性定型推入 `stable_lines`！
     - 遇到标题、分割线、图片、普通段落、列表项、单行引用：全部属于行级确定性语法，直接渲染并 `wrap_md_line` 推入 `stable_lines`，推进 `committed_bytes`！
4. **渲染剩余尾部 `tail`**：
   - 对剩余极短的 `&source[self.committed_bytes..]`（通常只有正在打字的最后半行，或末尾正在生长的一张小表格），用当前的 `(list_indents, open_fence_lang)` 状态渲染出 `tail_lines: Vec<MdLine>`。

### 6.3 `BlockPaint::Markdown` 双段结构（零拷贝绘制）

在 `src/ui/main_content/lx/history/paint.rs` 中，将 `BlockPaint::Markdown` 升级为前缀 `Arc` + 尾部 `Vec`：

```rust
pub(crate) enum BlockPaint {
    // ...
    /// 助手正文：已固化的稳定前缀 `prefix`（跨帧共享 `Arc`）+ 尾部动态行 `tail`。
    Markdown {
        prefix: Arc<Vec<MdLine>>,
        prefix_bytes: usize,
        tail: Vec<MdLine>,
    },
    // ...
}
```

对应的行数、字节数与绘制实现：

```rust
impl BlockPaint {
    pub(crate) fn rows(&self) -> usize {
        match self {
            // ...
            Self::Markdown { prefix, tail, .. } => prefix.len() + tail.len(),
            // ...
        }
    }

    pub(crate) fn bytes(&self) -> usize {
        match self {
            // ...
            Self::Markdown {
                prefix_bytes, tail, ..
            } => prefix_bytes.saturating_add(md_bytes(tail)),
            // ...
        }
    }
}
```

在 `draw_markdown`（`src/ui/main_content/lx/history/paint.rs:435`）中按视口偏移直接索引 `prefix` 或 `tail`：

```rust
fn draw_markdown(
    grid: &mut Grid,
    rect: Rect,
    history: HistorySource<'_>,
    source: BlockSource,
    scroll: usize,
    start: usize,
    viewport_end: usize,
    prefix: &[MdLine],
    tail: &[MdLine],
) {
    // ...
    let total = prefix.len().saturating_add(tail.len());
    for offset in scroll.saturating_sub(start)..total {
        let row = start.saturating_add(offset);
        if row >= viewport_end {
            break;
        }
        let Some(y) = visible_row(rect, scroll, row) else {
            continue;
        };
        let line = if offset < prefix.len() {
            &prefix[offset]
        } else {
            &tail[offset - prefix.len()]
        };
        // 绘制左侧步骤圆点与该行 spans（与原逻辑完全一致）
    }
}
```

---

## 7. 核心机制设计三：`PaneCache` 同槽位原地覆写与零克隆生命周期

### 7.1 消除流式 `BlockKey` 残影堆积（`history/cache.rs`）

原实现中，`self.blocks` 的键包含了 `sig: u64`：
- 同一个流式步骤 `(message_index, step_index, width)` 在 500 帧里插入了 500 个不同 key，旧版本全部滞留在 `self.blocks` 中。
- 事实上，对于给定的窗格和宽度，**同一个 `(message_index, step_index, width)` 在同一时刻只需要缓存当前最新的 `sig` 对应的绘制数据**！

改造方案：
1. 新增槽位键 `BlockSlot` 作为 `PaneCache.blocks` 的 `HashMap` 键，而把 `sig: u64` 存入 `BlockEntry` 内部：

```rust
/// 块在缓存中的物理槽位（不含内容指纹 `sig`；同一槽位内容更新时原地覆写）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) struct BlockSlot {
    pub(crate) message_index: usize,
    pub(crate) step_index: usize,
    pub(crate) width: u16,
}

#[derive(Debug)]
struct BlockEntry {
    /// 当前缓存内容对应的指纹；命中时校验 `entry.sig == key.sig`。
    sig: u64,
    paint: Arc<BlockPaint>,
    bytes: usize,
    last_frame: u64,
}
```

2. 当 `PaneCache::block(key, build)` 被调用时：
   - 以 `key.slot()` 查找 `self.blocks.get(&slot)`；
   - 若存在且 `entry.sig == key.sig`：直接命中返回 `Arc::clone(&entry.paint)`；
   - 若未命中或 `entry.sig != key.sig`：构建新 `paint`，插入同一 `slot` 自动替换旧 `BlockEntry`，并立即 `self.bytes = self.bytes.saturating_sub(previous.bytes).saturating_add(bytes)`！
   - **效果**：无论流式输出经历 100 次还是 10,000 次 delta，该步骤在 `self.blocks` 中永远只占 **1 个条目**，内存账本精确更新，彻底杜绝流式期间的 `evict()` 全表扫描风暴！

### 7.2 保证 `Arc::make_mut(&mut stable_lines)` 引用计数恒为 1

为了让增量 Markdown 在推进 `stable_lines: Arc<Vec<MdLine>>` 时做到 **零克隆**：
1. 在 `PaneCache::snapshot_source` 重建新快照前，先执行 `self.snapshot = None;`，释放上一帧快照持有的所有 `Arc<BlockPaint>` 引用；
2. 在 `PaneCache::markdown_block(key, content)` 中，先从 `self.blocks` 移除该 `slot` 的旧 `BlockEntry`（使其持有的 `Arc<BlockPaint>` 引用立即释放）；
3. 此时只有 `PaneCache.md_streams` 持有该槽位的 `stable_lines: Arc<Vec<MdLine>>`，其强引用计数 `Arc::strong_count` **恒等于 1**！
4. 调用 `Arc::make_mut(&mut state.stable_lines)` 时，Rust 标准库直接在原堆内存上原地 `push` 新增行，**0 次内存拷贝**！

---

## 8. 核心机制设计四：滚动条拖拽、滚轮与屏幕外流式解耦

### 8.1 鼠标事件走已就绪快照（`PaneCache::snapshot_ready_source`）

在 `PaneCache` 中新增 `snapshot_ready_source` 与 `snapshot_ready`：

```rust
/// 读取当前宽度下已就绪的几何快照（供高频鼠标滚轮、滚动条拖拽、Hover 命中调用）。
///
/// 若当前已存在同宽度的快照，直接 O(1) 克隆 `Arc` 返回，绝不在鼠标事件回调内
/// 触发同步签名扫描与 Markdown 重排；仅在冷启动或宽度变化尚未构建快照时回退构建。
pub(crate) fn snapshot_ready_source(
    &mut self,
    width: u16,
    source: super::source::HistorySource<'_>,
) -> Arc<HistorySnapshot> {
    if self.snapshot_width == Some(width)
        && let Some((_, snapshot)) = &self.snapshot
    {
        return Arc::clone(snapshot);
    }
    self.snapshot_source(width, source)
}
```

在 `src/ui/main_content/lx/history.rs` / `history_hit.rs` / `selection.rs` 中：
- 新增只读快速视图入口（`history::view_ready` + `history_hit::history_view_ready`，内部使用 `snapshot_ready_source`），供以下高频鼠标交互路径使用：
  - `history_scrollbar`（滚动条拖拽 `drag.rs:237` 与按下检测）
  - `history_rows`（滚轮滚动 `wheel.rs:553`）
  - `fold_target_at`（鼠标移动悬停 `hover.rs:251`）
  - `content_geom`（拖选滚轮/拖选移动/按下开始选区，`wheel.rs` / `drag.rs` / `press.rs`）
- 每一帧绘制前的 `resolve_lx_history`（`event/mod.rs`）与正式绘制 `lx::render`（`lx.rs`）仍然走完整校验签名的 `history::view`（`snapshot_source`）。
- 若 `self.snapshot` 尚未初始化（如单元测试直接调 `history_scrollbar` 而未先 `render`），`snapshot_ready_source` 自动回退到 `snapshot_source`，保证所有单元测试 100% 兼容。
- Agents 详情区的选区几何优先读 `resolve_task_agents_visible` 每帧维护的 `run.total_rows` / `run.scroll`（与绘制同口径）；仅在 `rows_dirty` 或尚未量行时回退完整视图。

### 8.2 用户滚离底部时的屏幕外流式节流（Offscreen Stream Coalescing）

当用户向上滚动阅读旧消息（`!source.chat.follow_bottom`）时：
- 用户的视口停留在上方 `scroll..scroll + visible_height`。
- 如果底部流式消息正在输出，且该流式消息的起始行已经位于当前视口底边之下（屏幕外不可见），每 16ms 重建一次整表 `HistorySnapshot` 只会微调滚动条总分母 `total_rows`，对当前视口内的画面像素没有任何影响。
- 在 `PaneCache` 中记录：
  - `snapshot_non_stream: u64`（除正在流式的最后一条消息外，其余所有历史状态与视口宽度的指纹）
  - `snapshot_at: Option<Instant>`
  - `viewport_rows: usize`（由 `history::view` 每帧写入）
- `snapshot_source` 命中以下全部条件时直接复用当前 `Arc<HistorySnapshot>`：
  - `source.coalesce_offscreen`（主聊天为 true；Agents 详情滚动位在 `SubagentRun` 上，必须关闭）；
  - `!source.chat.follow_bottom`；
  - 已有同宽度快照，`non_stream_signature_source` 与构建时一致（只有流式末条消息在变）；
  - 快照末条消息起始行 `last.start >= history_scroll + viewport_rows`（整块在视口下方）；
  - 距上次构建不足 `OFFSCREEN_STREAM_COALESCE`（生产 80ms）。
- 测试构建下默认窗口为 `Duration::ZERO`（关闭节流），避免时间敏感断言；单测通过
  `set_offscreen_coalesce` 显式打开窗口验证合并行为。

---

## 9. 模块拆分与文件行数控制方案

遵循 `docs/standards/code-writing-standards.md`（生产文件上限 1000 行，拆分后子文件控制在 500 行左右）：

| 文件路径 | 改造前 | 实施后 | 职责说明 |
|----------|--------|--------|----------|
| `src/ui/main_content/lx/history/cache.rs` | 924 行 | 433 行 | `RenderCache`、`PaneCache`、`BlockSlot`、`BlockKey`、原地覆写二级缓存、增量 Markdown 缓存驱动、就绪快照与屏幕外流式节流 |
| `src/ui/main_content/lx/history/cache/signature.rs` | （新建） | 314 行 | `Fnv`、`list_signature_source`、`non_stream_signature_source`、`step_signature`（含思考块签名优化）、`reasoning_body_visible`、`shell_signature` |
| `src/ui/main_content/lx/history/cache/tool_signature.rs` | （新建） | 289 行 | 工具调用指纹（含全部 `ToolDetailView` 分支） |
| `src/ui/main_content/lx/history/cache/block_signature.rs` | （新建） | 120 行 | 命令卡/汇报/通知/统计/撤销/compact/模型切换等单块指纹 |
| `src/ui/main_content/lx/history/snapshot.rs` | 863 行 | 882 行 | 快照构建；助手正文接入 `PaneCache::markdown_block`，思考块接入 `reasoning_body_visible` |
| `src/ui/main_content/lx/history/paint.rs` | 702 行 | 701 行 | `BlockPaint::Markdown { prefix, prefix_bytes, tail }` 双段绘制 |
| `src/ui/main_content/lx/history.rs` | 455 行 | 494 行 | 新增 `view_ready` / `view_source_ready` / `view_source_plain_ready` |
| `src/ui/main_content/lx/history_hit.rs` | 419 行 | 437 行 | 滚轮/滚动条/Hover/点击接入就绪快照 |
| `src/ui/markdown_render.rs` | 729 行 | 786 行 | 块级状态机 `ParserState` 抽离，暴露 `md_bytes` 与增量渲染入口 |
| `src/ui/markdown_render/incremental.rs` | （新建） | 215 行 | `IncrementalMarkdownState`：稳定前缀推进、围栏代码行直推、表格/缩进代码 Holdback |
| `src/ui/markdown_render/incremental/tests.rs` | （新建） | 116 行 | 逐字节等价性、未闭合块、长围栏、重写、宽度变化回归 |

---

## 10. 测试与性能验证方案

### 10.1 单元测试与等价性回归测试

1. **增量 Markdown 等价性测试（`src/ui/markdown_render/incremental/tests.rs`）**：
   - 将包含六级标题、粗斜体/行内代码/链接、嵌套无序/有序/任务列表、多行引用、围栏代码块（带语言高亮）、缩进代码块、方框表格、分割线的复杂 Markdown 文本，**逐字节**喂给 `IncrementalMarkdownState`（覆盖 8/20/45/96 四档宽度），断言**每一步**产出的 `prefix + tail` 与全量调用 `render_wrapped` 的结果**逐行逐 `Style` 100% 相等**。
   - 另覆盖流式中的未闭合围栏/表格/缩进代码块、120 行长围栏逐行固化、内容整体重写与宽度变化重置。
2. **思考块流式隔离测试（`src/ui/main_content/lx/history/cache/tests.rs`）**：
   - `streaming_thinking_deltas_do_not_rebuild`：`reasoning_streaming == true` 时连续追加 100 次思考文本并刷新时间戳，断言快照重建次数不增加；展开只重建一次且总行数不变（仍只有单行头部）；思考完成后展开态一次性渲染全文。
3. **同槽位原地覆写测试（`src/ui/main_content/lx/history/cache/tests.rs`）**：
   - `streaming_text_replaces_one_block_slot`：同一条流式 `PartKind::Text` 消息连续追加 100 次增量，断言 `cache.block_count() == 1`，不产生历史残影。
4. **就绪快照与屏幕外合并测试（`src/ui/main_content/lx/history/cache/tests.rs`）**：
   - `ready_snapshot_does_not_rebuild`：内容已变时 `snapshot_ready_source` 仍返回同一 `Arc` 且不重建；
   - `offscreen_stream_coalesces_within_window`：滚离底部且流式消息在视口下方时窗口内增量合并，其他消息变化立即重建。
5. **更新既有测试断言（`src/ui/main_content/lx/history/tests/transcript.rs`）**：
   - `transcript_lays_out_step_marks_with_two_column_gutter` 的思考块补上 `completed_at_ms`（真实流式下进入正文/工具步骤时前序思考已关闭），展开态仍渲染思考正文。
6. **评审补充测试（`review.md` P1/P2）**：
   - `selection_geometry_reads_ready_snapshot`：内容变化后选区几何不触发快照重建；
   - `long_markdown_scroll_reads_prefix_and_tail`：80 行稳定前缀 + 尾部表格，滚动到交界处断言实际绘制的行；
   - `randomized_block_sequences_match_full_render`：确定性伪随机块序列（标题/列表/引用/围栏/表格/缩进代码/空行）按采样步长验证 `prefix + tail == render_wrapped`。

### 10.2 Release 性能基准测试（`src/ui/main_content/lx/history/tests/perf.rs`）

新增两条 `#[ignore]` 性能回归测试：
1. `perf_lx_long_thinking_and_streaming`：
   - 在已有大历史会话基础上，模拟单条 **30,000 字符长思考（500 帧流式增量）** + **1,500 行 Markdown 与代码块长正文（300 帧流式增量）**，同时交织模拟滚动条拖拽与视口滚动；
   - **断言**：
     - 500 帧思考流式期间快照重建次数 `thinking_rebuilds == 0`，P95 ≤ 2ms；
     - 300 帧长正文流式 + 滚动期间单帧 P95 ≤ 8ms（远低于 16ms 预算）；
     - 二级缓存占用受控，同一流式块槽位数恒为 1。

---

## 11. 实施结果与实测（2026-11）

### 11.1 完成内容

- 思考块按 Claude Code 口径隔离：流式期间 `reasoning_body_visible == false`，正文与时间戳不进入指纹；`streaming_thinking_deltas_do_not_rebuild` 验证 200 次思考 delta 期间快照重建次数不增加。
- 增量 Markdown：`ParserState` 块级状态机 + 安全边界固化；表格/缩进代码块未闭合时 Holdback，围栏代码行换行即固化；逐字节等价性测试通过。
- `PaneCache.blocks` 改为 `BlockSlot` 槽位键 + `BlockEntry.sig`，流式块原地覆写；`md_streams` 与块同生命周期。
- 鼠标滚轮/滚动条拖拽/Hover/点击/**选区几何**读取 `snapshot_ready_source`；滚离底部且流式消息在视口下方时按 80ms（生产）合并快照，Agents 详情显式关闭合并（滚动位在 `SubagentRun`）。
- 评审补强（详见 `review.md`）：修复选区几何漏走就绪快照；性能夹具恢复 102,900 行；新增前缀/尾部交界像素级回归与随机块序列等价性测试。

### 11.2 Release 实测

| 场景 | 规模 | 优化后结果 | 预算 |
|------|------|------------|------|
| 长思考流式 | 30,000 字符 / 500 帧 | P95 ≈ **0.11ms**，重建 **0** 次 | ≤ 2ms |
| 长正文流式 + 滚动 | 1,500 行 / 300 帧 | P95 ≈ **0.19ms** | ≤ 8ms |
| 大历史热缓存滚动 | 102,900 行 / 120 帧 | P95 ≈ **1.92ms** | ≤ 16ms |
| 大历史流式追加 | 120 帧 | P95 ≈ **3.72ms** | ≤ 16ms |
| 展开 2000 行文件差异滚动 | 2,106 行 / 120 帧 | P95 ≈ **6.29ms** | ≤ 16ms |

命令：

```
cargo test --release --lib -- --ignored perf_lx --nocapture
```

> 说明：`perf_lx_history` 的性能夹具在用户消息折叠后只剩 64,400 行，原「> 90,000」
> 断言在 dev 分支已失效；评审修复将 `BODY_LINES` 调回 125（102,900 行）恢复原有
> 压力规模，而不是降低阈值。
