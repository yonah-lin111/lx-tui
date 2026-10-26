# Prompt 单模板块聚焦模式 — 实施任务

> 状态：**已实施并合并**（2026-10-27 合入 `dev`，见 `design.md` §13）。
>
> 前置阅读：同目录 `design.md`（决策与机制的唯一来源）。
>
> 全部 26 项边界已由用户逐条确认，本清单不再重复产品决策，只列可勾选的执行步骤。

## 0. 实施前复核（必须先做）

- [x] 0.1 用 codegraph / codebase-memory 复查这批符号的**当前**位置与调用者：
  `visual_rows`、`cursor_cell`、`cursor_visual_in`、`content_offset_at`、`max_scroll`、
  `bottom_padding`、`row_in_padding`、`select_all_prompt`、`template_buttons`、
  `block_buttons`、`prompt_scrollbar_for`、`settle`、`record`、`restore`、`snapshot`、
  `show_prompt_tree`、`load_editor_target`、`reload_editor_from_active_page`、
  `swap_display_prompt`、`restore_prompt_view`、`prompt_toolbar_button_rect`、
  `prompt_toolbar_button_at`、`render_header`、`render_toolbar`、`editor_command`、`apply_editor`。
- [x] 0.2 复核 `visual_rows()` 返回空 vec 时**全部 4 处**调用方的行为（design §3.2 / §10）：
  `cursor_visual_in`、`content_offset_at`、`cursor_cell`、`ui/prompt.rs::render`。
  任何一处存在 `rows[i]` 无保护索引都要在本任务内补保护并加用例。
- [x] 0.3 复核 `Prompt::set_text` / `restore_history` / 撤销 `Snapshot` 三处的字段清单，
  确认新增 `focus_block` 后没有遗漏的构造点（Rust 结构体字面量会强制覆盖，但 `..Default` 写法不会）。
- [x] 0.4 复核 `src/layout.rs` 是否已因其他分支逼近/超过 1000 行；若是，先完成拆分
  （见 0.5）再动手。
- [x] 0.5 **前置重构**：把 prompt 区域几何（`prompt_text_rect` / `prompt_content_rect` /
  `prompt_scrollbar_rect` / `prompt_tree_scrollbar_rect` / `prompt_body_rect` /
  `prompt_header_rect` / `prompt_border_rect` / `prompt_toolbar_rect` /
  `prompt_divider_rect` / `prompt_toolbar_button_rect` / `prompt_toolbar_button_at` /
  `prompt_inner_size` 及相关常量）拆到 `src/layout/prompt.rs`，
  `layout.rs` 用 `mod prompt; pub use prompt::*;` 保持所有现有路径不变。
  **拆分必须是纯搬迁**，不改任何行为；搬迁后先跑 `cargo test` 确认零回归。
- [x] 0.6 确认 `Cargo.toml` 的 `unicode-width` 版本对 `⊞`(U+229E)/`⊡`(U+22A1)/`○`(U+25CB)/
  `◉`(U+25C9) 的宽度计算为 1（design §7.1 风险项）。

## 1. 状态层（app）

- [x] 1.1 `src/app/prompt.rs`：`Prompt` 增加 `focus_block: Option<usize>`（注释写清
  「顶层 `&&&` 起始逻辑行；None = 普通视图」），`new()` 初始化为 None。
- [x] 1.2 `Snapshot` 增加 `focus_block: Option<usize>`；`snapshot()` 写入、`restore()` 读回。
- [x] 1.3 `PromptHistory` 增加 `focus_block: Option<usize>`；`history()` 写入、
  `restore_history()` 读回后调用 `reconcile_block_focus()`。
- [x] 1.4 `Prompt::settle()` 末尾调用 `self.reconcile_block_focus()`。
- [x] 1.5 `src/app/prompt.rs` 增加 `mod block_focus;`。

## 2. 块聚焦核心（新文件）

- [x] 2.1 新建 `src/app/prompt/block_focus.rs`，实现：
  - `focused_block_range(&self, scan: &BlockScan) -> Option<(usize, usize)>`
  - `template_starts(scan) -> Vec<usize>`
  - `block_focus_active(&self) -> bool`
  - `enter_block_focus(&mut self) -> bool`（design §4.1）
  - `exit_block_focus(&mut self) -> bool`
  - `focus_adjacent_block(&mut self, delta: isize) -> bool`（§4.2，两端钳制）
  - `focus_after_block_removed(&mut self, removed_start: usize)`（§4.3）
  - `reconcile_block_focus(&mut self)`（§4.4）
  - `title_value_offset(text, start_line) -> usize`（标题值落点，§4.1）
- [x] 2.2 保证 `enter_block_focus` 在块数为 0 时返回 `false` 且不改 `focus_block`。
- [x] 2.3 保证「进入时保留块内光标」与「切换时总是移动光标」两条路径分开实现，
  不要在 `enter` 里复用 `focus_adjacent_block` 的光标逻辑。

## 3. 视口过滤

- [x] 3.1 `src/app/prompt/viewport.rs::visual_rows()` 增加聚焦行过滤（design §3.2），
  `focus` 为 None 时行为与现状逐字节一致。
- [x] 3.2 为 `visual_rows()` 返回空 vec 的边界补用例：块被删后同一帧调用
  `cursor_cell` / `set_cursor_from_cell` / `selection_text` / `max_scroll` 均不 panic。

## 4. 命令与输入

- [x] 4.1 `src/app/actions.rs`：新增 `BlockPrev { cmd: bool }` / `BlockNext { cmd: bool }`。
- [x] 4.2 `src/input/mod.rs::editor_command`：Super 分支新增 `Left`/`Right`；
  Ctrl 分支把 `Left`/`Right` 从 `WordLeft/WordRight` 改为 `BlockPrev/BlockNext { cmd: false }`；
  Alt 分支保持不变。
- [x] 4.3 `src/app/update/editor.rs::apply_editor`：新增两个分支（design §5.3）：
  块模式内调 `focus_adjacent_block`；普通视图 `cmd == false` 回退 `move_word_*`，
  `cmd == true` 吞掉。分支位置在 `route_panel` 之后、选区处理之前。
- [x] 4.4 复核 `EditorCommand` 的派生 `PartialEq` 与新字段兼容（`BlockPrev { cmd }` 用
  结构体变体而非两个变体，避免 `matches!` 处漏改）。

## 5. 删除接续

- [x] 5.1 `src/app/update/editor.rs::delete_template_block`：删除成功后若原本处于块模式，
  调用 `focus_after_block_removed(start_line)`（design §4.3）。
- [x] 5.2 同文件 `clean_template_block` / `toggle_template_status`：无需额外处理
  （`settle()` 内的 `reconcile_block_focus()` 已兜底），但在测试中锁定「清理后仍在原块」。

## 6. 进入/退出路径

- [x] 6.1 `src/app/update/prompt.rs::show_prompt_tree`：**不要**清空块聚焦；
  `stash_editing_prompt` 会把 `focus_block` 随 `PromptHistory` 一并暂存，
  返回同一目标时恢复（design §4.6）。
- [x] 6.1a `open_temp_prompt` 与 `restore_prompt_view` 兜底空草稿：`set_text("")` 前
  调用 `exit_block_focus()`，避免新草稿继承旧聚焦行。
- [x] 6.2 `src/app/update/prompt.rs::load_editor_target`：`set_text` / `restore_history`
  之前调用 `exit_block_focus()`。
- [x] 6.3 `src/app/update/prompt/pages.rs::reload_editor_from_active_page`：`set_text` 前
  调用 `exit_block_focus()`。
- [x] 6.4 `src/app/update/prompt.rs`：新增 `toggle_prompt_block_focus(state) -> bool`。
- [x] 6.5 `src/app/update/pane.rs::resize_panes`：prompt 尺寸同步时，若模式按钮矩形为
  `None` 且当前处于块模式 → `exit_block_focus()`（design §6.3）。

## 7. 工具栏几何

- [x] 7.1 `src/layout/prompt.rs`（0.5 拆分后的落点）：`PromptToolbarButton` 增加 `BlockFocus`。
- [x] 7.2 常量：`PROMPT_PIN_WIDTH: 7 → 3`；新增 `PROMPT_BLOCK_FOCUS_WIDTH = 3` 与
  `PROMPT_BLOCK_FOCUS_MIN_WIDTH_EDITOR = 23`；`PROMPT_PIN_MIN_WIDTH_EDITOR` 保持 27、
  `PROMPT_PIN_MIN_WIDTH_TREE: 18 → 14`、`PROMPT_FOLDERS_MIN_WIDTH_EDITOR` 保持 33；
  `PROMPT_SELECT_ALL_MIN_WIDTH` 保持 19；注释同步写清新的推导（design §6.1 表格）。
- [x] 7.3 `prompt_toolbar_button_rect`：`SelectAll` 改为左组 `select_all_x`；
  新增 `BlockFocus` 分支；`Pin` 的编辑视图锚点改为 `BlockFocus`。
- [x] 7.4 `prompt_toolbar_button_at` 的候选数组加入 `BlockFocus`（顺序不影响命中，
  但必须列全）。
- [x] 7.5 复核右组右对齐不变量：`BlockFocus.right() + GAP == Save.x`、
  `Pin.right() + GAP == BlockFocus.x`、`Folders.right() + GAP == Pin.x`（编辑视图）。

## 8. 文案与渲染

- [x] 8.1 `src/ui/text.rs`：新增 `PROMPT_BLOCK_FOCUS_ALL_LABEL = "[⊞]"`、
  `PROMPT_BLOCK_FOCUS_ONE_LABEL = "[⊡]"`；`PROMPT_UNPINNED_LABEL = "[○]"`、
  `PROMPT_PINNED_LABEL = "[◉]"`；更新两处注释（去掉「纯英文单词、7 列占位」的说法）。
- [x] 8.2 `src/ui/prompt.rs::render_toolbar`：
  - 编辑视图按钮改为 `[≡][←][→][all]` + `[dir][○/◉][⊞/⊡]●`；
  - 树视图保持 `[≡]` + `[○/◉]` + `[tmp]`；
  - **删除右对齐补空格的偏移逻辑**（design §7.2），改回左对齐逐字符绘制；
  - 新增参数 `block_focus: bool` 与 `block_focus_available: bool`
    （不可用时 `style::muted()`）。
- [x] 8.3 `src/ui/prompt/header.rs::render_header`：签名增加
  `block_focus: bool`、`block_focus_available: bool` 并透传。
- [x] 8.4 `src/ui/prompt_frame.rs::render_body`：用
  `state.prompt.block_focus_active()` 与
  `block_scan(text).spans.iter().any(|s| s.kind == BlockKind::Template)` 计算两个参数。
- [x] 8.5 复核 `block_scan` 每帧调用成本：`render_body` 已间接调用多次扫描
  （`markdown::scan`、`block_line_infos`、`visual_rows`），新增一次 `block_scan`
  可接受；不要为此引入缓存。

## 9. Tooltip

- [x] 9.1 `src/app/tooltip.rs`：`TooltipTarget` 增加 `PromptBlockFocus`。
- [x] 9.2 `src/ui/tooltip.rs`：prompt 按钮目标表加入
  `(PromptToolbarButton::BlockFocus, TooltipTarget::PromptBlockFocus)`。
- [x] 9.3 `src/ui/text.rs`：`TooltipTarget::PromptBlockFocus` → 模式内
  `"show all blocks"`、模式外 `"focus template block"`。

## 10. 事件层

- [x] 10.1 `src/event/mouse/press.rs`：工具栏 `match button` 增加
  `PromptToolbarButton::BlockFocus => { update::toggle_prompt_block_focus(state); }`。
- [x] 10.2 复核 `editor_view` 闸门：`BlockFocus` 只在编辑视图存在，
  树视图下该按钮矩形为 `None`，无需额外判断。
- [x] 10.3 复核 `src/event/input.rs` 是否需要改动：`BlockPrev/BlockNext` 走
  `Routed::Editor` 现有通道，**不需要**改。

## 11. 测试

- [x] 11.1 `src/app/prompt/tests.rs`（或新建 `src/app/prompt/block_focus/tests.rs`）：
  design §9.1 全部用例。
- [x] 11.2 `src/input/tests.rs`：design §9.2 全部用例。
- [x] 11.3 `src/app/update/prompt/tests.rs`：design §9.3 全部用例。
- [x] 11.4 `src/layout/tests.rs`：design §9.4 全部用例（含阈值 ±2 边界）。
- [x] 11.5 `src/ui/prompt/tests.rs`、`src/ui/tests.rs`、`src/ui/tooltip/tests.rs`：
  design §9.5 全部用例；**同时更新**所有引用旧 pin 文案/宽度的断言
  （`ui/tests.rs:1529/1703/1714/1763/1773`、`ui/prompt/tests.rs:1744/1764/1768/1786`、
  `ui/prompt/tests.rs:649-690` 的左组位置断言、`event/tests.rs:3138-3160` 的 pin 点击）。
- [x] 11.6 `src/ui/text/tests.rs`：把 `[○]`/`[◉]` 从「单词形态」列表移到「图标形态」
  列表（design §7.1），并把 `[⊞]`/`[⊡]` 也加进图标列表。
- [x] 11.7 新增回归：`event/tests.rs` 中点击 `[⊞]` 进入模式后，
  再点 `[⊡]` 退出，并断言 `state.prompt.text()` 未变（纯视图操作）。

## 12. 文档

- [x] 12.1 `docs/standards/tui-design-requirements.md` 第 25 行（prompt 右栏）：
  工具栏顺序、pin 单字图标 3 列、`[all]` 左组、新模式按钮与块聚焦行为段。
- [x] 12.2 同文件第 65 行（prompt 编辑器键位段）：
  `Ctrl+←/→` 在块模式内切块、模式外按词；`Cmd+←/→` 模式内切块、模式外吞掉；
  新增「块聚焦模式」一段（进入/切换/退出/记忆/窄屏自动退出/`[all]` 范围/删除接续）。
- [x] 12.3 本目录 `design.md` 的状态行在实施后改为「已实施（见 §13）」，并追加实施记录
  （与设计初稿的差异）。

## 13. 精确验证与交付

- [x] 13.1 `cargo fmt`
- [x] 13.2 `cargo clippy --all-targets -- -D warnings`
- [x] 13.3 `cargo test`（受影响模块优先，提交前全量）
- [x] 13.4 交付用户 `cargo run` 交互验收（用户确认合并，视为通过）：按钮点击、键位切块、块模式渲染、
  删除接续、切工作区恢复、窄屏退化、`⊞`/`⊡`/`○`/`◉` 字形在各终端下的宽度。

## 14. 已知风险（实施时逐条确认，不额外发明方案）

| 风险 | 处理 |
| --- | --- |
| `visual_rows()` 空 vec | 0.2 复核 + 3.2 用例 |
| `○`(U+25CB) EAW=Ambiguous | 项目已在用 `PROMPT_DRAFT_DOT = "○"`；若某终端按 2 列渲染需用户实测反馈，本任务不改字形 |
| `[all]` 左移后极窄面板挤压 `[←][→]` | 各自独立门控，阈值固定，layout 测试锁边界 |
| 块聚焦 + `+++` 折叠叠加 | 不特殊处理，规则叠加 |
| `settle()` 内多一次 `block_scan` | 仅模式内执行，普通视图提前返回 |
| `src/layout.rs` 已 1000 行 | 0.5 前置纯搬迁拆分 |
