# 窄终端紧凑布局（Compact Layout）— 详细设计方案

> 状态：**边界已确认（Grill Me Q1–Q12），阶段 1–5 已实现；待用户 `cargo run` 手动验收**
>
> 参考实现：`/Users/yonah/projects/agent/herdr-master`
> - `src/ui.rs::compute_mobile_view`：`width <= ui.mobile_width_threshold`（默认 64）时进入 mobile 单列。
> - `src/ui/mobile.rs`：2 行 header（工作区状态 + tab 状态 / agent 摘要）+ 右侧 `switch`；全屏 switcher 按 `agents → spaces → tabs → menu` 排序。
> - `src/ui/mobile.rs::render_mobile_toast_banner`：mobile 模式全局 Toast 落在屏幕底部单行。
>
> 本方案对应 lx-tui 的“窄终端紧凑布局”，不复刻 herdr 的完整 mobile 功能集；边界见第 2 节。

---

## 1. 背景与现状问题

### 1.1 当前窄屏行为

`src/ui/layout.rs::compute` 目前只有“桌面三栏”一种形态：

- 左侧栏：`area.width >= config.narrow_width`（默认 80）时显示，否则宽度归 0。
- 标签栏：横跨主区 + 右栏，固定 1 行。
- 主区：`left_panel` 之外的剩余宽度减去右栏。
- 右栏：Prompt / Todo 统一容器，宽度由 `prompt_width` 决定，通常约为主区可用宽度的 2/5。

因此 60 列终端下实际是：

```text
[左侧栏 0][聊天区 30][Prompt/Todo 30]
```

80 列终端下则是：

```text
[左侧栏 24][聊天区 26][Prompt/Todo 30]
```

左侧栏虽然隐藏了，但右侧 Prompt/Todo 仍固定占约 30 列，聊天区没有真正拿到窄屏的横向空间；80 列时甚至因为侧栏重新出现，聊天区只剩 26 列。这不是“窄屏适配”，只是隐藏了一个栏。

### 1.2 herdr 的关键参考点

herdr 的 mobile 形态不是“只隐藏侧栏”，而是直接切成单列：

- `width <= mobile_width_threshold` 时不再计算侧栏、标签栏、右栏。
- 顶部 2 行状态条：
  - 第 1 行：工作区状态点 + 名称 + 右侧当前 tab；
  - 第 2 行：全局 agent 摘要（blocked / working / idle 等）；
  - 右侧固定 `switch` 按钮，贯穿状态条高度。
- 主区由当前 tab 的窗格占满。
- `switch` 打开全屏 switcher，按 `agents → spaces → tabs → menu` 排列。
- mobile 下全局 Toast 使用底部单行横幅。

### 1.3 本方案的核心判断

窄屏的主要矛盾是横向空间，不是“有没有侧栏”。如果只隐藏左侧栏、保留右栏，60 列聊天区仍然只有 30 列，等于没做适配。因此必须切成单列，并给被隐藏的导航/面板一个明确的进入方式。

---

## 2. 已确认边界（Grill Me 决策汇总）

| 编号 | 决策维度 | 最终确认口径 |
| --- | --- | --- |
| Q1 | 紧凑布局形态 | **单列全屏**：隐藏左侧栏、标签栏、右侧 Prompt/Todo 容器，只保留顶部状态条 + 主区窗格。 |
| Q2 | 触发阈值与配置 | 复用 `config.narrow_width`（默认 80），**不新增配置项**；判定为 `width <= narrow_width`。该配置语义升级为“低于等于此宽度进入紧凑布局”，设置页文案改为 `Narrow layout width`。 |
| Q3 | `switch` 覆盖范围 | 完整切换器：`Agents → Spaces → Tabs → Panels`，并保留 `+ new workspace`、`+ new tab`；选中 `Prompt`/`Todo` 进入全屏面板，顶部 `[back]` 返回聊天。 |
| Q4 | 多窗格平铺 | 保持现有 BSP 平铺，不自动合并窗格、不隐藏非焦点窗格；紧凑模式只是把腾出的宽度全部给主区。 |
| Q5 | 顶部状态条高度 | 自适应两档：终端高度 `>= 16` 时 2 行（状态/tab + agent 摘要），`< 16` 时 1 行；`switch` 始终占满状态条高度。 |
| Q6 | `Agents` 数据范围 | **全局 agents**（跨所有工作区/标签/窗格），每行带工作区、tab、pane 定位；选中后一次切到对应工作区 + 标签 + 窗格。 |
| Q7 | 键盘入口 | v1 **只做鼠标/触摸点击**打开 `switch`，不新增全局快捷键；`Ctrl+Q` 仍是唯一全局键。switcher 打开后支持 `↑/↓/Enter/Esc/PageUp/PageDown`，因为此时按键由浮层接管，不会写入 PTY。 |
| Q8 | 全屏面板返回 | 保留 1 行紧凑状态条，左侧 `[back]` + 面板名；进入时焦点切到 `Prompt`/`Todo`，点 `[back]` 回到聊天并把焦点还给主区焦点窗格；**不劫持 `Esc`**。 |
| Q9 | 拉宽退出紧凑 | 保留当前焦点/面板：显示 Chat 还是 Prompt/Todo 由 `FocusedContainer` + `RightTab` 推导；拉宽后恢复三栏但保留 Prompt 焦点，缩窄后回到同一全屏面板。 |
| Q10 | 桌面专属入口 | v1 不包含工作区历史列表、Web 设置入口、侧栏折叠/交换控件；这些仍需拉宽回桌面操作；退出统一 `Ctrl+Q`。 |
| Q11 | 最小宽度 | 保持 `min_width = 40`；紧凑布局覆盖 `40 <= width <= narrow_width`，低于 40 继续显示最小尺寸提示。 |
| Q12 | 全局 Toast | 紧凑模式下全局 Toast 改为底部单行横幅；`pane.lx.toast` 仍留在 lx 窗格内不动；桌面模式维持现状。 |

---

## 3. 目标与非目标

### 3.1 目标

1. `40 <= width <= narrow_width` 时自动进入单列布局，主区拿到全部宽度。
2. 顶部状态条能看清：当前工作区、当前 tab、全局 agent 摘要、进入切换器的入口。
3. 能通过 `switch` 完成：
   - 跳到任意工作区的任意 agent；
   - 切换工作区；
   - 切换当前工作区 tab；
   - 新建工作区、新建 tab；
   - 进入全屏 Prompt / Todo 并返回。
4. 多窗格平铺、PTY 尺寸、lx 历史/输入、终端快捷键等桌面行为不被破坏。
5. 退出紧凑布局后，原桌面布局与折叠状态可恢复。
6. 不引入新的配置项、数据库迁移或新的全局快捷键。

### 3.2 非目标

- 不实现 herdr 的 `Mode::Navigate` / prefix 键盘体系。
- 不把工作区历史列表、Web 设置、侧栏折叠/交换控件搬进紧凑模式。
- 不改变 BSP 窗格模型，不自动合并窗格。
- 不降低 `min_width`，不做 32 列以下的组件重排。
- 不改变桌面三栏布局的现有视觉与交互。
- 不引入新的主题色；header/switcher 复用现有 `style` token。

---

## 4. 总体架构与数据流

```text
终端 resize / 每轮事件循环
        │
        ▼
event::session::current_geometry(&mut state)
        │  1. 读取 terminal size
        │  2. update::sync_compact_layout(state, width, height, config)
        │     - 首次进入紧凑：快照 prompt_collapsed / panes_collapsed，强制展开
        │     - 退出紧凑：恢复快照
        │     - 同步 switcher viewport
        │  3. ui::layout::compute_with_panel(..., state.compact_panel())
        │  4. 生成 pane rects / prompt rect
        ▼
ui::render_cached(frame, state, config)
        │  - compact ? 渲染 header + 主区/全屏面板
        │  - desktop ? 走现有三栏
        │  - overlay::render → CompactSwitcher 全屏覆盖
        │  - toast::render → 紧凑模式底部横幅
        ▼
鼠标/键盘事件
        │  - 紧凑 header 命中：switch / back
        │  - switcher 浮层命中/滚轮/键盘
        │  - 其余按键：沿用 FocusedContainer 路由
        ▼
app::update::compact_layout / app::update::dialog::apply_overlay_key
```

核心原则：

- **宽度是唯一布局开关**：`view.compact == (area.width <= config.narrow_width)`。
- **焦点是面板屏幕的唯一真源**：`FocusedContainer::{Panes, Prompt, Todo}` 决定紧凑模式显示 Chat 还是哪个全屏面板；不新增 `compact_screen` 状态。
- **折叠状态可恢复**：紧凑期间只临时重置 `prompt_collapsed` / `panes_collapsed`，退出时按快照恢复；`sidebar_collapsed`、`prompt_left`、`prompt_width` 不改。
- **渲染只读**：header/switcher/ Toast 都只读 `AppState` 与 `ViewLayout`；状态变更全部在 `app/update`。

---

## 5. 布局模型

### 5.1 `ViewLayout` 扩展

`src/ui/layout.rs`：

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ViewLayout {
    pub left_panel: Rect,
    pub tab_bar: Rect,
    pub main_content: Rect,
    /// 右侧统一容器；紧凑 Chat 时为 0，紧凑全屏面板时为 body 整块。
    pub right_panel: Rect,
    pub right_panel_left: bool,
    /// 紧凑模式顶部状态条；桌面布局为 0。
    pub compact_header: Rect,
    /// 当前是否为紧凑布局。
    pub compact: bool,
}
```

说明：

- `compact` 是布局派生结果，不写回 `AppState`。
- `compact_header` 只在 `compact == true` 时有值；桌面布局保持 `Rect::new(area.x, area.y, 0, 0)`。
- 不在 `ViewLayout` 里存 `compact_screen`；全屏面板由 `right_panel` 是否非零 + `state.compact_panel()` 推导。

### 5.2 布局入口：保留 `compute`，新增 `compute_with_panel`

为了减少现有测试与调用点的改动，保留原 8 参数 `compute`，新增 9 参数内部入口：

```rust
/// 桌面/紧凑布局通用入口；紧凑模式固定显示 Chat（无全屏面板）。
pub fn compute(
    area: Rect,
    config: &Config,
    sidebar_collapsed: bool,
    sidebar_width: u16,
    prompt_collapsed: bool,
    prompt_width: u16,
    prompt_left: bool,
    panes_collapsed: bool,
) -> ViewLayout {
    compute_with_panel(
        area,
        config,
        sidebar_collapsed,
        sidebar_width,
        prompt_collapsed,
        prompt_width,
        prompt_left,
        panes_collapsed,
        None,
    )
}

/// 生产入口：紧凑模式下 `compact_panel` 决定显示 Chat 还是全屏 Prompt/Todo。
pub fn compute_with_panel(
    area: Rect,
    config: &Config,
    sidebar_collapsed: bool,
    sidebar_width: u16,
    prompt_collapsed: bool,
    prompt_width: u16,
    prompt_left: bool,
    panes_collapsed: bool,
    compact_panel: Option<RightTab>,
) -> ViewLayout
```

生产代码调用 `compute_with_panel` 的位置只有：

- `src/ui/mod.rs::render_cached`
- `src/event/session.rs::current_geometry`

测试与纯几何辅助函数继续调用 `compute`；需要断言全屏面板的测试调用 `compute_with_panel`。

### 5.3 紧凑布局几何

常量：

```rust
/// 紧凑 header 高度 >= 该值时显示第二行 agent 摘要。
const COMPACT_HEADER_TALL_MIN_HEIGHT: u16 = 16;
```

`compute_with_panel` 顶部先判断：

```rust
let compact = area.width > 0 && area.width <= config.narrow_width;
if compact {
    let header_height = if compact_panel.is_some() {
        1
    } else if area.height >= COMPACT_HEADER_TALL_MIN_HEIGHT {
        2
    } else {
        1
    }
    .min(area.height);

    let header = Rect::new(area.x, area.y, area.width, header_height);
    let body = Rect::new(
        area.x,
        area.y + header_height,
        area.width,
        area.height.saturating_sub(header_height),
    );
    let (main_content, right_panel) = match compact_panel {
        Some(_) => (
            Rect::new(body.x, body.y, 0, body.height),
            body,
        ),
        None => (
            body,
            Rect::new(body.x, body.y, 0, body.height),
        ),
    };

    return ViewLayout {
        left_panel: Rect::new(area.x, area.y, 0, area.height),
        tab_bar: Rect::new(area.x, area.y, 0, 0),
        main_content,
        right_panel,
        right_panel_left: false,
        compact_header: header,
        compact: true,
    };
}
```

之后是现有桌面逻辑，返回值补 `compact_header: Rect::new(area.x, area.y, 0, 0)`、`compact: false`。

### 5.4 高度与宽度档位

| 终端尺寸 | header | 主区 |
| --- | --- | --- |
| `width <= narrow_width`，`height >= 16`，Chat | 2 行 | `height - 2` |
| `width <= narrow_width`，`height < 16`，Chat | 1 行 | `height - 1` |
| `width <= narrow_width`，全屏面板 | 1 行 | 右栏占 `height - 1` |
| `width > narrow_width` | 0 | 现有三栏 |
| `width < min_width` | 不进入渲染 | 现有最小尺寸提示 |

示例（80x24）：

```text
 ● project-a                         tab 1 · 1/3   [switch]
 ● 1 blocked · 2 working
 ╭─ lx-Native ─────────────────────────────────────────────╮
 │ ...                                                      │
```

示例（60x10）：

```text
 ● project-a                         tab 1   [switch]
 ╭─ lx-Native ───────────────────────────────╮
 │ ...                                       │
```

示例（40x12，全屏 Prompt）：

```text
 [back] Prompt
 ╭─ Prompt ────────────────────────────────╮
 │ ...                                     │
```

### 5.5 桌面布局不变

`compact == false` 时：

- `left_panel` / `tab_bar` / `main_content` / `right_panel` 的现有计算完全保留。
- `compact_header` 恒为 0，`compact = false`。
- 现有 `narrow_width` 在桌面分支中不再需要 `resolved_sidebar_width` 的“隐藏侧栏”判断，因为 `width <= narrow_width` 已被紧凑分支截走；`width > narrow_width` 时侧栏恒显示。为避免死代码，保留 `resolved_sidebar_width` 供 `default_side_width` 等启动计算使用，但桌面分支中它只会走 `wide_enough == true` 的路径。实现时可直接简化为“宽 >= narrow_width 时按配置宽度”，并更新注释。

---

## 6. 状态与切换

### 6.1 `CompactRestore`

`src/app/state.rs` 新增字段：

```rust
/// 进入紧凑布局前的桌面折叠状态；Some 表示当前处于紧凑布局。
pub compact_restore: Option<CompactRestore>,
```

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CompactRestore {
    pub prompt_collapsed: bool,
    pub panes_collapsed: bool,
}
```

初始化位置：`AppState` 的唯一构造器（`demo()` / 测试构造路径共用的初始化处）设为 `None`。

### 6.2 `sync_compact_layout`

新增 `src/app/update/compact_layout.rs`：

```rust
pub fn sync_compact_layout(
    state: &mut AppState,
    width: u16,
    height: u16,
    config: &Config,
) {
    let compact = width > 0 && width <= config.narrow_width;
    match (compact, state.compact_restore) {
        (true, None) => {
            state.compact_restore = Some(CompactRestore {
                prompt_collapsed: state.prompt_collapsed,
                panes_collapsed: state.panes_collapsed,
            });
            // 紧凑模式始终把右栏视为展开；Chat 是否显示右栏由 ViewLayout 决定。
            state.prompt_collapsed = false;
            state.panes_collapsed = false;
            // 侧栏在紧凑模式不可见，焦点不能留在侧栏。
            if state.sidebar_focused() {
                super::focus::focus_panes(state);
            }
            // 焦点与右栏 tab 对齐，避免“焦点 Todo、画面 Prompt”的分裂。
            match state.focused {
                FocusedContainer::Prompt => state.right_tab = RightTab::Prompt,
                FocusedContainer::Todo => state.right_tab = RightTab::Todo,
                _ => {}
            }
        }
        (false, Some(restore)) => {
            state.compact_restore = None;
            // 用户正在编辑全屏面板：保留展开与焦点，不恢复“折叠”。
            if !matches!(state.focused, FocusedContainer::Prompt | FocusedContainer::Todo) {
                state.prompt_collapsed = restore.prompt_collapsed;
            }
            state.panes_collapsed = restore.panes_collapsed;
            // 切回桌面时关闭只属于紧凑模式的 switcher。
            if matches!(state.overlay, Some(Overlay::CompactSwitcher(_))) {
                state.overlay = None;
                state.compact_switcher_scroll_drag = None;
            }
        }
        _ => {}
    }

    // switcher 打开期间终端可能 resize：同步可视行数，避免选中项滚出视口。
    if let Some(Overlay::CompactSwitcher(switcher)) = state.overlay.as_mut() {
        switcher.set_viewport(height.saturating_sub(2) as usize);
    }
}
```

调用点：`event::session::current_geometry` 在读取 terminal size 后、`compute_with_panel` 之前调用。

### 6.3 `compact_panel()`

`AppState` 新增：

```rust
/// 紧凑模式下当前应全屏显示的面板；None 表示显示 Chat。
pub fn compact_panel(&self) -> Option<RightTab> {
    match self.focused {
        FocusedContainer::Prompt => Some(RightTab::Prompt),
        FocusedContainer::Todo => Some(RightTab::Todo),
        _ => None,
    }
}
```

说明：

- 不新增 `compact_screen` 字段；焦点是唯一真源。
- `sync_compact_layout` 在进入/退出时把 `right_tab` 与焦点对齐，保证 `right_panel::render` 与 header 标题一致。
- 点击全屏面板顶边框的 `Prompt`/`Todo` tab 时，现有 `select_right_tab` 会同时改焦点与 `right_tab`，因此会自动切换全屏面板。

### 6.4 进入/退出行为表

| 场景 | `prompt_collapsed` | `panes_collapsed` | 焦点 | 画面 |
| --- | --- | --- | --- | --- |
| 桌面 Prompt 展开，缩窄 | 快照 false，临时 false | 快照原值，临时 false | 不变 | 若焦点在 Prompt → 全屏 Prompt；否则 Chat |
| 桌面 Prompt 折叠，缩窄 | 快照 true，临时 false | 快照原值，临时 false | 若在 Sidebar → Panes | Chat（右栏由 ViewLayout 隐藏，不受临时 false 影响） |
| 桌面 Sidebar 聚焦，缩窄 | 同上 | 同上 | 强制 Panes | Chat |
| 紧凑 Chat，点 switch→Prompt | false | false | Prompt | 全屏 Prompt |
| 紧凑 Prompt，点 `[back]` | false | false | Panes | Chat |
| 紧凑 Prompt，拉宽 | 若原桌面折叠则**不恢复**（保持展开） | 恢复快照 | 保持 Prompt | 桌面右栏保持展开并聚焦 |
| 紧凑 Chat，拉宽 | 恢复快照（可能折叠） | 恢复快照 | 保持 Panes | 桌面原三栏 |
| 紧凑 switcher 打开，拉宽 | 恢复快照 | 恢复快照 | 不变 | 关闭 switcher，回桌面 |

### 6.5 为什么不新增 `compact_screen`

新增独立屏幕状态会与 `FocusedContainer` 形成第二真源，必须额外处理：

- 点 tab 改焦点但不改屏幕；
- 切工作区后屏幕与焦点错位；
- 拉宽/缩窄后恢复哪个屏幕；
- 键盘路由与鼠标命中各用哪份状态。

把屏幕直接绑定到 `FocusedContainer` 后，上述问题都不存在：焦点在 Prompt 就是 Prompt 全屏，焦点在 Panes 就是 Chat。

---

## 7. 顶部状态条（compact header）

### 7.1 代码位置

新增 `src/ui/widgets/compact_header.rs`，在 `src/ui/widgets/mod.rs` 注册。

```rust
pub struct CompactHeaderLayout {
    pub area: Rect,
    pub switch: Option<Rect>,
    pub back: Option<Rect>,
    pub status: Rect,
    pub title: Rect,
}

pub fn layout(view: &ViewLayout, state: &AppState) -> CompactHeaderLayout;
pub fn render(frame: &mut Frame<'_>, view: &ViewLayout, state: &AppState);
pub fn switch_at(view: &ViewLayout, state: &AppState, column: u16, row: u16) -> bool;
pub fn back_at(view: &ViewLayout, state: &AppState, column: u16, row: u16) -> bool;
```

### 7.2 几何

- 按钮文案宽度来自 `ui::text::compact`：
  - `COMPACT_SWITCH_LABEL = "[switch]"`（8 列）
  - `COMPACT_BACK_LABEL = "[back]"`（6 列）
  - `COMPACT_CLOSE_LABEL = "[close]"`（7 列，switcher 用）
- Chat 模式：
  - `switch` 右对齐，贴 header 右缘 1 列；
  - `switch.x - 1` 画一条 `│` 分隔线（宽度足够时）；
  - `status` 为 header 左缘到分隔线之间的区域。
- 全屏面板模式：
  - `back` 在 header 左缘 1 列；
  - `title` 紧随其后，显示 `Prompt` / `Todo`；
  - 右侧可显示 `workspace · tab` 上下文（宽度不足时先隐藏）。
- `switch` / `back` 的点击矩形覆盖整个 header 高度（高度 2 时也是 2 行）。

### 7.3 Chat 行内容

第 1 行：

```text
 ● project-a                         tab 1 · 1/3
```

- 左侧状态点：当前工作区 agent 聚合状态，取严重度 `Blocked > Working > Idle/Unknown`。
  - 使用 `text::agent_status_dot(state)` 与 `style::agent_status(state)`。
- 工作区名：`state.active_workspace().name`，`style::strong()`，按可用宽度 `text::ellipsize`。
- 右侧 tab：`tab_label(active_tab, name)`；tabs 数量 > 1 时追加 ` · {active+1}/{len}`；`style::muted()`。
- 宽度不足时先隐藏 tab 状态，再截断工作区名；状态点恒保留。

第 2 行（仅 `height >= 16`）：

```text
 ● 1 blocked · 2 working · 3 idle
```

- 统计来源：`state.global_agent_entries()`。
- 顺序固定 `blocked → working → idle`；数量为 0 的段不显示。
- 段之间用 ` · `（`text::COMPACT_SEPARATOR`）。
- 没有任何 agent 时显示 `no agents`（`text::COMPACT_NO_AGENTS`），整行 muted。
- 最严重的状态决定行首圆点颜色；只保留一个圆点，避免一行出现多个高饱和色点。
- 总宽不足时从尾部截断，并保留 `…`（使用 `text::truncate_to_width` / `ellipsize`）。

### 7.4 全屏面板标题

```text
 [back] Prompt                         project-a · tab 1
```

- `[back]` 使用 `style::accent()`，点击后 `update::back_to_compact_chat`。
- `Prompt` / `Todo` 使用 `style::strong()`。
- 右侧上下文 `workspace · tab` 使用 `style::muted()`，宽度不足时整段隐藏；不影响 `[back]` 与标题。
- 不显示 agent 摘要，把垂直空间留给编辑器。

### 7.5 渲染顺序

`ui::render_cached` 在紧凑模式下：

1. `widgets::compact_header::render(frame, &view, state)`；
2. 主区窗格或全屏右栏；
3. 跳过桌面控制按钮（exit/web/swap/panel/resize）；
4. 跳过 tooltip（v1 紧凑模式不显示 tooltip）；
5. overlay → toast。

header 自身先 `clear` 自己的矩形，避免上一帧残留；不画外框，保持与 herdr mobile 一致的轻量状态条。

---

## 8. 全屏切换器（CompactSwitcher）

### 8.1 数据模型

`src/app/state/compact.rs` 新增：

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompactSection {
    Agents,
    Spaces,
    Tabs,
    Panels,
    Actions,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CompactRow {
    Section(CompactSection),
    Agent(GlobalAgentEntry),
    Workspace(usize),
    Tab(usize),
    Panel(RightTab),
    NewWorkspace,
    NewTab,
}

impl CompactRow {
    pub fn is_selectable(&self) -> bool {
        !matches!(self, Self::Section(_))
    }
}
```

```rust
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GlobalAgentEntry {
    pub workspace: usize,
    pub entry: SidebarAgentEntry,
    /// 侧栏标题覆盖/Recent 标题；无标题为 None。
    pub title: Option<String>,
}
```

`SidebarAgentEntry` 已包含 `tab_index`、`pane_index`、`pane_id`、`kind`、`state`，直接复用，不复制字段。

### 8.2 全局 agents 收集

现有 `AppState::agent_entries()` 只遍历当前工作区。改造为：

```rust
/// 当前激活工作区的 Agents 条目（保持原语义）。
pub fn agent_entries(&self) -> Vec<SidebarAgentEntry> {
    self.agent_entries_for_workspace(self.active_workspace)
}

/// 所有工作区的 Agents 条目；紧凑 switcher 使用。
pub fn global_agent_entries(&self) -> Vec<GlobalAgentEntry> {
    let mut out = Vec::new();
    for workspace in 0..self.workspaces.len() {
        for entry in self.agent_entries_for_workspace(workspace) {
            let title = self
                .pane_anywhere(entry.pane_id)
                .and_then(|pane| pane.agent_display_title(entry.kind))
                .map(str::to_string);
            out.push(GlobalAgentEntry {
                workspace,
                entry,
                title,
            });
        }
    }
    out
}

fn agent_entries_for_workspace(&self, workspace_index: usize) -> Vec<SidebarAgentEntry> {
    // 复用现有 agent_entries 的实现，只把 workspace 从 active 改为参数。
}
```

注意：

- 全局条目顺序 = 工作区顺序 → tab 顺序 → 窗格树序；与侧栏当前工作区条目顺序一致。
- 同一窗格可能同时有 lx 与终端 agent（沿用现有 `agent_entries` 规则），不去重。
- 性能：紧约束在 `switch` 打开时构建一次；header 每帧仅做计数，不在渲染层反复分配大 Vec（如需可后续优化，v1 可接受）。

### 8.3 行顺序

```rust
pub fn compact_rows(&self) -> Vec<CompactRow> {
    let mut rows = Vec::new();

    let agents = self.global_agent_entries();
    if !agents.is_empty() {
        rows.push(CompactRow::Section(CompactSection::Agents));
        rows.extend(agents.into_iter().map(CompactRow::Agent));
    }

    rows.push(CompactRow::Section(CompactSection::Spaces));
    rows.extend((0..self.workspaces.len()).map(CompactRow::Workspace));

    rows.push(CompactRow::Section(CompactSection::Tabs));
    rows.extend(
        (0..self.active_workspace().tabs.len()).map(CompactRow::Tab),
    );

    rows.push(CompactRow::Section(CompactSection::Panels));
    rows.push(CompactRow::Panel(RightTab::Prompt));
    rows.push(CompactRow::Panel(RightTab::Todo));

    rows.push(CompactRow::Section(CompactSection::Actions));
    rows.push(CompactRow::NewWorkspace);
    rows.push(CompactRow::NewTab);

    rows
}
```

规则：

- 没有 agents 时**不显示空的 Agents 区**，避免第一屏出现一个没有内容的标题。
- Spaces 平铺列出所有工作区，不受侧栏分组折叠影响；保证任何工作区都能从 switcher 到达。
- Tabs 只列当前激活工作区的标签。
- Panels 与 Actions 恒定存在。

### 8.4 `CompactSwitcher` 状态

`src/app/overlay/compact_switcher.rs`：

```rust
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompactSwitcher {
    pub rows: Vec<CompactRow>,
    /// 当前高亮行在 rows 中的下标；可能是 Section，但导航/激活会跳过。
    pub selected: usize,
    /// 视口顶部行号。
    pub scroll: usize,
    /// 鼠标悬停行（只铺底色，不移动 ❯）。
    pub hover: Option<usize>,
    /// 可视行数；由 current_geometry 按屏幕高度同步。
    pub viewport_rows: usize,
}

impl CompactSwitcher {
    pub fn new(rows: Vec<CompactRow>, viewport_rows: usize) -> Self;
    pub fn set_viewport(&mut self, viewport_rows: usize);
    pub fn move_selection(&mut self, delta: isize);
    pub fn page(&mut self, delta: isize);
    pub fn ensure_visible(&mut self);
    pub fn max_scroll(&self) -> usize;
    pub fn selectable_row(&self) -> Option<usize>;
}
```

初始高亮：第一个可选行（通常第一条 agent；没有 agents 时第一个 workspace）。

### 8.5 打开与关闭

`src/app/update/compact_layout.rs`：

```rust
pub fn open_compact_switcher(state: &mut AppState, viewport_rows: usize) {
    let rows = state.compact_rows();
    let switcher = CompactSwitcher::new(rows, viewport_rows);
    state.overlay = Some(Overlay::CompactSwitcher(switcher));
    state.compact_switcher_scroll_drag = None;
}

pub fn close_compact_switcher(state: &mut AppState) {
    if matches!(state.overlay, Some(Overlay::CompactSwitcher(_))) {
        state.overlay = None;
        state.compact_switcher_scroll_drag = None;
    }
}
```

入口：点击 header `[switch]`，调用 `open_compact_switcher(state, screen.height.saturating_sub(2) as usize)`。

### 8.6 激活行为

`activate_compact_row`：

| 行 | 行为 |
| --- | --- |
| `Agent(agent)` | 校验工作区/标签/窗格仍存在 → `switch_workspace` → `switch_tab` → `focus_pane` → 关闭 overlay。 |
| `Workspace(index)` | `switch_workspace(state, index)` → 关闭 overlay。焦点留在主区。 |
| `Tab(index)` | `switch_tab(state, index)` → 关闭 overlay。 |
| `Panel(Prompt)` | `focus_prompt(state)`（内部把 `right_tab` 设回 Prompt）→ 关闭 overlay。 |
| `Panel(Todo)` | `state.right_tab = Todo` → `focus_todo(state)` → 关闭 overlay。 |
| `NewWorkspace` | 关闭 overlay → `open_new_workspace_dialog(state)`。 |
| `NewTab` | 关闭 overlay → `create_tab(state)`。 |
| `Section` | 不激活。 |

补充：

- 先关闭 overlay，再执行动作，避免动作内部打开的新浮层（NewWorkspace）被覆盖。
- Agent 行若 `pane_anywhere` 已不存在，直接关闭 overlay，不做半套切换。
- 切换后不调用 `ensure_workspace_visible` / `reveal_active_tab`：紧凑模式没有侧栏/标签栏；下次回桌面由正常事件循环重新计算滚动。

### 8.7 键盘

`OverlayKind::CompactSwitcher` 映射：

| 键 | 行为 |
| --- | --- |
| `Esc` | 关闭 switcher |
| `↑` | 上一个可选行 |
| `↓` | 下一个可选行 |
| `PageUp` | 上翻一屏 |
| `PageDown` | 下翻一屏 |
| `Enter` | 激活当前行 |
| 其他 | 吞掉（不写 PTY） |

`OverlayKey` 已有 `PageUp` / `PageDown`，只需在 `input::overlay_key` 增加映射。
`app::update::dialog::apply_overlay_key` 在其它浮层分支前增加：

```rust
if matches!(state.overlay, Some(Overlay::CompactSwitcher(_))) {
    super::compact::apply_compact_switcher_key(state, key);
    return None;
}
```

### 8.8 渲染

`src/ui/overlay/compact_switcher.rs`：

```rust
pub struct CompactSwitcherLayout {
    pub area: Rect,          // 整个屏幕
    pub close: Option<Rect>,
    pub viewport: Rect,      // 标题 + 横线之下的列表区
    pub scrollbar: Option<widgets::scrollbar::ScrollbarLayout>,
}

pub fn layout(
    screen: Rect,
    switcher: &CompactSwitcher,
) -> CompactSwitcherLayout;

pub fn render(
    frame: &mut Frame<'_>,
    screen: Rect,
    state: &AppState,
    switcher: &CompactSwitcher,
);

pub fn row_at(
    screen: Rect,
    switcher: &CompactSwitcher,
    column: u16,
    row: u16,
) -> Option<usize>;

pub fn close_at(
    screen: Rect,
    switcher: &CompactSwitcher,
    column: u16,
    row: u16,
) -> bool;
```

视觉：

```text
 switch                                      [close]
 ─────────────────────────────────────────────────────
 Agents
   ❯ ● lx  project-a · [T1 P1]
     ○ claude  project-b · [T2 P1]
 Spaces
   ❯ ● project-a
     ○ project-b
 Tabs
   ❯ ▪ tab 1  chat
     ▪ tab 2  terminal
 Panels
   ❯ Prompt
     Todo
 Actions
   ❯ + new workspace
     + new tab
```

- 顶部标题 ` switch`（`style::strong()`），右侧 `[close]`（`style::accent()`）。
- 第 2 行画一条 muted `─` 横线。
- Section 标题：`style::muted().add_modifier(BOLD)`，不参与选中。
- 当前高亮行：`style::overlay_selection()`，整行铺底；`❯` 标记。
- 鼠标悬停未选中行：`style::selection()` 底色，不移动 `❯`。
- Agent 行：
  - 状态圆点：`text::agent_status_dot` + `style::agent_status`；
  - 名称：`text::sidebar_agent_label(kind)`；
  - 若 `title` 非空，名称后显示 ` · title`；
  - 右侧定位：`workspace.name` + `text::agent_location(tab_index, pane_index)`。
- Workspace 行：
  - 左侧 `workspace.name`；
  - 活动工作区用 `style::strong()` + `●`；非活动用 `○`；
  - 右侧 `b:<branch>`（有 git 分支时）`style::muted()`。
- Tab 行：
  - `text::TAB_ITEM_ICON` + `tab_label(index, name)`；
  - 活动 tab 用 `style::strong()`；
  - `has_pending_ask()` 时追加 `text::TAB_ASK_MARKER`。
- Panel 行：`Prompt` / `Todo`；当前 `right_tab` 用 `●`/strong。
- Action 行：`+` 用 `style::accent()`，后接 `new workspace` / `new tab`。
- 宽度不足时先截断右侧定位，再截断标题；`[close]` 与 `❯` 恒保留。
- 溢出时右侧渲染滚动条 `▐`，支持滚轮、`PageUp/PageDown`、轨道点击与 thumb 拖拽。

### 8.9 鼠标命中与滚动

`event/mouse/press.rs` 的 overlay 分支增加：

```rust
if let Some(Overlay::CompactSwitcher(switcher)) = state.overlay.as_ref() {
    let layout = ui::overlay::compact_switcher::layout(*screen, switcher);
    if let Some(bar) = layout.scrollbar
        && bar.track.contains((mouse.column, mouse.row).into())
    {
        match ui::widgets::scrollbar::thumb_grab_offset(&bar, mouse.row) {
            Some(grab) => state.compact_switcher_scroll_drag = Some(grab),
            None => update::set_compact_switcher_scroll(
                state,
                ui::widgets::scrollbar::offset_from_track_row(&bar, mouse.row),
            ),
        }
        *dirty = true;
        return;
    }
}
```

`handle_overlay_click` 增加分支：

- 点 `[close]` → `close_compact_switcher`；
- 点列表行 → 若是可选行：设置 `selected` 并 `activate_compact_switcher`；Section 行忽略；
- 点顶部/横线空白 → 保持打开。

`event/mouse/wheel.rs` 在 overlay 统一早退之前增加：

```rust
if let Some(Overlay::CompactSwitcher(_)) = state.overlay.as_ref()
    && let Some(delta) = vertical_wheel_delta(mouse.kind)
{
    update::scroll_compact_switcher(state, delta);
    *dirty = true;
    return;
}
```

`event/mouse/drag.rs`：

- `compact_switcher_scroll_drag` 为 Some 时，按 `offset_from_drag_row` 更新滚动；
- 松开清除抓取偏移。

`event/mouse/hover.rs`：

- overlay 为 CompactSwitcher 时更新 `hover` 行；关闭按钮/标题区为 None。

### 8.10 新增 `AppState` 字段

```rust
/// 紧凑 switcher 滚动条 thumb 拖拽抓取偏移；None 表示未拖拽。
pub compact_switcher_scroll_drag: Option<u16>,
```

初始化 `None`；关闭 switcher / 退出紧凑时清空。

---

## 9. 渲染与事件集成

### 9.1 `ui::render_cached`

`src/ui/mod.rs`：

```rust
let view = layout::compute_with_panel(
    area,
    config,
    state.sidebar_collapsed,
    state.sidebar_width,
    state.prompt_collapsed,
    state.prompt_width,
    state.prompt_left,
    state.panes_collapsed,
    state.compact_panel(),
);

let pane_rects = if view.main_content.width == 0 || view.main_content.height == 0 {
    Vec::new()
} else {
    tiling::pane_rects(
        &state.active_tab().layout,
        view.main_content,
        config.min_pane_width,
        config.min_pane_height,
    )
};
```

紧凑模式分支：

```rust
if view.compact {
    widgets::compact_header::render(frame, &view, state);
}
```

主区与右栏沿用现有渲染：

- `sidebar_cursor`：`view.left_panel.width == 0` 时不渲染；
- `tab_bar::render`：`view.tab_bar` 为 0 时自动 no-op；
- `main_content::panes::render_panes`：`pane_rects` 为空时 no-op；
- `right_panel::render`：`view.right_panel.width == 0` 时 no-op；
- 桌面控制按钮只在 `!view.compact` 时渲染：

```rust
if !view.compact {
    controls::render_exit_button(...);
    controls::render_swap_button(...);
    controls::render_panel_buttons(...);
    controls::render_resize_hint(...);
}
```

- tooltip：

```rust
let tooltip_area = if view.compact {
    None
} else {
    (!border_tooltip_open)
        .then(|| tooltip::render(frame, area, state, &view, &pane_rects))
        .flatten()
};
```

### 9.2 `event::session::current_geometry`

签名改为 `&mut AppState`（只有 `event/mod.rs` 一个生产调用点）：

```rust
pub(crate) fn current_geometry(
    tui: &mut Tui,
    state: &mut AppState,
    config: &Config,
) -> io::Result<Geometry> {
    let size = tui.terminal().size()?;
    update::sync_compact_layout(state, size.width, size.height, config);
    let screen = Rect::new(0, 0, size.width, size.height);
    let view = ui::layout::compute_with_panel(..., state.compact_panel());

    let mut rects = if view.main_content.width == 0 || view.main_content.height == 0 {
        Vec::new()
    } else {
        tiling::pane_rects(...)
    };
    if !state.prompt_collapsed && view.right_panel.width > 0 {
        rects.push((state.prompt.id(), view.right_panel));
    }
    Ok(Geometry { screen, rects, view })
}
```

行为：

- 紧凑 Chat：主区窗格 rect 正常，prompt 不参与 PTY/命中。
- 紧凑全屏面板：pane rects 为空（PTY 保持原尺寸，不缩成 0），prompt rect 为整块 body。
- 桌面：行为不变。

### 9.3 鼠标按下顺序

`event/mouse/press.rs` 在 overlay 处理之后、`exit_button` / tab bar 之前加入：

```rust
if view.compact && view.compact_header.contains((mouse.column, mouse.row).into()) {
    if ui::widgets::compact_header::switch_at(view, state, mouse.column, mouse.row) {
        update::open_compact_switcher(state, screen.height.saturating_sub(2) as usize);
        *dirty = true;
        return;
    }
    if ui::widgets::compact_header::back_at(view, state, mouse.column, mouse.row) {
        update::back_to_compact_chat(state);
        *dirty = true;
        return;
    }
    // header 其他区域吞掉，不穿透到下方内容。
    return;
}
```

`back_to_compact_chat`：

```rust
pub fn back_to_compact_chat(state: &mut AppState) {
    state.prompt.clear_panel();
    super::focus::focus_panes(state);
}
```

### 9.4 隐藏控件必须一起失效

紧凑模式下以下控件不可见，但几何函数仍可能返回矩形，必须统一加 `view.compact` 闸门，否则会出现“看不见却能点”：

| 控件 | 文件 | 处理 |
| --- | --- | --- |
| 折叠按钮（Sidebar/Prompt/Panes/Agents） | `src/ui/controls/panels.rs::collapse_buttons` | `if view.compact { return Vec::new(); }` |
| Prompt 钉住按钮 | `src/ui/controls/panels.rs::prompt_pin_at` | `if view.compact { return false; }` |
| 交换按钮 | `src/ui/controls/swap.rs::swap_button` | 紧凑时 `main_content` 或 `right_panel` 必有一侧为 0，现有判断已返回 None；补测试确认 |
| 退出按钮 | `src/ui/controls/exit.rs::exit_button` | `tab_bar` 为 0，现有判断返回 None |
| Web 按钮 | `src/ui/controls/footer.rs::web_button` | `left_panel` 为 0，现有判断返回 None |
| 左右栏拖拽提示 | `src/ui/mod.rs`（`render_resize_hint` 调用点） | 紧凑模式不调用 |
| tooltip | `src/ui/tooltip.rs::render` | `if view.compact { return None; }` |

### 9.5 键盘路由

- 紧凑 Chat：`FocusedContainer::Panes`，按键照旧进入 lx 输入或终端 PTY。
- 紧凑全屏 Prompt/Todo：焦点为 `Prompt`/`Todo`，按键照旧进入编辑器/Todo，不写 PTY。
- switcher 打开：`OverlayKind::CompactSwitcher` 接管 `↑/↓/Enter/Esc/PageUp/PageDown`，其他键吞掉。
- `Ctrl+Q` 仍在 `input::route` 最前面处理，任何紧凑屏幕都能退出。
- 不新增任何裸键/全局键。

### 9.6 PTY 尺寸

- 紧凑 Chat：`main_content` 为 body 整块，窗格 PTY 按新宽度 resize。
- 紧凑全屏面板：`pane_rects` 为空，PTY **不** resize（与现有 `panes_collapsed` 同口径），避免把 PTY 缩成 0 列/0 行。
- 回到 Chat / 桌面：事件循环检测 `geometry.rects` 变化，`update::resize_panes` 与 `resize_sessions` 恢复。

---

## 10. 全局 Toast

### 10.1 几何

`src/ui/toast.rs::rect` 在最前面增加：

```rust
if view.compact {
    return Some(Rect::new(
        screen.x,
        screen.y + screen.height.saturating_sub(1),
        screen.width,
        1,
    ));
}
```

说明：

- 紧凑模式忽略 `toast.anchor`：右栏/侧栏可能不可见，底部横幅是唯一稳定落点。
- 桌面模式保持现有锚点规则不变。
- `pane.lx.toast` 不走这个函数，继续在 lx 窗格内容区绘制。

### 10.2 渲染

`src/ui/toast.rs::render` 增加紧凑分支：

```rust
if view.compact {
    // 清底 + 单行：` ● <title> · <message>`，按宽度截断，不换行。
    return Some(compact_banner::render(frame, area, toast));
}
```

建议新增 `src/ui/toast/compact.rs` 或 `ui/toast.rs` 内部的 `draw_compact_banner`：

- `clear` 整行；
- 行首 ` ● `，圆点颜色按 `ToastKind`（复用 `border_style` / `toast_title`）；
- 有标题时：`title · message`；无标题只显示 message；
- 正文加粗/语义色与现有 toast 一致；
- 按显示宽度截断，保证单行不越界。

### 10.3 交互

- 点击横幅或横幅外关闭：沿用现有 `ui::toast::rect` 命中与 `dismiss_toast`。
- 浮层打开时 Toast 仍可渲染在底部；点击横幅先关闭 Toast，不穿透到浮层（沿用现有按下顺序）。
- 测试：紧凑模式断言 rect 在最后一行；桌面模式原测试不变。

---

## 11. 配置、设置页与规范文档

### 11.1 不新增配置

继续使用 `Config.narrow_width`：

- 注释从“低于该宽度隐藏侧栏”改为“低于等于该宽度进入紧凑布局”。
- `Config::validate` 不改；`narrow_width < min_width` 时紧凑永远不会生效（低于 min 先显示最小尺寸提示），属于可接受配置。
- `to_json` 字段名不变，不需要 DB 迁移。

### 11.2 设置页

`assets/web.html`：

```html
<div><label for="narrow_width">Narrow layout width</label><input id="narrow_width" type="number" min="1" max="500"></div>
```

`assets/settings.js` 的 `CONFIG_KEYS` 不变（仍是 `narrow_width`）。

### 11.3 项目规范

在 `docs/standards/tui-design-requirements.md` 增加一条紧凑布局规则，至少包含：

- 触发条件 `width <= narrow_width`；
- 单列几何、隐藏侧栏/标签栏/右栏；
- header 两档高度与内容；
- `switch` 的全局 agents / spaces / tabs / panels / actions 顺序；
- `[back]` 返回、不劫持 `Esc`；
- 紧凑模式底部 Toast；
- 隐藏控件不得保留命中矩形；
- 全屏面板时 PTY 不 resize。

---

## 12. 测试与验收

### 12.1 布局单测（`src/ui/layout/tests.rs`）

新增：

1. `compact_layout_at_threshold_hides_all_columns`：80x24、`None` → `compact == true`、header 高 2、侧栏/标签栏/右栏宽 0、主区从 y=2 开始占满 80 列。
2. `compact_layout_short_screen_uses_one_row_header`：60x12 → header 高 1、主区 y=1。
3. `compact_fullscreen_panel_uses_body`：60x20、`Some(RightTab::Prompt)` → header 高 1、`right_panel == body`、`main_content.width == 0`。
4. `desktop_layout_starts_above_narrow_width`：81x24 → `compact == false`，三栏几何与现状一致。
5. `desktop_layout_unchanged_at_wide_screen`：100x30 断言现有几何。

### 12.2 Header 单测（`src/ui/widgets/compact_header/tests.rs`）

1. Chat 80x24：header 高 2，`[switch]` 贴右缘，点击矩形覆盖 2 行。
2. Chat 60x10：header 高 1，`[switch]` 仍在，状态行不越界。
3. 全屏面板：`[back]` 在左缘，`Prompt` 标题可见，右侧上下文放不下时隐藏。
4. 长工作区名/tab 状态按预期截断，`[switch]` 不被挤掉。
5. agent 摘要：blocked/working/idle 顺序、零值省略、无 agents 显示 `no agents`。

### 12.3 Switcher 单测（`src/ui/overlay/compact_switcher/tests.rs`）

1. 行顺序：Agents（有 agent 时）→ Spaces → Tabs → Panels → Actions。
2. Section 行不可选中；`↑/↓` 跳过 Section。
3. 高度不足时出现滚动条，`row_at` 与渲染使用同一份 `scroll`。
4. 选中行整行高亮，hover 只铺底色不移动 `❯`。
5. 宽度 40 时 `[close]` 与 `❯` 保留，右侧定位先截断。
6. `close_at` 命中 `[close]`。

### 12.4 状态与更新单测（`src/app/update/compact_layout/tests.rs`）

1. `sync_compact_layout` 进入时快照并清零 `prompt_collapsed`/`panes_collapsed`；退出恢复。
2. 退出时焦点在 Prompt/Todo 则保持面板展开，不恢复折叠。
3. Sidebar 焦点进入紧凑后落到 Panes。
4. `compact_rows` 正确包含全局 agents（跨工作区）与当前工作区 tabs。
5. `activate_agent` 切工作区、切 tab、聚焦窗格。
6. `activate_workspace` / `activate_tab` / `activate_panel` 行为。
7. `NewWorkspace` 打开现有 `NewWorkspace` 浮层；`NewTab` 立即新建。
8. switcher 打开时拉宽 → 关闭 overlay。

### 12.5 端到端渲染/事件测试（`src/event/tests/compact.rs`）

1. 60x20 渲染：第一行包含工作区名与 `[switch]`，第二行包含 agent 摘要；不出现侧栏/标签栏。
2. 60x10 渲染：header 只有 1 行。
3. 点击 `[switch]`：`Overlay::CompactSwitcher` 打开；再次点击 `[close]`/`Esc` 关闭。
4. 在 switcher 中选 `project-b` + tab 2：`state.active_workspace` / `active_tab` 正确，overlay 关闭。
5. 选全局 agent：工作区、tab、焦点窗格全部正确。
6. 选 `Prompt`：`focused == Prompt`，`right_tab == Prompt`，整屏渲染 Prompt；点 `[back]` 回到 Panes。
7. 全屏 Prompt 中保存/输入按键仍进入编辑器，不写 PTY。
8. 紧凑 Chat 中鼠标滚轮/终端按键仍作用于窗格。
9. 紧凑全屏面板时 `geometry.rects` 不包含窗格；PTY 尺寸不变（可用模拟 session size 断言）。
10. 紧凑模式点击全屏面板底部原折叠按钮位置，不会切换 `prompt_collapsed`。
11. 全局 Toast 渲染在最后一行；点击关闭。
12. 80 列恰好进入紧凑；81 列回桌面。

### 12.6 现有测试调整

- `src/ui/layout/tests.rs` 现有两条窄屏测试会与新语义冲突，必须改写：
  - `narrow_width_hides_sidebar_but_keeps_prompt`（60x30）：旧断言“右栏仍 30 列”不再成立，改为断言 `compact == true`、右栏宽 0、主区占满。
  - `collapsed_narrow_sidebar_is_hidden`（60x30）：改为断言紧凑布局下侧栏与右栏均为 0。
  - 若仍需保留“桌面窄屏隐藏侧栏”的旧语义测试，把宽度改为 81~100 并按桌面三栏断言，或直接删除已被新语义取代的断言。
- `src/event/tests/test_support.rs::geometry` 继续调用 `compute`；若测试在 80 列下依赖 Prompt 几何，把屏幕改为 100/120，或改用 `compute_with_panel(..., state.compact_panel())` 并按紧凑语义断言。
- `src/event/tests/focus_and_paste.rs`、`src/event/tests/workspace_history.rs` 中 80 列的桌面几何断言需逐条核对：
  - 若测的是桌面三栏 → 宽度改为 100；
  - 若测的是紧凑行为 → 增加 compact 专用断言。
- `src/ui/left_panel/tests.rs` 的 `ViewLayout { ... }` 字面量补 `compact_header`、`compact` 字段。
- 所有直接构造 `ViewLayout` 的测试补字段。
- 设置页/配置测试无需改字段，只需更新文案相关快照（若有）。

### 12.7 验证命令

```bash
cargo fmt
cargo test --lib
cargo clippy --all-targets -- -D warnings
cargo test
```

交互与外观必须由用户运行：

```bash
cargo run
```

手动验收：

1. 将终端拉到 80x24 / 60x24 / 40x12，确认自动切单列。
2. 点击 `[switch]`，确认 Agents/Spaces/Tabs/Panels/Actions 顺序与滚动。
3. 点击任意 agent，确认跨工作区/标签跳转。
4. 打开 Prompt，编辑并返回聊天。
5. 在紧凑 Prompt 中拉宽到 120，再缩回 60，确认焦点/面板保持。
6. 复制终端选区，确认 Toast 在底部。
7. 在 40 列下点击全屏面板底部原折叠按钮位置，确认无隐藏命中。
8. 回到 81 列，确认桌面三栏与折叠状态恢复。

---

## 13. 风险与缓解

| 风险 | 影响 | 缓解 |
| --- | --- | --- |
| `prompt_collapsed` 快照恢复错误 | 退出紧凑后右栏意外展开/折叠 | 状态机单测覆盖四种进入焦点 + 两种原折叠状态；退出时若焦点在面板则保留展开。 |
| 隐藏控件仍有命中矩形 | 点击不可见区域触发折叠/钉住 | `collapse_buttons` / `prompt_pin_at` / `render_resize_hint` / tooltip 统一加 `view.compact` 闸门；端到端测试点击原按钮位置。 |
| 80 列现有测试批量失败 | 开发验证噪音 | 设计阶段列出受影响测试；桌面断言统一改 100 列，80 列只用于 compact 测试。 |
| 全屏面板把 PTY resize 成 0 | 终端/lx 进程尺寸损坏 | 面板模式 `geometry.rects` 不含窗格，PTY 保持原尺寸；回 Chat 再 resize。 |
| switcher 行快照过期 | 关闭工作区后选中项失效 | 激活前校验工作区/tab/pane；失效则只关闭 overlay，不做半套切换。 |
| 40 列下 `[close]`/`❯` 被挤掉 | 无法关闭/看不出选中 | 固定保留 `[close]`、`❯` 与 1 列间距；先截断右侧定位，再截断标题。 |
| 底部 Toast 遮住 switcher 最后一行 | 最后一行不可点 | Toast 渲染在 overlay 之后；点击 Toast 只关闭 Toast；switcher 可通过滚轮/键盘到达最后一行。 |
| 现有 `narrow_width=80` 语义变化 | 80 列从三栏变单列 | 已确认这是目标行为；设置页文案改为 `Narrow layout width`，文档同步。 |
| 新增浮层变体漏改匹配 | 编译错误或行为遗漏 | `OverlayKind` / `Overlay` / `input::overlay_key` / `apply_overlay_key` / `ui::overlay::render` / 点击/悬停/滚轮逐处补分支，编译器兜底。 |

---

## 14. 关键代码清单（实现时对照）

### 14.1 布局

- `src/ui/layout.rs`
  - `ViewLayout` 增加 `compact_header`、`compact`
  - `compute` 保持 8 参数，委托 `compute_with_panel(..., None)`
  - `compute_with_panel` 增加紧凑分支
  - `layout/tests.rs` 增加紧凑边界测试

### 14.2 状态与更新

- `src/app/state.rs`
  - `AppState.compact_restore: Option<CompactRestore>`
  - `AppState.compact_switcher_scroll_drag: Option<u16>`
  - `AppState::compact_panel()`
- `src/app/state/compact.rs`
  - `CompactRestore`
  - `CompactSection` / `CompactRow` / `GlobalAgentEntry`
  - `agent_entries_for_workspace` / `global_agent_entries` / `compact_rows`
- `src/app/update/compact_layout.rs`
  - `sync_compact_layout`
  - `open_compact_switcher` / `close_compact_switcher`
  - `move_compact_switcher` / `scroll_compact_switcher` / `set_compact_switcher_scroll`
  - `activate_compact_switcher` / `apply_compact_switcher_key`
  - `back_to_compact_chat`
- `src/app/update.rs`
  - `mod compact; pub use compact::*;`
  - `Action::TogglePrompt` / `TogglePanes` 在 `compact_restore.is_some()` 时忽略（防止隐藏入口被触发）

### 14.3 浮层

- `src/app/overlay.rs`
  - `OverlayKind::CompactSwitcher`
  - `Overlay::CompactSwitcher(CompactSwitcher)`
  - `kind()` 分支
- `src/app/overlay/compact_switcher.rs`
  - `CompactSwitcher` 状态与导航方法
- `src/input/mod.rs`
  - `overlay_key` 的 `CompactSwitcher` 分支（Esc/Up/Down/Enter/PageUp/PageDown）
- `src/app/update/dialog.rs`
  - `apply_overlay_key` 的 CompactSwitcher 早分支
- `src/ui/overlay.rs`
  - `mod compact_switcher; pub use ...`
  - `render` 分支
- `src/ui/overlay/compact_switcher.rs`
  - `layout` / `render` / `row_at` / `close_at`

### 14.4 渲染与事件

- `src/ui/widgets/compact_header.rs` + `src/ui/widgets/mod.rs`
- `src/ui/text/compact.rs` + `src/ui/text.rs`
- `src/ui/mod.rs`
  - `compute_with_panel`
  - 紧凑 header
  - 控制按钮/tooltip 闸门
- `src/event/session.rs`
  - `current_geometry(&mut AppState)`
  - `sync_compact_layout`
  - `compute_with_panel`
  - pane rects 为空时保持 PTY
- `src/event/mouse/press.rs`
  - header switch/back
  - switcher 滚动条/点击
- `src/event/mouse/hover.rs`
  - switcher hover
- `src/event/mouse/wheel.rs`
  - switcher 滚轮
- `src/event/mouse/drag.rs`
  - switcher thumb 拖拽
- `src/ui/controls/panels.rs`
  - `collapse_buttons` / `prompt_pin_at` compact 闸门
- `src/ui/mod.rs`
  - 紧凑模式跳过 `controls::render_resize_hint`（渲染入口闸门）
- `src/ui/tooltip.rs`
  - compact 直接返回 None
- `src/ui/toast.rs`
  - compact 底部横幅 rect/render

### 14.5 配置与文档

- `assets/web.html`：`Narrow layout width`
- `src/config/mod.rs`：`narrow_width` 注释
- `docs/standards/tui-design-requirements.md`：紧凑布局规则

---

## 15. 验收定义

满足以下全部条件才算完成：

1. `width <= narrow_width` 自动进入单列，`width > narrow_width` 恢复桌面三栏。
2. 80/60/40 列下 header 不截断关键入口，`[switch]` 可点击。
3. `switch` 能完成全局 agent 跳转、工作区切换、tab 切换、新建工作区/标签、进入 Prompt/Todo。
4. 全屏 Prompt/Todo 可编辑，`[back]` 回聊天，`Esc` 不被劫持。
5. 拉宽/缩窄后焦点与面板保持一致，折叠状态可恢复。
6. 全屏面板不改变 PTY 尺寸；回聊天后尺寸正确恢复。
7. 紧凑模式底部 Toast 不遮挡 header；桌面 Toast 行为不变。
8. 隐藏控件（折叠/钉住/交换/退出/Web/拖拽提示）无命中、无渲染。
9. `cargo fmt`、`cargo clippy --all-targets -- -D warnings`、`cargo test` 全部通过。
10. 用户 `cargo run` 手动验收 40x12 / 60x24 / 80x24 / 120x30 四种尺寸。
