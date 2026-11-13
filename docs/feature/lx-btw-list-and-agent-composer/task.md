# 实施清单：BTW / Agents 列表-详情导航与统一 composer

> 状态：**已实现并于 2026-11-13 完成 review 修复，待用户运行 `cargo run` 确认后合并**。
>
> 设计依据：[design.md](design.md)；审查记录：[review.md](review.md)。
>
> 本文件中的步骤按依赖顺序排列；生产代码与测试已完成，验证结果见文末“实施结果”。

## 0. 实施前置门禁

- [ ] 用户已逐项确认 `design.md` §2 的 Q1–Q30。
- [ ] 用户已审核本文件的文件拆分与测试矩阵，明确允许开始实现。
- [ ] 当前分支为 `dev`；工作区无未确认的其他改动（本次不修改 Git 状态，不提交、不合并）。
- [ ] 已确认本次**不需要数据库迁移**；`SCHEMA_VERSION` 不动。
- [ ] 实现前记录受影响文件行数，确认拆分后生产文件 ≤1000 行、测试文件 ≤1500 行。

## 1. 文案与常量

### 1.1 `src/ui/text/btw.rs`

- [ ] 删除：
  - `LX_BTW_LIST_COMMAND_DESC`
  - `LX_BTW_LIST_PANEL_TITLE`
  - `LX_BTW_LIST_PANEL_FOOTER`
  - `LX_BTW_LIST_LOADING`
  - `LX_BTW_LIST_EMPTY`
  - 旧 `LX_BTW_HINTS`
- [ ] 保留：
  - `LX_BTW_EMPTY_THREAD`（改为 `No side conversation yet`）
  - `LX_BTW_QA_UNTITLED`
  - `LX_BTW_TARGETS_PREFIX`
  - `LX_BTW_LIST_THREAD_MARK`（`● side thread`）
  - `btw_status_label`
- [ ] 新增：

```rust
pub const LX_BTW_SEARCH_PLACEHOLDER: &str = "search questions…";
pub const LX_BTW_SEARCH_EMPTY: &str = "no matching questions";
pub const LX_BTW_LIST_LOADING: &str = "Loading main questions...";
pub const LX_BTW_LIST_EMPTY: &str = "No main questions yet";
pub const LX_BTW_INPUT_MARK: &str = "BTW";
pub const LX_BTW_HINTS_LIST: [crate::ui::text::Hint; 4] = [
    crate::ui::text::hint("select", "↑↓"),
    crate::ui::text::hint("enter", "⇧↵"),
    crate::ui::text::hint("tabs", "←→"),
    crate::ui::text::hint("close", "esc"),
];
pub const LX_BTW_HINTS_EDIT: [crate::ui::text::Hint; 4] = [
    crate::ui::text::hint("send", "↵"),
    crate::ui::text::hint("newline", "⇧↵"),
    crate::ui::text::hint("stop", "^d"),
    crate::ui::text::hint("back", "esc"),
];
```

> 文案最终以 `ui::text::key` 的符号规范为准：`⇧`/`↵` 单宽，禁止写 `shift`/`enter` 全拼。

### 1.2 `src/ui/text.rs`

- [ ] 删除 `LX_TASK_HINTS_AGENT_REPLY`。
- [ ] 删除 `LX_TASK_HINTS_AGENTS` 中的 `reply` 与 `stop all`。
- [ ] 新增：

```rust
pub const LX_TASK_AGENTS_SEARCH_PLACEHOLDER: &str = "search agents…";
pub const LX_TASK_AGENTS_SEARCH_EMPTY: &str = "no matching agents";
pub const LX_TASK_AGENTS_LIST_LOADING: &str = "Loading agents...";
pub const LX_TASK_AGENTS_LOADING: &str = "loading agent transcript…";
pub const LX_TASK_AGENT_INPUT_MARK: &str = "Subagent";
pub const LX_TASK_HINTS_AGENTS_LIST: [Hint; 6] = [
    hint("select", "↑↓"),
    hint("filter", "tab"),
    hint("enter", "⇧↵"),
    hint("stop", "^d"),
    hint("tabs", "←→"),
    hint("close", "esc"),
];
pub const LX_TASK_HINTS_AGENTS_EDIT: [Hint; 4] = [
    hint("send", "↵"),
    hint("newline", "⇧↵"),
    hint("stop", "^d"),
    hint("back", "esc"),
];
```

- [ ] `LX_TASK_AGENT_REPLY_CLOSED` 保留，供 cancelled agent 拒绝进入编辑态时使用。

### 1.3 测试

- [ ] 更新文案单宽/符号规范测试，确保没有 `shift`/`enter`/`ctrl` 全拼。
- [ ] 断言 `/btwList` 相关文案常量不再存在。

## 2. 状态层基础

### 2.1 新增 `src/app/state/lx/composer.rs`

- [ ] 从 `src/app/state/lx/btw.rs` 抽出 `BtwComposer` 为 `DetailComposer`：

```rust
use crate::app::state::Lx;

#[derive(Debug)]
pub struct DetailComposer(Box<Lx>);

impl DetailComposer {
    pub fn new_btw() -> Self { Self(Box::new(Lx::composer())) }
    pub fn new_agent() -> Self { Self(Box::new(Lx::agent_composer())) }
    pub fn editor(&self) -> &Lx { &self.0 }
    pub fn editor_mut(&mut self) -> &mut Lx { &mut self.0 }
    pub fn text(&self) -> &str { &self.0.input }
    pub fn is_empty(&self) -> bool { self.0.input.trim().is_empty() }
    pub fn clear(&mut self) { self.0.clear(); }
    pub fn revision(&self) -> u64 { self.0.revision() }
}
```

- [ ] 在 `src/app/state/lx.rs` 的 `pub use` 中导出 `DetailComposer`。
- [ ] `BtwThreadState.composer` 类型改为 `DetailComposer`，构造处从 `BtwComposer::new()` 改为 `DetailComposer::new_btw()`。
- [ ] 删除 `BtwComposer` 旧定义与 5 处引用。

### 2.2 `Lx::agent_composer()` 与 `allow_panels`

- [ ] `src/app/state/lx.rs` 增加私有字段：

```rust
allow_panels: bool,
```

- [ ] `build()` 默认 `allow_panels: true`。
- [ ] 新增：

```rust
pub fn agent_composer() -> Self {
    let mut lx = Self::build(false);
    lx.no_shell = true;
    lx.allow_panels = false;
    lx
}
```

- [ ] `refresh_panel()` 开头增加：

```rust
if !self.allow_panels {
    self.panel = None;
    self.slash = None;
    self.paste = None;
    self.text_paste = None;
    self.refer = None;
    self.mention.suppress();
    self.scroll_cursor_into_view();
    return;
}
```

- [ ] 确认 `Lx::composer()` 保持 `allow_panels = true`，不影响 BTW 的 `@`/粘贴/引用。

### 2.3 `TaskPage` 字段

`src/app/state/lx/task.rs`：

- [ ] 删除：

```rust
pub agent_reply: Option<TextInput>,
pub agent_reply_target: Option<(String, String)>,
```

- [ ] 新增：

```rust
use crate::app::overlay::TextInput;

pub agents_search: TextInput,
pub agent_edit: Option<String>,
pub agents_loading: bool,
```

- [ ] `Default`：`TextInput` 目前没有 `Default`；先为它实现 `Default { Self::new(String::new()) }`，这样 `TaskPage` / `LxBtw` 可继续 derive `Default`。确认 `TaskPage` 仍可 `Clone/PartialEq/Eq`。
- [ ] 在 `impl TaskPage` 增加：

```rust
pub fn agents_editing(&self) -> bool { self.agent_edit.is_some() }
pub fn agent_visible_indices(&self, agents: &AgentsState) -> Vec<usize> {
    agents.visible_indices(self.agent_filter, self.agents_search.text())
}
```

- [ ] 所有 `TaskPage` 字面量测试用 `..TaskPage::default()` 补齐新字段。

### 2.4 `LxBtw`

`src/app/state/lx/btw.rs`：

- [ ] 删除 `BtwFocus` 及 `focus` 字段、`next/previous`、`clear()` 中的重置。
- [ ] 删除 `picking`、`pending_pick`、`picker_loading` 及全部 picker 方法。
- [ ] 新增：

```rust
use crate::app::overlay::TextInput;
use crate::app::state::lx::slash::PanelWindow;

pub editing: bool,
pub search: TextInput,
pub qa_window: PanelWindow,
```

- [ ] `BtwThreadState::new` 的 `composer` 改为 `DetailComposer::new_btw()`。
- [ ] `LxBtw::clear()` 重置 `editing=false`、`search.clear()`、`qa_window = PanelWindow::new(0)`。
- [ ] 新增方法：

```rust
pub fn filtered_qa_indices(&self) -> Vec<usize>;
pub fn selected_index(&self) -> Option<usize>;
pub fn move_selection(&mut self, delta: isize) -> bool;
pub fn enter_edit(&mut self) -> bool;
pub fn leave_edit(&mut self) -> bool;
pub fn clear_search(&mut self) -> bool;
pub fn sync_selection_after_filter(&mut self);
```

过滤实现：

```rust
let query = self.search.text().trim().to_lowercase();
self.qa
    .iter()
    .enumerate()
    .filter(|(_, q)| query.is_empty() || q.title.to_lowercase().contains(&query))
    .map(|(index, _)| index)
    .collect()
```

- [ ] `sync_selection_after_filter`：
  - 用 `active` 在过滤结果中找位置，更新 `qa_window.active`；
  - 找不到且列表非空：`qa_window.active = 0`，但保留 `active` 供详情回退；
  - 列表空：`qa_window.active = 0`；
  - `qa_window.viewport = None`。
- [ ] `move_selection`：按过滤结果移动 `qa_window.active`，把对应 `qa.qa_id` 写入 `active`；不关闭面板；到两端返回 false。

### 2.5 `AgentsState`

`src/app/state/lx/agent.rs`：

- [ ] 新增：

```rust
use std::collections::BTreeMap;
use super::composer::DetailComposer;

pub composers: BTreeMap<String, DetailComposer>,
/// 稳定选中的 agent_id：面板关闭/搜索过滤后仍保留。
pub selected_id: Option<String>,
```

- [ ] 所有测试 `AgentsState { ... }` 补 `..Default::default()`。
- [ ] 新增 `visible_indices(filter, query)`：

```rust
pub fn visible_indices(&self, filter: AgentFilter, query: &str) -> Vec<usize> {
    let query = query.trim().to_lowercase();
    self.runs
        .iter()
        .enumerate()
        .filter(|(_, run)| {
            filter.matches(run)
                && (query.is_empty()
                    || run.display_identity().to_lowercase().contains(&query)
                    || run.role.as_str().to_lowercase().contains(&query)
                    || run.title.to_lowercase().contains(&query)
                    || run.agent_id.to_lowercase().contains(&query))
        })
        .map(|(index, _)| index)
        .collect()
}
```

- [ ] 保留 `filtered_indices(filter)` 原实现，供 `mention_runs` 与状态恢复使用；不要让它受搜索影响。
- [ ] 新增：

```rust
pub fn composer(&self, agent_id: &str) -> Option<&DetailComposer>;
pub fn composer_mut(&mut self, agent_id: &str) -> Option<&mut DetailComposer>;
pub fn ensure_composer(&mut self, agent_id: &str) -> &mut DetailComposer;
```

- [x] 不保留未调用的 `remove_composer`；当前 run 列表只增不减，会话切换/`clear_agents` 时整体清空草稿。

### 2.6 `Lx` 的活动编辑器

`src/app/state/lx.rs`：

- [ ] 新增：

```rust
pub fn is_agents_tab(&self) -> bool;
pub fn btw_editing(&self) -> bool;
pub fn agents_editing(&self) -> bool;
pub fn agent_edit_id(&self) -> Option<&str>;
```

- [ ] `active_editor()`：

```rust
pub fn active_editor(&self) -> &Lx {
    if self.btw_editing()
        && let Some(composer) = self.btw.active_composer()
    {
        return composer;
    }
    if self.agents_editing()
        && let Some(agent_id) = self.agent_edit_id()
        && let Some(composer) = self.agents.composer(agent_id)
    {
        return composer.editor();
    }
    self
}
```

- [ ] `with_active_editor` 同步扩展。
- [ ] **列表态不要返回隐藏 composer**：`btw_editing()` 为 false 时返回主 `Lx`；所有列表态操作必须在调用 `active_editor` 前显式处理。

### 2.7 测试

- [ ] `cargo test --lib app::state::lx::btw app::state::lx::agent app::state::lx::task`
- [ ] 新增：
  - `btw_search_filters_titles_case_insensitively`
  - `btw_selection_uses_stable_qa_id_after_filter`
  - `agent_visible_indices_combines_filter_and_query`
  - `agent_composer_is_lazy_and_keyed_by_agent_id`
  - `active_editor_stays_on_main_input_in_list_mode`

## 3. 详情渲染统一

### 3.1 新增 `src/ui/main_content/lx/task/detail.rs`

- [ ] 从 `task/agents.rs` 抽取并参数化：
  - `draw_detail`
  - `detail_rows` / `measure`
  - `anchor_before`
  - `anchor_row`
  - `scrollbar`
  - `draw_empty_detail`
- [ ] 定义：

```rust
pub(crate) struct DetailView<'a> {
    pub source: HistorySource<'a>,
    pub scroll: usize,
    pub follow: bool,
}

pub(crate) enum EmptyState<'a> {
    Placeholder(&'a str),
    Loading { label: &'a str, frame: usize },
}
```

- [ ] `draw`：
  - `!source.has_history()` -> 按 `EmptyState` 画占位/呼吸 loading；
  - 有历史 -> `view_source_plain` + `render_source` + `scrollbar`；
  - 滚动口径 `top = if follow { max } else { scroll.min(max) }`。
- [ ] `measure`：空/未加载返回 `(1, true)`，否则 `cache.set_viewport_rows` + `history::measure_cached`。
- [ ] `anchor_before` / `anchor_row`：照搬 `agents.rs` 现有逻辑，改为接收 `DetailView`。

### 3.2 Agents 接入

- [ ] `task/agents.rs` 的 `draw_detail` / `detail_rows` / `anchor_before` / `anchor_row` 改为薄包装：

```rust
let view = DetailView {
    source: HistorySource::from_run(run).with_viewport(HistoryViewport {
        scroll: run.scroll,
        follow_bottom: run.follow,
    }),
    scroll: run.scroll,
    follow: run.follow,
};
```

- [ ] 空态：`run.loaded ? Placeholder(text::LX_TASK_AGENTS_EMPTY) : Loading { label: text::LX_TASK_AGENTS_LOADING, frame: loading_frame }`。
- [ ] 保持 `agents_list` 与详情渲染解耦。

### 3.3 BTW 接入

- [ ] `task/btw.rs` 的 `draw_conversation` / `detail_rows` 改为 `DetailView`：

```rust
let view = DetailView {
    source: HistorySource::from_chat(&thread.chat, &thread.compact).with_viewport(
        HistoryViewport { scroll: thread.scroll, follow_bottom: thread.follow },
    ),
    scroll: thread.scroll,
    follow: thread.follow,
};
```

- [ ] 空态：
  - `!thread.loaded` -> `Loading { label: text::LX_BTW_THREAD_LOADING, frame: thread.chat.loading_frame }`
  - 已加载为空 -> `Placeholder(text::LX_BTW_EMPTY_THREAD)`
- [ ] 无线程视图（active 但无 thread）-> `Placeholder(text::LX_BTW_EMPTY_THREAD)`，不写库。

### 3.4 视口解析

`src/app/update/chat/viewport.rs`：

- [ ] `resolve_task_btw_visible` 改成与 `resolve_task_agents_visible` 相同的锚点流程：
  1. 读取旧快照锚点（`detail::anchor_before`）；
  2. composer 尺寸变化时 `editor.resize(width, rows)`；
  3. 重新量行（`detail::measure`）；
  4. 用 `anchor_row` 修正 `scroll`；
  5. 更新 `visible/total_rows/rows_dirty/follow/scroll`。
- [ ] 仅在编辑态 resize composer；列表态不碰隐藏 composer。
- [ ] 新增 Agents composer resize（编辑态）：
  - 从 `agents::composer_rect` 取宽高；
  - `editor.resize(width, rows)`。
- [ ] `invalidate_task_agent_rows` 同时失效 BTW 线程的 `rows_dirty`（窗口 resize 时两页都重排）。

### 3.5 命中与复制

- [ ] `history_hit.rs`：
  - `task_detail_code_copy_at`、`task_agents_card_target_at`、`task_btw_card_target_at`、`task_agents_scrollbar`、`task_btw_scrollbar` 改用 `DetailView`。
  - `task_output_rect` 在列表/编辑态返回同一对话区。
- [ ] `selection.rs` 的 `press_content_hit` / `unit_cell_range` 保持现有分支，但确认列表态没有 composer 时不误判。
- [ ] `chat_hit` 的 `pass_through` 逻辑不变。

### 3.6 测试

- [ ] 迁移并新增：
  - `btw_detail_loading_matches_agents`
  - `btw_detail_anchor_keeps_scroll_when_streaming`
  - `btw_and_agents_detail_use_same_empty_placeholder`
  - `btw_detail_code_copy_hits`
  - `btw_detail_scrollbar_matches_agents`
- [ ] 运行 `cargo test --lib ui::main_content::lx::task ui::main_content::lx::history_hit ui::main_content::lx::selection app::update::chat::viewport`

## 4. 底部面板布局与绘制

### 4.1 `src/ui/main_content/lx/task.rs`

- [ ] 常量：

```rust
pub(super) const TOP_BORDER: u16 = 1;
pub(super) const TAB_ROW: u16 = 1;
pub(super) const SEARCH_ROW: u16 = 1;
pub(crate) const LIST_MAX: usize = 6;
pub(crate) const LIST_MAX_SEARCH: usize = LIST_MAX - 1;
```

- [ ] 删除 `COMPOSER_GAP`。
- [ ] `task_panel_height`：

```rust
match lx.task.as_ref().map(|page| page.tab) {
    Some(TaskTab::Btw) if lx.btw.editing => composer_height(lx.btw.active_thread().map(|t| t.composer.editor())),
    Some(TaskTab::Agents) if lx.agents_editing() => composer_height(lx.agents.composer(lx.agent_edit_id().unwrap_or_default()).map(|c| c.editor())),
    _ => list_height(),
}
```

- [ ] `composer_height`：`rows.clamp(1,3) + 2`。
- [ ] 新增：

```rust
pub(super) fn search_rect(rect: Rect) -> Rect;
pub(super) fn list_rows_rect(rect: Rect, tab: TaskTab) -> Rect;
pub(super) fn tab_is_searchable(tab: TaskTab) -> bool;
```

- [ ] `draw_list`：
  - 编辑态 -> 直接调用 `btw::draw_edit` / `agents::draw_edit`，不画顶边框/tab/搜索/列表。
  - 列表态 -> 顶边框、tab 行、Agents 筛选胶囊、搜索行、列表、滚动条。
  - 返回光标。
- [ ] `list_rect` 仅保留给旧路径；新代码统一用 `list_rows_rect`。
- [ ] `draw_footer(buf, area, page, lx)`：
  - BTW -> `LX_BTW_HINTS_LIST` / `LX_BTW_HINTS_EDIT`
  - Agents -> `LX_TASK_HINTS_AGENTS_LIST` / `LX_TASK_HINTS_AGENTS_EDIT`
  - 删除 `agent_reply` 分支。
- [ ] `TaskHit` 增加 `Qa(usize)` / `Search`。
- [ ] `hit`：
  - 编辑态只允许 `[close]`? 不，编辑态 tab 行隐藏；返回 None（Esc 退出）。
  - 列表态：close/tab/filter/search/list rows。
  - 列表行命中范围使用 `list_rows_rect`；BTW 映射过滤位置 -> 真实 qa 下标；Agents -> 真实 runs 下标。
- [ ] `list_metrics` / `list_scrollbar`：
  - Processes：6 行。
  - Agents：`page.agent_visible_indices(agents).len()`，5 行。
  - Btw：`lx.btw.filtered_qa_indices().len()`，5 行。
  - 签名从 `(page, agents)` 改为 `(page, lx)` 或增加 `btw` 参数。
- [ ] `draw_search_row`：按 §7.4 实现，复用 `widgets::input`。
- [ ] `Composer` 几何：
  - 删除旧 `btw::composer_rect` 对 `list_rect` 的二次扣减。
  - 编辑态 `task_list` 高度就是 composer 高度。
  - `layout()` 中 `task_list = Some(input)` 保持不变，但 `input` 高度由 `task_panel_height` 决定。

### 4.2 `task/btw.rs`

- [ ] 删除 picker 全部函数（约 150 行）。
- [ ] 删除 `draw_bottom` 的 gap loading；列表态不画 composer。
- [ ] 删除 `chat_bar::draw_inline_loading`（移除 BTW 底部 loading 后无调用者）。
- [ ] 新增 `draw_list`（搜索行 + 5 行 Q）：
  - `❯` 标记；
  - `QA n · title`；
  - 右侧状态；
  - 无结果/空态/加载态；
  - 超宽标题 `…` 截断，`QA n` 与状态优先保留。
- [ ] 新增 `draw_edit`：
  - composer 占满 `task_list`；
  - `input::draw`；
  - `draw_composer_mark("BTW")`；
  - `draw_qa_badge`。
- [ ] `composer_rect(rect, lx)`：
  - 仅 `lx.btw.editing` 返回；列表态 None。
  - 直接使用传入矩形，不再扣 `list_rect`。
- [ ] `detail_rows` / `draw_conversation` 改用 `detail.rs`。
- [ ] 删除 `draw_qa_badge` 中的旧逻辑中依赖 `thread` 的部分可按需调整，但胶囊格式保持。
- [ ] `btw.rs` 如果超过 500 行，拆 `task/btw/list.rs` / `task/btw/composer.rs`；模块声明放 `btw.rs` 或 `task.rs`。

### 4.3 `task/agents.rs`

- [ ] `draw_list` 接收的 `list` 已是搜索行下方 5 行；列宽计算不变。
- [ ] 搜索无结果时 `draw_placeholder(nx, text::LX_TASK_AGENTS_SEARCH_EMPTY)`。
- [ ] 新增 `draw_edit`：
  - 从 `agent_edit` + `agents.composer` 取编辑器；
  - `input::draw`；
  - `draw_composer_mark("Agent")`；
  - 右侧 `To: nickname [role]` 胶囊。
- [ ] 新增 `composer_rect(rect, lx)`：仅编辑态返回。
- [ ] `draw_detail` / `detail_rows` / anchors 改用 `detail.rs`。
- [ ] 删除 `draw_empty_detail` 中重复的 loading 实现，统一调用 `detail.rs`。

### 4.4 `src/ui/main_content/lx.rs`

- [ ] `render()` 与 `build_chat()` 顶部 `task::draw_footer` 传入 `lx`。
- [ ] 删除 `task::btw::render_picker` 调用。
- [ ] 删除 `task_btw_picker_hit`。
- [ ] `layout()`：
  - `task_panel_height` 已覆盖编辑/列表高度；
  - `task_output` 使用 `stack_top`，编辑态自动变大；
  - `task_list` 仍为输入框矩形；
  - `chat_bar` 仍贴 `task_list` 上沿。
- [ ] `task_output_rows` / `task_output_inner` / `task_btw_rows` 保持；新增 `task_agent_composer_rect` 如需。
- [ ] `task_btw_composer_rect`：仅编辑态返回。
- [ ] 检查文件行数；若 >950，把 `layout` / `render` 中任务面板相关分支拆到 `ui/main_content/lx/task_layout.rs` 或在 `task.rs` 下沉。

### 4.5 `src/ui/main_content/lx/panels.rs`

- [ ] `input_rect`：
  - BTW 编辑态 -> `btw::composer_rect`
  - Agents 编辑态 -> `agents::composer_rect`
  - 列表态 -> None
- [ ] `input_geom`：
  - BTW / Agents 编辑态从 `active_editor()` 取行数与滚动；
  - 列表态 None。
- [ ] `chat_bar::bar_rect` 仍取 `input.or(ask).or(task_list)`；编辑态 `task_list` 已是 composer 高度。

### 4.6 测试

- [ ] `task_panel_height_is_8_in_btw_list_mode`
- [ ] `task_panel_height_is_8_in_agents_list_mode`
- [ ] `btw_edit_mode_hides_tab_and_search_and_sizes_to_draft`
- [ ] `agents_edit_mode_hides_tab_and_search_and_sizes_to_draft`
- [ ] `search_row_is_rendered_for_btw_and_agents_only`
- [ ] `list_rows_are_five_with_search_row`
- [ ] `btw_list_selected_row_uses_marker_not_fill`
- [ ] `btw_list_status_label_and_thread_mark`
- [ ] `footer_hints_switch_between_list_and_edit`
- [ ] 迁移旧 `btw_bottom_keeps_the_composer_visible...` / `btw_loading_sits_on_the_gap_row...` 等断言到新布局。

## 5. 输入路由与命令删除

### 5.1 `src/app/update/chat/btw/input.rs`

- [ ] 删除：
  - `btw_command_entries`
  - `refresh_composer_slash`
  - `accept_btw_composer_command`
  - `select_btw_slash`
  - `apply_picker_input`
  - `picker_open` / `slash_open` 分支
  - `BtwFocus` 分支
  - 空 composer `←/→` 切 tab（Q26）
- [ ] 新增列表态：

```rust
if !lx.btw.editing {
    return apply_list_input(state, pane, command);
}
```

- [ ] `apply_list_input`：
  - `Char/Backspace/Delete/Left/Right/Home/End/Word...` -> 搜索框；
  - `←/→` 且搜索为空 -> `cycle_task_tab`；
  - `Up/Down` -> `move_btw_selection`；
  - `Shift+Enter`（`LxInput::EnterFolder`） -> `enter_btw_edit`；
  - `Ctrl+Enter`（`LxInput::Newline`） -> 吞掉（列表态只有 `Shift+Enter` 一个入口）；
  - `Enter` -> true（吞掉）；
  - `Tab/PrevStep` -> true（吞掉）；
  - `PgUp/PgDn` -> `scroll_btw_detail`；
  - `Ctrl+D` -> `stop_current`；
  - `Esc` -> 搜索非空清空，否则 `close_task_page`；
  - 其他吞掉。
- [ ] 编辑态：
  - 保持浮层优先；
  - `Enter` -> `submit_current`；
  - `Esc` -> 浮层优先，否则 `leave_btw_edit`；
  - `Ctrl+D` -> `stop_current`；
  - `PgUp/PgDn` -> `scroll_btw_detail`；
  - `Tab`/`PrevStep` -> 无浮层时吞掉；
  - 其余交给 `edit_composer`。
- [ ] `LxInput::Newline` / `EnterFolder` 的区分：列表态只认 `EnterFolder`（`Shift+Enter`）进入；`Newline`（`Ctrl+Enter`）吞掉；编辑态由编辑器处理换行/目录进入。
- [ ] 每次搜索文本变化后调用 `sync_selection_after_filter`，若 `active` 改变则触发 `switch_qa`。
- [ ] 同步更新 `src/app/update/chat/mod.rs` 的 `pub use`：
  - 删除 `open_btw_list`、`select_btw_slash`；
  - `select_btw_qa` 改为列表选择语义或替换为 `select_btw_qa_by_id`；
  - 新增 `enter_btw_edit` / `leave_btw_edit` / `move_btw_selection` / `clear_btw_search`。

### 5.2 `src/app/update/chat/btw/mod.rs`

- [ ] 删除 `is_btw_list_command`、`open_btw_list`。
- [ ] `open_btw_tab`：
  - 不再清理 `agent_reply`（字段已删）；
  - 打开后默认 `btw.editing = false`；
  - 保留 `LoadQaList` + `ListSubagents`。
- [ ] `switch_qa`：
  - 设置 `active`；
  - 若切换了 Q：`thread.follow = true; thread.scroll = usize::MAX; thread.rows_dirty = true;`
  - 不再设置 `focus` / `close_picker`；
  - 保留 `LoadThread` 与 `sync_lx_mention_root`。
- [ ] 新增：

```rust
pub fn enter_btw_edit(state, pane) -> bool;
pub fn leave_btw_edit(state, pane) -> bool;
pub fn move_btw_selection(state, pane, delta) -> bool;
pub fn clear_btw_search(state, pane) -> bool;
```

- [ ] `try_submit_btw_command`：
  - 裸命令：`open_btw_tab` + `editing=false` + `active=None` + 清搜索，确保 QA 列表回投后高亮最新 Q；
  - 带 prompt：`open_btw_tab` + `pending_prompt` + `active=None`；`on_qa_list_loaded` 进入编辑态并提交；
  - 不再设置 `picking` / `pending_pick`。

### 5.3 `src/app/update/chat/btw/events.rs`

- [ ] `on_qa_list_loaded` 按设计 §10.1 重写：
  - 删除 picker 相关；
  - active 为空/失效时选最新；
  - `pending_prompt` -> `editing=true` + 注入 + `submit_current`；
  - 无 QA -> 正文回填主输入 + warning；
  - 列表变化后 `sync_selection_after_filter`。
- [ ] `on_thread_loaded` 保持 active 校验；编辑态不被重置。
- [ ] `on_accepted` / `on_failed` 不变。

### 5.4 `src/app/update/pane/task_page.rs`

- [ ] 删除：
  - `apply_agent_reply_input`
  - `edit_agent_reply`
  - `submit_agent_reply`
  - `open_agent_reply`
  - `apply_task_paste` 中 agent_reply 分支
  - `LxInput::LineStart` 的 `stop_all_subagents` 与函数
  - `LxInput::Enter` 的 `open_agent_reply`
- [ ] 新增 Agents 列表/编辑路由（建议放 `src/app/update/pane/task_agents.rs`）：
  - 列表：搜索编辑、`↑/↓` 选择、`Shift+Enter` 进入、`Tab` 筛选、`Esc` 清搜索/关闭、`Ctrl+D` 停止、`PgUp/PgDn` 详情；
  - 编辑：`Enter` 发送、`Shift+Enter` 换行、`Esc` 回列表、`Ctrl+D` 停止、`PgUp/PgDn` 详情、其余编辑。
- [ ] `apply_task_page_input` 总入口按 tab + editing 分派。
- [ ] `cycle_task_tab`：
  - 清空 BTW / Agents 搜索；
  - `btw.editing=false`、`agent_edit=None`；
  - 清理滚动拖拽、选区、`task_list_click`；
  - 保持高亮。
- [ ] `close_task_page`：退出编辑态、清空搜索、删除 `task_list_click`；composer 草稿保留在 `LxBtw` / `AgentsState`。
- [ ] `task_active_len`：按 tab 使用 `visible_indices` / `filtered_qa_indices`。
- [ ] `move_active`：Agents 使用 5 行；Processes 6 行；BTW 走 `LxBtw::move_selection`。
- [ ] `ensure_selected_agent_loaded`：用 `AgentsState.selected_id` 与可见选中项。
- [ ] `select_task_agent` / `select_task_agent_filter`：
  - 同步 `AgentsState.selected_id`；
  - 过滤变化后按稳定 id 恢复高亮；
  - 清理对话选区；
  - 不抢详情（搜索无结果时保留最后详情）。
- [ ] 新增 `enter_agent_edit` / `leave_agent_edit` / `submit_agent_edit`：
  - cancelled -> warning toast，不进入；
  - send -> `Effect::SendSubagentUserInput`；
  - 成功后清空并移除 composer，`agent_edit` 保持。
- [ ] 新增 `task_list_click` 双击判定（或复用现有 click helper）。
- [ ] `agents_loading`：
  - `open_agent_page`、`select_task_tab(Agents)`、`cycle_task_tab` 切入 Agents 且列表为空时置 true，并确保登记一次 `Effect::ListSubagents`；
  - `set_subagent_list` 成功后置 false；
  - 列表非空/已有缓存时直接画列表。
- [ ] `selected_agent_index` 拆成两个口径：
  - `selected_agent_index`：只认当前可见列表，供动作；
  - `detail_agent_index`：可见选中优先，取不到时按 `AgentsState.selected_id` 稳定 ID 回退，供绘制/量行/滚动条/`is_at_bottom`。
- [ ] 关闭面板时清空 `agent_edit`、`agents_loading`，但**保留** `AgentsState.selected_id`（Q10 高亮保留）；切换主会话时随 `AgentsState` 生命周期清理。
- [ ] 打开/切回 Agents 页时用 `selected_id` 恢复 `agents_window.active`。

### 5.5 `src/app/update/chat/slash.rs` 与 `src/app/state/lx/slash.rs`

- [ ] 删除 `SlashCommand::BtwList` 及所有 match 分支。
- [ ] 删除 `LX_BTW_LIST_COMMAND_DESC` 引用。
- [ ] 确认 `SlashCommand::ALL` 仍为 12，`/btw` 保留。
- [ ] 删除 BTW composer `/` 面板相关测试；新增“BTW composer 输入 `/` 不打开命令面板”。

### 5.6 测试

- [ ] `btw_list_shift_enter_enters_edit`
- [ ] `btw_list_enter_is_noop`
- [ ] `btw_list_up_down_never_closes_panel`
- [ ] `btw_list_search_filters_and_restores_selection`
- [ ] `btw_edit_esc_returns_to_list_and_keeps_draft`
- [ ] `btw_edit_tab_does_nothing_without_overlay`
- [ ] `btw_edit_left_right_move_cursor_not_tab`
- [ ] `btw_ctrl_d_stops_and_stays_in_edit`
- [ ] `btw_slash_panel_is_gone`
- [ ] `agents_list_shift_enter_enters_edit`
- [ ] `agents_list_enter_is_noop`
- [ ] `agents_edit_send_stays_in_edit_and_clears`
- [ ] `agents_edit_esc_returns_to_list_and_keeps_draft`
- [ ] `agents_ctrl_a_moves_search_cursor_not_stop_all`
- [ ] `agents_ctrl_d_stops_selected_agent`
- [ ] `agents_cancelled_agent_cannot_enter_edit`
- [ ] `btwlist_command_is_not_recognized_anywhere`

## 6. 鼠标、滚轮与选区

### 6.1 `src/event/mouse/press/lx.rs`

- [ ] 删除 `/btwList` picker 分支。
- [ ] `TaskHit::Search`：
  - 计算搜索框字符索引；
  - `begin_task_search_selection`（或直接设置光标/选区）；
  - 清空 `lx_selection`。
- [ ] `TaskHit::Qa(index)` / `TaskHit::Agent(index)`：
  - 单击 -> 选中并预览；
  - 双击 -> 进入编辑态；
  - 双击判定使用专用 `task_list_click`。
- [ ] `TaskHit::Tab`：
  - 如果当前编辑态，不命中（tab 隐藏）。
  - 切 tab 时退出编辑态、清搜索。
- [ ] `TaskHit::Close` 仅在列表态命中。
- [ ] `TaskHit::AgentFilter` 仅在列表态命中。
- [ ] composer 点击继续走 `input_geom`，编辑态能定位光标。

### 6.2 `src/event/mouse/wheel.rs`

- [ ] 对话区滚轮按 tab 分派（已有）。
- [ ] 列表区滚轮：
  - BTW 列表态 -> `scroll_btw_list`
  - BTW 编辑态 -> `scroll_lx_input`
  - Agents 列表态 -> `scroll_agents_list`
  - Agents 编辑态 -> `scroll_lx_input`
  - Processes -> `scroll_task_list`
- [ ] 搜索行滚轮吞掉。

### 6.3 `src/event/mouse/hit.rs` / `drag.rs`

- [ ] `task_list_scrollbar` 使用搜索行下方 5 行。
- [ ] `set_task_list_scroll` 按 tab 分派到 BTW/Agents/Processes。
- [ ] 拖拽 thumb 同步对话选区？沿用现有 Agents 行为，不新增。

### 6.4 `src/event/input.rs` / `src/event/paste.rs`

- [ ] 粘贴：
  - BTW 列表态 -> `btw.search`（单行化）；
  - BTW 编辑态 -> composer 的 `handle_lx_paste`（保留路径面板/芯片/@）；
  - Agents 列表态 -> `agents_search`（单行化）；
  - Agents 编辑态 -> 直接 `editor.insert_str(&text)`，保留换行但不打开路径粘贴面板/芯片；
  - Processes -> 吞掉。
- [ ] 复制/剪切：
  - 列表态优先搜索框选区；
  - 编辑态优先 composer 选区；
  - 其次对话选区。
- [ ] 删除 `target.lx.is_btw_tab()` 直接走 composer 的假设。
- [ ] 删除 `Ctrl+A` stop-all 分支。

### 6.5 测试

- [ ] `click_btw_row_selects_and_double_click_enters_edit`
- [ ] `click_agent_row_selects_and_double_click_enters_edit`
- [ ] `click_search_row_places_cursor`
- [ ] `wheel_over_btw_list_scrolls_list_not_composer`
- [ ] `wheel_over_agent_list_scrolls_list_not_composer`
- [ ] `paste_into_btw_search_is_single_line`
- [ ] `paste_into_agent_composer_keeps_newlines`
- [ ] `copy_prefers_search_selection_in_list_mode`
- [ ] `task_list_scrollbar_uses_five_rows`

## 7. 删除 `/btwList` 的残留

- [ ] `rg -n -i "btwlist|btw-list" src tests docs/standards src/skills`
- [ ] 删除所有生产代码引用。
- [ ] 更新 `tests/btw_verify.rs`：
  - 删除 `/btwList` 步骤；
  - 改为：打开 BTW 列表 -> 搜索 -> `Shift+Enter` -> 输入 -> 发送 -> `Esc` 回列表。
- [ ] 历史文档 `docs/feature/lx-btw/design.md` §15 / `task.md` §13.2 增加注记：

```text
> 已被 docs/feature/lx-btw-list-and-agent-composer/ 取代：/btwList 删除，
> Q 列表改为 BTW 页签底部常驻列表。
```

- [ ] 不删除 `docs/feature/lx-btw/` 的历史内容。

## 8. 测试与验证

### 8.1 精确测试命令

```bash
cargo fmt
cargo test --lib app::state::lx
cargo test --lib app::update::chat::btw
cargo test --lib app::update::pane
cargo test --lib app::update::chat::viewport
cargo test --lib ui::main_content::lx::task
cargo test --lib ui::main_content::lx::history_hit
cargo test --lib ui::main_content::lx::selection
cargo test --lib event::mouse
cargo test --lib event::input
cargo test --lib event::paste
cargo clippy --all-targets -- -D warnings
```

- [ ] 仅当受影响模块测试通过后，再运行完整 `cargo test`。
- [ ] `tests/btw_verify.rs` 为 ignored E2E；如需运行，按既有验证环境准备数据库。

### 8.2 手工验证（用户执行 `cargo run`）

- [ ] 主输入 `/btw`：打开列表态，最新 Q 高亮，底部 8 行。
- [ ] 搜索 Q 标题：列表过滤，↑/↓ 预览，`Shift+Enter` 进入。
- [ ] 编辑态：tab/搜索/列表消失，`BTW` 标记可见，输入框 3..=5 行。
- [ ] 发送后留在编辑态；`Esc` 回列表；草稿保留。
- [ ] Agents：搜索、`Shift+Enter`、发送、`Esc` 回列表。
- [ ] `/btwList` 不再出现在任何 `/` 面板。
- [ ] `Ctrl+A` 不再停止全部 agent；`Ctrl+D` 仍停止当前目标。
- [ ] 鼠标单击/双击、滚轮、滚动条、卡片折叠、复制均正常。
- [ ] 窄窗格（<40 列）不破边框、不出现半截提示。

## 9. 文件行数与拆分检查

实现过程中每完成一个阶段执行：

```bash
wc -l src/ui/main_content/lx/task.rs \
      src/ui/main_content/lx/task/btw.rs \
      src/ui/main_content/lx/task/agents.rs \
      src/ui/main_content/lx/task/detail.rs \
      src/app/update/pane/task_page.rs \
      src/app/update/pane/task_agents.rs \
      src/app/update/chat/btw/input.rs \
      src/app/update/chat/task_page.rs
```

- [x] 生产文件 ≤1000 行（`task.rs` 997 行）。
- [x] 新增职责文件保持约 500 行（`task/agents.rs` 511 行、`task/btw.rs` 306 行）。
- [x] 测试跟随生产模块拆分；BTW 场景在 `task/tests/btw_list.rs`，Agents 场景在 `task/tests/agent_list.rs`。
- [x] `task/tests.rs` 不再堆积 BTW/Agents 新场景（959 行）。

## 10. 完成标准

- [ ] `design.md` §16 的 10 条验收标准全部满足。
- [x] `rg -n -i "btwlist|btw-list" src` 无生产引用；测试保留 `/btwList` 回归断言（`parse_btw_command` 必须返回 `None`）。
- [x] `rg -n "agent_reply|agent_reply_target|BtwFocus|picker_open|pending_pick|picker_loading" src tests` 无引用。
- [x] `rg -n "stop_all_subagents|stop all" src/ui/text.rs src/app/update` 无引用（会话删除路径仍保留内部批量取消函数）。
- [x] `cargo fmt --check`、`cargo clippy --all-targets -- -D warnings`、完整 `cargo test` 通过（5316 passed / 0 failed / 6 ignored）。
- [ ] 用户完成 `cargo run` 手工确认后，才标记本任务完成。

## 11. 风险与回滚

- 本次不改数据库，回滚只需回退代码与文案。
- 若实现中途发现 `active_editor` 路由无法收敛，优先停止并回退到“显式 task 分支”方案，不允许用隐藏 composer 兜底。
- 若 BTW 详情锚点导致滚动异常，先回退 BTW 到旧 `draw_conversation`，保留其他部分，再单独修复；但最终验收必须以两页一致为准。
- 若文件超限，先拆文件再继续功能，不允许临时堆到超大文件。

## 12. 实施结果

- 已按 §1–§7 完成生产代码与测试改造；未新增数据库迁移。
- `/btwList`、`agent_reply`、`BtwFocus`、picker 状态与 `Ctrl+A` stop-all 已从生产代码清除；测试保留 `/btwList` 不再被识别的回归断言。
- 新增 `src/app/state/lx/composer.rs`、`src/ui/main_content/lx/task/detail.rs`。
- 文件行数：`src/ui/main_content/lx/task.rs` 997 行；`task/agents.rs` 511 行；`task/btw.rs` 306 行；`task/detail.rs` 131 行；`app/update/pane/task_page.rs` 718 行。
- 测试拆分：`task/tests.rs` 959 行，BTW 场景在 `task/tests/btw_list.rs`（306 行），Agents 场景在 `task/tests/agent_list.rs`（912 行）。
- 验证命令已执行：
  - `cargo fmt --all`
  - `cargo clippy --all-targets -- -D warnings`
  - `cargo test`
- 结果：完整测试通过，0 failed；`tests/btw_verify.rs` 的 `/btwList` 步骤已改为检查常驻列表。
- 待用户执行 `cargo run` 确认：`Shift+Enter` 的终端识别、列表/编辑态观感、鼠标双击与滚轮。

## 13. 2026-11-13 修复记录

- 修复 BTW / Agents 列表态搜索行硬件光标坐标写反：`draw_search_row` 返回 `(row, col)`，此前误返回 `(col, row)`，导致光标跑到详情区且无法移除。
- 进入 BTW / Agents 面板或编辑态时清除主输入框与历史区残留选区，避免旧反显/光标残影。
- Agents composer 左上角标记由 `Agent` 改为 `Subagent`。
- BTW / Agents 编辑态在 composer 上方的 chat bar 行显示当前目标 loading：BTW 看线程 `is_loading()`，Agents 看当前 run `is_running()`；列表态不显示。
- loading 沿用主 agent 的扫描窗口，但只用默认前景色（dim / bold 区分），不跑四色渐变。
- 新增回归测试：搜索行光标坐标、主输入/历史选区清理、BTW/Agents 目标 loading 与单色约束。

## 14. 2026-11-13 Review 修复

按 `review.md` 的 P1–P3 修复：

- BTW 搜索后 `active` 与列表高亮分叉：过滤仍有匹配时，`active`/高亮/详情一起落到第一项，并补发 `LoadThread`；`Shift+Enter` 不再静默失败。
- 搜索行硬件光标列改为以文本起点为基准，补非空文本与横向滚动测试。
- Agents composer 接入实际窗格宽高，恢复软换行、3..=5 行高度与光标可见性。
- BTW 切换 Q、BTW/Agents 搜索、`Ctrl+X` 剪切后统一清空旧对话选区。
- 搜索行滚轮守卫只在列表态且搜索行可见时生效，修复 Processes 首行与 composer 行滚轮被吞。
- 编辑态 `list_scrollbar` 返回 None，隐藏列表不再截获 composer 边缘点击/拖拽。
- BTW `Accepted` 回投携带 `qa_id`，按提交所属线程结算，避免切 Q 后 `pending_submit` 卡死。
- 同 tab 打开 agent 链接/`/subagents` 时显式退出旧 composer、清搜索。
- BTW/Agents 目标 loading 文案从 `stream_assistant` / streaming assistant 回退派生，不再恒为 `Working...`。
- Agents 状态筛选无匹配显示 `no agents in this filter`。
- 双击时先按点击行重新选中；裸 `/btw` 重开保留当前 Q；`Subagent` 标记窄窗不再溢出/被胶囊覆盖。
- 删除未调用的 `remove_composer`，修正陈旧注释。
- 新增/更新回归测试；完整 `cargo fmt`、`cargo clippy --all-targets -- -D warnings`、`cargo test` 通过。
