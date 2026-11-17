# 窄终端紧凑布局（Compact Layout）— 详细设计（v2）

> 状态：**已完成实现、Review 修复、`dev@d5bf1251` 合并冲突与 `P0-2` 对齐，全量自动化验证通过并合入 `dev`**
>
> 参考：opencode-dev TUI 以 `dimensions().width - verticalTabsWidth > 120`（总宽约 162 列）判定宽屏；lx 采用 **总宽 160** 作为紧凑阈值，使半屏偏宽终端也能触发紧凑布局。

## 1. 目标

窄终端下不再维护桌面三栏的最小宽度，而是切换为单列布局：

- 顶部单行导航栏：`[ws]` + tab 条 + `[panel]`；
- 中间内容默认是 Chat/窗格；
- 左侧栏与右侧栏按需全屏打开，内容复用桌面实现。

非目标：

- 不做抽屉动画、不做半屏分栏；
- 不新增第二套侧栏/右栏内容渲染；
- 不改变桌面三栏行为。

## 2. 触发与几何

### 2.1 阈值

`Config.narrow_width` 运行期固定默认 `160`（不再持久化到 `settings.json`，且忽略旧文件中的 `narrow_width` 字段；Web 设置页与 `settings` CLI 不再暴露该项，仅测试夹具在内存中覆写）：

```rust
pub fn is_compact(width: u16, config: &Config) -> bool {
    width > 0 && width <= config.narrow_width
}
```

- `width <= 160`：紧凑布局；
- `width >= 161`：桌面三栏。

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
    pub compact_header: Rect,   // 紧凑顶部导航栏（固定 1 行）
    pub compact_left_button: Rect,
    pub compact_right_button: Rect,
    pub compact: bool,
}
```

### 2.3 紧凑几何

```text
┌ [ws] │ tab 1 │ tab 2 │ [+] ───────────── │ [panel] ┐   row 0
├────────────────────────────────────────────────────┤
│                                                    │
│  Chat / 全屏左栏 / 全屏右栏                         │
│                                                    │
└────────────────────────────────────────────────────┘
```

- 顶部栏高度：固定 1 行（高度 0 时为 0）；
- `[ws]` 宽 4、`[panel]` 宽 7（总宽 `< 11` 时按 `area.width - left_width` 夹紧，保证两按钮及 `tab_bar` 互不重叠），两端各留 1 列分隔；
- `view.tab_bar` 指向两按钮之间的 strip（高度 1，起点夹到 `right_button.x` 之内），直接复用原标签栏布局/渲染/命中；
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

- 进入：清理在途拖拽/分割线悬停/tooltip，快照三个折叠状态并临时全部展开；不移动焦点（侧栏持焦点则直接显示全屏左栏）；
- 退出：清理在途拖拽/悬停，恢复快照；焦点仍在 Prompt/Todo 时保留右栏展开；`prompt_left && panes_focused()` 时保留主区展开；侧栏恢复为折叠态且焦点在侧栏时，若右列主区也折叠则转交展开中的右栏，否则交回主区；
- 紧凑期间每个事件循环对齐 `right_tab` 与 Prompt/Todo 焦点；`Action::ToggleSidebar`/`TogglePrompt`/`TogglePanes` 在紧凑期间忽略。

### 3.2 按钮、选区与返回

- `toggle_compact_left`：侧栏已聚焦 → `back_to_compact_chat`；否则 `focus_sidebar`；
- `toggle_compact_right`：右栏已聚焦 → `back_to_compact_chat`；否则先清理隐藏容器选区再 `select_right_tab`；
- `back_to_compact_chat`：先清理选区并收起 Prompt 瞬态面板，再 `focus_panes`；
- 点击中间任一 tab 或 `[+]` 走原 `switch_tab`/`create_tab`，主区获得焦点，自动回到 Chat；
- 仅全屏左栏允许裸 `Esc` 返回 Chat（工作区历史模式下首次 `Esc` 先退出历史模式）；右栏 Esc 保留 Prompt/Todo 原有语义；
- `has_visible_selection` 在紧凑模式下仅检查当前 `compact_panel()` 对应的唯一可见容器，避免隐藏容器残留选区吞 `Esc`；`open_prompt_for_pane`/`open_prompt_page` 补齐全量选区清理，`focus_prompt` 切换自 Todo 时提交进行中编辑，`commit_worktree_open` 打开已有 worktree 时交回主区焦点。

## 4. 顶部栏渲染

`src/ui/widgets/compact_header.rs`：

- 清空 `compact_header`；
- `[ws]` 在 `compact_left_button`，`[panel]` 在 `compact_right_button`；
- 打开对应栏时按钮使用 `style::selected_item()`（Cyan 底黑字），否则 `style::accent()`；
- 当 `view.tab_bar.width > 0` 时，在 `left_button.right()` 与 `tab_bar.right()` 两侧各绘制 `text::TAB_SEPARATOR`（`│`，`style::muted()`）；极窄无 tab 空间时跳过分隔符，避免覆盖 `[panel]`；
- 中间 tab 条由 `tab_bar::render` 在同一帧随后绘制。

## 5. 中间 tab 条

复用 `ui::tab_bar`：

- `tab_bar::layout` 在 `view.compact` 时不再为 `[EXIT]` 预留右端空间；
- `exit_button` 在紧凑模式直接返回 `None`，渲染与命中同时消失；
- 保留原行为：点击切换、右键菜单、悬停高亮、`[+]` 新建、`[<]/[>]` 滚动；溢出且宽度 `< 9` 时按 `[>]` → `[+]` → `[<]` 顺序省略而非重叠矩形；
- `view.tab_bar` 就是中间 strip，因此 `reveal_active_tab`、左右滚动、右键菜单无需分叉。

## 6. 全屏左/右栏

- 左栏：`ui::left_panel::render` 直接渲染 `view.left_panel`；工作区列表、历史列表、Agents 分区（保留分区内 `[▼]`/`[▲]` 折叠按钮）、`[+]`/`[WEB]` 等交互全部保留；
- 右栏：`ui::right_panel::render` 直接渲染 `view.right_panel`；Prompt/Todo 边框 tab、工具按钮、编辑与滚动行为全部保留；底边框身份标签在紧凑模式下使用全宽至 `area.right() - 2`（不为已隐藏的 `[pin]`/`[▶]` 预留空白）；
- `ui/mod.rs`、`ui/right_panel/mod.rs` 与 `ui/controls/footer.rs` 在 `view.compact` 时忽略桌面三栏折叠状态，防止直接调用 `ui::render` 时误画 1 列窄条或隐藏 `[WEB]`；
- 全屏栏打开时 `geometry.rects` 不含任何窗格，PTY 尺寸保持（同时保留桌面 `prompt_left && panes_collapsed` 不把 4 列折叠窄条放入 `geometry.rects` 的保护）；
- 隐藏控件统一加 `view.compact` 闸门：外框折叠按钮（`Sidebar`/`Prompt`/`Panes`）、钉住、交换、退出、桌面分割线拖拽/hover、tooltip（渲染与 `target_at` 命中/悬停计时一并关闭）；
- 窗格内部拆分线在 Chat 模式仍可拖拽。

## 7. 事件路由与缓存同步

- 鼠标按下：顶部栏内先判 `[ws]` / `[panel]`；点在 tab strip 内继续走原 tab 命中，点在间隙或空白则吞掉；
- 悬停：原 tab hover 逻辑直接复用；紧凑模式不产生侧栏/prompt 分割线 hover，不触发 `tooltip::target_at`；
- 滚轮与 Todo 视口：原 tab 条不处理滚轮；`event/mod.rs` 仅在右栏展开且为 `RightTab::Todo` 时调用 `resolve_todo_viewport`，避免隐藏状态下按 1 列宽度污染 `state.todo.scroll`；`autoscroll`、`drag.rs`、`wheel.rs` 与 `workspace_list_rect` 按可见面板宽度（`> COLLAPSED_STRIP`）守卫，防止隐藏面板触发拖拽/滚轮自动滚动；
- 缓存与缩放：`session::sync_layout_geometry` 按 `state.all_pane_ids()` 保留 `ui_cache`（全屏左/右栏期间不误清隐藏 `lx` 窗格缓存），并在终端尺寸或 `compact` 状态变化时钳制 `workspace_scroll`、调用 `reveal_active_tab` 及重置指针形状；
- 键盘：`event/input.rs` 在 workspace history 处理后、通用 `input::route` 前拦截左栏裸 Esc；
- 模态创建 Agent：全屏左栏右键 `Agents` 空白区打开 `New agent` 菜单/弹窗创建新终端标签后，焦点交回 `Panes` 自动返回 Chat 并滚入激活标签；
- `Resize` 事件置 dirty，下一帧重新同步紧凑状态与 PTY 几何。

## 8. Toast

- 紧凑模式全局 Toast 完全复用桌面同款带边框浮框（不再使用底部单行横幅）：
  - 锚点容器可见且放得下时贴容器顶边框下方居中；
  - 无锚点、容器不可见（如全屏左栏时）或放不下时，降级到屏幕顶部 `y = 1` 居中；
- 点击 Toast 盒或外部关闭；
- `pane.lx.toast` 仍绘制在 lx 窗格内。

## 9. 删除旧 switcher

顶部栏改为左右按钮 + 真实 tab 条后，旧 `[switch]` 全屏 switcher 的所有能力已被覆盖，已彻底移除（见 v1→v2 提交历史）。

## 10. 变更文件（相对基线 `1efb9292`）

### 新增

- `src/app/state/compact.rs` + `src/app/state/compact/tests.rs`
- `src/app/update/compact_layout.rs` + `src/app/update/compact_layout/tests.rs`
- `src/ui/widgets/compact_header.rs` + `src/ui/widgets/compact_header/tests.rs`
- `src/ui/text/compact.rs`
- `src/event/tests/compact.rs`
- `docs/feature/compact-layout/{design.md,review.md,task.md}`

### 修改

- 布局与 UI 渲染：`src/ui/layout.rs`、`src/ui/mod.rs`、`src/ui/tab_bar.rs`、`src/ui/controls/{exit,footer,panels}.rs`、`src/ui/left_panel/{mod,history}.rs`、`src/ui/right_panel/mod.rs`、`src/ui/main_content/lx/history/cache.rs`、`src/ui/tooltip.rs`、`src/ui/text.rs`
- 状态与更新：`src/app/state.rs`、`src/app/update.rs`、`src/app/update/{editor,workspace,worktree}.rs`、`src/app/update/prompt/recent.rs`、`src/app/update/selection/mod.rs`
- 事件循环与输入：`src/event/mod.rs`、`src/event/session.rs`、`src/event/input.rs`、`src/event/mouse/{autoscroll,drag,hover,press,wheel}.rs`
- 配置与设置：`src/config/mod.rs`、`src/settings/service/handlers.rs`、`src/cli/{args,help,settings}.rs`、`assets/{web.html,settings.js}`
- 规范文档：`docs/standards/tui-design-requirements.md`
- 单元测试与测试夹具：`src/config/tests.rs`、`src/settings/service/logs/tests.rs`、`src/cli/tests/config_domains_tests.rs`、`src/app/update/selection/tests.rs`、`src/ui/{test_support,tests}.rs`、`src/ui/controls/tests.rs`、`src/ui/layout/tests.rs`、`src/ui/left_panel/tests.rs`、`src/ui/right_panel/{tests,todo/tests}.rs`、`src/ui/tab_bar/tests.rs`、`src/ui/text/tests.rs`、`src/ui/toast/tests.rs`、`src/ui/tooltip/tests.rs`、`src/event/tests.rs`、`src/event/tests/{test_support,agent_menu,focus_and_paste,lx_ask_task,lx_toast,lx_view,mouse_click,mouse_drag,overlay,prompt_editor,prompt_pages,prompt_recent,prompt_tree,terminal,todo,workspace_history}.rs`
- 集成测试：`tests/{popup_border,prompt_border_title,prompt_copy,prompt_editor,prompt_management,prompt_panel_interaction,prompt_short_ref,prompt_template_commands,terminal_cursor,terminal_wheel}.rs`

## 11. 测试覆盖

- 阈值：160 → compact，161 → desktop；忽略旧配置 `narrow_width`；CLI/API 不暴露且拒绝 `--narrow-width`；
- 几何：单行顶部栏、按钮位于两端、极窄 `< 11`/`< 9` 按钮与分隔符互不重叠、tab strip 位于中间、左/右全屏 body、右栏底边框全宽身份标签；
- 状态：进入快照三项并清拖拽（含 `drag_autoscroll`/`lx_selection_autoscroll`）、退出恢复（含 `prompt_left && panes_collapsed` 与折叠侧栏交回右栏）、`Toggle*` 紧凑忽略、`commit_worktree_open` 回 Chat、`focus_prompt` 提交 Todo 编辑、`has_visible_selection` 隔离隐藏容器选区；
- 按钮与控制：左/右 toggle、左栏保留 `Agents` 折叠按钮与 `[WEB]`、全屏左栏 `New agent` 创建返回 Chat 并滚入激活标签、tab 点击返回并切换、Esc 仅关左栏（历史模式下双次 Esc）；
- tab 条：紧凑模式无 `[EXIT]`、`[+]` 新建、溢出滚动、缩窄自动 reveal 激活标签；
- 命中与缓存：顶部栏间隙吞点击、隐藏外框控件无命中、`tooltip::target_at`/hover 禁用、隐藏右栏忽略 `drag_autoscroll`、桌面折叠主区不入 PTY 几何、全屏面板不误删 `lx` 缓存、隐藏 Todo 不写乱滚动；
- Toast：紧凑模式复用桌面浮框（锚点容器内居中 / 屏幕顶部 `y=1` 居中 / 模态打开时全局顶部 `y=1`）；
- 渲染：`[ws]`/`[panel]`、分隔符 `│`、tab、活动按钮实心高亮。

## 12. 手动验收

在 worktree 运行 `cargo run`，检查：

- 160x30 进入紧凑，161x30 恢复三栏；
- `[ws]` 打开/关闭全屏左栏，工作区/Agents（含 `[▼]`/`[▲]`）/历史列表与 `[WEB]` 可点；
- `[panel]` 打开/关闭全屏右栏，Prompt/Todo 切换正常、底边框完整；
- 中间 tab 切换、`[+]`、多 tab 滚动、右键菜单，缩窄后激活标签保持可见；
- 左栏 Esc 返回（历史模式双 Esc），右栏 Esc 不清空编辑；
- 复制终端内容 Toast 复用桌面全局带边框浮框（顶部居中/容器顶部），不在底部；
- 拉宽/缩窄后焦点与折叠状态正确恢复。
