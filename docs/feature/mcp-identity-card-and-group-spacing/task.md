# MCP 身份卡、read 行数与合并组间隔 — 执行任务清单

> 状态：代码、测试与规范文档已完成；`cargo fmt` / `cargo clippy -D warnings` / `cargo test` 全部通过，待用户运行 `cargo run` 做视觉验收。
> 关联文档：`docs/feature/mcp-identity-card-and-group-spacing/design.md`
> 规则：生产代码禁止 `unwrap()/expect()`；文案一律英文；单文件 ≤1000 行；改动后按影响范围验证。

---

## 阶段 0：需求确认（已完成）

- [x] 盘问 1：MCP 参数与结果都不显示 → **纯身份卡**
- [x] 盘问 2：`read` 行数格式 → **`path:12-40 · 29 lines`**
- [x] 盘问 3：合并态 MCP → **保持两级**（server 一级带耗时，tool 二级）
- [x] 盘问 4：合并组空行范围 → **所有列表合并组**
- [x] 盘问 5：`wait_agent` → **不合并**
- [x] 盘问 6：MCP 合并键 → **所有相邻 MCP 调用一组**
- [x] 盘问 7：MCP resources 桥接工具 → **同组同规则**
- [x] 盘问 8：失败/中断 → **保留一条原因行**
- [x] 盘问 9：运行中 `read` → **只显示路径，不预估行数**
- [x] 盘问 10：MCP 图片计数 → **保留**

---

## 阶段 1：文案与 MCP 身份卡（`tool/mcp.rs`）

### 1.1 新增 MCP 动词

- [x] `src/ui/text/agent.rs`：在现有 `VERB_*` 常量区新增

```rust
/// MCP 卡片与合并组头动词。
pub const VERB_MCP: &str = "MCP";
```

### 1.2 重写 `src/ui/main_content/lx/history/tool/mcp.rs`

- [x] 删除参数/结果块：
  - `BLOCK_PREVIEW_ROWS`
  - `arguments_section`
  - `result_section`
  - `sanitize_result_line`
  - `McpLayout` 的 `arguments_rows` / `result_rows` / `arguments_expandable` / `result_expandable`
- [x] `McpLayout` 只保留 `layout: tree::Layout`。
- [x] 新增 `identity_node(view)`：server 浅黄 + ` · ` 灰 + tool 正常色。
- [x] 新增 `error_node(call, width, in_group)`：
  - 仅 `Failed` / `Interrupted`；
  - 原因为空返回 `None`；
  - 文案 `format!("{label}: {reason}", label = text::tool_status_label(call.status))`；
  - 宽度：组内 `tree::nested_content_width(width)`，单体 `tree::content_width(width)`；
  - 用 `truncate_with_ellipsis` 保证单行。
- [x] `layout_with_group`：
  - `in_group == false`：一个 `Section::fixed(identity_node)`；
  - `in_group == true`：`Section::fixed(server)` + `Section::fixed(tool)`；
  - 有错误时追加 `Section::fixed(error_node)`；
  - `in_group` 用 `tree::layout_in_group(&sections, width, 1)`，否则 `tree::layout`。
- [x] `rows` / `rows_with_group`：`1 + layout.rows.len()`。
- [x] `draw`：标题动词改用 `text::agent::VERB_MCP`，状态继续用 `status_of`。
- [x] `draw_body`：`decorate_group_call` + `apply_group_connector` 不变。
- [x] 删除 `has_more` 或改为恒 false（若外部仍需该函数）。
- [x] 模块文档注释更新为身份卡说明，删除参数/结果/图片元数据行描述。
- [x] 清理仅被旧 MCP 卡片使用的文案常量（先 `rg` 确认零引用后再删）：
  - `src/ui/text/agent.rs` 的 `MCP_NO_ARGUMENTS`；
  - `src/ui/text/mcp.rs` 的 `DETAIL_ARGUMENTS` / `DETAIL_RESULT` / `IMAGE_LINE_PREFIX` / `image_line` / `more_lines`（保留 `images_label` 与 toast 文案）。

关键代码见 `design.md` §5.1。

### 1.3 关闭 MCP 点击面（`src/ui/main_content/lx/history/tool.rs`）

- [x] `has_expandable_content_with_group` 的 `ToolDetailView::Mcp(_)` 分支改为 `false`。
- [x] `card_regions_with_group` 的 `ToolDetailView::Mcp(_)` 分支改为 `Vec::new()`。
- [x] 删除 `mcp_regions` 函数（`split_regions` 保留给 `web_search_regions`）。
- [x] 检查 `McpLayout` 其他字段引用并清理。

---

## 阶段 2：合并键与 `wait_agent`（`tool/group.rs`）

- [x] 新增常量：

```rust
/// MCP 合并键：所有相邻 MCP 调用归同一个 `MCP` 组。
const MCP_GROUP_KEY: &str = "mcp";
```

- [x] `group_key`：

```rust
pub(crate) fn group_key(call: &ToolCallView) -> String {
    if crate::mcp::is_mcp_or_resource_tool(&call.name) {
        MCP_GROUP_KEY.to_string()
    } else if call.name == "write_stdin" {
        format!("write_stdin:{}", display_verb(call))
    } else {
        call.name.clone()
    }
}
```

- [x] `is_groupable`：

```rust
pub(crate) fn is_groupable(call: &ToolCallView) -> bool {
    !matches!(call.name.as_str(), "ask" | "wait_agent")
}
```

- [x] `display_verb` 的 `Some(ToolDetailView::Mcp(_))` 改为 `text::agent::VERB_MCP.to_string()`。
- [x] 更新模块/函数注释：`ask` 与 `wait_agent` 不合并；MCP 合并键为 `mcp`。

---

## 阶段 3：`read` 行数与区间（`tool/file_summary.rs`）

- [x] 顶部按需补 `use unicode_width::UnicodeWidthStr;`。
- [x] 重写 `read_path_node`：

```rust
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
    let suffix_width = suffix.as_deref().map_or(0, UnicodeWidthStr::width);
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
```

- [x] 新增：

```rust
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

- [x] 更新 `file_summary.rs` 模块注释与 `detail_layout_with_group` 的 read 注释：原“只显示一条路径、不显示行范围/行数”改为“路径 + 结果体确认的区间/行数”。
- [x] 确认 `ToolStatus::Cancelled` / 运行中 / 错误分支行为不变。

---

## 阶段 4：合并组分隔行（`tool/group.rs` + `snapshot/steps.rs`）

- [x] `body_rows` 增加 `group: GroupSlot` 参数：

```rust
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
    total
        .saturating_sub(1)
        .max(1)
        .saturating_add(usize::from(!group.is_last()))
}
```

- [x] `draw_body_animated`：
  - `let separator = usize::from(!last);`
  - `let expected = body_rows(call, rect.width, folds, group);`
  - `let tree_expected = expected.saturating_sub(separator);`
  - 兜底填充循环上限改 `tree_expected`；
  - 循环结束后若 `separator`，画一行 `text::agent::TREE_BAR`，样式用 `super::tree::connector_style_of(call)`，并 `*row_index += 1`。
- [x] `src/ui/main_content/lx/history/snapshot/steps.rs`：

```rust
let rows = tool::body_rows(call, width, folds, group);
// 点击面不覆盖分隔行。
let click_rows = rows.saturating_sub(usize::from(!group.is_last()));
let total_rows = click_rows.saturating_add(1);
let regions = tool::body_card_regions(call, width, folds, total_rows);
legacy(rows, regions, folds)
```

- [x] 其他 `body_rows` 调用点全部改签名；确认无遗漏：

```bash
rg -n "body_rows\(" src
```

- [x] 确认 `GroupMode::Snapshot` 的 `display_count == 1`，天然不加分隔行。
- [x] 更新 `group.rs` 模块注释：组内调用之间加一行 `│` 分隔。

---

## 阶段 5：测试改造

### 5.1 `tool/mcp/tests.rs` 整体重写

删除旧的参数/结果/截断/图片元数据测试，新增：

- [x] `single_card_shows_only_identity_and_duration`：
  - 文案包含 `MCP`、`1.2s`、`github · create_issue`；
  - 不包含参数值 `Fix login crash on iOS`/`{`；
  - 不包含结果 `created #42`；
  - 不包含 `[image`。
- [x] `single_card_keeps_image_count`：标题包含 `1 image(s)`。
- [x] `failed_card_appends_reason_line`：包含 `failed: mcp tool timed out after 30s`。
- [x] `interrupted_card_appends_reason_line`：包含 `interrupted: ...`。
- [x] `mcp_card_registers_no_click_region`：`card_regions(...)` 为空；`has_expandable_content(...)` 为 false。
- [x] `narrow_width_does_not_panic`：宽度 1/4/8 下 rows == draw。
- [x] 保留 `render_with` 的 `row == total` 断言。

### 5.2 `history/tests/tool_grouping.rs`

- [x] 重写 `mcp_no_arg_step` 相关测试：
  - 两个不同 MCP 工具现在合并为一个 `■ MCP · 2 calls`；
  - 断言 server 一级、tool 二级、调用间 `TREE_BAR` 分隔行；
  - 断言总行数 = 组头 1 + 调用 A 2 行 + 分隔 1 + 调用 B 2 行。
- [x] 重写 `multi_target_wait_agent_in_group_keeps_all_targets_at_level_one`：
  - `wait_agent` 不再合并，两次调用各自标题；
  - 每张卡内部多目标仍是一级平级分支；
  - 两卡之间是普通块间空行。
- [x] 更新所有受“组内新增分隔行”影响的行号断言（`read` / `search` / `edit` / `command` 组）。

### 5.3 其他受影响的几何测试

- [x] 运行 `cargo test ui::main_content::lx::history`，按失败信息更新：
  - `history/tests/cards.rs`
  - `history/tests/tool_geometry.rs`
  - `history/tests/compact.rs`
  - `history/tests/structure.rs`
  - `history/tool/tests/*.rs`
- [x] 新增/保留 `rows == draw` 断言，确保分隔行不会导致滚动或命中漂移。
- [x] 检查 `mcp` 卡片不再有 `CardSection::Output` 命中区域。

### 5.4 `read` 测试（`file_summary` 相关测试或 `tool/tests/extras.rs`）

- [x] 完成态：`src/a.rs:12-40 · 29 lines`。
- [x] 单行区间：`:12`。
- [x] 运行中：只有路径。
- [x] 仅参数无结果：只有路径。
- [x] 长路径：后缀优先、路径 `…`。
- [x] 目录/图片：无行数。

---

## 阶段 6：规范文档

- [x] `docs/standards/lx-message-block-display.md` 按 `design.md` §8.1 更新：
  - [x] §2.4 空行口径；
  - [x] §3.1 合并键（MCP 常量 `mcp`、`wait_agent` 不合并）；
  - [x] §3.3 组内调用与分隔行；
  - [x] §4 工具清单（MCP 标题、`wait_agent` 合并形态）；
  - [x] §5.1 `read` 行数与区间；
  - [x] §5.10 `wait_agent` 不合并；
  - [x] §5.11 MCP 身份卡整节重写；
  - [x] §7 合并算法；
  - [x] §8 几何与命中。
- [x] `docs/standards/tui-design-requirements.md` 同步：
  - [x] 第 116 行“合并组内部不空行”；
  - [x] 第 129 行 MCP/read/分组键描述；
  - [x] 第 149 行 MCP 参数/结果块点击面描述。
- [x] 实施后 `rg` 检查旧口径残留：

```bash
rg -n "MCP 卡片的参数块|组内不空行|MCP 结果|read.*不显示.*行数|wait_agent.*列表合并" docs/standards
```

---

## 阶段 7：验证

- [x] `cargo fmt`
- [x] `cargo clippy --all-targets -- -D warnings`
- [x] `cargo test ui::main_content::lx::history`
- [x] `cargo test`（全量）
- [ ] 人工检查 `cargo run`（由用户执行）：
  - 单个 MCP 卡；
  - 两个不同 server 的 MCP 合并卡；
  - 连续两个 `wait_agent`；
  - `read` 完成/运行中；
  - 合并组内空行与点击/滚动；
  - 窄窗格。

> 验证记录（2026-11-14）：`cargo fmt` 无差异；`cargo clippy --all-targets -- -D warnings` 通过；`cargo test` 全量通过。
> 新增/更新的自动化用例覆盖 MCP 身份卡（参数/结果隐藏、图片数、失败原因、无点击面、合并嵌套）、`read` 区间/行数（单行、复数、运行中、旧记录、长路径、窄宽）、`wait_agent` 独立成卡、MCP resources 同组、快照组无分隔行、分隔行不进入点击面。

---

## 验收标准

1. MCP 单体严格为：

```text
■ MCP · 120ms
  └─ filesystem · read_file
```

2. MCP 合并严格为：

```text
■ MCP · 2 calls
  ├─ codegraph · 38ms
  │  └─ codegraph_explore
  │
  └─ codebase-memory-mcp · 38ms
     └─ list_projects
```

3. MCP 卡片不出现参数/结果正文，不响应点击展开。
4. `read` 完成态显示 `path:12-40 · 29 lines`，运行中不显示行数。
5. 连续 `wait_agent` 不合并。
6. 所有列表合并组的调用体之间有一行 `  │` 分隔，组尾不重复空行。
7. `cargo fmt` / `clippy -D warnings` / `cargo test` 全部通过。
8. 两份规范文档与实现一致，无旧口径残留。
