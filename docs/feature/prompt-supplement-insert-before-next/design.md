# Prompt `+++` 补充块插入落点调整

- 状态：已实施并完成 review 修复（2026-11-13），自动化校验通过，待用户人工验收
- 审查记录：`docs/feature/prompt-supplement-insert-before-next/review.md`
- 任务目录：`docs/feature/prompt-supplement-insert-before-next/`
- 影响层：`app`（纯状态编辑），不改 UI 渲染、DB、网络
- 关联旧文档：`docs/feature/prompt-block-restyle/design.md` §3.5 的“固定追加落点”由本任务调整

## 1. 背景与现状

当前 `/supplement` 与自定义 `block: supple` 都走 `Prompt::append_supple_block`（`src/app/prompt/block.rs:79`）。该函数完全不看命令行/光标位置，只按固定锚点追加：

```rust
// src/app/prompt/block.rs（现状）
let anchor_line = scan
    .spans
    .iter()
    .rev()
    .find(|span| span.kind == BlockKind::Supple && span.closed)
    .or_else(|| {
        scan.spans
            .iter()
            .find(|span| span.kind == BlockKind::Template && span.closed)
    })
    .map(|span| span.end);
```

由此产生的问题：

```text
+++ supplement --start
{title: S1}
             ← 光标在这里输入 /supplement
+++ supplement --end

+++ supplement --start
{title: S2}
+++ supplement --end
```

确认后新块被追加到 S2 之后，而不是插在“下一个块”S2 之前。命令写在第一个块内部、写在两个块之间、写在最后一个块内部，结果完全相同。

块扫描事实（`src/app/markdown/blocks.rs::block_scan`）：

- 只识别首个 `&&&` 任务块（`template_seen` 之后不再识别第二个 `&&&`）。
- `+++` 补充块是顶层平级块，按文档顺序产出 `spans`。
- `spans` 顺序即文档顺序；未闭合块排在最后并带 `closed == false`。
- `mask_pasted_text` 保留字节长度与换行，遮罩后的行号/字节偏移与原文一致。

## 2. 目标与非目标

### 目标

1. 光标/命令所在行之后存在块时，新 `+++` 插到“下一个块”的起始行之前。
2. 下一个块不区分类型：`+++` 和 `&&&` 都算。
3. 后续没有块时，保持旧的固定追加落点，不改变已验证行为。
4. 新块前后各保留一个空行；多余空行折叠；文首不额外补前导空行。
5. 光标落在新块 `{title: }` 的 `}` 前，滚动到新块可见。
6. 整个“清命令 + 插块”仍然是一步撤销。
7. 保留“页面存在未闭合块时不插入”的兜底守卫。

### 非目标

- 不放开第二个 `&&&` 模板块，不修改 `template_present` 过滤。
- 不修改 `+++` 的解析、渲染、折叠、清理、删除、worktree 绑定。
- 不修改 `&&&` 模板命令（`/add`、`/bug` 等）的插入逻辑。
- 不修改斜杠面板候选可见性、排序和 UI 文案。
- 不新增配置项、不改数据库。

## 3. 已确认边界（用户逐项确认）

| # | 决策点 | 结论 |
| --- | --- | --- |
| 1 | 目标块类型 | 只改 `+++` 补充块（`/supplement`、自定义 `block: supple`），不动 `&&&` |
| 2 | “下一个块”的类型 | `+++` 与 `&&&` 按文档顺序同等参与 |
| 3 | 后面没有块 | 维持旧落点：最后一个已闭合 `+++` 结束行之后 → 已闭合 `&&&` 结束行之后 → 文末 |
| 4 | 当前块与下一个块之间夹普通文字 | 新块紧贴下一个块起始行上方，普通文字留在新块上方 |
| 5 | 命令写在文档最前面、上方没有块 | 仍然插到下方第一个块之前 |
| 6 | 命令所在行 | 只清空命令文字，保留该空行（块内语义不变） |
| 7 | 空行 | 新块前后各一个空行；多余空行折叠；文首不额外补前导空行 |
| 8 | 存在未闭合块 | 保持禁止插入：面板隐藏候选，底层只清命令文字 |

## 4. 新落点规则

### 4.1 定义

1. 先清空命令行文字，保留该行（可能与现状一样留下空行）。
2. 在清空后的文本上执行 `block_scan`。
3. 存在任意未闭合块 → 直接返回，不插入。
4. `command_line = self.text[..trigger.line_start].matches('\n').count()`。
5. `next = spans.iter().find(|span| span.start > command_line)`。
6. `next` 存在 → 在 `next.start` 行首之前插入新块。
7. `next` 不存在 → 走旧锚点追加。

### 4.2 示例

光标在第一个 `+++` 内部，后面还有第二个 `+++`：

```text
+++ supplement --start
{title: S1}
/supplement              ← 命令在这里
+++ supplement --end

+++ supplement --start
{title: S2}
+++ supplement --end
```

结果：

```text
+++ supplement --start
{title: S1}

+++ supplement --end

+++ supplement --start   ← 新块插在 S2 之前
{title: }

## Supplementary Requirements
...
+++ supplement --end

+++ supplement --start
{title: S2}
+++ supplement --end
```

命令行清空后的空行留在 S1 内；新块与 S1、S2 各隔一个空行。

光标在 `&&&` 内部，后面有两个 `+++`：

```text
&&& addTemplate --start
{title: T}
/supplement              ← 命令在任务块内部
&&& addTemplate --end

+++ supplement --start
{title: S1}
+++ supplement --end

+++ supplement --start
{title: S2}
+++ supplement --end
```

结果新块插在 S1 之前（紧挨 `&&&` 之后），而不是 S2 之后。

命令写在第一个块之前、上方没有任何块：

```text
/supplement              ← 命令在最上方

+++ supplement --start
{title: S1}
+++ supplement --end
```

结果新块插在 S1 之前；文首不保留前导空行。

### 4.3 空行规则

- 新块前有普通内容：从最后一个非空白行的内容末端开始替换到下一个块行首，统一为 `\n\n`，即恰好一个空行；只含空格/制表符的行按空行处理。
- 新块后就是下一个块：正文末尾统一补 `\n\n`，即恰好一个空行。
- 文首命令（命令行前面只有空白）：清命令后折叠命令留下的前导空行，即使命令与下一个块之间还有普通文字；普通文字保留在新块上方。
- 文末追加（旧落点）：从最后一个非空白行的内容末端开始规整，锚点后只含空白的尾部也折叠；不额外制造双空行。

> 说明：命令在块内时，“保留命令空行”优先；命令在文首时，这段空行会成为前导空行，按文首规则折叠。

### 4.4 命令行清理范围

- 普通斜杠命令：整行替换。
- `+++` 补充块：只删除 `/` + `query`，保留行首缩进和同一行已有的折叠粘贴 chip；随后 `cursor` 落到清理后的命令行末尾。
- 未闭合块/未闭合围栏守卫提前返回时，光标也必须停在合法位置。

## 5. 关键实现

### 5.1 重命名

`Prompt::append_supple_block` 更名为 `Prompt::insert_supple_block`，因为“追加”已不能描述新语义。调用点只有一处：

```rust
// src/app/prompt/panel.rs:232
if matches!(item.insertion, markdown::SlashInsertion::SuppleBlock { .. }) {
    self.insert_supple_block(&trigger, &content, cursor);
} else {
    ...
}
```

测试里的直接调用 `src/app/prompt/block/tests.rs` 同步改名。

### 5.2 `src/app/prompt/block.rs` 核心流程

```rust
// 关键决策；完整实现见 src/app/prompt/block.rs。
let at_document_start = self.text[..trigger.line_start].trim().is_empty();
// 只删 `/` + query，保留行首缩进与粘贴 chip。
let command_start = trigger
    .line_end
    .saturating_sub(trigger.query.len() + 1)
    .max(trigger.line_start);
self.splice_text(command_start, trigger.line_end, "");
self.cursor = command_start;

let masked = mask_pasted_text(&self.text);
let scan = markdown::block_scan(&masked);
if scan.spans.iter().any(|span| !span.closed) {
    return;
}

let command_line = self.text[..trigger.line_start].matches('\n').count();
let next_start = scan
    .spans
    .iter()
    .find(|span| span.start > command_line)
    .map(|span| span.start);
let anchor_line = scan
    .spans
    .iter()
    .rev()
    .find(|span| span.kind == BlockKind::Supple && span.closed)
    .or_else(|| {
        scan.spans
            .iter()
            .find(|span| span.kind == BlockKind::Template && span.closed)
    })
    .map(|span| span.end);

// 无块锚点时只能追加文末；文末在未闭合围栏内会吞掉新块。
if next_start.is_none() && anchor_line.is_none() && markdown::open_fence(&masked).is_some() {
    return;
}
let mut block_start = match next_start {
    Some(line) => match self.insert_before_line(line, content) {
        Some(start) => start,
        None => return,
    },
    None => self.append_after_anchor(anchor_line, content),
};
if at_document_start {
    block_start = block_start.saturating_sub(self.trim_leading_blank_lines());
}
self.cursor = block_start + cursor;
self.scroll_lines_into_view(start_line, end_line);
```

辅助函数：

- `insert_before_line` 返回 `Option<usize>`；用 `last_content_end` 从最后一个非空白行内容末端替换，保留该行尾随空格并折叠只含空白的行。
- `append_after_anchor` 用 `first_content_line_start` 找锚点后的第一个内容行；没有内容行时用 `last_content_end` 从最后一个非空白行末尾追加。
- `trim_leading_blank_lines` 删除文首连续空白行并返回删除字节数。

### 5.3 关键点

1. **先清命令再扫描**：命令行清理后再定位，行号不会因删除文字而偏移；只删 `/` + query，保留缩进和粘贴 chip。
2. **`command_line` 而不是 `cursor`**：命令确认时光标在行尾，两者在同一行；用行号比较更直接。
3. **`span.start > command_line`**：自然跳过“命令所在的当前块”，取物理上下一个块。
4. **不借用 `scan` 跨可变借用**：`next_start` / `anchor_line` 都在调用 `splice_text` 前算成 `usize`/`Option<usize>`，避免 `masked` 的不可变借用与 `self.splice_text` 冲突。
5. **两次 `splice_text` 不会产生两条撤销**：调用方 `slash_confirm` 在调用前已经 `self.record(EditKind::Other)`，内部只改文本。
6. **光标偏移**：`block_start` 是正文起点，`cursor` 是 `supple_block_content` 相对正文的偏移，相加后正好落在 `{title: }` 的 `}` 前。
7. **滚动**：沿用 `scroll_lines_into_view(start_line, end_line)`，新块高于视口时至少保证起始行可见。
8. **文首折叠**：`at_document_start` 在清命令前判定；插入成功后 `trim_leading_blank_lines` 删除文首空白行，并按删除字节数修正 `block_start`。
9. **只含空白的行**：`last_content_end` 找最后一个非空白行内容末端，保留该行尾随空格；`first_content_line_start` 找锚点后的第一个内容行；两者共同保证恰好一个空行。
10. **越界安全**：`insert_before_line` 返回 `Option<usize>`，行号失效时直接放弃插入；守卫提前返回时 `cursor` 已在命令行末尾，不会越界。
11. **未闭合围栏**：无下一个块且无锚点时，若 `open_fence(&masked)` 存在则跳过插入，避免新块被围栏吞掉；面板可见性不变。
12. **粘贴 chip**：触发区间来自 masked 文本，清理只作用于 raw 文本中的命令子区间，避免整行删除 chip。

### 5.4 注释同步

- `src/app/markdown/slash.rs`：`SlashInsertion::SuppleBlock` 改为“清理命令文字后插到下一个块之前；无下一个块时追加到固定落点”，并同步 `SlashTrigger`、`slash_insertion` 的“整行替换”措辞。
- `src/app/prompt/panel.rs`：`slash_confirm` 的 doc 区分“普通命令整行替换”与“补充块按落点插入”。
- `src/app/prompt/block.rs`：模块注释更新为“清理、删除与插入”；`insert_supple_block` 的 doc 完整描述新规则。
- `docs/feature/prompt-block-restyle/design.md` §3.5 与 `task.md` P1.4 是历史记录，在对应段落顶部加一行“落点规则已由 `docs/feature/prompt-supplement-insert-before-next/` 调整”，不重写历史内容。

## 6. 测试矩阵

### 6.1 更新现有测试（旧行为必须改）

| 现有测试 | 变更 |
| --- | --- |
| `supplement_typed_before_template_keeps_command_blank_line_and_appends_after_template` | 改名为 `supplement_before_template_inserts_before_template_at_document_start`；断言文首无前导空行、新块在 `&&&` 之前 |
| `supplement_typed_inside_template_appends_after_last_existing_supplement` | 改名为 `supplement_inside_template_inserts_before_first_supple`；断言新块是 `spans[1]`，位于 `&&&` 与 S1 之间 |
| `unclosed_block_hides_supplement_command_and_skips_append` | 直接调用改为 `insert_supple_block`，断言不变 |
| `supplement_scrolls_viewport_so_new_block_start_line_is_visible` | 保持首个 Supple span 定位；确认新块在 `&&&` 之前时滚动仍生效 |

### 6.2 新增测试

1. `supplement_inside_supple_inserts_before_next_supple`
   - 两个 `+++`，光标在第一个内部；确认后 3 个 `+++`，新块位于 S1 与 S2 之间。
   - 断言 S1 内命令空行保留、光标在 `{title: |}`、一次 `undo` 恢复命令行与光标。
2. `supplement_before_second_template_inserts_before_it`
   - `+++` 在前、`&&&` 在后；断言新块插在 `&&&` 起始行之前。
3. `supplement_between_blocks_keeps_plain_text_above_new_block`
   - S1 → 普通文字 → S2；断言顺序仍为 S1 → 普通文字 → 新块 → S2。
4. `supplement_after_last_block_appends_after_last_supple`
   - 两个 `+++`，光标在最后一个内部；断言新块追加在最后一个 `+++` 之后。
5. `supplement_spacing_collapses_extra_blank_lines`
   - 下一个块前有 2 个以上空行；断言新块前后各一个空行。
6. `custom_supple_block_uses_same_insert_position`
   - 用 `SlashItem::custom_supple` / `SlashInsertion::SuppleBlock` 触发同一路径。
7. `supplement_at_document_start_with_plain_text_has_no_leading_blank`
   - 命令在文首、下面有普通文字；断言文首无空行，普通文字在新块上方。
8. `supplement_before_next_block_collapses_whitespace_only_lines`
   - 下一个块前有只含空格的行；断言折叠成一个空行。
9. `unclosed_block_guard_keeps_cursor_in_bounds`
   - 守卫提前返回后断言 `cursor <= text.len()` 且停在命令行末尾。
10. `supplement_after_pasted_chip_preserves_chip`
    - 命令行前有折叠粘贴 chip；断言 chip 保留，只删命令文字。
11. `supplement_after_last_block_collapses_whitespace_only_tail`
    - 锚点后是只含空白的尾部；断言新块前恰好一个空行。
12. `supplement_without_anchor_skips_unclosed_fence`
    - 无块锚点且文末围栏未闭合；断言不插入，避免新块被围栏吞掉。

### 6.3 保持不变的回归

- 空文档插入：全文就是 `+++` 块，光标在标题内。
- 只有一个 `&&&` 且命令在块内：无下一个块，追加到 `&&&` 之后。
- 最后一个 `+++` 之后有普通正文：旧落点插在块与正文之间，前后各一空行。
- 未闭合块：面板不提供 `/supplement`，底层不插入。
- `/title` 在 `+++` 内仍取最近前置 `&&&` 正文，不受落点影响。

## 7. 受影响文件

| 文件 | 变更 |
| --- | --- |
| `src/app/prompt/block.rs` | 重命名函数，新增 `next_start` 分支、`insert_before_line`、`append_after_anchor`、`trim_leading_blank_lines`、`last_content_end`、`first_content_line_start` |
| `src/app/prompt/panel.rs` | 调用点改名，同步 `slash_confirm` doc |
| `src/app/prompt/block/tests.rs` | 更新旧测试，新增 12 个落点/空行/自定义命令/守卫测试 |
| `src/app/markdown/slash.rs` | 同步 `SlashTrigger`、`SlashInsertion::SuppleBlock`、`slash_insertion` 注释；候选过滤逻辑未改 |
| `docs/feature/prompt-supplement-insert-before-next/review.md` | 本次 review 结论与 P0–P3 修复记录 |
| `docs/feature/prompt-block-restyle/design.md` | §3.5 加一行“已被本任务调整”的说明（不改历史内容） |
| `docs/feature/prompt-block-restyle/task.md` | P1.4 加一行同上说明 |

不改：`src/ui/**`、`src/app/markdown/blocks.rs`、`src/app/markdown/supple.rs`、DB、配置。

## 8. 风险与回归控制

1. **旧测试故意失效**：现有两个测试断言“追加到最后一个 `+++` 之后”，这是本次要改变的行为；更新测试而不是保留旧断言。
2. **`block_scan` 只识别首个 `&&&`**：不要为了“下一个块”去改扫描器；现有 `spans` 已满足文档顺序。若未来放开多 `&&&`，本规则仍按 `spans` 自然工作。
3. **空行规整范围**：只规整插入点前后到相邻内容的空行，不全局重排文档；只含空白的行按空行处理，保留最后一行自身的尾随空格。
4. **命令行清理**：块内空行语义保留；只删 `/` + query，保留缩进和粘贴 chip；文首命令按 4.3 折叠前导空行。
5. **未闭合守卫**：未闭合块必须先于 `next_start` 判断返回；无锚点且文末存在未闭合围栏时也跳过插入，避免新块被围栏吞掉。
6. **单步撤销**：`insert_supple_block` 内部不得再调用 `record`，否则会多出撤销步。
7. **生产代码约束**：新增代码不得使用 `unwrap()` / `expect()`；`insert_before_line` 用 `Option` 处理行号失效，守卫返回前先收敛光标。
8. **粘贴 chip**：触发区间来自 masked 文本，清理必须基于 raw 文本中的命令子区间，不能整行 `splice_text`。
9. **文首折叠**：`at_document_start` 只在命令前全是空白时成立；`trim_leading_blank_lines` 只删文首空白行，不动普通文字缩进。

## 9. 验收标准

- 6.2 全部新增测试通过，6.1 更新后的旧测试通过。
- `cargo fmt` 无差异。
- `cargo clippy --all-targets -- -D warnings` 通过。
- `cargo test` 全量通过。
- 用户运行 `cargo run` 人工确认：
  - 两个 `+++`，命令写在第一个内部 → 新块出现在两者之间；
  - 命令写在最后一个 `+++` 内部 → 新块追加在最后；
  - 命令行清空后块内空行保留；
  - 文首命令 + 普通文字 → 文档不以空行开头；
  - 下一个块前有只含空格的行 → 只保留一个空行；
  - 命令行前有粘贴 chip → chip 保留；
  - 新块前后各一个空行，光标在标题内。

## 10. 实施记录（2026-11-13）

- 首轮实现：`append_supple_block` 更名为 `insert_supple_block`；新增 `next_start` 分支、`insert_before_line` 与 `append_after_anchor`；更新 2 个旧测试，新增 6 个落点/空行/自定义命令测试。
- Review 修复（`review.md`）：
  - P1：文首命令 + 普通文字前导空行、只含空白行未折叠、未闭合守卫光标越界。
  - P2：粘贴 chip 被整行清理误删、`insert_before_line` 越界回退、无锚点未闭合围栏、旧锚点空白尾部。
  - P3：`SlashTrigger` / `SlashInsertion::SuppleBlock` / `slash_insertion` / `slash_confirm` 注释、`task.md` 检查项与测试矩阵同步。
- 新增辅助函数：`trim_leading_blank_lines`、`last_content_end`、`first_content_line_start`；`insert_before_line` 改为返回 `Option<usize>`。
- 未改动 `src/ui/**`、`src/app/markdown/blocks.rs`、`src/app/markdown/supple.rs`、DB 迁移与配置。
- 验证：`cargo fmt` 无差异；`cargo clippy --all-targets -- -D warnings` 通过；`cargo test` 全量 5429 passed、8 ignored；精确测试 `block::tests` 31 passed、`prompt::tests` 156 passed、`prompt_template_commands` 6 passed。首次全量运行出现过一次与本改动无关的 `lx_paste::wheel_scrolls_lx_input_viewport` 偶发失败，单测与全量复跑均通过。
