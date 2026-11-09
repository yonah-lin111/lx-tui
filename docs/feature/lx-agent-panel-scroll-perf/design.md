# lx 子 Agent 面板滚动性能与列表对齐 — 设计文档

- **目录**：`docs/feature/lx-agent-panel-scroll-perf/`
- **分支**：`feat/lx-agent-panel-scroll-perf`
- **Worktree**：`.worktrees/lx-agent-panel-scroll-perf`
- **关联任务**：[`task.md`](task.md)
- **验证记录**：[`verification.md`](verification.md)
- **参考仓库**：`/Users/yonah/projects/agent/opencode-dev`
  - `packages/tui/src/util/scroll.ts`（`CustomSpeedScroll` / `MacOSScrollAccel` 配置）
  - `packages/tui/src/mini/footer.subagent.tsx`（`scrollbox` + `stickyScroll` + `scrollBy`）
- **状态**：已实现，自动化验证通过；交互与视觉待用户验收

---

## 0. 结论摘要

用户确认的现象是：**多个 subagent 并行输出时，即使不滚动，消息区也会一顿一顿**。经 `codegraph` / `codebase-memory-mcp` 与源码核对，卡顿不是滚动函数本身，而是三条链路叠加：

1. **事件层**：每个 subagent delta 都执行一次 `refresh_subagent_cards`，对父会话消息和所有 run 的消息做全量扫描，复杂度随“并行数 × 会话长度”线性叠加成 O(N²) 级别。
2. **渲染层**：每个 delta 都把 `rows_dirty` 置真；事件循环在绘制前用**一次性 `PaneCache`** 把整段子会话从头 markdown 排版一次，绘制时又用另一份缓存重排一次。一次 delta = 两次全量排版。
3. **隐藏视图**：后台任务面板打开时主聊天历史根本不画，但 `resolve_lx_history` 仍然每帧对主聊天做一次全量 `history_view`。主聊天与详情共用同一个 `PaneCache` 的单快照槽，两份快照每帧互相覆盖，导致**两份全量排版每帧都逃不掉**。

用户已确认的修复方向：

- 状态数据逐条写入，界面刷新按帧/按可见性合并；
- 全局修复（主聊天 + Agents 详情 + 面板列表），不只修面板；
- 不引入滚动加速度、不做帧间插值，只借 opencode 的“只处理可见内容 + sticky follow”性能语义；
- 面板列表去掉 token 列，保留 `❯ ■ 昵称 [角色] 状态 时长 标题`，固定列宽、统一层级槽、标题用 `…` 截断；
- 增加 release 下 `#[ignore]` 的性能回归（6 个并行 subagent × 每个 200 条消息，P95 ≤ 16ms）。

预期收益：

| 路径 | 现状（每个 delta） | 改后 |
| --- | --- | --- |
| 父卡片状态回填 | O(主消息数 × run 数) | 每帧最多 1 次，同批 delta 合并 |
| 详情区排版 | 2 次全量 markdown 排版 | 每帧最多 1 次，且 resolve 与 draw 共用快照 |
| 面板打开时的主聊天 | 每帧 1 次全量排版（不可见） | 0 次（关闭面板后按需重建一次） |
| 面板列表 | 每行动态列宽 + token 格式化 | 每帧一次列宽计算，列位置稳定 |

---

## 1. 背景与用户确认

### 1.1 现象

后台任务面板的 `Agents` 页签中，同时有多个 subagent 运行时，即使不操作鼠标，消息区也会周期性卡顿；并行数越多越明显。截图中的列表还存在列不对齐、树形连接符被截成残渣（孤立的 `)`）、`done` 与 `[esc close]` 粘在一起等问题。

### 1.2 Grill Me 已确认决策

| # | 问题 | 用户确认结论 |
| --- | --- | --- |
| 1 | 卡顿触发方式 | **A：不动鼠标也卡**，多个 subagent 输出时自渲染掉帧 |
| 2 | 是否允许 UI 按帧/可见性合并 | **A：允许**。delta 逐条进状态，父卡片与不可见 run 的界面刷新合并到帧节拍；不可见 subagent 最多延迟 100ms |
| 3 | 列表简化程度 | **B：去掉 token 列**，保留 `❯ ■ 昵称 [角色] 状态 时长 标题` |
| 4 | 树形层级 | **A：保留 `├─ / └─` 连接符，只统一缩进** |
| 5 | 第一层缩进 | **A：所有层级共用固定宽度层级槽**，槽宽按当前筛选结果的最大前缀宽度；第一层留空占位 |
| 6 | 借鉴 opencode 滚动逻辑的层次 | **A：只借性能语义**（跟底/离底暂停、只排版视口内块、离屏合并），不加速度、不做帧间插值 |
| 7 | 修复范围 | **A：全局修复**，主聊天 + Agents 详情 + 面板列表共用链路一起改 |
| 8 | 验收标准 | **A：增加 release `#[ignore]` 性能回归**，6 个并行 subagent × 每个 200 条消息，P95 ≤ 16ms；再补状态合并与列表对齐单元测试 |
| 9 | 列宽计算范围 | **A：按当前筛选后的全部 run 计算**，滚动时不跳列 |
| 10 | 窄窗格降级顺序 | **A：从右往左整列丢弃**（标题 → 时长 → 状态），身份与状态图标永远保留；标题用单字符 `…` 截断 |
| 11 | 优化深度 | **B：不引入内容版本号**，保留现有指纹哈希；只消除重复全量排版与事件层全量扫描 |
| 12 | `refresh_subagent_cards` 改造 | **B：按帧合并**，delta 只置 `cards_dirty`，每批最多全量刷新一次 |
| 13 | 对齐范围 | **A：只改 Agents 列表**；Processes 已有 command/cwd 列对齐不动；顺带补筛选胶囊与 `[esc close]` 的 2 列间距 |
| 14 | 实现位置 | **A：独立 worktree + 分支**，`dev` 保持干净 |
| 15 | 文档目录 | **A：`docs/feature/lx-agent-panel-scroll-perf/`** |

---

## 2. 代码事实核验

### 2.1 现状数据流

```text
subagent delta（事件层）
  └─ update::on_subagent_delta
       ├─ apply_deltas(&mut run.chat, deltas)              // 便宜，O(delta)
       ├─ run.rows_dirty = true                            // 标记详情需要重量
       └─ refresh_subagent_cards(state, pane)              // 热点 1：每个 delta 全量扫
            ├─ 收集所有 run 的 RunCardSnapshot（clone 字符串）
            ├─ 扫描主聊天所有消息的所有 step
            └─ 扫描每个 run 的所有消息的所有 step

事件循环（event/mod.rs）
  └─ dirty = true
       └─ 下一帧绘制前：
            ├─ resolve_lx_history(...)                     // 热点 2：面板打开时主聊天不可见，仍然全量排版
            ├─ resolve_task_agents_visible(...)            // 热点 3：detail_rows 新建 PaneCache 全量排版
            └─ ui::render_cached(...)
                 └─ draw_agents_detail(...)                // 热点 4：再用 pane cache 全量排版一次
```

### 2.2 热点 1：每个 delta 全量刷新委派卡片

`src/app/update/chat/subagent.rs:106-126`：

```rust
pub fn on_subagent_delta(state: &mut AppState, child_session: &str, deltas: &[ChatDelta]) -> bool {
    // ...
    let changed = apply_deltas(&mut run.chat, deltas);
    if changed {
        run.rows_dirty = true;
    }
    // 子 agent 的实时动作挂在父消息的 spawn 卡片上：
    // 每次增量都要同步，否则要等下一次主轮事件才会刷新。
    changed | refresh_subagent_cards(state, pane) // ← 每个 delta 都全量扫描
}
```

`src/app/update/chat/subagent_cards.rs:53-82`：

```rust
pub fn refresh_subagent_cards(state: &mut AppState, pane: PaneId) -> bool {
    let runs: Vec<RunCardSnapshot> = target.lx.agents.runs.iter().map(|run| RunCardSnapshot {
        agent_id: run.agent_id.clone(),
        nickname: run.nickname.clone(),
        // ...
    }).collect();
    let mut changed = refresh_messages_with_runs(&mut target.lx.chat.messages, &runs, ROOT);
    for run in &mut target.lx.agents.runs {
        changed |= refresh_messages_with_runs(&mut run.chat.messages, &runs, &run.agent_path);
    }
    changed
}
```

并行 6 个 subagent 时，任意一个 delta 都会扫描 1 份主聊天 + 6 份子聊天；每秒几十条 delta 时，这部分就足以吃掉整帧预算。

### 2.3 热点 2：面板打开时仍在排版不可见的主聊天

`src/app/update/chat/viewport.rs:113-144`：

```rust
pub(crate) fn resolve_lx_history(state: &mut AppState, rects: &[(PaneId, Rect)], cache: &mut RenderCache) -> bool {
    for (pane, rect) in rects {
        let resolved = state.pane_anywhere(*pane)
            .filter(|target| target.view == crate::app::state::PaneView::Lx)
            .and_then(|target| {
                crate::ui::main_content::lx::history_view(inner, &target.lx, cache.pane(*pane))
            });
        // ...
    }
}
```

`history_view` 会走 `history::view` → `cache.snapshot_source`，对主聊天做**完整 markdown 排版**。但面板打开时 `ui::main_content::lx.rs` 的 `layout` 把历史区替换成 `task_output`，主聊天一条都不会画。这部分是纯浪费，而且在 `PaneCache` 里与详情快照互相覆盖（见 2.4）。

### 2.4 热点 3/4：详情一次 delta 排版两次

`src/app/update/chat/viewport.rs:189-224`：

```rust
let total = if needs_measure {
    state.pane_anywhere(*pane)
        .and_then(|target| crate::ui::main_content::lx::task_agents_rows(inner, &target.lx))
        .map(|(_, total)| total)
} else {
    None
};
```

`src/ui/main_content/lx/task/agents.rs:89-100`：

```rust
pub(crate) fn detail_rows(rect: Rect, run: &SubagentRun) -> usize {
    // ...
    history::measure(content_width, source)
}
```

`src/ui/main_content/lx/history.rs:449-457`：

```rust
pub(crate) fn measure(width: u16, source: HistorySource<'_>) -> usize {
    if width == 0 { return 0; }
    let mut cache = PaneCache::default();          // ← 每次新建，零复用
    cache.snapshot_source(width, source).total_rows
}
```

随后 `src/ui/main_content/lx/task/agents.rs:24-52` 的 `draw_detail` 又走 `history::view_source_plain(rect, source, cache)`，用**另一份** `PaneCache` 再排版一次。于是每个 delta：

- `detail_rows`：新建缓存 → 全量排版（不可复用，函数结束即丢弃）；
- `draw_detail`：pane cache → 全量排版（下一帧签名变化，再次失效）。

两次排版都发生在 UI 线程，markdown 折行、表格、工具卡片全部重算。

### 2.5 热点 5：并行动画放大脏帧

`src/app/update/chat/mod.rs:966-981` 每 100ms 对所有 running run 推进 `loading_frame`；只要任一 run 在跑，就持续置脏。本身不贵，但它保证“脏帧”连续出现，把上面几个热点从偶发变成持续。

### 2.6 列表问题

`src/ui/main_content/lx/task/agents.rs:102-227` 当前每个行独立计算：

- `role_w = display_identity.width()`，每行不同 → 状态列、时长列、标题列逐行漂移；
- token 列按 run 有无用量动态出现 → 同一屏内列数不一致；
- 树形前缀 `tree_prefix` 只对 `depth >= 2` 返回（`src/ui/main_content/lx/task/agents.rs:229-260`），第一层不占位 → 身份列整体左移，和孙辈列不对齐；
- 标题用 `put_clipped` 硬切（`src/ui/main_content/lx/task/agents.rs:221-225`），截图里的半个字符残渣来自同一类硬切；
- 筛选胶囊右端直接贴 `[esc close]`（`src/ui/main_content/lx/task.rs:441-483` 的 `right = close.x`），没有间距。

### 2.7 opencode 参考核对

- `packages/tui/src/util/scroll.ts`：`MacOSScrollAccel` 与 `CustomSpeedScroll` 只影响滚轮步长/加速度；本项目已在 `lx-scroll-speed` 决策中统一为 3 行/格，本次**不采纳**加速度。
- `packages/tui/src/mini/footer.subagent.tsx`：`scrollbox` 配置 `stickyScroll={true}` / `stickyStart="bottom"`，键盘 `scrollBy(1)`、`scrollBy(1, "viewport")`；内容只渲染 `commits` 列表。可借鉴的是 **sticky follow + 只处理可见内容**，lx 已有 `follow` 语义，本次补齐“隐藏视图不排版 + 离屏流合并”。

---

## 3. 详细设计

### 3.1 事件层：`cards_dirty` 按帧合并

#### 3.1.1 状态字段

`src/app/state/lx/agent.rs`：

```rust
#[derive(Debug, Default)]
pub struct AgentsState {
    /// 运行记录：按 `agent_path` 树形前序排列（兄弟间保持创建顺序）。
    pub runs: Vec<SubagentRun>,
    /// 父消息里的 `spawn_agent` / `wait_agent` 卡片待回填。
    ///
    /// delta 只置位，事件循环每批最多全量对账一次；避免并行 subagent 时
    /// 每条 delta 都扫描“主聊天 + 所有 run 聊天”。
    pub cards_dirty: bool,
}
```

#### 3.1.2 请求与 flush

`src/app/update/chat/subagent_cards.rs` 新增：

```rust
/// 标记本窗格委派卡片待刷新；返回是否是第一次标记。
///
/// 真正的全量对账由事件循环每批调用 [`flush_subagent_cards`] 执行一次。
pub fn request_subagent_cards_refresh(state: &mut AppState, pane: PaneId) -> bool {
    let Some(target) = state.pane_mut_anywhere(pane) else {
        return false;
    };
    let was_dirty = target.lx.agents.cards_dirty;
    target.lx.agents.cards_dirty = true;
    !was_dirty
}

/// 每个事件批次后执行一次：把本批 delta 产生的卡片状态变化统一回填。
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

> 注意借用顺序：先把 `cards_dirty` 落回 `false`，再调用 `refresh_subagent_cards(state, pane)`，避免 `&mut state` 重借。

#### 3.1.3 调用点替换

把以下调用从“立即全量刷新”改成“置位”：

- `src/app/update/chat/subagent.rs:126`（delta）
- `src/app/update/chat/subagent.rs:177`（tool pending）
- `src/app/update/chat/subagent.rs:192`（tool running）
- `src/app/update/chat/subagent.rs:246`（tool settled）
- `src/app/update/chat/subagent.rs:309`（列表回投）
- `src/app/update/chat/session.rs:266`
- `src/app/update/chat/process.rs:71`
- `src/app/update/chat/mod.rs:506`
- `src/event/chat/subagent/settle.rs:261`

示例：

```rust
// 改前（on_subagent_delta 末尾）
changed | refresh_subagent_cards(state, pane)

// 改后
let marked = request_subagent_cards_refresh(state, pane);
changed | marked
```

`refresh_subagent_cards` 本身保持“立即全量对账”的语义，只由 `flush_subagent_cards` 调用；文档注释标明“不要在 delta 路径直接调用”。

#### 3.1.4 事件循环接入

`src/event/mod.rs` 在绘制判断之前（当前 `if dirty && last_draw.elapsed() >= FRAME_INTERVAL` 上方）加：

```rust
// 同批 delta 只做一次委派卡片对账；视觉延迟最多一帧。
if update::flush_subagent_cards(state) {
    dirty = true;
}
```

事件循环本来就用 `while let Ok(message) = receiver.try_recv()` 排空同批消息（`src/event/mod.rs:581-600`），所以这里天然是“每批一次”，无需额外时间轮。

### 3.2 隐藏历史：面板打开时跳过主聊天排版

`src/app/update/chat/viewport.rs:113-144` 的过滤条件改为：

```rust
.filter(|target| {
    target.view == crate::app::state::PaneView::Lx && target.lx.task.is_none()
})
```

理由：

- 面板打开时 `ui::main_content::lx.rs` 的 `layout` 不画主历史（`lay.task_output` 取代 `history_rect`），`viewport_top` / `suppressed_prompt_header` 没有任何消费者；
- 关闭面板后下一帧会重新计算一次，`follow_bottom` 语义仍然正确；
- 这同时消除主聊天快照与详情快照在同一个 `PaneCache` 单快照槽里的每帧互踢。

`resolve_lx_rewind_center` 只在双击 Esc 的一次性请求时触发，保留现状；如后续发现面板打开时也可能触发，再按同一条件跳过。

### 3.3 详情区：共享 `PaneCache`，一次排版

#### 3.3.1 API 调整

`src/ui/main_content/lx.rs`：

```rust
pub fn task_agents_rows(
    area: Rect,
    lx: &Lx,
    cache: &mut PaneCache,
) -> Option<(usize, usize)> {
    task::agents_rows(area, lx, cache)
}
```

`src/ui/main_content/lx/task.rs`：

```rust
pub(super) fn agents_rows(
    area: Rect,
    lx: &crate::app::state::Lx,
    cache: &mut crate::ui::PaneCache,
) -> Option<(usize, usize)> {
    // ...
    Some((usize::from(rect.height), agents::detail_rows(rect, run, cache)))
}
```

`src/ui/main_content/lx/task/agents.rs`：

```rust
pub(crate) fn detail_rows(
    rect: Rect,
    run: &SubagentRun,
    cache: &mut crate::ui::PaneCache,
) -> usize {
    let content_width = history::content_rect(rect).width;
    if content_width == 0 {
        return usize::from(rect.height);
    }
    let source = HistorySource::from_chat(&run.chat, &run.compact)
        .with_viewport(HistoryViewport {
            scroll: run.scroll,
            follow_bottom: run.follow,
        });
    // 离屏合并判定需要视口行数；绘制路径会写，但 resolve 先于绘制，首帧也要写。
    cache.set_viewport_rows(usize::from(rect.height));
    history::measure_cached(content_width, source, cache)
}
```

`src/ui/main_content/lx/history.rs` 把一次性 `measure` 改成：

```rust
/// 只量总行数；复用调用方的跨帧缓存（Agents 详情区的滚动上限用）。
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

删除 `history::measure`（全仓只有 `detail_rows` 一个调用方）。

#### 3.3.2 事件循环接入

`src/app/update/chat/viewport.rs`：

```rust
pub fn resolve_task_agents_visible(
    state: &mut AppState,
    rects: &[(PaneId, ratatui::layout::Rect)],
    cache: &mut crate::ui::RenderCache,
) -> bool {
    // ...
    let total = if needs_measure {
        state
            .pane_anywhere(*pane)
            .and_then(|target| {
                crate::ui::main_content::lx::task_agents_rows(
                    inner,
                    &target.lx,
                    cache.pane(*pane),
                )
            })
            .map(|(_, total)| total)
    } else {
        None
    };
    // ...
}
```

`src/event/mod.rs:518`：

```rust
update::resolve_task_agents_visible(state, &geometry.rects, &mut ui_cache);
```

绘制路径不变：`ui::render_cached` 通过 `ui::panes` 拿到同一个 `PaneCache`，`draw_detail` 的 `view_source_plain` 直接命中 `resolve` 刚构建的快照。

#### 3.3.3 缓存抖动的边界

- 面板打开时主历史被跳过，`PaneCache` 只服务详情 → 不再互踢；
- 切换选中 run 或关闭面板 → 一次签名不匹配，重建一次，可接受；
- `rows_dirty` 仍由 delta 置位，但 `resolve` 每帧最多执行一次；同批 delta 不再逐个排版；
- `advance_lx_loading` 只改 `loading_frame`，指纹不含它，不会触发重建。

### 3.4 详情区离屏流式合并

现状 `HistorySource::from_chat` 把 `coalesce_offscreen` 固定为 `false`，因为详情滚动位在 `SubagentRun` 上（`src/ui/main_content/lx/history/source.rs:74-90`）。用户上翻阅读时，如果选中的 run 仍在流式输出，`draw_detail` 每帧仍会重建快照。

#### 3.4.1 显式视口

`src/ui/main_content/lx/history/source.rs`：

```rust
/// 离屏流式合并判定用的视口锚点。
#[derive(Debug, Clone, Copy)]
pub(crate) struct HistoryViewport {
    pub scroll: usize,
    pub follow_bottom: bool,
}

pub(crate) struct HistorySource<'a> {
    // ...
    /// `None` = 不参与离屏合并（命中/取词等只读路径）。
    pub viewport: Option<HistoryViewport>,
}

impl<'a> HistorySource<'a> {
    pub(crate) fn with_viewport(mut self, viewport: HistoryViewport) -> Self {
        self.viewport = Some(viewport);
        self.coalesce_offscreen = true;
        self
    }
}
```

`From<&Lx>` 填 `Some(HistoryViewport { scroll: lx.chat.history_scroll, follow_bottom: lx.chat.follow_bottom })`；`from_chat` 填 `None`。

#### 3.4.2 判定改为读显式视口

`src/ui/main_content/lx/history/cache.rs:269-307`：

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

#### 3.4.3 绘制与测量传真实视口

`draw_detail`：

```rust
let source = HistorySource::from_chat(&run.chat, &run.compact)
    .with_viewport(HistoryViewport {
        scroll,
        follow_bottom: run.follow,
    });
```

`detail_rows` 同样传 `run.scroll` / `run.follow`。这样：

- 用户贴底跟随时，`follow_bottom = true`，禁用合并，保证最新内容立即出现；
- 用户上翻后，流式末条在视口下方，80ms 内合并更新，滚动条总行数最多滞后 80ms；
- 回到贴底时 `follow` 恢复，马上重建到最新。

### 3.5 列表固定列布局

#### 3.5.1 目标形态

```text
❯ ■ Turing [explorer]     running     2m 59s  Count Rust files
  └─ ■ Curie [explorer]   completed   9s      Find SCHEMA_VERSION
    └─ ■ Euler [explorer] running     1m 06s  List top-level modules
  ■ Noether [explorer]    completed   4s      Report crate metadata
```

- `❯` 选择标记固定在 `list.x`；
- 层级槽固定在 `list.x + MARKER_COLS` 起，宽度 = 当前筛选结果最大前缀宽度；
- 第一层不画连接符，但保留同样宽度的空白；
- 状态图标、身份、状态、时长四列从同一列起；
- 标题占剩余宽度，溢出用 `…`；
- 不再显示 token 列。

#### 3.5.2 列宽结构

`src/ui/main_content/lx/task/agents.rs` 新增：

```rust
/// 固定列几何；所有 x 都是相对 `list.x` 的偏移。
#[derive(Debug, Clone, Copy)]
struct ListColumns {
    gutter: u16,
    identity_w: u16,
    status_w: u16,
    age_w: u16,
    x_identity: u16,
    x_status: u16,
    x_age: u16,
    x_title: u16,
}
```

计算规则：

```rust
fn list_columns(runs: &[&SubagentRun], now_ms: i64) -> ListColumns {
    let mark_w = /* StepMark::segment 宽度，恒 2 */;
    let gutter = runs
        .iter()
        .enumerate()
        .filter_map(|(index, run)| tree_prefix(run, runs, index))
        .map(|prefix| u16::try_from(prefix.width()).unwrap_or(0))
        .max()
        .unwrap_or(0);
    let identity_w = runs
        .iter()
        .map(|run| u16::try_from(run.display_identity().width()).unwrap_or(0))
        .max()
        .unwrap_or(0)
        .max(ROLE_MIN);
    let status_w = runs
        .iter()
        .map(|run| {
            u16::try_from(
                text::agent_status_with_queue(&run.status, run.queued_count).width(),
            )
            .unwrap_or(0)
        })
        .max()
        .unwrap_or(0)
        .max(STATUS_MIN);
    let age_w = runs
        .iter()
        .map(|run| {
            u16::try_from(
                crate::app::state::lx::task::age_label(run.elapsed_ms(now_ms)).width(),
            )
            .unwrap_or(0)
        })
        .max()
        .unwrap_or(0);
    let x_identity = super::MARKER_COLS
        .saturating_add(gutter)
        .saturating_add(mark_w);
    let x_status = x_identity.saturating_add(identity_w).saturating_add(CELL_GAP);
    let x_age = x_status.saturating_add(status_w).saturating_add(CELL_GAP);
    let x_title = x_age.saturating_add(age_w).saturating_add(CELL_GAP);
    ListColumns { gutter, identity_w, status_w, age_w, x_identity, x_status, x_age, x_title }
}
```

> `tree_prefix` 仍按现有逻辑判断“父节点是否在当前筛选可见集中”，父节点被隐藏时返回 `None`；此时该行在层级槽里留空，但不会破坏列对齐。

#### 3.5.3 绘制与降级

`draw_list` 改为：

```rust
let columns = list_columns(&visible_runs, now);

for (offset, run) in visible_runs.iter().skip(start).take(visible).enumerate() {
    // 选择标记
    // 层级槽：tree_prefix 写入 [list.x + MARKER_COLS, + gutter)
    // 状态图标：固定 x = list.x + columns.x_identity - mark_w - CELL_GAP
    // 身份：x = list.x + columns.x_identity，宽度 identity_w
    // 状态：x = list.x + columns.x_status，宽度 status_w
    // 时长：x = list.x + columns.x_age，右对齐在 age_w 内
    // 标题：x = list.x + columns.x_title，宽度 list.right() - x
}
```

降级顺序（从右往左）：

1. 标题宽度 = `list.right() - title_x`；若 `<= 0` 不画标题；
2. 若 `age_x + age_w > list.right()`：不画时长，标题起点回退到 `age_x`；
3. 若 `status_x + status_w > list.right()`：不画状态文字（状态图标保留），时长/标题起点回退到 `status_x`；
4. 若 `identity_x >= list.right()`：只画选择标记、层级槽、状态图标；
5. 身份在极端窄窗格用 `…` 截断（正常宽度下不截断，见决策 9）。

标题截断 helper：

```rust
/// 按显示宽度裁剪并用单字符 `…` 收尾；宽字符不会写半格。
fn clip_ellipsis(text: &str, max: u16) -> String {
    if max == 0 {
        return String::new();
    }
    if text.width() <= usize::from(max) {
        return text.to_string();
    }
    if max == 1 {
        return "…".to_string();
    }
    let budget = usize::from(max) - 1;
    let mut out = String::new();
    let mut width = 0usize;
    for ch in text.chars() {
        let w = ch.width().unwrap_or(0);
        if width + w > budget {
            break;
        }
        out.push(ch);
        width += w;
    }
    out.push('…');
    out
}
```

#### 3.5.4 状态图标位置

当前图标是 `format!("{} ", glyph)`，宽度 2。固定列下：

```text
x_identity = MARKER_COLS + gutter + mark_w
```

因此图标起点 = `x_identity - mark_w`。这样身份列在所有行严格对齐，状态图标也不会侵占层级槽。

### 3.6 筛选胶囊间距

`src/ui/main_content/lx/task.rs:441-483`：

```rust
let right = close.map_or(rect.right(), |close| close.x.saturating_sub(GAP));
```

同时保留 `start <= tabs_end + GAP` 的整组丢弃规则。绘制与命中都走同一个 `filter_row_rects`，不会出现“画在这、点在那”。

### 3.7 滚动语义保持不变

- 主聊天与详情继续使用 `follow_bottom` / `follow` 语义：贴底跟随时新增内容自动贴尾；用户上翻后保持顶部行稳定；回到底部自动恢复跟随。
- 滚轮步长继续使用全局 `WHEEL_LINES = 3`；`PageUp` / `PageDown` 继续按视口翻页。
- 不引入跨帧插值，不在渲染层做任何状态写入。

---

## 4. 文件级改动清单

| 文件 | 改动 |
| --- | --- |
| `src/app/state/lx/agent.rs` | `AgentsState` 新增 `cards_dirty: bool` |
| `src/app/update/chat/subagent_cards.rs` | 新增 `request_subagent_cards_refresh` / `flush_subagent_cards`；保留 `refresh_subagent_cards` 作立即对账实现 |
| `src/app/update/chat/subagent.rs` | delta / tool / 列表回投改为请求刷新 |
| `src/app/update/chat/session.rs` | 同上 |
| `src/app/update/chat/process.rs` | 同上 |
| `src/app/update/chat/mod.rs` | 导出新函数；调用点改请求 |
| `src/event/chat/subagent/settle.rs` | 直接调用改为请求 |
| `src/event/mod.rs` | 绘制前 `flush_subagent_cards`；`resolve_task_agents_visible` 传入 `&mut ui_cache` |
| `src/app/update/chat/viewport.rs` | `resolve_lx_history` 面板打开时跳过；`resolve_task_agents_visible` 接收 cache 并复用 |
| `src/ui/main_content/lx.rs` | `task_agents_rows` 增加 cache 参数 |
| `src/ui/main_content/lx/task.rs` | `agents_rows` 增加 cache；`filter_row_rects` 补 `GAP` |
| `src/ui/main_content/lx/task/agents.rs` | 共享缓存测量；固定列布局；去掉 token 列；`…` 截断；离屏视口 |
| `src/ui/main_content/lx/history.rs` | `measure` → `measure_cached` |
| `src/ui/main_content/lx/history/source.rs` | 新增 `HistoryViewport` / `with_viewport` |
| `src/ui/main_content/lx/history/cache.rs` | 离屏合并读显式视口 |
| `src/ui/main_content/lx/task/tests/agent_list.rs` | 新增列表对齐/截断测试 |
| `src/ui/main_content/lx/task/tests/perf.rs` | 新增 6×200 性能回归 |
| `src/app/update/chat/tests/subagent.rs` | 新增按帧合并测试 |
| `src/ui/main_content/lx/history/tests/perf.rs` | 如需要，补详情离屏合并断言 |

---

## 5. 测试与验收

### 5.1 单元测试

1. **按帧合并**
   - 连续注入多条 subagent delta，不调用 `flush`：父卡片保持旧值，`cards_dirty == true`；
   - 调用 `flush_subagent_cards`：父卡片更新，`cards_dirty == false`；
   - 一次 flush 后再次 flush：返回 false，不重复扫描。
2. **共享缓存**
   - 对同一 `rect` 先 `resolve_task_agents_visible` 再 `draw_detail`，`PaneCache::builds()` 只增加 1；
   - 修改 run 内容后，下一次 resolve + draw 只增加 1。
3. **列表对齐**
   - 构造不同昵称长度、不同深度、不同状态的 run，断言所有行身份列、状态列、时长列起点一致；
   - 构造超长标题，断言输出以 `…` 结尾且宽度不超过标题列；
   - 窄窗格按标题 → 时长 → 状态的顺序消失，状态图标保留；
   - 筛选胶囊与 `[esc close]` 之间至少 `GAP` 列。
4. **离屏合并**
   - 详情上翻、流式末条在视口下方：80ms 内连续 delta 不增加 `builds`；
   - 贴底跟随时：立即重建。
5. **回归**
   - 现有 `task::tests::history_takeover`、`history::tests::*`、`app::update::chat::tests::subagent` 全部通过。

### 5.2 性能回归

新增 `src/ui/main_content/lx/task/tests/perf.rs`：

```rust
//! Agents 详情滚动性能回归（`#[ignore]`，release 下手动跑）。
//!
//! 口径：6 个并行 subagent × 每个 200 条消息；连续 120 帧的
//! `layout + render` P95 ≤ 16ms。
//!
//! cargo test --release -- --ignored perf_agents_panel
```

测试内容：

1. 构造 6 个 `SubagentRun`，每个 200 条消息（正文 + 工具卡 + thinking）；
2. 每帧给每个 run 追加一行流式正文，模拟并行 delta；
3. 调用 `resolve_task_agents_visible` + `draw_agents_detail`（等价于事件循环顺序）；
4. 统计 120 帧 P95，断言 ≤ 16ms；
5. 同时断言 `PaneCache::builds()` 增量不超过帧数（不允许同帧重建两次）。

### 5.3 验证命令

```bash
cargo fmt
cargo clippy --all-targets -- -D warnings
cargo test
cargo test --release -- --ignored perf_agents_panel
```

交互与视觉最终由用户运行 `cargo run` 验收：

- 多个 subagent 并行时主消息区不再周期性卡顿；
- 面板打开时滚动详情丝滑，贴底跟随、上翻不被拽回；
- 列表各列对齐，树形连接符不再出现半截字符；
- 窄窗格按约定顺序降级。

---

## 6. 风险与回滚

| 风险 | 影响 | 缓解 |
| --- | --- | --- |
| 父卡片文案最多延迟一帧 | 肉眼不可见 | 事件循环按批 flush；测试覆盖 |
| 面板打开时 `viewport_top` 不更新 | 关闭面板后首帧重算一次 | `follow_bottom` 语义保留；关闭后下一帧恢复 |
| 主聊天与详情共享单快照槽 | 切换面板/选中项时重建一次 | 面板打开时跳过主历史；一次重建可接受 |
| 离屏合并导致滚动条总行数滞后 ≤ 80ms | 仅上翻阅读时可见 | 与主聊天现有策略一致；可测试关闭 |
| 列宽随筛选结果变化 | 切筛选时列整体位移 | 用户已确认按“筛选后全部 run”计算 |
| 新增 `cards_dirty` 忘记复位 | 卡片永不刷新 | 所有调用集中到 request/flush；测试断言 |

回滚策略：所有改动集中在 `feat/lx-agent-panel-scroll-perf` 分支，未改数据库、未新增迁移；如需回滚直接丢弃该分支即可，`dev` 不受影响。

---

## 7. 非目标

- 不引入滚动加速度、不做帧间平滑插值；
- 不改 `WHEEL_LINES = 3` 的全局滚动步长；
- 不改 Processes 页的列布局；
- 不改数据库 Schema、不新增迁移；
- 不重构 `HistorySnapshot` / `BlockPaint` 的缓存结构；
- 不引入内容版本号（决策 11：性能测试不达标再追加）。
