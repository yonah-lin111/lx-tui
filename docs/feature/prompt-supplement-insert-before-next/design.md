# Prompt `+++` 补充块插入落点调整

- 状态：已实施（2026-11-13），自动化校验通过，待用户人工验收
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

- 新块前有普通内容：替换“前文末尾到下一个块行首”的连续换行为 `\n\n`，即恰好一个空行。
- 新块后就是下一个块：正文末尾统一补 `\n\n`，即恰好一个空行。
- 文首只有空行（包括命令清空后留下的空行）：清掉前导空行，新块从第 0 行开始。
- 文末追加（旧落点）：沿用现有 `append_supple_block` 的换行规整逻辑，不额外制造双空行。

> 说明：命令在块内时，“保留命令空行”优先；只有当这段空行恰好位于文首、会成为前导空行时，才按文首规则折叠。

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

测试里唯一的直接调用 `src/app/prompt/block/tests.rs:196` 同步改名。

### 5.2 `src/app/prompt/block.rs` 核心代码

```rust
/// 清空斜杠命令行内容（保留该空行）后，把补充块插到下一个块之前；
/// 没有下一个块时按旧落点追加；调用方已记录撤销快照。
///
/// 下一个块按文档顺序取起始行在命令行之后的第一个已闭合块（`&&&` 与 `+++`
/// 同等参与）；有下一个块时插在其起始行之前，无下一个块时追加到最后一个
/// `+++` 结束行之后 → `&&&` 结束行之后 → 文末。新块与前后内容各隔一个空行，
/// 光标落在新块标题值（`{title: }` 的 `}` 前）。
pub(super) fn insert_supple_block(
    &mut self,
    trigger: &markdown::SlashTrigger,
    content: &str,
    cursor: usize,
) {
    self.splice_text(trigger.line_start, trigger.line_end, "");

    let masked = mask_pasted_text(&self.text);
    let scan = markdown::block_scan(&masked);
    // 页内存在未闭合块时不插入：落点会落在未闭合块内部，新块会被吞掉或误闭合前一块。
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

    let block_start = match next_start {
        Some(line) => self.insert_before_line(line, content),
        None => self.append_after_anchor(anchor_line, content),
    };

    self.cursor = block_start + cursor;
    let start_line = self.text[..block_start].matches('\n').count();
    let end_line = start_line + content.matches('\n').count();
    self.scroll_lines_into_view(start_line, end_line);
}

/// 在逻辑行 `line` 起始位置插入块正文，前后各保留一个空行；返回正文起始字节偏移。
///
/// 文首只有空行时清掉前导空行，避免文档以空行开头。
fn insert_before_line(&mut self, line: usize, content: &str) -> usize {
    let Some((line_start, _)) = self.line_bounds(line) else {
        return self.text.len();
    };
    let prefix = &self.text[..line_start];
    if prefix.trim().is_empty() {
        self.splice_text(0, line_start, &format!("{content}\n\n"));
        0
    } else {
        let base = prefix.trim_end_matches('\n').len();
        self.splice_text(base, line_start, &format!("\n\n{content}\n\n"));
        base + 2
    }
}

/// 无下一个块时的旧落点：最后一个已闭合 `+++` 结束行之后 → 首个已闭合 `&&&`
/// 结束行之后 → 文末；返回正文起始字节偏移。
fn append_after_anchor(&mut self, anchor_line: Option<usize>, content: &str) -> usize {
    match anchor_line.and_then(|line| self.line_bounds(line)) {
        Some((_, anchor_end)) => {
            let after = &self.text[anchor_end..];
            let newlines = after.bytes().take_while(|byte| *byte == b'\n').count();
            let tail = &after[newlines..];
            if tail.is_empty() {
                let base = self.text[..anchor_end + newlines]
                    .trim_end_matches('\n')
                    .len();
                let insert = format!("\n\n{content}");
                self.splice_text(base, self.text.len(), &insert);
                base + 2
            } else {
                let insert = format!("\n\n{content}\n\n");
                self.splice_text(anchor_end, anchor_end + newlines, &insert);
                anchor_end + 2
            }
        }
        None => {
            if self.text.trim_matches('\n').is_empty() {
                self.splice_text(0, self.text.len(), content);
                0
            } else {
                let base = self.text.trim_end_matches('\n').len();
                let insert = format!("\n\n{content}");
                self.splice_text(base, self.text.len(), &insert);
                base + 2
            }
        }
    }
}
```

### 5.3 关键点

1. **先清命令再扫描**：与现状一致，命令行清空成空行后再定位，行号不会因删除文字而偏移。
2. **`command_line` 而不是 `cursor`**：命令确认时光标在行尾，两者在同一行；用行号比较更直接。
3. **`span.start > command_line`**：自然跳过“命令所在的当前块”，取物理上下一个块。
4. **不借用 `scan` 跨可变借用**：`next_start` / `anchor_line` 都在调用 `splice_text` 前算成 `usize`/`Option<usize>`，避免 `masked` 的不可变借用与 `self.splice_text` 冲突。
5. **两次 `splice_text` 不会产生两条撤销**：调用方 `slash_confirm` 在调用前已经 `self.record(EditKind::Other)`，内部只改文本。
6. **光标偏移**：`block_start` 是正文起点，`cursor` 是 `supple_block_content` 相对正文的偏移，相加后正好落在 `{title: }` 的 `}` 前。
7. **滚动**：沿用 `scroll_lines_into_view(start_line, end_line)`，新块高于视口时至少保证起始行可见。

### 5.4 注释同步

- `src/app/markdown/slash.rs` 中 `SuppleBlock` 的注释由“确认时追加到固定落点”改为“确认时插到下一个块之前；无下一个块时追加到固定落点”。
- `src/app/prompt/block.rs` 顶部模块注释可保持“补充块操作”，但 `insert_supple_block` 的 doc 必须完整描述新规则。
- `docs/feature/prompt-block-restyle/design.md` §3.5 与 `task.md` P1.4 是历史记录，在对应段落顶部加一行“落点规则已由 `docs/feature/prompt-supplement-insert-before-next/` 调整”，不重写历史内容。

## 6. 测试矩阵

### 6.1 更新现有测试（旧行为必须改）

| 现有测试 | 变更 |
| --- | --- |
| `supplement_typed_before_template_keeps_command_blank_line_and_appends_after_template` | 改名并改为“命令在文首时插到 `&&&` 之前”，断言文首无前导空行 |
| `supplement_typed_inside_template_appends_after_last_existing_supplement` | 改名并改为“`&&&` 内命令插到第一个 `+++` 之前”，断言新块是 `spans[1]` |
| `unclosed_block_hides_supplement_command_and_skips_append` | 仅函数名改为 `insert_supple_block`，断言不变 |
| `supplement_scrolls_viewport_so_new_block_start_line_is_visible` | 定位仍取首个 Supple span；确认新块在 `&&&` 之前时滚动仍生效 |

### 6.2 新增测试

1. `supplement_inside_supple_inserts_before_next_supple`
   - 两个 `+++`，光标在第一个内部；确认后 3 个 `+++`，新块位于 S1 与 S2 之间。
   - 断言 S1 内命令空行保留。
   - 断言光标在 `{title: |}`。
   - 一次 `undo` 恢复命令行与光标。

2. `supplement_inside_template_inserts_before_first_supple`
   - `&&&` + S1 + S2，光标在 `&&&` 内；新块插在 S1 前，`spans` 顺序为 Template、Supple、Supple、Supple。

3. `supplement_before_template_inserts_before_template`
   - `+++` 在 `&&&` 之前，光标在 `+++` 内；下一个块是 `&&&`，新块插在 `&&&` 前，验证“任何块都算边界”。

4. `supplement_between_blocks_keeps_plain_text_above_new_block`
   - S1 → 普通文字 → S2；命令写在普通文字下方；新块紧贴 S2，普通文字仍在新块上方。

5. `supplement_after_last_block_appends_after_last_supple`
   - 两个 `+++`，光标在最后一个内部；无下一个块，新块追加到最后一个 `+++` 之后。

6. `supplement_spacing_collapses_extra_blank_lines`
   - 下一个块前有两个以上空行；插入后新块前后各恰好一个空行，不出现 `\n\n\n\n` 连续多空行。

7. `custom_supple_block_uses_same_insert_position`
   - 通过 `SlashItem::custom_supple` / `SlashInsertion::SuppleBlock` 走同一路径，确认与内置 `/supplement` 落点一致。

### 6.3 保持不变的回归

- 空文档插入：全文就是 `+++` 块，光标在标题内。
- 只有一个 `&&&` 且命令在块内：无下一个块，追加到 `&&&` 之后。
- 最后一个 `+++` 之后有普通正文：旧落点插在块与正文之间，前后各一空行。
- 未闭合块：面板不提供 `/supplement`，底层不插入。
- `/title` 在 `+++` 内仍取最近前置 `&&&` 正文，不受落点影响。

## 7. 受影响文件

| 文件 | 变更 |
| --- | --- |
| `src/app/prompt/block.rs` | 重命名函数，新增 `next_start` 分支、`insert_before_line`、`append_after_anchor` |
| `src/app/prompt/panel.rs` | 调用点改名 |
| `src/app/prompt/block/tests.rs` | 更新旧测试，新增落点/空行/自定义命令测试 |
| `src/app/markdown/slash.rs` | 仅注释同步，无逻辑变更 |
| `docs/feature/prompt-block-restyle/design.md` | §3.5 加一行“已被本任务调整”的说明（不改历史内容） |
| `docs/feature/prompt-block-restyle/task.md` | P1.4 加一行同上说明 |

不改：`src/ui/**`、`src/app/markdown/blocks.rs`、`src/app/markdown/supple.rs`、DB、配置。

## 8. 风险与回归控制

1. **旧测试故意失效**：现有两个测试断言“追加到最后一个 `+++` 之后”，这是本次要改变的行为；更新测试而不是保留旧断言。
2. **`block_scan` 只识别首个 `&&&`**：不要为了“下一个块”去改扫描器；现有 `spans` 已满足文档顺序。若未来放开多 `&&&`，本规则仍按 `spans` 自然工作。
3. **空行规整范围**：只规整“插入点前到下一个块行首”的换行，不全局重排文档；不要顺手修改其他块的空行。
4. **命令行空行**：块内必须保留，避免改变现有撤销/清理语义；文首例外按 4.3 折叠。
5. **未闭合守卫**：必须先于 `next_start` 判断返回，避免在半成品块之间插入。
6. **单步撤销**：`insert_supple_block` 内部不得再调用 `record`，否则会多出撤销步。
7. **生产代码约束**：新增代码不得使用 `unwrap()` / `expect()`；错误路径用 `Option` 直接返回。

## 9. 验收标准

- 6.2 全部新增测试通过，6.1 更新后的旧测试通过。
- `cargo fmt` 无差异。
- `cargo clippy --all-targets -- -D warnings` 通过。
- `cargo test` 全量通过。
- 用户运行 `cargo run` 人工确认：
  - 两个 `+++`，命令写在第一个内部 → 新块出现在两者之间；
  - 命令写在最后一个 `+++` 内部 → 新块追加在最后；
  - 命令行清空后块内空行保留；
  - 新块前后各一个空行，光标在标题内。

## 10. 实施记录（2026-11-13）

- `src/app/prompt/block.rs`：`append_supple_block` 更名为 `insert_supple_block`；新增 `next_start` 分支、`insert_before_line` 与 `append_after_anchor`。
- `src/app/prompt/panel.rs`：`slash_confirm` 的 `SuppleBlock` 分支改用 `insert_supple_block`。
- `src/app/prompt/block/tests.rs`：更新 2 个旧落点测试，新增 6 个落点/空行/自定义命令测试。
- `src/app/markdown/slash.rs`：仅同步注释，候选过滤逻辑未改。
- 未改动 `src/ui/**`、`src/app/markdown/blocks.rs`、`src/app/markdown/supple.rs`、DB 迁移与配置。
- 验证：`cargo fmt` 无差异；`cargo clippy --all-targets -- -D warnings` 通过；`cargo test` 全量 5423 passed、8 ignored。
