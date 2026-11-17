# 窄终端紧凑布局（Compact Layout）— 执行任务清单

- **任务目录**：`docs/feature/compact-layout/`
- **设计文档**：`docs/feature/compact-layout/design.md`
- **状态**：边界已确认（Grill Me Q1–Q12），设计已审核；阶段 1–5 已实现，待用户手动验收
- **工程约束**：严格遵守 `AGENTS.md` 与 `docs/standards/*`；保持 `app/`（纯状态与更新）与 `ui/`（纯渲染）边界；生产代码禁止 `unwrap()` / `expect()`；用户可见文案统一收敛到 `src/ui/text`；只做最小必要修改，不为未来假设加抽象。

---

## 阶段 0：需求质询与设计（当前阶段）

- [x] 使用 `codegraph` 与 `codebase-memory-mcp` 核对当前布局、事件路由、右栏/侧栏几何与 herdr mobile 实现
- [x] **Grill Me Q1**：确认单列全屏形态，隐藏侧栏/标签栏/右栏
- [x] **Grill Me Q2**：确认复用 `config.narrow_width`（默认 80），`width <= narrow_width` 触发，不新增配置
- [x] **Grill Me Q3**：确认 `switch` 覆盖 Agents / Spaces / Tabs / Panels / Actions，Prompt/Todo 全屏
- [x] **Grill Me Q4**：确认保留 BSP 多窗格平铺，不自动合并
- [x] **Grill Me Q5**：确认 header 自适应 1/2 行，`height >= 16` 显示 agent 摘要
- [x] **Grill Me Q6**：确认 switcher 列全局 agents（跨工作区/标签/窗格）
- [x] **Grill Me Q7**：确认 v1 只做鼠标/触摸打开 `switch`，不新增全局快捷键
- [x] **Grill Me Q8**：确认全屏面板 1 行 `[back]`，焦点切到 Prompt/Todo，不劫持 `Esc`
- [x] **Grill Me Q9**：确认拉宽保留焦点/面板，屏幕由 `FocusedContainer` 推导
- [x] **Grill Me Q10**：确认不搬工作区历史、Web 设置、折叠/交换控件
- [x] **Grill Me Q11**：确认保持 `min_width = 40`
- [x] **Grill Me Q12**：确认紧凑模式全局 Toast 走底部单行横幅
- [x] 编写 `design.md` 与 `task.md`
- [x] **用户审核并确认设计文档**
- [x] 审核通过后按阶段 1 开始实现

---

## 阶段 1：布局模型与状态机

### 1.1 `src/ui/layout.rs`

- [x] `ViewLayout` 增加字段：
  - `pub compact_header: Rect`
  - `pub compact: bool`
- [x] 保留原 `compute(8 参数)` 签名，内部委托 `compute_with_panel(..., None)`
- [x] 新增 `compute_with_panel(9 参数)`；紧凑分支：
  - `let compact = area.width > 0 && area.width <= config.narrow_width;`
  - header 高度：面板 1 行；Chat 且 `height >= 16` 为 2 行；否则 1 行
  - Chat：`main_content = body`，`right_panel` 宽 0
  - 面板：`right_panel = body`，`main_content` 宽 0
  - `left_panel` 宽 0、`tab_bar` 为 0、`right_panel_left = false`
- [x] 桌面分支保持现有几何，补 `compact_header = 0`、`compact = false`
- [x] 新增常量 `COMPACT_HEADER_TALL_MIN_HEIGHT: u16 = 16`
- [x] 更新 `resolved_sidebar_width` 注释：桌面分支只在 `width > narrow_width` 进入
- [x] `src/ui/layout/tests.rs` 新增第 12.1 节 5 条测试
- [x] 确认 `ViewLayout` 构造点：`src/ui/left_panel/tests.rs` 使用 `..view_for(...)`，无需补字段

### 1.2 `src/app/state.rs` 与 `src/app/state/compact.rs`

- [x] `AppState` 新增：
  - `pub compact_restore: Option<CompactRestore>`
  - `pub compact_switcher_scroll_drag: Option<u16>`
- [x] `AppState` 唯一构造器初始化 `None`
- [x] 新增 `CompactRestore { prompt_collapsed, panes_collapsed }`
- [x] 新增 `AppState::compact_panel() -> Option<RightTab>`：焦点 Prompt/Todo 映射，其余 None
- [x] 新增 `src/app/state/compact.rs` 并在 `state.rs` 注册：
  - `CompactSection`
  - `CompactRow`
  - `GlobalAgentEntry`
  - `agent_entries_for_workspace`（把现有 `agent_entries` 逻辑搬入）
  - `global_agent_entries`
  - `compact_rows`
- [x] `src/app/state.rs::agent_entries()` 改为委托 `agent_entries_for_workspace(active_workspace)`
- [x] `src/app/state/compact/tests.rs`：
  - 全局 agents 跨工作区收集
  - `agent_entries` 仍只返回当前工作区
  - `compact_rows` 顺序与空 agents 时跳过 Agents 区

### 1.3 `src/app/update/compact_layout.rs`

- [x] `src/app/update.rs` 注册 `mod compact; pub use compact::*;`
- [x] `sync_compact_layout(state, width, height, config)`：
  - 进入：快照 `prompt_collapsed` / `panes_collapsed`，临时置 false；Sidebar 焦点转 Panes；`right_tab` 与焦点对齐
  - 退出：恢复快照；焦点在 Prompt/Todo 时保留展开；关闭 CompactSwitcher；清空滚动拖拽
  - 同步 switcher `viewport_rows`
- [x] `back_to_compact_chat(state)`：`prompt.clear_panel()` + `focus_panes`
- [x] `open_compact_switcher(state, viewport_rows)`
- [x] `close_compact_switcher(state)`
- [x] `move_compact_switcher(state, delta)`：跳过 Section
- [x] `page_compact_switcher(state, delta)`
- [x] `scroll_compact_switcher(state, delta)`
- [x] `set_compact_switcher_scroll(state, offset)`
- [x] `activate_compact_switcher(state)`：按第 8.6 节分派
- [x] `apply_compact_switcher_key(state, key)`：Esc / Up / Down / PageUp / PageDown / Enter
- [x] `Action::TogglePrompt` / `TogglePanes` 在 `compact_restore.is_some()` 时忽略
- [x] `src/app/update/compact_layout/tests.rs` 覆盖第 12.4 节 8 条

### 1.4 `src/event/session.rs`

- [x] `current_geometry` 改为 `state: &mut AppState`
- [x] 读取 size 后调用 `update::sync_compact_layout`
- [x] 使用 `ui::layout::compute_with_panel(..., state.compact_panel())`
- [x] `main_content` 为 0 时 pane rects 置空（PTY 不 resize）
- [x] 仅当 `!state.prompt_collapsed && view.right_panel.width > 0` 时加入 prompt rect
- [x] 更新 `event/mod.rs` 调用处（已是 `&mut state`，确认编译）
- [x] 更新 `src/event/tests/test_support.rs::geometry` 的 `compute` 调用（保持桌面测试；需要紧凑的测试用 `compute_with_panel`）

---

## 阶段 2：顶部状态条

### 2.1 文案

- [x] 新增 `src/ui/text/compact.rs`，在 `src/ui/text.rs` 注册并 `pub use`
- [x] 常量至少包含：
  - `COMPACT_SWITCH_LABEL = "[switch]"`
  - `COMPACT_BACK_LABEL = "[back]"`
  - `COMPACT_CLOSE_LABEL = "[close]"`
  - `COMPACT_TITLE = "switch"`
  - `COMPACT_SECTION_AGENTS = "Agents"`
  - `COMPACT_SECTION_SPACES = "Spaces"`
  - `COMPACT_SECTION_TABS = "Tabs"`
  - `COMPACT_SECTION_PANELS = "Panels"`
  - `COMPACT_SECTION_ACTIONS = "Actions"`
  - `COMPACT_NEW_WORKSPACE = "+ new workspace"`
  - `COMPACT_NEW_TAB = "+ new tab"`
  - `COMPACT_NO_AGENTS = "no agents"`
  - `COMPACT_SEPARATOR = " · "`
- [x] `src/ui/text/tests.rs` 增加单宽/非私有区字形检查（如现有文案测试机制适用）

### 2.2 `src/ui/widgets/compact_header.rs`

- [x] 在 `src/ui/widgets/mod.rs` 注册
- [x] `CompactHeaderLayout { area, switch, back, status, title }`
- [x] `layout(view, state)`：
  - Chat：`switch` 右对齐、宽 8；`status` 为剩余区
  - 面板：`back` 左对齐、宽 6；`title` 紧随
  - 高度 2 时按钮矩形覆盖两行
- [x] `render`：
  - Chat 第 1 行：聚合状态点 + 工作区名 + 右对齐 tab 状态
  - Chat 第 2 行（height >= 16）：blocked → working → idle 摘要
  - 面板：`[back]` + `Prompt`/`Todo` + 可选右侧 `workspace · tab`
  - `switch` 垂直居中；`switch.x - 1` 画 `│`
  - 所有文本按显示宽度截断
- [x] `switch_at` / `back_at` 命中
- [x] `src/ui/widgets/compact_header/tests.rs` 覆盖第 12.2 节 5 条

### 2.3 `src/ui/mod.rs` 接入

- [x] `render_cached` 改用 `compute_with_panel(..., state.compact_panel())`
- [x] `pane_rects` 在 `main_content` 宽或高为 0 时为空
- [x] `view.compact` 时先渲染 `widgets::compact_header::render`
- [x] `view.compact` 时跳过：
  - `controls::render_exit_button`
  - `controls::render_swap_button`
  - `controls::render_panel_buttons`
  - `controls::render_resize_hint`
  - `tooltip::render`
- [x] 紧凑 Chat 仍渲染主区窗格；紧凑面板仍渲染 `right_panel`
- [x] `render_min_size_notice` 保持优先（低于 `min_width` 不进入布局）

---

## 阶段 3：全屏切换器

### 3.1 浮层模型

- [x] `src/app/overlay.rs`：
  - `mod compact_switcher;`
  - `pub use compact_switcher::CompactSwitcher;`
  - `OverlayKind::CompactSwitcher`
  - `Overlay::CompactSwitcher(CompactSwitcher)`
  - `kind()` 分支
- [x] `src/app/overlay/compact_switcher.rs`：
  - `CompactSwitcher { rows, selected, scroll, hover, viewport_rows }`
  - `new` / `set_viewport` / `move_selection` / `page` / `ensure_visible` / `max_scroll` / `selectable_row`
- [x] `src/app/overlay/tests.rs` 补 `kind()` 与构造测试（如适用）

### 3.2 输入映射

- [x] `src/input/mod.rs::overlay_key` 增加 `OverlayKind::CompactSwitcher`：
  - Esc / Up / Down / Enter / PageUp / PageDown
- [x] `src/app/update/dialog.rs::apply_overlay_key` 最前面增加 CompactSwitcher 分支，调用 `apply_compact_switcher_key`
- [x] 确认 `Ctrl+Q` 仍在 route 最前，不被 switcher 吞掉

### 3.3 渲染与命中

- [x] `src/ui/overlay.rs` 注册 `compact_switcher` 并在 `render` 最前面增加分支
- [x] `src/ui/overlay/compact_switcher.rs`：
  - `CompactSwitcherLayout { area, close, viewport, scrollbar }`
  - `layout`
  - `render`
  - `row_at`
  - `close_at`
- [x] 渲染规则按 design 第 8.8 节：
  - 标题 ` switch` + `[close]`
  - 第 2 行 muted `─`
  - Section 标题
  - 高亮/悬停底色
  - Agent 状态点 + 名称/标题 + 右侧 workspace/tab/pane
  - Workspace/Tab/Panel/Action 行
  - 滚动条
- [x] `src/ui/overlay/compact_switcher/tests.rs` 覆盖第 12.3 节 6 条

### 3.4 鼠标

- [x] `src/event/mouse/press.rs`：
  - overlay 分支增加 switcher 滚动条 track/thumb
  - `handle_overlay_click` 增加 `[close]` / 行点击 / Section 忽略
  - 下降顺序保持 toast → overlay → header → 其他
- [x] `src/event/mouse/hover.rs`：switcher hover 行
- [x] `src/event/mouse/wheel.rs`：overlay 早退前处理 switcher 滚轮
- [x] `src/event/mouse/drag.rs`：switcher thumb 拖拽；松开清空
- [x] `compact_switcher_scroll_drag` 在关闭/退出紧凑时清空

---

## 阶段 4：事件与隐藏控件

### 4.1 Header 命中

- [x] `src/event/mouse/press.rs` 在 overlay 之后、exit/tab bar 之前：
  - 紧凑 header 内：
    - `switch_at` → `open_compact_switcher(state, screen.height.saturating_sub(2) as usize)`
    - `back_at` → `back_to_compact_chat`
    - 其他区域直接 return（吞掉）
- [x] 确认点击 header 不会穿透到主区窗格

### 4.2 隐藏控件闸门

- [x] `src/ui/controls/panels.rs::collapse_buttons`：`if view.compact { return Vec::new(); }`
- [x] `src/ui/controls/panels.rs::prompt_pin_at`：`if view.compact { return false; }`
- [x] `src/ui/controls/resize.rs::render_resize_hint`：`if view.compact { return; }`
- [x] `src/ui/tooltip.rs::render`：`if view.compact { return None; }`
- [x] 确认 `swap_button` / `exit_button` / `web_button` 在紧凑几何下自然返回 None
- [x] 端到端测试点击原折叠/钉住位置无反应

### 4.3 键盘与焦点

- [x] 紧凑 Chat：焦点 Panes，lx/终端按键不变
- [x] 紧凑全屏面板：焦点 Prompt/Todo，编辑器/Todo 按键不变
- [x] switcher 打开：浮层按键接管
- [x] `Esc` 在 Prompt 中仍归编辑器，不做“返回聊天”
- [x] 进入紧凑时 Sidebar 焦点转 Panes
- [x] 退出紧凑时焦点在 Prompt/Todo 保留展开

---

## 阶段 5：Toast、配置与文档

### 5.1 Toast

- [x] `src/ui/toast.rs::rect`：`view.compact` 时返回最后一行整宽矩形
- [x] `src/ui/toast.rs::render`：紧凑模式绘制单行横幅
- [x] 新增/复用底部横幅绘制函数：
  - ` ● <title> · <message>` 或 ` ● <message>`
  - 圆点/文字语义色与现有 toast 一致
  - 单行截断，不越界
- [x] 桌面 Toast 现有锚点逻辑不变
- [x] `pane.lx.toast` 不接入该分支
- [x] 测试：
  - 紧凑 rect 在最后一行
  - 紧凑渲染内容可见
  - 点击关闭
  - 桌面测试不回归

### 5.2 配置/设置页

- [x] `src/config/mod.rs`：`narrow_width` 注释改为“低于等于该宽度进入紧凑布局”
- [x] `assets/web.html`：`Narrow width` → `Narrow layout width`
- [x] `assets/settings.js` 的 `CONFIG_KEYS` 不变
- [x] 确认无需 DB 迁移

### 5.3 规范文档

- [x] `docs/standards/tui-design-requirements.md` 增加紧凑布局规则：
  - 触发条件与单列几何
  - header 1/2 行规则
  - `switch` 分区顺序
  - `[back]` 返回、不劫持 `Esc`
  - 底部 Toast
  - 隐藏控件无命中
  - 全屏面板 PTY 不 resize
- [x] 如实现中产生额外约束，先回写 `design.md` 再改代码

---

## 阶段 6：测试、校验与收尾

### 6.1 新增测试

- [x] `src/ui/layout/tests.rs`：第 12.1 节 5 条
- [x] `src/ui/widgets/compact_header/tests.rs`：第 12.2 节 5 条
- [x] `src/ui/overlay/compact_switcher/tests.rs`：第 12.3 节 6 条
- [x] `src/app/update/compact_layout/tests.rs`：第 12.4 节 8 条
- [x] `src/app/state/compact/tests.rs`：全局 agents / rows 顺序
- [x] `src/event/tests/compact.rs`：第 12.5 节 12 条
- [x] `src/ui/toast/tests.rs`：紧凑底部横幅 2 条

### 6.2 现有测试调整

- [x] `src/ui/layout/tests.rs` 现有窄屏测试改写：
  - `narrow_width_hides_sidebar_but_keeps_prompt`（60x30）旧断言“右栏仍 30 列”不再成立 → 改为 compact 断言
  - `collapsed_narrow_sidebar_is_hidden`（60x30）→ 改为 compact 断言
  - 如需保留桌面窄屏语义，宽度改为 81~100 或删除被新语义取代的断言
- [x] 全仓搜索 `TestBackend::new(80` / `Rect::new(0, 0, 80` / `layout::compute` 的 80 列用例
- [x] 桌面三栏断言改 100/120 列
- [x] 紧凑行为新增独立断言
- [x] 所有 `ViewLayout { ... }` 字面量补 `compact_header` / `compact`
- [x] `event/tests/test_support.rs::geometry` 按需改用 `compute_with_panel`
- [x] `focus_and_paste` / `workspace_history` 等 80 列测试逐条核对

### 6.3 精准校验

- [x] `cargo fmt`
- [x] `cargo test --lib`
- [x] `cargo clippy --all-targets -- -D warnings`
- [x] `cargo test`
- [x] 检查生产代码无 `unwrap()` / `expect()`
- [x] 检查用户可见文案无中文
- [x] 检查 `ui/` 未新增状态修改/IO

### 6.4 手动验收（用户执行 `cargo run`）

- [ ] 80x24：单列 + 2 行 header
- [ ] 60x24：单列 + 2 行 header
- [ ] 60x10：单列 + 1 行 header
- [ ] 40x12：单列 + 1 行 header，`[switch]` 可点
- [ ] 81x24 / 120x30：桌面三栏不回归
- [ ] `switch` 中跨工作区选 agent
- [ ] `switch` 中切 Space / Tab
- [ ] `+ new workspace` 弹出现有新建工作区模态
- [ ] `+ new tab` 立即新建
- [ ] Prompt 全屏编辑、`[back]` 返回
- [ ] Todo 全屏编辑、`[back]` 返回
- [ ] 全屏 Prompt 中拉宽 / 缩窄，焦点与面板保持
- [ ] 紧凑 Chat 复制终端选区，Toast 在底部
- [ ] 点击全屏面板底部原折叠按钮位置，无隐藏命中
- [ ] 回到桌面后原 `prompt_collapsed` / `panes_collapsed` 恢复

---

## 阶段 7：审核与合并

- [ ] 用户确认手动验收结果
- [ ] 检查 git diff 只包含本任务相关文件
- [ ] 如有设计偏差，同步更新 `design.md`
- [ ] 按项目流程提交；**未获授权不得自行 commit / merge / 改分支**

---

## 变更文件总览（实际）

### 新增

- `docs/feature/compact-layout/design.md`
- `docs/feature/compact-layout/task.md`
- `src/app/state/compact.rs`
- `src/app/state/compact/tests.rs`
- `src/app/update/compact_layout.rs`
- `src/app/update/compact_layout/tests.rs`
- `src/app/overlay/compact_switcher.rs`
- `src/ui/widgets/compact_header.rs`
- `src/ui/widgets/compact_header/tests.rs`
- `src/ui/overlay/compact_switcher.rs`
- `src/ui/overlay/compact_switcher/tests.rs`
- `src/ui/text/compact.rs`
- `src/event/tests/compact.rs`

### 修改

- `src/ui/layout.rs`
- `src/ui/layout/tests.rs`
- `src/ui/mod.rs`
- `src/ui/controls/panels.rs`
- `src/ui/tooltip.rs`
- `src/ui/toast.rs`
- `src/ui/toast/tests.rs`
- `src/app/state.rs`
- `src/app/update.rs`
- `src/app/update/dialog.rs`
- `src/app/overlay.rs`
- `src/input/mod.rs`
- `src/event/session.rs`
- `src/event/mouse/press.rs`
- `src/event/mouse/hover.rs`
- `src/event/mouse/wheel.rs`
- `src/event/mouse/drag.rs`
- `src/event/tests.rs`
- `src/ui/text.rs`
- `src/ui/text/tests.rs`
- `src/ui/widgets/mod.rs`
- `src/ui/overlay.rs`
- `assets/web.html`
- `src/config/mod.rs`
- `docs/standards/tui-design-requirements.md`

---

## 风险提醒（执行前必看）

1. **`prompt_collapsed` 是全局状态**：紧凑模式进入时临时置 false，退出时按快照恢复；任何新增的 `!state.prompt_collapsed` 判断都要确认在紧凑全屏面板下行为正确。
2. **隐藏控件命中**：`collapse_buttons` / `prompt_pin_at` / `render_resize_hint` / tooltip 必须加 `view.compact` 闸门，否则会出现“看不见却能点”。
3. **80 列测试**：默认 `narrow_width = 80` 且条件为 `<=`，所有 80 列桌面断言都会变成紧凑布局，必须逐条改为 100/120 或改写为 compact 断言。
4. **PTY 尺寸**：全屏面板时 `geometry.rects` 不包含窗格，不能让 `resize_panes` 把 PTY 缩成 0；回 Chat 后必须恢复。
5. **switcher 快照**：rows 在打开时生成；激活前必须校验工作区/tab/pane 仍存在，避免半套切换。
6. **文案**：所有新增用户可见文案用英文，集中到 `src/ui/text/compact.rs`；禁止在 header/switcher 中硬编码字符串。
7. **不改 Git 状态**：未获授权不得 commit / merge / 切分支；文档审核通过前不得改生产代码。
