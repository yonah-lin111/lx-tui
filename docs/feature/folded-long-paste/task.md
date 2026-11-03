# 长文本折叠粘贴（prompt 编辑器与 lx 输入框）：执行任务清单

> 状态：**已在工作区 `.worktrees/folded-long-paste`（分支 `feat/folded-long-paste`）全部完成并通过验证**
>
> 关联文档：
> - 设计方案：`docs/feature/folded-long-paste/design.md`
> - 项目规范：`docs/standards/code-writing-standards.md`、`docs/standards/project-directory-structure.md`、`docs/standards/tui-design-requirements.md`

---

## 0. 实现边界（先读）

- 只改两个主输入框：`Prompt` 正文、`Lx` Chat 输入框。
- 不改数据库 schema；token 存在正文里。
- 路径粘贴面板逻辑保持原样，长文本只在“非路径”分支处理。
- 展开/折叠是纯视图状态，不写缓冲区、不触发自动保存、不进撤销栈。
- 所有生产代码遵守：无 `unwrap()`/`expect()`、文案集中 `src/ui/text.rs`、渲染只读、`app/` 不依赖 `ui/`。
- 新模块建议控制行数：`folded.rs` 目标 ≤ 600 行，测试拆分；若超 1000 行必须按职责拆子模块。
- `src/app/prompt.rs`（956 行）与 `src/app/state/lx.rs`（992 行）已接近 1000 行上限：
  - Prompt 的粘贴/展开方法放 `src/app/prompt/folded.rs`，`prompt.rs` 只加字段与 `mod folded;`；
  - Lx 的粘贴/展开方法放 `src/app/state/lx/folded.rs`；
  - 若 lx 编辑方法因原子边界改动超过 1000 行，先把 `insert_*`/`delete_*`/`move_*` 抽到 `src/app/state/lx/edit.rs`，再继续功能。

---

## Task 1：新增折叠 token 核心模块 `src/app/paste/folded.rs`

### 1.1 类型与编解码

- [x] 新建 `src/app/paste/folded.rs`，在 `src/app/paste.rs` 声明 `mod folded;` 并 `pub use`。
- [x] 定义 `FoldedToken { range, line_count, body, preview, id }`。
- [x] 实现 `escape_body` / `unescape_body` / `encode`：

```rust
pub fn escape_body(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for ch in text.chars() {
        match ch {
            '\\' => out.push_str("\\\\"),
            ')' => out.push_str("\\)"),
            _ => out.push(ch),
        }
    }
    out
}

pub fn encode(content: &str) -> String {
    let line_count = content.split('\n').count().max(1);
    format!("@[pasted:{line_count}]({})", escape_body(content))
}
```

- [x] `id` 用稳定哈希（`DefaultHasher` 即可，进程内身份；不跨进程持久化）。
- [x] 预览：第一条非空行，去行尾空白，40 显示列截断 + `…`；无内容时 `"{N} lines"`。
- [x] 单块上限常量 `MAX_FOLDED_BYTES: usize = 256 * 1024`，供面板与候选判断共用。

### 1.2 解析

- [x] `pub fn tokens(text: &str) -> Vec<FoldedToken>`。
- [x] 逐行扫描并维护 `fence` 状态；行首围栏标记只在 token 外生效。
- [x] 逐字符扫描 `@[pasted:`；解析 N、`](`、第一个未转义 `)`。
- [x] `\\` / `\)` 跳过；其他 `\x` 原样保留。
- [x] 围栏内候选直接跳过；token 整段不参与围栏状态（正文里的 ``` 不关外层围栏）。
- [x] 空正文、缺 `)`、非法 N 都按普通文本继续，不 panic。
- [x] 重复 token 由调用方按顺序计算 `occurrence`（投影层负责）。

### 1.3 展开状态

- [x] 定义 `ExpandedPastes { entries: Vec<(u64, usize)> }`。
- [x] `toggle(id, occurrence)`、`is_expanded(id, occurrence)`、`clear()`。
- [x] 不按字节偏移保存；编辑前后身份稳定。
- [x] 提供 `pub fn expand_all(text: &str) -> String`：把所有 token 替换为解码正文；无 token 时返回原串的 owned 值。
- [x] 提供 `pub fn mask(text: &str) -> Cow<'_, str>`：token 区间替换为等长空白（保留 `\n`）；无 token 时借用原串。

### 1.4 投影

- [x] 定义 `Projection<'a> { text: Cow<'a, str>, segments: Vec<Segment>, tokens: Vec<FoldedToken> }`。
- [x] `Projection::new(text, &expanded)`：
  - 无 token：`Cow::Borrowed` + 单个 Plain segment。
  - 有 token：普通段原样；折叠 token 段用 `label()`；展开 token 段用 `body`。
- [x] `Segment { display, raw, kind }`；`SegmentKind::{Plain, Chip { token, expanded, occurrence }}`。
- [x] 实现：
  - `raw_to_display(raw) -> usize`（token 内夹到起点）；
  - `display_to_raw(display) -> usize`（token 内夹到起点）；
  - `chip_at(display) -> Option<ChipHit { token, expanded, occurrence }>`；
  - `copy_display_range(range) -> String`（Plain/展开取显示切片；折叠整块取 raw token）；
  - `display_line(display) -> usize` / `raw_line(display) -> usize`（供 Markdown token 与块信息查找）。
- [x] 不实现 `Display`/`Serialize`；保持纯数据。

### 1.5 原子编辑辅助

- [x] `snap_range(text, range) -> Range<usize>`：部分覆盖 token 时扩展到整块边界。
- [x] `prev_boundary(text, offset) -> usize`、`next_boundary(text, offset) -> usize`：左右跨过 token。
- [x] `range_overlaps_token(text, range) -> bool`。
- [x] 仅在 token 外做普通字符边界计算；token 内一律吸附。

### 1.6 测试

- [x] 新建 `src/app/paste/folded/tests.rs`，`#[cfg(test)] mod tests;`。
- [x] 覆盖设计文档 §10.1 全部用例。
- [x] 重点回归：正文含 `)`、`\`、`@[pasted:`、```、`&&&`、`+++`、CJK、emoji、空行。
- [x] mask 前后字节长度相等、换行位置相等。

---

## Task 2：新增长文本粘贴面板状态机 `src/app/paste/text.rs`

- [x] 新建 `src/app/paste/text.rs`，在 `src/app/paste.rs` 声明并 `pub use`。
- [x] 定义：

```rust
pub enum TextPasteAction { Collapse, Full }

impl TextPasteAction {
    pub const ALL: [Self; 2] = [Self::Collapse, Self::Full];
}

pub struct TextPastePanel {
    /// 用于折叠的 trim 后正文。
    pub content: String,
    /// 原文插入用的归一化文本（保留首尾空白）。
    pub full_text: String,
    /// 粘贴锚点（prompt 支持选区；lx 使用光标）。
    pub anchor: PasteAnchor,
    /// 是否超过折叠上限。
    pub too_large: bool,
    active: usize,
}

pub struct TextPasteState {
    panel: Option<TextPastePanel>,
}
```

- [x] `new(content, full_text, anchor) -> Self`：计算 `too_large`，`active` 落到首个可用项。
- [x] `enabled(action)`：`Collapse` 在 `too_large` 时 false；`Full` 恒 true。
- [x] `active()`、`active_action()`、`set_active(index)`、`move_active(delta)`（跳过禁用项，循环）。
- [x] `cancel()`、`open(panel)`、`panel()`、`panel_mut()`。
- [x] `candidate(text) -> Option<TextPasteCandidate>`：
  - 归一化 CRLF/CR，再走 `prompt::text::sanitize`；
  - `trim()` 后计算行数与 Unicode 字符数（`chars().count()`）；
  - `>=3 行 || >150 字符` 才算候选；
  - 返回 sanitized 的 trimmed content、sanitized 的 full_text（未 trim）；
  - 空文本返回 None。
- [x] 测试 `src/app/paste/text/tests.rs`：阈值、超限、默认项、移动跳过禁用、取消。

---

## Task 3：Prompt 编辑器集成

### 3.1 状态与 API

文件：`src/app/prompt.rs`、`src/app/prompt/paste.rs`、`src/app/prompt/viewport.rs`、`src/app/prompt/navigation.rs`。

- [x] `Prompt` 增加：
  - `text_paste: TextPasteState`
  - `expanded_pastes: ExpandedPastes`
  - `preview_selection: Option<(usize, usize)>`
- [x] 方法放 `src/app/prompt/folded.rs`（`prompt.rs` 加 `mod folded;`），避免 `prompt.rs` 超过 1000 行。
- [x] 实现：
  - `text_paste_panel()`
  - `begin_text_paste(content, full_text, anchor) -> bool`（受保护行/围栏内返回 false）
  - `text_paste_move(delta)`
  - `text_paste_confirm(action)`
  - `text_paste_cancel()`
  - `projection()`
  - `toggle_paste(id, occurrence)`
  - `clear_expanded_pastes()`
  - `preview_selection()` / `set_preview_selection()`
- [x] 确认插入：
  - Collapse：`encode(content)`，用现有 `paste_insertion` 补前后空格，`replace_at(anchor.start, anchor.end, &token)`，一步撤销；
  - Full：`replace_at(anchor.start, anchor.end, full_text)`，一步撤销；
  - 插入后 `clear_other_panels()`、光标落到插入末尾、关闭面板。
- [x] 取消：只关闭面板，不触碰撤销栈与缓冲区。
- [x] `undo`/`redo` 开头：若 `text_paste_panel` 打开则 return（与路径面板一致）；成功后 `clear_expanded_pastes()`。
- [x] `set_text` / `restore_history` / 切换编辑目标：`clear_expanded_pastes()`、`text_paste_cancel()`。
- [x] `protected_edit` 改用 `mask_pasted_text()` 后的文本判定。
- [x] `settle()` 的 mention/refer/slash/block 触发扫描改用 mask 文本。
- [x] `in_fence()` 改用 mask 文本。

### 3.2 视觉行与坐标

- [x] `visual_rows()` 基于 `Projection::text` 软换行；`VisualRow.start/end` 改为投影文本字节偏移；`line` 保持原始行号。
- [x] `cursor_cell()` 先 `raw_to_display(cursor)`。
- [x] `set_cursor_from_cell()` 先 `display_to_raw()`；命中芯片时夹到芯片边界。
- [x] `up/down/home/end` 目标落在展开芯片内部时跳到芯片外侧。
- [x] `selection_bounds` / `selection_text_with` 按设计 §4.4 的坐标表改：
  - `selection_bounds_with` 显示入、原始出，供编辑/删除；
  - `selection_text_with` 显示入、复制文本出，统一走 `Projection::copy_display_range`；
  - `cell_range_for_byte_span` 原始入、显示出，先 `raw_to_display`。
- [x] 新增预览选区复制路径；普通选区复制保持“折叠 token / 展开显示文本”的语义。

### 3.3 原子编辑

- [x] `backspace`：光标在 token 内或紧跟 token 末尾时，整块删除；在 token 起点前时普通删除。
- [x] `delete`：光标在 token 起点或内部时，整块删除。
- [x] `insert_at` / `splice_text` / `replace_range`：编辑区间先 `snap_range`；部分覆盖 token 时扩展为整块。
- [x] `move_left` / `move_right`：`prev_boundary` / `next_boundary`。
- [x] `home` / `end`：结果夹到 token 边界。
- [x] 列表续写、缩进调整等触碰 token 时同样吸附。
- [x] 所有插入/删除沿用现有 `record`/`snapshot` 一步撤销语义。

### 3.4 渲染

文件：`src/ui/prompt.rs`、`src/ui/prompt/panels.rs`、`src/ui/markdown.rs`。

- [x] `render()` 用 `prompt.projection()` 的显示文本；`markdown::scan(mask_pasted_text(prompt.text()))` 得到原始 token。
- [x] `paint_row()`：
  - 普通段：`display_to_raw` → 原始 Markdown token 样式；
  - 折叠芯片：`style::pasted_chip()`；
  - 展开区：`style::pasted_bg()`，行首 2 列缩进（同底色）；
  - 宽字符后续列 `CellDiffOption::Skip`。
- [x] `render_panels()` 增加文本粘贴面板，优先级高于路径面板。
- [x] `src/ui/prompt/panels.rs` 新增 `TextPastePanelData`、`text_paste_view`、`text_paste_layout`、`text_paste_item_at`、`text_paste_panel_rect`。
- [x] `CommandPanelView` 增加 `disabled: Option<&[bool]>`（若采用置灰方案）：
  - `render_item` 对 disabled 行用 `style::muted()` 且不画 `❯`；
  - `item_at` 仍返回索引，由业务层拒绝；
  - 现有面板传 `None`，行为不变。
- [x] 预览选区绘制：按投影文本字节范围计算视觉行列，反显；与普通选区互斥。

### 3.5 鼠标/键盘路由

文件：`src/event/paste.rs`、`src/event/input.rs`、`src/event/mouse/press.rs`、`src/event/mouse/drag.rs`、`src/event/mouse/wheel.rs`、`src/app/update/editor.rs`。

- [x] `handle_terminal_paste`：
  - 先取消文本/路径面板；
  - 路径检测优先；
  - 再长文本候选；
  - 围栏/受保护行直接原文插入；
  - 最后普通插入。
- [x] `apply_editor` / `route_paste_panel`：文本面板打开时优先处理 `Up/Down/Enter/Escape`，其余键吞掉。
- [x] 鼠标面板命中：文本面板优先于路径面板。
- [x] 鼠标点击 prompt 文本区：
  - 先 `text_paste_item_at`；
  - 再芯片命中：折叠→展开，展开→开始预览选区；
  - 再普通光标/选区；
  - 点击面板外取消文本面板。
- [x] 拖拽：预览选区拖动只更新显示区间，不移动光标。
- [x] 松开：未拖动 → 收起；拖动 → 保留选区。
- [x] 滚轮：预览区滚动输入框，不触发收起。
- [x] 复制：预览选区优先；否则现有选区/全文逻辑。

### 3.6 保存/复制

文件：`src/app/update/prompt.rs`、`src/app/update/selection.rs`。

- [x] `commit_prompt_buffer` 保持存原始 token（不改）；提交前同时调用 `text_paste_cancel()`（与现有 `paste_cancel()` 并列），未确认的粘贴提案不落库。
- [x] `prompt_copy_text`：有预览选区时返回 `projection.copy_display_range`；否则现有逻辑。
- [x] `prompt_selection_text`：普通选区仍按原始字节；若选区完全在展开芯片内，走预览复制。
- [x] 模板块复制按钮保持原始 token（不展开），与“复制 token”决策一致。

### 3.7 测试

- [x] 新建 `src/app/prompt/tests/paste_folded.rs`，并在 `src/app/prompt/tests.rs` 声明子模块。
- [x] 覆盖设计 §10.2 全部用例。
- [x] 更新 `src/ui/prompt/tests.rs` 渲染断言。
- [x] 更新 `src/event/tests/focus_and_paste.rs` 端到端用例。

---

## Task 4：Lx 输入框集成

### 4.1 状态与 API

文件：`src/app/state/lx.rs`、`src/app/state/lx/panels.rs`。

- [x] `Lx` 增加：
  - `text_paste: Option<LxTextPaste>`
  - `expanded_pastes: ExpandedPastes`
  - `preview_selection: Option<(usize, usize)>`
- [x] 方法放 `src/app/state/lx/folded.rs`（`lx.rs` 加 `mod folded;`），避免 `lx.rs` 超过 1000 行；若编辑方法改动后仍超限，按规范先把编辑方法抽到 `src/app/state/lx/edit.rs`。
- [x] `LxTextPaste { panel: TextPastePanel }`。
- [x] 实现：
  - `text_paste_panel()`
  - `begin_text_paste(content, full_text) -> bool`
  - `text_paste_move(delta)`
  - `text_paste_confirm(action)`
  - `text_paste_cancel()`
  - `projection()`
  - `toggle_paste(id, occurrence)`
  - `clear_expanded_pastes()`
  - `expanded_input() -> String`
  - `paste_hit_at(row, col) -> Option<ChipHit>`
  - `preview_selection()` / `set_preview_selection()`
- [x] 确认插入：
  - 沿用 `insert_str` 语义（有选区先替换选区），但 token 需要作为一步撤销插入；
  - Collapse：插入 `encode(content)` + 尾随空格；
  - Full：插入 `full_text`；
  - 插入后 `panel = None`、`mention.suppress()`、`scroll_cursor_into_view()`。
- [x] 取消：只关闭面板。
- [x] `refresh_panel` Shell 分支清空 `text_paste`；`clear`/会话切换清空展开与预览选区。
- [x] `undo`/`redo` 成功后 `clear_expanded_pastes()`。
- [x] `import_full_text`、`apply_history`、`clear` 清空展开与预览选区。
- [x] `expanded_input()`：`folded::expand_all(&self.input)`；Shell 模式走 `shell::serialize` 后再展开（防御）。

### 4.2 原子编辑

- [x] `insert_char` / `insert_str` / `newline`：有选区先 `snap_range`，插入后光标不落入 token。
- [x] `backspace` / `delete`：token 边界整块删除。
- [x] `delete_to_line_start` / `delete_to_line_end` / `delete_word_backward` / `delete_word_forward`：先 `snap_range`。
- [x] `move_left` / `move_right`：`prev_boundary` / `next_boundary`。
- [x] `move_up` / `move_down` / `home` / `end`：结果夹到 token 边界；展开芯片内部行不可作为光标目标。
- [x] `begin_text_selection_with_clicks` / `drag_text_selection`：`offset_at_cell` 返回的 raw offset 夹到 token 边界；原子芯片整块选中。

### 4.3 视觉行与渲染

文件：`src/ui/main_content/lx/input.rs`、`src/ui/main_content/lx/panels.rs`、`src/ui/main_content/lx.rs`。

- [x] `visual_rows()` 基于 `Projection::text`；`LxRow.start/end` 为投影文本字节偏移。
- [x] `cursor_visual_in` 使用 `raw_to_display(cursor)`。
- [x] `offset_at_cell` 使用 `display_to_raw`；新增 `display_offset_at_cell` 供预览选区。
- [x] `text_selection_range` / `selected_text` 继续使用原始字节区间；`offset_at_cell` 返回原始偏移时吸附到 token 边界。展开区选区单独存投影文本区间，不混入普通选区。
- [x] `paint_line` 按普通段/芯片段绘制；普通段用 mask 后的 Markdown token，芯片段用 `pasted_chip` / `pasted_bg`。
- [x] 展开区行首 2 列缩进（同底色）；软换行续行同样缩进。
- [x] `render_paste_panel` 按文本/路径状态互斥渲染；新增文本面板命中/布局/矩形。
- [x] 预览选区反显。
- [x] `input_geom` / `input_scrollbar` 口径不变，但高度随展开内容变化。

### 4.4 键盘/鼠标路由

文件：`src/app/update/pane.rs`、`src/app/update/pane/popups.rs`、`src/event/mouse/press.rs`、`src/event/mouse/drag.rs`、`src/event/mouse/hover.rs`、`src/event/mouse/wheel.rs`、`src/app/update/selection.rs`。

- [x] `apply_lx_input` 在路径面板分支前增加文本面板分支：`Up/Down/Enter/Escape`，其余键吞掉。
- [x] `select_lx_paste` 增加文本面板确认入口。
- [x] 鼠标按下 lx 输入框：
  - 先文本面板命中；
  - 再芯片命中：折叠→展开，展开→开始预览选区；
  - 再普通输入框选区。
- [x] `drag_lx_input_selection` 判断预览选区拖动，更新显示区间而非 raw 选区。
- [x] `end_lx_input_selection_drag`：未拖动→收起；拖动→保留。
- [x] 鼠标悬停面板项高亮。
- [x] 滚轮面板/输入框滚动。
- [x] `Ctrl/Cmd+C`：预览选区优先。

### 4.5 提交与历史

文件：`src/app/update/chat/mod.rs`、`src/app/update/chat/user_shell.rs`、`src/app/update/chat/session.rs`、`src/app/update/chat/title.rs`。

- [x] `try_send` 的 `draft` 改用 `expanded_input().trim().to_string()`。
- [x] `queue_chat_while_shell_runs` 的 `draft` 改用 `expanded_input()`。
- [x] `try_submit_user_shell` 的草稿取值改用 `expanded_input()`（Shell 无 token，纯防御）。
- [x] `QueuedSubmission::Chat` 存展开文本。
- [x] 标题生成、prompt 历史记录使用展开文本。
- [x] 会话切换/清空输入时清空展开与预览选区。

### 4.6 测试

- [x] 新建 `src/app/state/lx/tests/paste_folded.rs`，并在 `src/app/state/lx/tests.rs` 声明子模块。
- [x] 覆盖设计 §10.3 全部用例。
- [x] 更新 `src/ui/main_content/lx/input/tests.rs` 渲染/命中断言。
- [x] 更新 `src/event/tests/focus_and_paste.rs`、`src/event/tests/lx_view.rs` 端到端用例。
- [x] 新增发送路径回归：`try_send` 必须把 token 解码后再交给 `prepare_user_message`。

---

## Task 5：样式与文案

### 5.1 `src/ui/style.rs`

- [x] 新增 `pasted_chip()`：

```rust
pub fn pasted_chip() -> Style {
    Style::default()
        .fg(Color::Yellow)
        .bg(Color::Indexed(236))
        .add_modifier(Modifier::BOLD)
}
```

- [x] 新增 `pasted_bg()`：

```rust
pub fn pasted_bg() -> Style {
    Style::default().bg(Color::Indexed(236))
}
```

- [x] 不加 RGB；不新增主题配置。
- [x] 补充 `style/tests.rs`：颜色、背景、BOLD 断言。

### 5.2 `src/ui/text.rs`

- [x] 新增：

```rust
pub const TEXT_PASTE_PANEL_TITLE: &str = "Paste";
pub const TEXT_PASTE_ACTION_COLLAPSE: &str = "Collapse paste";
pub const TEXT_PASTE_ACTION_FULL: &str = "Paste full text";
pub const TEXT_PASTE_TOO_LARGE: &str = "too large to collapse";
pub fn text_paste_lines(count: usize) -> String { format!("{count} lines") }
```

- [x] 所有用户可见文案英文，禁止散落在 UI 组件。

---

## Task 6：面板置灰能力（共享组件）

文件：`src/ui/widgets/command_panel.rs`、`src/ui/widgets/command_panel/tests.rs`。

- [x] `CommandPanelView` 增加 `disabled: Option<&'a [bool]>`。
- [x] `render_items`：disabled 项用 `style::muted()`，不画 `❯`；不响应 hover 高亮。
- [x] `item_at` 保持返回索引；业务层根据 `panel.enabled(action)` 拒绝。
- [x] 现有面板不传 `disabled`，渲染与命中不变。
- [x] 新增测试：disabled 行灰色、无选择标记、点击不生效（业务层测试）。

---

## Task 7：文档与规范

- [x] 实现完成后更新 `docs/standards/tui-design-requirements.md`：
  - 新增折叠粘贴芯片语法与样式；
  - 展开区只读、缩进 2 列、共用底色；
  - 单击折叠 / 拖拽选择的鼠标语义。
- [x] 更新本目录 `design.md` 的“状态”为已完成，并记录实际偏差（如有）。
- [x] 在 `task.md` 勾选任务并记录验证命令结果。

---

## Task 8：验证

按影响范围执行：

```bash
cargo fmt
cargo clippy --all-targets -- -D warnings
cargo test
```

重点回归：

- [x] 现有路径粘贴面板：`src/event/tests/focus_and_paste.rs`、`src/app/update/prompt/tests.rs`、`src/app/state/lx/tests.rs`。
- [x] 输入框光标/选区：`src/ui/main_content/lx/input/tests.rs`、`src/ui/prompt/tests.rs`、`src/app/update/tests.rs`。
- [x] 发送路径：`src/app/update/chat/tests.rs`、`src/app/update/chat/tests/prompt_history.rs`、`src/app/update/chat/tests/user_shell.rs`。
- [x] 提示词页保存/重载：`src/app/update/prompt/tests.rs`、`src/db/service/tests.rs`。
- [x] `cargo clippy` 无新增 `allow`；若必须 `#[allow]`，附注释说明原因。
- [x] 单文件行数：新增生产文件 ≤ 1000 行；测试文件 ≤ 1500 行并按场景拆分。

---

## 9. 实施顺序建议

1. Task 1（folded 核心 + 测试）——先把格式、投影、原子边界锁死。
2. Task 2（text 面板状态机 + 测试）——不依赖 UI。
3. Task 5/6（样式、文案、面板置灰）——UI 基础设施。
4. Task 3（Prompt 集成）——先做右栏，验证投影/渲染/编辑。
5. Task 4（Lx 集成）——复用同一套核心，处理发送路径。
6. Task 7/8（文档、全量验证）。

每完成一个 Task 执行对应子集测试；不要等全部写完再编译。

---

## 10. 风险与回滚

| 风险 | 缓解 |
| :--- | :--- |
| 投影改动面大，影响现有光标/选区 | 先写 `Projection` 单测；每个编辑器先接渲染再接鼠标；保留无 token 时 `Cow::Borrowed` 路径，确保原行为不变 |
| token 正文污染 Markdown/块扫描 | 全链路使用 `mask()`；增加含 ``` / `&&&` / `+++` 的回归测试 |
| 展开区选区与普通选区冲突 | 按“按下点落在哪”二选一，不做混合选区；文档明确限制 |
| 发送路径漏解码 | 统一 `expanded_input()`，为 `try_send` / queue / title / history 各加测试 |
| 超长粘贴卡顿 | 256 KiB 上限；超限禁用折叠；无 token 时零分配 |
| 现有路径粘贴回归 | 路径检测保持在长文本之前；不改 `PastePanel` 现有 API |

回滚方式：功能集中在新增模块与两个编辑器的字段/分支，未改数据库；如需回滚，移除长文本面板分支与 `folded` 模块即可，token 作为普通文本仍可读写。
