# lx 消息列表折叠命中区收窄为文字：代码 Review

> 状态：**Review 完成；P1/P2 已修复并通过自动化验证，待用户手工验收**
>
> 关联文档：
> - 设计方案：`docs/feature/lx-fold-hit-text-only/design.md`
> - 任务清单：`docs/feature/lx-fold-hit-text-only/task.md`
>
> 审查方式：codegraph + codebase-memory 结构检索；三个只读子代理分别审查核心命中、
> 快照/系统块、工具卡；主 agent 复现并修复。审查基线为分支创建时的 7 个提交
> （基于 `b5d3eff4`）；review 期间 `dev` 已前进，合并前必须同步 `dev` 并重跑验证。

---

## 0. 结论

未发现 P0。主链路（`card_at` / `card_target_at` / `card_at_source`）的列命中、
`Full`/`Chip` 兼容、重叠点击面顺序、BTW/Agents 滚动口径均正确。

发现一个会直接违背“点文字才折叠”的 P1：**合并组调用体的二级前缀在排版后被绘制
改写为 8 列，列区间仍按 5 列计算**，导致行尾 3 列文字点不中，部分行首空白反而可点。
该问题已在本次 review 中修复并补端到端测试。

此外修复了三个 P2（web_search 标题死点击面、合并组状态尾巴不可点、纯空白展开块
无法用鼠标收起）和若干 P3 清理项。

| 级别 | 数量 | 状态 |
|---|---|---|
| P0 | 0 | - |
| P1 | 1 | 已修复 |
| P2 | 3 | 已修复 |
| P2/P3 | 3 | 已收口/记录 |
| P3 | 3 | 已修复或明确暂缓 |

---

## 1. P1：合并组二级子树列区间整体左移 3 列

### 关键点

`tree::layout_with_roots` 排版本时先给所有行分配 5 列一级前缀
（`  ├─ ` / `  │  ` / `  └─ `），`Layout::row_cols()` 也在这时取列。但合并组
`draw_body` 在绘制前调用 `apply_group_connector*`，把 `node >= roots` 的行改写为
8 列二级前缀（`  │  ├─ ` / `     └─ ` 等）。列区间比实际绘制少 3 列。

进一步定位到第二个根因：`apply_group_connector_with_roots` 内部执行
`root_nodes.max(1)`，而命令输出单独排版时传 `root_nodes = 0`（表示整块都在二级
子树），于是输出块永远按一级前缀取列，和 `draw_body` 的 `roots = 1` 合并布局
错位。

### 错误原因

- 行数与绘制一致，但“列”来自装饰前的 `Layout`，形成两套几何来源。
- 用户可见后果：
  - 合并组内每行最后 3 个显示列的文字点了不折叠；
  - 二级叶节点行的行首 3 列空白反而会触发折叠；
  - 命令输出块、search 命中、文件 facts、subagent 报告、web_search 结果均受影响。
- 行数不变，现有 `rows`/`rendered rows` 断言全部通过，所以只靠旧测试发现不了。

### 修复

- `tool/tree.rs` 新增 `layout_in_group_final(sections, width, root_nodes, last)`：
  排版后立即写入最终连接符，`row_cols()` 与绘制共用同一份行。
- `apply_group_connector_with_roots` 允许 `roots = 0`，不再强制至少一个一级根节点。
- 把 `last` 从 `GroupSlot` 贯穿到各 `*_body_cols` / `body_card_regions` /
  `card_regions_with_group`；合并组内各工具模块的列区间改用最终布局。
- 保留原来的 `layout_in_group` 给只关心行数的路径，避免改变绘制/行数语义。

### 测试

- `src/ui/main_content/lx/history/text_cols/tests.rs`
  - `grouped_nested_rows_include_the_eight_column_prefix`
  - `grouped_root_rows_keep_the_five_column_prefix`
- `src/ui/main_content/lx/history/tests/text_hit.rs`
  - `merged_group_rows_match_rendered_text_columns`：两连 `exec_command` 组成合并组，
    逐个 Tool 卡片逐物理行比较 `CardCols::Text` 与真实渲染的 first/last 列。
    修复前该测试报 `(2,37) != (2,40)`，修复后通过。

---

## 2. P2：web_search 标题在“查询不可展开、结果可展开”时是死点击面

### 关键点

`split_regions` 把标题行只挂在第一块（查询块）上，`clickable` 取
`first_expandable`。当只有一条查询（查询块不折叠）而结果超预算时，标题行
`clickable = false`，点标题没有任何反应；下方结果块却可以折叠。这与
`task.md` 的“web_search 标题继续可点”承诺不一致。

### 修复

- `tool.rs::split_regions` 在“第一块不可展开、第二块可展开”时，额外登记一条
  1 行标题点击面，section 指向 `Output`。标题文字由这条区域优先命中。
- `card_regions_with_group` 的 WebSearch 分支增加
  `has_expandable_content_with_group` 门控：两块都不可展开时不再登记无效点击面，
  与 file/search/subagent 分支一致。

### 测试

- `text_hit.rs::web_search_title_targets_the_only_expandable_block`：
  单查询 + 长结果，标题文字必须命中 `Tool { section: Output }`。

---

## 3. P2：合并组状态尾巴（`· Exited 0 · 241ms`）不在列区间内

### 关键点

`decorate_group_call` 在绘制前把状态文字追加到首节点末行；列区间在此之前已经
算完，所以状态文字本身不可点。属于“少命中”，不会误触空白。

### 修复

- `command::blocks_with_group`：合并组参数布局先 `decorate_group_call`，再
  `row_cols()`。
- `search::body_cols`、`file_summary::body_cols`、`subagent::body_cols`、
  `web_search::layout_with_group`：同样先装饰再取列；各模块的判定条件与
  `draw_body` 保持一致（`Read` 卡只染色、不追加状态；web_search 状态挂到首个
  非空块）。

### 测试

- `merged_group_rows_match_rendered_text_columns` 已从只比较 Output 扩展到全部
  Tool 卡片，命令参数行尾的状态文字必须落在 `CardCols::Text` 的 `last` 内。

---

## 4. P2：展开后的纯空白/纯树线块无法用鼠标收起

### 关键点

`text_cols::cols_from` 对只有树线/空白的行返回 `None`。若一个输出块展开后所有
物理行都是 `│`/空白（例如输出只有框线），`CardCols::Text` 全为 `None`，折叠态
有 `… N more lines` 可点，展开态却没有任何可点文字，鼠标无法收回去。

### 修复

- `CardCols::text_is_blank`：整块没有任何可点文字时为真。
- `history_hit::card_at_source`：`Text` 全空时退化为整块命中；只在该区域本来就
  `clickable` 时由 `card_target_at` 放行。正常有文字的块行为不变。

### 测试

- `text_hit.rs::expanded_all_blank_output_still_can_collapse`：
  输出为多行 `│`，展开后用键盘切到展开态，点击行尾仍能收起。

---

## 5. P2/P3：其它已收口项

### 5.1 `HistoryLayout.cards` 与真实命中语义不一致

- 现状：`history/layout.rs` 仍把每个 `CardRegion` 映射为整块矩形；而真实命中
  `CardHit.card` 对 `Text` 是单行矩形。
- 处理：明确 `HistoryLayout.cards` 只是**块级布局范围**，不能当点击矩形；
  `history_hit::card_target_at` 是唯一命中入口。新增的 `text_hit` 测试全部走真实
  入口，不再用 `layout().cards` 证明收窄。
- 未删除 `HistoryLayout`：它仍被多处布局测试用来取块起止行，删除会扩大回归面。

### 5.2 列区间每帧 clone / thinking 正文重算

- `CardRegion.card_cols` 目前是拥有型 `Vec`，快照重建时 `CardSpan` -> `CardRegion`
  会复制整份列区间；`thinking_body_cols` 每帧重新扫描正文。
- 处理：本轮不做数据结构替换（`Arc<[...]>`）和块缓存扩展；当前量级为每行 6 B，
  没有实测卡顿。列为后续优化，风险是长会话流式期间分配抖动。

### 5.3 `user_shell::blocks` 分段 layout 与绘制不同源

- 现状：参数/输出分别 `tree::layout`，而绘制用合并 `body_layout`；参数区后续有输出
  时，续行画 `│`，列区间起点会偏 3 列。
- 处理：`blocks` 改为取 `body_layout(...).row_cols()` 并按 `node_rows[1]` 切片，
  与绘制同源。

---

## 6. P3：测试与文档

### 6.1 已修复

- `command_params_and_output_hit_text_only` 的“空白行不命中”原来是
  `if let Some(blank)`，fixture 里没有空白行，断言永远不执行；已改为硬断言，
  并在输出首行加入纯 `│` 行。
- `CardCols::skip_rows` 对 `Chip` 增加 `debug_assert_eq!(skip, 0)`，防止未来把
  芯片放进合并组时静默错位。
- `CardHit` / `HistoryLayout` 文档已改为“当前命中矩形 / 块级范围”，不再声称文件
  工具是整块点击面。

### 6.2 明确暂缓

- 零宽字符（ZWJ/组合符）在命中列计算与绘制 `max(1)` 之间的推进差异未统一；
  影响极小，属于后续渲染几何统一任务。
- `CardCols` 的 `Arc` 化、thinking 正文列缓存未做，见 5.2。

### 6.3 文档修正

- `task.md` 中 Task 7 声称已更新 `open_entry.rs` / `tool_geometry.rs` /
  `tool_grouping.rs` / `task/tests.rs` / `mouse_click.rs` / `lx/tests.rs`，但实际
  这些文件未改动；本轮已把勾选改为真实状态，并说明现有测试仍通过是因为点击坐标
  恰好落在文字上，收窄行为由新增 `text_hit` 用例覆盖。
- Task 1 的 `CardCols` 单测、Task 3 的左侧缩进、Task 4 的折叠态只有头部等缺口，
  已在本轮补齐或明确标注。

---

## 7. 验证

```bash
cargo fmt
cargo clippy --all-targets -- -D warnings
cargo test
```

结果：

- `cargo fmt`：通过。
- `cargo clippy --all-targets -- -D warnings`：通过。
- `cargo test`：全量通过。
- `ui::main_content::lx`：791 项通过，5 项 ignored。
- `text_hit`：14 项通过。
- `text_cols`：9 项通过。
- `cards`：3 项通过。

---

## 8. 残余风险

1. 合并组修复引入了 `layout_in_group_final` 和 `last` 参数链路；所有调用点已编译
   通过，但真实 TUI 中仍需确认组内首/末调用的连接符与点击面一致。
2. `Text` 全空退化整块只在无可点文字时生效；正常块应保持逐列命中，需在手工验收
   中确认没有把普通空白行误放行。
3. `HistoryLayout.cards` 仍是块级范围，后续若有人用它做命中测试仍可能自欺；当前
   文档已标注，建议后续直接删除该字段。
