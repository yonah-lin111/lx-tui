# 右侧统一容器与 Prompt / Todo 标签页 — 设计

> 状态：**待用户审核确认后执行**（Grill Me 已逐条确认 13 条边界，本文即收口版本）。
>
> 任务范围：`src/app/state.rs`、`src/app/update/`（新增 `right_panel.rs`）、`src/event/mouse/*`、
> `src/ui/right_panel.rs`（新增）、`src/ui/prompt_frame.rs`、`src/ui/prompt.rs`、
> `src/ui/prompt/header.rs`、`src/ui/text.rs`、`docs/standards/tui-design-requirements.md`。
>
> 不改数据库、不改 `SCHEMA_VERSION`、不改配置项、不改 `src/db/`、不改终端 PTY 行为。

## 1. 背景与目标

### 1.1 现状（代码事实）

- `ViewLayout.prompt`（`src/ui/layout.rs:22-86`）就是右侧那一列的矩形；`prompt_left` 为真时它换到左边，
  但字段名与语义始终是"prompt 这一列"。
- `src/ui/prompt_frame.rs:129` 的 `render_prompt` 一个函数同时干三件事：
  画外层圆角边框、在**顶边框**内嵌 `条目名 · 页名` + 右侧 `‹ n/m ›`、渲染 prompt 正文。
- 顶边框标题绘制在 `src/ui/prompt/header.rs:171 paint_border_title`，命中在
  `src/ui/prompt_frame.rs:101 prompt_page_control_at` → `header.rs:294 border_page_control_at`；
  两者共用 `prompt_border_rect`（= 面板顶边框整行）。
- 工具栏行（功能按钮）在顶边框下一行，再下一行是分割线 `├───┤`（`src/ui/prompt.rs:31 render_divider`），
  分割线当前**不带任何文字**。
- 树视图在顶边框写死 `Prompts`（`text::PROMPT_TREE_BORDER_TITLE`）。
- 底边框左侧是身份标签 `项目 · b:分支 · wt:工作区名`（`prompt_frame.rs:270 render_prompt_bottom`），
  右侧留给折叠按钮。
- 鼠标命中顺序见 `src/event/mouse/press.rs:274-348`：工具栏按钮 → 页重命名输入 → 页控件 →
  顶边框 → 表头空白 → 窗格内容。

所以现在是"一列 = 一个面板"：列的外框、列的控件、列的内容全部由 prompt 独占，第二块面板无处安放。

### 1.2 目标

1. 右侧列升级为**统一容器**：容器独占外框、顶边框标签页与底边框身份标签；容器内部可以挂多个面板。
2. 容器顶边框内嵌两个标签页 `Prompt` / `Todo`，默认 `Prompt`；点击切换。
3. `Prompt` 面板 = 编辑页 + 顶部功能按钮行 + 列表管理页（树视图），作为**独立子面板**渲染，
   自己不再画外框。
4. prompt 的 title 与分页控件从顶边框**下移到功能按钮行下方的分割线**上（用户第 13 问选 A）。
5. `Todo` 面板为占位：功能按钮行为空、分割线为纯 `─`、内容区空白、**零文案**。
6. 底边框（身份标签 + 折叠按钮）属于容器，不随标签页变化。

### 1.3 非目标

- 不实现 Todo 的任何业务功能、不建 Todo 数据表、不持久化选中标签页。
- 不改 prompt 正文/分页/树视图/浮层/自动保存的既有语义。
- 不重命名 `ViewLayout.prompt`、`AppState.prompt_*` 这一批既有字段（见 §5.4 的取舍说明）。
- 不改左侧折叠、标签栏、主区窗格、`prompt_left` 交换逻辑。

## 2. 已确认决策（Grill Me 结论）

| # | 决策项 | 结论 |
| --- | --- | --- |
| 1 | 边框层级 | 容器只保留**一层**边框；prompt 成为容器内子面板，不再自画边框。 |
| 2 | 标签页所有权 | prompt 归属**容器**（外框、tab、底边框），prompt 只负责自己的内容槽。 |
| 3 | tab 状态归属 | `AppState.right_tab`，**全局一份**，不按工作区隔离，不落库、不写配置，启动恒为 `Prompt`。 |
| 4 | 切走时的瞬时状态 | 提交页重命名（非空写回）；清 `prompt_focused`、prompt 选区、`prompt.clear_panel()`（块/斜杠/提及/引用/粘贴面板）、`prompt_tree_hover`；**正文/光标/滚动/撤销栈/页/模式/编辑目标原样保留**。 |
| 5 | 切回 Prompt | 不自动聚焦；用户点内容区或点页控件才聚焦。 |
| 6 | 数据不丢失 | 切到 Todo 期间 `AppState.prompt` 与 `PromptPanel` 的任何字段都不得被改写（见 §5.1 逐字段表）。 |
| 7 | tab 高亮 | 复用顶部标签栏口径：激活 `style::selected_item()`（青底黑字加粗）、悬停未激活 `style::selection()`、其余 `style::muted()`。 |
| 8 | tab 形态 | 刻在顶边框线上：`╭─ Prompt ─ Todo ─────╮`；文案左右各 1 列空格；不加 `▪` 图标、不加 `\|` 分隔符（tab 之间的 `─` 即分隔）。 |
| 9 | 树视图标题 | 删除固定标题 `Prompts`（tab 已写 `Prompt`，不再冗余）；树视图分割线为纯 `─`。 |
| 10 | Todo 体 | 功能按钮行整行空白 + 纯分割线 + 内容区空白；零占位文案。 |
| 11 | Todo 键盘/点击 | `prompt_focused` 为假，键盘退回聚焦窗格；点击 Todo 任何区域（含按钮行、分割线、内容区）一律**吞掉**，不聚焦、不穿透到窗格。 |
| 12 | 交换按钮 | 保留 `prompt_left`；交换后整个容器（含 tab）搬到左侧，tab 跟随。 |
| 13 | title + 分页位置 | 内嵌在**功能按钮行下方的分割线**上（`├─ 条目名 · 页名 ──── ‹ 1/2 › ─┤`），不再占用容器顶边框。 |

## 3. 目标布局

### 3.1 Prompt 编辑视图

```text
╭─ Prompt ─ Todo ──────────────────────╮   ← 容器顶边框：tab（激活青底黑字）
│ [≡] [dir] [○]     [←] [→] [*] ●      │   ← 功能按钮行（prompt 面板自己画）
├─ 条目名 · 页名 ────────── ‹ 1/2 › ───┤   ← 标题行：分割线 + 内嵌 title/分页
│ 正文……                                │
│                                       │
╰─lx-tui · b:main ─────────── [▸]──────╯   ← 容器底边框：身份标签 + 折叠按钮
```

### 3.2 Prompt 列表管理页（树视图）

```text
╭─ Prompt ─ Todo ──────────────────────╮
│ [≡] [tmp] [○]                        │   ← 工具栏（无 undo/redo/[*]/保存点）
├──────────────────────────────────────┤   ← 纯分割线（无 title、无分页）
│ > search prompts…                    │   ← 树视图自带搜索框
├──────────────────────────────────────┤
│ tmp:                                 │
│ ❯ default                            │
╰─lx-tui · b:main ─────────── [▸]──────╯
```

### 3.3 Todo

```text
╭─ Prompt ─ Todo ──────────────────────╮
│                                       │   ← 功能按钮行空白
├──────────────────────────────────────┤   ← 纯分割线
│                                       │   ← 内容区空白
╰─lx-tui · b:main ─────────── [▸]──────╯
```

### 3.4 窄面板降级

```text
╭─ Prompt ─────────────────────────────╮   ← 放不下 Todo 时丢弃尾部的 tab
╭──────────────────────────────────────╮   ← 连 Prompt 都放不下：整排 tab 不画
```

### 3.5 折叠态

不变：仍为 4 列窄条（`ui::controls::render_collapsed_strip`），不画 tab 与内容。

## 4. 架构与数据流

### 4.1 状态层（`src/app/state.rs`、新增 `src/app/update/right_panel.rs`）

容器当前面板枚举就地定义在 `state.rs`（16 行级，不单开文件）：

```rust
/// 右侧统一容器的当前面板。
///
/// 纯视图偏好：不落库、不写配置，启动恒为 `Prompt`（与 `prompt_left` 同级）。
/// 切换工作区不改变它。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum RightTab {
    #[default]
    Prompt,
    Todo,
}
```

`AppState` 新增两个字段（`AppState::demo` 同步初始化）：

```rust
/// 右侧容器当前面板；仅鼠标点击顶边框 tab 切换。
pub right_tab: RightTab,
/// 鼠标悬停的 tab（仅未激活那个会显示悬停反馈）。
pub right_tab_hover: Option<RightTab>,
```

状态迁移集中在一个函数，事件层只调它：

```rust
/// 切换右侧容器面板。切走 Prompt 时提交页重命名并收起"指向可见位置的瞬时指针"；
/// Prompt 的正文、光标、滚动、撤销栈、页与视图模式**一律不动**。
pub fn select_right_tab(state: &mut AppState, tab: RightTab) -> bool {
    if state.right_tab == tab {
        return false;
    }
    finish_page_rename(state);          // 非空写回并产出 Effect::RenamePage；空名保留旧名
    state.right_tab = tab;
    state.right_tab_hover = None;
    if tab != RightTab::Prompt {
        state.prompt_focused = false;   // 键盘退回聚焦窗格
        clear_selection(state);         // prompt 选区
        state.prompt.clear_panel();     // 块/斜杠/提及/引用/粘贴面板
        state.prompt_tree_hover = None; // 列表悬停行
    }
    true
}

/// prompt 面板当前是否可交互（面板可见且容器停在 Prompt）。
pub fn prompt_panel_active(state: &AppState) -> bool {
    state.right_tab == RightTab::Prompt && !state.prompt_collapsed
}
```

**为什么把 tab 命中放在 `press.rs` 而把副作用放在 `update`**：`press.rs` 只做几何命中与分派，
状态迁移必须在 `app/` 内，否则无法在无终端环境下写回归测试。

**为什么在 `focus_prompt` 里补 `right_tab = Prompt`**（`src/app/update/editor.rs:533`）：
"聚焦 prompt"必然意味着"Prompt 面板可见"，否则会出现 `prompt_focused = true` 而画面停在 Todo
的分裂状态。工作区/窗格菜单里的 `Open prompt`、页码菜单激活页、新建临时提示词等 6 条路径
都走 `focus_prompt`，一处兜住即可，不必逐条改。

### 4.2 布局层（几何复用，不新增 `src/layout.rs` 行数）

`src/layout.rs` 当前 **999 行**，已贴 1000 行硬上限。本次**不往里面加任何一行**，只改两个已有函数的
文档注释语义：

| 几何函数 | 新语义 |
| --- | --- |
| `layout::prompt_border_rect(panel)` | 容器**顶边框行**（tab 所在行），`x = panel.x`、`width = panel.width`。 |
| `layout::prompt_divider_rect(panel)` | 工具栏下方**标题行**（`x = inner.x`、`y = inner.y + PROMPT_TOOLBAR_HEIGHT`）。 |
| `prompt_header_rect` / `prompt_toolbar_rect` / `prompt_text_rect` / `prompt_scrollbar_rect` / `prompt_tree_scrollbar_rect` / `prompt_toolbar_button_rect` | **完全不变**（表头仍是 2 行，正文起点不变）。 |

标题行需要"与顶边框同样的右边角留白"，因此给它一个整行版本的几何助手（放渲染层，与
`border_page_controls` 同层，避免 `layout.rs` 超限）：

```rust
/// 标题行几何：工具栏下一行**整行**（含两侧边框列）；表头隐藏时为 None。
///
/// 用整行而不是 `prompt_divider_rect` 的内容区矩形，是为了让页控件"距右边角留 1 列
/// 边框线"的算术与旧顶边框版本完全一致（`next = area.right() - 3`）。
pub fn title_row(panel: Rect) -> Option<Rect> {
    crate::layout::prompt_divider_rect(panel).map(|row| Rect {
        x: panel.x,
        width: panel.width,
        ..row
    })
}
```

tab 几何也放在渲染层 `src/ui/right_panel.rs`（同一先例：顶边框内嵌控件的几何本来就住在
`ui/prompt/header.rs`，绘制与命中共用同一份纯函数）：

```rust
/// tab 文案左右内边距（列）。
const TAB_PAD: u16 = 1;

/// tab 文案（用户可见文案统一收敛在 `ui::text`）。
fn tab_label(tab: RightTab) -> &'static str {
    match tab {
        RightTab::Prompt => text::RIGHT_TAB_PROMPT_LABEL,
        RightTab::Todo => text::RIGHT_TAB_TODO_LABEL,
    }
}

/// 容器顶边框上的 tab 矩形：从 `╭─` 之后 1 列起排，放不下的尾部 tab 丢弃。
pub fn tab_rects(panel: Rect) -> Vec<(RightTab, Rect)> {
    let Some(row) = crate::layout::prompt_border_rect(panel) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    let mut x = row.x.saturating_add(2);
    for tab in [RightTab::Prompt, RightTab::Todo] {
        let width = u16::try_from(tab_label(tab).chars().count())
            .unwrap_or(u16::MAX)
            .saturating_add(TAB_PAD.saturating_mul(2));
        if x.saturating_add(width) > row.right() {
            break; // 尾部丢弃：宁可少一个 tab，也不画半个宽字符/压住圆角
        }
        out.push((tab, Rect::new(x, row.y, width, 1)));
        x = x.saturating_add(width).saturating_add(1); // 1 列为 tab 间的 `─`
    }
    out
}

/// 命中 tab；未命中返回 None。
pub fn tab_at(panel: Rect, column: u16, row: u16) -> Option<RightTab> {
    tab_rects(panel)
        .into_iter()
        .find_map(|(tab, rect)| rect.contains((column, row).into()).then_some(tab))
}

/// 坐标是否落在容器顶边框行（tab 之外的空白同样属于容器控件区，点击须吞掉）。
pub fn tab_row_at(panel: Rect, column: u16, row: u16) -> bool {
    crate::layout::prompt_border_rect(panel)
        .is_some_and(|row_rect| row_rect.contains((column, row).into()))
}
```

### 4.3 渲染层

#### 4.3.1 新文件 `src/ui/right_panel.rs`（容器）

职责：外框、tab、底边框身份标签、折叠窄条、面板分发。**只读 `&AppState`**。

```rust
/// 右侧统一容器：外框 + 顶边框 tab + 底边框身份标签，并按 `right_tab` 分发面板体。
/// 返回 prompt 面板聚焦时的硬件光标位置。
pub(crate) fn render(
    frame: &mut Frame<'_>,
    view: &layout::ViewLayout,
    state: &AppState,
) -> Option<(u16, u16)> {
    let area = view.prompt;
    if area.width == 0 || area.height == 0 {
        return None;
    }
    if state.prompt_collapsed {
        render_collapsed_strip(frame, area, false);
        return None;
    }
    let focused = state.prompt_focused && state.right_tab == RightTab::Prompt;
    let border = Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(style::border(focused));
    let inner = border.inner(area);
    frame.render_widget(border, area);
    if inner.width == 0 || inner.height == 0 {
        return None;
    }
    paint_tabs(frame.buffer_mut(), area, state.right_tab, state.right_tab_hover);
    render_bottom(frame, area, state);
    match state.right_tab {
        // Prompt 体：功能按钮行 + 标题行 + 正文/树 + 滚动条 + 硬件光标
        RightTab::Prompt => prompt_frame::render_body(frame, area, state),
        // Todo：功能按钮行留空，只画一条纯分割线；内容区由 ratatui 每帧清空后天然空白
        RightTab::Todo => {
            render_empty_body(area, frame.buffer_mut(), focused);
            None
        }
    }
}

/// 占位面板体：功能按钮行不画任何东西，只画工具栏下一行的分割线（带 `├`/`┤` 衔接）。
fn render_empty_body(panel: Rect, buf: &mut Buffer, focused: bool) {
    let Some(header) = crate::layout::prompt_header_rect(panel) else {
        return;
    };
    prompt::render_divider(panel, header, buf, focused);
}

/// 绘制顶边框 tab：激活 `selected_item`、悬停未激活 `selection`、其余 `muted`。
///
/// tab 之间那 1 列 `─` 由 Block 已经画好，这里不碰，避免把 tab 连成一片。
pub fn paint_tabs(buf: &mut Buffer, panel: Rect, active: RightTab, hover: Option<RightTab>) {
    for (tab, rect) in tab_rects(panel) {
        let style = if tab == active {
            style::selected_item()
        } else if hover == Some(tab) {
            style::selection()
        } else {
            style::muted()
        };
        let label = tab_label(tab);
        let text = format!(" {label} ");
        buf.set_stringn(rect.x, rect.y, &text, usize::from(rect.width), style);
    }
}
```

`render_bottom` 与 `prompt_identity` 从 `src/ui/prompt_frame.rs:261-301` **原样搬过来**（函数体不改，
只改 `fn` → `fn`，模块头注释改为"容器底边框"）。它在容器层，因此 tab 切换不影响它。

#### 4.3.2 `src/ui/prompt_frame.rs` 收敛为面板体

```rust
pub(crate) fn render_body(
    frame: &mut Frame<'_>,
    area: Rect,
    state: &AppState,
) -> Option<(u16, u16)> {
    let focused = state.prompt_focused;
    let mode = state.prompt_owner().prompt_panel.mode;
    let title = title_source(state);             // 树视图 → None（纯分割线）
    let (title_text, draft, page) = /* 同现状，树视图三者为 (None, false, None) */;

    // 顺序不可交换：先画分割线，再把标题/页控件覆盖上去（否则标题被 `─` 冲掉）。
    prompt::render_header(area, frame.buffer_mut(), &state.prompt, focused, mode, draft,
                          state.prompt_pinned.is_some());
    if let Some(row) = prompt::title_row(area) {
        prompt::paint_title_row(frame.buffer_mut(), row, &page_data);
    }
    let text_area = crate::layout::prompt_text_rect(area);
    /* 树视图 / 编辑视图正文、滚动条、光标：与现状完全一致 */
}
```

`prompt_scrollbar(view, state)` 增加一行闸门，使 Todo 下滚动条既不可见也不可命中：

```rust
pub fn prompt_scrollbar(view: &layout::ViewLayout, state: &AppState) -> Option<...> {
    if state.right_tab != RightTab::Prompt {
        return None;
    }
    prompt_scrollbar_for(view.prompt, state)
}
```

#### 4.3.3 `src/ui/prompt/header.rs` 改名（语义纠偏）

标题已经不在边框上，函数名/类型名必须跟着改，避免留下会撒谎的命名（纯重命名，不增行）：

| 旧 | 新 |
| --- | --- |
| `BorderTitleData` | `TitleRowData` |
| `paint_border_title` | `paint_title_row` |
| `border_page_controls` | `page_controls` |
| `border_title_rect` | `title_text_rect` |
| `border_page_control_at` | `page_control_at` |
| `border_rename_cursor` | `rename_cursor` |

`border_page_controls` 内部的算术**一字不改**（`next = area.right() - 3` 等），因为 `title_row()`
给出的整行矩形与旧顶边框矩形同列同宽，页控件"与右边角留 1 列"的观感不变。

树视图不再有固定标题：`prompt_frame` 的 `TitleRow` 枚举里 `Tree` 变体只表示"无标题"，
`text::PROMPT_TREE_BORDER_TITLE` 删除。

#### 4.3.4 `src/ui/prompt.rs`

- `render_divider` 从私有改为 `pub(crate)`（Todo 占位体复用同一条分割线，不复制第二套画法）。
- `render_toolbar` 不变。

#### 4.3.5 `src/ui/text.rs`

```rust
/// 右侧容器顶边框标签页文案。
pub const RIGHT_TAB_PROMPT_LABEL: &str = "Prompt";
pub const RIGHT_TAB_TODO_LABEL: &str = "Todo";
```

删除 `PROMPT_TREE_BORDER_TITLE`（唯一使用点随树视图标题一起移除）。

#### 4.3.6 `src/ui/mod.rs`

```rust
pub mod right_panel;
...
let prompt_cursor = right_panel::render(frame, &view, state).or(pane_cursor);
```

### 4.4 事件与交互

#### 4.4.1 左键分派顺序（`src/event/mouse/press.rs`）

在"折叠按钮/分割线/滚动条/侧栏/工作区/交换按钮/拖拽调整宽度"之后、prompt 内部控件之前插入 tab 分支，
并给所有 prompt 内部命中加 `right_tab == Prompt` 闸门：

```rust
} else if !state.prompt_collapsed
    && let Some(tab) = ui::right_panel::tab_at(view.prompt, mouse.column, mouse.row)
{
    update::select_right_tab(state, tab);   // 内部提交页重命名、清瞬时指针
    *dirty = true;
} else if !state.prompt_collapsed
    && ui::right_panel::tab_row_at(view.prompt, mouse.column, mouse.row)
{
    // 顶边框 tab 之外的空白属于容器控件区：吞掉，不聚焦编辑器。
} else if !update::prompt_panel_active(state) {
    // Todo 面板：按钮行/分割线/内容区一律吞掉，不聚焦、不穿透到窗格。
    // （只有容器顶边框的 tab 在上一分支已处理，其余位置全部落在这里。）
} else if let Some(button) = layout::prompt_toolbar_button_at(...) {
    /* 同现状 */
} else if update::page_rename_active(state) && state.overlay.is_none() {
    /* 同现状 */
} else if let Some(control) = ui::prompt_page_control_at(view.prompt, state, col, row) {
    /* 同现状 */
} else if ui::prompt_title_at(view.prompt, col, row) {
    // 标题行是控件区：点条目名/空白只把焦点交给编辑器，不移动正文光标（旧顶边框语义保留）。
    update::focus_prompt(state);
    *dirty = true;
} else if prompt_header_at(view.prompt, col, row) {
    // 工具栏行空白：吞掉，不聚焦。
} else if let Some(pane) = pane_at(...) {
    /* 内容区：条件从 `pane == state.prompt.id()` 保持不变即可——上面已保证此刻停在 Prompt */
}
```

注意执行顺序：tab 分支必须在 `pane_at` 之前（顶边框行不是内容区，但显式优先级更可读）；
`!prompt_panel_active` 的整块吞掉分支必须在所有 prompt 命中之前。

#### 4.4.2 悬停（`src/event/mouse/hover.rs`）

```rust
// 容器 tab 悬停：仅在未折叠且指针不在 toast 上时生效；离开即清空。
let right_tab_hover = if hovering_toast || state.prompt_collapsed {
    None
} else {
    ui::right_panel::tab_at(view.prompt, mouse.column, mouse.row)
};
if update::set_right_tab_hover(state, right_tab_hover) {
    *dirty = true;
}
```

树视图行悬停的闸门从 `state.prompt_collapsed` 改为 `!update::prompt_panel_active(state)`。

#### 4.4.3 其余需要闸门的命中点

| 位置 | 改动 |
| --- | --- |
| `event/mouse/hit.rs::pane_content_inner` | prompt 且 `!prompt_panel_active` 时返回零矩形（内容区不存在）。 |
| `event/mouse/wheel.rs` prompt 分支 | 加 `prompt_panel_active` 闸门，Todo 下滚轮不滚动隐藏编辑器。 |
| `event/mouse/drag.rs` 光标跟随 | 加同一闸门（Todo 下拖拽不写隐藏光标）。 |
| `event/overlay.rs:456` 右键树菜单 | 加 `prompt_panel_active` 闸门，Todo 下右键不打开提示词树菜单。 |
| `event/mouse/hit.rs::prompt_border_at` | 删除；改用 `ui::right_panel::tab_row_at`。 |
| `event/session.rs` rects | **不改**：仍把 `(prompt.id(), view.prompt)` 放进 rects，保证容器与主区的分割线可拖拽、toast 锚点可用。 |

`pane_content_inner` 返回零矩形是安全的兜底：即使别处漏了闸门，任何"按内容区坐标换算"的路径
都会得到 `width == 0 / height == 0`，而不会变成一次看不见的光标定位。

#### 4.4.4 键盘与粘贴

不改代码：`prompt_focused` 在切到 Todo 时置假，`input::route` 与 `handle_paste_event`
本来就以 `prompt_focused` 为准，键盘与粘贴自然退回聚焦窗格（与折叠态同一条路径）。

### 4.5 不变量

1. **单层边框**：容器是唯一画 `Block` 的地方；`prompt_frame` 不再调用 `Block`。
2. **绘制顺序**：Block → tab → 底边框身份标签 → 面板体（分割线 → 标题）→ 内容 → 滚动条。
   标题必须在分割线之后绘制。
3. **几何同源**：tab 的绘制与命中共用 `tab_rects`；标题/页控件的绘制与命中共用
   `title_row` + `page_controls`，禁止在事件层重算列号。
4. **聚焦蕴含**：`prompt_focused == true` ⇒ `right_tab == Prompt`（由 `focus_prompt` 与
   `select_right_tab` 双向保证）。
5. **数据不丢**：切换 tab 不改写任何 prompt 内容字段（§5.1）。

## 5. 边界与降级

### 5.1 「数据不丢失」逐字段表

切到 Todo 再切回 Prompt 后，下列字段必须与切走前**完全一致**：

| 字段 | 是否被 tab 切换改写 |
| --- | --- |
| `AppState.prompt.text` / `cursor` / `scroll` | 否 |
| `Prompt.undo` / `redo` / `last_edit` | 否 |
| `Prompt.is_saved` / `revision` / `autosave` | 否 |
| `Prompt.expanded_supplements` / `fold_blocked` | 否 |
| `PromptPanel.mode` / `selected` / `editing` / `scroll` / `temps` / `tree_return` | 否 |
| `PromptPanel.tree`（文件夹/条目/页/当前页） | 否 |
| `PromptPanel.search`（过滤词） | 否 |
| `PromptPanel.editor_stash` | 否 |
| `AppState.prompt_root` / `prompt_pinned` | 否 |
| `Prompt.panel` / `slash` / `mention` / `refer` / `paste`（瞬时浮层） | **是**：`clear_panel()` 收起 |
| `PromptPanel.page_rename`（瞬时输入） | **是**：提交（非空写回，空名保留旧名） |
| `AppState.prompt_focused` | **是**：置假 |
| `AppState.selection`（瞬时选区） | **是**：清空 |
| `AppState.prompt_tree_hover` | **是**：清空 |

"瞬时浮层/选区/焦点"是指向某个**可见位置**的指针，面板隐藏后它们指向空白，必须收回；
正文与页数据是内容，一律不动。这条区分就是本设计的核心口径。

### 5.2 降级矩阵

| 场景 | 表现 |
| --- | --- |
| 面板宽度放不下两个 tab | 从尾部丢弃（先丢 `Todo`，只留 `Prompt`）。 |
| 面板宽度连 `Prompt` 都放不下（`width < 10`） | 整排 tab 不画，只剩顶边框 `╭────╮`；此时无法切换面板，展开或拖宽后恢复。 |
| 内容区高度 ≤ 2（`prompt_header_rect` 为 None） | 不画工具栏/标题行/Todo 分割线，正文退回整块内容区（与现状一致）。 |
| 折叠态 | 4 列窄条，不画 tab（与现状一致）。 |
| `prompt_left` 交换后 | 容器整体搬到左侧，tab 与底边框跟随，行为不变。 |
| 树视图 | 标题行为纯 `─`，无页名/页控件（与现状视觉一致，只是固定标题 `Prompts` 消失）。 |
| 编辑视图未绑定条目 | 标题为空、页控件仍按页信息绘制（与现状一致）。 |
| Todo 下的滚轮/拖拽/右键 | 一律吞掉，不写隐藏编辑器、不打开提示词树菜单。 |
| 浮层打开时点 tab | 浮层是模态，点击先被浮层消费（关闭浮层），不会切换 tab；需再点一次。这是既有模态语义，不特殊处理。 |

### 5.3 视觉验收要点（用户 `cargo run` 人工确认）

1. 顶边框是 `╭─ Prompt ─ Todo ─────╮`，激活 tab 青底黑字。
2. 第二行是功能按钮，第三行是带 title 与 `‹ n/m ›` 的分割线（`├─…─┤` 衔接两侧边框）。
3. 点 `Todo`：按钮行空白、纯分割线、内容空白，底边框身份标签与折叠按钮不变。
4. 切回 `Prompt`：正文、光标位置、当前页、滚动位置原样还在。
5. 点 `Todo` 内容区：不出现光标，终端窗格不被操作。

## 6. 测试计划

### 6.1 布局层（`src/layout/tests.rs`）

- 保持 `prompt_border_rect_covers_top_border_row`（语义改为 tab 行，断言不变）。
- 保持 `prompt_header_shifts_body_and_scrollbar` / `prompt_header_hidden_when_body_too_short`。

### 6.2 渲染层（`src/ui/right_panel/tests.rs` 新增）

- `tabs_render_on_top_border_with_accent_active`：顶边框整行含 `Prompt`、`Todo`；激活 tab 单元格 bg = Cyan、fg = Black、BOLD。
- `tabs_drop_trailing_when_narrow`：宽 12 只留 `Prompt`；宽 8 一个都不画、圆角完整。
- `tab_hit_areas`：`Prompt`/`Todo` 文字区命中正确；tab 之间的 `─`、边框空白、非顶边框行返回 None。
- `todo_body_is_blank_with_plain_divider`：Todo 下工具栏行无按钮文案、分割线为纯 `─`。
- `bottom_identity_survives_tab_switch`：两种 tab 下底边框身份标签一致。

### 6.3 渲染层（`src/ui/prompt/tests.rs` 改造）

- 标题测试改为"先画分割线再覆盖标题"，断言标题落在**分割线行**（`inner.y + 1`），且该行仍以 `├`/`┤` 起止。
- `border_rename_keeps_top_border_line_right_of_input` → 改为 `rename_keeps_divider_line_right_of_input`。
- 删除 `border_tree_view_title_has_no_page_controls` 中对 `PROMPT_TREE_BORDER_TITLE` 的断言，改为"树视图标题行为纯 `─`"。
- `prompt_renders_toolbar_and_divider_rows`（`src/ui/tests.rs`）改为断言：顶边框含两个 tab、分割线行含条目名与 `1/1`。

### 6.4 状态层（`src/app/update/tests.rs`）

- `select_right_tab_commits_rename_and_clears_transient_state`：重命名输入中切 tab → 名称写回、`page_rename` 为空、`prompt_focused == false`、选区为 None、`panel/slash/mention/refer/paste` 全空。
- `select_right_tab_preserves_prompt_content`：预置正文/光标/滚动/撤销栈/第二页 → 切 Todo 再切回，逐字段相等；`undo()` 仍可回到上一次编辑。
- `select_right_tab_same_tab_is_noop`：重复选当前 tab 返回 false，不产生 `Effect`。
- `focus_prompt_switches_back_to_prompt_tab`：Todo 下调用 `focus_prompt` → `right_tab == Prompt`。

### 6.5 事件层（`src/event/tests.rs`）

- `mouse_click_right_tab_switches_panel`：点 `Todo` → `right_tab == Todo`、`prompt_focused == false`。
- `mouse_click_todo_body_swallowed_without_focus_or_cursor`：点 Todo 内容区 → 光标不动、pane 焦点不变。
- `mouse_click_prompt_toolbar_ignored_on_todo_tab`：Todo 下点原工具栏按钮坐标 → 不切模式。
- `mouse_click_title_row_focuses_without_moving_cursor`（迁移自旧顶边框用例）。
- `mouse_click_prompt_page_arrows_switch_and_counter_opens_menu` / `..._name_starts_rename_and_keys_commit`：命中 row 改为标题行（`inner.y + 1`）。
- `mouse_wheel_over_todo_does_not_scroll_prompt`。
- 悬停：`mouse_motion_tracks_right_tab_hover`。

## 7. 文档同步

`docs/standards/tui-design-requirements.md` 第 25 行（prompt 右栏一整段）必须同步，逐点改：

1. 顶边框由"内嵌条目名/页名/页控件"改为"内嵌 `Prompt` / `Todo` 两个标签页（激活青底黑字、悬停中灰底白字、
   其余灰字；tab 之间为 `─`；宽度不足从尾部丢弃）"。
2. 新增"功能按钮行下一行为标题行：左侧 `条目名 · 页名`、右端 `‹ n/m ›`，两端以 `├`/`┤` 衔接边框"。
3. 删除"树视图顶边框为固定标题 `Prompts`"，改为"树视图标题行为纯 `─`"。
4. 底边框身份标签的描述加一句"属于容器，不随标签页变化"。
5. 增补 `Todo` 面板：占位、按钮行空白、纯分割线、零文案、点击与滚轮一律吞掉、不持久化。

## 8. 风险与取舍

1. **`ViewLayout.prompt` / `AppState.prompt_*` 命名债**：字段含义已变成"容器"，但重命名要动 40+ 处
   调用点且会与 `state.prompt`（编辑器）语义打架。本次不重命名，在 `ui/layout.rs` 与
   `app/state.rs` 的注释里写明"prompt 列 = 右侧统一容器"。这是**有意保留的债**，不是遗漏。
2. **命中闸门分散在 5 处**（press/wheel/drag/hover/overlay）：靠 `prompt_panel_active` 统一语义 +
   `pane_content_inner` 零矩形兜底 + 事件层测试覆盖，避免"点 Todo 却操作了隐藏编辑器"。
3. **`layout.rs` 已 999 行**：本次刻意不新增该文件任何行；若后续必须加几何，先按职责拆分该文件，
   不要为了几十行去顶破 1000 行上限。
4. **Todo 是占位**：将来接入真实面板时，只需在 `render` 的 `match` 里加分支、在 `RightTab` 加枚举值，
   容器/tab/底边框/命中闸门都不用重切。
