# MCP 身份卡、read 行数与合并组间隔 — 技术设计

- 任务目录：`docs/feature/mcp-identity-card-and-group-spacing/`
- 目标代码：`src/ui/main_content/lx/history/tool/mcp.rs`、`src/ui/main_content/lx/history/tool/mcp/tests.rs`、`src/ui/main_content/lx/history/tool/group.rs`、`src/ui/main_content/lx/history/tool/file_summary.rs`、`src/ui/main_content/lx/history/tool.rs`、`src/ui/main_content/lx/history/snapshot/steps.rs`、`src/ui/text/agent.rs`
- 目标文档：`docs/standards/lx-message-block-display.md`、`docs/standards/tui-design-requirements.md`
- 状态：已合入 `dev`（2026-11-14，fast-forward 至 `41fc6751`）；真实 TUI 视觉验收仍按 [`verification.md`](verification.md) V14 由用户执行。

---

## 1. 背景

当前 lx 消息列表里三处显示不符合用户预期：

1. **MCP 卡片信息过载**：`tool/mcp.rs` 会渲染参数块（`CardSection::Body`）与结果块（`CardSection::Output`），折叠态各显示 3 行、展开态最多 20 行；参数是 JSON 字段噪音，结果也很少有人逐行看。用户要求改成**纯身份卡**：只回答“哪个 server、调了哪个 tool、花了多久、有没有图片、失败原因是什么”，不再显示参数与结果。
2. **`read` 卡片丢掉了行数**：`file_summary.rs` 为了“单行路径”把 `ToolSummary::read_lines` / `range` 全部隐藏，用户看不出这次到底读了多少行。数据已经存在（结果体 `start_line` / `end_line` 解析而来），只是没有渲染。
3. **合并组内部过密**：连续同名工具组内“组头 + 调用体 + 调用体”紧贴，调用边界只能靠连接符分辨；同时 `wait_agent` 被列表合并，多次等待的目标集合与结果被压成一组，语义上不该合并。

本任务一次解决这四件事，并同步规范文档。

---

## 2. 目标与非目标

### 2.1 目标

1. MCP 卡片改为纯身份卡：
   - 单个：`■ MCP · 120ms` + `└─ server · tool`；
   - 合并：`■ MCP · N calls` + 每个调用 `server · 耗时` 一级 + `tool` 二级；
   - 所有相邻 MCP 调用合并成一组（含 MCP resources 桥接工具），不看 server / tool 是否相同；
   - 不再显示参数与结果正文，不再登记任何点击面；
   - 失败/中断追加一条 `failed: ...` / `interrupted: ...` 原因行；
   - 图片张数保留在标题/首节点状态里。
2. `read` 卡片在路径后显示**结果体确认过的**行区间与实际读取行数：`path:12-40 · 29 lines`。
3. `wait_agent` 退出合并逻辑，每次等待独立成卡。
4. 所有列表合并组在调用体之间插入一行分隔（非末尾调用画 `  │`）。
5. 更新 `docs/standards/lx-message-block-display.md`，并同步 `docs/standards/tui-design-requirements.md` 中冲突的段落。

### 2.2 非目标

- 不改 `McpDetailView` 的数据结构与落库 JSON 形状（`arguments` / `result` / `images` 仍照常解析，只是渲染层不再展示参数和结果）。
- 不改 MCP 执行、超时、错误分类逻辑。
- 不改 `read` 的结果体 JSON、数据库 schema 或 `ToolSummary::absorb_result_json` 的解析口径。
- 不引入新的空行累加点：分隔行仍属于工具调用体块，不经过 `history::snapshot::spacer()`。

---

## 3. 需求确认记录

| # | 问题 | 结论 |
| --- | --- | --- |
| 1 | MCP 是否连结果也不显示 | **参数与结果都不显示**，纯身份卡，不可展开 |
| 2 | `read` 行数格式 | **`path:12-40 · 29 lines`** |
| 3 | 合并态 MCP 结构 | **保持两级**：server 一级带耗时，tool 缩进一级 |
| 4 | 合并组空行的范围 | **所有列表合并组统一加**，非末尾调用尾部画 `  │` |
| 5 | `wait_agent` 合并 | **完全退出合并**，其余子 agent 工具不变 |
| 6 | MCP 合并键 | **所有相邻 MCP 调用合并**，不区分 server/tool |
| 7 | MCP resources 桥接工具 | **与 MCP 工具同组同规则** |
| 8 | 失败/中断是否显示原因 | **保留一条原因行**，单行尾部省略 |
| 9 | 运行中的 `read` 行数 | **只显示结果体确认过的范围与行数**，运行中只显示路径 |
| 10 | MCP 图片计数 | **保留**，单个在标题、合并挂到 server 行尾 |

---

## 4. 显示规格

### 4.1 MCP 单体（成功）

```text
■ MCP · 120ms
  └─ filesystem · read_file
```

带图片：

```text
■ MCP · 120ms · 1 image(s)
  └─ filesystem · read_file
```

规则：

- 标题动词固定 `MCP`（新增 `text::agent::VERB_MCP`），右侧状态为 `耗时 [· 图片数]`；图片数包含 `images_omitted`。失败/中断/取消时状态置空、整行走语义色。
- 身份行由两段组成：server 浅黄（`style::chat_tool_param()`）、` · ` 用次要信息色（`style::agent_step_detail()`），tool 名用正常文字色（`style::text()`）。
- 无参数、无结果、无 `… N more lines`、无 `[truncated]`、无点击面。
- 身份行超宽时按普通树体规则折行（不做 read 那样的强制单行截断），保证真实长 server/tool 名不丢信息；进入布局前先 `single_line(..., TEXT_SAMPLE_CHARS)`，控制字符转空格并受 512 字符安全上限约束。

### 4.2 MCP 单体（失败/中断/取消）

```text
■ MCP
  ├─ filesystem · read_file
  └─ failed: server not connected
```

规则：

- 失败/中断时标题不写耗时（沿用现有 `status_of` 行为），标题与首节点连接符走红/黄语义色。
- 原因行是第二个一级节点：`failed: ` / `interrupted: ` 前缀取 `text::tool_status_label(status)`，原因先 `single_line(reason, TEXT_SAMPLE_CHARS)` 再按内容宽度尾部省略，**恒一行**。中断的稳定错误码与状态词同为 `interrupted` 时只输出一次。
- 原因为空时失败/中断不画原因行；`Cancelled` 无原因时显示一行 `cancelled`。
- 合并态由 `decorate_group_call` 在首节点尾部补 ` · failed` / ` · interrupted`（仅原因为空时）；`Cancelled` 作为 tool 的兄弟节点显示 `cancelled`。

### 4.3 MCP 合并

```text
■ MCP · 2 calls
  ├─ codegraph · 38ms
  │  └─ codegraph_explore
  │
  └─ codebase-memory-mcp · 38ms
     └─ list_projects
```

规则：

- 组头 `■ MCP · N calls`，所有相邻 MCP 调用进同一组，不再按 `mcp__server__tool` 全名区分。
- 每个调用体两个一级/二级节点：server（一级、浅黄，尾接 `耗时 [· 图片数]`）、tool（二级、正常文字色）。
- 调用体之间由统一的合并组分隔行承担空行（见 §6）：非末尾调用尾部多一行 `  │`。
- 失败/中断调用的原因行是 tool 的兄弟二级节点：

```text
■ MCP · 2 calls · 1 failed
  ├─ filesystem
  │  ├─ read_file
  │  └─ failed: server not connected
  │
  └─ codegraph · 38ms
     └─ codegraph_explore
```

### 4.4 `read`

运行中（结果未回）：

```text
■ Read · file
  └─ src/a.rs
```

完成：

```text
■ Read · file
  └─ src/a.rs:12-40 · 29 lines
```

规则：

- 路径浅黄；`:12-40 · 29 lines` 整段后缀用 `style::agent_step_detail()`（muted），与路径区分。
- `range == (start, end)` 且 `read_lines == Some(n)` 才显示。`start == end` 时只写 `:12`。
- 两个字段都只来自结果体（`ToolSummary::absorb_result_json`），运行中/旧记录没有结果体时只显示路径，绝不拿参数 `offset`/`limit` 预估。
- 目录（`folder`）、图片（`image`）不显示行数；目录条目数仍在标题 `folder · N files`。
- 仍保持单行：先按内容宽度为后缀预留列，再截断路径；后缀放不下时丢弃后缀、只截断路径。路径恰好放得下时，1 列内容即可保留后缀；只有路径需要省略时才要求 2 列（1 列内容 + `…`）。路径尾部省略符为 `…`。
- 组合字符/零宽字符不占列：截断与折行统一按 `ch.width().unwrap_or(0)` 计宽，NFD 路径也不会被拆成多行。
- 合并组内每个 `read` 调用体同样带行数，因为组内不再显示标题状态，行数必须挂在路径行上。

### 4.5 `wait_agent`

```text
■ Wait agents · 2 targets · all · until done
  ├─ ■ explorer · completed
  └─ ■ executor · running

■ Wait agents · 1 target · any · until done
  └─ ■ explorer · running
```

规则：

- `wait_agent` 在 `is_groupable` 直接返回 `false`；连续调用各自成卡，卡片之间走正常块间空行。
- 单卡内部多目标仍然全部是一级平级分支（`layout_with_roots` 的现有行为不变）。

### 4.6 合并组通用分隔

```text
■ Search · 2 calls · 5 matches
  ├─ "fixme" in src · 3 matches
  │  └─ src/ui/main_content/lx/history.rs:42: // FIXME...
  │
  └─ "todo" in docs · 2 matches
     └─ docs/standards/tui-design-requirements.md:120: TODO...
```

规则：

- 所有 `GroupMode::List` 的合并组都生效；`GroupMode::Snapshot` 只显示一个调用体，天然没有分隔行。
- 分隔行只出现在**非末尾调用**尾部，内容是 5 列树前缀 `  │  `（`text::agent::TREE_BAR`）且无正文；末尾调用不追加，避免组尾与下一个块之间出现双空行。
- 分隔行计入该调用体块的行数与滚动几何，但不计入该调用体的点击面。

---

## 5. 实现方案

### 5.1 MCP 渲染器重写（`tool/mcp.rs`）

删除参数/结果块相关代码：`arguments_section`、`result_section`、`sanitize_result_line`、`BLOCK_PREVIEW_ROWS`、`McpLayout.arguments_rows/result_rows/arguments_expandable/result_expandable`。模块只保留：

```rust
/// MCP 身份卡：标题 `■ MCP · 耗时` + `server · tool`（合并时 server 一级、tool 二级）。
pub(super) struct McpLayout {
    layout: tree::Layout,
}

/// 单个调用的身份行：server 浅黄，` · ` muted，tool 正常文字色；两者先单行化并限长。
fn identity_node(view: &McpDetailView) -> Node {
    Node::new(vec![
        Segment::new(single_line(&view.server, TEXT_SAMPLE_CHARS), style::chat_tool_param()),
        Segment::new(text::LX_CHAT_TOOL_STATUS_SEP, style::agent_step_detail()),
        Segment::new(single_line(&view.tool, TEXT_SAMPLE_CHARS), style::text()),
    ])
}

/// 失败/中断/取消原因行：恒一行；稳定错误码与状态词重复时只输出一次。
fn error_node(call: &ToolCallView, width: u16, in_group: bool) -> Option<Node> {
    if !matches!(
        call.status,
        ToolStatus::Failed | ToolStatus::Interrupted | ToolStatus::Cancelled
    ) {
        return None;
    }
    let label = text::tool_status_label(call.status);
    let reason = call
        .error
        .as_deref()
        .map(|reason| single_line(reason, TEXT_SAMPLE_CHARS))
        .unwrap_or_default();
    let reason = reason.trim();
    let message = if reason.is_empty() {
        if call.status == ToolStatus::Cancelled {
            label.to_string()
        } else {
            return None;
        }
    } else if reason.eq_ignore_ascii_case(label) {
        label.to_string()
    } else {
        format!("{label}: {reason}")
    };
    let body = if in_group {
        tree::nested_content_width(width)
    } else {
        tree::content_width(width)
    };
    Some(Node::text(
        crate::ui::main_content::lx::history::truncate_with_ellipsis(&message, body),
        style::text(),
    ))
}

pub(super) fn layout_with_group(
    call: &ToolCallView,
    view: &McpDetailView,
    width: u16,
    _folds: Folds,
    in_group: bool,
) -> McpLayout {
    let mut sections = Vec::with_capacity(2);
    if in_group {
        // 合并态：server 为一级节点（承载耗时/图片状态），tool 与错误原因为二级子节点。
        sections.push(Section::fixed(vec![Node::text(
            single_line(&view.server, TEXT_SAMPLE_CHARS),
            style::chat_tool_param(),
        )]));
        let mut children = vec![Node::text(
            single_line(&view.tool, TEXT_SAMPLE_CHARS),
            style::text(),
        )];
        children.extend(error_node(call, width, true));
        sections.push(Section::fixed(children));
    } else {
        // 单体：`server · tool` 一行，`└─` 收口；错误原因是兄弟节点。
        let mut roots = vec![identity_node(view)];
        roots.extend(error_node(call, width, false));
        sections.push(Section::fixed(roots));
    }
    McpLayout {
        layout: if in_group {
            tree::layout_in_group(&sections, width, 1)
        } else {
            tree::layout(&sections, width)
        },
    }
}
```

标题与绘制：

```rust
pub(super) fn draw(...) {
    if let Some(y) = visible_row(rect, scroll, *row_index) {
        let segments = header_with_verb(
            call,
            rect.width,
            loading_frame,
            animating,
            text::agent::VERB_MCP,
            status_of(call, view),
        );
        put_segments(grid, rect.x, y, rect.width, &segments);
    }
    *row_index += 1;
    tree::draw(..., &layout_with_group(call, view, rect.width, folds, false).layout.rows);
}

pub(super) fn draw_body(..., last: bool) {
    let mut layout = layout_with_group(call, view, rect.width, folds, true).layout;
    tree::decorate_group_call(&mut layout.rows, call, status_of(call, view), rect.width);
    tree::apply_group_connector(&mut layout.rows, last);
    tree::draw(...);
}
```

`status_of`：完成态返回 `耗时 [· 图片数]`，图片数包含 `images_omitted`；失败/中断/取消返回 `None`。流式早期由 `ToolCallView::new` 直接写入 `McpDetailView`，不等参数接收完成。

### 5.2 MCP 点击面与可展开判定（`tool.rs`）

- `has_expandable_content_with_group` 的 `Mcp` 分支改为 `false`。
- `card_regions_with_group` 的 `Mcp` 分支改为 `Vec::new()`，删除 `mcp_regions`；`split_regions` 仍被 `web_search_regions` 使用，保留。

### 5.3 MCP 合并键/动词（`tool/group.rs`）

```rust
/// MCP 合并键：所有相邻 MCP 调用归同一个 `MCP` 组。
const MCP_GROUP_KEY: &str = "mcp";

pub(crate) fn group_key(call: &ToolCallView) -> String {
    if crate::mcp::is_mcp_or_resource_tool(&call.name) {
        MCP_GROUP_KEY.to_string()
    } else if call.name == "write_stdin" {
        format!("write_stdin:{}", display_verb(call))
    } else {
        call.name.clone()
    }
}

pub(crate) fn is_groupable(call: &ToolCallView) -> bool {
    // ask 是阻塞交互；wait_agent 每次等待的目标集合独立，合并会丢失“等了谁”的语义。
    !matches!(call.name.as_str(), "ask" | "wait_agent")
}

pub(crate) fn display_verb(call: &ToolCallView) -> String {
    match call.detail.as_deref() {
        ...
        Some(ToolDetailView::Mcp(_)) => text::agent::VERB_MCP.to_string(),
        ...
    }
}
```

注意：`group_mode` 对 MCP 仍返回 `GroupMode::List`；`group_key` 用 `is_mcp_or_resource_tool` 而不是只看 `detail`，保证运行中/旧记录即使暂时没有 `McpDetailView` 也能进同一组。

### 5.4 `read` 行数与区间（`file_summary.rs`）

新增只读摘要的路径节点构造函数，替换 `read_path_node` 内部逻辑：

```rust
/// `read` 路径行：`path:12-40 · 29 lines`（单行尾部省略）。
///
/// 行区间与实际行数只认结果体写入的 `summary.range` / `summary.read_lines`；
/// 参数侧的 offset/limit 不参与展示（运行中只显示路径）。
fn read_path_node(call: &ToolCallView, width: u16) -> Option<Node> {
    let mut cache: Option<ToolSummary> = None;
    let path = field(
        call,
        needs_argument_fallback(call, Kind::Read),
        &mut cache,
        |summary| summary.path.clone(),
    )
    .map(|path| single_line(&path, TEXT_SAMPLE_CHARS))
    .filter(|path| !path.is_empty())?;

    let available = tree::content_width(width);
    let suffix = read_line_suffix(call.summary.as_deref());
    let suffix_width = suffix.as_deref().map_or(0, |suffix| suffix.width());
    // 后缀放得下才保留；路径至少保留 1 列 + 省略符。
    let keep_suffix = suffix_width > 0 && available > suffix_width.saturating_add(1);
    let path_budget = if keep_suffix {
        available.saturating_sub(suffix_width)
    } else {
        available
    };
    let path = crate::ui::main_content::lx::history::truncate_with_ellipsis(&path, path_budget);
    if path.is_empty() {
        return None;
    }
    let mut spans = vec![Segment::new(path, style::chat_tool_param())];
    if keep_suffix {
        if let Some(suffix) = suffix {
            spans.push(Segment::new(suffix, style::agent_step_detail()));
        }
    }
    Some(Node::new(spans))
}

/// 结果体确认过的行区间与行数：`:12-40 · 29 lines`。
fn read_line_suffix(summary: Option<&ToolSummary>) -> Option<String> {
    let summary = summary?;
    let (start, end) = summary.range?;
    let lines = summary.read_lines?;
    let range = if start == end {
        format!(":{start}")
    } else {
        format!(":{start}-{end}")
    };
    Some(format!(
        "{range}{}{lines} line{}",
        text::LX_CHAT_TOOL_STATUS_SEP,
        if lines == 1 { "" } else { "s" }
    ))
}
```

说明：

- `read_line_suffix` 已经是“后缀”文本，`keep_suffix` 判断后整体追加；因此后缀宽度包含前面的 `:12-40`。
- `UnicodeWidthStr` 已在 `file_summary.rs` 通过 `tree` 间接可用；若未导入则在文件顶部补 `use unicode_width::UnicodeWidthStr;`。
- 路径 + 后缀整体超宽时只截断路径，后缀完整保留（后缀本身不超过内容宽度）。
- 目录/图片的 `range` / `read_lines` 为 `None`，自然不加后缀。
- `detail_layout_with_group` 的 `Read` 分支继续使用 `read_path_node`，组内也走同一函数；`draw_body` 的 read 分支无需改。

### 5.5 合并组分隔行（`tool/group.rs` + `snapshot/steps.rs`）

`body_rows` 增加组内位置参数：

```rust
/// 调用体行数（不含标题；空详情补 1 行兜底）。
/// 非末尾调用额外占 1 行分隔（空白 + `  │`），由 [`draw_body_animated`] 绘制。
pub(crate) fn body_rows(
    call: &ToolCallView,
    width: u16,
    folds: Folds,
    group: GroupSlot,
) -> usize {
    let total = super::rows_with_group(call, width, folds, true);
    if total == 0 {
        return 0;
    }
    let tree_rows = total.saturating_sub(1).max(1);
    tree_rows.saturating_add(usize::from(!group.is_last()))
}
```

`draw_body_animated` 的兜底填充与分隔行：

```rust
pub(crate) fn draw_body_animated(..., group: GroupSlot, ...) {
    if rect.width == 0 {
        return;
    }
    let last = group.is_last();
    let separator = usize::from(!last);
    let expected = body_rows(call, rect.width, folds, group);
    let tree_expected = expected.saturating_sub(separator);
    let before = *row_index;
    match call.detail.as_deref() { ... }
    let drawn = (*row_index).saturating_sub(before);
    let connector = if last { TREE_LEAF } else { TREE_BRANCH };
    let connector_style = /* 现有 pending 连接符样式 */;
    for _ in drawn..tree_expected {
        if let Some(y) = visible_row(rect, scroll, *row_index) {
            grid.put_clipped(rect.x, y, connector, connector_style, rect.width);
        }
        *row_index += 1;
    }
    if separator {
        if let Some(y) = visible_row(rect, scroll, *row_index) {
            grid.put_clipped(
                rect.x,
                y,
                text::agent::TREE_BAR,
                super::tree::connector_style_of(call),
                rect.width,
            );
        }
        *row_index += 1;
    }
}
```

`snapshot/steps.rs` 调用点：

```rust
let rows = tool::body_rows(call, width, folds, group);
// 点击面不覆盖分隔行：分隔行属于排版留白，点它不应展开上一个调用。
let click_rows = rows.saturating_sub(usize::from(!group.is_last()));
let total_rows = click_rows.saturating_add(1);
let regions = tool::body_card_regions(call, width, folds, total_rows);
legacy(rows, regions, folds)
```

行数来源仍然唯一：`BlockPaint::Legacy.rows` 记录 `body_rows`（含分隔行），`draw_body_animated` 推进行数必须与之相等；`history::snapshot` 只负责累加，不在绘制路径另算。

### 5.6 文案与常量（`ui/text/agent.rs`）

新增：

```rust
/// MCP 卡片与合并组头动词。
pub const VERB_MCP: &str = "MCP";
```

`read_line_suffix` 的复数直接拼 `line/lines`；若项目已有行数文案函数（`file_diff_more_lines` 是“more lines”语义，不适配），不新增全局常量。

---

## 6. 行数与几何约束

1. **单一来源**：MCP/read/分隔行的行数只由 `tool::rows*` / `body_rows` 产生；`snapshot`、滚动条、sticky、点击面、渲染全部读同一份快照。
2. **分隔行归属**：分隔行是“非末尾调用体”的最后一行，属于该块的行数，但不属于任何 `CardSection` 点击面。
3. **MCP 无点击面**：`has_expandable_content_with_group(Mcp)` = false，`card_regions_with_group(Mcp)` = 空；Ctrl+O 命中 MCP 为 no-op，Alt+↑/↓ 的 `is_expandable` 排除 MCP。
4. **缓存指纹**：`tool_body_block_signature` 已包含 `GroupSlot` 与 `Folds`，`group.is_last()` 变化会改变指纹并触发重排；无需新增字段。MCP 布局不再依赖 folds，但签名仍会随 folds 变化，属于可接受的保守失效。
5. **极窄宽度**：`width <= TREE_COLS` 时树前缀整体让位，身份行/原因行按整宽排版；分隔行只在 `width > TREE_NESTED_COLS` 时绘制竖线，更窄时仅保留一行空白（仍计入行数），避免孤立竖线。`width == 0` 时 `rows == 0` 且 `draw` 直接返回。

---

## 7. 测试计划

### 7.1 MCP（`tool/mcp/tests.rs` 重写）

- 单体成功：标题 `MCP · 1.2s`、正文只有 `└─ github · create_issue`；断言**不包含**参数值与结果文本。
- 单体带图片：标题包含 `1 image(s)`。
- 单体失败：`├─ github · create_issue` + `└─ failed: mcp tool timed out after 30s`；标题不含耗时。
- 合并：由快照/组渲染断言 `■ MCP · 2 calls`、server 一级、tool 二级、调用间 `  │` 分隔行。
- 无点击面：`card_regions(...)` 为空、`has_expandable_content(...)` 为 false。
- 长 server/tool 名：宽度不足时不 panic、折行正确。

### 7.2 `read`

- 完成态：`src/a.rs:12-40 · 29 lines`，路径浅黄、后缀 muted。
- `start == end`：显示 `:12`。
- 运行中：只有路径，没有 `range`/`lines`。
- 旧记录（无结果体，仅参数）：只有路径。
- 路径超长：后缀优先保留、路径尾部 `…`；后缀放不下时丢弃后缀。
- 合并组内 read：路径行仍带行数。

### 7.3 合并行为

- `group_key`：两个不同 `mcp__server__tool` 相邻调用得到同一个键；`list_resources` 也进同组。
- `is_groupable`：`wait_agent` 返回 false，`ask` 仍 false，`read`/`spawn_agent` 仍 true。
- 分隔行：两个 `read` 合并时总行数 = 组头 1 + 调用 A（路径 1 + 分隔 1）+ 调用 B（路径 1）；非末尾调用尾部是 `TREE_BAR`，末尾调用尾部不是。
- 快照组（`update_plan` / `list_agents`）：只有一个调用体，不应出现分隔行。
- 三调用组：前两个调用各多 1 行，最后一个不多。

### 7.4 几何回归

- `rows == draw` 推进：所有工具组测试继续断言。
- 点击面不覆盖分隔行：最后一个可点击区域底边 ≤ 树体末行。
- 现有 `history/tests/tool_grouping.rs` 中 MCP 无参用例与 `wait_agent` 合并用例按新规格改写。

---

## 8. 文档同步

### 8.1 `docs/standards/lx-message-block-display.md`

- §2.4 空行：把“工具合并组内部不空行”改为“组头与首个调用紧邻；调用体之间由非末尾调用尾部的一行 `  │` 分隔；末尾调用后不加，组与前后块仍空一行”。
- §3.1 合并键：MCP 按 `mcp` 聚合（不区分 server/tool）；`wait_agent` 不合并。
- §3.3 组内调用：补充分隔行归属与点击面排除规则。
- §4 工具清单：`read` 单体标题不变；MCP 标题改 `■ MCP · 840ms`；`wait_agent` 合并形态改“不合并”。
- §5.1 `read`：删掉“不显示行范围/行数”的旧规则，改为结果体确认后显示 `path:12-40 · 29 lines`。
- §5.10 `wait_agent`：单体示例保留，删除合并示例，写明不合并理由。
- §5.11 MCP：整节重写为身份卡规格（成功/失败/合并/resources），删除参数块、结果块、点击面描述。
- §7 合并算法：注明 `wait_agent` 被 `is_groupable` 排除、MCP 组键为常量 `mcp`。
- §8 几何：组内调用体的行数包含分隔行；分隔行不登记点击面。

### 8.2 `docs/standards/tui-design-requirements.md`

- 第 116 行：更新“合并组内部不空行”的旧口径。
- 第 129 行：更新 MCP 结果/参数展示、`read` 不显示行数与分组键描述。
- 第 149 行：删除“MCP 参数块 / 结果块各自点击面”的描述，改为“MCP 身份卡无点击面”。

---

## 9. 风险与回滚

| 风险 | 影响 | 缓解 |
| --- | --- | --- |
| 分隔行改行数 | 滚动条、sticky、点击偏移错位 | 行数只从 `body_rows` 出；补 `rows == draw` 回归测试 |
| MCP 合并范围扩大 | 一段连续 MCP 调用变一个组，组头可能很长 | 组头只写 `MCP · N calls`，不写 server/tool，天然短 |
| 身份卡隐藏错误细节以外的一切 | 用户无法从消息列表复制结果 | 明确非目标：需要完整结果请用日志/调试；设计已由用户确认 |
| 缓存签名 | 旧缓存与新布局不一致 | 签名已含 group/folds；必要时提升指纹版本由测试覆盖 |
| 文档冲突 | 规范自相矛盾 | 同步 `tui-design-requirements.md`，实施后 grep 旧口径 |

回滚：本任务不涉及数据库与持久化；回滚只需还原上述源码与文档文件。
