# lx 与全局滚动速度统一 — 设计文档

- 目录：`docs/feature/lx-scroll-speed/`
- 状态：**已实现并合并到 `dev` 分支**（2026-10-31，合并提交 `4c1d2ac9`）；自动测试与 Code Review P0–P2 修复全部落地（见 [review.md](./review.md)），工作区 `.worktrees/lx-scroll-speed` 与分支 `feat/lx-scroll-speed` 已清理。

---

## 1. 背景与根因分析

### 1.1 问题现象

1. 在 `src/ui/main_content/lx/`（主聊天历史与后台任务页 `Agents` 详情）中使用鼠标滚轮或触控板双指滚动时，滚动速度明显慢于右侧 `src/ui/prompt/` 编辑器。
2. 全局各可滚动容器的滚轮步长不统一：部分区域每格滚动 **3 行**，部分区域每格仅滚动 **1 行（项）**，在跨区域操作时存在明显的滚轮手感割裂。

### 1.2 核心代码根因

经 `codegraph` 与 `codebase-memory-mcp` 检索全仓滚动链路，定位到 `lx` 与 `prompt` 滚轮速度差异的直接根因位于 `src/event/mouse/wheel.rs` 与 `src/app/update/`：

1. **`prompt` 编辑器与 `lx` 输入框等区域乘了 `WHEEL_LINES = 3`**：
   - `prompt` 编辑器（`src/app/update/editor.rs:559-561`）：
     ```rust
     pub fn scroll_prompt(state: &mut AppState, direction: isize) -> bool {
         scroll_prompt_selection(state, direction, WHEEL_LINES)
     }
     ```
   - `lx` 底部输入框（`src/app/update/pane.rs:47-55`）：
     ```rust
     pub fn scroll_lx_input(state: &mut AppState, id: PaneId, direction: isize) -> bool {
         // ...
         pane.lx.scroll_by(direction.saturating_mul(WHEEL_LINES))
     }
     ```
   - `lx` 后台任务页进程输出与任务列表（`src/event/mouse/wheel.rs:419, 436`）：
     ```rust
     update::scroll_task_output(state, pane, direction.saturating_mul(WHEEL_LINES))
     update::scroll_task_list(state, pane, direction.saturating_mul(WHEEL_LINES))
     ```

2. **`lx` 主聊天历史与 `Agents` 详情未乘 `WHEEL_LINES`，仅传入 `direction`（`±1`）**：
   - 在 `src/event/mouse/wheel.rs` 的普通滚轮（L417、L474）与拖拽选区滚轮 `wheel_lx_selection`（L91、L99）中：
     ```rust
     // L91 / L417：Agents 详情直接传 direction (±1)
     update::scroll_agents_detail(state, pane, direction)

     // L99 / L474：主聊天历史直接传 direction (±1)
     update::scroll_lx_history(state, pane, direction, total, visible)
     ```
   - 由于 `vertical_wheel_direction` 返回 `-1` 或 `1`，`lx` 聊天历史与 `Agents` 详情每次滚轮事件只移动 **1 行**，速度仅为 `prompt` 编辑器的 **1/3**。

3. **全局 15 类滚动容器的步长与缓冲现状盘点**：

| 区域 / 容器 | 状态更新入口 | 当前滚轮步长 | 底部留白 / 呼吸位缓冲 | 渲染缓存机制 |
|-------------|--------------|--------------|-----------------------|--------------|
| `prompt` 编辑器正文 | `update::scroll_prompt` / `wheel_prompt_selection` | **3 行/格** (`WHEEL_LINES`) | `height / 4` 虚拟留白（溢出时可滚入） | 实时计算 `visual_rows` |
| `lx` 底部多行输入框 | `update::scroll_lx_input` | **3 行/格** (`WHEEL_LINES`) | 无 | 实时计算 `visual_rows` |
| `lx` 后台进程输出 | `update::scroll_task_output` | **3 行/格** (`WHEEL_LINES`) | 无 | 内存行切片 |
| `lx` 后台任务列表 | `update::scroll_task_list` | **3 项/格** (`WHEEL_LINES`) | 无 | 固定窗口 `LIST_MAX=6` |
| `lx` `ask` 面板（预览 / 选项） | `update::apply_ask_wheel` | **3 行(项)/格** (`PREVIEW_STEP`/`WHEEL_STEP`) | 无 | 实时计算 |
| 右栏 `Todo` 列表 | `update::scroll_todo` | **3 项/格** (`WHEEL_LINES`) | 无 | 实时计算 |
| 终端窗格回滚 (`HostScroll`) | `update::scroll_pane` | **3 行/格** (`WHEEL_LINES`) | 无 | `alacritty_terminal` 网格 |
| **`lx` 主聊天历史** | `update::scroll_lx_history` | **1 行/格**（过慢） | 末尾固定 **1 行** 呼吸位 (`snapshot.rs:146`) | `RenderCache` 两级缓存 + `Grid` |
| **`lx` `Agents` 详情** | `update::scroll_agents_detail` | **1 行/格**（过慢） | 末尾固定 **1 行** 呼吸位（同 `snapshot`） | `RenderCache` 两级缓存 + `Grid` |
| **`prompt` 树视图** | `update::scroll_prompt_tree` | **1 行/格** | 无 | 实时计算 |
| **侧栏 `Workspaces` 列表** | `update::scroll_workspace_list` | **1 项/格** | 无 | 实时计算 |
| **侧栏 `Agents` 列表** | `update::scroll_agent_list` | **1 项/格** | 无 | 实时计算 |
| **`prompt` 候选面板（`@`/`/`/块命令）** | `scroll_mention` / `scroll_slash` / `scroll_panel` | **1 项/格** | 无 | `command_panel::layout` |
| **`lx` 候选面板（`@`/快捷引用/`/`）** | `scroll_lx_mention` / `scroll_lx_refer` / `scroll_lx_slash` | **1 项/格** | 无 | `command_panel::layout` |
| **引用文件夹弹窗（菜单 / 搜索）** | `scroll_referenced_menu` / `scroll_referenced_search` | **1 项/格** | 无 | `overlay` 布局 |

4. **`WHEEL_LINES` 常量散落重复定义**：
   - `src/app/update/selection.rs:15`：`pub(crate) const WHEEL_LINES: isize = 3;`
   - `src/app/update/todo.rs:18`：`const WHEEL_LINES: usize = 3;`
   - `src/event/mouse/wheel.rs:23`：`const WHEEL_LINES: isize = 3;`
   - `src/app/update/chat/ask.rs:13-15`：`const PREVIEW_STEP: isize = 3;`、`const WHEEL_STEP: isize = 3;`

5. **`MentionState::scroll_viewport` 非 `Files` 标签页的视口累加偏差**：
   - 在 `src/app/prompt/mention.rs:529` 与 `src/app/state/lx/mention.rs:421` 中，当处于 `RefDir` / `Skills` / `Mcp` 标签页时：
     ```rust
     let current = self.ref_viewport.unwrap_or(base);
     let next = (current as isize + delta).max(0) as usize;
     let clamped = next.min(count.saturating_sub(1));
     ```
   - `Files` 标签页（`MentionPanel::scroll_viewport`）始终以渲染层传入的 `base`（即 `command_panel::layout` 已按 `max_window_start` 钳制后的 `panel.start`）为起点加上 `delta`；而非 `Files` 标签页却取了未按窗口可见高度钳制的 `self.ref_viewport`（上限只钳到了 `count - 1`）。当步长提升为 3 时，向下滚到底部会让 `ref_viewport` 虚高到 `count - 1`，导致反向向上滚时需要先空耗一格才能动。改为与 `Files` 标签页一致、直接从 `base` 起算即可彻底消除该隐患。

---

## 2. 已确认决策（Grill Me 收口）

| # | 决策项 | 结论 | 理由 |
|---|--------|------|------|
| 1 | `lx` 滚动速度调整范围與步长 | 将 `lx` 主聊天历史（含普通滚轮与拖拽选区滚轮 `wheel_lx_selection`）及后台任务页 `Agents` 详情统一调整为每格 **`WHEEL_LINES = 3` 行** | 对齐 `prompt` 编辑器、`lx` 输入框与后台进程输出的速度，消除 1 行/格带来的粘滞感与主历史 / 子 Agent 详情之间的手感割裂 |
| 2 | 全局滚动逻辑与缓冲策略统一 | 1. **步长全局统一**：全应用所有可滚动容器（含 `prompt` 树视图、侧栏 `Workspaces`/`Agents`、全部浮层/候选面板）滚轮步长统一为 **`WHEEL_LINES = 3` 行（项）/格**；<br>2. **保持即时跳行（不加跨帧插值拆帧）**：字符终端最小粒度为整行，跨帧缓动拆帧会导致快速滚动时画面滞后于手指；<br>3. **保留各区域既有底部缓冲与渲染缓存**：保留 `lx` 聊天历史末尾 1 行呼吸位 + `RenderCache` 两级缓存，以及 `prompt` 编辑器底部 `height / 4` 虚拟留白 | 统一全应用滚轮心理预期，触顶/触底均由 `clamp(0, max)` 自然钳制，短列表也不会越界 |

---

## 3. 详细技术方案与关键代码

### 3.1 统一全局滚轮步长常量 `WHEEL_LINES`

將 `src/app/update/selection.rs` 中的 `WHEEL_LINES` 提升为 `pub const WHEEL_LINES: isize = 3;`（由 `src/app/update.rs` 的 `pub use selection::*;` 统一对外暴露），删除 `wheel.rs`、`todo.rs`、`ask.rs` 中的重复私有常量定义。

- **文件**：`src/app/update/selection.rs`
```rust
// 修改前（L14-15）
/// 滚轮一格滚动的视觉行数；对齐 opencode 默认步长。
pub(crate) const WHEEL_LINES: isize = 3;

// 修改后
/// 全局滚轮一格滚动的视觉行数/条目数；对齐 opencode 默认步长。
pub const WHEEL_LINES: isize = 3;
```

- **文件**：`src/app/update/todo.rs`
```rust
// 修改前（L17-18, L535-538）
/// 滚轮每格移动的行数（与 prompt 正文一致）。
const WHEEL_LINES: usize = 3;
// ...
pub fn scroll_todo(state: &mut AppState, direction: isize) -> bool {
    let step = isize::try_from(WHEEL_LINES).unwrap_or(3);
    state.todo.scroll_by(direction.saturating_mul(step))
}

// 修改后
use super::selection::WHEEL_LINES;
// ...
pub fn scroll_todo(state: &mut AppState, direction: isize) -> bool {
    state.todo.scroll_by(direction.saturating_mul(WHEEL_LINES))
}
```

- **文件**：`src/app/update/chat/ask.rs`
```rust
// 修改前（L13-15, L279-286）
const PREVIEW_STEP: isize = 3;
const WHEEL_STEP: isize = 3;

// 修改后（复用 super::super::WHEEL_LINES）
use super::super::WHEEL_LINES;
// ...
    match zone {
        AskWheelZone::Preview => ask.scroll_preview(direction.saturating_mul(WHEEL_LINES)),
        AskWheelZone::Options => {
            let Some((base, total, visible)) = window else {
                return false;
            };
            ask.scroll_viewport(direction.saturating_mul(WHEEL_LINES), base, total, visible)
        }
    }
```

---

### 3.2 事件层滚轮入口统一乘 `WHEEL_LINES`（`src/event/mouse/wheel.rs`）

保持 `app::update` 层各 `scroll_*` 函数的“按精确行数/项数 `delta` 滚动”语义不变（避免破坏 `PageUp`/`PageDown` 的 `±10` 行调用与状态层细粒度单元测试），在 `src/event/mouse/wheel.rs` 统一将鼠标滚轮方向 `direction`（`±1`）换算为 `delta = direction.saturating_mul(WHEEL_LINES)`（`±3`）。

- **文件**：`src/event/mouse/wheel.rs`

#### 关键点 1：引入 `update::WHEEL_LINES` 与 `vertical_wheel_delta`
```rust
use crate::app::update::{self, WHEEL_LINES};

/// 纵向滚轮方向（负数向上）；横向滚轮返回 None。
pub(crate) fn vertical_wheel_direction(kind: MouseEventKind) -> Option<isize> {
    match kind {
        MouseEventKind::ScrollUp => Some(-1),
        MouseEventKind::ScrollDown => Some(1),
        _ => None,
    }
}

/// 纵向滚轮单格步长（负数向上，每格 `WHEEL_LINES` 行/项）；横向滚轮返回 None。
pub(crate) fn vertical_wheel_delta(kind: MouseEventKind) -> Option<isize> {
    vertical_wheel_direction(kind).map(|direction| direction.saturating_mul(WHEEL_LINES))
}
```

#### 关键点 2：`wheel_lx_selection`（`lx` 拖拽选区滚轮）统一为 `WHEEL_LINES`
```rust
// 修改前（L86-102）
    let moved = if has_task {
        update::resolve_task_output_visible(state, rects);
        update::resolve_task_agents_visible(state, rects);
        let agents_tab = state
            .pane_anywhere(pane)
            .and_then(|target| target.lx.task.as_ref())
            .is_some_and(|page| page.tab == crate::app::state::TaskTab::Agents);
        if agents_tab {
            update::scroll_agents_detail(state, pane, direction)
        } else {
            update::scroll_task_output(state, pane, direction.saturating_mul(WHEEL_LINES))
        }
    } else if let Some((total, visible)) = state
        .pane_anywhere(pane)
        .and_then(|target| ui::main_content::lx::history_rows(inner, &target.lx, cache.pane(pane)))
    {
        update::scroll_lx_history(state, pane, direction, total, visible)
    } else {
        false
    };

// 修改后
    let step = direction.saturating_mul(WHEEL_LINES);
    let moved = if has_task {
        update::resolve_task_output_visible(state, rects);
        update::resolve_task_agents_visible(state, rects);
        let agents_tab = state
            .pane_anywhere(pane)
            .and_then(|target| target.lx.task.as_ref())
            .is_some_and(|page| page.tab == crate::app::state::TaskTab::Agents);
        if agents_tab {
            update::scroll_agents_detail(state, pane, step)
        } else {
            update::scroll_task_output(state, pane, step)
        }
    } else if let Some((total, visible)) = state
        .pane_anywhere(pane)
        .and_then(|target| ui::main_content::lx::history_rows(inner, &target.lx, cache.pane(pane)))
    {
        update::scroll_lx_history(state, pane, step, total, visible)
    } else {
        false
    };
```

#### 关键点 3：`handle_mouse_wheel` 全局各滚动分支统一使用 `WHEEL_LINES`
```rust
    // 1. 引用文件夹内容搜索弹窗（L244-255）
    if let Some(delta) = vertical_wheel_delta(mouse.kind)
        && let Some(crate::app::overlay::Overlay::ReferencedFolderSearch(search)) =
            state.overlay.as_ref()
        && let Some(layout) = ui::overlay::referenced_search_layout(geometry.screen, search)
        && layout.area.contains((mouse.column, mouse.row).into())
    {
        if update::scroll_referenced_search(state, delta, layout.start, layout.visible()) {
            *dirty = true;
        }
        return;
    }

    // 2. 引用文件夹目录弹窗（L256-268）
    if let Some(delta) = vertical_wheel_delta(mouse.kind)
        && let Some(crate::app::overlay::Overlay::ReferencedFolderMenu(menu)) =
            state.overlay.as_ref()
    {
        let layout = ui::overlay::referenced_menu_layout(geometry.screen, menu);
        if layout.area.contains((mouse.column, mouse.row).into()) {
            if update::scroll_referenced_menu(state, delta, layout.start, layout.visible()) {
                *dirty = true;
            }
            return;
        }
    }

    // 3. prompt `@` 提及面板（L276-283）
    if let Some(delta) = vertical_wheel_delta(mouse.kind)
        && let Some(panel) = mention_panel_layout(state, view)
        && panel.rect.contains((mouse.column, mouse.row).into())
    {
        update::scroll_mention(state, delta, panel.start);
        *dirty = true;
        return;
    }

    // 4. prompt `/` 斜杠命令面板（L285-292）
    if let Some(delta) = vertical_wheel_delta(mouse.kind)
        && let Some(panel) = slash_panel_layout(state, view)
        && panel.rect.contains((mouse.column, mouse.row).into())
    {
        update::scroll_slash(state, delta, panel.start);
        *dirty = true;
        return;
    }

    // 5. prompt 块命令面板（L294-301）
    if let Some(delta) = vertical_wheel_delta(mouse.kind)
        && let Some(panel) = block_panel_layout(state, view)
        && panel.rect.contains((mouse.column, mouse.row).into())
    {
        update::scroll_panel(state, delta, panel.start);
        *dirty = true;
        return;
    }

    // 6. prompt 树视图（L360-367）
    if let Some(direction) = vertical_wheel_direction(mouse.kind) {
        if update::prompt_tree_mode(state) {
            let visible = update::prompt_tree_visible_rows(state);
            if update::scroll_prompt_tree(
                state,
                direction.saturating_mul(WHEEL_LINES),
                visible,
            ) {
                *dirty = true;
            }
        } else if update::scroll_prompt(state, direction) {
            *dirty = true;
        }
    }

    // 7. lx `@` 提及面板（L379-389）
    if let Some(delta) = vertical_wheel_delta(mouse.kind)
        && let Some(bar) = state
            .pane_anywhere(pane)
            .and_then(|target| ui::main_content::lx::mention_layout(inner, &target.lx))
        && bar.rect.contains((mouse.column, mouse.row).into())
    {
        if update::scroll_lx_mention(state, pane, delta, bar.start) {
            *dirty = true;
        }
        return;
    }

    // 8. lx 快捷引用面板（L391-401）
    if let Some(delta) = vertical_wheel_delta(mouse.kind)
        && let Some(bar) = state
            .pane_anywhere(pane)
            .and_then(|target| ui::main_content::lx::refer_layout(inner, &target.lx))
        && bar.rect.contains((mouse.column, mouse.row).into())
    {
        if update::scroll_lx_refer(state, pane, delta, bar.start) {
            *dirty = true;
        }
        return;
    }

    // 9. lx 后台任务页输出区（Processes 输出与 Agents 详情统一为 WHEEL_LINES = 3 行/格，L403-425）
    if let Some(delta) = vertical_wheel_delta(mouse.kind)
        && let Some(task_rects) = state
            .pane_anywhere(pane)
            .and_then(|target| ui::main_content::lx::task_rects(inner, &target.lx))
        && task_rects.output.contains((mouse.column, mouse.row).into())
    {
        update::resolve_task_output_visible(state, rects);
        update::resolve_task_agents_visible(state, rects);
        let agents_tab = state
            .pane_anywhere(pane)
            .and_then(|target| target.lx.task.as_ref())
            .is_some_and(|page| page.tab == crate::app::state::TaskTab::Agents);
        // Agents 详情与进程输出统一按 WHEEL_LINES（3 行/格）滚动。
        let changed = if agents_tab {
            update::scroll_agents_detail(state, pane, delta)
        } else {
            update::scroll_task_output(state, pane, delta)
        };
        if changed {
            *dirty = true;
        }
        return;
    }

    // 10. lx 后台任务页 item 列表（L430-440）
    if let Some(delta) = vertical_wheel_delta(mouse.kind)
        && let Some(rects) = state
            .pane_anywhere(pane)
            .and_then(|target| ui::main_content::lx::task_rects(inner, &target.lx))
        && rects.list.contains((mouse.column, mouse.row).into())
    {
        if update::scroll_task_list(state, pane, delta) {
            *dirty = true;
        }
        return;
    }

    // 11. lx 主聊天历史区（L466-478）
    if let Some(delta) = vertical_wheel_delta(mouse.kind)
        && let Some(history_rect) = state.pane_anywhere(pane).and_then(|target| {
            ui::main_content::lx::history_wheel_region(inner, &target.lx)
        })
        && history_rect.contains((mouse.column, mouse.row).into())
        && let Some((total, visible)) = state.pane_anywhere(pane).and_then(|target| {
            ui::main_content::lx::history_rows(inner, &target.lx, cache.pane(pane))
        })
        && update::scroll_lx_history(state, pane, delta, total, visible)
    {
        *dirty = true;
        return;
    }

    // 12. lx `/` 斜杠命令面板（L480-490）
    if let Some(delta) = vertical_wheel_delta(mouse.kind)
        && let Some((pane, inner)) =
            super::hit::lx_pane_at(state, rects, mouse.column, mouse.row)
        && let Some(bar) = super::hit::lx_slash_panel_layout(state, pane, inner)
        && bar.rect.contains((mouse.column, mouse.row).into())
    {
        if update::scroll_lx_slash(state, pane, delta, bar.start) {
            *dirty = true;
        }
        return;
    }

    // 13. 侧栏 Workspaces 与 Agents 列表（L503-521）
    } else if ui::workspace_section_at(view, state, mouse.column, mouse.row)
        && let Some(rows) = ui::workspace_list_rows(view, state)
        && let Some(delta) = vertical_wheel_delta(mouse.kind)
        && update::scroll_workspace_list(state, delta, rows)
    {
        let hover = ui::workspace_item_at(view, state, mouse.column, mouse.row);
        update::set_workspace_hover(state, hover);
        *dirty = true;
    } else if ui::agents_section_at(view, state, mouse.column, mouse.row)
        && let Some(rows) = ui::agent_list_rows(view, state)
        && let Some(delta) = vertical_wheel_delta(mouse.kind)
        && update::scroll_agent_list(state, delta, rows)
    {
        let hover = ui::agent_item_index_at(view, state, mouse.column, mouse.row);
        update::set_agent_hover(state, hover);
        *dirty = true;
    }
```

---

### 3.3 `MentionState::scroll_viewport` 视口起点对齐（`src/app/prompt/mention.rs` & `src/app/state/lx/mention.rs`）

当 `self.tab != MentionTab::Files` 时，与 `MentionPanel::scroll_viewport`（`src/app/markdown.rs:304`）保持完全一致的算式：以调用方传入的 `base`（渲染布局实际生效的 `panel.start`）作为起点累加 `delta` 并钳制到 `0..=max`，不再使用可能超过 `max_window_start` 的旧 `self.ref_viewport`。

- **文件**：`src/app/prompt/mention.rs`（L523-537）与 `src/app/state/lx/mention.rs`（L415-429）
```rust
    pub fn scroll_viewport(&mut self, delta: isize, base: usize) -> bool {
        if self.tab != MentionTab::Files {
            let count = self.secondary_count();
            if count == 0 {
                return false;
            }
            let max = count.saturating_sub(1) as isize;
            let clamped = (base.min(max as usize) as isize + delta).clamp(0, max) as usize;
            let changed = clamped != base;
            self.ref_viewport = Some(clamped);
            return changed;
        }
        match self.panel.as_mut() {
            Some(panel) => panel.scroll_viewport(delta, base),
            None => false,
        }
    }
```

---

### 3.4 规范文档同步（`docs/standards/tui-design-requirements.md`）

- **文件**：`docs/standards/tui-design-requirements.md`（L70）
  - 将“滚轮悬停在 workspaces 区滚动工作区列表（每格 1 项）”更新为“滚轮悬停在 workspaces 区滚动工作区列表（每格 3 项，与全局滚轮步长一致）”。

---

### 3.5 代码评审（Review P0–P2）发现的边界缺陷与加固设计

详见 `docs/feature/lx-scroll-speed/review.md`：

1. **P0-1：`lx` `/` 命令面板滚轮优先级高于 `history_wheel_region`**
   - `/` 命令面板（含 `/model`、`/resume`、`/rewind`）以 `AboveAnchor` 贴在输入框上沿，其矩形位于 `history_wheel_region` 内。将 `lx_slash_panel_layout` 移至 `refer_layout` 之后、`task_rects` 与 `history_wheel_region` 之前，防止有聊天历史时滚轮被 `history_wheel_region` 抢先拦截。
2. **P0-2：`lx` `history_wheel_region` 触顶/触底时消费滚轮事件，严禁穿透到底部输入框**
   - 当 `history_rect.contains(...)` 命中时，在块内判断 `update::scroll_lx_history` 是否返回 `true` 来设置 `*dirty = true`，随后无条件 `return;`，防止历史区触顶/触底返回 `false` 时落入末尾的 `update::scroll_lx_input`。
3. **P1-1：补齐 `prompt` 快捷引用面板（`ReferPanel`）滚轮支持**
   - `src/ui/prompt/panels.rs` 的 `ReferPanelData` 补充 `viewport: panel.viewport()` 并映射至 `CommandPanelView.window_start`；
   - 新增 `Prompt::refer_scroll`（`src/app/prompt/refer.rs`）、`update::scroll_refer`（`src/app/update/editor.rs`）、`hit::refer_panel_layout`（`src/event/mouse/hit.rs`），并在 `handle_mouse_wheel` 中按 `WHEEL_LINES = 3` 拦截滚动。
4. **P1-2：`prompt` 与 `lx` 路径粘贴选择面板（`paste_panel`）吞没滚轮防穿透**
   - 在 `handle_mouse_wheel` 中对 `prompt` 的 `paste_panel_rect` 与 `lx` 的 `paste_layout` 矩形做滚轮命中吞没（`return;`）。
5. **P2-1：`prompt` 浮层面板滚轮仅在产生实际位移时置 `*dirty = true`**
   - `update::scroll_mention`、`update::scroll_refer`、`update::scroll_slash`、`update::scroll_panel` 返回 `true` 时才置 `*dirty = true`。

---

## 4. 影响范围与不变量检查

1. **`app/` 与 `ui/` 边界不变量**：
   - `ui/` 完全不修改，保持只读状态。
   - `app::update` 中所有 `scroll_*` 函数签名保持不变，键盘 `PageUp`/`PageDown`（`±10` 行）和方向键 `↑`/`↓`（`±1` 项）行为不受任何影响。
2. **越界钳制与滚动守卫不变量**：
   - 所有滚动函数内部均通过 `.clamp(0, max as isize)` 钳制。当剩余可滚行数 `< 3` 时（例如只差 1 行或 2 行到底），拨一下滚轮直接精确停在底部 `max`；已在顶部/底部或内容未溢出（`max == 0`）时返回 `false`，不置 `*dirty`，不触发无效重绘。
3. **拖拽选区滚轮联动不变量**：
   - 在 `lx` 聊天历史或 `Agents` 详情中按住鼠标左键拖拽选区并拨动滚轮时，视口每次移动 3 行（受 `max_top` 钳制），随后 `geom.cell(mouse.column, mouse.row)` 基于新的 `scroll` 偏移重算鼠标下的内容行坐标，选区锚点保持固定、终点实时准确跟随。
4. **缓冲与缓存机制保留**：
   - `lx` 聊天历史继续保留 `RenderCache`（`PaneCache` 一级 `HistorySnapshot` + 二级 32MB LRU `BlockPaint`）以及末尾 1 行呼吸位（`snapshot.rs:146`）。
   - `prompt` 编辑器继续保留 `bottom_padding() = height / 4` 的底部虚拟留白与 `max_scroll` 守卫恒等式。

---

## 5. 测试更新与新增验证计划

### 5.1 既有事件层测试期望值同步（因鼠标滚轮步长从 1 升为 `WHEEL_LINES = 3`）

1. `src/event/tests/lx_view.rs`：
   - `lx_mention_wheel_scrolls_panel_not_input`（L384）：10 个候选项从 `0` 向下滚 1 格，`viewport()` 期望值从 `Some(1)` 更新为 `Some(3)`。
   - `lx_history_selection_waits_for_copy_shortcut_and_follows_wheel_scroll`（L569-645）：总行数 8、可见行数 4（`max_top = 4`），按住左键向下滚轮 1 次即移动 3 行、2 次即钳制在 `max_top = 4`，选区终点保持 `Some(((0, 0), (4, 15)))`（核对并新增单次滚轮移动 3 行的断言）。
   - `lx_task_output_and_agents_detail_selection_scroll_and_copy`（L983-1013）：`Agents` 详情从 `scroll = 0` 按住左键向下滚轮 1 次（移动 3 行），鼠标停在 `agent_geom.rect.y + 2`（可视第 2 行），对应内容行为 `3 + 2 = 5`，选区范围从 `Some(((0, 0), (3, 6)))` 更新为 `Some(((0, 0), (5, 6)))`。
2. `src/event/tests/mouse_click.rs`：
   - `wheel_over_workspace_list_rescrolls_hover`（L298-299）：30 个工作区向下滚轮 1 次，`workspace_scroll` 与 `workspace_hover` 从 `1` / `Some(1)` 更新为 `3` / `Some(3)`。
   - `wheel_over_agents_section_scrolls_list`（L413-414）：20 个 agent 向下滚轮 1 次，`agent_scroll` 与 `agent_hover` 从 `1` / `Some(1)` 更新为 `3` / `Some(3)`。
3. `src/event/tests/prompt_menu.rs`：
   - `mouse_hover_over_referenced_menu_after_wheel_keeps_viewport`（L338）：30 项引用菜单向下滚轮 1 次后 `menu.viewport` 从 `Some(1)` 更新为 `Some(3)`。
   - `mouse_wheel_over_referenced_menu_scrolls_viewport_not_highlight`（L505）：向下滚轮 1 次后 `menu.viewport` 从 `Some(1)` 更新为 `Some(3)`。
4. `src/event/tests/prompt_tree.rs`：
   - `mouse_wheel_and_hover_and_click_on_block_panel`（L338）：块命令面板向下滚轮 1 次后 `viewport()` 从 `Some(Some(1))` 更新为 `Some(Some(3))`。
   - `mouse_wheel_over_mention_panel_scrolls_viewport_only`（L415）：`@` 提及面板（8 项双行条目，可见 4 项，`max_window_start = 4`）向下滚轮 1 次后 `after.start` 从 `before.start + 1` 更新为 `before.start + 3`。

### 5.2 新增专项回归测试

- 在 `src/event/tests/lx_view.rs` 新增测试 `lx_history_wheel_scrolls_by_wheel_lines_and_clamps`：
  - 构造长消息历史，验证在 `lx` 主聊天历史区向下/向上拨动 1 格滚轮时，`history_scroll` 精确移动 `WHEEL_LINES`（3 行）、触顶钳制在 `0` 且不置脏、距底不足 3 行时向下滚轮直接钳制到 `max_top` 并恢复 `follow_bottom`。
- 在 `src/event/tests/prompt_editor.rs` 新增测试：
  - `lx_slash_wheel_prioritizes_over_history_and_clamped_history_never_leaks_to_input`（覆盖 P0-1、P0-2）
  - `prompt_refer_panel_wheel_scrolls_by_wheel_lines_without_scrolling_editor`（覆盖 P1-1、P2-1）
- 在 `src/event/tests/focus_and_paste.rs` 新增测试：
  - `paste_panel_swallows_mouse_wheel_without_scrolling_underlying_views`（覆盖 P1-2）
- 执行 `cargo fmt --check`、`cargo clippy --all-targets -- -D warnings` 与受影响模块 `cargo test` 验证全绿。
