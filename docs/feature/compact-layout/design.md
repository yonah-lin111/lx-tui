# 窄终端紧凑布局（Compact Layout）— 详细设计（v2）

> 状态：**已实现，待用户 `cargo run` 手动验收**
>
> 参考：opencode-dev TUI 以 `dimensions().width - verticalTabsWidth > 120` 判定宽屏；lx 直接采用 **总宽 120** 作为紧凑阈值。

## 1. 目标

窄终端下不再维护桌面三栏的最小宽度，而是切换为单列布局：

- 顶部栏一行导航、一行状态；
- 中间内容默认是 Chat/窗格；
- 左侧栏与右侧栏按需全屏打开，内容复用桌面实现。

非目标：

- 不做抽屉动画、不做半屏分栏；
- 不新增第二套侧栏/右栏内容渲染；
- 不改变桌面三栏行为。

## 2. 触发与几何

### 2.1 阈值

`Config.narrow_width` 默认值改为 `120`：

```rust
pub fn is_compact(width: u16, config: &Config) -> bool {
    width > 0 && width <= config.narrow_width
}
```

- `width <= 120`：紧凑布局；
- `width >= 121`：桌面三栏。

`resolved_sidebar_width` 的宽屏判断同步改为 `area.width > config.narrow_width`。

### 2.2 `ViewLayout`

新增/调整字段：

```rust
pub struct ViewLayout {
    pub left_panel: Rect,
    pub tab_bar: Rect,          // 紧凑模式 = 顶部栏中间的 tab 条
    pub main_content: Rect,
    pub right_panel: Rect,
    pub right_panel_left: bool,
    pub compact_header: Rect,   // 紧凑顶部栏整体（1 或 2 行）
    pub compact_left_button: Rect,
    pub compact_right_button: Rect,
    pub compact_status: Rect,   // 第二行；短屏为 0
    pub compact: bool,
}
```

### 2.3 紧凑几何

```text
┌ [ws] │ tab 1 │ tab 2 │ [+] ───────────── [panel] ┐   row 0
│ ● workspace          ○ 1 working · 2 idle          │   row 1（高度 >= 16）
├────────────────────────────────────────────────────┤
│                                                    │
│  Chat / 全屏左栏 / 全屏右栏                         │
│                                                    │
└────────────────────────────────────────────────────┘
```

- 顶部栏高度：`height >= 16` 为 2，否则为 1；
- `[ws]` 宽 4、`[panel]` 宽 7，各留 1 列分隔；
- `view.tab_bar` 指向两按钮之间的 strip（高度 1），直接复用原标签栏布局/渲染/命中；
- `CompactPanel::Left`：`left_panel = body`，主区与右栏为 0；
- `CompactPanel::Right`：`right_panel = body`，主区与左栏为 0；
- 无面板：`main_content = body`。

全屏面板不改变顶部栏高度，避免切换时整屏跳动。

## 3. 状态模型

`src/app/state/compact.rs`：

```rust
pub struct CompactRestore {
    pub sidebar_collapsed: bool,
    pub prompt_collapsed: bool,
    pub panes_collapsed: bool,
}

pub enum CompactPanel {
    Left,
    Right,
}
```

`AppState::compact_panel()` 由焦点唯一推导：

| 焦点 | 全屏面板 |
| --- | --- |
| `Sidebar` | `Left` |
| `Prompt` / `Todo` | `Right`（具体页由 `right_tab` 决定） |
| `Panes` | `None`（Chat） |

不新增 `compact_screen` 状态。

### 3.1 进入/退出

`sync_compact_layout(state, width, config)`：

- 进入：快照三个折叠状态，临时全部展开；不移动焦点（侧栏持焦点则直接显示全屏左栏）；
- 退出：恢复快照；焦点仍在 Prompt/Todo 时保留右栏展开；侧栏恢复为折叠态且焦点在侧栏时，把焦点交回主区；
- 紧凑期间每个事件循环对齐 `right_tab` 与 Prompt/Todo 焦点。

### 3.2 按钮与返回

- `toggle_compact_left`：侧栏已聚焦 → `back_to_compact_chat`；否则 `focus_sidebar`；
- `toggle_compact_right`：右栏已聚焦 → `back_to_compact_chat`；否则聚焦 `right_tab` 当前页；
- 点击中间任一 tab 走原 `switch_tab`，主区获得焦点，自动回到 Chat；
- 仅全屏左栏允许裸 `Esc` 返回 Chat；右栏 Esc 保留 Prompt/Todo 原有语义。

## 4. 顶部栏渲染

`src/ui/widgets/compact_header.rs`：

- 清空 `compact_header`；
- `[ws]` 在 `compact_left_button`，`[panel]` 在 `compact_right_button`；
- 打开对应栏时按钮使用 `style::selected_item()`（Cyan 底黑字），否则 `style::accent()`；
- 第二行左侧：当前工作区 Agent 聚合状态点 + 工作区名（按可用宽度截断）；
- 第二行右侧：全局 Agent 摘要 `N blocked · N working · N idle`，无 Agent 时 `○ no agents`；空间不足时优先省略摘要；
- 中间 tab 条由 `tab_bar::render` 在同一帧随后绘制。

## 5. 中间 tab 条

复用 `ui::tab_bar`：

- `tab_bar::layout` 在 `view.compact` 时不再为 `[EXIT]` 预留右端空间；
- `exit_button` 在紧凑模式直接返回 `None`，渲染与命中同时消失；
- 保留原行为：点击切换、右键菜单、悬停高亮、`[+]` 新建、`[<]/[>]` 滚动；
- `view.tab_bar` 就是中间 strip，因此 `reveal_active_tab`、左右滚动、右键菜单无需分叉。

## 6. 全屏左/右栏

- 左栏：`ui::left_panel::render` 直接渲染 `view.left_panel`；工作区列表、历史列表、Agents 分区、`[+]`/`[WEB]` 等交互全部保留；
- 右栏：`ui::right_panel::render` 直接渲染 `view.right_panel`；Prompt/Todo 边框 tab、工具按钮、编辑与滚动行为全部保留；
- `ui/mod.rs` 在紧凑模式忽略 `sidebar_collapsed` 的窄条分支，左栏始终按真实内容绘制；
- 全屏栏打开时 `geometry.rects` 不含任何窗格，PTY 尺寸保持；
- 隐藏控件统一加 `view.compact` 闸门：折叠按钮、钉住、交换、退出、桌面分割线拖拽/hover、tooltip；
- 窗格内部拆分线在 Chat 模式仍可拖拽。

## 7. 事件路由

- 鼠标按下：顶部栏内先判 `[ws]` / `[panel]`；点在 tab strip 内继续走原 tab 命中，点在状态行或空白则吞掉；
- 悬停：原 tab hover 逻辑直接复用；紧凑模式不产生侧栏/prompt 分割线 hover；
- 滚轮：原 tab 条不处理滚轮，保持现状；右侧栏/左栏内部滚动走各自原有逻辑；
- 键盘：`event/input.rs` 在 workspace history 处理后、通用 `input::route` 前拦截左栏裸 Esc；
- `Resize` 事件置 dirty，下一帧重新同步紧凑状态与 PTY 几何。

## 8. Toast

- 紧凑模式全局 Toast 固定为屏幕底部单行横幅：`● title · message`；
- 点击横幅关闭；
- `pane.lx.toast` 仍绘制在 lx 窗格内。

## 9. 删除旧 switcher

顶部栏改为左右按钮 + 真实 tab 条后，旧 `[switch]` 全屏 switcher 的所有能力已被覆盖：

- Agents/Spaces → 全屏左栏；
- Tabs → 中间 tab 条；
- Panels → 全屏右栏（Prompt/Todo 边框 tab）；
- `+ new workspace` → 左栏原有 `[+]`/右键菜单；
- `+ new tab` → tab 条 `[+]`。

因此删除：

- `OverlayKind::CompactSwitcher` / `Overlay::CompactSwitcher`；
- `src/app/overlay/compact_switcher.rs`；
- `src/ui/overlay/compact_switcher.rs` 及其测试；
- `CompactRow` / `CompactSection` / `CompactAgentRow` / `compact_rows` / 全局切换器导航；
- `compact_switcher_scroll_drag` 状态与鼠标/滚轮/拖拽分支。

## 10. 变更文件

### 新增

- `src/app/state/compact.rs` + `src/app/state/compact/tests.rs`
- `src/app/update/compact_layout.rs` + `src/app/update/compact_layout/tests.rs`
- `src/ui/widgets/compact_header.rs` + `src/ui/widgets/compact_header/tests.rs`
- `src/ui/text/compact.rs`
- `src/event/tests/compact.rs`

### 修改

- `src/ui/layout.rs` + `src/ui/layout/tests.rs`
- `src/ui/mod.rs`
- `src/ui/tab_bar.rs` + `src/ui/tab_bar/tests.rs`
- `src/ui/controls/exit.rs`
- `src/ui/controls/panels.rs`
- `src/ui/tooltip.rs`
- `src/ui/toast.rs` + `src/ui/toast/tests.rs`
- `src/app/state.rs`
- `src/app/update.rs`
- `src/app/update/input.rs`（Esc）
- `src/app/overlay.rs`
- `src/ui/overlay.rs`
- `src/input/mod.rs`
- `src/event/input.rs`
- `src/event/session.rs`
- `src/event/mod.rs`
- `src/event/mouse/{press,hover,wheel,drag}.rs`
- `src/event/overlay.rs`
- `src/config/mod.rs`
- `assets/web.html`
- `docs/standards/tui-design-requirements.md`
- 测试夹具：`src/ui/test_support.rs`、`src/event/tests/test_support.rs`、`tests/prompt_editor.rs`、`tests/prompt_short_ref.rs`、`tests/terminal_cursor.rs`

## 11. 测试覆盖

- 阈值：120 → compact，121 → desktop；
- 几何：2 行/1 行顶部栏、按钮位于两端、tab strip 位于中间、左/右全屏 body；
- 状态：进入快照三项、退出恢复、右栏焦点保留、折叠侧栏退出交回主区；
- 按钮：左/右 toggle、tab 点击返回并切换、Esc 仅关左栏；
- tab 条：紧凑模式无 `[EXIT]`、`[+]` 新建、溢出滚动；
- 命中：状态行吞点击、隐藏控件无命中、全屏侧栏行仍可点；
- 渲染：`[ws]`/`[panel]`、tab、状态行、Agent 摘要、活动按钮实心高亮。

## 12. 手动验收

在 worktree 运行 `cargo run`，检查：

- 120x30 进入紧凑，121x30 恢复三栏；
- `[ws]` 打开/关闭全屏左栏，工作区/Agents 可点；
- `[panel]` 打开/关闭全屏右栏，Prompt/Todo 切换正常；
- 中间 tab 切换、`[+]`、多 tab 滚动、右键菜单；
- 左栏 Esc 返回，右栏 Esc 不清空编辑；
- 复制终端内容 Toast 在底部单行显示；
- 拉宽/缩窄后焦点与折叠状态正确恢复。
