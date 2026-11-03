# Prompt Recent 抽屉高度、选择行为与条目布局调整设计

- 状态：**已复审并修复（待合并）**
- 关联任务：`docs/feature/prompt-recent-drawer-polish/task.md`
- 复审报告：`docs/feature/prompt-recent-drawer-polish/review.md`
- 日期：2026-11-03
- 范围：`src/ui/prompt/recent.rs`、`src/layout/prompt.rs`、`src/app/update/prompt/recent.rs`、`src/app/update/pane.rs`、`src/app/prompt/tree/panel.rs` 及对应测试

---

## 0. 背景与事实核验

本次改造基于当前 `dev` 工作区的 Recent Pages 抽屉实现，不依赖外部参考目录。

事实核验结果：

1. 当前 Recent 抽屉由 `layout::prompt_body_split` 计算几何：
   - 抽屉高度上限已经是 `body.height / 2`；
   - 但同时硬编码最多显示 3 条（1 行分割线 + 6 行内容 = 7 行），所以大面板下抽屉永远长不高。
2. 当前条目渲染为两行：
   - 第 1 行：`❯` / 空格 + `prompt_name` + 右对齐 `time_label`；
   - 第 2 行：2 列缩进 + `page_title` + 右端状态圆点（`○` / `●`）。
3. 当前点击 / 回车选择条目后，抽屉**保持展开**：
   - `open_recent_page` 在切换后重新设置 `recent_active` 与 `recent_scroll`；
   - 事件层注释明确写着“抽屉保持展开”。
4. 当前页标题颜色规则来自编辑器顶边框标题行：
   - `Todo` → 默认亮白（`Color::Reset`）；
   - `InProgress` → 黄色；
   - `Done` → 绿色；
   - 无 `&&&` 模板块 → `muted_on_border`（暗灰）。
   对应 token 为 `style::template_border(status)`，`None` 时使用 `style::muted_on_border()`。
5. “仅打开不修改”不会自动重排 recent：
   - `commit_prompt_buffer` 在 `page.content == content && !pending` 时直接 `confirm_saved(revision)` 并返回，不会执行 `page.touch(now)`；
   - `Effect::SetActivePage` 写入的是 `workspace_prompt.updated_at` / `active_page_id`，而 recent 排序使用 `workspace_prompt_page.updated_at`；
   - `PromptTree::replace` 按 item id 排序，不按 `updated_at` 排序。
   本设计保留该行为，并新增回归测试锁死。

> 说明：用户提到的 `ref-content/ui/prompt/` 在当前工作区不存在（`find` / `mdfind` 均无结果）。本设计按现有 `src/ui/prompt/recent.*` 实现理解；若参考实现另有含义，以评审意见为准再修订。

---

## 1. Grill Me 已确认决策

| 编号 | 决策 | 结论 |
| --- | --- | --- |
| Q1 | 抽屉高度策略 | **A**：取消“最多 3 条”硬上限，按内容区半屏能放多少条就显示多少条；至少 2 条、最多 20 条，总高度 ≤ 内容区一半 |
| Q2 | 选择后关闭范围 | **A**：鼠标单击与键盘 `Enter` 都关闭；选中“当前正在编辑的页”也关闭，只收起抽屉、不重新加载正文 |
| Q3 | 互换后的行样式 | **A**：第 1 行页标题按状态着色、当前页加粗；第 2 行条目名恒为 muted 次要信息 |
| Q4 | 半屏基准 | **A**：以 `prompt_body_rect`（已扣顶部工具栏）为基准，抽屉总高度 ≤ 内容区一半 |
| Q5 | 仅打开不修改 | 不调整 recent item 排序（不刷新 `page.updated_at`，不重排） |

---

## 2. 目标与非目标

### 2.1 目标

1. Recent 抽屉在大面板中最多可以增长到 Prompt 内容区高度的一半，而不是固定 7 行。
2. 点击或回车选择任意一条记录后，抽屉立即收起，键盘和视口回到编辑器。
3. Recent 条目布局调整为：
   - 第 1 行：当前页标记 + **页标题** + 右对齐时间；
   - 第 2 行：2 列缩进 + **提示词条目名**；
   - 移除右端状态圆点；
   - 页标题颜色跟随页模板状态（todo 亮白 / in_progress 黄 / done 绿 / 无状态暗灰）。
4. 仅打开页面、不修改正文时，recent 列表顺序保持不变。

### 2.2 非目标

- 不改变 recent 的排序规则（仍为 `updated_at` 降序，同刻按 page id 降序）。
- 不改变最近页面数量上限（仍为 `MAX_RECENT_PAGES = 20`）。
- 不改变页列表视图（`PromptMode::Pages`）的状态圆点；圆点只从 recent 条目移除。
- 不改变 `open_recent_page` 的“提交当前编辑、切换条目 + 页、定位编辑器”主流程。
- 不新增数据库迁移、不新增依赖、不新增用户可见文案。

---

## 3. 关键设计

### 3.1 抽屉高度：半屏内按内容动态增长

#### 3.1.1 现状问题

`src/layout/prompt.rs` 当前逻辑：

```rust
let max_height = body.height / 2;
let mut visible = if recent_count == 0 {
    0
} else {
    recent_count.clamp(
        crate::app::prompt::RECENT_MIN_VISIBLE_ENTRIES,
        crate::app::prompt::RECENT_VISIBLE_ENTRIES, // 3
    )
};
let mut drawer_height = if visible == 0 {
    2
} else {
    1 + 2 * visible as u16
};
while visible > crate::app::prompt::RECENT_MIN_VISIBLE_ENTRIES && drawer_height > max_height {
    visible -= 1;
    drawer_height = 1 + 2 * visible as u16;
}
if drawer_height > max_height {
    // 收起抽屉
}
```

问题：即使 `max_height` 足够大，`visible` 也被 `RECENT_VISIBLE_ENTRIES = 3` 卡死，抽屉最多 7 行。

#### 3.1.2 新算法

以 Prompt 内容区（`prompt_body_rect`，已扣除工具栏两行）高度的一半为上限：

```text
max_height = body.height / 2
若 recent_count == 0：
    drawer_height = 2（1 行分割线 + 1 行空态）
    若 max_height < 2 → 收起
否则：
    capacity = (max_height - 1) / 2          // 分割线占 1 行，每条占 2 行
    若 capacity < RECENT_MIN_VISIBLE_ENTRIES(2) → 收起
    visible = min(max(recent_count, 2), capacity, MAX_RECENT_PAGES)
    drawer_height = 1 + 2 * visible
```

关键点：

- `visible` 最少 2：只有 1 条记录时仍保留 2 条高度，避免抽屉忽高忽低（保留现有观感）。
- `visible` 最多 `MAX_RECENT_PAGES = 20`：与 `recent_page_rows` 的截断保持一致。
- `recent_page_count` 可能大于 20，因此进入布局时先 `min(MAX_RECENT_PAGES)`，否则会出现“布局按 25 条算、渲染只有 20 条”的高度错位。
- 抽屉总高度（含分割线）不超过 `body.height / 2`；半屏为奇数时允许最下面空 1 行，不强行凑满。
- `capacity < 2` 时继续沿用现有策略：自动收起抽屉，避免不可见抽屉吞掉键盘输入。

#### 3.1.3 关键代码（`src/layout/prompt.rs`）

```rust
pub fn prompt_body_split(panel: Rect, recent_open: bool, recent_count: usize) -> PromptBodySplit {
    let inner = pane_inner_rect(panel);
    let body = prompt_body_rect(inner);
    if !recent_open || body.height == 0 {
        return suppressed_split(body);
    }
    let main = body_text_rect(body);
    let max_height = body.height / 2;
    // 与 recent_page_rows 的截断保持一致，避免布局与渲染行数不一致。
    let recent_count = recent_count.min(crate::app::prompt::MAX_RECENT_PAGES);
    let (visible, drawer_height) = if recent_count == 0 {
        (0, 2)
    } else {
        let capacity = usize::from(max_height.saturating_sub(1) / 2);
        if capacity < crate::app::prompt::RECENT_MIN_VISIBLE_ENTRIES {
            // 半屏连最低 2 条都放不下：收起抽屉。
            return suppressed_split(body);
        }
        let visible = recent_count
            .max(crate::app::prompt::RECENT_MIN_VISIBLE_ENTRIES)
            .min(capacity);
        (visible, 1 + 2 * visible as u16)
    };
    if drawer_height > max_height {
        return suppressed_split(body);
    }
    // divider_y / content / scrollbar / main 收缩见实际实现。
    PromptBodySplit {
        visible_entries: visible,
        // ...
    }
}
```

> “收起”分支已抽成局部辅助 `suppressed_split(body)`，避免复制三处 `PromptBodySplit` 构造；不新增公共 API。
> 空态（`recent_count == 0`）固定占 2 行；“至少 2 条”只约束非空记录。

#### 3.1.4 高度示例

| 内容区高度 | 半屏上限 | 容量（条） | 记录数 | 实际显示 | 抽屉高度 |
| --- | --- | --- | --- | --- | --- |
| 40 | 20 | 9 | 20 | 9 | 19 |
| 40 | 20 | 9 | 3 | 3 | 7 |
| 40 | 20 | 9 | 1 | 2（留空） | 5 |
| 26 | 13 | 6 | 6 | 6 | 13 |
| 14 | 7 | 3 | 10 | 3 | 7 |
| 10 | 5 | 2 | 10 | 2 | 5 |
| 8 | 4 | 1 | 10 | 收起 | 0 |
| 8 | 4 | — | 0 | 空态 | 2 |

#### 3.1.5 关联修正：`recent_visible` 状态缓存

`sync_prompt_recent_selection` 目前用固定的 `RECENT_VISIBLE_ENTRIES = 3` 判断选中项是否在可见窗口内：

```rust
let visible = crate::app::prompt::RECENT_VISIBLE_ENTRIES;
panel.recent_active = Some(next);
panel.recent_scroll = keep_visible(panel.recent_scroll, next, visible, rows.len());
```

动态高度后，这个固定值会让 `recent_scroll` 错误回滚（例如实际可见 9 条，却按 3 条窗口计算）。方案：

- 在 `PromptPanel` 中新增 `recent_visible: usize`，由 `resize_panes` 写入当前 `split.visible_entries`；
- `close_prompt_recent` 置 0；
- `sync_prompt_recent_selection` 使用 `panel.recent_visible.max(1)`。

`PromptPanel` 新增字段：

```rust
/// 抽屉当前可见条目数；由 resize_panes 按几何写入，抽屉收起时为 0。
/// 只供更新层判断选中窗口，渲染层不读取（渲染直接用 PromptBodySplit）。
pub recent_visible: usize,
```

`src/app/update/pane.rs`：

```rust
let split = layout::prompt_body_split(*rect, recent_open, recent_count);
// ...
state.prompt.resize(cols, rows);
let workspace = state.prompt_workspace();
if let Some(workspace_state) = state.workspaces.get_mut(workspace) {
    workspace_state.prompt_panel.recent_visible = split.visible_entries;
}
```

`src/app/update/prompt/recent.rs`：

```rust
let visible = panel.recent_visible.max(1);
panel.recent_active = Some(next);
panel.recent_scroll = keep_visible(panel.recent_scroll, next, visible, rows.len());
```

同时删除 `RECENT_VISIBLE_ENTRIES` 常量及其在 `src/app/prompt.rs` 的再导出；`RECENT_MIN_VISIBLE_ENTRIES` 保留。

### 3.2 选择 item 后关闭抽屉

#### 3.2.1 行为

- 鼠标左键单击任意 recent 条目：打开对应 `PromptItem + PromptPage`，然后立刻收起抽屉。
- 键盘 `Enter`：打开高亮项，然后收起抽屉。
- 选中“当前正在编辑的页”：不重新加载正文，只收起抽屉；若当前处于页列表视图（`PromptMode::Pages`），
  同时切回编辑视图（`PromptMode::Editor`），否则“选择条目”会只关抽屉、画面仍停在页列表。
- 选中任意条目后键盘焦点交还 prompt（`focus_prompt`），覆盖从窗格焦点点击 `[rec]` 再选择条目的路径。
- 收起后：
  - `recent_open = false`；
  - `recent_active = None`；
  - `recent_scroll = 0`；
  - `recent_visible = 0`；
  - `prompt_resize_pending = true`，下一帧编辑器视口恢复完整高度；
  - `[rec]` 按钮从 accent 高亮恢复默认样式。

#### 3.2.2 关键代码（`src/app/update/prompt/recent.rs`）

改造前“当前页”分支只更新选中态并返回：

```rust
if 已经是当前页 {
    panel.recent_active = Some(index);
    panel.recent_scroll = index.saturating_sub(1);
    state.prompt_recent_hover = None;
    return true;
}
```

改为：

```rust
if 已经是当前页 {
    // 只收起抽屉，不重载正文，也不刷新 updated_at。
    // 页列表视图下仍须回到编辑视图；鼠标从窗格焦点打开时还要交还键盘焦点。
    if let Some(workspace_state) = state.workspaces.get_mut(workspace) {
        workspace_state.prompt_panel.mode = PromptMode::Editor;
    }
    state.prompt_recent_hover = None;
    focus_prompt(state);
    close_prompt_recent(state);
    return true;
}
```

切换目标后的尾部：

```rust
// 原逻辑：重新定位 recent_active / recent_scroll，保持抽屉展开。
state.prompt_recent_hover = None;
close_prompt_recent(state);
true
```

`apply_prompt_recent_key` 不需要改动：`Enter` 已调用 `open_active_recent_page` → `open_recent_page`，关闭行为自动生效。

鼠标事件层同步修改注释（`src/event/mouse/press.rs`）：

```rust
// 最近页面抽屉条目：单击打开对应条目 + 页，随后收起抽屉。
update::open_recent_page(state, index);
```

#### 3.2.3 “仅打开不修改不重排”的不变量

- `open_recent_page` 不直接写 `updated_at`。
- `commit_editing_prompt` 只在正文实际变化（或存在 pending 保存）时调用 `page.touch(now)`；内容相同直接 `confirm_saved` 返回。
- `Effect::SetActivePage` 只更新 `workspace_prompt.active_page_id` / `workspace_prompt.updated_at`，不影响 `workspace_prompt_page.updated_at`。
- recent 排序读取 `PromptPage.updated_at`，因此仅打开页面不会改变 recent 顺序。

新增回归测试锁死该不变量（见 §5.3）。

### 3.3 条目布局与状态色

#### 3.3.1 改造前后

改造前：

```text
❯ Alpha                       [5m]
  Auth Flow                      ●
```

改造后：

```text
❯ Auth Flow                   [5m]
  Alpha
```

- 第 1 行：2 列标记位（当前页 `❯ `，否则两个空格）+ 页标题 + 右对齐时间；
- 第 2 行：2 列缩进 + 提示词条目名；
- 状态圆点（`○` / `●`）完全移除，状态只由页标题颜色表达。

#### 3.3.2 颜色映射

| `row.status` | 第 1 行页标题样式 | 说明 |
| --- | --- | --- |
| `Some(TemplateStatus::Todo)` | `style::text()` | 默认亮白，对齐 prompt 标题行 |
| `Some(TemplateStatus::InProgress)` | `style::template_status(InProgress)` | 黄色 |
| `Some(TemplateStatus::Done)` | `style::template_status(Done)` | 绿色 |
| `None` | `style::muted()` | 暗灰，表示无 `&&&` 模板块状态 |

附加规则：

- `row.current == true` 时，页标题再叠加 `style::strong()`（加粗）；`❯` 标记保留在第 1 行最左。
- 第 2 行条目名恒为 `style::muted()`；悬停 / 键盘选中时叠加 `style::selection()` 底色。
- 时间标签保持 `style::muted()` + 选中底色；窄屏优先保留页标题，空间不足时省略时间。
- 悬停 / 选中时，状态色优先于选中前景色（保持黄 / 绿可辨），选中底色仍然生效。

#### 3.3.3 关键代码（`src/ui/prompt/recent.rs`）

```rust
fn paint_entry(buf: &mut Buffer, area: Rect, y: u16, row: &RecentPageRow, hovered: bool) {
    let highlight = if hovered {
        tree::fill_row(buf, area, y);
        tree::fill_row(buf, area, y + 1);
        style::selection()
    } else {
        Style::default()
    };
    let base = style::text().patch(highlight);
    let marker = if row.current {
        format!("{} ", text::AGENT_SELECTED_MARKER)
    } else {
        "  ".to_string()
    };
    let x = tree::paint_text(buf, area.x, y, &marker, base, area.right());

    // 第 1 行：页标题 + 右对齐时间。
    let time_width = row.time_label.width();
    let time_x = area.right().saturating_sub(time_width as u16);
    let title_right = if time_x >= x.saturating_add(5) {
        tree::paint_text(
            buf,
            time_x,
            y,
            &row.time_label,
            style::muted().patch(highlight),
            area.right(),
        );
        time_x.saturating_sub(1)
    } else {
        area.right()
    };
    let title_style = match row.status {
        Some(TemplateStatus::Todo) => style::text(),
        Some(status) => style::template_status(status),
        None => style::muted(),
    };
    let title_style = if row.current {
        title_style.patch(style::strong())
    } else {
        title_style
    };
    tree::paint_name(
        buf,
        x,
        y,
        &row.page_title,
        base.patch(title_style),
        usize::from(title_right.saturating_sub(x)),
        title_right,
    );

    // 第 2 行：条目名（次要信息）。
    let name_x = tree::paint_text(buf, area.x, y + 1, "  ", base, area.right());
    tree::paint_name(
        buf,
        name_x,
        y + 1,
        &row.prompt_name,
        style::muted().patch(highlight),
        usize::from(area.right().saturating_sub(name_x)),
        area.right(),
    );
}
```

注意：

- `base.patch(title_style)` 的顺序保证 `InProgress` / `Done` 的前景色覆盖选中前景色，但仍保留 `highlight` 的背景色。
- `Todo` 的 `style::text()` 不设置前景色，因此悬停时继续使用选中前景色（高亮白字）。
- `None` 的 `style::muted()` 只加 `DIM`，不覆盖前景色。
- 删除原来的 `PAGE_STATUS_OPEN` / `PAGE_STATUS_DOT` 绘制分支；这两个常量仍被页列表视图使用，不删除。

#### 3.3.4 宽度与截断

- 第 1 行：最少给页标题保留 4 列（首字符 + 省略号），否则不画时间标签，标题占满剩余宽度。
- 第 2 行：条目名占用整行剩余宽度，超宽由 `tree::paint_name` 截断并追加 `…`。
- 右端不再有状态圆点，因此第 2 行右侧没有额外预留。

### 3.4 数据流与调用链

```text
用户点击 [rec] / Ctrl+R
  └─ update::toggle_prompt_recent
       ├─ panel.recent_open = true
       ├─ prompt_resize_pending = true
       └─ 下一帧 resize_panes
            └─ layout::prompt_body_split(panel, true, recent_page_count)
                 ├─ 计算 visible / drawer_height（本次改为半屏动态容量）
                 ├─ 写入 panel.recent_visible
                 └─ state.prompt.resize(split.main)

用户点击条目 / 按 Enter
  └─ update::open_recent_page / open_active_recent_page
       ├─ commit_editing_prompt（仅内容变化时 touch + 排队保存）
       ├─ 切换 item.active_page / editing / mode
       ├─ load_editor_target
       ├─ focus_prompt
       └─ close_prompt_recent
            ├─ recent_open = false
            ├─ recent_scroll = 0 / recent_active = None / recent_visible = 0
            └─ prompt_resize_pending = true

渲染
  └─ ui::prompt_frame::render_body
       ├─ split = prompt_body_split(area, recent_open, recent_rows.len())
       ├─ recent::render(RecentRender { visible_entries: split.visible_entries, ... })
       └─ recent::paint_entry
            ├─ 第 1 行：marker + page_title（状态色）+ time
            └─ 第 2 行：prompt_name（muted）
```

---

## 4. 文件变更清单

| 文件 | 变更 |
| --- | --- |
| `src/layout/prompt.rs` | 重写 `prompt_body_split` 高度计算；移除 3 条硬上限；`recent_count` 先按 `MAX_RECENT_PAGES` 截断；保留矮屏收起逻辑 |
| `src/app/prompt/recent.rs` | 删除 `RECENT_VISIBLE_ENTRIES` 常量 |
| `src/app/prompt.rs` | 移除 `RECENT_VISIBLE_ENTRIES` 的 `pub use` |
| `src/app/prompt/tree/panel.rs` | `PromptPanel` 新增 `recent_visible: usize` 字段并初始化 |
| `src/app/update/pane.rs` | `resize_panes` 写入 `prompt_panel.recent_visible = split.visible_entries` |
| `src/app/update/prompt/recent.rs` | 选择后调用 `close_prompt_recent`；`sync_prompt_recent_selection` 使用 `recent_visible`；`close_prompt_recent` 清零 `recent_visible` |
| `src/ui/prompt/recent.rs` | 交换两行内容；页标题按状态着色；移除状态圆点；条目名 muted |
| `src/event/mouse/press.rs` | 更新条目点击注释（行为由 `open_recent_page` 驱动） |
| `src/layout/tests.rs` | 新增/调整半屏动态容量测试、20 条截断测试 |
| `src/app/update/prompt/recent/tests.rs` | 选择后关闭、当前页关闭、不修改不重排测试 |
| `src/event/tests/prompt_recent.rs` | 点击/回车关闭、滚动测试几何调整 |
| `src/ui/prompt/recent/tests.rs` | 两行布局、状态色、无圆点、当前页加粗测试 |

不涉及：数据库 schema、Effect 定义、用户可见文案、页列表视图渲染。

---

## 5. 测试计划

### 5.1 布局（`src/layout/tests.rs`）

1. `prompt_body_split_reserves_drawer_rows`：
   - 保留现有 40×18 面板断言（半屏容量 3 条）；
   - 更新注释：3 条来自半屏容量，而不是固定常量。
2. 新增 `prompt_body_split_grows_to_half_height`：
   - 面板 40×30（内容区 26 行，半屏 13 行，容量 6 条）；
   - `recent_count = 10` → `visible_entries == 6`，`recent_text.height == 12`，`main.height == 13`。
3. 新增 `prompt_body_split_caps_at_twenty_entries`：
   - 面板足够高，`recent_count = 25` → `visible_entries == 20`，抽屉高度 41。
4. 保留 `prompt_body_split_shrinks_to_minimum_two_then_suppresses`：
   - 内容区 10 行 → 2 条；内容区 8 / 4 行 → 收起。
5. 保留 `prompt_body_split_keeps_empty_state_row`。

### 5.2 Recent 渲染（`src/ui/prompt/recent/tests.rs`）

1. `renders_page_title_then_prompt_name_without_dot`：
   - 第 1 行包含 `Auth Flow` 与 `[5m]`，以 `❯` 开头；
   - 第 2 行包含 `Alpha`；
   - 两行都不包含 `●` / `○`。
2. `page_title_uses_status_colors`：
   - `Todo` → 默认前景、无 `DIM`；
   - `InProgress` → `Color::Yellow`；
   - `Done` → `Color::Green`；
   - `None` → `DIM`。
3. `current_page_title_is_bold`：
   - `current = true` 时第 1 行标题单元格含 `Modifier::BOLD`。
4. `status_color_wins_over_hover_foreground`：覆盖 `InProgress` 黄与 `Done` 绿在悬停底色上的前景优先级。
5. 保留悬停 / 键盘选中双行底色、关闭按钮、空态、滚动窗口测试。

### 5.3 更新层（`src/app/update/prompt/recent/tests.rs`）

1. `open_recent_page_commits_current_and_switches_target`：
   - 断言切换后 `mode == Editor`、`editing == Item(12)`、正文为 `beta`；
   - 断言 `!recent_open`、`recent_active == None`、`recent_scroll == 0`、`recent_visible == 0`；
   - 旧页正文仍在内存中提交。
2. 新增 `selecting_current_page_closes_without_reload`：
   - 抽屉展开、当前页高亮、焦点在窗格，调用 `open_recent_page`；
   - 断言抽屉关闭、正文/光标/修订号不变、焦点回到 prompt。
3. 新增 `selecting_current_page_from_pages_returns_to_editor`：
   - 页列表视图 + 当前页，调用 `open_recent_page`；
   - 断言回到 `PromptMode::Editor`、抽屉关闭、正文不变、焦点回到 prompt。
4. 新增 `opening_page_without_edit_keeps_recent_order`：
   - Alpha `updated_at = 100`、Beta `updated_at = 200`；
   - 打开 Beta 不修改；断言行顺序仍为 Beta、Alpha，且 Alpha / Beta 的 `updated_at` 都未变化。
5. 新增 `sync_uses_cached_visible_entries`：
   - `recent_visible = 2`、3 条记录、目标为第 3 条；
   - 断言 `recent_scroll == 1`，锁定动态可见窗口（而不是固定 3 条）。
6. 保留滚动钳制、`recent_active` 越界修正测试。

### 5.4 事件层（`src/event/tests/prompt_recent.rs`）

1. `clicking_recent_entry_opens_page_and_closes_drawer`：
   - 原 `clicking_recent_entry_opens_page_and_keeps_drawer` 改为断言关闭；
   - 断言 `recent_active == None`。
2. `clicking_recent_entry_opens_from_tree_and_pages`：
   - 树视图与页列表视图点击条目后都进入 Editor 且抽屉关闭。
3. `keyboard_defaults_to_current_page_and_navigates_to_open`：
   - `Enter` 打开后抽屉关闭；
   - 正文仍为 `beta`。
4. 新增 `escape_closes_drawer_without_changing_text`：
   - 抽屉展开后按 `Esc`，断言关闭、`recent_active` 清空、正文不变。
5. 滚动测试几何调整：
   - `recent_scrollbar_supports_track_click_and_thumb_drag`、`keyboard_navigation_keeps_selection_visible`、`wheel_over_drawer_scrolls_entries_without_touching_editor` 使用 140×20 面板，确保半屏容量小于记录数，滚动条与滚轮仍可测。
6. 保留矮屏压制、悬停、右键吞掉、删除条目钳制测试。

---

## 6. 边界与风险

1. **半屏奇数高度**：抽屉高度恒为 `1 + 2n`，半屏为奇数时最下面空 1 行；这是双行条目的自然结果，不额外补一行。
2. **记录数 > 20**：布局先按 `MAX_RECENT_PAGES` 截断，保证 `resize_panes` 与 `render_body` 使用同一行数；否则会出现编辑器视口比实际渲染小的问题。
3. **极矮面板**：`capacity < 2` 继续自动收起，避免不可见抽屉吞键；`short_terminal_suppresses_drawer_without_swallowing_typing` 必须保持通过。
4. **状态色与选中底色**：状态色覆盖选中前景色，选中背景仍保留；需要在渲染测试中覆盖 `InProgress` / `Done` 的悬停场景，防止被高亮白字冲掉。
5. **关闭后视口恢复**：`close_prompt_recent` 已设置 `prompt_resize_pending`；不要在 `open_recent_page` 里手动 resize 编辑器，避免双重计算。
6. **排序不变量**：只有正文实际变化才允许 `page.touch`；任何“打开即刷新 updated_at”的改动都会破坏本次需求，代码评审时重点检查。
7. **页列表圆点**：本次只移除 recent 条目的圆点，`src/ui/prompt/pages.rs` 保持不变；不要误删 `text::PAGE_STATUS_*` 常量。

---

## 7. 验收标准

- [x] 大面板（内容区 ≥ 26 行）下 recent 抽屉可显示超过 3 条，最多到半屏容量。
- [x] 抽屉总高度（含分割线）不超过 Prompt 内容区高度的一半。
- [x] 记录不足 2 条时仍保留 2 条高度；矮屏放不下 2 条时自动收起且不吞键。
- [x] 鼠标单击 recent 任意条目后抽屉关闭，进入对应 Editor 页。
- [x] 键盘 `Enter` 打开高亮项后抽屉关闭。
- [x] 点击当前正在编辑的页只关闭抽屉，不重载正文。
- [x] 仅打开、不修改正文时，recent 列表顺序不变。
- [x] 第 1 行为页标题 + 时间，第 2 行为条目名；两行不再出现状态圆点。
- [x] 页标题颜色与 prompt 标题行一致：todo 亮白、in_progress 黄、done 绿、无状态暗灰。
- [x] 当前页仍显示 `❯` 且页标题加粗。
- [x] `cargo fmt`、`cargo clippy --all-targets -- -D warnings`、相关测试与 `cargo test` 全部通过。

---

## 8. 假设与待评审事项

1. `ref-content/ui/prompt/` 不存在，本设计按现有 Recent 抽屉理解；若参考实现是浮层面板而非底部抽屉，需要重新评估 §3.1 / §3.2。
2. 第 2 行条目名采用 muted；若希望条目名保持亮白、仅页标题变色，可在评审时改为 `base`。
3. 无 `&&&` 状态的页标题使用 muted；若希望它和 todo 一样亮白，可改为 `style::text()`。

---

## 9. 复审修正

完整问题清单与根因见 `review.md`。本轮修复：

- **P1-1**：当前页快速路径在页列表视图下切回 `PromptMode::Editor`，避免只关抽屉、画面仍停在页列表。
- **P2-1**：当前页快速路径调用 `focus_prompt`，从窗格焦点打开抽屉后选择条目也会把键盘交还编辑器。
- **P2-2**：补充 Pages 当前页、焦点/光标/修订号、目标页 `updated_at`、动态可见窗口、`Esc` 关闭与
  `InProgress` 悬停状态色测试。
- **P3-1**：`recent_page_count` 注释同步 `MAX_RECENT_PAGES`。
- **P3-2**：同步本设计与 `task.md` 的代码片段、测试名与旁路行为描述。
