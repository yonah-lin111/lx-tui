# 任务：`+++` 补充块插入落点调整

- 设计文档：`docs/feature/prompt-supplement-insert-before-next/design.md`
- 状态：已合并到 `dev`（2026-11-13，合并提交 `88c16880`）；自动化校验通过，P4 待用户人工验收
- 审查记录：`docs/feature/prompt-supplement-insert-before-next/review.md`
- 合并记录：fast-forward 合并；工作区与分支已清理。
- 执行原则：只改 `app` 纯状态路径；不碰 UI 渲染、DB、配置；每步完成后做精确校验
- 实施结果：P0–P3 与 P5 已完成；`cargo fmt`、`cargo clippy --all-targets -- -D warnings`、`cargo test` 全部通过（block 31、prompt 156、integration 6、全量 5429 passed / 8 ignored）。P4 待用户 `cargo run` 人工验收。

## P0 前置核对

- [x] 确认工作区在 `lx-tui` 主分支预期分支，`git status` 无与本任务冲突的未提交改动。
- [x] 重新读取 `src/app/prompt/block.rs`、`src/app/prompt/panel.rs`、`src/app/markdown/blocks.rs`，确认行号与设计文档一致；若已变化，先更新设计文档再写代码。
- [x] 确认 `block_scan` 仍只识别首个 `&&&`、`+++` 按文档顺序产出 `spans`。

## P1 核心实现

### P1.1 `src/app/prompt/block.rs`

- [x] 把 `pub(super) fn append_supple_block` 重命名为 `pub(super) fn insert_supple_block`。
- [x] 只清 `/` + query：按 `trigger.query` 计算命令起点，保留行首缩进与同一行的粘贴 chip；清理后 `cursor = command_start`。
- [x] 保留未闭合块守卫：`scan.spans.iter().any(|span| !span.closed)` 时直接返回。
- [x] 清命令前记录 `at_document_start = self.text[..trigger.line_start].trim().is_empty()`，成功后用 `trim_leading_blank_lines` 折叠文首空行。
- [x] 计算 `command_line = self.text[..trigger.line_start].matches('\n').count()`。
- [x] 计算 `next_start = scan.spans.iter().find(|span| span.start > command_line).map(|span| span.start)`。
- [x] 计算旧锚点 `anchor_line`：最后一个已闭合 `+++` 的 `end` → 首个已闭合 `&&&` 的 `end` → `None`。
- [x] 在调用 `splice_text` 前把 `next_start` / `anchor_line` 固化为 `usize` / `Option<usize>`，避免 `masked` 借用冲突。
- [x] 无 `next_start` 且无 `anchor_line` 时，若 `open_fence(&masked)` 存在则跳过插入，避免追加进未闭合围栏。
- [x] 新增 `fn insert_before_line(&mut self, line: usize, content: &str) -> Option<usize>`：
  - [x] `line_bounds(line)?` 取行首字节；越界返回 `None`，主流程放弃插入。
  - [x] `prefix.trim().is_empty()` 时清掉 `0..line_start`，插入 `content + "\n\n"`，返回 `Some(0)`。
  - [x] 否则 `base = last_content_end(prefix)`，替换 `base..line_start` 为 `"\n\n" + content + "\n\n"`，返回 `Some(base + 2)`。
- [x] 新增 `last_content_end(prefix)`：最后一个非空白行内容末端，保留该行尾随空格，折叠只含空白的行。
- [x] 新增 `fn append_after_anchor(&mut self, anchor_line: Option<usize>, content: &str) -> usize`：
  - [x] 有锚点且后面有内容：用 `first_content_line_start` 找第一个内容行，替换区间为 `"\n\n" + content + "\n\n"`。
  - [x] 有锚点且后面无内容：用 `last_content_end` 从最后一个非空白行末尾追加 `"\n\n" + content`。
  - [x] 无锚点且文本为空：`content` 占全文，返回 `0`。
  - [x] 无锚点且文本非空：用 `last_content_end` 从最后一个非空白行末尾追加 `"\n\n" + content`。
- [x] 主流程按 `next_start` 分支：
  - [x] `Some(line)` → `self.insert_before_line(line, content)`，`None` 时返回；
  - [x] `None` → `self.append_after_anchor(anchor_line, content)`。
- [x] 统一收尾：修正 `block_start` 后 `self.cursor = block_start + cursor`；按 `content` 计算 `start_line` / `end_line`；`self.scroll_lines_into_view(start_line, end_line)`。
- [x] 更新函数 doc：明确“下一个块不区分 `+++` / `&&&`；无下一个块走旧落点；前后各一空行；文首折叠前导空行；保留粘贴 chip”。

### P1.2 `src/app/prompt/panel.rs`

- [x] `slash_confirm` 的 `SuppleBlock` 分支调用改为 `self.insert_supple_block(&trigger, &content, cursor)`。
- [x] 确认 `self.record(EditKind::Other)` 仍在调用前执行，保证整次插入是一步撤销。
- [x] 不改其他分支。

### P1.3 注释同步

- [x] `src/app/markdown/slash.rs`：`SlashInsertion::SuppleBlock` 注释改为“清理命令文字后插到下一个块之前；无下一个块时追加到固定落点”。
- [x] `src/app/markdown/slash.rs`：`SlashTrigger`、`slash_insertion` 的“整行替换”措辞同步修改。
- [x] `src/app/markdown/slash.rs`：`slash_filter` 顶部 doc 中“确认时追加到固定落点”同步修改。
- [x] `src/app/prompt/panel.rs`：`slash_confirm` doc 区分普通命令与补充块。
- [x] `src/app/prompt/block.rs` 模块注释更新为“补充块操作：清理、删除与插入”（插入本来就在模块内，语义更准确）。
- [x] `docs/feature/prompt-block-restyle/design.md` §3.5 顶部加一行“落点规则已由 `docs/feature/prompt-supplement-insert-before-next/` 调整”。
- [x] `docs/feature/prompt-block-restyle/task.md` P1.4 顶部加同样一行说明。

## P2 测试

### P2.1 更新旧测试（`src/app/prompt/block/tests.rs`）

- [x] `supplement_typed_before_template_keeps_command_blank_line_and_appends_after_template`
  - 改名为 `supplement_before_template_inserts_before_template_at_document_start`
  - 断言：`p.text().starts_with("+++ supplement --start\n{title: }")`，无前导空行
  - 断言：`p.text().contains("+++ supplement --end\n\n&&& addTemplate --start")`
  - 断言：光标仍在 `{title: |}`。
- [x] `supplement_typed_inside_template_appends_after_last_existing_supplement`
  - 改名为 `supplement_inside_template_inserts_before_first_supple`
  - 断言：`spans` 顺序为 `Template, Supple, Supple, Supple`
  - 断言：新块位于 `&&& --end` 与 S1 之间
  - 断言：`&&&` 内命令空行保留。
- [x] `unclosed_block_hides_supplement_command_and_skips_append`
  - 直接把 `p.append_supple_block(...)` 改为 `p.insert_supple_block(...)`，其余断言不变。
- [x] `supplement_scrolls_viewport_so_new_block_start_line_is_visible`
  - 保持取首个 `Supple` span 的定位方式，确认新块在 `&&&` 之前时起始行仍滚入视口。

### P2.2 新增测试（`src/app/prompt/block/tests.rs`）

- [x] `supplement_inside_supple_inserts_before_next_supple`
  - 两个 `+++`，光标在第一个内部空行输入 `/supplement`
  - 断言 3 个 `+++`，新块在 S1 与 S2 之间
  - 断言 S1 内命令空行保留
  - 断言光标在 `{title: |}`
  - 一次 `undo` 恢复命令行与光标。
- [x] `supplement_before_second_template_inserts_before_it`
  - `+++` 在前、`&&&` 在后，光标在 `+++` 内
  - 断言新块插在 `&&&` 起始行之前，验证“任何块都算边界”。
- [x] `supplement_between_blocks_keeps_plain_text_above_new_block`
  - S1 → 普通文字 → S2，命令写在普通文字下方
  - 断言顺序仍为 S1 → 普通文字 → 新块 → S2。
- [x] `supplement_after_last_block_appends_after_last_supple`
  - 两个 `+++`，光标在最后一个内部
  - 断言新块追加在最后一个 `+++` 之后，不是插到前面。
- [x] `supplement_spacing_collapses_extra_blank_lines`
  - 下一个块前有 2 个以上空行
  - 断言插入后新块前后各一个空行，不出现连续多空行。
- [x] `custom_supple_block_uses_same_insert_position`
  - 用 `SlashItem::custom_supple` / `SlashInsertion::SuppleBlock` 触发同一路径
  - 断言落点与内置 `/supplement` 一致。
- [x] `supplement_at_document_start_with_plain_text_has_no_leading_blank`
  - 命令在文首、下面有普通文字；断言文首无空行。
- [x] `supplement_before_next_block_collapses_whitespace_only_lines`
  - 下一个块前有只含空格的行；断言折叠成一个空行。
- [x] `unclosed_block_guard_keeps_cursor_in_bounds`
  - 守卫提前返回后 `cursor <= text.len()` 且停在命令行末尾。
- [x] `supplement_after_pasted_chip_preserves_chip`
  - 命令行前有折叠粘贴 chip；断言 chip 保留。
- [x] `supplement_after_last_block_collapses_whitespace_only_tail`
  - 锚点后是只含空白的尾部；断言新块前恰好一个空行。
- [x] `supplement_without_anchor_skips_unclosed_fence`
  - 无块锚点且文末围栏未闭合；断言不插入。

### P2.3 保持不动的回归

- [x] `supplement_in_empty_document_replaces_entire_text_and_positions_cursor_in_title`
- [x] `supplement_with_trailing_plain_text_inserts_between_block_and_tail_with_blank_lines`
- [x] `supplement_when_anchor_already_followed_by_blank_line_does_not_double_blanks`
- [x] `slash_supplement_offered_inside_template_and_inserts_subblock`
- [x] `slash_title_in_supplement_after_template_includes_outer_task`
- [x] `slash_title_in_supple_of_second_template_includes_second_task`

## P3 校验

- [x] `cargo fmt`
- [x] 精确测试：
  - [x] `cargo test --lib app::prompt::block::tests`
  - [x] `cargo test --lib app::prompt::tests`
  - [x] `cargo test --test prompt_template_commands`
- [x] `cargo clippy --all-targets -- -D warnings`
- [x] `cargo test`：5429 passed、8 ignored；首次全量运行的 `lx_paste::wheel_scrolls_lx_input_viewport` 偶发失败与本改动无关，单测与复跑通过。
- [x] 检查 diff：确认没有修改 `src/ui/**`、`src/app/markdown/blocks.rs`、`src/app/markdown/supple.rs`、DB 迁移和配置。
- [x] 检查生产代码无新增 `unwrap()` / `expect()`。
- [x] 检查 `insert_supple_block` 内部没有额外 `record` 调用。

## P5 Review 修复（`review.md`）

- [x] P1.1 文首命令 + 普通文字：`at_document_start` + `trim_leading_blank_lines`，补测试。
- [x] P1.2 只含空白行：`last_content_end`，补测试。
- [x] P1.3 未闭合守卫光标越界：清命令后 `cursor = command_start`，补测试。
- [x] P2.1 粘贴 chip 被整行清理误删：只删 `/` + query，补测试。
- [x] P2.2 `insert_before_line` 越界回退：改为 `Option<usize>`。
- [x] P2.3 无锚点 + 未闭合围栏：`open_fence(&masked)` 守卫，补测试。
- [x] P2.4 旧锚点空白尾部：`first_content_line_start` / `last_content_end`，补测试。
- [x] P3.1 注释同步：`SlashTrigger`、`SlashInsertion::SuppleBlock`、`slash_insertion`、`slash_confirm`。
- [x] P3.2 文档同步：本 `task.md` 检查项、`design.md` 测试矩阵与实施记录。
- [x] P3.3 模块注释：保留“清理、删除与插入”，文档同步。
- [x] P3.4 已知限制：旧锚点语义、第二个 `&&&` 不识别，记录在 `review.md`，本次不改。

## P4 人工验收（用户执行）

- [ ] `cargo run`，构造两个 `+++` 块：
  - 光标在第一个块内输入 `/supplement` → 新块出现在第一个和第二个之间。
  - 光标在最后一个块内输入 `/supplement` → 新块追加到最后。
- [ ] 命令行确认后，原块内空行保留。
- [ ] 文首命令 + 普通文字 → 文档不以空行开头。
- [ ] 下一个块前有只含空格的行 → 只保留一个空行。
- [ ] 命令行前有粘贴 chip → chip 保留。
- [ ] 新块与前后内容各一个空行。
- [ ] 光标落在新块 `{title: }` 的 `}` 前。
- [ ] 新块在视口可见；块高于视口时至少起始行可见。
- [ ] 一次 `undo` 恢复命令行、删除新块。

## 完成定义

- 设计文档与实现一致；所有新增/更新测试通过。
- `cargo fmt`、`cargo clippy --all-targets -- -D warnings`、`cargo test` 全绿。
- 用户人工验收通过。
