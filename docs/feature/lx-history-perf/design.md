# lx 消息列表滚动性能优化与工具输出高亮调整设计

> 状态：**已实现并合并到 `dev`**（`03b1ab1`）。实现偏差与实测见 §13 与
> `task.md` 的「性能记录」，评审结论与修复清单见 `review.md`。
>
> 本文所有“当前代码位置”均以 2026-10-25 的 `dev` 工作区为准。

## 0. 已确认的决策

| # | 决策 | 结论 |
|---|------|------|
| 1 | 验收规模与指标 | 1000 条消息 / 约 10 万渲染行；滚轮或拖滚动条时单帧 `draw` P95 ≤16ms；仓库内保留 `#[ignore]` 性能回归测试 |
| 2 | 缓存位置 | UI 层显式 `RenderCache`，由事件循环持有，显式传入 `ui::render` 与鼠标命中入口；不放 `LxChat`、不用 `RefCell` |
| 3 | 失效方式 | 每帧计算 O(步骤数) 的结构指纹；不新增 `view_rev` 字段 |
| 4 | 高亮语义 | 折叠态恒为暗灰、鼠标悬停不变色；展开态用终端默认前景；`… N more lines` 跟随折叠态保持暗灰；thinking 块保留悬停/展开亮粉 |
| 5 | 缓存粒度 | 整段历史快照（时间线 + 每块排好的行 + 卡片行区间 + 总行数），按“列表指纹 + 宽度 + 折叠状态”复用；每帧只绘制视口内的行 |
| 6 | 范围 | 只改 lx 消息列表及其滚动/命中/置顶/滚动条；后台任务面板、提示词历史、侧栏列表不动 |
| 7 | 内存 | 两级缓存：几何/行数常驻；渲染行 32MB LRU，视口内不淘汰 |
| 8 | 验证 | 现有几何测试不改断言；新增缓存单测；新增 `#[ignore]` release 性能测试；交互手感由用户 `cargo run` 确认 |

## 1. 问题与根因

### 1.1 现象

会话消息一多，滚轮滚动、拖历史滚动条、鼠标在消息列表上移动都会出现明显掉帧；消息越多越卡。

### 1.2 根因：每帧把整段历史重新排版多次

当前实现没有任何跨帧布局缓存。`history::layout` 对**全部**消息执行：

- 助手正文：`markdown_render::render_wrapped(content, width)`（完整 markdown 解析 + 词法高亮 + 折行）；
- 用户正文：`wrap(content, width)`（逐字符折行并分配 `Vec<String>`）；
- 每个工具卡片：`tool::rows` → 重新组装完整参数/输出/差异正文；
- 每个本地命令卡片：`user_shell::rows` → 重新组装参数与输出；
- 每条消息的统计行、通知、compact/rewind 条目。

`history::render` 又把这些工作重做一遍；屏幕外的消息也照常解析，`visible_row` 只拦住写单元格，没拦住解析。

### 1.3 一帧内的重复次数

| 触发路径 | 全量 `layout` 次数 | 说明 |
|----------|-------------------|------|
| 绘制前一帧 `resolve_lx_sticky_prompt` | 1 | `history_view` → `history::view` → `layout` |
| 绘制前一帧 `resolve_lx_viewport_top` | 1 | 同上，重复算一遍 |
| `ui::render` → `build_chat` → `history::view` | 1 | 同上，第三次 |
| `history::render` | 1 | 行数再算一遍；可见正文在 `draw_body` 里再解析一次 |
| 鼠标移动 `fold_target_at` | 2~3 | `card_at` 内部一次 `history_view` + 一次 `history::layout`，外层再一次 `history_view` |
| 滚轮 `history_rows` | 1 | 事件处理阶段先全量算一次 |

结果：鼠标移动一帧最多 **7 次全量排版**；滚轮每来一个事件（触控板一秒可发几十上百个）至少一次全量排版，再叠加绘制路径的 4 次。

### 1.4 关键代码证据

- `src/ui/main_content/lx/history.rs:277` `layout()`：遍历 `timeline(lx)`，逐消息、逐步骤算行数。
- `src/ui/main_content/lx/history.rs:667` `render()`：再次遍历全部消息；正文行数用 `text_rows`，可见时 `body::draw_body` 又调一次 `render_wrapped`。
- `src/ui/main_content/lx/history/rows.rs:32-59`：`text_rows` / `tool_rows` 是每次全量解析的入口。
- `src/ui/main_content/lx/history/body.rs:48`：`draw_body` 内部 `markdown_render::render_wrapped(content, inner)`。
- `src/ui/main_content/lx.rs:711-726`：`build_chat` 先 `history::view`，再 `history::render`。
- `src/app/update/chat/viewport.rs:107-132`、`179-205`：两个 resolve 函数各自算一次 `history_view`。
- `src/ui/main_content/lx/history_hit.rs:142-179`：`card_at` 与 `fold_target_at` 重复调用 `history_view` / `history::layout`。
- `src/event/mouse/wheel.rs:319-331`：滚轮先 `history_rows` 全量算，再置脏触发重绘。
- `src/ui/markdown_render.rs:91-98`：`render_wrapped` 无缓存，每次从头解析。

### 1.5 codex 的对照做法

- `codex-rs/tui/src/transcript_view/layout.rs:61` `LayoutCache`：按 `Weak<dyn HistoryCell>` 身份 + 宽度 + `CellPresentation` + 动画 tick 缓存 `Arc<TextLayout>`。
- `codex-rs/tui/src/transcript_view.rs:247-287`：`render` 只遍历 `area.top()..area.bottom()`，每个可见行单独 `layout.render(row_area, buf, row)`；屏幕外的 cell 不排版。
- `codex-rs/tui/src/history_cell/markdown_render_cache.rs`：markdown 结果按宽度 + 终端配色缓存。
- 缓存上限：`MAX_CACHED_ENTRIES = 64`、`MAX_CACHED_TEXT_BYTES = 8MB`（`layout.rs:13-14`）。

本次采用同一思路，但适配 lx 的纯函数式 `layout/render`：把“每块排好的行”做成显式快照，避免 codex 的 trait/cell 体系整体引入。

## 2. 目标与非目标

### 2.1 目标

1. 历史区每帧成本从 **O(全部历史字节)** 降到 **O(消息数 + 步骤数 + 视口行数)**。
2. 滚动、拖滚动条、鼠标悬停时不再重复解析 markdown/工具输出。
3. 工具输出折叠态不再随 hover 高亮；展开态文字保持现有默认前景。
4. 所有现有几何测试（行数、滚动条、置顶行、thinking 命中、卡片命中）保持通过。
5. 输出视觉除第 3 条外与现状逐字符一致。

### 2.2 非目标

- 不改后台任务面板、提示词历史、侧栏、输入框的渲染策略。
- 不引入 codex 的 `HistoryCell` trait 体系，不重写整个 UI 渲染层。
- 不改滚动步长、帧率上限、终端 diff 提交方式。
- 不改数据库结构、不新增持久化字段。

## 3. 总体架构

```
event loop
  ├─ 持有 ui::RenderCache（跨帧）
  ├─ 命中/滚轮：传入 &mut RenderCache → PaneCache.snapshot
  ├─ resolve_lx_history：一次 snapshot + view，写 viewport_top / sticky 抑制
  └─ tui.draw → ui::render(..., &mut RenderCache)
                    └─ panes → main_content → lx::render → history::render
                                              └─ 只画视口内的块
```

`RenderCache` 是 UI 层对象，不进入 `AppState`。渲染仍然只读 `Lx`；缓存只保存从 `Lx` 推导出的纯数据。

## 4. 缓存设计

### 4.1 模块落点

新增 `src/ui/main_content/lx/history/cache.rs`：

```rust
/// 跨帧渲染缓存：事件循环持有，按窗格隔离。
#[derive(Debug, Default)]
pub(crate) struct RenderCache {
    frame: u64,
    panes: HashMap<PaneId, PaneCache>,
}

impl RenderCache {
    /// 每帧开始时推进帧号；用于 LRU 与“本帧使用过”的判定。
    pub(crate) fn begin_frame(&mut self) {
        self.frame = self.frame.wrapping_add(1);
    }

    pub(crate) fn pane(&mut self, id: PaneId) -> &mut PaneCache {
        let frame = self.frame;
        self.panes
            .entry(id)
            .or_insert_with(|| PaneCache::new(frame))
    }

    /// 窗格关闭/标签切换后清理，避免缓存泄漏。
    pub(crate) fn retain_panes(&mut self, alive: &[PaneId]) {
        self.panes.retain(|id, _| alive.contains(id));
    }
}
```

`PaneCache` 持有两级数据：

```rust
pub(crate) struct PaneCache {
    frame: u64,
    snapshot: Option<(u64, Arc<HistorySnapshot>)>, // (列表指纹, 快照)
    blocks: HashMap<BlockKey, BlockEntry>,         // 第二级：渲染行
    bytes: usize,
}

struct BlockEntry {
    paint: Arc<BlockPaint>,
    bytes: usize,
    last_frame: u64,
}
```

### 4.2 两级缓存

| 级别 | 内容 | 生命周期 |
|------|------|----------|
| 第一级（几何） | 每条消息/每个块的总行数、起止行、卡片与 thinking 的内容行区间 | 随快照常驻；1000 条消息也只有几百 KB |
| 第二级（绘制数据） | 排好的 `MdLine` / `String` 行 / `BodyRow` / 文件差异行 / 本地命令 sections | 32MB LRU，视口内不淘汰 |

第二级被淘汰时，第一级行数仍然有效，滚动条和置顶几何不会跳；滚回该消息时重新排版一次。

### 4.3 快照与块

```rust
pub(crate) struct HistorySnapshot {
    /// 内容行空间（0 = 最早），不含屏幕坐标与滚动偏移。
    pub(crate) entries: Vec<SnapEntry>,
    pub(crate) messages: Vec<MessageRows>,
    pub(crate) thinking: Vec<ThinkingRegion>,
    pub(crate) cards: Vec<CardRegion>,
    pub(crate) total_rows: usize,
}

pub(crate) struct SnapEntry {
    pub(crate) start: usize,
    pub(crate) end: usize,
    pub(crate) blocks: Vec<SnapBlock>,
}

pub(crate) struct SnapBlock {
    pub(crate) start: usize,
    pub(crate) paint: Arc<BlockPaint>,
}
```

`BlockPaint` 是绘制数据的总入口，**只存内容与静态样式，不存动态字形/hover**：

```rust
pub(crate) enum BlockPaint {
    /// 用户消息：软换行后的纯文本行。
    UserText { lines: Vec<String> },
    /// 助手正文/思考：markdown 渲染后的行。
    Markdown { lines: Vec<MdLine> },
    /// thinking 头 + 展开正文。
    Thinking { label: String, body: Vec<MdLine> },
    /// 工具卡片（命令/文件/计划/差异）。
    Tool(Arc<ToolPaint>),
    /// 本地命令卡片。
    Shell(Arc<ShellPaint>),
    /// 失败/中断通知。
    Notice { title: &'static str, reason: Vec<String> },
    /// 统计行。
    Stats { lines: Vec<String> },
}

impl BlockPaint {
    pub(crate) fn rows(&self) -> usize { /* 每个变体返回已排好的行数 */ }
}
```

动态内容在 `draw` 时叠加：
- 运行/完成方块字形（`StepMark::glyph(loading_frame, animating)`）；
- thinking 的 hover 亮粉；
- 用户消息的 rewind 选中底色；
- 文件差异展开态的 `chat_expanded_bg`（如果行数据里需要底色，缓存行文本、绘制时补 fill）。

### 4.4 指纹与失效

#### 列表指纹

```rust
fn list_signature(lx: &Lx, width: u16) -> u64 {
    let mut h = Fnv::new();
    h.write_u16(width);
    h.write_usize(lx.chat.messages.len());
    h.write_usize(lx.chat.notices.len());
    h.write_usize(lx.compact.checkpoints.len());
    for (index, message) in lx.chat.messages.iter().enumerate() {
        h.write_usize(index);
        h.write_u64(hash_str(&message.id));
        h.write_i64(message.sequence);
        h.write_u8(role_kind(message));
        h.write_u8(status_kind(&message.status));
        for (step_index, step) in message.steps.iter().enumerate() {
            h.write_u64(step_signature(lx, message, step_index, step, width));
        }
    }
    h.finish()
}
```

任何块的内容变化都会改变列表指纹，从而重建快照；快照重建时每个块仍按 `BlockKey` 命中第二级缓存，只有真正变化的块会重新排版。

#### 块指纹（O(步骤数)，不扫全文字节）

```rust
fn step_signature(lx: &Lx, message: &ChatMessageView, step: usize, item: &AssistantStepView, width: u16) -> u64 {
    match item {
        AssistantStepView::Part { part, .. } => {
            let mut h = Fnv::new();
            h.write_u8(kind_of(part));
            h.write_usize(part.content.len());
            write_edges(&mut h, &part.content);        // 首尾各 64 字节
            h.write_i64(part.started_at_ms.unwrap_or(-1));
            h.write_i64(part.completed_at_ms.unwrap_or(-1));
            h.write_u8(u8::from(lx.chat.reasoning_expanded(message_index, step)));
            h.write_u16(width);
            h.finish()
        }
        AssistantStepView::Tool { call, .. } => tool_signature(lx, message, call, width),
    }
}

fn tool_signature(lx: &Lx, message: &ChatMessageView, call: &ToolCallView, width: u16) -> u64 {
    let mut h = Fnv::new();
    h.write_u64(hash_str(&call.call_id));
    h.write_u64(hash_str(&call.name));
    h.write_u8(call.status as u8);
    h.write_u16(width);
    h.write_u32(fold_mask(lx, message, call));       // 参数区/输出区展开状态
    match call.detail.as_deref() {
        Some(ToolDetailView::Command(view)) => {
            h.write_usize(view.command.len());
            h.write_usize(view.output_tail.len());
            write_edges(&mut h, &view.output_tail);
            h.write_u64(view.dropped_bytes);
            h.write_u8(view.state.kind());           // Running / Exited / ...
            h.write_u64(view.process_session_id.unwrap_or(0));
        }
        Some(ToolDetailView::File(diff)) => {
            // 差异正文在加载后不变；指纹覆盖 hunk 数与每行长度即可。
            for hunk in &diff.hunks { h.write_usize(hunk.lines.len()); }
        }
        Some(ToolDetailView::Plan(plan)) => {
            h.write_u64(plan.revision);
            h.write_usize(plan.steps.len());
            for (step, status) in &plan.steps {
                h.write_usize(step.len());
                h.write_u8(status_kind(status));
            }
        }
        Some(ToolDetailView::Interaction(view)) => { h.write_u64(view.kind()); }
        None => {}
    }
    h.finish()
}
```

`write_edges` 只写首尾各 64 字节：

```rust
fn write_edges(h: &mut Fnv, text: &str) {
    let bytes = text.as_bytes();
    let head = &bytes[..bytes.len().min(64)];
    let tail = &bytes[bytes.len().saturating_sub(64)..];
    h.write(head);
    h.write(tail);
}
```

为什么首尾字节足够：
- 流式正文只追加 → 长度 + 尾部必变；
- 进程输出环形缓冲满容量时长度可能不变，但新输出从尾部进入、头部被裁掉 → 首尾必变；
- 会话重载/回退会改变消息 id 或列表长度 → 列表指纹变。

已知残余风险：某段内容被“等长改写且首尾 64 字节完全相同”。当前生产路径没有这种写入；测试中直接赋值 `part.content` 的场景会因测试单独构造新 `Lx` 而不受影响。若未来出现，只需把该字段加入指纹并补一条单测。

### 4.5 淘汰策略

```rust
const MAX_CACHED_BYTES: usize = 32 * 1024 * 1024;

fn evict(&mut self) {
    while self.bytes > MAX_CACHED_BYTES {
        let frame = self.frame;
        // 本帧用过的块不淘汰：正在视口里画的内容重排会直接掉帧。
        let Some(victim) = self
            .blocks
            .iter()
            .filter(|(_, entry)| entry.last_frame < frame)
            .min_by_key(|(_, entry)| entry.last_frame)
            .map(|(key, _)| *key)
        else {
            break;
        };
        if let Some(entry) = self.blocks.remove(&victim) {
            self.bytes -= entry.bytes;
        }
    }
}
```

- 字节数按 `lines.capacity() * size_of::<行类型>()` 近似统计；不追求精确，只要不失控。
- 视口命中的块在 `render` 时刷新 `last_frame`。
- 窗格关闭/工作区切换：`RenderCache::retain_panes` 清理。
- 会话切换（`session_id` 改变或 `messages` 被整体替换）：列表指纹必然变化，旧块按 LRU 自然淘汰；不额外做全清。

### 4.6 哪些内容不能进缓存

以下内容每帧可能变化，必须绘制时现算，不能烘进 `BlockPaint`：

- `StepMark::glyph(loading_frame, animating)`：方块呼吸字形；
- thinking 头部的 hover 亮粉；
- `rewind` 选中用户消息的底色；
- 光标/滚动条/置顶行；
- 任何 `Instant::now()` 相关文案（耗时已在状态里冻结，见 `turn_elapsed_ms`）。

## 5. 快照构建、布局、绘制、命中

### 5.1 构建快照

`history/snapshot.rs`：

```rust
pub(crate) fn build(area: Rect, lx: &Lx, cache: &mut PaneCache) -> HistorySnapshot {
    let content = content_rect(area);
    let width = content.width;
    let mut rows = 0usize;
    let mut entries = Vec::with_capacity(/* 时间线长度 */);
    let mut messages = Vec::new();
    let mut cards = Vec::new();
    let mut thinking = Vec::new();

    for entry in timeline(lx) {
        match entry {
            TimelineEntry::Message(index) => {
                let message = &lx.chat.messages[index];
                let start = rows;
                let blocks = build_message(message, index, width, lx, cache, &mut cards, &mut thinking);
                rows += blocks.iter().map(|b| b.paint.rows()).sum::<usize>();
                entries.push(SnapEntry { start, end: rows, blocks });
                messages.push(MessageRows { start, end: rows, is_user: /* 同现有口径 */ });
            }
            TimelineEntry::Notices { .. } | TimelineEntry::Compact(_) => { /* 按现有口径累加 */ }
        }
    }
    // 末尾呼吸行等与现有一致
    HistorySnapshot { entries, messages, thinking, cards, total_rows: rows }
}
```

行数口径必须与现状完全一致：块之间的空行、消息间空行、末尾呼吸行、嵌套列表/表格换行都由同一个 `BlockPaint::rows()` 给出。

### 5.2 `layout` 仍返回现有 `HistoryLayout`

为了不破坏测试与调用方，`history::layout(area, lx, cache)` 保留现有返回类型，把快照中的**内容行区间**映射成屏幕矩形：

```rust
pub(crate) fn layout(area: Rect, lx: &Lx, cache: &mut PaneCache) -> HistoryLayout {
    let snapshot = cache.snapshot(area, lx);
    let to_rect = |start: usize, rows: usize| Rect::new(
        area.x,
        area.y.saturating_add(rows_to_u16(start)),
        content_rect(area).width,
        rows_to_u16(rows),
    );
    HistoryLayout {
        area,
        thinking: snapshot.thinking.iter()
            .map(|r| (r.message_index, r.step_index, to_rect(r.start, 1)))
            .collect(),
        cards: snapshot.cards.iter()
            .map(|r| r.to_hit(to_rect))   // 只 clone id，不重算内容
            .collect(),
        messages: snapshot.messages.clone(),
        total_rows: snapshot.total_rows,
    }
}
```

`cards` 只有可折叠块（数量远小于消息数），每次 clone 少量 id 字符串可接受；真正昂贵的正文排版只在快照重建时发生。

### 5.3 `view` 与置顶行

`view` 逻辑保持不变（让位、重判、抑制记忆），只是把 `layout` 换成带缓存的版本：

```rust
pub(super) fn view(area: Rect, lx: &Lx, cache: &mut PaneCache) -> HistoryView {
    let layout = self::layout(area, lx, cache);
    // 以下与现状完全相同
}
```

### 5.4 绘制：只画视口内的块

`history/render.rs`：

```rust
pub(super) fn render(
    grid: &mut Grid,
    rect: Rect,
    lx: &Lx,
    scroll: usize,
    cache: &mut PaneCache,
) -> usize {
    let snapshot = cache.snapshot(rect, lx);
    let content = content_rect(rect);
    let height = usize::from(rect.height);
    let viewport = scroll..scroll.saturating_add(height);

    for entry in &snapshot.entries {
        if entry.end <= viewport.start || entry.start >= viewport.end {
            continue; // 整条在视口外：不加行、不排版、不绘字
        }
        for block in &entry.blocks {
            let start = block.start;
            let end = start + block.paint.rows();
            if end <= viewport.start || start >= viewport.end {
                continue;
            }
            cache.mark_used(block.key);      // 刷新 LRU
            block.paint.draw(grid, content, scroll, start);
        }
    }
    snapshot.total_rows
}
```

块内绘制同样做行窗口裁剪（例如一条超长命令输出只画视口内的几十行），避免“消息在视口内但正文几千行”时仍然逐行空转。

### 5.5 命中测试

`card_at` / `fold_target_at` / `chat_hit` / `history_rows` / `sticky_prompt_jump` / `history_scrollbar` 全部改为：

1. `history_view(area, lx, cache)` 取快照；
2. 把指针屏幕行换算成内容行：`content_row = view.scroll + (row - view.body.y)`；
3. 在 `snapshot.cards`（按 start 排序）里二分/线性查找。

悬停路径不再调用 `history::layout`，也不再重复 `history_view`。

## 6. 事件循环接入

### 6.1 绘制入口

```rust
// src/ui/mod.rs
pub fn render(frame: &mut Frame<'_>, state: &AppState, config: &Config, cache: &mut RenderCache) {
    ...
    let pane_cursor = panes::render_panes(frame, &pane_rects, state, cache);
}
```

`panes::render_panes` → `main_content::render` → `lx::render` → `history::view/render` 逐层透传 `&mut PaneCache`。测试中的 `ui::render` 调用改为传入 `&mut RenderCache::default()`。

### 6.2 resolve 合并

当前每帧调用两次 `history_view`：

```rust
update::resolve_lx_sticky_prompt(state, &geometry.rects);
update::resolve_lx_viewport_top(state, &geometry.rects);
```

合并为一次：

```rust
/// 一次几何计算，同时落 viewport_top 与置顶抑制记忆。
pub fn resolve_lx_history(
    state: &mut AppState,
    rects: &[(PaneId, Rect)],
    cache: &mut RenderCache,
) -> bool {
    let mut changed = false;
    for (pane, rect) in rects {
        let inner = crate::layout::pane_inner_rect(*rect);
        let Some(target) = state
            .pane_anywhere(*pane)
            .filter(|target| target.view == PaneView::Lx)
        else { continue };
        let cache_pane = cache.pane(*pane);
        let Some(view) = crate::ui::main_content::lx::history_view(inner, &target.lx, cache_pane)
        else { continue };
        let Some(target) = state.pane_mut_anywhere(*pane) else { continue };
        if target.lx.chat.viewport_top != Some(view.scroll) {
            target.lx.chat.viewport_top = Some(view.scroll);
            changed = true;
        }
        if target.lx.chat.suppressed_prompt_header != view.suppressed {
            target.lx.chat.suppressed_prompt_header = view.suppressed;
            changed = true;
        }
    }
    changed
}
```

对应测试 `app/update/chat/tests/history.rs` 的三处调用同步改名并传缓存。

### 6.3 鼠标入口

`event/mouse/{hover,wheel,press,drag,hit}.rs` 增加 `cache: &mut RenderCache` 参数（或通过事件循环上下文统一传入），命中函数从缓存取几何。事件循环里的 `MouseEvent` 分派处把同一个 `ui_cache` 传下去。

## 7. 工具输出高亮调整

### 7.1 折叠态不再随 hover 提亮

`src/ui/main_content/lx/history/tool.rs:324`：

```rust
// 现在
pub(super) fn output_style(expanded: bool, hovered: bool) -> Style {
    if expanded || hovered { style::text() } else { style::muted() }
}

// 改后
pub(super) fn output_style(expanded: bool) -> Style {
    if expanded { style::text() } else { style::muted() }
}
```

传播到：
- `tool/command.rs`：`finished_output(view, width, expanded, hovered)`、`guide_row(hovered)`、`blocks()` 去掉 hover 参数；
- `user_shell.rs`：`sections(..., hovered)`、`finished_output(..., hovered)` 去掉 hover 参数；
- `tool/file_summary.rs`：`detail(..., hovered)` 去掉 hover，末尾 `… N more lines` 提示恒用 `style::agent_step_detail()`；
- `tool/file_diff.rs`：`layout(..., hovered)` 去掉 hover，提示行不再提亮；文件头路径与 diff 正文本来就是恒色，不动。

### 7.2 hover 状态只保留 thinking

`history_hit.rs`：

```rust
pub fn fold_target_at(area: Rect, lx: &Lx, column: u16, row: u16, cache: &mut PaneCache) -> Option<CardTarget> {
    let hit = card_at(area, lx, column, row, cache)?;
    // 工具/本地命令输出不再有悬停态；只有 thinking 块提亮。
    matches!(hit.kind, history::CardKind::Thinking { .. }).then(|| target_of(&hit.kind))
}
```

随之删除不再使用的：
- `LxChat::tool_hovered_section` / `tool_is_hovered` / `shell_hovered_section`（`src/app/state/lx/chat.rs:561-587`）；
- `history.rs` 中 `shell_hovered_section` / `tool_hovered_section` 传参；
- 合并卡 `hovered` 查找逻辑。

保留：
- `thinking_is_hovered` 与 `StepKind::Thinking` 的 hover/expanded 亮粉；
- `card_target_at`（点击展开）不变；
- ~~`file_summary::foldable_rows` / `file_diff::folded_rows` / `tool::fold_region`~~：
  hover 提亮删除后已无消费者，**实现时整条折区已删除**（`review.md` §5）。

### 7.3 测试更新

以下测试的预期会**有意**改变，需要改写断言：

- `src/ui/main_content/lx/history/tool/file_diff/tests.rs:638` `file_diff_hover_region_is_the_whole_folded_block_but_only_the_hint_lifts`：提示行不再随 hover 提亮，改为断言“hover 前后样式完全相同”。
- 命令/本地命令 hover 提亮相关测试：改为“折叠态恒 muted，展开态恒 text”。
- `src/event/tests.rs` 中依赖 hover 提亮/命中面的用例：命中面仍保留（点击可用），只删掉样式提亮断言。
- thinking hover 测试保持不变。

## 8. 文件拆分

`history.rs` 当前 959 行，已接近 1000 行上限。本次顺带按职责拆：

| 文件 | 内容 | 预计行数 |
|------|------|----------|
| `history.rs` | 类型、`StepMark`/`StepKind`、`view`、`scrollbar`、`notice`、folds 辅助 | ~450 |
| `history/cache.rs` | `RenderCache`、`PaneCache`、指纹、LRU、块缓存访问器 | ~400 |
| `history/snapshot.rs` | `HistorySnapshot`、`build`、时间线遍历与行数累加 | ~450 |
| `history/layout.rs` | `layout()`：快照 → 屏幕矩形（保持现有返回类型） | ~200 |
| `history/render.rs` | `render()`：视口裁剪与块绘制 | ~350 |

`cache.rs` 可以再拆 `signature.rs`，但不必预先创建；超过 500 行再拆。

## 9. 兼容性与约束

1. **渲染只读状态**：`ui::render` 只接收 `&mut RenderCache`，不碰 `AppState`；`Lx` 的所有字段仍是不可变读取。
2. **几何唯一口径**：行数只由 `BlockPaint::rows()` / `snapshot.total_rows` 提供；`layout`、`render`、滚动条、置顶、命中全部读同一份快照。
3. **不做 IO**：缓存构建只做内存计算。
4. **单文件限制**：见第 8 节。
5. **无 `unwrap()`/`expect()`**：缓存查找全部用 `Option`/`?`/`unwrap_or`。
6. **clippy**：删除 hover 参数后检查所有调用点，禁止留下未使用函数/字段（`-D warnings` 会失败）。
7. **测试兼容**：`history::layout` / `history::view` / `history_view` 的签名会多一个 `&mut PaneCache`；测试调用点集中修改，断言不变（第 7.3 节列出的高亮测试除外）。

## 10. 风险与缓解

| 风险 | 影响 | 缓解 |
|------|------|------|
| 指纹漏判 | 界面显示旧内容 | 指纹覆盖状态、长度、首尾字节、折叠状态；缓存单测覆盖流式追加、输出环形缓冲、宽度变化、折叠切换 |
| 快照行数与绘制不一致 | 滚动条/置顶/命中漂移 | 现有 `layout_rows_equal_rendered_rows*` 与 tool geometry 测试必须全绿；新增“快照总行数 == 绘制推进行数”断言 |
| 缓存占用过高 | 内存上涨 | 32MB LRU + 第一级常驻；性能测试同时记录缓存字节数 |
| 每个块重建仍慢 | 首次打开长会话依旧卡一下 | 首次构建允许慢；滚动路径只要求热缓存 P95 ≤16ms |
| LRU 淘汰视口内块 | 滚动时重新排版、掉帧 | 淘汰只看 `last_frame < current_frame`，本帧用过的块不淘汰 |
| 多窗格/切换会话串味 | 显示别的会话内容 | `RenderCache` 按 `PaneId` 隔离；列表指纹含消息 id/sequence |
| 删除 hover 参数影响点击 | 折叠内容点不开 | `card_target_at` 与 `CardHit.clickable` 完全不动，只改样式与 hover 命中 |

## 11. 验证

### 11.1 必跑

```bash
cargo fmt
cargo clippy --all-targets -- -D warnings
cargo test
cargo test --release -- --ignored perf_lx_history
```

### 11.2 性能测试设计

`src/ui/main_content/lx/history/tests/perf.rs`：

- 构造 1000 条消息（用户/助手交替），助手消息包含多段正文、thinking、命令卡片与文件卡片，总渲染行数约 10 万。
- 视口 200 列 × 50 行。
- 预热一次后，连续 120 帧改变 `scroll`（每帧 +20 行），每帧执行 `layout + render`，记录耗时。
- 断言 P95 ≤ 16ms；同时打印/断言缓存字节数 ≤ 32MB。
- 追加一轮“流式追加”场景：每帧向最后一条消息追加一段文本，断言 P95 ≤ 16ms（只重排最后一条）。
- 标注 `#[ignore]`，默认 `cargo test` 不跑；在任务清单中要求实现完成后手动跑一次并记录结果。

### 11.3 缓存单测

- 同一块连续两次取用只排版一次（缓存命中计数 = 1）。
- 追加正文后只重排该块，其它块的 `Arc` 指针保持不变。
- 宽度改变后全部重排。
- 折叠切换只重排对应块。
- 强制 LRU 淘汰后重新构建，`HistoryLayout` 与未淘汰时 `assert_eq!`。
- 列表指纹在消息 id、顺序、状态、展开状态变化时均改变。

## 12. 关键代码索引（实现时对照）

| 位置 | 用途 |
|------|------|
| `src/ui/main_content/lx/history.rs:277` | 现有 `layout`，拆到 `layout.rs` / `snapshot.rs` |
| `src/ui/main_content/lx/history.rs:600` | 现有 `view`，改为接收缓存 |
| `src/ui/main_content/lx/history.rs:667` | 现有 `render`，拆到 `render.rs` 并做视口裁剪 |
| `src/ui/main_content/lx/history/rows.rs` | `text_rows` / `tool_rows` / `wrap`，缓存访问器的接入点 |
| `src/ui/main_content/lx/history/body.rs:48` | `draw_body` 的 `render_wrapped`，改为读缓存行 |
| `src/ui/main_content/lx/history/tool.rs:324` | `output_style` hover 项 |
| `src/ui/main_content/lx/history/tool/command.rs:57-86` | `rows` / `blocks`，缓存 `BodyRow` |
| `src/ui/main_content/lx/history/tool/file_summary.rs:489` | 剩余行数提示的 hover 提亮 |
| `src/ui/main_content/lx/history/tool/file_diff.rs:198-215` | 差异卡 hover 提亮 |
| `src/ui/main_content/lx/history/user_shell.rs:59-162` | 本地命令 sections 与输出样式 |
| `src/ui/main_content/lx/history_hit.rs:30-179` | 命中入口，改为快照查询 |
| `src/app/update/chat/viewport.rs:107-205` | 两个 resolve 合并 |
| `src/event/mouse/{hover,wheel,press,drag,hit}.rs` | 传入 `RenderCache` |
| `src/ui/{mod,panes,main_content}.rs` | `render` 逐层透传缓存 |

## 13. 实现偏差与实测（2026-10-25 回写）

### 13.1 与设计的差异

| 项 | 设计 | 实现 | 原因 |
|----|------|------|------|
| `ui::render` 签名 | 直接加 `cache` 参数 | 保留三参 `render`（内部建临时缓存）+ 新增 `render_cached` | `tests/` 下 8 个集成测试文件依赖三参签名，改签名要污染全部集成测试 |
| 文件拆分 | `history.rs` → `cache/snapshot/layout/render` | 额外拆出 `paint.rs`（绘制数据与绘制函数），`body.rs` 并入 | 绘制数据与绘制逻辑放一处，`rows()` 与 `draw()` 同源 |
| 二级缓存覆盖 | C1~C8 全部块 | 绘制数据只覆盖用户正文、markdown、thinking、统计行；工具卡片走 `BlockPaint::Legacy`，但**行数与点击面几何**同键缓存 | 命令/文件摘要展开上限 24 行；唯一无上限的展开差异实测 P95 6.19ms，未超预算；几何重算是流式的主要成本（4.7ms/帧），已随块缓存 |
| LRU 使用标记 | 「本帧使用」在取块时刷新 | 取块**不**刷新，只有绘制路径 `mark_used` 刷新 | 快照重建会遍历全部块，取块即刷新会让一次流式追加把全部缓存钉住，LRU 失效 |

### 13.2 实测（release，200×50 视口）

构建 1400 条消息、109,200 渲染行。**每条 assistant 消息带两个工具步骤**
（`read` + `exec_command`，合计 1400 张工具卡片，全部走 `BlockPaint::Legacy`）——
工具卡片是唯一没进二级内容缓存的路径，不含它们的数字只能代表 markdown/thinking：

| 场景 | 结果 |
|------|------|
| 冷启动建快照 | 74.5ms（一次性） |
| 热缓存滚动 P95（120 帧） | 1.95ms |
| 流式追加 P95（120 帧） | 2.86ms |
| 展开 2000 行差异滚动 P95（Legacy） | 6.19ms |
| 二级缓存占用 | 13.63MB / 32MB（全窗格共享） |

**点击面几何与块同键缓存**：`card_regions` / `group_blocks` / `user_shell::regions`
都是 `(块内容, 宽度, 折叠态)` 的纯函数，原先在 `cache.block()` 闭包外面每帧重算，
1400 张工具卡片实测 4.7ms/帧。现在它们算在闭包里、结果存进 `BlockPaint::Legacy.spans`，
流式追加 P95 从 8.0ms 降到 2.9ms（`snapshot` 重建只剩 1.8ms + 指纹 0.9ms）。

优化前：鼠标移动一帧最多 7 次全量 `layout`（每次都要解析全部历史正文），滚轮事件每次先全量算一遍。
优化后：一帧只建一次快照（指纹不变则零次），绘制只碰视口内的块与行。

### 13.3 已知边界

- 多个「展开的大文件差异」同屏时，每个约 6ms，两个以上会逼近 16ms 预算；届时给
  `file_diff` 接 `BlockPaint` 即可，接口已就绪（`SnapBlock.paint` = `Legacy` 换成实际变体）。
- 指纹对文件差异覆盖 hunk 数、每 hunk 行数与**每行文本长度 + 行类别**（O(行数) 整数哈希）。
  折行后的物理行数因此能区分开；仍会漏判的情况只剩「行数与每行长度都相同、内容不同」，
  而 `Legacy` 只缓存行数、不缓存正文，这种漏判不会造成行数漂移。
- 32MB 是**所有窗格共享**的预算：`RenderCache::share_budget` 按窗格数平摊（单窗格
  独占 32MB，两个窗格各 16MB），窗格开关时重新摊分。它仍只约束「上一代快照」的块——
  当前快照通过 `Arc` 持有它引用的 paint，不会被淘汰，因此实际占用 ≈ 当前快照 +
  ≤32MB 旧代。
- 拖选复制复用事件循环的常驻 `PaneCache`（`copy_text` 收 `&mut PaneCache`）：松开时
  历史刚被这一帧画过，快照与 paint 全部命中。此前临时建缓存会让整段历史重排，
  且在 `frame = 0` 下 `evict` 恒无候选（`last_frame < frame` 永不成立）。
