# Review：`+++` 补充块插入落点

- 状态：P1–P3 已修复，并已合并到 `dev`（2026-11-13，合并提交 `88c16880`）。
- 审查对象：`feat/prompt-supplement-insert-before-next`，初始提交 `d8cf1448`
- 审查范围：`src/app/prompt/block.rs`、`src/app/prompt/block/tests.rs`、`src/app/prompt/panel.rs`、`src/app/markdown/slash.rs`、`src/app/markdown/blocks.rs` 及本目录 `design.md` / `task.md`
- 结论：主路径（下一个块定位、`&&&`/`+++` 边界、无 next 回退、撤销、滚动、空文档）成立；未发现 P0。修复 3 个 P1、4 个 P2，并同步 P3 文档问题。

## P0（阻塞）

无。未发现必现崩溃、数据丢失或主路径落点错误。

## P1（必须修复）

### P1.1 文首命令 + 普通文字时保留前导空行

- 关键点：命令在文档最前面，命令与下一个块之间还有普通文字时，确认后文档以空行开头。
- 复现输入：

```text
/supplement
plain note

+++ supplement --start
{title: S1}
+++ supplement --end
```

- 实际：`\nplain note\n\n+++ ...`，首行是命令清空后留下的空行。
- 期望：`plain note\n\n+++ ...`，普通文字在新块上方，文首无前导空行。
- 错误原因：`insert_before_line` 只用 `prefix.trim().is_empty()` 判断文首；一旦 prefix 含普通文字就走 else，`trim_end_matches('\n')` 不会删除最前面的 `\n`。`at_document_start` 只覆盖“整段 prefix 都是空白”的情况。
- 修复：`insert_supple_block` 在清命令前记录 `at_document_start`，插入成功后调用 `trim_leading_blank_lines`，按字节修正 `block_start` 与光标。
- 测试：`supplement_at_document_start_with_plain_text_has_no_leading_blank`。

### P1.2 只含空白的行未按空行折叠

- 关键点：下一个块前存在只含空格/制表符的行时，新块前会出现两层视觉空行。
- 复现输入：

```text
+++ supplement --start
{title: S1}

+++ supplement --end
   
+++ supplement --start
{title: S2}
+++ supplement --end
```

- 实际：`+++ supplement --end\n   \n\n+++ supplement --start`，`   ` 行与空行叠加。
- 期望：`+++ supplement --end\n\n+++ supplement --start`，恰好一个空行。
- 错误原因：`insert_before_line` 用 `prefix.trim_end_matches('\n')` 只跳过换行，不跳过只含空白的行；`base` 落在空白行之后，替换范围只剩最后一个 `\n`。
- 修复：新增 `last_content_end`，从最后一个非空白行的内容末端开始替换；保留该行自身的尾随空格。
- 测试：`supplement_before_next_block_collapses_whitespace_only_lines`。

### P1.3 未闭合块守卫提前返回时光标越界

- 关键点：`splice_text` 删除命令后文本变短，但 `cursor` 仍停在删除前的行尾；未闭合守卫直接返回时没有重置。
- 复现：未闭合 `&&&` 末尾输入 `/supplement`，直接调用 `insert_supple_block`；测试中 `cursor=51`、`text.len()=40`。
- 期望：守卫返回后 `cursor <= text.len()`，停在清空后的命令行末尾。
- 错误原因：清理命令与守卫返回之间没有光标收敛；正常插入路径后面会重设光标，异常路径不会。
- 修复：清命令后立即 `self.cursor = command_start`；正常路径后续再覆盖为标题位置。
- 测试：`unclosed_block_guard_keeps_cursor_in_bounds`。

## P2（应修复）

### P2.1 命令行清理会误删同一行的粘贴 chip

- 关键点：`slash_trigger` 基于 masked 文本取 `line_start..line_end`，清理却作用于 raw 文本；若命令行前面有折叠粘贴 chip，整行删除会连带删掉 chip 标签和正文。
- 复现：`<pasted-text ...>...</pasted-text>/supplement` 后确认。
- 期望：chip 原样保留，只删除 `/supplement`。
- 错误原因：`self.splice_text(trigger.line_start, trigger.line_end, "")` 删除整行，而触发区间来自遮罩文本。
- 修复：按 `trigger.query` 计算命令起点 `line_end - (query.len() + 1)`，只删 `/` + query；保留行首缩进与 chip。
- 测试：`supplement_after_pasted_chip_preserves_chip`。
- 备注：该缺陷在旧 `append_supple_block` 已存在，本分支路径同样触发；本次一并修复。

### P2.2 `insert_before_line` 行号越界回退会制造越界光标

- 关键点：`line_bounds(line)` 失败时返回 `self.text.len()` 当作 `block_start`，随后 `self.cursor = block_start + cursor` 可能超过文本长度。
- 期望：不插入，且光标保持在合法位置。
- 错误原因：用“文末偏移”冒充“插入点”，没有区分失败与成功。
- 修复：`insert_before_line` 返回 `Option<usize>`；`None` 时 `insert_supple_block` 直接返回（命令已清空、光标在命令行末尾）。
- 验证：类型层面消除该路径；全量 clippy/test 通过。

### P2.3 无锚点 + 未闭合代码围栏时插入到围栏内部

- 关键点：页面没有 `+++`/`&&&` 锚点，但文末有未闭合代码围栏；旧回退会追加到文末，新块落在围栏内部，解析器看不到。
- 复现：

```text
/supplement

```
code
```
```

- 实际：`+++` 被追加到 `code` 之后，仍在围栏内。
- 期望：不插入（或至少不落进围栏）。
- 错误原因：守卫只检查未闭合块，没有检查未闭合围栏；`append_after_anchor(None)` 总是追加文末。
- 修复：无 `next_start` 且无 `anchor_line` 时，若 `open_fence(&masked)` 为 `Some` 则直接返回；有锚点时锚点必在围栏之前，插入仍安全。面板可见性不变（遵守本任务非目标）。
- 测试：`supplement_without_anchor_skips_unclosed_fence`。

### P2.4 旧锚点后只含空白的尾部未折叠

- 关键点：命令在最后一个块之后，锚点与文末之间只有空白行/空格行时，旧回退会留下多余空行。
- 复现：`+++ ... --end\n   \n` 后输入 `/supplement`。
- 实际：`--end\n\n+++ ...\n\n   \n`（新块后还挂着空白行）。
- 期望：`--end\n\n+++ ...`。
- 错误原因：`append_after_anchor` 只统计紧跟锚点的连续 `\n`；`tail.is_empty()` 不把只含空白的 tail 当空。
- 修复：新增 `first_content_line_start` 找到锚点后第一个内容行；没有内容行时用 `last_content_end` 从最后一个非空白行末尾追加。无锚点分支同样改用 `last_content_end`。
- 测试：`supplement_after_last_block_collapses_whitespace_only_tail`。

## P3（文档与已知限制）

### P3.1 注释与实现不一致

- `SlashInsertion::SuppleBlock` 仍写“整行替换”，`slash_insertion` 仍写“整行替换文本”，`slash_confirm` 仍写“内置/自定义命令整行替换”，`SlashTrigger` 仍写“整行替换范围”。
- 已同步为“清理命令文字后按落点插入，保留行内其他内容”。

### P3.2 `task.md` / `design.md` 状态与测试矩阵过期

- `task.md` 顶部写“已实施”，P0–P3 却全部未勾选；`design.md` §6.2 的测试名与实际不一致（如 `supplement_before_template_inserts_before_template` 实际为 `supplement_before_second_template_inserts_before_it`），且重复列出已更新测试。
- 已更新检查项、测试名、新增测试数量与实施记录。

### P3.3 模块注释与任务描述的差异

- 实现把模块注释从“清理与删除”改为“清理、删除与插入”；任务 P1.3 原要求“保持”。插入本来就在模块内，新注释更准确；保留代码并修正文档，不回退注释。

### P3.4 已知限制（本次不改）

- 无 next 时旧锚点优先“最后一个 `+++`”，其次“首个 `&&&`”；若命令在 `&&&` 内且该 `&&&` 上方有 `+++`，新块会落在 `+++` 之后、`&&&` 之前。这是任务明确保留的旧行为，不属于回归。
- `block_scan` 只识别首个 `&&&`；第二个 `&&&` 不作为落点边界。设计 §8.2 已列为非目标，本次不改。

## 修复后验证

- `cargo fmt` 无差异。
- `cargo clippy --all-targets -- -D warnings` 通过。
- `cargo test --lib app::prompt::block::tests`：31 passed。
- `cargo test --lib app::prompt::tests`：156 passed。
- `cargo test --test prompt_template_commands`：6 passed。
- `cargo test` 全量：5429 passed、8 ignored。首次全量运行出现过一次与本改动无关的 `tests/lx_paste.rs::wheel_scrolls_lx_input_viewport` 偶发失败，单测与全量复跑均通过。
