# Prompt 编辑页快捷键调整、右键菜单与工具栏/底边按钮优化代码审查 (Code Review)

> **状态**：**全部 P0–P2 项已修复验证并合入 `dev` 分支**（2026-11-02）
> **原评审工作区**：`.worktrees/prompt-page-menu-and-shortcut`（分支 `feat/prompt-page-menu-and-shortcut`，已合并至 `dev` 并清理）
> **关联文档**：`docs/feature/prompt-page-menu-and-shortcut/design.md`、`docs/feature/prompt-page-menu-and-shortcut/task.md`

本次 Review 覆盖工作区分支 `feat/prompt-page-menu-and-shortcut` 引入的全部改动（`Cmd/Ctrl + Shift + →` 切页/末页新建、编辑页与页列表右键菜单 `Select All / Copy / Rename Page / New Page / Fork Page / Delete`、移除工具栏 `[all]` 并将 `Pin` 按钮移至底边框折叠按钮左侧改为英文 `[pin]`/`[unpin]`）。

审查发现的问题按照严重级别从高到低（`P0` → `P1` → `P2`）归类如下，明确记录**关键点**、**错误原因**与**修复方案**。

---

## 缺陷清单与优先级排序

### [P0-1] 编辑页右键菜单及页菜单动作（`Select All` / `New Page` / `Fork Page` / `Delete` / `Open`）未将焦点切至 Prompt 面板，导致全选或切页后打字/退格/`Ctrl+C` 误发往终端 PTY

- **关键点**：
  - `src/event/overlay.rs`：`open_right_click_menu`（第 `545–555` 行）
  - `src/app/update/menu.rs`：`activate_menu` 中 `OverlayTarget::PromptPage` 分支（第 `480–533` 行）
  - `src/app/update/prompt/pages.rs`：`open_new_page_with_content`（Temp 分支）、`create_or_toast_new_page`、`perform_delete_page`
- **错误原因**：
  1. 左键点击 Prompt 正文区会调用 `update::focus_prompt(state)`，但右键打开编辑页菜单时未切换 `state.focused`；
  2. 当焦点原本在 `FocusedContainer::Panes`（终端）、`Sidebar` 或 `Todo` 时，用户右键点击 Prompt 编辑页并选择 `Select All`，虽然编辑器正文反白高亮，但 `state.focused` 仍停留在旧容器（如终端）；
  3. 随后用户直接打字替换、按 `Backspace` 删除选区或按 `Ctrl+C` 复制时，`input::route` 按 `state.focused` 将按键编码进终端 PTY（甚至将 `Ctrl+C` 作为 `SIGINT` 发送给终端进程），而不是操作 Prompt 选区；同理，Temp 提示词在右键执行 `New Page` / `Fork Page` / `Delete` 后焦点也未切入编辑器。
- **修复方案**：
  1. 在 `src/event/overlay.rs` 编辑页右键分支中，打开编辑页菜单时同步调用 `update::focus_prompt(state)` 聚焦 Prompt 面板；
  2. 在 `src/app/update/menu.rs::activate_menu` 的 `OverlayTarget::PromptPage` 分支中，对 `SelectAll`、`OpenPage`、`NewPage`、`ForkPage`、`DeletePage`（当处于或切入 `PromptMode::Editor` 时）确保调用 `focus_prompt(state)`。

---

### [P0-2] 共享同一 `repo_key` 的多工作区（主 checkout + linked worktree）在切页/新建页/Fork 页后，后台停泊工作区的 `Workspace.prompt` 缓冲未随新 `active_page` 刷新，切回后会将旧页正文覆盖写进新页

- **关键点**：
  - `src/app/update/prompt.rs`：`apply_prompt_tree`（第 `76–79`、`150–154` 行）
  - `src/app/update/editor.rs`：`swap_display_prompt`、`restore_prompt_view`（第 `325–381` 行）
- **错误原因**：
  1. 主工作区 A 与 linked worktree B 共享同一棵提示词树（相同 `repo_key`）。当 B 曾打开条目 `I` 的第 1 页 `P1` 并切走到 A 时，B 的编辑器缓冲（含 `P1` 正文，已 `mark_saved`）停泊在 `workspaces[B].prompt` 中；
  2. 在 A 中执行 `Fork Page`、`New Page` 或切换页面后，`Effect::CreatePage` / `Effect::SetActivePage` 更新了 DB 的 `active_page_id` 并对所有同 `repo_key` 工作区广播 `apply_prompt_tree`；
  3. `apply_prompt_tree` 仅在 `displayed == true`（当前显示的工作区 A）时按新 `active_page` 重载 `state.prompt`，对后台停泊工作区 B 仅替换了 `prompt_panel.tree`（使 B 中条目 `I` 的 `active_page` 变成了新页 `P2`），却未同步刷新 `workspaces[B].prompt` 中的停泊缓冲；
  4. 当用户从 A 切到 B 再切回 A（或切到 B 后编辑）时，`swap_display_prompt` 调用 `commit_prompt_buffer(state, B)`，将 B 停泊缓冲中的旧页 `P1` 正文直接按新的 `item.active_page`（`P2`）写入并触发 `Effect::SavePromptContent`，**静默覆盖掉刚 Fork 或新建的页面内容**。
- **修复方案**：
  - 在 `src/app/update/prompt.rs::apply_prompt_tree` 中，当 `!displayed`（工作区处于后台停泊态）且 `editing == Some(PromptEditing::Item(id))` 时，若 `page_switched` 或新树中当前页正文已更新且停泊缓冲 `draft.editor.is_saved()`，同步将 `workspaces[index].prompt` 停泊缓冲刷新为新 `active_page` 的正文并 `mark_saved()`。

---

### [P1-1] `Rename Page` 在首个模板块不在首屏时无条件执行 `scroll_to(0)` 把刚选中的 `{title: ...}` 滚出视口，且标题末尾含零宽字符（VS16/ZWJ emoji）时 `cell_range_for_byte_span` 终点越过 `}` 导致打字替换被静默拒绝

- **关键点**：
  - `src/app/update/prompt/pages.rs`：`rename_prompt_page_title`（第 `384–415` 行）
  - `src/app/prompt/viewport.rs`：`cell_range_for_byte_span`（第 `219–252` 行）
- **错误原因**：
  1. **长前置文本下滚出视口**：`set_cursor_offset` 内部已通过 `settle() -> scroll_cursor_into_view()` 将标题行滚入视口；随后无条件调用 `state.prompt.scroll_to(0)`。若首个 `&&&` 模板块上方有超过一屏的普通文本（标题视觉行 `>= height`），`scroll_to(0)` 会把刚选中的 `{title: ...}`（或刚补入的 `{title: }`）滚出屏幕下方，用户看不见选区与光标；
  2. **标题末尾零宽字符越界**：`cell_range_for_byte_span` 计算末字符列坐标时使用了 `ch_width.max(1) - 1`。当标题末尾是带变体选择符的 Emoji（如 `❤️`，末字符 `U+FE0F` 显示宽度为 `0`）时，`.max(1)` 将其算作 1 列，导致 `end_col` 落到右花括号 `}` 所在列；后续 `selection_bounds_with` 算出的字节终点越过 `}`，触发 `edit_touches_title_line` 的 `to > marker.brace` 保护，**用户打字或退格被静默拦截、毫无反应**。
- **修复方案**：
  1. 在 `cell_range_for_byte_span` 中，逆序寻找 `start_byte..end_byte` 内**最后一个显示宽度 `> 0` 的字符**计算 `end_col`（若全为零宽则回退 `start_byte`），确保任何尾随零宽组合字符都不会把 `end_col` 推到 `}` 格上；
  2. 在 `rename_prompt_page_title` 中，先调用 `state.prompt.scroll_to(0)`，再调用 `set_cursor_offset`（或当目标行位于首屏内时保持 `scroll_to(0)`，超出首屏时由 `set_cursor_offset` 自动滚入视口可见区）。

---

### [P1-2] 编辑页右键菜单无条件调用 `state.prompt.clear_panel()` 会误取消未确认的粘贴选择面板（`PastePanel`）并回滚已粘贴文本，且右键落在已打开的 `#`/`/`/`@` 候选面板上时未拦截

- **关键点**：
  - `src/event/overlay.rs`：`open_right_click_menu`（第 `545–555` 行）
  - `src/app/prompt.rs`：`clear_panel` vs `dismiss_completion_panels`
- **错误原因**：
  1. `Prompt::clear_panel()` 首行会调用 `self.paste_cancel()`，将处于待确认状态的 `PastePanel` 回滚（还原粘贴前的文本、撤销栈与保存状态）。用户在粘贴弹出 `PastePanel` 后若右键编辑器想打开菜单，刚粘贴的内容会被意外回滚；
  2. 左键点击在 `press.rs:375` 优先检查 `prompt_command_panel_contains` 拦截浮层点击，而右键未做该检查，右键点在 `@Command` / `/Command` 面板内部会直接关掉面板并弹出底层页菜单。
- **修复方案**：
  1. 在 `Prompt` 新增 `dismiss_completion_panels(&mut self)`（仅关闭 `panel`、`slash`、`mention`、`refer`，不调用 `paste_cancel`），供编辑页右键菜单使用；
  2. 在 `open_right_click_menu` 的 `PromptMode::Editor` 分支中，若 `super::mouse::hit::prompt_command_panel_contains(state, view, column, row)` 为真则直接拦截返回 `false`；同时将右键命中区域扩至 `layout::prompt_body_rect(layout::pane_inner_rect(view.prompt))`（覆盖滚动条列），并让 `open_prompt_editor_menu` 返回 `bool`。

---

### [P2-1] `Fork Page` / `New Page` 在 `open_new_page_with_content` 中重复调用 `commit_editing_prompt` 导致产生两条重复的 `Effect::SavePromptContent`，且 `rename_prompt_page_title` / `fork_prompt_page` 缺少 `editing == Some(owner)` 守卫与缩进保持

- **关键点**：
  - `src/app/update/prompt/pages.rs`：`open_new_page_with_content`、`create_or_toast_new_page`、`fork_prompt_page`、`rename_prompt_page_title`
  - `src/app/prompt.rs`：`insert_template_title_line`
  - `src/app/selection.rs`：`Selection::programmatic_range`
- **错误原因**：
  1. `create_or_toast_new_page` 与 `fork_prompt_page` 在入口已调用 `commit_editing_prompt(state)`，随后调用 `open_new_page_with_content` 时又再次调用 `commit_editing_prompt(state)`；由于第一次 commit 已将 `page_pending` 置为 `true`，第二次 commit 会因为 `page_pending == true` 向 `state.effects` 再推入一条完全相同的 `Effect::SavePromptContent`；
  2. `rename_prompt_page_title` 与 `fork_prompt_page` 在 `hide_prompt_pages` 前未校验 `ws.prompt_panel.editing == Some(owner)`，且 `hide_prompt_pages` 放在了可能失败的校验之前；
  3. `insert_template_title_line` 插入 `\n{title: }` 时未继承模板起始行的前导空格缩进；`Selection::programmatic_range` 中 `origin` 写成了 `(start, start)` 而非 `(start, end)`。
- **修复方案**：
  1. 在 `open_new_page_with_content` 中仅当 `state.prompt.is_dirty()` 时才调用 `commit_editing_prompt(state)`（或避免已完成结算的调用方触发重复 `SavePromptContent`）；
  2. 在 `rename_prompt_page_title` 与 `fork_prompt_page` 入口校验 `ws.prompt_panel.editing == Some(owner)`，仅在目标页切换/校验通过后再执行 `hide_prompt_pages`；
  3. `insert_template_title_line` 自动提取起始行前导空格缩进；`Selection::programmatic_range` 将 `origin` 设为 `(start, end)`。

---

### [P2-2] 清理 `render_header` / `render_toolbar` 遗留的死参数 `_pinned: bool`、同步规范文档并将超 1500 行的 `src/app/markdown/tests.rs` 按模块拆分

- **关键点**：
  - `src/ui/prompt.rs`（`render_toolbar`）、`src/ui/prompt/header.rs`（`render_header`）、`src/ui/prompt_frame.rs`
  - `docs/standards/tui-design-requirements.md`
  - `src/app/markdown/template.rs`、`src/app/markdown/tests.rs`
- **错误原因**：
  1. `Pin` 按钮从顶部工具栏移至底边框后，`render_header` 与 `render_toolbar` 仍保留了未使用的 `_pinned: bool` 参数；
  2. `docs/standards/tui-design-requirements.md` 第 25 行仍写着页行右键菜单“不提供 `Rename`”，未同步新增的 `Rename Page` 与 `Fork Page`；
  3. `src/app/markdown/tests.rs` 因新增模板标题与 Fork 测试达到 `1638` 行，超过 `AGENTS.md` 规定的单测试文件 1500 行上限。
- **修复方案**：
  1. 移除 `render_header` 与 `render_toolbar` 的 `_pinned: bool` 参数及所有调用点传参；
  2. 同步更新 `docs/standards/tui-design-requirements.md` 中关于页列表右键菜单与编辑页右键菜单的说明；
  3. 将模板相关测试从 `src/app/markdown/tests.rs` 迁入 `src/app/markdown/template/tests.rs`，使 `src/app/markdown/tests.rs` 降至 1000 行左右。

---

## 修复验证记录

- [x] **[P0-1]** 编辑页右键与页菜单动作聚焦修复及回归测试
- [x] **[P0-2]** 共享 `repo_key` 停泊工作区 `Workspace.prompt` 缓冲随 `active_page` 同步刷新及回归测试
- [x] **[P1-1]** `Rename Page` 跨首屏视口滚动保底与尾随零宽字符（VS16/ZWJ）选区边界修复及回归测试
- [x] **[P1-2]** `dismiss_completion_panels` 不回滚粘贴提案、命令面板右键拦截与滚动条列右键覆盖
- [x] **[P2-1]** 消除 `open_new_page_with_content` 重复 `SavePromptContent`、补齐 `editing == Some(owner)` 守卫、继承起始行缩进与 `Selection::programmatic_range` `origin` 修复
- [x] **[P2-2]** 移除 `render_header`/`render_toolbar` 死参数 `_pinned`、更新 `docs/standards/tui-design-requirements.md`、拆分 `src/app/markdown/template/tests.rs`
