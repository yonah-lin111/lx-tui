# lx 与全局滚动速度统一 — 代码评审报告（Review）

- 目录：`docs/feature/lx-scroll-speed/`
- 分支：`feat/lx-scroll-speed`（已合并入主工作区 `dev` 分支，合并提交 `4c1d2ac9`；工作区 `.worktrees/lx-scroll-speed` 与分支已清理）
- 评审方式：基于 `codegraph` 与 `codebase-memory-mcp` 全链路追踪 `handle_mouse_wheel`、浮层面板优先级、视口钳制与脏标记传播

---

## 问题分级汇总（P0 → P2）

| 优先级 | 编号 | 模块 / 文件 | 关键问题摘要 | 状态 |
|--------|------|-------------|--------------|------|
| **P0** | P0-1 | `src/event/mouse/wheel.rs` | `lx` 存在聊天历史时，`/` 命令面板（含 `/model`、`/resume`、`/rewind`）滚轮被 `history_wheel_region` 抢先拦截，导致面板滚不动、误滚底层历史甚至直接关闭 `/rewind` 面板 | 已修复 |
| **P0** | P0-2 | `src/event/mouse/wheel.rs` | `lx` 主聊天历史区滚轮触顶/触底或未溢出时未 `return` 消费事件，导致滚轮向下穿透滚动底部多行输入框 | 已修复 |
| **P1** | P1-1 | `src/ui/prompt/panels.rs`<br>`src/app/prompt/refer.rs`<br>`src/app/update/editor.rs`<br>`src/event/mouse/hit.rs`<br>`src/event/mouse/wheel.rs` | `prompt` 快捷引用面板（`ReferPanel`）未映射 `viewport` 且未在 `wheel.rs` 拦截滚轮，拨动滚轮时穿透滚动底层 `prompt` 正文 | 已修复 |
| **P1** | P1-2 | `src/event/mouse/hit.rs`<br>`src/event/mouse/wheel.rs`<br>`src/ui/main_content/lx.rs` | `prompt` 与 `lx` 的路径粘贴选择面板（`paste_panel`）未拦截滚轮事件，在浮层上拨动滚轮会穿透滚动底层编辑器或聊天历史 | 已修复 |
| **P2** | P2-1 | `src/event/mouse/wheel.rs` | `prompt` 浮层面板（`@` 提及、`/` 斜杠、块命令）滚轮未产生位移（`scroll_*` 返回 `false`）时仍无条件置 `*dirty = true`，触发多余重绘 | 已修复 |

---

## 详细分析与修复方案

### P0-1：`lx` 存在聊天历史时，`/` 命令面板滚轮被 `history_wheel_region` 抢先拦截

- **关键点**：
  - `src/event/mouse/wheel.rs` 中 `lx` 窗格内浮层面板与 `history_wheel_region` 的滚轮命中顺序。
  - `src/app/update/chat/viewport.rs:21` 中 `scroll_lx_history` 调用的 `super::rewind::cancel_on_scroll(state, pane)`。
- **错误原因**：
  1. `lx` 的 `/` 命令面板（含 `/model`、`/resume`、`/rewind`）使用 `PanelPlacement::AboveAnchor` 紧贴底部输入框上沿展开，其屏幕矩形 `bar.rect` 完全落在 `history_wheel_region`（`area.y .. input.y`）内部。
  2. 在 `handle_mouse_wheel` 中，`@` 提及面板（`mention_layout`）与快捷引用面板（`refer_layout`）均位于 `history_wheel_region` 之前；唯独 `/` 命令面板（`lx_slash_panel_layout`）被放在了 `history_wheel_region` **之后**。
  3. 当会话为空（`messages.is_empty()`）时，`history_wheel_region` 返回 `None`，`/` 面板滚轮看似正常；但只要会话中有任意一条历史消息（`has_history(lx) == true`），在 `/`、`/model`、`/resume` 面板上拨动滚轮就会先命中 `history_wheel_region`，导致 `/` 面板完全滚不动，反而把背景聊天历史按 3 行/格滚走。
  4. 更严重的是，当打开 `/rewind` 面板时，误触发的 `scroll_lx_history` 会执行 `rewind::cancel_on_scroll`，直接把正在操作的 `/rewind` 面板关闭。
- **修复方案**：
  - 将 `lx_slash_panel_layout` 移至 `refer_layout` 之后、`task_rects` 与 `history_wheel_region` 之前，确保所有浮层命令面板在历史区与输入框之前优先拦截并消费滚轮事件。

---

### P0-2：`lx` 主聊天历史区滚轮触顶/触底或未溢出时穿透到底部多行输入框

- **关键点**：
  - `src/event/mouse/wheel.rs` 中 `history_wheel_region` 分支的条件守卫与 `return` 控制流。
- **错误原因**：
  原代码将 `&& update::scroll_lx_history(state, pane, delta, total, visible)` 写在了 `if` 条件头中：
  ```rust
  if let Some(delta) = vertical_wheel_delta(mouse.kind)
      && let Some(history_rect) = ...
      && history_rect.contains((mouse.column, mouse.row).into())
      && let Some((total, visible)) = ...
      && update::scroll_lx_history(state, pane, delta, total, visible)
  {
      *dirty = true;
      return;
  }
  ```
  当步长提升为 `WHEEL_LINES = 3` 后，聊天历史更快触顶（`history_scroll == 0`）或触底（`follow_bottom == true`），或者短对话未溢出（`max_top == 0`）。此时 `update::scroll_lx_history` 返回 `false`，导致整个 `if` 分支不进入、未执行 `return;`，控制流直接落入函数末尾的 `update::scroll_lx_input(state, pane, direction)`——鼠标明明悬停在上方聊天历史区，继续拨滚轮却把底部多行输入框滚走了。
- **修复方案**：
  - 只要 `history_rect.contains((mouse.column, mouse.row).into())` 命中，立即进入分支并在内部判断 `scroll_lx_history` 是否返回 `true` 来设置 `*dirty = true`，随后无条件 `return;` 终止事件向下穿透。

---

### P1-1：`prompt` 快捷引用面板（`ReferPanel`）缺失滚轮视口映射与事件拦截

- **关键点**：
  - `src/ui/prompt/panels.rs:96-135`（`ReferPanelData` 与 `refer_view`）
  - `src/app/prompt/refer.rs`（`Prompt::refer_scroll`）
  - `src/app/update/editor.rs`（`update::scroll_refer`）
  - `src/event/mouse/hit.rs`（`refer_panel_layout`）
  - `src/event/mouse/wheel.rs`（`handle_mouse_wheel`）
- **错误原因**：
  `ReferPanel`（`src/app/markdown/refer_shortcut.rs:145`）已实现 `scroll_viewport(delta, base)`，且 `lx` 输入框已接入 `scroll_lx_refer`；但 `prompt` 右栏侧：
  1. `ReferPanelData` 未保存 `panel.viewport()`，`refer_view` 将 `window_start` 写死为 `None`；
  2. `Prompt` 与 `app::update` 缺少 `refer_scroll` / `scroll_refer` 转发入口；
  3. `handle_mouse_wheel` 只拦截了 `mention`、`slash`、`block` 三个面板，漏掉了 `refer` 面板。用户在 `prompt` 快捷引用面板上拨滚轮时，面板不滚动，反而按 3 行/格滚动底层 `prompt` 编辑器（一旦光标被滚出视口，`refer_panel_data` 因 `prompt.cursor_cell()?` 返回 `None` 而直接导致面板消失）。
- **修复方案**：
  - 在 `ReferPanelData` 中补充 `viewport: Option<usize>` 并传入 `refer_view` 的 `window_start`；
  - 新增 `Prompt::refer_scroll`、`update::scroll_refer`、`hit::refer_panel_layout`；
  - 在 `handle_mouse_wheel` 中按 `WHEEL_LINES = 3` 拦截 `refer_panel_layout`。

---

### P1-2：`prompt` 与 `lx` 的粘贴选择浮层（`paste_panel`）未拦截滚轮事件

- **关键点**：
  - `src/ui/prompt/panels.rs:91`（`paste_panel_rect`）
  - `src/ui/main_content/lx.rs:349`（`paste_layout`）
  - `src/event/mouse/hit.rs` 与 `src/event/mouse/wheel.rs`
- **错误原因**：
  `src/ui/prompt/panels.rs:91` 定义了 `pub fn paste_panel_rect`（注释明确写着“用于滚轮命中”），`src/ui/main_content/lx.rs:383` 的 `command_panel_contains` 也将 `paste_layout` 列为四大浮层之一；但 `handle_mouse_wheel` 从未对 `paste_panel` 矩形做滚轮拦截。当粘贴选择浮层打开且指针位于其上时，拨动滚轮会穿透到底层滚动 `prompt` 正文或 `lx` 聊天历史/输入框。
- **修复方案**：
  - 在 `src/event/mouse/hit.rs` 新增 `paste_panel_rect(state, view)`；
  - 在 `src/event/mouse/wheel.rs` 中，当指针落在 `prompt` 的 `paste_panel_rect` 或 `lx` 的 `paste_layout` 内时直接吞没纵向滚轮并 `return;`。

---

### P2-1：`prompt` 浮层面板滚轮未移动视口时仍无条件置 `*dirty = true`

- **关键点**：
  - `src/event/mouse/wheel.rs:280-307`（`scroll_mention`、`scroll_slash`、`scroll_panel`）
- **错误原因**：
  `update::scroll_mention`、`update::scroll_slash`、`update::scroll_panel` 返回 `bool` 表示视口起点是否实际改变，但 `handle_mouse_wheel` 未判断返回值便无条件执行 `*dirty = true;`（而 `lx` 侧同类面板均判断了 `if update::scroll_lx_*(...) { *dirty = true; }`）。当面板已在顶部（`start == 0`）继续向上滚轮时，仍会触发无效重绘。
- **修复方案**：
  - 统一改为 `if update::scroll_*(state, delta, panel.start) { *dirty = true; }`。
