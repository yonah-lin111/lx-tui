# Toast 容器归属与复制反馈位置重调整 — 详细设计方案

> 状态：**已完成边界确认（Grill Me Q1–Q3）、代码实现、Code Review（P0–P2 修复）与测试校验**
>
> 涉及模块：`src/event/mouse/hit.rs`、`src/event/mouse/hover.rs`、`src/event/input.rs`、`src/db/service.rs`、`src/app/update/prompt.rs`、`src/app/update/menu.rs`、`src/app/update/todo.rs`、`src/app/update/interaction.rs`、`src/app/update/pane.rs`、`src/app/update/workspace.rs`、`src/ui/toast.rs`、`src/ui/main_content/lx/toast.rs`、`docs/standards/tui-design-requirements.md`

---

## 1. 背景与现状问题分析

### 1.1 现状双轨 Toast 渲染机制

当前 `lx-tui` 中已具备完善的双轨 Toast 渲染与几何定位能力：

1. **lx 窗格内专属 Toast 槽（`pane.lx.toast: Option<Toast>`）**：
   - **写入入口**：`update::show_lx_toast(state, pane, toast)` 或分派函数 `update::show_toast_at(state, Some(pane), toast)`（当 `pane.view == PaneView::Lx` 时）。
   - **渲染位置**：`src/ui/main_content/lx.rs` 调 `src/ui/main_content/lx/toast.rs::draw(buf, area, toast)`，在当前 lx 窗格内容区顶部水平居中（`area.x + (area.width - width) / 2, area.y + 1`）绘制，窄窗格自动折行（上限 `MAX_BODY_ROWS = 4`），多分屏时严格隔离在触发它的那个 lx 窗格内。
2. **带可选锚点的全局覆盖层 Toast 槽（`AppState.toast: Option<Toast>`）**：
   - **写入入口**：`update::show_toast(state, toast)` 或分派函数 `update::show_toast_at(state, anchor, toast)`（当 `anchor` 为 `None`、右侧面板 `Some(state.prompt.id())` 或终端窗格 `Some(terminal_pane_id)` 时）。
   - **渲染位置**：`src/ui/toast.rs::rect(state, view, pane_rects, screen, config)` 按 `toast.anchor` 解析目标容器：
     - `Some(id) if id == state.prompt.id()` → 容器取右侧面板 `view.prompt`（覆盖 `Prompt` 与 `Todo` 两个标签页），在右侧面板顶边框下方水平居中（`view.prompt.x + (view.prompt.width - width) / 2, view.prompt.y + 1`）；
     - `Some(id)`（终端窗格 `PaneId`）→ 容器取 `pane_rects` 中对应的窗格 `rect`，在该终端窗格顶边框下方水平居中（`container.x + (container.width - width) / 2, container.y + 1`）；
     - `None`（无锚点，或目标容器折叠/过窄放不下时）→ 降级到全屏顶边下方水平居中（`screen.x + (screen.width - width) / 2, screen.y + 1`）。

### 1.2 现状痛点：复制与部分右侧面板 Toast 锚点丢失

尽管底层渲染器（`ui::toast::rect` 与 `ui::main_content::lx::toast`）和状态分派器（`update::show_toast_at`）均已就绪，但当前代码中所有剪贴板复制路径及部分右侧面板错误提示被硬编码成了 `anchor = None`，导致本应出现在 `src/ui/main_content/` 或 `src/ui/right_panel/` 容器内的 Toast 全部跳到了全屏顶部正中央：

| 触发位置 | 触发路径 / 函数 | 现状传入 `anchor` | 现状问题 |
| --- | --- | --- | --- |
| **主内容区（终端窗格）** | 终端选区 `Ctrl/Cmd+C`（`event::input::copy_terminal_selection` → `copy_with_toast`） | 硬编码 `None` | 未锚定到当前焦点终端窗格 `Some(pane)` |
| **主内容区（lx 窗格）** | 历史/任务选区、底部输入框选区、Ask 编辑框选区 `Ctrl/Cmd+C`（`event::input::copy_lx_selection` → `copy_with_toast`） | 硬编码 `None` | 直接写 `state.toast`（`None`），未走 `show_toast_at` 落入 `pane.lx.toast` |
| **主内容区（lx `/resume` 面板）** | `/resume` 历史面板 `Ctrl/Cmd+C` 复制会话 ID（`resume_edit::copy_resume_session_id` → `Effect::CopyToClipboard`） | `Effect` 传了 `Some(pane)`，但 `db/service.rs` 写了 `anchor: _` 并硬编码 `None` | `service.rs` 丢弃了 `anchor`，未调 `show_toast_at` |
| **主内容区（窗格右键菜单）** | 右键终端或 lx 窗格点击 `Copy path`（`menu::activate_menu` → `Effect::CopyToClipboard`） | 硬编码 `None` | 未锚定到被右键的目标窗格 `Some(pane)` |
| **右侧面板（Prompt 编辑器）** | 编辑器 `Ctrl/Cmd+C` 复制选区/全文（`Routed::Copy` → `copy_with_toast`） | 硬编码 `None` | 未锚定到右侧面板 `Some(state.prompt.id())` |
| **右侧面板（Prompt 编辑器）** | `Ctrl/Cmd+Shift+C` 复制光标所在模板块（`Routed::CopyTemplateBlock` → `copy_with_toast`） | 硬编码 `None` | 未锚定到右侧面板 `Some(state.prompt.id())` |
| **右侧面板（Prompt 编辑器）** | 鼠标点击 `&&&` 模板块 `[copy]` 按钮（`hit::apply_template_button` → `copy_with_toast`） | 硬编码 `None` | 未锚定到右侧面板 `Some(state.prompt.id())` |
| **右侧面板（Prompt 编辑器）** | 鼠标点击 `+++` 补充块 `[copy]` 按钮（`hit::apply_block_button` → `copy_with_toast`） | 硬编码 `None` | 未锚定到右侧面板 `Some(state.prompt.id())` |
| **右侧面板（Prompt 树右键菜单）** | 右键提示词条目或临时提示词点击 `Copy content`（`copy_prompt_content` / `copy_prompt_temp_content` → `Effect::CopyToClipboard`） | 硬编码 `None` | 未锚定到右侧面板 `Some(state.prompt.id())` |
| **右侧面板（Todo 标签页）** | Todo 行内编辑框选中文字 `Ctrl/Cmd+C`（`Routed::Todo(TodoKey::Clear)` → `copy_with_toast`） | 硬编码 `None` | 未锚定到右侧面板 `Some(state.prompt.id())` |
| **右侧面板（保存失败反馈）** | Prompt 正文保存失败（`service.rs` `Effect::SavePromptContent`）与 Todo 写库失败（`todo::fail_todo_write`） | 硬编码 `None` | 与右侧面板其余错误 Toast（拖拽拒绝、移动失败、补充块禁折均用 `Some(state.prompt.id())`）不一致 |

---

## 2. 已确认边界（Grill Me 决策汇总）

| 编号 | 决策维度 | 最终确认口径 |
| --- | --- | --- |
| **Q1** | **三大区域复制 Toast 容器归属** | 统一通过 `update::show_toast_at(state, anchor, toast)` 分派：<br>1. **`src/ui/main_content/`（主内容区窗格）**：传入 `anchor: Some(pane_id)`。若目标窗格为 `PaneView::Lx`，自动落入该窗格专属槽 `pane.lx.toast`（在窗格内顶部居中绘制并支持折行）；若为 `PaneView::Terminal`，落入 `AppState.toast`（`anchor = Some(pane_id)`，在终端窗格顶部居中绘制）。<br>2. **`src/ui/right_panel/`（右侧面板，含 `Prompt` 与 `Todo`）**：传入 `anchor: Some(state.prompt.id())`，在右侧面板容器 `view.prompt` 顶部居中绘制。<br>3. **左侧栏（`src/ui/sidebar/`）**：工作区右键菜单 `Copy path` 保持 `anchor: None`，直接显示在全屏顶部正中央（全局 Toast）。 |
| **Q2** | **居中模态弹窗（Overlay）内复制归属** | 当居中模态弹窗处于打开状态且未关闭时（包括弹窗单行输入框 `overlay_selected_text` 选中复制、以及 `ReferencedFolderMenu` / `ReferencedFolderSearch` 弹窗内复制引用或根路径），统一传入 `anchor: None`，直接显示在**全屏顶部正中央（全局 Toast）**。<br>而右键上下文菜单（`Overlay::Menu`）点击 `Copy path` / `Copy content` 后菜单立即关闭，按 Q1 归入被操作的窗格或右侧面板容器。 |
| **Q3** | **右侧面板非复制类 Toast 统一性** | 将右侧面板剩余 2 处原本传 `None` 的保存失败 Toast（`Effect::SavePromptContent` 保存失败、`todo::fail_todo_write` 保存失败）也统一改为 `anchor: Some(state.prompt.id())`，使右侧面板内所有操作反馈位置完全一致。 |

---

## 3. 核心架构与数据流设计

### 3.1 剪贴板复制与 Toast 分派全景流

```text
┌───────────────────────────────────────────────────────────────────────────────┐
│                         触发复制 / 容器内反馈事件                              │
└───────────────────────────────────────┬───────────────────────────────────────┘
                                        │
        ┌───────────────────────────────┴───────────────────────────────┐
        ▼ (事件层直接复制)                                               ▼ (App 层纯状态产生 Effect)
[event::mouse::hit::copy_with_toast(state, text, anchor)]      [Effect::CopyToClipboard { text, anchor }]
  ├─ 主内容区终端选区 Ctrl/Cmd+C     -> Some(focus_pane)          ├─ lx /resume 面板 ^C 复制会话 ID -> Some(pane)
  ├─ 主内容区 lx 选区/输入框 Ctrl+C  -> Some(focus_pane)          ├─ 窗格右键菜单 Copy path         -> Some(pane)
  ├─ 右侧面板 Prompt 选区/全文 Ctrl+C -> Some(state.prompt.id())  ├─ Prompt 条目右键 Copy content   -> Some(state.prompt.id())
  ├─ 右侧面板 Prompt 模板块快捷键复制 -> Some(state.prompt.id())  ├─ 临时提示词右键 Copy content    -> Some(state.prompt.id())
  ├─ 右侧面板 &&& / +++ 块 [copy] 按钮-> Some(state.prompt.id())  ├─ 左侧工作区右键 Copy path       -> None
  ├─ 右侧面板 Todo 编辑框选区 Ctrl+C  -> Some(state.prompt.id())  └─ 引用文件夹模态弹窗内复制       -> None
  └─ 居中模态弹窗输入框选区 Ctrl+C    -> None                                    │
        │                                                                       │ (由 db::service::Persistence::execute 消费)
        └───────────────────────────────┬───────────────────────────────────────┘
                                        ▼
             platform::write_clipboard(text) -> 生成带标题 "Clipboard" 的 Toast
                                        │
                                        ▼
                 update::show_toast_at(state, anchor, toast)
                                        │
        ┌───────────────────────────────┼───────────────────────────────┐
        │ anchor == Some(pane)          │ anchor == Some(pane)          │ anchor == Some(state.prompt.id())
        │ 且 pane.view == PaneView::Lx  │ 且 pane.view == Terminal      │ 或 anchor == None
        ▼                               ▼                               ▼
[update::show_lx_toast]         [update::show_toast]            [update::show_toast]
写入 target.lx.toast            写入 state.toast                 写入 state.toast
        │                       (anchor = Some(pane))           (anchor = Some(prompt_id) 或 None)
        ▼                               │                               │
[ui::main_content::lx::toast]           ▼                               ▼
在该 lx 窗格内容区顶部居中绘制   [ui::toast::rect]               [ui::toast::rect]
(窄窗格内自动折行，多窗格隔离)   在该终端窗格顶边下居中绘制       在右侧面板顶边下居中 / 全屏顶部居中绘制
```

---

## 4. 模块级详细设计与关键代码

### 4.1 事件层剪贴板辅助函数与调用点改造（`src/event/mouse/hit.rs`、`src/event/input.rs`）

#### 4.1.1 `src/event/mouse/hit.rs`
1. 将 `copy_with_toast` 签名扩展为接收 `anchor: Option<PaneId>`，内部改调 `update::show_toast_at(state, anchor, toast)`（并将同一个 `anchor` 写入 `Toast::new`）。
2. 在 `apply_template_button`（`&&&` 模板块 `[copy]` 按钮）与 `apply_block_button`（`+++` 补充块 `[copy]` 按钮）中传入 `Some(state.prompt.id())`。

**关键代码（`src/event/mouse/hit.rs`）**：

```rust
/// 执行模板块按钮动作：复制写剪贴板并在右侧面板弹出 Toast，其余就地变更（均单步可撤销）。
pub(crate) fn apply_template_button(state: &mut AppState, hit: ui::prompt::TemplateButtonHit) {
    match hit.button {
        ui::prompt::TemplateBlockButton::Copy => {
            if let Some(text) = update::copy_template_block(state, hit.line) {
                let anchor = Some(state.prompt.id());
                copy_with_toast(state, &text, anchor);
            }
        }
        ui::prompt::TemplateBlockButton::Clean => {
            update::clean_template_block(state, hit.line);
        }
        ui::prompt::TemplateBlockButton::Delete => {
            update::delete_template_block(state, hit.line);
        }
        ui::prompt::TemplateBlockButton::Status => {
            update::toggle_template_status(state, hit.line);
        }
    }
}

/// 执行补充块按钮动作（单步可撤销）。
pub(crate) fn apply_block_button(state: &mut AppState, hit: ui::prompt::BlockButtonHit) {
    use crate::app::markdown::BlockKind;
    use ui::prompt::BlockButton;
    match (hit.kind, hit.button) {
        (BlockKind::Supple, BlockButton::Fold) => {
            state.prompt.toggle_supple_fold(hit.line);
        }
        (BlockKind::Supple, BlockButton::Copy) => {
            if let Some(text) = update::copy_supple_block(state, hit.line) {
                let anchor = Some(state.prompt.id());
                copy_with_toast(state, &text, anchor);
            }
        }
        (BlockKind::Supple, BlockButton::Clean) => {
            update::clean_supple_block(state, hit.line);
        }
        (BlockKind::Supple, BlockButton::Delete) => {
            update::delete_supple_block(state, hit.line);
        }
        _ => {}
    }
}

/// 写系统剪贴板并按 `anchor` 在目标容器（或全局屏幕顶部居中）弹出成功/失败 toast。
pub(crate) fn copy_with_toast(state: &mut AppState, text: &str, anchor: Option<PaneId>) {
    let now = Instant::now();
    let (kind, message) = if crate::platform::write_clipboard(text) {
        (ToastKind::Default, ui::text::toast_copied(text))
    } else {
        tracing::warn!("clipboard write failed");
        (ToastKind::Danger, ui::text::TOAST_COPY_FAILED.to_string())
    };
    update::show_toast_at(
        state,
        anchor,
        Toast::new(kind, message, anchor, now).with_title(ui::text::TOAST_CLIPBOARD_TITLE),
    );
}
```

#### 4.1.2 `src/event/input.rs`
在键盘复制路由中，按触发容器传入对应的 `anchor`：
- `Routed::Overlay`（居中模态弹窗输入框选区复制）→ `None`
- `Routed::Copy`（右侧面板 Prompt 编辑器复制）→ `Some(state.prompt.id())`
- `Routed::CopyTemplateBlock`（右侧面板 Prompt 模板块快捷键复制）→ `Some(state.prompt.id())`
- `Routed::Todo`（右侧面板 Todo 行内编辑框选区复制）→ `Some(state.prompt.id())`
- `copy_terminal_selection`（主内容区焦点终端窗格选区复制）→ `Some(pane)`
- `copy_lx_selection`（主内容区焦点 lx 窗格输入框/Ask 编辑框/历史与任务选区复制）→ `Some(pane)`

**关键代码（`src/event/input.rs`）**：

```rust
        Routed::Overlay(key) => {
            if key == OverlayKey::Clear
                && let Some(text) = overlay_selected_text(state)
            {
                if !text.trim().is_empty() {
                    copy_with_toast(state, &text, None);
                }
                *dirty = true;
                return;
            }
            // ...
        }
        Routed::Copy => {
            if update::prompt_tree_mode(state) {
                // 树视图 Ctrl+C 与模态输入框一致：清空搜索过滤。
                update::clear_prompt_search(state);
                *dirty = true;
            } else if let Some(text) = update::prompt_copy_text(state) {
                let anchor = Some(state.prompt.id());
                copy_with_toast(state, &text, anchor);
                *dirty = true;
            }
        }
        Routed::CopyTemplateBlock => {
            if let Some(text) = update::copy_template_block_at_cursor(state) {
                let anchor = Some(state.prompt.id());
                copy_with_toast(state, &text, anchor);
                *dirty = true;
            }
        }
        // ...
        Routed::Todo(code) => {
            if code == TodoKey::Clear
                && let Some(text) = state
                    .todo
                    .edit_input_mut()
                    .and_then(|inp| inp.selected_text())
            {
                if !text.trim().is_empty() {
                    let anchor = Some(state.prompt.id());
                    copy_with_toast(state, &text, anchor);
                }
                *dirty = true;
                return;
            }
            if update::apply_todo_key(state, code) {
                *dirty = true;
            }
        }
```

以及 `copy_terminal_selection` 与 `copy_lx_selection`：

```rust
/// 复制焦点终端窗格的选区文本并在该窗格顶部居中弹出 Toast；选区保留，无选区返回 false。
fn copy_terminal_selection(state: &mut AppState) -> bool {
    let pane = state.active_tab().layout.focus();
    let Some(text) = update::terminal_selection_text(state, pane) else {
        return false;
    };
    copy_with_toast(state, &text, Some(pane));
    true
}

/// 复制焦点 lx 窗格的底部输入框选区或主内容区选区文本并在该 lx 窗格内弹出 Toast；
/// 选区保留，无选区返回 false。
fn copy_lx_selection(
    state: &mut AppState,
    rects: &[(PaneId, ratatui::layout::Rect)],
    cache: &mut ui::RenderCache,
) -> bool {
    let pane = state.active_tab().layout.focus();
    // `/resume` 历史面板打开时 Ctrl/Cmd+C 归会话 ID 复制，不走选区复制。
    if state.pane_anywhere(pane).is_some_and(|target| {
        matches!(
            target.lx.slash,
            Some(crate::app::state::SlashPanel::History(_))
        )
    }) {
        return false;
    }
    if let Some(text) = state
        .pane_anywhere(pane)
        .filter(|target| target.view == PaneView::Lx)
        .and_then(|target| {
            if let Some(ask) = target.lx.ask.as_ref()
                && ask.editing
            {
                ask.editor().and_then(|ed| ed.selected_text())
            } else {
                target.lx.selected_text()
            }
        })
    {
        if !text.trim().is_empty() {
            copy_with_toast(state, &text, Some(pane));
        }
        return true;
    }
    let Some(selection) = state.lx_selection.filter(|sel| sel.pane() == pane) else {
        return false;
    };
    let Some(range) = selection.range() else {
        return false;
    };
    let area = rects
        .iter()
        .find(|(id, _)| *id == pane)
        .map(|(_, rect)| layout::pane_inner_rect(*rect));
    let Some((area, target)) = area.zip(state.pane_anywhere(pane)) else {
        return false;
    };
    if target.view != PaneView::Lx {
        return false;
    }
    let text = ui::main_content::lx::copy_text(area, &target.lx, range, cache.pane(pane));
    if !text.trim().is_empty() {
        copy_with_toast(state, &text, Some(pane));
    }
    true
}
```

---

### 4.2 状态更新层 `Effect::CopyToClipboard` 锚点绑定与右侧面板错误 Toast 统一（`src/app/update/`）

#### 4.2.1 `src/app/update/prompt.rs`
在右侧面板提示词树右键菜单触发的 `copy_prompt_content` 与 `copy_prompt_temp_content` 中，将 `anchor` 从 `None` 改为 `Some(state.prompt.id())`：

```rust
/// 复制提示词当前页正文到剪贴板（经效应出箱由事件层写入系统剪贴板，Toast 锚定右侧面板）。
pub fn copy_prompt_content(state: &mut AppState, workspace: usize, item_id: i64) {
    let Some(content) = state
        .workspaces
        .get(workspace)
        .and_then(|workspace| workspace.prompt_panel.tree.item(item_id))
        .and_then(|item| item.active_page())
        .map(|page| page.content.clone())
    else {
        return;
    };
    let anchor = Some(state.prompt.id());
    state.effects.push(Effect::CopyToClipboard {
        text: content,
        anchor,
    });
}

/// 复制临时提示词当前页正文到剪贴板；不入库、无确认，Toast 锚定右侧面板。
pub fn copy_prompt_temp_content(state: &mut AppState, workspace: usize, temp_id: u64) {
    let Some(content) = state
        .workspaces
        .get(workspace)
        .and_then(|workspace| workspace.prompt_panel.temp(temp_id))
        .and_then(|temp| temp.active_page())
        .map(|page| page.content.clone())
    else {
        return;
    };
    let anchor = Some(state.prompt.id());
    state.effects.push(Effect::CopyToClipboard {
        text: content,
        anchor,
    });
}
```

#### 4.2.2 `src/app/update/menu.rs`
在 `activate_menu` 中：
- 左侧工作区列表右键菜单 `OverlayTarget::Workspace(target)` 的 `MenuCommand::CopyPath` 保持 `anchor: None`（显示在全局 Toast）；
- 主内容区窗格右键菜单 `OverlayTarget::Pane { workspace, tab, pane }` 的 `MenuCommand::CopyPath` 将 `anchor` 改为 `Some(pane)`（显示在该窗格容器内）。

```rust
        (Some(MenuCommand::CopyPath), OverlayTarget::Workspace(target)) => {
            if let Some(path) = state
                .workspaces
                .get(target)
                .and_then(|workspace| workspace.cwd.as_ref())
            {
                state.effects.push(Effect::CopyToClipboard {
                    text: path.display().to_string(),
                    anchor: None,
                });
            }
        }
        (
            Some(MenuCommand::CopyPath),
            OverlayTarget::Pane {
                workspace,
                tab,
                pane,
            },
        ) => {
            let path = state.workspaces.get(workspace).and_then(|ws| {
                ws.tabs
                    .get(tab)
                    .and_then(|t| t.pane(pane))
                    .and_then(|p| p.cwd.clone())
                    .or_else(|| ws.cwd.clone())
            });
            if let Some(path) = path {
                state.effects.push(Effect::CopyToClipboard {
                    text: path.display().to_string(),
                    anchor: Some(pane),
                });
            }
        }
```

#### 4.2.3 `src/app/update/todo.rs`
将 `fail_todo_write` 的 Toast 锚点统一为右侧面板 `Some(state.prompt.id())`：

```rust
/// 服务回执：写入失败；在右侧面板展示错误 toast（内存列表保持不变）。
pub fn fail_todo_write(state: &mut AppState) {
    let anchor = Some(state.prompt.id());
    show_toast(
        state,
        Toast::new(
            ToastKind::Danger,
            crate::ui::text::TOAST_TODO_SAVE_FAILED,
            anchor,
            Instant::now(),
        ),
    );
}
```

---

### 4.3 持久层效应执行器改造（`src/db/service.rs`）

1. **`Effect::CopyToClipboard { text, anchor }`**：不再忽略 `anchor`，改为将 `anchor` 传给 `Toast::new` 并调用 `update::show_toast_at(state, anchor, toast)` 分派。
2. **`Effect::SavePromptContent` 保存失败分支**：将 `Toast::new` 的 `anchor` 从 `None` 改为 `Some(state.prompt.id())`。

**关键代码（`src/db/service.rs`）**：

```rust
            Effect::SavePromptContent {
                id,
                page_id,
                content,
                prompt,
                revision,
                ..
            } => {
                let saved = self.with_db(|db| db.update_page_content(page_id, &content));
                if saved.is_some() {
                    update::mark_prompt_page_persisted(
                        state, id, page_id, &content, prompt, revision,
                    );
                    true
                } else {
                    // 失败：保留脏状态与待保存内容，下一轮静置重试并在右侧面板给出可见反馈。
                    update::reset_prompt_autosave(state, prompt);
                    let anchor = Some(state.prompt.id());
                    update::show_toast(
                        state,
                        Toast::new(
                            ToastKind::Danger,
                            text::TOAST_PROMPT_SAVE_FAILED,
                            anchor,
                            Instant::now(),
                        ),
                    );
                    true
                }
            }
```

```rust
            Effect::CopyToClipboard { text, anchor } => {
                let now = Instant::now();
                let (kind, message) = if crate::platform::write_clipboard(&text) {
                    (ToastKind::Default, text::toast_copied(&text))
                } else {
                    tracing::warn!("clipboard write failed");
                    (ToastKind::Danger, text::TOAST_COPY_FAILED.to_string())
                };
                // 按触发容器分派：lx 窗格落窗格专属槽，终端窗格与右侧面板落各自容器锚点，
                // 左侧栏与居中模态弹窗（anchor = None）落全局屏幕顶部居中。
                update::show_toast_at(
                    state,
                    anchor,
                    Toast::new(kind, message, anchor, now).with_title(text::TOAST_CLIPBOARD_TITLE),
                );
                true
            }
```

---

## 5. 全场景 Toast 位置矩阵总览

修改完成后，应用中所有 Toast 的出现位置严格遵循以下矩阵：

| 区域 / 容器 | 具体场景 | 状态槽位 | `toast.anchor` | 最终渲染位置 |
| --- | --- | --- | --- | --- |
| **`src/ui/main_content/`（lx 视图）** | 历史/任务选区复制、底部输入框选区复制、Ask 编辑框选区复制、`/resume` 面板 `^C` 复制会话 ID、窗格右键 `Copy path`、命令/压缩/轮次/撤销等 lx 提示 | `pane.lx.toast` | `Some(pane_id)` | **该 lx 窗格内容区顶部水平居中**（窄窗格内自动折行） |
| **`src/ui/main_content/`（Terminal 视图）** | 终端选区 `Ctrl/Cmd+C` 复制、终端窗格右键 `Copy path` | `AppState.toast` | `Some(pane_id)` | **该终端窗格顶边框下方水平居中**（窗格过窄时降级全屏顶部居中） |
| **`src/ui/right_panel/`（Prompt / Todo）** | Prompt 编辑器 `Ctrl/Cmd+C` 复制、`Ctrl/Cmd+Shift+C` 复制模板块、`&&&` / `+++` 块 `[copy]` 按钮、提示词/临时提示词右键 `Copy content`、Todo 行内编辑框 `Ctrl/Cmd+C` 复制、Prompt 拖拽/移动拒绝、补充块非空禁折、Prompt/Todo 保存失败 | `AppState.toast` | `Some(state.prompt.id())` | **右侧面板（`view.prompt`）顶边框下方水平居中**（面板折叠或过窄时降级全屏顶部居中） |
| **左侧栏（`src/ui/sidebar/`）** | 工作区右键菜单 `Copy path`、左下角设置按钮不可用提示 | `AppState.toast` | `None` | **全屏顶部正中央（全局 Toast）** |
| **居中模态弹窗（`src/ui/overlay/`）** | 弹窗单行输入框选区 `Ctrl/Cmd+C` 复制、`ReferencedFolderMenu` / `ReferencedFolderSearch` 内复制引用或根路径、新建工作区路径校验失败、信任拒绝取消提示、引用文件夹增删失败 | `AppState.toast` | `None` | **全屏顶部正中央（全局 Toast）** |

---

## 6. 影响面与测试计划

### 6.1 受影响现有测试更新
1. **`src/app/update/tests.rs`**：
   - `activate_menu_copy_path_from_terminal_pane_prefers_pane_cwd`：断言 `Effect::CopyToClipboard` 的 `anchor` 为 `Some(pane)`；
   - `activate_menu_copy_path_from_lx_pane_falls_back_to_workspace_cwd`：断言 `Effect::CopyToClipboard` 的 `anchor` 为 `Some(pane)`；
   - `activate_menu_copy_path_from_workspace_emits_effect`：保持断言 `anchor: None`（左侧工作区栏走全局 Toast）。
2. **`src/app/update/prompt/tests.rs`**：
   - `item_menu_lists_commands_and_dispatches_copy_delete`、`temp_menu_lists_commands_and_deletes_immediately`、`copy_content_uses_active_page_only`：补充断言 `Effect::CopyToClipboard` 的 `anchor == Some(state.prompt.id())`。
3. **`src/event/tests/lx_view.rs`**：
   - `lx_history_selection_waits_for_copy_shortcut_and_follows_wheel_scroll`：更新断言——在 lx 历史选区按 `Ctrl+C` 后，复制 Toast 落在 `pane.lx.toast`（`toast.anchor == Some(pane)`），且全局 `state.toast.is_none()`；
   - `lx_task_output_and_agents_detail_selection_scroll_and_copy`：更新断言——在 lx 后台任务输出区按 `Ctrl+C` 后，复制 Toast 落在 `pane.lx.toast`（`toast.anchor == Some(pane)`），且全局 `state.toast.is_none()`。
4. **`src/event/tests/terminal.rs`**：
   - `terminal_selection_waits_for_copy_shortcut_and_clears_on_typing`（L486）：更新断言 `toast.anchor == Some(pane)`；
   - `double_and_triple_click_select_word_and_line_across_inputs`（L673）：
     - 双击 lx 输入框选词后按 `Ctrl+C`：断言复制 Toast 落在 `pane.lx.toast`（`Some("Copied 10 chars")`）且 `state.toast.is_none()`；
     - 双击右侧 Prompt 编辑器选词后按 `Ctrl+C`：断言复制 Toast 落在 `state.toast` 且 `toast.anchor == Some(state.prompt.id())`。
5. **`src/event/tests/prompt_editor.rs`**：
   - `mouse_supple_copy_button_writes_clipboard_toast`：补充断言 `toast.anchor == Some(state.prompt.id())`，并验证 `ui::toast::rect` 计算出的矩形水平居中于 `geo.view.prompt` 内部。

### 6.2 新增回归测试
- **右侧面板 Todo 编辑框复制与保存失败锚点测试**：验证在 Todo 行内编辑框选中文字按 `Ctrl+C` 以及 `fail_todo_write` 触发时，`state.toast` 的 `anchor` 均为 `Some(state.prompt.id())`。
- **`Persistence::execute` 处理 `Effect::CopyToClipboard` 路由测试**：验证 `anchor: Some(lx_pane)` 落入 `pane.lx.toast`、`anchor: Some(terminal_pane)` 落入带窗格锚点的 `state.toast`、`anchor: Some(state.prompt.id())` 落入带右侧面板锚点的 `state.toast`、`anchor: None` 落入全局 `state.toast`。
- **Code Review（P0–P2）回归测试**：
  - `lx_toast_hover_takes_priority_over_slash_panel_and_clears_on_overlay`（`src/event/tests/lx_toast.rs`）：验证 `/resume` 等二级面板打开时 `lx.toast` 悬停优先冻结且不穿透改写底部高亮行，以及打开顶层 `Overlay` 时自动解除 `lx.toast` 悬停冻结；
  - `toggling_pane_view_or_closing_pane_clears_anchored_toast`（`src/app/update/tests.rs`）：验证 `Terminal ↔ Lx` 视图切换或关闭窗格时自动清理旧视图关联的 Toast，防止同窗格双框叠画或关闭后窜位；
  - `inactive_tab_pane_toast_does_not_leak_to_global_top_center`（`src/ui/toast/tests.rs`）：验证切换标签页后非活动标签页窗格的 `state.toast` 不降级泄漏到全屏顶部居中。

### 6.3 规范文档同步
- 同步更新 `docs/standards/tui-design-requirements.md` 中“选择与复制 → 复制反馈”与“全局 toast”章节关于复制 Toast 按容器归属显示的描述。
