# Prompt 单模板块聚焦模式 — 设计

> 状态：**已实施并合并**（2026-10-26 实施，2026-10-27 合入 `dev`，见 §13）。
>
> 任务目录：`docs/feature/prompt-block-focus/`
>
> 影响范围：`src/app/prompt.rs`、`src/app/prompt/{viewport,navigation,panel,template}.rs`、
> `src/app/actions.rs`、`src/app/update/{editor,prompt}.rs`、`src/app/tooltip.rs`、
> `src/input/mod.rs`、`src/layout.rs`、`src/ui/prompt.rs`、`src/ui/prompt/header.rs`、
> `src/ui/prompt_frame.rs`、`src/ui/text.rs`、`src/ui/tooltip.rs`、`src/event/mouse/press.rs`、
> `src/event/input.rs`、`docs/standards/tui-design-requirements.md` 及各自的测试文件。
>
> **不改**数据库、不改配置、不改终端 PTY、不改 prompt 正文语法与持久化格式。

---

## 1. 背景与目标

Prompt 右栏编辑视图当前把整页 Markdown 正文连续渲染，`&&&` 模板块散落在长文档里。
一页提示词经常包含多个模板块，编辑其中一个块时需要在整页里滚动定位、且容易误改相邻块。

本次新增一个**视图模式**：进入后编辑视图只渲染当前聚焦的那一个顶层 `&&&` 模板块，
块本身（起止行、`{title: }` 标题行、正文、`+++` 补充子块、边框底色、状态与操作按钮）
全部保留并**仍可编辑**；用 `Ctrl/Cmd+←/→` 在块之间切换。

同时按需求调整工具栏：

1. 新增模式按钮，位于保存状态点左侧；
2. `pin` 按钮从 `[pin]`/`[unpin]` 恢复为单字图标 `[○]`/`[◉]`；
3. `[all]` 从右组移到左组。

---

## 2. 已确认的产品决定

| # | 项目 | 决定 |
| --- | --- | --- |
| 1 | 显示内容 | 完整 `&&&` 模板块（含起止行、标题行、`+++` 子块、边框/底色/按钮），**可编辑** |
| 2 | 进入时的聚焦块 | 光标所在块优先；否则本页第一个顶层 `&&&`（含未闭合块）；本页无块则按钮**置灰**、不进入 |
| 3 | 切换后的光标 | 落到目标块 `{title: }` 值内（无标题行落正文首行、正文为空落起始行）；进入时若光标本就在该块则**保持原地**，只调整滚动 |
| 4 | 切换范围 | **当前页**的**顶层 `&&&`**（`+++` 子块不单独停靠、代码围栏内不算、未闭合块算一块），按文档顺序；两端**钳制不循环** |
| 5 | 键位归属 | 模式内 `Ctrl/Cmd+←/→` 切块；模式外 `Ctrl+←/→` 仍按词移动、`Cmd+←/→` 仍吞掉；`Alt+B/F`、`Alt+←/→` **永远**按词移动 |
| 6 | 工具栏顺序 | 左组 `[≡] [←] [→] [all]`；右组 `[dir] [○]/[◉] [⊞]/[⊡] ●` |
| 7 | 模式按钮字形 | 未进入 `[⊞]`、已进入 `[⊡]`；tooltip `focus template block` / `show all blocks` |
| 8 | 进出方式 | 点击按钮 toggle；`Esc` **仍直接回树视图**（不做分层退出）；块模式随 `EditorStash` 暂存，返回同一目标时恢复 |
| 9 | 记忆粒度 | 按**工作区**记忆（随 `Prompt` 草稿 swap）；工作区内切条目/临时提示词/切页即清除；切回同一工作区恢复 |
| 10 | 记忆识别 | 只记**块起始逻辑行号**；切回时若该行已不是顶层 `&&&` 起始行则按「光标块 → 首块」重定位；无块则退出 |
| 11 | 删除当前块 | 自动聚焦**下一个** → 没有再**上一个** → 都没有则退出模式 |
| 12 | 编辑越界 | 不拦截编辑命令；每次编辑/撤销后按光标重定位：仍在当前块→保持；落到另一块→切过去；落在块外→退出模式 |
| 13 | `[all]` 与复制 | `[all]` 只选当前块；无选区 `Ctrl+C` 仍复制**全文**；`Ctrl/Cmd+Shift+C` 语义不变 |
| 14 | 窄屏隐藏顺序 | `●` 恒留 → `[all]` 先隐 → `[dir]` → pin → `[⊡]`；`[⊡]` 被隐藏时**自动退出模式** |
| 15 | 底部虚拟空白 | **保留**现有规则（公式不变） |
| 16 | 切 Todo 标签 | 块模式**保留**（与正文/光标/滚动/撤销栈同层） |
| 17 | 实现机制 | `Prompt` 持 `focus_block`，**`visual_rows()` 统一过滤**；不改文本、不在渲染层裁剪 |
| 18 | 状态存储 | 只存块起始逻辑行号，块范围每次用 `block_scan` 现算 |
| 19 | 显式退出点 | `load_editor_target`（切条目/临时提示词）、`reload_editor_from_active_page`（切页）、`open_temp_prompt`（新建草稿）与 `restore_prompt_view` 兜底空草稿；`show_prompt_tree` **不再清空**，`set_text` 内部**不清** |
| 20 | 输入层翻译 | 新命令 `BlockPrev { cmd }` / `BlockNext { cmd }`，`cmd` 标记是否来自 Super |
| 21 | 状态归属 | `Prompt`（并纳入 `PromptHistory` 与撤销 `Snapshot`） |
| 22 | 鼠标 | 零新增分支：点击/拖选/滚轮/滚动条全部靠过滤后的 `visual_rows()` 自动收敛 |
| 23 | 左组排布 | 固定位置、各自独立宽度门控，不做动态挤压 |
| 24 | 额外提示 | 不加（仅按钮字形 + tooltip 变化） |
| 26 | 树视图往返 | 往返**恢复**块模式（同一目标、正文未变时由 `EditorStash` 带回）；打开别的目标则不恢复；撤销删除块时**恢复**该块聚焦 |

---

## 3. 核心机制：`visual_rows()` 统一过滤

### 3.1 为什么只能改这一处

`Prompt::visual_rows()`（`src/app/prompt/viewport.rs:13`）是全项目唯一的「文本 → 可见视觉行」投影：

| 依赖方 | 位置 | 依赖内容 |
| --- | --- | --- |
| 光标单元格 | `viewport.rs:69` `cursor_cell` | `cursor_visual_in(&rows)` 后减 `scroll` |
| 点击定位 | `viewport.rs:123` `set_cursor_from_cell` → `viewport_offset` | 行索引 → 字节偏移 |
| 选区映射 | `viewport.rs:129/143` `selection_bounds` / `selection_text` | 行区间 → 字节区间 |
| 滚动上限 | `viewport.rs:84/108/118` `scroll_by` / `text_max_scroll` / `max_scroll` | `rows.len()` |
| 光标可见 | `viewport.rs:248` `scroll_cursor_into_view` | `cursor_visual_in` |
| 上下/行首尾移动 | `navigation.rs:26/40/54/63` | `cursor_visual_in` |
| 渲染 | `ui/prompt.rs:181` `render` | 逐 `VisualRow` 绘制 |
| 块按钮命中 | `ui/prompt.rs:397/453` `template_buttons` / `block_buttons` | `rows` + `scroll` |
| 滚动条 | `ui/prompt_frame.rs:243` `prompt_scrollbar_for` | `rows.len()` + `bottom_padding()` |
| 全选 | `app/state.rs:504` `select_all_prompt` | `rows.len()` |

任何一处不共用这份投影都会出现「看到的行 ≠ 点到的行」。
因此**唯一改动点是 `visual_rows()` 的过滤**，其余路径零改动即自动对齐。

### 3.2 过滤实现

```rust
// src/app/prompt/viewport.rs
pub fn visual_rows(&self) -> Vec<VisualRow> {
    let width = usize::from(self.width.max(1));
    let margins = usize::from(
        crate::layout::PROMPT_TEMPLATE_GUTTER_LEFT
            + crate::layout::PROMPT_TEMPLATE_GUTTER_RIGHT,
    );
    let scan = markdown::block_scan(&self.text);
    let lines: Vec<&str> = self.text.split('\n').collect();
    // 新增：块聚焦时只保留该块的逻辑行区间（含起止行）。
    let focus = self.focused_block_range(&scan);
    let mut hidden = vec![false; lines.len()];
    for block in fold::folded_blocks_from(&scan, &self.text, &self.expanded_supplements) {
        for line in block.hidden_from_line..=block.end_line {
            if let Some(slot) = hidden.get_mut(line) {
                *slot = true;
            }
        }
    }
    let mut rows = Vec::new();
    let mut line_start = 0;
    for (line, segment) in lines.iter().enumerate() {
        let line_end = line_start + segment.len();
        // 新增：聚焦块外（或聚焦范围失效）的行整体跳过。
        let outside_focus = focus.is_some_and(|(start, end)| line < start || line > end);
        if hidden.get(line).copied().unwrap_or(false) || outside_focus {
            line_start = line_end + 1;
            continue;
        }
        // ……以下与现状完全一致（块行扣 gutter、软换行、push VisualRow）
    }
    rows
}
```

关键点：

- **判定用逻辑行区间**，不是视觉行，所以软换行不受影响；
- `focus` 为 `None` 时行为与现状**逐字节一致**（普通视图零风险）；
- `visual_rows()` 可能返回**空 vec**（块刚被删、文本被外部清空）；现有调用方已能处理空：
  `cursor_visual_in` 用 `rows.len().saturating_sub(1)`、`cursor_cell` 有 `height == 0` 早退、
  `content_offset_at` 用 `min(rows.len() - 1)`。**实施时必须复核这四处不会越界**（见 §8 风险）。

### 3.3 块范围派生（新代码，放 `src/app/prompt/block_focus.rs`）

```rust
// src/app/prompt/block_focus.rs（新增文件，约 150 行）
//! `&&&` 模板块聚焦：范围派生、切换、进入/退出与光标落点。

use super::super::markdown::{self, BlockKind, BlockScan};
use super::Prompt;

/// 块聚焦范围（逻辑行，端点含）。
pub(crate) type FocusRange = (usize, usize);

impl Prompt {
    /// 当前聚焦的顶层 `&&&` 块行区间；未进入模式或聚焦行已失效时为 None。
    pub(crate) fn focused_block_range(&self, scan: &BlockScan) -> Option<FocusRange> {
        let line = self.focus_block?;
        scan.spans
            .iter()
            .find(|span| span.kind == BlockKind::Template && span.start == line)
            .map(|span| (span.start, span.end))
    }

    /// 本页全部顶层 `&&&` 起始行（文档顺序，含未闭合块）。
    fn template_starts(scan: &BlockScan) -> Vec<usize> {
        scan.spans
            .iter()
            .filter(|span| span.kind == BlockKind::Template)
            .map(|span| span.start)
            .collect()
    }

    /// 是否处于块聚焦模式（聚焦行仍有效）。
    pub fn block_focus_active(&self) -> bool {
        self.focused_block_range(&markdown::block_scan(&self.text)).is_some()
    }

    /// 进入块聚焦：光标所在块优先，否则第一个块；无块返回 false。
    pub fn enter_block_focus(&mut self) -> bool { /* 见 §4.1 */ }

    /// 退出块聚焦；返回是否原本处于模式中。
    pub fn exit_block_focus(&mut self) -> bool { /* 只清字段 */ }

    /// 切到相邻块：-1 上一个、+1 下一个；两端钳制；返回是否移动。
    pub fn focus_adjacent_block(&mut self, delta: isize) -> bool { /* 见 §4.2 */ }

    /// 删除块后的接续：下一个 → 上一个 → 退出。
    pub(super) fn focus_after_block_removed(&mut self, removed_start: usize) { /* §4.3 */ }

    /// 编辑/撤销后按光标重定位；出块即退出。
    pub(super) fn reconcile_block_focus(&mut self) { /* §4.4 */ }
}
```

---

## 4. 行为细节

### 4.1 进入（`enter_block_focus`）

```text
scan = block_scan(text)
starts = 全部顶层 &&& 起始行
若 starts 为空 → return false（按钮置灰，不会走到这里）
光标行号 = text[..cursor].matches('\n').count()
若存在 span.start <= 光标行 <= span.end → focus_block = span.start（光标不动）
否则 → focus_block = starts[0]，光标落到该块标题值（见下）
scroll 对齐块首：scroll_cursor_into_view() 之后把 scroll 钳到不超过块的视觉首行
```

**标题值落点**复用现有逻辑：`markdown::parse_title_line` 命中起始行下一行时，
光标落在 `}` 之前（等价 `title_cursor()` 的口径，`app/markdown/slash.rs:334`）；
否则落到正文首行行首；正文为空落到起始行行首。

### 4.2 切换（`focus_adjacent_block`）

```text
starts = 全部顶层 &&& 起始行（现算）
current = 当前 focus_block 在 starts 中的位置（不在则按光标重定位）
target = clamp(current + delta, 0, starts.len()-1)
若 target == current → return false（两端钳制，不循环）
focus_block = starts[target]
光标落到目标块标题值（§4.1 同款）——注意：切换时**总是**改光标，
      与「进入时保留光标」不同（问题 3 的区分点）
scroll_cursor_into_view()
return true
```

### 4.3 删除块后的接续

`delete_template_block(start_line)`（`app/prompt/template.rs:61`）成功后在
`app/update/editor.rs:501` 的包装里调用 `focus_after_block_removed(start_line)`：

```text
若原本非块模式 → no-op
scan 重新扫描；对每个 span 记录其起始行相对删除位置的先后
下一个块 = 第一个 start > removed_start 的块 → focus_block = 它的 start
否则上一个块 = 最后一个 start < removed_start 的块 → focus_block = 它的 start
两者都无 → focus_block = None（退出模式）
新块内光标落在标题值
```

清理块（`clean_template_block`）与状态切换（`toggle_template_status`）**不改变块集合**，
仅需 `reconcile_block_focus()` 兜底。

### 4.4 编辑后重定位（`reconcile_block_focus`）

在 `Prompt::settle()`（`app/prompt.rs:837`）末尾调用：

```text
若 focus_block 为 None → 直接返回（普通视图零开销）
scan = block_scan(text)
若 focus_block 仍是顶层 &&& 起始行 → 只需检查光标：
    光标在当前块范围内 → 保持
    光标落在另一个 &&& 块内 → focus_block = 那个块的 start
    光标落在块外 → focus_block = None（出块即退出）
否则（聚焦行已失效，如被改动/被删）→
    光标在某个 &&& 块内 → 切到那个块
    否则 → focus_block = None
```

> 注意：`settle()` 内已经跑 `mention.refresh` / `refresh_slash_panel` 等，扫描成本与
> `visual_rows()` 同阶（O(行数)），仅在模式内额外跑一次；普通视图提前返回。

### 4.5 撤销/重做

`Snapshot`（`app/prompt.rs:104`）增加 `focus_block: Option<usize>`；
`snapshot()` 写入、`restore()` 读回。语义：撤销一次编辑即回到那次编辑前的块聚焦状态，
因此「删块 → 自动切到下一块 → `Ctrl+Z` → 原块回来并重新聚焦」自然成立。

### 4.6 工作区记忆

`Prompt` 随 `PromptDraft` 在 `swap_display_prompt`（`app/update/editor.rs:289`）
里整体 `std::mem::swap`，块聚焦字段自动跟着工作区走，**无需额外映射表**。

工作区切换的显式清除点（问题 19）实际落在四处：

| 位置 | 时机 | 处理 |
| --- | --- | --- |
| `load_editor_target`（`app/update/prompt.rs:484`） | 切条目 / 切临时提示词 | `set_text` / `restore_history` 前 `exit_block_focus()` |
| `reload_editor_from_active_page`（`app/update/prompt/pages.rs`） | 切页 | `set_text` 前 `exit_block_focus()` |
| `open_temp_prompt`（`app/update/prompt.rs`） | 列表新建临时提示词 | `set_text("")` 前 `exit_block_focus()` |
| `restore_prompt_view`（`app/update/editor.rs`） | 兜底空草稿 | `set_text("")` 前 `exit_block_focus()` |

`show_prompt_tree` **不**在暂存前清空：块聚焦行随 `PromptHistory` 进入 `EditorStash`，
返回同一目标（目标、页与正文均未变）时由 `restore_history` 带回；打开别的目标时
`load_editor_target` 先清空当前缓冲，再按新目标 `set_text`，不会继承旧聚焦。
`set_text` / `restore_history` 本身仍不碰该字段（问题 19）。

### 4.7 工作区切回后的重定位

`restore_prompt_view` → `select_prompt_item` → `load_editor_target` → `restore_history`
路径中块聚焦随历史回来。`restore_history` 之后立即 `reconcile_block_focus()`：
若记录的起始行已不是 `&&&` 起始行，按「光标块 → 首块 → 退出」处理（问题 10）。

---

## 5. 键位与输入层

### 5.1 新命令

```rust
// src/app/actions.rs
pub enum EditorCommand {
    // ……现有变体
    /// Ctrl+←（`cmd == false`）/ Cmd+←（`cmd == true`）：块模式内切上一块、
    /// 普通视图 Ctrl 为按词左移、Cmd 吞掉。
    BlockPrev { cmd: bool },
    /// Ctrl+→ / Cmd+→：同 `BlockPrev`，方向相反。
    BlockNext { cmd: bool },
}
```

### 5.2 输入层映射（`src/input/mod.rs:434` `editor_command`）

```rust
// Super（Cmd）分支：仅新增两个方向键，其余保持现状（问题 5）
if modifiers.contains(KeyModifiers::SUPER) {
    return match key.code {
        KeyCode::Enter => Some(EditorCommand::NewlineBelow),
        KeyCode::Left => Some(EditorCommand::BlockPrev { cmd: true }),
        KeyCode::Right => Some(EditorCommand::BlockNext { cmd: true }),
        KeyCode::Char('z' | 'Z') if shift => Some(EditorCommand::Redo),
        KeyCode::Char('z' | 'Z') => Some(EditorCommand::Undo),
        KeyCode::Char('y' | 'Y') => Some(EditorCommand::Redo),
        _ => None,
    };
}
// Ctrl 分支：Ctrl+←/→ 从 WordLeft/WordRight 改为 BlockPrev/BlockNext
(true, false, KeyCode::Left) => Some(EditorCommand::BlockPrev { cmd: false }),
(true, false, KeyCode::Right) => Some(EditorCommand::BlockNext { cmd: false }),
// Alt 分支保持 WordLeft/WordRight 不变
```

### 5.3 update 层分派（`app/update/editor.rs:33` `apply_editor`）

**必须在 `route_panel` 之前还是之后？** 放在 `route_panel` **之后**、选区处理**之前**：

```rust
BlockPrev { cmd } => {
    if !state.prompt.focus_adjacent_block(-1) {
        // 块模式内已到首块（返回 false 且仍处于模式）→ 吞掉，不按词移动
        if !state.prompt.block_focus_active() && !cmd {
            state.prompt.move_word_backward();
        }
    }
}
```

> 语义表（与问题 5 一致）：
>
> | 视图 | Ctrl+← | Cmd+← |
> | --- | --- | --- |
> | 块模式 | 切上一块（首块处吞掉） | 切上一块（首块处吞掉） |
> | 普通视图 | 按词左移 | 吞掉 |
>
> `Alt+B/F`、`Alt+←/→` 仍映射 `WordLeft/WordRight`，在任何视图都按词移动。

实现上更清晰的做法是给 `focus_adjacent_block` 返回值区分「已移动」与「无相邻块」，
并在 update 层先判 `block_focus_active()`：

```rust
EditorCommand::BlockPrev { cmd } => {
    if state.prompt.block_focus_active() {
        state.prompt.focus_adjacent_block(-1); // 两端钳制，内部 no-op
    } else if !cmd {
        state.prompt.move_word_backward();
    }
}
```

### 5.4 选区与编辑命令的先后

`apply_editor` 现有顺序：树视图 → 面板 → Esc → 选区替换/清除 → 编辑命令。
`BlockPrev/BlockNext` 属于「光标移动」，必须走**选区先清除**这条既有路径
（`apply_editor:49-68` 对非写入类命令执行 `clear_selection`），行为与 `WordLeft` 一致。

---

## 6. 工具栏与布局

### 6.1 新枚举与常量（`src/layout.rs`）

```rust
pub enum PromptToolbarButton {
    Mode,
    Pin,
    TempPrompt,
    Folders,
    /// 编辑视图：切换「单模板块聚焦」模式（`[⊞]` / `[⊡]`）。
    BlockFocus,
    Undo,
    Redo,
    SelectAll,
    Save,
}
```

常量调整（**只在编辑视图生效的新按钮**，树视图不显示）：

```rust
/// 块聚焦按钮宽度（列）；与 `[⊞]` 同宽 3。
const PROMPT_BLOCK_FOCUS_WIDTH: u16 = 3;
/// pin 恢复单字图标后的宽度（`[○]` = 3 列），替代原 `PROMPT_PIN_WIDTH = 7`。
const PROMPT_PIN_WIDTH: u16 = 3;
```

**阈值常量随之变化**（这些常量是 `prompt_toolbar_button_rect` 的门槛，必须同步）。
按新布局重新推导（左组 `[≡][←][→][all]` 总宽 = `3+1+3+1+3+1+5 = 17`，右组自右向左
`● ← [⊡] ← [○] ← [dir]`）：

| 常量 | 现值 | 新值 | 推导 |
| --- | --- | --- | --- |
| `PROMPT_SELECT_ALL_MIN_WIDTH` | 19 | **19**（不变） | 左组 17 + 1 列间隔 + 1 列余量 |
| `PROMPT_PIN_WIDTH` | 7 | **3** | 单字图标 `[○]`/`[◉]` |
| 新增 `PROMPT_BLOCK_FOCUS_MIN_WIDTH_EDITOR` | — | **23** | 左组 17 + 1 + `[⊡]`3 + 1 + `●`1 |
| `PROMPT_PIN_MIN_WIDTH_EDITOR` | 27 | **27**（不变） | 上值 + 1 + `[○]`3（pin 缩 4 列、新按钮占 4 列，净零） |
| `PROMPT_PIN_MIN_WIDTH_TREE` | 18（旧注释写 12 已过期） | **14** | 左组 3 + 1 + pin 3 + 1 + `[tmp]`5 + 1 |
| `PROMPT_FOLDERS_MIN_WIDTH_EDITOR` | 33 | **33**（不变） | `PIN_MIN_EDITOR(27)` + 1 + `[dir]`5 |

> 注意：`[⊡]` 的阈值（23）必须**低于** `[○]`（27）与 `[dir]`（33），这样窄屏时
> `[⊡]` 最后消失（问题 14 的隐藏顺序）。右组自右向左绘制天然满足：越靠右的按钮越晚隐藏。

### 6.2 几何（`prompt_toolbar_button_rect`）

```rust
let bar = prompt_toolbar_rect(panel)?;
let inner = pane_inner_rect(panel);
let undo_x = bar.x + PROMPT_TOOLBAR_BUTTON_WIDTH + PROMPT_TOOLBAR_BUTTON_GAP;
let redo_x = undo_x + PROMPT_UNDO_REDO_WIDTH + PROMPT_TOOLBAR_BUTTON_GAP;
// 新增：`[all]` 紧贴 `[→]` 右侧
let select_all_x = redo_x + PROMPT_UNDO_REDO_WIDTH + PROMPT_TOOLBAR_BUTTON_GAP;
let rect = match button {
    PromptToolbarButton::Mode => Rect::new(bar.x, bar.y, PROMPT_TOOLBAR_BUTTON_WIDTH, 1),
    PromptToolbarButton::Undo => /* 不变 */,
    PromptToolbarButton::Redo => /* 不变 */,
    PromptToolbarButton::SelectAll => {
        if tree || inner.width < PROMPT_SELECT_ALL_MIN_WIDTH { return None; }
        Rect::new(select_all_x, bar.y, PROMPT_SELECT_ALL_WIDTH, 1)
    }
    PromptToolbarButton::Save => Rect::new(bar.right() - 1, bar.y, 1, 1),
    // 右组自右向左：Save ← BlockFocus ← Pin ← Folders
    PromptToolbarButton::BlockFocus => {
        if tree || inner.width < PROMPT_BLOCK_FOCUS_MIN_WIDTH_EDITOR { return None; }
        Rect::new(bar.right() - 1 - PROMPT_TOOLBAR_BUTTON_GAP - PROMPT_BLOCK_FOCUS_WIDTH,
                  bar.y, PROMPT_BLOCK_FOCUS_WIDTH, 1)
    }
    PromptToolbarButton::Pin => {
        let (anchor, min_width) = if tree {
            (PromptToolbarButton::TempPrompt, PROMPT_PIN_MIN_WIDTH_TREE)
        } else {
            (PromptToolbarButton::BlockFocus, PROMPT_PIN_MIN_WIDTH_EDITOR)
        };
        if inner.width < min_width { return None; }
        let anchor = prompt_toolbar_button_rect(panel, anchor, tree)?;
        Rect::new(anchor.x.saturating_sub(PROMPT_PIN_WIDTH + PROMPT_TOOLBAR_BUTTON_GAP),
                  bar.y, PROMPT_PIN_WIDTH, 1)
    }
    PromptToolbarButton::Folders => { /* 锚点改为 Pin，阈值改为新值 */ }
    PromptToolbarButton::TempPrompt => { /* 不变（树视图） */ }
};
(rect.right() <= bar.right()).then_some(rect)
```

**布局示意**（内容区 40 列）：

```text
[≡] [←] [→] [all]              [dir] [○] [⊡] ●
 ↑   ↑   ↑   ↑                   ↑    ↑    ↑  ↑
bar.x +0  +4  +8  +12           ...  -14  -10 -1
```

### 6.3 窄屏降级链（问题 14）

阈值：`[all]` 19、`[⊡]` 23、`[○]` 27、`[dir]` 33（内宽 = `panel.width - 2`）。

| 内容区内宽 | 可见按钮 |
| --- | --- |
| ≥ 33 | `[≡] [←] [→] [all]` · `[dir] [○] [⊡] ●` |
| 27–32 | `[≡] [←] [→] [all]` · `[○] [⊡] ●`（`[dir]` 隐藏） |
| 23–26 | `[≡] [←] [→] [all]` · `[⊡] ●`（`[○]` 再隐藏） |
| 19–22 | `[≡] [←] [→] [all]` · `●`（`[⊡]` 隐藏 → **自动退出块模式**） |
| < 19 | `[≡] [←] [→]` · `●`（`[all]` 也隐藏） |

> 单调性由阈值本身保证：23 < 27 < 33，因此实际隐藏顺序恒为 `[dir]` → `[○]` → `[⊡]`
> → `[all]`，与问题 14 的约定一致。实施时以 `layout/tests.rs` 的断言为准（§9.4）。

若当前处于块模式而 `prompt_toolbar_button_rect(.., BlockFocus, false)` 返回 `None`，
**自动退出模式**。实现位置：`resize_panes`（`app/update/pane.rs:886`）在尺寸变化后：

```rust
if *id == state.prompt.id() {
    let (cols, rows) = layout::prompt_inner_size(*rect);
    // 新增：模式按钮放不下时自动退出块聚焦（避免窄到无法用按钮退出）
    if state.prompt.block_focus_active()
        && layout::prompt_toolbar_button_rect(*rect, layout::PromptToolbarButton::BlockFocus, false).is_none()
    {
        state.prompt.exit_block_focus();
    }
    // ……原有 resize
}
```

> `resize_panes` 是否算「渲染层」？不算——它是 update 层的几何同步，允许改状态。

---

## 7. UI 渲染

### 7.1 文案（`src/ui/text.rs`）

```rust
/// 块聚焦模式按钮：未进入 `[⊞]`（全部块）、已进入 `[⊡]`（聚焦单块）。
pub const PROMPT_BLOCK_FOCUS_ALL_LABEL: &str = "[⊞]";
pub const PROMPT_BLOCK_FOCUS_ONE_LABEL: &str = "[⊡]";

/// prompt 钉住按钮恢复单字图标：未钉 `[○]`、已钉 `[◉]`。
pub const PROMPT_UNPINNED_LABEL: &str = "[○]";
pub const PROMPT_PINNED_LABEL: &str = "[◉]";
```

**字形校验**（`ui/text/tests.rs:232` 已有规则）：`⊞` 与 `⊡` 均为 East Asian Width `N`、
1 列宽、非 PUA，符合现有 `prompt_button_icons_are_single_width_and_non_pua`。
`○`（U+25CB）EAW 为 `A`（Ambiguous），但项目已在用 `PROMPT_DRAFT_DOT = "○"`，
且终端普遍按 1 列渲染；测试里 `○` 已通过 `width() == 1` 断言（`text/tests.rs:288-289`），
`[○]` 同理。**实施时若某终端渲染为 2 列会撑破 3 列槽位——这是恢复单字图标的已知风险**，
见 §8。

### 7.2 工具栏绘制（`src/ui/prompt.rs:64` `render_toolbar`）

`render_toolbar` 签名增加一个参数：

```rust
pub fn render_header(
    panel: Rect, buf: &mut Buffer, prompt: &Prompt,
    focused: bool, mode: PromptMode, draft: bool, pinned: bool, block_focus: bool,
) { ... }
```

`render_toolbar` 内：

```rust
let mut buttons = vec![
    (PromptToolbarButton::Mode, mode_label, style::accent()),
    (PromptToolbarButton::Undo, ..., can_undo 决定 color),
    (PromptToolbarButton::Redo, ..., can_redo 决定 color),
    (PromptToolbarButton::SelectAll, text::PROMPT_SELECT_ALL_LABEL, style::accent()),
];
if tree {
    // 树视图：左侧只有 [≡]，右组 [tmp] 与 pin
    buttons = vec![
        (PromptToolbarButton::Mode, mode_label, style::accent()),
        (PromptToolbarButton::Pin, pin_label, style::accent()),
        (PromptToolbarButton::TempPrompt, text::PROMPT_TEMP_PROMPT_LABEL, style::accent()),
    ];
} else {
    buttons.extend([
        (PromptToolbarButton::Folders, text::PROMPT_FOLDERS_LABEL, style::accent()),
        (PromptToolbarButton::Pin, pin_label, style::accent()),
        (PromptToolbarButton::BlockFocus,
         if block_focus { text::PROMPT_BLOCK_FOCUS_ONE_LABEL }
         else { text::PROMPT_BLOCK_FOCUS_ALL_LABEL },
         style::accent()),
        (PromptToolbarButton::Save, save_label, save_style),
    ]);
}
```

> **注意顺序**：绘制顺序与几何无关（每个按钮按 `rect` 独立落位），
> 但为避免「右对齐 + 左补空格」那套逻辑失效，`[pin]` 恢复 3 列后
> **不再需要右对齐补空格**（`[○]` 与 `[◉]` 同宽 3 列）：删除 `render_toolbar:144-169`
> 里的 `start = rect.right() - label.chars().count()` 偏移，改回**左对齐**。

### 7.3 渲染层是否需要知道模式？

**不需要**。`ui/prompt.rs:177` 的 `render` 直接遍历 `prompt.visual_rows()`，
过滤已在状态层完成；块边框、底色、按钮全部按 `block_line_infos` 派生，天然只覆盖当前块。

唯一例外：`prompt.visual_rows()` 为空时 `render` 的 `scroll` 计算
（`ui/prompt.rs:196`）会钳到 `rows.len().saturating_sub(1)` → 0，安全。

### 7.4 Tooltip（`src/app/tooltip.rs:47`、`src/ui/tooltip.rs:166`、`src/ui/text.rs:932`）

```rust
// app/tooltip.rs
pub enum TooltipTarget {
    // ……
    PromptBlockFocus,
}

// ui/tooltip.rs：按钮组追加
(layout::PromptToolbarButton::BlockFocus, TooltipTarget::PromptBlockFocus),

// ui/text.rs
TooltipTarget::PromptBlockFocus if state.prompt.block_focus_active() => "show all blocks",
TooltipTarget::PromptBlockFocus => "focus template block",
```

---

## 8. 鼠标与事件层

### 8.1 点击工具栏（`src/event/mouse/press.rs:336-371`）

```rust
layout::PromptToolbarButton::BlockFocus => {
    update::toggle_prompt_block_focus(state);
}
```

新增 update 函数（`app/update/prompt.rs`）：

```rust
/// 工具栏块聚焦按钮：未进入则尝试进入（无模板块时 no-op），已进入则退出。
pub fn toggle_prompt_block_focus(state: &mut AppState) -> bool {
    if state.prompt.block_focus_active() {
        state.prompt.exit_block_focus();
        true
    } else {
        state.prompt.enter_block_focus()
    }
}
```

按钮**置灰**判定（问题 2）：`state.prompt` 里没有顶层 `&&&` 时按钮用 `style::muted()`。
`render_toolbar` 需要读一个 `block_focus_available: bool` 参数（由 `prompt_frame` 传入，
用 `block_scan(text).spans.iter().any(|s| s.kind == BlockKind::Template)` 计算）。
点击时若不可用 → `enter_block_focus()` 返回 false、不置脏。

### 8.2 点击内容区 / 拖选 / 滚轮 / 滚动条

**零新增分支**：全部走 `prompt.visual_rows()`，而它已过滤。

唯一需要复核的是 `row_in_padding`（`app/prompt/viewport.rs:104`）：

```rust
pub fn row_in_padding(&self, viewport_row: u16) -> bool {
    self.scroll.saturating_add(usize::from(viewport_row)) >= self.visual_rows().len()
}
```

块模式下 `visual_rows().len()` 是**块的行数**，因此块底部以下是 padding →
点击只聚焦、不移动光标，与现有语义一致。

### 8.3 块按钮

`template_buttons` / `block_buttons`（`ui/prompt.rs:397/453`）遍历 `visual_rows()`，
块模式下只会产出当前块的起始行命中——自动正确。删除按钮走 §4.3 的接续。

---

## 9. 测试计划

### 9.1 app 层（`src/app/prompt/tests.rs`，必要时新增 `src/app/prompt/block_focus/tests.rs`）

| 用例 | 断言 |
| --- | --- |
| `enter_focus_prefers_cursor_block` | 光标在块 2 → `focus_block == 块 2 起始行`，光标不动 |
| `enter_focus_falls_back_to_first_block` | 光标在普通段落 → 聚焦块 1，光标落在标题值 |
| `enter_focus_returns_false_without_blocks` | 纯文本 → false，`focus_block` 仍 None |
| `adjacent_block_clamps_at_ends` | 块 1 处 `-1` 返回 false；块 3 处 `+1` 返回 false |
| `adjacent_block_skips_supple_and_fence` | 含 `+++` 与围栏代码块 → 只停顶层 `&&&` |
| `adjacent_block_includes_unclosed` | 末尾未闭合 `&&&` 也参与切换 |
| `visual_rows_only_contains_focus_range` | 过滤后行 `line` 全在 `[start, end]` |
| `cursor_cell_and_click_agree_under_focus` | `cursor_cell()` 与实际点击映射互为逆 |
| `selection_bounds_clamped_to_focus_block` | 拖选只覆盖当前块字节区间 |
| `delete_current_block_focuses_next` / `..._previous` / `..._exits` | 三种接续 |
| `undo_restores_block_focus` | 删块后撤销 → 原块重新聚焦 |
| `edit_inside_block_keeps_focus` / `edit_outside_exits` | 光标驱动重定位 |
| `focus_survives_todo_tab_roundtrip` | 切 Tab 再切回，模式保留 |
| `focus_survives_tree_roundtrip_for_same_target` | 进列表再返回同一目标 → 块模式与光标保留 |
| `esc_tree_roundtrip_keeps_block_focus` | `Esc` 进列表再返回 → 与工具栏往返同语义 |
| `opening_other_target_clears_block_focus` | 打开别的目标消费暂存 → 原目标也不再恢复 |
| `new_temp_does_not_inherit_old_block_focus` | 新建草稿不继承旧聚焦行 |
| `focus_restored_per_workspace` | 切工作区再切回 → 恢复同一块 |
| `focus_relocates_when_recorded_line_invalid` | 记录的起始行变成普通行 → 按光标块/首块重定位 |
| `select_all_covers_only_focus_block` | `select_all_prompt()` 的选区文本 == 当前块文本 |

### 9.2 input 层（`src/input/tests.rs`）

| 用例 | 断言 |
| --- | --- |
| `ctrl_arrows_map_to_block_navigation` | `Ctrl+←/→` → `BlockPrev{cmd:false}` / `BlockNext{cmd:false}` |
| `cmd_arrows_map_to_block_navigation` | `Super+←/→` → `{cmd:true}` |
| `alt_word_motion_unaffected` | `Alt+B/F`、`Alt+←/→` 仍 `WordLeft/WordRight` |

### 9.3 update 层（`src/app/update/prompt/tests.rs`）

| 用例 | 断言 |
| --- | --- |
| `block_focus_nav_switches_blocks` | 块模式下 `BlockNext` 切到下一块 |
| `ctrl_arrow_moves_by_word_outside_focus` | 普通视图 `BlockPrev{cmd:false}` → 按词左移 |
| `cmd_arrow_is_noop_outside_focus` | 普通视图 `BlockPrev{cmd:true}` → 光标不动 |
| `toggle_block_focus_button_roundtrip` | 连点两次回到普通视图 |

### 9.4 layout 层（`src/layout/tests.rs`）

| 用例 | 断言 |
| --- | --- |
| `toolbar_select_all_sits_right_of_redo` | `[all].x == redo.right() + 1` |
| `toolbar_block_focus_sits_left_of_save` | `[⊞].right() + 1 == Save.x` |
| `toolbar_pin_is_three_wide_left_of_block_focus` | `pin.width == 3`、`pin.right() + 1 == block_focus.x` |
| `toolbar_folders_left_of_pin` | 既有断言按新阈值更新 |
| 各阈值的边界断言 | 新阈值 19 / 23 / 27 / 33 的 `±2` 边界 |

### 9.5 ui 层（`src/ui/prompt/tests.rs`、`src/ui/tests.rs`、`src/ui/tooltip/tests.rs`）

| 用例 | 断言 |
| --- | --- |
| 按钮字形 | `[⊞]` / `[⊡]` / `[○]` / `[◉]` 逐格渲染正确 |
| `[all]` 左组位置 | 渲染出的 `[all]` 紧邻 `[→]` |
| 块模式渲染 | `visual_rows()` 过滤后 `render` 只画当前块（块外文本不出现） |
| 按钮置灰 | 无模板块时 `[⊞]` 用 `muted()` |
| 窄屏自动退出 | 宽度压到阈值以下 → `block_focus_active() == false` |

### 9.6 文档

更新 `docs/standards/tui-design-requirements.md` 第 25 行（prompt 右栏）与第 65 行（prompt 编辑器）：

- 工具栏顺序改为 `[≡] [←] [→] [all]` + `[dir] [○/◉] [⊞/⊡] ●`；
- pin 由「纯英文单词、7 列占位、右对齐」改为「单字图标、3 列」；
- 新增块聚焦模式的行为段（进入/切换/退出/记忆/窄屏自动退出/`[all]` 范围）。

### 9.7 交互验收

由用户 `cargo run` 确认：按钮点击、`Ctrl/Cmd+←/→` 切块、块模式渲染、删除接续、
切工作区恢复、窄屏退化、各终端字形宽度。

---

## 10. 风险与边界

| 风险 | 说明 | 缓解 |
| --- | --- | --- |
| `visual_rows()` 返回空 vec | 块被删且 `focus_block` 尚未清理的那一帧 | 现有 4 处调用已有空保护；§9.1 增加 `empty_rows_do_not_panic` 用例 |
| `○` / `⊡` 字形宽度 | `○` EAW 为 Ambiguous，个别终端按 2 列 | 沿用项目已用的 `○`；`[○]`/`[◉]` 都是 3 列槽位，实测由用户确认 |
| `[all]` 移到左组后与 `[≡]` 抢空间 | 极窄面板左组可能挤掉 `[←] [→]` | 各自独立门控（问题 23），阈值固定，`layout` 测试锁边界 |
| 块聚焦与 `+++` 折叠叠加 | 折叠块被聚焦时正文隐藏 | 不特殊处理：聚焦范围只按逻辑行裁剪，折叠规则照旧叠加 |
| `settle()` 内多跑一次 `block_scan` | 每键一次 O(行数) | 仅模式内执行；普通视图 `focus_block.is_none()` 提前返回 |
| 块模式与 `Ctrl+Shift+C` | 复制光标所在块正文 | 语义不变，无冲突 |
| 工作区记忆与钉住 | 钉住时显示的是被钉工作区草稿 | 聚焦字段随该草稿，天然跟随 |

---

## 11. 文件与实施顺序

### 11.1 新增

- `src/app/prompt/block_focus.rs`（约 150 行）：范围派生、进入/退出/切换/接续/重定位。

### 11.2 修改（按实施顺序）

1. `src/app/prompt.rs`：`Prompt` 加 `focus_block`、`Snapshot`、`PromptHistory` 字段；
   `settle()` 末尾调用 `reconcile_block_focus()`；`mod block_focus;`。
2. `src/app/prompt/viewport.rs`：`visual_rows()` 加聚焦过滤。
3. `src/app/prompt/block_focus.rs`：新文件（上表逻辑）。
4. `src/app/prompt/template.rs`：`delete_template_block` 返回删除的起始行（或由调用方取）。
5. `src/app/actions.rs`：`BlockPrev { cmd }` / `BlockNext { cmd }`。
6. `src/input/mod.rs`：`editor_command` 的 Super / Ctrl 分支。
7. `src/app/update/editor.rs`：`apply_editor` 分派；`delete_template_block` 包装接续。
8. `src/app/update/prompt.rs`：`toggle_prompt_block_focus`；`load_editor_target` /
   `open_temp_prompt` 清除。
9. `src/app/update/prompt/pages.rs`：`reload_editor_from_active_page` 清除。
10. `src/app/update/editor.rs`：`restore_prompt_view` 兜底空草稿时清除。
11. `src/app/update/pane.rs`：`resize_panes` 自动退出。
12. `src/layout.rs`：新枚举变体、常量与阈值、`prompt_toolbar_button_rect` / `_at`。
13. `src/ui/text.rs`：新增/修改标签。
14. `src/ui/prompt.rs`：`render_toolbar` 按键位重排、pin 改左对齐、`block_focus` 参数。
15. `src/ui/prompt/header.rs`：`render_header` 透传 `block_focus` 与可用性。
16. `src/ui/prompt_frame.rs`：计算可用性并传入。
17. `src/app/tooltip.rs` / `src/ui/tooltip.rs` / `src/ui/text.rs`：tooltip 目标与文案。
18. `src/event/mouse/press.rs`：按钮分派。
19. 测试文件：`src/app/prompt/tests.rs`、`src/app/update/prompt/tests.rs`、
    `src/input/tests.rs`、`src/layout/tests.rs`、`src/ui/prompt/tests.rs`、
    `src/ui/tests.rs`、`src/ui/text/tests.rs`、`src/ui/tooltip/tests.rs`、`src/event/tests.rs`。
20. `docs/standards/tui-design-requirements.md`。

### 11.3 行数约束

- `src/layout.rs` 当前 **1000 行**（已达上限）。新增约 30 行会超限 →
  实施时**必须**先按职责拆分（建议把 prompt 工具栏几何 + prompt 区域几何拆到
  `src/layout/prompt.rs`，与 `layout.rs` 同层、`mod prompt;` + `pub use prompt::*;`），
  再在其中加新代码。**这是本任务的前置条件**，不要靠压缩排版蒙混。
- `src/ui/prompt.rs` 805 行、`src/app/prompt.rs` 864 行，均有空间；
  若 `app/prompt.rs` 因新字段与方法逼近 900 行，把块聚焦逻辑留在 `block_focus.rs` 即可。

---

## 12. 验收标准

1. 编辑视图工具栏为 `[≡] [←] [→] [all]` + `[dir] [○/◉] [⊞/⊡] ●`，pin 为单字、3 列、左对齐。
2. 点击 `[⊞]` 进入块模式：只显示一个完整 `&&&` 块，可编辑、可点块按钮、可拖选、可滚动。
3. `Ctrl+←/→` 与 `Cmd+←/→` 在块模式内切块，两端钳制不循环；块外 Ctrl 按词、Cmd 无反应；
   `Alt+B/F`、`Alt+←/→` 始终按词。
4. 退出路径：点 `[⊡]`、切条目/临时提示词/切页、窄屏按钮消失 → 均退出；`Esc` 回列表时
   随 `EditorStash` 暂存，返回同一目标恢复。
5. 记忆：切工作区再切回恢复同一块；树视图往返（未打开别的目标）恢复；撤销删除恢复。
6. 删除当前块自动接续下一个 → 上一个 → 退出。
7. `[all]` 只选当前块；无选区 `Ctrl+C` 仍复制全文。
8. 无模板块时 `[⊞]` 置灰且点击无效。
9. `cargo fmt`、`cargo clippy --all-targets -- -D warnings`、`cargo test` 全绿。
10. 用户运行 `cargo run` 确认外观与交互。

## 13. 实施记录（2026-10-26）

- 已按本设计完成；`src/layout.rs` 的 prompt 区域几何与工具栏常量/命中函数已**纯搬迁**到
  `src/layout/prompt.rs`，`layout.rs` 以 `mod prompt; pub use prompt::*;` 保持原路径。
- 与设计初稿的差异：
  1. `visual_rows()` 返回空 vec 时，现有 `cursor_visual_in` / `content_offset_at` 存在越界风险，
     已补空保护，并同步保护 `cursor_cell`、`scroll_cursor_into_view` 与四个方向移动入口。
  2. 删除接续的「下一个块」判定使用 `start >= removed_start`：删除后紧邻的下一块会顶到
     原起始行，按初稿的 `>` 会跳过它。
  3. `exit_block_focus()` 清除聚焦后重算滚动；`reconcile_block_focus()` 改变聚焦范围后同样
     重算，避免块行集合与全文行集合切换时把光标甩出视口。
  4. 工具栏切换块聚焦时清除 prompt 选区，避免过滤后的行坐标在退出后映射到错误文本。
  5. `set_text` 内部仍不碰聚焦字段；退出由 design §4.6 的四处显式路径负责。
  6. 2026-10-26 交互调整：树视图往返不再清空块聚焦（决策 8/26 反转）。`show_prompt_tree`
     不清空，块聚焦行随 `EditorStash.history` 暂存；返回同一目标且正文未变时由
     `restore_history` 恢复，打开别的目标则消费暂存后按普通视图重建。
     `open_temp_prompt` 与 `restore_prompt_view` 兜底空草稿补显式退出，防止新草稿继承旧聚焦行。
- 验证：`cargo fmt`、`cargo clippy --all-targets -- -D warnings`、`cargo test`（lib 3571 passed）
  全绿；集成测试含真实鼠标的「块聚焦→列表→返回」往返。用户确认合并，视为交互验收通过。
- 合并记录：2026-10-27 合入 `dev`，merge commit `b8a3915`；功能提交 `a7c93d8`、`6d7a139`。
- 清理：`feat/prompt-block-focus` 分支与 `.worktrees/prompt-block-focus` 已移除，隔离库
  `~/.lx-t-dev/db/worktrees/prompt-block-focus-779b288e/` 同步删除。
