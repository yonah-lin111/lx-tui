# lx 工具长内容截断与消息列表截断展示统一：设计

> 状态：**已实施并完成代码审查修复（2026-11-11），已合并至 `dev`**。
> 日期：2026-11-11。
> 范围：`read`（文件读取 + 目录列表）、`web_search`、`exec_command` / `write_stdin` 返回给模型与落库的 50 KiB / 2000 行 / 单行 2000 字符截断；以及 TUI 消息列表工具卡片 `[truncated]` 展示位置统一与单体 `read` 单行路径显示。
> 明确不包含：DB 迁移、临时文件落盘（opencode `tool-output`）、`search` / MCP / 子 agent 汇报的执行层截断策略变更。
> 配套文档：[task.md](./task.md) · [review.md](./review.md)。
> 相关规范：`docs/standards/project-directory-structure.md`、`code-writing-standards.md`、`tui-design-requirements.md`、`docs/standards/lx-message-block-display.md`。
> 参考实现：`/Users/yonah/projects/agent/opencode-dev`、`/Users/yonah/projects/agent/codex-main`。

## 1. 已确认的产品决策

| 决策 | 已确认行为 |
| --- | --- |
| 覆盖工具 | `read`（文件 + 目录）、`web_search`、`exec_command` / `write_stdin` 统一实施单次 **50 KiB**（JSON 转义后字节预算）+ **2000 行** + **单行 2000 字符**上限，确保 `to_result_json` 稳定低于 64 KiB 回放上限（`MAX_TOOL_RESULT_REPLAY_BYTES`）。 |
| 单次体积 | 对齐 opencode：**50 KiB**（`50 * 1024` 字节）；按落库 JSON 转义后的字节计（零分配字节扫描），路径/查询/命令及元数据另占预算。 |
| 单行宽度 | 对齐 opencode：**2000 字符**，超出截断并在模型文本中显式标记 `…[truncated]`；UI 采样时剥离 `[truncated]` 标签并保留末尾省略号 `…`。 |
| 行数 | 保持 **2000 行**（`read` 的 `limit` 默认值与硬上限、`web_search` 与进程输出总行数上限均为 2000）。 |
| `read` 分页 | 保留 `offset` / `limit` / `next_offset` 的页式截断，不做中间截断；新增 `line_truncated: bool` 与 `entries_truncated: bool` 落库字段。 |
| `web_search` 策略 | 保留单查询 `MAX_QUERY_OUTPUT_CHARS = 12_000`；整次调用按头部保留施加 50 KiB / 2000 行 / 单行 2000 字符上限。 |
| 进程输出策略 | 终态进程超限对齐 codex 保留首尾（head + tail，中间写 `…[N lines omitted]`），运行中进程按 2000 行与转义预算停止推进游标；内置 `rtk` 省略时同步置 `result.truncated = true`。 |
| UI `[truncated]` 唯一位置 | 移除标题栏截断状态（`results truncated` / `truncated`）与独立 `output limited` 节点；`[truncated]`（不带省略号，`text::agent::INLINE_TRUNCATED`）**只允许**出现在 `… N more lines [truncated]` 提示行上。 |
| UI `read` 单行卡片 | 单体 `read` 卡片标题保留 `file` / `folder · N files` / `image`，树体仅显示单行路径或单行错误/状态词（超宽尾部 `…` 省略，不软换行、不显示 `output limited`、不登记可展开点击面）。 |
| 旧数据 | 不迁移 DB；旧结果 JSON 缺少新字段时按 `false` 处理。 |

## 2. 当前实现事实

### 2.1 常量与数据流

当前 `read` 的上限在 `src/chat/tools/mod.rs:88-107`：

```rust
/// `read` 单次返回内容字节上限（256 KiB）：超出按行截断并给出 `next_offset`。
pub const MAX_READ_BYTES: usize = 256 * 1024;

/// `read` 单次返回行数上限（也是 `limit` 的默认值）。
pub const MAX_READ_LINES: usize = 2000;

/// `read` 单行输出字符上限：超长行截断并显式标记（避免单行撑爆上下文）。
pub const MAX_READ_LINE_CHARS: usize = 4000;

/// `read` 单文件输入字节上限（16 MiB）。
pub const MAX_READ_FILE_INPUT_BYTES: u64 = 16 * 1024 * 1024;

/// `read` 单次列目录的条目上限（对齐 codex 的 50 000）。
pub const MAX_READ_DIR_ENTRIES: usize = 50_000;
```

数据流（三个投影必须同源，不能只改其中一个）：

```text
read::run
  ├─ read_file / read_directory
  │    └─ ToolExecutionResult::Read { content, start_line, end_line, next_offset, truncated, ... }
  ├─ format_for_model  -> 模型本轮看到的文本
  ├─ to_result_json    -> DB agent_tool_call.result_json（1 MiB 存储上限）
  └─ to_summary        -> UI 摘要（path / kind / range / total_lines / truncated）
```

关键代码位置（设计时定位，不是未来固定行号）：

| 位置 | 当前行为 |
| --- | --- |
| `src/chat/tools/types.rs:268-292` | `ToolExecutionResult::Read` 只有聚合布尔 `truncated`，无法区分“还有后续行”和“单行被截”。 |
| `src/chat/tools/read.rs:219-253` | 文件循环按 `MAX_READ_BYTES` 与 `limit` 截断；局部 `line_truncated` 只折进聚合 `truncated`。 |
| `src/chat/tools/read.rs:256-264` | `render_line` 超过 `MAX_READ_LINE_CHARS` 时截断并追加 `…[truncated]`。 |
| `src/chat/tools/read.rs:296-375` | 目录循环同样使用 `MAX_READ_BYTES`，局部 `entries_truncated` 只折进聚合 `truncated`。 |
| `src/chat/tools/output.rs:399-465` | `format_read` 的截断提示是笼统的 `long lines and/or output size were cut` / `only the first entries are shown`。 |
| `src/chat/tools/output.rs:687-716` | `to_result_json` 只写聚合 `truncated`。 |
| `src/chat/context/mod.rs:39-42,465-477` | 回放单条工具结果上限 **64 KiB**，超出追加 `…[truncated]`。当前 256 KiB 的 `read` 会先被 read 截一次、再被回放截一次。 |
| `src/db/chat/tools.rs:17-18` | DB 单次工具结果 JSON 上限 1 MiB；只与存储有关，不是模型预算。 |
| `src/app/state/lx/chat/steps.rs:242-344` | `ToolSummary::absorb_result_json` 只读聚合 `truncated`；新增字段不需要 UI 消费。 |
| `src/ui/main_content/lx/history/tool/file_summary.rs:297-343` | `read` 卡片在 `truncated` 为真时显示 `output limited`；本任务不修改该逻辑。 |

### 2.2 当前的实际问题

1. **256 KiB 太大**：一次 `read` 约等于 6 万 token（英文代码），一次调用就能吃掉大半上下文；opencode 的对应上限是 50 KiB，codex 的工具输出按 token 预算截断。
2. **双层截断不可解释**：`read` 先按 256 KiB 返回，进入下一轮采样时 `context` 又按 64 KiB 截一次；模型先看到 read 自己的 `truncated`，再看到 `…[truncated]`，无法判断到底还剩多少。
3. **单行截断不可恢复**：`render_line` 只保留前 4000 字符，`offset` 是按行翻页，永远拿不到该行的后半段；当前提示没有说明这一点。
4. **目录收集上限不可见**：50,000 条触顶后 `total_lines` 是“已收集条目数”，`next_offset` 翻到最后一页也不会出现第 50,001 条；当前提示只说“only the first entries are shown”。

## 3. 参考实现与取舍

### 3.1 opencode

`packages/core/src/tool/read-filesystem.ts`：

```ts
export const MAX_READ_LINES = 2_000
export const MAX_READ_BYTES = 50 * 1024
const MAX_LINE_LENGTH = 2_000
const MAX_LINE_SUFFIX = `... (line truncated to ${MAX_LINE_LENGTH} chars)`
```

- 按行分页，保留 `next`（1-based offset）。
- 单行超过 2000 字符时追加后缀。
- 目录页只按条数分页，没有单独的字节上限（本设计**不采用**这一点，目录也套 50 KiB，理由见 §1）。

`packages/core/src/tool-output.ts`：

```ts
export const MAX_LINES = 2_000
export const MAX_BYTES = 50 * 1024
// 超出后把全文写入 tool-output 临时文件，结果里给路径：
const marker = `[showing ${shown} of ${lines.length}; full output saved to ${file}]`
```

- **采用**：50 KiB 与 2000 行。
- **不采用**：把全文写临时文件。lx 的 `read` 已有 `offset` 页式续读，落盘会引入临时文件生命周期、清理与安全边界，收益不足。

### 3.2 codex

`codex-rs/utils/output-truncation/src/lib.rs::formatted_truncate_text`：

```rust
let header = format!(
    "Warning: truncated output (original token count: {original_token_count})\nTotal output lines: {total_lines}\n\n"
);
let (snippet, edit) = truncate_text_with_plan(content, policy); // Tokens -> middle truncation
```

- **采用**：截断必须显式、必须报告原始规模（lx 已有 `lines X-Y of Z total lines`）。
- **不采用**：`truncate_middle_chars` 的中间截断。`read` 的语义是“从头顺读”，中间截断会让模型永远读不到中段；页式 `offset` 更适合 `read`。

`codex-rs/tui/src/tool_output.rs`：

```rust
const PREVIEW_LINES: usize = 3;
const MAX_PREVIEW_LINE_BYTES: usize = 16 * 1024;
```

- 这是 **UI 预览**，本任务明确不改 UI，不采用。

### 3.3 历史设计的关系

`docs/feature/lx-tool-message-tree-and-preview/design.md` 曾写明“模型工具结果字符串、DB JSON 和既有执行截断上限无变化”。本任务**有意修改执行截断上限**，因此该句在新功能范围内被本设计取代；其余“UI 有界预览、模型结果与 UI 分离”的原则不变。

## 4. 目标行为

### 4.1 常量最终值

| 常量 | 当前 | 目标 | 说明 |
| --- | --- | --- | --- |
| `MAX_READ_BYTES` | `256 * 1024` | `50 * 1024` | 文件与目录共用；按落库 JSON 转义后的 `content` 字节计（普通文本近似 UTF-8 字节，含 `N: ` 行号前缀与换行），路径/元数据另占预算（固定 512 字节 + 转义后的路径）。这样引号/反斜杠密集的文本落库后不会翻倍冲破 64 KiB 回放上限。 |
| `MAX_READ_LINE_CHARS` | `4000` | `2000` | 按字符计；超长行保留前 2000 字符并追加 `…[truncated]`。 |
| `MAX_READ_LINES` | `2000` | `2000`（不变） | `limit` 默认值与硬上限。 |
| `MAX_READ_FILE_INPUT_BYTES` | `16 MiB` | `16 MiB`（不变） | 输入侧上限，与输出上限相互独立。 |
| `MAX_READ_DIR_ENTRIES` | `50_000` | `50_000`（不变） | 收集上限；触顶置 `entries_truncated = true`。 |

模型文本总长上界：`50 KiB` 正文 + 约 300~500 字节头部/尾部；落库 JSON 的正文也按转义后 ≤50 KiB 计，加上字段名/路径余量仍稳定低于回放上限 `64 KiB`。这是选择 50 KiB 而不是 64 KiB 的核心理由。

### 4.2 文件读取算法

```rust
let mut line_truncated = false;
let budget = content_budget(target); // 50 KiB 扣除路径/元数据后的 content 预算
for index in start_index..total_lines {
    if emitted >= limit {
        break;
    }
    let (rendered, clipped) = render_line(index + 1, &source.lines[index]);
    // 预算按落库 JSON 转义后的字节计，避免引号/反斜杠密集的文本翻倍超过 64 KiB。
    let cost = json_escaped_len(&rendered) + 2;
    // 至少输出一行，避免单行超限时死循环或空结果。
    if emitted > 0 && used_bytes + cost > budget {
        break;
    }
    line_truncated |= clipped;
    content.push_str(&rendered);
    content.push('\n');
    used_bytes += cost;
    emitted += 1;
}
let end_line = start_index + emitted;
let next_offset = (end_line < total_lines).then_some(end_line + 1);
// 聚合标志 = 分页截断 || 单行截断
let truncated = next_offset.is_some() || line_truncated;
```

语义：

- `next_offset.is_some()`：后面还有完整行，模型可以用该 offset 继续读。
- `line_truncated`：至少一行超过 2000 字符被切掉尾部；该尾部**不能用 offset 恢复**。
- 两者可以同时为真。
- 空文件、图片：两个新字段均为 `false`。

### 4.3 目录读取算法

```rust
let mut entries_truncated = false;
for entry in entries {
    if names.len() >= caps.dir_entries {
        entries_truncated = true;
        break;
    }
    let rendered = format!("{} {name}", if is_dir { "[dir]" } else { "[file]" });
    let cost = json_escaped_len(&rendered) + 2;
    // ... 与文件读取相同的 content_budget 字节预算判定
}
// ...
let truncated = next_offset.is_some() || entries_truncated;
```

语义：

- `next_offset.is_some()`：在**已收集**的条目里还有下一页。
- `entries_truncated`：收集阶段已满 50,000 条且还存在未收集条目（即第 50,001 条），`total_lines` 只是已收集数；即使翻到最后一页，也不能保证看到目录的全部条目。
- 恰好 50,000 条时 `entries_truncated` 为 `false`（listing 完整）。
- 两者可以同时为真。

### 4.4 模型看到的文本

文件分页（常见）：

```text
Read /repo/src/big.rs lines 1-824 of 5000 total lines
file_version: abc123 (pass this as read_version when editing this file)
truncated: true (4176 more lines available with offset=825)
---
1: ...
...
next_offset: 825 (call read again with this offset to continue)
```

文件单行被截：

```text
Read /repo/dist/app.min.js lines 1-1 of 1 total lines
file_version: abc123 (pass this as read_version when editing this file)
truncated: true (line(s) longer than 2000 characters were clipped; the clipped tail cannot be recovered with offset)
---
1: !function(){...…[truncated]
```

文件同时分页 + 单行截断：

```text
truncated: true (4176 more lines available with offset=825; line(s) longer than 2000 characters were clipped; the clipped tail cannot be recovered with offset)
```

目录分页：

```text
Listed 50000 entries in directory /repo/node_modules
truncated: true (1200 more entries available with offset=48801)
---
[dir] .bin
...
next_offset: 48801 (call read again with this offset to continue)
```

目录收集触顶：

```text
Listed 50000 entries in directory /repo/huge
truncated: true (listing stopped at the 50000-entry collection cap; more entries may exist)
---
[dir] ...
```

目录同时触顶 + 分页：

```text
truncated: true (1200 more entries available with offset=48801; listing stopped at the 50000-entry collection cap; more entries may exist)
```

规则：

- 保留现有 `Read ... lines X-Y of Z total lines` / `Listed ... entries` 头部。
- 保留现有 `next_offset: N (call read again with this offset to continue)` 尾部；与截断行里的 offset 重复是刻意的——模型可能只扫头部或只扫尾部。
- `truncated: true` 前缀保持不变，旧解析器与测试仍能识别。
- 多个原因用 `; ` 连接，顺序固定：先分页，再单行/收集上限。
- 目录收集触顶的数字用 `total_lines`（触顶时它等于实际收集上限；生产是 50000，测试注入更小上限时也不会写错数字），而不是在渲染层硬编码常量。
- 不出现 `…[truncated]` 之外的合成标记；单行行尾的 `…[truncated]` 保留（模型能直接看到是哪一行被切）。

### 4.5 落库 JSON

新增两个字段（纯加法，旧记录缺失时按 `false`）：

```json
{
  "tool": "read",
  "path": "/repo/src/big.rs",
  "kind": "file",
  "content": "1: ...\n",
  "start_line": 1,
  "end_line": 824,
  "next_offset": 825,
  "truncated": true,
  "line_truncated": false,
  "entries_truncated": false,
  "total_lines": 5000,
  "file_version": "abc123",
  "image": null
}
```

- `truncated` 仍是聚合标志：`next_offset.is_some() || line_truncated || entries_truncated`。
- `line_truncated` / `entries_truncated` 不参与 UI 摘要；`ToolSummary::absorb_result_json` 不需要改动。
- 旧结果 JSON 没有新字段：`serde_json::Value::get(...).and_then(as_bool).unwrap_or(false)` 的消费者自然按 `false` 处理；当前仓库没有对 `ToolExecutionResult::Read` 的反序列化结构体，因此不需要版本迁移。
- 1 MiB 存储上限逻辑不变：如果未来其他工具结果超限，`truncate_value` 仍会截 `content` 并置 `truncated: true`，新字段会随对象保留。

### 4.6 回放与实时的一致性

- **实时**：`agent_loop` 调 `format_for_model`，模型看到 §4.4 的文本。
- **落库**：`to_result_json` 写 §4.5 的 JSON。
- **重进会话**：`context::replay_pair` 直接把落库 JSON（剥掉 UI diff 字段）作为 `ToolResult` 内容；模型看到的是 JSON，不是 §4.4 文本。这是现状，本任务不改变。
- **新不变量**：`read` 的 50 KiB 预算按落库 JSON 转义后的内容计，`format_for_model` 文本与 `to_result_json` JSON 都小于 64 KiB 回放上限；因此回放层不会对 `read` 结果再截一次（引号/反斜杠密集的文本也不会翻倍越界）。旧会话里 256 KiB 的 read 结果仍可能被回放层截断，这是预期兼容行为。

## 5. 关键代码改动

### 5.1 `src/chat/tools/types.rs`

在 `ToolExecutionResult::Read` 的 `truncated` 之后新增：

```rust
/// 是否因输出上限 / 行数上限 / 单行截断而截断（聚合标志；UI 摘要与旧消费者只读它）。
truncated: bool,
/// 是否有单行超过 `MAX_READ_LINE_CHARS` 被截断（仅文件；offset 无法恢复该行尾部）。
line_truncated: bool,
/// 目录收集是否触达 `MAX_READ_DIR_ENTRIES`（仅目录；此时 `total_lines` 不是真实总数）。
entries_truncated: bool,
```

### 5.2 `src/chat/tools/mod.rs`

```rust
/// `read` 单次返回内容字节上限（50 KiB，对齐 opencode）：按落库 JSON 转义后的
/// 字节计（普通文本近似 UTF-8 字节），超出按行截断并给出 `next_offset`。
pub const MAX_READ_BYTES: usize = 50 * 1024;

/// `read` 单行输出字符上限（2000，对齐 opencode）：超长行截断并显式标记。
pub const MAX_READ_LINE_CHARS: usize = 2000;
```

同时更新模块顶部契约第 11 行，补充 `read` 输出 50 KiB 上限（输入 16 MiB / 目录 50 000 条保持不变）。

### 5.3 `src/chat/tools/read.rs`

- `read_image`（约 136-153）：`line_truncated: false, entries_truncated: false`。
- 空文件（约 202-213）：同上。
- 文件循环（约 219-253）：把局部 `line_truncated` 写入结果；`entries_truncated: false`；聚合 `truncated` 改为 `next_offset.is_some() || line_truncated`。
- `read_directory` 空目录（约 326-340）：`line_truncated: false, entries_truncated`。
- `read_directory` 正常返回（约 362-374）：`line_truncated: false, entries_truncated`；聚合 `truncated` 改为 `next_offset.is_some() || entries_truncated`。
- 新增 `json_escaped_len` 辅助（`serde_json::to_string(text)` 长度减 2）；文件与目录循环的 `cost` 都改为 `json_escaped_len(&rendered) + 2`。
- 新增 `content_budget(target)`：`MAX_READ_BYTES` 扣除 JSON 转义后的路径与 512 字节固定元数据余量，保证整份落库 JSON 低于 64 KiB 回放上限。
- `render_line` 不变，只受常量变化影响。

### 5.4 `src/chat/tools/output.rs`

`format_read` 当前有 9 个参数且带 `#[allow(clippy::too_many_arguments)]`。新增两个布尔后改为一个小型只读结构，避免 11 参数：

```rust
struct ReadReport<'a> {
    path: &'a std::path::Path,
    kind: ReadKind,
    content: &'a str,
    start_line: usize,
    end_line: usize,
    next_offset: Option<usize>,
    total_lines: usize,
    version: Option<&'a str>,
    line_truncated: bool,
    entries_truncated: bool,
}

fn format_read(report: ReadReport<'_>) -> String {
    let ReadReport {
        path, kind, content, start_line, end_line, next_offset,
        total_lines, version, line_truncated, entries_truncated,
    } = report;

    let mut text = String::new();
    match kind {
        ReadKind::Image => { /* 原样 */ }
        ReadKind::Directory => {
            text.push_str(&format!(
                "Listed {} entries in directory {}\n",
                total_lines,
                path.display()
            ));
            let mut cuts: Vec<String> = Vec::new();
            if let Some(next) = next_offset {
                let remaining = total_lines.saturating_sub(end_line);
                let unit = if remaining == 1 { "entry" } else { "entries" };
                cuts.push(format!("{remaining} more {unit} available with offset={next}"));
            }
            if entries_truncated {
                cuts.push(format!(
                    "listing stopped at the {total_lines}-entry collection cap; more entries may exist"
                ));
            }
            if !cuts.is_empty() {
                text.push_str(&format!("truncated: true ({})\n", cuts.join("; ")));
            }
        }
        ReadKind::File => {
            if start_line == 0 { /* 原样：empty file */ }
            text.push_str(&format!(
                "Read {} lines {}-{} of {} total lines\n",
                path.display(),
                start_line,
                end_line,
                total_lines
            ));
            if let Some(version) = version { /* 原样 */ }
            let mut cuts: Vec<String> = Vec::new();
            if let Some(next) = next_offset {
                let remaining = total_lines.saturating_sub(end_line);
                let unit = if remaining == 1 { "line" } else { "lines" };
                cuts.push(format!("{remaining} more {unit} available with offset={next}"));
            }
            if line_truncated {
                cuts.push(format!(
                    "line(s) longer than {MAX_READ_LINE_CHARS} characters were clipped; \
                     the clipped tail cannot be recovered with offset"
                ));
            }
            if !cuts.is_empty() {
                text.push_str(&format!("truncated: true ({})\n", cuts.join("; ")));
            }
        }
    }
    text.push_str("---\n");
    text.push_str(content);
    if !content.ends_with('\n') && !content.is_empty() {
        text.push('\n');
    }
    if let Some(next) = next_offset {
        text.push_str(&format!(
            "next_offset: {next} (call read again with this offset to continue)\n"
        ));
    }
    text
}
```

要点：

- `format_read` 不再接收聚合 `truncated`；截断提示完全由 `next_offset` / `line_truncated` / `entries_truncated` 派生，避免调用方传错聚合值。
- `truncated: true` 前缀保留，旧测试与模型习惯不变。
- 头部 `Read ... of Z total lines` / `Listed ... entries` 保留。
- `next_offset` 尾部保留。

`to_result_json` 的 `Read` 分支新增：

```rust
"truncated": truncated,
"line_truncated": line_truncated,
"entries_truncated": entries_truncated,
```

### 5.5 `src/chat/tools/schema.rs`

`read` 的工具描述在“Long output is truncated explicitly and reports `next_offset`”后补一句：

```text
Output is capped at 2000 lines or 50 KiB per call, whichever comes first; individual
lines are clipped at 2000 characters, and directory listings share the same 50 KiB cap.
Truncation is explicit and reports next_offset when more lines are available.
```

只改模型描述，不改 schema 字段；`limit` 的文案仍由 `MAX_READ_LINES` 生成。

### 5.6 `src/skills/assets/lx-guide/references/tools.md`

更新 `read` 行为段（约 53-54 行）：

```text
- A single text file larger than 16 MiB is rejected with `too_large`. Output is capped
  at 2000 lines or 50 KiB (whichever comes first), and each line is clipped at 2000
  characters; truncation is reported explicitly and the remaining lines can be read
  with a later `offset`.
- Directory paths return a sorted listing with `[dir]` and `[file]` prefixes. At most
  50,000 entries are collected; the same 50 KiB output cap applies. When the collection
  cap is hit the result says the listing may be incomplete; otherwise `next_offset`
  continues the listing.
```

### 5.7 `web_search` 与 `exec_command` / `write_stdin` 的统一 50 KiB 截断补充

- **`src/chat/tools/web_search/mod.rs`**：
  - 保留单查询 `MAX_QUERY_OUTPUT_CHARS = 12_000` 字符上限；
  - 新增整次调用聚合上限：`MAX_WEB_SEARCH_BYTES = super::MAX_READ_BYTES`（50 KiB，按落库 JSON 转义后字节计，扣除查询参数与 512 字节元数据）、`MAX_WEB_SEARCH_LINES = 2000`、`MAX_WEB_SEARCH_LINE_CHARS = 2000`；
  - 头部保留（head），单行超 2000 字符追加 `…[truncated]`，聚合行数/字节超限保留最多 1999 行正文并追加独立行 `\n…[truncated]`（总行数 `<= 2000`），任一触发均置 `truncated = true`。
- **`src/chat/process/{output.rs,mod.rs,manager/{mod.rs,handle.rs},rtk/{mod.rs,builtin.rs}}` & `src/chat/agent_loop/executor.rs`**：
  - `MAX_MODEL_OUTPUT_BYTES`：`64 * 1024` → `50 * 1024`；`MAX_OUTPUT_TOKENS`：`16_000` → `12_500`；
  - 新增 `MAX_MODEL_OUTPUT_LINES = 2000`、`MAX_MODEL_OUTPUT_LINE_CHARS = 2000`；
  - `OutputBuffer::sanitize_text` 在 `flush == false`（运行中进程）时按 2000 行与安全转义预算在行边界停止消费字节，保证 `handle.cursor` 不越过未返回的行；
  - `ProcessResult::enforce_output_bounds` 在 `rtk::filter_output_flagged` 之后统一执行唯一一次，限制持久化 `command` 长度并对 `output` 施加单行 2000 字符、总计 2000 行与 50 KiB JSON 转义后预算（终态保留首尾 head + tail 并写 `…[N lines omitted]`，运行中保留头部并写 `…[truncated]`）；
  - `rtk::builtin::filter_output_flagged` 在内置过滤器触发行数/字节/条目省略时返回 `true`，由 `executor.rs` 汇总进 `result.truncated |= omitted`。
  - 将 `ProcessHandle` 拆分为 `src/chat/process/manager/handle.rs`，保持生产单文件行数 `< 1000`。

### 5.8 TUI 消息列表截断展示统一与单体 `read` 单行化

- **`src/ui/text/agent.rs` & `src/ui/text/mcp.rs`**：
  - 定义 `pub const INLINE_TRUNCATED: &str = "[truncated]"`（不带省略号）；移除无引用的 `text::mcp::RESULT_TRUNCATED`。
- **`src/ui/main_content/lx/history/tool/tree.rs`**：
  - `Section` 新增 `omitted_lines: usize`（`.with_omitted(usize)`）与 `truncated: bool`（`.with_truncated(bool)`）；
  - `layout.hidden` 仅统计被 `Preview` 折叠隐藏的物理行数，而底部提示行按 `total_omitted = section_hidden + section.omitted_lines` 统一生成唯一一行 `… N more lines [truncated]`；当 `total_omitted == 0` 时不生成独立 `[truncated]` 行。
- **`src/ui/main_content/lx/history/tool/{command.rs,web_search.rs,mcp.rs,search.rs,file_summary.rs}` & `src/ui/main_content/lx/history/tool.rs`**：
  - 移除 `web_search`、`search`、`mcp` 标题右侧的截断状态词，以及 `command` / `read` 的 `output limited` 事实行；
  - 剥离工具输出正文与命中行末尾内联的 `…[truncated]` / `[...truncated]` / `[truncated]` 文字标签（保留被截断单行末尾的 `'…'`），确保 `[truncated]` 只出现在 `… N more lines [truncated]` 提示行上；
  - `file_summary.rs` 中 `Kind::Read` 遵循 `lx-message-block-display.md` §5.1：标题下方只显示单行路径或单行错误/状态词（超宽尾部 `…` 省略，不软换行、不登记可展开点击面）。

### 5.9 明确不改的文件

- `src/app/state/lx/chat/steps.rs`：`ToolSummary` 不新增字段；`truncated` 聚合继续工作。
- `src/app/update/chat/view.rs`：投影不改。
- `src/chat/context/mod.rs`：回放上限 64 KiB 不改。
- `src/db/**`：无迁移、无 schema 变更。

## 6. 兼容性与边界

| 场景 | 行为 |
| --- | --- |
| 旧会话的 read 结果（≤256 KiB） | UI 照常渲染；重新进入上下文时仍受 64 KiB 回放上限；新字段缺失按 `false`。 |
| 实时结算 | `format_for_model` 使用新提示；`to_result_json` 写新字段。 |
| `/resume` | `absorb_result_json` 只读 `truncated` / `total_lines` / `kind` / 区间；不依赖新字段。 |
| 子 agent | 与主 agent 共用 `chat::tools`，自动获得同样的截断。 |
| 图片 | 不参与字节/行截断，两个新字段为 `false`。 |
| 空文件 | 同上。 |
| `limit` 参数 | 仍是 `MAX_READ_LINES` 的硬上限；即使 `limit=2000`，50 KiB 也可能提前结束，`next_offset` 给出继续位置。 |
| UTF-8 | 50 KiB 按字节计；截断只发生在整行边界，不会切断 UTF-8。单行截断按字符计。 |
| 单行超长 + 分页 | 两个提示同时出现；模型知道“后面还有行”，也知道“被切的那行尾部拿不回来”。 |
| 目录 50,000 条触顶 + 分页 | 两个提示同时出现；模型知道“先翻页，但翻完也不是全部”。 |

## 7. 测试计划

### 7.1 新增/修改的单元测试

`src/chat/tools/read/tests.rs`：

| 测试 | 断言 |
| --- | --- |
| `read_limits_match_opencode` | 常量锁定 `50 * 1024` / `2000` / `2000`。 |
| `byte_limit_truncates_with_next_offset` | 造 200 行 × 3000 字符；`content.len() <= 50 * 1024`、`next_offset.is_some()`、`truncated == true`、`line_truncated == true`。 |
| `overlong_line_is_truncated_and_flagged` | 单行 `MAX_READ_LINE_CHARS + 50` 字符；`line_truncated == true`、`next_offset == None`、内容含 `…[truncated]`。 |
| `line_and_byte_truncation_can_coexist` | 多行超长 + 总体超 50 KiB；两个标志都为 `true`。 |
| `byte_limit_counts_utf8_bytes` | 1000 个中文字符 × 40 行；按 UTF-8 字节提前截断，`line_truncated == false`。 |
| `directory_byte_limit_paginates` | 长文件名目录；`content.len() <= 50 * 1024`、`next_offset.is_some()`、`entries_truncated == false`。 |
| `directory_read_caps_entries`（扩展现有） | 注入 `dir_entries: 3`；`entries_truncated == true`、`truncated == true`、`next_offset == None`、模型文本含 `3-entry collection cap`。 |
| `read_output_stays_below_replay_budget` | 读大文件后 `format_for_model(&result).len() < 64 * 1024`。 |
| `escaped_content_stays_below_replay_budget` | 全引号文件：落库 JSON 的 `content` 按转义后大小计，`to_result_json(...).len() < 64 * 1024`。 |

`src/chat/tools/output/tests.rs`：

| 测试 | 断言 |
| --- | --- |
| `read_file_paged_reports_remaining_lines_and_offset` | 文本含 `4176 more lines available with offset=825` 与 `next_offset: 825`。 |
| `read_file_line_clamp_reports_unrecoverable_tail` | 文本含 `clipped; the clipped tail cannot be recovered with offset`。 |
| `read_file_reports_paging_and_clamp_together` | 文本同时含 `more lines` 与 `clipped`。 |
| `read_directory_paged_reports_remaining_entries` | 文本含 `more entries available with offset=`。 |
| `read_directory_collection_cap_reports_incomplete_listing` | 文本含 `collection cap; more entries may exist`。 |
| `read_directory_reports_paging_and_collection_cap_together` | 文本同时含剩余条目与 collection cap。 |
| `read_json_carries_truncation_flags` | JSON 的 `truncated` / `line_truncated` / `entries_truncated` 正确。 |
| `oversized_read_json_preserves_truncation_flags` | 直接构造 `MAX_TOOL_RESULT_BYTES + 1024` 字节 `content` 的 `Read`，`to_result_json` ≤ 1 MiB、`truncated == true`、新字段保留。 |

### 7.2 需要同步修改的既有测试

| 文件 | 原因 |
| --- | --- |
| `src/chat/tools/tests.rs::result_json_is_normalized_and_flags_truncation` | 原来用 4000 行 × 4000 字符的 read 触发 1 MiB 存储截断；read 现在 ≤50 KiB，无法再触发。改为直接构造超大 `Read`。 |
| `src/chat/agent_loop/tests/context.rs::current_turn_truncates_oversized_tool_result` | 原来依赖 read 返回 >64 KiB；现在 read ≤50 KiB。改为 `current_turn_read_result_stays_inside_replay_budget`（断言 read 结果不被二次截断），回放上限仍由 `push_tool_result_uses_the_same_replay_limit` 覆盖。 |
| `src/chat/tools/output/tests.rs` 中所有 `ToolExecutionResult::Read { .. }` 构造 | 补 `line_truncated` / `entries_truncated`。 |
| `src/chat/agent/exec/tests/processor_tests.rs` | 同上。 |
| `src/app/update/chat/view/tests.rs` | 同上。 |

`src/chat/tools/test_support.rs::expect_read` 与 `src/chat/tools/read_version/tests.rs` 使用 `..` 解构，无需补字段；前者新增 `expect_read_flags` 辅助读取新字段。

机械替换入口：

```bash
rg -n "ToolExecutionResult::Read \\{" src
```

除 `src/chat/tools/read.rs` 的真实返回点外，其余构造统一补：

```rust
line_truncated: false,
entries_truncated: false,
```

### 7.3 验证命令

```bash
cargo fmt
cargo clippy --all-targets -- -D warnings
cargo test chat::tools::read
cargo test chat::tools::output
cargo test chat::agent_loop::tests::context
cargo test app::update::chat::view
cargo test
```

项目规范要求全量 `cargo test`；定向测试只用于快速回归，不替代全量。

## 8. 风险与非目标

### 8.1 风险

1. **无 `more lines` 行时不单独显示 `[truncated]`**：当工具输出很短（未触发折叠且无窗口外省略行）或已完全展开且无窗口外省略行时，卡片内不再显示独立的 `[truncated]` 行（单行被截断处仍保留末尾 `'…'`）。
2. **压缩/单行文件读不全**：2000 字符的行截断无法通过 `offset` 恢复尾部，这是 opencode 同样的取舍；模型提示已明确说明“不可恢复”。
3. **大文件翻页次数增加**：模型可能更多次调用 `read`；这是控制单次上下文占用换来的成本。
4. **旧会话兼容性**：旧结果 JSON 缺少新字段时按 `false` 处理；历史结果正文中的内联 `…[truncated]` / `[...truncated]` 在视图/渲染层同样会被剥离并统一至 `… N more lines [truncated]`。
5. **与 `dev` 分支（`feat/lx-tool-call-grouping`）合并对齐**：`dev` 已合入连续工具合并特性，合入 `dev` 时需在 `tree.rs`、`file_summary.rs`、`search.rs`、`web_search.rs`、`mcp.rs`、`group.rs` 中保持分组树连接符与 `… N more lines [truncated]` 口径一致。

### 8.2 非目标

- 不修改 `search` / MCP / 子 agent 汇报的执行层截断阈值。
- 不引入临时文件落盘（opencode 的 `tool-output` 方案）。
- 不增加 DB 迁移或新表。
- 不增加用户可见配置项；上限是常量。
- 不修改 `MAX_TOOL_RESULT_REPLAY_BYTES`（64 KiB）或 `MAX_TOOL_RESULT_BYTES`（1 MiB）。

## 9. 验收标准

1. 读 5000 行、每行 60 字符的文件：单次 `read` 文本 ≤ 约 50.5 KiB；`next_offset` 可用；模型文本含 `N more lines available with offset=X`。
2. 读单行 2500 字符的文件：`line_truncated == true`；模型文本含 `clipped; the clipped tail cannot be recovered with offset`；JSON 同名字段为 `true`。
3. 同时触发字节上限与单行截断：两个提示同时出现，`truncated == true`。
4. 目录 1800 条、平均路径 40 字符：单次输出 ≤ 50 KiB；`next_offset` 可用；无 `entries_truncated`。
5. 注入小 `dir_entries` 的目录测试：`entries_truncated == true`，模型文本含 `collection cap; more entries may exist`。
6. `web_search` 与 `exec_command` / `write_stdin` 输出受 50 KiB JSON 转义预算 + 2000 行 + 单行 2000 字符约束，`to_result_json` 恒 ≤ 64 KiB。
7. TUI 工具卡片标题栏与正文不出现 `[truncated]` 或 `output limited`；仅在 `… N more lines` 提示行末尾显示 `… N more lines [truncated]`。
8. 单体 `read` 卡片标题下方只显示单行路径或单行错误/状态词（超宽尾部 `…` 省略，不软换行、不登记点击面）。
9. 全量 `cargo fmt --check`、`cargo clippy --all-targets -- -D warnings`、`cargo test` 通过。
