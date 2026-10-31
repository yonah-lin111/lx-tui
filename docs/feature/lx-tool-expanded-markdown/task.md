# lx 工具与卡片展开内容统一思考块同款压暗 Markdown 格式与配色：任务清单

> 状态：**已在 `.worktrees/lx-tool-expanded-markdown`（分支 `feat/lx-tool-expanded-markdown`）完成 T1–T6，待用户确认是否合并到 `dev`**。
> 日期：2026-10-31。
> 设计依据：[design.md](./design.md)。

---

## 1. 当前已完成的设计准备

- [x] 阅读根 `AGENTS.md` 与项目级 `AGENTS.md` 及四份核心规范文档（`project-directory-structure.md`、`code-writing-standards.md`、`tui-design-requirements.md`、`html-assets.md`）。
- [x] 使用 `codegraph`（`codegraph_explore`）与 `codebase-memory-mcp`（`list_projects`、`search_graph`、`index_status`、`check_index_coverage`）分析 `src/ui/main_content/lx/history/` 下思考块与全部工具卡片的渲染链路。
- [x] 使用 `grill-me` 逐题向用户确认范围分类（D1）、折叠预览与展开态同源渲染策略（D2）、附属标签行与规范/测试同步策略（D3）。
- [x] 在 `docs/feature/lx-tool-expanded-markdown/` 下编写详细的 `design.md` 与 `task.md`。
- [x] 用户审核确认设计方案与任务清单，新建 `.worktrees/lx-tool-expanded-markdown` 并完成 `codegraph` 与 `codebase-memory-mcp` 初始化。

---

## 2. 执行任务拆分（审核通过后按序执行）

### T1：在 `history/tool.rs` 中实现共用的压暗 Markdown 渲染辅助函数并收口 `output_style`

- **目标文件**：`src/ui/main_content/lx/history/tool.rs`
- **具体步骤**：
  - [x] 将 `pub(super) fn output_style(_expanded: bool) -> Style` 改为恒定返回 `style::muted()`（移除展开态返回 `style::text()` 的分支）。
  - [x] 新增 `pub(super) fn muted_markdown_lines(source: &str, width: u16) -> Vec<Vec<Segment>>`：调用 `crate::ui::markdown_render::render_wrapped(source, width.max(1))`，将每行 `MdLine` 转换为 `Vec<Segment>`。
  - [x] 新增 `pub(super) fn md_line_segments(line: crate::ui::markdown_render::MdLine) -> Vec<Segment>`：处理 `line.indent` 前导空格（`style::muted()`），并对 `line.spans` 中每个非空分段应用 `span_style.patch(style::muted())`（与 `paint.rs::draw_thinking` 第 546 行完全同源）。
  - [x] 新增 `pub(super) fn sample_markdown_source(raw: &str) -> String`：按行去除尾部 `\r` 并执行 `sample_output_line` 单行限长采样、剥离末尾空行，保证极长单行输出在送入 `markdown_render` 前仍有界且保留 `…` 截断标记。

### T2：改造 `subagent.rs` 与 `report.rs` 的子 Agent 报告渲染

- **目标文件**：
  - `src/ui/main_content/lx/history/tool/subagent.rs`
  - `src/ui/main_content/lx/history/report.rs`
- **具体步骤**：
  - [x] 在 `tool/subagent.rs::report_node` 中：
    - 将 `text::agent::REPORT_LABEL` 的样式从 `style::text()` 改为 `style::muted()`；
    - 移除原先丢弃 `MdLine.spans` 样式并涂成 `style::text()` 的循环，直接调用 `muted_markdown_lines(body, body_width)` 追加到 `lines`。
  - [x] 在 `report.rs` 中：
    - 将 `body_lines` 返回值由 `Vec<String>` 改为 `Vec<Vec<Segment>>`，内部直接调用 `muted_markdown_lines(&report.body, body_width)`；
    - 将 `BodyView.lines` 类型更新为 `Vec<Vec<Segment>>`；
    - 在 `draw` 中移除 `expanded -> style::text()` 分支，`Report:` 标签固定用 `style::muted()`，正文行通过 `put_segments(grid, body_x, y, body_width, line)` 绘制。

### T3：改造 `mcp.rs`、`command.rs`、`user_shell.rs` 与 `compact.rs` 的展开/预览内容渲染

- **目标文件**：
  - `src/ui/main_content/lx/history/tool/mcp.rs`
  - `src/ui/main_content/lx/history/tool/command.rs`
  - `src/ui/main_content/lx/history/user_shell.rs`
  - `src/ui/main_content/lx/history/compact.rs`
  - `src/ui/main_content/lx/history/tool/search.rs`
- **具体步骤**：
  - [x] **`tool/mcp.rs`**：
    - 为 `result_section` 增加 `width: u16` 参数；
    - 新增 `sanitize_result_line(line: &str, max_chars: usize) -> String`（保留行首空格以支持 Markdown 列表/代码缩进，过滤控制字符并限长 8000 字符）；
    - 将 `error_fallback` 或 `view.result` 按行净化后拼接，调用 `muted_markdown_lines(&raw_source, body_width)` 生成结果节点行；
    - 图片行 `image_line` 样式统一为 `style::muted()`。
  - [x] **`tool/command.rs`**：
    - 在 `output_section` 中使用 `sample_markdown_source(output)` + `muted_markdown_lines(&sampled, body_width)` 替代原纯文本 `output_lines`；
    - 移除不再使用的 `output_lines` 私有函数；
    - 确认 `SendInput` 的 `input_preview` 通过 `output_style(folds.output)` 保持 `style::muted()`。
  - [x] **`user_shell.rs`**：
    - 在 `output_section` 中使用 `sample_markdown_source(&shell.output)` + `muted_markdown_lines(&sampled, body_width)` 替代原 `all_output_lines`；
    - 无输出时 `text::LX_SHELL_NO_OUTPUT` 使用 `style::muted()`；
    - 移除不再使用的 `all_output_lines` 私有函数。
  - [x] **`compact.rs`**：
    - 在 `rows` 中将展开态 `super::wrap(&notice.summary, width).len()` 替换为 `super::tool::muted_markdown_lines(&notice.summary, width).len()`；
    - 在 `draw` 中遍历 `super::tool::muted_markdown_lines(&notice.summary, content.width)` 并使用 `super::tool::put_segments(grid, content.x, y, content.width, &segments)` 绘制。
  - [x] **`tool/search.rs`**：
    - 确认 `hit_line` 中的命中正文随 `output_style(expanded)` 在折叠与展开态下均保持 `style::muted()`，且不经过 Markdown 解析。

### T4：更新单元测试与补充压暗 Markdown 渲染回归测试

- **目标文件**：
  - `src/ui/main_content/lx/history/report.rs`
  - `src/ui/main_content/lx/history/tool/subagent.rs`
  - `src/ui/main_content/lx/history/tool/mcp/tests.rs`
  - `src/ui/main_content/lx/history/tool/command.rs`
  - `src/ui/main_content/lx/history/tool/tests/background_cards.rs`
  - `src/ui/main_content/lx/history/user_shell.rs`
  - `src/ui/main_content/lx/history/tool/search.rs`
  - `src/ui/main_content/lx/history/tests/compact.rs`
- **具体步骤**：
  - [x] 更新旧测试中要求「展开后变为 `style::text()`」的断言为 `style::muted()`。
  - [x] 新增断言：验证 `report.rs`、`subagent.rs`、`mcp.rs`、`command.rs`、`user_shell.rs`、`compact.rs` 在包含 Markdown 行内代码（如 `` `cargo test` ``）或标题/列表时，单元格样式等于 `style::md_read_inline_code().patch(style::muted())`（兼具 GitHub Dark 前景色与 `Modifier::DIM`），普通文本段样式等于 `style::muted()`，且 `rows()` 与 `draw()` 行数严格一致。
  - [x] 验证 `search.rs` 命中行在展开后保持原样纯文本与 `style::muted()`。

### T5：同步更新项目设计规范文档

- **目标文件**：`docs/standards/tui-design-requirements.md`、`AGENTS.md`
- **具体步骤**：
  - [x] 更新第 125、147、158 行中关于工具卡片输出区、MCP 结果块、子 Agent 汇报块及本地命令输出在展开态下的样式规范描述，与本次实现保持严格一致。

### T6：受影响范围精确验证（优先使用 `rtk`）

- **执行命令**：
  ```bash
  rtk cargo fmt
  rtk cargo clippy --all-targets -- -D warnings
  rtk cargo test --locked --lib ui::main_content::lx::history
  ```
- **验收标准**：
  - [x] `ui::main_content::lx::history` 下全部单元测试（含新增 Markdown 压暗样式测试与几何一致性测试）100% 通过；
  - [x] `cargo clippy --all-targets -- -D warnings` 零警告；
  - [x] `cargo fmt -- --check` 零差异；
  - [x] 单文件行数均不超过 1000 行规范上限。
