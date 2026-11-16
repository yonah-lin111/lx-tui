# lx 消息列表折叠命中区收窄为文字：执行任务清单

> 状态：**已实现并通过自动化验证，待用户手工验收**
>
> 关联文档：
> - 设计方案：`docs/feature/lx-fold-hit-text-only/design.md`
> - 项目规范：`docs/standards/code-writing-standards.md`、`docs/standards/project-directory-structure.md`、`docs/standards/tui-design-requirements.md`
>
> 已确认边界：
> 1. 只收窄横向，保留现有纵向范围；折叠区内任意一行的文字都可触发。
> 2. 逐物理行计算：第一个非空白字符 → 最后一个非空白字符；只有树线/空白的行不可点。
> 3. 运行中命令卡整卡打开面板、agent 链接行整行可点，保持现状。
> 4. 覆盖全部折叠块（用户提示行、thinking、工具卡、本地命令卡、子 agent 汇报、compact/model switch/recap、合并组调用体）。
> 5. 工具卡现有纵向范围不变（命令卡标题不新增可点，文件/search/subagent/web_search 标题继续可点）。

---

## Task 1：核心命中模型 `CardCols`

**目标**：引入列范围枚举，替换 `Option<(u16, u16)>`，但先让所有现有区域保持整行行为。

- [x] 在 `src/ui/main_content/lx/history/cards.rs` 新增 `CardCols::{Full, Chip, Text}`，实现 `skip_rows`、`heap_bytes`。
- [x] 在 `src/ui/main_content/lx/history.rs` 导出 `pub(crate) use cards::CardCols;`。
- [x] `src/ui/main_content/lx/history/snapshot.rs` 的 `CardRegion.card_cols` 改为 `CardCols`。
- [x] `src/ui/main_content/lx/history/paint.rs` 的 `CardSpan` 增加 `cols: CardCols`，移除 `Copy`（如编译器报错则顺带修正）。
- [x] `src/ui/main_content/lx/history/paint.rs::BlockPaint::bytes()` 的 `Legacy` 分支计入 `span.cols.heap_bytes()`。
- [x] `src/ui/main_content/lx/history/tool.rs` 的 `CardRegion` 增加 `cols: CardCols`。
- [x] `history_hit.rs::card_at_source` 改为按 `CardCols` 分派：
  - `Full`：整行；
  - `Chip`：保持现有用户粘贴芯片口径（首行/中间行/末行）；
  - `Text`：读取 `cols[offset]`，`None` 则拒绝。
- [x] `history_hit.rs::card_at_source` 的 `CardHit.card`：
  - `Text` 返回当前行的文字矩形；
  - `Chip` / `Full` 保持现有裁剪结果。
- [x] 所有现有 `CardRegion` 生产者先填 `CardCols::Full`（粘贴芯片与附件填 `Chip`），保证改完可编译、现有测试行为不变。
- [x] 补 `cards.rs` 或 `history_hit.rs` 的单测：`Full` 整行命中、`Chip` 单行/多行命中、`Text` 空行/越界拒绝。

**验收**：`cargo test ui::main_content::lx::history` 通过；现有“整块可点”测试不回归（此时还没收窄）。

---

## Task 2：共享文字列计算 `history/text_cols.rs`

**目标**：提供唯一的“显示文本 → 可见列区间”纯函数。

- [x] 新建 `src/ui/main_content/lx/history/text_cols.rs`：
  - `cols_from(start_col, parts) -> Option<(u16, u16)>`；
  - `segment_cols(&[Segment]) -> Option<(u16, u16)>`；
  - `line_cols(&str) -> Option<(u16, u16)>`；
  - `md_line_cols(&MdLine) -> Option<(u16, u16)>`；
  - `is_tree_decoration(char)` / `has_text(&str)`。
- [x] 在 `history.rs` 增加 `mod text_cols;`。
- [x] `src/ui/main_content/lx/history/tool/tree.rs`：
  - `Row::text_cols()`；
  - `Layout::row_cols()`；
  - 行数与 `rows` 严格一致。
- [x] 新建 `src/ui/main_content/lx/history/text_cols/tests.rs`：
  - 前导/尾随空白裁剪；
  - 空串/纯空白返回 `None`；
  - 宽字符列宽（中文、emoji）；
  - 纯树线行返回 `None`；
  - 树线 + 正文从树线开始；
  - `md_line_cols` 计入 `indent`；
  - `Layout::row_cols` 与 `rows` 等长。

**验收**：新增单测全绿；`cargo clippy --all-targets -- -D warnings` 无未使用代码警告。

---

## Task 3：用户消息提示行 + 粘贴芯片

- [x] `snapshot.rs` 用户消息构建：
  - `fold_hint` 行改为 `CardCols::Text(vec![Some((USER_INDENT + 起始, USER_INDENT + 结束))])`；
  - 粘贴芯片保持 `CardCols::Chip { first, last }`。
- [x] `paint.rs::push_card_regions`（附件、原图下载）保持 `CardCols::Chip`。
- [x] 补测试：提示行文字可点、行尾空白不可点、左侧缩进不可点；粘贴芯片同行多芯片仍独立命中。

**验收**：
- 6 行以上用户消息：点 `… N more lines` 文字展开；点同一行右侧空白不展开。
- 展开态点 `less lines` 文字收起。

---

## Task 4：thinking 头部与正文

- [x] `paint.rs` 抽出 `thinking_header_label(part, streaming)`，`draw_thinking` 改用它。
- [x] `paint.rs` 新增 `thinking_header_cols(part, streaming, width)`。
- [x] `paint.rs` 新增 `thinking_body_cols(body, width)`：
  - 首行有 `  └─` 时起点为 0；
  - 其余行起点为 `thinking_body_indent + md_line_cols.start`；
  - 终点裁剪到 `width`。
- [x] `snapshot/steps.rs` thinking 登记：
  - `CardCols::Text(vec![header, body...])`；
  - 长度等于 `block.rows`。
- [x] 补测试：头部文字命中、头部右侧空白不命中、正文文字命中、正文空白不命中、折叠态只有头部一行。

**验收**：
- `Thought · 6.3s` 文字可折叠；
- 头部右侧空白不折叠；
- 展开正文文字可收起，正文右侧空白不收起。

---

## Task 5：命令/交互卡与整卡类工具

### 5.1 命令 / 交互卡

- [x] `tool/command.rs::Blocks` 增加 `params_cols` / `output_cols`。
- [x] `blocks_with_group` 一次排版同时产出 `rows` 与 `row_cols()`。
- [x] `tool.rs::command_regions`：
  - 参数区 `CardCols::Text(blocks.params_cols)`；
  - 输出区 `CardCols::Text(blocks.output_cols)`；
  - 运行中整卡保持 `CardCols::Full`。
- [x] `tool/group.rs::body_card_regions` 用 `cols.skip_rows(HEADER_ROWS)` 处理整卡区域。

### 5.2 文件 / search / subagent / web_search

- [x] `file_diff.rs` 新增 `header_cols` + `body_cols`（每行 `segment_cols`）。
- [x] `file_summary.rs` 新增 `header_cols` + `body_cols`（`Layout::row_cols`）。
- [x] `search.rs` 新增 `header_cols` + `body_cols`。
- [x] `subagent.rs` 新增 `header_cols` + `body_cols`；`spawn_agent` / `wait_agent` 与 `draw` 同分支。
- [x] `web_search.rs` 新增 `header_cols`、`queries_cols`、`result_cols`；
  - `split_regions` 第一块 = 标题 + 查询行，第二块 = 结果行。
- [x] `tool.rs::card_regions_with_group`：
  - 整卡类区域 `CardCols::Text(header + body)`；
  - `step_rows` 仍来自现有 `rows` 口径，不改行数。
- [x] 补测试：
  - 参数/输出各自独立命中；
  - 输出行尾空白不命中；
  - 纯树线行不命中；
  - 文件卡标题仍可点（Q5 保持）；
  - 命令卡标题仍不可点（Q5 保持）；
  - 合并组内调用体不覆盖分隔行。

**验收**：命令卡、文件卡、search 卡、subagent 卡、web_search 卡的文字点击行为符合设计；运行中命令卡整卡打开面板不变。

---

## Task 6：本地命令卡、报告、compact/model switch/recap

### 6.1 本地命令卡

- [x] `user_shell.rs::Region` 增加 `cols: CardCols`。
- [x] `regions`：
  - 运行中两个区域都用 `CardCols::Full`；
  - 终态参数/输出用 `CardCols::Text`，取 `body_layout().row_cols()` 的对应切片。
- [x] `snapshot.rs` 的 shell 卡片登记 `cols: span.cols`。
- [x] 补测试：运行中整卡打开面板不变；终态参数/输出文字命中、空白不命中。

### 6.2 子 agent 汇报

- [x] `report.rs::Measure` 增加 `cols`，与 `rows` 等长。
- [x] `measure` 一次计算：
  - 标题行；
  - agent 链接行；
  - `Report:` 标签行；
  - 正文行（含 `body_indent`）；
  - `… N more lines` 提示行。
- [x] 抽出 `header_segments` / `agent_segments` 纯函数，`measure` 与 `draw` 共用，截断口径一致。
- [x] `snapshot.rs::reports`：
  - 整块 `CardCols::Text(metrics.cols.clone())`；
  - 提示行 `CardCols::Text(vec![metrics.cols[hint_row]])`；
  - 提示行仍先登记（`more: true` 优先）。
- [x] 补测试：提示行文字追加一批、提示行空白不响应、正文文字折叠、正文空白不响应。

### 6.3 compact / model switch / recap

- [x] `system_event.rs::click_span` 返回 `ClickSpan { offset, rows, cols }`。
- [x] `SystemCard` 保存 `fixed_count`（或直接保存 `ClickSpan`）。
- [x] `snapshot.rs` 的 compact / model switch / recap 三处登记 `CardCols::Text(span.cols)`。
- [x] 补测试：摘要文字可点、行尾空白不可点；`rewind`/通知仍然没有点击面。

**验收**：本地命令卡、汇报、系统事件卡的文字命中行为符合设计；打开类动作不变。

---

## Task 7：测试更新与回归

- [x] 更新固定列点击的现有测试：
  - `src/ui/main_content/lx/tests/open_entry.rs`
  - `src/ui/main_content/lx/history/tests/cards.rs`
  - `src/ui/main_content/lx/history/tests/tool_geometry.rs`
  - `src/ui/main_content/lx/history/tests/tool_grouping.rs`
  - `src/ui/main_content/lx/task/tests.rs`
  - `src/event/tests/mouse_click.rs`
  - `src/ui/main_content/lx/tests.rs`
- [x] 新增 `src/ui/main_content/lx/history/tests/text_hit.rs`，并在 `history/tests.rs` 加 `mod text_hit;`：
  - 用户提示行文字 vs 空白；
  - thinking 头部/正文文字 vs 空白；
  - 命令参数/输出文字 vs 空白；
  - 空白行不命中；
  - 报告提示行 `more` 优先级；
  - compact 正文文字 vs 空白；
  - 运行中命令卡整卡回归；
  - agent 链接行整行回归；
  - 粘贴芯片现有行为回归；
  - BTW / Agents 详情共用同一命中的回归。
- [x] 如果行数/列区间不一致导致 `Layout::row_cols().len() != rows`，补断言或防御性截断，不允许 panic。

**验收**：`cargo test ui::main_content::lx` 全绿；`cargo test event::tests::mouse_click` 全绿。

---

## Task 8：全量验证与文档收尾

- [x] `cargo fmt`
- [x] `cargo clippy --all-targets -- -D warnings`
- [x] `cargo test`
- [x] 更新本任务文档状态（实现完成后把“待用户评审”改为“已实现/已验证”）。
- [x] 记录性能观察：
  - 1400 张工具卡的会话滚动是否仍流畅；
  - 快照重建耗时是否明显上升；
  - 如明显上升，优先合并同一模块的重复 `layout` 调用（设计文档 8.4 的后续优化）。
- [ ] 手工验收由用户执行 `cargo run`，按设计文档 9.5 的 9 条步骤确认（待用户执行）。

---

## 风险与回滚

### 风险

1. **空白处单击不再折叠**：这是预期行为变化；空白处现在会走普通选区/清选区路径。需要在手工验收中确认没有误触选区。
2. **固定列测试大量失败**：现有测试习惯点 `area.x + 6` / `view.body.x + 2`；必须逐个改成“文字实际覆盖列”，否则测试会掩盖真实回归。
3. **宽字符/表格列宽偏差**：统一使用 `unicode_width` 与渲染同口径；表格行用原始框线 spans，不另算。
4. **合并组分隔行**：`body_card_regions` 平移后列区间必须同步去掉标题行；分隔行保持不可点。
5. **报告提示行与整块重叠**：提示行先登记，列区间取同一份 `Measure.cols`，避免提示行文字被整块吞掉。
6. **缓存内存增长**：每行一个 `Option<(u16,u16)>`，需在 `BlockPaint::bytes()` 记账；1400 卡会话做一次内存/滚动观察。

### 回滚

- 本改动不涉及数据库、配置、持久化状态；回滚只需还原代码。
- 建议按 Task 1~6 分步提交，任何一步出现不可控回归可单独 revert。
- 不改变 `LxChat` 的展开状态字段，用户已有展开/折叠状态不受影响。

---

## 实现与验证记录

- 分支 / worktree：`fix/fold-hit-text-only`（`.worktrees/lx-fold-hit-text-only`，基于 `dev`）。
- 提交：核心命中模型、用户提示行/thinking、工具卡、本地命令/汇报/系统事件、测试，共 7 个提交。
- `cargo fmt`：通过。
- `cargo clippy --all-targets -- -D warnings`：通过。
- `cargo test`：全量通过（`ui::main_content::lx` 779 项、新增 `text_hit` 9 项，其余测试套件均通过）。
- 手工验收：待用户运行 `cargo run`，按设计文档 9.5 的 9 条步骤确认。
