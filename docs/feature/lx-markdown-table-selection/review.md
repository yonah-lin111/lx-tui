# `lx` 消息列表 Markdown 表格单元格选区与去边框复制 — 代码评审报告 (`review.md`)

- **评审范围**：`feat/lx-markdown-table-selection` 分支（worktree `.worktrees/lx-markdown-table-selection`，初始实现提交 `74ea7150`，对比基准 `dev`）
- **特性目录**：`docs/feature/lx-markdown-table-selection/`
- **合并状态**：已合入主工作区 `dev` 分支；worktree `.worktrees/lx-markdown-table-selection` 与分支 `feat/lx-markdown-table-selection` 已移除。
- **关联文档**：
  - 详细设计：[`design.md`](./design.md)
  - 任务清单：[`task.md`](./task.md)
- **评审方式**：结合 `codegraph`、`codebase-memory-mcp` 与 3 个并行子 Agent（`general` 审查 `selection/table.rs`、`selection.rs` 与事件链路，`general` 审查 `markdown_render/table.rs` 与 `history/*` 全渲染路径，`explore` 核对项目级开发规范與 `docs/standards/*.md`）对新增代码、边界交互与测试覆盖进行全量深度审计。
- **优先级定义**：
  - **P0（阻断级）**：核心功能在真实交互链路不可达、或折叠/截断态下出现跨行误高亮与隐藏行越界。
  - **P1（高优先级）**：复制内容准确性缺陷（空单元格列移位、连续 `<br><br>` 空行丢失、CJK 双宽字符半格拖选漏字）。
  - **P2（中优先级）**：边界条件交互与性能退化（边框/填充行双击误选单字、表格左右空白区误吸附、极窄宽度收缩失效、大表全量渲染与深比较开销）。
  - **P3（低优先级）**：子模块测试拆分规范、异常空集合回退与设计/任务文档同步。

---

## 问题分级总览（按 P0 → P3 排序）

| 级别 | 编号 | 模块 / 文件 | 关键点摘要 | 状态 |
| :---: | :---: | :--- | :--- | :---: |
| **P0** | **P0-1** | `src/event/mouse/press.rs`<br>`src/ui/main_content/lx/selection.rs`<br>`src/ui/main_content/lx.rs` | 可折叠卡片（思考块、汇报块、工具输出卡片）展开后，表格内双击选词（`Word`）与三击选整格（`Line`）被 `card_target_at` 拦截，无法选中且松开即误折叠卡片 | 已修复 |
| **P0** | **P0-2** | `src/ui/main_content/lx/selection/table.rs`<br>`src/ui/main_content/lx/selection.rs` | 折叠/截断卡片（如汇报块 `REPORT_PREVIEW_ROWS = 2`、命令 `Preview::head_tail`）中的表格按完整未截断高度计算 `table_end_row`，在表格下方普通行横向拖选会误反显上方表格行，且三击选整格可能越界覆盖隐藏行 | 已修复 |
| **P1** | **P1-1** | `src/ui/main_content/lx/selection/table.rs` | `ColumnLocked` / `Grid` 模式下空单元格被从 `row_cells` 中过滤丢弃，导致多列 TSV 复制时后续列左移错位 | 已修复 |
| **P1** | **P1-2** | `src/ui/main_content/lx/selection/table.rs` | 单元格内连续 `<br><br>` 产生的空物理行因 `text_start == text_end` 被跳过，未结算 `hard_break`，导致连续硬换行 `\n\n` 被压扁为单个 `\n` | 已修复 |
| **P1** | **P1-3** | `src/ui/main_content/lx/selection/table.rs`<br>`src/ui/main_content/lx/selection.rs` | 从 CJK 双宽字符右半格（`CONTINUATION` 列）按下拖选，或拖选终点落在双宽字符左半格时，未对齐完整字形列边界，导致首字漏高亮/漏复制 | 已修复 |
| **P2** | **P2-1** | `src/ui/main_content/lx/selection/table.rs`<br>`src/ui/main_content/lx/selection.rs` | 在表格水平边框行或空白填充行（`is_filler`）双击（`SelectionUnit::Word`）返回单点选区，复制时被 `clamp_point_to_cell` 钳制并误复制邻格单字符；且 `unit_cell_range` 传入未钳制行号 | 已修复 |
| **P2** | **P2-2** | `src/ui/main_content/lx/selection/table.rs` | 在表格左侧缩进空白区（`< base_col`）或右侧空白区（`>= base_col + total_width`）水平拖选或双击时，被误判为表格内选区并吸附首列/末列 | 已修复 |
| **P2** | **P2-3** | `src/ui/markdown_render/table.rs`<br>`src/ui/main_content/lx/history/tool.rs` | 极窄宽度下（`floor_total > available`）`shrink_widths` 误写 `*slot = (*slot).max(*floor)` 导致列宽未收缩至下限；`tool::wrap_spans` 将 `table` 标记重挂在含前导缩进的首段上 | 已修复 |
| **P2** | **P2-4** | `src/ui/main_content/lx/selection.rs`<br>`src/ui/main_content/lx/selection/table.rs` | `copy_text` 无论选区大小均渲染整张表、对 `Arc<TableLayoutMeta>` 做无短路深比较、对 `present_offsets` 线性查找；`paint_selection` 逐像素列重复遍历全部列 | 已修复 |
| **P3** | **P3-1** | `src/ui/main_content/lx/selection/table.rs`<br>`src/ui/main_content/lx/selection/table/tests.rs`<br>`docs/feature/lx-markdown-table-selection/{design.md,task.md}` | 子模块 `selection/table.rs` 测试拆入 `selection/table/tests.rs`、补充 `present_offsets` 为空时的安全回退、同步更新 `design.md` 与 `task.md` | 已修复 |

---

## 详细问题分析与错误原因

### P0-1：可折叠卡片（思考块、汇报块、工具输出卡片）内的表格双击选词与三击选整格不可达且误折叠卡片

- **涉及位置**：
  - `src/event/mouse/press.rs:907-933`
  - `src/ui/main_content/lx/selection.rs:201-228`
  - `src/ui/main_content/lx.rs:64-66`
- **关键点**：
  设计文档 Q4 明确要求双击选词（`SelectionUnit::Word`）与三击选整格（`SelectionUnit::Line`）覆盖助手主回答、思考块（`PartKind::Reasoning`）、工具输出与子智能体汇报块中的 Markdown 表格。但在展开的思考块、汇报块或工具输出卡片中双击或三击表格单元格时，真实鼠标事件链路不仅不会选中单词或整格，反而在第一次点击松开鼠标时把卡片折叠收起。
- **错误原因**：
  1. `snapshot.rs` 为思考块（`:582-592`）、汇报块（`:452-464`）与工具卡片（`:565-571`）登记的可折叠点击面覆盖了整个块高度；
  2. `handle_mouse_press`（`src/event/mouse/press.rs:907`）只要检测到 `card_target_at` 返回 `Some(_)`，便无条件将 `state.selection_click = None` 清零（打断双击/三击计数），仅调用 `begin_lx_selection` 启动 `SelectionUnit::Character` 单点选区，并记录 `state.lx_card_press`；在松开鼠标时未发生拖拽，遂触发 `toggle_lx_card` 折叠卡片；
  3. 初版测试直接调用了 `ui::main_content::lx::unit_cell_range`，绕过了 `handle_terminal_event` -> `handle_mouse_press` 真实鼠标入口，因而未能捕获该拦截问题。
- **修复措施**：
  1. 在 `src/ui/main_content/lx/selection.rs` 新增 `pub(crate) fn is_table_cell_at(area, lx, cell, cache) -> bool`（并由 `src/ui/main_content/lx.rs` 导出），渲染目标单行并判断点击点 `(content_row, content_col)` 是否落在携带 `GridTableRow` 的表格水平范围 `[base_col, base_col + table.total_width)` 内；
  2. 在 `src/event/mouse/press.rs:907` 中，当点击位置命中 Markdown 表格（`on_table == true`）时，不进入 `lx_card_press` 折叠分支，而是走 `next_click_count` + `unit_cell_range` + `begin_lx_selection_range` 选区分支（且不设置 `lx_card_press`），同时保留点击卡片头部及非表格正文行切换折叠的原语义；
  3. 新增经由 `handle_terminal_event` 真实鼠标连击事件的集成测试，验证思考块与汇报块内表格双击选词、三击选整格正常生效且不折叠卡片。

---

### P0-2：折叠/截断卡片下的表格按完整高度推算 `table_end_row`，导致表格下方普通行拖选误反显上方表格行

- **涉及位置**：
  - `src/ui/main_content/lx/selection/table.rs:141-177, 297-331`
  - `src/ui/main_content/lx/selection.rs:153-184, 218-228`
- **关键点**：
  当子智能体汇报块处于折叠态（`REPORT_PREVIEW_ROWS = 2`，仅显示表格顶边框 `line_offset=0` 与表头 `line_offset=1`，下方紧跟 `… N more lines` 提示行或下一消息）或命令输出使用 `Preview::head_tail(3, 2)` 截断表格时：
  1. 用户在表格下方的提示行或普通正文行上横向拖选，上方已折叠表格的表头行却被错误添加 `Modifier::REVERSED` 反显，导致“选在下行、亮在上表”；
  2. 在部分截断的多行单元格上三击（`SelectionUnit::Line`），返回的 `start_row..=end_row` 会按未截断的 `logical.first_line..=last_non_filler` 跨越隐藏行。
- **错误原因**：
  `resolve_table_selection` 直接使用 `table_end_row = table_start_row + table.lines.len() - 1`，假定 `TableLayoutMeta` 中的所有物理行都在内容行空间里连续完整渲染。当表格被折叠截断时，下方非表格行的 `content_row` 仍小于虚拟的 `table_end_row`，导致 `anchor_in_table && cursor_in_table` 被误判为 `true`，并将虚拟的 `line_offset = 2`（中间分割线）吸附回 `row_idx = 0`（表头行）进行高亮。
- **修复措施**：
  1. 在 `src/ui/main_content/lx/selection/table.rs` 新增 `rendered_table_row_bounds(grid, grid_first_row, grid_y_offset, table_row, content_row) -> (i32, i32)`，从当前行在 `grid` 内向上、向下扫描：若视口内某内容行落在理论 `[table_start_row, table_end_row]` 内，但 `grid` 在该行并未挂载同一表格实例（`same_table_segment` 为 `false`），则将表格的真实有效行范围 `[actual_start_row, actual_end_row]` 严格截断在该边界处；
  2. `resolve_table_selection` 接收 `(actual_start_row, actual_end_row)`，据此判断选区是否与当前可见表格段相交以及 `anchor_in_table` / `cursor_in_table`，并在跨出截断底边时将 `last_row_idx` 钳制在最后一条实际可见的逻辑行；
  3. `unit_cell_range` 在处理 `SelectionUnit::Line` 三击时，按候选行范围渲染并仅保留实际挂载同一表格 `GridTableRow` 的首尾行。

---

### P1-1：`ColumnLocked` / `Grid` 模式下空单元格被丢弃，导致多列 TSV 复制列移位

- **涉及位置**：
  - `src/ui/main_content/lx/selection/table.rs:410-470`
- **关键点**：
  在含空单元格的表格（如第 2 行第 2 列为空 `| x |   | z |`）上执行跨列 `Grid` 选区复制时，输出为 `"x\tz"` 而非 `"x\t\tz"`，空单元格对应的制表符占位丢失，粘贴到电子表格或文本对比时后续列全部左移错位。
- **错误原因**：
  `extract_table_lines` 对所有模式统一执行了 `if !trimmed.is_empty() { row_cells.push(trimmed.to_string()); }`，将空单元格直接从 `row_cells` 中过滤掉，混淆了 `SingleCell`（单格提取）与 `ColumnLocked` / `Grid`（多列/多行表格结构提取）的占位规则。
- **修复措施**：
  在 `extract_table_lines` 中：
  - 当处于 `SingleCell` 模式时，仅在 `!trimmed.is_empty()` 时推入结果；
  - 当处于 `ColumnLocked` 或 `Grid` 模式时，只要该逻辑行在 `present_line_offsets` 中存在已渲染物理行，选中的每一列 `col_idx` 均推入 `trimmed.to_string()`（空单元格保留 `""` 占位），确保同一逻辑行内选中列数与 `\t` 分隔字段数严格一致。

---

### P1-2：单元格内连续 `<br><br>` 产生的空物理行在复制时被跳过，丢失连续硬换行 `\n\n`

- **涉及位置**：
  - `src/ui/main_content/lx/selection/table.rs:51-63, 420-460`
- **关键点**：
  当单元格包含连续 `<br><br>`（如 `x<br><br>y`）时，`grid_table` 会生成一条 `text_start == text_end` 且 `hard_break == true` 的空物理行；但复制该单元格时输出为 `"x\ny"`，中间空行丢失，未能还原 `"x\n\ny"`。
- **错误原因**：
  `selected_cell_cols` 对 `slice.text_start >= slice.text_end` 直接返回 `None`；`extract_table_lines` 遇到 `selected_cell_cols` 返回 `None` 时立即 `continue`，跳过了该空物理行的 `slice.hard_break` 结算，导致连续的 `<br>` 被合并成一次换行。
- **修复措施**：
  在 `TableSelectionPlan` 中新增 `is_cell_line_selected(&self, line_offset: usize, col_idx: usize) -> bool` 判断该物理行是否位于选中范围内且 `!slice.is_filler`；在 `extract_table_lines` 中，对位于选中范围内但无字符（`selected_cell_cols` 为 `None`）的非填充空物理行，正常结算前序 `prev_hard_break`（压入 `\n`）并更新 `has_prev_line = true`、`prev_hard_break = slice.hard_break`。

---

### P1-3：从 CJK 双宽字符右半格（`CONTINUATION` 列）按下拖选或终点落在左半格时漏字

- **涉及位置**：
  - `src/ui/main_content/lx/selection/table.rs:51-137, 420-460`
  - `src/ui/main_content/lx/selection.rs:165-196, 362-372`
- **关键点**：
  在含中文等双宽字符（占 2 列，右半格为 `CONTINUATION`）的单元格内，若鼠标从某个宽字符的第 2 列（右半格）按下向右拖选，该宽字符的主格（左半格 `col0 - 1`）未被纳入 `[col0, col1]`，而右半格 `col0` 在反显与提取时均被跳过，导致首字既不高亮也不出现在复制文本中。
- **错误原因**：
  `clamp_point_to_cell` 与 `selected_cell_cols` 仅根据列索引数值计算区间，未结合 `Grid` 中的 `CONTINUATION` 占位符将选区起止列吸附到完整双宽字符的左右边界。
- **修复措施**：
  在 `src/ui/main_content/lx/selection/table.rs` 新增 `pub(super) fn align_wide_char_cols(grid: &Grid, y: u16, col0: u16, col1: u16) -> (u16, u16)`：
  - 若 `col0 > 0` 且 `grid.cell(col0, y).ch == CONTINUATION`，将 `col0` 左扩至 `col0 - 1`；
  - 若 `col1 + 1 < grid.area().width` 且 `grid.cell(col1 + 1, y).ch == CONTINUATION`（且 `col1` 为主格），将 `col1` 右扩至 `col1 + 1`；
  - 在表格 `paint_selection`、`extract_table_lines` 以及普通行流 `copy_text` / `paint_selection` 中统一调用该函数对齐双宽字符边界。

---

### P2-1：在表格水平边框行或空白填充行上双击（`SelectionUnit::Word`）误复制邻格单字符，且 `unit_cell_range` 传入未钳制行号

- **涉及位置**：
  - `src/ui/main_content/lx/selection/table.rs:183-215, 332-348`
  - `src/ui/main_content/lx/selection.rs:218-228`
- **关键点**：
  1. 双击表格水平边框线（`┌─┬─┐` / `├─┼─┤` / `└─┴─┘`）或单元格下方的空白填充行（`is_filler`）时，`table_unit_cell_range` 返回单点 `((row, c), (row, c))`；由于 `unit == SelectionUnit::Word`，`Selection::range()` 视为非空选区，随后 `resolve_table_selection` 调用 `clamp_point_to_cell` 将该点钳到邻近单元格的首字符或末字符，导致复制出 1 个字符；
  2. `unit_cell_range` 将 `row` 钳制到 `geom.total_rows - 1` 后渲染单行 `grid`，但传给 `table_unit_cell_range` 的仍是未钳制的原始 `cell`。
- **错误原因**：
  `resolve_table_selection` 未区分“拖拽产生的跨格/跨列选区”与“`anchor == cursor` 单点选区落在无文本的边框行/填充行/空单元格上”；当 `anchor == cursor` 且所在物理行无有效单元格文本时，仍经 `clamp_point_to_cell` 强行钳入邻近物理行。
- **修复措施**：
  1. 在 `unit_cell_range` 中将 `clamped_cell = (i32::try_from(row).unwrap_or(cell.0), cell.1)` 传给 `table_unit_cell_range`；
  2. 在 `resolve_table_selection` 中，当 `anchor == cursor` 时，检查 `anchor_line` 是否为非填充的 `TableLineKind::Cells` 且目标列 `slice.text_start < slice.text_end`；若不满足（即单点落在水平边框行、空白填充行或空单元格上），直接返回 `None`，既不反显也不复制任何字符。

---

### P2-2：表格左侧缩进空白区或右侧空白区水平拖选/双击被误判为表格内选区

- **涉及位置**：
  - `src/ui/main_content/lx/selection/table.rs:157-168, 297-311`
- **关键点**：
  在表格左侧缩进列（`< base_col`）或表格右边框外侧空白列（`>= base_col + table.total_width`）按下并在表格外水平拖动或双击时，仍会进入 `resolve_table_selection` / `table_unit_cell_range` 并将列坐标夹取到第 0 列或最后一列，导致鼠标未触碰表格却误选首列/末列。
- **错误原因**：
  `resolve_table_selection` 与 `table_unit_cell_range` 未使用 `table.total_width` 校验鼠标横坐标是否与表格水平区间 `[base_col, base_col + table.total_width)` 相交。
- **修复措施**：
  1. 在 `table_unit_cell_range` 中，若 `cell.1 < base_col || cell.1 >= base_col.saturating_add(table.total_width)`，直接返回 `None`；
  2. 在 `resolve_table_selection` 中，当 `anchor_in_table && cursor_in_table`（起止行均在表格行范围内）时，若 `(anchor.1 < base_col && cursor.1 < base_col)` 或 `(anchor.1 >= table_right && cursor.1 >= table_right)`（两端列坐标均完全在表格同侧外部），直接返回 `None`。

---

### P2-3：极窄宽度下 `shrink_widths` 未收缩至 `floors` 下限，且 `tool::wrap_spans` 将 `table` 标记错挂在缩进段上

- **涉及位置**：
  - `src/ui/markdown_render/table.rs:92-100`
  - `src/ui/main_content/lx/history/tool.rs:803-835`
- **关键点**：
  1. 当窄窗格可用宽度小于各列最宽字符之和（`floor_total > available`）时，`shrink_widths` 执行了 `*slot = (*slot).max(*floor)`，由于初始自然列宽恒 `>= *floor`，导致列宽保持未收缩的自然总宽，随后被 `wrap_md_line` 硬切断并丢失全部 `TableLineRef` 元数据；
  2. `tool::wrap_spans` 在 `rows.len() == 1` 时将 `first_table` 挂到 `rows[0][0]`，若存在前导 `indent` 空白 segment，则 `put_segments` 会在缩进前登记 `base_col`。
- **错误原因**：
  1. `shrink_widths` 的下限兜底分支把赋值误写成了 `(*slot).max(*floor)` 而非收缩到 `*floor`；
  2. `wrap_spans` 未在拆分循环中保留原 segment 的 `table` 属性，而是在末尾盲目赋给首段 `rows[0][0]`。
- **修复措施**：
  1. 在 `src/ui/markdown_render/table.rs:93-100` 中，当 `floor_total > available` 时将各列收缩至 `(*floor).min(*slot).max(1)`；
  2. 在 `src/ui/main_content/lx/history/tool.rs:803-835` 中，让每个源 `Segment` 的首个切分片段继承自身的 `table` 属性，仅当 `rows.len() > 1` 发生二次折行时清空 `table` 标记。

---

### P2-4：`copy_text` 与 `paint_selection` 在大表格下的性能退化

- **涉及位置**：
  - `src/ui/main_content/lx/selection.rs:170-183, 319-352`
  - `src/ui/main_content/lx/selection/table.rs:131-137, 420-423`
- **关键点**：
  1. `copy_text` 遇到表格选区时按整张表 `table_start_row..=table_end_row` 分配并渲染 `t_grid`，在大表格中即使只选 1 个单元格也会渲染整张表；
  2. `tr.line_ref.table == plan.table` 直接触发 `Arc<TableLayoutMeta>` 的完整深比较，未先用 `Arc::ptr_eq` 短路；
  3. `extract_table_lines` 中 `present_line_offsets.contains(&line_offset)` 为线性扫描；
  4. `paint_selection` 对每个可见列 `dx` 重复调用 `plan.contains_col(line_offset, dx)` 遍历所有列。
- **错误原因**：
  初版实现未按 `TableSelectionPlan` 中实际选中逻辑行的物理行区间裁剪 `t_grid` 渲染范围，也未对 `Arc` 同一性比较和单行选中列区间做预计算优化。
- **修复措施**：
  1. 在 `TableSelectionPlan` 新增 `selected_line_offset_range(&self) -> (usize, usize)`，`copy_text` 仅渲染选中逻辑行对应的物理行范围 `[t_first, t_last]`；
  2. 新增 `same_table_meta(a, b)`，先用 `Arc::ptr_eq(a, b)` 短路（`O(1)`），仅在指针不同时回退值比较；
  3. `present_line_offsets` 使用 `binary_search` 替代 `Vec::contains`；
  4. 在 `TableSelectionPlan` 新增 `selected_col_ranges(&self, line_offset: usize) -> Vec<(u16, u16)>`，`paint_selection` 每行仅计算一次选中列区间并对齐双宽字符后直接反显。

---

### P3-1：子模块测试组织、空集合安全回退与特性文档同步

- **涉及位置**：
  - `src/ui/main_content/lx/selection/table.rs`
  - `src/ui/main_content/lx/selection/table/tests.rs`
  - `src/ui/main_content/lx/selection/tests.rs`
  - `docs/feature/lx-markdown-table-selection/{design.md,task.md}`
- **关键点**：
  1. 遵循 `docs/standards/code-writing-standards.md`“生产代码拆出子模块时测试同步迁入对应子模块（`<module>/<submodule>/tests.rs`）”规范，将 `selection/table.rs` 的纯几何与提取测试组织到 `src/ui/main_content/lx/selection/table/tests.rs`；
  2. `copy_text` 在 `present_offsets` 为空时应回退到普通行复制而非静默跳过；
  3. 同步更新 `design.md` 第 3.4 节与 `task.md`，确保文档与最终代码实现完全一致。
- **错误原因**：
  初版实现将全部测试集中在 `selection/tests.rs`，且未在 `design.md` 中同步清理早期草稿伪代码。
- **修复措施**：
  1. 创建 `src/ui/main_content/lx/selection/table/tests.rs` 覆盖 P0–P3 各项表格几何、边界与提取单测，保持生产文件与测试文件职责清晰、行数合规；
  2. 在 `copy_text` 中当 `!present_offsets.is_empty()` 时才跳过普通行提取；
  3. 更新 `design.md` 与 `task.md`。
