# Prompt 块结构与页列表重构 — Code Review (`review.md`)

- 分支 / 工作区：已合入 `dev`（原 `feat/prompt-block-restyle` @ `.worktrees/prompt-block-restyle`）
- 审查范围：`dev..HEAD` 全部代码变更、`docs/feature/prompt-block-restyle/{design,task}.md` 与 `docs/standards/tui-design-requirements.md`
- 排序规则：按缺陷严重度与影响面从 `P0` 到 `P3` 排列
- 状态：**全部已修复、验证通过并已合入 `dev`**

---

## P0（阻塞性功能错误 / 结构破坏）

### P0-1：页面存在未闭合块时触发 `/supplement` 会把新 `+++` 块吞进未闭合块内部，且未闭合 `&&&` 渲染无效的 `[todo]` 状态标签

- **关键点**：
  - `src/app/prompt/block.rs:90-132`（`Prompt::append_supple_block`）
  - `src/app/markdown/slash.rs:200-270`（`SlashContext`、`slash_filter`）
  - `src/ui/prompt_frame.rs:21-26`（`prompt_template_status`）
  - `src/app/prompt/template.rs:34-44`（`Prompt::cycle_template_status`）
- **错误原因**：
  1. `block_scan` 对缺少 `--end` 行的未闭合 `&&&` 或 `+++` 块，会将 `span.end` 延伸至文档末行（`lines.len() - 1`）并标记 `span.closed = false`。
  2. `append_supple_block` 查找插入锚点时未检查 `span.closed`，直接将 `span.end`（文末行）作为锚点：
     - 若页面存在未闭合的 `&&&` 块，新插入的 `+++ supplement --start ... --end` 落在未闭合 `&&&` 内部，被 `block_scan` 当作 `&&&` 的普通 `Middle` 正文行吞掉，不成块、无色条、无按钮；
     - 若页面存在未闭合的 `+++` 块，新插入的 `+++` 块的 `--end` 会错误闭合前一个未闭合的 `+++` 块，导致两个补充块粘连损坏。
  3. `prompt_template_status` 未过滤 `span.closed`，导致未闭合的 `&&&` 块在标题行显示可点击的 `[todo]` 状态标签；但点击标签或按 `Shift+Tab` 时，`toggle_template_status` 因文末不是合法 `--end` 行而静默返回 `false`，造成“显示可交互标签却点击无效、且 `Shift+Tab` 也无法回退反缩进”的死锁体验。
- **修复方案**：
  1. `SlashContext` 新增 `unclosed_block: bool`（由 `block_scan(&self.text).spans.iter().any(|s| !s.closed)` 计算）；当 `unclosed_block == true` 时，`slash_filter` 隐藏所有 `SlashInsertion::SuppleBlock`（内置 `/supplement` 与自定义 `+++` 块命令）。
  2. `append_supple_block` 中若存在 `span.closed == false` 的块则直接返回不插入，取锚点时仅匹配 `span.closed == true` 的块。
  3. `prompt_template_status` 与 `Prompt::cycle_template_status` 均要求 `span.kind == BlockKind::Template && span.closed`，未闭合 `&&&` 不在标题行显示状态标签，`Shift+Tab` 正常回退为反缩进（注：Pages 列表 `page_title_status` 保持未闭合 `&&&` 计为 `Todo` 空心圆点不变）。

---

## P1（核心交互缺陷 / 状态不一致）

### P1-1：Pages 列表视图下右键 `New page` 未与快捷键统一空白页防重，且未清空搜索过滤词、未滚入视口

- **关键点**：
  - `src/app/update/menu.rs:470-474`（`MenuCommand::NewPage` 分派）
  - `src/app/update/prompt/pages.rs:177-187, 206-290`（`open_new_page`、`create_or_toast_new_page`、`open_created_page`）
- **错误原因**：
  1. 编辑视图按 `Ctrl/Cmd+Shift+Right` 走 `create_or_toast_new_page`（已有空白页时切过去并弹 `Already have a new page` Warning Toast），但 Pages 列表右键菜单 `MenuCommand::NewPage` 直接调用了 `open_new_page`，绕过了空白页防重与 Toast 提示，导致在 Pages 列表可无限追加空页；
  2. `open_new_page`（临时提示词分支）与 `open_created_page`（正式条目回调分支）在 `PromptMode::Pages` 下只设置了 `panel.pages_view.selected = pos`，**未调用 `clamp_page_view`**，当页数超过视口高度时新选中的页落在视口外不可见；
  3. 当 `pages_view.search` 搜索词非空时，新建的空白页标题 `Page N` 不匹配过滤词，`rows.iter().position(...)` 返回 `None`，新页被过滤词遮蔽，列表无任何变化反馈。
- **修复方案**：
  1. `MenuCommand::NewPage` 统一改为调用 `create_or_toast_new_page(state)`；
  2. `create_or_toast_new_page` 检测到已存在空白页时：
     - 若当前为 `PromptMode::Pages`，保持在 `Pages` 视图，先清空 `panel.pages_view.search`，在完整行列表中定位该空白页设置 `selected` 并调用 `clamp_page_view` 滚入视口，同时弹出 `Already have a new page` Warning Toast；
     - 若当前为 `PromptMode::Editor`，切到该空白页并弹 `Already have a new page` Warning Toast；
  3. 新建页成功时（`open_new_page` Temp 分支与 `open_created_page` Item 分支），在 `PromptMode::Pages` 下统一先 `panel.pages_view.search.clear()`，再定位新页 `selected = pos` 并调用 `clamp_page_view(panel, rows.len(), visible)`。

### P1-2：编辑视图隐藏标题行页名后，Pages 列表右键 `Rename` 修改的 `page.name` 被 `&&&` 标题遮蔽，重命名无任何界面反馈

- **关键点**：
  - `src/app/update/prompt/pages_view.rs:272-280, 313-340`（`open_page_menu`、`open_page_rename`）
  - `src/app/overlay.rs`（`MenuCommand::RenamePage`、`RenameTarget::PromptPage`）
  - `src/app/update/menu.rs:450-458`、`src/app/update/prompt.rs:789-838`
- **错误原因**：
  编辑视图标题行已按需求移除 `· 页名` 展示（`PromptMode::Editor` 恒传空串），而 Pages 列表行标题优先级为 `独立 {title:} → 行内 {title:} → &&& 模板命令名 → page.name`。只要页内有 `&&&` 块（哪怕 `{title:}` 为空也会取 `addTemplate` 等命令名），用户通过右键 `Rename` 修改 `page.name` 后，在编辑视图和 Pages 列表中都看不到任何变化。
- **修复方案**：
  移除 Pages 列表右键菜单中的 `Rename` 入口（删除 `MenuCommand::RenamePage`、`RenameTarget::PromptPage`、`open_page_rename` 及 `commit_prompt_rename` / `ui/overlay/menu.rs` 中的对应分支）。页标题统一由正文 `&&&` 的 `{title:}` / 模板名派生，无 `&&&` 时按序号显示 `Page N`。

### P1-3：鼠标点击标题行状态标签及块内 `[⌫]` / `[✕]` 按钮改写正文时未清除选区 `state.selection`

- **关键点**：
  - `src/event/mouse/press.rs:386-392`（标题行状态标签点击）
  - `src/event/mouse/hit.rs:396-425`（`apply_template_button`、`apply_block_button`）
- **错误原因**：
  键盘路径（`apply_editor`）在执行块状态轮转或编辑命令前会先清理 `state.selection`；而鼠标点击标题行 `[todo]`/`[run]`/`[done]` 标签、或点击块内 `[⌫]`（Clean）与 `[✕]`（Delete）按钮时，直接原地改写了 `state.prompt` 文本缓冲，未清除 `state.selection` 中记录的旧字节偏移区间。随后用户按键输入或退格时，会按过期的选区偏移误替换无关文本。
- **修复方案**：
  在 `press.rs` 的标题行状态标签点击分支，以及 `hit.rs` 的 `apply_template_button` / `apply_block_button` 的 `Clean`、`Delete` 分支中显式调用 `update::clear_selection(state)`。

### P1-4：切换工作区往返时，正式条目（`Item`）的 `PromptMode::Pages` 模式被重置为 `Editor`，与临时提示词（`Temp`）不一致

- **关键点**：
  - `src/app/update/editor.rs:355-376`（`restore_prompt_view`）
  - `src/app/update/prompt.rs:407-425`（`select_prompt_item`）
- **错误原因**：
  `restore_prompt_view` 在工作区处于 `PromptMode::Editor | PromptMode::Pages` 时，对 `PromptEditing::Temp(_)` 直接返回从而保留了 `PromptMode::Pages`；但对 `PromptEditing::Item(id)` 调用了 `select_prompt_item(state, workspace, id)`，后者无条件执行 `ws.prompt_panel.mode = PromptMode::Editor`，导致正式条目在切换工作区或取消钉住后丢失 Pages 列表视图状态。
- **修复方案**：
  在 `restore_prompt_view` 中记录调用前的 `mode`，`select_prompt_item` 完成后若原 `mode == PromptMode::Pages`，恢复 `panel.mode = PromptMode::Pages` 并调用 `clamp_page_view` 修正视口。

---

## P2（边界交互与防御性守卫）

### P2-1：Pages 列表右键删除非选中页后 `selected` 漂移、清空搜索（`Ctrl+C`）未重定位选中页、搜索无匹配时标题行计数回退歧义

- **关键点**：
  - `src/app/update/prompt/pages.rs:390-414`（`perform_delete_page`）
  - `src/app/update/prompt/pages_view.rs:194-211`（`clear_page_search`）
  - `src/ui/prompt_frame.rs:97-105`（`prompt_page_header`）
- **错误原因**：
  1. 在 Pages 列表中右键点击非当前选中行并确认 `Delete` 时，`perform_delete_page` 未记录原先选中行的 `PromptPageId`，仅对原 `selected` 下标做 `.min(rows_len - 1)`；若被删行位于选中行上方，选中箭头 `❯` 会漂移到下一行；
  2. `clear_page_search`（`Ctrl+C`）清空过滤词后直接沿用了过滤列表中的下标 `selected`，未重新定位到原先选中的 `PromptPageId`（或 `active_page`）；
  3. 当搜索无匹配结果（`rows.is_empty()`）时，`prompt_page_header` 通过 `.or_else` 回退显示 `active` 页的全局序号（如 `2/5`），与列表正文的 `no matching pages` 语义冲突。
- **修复方案**：
  1. `perform_delete_page` 删除前记录当前选中行的 `PromptPageId`；若删除的不是选中页，删除后在新的过滤行列表中重定位该 `PromptPageId` 的下标，再调用 `clamp_page_view`；
  2. `clear_page_search` 清空前取当前选中行的 `PromptPageId`（无则取 `active_page`），清空搜索词后定位到该页下标并调用 `clamp_page_view`；
  3. `prompt_page_header` 在 `PromptMode::Pages` 下若 `rows.is_empty()`，将 `current` 置为 `0`（显示 `0/m`）。

### P2-2：`prompt_page_control_at` 缺少 `state.right_tab == RightTab::Prompt` 守卫

- **关键点**：
  - `src/ui/prompt_frame.rs:132-155`（`prompt_page_control_at`）
- **错误原因**：
  同文件的 `prompt_status_rect` 与 `prompt_scrollbar` 均检查 `state.prompt_collapsed || state.right_tab != RightTab::Prompt`，而 `prompt_page_control_at` 仅检查 `state.prompt_collapsed`。当右侧面板切到 `Todo` 标签页时，若调用方未前置过滤 `right_tab`，会误命中隐藏的页控件。
- **修复方案**：
  在 `prompt_page_control_at` 开头统一加上 `if state.prompt_collapsed || state.right_tab != RightTab::Prompt { return None; }`，并补充 Todo 标签页下不命中的单测。

---

## P3（代码整洁度与文档同步）

### P3-1：清理兼容壳与死代码（`From<bool> for PromptMode`、`cycle_template_status_at_cursor`、`Effect::RenamePage` / `RENAME_PAGE_TITLE`）

- **关键点**：
  - `src/layout/prompt.rs:64-68, 181-186, 289-295`
  - `src/app/prompt/template.rs:46-49`
  - `src/app/effects.rs`、`src/db/service.rs`、`src/ui/text.rs`
- **错误原因**：
  1. `impl From<bool> for PromptMode` 与 `mode: impl Into<PromptMode>` 是迁移期为兼容旧测试布尔参数留下的垫片，使三态枚举退化为隐式接受布尔值；
  2. `Prompt::cycle_template_status_at_cursor`、`Effect::RenamePage`、`text::RENAME_PAGE_TITLE` 在重构与移除页重命名后已无任何调用方。
- **修复方案**：
  1. 删除 `impl From<bool> for PromptMode`，将 `prompt_toolbar_button_rect` 与 `prompt_toolbar_button_at` 签名收窄为 `mode: PromptMode`，更新 `src/layout/tests.rs` 等测试调用方显式传 `PromptMode`；
  2. 删除 `cycle_template_status_at_cursor`、`Effect::RenamePage`、`RENAME_PAGE_TITLE` 及其测试引用。

### P3-2：`design.md`、`task.md` 与 `docs/standards/tui-design-requirements.md` 同步更新

- **关键点**：
  - `docs/feature/prompt-block-restyle/design.md`
  - `docs/feature/prompt-block-restyle/task.md`
  - `docs/standards/tui-design-requirements.md`
- **错误原因**：
  P5 之后的多次增量迭代（全页 `Shift+Tab`、编辑视图 `Esc` 不退回 Tree、Pages 列表新建页留在列表并防重、编辑视图 `Ctrl/Cmd+Shift+Right` 新建页、编辑视图标题行移除 `· 页名`、浮动面板让开 1 列色条槽、`&&&` 色条固定 `LX_PINK`、移除块内状态按钮与外框染色并改为标题行可点击状态标签 + tooltip、移除 Pages 右键 `Rename`、未闭合块隐藏状态标签与 `/supplement`）尚未完整回写到 `design.md`、`task.md`（复选框仍未勾选）及 `tui-design-requirements.md`。
- **修复方案**：
  全面更新上述三份文档，确保设计文档、任务清单与项目规范同最新代码实现 100% 对齐。
