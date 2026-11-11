# lx 工具长内容截断与消息列表截断展示统一：实施任务

> 状态：**已实施并完成代码审查修复（P0 ～ P3），已合并至 `dev`**。
> 对应设计：[design.md](./design.md) · 审查报告：[review.md](./review.md)。
> 范围：`read`、`web_search`、`exec_command` / `write_stdin` 返回给模型/落库的 50 KiB / 2000 行 / 单行 2000 字符截断；以及 TUI 消息列表工具卡片 `[truncated]` 位置统一与单体 `read` 单行路径显示。
> 日期：2026-11-11。

## 0. 执行前检查

- [x] 用户已审核并确认 `design.md` 的核心决策。
- [x] 在独立 worktree `/Users/yonah/projects/agent/lx-tui/.worktrees/lx-read-output-truncation`（分支 `feat/lx-read-output-truncation`）执行实施。
- [x] 确认涉及文件清单：
  - `src/chat/tools/{mod.rs,types.rs,read.rs,output.rs,schema.rs}`
  - `src/chat/tools/web_search/mod.rs`
  - `src/chat/process/{mod.rs,output.rs,manager/{mod.rs,handle.rs},rtk/{mod.rs,builtin.rs}}`
  - `src/chat/agent_loop/executor.rs`
  - `src/skills/assets/lx-guide/references/tools.md`
  - `src/app/state/lx/chat/tool_view/{web_search.rs,mcp.rs}`
  - `src/ui/text/{agent.rs,mcp.rs}`
  - `src/ui/main_content/lx/history/{tool.rs,tool/{tree.rs,command.rs,file_summary.rs,search.rs,web_search.rs,mcp.rs}}`

## 1. 常量与结果类型

### 1.1 `src/chat/tools/mod.rs`

- [x] `MAX_READ_BYTES`：`256 * 1024` → `50 * 1024`。
- [x] `MAX_READ_LINE_CHARS`：`4000` → `2000`。
- [x] 更新 doc comment，写明“对齐 opencode”。
- [x] 更新模块顶部契约：补充 `read` 输出 50 KiB 上限；输入 16 MiB / 目录 50 000 条不变。
- [x] `MAX_READ_LINES`、`MAX_READ_FILE_INPUT_BYTES`、`MAX_READ_DIR_ENTRIES` 保持不变。

### 1.2 `src/chat/tools/types.rs`

- [x] 在 `ToolExecutionResult::Read` 的 `truncated` 后新增 `line_truncated: bool` 与 `entries_truncated: bool`。
- [x] 确认 `ToolExecutionResult` 仍派生 `Debug, Clone, PartialEq, Eq`。

## 2. `read`、`web_search` 与进程执行层

### 2.1 `src/chat/tools/read.rs`

- [x] `read_image`、空文件、文件循环、空目录、目录正常返回 5 处构造点补齐 `line_truncated` 与 `entries_truncated`。
- [x] 新增零分配 `json_escaped_len(text)` 与 `content_budget(target)`（`MAX_READ_BYTES` 扣除 JSON 转义后的路径与 512 字节固定元数据余量），文件与目录循环按 `json_escaped_len(&rendered) + 2` 计费。
- [x] 确认 `entries_truncated` 只在收集阶段已满 `caps.dir_entries` 且还存在第 N+1 条时为 `true`。

### 2.2 `src/chat/tools/web_search/mod.rs`

- [x] 保留单条查询 `MAX_QUERY_OUTPUT_CHARS = 12_000` 字符上限。
- [x] 新增 `MAX_WEB_SEARCH_BYTES = super::MAX_READ_BYTES`（50 KiB）、`MAX_WEB_SEARCH_LINES = 2000`、`MAX_WEB_SEARCH_LINE_CHARS = 2000`。
- [x] 实现 `bound_combined_text` 与 `clip_line`：单行超 2000 字符追加 `…[truncated]`，聚合超行数/字节预算时保留前段（总行数 `<= 2000`）并追加独立 `\n…[truncated]` 行，任一触发置 `truncated = true`。

### 2.3 `src/chat/process/**` & `src/chat/agent_loop/executor.rs`

- [x] `MAX_MODEL_OUTPUT_BYTES` 收紧为 `50 * 1024`，`MAX_OUTPUT_TOKENS` 收紧为 `12_500`，新增 `MAX_MODEL_OUTPUT_LINES = 2000`、`MAX_MODEL_OUTPUT_LINE_CHARS = 2000`。
- [x] 实现 `bound_model_output`：终态超限保留首尾（head + tail，中间写 `…[N lines omitted]`），运行中超限保留前段并写 `…[truncated]`。
- [x] `OutputBuffer::sanitize_text` 在运行态（`flush == false`）按 2000 行与安全转义预算在行边界停止消费字节，防止游标越过未返回的行。
- [x] `rtk::builtin::filter_output_flagged` 返回省略标志 `omitted: bool`，由 `executor.rs` 汇总进 `result.truncated |= omitted`，并在 `rtk` 过滤完成后统一执行唯一一次 `result.enforce_output_bounds()`。
- [x] 将 `ProcessHandle` 拆分为 `src/chat/process/manager/handle.rs`，保持生产代码单文件行数 `< 1000`。

## 3. 模型文本与落库 JSON

### 3.1 `src/chat/tools/output.rs`

- [x] 新增私有 `ReadReport<'a>` 结构，重构 `format_read`，按 `next_offset` / `line_truncated` / `entries_truncated` 派生精确截断提示。
- [x] `to_result_json` 的 `Read` 分支写入 `"truncated"`、`"line_truncated"`、`"entries_truncated"`。
- [x] 确认 1 MiB 存储上限的 `truncate_value` 保留新字段。

## 4. 模型可见文档

- [x] 更新 `src/chat/tools/schema.rs` 中 `read` 的 2000 行 / 50 KiB / 单行 2000 字符描述。
- [x] 更新 `src/skills/assets/lx-guide/references/tools.md` 中 `read` 行为说明。

## 5. TUI 消息列表截断位置统一与单体 `read` 单行化

- [x] `src/ui/text/agent.rs`：定义 `pub const INLINE_TRUNCATED: &str = "[truncated]"`（无省略号前缀）；清理 `src/ui/text/mcp.rs::RESULT_TRUNCATED`。
- [x] `src/ui/main_content/lx/history/tool/tree.rs`：`Section` 新增 `omitted_lines: usize`（`.with_omitted(usize)`）与 `truncated: bool`（`.with_truncated(bool)`），统一在 `total_omitted > 0` 时生成唯一一行 `… N more lines [truncated]`，无 `more lines` 时不生成独立 `[truncated]` 行。
- [x] `src/ui/main_content/lx/history/tool/{command.rs,web_search.rs,mcp.rs,search.rs}` & `src/ui/main_content/lx/history/tool.rs`：
  - 移除标题右侧的截断状态词与 `output limited`；
  - 剥离输出正文与命中行末尾的 `[truncated]` 文字标签（保留被截断单行末尾的 `'…'`），确保 `[truncated]` 只出现在 `… N more lines [truncated]` 提示行上。
- [x] `src/ui/main_content/lx/history/tool/file_summary.rs`：单体 `read` 卡片标题下方只显示单行路径或单行错误/状态词（超宽尾部 `…` 省略，不软换行、不登记点击面）。

## 6. 测试与代码审查修复

- [x] `src/chat/tools/read/tests.rs`：覆盖 50 KiB / 2000 行 / 2000 字符上限、目录分页与收集上限、JSON 转义预算与 64 KiB 回放边界。
- [x] `src/chat/tools/output/tests.rs`：覆盖文件/目录分页与单行裁剪提示组合、JSON 字段透传与超限回退。
- [x] `src/chat/tools/web_search/tests.rs`：覆盖单查询 12,000 字符上限、多查询聚合 50 KiB / 2000 行上限与单行裁剪后尾部独立 `\n…[truncated]` 标记。
- [x] `src/chat/process/output/tests.rs` & `src/chat/process/rtk/tests.rs`：覆盖命令输出 50 KiB / 2000 行 / 2000 字符上限、超长 `command` 64 KiB 回放保护、运行中进程 2500 行分批读取零丢行、以及 `rtk` 省略标记汇总。
- [x] `src/ui/main_content/lx/history/tool/**`：覆盖 `read` 单行展示、`command` / `web_search` / `mcp` / `search` 的 `… N more lines [truncated]` 唯一性断言与 `lines.len() == 3` / 含图片边界回归测试。
- [x] 完成 `review.md` 中 P0 ～ P3 共 10 项问题的修复与验证。

## 7. 验证结果

```bash
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test --lib
```

- [x] `cargo fmt --check` 通过。
- [x] `cargo clippy --all-targets -- -D warnings` 零警告通过。
- [x] `cargo test --lib` 全量通过（5171 passed; 0 failed; 5 ignored）。
- [x] 在当前 worktree 隔离库创建并保留持久演示会话 `sess-18e6de95ad865e18-0`（`Truncation Demo (read / exec_command / web_search)`）供 `cargo run` 验收。
