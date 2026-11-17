# Agents 菜单、拖拽滚动、New agent 搜索与设置页修复 — 代码审查（Review）

> 审查范围：`dev..HEAD`（`1efb9292..150de2f2`，共 9 个提交：`df1e1ec1`、`87c9fbcb`、`e5f8823b`、`6fabb6a9`、`e8742e7c`、`a30c6a1e`、`65fdc1f3`、`8435fb78`、`150de2f2`）
> 工作区：`.worktrees/agents-menu-drag-scroll`（分支 `feature/agents-menu-drag-scroll`）
> 依据规范：`docs/standards/code-writing-standards.md`、`docs/standards/project-directory-structure.md`、`docs/standards/tui-design-requirements.md`、`docs/standards/html-assets.md`

---

## 审查结论概览

本分支已实现的主要能力中，P1 终端 Agent `Close` 主路径、P4 设置页 `element(` → `domEl(` 替换（4 个 JS 文件共 64 处）以及 PTY 异步启动队列基本正确；但在后续追加的 **New agent 搜索**、**列表拖拽与 `lx` 选区边缘自动滚动**以及部分边界耦合路径上存在 **3 项 P0 级运行态缺陷、5 项 P1 级交互/边界缺陷、4 项 P2 级守卫/规范/文档偏差**。

下表按 **P0 → P1 → P2** 排序列出全部已核实问题、关键点、错误原因与修复方案，后续按此清单逐项修复并重新验证。

---

## P0 — 严重运行态缺陷（死循环空转、选区破坏、核心输入契约缺失）

### P0-1：`scroll_lx_history` 贴底判定错误导致 `lx` 历史向下自动滚动 33Hz 死循环

- **涉及位置**：`src/app/update/chat/viewport.rs:25-39`、`src/app/update/selection.rs:763-805`
- **关键点**：
  - `scroll_lx_history` 第 35 行计算视口是否移动：`let moved = next != current || target.lx.chat.follow_bottom;`。
  - 仅以 `target.lx.chat.messages.is_empty()` 判断无历史，未覆盖仅含 `notices` / `compact` / `model_switch` / `recap` 的情况。
- **错误原因**：
  1. `LxChat::new()` 默认 `follow_bottom: true`。当视口已贴底（或内容不足一屏 `total <= visible`）时，向下滚动（`delta > 0`）使 `current == max_top`、`next == max_top`，且 `follow_bottom` 仍保持 `true`；但由于表达式写了 `|| target.lx.chat.follow_bottom`（而非比较 `follow_bottom != next_follow`），函数恒返回 `true`。
  2. `tick_lx_selection_autoscroll` 依赖 `lx_autoscroll_step` 返回 `false` 来识别到达边界并停止；此处永远返回 `true`，导致 `state.lx_selection_autoscroll` 永不清空，每 30ms 重设 `next_at` 并置 `dirty = true`，事件循环以约 33Hz 无限空转重绘（`wheel_lx_selection` 在底部向下滚轮也会误判已移动）。
  3. 与同文件中 `set_lx_history_scroll`（`was_following != target.lx.chat.follow_bottom`）及 `scroll_agents_detail` / `scroll_btw_detail`（`run.follow != next_follow`）写法不一致；原单元测试 `history_scroll_clamps_and_tracks_follow_bottom` 仅在 `follow_bottom = true` 初始态下测试过一次 `+999` 掩盖了该问题。
  4. `HistorySource::has_history` 允许无 `messages` 时渲染通知/检查点并生成 `content_geom`，但 `scroll_lx_history` 在 `messages.is_empty()` 时直接返回 `cancelled`，导致此类历史无法滚动。
- **修复方案**：
  - 将 `scroll_lx_history` 改为：`let next_follow = next >= max_top; let moved = next != current || target.lx.chat.follow_bottom != next_follow;`，仅在 `moved` 为真时更新 `history_scroll`/`follow_bottom`，返回 `moved || cancelled`。
  - 在 `Lx` 上抽取与 `HistorySource::has_history` 同源的 `has_history_content()`，替换 `viewport.rs` 中的 `messages.is_empty()` 判空。
  - 补充“贴底/单屏 `follow_bottom == true` 时向下滚动返回 `false`，且自动滚动立即停止”的回归测试。

---

### P0-2：`lx` 选区边缘自动滚动停留在 `app/` 层导致多击粒度破坏、几何冻结与 `Task` 可见行数未解析

- **涉及位置**：`src/app/update/selection.rs:705-867`、`src/app/update/interaction.rs:327`、`src/event/mouse/drag.rs:520-557`、`src/event/mod.rs:498-503`
- **关键点**：
  - `arm_lx_selection_autoscroll` / `tick_lx_selection_autoscroll` / `lx_autoscroll_step` 放在 `app/` 层，只在 `Drag` 时快照 `rect` 与 `total_rows`，每步只调用单格 `drag_lx_selection(state, pane, content_row, col)`。
- **错误原因**：
  1. **粒度与结构选区被破坏**：`app/` 无法访问 `ui::main_content::lx::unit_cell_range`，导致 `lx_autoscroll_step` 只能按 `Character` 推进终点。用户以双击选词（`Word`）、三击选行（`Line`）、表格单元格或代码块拖选超出上下边缘（`distance > 0`）时，`drag.rs:545` 刚写入的完整粒度范围在下一行 `drag.rs:546` 立即被覆盖成单格终点，后续每次 30ms tick 也丢弃词/行/表格边界与代码块左栏 `▎` 跳过规则。
  2. **缺少 `resolve_task_*_visible` 与动态几何**：对比 `wheel_lx_selection`（`wheel.rs:88-105`），自动滚动未在每步先同步 `resolve_task_output_visible` / `resolve_task_agents_visible` / `resolve_task_btw_visible` 与 `history_rows`。当 `TaskAgents`/`TaskBtw` 的 `rows_dirty == true`（流式输出或刚切 tab 未绘制）或 `visible == 0` 时，`scroll_agents_detail`/`scroll_btw_detail` 读到的 `total_rows` 为 0 或 `visible.max(1) == 1`，造成自动滚动直接失效或首步跳变整屏。
  3. **快照几何陈旧**：`History` 向上脱离 `follow_bottom` 后 `chat_bar` 出现使正文高度缩 1 行、滚动跨越用户消息使 `sticky` 置顶行显隐切换、或流式增量改变 `total_rows` 时，`geometry.rects` 未变不会触发清理，`lx_autoscroll_step` 持续使用过期 `rect`/`total_rows` 映射错误行号。
  4. **状态守卫与边界保护缺失**：`tick_lx_selection_autoscroll` 未检查 `state.overlay.is_some()`、`loading_session`、以及 `surface` 是否仍与 `pane.lx.task` 一致；`lx_autoscroll_step` 在 `rect.width == 0 && rect.x > 0` 时调用 `mouse.0.clamp(rect.x, rect.right().saturating_sub(1))` 会发生 `min > max` panic。
- **修复方案**：
  - 参考 `wheel_lx_selection` 与 `mouse::autoscroll::tick_drag` 架构，将 `lx` 选区自动滚动的 arm/tick/step 执行期全部迁入 `src/event/mouse/autoscroll.rs`（`app/state` 仅保留状态数据 `LxSelectionAutoscroll` 供 `next_deadline` 与清理由 `app/` 调用）。
  - 在每次越界立即跳步与 30ms tick 中：检查 `state.overlay.is_none()`、`PaneView::Lx`、实时状态匹配与 `rect` 非空；调用 `resolve_task_*_visible` / `history_rows` 滚动后，用当前帧 `content_geom` + `unit_cell_range` + `drag_lx_selection_range` 推进选区。

---

### P0-3：`Overlay::NewAgent` 未接入浮层输入契约与键盘创建标签露出逻辑

- **涉及位置**：`src/app/overlay.rs:99-157`、`src/app/update/dialog.rs:519-634`、`src/app/update/new_agent.rs:43-53, 83-91`、`src/event/overlay.rs:509-596`、`src/event/input.rs:189-205`
- **关键点**：
  - `65fdc1f3` 给 `NewAgentDialog` 加入了 `pub query: TextInput`，但未同步补全 `Overlay` 的输入访问器、`overlay_input_rect`、粘贴/剪切后处理及键盘确认露出逻辑。
- **错误原因**：
  1. **`Overlay::input` / `input_mut` / `clear_selection` 漏掉 `Self::NewAgent`**：终端粘贴（`apply_overlay_paste`）与剪切（`cut_overlay_selected_text`）全部失效；选中文本后按 `Ctrl/Cmd+C` 因 `overlay_selected_text` 返回 `None` 而穿透去执行 `Clear` 清空整个搜索框（违反 `tui-design-requirements.md` 选区复制优先级）；`Ctrl+Shift+A` 全选后按 `Esc` 无法先清选区而直接关弹窗。
  2. **粘贴/剪切及改词状态同步缺失**：接入 `input_mut()` 后，`apply_overlay_paste` 与 `cut_overlay_selected_text` 未在 `Overlay::NewAgent` 文本变化时将 `dialog.selected` 置 `0`、将 `viewport` 置 `None`，会使 `selected >= filtered_indices().len()`；此外 `apply_new_agent_key` 改搜索词时未重置 `state.new_agent_click = None`，400ms 内改动查询后点击同下标第 0 行会误判双击立即创建。
  3. **`overlay_input_rect` 漏掉 `Overlay::NewAgent`**：`handle_overlay_click` 与 `drag.rs` 无法命中搜索框，导致鼠标点击定位光标、双击选词、三击全选、拖选文本全部不可用。
  4. **键盘 `Enter` 创建后未露出新标签**：`src/event/input.rs:189-205` 仅对 `Menu`/`WorktreeOpen`/`NewWorkspace`/`TrustWorkspace` 调用 `reveal_active_tab(state, view)`，漏掉 `Overlay::NewAgent`，导致标签栏溢出时键盘新建并激活的末尾标签不可见。
- **修复方案**：
  - 在 `Overlay::input` / `input_mut` / `clear_selection` 补全 `Self::NewAgent(dialog) => Some(&dialog.query)`。
  - 在 `apply_overlay_paste`、`cut_overlay_selected_text` 与 `apply_new_agent_key` 中，当 `NewAgent` 的 `query` 变更时重置 `dialog.selected = 0`、`dialog.viewport = None` 与 `state.new_agent_click = None`。
  - 在 `overlay_input_rect` 增加 `Overlay::NewAgent(_)` 返回 `new_agent_search_rect(&shell)`。
  - 在 `src/event/input.rs` 对 `Overlay::NewAgent(_)` 的 `OverlayKey::Enter` 分支调用 `ensure_workspace_visible` 与 `reveal_active_tab`。

---

## P1 — 重要交互与边界缺陷（饥饿、误触、掉目标、布局与对称边界）

### P1-1：三套 `arm_*` 在 `Drag` 事件中无条件推迟 `next_at`，导致边缘移动时自动滚动饥饿与抖动超滚

- **涉及位置**：`src/event/mouse/autoscroll.rs:36-52`、`src/app/update/selection.rs:675-691, 722-743`
- **关键点**：每次 `MouseEventKind::Drag` 均无条件重写 `next_at: now + AUTOSCROLL_INTERVAL`，且 `distance > 0` 无条件立即步进。
- **错误原因**：
  1. 指针停在 2 行检测区内（`distance == 0`）并横向移动/微动时，只要终端连续上报间隔 < 30ms 的 `Drag` 事件，`next_at` 就会被持续推迟到未来，定时器永远不到期，导致自动滚动完全饥饿卡死。
  2. 指针越出视口（`distance > 0`）后横向轻微晃动时，每次 `Drag` 事件又会反复执行 `edge_scroll_lines(distance)`（最多 15 行/次）叠加推迟定时器，产生抖动暴冲。
- **修复方案**：
  - 在登记前比对已有计划：若当前已有同目标（同 `surface` / 同 `pane`）且同 `direction` 的自动滚动计划，**保留既有 `next_at`**（仅刷新 `mouse`/`distance`/`rect` 等参数）；仅在首次进入检测区或方向反转时设为 `now + AUTOSCROLL_INTERVAL`。
  - 对 `distance > 0` 的立即滚动，仅在首次进入视口外（之前未激活、方向切换或原 `distance == 0`）或越出距离进一步增大时立即步进，避免同距离横向抖动反复跳滚。

---

### P1-2：`autoscroll::drag_active` 未校验 `dragging` / `enabled`，边缘 2 行内点击抖动会误触发自动滚动与意外拖拽

- **涉及位置**：`src/event/mouse/autoscroll.rs:84-91`
- **关键点**：`drag_active` 仅检查 `Option::is_some()`，不检查 `drag.enabled` 与 `drag.dragging`。
- **错误原因**：
  1. Prompt 树点击 `Temp` 行或在搜索过滤态按下时，`begin_prompt_drag` 已设置 `enabled = false` 禁止拖拽；但指针停在顶/底 2 行内发生 1 列横向抖动时，`arm_drag` 仍激活 30ms 滚动，将用户点击中的树列表滚走。
  2. `Todo` 与 `PromptTree` 设计为“跨行离开起点行才激活 `dragging`”，原意是保障边缘行上的单击/长按/双击不会误合并或误移动。但按下顶/底 2 行内的条目后仅横向移动 1 列（`dragging == false`）也会被 `arm_drag` 登记定时滚动，30ms 后 `tick_drag` 滚出新行再调用 `drag_todo_to`/`drag_prompt_to`，强行将 `dragging` 翻为 `true` 并清空双击记录，导致普通点击/双击变成意外合并/移动。
- **修复方案**：
  - `PromptTree` 仅在 `drag.enabled && (drag.dragging || distance > 0)` 时允许激活/继续自动滚动。
  - `Todo` 仅在 `drag.dragging || distance > 0` 时允许激活/继续自动滚动（既屏蔽同一行内横向抖动误触，又保留从可见首/末行直接向上/下拖出列表边缘启动自动滚动的能力）。
  - `WorkspaceHistory` 同步校验 `state.workspace_history.search.text().trim().is_empty()`（搜索过滤态禁止拖拽重排）。

---

### P1-3：`arm_drag` 横向过早切断，且 `Todo` / `PromptTree` 纵向拖出列表边缘时落点丢失

- **涉及位置**：`src/event/mouse/autoscroll.rs:28-31, 93-111, 150-170`、`src/event/mouse/drag.rs:412-452, 648-672`、`src/ui/right_panel/todo.rs:381-387`、`src/ui/right_panel/prompt/tree.rs:298-313`
- **关键点**：
  - `arm_drag` 在 `mouse.0 < rect.x || mouse.0 >= rect.right()` 时直接清空 `drag_autoscroll`。
  - `todo::row_at` 与 `prompt::tree::drop_hit_at` 在列表区外返回 `None` / `Outside`。
- **错误原因**：
  1. `WorkspaceActive` / `WorkspaceHistory` 的落点函数 `workspace_drop_index` / `workspace_history_drop_index` 设计为“纵向钳制、横向不限”，但 `arm_drag` 却用横向判断在拖出窄侧栏一点或进入滚动条列时切断自动滚动。
  2. `Todo` 的 `effective_list_rect` 与 `PromptTree` 的 `prompt_main_rect` 均扣除了最右 1 列滚动条槽，指针仅滑到**面板自己的滚动条列**（`mouse.0 == rect.right()`）就会中止自动滚动。
  3. `PromptTree` 把自动滚动上界定在 `area.y + 2`（搜索框/分割线之下），而 `drop_hit_at` 对搜索框/分割线行及底边之外返回 `Outside`；`Todo` 的 `row_at` 对列表上下缘外返回 `None`。当指针在面板列范围内拖到上下边缘外触发自动滚动时，`step` 与 `drag.rs` 反复清除落点高亮，松手时还会把整次拖拽静默丢弃（同时也无法从可见首行直接向上拖出置臂）。
- **修复方案**：
  - `arm_drag` 对 `WorkspaceActive` / `WorkspaceHistory` 移除横向限制；对 `Todo` / `PromptTree` 放宽为以完整面板 `view.right_panel` 的水平范围 `x..right()` 判定（包含滚动条列，仅横向拖出右栏进入主区时按“拖出取消”处理）。
  - 保留 `todo::row_at` 与 `prompt::tree::drop_hit_at` 原有纯命中契约与单测，另增拖拽专用落点辅助（或仅在 `mouse.0` 仍在 `view.right_panel` 水平范围内且处于拖拽期时，对纵向越出列表的行钳位到列表首/末行，末行下方内部留白仍保留 `PromptDropHit::Root`），统一在 `drag.rs` 拖动/松手与 `autoscroll::step` 中使用；拖到右栏水平范围外仍返回 `None` / `Outside` 取消落点。

---

### P1-4：`NewAgentDialog` 缺少分隔线、呼吸空行、滚轮/滚动条与边界夹取，且 `app/` 引入 `ui::text` 冗余依赖

- **涉及位置**：`src/ui/overlay/new_agent.rs:29-186`、`src/app/overlay.rs:582-655`、`src/event/mouse/wheel.rs:306-355`、`src/event/overlay.rs:425-485`
- **关键点**：
  - `NewAgentDialog` 弹窗内无搜索分隔线与按钮前呼吸空行，无滚轮与滚动条支持，且 `filtered_indices` 调用了 `crate::ui::text::agent_label`。
- **错误原因**：
  1. **视觉与对齐不一致**：搜索行（`modal_input` 深灰底）直接紧贴选中行（`overlay_selection` 中灰底），末行候选/空态直接紧贴底部实心按钮，缺少 `WorktreeOpen`/`AgentTitle`/`SaveDraft` 统一的 `DIVIDER_MID` 分割线与 1 行呼吸区；空态/无匹配提示左上角绘制且未居中截断。
  2. **矮屏浏览与窗口边界缺口**：`wheel.rs` 拦截并吞掉 `NewAgent` 的滚轮事件，矮屏（`max_rows < filtered.len()`）下无法用滚轮滚动列表，也无右缘滚动条与 `viewport` 偏移；`new_agent_visible_start` 未对 `dialog.selected` 用 `filtered.len()` 夹取，一旦 `selected >= filtered.len()` 会渲染出空白窗口。
  3. **依赖方向违例**：`src/app/overlay.rs:610` 新增 `crate::ui::text::agent_label`，扩大了 `app -> ui::text` 历史债（违反 `project-directory-structure.md`），且 `crate::detect::launch_names(agent.kind)` 已覆盖全部 8 种 Agent 的稳定英文标签。
- **修复方案**：
  - 移除 `src/app/overlay.rs` 对 `crate::ui::text::agent_label` 的调用，改为匹配 `agent.command` 与 `detect::launch_names(agent.kind)`。
  - 为 `NewAgentDialog` 增加 `viewport: Option<usize>`、`normalize_selection()` 与 `scroll_viewport`/`set_viewport`，键盘移动/点选/改词时重置 `viewport = None`。
  - 更新 `src/ui/overlay/new_agent.rs`：加入搜索行下 `DIVIDER_MID`、按钮上方 1 行呼吸空白（`new_agent_shell` 高度 `rows + 8`）、空态居中截断、右缘滚动条与夹紧后的 `new_agent_visible_start`。
  - 在 `src/event/mouse/wheel.rs`（及滚动条点击/拖拽 `new_agent_scroll_drag`）接入 `Overlay::NewAgent` 滚动。

---

### P1-5：既有 Prompt 选区 `autoscroll_step` 顶底边界对称缺陷（底部不能上滚 / 顶部死循环）

- **涉及位置**：`src/app/update/selection.rs:916-927`
- **关键点**：`autoscroll_step` 的 Prompt 分支在滚动前无条件检查 `if state.prompt.scroll() >= cap { false }`，滚动后无条件返回 `true`。
- **错误原因**：
  1. 未区分 `AutoscrollDirection::Up` / `Down`：在正文底部（`scroll >= cap`）向上拖选时，先被 `>= cap` 拦下返回 `false`，导致到达过底部后无法向顶回滚。
  2. 当 `cap > 0` 且已滚到顶部 `scroll == 0` 时，进入 `else` 分支后忽略 `scroll_prompt_selection(state, -1, lines)` 返回的 `false`，恒返回 `true`，导致 `tick_selection_autoscroll` 在顶部永久每 30ms 空转（与 `P0-1` 对称）。
- **修复方案**：按 `direction` 分支处理——`Up` 直接返回 `scroll_prompt_selection(state, -1, lines)`；`Down` 在 `scroll >= cap` 时返回 `false`，否则先记 `before = state.prompt.scroll()`，滚动并钳到 `cap`，返回 `state.prompt.scroll() != before`。

---

## P2 — 生命周期守卫、轮子/探测细节、超限文件拆分与文档同步

### P2-1：拖拽滚轮与自动滚动的生命周期清理及可见性守卫缺口

- **涉及位置**：`src/event/mouse/wheel.rs:248-305`、`src/event/mouse/autoscroll.rs:56-111`、`src/event/mouse/drag.rs:381-452, 721`、`src/event/mouse/press.rs:691`、`src/app/update/menu.rs:161-223`、`src/app/update/prompt/menu.rs:15-103`、`src/app/update/workspace.rs:498-550`、`src/app/update/prompt/tree_view.rs:12-21`
- **关键点与原因**：
  1. `wheel.rs` 中 `prompt_drag` 未校验 `prompt_panel_active && prompt_tree_mode`、`workspace_drag` 未校验 `WorkspaceMode::Active`，且 `todo_drag`/`prompt_drag` 未包含右栏滚动条列；`prompt_tree_visible_rows` 读 `state.prompt.size().1`，在未同步尺寸前可能停留在默认值 `1`。
  2. `autoscroll::tick_drag` 缺少 `state.overlay.is_none()` 与面板可见性/模式守卫（`WorkspaceMode`、`RightTab::Todo && !prompt_collapsed`、`prompt_panel_active && prompt_tree_mode`）；`drag.rs` 中 `workspace_drag`/`workspace_history.drag`/`prompt_drag` 也未像 `todo_drag` 那样在浮层/模式切换时自愈清理，且 `Up` 路径若 `overlay.is_some()` 未清它们。
  3. `handle_mouse_down_right` 仅清 `todo_drag` 而漏清 `workspace_drag`、`workspace_history.drag`、`prompt_drag`、`drag_autoscroll`、`lx_selection_autoscroll`；`open_prompt_*_menu` 与 `open_pane_menu` 漏停在途拖拽/自动滚动；`close_pane` 漏清 `lx_selection` 与 `lx_selection_autoscroll`；`close_tab` 漏清锚定在该标签窗格上的 `state.toast`。
- **修复方案**：逐点补全上述可见性/模式/浮层守卫与对称清理入口；将 `wheel.rs` 中 `prompt_drag`/`todo_drag` 命中放宽至完整面板（含滚动条列），`prompt_tree_visible_rows` 尽量与当前几何一致（或在 `wheel.rs`/`autoscroll.rs` 直接按 `prompt_main_rect.height - 2` 计算）。

---

### P2-2：右键 Agents 滚动条无条件同步扫描 `PATH`，及 `platform/executable.rs` 探测边界瑕疵

- **涉及位置**：`src/event/overlay.rs:654-666`、`src/platform/executable.rs:15-76`
- **关键点与原因**：
  1. `open_right_click_menu` 在 `agents_section_at` 为真时直接执行全量 `crate::detect::installed_agents()`，右键 Agents 滚动条轨道（不弹任何菜单）也会同步遍历并 `stat` 全部 `PATH × 别名` 组合。
  2. `find_executable_in` 未跳过 `PATH` 中的空条目（如 `":/usr/bin"` 产生的 `""`），会按 TUI 进程当前工作目录解析相对路径；Unix `mode & 0o111 != 0` 未校验当前有效用户可执行权限（如 root 属主 `0o700` 会误报）；Windows 在内层循环重复读 `std::env::var("PATHEXT")` 并分配 `Vec`。
- **修复方案**：
  - 将 `state.installed_agents = crate::detect::installed_agents()` 移到确认命中 `agent_item_index_at` 或 `agents_blank_at` 之后执行，避免右键滚动条列做无效 IO。
  - `find_executable_in` 跳过空 `PATH` 段；Unix 侧先判 `metadata().is_file()` 再用 `libc::access(..., libc::X_OK) == 0` 精确校验可执行权限（NUL 含字路径安全拒绝）；Windows 在外层仅解析一次 `PATHEXT`。

---

### P2-3：超限文件拆分、失效注释与单元测试缺口

- **涉及位置**：
  - `src/app/state.rs`（1006 行，硬上限 1000）
  - `src/app/update/selection.rs`（1035 行，硬上限 1000）
  - `src/input/tests.rs`（1526 行，测试上限 1500）
  - `src/app/update/dialog.rs:46`（过期注释仍写“无文本输入”）
  - `src/ui/text/tests.rs:571-576`（`action_words_stay_spelled_out_while_keys_use_symbols` 漏检 `BUTTON_CREATE` / `BUTTON_TRUST`）
  - `src/app/overlay/tests.rs:203-235`（`overlay_kind_reports_variant` 漏测 `Overlay::NewAgent`）
- **错误原因**：本分支新增自动滚动结构体/更新函数与测试直接把 `state.rs`、`selection.rs` 与 `input/tests.rs` 推过规范行数上限；`65fdc1f3` 修改后未更新 `dialog.rs` 注释，且个别穷举测试未纳入新弹窗/按钮常量。
- **修复方案**：
  - 拆分 `src/app/state.rs` 中自动滚动/拖拽相关定义到 `src/app/state/interaction.rs`（并在 `state.rs` 保持重导出）；
  - 将 `src/app/update/selection.rs` 拆为目录模块 `src/app/update/selection/{mod.rs, lx.rs, autoscroll.rs, tests.rs}`（每个文件控制在 500 行左右）；
  - 将 `src/input/tests.rs` 按场景拆为 `src/input/tests/{mod.rs, overlay.rs, ...}`，使单测试文件 < 1000 行；
  - 更新过期注释并补全 `BUTTON_CREATE` / `Overlay::NewAgent` 回归测试。

---

### P2-4：特征文档与规范文档同步漂移

- **涉及位置**：
  - `docs/feature/agents-menu-drag-scroll-and-settings-fix/design.md`
  - `docs/feature/agents-menu-drag-scroll-and-settings-fix/task.md`
  - `docs/standards/tui-design-requirements.md:21`
- **错误原因**：
  1. 后续 3 个提交（`65fdc1f3` New agent 搜索、`8435fb78` 拖拽列表与 `lx` 选区边缘自动滚动、`150de2f2` 按键路由测试）已合入分支但未同步写入 `design.md` 与 `task.md`，`design.md` 仍明确写着“无文本输入”“不做拖拽边缘自动滚动”，文件清单也漏列了 10 余个改动文件。
  2. `task.md` 状态仍为 `待用户审核，未开始实现`，254 个复选框全部处于未勾选 `[ ]` 状态，且执行记录停留在 `a30c6a1e`（P1–P4）。
  3. `tui-design-requirements.md:21` 仍沿用旧版 Agents 菜单描述（缺少 `New agent` 且仍写“终端 Agent 是标签页唯一窗格时不显示 `Close`”）。
- **修复方案**：同步更新 `design.md`、`task.md` 与 `tui-design-requirements.md:21`，并将在本次 `review.md` 修复的 P0–P2 事项以完整记录与勾选态落入 `task.md`。

---

## 修复执行与验证结论

| 级别 | 编号 | 修复内容 | 状态 |
|---|---|---|---|
| P0 | P0-1 | 修复 `scroll_lx_history` 贴底 `moved` 误判死循环并支持非消息历史滚动 | 已修复并验证 |
| P0 | P0-2 | 将 `lx` 选区自动滚动执行期迁入 `event/mouse/autoscroll.rs`，保留多击/表格/代码块粒度并实时解析可见行数与几何 | 已修复并验证 |
| P0 | P0-3 | `Overlay::NewAgent` 完整接入 `Overlay` 输入/选区/矩形/粘贴剪切/`Enter` 露出与双击重置 | 已修复并验证 |
| P1 | P1-1 | `arm_drag` / `arm_lx_selection` / `arm_selection_autoscroll` 保留同向既有 `next_at`，避免横向移动饥饿与重复跳滚 | 已修复并验证 |
| P1 | P1-2 | `drag_active` 校验 `dragging` / `enabled` 与历史搜索态，消除边缘 2 行点击抖动误触 | 已修复并验证 |
| P1 | P1-3 | 放宽横向限制并接入 `todo_drag_hit` / `prompt_drag_hit` 右栏内纵向钳位与出栏取消 | 已修复并验证 |
| P1 | P1-4 | 移除 `app -> ui::text` 冗余依赖；补齐 `NewAgentDialog` 分隔线、呼吸行、空态居中、滚轮与滚动条 | 已修复并验证 |
| P1 | P1-5 | 修复 Prompt 选区 `autoscroll_step` 底部不能上滚与顶部死循环对称缺陷 | 已修复并验证 |
| P2 | P2-1 | 补全 `wheel.rs` / `autoscroll.rs` / `drag.rs` 守卫与右键/菜单/关窗格/关标签清理 | 已修复并验证 |
| P2 | P2-2 | 右键 Agents 滚动条不触发 PATH 扫描；`executable.rs` 跳过空 PATH 段、Unix 用 `libc::access(X_OK)`、Windows 单次读 `PATHEXT` | 已修复并验证 |
| P2 | P2-3 | 拆分 `state.rs`、`selection.rs`、`input/tests.rs` 满足行数规范，清理过期注释并补全回归测试 | 已修复并验证 |
| P2 | P2-4 | 同步更新 `design.md`、`task.md` 与 `docs/standards/tui-design-requirements.md` | 已完成 |

验证结果：`cargo fmt`、`cargo clippy --all-targets -- -D warnings`、`cargo test`（lib `5838 passed` + 全部集成测试）及 `cargo test --test web_assets`（`3 passed`）均全绿通过。
