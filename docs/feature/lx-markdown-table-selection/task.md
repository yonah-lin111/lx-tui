# `lx` 消息列表 Markdown 表格单元格长按拖选、高亮与去边框复制 — 执行任务清单 (`task.md`)

- **状态**：已完成
- **设计依据**：`docs/feature/lx-markdown-table-selection/design.md`
- **参考代码库**：`/Users/yonah/projects/agent/opencode-dev`（`@opentui/core` `TextTableRenderable`）

---

## 0. 开工门禁与硬性规范

- **门禁**：必须等用户审核并明确确认 `design.md` 与 `task.md` 后，才可修改 `src/` 与 `tests/` 下的代码。
- **架构红线**：
  1. 严格保持 `app/`（纯状态与更新）与 `ui/`（纯渲染与视图几何计算）边界清晰，禁止在渲染中修改状态或执行 IO；
  2. 所有生产代码文件严格 ≤ 1000 行，拆分后的子模块控制在 500 行左右；测试文件严格跟随被测模块目录层级（`<module>/<submodule>/tests.rs`），单测试文件 ≤ 1500 行；
  3. 生产代码严禁出现 `unwrap()` / `expect()`；
  4. 未经用户明确授权，严禁执行任何修改 Git 状态的命令。

---

## 1. 需求与边界确认清单（Grill Me 已确认）

- [x] **Q1（表格内长按拖选的三级锁定规则）**：对齐 `opencode-dev` 的三级模式——同格多行（`SingleCell`）严格锁定在当前单元格列宽内；同列跨行（`ColumnLocked`）严格锁定在当前列各行单元格内；跨列（`Grid`）按起止单元格选中多列内容并剔除所有边框线与补齐空白。
- [x] **Q2（复制文本的换行还原与多列/多行拼接格式）**：单元格内因列宽限制自动产生的软折行无缝拼接还原（不加 `\n`，剥离对齐填充空格并保留真实词间空格）；显式 `<br>` 换行保留 `\n`；同一逻辑行多列用 `\t` 分隔，不同逻辑行用 `\n` 分隔，绝不复制 `┌─┬─┐` / `├─┼─┤` / `└─┴─┘` / `│`。
- [x] **Q3（单元格留白/紧邻边框防手抖吸附与跨块拖选）**：单元格内左右 1 格留白归属该单元格；从单元格内拖选压到该单元格自身的上下横线或左右竖线 `│`（未越过竖线进入相邻列内容区，或拖至最左/最右列外侧空白）时自动吸附回当前单元格/当前列；跨出表格与外部普通段落一起拖选时，外部段落原样复制，表格部分自动按去边框格式提取。
- [x] **Q4（双击选词 / 三击选整格与生效视图范围）**：表格内双击选词钳在当前单元格真实文本区间内；三击直接选中当前单元格的全部多行内容；生效范围覆盖 `src/ui/main_content/lx/` 下主聊天正文、思考块、工具/汇报/系统事件卡片与 `TaskTab::Agents` 子智能体对话页中的所有 Markdown 表格。

---

## 2. 分阶段实施步骤拆解

### Phase 1：选区端点访问器与 Markdown 表格排版元数据（`src/app/selection.rs`、`src/ui/markdown_render*`）

> 目标：让 `Selection` 暴露拖选起点/终点方向，并让 `markdown_render` 在排版表格时为每条 `MdLine` 挂上共享的 `Arc<TableLayoutMeta>` 与 `line_offset`。

- [x] **1.1 补充 `Selection` 起点/终点访问器**
  - 在 **`src/app/selection.rs`** 中为 `Selection` 新增 `pub fn anchor(&self) -> (i32, u16)` 与 `pub fn cursor(&self) -> (i32, u16)`。
- [x] **1.2 定义表格布局元数据结构并接入 `MdLine`**
  - 在 **`src/ui/markdown_render.rs`** 中定义并导出 `TableColumnSpan`、`TableCellSlice`、`TableLogicalRow`、`TableLineKind`、`TableLayoutMeta`、`TableLineRef`。
  - 为 `MdLine` 新增 `pub table: Option<TableLineRef>` 字段及 `MdLine::with_table` 构造器；在 `MdLine::plain`、`MdLine::spans`、`MdLine::hanging` 中默认置为 `None`。
  - 在 `wrap_md_line` 中：当输入行携带 `line.table.is_some()` 且整行未发生二次截断折行时，将 `line.table.clone()` 保留在输出的 `MdLine` 上。
- [x] **1.3 在 `grid_table` 中生成完整 `TableLayoutMeta`**
  - 在 **`src/ui/markdown_render/table.rs`** 中扩展 `wrap_cell_spans` 与 `pad_line`，记录每条物理行的前导填充列数 `pad_before`、真实文本列宽 `text_width`、以及是否由 `<br>` 触发的 `hard_break` 标记，和空白高度填充行 `is_filler` 标记。
  - 在 `grid_table` 中汇总生成 `Arc<TableLayoutMeta>`，并为产出的每条表格 `MdLine`（含 `┌─┬─┐`、`├─┼─┤`、`└─┴─┘` 与单元格行）挂载 `TableLineRef { table, line_offset }`。

### Phase 2：`Grid` 行级表格几何登记（`src/ui/main_content/lx/grid.rs`、`paint.rs`、`tool.rs`）

> 目标：在所有 Markdown 渲染落入 `Grid` 的入口处登记 `GridTableRow { base_col, line_ref }`。

- [x] **2.1 扩展 `Grid` 存储行级表格几何**
  - 在 **`src/ui/main_content/lx/grid.rs`** 中新增 `GridTableRow { pub(crate) base_col: u16, pub(crate) line_ref: TableLineRef }` 与 `table_rows: Vec<Option<GridTableRow>>`。
  - 新增 `Grid::set_table_row(&mut self, x: u16, y: u16, line_ref: TableLineRef)` 与 `Grid::table_row_at(&self, dy: u16) -> Option<&GridTableRow>`。
- [x] **2.2 在正文、思考块与工具/汇报块绘制时登记表格行**
  - 在 **`src/ui/main_content/lx/history/paint.rs`** 的 `draw_markdown` 与 `draw_thinking` 中，当 `line.table` 为 `Some(ref line_ref)` 时，以实际起始列 `cx` 调用 `grid.set_table_row(cx, y, line_ref.clone())`。
  - 在 **`src/ui/main_content/lx/history/tool.rs`** 的 `Segment` 中增加可选 `pub(crate) table: Option<TableLineRef>`，`md_line_segments` 将 `line.table` 挂在首个 `Segment` 上，`wrap_spans` 在单行不折断时保留该标记，`put_segments` 绘制时调用 `grid.set_table_row(cx, y, line_ref.clone())`。

### Phase 3：表格三级选区解析、局部高亮、双击/三击与去边框复制（`src/ui/main_content/lx/selection*`、`lx.rs`、`input.rs`）

> 目标：实现 `selection/table.rs` 核心算法，并闭环接入 `paint_selection`、`unit_cell_range`、`copy_text`。

- [x] **3.1 新建 `src/ui/main_content/lx/selection/table.rs`**
  - 实现表格实例探测与起止点映射（含单元格左右 1 格留白归属、紧邻上下横线与左右 `│` 竖线防手抖吸附、最左/最右列外侧拖拽钳制）。
  - 实现三级模式判定（`SingleCell`、`ColumnLocked`、`Grid`）与 `TableSelectionPlan::selected_cell_cols` / `contains_col`。
  - 实现表格内双击选词（钳制在单元格 `[text_start, text_end)` 内）与三击选整格（返回该单元格首条物理行至末条非填充物理行的闭区间）的 `table_unit_cell_range`。
  - 实现 `extract_table_lines`：按逻辑行与列提取选中单元格文本，软折行无缝拼接、`<br>` 保留 `\n`、同行多列以 `\t` 连接、跨行以 `\n` 连接。
- [x] **3.2 接入 `selection.rs`、`lx.rs` 与 `event/input.rs`**
  - 在 **`src/ui/main_content/lx/selection.rs`** 中将 `paint_selection` 签名更新为接收 `&Grid`，对表格行按 `TableSelectionPlan::contains_col` 局部反显；在 `unit_cell_range` 与 `copy_text` 中接入 `selection::table` 分支。
  - 在 **`src/ui/main_content/lx.rs`** 的 `render` 中向 `selection::paint_selection` 传入 `&grid`。
  - 在 **`src/event/input.rs`** 的 `copy_lx_selection` 中向 `ui::main_content::lx::copy_text` 传入 `(selection.anchor(), selection.cursor())`。

### Phase 4：单元测试与回归测试补齐（`src/ui/markdown_render/tests.rs`、`src/ui/main_content/lx/selection/tests.rs`）

> 目标：覆盖 Q1–Q4 全部场景与边界条件，确保精确验证通过。

- [x] **4.1 补齐 `markdown_render` 表格元数据单测**
  - 在 **`src/ui/markdown_render/tests.rs`** 中验证 `render_wrapped` / `IncrementalMarkdownState` 产出的表格 `MdLine` 携带正确的 `TableLayoutMeta`（列区间、真实文本区间、软折行 `hard_break = false` 与 `<br>` `hard_break = true`、`is_filler` 标记）。
- [x] **4.2 新建 `src/ui/main_content/lx/selection/tests.rs` 覆盖全部表格选区场景**
  - 测试 1：单格多行软折行拖选（英文词间空格保留、中文无缝拼接、相邻列与 `│` 绝不被反显或复制）。
  - 测试 2：单格多行 `<br>` 显式换行拖选（保留 `\n`）。
  - 测试 3：同列跨多行拖选（`ColumnLocked`，跳过 `├─┼─┤` 与其它列）。
  - 测试 4：跨列拖选与整表复制（`Grid`，同行 `\t`、跨行 `\n`，多行单元格不拆行）。
  - 测试 5：单元格留白与紧邻 `│` / `├─┼─┤` 边框防手抖吸附。
  - 测试 6：跨表格与外部普通段落一起拖选复制。
  - 测试 7：单元格内双击选词与三击选整格（含多行单元格三击整格复制）。
  - 测试 8：思考块、工具/汇报块与 `TaskTab::Agents` 子智能体页中的表格单格多行拖选复制。

---

## 3. 验证命令

```bash
rtk cargo fmt --check
rtk cargo clippy --all-targets -- -D warnings
rtk cargo test markdown_render
rtk cargo test main_content::lx
```

---

## 4. 完成定义（DoD）

- [x] 在 `lx` 消息列表（含主回答正文、思考块、工具/汇报展开内容、子智能体详情页）的 Markdown 表格中，长按拖选单个多行单元格时，仅反显并复制该单元格内部文本，绝不出现整行连带相邻列与 `│` 边框一起被选中或复制的情况。
- [x] 单元格内自动软折行复制时无缝还原为连续文本，显式 `<br>` 保留 `\n`；同列跨行锁定在单列并用 `\n` 连接；跨列多格用 `\t` 与 `\n` 连接且无任何框线字符。
- [x] 单元格留白与紧邻边框具备防手抖吸附；单元格内双击选词不越界、三击直接选中整格全部内容；跨表格与外部正文拖选时表格段同样去边框提取。
- [x] 所有涉及的生产代码文件行数严格 ≤ 1000 行，无 `unwrap()` / `expect()`，`rtk cargo fmt --check`、`rtk cargo clippy --all-targets -- -D warnings` 与相关测试全部通过。
