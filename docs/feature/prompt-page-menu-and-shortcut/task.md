# Prompt 编辑页快捷键调整、右键菜单与工具栏/底边按钮优化实施任务

- 状态：**已完成并合入 `dev` 分支，工作区与特性分支已清理**（2026-11-02）
- 关联设计：`docs/feature/prompt-page-menu-and-shortcut/design.md` | 评审记录：`docs/feature/prompt-page-menu-and-shortcut/review.md`

---

## 1. 需求与边界确认（已完成）

- [x] **问题 1（Rename Page 定位与选中）**：滚回顶部（`scroll_to(0)`）；非空 `{title: xxx}` 选中标题值并将光标置于值末尾；空 `{title: }` 光标停在冒号空格后、`}` 前；有 `&&&` 块但无 `{title:}` 行时在起始行下自动补入 `{title: }` 并定位光标；无 `&&&` 模板块时弹 Warning Toast `"No {title:} found in current page"`。
- [x] **问题 2（Fork Page 行为）**：先结算当前缓冲；若源页为空白页则复用空页防呆提示；否则复制源页正文（若首个模板块含非空 `{title: xxx}` 则改写为 `{title: xxx (fork)}`），追加到页列表末尾并立即切换到新页，弹出 Toast `"Page forked"`。
- [x] **问题 3（右键菜单项与顺序）**：`PromptMode::Editor` 右键菜单依次为 `Select All`、`Copy`、`Rename Page`、`New Page`、`Fork Page`、`Delete`（仅多页时显示，支持行内二次确认）；`PromptMode::Pages` 页行右键菜单同步补齐 `Rename Page` 与 `Fork Page`。
- [x] **问题 4（快捷键与移除 `[all]` 按钮）**：`Cmd/Ctrl + Shift + →` 在非最后一页时切换到下一页，仅在最后一页时触发新建页；移除顶部工具栏 `[all]` 按钮。
- [x] **问题 5（Pin 按钮移至底边框折叠按钮左侧并改用英文）**：移除工具栏圆形符号钉住按钮 `[○]`/`[◉]`，迁移至右侧容器底边框折叠按钮左侧（留 1 列 `─` 间隔），文案改为 `[pin]`（未钉，5 列）与 `[unpin]`（已钉，7 列），底边框身份标签自动避让；编辑视图工具栏右组精简为 `[refDir] ●`（`[refDir]` 最小显示宽度阈值降至 22 列）。

---

## 2. 实施步骤拆解

### Task 1：移除工具栏 `[all]`、将 `Pin` 按钮移至底边框折叠按钮左侧并改为英文
- [x] **1.1 布局层（`src/layout/prompt.rs` & `src/layout.rs`）**
  - 从 `PromptToolbarButton`、`prompt_toolbar_button_rect`、`prompt_toolbar_button_at` 中移除 `SelectAll` 与 `Pin`。
  - 移除 `PROMPT_SELECT_ALL_*` 与 `PROMPT_PIN_*` 常量及 `src/layout.rs` 中的重导出。
  - 新增 `PROMPT_LEFT_GROUP_MIN_WIDTH: u16 = 13`，将 `PROMPT_FOLDERS_MIN_WIDTH_EDITOR` 更新为 `22`，并让 `PromptToolbarButton::Folders` 直接锚定在保存圆点 `●` 左侧。
- [x] **1.2 底边框 Pin 按钮几何、渲染与避让（`src/ui/controls.rs`、`src/ui/right_panel.rs`、`src/ui/text.rs`）**
  - `src/ui/text.rs`：删除 `PROMPT_SELECT_ALL_LABEL`；将 `PROMPT_UNPINNED_LABEL` 改为 `"[pin]"`、`PROMPT_PINNED_LABEL` 改为 `"[unpin]"`。
  - `src/ui/controls.rs`：新增 `prompt_pin_button(panel, pinned)`、`prompt_pin_at(view, state, column, row)` 与 `render_prompt_pin(frame, view, state)`，在 `render_panel_buttons` 中绘制于折叠按钮左侧（间隔 1 列）。
  - `src/ui/right_panel.rs`：`render_bottom` 计算身份标签右界 `end` 时避让 `prompt_pin_button`。
- [x] **1.3 工具栏渲染、Tooltip 与鼠标事件层**
  - `src/ui/prompt.rs` & `src/ui/prompt/header.rs` & `src/ui/prompt_frame.rs`：从 `render_toolbar` 移除 `SelectAll` 与 `Pin`。
  - `src/app/tooltip.rs` & `src/ui/tooltip.rs`：移除 `TooltipTarget::PromptSelectAll`；将 `TooltipTarget::PromptPin` 改为取 `ui::prompt_pin_button(view.prompt, state.prompt_pinned.is_some())`。
  - `src/event/mouse/press.rs`：在 `collapse_button_at` 前增加 `ui::prompt_pin_at` 点击处理，移除工具栏 `SelectAll` 与 `Pin` 分支。
  - `docs/standards/tui-design-requirements.md`：同步更新工具栏与底边框 Pin 按钮描述。

### Task 2：调整 `Cmd/Ctrl + Shift + →` 快捷键切页/新建页逻辑
- [x] **2.1 状态更新层（`src/app/update/prompt/pages.rs` & `src/app/update/editor.rs` & `src/app/update/mod.rs`）**
  - 在 `src/app/update/prompt/pages.rs` 实现 `advance_or_create_prompt_page(state: &mut AppState) -> bool`：
    - 当 `active_index + 1 < pages.len()` 时调用 `switch_prompt_page(state, 1)`；
    - 当 `active_index + 1 == pages.len()` 时调用 `create_or_toast_new_page(state)`。
  - 在 `src/app/update/editor.rs` 中将 `EditorCommand::NewPage` 路由至 `advance_or_create_prompt_page`。

### Task 3：实现模板标题定位、字节转选区与 `Rename Page`
- [x] **3.1 模板标题解析与 Fork 文本转换（`src/app/markdown/template.rs` & `src/app/markdown/mod.rs`）**
  - 实现 `TemplateTitleLookup` 与 `find_first_template_title_span(text: &str) -> TemplateTitleLookup`（支持独立行 `{title: ...}`、起始行内联 `{title: ...}`、缺失标题行与无模板块四种情形）。
  - 实现 `fork_page_content(content: &str) -> String`（对非空标题追加 ` (fork)`）。
- [x] **3.2 选区与编辑器视口辅助（`src/app/selection.rs` & `src/app/prompt/viewport.rs` & `src/app/prompt.rs`）**
  - 在 `src/app/selection.rs` 新增 `Selection::programmatic_range(pane, start, end)`，确保单字符与多字符标题选区均有效。
  - 在 `src/app/prompt/viewport.rs` 新增 `Prompt::cell_range_for_byte_span(&self, start_byte: usize, end_byte: usize) -> Option<((i32, u16), (i32, u16))>`（跳过 `top_border` 虚拟行）。
  - 在 `src/app/prompt.rs` 新增 `Prompt::insert_template_title_line(&mut self, line_end_byte: usize) -> usize`，用于在首个模板起始行下一行补入 `\n{title: }` 并返回 `{title: |}` 光标字节偏移。
- [x] **3.3 状态层 `rename_prompt_page_title`（`src/app/update/prompt/pages.rs`）**
  - 支持从 `PromptMode::Editor` 或 `PromptMode::Pages` 触发（若在 `Pages` 视图则先切回 `Editor` 并选中目标页）。
  - 执行 `scroll_to(0)` 并根据 `TemplateTitleLookup` 选中标题内容、定位光标或弹出 `TOAST_PAGE_NO_TITLE`。

### Task 4：实现 `Fork Page` 与 DB 初始内容支持
- [x] **4.1 Effect 与 DB 层（`src/app/effects.rs`、`src/db/mod.rs`、`src/db/service.rs`）**
  - `Effect::CreatePage` 增加 `content: String` 字段。
  - `Database::create_prompt_page(&self, prompt_id: i64, name: &str, content: &str) -> Result<i64>` 支持写入初始 `content`。
  - `src/db/service.rs` 将 `content` 透传给 `db.create_prompt_page`。
- [x] **4.2 状态层 `open_new_page_with_content` 与 `fork_prompt_page`（`src/app/update/prompt/pages.rs`）**
  - 抽取 `open_new_page_with_content(state: &mut AppState, content: String) -> bool`，原 `open_new_page` 委托传入 `String::new()`。
  - 实现 `fork_prompt_page(state, workspace, owner, page)`：结算缓冲 → 空页防呆拦截 → `fork_page_content` 生成副本正文 → 退出 `Pages` 视图并创建新页 → 弹出 `TOAST_PAGE_FORKED`。

### Task 5：接入 `PromptMode::Editor` 与 `PromptMode::Pages` 右键菜单
- [x] **5.1 菜单枚举与文案（`src/app/overlay.rs`、`src/ui/overlay/menu.rs`、`src/ui/text.rs`）**
  - `MenuCommand` 新增 `SelectAll`、`RenamePage`、`ForkPage`。
  - `src/ui/text.rs` 新增菜单项与 Toast 常量，`src/ui/overlay/menu.rs` 补齐 `menu_labels` 映射。
- [x] **5.2 菜单打开与鼠标右键路由（`src/app/update/prompt/pages_view.rs`、`src/event/overlay.rs`）**
  - 实现 `open_prompt_editor_menu(state, anchor)`，更新 `open_page_menu(state, page, anchor)`。
  - 在 `src/event/overlay.rs` 的 `open_right_click_menu` 中接入 `PromptMode::Editor` 文本区右键命中。
- [x] **5.3 菜单命令激活分派（`src/app/update/menu.rs`）**
  - 在 `activate_menu` 中处理 `OverlayTarget::PromptPage` 下的 `SelectAll`、`CopyPageContent`（先 `commit_editing_prompt`）、`RenamePage`、`NewPage`、`ForkPage`、`DeletePage`。

---

## 3. 测试与验证计划

- [x] **单元测试更新与新增**：
  1. `src/layout/tests.rs`、`src/ui/prompt/tests.rs`、`src/ui/tests.rs`、`src/ui/tooltip/tests.rs`、`src/ui/text/tests.rs`：
     - 更新工具栏几何测试（移除 `SelectAll` 与 `Pin`，验证 `[refDir]` 锚定在保存圆点左侧、最小显示宽度阈值为 22 列）。
     - 验证底边框 `prompt_pin_button` 紧贴折叠按钮左侧（间隔 1 列），未钉显示 `[pin]`、已钉显示 `[unpin]`，且底边框身份标签避让钉住按钮。
  2. `src/app/markdown/tests.rs`：测试 `find_first_template_title_span` 与 `fork_page_content`。
  3. `src/app/update/tests.rs`、`src/event/tests/prompt_editor.rs`、`src/event/tests/prompt_tree.rs`：
     - 验证 5 页中第 3 页按 `Cmd/Ctrl + Shift + Right` 切到第 4 页、第 4 页按切到第 5 页、第 5 页按新建第 6 页。
     - 验证 `PromptMode::Editor` 右键弹出菜单，包含 `Select All`、`Copy`、`Rename Page`、`New Page`、`Fork Page`（多页时含 `Delete` 及二次确认）。
     - 验证点击 `Rename Page` 后 `scroll == 0`，且 `{title: xxx}` 中的 `xxx` 被选中，紧接着输入字符可直接替换 `xxx`。
     - 验证点击 `Fork Page` 后在末尾新增页、自动切入新页、且标题变为 `{title: xxx (fork)}`。
     - 验证点击底边框 `[pin]` / `[unpin]` 切换钉住状态。
- [x] **受影响模块精确校验**：
  - `cargo fmt --check`
  - `cargo clippy --all-targets -- -D warnings`
  - `cargo test`

---

## 4. Code Review 修复任务（关联 `review.md`）

- [x] **[P0-1] 编辑页右键与页菜单动作聚焦修复**：
  - `src/event/overlay.rs`：编辑页右键打开菜单时调用 `update::focus_prompt(state)`；
  - `src/app/update/menu.rs`：`SelectAll`、`OpenPage`、`NewPage`、`ForkPage`、`DeletePage`（Editor 模式下）确保聚焦 Prompt 面板。
- [x] **[P0-2] 共享 `repo_key` 后台停泊工作区缓冲同步**：
  - `src/app/update/prompt.rs`：`apply_prompt_tree` 在 `!displayed` 且 `editing == Some(PromptEditing::Item(id))` 时同步刷新已保存的停泊 `Workspace.prompt` 缓冲。
- [x] **[P1-1] `Rename Page` 尾随零宽字符选区与超首屏滚动保底**：
  - `src/app/prompt/viewport.rs`：`cell_range_for_byte_span` 按区间内最后一个非零宽字符计算 `end_col`；
  - `src/app/update/prompt/pages.rs`：`rename_prompt_page_title` 先 `scroll_to(0)` 再确保超首屏标题滚入视口。
- [x] **[P1-2] `dismiss_completion_panels` 与命令面板右键拦截**：
  - `src/app/prompt.rs`：新增 `dismiss_completion_panels`（不触发 `paste_cancel`）；
  - `src/event/overlay.rs` & `src/app/update/prompt/pages_view.rs`：拦截命令面板内右键、覆盖 `prompt_body_rect`、`open_prompt_editor_menu` 返回 `bool`。
- [x] **[P2-1] 消除重复 `SavePromptContent`、补齐守卫与缩进**：
  - `src/app/update/prompt/pages.rs`：避免二次 `commit_editing_prompt` 重复入队 `SavePromptContent`，补齐 `editing == Some(owner)` 守卫；
  - `src/app/prompt.rs` & `src/app/selection.rs`：`insert_template_title_line` 继承起始行前导空格，`Selection::programmatic_range` 设置 `origin: (start, end)`。
- [x] **[P2-2] 移除 `_pinned` 死参数、更新规范文档并拆分 `markdown/tests.rs`**：
  - `src/ui/prompt.rs`、`src/ui/prompt/header.rs`、`src/ui/prompt_frame.rs`、`src/ui/prompt/tests.rs`：移除 `_pinned: bool` 参数；
  - `docs/standards/tui-design-requirements.md`：同步更新页右键菜单项描述；
  - `src/app/markdown/template/tests.rs` & `src/app/markdown/tests.rs`：将模板测试拆分至子模块，使单测试文件低于 1500 行上限。
