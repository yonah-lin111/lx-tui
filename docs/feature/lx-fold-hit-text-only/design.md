# lx 消息列表折叠命中区收窄为文字：设计方案

> 状态：**已通过 review 修复与自动化验证，已合并到 `dev`，待用户手工验收**
>
> 关联文档：
> - 任务清单：`docs/feature/lx-fold-hit-text-only/task.md`
> - Review 与修复记录：`docs/feature/lx-fold-hit-text-only/review.md`
> - 项目规范：`docs/standards/code-writing-standards.md`、`docs/standards/project-directory-structure.md`、`docs/standards/tui-design-requirements.md`
>
> 适用范围：主 agent 消息列表、BTW 对话列表、Agents（subagent）详情列表。三者共用同一套历史快照与命中入口，改一处即同步生效。

---

## 1. 背景与现状（代码事实）

### 1.1 唯一的命中入口

所有 lx 消息列表的折叠/展开命中都走：

- `src/ui/main_content/lx/history_hit.rs::card_at_source`
- `src/ui/main_content/lx/history_hit.rs::card_target_at`
- `src/ui/main_content/lx/history_hit.rs::task_agents_card_target_at`
- `src/ui/main_content/lx/history_hit.rs::task_btw_card_target_at`

绘制、滚动条、置顶行与命中共享同一份 `HistorySnapshot`，点击面登记在 `HistorySnapshot.cards`。

现状 `CardRegion`（`src/ui/main_content/lx/history/snapshot.rs:93`）：

```rust
pub(crate) struct CardRegion {
    pub(crate) kind: CardKind,
    pub(crate) card_start: usize,
    pub(crate) card_rows: usize,
    /// 行内芯片的 `(首行起始列, 末行结束列)`；`None` 表示整行宽。
    pub(crate) card_cols: Option<(u16, u16)>,
    pub(crate) clickable: bool,
}
```

现状命中（`history_hit.rs::card_at_source`）对 `card_cols` 的处理：

```rust
let Some((first_col, last_col)) = region.card_cols else {
    return true; // 整行宽
};
// 单行：首列..末列
// 多行：首行从首列到整宽、中间行从 USER_INDENT 到整宽、末行从 USER_INDENT 到末列
```

`card_cols = None` 时，整个矩形宽度都可点。因此用户在卡片右侧空白、树线续行的空白处点击，也会触发展开/折叠。

### 1.2 当前各折叠块的点击面

| 折叠块 | 登记位置 | 现状点击面 |
|---|---|---|
| 用户消息 `… N more lines` / `less lines` | `snapshot.rs` 用户消息构建末尾 | 提示行整行宽 |
| 用户消息粘贴芯片 | `snapshot.rs` 用户消息构建末尾 | 已按芯片列区间（`Some(...)`） |
| thinking 头部 + 展开正文 | `snapshot/steps.rs` 步骤扫描 | 整块（头部 + 所有正文行）整行宽 |
| 命令/交互卡参数区 | `tool.rs::command_regions` | 参数区所有物理行整行宽 |
| 命令/交互卡输出区 | `tool.rs::command_regions` | 输出区所有物理行整行宽 |
| 文件 diff 卡 | `tool.rs::card_regions_with_group` | 标题 + 全部正文，整行宽 |
| 文件摘要卡（read/write 等） | 同上 | 标题 + 全部详情，整行宽 |
| search 卡 | 同上 | 标题 + 全部正文，整行宽 |
| subagent 卡 | 同上 | 标题 + 全部正文，整行宽 |
| web_search 查询区 | `tool.rs::split_regions` | 标题 + 查询行，整行宽 |
| web_search 结果区 | `tool.rs::split_regions` | 结果行，整行宽 |
| 合并组内调用体 | `tool/group.rs::body_card_regions` | 由整卡区域平移，仍整行宽 |
| 本地命令卡参数/输出 | `user_shell.rs::regions` | 两个区各自整行宽 |
| 子 agent 汇报块 | `snapshot.rs::reports` | 整块 + `… N more lines` 提示行，整行宽 |
| compact / model switch / recap 正文 | `snapshot.rs` + `system_event.rs::click_span` | 可折叠正文区整行宽 |
| 运行中的命令卡 | `tool.rs::card_regions_with_group` | 整卡（打开后台任务面板） |
| 运行中的本地命令卡 | `user_shell.rs::regions` | 整卡（打开后台任务面板） |
| 子 agent 卡片里的 agent 链接行 | `snapshot/steps.rs` | 整行（打开 Agents 面板） |
| 附件行 / 原图下载按钮 | `paint.rs::push_card_regions` | 已是文字列区间 |

补充事实：

- `rewind`（撤销提示）与系统通知没有登记 `CardRegion`，当前不可点击，不在本次范围。
- 用户消息正文本身不是折叠触发面，只有折叠提示行是；因此用户消息不需要给正文算文字列。
- 粘贴芯片的“展开态点芯片文字（未拖选）收起”，实现见 `src/app/prompt/folded.rs::finish_preview_selection`，是本任务的交互参考。

### 1.3 问题复现

以命令卡输出为例：

```text
■ Bash
  ├─ $ ls -la
  └─ total 0
     drwxr-xr-x  3 user staff   96 Nov 16 08:00 .
     … 18 more lines
```

现在点击 `total 0` 右侧第 40 列空白，会展开输出区；点击任意正文行右侧空白也会触发展开/折叠。用户要求只有点到文字本身才触发。

---

## 2. 目标与非目标

### 2.1 目标

1. 所有**展开/折叠**点击面从“整行宽”收窄为“该物理行文字覆盖的列区间”。
2. 逐物理行独立计算：每行从第一个非空白字符到最后一个非空白字符。
3. 只含树线（`│`、`├─`、`└─`）或空白的行不算可点文字。
4. 主 agent、BTW、Agents 详情三处行为一致。
5. 点击文字仍然可以拖拽选中文本；单击（未拖出选区）才切换折叠，保持现有 `lx_card_press` / `end_lx_selection_drag` 语义。

### 2.2 非目标（本次不做）

1. **不改纵向范围**：各块原来哪些行能点，改后仍是哪些行能点；只收窄横向。
2. **不统一工具卡标题行行为**：
   - 命令卡 / 交互卡标题行仍然不触发（点击面从第 1 行开始）；
   - 文件卡、search 卡、subagent 卡、web_search 查询区标题行仍然触发（点击面从第 0 行开始）。
3. **不改打开类动作**：
   - 运行中的命令卡整卡打开后台任务面板，保持整卡；
   - 运行中的本地命令卡整卡打开后台任务面板，保持整卡；
   - 子 agent 卡片里的 agent 链接行保持整行可点。
4. 不改变粘贴芯片、附件行、原图下载按钮的命中规则。
5. 不改变键盘快捷键（`Ctrl+T` 等）、颜色、字形、折叠预算。
6. 不新增配置项、数据库迁移或用户可见文案。

---

## 3. 已确认的边界（grill 结论）

| 编号 | 决策 |
|---|---|
| Q1 | 只收窄横向，保留现有纵向范围；折叠区内任意一行的文字都可触发；展开态仍可点正文文字收起。 |
| Q2 | 逐物理行独立计算文字列：第一个非空白字符 → 最后一个非空白字符；只有树线或空白的行不可点。 |
| Q3 | 非折叠类点击面保持现状：运行中命令卡整卡打开面板、agent 链接行整行可点。 |
| Q4 | 覆盖全部折叠块：用户提示行、thinking、工具卡、本地命令卡、子 agent 汇报、compact/model switch/recap、合并组调用体。 |
| Q5 | 各工具卡现有纵向范围不变：命令卡标题不新增可点，文件/search/subagent/web_search 标题继续可点；本次不统一。 |

---

## 4. 命中模型设计

### 4.1 新增 `CardCols`

位置：`src/ui/main_content/lx/history/cards.rs`（与 `CardKind` 同文件，命中数据的唯一来源）。

```rust
/// 一个点击面的列范围口径。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum CardCols {
    /// 整行宽：运行中的命令/本地命令卡（打开后台任务面板）、agent 链接行。
    Full,
    /// 折叠芯片：首行起列、末行止列；沿用现有用户粘贴芯片的多行口径。
    Chip { first: u16, last: u16 },
    /// 逐物理行的文字列区间（相对内容区左缘）；`None` 行不可点。
    Text(Vec<Option<(u16, u16)>>),
}

impl CardCols {
    /// 合并组调用体：去掉标题行后的区域切片。
    pub(crate) fn skip_rows(&self, skip: usize) -> Self {
        match self {
            Self::Full => Self::Full,
            Self::Chip { .. } => self.clone(),
            Self::Text(cols) => Self::Text(cols.get(skip..).unwrap_or(&[]).to_vec()),
        }
    }

    /// 仅用于缓存 LRU 记账的近似字节数。
    pub(crate) fn heap_bytes(&self) -> usize {
        match self {
            Self::Text(cols) => cols.len() * std::mem::size_of::<Option<(u16, u16)>>(),
            Self::Full | Self::Chip { .. } => 0,
        }
    }
}
```

`CardCols` 从 `history.rs` 以 `pub(crate) use cards::CardCols;` 导出，供 `tool`、`snapshot`、`paint` 使用。

### 4.2 修改 `CardRegion` / `CardSpan`

`src/ui/main_content/lx/history/snapshot.rs`：

```rust
pub(crate) struct CardRegion {
    pub(crate) kind: CardKind,
    pub(crate) card_start: usize,
    pub(crate) card_rows: usize,
    pub(crate) card_cols: CardCols,
    pub(crate) clickable: bool,
}
```

`src/ui/main_content/lx/history/paint.rs`：

```rust
pub(crate) struct CardSpan {
    pub(crate) call: usize,
    pub(crate) section: CardSection,
    pub(crate) expanded: bool,
    pub(crate) offset: usize,
    pub(crate) rows: usize,
    pub(crate) cols: CardCols,
    pub(crate) clickable: bool,
}
```

`CardSpan` 不再 `Copy`（`CardCols::Text` 含 `Vec`）；现有代码只读 `&[CardSpan]`、按值 `map`，不会触发额外克隆。

`src/ui/main_content/lx/history/tool.rs`：

```rust
pub(crate) struct CardRegion {
    pub(crate) section: CardSection,
    pub(crate) card: RowSpan,
    pub(crate) cols: CardCols,
    pub(crate) clickable: bool,
}
```

### 4.3 命中算法

`history_hit.rs::card_at_source` 的核心改为：

```rust
let offset = content_row.saturating_sub(region.card_start);
let (first_col, last_col) = match &region.card_cols {
    CardCols::Full => (0, width),
    CardCols::Chip { first, last } => {
        chip_cols(*first, *last, offset, region.card_rows)?
    }
    CardCols::Text(cols) => *cols.get(offset).and_then(Option::as_ref)?,
};
let last_col = last_col.min(width);
if rel_col < first_col || rel_col >= last_col {
    return None;
}
```

`chip_cols` 保持现有粘贴芯片口径，不改变行为：

```rust
/// 折叠芯片的列区间：单行直接取区间；多行按首行/中间行/末行分别处理。
fn chip_cols(
    first: u16,
    last: u16,
    offset: usize,
    rows: usize,
) -> Option<(u16, u16)> {
    if rows <= 1 {
        return Some((first, last));
    }
    if offset == 0 {
        Some((first, u16::MAX))
    } else if offset + 1 == rows {
        Some((USER_INDENT, last))
    } else {
        Some((USER_INDENT, u16::MAX))
    }
}
```

`CardHit.card`（命中区域矩形）：

- `CardCols::Text`：返回当前点击行的文字矩形（高度 1、x 为 `body.x + first_col`、宽度为 `last_col - first_col`），不再返回整块矩形；
- `CardCols::Chip`：保持现有 `clip` 结果；
- `CardCols::Full`：保持现有整块裁剪结果。

`CardHit.card` 目前只被测试与 `layout` 读取（`src/ui/main_content/lx/history/layout.rs`），生产路径只用 `kind` 与 `clickable`。

### 4.4 不变量

1. `CardCols::Text` 的长度应等于 `card_rows`；命中时用 `get(offset)`，长度不足的行视为不可点（安全降级）。
2. 所有列区间在命中时统一 `min(width)` 裁剪。
3. `first_col >= last_col` 的行不可点。
4. 空白行（`is_whitespace` 全部为真）返回 `None`，不登记可点行。
5. `CardCols::Chip` 的 `USER_INDENT` 口径与用户消息左缩进一致（2 列），保持现有行为。
6. 运行中卡片、agent 链接行必须使用 `CardCols::Full`，不得被误改成 `Text`。

---

## 5. 文字列区间计算

### 5.1 新增共享模块 `history/text_cols.rs`

职责：只做“显示文本 → 可见列区间”的纯计算，不依赖 `Grid`、不渲染。

```rust
//! 物理行文字列区间：命中几何的唯一计算口径。

use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use crate::ui::markdown_render::MdLine;

use super::tool::Segment;

/// 从 `start_col` 起扫描若干段文本；返回首个非空白字符到末个非空白字符的列区间。
///
/// - 整行只有树线/空白 → `None`（不可点）；
/// - 行内只要有一个正文/标志字符，区间就从第一个非空白字符算起（含 `├─`/`│`）；
/// - 行尾空白不进入区间。
pub(super) fn cols_from<'a>(
    start_col: u16,
    parts: impl IntoIterator<Item = &'a str>,
) -> Option<(u16, u16)> {
    let mut col = u32::from(start_col);
    let mut first: Option<u32> = None;
    let mut last = 0u32;
    let mut has_text = false;
    for part in parts {
        for ch in part.chars() {
            let width = u32::from(ch.width().unwrap_or(0));
            if !ch.is_whitespace() {
                first.get_or_insert(col);
                last = col.saturating_add(width);
                if !is_tree_decoration(ch) {
                    has_text = true;
                }
            }
            col = col.saturating_add(width);
        }
    }
    has_text.then(|| (clamp_u16(first.unwrap_or(0)), clamp_u16(last)))
}

/// 该字符是否只属于树体/表格装饰：连接符、竖线、横线、框线。
fn is_tree_decoration(ch: char) -> bool {
    matches!(
        ch,
        '├' | '└' | '│' | '─' | '┬' | '┴' | '┼' | '┤' | '┌' | '┐' | '┘'
            | '┗' | '┏' | '┓' | '┣' | '┫' | '┳' | '┻' | '╋'
    )
}

/// `Segment` 行（树体、汇报正文等）的文字列区间。
pub(super) fn segment_cols(segments: &[Segment]) -> Option<(u16, u16)> {
    cols_from(0, segments.iter().map(|segment| segment.text.as_str()))
}

/// 单个纯文本行的文字列区间。
pub(super) fn line_cols(text: &str) -> Option<(u16, u16)> {
    cols_from(0, std::iter::once(text))
}

/// Markdown 物理行：行首 `indent` 计入列宽，返回相对行首的区间。
pub(super) fn md_line_cols(line: &MdLine) -> Option<(u16, u16)> {
    cols_from(
        line.indent,
        line.spans.iter().map(|(text, _)| text.as_str()),
    )
}

fn clamp_u16(value: u32) -> u16 {
    u16::try_from(value).unwrap_or(u16::MAX)
}
```

说明：

- 宽度函数与渲染保持一致：`UnicodeWidthChar` / `UnicodeWidthStr`，宽字符占 2 列，控制字符按 0 列（与 `Segment::cols()` 同口径）。
- 跨段连续扫描：段与段之间的空格计入列宽；首个非空白字符之前的前导空格/缩进不计入 `start`。
- 行尾空格不进入 `last`。
- 表格行的 spans 已经是表格原始框线文本（宽度等于 `TableLayoutMeta::total_width`），不需要单独处理表格。

### 5.2 树体行：`tool/tree.rs`

```rust
impl Layout {
    /// 每个物理行的文字列区间；行数与 `rows` 严格一一对应。
    pub(crate) fn row_cols(&self) -> Vec<Option<(u16, u16)>> {
        self.rows.iter().map(Row::text_cols).collect()
    }
}

impl Row {
    pub(crate) fn text_cols(&self) -> Option<(u16, u16)> {
        super::super::text_cols::cols_from(
            0,
            std::iter::once(self.prefix)
                .chain(self.segments.iter().map(|segment| segment.text.as_str())),
        )
    }
}
```

覆盖范围：

- 命令/交互卡参数与输出（`command.rs`）
- 文件摘要（`file_summary.rs`）
- search / subagent / web_search 树体
- 本地命令卡（`user_shell.rs`）
- system event 正文（`system_event.rs`）
- 工具合并组调用体（`group.rs`）

### 5.3 thinking 正文

`paint.rs` 已有 `MdLine` 列表；`md_line_cols` 返回相对行首区间，再叠加 `thinking_body_indent`：

```rust
pub(super) fn thinking_body_cols(
    body: &[crate::ui::markdown_render::MdLine],
    width: u16,
) -> Vec<Option<(u16, u16)>> {
    let base = thinking_body_indent(width);
    let tree = thinking_tree(width);
    body.iter()
        .enumerate()
        .map(|(index, line)| {
            let (start, end) = text_cols::md_line_cols(line)?;
            // 首个正文行在 `tree` 模式下从 0 列画 `  └─`，因此起点为 0。
            let start = if tree && index == 0 {
                0
            } else {
                base.saturating_add(start)
            };
            Some((start, base.saturating_add(end).min(width)))
        })
        .collect()
}
```

### 5.4 thinking 头部

把绘制时的标签生成抽成一个纯函数，快照与绘制共用，避免两套文案漂移：

```rust
/// 思考头部文案：生成中 `Thinking`；完成 `Thought · 6.3s`（无耗时不加分隔）。
pub(super) fn thinking_header_label(part: &ChatPartView, streaming: bool) -> String {
    if streaming {
        text::LX_CHAT_THINKING_LABEL.to_string()
    } else {
        let mut label = String::from(text::LX_CHAT_THOUGHT_LABEL);
        if let Some(duration) = thinking_duration(part) {
            label.push_str(THINKING_DURATION_SEP);
            label.push_str(&format_duration(duration));
        }
        label
    }
}

/// 思考头部文字列区间：标记在 0 列，标签从 `STEP_GUTTER` 列开始。
pub(super) fn thinking_header_cols(
    part: &ChatPartView,
    streaming: bool,
    width: u16,
) -> Option<(u16, u16)> {
    if width == 0 {
        return None;
    }
    let label = thinking_header_label(part, streaming);
    let end = STEP_GUTTER
        .saturating_add(u16::try_from(label.width()).unwrap_or(u16::MAX))
        .min(width);
    Some((0, end))
}
```

`draw_thinking` 改用 `thinking_header_label`，行为与配色不变。

### 5.5 各模块取列来源

| 折叠块 | 行数来源 | 文字列来源 |
|---|---|---|
| 用户消息提示行 | 1 行（`fold_hint`） | `line_cols(hint)` + `USER_INDENT` |
| 用户粘贴芯片 | `UserPasteSpan` | 保持 `CardCols::Chip` |
| thinking | `1 + body.len()` | `thinking_header_cols` + `thinking_body_cols` |
| 命令/交互参数 | `command::Blocks.params` | `params_layout.row_cols()` |
| 命令/交互输出 | `command::Blocks.output` | `output_layout.row_cols()` |
| 文件 diff | 标题 + `layout_with_group` 行数 | 标题 `header_with_verb` 的文字列 + `file_diff` 每行 `segment_cols` |
| 文件摘要 | 标题 + `detail_layout_with_group` 行数 | 标题 `header(...)` 的文字列 + `Layout::row_cols()` |
| search | 标题 + `layout` 行数 | 标题 `header_with_verb` 的文字列 + `Layout::row_cols()` |
| subagent | 标题 + `layout` 行数 | 标题（`spawn_agent` 用 `header_with_mark`，其余 `header_with_verb`）的文字列 + `Layout::row_cols()` |
| web_search 查询区 | 标题 + `queries_rows` | 标题文字列 + 查询块 `row_cols()` |
| web_search 结果区 | `result_rows` | 结果块 `row_cols()` |
| 合并组调用体 | `body_rows` | 整卡列区间按 `HEADER_ROWS` 切片 |
| 本地命令参数 | `blocks.params` | `body_layout().row_cols()[..params]` |
| 本地命令输出 | `blocks.output` | `body_layout().row_cols()[params..]` |
| 子 agent 汇报 | `report::measure` | 标题/链接/`Report:`/正文/提示逐行计算 |
| compact/model switch/recap | `SystemCard::rows` | `body.row_cols()` 按 `click_span` 起点切片 |

---

## 6. 关键代码改动

### 6.1 `history/tool.rs`：工具区域带列区间

```rust
fn command_regions(
    call: &ToolCallView,
    width: u16,
    blocks: command::Blocks,
    in_group: bool,
) -> Vec<CardRegion> {
    let mut regions = vec![CardRegion {
        section: CardSection::Body,
        card: RowSpan {
            offset: HEADER_ROWS,
            rows: blocks.params,
        },
        cols: CardCols::Text(blocks.params_cols),
        clickable: command::params_expandable(call, width),
    }];
    if blocks.output > 0 {
        regions.push(CardRegion {
            section: CardSection::Output,
            card: RowSpan {
                offset: HEADER_ROWS + blocks.params,
                rows: blocks.output,
            },
            cols: CardCols::Text(blocks.output_cols),
            clickable: command::output_expandable_with_group(call, width, in_group),
        });
    }
    regions
}
```

运行中命令卡保持：

```rust
return vec![CardRegion {
    section: CardSection::Body,
    card: RowSpan {
        offset: 0,
        rows: HEADER_ROWS + blocks.params + blocks.output,
    },
    cols: CardCols::Full, // 打开后台任务面板：保持整卡
    clickable: true,
}];
```

整卡类工具（文件/search/subagent）：

```rust
let header = header_cols(call, width); // Option<(u16, u16)>
let body = body_cols(call, width, folds, in_group); // Vec<Option<(u16, u16)>>
let cols = std::iter::once(header).chain(body).collect();
vec![CardRegion {
    section: CardSection::Body,
    card: RowSpan {
        offset: 0,
        rows: step_rows,
    },
    cols: CardCols::Text(cols),
    clickable: true,
}]
```

`header_cols` 只按宽度计算，不依赖动画帧：

```rust
/// 工具卡片标题行的文字列区间（与 draw 同文案；帧不影响宽度，固定传 0/true）。
fn header_cols(call: &ToolCallView, width: u16) -> Option<(u16, u16)> {
    let segments = header(call, width, 0, true);
    text_cols::segment_cols(&segments)
}
```

文件 diff / search / subagent / web_search 各自的标题构造函数分别返回同款 `Option<(u16, u16)>`：

```rust
// file_diff.rs
pub(super) fn header_cols(call: &ToolCallView, width: u16) -> Option<(u16, u16)> {
    let segments = header_with_verb(call, width, 0, true, verb_of(call), None);
    text_cols::segment_cols(&segments)
}

// search.rs
pub(super) fn header_cols(
    call: &ToolCallView,
    view: &SearchDetailView,
    width: u16,
) -> Option<(u16, u16)> {
    let segments = header_with_verb(
        call,
        width,
        0,
        true,
        text::agent::VERB_TOOL_SEARCH,
        status_of(call, view),
    );
    text_cols::segment_cols(&segments)
}
```

> `subagent` 的 `spawn_agent` 走 `header_with_mark`，`wait_agent` 走 `header_with_verb`，分别保持与 `draw` 相同的分支；宽度与帧无关，固定 `0/true`。

### 6.2 `tool/command.rs`：`Blocks` 携带逐行列区间

```rust
pub(super) struct Blocks {
    pub(super) params: usize,
    pub(super) output: usize,
    pub(super) params_cols: Vec<Option<(u16, u16)>>,
    pub(super) output_cols: Vec<Option<(u16, u16)>>,
}
```

`blocks_with_group` 只排版一次，行数与列区间同源：

```rust
let params_layout = if in_group && !output.is_empty() {
    tree::layout_in_group(&params, width, 1)
} else {
    tree::layout(&params, width)
};
let output_layout = if in_group {
    tree::layout_in_group(&output, width, 0)
} else {
    tree::layout(&output, width)
};
Blocks {
    params: params_layout.rows.len(),
    output: output_layout.rows.len(),
    params_cols: params_layout.row_cols(),
    output_cols: output_layout.row_cols(),
}
```

### 6.3 `tool/group.rs`：合并组调用体切片

```rust
pub(crate) fn body_card_regions(
    call: &ToolCallView,
    width: u16,
    folds: Folds,
    total_rows: usize,
) -> Vec<super::CardRegion> {
    super::card_regions_with_group(call, width, folds, total_rows, true)
        .into_iter()
        .filter_map(|region| {
            let offset = region.card.offset;
            let rows = region.card.rows;
            if offset == 0 {
                if rows <= super::HEADER_ROWS {
                    return None;
                }
                Some(super::CardRegion {
                    card: super::RowSpan {
                        offset: 0,
                        rows: rows.saturating_sub(super::HEADER_ROWS),
                    },
                    cols: region.cols.skip_rows(super::HEADER_ROWS),
                    ..region
                })
            } else {
                Some(super::CardRegion {
                    card: super::RowSpan {
                        offset: offset.saturating_sub(super::HEADER_ROWS),
                        rows,
                    },
                    ..region
                })
            }
        })
        .collect()
}
```

### 6.4 `user_shell.rs`：参数/输出两块的列区间

```rust
pub fn regions(shell: &UserShellView, width: u16, folds: Folds) -> Vec<Region> {
    let blocks = blocks(shell, width, folds);
    let cols = body_layout(shell, width, folds).row_cols();
    let params_end = 1 + blocks.params;

    if shell.is_running() {
        // 运行中整卡打开后台任务面板：保持整卡命中。
        return vec![
            Region {
                section: CardSection::Body,
                offset: 0,
                rows: params_end,
                cols: CardCols::Full,
                clickable: true,
            },
            // output 区同理 Full
        ];
    }

    let mut out = vec![Region {
        section: CardSection::Body,
        offset: 1,
        rows: blocks.params,
        cols: CardCols::Text(
            cols.get(..blocks.params).unwrap_or(&[]).to_vec(),
        ),
        clickable: params_expandable(shell, width),
    }];
    if blocks.output > 0 {
        out.push(Region {
            section: CardSection::Output,
            offset: params_end,
            rows: blocks.output,
            cols: CardCols::Text(cols.get(blocks.params..).unwrap_or(&[]).to_vec()),
            clickable: output_expandable(shell, width),
        });
    }
    out
}
```

### 6.5 `report.rs`：`Measure` 携带逐行列区间

```rust
pub(super) struct Measure {
    pub(super) rows: usize,
    pub(super) hint_row: Option<usize>,
    pub(super) has_more: bool,
    /// 每个物理行的文字列区间（相对卡片左缘）；与 `rows` 等长。
    pub(super) cols: Vec<Option<(u16, u16)>>,
}
```

`measure` 内一次算完：

```rust
let indent = body_indent(width);
let header_cols = text_cols::segment_cols(&header_segments(report));
let agent_cols = text_cols::segment_cols(&agent_segments(report, width));
let body_cols: Vec<_> = all.iter()
    .take(shown)
    .map(|line| text_cols::segment_cols(line).map(|(s, e)| {
        (indent.saturating_add(s), indent.saturating_add(e))
    }))
    .collect();
let hint_cols = (remaining > 0).then(|| {
    let hint = text::agent::file_diff_more_lines(remaining);
    text_cols::line_cols(&hint).map(|(s, e)| {
        (indent.saturating_add(s), indent.saturating_add(e))
    })
});
// cols = [header, agent, (Report: label), body..., hint...]
```

`draw` 里的 header / agent / label 构造函数抽成纯函数，`measure` 与 `draw` 共用，避免文案与截断口径漂移。

### 6.6 `system_event.rs`：`click_span` 带列区间

```rust
pub(super) struct ClickSpan {
    pub(super) offset: usize,
    pub(super) rows: usize,
    pub(super) cols: Vec<Option<(u16, u16)>>,
}

pub(super) fn click_span(&self) -> Option<ClickSpan> {
    let first = self.body.node_rows.get(self.fixed_count).copied().unwrap_or(0);
    let rows = self.body.rows.len().saturating_sub(first);
    (rows > 0).then(|| ClickSpan {
        offset: self.header.len() + first,
        rows,
        cols: self.body.row_cols().get(first..).unwrap_or(&[]).to_vec(),
    })
}
```

> 当前 `click_span` 只返回 `(offset, rows)`；`fixed_count` 需要保存在 `SystemCard` 内（现有 `click` 字段可替换为 `ClickSpan`）。

### 6.7 `snapshot.rs`：用户提示行、粘贴芯片、报告与系统事件

用户提示行：

```rust
if fold_hint.is_some() {
    let hint = fold_hint.as_deref().unwrap_or_default();
    let (start_col, end_col) = text_cols::line_cols(hint)
        .map(|(start, end)| {
            (
                super::USER_INDENT.saturating_add(start),
                super::USER_INDENT.saturating_add(end),
            )
        })
        .unwrap_or((super::USER_INDENT, super::USER_INDENT));
    self.snapshot.cards.push(CardRegion {
        kind: CardKind::UserMessage {
            message_id: message.id.clone(),
            expanded: user_expanded,
        },
        card_start: start.saturating_add(text_lines.len()),
        card_rows: 1,
        card_cols: CardCols::Text(vec![Some((start_col, end_col))]),
        clickable: true,
    });
}
```

粘贴芯片保持：

```rust
card_cols: CardCols::Chip {
    first: span.cols.0,
    last: last_col,
},
```

报告：

```rust
let card_cols = CardCols::Text(metrics.cols.clone());
// 全文块
CardRegion {
    kind: CardKind::Report {
        message_id: message.id.clone(),
        agent_id: report.agent_id.clone(),
        expanded,
        more: false,
    },
    card_start: start,
    card_rows: block_rows,
    card_cols: cols.clone(),
    clickable: true,
}
// 提示行：取同一份 cols 的第 hint_row 项（get + flatten，越界/空行安全降级）
CardRegion {
    kind: CardKind::Report { .. more: true },
    card_start: start.saturating_add(offset),
    card_rows: 1,
    card_cols: CardCols::Text(vec![
        cols.get(offset).copied().flatten(),
    ]),
    clickable: true,
}
```

compact / model switch / recap：

```rust
if let Some(span) = card.click_span() {
    self.snapshot.cards.push(CardRegion {
        kind: CardKind::Compact { .. },
        card_start: start + span.offset,
        card_rows: span.rows,
        card_cols: CardCols::Text(span.cols),
        clickable: true,
    });
}
```

### 6.8 `snapshot/steps.rs`：thinking

```rust
if let AssistantStepView::Part { part, .. } = step
    && part.kind == crate::chat::events::PartKind::Reasoning
{
    let streaming = source.chat.reasoning_streaming(message_index, step_index);
    let mut cols = Vec::with_capacity(block.rows);
    cols.push(paint::thinking_header_cols(part, streaming, self.width));
    if let BlockPaint::Thinking { body } = block.paint.as_ref() {
        cols.extend(paint::thinking_body_cols(body, self.width));
    }
    self.snapshot.cards.push(CardRegion {
        kind: CardKind::Thinking {
            message_index,
            step_index,
            expanded: source.chat.reasoning_expanded(message_index, step_index),
        },
        card_start: block_start,
        card_rows: block.rows,
        card_cols: CardCols::Text(cols),
        clickable: true,
    });
}
```

### 6.9 `paint.rs`：缓存记账与标题复用

- `BlockPaint::bytes()` 的 `Legacy` 分支改为：

```rust
Self::Legacy { spans, .. } => spans
    .iter()
    .map(|span| std::mem::size_of::<CardSpan>() + span.cols.heap_bytes())
    .sum(),
```

- `draw_thinking` 的标签生成改用 `thinking_header_label`，不再内联 `if streaming { ... }` 分支。

### 6.10 附件行 / 原图按钮

只改类型，不改行为：

```rust
card_cols: CardCols::Chip {
    first: USER_INDENT,
    last: prefix_cols.saturating_add(label_cols),
},
```

---

## 7. 边界情况与处理

### 7.1 行内有宽字符

统一用 `UnicodeWidthChar` / `UnicodeWidthStr` 计算列宽，与渲染层 `Segment::cols()`、`put_clipped` 一致。中文、emoji、组合字符不会出现“看着在文字上却点不中”或“点到空白却触发”。

### 7.2 行被窗格宽度裁剪

`last_col` 在命中时统一 `min(width)`。一行的文字超出窗格时，可见的最后一段仍可点；不可见部分不参与命中。

### 7.3 空白行 / 纯树线行

`text_cols` 只把非空白字符计入区间，并区分“装饰”与“正文”：

- `│` 单独一行：整行只有装饰 → `None`，不可点；
- `  ├─ cmd: ls`：`├─` 是装饰，但行内有 `cmd` / `ls` 正文 → 区间从 `├` 到 `ls`；
- `├───┼───┤`（表格边框）：整行只有装饰 → `None`，不可点。

判定规则：

```rust
// text_cols::cols_from 内：
if !ch.is_whitespace() {
    first.get_or_insert(col);          // 首个非空白字符（含树线）
    last = col.saturating_add(width);  // 末个非空白字符
    if !is_tree_decoration(ch) {
        has_text = true;               // 至少有一个正文/标志字符
    }
}
// 最后：
has_text.then(|| (first.unwrap_or(0), last))
```

> `■` / `●` / `▲` / `◆` / `…` 等可见标志不算装饰，因此“只有标记 + 空白”的行仍可点；只有纯树线/框线/空白行不可点。

### 7.4 表格行

`MdLine.spans` 已是表格框线的真实文本，列宽等于 `TableLayoutMeta::total_width`；按普通行计算即可。表格行只有边框（如 `├───┼───┤`）时，`has_text` 判定为装饰，不登记点击。

### 7.5 代码块按钮

`[❐]` 复制按钮在 `chat_hit` 中先于折叠命中处理，行为不变。文字列区间覆盖按钮区域时，复制按钮仍优先。

### 7.6 用户粘贴芯片

保持 `CardCols::Chip` 的现有首行/中间行/末行口径，不改成逐行文字。原因：粘贴芯片的展开态可能被窗格宽度裁剪，`UserPasteSpan` 只记录了首行起始列与末行结束列；改成逐行需要重建投影数据，超出本任务范围且会改变用户已经习惯的交互。

### 7.7 报告提示行优先级

报告提示行 `… N more lines` 的 `CardRegion` 先于整块登记，`card_at_source` 取第一条重叠区域。文字列区间在提示行与整块上取同一份 `Measure.cols[hint_row]`，因此：

- 点提示行文字 → `more: true`（追加一批）；
- 点提示行空白 → 两个区域都拒绝，不触发展开；
- 点正文文字 → 整块 `more: false`（折叠）。

### 7.8 选择与折叠的先后

`press_content_hit` 在指针命中折叠文字时仍会 `begin_lx_selection` 并记录 `lx_card_press`；松开时 `end_lx_selection_drag` 先判断是否拖出了非空选区：

- 单击文字：无选区 → 切换折叠；
- 从文字开始拖拽：有选区 → 保留选区，不切换；
- 在空白处单击：不再命中折叠，进入普通选区/清选区路径（预期行为变化）。

### 7.9 运行中卡片

`CardCols::Full` 保持整卡可点，不受 `Text` 影响。涉及：

- `tool.rs` 运行中的 `ToolDetailView::Command(_)`
- `user_shell.rs` 的 `shell.is_running()`
- `CardKind::AgentLink`

---

## 8. 性能、内存与缓存

1. 列区间与块绘制数据同生命周期，挂在 `CardSpan` / `CardRegion` 上；只在快照缓存未命中时计算，不进入每帧热路径。
2. 每个物理行只存一个 `Option<(u16, u16)>`（4 字节）；典型 50 行卡片约 200 字节，1400 张卡片的最坏额外内存约 0.3 MiB，可接受。
3. `BlockPaint::bytes()` 增加 `CardCols::heap_bytes()`，避免 LRU 记账低估。
4. 现有块指纹已包含展开状态与宽度；列区间只依赖 `(内容, 宽度, 折叠态)`，不会漏失效。若实现时发现某模块的列区间依赖额外状态（如 streaming 文案），必须把该状态纳入对应块指纹（thinking 头部由快照构建时按当前 `reasoning_streaming` 计算，不缓存绘制数据）。

---

## 9. 测试与验收

### 9.1 新增单元测试

`src/ui/main_content/lx/history/text_cols/tests.rs`：

- 前导/尾随空白裁剪：`"  hello  "` → `(2, 7)`；
- 纯空白/空串 → `None`；
- 宽字符：`"  中文"` → 列宽 4；
- 纯树线行 `"  │"` → `None`；
- 树线 + 正文 `"  ├─ cmd: ls"` → 从 `├` 到 `ls`；
- `md_line_cols` 的 `indent` 计入列宽；
- `Layout::row_cols` 行数与 `rows` 严格一致；
- 合并组二级行按 8 列前缀取列（`layout_in_group_final`，`last=false/true` 各一条）；
- 合并组一级根行仍按 5 列前缀取列。

### 9.2 新增命中回归测试

`src/ui/main_content/lx/history/tests/text_hit.rs`（新文件，`history/tests.rs` 加 `mod text_hit;`）：

- `user_hint_text_toggles_but_row_tail_does_not`（含左侧缩进不命中）；
- `thinking_header_and_body_hit_text_only`；
- `folded_thinking_card_is_header_only`；
- `command_params_and_output_hit_text_only`（含纯 `│` 行硬断言）；
- `report_hint_and_body_hit_text_only`；
- `compact_body_hits_text_only`；
- `running_command_card_keeps_whole_row_hit`（Q3 回归）；
- `user_paste_chip_keeps_existing_chip_hit`（芯片行为回归）；
- `agents_history_shares_the_same_text_hit`；
- `btw_history_shares_the_same_text_hit`；
- `merged_group_rows_match_rendered_text_columns`（逐 Tool 卡片与真实渲染列对齐）；
- `web_search_title_targets_the_only_expandable_block`；
- `expanded_all_blank_output_still_can_collapse`；
- `shell_params_and_output_hit_text_only`。

### 9.3 现有测试更新

实现后实际处理：

- `src/ui/main_content/lx/history/tests/cards.rs`、`tests/user_folding.rs` 已按新列范围调整；
- `open_entry.rs`、`tool_geometry.rs`、`tool_grouping.rs`、`task/tests.rs`、
  `event/tests/mouse_click.rs`、`lx/tests.rs` 未逐文件改写，仍通过是因为点击列恰好
  落在文字上；
- 收窄行为统一由 `text_hit.rs` 的真实命中入口断言，不依赖上述旧测试的固定列坐标。

### 9.4 验证命令

```bash
cargo fmt
cargo clippy --all-targets -- -D warnings
cargo test ui::main_content::lx
cargo test event::tests::mouse_click
cargo test
```

### 9.5 手工验收（用户执行）

```bash
cargo run
```

1. 主 agent 列表：发一条 6 行以上用户消息，点 `… N more lines` 的文字 → 展开；点同一行右侧空白 → 不展开。
2. 展开思考块：点 `Thought · 6.3s` 文字 → 折叠；点头部右侧空白 → 不折叠；点正文文字 → 折叠；点正文右侧空白 → 不折叠。
3. 命令卡：折叠态点 `├─ $ ls -la` 的文字 → 展开；点行尾空白 → 不展开；展开后点输出正文文字 → 收起；点纯 `│` 行 → 不收起。
4. 子 agent 汇报：折叠态点 `… N more lines` 文字 → 追加一批；点提示行空白 → 不追加。
5. `/compact` 卡：点摘要文字 → 展开；点摘要行尾部空白 → 不展开。
6. BTW 页签与 Agents 详情：重复 2~5，行为一致。
7. 运行中的命令卡：点卡片右侧空白仍能打开后台任务面板（Q3 不变）。
8. 子 agent 卡片里的 agent 链接：点整行仍能打开 Agents 面板（Q3 不变）。
9. 文本选择：从折叠文字上按下拖拽 → 选中文本，不触发折叠；单击文字不拖拽 → 折叠。

---

## 10. 备选方案与否决原因

| 方案 | 否决原因 |
|---|---|
| 每个折叠区用一个矩形（左缩进 → 区最宽行） | Q2 已否决：一行很长会把整个矩形撑宽，短行右侧空白仍然误触。 |
| 只有标题行/提示行可点 | Q1 已否决：展开态没有提示行，正文无法收起，交互倒退。 |
| 命中时重新排版并逐行测量 | 命中路径是鼠标事件热路径，重新排版会卡顿，且必须与绘制共享几何；改为快照构建时算好。 |
| 改成整行只保留左侧 2 列可点 | 不满足“点文字才触发”，长行中后段文字会点不中。 |
| 统一把工具卡标题行改成可点/不可点 | Q5 已否决：超出本次“命中收窄”范围，回归面不可控，另立任务。 |

---

## 11. 实现顺序建议

1. `CardCols` + `CardRegion`/`CardSpan` + `card_at_source`（保持所有现有区域 `Full`，先保证编译与现有行为不变）。
2. `text_cols` 模块 + `tree::Layout::row_cols` + 单元测试。
3. 用户消息提示行、thinking（最容易验证）。
4. 命令卡 / 文件卡 / search / subagent / web_search / 合并组。
5. 本地命令卡、报告、compact/model switch/recap。
6. 更新现有测试 + 新增边界测试 + 全量验证。

> 每一步都可以单独编译、单独回归；不要在最后一次性切换所有区域。

---

## 12. Review 修复记录

实现完成后按 `review.md` 做了一轮独立审查，修复了以下偏差：

1. **P1 合并组二级前缀列错位**：新增 `tree::layout_in_group_final`，排版后立即写入
   最终 8 列连接符，再取 `row_cols()`；`apply_group_connector_with_roots` 允许
   `roots = 0`。`last` 从 `GroupSlot` 贯穿到各工具模块的列区间。
2. **P2 web_search 标题死点击面**：查询不可展开、结果可展开时，标题行改挂到
   `Output` 的独立 1 行点击面。
3. **P2 合并组状态尾巴不可点**：各工具模块先 `decorate_group_call` 再
   `row_cols()`，状态文字进入列区间。
4. **P2 纯空白展开块无法收起**：`CardCols::text_is_blank` + 命中侧退化为整块，
   只在区域本来 `clickable` 时生效。
5. **P3 本地命令卡列区间不同源**：`user_shell::blocks` 改用合并 `body_layout` 的
   `row_cols` 切片。

完整问题清单、错误原因与验证结果见 `review.md`。
