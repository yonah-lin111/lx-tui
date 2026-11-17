# 窄终端紧凑布局（Compact Layout）— 代码审查（Review）

> 审查范围：`1efb9292..HEAD`（共 7 个提交：`396add67`、`66bc6345`、`4615dbb7`、`4ed1abbd`、`246b0b8b`、`02da544b`、`615ab627`），并对照 `dev`（`d5bf1251`）合入后的增量漂移
> 工作区：`.worktrees/compact-layout`（分支 `feature/compact-layout`）
> 依据规范：`AGENTS.md`、`docs/standards/code-writing-standards.md`、`docs/standards/project-directory-structure.md`、`docs/standards/tui-design-requirements.md`
> 状态：**分支内 P0/P1/P2/P3-1 已修复并通过全量验证（5832 lib + 63 集成测试）；`P0-2` 留待合并 `dev` 时按清单对齐、`P3-2` 记录为既有超限债务；待用户 `cargo run` 手动验收**

---

## 0. 范围与方法

- **覆盖链路**：
  1. **UI 几何与渲染**：阈值（160/161）、单行顶部栏 `[ws] │ tabs │ [panel]`、中间 `tab_bar`、`Agents` 分区按钮、全屏左/右栏、`Toast`/`Tooltip`/硬件光标、`min_width` 极窄边界；
  2. **状态机与键鼠路由**：启动首帧同步、进入/退出紧凑折叠快照恢复、`has_visible_selection`、`Esc` 与键盘隔离、跨阈值拖拽/悬停清理、PTY 尺寸同步与渲染缓存；
  3. **配置、测试、规范与 `dev` 漂移**：`narrow_width` 持久化移除（`Config` / `settings` HTTP API / CLI / Web UI）、文件行数上限与依赖边界、与 `dev@d5bf1251` 的合入兼容性、`design.md`/`task.md`/`tui-design-requirements.md` 一致性。
- **审查方法**：委派 3 个只读子任务并行深入源码与测试文件逐行核实，主会话复核全部涉及函数与调用链，仅记录经源码证实的问题。

---

## 1. 审查结论概览

| 级别 | 数量 | 说明 |
|---|---|---|
| P0 | 2 | 1 项分支内严重运行态回归（桌面折叠主区 PTY 缩成 2 列）+ 1 项合入 `dev` 必现测试阻断 |
| P1 | 5 | 退出恢复焦点困死/漏 `!view.compact` 保护、隐藏选区吞 `Esc`、全屏左栏丢失 `Agents` 折叠按钮、`Tooltip` 未闸门、规范文档 80 列冲突 |
| P2 | 6 | 全屏缓存误清空/缩放未 reveal、`Worktree`/`Todo`/跨阈值拖拽清理、隐藏 Todo 污染滚屏、右栏底边框空白预留、极窄按钮重叠、文档与测试缺口 |
| P3 | 2 | `compact_header` 硬编码 `"│"` 分隔符、既有超限文件债务记录 |

---

## 2. P0 — 严重运行态缺陷与合并阻断

### P0-1：`session::current_geometry` 丢失桌面 `prompt_left && panes_collapsed` 保护，主区折叠时 PTY 被缩成 2 列

- **涉及位置**：`src/event/session.rs:70-81`（对照 `src/ui/mod.rs:106-111`、`src/ui/layout.rs:93-97`、`src/event/tests/test_support.rs:39-48`）
- **关键点**：
  - 本分支将 `src/event/session.rs:72` 原有的 `if state.prompt_left && state.panes_collapsed` 改为 `if view.main_content.width == 0 || view.main_content.height == 0`。
  - 桌面模式在 `prompt_left == true && panes_collapsed == true` 时，`ui::layout::compute_with_panel` 为 `view.main_content` 分配的宽度是 `COLLAPSED_STRIP`（`4` 列）而非 `0`。
- **错误原因**：
  1. 用 `view.main_content.width == 0` 替代桌面折叠条件后，桌面主区折叠态仍满足 `width == 4 > 0`，导致 `geometry.rects` 把 4 列折叠窄条算作有效窗格矩形。
  2. 随后事件循环调用 `update::resize_panes` 与 `session::resize_sessions`，直接将所有终端仿真器及后台 PTY resize 成 **2 列宽**（`4 - 2` 边框），且鼠标左/右键会误命中已折叠的窄条窗格；原测试 `mouse_click::mouse_down_on_panes_collapse_button_collapses_panes` 因测试夹具 `test_support.rs:39` 仍保留旧判断而未能暴露该问题。
- **修复方案**：在 `src/event/session.rs:72`（并同步 `test_support.rs:39`）恢复桌面主区折叠判断：`if view.main_content.width == 0 || view.main_content.height == 0 || (!view.compact && state.prompt_left && state.panes_collapsed)`，并补齐回归测试。

### P0-2：合入 `dev`（`d5bf1251`）后，`dev` 新增及改写的大量桌面事件测试将在 80–120 列下误入紧凑模式并失败（合入阻断）

- **涉及位置**：
  - 文本冲突：`src/event/tests/overlay.rs:155-189`（`HEAD` 切换 `desktop_config()`，`dev` 改名为 `mouse_right_click_agents_blank_opens_new_agent_menu` 且仍用 `Config::default()` @ `120x30`）
  - 语义漂移（`dev` 在 `1efb9292..d5bf1251` 新增）：
    - `src/event/tests/agent_menu.rs`（`80x24` / `100x24` / `120x30` + `Config::default()`）
    - `src/event/tests/mouse_drag.rs`（`120x20` / `120x24` / `120x30` + `Config::default()`）
    - `src/event/tests/workspace_history.rs:826`（`120x30` + `Config::default()`）
    - `src/app/state.rs`（两处 `mod` / `pub use` 小范围合并冲突）
- **关键点与错误原因**：`feature/compact-layout` 将运行期 `Config::default().narrow_width` 从 `80` 提高到 `160`，而 `dev` 上合入的 `agents-menu-drag-scroll` 新增的桌面三栏测试基于旧阈值 `80` 并直接调用 `Config::default()`。合入 `dev` 后这些 `80~120` 列用例会触发紧凑布局（`left_panel.width == 0`、`right_panel.width == 0`），导致 `sidebar_sections(...).expect(...)` 等断言 Panic 或失败。
- **处理边界**：按项目规范当前不得修改 `dev` 或提前执行合并；本项在 `review.md` 与 `task.md` 标为合并时必做动作（合入/rebase `dev` 后将上述桌面用例统一切为 `desktop_config()` 并复核 `enter_in_new_agent_reveals_newly_created_tab_when_tab_bar_overflows`）。

---

## 3. P1 — 重要状态机、交互、控件与规范缺陷

### P1-1：`sync_compact_layout` 退出无条件恢复 `panes_collapsed` 导致焦点困死，且 3 处渲染分支缺失 `!view.compact` 保护

- **涉及位置**：`src/app/update/compact_layout.rs:31-46`、`src/ui/mod.rs:106`、`src/ui/right_panel/mod.rs:85`、`src/ui/controls/footer.rs:15`
- **关键点**：
  1. 退出紧凑分支中，`Prompt`/`Todo` 聚焦时会保留 `prompt_collapsed = false`，但第 41 行无条件执行 `state.panes_collapsed = restore.panes_collapsed`，且第 43-45 行在 `sidebar_collapsed && sidebar_focused()` 时无条件调用 `focus_panes(state)`。
  2. `ui/mod.rs:106`（`state.prompt_left && state.panes_collapsed`）、`ui/right_panel/mod.rs:85`（`state.prompt_collapsed`）与 `ui/controls/footer.rs:15`（`state.sidebar_collapsed`）均漏掉 `&& !view.compact` 保护（仅 `ui/mod.rs:90` 加了）。
- **错误原因**：
  1. 若用户在桌面端处于 `prompt_left && panes_collapsed`（主区折叠为 4 列窄条），缩窄进入紧凑后切换回 Chat（`focused == Panes`）再拉宽回桌面，主区会重新折叠为不可见窄条而焦点留在 `FocusedContainer::Panes`，导致键盘继续打入隐藏窗格。
  2. 若桌面端同时折叠了侧栏与右列主区（`sidebar_collapsed && panes_collapsed && prompt_left`），在紧凑 `[ws]` 打开状态下直接拉宽，第 43-45 行会将焦点从折叠侧栏交到同样折叠的 `Panes`，而不是唯一展开的 `RightTab`。
  3. 渲染层缺少 `!view.compact` 时，直接调用 `ui::render` 或帧间未同步时，在紧凑模式下仍会把 Chat/右栏绘成 1 列窄条并隐藏 `[WEB]` 按钮。
- **修复方案**：退出分支在 `state.prompt_left && state.panes_focused()` 时保持 `panes_collapsed = false`；在 `sidebar_collapsed && sidebar_focused()` 且 `prompt_left && panes_collapsed` 时将焦点交给展开的 `right_tab`（否则确保主区展开）；三处渲染分支统一补齐 `!view.compact`。

### P1-2：`has_visible_selection` 未按紧凑可见容器隔离，隐藏面板/窗格残留选区会吞掉当前视图的 `Esc`

- **涉及位置**：`src/app/update/selection.rs:471-566`、`src/app/update/workspace.rs:235-244`、`src/app/update/prompt/recent.rs:140-149, 191`、`src/app/update/compact_layout.rs:61-64, 76-82`
- **关键点**：`has_visible_selection` 通过 `!state.sidebar_collapsed`、`!state.prompt_collapsed`、`!state.panes_collapsed` 判断三栏选区是否可见；同时 `open_prompt_for_pane`、`open_prompt_page` 与 `back_to_compact_chat` 未统一清选区。
- **错误原因**：
  1. 进入紧凑模式后 `sync_compact_layout` 会把三个 `*_collapsed` 布尔值全部临时置为 `false`，但同一时刻只有 `state.compact_panel()` 对应的那一个容器可见。`has_visible_selection` 忽略 `compact_restore` / `compact_panel()`，把另外两个不可见容器内的旧选区也算作“当前可见选区”。
  2. 例如在紧凑 Chat 窗格拖选文本后右键菜单按回车 `Open prompt`（或由 `open_prompt_page` 切入 Prompt，其中 140-149 行未清、191 行漏清 `lx_selection`/`terminal_selection`），不可见窗格内的选区仍留在状态中；随后用户在 Prompt 按 `Esc` 关闭 `@`/`/` 面板、或切去 `[ws]` 按 `Esc` 返回 Chat 时，第一次 `Esc` 被 `escape_clear_selection` 消费且界面无视觉反馈。
- **修复方案**：在 `has_visible_selection` 内判断 `state.compact_restore.is_some()` 时按 `state.compact_panel()` 严格隔离三栏可见性；并在 `open_prompt_for_pane`、`open_prompt_page`、`back_to_compact_chat` 与 `toggle_compact_right` 切换时补齐 `clear_selection(state)`。

### P1-3：紧凑全屏左栏下的 `Agents` 分区折叠按钮 `[▼]`/`[▲]` 不绘制且无法点击

- **涉及位置**：`src/ui/controls/panels.rs:34-54, 144-175`、`src/ui/mod.rs:115-119`、`src/ui/left_panel/mod.rs:80-85, 493`
- **关键点**：`controls::collapse_buttons` 在 `view.compact` 时直接返回空向量，且 `ui/mod.rs:117` 用 `if !view.compact` 跳过了整组 `render_panel_buttons`。
- **错误原因**：把左栏内部分区的 `CollapseTarget::Agents` 与桌面三栏外框折叠按钮（`Sidebar` / `Prompt` / `Panes`）一并关闭。而 `left_panel::render` 仍为 `agents_button_area` 避让标题且按 `state.agents_collapsed` 收起 `Agents` 列表，若用户在桌面端折叠 `Agents` 后缩窄进 `[ws]`，全屏左栏中将无法点击展开 `Agents`，违背 `design.md`“Agents 分区交互全部保留”的约定。
- **修复方案**：`collapse_buttons` 仅对 `Sidebar`/`Prompt`/`Panes` 应用 `!view.compact`，保留 `agents_button`（其内部已要求 `view.left_panel.width > COLLAPSED_STRIP`，因此 Chat 与全屏右栏下仍为 `None`）；`ui/mod.rs` 保持调用 `render_panel_buttons` 并将 `render_prompt_pin` 门控在 `!view.compact`。

### P1-4：紧凑模式下 `ui::tooltip::target_at` 与 `hover.rs` 仍命中按钮并调度 500ms 定时重绘

- **涉及位置**：`src/ui/tooltip.rs:20-31, 82-108`、`src/event/mouse/hover.rs:130-137, 355-367`
- **关键点**：仅 `tooltip::render` 与 `ui::mod` 加了 `!view.compact` 门控，而 `tooltip::target_at` 与 `hover.rs`（普通悬停及 `Overlay::ReferencedFolderSearch` 分支）未加拦截。
- **错误原因**：紧凑模式下鼠标经过顶部栏按钮或左右栏按钮时，`hover.rs` 仍写入 `state.tooltip`（含不可见的 `TooltipTarget::PromptPin`），触发 `dirty = true` 与 500ms 定时器无意义唤醒，且若悬停期间拉宽窗口会瞬间闪现残留 tooltip，违反 `tui-design-requirements.md`“紧凑模式 tooltip 一律失效”的要求。
- **修复方案**：在 `ui::tooltip::target_at` 与 `targets` 入口加入 `if view.compact` 早退，并在 `hover.rs:130, 355` 同步加入 `!view.compact` 门控。

### P1-5：`docs/standards/tui-design-requirements.md` 第 103 行仍保留旧的 80 列降级规则

- **涉及位置**：`docs/standards/tui-design-requirements.md:103`（对照第 174-182 行）
- **关键点与错误原因**：`## 尺寸与降级` 第 103 行仍写“宽度不足 80 列时隐藏左栏（右栏保留）”，与第 176 行“宽度 `<= 160` 进入单列紧凑布局”直接冲突。
- **修复方案**：同步更新第 103 行描述，统一指向 `160` 列紧凑布局与 `161+` 桌面折叠规则。

---

## 4. P2 — 缓存与滚动同步、跨阈值清理、极窄边界与文档/测试缺口

### P2-1：全屏面板下 `ui_cache.retain_panes` 误清隐藏 `lx` 窗格缓存，且全屏左栏/跨阈值缩放跳过滚动钳制与激活标签露出

- **涉及位置**：`src/event/mod.rs:279, 474-492`、`src/event/session.rs:221-228`
- **关键点与错误原因**：
  1. `ui_cache.retain_panes` 直接传入可见的 `geometry.rects`：切换到 `[ws]`/`[panel]` 时 `rects` 中不含 `main_content` 窗格，导致返回 Chat 时所有 `lx` 窗格的 Markdown 布局缓存被销毁重建。
  2. `clamp_workspace_scroll` 与 `tab_scroll` 钳制放在 `if geometry.rects != last_rects` 内部：全屏左栏期间 `rects` 恒为 `[]`，缩窄或改高不会触发钳制；且桌面缩窄进紧凑导致激活标签被挤出右侧时，仅做了 `set_tab_scroll` 上限钳制而未调用 `session::reveal_active_tab`，激活标签在紧凑顶栏右侧不可见。
- **修复方案**：`ui_cache.retain_panes` 改用 `state.all_pane_ids()` 仅回收已关闭窗格；在 `event/mod.rs` 同步跟踪 `last_screen` 与 `last_compact`，当屏幕尺寸或紧凑态变化时额外钳制 `workspace_scroll` 并调用 `session::reveal_active_tab`（不影响用户平时点击 `[<]`/`[>]` 滚动）。

### P2-2：跨 160/161 阈值或动作切面板时未清理进行中的拖拽/悬停与 Todo 编辑态

- **涉及位置**：`src/app/update/compact_layout.rs:18-48`、`src/event/mouse/drag.rs:369-380`、`src/app/update/worktree.rs:310-314`、`src/app/update/editor.rs:681-686`
- **关键点与错误原因**：
  1. 桌面拖拽侧栏/右栏分割线（`resizing_sidebar` / `resizing_prompt`）或未聚焦面板滚动条期间缩窄进紧凑，`drag.rs:369-380` 未门控 `!view.compact`，直接把 0 宽紧凑矩形喂给 `prompt_width_at`/`sidebar_width_at` 写坏保存的栏宽（如把 `prompt_width` 覆写成 `min_pane_width`），并残留 `EwResize` 指针；紧凑全屏左栏拖拽排序期间拉宽且恢复折叠时，旧拖拽态也会滞留直到 `MouseUp`。
  2. `commit_worktree_open` 打开**已在活跃列表中的** worktree 时仅调 `switch_workspace`，不会把 `Sidebar` 焦点交回 `Panes`，导致紧凑 `[ws]` 选完后停在左栏（与同文件新建 worktree 及历史列表行为不一致）。
  3. `focus_prompt` 直接覆写 `state.right_tab = RightTab::Prompt`，漏掉 `commit_todo_edit` 与 Todo 悬停/拖拽状态清理。
- **修复方案**：`sync_compact_layout` 在进入/退出紧凑态切换时统一清空拖拽/悬停/tooltip（事件循环同步复位鼠标指针形状），并在 `drag.rs:369-380` 补 `!view.compact`；`commit_worktree_open` 已打开分支补齐 `focus_panes(state)`；`focus_prompt` 补齐 `commit_todo_edit(state)` 与 Todo 瞬态清理。

### P2-3：右栏不可见时每帧仍以 1 列宽度重算 Todo 视口，污染 `state.todo.scroll`

- **涉及位置**：`src/event/mod.rs:526`、`src/app/update/todo.rs:640-650`、`src/ui/right_panel/todo.rs:218-225, 263-309`
- **关键点与错误原因**：`event/mod.rs:526` 每帧无条件用 `geometry.view.right_panel` 调用 `resolve_todo_viewport`。紧凑 Chat/Left 下 `right_panel.width == 0`，`todo.rs:219` 通过 `.max(1)` 按 **1 列宽**折行并写回 3 行高与偏大的 `state.todo.scroll`，切回 `[panel]` Todo 后列表顶行会被异常滚出。
- **修复方案**：仅在右栏真正展开且当前页为 `RightTab::Todo`（`right_panel.width > COLLAPSED_STRIP && right_panel.height > 2 && (!state.prompt_collapsed || view.compact) && state.right_tab == RightTab::Todo`）时调用 `resolve_todo_viewport`。

### P2-4：紧凑全屏右栏底边框仍为隐藏按钮预留 10–12 列空白，过早截断工作区身份标签

- **涉及位置**：`src/ui/right_panel/mod.rs:106, 152-168`
- **关键点与错误原因**：`right_panel::render_bottom` 未感知 `view.compact`，在紧凑全屏右栏下仍为已隐藏的 `[pin]` 与 `[▶]` 扣减右端 10–12 列，导致窄屏底边框 `项目 · b:分支 · wt:工作区` 过早截断。
- **修复方案**：向 `render_bottom` 传入 `view.compact`，紧凑模式右界直接取 `area.right().saturating_sub(2)`。

### P2-5：极窄宽度（`< 11` / `< 9` 列）下 `compact_header` 按钮/分隔符与 `tab_bar` 溢出按钮重叠

- **涉及位置**：`src/ui/layout.rs:152-175`、`src/ui/widgets/compact_header.rs:60-79`、`src/ui/tab_bar.rs:100-135`
- **关键点与错误原因**：`Config` 允许 `min_width < 11`。宽度 `< 11` 时 `left_button` 与 `right_button` 重叠；宽度 `11` 时 `tab_bar.width == 0` 但 `tab_bar.x > left_button.right()` 仍成立，致使 `compact_header` 在 `[panel]` 首列覆写 `"│"`；`tab_bar::layout` 在 `available < 9` 溢出时 `scroll_left` 与 `add` 矩形重叠。
- **修复方案**：`compact_layout` 按 `area.width - left_width` 夹紧 `right_width`；`compact_header` 仅在 `view.tab_bar.width > 0` 时画分隔符；`tab_bar::layout` 溢出分支按可用宽度顺序分配 `scroll_right` → `add` → `scroll_left`，不足时省略而非重叠。

### P2-6：`design.md` / `task.md` 文件清单与状态漂移，及测试与注释缺口

- **涉及位置**：
  - `docs/feature/compact-layout/design.md:122-125, 197-214`：列出不存在的 `src/app/update/input.rs`、7 个已回滚为净空 diff 的文件、漏列 3 个真实 PTY 集成测试与多个单测文件，且未说明 `│` 分隔符与 `Agents` 折叠按钮保留规则；
  - `docs/feature/compact-layout/task.md:54`：仍勾选已删掉的 `assets/web.html` 文案 `Narrow layout width`；
  - `src/ui/test_support.rs:8`、`src/event/tests/test_support.rs:19`、`tests/{prompt_editor,prompt_short_ref,terminal_cursor}.rs` 仍残留“默认 120 列”注释；
  - `src/ui/widgets/compact_header/tests.rs:99` 残留已删除状态行的测试命名 `long_workspace_name_does_not_push_buttons_off_screen`；
  - `src/settings/service/logs/tests.rs:154` 仍发送已废弃 `"narrow_width":80`，`logs/tests.rs` 与 `src/cli/tests/config_domains_tests.rs` 缺少对 `narrow_width` 被忽略/CLI `--narrow-width` 被拒绝的回归断言；
  - `compact.rs`、`toast/tests.rs`、`compact_header/tests.rs`、`compact_layout/tests.rs` 缺少多项边界测试（如工作区历史模式下双次 `Esc`、右栏 `Prompt`/`Todo` 切换、`Overlay` 期间紧凑全局 Toast 等）。
- **修复方案**：同步修正文档、过时注释与测试命名，补全配置忽略/拒绝及上述紧凑布局回归测试。

---

## 5. P3 — 轻微文案规范与既有超限债务

### P3-1：`compact_header.rs` 硬编码 `"│"` 分隔符

- **涉及位置**：`src/ui/widgets/compact_header.rs:65, 75`
- **关键点与修复**：复用 `crate::ui::text::TAB_SEPARATOR`（`src/ui/text/workspace.rs:253`），避免在组件内直接写死字面量。

### P3-2：既有文件行数超限债务记录

- **涉及位置**：`src/ui/text.rs`（1080 行，`1efb9292` 基线 1078 行，本分支只因引入 `pub mod compact;` 增加 2 行）、`src/ui/tests.rs`（3222 行）、`src/event/mod.rs`（990 行，接近 1000 行上限）。
- **处理说明**：本分支紧凑新增文案已独立收敛在 `src/ui/text/compact.rs`，未把逻辑写进 `text.rs` 或 `ui/tests.rs`；为避免与 `dev` 产生无谓大面积冲突，此处记录历史债务，待合并后按专项拆分子模块。

---

## 6. 修复与验证记录

| 级别 | 编号 | 修复内容 | 状态 | 关键回归测试 |
|---|---|---|---|---|
| P0 | P0-1 | `session::current_geometry` 与测试夹具恢复 `(!view.compact && state.prompt_left && state.panes_collapsed)` | 已修复并验证 | `desktop_collapsed_panes_exclude_panes_from_geometry_rects` |
| P0 | P0-2 | 记录合入 `dev@d5bf1251` 时 `agent_menu`/`mouse_drag`/`overlay`/`workspace_history` 测试切 `desktop_config()` 与 `state.rs` 冲突清单 | 合并前清单（不擅改 `dev`） | — |
| P1 | P1-1 | `sync_compact_layout` 退出保留主动交互区展开及安全交还焦点；`ui/mod.rs`、`right_panel/mod.rs`、`footer.rs` 补 `!view.compact` | 已修复并验证 | `sync_exit_keeps_panes_expanded_when_prompt_left_and_panes_focused`、`sync_exit_from_collapsed_sidebar_moves_focus_to_right_when_panes_collapsed` |
| P1 | P1-2 | `has_visible_selection` 按紧凑唯一可见容器隔离；切面板与打开页/Prompt 统一 `clear_selection` | 已修复并验证 | `has_visible_selection_isolates_hidden_containers_in_compact_mode`、`hidden_chat_selection_does_not_swallow_escape_in_fullscreen_left_panel` |
| P1 | P1-3 | `controls::collapse_buttons` 与 `ui/mod.rs` 在全屏左栏保留 `Agents` 分区 `[▼]`/`[▲]` 折叠按钮 | 已修复并验证 | `compact_fullscreen_left_panel_keeps_agents_toggle_and_web_button`、`fullscreen_left_panel_keeps_agents_toggle_clickable` |
| P1 | P1-4 | `ui::tooltip::target_at` / `targets` 与 `hover.rs` 完整接入 `!view.compact` 门控 | 已修复并验证 | `target_at_returns_none_in_compact_mode`、`compact_hover_never_arms_tooltip_or_divider_hover` |
| P1 | P1-5 | 更新 `docs/standards/tui-design-requirements.md:103` 移除旧 80 列冲突规则 | 已修复并验证 | — |
| P2 | P2-1 | `session::sync_layout_geometry` 按 `state.all_pane_ids()` 保留隐藏 `lx` 缓存，尺寸/模式切换时钳制工作区滚动、reveal 激活标签并复位指针形状 | 已修复并验证 | `sync_layout_geometry_preserves_lx_cache_and_reveals_active_tab_on_narrow_resize` |
| P2-2 | P2-2 | 跨 160/161 阈值清理在途拖拽/悬停与 `drag.rs` `!view.compact`；`commit_worktree_open` 已打开项回 Chat；`focus_prompt` 提交 Todo 编辑 | 已修复并验证 | `compact_transition_clears_active_resizing_drag_and_hover`、`open_existing_worktree_from_compact_left_panel_returns_to_chat`、`focus_prompt_commits_in_progress_todo_edit` |
| P2-3 | P2-3 | `event/mod.rs` 仅在右栏展开且为 `RightTab::Todo` 时调用 `resolve_todo_viewport` | 已修复并验证 | —（事件循环条件门控复核） |
| P2-4 | P2-4 | `right_panel::render_bottom` 紧凑模式下取消隐藏按钮右侧预留空白 | 已修复并验证 | `compact_fullscreen_right_panel_uses_full_bottom_border_for_identity` |
| P2-5 | P2-5 | `compact_layout`、`compact_header` 与 `tab_bar::layout` 极窄宽度防重叠保护 | 已修复并验证 | `ultra_narrow_compact_header_buttons_never_overlap`、`ultra_narrow_header_without_tab_space_skips_separators`、`overflow_in_very_narrow_tab_bar_never_overlaps_controls` |
| P2 | P2-6 | 同步校准 `design.md`、`task.md`、过期注释与 CLI/Settings/Compact 回归测试 | 已修复并验证 | `history_mode_in_fullscreen_sidebar_takes_double_escape_to_return_chat`、`fullscreen_right_panel_switches_prompt_and_todo_tabs`、`compact_fullscreen_right_panel_anchors_prompt_toast_inside_panel_and_modal_uses_top_center`、`settings::service::logs`、`cli::tests::config_domains_tests` |
| P3 | P3-1 | `compact_header.rs` 改用 `text::TAB_SEPARATOR` | 已修复并验证 | `long_tab_titles_do_not_push_buttons_off_screen` |
| P3 | P3-2 | 记录 `src/ui/text.rs`（1080 行）、`src/ui/tests.rs`（3222 行）既有超限债务 | 保留（既有历史债） | — |

### 最终验证结果

```bash
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
```

- `cargo fmt --check`：通过
- `cargo clippy --all-targets -- -D warnings`：无警告通过
- `cargo test`：
  - 库测试（`src/lib.rs`）：**5832 passed; 0 failed; 6 ignored**
  - 集成测试：**63 passed; 0 failed; 2 ignored**（覆盖 `cursor_steady`、`lx_paste`、`popup_border`、`prompt_border_title`、`prompt_copy`、`prompt_editor`、`prompt_management`、`prompt_panel_interaction`、`prompt_short_ref`、`prompt_template_commands`、`terminal_cursor`、`terminal_wheel`、`trust_prompt`、`web_assets`、`btw_verify`）
