# Toast 容器归属与复制反馈位置重调整 — 代码评审报告 (Code Review)

> 评审范围：`.worktrees/toast-container-placement`（分支 `feat/toast-container-placement`）
>
> 检索与分析工具：`codegraph`（调用链与影响面分析）、`codebase-memory-mcp`（`detect_changes` 变更影响面与全量符号检索）
>
> 优先级定义：
> - **P0**：功能性缺陷 / 交互竞态 Bug（直接导致 Toast 悬停冻结失效、鼠标穿透或 Toast 永久卡死不消失）
> - **P1**：容器生命周期与跨视图/跨标签页边界缺陷（导致同窗格双 Toast 重叠或后台窗格 Toast 窜位到全屏顶部）
> - **P2**：集成测试断言强化与文档注释同步

---

## P0：`/resume` 等二级面板或模态浮层打开期间，`lx_toast` 悬停判定被提前 `return` 截断，导致悬停冻结失效或 Toast 永久卡死不消失

### 1. 关键点
- **涉及文件**：
  - `src/event/mouse/hover.rs`（`handle_mouse_moved` L54-236）
  - `src/event/mouse/hit.rs`（`lx_toast_at`）
  - `src/ui/main_content/lx.rs`（`render` L124-135）
  - `src/app/update/resume_edit.rs`（`copy_resume_session_id`）

### 2. 错误原因
1. **渲染层与点击层的Z轴顺序**：在 `src/ui/main_content/lx.rs::render` 中，`pane.lx.toast` 在 `mention::render`、`render_paste_panel`、`refer::render`、`render_slash_panel` **之后**绘制（即视觉上浮在 `/resume`、`@` 提及面板等二级面板之上）；在左键点击处理 `src/event/mouse/press.rs`（L126）中，`lx_toast_at` 同样排在所有二级面板之前优先命中。
2. **悬停层的顺序错位**：在 `src/event/mouse/hover.rs::handle_mouse_moved` 中：
   - 全局 `state.toast` 的悬停状态更新（`set_toast_hover`）位于函数最顶部（L54-58，早于 `if state.overlay.is_some()` 及全部二级面板的 `return`）；
   - 但窗格内 `pane.lx.toast` 的悬停状态更新（`lx_toast_at` + `set_lx_toast_hover`）却被放在 L227-234，处于 `state.overlay.is_some()`（L60-128）以及 `paste_item_at`、`refer_item_at`、`mention_item_at`、`lx_paste_item_at`、`lx_mention_item_at`、`lx_refer_item_at`、`lx_slash_item_at`、`slash_item_at`、`panel_item_at` **共 10 处提前 `return` 分支之后**。
3. **触发的实际 Bug**：本次改造将 lx 窗格内的全部复制反馈（含 `/resume` 历史面板内按 `Ctrl/Cmd+C` 复制会话 ID）统一路由到 `pane.lx.toast`，从而触发两类必现缺陷：
   - **缺陷 A（悬停冻结失效 + 鼠标穿透）**：用户在 `/resume` 面板按 `Ctrl+C` 复制会话 ID 后，`/resume` 面板保持打开且 `pane.lx.toast` 浮在面板上方；当用户把鼠标移到 `pane.lx.toast` 上且该位置与底层 `/resume` 列表行重叠时，`lx_slash_item_at` 在 L198 提前命中并 `return`，导致 `set_lx_toast_hover(state, pane, true)` 根本不会执行（悬停无法暂停 2s 倒计时），同时鼠标在 Toast 盒内移动还会穿透高亮底下被遮挡的 `/resume` 候选行。
   - **缺陷 B（离开后 Toast 永久不消失）**：当用户先将鼠标悬停在 `pane.lx.toast` 上（此时 `toast.hovered == true`，`next_deadline()` 返回 `None`，到期计时被冻结），随后将鼠标移入打开的二级面板（如 `/resume`、`@` 提及面板）或右键打开上下文菜单（`state.overlay.is_some()`），`handle_mouse_moved` 在前方分支提前 `return`，跳过了 L227 的 `set_lx_toast_hover(state, pane, false)`，导致 `pane.lx.toast` 的 `hovered` 永久残留为 `true`，**该 Toast 永远不会自动过期消失**。

### 3. 修复方案与状态（已修复）
- [x] 在 `src/event/mouse/hover.rs::handle_mouse_moved` 顶部（紧随 `set_toast_hover` 之后、所有提前 `return` 分支之前）统一计算 `hovering_lx_toast` 并更新全部窗格的 `set_lx_toast_hover(state, pane, hovered)`（当顶层 `state.overlay.is_some()` 打开时，`hovering_lx_toast` 置 `None`，确保已悬停的 `lx_toast` 立即解除冻结）。
- [x] 当指针悬停在 `pane.lx.toast` 上（`hovering_lx_toast.is_some()`）时，在 `handle_mouse_moved` 中清理底层 tooltip 并直接返回，阻断鼠标悬停穿透到底下被 Toast 遮盖的 `/resume`、`@` 提及等二级面板。
- [x] 在 `src/event/tests/lx_toast.rs` 新增回归测试 `lx_toast_hover_takes_priority_over_slash_panel_and_clears_on_overlay`。

---

## P1-1：窗格视图切换（`Terminal ↔ Lx`）或关闭窗格时未清理旧视图 Toast，导致同窗格双 Toast 叠画或已关窗格 Toast 窜位

### 1. 关键点
- **涉及文件**：
  - `src/app/update/interaction.rs`（`show_toast` L152、`show_lx_toast` L161）
  - `src/app/update/pane.rs`（`toggle_pane_view` L920、`focus_agent_pane` L1045）
  - `src/app/update/workspace.rs`（`close_pane` L368）

### 2. 错误原因
1. 本次改造后，同一窗格 `pane_id` 在 `PaneView::Terminal` 下将复制 Toast 写入 `state.toast`（`anchor = Some(pane_id)`），在 `PaneView::Lx` 下将复制 Toast 写入 `pane.lx.toast`。
2. **视图切换导致同窗格双 Toast 重叠或冻结残留**：
   - 若用户在终端视图复制文本（`state.toast` 锚定 `Some(pane_id)`），在 2 秒内点击右上角 `[lx]` 切换到 `PaneView::Lx`（`toggle_pane_view`）：`state.toast` 未被清理，由于 `pane_id` 仍在 `pane_rects` 中，`state.toast` 会继续浮在 `Lx` 视图上方（违反 `AGENTS.md` “lx 页禁止使用全局 `AppState.toast` 覆盖层”规则）；若此时在 `Lx` 页内再触发复制或提示（写入 `pane.lx.toast`），同一窗格顶部会同时渲染 `state.toast`（`rect.y + 1`）与 `pane.lx.toast`（`inner.y + 1`），造成双框重叠。
   - 反之，若用户在 `Lx` 视图触发了 `pane.lx.toast` 后切换到 `Terminal` 视图（`toggle_pane_view` 或 `focus_agent_pane`），`pane.lx.toast` 未被清理；若切换前处于 `hovered = true` 冻结态，切回 `Lx` 视图时旧 Toast 仍残留。
3. **关闭窗格导致 Toast 窜位**：若用户在终端窗格 `pane_a` 复制文本后在 2 秒内关闭 `pane_a`（`close_pane`），锚定在 `Some(pane_a)` 的 `state.toast` 未被清除，下一帧因找不到 `pane_a` 而降级跳到全屏顶部正中央。

### 3. 修复方案与状态（已修复）
- [x] 在 `src/app/update/interaction.rs` 中保证同一窗格的两套槽位互斥：
  - `show_lx_toast(state, pane, toast)` 写入 `target.lx.toast` 时，若 `state.toast` 的 `anchor == Some(pane)`，同步清除 `state.toast = None`；
  - `show_toast(state, toast)` 写入 `state.toast` 时，若 `toast.anchor == Some(pane)`，同步清除该窗格的 `target.lx.toast = None`。
- [x] 在 `src/app/update/pane.rs` 的 `toggle_pane_view` 与 `focus_agent_pane` 中，切换视图时同步清理该窗格的 `pane.lx.toast` 及锚定在该窗格的 `state.toast`。
- [x] 在 `src/app/update/workspace.rs::close_pane` 中，关闭窗格时若 `state.toast` 锚定在被关闭的 `pane` 上，同步清除 `state.toast = None`。
- [x] 在 `src/app/update/tests.rs` 新增回归测试 `toggling_pane_view_or_closing_pane_clears_anchored_toast`。

---

## P1-2：切换标签页或工作区时，锚定在后台标签页终端窗格上的 `AppState.toast` 错误降级到全屏顶部正中央

### 1. 关键点
- **涉及文件**：
  - `src/ui/toast.rs`（`rect` L29-67）
  - `src/ui/toast/tests.rs`

### 2. 错误原因
1. `src/ui/toast.rs::rect` 在解析 `toast.anchor` 时：
   - 当 `toast.anchor == Some(pane_id)`（终端窗格）且用户切换到了其他标签页（`switch_tab`）或其他工作区（`switch_workspace`）时，当前活动标签页的 `pane_rects` 中不包含 `pane_id`，`container` 解析为 `None`。
   - 原逻辑将“`container` 为 `None`”一律等同于“无锚点（`anchor == None`）或容器过窄”，直接走到了末尾的屏幕顶部正中央降级分支。
2. 这导致用户在 Tab 1 的终端窗格复制文本后，若在 2 秒内切到 Tab 2 或 Workspace 2，原本属于 Tab 1 终端窗格的复制 Toast 会突然跳到 Tab 2 的全屏顶部正中央（而 `Lx` 窗格的 `pane.lx.toast` 则不会跨标签页泄漏）。

### 3. 修复方案与状态（已修复）
- [x] 在 `src/ui/toast.rs::rect` 中明确区分三类 `toast.anchor`：
  - `Some(id) if id == state.prompt.id()`：目标为全局右侧面板 `view.prompt`，面板展开且放得下时在面板内顶部居中，面板折叠或过窄时降级到屏幕顶部居中；
  - `Some(id)`（窗格锚点）：先检查 `state.active_tab().pane(id).is_some()`——若该窗格**不在当前活动标签页中**（已切到其他标签页/工作区或窗格不存在），直接返回 `None`（不跨标签页泄漏渲染）；若在当前活动标签页中，窗格放得下时在窗格内顶部居中，窗格过窄或主区折叠（`panes_collapsed`）放不下时降级到屏幕顶部居中；
  - `None`：直接在屏幕顶部正中央渲染。
- [x] 在 `src/ui/toast/tests.rs` 新增回归测试 `inactive_tab_pane_toast_does_not_leak_to_global_top_center`。

---

## P2：集成测试 `tests/prompt_copy.rs` 与 `src/app/toast.rs` 注释同步

### 1. 关键点
- **涉及文件**：
  - `tests/prompt_copy.rs`
  - `src/app/toast.rs`

### 2. 错误原因
1. `tests/prompt_copy.rs` 文件头注释仍写着旧文案 `Copied to clipboard`，且测试中仅断言屏幕包含 `"Copied "`，未验证复制 Toast 渲染在右侧 Prompt 面板所在的右半屏列区间内。
2. `src/app/toast.rs` 模块头注释仍写着“全局 toast 模型”，未反映 `Toast` 同时服务于 `AppState.toast`（全局/右侧面板/终端窗格锚点）与 `Lx.toast`（lx 窗格内槽位）的现状。

### 3. 修复方案与状态（已修复）
- [x] 更新 `tests/prompt_copy.rs` 注释，并在复制断言中校验 `"Copied "` 所在列坐标位于右侧 Prompt 面板列范围内（而非全屏居中的左中列）。
- [x] 更新 `src/app/toast.rs` 模块级文档注释。
