# lx 工具与卡片展开内容统一思考块同款压暗 Markdown 格式与配色：设计方案

> 状态：**已在 `.worktrees/lx-tool-expanded-markdown`（分支 `feat/lx-tool-expanded-markdown`）完成实现、视觉微调与 Code Review 修复（见 [review.md](./review.md)），待用户确认是否合并到 `dev`**。
> 日期：2026-10-31。
> 项目：`/Users/yonah/projects/agent/lx-tui`。
> 作用域：`src/ui/main_content/lx/history/`、`src/ui/markdown_render.rs`、`src/ui/style.rs` 及配套设计规范 `docs/standards/tui-design-requirements.md`。
> 配套文档：[task.md](./task.md) · [review.md](./review.md)。

---

## 1. 背景与目标

在 `lx` 聊天历史视图（`src/ui/main_content/lx/history/`）中，**思考块（`Thinking` / `Thought`）** 展开后的内容通过 `ui::markdown_render::render_wrapped` 解析为结构化 Markdown 行（`MdLine`），并在绘制时对每个分段样式叠加 `style::muted()`（即 `span_style.patch(style::muted())`）。这种处理方式具备两个核心优势：
1. **视觉层级克制**：普通文本段（原本为 `Style::default()`）叠加 `Modifier::DIM` 后呈现为柔和暗色，不会像默认亮白正文字体（`style::text()`）那样与主回答（Assistant Body）争夺视觉焦点；
2. **保留 Markdown 语义结构与语法色彩**：`ui::markdown_render` 产出的 GitHub Dark 阅读色板（如行内代码/常量 `#79c0ff`、关键字 `#ff7b72`、字符串 `#a5d6ff`、实体/类型 `#d2a8ff`、列表符号/链接 `#4493f8`、引用/标记 `#7d8590`）以及标题加粗/下划线修饰符在 `.patch(style::muted())` 后得以完整保留并整体压暗。

相比之下，当前其他工具卡片与历史条目在展开内容时存在两类不一致问题：
- **展开后退化为刺眼的默认亮色（`style::text()`）**：`tool::output_style(expanded)`、`report.rs`、`tool/subagent.rs`、`tool/mcp.rs`、`tool/command.rs`、`user_shell.rs`、`tool/search.rs` 在展开态（`expanded == true`）下将输出文本切换为 `style::text()`，导致大段工具输出或报告在点开后与主回答正文同亮；
- **丢失或未接入 Markdown 格式染色**：
  - `report.rs`（`▲ Subagent report`）与 `tool/subagent.rs`（`spawn_agent` / `wait_agent` 展开报告）虽然调用了 `markdown_render::render_wrapped`，但在构造渲染行时丢弃了 `MdLine.spans` 中的 `Style`，将整行拼接成纯文本 `String` 并赋予单一样式；
  - `tool/mcp.rs`（MCP 结果块）、`tool/command.rs`（`exec_command` / `write_stdin` 输出块）、`user_shell.rs`（`!` 本地命令输出块）、`compact.rs`（压缩检查点展开摘要）仅按纯文本折行，未接入 `markdown_render::render_wrapped` 的压暗 Markdown 渲染。

**本设计目标**：
参考思考块展开内容的 Markdown 格式与配色机制（`markdown_render::render_wrapped` + `span_style.patch(style::muted())`），为其他工具/卡片的展开内容统一添加同款压暗 Markdown 格式，并移除「展开后变成默认亮白颜色 `style::text()`」的旧逻辑。

---

## 2. 已确认边界与决策记录（Grill Me）

| 编号 | 决策项 | 已确认方案 | 用户确认依据 |
| --- | --- | --- | --- |
| **D1** | **覆盖范围分类** | 1. **接入压暗 Markdown 渲染（`markdown_render::render_wrapped` + `span_style.patch(style::muted())`）**：<br>   - `Subagent report` 卡片正文（`report.rs`）<br>   - `spawn_agent` / `wait_agent` 展开后的 `Report:` 正文（`tool/subagent.rs`）<br>   - `MCP` 工具结果块（`tool/mcp.rs` 的 `result_section`）<br>   - `exec_command` / `write_stdin` 命令输出块（`tool/command.rs` 的 `output_section`）<br>   - 用户 `!` 本地命令输出块（`user_shell.rs` 的 `output_section`）<br>   - `Compact` 压缩检查点展开后的 `summary` 摘要（`compact.rs`）<br>2. **仅移除展开变默认亮色（保持 `style::muted()`，不解析 Markdown）**：<br>   - `search` 工具的代码命中行（`tool/search.rs` 的 `hit_line`），防止代码行中的 `#`、`*` 等字符被误解析为 Markdown 标题或列表。<br>3. **保持现有专有样式不变**：<br>   - `edit` / `write` 的文件 Diff 卡片（`tool/file_diff.rs`）及各工具顶部的入参路径/命令文本（`style::chat_tool_param()`）。 | 问题 1：用户确认「使用推荐」 |
| **D2** | **折叠预览与展开态同源渲染** | 对于未展开时带 2~3 行（或首 3 尾 2 行）折叠预览的卡片（`report.rs`、`tool/mcp.rs`、`tool/command.rs`、`user_shell.rs`），**折叠预览态与展开态全程共用同一份 `muted_markdown_lines` 物理行序列**：折叠态直接通过 `Preview`（或行上限）截取可见行，展开态展示全部行。既保证折叠预览行也具备柔和 Markdown 染色，又保证折叠态提示的 `… N more lines` 与展开后的实际物理行数 100% 严格相等，绝不因围栏折叠或表格收敛产生行数跳变。 | 问题 2：用户确认「使用推荐」 |
| **D3** | **附属标签行与规范/测试同步** | 1. `report.rs` 与 `tool/subagent.rs` 中的 `Report:` 标签行、`tool/command.rs` 中 `Send input` 的输入预览行、`user_shell.rs` 的 `(no output)` 占位行统一使用恒定 `style::muted()`（彻底移除 `tool::output_style(expanded)` 中展开变 `style::text()` 的分支）；<br>2. 同步更新 `docs/standards/tui-design-requirements.md` 中关于「折叠态 `muted()`、展开态恢复 `style::text()`」的旧描述，以及相关单元测试断言。 | 问题 3：用户确认「使用推荐」 |
| **D4** | **展开内容保留 `---` 字面量（不画整行边框线）** | 工具/卡片展开内容（`muted_markdown_lines`）调用 `markdown_render::render_wrapped_literal_rules`，遇到 `---` / `***` / `___` 时保留原始文本（如 `---`），不扩展成整行 `─` 分割线；思考块与主回答正文仍用 `render_wrapped` 保持不变。 | 用户实机截图反馈 1 |
| **D5** | **Markdown 标题（`# title`）与引用块（`> `）样式调整** | 1. 所有级别标题（`h1`–`h6`，`style::md_read_heading_level`）统一为同一种格式：GitHub Dark 实体紫高亮（`#d2a8ff`）+ 加粗 + 下划线（`Modifier::BOLD \| Modifier::UNDERLINED`），不含斜体；<br>2. `> ` 引用块（`style::md_read_quote`）字体颜色调整为与底栏模型上下文占用同款的灰色（`style::chat_provider()` = `style::muted()`），避免使用 `#7d8590` 暗灰色叠加 `DIM` 后过暗。 | 用户实机截图反馈 1–3 |

---

## 3. 代码检索与索引覆盖核验

本设计严格结合 `codegraph` 与 `codebase-memory-mcp`（项目标识 `Users-yonah-projects-agent-lx-tui`）进行结构与调用链分析，并对全部涉及路径执行了 `check_index_coverage` 校验：

| 文件路径 | 覆盖状态 (`check_index_coverage`) | 核心职责与本次改动定位 |
| --- | --- | --- |
| `src/ui/main_content/lx/history/paint.rs` | `no_recorded_issue` (`metadata_match`) | 参考基准：`draw_thinking`（第 515–557 行）示范了 `MdLine` 缩进与 `span_style.patch(style::muted())` 的标准画法。 |
| `src/ui/main_content/lx/history/snapshot.rs` | `no_recorded_issue` (`metadata_match`) | 参考基准：`build_step`（第 600–609 行）通过 `markdown_render::render_wrapped` 构建思考块 `body`。 |
| `src/ui/markdown_render.rs` | `no_recorded_issue` (`metadata_match`) | 提供 `render_wrapped(source, width) -> Vec<MdLine>`，处理代码块高亮、列表缩进、表格自适应、标题与行内样式。 |
| `src/ui/main_content/lx/history/tool.rs` | `no_recorded_issue` (`metadata_match`) | 工具渲染总入口：修改 `output_style` 为恒定 `style::muted()`，新增共用转换函数 `muted_markdown_lines` 与 `md_line_segments`。 |
| `src/ui/main_content/lx/history/tool/tree.rs` | `no_recorded_issue` (`metadata_match`) | 树体排版器：`Node::block(lines)` 接收 `Vec<Vec<Segment>>`，通过 `wrap_spans` 与 `Preview` 计算可见行与 `hidden` 省略行数。 |
| `src/ui/main_content/lx/history/tool/subagent.rs` | `no_recorded_issue` (`metadata_match`) | 委派卡片：`report_node` 改用 `muted_markdown_lines`，`Report:` 标签改用 `style::muted()`。 |
| `src/ui/main_content/lx/history/report.rs` | `no_recorded_issue` (`metadata_match`) | 子 Agent 主动汇报卡片：`body_lines` 保留 `Vec<Vec<Segment>>` 压暗 Markdown 样式，移除展开变 `style::text()`。 |
| `src/ui/main_content/lx/history/tool/mcp.rs` | `no_recorded_issue` (`metadata_match`) | MCP 工具卡片：`result_section` 接入 `muted_markdown_lines`（按树体内容列宽排版），移除展开变 `style::text()`。 |
| `src/ui/main_content/lx/history/tool/command.rs` | `no_recorded_issue` (`metadata_match`) | `exec_command` / `write_stdin` 卡片：`output_section` 接入 `muted_markdown_lines`（保留单行长度采样上限），移除展开变 `style::text()`。 |
| `src/ui/main_content/lx/history/user_shell.rs` | `no_recorded_issue` (`metadata_match`) | `!` 本地命令卡片：`output_section` 接入 `muted_markdown_lines`，`(no output)` 与输出正文恒为压暗样式。 |
| `src/ui/main_content/lx/history/compact.rs` | `no_recorded_issue` (`metadata_match`) | 上下文压缩检查点：展开态 `notice.summary` 在 `rows` 与 `draw` 中统一改用 `muted_markdown_lines`。 |
| `src/ui/main_content/lx/history/tool/search.rs` | `no_recorded_issue` (`metadata_match`) | 搜索卡片：`hit_line` 随 `output_style` 保持 `style::muted()`，不做 Markdown 解析。 |

---

## 4. 核心机制与架构设计

### 4.1 思考块（Thinking Block）的渲染基准分析

在 `src/ui/main_content/lx/history/snapshot.rs`（第 600–609 行）与 `paint.rs`（第 541–556 行）中，思考块展开内容的处理流程如下：

```rust
// 1. snapshot.rs：按可用列宽将 Markdown 源码解析并软换行为 Vec<MdLine>
crate::chat::events::PartKind::Reasoning => BlockPaint::Thinking {
    body: if expanded {
        markdown_render::render_wrapped(
            &part.content,
            width.saturating_sub(super::STEP_GUTTER),
        )
    } else {
        Vec::new()
    },
},

// 2. paint.rs (draw_thinking)：保留 line.indent 缩进，并对每个 span_style 叠加 style::muted()
let mut cx = rect
    .x
    .saturating_add(body_indent)
    .saturating_add(line.indent);
for (value, span_style) in &line.spans {
    let cell_style = span_style.patch(style::muted());
    for ch in value.chars() {
        let cell = ch.width().unwrap_or(1).max(1) as u16;
        if cx.saturating_add(cell) > rect.right() {
            break;
        }
        grid.put_char(cx, y, ch, cell_style);
        cx = cx.saturating_add(cell);
    }
}
```

**关键点剖析**：
1. `ratatui::style::Style::patch` 的语义：`span_style.patch(style::muted())` 会在保留 `span_style` 原有前景色（`fg`，例如 GitHub Dark 的代码蓝 `#79c0ff`、关键字珊瑚红 `#ff7b72`、列表符号蓝 `#4493f8`）和原有修饰符（`BOLD`、`ITALIC`、`UNDERLINED`）的基础上，合并 `style::muted()` 的 `Modifier::DIM`。
2. 当 `span_style` 是普通正文（`Style::default()`）时，`span_style.patch(style::muted())` 的结果恰好等于 `style::muted()`（终端默认前景 + `DIM`）。
3. `MdLine` 的结构：
   ```rust
   pub struct MdLine {
       pub indent: u16,
       pub hang: u16,
       pub spans: Vec<(String, Style)>,
   }
   ```
   当调用 `markdown_render::render_wrapped(source, body_width)` 时，`wrap_md_line` 已经将每一行（含列表/引用的悬挂缩进 `hang`）拆分为显示宽度严格 `<= body_width` 的物理行，并将每行的左缩进记录在 `MdLine::indent` 中。

### 4.2 树体节点（`tree::Node`）与 `MdLine` 的无损桥接

工具卡片（`subagent`、`mcp`、`command`、`user_shell`）统一通过 `tree::layout(&sections, width)` 排版，而 `report.rs` 与 `compact.rs` 则通过 `put_segments` 直接绘制分段行。
为了避免在各模块重复编写 `MdLine -> Vec<Segment>` 的转换逻辑，我们在 `src/ui/main_content/lx/history/tool.rs` 中提供统一的纯函数辅助接口：

```rust
/// 将 markdown 源码按给定列宽排版为思考块同款的压暗多段物理行（`span_style.patch(style::muted())`）。
///
/// 每条返回的 `Vec<Segment>` 对应一个已按 `width` 折好行的物理行（含 `MdLine::indent` 前导空格），
/// 显示总列宽 `<= width.max(1)`；送入 `tree::Node::block` 后经 `wrap_spans(logical, body)` 保持 1:1 物理行映射。
pub(super) fn muted_markdown_lines(source: &str, width: u16) -> Vec<Vec<Segment>> {
    if source.trim().is_empty() {
        return Vec::new();
    }
    crate::ui::markdown_render::render_wrapped(source, width.max(1))
        .into_iter()
        .map(md_line_segments)
        .collect()
}

/// 单行 [`MdLine`] 转为思考块同款的压暗 [`Segment`] 序列：
/// - `line.indent > 0` 时在行首补等宽空格段（`style::muted()`）；
/// - 每个非空分段保留自身 markdown 语法前景色与修饰符，并叠加 `style::muted()`（`DIM`）。
pub(super) fn md_line_segments(line: crate::ui::markdown_render::MdLine) -> Vec<Segment> {
    let mut segments = Vec::with_capacity(line.spans.len() + usize::from(line.indent > 0));
    if line.indent > 0 {
        segments.push(Segment::new(
            " ".repeat(usize::from(line.indent)),
            style::muted(),
        ));
    }
    for (value, span_style) in line.spans {
        if !value.is_empty() {
            segments.push(Segment::new(value, span_style.patch(style::muted())));
        }
    }
    segments
}
```

**几何不变量证明（为什么不会导致 `rows` 与 `draw` 漂移）**：
1. 在 `tree::layout(sections, width)` 中，树体内容区可用列数为 `body = tree::content_width(width)`（大于 `TREE_COLS` 时为 `width - 5`，否则为 `width.max(1)`）。
2. 各工具模块调用 `muted_markdown_lines(source, body_width)` 时传入的正是 `u16::try_from(tree::content_width(width)).unwrap_or(u16::MAX)`。
3. `markdown_render::render_wrapped` 产出的每一行 `MdLine`，其 `usize::from(line.indent) + spans 显示宽度` 恒 `<= body_width`。
4. 当 `Node::block(lines)` 进入 `tree::layout` 时，`wrap_spans(logical, body)` 对每个 `logical` 行检查列宽：由于 `logical` 的总显示宽度已经 `<= body`，`wrap_spans` 不会二次断行，每个 `MdLine` 精确对应 `tree::Layout` 中的 1 个物理行。
5. 折叠态 `Preview::head(...)` / `Preview::head_tail(...)` 与展开态操作的是同一个 `lines` 向量，因此：
   - `section_total` 在折叠与展开态下完全一致；
   - `hidden = section_total.saturating_sub(shown)` 与展开后实际新增的物理行数完全相等；
   - `rows()`、`draw()` 与 `card_regions()` 共用同一个 `layout` 函数，滚动条、置顶行与点击命中矩形零漂移。

---

## 5. 各模块详细改动与关键代码

### 5.1 `src/ui/main_content/lx/history/tool.rs`

**改动点**：
1. `output_style(expanded: bool) -> Style`：移除 `if expanded { style::text() }` 分支，统一返回 `style::muted()`（保留参数签名或更新注释，使 `search.rs`、`command.rs`、`user_shell.rs` 的非 Markdown 附属行也自动保持 `style::muted()`）。
2. 新增 `muted_markdown_lines`、`md_line_segments` 及用于命令/多行文本预净化的 `sample_markdown_source` 辅助函数。

**关键代码对比**：
```rust
// 修改前（src/ui/main_content/lx/history/tool.rs:306-317）
pub(super) fn output_style(expanded: bool) -> Style {
    if expanded {
        style::text()
    } else {
        style::muted()
    }
}

// 修改后
/// 输出区与附属说明的基礎文字档位：折叠与展开均保持 `style::muted()`（dim），
/// 不在展开后切换为默认亮白前景 `style::text()`，避免抢夺主回答正文的视觉焦点。
pub(super) fn output_style(_expanded: bool) -> Style {
    style::muted()
}

/// 将多行原始输出按行做单行长度采样（上限 [`MAX_OUTPUT_LINE_CHARS`]）并保留行内前导缩进，
/// 供 [`muted_markdown_lines`] 进行有界 Markdown 渲染。
pub(super) fn sample_markdown_source(raw: &str) -> String {
    if raw.is_empty() {
        return String::new();
    }
    let mut out = Vec::new();
    for line in raw.split('\n') {
        let line = line.trim_end_matches('\r');
        out.push(sample_output_line(line));
    }
    while out.last().is_some_and(String::is_empty) {
        out.pop();
    }
    out.join("\n")
}
```

---

### 5.2 `src/ui/main_content/lx/history/tool/subagent.rs`（委派卡片展开报告）

**改动点**：
- `report_node(body: &str, width: u16) -> Node`（第 219–233 行）：
  - 原先将 `markdown_render::render_wrapped` 的 `line.spans` 拼成纯文本 `String` 并全部涂成 `style::text()`；
  - 现改为：`Report:` 标题行使用 `style::muted()`，正文直接调用 `super::muted_markdown_lines(body, body_width)`，完整保留思考块同款压暗 Markdown 样式。

**关键代码对比**：
```rust
// 修改前（src/ui/main_content/lx/history/tool/subagent.rs:218-233）
fn report_node(body: &str, width: u16) -> Node {
    let mut lines: Vec<Vec<Segment>> =
        vec![vec![Segment::new(text::agent::REPORT_LABEL, style::text())]];
    for line in markdown_render::render_wrapped(
        body,
        u16::try_from(tree::content_width(width)).unwrap_or(1),
    ) {
        let mut text = " ".repeat(usize::from(line.indent));
        for (value, _) in line.spans {
            text.push_str(&value);
        }
        lines.push(vec![Segment::new(text, style::text())]);
    }
    Node::block(lines)
}

// 修改后
/// 展开态报告：`Report:` + 压暗 markdown 正文，整块一个多行节点。
fn report_node(body: &str, width: u16) -> Node {
    let body_width = u16::try_from(tree::content_width(width)).unwrap_or(1);
    let mut lines: Vec<Vec<Segment>> =
        vec![vec![Segment::new(text::agent::REPORT_LABEL, style::muted())]];
    lines.extend(super::muted_markdown_lines(body, body_width));
    Node::block(lines)
}
```

---

### 5.3 `src/ui/main_content/lx/history/report.rs`（`▲ Subagent report` 卡片）

**改动点**：
1. `body_lines`（第 70–82 行）：返回值由 `Vec<String>` 改为 `Vec<Vec<Segment>>`，直接调用 `super::tool::muted_markdown_lines(&report.body, body_width)`，不再丢弃 `MdLine.spans` 的样式。
2. `BodyView`（第 85–88 行）：`lines` 字段类型改为 `Vec<Vec<Segment>>`。
3. `draw`（第 233–260 行）：
   - 移除 `let body_style = if expanded { style::text() } else { style::agent_step_detail() };`；
   - `Report:` 标签行固定使用 `style::muted()`；
   - 正文行遍历 `&body.lines`，通过 `put_segments(grid, body_x, y, body_width, line)` 绘制多段压暗 Markdown 样式。

**关键代码对比**：
```rust
// 修改前（src/ui/main_content/lx/history/report.rs:70-88, 233-260）
fn body_lines(report: &SubagentReportView, width: u16) -> Vec<String> {
    let body_width = width.saturating_sub(body_indent(width)).max(1);
    markdown_render::render_wrapped(&report.body, body_width)
        .into_iter()
        .map(|line| {
            let mut text = " ".repeat(usize::from(line.indent));
            for (value, _) in line.spans {
                text.push_str(&value);
            }
            text
        })
        .collect()
}

struct BodyView {
    lines: Vec<String>,
    remaining: usize,
}
// ...
    let body_style = if expanded {
        style::text()
    } else {
        style::agent_step_detail()
    };
    if !body.lines.is_empty() {
        let label = vec![
            Segment::new(
                text::agent::TREE_LEAF.to_string(),
                style::chat_notice_connector(),
            ),
            Segment::new(text::agent::REPORT_LABEL, body_style),
        ];
        if let Some(y) = visible_row(rect, scroll, *row) {
            put_segments(grid, rect.x, y, rect.width, &label);
        }
        *row += 1;
    }

    let indent = body_indent(rect.width);
    let body_x = rect.x.saturating_add(indent);
    let body_width = rect.width.saturating_sub(indent);
    for line in &body.lines {
        if let Some(y) = visible_row(rect, scroll, *row) {
            grid.put_clipped(body_x, y, line, body_style, body_width);
        }
        *row += 1;
    }

// 修改后
fn body_lines(report: &SubagentReportView, width: u16) -> Vec<Vec<Segment>> {
    let body_width = width.saturating_sub(body_indent(width)).max(1);
    super::tool::muted_markdown_lines(&report.body, body_width)
}

struct BodyView {
    lines: Vec<Vec<Segment>>,
    remaining: usize,
}
// ...
    if !body.lines.is_empty() {
        let label = vec![
            Segment::new(
                text::agent::TREE_LEAF.to_string(),
                style::chat_notice_connector(),
            ),
            Segment::new(text::agent::REPORT_LABEL, style::muted()),
        ];
        if let Some(y) = visible_row(rect, scroll, *row) {
            put_segments(grid, rect.x, y, rect.width, &label);
        }
        *row += 1;
    }

    let indent = body_indent(rect.width);
    let body_x = rect.x.saturating_add(indent);
    let body_width = rect.width.saturating_sub(indent);
    for line in &body.lines {
        if let Some(y) = visible_row(rect, scroll, *row) {
            put_segments(grid, body_x, y, body_width, line);
        }
        *row += 1;
    }
```

---

### 5.4 `src/ui/main_content/lx/history/tool/mcp.rs`（MCP 工具结果块）

**改动点**：
1. `result_section(call: &ToolCallView, view: &McpDetailView, width: u16, expanded: bool) -> Option<Section>`：
   - 增加 `width: u16` 参数，计算树体内容列宽 `let body_width = u16::try_from(tree::content_width(width)).unwrap_or(u16::MAX);`；
   - 对 `error_fallback` 或 `view.result` 中的每一行，先做控制字符安全净化并限长 8000 字符（保留行首缩进以支持 Markdown 列表与缩进代码块，不调用会 `.trim()` 剥离前导空格的 `single_line`），拼接为多行文本后送入 `super::muted_markdown_lines(&source, body_width)`；
   - 若 `muted_markdown_lines` 返回空（例如结果行仅含 `---` 以外的空白，但 `view.result` 非空），回退保留单行 `style::muted()`，避免丢失非空结果节点；
   - 图片元数据行 `image_line` 统一使用 `style::muted()`。

**关键代码对比**：
```rust
// 修改后（src/ui/main_content/lx/history/tool/mcp.rs）
fn sanitize_result_line(line: &str, max_chars: usize) -> String {
    line.chars()
        .take(max_chars)
        .map(|ch| if ch.is_control() { ' ' } else { ch })
        .collect::<String>()
        .trim_end()
        .to_string()
}

fn result_section(
    call: &ToolCallView,
    view: &McpDetailView,
    width: u16,
    expanded: bool,
) -> Option<Section> {
    let error_fallback = if view.result.is_empty()
        && matches!(call.status, ToolStatus::Failed | ToolStatus::Interrupted)
    {
        call.error.as_deref().filter(|error| !error.is_empty())
    } else {
        None
    };
    if view.result.is_empty()
        && error_fallback.is_none()
        && view.images.is_empty()
        && view.images_omitted == 0
    {
        return None;
    }
    let body_width = u16::try_from(tree::content_width(width)).unwrap_or(u16::MAX);
    let raw_source = if let Some(error) = error_fallback {
        error
            .lines()
            .take(20)
            .map(|line| sanitize_result_line(line, 8000))
            .collect::<Vec<_>>()
            .join("\n")
    } else {
        view.result
            .iter()
            .map(|line| sanitize_result_line(line, 8000))
            .collect::<Vec<_>>()
            .join("\n")
    };
    let mut lines: Vec<Vec<Segment>> = super::muted_markdown_lines(&raw_source, body_width);
    let more = if let Some(error) = error_fallback {
        error.lines().count().saturating_sub(20)
    } else {
        view.result_more
    };
    if more > 0 {
        lines.push(vec![Segment::new(
            text::mcp::more_lines(more),
            style::fold_hint(),
        )]);
    }
    for image in &view.images {
        lines.push(vec![Segment::new(
            text::mcp::image_line(&image.mime, image.width, image.height, &image.path),
            style::muted(),
        )]);
    }
    if view.images_omitted > 0 {
        lines.push(vec![Segment::new(
            text::mcp::images_label(view.images_omitted),
            style::fold_hint(),
        )]);
    }
    if lines.is_empty() {
        return None;
    }
    Some(Section::new(
        vec![Node::block(lines)],
        (!expanded).then(|| Preview::head(BLOCK_PREVIEW_ROWS)),
    ))
}
```

---

### 5.5 `src/ui/main_content/lx/history/tool/command.rs` 与 `src/ui/main_content/lx/history/user_shell.rs`

**改动点**：
1. `command.rs` 的 `output_section` 與 `output_lines`：
   - 先用 `super::sample_markdown_source(output)` 对每行做 `MAX_OUTPUT_LINE_CHARS` 采样截断（保留超长单行的 `…` 截断保证和有界性）；
   - 再调用 `super::muted_markdown_lines(&sampled, u16::try_from(body_width).unwrap_or(u16::MAX))` 生成带压暗 Markdown 样式的多段物理行；
   - `SendInput` 的 `input_preview` 节点继续调用 `output_style(folds.output)`（现恒为 `style::muted()`）。
2. `user_shell.rs` 的 `output_section` 与 `all_output_lines`：
   - 同样通过 `super::tool::sample_markdown_source(&shell.output)` + `super::tool::muted_markdown_lines` 生成 `Vec<Vec<Segment>>`；
   - 当无可见输出行时，显示 `text::LX_SHELL_NO_OUTPUT`（`(no output)`），样式为 `style::muted()`。

**关键代码对比（`command.rs`）**：
```rust
// 修改前（src/ui/main_content/lx/history/tool/command.rs:143-175）
fn output_section(
    output: &str,
    width: u16,
    folded: bool,
    truncated: bool,
    dropped: u64,
) -> Option<Section> {
    let body_width = tree::content_width(width);
    let mut lines: Vec<Vec<Segment>> =
        output_lines(output, u16::try_from(body_width).unwrap_or(u16::MAX))
            .into_iter()
            .map(|line| vec![Segment::new(line, output_style(!folded))])
            .collect();
    // ...

// 修改后
fn output_section(
    output: &str,
    width: u16,
    folded: bool,
    truncated: bool,
    dropped: u64,
) -> Option<Section> {
    let body_width = u16::try_from(tree::content_width(width)).unwrap_or(u16::MAX);
    let sampled = super::sample_markdown_source(output);
    let mut lines: Vec<Vec<Segment>> = super::muted_markdown_lines(&sampled, body_width);
    if truncated {
        lines.push(vec![Segment::new(
            text::agent::OUTPUT_LIMITED,
            style::agent_warn(),
        )]);
    }
    if dropped > 0 {
        lines.push(vec![Segment::new(
            text::agent::dropped_bytes_label(dropped),
            style::agent_warn(),
        )]);
    }
    if lines.is_empty() {
        return None;
    }
    Some(Section::new(
        vec![Node::block(lines)],
        folded.then(|| Preview::head_tail(OUTPUT_PREVIEW_HEAD, OUTPUT_PREVIEW_TAIL)),
    ))
}
```

**关键代码对比（`user_shell.rs`）**：
```rust
// 修改后（src/ui/main_content/lx/history/user_shell.rs:75-101）
fn output_section(shell: &UserShellView, width: u16, folded: bool) -> Option<Section> {
    if shell.is_running() {
        return None;
    }
    let body_width = u16::try_from(tree::content_width(width)).unwrap_or(u16::MAX);
    let sampled = super::tool::sample_markdown_source(&shell.output);
    let rows = super::tool::muted_markdown_lines(&sampled, body_width);
    let mut lines: Vec<Vec<Segment>> = if rows.is_empty() {
        vec![vec![Segment::new(
            text::LX_SHELL_NO_OUTPUT,
            style::muted(),
        )]]
    } else {
        rows
    };
    if shell.output_truncated() {
        lines.push(vec![Segment::new(
            text::agent::dropped_bytes_label(shell.dropped_bytes),
            style::agent_warn(),
        )]);
    }
    Some(Section::new(
        vec![Node::block(lines)],
        folded.then(|| Preview::head_tail(PREVIEW_HEAD, PREVIEW_TAIL)),
    ))
}
```

---

### 5.6 `src/ui/main_content/lx/history/compact.rs`（压缩检查点展开摘要）

**改动点**：
- `rows` 与 `draw` 中展开态的 `notice.summary` 原本使用 `super::wrap(&notice.summary, width)` 纯文本折行并绘制为无高亮 `style::muted()`；
- 现统一改用 `super::tool::muted_markdown_lines(&notice.summary, width)`，`rows` 取其 `.len()`，`draw` 通过 `super::tool::put_segments` 绘制，使 `/compact` 产出的结构化 Markdown 摘要在展开后具备与思考块一致的压暗 Markdown 格式与色彩。

**关键代码对比**：
```rust
// 修改前（src/ui/main_content/lx/history/compact.rs:19-27, 88-93）
pub(super) fn rows(notice: &CompactNoticeView, count: usize, width: u16, expanded: bool) -> usize {
    let mut total = super::wrap(&title(count), width).len();
    total += super::wrap(&description(notice), width).len();
    if expanded {
        total += super::wrap(&range_line(notice), width).len();
        total += super::wrap(&notice.summary, width).len();
    }
    total
}
// ...
    for line in super::wrap(&notice.summary, content.width) {
        if let Some(y) = visible_row(content, scroll, *row_index) {
            grid.put_clipped(content.x, y, &line, style::muted(), content.width);
        }
        *row_index += 1;
    }

// 修改后
pub(super) fn rows(notice: &CompactNoticeView, count: usize, width: u16, expanded: bool) -> usize {
    let mut total = super::wrap(&title(count), width).len();
    total += super::wrap(&description(notice), width).len();
    if expanded {
        total += super::wrap(&range_line(notice), width).len();
        total += super::tool::muted_markdown_lines(&notice.summary, width).len();
    }
    total
}
// ...
    for segments in super::tool::muted_markdown_lines(&notice.summary, content.width) {
        if let Some(y) = visible_row(content, scroll, *row_index) {
            super::tool::put_segments(grid, content.x, y, content.width, &segments);
        }
        *row_index += 1;
    }
```

---

### 5.7 `src/ui/main_content/lx/history/tool/search.rs`（搜索命中行）

**改动点**：
- `hit_line`（第 39–55 行）调用 `output_style(expanded)`；随着 `tool::output_style` 统一返回 `style::muted()`，搜索命中行在折叠与展开态下均保持 `style::muted()`（不再在展开后变回 `style::text()`），且保持纯文本代码行展示（不解析 Markdown）。

---

## 6. 规范文档与测试同步清单

1. **`docs/standards/tui-design-requirements.md`**：
   - 第 125 行、第 147 行、第 158 行：将工具输出区、汇报块、MCP 结果块「展开为终端默认前景 `style::text()`」更新为「折叠与展开均使用思考块同款压暗 Markdown 格式（`markdown_render::render_wrapped` + `span_style.patch(style::muted())`），`search` 命中行展开保持 `style::muted()`，移除展开后恢复默认亮白前景 `style::text()` 的逻辑」。
2. **受影响单元测试更新与新增**：
   - `src/ui/main_content/lx/history/report.rs`：更新 `renders_as_a_report_card_with_triangle_title`（展开态 `Report:` 与普通段落保持 `style::muted()`），新增报告内含 Markdown（如行内代码 `` `code` ``、标题、列表）时保留压暗语法色（`style::md_read_inline_code().patch(style::muted())`）的测试；
   - `src/ui/main_content/lx/history/tool/subagent.rs`：新增 `spawn_agent` / `wait_agent` 展开报告的压暗 Markdown 样式断言；
   - `src/ui/main_content/lx/history/tool/mcp/tests.rs`：更新 `mcp_card_registers_a_click_region_and_connector_colors` 与 `collapsed_leaf_uses_dim_and_expanded_leaf_uses_default_foreground`，并新增 MCP 结果含 Markdown 语法时的压暗高亮测试；
   - `src/ui/main_content/lx/history/tool/command.rs` 与 `src/ui/main_content/lx/history/tool/tests/background_cards.rs`：更新展开态输出样式断言为 `style::muted()` 及压暗 Markdown 样式；
   - `src/ui/main_content/lx/history/user_shell.rs`：更新 `params_stay_light_yellow_and_output_follows_expanded_only` 断言；
   - `src/ui/main_content/lx/history/tool/search.rs`：更新 `folded_search_hits_are_dim_until_expanded` 与 `search_match_text_is_grid_text_for_copy` 断言为展开后仍保持 `style::muted()`；
   - `src/ui/main_content/lx/history/tests/compact.rs`：新增展开 `summary` 含 Markdown 行内代码时的压暗高亮与几何一致性断言。

---

## 7. 风险与边界评估

- **性能与行预算**：所有工具卡片与汇报块已各自具备严格的内容截断上限（`command.rs` 单行 `MAX_OUTPUT_LINE_CHARS` + 尾部窗口、`mcp.rs` 最多 20 行、`report.rs` 按 24 行批次展开），且二级块缓存 `PaneCache` 按 `BlockKey` 缓存块高度，不会引入全量重排性能退化。
- **几何一致性**：`rows()`、`draw()` 与 `card_regions()` 共享同一份 `muted_markdown_lines` 输出，折行宽度均取自各自容器的真实内容列宽（`tree::content_width(width)` 或 `body_width`），不会破坏滚动条、置顶行或点击命中区域。
