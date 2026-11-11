# lx `read` 工具长内容截断：实施任务

> 状态：**已实施；验证通过，待合并**。
> 实施记录：下方 `[ ]` 保留为实施前计划；实际完成情况以上方状态与 §6.2 验证结果为准。
> 对应设计：`docs/feature/lx-read-output-truncation/design.md`。
> 范围：`read`、`web_search`、`exec_command` / `write_stdin` 返回给模型/落库的结果体；**不改消息列表展示**。
> 日期：2026-11-11。

## 0. 执行前检查

- [ ] 用户已审核并确认 `design.md` 的 §1、§4、§10。
- [ ] 当前分支状态干净（`git status --short`），没有与本任务无关的未提交改动。
- [ ] 未获得明确授权前，不执行 `git add` / `git commit` / 切分支 / rebase。
- [ ] 重新确认以下文件没有在审核期间被其他任务改动：
  - `src/chat/tools/mod.rs`
  - `src/chat/tools/types.rs`
  - `src/chat/tools/read.rs`
  - `src/chat/tools/output.rs`
  - `src/chat/tools/schema.rs`
  - `src/skills/assets/lx-guide/references/tools.md`

## 1. 常量与结果类型

### 1.1 `src/chat/tools/mod.rs`

- [ ] `MAX_READ_BYTES`：`256 * 1024` → `50 * 1024`。
- [ ] `MAX_READ_LINE_CHARS`：`4000` → `2000`。
- [ ] 更新两处 doc comment，写明“对齐 opencode”。
- [ ] 更新模块顶部契约（约第 11 行）：补充 `read` 输出 50 KiB 上限；输入 16 MiB / 目录 50 000 条不变。
- [ ] `MAX_READ_LINES`、`MAX_READ_FILE_INPUT_BYTES`、`MAX_READ_DIR_ENTRIES` 保持不变。

目标代码：

```rust
/// `read` 单次结果体的内容预算（50 KiB，对齐 opencode）：按落库 JSON 转义后的
/// 字节计（普通文本近似 UTF-8 字节），并为路径/元数据预留空间；超出按行截断并给出
/// `next_offset`。
pub const MAX_READ_BYTES: usize = 50 * 1024;

/// `read` 单行输出字符上限（2000，对齐 opencode）：超长行截断并显式标记。
pub const MAX_READ_LINE_CHARS: usize = 2000;
```

### 1.2 `src/chat/tools/types.rs`

- [ ] 在 `ToolExecutionResult::Read` 的 `truncated` 后新增两个字段：

```rust
/// 是否因输出上限 / 行数上限 / 单行截断而截断（聚合标志；UI 摘要与旧消费者只读它）。
truncated: bool,
/// 是否有单行超过 `MAX_READ_LINE_CHARS` 被截断（仅文件；offset 无法恢复该行尾部）。
line_truncated: bool,
/// 目录收集是否触达 `MAX_READ_DIR_ENTRIES`（仅目录；此时 `total_lines` 不是真实总数）。
entries_truncated: bool,
```

- [ ] 确认 `ToolExecutionResult` 仍派生 `Debug, Clone, PartialEq, Eq`，bool 无额外 trait 要求。

## 2. `read` 执行层

### 2.1 `src/chat/tools/read.rs`

逐构造点补字段（行号为设计时定位）：

- [ ] `read_image` 返回（约 136-153）：`line_truncated: false, entries_truncated: false`。
- [ ] 空文件返回（约 202-213）：同上。
- [ ] 文件循环返回（约 242-253）：
  - 写入局部 `line_truncated`；
  - `entries_truncated: false`；
  - 聚合 `truncated` = `next_offset.is_some() || line_truncated`。
- [ ] 空目录返回（约 326-340）：`line_truncated: false, entries_truncated`（使用局部变量）。
- [ ] 目录正常返回（约 362-374）：
  - `line_truncated: false, entries_truncated`；
  - 聚合 `truncated` = `next_offset.is_some() || entries_truncated`。
- [ ] `render_line` 不改逻辑，只受 `MAX_READ_LINE_CHARS = 2000` 影响。
- [ ] 确认 `entries_truncated` 只在收集阶段已满 `caps.dir_entries` 且还存在第 N+1 条时为 `true`，不因 `next_offset` 置位；恰好 N 条时为 `false`。

关键代码（文件循环）：

```rust
let mut line_truncated = false;
let budget = content_budget(target);
for index in start_index..total_lines {
    if emitted >= limit {
        break;
    }
    let (rendered, truncated) = render_line(index + 1, &source.lines[index]);
    // 按落库 JSON 转义后的字节计，避免引号/反斜杠密集的文本翻倍超过 64 KiB。
    let cost = json_escaped_len(&rendered) + 2;
    if emitted > 0 && used_bytes + cost > budget {
        break;
    }
    line_truncated |= truncated;
    content.push_str(&rendered);
    content.push('\n');
    used_bytes += cost;
    emitted += 1;
}
let end_line = start_index + emitted;
let next_offset = (end_line < total_lines).then_some(end_line + 1);
Ok(ToolExecutionResult::Read {
    path: target.to_path_buf(),
    kind: ReadKind::File,
    content,
    start_line: start_index + 1,
    end_line,
    next_offset,
    truncated: next_offset.is_some() || line_truncated,
    line_truncated,
    entries_truncated: false,
    total_lines,
    version,
    image: None,
})
```

关键代码（目录返回）：

```rust
let next_offset = (end_line < total_lines).then_some(end_line + 1);
Ok(ToolExecutionResult::Read {
    path: target.to_path_buf(),
    kind: ReadKind::Directory,
    content,
    start_line: start_index + 1,
    end_line,
    next_offset,
    truncated: next_offset.is_some() || entries_truncated,
    line_truncated: false,
    entries_truncated,
    total_lines,
    version: None,
    image: None,
})
```

- [ ] 新增 `json_escaped_len(text)` 辅助：`serde_json::to_string(text)` 长度减 2；文件与目录循环都用 `json_escaped_len(&rendered) + 2` 计费。
- [ ] 新增 `content_budget(target)`：`MAX_READ_BYTES` 扣除 JSON 转义后的路径与 512 字节固定元数据余量，保证整份落库 JSON 低于 64 KiB。

## 3. 模型文本与落库 JSON

### 3.1 `src/chat/tools/output.rs`：`format_read`

- [ ] 新增私有 `ReadReport<'a>` 结构（见 design §5.4），替换 `format_read` 的 9 个位置参数。
- [ ] 删除 `format_read` 的聚合 `truncated` 参数；截断提示由 `next_offset` / `line_truncated` / `entries_truncated` 派生。
- [ ] 删除 `#[allow(clippy::too_many_arguments)]`（参数不再过多）。
- [ ] 文件分支：
  - `next_offset` Some → `N more lines available with offset=X`（单数用 `line`）。
  - `line_truncated` → `line(s) longer than 2000 characters were clipped; the clipped tail cannot be recovered with offset`。
  - 两者都有 → 用 `; ` 连接，顺序固定。
- [ ] 目录分支：
  - `next_offset` Some → `N more entries available with offset=X`（单数用 `entry`）。
  - `entries_truncated` → `listing stopped at the {total_lines}-entry collection cap; more entries may exist`（生产是 50000）。
  - 两者都有 → 用 `; ` 连接。
- [ ] 保留 `truncated: true (...)` 前缀、头部 `Read ... lines X-Y of Z total lines` / `Listed ... entries`、尾部 `next_offset: N (...)`。
- [ ] 图片与空文件的早退逻辑不变。

目标文本见 design §4.4。实现时用一个本地 `cuts: Vec<String>` + `cuts.join("; ")`，不要在多个分支里重复拼 `truncated:`。

### 3.2 `src/chat/tools/output.rs`：`to_result_json`

- [ ] `Read` 分支新增：

```rust
"truncated": truncated,
"line_truncated": line_truncated,
"entries_truncated": entries_truncated,
```

- [ ] 确认 1 MiB 存储上限的 `truncate_value`（约 800-856 行）不会删除新字段：它只改 `content` / `output` / `text` / `matches` 并插入 `truncated: true`，其余对象字段原样保留。
- [ ] 不需要给 `ToolSummary` 增加字段；`absorb_result_json` 继续只读 `truncated`。

## 4. 模型可见文档

### 4.1 `src/chat/tools/schema.rs`

- [ ] `read` 工具描述在“Long output is truncated explicitly...”后补充：

```text
Output is capped at 2000 lines or 50 KiB per call, whichever comes first; individual
lines are clipped at 2000 characters. Truncation is explicit and reports next_offset
when more lines are available.
```

- [ ] 不改 `read_parameters()` 的字段与 `limit` 文案（仍由 `MAX_READ_LINES` 生成）。

### 4.2 `src/skills/assets/lx-guide/references/tools.md`

- [ ] 更新 `read` 行为段（约 53-54 行）：
  - 2000 行 / 50 KiB / 2000 字符；
  - 目录共用 50 KiB；
  - 50,000 条收集上限触顶时明确说 listing 可能不完整；
  - `next_offset` 继续分页。
- [ ] 不在历史设计文档 `docs/feature/lx-agent-file-tools/design.md` 中追改旧数值；本文件是新的权威来源。

## 5. 测试

### 5.1 先机械修复所有构造点

只给**构造** `ToolExecutionResult::Read { ... }` 的位置补字段；带 `..` 的解构模式不需要改。执行：

```bash
rg -n "ToolExecutionResult::Read \\{" src
```

**必须补字段的构造点**：

- `src/chat/tools/read.rs`：5 个真实返回点（约 136、202、242、326、362 行）。
- `src/chat/tools/output/tests.rs`：约 14、36、50、244、440、544、560、576、592 行。
- `src/chat/agent/exec/tests/processor_tests.rs`：约 35 行。
- `src/app/update/chat/view/tests.rs`：约 337、353 行。

除真实读工具路径外，测试统一填 `false`；若要模拟截断，按测试意图填 `true`。

**需要改的显式解构模式**（没有 `..`）：

- `src/chat/tools/output.rs` 的 `format_for_model`（约 21 行）：加入 `line_truncated` / `entries_truncated`，或改成 `..` 后按需取字段。
- `src/chat/tools/output.rs` 的 `to_result_json`（约 689 行）：必须显式取两个新字段写 JSON。
- `src/chat/tools/output.rs` 的 `to_summary`（约 556 行）：已含 `..`，无需改动；不要给 `ToolSummary` 加字段。

**不需要改的 `..` 解构点**（确认即可）：

- `src/chat/tools/test_support.rs::expect_read`
- `src/chat/tools/read/tests.rs` 的目录/图片断言
- `src/chat/tools/read_version/tests.rs`
- `src/chat/agent/exec/events.rs`
- `src/chat/agent_loop/mod.rs`
- `src/chat/agent_loop/executor/tests.rs`

### 5.2 `src/chat/tools/read/tests.rs`

- [ ] 新增 `read_limits_match_opencode`：锁定 `50 * 1024` / `2000` / `2000`。
- [ ] 扩展 `byte_limit_truncates_with_next_offset`：
  - 断言 `content.len() <= 50 * 1024`；
  - 断言 `line_truncated`（3000 字符行 > 2000）。
- [ ] 扩展 `overlong_line_is_truncated_and_flagged`：
  - 断言 `line_truncated == true`；
  - 断言 `next_offset == None`（单行文件）；
  - 断言内容含 `…[truncated]`。
- [ ] 新增 `directory_byte_limit_paginates`：长文件名目录，断言 ≤50 KiB、`next_offset` Some、`entries_truncated == false`。
- [ ] 扩展 `directory_read_caps_entries`：断言 `entries_truncated == true`、`truncated == true`、`next_offset == None`。
- [ ] 新增 `read_output_stays_below_replay_budget`：读大文件后 `format_for_model(&result).len() < 64 * 1024`。
- [ ] 新增 `escaped_content_stays_below_replay_budget`：全引号文件，断言 `to_result_json(&result).len() < 64 * 1024`（验证预算按 JSON 转义后大小计）。
- [ ] 新增 UTF-8 字节上限测试：中文/emoji 行按字节提前截断，不产生非法 UTF-8。

### 5.3 `src/chat/tools/output/tests.rs`

- [ ] 新增 `read_file_paged_reports_remaining_lines_and_offset`。
- [ ] 新增 `read_file_line_clamp_reports_unrecoverable_tail`。
- [ ] 新增 `read_file_reports_paging_and_clamp_together`。
- [ ] 新增 `read_directory_paged_reports_remaining_entries`。
- [ ] 新增 `read_directory_collection_cap_reports_incomplete_listing`。
- [ ] 新增 `read_directory_reports_paging_and_collection_cap_together`。
- [ ] 新增 `read_json_carries_truncation_flags`。
- [ ] 新增 `oversized_read_json_preserves_truncation_flags`：直接构造 `MAX_TOOL_RESULT_BYTES + 1024` 字节 `content` 的 `Read`，断言 `to_result_json` ≤ 1 MiB、`truncated == true`、新字段保留。
- [ ] 更新所有现有 `Read` 构造。

### 5.4 `src/chat/tools/tests.rs`

- [ ] 重写 `result_json_is_normalized_and_flags_truncation`：
  - read 现在 ≤50 KiB，无法触发 1 MiB 存储截断；
  - 改为直接构造超大 `ToolExecutionResult::Read`（或改用其他工具）测试存储上限；
  - 保留 read JSON 的规范化断言（`tool` / `kind` / 新字段）。

### 5.5 `src/chat/agent_loop/tests/context.rs`

- [ ] 用 `current_turn_read_result_stays_inside_replay_budget` 替换 `current_turn_truncates_oversized_tool_result`：
  - 读一个大文件；
  - 断言注入下一次采样的 `ToolResult.content.len() < MAX_TOOL_RESULT_REPLAY_BYTES`；
  - 断言**不以** `[truncated]` 结尾（read 自身已截断，不应再被回放层截一次）。
- [ ] 保留 `push_tool_result_uses_the_same_replay_limit`，回放上限覆盖不丢失。
- [ ] 如果仍需要端到端“超 64 KiB 工具结果被回放截断”的覆盖，新增一个返回合成大结果的测试 executor（不要再用 read 造数据）。

## 6. 验证

### 6.1 定向

```bash
cargo fmt --check
cargo test chat::tools::read
cargo test chat::tools::output
cargo test chat::tools::tests::result_json_is_normalized_and_flags_truncation
cargo test chat::agent_loop::tests::context
cargo test app::update::chat::view
```

### 6.2 全量

```bash
cargo fmt
cargo clippy --all-targets -- -D warnings
cargo test
```

- [ ] 全量通过，无 ignored 新增。
- [ ] 若 `cargo clippy` 报 `too_many_arguments`，确认 `ReadReport` 已替换旧签名，而不是重新加 `allow`。

### 6.3 人工验收（用户执行）

- [ ] `cargo run`，在 lx 会话里让模型读一个 >2000 行或 >50 KiB 的文件。
- [ ] 确认消息列表里的 read 卡片外观与改动前一致：没有新增行、没有新按钮、没有新的展开区。
- [ ] 确认模型下一轮能看到 `truncated: true (N more lines available with offset=X)`，并能用该 offset 继续读。
- [ ] 确认单行超长文件出现 `clipped; ... cannot be recovered with offset`，模型不会反复用 offset 重试。
- [ ] 确认 `/resume` 后旧会话与新会话的 read 卡片一致。

## 7. 禁止修改清单

- `src/ui/main_content/lx/**`（卡片、折叠、点击面、`output limited` 显示）。
- `src/app/state/lx/chat/steps.rs` 的 `ToolSummary` 字段与 `absorb_result_json`。
- `src/app/update/chat/view.rs` 的投影逻辑。
- `src/chat/context/mod.rs` 的 `MAX_TOOL_RESULT_REPLAY_BYTES`。
- `src/db/**` 的 schema 与迁移。
- `search` / MCP 的截断逻辑。
- 不引入 Tailwind、构建步骤或新依赖。

## 8. 回滚

- 本改动无 DB 迁移，回滚只需还原代码：
  - `src/chat/tools/mod.rs`
  - `src/chat/tools/types.rs`
  - `src/chat/tools/read.rs`
  - `src/chat/tools/output.rs`
  - `src/chat/tools/schema.rs`
  - `src/skills/assets/lx-guide/references/tools.md`
  - 对应测试文件
- 旧会话数据不受影响；回滚后新字段不再写入，旧行仍可正常读取。
- 如果只回滚常量而保留新字段，行为会回到 256 KiB/4000 字符，但提示仍是新的精确文案；不建议半回滚。

## 9. 提交建议（需用户明确授权）

建议拆成两个提交：

1. `feat(tools): cap read output at 50 KiB and report truncation precisely`
   - 常量、类型、read.rs、output.rs、schema、guide。
2. `test(tools): cover read truncation budget and replay boundary`
   - 测试修改与新增。

不要在同一提交里混入 `docs/feature/lx-read-output-truncation/` 之外的文档改动。
