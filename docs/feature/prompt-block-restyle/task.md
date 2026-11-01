# Prompt 块结构与页列表重构 — 实施任务

- 状态：**已实施**。
- 设计：同目录 `design.md`。本清单中的 `§x` 指设计文档的章节。
- 工作区：`.worktrees/prompt-block-restyle`，分支 `feat/prompt-block-restyle`。codegraph 和 codebase-memory（`lx-tui-prompt-block-restyle`）已初始化。
- 提交：每个阶段验证通过后由 agent 自行 commit（用户已授权），提交信息沿用仓库风格 `feat(prompt): …` / `refactor(prompt): …`。
- 验证口径：每个阶段执行 `cargo fmt`、`cargo clippy --all-targets -- -D warnings`，以及受影响模块的 `cargo test <模块路径>`。最后一个阶段跑一次全量 `cargo test`。TUI 外观与交互由用户 `cargo run` 自行确认。

---

## 0. 实施前复核

- [x] 用 codegraph `codegraph_explore` 复查 `block_scan`、`template_line_infos`、`ui::markdown::scan` 的全部调用方，和 design §3.3 的表格逐条对照。新发现的调用方补进本清单。
- [x] 用 codebase-memory `trace_path` 复查 `PromptMode` 的所有 match 点（design §7.1 列表），以及 `prompt_tree_mode` 的全部调用方。
- [x] 确认 `Menu` 没有 disabled 能力（design §9.1）。如果已经有，就改为用它置灰 `DeletePage`，并更新 design。
- [x] 确认 `select_all_prompt` 和 `state.rs` 里没有块聚焦分支残留。
- [x] 记录基线：`cargo test 2>&1 | tail -3` 的通过数，用于每个阶段对比。

## P1 解析层（design §3）

### P1.1 `block_scan` 平级化

- [x] `blocks.rs`：引入 `Open { Template(usize), Supple(usize) }` 单层状态机，并加 `template_seen` 标志。
- [x] `fill_roles` 改成一轮遍历。`block_spans` 的 doc 改为「文档顺序」。模块头注释改为「顶层 `&&&`（仅首个）与顶层 `+++` 平级」。
- [x] 新建 `app/markdown/blocks/tests.rs`（`blocks.rs` 末尾加 `#[cfg(test)] mod tests;`），覆盖以下用例：
  - [x] `&&&` 后跟多个 `+++`：spans 顺序、每行的 role 和 kind 都正确。
  - [x] 第二个 `&&&` 起止行为 `None`（不是块），第一个 `&&&` 正常。
  - [x] 第一个 `&&&` 未闭合：吞到文末，其后的 `+++` 不成块。
  - [x] `&&&` 内的 `+++ x --start` 行是 `&&&` 的 Middle 行。
  - [x] `+++` 内的 `&&&` 行是 `+++` 的 Middle 行。
  - [x] `+++` 写在 `&&&` 之前也能成块。
  - [x] 围栏内的 `&&&` / `+++` 不识别。
  - [x] 顶层 `+++` 的标题分割行 `separator == true`。
  - [x] `+++ log` 保留词不成块。
- [x] `app/markdown/tests.rs` 中依赖嵌套语义的旧用例：改写或迁入 `blocks/tests.rs`。

### P1.2 扫描器统一（design §3.3）

- [x] `template.rs`：`template_line_infos`、`parse_template_block_at_line`、`template_block_start_at` 改为从 `block_scan` 投影。
- [x] 删除 `inside_template_block` 及 `app/markdown/tests.rs:1197` 的用例。
- [x] `ui/markdown.rs`：`scan` 先取 `block_line_infos`，`ScanState` 只保留 `fence`，`scan_line` 增加 `Option<BlockLineInfo>` 参数，按 design §3.3.2 的分支表分派。
- [x] `ui/markdown/tests.rs`：补充用例：第二个 `&&&` 不产生 `TemplateMarker`；`&&&` 内的 `+++` 不产生 `SuppleMarker`；顶层 `+++` 产生 `SuppleMarker` / `SuppleCommand`。
- [x] `refer_shortcut` 现有测试全部通过（lx 输入框 refer 范围走投影后的 `template_line_infos`）。

### P1.3 斜杠命令（design §3.4）

- [x] `SlashContext` 增加 `template_present`，`panel.rs:400` 处填充。
- [x] `slash_filter` 按 design §3.4 的可见性矩阵调整。`builtin_supplement` 的 doc 同步修改。
- [x] `slash.rs` 顶部的 doc（107–112 行）同步修改。
- [x] 测试：页面已有 `&&&` 时，Normal 上下文中不出现内置模板命令和自定义 AgentBlock；`/supplement` 在 Normal、Template、Supple 三种上下文中都可见。

### P1.4 `+++` 追加落点（design §3.5）

- [x] `prompt/block.rs` 新增 `append_supple_block`，`viewport.rs` 新增 `scroll_lines_into_view`。
- [x] `panel.rs::slash_confirm`：`SuppleBlock` 分支改为追加，删除 `content.starts_with("+++")` 嗅探。`expanded_supplements.insert` 在 P1 **暂时保留**，键改为新块起始行的行首偏移，这样新块在折叠被删除之前仍然是展开的。到 P2 删除折叠时，连同字段一起删掉。
- [x] 测试（放入 `app/prompt/block/tests.rs`，新建）：
  - [x] 空文档中 `/supplement` → 全文就是块，光标在 `{title: |}`。
  - [x] 只有 `&&&`，命令写在 `&&&` 之前 → 命令行消失，块追加在 `&&&` 之后，中间恰好空一行。
  - [x] 已有 2 个 `+++`，命令写在 `&&&` 内 → 追加到第 2 个 `+++` 之后，空一行。
  - [x] 最后一个 `+++` 之后还有普通正文 → 块插在 `+++` 和正文之间，上下各空一行。
  - [x] 锚点下一行原本就是空行 → 不产生两个连续空行。
  - [x] 一次 `undo` 完全恢复（命令行回来，块消失）。
  - [x] 插入后 `scroll` 使新块的起始行可见（视口高度小于文档长度的场景）。

### P1.5 复制（design §3.6）

- [x] `copy_template_content`：删除 `inside_supple` 和 `CopySubblock::Supple`，`CopySubblock` 简化为 `is_log_subblock_start`。
- [x] 测试：`&&&` 的 `[copy]` 不包含任何 `+++` 的正文；`+++ log` 仍被跳过。

### P1.6 worktree

- [x] 补用例：顶层 `+++` 的 `{wt:}` 绑定能写入、能识别（`app/markdown/worktree` 的测试文件）。

### P1 收口

- [x] `cargo fmt`、`cargo clippy --all-targets -- -D warnings`、`cargo test app::markdown ui::markdown app::prompt`。
- [x] commit：`refactor(prompt): flatten +++ blocks to top level and allow a single &&& per page`

## P2 删除块聚焦与折叠（design §4）

### P2.1 折叠

- [x] 按 design §4.1 表格逐行删除。`git rm src/app/prompt/fold.rs src/app/prompt/fold/tests.rs`。
- [x] `navigation.rs` 中的 4 处 `snap_visible` 改为直接赋值。
- [x] `viewport.rs` 的模块文档删除折叠描述。
- [x] 删除 `app/prompt/tests.rs`、`event/tests/prompt_editor.rs`、`ui/prompt/tests.rs` 中的折叠用例（grep `fold|folded|expanded`）。

### P2.2 块聚焦

- [x] 按 design §4.2 表格逐行删除。`git rm src/app/prompt/block_focus.rs`。
- [x] 确认 `title_value_offset` 没有其他调用方后，随文件一起删除。
- [x] 删除 `app/prompt/tests.rs`、`app/update/prompt/tests.rs`、`event/tests/prompt_tree.rs`、`ui/tests.rs`、`ui/tooltip/tests.rs`、`layout/tests.rs` 中的块聚焦用例。

### P2.3 切页快捷键（design §4.3）

- [x] `actions.rs`：`BlockPrev/BlockNext { cmd }` 改为 `PagePrev/PageNext`。
- [x] `input/mod.rs`：Ctrl 和 Super 两个分支都改为映射到切页；doc 注释去掉「Ctrl+←/→ 按词移动」。
- [x] `editor.rs::apply_editor`：调用 `switch_prompt_page(±1)`，事先关闭补全面板。
- [x] 测试：
  - [x] `input/tests.rs`：`Ctrl+←`、`Cmd+←` 映射为 `PagePrev`；`Alt+←` 仍然是 `WordLeft`。
  - [x] `app/update/prompt/tests.rs`：3 页时在第 2 页按 `PageNext` 切到第 3 页，再按一次 `PageNext` 仍在第 3 页；第 1 页按 `PagePrev` 不动；只有 1 页时无副作用。
  - [x] 切页前 slash 面板打开 → 切页后面板关闭。

### P2.4 工具栏布局（design §4.4）

- [x] `layout/prompt.rs`：删除 `BlockFocus` 及其常量，`PROMPT_PIN_MIN_WIDTH_EDITOR` 重新计算，`Pin` 改为以 `Save` 为锚点。
- [x] `ui/prompt.rs::render_toolbar`、`header.rs::render_header`、`prompt_frame.rs`：删除 `block_focus*` 参数。
- [x] `layout/tests.rs`：更新 Pin 和 Folders 的阈值、坐标用例。

### P2 收口

- [x] `cargo fmt`、`cargo clippy --all-targets -- -D warnings`、`cargo test app::prompt app::update input layout ui::prompt event`。
- [x] commit：`refactor(prompt): drop supplement folding and block focus, map Ctrl/Cmd+arrows to page switch`

## P3 渲染（design §5、§6）

### P3.1 色条与分割线

- [x] `layout/prompt.rs`：`PROMPT_TEMPLATE_GUTTER_LEFT/RIGHT` 改为 `PROMPT_BLOCK_GUTTER_LEFT = 1`，删除 RIGHT。
- [x] `viewport.rs::visual_rows`、`line_gutter` 改用新常量。
- [x] `ui/text.rs`：增加 `PROMPT_BLOCK_BAR`、`PROMPT_BLOCK_RULE`。
- [x] `ui/prompt.rs`：新增 `paint_block_bar`、`block_color`；改写 `paint_block_separator`；删除 `paint_block_border`、`block_row_role`；`block_gutter`、`block_buttons_start`、`block_text_end` 改用新边界。
- [x] `SUPPLE_BUTTONS` 改为 3 个元素。
- [x] `style.rs`：删除 `template_block_bg`、`supple_block_bg` 及相关注释和测试。
- [x] 测试（`ui/prompt/blocks/tests.rs`，新建，不往 1896 行的 `ui/prompt/tests.rs` 里加）：
  - [x] `&&&` 每个视觉行（含软换行续行）首列是 `▎`，前景固定 `style::LX_PINK`（`Indexed 218`），不随状态变化。
  - [x] `+++` 首列是 `▎`，前景为品红。
  - [x] 分割行：首列 `▎`，其余全是 `─`，没有 `├` / `┤`，颜色与所在块一致且不随状态变化。
  - [x] 起始行按钮组右对齐到文本区右缘，文本超宽时以 `…` 截断；`&&&` 只有 `[❐]`/`[⌫]`/`[✕]` 3 个按钮，没有 `[status]`。
  - [x] 第二个 `&&&` 行没有色条，也没有按钮。
  - [x] 普通行首列不是 `▎`。
- [x] 删除或改写 `ui/prompt/tests.rs` 中断言 `┌ │ └ ┐ ┘` 的旧用例。

### P3.2 标题行状态标签（design §6）

- [x] `prompt_frame.rs`：新增 `prompt_template_status`、`prompt_status_rect`、`prompt_status_at`；编辑视图且本页存在已闭合 `&&&` 时取状态，否则 `None`。
- [x] `ui/prompt/header.rs`：状态标签 `[todo]`/`[run]`/`[done]` 右对齐贴页控件左侧 1 列，按状态取蓝/黄/绿；空间不足时隐藏。
- [x] `ui/right_panel.rs`：**不新增** `tint_edit_border`，外框恒为暗灰；删除迁移期的 `edit_border_style` 方案。
- [x] 测试：
  - [x] Editor 视图 + in_progress：标签为 `[run]` 且为黄色，位置在 `‹ n/m ›` 左侧 1 列。
  - [x] todo 蓝、done 绿；Tree 视图、Pages 视图不渲染标签。
  - [x] 未闭合的 `&&&` 不渲染标签。
  - [x] 外框（分割线以下左右竖边、底边）在任意状态、任意 tab 下都保持暗灰。

### P3 收口

- [x] `cargo fmt`、`cargo clippy --all-targets -- -D warnings`、`cargo test ui app::prompt layout`。
- [x] commit：`feat(prompt): replace block frames with status bars`（外框染色方案随后在 P6 被标题行状态标签取代）

## P4 Pages 视图（design §7、§8、§9）

### P4.1 状态与模式

- [x] `tree/panel.rs`：`PromptMode::Pages`、`PageListView`、`PromptPanel.pages_view`。
- [x] `name_matches` 改为 `pub(crate)`。
- [x] 逐一补全所有 `match panel.mode`（以 P0 的复核清单为准）。
- [x] `update`：新增 `prompt_pages_mode`。

### P4.2 行模型（`app/prompt/pages_list.rs` + `pages_list/tests.rs`）

- [x] `PageRow`、`page_title_status`、`page_rows`。
- [x] 测试：
  - [x] 独立 `{title: 重构}` → 标题为「重构」。
  - [x] 独立标题为空，行内 `{title:x}` → 标题为「x」。
  - [x] 两种标题都为空 → 标题为 `refactorTemplate`。
  - [x] 没有 `&&&` → 显示 `Page N`（按页序号，不读 `page.name`），status 为 None。
  - [x] 页序号按页在集合中的位置生成，删除中间页后重新编号。
  - [x] 未闭合的 `&&&` → status 为 `Todo`。
  - [x] 第二个 `&&&` 的标题不会被采用。
  - [x] 搜索按显示标题过滤，大小写不敏感。

### P4.3 进入、返回与键盘（`app/update/prompt/pages_view.rs` + tests）

- [x] `show/hide/toggle_prompt_pages`、`open_selected_page`、`move_page_selection`、`select_page_row`、`apply_page_search`、`scroll_page_list`。
- [x] `reload_editor_from_active_page` 的条件放宽到 `mode != Tree`。
- [x] `actions.rs`：增加 `TogglePromptList`、`TogglePageList`；`input/mod.rs` 增加 Ctrl 和 Super 的 `l` / `p` 映射。
- [x] `editor.rs::apply_editor`：最前面先关闭补全面板，再按 design §7.3 的切换矩阵分派；新增 `route_prompt_pages`；编辑视图 `Esc` 只关闭浮层/面板，不返回 Tree。
- [x] `event/input.rs` 的 `Routed::Copy`、`event/paste.rs`：增加 Pages 分支。
- [x] 测试：
  - [x] 切换矩阵逐一覆盖（Editor / Tree / Pages × `Ctrl+L` / `Ctrl+P` / `Esc`）：Editor 的 `Esc` 保持编辑视图且只关面板，Tree/Pages 的 `Esc` 返回 Editor。
  - [x] Editor → Pages → Esc：光标、撤销栈、滚动不变。
  - [x] Pages 中 Enter 打开其他页 → Editor 显示该页正文，原页正文已 commit。
  - [x] Pages 中 Enter 打开当前页 → 只是返回，撤销栈保留。
  - [x] 没有编辑目标（Tree 视图）时按 `Ctrl+P` 无反应。
  - [x] slash 面板打开时按 `Ctrl+L`：面板关闭，进入 Tree。
  - [x] `input/tests.rs`：`Ctrl+L`、`Cmd+L`、`Ctrl+P`、`Cmd+P` 都能映射；`Ctrl+Alt+L` 不映射。

### P4.4 渲染与头部（`ui/prompt/pages.rs` + tests）

- [x] 搜索行、分割线、行（`❯`、加粗当前页、截断、圆点）、空态、悬停底色。
- [x] `tree.rs` 的 `fill_row`、`paint_text` 改为 `pub(super)` 共用。
- [x] `prompt_frame.rs`：`title_source`、`render_body`、`prompt_scrollbar_for` 增加 Pages 分支；页视图硬件光标落在搜索框上。
- [x] `render_toolbar` / `prompt_toolbar_button_rect`：参数由 `tree: bool` 改为 `mode`；Pages 视图显示 `[/]` 和 Pin。
- [x] `header.rs::page_controls` 增加 `arrows` 参数；Pages 视图标题行为 `条目名 · pages … n/m`，搜索无匹配时显示 `0/m`；编辑视图标题行只显示条目名（临时提示词 `Temp prompt`），不显示 `· 页名`。
- [x] 测试：
  - [x] in_progress 页显示黄色 `●`，done 页显示绿色 `●`，todo 页显示灰色 `○`，没有 `&&&` 的页不画圆点。
  - [x] 当前页加粗，选中行有 `❯`。
  - [x] 长标题截断后圆点仍然保留。
  - [x] 标题行在 Pages 视图中没有 `‹` / `›`，计数为选中行序号。
  - [x] 窄宽度（例如 12 列）下不 panic、不画半个宽字符。

### P4.5 删除行内重命名与 PageMenu（design §8.4、§8.5）

- [x] 按两张表逐行删除；`git rm src/ui/overlay/page_menu.rs`。
- [x] 删除旧流程 `delete_current_page`、`confirm_delete_page`，以及 `ConfirmClose(PromptPage)` 的分支。
- [x] 删除或改写 `ui/overlay/tests.rs`、`event/tests/overlay.rs`、`event/tests/prompt_editor.rs`、`app/update/prompt/tests.rs`、`input/tests.rs`、`ui/tooltip/tests.rs`、`db/service/tests.rs` 中的相关用例。
- [x] `docs/standards/tui-design-requirements.md`：第 25 行（工具栏和标题行描述）、第 29 行（浮层清单）、第 35 行（确认浮层示例）、第 64 行（草稿状态中的「块聚焦」）、第 84 行（「折叠或尺寸变化」）同步修改。

### P4.6 右键菜单（design §9）

- [x] `overlay.rs`：增加 `MenuCommand::{OpenPage, CopyPageContent, NewPage, DeletePage}`、`OverlayTarget::PromptPages`；`requires_confirm` 中加入 `DeletePage`；不新增 `RenamePage` / `RenameTarget::PromptPage`。
- [x] `pages_view.rs`：`open_page_menu`（只有一页时不放入 Delete，菜单仅 `Open / Copy content / New page / Delete`）、`open_pages_blank_menu`。
- [x] `event/overlay.rs`：右键分发增加 Pages 分支。
- [x] `menu.rs`：命令分派；`NewPage` 与编辑视图 `Ctrl/Cmd+Shift+Right` 统一走 `create_or_toast_new_page`；`prompt.rs::commit_prompt_rename` 不加 `PromptPage` 分支。
- [x] `pages.rs`：`open_new_page` / `open_created_page` 去掉 `start_page_rename`；`NewPage` 在 Pages 视图留在列表、清空搜索、选中新页并滚入视口，编辑视图切入新页；`perform_delete_page` 只在删除当前页时重载编辑缓冲。
- [x] `pages_list.rs`：页标题由正文 `&&&` 的 `{title:}` / 模板名派生，无 `&&&` 时显示 `Page N`，不回退 `page.name`。
- [x] `ui/overlay/menu.rs`、`ui/text.rs`：文案能复用就复用；新增 `Already have a new page` / `New page created` Toast 文案。
- [x] 测试：
  - [x] 右键页行 → 菜单包含 4 项；只有一页时包含 3 项（没有 Delete）。
  - [x] 右键不改变 `pages_view.selected`。
  - [x] 页标题：有 `{title:}` / 模板名时取之，无 `&&&` 时显示 `Page N`，不显示 `page.name`。
  - [x] Delete 第一次点击进入确认态，第二次点击才删除；删除后留在 Pages 视图，删除非选中页按 `PromptPageId` 重新定位选中行，不漂移。
  - [x] 删除非当前页 → 编辑缓冲的撤销栈不变。
  - [x] 删除当前页 → `active_page` 变为下一页（如果是末页则为上一页），编辑缓冲载入该页。
  - [x] New page（正式条目）→ 留在 Pages 视图并推入 `Effect::CreatePage`；回调后仍留在列表、清空搜索、选中新页滚入视口并弹 `New page created`，不进入命名态。
  - [x] New page（临时提示词）→ 末尾追加，留在 Pages 视图并弹 `New page created`。
  - [x] 已有空白页时 `New page` / `Ctrl/Cmd+Shift+Right` 不新建，切到空白页并弹 `Already have a new page`。
  - [x] Copy content → 剪贴板内容为该页全文，并弹出 toast。

### P4.7 鼠标（design §9.5）

- [x] `AppState.page_row_click`；`press.rs` 增加 Pages 分支（单击选中，双击打开）。
- [x] `hover.rs`：Pages 复用 `prompt_tree_hover`，切换模式时清空。
- [x] `wheel.rs`：增加 Pages 分支。
- [x] `press.rs:420`：点击计数改为切换 Pages。
- [x] 测试（`event/tests/prompt_pages.rs`，新建）：
  - [x] 单击选中，不切页。
  - [x] 400ms 内双击同一行 → 打开该页。
  - [x] 先双击 Tree 再进入 Pages 单击 → 不会被误判为双击。
  - [x] 滚轮滚动不移动选中行。
  - [x] 点击计数 Editor ↔ Pages 往返。

### P4 收口

- [x] `cargo fmt`、`cargo clippy --all-targets -- -D warnings`、**全量 `cargo test`**。
- [x] commit：`feat(prompt): add page list view with context menu and Ctrl/Cmd+L/P toggles`

## P5 settings 与文档

- [x] `assets/settings.js:512` 文案改为 `Inside blocks (&&& / +++)`。
- [x] `docs/feature/prompt-block-focus/design.md`、`prompt-supplement-fold/design.md`、`prompt-pagination/design.md` 顶部加「已被 `prompt-block-restyle` 取代（部分/全部）」标注。
- [x] `docs/feature/prompt-supplement-blocks/design.md` §2 中「合法 `+++` 只能位于 `&&&` 内」改为指向本设计。
- [x] 本 `design.md` 的状态改为「已实施」，并追加「实施差异」一节（如有）。
- [x] commit：`docs(prompt): document block restyle and page list, mark superseded designs`

## P6 交互与视觉打磨

### 视觉与布局

- [x] `&&&` 色条 `▎` 与块内分割线固定为 `style::LX_PINK`（`Indexed 218`），不随状态变色；`+++` 保持品红。
- [x] 移除 `&&&` 起始行右上角的 `[status]` 按钮，`&&&` / `+++` 均只保留 `[❐]` / `[⌫]` / `[✕]` 3 个动作按钮。
- [x] 编辑视图标题行移除 `· 页名`，只显示条目名（临时提示词 `Temp prompt`）；Pages 视图标题行保留 `条目名 · pages`，搜索过滤结果为空时显示 `0/m`。
- [x] 编辑视图本页存在已闭合 `&&&` 时，在标题行页控件 `‹ n/m ›` 左侧 1 列渲染 `[todo]`/`[run]`/`[done]` 状态标签（蓝/黄/绿）。
- [x] 浮动面板（`/`、`@`、块命令、粘贴、引用）统一经 `ui/prompt/panels.rs::panel_content_area` 向右缩进 `PROMPT_BLOCK_GUTTER_LEFT`（1 列），不遮挡左侧 `▎` 色条。

### 交互

- [x] 状态标签支持点击循环 `todo → in_progress → done → todo`：单步可撤销、清除选区、不移动光标；悬停显示 `cycle block status` tooltip。
- [x] `Shift+Tab`（`EditorCommand::Outdent`）在页内任意光标位置生效：本页存在已闭合 `&&&` 时循环其状态，否则回退反缩进。
- [x] 编辑视图 `Esc` 只关闭浮层/补全面板，不再退回 Tree；Tree / Pages 视图按 `Esc` 仍返回 Editor。
- [x] 编辑视图新增 `Ctrl/Cmd+Shift+Right`（`EditorCommand::NewPage`），与 Pages 列表右键 `New page` 统一走 `create_or_toast_new_page`：已有空白页时切过去并弹 Warning `Already have a new page`，否则新建并弹 Default `New page created`。
- [x] Pages 列表新建页后留在列表、清空搜索、选中新页并滚入视口（编辑视图则切入新页）。
- [x] Pages 列表页行右键菜单移除 `Rename`，只保留 `Open / Copy content / New page / Delete`（单页时隐藏 `Delete`）；页标题由正文 `&&&` 的 `{title:}` / 模板名派生，无 `&&&` 时显示 `Page N`。

## P7 Code Review 缺陷修复（对应 review.md P0–P3）

### P0 未闭合块约束

- [x] `SlashContext` 新增 `unclosed_block`（`spans.iter().any(|s| !s.closed)`）；`slash_filter` 在存在未闭合 `&&&` 或 `+++` 时隐藏所有 `SuppleBlock` 候选（`/supplement` 与自定义 `+++`）。
- [x] `append_supple_block` 存在未闭合块时直接返回不插入；锚点只匹配 `span.closed == true` 的 `+++` / `&&&`。
- [x] `prompt_template_status` 与 `Prompt::cycle_template_status` 只处理已闭合 `&&&`：未闭合块不显示状态标签，`Shift+Tab` 回退反缩进；Pages 列表 `page_title_status` 保持未闭合 `&&&` 计为 `Todo`。

### P1 核心交互

- [x] Pages 列表右键 `New page`（`MenuCommand::NewPage`）改走 `create_or_toast_new_page`，与 `Ctrl/Cmd+Shift+Right` 一致；Pages 视图清空搜索、按行定位新页/空白页并 `clamp_page_view` 滚入视口，两条分支分别弹 `New page created` / `Already have a new page`。
- [x] 移除 Pages 右键 `Rename`：删除 `MenuCommand::RenamePage`、`RenameTarget::PromptPage`、`open_page_rename`、`commit_prompt_rename` 的 `PromptPage` 分支与 `ui/overlay/menu.rs` 对应分支；标题回退改为 `Page N`。
- [x] 鼠标点击标题行状态标签、块内 `[⌫]`（Clean）/ `[✕]`（Delete）按钮改写正文前调用 `update::clear_selection(state)`。
- [x] `restore_prompt_view` 在 `select_prompt_item` 后恢复原 `PromptMode::Pages` 并调用 `clamp_page_view`，正式条目与临时提示词的视图状态一致。

### P2 边界交互

- [x] `perform_delete_page` 删除前记录选中行的 `PromptPageId`，删除非选中页后按 id 重定位选中行并 `clamp_page_view`。
- [x] `clear_page_search`（`Ctrl+C`）清空前记录 `PromptPageId`（无则 `active_page`），清空后重定位并 `clamp_page_view`。
- [x] `prompt_page_header` 在 Pages 视图搜索无匹配（`rows.is_empty()`）时显示 `0/m`。
- [x] `prompt_page_control_at` 补 `state.right_tab == RightTab::Prompt` 守卫，并补 Todo 标签页不命中的测试。

### P3 代码整洁与文档

- [x] 删除 `impl From<bool> for PromptMode`，`prompt_toolbar_button_rect` / `prompt_toolbar_button_at` 签名收窄为 `mode: PromptMode`，更新 `layout/tests.rs` 调用方。
- [x] 删除 `Prompt::cycle_template_status_at_cursor`、`Effect::RenamePage`、`text::RENAME_PAGE_TITLE` 及测试引用。
- [x] 同步 `design.md`、`task.md`、`docs/standards/tui-design-requirements.md` 与最新实现（本文件本轮更新）。

### P6 / P7 收口

- [x] `cargo fmt`、`cargo clippy --all-targets -- -D warnings`、受影响模块测试与全量 `cargo test`。
- [x] P6、P7 变更按阶段提交。

## 8. 交付

- [x] 每个阶段的 commit 都已推进，`git log --oneline dev..` 显示 P1–P7 各阶段与文档同步 commit（数量以实际日志为准）。
- [x] 向用户汇报：修改要点、验证结果（测试数对比基线）、未验证的部分（TUI 外观与交互需要用户 `cargo run`）。
- [x] **不自行合并到 `dev`**。合并由用户确认后执行；合并前按 AGENTS.md 的要求核对 `SCHEMA_VERSION`（本分支不变）。

## 9. 已知遗留（不在本次范围）

- `src/app/prompt/tests.rs`（3162 行）和 `src/ui/prompt/tests.rs`（1896 行）存量超过 1500 行上限，需要单独立项按场景拆分。
- 页拖动排序没有做（决策 16）。
- Tree 视图条目的 `n|m|x` 计数在单 `&&&` 规则下最多是「页数」，是否改为圆点聚合单独评估。
