# Prompt 编辑页快捷键调整、右键菜单与工具栏/底边按钮优化设计

- 状态：**已完成**
- 适用范围：`src/input/`、`src/event/`、`src/layout/prompt.rs`、`src/app/`（`overlay.rs`、`effects.rs`、`prompt/`、`markdown/template.rs`、`update/`）、`src/db/`、`src/ui/`（`prompt.rs`、`prompt_frame.rs`、`controls.rs`、`right_panel.rs`、`overlay/menu.rs`、`text.rs`、`tooltip.rs`）。
- 架构约束：保持 `app/`（纯状态与更新）、`ui/`（纯渲染只读状态）、`layout/`（纯几何计算）与 `db/`（SQLite 持久化）边界清晰；所有用户可见界面文案与 Toast 统一使用英文并集中定义在 `src/ui/text.rs`；生产代码严禁 `unwrap()` / `expect()`。

---

## 1. 背景与目标

当前 `src/ui/prompt/`、`src/ui/controls.rs` 与 `src/app/update/prompt/` 在分页编辑、工具栏与底边框按钮排布上存在以下改进诉求：

1. **快捷键行为调整（`Cmd/Ctrl + Shift + →`）**：
   - 目前在 `PromptMode::Editor` 下按 `Cmd/Ctrl + Shift + →` 无论处于第几页都会直接触发新建页（`EditorCommand::NewPage`）。
   - 调整为：在多页文档（例如共 5 页，当前在第 3 页）中按 `Cmd/Ctrl + Shift + →` 时，**先切换到下一页**（第 4 页）；**只有在最后一页（第 5/5 页）中使用该快捷键时才触发新建页面**。
2. **为 Prompt 编辑页添加右键菜单**：
   - 目前 `PromptMode::Editor` 正文区右键点击无响应。
   - 新增编辑页右键菜单，支持 **全选（Select All）、复制全文（Copy）、重命名页（Rename Page）、新建页（New Page）、克隆页（Fork Page）、删除页（Delete）**；同时在 `PromptMode::Pages` 页列表行右键菜单中同步补齐重命名页与克隆页。
   - 点击「重命名页（Rename Page）」后，滚动条调整到顶部（`scroll_to(0)`），并选中 `{title: ...}` 中的标题内容。
3. **移除顶部工具栏 `[all]` 按钮**：
   - 全选能力已交由右键菜单 `Select All` 承接，顶部工具栏不再保留 `[all]`（`PromptToolbarButton::SelectAll`）。
4. **将 Pin 按钮移至底边框折叠按钮左侧并改为英文**：
   - 移除顶部工具栏的圆形符号钉住按钮（`[○]` / `[◉]`）。
   - 将钉住按钮移至右侧容器底边框行、紧贴折叠按钮（`[▶]` / `[◀]`）左侧（留 1 列 `─` 间隔），文案改为英文 **`[pin]`**（未钉住）与 **`[unpin]`**（已钉住），底边框左侧工作区身份标签自动向右避让钉住按钮。

---

## 2. 已确认的边界决策汇总（Grill Me 结论）

| 决策项 | 确认方案 |
| --- | --- |
| **`Cmd/Ctrl + Shift + →` 行为** | 在 `PromptMode::Editor` 下：若当前页不是最后一页（`active_index + 1 < pages.len()`），行为等同于 `PageNext`，直接切换到下一页；仅当已处于最后一页（`active_index + 1 == pages.len()`）时，才执行 `create_or_toast_new_page`（若已有空白页则跳转并弹 Warning Toast，否则末尾新建页并切入）。`Cmd/Ctrl + ←` 与 `Cmd/Ctrl + Shift + ←` 保持切换上一页不变。 |
| **Rename Page 定位与选中** | 1. **有标题值**（如 `{title: 登录接口重构}`）：编辑器 `scroll_to(0)`，反白选中冒号后、右花括号前的标题内容（`登录接口重构`，不含前导空白与外侧 `{title:` / `}`），光标置于标题值末尾，直接打字即可替换。<br>2. **空标题值**（如 `{title: }`）：编辑器 `scroll_to(0)`，清空选区，光标精准停在 `{title: ` 与 `}` 之间。<br>3. **首行有 `&&&` 模板块但无 `{title: ...}` 行**：在首个模板块起始行下一行自动补入 `{title: }`，滚回顶部并将光标置入 `{title: |}`。<br>4. **页面无任何 `&&&` 模板块**：不破坏纯文本内容，弹出 Warning Toast `"No {title:} found in current page"`。 |
| **Fork Page 行为** | 1. 先结算当前编辑器未保存修改（`commit_editing_prompt`）。<br>2. **空页防呆**：若被 Fork 的源页内容为空（`content.trim().is_empty()`），复用空页拦截逻辑跳转并弹出 Warning Toast `"Already have a new page"`。<br>3. **内容与标题处理**：克隆源页完整正文；若源页首个模板块含非空 `{title: xxx}`（独立行或行内），自动将新页中的标题改写为 `{title: xxx (fork)}`；若为空标题或无标题则原样复制。底层 `PromptPage.name` 使用 `next_default_page_name` 生成 `Page N`。<br>4. **位置与跳转**：追加到当前 Prompt 页面列表末尾，创建后立即切换到新页，光标置于文末，弹出 Default Toast `"Page forked"`。 |
| **右键菜单项与顺序** | 1. **`PromptMode::Editor` 右键菜单**：<br>依次为：`Select All` → `Copy` → `Rename Page` → `New Page` → `Fork Page` → `Delete`（仅当 `pages.len() > 1` 时显示 `Delete`，点击后进入行内红色 `"Confirm Delete"` 二次确认）。<br>2. **`PromptMode::Pages` 页行右键菜单**：<br>依次为：`Open` → `Rename Page` → `Copy` → `New Page` → `Fork Page` → `Delete`（仅当 `pages.len() > 1` 时显示 `Delete`）。在 `Pages` 模式下点击 `Rename Page` 会先打开该页进入 `Editor` 模式，再滚回顶部并选中 `{title: ...}`。 |
| **移除 `[all]` 与迁移 `Pin` 按钮** | 1. **移除 `[all]`**：从工具栏移除 `PromptToolbarButton::SelectAll` 及相关常量/Tooltip；保留 `AppState::select_all_prompt()` 供右键菜单调用。<br>2. **迁移 `Pin` 到底边框**：从 `PromptToolbarButton` 移除 `Pin`；在 `src/ui/controls.rs` 提供 `prompt_pin_button(panel, pinned)` 与 `prompt_pin_at(view, state, col, row)`，绘制在 `view.prompt` 底边框折叠按钮左侧（间隔 1 列 `─`），文案为 `PROMPT_UNPINNED_LABEL = "[pin]"`（5 列）与 `PROMPT_PINNED_LABEL = "[unpin]"`（7 列），样式为 `style::accent()`；`src/ui/right_panel.rs` 底边框身份标签自动避让 `prompt_pin_button`。<br>3. **工具栏精简后阈值**：`Editor` 视图左组为 `[≡] [←] [→]`（11 列），右组为 `[refDir] ●`（`[refDir]` 直接位于保存圆点 `●` 左侧 1 列，`PROMPT_FOLDERS_MIN_WIDTH_EDITOR` 由 32 列大幅降至 **22 列**）；`Tree` 视图为 `[/]` + `[tmp]`；`Pages` 视图为 `[/]`。 |

---

## 3. 核心架构与数据流

```text
┌──────────────────────────────────────────────────────────────────────────────┐
│ 1. 快捷键链路 (Cmd/Ctrl + Shift + Right)                                     │
│   input::editor_command() -> EditorCommand::NewPage                          │
│   app::update::editor::apply_editor()                                        │
│     └─ advance_or_create_prompt_page(state)                                  │
│          ├─ active_index + 1 < pages.len() ──► switch_prompt_page(state, 1)  │
│          └─ active_index + 1 == pages.len() ─► create_or_toast_new_page()    │
├──────────────────────────────────────────────────────────────────────────────┤
│ 2. 编辑页右键菜单链路 (MouseButton::Right in PromptMode::Editor)              │
│   event::mouse::press::handle_mouse_down_right()                             │
│     └─ event::overlay::open_right_click_menu()                               │
│          └─ update::open_prompt_editor_menu(state, (col, row))               │
│               └─ Overlay::Menu (OverlayTarget::PromptPage)                   │
│   app::update::menu::activate_menu()                                         │
│     ├─ MenuCommand::SelectAll        ─► state.select_all_prompt()            │
│     ├─ MenuCommand::CopyPageContent  ─► commit + copy_page_content(...)      │
│     ├─ MenuCommand::RenamePage       ─► rename_prompt_page_title(...)        │
│     ├─ MenuCommand::NewPage          ─► create_or_toast_new_page(state)      │
│     ├─ MenuCommand::ForkPage         ─► fork_prompt_page(...)                │
│     └─ MenuCommand::DeletePage       ─► confirming ─► perform_delete_page()  │
├──────────────────────────────────────────────────────────────────────────────┤
│ 3. 底边框 Pin 按钮几何、渲染与点击链路                                        │
│   ui::controls::prompt_pin_button(view.prompt, pinned)                       │
│     ├─ right_panel::render_bottom() ──► 身份标签右边界避让 pin.x - 1          │
│     ├─ controls::render_panel_buttons() ──► 绘制 [pin] / [unpin] (accent)    │
│     ├─ ui::tooltip::collect_targets() ──► TooltipTarget::PromptPin           │
│     └─ event::mouse::press::handle_mouse_down_left()                         │
│          └─ ui::prompt_pin_at() ──► update::toggle_prompt_pin(state)         │
└──────────────────────────────────────────────────────────────────────────────┘
```

---

## 4. 详细技术设计与关键代码

### 4.1 调整新建 Page 快捷键逻辑（`Cmd/Ctrl + Shift + →`）

#### 关键点
- `src/input/mod.rs` 保持将 `Cmd/Ctrl + Shift + Right` 映射为 `EditorCommand::NewPage`。
- 在 `src/app/update/prompt/pages.rs` 新增 `advance_or_create_prompt_page(state: &mut AppState) -> bool`：
  1. 通过 `displayed_prompt_pages(state)` 获取 `(pages, active)`。
  2. 计算当前页索引 `index = pages.iter().position(|p| p.id == active).unwrap_or(0)`。
  3. 若 `index + 1 < pages.len()`，清理补全面板并调用 `switch_prompt_page(state, 1)` 切换到下一页。
  4. 若 `index + 1 >= pages.len()`（即处于最后一页），调用 `create_or_toast_new_page(state)`。
- 在 `src/app/update/editor.rs` 中将 `EditorCommand::NewPage` 的处理改为调用 `super::advance_or_create_prompt_page(state)`。

#### 关键代码（`src/app/update/prompt/pages.rs` & `src/app/update/editor.rs`）

```rust
// src/app/update/prompt/pages.rs

/// `Ctrl/Cmd+Shift+→`：非末页时切换到下一页；末页时新建页（或跳转到已有空白页并 Toast）。
pub fn advance_or_create_prompt_page(state: &mut AppState) -> bool {
    let Some((pages, active)) = displayed_prompt_pages(state) else {
        return false;
    };
    let index = pages.iter().position(|page| page.id == active).unwrap_or(0);
    if index + 1 < pages.len() {
        state.prompt.clear_panel();
        return switch_prompt_page(state, 1);
    }
    create_or_toast_new_page(state)
}
```

```rust
// src/app/update/editor.rs
EditorCommand::NewPage => {
    super::advance_or_create_prompt_page(state);
}
```

---

### 4.2 移除工具栏 `[all]` 与 `Pin`，并将 `Pin` 移至底边框折叠按钮左侧

#### 关键点
1. **`src/ui/text.rs`**：
   - 删除 `PROMPT_SELECT_ALL_LABEL` 及 `TooltipTarget::PromptSelectAll` 映射。
   - 将钉住按钮文案更新为英文（移除圆形符号）：
     ```rust
     /// prompt 底边钉住按钮：未钉显示 `[pin]`、已钉显示 `[unpin]`。
     pub const PROMPT_UNPINNED_LABEL: &str = "[pin]";
     pub const PROMPT_PINNED_LABEL: &str = "[unpin]";
     ```
2. **`src/ui/controls.rs` & `src/ui/right_panel.rs`**：
   - 在 `src/ui/controls.rs` 新增 `prompt_pin_button(panel: Rect, pinned: bool) -> Option<Rect>` 与 `prompt_pin_at(view: &layout::ViewLayout, state: &AppState, column: u16, row: u16) -> bool`：
     - 展开态（`panel.width > COLLAPSED_STRIP && panel.height > 0`）下位于 `panel.bottom() - 1` 行；
     - 折叠按钮起始列为 `collapse_x = panel.right().saturating_sub(PANEL_BUTTON_WIDTH + PANEL_BUTTON_MARGIN)`；
     - Pin 按钮宽度为 `label.chars().count() as u16`（未钉 `[pin]` 为 5，已钉 `[unpin]` 为 7），起始列 `x = collapse_x.checked_sub(width + 1)?`，需满足 `x > panel.x`（不压左边框角 `╰`）。
   - 在 `render_panel_buttons` 末尾调用 `render_prompt_pin(frame, view, state)`，以 `style::accent()` 绘制 `[pin]` 或 `[unpin]`。
   - 在 `src/ui/right_panel.rs` 的 `render_bottom` 中，右端边界 `end` 优先避让 `prompt_pin_button(area, state.prompt_pinned.is_some())`（取 `pin.x.saturating_sub(1)`），放不下 Pin 时回退避让折叠按钮。
3. **`src/layout/prompt.rs` & `src/layout.rs`**：
   - 从 `PromptToolbarButton` 枚举、`prompt_toolbar_button_rect`、`prompt_toolbar_button_at` 中移除 `SelectAll` 和 `Pin`。
   - 删除 `PROMPT_SELECT_ALL_WIDTH`、`PROMPT_SELECT_ALL_MIN_WIDTH`、`PROMPT_PIN_WIDTH`、`PROMPT_PIN_MIN_WIDTH_EDITOR`、`PROMPT_PIN_MIN_WIDTH_TREE`、`PROMPT_PIN_MIN_WIDTH_PAGES`（以及 `src/layout.rs` 中对 `PROMPT_PIN_MIN_WIDTH_EDITOR` 和 `PROMPT_SELECT_ALL_MIN_WIDTH` 的测试重导出）。
   - 编辑视图左组 `[≡] [←] [→]` 占 `3 + 1 + 3 + 1 + 3 = 11` 列；右侧 `[refDir]`（8 列）直接位于保存圆点 `●`（1 列）左侧 1 列：
     - `PROMPT_FOLDERS_MIN_WIDTH_EDITOR` 直接由左组 + 间距 + `[refDir]` + 间距 + 保存点推导：
       ```rust
       /// 编辑视图左组 `[≡] [←] [→]` 所需的最小内容区宽度（列）：左组总宽 11 + 1 列间隔 + 1 列保存点 = 13。
       const PROMPT_LEFT_GROUP_MIN_WIDTH: u16 = PROMPT_TOOLBAR_BUTTON_WIDTH
           + PROMPT_TOOLBAR_BUTTON_GAP
           + PROMPT_UNDO_REDO_WIDTH
           + PROMPT_TOOLBAR_BUTTON_GAP
           + PROMPT_UNDO_REDO_WIDTH
           + PROMPT_TOOLBAR_BUTTON_GAP
           + 1;
       /// 显示引用文件夹按钮所需的最小内容区宽度（列）：在保存点左侧让出宽度与间距（13 + 1 + 8 = 22）。
       pub(crate) const PROMPT_FOLDERS_MIN_WIDTH_EDITOR: u16 =
           PROMPT_LEFT_GROUP_MIN_WIDTH + PROMPT_TOOLBAR_BUTTON_GAP + PROMPT_FOLDERS_WIDTH;
       ```
     - `PromptToolbarButton::Folders` 的矩形直接锚定在保存点左侧：
       ```rust
       PromptToolbarButton::Folders => {
           if mode != PromptMode::Editor || inner.width < PROMPT_FOLDERS_MIN_WIDTH_EDITOR {
               return None;
           }
           Rect::new(
               bar.right()
                   .saturating_sub(1 + PROMPT_TOOLBAR_BUTTON_GAP + PROMPT_FOLDERS_WIDTH),
               bar.y,
               PROMPT_FOLDERS_WIDTH,
               1,
           )
       }
       ```
4. **`src/ui/prompt.rs` & `src/ui/prompt/header.rs` & `src/ui/prompt_frame.rs`**：
   - `render_toolbar` 移除 `Pin` 与 `SelectAll`（并可移除不再使用的 `pinned: bool` 参数或保持干净签名）。
5. **`src/event/mouse/press.rs` & `src/ui/tooltip.rs`**：
   - `src/event/mouse/press.rs`：在 `collapse_button_at` 旁新增 `if ui::prompt_pin_at(view, state, mouse.column, mouse.row)` 分支，调用 `update::toggle_prompt_pin(state)` 并置 `*dirty = true`；移除工具栏中的 `Pin` 与 `SelectAll` 分支。
   - `src/ui/tooltip.rs`：将 `TooltipTarget::PromptPin` 改为绑定 `ui::prompt_pin_button(view.prompt, state.prompt_pinned.is_some())`（当 `!state.prompt_collapsed` 时在 Prompt 与 Todo 标签页下均生效），移除 `TooltipTarget::PromptSelectAll`。

#### 关键代码（`src/ui/controls.rs` & `src/ui/right_panel.rs`）

```rust
// src/ui/controls.rs

/// prompt 底边钉住按钮：展开态位于底边折叠按钮左侧（间隔 1 列 `─`）；折叠态或空间不足返回 None。
///
/// 标签宽度随钉住态变化（未钉 `[pin]` 5 列 / 已钉 `[unpin]` 7 列），右缘固定贴折叠按钮左侧。
pub fn prompt_pin_button(panel: Rect, pinned: bool) -> Option<Rect> {
    if panel.width <= COLLAPSED_STRIP || panel.height == 0 {
        return None;
    }
    let label = if pinned {
        text::PROMPT_PINNED_LABEL
    } else {
        text::PROMPT_UNPINNED_LABEL
    };
    let width = u16::try_from(label.chars().count()).unwrap_or(u16::MAX);
    let collapse_x = panel
        .right()
        .saturating_sub(PANEL_BUTTON_WIDTH + PANEL_BUTTON_MARGIN);
    let x = collapse_x.checked_sub(width + 1)?;
    if x <= panel.x {
        return None;
    }
    Some(Rect::new(x, panel.bottom().saturating_sub(1), width, 1))
}

/// 钉住按钮命中：坐标是否落在右侧容器底边钉住按钮内。
pub fn prompt_pin_at(view: &layout::ViewLayout, state: &AppState, column: u16, row: u16) -> bool {
    !state.prompt_collapsed
        && prompt_pin_button(view.prompt, state.prompt_pinned.is_some())
            .is_some_and(|area| area.contains((column, row).into()))
}

fn render_prompt_pin(frame: &mut Frame<'_>, view: &layout::ViewLayout, state: &AppState) {
    if state.prompt_collapsed {
        return;
    }
    let pinned = state.prompt_pinned.is_some();
    let Some(area) = prompt_pin_button(view.prompt, pinned) else {
        return;
    };
    let label = if pinned {
        text::PROMPT_PINNED_LABEL
    } else {
        text::PROMPT_UNPINNED_LABEL
    };
    for (offset, symbol) in label.chars().enumerate() {
        let x = area.x + offset as u16;
        if let Some(cell) = frame.buffer_mut().cell_mut((x, area.y)) {
            cell.reset();
            cell.set_char(symbol);
            cell.set_style(style::accent());
        }
    }
}
```

```rust
// src/ui/right_panel.rs (render_bottom 避让钉住按钮)
let end = super::controls::prompt_pin_button(area, state.prompt_pinned.is_some()).map_or(
    area.right()
        .saturating_sub(PANEL_BUTTON_WIDTH + PANEL_BUTTON_MARGIN),
    |pin| pin.x.saturating_sub(1),
);
```

---

### 4.3 右键菜单扩展（`OverlayTarget`、`MenuCommand` 与鼠标命中）

#### 关键点
1. **命令定义（`src/app/overlay.rs`）**：
   - 在 `MenuCommand` 中新增：
     - `SelectAll`：全选当前页正文（仅在 `PromptMode::Editor` 右键菜单中出现）。
     - `RenamePage`：重命名页标题（滚到顶部并选中 `{title: ...}`）。
     - `ForkPage`：基于目标页克隆新页并切入。
2. **鼠标右键命中（`src/event/overlay.rs`）**：
   - 在 `open_right_click_menu` 中，当 `update::prompt_panel_active(state)` 且处于 `PromptMode::Editor`（即非 `prompt_tree_mode` 且非 `prompt_pages_mode`）时：
     - 检查右键坐标 `(column, row)` 是否落在 `paste::prompt_text_area(view)` 内。
     - 若命中：先调用 `state.prompt.clear_panel()` 关闭可能打开的 `#` / `/` / `@` 补全面板（**不清空 `state.selection`**），然后调用 `update::open_prompt_editor_menu(state, (column, row))` 打开编辑页右键菜单并返回 `true`。
3. **菜单组装函数（`src/app/update/prompt/pages_view.rs`）**：
   - 新增 `open_prompt_editor_menu(state: &mut AppState, anchor: (u16, u16))`：
     - 组装命令：`[SelectAll, CopyPageContent, RenamePage, NewPage, ForkPage]`，当 `page_count > 1` 时追加 `DeletePage`。
   - 更新 `open_page_menu(state: &mut AppState, page: PromptPageId, anchor: (u16, u16))`（`PromptMode::Pages` 行右键菜单）：
     - 组装命令：`[OpenPage, RenamePage, CopyPageContent, NewPage, ForkPage]`，当 `page_count > 1` 时追加 `DeletePage`。

#### 关键代码（`src/app/overlay.rs`、`src/event/overlay.rs`、`src/app/update/prompt/pages_view.rs`）

```rust
// src/app/overlay.rs
pub enum MenuCommand {
    // ... 现有命令 ...
    OpenPage,
    SelectAll,
    RenamePage,
    NewPage,
    ForkPage,
    DeletePage,
}
```

```rust
// src/event/overlay.rs (open_right_click_menu 内新增 Editor 分支)
if update::prompt_panel_active(state)
    && !update::prompt_tree_mode(state)
    && !update::prompt_pages_mode(state)
{
    let area = paste::prompt_text_area(view);
    if area.contains((column, row).into()) {
        state.prompt.clear_panel();
        update::open_prompt_editor_menu(state, (column, row));
        return true;
    }
}
```

```rust
// src/app/update/prompt/pages_view.rs

/// 编辑视图正文区右键菜单：全选、复制全文、重命名页、新建页、Fork 页、删除页（仅多页时）。
pub fn open_prompt_editor_menu(state: &mut AppState, anchor: (u16, u16)) {
    let workspace = state.prompt_workspace();
    let Some((owner, page, page_count)) = state.workspaces.get(workspace).and_then(|ws| {
        let owner = ws.prompt_panel.editing?;
        let (pages, active) = ws.prompt_panel.editing_pages()?;
        let page = pages
            .iter()
            .find(|candidate| candidate.id == active)
            .or_else(|| pages.first())?
            .id;
        Some((owner, page, pages.len()))
    }) else {
        return;
    };
    let mut commands = vec![
        MenuCommand::SelectAll,
        MenuCommand::CopyPageContent,
        MenuCommand::RenamePage,
        MenuCommand::NewPage,
        MenuCommand::ForkPage,
    ];
    if page_count > 1 {
        commands.push(MenuCommand::DeletePage);
    }
    state.overlay = Some(Overlay::Menu(Menu {
        anchor,
        target: OverlayTarget::PromptPage {
            workspace,
            owner,
            page,
        },
        commands,
        selected: 0,
        confirming: false,
    }));
}
```

---

### 4.4 「重命名 Page」：滚动到顶部并选中 `{title: ...}` 内容

#### 关键点
1. **单字符与多字符选区的底层细节（关键陷阱规避）**：
   - 在 `src/app/selection.rs` 中，`Selection::range()` 规定：当 `self.unit == SelectionUnit::Character && self.anchor == self.cursor` 时返回 `None`。
   - 同时 `src/app/update/selection.rs` 中 `prompt_selection_bounds` 和 `prompt_selection_text` 仅当 `sel.unit() != SelectionUnit::Character` 时才将 `allow_single_cell` 设为 `true`。
   - **解决方案**：在 `Selection` 新增构造函数 `Selection::programmatic_range(pane: PaneId, start: (i32, u16), end: (i32, u16)) -> Self`，内部设置 `unit: SelectionUnit::Word, dragging: false`。这样无论是 1 列宽字符（`{title: A}`）还是多字符标题（`{title: 登录接口重构}`），反白渲染、`Ctrl/Cmd+C` 复制、以及直接打字替换选区（`delete_prompt_selection`）都能 100% 正常工作。
2. **模板标题解析与定位（`src/app/markdown/template.rs`）**：
   - 新增纯函数 `find_first_template_title_span(text: &str) -> TemplateTitleLookup`：
     - 优先检查首个 `BlockKind::Template` 起始行下一行是否为独立 `{title: ...}` 行；
     - 其次检查起始行自身是否包含内联 `{title: ...}`；
     - 若有模板块但无 `{title:}`，返回 `MissingTitleLine { start_line_end }`；
     - 若无模板块，返回 `NoTemplateBlock`。
3. **字节区间转视觉行单元格坐标（`src/app/prompt/viewport.rs`）**：
   - 在 `Prompt` 上新增 `pub fn cell_range_for_byte_span(&self, start_byte: usize, end_byte: usize) -> Option<((i32, u16), (i32, u16))>`，显式跳过 `row.top_border == true` 的虚拟顶边框行。
4. **执行重命名页流程（`src/app/update/prompt/pages.rs`）**：
   - 函数签名：`pub fn rename_prompt_page_title(state: &mut AppState, workspace: usize, owner: PromptEditing, page: PromptPageId) -> bool`
   - 若处于 `PromptMode::Pages`，先 `hide_prompt_pages(state)`；若 `page != active`，先 `select_prompt_page(state, page)`。
   - 调用 `focus_prompt(state)` 聚焦编辑器并关闭补全面板。
   - 根据 `markdown::find_first_template_title_span(state.prompt.text())` 分支执行滚回顶部（`scroll_to(0)`）与选中/光标定位/补入 `{title: }` 或弹 Warning Toast `text::TOAST_PAGE_NO_TITLE`。

#### 关键代码（`src/app/markdown/template.rs` & `src/app/prompt/viewport.rs`）

```rust
// src/app/markdown/template.rs

/// 模板块标题定位结果（供 Rename Page 定位与选中）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TemplateTitleLookup {
    Found { value_start: usize, value_end: usize },
    MissingTitleLine { start_line_end: usize },
    NoTemplateBlock,
}

/// 在正文中查找首个 `&&&` 模板块的 `{title: ...}` 值字节范围。
pub fn find_first_template_title_span(text: &str) -> TemplateTitleLookup {
    let scan = super::blocks::block_scan(text);
    let Some(span) = scan.spans.iter().find(|s| s.kind == super::BlockKind::Template) else {
        return TemplateTitleLookup::NoTemplateBlock;
    };
    let lines: Vec<&str> = text.split('\n').collect();
    let mut offsets = Vec::with_capacity(lines.len());
    let mut offset = 0;
    for line in &lines {
        offsets.push(offset);
        offset += line.len() + 1;
    }
    let Some(start_line) = lines.get(span.start).copied() else {
        return TemplateTitleLookup::NoTemplateBlock;
    };
    let start_line_offset = offsets[span.start];

    // 1. 优先检查下一行是否为独立 `{title: ...}` 行
    if let Some(next_line) = lines.get(span.start + 1).copied()
        && is_title_line(next_line)
    {
        let line_offset = offsets[span.start + 1];
        if let Some(range) = title_value_byte_range(next_line, line_offset) {
            return TemplateTitleLookup::Found {
                value_start: range.0,
                value_end: range.1,
            };
        }
    }

    // 2. 其次检查起始行内联 `{title: ...}`
    if parse_template_start_line(start_line).and_then(|s| s.title).is_some()
        && let Some(range) = title_value_byte_range(start_line, start_line_offset)
    {
        return TemplateTitleLookup::Found {
            value_start: range.0,
            value_end: range.1,
        };
    }

    TemplateTitleLookup::MissingTitleLine {
        start_line_end: start_line_offset + start_line.len(),
    }
}
```

---

### 4.5 「Fork Page」：带正文与标题后缀 `(fork)` 克隆页面

#### 关键点
1. **标题追加 `(fork)` 辅助函数（`src/app/markdown/template.rs`）**：
   - 新增 `pub fn fork_page_content(content: &str) -> String`：当 `find_first_template_title_span(content)` 返回非空 `Found { value_start, value_end }` 时，在 `value_end` 处插入 `" (fork)"`；否则原样返回 `content.to_string()`。
2. **统一 `Effect::CreatePage` 与 `Database::create_prompt_page` 支持初始正文**：
   - `src/app/effects.rs`：为 `Effect::CreatePage` 增加 `content: String` 字段。
   - `src/db/mod.rs`：`pub fn create_prompt_page(&self, prompt_id: i64, name: &str, content: &str) -> Result<i64>`。
   - `src/db/service.rs`：`Effect::CreatePage` 透传 `&content`。
3. **执行 `fork_prompt_page`（`src/app/update/prompt/pages.rs`）**：
   - 先 `commit_editing_prompt(state)`；若源页为空白页则调用 `create_or_toast_new_page(state)`；否则生成 `forked_content`、退出 `Pages` 模式、调用 `open_new_page_with_content(state, forked_content)` 并弹出 Default Toast `text::TOAST_PAGE_FORKED`（`"Page forked"`）。

---

### 4.6 文案常量汇总（`src/ui/text.rs`）

- **移除**：
  - `pub const PROMPT_SELECT_ALL_LABEL: &str = "[all]";`
  - `TooltipTarget::PromptSelectAll => "select all"`
- **更新**：
  - `pub const PROMPT_UNPINNED_LABEL: &str = "[pin]";`（原 `"[○]"`）
  - `pub const PROMPT_PINNED_LABEL: &str = "[unpin]";`（原 `"[◉]"`）
  - `pub const MENU_PAGE_NEW: &str = "New Page";`
- **新增**：
  - `pub const MENU_PAGE_SELECT_ALL: &str = "Select All";`
  - `pub const MENU_PAGE_RENAME: &str = "Rename Page";`
  - `pub const MENU_PAGE_FORK: &str = "Fork Page";`
  - `pub const TOAST_PAGE_FORKED: &str = "Page forked";`
  - `pub const TOAST_PAGE_NO_TITLE: &str = "No {title:} found in current page";`

---

## 5. 影响文件清单与模块边界检查

| 文件路径 | 修改职责 | 预计改动规模 |
| --- | --- | --- |
| `src/layout/prompt.rs` & `src/layout.rs` | 移除 `PromptToolbarButton::{SelectAll, Pin}` 与相关宽度常量；更新 `PROMPT_FOLDERS_MIN_WIDTH_EDITOR` 为 22 并锚定在保存点左侧 | ~-55 / +12 行 |
| `src/ui/controls.rs` | 新增 `prompt_pin_button`、`prompt_pin_at`、`render_prompt_pin`（底边框折叠按钮左侧绘制 `[pin]`/`[unpin]`） | ~+50 行 |
| `src/ui/right_panel.rs` | `render_bottom` 身份标签右端避让 `prompt_pin_button` | ~+6 / -3 行 |
| `src/ui/prompt.rs` & `src/ui/prompt/header.rs` & `src/ui/prompt_frame.rs` | 工具栏移除 `SelectAll` 与 `Pin` 渲染 | ~-20 行 |
| `src/app/tooltip.rs` & `src/ui/tooltip.rs` | 移除 `TooltipTarget::PromptSelectAll`，将 `TooltipTarget::PromptPin` 改为绑定底边 `ui::prompt_pin_button` | ~-6 / +6 行 |
| `src/ui/text.rs` | 移除 `PROMPT_SELECT_ALL_LABEL`；更新 `PROMPT_UNPINNED_LABEL` / `PROMPT_PINNED_LABEL` 为 `[pin]` / `[unpin]`；新增菜单与 Toast 常量 | ~+10 行 |
| `src/app/overlay.rs` & `src/ui/overlay/menu.rs` | `MenuCommand` 新增 `SelectAll`、`RenamePage`、`ForkPage` 并映射文案 | ~+12 行 |
| `src/event/overlay.rs` | `open_right_click_menu` 新增 `PromptMode::Editor` 内容区右键分支 | ~+12 行 |
| `src/event/mouse/press.rs` | 左键点击新增 `ui::prompt_pin_at` 分支，移除工具栏 `Pin` 与 `SelectAll` 分支 | ~+6 / -6 行 |
| `src/app/selection.rs` | 新增 `Selection::programmatic_range` 支持包含单字符的静态选区 | ~+14 行 |
| `src/app/prompt/viewport.rs` & `src/app/prompt.rs` | 新增 `Prompt::cell_range_for_byte_span` 与 `Prompt::insert_template_title_line` | ~+55 行 |
| `src/app/markdown/template.rs` & `src/app/markdown/mod.rs` | 新增 `TemplateTitleLookup`、`find_first_template_title_span`、`fork_page_content` | ~+75 行 |
| `src/app/effects.rs`、`src/db/mod.rs`、`src/db/service.rs` | `Effect::CreatePage` 与 `create_prompt_page` 支持初始 `content` | ~+8 行 |
| `src/app/update/editor.rs`、`src/app/update/prompt/pages.rs`、`src/app/update/prompt/pages_view.rs`、`src/app/update/menu.rs` | 实现 `advance_or_create_prompt_page`、`rename_prompt_page_title`、`fork_prompt_page`、`open_prompt_editor_menu` 及菜单分派 | ~+180 行 |
| `docs/standards/tui-design-requirements.md` | 同步更新工具栏与底边框钉住按钮规范说明 | ~+4 / -4 行 |

---

## 6. Code Review 补充设计与边界收敛（详见 `review.md`）

1. **[P0-1] 编辑页右键与页菜单动作自动聚焦 Prompt 面板**：
   - 在 `src/event/overlay.rs` 的 `PromptMode::Editor` 右键分支中，若未命中已打开的 `#`/`/`/`@` 补全面板（`!prompt_command_panel_contains`），先调用 `update::focus_prompt(state)` 聚焦编辑器并调用 `state.prompt.dismiss_completion_panels()`（不回滚粘贴提案），再打开页菜单。
   - 在 `src/app/update/menu.rs::activate_menu` 的 `OverlayTarget::PromptPage` 分支中，对 `SelectAll`、`OpenPage`、`NewPage`、`ForkPage`、`DeletePage`（当处于 `PromptMode::Editor` 时）确保调用 `focus_prompt(state)`，保证全选或切页后打字/退格/`Ctrl+C` 作用于 Prompt 而非终端 PTY。
2. **[P0-2] 共享 `repo_key` 后台停泊工作区的 `Workspace.prompt` 缓冲同步刷新**：
   - 在 `src/app/update/prompt.rs::apply_prompt_tree` 中，当 `!displayed`（工作区处于后台停泊态）且 `editing == Some(PromptEditing::Item(id))` 时，若 `page_switched` 或新树中当前 `active_page` 的正文发生变化且停泊缓冲 `draft.editor.is_saved()`，同步将 `workspaces[index].prompt` 停泊缓冲刷新为新 `active_page` 的正文并 `mark_saved()`，防止切回该工作区再切走时将旧页缓冲覆盖写回新页。
3. **[P1-1] `Rename Page` 超首屏标题可见性保底与尾随零宽字符（VS16/ZWJ）选区修正**：
   - `Prompt::cell_range_for_byte_span` 逆序查找 `start_byte..end_byte` 内最后一个显示列宽 `> 0` 的字符计算 `end_col`，防止 Emoji 变体选择符（`U+FE0F`，列宽为 `0`）把 `end_col` 推到右花括号 `}` 上导致替换被拒。
   - `rename_prompt_page_title` 先执行 `state.prompt.scroll_to(0)`，当目标标题行位于首屏内时保持 `scroll == 0`，当首个模板块前有超过一屏的前置文本时通过 `set_cursor_offset` 自动将标题滚入可见视口。
4. **[P1-2] `Prompt::dismiss_completion_panels` 与滚动条列右键覆盖**：
   - 新增 `Prompt::dismiss_completion_panels(&mut self)`（仅清理 `panel`/`slash`/`mention`/`refer`，不触发 `paste_cancel()`），避免右键菜单误回滚未确认的粘贴内容；
   - 编辑页右键命中区域使用 `layout::prompt_body_rect(layout::pane_inner_rect(view.prompt))` 覆盖右侧滚动条列，且 `open_prompt_editor_menu` 返回 `bool`。
5. **[P2-1 & P2-2] 去重 `SavePromptContent`、守卫补齐、死参数清理与测试拆分**：
   - `open_new_page_with_content` 仅在 `state.prompt.is_dirty()` 时调用 `commit_editing_prompt`，消除 `Fork Page` / `New Page` 重复产生两条相同 `SavePromptContent` 的问题；
   - `rename_prompt_page_title` 与 `fork_prompt_page` 校验 `ws.prompt_panel.editing == Some(owner)`，且 `insert_template_title_line` 继承模板起始行前导空格缩进；
   - 移除 `render_header` 与 `render_toolbar` 未使用的 `_pinned: bool` 参数；将模板单元测试拆分为 `src/app/markdown/template/tests.rs`，使 `src/app/markdown/tests.rs` 控制在 1500 行上限以内。
