# `lx` 消息列表 Markdown 表格单元格长按拖选、高亮与去边框复制 — 详细设计 (`design.md`)

- **特性目录**：`docs/feature/lx-markdown-table-selection/`
- **关联任务清单**：`docs/feature/lx-markdown-table-selection/task.md`
- **关联评审报告**：`docs/feature/lx-markdown-table-selection/review.md`
- **当前状态**：已完成实现、代码评审与 P0–P3 修复，并合入主工作区 `dev` 分支（worktree `.worktrees/lx-markdown-table-selection` 与分支 `feat/lx-markdown-table-selection` 已移除）
- **参考基线**：`/Users/yonah/projects/agent/opencode-dev`（`@opentui/core` 中 `TextTableRenderable` 的三级选区解析 `single-cell` / `column-locked` / `grid` 与 `getSelectedText` 去边框 TSV 提取）
- **目标范围**：
  - `src/app/selection.rs`（补充 `Selection::anchor()` 与 `Selection::cursor()` 访问器，供表格方向感知与边框防手抖吸附使用）
  - `src/ui/markdown_render.rs`、`src/ui/markdown_render/table.rs`（为 `MdLine` 增加可选表格行引用 `Option<TableLineRef>`，在 `grid_table` 排版时一次性产出整表共享的 `Arc<TableLayoutMeta>`，记录每列边框/内容/真实文本区间、逻辑行物理区间、`<br>` 硬换行与软折行标记、空白填充行标记）
  - `src/ui/main_content/lx/grid.rs`（为 `Grid` 增加按行挂载的 `Option<GridTableRow>`，记录表格在内容区中的左起列偏移 `base_col` 与 `TableLineRef`）
  - `src/ui/main_content/lx/history/paint.rs`、`src/ui/main_content/lx/history/tool.rs`（在 `draw_markdown`、`draw_thinking` 与 `put_segments` 绘制 `MdLine` / `Segment` 时将 `TableLineRef` 连同实际起始列 `cx` 登记到 `Grid`）
  - `src/ui/main_content/lx/selection.rs`、`src/ui/main_content/lx/selection/table.rs`（新建子模块，实现对齐 `opencode-dev` 的三级表格选区解析 `SingleCell` / `ColumnLocked` / `Grid`、单元格留白与紧邻边框防手抖吸附、单元格内局部反显高亮 `paint_selection`、双击选词/三击选整格 `unit_cell_range`、以及软折行无缝拼接 + `<br>` 保换行 + 同行多列 `\t` / 跨行 `\n` 的 `copy_text`）
  - `src/ui/main_content/lx.rs`、`src/event/input.rs`（`lx::render` 将 `&grid` 传入 `selection::paint_selection`；`copy_lx_selection` 传递 `(selection.anchor(), selection.cursor())` 保留拖选起点与终点方向）
  - `src/ui/markdown_render/tests.rs`、`src/ui/main_content/lx/selection/tests.rs`（新增表格选区高亮、单格多行软折行/硬换行复制、同列跨行复制、跨列网格复制、跨块拖选复制、边框防手抖吸附、双击/三击选格等完整回归测试）
- **架构约束**：严格保持 `app/`（纯状态与更新）与 `ui/`（纯渲染与视图几何计算）边界清晰；不在 `ui/` 中修改状态或执行 IO；单生产文件严格控制在 1000 行以内，拆分后的子文件控制在 500 行左右；严禁 `unwrap()` / `expect()`。

---

## 0. 现状核实与事实基线（CodeGraph + Codebase Memory 实测）

| 维度 | 现状实测（带文件与行号） | 对本特性的设计影响 |
| :--- | :--- | :--- |
| **`lx-tui` 表格排版方式** | `src/ui/markdown_render/table.rs:14-74` `table()` 测量列宽并调用 `:177-264` `grid_table()`。<br>1. `row_lines`（`:189-227`）对每个单元格依次执行 `render_cell_spans`（`:283`）→ `split_br`（`:301`，解析 `<br>`/`<br/>`/`<br />`）→ `wrap_cell_spans`（`:364`，按列宽折行并经 `:414` `pad_line` 补齐空格到列宽）。<br>2. 将同一逻辑行各列对应的第 `row` 条物理行与左右 `│`、单元格左右 1 格空白字面拼接成同一条 `MdLine.spans`（`│ col0 │ col1 │ col2 │`）。<br>3. 表头上方画 `┌─┬─┐`、表头后画 `├─┼─┤`、数据行间（非续行）画 `├─┼─┤`、表尾画 `└─┴─┘`，样式均为 `style::md_read_rule()`。 | 表格在 `MdLine` 层已被压平为无结构的单行文本段，`│` 与单元格内容混在同一物理行中。必须在 `grid_table()` 构造 `MdLine` 时同步生成 `Arc<TableLayoutMeta>`，把列边界、真实文本区间（扣除 `pad_line` 填充空格）、`<br>` 硬换行与自动软折行标记保留下来。 |
| **`MdLine` 与 `wrap_md_line` 传递链** | `src/ui/markdown_render.rs:29-36` `MdLine` 仅有 `indent: u16`、`hang: u16`、`spans: Vec<(String, Style)>`。<br>`render_wrapped_with_options`（`:106-113`）与流式增量解析 `IncrementalMarkdownState::update`（`src/ui/markdown_render/incremental.rs:65-68, 82-85`）会对 `render_blocks` 产出的每条 `MdLine` 再调 `wrap_md_line`（`:669-740`）。 | 为 `MdLine` 增加 `pub table: Option<TableLineRef>`（默认 `None`）。当 `wrap_md_line` 处理未超宽的表格行（`first && break_at == cells.len()`）时原样透传 `line.table.clone()`，确保静态渲染与流式增量渲染均保留表格元数据。 |
| **所有 Markdown 块写入 `Grid` 的入口** | `lx` 下所有 Markdown 渲染最终只经由两条路径写入 `Grid`（`src/ui/main_content/lx/grid.rs:29-32`）：<br>1. 主回答正文 `draw_markdown`（`src/ui/main_content/lx/history/paint.rs:427-495`）与思考块 `draw_thinking`（`:498-601`）：在 `cx = rect.x + STEP_GUTTER/body_indent + line.indent` 处逐字符写 `Grid`；<br>2. 工具输出/汇报块/Compact/ModelSwitch/UserShell 统一走 `muted_markdown_lines` → `md_line_segments`（`src/ui/main_content/lx/history/tool.rs:174-201`）转为 `Vec<Segment>`，再经 `put_segments`（`:776-787`）写入 `Grid`。 | 只需在 `draw_markdown`、`draw_thinking` 与 `put_segments` 三处将 `TableLineRef` 连同当前行实际落点的水平列偏移 `base_col = cx - grid.area.x` 写入 `Grid` 的行级表槽 `table_rows[dy]`，即可一次性覆盖主对话正文、思考块、工具/汇报块与 `TaskTab::Agents` 子智能体详情页中的所有 Markdown 表格。 |
| **`lx-tui` 现有选区模型与复制缺陷根因** | 1. `src/app/selection.rs:135-142` `Selection` 存储 `origin`、`anchor`、`cursor`、`unit`。<br>2. `Selection::contains`（`:277-294`）与 `row_col_bounds`（`src/ui/main_content/lx/selection.rs:327-345`）均按纯屏幕行流矩形计算：首行选 `start_col..=max_col`、**中间行选 `0..=max_col`（整行全宽）**、末行选 `0..=end_col`。<br>3. 当用户在多行单元格内上下拖选时，首行右侧相邻列、中间行全部列及所有 `│` 竖线均被反显并原样拼进复制结果。 | 需要将表格行的选区判定从「屏幕整行流」升级为「表格单元格感知流」：`paint_selection`、`unit_cell_range`、`copy_text` 统一通过 `selection/table.rs` 解析表格选区模式与每个单元格的命中区间。 |
| **`opencode-dev` 参考实现机制** | 在 `@opentui/core` 的 `TextTableRenderable`（`index.node.js:8405-9150`）中：<br>1. `resolveSelectionResolution`（`:9053-9077`）将表格内选区划分为三级模式：<br>   - `single-cell`：`anchorCell` 与 `focusCell` 同行同列 → 仅在该单元格内按局部坐标选字，其余单元格清空选区；<br>   - `column-locked`：`anchorColumn == focusColumn` 且跨行 → 仅锁定在该列各行单元格，其余列清空选区；<br>   - `grid`：跨列 → 按起止单元格覆盖所跨行与列。<br>2. 单元格独立維護软折行视图，软折行不插入 `\n`，显式换行保留 `\n`。<br>3. `getSelectedText`（`:8451-8470`）按逻辑行遍历选中单元格：同行多列以 `"\t"` 拼接，跨逻辑行以 `"\n"` 拼接，所有表格边框与 padding 均不进入剪贴板。 | `lx-tui` 在保持单二进制 `ratatui` + `Grid` 架构的前提下，通过 `TableLayoutMeta` + `selection/table.rs` 1:1 复刻 `opencode-dev` 的三级锁定与去边框 TSV 复制语义，并额外增强单元格边缘/紧邻边框防手抖吸附与跨块拖选支持。 |

---

## 1. 已确认的边界决策汇总（Grill Me 结论表）

| 编号 | 决策维度 | 已确认方案 |
| :--- | :--- | :--- |
| **Q1** | **表格内长按拖选的三级锁定规则** | 完整对齐 `opencode-dev` 的三级单元格锁定模式：<br>1. **同单元格多行（`SingleCell`）**：起点 `anchor` 与终点 `cursor` 落在同一个单元格 `(row_idx, col_idx)` 内（含因列宽自动软折行或 `<br>` 拆出的多条物理行）时，高亮与复制严格限制在该单元格的列边界内部，首行到该格右边界为止、续行从该格左边界开始，绝不选中左右相邻列与 `│` 边框。<br>2. **同列跨行（`ColumnLocked`）**：起点与终点落在同一列 `col_idx` 的不同逻辑行时，选区锁定在该列，只高亮并复制该列在起止逻辑行内的单元格内容，跳过水平分割线 `├─┼─┤` 及左右其它列。<br>3. **跨列拖选（`Grid`）**：当终点横向跨入另一列时，按单元格阅读流选中起止范围内的各列单元格内容，但自动排除所有表格边框（`┌─┬─┐`、`├─┼─┤`、`└─┴─┘`、`│`）与单元格补齐空白。 |
| **Q2** | **复制文本的换行还原与多列/多行拼接格式** | 1. **单元格内自动软折行（Soft Wrap）**：因列宽限制自动折行的多条物理行，复制时无缝拼接还原为连续文本，**不插入多余的 `\n`**，并精确剥离 `pad_line` 为对齐列宽补充的前后空格（同时完整保留英文单词间的真实空格）。<br>2. **单元格内显式 `<br>` 换行（Hard Break）**：保留真实的 `\n` 换行符。<br>3. **多列与多行拼接**：同一逻辑行内选中的多个非空单元格用 `\t`（Tab）连接；不同逻辑行之间用 `\n` 连接；所有表格框线字符一律不进入剪贴板。 |
| **Q3** | **单元格留白/紧邻边框防手抖吸附与跨块拖选** | 1. **单元格留白与边框防手抖吸附**：单元格左右各有 1 格内边距留白，落在留白处直接归属该单元格；当起点在某单元格内，拖动终点不小心压到该单元格自身的上下水平边框线或左右竖线 `│`（未越过 `│` 进入相邻列内部区域，或拖到最左/最右列外侧空白）时，自动吸附回当前单元格/当前列，避免因手抖触碰边框而误跳入跨列整行模式。<br>2. **跨表格与外部普通正文一起拖选**：当选区一端或两端在表格外的普通段落时，外部普通段落按原行流复制，落入表格的行自动按表格 `Grid` 模式去边框提取（以 `\t` 与 `\n` 拼接单元格内容，不复制 `┌─┬─┐` / `├─┼─┤` / `│`）。 |
| **Q4** | **双击选词 / 三击选整格与生效视图范围** | 1. **双击（`SelectionUnit::Word`）**：在表格单元格内双击时，选词边界严格钳制在当前单元格的真实文本列区间 `[text_start, text_end)` 内，绝不把 `│` 边框或填充空格并入词选区；双击后继续拖动同样遵循三级单元格锁定。<br>2. **三击（`SelectionUnit::Line`）**：在表格某个单元格内三击左键时，直接选中**该单元格的全部内容**（跨该单元格的所有非空白填充物理行，水平钳在该单元格内容列区间内），使 `anchor` 与 `cursor` 均落在该单元格内并自然进入 `SingleCell` 模式。<br>3. **生效范围**：覆盖 `src/ui/main_content/lx/` 下所有经 `markdown_render` 渲染的 Markdown 表格（主聊天回答正文、思考块、工具/汇报/系统事件卡片展开内容、以及 `TaskTab::Agents` 子智能体对话历史）。 |

---

## 2. 核心架构与数据流

```text
┌──────────────────────────────────────────────────────────────────────────────┐
│ 1. Markdown 表格排版期（src/ui/markdown_render/table.rs）                    │
│                                                                              │
│  grid_table(header, body, widths, align, lines)                              │
│    ├─ 计算各列 TableColumnSpan { box_start, box_end, content_start, end }    │
│    ├─ 对每个单元格执行 render_cell_spans -> split_br -> wrap_cell_spans_meta │
│    │    └─ 产出每条物理行的 (spans, text_start, text_end, hard_break, filler)│
│    ├─ 汇总构建单份 Arc<TableLayoutMeta>（含 columns, logical_rows, lines）   │
│    └─ 为表格的每条 MdLine 挂载 Some(TableLineRef { table, line_offset })     │
└──────────────────────────────────────┬───────────────────────────────────────┘
                                       │
                                       ▼
┌──────────────────────────────────────────────────────────────────────────────┐
│ 2. 网格绘制期（paint.rs: draw_markdown / draw_thinking & tool.rs: put_segments）│
│                                                                              │
│  写入每条物理行时：                                                          │
│    ├─ 计算该行表格在屏幕上的实际起始列 cx                                    │
│    └─ 调用 grid.set_table_row(cx, y, &table_line_ref)                        │
│         └─ Grid.table_rows[y - area.y] = Some(GridTableRow {                 │
│                base_col: cx - area.x, meta: table_line_ref                   │
│            })                                                                │
└──────────────────────────────────────┬───────────────────────────────────────┘
                                       │
         ┌─────────────────────────────┼─────────────────────────────┐
         ▼                             ▼                             ▼
┌─────────────────────┐   ┌─────────────────────────┐   ┌──────────────────────┐
│ 3A. 双击 / 三击定位 │   │ 3B. 选区反显高亮        │   │ 3C. 剪贴板文本提取   │
│ unit_cell_range     │   │ paint_selection         │   │ copy_text            │
│                     │   │                         │   │                      │
│ • Word: 钳在当前格  │   │ • 识别视口内表格实例    │   │ • 普通行：原样提取   │
│   [text_start, end) │   │ • 解析 SingleCell /     │   │ • 表格段：按逻辑行与 │
│ • Line(三击): 返回  │   │   ColumnLocked / Grid   │   │   选中列提取真实文本 │
│   当前格首行至末非  │   │ • 仅对命中的单元格真实  │   │ • 软折行无缝拼接     │
│   填充行的闭区间    │   │   文本区间设置 REVERSED │   │ • <br> 保留 '\n'     │
│   (进入 SingleCell) │   │ • 边框与留白绝不反显    │   │ • 同行多列 '\t' 拼接 │
└─────────────────────┘   └─────────────────────────┘   └──────────────────────┘
```

---

## 3. 详细技术设计与关键代码

### 3.1 `Selection` 端点访问器扩展（`src/app/selection.rs` 与 `src/event/input.rs`）

#### 关键点

1. `Selection`（`src/app/selection.rs:135-142`）内部已持有 `anchor: (i32, u16)`（按下起点）与 `cursor: (i32, u16)`（当前拖动终点），但原先只暴露规范化后的 `range()`。
2. 新增 `pub fn anchor(&self) -> (i32, u16)` 与 `pub fn cursor(&self) -> (i32, u16)`，使 `selection/table.rs` 能够区分「按下起点 `anchor`」与「拖动终点 `cursor`」，从而在 `cursor` 触碰 `anchor` 所在单元格紧邻边框时精准向 `anchor` 所在单元格吸附（Q3）。
3. `src/event/input.rs:463-476` 中 `copy_lx_selection` 在确认 `selection.range().is_some()` 后，将 `(selection.anchor(), selection.cursor())` 传给 `ui::main_content::lx::copy_text`，保持函数签名完全兼容的同时保留拖选方向。

#### 关键代码（`src/app/selection.rs`）

```rust
impl Selection {
    /// 按下起点（内容行、列坐标）。
    pub fn anchor(&self) -> (i32, u16) {
        self.anchor
    }

    /// 当前拖动终点（内容行、列坐标）。
    pub fn cursor(&self) -> (i32, u16) {
        self.cursor
    }
}
```

---

### 3.2 Markdown 表格结构元数据定义与生成（`src/ui/markdown_render.rs`、`src/ui/markdown_render/table.rs`）

#### 关键点

1. 在 `src/ui/markdown_render.rs` 中定义并导出表格结构元数据类型：
   - `TableColumnSpan`：描述单列相对于表格左边缘（`x = 0`）的单元格内框区间 `box_start..box_end`（含左右各 1 格留白）与内容槽区间 `content_start..content_end`（扣除左右 1 格留白）。
   - `TableCellSlice`：描述某条物理行上单个单元格的真实文本列区间 `text_start..text_end`（扣除 `pad_line` 因左/中/右对齐补齐的前后空格）、`hard_break: bool`（该物理行后是否紧跟显式 `<br>` 换行）、`is_filler: bool`（是否为同逻辑行其它更高列撑高产生的空白填充行）。
   - `TableLogicalRow`：记录每个逻辑行（`0` 为表头，`1..` 为数据行）在表格内的物理行起止偏移 `first_line..=last_line`。
   - `TableLineKind`：区分水平边框行 `Border { nearest_row }` 与内容行 `Cells { row_idx, line_in_row, cells }`。
   - `TableLayoutMeta` 与 `TableLineRef`：同一张表的所有 `MdLine` 共享同一个 `Arc<TableLayoutMeta>`，每条 `MdLine` 携带自身在表内的 `line_offset: usize`。
2. 在 `src/ui/markdown_render.rs` 的 `wrap_md_line` 中：当 `line.table.is_some()` 且整行未发生二次硬切折行（`first && break_at == cells.len()`）时，将 `line.table.clone()` 赋给输出的 `MdLine`，确保 `render_wrapped`、`render_wrapped_literal_rules` 与 `IncrementalMarkdownState::update` 均无损保留表格元数据。
3. 在 `src/ui/markdown_render/table.rs` 的 `grid_table` 中：
   - `pad_line_meta` 在补齐空格的同时返回 `(spans, pad_before, text_width)`；
   - ` split_br` 拆出的每个逻辑子行 `logical[seg_idx]` 在经 `wrap_cell_spans_meta` 折行后，仅将该子行的**最后一条物理行**（当 `seg_idx + 1 < logical.len()` 时）标记为 `hard_break = true`，其余软折行物理行恒为 `hard_break = false`；
   - 当某列在当前逻辑行的物理行数少于该逻辑行最大高度 `height` 时，补齐的空白行标记为 `is_filler = true`。

#### 关键代码（`src/ui/markdown_render.rs` 与 `src/ui/markdown_render/table.rs`）

```rust
// src/ui/markdown_render.rs
use std::sync::Arc;

/// 表格单列的水平区间（列偏移均相对表格左边框 `x = 0`）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TableColumnSpan {
    /// 单元格内框起始列（左 `│` 右侧第 1 列，含左 1 格留白）。
    pub box_start: u16,
    /// 单元格内框结束列（开区间，右 `│` 所在列，含右 1 格留白）。
    pub box_end: u16,
    /// 单元格内容槽起始列（扣除左 1 格留白）。
    pub content_start: u16,
    /// 单元格内容槽结束列（开区间，扣除右 1 格留白）。
    pub content_end: u16,
}

/// 单个单元格在某条物理行上的真实文本切片（列偏移均相对表格左边框 `x = 0`）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TableCellSlice {
    /// 真实文本起始列（扣除居中/右对齐的前导填充空格）。
    pub text_start: u16,
    /// 真实文本结束列（开区间，扣除左对齐/居中的尾随填充空格）。
    pub text_end: u16,
    /// 当前物理行结束后是否为显式 `<br>` 硬换行（`false` 表示自动软折行或本单元格末行）。
    pub hard_break: bool,
    /// 当前物理行是否为同逻辑行其它列撑高产生的空白填充行。
    pub is_filler: bool,
}

/// 表格单个逻辑行（`0` 为表头，`1..` 为数据行）在表格内的物理行偏移区间。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TableLogicalRow {
    /// 该逻辑行首条物理行在表格内的行偏移（相对顶边框 `0`）。
    pub first_line: usize,
    /// 该逻辑行末条物理行在表格内的行偏移（闭区间）。
    pub last_line: usize,
}

/// 表格单条物理行的结构类型。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TableLineKind {
    /// 水平边框行（`┌─┬─┐`、`├─┼─┤`、`└─┴─┘`）；`nearest_row` 为紧邻的逻辑行下标。
    Border { nearest_row: usize },
    /// 表头或数据单元格行。
    Cells {
        /// 逻辑行下标（`0` 为表头，`1..` 为数据行）。
        row_idx: usize,
        /// 该逻辑行内的物理行偏移（`0..height`）。
        line_in_row: usize,
        /// 本条物理行上各列的文本切片（与 `columns` 等长）。
        cells: Vec<TableCellSlice>,
    },
}

/// 整张 Markdown 表格的排版与几何元数据（同一张表的所有物理行共享同一个 `Arc`）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TableLayoutMeta {
    /// 表格总显示列宽（含首尾 `│`）。
    pub total_width: u16,
    /// 各列水平区间（长度等于列数）。
    pub columns: Vec<TableColumnSpan>,
    /// 各逻辑行的物理行起止偏移（下标即 `row_idx`）。
    pub logical_rows: Vec<TableLogicalRow>,
    /// 表格内每条物理行的类型与单元格切片（下标即 `line_offset`）。
    pub lines: Vec<TableLineKind>,
}

/// 附加在 [`MdLine`] 上的表格行引用。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TableLineRef {
    /// 所属表格的完整几何元数据。
    pub table: Arc<TableLayoutMeta>,
    /// 当前物理行在表格内的行偏移（`0..table.lines.len()`）。
    pub line_offset: usize,
}

#[derive(Debug, Clone, PartialEq)]
pub struct MdLine {
    pub indent: u16,
    pub hang: u16,
    pub spans: Vec<(String, Style)>,
    /// 若该行属于 Markdown 表格，记录其表格结构与行内偏移。
    pub table: Option<TableLineRef>,
}
```

```rust
// src/ui/markdown_render/table.rs
struct WrappedCellLine {
    spans: Vec<(String, Style)>,
    pad_before: u16,
    text_width: u16,
    hard_break: bool,
}

fn grid_table(
    header: &[String],
    body: &[Vec<String>],
    widths: &[usize],
    align: &[Align],
    lines: &mut Vec<MdLine>,
) {
    // 1. 计算每列在表格行内的水平区间（左 `│` 在 0，每列格式为 `│ <width> `）
    let mut columns = Vec::with_capacity(widths.len());
    let mut cursor = 1u16; // 跳过最左侧 `│`（占 1 列）
    for &w in widths {
        let w_u16 = u16::try_from(w).unwrap_or(u16::MAX);
        let box_start = cursor;
        let content_start = cursor.saturating_add(1);
        let content_end = content_start.saturating_add(w_u16);
        let box_end = content_end.saturating_add(1);
        columns.push(TableColumnSpan {
            box_start,
            box_end,
            content_start,
            content_end,
        });
        cursor = box_end.saturating_add(1); // 跳过列间/行尾 `│`
    }
    let total_width = cursor;

    // 2. 生成所有物理行 spans 与对应的 TableLineKind、TableLogicalRow
    let mut raw_lines: Vec<Vec<(String, Style)>> = Vec::new();
    let mut line_kinds: Vec<TableLineKind> = Vec::new();
    let mut logical_rows: Vec<TableLogicalRow> = Vec::with_capacity(1 + body.len());

    // 顶边框 `┌─┬─┐`
    raw_lines.push(vec![(border("┌", "─", "┬", "┐"), style::md_read_rule())]);
    line_kinds.push(TableLineKind::Border { nearest_row: 0 });

    // 表头（row_idx = 0）与数据行（row_idx = 1..=body.len()）统一生成
    // ...
    // 3. 封装为单一 Arc<TableLayoutMeta>，按 line_offset 写入每条 MdLine
    let table_meta = Arc::new(TableLayoutMeta {
        total_width,
        columns,
        logical_rows,
        lines: line_kinds,
    });
    for (line_offset, spans) in raw_lines.into_iter().enumerate() {
        lines.push(MdLine::with_table(
            0,
            spans,
            TableLineRef {
                table: Arc::clone(&table_meta),
                line_offset,
            },
        ));
    }
}
```

---

### 3.3 `Grid` 行级表格几何挂载（`src/ui/main_content/lx/grid.rs`、`paint.rs`、`tool.rs`）

#### 关键点

1. `Grid`（`src/ui/main_content/lx/grid.rs`）新增 `table_rows: Vec<Option<GridTableRow>>`，长度等于 `usize::from(area.height)`。
2. `GridTableRow` 记录：
   - `base_col: u16`：表格左边缘相对于 `Grid.area.x` 的列偏移（例如在主回答正文中为 `STEP_GUTTER = 2`，在带树形连接符的工具/汇报块中为 `5`）；
   - `line_ref: TableLineRef`：该物理行对应的 `Arc<TableLayoutMeta>` 与 `line_offset`。
3. 在 `draw_markdown`、`draw_thinking`（`src/ui/main_content/lx/history/paint.rs`）中，当 `line.table` 为 `Some(ref table_ref)` 时，调用 `grid.set_table_row(cx, y, table_ref.clone())`。
4. 在 `Segment`（`src/ui/main_content/lx/history/tool.rs`）中增加 `pub(crate) table: Option<TableLineRef>`（仅在 `md_line_segments` 转换表格行时挂在首个文本段上，`wrap_spans` 单行不折断时保留），`put_segments` 写入该段时调用 `grid.set_table_row(cx, y, table_ref.clone())`。

#### 关键代码（`src/ui/main_content/lx/grid.rs`）

```rust
use crate::ui::markdown_render::TableLineRef;

/// 网格中单行的表格几何（列偏移相对 `Grid.area.x`）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct GridTableRow {
    /// 表格左边框在内容网格中的起始列（相对 `area.x`）。
    pub(crate) base_col: u16,
    /// 表格结构元数据与行内偏移。
    pub(crate) line_ref: TableLineRef,
}

pub(crate) struct Grid {
    area: Rect,
    cells: Vec<Glyph>,
    table_rows: Vec<Option<GridTableRow>>,
}

impl Grid {
    pub(super) fn new(area: Rect) -> Self {
        Self {
            area,
            cells: vec![Glyph::blank(); usize::from(area.width) * usize::from(area.height)],
            table_rows: vec![None; usize::from(area.height)],
        }
    }

    /// 登记第 `y` 行的表格结构元数据（`x` 为表格左边框的绝对屏幕列坐标）。
    pub(super) fn set_table_row(&mut self, x: u16, y: u16, line_ref: TableLineRef) {
        if x < self.area.x || y < self.area.y {
            return;
        }
        let dy = y - self.area.y;
        if dy < self.area.height {
            self.table_rows[usize::from(dy)] = Some(GridTableRow {
                base_col: x - self.area.x,
                line_ref,
            });
        }
    }

    /// 读取相对行 `dy`（`0..area.height`）的表格行元数据。
    pub(super) fn table_row_at(&self, dy: u16) -> Option<&GridTableRow> {
        self.table_rows.get(usize::from(dy)).and_then(Option::as_ref)
    }
}
```

---

### 3.4 表格三级选区解析、防手抖吸附、高亮与复制（`src/ui/main_content/lx/selection/table.rs`）

#### 关键点

1. **表格实例定位与可见段截断边界（`rendered_table_row_bounds`）**：
   - 给定任意一条携带 `GridTableRow` 的内容行 `content_row`，由 `line_ref.line_offset` 可直接推算该表格理论起点 `table_start_row = content_row - line_offset as i32` 与理论终点 `table_end_row = table_start_row + table.lines.len() as i32 - 1`。
   - 对折叠汇报卡片（`REPORT_PREVIEW_ROWS = 2`）、系统事件卡片或命令 `Preview::head_tail` 等仅渲染部分表格物理行的场景，通过 `rendered_table_row_bounds` 在当前 `Grid` 视口内上下扫描同一表格实例的连续边界 `[actual_start_row, actual_end_row]`，将紧邻的非表格行严格排除在表格选区之外，防止在表格下方普通行拖选时误反显上方表格行。
2. **单元格留白、水平外侧过滤与紧邻边框防手抖吸附（`resolve_table_selection`，对应 Q1 & Q3）**：
   - **水平外侧过滤**：若起止点行号在表格内但列坐标同时位于表格左边界外（`< base_col`）或同时位于右边界外（`>= base_col + table.total_width`），返回 `None`。
   - **起点 `anchor` 映射**：
     - 行映射：若 `anchor` 落在 `TableLineKind::Cells { row_idx, .. }` 则取 `row_idx`；若落在 `Border { nearest_row }` 则取 `nearest_row`。
     - 列映射：若 `anchor.col` 落在某列 `box_start..box_end`（含左右 1 格留白）则直接取该列；若恰好压在左/中/右竖线 `│` 上则吸附到紧邻列。
   - **终点 `cursor` 防手抖吸附**：
     - 水平方向：设起点所在列为 `anchor_col`，其内框加左右自身竖线 `│` 的吸附区间为 `[base_col + box_start - 1, base_col + box_end]`（若 `anchor_col == 0` 则左扩到 `0`；若 `anchor_col` 为末列则右扩到 `u16::MAX`）。只要 `cursor.col` 仍在该区间内，`focus_col` 恒锁定为 `anchor_col`。
     - 垂直方向：设起点所在逻辑行 `anchor_row` 的物理行区间为 `[first_line, last_line]`。若 `cursor.row` 压在该逻辑行紧邻的上下水平分割线（`first_line - 1` 或 `last_line + 1`）上，则 `focus_row` 仍吸附为 `anchor_row`；若向其它逻辑行拖动时停在某条中间分割线 `├─┼─┤` 上，则归属到拖动方向上已滑过的逻辑行。
3. **三级模式判定与单元格列区间裁剪（`TableSelectionPlan`，对应 Q1）**：
   - **`SingleCell { row_idx, col_idx }`**：当 `anchor_row == focus_row && anchor_col == focus_col` 时触发（若 `anchor == cursor` 且落在无文本的水平边框行、填充行或空单元格上则返回 `None`）。
     - 将起止点钳入该单元格的非空白填充物理行区间 `[first_line..=last_non_filler]` 与内容槽列区间 `[content_col0, content_col1]`；
     - 在首行选 `[start_col, content_col1]`、中间物理行选 `[content_col0, content_col1]`、末行选 `[content_col0, end_col]`，并与每条物理行的真实文本区间 `[text_col0, text_col1]` 求交集。
   - **`ColumnLocked { col_idx, first_row_idx, last_row_idx }`**：当 `anchor_col == focus_col && anchor_row != focus_row` 时触发，锁定 `col_idx` 列并整格选中 `first_row_idx..=last_row_idx` 各逻辑行的真实文本区间。
   - **`Grid { first_row_idx, last_row_idx, first_col_idx, last_col_idx }`**：跨列或从表格外部跨入表格时触发，整格选中矩形区间内各单元格的真实文本区间。
   - **CJK 双宽字符边界对齐（`align_wide_char_cols`）**：渲染反显与文本提取时，若选中列起点落在 `CONTINUATION` 右半格则自动左扩 1 列至主格，若终点落在双宽字符主格则自动右扩 1 列包含右半格。
4. **双击/三击选区与可折叠卡片穿透（`table_unit_cell_range` & `is_table_cell_at`，对应 Q4）**：
   - `SelectionUnit::Word`：当点击点落在表格某单元格内时，将分词列边界严格钳在该单元格当行的真实文本列 `[text_col0, text_col1]` 内。
   - `SelectionUnit::Line`（三击）：返回该单元格首条物理行内容起点到最后一条非填充物理行内容终点（并按 `rendered_table_row_bounds` 钳制在可见表格行内）。
   - 在 `src/event/mouse/press.rs` 中通过 `lx::is_table_cell_at` 识别已展开思考块、汇报块与工具卡片内的 Markdown 表格点击，优先执行连击选词/选整格而不触发卡片折叠。
5. **去边框、连续 `<br>` 保空行与 TSV 列对齐复制（`extract_table_lines`，对应 Q2 & Q3）**：
   - 按逻辑行 `row_idx` 遍历 `first_row_idx..=last_row_idx`，在每个逻辑行内遍历 `first_col_idx..=last_col_idx`：
     - 对选中范围内的非填充物理行（含连续 `<br><br>` 产生的空物理行），当上一条物理行 `prev_hard_break == true` 时裁剪行尾空格并压入 `'\n'`；当 `prev_hard_break == false`（自动软折行）时直接无缝拼接；
     - `ColumnLocked` 与 `Grid` 模式下保留空单元格 `""` 占位并以 `"\t"` 连接同一逻辑行的各列，确保 TSV 列对齐不左移；`SingleCell` 模式下仅输出非空单元格文本。

---

### 3.5 `paint_selection`、`is_table_cell_at`、`unit_cell_range` 与 `copy_text` 接入（`src/ui/main_content/lx/selection.rs`）

#### 关键点

1. `paint_selection(buf: &mut Buffer, grid: &Grid, geom: ContentGeom, selection: &Selection)`：
   - 对视口内每一行 `dy in 0..geom.rect.height`，检查 `grid.table_row_at(grid_dy)`；
   - 若该行属于 Markdown 表格，先通过 `table::rendered_table_row_bounds` 计算可见连续表格上下界，再调用 `table::resolve_table_selection` 得到 `TableSelectionPlan`，通过 `plan.selected_col_ranges(line_offset)` 与 `align_wide_char_cols` 对齐双宽字符后反显目标区间；
   - 若该行不属于表格，则按 `row_col_bounds` + `align_wide_char_cols` 反显普通正文行。
2. `is_table_cell_at` & `unit_cell_range`：
   - `is_table_cell_at` 判断点击坐标是否落在已渲染表格行的 `[base_col, base_col + table.total_width)` 内，供 `event/mouse/press.rs` 放行卡片内表格的双击/三击选区；
   - `unit_cell_range` 在表格行上委托 `table::table_unit_cell_range`，并在 `SelectionUnit::Line` 三击时结合 `rendered_table_row_bounds` 钳制可见行范围。
3. `copy_text`：
   - 遍历遇到 `GridTableRow` 时，通过 `plan.selected_line_offset_range()` 仅渲染选中逻辑行对应的物理行区间 `[t_first, t_last]`（避免大表全表分配渲染），使用 `same_table_meta`（`Arc::ptr_eq` O(1) 短路 + 值比较兜底）收集有序 `present_offsets`，调用 `table::extract_table_lines` 提取去边框 TSV 文本。

---

## 4. 涉及文件与预估行数表

| 文件路径 | 操作 | 当前行数 | 预估改后行数 | 说明（生产代码均严格 ≤ 1000 行） |
| :--- | :--- | :---: | :---: | :--- |
| `src/app/selection.rs` | 修改 | 298 | ~312 | 新增 `Selection::anchor()` 与 `Selection::cursor()` 访问器 |
| `src/event/input.rs` | 修改 | 548 | ~550 | `copy_lx_selection` 传递 `(selection.anchor(), selection.cursor())` |
| `src/ui/markdown_render.rs` | 修改 | 766 | ~845 | 定义 `TableColumnSpan`、`TableCellSlice`、`TableLogicalRow`、`TableLineKind`、`TableLayoutMeta`、`TableLineRef`，并在 `MdLine` 与 `wrap_md_line` 中透传 |
| `src/ui/markdown_render/table.rs` | 修改 | 435 | ~520 | `grid_table`、`wrap_cell_spans`、`pad_line` 产出精确的列区间、真实文本区间、软折行与 `<br>` 标记 |
| `src/ui/main_content/lx/grid.rs` | 修改 | 130 | ~175 | 新增 `GridTableRow` 与 `set_table_row` / `table_row_at` |
| `src/ui/main_content/lx/history/paint.rs` | 修改 | 704 | ~720 | `draw_markdown` 与 `draw_thinking` 将表格行元数据登记到 `Grid` |
| `src/ui/main_content/lx/history/tool.rs` | 修改 | 832 | ~855 | `Segment` 与 `md_line_segments` / `put_segments` 透传并登记表格行元数据 |
| `src/ui/main_content/lx.rs` | 修改 | 328 | ~330 | `render` 调用 `selection::paint_selection` 时传入 `&grid` |
| `src/ui/main_content/lx/selection.rs` | 拆分修改 | 345 | ~360 | 声明 `mod table;`（及 `#[cfg(test)] mod tests;`），在 `paint_selection`、`unit_cell_range`、`copy_text` 中接入表格选区分支 |
| `src/ui/main_content/lx/selection/table.rs` | **新建** | 0 | ~360 | 独立承载表格三级选区解析、边框防手抖吸附、双击/三击单元格边界与去边框 TSV 复制逻辑 |
| `src/ui/markdown_render/tests.rs` | 修改 | 695 | ~760 | 新增表格 `TableLayoutMeta` 列区间、软折行 vs `<br>` 切片元数据单测 |
| `src/ui/main_content/lx/selection/tests.rs` | **新建** | 0 | ~380 | 独立承载 Markdown 表格选区高亮、单格多行复制、同列跨行复制、跨列网格复制、边框吸附、跨块复制、双击/三击测试 |

---

## 5. 测试矩阵与风险控制

### 5.1 单元测试与集成测试矩阵

1. **单单元格内多行自动软折行拖选与复制（`src/ui/main_content/lx/selection/tests.rs`）**：
   - **场景**：渲染窄宽度下的多列表格，某单元格包含长中文句子或英文短语（如 `"cargo test --all-targets"`）并自动折成 2~3 条物理行；从该单元格第 1 行拖选到第 2/3 行。
   - **期望断言**：
     - 选区模式为 `SingleCell`；
     - `paint_selection` 仅反显该单元格内部的真实文本列，同行左侧/右侧其它列与 `│` 竖线、1 格留白、末尾填充空格均无 `Modifier::REVERSED`；
     - `copy_text` 返回无缝拼接的单行文本 `"cargo test --all-targets"`，不含 `\n`、不含 `│`、不含其它列文字。
2. **单单元格内显式 `<br>` 多行拖选与复制（`src/ui/main_content/lx/selection/tests.rs`）**：
   - **场景**：单元格源码含 `"第一行<br>第二行<br/>第三行"`，在格内从第 1 行拖选到第 3 行。
   - **期望断言**：`copy_text` 返回 `"第一行\n第二行\n第三行"`，保留 `<br>` 显式换行且不带任何相邻列或边框字符。
3. **同一列上下跨行拖选（`ColumnLocked` 模式，`src/ui/main_content/lx/selection/tests.rs`）**：
   - **场景**：从第 1 行第 2 列垂直向下拖选到第 3 行第 2 列（跨过中间的 `├─┼─┤` 分割线）。
   - **期望断言**：仅第 2 列的单元格被反显和提取，复制结果为 `"格(1,2)\n格(2,2)\n格(3,2)"`，第 1 列、第 3 列及 `├─┼─┤` 均不被选中。
4. **跨列拖选与整表复制（`Grid` 模式，`src/ui/main_content/lx/selection/tests.rs`）**：
   - **场景**：从表头第 1 列拖选到末行末列（其中部分单元格含软折行多行内容）。
   - **期望断言**：同一逻辑行的多列以 `\t` 连接，不同逻辑行以 `\n` 连接，多行单元格在所属逻辑行内无缝合并，不拆碎逻辑行，也不含任何 `┌─┬─┐` / `├─┼─┤` / `└─┴─┘` / `│` 框线。
5. **单元格留白与紧邻边框防手抖吸附（`src/ui/main_content/lx/selection/tests.rs`）**：
   - **场景**：起点在单元格 `(1, 1)` 内容区，终点拖到该单元格右侧紧邻的 `│` 竖线或下方紧邻的 `├─┼─┤` 分割线上；以及起点在最右列、终点拖到表格右侧外部空白处。
   - **期望断言**：仍保持在 `SingleCell` 模式，仅复制单元格 `(1, 1)`（或最右列该格）的内容，不跳变为整行跨列选中。
6. **跨表格与外部普通正文联合拖选（`src/ui/main_content/lx/selection/tests.rs`）**：
   - **场景**：消息包含「前置段落 + Markdown 表格 + 后置段落」，从前置段落首行一直拖选到后置段落末行。
   - **期望断言**：复制结果中前置段落与后置段落保持原文本，中间表格段自动去边框并以 `\t` / `\n` 格式化输出。
7. **表格单元格内双击选词与三击选整格（`src/ui/main_content/lx/selection/tests.rs`）**：
   - **场景**：在紧贴 `│` 的单元格首词上双击（`SelectionUnit::Word`）；在折成 3 行的单元格内任意位置三击（`SelectionUnit::Line`）。
   - **期望断言**：双击只选中该词（不含 `│`）；三击精准选中该单元格全部 3 行内容（不含相邻列与边框）。
8. **思考块、工具卡片输出与 `Agents` 子智能体详情页中的表格复制（`src/ui/main_content/lx/selection/tests.rs`）**：
   - **场景**：在展开的思考块、带 `└─` 树前缀的汇报/工具块以及 `TaskTab::Agents` 详情页中对 Markdown 表格执行单格多行拖选复制。
   - **期望断言**：左起偏移 `base_col` 准确包含 `STEP_GUTTER` 与树形前缀宽度，单格多行复制同样只输出格内干净文本。

### 5.2 风险与回滚

- **对现有非表格选区的影响**：非表格行的 `MdLine.table` 与 `Grid.table_rows[dy]` 恒为 `None`，`paint_selection`、`unit_cell_range` 与 `copy_text` 对 `None` 行 100% 走原有代码路径，现有全部选区测试零行为变更。
- **数据库与持久化影响**：纯内存 UI 渲染与选区计算改动，不涉及 SQLite `SCHEMA_VERSION`、配置文件或模型提示词，可随时无损回滚。
