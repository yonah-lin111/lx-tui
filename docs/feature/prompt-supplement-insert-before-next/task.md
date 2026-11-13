# 任务：`+++` 补充块插入落点调整

- 设计文档：`docs/feature/prompt-supplement-insert-before-next/design.md`
- 状态：已实施（2026-11-13），自动化校验通过，待用户人工验收
- 执行原则：只改 `app` 纯状态路径；不碰 UI 渲染、DB、配置；每步完成后做精确校验
- 实施结果：P0–P3 已完成；`cargo fmt`、`cargo clippy --all-targets -- -D warnings`、`cargo test` 全部通过（5423 passed，8 ignored；另有一次全量运行出现与本改动无关的 shell 测试偶发失败，单测与复跑均通过）。P4 待用户 `cargo run` 人工验收。

## P0 前置核对

- [ ] 确认工作区在 `lx-tui` 主分支预期分支，`git status` 无与本任务冲突的未提交改动。
- [ ] 重新读取 `src/app/prompt/block.rs`、`src/app/prompt/panel.rs`、`src/app/markdown/blocks.rs`，确认行号与设计文档一致；若已变化，先更新设计文档再写代码。
- [ ] 确认 `block_scan` 仍只识别首个 `&&&`、`+++` 按文档顺序产出 `spans`。

## P1 核心实现

### P1.1 `src/app/prompt/block.rs`

- [ ] 把 `pub(super) fn append_supple_block` 重命名为 `pub(super) fn insert_supple_block`。
- [ ] 保留第一步 `self.splice_text(trigger.line_start, trigger.line_end, "")`，即只清命令文字、保留空行。
- [ ] 保留未闭合块守卫：`scan.spans.iter().any(|span| !span.closed)` 时直接返回。
- [ ] 计算 `command_line = self.text[..trigger.line_start].matches('\n').count()`。
- [ ] 计算 `next_start = scan.spans.iter().find(|span| span.start > command_line).map(|span| span.start)`。
- [ ] 计算旧锚点 `anchor_line`：最后一个已闭合 `+++` 的 `end` → 首个已闭合 `&&&` 的 `end` → `None`。
- [ ] 在调用 `splice_text` 前把 `next_start` / `anchor_line` 固化为 `usize` / `Option<usize>`，避免 `masked` 借用冲突。
- [ ] 新增 `fn insert_before_line(&mut self, line: usize, content: &str) -> usize`：
  - [ ] `line_bounds(line)` 取行首字节；越界返回 `self.text.len()`。
  - [ ] `prefix.trim().is_empty()` 时清掉 `0..line_start`，插入 `content + "\n\n"`，返回 `0`。
  - [ ] 否则 `base = prefix.trim_end_matches('\n').len()`，替换 `base..line_start` 为 `"\n\n" + content + "\n\n"`，返回 `base + 2`。
- [ ] 新增 `fn append_after_anchor(&mut self, anchor_line: Option<usize>, content: &str) -> usize`，逐字保留现有追加逻辑：
  - [ ] 有锚点且锚点后还有内容：替换锚点行尾的换行串为 `"\n\n" + content + "\n\n"`，返回 `anchor_end + 2`。
  - [ ] 有锚点且锚点后为空：去掉文末换行后追加 `"\n\n" + content`，返回 `base + 2`。
  - [ ] 无锚点且文本为空：`content` 占全文，返回 `0`。
  - [ ] 无锚点且文本非空：去掉文末换行后追加 `"\n\n" + content`，返回 `base + 2`。
- [ ] 主流程按 `next_start` 分支：
  - [ ] `Some(line)` → `self.insert_before_line(line, content)`；
  - [ ] `None` → `self.append_after_anchor(anchor_line, content)`。
- [ ] 统一收尾：`self.cursor = block_start + cursor`；按 `content` 计算 `start_line` / `end_line`；`self.scroll_lines_into_view(start_line, end_line)`。
- [ ] 更新函数 doc：明确“下一个块不区分 `+++` / `&&&`；无下一个块走旧落点；前后各一空行；文首折叠前导空行”。

### P1.2 `src/app/prompt/panel.rs`

- [ ] `slash_confirm` 的 `SuppleBlock` 分支调用改为 `self.insert_supple_block(&trigger, &content, cursor)`。
- [ ] 确认 `self.record(EditKind::Other)` 仍在调用前执行，保证整次插入是一步撤销。
- [ ] 不改其他分支。

### P1.3 注释同步

- [ ] `src/app/markdown/slash.rs`：`SlashInsertion::SuppleBlock` 注释改为“确认时插到下一个块之前；无下一个块时追加到固定落点”。
- [ ] `src/app/markdown/slash.rs`：`slash_filter` 顶部 doc 中“确认时追加到固定落点”同步修改。
- [ ] `src/app/prompt/block.rs` 模块注释保持“补充块操作：清理与删除”，不扩大职责描述。
- [ ] `docs/feature/prompt-block-restyle/design.md` §3.5 顶部加一行“落点规则已由 `docs/feature/prompt-supplement-insert-before-next/` 调整”。
- [ ] `docs/feature/prompt-block-restyle/task.md` P1.4 顶部加同样一行说明。

## P2 测试

### P2.1 更新旧测试（`src/app/prompt/block/tests.rs`）

- [ ] `supplement_typed_before_template_keeps_command_blank_line_and_appends_after_template`
  - 改名为 `supplement_before_template_inserts_before_template_at_document_start`
  - 断言：`p.text().starts_with("+++ supplement --start\n{title: }")`，无前导空行
  - 断言：`p.text().contains("+++ supplement --end\n\n&&& addTemplate --start")`
  - 断言：光标仍在 `{title: |}`。
- [ ] `supplement_typed_inside_template_appends_after_last_existing_supplement`
  - 改名为 `supplement_inside_template_inserts_before_first_supple`
  - 断言：`spans` 顺序为 `Template, Supple, Supple, Supple`
  - 断言：新块位于 `&&& --end` 与 S1 之间
  - 断言：`&&&` 内命令空行保留。
- [ ] `unclosed_block_hides_supplement_command_and_skips_append`
  - 直接把 `p.append_supple_block(...)` 改为 `p.insert_supple_block(...)`，其余断言不变。
- [ ] `supplement_scrolls_viewport_so_new_block_start_line_is_visible`
  - 保持取首个 `Supple` span 的定位方式，确认新块在 `&&&` 之前时起始行仍滚入视口。

### P2.2 新增测试（`src/app/prompt/block/tests.rs`）

- [ ] `supplement_inside_supple_inserts_before_next_supple`
  - 两个 `+++`，光标在第一个内部空行输入 `/supplement`
  - 断言 3 个 `+++`，新块在 S1 与 S2 之间
  - 断言 S1 内命令空行保留
  - 断言光标在 `{title: |}`
  - 一次 `undo` 恢复命令行与光标。
- [ ] `supplement_before_second_template_inserts_before_it`
  - `+++` 在前、`&&&` 在后，光标在 `+++` 内
  - 断言新块插在 `&&&` 起始行之前，验证“任何块都算边界”。
- [ ] `supplement_between_blocks_keeps_plain_text_above_new_block`
  - S1 → 普通文字 → S2，命令写在普通文字下方
  - 断言顺序仍为 S1 → 普通文字 → 新块 → S2。
- [ ] `supplement_after_last_block_appends_after_last_supple`
  - 两个 `+++`，光标在最后一个内部
  - 断言新块追加在最后一个 `+++` 之后，不是插到前面。
- [ ] `supplement_spacing_collapses_extra_blank_lines`
  - 下一个块前有 2 个以上空行
  - 断言插入后新块前后各一个空行，不出现连续多空行。
- [ ] `custom_supple_block_uses_same_insert_position`
  - 用 `SlashItem::custom_supple` / `SlashInsertion::SuppleBlock` 触发同一路径
  - 断言落点与内置 `/supplement` 一致。

### P2.3 保持不动的回归

- [ ] `supplement_in_empty_document_replaces_entire_text_and_positions_cursor_in_title`
- [ ] `supplement_with_trailing_plain_text_inserts_between_block_and_tail_with_blank_lines`
- [ ] `supplement_when_anchor_already_followed_by_blank_line_does_not_double_blanks`
- [ ] `slash_supplement_offered_inside_template_and_inserts_subblock`
- [ ] `slash_title_in_supplement_after_template_includes_outer_task`
- [ ] `slash_title_in_supple_of_second_template_includes_second_task`

## P3 校验

- [ ] `cargo fmt`
- [ ] 精确测试：
  - [ ] `cargo test --lib app::prompt::block::tests`
  - [ ] `cargo test --lib app::prompt::tests`
  - [ ] `cargo test --test prompt_template_commands`
- [ ] `cargo clippy --all-targets -- -D warnings`
- [ ] `cargo test`
- [ ] 检查 diff：确认没有修改 `src/ui/**`、`src/app/markdown/blocks.rs`、`src/app/markdown/supple.rs`、DB 迁移和配置。
- [ ] 检查生产代码无新增 `unwrap()` / `expect()`。
- [ ] 检查 `insert_supple_block` 内部没有额外 `record` 调用。

## P4 人工验收（用户执行）

- [ ] `cargo run`，构造两个 `+++` 块：
  - 光标在第一个块内输入 `/supplement` → 新块出现在第一个和第二个之间。
  - 光标在最后一个块内输入 `/supplement` → 新块追加到最后。
- [ ] 命令行确认后，原块内空行保留。
- [ ] 新块与前后内容各一个空行。
- [ ] 光标落在新块 `{title: }` 的 `}` 前。
- [ ] 新块在视口可见；块高于视口时至少起始行可见。
- [ ] 一次 `undo` 恢复命令行、删除新块。

## 完成定义

- 设计文档与实现一致；所有新增/更新测试通过。
- `cargo fmt`、`cargo clippy --all-targets -- -D warnings`、`cargo test` 全绿。
- 用户人工验收通过。
