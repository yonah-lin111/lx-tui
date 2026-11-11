# lx 工具输出截断与消息列表截断展示统一：代码审查报告（Review）

> 状态：**已审查并完成全部修复（P0 ～ P3，共 10 项），已合并至 `dev`**
> 日期：2026-11-11
> 分支：`feat/lx-read-output-truncation` → `dev`
> 工作区：`/Users/yonah/projects/agent/lx-tui`（原工作区 `.worktrees/lx-read-output-truncation` 已清理）
> 配套文档：[design.md](./design.md) · [task.md](./task.md)

---

## 审查概览

本次对工作区分支 `feat/lx-read-output-truncation` 新增的 **工具执行层输出截断（`read` / `web_search` / `exec_command` / `write_stdin` 统一 50 KiB JSON 转义预算 + 2000 行 + 单行 2000 字符上限与 `rtk` 省略标记汇总）** 与 **TUI 消息列表工具卡片截断展示统一（`… N more lines [truncated]` 唯一位置约束、`read` 单行路径显示、移除标题栏截断文案与 `output limited`）** 共 41 个改动文件进行了深度对抗式代码审查，共识别出 **10 项问题（P0 × 1、P1 × 3、P2 × 4、P3 × 2）**，涵盖折叠布局行数错算与双倍提示行、运行态进程读取游标越界丢行、`rtk` 过滤前提前截断、超长命令参数回放越界、单文件 1000 行上限红线、单行尾部省略号丢失与文档漂移等，并已全部完成修复与回归测试覆盖。

| 优先级 | 编号 | 关键点 | 涉及文件 | 状态 |
| :---: | :---: | :--- | :--- | :---: |
| **P0** | **P0-1** | `web_search` 与 `mcp` 在折叠态/展开态手推 `more_lines` 提示行，导致 `lines.len() == 3` 时被 `Preview::head(3)` 二次截成 `… 1 more line [truncated]`、含图片时出现双倍提示行、折叠与展开态剩余行数跳变 | `src/ui/main_content/lx/history/tool/{tree.rs,web_search.rs,mcp.rs}` | 已修复 |
| **P1** | **P1-1** | `ProcessManager::result_of` 在 `rtk::filter_output_flagged` 之前提前执行 `enforce_output_bounds()`，且运行中进程（`!is_terminal`）先将 `cursor` 推进到 `read.end` 再裁掉超限行，导致后续 `write_stdin` 永久丢行及 `truncated` 假阳性 | `src/chat/process/manager/mod.rs`、`src/chat/process/output.rs`、`src/chat/agent_loop/executor.rs` | 已修复 |
| **P1** | **P1-2** | 生产文件 `src/chat/process/manager/mod.rs` 行数达 1080 行，超出项目生产代码单文件 1000 行上限红线 | `src/chat/process/manager/{mod.rs,handle.rs}` | 已修复 |
| **P1** | **P1-3** | 超长 `cmd` 参数在 `ProcessResult` 中未限长可使 `result_json` 突破 64 KiB 回放上限，且 `bound_model_output` 在非终态/单行超预算分支无条件追加 `…[truncated]` 导致单行短输出逆向膨胀或重复标记 | `src/chat/process/{mod.rs,output.rs}` | 已修复 |
| **P2** | **P2-1** | `web_search` 的 `bound_combined_text` 在最后一条保留行恰触发单行 2000 字符裁剪时漏加独立 `\n…[truncated]` 标记行，且触发 2000 行上限时总行数为 2001 行 | `src/chat/tools/web_search/mod.rs` | 已修复 |
| **P2** | **P2-2** | UI 剥离行尾 `…[truncated]` 时连同省略号 `…` 一并删掉、`web_search.rs` 对整行调用 `.replace(...)` 可能误删正文字面量、`search.rs` 丢弃 `hit.line_truncated` 的 `…` 提示，以及 `tool.rs` 跳过 `…[output truncated]` 导致首尾残行无分隔粘连 | `src/ui/main_content/lx/history/{tool.rs,tool/web_search.rs,tool/mcp.rs,tool/search.rs}`、`src/app/state/lx/chat/tool_view/web_search.rs` | 已修复 |
| **P2** | **P2-3** | `file_summary.rs` 中 `read` 卡片在无路径时的状态兜底词（`receiving input` / `cancelled` 等）未做单行尾部省略，窄屏下会被软折成多行；`web_search` / `mcp` 错误回退误走 `muted_markdown_lines` | `src/ui/main_content/lx/history/tool/{file_summary.rs,web_search.rs,mcp.rs}` | 已修复 |
| **P2** | **P2-4** | `json_escaped_len` 逐行调用 `serde_json::to_string` 产生大量堆分配，且 `bound_model_output` 在检查 2000 行上限前对全部原始行执行裁剪与序列化 | `src/chat/process/output.rs`、`src/chat/tools/{read.rs,web_search/mod.rs}` | 已修复 |
| **P3** | **P3-1** | `src/ui/text/mcp.rs::RESULT_TRUNCATED` 成为无引用死常量，`file_summary.rs` / `search.rs` / `command.rs` 模块顶部文档注释仍描述旧的 `output limited` / `results truncated` 行为 | `src/ui/text/mcp.rs`、`src/ui/main_content/lx/history/tool/{file_summary.rs,search.rs,command.rs}` | 已修复 |
| **P3** | **P3-2** | `docs/feature/lx-read-output-truncation/{design.md,task.md}` 停留在早期仅改后端的阶段，未同步后续 UI 截断展示统一与 `read` 单行改造，任务清单状态未勾选 | `docs/feature/lx-read-output-truncation/{design.md,task.md,review.md}` | 已修复 |

---

## 详细问题清单（按 P0 → P3 排序）

### P0：阻断性功能与渲染缺陷

#### [P0-1] `web_search` 与 `mcp` 在折叠态/展开态手推 `more_lines` 提示行，导致 `lines.len() == 3` 时被 `Preview::head(3)` 二次截断、含图片时双倍提示行、剩余行数口径跳变

- **关键点**：
  - 位置：`src/ui/main_content/lx/history/tool/web_search.rs:142-159`、`src/ui/main_content/lx/history/tool/mcp.rs:43-52, 117-146`、`src/ui/main_content/lx/history/tool/tree.rs:80-111, 239-256`
  - 现象：
    1. 当 `web_search` 或 `mcp` 的保留窗口内折行后恰好为 3 行（`lines.len() == BLOCK_PREVIEW_ROWS`）且窗口外仍有省略行（如 `result_more = 10`）时，折叠态错误显示 `… 1 more line [truncated]` 而非 `… 10 more lines [truncated]`，且将并无更多窗口内内容可展开的卡片标为可点击；
    2. 当 `mcp` 结果文本 `<= 3` 行、`result_more > 0` 且附带 `view.images` 时，折叠态底部连续渲染两行 `… N more lines [truncated]`；
    3. 当保留窗口内有 40 行（`web_search`）或 20 行（`mcp`）且 `result_more = 500` 时，折叠态只显示窗口内隐藏行数（如 `… 37 more lines`），点击展开后才跳变为 `… 500 more lines`。
- **错误原因**：
  - `web_search::result_section` 与 `mcp::result_section` 使用了 `if more > 0 && (expanded || lines.len() <= BLOCK_PREVIEW_ROWS)` 将 `more_lines` 提示行直接推入正文节点 `Node::block(lines)`：
    - 当 `lines.len() == 3` 且 `!expanded` 时，`3 <= 3` 为真，推入第 4 行提示后交由 `Preview::head(3)` 裁剪，`tree::layout` 把第 4 行（提示行本身）当成被折叠的正文行隐藏（`section_hidden = 1`），并在下方重新生成 `… 1 more line [truncated]`；
    - `mcp::result_section` 在推入提示行之后才推入 `view.images`，导致提示行占掉前 3 行预览配额，后续图片行被隐藏并触发 `tree::layout` 生成第二条 `more lines [truncated]`；
    - `mcp::arguments_section` 无条件推入 `more_lines(view.arguments_more)`，使折叠态下该提示行本身也被计入 `section_hidden`（多算 1 行）。
- **修复内容**：
  1. 在 `tree::Section` 上新增 `omitted_lines: usize` 字段与 `.with_omitted(usize)` 构造器，专门承载视图保留窗口之外省略的行数；
  2. `layout.hidden` 仍只累加被 `Preview` 折叠隐藏的窗口内物理行 `section_hidden`（保证仅当展开能多看内容时 `expandable` 才为 `true`），而底部提示行统一由 `tree::layout` 按 `total_omitted = section_hidden.saturating_add(section.omitted_lines)` 生成唯一一行 `… {total_omitted} more line(s) [truncated]`（当带 `[truncated]` 超出窄列宽但基础提示放得下时自动退回基础提示）；
  3. 移除 `web_search::result_section`、`mcp::arguments_section`、`mcp::result_section` 手动向 `lines` 推入的 `more_lines` 行，统一改调 `.with_omitted(more)`。
- **回归测试**：
  - `ui::main_content::lx::history::tool::web_search::tests::three_line_result_with_omitted_lines_reports_exact_more_count_without_phantom_fold`
  - `ui::main_content::lx::history::tool::mcp::tests::short_mcp_result_with_images_and_more_lines_never_renders_duplicate_more_hints`

---

### P1：数据丢失、规范红线违背与核心边界缺陷

#### [P1-1] `ProcessManager::result_of` 在 `rtk` 过滤前提前执行 `enforce_output_bounds()`，且运行中进程先推进游标再裁掉尾部行导致 `write_stdin` 永久丢行

- **关键点**：
  - 位置：`src/chat/process/manager/mod.rs:400-405, 875-915`、`src/chat/process/output.rs:221-296, 379-478`、`src/chat/agent_loop/executor.rs:386-398`
  - 现象：
    1. 运行中进程（`Running`）单次产生 > 2000 行短行或超转义预算输出时，`handle.read_for_model` 先将 `handle.cursor` 推进到 `read.end`，随后 `enforce_output_bounds()` 把 2000 行之后的尾部行丢弃；模型下次调用 `write_stdin` 从 `read.end` 续读时，被裁掉的行永久丢失；
    2. 运行中进程若仅输出 1 行 > 2000 字符的文本且缓冲已读空（`read.truncated == false`），`enforce_output_bounds()` 因单行裁剪将 `result.truncated` 置为 `true`，导致 `format_process` 输出 `" · more output pending"` 误导模型空转轮询；
    3. `exec_observed` 内部通过 `result_of` 在 `AgentTools::execute_process` 调用 `rtk::filter_output_flagged` **之前**就执行了一次 `enforce_output_bounds()`（随后 `executor.rs` 又执行第二次），且终态默认预算被限制在 50 KiB 而非完整缓冲 256 KiB：导致 > 2000 行全部通过的 `cargo test` 被提前置 `truncated = true`（假阳性），或 > 50 KiB 输出的末尾退出摘要在 `rtk` 过滤与 `head + tail` 截取前就被提前截掉。
- **错误不在一处，根因如下**：
  1. `OutputBuffer::sanitize_text` 在 `flush == false`（运行态）时未感知 2000 行与 JSON 转义预算上限，消费了超出单次可返回范围的原始字节；
  2. `bound_model_output` 在 `!is_terminal` 且仅触发单行字符裁剪（未省略任何行）时也将返回标志置为 `true`；
  3. `result_of` 过早调用 `enforce_output_bounds()` 且对终态默认 `max_output_tokens` 未读取完整保留缓冲（`MAX_PROCESS_OUTPUT_BYTES`）。
- **修复内容**：
  1. 在 `OutputBuffer::sanitize_text` 中，当 `flush == false`（运行中进程）时按行跟踪已输出行数与转义字节开销，到达 `MAX_MODEL_OUTPUT_LINES`（2000 行）或安全转义预算时在行边界停止消费字节，保证 `read.end` 与 `handle.cursor` 严格停在实际返回给模型的最后一行末尾；
  2. 在 `bound_model_output` 中，当 `!is_terminal` 且未省略任何行（仅单行内联裁剪 `…[truncated]`）时返回 `flagged = false`，避免空缓冲误报 `more output pending`；
  3. 从 `ProcessManager::result_of` 移除提前执行的 `enforce_output_bounds()`，改为在 `ProcessManager::{exec, interact, stop}` 与 `AgentTools::execute_process`（完成 `rtk::filter_output_flagged` 之后）各执行唯一一次 `enforce_output_bounds()`；同时对终态默认预算读取完整保留缓冲（`MAX_PROCESS_OUTPUT_BYTES`），让 `rtk` 与终态 `head + tail` 拿到真实的尾部摘要。
- **回归测试**：
  - `chat::process::output::tests::bound_model_output_clips_long_lines_and_caps_lines_and_escaped_bytes`（第 5 项：运行中进程 2500 行分两次读取零丢行）

#### [P1-2] 生产文件 `src/chat/process/manager/mod.rs` 超出 1000 行上限（1080 行）

- **关键点**：
  - 位置：`src/chat/process/manager/mod.rs`（1080 行）
  - 现象：违反 `AGENTS.md` 与 `docs/standards/code-writing-standards.md` 中“**生产代码单文件行数上限 1000，超限必须按职责拆分，拆分后子文件控制在 500 行左右**”的硬性规范。
- **错误原因**：
  - `src/chat/process/manager/mod.rs` 原本已接近 1000 行，叠加本分支的进程输出上限逻辑后膨胀至 1080 行。
- **修复内容**：
  - 将受管进程共享状态句柄 `ProcessHandle`、`StopReason`、`StdinRequest`、`stop_handle` 与 `estimated_tokens` 按职责拆分为子模块 `src/chat/process/manager/handle.rs`（296 行），使 `src/chat/process/manager/mod.rs` 降至 741 行。

#### [P1-3] 超长 `cmd` 参数可使 `ProcessResult::to_json` 突破 64 KiB 回放上限，且 `bound_model_output` 非终态/单行超预算分支无条件追加 `…[truncated]`

- **关键点**：
  - 位置：`src/chat/process/mod.rs:317-332`、`src/chat/process/output.rs:462-478`
  - 现象：
    1. 当模型传入超长 `cmd`（如 60 KiB 的 `python3 -c "..."` 内联脚本）时，`ProcessResult::to_json` 原样序列化 `self.command`，导致 `result_json` 突破 `MAX_TOOL_RESULT_REPLAY_BYTES`（64 KiB），重进会话回放时 JSON 字符串被硬截断损坏；
    2. 当 `meta_bytes` 很大使 `budget` 极小，或单行已被 `clip_model_line` 裁剪且超出 `budget` 时，`bound_model_output` 末尾分支在 `kept.len() == line_costs.len()`（并未省略任何行）的情况下仍无条件 `kept.push("…[truncated]")`，导致 1 行短输出被逆向膨胀或出现重复两次 `…[truncated]`。
- **错误原因**：
  1. `ProcessResult::enforce_output_bounds` 只裁剪了 `self.output`，未限制持久化在 `ProcessResult.command` 中的命令文本长度；
  2. `bound_model_output` 末尾分支未检查 `kept.len() < line_costs.len()` 就无条件推入 `"…[truncated]"` 并返回 `true`。
- **修复内容**：
  1. 在 `ProcessResult::enforce_output_bounds` 中，当 `self.command` 超过 `MAX_MODEL_OUTPUT_LINE_CHARS`（2000 字符）时截断并补 `…`，确保 `meta_bytes` 有界且 `to_json()` 恒 `<= 64 KiB`；
  2. 在 `bound_model_output` 末尾分支仅当 `omitted_lines`（`kept.len() < line_costs.len()`）为真时才推入 `"…[truncated]"`。
- **回归测试**：
  - `chat::process::output::tests::bound_model_output_clips_long_lines_and_caps_lines_and_escaped_bytes`（第 4 项：60 KiB 超长 `command` 落库 JSON 保持在 64 KiB 以内）

---

### P2：次要边界缺陷、性能与交互细节

#### [P2-1] `web_search` 的 `bound_combined_text` 在最后一条保留行恰触发单行 2000 字符裁剪时漏加独立 `\n…[truncated]` 行，且触发 2000 行上限时总行数为 2001 行

- **关键点**：
  - 位置：`src/chat/tools/web_search/mod.rs:382-412`
  - 现象：当 `bound_combined_text` 因达到行数或字节预算而丢弃后续行/后续查询（`omitted_lines = true`），且最后一条被保留的行恰好超过 2000 字符（被 `clip_line` 加上了行尾内联 `…[truncated]`）时，末尾缺少独立的 `\n…[truncated]` 标记行；同时 2500 行输入被截断后实际产出 2001 行。
- **错误原因**：
  - `if omitted_lines && !out.ends_with("…[truncated]")` 混淆了行尾内联的 `"…[truncated]"` 与独立行 `"\n…[truncated]"`；且循环在 `emitted >= MAX_WEB_SEARCH_LINES`（已满 2000 行）后才退出并追加第 2001 行标记。
- **修复内容**：
  1. 当 `lines.len() > MAX_WEB_SEARCH_LINES` 时将正文保留上限设为 `MAX_WEB_SEARCH_LINES - 1`（1999 行），使加上尾部标记行后总行数严格 `<= 2000`；
  2. 将尾部判断改为 `if omitted_lines && !out.ends_with(TRUNCATED_SUFFIX) && out != "…[truncated]"`。
- **回归测试**：
  - `chat::tools::web_search::tests::combined_batch_respects_50_kib_json_escaped_budget_and_2000_line_cap`

#### [P2-2] UI 剥离行尾 `…[truncated]` 时连同省略号 `…` 一并删掉、`web_search.rs` 全行 `.replace` 误删正文字面量、`search.rs` 丢弃 `line_truncated` 的 `…`、以及 `tool.rs` 吞掉中间省略分隔行

- **关键点**：
  - 位置：`src/ui/main_content/lx/history/tool.rs:143-185`、`src/ui/main_content/lx/history/tool/web_search.rs:77-95`、`src/app/state/lx/chat/tool_view/web_search.rs:170-184`、`src/ui/main_content/lx/history/tool/mcp.rs:55-68`、`src/ui/main_content/lx/history/tool/search.rs:39-54`
  - 现象：
    1. 被 2000 字符上限截断的单行在剥离 `"…[truncated]"` 后连末尾 `'…'` 也一并消失，在 UI 中看起来像完整行；`search` 命中行的 `hit.line_truncated` 不再显示 `'…'`；`mcp::sanitize_result_line` 超 8000 字符截断时漏补 `'…'`；
    2. `web_search.rs:91-94` 对整行调用 `.replace("[truncated]", "")`，若网页正文含 `"[truncated]"` 字面量会被静默删空；
    3. `tool.rs::sample_markdown_source` 将 `rtk::filter_generic` 插入在前 8 KiB 与后 8 KiB 之间的 `"…[output truncated]"` 直接跳过，导致展开态首尾残行无分隔粘连；而 `rtk` 在输出最后一行追加的 `…[N matches omitted]` 在折叠态 `head_tail(3, 2)` 预览中与 `… M more lines [truncated]` 重复出现。
- **错误原因**：
  - `strip_suffix("…[truncated]")` 把代表单行被截断的省略号 `'…'` 与 `"[truncated]"` 标签一起剥除；`sample_markdown_source` 未区分“位于输出中间的分隔标记”与“位于输出末尾的独立截断标记”。
- **修复内容**：
  1. 在 `tool.rs::sample_output_line`、`web_search.rs::sanitize_result_line`、`tool_view/web_search.rs::result_line`、`mcp.rs::sanitize_result_line` 中剥离行尾 `"…[truncated]"` / `"[...truncated]"` 时保留单个 `'…'`，仅移除 `"[truncated]"` 文字标签；移除 `web_search.rs` 中对整行的 `.replace(...)`；
  2. 在 `search.rs::hit_line` 中当 `hit.line_truncated` 为真时追加 `"…"`（不追加 `[truncated]`）；
  3. 在 `tool.rs::sample_markdown_source` 中将中间的 `"…[output truncated]"` 转为 `"…"` 分隔行，并剥离输出末尾独立的 `…[... omitted]` / `…[truncated]` 行。

#### [P2-3] `file_summary.rs` 中 `read` 状态兜底词窄屏未截断，以及 `web_search` / `mcp` 错误回退样式压暗

- **关键点**：
  - 位置：`src/ui/main_content/lx/history/tool/file_summary.rs:353-395`、`src/ui/main_content/lx/history/tool/web_search.rs:114-128`、`src/ui/main_content/lx/history/tool/mcp.rs:92-104`
  - 现象：
    1. `read` 卡片在 `Cancelled` 或无路径的 `InputStreaming`（`"receiving input"`，15 列）状态下，窄屏（`width < 20`）会将状态词软折成 2～5 行，违反 `read` 卡片恒单行的不变量；
    2. `web_search` 与 `mcp` 在失败且结果为空时的 `error_fallback` 走了 `muted_markdown_lines`，使错误文本被压暗且按 Markdown 解析。
- **错误原因**：
  1. `Kind::Read` 分支仅对 `path` 和 `error` 调用了 `truncate_with_ellipsis(..., body_width)`，遗漏了 `Cancelled` 与 `InputStreaming`/`Pending`/`Running` 兜底状态词；
  2. `web_search::result_section` 与 `mcp::result_section` 将 `error_fallback` 拼入 `raw_source` 交给 `muted_markdown_lines`。
- **修复内容**：
  1. 对 `Kind::Read` 的状态兜底词统一调用 `truncate_with_ellipsis(label, body_width)`；
  2. `web_search::result_section` 与 `mcp::result_section` 的 `error_fallback` 分支直接构造 `style::text()` 的 `Segment` 行。

#### [P2-4] `json_escaped_len` 逐行序列化分配与 `bound_model_output` 超多行全量预处理开销

- **关键点**：
  - 位置：`src/chat/process/output.rs:368-455`、`src/chat/tools/read.rs:277-288`、`src/chat/tools/web_search/mod.rs:424-435`
  - 现象：`json_escaped_len` 每次调用都执行 `serde_json::to_string(text)` 分配新 `String`；且 `bound_model_output` 在判断 2000 行上限之前对 `output.lines()` 的全部行执行 `clip_model_line` 与 `json_escaped_len`。
- **错误原因**：
  - 直接借助 `serde_json::to_string` 测量长度，且未在 `raw_lines.len() > MAX_MODEL_OUTPUT_LINES` 时跳过中间不可能被选入 head/tail 的行。
- **修复内容**：
  1. 将三处 `json_escaped_len` 统一改为零分配字节扫描（`"`、`\`、`\n`、`r`、`\t`、`\x08`、`\x0c` 计 +1 字节，其余 `< 0x20` 控制字符计 +5 字节）；
  2. 在 `bound_model_output` 中当 `raw_lines.len() > MAX_MODEL_OUTPUT_LINES` 时，终态仅处理前 1000 行与后 1000 行候选，非终态仅处理前 1999 行候选。

---

### P3：死常量、模块注释与文档同步

#### [P3-1] 清理未使用的 `text::mcp::RESULT_TRUNCATED` 常量与过时的模块顶部文档注释

- **关键点**：
  - 位置：`src/ui/text/mcp.rs:20`、`src/ui/main_content/lx/history/tool/{file_summary.rs,search.rs,command.rs}`
  - 现象：`text::mcp::RESULT_TRUNCATED` 在全仓已无任何引用；`file_summary.rs`、`search.rs`、`command.rs` 顶部文档注释仍描述 `output limited` / `results truncated` 与独立截断节点。
- **修复内容**：
  - 移除 `src/ui/text/mcp.rs::RESULT_TRUNCATED`，同步更新 `file_summary.rs`、`search.rs`、`command.rs` 的模块级文档注释。

#### [P3-2] 更新 `docs/feature/lx-read-output-truncation/{design.md,task.md}` 并新增 `review.md`

- **关键点**：
  - 位置：`docs/feature/lx-read-output-truncation/{design.md,task.md,review.md}`
  - 现象：`design.md` 与 `task.md` 仍停留在最初“只改 `read` 后端、不改 `src/ui/main_content/lx/**`”的口径，未反映后续扩展的 `web_search` / `exec_command` / `write_stdin` 截断、UI `… N more lines [truncated]` 统一展示、`read` 单行卡片以及本次审查修复；`task.md` 验收项未勾选。
- **修复内容**：
  - 全面更新 `design.md` 与 `task.md` 至最终实现口径，勾选全部已完成任务项，并编写本 `review.md`。

---

## 验证结果

- `cargo fmt --check`：通过
- `cargo clippy --all-targets -- -D warnings`：0 warnings / 0 errors
- `cargo test --lib chat::`：1767 passed
- `cargo test --lib ui::main_content::lx`：671 passed, 4 ignored
- `cargo test --lib app::state::lx`：228 passed
