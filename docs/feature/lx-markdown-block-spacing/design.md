# lx 助手正文 Markdown 块间距与超链接/生图交互：设计方案

> 状态：**已完成（Phase 1 块间距 + Phase 2 超链接与生图交互）**
> 日期：2026-11-10
> 项目：`/Users/yonah/projects/agent/lx-tui`（工作区：`.worktrees/lx-markdown-block-spacing`）
> 参考实现：`/Users/yonah/projects/agent/opencode-dev`（`@opentui/core` `Markdown.ts`、`packages/tui/src/ui/link.tsx`）、`/Users/yonah/projects/agent/codex-main`（`codex-rs/tui/src/markdown_render.rs`、`markdown_render/web_links.rs`）
> 作用域：`src/ui/markdown_render*`、`src/ui/main_content/lx/*`、`src/app/state/lx/chat.rs`、`src/app/update/chat/*`、`src/event/mouse/*`、`docs/standards/tui-design-requirements.md`
> 配套文档：[task.md](./task.md)

---

## 1. 背景与问题根因

### 1.1 现状问题（截图现象复盘）

在 `lx` 的聊天历史视图中，AI 助手输出的多段 Markdown 正文（`PartKind::Text`）存在严重的视觉粘连问题：

1. **段落与段落、段落与标题紧贴**：`## 核查结论` 上方与前一段结论文字无空行，下方与过渡段落无空行，小标题夹在正文中难以一眼识别。
2. **段落与列表、列表与后续段落紧贴**：过渡句下方紧贴 `- app/ 不是完全无 IO...` 列表，列表结束后紧贴下一段粗体大点 `**2. 规范红线...**` 与两段统计正文。
3. **源码中的 `\n\n` 空行被完全吞噬**：核查数据库落库原文发现，模型输出的 Markdown 源码在各段落、标题、列表、大点之间均规范书写了 `\n\n` 空行，但 `src/ui/markdown_render.rs:246-250` 在块级解析循环中直接执行了：
   ```rust
   let trimmed = line.trim_end();
   if trimmed.is_empty() {
       index += 1;
       continue;
   }
   ```
   除代码块下方通过 `state.after_code_block` 单独补了 1 行空行外，其余所有块级边界（段落、标题、列表、表格、引用块、水平分割线）之间的空行全部被丢弃，导致整段回答退化为无间隔的密集文字墙。

### 1.2 参考实现对比（`opencode-dev` 与 `codex-main`）

| 维度 | `opencode-dev`（`@opentui/core` `Markdown.ts`） | `codex-main`（`codex-rs/tui/src/markdown_render.rs`） | `lx-tui` 目标方案 |
| :--- | :--- | :--- | :--- |
| **间距驱动机制** | `buildTopLevelRenderBlocks` + `shouldAddTopLevelMargin`：仅在进入下一个块时设 `marginTop = 1`，块尾换行全部剥离，绝不产出尾部空行或双空行。 | `Writer::needs_newline`：每个块结束置 `true`，下一个块开始时吐出 1 行空行，文档首尾不留空行。 | 在 `ParserState` 中维护 `prev_block: Option<PrevBlock>` 与 `saw_blank_line: bool`，仅在**下一个非空块产出前**按转换矩阵插入至多 1 行空行。 |
| **顶层块边界** | `heading`、`list`、`code`、`table`、`blockquote`、`hr` 为独立分隔块，与任意相邻块恒隔 1 行；`paragraph → paragraph` 在源码含 `\n\n` 时隔 1 行。 | `Paragraph`、`Heading`、`List`、`CodeBlock`、`BlockQuote`、`Table`、`Rule` 之间恒隔 1 行；段内软换行（单 `\n`）不空行。 | 对齐两者：独立结构块与相邻顶层块恒隔 1 行；`Paragraph → Paragraph` 仅在源码含空行（`\n\n`）时隔 1 行（单 `\n` 紧邻）；代码块上方保留 `▂` 顶边自成留白（不叠加空行）。 |
| **列表项间距** | Tight 列表项 `marginBottom = 0`；Loose 列表项（源码含 `\n\n`）`marginBottom = 1`；项内多段按 `space` token 隔 1 行；嵌套子列表 `marginTop = 0`（紧贴父项）。 | `ListSpacing` 按渲染行数或块数决定是否插空行；嵌套子列表紧贴父项。 | 采用 `opencode` 松紧列表规则 + 子列表退回外层 / 同级不同类型列表切换自动补 1 行空行。 |
| **生效范围** | 仅助手正文 `<markdown internalBlockMode="top-level">` 启用块间距；思考块与工具输出保持紧凑。 | 仅 `AgentMarkdownCell` 使用。 | 仅助手正文 `render_wrapped_code_blocks`（`code_chrome == true`）启用；思考块 `render_wrapped` 与工具/汇报卡片 `render_wrapped_literal_rules` 保持紧凑。 |

---

## 2. 已确认边界与决策（Grill Me 结论）

| 编号 | 决策项 | 已确认方案 |
| :--- | :--- | :--- |
| **Q1** | **顶层块间垂直空行规则** | 任意两个相邻顶层块之间最多插入 **1 行空行**（源码中连续多个空行合并为 1 行；块首行前与末行后绝不产生空行，避免与 `history::snapshot::Builder::spacer()` 的步骤间空行叠加）：<br>1. **段落 ↔ 段落**：源码中有空行（`\n\n`）时空 1 行；单个换行（`\n`）保持紧邻。<br>2. **标题 / 最外层列表 / 表格 / 引用块 / 水平分割线 ↔ 任意相邻顶层块**：即使源码漏写空行，进入该块前与离开该块后也固定隔开 **1 行空行**。 |
| **Q2** | **列表内部垂直间距规则** | 1. **紧凑同级同类型列表项之间（源码无 `\n\n`）**：即使单条因字数多软折行成 2～3 行，项间仍保持 **0 空行**。<br>2. **松散列表项 / 列表项内多段之间（源码有 `\n\n`）**：保留 **1 行空行**；列表项内出现续段（`Paragraph { in_list: true }`）后再进入下一条同级/外层列表项时，也隔开 **1 行空行**。<br>3. **父列表项 → 嵌套子列表（`depth > prev_depth`）**：固定 **0 空行**，子列表紧贴父项。<br>4. **子列表结束退回外层列表项（`depth < prev_depth`）或同级不同类型列表切换（`Bullet` / `Ordered` / `Task` 互切）**：即便源码未写空行，自动补 **1 行空行**。 |
| **Q3** | **作用范围** | **仅在 AI 助手主回答正文**（主聊天流与子 Agent 详情页 `PartKind::Text`，即 `render_wrapped_code_blocks` / `IncrementalMarkdownState`，`code_chrome == true`）中启用块间空行规则；思考块（`render_wrapped`）与工具/命令/汇报/Compact 卡片（`render_wrapped_literal_rules`）保持现有紧凑无空行排版，不侵占折叠预览的 3～4 行额度。 |
| **Q4** | **代码块上方 `▂` 顶边留白** | `lx` 代码块第 1 行 `TopBorder` 为下八分之一块字符 `▂`（`U+2582`），其上半部分 7/8 本身即为视觉留白：<br>1. 前一块为**非代码块**时，代码块 `TopBorder` 上方**不额外插空行**（避免出现近 2 行高的大白洞）；<br>2. 前一块同为**代码块**（两个代码块相邻）时，在第二个代码块 `TopBorder` 前保留 **1 行空行**；<br>3. 代码块下方与后续任意非代码块之间固定保留 **1 行空行**。 |

---

## 3. 核心架构与状态机设计

### 3.1 为什么必须在 `markdown_render` 内部产出空行？

遵循项目规范（`AGENTS.md` 与 `docs/standards/tui-design-requirements.md`）：

- **行数与绘制同源**：每个 `SnapBlock` 的高度 `rows` 唯一来自 `BlockPaint::rows()`（对于 `BlockPaint::Markdown` 即 `prefix.len() + tail.len()`），跨步骤空行唯一由 `history::snapshot::Builder::spacer()` 累加，绘制路径 `paint.rs` 严禁另算空行。
- **与步骤标记 `●`、选区、复制、命中天然兼容**：
  - `draw_markdown`（`src/ui/main_content/lx/history/paint.rs:720`）只在 `offset == 0`（块的第 0 行）绘制 `●` 标记；只要块首不产出前导空行，`●` 恒定落在首个可见正文行。
  - 块内空行表现为 `MdLine::spans(0, Vec::new())`（无 `CodeLineKind`、无 `TableLineRef`、宽度 0），在 `selection::copy_text` 中自然产出 `""` 空行并在末尾自动裁剪，在 `history_hit` 中不触发任何误命中。
- **流式增量渲染 100% 等价**：`IncrementalMarkdownState`（`src/ui/markdown_render/incremental.rs`）将源码切分为已固化的 `committed` 前缀与活跃 `tail` 尾部，两者共享同一个 `ParserState` 并调用同一个 `render_blocks_stateful`。只要块间距状态完整保存在 `ParserState` 中，且**只在下一个非空块实际产出前才推入空行**，流式任意字节边界下的 `prefix + tail` 就必然与全量 `render_wrapped_code_blocks` 逐行严格相等。

### 3.2 状态机数据结构

在 `src/ui/markdown_render.rs` 中将原有的单一布尔量 `after_code_block: bool` 升级为完整的块级前驱状态：

```rust
/// 列表项种类：用于区分无序、有序与任务列表，同级切换列表类型时视作新列表。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ListKind {
    Bullet,
    Ordered,
    Task,
}

/// 上一个已产出块的种类（供 `code_chrome == true` 的块间距状态机判定是否插入前导空行）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PrevBlock {
    /// 普通段落或图片行；`in_list` 表示该行是否作为列表项内的缩进续段，`depth` 为其所属列表层级。
    Paragraph { in_list: bool, depth: usize },
    /// 标题行（`#`..`######`）。
    Heading,
    /// 列表项（记录换算后的层级 `depth` 与列表种类 `kind`）。
    ListItem { depth: usize, kind: ListKind },
    /// 围栏代码块或 4 空格缩进代码块。
    CodeBlock,
    /// 表格块。
    Table,
    /// 引用块（`>`）。
    Quote,
    /// 水平分割线（`---` / `***` / `___`）。
    Rule,
}
```

`ParserState` 结构更新如下：

```rust
#[derive(Debug, Clone, Default)]
struct ParserState {
    /// 列表缩进档位栈：把源码缩进列数换算成层级，跨行维护，非列表行按缩进回退（空行不清，允许松散列表）。
    list_indents: Vec<usize>,
    /// 与 `list_indents` 平行的列表项内容起始列栈（最内层为 last）。
    list_content_indents: Vec<usize>,
    /// 与 `list_indents` 平行的列表项标记悬挂宽度栈。
    list_hangs: Vec<u16>,
    /// `Some(lang)` 表示位于未闭合围栏代码块内（空串 = 围栏未写语言）。
    open_fence_lang: Option<String>,
    /// 当前未闭合围栏 opener 的反引号数量；关闭行必须不少于该数量。
    open_fence_ticks: usize,
    /// 下一个代码块的 0-based 序号（围栏与 4 空格缩进代码块统一递增）。
    next_code_block_index: usize,
    /// 上一个已产出的块种类（跨增量分块保留；`None` 表示尚未产出任何块）。
    prev_block: Option<PrevBlock>,
    /// 自上一个块产出后，源码中是否至少出现过一条空行（跨增量分块保留）。
    saw_blank_line: bool,
}
```

### 3.3 块间空行转换矩阵与判定函数

定义即将产出的当前块类型与统一间距判定函数 `push_block_gap_if_needed`：

| 前驱块 `prev_block` | 当前块 `curr` | `saw_blank_line == false`（源码无空行） | `saw_blank_line == true`（源码有 `\n\n`） | 说明 |
| :--- | :--- | :---: | :---: | :--- |
| `None`（块首） | 任意块 | **0** | **0** | 首行前绝不留空行，保证 `●` 落在第 0 行。 |
| `CodeBlock` | `CodeBlock` | **1** | **1** | 两个相邻代码块之间共享 1 行空行。 |
| 非 `CodeBlock` | `CodeBlock` | **0** | **0** | 代码块顶边 `▂` 上半截 7/8 自带视觉留白，不叠加空行。 |
| 任意块 | `Heading` / `Table` / `Rule` | **1** | **1** | 独立结构块上方恒隔 1 行。 |
| `Quote` | `Quote` | **0** | **1** | 连续 `>` 行（含跨增量分块）同属一个引用块；中间隔空行则为两个独立引用块。 |
| 非 `Quote` | `Quote` | **1** | **1** | 进入引用块前恒隔 1 行。 |
| 非列表块（顶层段落/标题/代码/表/引用/线） | `ListItem { .. }` | **1** | **1** | 从非列表块进入列表恒隔 1 行。 |
| `ListItem { prev_depth, prev_kind }` | `ListItem { depth, kind }` | 若 `depth > prev_depth`：**0**<br>若 `depth < prev_depth`：**1**<br>同级：`prev_kind != kind` 时 **1**，否则 **0** | 若 `depth > prev_depth`：**0**<br>其余：**1** | 父项→嵌套子列表恒 0 行；子列表退回外层或同级切列表类型自动空 1 行；松散同级列表项空 1 行；紧凑同级列表项 0 行。 |
| `Paragraph { in_list: true, prev_depth }` | `ListItem { depth, .. }` | 若 `depth > prev_depth`：**0**，否则 **1** | 若 `depth > prev_depth`：**0**，否则 **1** | 列表项带续段后，进入下一条同级或外层列表项前空 1 行；若续段下紧跟更深层子列表则紧贴。 |
| `Heading` / `CodeBlock` / `Table` / `Quote` / `Rule` | `Paragraph { .. }` | **1** | **1** | 离开独立结构块进入段落恒隔 1 行。 |
| `ListItem { .. }` | `Paragraph { in_list, .. }` | `!in_list` 时 **1**，`in_list` 时 **0** | **1** | 未缩进段落退出列表时空 1 行；紧贴的缩进续行 0 行；隔了 `\n\n` 的缩进续段空 1 行。 |
| `Paragraph { prev_in_list, prev_depth }` | `Paragraph { in_list, depth }` | `prev_in_list && (!in_list \|\| depth != prev_depth)` 时 **1**，否则 **0** | **1** | 同一段落内单 `\n` 换行 0 行；`\n\n` 分段空 1 行；从列表内续段退出到顶层段落或跨层回退时空 1 行。 |

---

## 4. 关键代码实现

### 4.1 `ListItem::kind` 与块间距辅助函数（`src/ui/markdown_render.rs`）

```rust
impl ListItem<'_> {
    /// 列表项种类。
    fn kind(&self) -> ListKind {
        match *self {
            Self::Bullet { .. } => ListKind::Bullet,
            Self::Ordered { .. } => ListKind::Ordered,
            Self::Task { .. } => ListKind::Task,
        }
    }
}

/// 在产出下一个非空块 `curr` 之前，按块间距规则推入至多 1 行空行并推进 `state`。
fn transition_block(
    state: &mut ParserState,
    curr: PrevBlock,
    code_chrome: bool,
    lines: &mut Vec<MdLine>,
) {
    if code_chrome && should_insert_blank_line(state.prev_block, state.saw_blank_line, curr) {
        lines.push(MdLine::spans(0, Vec::new()));
    }
    state.prev_block = Some(curr);
    state.saw_blank_line = false;
}

/// 判定前驱块 `prev` 与当前块 `curr` 之间是否需要插入 1 行空行。
fn should_insert_blank_line(
    prev: Option<PrevBlock>,
    saw_blank_line: bool,
    curr: PrevBlock,
) -> bool {
    let Some(prev) = prev else {
        return false;
    };
    match curr {
        // 代码块自带 `▂` 顶边（上半截 7/8 留白），仅在连续两个代码块之间补 1 行空行。
        PrevBlock::CodeBlock => prev == PrevBlock::CodeBlock,
        // 标题、表格、分割线作为独立顶层块，上方恒隔 1 行。
        PrevBlock::Heading | PrevBlock::Table | PrevBlock::Rule => true,
        // 引用块：连续 `>` 行（无空行分隔）同属一个引用块，其余情况上方隔 1 行。
        PrevBlock::Quote => prev != PrevBlock::Quote || saw_blank_line,
        // 列表项：区分进入列表、父进子、子退父、同级异类切换与松紧列表。
        PrevBlock::ListItem { depth, kind } => match prev {
            PrevBlock::ListItem {
                depth: prev_depth,
                kind: prev_kind,
            } => {
                if depth > prev_depth {
                    false
                } else if saw_blank_line || depth < prev_depth {
                    true
                } else {
                    prev_kind != kind
                }
            }
            PrevBlock::Paragraph {
                in_list: true,
                depth: prev_depth,
            } => depth <= prev_depth,
            _ => true,
        },
        // 段落或图片行：区分顶层段落、列表内紧凑续行、列表内松散续段与退出列表。
        PrevBlock::Paragraph { in_list, depth } => match prev {
            PrevBlock::Heading
            | PrevBlock::CodeBlock
            | PrevBlock::Table
            | PrevBlock::Quote
            | PrevBlock::Rule => true,
            PrevBlock::ListItem { .. } => saw_blank_line || !in_list,
            PrevBlock::Paragraph {
                in_list: prev_in_list,
                depth: prev_depth,
            } => saw_blank_line || (prev_in_list && (!in_list || depth != prev_depth)),
        },
    }
}
```

### 4.2 `render_blocks_stateful` 主循环改动（`src/ui/markdown_render.rs`）

```rust
fn render_blocks_stateful(
    source: &str,
    width: Option<usize>,
    expand_rules: bool,
    code_chrome: bool,
    state: &mut ParserState,
) -> Vec<MdLine> {
    let mut lines = Vec::new();
    let source_lines = split_source_lines(source);
    let mut index = 0;
    while index < source_lines.len() {
        let line = source_lines[index];
        // 未闭合围栏内部：每行都是独立代码行（闭合行只推进状态）。
        if let Some(lang) = state.open_fence_lang.take() {
            if fence_ticks(line).is_some_and(|ticks| ticks >= state.open_fence_ticks) {
                state.prev_block = Some(PrevBlock::CodeBlock);
                state.saw_blank_line = false;
                state.open_fence_ticks = 0;
                index += 1;
                continue;
            }
            code_block::render_code_line(
                line,
                (!lang.is_empty()).then_some(lang.as_str()),
                code_chrome,
                &mut lines,
            );
            state.open_fence_lang = Some(lang);
            index += 1;
            continue;
        }
        let trimmed = line.trim_end();
        if trimmed.is_empty() {
            if state.prev_block.is_some() {
                state.saw_blank_line = true;
            }
            index += 1;
            continue;
        }
        let item = list_item(line);
        update_list_context(state, item.as_ref(), line);
        let min_indent = list_code_min_indent(state);
        if let Some(ticks) = fence_ticks(line) {
            transition_block(state, PrevBlock::CodeBlock, code_chrome, &mut lines);
            let rest = trimmed.trim_start();
            let lang = rest[ticks..].trim().to_string();
            let block_index = state.next_code_block_index;
            state.next_code_block_index += 1;
            if code_chrome {
                lines.push(code_block::top_border_line(width));
                lines.push(code_block::header_line(&lang, block_index, width));
            }
            state.open_fence_lang = Some(lang);
            state.open_fence_ticks = ticks;
            index += 1;
            continue;
        }
        if let Some(item) = item {
            let depth = state.list_indents.len().saturating_sub(1);
            transition_block(
                state,
                PrevBlock::ListItem {
                    depth,
                    kind: item.kind(),
                },
                code_chrome,
                &mut lines,
            );
            lines.push(item.render(depth));
            index += 1;
            continue;
        }
        if is_indented_code(line, min_indent) {
            transition_block(state, PrevBlock::CodeBlock, code_chrome, &mut lines);
            let block_index = state.next_code_block_index;
            state.next_code_block_index += 1;
            index = indented_code(
                &source_lines,
                index,
                width,
                block_index,
                code_chrome,
                min_indent,
                &mut lines,
            );
            continue;
        }
        if let Some((level, body)) = heading(trimmed) {
            state.list_indents.clear();
            state.list_content_indents.clear();
            state.list_hangs.clear();
            transition_block(state, PrevBlock::Heading, code_chrome, &mut lines);
            lines.push(heading_line(level, body));
            index += 1;
            continue;
        }
        if is_rule(trimmed) {
            state.list_indents.clear();
            state.list_content_indents.clear();
            state.list_hangs.clear();
            transition_block(state, PrevBlock::Rule, code_chrome, &mut lines);
            if expand_rules {
                let rule_width = width.unwrap_or(40).max(1);
                lines.push(MdLine::plain(
                    0,
                    RULE.repeat(rule_width),
                    style::md_read_rule(),
                ));
            } else {
                lines.push(MdLine::plain(0, trimmed, Style::default()));
            }
            index += 1;
            continue;
        }
        if trimmed.trim_start().starts_with('|') {
            state.list_indents.clear();
            state.list_content_indents.clear();
            state.list_hangs.clear();
            transition_block(state, PrevBlock::Table, code_chrome, &mut lines);
            index = table::table(&source_lines, index, &mut lines, width);
            continue;
        }
        if trimmed.trim_start().starts_with('>') {
            state.list_indents.clear();
            state.list_content_indents.clear();
            state.list_hangs.clear();
            transition_block(state, PrevBlock::Quote, code_chrome, &mut lines);
            index = blockquote(&source_lines, index, &mut lines);
            continue;
        }
        let in_list = !state.list_indents.is_empty();
        let depth = state.list_indents.len().saturating_sub(1);
        transition_block(
            state,
            PrevBlock::Paragraph { in_list, depth },
            code_chrome,
            &mut lines,
        );
        if let Some((alt, url)) = image(trimmed) {
            lines.push(MdLine::spans(
                0,
                vec![
                    (alt.to_string(), style::text()),
                    (" ".to_string(), style::text()),
                    (format!("({url})"), style::md_read_image()),
                ],
            ));
            index += 1;
            continue;
        }
        // 普通段落行：行内渲染（若仍处于列表上下文内，续段按当前列表层级的正文列缩进）。
        let cont_indent = if !in_list {
            0
        } else {
            let hang = state.list_hangs.get(depth).copied().unwrap_or(2);
            indent(depth).saturating_add(hang)
        };
        let body = if cont_indent > 0 {
            trimmed.trim_start()
        } else {
            trimmed
        };
        lines.push(MdLine::spans(cont_indent, inline_spans(body)));
        index += 1;
    }
    lines
}
```

> **关键细节验证**：
> 1. 当围栏代码块**尚未闭合**（流式正在输出代码行）时，`transition_block(state, PrevBlock::CodeBlock, ...)` 在围栏 opener 行已执行（若前驱也是 `CodeBlock` 则插入 1 行空行，否则不插入），且将 `state.prev_block` 置为 `Some(PrevBlock::CodeBlock)`、`state.saw_blank_line = false`。围栏内每行代码走 `if let Some(lang) = state.open_fence_lang.take()` 分支，直到闭合行再次巩固 `state.prev_block = Some(PrevBlock::CodeBlock)`。
> 2. 带有前导空格的 `---`（水平线）、`| ... |`（表格）、`> ...`（引用块）出现时，显式清空 `state.list_indents` / `list_content_indents` / `list_hangs`，确保列表状态干净退出，随后若跟普通段落或新列表均从第 0 列顶层重新开始。

---

## 5. 影响面与兼容性分析

1. **AI 助手正文（`render_wrapped_code_blocks` & `IncrementalMarkdownState`）**：
   - 段落与段落（`\n\n`）、标题与前后块、列表与前后块、表格/引用/分割线与前后块之间均呈现 1 行清晰的空白行；
   - 紧凑列表项之间保持 0 空行，松散列表项、子列表退回外层、同级异类列表切换自动空 1 行；
   - 代码块上方保持 `▂` 顶边紧接前文（相邻双代码块间仍空 1 行），代码块下方空 1 行；
   - 流式增量渲染 `IncrementalMarkdownState` 与全量 `render_wrapped_code_blocks` 在任意字节切分点保持 100% 逐行一致。
2. **思考块与工具/汇报卡片（`render_wrapped` / `render_wrapped_literal_rules` / `render`）**：
   - `code_chrome == false`，`transition_block` 不插入任何额外空行，保持原有紧凑排版，折叠预览行数与既有测试完全不变。
3. **历史快照、滚动条、置顶行、选区复制与点击命中**：
   - 空行包含在 `BlockPaint::Markdown` 的 `prefix`/`tail` 内，`paint.rows()` 自动反映最新行数；
   - 代码块 Header 的行偏移随上方新增空行自然顺延，`history_hit::code_copy_at_source` 通过遍历 `prefix.iter().chain(tail.iter())` 按内容行下标精确索引，无需任何改动即可准确命中 `[❐]`；
   - 表格选区 `Grid::set_table_row` 随 `draw_markdown` 的实际 `y` 行号写入，表格复制与反显不受上方空行影响。

---

## 6. 测试方案

在 `src/ui/markdown_render/code_block/tests.rs` 与 `src/ui/markdown_render/incremental/tests.rs` 中新增并覆盖以下测试场景：

1. **顶层块间距测试**：
   - 段落与段落：源码单换行 `\n` 不产生空行，源码双换行 `\n\n` 产生 1 行空行，连续多个空行 `\n\n\n\n` 合并为 1 行空行；首尾空行不产生前导或尾部空行。
   - 标题（`#`..`######`）、表格、引用块（`>`）、水平分割线（`---`）与前后段落/列表之间（无论源码是否写空行）均恰好产生 1 行空行。
   - 多行连续引用（`> a\n> b`）内部不拆散，两个隔空行的引用块（`> a\n\n> b`）之间产生 1 行空行。
2. **列表间距测试**：
   - 紧凑列表（`- a\n- b\n- c`，含软换行成多视觉行的项）项间保持 0 空行。
   - 松散列表（`- a\n\n- b` 或 `1. a\n\n2. b`）项间产生 1 行空行。
   - 父列表项与嵌套子列表（`1. parent\n   - child`）之间保持 0 空行。
   - 嵌套子列表结束退回外层列表项（`1. one\n   - sub1\n   - sub2\n2. two`）在 `2. two` 前自动产生 1 行空行。
   - 同级不同类型列表切换（`- bullet\n1. ordered`）之间自动产生 1 行空行。
   - 列表项内含缩进续段后进入下一条列表项（`1. step\n\n    explanation\n2. next`）在续段前后各产生 1 行空行。
3. **真实坏例回归测试**：
   - 使用截图中对应的真实 Markdown 片段（段落 + `## 核查结论` + 粗体段 + 无序列表 + 粗体段 + 多段统计），验证 `render_wrapped_code_blocks` 产出的空行位置与预期完全吻合。
4. **流式增量等价性回归测试**：
   - `incremental/tests.rs` 的逐字节流式等价性测试与伪随机块序列测试全部通过，并补充松散列表、子列表退回外层、连续引用等流式分块边界用例。
5. **非助手正文紧凑性守卫测试**：
   - 验证 `render`、`render_wrapped`、`render_wrapped_literal_rules` 对含 `\n\n` 的多段/标题/列表输入仍不插入块间空行。

---

## 7. Phase 2：Markdown 超链接渲染优化与 `Ctrl/Cmd + 点击` 打开（含 AI Web 生图行）

### 7.1 背景与已确认边界（Grill Me 结论）

在新闻检索、文档汇总等场景中，模型输出的 `[标题](https://...)` 若强行展开为 `标题 (https://...)`，会导致超长 URL 独占整行且与标题同为高饱和亮蓝色，严重破坏阅读流。对比 `opencode-dev`（`Markdown.ts`）与 `codex-main`（`markdown_render/web_links.rs`）后，已与用户确认以下 3 项边界：

| 编号 | 决策项 | 已确认方案 |
| :--- | :--- | :--- |
| **Q5** | **`[文字](url)` 渲染格式** | 当 `url` 为 `http://` 或 `https://` 网页链接且 `文字` 非空时，**仅渲染 `文字`**（样式 `style::md_read_link_text()`：链接蓝 `#4493f8` + 下划线），**彻底隐藏 `(url)`**；当 `文字` 为空（`[](https://...)`）、自动链接（`<https://...>`）或裸网址（`https://...`）时，直接渲染网址本身并带下划线；当 `url` 为非 `http(s)://` 本地相对路径（如 `[说明](docs/readme.md)`）时，保留显示 `说明 (docs/readme.md)` 且不作为网页链接打开。 |
| **Q6** | **Markdown 超链接点击与选区复制** | 在 `lx` 聊天区所有 Markdown 文本（主对话与子 Agent 详情页的段落、列表、引用、表格单元格、思考块与汇报/工具卡片，含软换行折成多行的长链接标题）中，按住 **`Ctrl / Cmd` + 鼠标左键单击**链接文字任意位置，即调用系统默认浏览器打开目标 `http(s)://` 网址（`Effect::OpenBrowser`）；普通左键拖拽选区复制（`Ctrl/Cmd+C`）时仅复制界面可见文字。 |
| **Q7** | **AI Web 生成图片行的点击打开方式** | 助手消息底部的 AI Web 生图行（`└─ [image](preview): xxx.png  [↓ Download original]`）：<br>1. 左侧带下划线的预览图文件名 `xxx.png`（`CardTarget::Attachment`）与右侧下载完成后的 `[✓ Downloaded]`，**改为必须按住 `Ctrl / Cmd` + 鼠标左键单击**才在系统文件管理器（Finder / Explorer）中定位打开（普通左键点击不再弹出外部窗口，避免拖选文本时误触）；<br>2. 未下载状态的 `[↓ Download original]`（`OriginalImageDownloadState::Idle`）保留普通左键单击（兼容 `Ctrl/Cmd + 点击`）立即触发后台下载。 |

### 7.2 核心数据结构与链路设计

#### 1. `InlineSpan` 与 `MdLine::links`（`src/ui/markdown_render/inline.rs` & `src/ui/markdown_render.rs`）

在 `inline.rs` 中新增带可选链接目标的 `InlineSpan`：

```rust
/// 行内渲染分段：携带显示文本、样式与可选的 `http(s)://` 超链接目标。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct InlineSpan {
    pub(super) text: String,
    pub(super) style: Style,
    pub(super) link: Option<String>,
}

/// 判定是否为安全的 `http://` 或 `https://` 网页链接。
pub(crate) fn is_web_url(url: &str) -> bool {
    let trimmed = url.trim();
    let rest = trimmed
        .strip_prefix("https://")
        .or_else(|| trimmed.strip_prefix("http://"));
    rest.is_some_and(|tail| {
        !tail.is_empty() && !tail.chars().any(|ch| ch.is_whitespace() || ch.is_control())
    })
}
```

- 解析 `[label](url)` 时：
  - 若 `is_web_url(url)` 为真：提取剥离内联标记后的 `display_label`（若为空则回退 `url.trim()`），推入单段 `InlineSpan { text: display_label, style: style::md_read_link_text(), link: Some(url.trim().to_string()) }`，**不再追加 ` ({url})`**；
  - 若 `is_web_url(url)` 为假（本地路径等）：保留 `display_label`（`md_read_link_text`）+ ` ({url})`（`md_read_url`），`link: None`。
- 解析 `<url>` 与裸 `http(s)://...` 时：
  - 裸 URL 自动剥离句尾常见中英文标点（`。，、；！？）》】,.!?;:)` 等未配对尾随标点），使用 `style::md_read_link_text()`（链接蓝 + 下划线）并挂载 `link: Some(url)`。

在 `MdLine` 上记录行内超链接的列范围（相对 `line.indent` 之后的起始列 `0`）：

```rust
/// 单行内的超链接列区间（列偏移相对 `MdLine::indent` 之后的正文起点，左闭右开 `start_col..end_col`）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MdLinkSpan {
    pub start_col: u16,
    pub end_col: u16,
    pub url: String,
}

pub struct MdLine {
    pub indent: u16,
    pub hang: u16,
    pub spans: Vec<(String, Style)>,
    pub links: Vec<MdLinkSpan>,
    pub table: Option<TableLineRef>,
    pub code: Option<CodeLineKind>,
}
```

- **软换行保持超链接映射（`wrap_md_line_inner`）**：
  - 将 `cells` 扩展为 `Vec<(char, Style, Option<&str>)>`，按字符所在列从 `line.links` 查找其所属 URL；
  - 折行切分出 `cells[index..break_at]` 后，不仅聚合 `spans`，同时按连续相同的 `Some(url)` 聚合计算该视觉行的 `links: Vec<MdLinkSpan>`。因此长链接标题被软折成 2～3 行时，每一行对应的字符列区间都精准携带原始 URL。
- **表格单元格超链接映射（`src/ui/markdown_render/table.rs`）**：
  - `CellSpan` 携带 `link: Option<String>`，经 `split_br` 与单元格内软换行保留；组装表格行 `│ cell │` 时，将单元格内带 `link` 的分段换算为整行相对列偏移写入该表格行的 `MdLine.links`。
- **工具/汇报/Compact 压暗 Markdown（`src/ui/main_content/lx/history/tool.rs`）**：
  - `tool::Segment` 新增 `pub(crate) link: Option<String>`，`md_line_segments` 与 `wrap_spans` 原样传递 `link`。

#### 2. `Grid` 登记超链接与命中查询（`src/ui/main_content/lx/grid.rs` & `selection.rs`）

在 `Grid` 中维护每行的超链接区间表 `link_rows: Vec<Vec<GridLinkSpan>>`：

```rust
/// 网格中单行的超链接热区（列偏移相对 `Grid.area.x`，左闭右开 `start_col..end_col`）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct GridLinkSpan {
    pub(crate) start_col: u16,
    pub(crate) end_col: u16,
    pub(crate) url: String,
}
```

- `draw_markdown`、`draw_thinking` 与 `tool::put_segments` 在向 `Grid` 写入 `MdLine` / `Segment` 时，调用 `grid.set_link_span(x, y, cols, url)` 登记屏幕列区间；
- `selection::link_at_cell(area, lx, (content_row, content_col), cache)` 复用 1 行 `Grid`（`render_chunk(&mut grid, chunk_rect, lx, row, cache)`），调用 `grid.link_at(content_col, 0)` 即可在 O(1) 行开销内精确返回命中的 `url`（天然支持主聊天流与 `TaskTab::Agents` 子 Agent 详情页）。

#### 3. `Ctrl / Cmd + 鼠标左键点击` 交互分派（`src/event/mouse/press.rs`、`src/ui/main_content/lx/selection.rs`、`src/app/update/chat/viewport.rs`）

```rust
/// 是否按下用于“打开外部链接/文件”的 `Ctrl` 修饰键。
pub(crate) fn is_open_modifier(modifiers: crossterm::event::KeyModifiers) -> bool {
    modifiers.contains(crossterm::event::KeyModifiers::CONTROL)
}
```

- 在 `CardTarget` 中新增 `OpenUrl { url: String }`；
- 在 `press_content_hit(area, lx, column, row, modifiers, cache)` 中：
  1. 若 `is_open_modifier(modifiers)` 为真，优先调用 `link_at_cell`：命中超链接时直接返回 `Some(CardTarget::OpenUrl { url })`（即使该链接位于表格单元格或已展开的思考块/汇报卡内，也优先打开链接而非折叠卡片）；
  2. 对卡片命中结果做修饰键门控：
     - `CardTarget::Attachment { .. }`（AI Web 预览图文件名）：仅当 `is_open_modifier(modifiers)` 为真时返回，普通左键点击返回 `None`（退回普通文本选区）；
     - `CardTarget::DownloadOriginalImage { attachment_id }`：当状态为 `OriginalImageDownloadState::Idle`（`[↓ Download original]`）时普通左键与 `Ctrl/Cmd + 点击` 均放行；当状态为 `Downloaded`（`[✓ Downloaded]`）时仅在 `is_open_modifier(modifiers)` 为真时放行。
- 在 `toggle_lx_card` 与 `toggle_agent_card` 中处理 `CardTarget::OpenUrl { url }`：推入 `Effect::OpenBrowser { url }`，由后台线程调用 `platform::open_browser(&url)` 在系统默认浏览器中打开。
