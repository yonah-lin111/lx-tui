# lx 工具与卡片展开内容统一压暗 Markdown 格式与配色 — 代码评审报告 (Code Review)

> 评审范围：`feat/lx-tool-expanded-markdown`（工作区 `/Users/yonah/projects/agent/lx-tui/.worktrees/lx-tool-expanded-markdown`）
>
> 检索与分析工具：`codegraph`（`codegraph_explore` 调用链与影响面分析）、`codebase-memory-mcp`（`detect_changes` 变更爆炸半径、`search_graph` 符号检索、`check_index_coverage` 索引覆盖核验）
>
> 优先级定义：
> - **P0**：功能性缺陷 / 终端渲染内容失真（吞字符、误食下划线或通配符星号导致输出文本残缺）
> - **P1**：终端网格列宽与控制字符净化不变量缺失（`\t` 等 0 宽控制字符导致词粘连与折行/绘制列宽失配）
> - **P2**：Markdown 结构块内部行内语法完备性缺失（`> ` 引用块内未折叠行内标记）
>
> 结论：**P0 / P1 / P2 全部已修复**，回归测试已补齐；`rtk cargo fmt`、`rtk cargo clippy --all-targets -- -D warnings`、相关单元与渲染测试全绿。

---

## P0-1：`markdown_render::inline_spans` 未校验词内下划线（intraword underscore）与分隔符首尾空白，导致 `snake_case` 标识符（如 `exec_command / write_stdin`）与通配符（如 `*.rs *.toml`）被吞掉符号并误渲染为斜体/粗体

### 1. 关键点
- **涉及文件**：
  - `src/ui/markdown_render.rs`（`inline_spans`、`delimited_emphasis`、`emphasis`）
  - `src/ui/markdown_render/tests.rs`（`intraword_underscores_and_spaced_asterisks_stay_literal`）
  - `src/ui/main_content/lx/history/tool/tests/background_cards.rs`（`command_output_sanitizes_tabs_and_preserves_snake_case_identifiers`）

### 2. 错误原因
1. 在 `src/ui/markdown_render.rs` 的 `inline_spans` 中，原先匹配 `__…__` 粗体与 `_…_` 斜体时直接调用 `delimited(rest, "__", "__")` 与 `delimited(rest, "_", "_")`，完全未检查开闭 `_` 前后是否紧邻 ASCII 字母或数字（CommonMark 词内下划线规则）。
2. 当工具输出（如 `command.rs`、`user_shell.rs`、`mcp.rs`）或列表项同一行出现两个带下划线的标识符（例如实际截图中的 `- exec_command / write_stdin`）时：
   - `exec_command` 中的 `_` 与 `write_stdin` 中的 `_` 被配对为 `_command / write_` 斜体；
   - 两个 `_` 字符被静默吞掉，屏幕上显示为错误的 `• execcommand / writestdin`，且中间的 `command / write` 被错误倾斜。
3. 同时，原 `delimited` 未校验开标记后与闭标记前不得紧邻空白字符（CommonMark 规则），导致命令输出中的 `ls *.rs *.toml` 或算术表达式 `2 * 3 * 4` 中的两个 `*` 跨空格配对，吞掉 `*` 并误设为斜体。

### 3. 修复方案与状态（已修复）
- [x] 在 `src/ui/markdown_render.rs` 中新增 `delimited_emphasis(rest: &str, delim: &str, prev: Option<char>) -> Option<&str>`：
  - 强制要求开标记后首个字符非空白且非同种标记字符，闭标记前一个字符非空白且非同种标记字符；
  - 对 `_` 与 `__` 标记，额外校验开标记前一个字符 `prev` 不得为 ASCII 字母/数字或 `_`，闭标记后一个字符不得为 ASCII 字母/数字；
  - 循环向后查找首个合法闭标记（确保 `_snake_case_` 外层斜体仍可跨内部词内 `_` 正常闭合，而 `exec_command / write_stdin` 与 `_a_b and _c` 保持字面量）。
- [x] 新增单元测试 `intraword_underscores_and_spaced_asterisks_stay_literal` 与 `command_output_sanitizes_tabs_and_preserves_snake_case_identifiers` 覆盖验证。

---

## P1-1：`sample_output_line` 未净化 `\t` 等控制字符，导致含 Tab 的命令输出在 `Grid::put_clipped` 绘制时被当作 0 宽跳过、词粘连且列宽失配

### 1. 关键点
- **涉及文件**：
  - `src/ui/main_content/lx/history/tool.rs`（`sample_output_line`、`sample_markdown_source`）
  - `src/ui/main_content/lx/grid.rs`（`Grid::put_clipped`）
  - `src/ui/main_content/lx/history/tool/tests/background_cards.rs`

### 2. 错误原因
1. `exec_command` / `write_stdin`（`command.rs`）与本地 `!` 命令（`user_shell.rs`）的输出经常包含 `\t`（Tab）或其他控制字符。
2. 原 `sample_output_line` 仅按 `MAX_OUTPUT_LINE_CHARS` 截断字符，未将 `ch.is_control()` 转换为可见空格 `' '`（而 `mcp.rs::sanitize_result_line` 已做了控制字符转空格）。
3. 当含 `\t` 的字符串（如 `"755\tcheck.sh\t65B"`）进入渲染管线时：
   - `markdown_render::char_width('\t')` 与 `tool::wrap_spans` 因 `'\t'.width()` 返回 `None` 而走 `unwrap_or(1)` 计为 **1 列**；
   - `Segment::cols()` 调用 `UnicodeWidthStr::width` 对 `\t` 计为 **0 列**；
   - `Grid::put_clipped`（`src/ui/main_content/lx/grid.rs:82`）执行 `let cell = ch.width().unwrap_or(0) as u16; if cell == 0 { continue; }`，直接将 `\t` 跳过不写任何单元格！
   - 结果导致 `"755\tcheck.sh\t65B"` 在屏幕上粘连成 `"755check.sh65B"`，且同一行多段 `Segment` 的绘制起始列 `cx` 与折行计算的列宽不一致。

### 3. 修复方案与状态（已修复）
- [x] 在 `src/ui/main_content/lx/history/tool.rs::sample_output_line` 中，在单遍字符遍历限长（`MAX_OUTPUT_LINE_CHARS`）的同时将 `ch.is_control()`（含 `\t`）统一替换为空格 `' '`。
- [x] 新增回归测试 `command_output_sanitizes_tabs_and_preserves_snake_case_identifiers` 验证 `"755\tcheck.sh\t65B"` 渲染为 `"755 check.sh 65B"` 且行数与高度严格一致。

---

## P2-1：`markdown_render::blockquote` 未对引用正文执行 `inline_spans` 行内解析，导致 `> ` 引用块内的行内代码与加粗标记原样泄漏

### 1. 关键点
- **涉及文件**：
  - `src/ui/markdown_render.rs`（`blockquote`）
  - `src/ui/markdown_render/tests.rs`（`blockquote_renders_inline_markdown_spans_with_quote_tint`）

### 2. 错误原因
1. 在 `src/ui/markdown_render.rs` 的 `blockquote` 函数中，原先直接将整条引用正文构造为单段 `(body.to_string(), style::md_read_quote())`，未调用 `inline_spans(body)`。
2. 当引用块内包含行内代码（`` `code` ``）、粗体（`**bold**`）或链接时，原始反引号与星号标记直接泄漏到终端，且行内代码颜色与加粗修饰符全部丢失（而同文件的列表项 `marker_spans` 则正确调用了 `inline_spans`）。

### 3. 修复方案与状态（已修复）
- [x] 在 `src/ui/markdown_render.rs::blockquote` 中对 `body` 调用 `inline_spans(body)`，并对每个分段样式执行 `quote_style.patch(span_style)`，使普通文本保持模型上下文同款灰色（`style::md_read_quote()` = `style::chat_provider()`），同时完整保留行内代码着色与加粗/斜体修饰符。
- [x] 在 `src/ui/markdown_render/tests.rs` 新增 `blockquote_renders_inline_markdown_spans_with_quote_tint` 单元测试验证。
