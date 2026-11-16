# Prompt 容器滚动隔离 — 设计

- 建议分支：`fix/prompt-scroll-isolation`
- 状态：**已实现，待用户手工验收（2026-11-16）**
- 关联文档：`docs/feature/prompt-scroll-guard/`、`docs/feature/prompt-per-workspace/`、`docs/feature/prompt-header-and-recent-pages/`

## 1. 背景与目标

**Bug**：在 `src/ui/left_panel/` 的 Workspaces 列表中切换 item（切换激活工作区）后，右侧 `src/ui/right_panel/prompt/` 的滚动位置会变化：典型复现是把长提示词正文滚到顶部（光标仍在文末），切换工作区后视口被拽回光标附近/底部；树列表、Pages 列表、Recent 抽屉也存在同类“自动改滚动”的路径。

**期望**：

1. 正常情况下切换工作区不改变 prompt 面板任何容器的滚动位置。
2. 每个容器的滚动条互不影响：正文视口 / 树列表 / Pages 列表 / Recent 抽屉各自持有滚动状态；自动路径只能做范围钳制，不能主动滚动。
3. 左侧栏自身列表（Workspaces / Agents / 历史列表）行为不变（见 §3 决策 7）。

**非目标**：

- 不改 per-workspace 草稿模型（`Workspace.prompt: Option<PromptDraft>`）。
- 不改“打开另一个条目 / 切换到另一页 = 换文档”的定位语义（`Prompt::set_text` 仍把光标放文末并滚到光标）。
- 不改左侧 Workspaces/Agents/历史列表的跟随与复位行为。
- 不新增任何持久化字段、不涉及 DB/schema。

## 2. 复现与根因

### 2.1 主链路：切换工作区 → 编辑器视口被拉回光标（必现）

1. 左栏点击工作区 item：`src/event/mouse/hit.rs:82-90` → `update::switch_workspace`。
2. `switch_workspace`（`src/app/update/workspace.rs:332-357`）在未钉住时调用 `swap_display_prompt`（`src/app/update/editor.rs:384-412`），把目标工作区的草稿换进 `state.prompt`。
3. 每份草稿的编辑器各带独立 `PaneId`（`src/app/state/workspace.rs:41-53` `PromptDraft::new` → `Prompt::new(PaneId::alloc())`），因此切换后 `state.prompt.id()` 变化。
4. `current_geometry` 把 `(state.prompt.id(), view.right_panel)` 放入几何列表（`src/event/session.rs:79-81`），id 变化 ⇒ 下一轮 `geometry.rects != last_rects`。
5. 事件循环因此调用 `resize_panes`（`src/event/mod.rs:474-477`）；prompt 分支执行 `state.prompt.resize(cols, rows)`（`src/app/update/pane.rs:848-871`）。
6. `Prompt::resize` **无条件**调用 `scroll_cursor_into_view()`（`src/app/prompt.rs:382-387`）：

```rust
// 现状 src/app/prompt.rs
pub fn resize(&mut self, cols: u16, rows: u16) {
    self.width = cols.max(1);
    self.height = rows.max(1);
    self.scroll_cursor_into_view(); // ← 切换草稿时也执行，把视口拉回光标
}
```

`scroll_cursor_into_view`（`src/app/prompt/viewport.rs:472-490`）在光标行 `< scroll` 时把 `scroll` 设为光标行，在光标行 `>= scroll + height` 时设为 `row + 1 - height`。长文本滚到顶部、光标在文末时，切换即被拉到底部。

> 关键事实：几何列表差异只来自草稿 `PaneId` 变化，**面板尺寸本身没有变**；但 `resize` 不区分“尺寸是否真的变化”，照样执行光标回拉。

### 2.2 次要链路：切换工作区时树 / Pages 列表被“选中项滚入视口”

`swap_display_prompt` 末尾调用 `restore_prompt_view`（`src/app/update/editor.rs:416-453`）：

```rust
// 现状（节选）
Some(PromptEditing::Item(id)) if panel.tree.item(id).is_some() => {
    select_prompt_item(state, index, id);          // 内部 scroll_selection_into_view
    if prev_mode == PromptMode::Pages {
        ...
        clamp_page_view(panel, rows_len, visible); // 内部同样跟随选中行
    }
    return;
}
```

- `select_prompt_item`（`src/app/update/prompt/tree_view.rs:330-350`）在选中后调用 `scroll_selection_into_view`（同文件 `443-456`）。
- `clamp_page_view`（`src/app/update/prompt/pages_view.rs:432-449`）除了钳制范围，还无条件把选中行滚进视口。

结果：即使树/Pages 当前不可见，切换工作区也会改写目标工作区这两个列表的滚动偏移；用户下次打开列表发现位置被换掉。

### 2.3 返回编辑视图：`restore_history` 再次回拉光标

`restore_history`（`src/app/prompt.rs:359-380`）恢复同一目标/同一页的编辑历史时：

```rust
self.scroll = history.scroll.min(self.max_scroll());
self.scroll_cursor_into_view(); // ← 光标在视口外时覆盖刚恢复的 scroll
```

树/页 → 编辑视图往返走 `edit_prompt_item → load_editor_target → restore_history`（`src/app/update/prompt/items.rs:118-135`），因此“看顶部 → 打开列表 → 返回”也会丢失阅读位置。

### 2.4 后台维护：停泊草稿被 `set_text` 重置

两条后台路径会刷新**未显示**的工作区草稿：

- `apply_prompt_tree`：同一页正文更新（`src/app/update/prompt/mod.rs:155-169`）→ `draft.editor.set_text(&active_content)`。
- `mark_prompt_page_persisted`：保存回执刷新同键停泊缓冲（`src/app/update/prompt/save.rs:155-163`）→ `draft.editor.set_text(content)`。

`set_text` 把 `scroll = 0` 后执行 `scroll_cursor_into_view()`（`src/app/prompt.rs:310-330`），光标在文末 ⇒ 草稿在后台被滚到底部。用户切回该工作区时，看到的是已被后台改掉的位置。

### 2.5 Recent 抽屉：自动同步 `keep_visible`

`sync_prompt_recent_selection`（`src/app/update/prompt/recent.rs:347-383`）在自动保存重排、条目/页增删、树重载后调用，并用 `keep_visible`（同文件 `256-265`）把 `recent_active` 拉进视口 ⇒ 自动路径改写了抽屉滚动。

## 3. 已确认决策（Grill Me 收口）

| # | 决策项 | 结论 |
|---|--------|------|
| 1 | 主复现场景 | 以**编辑视图正文视口**为主；树 / Pages / Recent 一并纳入“切换工作区不得改变任何容器滚动位置” |
| 2 | 真实尺寸变化（窗口缩放、Recent 开合、侧栏拖宽） | **混合 C**：尺寸变化前光标可见，才继续保持光标可见；用户已把光标滚出视口时绝不回拉 |
| 3 | 自动路径的范围 | **全局**：任何非用户滚动/导航意图的路径都不得改动列表滚动，**包括进入树视图时的“选中项滚入视口”**；只允许钳制 |
| 4 | 自动恢复编辑视图 | 工作区切换、树/页 → 编辑返回都只钳制 `scroll`；只有输入、方向键/翻页、点击定位、滚轮等用户动作才把光标滚进视口 |
| 5 | 进入树 / Pages 视图 | **保留该列表上次滚动位置，只钳制**；当前选中项/当前页可能在视口外，不主动滚过去（Pages 进入不再重置为 0） |
| 6 | Recent 抽屉自动同步 | 只更新 `recent_active` + 钳制 `recent_scroll`；不为让选中项可见而滚动；`keep_visible` 只保留在用户 ↑↓ / 滚轮 / 拖动路径 |
| 7 | 左侧栏边界 | 只修右侧 prompt 面板；Workspaces 跟随激活项、Agents 随工作区复位、历史列表行为均保持不变 |
| 8 | 停泊草稿后台刷新 | **同一页**正文后台刷新保留原 `scroll`（只钳制，光标越界钳到文末）；页身份切换 / 打开另一条目仍走 `set_text` 的换文档语义 |

## 4. 统一不变量

**I1（所有权）**：滚动偏移只归该容器自身所有。`Prompt.scroll` / `PromptPanel.scroll` / `PageListView.scroll` / `PromptPanel.recent_scroll` 互不读写，也不因其它容器或其它工作区的动作变化。

**I2（自动路径只钳制）**：所有自动路径——工作区切换、视图进入/返回、树重载、保存回执、增删改后的同步、几何变化同步——只允许：

- 把偏移钳到 `0..=max`（`max = total.saturating_sub(visible)`）；
- 把选中索引钳到合法行/页；
- **不得**为让选中项/光标可见而主动滚动。

**I3（用户滚动意图）**：只有以下动作可以改变滚动偏移或把选中项/光标滚进视口：

- 鼠标滚轮；
- 滚动条 thumb 拖拽、轨道点击；
- 列表 ↑/↓/Home/End/PageUp/PageDown / 翻页键；
- 树/页搜索框输入导致过滤集变化（允许把首项/当前项定位可见）；
- 正文编辑、光标移动、点击定位、粘贴等显式编辑动作（`scroll_cursor_into_view`）；
- 显式的新建页/打开页/折叠展开等用户导航动作（仅当实现处明确需要跟随选中项时保留）。

**I4（尺寸混合规则）**：`Prompt::resize`：

- 尺寸未变 ⇒ **直接返回**，不做任何滚动调整（这是切换草稿时几何重排的常态）；
- 尺寸变化 ⇒ 记录变化前 `cursor_cell().is_some()`；更新尺寸后先钳制 `scroll`；仅当“变化前可见”且“变化后不可见”时才 `scroll_cursor_into_view()`。

**I5（换文档例外）**：打开另一个条目 / 切换页 / 页身份变化属于“换文档”，沿用 `set_text`：光标置文末、滚到光标、清空撤销历史。此例外不适用于同一页正文的后台刷新。

## 5. 实现设计（按文件）

### 5.1 `src/app/prompt.rs` — 编辑器

#### 5.1.1 `resize` 改为混合规则

```rust
// 目标实现
pub fn resize(&mut self, cols: u16, rows: u16) {
    let next = (cols.max(1), rows.max(1));
    if (self.width, self.height) == next {
        return; // 容器尺寸未变：草稿 id 变化触发的几何重排不得动视口
    }
    let cursor_was_visible = self.cursor_cell().is_some();
    self.width = next.0;
    self.height = next.1;
    self.scroll = self.scroll.min(self.max_scroll());
    if cursor_was_visible && self.cursor_cell().is_none() {
        self.scroll_cursor_into_view();
    }
}
```

要点：

- `cursor_cell()` 依赖旧的 `width/height/scroll`，必须在更新字段前取值。
- 尺寸未变时连 `min(max)` 都不做：内容未变，旧 `scroll` 必然仍合法；也避免任何隐蔽位移。
- 尺寸变化时先钳制，防止 Recent 抽屉开合导致 `max_scroll` 变小后状态越界（渲染层 `widgets::scrollbar::layout` 虽会钳制，但状态应自洽）。

#### 5.1.2 `restore_history` 只钳制

```rust
// 现状
self.scroll = history.scroll.min(self.max_scroll());
self.scroll_cursor_into_view();

// 目标
self.scroll = history.scroll.min(self.max_scroll());
```

同目标/同页的历史恢复不得回拉光标（决策 4）；光标重新可见由下一次编辑/导航触发。

#### 5.1.3 新增 `replace_text_preserving_view`

用于停泊草稿的**同一页正文**后台刷新（决策 8）：

```rust
/// 后台同步同一页正文：保留视口位置（只钳制），保留光标偏移（越界钳到字符边界/文末），
/// 不像 `set_text` 那样把光标放文末并滚回视口。
pub fn replace_text_preserving_view(&mut self, text: &str) {
    let scroll = self.scroll;
    let cursor = self.cursor;
    self.set_text(text); // 复用净化、清浮层/历史、置已保存态
    self.cursor = clamp_cursor(&self.text, self.text.floor_char_boundary(cursor.min(self.text.len())));
    self.scroll = scroll.min(self.max_scroll());
}
```

说明：`set_text` 仍负责清空撤销/重做、关闭面板、`is_saved = true`、`autosave = None`；随后恢复光标与滚动。`clamp_cursor` 复用于 `src/app/paste.rs`（`viewport.rs` 已在用）。

### 5.2 树列表（`src/app/update/prompt/tree_view.rs`、`src/app/prompt/tree/panel.rs`）

#### 5.2.1 `PromptPanel::clamp_selection` 只钳制

```rust
// 现状 src/app/prompt/tree/panel.rs:465-480
pub fn clamp_selection(&mut self, visible: usize) {
    let rows = self.rows();
    let len = rows.len();
    self.selected = snap_selectable(&rows, self.selected.min(len.saturating_sub(1)));
    self.scroll = self.scroll.min(len.saturating_sub(visible.max(1)));
    if self.selected < self.scroll {
        self.scroll = self.selected;                       // ← 删除
    } else if self.selected >= self.scroll + visible.max(1) {
        self.scroll = self.selected + 1 - visible.max(1);   // ← 删除
    }
}
```

调用方影响：

- `show_prompt_tree`（`src/app/update/prompt/mod.rs:201-222`）：进入树视图不再被 `clamp_selection` 拉走；
- `apply_prompt_tree`（同文件 120-140）：树重载只钳制；
- `delete_prompt_item` / `delete_prompt_folder` / `delete_prompt_temp`（`items.rs`）：删除后的选中吸附不滚动；
- `apply_prompt_search`（`tree_view.rs:229-240`）：搜索词非空时仍显式 `scroll_selection_into_view`（用户输入），清空时只钳制。

#### 5.2.2 服务型选中不改滚动

`select_prompt_item`（`tree_view.rs:330-350`）、`select_prompt_temp`（`352-370`）、`select_prompt_folder`（`156-177`）只设置 `selected`，删除内部的 `scroll_selection_into_view` 与不再需要的 `visible` 计算。

保留 `scroll_selection_into_view`（`443-456`）仅给用户导航路径：

- `move_prompt_selection`（↑/↓）；
- `apply_prompt_search` 的非空过滤；
- 文件夹折叠/展开（键盘 Left/Right 或点击，用户显式导航）；
- `clamp_prompt_tree_scroll` 改为仅钳制（当前无生产调用，仅测试使用；若实现后仍无调用方则一并删除并调整测试，避免留死代码）。

#### 5.2.3 进入树视图

`show_prompt_tree` 保持现有流程，但：

- `select_tree_entry_row` 走新的无滚动 `select_prompt_item` / `select_prompt_temp`；
- `clamp_selection` 只钳制；
- 因此 `panel.scroll` 保持进入前的位置（决策 5）。选中项可能在视口外，按 ↑/↓ 导航时由 `move_prompt_selection` 正常拉入视口。

### 5.3 Pages 列表（`src/app/update/prompt/pages_view.rs`、`pages.rs`）

#### 5.3.1 拆分“钳制”与“跟随选中”

```rust
// 目标：自动路径使用
pub(crate) fn clamp_page_view(panel: &mut PromptPanel, rows_len: usize, visible: usize) {
    let visible = visible.max(1);
    if rows_len == 0 {
        panel.pages_view.selected = 0;
        panel.pages_view.scroll = 0;
        return;
    }
    panel.pages_view.selected = panel.pages_view.selected.min(rows_len - 1);
    panel.pages_view.scroll = panel
        .pages_view
        .scroll
        .min(rows_len.saturating_sub(visible));
}

// 目标：用户导航路径使用（原 keep_visible 逻辑）
fn reveal_page_selection(panel: &mut PromptPanel, rows_len: usize, visible: usize) {
    let visible = visible.max(1);
    if rows_len == 0 {
        panel.pages_view.selected = 0;
        panel.pages_view.scroll = 0;
        return;
    }
    panel.pages_view.selected = panel.pages_view.selected.min(rows_len - 1);
    panel.pages_view.scroll = panel
        .pages_view
        .scroll
        .min(rows_len.saturating_sub(visible));
    if panel.pages_view.selected < panel.pages_view.scroll {
        panel.pages_view.scroll = panel.pages_view.selected;
    } else if panel.pages_view.selected >= panel.pages_view.scroll + visible {
        panel.pages_view.scroll = panel.pages_view.selected + 1 - visible;
    }
}
```

调用方分类：

| 调用点 | 路径性质 | 目标函数 |
|--------|----------|----------|
| `show_prompt_pages`（`pages_view.rs:21-56`） | 用户进入视图，但按决策 5 **不重置、不跟随** | `clamp_page_view`（删除 `scroll = 0`） |
| `restore_prompt_view`（`editor.rs:432-438`） | 自动恢复 | `clamp_page_view` |
| `move_page_selection`（`pages_view.rs:112-133`） | 用户 ↑/↓ | `reveal_page_selection` |
| `apply_page_search`（`pages_view.rs:154-199`） | 用户输入过滤 | `reveal_page_selection` |
| `clear_page_search`（`pages_view.rs:203-230`） | 用户清空过滤 | `reveal_page_selection` |
| 新建页 / 打开指定页（`pages.rs:226,270,500`） | 用户显式创建/打开 | `reveal_page_selection` |
| 删除页（`pages.rs:650`） | 用户删除；选中仍在可见处 | `clamp_page_view` |
| `apply_prompt_tree` / 页数量变化自动同步（`pages.rs:226` 以外的后台路径） | 自动 | `clamp_page_view` |

#### 5.3.2 `show_prompt_pages` 保留上次位置

删除 `panel.pages_view.scroll = 0;`，进入后只做：

```rust
panel.pages_view.selected = selected; // 当前页行
clamp_page_view(panel, total, visible);
```

仍清空搜索词（现有行为）；清空后行集可能变化，偏移只钳制。按决策 5，当前页可能在视口外，不主动定位。

### 5.4 Recent 抽屉（`src/app/update/prompt/recent.rs`）

`sync_prompt_recent_selection`：

```rust
// 现状
panel.recent_active = Some(next);
panel.recent_scroll = keep_visible(panel.recent_scroll, next, visible, rows.len());

// 目标（自动同步只钳制）
panel.recent_active = Some(next);
let max = rows.len().saturating_sub(panel.recent_visible.max(1));
panel.recent_scroll = panel.recent_scroll.min(max);
```

`keep_visible` 与 `clamp_recent_active` 保留给：

- `move_prompt_recent`（键盘 ↑/↓）；
- `scroll_prompt_recent`（滚轮）；
- `set_prompt_recent_scroll`（滚动条）；
- `toggle_prompt_recent` 打开时按当前页定位（用户动作）。

### 5.5 停泊草稿后台刷新

- `src/app/update/prompt/mod.rs:155-169`：`draft.editor.set_text(&active_content)` → `draft.editor.replace_text_preserving_view(&active_content)`。
- `src/app/update/prompt/save.rs:155-163`：同上。
- `reload_editor_from_active_page`（`pages.rs:681-708`）不改：它服务页切换/删除当前页等“换文档”路径（决策 8 / I5）。

### 5.6 左侧栏

`src/ui/left_panel/` 与 `src/app/update/workspace_list.rs` 不改：

- `ensure_workspace_visible` 继续让激活项在 Workspaces 列表可见；
- `switch_workspace` 继续 `agent_scroll = 0` / `agent_hover = None`（Agents 条目随工作区替换）；
- 历史列表现有重置/钳制逻辑不动。

## 6. 边界与例外

1. **必须钳制的位移**：内容/行数变少、视口变高导致 `max` 变小，`scroll` 必须钳到新上限；这是不变量允许的唯一“位置变化”。
2. **首次显示某草稿且尺寸过期**：草稿在停泊期间窗口被缩放，切回时尺寸确实变化 ⇒ 按 I4：变化前光标可见才跟随；否则仅钳制。
3. **换文档**：打开另一条目、切换页、页身份变化 ⇒ `set_text` 语义（光标文末 + 滚到光标）。
4. **搜索过滤**：输入过滤词会改变行集合并把选中落回首项；允许 `reveal_page_selection` / `scroll_selection_into_view`（用户输入路径）。
5. **渲染层**：`ui/widgets/scrollbar::layout` 已对 offset 做 `min(max)`，无需改动；本次只改状态层。
6. **拖拽中的滚动条**：若切换发生时正在拖拽某容器 thumb，现有代码在几何变化时结束拖拽（`stop_selection_autoscroll`）与各 drag 结束路径保持不变；本次不新增跨容器联动。

## 7. 测试计划（先红后绿）

### 7.1 状态层 `src/app/prompt/tests.rs`

- `resize_same_size_keeps_manual_scroll`：长文本、光标在文末、`scroll_to(0)`，`resize` 同尺寸 ⇒ `scroll == 0`、`cursor_cell().is_none()`。
- `resize_shrink_keeps_visible_cursor_visible`：光标可见时缩小尺寸 ⇒ `cursor_cell().is_some()`（I4 前半）。
- `resize_shrink_keeps_user_scrolled_away_cursor`：光标在文末、滚到顶部后缩小尺寸 ⇒ `scroll` 不变（仅钳制），不把光标拉回。
- `restore_history_keeps_scrolled_view`：长文本、光标文末、`scroll_to(0)`，取 `history()` 后 `restore_history` ⇒ `scroll == 0`。
- `replace_text_preserving_view_keeps_scroll_and_clamps_cursor`：`scroll_to(5)` 后用更短正文替换 ⇒ `scroll <= 新 max` 且不被拉到文末；光标在合法字符边界。

### 7.2 更新层 `src/app/update/prompt/tests.rs` / `pages_view/tests.rs` / `recent/tests.rs`

- `workspace_switch_preserves_editor_scroll`：工作区 0 长正文滚到顶部 → `switch_workspace` + 模拟 `resize_panes`（prompt id 变化后的真实路径）→ 切回，`scroll` 保持。
- `workspace_switch_preserves_target_tree_scroll`：目标工作区树 `scroll = 0`、`selected` 在下方；`swap_display_prompt` 后 `panel.scroll == 0`。
- `workspace_switch_preserves_target_pages_scroll`：目标工作区 Pages 模式、`pages_view.scroll = 0`、`selected` 在下方；`swap_display_prompt` 后 `pages_view.scroll == 0`。
- `tree_roundtrip_keeps_scroll_when_cursor_out_of_view`：在现有 `edit_position_and_scroll_preserved_when_switching_to_tree_and_back` 旁补一条“光标在视口外”的用例。
- `show_prompt_tree_preserves_scroll` / `show_prompt_pages_preserves_scroll`：进入视图不改 `scroll`（决策 5）。
- `background_parked_refresh_preserves_scroll`：停泊草稿 `scroll = 5`，同一页正文经 `apply_prompt_tree` / `mark_prompt_page_persisted` 刷新后仍为 5（或被钳制到新 max）。
- `recent_auto_sync_does_not_scroll`：`recent_open`、`recent_scroll = 0`、目标行在下方，`sync_prompt_recent_selection` 后仍为 0。

### 7.3 事件层 `src/event/tests/`

- `workspace_item_click_keeps_prompt_scroll`：真实鼠标点击 Workspaces item（走 `handle_sidebar_press → switch_workspace → resize_panes`）后，`state.prompt.scroll()` 不变；`prompt_scrollbar` 的 thumb 位置不变。

### 7.4 需要同步修改的既有测试

- `src/app/prompt/tests.rs::resize_keeps_cursor_visible_and_guards_zero_size`：按 I4 拆成“尺寸变化且光标原本可见”与“尺寸未变”两段。
- `src/app/update/prompt/tests.rs::scroll_clamps_and_keeps_selection_visible`：`clamp_prompt_tree_scroll` 改为只钳制后，断言不再要求选中项可见。
- `src/app/update/prompt/pages_view/tests.rs` 中依赖 `show_prompt_pages` 重置 `scroll = 0` 的断言（如存在）按决策 5 更新。
- `src/app/update/prompt/recent/tests.rs` 中依赖自动同步 `keep_visible` 的断言改为只钳制。

## 8. 风险与回滚

| 风险 | 影响 | 缓解 |
|------|------|------|
| 进入树/页视图不 reveal 选中项，用户找不到高亮行 | 可用性下降 | 按决策 5 执行；↑/↓ 首次按键即拉入视口；文档明确 |
| 尺寸缩小时光标暂时不可见 | 编辑体验 | 决策 2 的“变化前可见才继续可见”保证正在输入时不丢光标；其余情况下一次编辑即拉回 |
| 后台刷新保留旧 scroll 但正文大幅变化 | 视口落在内容中段 | 只钳制到合法范围；换页/换条目走 `set_text` 重置 |
| 搜索过滤后保存旧 scroll | 过滤集变化导致视觉跳位 | 过滤路径属于用户输入，保留 reveal；仅自动同步不 reveal |

回滚：无 DB/schema/协议变更；按文件 revert 即可。建议实现拆成两个提交：①状态层（`prompt.rs` / tree / pages / recent）②调用点与测试；回滚粒度清晰。

## 9. 验收标准

1. 单元/更新/事件层新增与修改测试全部通过。
2. 手工验收（`cargo run`）：
   - 长正文滚到顶部（光标在文末）→ 切换 Workspaces item → 位置保持；切回仍保持。
   - 树列表滚到顶部、选中项在下方 → 切换工作区 → 树位置不变；打开树视图位置不变。
   - Pages 列表滚到中部 → 切换工作区 / 进入 Pages → 位置保持（仅必要时钳制）。
   - Recent 抽屉滚到顶部 → 触发自动保存/重排 → 位置保持。
   - 窗口缩放：光标原本可见时仍可见；光标已被滚出视口时位置不跳。
   - 左侧 Workspaces/Agents 列表行为与改动前一致。
3. `cargo fmt`、`cargo clippy --all-targets -- -D warnings`、`cargo test` 通过。
